use super::*;

#[test]
fn associated_completion_rechecks_values_and_always_releases_initializer_state() {
    let (package, nominal, packet) = fixture();
    let outcome = Cell::new(ContractOutcome::Unknown);
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| outcome.get();
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_budgeted_type_solver(&solver);
    let key = (
        nir::program::InterfaceInstance {
            declaration: id(5),
            arguments: vec![],
        },
        Term::Primitive(Primitive::Bool),
        id(6),
    );
    for (result, expected) in [
        (ContractOutcome::Unknown, ErrorKind::TypeContractUnresolved),
        (ContractOutcome::Rejected, ErrorKind::TypeContractRejected),
    ] {
        outcome.set(result);
        vm.active_associated_initializers.insert(key.clone());
        assert_eq!(
            vm.complete_associated_value(
                Some(key.clone()),
                &nominal,
                SOURCE,
                Ok(Flow::Value(packet.clone()))
            )
            .unwrap_err()
            .kind,
            expected
        );
        assert!(vm.active_associated_initializers.is_empty());
    }
    outcome.set(ContractOutcome::Satisfied);
    vm.active_associated_initializers.insert(key.clone());
    assert!(
        matches!(vm.complete_associated_value(Some(key), &nominal, SOURCE,
        Ok(Flow::Value(packet.clone()))).unwrap(), Flow::Value(value) if value == packet)
    );
    assert!(vm.active_associated_initializers.is_empty());
    assert!(vm.assumed_interfaces.is_empty());
}

#[test]
fn dynamic_clause_packets_check_formation_before_freezing_values() {
    let (package, nominal, packet) = fixture();
    let outcome = Cell::new(ContractOutcome::Unknown);
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| outcome.get();
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_budgeted_type_solver(&solver);
    let domains = Term::Tuple(vec![nominal.clone()]);
    for (result, expected) in [
        (ContractOutcome::Unknown, ErrorKind::TypeContractUnresolved),
        (ContractOutcome::Rejected, ErrorKind::TypeContractRejected),
    ] {
        outcome.set(result);
        assert_eq!(
            vm.dynamic_instance_arguments(&domains, Value::Tuple(vec![packet.clone()]), SOURCE)
                .unwrap_err()
                .kind,
            expected
        );
        assert!(vm.events.is_empty());
    }
    outcome.set(ContractOutcome::Satisfied);
    assert_eq!(
        vm.dynamic_instance_arguments(&domains, Value::Tuple(vec![packet]), SOURCE)
            .unwrap(),
        vec![Term::NominalValue {
            ty: Box::new(nominal),
            variant: None,
            fields: vec![Term::Bool(true)]
        }]
    );
    assert!(vm.assumed_interfaces.is_empty());
}

#[test]
fn enum_names_check_formation_on_both_execution_paths() {
    let (mut package, nominal, _) = fixture();
    let definition = package.structs.pop().unwrap();
    package.enums.push(nir::program::Enum {
        declaration: definition.declaration,
        name: "State".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        is_intrinsic: false,
        requirements: definition.requirements,
        capabilities: definition.capabilities,
        variants: vec![nir::program::Variant {
            name: "ready".into(),
            shape: nir::program::VariantShape::Unit,
            fields: vec![],
        }],
        groups: vec![],
        encoding: None,
        symbol_encoding: None,
    });
    let input = LocalId { owner: 1, index: 0 };
    let variant = Value::Variant {
        declaration: id(1),
        arguments: vec![],
        index: 0,
        fields: vec![],
    };
    let body = Expr {
        source: SOURCE,
        ty: Term::Primitive(Primitive::Str),
        kind: K::EnumName(Box::new(Expr {
            source: SOURCE,
            ty: nominal,
            kind: K::Local(input),
        })),
    };
    let code = Arc::new(comptime_compiler::compile(&body, &mut 10_000).unwrap());
    for compiled in [false, true] {
        let outcome = Cell::new(ContractOutcome::Unknown);
        let solver = |_: &nir::program::TypePredicate, _: &mut u64| outcome.get();
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_budgeted_type_solver(&solver);
        let mut environment = Environment::from([(input, variant.clone())]);
        let run = |vm: &mut Interpreter<'_>, environment: &mut Environment| {
            if compiled {
                vm.execute_code(&code, environment)
            } else {
                vm.eval(&body, environment)
            }
        };
        for (result, expected) in [
            (ContractOutcome::Unknown, ErrorKind::TypeContractUnresolved),
            (ContractOutcome::Rejected, ErrorKind::TypeContractRejected),
        ] {
            outcome.set(result);
            assert_eq!(run(&mut vm, &mut environment).unwrap_err().kind, expected);
            assert_eq!(environment[&input], variant);
        }
        outcome.set(ContractOutcome::Satisfied);
        assert!(
            matches!(run(&mut vm, &mut environment).unwrap(), Flow::Value(Value::String(name)) if name.as_ref() == "ready")
        );
        assert!(vm.assumed_interfaces.is_empty());
    }
}
