//! NF input contracts use the shared invocation input scheduler.
use super::*;
use input_collection::{Plan, Progress, Target};
use nir::program::NormalFormArgument as Argument;

pub(super) struct Source<'a> {
    head: DeclarationId,
    result: Term,
    signature_result: Option<Term>,
    domains: Domains<'a>,
    locals: Arc<[LocalId]>,
    plan: Plan<'a>,
    bindings: input_bindings::Bindings,
}

/// Public callable slots have invocation-local identities. The selected head
/// retains its own declaration identities for final formation and contracts.
pub(super) enum Domains<'a> {
    Declaration(&'a [nir::normal_forms::Parameter]),
    Callable(Vec<(DeclarationId, nir::Sort, Term)>),
}

impl Domains<'_> {
    fn parameter(&self, slot: usize) -> Option<(nir::DeclarationParameter, &Term)> {
        match self {
            Self::Callable(inputs) => inputs.get(slot).map(|(id, sort, domain)| {
                (
                    nir::DeclarationParameter {
                        declaration: *id,
                        sort: *sort,
                    },
                    domain,
                )
            }),
            Self::Declaration(inputs) => inputs.get(slot).map(|p| {
                let sort = match p.kind {
                    nir::normal_forms::Kind::Type => nir::Sort::Type,
                    nir::normal_forms::Kind::Value => nir::Sort::ComptimeValue,
                };
                (
                    nir::DeclarationParameter {
                        declaration: p.declaration,
                        sort,
                    },
                    &p.ty,
                )
            }),
        }
    }

    pub(super) fn bind(
        &self,
        vm: &mut Interpreter<'_>,
        bindings: &mut input_bindings::Bindings,
        slot: usize,
        value: Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        let (parameter, domain) = self
            .parameter(slot)
            .ok_or_else(|| vm.failure(ErrorKind::Type, Some(source)))?;
        bindings.bind(vm, slot, parameter, domain, value, source)
    }
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
    #[cfg(test)]
    pub(super) fn new<'world: 'a>(
        vm: &mut Interpreter<'world>,
        plan: Plan<'a>,
        environment: &Environment,
    ) -> Result<Self, Error> {
        Self::application(vm, plan, environment, None)
    }

    pub(super) fn application<'world: 'a>(
        vm: &mut Interpreter<'world>,
        plan: Plan<'a>,
        environment: &Environment,
        selected: Option<Value>,
    ) -> Result<Self, Error> {
        let view = plan.view();
        let source = view.source;
        let (head, public_inputs, signature_result) = match (view.target, selected) {
            (Target::Normal(head), None) => (head, None, None),
            (
                Target::Selected {
                    signature,
                    source: callee_source,
                    input_ids,
                },
                Some(value),
            ) => {
                vm.check_value_type(&value, signature, Some(callee_source))?;
                let Value::Function(head) = value else {
                    return Err(vm.failure(ErrorKind::Type, Some(callee_source)));
                };
                let mut fuel = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
                let before = fuel;
                let opened = nir::normal_forms::open_signature(signature, input_ids, &mut fuel);
                vm.budget.steps -= (before - fuel) as u64;
                let (opened, opened_result) = opened.map_err(|reason| {
                    vm.failure(
                        match reason {
                            nir::type_computations::Error::Budget => ErrorKind::StepsExhausted,
                            _ => ErrorKind::Type,
                        },
                        Some(source),
                    )
                })?;
                (head, Some(opened), Some(opened_result))
            }
            _ => return Err(vm.failure(ErrorKind::Type, Some(source))),
        };
        let locals = plan.admit_scope(vm, environment)?;
        let metadata = vm
            .normal_forms
            .get(&head)
            .ok_or_else(|| vm.failure(ErrorKind::Type, Some(source)))?;
        let mut signature_result = signature_result;
        let count = view.arguments.len();
        let diverges = plan.inputs_diverge();
        if diverges && *view.ty != Term::Primitive(Primitive::Never) {
            return Err(vm.failure(ErrorKind::Type, Some(source)));
        }
        if !diverges {
            if view.ty.sort() != nir::Sort::Type {
                return Err(vm.failure(ErrorKind::Type, Some(source)));
            }
            if *view.ty != Term::Primitive(Primitive::Type)
                && (!matches!(metadata.role, nir::normal_forms::Role::Opaque)
                    || (public_inputs.is_none() && vm.functions.declaration(&head).is_none()))
            {
                return Err(vm.failure(ErrorKind::Type, Some(source)));
            }
            if public_inputs.is_none()
                && let Some(function) = vm.functions.declaration(&head)
                && let Term::Function { result, .. } = &function.signature
                && result.check_scope(&[]).is_ok()
            {
                let mut referenced = BTreeSet::new();
                let mut fuel = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
                let before = fuel;
                let checked = nir::type_computations::rigid_declarations_bounded(
                    result,
                    &mut fuel,
                    &mut referenced,
                );
                let copied = match checked {
                    Ok(()) => {
                        nir::copy_budget::clone_term(result, &mut fuel).map_err(|limit| match limit
                        {
                            nir::copy_budget::CopyBudgetExhausted::Steps => {
                                nir::type_computations::Error::Budget
                            }
                            nir::copy_budget::CopyBudgetExhausted::Depth => {
                                nir::type_computations::Error::Budget
                            }
                        })
                    }
                    Err(reason) => Err(reason),
                };
                vm.budget.steps -= (before - fuel) as u64;
                let expected_result = copied.map_err(|reason| {
                    vm.failure(
                        match reason {
                            nir::type_computations::Error::Budget => ErrorKind::StepsExhausted,
                            _ => ErrorKind::Type,
                        },
                        Some(source),
                    )
                })?;
                if metadata
                    .parameters
                    .iter()
                    .all(|parameter| !referenced.contains(&parameter.declaration))
                {
                    if *view.ty != expected_result {
                        return Err(vm.failure(ErrorKind::Type, Some(source)));
                    }
                } else {
                    signature_result = Some(expected_result);
                }
            }
        }
        if metadata.parameters.len() != count
            || public_inputs
                .as_ref()
                .is_some_and(|inputs| inputs.len() != count)
        {
            return Err(vm.failure(ErrorKind::Type, Some(source)));
        }
        vm.integer_operation(Some(source), |budget| budget.consume(count))?;
        let domains = match public_inputs {
            Some(inputs) => Domains::Callable(inputs),
            None => Domains::Declaration(&metadata.parameters),
        };
        let bindings = input_bindings::Bindings::structural(
            (0..count).map(|i| domains.parameter(i).unwrap().0.declaration),
        );
        Ok(Self::from_source(Source {
            head,
            result: view.ty.clone(),
            signature_result,
            domains,
            locals,
            bindings,
            plan,
        }))
    }
}

impl input_collection::Source for Source<'_> {
    fn plan(&self) -> &input_collection::Plan<'_> {
        &self.plan
    }

    fn substitutions(&self, _slot: usize) -> Option<&input_bindings::Bindings> {
        Some(&self.bindings)
    }

    fn check_input(
        &self,
        _vm: &mut Interpreter<'_>,
        _slot: usize,
        _operand: &Argument,
        _bindings: Option<&input_bindings::Bindings>,
        _expected: Option<&Term>,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn pending_inputs(
        &self,
        vm: &mut Interpreter<'_>,
        slot: usize,
    ) -> Result<BTreeSet<usize>, Error> {
        let (_, domain) = self.domains.parameter(slot).expect("admitted parameter");
        self.bindings
            .pending_inputs(vm, domain, self.plan.view().source)
    }

    fn expected(&self, vm: &mut Interpreter<'_>, slot: usize) -> Result<Term, Error> {
        let (_, domain) = self.domains.parameter(slot).expect("admitted parameter");
        self.bindings
            .instantiate(vm, domain, self.plan.view().source)
    }

    fn bind(&mut self, vm: &mut Interpreter<'_>, slot: usize, value: Value) -> Result<(), Error> {
        let source = self.input(slot).source();
        self.domains
            .bind(vm, &mut self.bindings, slot, value, source)
    }

    fn complete(&mut self, vm: &mut Interpreter<'_>) -> Result<Progress, Error> {
        let source = self.plan.view().source;
        if self.result == Term::Primitive(Primitive::Never) {
            return Err(vm.failure(ErrorKind::Type, Some(source)));
        }
        if let Some(expected) = &self.signature_result {
            let instantiated = self.bindings.instantiate(vm, expected, source)?;
            if instantiated != self.result {
                return Err(vm.failure(ErrorKind::Type, Some(source)));
            }
        }
        let bindings =
            std::mem::replace(&mut self.bindings, input_bindings::Bindings::structural([]));
        let arguments = bindings
            .finish_normal()
            .ok_or_else(|| vm.failure(ErrorKind::Type, Some(source)))?;
        Ok(Progress::Complete(Flow::Value(vm.complete_normal_data(
            self.head,
            arguments,
            &self.result,
            source,
        )?)))
    }

    fn cleanup(&self, environment: &mut Environment) {
        for binding in self.locals.iter() {
            environment.remove(binding);
        }
    }
}

impl Interpreter<'_> {
    pub(super) fn eval_normal_form_type(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let selected = if let K::ApplyNormalForm { callee, .. } = &expr.kind {
            match self.eval(callee, environment)? {
                Flow::Value(value) => Some(value),
                exit => return Ok(exit),
            }
        } else {
            None
        };
        let mut frame = Frame::application(self, Plan::borrowed(expr), environment, selected)?;
        let result = self.run_normal_plan(&mut frame, environment);
        input_collection::InputPlan::cleanup(&frame, environment);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use input_collection::{Completion, InputPlan, Request};
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    use nir::program::NormalFormInputPlan;

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

    fn package() -> Package {
        Package {
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
        }
    }

    #[test]
    fn unplanned_inputs_preserve_caller_bindings_and_recover_across_entries() {
        let package = package();
        for entry in ["tree", "direct-frame", "bytecode"] {
            let mut vm = if entry == "tree" {
                Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
            } else {
                Interpreter::for_comptime(&package, Budget::default()).unwrap()
            };
            // These identities collide with head bindings, not with any input
            // plan locals: the unplanned expression owns no caller bindings.
            for (spelling, from_local) in [("256", false), ("7", false), ("256", true), ("7", true)]
            {
                let original = Environment::from([
                    (local(0), Value::UnsignedInteger(spelling.parse().unwrap())),
                    (local(1), Value::Bool(true)),
                    (local(10), Value::Char('x')),
                ]);
                let value = if from_local {
                    Argument::Value(expr(Primitive::U8, K::Local(local(0))))
                } else {
                    Argument::integer(SOURCE, spelling.into(), false)
                };
                let body = expr(
                    Primitive::Type,
                    K::NormalFormType {
                        head: id(0),
                        arguments: vec![
                            Argument::Value(expr(
                                Primitive::Type,
                                K::TypeValue(Term::Primitive(Primitive::U8)),
                            )),
                            value,
                        ],
                        plan: None,
                    },
                );
                let mut environment = original.clone();
                let result = match entry {
                    "tree" => vm.eval(&body, &mut environment),
                    "direct-frame" => {
                        let mut frame =
                            Frame::new(&mut vm, Plan::borrowed(&body), &environment).unwrap();
                        let result = vm.run_type_plan(&mut frame, &mut environment);
                        frame.cleanup(&mut environment);
                        result
                    }
                    "bytecode" => {
                        let code =
                            Arc::new(comptime_compiler::compile(&body, &mut 100_000).unwrap());
                        vm.execute_code(&code, &mut environment)
                    }
                    _ => unreachable!(),
                };
                if spelling == "256" {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::Type, "{entry}");
                } else {
                    let Flow::Value(actual) = result.unwrap() else {
                        panic!("{entry}");
                    };
                    assert_eq!(
                        actual,
                        Value::Type(Term::NormalForm {
                            head: id(0),
                            sort: nir::Sort::Type,
                            arguments: vec![
                                Term::Primitive(Primitive::U8),
                                Term::Integer(comptime_runtime::Integer::from_u128(7))
                            ],
                        }),
                        "{entry}"
                    );
                }
                assert_eq!(environment, original, "{entry}: {spelling}");
            }
        }
    }

    #[test]
    fn a_reusable_plan_checks_both_caller_registers_and_lexical_bindings_at_entry() {
        let package = package();
        let plan = expr(
            Primitive::Type,
            K::NormalFormType {
                head: id(0),
                arguments: vec![
                    Argument::Value(expr(Primitive::Type, K::Local(local(7)))),
                    Argument::integer(SOURCE, "7".into(), false),
                ],
                plan: Some(NormalFormInputPlan {
                    order: vec![0, 1],
                    bindings: vec![local(10), local(11)],
                    preparations: vec![],
                }),
            },
        );
        // The later let reserves a caller register for 10. Local 11 has no register.
        let body = expr(
            Primitive::Void,
            K::Block {
                statements: vec![
                    plan,
                    expr(
                        Primitive::Void,
                        K::Let {
                            binding: local(10),
                            value: Box::new(expr(Primitive::Void, K::Unit)),
                        },
                    ),
                ],
                result: None,
            },
        );
        let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for collision in [Some(local(10)), None, Some(local(11)), None] {
            let mut environment =
                Environment::from([(local(7), Value::Type(Term::Primitive(Primitive::U8)))]);
            if let Some(binding) = collision {
                environment.insert(binding, Value::Bool(false));
            }
            let before = environment.clone();
            let result = vm.execute_code(&code, &mut environment);
            if collision.is_some() {
                let error = result.unwrap_err();
                assert_eq!(error.kind, ErrorKind::Type);
                assert_eq!(error.source, Some(SOURCE));
            } else {
                assert!(matches!(result.unwrap(), Flow::Value(Value::Unit)));
            }
            assert_eq!(environment, before);
        }
    }

    #[test]
    fn nested_tree_input_plans_obey_depth_and_recover() {
        let mut package = package();
        package.normal_forms.push(Head {
            declaration: id(20),
            collector: None,
            role: Role::Opaque,
            requirements: vec![],
            parameters: vec![Parameter {
                declaration: id(21),
                binding: local(21),
                kind: Kind::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
        });
        let leaf = expr(
            Primitive::Type,
            K::TypeValue(Term::Primitive(Primitive::U8)),
        );
        let mut nested = leaf.clone();
        for _ in 0..40 {
            nested = expr(
                Primitive::Type,
                K::NormalFormType {
                    head: id(20),
                    arguments: vec![Argument::Value(nested)],
                    plan: None,
                },
            );
        }
        let mut vm = Interpreter::for_comptime_tree(
            &package,
            Budget {
                steps: 100_000,
                depth: 16,
            },
        )
        .unwrap();
        let mut environment = Environment::from([(local(21), Value::Bool(true))]);
        assert_eq!(
            vm.eval(&nested, &mut environment).unwrap_err().kind,
            ErrorKind::DepthExhausted
        );
        assert_eq!(environment.get(&local(21)), Some(&Value::Bool(true)));
        let shallow = expr(
            Primitive::Type,
            K::NormalFormType {
                head: id(20),
                arguments: vec![Argument::Value(leaf)],
                plan: None,
            },
        );
        let Flow::Value(value) = vm.eval(&shallow, &mut environment).unwrap() else {
            panic!("type value");
        };
        assert_eq!(
            value,
            Value::Type(Term::NormalForm {
                head: id(20),
                sort: nir::Sort::Type,
                arguments: vec![Term::Primitive(Primitive::U8)],
            })
        );
        assert_eq!(
            environment,
            Environment::from([(local(21), Value::Bool(true))])
        );
    }

    #[test]
    fn suspended_plan_resumes_once_and_retains_pending_literal_identity() {
        let package = package();
        let arguments = vec![
            Argument::Value(expr(
                Primitive::Type,
                K::TypeValue(Term::Primitive(Primitive::U8)),
            )),
            Argument::integer(SOURCE, "7".into(), false),
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
        let constructor = |plan| {
            expr(
                Primitive::Type,
                K::NormalFormType {
                    head: id(0),
                    arguments: arguments.clone(),
                    plan: Some(plan),
                },
            )
        };
        let ordered = constructor(ordered);
        let reversed = constructor(plan);
        let mut failed = Frame::new(&mut vm, Plan::borrowed(&ordered), &env).unwrap();
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
        let mut failed = Frame::new(&mut vm, Plan::borrowed(&ordered), &env).unwrap();
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
        let mut frame = Frame::new(&mut vm, Plan::borrowed(&reversed), &env).unwrap();
        let Request::Stage(argument) = frame.advance(&mut vm, &mut env).unwrap() else {
            panic!("expected deferred input");
        };
        let std::ops::ControlFlow::Continue(staged) =
            vm.stage_normal_argument(&argument, &mut env, 0).unwrap()
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
            .evaluate_contextual_argument(&argument, &expected, &mut env)
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
            Request::DomainDelivered
        ));
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

    #[test]
    fn non_type_normal_form_applications_keep_head_and_value_sort() {
        let mut package = package();
        let param_ty = Term::Rigid {
            declaration: id(1),
            sort: nir::Sort::Type,
        };
        package.functions.push(nir::program::Function {
            interface: nir::program::FunctionDeclaration {
                source: SOURCE,
                declaration: id(0),
                name: "flag".into(),
                phase: nir::program::FunctionPhase::NormalForm,
                inline: false,
                pure: true,
                is_unsafe: false,
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                type_parameters: vec![id(1)],
                requirements: vec![],
                capabilities: vec![],
                signature: Term::Function {
                    parameters: vec![nir::Parameter {
                        sort: nir::Sort::ComptimeValue,
                        ty: param_ty.clone(),
                    }],
                    result: Box::new(Term::Primitive(Primitive::Bool)),
                },
                parameters: vec![local(1)],
                env_parameters: vec![],
                defaults: vec![],
            },
            body: expr(
                Primitive::Bool,
                K::Literal(nir::program::Literal::Bool(false)),
            ),
            runtime_deliveries: Default::default(),
        });
        let expected_term = Term::NormalForm {
            head: id(0),
            sort: nir::Sort::ComptimeValue,
            arguments: vec![
                Term::Primitive(Primitive::U8),
                Term::Integer(comptime_runtime::Integer::from_i128(7)),
            ],
        };
        let expected_value = Value::NormalForm {
            term: expected_term.clone(),
            ty: Term::Primitive(Primitive::Bool),
        };
        let body = Expr {
            source: SOURCE,
            ty: Term::Primitive(Primitive::Bool),
            kind: K::NormalFormType {
                head: id(0),
                arguments: vec![
                    Argument::Value(expr(
                        Primitive::Type,
                        K::TypeValue(Term::Primitive(Primitive::U8)),
                    )),
                    Argument::integer(SOURCE, "7".into(), false),
                ],
                plan: None,
            },
        };
        for entry in ["tree", "direct-frame", "bytecode"] {
            let mut vm = if entry == "tree" {
                Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
            } else {
                Interpreter::for_comptime(&package, Budget::default()).unwrap()
            };
            let mut env = Environment::new();
            let flow = if entry == "bytecode" {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10_000).unwrap());
                vm.execute_code(&code, &mut env).unwrap()
            } else if entry == "direct-frame" {
                let mut frame = Frame::new(&mut vm, Plan::borrowed(&body), &env).unwrap();
                let flow = vm.run_normal_plan(&mut frame, &mut env).unwrap();
                input_collection::InputPlan::cleanup(&frame, &mut env);
                flow
            } else {
                vm.eval(&body, &mut env).unwrap()
            };
            let Flow::Value(ref value) = flow else {
                panic!("expected value");
            };
            assert_eq!(value, &expected_value, "{entry}");
            assert!(
                value
                    .to_string()
                    .contains("(normal-form-value (type (primitive bool))")
            );
            let mut rt_budget = comptime_runtime::Budget::default();
            let frozen = value
                .to_scoped_term(
                    &mut rt_budget,
                    &Default::default(),
                    &Default::default(),
                    &Default::default(),
                    true,
                )
                .unwrap();
            assert_eq!(frozen, expected_term);
            let decoded = vm
                .normal_pattern_value(&frozen, &Term::Primitive(Primitive::Bool))
                .unwrap();
            assert_eq!(decoded, expected_value);
        }
    }

    #[test]
    fn dependent_non_type_normal_form_rejects_mismatched_result_type() {
        let mut package = package();
        let param_ty = Term::Rigid {
            declaration: id(1),
            sort: nir::Sort::Type,
        };
        package.functions.push(nir::program::Function {
            interface: nir::program::FunctionDeclaration {
                source: SOURCE,
                declaration: id(0),
                name: "choose_val".into(),
                phase: nir::program::FunctionPhase::NormalForm,
                inline: false,
                pure: true,
                is_unsafe: false,
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                type_parameters: vec![id(1)],
                requirements: vec![],
                capabilities: vec![],
                signature: Term::Function {
                    parameters: vec![nir::Parameter {
                        sort: nir::Sort::ComptimeValue,
                        ty: param_ty.clone(),
                    }],
                    result: Box::new(param_ty),
                },
                parameters: vec![local(1)],
                env_parameters: vec![],
                defaults: vec![],
            },
            body: expr(Primitive::U32, K::Local(local(1))),
            runtime_deliveries: Default::default(),
        });
        let valid_body = Expr {
            source: SOURCE,
            ty: Term::Primitive(Primitive::U32),
            kind: K::NormalFormType {
                head: id(0),
                arguments: vec![
                    Argument::Value(expr(
                        Primitive::Type,
                        K::TypeValue(Term::Primitive(Primitive::U32)),
                    )),
                    Argument::integer(SOURCE, "7".into(), false),
                ],
                plan: None,
            },
        };
        let forged_body = Expr {
            source: SOURCE,
            ty: Term::Primitive(Primitive::Bool),
            kind: valid_body.kind.clone(),
        };
        for entry in ["tree", "direct-frame", "bytecode"] {
            let mut vm = if entry == "tree" {
                Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
            } else {
                Interpreter::for_comptime(&package, Budget::default()).unwrap()
            };
            let mut env = Environment::new();
            let valid = if entry == "bytecode" {
                let code = Arc::new(comptime_compiler::compile(&valid_body, &mut 10_000).unwrap());
                vm.execute_code(&code, &mut env).unwrap()
            } else if entry == "direct-frame" {
                let mut frame = Frame::new(&mut vm, Plan::borrowed(&valid_body), &env).unwrap();
                let flow = vm.run_normal_plan(&mut frame, &mut env).unwrap();
                input_collection::InputPlan::cleanup(&frame, &mut env);
                flow
            } else {
                vm.eval(&valid_body, &mut env).unwrap()
            };
            let Flow::Value(valid) = valid else {
                panic!("expected value for {entry}");
            };
            assert_eq!(
                valid,
                Value::NormalForm {
                    term: Term::NormalForm {
                        head: id(0),
                        sort: nir::Sort::ComptimeValue,
                        arguments: vec![
                            Term::Primitive(Primitive::U32),
                            Term::Integer(comptime_runtime::Integer::from_i128(7)),
                        ],
                    },
                    ty: Term::Primitive(Primitive::U32),
                },
                "{entry}"
            );

            let forged = if entry == "bytecode" {
                let code = Arc::new(comptime_compiler::compile(&forged_body, &mut 10_000).unwrap());
                vm.execute_code(&code, &mut env)
            } else if entry == "direct-frame" {
                let mut frame = Frame::new(&mut vm, Plan::borrowed(&forged_body), &env).unwrap();
                let flow = vm.run_normal_plan(&mut frame, &mut env);
                input_collection::InputPlan::cleanup(&frame, &mut env);
                flow
            } else {
                vm.eval(&forged_body, &mut env)
            };
            assert_eq!(forged.unwrap_err().kind, ErrorKind::Type, "{entry}");
        }
    }
}
