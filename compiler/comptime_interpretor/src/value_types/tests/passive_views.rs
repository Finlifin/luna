use super::*;

fn projection() -> Term {
    Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(1),
            member: id(2),
        }),
        subject: Box::new(Term::Primitive(Primitive::Bool)),
        arguments: vec![],
    }
}

#[test]
fn value_shape_observations_share_fuel_preserve_failures_and_recover() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let key = (
        InterfaceInstance {
            declaration: id(1),
            arguments: vec![],
        },
        Term::Primitive(Primitive::Bool),
        id(2),
    );
    let selected = Expr {
        source: SOURCE,
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
    };
    assert_eq!(
        vm.check_value_type(&Value::Bool(true), &projection(), Some(SOURCE))
            .unwrap_err(),
        Error {
            kind: ErrorKind::UnmaterializedDispatch,
            source: Some(SOURCE)
        }
    );
    vm.associated_values.insert(key, &selected);
    vm.budget.steps = 1000;
    vm.check_value_type(&Value::Bool(true), &projection(), Some(SOURCE))
        .unwrap();
    let cost = 1000 - vm.remaining_steps();
    assert!(
        cost > 1,
        "projection work must be charged beyond the shape visit"
    );
    vm.budget.steps = cost;
    let pair = Value::Tuple(vec![Value::Bool(true), Value::Bool(false)]);
    let pair_type = Term::Tuple(vec![projection(), projection()]);
    assert_eq!(
        vm.check_value_type(&pair, &pair_type, Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    vm.budget.steps = 1000;
    vm.check_value_type(&pair, &pair_type, Some(SOURCE))
        .unwrap();
    assert!(1000 - vm.remaining_steps() >= cost * 2);
    assert_eq!(
        vm.check_value_type(&Value::Char('a'), &projection(), Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert!(
        vm.events.is_empty(),
        "passive shape checks must not execute code"
    );
}

#[test]
fn retained_capability_limits_cannot_trigger_a_second_proof_attempt() {
    let package = Package::default();
    let calls = Cell::new(0);
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| {
        calls.set(calls.get() + 1);
        Ok(ContractOutcome::Satisfied)
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_budgeted_type_solver(&solver);
    let requirement = nir::program::TraitRequirement {
        interface: id(3),
        subject: projection(),
        interface_arguments: vec![],
    };
    assert_eq!(
        vm.capabilities_hold_parameters(std::slice::from_ref(&requirement), &[], &[], &mut 0),
        Err(ErrorKind::StepsExhausted)
    );
    assert_eq!(
        vm.capabilities_hold_parameters(std::slice::from_ref(&requirement), &[], &[], &mut 1000),
        Err(ErrorKind::UnmaterializedDispatch)
    );
    assert_eq!(
        calls.get(),
        0,
        "retained queries cannot invoke the proof service"
    );
    vm.budget.steps = 1;
    assert_eq!(
        vm.check_capability_contract(&requirement, SOURCE)
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(calls.get(), 0);
    vm.budget.steps = 1000;
    vm.check_capability_contract(&requirement, SOURCE).unwrap();
    assert_eq!(
        calls.get(),
        1,
        "missing retained views may use the active proof service"
    );
    assert!(vm.events.is_empty());
}

#[test]
fn read_only_scalar_shapes_do_not_invent_a_new_width_limit() {
    let package = Package::default();
    let vm = Interpreter::new(&package, Budget::default()).unwrap();
    let values = Value::Tuple(vec![Value::Bool(true); 100_001]);
    let ty = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 100_001]);
    assert!(vm.has_type(&values, &ty));
}
