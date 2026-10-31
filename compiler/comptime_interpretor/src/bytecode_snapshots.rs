//! Snapshot wrappers retain operand ordering and value-copy semantics in code.
use super::*;
use nir::program::{Literal, Place};
const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty,
        kind,
    }
}
fn run(
    vm: &mut Interpreter<'_>,
    body: &Expr,
    env: &mut Environment,
    compiled: bool,
) -> Result<Flow, Error> {
    if compiled {
        let code = Arc::new(comptime_compiler::compile(body, &mut 10000).unwrap());
        assert!(code.to_string().contains("(prepare-snapshot "));
        vm.execute_code(&code, env)
    } else {
        vm.eval(body, env)
    }
}
#[test]
fn snapshot_admission_precedes_operand_effects_and_propagates_return() {
    let package = Package::default();
    let trace = LocalId { owner: 1, index: 0 };
    let boolean = Term::Primitive(Primitive::Bool);
    for compiled in [false, true] {
        for valid in [false, true] {
            let operand = expr(
                boolean.clone(),
                K::Block {
                    statements: vec![expr(
                        Term::Primitive(Primitive::Void),
                        K::Assign {
                            place: Place {
                                root: trace,
                                fields: vec![],
                            },
                            value: Box::new(expr(boolean.clone(), K::Literal(Literal::Bool(true)))),
                        },
                    )],
                    result: Some(Box::new(expr(
                        Term::Primitive(Primitive::Never),
                        K::Return(Some(Box::new(expr(
                            boolean.clone(),
                            K::Literal(Literal::Bool(false)),
                        )))),
                    ))),
                },
            );
            let body = expr(
                if valid {
                    boolean.clone()
                } else {
                    Term::Primitive(Primitive::Char)
                },
                K::Snapshot(Box::new(operand)),
            );
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env = Environment::from([(trace, Value::Bool(false))]);
            let result = run(&mut vm, &body, &mut env, compiled);
            if valid {
                assert!(matches!(result.unwrap(), Flow::Return(Value::Bool(false))));
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            }
            assert_eq!(env[&trace], Value::Bool(valid));
        }
    }
}
#[test]
fn snapshot_reads_storage_without_retaining_a_live_alias() {
    let package = Package::default();
    let input = LocalId { owner: 1, index: 0 };
    let ty = Term::Primitive(Primitive::Bool);
    let body = expr(ty.clone(), K::Snapshot(Box::new(expr(ty, K::Local(input)))));
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let storage = Storage::new(Value::Bool(false));
        let mut env = Environment::from([(input, Value::Storage(storage.clone()))]);
        let Flow::Value(before) = run(&mut vm, &body, &mut env, compiled).unwrap() else {
            panic!("value")
        };
        *storage.0.lock().unwrap() = Value::Bool(true);
        let Flow::Value(after) = run(&mut vm, &body, &mut env, compiled).unwrap() else {
            panic!("value")
        };
        assert_eq!(before, Value::Bool(false));
        assert_eq!(after, Value::Bool(true));
    }
}
