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
        vm.retains_capability(&requirement, &mut 0),
        Err(ErrorKind::StepsExhausted)
    );
    assert_eq!(
        vm.retains_capability(&requirement, &mut 1000),
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

#[test]
fn passive_nominal_checks_use_the_admitted_dependent_input_for_capabilities() {
    // Packet<T, X: T> requires bool :- Choice<T, X>.
    let (mut package, _, _) = fixture();
    let type_domain = Term::Primitive(Primitive::Type);
    let boolean = Term::Primitive(Primitive::Bool);
    let type_parameter = |local| Term::Rigid {
        declaration: id(local),
        sort: nir::Sort::Type,
    };
    let record = &mut package.structs[0];
    record.type_parameters = vec![id(3), id(4)];
    record.parameter_types = vec![type_domain.clone(), type_parameter(3)];
    record.capabilities[0].interface_arguments = vec![
        type_parameter(3),
        Term::Rigid {
            declaration: id(4),
            sort: nir::Sort::ComptimeValue,
        },
    ];
    package.traits.push(nir::program::Trait {
        declaration: id(2),
        name: "Choice".into(),
        self_parameter: id(5),
        type_parameters: vec![id(6), id(7)],
        parameter_types: vec![type_domain.clone(), type_parameter(6)],
        comptime_only: false,
        parents: vec![],
        requirements: vec![],
        capabilities: vec![],
        associated: vec![],
        methods: vec![],
    });
    for arguments in [
        vec![type_domain, boolean.clone()],
        vec![boolean.clone(), Term::Bool(false)],
    ] {
        let requirement = nir::program::TraitRequirement {
            interface: id(2),
            subject: boolean.clone(),
            interface_arguments: arguments.clone(),
        };
        let value = Value::Record {
            declaration: id(1),
            arguments: arguments.clone(),
            fields: vec![Value::Bool(true)],
        };
        let ty = Term::Nominal {
            declaration: id(1),
            arguments,
        };
        let absent = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert!(!absent.has_type(&value, &ty));
        let vm =
            Interpreter::for_comptime_with_assumptions(&package, &[requirement], Budget::default())
                .unwrap();
        assert!(vm.has_type(&value, &ty));
    }
}
