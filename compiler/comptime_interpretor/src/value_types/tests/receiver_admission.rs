use super::*;

#[test]
fn proof_can_change_receiver_before_an_embedded_type_is_admitted() {
    let (package, nominal, record) = fixture();
    let domain = Term::Tuple(vec![nominal.clone(), Term::Primitive(Primitive::Type)]);
    let receiver = ValueReference::owned(Value::Tuple(vec![record, Value::Type(nominal)]));
    let calls = Cell::new(0);
    let solver = |_: &nir::program::TypePredicate, fuel: &mut u64| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            receiver.replace(Value::Unit, fuel)?;
            Ok(ContractOutcome::Satisfied)
        } else {
            Ok(ContractOutcome::Rejected)
        }
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_budgeted_type_solver(&solver);
    let error = vm
        .admit_reference_input(&receiver, &domain, SOURCE)
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::TypeContractRejected);
    assert_eq!(calls.get(), 2);
    assert_eq!(receiver.borrow(&mut 32).unwrap().value(), &Value::Unit);
}

#[test]
fn projected_receiver_checks_match_value_admission_at_every_fuel_boundary() {
    let boolean = Term::Primitive(Primitive::Bool);
    let object = Value::Object(Object::from_entries(vec![ObjectEntry::Value(
        ObjectValue {
            ty: Term::Primitive(Primitive::Type),
            value: Value::Type(boolean.clone()),
        },
    )]));
    let payload = Value::Tuple(vec![
        Value::Tuple(vec![Value::Bool(true); 7]),
        object,
        Value::Bool(false),
    ]);
    let domain = Term::Tuple(vec![
        Term::Tuple(vec![boolean.clone(); 7]),
        Term::Primitive(Primitive::Object),
        boolean,
    ]);
    let root = Storage::new(Value::AnonymousRecord(vec![("payload".into(), payload)]));
    let reference =
        ValueReference::projected(root, &[(nir::program::FieldOwner::Anonymous, 0)], &mut 32)
            .unwrap();
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for steps in 0..512 {
        vm.budget.steps = steps;
        let expected = (|| {
            let snapshot = {
                let guard = reference
                    .borrow(&mut vm.budget.steps)
                    .map_err(|kind| vm.failure(kind, Some(SOURCE)))?;
                vm.check_borrowed_representation(guard.value(), SOURCE, 0)?;
                guard.value().clone()
            };
            vm.check_value_type(&snapshot, &domain, Some(SOURCE))?;
            vm.admit_embedded_type_descriptions(&snapshot, 0)
        })();
        let remaining = vm.remaining_steps();
        vm.budget.steps = steps;
        assert_eq!(
            vm.admit_reference_input(&reference, &domain, SOURCE),
            expected,
            "steps={steps}"
        );
        assert_eq!(vm.remaining_steps(), remaining, "steps={steps}");
        if expected.is_ok() {
            return;
        }
    }
    panic!("receiver was never admitted");
}
