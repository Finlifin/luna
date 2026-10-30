use super::*;
use nir::program::Place;

#[test]
fn bit_operand_failures_stop_before_later_effects_and_bounds_follow_evaluation() {
    let source = SourceRange { start: 1, end: 8 };
    let trace = LocalId { owner: 1, index: 0 };
    let expr = |ty, kind| Expr {
        source,
        ty: Term::Primitive(ty),
        kind,
    };
    let bits = || {
        expr(
            Primitive::BitVec(4),
            K::Literal(Literal::BitVec {
                width: 4,
                words: vec![13],
            }),
        )
    };
    let effect = |value: Expr| Expr {
        source,
        ty: value.ty.clone(),
        kind: K::Block {
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
            result: Some(Box::new(value)),
        },
    };
    let package = Package::default();
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for bad in [true, false] {
            for kind in 0..4 {
                let value = Box::new(if bad {
                    expr(Primitive::Bool, K::Literal(Literal::Bool(false)))
                } else {
                    bits()
                });
                let index = Box::new(effect(expr(
                    Primitive::U8,
                    K::Literal(Literal::UnsignedInteger(4)),
                )));
                let body = match kind {
                    0 => expr(Primitive::BitVec(1), K::BitIndex(value, index)),
                    1 => expr(
                        Primitive::BitVec(0),
                        K::BitWindow {
                            value,
                            start: Box::new(expr(
                                Primitive::U8,
                                K::Literal(Literal::UnsignedInteger(4)),
                            )),
                            end: index,
                        },
                    ),
                    2 => expr(
                        Primitive::BitVec(8),
                        K::BitConcat(value, Box::new(effect(bits()))),
                    ),
                    _ => expr(
                        Primitive::Bool,
                        K::BitEqual(value, Box::new(effect(bits()))),
                    ),
                };
                let mut environment = Environment::from([(trace, Value::Bool(false))]);
                let result = if compiled {
                    let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                    assert!(code.to_string().contains("(bits "));
                    vm.execute_code(&code, &mut environment)
                } else {
                    vm.eval(&body, &mut environment)
                };
                if bad {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                } else if kind == 0 {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::IndexOutOfBounds);
                } else {
                    assert!(matches!(
                        result.unwrap(),
                        Flow::Value(Value::BitVec(_) | Value::Bool(true))
                    ));
                }
                assert_eq!(
                    vm.local_value(&environment, trace, Some(source)).unwrap(),
                    Value::Bool(!bad)
                );
            }
        }
    }
}

#[test]
fn bit_conversion_rejects_narrowing_and_restricted_representations() {
    let package = Package::default();
    let source = SourceRange { start: 20, end: 25 };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for output in [
            Primitive::U8,
            Primitive::I8,
            Primitive::F32,
            Primitive::Bool,
            Primitive::BitVec(8),
            Primitive::U32,
        ] {
            let body = Expr {
                source,
                ty: Term::Primitive(output),
                kind: K::BitConvert(Box::new(Expr {
                    source,
                    ty: Term::Primitive(Primitive::BitVec(16)),
                    kind: K::Literal(Literal::BitVec {
                        width: 16,
                        words: vec![1],
                    }),
                })),
            };
            let mut environment = Environment::default();
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                vm.execute_code(&code, &mut environment)
            } else {
                vm.eval(&body, &mut environment)
            };
            if output == Primitive::U32 {
                assert!(matches!(
                    result.unwrap(),
                    Flow::Value(Value::UnsignedInteger(1))
                ));
            } else {
                assert!(result.is_err(), "{output:?}");
            }
        }
    }
}

#[test]
fn taking_bits_checks_source_domain_and_output_width_before_delivery() {
    let package = Package::default();
    let source = SourceRange { start: 30, end: 35 };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (input, output, literal, valid) in [
            (
                Primitive::U8,
                Primitive::BitVec(16),
                Literal::UnsignedInteger(1),
                false,
            ),
            (
                Primitive::U8,
                Primitive::BitVec(8),
                Literal::UnsignedInteger(256),
                false,
            ),
            (
                Primitive::Bool,
                Primitive::BitVec(1),
                Literal::Bool(true),
                false,
            ),
            (
                Primitive::U8,
                Primitive::U8,
                Literal::UnsignedInteger(1),
                false,
            ),
            (
                Primitive::U8,
                Primitive::BitVec(8),
                Literal::UnsignedInteger(255),
                true,
            ),
        ] {
            let body = Expr {
                source,
                ty: Term::Primitive(output),
                kind: K::TakeBits(Box::new(Expr {
                    source,
                    ty: Term::Primitive(input),
                    kind: K::Literal(literal),
                })),
            };
            let mut environment = Environment::default();
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                vm.execute_code(&code, &mut environment)
            } else {
                vm.eval(&body, &mut environment)
            };
            if valid {
                let Flow::Value(Value::BitVec(bits)) = result.unwrap() else {
                    panic!("expected bits")
                };
                assert_eq!(bits.width(), 8);
                assert_eq!(bits.words(), &[255]);
            } else {
                assert!(result.is_err(), "{input:?} -> {output:?}");
            }
        }
    }
}

#[test]
fn failed_bit_writeback_preserves_completed_rhs_writes() {
    let package = Package::default();
    let source = SourceRange { start: 40, end: 45 };
    let root = LocalId { owner: 1, index: 0 };
    let expr = |ty, kind| Expr {
        source,
        ty: Term::Primitive(ty),
        kind,
    };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (start, end, valid) in [(7, 11, false), (0, 3, false), (0, 4, true)] {
            let body = expr(
                Primitive::Void,
                K::BitAssign {
                    place: Place {
                        root,
                        fields: vec![],
                    },
                    storage_type: Term::Primitive(Primitive::U8),
                    start,
                    end,
                    value: Box::new(expr(
                        Primitive::BitVec(4),
                        K::Block {
                            statements: vec![expr(
                                Primitive::Void,
                                K::Assign {
                                    place: Place {
                                        root,
                                        fields: vec![],
                                    },
                                    value: Box::new(expr(
                                        Primitive::U8,
                                        K::Literal(Literal::UnsignedInteger(0xa0)),
                                    )),
                                },
                            )],
                            result: Some(Box::new(expr(
                                Primitive::BitVec(4),
                                K::Literal(Literal::BitVec {
                                    width: 4,
                                    words: vec![5],
                                }),
                            ))),
                        },
                    )),
                },
            );
            let mut environment = Environment::from([(root, Value::UnsignedInteger(0))]);
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                assert!(code.to_string().contains("(write-bits "));
                vm.execute_code(&code, &mut environment)
            } else {
                vm.eval(&body, &mut environment)
            };
            assert_eq!(result.is_ok(), valid);
            assert_eq!(
                vm.local_value(&environment, root, Some(source)).unwrap(),
                Value::UnsignedInteger(if valid { 0xa5 } else { 0xa0 })
            );
        }
    }
}

#[test]
fn dynamic_bit_ranges_check_in_order_before_rhs_effects() {
    use nir::program::BitSelection;
    let package = Package::default();
    let source = SourceRange { start: 50, end: 55 };
    let root = LocalId { owner: 1, index: 0 };
    let trace = LocalId { owner: 1, index: 1 };
    let expr = |ty, kind| Expr {
        source,
        ty: Term::Primitive(ty),
        kind,
    };
    let effect = |value: Expr, stamp| Expr {
        source,
        ty: value.ty.clone(),
        kind: K::Block {
            statements: vec![expr(
                Primitive::Void,
                K::Assign {
                    place: Place {
                        root: trace,
                        fields: vec![],
                    },
                    value: Box::new(expr(
                        Primitive::U8,
                        K::Literal(Literal::UnsignedInteger(stamp)),
                    )),
                },
            )],
            result: Some(Box::new(value)),
        },
    };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        // Shape failure runs no endpoints; range failure runs both endpoints
        // but not the next selection or RHS; successful selection runs all.
        for (lower, upper, width, valid, stamp) in [
            (0, 9, 9, false, 0),
            (7, 11, 4, false, 2),
            (2, 6, 4, true, 4),
        ] {
            let body = expr(
                Primitive::Void,
                K::BitUpdate {
                    place: Place {
                        root,
                        fields: vec![],
                    },
                    storage_type: Term::Primitive(Primitive::U8),
                    selections: vec![
                        BitSelection::Window {
                            start: Box::new(effect(
                                expr(Primitive::U8, K::Literal(Literal::UnsignedInteger(lower))),
                                1,
                            )),
                            end: Box::new(effect(
                                expr(Primitive::U8, K::Literal(Literal::UnsignedInteger(upper))),
                                2,
                            )),
                            width,
                        },
                        BitSelection::Index(Box::new(effect(
                            expr(Primitive::U8, K::Literal(Literal::UnsignedInteger(1))),
                            3,
                        ))),
                    ],
                    value: Box::new(effect(
                        expr(
                            Primitive::BitVec(1),
                            K::Literal(Literal::BitVec {
                                width: 1,
                                words: vec![1],
                            }),
                        ),
                        4,
                    )),
                },
            );
            let mut env = Environment::from([
                (root, Value::UnsignedInteger(0)),
                (trace, Value::UnsignedInteger(0)),
            ]);
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                assert!(code.to_string().contains("(select-bit-range "));
                vm.execute_code(&code, &mut env)
            } else {
                vm.eval(&body, &mut env)
            };
            assert_eq!(result.is_ok(), valid);
            assert_eq!(
                vm.local_value(&env, trace, Some(source)).unwrap(),
                Value::UnsignedInteger(stamp)
            );
            assert_eq!(
                vm.local_value(&env, root, Some(source)).unwrap(),
                Value::UnsignedInteger(if valid { 8 } else { 0 })
            );
        }
    }
}
