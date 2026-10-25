use hir::{
    definitions::{DefinitionBatch, PublicationError},
    *,
};
use rustc_span::DUMMY_SP;
use std::sync::Arc;

#[test]
fn candidate_metadata_copy_is_budgeted_before_semantic_callback_and_can_retry() {
    let arena = HirArena::new();
    for case in 0..4 {
        let mut package = Package::new();
        let owner = package.alloc_owner_id();
        let item = arena.alloc_item(Item {
            owner_id: owner,
            ident: Ident::new(Symbol::intern("base"), DUMMY_SP),
            kind: ItemKind::Mod(ModDef { items: vec![] }),
            span: DUMMY_SP,
        });
        let mut nodes = OwnerNodes::new();
        if case == 0 {
            nodes.nodes.ensure_contains(ItemLocalId::new(1024));
        }
        package.insert_owner(
            owner,
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes,
            },
        );
        let value = arena.alloc_expr(Expr {
            hir_id: HirId::new(owner, ItemLocalId::new(1)),
            kind: ExprKind::TyVoid,
            span: DUMMY_SP,
        });
        if case == 1 {
            package.pending_semantics.push((DUMMY_SP, "x".repeat(1024)));
        }
        if case >= 2 {
            package
                .definition_attributes
                .push(DeferredDefinitionAttribute {
                    target: AttributeTarget::Definition(owner),
                    lexical_owner: owner,
                    expression: if case == 2 {
                        "x".repeat(1024)
                    } else {
                        "x".into()
                    },
                    configuration_key: (case == 3).then(|| "k".repeat(1024)),
                    value,
                    span: DUMMY_SP,
                });
        }
        let mut batch = DefinitionBatch::new(&mut package);
        let mut called = false;
        let result = batch.publish_checked(&mut 20, |_| {
            called = true;
            Ok::<_, ()>(())
        });
        assert!(
            matches!(result, Err(PublicationError::Structure(_))),
            "case {case}"
        );
        assert!(!called, "case {case}");
        assert!(std::ptr::eq(batch.published().item(owner).unwrap(), item));
        batch
            .publish_checked(&mut 10_000, |candidate| {
                assert!(std::ptr::eq(candidate.item(owner).unwrap(), item));
                assert_eq!(
                    candidate.owner(owner).unwrap().nodes.len(),
                    if case == 0 { 1025 } else { 0 }
                );
                Ok::<_, ()>(())
            })
            .unwrap();
    }
}

#[test]
fn shared_member_candidate_payloads_are_not_charged_as_deep_copies() {
    let mut package = Package::new();
    let id = HirId::new(OwnerId::new(LocalDefId::new(0)), ItemLocalId::new(1));
    let pair = (symbol::DefId::new(0, 1), symbol::DefId::new(0, 2));
    let shared: Arc<[(symbol::DefId, symbol::DefId)]> = vec![pair; 4096].into();
    package.member_access.insert(id, shared.clone());
    package
        .deferred_member_projections
        .insert(id, shared.clone());
    let mut batch = DefinitionBatch::new(&mut package);
    batch
        .publish_checked(&mut 100, |candidate| {
            assert!(Arc::ptr_eq(&candidate.member_access[&id], &shared));
            assert!(Arc::ptr_eq(
                &candidate.deferred_member_projections[&id],
                &shared
            ));
            Ok::<_, ()>(())
        })
        .unwrap();
}

#[test]
fn draft_attribute_text_is_charged_and_failure_preserves_editable_batch() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: Ident::new(Symbol::intern("draft"), DUMMY_SP),
        kind: ItemKind::Mod(ModDef { items: vec![] }),
        span: DUMMY_SP,
    });
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes: OwnerNodes::new(),
            },
            None,
            None,
        )
        .unwrap();
    let value = arena.alloc_expr(Expr {
        hir_id: HirId::new(owner, ItemLocalId::new(1)),
        kind: ExprKind::TyVoid,
        span: DUMMY_SP,
    });
    batch.set_attributes(vec![DeferredDefinitionAttribute {
        target: AttributeTarget::Definition(owner),
        lexical_owner: owner,
        expression: "a".repeat(1024),
        value,
        configuration_key: None,
        span: DUMMY_SP,
    }]);
    let before = batch.to_string();
    let mut called = false;
    assert!(matches!(
        batch.publish_checked(&mut 20, |_| {
            called = true;
            Ok::<_, ()>(())
        }),
        Err(PublicationError::Structure(_))
    ));
    assert!(!called);
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .publish_checked(&mut 10_000, |candidate| {
            assert_eq!(candidate.definition_attributes[0].expression.len(), 1024);
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(batch.published().num_defs(), 1);
}

#[test]
fn sparse_hash_table_capacity_is_charged_even_when_empty() {
    let mut package = Package::new();
    package.member_projections.reserve(1024);
    let mut batch = DefinitionBatch::new(&mut package);
    let mut called = false;
    assert!(matches!(
        batch.publish_checked(&mut 20, |_| {
            called = true;
            Ok::<_, ()>(())
        }),
        Err(PublicationError::Structure(_))
    ));
    assert!(!called);
    assert!(batch.published().member_projections.is_empty());
    batch
        .publish_checked(&mut 10_000, |_| Ok::<_, ()>(()))
        .unwrap();
}
