//! NF input contracts use the shared invocation input scheduler.
use super::*;
use input_collection::Progress;
use nir::program::{NormalFormArgument as Argument, NormalFormInputPlan, NormalFormPreparation};

enum Inputs<'a> {
    Borrowed(&'a [Argument], &'a NormalFormInputPlan),
    Owned(Arc<Expr>),
}
impl Inputs<'_> {
    fn parts(&self) -> (&[Argument], &NormalFormInputPlan) {
        match self {
            Self::Borrowed(arguments, plan) => (arguments, plan),
            Self::Owned(expression) => match &expression.kind {
                K::NormalFormType {
                    arguments,
                    plan: Some(plan),
                    ..
                } => (arguments, plan),
                _ => unreachable!("validated normal plan"),
            },
        }
    }
}

pub(super) struct Source<'a> {
    head: DeclarationId,
    source: SourceRange,
    inputs: Inputs<'a>,
    specialized: BTreeMap<usize, Box<Argument>>,
    bindings: input_bindings::Bindings,
}

pub(super) type Frame<'a> = input_collection::Frame<Source<'a>>;

impl fmt::Display for Source<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(normal-input-source (head {}) {})",
            self.head, self.bindings
        )
    }
}

impl<'a> Frame<'a> {
    fn new(
        head: DeclarationId,
        source: SourceRange,
        arguments: &'a [Argument],
        plan: &'a NormalFormInputPlan,
    ) -> Self {
        Self::from_source(Source {
            head,
            source,
            inputs: Inputs::Borrowed(arguments, plan),
            specialized: BTreeMap::new(),
            bindings: input_bindings::Bindings::new(arguments.len()),
        })
    }

    pub(super) fn owned(
        vm: &mut Interpreter<'_>,
        expression: Arc<Expr>,
        environment: &Environment,
    ) -> Result<Frame<'static>, Error> {
        let K::NormalFormType {
            head,
            arguments,
            plan: Some(plan),
        } = &expression.kind
        else {
            return Err(vm.failure(ErrorKind::Type, Some(expression.source)));
        };
        if !plan.valid(arguments.len())
            || plan
                .local_bindings()
                .unwrap()
                .iter()
                .any(|id| environment.contains_key(id))
        {
            return Err(vm.failure(ErrorKind::Type, Some(expression.source)));
        }
        let diverges = arguments.iter().any(Argument::is_diverging)
            || plan
                .preparations
                .iter()
                .any(|p| p.expression.ty == Term::Primitive(Primitive::Never));
        vm.prepare_normal_type(
            *head,
            &expression.ty,
            arguments.len(),
            diverges,
            expression.source,
        )?;
        nir::copy_budget::charge_type_rewrite(&expression, &mut vm.budget.steps).map_err(
            |error| {
                vm.failure(
                    match error {
                        nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
                        nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
                    },
                    Some(expression.source),
                )
            },
        )?;
        Ok(Frame::from_source(Source {
            head: *head,
            source: expression.source,
            bindings: input_bindings::Bindings::new(arguments.len()),
            inputs: Inputs::Owned(expression),
            specialized: BTreeMap::new(),
        }))
    }
}

impl input_collection::Source for Source<'_> {
    fn source(&self) -> SourceRange {
        self.source
    }
    fn preparation(&self, index: usize) -> Option<&NormalFormPreparation> {
        self.inputs.parts().1.preparations.get(index)
    }
    fn slot(&self, position: usize) -> Option<usize> {
        self.inputs.parts().1.order.get(position).copied()
    }
    fn input(&self, slot: usize) -> &Argument {
        self.specialized
            .get(&slot)
            .map_or(&self.inputs.parts().0[slot], |input| input)
    }
    fn specialize(&mut self, vm: &mut Interpreter<'_>, slot: usize) -> Result<(), Error> {
        // Entry admission has charged the input copies. Retain each specialized
        // argument while suspended; completing its domain never copies producers.
        let mut argument = self
            .specialized
            .remove(&slot)
            .unwrap_or_else(|| Box::new(self.inputs.parts().0[slot].clone()));
        let result = self.bindings.specialize(vm, &mut argument);
        self.specialized.insert(slot, argument);
        result
    }
    fn check_input(
        &self,
        _vm: &Interpreter<'_>,
        _slot: usize,
        _expected: Option<&Term>,
    ) -> Result<(), Error> {
        Ok(())
    }
    fn domain(&self, vm: &mut Interpreter<'_>, slot: usize) -> Result<(Term, bool), Error> {
        let metadata = vm
            .normal_forms
            .get(&self.head)
            .expect("admitted normal head");
        let expected = self
            .bindings
            .instantiate(vm, &metadata.parameters[slot].ty, self.source)?;
        let inputs = vm.observe_input_dependencies(&expected, self.source)?;
        vm.integer_operation(Some(self.source), |budget| {
            budget.consume(metadata.parameters.len())
        })?;
        let unresolved = metadata
            .parameters
            .iter()
            .any(|p| inputs.declarations.contains_key(&p.declaration));
        Ok((expected, unresolved))
    }

    fn bind(
        &mut self,
        vm: &mut Interpreter<'_>,
        env: &mut Environment,
        slot: usize,
        value: Value,
    ) -> Result<(), Error> {
        let source = self.input(slot).source();
        let copy = vm.checkpoint_representation(&value, source)?;
        vm.bind_normal_input(self.head, &mut self.bindings, slot, value, source)?;
        env.insert(self.inputs.parts().1.bindings[slot], copy);
        self.specialized.remove(&slot);
        Ok(())
    }

    fn complete(&mut self, vm: &mut Interpreter<'_>) -> Result<Progress, Error> {
        let bindings = std::mem::replace(&mut self.bindings, input_bindings::Bindings::new(0));
        let arguments = bindings
            .finish()
            .ok_or_else(|| vm.failure(ErrorKind::Type, Some(self.source)))?;
        Ok(Progress::Complete(Flow::Value(vm.complete_normal_type(
            self.head,
            arguments,
            self.source,
        )?)))
    }
    fn cleanup(&self, environment: &mut Environment) {
        for binding in self
            .inputs
            .parts()
            .1
            .local_bindings()
            .expect("validated plan")
        {
            environment.remove(&binding);
        }
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
    use input_collection::{Completion, InputPlan, Request};
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
        assert!(!failed.source.bindings.contains(0));
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
        let values = frame.source.bindings.finish().unwrap();
        assert_eq!(values[0], Term::Primitive(Primitive::U8));
        assert!(
            matches!(vm.complete_normal_type(id(0), values, SOURCE).unwrap(), Value::Type(Term::NormalForm { head, .. }) if head == id(0))
        );
    }
}
