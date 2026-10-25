use hir::definitions::{DefinitionBatch, DefinitionNodes};
use hir::*;
use rustc_span::DUMMY_SP;
fn item<'a>(arena: &'a HirArena, owner: OwnerId) -> &'a Item<'a> {
    arena.alloc_item(Item {
        owner_id: owner,
        ident: Ident::new(Symbol::intern("generated"), DUMMY_SP),
        kind: ItemKind::Mod(ModDef { items: vec![] }),
        span: DUMMY_SP,
    })
}
fn expr<'a>(arena: &'a HirArena, id: HirId) -> &'a Expr<'a> {
    arena.alloc_expr(Expr {
        hir_id: id,
        kind: ExprKind::TyVoid,
        span: DUMMY_SP,
    })
}
#[test]
fn reserved_parents_allow_bottom_up_registration_and_canonical_publication() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let mut draft = DefinitionNodes::new(owner).unwrap();
    let mut fuel = 100;
    let parent = draft.reserve(draft.root(), &mut fuel).unwrap();
    let child = draft.reserve(parent, &mut fuel).unwrap();
    draft
        .register(Node::Expr(expr(&arena, child)), &mut fuel)
        .unwrap();
    draft
        .register(Node::Expr(expr(&arena, parent)), &mut fuel)
        .unwrap();
    let root = item(&arena, owner);
    let info = draft.finish(root, &mut fuel).unwrap();
    assert_eq!(
        info.nodes.get(child.local_id).unwrap().parent,
        parent.local_id
    );
    assert!(
        matches!(info.nodes.get(ItemLocalId::new(0)).unwrap().node,Node::Item(indexed) if std::ptr::eq(indexed,root))
    );
    batch.define(info, None, None).unwrap();
    batch.publish(&mut fuel).unwrap();
    assert!(batch.published().node(child).is_some());
    assert!(draft.reserve(draft.root(), &mut fuel).is_err());
    assert!(draft.finish(root, &mut fuel).is_err());
}
#[test]
fn rejected_operations_preserve_the_editable_draft_and_fuel_failures_recover() {
    let arena = HirArena::new();
    let owner = OwnerId::new(LocalDefId::new(1));
    let other = OwnerId::new(LocalDefId::new(2));
    let mut draft = DefinitionNodes::new(owner).unwrap();
    let initial = draft.to_string();
    assert!(draft.reserve(draft.root(), &mut 0).is_err());
    assert_eq!(draft.to_string(), initial);
    assert!(
        draft
            .reserve(HirId::new(other, ItemLocalId::new(0)), &mut 100)
            .is_err()
    );
    assert!(
        draft
            .reserve(HirId::new(owner, ItemLocalId::new(7)), &mut 100)
            .is_err()
    );
    let id = draft.reserve(draft.root(), &mut 100).unwrap();
    let pending = draft.to_string();
    assert!(draft.finish(item(&arena, owner), &mut 100).is_err());
    assert!(
        draft
            .register(Node::Expr(expr(&arena, id)), &mut 0)
            .is_err()
    );
    assert!(
        draft
            .register(
                Node::Expr(expr(&arena, HirId::new(other, id.local_id))),
                &mut 100
            )
            .is_err()
    );
    assert_eq!(draft.to_string(), pending);
    let value = expr(&arena, id);
    draft.register(Node::Expr(value), &mut 100).unwrap();
    let filled = draft.to_string();
    assert!(draft.register(Node::Expr(value), &mut 100).is_err());
    assert!(
        draft
            .register(Node::Item(item(&arena, owner)), &mut 100)
            .is_err()
    );
    assert!(draft.finish(item(&arena, other), &mut 100).is_err());
    assert!(draft.finish(item(&arena, owner), &mut 0).is_err());
    assert_eq!(draft.to_string(), filled);
    assert!(filled.contains("(kind \"expression\")"));
    draft.finish(item(&arena, owner), &mut 100).unwrap();
    assert!(DefinitionNodes::new(OwnerId::INVALID).is_err());
}
