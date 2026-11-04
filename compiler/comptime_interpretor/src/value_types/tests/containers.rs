use super::*;

fn object(ty: &Term, value: Value) -> Object<ObjectValue> {
    Object::from_entries(vec![ObjectEntry::Property {
        key: "packet".into(),
        value: ObjectValue {
            ty: ty.clone(),
            value,
        },
    }])
}

#[test]
fn capture_failure_does_not_publish_a_closure_and_retry_owns_fresh_storage() {
    let (package, nominal, packet) = fixture();
    let input = LocalId { owner: 1, index: 0 };
    let captured = LocalId { owner: 2, index: 0 };
    let boolean = Term::Primitive(Primitive::Bool);
    let body = Expr {
        source: SOURCE,
        ty: Term::Closure {
            environment: vec![nominal.clone()],
            signature: Box::new(Term::ComptimeFunction(Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(boolean.clone()),
            }))),
        },
        kind: K::Closure {
            parameters: vec![],
            captures: vec![nir::program::Capture {
                binding: captured,
                ty: nominal.clone(),
                value: Some(Box::new(Expr {
                    source: SOURCE,
                    ty: nominal,
                    kind: K::Local(input),
                })),
            }],
            body: Arc::new(Expr {
                source: SOURCE,
                ty: boolean,
                kind: K::Literal(Literal::Bool(true)),
            }),
        },
    };
    let code = Arc::new(comptime_compiler::compile(&body, &mut 10_000).unwrap());
    for compiled in [false, true] {
        let outcome = Cell::new(ContractOutcome::Unknown);
        let solver = |_: &nir::program::TypePredicate, _: &mut u64| outcome.get();
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_budgeted_type_solver(&solver);
        let mut environment = Environment::from([(input, packet.clone())]);
        let run = |vm: &mut Interpreter<'_>, env: &mut Environment| {
            if compiled {
                vm.execute_code(&code, env)
            } else {
                vm.eval(&body, env)
            }
        };
        for (result, expected) in [
            (ContractOutcome::Unknown, ErrorKind::TypeContractUnresolved),
            (ContractOutcome::Rejected, ErrorKind::TypeContractRejected),
        ] {
            outcome.set(result);
            assert_eq!(run(&mut vm, &mut environment).unwrap_err().kind, expected);
            assert!(
                !vm.events
                    .iter()
                    .any(|event| matches!(event, Event::CreateClosure { .. }))
            );
            assert_eq!(environment[&input], packet);
        }
        outcome.set(ContractOutcome::Satisfied);
        let Flow::Value(Value::Closure(closure)) = run(&mut vm, &mut environment).unwrap() else {
            panic!()
        };
        let Value::Storage(storage) = &closure.environment[&captured] else {
            panic!()
        };
        assert_eq!(storage.read().unwrap(), packet);
        assert_eq!(
            vm.events
                .iter()
                .filter(|event| matches!(event, Event::CreateClosure { .. }))
                .count(),
            1
        );
        assert!(vm.assumed_interfaces.is_empty());
    }
}

#[test]
fn object_reads_and_edits_preserve_query_failures_and_old_snapshots() {
    let (package, nominal, packet) = fixture();
    let original = object(&nominal, packet.clone());
    let outcome = Cell::new(ContractOutcome::Unknown);
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| outcome.get();
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_budgeted_type_solver(&solver);
    let integer = || Value::Integer(comptime_runtime::Integer::from_u128(0));
    let object_ty = Term::Primitive(Primitive::Object);
    let input_ty = Term::Tuple(vec![
        object_ty.clone(),
        Term::Primitive(Primitive::Integer),
        nominal.clone(),
    ]);
    let query_ty = Term::Optional(Box::new(nominal.clone()));
    for (result, expected) in [
        (ContractOutcome::Unknown, ErrorKind::TypeContractUnresolved),
        (ContractOutcome::Rejected, ErrorKind::TypeContractRejected),
    ] {
        outcome.set(result);
        assert_eq!(
            vm.lookup_object_property(&original, "packet", &nominal)
                .unwrap_err()
                .kind,
            expected
        );
        assert_eq!(
            vm.complete_object_query(
                nir::program::ObjectQueryKind::Value,
                &query_ty,
                Value::Tuple(vec![Value::Object(original.clone()), integer()]),
                SOURCE,
            )
            .unwrap_err()
            .kind,
            expected
        );
        assert_eq!(
            vm.complete_object_edit(
                nir::program::ObjectEditKind::Replace,
                &object_ty,
                &input_ty,
                Value::Tuple(vec![
                    Value::Object(original.clone()),
                    integer(),
                    packet.clone()
                ]),
                SOURCE,
            )
            .unwrap_err()
            .kind,
            expected
        );
        assert_eq!(original.entries()[0].value().value, packet);
    }
    outcome.set(ContractOutcome::Satisfied);
    assert!(matches!(
        vm.lookup_object_property(&original, "packet", &nominal)
            .unwrap(),
        ObjectProperty::Found(_)
    ));
    vm.complete_object_edit(
        nir::program::ObjectEditKind::Replace,
        &object_ty,
        &input_ty,
        Value::Tuple(vec![
            Value::Object(original.clone()),
            integer(),
            packet.clone(),
        ]),
        SOURCE,
    )
    .unwrap();
    assert!(vm.assumed_interfaces.is_empty());
}

#[test]
fn rejected_object_replacement_keeps_reference_storage_and_can_retry() {
    let (package, nominal, packet) = fixture();
    let old = Value::Object(object(&nominal, packet.clone()));
    let mut changed = packet;
    let Value::Record { fields, .. } = &mut changed else {
        panic!()
    };
    fields[0] = Value::Bool(false);
    let new = Value::Object(object(&nominal, changed));
    let storage = Storage::new(old.clone());
    let mut target =
        Value::ComptimeReference(value_reference::ValueReference::root(storage.clone()));
    let outcome = Cell::new(ContractOutcome::Unknown);
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| outcome.get();
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_budgeted_type_solver(&solver);
    for (result, expected) in [
        (ContractOutcome::Unknown, ErrorKind::TypeContractUnresolved),
        (ContractOutcome::Rejected, ErrorKind::TypeContractRejected),
    ] {
        outcome.set(result);
        assert_eq!(
            vm.write_comptime_place(&mut target, &[], new.clone(), SOURCE)
                .unwrap_err()
                .kind,
            expected
        );
        assert_eq!(storage.read().unwrap(), old);
    }
    outcome.set(ContractOutcome::Satisfied);
    vm.budget.steps = 1;
    assert_eq!(
        vm.write_comptime_place(&mut target, &[], new.clone(), SOURCE)
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(storage.read().unwrap(), old);
    vm.budget.steps = Budget::default().steps;
    vm.write_comptime_place(&mut target, &[], new.clone(), SOURCE)
        .unwrap();
    assert_eq!(storage.read().unwrap(), new);
    let Value::Object(snapshot) = old else {
        panic!()
    };
    let Value::Record { fields, .. } = &snapshot.entries()[0].value().value else {
        panic!()
    };
    assert_eq!(fields[0], Value::Bool(true));
}
