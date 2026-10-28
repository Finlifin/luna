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
        captures: HashMap::new(),
        capture_uses: BTreeSet::new(),
        self_type: None,
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
    source.bind_definition(DefId::new(3, 4), member);
    source.source_origins.insert(site, original_member);
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
            // Binding identities move; original source configuration does not.
            assert_eq!(
                candidate.source_origin(generated_site),
                Some(original_member)
            );
            let second = clone_member(
                candidate,
                &arena,
                owner,
                OwnerId::new(hir::LocalDefId::new(999)),
                &HashMap::from([(original_member, generated_member)]),
                &mut 1000,
            )
            .unwrap();
            let second_site = second.bindings[&generated_site];
            assert_eq!(
                second
                    .resolution
                    .iter()
                    .find(|(id, _)| *id == second_site)
                    .unwrap()
                    .1
                    .source_origin,
                Some(original_member)
            );
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(source.member_projections.len(), 1);
}

fn generic_fixture<'h>(arena: &'h HirArena) -> (Package<'h>, OwnerId, DefId, DefId) {
    let (mut source, member) = fixture(arena, false);
    let first = DefId::new(0, 40);
    let second = DefId::new(0, 41);
    let name = Ident::new(Symbol::intern("T"), DUMMY_SP);
    let reference = arena.alloc_expr(Expr {
        hir_id: HirId::new(member, ItemLocalId::new(5)),
        kind: E::ResolvedClauseParam(name.name, first),
        span: DUMMY_SP,
    });
    let domain = arena.alloc_expr(Expr {
        hir_id: HirId::new(member, ItemLocalId::new(6)),
        kind: E::TyType,
        span: DUMMY_SP,
    });
    let mut item = source.item(member).unwrap().clone();
    let ItemKind::Fn(signature, _) = &mut item.kind else {
        panic!()
    };
    signature.clause_params = arena.alloc_clause_param_slice(vec![
        hir::ClauseParam {
            hir_id: HirId::new(member, ItemLocalId::new(3)),
            definition: Some(first),
            name,
            kind: hir::clause::ClauseParamKind::Type(name),
            span: DUMMY_SP,
        },
        hir::ClauseParam {
            hir_id: HirId::new(member, ItemLocalId::new(4)),
            definition: Some(second),
            name: Ident::new(Symbol::intern("U"), DUMMY_SP),
            kind: hir::clause::ClauseParamKind::Optional(name, domain, reference),
            span: DUMMY_SP,
        },
    ]);
    signature.return_ty = Some(reference);
    source.insert_owner(
        member,
        OwnerInfo {
            node: OwnerNode::Item(arena.alloc_item(item)),
            nodes: OwnerNodes::new(),
        },
    );
    (source, member, first, second)
}

#[test]
fn generic_method_defaults_and_returns_use_fresh_clause_identities() {
    let arena = HirArena::new();
    let (source, member, first, second) = generic_fixture(&arena);
    let mut destination = source.clone();
    let mut batch = hir::definitions::DefinitionBatch::new(&mut destination);
    for _ in 0..2 {
        let owner = batch.reserve().unwrap();
        let first_new = batch.reserve_identity(0, &mut 1000).unwrap();
        let second_new = batch.reserve_identity(0, &mut 1000).unwrap();
        let draft = clone_member(
            &source,
            &arena,
            member,
            owner,
            &HashMap::from([(first, first_new), (second, second_new)]),
            &mut 1000,
        )
        .unwrap();
        let ItemKind::Fn(signature, _) = &draft.item.kind else {
            panic!()
        };
        assert_eq!(signature.clause_params[0].definition, Some(first_new));
        assert!(
            matches!(signature.return_ty.unwrap().kind, E::ResolvedClauseParam(_, id) if id == first_new)
        );
        let hir::clause::ClauseParamKind::Optional(_, _, default) = signature.clause_params[1].kind
        else {
            panic!()
        };
        assert!(matches!(default.kind, E::ResolvedClauseParam(_, id) if id == first_new));
        batch
            .define(
                OwnerInfo {
                    node: OwnerNode::Item(arena.alloc_item(draft.item)),
                    nodes: draft.nodes,
                },
                None,
                None,
            )
            .unwrap();
        for (id, body) in draft.bodies {
            batch.body(id, body).unwrap();
        }
    }
    batch.publish(&mut 1000).unwrap();
    let owner = OwnerId::new(hir::LocalDefId::new(99));
    for mapping in [
        HashMap::new(),
        HashMap::from([(first, first), (second, second)]),
        HashMap::from([(first, DefId::new(0, 80)), (second, DefId::new(0, 80))]),
    ] {
        assert!(clone_member(&source, &arena, member, owner, &mapping, &mut 1000).is_err());
    }
}

#[test]
fn member_attributes_keep_order_targets_and_distinct_lexical_contexts() {
    use hir::{AttributeTarget as Target, DeferredDefinitionAttribute as Attribute};
    let arena = HirArena::new();
    let (mut source, member, first, second) = generic_fixture(&arena);
    let outer = source.alloc_owner_id();
    let outer_item = arena.alloc_item(Item {
        owner_id: outer,
        ident: Ident::new(Symbol::intern("outer"), DUMMY_SP),
        kind: ItemKind::Mod(hir::ModDef {
            items: vec![member],
        }),
        span: DUMMY_SP,
    });
    source.insert_owner(
        outer,
        OwnerInfo {
            node: OwnerNode::Item(outer_item),
            nodes: OwnerNodes::new(),
        },
    );
    let outer_value = arena.alloc_expr(Expr {
        hir_id: HirId::new(outer, ItemLocalId::new(1)),
        kind: E::TyType,
        span: DUMMY_SP,
    });
    let ItemKind::Fn(signature, _) = &source.item(member).unwrap().kind else {
        panic!()
    };
    let local_value = signature.return_ty.unwrap();
    source.definition_attributes = vec![
        Attribute {
            target: Target::Definition(member),
            lexical_owner: outer,
            expression: "prefix".into(),
            value: outer_value,
            configuration_key: None,
            span: DUMMY_SP,
        },
        Attribute {
            target: Target::Definition(member),
            lexical_owner: member,
            expression: "T".into(),
            value: local_value,
            configuration_key: Some("configuration".into()),
            span: DUMMY_SP,
        },
        Attribute {
            target: Target::Member(HirId::make_owner(member)),
            lexical_owner: outer,
            expression: "parameter".into(),
            value: outer_value,
            configuration_key: None,
            span: DUMMY_SP,
        },
    ];
    let mut destination = source.clone();
    let mut batch = hir::definitions::DefinitionBatch::new(&mut destination);
    let owner = batch.reserve().unwrap();
    let first_new = batch.reserve_identity(0, &mut 1000).unwrap();
    let second_new = batch.reserve_identity(0, &mut 1000).unwrap();
    let draft = clone_member(
        &source,
        &arena,
        member,
        owner,
        &HashMap::from([(first, first_new), (second, second_new)]),
        &mut 1000,
    )
    .unwrap();
    assert_eq!(draft.attributes.len(), 3);
    assert_eq!(draft.attributes[0].lexical_owner, outer);
    assert!(std::ptr::eq(draft.attributes[0].value, outer_value));
    assert_eq!(draft.attributes[1].lexical_owner, owner);
    assert!(
        matches!(draft.attributes[1].value.kind, E::ResolvedClauseParam(_, id) if id == first_new)
    );
    assert!(
        matches!(draft.attributes[2].target, Target::Member(id) if id == draft.bindings[&HirId::make_owner(member)])
    );
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(draft.item)),
                nodes: draft.nodes,
            },
            None,
            None,
        )
        .unwrap();
    for (id, body) in draft.bodies {
        batch.body(id, body).unwrap();
    }
    batch.set_attributes(draft.attributes);
    batch.set_resolution(draft.resolution);
    batch
        .publish_checked(&mut 1000, |candidate| {
            assert_eq!(candidate.definition_attributes.len(), 6);
            assert_eq!(candidate.definition_attributes[3].expression, "prefix");
            assert_eq!(
                candidate.definition_attributes[4]
                    .configuration_key
                    .as_deref(),
                Some("configuration")
            );
            assert_eq!(candidate.definition_attributes[5].expression, "parameter");
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(source.definition_attributes.len(), 3);
}

#[test]
fn staged_method_inputs_keep_independent_published_identities() {
    let arena = HirArena::new();
    let (mut source, member) = fixture(&arena, false);
    let binding = HirId::make_owner(member);
    let original = DefId::new(0, 50);
    let mut item = source.item(member).unwrap().clone();
    let ItemKind::Fn(signature, body_id) = &mut item.kind else {
        panic!()
    };
    let mut parameter = signature.params[0].clone();
    parameter.ty = parameter.ty.with_comptime();
    signature.params = arena.alloc_fn_param_slice(vec![parameter.clone()]);
    let mut body = source.body(*body_id).unwrap().clone();
    body.params = arena.alloc_param_slice(vec![hir::body::BodyParam::from_fn_param(&parameter)]);
    source.insert_body(*body_id, body);
    source.insert_owner(
        member,
        OwnerInfo {
            node: OwnerNode::Item(arena.alloc_item(item)),
            nodes: OwnerNodes::new(),
        },
    );
    source
        .staged_parameter_definitions
        .insert(binding, original);
    source.reserve_definition_prefix(original.index + 1);
    let owner = OwnerId::new(hir::LocalDefId::new(1));
    for mapping in [HashMap::new(), HashMap::from([(original, original)])] {
        assert!(clone_member(&source, &arena, member, owner, &mapping, &mut 1000).is_err());
    }
    let mut destination = source.clone();
    let mut batch = hir::definitions::DefinitionBatch::new(&mut destination);
    let mut staged = Vec::new();
    for _ in 0..2 {
        let owner = batch.reserve().unwrap();
        let fresh = batch.reserve_identity(0, &mut 1000).unwrap();
        let draft = clone_member(
            &source,
            &arena,
            member,
            owner,
            &HashMap::from([(original, fresh)]),
            &mut 1000,
        )
        .unwrap();
        let new_binding = draft.bindings[&binding];
        assert_eq!(draft.staged_parameters, vec![(new_binding, fresh)]);
        staged.extend(draft.staged_parameters);
        batch
            .define(
                OwnerInfo {
                    node: OwnerNode::Item(arena.alloc_item(draft.item)),
                    nodes: draft.nodes,
                },
                None,
                None,
            )
            .unwrap();
        for (id, body) in draft.bodies {
            batch.body(id, body).unwrap();
        }
    }
    assert_ne!(staged[0].1, staged[1].1);
    batch.set_staged_parameters(staged.clone());
    batch
        .publish_checked(&mut 1000, |candidate| {
            for (binding, identity) in &staged {
                assert_eq!(candidate.staged_parameter_definitions[binding], *identity);
            }
            assert_eq!(candidate.staged_parameter_definitions[&binding], original);
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(source.staged_parameter_definitions.len(), 1);
}

#[test]
fn associated_members_rebind_generic_defaults_without_creating_function_bodies() {
    let arena = HirArena::new();
    let (mut source, member, first, second) = generic_fixture(&arena);
    let mut item = source.item(member).unwrap().clone();
    let ItemKind::Fn(signature, _) = &item.kind else {
        panic!()
    };
    let ty = arena.alloc_expr(Expr {
        hir_id: HirId::new(member, ItemLocalId::new(7)),
        kind: E::TyType,
        span: DUMMY_SP,
    });
    item.kind = ItemKind::Associated(hir::item::AssociatedDecl {
        parameters: &[],
        ty,
        default: signature.return_ty,
        clause_params: signature.clause_params,
        clause_constraints: signature.clause_constraints,
    });
    source.insert_owner(
        member,
        OwnerInfo {
            node: OwnerNode::Item(arena.alloc_item(item)),
            nodes: OwnerNodes::new(),
        },
    );
    let mut destination = source.clone();
    let mut batch = hir::definitions::DefinitionBatch::new(&mut destination);
    let owner = batch.reserve().unwrap();
    let first_new = batch.reserve_identity(0, &mut 1000).unwrap();
    let second_new = batch.reserve_identity(0, &mut 1000).unwrap();
    let draft = clone_member(
        &source,
        &arena,
        member,
        owner,
        &HashMap::from([(first, first_new), (second, second_new)]),
        &mut 1000,
    )
    .unwrap();
    assert!(draft.bodies.is_empty());
    let ItemKind::Associated(definition) = &draft.item.kind else {
        panic!()
    };
    assert_eq!(definition.clause_params[0].definition, Some(first_new));
    assert!(
        matches!(definition.default.unwrap().kind, E::ResolvedClauseParam(_, id) if id == first_new)
    );
    assert_eq!(definition.ty.hir_id.owner, owner);
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(draft.item)),
                nodes: draft.nodes,
            },
            None,
            None,
        )
        .unwrap();
    batch.publish(&mut 1000).unwrap();
    assert_eq!(source.num_defs(), 1);
}

#[test]
fn explicit_generation_inputs_do_not_become_method_parameters() {
    let arena = HirArena::new();
    let (source, member) = fixture(&arena, true);
    let capture = HirId::make_owner(OwnerId::new(hir::LocalDefId::new(99)));
    for index in [1, 2] {
        let owner = OwnerId::new(hir::LocalDefId::new(index));
        let definition = DefId::new(0, 100 + index);
        let captures = HashMap::from([(capture, InputCapture::Definition(definition))]);
        let draft = clone_member_with_inputs(
            &source,
            &arena,
            member,
            owner,
            &HashMap::new(),
            &captures,
            &mut 1000,
        )
        .unwrap();
        let ItemKind::Fn(signature, body) = &draft.item.kind else {
            panic!()
        };
        assert_eq!(signature.params.len(), 1);
        let (_, body) = draft.bodies.iter().find(|(id, _)| id == body).unwrap();
        assert!(matches!(body.value.kind, E::ResolvedDef { def_id, .. } if def_id == definition));
        assert_eq!(body.value.hir_id.owner, owner);
        assert!(!draft.bindings.contains_key(&capture));
        assert_eq!(draft.capture_uses, BTreeSet::from([definition]));
        assert!(
            clone_member_with_inputs(
                &source,
                &arena,
                member,
                owner,
                &HashMap::new(),
                &captures,
                &mut 1
            )
            .is_err()
        );
    }
    let original = source.body(BodyId::new(HirId::make_owner(member))).unwrap();
    assert!(matches!(original.value.kind, E::ResolvedFnParam(_, id) if id == capture));
}
