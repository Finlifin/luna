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
