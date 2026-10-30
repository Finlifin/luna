//! Borrowed receiver queries must release locks before aliased selector reads.
use super::*;
use nir::program::ObjectQueryKind as Q;

#[test]
fn local_object_queries_handle_sibling_references_and_locked_receivers() {
    let source = SourceRange { start: 2, end: 7 };
    let receiver = LocalId { owner: 1, index: 0 };
    let selector = LocalId { owner: 1, index: 1 };
    let boolean = Term::Primitive(Primitive::Bool);
    let object = Value::Object(Object::from_entries(vec![ObjectEntry::Property {
        key: "flag".into(),
        value: ObjectValue {
            ty: boolean.clone(),
            value: Value::Bool(true),
        },
    }]));
    let storage = Storage::new(Value::AnonymousRecord(vec![
        ("object".into(), object),
        (
            "index".into(),
            Value::Integer(comptime_runtime::Integer::from_u128(0)),
        ),
    ]));
    let mut environment = Environment::new();
    for (id, index) in [(receiver, 0), (selector, 1)] {
        let reference = value_reference::ValueReference::projected(
            storage.clone(),
            &[(nir::program::FieldOwner::Anonymous, index)],
            &mut 1000,
        )
        .unwrap();
        environment.insert(id, Value::ComptimeReference(reference));
    }
    let fields = vec![
        Expr {
            source,
            ty: Term::Primitive(Primitive::Object),
            kind: K::Local(receiver),
        },
        Expr {
            source,
            ty: Term::Primitive(Primitive::Integer),
            kind: K::Local(selector),
        },
    ];
    let body = Expr {
        source,
        ty: Term::Optional(Box::new(boolean)),
        kind: K::ObjectQuery {
            query: Q::Value,
            subject: Box::new(Expr {
                source,
                ty: Term::Tuple(fields.iter().map(|v| v.ty.clone()).collect()),
                kind: K::Tuple(fields),
            }),
        },
    };
    let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
    assert!(code.to_string().contains("(local-object-access "));
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let expected = Value::Optional {
        ty: body.ty.clone(),
        value: Some(Box::new(Value::Bool(true))),
    };
    let Flow::Value(actual) = vm.execute_code(&code, &mut environment).unwrap() else {
        panic!("flow")
    };
    assert_eq!(actual, expected);
    // Owned selectors permit borrowing, but never bypass an invalid receiver lock.
    environment.insert(
        selector,
        Value::Integer(comptime_runtime::Integer::from_u128(0)),
    );
    let locked = storage.0.lock().unwrap();
    assert_eq!(
        vm.execute_code(&code, &mut environment).unwrap_err().kind,
        ErrorKind::InvalidValueReference
    );
    drop(locked);
    let Flow::Value(actual) = vm.execute_code(&code, &mut environment).unwrap() else {
        panic!("flow")
    };
    assert_eq!(actual, expected);
}

#[test]
fn local_object_slices_keep_snapshots_and_release_locks_for_sibling_bounds() {
    let source = SourceRange { start: 4, end: 8 };
    let local = |index, ty| Expr {
        source,
        ty: Term::Primitive(ty),
        kind: K::Local(LocalId { owner: 1, index }),
    };
    let fields = vec![
        local(0, Primitive::Object),
        local(1, Primitive::Integer),
        local(2, Primitive::Integer),
    ];
    let body = Expr {
        source,
        ty: Term::Primitive(Primitive::Object),
        kind: K::ObjectEdit {
            operation: nir::program::ObjectEditKind::Slice,
            subject: Box::new(Expr {
                source,
                ty: Term::Tuple(fields.iter().map(|v| v.ty.clone()).collect()),
                kind: K::Tuple(fields),
            }),
        },
    };
    let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
    assert!(code.to_string().contains("(local-object-access "));
    let original = Value::Object(Object::from_entries(vec![
        ObjectEntry::Property {
            key: "same".into(),
            value: ObjectValue {
                ty: Term::Primitive(Primitive::Bool),
                value: Value::Bool(true),
            },
        },
        ObjectEntry::Property {
            key: "same".into(),
            value: ObjectValue {
                ty: Term::Primitive(Primitive::Char),
                value: Value::Char('x'),
            },
        },
    ]));
    let integer = |n| Value::Integer(comptime_runtime::Integer::from_u128(n));
    let expected = Value::Object(Object::from_entries(vec![ObjectEntry::Property {
        key: "same".into(),
        value: ObjectValue {
            ty: Term::Primitive(Primitive::Char),
            value: Value::Char('x'),
        },
    }]));
    let package = Package::default();
    for borrowed in [false, true] {
        let storage = Storage::new(Value::AnonymousRecord(vec![
            ("object".into(), original.clone()),
            ("start".into(), integer(1)),
            ("end".into(), integer(2)),
        ]));
        let mut env = Environment::new();
        for index in 0..3 {
            let value = if index == 0 || !borrowed {
                Value::ComptimeReference(
                    value_reference::ValueReference::projected(
                        storage.clone(),
                        &[(nir::program::FieldOwner::Anonymous, index as usize)],
                        &mut 1000,
                    )
                    .unwrap(),
                )
            } else {
                integer(index as u128)
            };
            env.insert(LocalId { owner: 1, index }, value);
        }
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let Flow::Value(value) = vm.execute_code(&code, &mut env).unwrap() else {
            panic!("value")
        };
        assert_eq!(value, expected);
        assert_eq!(
            vm.local_value(&env, LocalId { owner: 1, index: 0 }, Some(source))
                .unwrap(),
            original
        );
        let lock = storage.0.lock().unwrap();
        assert_eq!(
            vm.execute_code(&code, &mut env).unwrap_err().kind,
            ErrorKind::InvalidValueReference
        );
        drop(lock);
        env.insert(LocalId { owner: 1, index: 2 }, integer(3));
        assert_eq!(
            vm.execute_code(&code, &mut env).unwrap_err().kind,
            ErrorKind::IndexOutOfBounds
        );
        env.insert(LocalId { owner: 1, index: 2 }, integer(2));
        assert!(
            matches!(vm.execute_code(&code,&mut env).unwrap(),Flow::Value(value) if value==expected)
        );
    }
    // Even an empty selection must reject an invalid unselected Type payload.
    let invalid = Value::Object(Object::from_entries(vec![ObjectEntry::Value(
        ObjectValue {
            ty: Term::Primitive(Primitive::Type),
            value: Value::Type(Term::Bound {
                depth: 0,
                index: 0,
                sort: nir::Sort::Type,
            }),
        },
    )]));
    let mut env = Environment::from([
        (LocalId { owner: 1, index: 0 }, invalid),
        (LocalId { owner: 1, index: 1 }, integer(0)),
        (LocalId { owner: 1, index: 2 }, integer(0)),
    ]);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let compiled_error = vm.execute_code(&code, &mut env).unwrap_err();
    let tree_error = vm.eval(&body, &mut env).unwrap_err();
    assert_eq!(compiled_error.kind, tree_error.kind);
    env.insert(LocalId { owner: 1, index: 0 }, original);
    vm.budget.steps = 1;
    assert_eq!(
        vm.execute_code(&code, &mut env).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    vm.budget.steps = 10000;
    let Flow::Value(Value::Object(empty)) = vm.execute_code(&code, &mut env).unwrap() else {
        panic!("Object")
    };
    assert!(empty.entries().is_empty());
}
