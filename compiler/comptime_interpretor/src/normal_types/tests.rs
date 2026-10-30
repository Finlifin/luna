use super::*;
use nir::normal_forms::{Head, Kind, Parameter, Role};
use nir::program::{Literal, NormalFormArgument as A, Place};
const SOURCE: SourceRange = SourceRange { start: 1, end: 9 };
fn expr(ty: Primitive, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty: Term::Primitive(ty),
        kind,
    }
}

#[test]
fn sequential_normal_inputs_preserve_identity_order_and_recovery() {
    let head = DeclarationId {
        package: 3,
        local: 1,
    };
    let parameter = DeclarationId {
        package: 3,
        local: 2,
    };
    let trace = LocalId { owner: 5, index: 0 };
    let package = Package {
        normal_forms: vec![Head {
            declaration: head,
            collector: None,
            role: Role::Opaque,
            requirements: vec![],
            parameters: vec![
                Parameter {
                    declaration: parameter,
                    binding: LocalId { owner: 1, index: 0 },
                    kind: Kind::Type,
                    ty: Term::Primitive(Primitive::Type),
                },
                Parameter {
                    declaration: DeclarationId {
                        package: 3,
                        local: 3,
                    },
                    binding: LocalId { owner: 1, index: 1 },
                    kind: Kind::Value,
                    ty: Term::Rigid {
                        declaration: parameter,
                        sort: nir::Sort::Type,
                    },
                },
            ],
        }],
        ..Default::default()
    };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for invalid in [true, false, true, false] {
            let body = expr(
                Primitive::Type,
                K::NormalFormType {
                    head,
                    plan: None,
                    arguments: vec![
                        A::Value(expr(
                            Primitive::Type,
                            K::TypeValue(Term::Primitive(Primitive::Bool)),
                        )),
                        A::Value(expr(
                            if invalid {
                                Primitive::Char
                            } else {
                                Primitive::Bool
                            },
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
                                            K::Literal(Literal::Bool(true)),
                                        )),
                                    },
                                )],
                                result: Some(Box::new(expr(
                                    Primitive::Bool,
                                    K::Literal(Literal::Bool(false)),
                                ))),
                            },
                        )),
                    ],
                },
            );
            let mut env = Environment::from([(trace, Value::Bool(false))]);
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                assert!(code.to_string().contains("(begin-normal-type "));
                vm.execute_code(&code, &mut env)
            } else {
                vm.eval(&body, &mut env)
            };
            if invalid {
                assert!(result.is_err());
            } else {
                let Flow::Value(value) = result.unwrap() else {
                    panic!("expected value")
                };
                assert_eq!(
                    value,
                    Value::Type(Term::NormalForm {
                        head,
                        sort: nir::Sort::Type,
                        arguments: vec![Term::Primitive(Primitive::Bool), Term::Bool(false)],
                    })
                );
            }
            assert_eq!(env[&trace], Value::Bool(!invalid));
        }
    }
}

#[test]
fn final_requirements_reject_and_next_construction_recovers() {
    let head = DeclarationId {
        package: 4,
        local: 1,
    };
    let parameter = DeclarationId {
        package: 4,
        local: 2,
    };
    let package = Package {
        normal_forms: vec![Head {
            declaration: head,
            collector: None,
            role: Role::Opaque,
            requirements: vec![nir::program::TypePredicate::Equal(
                Term::Rigid {
                    declaration: parameter,
                    sort: nir::Sort::Type,
                },
                Term::Primitive(Primitive::Bool),
            )],
            parameters: vec![Parameter {
                declaration: parameter,
                binding: LocalId { owner: 1, index: 0 },
                kind: Kind::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
        }],
        ..Default::default()
    };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for primitive in [
            Primitive::Char,
            Primitive::Bool,
            Primitive::Char,
            Primitive::Bool,
        ] {
            let body = expr(
                Primitive::Type,
                K::NormalFormType {
                    head,
                    plan: None,
                    arguments: vec![A::Value(expr(
                        Primitive::Type,
                        K::TypeValue(Term::Primitive(primitive)),
                    ))],
                },
            );
            let mut env = Environment::new();
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                vm.execute_code(&code, &mut env)
            } else {
                vm.eval(&body, &mut env)
            };
            assert_eq!(result.is_ok(), primitive == Primitive::Bool);
        }
    }
}

#[test]
fn malformed_normal_constructor_rejects_before_operand_effects() {
    let head = DeclarationId {
        package: 6,
        local: 1,
    };
    let trace = LocalId { owner: 1, index: 7 };
    let package = Package {
        normal_forms: vec![Head {
            declaration: head,
            collector: None,
            role: Role::Opaque,
            requirements: vec![],
            parameters: vec![Parameter {
                declaration: DeclarationId {
                    package: 6,
                    local: 2,
                },
                binding: LocalId { owner: 1, index: 0 },
                kind: Kind::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
        }],
        ..Default::default()
    };
    let effect = A::Value(expr(
        Primitive::Type,
        K::Block {
            statements: vec![expr(
                Primitive::Void,
                K::Assign {
                    place: Place {
                        root: trace,
                        fields: vec![],
                    },
                    value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(true)))),
                },
            )],
            result: Some(Box::new(expr(
                Primitive::Type,
                K::TypeValue(Term::Primitive(Primitive::Bool)),
            ))),
        },
    ));
    for compiled in [false, true] {
        for case in 0..3 {
            let body = expr(
                if case == 0 {
                    Primitive::Bool
                } else {
                    Primitive::Type
                },
                K::NormalFormType {
                    head: if case == 1 {
                        DeclarationId {
                            package: 6,
                            local: 9,
                        }
                    } else {
                        head
                    },
                    plan: None,
                    arguments: if case == 2 {
                        vec![effect.clone(), effect.clone()]
                    } else {
                        vec![effect.clone()]
                    },
                },
            );
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env = Environment::from([(trace, Value::Bool(false))]);
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                vm.execute_code(&code, &mut env)
            } else {
                vm.eval(&body, &mut env)
            };
            assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            assert_eq!(env[&trace], Value::Bool(false));
        }
        let body = expr(
            Primitive::Never,
            K::NormalFormType {
                head,
                plan: None,
                arguments: vec![A::Value(expr(
                    Primitive::Never,
                    K::Return(Some(Box::new(expr(
                        Primitive::Bool,
                        K::Literal(Literal::Bool(true)),
                    )))),
                ))],
            },
        );
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        let result = if compiled {
            let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
            vm.execute_code(&code, &mut env)
        } else {
            vm.eval(&body, &mut env)
        };
        assert!(matches!(result.unwrap(), Flow::Return(Value::Bool(true))));
    }
}

#[test]
fn contextual_scalars_use_the_preceding_type_input() {
    let head = DeclarationId {
        package: 8,
        local: 1,
    };
    let parameter = DeclarationId {
        package: 8,
        local: 2,
    };
    let package = Package {
        normal_forms: vec![Head {
            declaration: head,
            collector: None,
            role: Role::Opaque,
            requirements: vec![],
            parameters: vec![
                Parameter {
                    declaration: parameter,
                    binding: LocalId { owner: 1, index: 0 },
                    kind: Kind::Type,
                    ty: Term::Primitive(Primitive::Type),
                },
                Parameter {
                    declaration: DeclarationId {
                        package: 8,
                        local: 3,
                    },
                    binding: LocalId { owner: 1, index: 1 },
                    kind: Kind::Value,
                    ty: Term::Rigid {
                        declaration: parameter,
                        sort: nir::Sort::Type,
                    },
                },
            ],
        }],
        ..Default::default()
    };
    let integer = |spelling: &str, negative| A::Integer {
        source: SOURCE,
        spelling: spelling.into(),
        negative,
    };
    for (ty, input, valid) in [
        (
            Primitive::Integer,
            integer("123456789012345678901234567890", false),
            true,
        ),
        (Primitive::I8, integer("128", true), true),
        (Primitive::I8, integer("129", true), false),
        (Primitive::U8, integer("255", false), true),
        (Primitive::U8, integer("256", false), false),
        (Primitive::U8, integer("1", true), false),
        (Primitive::BitVec(9), integer("511", false), true),
        (Primitive::Bool, integer("1", false), false),
        (Primitive::Type, A::Unit { source: SOURCE }, true),
        (Primitive::Void, A::Unit { source: SOURCE }, true),
        (Primitive::Bool, A::Unit { source: SOURCE }, false),
    ] {
        let body = expr(
            Primitive::Type,
            K::NormalFormType {
                head,
                plan: None,
                arguments: vec![
                    A::Value(expr(Primitive::Type, K::TypeValue(Term::Primitive(ty)))),
                    input,
                ],
            },
        );
        let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
        assert!(code.to_string().contains("(normal-literal "));
        let mut tree = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let expected = tree.eval(&body, &mut Environment::new());
        let actual = vm.execute_code(&code, &mut Environment::new());
        assert_eq!(expected.is_ok(), valid, "{ty}");
        match (expected, actual) {
            (Ok(Flow::Value(expected)), Ok(Flow::Value(actual))) => {
                assert_eq!(actual, expected);
                if ty == Primitive::Type || ty == Primitive::Void {
                    assert_eq!(
                        actual,
                        Value::Type(Term::NormalForm {
                            head,
                            sort: nir::Sort::Type,
                            arguments: vec![
                                Term::Primitive(ty),
                                if ty == Primitive::Type {
                                    Term::Primitive(Primitive::Void)
                                } else {
                                    Term::Unit
                                }
                            ],
                        })
                    );
                }
            }
            (Err(expected), Err(actual)) => assert_eq!(actual.kind, expected.kind),
            pair => panic!("tree/bytecode mismatch: {pair:?}"),
        }
    }
}

#[test]
fn diverging_annotation_cannot_produce_a_normal_input_value() {
    let head = DeclarationId {
        package: 8,
        local: 10,
    };
    let package = Package {
        normal_forms: vec![Head {
            declaration: head,
            collector: None,
            role: Role::Opaque,
            requirements: vec![],
            parameters: vec![Parameter {
                declaration: DeclarationId {
                    package: 8,
                    local: 11,
                },
                binding: LocalId { owner: 1, index: 0 },
                kind: Kind::Value,
                ty: Term::Primitive(Primitive::Bool),
            }],
        }],
        ..Default::default()
    };
    let body = expr(
        Primitive::Never,
        K::NormalFormType {
            head,
            plan: None,
            arguments: vec![A::Value(expr(
                Primitive::Never,
                K::Literal(Literal::Bool(true)),
            ))],
        },
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
    assert_eq!(
        vm.eval(&body, &mut Environment::new()).unwrap_err().kind,
        ErrorKind::Type
    );
    assert_eq!(
        vm.execute_code(&code, &mut Environment::new())
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
}

fn container_package() -> Package {
    let id = |local| DeclarationId { package: 12, local };
    let param = |local, index, kind, ty| Parameter {
        declaration: id(local),
        binding: LocalId { owner: 12, index },
        kind,
        ty,
    };
    Package {
        normal_forms: vec![
            Head {
                declaration: id(1),
                collector: None,
                role: Role::Opaque,
                requirements: vec![],
                parameters: vec![
                    param(2, 0, Kind::Type, Term::Primitive(Primitive::Type)),
                    param(
                        3,
                        1,
                        Kind::Value,
                        Term::Rigid {
                            declaration: id(2),
                            sort: nir::Sort::Type,
                        },
                    ),
                ],
            },
            Head {
                declaration: id(4),
                collector: None,
                role: Role::TypeList,
                requirements: vec![],
                parameters: vec![param(5, 0, Kind::Type, Term::Primitive(Primitive::Type))],
            },
            Head {
                declaration: id(6),
                collector: Some(0),
                role: Role::Tuple,
                requirements: vec![],
                parameters: vec![param(
                    7,
                    0,
                    Kind::Value,
                    container_list(Term::Primitive(Primitive::Type)),
                )],
            },
        ],
        ..Default::default()
    }
}
fn container_list(element: Term) -> Term {
    Term::NormalForm {
        head: DeclarationId {
            package: 12,
            local: 4,
        },
        sort: nir::Sort::Type,
        arguments: vec![element],
    }
}
fn container_body(domain: Term, input: A) -> Expr {
    expr(
        Primitive::Type,
        K::NormalFormType {
            head: DeclarationId {
                package: 12,
                local: 1,
            },
            plan: None,
            arguments: vec![A::Value(expr(Primitive::Type, K::TypeValue(domain))), input],
        },
    )
}

#[test]
fn nested_contextual_containers_preserve_domains_and_tuple_heads() {
    let package = container_package();
    let integer = || A::Integer {
        source: SOURCE,
        spelling: "17".into(),
        negative: false,
    };
    let list = |items| A::List {
        source: SOURCE,
        items,
    };
    let tuple = |items| A::Tuple {
        source: SOURCE,
        type_head: None,
        items,
    };
    let boolean = || A::Value(expr(Primitive::Bool, K::Literal(Literal::Bool(true))));
    let tuple_head = DeclarationId {
        package: 12,
        local: 6,
    };
    for (domain, input) in [
        (
            container_list(container_list(Term::Primitive(Primitive::U8))),
            list(vec![list(vec![integer()]), list(vec![])]),
        ),
        (
            Term::Tuple(vec![
                Term::Primitive(Primitive::U8),
                container_list(Term::Primitive(Primitive::Bool)),
            ]),
            tuple(vec![integer(), list(vec![boolean()])]),
        ),
        (
            Term::Primitive(Primitive::Type),
            A::Tuple {
                source: SOURCE,
                type_head: Some(tuple_head),
                items: vec![
                    A::Value(expr(
                        Primitive::Type,
                        K::TypeValue(Term::Primitive(Primitive::Bool)),
                    )),
                    A::Unit { source: SOURCE },
                ],
            },
        ),
    ] {
        let body = container_body(domain.clone(), input);
        let code = Arc::new(comptime_compiler::compile(&body, &mut 100000).unwrap());
        assert!(code.to_string().contains("(finish-normal-container "));
        let mut tree = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let Flow::Value(expected) = tree.eval(&body, &mut Environment::new()).unwrap() else {
            panic!("value")
        };
        let Flow::Value(actual) = vm.execute_code(&code, &mut Environment::new()).unwrap() else {
            panic!("value")
        };
        assert_eq!(actual, expected);
        if domain == Term::Primitive(Primitive::Type) {
            assert_eq!(
                actual,
                Value::Type(Term::NormalForm {
                    head: DeclarationId {
                        package: 12,
                        local: 1
                    },
                    sort: nir::Sort::Type,
                    arguments: vec![
                        domain,
                        Term::NormalForm {
                            head: tuple_head,
                            sort: nir::Sort::Type,
                            arguments: vec![frozen_types(vec![
                                Term::Primitive(Primitive::Bool),
                                Term::Primitive(Primitive::Void)
                            ])]
                        }
                    ],
                })
            );
        }
    }
}

#[test]
fn contextual_container_checks_do_not_reorder_operand_effects() {
    let package = container_package();
    let source = LocalId {
        owner: 20,
        index: 0,
    };
    let trace = LocalId {
        owner: 20,
        index: 1,
    };
    let list_ty = container_list(Term::Primitive(Primitive::Type));
    let effect = A::Value(expr(
        Primitive::Type,
        K::Block {
            statements: vec![expr(
                Primitive::Void,
                K::Assign {
                    place: Place {
                        root: trace,
                        fields: vec![],
                    },
                    value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(true)))),
                },
            )],
            result: Some(Box::new(expr(
                Primitive::Type,
                K::TypeValue(Term::Primitive(Primitive::Bool)),
            ))),
        },
    ));
    for compiled in [false, true] {
        for valid in [false, true, false, true] {
            let body = container_body(
                list_ty.clone(),
                A::List {
                    source: SOURCE,
                    items: vec![
                        A::ListSpread(Expr {
                            source: SOURCE,
                            ty: list_ty.clone(),
                            kind: K::Local(source),
                        }),
                        effect.clone(),
                    ],
                },
            );
            let mut env = Environment::from([
                (
                    source,
                    if valid {
                        Value::List {
                            ty: list_ty.clone(),
                            values: vec![Value::Type(Term::Primitive(Primitive::Char))],
                        }
                    } else {
                        Value::Bool(false)
                    },
                ),
                (trace, Value::Bool(false)),
            ]);
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 100000).unwrap());
                vm.execute_code(&code, &mut env)
            } else {
                vm.eval(&body, &mut env)
            };
            assert_eq!(result.is_ok(), valid);
            assert_eq!(env[&trace], Value::Bool(valid));
        }
        // Preparation checks even a later contextual literal before any code runs.
        let body = container_body(
            list_ty.clone(),
            A::List {
                source: SOURCE,
                items: vec![
                    effect.clone(),
                    A::Integer {
                        source: SOURCE,
                        spelling: "1".into(),
                        negative: false,
                    },
                ],
            },
        );
        let mut env = Environment::from([(trace, Value::Bool(false))]);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let result = if compiled {
            let code = Arc::new(comptime_compiler::compile(&body, &mut 100000).unwrap());
            vm.execute_code(&code, &mut env)
        } else {
            vm.eval(&body, &mut env)
        };
        assert!(result.is_err());
        assert_eq!(env[&trace], Value::Bool(false));
    }
}

#[test]
fn contextual_choices_prepare_both_branches_and_clean_selected_blocks() {
    let package = container_package();
    let flag = LocalId {
        owner: 21,
        index: 0,
    };
    let observed = LocalId {
        owner: 21,
        index: 1,
    };
    let selected = LocalId {
        owner: 21,
        index: 2,
    };
    let temporary = LocalId {
        owner: 21,
        index: 3,
    };
    let set = |binding, value| {
        expr(
            Primitive::Void,
            K::Assign {
                place: Place {
                    root: binding,
                    fields: vec![],
                },
                value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(value)))),
            },
        )
    };
    let branch = |choice, invalid| A::Block {
        source: SOURCE,
        statements: vec![
            expr(
                Primitive::Void,
                K::Let {
                    binding: temporary,
                    value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(choice)))),
                },
            ),
            set(selected, choice),
        ],
        result: Some(Box::new(A::Integer {
            source: SOURCE,
            spelling: if invalid {
                "256"
            } else if choice {
                "17"
            } else {
                "23"
            }
            .into(),
            negative: false,
        })),
    };
    for compiled in [false, true] {
        for invalid in [false, true] {
            for choice in [false, true] {
                let body = container_body(
                    Term::Primitive(Primitive::U8),
                    A::If {
                        source: SOURCE,
                        condition: expr(
                            Primitive::Bool,
                            K::Block {
                                statements: vec![set(observed, true)],
                                result: Some(Box::new(expr(Primitive::Bool, K::Local(flag)))),
                            },
                        ),
                        then_input: Box::new(branch(true, false)),
                        else_input: Box::new(branch(false, invalid)),
                    },
                );
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let mut env = Environment::from([
                    (flag, Value::Bool(choice)),
                    (observed, Value::Bool(false)),
                    (selected, Value::Bool(!choice)),
                ]);
                let result = if compiled {
                    let code = Arc::new(comptime_compiler::compile(&body, &mut 100000).unwrap());
                    vm.execute_code(&code, &mut env)
                } else {
                    vm.eval(&body, &mut env)
                };
                if invalid {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                    assert_eq!(env[&observed], Value::Bool(false));
                    assert_eq!(env[&selected], Value::Bool(!choice));
                } else {
                    let Flow::Value(value) = result.unwrap() else {
                        panic!("value")
                    };
                    assert_eq!(
                        value,
                        Value::Type(Term::NormalForm {
                            head: DeclarationId {
                                package: 12,
                                local: 1
                            },
                            sort: nir::Sort::Type,
                            arguments: vec![
                                Term::Primitive(Primitive::U8),
                                Term::Integer(comptime_runtime::Integer::from_u128(if choice {
                                    17
                                } else {
                                    23
                                }))
                            ],
                        })
                    );
                    assert_eq!(env[&observed], Value::Bool(true));
                    assert_eq!(env[&selected], Value::Bool(choice));
                }
                assert!(!env.contains_key(&temporary));
            }
        }
    }
}

#[test]
fn contextual_blocks_preserve_unit_and_early_return_boundaries() {
    let package = container_package();
    let temporary = LocalId {
        owner: 22,
        index: 0,
    };
    for compiled in [false, true] {
        for early in [false, true] {
            let mut statements = vec![expr(
                Primitive::Void,
                K::Let {
                    binding: temporary,
                    value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(true)))),
                },
            )];
            if early {
                statements.push(expr(
                    Primitive::Never,
                    K::Return(Some(Box::new(expr(Primitive::Bool, K::Local(temporary))))),
                ));
            }
            let mut body = container_body(
                Term::Primitive(Primitive::Void),
                A::Block {
                    source: SOURCE,
                    statements,
                    result: None,
                },
            );
            if early {
                body.ty = Term::Primitive(Primitive::Never);
            }
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env = Environment::new();
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 100000).unwrap());
                vm.execute_code(&code, &mut env)
            } else {
                vm.eval(&body, &mut env)
            };
            if early {
                assert!(matches!(result.unwrap(), Flow::Return(Value::Bool(true))));
            } else {
                assert!(matches!(result.unwrap(), Flow::Value(Value::Type(_))));
            }
            assert!(env.is_empty());
        }
    }
}

#[test]
fn contextual_matches_preserve_guard_effects_and_binding_rollback() {
    use nir::program::{NormalMatchArm, Pattern};
    let package = container_package();
    let flag = LocalId {
        owner: 23,
        index: 0,
    };
    let subject_seen = LocalId {
        owner: 23,
        index: 1,
    };
    let guard_seen = LocalId {
        owner: 23,
        index: 2,
    };
    let wrong_body_seen = LocalId {
        owner: 23,
        index: 3,
    };
    let captured = LocalId {
        owner: 23,
        index: 4,
    };
    let set = |binding| {
        expr(
            Primitive::Void,
            K::Assign {
                place: Place {
                    root: binding,
                    fields: vec![],
                },
                value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(true)))),
            },
        )
    };
    let integer = |number: &str| A::Integer {
        source: SOURCE,
        spelling: number.into(),
        negative: false,
    };
    for compiled in [false, true] {
        for embedded_guard in [false, true] {
            for invalid in [false, true] {
                for choice in [false, true] {
                    let pattern = Pattern::Binding {
                        binding: captured,
                        inner: Box::new(Pattern::Wild),
                    };
                    let guard = expr(
                        Primitive::Bool,
                        K::Block {
                            statements: vec![set(guard_seen)],
                            result: Some(Box::new(expr(
                                Primitive::Bool,
                                K::Literal(Literal::Bool(false)),
                            ))),
                        },
                    );
                    let (pattern, guard) = if embedded_guard {
                        (
                            Pattern::Guard {
                                pattern: Box::new(pattern),
                                condition: Box::new(guard),
                            },
                            None,
                        )
                    } else {
                        (pattern, Some(guard))
                    };
                    let body = container_body(
                        Term::Primitive(Primitive::U8),
                        A::Match {
                            source: SOURCE,
                            value: expr(
                                Primitive::Bool,
                                K::Block {
                                    statements: vec![set(subject_seen)],
                                    result: Some(Box::new(expr(Primitive::Bool, K::Local(flag)))),
                                },
                            ),
                            arms: vec![
                                NormalMatchArm {
                                    pattern,
                                    guard,
                                    body: A::Block {
                                        source: SOURCE,
                                        statements: vec![set(wrong_body_seen)],
                                        result: Some(Box::new(integer("17"))),
                                    },
                                },
                                NormalMatchArm {
                                    pattern: Pattern::Literal(Literal::Bool(true)),
                                    guard: None,
                                    body: integer("23"),
                                },
                                NormalMatchArm {
                                    pattern: Pattern::Wild,
                                    guard: None,
                                    body: integer(if invalid { "256" } else { "31" }),
                                },
                            ],
                        },
                    );
                    let mut env = Environment::from([
                        (flag, Value::Bool(choice)),
                        (subject_seen, Value::Bool(false)),
                        (guard_seen, Value::Bool(false)),
                        (wrong_body_seen, Value::Bool(false)),
                    ]);
                    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                    let result = if compiled {
                        let code =
                            Arc::new(comptime_compiler::compile(&body, &mut 100000).unwrap());
                        vm.execute_code(&code, &mut env)
                    } else {
                        vm.eval(&body, &mut env)
                    };
                    if invalid {
                        assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                    } else {
                        let Flow::Value(actual) = result.unwrap() else {
                            panic!("value")
                        };
                        assert_eq!(
                            actual,
                            Value::Type(Term::NormalForm {
                                head: DeclarationId {
                                    package: 12,
                                    local: 1
                                },
                                sort: nir::Sort::Type,
                                arguments: vec![
                                    Term::Primitive(Primitive::U8),
                                    Term::Integer(comptime_runtime::Integer::from_u128(
                                        if choice { 23 } else { 31 }
                                    ))
                                ]
                            })
                        );
                    }
                    assert_eq!(env[&subject_seen], Value::Bool(!invalid));
                    assert_eq!(env[&guard_seen], Value::Bool(!invalid));
                    assert_eq!(env[&wrong_body_seen], Value::Bool(false));
                    assert!(!env.contains_key(&captured));
                }
            }
        }
    }
}

#[test]
fn contextual_match_failure_and_guard_return_clean_bindings() {
    use nir::program::{NormalMatchArm, Pattern};
    let package = container_package();
    let captured = LocalId {
        owner: 24,
        index: 0,
    };
    for compiled in [false, true] {
        for case in 0..4 {
            let mut body = container_body(
                Term::Primitive(Primitive::U8),
                A::Match {
                    source: SOURCE,
                    value: expr(
                        if case == 3 {
                            Primitive::Never
                        } else {
                            Primitive::Bool
                        },
                        K::Literal(Literal::Bool(true)),
                    ),
                    arms: vec![NormalMatchArm {
                        pattern: Pattern::Binding {
                            binding: captured,
                            inner: Box::new(Pattern::Wild),
                        },
                        guard: if case == 3 {
                            None
                        } else {
                            Some(if case == 1 {
                                expr(
                                    Primitive::Never,
                                    K::Return(Some(Box::new(expr(
                                        Primitive::Bool,
                                        K::Local(captured),
                                    )))),
                                )
                            } else {
                                expr(
                                    if case == 2 {
                                        Primitive::Char
                                    } else {
                                        Primitive::Bool
                                    },
                                    K::Literal(Literal::Bool(false)),
                                )
                            })
                        },
                        body: A::Integer {
                            source: SOURCE,
                            spelling: "17".into(),
                            negative: false,
                        },
                    }],
                },
            );
            if case == 3 {
                body.ty = Term::Primitive(Primitive::Never);
            }
            let mut env = Environment::new();
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 100000).unwrap());
                vm.execute_code(&code, &mut env)
            } else {
                vm.eval(&body, &mut env)
            };
            if case == 1 {
                assert!(matches!(result.unwrap(), Flow::Return(Value::Bool(true))));
            } else {
                assert_eq!(
                    result.unwrap_err().kind,
                    if case == 0 {
                        ErrorKind::NonExhaustiveMatch
                    } else {
                        ErrorKind::Type
                    }
                );
            }
            assert!(env.is_empty());
        }
    }
}

#[test]
fn normal_bindings_preserve_declaration_order_and_reject_duplicate_slots() {
    let package = container_package();
    let head = package.normal_forms[0].declaration;
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut bindings = normal_types::Bindings::new(2);
    vm.bind_normal_input(head, &mut bindings, 1, Value::Bool(true), SOURCE)
        .unwrap();
    assert!(bindings.contains(1));
    assert!(!bindings.contains(0));
    assert!(bindings.to_string().contains("(slot 0 pending)"));
    for slot in [1, 2] {
        assert_eq!(
            vm.bind_normal_input(head, &mut bindings, slot, Value::Bool(false), SOURCE)
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
    }
    vm.bind_normal_input(
        head,
        &mut bindings,
        0,
        Value::Type(Term::Primitive(Primitive::Bool)),
        SOURCE,
    )
    .unwrap();
    assert_eq!(bindings.parameters[0].sort, nir::Sort::ComptimeValue);
    assert_eq!(bindings.parameters[1].sort, nir::Sort::Type);
    assert_eq!(
        bindings.finish().unwrap(),
        vec![Term::Primitive(Primitive::Bool), Term::Bool(true)]
    );
    assert!(normal_types::Bindings::new(1).finish().is_none());
    assert_eq!(normal_types::Bindings::new(0).finish(), Some(vec![]));
}

#[test]
fn planned_contextual_inputs_recover_after_deferred_literal_failure() {
    use nir::program::NormalFormInputPlan;
    let package = container_package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let locals = vec![
        LocalId {
            owner: 90,
            index: 0,
        },
        LocalId {
            owner: 90,
            index: 1,
        },
    ];
    let mut environment = Environment::new();
    for spelling in ["256", "17", "256", "23"] {
        let mut body = container_body(
            Term::Primitive(Primitive::U8),
            A::Integer {
                source: SOURCE,
                spelling: spelling.into(),
                negative: false,
            },
        );
        let K::NormalFormType { plan, head, .. } = &mut body.kind else {
            unreachable!()
        };
        let head = *head;
        *plan = Some(NormalFormInputPlan {
            order: vec![1, 0],
            bindings: locals.clone(),
            preparations: vec![],
        });
        let result = vm.eval(&body, &mut environment);
        if spelling == "256" {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
        } else {
            let Flow::Value(Value::Type(Term::NormalForm {
                head: actual,
                arguments,
                ..
            })) = result.unwrap()
            else {
                panic!("expected opaque normal form")
            };
            assert_eq!(actual, head);
            assert_eq!(
                arguments,
                vec![
                    Term::Primitive(Primitive::U8),
                    Term::Integer(comptime_runtime::Integer::from_u128(
                        spelling.parse().unwrap()
                    )),
                ]
            );
        }
        assert!(locals.iter().all(|local| !environment.contains_key(local)));
    }
}

#[test]
fn contextual_type_tuples_accept_scoped_parameters_in_every_input_order() {
    use nir::program::NormalFormInputPlan;
    let package = container_package();
    let authorized = DeclarationId {
        package: 12,
        local: 90,
    };
    let foreign = DeclarationId {
        package: 12,
        local: 91,
    };
    let local = LocalId {
        owner: 90,
        index: 0,
    };
    let plan_locals = vec![
        LocalId {
            owner: 90,
            index: 1,
        },
        LocalId {
            owner: 90,
            index: 2,
        },
    ];
    let tuple_head = DeclarationId {
        package: 12,
        local: 6,
    };
    for mode in 0..4 {
        let mut body = container_body(
            Term::Primitive(Primitive::Type),
            A::Tuple {
                source: SOURCE,
                type_head: Some(tuple_head),
                items: vec![
                    A::Value(expr(Primitive::Type, K::Local(local))),
                    A::Value(expr(
                        Primitive::Type,
                        K::TypeValue(Term::Primitive(Primitive::U8)),
                    )),
                ],
            },
        );
        if mode >= 2 {
            let K::NormalFormType { plan, .. } = &mut body.kind else {
                unreachable!()
            };
            *plan = Some(NormalFormInputPlan {
                order: if mode == 2 { vec![0, 1] } else { vec![1, 0] },
                bindings: plan_locals.clone(),
                preparations: vec![],
            });
        }
        let code =
            (mode == 1).then(|| Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap()));
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_abstract_type_parameters(&[authorized])
            .unwrap();
        for declaration in [foreign, authorized, foreign, authorized] {
            let mut env = Environment::new();
            env.insert(local, Value::TypeParameter(declaration));
            let result = if let Some(code) = &code {
                vm.execute_code(code, &mut env)
            } else {
                vm.eval(&body, &mut env)
            };
            if declaration == foreign {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type, "mode {mode}");
            } else {
                let Flow::Value(actual) =
                    result.unwrap_or_else(|error| panic!("mode {mode}: {error:?}"))
                else {
                    panic!("transfer")
                };
                assert_eq!(
                    actual,
                    Value::Type(Term::NormalForm {
                        head: package.normal_forms[0].declaration,
                        sort: nir::Sort::Type,
                        arguments: vec![
                            Term::Primitive(Primitive::Type),
                            Term::NormalForm {
                                head: tuple_head,
                                sort: nir::Sort::Type,
                                arguments: vec![frozen_types(vec![
                                    Term::Rigid {
                                        declaration: authorized,
                                        sort: nir::Sort::Type
                                    },
                                    Term::Primitive(Primitive::U8),
                                ])],
                            }
                        ],
                    }),
                    "mode {mode}"
                );
            }
            assert!(plan_locals.iter().all(|local| !env.contains_key(local)));
            assert_eq!(env.get(&local), Some(&Value::TypeParameter(declaration)));
            if mode == 3 {
                assert!(vm.bytecode_staged_inputs() > 0);
            }
        }
    }
}

#[test]
fn staged_list_spreads_finish_after_all_operand_effects() {
    let package = container_package();
    let ty = container_list(Term::Primitive(Primitive::U8));
    let seed = LocalId {
        owner: 92,
        index: 0,
    };
    let touched = LocalId {
        owner: 92,
        index: 1,
    };
    let input = A::List {
        source: SOURCE,
        items: vec![
            A::ListSpread(Expr {
                source: SOURCE,
                ty: ty.clone(),
                kind: K::Local(seed),
            }),
            A::Block {
                source: SOURCE,
                statements: vec![expr(
                    Primitive::Void,
                    K::Assign {
                        place: Place {
                            root: touched,
                            fields: vec![],
                        },
                        value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(true)))),
                    },
                )],
                result: Some(Box::new(A::Integer {
                    source: SOURCE,
                    spelling: "29".into(),
                    negative: false,
                })),
            },
        ],
    };
    for invalid in [true, false] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        env.insert(touched, Value::Bool(false));
        env.insert(
            seed,
            if invalid {
                Value::List {
                    ty: container_list(Term::Primitive(Primitive::Bool)),
                    values: vec![Value::Bool(true)],
                }
            } else {
                Value::List {
                    ty: ty.clone(),
                    values: vec![Value::UnsignedInteger(17), Value::UnsignedInteger(23)],
                }
            },
        );
        let std::ops::ControlFlow::Continue(staged) =
            vm.stage_normal_argument(&input, &mut env, 0).unwrap()
        else {
            panic!("transfer")
        };
        assert_eq!(vm.bytecode_staged_inputs(), 1);
        assert_eq!(env.get(&touched), Some(&Value::Bool(true)));
        let mut prepared = BTreeMap::new();
        vm.prepare_normal_argument(&input, &ty, &mut prepared, 0)
            .unwrap();
        let result = vm.finish_staged_normal_argument(staged, &mut prepared, SOURCE, 0);
        if invalid {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
        } else {
            assert_eq!(
                result.unwrap(),
                Value::List {
                    ty: ty.clone(),
                    values: vec![
                        Value::UnsignedInteger(17),
                        Value::UnsignedInteger(23),
                        Value::UnsignedInteger(29)
                    ]
                }
            );
        }
    }
}

#[test]
fn known_contextual_domains_precheck_all_branches_before_effects() {
    let package = container_package();
    let trace = LocalId {
        owner: 95,
        index: 0,
    };
    for invalid in [true, false] {
        let input = A::Block {
            source: SOURCE,
            statements: vec![expr(
                Primitive::Void,
                K::Assign {
                    place: Place {
                        root: trace,
                        fields: vec![],
                    },
                    value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(true)))),
                },
            )],
            result: Some(Box::new(A::If {
                source: SOURCE,
                condition: expr(Primitive::Bool, K::Literal(Literal::Bool(true))),
                then_input: Box::new(A::Integer {
                    source: SOURCE,
                    spelling: "17".into(),
                    negative: false,
                }),
                else_input: Box::new(A::Integer {
                    source: SOURCE,
                    spelling: if invalid { "256" } else { "23" }.into(),
                    negative: false,
                }),
            })),
        };
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env = Environment::new();
            env.insert(trace, Value::Bool(false));
            let expected = Term::Primitive(Primitive::U8);
            let result = if compiled {
                vm.evaluate_contextual_argument(&input, &expected, &mut env)
            } else {
                let mut prepared = BTreeMap::new();
                vm.prepare_normal_argument(&input, &expected, &mut prepared, 0)
                    .and_then(|()| vm.eval_normal_argument(&input, &mut prepared, &mut env))
            };
            assert_eq!(vm.bytecode_contextual_inputs(), usize::from(compiled));
            if invalid {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                assert_eq!(env.get(&trace), Some(&Value::Bool(false)));
            } else {
                assert!(matches!(
                    result.unwrap(),
                    Flow::Value(Value::UnsignedInteger(17))
                ));
                assert_eq!(env.get(&trace), Some(&Value::Bool(true)));
            }
            assert_eq!(env.len(), 1);
        }
    }
}

#[test]
fn contextual_fragment_does_not_replace_nested_normal_form_identity() {
    let package = container_package();
    let input = A::Value(container_body(
        Term::Primitive(Primitive::Bool),
        A::Value(expr(Primitive::Bool, K::Literal(Literal::Bool(true)))),
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm
        .evaluate_contextual_argument(
            &input,
            &Term::Primitive(Primitive::Type),
            &mut Environment::new(),
        )
        .unwrap();
    let Flow::Value(value) = result else {
        panic!("transfer")
    };
    assert_eq!(
        value,
        Value::Type(Term::NormalForm {
            head: package.normal_forms[0].declaration,
            sort: nir::Sort::Type,
            arguments: vec![Term::Primitive(Primitive::Bool), Term::Bool(true)],
        })
    );
    assert_eq!(vm.bytecode_contextual_inputs(), 1);
}

#[test]
fn dynamic_normal_calls_compile_and_preserve_order_plan_checks_and_identity() {
    use nir::program::{ArgumentPassing, CallArgument};
    let head = DeclarationId {
        package: 71,
        local: 1,
    };
    let trace = LocalId {
        owner: 71,
        index: 0,
    };
    let factory = LocalId {
        owner: 71,
        index: 1,
    };
    let metadata = Head {
        declaration: head,
        collector: None,
        role: Role::Opaque,
        requirements: vec![],
        parameters: (0..2)
            .map(|index| Parameter {
                declaration: DeclarationId {
                    package: 71,
                    local: 2 + index,
                },
                binding: LocalId { owner: 71, index },
                kind: Kind::Value,
                ty: Term::Primitive(Primitive::Bool),
            })
            .collect(),
    };
    let callable = metadata.callable_type().unwrap();
    let package = Package {
        normal_forms: vec![metadata],
        ..Default::default()
    };
    let set = |value| {
        expr(
            Primitive::Void,
            K::Assign {
                place: Place {
                    root: trace,
                    fields: vec![],
                },
                value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(value)))),
            },
        )
    };
    let first = expr(
        Primitive::Bool,
        K::Block {
            statements: vec![set(true)],
            result: Some(Box::new(expr(
                Primitive::Bool,
                K::Literal(Literal::Bool(false)),
            ))),
        },
    );
    let callee = Expr {
        source: SOURCE,
        ty: callable.clone(),
        kind: K::Block {
            statements: vec![set(false)],
            result: Some(Box::new(Expr {
                source: SOURCE,
                ty: callable,
                kind: K::Local(factory),
            })),
        },
    };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for case in 0..7 {
            let mut arguments = vec![
                CallArgument {
                    parameter: 0,
                    passing: ArgumentPassing::Value,
                    is_default: false,
                    value: first.clone(),
                },
                CallArgument {
                    parameter: if case == 1 { 0 } else { 1 },
                    passing: ArgumentPassing::Value,
                    is_default: case == 3,
                    value: expr(Primitive::Bool, K::Local(trace)),
                },
            ];
            if case == 2 {
                arguments.pop();
            }
            if case == 5 {
                arguments[1].value = expr(Primitive::Char, K::Literal(Literal::Char('x')));
            }
            let body = expr(
                Primitive::Type,
                K::Call {
                    callee: Box::new(callee.clone()),
                    arguments,
                    parameter_bindings: vec![],
                },
            );
            let mut env =
                Environment::from([(trace, Value::Bool(true)), (factory, Value::Function(head))]);
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                assert!(code.to_string().contains("(normal-form (type "));
                vm.execute_code(&code, &mut env)
            } else {
                vm.eval(&body, &mut env)
            };
            if matches!(case, 0 | 4 | 6) {
                let Flow::Value(value) = result.unwrap() else {
                    panic!("expected a Type value")
                };
                assert_eq!(
                    value,
                    Value::Type(Term::NormalForm {
                        head,
                        sort: nir::Sort::Type,
                        arguments: vec![Term::Bool(false), Term::Bool(true)]
                    })
                );
            } else {
                assert_eq!(
                    result.unwrap_err().kind,
                    if case == 5 {
                        ErrorKind::Type
                    } else {
                        ErrorKind::InvalidCallPlan
                    }
                );
            }
            assert_eq!(
                vm.local_value(&env, trace, Some(SOURCE)).unwrap(),
                Value::Bool(case != 2)
            );
            assert_eq!(vm.depth, 0);
        }
    }
}

fn frozen_types(fields: Vec<Term>) -> Term {
    Term::NominalValue {
        ty: Box::new(container_list(Term::Primitive(Primitive::Type))),
        variant: None,
        fields,
    }
}
