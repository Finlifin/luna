use hir::common::DefId;
use hir::{Body, BodyId, Expr, HirId, Ident, Item, ItemLocalId, OwnerNodes, Symbol};
use hir::{ExprKind as E, HirArena, ItemKind, OwnerId, OwnerInfo, OwnerNode, Package};
use rustc_span::DUMMY_SP;
use std::collections::BTreeMap;

fn fixture<'h>(arena: &'h HirArena, target_package: u32) -> Package<'h> {
    let mut package = Package::new();
    let root = package.alloc_owner_id();
    let interface = package.alloc_owner_id();
    let subject = package.alloc_owner_id();
    let strategy = package.alloc_owner_id();
    let request = package.alloc_owner_id();
    package.root_mod = root;
    let interface_id = DefId::new(7, 10);
    let subject_id = DefId::new(target_package, 11);
    let strategy_id = DefId::new(7, 12);
    let expression = |owner, local, name: &str, id| {
        arena.alloc_expr(Expr {
            hir_id: HirId::new(owner, ItemLocalId::new(local)),
            kind: E::ResolvedDef {
                name: Symbol::intern(name),
                def_id: id,
                has_clause_param: false,
            },
            span: DUMMY_SP,
        })
    };
    let expected = expression(strategy, 3, "Flag", interface_id);
    let requested = expression(request, 1, "Flag", interface_id);
    let target = expression(request, 2, "Subject", subject_id);
    let selected = expression(request, 3, "enabled", strategy_id);
    let body = BodyId::new(HirId::make_owner(strategy));
    let block = arena.alloc_block(hir::Block {
        hir_id: HirId::new(strategy, ItemLocalId::new(1)),
        stmts: &[],
        expr: None,
        span: DUMMY_SP,
    });
    package.insert_body(
        body,
        Body {
            params: &[],
            value: arena.alloc_expr(Expr {
                hir_id: HirId::new(strategy, ItemLocalId::new(2)),
                kind: E::Block(block),
                span: DUMMY_SP,
            }),
        },
    );
    for (owner, name, identity, kind) in [
        (
            root,
            "root",
            DefId::new(0, 0),
            ItemKind::Mod(hir::ModDef {
                items: vec![interface, subject, strategy, request],
            }),
        ),
        (
            interface,
            "Flag",
            interface_id,
            ItemKind::Trait(hir::TraitDef {
                comptime_only: false,
                super_traits: None,
                clause_params: &[],
                clause_constraints: &[],
                items: vec![],
            }),
        ),
        (
            subject,
            "Subject",
            subject_id,
            ItemKind::Struct(hir::StructDef {
                fields: &[],
                clause_params: &[],
                clause_constraints: &[],
                nested_items: vec![],
            }),
        ),
        (
            strategy,
            "enabled",
            strategy_id,
            ItemKind::Deriver(hir::item::DeriverDef {
                parameters: &[],
                trait_ref: expected,
                clause_params: &[],
                clause_constraints: &[],
                body,
            }),
        ),
        (
            request,
            "derive",
            DefId::new(0, 13),
            ItemKind::Derive(hir::item::DeriveRequest {
                phase: hir::ImplementationPhase::Runtime,
                trait_ref: requested,
                self_ty: target,
                strategy: Some(selected),
                clause_params: &[],
                clause_constraints: &[],
            }),
        ),
    ] {
        package.insert_owner(
            owner,
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(Item {
                    owner_id: owner,
                    ident: Ident::new(Symbol::intern(name), DUMMY_SP),
                    kind,
                    span: DUMMY_SP,
                })),
                nodes: OwnerNodes::new(),
            },
        );
        package.bind_definition(identity, owner);
    }
    package
        .pending_semantics
        .push((DUMMY_SP, "derivation expansion".into()));
    package
}

#[test]
fn external_strategy_does_not_transfer_implementation_ownership() {
    let arena = HirArena::new();
    let identities = BTreeMap::from([(0, 100), (7, 700)]);
    let local = fixture(&arena, 0);
    let checked = elaboration::program::elaborate_packages(&local, &identities).unwrap();
    assert_eq!(checked.implementations.len(), 1);
    assert_eq!(checked.implementations[0].declaration.package, 100);
    let foreign = fixture(&arena, 7);
    let error = elaboration::program::elaborate_packages(&foreign, &identities).unwrap_err();
    assert!(error.message.contains("NoLocalIdentity"), "{error}");
    assert_eq!(local.pending_semantics.len(), 1);
}
