//! Type evidence and operand effects must agree across execution carriers.
use super::*;
use nir::program::{Literal, TypePredicate};
use std::cell::RefCell;

const SOURCE: SourceRange = SourceRange { start: 10, end: 20 };
fn expr(ty: Primitive, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty: Term::Primitive(ty),
        kind,
    }
}
fn comparison() -> Expr {
    expr(
        Primitive::Bool,
        K::TypeEqual(
            Box::new(expr(
                Primitive::Type,
                K::Local(LocalId { owner: 1, index: 0 }),
            )),
            Box::new(expr(
                Primitive::Type,
                K::Local(LocalId { owner: 1, index: 1 }),
            )),
        ),
    )
}
fn run(
    vm: &mut Interpreter<'_>,
    body: &Expr,
    env: &mut Environment,
    compiled: bool,
) -> Result<Value, Error> {
    let flow = if compiled {
        let code = Arc::new(comptime_compiler::compile(body, &mut 10000).unwrap());
        assert!(code.to_string().contains("(type-equal %"));
        vm.execute_code(&code, env)
    } else {
        vm.eval(body, env)
    }?;
    match flow {
        Flow::Value(value) => Ok(value),
        _ => panic!("unexpected control flow"),
    }
}

#[test]
fn proofs_unknown_and_recovery_preserve_the_query_protocol() {
    let package = Package::default();
    for compiled in [false, true] {
        for outcome in [
            ContractOutcome::Satisfied,
            ContractOutcome::Rejected,
            ContractOutcome::Unknown,
        ] {
            for prove_negation in [false, true] {
                let queries = RefCell::new(Vec::new());
                let solver = |predicate: &TypePredicate, budget: &mut u64| {
                    *budget = budget.saturating_sub(3);
                    queries.borrow_mut().push(predicate.clone());
                    match predicate {
                        TypePredicate::Equal(..) => outcome,
                        TypePredicate::Not(_) if prove_negation => ContractOutcome::Satisfied,
                        _ => ContractOutcome::Unknown,
                    }
                };
                let mut vm = Interpreter::for_comptime(&package, Budget::default())
                    .unwrap()
                    .with_budgeted_type_solver(&solver);
                let mut env = Environment::new();
                let boolean = Term::Primitive(Primitive::Bool);
                let byte = Term::Primitive(Primitive::U8);
                env.insert(LocalId { owner: 1, index: 0 }, Value::Type(boolean.clone()));
                env.insert(LocalId { owner: 1, index: 1 }, Value::Type(byte.clone()));
                let predicate = TypePredicate::Equal(boolean.clone(), byte);
                let result = run(&mut vm, &comparison(), &mut env, compiled);
                if outcome == ContractOutcome::Satisfied {
                    assert_eq!(result.unwrap(), Value::Bool(true));
                    assert_eq!(*queries.borrow(), vec![predicate]);
                } else {
                    if prove_negation {
                        assert_eq!(result.unwrap(), Value::Bool(false));
                    } else {
                        assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractUnresolved);
                    }
                    assert_eq!(
                        *queries.borrow(),
                        vec![predicate.clone(), TypePredicate::Not(Box::new(predicate))]
                    );
                }
                queries.borrow_mut().clear();
                env.insert(LocalId { owner: 1, index: 1 }, Value::Type(boolean));
                assert_eq!(
                    run(&mut vm, &comparison(), &mut env, compiled).unwrap(),
                    Value::Bool(true)
                );
                assert!(queries.borrow().is_empty());
            }
        }
    }
}

#[test]
fn operand_validation_stops_before_later_effects() {
    let package = Package::default();
    let trace = LocalId { owner: 1, index: 2 };
    let effect = |index, flag| {
        expr(
            Primitive::Type,
            K::Block {
                statements: vec![expr(
                    Primitive::Void,
                    K::Assign {
                        place: nir::program::Place {
                            root: trace,
                            fields: vec![],
                        },
                        value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(flag)))),
                    },
                )],
                result: Some(Box::new(expr(
                    Primitive::Type,
                    K::Local(LocalId { owner: 1, index }),
                ))),
            },
        )
    };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for case in 0..4 {
            let mut body = expr(
                Primitive::Bool,
                K::TypeEqual(Box::new(effect(0, true)), Box::new(effect(1, false))),
            );
            if case == 0 {
                body.ty = Term::Primitive(Primitive::Type);
            }
            let mut env = Environment::new();
            env.insert(trace, Value::Bool(false));
            env.insert(
                LocalId { owner: 1, index: 0 },
                if case == 1 {
                    Value::Bool(true)
                } else {
                    Value::Type(Term::Primitive(Primitive::Bool))
                },
            );
            env.insert(
                LocalId { owner: 1, index: 1 },
                if case == 2 {
                    Value::Bool(true)
                } else {
                    Value::Type(Term::Primitive(Primitive::Bool))
                },
            );
            let result = run(&mut vm, &body, &mut env, compiled);
            if case < 3 {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            } else {
                assert_eq!(result.unwrap(), Value::Bool(true));
            }
            assert_eq!(env.get(&trace), Some(&Value::Bool(case == 1)));
        }
    }
}

#[test]
fn exhausted_proof_budget_stops_before_the_negation_query() {
    let package = Package::default();
    for compiled in [false, true] {
        let calls = std::cell::Cell::new(0);
        let solver = |_: &TypePredicate, remaining: &mut u64| {
            calls.set(calls.get() + 1);
            *remaining = 0;
            ContractOutcome::Unknown
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_budgeted_type_solver(&solver);
        let mut env = Environment::new();
        env.insert(
            LocalId { owner: 1, index: 0 },
            Value::Type(Term::Primitive(Primitive::Bool)),
        );
        env.insert(
            LocalId { owner: 1, index: 1 },
            Value::Type(Term::Primitive(Primitive::Char)),
        );
        assert_eq!(
            run(&mut vm, &comparison(), &mut env, compiled)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(vm.remaining_steps(), 0);
        vm.budget = Budget::default();
        env.insert(
            LocalId { owner: 1, index: 1 },
            Value::Type(Term::Primitive(Primitive::Bool)),
        );
        assert_eq!(
            run(&mut vm, &comparison(), &mut env, compiled).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(calls.get(), 1);
    }
}
