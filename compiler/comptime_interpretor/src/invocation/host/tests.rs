use super::*;
use nir::program::{IntegerOp, Place};
mod contracts;
mod definitions;

const SOURCE: SourceRange = SourceRange { start: 10, end: 20 };
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 70, local }
}
fn local(index: u32) -> LocalId {
    LocalId { owner: 70, index }
}
fn ty(value: Primitive) -> Term {
    Term::Primitive(value)
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty,
        kind,
    }
}
fn read(member: u32) -> Expr {
    expr(
        ty(Primitive::Bool),
        K::AssociatedValue {
            interface: InterfaceInstance {
                declaration: id(1),
                arguments: vec![],
            },
            subject: ty(Primitive::Bool),
            member: id(member),
        },
    )
}
fn integer(value: &str) -> Expr {
    expr(
        ty(Primitive::Integer),
        K::Literal(Literal::Integer(value.into())),
    )
}
fn callable(body: Expr, child: Option<Value>) -> Value {
    Value::Closure(Arc::new(Closure {
        definition_source: None,
        parameters: vec![],
        capture_order: child.as_ref().map(|_| vec![local(0)]).unwrap_or_default(),
        environment: child
            .map(|value| Environment::from([(local(0), value)]))
            .unwrap_or_default(),
        body: Arc::new(body),
        ty: Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(ty(Primitive::Bool)),
        })),
    }))
}
fn fixture(two: bool, nested: bool) -> Value {
    let count = || expr(ty(Primitive::Integer), K::Local(local(1)));
    let mut statements = vec![
        expr(
            ty(Primitive::Void),
            K::Let {
                binding: local(1),
                value: Box::new(integer("0")),
            },
        ),
        expr(
            ty(Primitive::Void),
            K::Assign {
                place: Place {
                    root: local(1),
                    fields: vec![],
                },
                value: Box::new(expr(
                    ty(Primitive::Integer),
                    K::IntegerBinary {
                        op: IntegerOp::Add,
                        left: Box::new(count()),
                        right: Box::new(integer("1")),
                    },
                )),
            },
        ),
        read(2),
    ];
    if two {
        statements.push(read(3));
    }
    let mut value = callable(
        expr(
            ty(Primitive::Bool),
            K::Block {
                statements,
                result: Some(Box::new(expr(
                    ty(Primitive::Bool),
                    K::IntegerBinary {
                        op: IntegerOp::Equal,
                        left: Box::new(count()),
                        right: Box::new(integer("1")),
                    },
                ))),
            },
        ),
        None,
    );
    if nested {
        for _ in 0..8 {
            let signature = match &value {
                Value::Closure(value) => value.ty.clone(),
                _ => unreachable!(),
            };
            value = callable(
                expr(
                    ty(Primitive::Bool),
                    K::Call {
                        callee: Box::new(expr(signature, K::Local(local(0)))),
                        arguments: vec![],
                        parameter_bindings: vec![],
                    },
                ),
                Some(value),
            );
        }
    }
    value
}
fn take<'a>(session: &mut InvocationSession<'_, 'a>) -> AssociatedHostRequest<'a> {
    for _ in 0..1000 {
        match session.poll().unwrap() {
            InvocationProgress::Running => {}
            InvocationProgress::Associated(request) => return request,
            state => panic!("expected new request: {state}"),
        }
    }
    panic!("no request");
}
fn finish(session: &mut InvocationSession<'_, '_>) -> Result<Value, Error> {
    for _ in 0..1000 {
        match session.poll()? {
            InvocationProgress::Running => {}
            InvocationProgress::Complete(value) => return Ok(value),
            state => panic!("unexpected unfinished host work: {state}"),
        }
    }
    panic!("no completion");
}
fn selected(value: Expr, fuel: &mut u64) -> Result<AssociatedSelection, Error> {
    *fuel -= 7;
    let mut allowance = *fuel as usize;
    let initializer =
        nir::associated_initializers::AssociatedInitializer::new(value, &mut allowance).unwrap();
    *fuel = allowance as u64;
    Ok(AssociatedSelection {
        initializer,
        requires_invocation: true,
    })
}
fn answer(request: AssociatedHostRequest<'_>) -> AssociatedHostResponse<'_> {
    request.resolve(|_, fuel| {
        selected(
            expr(ty(Primitive::Bool), K::Literal(Literal::Bool(true))),
            fuel,
        )
    })
}

#[test]
fn external_selection_preserves_nested_activations_and_waits_without_spending() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut active = Box::new(vm.start_invocation(fixture(true, true), vec![]).unwrap());
    let first = take(&mut active);
    assert!(std::ptr::eq(first.source_world(), &package));
    assert!(first.to_string().contains("(generation 1)"));
    let state = active.to_string();
    for _ in 0..4 {
        assert!(matches!(
            active.poll().unwrap(),
            InvocationProgress::Waiting
        ));
        assert_eq!(active.to_string(), state);
        assert_eq!(active.remaining_steps(), 0);
    }
    let stale = first.lease.ticket.clone();
    active = Box::new(*active);
    active.resume(answer(first)).unwrap();
    let second = take(&mut active);
    assert!(second.to_string().contains("(generation 2)"));
    let mut response = answer(second);
    let current = response.lease.ticket.clone();
    response.lease.ticket = stale;
    let state = active.to_string();
    let rejected = active.resume(response).unwrap_err();
    assert_eq!(rejected.error().kind, ErrorKind::InvalidControlFlow);
    assert_eq!(active.to_string(), state);
    assert_eq!(active.remaining_steps(), 0);
    let mut response = rejected.into_response();
    response.lease.ticket = current;
    active.resume(response).unwrap();
    assert_eq!(finish(&mut active).unwrap(), Value::Bool(true));
    assert_eq!(
        active.poll().unwrap_err().kind,
        ErrorKind::InvalidControlFlow
    );
    drop(active);
    assert_eq!(vm.invocation_depth, 0);
    assert_eq!(vm.depth, 0);
    assert!(vm.active_associated_initializers.is_empty());
}

#[test]
fn sessions_reject_foreign_answers_before_touching_waiters_or_budget() {
    let first_world = Package::default();
    let second_world = Package::default();
    for second_world in [&first_world, &second_world] {
        let mut first_vm = Interpreter::for_comptime(&first_world, Budget::default()).unwrap();
        let mut second_vm = Interpreter::for_comptime(second_world, Budget::default()).unwrap();
        let shared = fixture(false, false);
        let mut first = first_vm.start_invocation(shared.clone(), vec![]).unwrap();
        let mut second = second_vm.start_invocation(shared, vec![]).unwrap();
        let response = answer(take(&mut first));
        let second_request = take(&mut second);
        let state = second.to_string();
        let rejected = second.resume(response).unwrap_err();
        assert_eq!(rejected.error().kind, ErrorKind::InvalidControlFlow);
        assert_eq!(second.to_string(), state);
        assert_eq!(second.remaining_steps(), 0);
        first.resume(rejected.into_response()).unwrap();
        second.resume(answer(second_request)).unwrap();
        assert_eq!(finish(&mut first).unwrap(), Value::Bool(true));
        assert_eq!(finish(&mut second).unwrap(), Value::Bool(true));
    }
}

#[test]
fn cancelled_and_completed_sessions_reject_late_responses() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut active = vm.start_invocation(fixture(false, true), vec![]).unwrap();
    let response = answer(take(&mut active));
    active.cancel();
    assert_eq!(
        active.remaining_steps(),
        0,
        "undelivered allowance is not refunded"
    );
    let response = active.resume(response).unwrap_err().into_response();
    drop(active);
    assert_eq!(vm.invocation_depth, 0);
    assert!(
        vm.associated_initializers
            .get_body(&(
                InterfaceInstance {
                    declaration: id(1),
                    arguments: vec![]
                },
                ty(Primitive::Bool),
                id(2)
            ))
            .is_none()
    );
    vm.budget.steps = Budget::default().steps;
    let mut next = vm.start_invocation(fixture(false, false), vec![]).unwrap();
    let current = take(&mut next);
    let state = next.to_string();
    let response = next.resume(response).unwrap_err().into_response();
    assert_eq!(next.to_string(), state);
    next.resume(answer(current)).unwrap();
    assert_eq!(finish(&mut next).unwrap(), Value::Bool(true));
    let remaining = next.remaining_steps();
    assert_eq!(
        next.resume(response).unwrap_err().error().kind,
        ErrorKind::InvalidControlFlow
    );
    assert_eq!(next.remaining_steps(), remaining);
}

#[test]
fn host_failures_bad_domains_and_refills_still_pass_through_admission() {
    let package = Package::default();
    for fault in ["error", "domain", "refill", "exhausted"] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut active = vm.start_invocation(fixture(false, true), vec![]).unwrap();
        let response = take(&mut active).resolve(|_, fuel| {
            let result = selected(
                expr(ty(Primitive::Char), K::Literal(Literal::Char('x'))),
                fuel,
            );
            match fault {
                "error" => Err(Error {
                    kind: ErrorKind::UnmaterializedDispatch,
                    source: Some(SOURCE),
                }),
                "refill" => {
                    *fuel = u64::MAX;
                    result
                }
                "exhausted" => {
                    *fuel = 0;
                    result
                }
                _ => result,
            }
        });
        active.resume(response).unwrap();
        let failure = finish(&mut active).unwrap_err();
        assert_eq!(failure.source, Some(SOURCE));
        assert_eq!(
            failure.kind,
            match fault {
                "error" => ErrorKind::UnmaterializedDispatch,
                "domain" => ErrorKind::Type,
                _ => ErrorKind::StepsExhausted,
            }
        );
        drop(active);
        assert_eq!(vm.invocation_depth, 0);
        assert!(vm.active_associated_initializers.is_empty());
        vm.budget.steps = Budget::default().steps;
        let mut fresh = vm.start_invocation(fixture(false, false), vec![]).unwrap();
        let request = take(&mut fresh);
        fresh.resume(answer(request)).unwrap();
        assert_eq!(finish(&mut fresh).unwrap(), Value::Bool(true));
    }
}

#[test]
fn lost_or_panicking_host_work_requires_cancellation_and_cannot_refund_fuel() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for panic in [false, true] {
        vm.budget.steps = Budget::default().steps;
        let mut active = vm.start_invocation(fixture(false, true), vec![]).unwrap();
        let request = take(&mut active);
        if panic {
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    request.resolve(|_, fuel| {
                        *fuel = u64::MAX;
                        panic!("host interrupted")
                    })
                }))
                .is_err()
            );
        } else {
            drop(request);
        }
        assert!(matches!(
            active.poll().unwrap(),
            InvocationProgress::Waiting
        ));
        assert_eq!(active.remaining_steps(), 0);
        drop(active);
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.remaining_steps(), 0);
    }
}

#[test]
fn source_host_driver_does_not_authorize_runtime_or_tree_execution() {
    let package = Package::default();
    for mut vm in [
        Interpreter::new(&package, Budget::default()).unwrap(),
        Interpreter::for_comptime_tree(&package, Budget::default()).unwrap(),
    ] {
        assert_eq!(
            vm.start_invocation(fixture(false, false), vec![])
                .unwrap_err()
                .kind,
            ErrorKind::PhaseMismatch
        );
        assert_eq!(vm.invocation_depth, 0);
    }
}
