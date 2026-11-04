//! Resumable nominal/alias input collection. Admission and alias expansion run
//! only after every input has completed; capture domains use fixed-point wakeup.
use super::*;
use nir::program::{NormalFormArgument as A, NormalFormPreparation};

pub(super) struct Frame<'e> {
    expression: nominal_plan::Expression<'e>,
    specialized: Option<Vec<A>>,
    parameters: Vec<nir::DeclarationParameter>,
    domains: Vec<Term>,
    alias: bool,
    source: SourceRange,
    completed: input_bindings::Bindings,
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
        write!(f, ") {} (pending", self.completed)?;
        for slot in self.pending.keys() {
            write!(f, " (slot {slot})")?;
        }
        write!(f, "))")
    }
}

impl<'e> Frame<'e> {
    pub fn new(
        expression: nominal_plan::Expression<'e>,
        parameters: Vec<nir::DeclarationParameter>,
        domains: Vec<Term>,
        specialize: bool,
    ) -> Self {
        let arguments = Self::arguments_of(&expression);
        Self {
            specialized: specialize.then(|| arguments.to_vec()),
            alias: matches!(expression.kind, K::CheckTypeAlias { .. }),
            source: expression.source,
            completed: input_bindings::Bindings::new(parameters.len()),
            expression,
            parameters,
            domains,
            pending: BTreeMap::new(),
            position: 0,
            preparation: 0,
            waiting: None,
            drain: false,
            terminal: false,
        }
    }
    fn arguments_of(expression: &Expr) -> &[A] {
        match &expression.kind {
            K::NominalType { arguments, .. } | K::CheckTypeAlias { arguments, .. } => arguments,
            _ => unreachable!(),
        }
    }
    fn arguments(&self) -> &[A] {
        Self::arguments_of(&self.expression)
    }
    fn bindings(&self) -> Option<&[LocalId]> {
        match &self.expression.kind {
            K::NominalType { bindings, .. } | K::CheckTypeAlias { bindings, .. } => {
                bindings.as_deref()
            }
            _ => unreachable!(),
        }
    }
    fn order(&self) -> Option<&[usize]> {
        match &self.expression.kind {
            K::NominalType { order, .. } | K::CheckTypeAlias { order, .. } => order.as_deref(),
            _ => unreachable!(),
        }
    }
    fn preparations(&self) -> &[NormalFormPreparation] {
        match &self.expression.kind {
            K::NominalType { preparations, .. } | K::CheckTypeAlias { preparations, .. } => {
                preparations
            }
            _ => unreachable!(),
        }
    }
    pub fn finish_parts(self) -> Option<(Vec<nir::DeclarationParameter>, Vec<Term>)> {
        Some((self.parameters, self.completed.finish()?))
    }
    #[cfg(test)]
    pub fn finish(self) -> Option<Vec<Term>> {
        self.completed.finish()
    }
    fn input(&self, slot: usize) -> &A {
        self.specialized
            .as_ref()
            .map_or(&self.arguments()[slot], |inputs| &inputs[slot])
    }
    fn instantiate(
        &self,
        vm: &mut Interpreter<'_>,
        term: &Term,
        source: SourceRange,
    ) -> Result<Term, Error> {
        vm.integer_operation(Some(source), |budget| budget.consume(self.completed.len()))?;
        if self
            .completed
            .parameters
            .iter()
            .zip(&self.completed.arguments)
            .all(|(parameter, argument)| parameter.sort == argument.sort())
        {
            return vm.instantiate_contextual_domain(
                term,
                &self.completed.parameters,
                &self.completed.arguments,
                source,
            );
        }
        let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let result = (|| -> Result<Term, nir::NominalAdmissionError> {
            remaining = remaining
                .checked_sub(self.completed.slot_count())
                .ok_or(nir::NominalAdmissionError::Budget)?;
            let mut ids = Vec::new();
            let mut domains = Vec::new();
            let mut arguments = Vec::new();
            for (parameter, domain, argument) in self.completed.completed() {
                nir::type_computations::charge_term(domain, &mut remaining)?;
                nir::type_computations::charge_term(argument, &mut remaining)?;
                ids.push(parameter.declaration);
                domains.push(domain.clone());
                arguments.push(argument.clone());
            }
            let admitted = nir::admit_declaration_arguments_with_evidence_in(
                &ids,
                &domains,
                &arguments,
                &mut remaining,
                |ty, variant, fuel| vm.query_normal_value_fields(ty, variant, fuel),
                |value, domain, _| {
                    matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue}
                    if vm.abstract_value_parameters.get(declaration) == Some(domain))
                },
                vm.target,
            )?;
            admitted
                .instantiate(term, &mut remaining)
                .map_err(Into::into)
        })();
        vm.budget.steps -= (before - remaining) as u64;
        result.map_err(|error| {
            vm.failure(
                match error {
                    nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
                    nir::NominalAdmissionError::Unresolved(_) => ErrorKind::TypeContractUnresolved,
                    nir::NominalAdmissionError::Invalid(_) => ErrorKind::Type,
                },
                Some(source),
            )
        })
    }
    fn specialize(&mut self, vm: &mut Interpreter<'_>, slot: usize) -> Result<(), Error> {
        let Some(mut inputs) = self.specialized.take() else {
            return Ok(());
        };
        // Moving the vector does not move its heap storage: pending staged keys
        // continue to refer to the same argument nodes during specialization.
        let source = inputs[slot].source();
        let result = (|| {
            let mut rewrite = |ty: &Term| self.instantiate(vm, ty, source);
            for pattern in inputs[slot].patterns_mut() {
                pattern.rewrite_types(&mut rewrite)?;
            }
            for expression in inputs[slot].expressions_mut() {
                expression.rewrite_types(&mut rewrite)?;
            }
            Ok(())
        })();
        self.specialized = Some(inputs);
        result
    }
    fn domain(&self, vm: &mut Interpreter<'_>, slot: usize) -> Result<(Term, bool), Error> {
        let domain = self
            .domains
            .get(slot)
            .unwrap_or(&Term::Primitive(Primitive::Type));
        let expected = self.instantiate(vm, domain, self.arguments()[slot].source())?;
        let source = self.arguments()[slot].source();
        let inputs = vm.observe_input_dependencies(&expected, source)?;
        vm.integer_operation(Some(source), |budget| budget.consume(self.parameters.len()))?;
        let unresolved = self.parameters.iter().enumerate().any(|(index, p)| {
            !self.completed.contains(index) && inputs.declarations.contains_key(&p.declaration)
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
        let source = self.arguments()[slot].source();
        let copy = self
            .bindings()
            .map(|_| vm.checkpoint_representation(&value, source))
            .transpose()?;
        let domain = self
            .domains
            .get(slot)
            .unwrap_or(&Term::Primitive(Primitive::Type));
        self.completed.bind(
            vm,
            slot,
            self.parameters[slot].clone(),
            domain,
            value,
            source,
        )?;
        if let Some(copy) = copy {
            environment.insert(self.bindings().unwrap()[slot], copy);
        }
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
                    self.arguments()[slot].source(),
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
            if let Some(preparation) = self.preparations().get(self.preparation)
                && preparation.before == self.position
            {
                if let Some(frame) = vm.start_input_preparation(preparation, environment)? {
                    self.waiting = Some(Waiting::Preparation);
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
            if self.position == self.arguments().len() {
                if self.completed.len() != self.arguments().len() {
                    return Err(vm.failure(ErrorKind::Type, Some(self.source)));
                }
                return Ok(normal_plan::Progress::InputsComplete);
            }
            let slot = self
                .order()
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
    fn cleanup(&self, environment: &mut Environment) {
        for binding in self
            .expression
            .kind
            .nominal_local_bindings()
            .expect("validated input plan")
        {
            environment.remove(&binding);
        }
    }
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
                vm.rekey_staged_input(&shape, &mut staged)?;
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
        let mut frame = Frame::new(
            nominal_plan::Expression::Owned(Arc::new(expression)),
            parameters,
            domains,
            true,
        );
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
        assert!(frame.to_string().contains("(pending (slot 0))"));
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
            frame.finish().unwrap(),
            vec![
                Term::Integer(comptime_runtime::Integer::from_u128(17)),
                Term::Primitive(Primitive::U8)
            ]
        );
    }

    #[test]
    fn completed_inputs_retain_dynamic_sort_admission_and_abstract_evidence() {
        let source = SourceRange { start: 1, end: 2 };
        let id = |local| DeclarationId { package: 92, local };
        let rigid = |local, sort| Term::Rigid {
            declaration: id(local),
            sort,
        };
        let ty = Term::Primitive(Primitive::Type);
        let boolean = Term::Primitive(Primitive::Bool);
        let parameters = vec![
            nir::DeclarationParameter {
                declaration: id(1),
                sort: nir::Sort::Type,
            },
            nir::DeclarationParameter {
                declaration: id(2),
                sort: nir::Sort::ComptimeValue,
            },
            nir::DeclarationParameter {
                declaration: id(3),
                sort: nir::Sort::ComptimeValue,
            },
        ];
        let domains = vec![ty.clone(), rigid(1, nir::Sort::Type), boolean.clone()];
        let expression = Arc::new(Expr {
            source,
            ty: ty.clone(),
            kind: K::NominalType {
                declaration: id(0),
                preparations: vec![],
                order: None,
                bindings: None,
                arguments: (0..3)
                    .map(|_| {
                        A::Value(Expr {
                            source,
                            ty: ty.clone(),
                            kind: K::TypeValue(ty.clone()),
                        })
                    })
                    .collect(),
            },
        });
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.authorize_abstract_value_parameters(&[(id(4), boolean.clone())])
            .unwrap();
        for valid in [false, true] {
            let mut frame = Frame::new(
                nominal_plan::Expression::Owned(expression.clone()),
                parameters.clone(),
                domains.clone(),
                false,
            );
            let mut environment = Environment::new();
            // Source completion order differs from the declaration telescope.
            let value = if valid {
                Value::AbstractValue {
                    declaration: id(4),
                    ty: boolean.clone(),
                }
            } else {
                Value::UnsignedInteger(7)
            };
            frame.bind(&mut vm, &mut environment, 2, value).unwrap();
            let (domain, unresolved) = frame.domain(&mut vm, 1).unwrap();
            assert_eq!(domain, rigid(1, nir::Sort::Type));
            assert!(unresolved);
            frame
                .bind(&mut vm, &mut environment, 0, Value::Type(ty.clone()))
                .unwrap();
            frame
                .bind(
                    &mut vm,
                    &mut environment,
                    1,
                    Value::Type(Term::Primitive(Primitive::U8)),
                )
                .unwrap();
            let result = frame.instantiate(&mut vm, &rigid(2, nir::Sort::ComptimeValue), source);
            if valid {
                assert_eq!(result.unwrap(), Term::Primitive(Primitive::U8));
                assert_eq!(
                    frame.finish().unwrap(),
                    vec![
                        ty.clone(),
                        Term::Primitive(Primitive::U8),
                        rigid(4, nir::Sort::ComptimeValue)
                    ]
                );
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            }
        }
    }
}
