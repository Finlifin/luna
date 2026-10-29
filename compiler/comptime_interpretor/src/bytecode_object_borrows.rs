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
    assert!(code.to_string().contains("(local-object-query "));
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
