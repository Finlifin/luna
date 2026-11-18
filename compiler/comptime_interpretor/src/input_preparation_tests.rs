use super::*;
use nir::program::{NormalFormPreparation, Pattern, Place};
const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
fn local(index: u32) -> LocalId {
    LocalId { owner: 74, index }
}
fn expr(ty: Primitive, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty: Term::Primitive(ty),
        kind,
    }
}
fn boolean(value: bool) -> Expr {
    expr(Primitive::Bool, K::Literal(Literal::Bool(value)))
}
fn tuple(values: Vec<Expr>) -> Expr {
    Expr {
        source: SOURCE,
        ty: Term::Tuple(values.iter().map(|value| value.ty.clone()).collect()),
        kind: K::Tuple(values),
    }
}
fn preparation(result: Expr) -> NormalFormPreparation {
    let ty = result.ty.clone();
    NormalFormPreparation {
        before: 0,
        expression: expr(
            Primitive::Void,
            K::LetPattern {
                pattern: Pattern::Tuple(
                    vec![1, 2]
                        .into_iter()
                        .map(|index| Pattern::Binding {
                            binding: local(index),
                            inner: Box::new(Pattern::Wild),
                        })
                        .collect(),
                ),
                value: Box::new(Expr {
                    source: SOURCE,
                    ty,
                    kind: K::Block {
                        statements: vec![
                            expr(
                                Primitive::Void,
                                K::Let {
                                    binding: local(3),
                                    value: Box::new(boolean(false)),
                                },
                            ),
                            expr(
                                Primitive::Void,
                                K::Assign {
                                    place: Place {
                                        root: local(0),
                                        fields: vec![],
                                    },
                                    value: Box::new(expr(
                                        Primitive::Bool,
                                        K::Not(Box::new(expr(Primitive::Bool, K::Local(local(0))))),
                                    )),
                                },
                            ),
                        ],
                        result: Some(Box::new(result)),
                    },
                }),
                guard: None,
                otherwise: None,
            },
        ),
    }
}

#[test]
fn packet_exports_commit_only_declared_bindings_and_preserve_transfers() {
    let package = Package::default();
    for (case, result) in [
        tuple(vec![boolean(true), boolean(false)]),
        expr(Primitive::Never, K::Return(Some(Box::new(boolean(true))))),
        expr(Primitive::Bool, K::Local(local(99))),
        tuple(vec![boolean(true)]),
    ]
    .into_iter()
    .enumerate()
    {
        let prep = preparation(result);
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env = Environment::new();
            env.insert(local(0), Value::Bool(false));
            let result = if compiled {
                vm.evaluate_input_preparation(&prep, &mut env)
            } else {
                vm.eval(&prep.expression, &mut env)
            };
            assert_eq!(vm.bytecode_input_preparations(), usize::from(compiled));
            assert_eq!(env.get(&local(0)), Some(&Value::Bool(true)), "case {case}");
            assert!(!env.contains_key(&local(3)));
            if case == 0 {
                assert!(matches!(result, Ok(Flow::Value(Value::Unit))));
                assert_eq!(env.get(&local(1)), Some(&Value::Bool(true)));
                assert_eq!(env.get(&local(2)), Some(&Value::Bool(false)));
                assert_eq!(env.len(), 3);
            } else {
                assert_eq!(env.len(), 1);
                match case {
                    1 => assert!(matches!(result, Ok(Flow::Return(Value::Bool(true))))),
                    2 => assert_eq!(result.unwrap_err().kind, ErrorKind::MissingBinding),
                    3 => assert_eq!(result.unwrap_err().kind, ErrorKind::NonExhaustiveMatch),
                    _ => unreachable!(),
                }
            }
        }
    }
}

#[test]
fn packet_budget_exhaustion_preserves_effects_without_partial_exports() {
    let package = Package::default();
    let prep = preparation(tuple(vec![boolean(true), boolean(false)]));
    let work = comptime_compiler::compile(&prep.expression, &mut 10000)
        .unwrap()
        .storage_work();
    let mut failed_before = false;
    let mut failed_after = false;
    let mut completed = false;
    for steps in 0..=work as u64 + 256 {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.budget.steps = steps;
        let mut env = Environment::new();
        env.insert(local(0), Value::Bool(false));
        match vm.evaluate_input_preparation(&prep, &mut env) {
            Ok(Flow::Value(Value::Unit)) => {
                completed = true;
                assert_eq!(env.len(), 3);
                assert_eq!(env.get(&local(0)), Some(&Value::Bool(true)));
                assert_eq!(env.get(&local(1)), Some(&Value::Bool(true)));
                assert_eq!(env.get(&local(2)), Some(&Value::Bool(false)));
            }
            Err(error) => {
                assert_eq!(error.kind, ErrorKind::StepsExhausted);
                assert_eq!(env.len(), 1);
                if env.get(&local(0)) == Some(&Value::Bool(true)) {
                    failed_after = true;
                } else {
                    failed_before = true;
                }
            }
            flow => panic!("unexpected result {flow:?}"),
        }
        assert!(!env.contains_key(&local(3)));
    }
    assert!(failed_before && failed_after && completed);
}

#[test]
fn packet_binding_collisions_are_rejected_before_producer_effects() {
    let package = Package::default();
    let prep = preparation(tuple(vec![boolean(true), boolean(false)]));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut env = Environment::new();
    env.insert(local(0), Value::Bool(false));
    env.insert(local(1), Value::Bool(false));
    assert_eq!(
        vm.evaluate_input_preparation(&prep, &mut env)
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert_eq!(env.get(&local(0)), Some(&Value::Bool(false)));
    assert_eq!(env.get(&local(1)), Some(&Value::Bool(false)));
    assert_eq!(vm.bytecode_input_preparations(), 0);
    env.remove(&local(1));
    assert!(matches!(
        vm.evaluate_input_preparation(&prep, &mut env),
        Ok(Flow::Value(Value::Unit))
    ));
}

#[test]
fn normal_plan_cleans_exported_bindings_and_executes_trailing_empty_packets() {
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    use nir::program::{NormalFormArgument as A, NormalFormInputPlan};
    let id = |local| DeclarationId { package: 74, local };
    let package = Package {
        normal_forms: vec![Head {
            declaration: id(1),
            collector: None,
            role: Role::Opaque,
            requirements: vec![],
            parameters: vec![
                Parameter {
                    declaration: id(2),
                    binding: local(10),
                    kind: Kind::Type,
                    ty: Term::Primitive(Primitive::Type),
                },
                Parameter {
                    declaration: id(3),
                    binding: local(11),
                    kind: Kind::Value,
                    ty: Term::Rigid {
                        declaration: id(2),
                        sort: nir::Sort::Type,
                    },
                },
            ],
        }],
        ..Default::default()
    };
    for (invalid, reversed, compiled) in [
        (true, false, false),
        (false, false, false),
        (true, true, false),
        (false, true, false),
        (true, false, true),
        (false, false, true),
        (true, true, true),
        (false, true, true),
    ] {
        let first = preparation(tuple(vec![
            expr(
                Primitive::Type,
                K::TypeValue(Term::Primitive(Primitive::Bool)),
            ),
            boolean(true),
        ]));
        let mut empty = preparation(tuple(vec![]));
        empty.before = 2;
        let K::LetPattern { pattern, .. } = &mut empty.expression.kind else {
            unreachable!()
        };
        *pattern = Pattern::Tuple(vec![]);
        let body = expr(
            Primitive::Type,
            K::NormalFormType {
                head: id(1),
                arguments: vec![
                    A::Value(expr(Primitive::Type, K::Local(local(1)))),
                    A::Value(expr(
                        Primitive::Bool,
                        K::Local(local(if invalid { 99 } else { 2 })),
                    )),
                ],
                plan: Some(NormalFormInputPlan {
                    order: if reversed { vec![1, 0] } else { vec![0, 1] },
                    bindings: vec![local(10), local(11)],
                    preparations: vec![first, empty],
                }),
            },
        );
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        env.insert(local(0), Value::Bool(false));
        let result = if compiled {
            let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
            assert!(code.to_string().contains("(type-input-plan %"));
            let result = vm.execute_code(&code, &mut env);
            let plan = code
                .instructions()
                .iter()
                .find_map(|instruction| match &instruction.op {
                    Op::TypeInputPlan { plan, .. } => Some(plan),
                    _ => None,
                })
                .unwrap();
            assert!(
                plan.preparation(0)
                    .compiled(&vm.compilation_session)
                    .is_some()
            );
            // Failure in the intervening operand must not compile the trailing packet.
            assert_eq!(
                plan.preparation(1)
                    .compiled(&vm.compilation_session)
                    .is_some(),
                !invalid
            );
            result
        } else {
            vm.eval(&body, &mut env)
        };
        assert_eq!(env.len(), 1);
        assert_eq!(vm.bytecode_staged_inputs(), usize::from(reversed));
        if invalid {
            assert_eq!(result.unwrap_err().kind, ErrorKind::MissingBinding);
            assert_eq!(env.get(&local(0)), Some(&Value::Bool(true)));
            assert_eq!(vm.bytecode_input_preparations(), 1);
        } else {
            let Flow::Value(value) = result.unwrap() else {
                panic!("transfer")
            };
            assert_eq!(
                value,
                Value::Type(Term::NormalForm {
                    head: id(1),
                    sort: nir::Sort::Type,
                    arguments: vec![Term::Primitive(Primitive::Bool), Term::Bool(true)]
                })
            );
            assert_eq!(env.get(&local(0)), Some(&Value::Bool(false)));
            assert_eq!(vm.bytecode_input_preparations(), 2);
        }
    }
}

#[test]
fn fragment_exports_reject_invalid_patterns_before_taking_the_environment() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for pattern in [
        Pattern::Tuple(vec![Pattern::Wild]),
        Pattern::Tuple(vec![
            Pattern::Binding {
                binding: local(1),
                inner: Box::new(Pattern::Wild)
            };
            2
        ]),
    ] {
        let mut prep = preparation(tuple(vec![boolean(true), boolean(false)]));
        let K::LetPattern {
            pattern: output, ..
        } = &mut prep.expression.kind
        else {
            unreachable!()
        };
        *output = pattern;
        let original = Environment::from([(local(0), Value::Bool(false))]);
        let mut environment = original.clone();
        let error = vm
            .start_input_preparation(PreparationSite::borrowed(&prep), &mut environment)
            .err()
            .unwrap();
        assert_eq!(error.kind, ErrorKind::Type);
        assert_eq!(environment, original);
    }
}

#[test]
fn a_packet_site_reuses_code_across_activations_and_publishes_only_after_compilation() {
    use nir::program::NormalFormInputPlan;
    let package = Package::default();
    let make_plan = |value| {
        let body = expr(
            Primitive::Type,
            K::NormalFormType {
                head: DeclarationId {
                    package: 74,
                    local: 1,
                },
                arguments: vec![],
                plan: Some(NormalFormInputPlan {
                    order: vec![],
                    bindings: vec![],
                    preparations: vec![preparation(tuple(vec![
                        boolean(value),
                        expr(Primitive::Bool, K::Local(local(0))),
                    ]))],
                }),
            },
        );
        let code = comptime_compiler::compile(&body, &mut 10000).unwrap();
        code.instructions()
            .iter()
            .find_map(|instruction| match &instruction.op {
                Op::TypeInputPlan { plan, .. } => Some(plan.clone()),
                _ => None,
            })
            .unwrap()
    };
    let first = make_plan(true);
    let second = make_plan(false);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut env = Environment::from([(local(0), Value::Bool(false))]);
    vm.budget.steps = 2;
    let error = vm
        .start_input_preparation(first.preparation(0), &mut env)
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::StepsExhausted);
    assert!(
        first
            .preparation(0)
            .compiled(&vm.compilation_session)
            .is_none()
    );
    assert_eq!(env.len(), 1);
    vm.budget = Budget::default();
    for (plan, field, input, cold) in [
        (&first, true, false, true),
        (&first, true, true, false),
        (&second, false, false, true),
    ] {
        let mut env = Environment::from([(local(0), Value::Bool(input))]);
        let count = vm.bytecode_compilations();
        let frame = vm
            .start_input_preparation(plan.preparation(0), &mut env)
            .unwrap()
            .unwrap();
        let result = frame.run(&mut vm);
        assert!(matches!(result.flow, Ok(Flow::Value(Value::Unit))));
        assert_eq!(vm.bytecode_compilations() - count, usize::from(cold));
        assert_eq!(result.environment.get(&local(1)), Some(&Value::Bool(field)));
        assert_eq!(
            result.environment.get(&local(2)),
            Some(&Value::Bool(!input))
        );
    }
    let mut colliding = Environment::from([(local(1), Value::Bool(false))]);
    let count = vm.bytecode_compilations();
    let error = vm
        .start_input_preparation(first.preparation(0), &mut colliding)
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::Type);
    assert_eq!(vm.bytecode_compilations(), count);
    assert_eq!(colliding.len(), 1);
    assert_eq!(vm.bytecode_input_preparations(), 3);
}

#[test]
fn fragment_exports_publish_atomically_only_at_closed_completion() {
    let package = Package::default();
    let prep = preparation(tuple(vec![boolean(true), boolean(false)]));
    for fault in 0..4 {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut environment = Environment::from([(local(0), Value::Bool(false))]);
        let InputFrame { mut code, finish } = vm
            .start_input_preparation(PreparationSite::borrowed(&prep), &mut environment)
            .unwrap()
            .unwrap();
        assert!(environment.is_empty());
        assert!(code.to_string().contains("(bound-exports (source 1 2)"));
        let flow = vm.run_fragment(&mut code);
        assert!(matches!(flow, Ok(Flow::Value(Value::Unit))));
        assert!(code.scopes.is_empty());
        let output = code
            .code
            .locals()
            .iter()
            .find(|(binding, _)| *binding == local(2))
            .unwrap()
            .1;
        let expected = match fault {
            0 => {
                code.slots[output] = None;
                ErrorKind::MissingBinding
            }
            1 => {
                code.scopes.push(PatternScope::default());
                ErrorKind::InvalidControlFlow
            }
            2 => {
                code.terminal = false;
                ErrorKind::InvalidControlFlow
            }
            3 => {
                code.environment.insert(local(2), Value::Bool(true));
                ErrorKind::Type
            }
            _ => unreachable!(),
        };
        let result = finish.complete(&mut vm, code, flow);
        let failure = result.flow.unwrap_err();
        assert_eq!(failure.kind, expected);
        assert_eq!(failure.source, Some(SOURCE));
        assert_eq!(result.environment.get(&local(0)), Some(&Value::Bool(true)));
        assert!(!result.environment.contains_key(&local(1)));
        if fault == 3 {
            assert_eq!(result.environment.get(&local(2)), Some(&Value::Bool(true)));
        } else {
            assert!(!result.environment.contains_key(&local(2)));
        }
    }
}

#[test]
fn fragment_handoff_moves_storage_without_replacing_its_identity() {
    let package = Package::default();
    let prep = preparation(tuple(vec![boolean(true), boolean(false)]));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut environment = Environment::from([(local(0), Value::Bool(false))]);
    let InputFrame { mut code, finish } = vm
        .start_input_preparation(PreparationSite::borrowed(&prep), &mut environment)
        .unwrap()
        .unwrap();
    let flow = vm.run_fragment(&mut code);
    assert!(matches!(flow, Ok(Flow::Value(Value::Unit))));
    let slot = code
        .code
        .locals()
        .iter()
        .find(|(binding, _)| *binding == local(1))
        .unwrap()
        .1;
    // The frame handoff must transport internal storage, not freeze or copy
    // its payload. Domain and ownership admission belong to other boundaries.
    let storage = Storage::new(code.slots[slot].take().unwrap());
    code.slots[slot] = Some(Value::Storage(storage.clone()));
    let result = finish.complete(&mut vm, code, flow);
    assert!(matches!(result.flow, Ok(Flow::Value(Value::Unit))));
    assert_eq!(
        result.environment.get(&local(1)),
        Some(&Value::Storage(storage))
    );
    assert_eq!(result.environment.get(&local(2)), Some(&Value::Bool(false)));
}
