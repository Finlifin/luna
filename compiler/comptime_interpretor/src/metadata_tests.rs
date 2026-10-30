use super::*;
use nir::program::Place;
const SOURCE: SourceRange = SourceRange { start: 13, end: 21 };
const ALIAS: LocalId = LocalId { owner: 1, index: 0 };
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
        Primitive::Bool,
        K::BuildMetadata {
            owner: 17,
            entries: vec![
                ("yes".into(), 21, Some(Term::Bool(true))),
                ("no".into(), 22, Some(Term::Bool(false))),
                ("missing".into(), 23, None),
                ("invalid".into(), 24, Some(Term::Primitive(Primitive::Type))),
            ],
            selector: Box::new(expr(
                Primitive::Str,
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
                    result: Some(Box::new(expr(Primitive::Str, K::Local(ALIAS)))),
                },
            )),
        },
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
fn metadata_reads_only_the_selected_input_and_recovers_without_replaying_effects() {
    let package = Package::default();
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for alias in ["yes", "missing", "unknown", "invalid", "no", "yes"] {
            let mut env = Environment::from([
                (ALIAS, Value::String(alias.into())),
                (TRACE, Value::Bool(false)),
            ]);
            let result = execute(&mut vm, &query(), &mut env, compiled);
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(true)
            );
            match alias {
                "yes" | "no" => assert!(
                    matches!(result.unwrap(),Flow::Value(Value::Bool(value)) if value==(alias=="yes"))
                ),
                "invalid" => assert!(result.is_err()),
                _ => assert_eq!(
                    result.unwrap_err(),
                    Error {
                        source: Some(SOURCE),
                        kind: ErrorKind::BuildMetadataUnavailable {
                            owner: 17,
                            provider: if alias == "missing" { Some(23) } else { None },
                            alias: alias.into()
                        }
                    }
                ),
            }
        }
        vm.comptime_only = false;
        let mut env = Environment::from([
            (ALIAS, Value::String("yes".into())),
            (TRACE, Value::Bool(false)),
        ]);
        assert_eq!(
            execute(&mut vm, &query(), &mut env, compiled)
                .unwrap_err()
                .kind,
            ErrorKind::PhaseMismatch
        );
        assert_eq!(
            vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
            Value::Bool(false)
        );
    }
}
#[test]
fn metadata_selector_return_skips_unavailable_input() {
    let package = Package::default();
    let mut body = query();
    let K::BuildMetadata { selector, .. } = &mut body.kind else {
        unreachable!()
    };
    **selector = expr(
        Primitive::Never,
        K::Return(Some(Box::new(expr(
            Primitive::Bool,
            K::Literal(Literal::Bool(true)),
        )))),
    );
    body.ty = Term::Primitive(Primitive::Never);
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert!(matches!(
            execute(&mut vm, &body, &mut Environment::new(), compiled).unwrap(),
            Flow::Return(Value::Bool(true))
        ));
    }
}
