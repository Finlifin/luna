use elaboration::{definitions::publish, program::Protocols};
use hir::definitions::{DefinitionBatch, PublicationError};
use hir::*;
use nir::{Primitive, Term};
use rustc_span::DUMMY_SP;
use std::{collections::BTreeMap, sync::Arc};

fn description<'a>(
    arena: &'a HirArena,
    owner: OwnerId,
    ty: Term,
    default: Option<LitKind>,
) -> OwnerInfo<'a> {
    let id = |local| HirId::new(owner, ItemLocalId::new(local));
    let ty = arena.alloc_expr(Expr {
        hir_id: id(2),
        kind: ExprKind::DeliveredType(Arc::new(ty)),
        span: DUMMY_SP,
    });
    let default = default.map(|kind| {
        arena.alloc_expr(Expr {
            hir_id: id(3),
            kind: ExprKind::Lit(Lit {
                kind,
                span: DUMMY_SP,
            }),
            span: DUMMY_SP,
        })
    });
    OwnerInfo {
        node: OwnerNode::Item(arena.alloc_item(Item {
            owner_id: owner,
            ident: Ident::new(Symbol::intern("Generated"), DUMMY_SP),
            kind: ItemKind::Struct(StructDef {
                fields: arena.alloc_field_def_slice(vec![FieldDef {
                    hir_id: id(1),
                    ident: Ident::new(Symbol::intern("value"), DUMMY_SP),
                    ty,
                    default,
                    span: DUMMY_SP,
                }]),
                clause_params: &[],
                clause_constraints: &[],
                nested_items: vec![],
            }),
            span: DUMMY_SP,
        })),
        nodes: OwnerNodes::new(),
    }
}
fn identities() -> BTreeMap<u32, u64> {
    BTreeMap::from([(0, 73)])
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}

#[test]
fn delivered_types_keep_default_checking_and_survive_later_publication() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(
            description(&arena, owner, boolean(), Some(LitKind::Char('x'))),
            None,
            None,
        )
        .unwrap();
    let before = batch.to_string();
    assert!(matches!(
        publish(
            &mut batch,
            &mut 100,
            &identities(),
            &Protocols::default(),
            None
        ),
        Err(PublicationError::Check(_))
    ));
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(
            description(&arena, owner, boolean(), Some(LitKind::Bool(true))),
            None,
            None,
        )
        .unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(checked.structs[0].fields[0].1, boolean());
    let nominal = Term::Nominal {
        declaration: checked.structs[0].declaration,
        arguments: vec![],
    };
    drop(batch);
    let mut next = DefinitionBatch::new(&mut package);
    let owner = next.reserve().unwrap();
    next.define(
        description(&arena, owner, nominal.clone(), None),
        None,
        None,
    )
    .unwrap();
    let (_, checked) = publish(
        &mut next,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(checked.structs.len(), 2);
    assert_eq!(checked.structs[1].fields[0].1, nominal);
}

#[test]
fn malformed_or_foreign_type_inputs_reject_atomically_and_can_be_repaired() {
    let bad = [
        Term::Bool(true),
        Term::Hole {
            index: 0,
            sort: nir::Sort::Type,
        },
        Term::Rigid {
            declaration: nir::DeclarationId {
                package: 73,
                local: 999,
            },
            sort: nir::Sort::Type,
        },
        Term::Nominal {
            declaration: nir::DeclarationId {
                package: 99,
                local: 999,
            },
            arguments: vec![],
        },
        Term::NormalForm {
            head: nir::DeclarationId {
                package: 99,
                local: 999,
            },
            sort: nir::Sort::Type,
            arguments: vec![],
        },
    ];
    for ty in bad {
        let arena = HirArena::new();
        let mut package = Package::new();
        let mut batch = DefinitionBatch::new(&mut package);
        let owner = batch.reserve().unwrap();
        batch
            .define(description(&arena, owner, ty.clone(), None), None, None)
            .unwrap();
        let before = batch.to_string();
        assert!(
            publish(
                &mut batch,
                &mut 100,
                &identities(),
                &Protocols::default(),
                None
            )
            .is_err(),
            "accepted {ty}"
        );
        assert_eq!(batch.published().num_defs(), 0);
        assert_eq!(batch.to_string(), before);
        batch
            .define(description(&arena, owner, boolean(), None), None, None)
            .unwrap();
        publish(
            &mut batch,
            &mut 100,
            &identities(),
            &Protocols::default(),
            None,
        )
        .unwrap();
    }
}

#[test]
fn reflected_type_values_can_feed_generated_fields_without_source_reconstruction() {
    let mut source = nir::program::Package::default();
    let declaration = nir::DeclarationId {
        package: 12,
        local: 0,
    };
    source.structs.push(nir::program::Struct {
        declaration,
        name: "Source".into(),
        fields: vec![("flag".into(), boolean())],
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
    });
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        &source,
        comptime_interpretor::Budget::default(),
    )
    .unwrap();
    let reflected = vm
        .reflect_struct(&Term::Nominal {
            declaration,
            arguments: vec![],
        })
        .unwrap();
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let fields: Vec<_> = reflected
        .fields
        .iter()
        .map(|field| hir::definitions::TypedField {
            name: Ident::new(Symbol::intern(&field.name), DUMMY_SP),
            ty: Arc::new(field.ty.clone()),
            span: DUMMY_SP,
        })
        .collect();
    let info = hir::definitions::assemble_struct(
        &arena,
        owner,
        Ident::new(Symbol::intern("Generated"), DUMMY_SP),
        &fields,
        DUMMY_SP,
        &mut 100,
    )
    .unwrap();
    batch.define(info, None, None).unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(checked.structs[0].fields[0].1, reflected.fields[0].ty);
}

#[test]
fn delivered_normal_form_preserves_head_and_rejects_body_type_default() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let head = batch.reserve().unwrap();
    let owner = batch.reserve().unwrap();
    let id = |local| HirId::new(head, ItemLocalId::new(local));
    let result = arena.alloc_expr(Expr {
        hir_id: id(1),
        kind: ExprKind::TyType,
        span: DUMMY_SP,
    });
    let value = arena.alloc_expr(Expr {
        hir_id: id(2),
        kind: ExprKind::Builtin(Symbol::intern("bool")),
        span: DUMMY_SP,
    });
    let body = BodyId::new(id(2));
    batch.body(body, Body { params: &[], value }).unwrap();
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(Item {
                    owner_id: head,
                    ident: Ident::new(Symbol::intern("Token"), DUMMY_SP),
                    span: DUMMY_SP,
                    kind: ItemKind::Fn(
                        FnSig {
                            params: &[],
                            return_ty: Some(result),
                            return_bind: None,
                            return_binding: None,
                            modifiers: hir::item::FnModifiers {
                                is_pure: true,
                                is_comptime: true,
                                is_normal_form_head: true,
                                ..Default::default()
                            },
                            clause_params: &[],
                            clause_constraints: &[],
                            span: DUMMY_SP,
                        },
                        body,
                    ),
                })),
                nodes: OwnerNodes::new(),
            },
            None,
            None,
        )
        .unwrap();
    let opaque = Term::NormalForm {
        head: nir::DeclarationId {
            package: 73,
            local: head.def_id.raw(),
        },
        sort: nir::Sort::Type,
        arguments: vec![],
    };
    batch
        .define(
            description(&arena, owner, opaque.clone(), Some(LitKind::Bool(true))),
            None,
            None,
        )
        .unwrap();
    assert!(
        publish(
            &mut batch,
            &mut 100,
            &identities(),
            &Protocols::default(),
            None
        )
        .is_err()
    );
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(description(&arena, owner, opaque.clone(), None), None, None)
        .unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(checked.structs[0].fields[0].1, opaque);
    assert_ne!(checked.structs[0].fields[0].1, boolean());
}

#[test]
fn destination_formation_predicates_are_checked_before_delivery() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let constrained = batch.reserve().unwrap();
    let owner = batch.reserve().unwrap();
    let predicate = arena.alloc_expr(Expr {
        hir_id: HirId::new(constrained, ItemLocalId::new(1)),
        kind: ExprKind::Lit(Lit {
            kind: LitKind::Bool(false),
            span: DUMMY_SP,
        }),
        span: DUMMY_SP,
    });
    let constraints = [ClauseConstraint {
        label: None,
        hir_id: HirId::new(constrained, ItemLocalId::new(2)),
        kind: ClauseConstraintKind::Requires(predicate),
        span: DUMMY_SP,
    }];
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(Item {
                    owner_id: constrained,
                    ident: Ident::new(Symbol::intern("Unavailable"), DUMMY_SP),
                    span: DUMMY_SP,
                    kind: ItemKind::Struct(StructDef {
                        fields: &[],
                        clause_params: &[],
                        clause_constraints: &constraints,
                        nested_items: vec![],
                    }),
                })),
                nodes: OwnerNodes::new(),
            },
            None,
            None,
        )
        .unwrap();
    let ty = Term::Nominal {
        declaration: nir::DeclarationId {
            package: 73,
            local: constrained.def_id.raw(),
        },
        arguments: vec![],
    };
    batch
        .define(description(&arena, owner, ty, None), None, None)
        .unwrap();
    let failure = publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap_err();
    assert!(
        failure
            .to_string()
            .contains("delivered type admission failed"),
        "{failure}"
    );
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(description(&arena, owner, boolean(), None), None, None)
        .unwrap();
    publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
}

#[test]
fn assembled_structs_use_ordinary_name_and_type_validation() {
    use hir::definitions::{TypedField, assemble_struct};
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let field = |name: &str, ty| TypedField {
        name: Ident::new(Symbol::intern(name), DUMMY_SP),
        ty: Arc::new(ty),
        span: DUMMY_SP,
    };
    let assemble = |fields: &[TypedField]| {
        assemble_struct(
            &arena,
            owner,
            Ident::new(Symbol::intern("Generated"), DUMMY_SP),
            fields,
            DUMMY_SP,
            &mut 100,
        )
        .unwrap()
    };
    batch
        .define(
            assemble(&[field("value", boolean()), field("value", boolean())]),
            None,
            None,
        )
        .unwrap();
    let before = batch.to_string();
    let failure = publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap_err();
    assert!(
        failure.to_string().contains("duplicate struct field"),
        "{failure}"
    );
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(assemble(&[field("value", Term::Bool(true))]), None, None)
        .unwrap();
    assert!(
        publish(
            &mut batch,
            &mut 100,
            &identities(),
            &Protocols::default(),
            None
        )
        .is_err()
    );
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(assemble(&[field("value", boolean())]), None, None)
        .unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(checked.structs[0].fields, [("value".into(), boolean())]);
}

#[test]
fn assembled_defaults_use_cmtt_splicing_and_ordinary_field_type_checking() {
    use hir::definitions::{TypedField, assemble_struct_with_defaults};
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let inputs = [TypedField {
        name: Ident::new(Symbol::intern("flag"), DUMMY_SP),
        ty: Arc::new(boolean()),
        span: DUMMY_SP,
    }];
    let assemble = |valid| {
        assemble_struct_with_defaults(
            &arena,
            owner,
            Ident::new(Symbol::intern("Generated"), DUMMY_SP),
            &inputs,
            DUMMY_SP,
            &mut 100,
            |_, field_id, nodes, fuel| {
                let root_id = nodes.reserve(field_id, fuel)?;
                let quote_id = nodes.reserve(root_id, fuel)?;
                let value_id = nodes.reserve(quote_id, fuel)?;
                let value = arena.alloc_expr(Expr {
                    hir_id: value_id,
                    span: DUMMY_SP,
                    kind: ExprKind::Lit(Lit {
                        kind: if valid {
                            LitKind::Bool(true)
                        } else {
                            LitKind::Char('x')
                        },
                        span: DUMMY_SP,
                    }),
                });
                nodes.register(Node::Expr(value), fuel)?;
                let quote = arena.alloc_expr(Expr {
                    hir_id: quote_id,
                    span: DUMMY_SP,
                    kind: ExprKind::CodeQuote(value, vec![]),
                });
                nodes.register(Node::Expr(quote), fuel)?;
                Ok(Some(arena.alloc_expr(Expr {
                    hir_id: root_id,
                    span: DUMMY_SP,
                    kind: ExprKind::CodeSplice(quote, vec![]),
                })))
            },
        )
        .unwrap()
    };
    let consumer = batch.reserve().unwrap();
    let id = |local| HirId::new(consumer, ItemLocalId::new(local));
    let return_ty = arena.alloc_expr(Expr {
        hir_id: id(4),
        span: DUMMY_SP,
        kind: ExprKind::Builtin(Symbol::intern("bool")),
    });
    let base = arena.alloc_expr(Expr {
        hir_id: id(3),
        span: DUMMY_SP,
        kind: ExprKind::DeliveredType(Arc::new(Term::Nominal {
            declaration: nir::DeclarationId {
                package: 73,
                local: owner.def_id.raw(),
            },
            arguments: vec![],
        })),
    });
    let construction = arena.alloc_expr(Expr {
        hir_id: id(2),
        span: DUMMY_SP,
        kind: ExprKind::ExtendedApplication(base, &[]),
    });
    let body_expr = arena.alloc_expr(Expr {
        hir_id: id(1),
        span: DUMMY_SP,
        kind: ExprKind::Projection(construction, Ident::new(Symbol::intern("flag"), DUMMY_SP)),
    });
    let body = BodyId::new(id(1));
    batch
        .body(
            body,
            Body {
                params: &[],
                value: body_expr,
            },
        )
        .unwrap();
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(Item {
                    owner_id: consumer,
                    ident: Ident::new(Symbol::intern("run"), DUMMY_SP),
                    span: DUMMY_SP,
                    kind: ItemKind::Fn(
                        FnSig {
                            params: &[],
                            return_ty: Some(return_ty),
                            return_bind: None,
                            return_binding: None,
                            modifiers: Default::default(),
                            clause_params: &[],
                            clause_constraints: &[],
                            span: DUMMY_SP,
                        },
                        body,
                    ),
                })),
                nodes: OwnerNodes::new(),
            },
            None,
            None,
        )
        .unwrap();
    batch.define(assemble(false), None, None).unwrap();
    let before = batch.to_string();
    assert!(
        publish(
            &mut batch,
            &mut 100,
            &identities(),
            &Protocols::default(),
            None
        )
        .is_err()
    );
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    batch.define(assemble(true), None, None).unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 100,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(checked.structs[0].fields[0].1, boolean());
    let mut vm =
        comptime_interpretor::Interpreter::new(&checked, comptime_interpretor::Budget::default())
            .unwrap();
    assert_eq!(
        vm.invoke(
            nir::DeclarationId {
                package: 73,
                local: consumer.def_id.raw()
            },
            vec![]
        )
        .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn publication_reuses_invocation_budget_and_preserves_failed_attempt_consumption() {
    use elaboration::definitions::publish_with_invocation_budget;
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(description(&arena, owner, boolean(), None), None, None)
        .unwrap();
    let before = batch.to_string();
    let protocols = Protocols::default();
    let mut steps = 10_000;
    assert!(matches!(
        publish_with_invocation_budget(
            &mut batch,
            &mut 0,
            &mut steps,
            &identities(),
            &protocols,
            None
        ),
        Err(PublicationError::Structure(_))
    ));
    assert_eq!(steps, 10_000);
    assert_eq!(batch.to_string(), before);
    publish_with_invocation_budget(
        &mut batch,
        &mut 1000,
        &mut steps,
        &identities(),
        &protocols,
        None,
    )
    .unwrap();
    let spent = 10_000 - steps;
    assert!(spent > 0);
    let after_first = steps;
    publish_with_invocation_budget(
        &mut batch,
        &mut 1000,
        &mut steps,
        &identities(),
        &protocols,
        None,
    )
    .unwrap();
    assert_eq!(after_first - steps, spent);
    // Rechecking the same candidate with one fewer step must not silently gain
    // the default million-step allowance. Failed work also reduces the balance.
    steps = spent - 1;
    let available = steps;
    let before = batch.to_string();
    assert!(matches!(
        publish_with_invocation_budget(
            &mut batch,
            &mut 1000,
            &mut steps,
            &identities(),
            &protocols,
            None
        ),
        Err(PublicationError::Check(_))
    ));
    assert!(steps < available, "remaining {steps}, initial {available}");
    assert_eq!(steps, 0, "failed final admission must not refund VM work");
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 1);
    // The owner stays published and no failed candidate replaces it. A caller
    // may explicitly grant more fuel, then retry without rebuilding its HIR.
    steps = 10_000;
    publish_with_invocation_budget(
        &mut batch,
        &mut 1000,
        &mut steps,
        &identities(),
        &protocols,
        None,
    )
    .unwrap();
    assert_eq!(10_000 - steps, spent);
}

#[test]
fn prepared_hir_and_vm_struct_installation_commit_together_without_rechecking() {
    use comptime_interpretor::{Budget, Interpreter, StructPublicationError};
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(description(&arena, owner, boolean(), None), None, None)
        .unwrap();
    let before = batch.to_string();
    let baseline = nir::program::Package::default();
    let mut vm = Interpreter::for_comptime(&baseline, Budget::default()).unwrap();
    let mut structural_fuel = 100_000;
    let mut invocation_steps = 1_000_000;
    let prepared = elaboration::definitions::prepare_with_invocation_budget(
        &mut batch,
        &mut structural_fuel,
        &mut invocation_steps,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap()
    .map_checked(Arc::new);
    let rejected_world = Arc::clone(prepared.checked());
    let generated = Term::Nominal {
        declaration: rejected_world.structs[0].declaration,
        arguments: vec![],
    };
    assert_eq!(prepared.published().num_defs(), 0);
    let rejected = vm.install_generated_structs_with(&rejected_world.structs, || {
        drop(prepared);
        Err::<(), _>("publisher declined")
    });
    assert!(matches!(
        rejected,
        Err(StructPublicationError::Commit("publisher declined"))
    ));
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    assert!(vm.reflect_struct(&generated).is_err());

    let prepared = elaboration::definitions::prepare_with_invocation_budget(
        &mut batch,
        &mut structural_fuel,
        &mut invocation_steps,
        &identities(),
        &Protocols::default(),
        None,
    )
    .unwrap()
    .map_checked(Arc::new);
    let checked = Arc::clone(prepared.checked());
    let fuel_before_commit = structural_fuel;
    let steps_before_commit = invocation_steps;
    let (owners, committed) = vm
        .install_generated_structs_with(&checked.structs, || Ok::<_, ()>(prepared.commit()))
        .unwrap();
    assert_eq!(owners, vec![owner]);
    assert!(Arc::ptr_eq(&checked, &committed));
    assert_eq!(structural_fuel, fuel_before_commit);
    assert_eq!(invocation_steps, steps_before_commit);
    assert_eq!(batch.published().num_defs(), 1);
    assert_eq!(
        vm.reflect_struct(&generated).unwrap().fields[0].ty,
        boolean()
    );
}
