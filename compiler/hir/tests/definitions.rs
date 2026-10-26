use hir::definitions::DefinitionBatch;
use hir::item::FnModifiers;
use hir::*;
use rustc_span::DUMMY_SP;
use symbol::DefId;

fn info<'a>(arena: &'a HirArena, owner: OwnerId, kind: ItemKind<'a>) -> OwnerInfo<'a> {
    OwnerInfo {
        node: OwnerNode::Item(arena.alloc_item(Item {
            owner_id: owner,
            ident: Ident::new(Symbol::intern("generated"), DUMMY_SP),
            kind,
            span: DUMMY_SP,
        })),
        nodes: OwnerNodes::new(),
    }
}
fn module<'a>(arena: &'a HirArena, owner: OwnerId, items: Vec<OwnerId>) -> OwnerInfo<'a> {
    info(arena, owner, ItemKind::Mod(ModDef { items }))
}
fn signature<'a>() -> FnSig<'a> {
    FnSig {
        params: &[],
        return_ty: None,
        return_bind: None,
        return_binding: None,
        modifiers: FnModifiers::default(),
        clause_params: &[],
        clause_constraints: &[],
        span: DUMMY_SP,
    }
}

#[test]
fn indexed_nodes_require_matching_owners_and_acyclic_present_parents() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let expression = arena.alloc_expr(Expr {
        hir_id: HirId::new(owner, ItemLocalId::new(1)),
        kind: ExprKind::TyVoid,
        span: DUMMY_SP,
    });
    for (local, parent, expected) in [
        (2, 0, "node index does not match its identity"),
        (1, 1, "cyclic node parents"),
        (1, 99, "missing node parent"),
    ] {
        let mut definition = module(&arena, owner, vec![]);
        definition.nodes.insert(
            ItemLocalId::new(local),
            ItemLocalId::new(parent),
            Node::Expr(expression),
        );
        batch.define(definition, None, None).unwrap();
        assert_eq!(batch.publish(&mut 100).unwrap_err().kind, expected);
        assert_eq!(batch.published().num_defs(), 0);
    }
    let root_expression = arena.alloc_expr(Expr {
        hir_id: HirId::make_owner(owner),
        kind: ExprKind::TyVoid,
        span: DUMMY_SP,
    });
    let mut definition = module(&arena, owner, vec![]);
    definition.nodes.insert(
        ItemLocalId::new(0),
        ItemLocalId::new(0),
        Node::Expr(root_expression),
    );
    batch.define(definition, None, None).unwrap();
    assert_eq!(
        batch.publish(&mut 100).unwrap_err().kind,
        "root index does not refer to its definition"
    );
    let mut definition = module(&arena, owner, vec![]);
    definition.nodes.insert(
        ItemLocalId::new(0),
        ItemLocalId::new(0),
        Node::Item(definition.node.expect_item()),
    );
    definition.nodes.insert(
        ItemLocalId::new(1),
        ItemLocalId::new(0),
        Node::Expr(expression),
    );
    batch.define(definition, None, None).unwrap();
    assert!(
        batch
            .to_string()
            .contains("(node-ref (local 1) (parent 0) (kind \"expression\"))")
    );
    batch.publish(&mut 100).unwrap();
    assert_eq!(
        batch.published().node(expression.hir_id).unwrap().hir_id(),
        expression.hir_id
    );
}

#[test]
fn deeply_nested_generated_definitions_share_validation_work() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owners: Vec<_> = (0..1024).map(|_| batch.reserve().unwrap()).collect();
    for (i, owner) in owners.iter().enumerate() {
        batch
            .define(
                module(
                    &arena,
                    *owner,
                    owners.get(i + 1).copied().into_iter().collect(),
                ),
                None,
                i.checked_sub(1).map(|parent| owners[parent]),
            )
            .unwrap();
    }
    // A quadratic walk over every ancestor/descendant cannot fit this budget.
    assert_eq!(batch.publish(&mut (owners.len() * 16)).unwrap(), owners);
}

#[test]
fn deeply_nested_node_indices_share_parent_validation_work() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let mut definition = module(&arena, owner, vec![]);
    for index in 1..=1024 {
        let node = arena.alloc_expr(Expr {
            hir_id: HirId::new(owner, ItemLocalId::new(index)),
            kind: ExprKind::TyVoid,
            span: DUMMY_SP,
        });
        definition.nodes.insert(
            ItemLocalId::new(index),
            ItemLocalId::new(index - 1),
            Node::Expr(node),
        );
    }
    batch.define(definition, None, None).unwrap();
    batch.publish(&mut (1024 * 16)).unwrap();
    assert_eq!(batch.published().num_defs(), 1);
}

#[test]
fn unfinished_definitions_are_invisible_and_dropping_does_not_consume_identities() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let first;
    {
        let mut batch = DefinitionBatch::new(&mut package);
        first = batch.reserve().unwrap();
        batch
            .define(module(&arena, first, vec![]), Some(DefId::new(1, 1)), None)
            .unwrap();
        let missing = batch.reserve().unwrap();
        let before = batch.published().to_string();
        assert_eq!(batch.publish(&mut 100).unwrap_err().owner, Some(missing));
        assert_eq!(batch.published().to_string(), before);
        assert_eq!(batch.published().num_defs(), 0);
        assert_eq!(batch.published().definition_owner(DefId::new(1, 1)), None);
    }
    assert_eq!(package.alloc_owner_id(), first);
}

#[test]
fn forward_containers_and_different_definition_kinds_publish_together() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owners: Vec<_> = (0..8).map(|_| batch.reserve().unwrap()).collect();
    let ty = arena.alloc_expr(Expr {
        hir_id: HirId::new(owners[5], ItemLocalId::new(1)),
        kind: ExprKind::TyVoid,
        span: DUMMY_SP,
    });
    let body_id = BodyId::new(HirId::make_owner(owners[6]));
    let value = arena.alloc_expr(Expr {
        hir_id: HirId::new(owners[6], ItemLocalId::new(1)),
        kind: ExprKind::Lit(Lit {
            kind: LitKind::Bool(true),
            span: DUMMY_SP,
        }),
        span: DUMMY_SP,
    });
    let kinds = [
        ItemKind::Mod(ModDef {
            items: owners[1..].to_vec(),
        }),
        ItemKind::Struct(StructDef {
            fields: &[],
            clause_params: &[],
            clause_constraints: &[],
            nested_items: vec![],
        }),
        ItemKind::Enum(EnumDef {
            is_intrinsic: false,
            variants: &[],
            configurations: vec![],
            clause_params: &[],
            clause_constraints: &[],
            nested_items: vec![],
        }),
        ItemKind::Trait(TraitDef {
            super_traits: None,
            clause_params: &[],
            clause_constraints: &[],
            items: vec![],
        }),
        ItemKind::FnDeclaration(signature()),
        ItemKind::Impl(ImplDef {
            self_ty: ty,
            trait_ref: None,
            clause_params: &[],
            clause_constraints: &[],
            items: vec![],
        }),
        ItemKind::Fn(signature(), body_id),
        ItemKind::Effect(signature(), hir::item::EffectModifiers::default()),
    ];
    for (index, (owner, kind)) in owners.iter().zip(kinds).enumerate() {
        batch
            .define(
                info(&arena, *owner, kind),
                Some(DefId::new(1, index as u32)),
                (*owner != owners[0]).then_some(owners[0]),
            )
            .unwrap();
    }
    assert!(batch.publish(&mut 1000).is_err()); // Missing callable body.
    assert_eq!(batch.published().num_defs(), 0);
    batch.body(body_id, Body { params: &[], value }).unwrap();
    assert_eq!(batch.publish(&mut 1000).unwrap(), owners);
    assert_eq!(batch.published().num_defs(), 8);
    assert_eq!(batch.published().num_bodies(), 1);
    for (i, owner) in owners.iter().enumerate() {
        assert_eq!(
            batch.published().definition_owner(DefId::new(1, i as u32)),
            Some(*owner)
        );
        assert_eq!(
            batch.published().owner_definition(*owner),
            Some(DefId::new(1, i as u32))
        );
    }
    assert_eq!(
        batch.published().lexical_parents.get(&owners[6]),
        Some(&owners[0])
    );
    assert!(batch.body(body_id, Body { params: &[], value }).is_err());
}

#[test]
fn duplicate_identities_and_cycles_can_be_corrected_without_partial_publication() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let a = batch.reserve().unwrap();
    let b = batch.reserve().unwrap();
    let identity = DefId::new(4, 5);
    batch
        .define(module(&arena, a, vec![]), Some(identity), None)
        .unwrap();
    batch
        .define(module(&arena, b, vec![]), Some(identity), None)
        .unwrap();
    assert_eq!(
        batch.publish(&mut 100).unwrap_err().kind,
        "invalid or duplicate definition identity"
    );
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(module(&arena, b, vec![a]), None, None)
        .unwrap();
    batch
        .define(module(&arena, a, vec![b]), Some(identity), None)
        .unwrap();
    assert_eq!(
        batch.publish(&mut 100).unwrap_err().kind,
        "cyclic definition containers"
    );
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(module(&arena, a, vec![]), Some(identity), Some(b))
        .unwrap();
    batch
        .define(module(&arena, b, vec![]), None, Some(a))
        .unwrap();
    assert_eq!(
        batch.publish(&mut 100).unwrap_err().kind,
        "cyclic lexical parents"
    );
    batch.define(module(&arena, b, vec![]), None, None).unwrap();
    assert_eq!(batch.publish(&mut 100).unwrap(), [a, b]);
    let c = batch.reserve().unwrap();
    batch
        .define(module(&arena, c, vec![]), Some(identity), None)
        .unwrap();
    assert!(batch.publish(&mut 100).is_err());
    assert_eq!(batch.published().definition_owner(identity), Some(a));
    assert_eq!(batch.published().num_defs(), 2);
}

#[test]
fn fuel_failure_and_foreign_bodies_leave_the_package_unchanged() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let body_id = BodyId::new(HirId::make_owner(owner));
    batch
        .define(
            info(&arena, owner, ItemKind::Fn(signature(), body_id)),
            None,
            None,
        )
        .unwrap();
    let foreign = OwnerId::new(LocalDefId::new(999));
    let bad = arena.alloc_expr(Expr {
        hir_id: HirId::make_owner(foreign),
        kind: ExprKind::TyVoid,
        span: DUMMY_SP,
    });
    assert!(
        batch
            .body(
                BodyId::new(HirId::make_owner(foreign)),
                Body {
                    params: &[],
                    value: bad
                }
            )
            .is_err()
    );
    batch
        .body(
            body_id,
            Body {
                params: &[],
                value: bad,
            },
        )
        .unwrap();
    assert_eq!(
        batch.publish(&mut 100).unwrap_err().kind,
        "body belongs to a different owner"
    );
    let value = arena.alloc_expr(Expr {
        hir_id: HirId::new(owner, ItemLocalId::new(1)),
        kind: ExprKind::TyVoid,
        span: DUMMY_SP,
    });
    batch.body(body_id, Body { params: &[], value }).unwrap();
    for fuel in [0, 1, 2] {
        assert_eq!(
            batch.publish(&mut { fuel }).unwrap_err().kind,
            "budget exhausted"
        );
        assert_eq!(batch.published().num_bodies(), 0);
        assert_eq!(batch.published().num_defs(), 0);
    }
    batch.publish(&mut 100).unwrap();
    assert_eq!(batch.published().body(body_id).unwrap().value, value);
}

#[test]
fn checked_publication_failure_and_unwind_keep_the_original_draft() {
    use hir::definitions::PublicationError;
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(module(&arena, owner, vec![]), None, None)
        .unwrap();
    let before = batch.to_string();
    let rejection = batch.publish_checked(&mut 100, |candidate| {
        assert_eq!(candidate.num_defs(), 1);
        Err::<(), _>("semantic rejection")
    });
    assert!(matches!(
        rejection,
        Err(PublicationError::Check("semantic rejection"))
    ));
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = batch.publish_checked(&mut 100, |_| -> Result<(), ()> { panic!("checker panic") });
    }));
    assert!(panic.is_err());
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    let mut called = false;
    let rejection = batch.publish_checked(&mut 0, |_| {
        called = true;
        Ok::<_, ()>(())
    });
    assert!(matches!(rejection, Err(PublicationError::Structure(_))));
    assert!(!called);
    let (owners, count) = batch
        .publish_checked(&mut 100, |candidate| Ok::<_, ()>(candidate.num_defs()))
        .unwrap();
    assert_eq!(owners, vec![owner]);
    assert_eq!(count, 1);
    assert_eq!(batch.published().num_defs(), 1);
    assert_eq!(
        batch.reserve().unwrap().def_id.raw(),
        owner.def_id.raw() + 1
    );
}

#[test]
fn generated_attributes_keep_order_and_validate_context_before_publication() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let value = arena.alloc_expr(Expr {
        hir_id: HirId::new(owner, ItemLocalId::new(1)),
        kind: ExprKind::TyVoid,
        span: DUMMY_SP,
    });
    batch
        .define(module(&arena, owner, vec![]), None, None)
        .unwrap();
    let attribute = |name: &str| DeferredDefinitionAttribute {
        target: AttributeTarget::Definition(owner),
        lexical_owner: owner,
        expression: name.into(),
        value,
        configuration_key: None,
        span: DUMMY_SP,
    };
    let mut invalid = attribute("invalid context");
    invalid.lexical_owner = OwnerId::INVALID;
    batch.set_attributes(vec![invalid]);
    assert_eq!(
        batch.publish(&mut 100).unwrap_err().kind,
        "invalid attribute lexical context"
    );
    assert!(batch.published().definition_attributes.is_empty());
    let mut missing_member = attribute("missing member");
    missing_member.target = AttributeTarget::Member(value.hir_id);
    batch.set_attributes(vec![missing_member]);
    assert_eq!(
        batch.publish(&mut 100).unwrap_err().kind,
        "missing attribute member target"
    );
    batch.set_attributes(vec![attribute("inner"), attribute("outer")]);
    let before = batch.to_string();
    assert!(before.find("inner").unwrap() < before.find("outer").unwrap());
    assert!(batch.publish(&mut 0).is_err());
    assert_eq!(batch.to_string(), before);
    batch.publish(&mut 100).unwrap();
    let attributes = &batch.published().definition_attributes;
    assert_eq!(
        attributes
            .iter()
            .map(|a| a.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["inner", "outer"]
    );
    assert!(std::ptr::eq(attributes[0].value, value));
    batch.set_attributes(vec![attribute("modify old owner")]);
    assert_eq!(
        batch.publish(&mut 100).unwrap_err().kind,
        "attribute target not defined in this batch"
    );
    assert_eq!(batch.published().definition_attributes.len(), 2);
}

#[test]
fn clause_definition_aliases_reject_publication_and_allow_repair() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let first = DefId::new(0, 100);
    for second in [Some(first), Some(DefId::INVALID), Some(DefId::new(0, 101))] {
        let clauses =
            arena.alloc_clause_param_slice([Some(first), second].into_iter().enumerate().map(
                |(index, definition)| {
                    let name =
                        Ident::new(Symbol::intern(if index == 0 { "T" } else { "U" }), DUMMY_SP);
                    ClauseParam {
                        hir_id: HirId::new(owner, ItemLocalId::new(index as u32 + 1)),
                        definition,
                        name,
                        kind: ClauseParamKind::Type(name),
                        span: DUMMY_SP,
                    }
                },
            ));
        let mut sig = signature();
        sig.clause_params = clauses;
        batch
            .define(
                info(&arena, owner, ItemKind::FnDeclaration(sig)),
                None,
                None,
            )
            .unwrap();
        if second == Some(DefId::new(0, 101)) {
            assert_eq!(batch.publish(&mut 100).unwrap(), vec![owner]);
            assert_eq!(batch.published().num_defs(), 1);
        } else {
            assert_eq!(
                batch.publish(&mut 100).unwrap_err().kind,
                "invalid or duplicate clause definition identity"
            );
            assert_eq!(batch.published().num_defs(), 0);
        }
    }
}

#[test]
fn clause_identities_are_unique_across_batches_and_owner_declarations() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let first = batch.reserve().unwrap();
    let second = batch.reserve().unwrap();
    let make = |owner, identity| {
        let clauses = arena.alloc_clause_param_slice([ClauseParam {
            hir_id: HirId::new(owner, ItemLocalId::new(1)),
            definition: Some(identity),
            name: Ident::new(Symbol::intern("T"), DUMMY_SP),
            kind: ClauseParamKind::Type(Ident::new(Symbol::intern("T"), DUMMY_SP)),
            span: DUMMY_SP,
        }]);
        let mut sig = signature();
        sig.clause_params = clauses;
        info(&arena, owner, ItemKind::FnDeclaration(sig))
    };
    let input = DefId::new(0, 100);
    batch.define(make(first, input), None, None).unwrap();
    batch.define(make(second, input), None, None).unwrap();
    assert_eq!(
        batch.publish(&mut 100).unwrap_err().kind,
        "clause definition identity is already declared"
    );
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(make(second, DefId::new(0, 101)), None, None)
        .unwrap();
    assert_eq!(batch.publish(&mut 100).unwrap(), vec![first, second]);
    let third = batch.reserve().unwrap();
    batch.define(make(third, input), None, None).unwrap();
    assert_eq!(
        batch.publish(&mut 100).unwrap_err().kind,
        "clause definition identity is already declared"
    );
    batch
        .define(module(&arena, third, vec![]), Some(input), None)
        .unwrap();
    assert_eq!(
        batch.publish(&mut 100).unwrap_err().kind,
        "invalid or duplicate definition identity"
    );
    assert_eq!(batch.published().num_defs(), 2);
    batch
        .define(
            make(third, DefId::new(0, 102)),
            Some(DefId::new(0, 103)),
            None,
        )
        .unwrap();
    assert_eq!(batch.publish(&mut 0).unwrap_err().kind, "budget exhausted");
    assert_eq!(batch.published().num_defs(), 2);
    assert_eq!(batch.publish(&mut 100).unwrap(), vec![third]);
}

#[test]
fn repeated_small_publications_do_not_rescan_existing_clauses() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let mut expected_cost = None;
    for index in 0..256 {
        let owner = batch.reserve().unwrap();
        let clauses = arena.alloc_clause_param_slice([ClauseParam {
            hir_id: HirId::new(owner, ItemLocalId::new(1)),
            definition: Some(DefId::new(0, 1000 + index)),
            name: Ident::new(Symbol::intern("T"), DUMMY_SP),
            kind: ClauseParamKind::Type(Ident::new(Symbol::intern("T"), DUMMY_SP)),
            span: DUMMY_SP,
        }]);
        let mut sig = signature();
        sig.clause_params = clauses;
        batch
            .define(
                info(&arena, owner, ItemKind::FnDeclaration(sig)),
                None,
                None,
            )
            .unwrap();
        let mut fuel = 32;
        batch.publish(&mut fuel).unwrap();
        assert_eq!(*expected_cost.get_or_insert(32 - fuel), 32 - fuel);
    }
    assert_eq!(batch.published().num_defs(), 256);
}

#[test]
fn replacing_source_owners_releases_only_their_clause_identities() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let identity = DefId::new(0, 1000);
    let first = package.alloc_owner_id();
    let second = package.alloc_owner_id();
    for owner in [first, second] {
        let clauses = arena.alloc_clause_param_slice([ClauseParam {
            hir_id: HirId::new(owner, ItemLocalId::new(1)),
            definition: Some(identity),
            name: Ident::new(Symbol::intern("T"), DUMMY_SP),
            kind: ClauseParamKind::Type(Ident::new(Symbol::intern("T"), DUMMY_SP)),
            span: DUMMY_SP,
        }]);
        let mut sig = signature();
        sig.clause_params = clauses;
        package.insert_owner(owner, info(&arena, owner, ItemKind::FnDeclaration(sig)));
    }
    package.insert_owner(first, module(&arena, first, vec![]));
    {
        let mut batch = DefinitionBatch::new(&mut package);
        let owner = batch.reserve().unwrap();
        batch
            .define(module(&arena, owner, vec![]), Some(identity), None)
            .unwrap();
        assert_eq!(
            batch.publish(&mut 100).unwrap_err().kind,
            "invalid or duplicate definition identity"
        );
        assert_eq!(batch.published().num_defs(), 2);
    }
    package.insert_owner(second, module(&arena, second, vec![]));
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(module(&arena, owner, vec![]), Some(identity), None)
        .unwrap();
    assert_eq!(batch.publish(&mut 100).unwrap(), vec![owner]);
}

#[test]
fn generated_identities_continue_source_frontier_and_publish_transactionally() {
    let arena = HirArena::new();
    let mut package = Package::new();
    package.reserve_definition_prefix(200);
    package.reserve_definition_prefix(10);
    assert_eq!(package.next_definition_index(), 200);
    {
        let mut batch = DefinitionBatch::new(&mut package);
        assert!(batch.reserve_identity(3, &mut 0).is_err());
        assert_eq!(
            batch.reserve_identity(3, &mut 1).unwrap(),
            DefId::new(3, 200)
        );
        assert_eq!(batch.published().next_definition_index(), 200);
    }
    let mut batch = DefinitionBatch::new(&mut package);
    let identity = batch.reserve_identity(3, &mut 1).unwrap();
    assert_eq!(identity, DefId::new(3, 200));
    let owner = batch.reserve().unwrap();
    batch
        .define(module(&arena, owner, vec![]), Some(identity), None)
        .unwrap();
    let before = batch.to_string();
    let rejected = batch.publish_checked(&mut 100, |_| Err::<(), _>("reject"));
    assert!(matches!(
        rejected,
        Err(hir::definitions::PublicationError::Check("reject"))
    ));
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().next_definition_index(), 200);
    batch
        .publish_checked(&mut 100, |_| Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(batch.published().next_definition_index(), 201);
    assert_eq!(
        batch.reserve_identity(9, &mut 1).unwrap(),
        DefId::new(9, 201)
    );
    drop(batch);
    let mut next = DefinitionBatch::new(&mut package);
    assert_eq!(
        next.reserve_identity(3, &mut 1).unwrap(),
        DefId::new(3, 201)
    );
    next.publish(&mut 100).unwrap();
    assert_eq!(next.published().next_definition_index(), 202);
}

#[test]
fn explicit_draft_identities_advance_allocator_and_exhaustion_never_wraps() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(
            module(&arena, owner, vec![]),
            Some(DefId::new(0, 500)),
            None,
        )
        .unwrap();
    assert_eq!(
        batch.reserve_identity(0, &mut 1).unwrap(),
        DefId::new(0, 501)
    );
    batch.publish(&mut 100).unwrap();
    drop(batch);
    package.reserve_definition_prefix(u32::MAX);
    let mut batch = DefinitionBatch::new(&mut package);
    for _ in 0..2 {
        assert_eq!(
            batch.reserve_identity(0, &mut 1).unwrap_err().kind,
            "definition identity exhausted"
        );
    }
    assert_eq!(batch.published().next_definition_index(), u32::MAX);
}

#[test]
fn prepared_publication_is_private_until_commit_and_checks_only_once() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(module(&arena, owner, vec![]), None, None)
        .unwrap();
    let before = batch.to_string();
    let calls = std::cell::Cell::new(0);
    let prepared = batch
        .prepare_checked(&mut 100, |candidate| {
            calls.set(calls.get() + 1);
            Ok::<_, ()>(candidate.num_defs())
        })
        .unwrap();
    assert_eq!(prepared.published().num_defs(), 0);
    assert_eq!(prepared.candidate().num_defs(), 1);
    assert_eq!(prepared.owners(), &[owner]);
    assert_eq!(*prepared.checked(), 1);
    assert!(prepared.to_string().contains("(candidate-definitions 1)"));
    drop(prepared);
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    let prepared = batch
        .prepare_checked(&mut 100, |_| {
            calls.set(calls.get() + 1);
            Ok::<_, ()>(17usize)
        })
        .unwrap()
        .map_checked(std::sync::Arc::new);
    let checked = prepared.checked().clone();
    let (owners, result) = prepared.commit();
    assert_eq!(owners, vec![owner]);
    assert!(std::sync::Arc::ptr_eq(&checked, &result));
    assert_eq!(*result, 17);
    assert_eq!(calls.get(), 2);
    assert_eq!(batch.published().num_defs(), 1);
    assert_eq!(
        batch.reserve().unwrap().def_id.raw(),
        owner.def_id.raw() + 1
    );
}

#[test]
fn checked_output_transfer_unwind_leaves_the_draft_repairable() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(module(&arena, owner, vec![]), None, None)
        .unwrap();
    let before = batch.to_string();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let prepared = batch
            .prepare_checked(&mut 100, |_| Ok::<_, ()>(()))
            .unwrap();
        let _ = prepared.map_checked(|_| -> () { panic!("storage transfer failed") });
    }));
    assert!(panic.is_err());
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .prepare_checked(&mut 100, |_| Ok::<_, ()>(()))
        .unwrap()
        .commit();
    assert_eq!(batch.published().num_defs(), 1);
}
