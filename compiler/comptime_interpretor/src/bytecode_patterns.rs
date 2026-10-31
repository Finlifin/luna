use super::*;
use nir::program::{MatchArm, Pattern as P, Place};

const SOURCE: SourceRange = SourceRange { start: 50, end: 60 };
const X: LocalId = LocalId { owner: 1, index: 0 };
const Y: LocalId = LocalId { owner: 1, index: 1 };
const TRACE: LocalId = LocalId { owner: 1, index: 2 };
fn expr(kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty: Term::Primitive(Primitive::Bool),
        kind,
    }
}
fn boolean(value: bool) -> Expr {
    expr(K::Literal(Literal::Bool(value)))
}
fn binding(id: LocalId) -> P {
    P::Binding {
        binding: id,
        inner: Box::new(P::Wild),
    }
}
fn effect() -> Expr {
    expr(K::Block {
        statements: vec![Expr {
            source: SOURCE,
            ty: Term::Primitive(Primitive::Void),
            kind: K::Assign {
                place: Place {
                    root: TRACE,
                    fields: vec![],
                },
                value: Box::new(boolean(true)),
            },
        }],
        result: Some(Box::new(boolean(false))),
    })
}
fn execute(
    vm: &mut Interpreter<'_>,
    body: &Expr,
    env: &mut Environment,
    compiled: bool,
) -> Result<Flow, Error> {
    if compiled {
        let code = Arc::new(comptime_compiler::compile(body, &mut 10000).unwrap());
        vm.execute_code(&code, env)
    } else {
        vm.eval(body, env)
    }
}

#[test]
fn failed_alternatives_restore_shadowed_bindings_without_undoing_effects() {
    let package = Package::default();
    let body = expr(K::Match {
        value: Box::new(boolean(true)),
        arms: vec![MatchArm {
            pattern: P::Or(vec![
                P::Then {
                    first: Box::new(binding(X)),
                    value: Box::new(effect()),
                    next: Box::new(P::Literal(Literal::Bool(true))),
                },
                P::Wild,
            ]),
            guard: None,
            body: expr(K::Local(X)),
        }],
    });
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::from([(X, Value::Bool(false)), (TRACE, Value::Bool(false))]);
        assert!(matches!(
            execute(&mut vm, &body, &mut env, compiled).unwrap(),
            Flow::Value(Value::Bool(false))
        ));
        assert_eq!(
            vm.local_value(&env, X, Some(SOURCE)).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn remaps_reject_ambient_sources_and_duplicates_at_the_correct_boundary() {
    let package = Package::default();
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for duplicate in [false, true, false] {
            let pairs = if duplicate {
                vec![(X, Y), (X, TRACE)]
            } else {
                vec![(X, Y)]
            };
            let body = expr(K::Match {
                value: Box::new(boolean(true)),
                arms: vec![MatchArm {
                    pattern: P::Remap {
                        bindings: pairs,
                        pattern: Box::new(P::Then {
                            first: Box::new(P::Wild),
                            value: Box::new(effect()),
                            next: Box::new(P::Wild),
                        }),
                    },
                    guard: None,
                    body: boolean(true),
                }],
            });
            let mut env = Environment::from([(X, Value::Bool(false)), (TRACE, Value::Bool(false))]);
            assert_eq!(
                execute(&mut vm, &body, &mut env, compiled)
                    .unwrap_err()
                    .kind,
                ErrorKind::InvalidCallPlan
            );
            assert_eq!(
                vm.local_value(&env, X, Some(SOURCE)).unwrap(),
                Value::Bool(false)
            );
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(!duplicate)
            );
        }
    }
}

#[test]
fn remap_swaps_are_simultaneous_and_observation_aliases_never_escape() {
    let package = Package::default();
    let tuple = |values| Expr {
        source: SOURCE,
        ty: Term::Tuple(vec![Term::Primitive(Primitive::Bool); 2]),
        kind: K::Tuple(values),
    };
    let swap = Expr {
        source: SOURCE,
        ty: Term::Tuple(vec![Term::Primitive(Primitive::Bool); 2]),
        kind: K::Match {
            value: Box::new(tuple(vec![boolean(true), boolean(false)])),
            arms: vec![MatchArm {
                pattern: P::Remap {
                    bindings: vec![(X, Y), (Y, X)],
                    pattern: Box::new(P::Tuple(vec![binding(X), binding(Y)])),
                },
                guard: None,
                body: tuple(vec![expr(K::Local(X)), expr(K::Local(Y))]),
            }],
        },
    };
    let observed = expr(K::Match {
        value: Box::new(boolean(true)),
        arms: vec![MatchArm {
            pattern: P::Observe {
                binding: X,
                inner: Box::new(P::Then {
                    first: Box::new(P::Wild),
                    value: Box::new(expr(K::Local(X))),
                    next: Box::new(binding(Y)),
                }),
            },
            guard: None,
            body: expr(K::Local(Y)),
        }],
    });
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        let Flow::Value(value) = execute(&mut vm, &swap, &mut env, compiled).unwrap() else {
            panic!("value")
        };
        assert_eq!(
            value,
            Value::Tuple(vec![Value::Bool(false), Value::Bool(true)])
        );
        assert!(env.is_empty());
        assert!(matches!(
            execute(&mut vm, &observed, &mut env, compiled).unwrap(),
            Flow::Value(Value::Bool(true))
        ));
        assert!(env.is_empty());
        env.insert(X, Value::Bool(false));
        assert_eq!(
            execute(&mut vm, &observed, &mut env, compiled)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidCallPlan
        );
        assert_eq!(env.get(&X), Some(&Value::Bool(false)));
    }
}

#[test]
fn transfers_and_errors_unwind_active_candidate_journals() {
    let package = Package::default();
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for returns in [false, true, false, true] {
            let transfer = if returns {
                expr(K::Return(Some(Box::new(boolean(true)))))
            } else {
                expr(K::Local(LocalId {
                    owner: 99,
                    index: 99,
                }))
            };
            let body = expr(K::Match {
                value: Box::new(boolean(true)),
                arms: vec![MatchArm {
                    pattern: P::Observe {
                        binding: Y,
                        inner: Box::new(P::Then {
                            first: Box::new(binding(X)),
                            value: Box::new(expr(K::Block {
                                statements: vec![effect()],
                                result: Some(Box::new(transfer)),
                            })),
                            next: Box::new(P::Wild),
                        }),
                    },
                    guard: None,
                    body: boolean(false),
                }],
            });
            let mut env = Environment::from([(X, Value::Bool(false)), (TRACE, Value::Bool(false))]);
            let result = execute(&mut vm, &body, &mut env, compiled);
            if returns {
                assert!(matches!(result.unwrap(), Flow::Return(Value::Bool(true))));
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::MissingBinding);
            }
            assert_eq!(
                vm.local_value(&env, X, Some(SOURCE)).unwrap(),
                Value::Bool(false)
            );
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(true)
            );
            assert!(!env.contains_key(&Y));
        }
    }
}

#[test]
fn later_record_labels_are_checked_only_after_earlier_patterns_succeed() {
    let package = Package::default();
    let input = LocalId { owner: 1, index: 3 };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (first_matches, arity) in [(false, 2), (true, 2), (true, 1), (false, 2)] {
            let body = expr(K::Match {
                value: Box::new(expr(K::Local(input))),
                arms: vec![
                    MatchArm {
                        pattern: P::AnonymousRecord {
                            names: vec!["first".into(), "second".into()],
                            fields: vec![
                                P::Then {
                                    first: Box::new(P::Wild),
                                    value: Box::new(effect()),
                                    next: Box::new(P::Literal(Literal::Bool(!first_matches))),
                                },
                                P::Wild,
                            ],
                        },
                        guard: None,
                        body: boolean(true),
                    },
                    MatchArm {
                        pattern: P::Wild,
                        guard: None,
                        body: boolean(false),
                    },
                ],
            });
            let mut fields = vec![
                ("first".into(), Value::Bool(true)),
                ("wrong".into(), Value::Bool(false)),
            ];
            fields.truncate(arity);
            let mut env = Environment::from([
                (input, Value::AnonymousRecord(fields)),
                (TRACE, Value::Bool(false)),
            ]);
            let result = execute(&mut vm, &body, &mut env, compiled);
            if first_matches || arity != 2 {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            } else {
                assert!(matches!(result.unwrap(), Flow::Value(Value::Bool(false))));
            }
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(arity == 2)
            );
        }
    }
}

#[test]
fn type_fields_stop_after_failure_and_unwind_before_recovery() {
    let package = Package::default();
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::from([(X, Value::Bool(false)), (TRACE, Value::Bool(false))]);
        for reject_first in [false, true, false] {
            let body = expr(K::Match {
                value: Box::new(Expr {
                    source: SOURCE,
                    ty: Term::Primitive(Primitive::Type),
                    kind: K::TypeValue(Term::Tuple(vec![
                        Term::Primitive(Primitive::Bool),
                        Term::Primitive(Primitive::Bool),
                    ])),
                }),
                arms: vec![
                    MatchArm {
                        pattern: P::TypeTuple(vec![
                            P::Then {
                                first: Box::new(binding(X)),
                                value: Box::new(effect()),
                                next: Box::new(P::Literal(Literal::Bool(reject_first))),
                            },
                            P::Then {
                                first: Box::new(P::Wild),
                                value: Box::new(expr(K::Return(Some(Box::new(boolean(true)))))),
                                next: Box::new(P::Wild),
                            },
                        ]),
                        guard: None,
                        body: boolean(false),
                    },
                    MatchArm {
                        pattern: P::Wild,
                        guard: None,
                        body: boolean(false),
                    },
                ],
            });
            let result = execute(&mut vm, &body, &mut env, compiled).unwrap_or_else(|e| {
                panic!("compiled={compiled} reject_first={reject_first}: {e:?}")
            });
            if reject_first {
                assert!(matches!(result, Flow::Value(Value::Bool(false))));
            } else {
                assert!(matches!(result, Flow::Return(Value::Bool(true))));
            }
            assert_eq!(
                vm.local_value(&env, X, Some(SOURCE)).unwrap(),
                Value::Bool(false)
            );
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn nominal_type_fields_convert_against_the_completed_telescope() {
    let id = |local| DeclarationId { package: 1, local };
    let mut package = Package::default();
    package.structs.push(nir::program::Struct {
        declaration: id(1),
        name: "Dependent".into(),
        parameter_types: vec![
            Term::Primitive(Primitive::Type),
            Term::Rigid {
                declaration: id(2),
                sort: nir::Sort::Type,
            },
        ],
        type_parameters: vec![id(2), id(3)],
        capabilities: vec![],
        requirements: vec![],
        fields: vec![],
    });
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for flag in [false, true] {
            let body = expr(K::Match {
                value: Box::new(Expr {
                    source: SOURCE,
                    ty: Term::Primitive(Primitive::Type),
                    kind: K::TypeValue(Term::Nominal {
                        declaration: id(1),
                        arguments: vec![Term::Primitive(Primitive::Bool), Term::Bool(flag)],
                    }),
                }),
                arms: vec![MatchArm {
                    pattern: P::NominalType {
                        declaration: id(1),
                        fields: vec![
                            P::Wild,
                            P::Then {
                                first: Box::new(binding(X)),
                                value: Box::new(expr(K::Not(Box::new(expr(K::Local(X)))))),
                                next: Box::new(binding(Y)),
                            },
                        ],
                    },
                    guard: None,
                    body: expr(K::Local(Y)),
                }],
            });
            assert!(
                matches!(execute(&mut vm, &body, &mut Environment::new(), compiled).unwrap(), Flow::Value(Value::Bool(value)) if value == !flag)
            );
        }
    }
}

#[test]
fn leaf_tests_preserve_domains_and_unresolved_input_diagnostics() {
    let package = Package::default();
    let declaration = DeclarationId {
        package: 1,
        local: 99,
    };
    let cases = vec![
        (P::Wild, Value::TypeParameter(declaration), Ok(true)),
        (P::Unit, Value::Unit, Ok(true)),
        (P::Unit, Value::Bool(false), Ok(false)),
        (
            P::Literal(Literal::Float32(f32::NAN.to_bits())),
            Value::Float32(f32::NAN.to_bits()),
            Ok(false),
        ),
        (
            P::Literal(Literal::Float64(0.0f64.to_bits())),
            Value::Float64((-0.0f64).to_bits()),
            Ok(true),
        ),
        (
            P::Literal(Literal::Symbol("雪".into())),
            Value::String("雪".into()),
            Ok(false),
        ),
        (
            P::Literal(Literal::String("雪\n\"".into())),
            Value::String("雪\n\"".into()),
            Ok(true),
        ),
        (
            P::Unit,
            Value::TypeParameter(declaration),
            Err(ErrorKind::TypeContractUnresolved),
        ),
        (
            P::OptionalAbsent,
            Value::AbstractValue {
                declaration,
                ty: Term::Primitive(Primitive::Bool),
            },
            Err(ErrorKind::AbstractValueUnresolved),
        ),
        (
            P::OptionalAbsent,
            Value::Optional {
                ty: Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
                value: None,
            },
            Ok(true),
        ),
        (
            P::OptionalAbsent,
            Value::Optional {
                ty: Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
                value: Some(Box::new(Value::Bool(false))),
            },
            Ok(false),
        ),
    ];
    for (pattern, input, expected) in cases {
        let body = expr(K::Match {
            value: Box::new(expr(K::Local(X))),
            arms: vec![
                MatchArm {
                    pattern,
                    guard: None,
                    body: boolean(true),
                },
                MatchArm {
                    pattern: P::Wild,
                    guard: None,
                    body: boolean(false),
                },
            ],
        });
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env = Environment::from([(X, input.clone())]);
            let actual = match execute(&mut vm, &body, &mut env, compiled) {
                Ok(Flow::Value(Value::Bool(value))) => Ok(value),
                Err(error) => Err(error.kind),
                _ => panic!("leaf match must return bool"),
            };
            assert_eq!(actual, expected, "compiled={compiled}, input={input:?}");
        }
    }
}

#[test]
fn compiled_text_leaf_tests_charge_comparison_work() {
    let package = Package::default();
    let text = "雪".repeat(200);
    let body = expr(K::Match {
        value: Box::new(expr(K::Local(X))),
        arms: vec![MatchArm {
            pattern: P::Literal(Literal::String(text.clone())),
            guard: None,
            body: boolean(true),
        }],
    });
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 32,
                depth: 128,
            },
        )
        .unwrap();
        let mut env = Environment::from([(X, Value::String(text.clone().into()))]);
        assert_eq!(
            execute(&mut vm, &body, &mut env, compiled)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
    }
}
