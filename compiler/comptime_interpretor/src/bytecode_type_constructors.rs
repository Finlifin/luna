//! Invalid Type producers must not run subsequent operands.
use super::*;
use nir::program::{Literal, Place};
const SOURCE: SourceRange = SourceRange { start: 1, end: 8 };
fn expr(ty: Primitive, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty: Term::Primitive(ty),
        kind,
    }
}

#[test]
fn type_constructor_validation_orders_effects_and_recovers() {
    let package = Package::default();
    let trace = LocalId { owner: 1, index: 0 };
    let input = LocalId { owner: 1, index: 1 };
    let second = expr(
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
    );
    let first = expr(Primitive::Type, K::Local(input));
    for compiled in [false, true] {
        for result_replacement in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            for case in 0..4 {
                let body = expr(
                    if case == 0 {
                        Primitive::Bool
                    } else {
                        Primitive::Type
                    },
                    if result_replacement {
                        K::FunctionResultType {
                            signature: Box::new(first.clone()),
                            result: Box::new(second.clone()),
                        }
                    } else {
                        K::FunctionType {
                            comptime: true,
                            comptime_parameters: if case == 1 { vec![] } else { vec![false, true] },
                            parameters: vec![first.clone(), second.clone()],
                        }
                    },
                );
                let mut env = Environment::new();
                env.insert(trace, Value::Bool(false));
                env.insert(
                    input,
                    if case < 3 {
                        if result_replacement {
                            Value::Type(Term::Primitive(Primitive::Bool))
                        } else {
                            Value::Bool(true)
                        }
                    } else {
                        Value::Type(Term::Function {
                            parameters: vec![],
                            result: Box::new(Term::Primitive(Primitive::Void)),
                        })
                    },
                );
                let result = if compiled {
                    let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                    assert!(code.to_string().contains("(construct-type "));
                    vm.execute_code(&code, &mut env)
                } else {
                    vm.eval(&body, &mut env)
                };
                if case < 3 {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                } else {
                    assert!(matches!(result.unwrap(), Flow::Value(Value::Type(_))));
                }
                assert_eq!(env.get(&trace), Some(&Value::Bool(case == 3)));
            }
        }
    }
}
