//! Construction validates shape before capture effects and each value in order.
use super::*;
use nir::program::{Capture, Place};

#[test]
fn closure_capture_errors_preserve_order_and_allow_fresh_invocations() {
    let source = SourceRange { start: 1, end: 9 };
    let flag = LocalId { owner: 1, index: 0 };
    let left = LocalId { owner: 2, index: 0 };
    let right = LocalId { owner: 2, index: 1 };
    let boolean = Term::Primitive(Primitive::Bool);
    let expression = |ty, kind| Expr { source, ty, kind };
    let set = |value| {
        expression(
            Term::Primitive(Primitive::Void),
            K::Assign {
                place: Place {
                    root: flag,
                    fields: vec![],
                },
                value: Box::new(expression(
                    boolean.clone(),
                    K::Literal(Literal::Bool(value)),
                )),
            },
        )
    };
    let value = |value| {
        expression(
            boolean.clone(),
            K::Block {
                statements: vec![set(value)],
                result: Some(Box::new(expression(
                    boolean.clone(),
                    K::Literal(Literal::Bool(value)),
                ))),
            },
        )
    };
    let ty = Term::Closure {
        environment: vec![boolean.clone(), boolean.clone()],
        signature: Box::new(Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(boolean.clone()),
        }))),
    };
    let package = Package::default();
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for case in 0..7 {
            let mut body = expression(
                ty.clone(),
                K::Closure {
                    parameters: vec![],
                    captures: vec![
                        Capture {
                            binding: left,
                            ty: boolean.clone(),
                            value: Some(Box::new(value(true))),
                        },
                        Capture {
                            binding: if case == 2 { left } else { right },
                            ty: if case == 1 {
                                Term::Primitive(Primitive::Char)
                            } else {
                                boolean.clone()
                            },
                            value: Some(Box::new(value(false))),
                        },
                    ],
                    body: Arc::new(expression(boolean.clone(), K::Local(left))),
                },
            );
            if case == 3 {
                body.ty = Term::ComptimeFunction(Box::new(Term::Function {
                    parameters: vec![],
                    result: Box::new(boolean.clone()),
                }));
            }
            if case == 5 {
                let K::Closure { captures, .. } = &mut body.kind else {
                    unreachable!()
                };
                captures[0].value = Some(Box::new(expression(
                    Term::Primitive(Primitive::Never),
                    K::Return(Some(Box::new(expression(
                        boolean.clone(),
                        K::Literal(Literal::Bool(true)),
                    )))),
                )));
            }
            let mut environment = Environment::from([(flag, Value::Bool(true))]);
            let result = if compiled {
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                let dump = code.to_string();
                assert!(dump.contains("(begin-closure"));
                assert!(dump.contains("(capture"));
                vm.execute_code(&code, &mut environment)
            } else {
                vm.eval(&body, &mut environment)
            };
            match case {
                1 | 3 => assert_eq!(result.unwrap_err().kind, ErrorKind::Type),
                2 => assert_eq!(result.unwrap_err().kind, ErrorKind::InvalidCallPlan),
                5 => assert!(matches!(result.unwrap(), Flow::Return(Value::Bool(true)))),
                _ => {
                    let Flow::Value(Value::Closure(closure)) = result.unwrap() else {
                        panic!("expected closure")
                    };
                    assert_eq!(
                        vm.local_value(&closure.environment, left, Some(source))
                            .unwrap(),
                        Value::Bool(true)
                    );
                    assert_eq!(
                        vm.local_value(&closure.environment, right, Some(source))
                            .unwrap(),
                        Value::Bool(false)
                    );
                }
            }
            assert_eq!(
                vm.local_value(&environment, flag, Some(source)).unwrap(),
                Value::Bool(matches!(case, 3 | 5))
            );
            assert_eq!(vm.depth, 0);
        }
    }
}
