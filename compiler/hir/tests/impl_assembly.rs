use hir::{definitions::assemble_inherent_impl, *};
use rustc_span::DUMMY_SP;
use std::sync::Arc;

fn owner(local: u32) -> OwnerId {
    OwnerId::new(LocalDefId::new(local))
}
fn name() -> Ident {
    Ident::new(Symbol::intern("generated_impl"), DUMMY_SP)
}
fn ty() -> Arc<nir::Term> {
    Arc::new(nir::Term::Primitive(nir::Primitive::Bool))
}

#[test]
fn inherent_impl_retains_member_order_and_indexes_receiver_type() {
    let arena = HirArena::new();
    let input = ty();
    let mut fuel = 5;
    let info = assemble_inherent_impl(
        &arena,
        owner(1),
        name(),
        input.clone(),
        &[owner(3), owner(2)],
        DUMMY_SP,
        &mut fuel,
    )
    .unwrap();
    assert_eq!(fuel, 0);
    let ItemKind::Impl(definition) = &info.node.expect_item().kind else {
        panic!()
    };
    assert_eq!(definition.items, [owner(3), owner(2)]);
    assert!(definition.trait_ref.is_none());
    assert_eq!(definition.self_ty.hir_id.owner, owner(1));
    assert_eq!(
        info.nodes
            .get(definition.self_ty.hir_id.local_id)
            .unwrap()
            .parent,
        ItemLocalId::new(0)
    );
    assert!(
        matches!(&definition.self_ty.kind, ExprKind::DeliveredType(ty) if Arc::ptr_eq(ty, &input))
    );
    assert!(
        matches!(info.nodes.get(ItemLocalId::new(0)).unwrap().node, Node::Item(root) if std::ptr::eq(root, info.node.expect_item()))
    );
}

#[test]
fn implementation_budget_is_preflighted_and_empty_impl_is_supported() {
    let arena = HirArena::new();
    let mut fuel = 3;
    assert_eq!(
        assemble_inherent_impl(
            &arena,
            owner(1),
            name(),
            ty(),
            &[owner(2)],
            DUMMY_SP,
            &mut fuel
        )
        .unwrap_err()
        .kind,
        "budget exhausted"
    );
    assert_eq!(fuel, 3);
    assert!(
        assemble_inherent_impl(&arena, owner(1), name(), ty(), &[], DUMMY_SP, &mut fuel).is_ok()
    );
    assert_eq!(fuel, 0);
    assert!(
        assemble_inherent_impl(
            &arena,
            OwnerId::INVALID,
            name(),
            ty(),
            &[],
            DUMMY_SP,
            &mut 100
        )
        .is_err()
    );
}
