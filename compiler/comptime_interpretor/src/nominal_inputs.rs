//! Resumable nominal/alias input collection. Admission and alias expansion run
//! only after every input has completed; capture domains use fixed-point wakeup.
use super::*;
use nir::program::{NormalFormArgument as A, NormalFormPreparation};

pub(super) struct Frame<'a> {
    arguments: &'a [A],
    specialized: Option<Vec<A>>,
    parameters: &'a [nir::DeclarationParameter],
    domains: &'a [Term],
    bindings: Option<&'a [LocalId]>,
    order: Option<&'a [usize]>,
    preparations: &'a [NormalFormPreparation],
    alias: bool,
    source: SourceRange,
    types: Vec<Term>,
    ready: BTreeSet<usize>,
    pending: BTreeMap<usize, normal_inputs::StagedNormalInput>,
    position: usize,
    preparation: usize,
    waiting: Option<Waiting>,
    drain: bool,
    terminal: bool,
}
enum Waiting {
    Preparation,
    Ready(usize),
    Stage(usize),
}

impl fmt::Display for Frame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(nominal-input-frame (position {}) (preparation {}) (alias {}) (terminal {}) (waiting",
            self.position, self.preparation, self.alias, self.terminal
        )?;
        match &self.waiting {
            Some(Waiting::Preparation) => write!(f, " preparation")?,
            Some(Waiting::Ready(slot)) => write!(f, " (ready {slot})")?,
            Some(Waiting::Stage(slot)) => write!(f, " (stage {slot})")?,
            None => {}
        }
        write!(f, ") (inputs")?;
        for (slot, ty) in self.types.iter().enumerate() {
            write!(
                f,
                " (input {slot} (ready {}) (pending {}) {ty})",
                self.ready.contains(&slot),
                self.pending.contains_key(&slot)
            )?;
        }
        write!(f, "))")
    }
}

impl<'a> Frame<'a> {
    pub fn new(
        expr: &'a Expr,
        parameters: &'a [nir::DeclarationParameter],
        domains: &'a [Term],
        specialize: bool,
    ) -> Self {
        let (K::NominalType {
            arguments,
            bindings,
            order,
            preparations,
            ..
        }
        | K::CheckTypeAlias {
            arguments,
            bindings,
            order,
            preparations,
            ..
        }) = &expr.kind
        else {
            unreachable!()
        };
        Self {
            arguments,
            specialized: specialize.then(|| arguments.clone()),
            parameters,
            domains,
            bindings: bindings.as_deref(),
            order: order.as_deref(),
            preparations,
            alias: matches!(expr.kind, K::CheckTypeAlias { .. }),
            source: expr.source,
            types: parameters
                .iter()
                .map(|parameter| Term::Rigid {
                    declaration: parameter.declaration,
                    sort: parameter.sort,
                })
                .collect(),
            ready: BTreeSet::new(),
            pending: BTreeMap::new(),
            position: 0,
            preparation: 0,
            waiting: None,
            drain: false,
            terminal: false,
        }
    }
    pub fn finish(self) -> Vec<Term> {
        self.types
    }
    fn input(&self, slot: usize) -> &A {
        self.specialized
            .as_ref()
            .map_or(&self.arguments[slot], |inputs| &inputs[slot])
    }
    fn specialize(&mut self, vm: &mut Interpreter<'_>, slot: usize) -> Result<(), Error> {
        if let Some(inputs) = &mut self.specialized {
            vm.specialize_contextual_input(&mut inputs[slot], self.parameters, &self.types)?;
        }
        Ok(())
    }
    fn domain(&self, vm: &mut Interpreter<'_>, slot: usize) -> Result<(Term, bool), Error> {
        let domain = self
            .domains
            .get(slot)
            .unwrap_or(&Term::Primitive(Primitive::Type));
        let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let expected = nir::type_computations::instantiate_parameters_bounded(
            domain,
            self.parameters,
            &self.types,
            &[],
            &mut remaining,
        );
        vm.budget.steps -= (before - remaining) as u64;
        let expected = expected.map_err(|error| {
            vm.failure(
                if matches!(error, nir::type_computations::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(self.arguments[slot].source()),
            )
        })?;
        let unresolved = self.parameters.iter().enumerate().any(|(index, p)| {
            !self.ready.contains(&index) && expected.mentions_rigid(p.declaration)
        });
        Ok((expected, unresolved))
    }
    fn check_alias_input(
        &self,
        vm: &Interpreter<'_>,
        slot: usize,
        expected: &Term,
    ) -> Result<(), Error> {
        if self.alias
            && let A::Value(value) = self.input(slot)
            && value.ty != *expected
            && value.ty != Term::Primitive(Primitive::Never)
        {
            return Err(vm.failure(ErrorKind::Type, Some(value.source)));
        }
        Ok(())
    }
    fn bind(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
        slot: usize,
        value: Value,
    ) -> Result<(), Error> {
        let source = self.arguments[slot].source();
        let copy = self
            .bindings
            .map(|_| vm.checkpoint_representation(&value, source))
            .transpose()?;
        self.types[slot] = vm.residual_input_term(value, source)?;
        if let Some(copy) = copy {
            environment.insert(self.bindings.unwrap()[slot], copy);
        }
        self.ready.insert(slot);
        Ok(())
    }
    fn drain(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
    ) -> Result<(), Error> {
        loop {
            let mut progress = false;
            vm.integer_operation(Some(self.source), |budget| {
                budget.consume(self.pending.len())
            })?;
            for slot in self.pending.keys().copied().collect::<Vec<_>>() {
                let (expected, unresolved) = self.domain(vm, slot)?;
                if unresolved {
                    continue;
                }
                self.specialize(vm, slot)?;
                self.check_alias_input(vm, slot, &expected)?;
                let mut literals = BTreeMap::new();
                vm.prepare_normal_argument(self.input(slot), &expected, &mut literals, 0)?;
                let staged = self.pending.remove(&slot).unwrap();
                let value = vm.finish_staged_normal_argument(
                    staged,
                    &mut literals,
                    self.arguments[slot].source(),
                    0,
                )?;
                self.bind(vm, environment, slot, value)?;
                progress = true;
            }
            if !progress {
                return Ok(());
            }
        }
    }
    fn next(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
    ) -> Result<normal_plan::Progress, Error> {
        if self.terminal || self.waiting.is_some() {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source)));
        }
        loop {
            if self.drain {
                self.drain = false;
                self.drain(vm, environment)?;
            }
            if let Some(preparation) = self.preparations.get(self.preparation)
                && preparation.before == self.position
            {
                self.waiting = Some(Waiting::Preparation);
                if let Some(frame) = vm.start_input_preparation(preparation, environment)? {
                    return Ok(normal_plan::Progress::Input(frame));
                }
                match vm.eval(&preparation.expression, environment)? {
                    Flow::Value(_) => {
                        self.waiting = None;
                        self.preparation += 1;
                        continue;
                    }
                    flow => return Ok(normal_plan::Progress::Complete(flow)),
                }
            }
            if self.position == self.arguments.len() {
                if self.ready.len() != self.arguments.len() {
                    return Err(vm.failure(ErrorKind::Type, Some(self.source)));
                }
                return Ok(normal_plan::Progress::InputsComplete);
            }
            let slot = self
                .order
                .map_or(self.position, |order| order[self.position]);
            self.specialize(vm, slot)?;
            if self.parameters[slot].sort == nir::Sort::Type
                && let A::Value(value) = self.input(slot)
                && !matches!(
                    value.ty,
                    Term::Primitive(Primitive::Type | Primitive::Never)
                )
            {
                return Err(vm.failure(ErrorKind::Type, Some(value.source)));
            }
            let (expected, unresolved) = self.domain(vm, slot)?;
            if unresolved {
                self.waiting = Some(Waiting::Stage(slot));
                if let Some(frame) = vm.start_staged_input(self.input(slot), environment, 0)? {
                    return Ok(normal_plan::Progress::Input(frame));
                }
                match vm.interpret_staged_argument(self.input(slot), environment, 0)? {
                    std::ops::ControlFlow::Continue(value) => {
                        self.pending.insert(slot, value);
                    }
                    std::ops::ControlFlow::Break(flow) => {
                        return Ok(normal_plan::Progress::Complete(flow));
                    }
                }
            } else {
                self.check_alias_input(vm, slot, &expected)?;
                self.waiting = Some(Waiting::Ready(slot));
                if let Some(frame) =
                    vm.start_contextual_input(self.input(slot), &expected, environment)?
                {
                    return Ok(normal_plan::Progress::Input(frame));
                }
                match vm.interpret_contextual_input(self.input(slot), &expected, environment)? {
                    Flow::Value(value) => self.bind(vm, environment, slot, value)?,
                    flow => return Ok(normal_plan::Progress::Complete(flow)),
                }
            }
            self.waiting = None;
            self.position += 1;
            self.drain = true;
        }
    }
}
impl normal_plan::InputPlan for Frame<'_> {
    fn next_input(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
    ) -> Result<normal_plan::Progress, Error> {
        let result = self.next(vm, environment);
        if !matches!(&result, Ok(normal_plan::Progress::Input(_))) {
            self.terminal = true;
        }
        result
    }
    fn accept_input(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
        result: bytecode::InputResult,
    ) -> Result<Option<Flow>, Error> {
        if self.terminal || self.waiting.is_none() {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source)));
        }
        *environment = result.environment;
        self.terminal = true;
        let value = match result.flow? {
            Flow::Value(value) => value,
            flow => return Ok(Some(flow)),
        };
        match self.waiting.take().unwrap() {
            Waiting::Preparation => self.preparation += 1,
            Waiting::Ready(slot) => {
                self.bind(vm, environment, slot, value)?;
                self.position += 1;
                self.drain = true;
            }
            Waiting::Stage(slot) => {
                let (shape, mut staged) = result
                    .staged
                    .ok_or_else(|| vm.failure(ErrorKind::InvalidControlFlow, Some(self.source)))?;
                vm.rekey_staged_input(self.input(slot), &shape, &mut staged)?;
                self.pending.insert(slot, staged);
                self.position += 1;
                self.drain = true;
            }
        }
        self.terminal = false;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use normal_plan::InputPlan;

    #[test]
    fn captured_type_arriving_last_completes_earlier_staged_input() {
        let source = SourceRange { start: 1, end: 2 };
        let id = |local| DeclarationId { package: 92, local };
        let local = |index| LocalId { owner: 92, index };
        let parameters = vec![
            nir::DeclarationParameter {
                declaration: id(1),
                sort: nir::Sort::ComptimeValue,
            },
            nir::DeclarationParameter {
                declaration: id(2),
                sort: nir::Sort::Type,
            },
        ];
        let domains = vec![
            Term::Rigid {
                declaration: id(2),
                sort: nir::Sort::Type,
            },
            Term::Primitive(Primitive::Type),
        ];
        let expression = Expr {
            source,
            ty: Term::Primitive(Primitive::Type),
            kind: K::NominalType {
                declaration: id(0),
                preparations: vec![],
                order: None,
                bindings: Some(vec![local(0), local(1)]),
                arguments: vec![
                    A::Integer {
                        spelling: "17".into(),
                        negative: false,
                        source,
                    },
                    A::Value(Expr {
                        source,
                        ty: Term::Primitive(Primitive::Type),
                        kind: K::TypeValue(Term::Primitive(Primitive::U8)),
                    }),
                ],
            },
        };
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut frame = Frame::new(&expression, &parameters, &domains, true);
        let mut environment = Environment::new();
        let normal_plan::Progress::Input(bytecode::InputFrame { mut code, finish }) =
            frame.next_input(&mut vm, &mut environment).unwrap()
        else {
            panic!("expected staged input");
        };
        let flow = vm.run_fragment(&mut code);
        let result = finish.complete(&mut vm, code, flow);
        assert!(
            frame
                .accept_input(&mut vm, &mut environment, result)
                .unwrap()
                .is_none()
        );
        assert!(environment.is_empty());
        assert!(frame.to_string().contains("(pending true)"));
        let mut frame = Box::new(frame);
        assert!(
            vm.run_nominal_inputs(&mut frame, &mut environment)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            environment.get(&local(1)),
            Some(&Value::Type(Term::Primitive(Primitive::U8)))
        );
        assert_eq!(vm.bytecode_staged_inputs(), 1);
        assert_eq!(vm.bytecode_contextual_inputs(), 1);
        assert_eq!(
            frame
                .next_input(&mut vm, &mut environment)
                .err()
                .unwrap()
                .kind,
            ErrorKind::InvalidControlFlow
        );
        assert_eq!(
            frame.finish(),
            vec![
                Term::Integer(comptime_runtime::Integer::from_u128(17)),
                Term::Primitive(Primitive::U8)
            ]
        );
    }
}
