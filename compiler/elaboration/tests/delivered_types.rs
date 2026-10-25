use elaboration::{definitions::publish, program::Protocols};
use hir::definitions::{DefinitionBatch, PublicationError};
use hir::*;
use nir::{Primitive, Term};
use rustc_span::DUMMY_SP;
use std::{collections::BTreeMap, sync::Arc};

fn description<'a>(
    arena: &'a HirArena,
    owner: OwnerId,
    ty: Term,
    default: Option<LitKind>,
) -> OwnerInfo<'a> {
    let id = |local| HirId::new(owner, ItemLocalId::new(local));
    let ty = arena.alloc_expr(Expr {
        hir_id: id(2),
        kind: ExprKind::DeliveredType(Arc::new(ty)),
        span: DUMMY_SP,
    });
    let default = default.map(|kind| {
        arena.alloc_expr(Expr {
            hir_id: id(3),
            kind: ExprKind::Lit(Lit {
                kind,
                span: DUMMY_SP,
            }),
            span: DUMMY_SP,
        })
    });
    OwnerInfo {
        node: OwnerNode::Item(arena.alloc_item(Item {
            owner_id: owner,
            ident: Ident::new(Symbol::intern("Generated"), DUMMY_SP),
            kind: ItemKind::Struct(StructDef {
                fields: arena.alloc_field_def_slice(vec![FieldDef {
                    hir_id: id(1),
                    ident: Ident::new(Symbol::intern("value"), DUMMY_SP),
                    ty,
                    default,
                    span: DUMMY_SP,
                }]),
                clause_params: &[],
                clause_constraints: &[],
                nested_items: vec![],
            }),
            span: DUMMY_SP,
        })),
        nodes: OwnerNodes::new(),
    }
}
fn identities() -> BTreeMap<u32, u64> {
    BTreeMap::from([(0, 73)])
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}

#[test]
fn delivered_types_keep_default_checking_and_survive_later_publication() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(
            description(&arena, owner, boolean(), Some(LitKind::Char('x'))),
            None,
            None,
        )
        .unwrap();
    let before = batch.to_string();
    assert!(matches!(
        publish(
            &mut batch,
            &mut 100,
            &identities(),
            &Protocols::default(),
            None
        ),
        Err(PublicationError::Check(_))
    ));
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(
            description(&arena, owner, boolean(), Some(LitKind::Bool(true))),
            None,
            None,
        )
        .unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(checked.structs[0].fields[0].1, boolean());
    let nominal = Term::Nominal {
        declaration: checked.structs[0].declaration,
        arguments: vec![],
    };
    drop(batch);
    let mut next = DefinitionBatch::new(&mut package);
    let owner = next.reserve().unwrap();
    next.define(
        description(&arena, owner, nominal.clone(), None),
        None,
        None,
    )
    .unwrap();
    let (_, checked) = publish(
        &mut next,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(checked.structs.len(), 2);
    assert_eq!(checked.structs[1].fields[0].1, nominal);
}

#[test]
fn malformed_or_foreign_type_inputs_reject_atomically_and_can_be_repaired() {
    let bad = [
        Term::Bool(true),
        Term::Hole {
            index: 0,
            sort: nir::Sort::Type,
        },
        Term::Rigid {
            declaration: nir::DeclarationId {
                package: 73,
                local: 999,
            },
            sort: nir::Sort::Type,
        },
        Term::Nominal {
            declaration: nir::DeclarationId {
                package: 99,
                local: 999,
            },
            arguments: vec![],
        },
        Term::NormalForm {
            head: nir::DeclarationId {
                package: 99,
                local: 999,
            },
            sort: nir::Sort::Type,
            arguments: vec![],
        },
    ];
    for ty in bad {
        let arena = HirArena::new();
        let mut package = Package::new();
        let mut batch = DefinitionBatch::new(&mut package);
        let owner = batch.reserve().unwrap();
        batch
            .define(description(&arena, owner, ty.clone(), None), None, None)
            .unwrap();
        let before = batch.to_string();
        assert!(
            publish(
                &mut batch,
                &mut 100,
                &identities(),
                &Protocols::default(),
                None
            )
            .is_err(),
            "accepted {ty}"
        );
        assert_eq!(batch.published().num_defs(), 0);
        assert_eq!(batch.to_string(), before);
        batch
            .define(description(&arena, owner, boolean(), None), None, None)
            .unwrap();
        publish(
            &mut batch,
            &mut 100,
            &identities(),
            &Protocols::default(),
            None,
        )
        .unwrap();
    }
}

#[test]
fn reflected_type_values_can_feed_generated_fields_without_source_reconstruction() {
    let mut source = nir::program::Package::default();
    let declaration = nir::DeclarationId {
        package: 12,
        local: 0,
    };
    source.structs.push(nir::program::Struct {
        declaration,
        name: "Source".into(),
        fields: vec![("flag".into(), boolean())],
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
    });
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        &source,
        comptime_interpretor::Budget::default(),
    )
    .unwrap();
    let reflected = vm
        .reflect_struct(&Term::Nominal {
            declaration,
            arguments: vec![],
        })
        .unwrap();
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(
            description(&arena, owner, reflected.fields[0].ty.clone(), None),
            None,
            None,
        )
        .unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(checked.structs[0].fields[0].1, reflected.fields[0].ty);
}

#[test]
fn delivered_normal_form_preserves_head_and_rejects_body_type_default() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let head = batch.reserve().unwrap();
    let owner = batch.reserve().unwrap();
    let id = |local| HirId::new(head, ItemLocalId::new(local));
    let result = arena.alloc_expr(Expr {
        hir_id: id(1),
        kind: ExprKind::TyType,
        span: DUMMY_SP,
    });
    let value = arena.alloc_expr(Expr {
        hir_id: id(2),
        kind: ExprKind::Builtin(Symbol::intern("bool")),
        span: DUMMY_SP,
    });
    let body = BodyId::new(id(2));
    batch.body(body, Body { params: &[], value }).unwrap();
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(Item {
                    owner_id: head,
                    ident: Ident::new(Symbol::intern("Token"), DUMMY_SP),
                    span: DUMMY_SP,
                    kind: ItemKind::Fn(
                        FnSig {
                            params: &[],
                            return_ty: Some(result),
                            return_bind: None,
                            return_binding: None,
                            modifiers: hir::item::FnModifiers {
                                is_pure: true,
                                is_comptime: true,
                                is_normal_form_head: true,
                                ..Default::default()
                            },
                            clause_params: &[],
                            clause_constraints: &[],
                            span: DUMMY_SP,
                        },
                        body,
                    ),
                })),
                nodes: OwnerNodes::new(),
            },
            None,
            None,
        )
        .unwrap();
    let opaque = Term::NormalForm {
        head: nir::DeclarationId {
            package: 73,
            local: head.def_id.raw(),
        },
        sort: nir::Sort::Type,
        arguments: vec![],
    };
    batch
        .define(
            description(&arena, owner, opaque.clone(), Some(LitKind::Bool(true))),
            None,
            None,
        )
        .unwrap();
    assert!(
        publish(
            &mut batch,
            &mut 100,
            &identities(),
            &Protocols::default(),
            None
        )
        .is_err()
    );
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(description(&arena, owner, opaque.clone(), None), None, None)
        .unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(checked.structs[0].fields[0].1, opaque);
    assert_ne!(checked.structs[0].fields[0].1, boolean());
}

#[test]
fn destination_formation_predicates_are_checked_before_delivery() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let constrained = batch.reserve().unwrap();
    let owner = batch.reserve().unwrap();
    let predicate = arena.alloc_expr(Expr {
        hir_id: HirId::new(constrained, ItemLocalId::new(1)),
        kind: ExprKind::Lit(Lit {
            kind: LitKind::Bool(false),
            span: DUMMY_SP,
        }),
        span: DUMMY_SP,
    });
    let constraints = [ClauseConstraint {
        label: None,
        hir_id: HirId::new(constrained, ItemLocalId::new(2)),
        kind: ClauseConstraintKind::Requires(predicate),
        span: DUMMY_SP,
    }];
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(Item {
                    owner_id: constrained,
                    ident: Ident::new(Symbol::intern("Unavailable"), DUMMY_SP),
                    span: DUMMY_SP,
                    kind: ItemKind::Struct(StructDef {
                        fields: &[],
                        clause_params: &[],
                        clause_constraints: &constraints,
                        nested_items: vec![],
                    }),
                })),
                nodes: OwnerNodes::new(),
            },
            None,
            None,
        )
        .unwrap();
    let ty = Term::Nominal {
        declaration: nir::DeclarationId {
            package: 73,
            local: constrained.def_id.raw(),
        },
        arguments: vec![],
    };
    batch
        .define(description(&arena, owner, ty, None), None, None)
        .unwrap();
    let failure = publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap_err();
    assert!(
        failure
            .to_string()
            .contains("delivered type admission failed"),
        "{failure}"
    );
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(description(&arena, owner, boolean(), None), None, None)
        .unwrap();
    publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
}
