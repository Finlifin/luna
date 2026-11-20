//! Ordinary declaration contracts can suspend through the public session API.
use super::*;

fn package(returned: bool) -> Package {
    let body = if returned {
        expr(ty(Primitive::Never), K::Return(Some(Box::new(read(3)))))
    } else {
        read(3)
    };
    Package {
        traits: vec![nir::program::Trait {
            comptime_only: true,
            parameter_types: vec![],
            associated: (2..=4)
                .map(|member| nir::program::AssociatedContract {
                    phase: nir::program::TraitPhase::Comptime,
                    declaration: id(member),
                    name: format!("condition_{member}"),
                    ty: ty(Primitive::Bool),
                    default: None,
                })
                .collect(),
            requirements: vec![],
            capabilities: vec![],
            type_parameters: vec![],
            declaration: id(1),
            parents: vec![],
            self_parameter: id(5),
            name: "Conditions".into(),
            methods: vec![],
        }],
        functions: vec![nir::program::Function {
            interface: nir::program::FunctionDeclaration {
                source: nir::program::SourceRange { start: 0, end: 0 },
                phase: nir::program::FunctionPhase::Comptime,
                inline: false,
                pure: false,
                is_unsafe: false,
                preconditions: vec![read(2)],
                postconditions: vec![read(4), expr(ty(Primitive::Bool), K::Local(local(8)))],
                result_binding: Some(local(8)),
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration: id(10),
                name: "contracted".into(),
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(ty(Primitive::Bool)),
                },
                parameters: vec![],
                env_parameters: vec![],
                defaults: vec![],
            },
            body,
            runtime_deliveries: Default::default(),
        }],
        ..Package::default()
    }
}

fn request_at<'a>(
    session: &mut InvocationSession<'_, 'a>,
    member: u32,
) -> AssociatedHostRequest<'a> {
    let request = take(session);
    assert!(
        matches!(&request.access().kind, K::AssociatedValue { member: actual, .. } if *actual == id(member))
    );
    request
}

#[test]
fn entry_body_and_exit_share_a_suspendable_activation() {
    for returned in [false, true] {
        let package = package(returned);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut active = vm
            .start_invocation(Value::Function(id(10)), vec![])
            .unwrap();
        for (member, phase) in [
            (2, "(entry (next 0))"),
            (3, "(body (cache-key "),
            (4, "(exit (next 0)"),
        ] {
            let request = request_at(&mut active, member);
            let saved = active.to_string();
            assert!(saved.contains(phase), "{saved}");
            for _ in 0..3 {
                assert!(matches!(
                    active.poll().unwrap(),
                    InvocationProgress::Waiting
                ));
                assert_eq!(active.to_string(), saved);
            }
            active.resume(answer(request)).unwrap();
        }
        assert_eq!(finish(&mut active).unwrap(), Value::Bool(true));
        drop(active);
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.depth, 0);
        let checks = vm
            .events()
            .iter()
            .filter(|event| matches!(event, Event::CheckPredicate { passed: true, .. }))
            .count();
        assert_eq!(checks, 3);
        assert_eq!(vm.events().iter().filter(|event| matches!(event, Event::EnterFunction(declaration) if *declaration == id(10))).count(), 1);
        assert_eq!(
            vm.bytecode_invocations, 1,
            "conditions are not extra function entries"
        );
    }
}

#[test]
fn rejected_contracts_stop_at_the_current_stage_and_restore_the_caller() {
    for failed in [2, 4] {
        let package = package(false);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut active = vm
            .start_invocation(Value::Function(id(10)), vec![])
            .unwrap();
        for member in 2..=failed {
            let request = request_at(&mut active, member);
            let passed = member != failed;
            active
                .resume(request.resolve(|_, fuel| {
                    selected(
                        expr(ty(Primitive::Bool), K::Literal(Literal::Bool(passed))),
                        fuel,
                    )
                }))
                .unwrap();
        }
        let failure = finish(&mut active).unwrap_err();
        assert_eq!(failure.kind, ErrorKind::ProgramContractRejected);
        assert_eq!(failure.source, Some(SOURCE));
        drop(active);
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.events().iter().filter(|event| matches!(event, Event::EnterFunction(declaration) if *declaration == id(10))).count(), usize::from(failed == 4));
    }
}

#[test]
fn exhausted_contract_requests_unwind_without_advancing_the_body_or_exit() {
    for exhausted in [2, 4] {
        let package = package(false);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut active = vm
            .start_invocation(Value::Function(id(10)), vec![])
            .unwrap();
        for member in 2..exhausted {
            let request = request_at(&mut active, member);
            active.resume(answer(request)).unwrap();
        }
        let request = request_at(&mut active, exhausted);
        active
            .resume(request.resolve(|_, fuel| {
                let selection = selected(
                    expr(ty(Primitive::Bool), K::Literal(Literal::Bool(true))),
                    fuel,
                );
                *fuel = 0;
                selection
            }))
            .unwrap();
        assert_eq!(
            finish(&mut active).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        drop(active);
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.remaining_steps(), 0);
        assert_eq!(vm.events().iter().filter(|event| matches!(event, Event::EnterFunction(declaration) if *declaration == id(10))).count(), usize::from(exhausted == 4));
        assert_eq!(
            vm.events()
                .iter()
                .filter(|event| matches!(event, Event::CheckPredicate { .. }))
                .count(),
            usize::from(exhausted == 4)
        );
    }
}

#[test]
fn cancelling_each_contract_stage_discards_work_without_executing_the_rest() {
    for cancelled in 2..=4 {
        let package = package(false);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut active = vm
            .start_invocation(Value::Function(id(10)), vec![])
            .unwrap();
        for member in 2..cancelled {
            let request = request_at(&mut active, member);
            active.resume(answer(request)).unwrap();
        }
        let request = request_at(&mut active, cancelled);
        active.cancel();
        assert_eq!(
            active.poll().unwrap_err().kind,
            ErrorKind::InvalidControlFlow
        );
        drop(active);
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.depth, 0);
        assert!(vm.active_associated_initializers.is_empty());
        assert_eq!(
            vm.remaining_steps(),
            0,
            "undelivered host allowance is not refunded"
        );
        // An explicit new allowance permits a retry; cancellation cannot mint it.
        vm.budget.steps = Budget::default().steps;
        let mut retry = vm
            .start_invocation(Value::Function(id(10)), vec![])
            .unwrap();
        let current = request_at(&mut retry, cancelled);
        let remaining = retry.remaining_steps();
        assert_eq!(
            retry.resume(answer(request)).unwrap_err().error().kind,
            ErrorKind::InvalidControlFlow
        );
        assert_eq!(retry.remaining_steps(), remaining);
        retry.resume(answer(current)).unwrap();
        for member in cancelled + 1..=4 {
            let request = request_at(&mut retry, member);
            retry.resume(answer(request)).unwrap();
        }
        assert_eq!(finish(&mut retry).unwrap(), Value::Bool(true));
    }
}
