use super::*;
use hir::{Body, BodyId, Ident, Item, ItemLocalId, OwnerInfo, OwnerNode, OwnerNodes, Symbol};
use rustc_span::DUMMY_SP;

fn fixture<'h>(arena: &'h HirArena, capture: bool) -> (Package<'h>, OwnerId) {
    let mut package = Package::new();
    let owner = package.alloc_owner_id();
    let id = HirId::make_owner(owner);
    let name = Ident::new(Symbol::intern("self"), DUMMY_SP);
    let parameter = hir::common::FnParam {
        hir_id: id,
        name,
        index: 0,
        ty: TyParam {
            hir_id: id,
            kind: T::Itself { is_ref: true },
            flags: 0,
            span: DUMMY_SP,
        },
        span: DUMMY_SP,
    };
    let reference = if capture {
        HirId::make_owner(OwnerId::new(hir::LocalDefId::new(99)))
    } else {
        id
    };
    let value = arena.alloc_expr(Expr {
        hir_id: HirId::new(owner, ItemLocalId::new(2)),
        kind: E::ResolvedFnParam(name.name, reference),
        span: DUMMY_SP,
    });
    let body = BodyId::new(id);
    package.insert_body(
        body,
        Body {
            params: arena.alloc_param_slice(vec![hir::body::BodyParam::from_fn_param(&parameter)]),
            value,
        },
    );
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        span: DUMMY_SP,
        kind: ItemKind::Fn(
            hir::FnSig {
                params: arena.alloc_fn_param_slice(vec![parameter]),
                return_ty: None,
                return_bind: None,
                return_binding: None,
                modifiers: hir::item::FnModifiers::default(),
                clause_params: &[],
                clause_constraints: &[],
                span: DUMMY_SP,
            },
            body,
        ),
    });
    package.insert_owner(
        owner,
        OwnerInfo {
            node: OwnerNode::Item(item),
            nodes: OwnerNodes::new(),
        },
    );
    (package, owner)
}

#[test]
fn independent_members_preserve_receiver_links_and_publish() {
    let arena = HirArena::new();
    let (source, member) = fixture(&arena, false);
    let mut destination = source.clone();
    let mut batch = hir::definitions::DefinitionBatch::new(&mut destination);
    let mut receivers = Vec::new();
    for _ in 0..2 {
        let owner = batch.reserve().unwrap();
        let draft =
            clone_member(&source, &arena, member, owner, &HashMap::new(), &mut 1000).unwrap();
        let ItemKind::Fn(signature, body) = &draft.item.kind else {
            panic!()
        };
        let receiver = signature.params[0].hir_id;
        assert_ne!(receiver.local_id.raw(), 0);
        assert_eq!(draft.bindings[&HirId::make_owner(member)], receiver);
        let (_, cloned_body) = draft.bodies.iter().find(|(id, _)| id == body).unwrap();
        assert_eq!(cloned_body.params[0].hir_id, receiver);
        assert!(matches!(cloned_body.value.kind, E::ResolvedFnParam(_, id) if id == receiver));
        receivers.push(receiver);
        batch
            .define(
                OwnerInfo {
                    node: OwnerNode::Item(arena.alloc_item(draft.item)),
                    nodes: OwnerNodes::new(),
                },
                None,
                None,
            )
            .unwrap();
        for (id, body) in draft.bodies {
            batch.body(id, body).unwrap();
        }
    }
    assert_ne!(receivers[0], receivers[1]);
    batch.publish(&mut 1000).unwrap();
    assert_eq!(source.num_defs(), 1);
}

#[test]
fn rejects_captures_occupied_owners_and_recovers_after_budget_exhaustion() {
    let arena = HirArena::new();
    let (source, member) = fixture(&arena, true);
    let owner = OwnerId::new(hir::LocalDefId::new(1));
    assert!(clone_member(&source, &arena, member, owner, &HashMap::new(), &mut 1000).is_err());
    let (source, member) = fixture(&arena, false);
    assert!(clone_member(&source, &arena, member, member, &HashMap::new(), &mut 1000).is_err());
    assert!(clone_member(&source, &arena, member, owner, &HashMap::new(), &mut 1).is_err());
    assert!(clone_member(&source, &arena, member, owner, &HashMap::new(), &mut 1000).is_ok());
    assert_eq!(source.num_defs(), 1);
}

#[test]
fn remaps_generated_definition_references_without_rewriting_external_ones() {
    let arena = HirArena::new();
    let source = Package::new();
    let owner = OwnerId::new(hir::LocalDefId::new(1));
    let old = DefId::new(0, 8);
    let new = DefId::new(0, 9);
    let external = DefId::new(1, 8);
    let mut rebind = Rebind {
        source: &source,
        arena: &arena,
        namespace: owner,
        bindings: HashMap::new(),
        budget: 100,
        bodies: Vec::new(),
        definitions: HashMap::from([(old, new)]),
        member_owner: Some(owner),
    };
    for (input, expected) in [(old, new), (external, external)] {
        let expression = Expr {
            hir_id: HirId::make_owner(owner),
            kind: E::ResolvedDef {
                name: Symbol::intern("member"),
                def_id: input,
                has_clause_param: false,
            },
            span: DUMMY_SP,
        };
        assert!(matches!(rebind.expr(&expression, 0).unwrap().kind,
            E::ResolvedDef { def_id, .. } if def_id == expected));
    }
}

#[test]
fn cloned_resolution_remaps_both_candidate_roles_and_publishes_atomically() {
    let arena = HirArena::new();
    let (mut source, member) = fixture(&arena, false);
    let site = HirId::new(member, ItemLocalId::new(2));
    let original_container = DefId::new(0, 8);
    let original_member = DefId::new(0, 9);
    let generated_container = DefId::new(0, 18);
    let generated_member = DefId::new(0, 19);
    let external = DefId::new(1, 9);
    source.member_projections.insert(site, original_member);
    source.deferred_member_projections.insert(
        site,
        vec![(original_container, original_member), (external, external)].into(),
    );
    source.member_access.insert(site, Vec::new().into());
    let mut destination = source.clone();
    let mut batch = hir::definitions::DefinitionBatch::new(&mut destination);
    let owner = batch.reserve().unwrap();
    let draft = clone_member(
        &source,
        &arena,
        member,
        owner,
        &HashMap::from([
            (original_container, generated_container),
            (original_member, generated_member),
        ]),
        &mut 1000,
    )
    .unwrap();
    let generated_site = draft.bindings[&site];
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(draft.item)),
                nodes: OwnerNodes::new(),
            },
            None,
            None,
        )
        .unwrap();
    for (id, body) in draft.bodies {
        batch.body(id, body).unwrap();
    }
    batch.set_resolution(draft.resolution);
    batch
        .publish_checked(&mut 1000, |candidate| {
            assert_eq!(
                candidate.member_projections[&generated_site],
                generated_member
            );
            assert_eq!(
                &*candidate.deferred_member_projections[&generated_site],
                &[
                    (generated_container, generated_member),
                    (external, external)
                ]
            );
            assert!(candidate.member_access[&generated_site].is_empty());
            assert_eq!(candidate.member_projections[&site], original_member);
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(source.member_projections.len(), 1);
}
