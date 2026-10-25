use hir::{
    definitions::{DefinitionBatch, TypedField, assemble_struct},
    *,
};
use rustc_span::DUMMY_SP;
use std::sync::Arc;

fn field(name: &str) -> TypedField {
    TypedField {
        name: Ident::new(Symbol::intern(name), DUMMY_SP),
        ty: Arc::new(nir::Term::Primitive(nir::Primitive::Bool)),
        span: DUMMY_SP,
    }
}
fn name() -> Ident {
    Ident::new(Symbol::intern("Generated"), DUMMY_SP)
}

#[test]
fn assembly_indexes_every_field_and_type_with_the_right_parents() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let inputs = [field("first"), field("second")];
    let mut fuel = 9;
    let info = assemble_struct(&arena, owner, name(), &inputs, DUMMY_SP, &mut fuel).unwrap();
    assert_eq!(fuel, 0);
    let ItemKind::Struct(definition) = &info.node.expect_item().kind else {
        panic!()
    };
    assert_eq!(definition.fields.len(), 2);
    for (field, input) in definition.fields.iter().zip(&inputs) {
        assert_eq!(field.ident, input.name);
        assert_eq!(field.hir_id.owner, owner);
        assert_eq!(
            info.nodes.get(field.hir_id.local_id).unwrap().parent,
            ItemLocalId::new(0)
        );
        assert_eq!(
            info.nodes.get(field.ty.hir_id.local_id).unwrap().parent,
            field.hir_id.local_id
        );
        let ExprKind::DeliveredType(ty) = &field.ty.kind else {
            panic!()
        };
        assert!(Arc::ptr_eq(ty, &input.ty));
        assert!(field.default.is_none());
    }
    assert!(
        matches!(info.nodes.get(ItemLocalId::new(0)).unwrap().node, Node::Item(root) if std::ptr::eq(root, info.node.expect_item()))
    );
    batch.define(info, None, None).unwrap();
    batch.publish(&mut 100).unwrap();
    assert_eq!(batch.published().num_defs(), 1);
}

#[test]
fn insufficient_budget_keeps_existing_draft_and_can_retry() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let original = assemble_struct(&arena, owner, name(), &[], DUMMY_SP, &mut 1).unwrap();
    batch.define(original, None, None).unwrap();
    let before = batch.to_string();
    let mut fuel = 4;
    assert_eq!(
        assemble_struct(
            &arena,
            owner,
            name(),
            &[field("value")],
            DUMMY_SP,
            &mut fuel
        )
        .unwrap_err()
        .kind,
        "budget exhausted"
    );
    assert_eq!(fuel, 4);
    assert_eq!(batch.to_string(), before);
    let replacement =
        assemble_struct(&arena, owner, name(), &[field("value")], DUMMY_SP, &mut 5).unwrap();
    batch.define(replacement, None, None).unwrap();
    batch.publish(&mut 100).unwrap();
    assert!(assemble_struct(&arena, OwnerId::INVALID, name(), &[], DUMMY_SP, &mut 1).is_err());
}

#[test]
fn field_description_dump_escapes_names() {
    let description = field("name\n\"").to_string();
    assert!(description.starts_with("(typed-field (name "));
    assert!(description.contains("name\\n\\\""), "{description}");
}
