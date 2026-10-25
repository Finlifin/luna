use elaboration::{definitions::publish_with_invocation_budget, program::Protocols};
use hir::{
    definitions::{DefinitionBatch, PublicationError},
    *,
};
use rustc_span::DUMMY_SP;
use std::collections::BTreeMap;

#[test]
fn required_function_invocation_obeys_publication_budget_and_can_retry() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let factory = batch.reserve().unwrap();
    let alias = batch.reserve().unwrap();
    let make_expr = |owner, local, kind| {
        arena.alloc_expr(Expr {
            hir_id: HirId::new(owner, ItemLocalId::new(local)),
            kind,
            span: DUMMY_SP,
        })
    };
    let result = make_expr(factory, 1, ExprKind::TyType);
    let value = make_expr(factory, 2, ExprKind::Builtin(Symbol::intern("bool")));
    let body = BodyId::new(value.hir_id);
    batch.body(body, Body { params: &[], value }).unwrap();
    let sig = FnSig {
        params: &[],
        return_ty: Some(result),
        return_bind: None,
        return_binding: None,
        modifiers: hir::item::FnModifiers {
            is_comptime: true,
            is_pure: true,
            ..Default::default()
        },
        clause_params: &[],
        clause_constraints: &[],
        span: DUMMY_SP,
    };
    let factory_id = hir::common::DefId::new(0, factory.def_id.raw());
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(Item {
                    owner_id: factory,
                    ident: Ident::new(Symbol::intern("Make"), DUMMY_SP),
                    kind: ItemKind::Fn(sig, body),
                    span: DUMMY_SP,
                })),
                nodes: OwnerNodes::new(),
            },
            Some(factory_id),
            None,
        )
        .unwrap();
    let callee = make_expr(
        alias,
        2,
        ExprKind::ResolvedDef {
            name: Symbol::intern("Make"),
            has_clause_param: false,
            def_id: factory_id,
        },
    );
    let call = make_expr(alias, 1, ExprKind::Application(callee, &[]));
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(Item {
                    owner_id: alias,
                    ident: Ident::new(Symbol::intern("Generated"), DUMMY_SP),
                    kind: ItemKind::TypeAlias(call, &[], &[]),
                    span: DUMMY_SP,
                })),
                nodes: OwnerNodes::new(),
            },
            None,
            None,
        )
        .unwrap();
    let identities = BTreeMap::from([(0, 73)]);
    let before = batch.to_string();
    let mut steps = 0;
    let failed = publish_with_invocation_budget(
        &mut batch,
        &mut 1000,
        &mut steps,
        &identities,
        &Protocols::default(),
        None,
    );
    assert!(
        matches!(failed, Err(PublicationError::Check(_))),
        "{failed:?}"
    );
    assert_eq!(steps, 0);
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    steps = 10_000;
    let (_, checked) = publish_with_invocation_budget(
        &mut batch,
        &mut 1000,
        &mut steps,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert!(steps < 10_000);
    assert_eq!(batch.published().num_defs(), 2);
    assert!(
        checked
            .functions
            .iter()
            .any(|function| function.name == "Make")
    );
}
