//! Returned host work is not yet an admitted initializer or a published value.
use super::*;
use std::cell::Cell;

fn key() -> (InterfaceInstance, Term, DeclarationId) {
    (
        InterfaceInstance {
            declaration: id(1),
            arguments: vec![],
        },
        ty(Primitive::Bool),
        id(2),
    )
}

fn fragment() -> Arc<comptime_compiler::Program> {
    let integer = ty(Primitive::Integer);
    let increment = expr(
        ty(Primitive::Void),
        K::Assign {
            place: Place {
                root: local(1),
                fields: vec![],
            },
            value: Box::new(expr(
                integer.clone(),
                K::IntegerBinary {
                    op: nir::program::IntegerOp::Add,
                    left: Box::new(expr(integer.clone(), K::Local(local(1)))),
                    right: Box::new(expr(integer, K::Literal(Literal::Integer("1".into())))),
                },
            )),
        },
    );
    let (interface, subject, member) = key();
    let body = expr(
        ty(Primitive::Bool),
        K::Block {
            statements: vec![increment],
            result: Some(Box::new(expr(
                ty(Primitive::Bool),
                K::AssociatedValue {
                    interface,
                    subject,
                    member,
                },
            ))),
        },
    );
    Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap())
}

#[test]
fn returned_answers_wait_for_admission_and_cancel_without_publication() {
    let package = Package::default();
    let code = fragment();
    for requires_invocation in [false, true] {
        for cancel in [false, true] {
            for fault in ["valid", "wrong-domain", "failure", "refill", "no-budget"] {
                let mode = Cell::new(fault);
                let calls = Cell::new(0);
                let remaining_after_host = Cell::new(0);
                let resolver = |_: &Expr, fuel: &mut u64| {
                    calls.set(calls.get() + 1);
                    *fuel -= 7;
                    let body = if mode.get() == "wrong-domain" {
                        expr(ty(Primitive::Char), K::Literal(Literal::Char('x')))
                    } else {
                        expr(ty(Primitive::Bool), K::Literal(Literal::Bool(true)))
                    };
                    let mut allowance = *fuel as usize;
                    let selection = if requires_invocation {
                        AssociatedSelection::Initializer(
                            nir::associated_initializers::AssociatedInitializer::new(
                                body,
                                &mut allowance,
                            )
                            .unwrap(),
                        )
                    } else {
                        AssociatedSelection::Completed(
                            nir::completed_values::CompletedValue::new(
                                body,
                                &package,
                                &|_| None,
                                &mut allowance,
                            )
                            .unwrap(),
                        )
                    };
                    *fuel = allowance as u64;
                    if mode.get() == "refill" {
                        *fuel = u64::MAX;
                    }
                    if mode.get() == "no-budget" {
                        *fuel = 0;
                    }
                    remaining_after_host.set(if mode.get() == "refill" { 0 } else { *fuel });
                    if mode.get() == "failure" {
                        Err(Error {
                            kind: ErrorKind::UnmaterializedDispatch,
                            source: Some(SOURCE),
                        })
                    } else {
                        Ok(selection)
                    }
                };
                let mut vm = Interpreter::for_comptime(&package, Budget::default())
                    .unwrap()
                    .with_associated_resolver(&resolver);
                let mut environment = Environment::from([(
                    local(1),
                    Value::Integer(comptime_runtime::Integer::from_u128(0)),
                )]);
                let mut frame = vm.start_frame(&code, &mut environment).unwrap();
                {
                    let mut session = Box::new(Session::fragment(&mut vm, &mut frame));
                    assert!(session.poll().unwrap().is_pending());
                    assert_eq!(calls.get(), 0);
                    assert!(session.poll().unwrap().is_pending());
                    assert_eq!(calls.get(), 1);
                    assert!(
                        session
                            .to_string()
                            .contains("(associated-response (associated-request")
                    );
                    assert_eq!(session.vm.remaining_steps(), remaining_after_host.get());
                    assert!(!session.vm.associated_values.contains_key(&key()));
                    assert!(!session.vm.associated_initializers.contains_key(&key()));
                    assert!(session.vm.active_associated_initializers.is_empty());

                    // Moving the whole continuation retains the pending answer;
                    // neither cancellation nor delivery repeats the source prefix.
                    session = Box::new(*session);
                    if cancel {
                        let remaining = session.vm.remaining_steps();
                        session.cancel();
                        assert_eq!(session.vm.remaining_steps(), remaining);
                        assert_eq!(calls.get(), 1);
                        assert_eq!(
                            session.poll().unwrap_err().kind,
                            ErrorKind::InvalidControlFlow
                        );
                        drop(session);
                    } else {
                        let outcome = (*session).run();
                        if fault == "valid" {
                            assert!(matches!(
                                outcome.unwrap(),
                                Some(Flow::Value(Value::Bool(true)))
                            ));
                        } else {
                            let failure = outcome.err().unwrap();
                            assert_eq!(
                                failure.kind,
                                match fault {
                                    "wrong-domain" => ErrorKind::Type,
                                    "failure" => ErrorKind::UnmaterializedDispatch,
                                    "refill" | "no-budget" => ErrorKind::StepsExhausted,
                                    _ => unreachable!(),
                                }
                            );
                            assert_eq!(failure.source, Some(SOURCE));
                        }
                    }
                }
                assert!(vm.active_associated_initializers.is_empty());
                let published = !cancel && fault == "valid";
                assert_eq!(
                    vm.associated_values.contains_key(&key()),
                    published && !requires_invocation
                );
                assert_eq!(
                    vm.associated_initializers.contains_key(&key()),
                    published && requires_invocation
                );
                let mut environment = frame.restore_environment();
                assert_eq!(
                    environment[&local(1)],
                    Value::Integer(comptime_runtime::Integer::from_u128(1))
                );

                // A new invocation may retry unadmitted host work. A published
                // selection is reused, while its initializer executes normally.
                mode.set("valid");
                vm.budget.steps = Budget::default().steps;
                assert!(matches!(
                    vm.execute_code(&code, &mut environment).unwrap(),
                    Flow::Value(Value::Bool(true))
                ));
                assert_eq!(calls.get(), if published { 1 } else { 2 });
                assert_eq!(
                    environment[&local(1)],
                    Value::Integer(comptime_runtime::Integer::from_u128(2))
                );
                assert!(vm.active_associated_initializers.is_empty());
            }
        }
    }
}
