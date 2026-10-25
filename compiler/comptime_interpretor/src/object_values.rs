use super::*;

fn object(values: Vec<Value>) -> Value {
    Value::Object(Object::from_entries(
        values
            .into_iter()
            .enumerate()
            .map(|(i, value)| {
                if i % 2 == 0 {
                    ObjectEntry::Value(value)
                } else {
                    ObjectEntry::Property {
                        key: "key\"\\\n".into(),
                        value,
                    }
                }
            })
            .collect(),
    ))
}

#[test]
fn traversal_reaches_nested_owned_values_and_deduplicates_shared_cells() {
    let child = Storage::new(Value::Bool(true));
    let id = DeclarationId {
        package: 7,
        local: 9,
    };
    let value = object(vec![
        Value::Storage(child.clone()),
        object(vec![Value::Storage(child), Value::Function(id)]),
    ]);
    let mut cells = 0;
    let mut booleans = 0;
    let mut functions = vec![];
    value
        .visit_owned_graph(&mut 100, &mut |value, _| {
            match value {
                Value::Storage(_) => cells += 1,
                Value::Bool(_) => booleans += 1,
                Value::Function(id) => functions.push(*id),
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert_eq!((cells, booleans), (1, 1));
    assert_eq!(functions, vec![id]);
    assert_eq!(
        value.visit_owned_graph(&mut 1, &mut |_, _| Ok(())),
        Err(ErrorKind::StepsExhausted)
    );
}

#[test]
fn immutable_work_counts_payloads_and_keys_and_rejects_hidden_state() {
    let value = Value::Object(Object::from_entries(vec![ObjectEntry::Property {
        key: "abc".into(),
        value: Value::Bool(true),
    }]));
    assert_eq!(value.immutable_copy_work(), Some(5));
    assert_eq!(
        object(vec![Value::Storage(Storage::new(Value::Bool(true)))]).immutable_copy_work(),
        None
    );
    let invalid = object(vec![Value::Type(Term::Rigid {
        declaration: DeclarationId {
            package: 1,
            local: 2,
        },
        sort: nir::Sort::Type,
    })]);
    assert_eq!(invalid.immutable_copy_work(), None);
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.admit_embedded_type_descriptions(&invalid, 0)
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
}

#[test]
fn checkpoints_restore_storage_inside_objects_without_changing_identity() {
    let child = Storage::new(Value::Bool(false));
    let root = Storage::new(object(vec![Value::Storage(child.clone())]));
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let saved = vm
        .checkpoint_owned_graph(&[Value::Storage(root.clone())])
        .unwrap();
    *child.0.lock().unwrap() = Value::Bool(true);
    *root.0.lock().unwrap() = Value::Unit;
    saved.restore().unwrap();
    assert_eq!(child.read().unwrap(), Value::Bool(false));
    let Value::Object(value) = vm.read_owned_value(&root).unwrap() else {
        panic!()
    };
    let Value::Storage(observed) = value.entries()[0].value() else {
        panic!()
    };
    assert!(Arc::ptr_eq(&child.0, &observed.0));
}

#[test]
fn explicit_representation_copy_uses_fresh_owned_cells() {
    let child = Storage::new(Value::Bool(false));
    let root = Storage::new(object(vec![Value::Storage(child.clone())]));
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let Value::Object(copy) = vm.copy_owned_value(&root).unwrap() else {
        panic!()
    };
    let Value::Storage(copied) = copy.entries()[0].value() else {
        panic!()
    };
    assert!(!Arc::ptr_eq(&child.0, &copied.0));
    *copied.0.lock().unwrap() = Value::Bool(true);
    assert_eq!(child.read().unwrap(), Value::Bool(false));
}

#[test]
fn budget_and_runtime_rejections_preserve_original_storage() {
    let value = Value::Object(Object::from_entries(vec![ObjectEntry::Property {
        key: "x".repeat(1024),
        value: Value::Bool(true),
    }]));
    let root = Storage::new(value.clone());
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 30,
            depth: 128,
        },
    )
    .unwrap();
    assert_eq!(
        vm.read_owned_value(&root).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(root.read().unwrap(), value);
    let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        runtime.read_owned_value(&root).unwrap_err().kind,
        ErrorKind::Unsupported
    );
    assert_eq!(root.read().unwrap(), value);
}

#[test]
fn unsupported_freezing_drops_deep_objects_iteratively() {
    let mut value = Value::Bool(true);
    for _ in 0..20_000 {
        value = object(vec![value]);
    }
    assert_eq!(
        value.into_normal_term(&mut Default::default()),
        Err(ErrorKind::Unsupported)
    );
}

#[test]
fn object_values_dump_the_ordered_container_not_a_record() {
    let value = object(vec![Value::Bool(false), Value::Bool(true)]);
    assert_eq!(
        value.to_string(),
        "(object (value (bool false)) (property (key \"key\\\"\\\\\\n\") (value (bool true))))"
    );
    assert_eq!(value.clone().to_string(), value.to_string());
}
