//! Resumable normal-form input scheduling. A request never executes user code;
//! the driver supplies exactly one completion before the plan can advance.
use super::*;
use nir::program::{NormalFormArgument as Argument, NormalFormInputPlan, NormalFormPreparation};
use normal_inputs::StagedNormalInput;

struct Pending {
    argument: Box<Argument>,
    staged: StagedNormalInput,
}

enum Waiting {
    Preparation(usize),
    Ready {
        slot: usize,
        argument: Box<Argument>,
        expected: Term,
    },
    Stage {
        slot: usize,
        argument: Box<Argument>,
    },
}

enum Request<'a> {
    Preparation(&'a NormalFormPreparation),
    Ready(&'a Argument, &'a Term),
    Stage(&'a Argument),
    Complete,
}

enum Completion {
    Prepared,
    Value(Value),
    Staged(StagedNormalInput),
}

pub(super) struct Frame<'a> {
    head: DeclarationId,
    source: SourceRange,
    arguments: &'a [Argument],
    plan: &'a NormalFormInputPlan,
    position: usize,
    preparation: usize,
    waiting: Option<Waiting>,
    pending: BTreeMap<usize, Pending>,
    bindings: normal_types::Bindings,
    drain: bool,
    terminal: bool,
}

impl fmt::Display for Frame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(normal-plan-frame (head {}) (position {}) (preparation {}) (terminal {}) (drain {}) {} (waiting",
            self.head, self.position, self.preparation, self.terminal, self.drain, self.bindings
        )?;
        match &self.waiting {
            Some(Waiting::Preparation(index)) => write!(f, " (preparation {index})")?,
            Some(Waiting::Ready {
                slot,
                argument,
                expected,
            }) => write!(f, " (ready {slot} {expected} {argument})")?,
            Some(Waiting::Stage { slot, argument }) => write!(f, " (stage {slot} {argument})")?,
            None => {}
        }
        write!(f, ") (pending")?;
        for (slot, pending) in &self.pending {
            write!(f, " (input {slot} {})", pending.argument)?;
        }
        write!(f, "))")
    }
}

pub(super) enum Progress {
    Input(bytecode::InputFrame),
    Complete(Flow),
}

impl<'a> Frame<'a> {
    fn new(
        head: DeclarationId,
        source: SourceRange,
        arguments: &'a [Argument],
        plan: &'a NormalFormInputPlan,
    ) -> Self {
        Self {
            head,
            source,
            arguments,
            plan,
            position: 0,
            preparation: 0,
            waiting: None,
            pending: BTreeMap::new(),
            bindings: normal_types::Bindings::new(arguments.len()),
            drain: false,
            terminal: false,
        }
    }

    pub(super) fn next_input(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
    ) -> Result<Progress, Error> {
        let result = self.next_input_inner(vm, environment);
        if matches!(&result, Err(_) | Ok(Progress::Complete(_))) {
            self.terminal = true;
        }
        result
    }

    fn next_input_inner(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
    ) -> Result<Progress, Error> {
        loop {
            let completion = match self.advance(vm, environment)? {
                Request::Preparation(preparation) => {
                    if let Some(frame) = vm.start_input_preparation(preparation, environment)? {
                        return Ok(Progress::Input(frame));
                    }
                    match vm.eval(&preparation.expression, environment)? {
                        Flow::Value(_) => Completion::Prepared,
                        flow => return Ok(Progress::Complete(flow)),
                    }
                }
                Request::Ready(argument, expected) => {
                    if let Some(frame) =
                        vm.start_contextual_input(argument, expected, environment)?
                    {
                        return Ok(Progress::Input(frame));
                    }
                    match vm.interpret_contextual_input(argument, expected, environment)? {
                        Flow::Value(value) => Completion::Value(value),
                        flow => return Ok(Progress::Complete(flow)),
                    }
                }
                Request::Stage(argument) => {
                    if let Some(frame) = vm.start_staged_input(argument, environment, 0)? {
                        return Ok(Progress::Input(frame));
                    }
                    match vm.interpret_staged_argument(argument, environment, 0)? {
                        std::ops::ControlFlow::Continue(value) => Completion::Staged(value),
                        std::ops::ControlFlow::Break(flow) => return Ok(Progress::Complete(flow)),
                    }
                }
                Request::Complete => {
                    let bindings =
                        std::mem::replace(&mut self.bindings, normal_types::Bindings::new(0));
                    let arguments = bindings
                        .finish()
                        .ok_or_else(|| vm.failure(ErrorKind::Type, Some(self.source)))?;
                    return Ok(Progress::Complete(Flow::Value(vm.complete_normal_type(
                        self.head,
                        arguments,
                        self.source,
                    )?)));
                }
            };
            self.receive(vm, environment, completion)?;
        }
    }

    pub(super) fn accept_input(
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
        let completion = match result.flow? {
            Flow::Value(value) => match &self.waiting {
                Some(Waiting::Preparation(_)) => Completion::Prepared,
                Some(Waiting::Ready { .. }) => Completion::Value(value),
                Some(Waiting::Stage { argument, .. }) => {
                    let (shape, mut value) = result.staged.ok_or_else(|| {
                        vm.failure(ErrorKind::InvalidControlFlow, Some(self.source))
                    })?;
                    vm.rekey_staged_input(argument, &shape, &mut value)?;
                    Completion::Staged(value)
                }
                None => return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source))),
            },
            flow => return Ok(Some(flow)),
        };
        self.receive(vm, environment, completion)?;
        Ok(None)
    }

    fn domain(&self, vm: &mut Interpreter<'_>, slot: usize) -> Result<(Term, bool), Error> {
        let metadata = vm.normal_forms[&self.head];
        let expected = vm.instantiate_contextual_domain(
            &metadata.parameters[slot].ty,
            &self.bindings.parameters,
            &self.bindings.arguments,
            self.source,
        )?;
        let unresolved = metadata
            .parameters
            .iter()
            .any(|p| expected.mentions_rigid(p.declaration));
        Ok((expected, unresolved))
    }

    fn bind(
        &mut self,
        vm: &mut Interpreter<'_>,
        env: &mut Environment,
        slot: usize,
        value: Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        let copy = vm.checkpoint_representation(&value, source)?;
        vm.bind_normal_input(self.head, &mut self.bindings, slot, value, source)?;
        env.insert(self.plan.bindings[slot], copy);
        Ok(())
    }

    fn advance(
        &mut self,
        vm: &mut Interpreter<'_>,
        env: &mut Environment,
    ) -> Result<Request<'_>, Error> {
        if self.waiting.is_some() || self.terminal {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source)));
        }
        // Semantic failures are terminal: resuming must never replay an operand.
        self.terminal = true;
        if self.drain {
            self.drain = false;
            for slot in self.pending.keys().copied().collect::<Vec<_>>() {
                vm.tick(Some(self.source))?;
                let (expected, unresolved) = self.domain(vm, slot)?;
                if unresolved {
                    continue;
                }
                let Pending {
                    mut argument,
                    staged,
                } = self.pending.remove(&slot).unwrap();
                vm.specialize_contextual_input(
                    &mut argument,
                    &self.bindings.parameters,
                    &self.bindings.arguments,
                )?;
                let mut literals = BTreeMap::new();
                vm.prepare_normal_argument(&argument, &expected, &mut literals, 0)?;
                let value =
                    vm.finish_staged_normal_argument(staged, &mut literals, argument.source(), 0)?;
                self.bind(vm, env, slot, value, argument.source())?;
            }
        }
        if let Some(preparation) = self.plan.preparations.get(self.preparation)
            && preparation.before == self.position
        {
            self.waiting = Some(Waiting::Preparation(self.preparation));
        } else if let Some(&slot) = self.plan.order.get(self.position) {
            let mut argument = Box::new(self.arguments[slot].clone());
            vm.specialize_contextual_input(
                &mut argument,
                &self.bindings.parameters,
                &self.bindings.arguments,
            )?;
            let (expected, unresolved) = self.domain(vm, slot)?;
            self.waiting = Some(if unresolved {
                Waiting::Stage { slot, argument }
            } else {
                Waiting::Ready {
                    slot,
                    argument,
                    expected,
                }
            });
        } else {
            self.terminal = true;
            return Ok(Request::Complete);
        }
        self.terminal = false;
        Ok(match self.waiting.as_ref().unwrap() {
            Waiting::Preparation(index) => Request::Preparation(&self.plan.preparations[*index]),
            Waiting::Ready {
                argument, expected, ..
            } => Request::Ready(argument, expected),
            Waiting::Stage { argument, .. } => Request::Stage(argument),
        })
    }

    fn receive(
        &mut self,
        vm: &mut Interpreter<'_>,
        env: &mut Environment,
        completion: Completion,
    ) -> Result<(), Error> {
        if !matches!(
            (&self.waiting, &completion),
            (Some(Waiting::Preparation(_)), Completion::Prepared)
                | (Some(Waiting::Ready { .. }), Completion::Value(_))
                | (Some(Waiting::Stage { .. }), Completion::Staged(_))
        ) {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source)));
        }
        self.terminal = true;
        match (self.waiting.take().unwrap(), completion) {
            (Waiting::Preparation(_), Completion::Prepared) => self.preparation += 1,
            (Waiting::Ready { slot, argument, .. }, Completion::Value(value)) => {
                self.bind(vm, env, slot, value, argument.source())?;
                self.position += 1;
                self.drain = true;
            }
            (Waiting::Stage { slot, argument }, Completion::Staged(staged)) => {
                self.pending.insert(slot, Pending { argument, staged });
                self.position += 1;
            }
            _ => unreachable!(),
        }
        self.terminal = false;
        Ok(())
    }
}

impl Interpreter<'_> {
    pub(super) fn eval_normal_plan(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::NormalFormType {
            head,
            arguments,
            plan: Some(plan),
        } = &expr.kind
        else {
            unreachable!()
        };
        nir::copy_budget::charge_type_rewrite(expr, &mut self.budget.steps).map_err(|error| {
            self.failure(
                match error {
                    nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
                    nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
                },
                Some(expr.source),
            )
        })?;
        let mut frame = Frame::new(*head, expr.source, arguments, plan);
        self.run_normal_plan(&mut frame, environment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 81, local }
    }
    fn local(index: u32) -> LocalId {
        LocalId { owner: 81, index }
    }
    fn expr(ty: Primitive, kind: K) -> Expr {
        Expr {
            ty: Term::Primitive(ty),
            kind,
            source: SOURCE,
        }
    }

    #[test]
    fn suspended_plan_resumes_once_and_retains_pending_literal_identity() {
        let package = Package {
            normal_forms: vec![Head {
                declaration: id(0),
                collector: None,
                role: Role::Opaque,
                requirements: vec![],
                parameters: vec![
                    Parameter {
                        declaration: id(1),
                        binding: local(0),
                        kind: Kind::Type,
                        ty: Term::Primitive(Primitive::Type),
                    },
                    Parameter {
                        declaration: id(2),
                        binding: local(1),
                        kind: Kind::Value,
                        ty: Term::Rigid {
                            declaration: id(1),
                            sort: nir::Sort::Type,
                        },
                    },
                ],
            }],
            ..Default::default()
        };
        let arguments = vec![
            Argument::Value(expr(
                Primitive::Type,
                K::TypeValue(Term::Primitive(Primitive::U8)),
            )),
            Argument::Integer {
                spelling: "7".into(),
                negative: false,
                source: SOURCE,
            },
        ];
        let plan = NormalFormInputPlan {
            order: vec![1, 0],
            bindings: vec![local(10), local(11)],
            preparations: vec![],
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        let ordered = NormalFormInputPlan {
            order: vec![0, 1],
            ..plan.clone()
        };
        let mut failed = Frame::new(id(0), SOURCE, &arguments, &ordered);
        assert!(matches!(
            failed.advance(&mut vm, &mut env).unwrap(),
            Request::Ready(..)
        ));
        vm.budget.steps = 0;
        assert_eq!(
            failed
                .receive(
                    &mut vm,
                    &mut env,
                    Completion::Value(Value::Type(Term::Primitive(Primitive::U8)))
                )
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert!(env.is_empty());
        assert!(!failed.bindings.contains(0));
        assert_eq!(
            failed.advance(&mut vm, &mut env).err().unwrap().kind,
            ErrorKind::InvalidControlFlow
        );
        vm.budget = Budget::default();
        let mut failed = Frame::new(id(0), SOURCE, &arguments, &ordered);
        env.insert(local(90), Value::Bool(true));
        let Progress::Input(bytecode::InputFrame { mut code, finish }) =
            failed.next_input(&mut vm, &mut env).unwrap()
        else {
            panic!("expected compiled input");
        };
        vm.budget.steps = 0;
        let flow = vm.run_fragment(&mut code);
        let result = finish.complete(&mut vm, code, flow);
        assert_eq!(
            failed
                .accept_input(&mut vm, &mut env, result)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(env.remove(&local(90)), Some(Value::Bool(true)));
        assert!(env.is_empty());
        assert_eq!(
            failed.next_input(&mut vm, &mut env).err().unwrap().kind,
            ErrorKind::InvalidControlFlow
        );
        vm.budget = Budget::default();
        let mut frame = Frame::new(id(0), SOURCE, &arguments, &plan);
        let Request::Stage(argument) = frame.advance(&mut vm, &mut env).unwrap() else {
            panic!("expected deferred input");
        };
        let std::ops::ControlFlow::Continue(staged) =
            vm.stage_normal_argument(argument, &mut env, 0).unwrap()
        else {
            panic!("unexpected transfer");
        };
        assert_eq!(
            frame.advance(&mut vm, &mut env).err().unwrap().kind,
            ErrorKind::InvalidControlFlow
        );
        assert_eq!(
            frame
                .receive(&mut vm, &mut env, Completion::Prepared)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidControlFlow
        );
        frame
            .receive(&mut vm, &mut env, Completion::Staged(staged))
            .unwrap();
        assert!(env.is_empty());
        assert!(frame.to_string().contains("(pending (input 1"));

        // Moving a suspended frame must not invalidate its staged literal keys.
        let mut frame = Box::new(frame);
        let Request::Ready(argument, expected) = frame.advance(&mut vm, &mut env).unwrap() else {
            panic!("expected Type input");
        };
        let Flow::Value(value) = vm
            .evaluate_contextual_argument(argument, expected, &mut env)
            .unwrap()
        else {
            panic!("unexpected transfer");
        };
        frame
            .receive(&mut vm, &mut env, Completion::Value(value))
            .unwrap();
        assert_eq!(
            frame
                .receive(&mut vm, &mut env, Completion::Prepared)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidControlFlow
        );
        assert!(matches!(
            frame.advance(&mut vm, &mut env).unwrap(),
            Request::Complete
        ));
        assert_eq!(
            env.get(&local(10)),
            Some(&Value::Type(Term::Primitive(Primitive::U8)))
        );
        assert!(env.contains_key(&local(11)));
        assert!(frame.to_string().contains("(terminal true)"));
        assert_eq!(
            frame.advance(&mut vm, &mut env).err().unwrap().kind,
            ErrorKind::InvalidControlFlow
        );
        let values = frame.bindings.finish().unwrap();
        assert_eq!(values[0], Term::Primitive(Primitive::U8));
        assert!(
            matches!(vm.complete_normal_type(id(0), values, SOURCE).unwrap(), Value::Type(Term::NormalForm { head, .. }) if head == id(0))
        );
    }
}
