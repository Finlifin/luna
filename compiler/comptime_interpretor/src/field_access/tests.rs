use super::*;
fn local() -> LocalId {
    LocalId { owner: 1, index: 0 }
}
fn source() -> SourceRange {
    SourceRange { start: 5, end: 9 }
}
fn base() -> Expr {
    Expr {
        source: SourceRange { start: 1, end: 4 },
        ty: Term::Record(vec![]),
        kind: K::Local(local()),
    }
}
fn field(name: &str, index: usize) -> Expr {
    Expr {
        source: source(),
        ty: Term::Primitive(Primitive::Bool),
        kind: K::AnonymousField {
            value: Box::new(base()),
            name: name.into(),
            index,
        },
    }
}
fn record(size: usize) -> Value {
    Value::AnonymousRecord(vec![
        ("selected".into(), Value::Bool(true)),
        (
            "unrelated".into(),
            Value::Tuple(vec![Value::Bool(false); size]),
        ),
    ])
}
fn environment(value: Value) -> Environment {
    let mut env = Environment::new();
    env.insert(local(), value);
    env
}

#[test]
fn field_read_budget_is_independent_of_unselected_builder_state() {
    let package = Package::default();
    for addressable in [false, true] {
        let mut costs = Vec::new();
        let mut general_costs = Vec::new();
        for size in [1, 1024, 65536] {
            for optimized in [true, false] {
                let mut vm = Interpreter::for_comptime(
                    &package,
                    Budget {
                        steps: 1_000_000,
                        depth: 128,
                    },
                )
                .unwrap();
                // Both inputs contain only ordinary bool/tuple/record values.
                // Runtime mode selects the unchanged general projection path.
                vm.comptime_only = optimized;
                let value = record(size);
                let mut env = environment(if addressable {
                    Value::Storage(Storage::new(value))
                } else {
                    value
                });
                let before = vm.remaining_steps();
                let mut expression = field("selected", 0);
                let K::AnonymousField { value, .. } = &mut expression.kind else {
                    unreachable!()
                };
                value.ty = Term::Record(vec![
                    ("selected".into(), Term::Primitive(Primitive::Bool)),
                    (
                        "unrelated".into(),
                        Term::Tuple(vec![Term::Primitive(Primitive::Bool); size]),
                    ),
                ]);
                assert!(matches!(
                    vm.eval(&expression, &mut env).unwrap(),
                    Flow::Value(Value::Bool(true))
                ));
                if optimized {
                    costs.push(before - vm.remaining_steps());
                } else {
                    general_costs.push(before - vm.remaining_steps());
                }
            }
        }
        assert!(costs.iter().all(|cost| *cost == costs[0]), "{costs:?}");
        assert!(costs[0] < 16, "{costs:?}");
        assert!(
            general_costs[2] > costs[2] * 1000,
            "{general_costs:?} versus {costs:?}"
        );
        println!(
            "addressable={addressable}, siblings=[1,1024,65536], selected-field steps={costs:?}, general steps={general_costs:?}"
        );
    }
}

#[test]
fn malformed_selection_missing_local_and_locked_storage_keep_diagnostics() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let error = vm
        .eval(&field("selected", 0), &mut Environment::new())
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::MissingBinding);
    assert_eq!(error.source, Some(base().source));
    let mut env = environment(record(3));
    for expression in [
        field("wrong", 0),
        field("selected", 9),
        field("unrelated", 1),
    ] {
        let error = vm.eval(&expression, &mut env).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Type);
        assert_eq!(error.source, Some(source()));
    }
    let storage = Storage::new(record(3));
    let mut env = environment(Value::Storage(storage.clone()));
    let guard = storage.0.try_lock().unwrap();
    let error = vm.eval(&field("selected", 0), &mut env).unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidPointer);
    assert_eq!(error.source, Some(base().source));
    drop(guard);
    assert!(vm.eval(&field("selected", 0), &mut env).is_ok());
}

#[test]
fn field_snapshots_do_not_alias_later_record_mutations_and_charge_selected_data() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let storage = Storage::new(record(2048));
    let mut env = environment(Value::Storage(storage.clone()));
    let first = vm.eval(&field("selected", 0), &mut env).unwrap();
    {
        let mut guard = storage.0.try_lock().unwrap();
        let Value::AnonymousRecord(fields) = &mut *guard else {
            unreachable!()
        };
        fields[0].1 = Value::Bool(false);
    }
    assert!(matches!(first, Flow::Value(Value::Bool(true))));
    assert!(matches!(
        vm.eval(&field("selected", 0), &mut env).unwrap(),
        Flow::Value(Value::Bool(false))
    ));
    let mut large = field("unrelated", 1);
    large.ty = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 2048]);
    vm.budget.steps = 32;
    assert_eq!(
        vm.eval(&large, &mut env).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
}

#[test]
fn nominal_fields_check_declaration_and_runtime_reads_use_the_general_path() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let declaration = DeclarationId {
        package: 12,
        local: 1,
    };
    let mut env = environment(Value::Record {
        declaration,
        arguments: vec![],
        fields: vec![Value::Bool(true)],
    });
    let mut expression = Expr {
        source: source(),
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Field {
            value: Box::new(base()),
            declaration,
            index: 0,
        },
    };
    assert!(matches!(
        vm.eval(&expression, &mut env).unwrap(),
        Flow::Value(Value::Bool(true))
    ));
    if let K::Field { declaration, .. } = &mut expression.kind {
        declaration.local += 1;
    }
    assert_eq!(
        vm.eval(&expression, &mut env).unwrap_err().kind,
        ErrorKind::Type
    );
    vm.comptime_only = false;
    assert!(vm.eval_local_field(&expression, &env).is_none());
}

#[test]
fn computed_record_receivers_keep_the_general_control_flow_path() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut expression = field("selected", 0);
    let K::AnonymousField { value, .. } = &mut expression.kind else {
        unreachable!()
    };
    value.kind = K::Block {
        statements: vec![],
        result: Some(Box::new(base())),
    };
    let mut env = environment(record(3));
    assert!(vm.eval_local_field(&expression, &env).is_none());
    assert!(matches!(
        vm.eval(&expression, &mut env).unwrap(),
        Flow::Value(Value::Bool(true))
    ));
    let K::AnonymousField { value, .. } = &mut expression.kind else {
        unreachable!()
    };
    value.kind = K::Return(Some(Box::new(Expr {
        source: source(),
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Literal(Literal::Bool(false)),
    })));
    value.ty = Term::Primitive(Primitive::Never);
    assert!(matches!(
        vm.eval(&expression, &mut env).unwrap(),
        Flow::Return(Value::Bool(false))
    ));
}
