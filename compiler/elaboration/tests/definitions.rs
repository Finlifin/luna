use elaboration::definitions::publish;
use elaboration::program::Protocols;
use hir::definitions::{DefinitionBatch, PublicationError};
use hir::*;
use nir::target::TargetData;
use rustc_span::DUMMY_SP;
use std::collections::BTreeMap;

#[test]
fn semantic_rejection_preserves_draft_then_publishes_checked_function() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let id = |local| HirId::new(owner, ItemLocalId::new(local));
    let invalid_type = arena.alloc_expr(Expr {
        hir_id: id(1),
        kind: ExprKind::Lit(Lit {
            kind: LitKind::Bool(true),
            span: DUMMY_SP,
        }),
        span: DUMMY_SP,
    });
    let valid_type = arena.alloc_expr(Expr {
        hir_id: id(1),
        kind: ExprKind::TyVoid,
        span: DUMMY_SP,
    });
    let block = Block {
        hir_id: id(3),
        stmts: &[],
        expr: None,
        span: DUMMY_SP,
    };
    let value = Expr {
        hir_id: id(2),
        kind: ExprKind::Block(&block),
        span: DUMMY_SP,
    };
    let body_id = BodyId::new(id(2));
    batch
        .body(
            body_id,
            Body {
                params: &[],
                value: &value,
            },
        )
        .unwrap();
    let define = |return_ty| OwnerInfo {
        node: OwnerNode::Item(arena.alloc_item(Item {
            owner_id: owner,
            ident: Ident::new(Symbol::intern("generated"), DUMMY_SP),
            kind: ItemKind::Fn(
                FnSig {
                    params: &[],
                    return_ty: Some(return_ty),
                    return_bind: None,
                    return_binding: None,
                    modifiers: hir::item::FnModifiers::default(),
                    clause_params: &[],
                    clause_constraints: &[],
                    span: DUMMY_SP,
                },
                body_id,
            ),
            span: DUMMY_SP,
        })),
        nodes: OwnerNodes::new(),
    };
    batch.define(define(invalid_type), None, None).unwrap();
    let before = batch.to_string();
    let identities = BTreeMap::from([(0, 73)]);
    let target = Some(TargetData::new(32).unwrap());
    let error = publish(
        &mut batch,
        &mut 100,
        &identities,
        &Protocols::default(),
        target,
    )
    .unwrap_err();
    assert!(matches!(error, PublicationError::Check(_)), "{error}");
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    assert_eq!(batch.published().num_bodies(), 0);
    batch.define(define(valid_type), None, None).unwrap();
    let (owners, checked) = publish(
        &mut batch,
        &mut 100,
        &identities,
        &Protocols::default(),
        target,
    )
    .unwrap();
    assert_eq!(owners, vec![owner]);
    assert_eq!(checked.functions.len(), 1);
    assert_eq!(checked.functions[0].declaration.package, 73);
    assert_eq!(checked.target, target);
    assert_eq!(batch.published().num_defs(), 1);
    assert_eq!(batch.published().num_bodies(), 1);
    // A second rejected candidate must preserve the previously checked package.
    let unchanged = batch.published().to_string();
    let incomplete = batch.reserve().unwrap();
    assert_ne!(incomplete, owner);
    assert!(matches!(
        publish(
            &mut batch,
            &mut 100,
            &identities,
            &Protocols::default(),
            target
        ),
        Err(PublicationError::Structure(_))
    ));
    assert_eq!(batch.published().to_string(), unchanged);
    assert_eq!(checked.functions.len(), 1);
}
