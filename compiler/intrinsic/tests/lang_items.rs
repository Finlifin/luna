use intrinsic::{LangItem, LangItemDef, LangItems};
use nir::DeclarationId;
fn definition(package: u64, local: u32) -> LangItemDef {
    LangItemDef::UserDef(DeclarationId { package, local })
}
#[test]
fn registration_preserves_package_identity_and_rejects_conflicts() {
    let mut items = LangItems::new();
    items.set(LangItem::Copy, definition(1, 7)).unwrap();
    items.set(LangItem::Copy, definition(1, 7)).unwrap();
    let before = items.to_string();
    let conflict = items.set(LangItem::Copy, definition(2, 7)).unwrap_err();
    assert_eq!(conflict.previous, definition(1, 7));
    assert_eq!(conflict.attempted, definition(2, 7));
    assert_eq!(items.to_string(), before);
    assert_eq!(items.len(), 1);
    assert!(items.set(LangItem::Copy, LangItemDef::Builtin).is_err());
    assert_eq!(items.get(LangItem::Copy), Some(definition(1, 7)));
}
#[test]
fn registry_order_is_stable_and_resource_items_remain_unbound_until_registered() {
    let mut forward = LangItems::new();
    let mut reverse = LangItems::new();
    let bindings = [
        (LangItem::Once, definition(1, 1)),
        (LangItem::Clone, definition(1, 2)),
        (LangItem::Copy, definition(1, 3)),
        (LangItem::Drop, definition(1, 4)),
        (LangItem::U8, LangItemDef::Builtin),
    ];
    assert_eq!(forward.get(LangItem::Once), None);
    for (item, def) in bindings {
        forward.set(item, def).unwrap();
    }
    for (item, def) in bindings.into_iter().rev() {
        reverse.set(item, def).unwrap();
    }
    assert_eq!(forward.to_string(), reverse.to_string());
    assert_eq!(
        forward.iter().map(|(i, _)| i.name()).collect::<Vec<_>>(),
        vec!["Clone", "Copy", "Drop", "Once", "u8"]
    );
    assert_eq!(forward.get(LangItem::Drop), Some(definition(1, 4)));
}

fn trait_package(ids: &[DeclarationId]) -> nir::program::Package {
    nir::program::Package {
        traits: ids
            .iter()
            .map(|&declaration| nir::program::Trait {
                parameter_types: Vec::new(),
                associated: vec![],
                declaration,
                self_parameter: DeclarationId {
                    local: declaration.local + 1000,
                    ..declaration
                },
                name: "UnrelatedSpelling".into(),
                type_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                parents: vec![],
                methods: vec![],
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn source_binding_is_atomic_and_package_qualified() {
    let first = DeclarationId {
        package: 7,
        local: 1,
    };
    let second = DeclarationId {
        package: 8,
        local: 1,
    };
    let missing = DeclarationId {
        package: 9,
        local: 1,
    };
    let package = trait_package(&[first, second]);
    let mut items = LangItems::new();
    items.set(LangItem::U8, LangItemDef::Builtin).unwrap();
    let before = items.to_string();
    assert!(
        items
            .bind_source_traits(
                &package,
                &[(LangItem::Once, first), (LangItem::Copy, missing)]
            )
            .is_err()
    );
    assert_eq!(items.to_string(), before);
    items
        .bind_source_traits(
            &package,
            &[(LangItem::Once, first), (LangItem::Copy, second)],
        )
        .unwrap();
    let before = items.to_string();
    // A later conflict must also roll back an earlier valid addition.
    assert!(
        items
            .bind_source_traits(
                &package,
                &[(LangItem::Clone, first), (LangItem::Copy, first)]
            )
            .is_err()
    );
    assert_eq!(items.to_string(), before);
    items
        .bind_source_traits(&package, &[(LangItem::Copy, second)])
        .unwrap();
    assert_eq!(
        items.get(LangItem::Copy),
        Some(LangItemDef::UserDef(second))
    );
}

#[test]
fn malformed_source_targets_do_not_register() {
    let id = DeclarationId {
        package: 7,
        local: 1,
    };
    let mut items = LangItems::new();
    for package in [trait_package(&[]), trait_package(&[id, id])] {
        assert!(
            items
                .bind_source_traits(&package, &[(LangItem::Copy, id)])
                .is_err()
        );
        assert!(items.is_empty());
    }
    assert!(
        items
            .bind_source_traits(&trait_package(&[id]), &[(LangItem::U8, id)])
            .is_err()
    );
    assert!(items.is_empty());
}

#[test]
fn splice_registration_is_explicit_atomic_and_rejects_foreign_identities() {
    let id = DeclarationId {
        package: 42,
        local: 7,
    };
    let foreign = DeclarationId {
        package: 43,
        local: 7,
    };
    let package = trait_package(&[id]);
    let mut items = LangItems::new();
    assert!(!items.has(LangItem::Splice));
    assert!(
        items
            .bind_source_traits(
                &package,
                &[(LangItem::Splice, id), (LangItem::Copy, foreign),]
            )
            .is_err()
    );
    assert!(items.is_empty());
    items
        .bind_source_traits(&package, &[(LangItem::Splice, id)])
        .unwrap();
    assert_eq!(items.get(LangItem::Splice), Some(LangItemDef::UserDef(id)));
    assert_eq!(LangItem::Splice.name(), "Splice");
    let before = items.to_string();
    assert!(
        items
            .bind_source_traits(&trait_package(&[foreign]), &[(LangItem::Splice, foreign)])
            .is_err()
    );
    assert_eq!(items.to_string(), before);
}

#[test]
fn tuple_head_registration_requires_explicit_package_qualified_identity() {
    let first = DeclarationId {
        package: 1,
        local: 8,
    };
    let second = DeclarationId {
        package: 2,
        local: 8,
    };
    let make = |declaration| nir::normal_forms::Head {
        role: nir::normal_forms::Role::Opaque,
        declaration,
        parameters: vec![],
        requirements: vec![],
    };
    let package = nir::program::Package {
        normal_forms: vec![make(first), make(second)],
        ..Default::default()
    };
    let mut items = LangItems::new();
    assert!(items.get(LangItem::Tuple).is_none());
    items
        .bind_source_normal_form_heads(&package, &[(LangItem::Tuple, first)])
        .unwrap();
    items
        .bind_source_normal_form_heads(&package, &[(LangItem::Tuple, first)])
        .unwrap();
    let before = items.to_string();
    assert!(
        items
            .bind_source_normal_form_heads(&package, &[(LangItem::Tuple, second)])
            .is_err()
    );
    assert_eq!(items.to_string(), before);
    assert_eq!(
        items.get(LangItem::Tuple),
        Some(LangItemDef::UserDef(first))
    );
    assert!(
        items
            .bind_source_traits(&package, &[(LangItem::Tuple, first)])
            .is_err()
    );
}

#[test]
fn normal_form_registration_rolls_back_missing_duplicate_and_wrong_kind_heads() {
    let id = DeclarationId {
        package: 1,
        local: 8,
    };
    let metadata = nir::normal_forms::Head {
        role: nir::normal_forms::Role::Opaque,
        declaration: id,
        parameters: vec![],
        requirements: vec![],
    };
    let package = nir::program::Package {
        normal_forms: vec![metadata.clone()],
        ..Default::default()
    };
    let mut items = LangItems::new();
    let before = items.to_string();
    assert!(
        items
            .bind_source_normal_form_heads(&package, &[(LangItem::Tuple, id), (LangItem::Copy, id)])
            .is_err()
    );
    assert_eq!(items.to_string(), before);
    assert!(
        items
            .bind_source_normal_form_heads(
                &nir::program::Package::default(),
                &[(LangItem::Tuple, id)]
            )
            .is_err()
    );
    let duplicate = nir::program::Package {
        normal_forms: vec![metadata.clone(), metadata],
        ..Default::default()
    };
    assert!(
        items
            .bind_source_normal_form_heads(&duplicate, &[(LangItem::Tuple, id)])
            .is_err()
    );
    assert_eq!(items.to_string(), before);
}

#[test]
fn a_function_named_tuple_does_not_supply_a_pure_normal_form_head() {
    use nir::program::{Expr, ExprKind, Function, FunctionPhase, Package, SourceRange};
    use nir::{Primitive, Term};
    let id = DeclarationId {
        package: 3,
        local: 9,
    };
    let function = Function {
        declaration: id,
        name: "Tuple".into(),
        phase: FunctionPhase::Runtime,
        inline: false,
        pure: false,
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        parameters: vec![],
        defaults: vec![],
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Type)),
        },
        body: Expr {
            ty: Term::Primitive(Primitive::Type),
            kind: ExprKind::TypeValue(Term::Primitive(Primitive::Bool)),
            source: SourceRange { start: 0, end: 0 },
        },
    };
    let mut package = Package {
        functions: vec![function],
        ..Default::default()
    };
    let mut items = LangItems::new();
    assert!(
        items
            .bind_source_normal_form_heads(&package, &[(LangItem::Tuple, id)])
            .is_err()
    );
    package.normal_forms.push(nir::normal_forms::Head {
        role: nir::normal_forms::Role::Opaque,
        declaration: id,
        parameters: vec![],
        requirements: vec![],
    });
    for (phase, pure) in [
        (FunctionPhase::Runtime, true),
        (FunctionPhase::NormalForm, false),
    ] {
        package.functions[0].phase = phase;
        package.functions[0].pure = pure;
        assert!(
            items
                .bind_source_normal_form_heads(&package, &[(LangItem::Tuple, id)])
                .is_err()
        );
        assert!(items.get(LangItem::Tuple).is_none());
    }
    package.functions[0].phase = FunctionPhase::NormalForm;
    package.functions[0].pure = true;
    items
        .bind_source_normal_form_heads(&package, &[(LangItem::Tuple, id)])
        .unwrap();
    let before = items.to_string();
    package.functions.push(package.functions[0].clone());
    assert!(
        items
            .bind_source_normal_form_heads(&package, &[(LangItem::Tuple, id)])
            .is_err()
    );
    assert_eq!(items.to_string(), before);
}
