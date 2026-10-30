use super::*;
use nir::program::{Literal, Place};
const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
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

#[test]
fn nested_inputs_preserve_depth_limits_and_recover() {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let package = Package::default();
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            for nesting in [0, 1, 32, 126, 127, 128, 160] {
                let mut input = A::Value(boolean(true));
                for _ in 0..nesting {
                    input = A::Tuple {
                        source: SOURCE,
                        type_head: None,
                        items: vec![input],
                    };
                }
                vm.budget = Budget {
                    steps: 100000,
                    depth: usize::MAX,
                };
                let result = vm.stage_normal_argument(&input, &mut Environment::new(), 0);
                assert_eq!(vm.depth, 0);
                if nesting >= 128 {
                    assert!(matches!(
                        result,
                        Err(Error {
                            kind: ErrorKind::DepthExhausted,
                            ..
                        })
                    ));
                } else {
                    let ControlFlow::Continue(staged) = result.unwrap() else {
                        panic!("transfer")
                    };
                    let mut value = vm
                        .finish_staged_normal_argument(staged, &mut BTreeMap::new(), SOURCE, 0)
                        .unwrap();
                    for _ in 0..nesting {
                        let Value::Tuple(mut items) = value else {
                            panic!("tuple")
                        };
                        assert_eq!(items.len(), 1);
                        value = items.pop().unwrap();
                    }
                    assert_eq!(value, Value::Bool(true));
                }
                let result = vm
                    .stage_normal_argument(&A::Value(boolean(false)), &mut Environment::new(), 0)
                    .unwrap();
                assert!(matches!(
                    result,
                    ControlFlow::Continue(StagedNormalInput::Value(Value::Bool(false)))
                ));
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn staged_scopes_cleanup_on_errors_transfers_and_budget_exhaustion() {
    let package = Package::default();
    let outer = LocalId { owner: 1, index: 0 };
    let inner = LocalId { owner: 1, index: 1 };
    for ending in [
        boolean(false),
        expr(Primitive::Never, K::Return(Some(Box::new(boolean(true))))),
        expr(
            Primitive::Bool,
            K::Local(LocalId {
                owner: 1,
                index: 99,
            }),
        ),
    ] {
        let input = A::Block {
            source: SOURCE,
            statements: vec![
                expr(
                    Primitive::Void,
                    K::Let {
                        binding: inner,
                        value: Box::new(boolean(true)),
                    },
                ),
                expr(
                    Primitive::Void,
                    K::Assign {
                        place: Place {
                            root: outer,
                            fields: vec![],
                        },
                        value: Box::new(boolean(true)),
                    },
                ),
            ],
            result: Some(Box::new(A::Block {
                source: SOURCE,
                statements: vec![],
                result: Some(Box::new(A::Value(ending.clone()))),
            })),
        };
        let (code, _) = comptime_compiler::compile_staged_input(&input, &mut 10000).unwrap();
        let maximum = code.storage_work() as u64 + 256;
        let mut failed_before_effect = false;
        let mut failed_after_effect = false;
        for steps in 0..=maximum {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            vm.budget.steps = steps;
            let mut env = Environment::new();
            env.insert(outer, Value::Bool(false));
            let result = vm.stage_normal_argument(&input, &mut env, 0);
            assert_eq!(vm.depth, 0);
            assert!(!env.contains_key(&inner));
            assert_eq!(env.len(), 1);
            if matches!(
                &result,
                Err(Error {
                    kind: ErrorKind::StepsExhausted,
                    ..
                })
            ) {
                if env.get(&outer) == Some(&Value::Bool(true)) {
                    failed_after_effect = true;
                } else {
                    failed_before_effect = true;
                }
            }
            if steps == maximum {
                assert_eq!(vm.bytecode_staged_inputs(), 1);
                assert_eq!(env.get(&outer), Some(&Value::Bool(true)));
                match ending.kind {
                    K::Return(_) => assert!(matches!(
                        result,
                        Ok(ControlFlow::Break(Flow::Return(Value::Bool(true))))
                    )),
                    K::Local(_) => assert!(matches!(
                        result,
                        Err(Error {
                            kind: ErrorKind::MissingBinding,
                            ..
                        })
                    )),
                    _ => assert!(matches!(
                        result,
                        Ok(ControlFlow::Continue(StagedNormalInput::Value(
                            Value::Bool(false)
                        )))
                    )),
                }
            }
        }
        assert!(failed_before_effect);
        assert!(failed_after_effect);
    }
}

#[test]
fn staged_bytecode_preserves_selected_shapes_and_delayed_preflight() {
    use nir::program::{NormalMatchArm, Pattern};
    let package = Package::default();
    let trace = LocalId { owner: 7, index: 0 };
    let literal = |spelling: &str| A::Integer {
        source: SOURCE,
        spelling: spelling.into(),
        negative: false,
    };
    for invalid in [false, true] {
        let input = A::Tuple {
            source: SOURCE,
            type_head: None,
            items: vec![
                A::Block {
                    source: SOURCE,
                    statements: vec![expr(
                        Primitive::Void,
                        K::Assign {
                            place: Place {
                                root: trace,
                                fields: vec![],
                            },
                            value: Box::new(expr(
                                Primitive::Bool,
                                K::Not(Box::new(expr(Primitive::Bool, K::Local(trace)))),
                            )),
                        },
                    )],
                    result: Some(Box::new(literal("17"))),
                },
                A::If {
                    source: SOURCE,
                    condition: boolean(false),
                    then_input: Box::new(literal(if invalid { "256" } else { "99" })),
                    else_input: Box::new(literal("23")),
                },
                A::Match {
                    source: SOURCE,
                    value: boolean(true),
                    arms: vec![
                        NormalMatchArm {
                            pattern: Pattern::Wild,
                            guard: Some(boolean(false)),
                            body: literal("99"),
                        },
                        NormalMatchArm {
                            pattern: Pattern::Wild,
                            guard: Some(boolean(true)),
                            body: literal("29"),
                        },
                    ],
                },
            ],
        };
        let expected = Term::Tuple(vec![Term::Primitive(Primitive::U8); 3]);
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env = Environment::new();
            env.insert(trace, Value::Bool(false));
            let staged = if compiled {
                vm.stage_normal_argument(&input, &mut env, 0)
            } else {
                vm.interpret_staged_argument(&input, &mut env, 0)
            }
            .unwrap();
            assert_eq!(vm.bytecode_staged_inputs(), usize::from(compiled));
            let ControlFlow::Continue(staged) = staged else {
                panic!("unexpected transfer")
            };
            assert_eq!(env.get(&trace), Some(&Value::Bool(true)));
            let mut prepared = BTreeMap::new();
            let result = vm.prepare_normal_argument(&input, &expected, &mut prepared, 0);
            if invalid {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            } else {
                result.unwrap();
                assert_eq!(
                    vm.finish_staged_normal_argument(staged, &mut prepared, SOURCE, 0)
                        .unwrap(),
                    Value::Tuple(vec![
                        Value::UnsignedInteger(17),
                        Value::UnsignedInteger(23),
                        Value::UnsignedInteger(29)
                    ])
                );
            }
            assert_eq!(env.get(&trace), Some(&Value::Bool(true)));
            assert_eq!(vm.depth, 0);
        }
    }
}

#[test]
fn staged_patterns_compile_or_fall_back_before_operand_effects() {
    use nir::program::{MatchArm, Pattern};
    let package = Package::default();
    let trace = LocalId { owner: 7, index: 0 };
    for case in 0..3 {
        let sequential = Pattern::Then {
            first: Box::new(Pattern::Wild),
            value: Box::new(boolean(false)),
            next: Box::new(Pattern::Wild),
        };
        let pattern = match case {
            0 => sequential,
            1 => Pattern::TypeTuple(vec![sequential]),
            _ => Pattern::CodeEvidence { outputs: vec![] },
        };
        let input = A::Value(expr(
            Primitive::Bool,
            K::Block {
                statements: vec![expr(
                    Primitive::Void,
                    K::Assign {
                        place: Place {
                            root: trace,
                            fields: vec![],
                        },
                        value: Box::new(expr(
                            Primitive::Bool,
                            K::Not(Box::new(expr(Primitive::Bool, K::Local(trace)))),
                        )),
                    },
                )],
                result: Some(Box::new(expr(
                    Primitive::Bool,
                    K::Match {
                        value: Box::new(if case == 1 {
                            Expr {
                                source: SOURCE,
                                ty: Term::Primitive(Primitive::Type),
                                kind: K::TypeValue(Term::Tuple(vec![Term::Primitive(
                                    Primitive::Bool,
                                )])),
                            }
                        } else {
                            boolean(true)
                        }),
                        arms: vec![MatchArm {
                            pattern,
                            guard: None,
                            body: boolean(true),
                        }],
                    },
                ))),
            },
        ));
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        env.insert(trace, Value::Bool(false));
        let result = vm.stage_normal_argument(&input, &mut env, 0);
        if case == 2 {
            assert!(matches!(
                result,
                Err(Error {
                    kind: ErrorKind::TypeContractUnresolved,
                    ..
                })
            ));
        } else {
            let result = result.unwrap();
            assert!(matches!(
                result,
                ControlFlow::Continue(StagedNormalInput::Value(Value::Bool(true)))
            ));
        }
        assert_eq!(env.get(&trace), Some(&Value::Bool(true)));
        assert_eq!(vm.bytecode_staged_inputs(), usize::from(case != 2));
    }
}
