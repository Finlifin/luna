use super::*;
use nir::program::{Literal, Place};
const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
fn expr(ty: Primitive, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty: Term::Primitive(ty),
        kind,
    }
}
fn boolean(value: bool) -> Expr {
    expr(Primitive::Bool, K::Literal(Literal::Bool(value)))
}

#[test]
fn nested_inputs_preserve_depth_limits_and_recover() {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let package = Package::default();
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            for nesting in [0, 1, 32, 126, 127, 128, 160] {
                let mut input = A::Value(boolean(true));
                for _ in 0..nesting {
                    input = A::Tuple {
                        source: SOURCE,
                        type_head: None,
                        items: vec![input],
                    };
                }
                vm.budget = Budget {
                    steps: 100000,
                    depth: usize::MAX,
                };
                let result = vm.stage_normal_argument(&input, &mut Environment::new(), 0);
                assert_eq!(vm.depth, 0);
                if nesting >= 128 {
                    assert!(matches!(
                        result,
                        Err(Error {
                            kind: ErrorKind::DepthExhausted,
                            ..
                        })
                    ));
                } else {
                    let ControlFlow::Continue(staged) = result.unwrap() else {
                        panic!("transfer")
                    };
                    let mut value = vm
                        .finish_staged_normal_argument(staged, &mut BTreeMap::new(), SOURCE, 0)
                        .unwrap();
                    for _ in 0..nesting {
                        let Value::Tuple(mut items) = value else {
                            panic!("tuple")
                        };
                        assert_eq!(items.len(), 1);
                        value = items.pop().unwrap();
                    }
                    assert_eq!(value, Value::Bool(true));
                }
                let result = vm
                    .stage_normal_argument(&A::Value(boolean(false)), &mut Environment::new(), 0)
                    .unwrap();
                assert!(matches!(
                    result,
                    ControlFlow::Continue(StagedNormalInput::Value(Value::Bool(false)))
                ));
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn staged_scopes_cleanup_on_errors_transfers_and_budget_exhaustion() {
    let package = Package::default();
    let outer = LocalId { owner: 1, index: 0 };
    let inner = LocalId { owner: 1, index: 1 };
    for ending in [
        boolean(false),
        expr(Primitive::Never, K::Return(Some(Box::new(boolean(true))))),
        expr(
            Primitive::Bool,
            K::Local(LocalId {
                owner: 1,
                index: 99,
            }),
        ),
    ] {
        let input = A::Block {
            source: SOURCE,
            statements: vec![
                expr(
                    Primitive::Void,
                    K::Let {
                        binding: inner,
                        value: Box::new(boolean(true)),
                    },
                ),
                expr(
                    Primitive::Void,
                    K::Assign {
                        place: Place {
                            root: outer,
                            fields: vec![],
                        },
                        value: Box::new(boolean(true)),
                    },
                ),
            ],
            result: Some(Box::new(A::Block {
                source: SOURCE,
                statements: vec![],
                result: Some(Box::new(A::Value(ending.clone()))),
            })),
        };
        for steps in 0..40 {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            vm.budget.steps = steps;
            let mut env = Environment::new();
            env.insert(outer, Value::Bool(false));
            let result = vm.stage_normal_argument(&input, &mut env, 0);
            assert_eq!(vm.depth, 0);
            assert!(!env.contains_key(&inner));
            assert_eq!(env.len(), 1);
            if steps == 39 {
                assert_eq!(env.get(&outer), Some(&Value::Bool(true)));
                match ending.kind {
                    K::Return(_) => assert!(matches!(
                        result,
                        Ok(ControlFlow::Break(Flow::Return(Value::Bool(true))))
                    )),
                    K::Local(_) => assert!(matches!(
                        result,
                        Err(Error {
                            kind: ErrorKind::MissingBinding,
                            ..
                        })
                    )),
                    _ => assert!(matches!(
                        result,
                        Ok(ControlFlow::Continue(StagedNormalInput::Value(
                            Value::Bool(false)
                        )))
                    )),
                }
            }
        }
    }
}
