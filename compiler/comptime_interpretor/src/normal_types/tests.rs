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
