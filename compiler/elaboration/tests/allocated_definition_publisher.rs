use comptime_interpretor::{
    EnumBuildRequest, ErrorKind, Interpreter, ReflectedField, StructBuildRequest,
};
use elaboration::definitions::AllocatedDefinitionPublisher;
use nir::{
    DeclarationId, Primitive, Term,
    program::{Package, SourceRange, Variant, VariantShape},
};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 75, local }
}
fn ty(local: u32) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![],
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn source() -> SourceRange {
    SourceRange { start: 12, end: 25 }
}
fn structure(field: Term) -> StructBuildRequest {
    StructBuildRequest {
        name: "Generated".into(),
        fields: vec![ReflectedField {
            name: "value".into(),
            ty: field,
        }],
        source: source(),
    }
}
fn enumeration(field: Term) -> EnumBuildRequest {
    EnumBuildRequest {
        encoding: None,
        symbol_encoding: None,
        name: "GeneratedEnum".into(),
        groups: vec!["Group".into()],
        source: source(),
        variants: vec![
            Variant {
                name: "Empty".into(),
                shape: VariantShape::Unit,
                fields: vec![],
            },
            Variant {
                name: "Positional".into(),
                shape: VariantShape::Tuple,
                fields: vec![boolean()],
            },
            Variant {
                name: "Group.Data".into(),
                shape: VariantShape::Named(vec!["value".into()]),
                fields: vec![field],
            },
        ],
    }
}

#[test]
fn alternating_requests_allocate_once_and_retain_shapes_and_payload_identities() {
    let baseline = Package::default();
    let publisher = AllocatedDefinitionPublisher::new(&baseline, 75, 100, 100_000);
    {
        let mut steps = 1_000_000;
        let mut vm = Interpreter::for_comptime(&baseline, Default::default()).unwrap();
        let first = publisher
            .prepare_struct(&structure(boolean()), &mut steps)
            .unwrap();
        assert_eq!(first.result, id(100));
        vm.install_owned_generated_structs_with(first.definitions, first.commit)
            .unwrap();
        let second = publisher
            .prepare_enum(&enumeration(ty(100)), &mut steps)
            .unwrap();
        assert_eq!(second.result, id(101));
        vm.install_owned_generated_enums_with(second.definitions, second.commit)
            .unwrap();
        let third = publisher
            .prepare_struct(&structure(ty(101)), &mut steps)
            .unwrap();
        assert_eq!(third.result, id(102));
        vm.install_owned_generated_structs_with(third.definitions, third.commit)
            .unwrap();
        assert_eq!(
            vm.reflect_enum(&ty(101)).unwrap().variants[2].fields,
            [ty(100)]
        );
        assert!(publisher.to_string().contains("(next 103)"));
    }
    let (definitions, next) = publisher.finish();
    assert_eq!(next, 103);
    assert_eq!(definitions.structs.len(), 2);
    assert_eq!(definitions.enums.len(), 1);
    let generated = &definitions.enums[0][0];
    assert_eq!(generated.groups, ["Group"]);
    assert_eq!(generated.variants[0].shape, VariantShape::Unit);
    assert_eq!(generated.variants[1].shape, VariantShape::Tuple);
    assert_eq!(
        generated.variants[2].shape,
        VariantShape::Named(vec!["value".into()])
    );
}

#[test]
fn abandoned_rejected_and_stale_candidates_never_reuse_reserved_identities() {
    let baseline = Package::default();
    let publisher = AllocatedDefinitionPublisher::new(&baseline, 75, 100, 100_000);
    {
        let mut steps = 1_000_000;
        let discarded = publisher
            .prepare_enum(&enumeration(boolean()), &mut steps)
            .unwrap();
        assert_eq!(discarded.result, id(100));
        drop(discarded);
        let rejected = publisher
            .prepare_enum(&enumeration(ty(999)), &mut steps)
            .err()
            .unwrap();
        assert_eq!(rejected.source, Some(source()));
        let stale = publisher
            .prepare_enum(&enumeration(boolean()), &mut steps)
            .unwrap();
        assert_eq!(stale.result, id(102));
        let current = publisher
            .prepare_struct(&structure(boolean()), &mut steps)
            .unwrap();
        assert_eq!(current.result, id(103));
        (current.commit)().unwrap();
        assert_eq!(
            (stale.commit)().unwrap_err().kind,
            ErrorKind::InvalidCallPlan
        );
        assert!(publisher.last_diagnostic().unwrap().contains("stale"));
        let retry = publisher
            .prepare_enum(&enumeration(ty(103)), &mut steps)
            .unwrap();
        assert_eq!(retry.result, id(104));
        (retry.commit)().unwrap();
    }
    let (definitions, next) = publisher.finish();
    assert_eq!(next, 105);
    assert_eq!(definitions.enums[0][0].declaration, id(104));
    assert_eq!(definitions.structs[0][0].declaration, id(103));
}

#[test]
fn assembly_and_identity_exhaustion_are_distinct_from_invocation_exhaustion() {
    let baseline = Package::default();
    let request = enumeration(Term::Tuple(vec![boolean(); 100]));
    for fuel in [0, 1, 10, 100] {
        let publisher = AllocatedDefinitionPublisher::new(&baseline, 75, 100, fuel);
        let mut steps = 1_000_000;
        let error = publisher.prepare_enum(&request, &mut steps).err().unwrap();
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(error.source, Some(source()));
        assert_eq!(steps, 1_000_000);
        assert!(
            publisher
                .last_diagnostic()
                .unwrap()
                .contains("structural budget")
        );
        let (definitions, next) = publisher.finish();
        assert_eq!(next, 100);
        assert!(definitions.enums.is_empty());
    }
    let publisher = AllocatedDefinitionPublisher::new(&baseline, 75, u32::MAX, 100_000);
    assert_eq!(
        publisher
            .prepare_enum(&enumeration(boolean()), &mut 1_000_000)
            .err()
            .unwrap()
            .kind,
        ErrorKind::InvalidCallPlan
    );
    assert!(
        publisher
            .last_diagnostic()
            .unwrap()
            .contains("identity space")
    );
    assert_eq!(publisher.finish().1, u32::MAX);
    let publisher = AllocatedDefinitionPublisher::new(&baseline, 75, 100, 100_000);
    assert_eq!(
        publisher
            .prepare_enum(&enumeration(boolean()), &mut 0)
            .err()
            .unwrap()
            .kind,
        ErrorKind::StepsExhausted
    );
    {
        let retry = publisher
            .prepare_enum(&enumeration(boolean()), &mut 1_000_000)
            .unwrap();
        assert_eq!(retry.result, id(101));
        (retry.commit)().unwrap();
    }
    assert_eq!(publisher.finish().1, 102);
}

#[test]
fn enum_request_dump_preserves_shapes_and_escapes_symbols() {
    let mut request = enumeration(boolean());
    request.name = "Quoted\"Enum".into();
    request.groups = vec!["a\nb".into()];
    request.variants[2].shape = VariantShape::Named(vec!["field\"name".into()]);
    let dump = request.to_string();
    assert!(dump.contains("(name \"Quoted\\\"Enum\")"), "{dump}");
    assert!(dump.contains("(groups \"a\\nb\")"), "{dump}");
    assert!(dump.contains("(shape unit)"));
    assert!(dump.contains("(shape tuple)"));
    assert!(dump.contains("(shape (named \"field\\\"name\"))"));
}
