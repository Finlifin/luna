use super::*;
use nir::program::Place;
const SOURCE: SourceRange = SourceRange { start: 5, end: 17 };
const WIDTH: LocalId = LocalId { owner: 1, index: 0 };
const TRACE: LocalId = LocalId { owner: 1, index: 1 };
fn expr(ty: Primitive, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty: Term::Primitive(ty),
        kind,
    }
}
fn query() -> Expr {
    expr(
        Primitive::Type,
        K::BitVecType(Box::new(expr(
            Primitive::Integer,
            K::Block {
                statements: vec![expr(
                    Primitive::Void,
                    K::Assign {
                        place: Place {
                            root: TRACE,
                            fields: vec![],
                        },
                        value: Box::new(expr(
                            Primitive::Bool,
                            K::Not(Box::new(expr(Primitive::Bool, K::Local(TRACE)))),
                        )),
                    },
                )],
                result: Some(Box::new(expr(Primitive::Integer, K::Local(WIDTH)))),
            },
        ))),
    )
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
fn bitvector_width_validation_preserves_effect_order_and_recovers() {
    let package = Package::default();
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for width in [-1, 0, 1, usize::MAX as i128 + 1, 8] {
            let mut env = Environment::from([
                (
                    WIDTH,
                    Value::Integer(comptime_runtime::Integer::from_i128(width)),
                ),
                (TRACE, Value::Bool(false)),
            ]);
            let result = execute(&mut vm, &query(), &mut env, compiled);
            if width < 0 || width > usize::MAX as i128 {
                assert_eq!(
                    result.unwrap_err(),
                    Error {
                        kind: ErrorKind::InvalidBitWidth,
                        source: Some(SOURCE)
                    }
                );
            } else {
                assert!(
                    matches!(result.unwrap(),Flow::Value(Value::Type(Term::Primitive(Primitive::BitVec(actual)))) if actual==width as usize)
                );
            }
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(true)
            );
        }
        for wrong_input in [false, true] {
            let mut body = query();
            if wrong_input {
                let K::BitVecType(width) = &mut body.kind else {
                    unreachable!()
                };
                width.ty = Term::Primitive(Primitive::Bool);
            } else {
                body.ty = Term::Primitive(Primitive::Bool);
            }
            let mut env = Environment::from([
                (
                    WIDTH,
                    Value::Integer(comptime_runtime::Integer::from_i128(8)),
                ),
                (TRACE, Value::Bool(false)),
            ]);
            assert_eq!(
                execute(&mut vm, &body, &mut env, compiled)
                    .unwrap_err()
                    .kind,
                ErrorKind::Type
            );
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(false)
            );
        }
    }
}
#[test]
fn bitvector_constructor_propagates_return_and_budget_exhaustion() {
    let package = Package::default();
    let early = expr(
        Primitive::Never,
        K::BitVecType(Box::new(expr(
            Primitive::Never,
            K::Return(Some(Box::new(expr(
                Primitive::Bool,
                K::Literal(Literal::Bool(true)),
            )))),
        ))),
    );
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert!(matches!(
            execute(&mut vm, &early, &mut Environment::new(), compiled).unwrap(),
            Flow::Return(Value::Bool(true))
        ));
        let mut env = Environment::from([
            (
                WIDTH,
                Value::Integer(comptime_runtime::Integer::from_i128(8)),
            ),
            (TRACE, Value::Bool(false)),
        ]);
        vm.budget.steps = 0;
        assert_eq!(
            execute(&mut vm, &query(), &mut env, compiled)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        vm.budget.steps = 10000;
        assert!(matches!(
            execute(&mut vm, &query(), &mut env, compiled).unwrap(),
            Flow::Value(Value::Type(Term::Primitive(Primitive::BitVec(8))))
        ));
    }
}
