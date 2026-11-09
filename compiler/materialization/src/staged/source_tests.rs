use super::*;
use std::sync::Arc;

fn source() -> n::Package {
    let boolean = Term::Primitive(nir::Primitive::Bool);
    n::Package {
        quote_templates: vec![n::Function {
            declaration: Id {
                package: 1,
                local: 1,
            },
            name: "retained".into(),
            phase: n::FunctionPhase::Runtime,
            inline: false,
            pure: true,
            is_unsafe: false,
            type_parameters: vec![],
            requirements: vec![n::TypePredicate::Bool(false)],
            capabilities: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            parameters: vec![],
            defaults: vec![],
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(boolean.clone()),
            },
            body: n::Expr {
                source: n::SourceRange { start: 0, end: 1 },
                ty: boolean,
                kind: K::Literal(n::Literal::Bool(true)),
            },
        }],
        ..Default::default()
    }
}

#[test]
fn retained_source_shares_ownership_without_promoting_templates() {
    let package = Arc::new(source());
    let weak = Arc::downgrade(&package);
    let owner = DeclarationSnapshot::new(package.clone());
    assert!(Snapshot::capture(DeclarationSource::Snapshot(&owner), &mut 0).is_err());
    let mut fuel = 1;
    let retained = Snapshot::capture(DeclarationSource::Snapshot(&owner), &mut fuel).unwrap();
    assert_eq!(fuel, 0);
    assert!(Arc::ptr_eq(retained.source.package(), &package));
    assert!(retained.source.package().functions.is_empty());
    assert_eq!(
        retained.source.package().quote_templates,
        package.quote_templates
    );
    drop(owner);
    drop(package);
    assert!(weak.upgrade().is_some());
    drop(retained);
    assert!(weak.upgrade().is_none());
}

#[test]
fn borrowed_and_closed_sources_capture_separate_declaration_worlds() {
    let mut package = source();
    let before = package.clone();
    let retained = Snapshot::capture(DeclarationSource::Borrowed(&package), &mut 10_000).unwrap();
    assert_eq!(retained.source.package().as_ref(), &package);
    assert!(!std::ptr::eq(retained.source.package().as_ref(), &package));
    assert_eq!(package, before);

    package.dispatch_instances.push(n::DispatchInstance {
        interface: Id {
            package: 1,
            local: 2,
        },
        interface_arguments: vec![],
        subject: Term::Primitive(nir::Primitive::Bool),
        implementation: Id {
            package: 1,
            local: 3,
        },
        arguments: vec![],
        methods: vec![],
        associated: vec![],
        associated_initializers: vec![],
    });
    let owner = DeclarationSnapshot::new(Arc::new(package));
    assert!(Snapshot::capture(DeclarationSource::Snapshot(&owner), &mut 1).is_err());
    let projected = Snapshot::capture(DeclarationSource::Snapshot(&owner), &mut 10_000).unwrap();
    assert!(!Arc::ptr_eq(projected.source.package(), owner.package()));
    assert!(projected.source.package().dispatch_instances.is_empty());
    assert_eq!(owner.package().dispatch_instances.len(), 1);
    assert_eq!(
        projected.source.package().quote_templates,
        owner.package().quote_templates
    );
    assert!(projected.source.package().functions.is_empty());
}

#[test]
fn borrowed_capture_meters_unrelated_declarations_without_penalizing_shared_owners() {
    let mut package = source();
    package.structs.push(n::Struct {
        declaration: Id {
            package: 1,
            local: 9,
        },
        name: "Metadata".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        fields: vec![("field".repeat(1000), Term::Primitive(nir::Primitive::Bool))],
    });
    let before = package.clone();
    let failure = Snapshot::capture(DeclarationSource::Borrowed(&package), &mut 100)
        .err()
        .expect("the copy includes unused declaration metadata");
    assert!(
        failure.contains("staged source copy budget exhausted"),
        "{failure}"
    );
    assert_eq!(package, before);
    let copied = Snapshot::capture(DeclarationSource::Borrowed(&package), &mut 10_000).unwrap();
    assert_eq!(copied.source.package().as_ref(), &package);
    let owner = DeclarationSnapshot::new(Arc::new(package));
    let retained = Snapshot::capture(DeclarationSource::Snapshot(&owner), &mut 1).unwrap();
    assert!(Arc::ptr_eq(owner.package(), retained.source.package()));
}
