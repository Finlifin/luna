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
