use super::*;
use nir::program::{Place, TextQueryKind};

#[test]
fn observation_admission_precedes_operand_effects() {
    let package = Package::default();
    let source = SourceRange { start: 60, end: 65 };
    let trace = LocalId { owner: 1, index: 0 };
    let expr = |ty, kind| Expr { source, ty, kind };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for kind in 0..3 {
            for valid in [false, true] {
                if kind == 2 && valid {
                    continue;
                }
                let ty = if valid {
                    Term::Primitive(Primitive::Symbol)
                } else {
                    Term::Primitive(Primitive::Bool)
                };
                let operand = expr(
                    ty,
                    K::Block {
                        statements: vec![expr(
                            Term::Primitive(Primitive::Void),
                            K::Assign {
                                place: Place {
                                    root: trace,
                                    fields: vec![],
                                },
                                value: Box::new(expr(
                                    Term::Primitive(Primitive::Bool),
                                    K::Literal(Literal::Bool(true)),
                                )),
                            },
                        )],
                        result: Some(Box::new(expr(
                            Term::Primitive(Primitive::Symbol),
                            K::Literal(Literal::Symbol("ready".into())),
                        ))),
                    },
                );
                let body = match kind {
                    0 => expr(
                        Term::Primitive(Primitive::Str),
                        K::SymbolName(Box::new(operand)),
                    ),
                    1 => expr(
                        Term::Primitive(Primitive::U64),
                        K::TextQuery {
                            subject: Box::new(expr(
                                Term::Tuple(vec![operand.ty.clone()]),
                                K::Tuple(vec![operand]),
                            )),
                            query: TextQueryKind::SymbolByteLength,
                        },
                    ),
                    _ => expr(
                        Term::Primitive(Primitive::Str),
                        K::EnumName(Box::new(operand)),
                    ),
                };
                let mut env = Environment::from([(trace, Value::Bool(false))]);
                let result = if compiled {
                    let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                    assert!(code.to_string().contains("(check-observation "));
                    vm.execute_code(&code, &mut env)
                } else {
                    vm.eval(&body, &mut env)
                };
                assert_eq!(result.is_ok(), valid);
                assert_eq!(
                    vm.local_value(&env, trace, Some(source)).unwrap(),
                    Value::Bool(valid)
                );
            }
        }
    }
}
