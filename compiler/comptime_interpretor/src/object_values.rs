use super::*;

fn object(values: Vec<Value>) -> Value {
    Value::Object(Object::from_entries(
        values
            .into_iter()
            .enumerate()
            .map(|(i, value)| {
                let ty = match &value {
                    Value::Object(_) => Term::Primitive(Primitive::Object),
                    Value::Type(_) => Term::Primitive(Primitive::Type),
                    Value::Function(_) => Term::Function {
                        parameters: vec![],
                        result: Box::new(Term::Primitive(Primitive::Void)),
                    },
                    _ => Term::Primitive(Primitive::Bool),
                };
                let value = ObjectValue { ty, value };
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
        value: ObjectValue {
            ty: Term::Primitive(Primitive::Bool),
            value: Value::Bool(true),
        },
    }]));
    assert_eq!(value.immutable_copy_work(), Some(6));
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
    let Value::Storage(observed) = &value.entries()[0].value().value else {
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
    let Value::Storage(copied) = &copy.entries()[0].value().value else {
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
        value: ObjectValue {
            ty: Term::Primitive(Primitive::Bool),
            value: Value::Bool(true),
        },
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
fn depth_limited_freezing_drops_deep_objects_iteratively() {
    let mut value = Value::Bool(true);
    for _ in 0..20_000 {
        value = object(vec![value]);
    }
    assert_eq!(
        value.into_normal_term(&mut Default::default()),
        Err(ErrorKind::DepthExhausted)
    );
}

#[test]
fn object_values_dump_the_ordered_container_not_a_record() {
    let value = object(vec![Value::Bool(false), Value::Bool(true)]);
    assert_eq!(
        value.to_string(),
        "(object (value (object-value (type (primitive bool)) (value (bool false)))) (property (key \"key\\\"\\\\\\n\") (value (object-value (type (primitive bool)) (value (bool true))))))"
    );
    assert_eq!(value.clone().to_string(), value.to_string());
}

fn identity_package() -> Package {
    let local = LocalId { owner: 0, index: 0 };
    let ty = Term::Primitive(Primitive::Object);
    Package {
        functions: vec![nir::program::Function {
            declaration: DeclarationId {
                package: 1,
                local: 0,
            },
            name: "forward".into(),
            phase: nir::program::FunctionPhase::Comptime,
            inline: false,
            pure: true,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            signature: Term::Function {
                parameters: vec![nir::Parameter {
                    sort: nir::Sort::Runtime,
                    ty: ty.clone(),
                }],
                result: Box::new(ty.clone()),
            },
            parameters: vec![local],
            defaults: vec![],
            body: Expr {
                ty,
                source: SourceRange { start: 0, end: 1 },
                kind: K::Local(local),
            },
        }],
        ..Default::default()
    }
}

#[test]
fn typed_objects_roundtrip_through_comptime_parameters_and_results() {
    let mut package = identity_package();
    let head = DeclarationId {
        package: 1,
        local: 9,
    };
    package.normal_forms.push(nir::normal_forms::Head {
        role: nir::normal_forms::Role::Opaque,
        declaration: head,
        parameters: vec![],
        requirements: vec![],
    });
    let opaque = Term::NormalForm {
        head,
        sort: nir::Sort::Type,
        arguments: vec![],
    };
    let input = object(vec![
        Value::Bool(true),
        object(vec![Value::Type(opaque.clone())]),
    ]);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let returned = vm
        .invoke(package.functions[0].declaration, vec![input.clone()])
        .unwrap();
    assert_eq!(returned, input);
    let Value::Object(outer) = returned else {
        panic!()
    };
    assert_eq!(
        outer.entries()[1].value().ty,
        Term::Primitive(Primitive::Object)
    );
    let Value::Object(inner) = &outer.entries()[1].value().value else {
        panic!()
    };
    assert_eq!(inner.entries()[0].value().value, Value::Type(opaque));
}

#[test]
fn forged_entry_tags_and_runtime_object_signatures_are_rejected() {
    let package = identity_package();
    let forged = Value::Object(Object::from_entries(vec![ObjectEntry::Value(
        ObjectValue {
            ty: Term::Primitive(Primitive::Char),
            value: Value::Bool(true),
        },
    )]));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(package.functions[0].declaration, vec![forged])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    let valid = object(vec![Value::Bool(true)]);
    assert_eq!(
        vm.invoke(package.functions[0].declaration, vec![valid.clone()])
            .unwrap(),
        valid
    );
    let mut runtime = package.clone();
    runtime.functions[0].phase = nir::program::FunctionPhase::Runtime;
    assert!(nir::phases::validate(&runtime).is_err());
    // Frozen Object inputs do not grant runtime transport.
    assert!(nir::normal_forms::supported_input_type(
        &Term::Primitive(Primitive::Object),
        &[]
    ));
}

#[test]
fn object_evaluation_checks_shape_and_payload_types_even_without_phase_preflight() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let source = SourceRange { start: 4, end: 8 };
    let field = Expr {
        ty: Term::Primitive(Primitive::Bool),
        source,
        kind: K::Literal(Literal::Bool(true)),
    };
    let mut value = Expr {
        ty: Term::Primitive(Primitive::Object),
        source,
        kind: K::Object {
            names: vec![],
            fields: vec![field.clone()],
        },
    };
    assert_eq!(
        vm.eval(&value, &mut Environment::new()).unwrap_err().kind,
        ErrorKind::Type
    );
    value.kind = K::Object {
        names: vec![Some("x".into())],
        fields: vec![field],
    };
    let Flow::Value(Value::Object(result)) = vm.eval(&value, &mut Environment::new()).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        result.entries()[0].value().ty,
        Term::Primitive(Primitive::Bool)
    );
    assert_eq!(result.entries()[0].value().value, Value::Bool(true));
    let K::Object { fields, .. } = &mut value.kind else {
        panic!()
    };
    fields[0].kind = K::Literal(Literal::Char('x'));
    assert_eq!(
        vm.eval(&value, &mut Environment::new()).unwrap_err().kind,
        ErrorKind::Type
    );
}

#[test]
fn property_lookup_distinguishes_missing_and_mismatch_without_copying() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let data = Object::from_entries(vec![
        ObjectEntry::Property {
            key: "flag".into(),
            value: ObjectValue {
                ty: Term::Primitive(Primitive::Bool),
                value: Value::Bool(true),
            },
        },
        ObjectEntry::Value(ObjectValue {
            ty: Term::Primitive(Primitive::Bool),
            value: Value::Bool(false),
        }),
        ObjectEntry::Property {
            key: "flag".into(),
            value: ObjectValue {
                ty: Term::Primitive(Primitive::Char),
                value: Value::Char('x'),
            },
        },
    ]);
    let ObjectProperty::TypeMismatch(value) = vm
        .lookup_object_property(&data, "flag", &Term::Primitive(Primitive::Bool))
        .unwrap()
    else {
        panic!()
    };
    assert!(std::ptr::eq(value, data.entries()[2].value()));
    let ObjectProperty::Found(value) = vm
        .lookup_object_property(&data, "flag", &Term::Primitive(Primitive::Char))
        .unwrap()
    else {
        panic!()
    };
    assert!(std::ptr::eq(value, data.entries()[2].value()));
    assert!(matches!(
        vm.lookup_object_property(&data, "absent", &Term::Primitive(Primitive::Bool))
            .unwrap(),
        ObjectProperty::Missing
    ));
    let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        runtime
            .lookup_object_property(&data, "flag", &Term::Primitive(Primitive::Char))
            .unwrap_err()
            .kind,
        ErrorKind::PhaseMismatch
    );
}

#[test]
fn property_lookup_rejects_forged_hidden_tags_and_recovers() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let invalid = Object::from_entries(vec![
        ObjectEntry::Property {
            key: "flag".into(),
            value: ObjectValue {
                ty: Term::Primitive(Primitive::Char),
                value: Value::Bool(true),
            },
        },
        ObjectEntry::Property {
            key: "flag".into(),
            value: ObjectValue {
                ty: Term::Primitive(Primitive::Bool),
                value: Value::Bool(false),
            },
        },
    ]);
    assert_eq!(
        vm.lookup_object_property(&invalid, "flag", &Term::Primitive(Primitive::Bool))
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    let valid = Object::from_entries(vec![invalid.entries()[1].clone()]);
    assert!(matches!(
        vm.lookup_object_property(&valid, "flag", &Term::Primitive(Primitive::Bool))
            .unwrap(),
        ObjectProperty::Found(_)
    ));
    let mut exhausted = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    exhausted
        .charge_preparation(exhausted.remaining_steps() as usize)
        .unwrap();
    assert_eq!(
        exhausted
            .lookup_object_property(&valid, "flag", &Term::Primitive(Primitive::Bool))
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
}

#[test]
fn property_lookup_keeps_opaque_type_identity_and_type_payloads() {
    let head = DeclarationId {
        package: 1,
        local: 9,
    };
    let mut package = Package::default();
    package.normal_forms.push(nir::normal_forms::Head {
        role: nir::normal_forms::Role::Opaque,
        declaration: head,
        parameters: vec![],
        requirements: vec![],
    });
    let opaque = Term::NormalForm {
        head,
        sort: nir::Sort::Type,
        arguments: vec![],
    };
    let data = Object::from_entries(vec![
        ObjectEntry::Property {
            key: "flag".into(),
            value: ObjectValue {
                ty: Term::Primitive(Primitive::Bool),
                value: Value::Bool(true),
            },
        },
        ObjectEntry::Property {
            key: "ty".into(),
            value: ObjectValue {
                ty: Term::Primitive(Primitive::Type),
                value: Value::Type(opaque.clone()),
            },
        },
    ]);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(matches!(
        vm.lookup_object_property(&data, "flag", &opaque).unwrap(),
        ObjectProperty::TypeMismatch(_)
    ));
    let ObjectProperty::Found(value) = vm
        .lookup_object_property(&data, "ty", &Term::Primitive(Primitive::Type))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(value.value, Value::Type(opaque));
    assert_eq!(
        vm.lookup_object_property(&data, "flag", &Term::Bool(true))
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
}

#[test]
fn property_lookup_does_not_copy_or_extend_pointer_storage_lifetime() {
    let package = Package::default();
    let storage = Storage::new(Value::Bool(true));
    let pointee = Term::Primitive(Primitive::Bool);
    let pointer = storage.pointer(pointee.clone(), vec![]);
    let ty = Term::Pointer(Box::new(pointee));
    let data = Object::from_entries(vec![ObjectEntry::Property {
        key: "pointer".into(),
        value: ObjectValue {
            ty: ty.clone(),
            value: Value::Pointer(pointer),
        },
    }]);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let ObjectProperty::Found(value) = vm.lookup_object_property(&data, "pointer", &ty).unwrap()
    else {
        panic!()
    };
    assert!(std::ptr::eq(value, data.entries()[0].value()));
    let Value::Pointer(pointer) = &value.value else {
        panic!()
    };
    assert_eq!(
        vm.observe_pointer_scalar(pointer).unwrap(),
        Some(Value::Bool(true))
    );
    drop(storage);
    assert_eq!(vm.observe_pointer_scalar(pointer).unwrap(), None);
}

#[test]
fn frozen_object_round_trip_checks_domains_keys_and_phase() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let original = object(vec![
        Value::Bool(true),
        Value::Type(Term::Primitive(Primitive::Char)),
        object(vec![Value::Bool(false)]),
    ]);
    let term = original
        .clone()
        .into_normal_term(&mut Default::default())
        .unwrap();
    let domain = Term::Primitive(Primitive::Object);
    assert_eq!(vm.normal_pattern_value(&term, &domain).unwrap(), original);
    let bad = nir::objects::value(vec![nir::objects::entry(
        Some("bad"),
        Term::Primitive(Primitive::Bool),
        Term::Char('x'),
    )]);
    assert_eq!(
        vm.normal_pattern_value(&bad, &domain).unwrap_err().kind,
        ErrorKind::Type
    );
    assert_eq!(vm.normal_pattern_value(&term, &domain).unwrap(), original);
    vm.comptime_only = false;
    assert_eq!(
        vm.normal_pattern_value(&term, &domain).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
}

#[test]
fn freezing_objects_rejects_pointer_payloads_and_charges_keys() {
    let storage = Storage::new(Value::Bool(true));
    let pointer = Value::Pointer(storage.pointer(Term::Primitive(Primitive::Bool), vec![]));
    let value = Value::Object(Object::from_entries(vec![ObjectEntry::Value(
        ObjectValue {
            ty: Term::Pointer(Box::new(Term::Primitive(Primitive::Bool))),
            value: pointer,
        },
    )]));
    assert_eq!(
        value.into_normal_term(&mut Default::default()),
        Err(ErrorKind::Unsupported)
    );
    let value = Value::Object(Object::from_entries(vec![ObjectEntry::Property {
        key: "x".repeat(4096),
        value: ObjectValue {
            ty: Term::Primitive(Primitive::Bool),
            value: Value::Bool(true),
        },
    }]));
    let mut budget = comptime_runtime::Budget::default();
    budget.steps = 100;
    assert_eq!(
        value.into_normal_term(&mut budget),
        Err(ErrorKind::StepsExhausted)
    );
}
