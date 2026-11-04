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

#[test]
fn clause_domains_share_publication_fuel_for_normal_and_contextual_functions() {
    use hir::definitions::{DefinitionNodes, assemble_function};
    for (optional, category) in [false, true]
        .into_iter()
        .flat_map(|optional| (0..3).map(move |category| (optional, category)))
    {
        let arena = HirArena::new();
        let mut package = Package::new();
        let mut batch = DefinitionBatch::new(&mut package);
        let owner = batch.reserve().unwrap();
        let definition = assemble_function(
            &arena,
            owner,
            Ident::new(Symbol::intern("Flagged"), DUMMY_SP),
            DUMMY_SP,
            &mut 200,
            |nodes: &mut DefinitionNodes<'_>, fuel| {
                let root = nodes.root();
                let clause = nodes.reserve(root, fuel)?;
                let domain = arena.alloc_expr(Expr {
                    hir_id: nodes.reserve(clause, fuel)?,
                    span: DUMMY_SP,
                    kind: ExprKind::Builtin(Symbol::intern("bool")),
                });
                nodes.register(Node::Expr(domain), fuel)?;
                let name = Ident::new(Symbol::intern("flag"), DUMMY_SP);
                let kind = if optional {
                    let default = arena.alloc_expr(Expr {
                        hir_id: nodes.reserve(clause, fuel)?,
                        span: DUMMY_SP,
                        kind: ExprKind::Lit(Lit {
                            kind: LitKind::Bool(true),
                            span: DUMMY_SP,
                        }),
                    });
                    nodes.register(Node::Expr(default), fuel)?;
                    ClauseParamKind::Optional(name, domain, default)
                } else {
                    ClauseParamKind::Positional(name, domain)
                };
                let clauses = arena.alloc_clause_param_slice(vec![ClauseParam {
                    hir_id: clause,
                    definition: Some(hir::common::DefId::new(0, 10_001)),
                    name,
                    kind,
                    span: DUMMY_SP,
                }]);
                nodes.register(Node::ClauseParam(&clauses[0]), fuel)?;
                let result_id = nodes.reserve(root, fuel)?;
                let result_kind = if category == 0 {
                    ExprKind::Builtin(Symbol::intern("Type"))
                } else {
                    let domain = arena.alloc_expr(Expr {
                        hir_id: nodes.reserve(result_id, fuel)?,
                        span: DUMMY_SP,
                        kind: ExprKind::Builtin(Symbol::intern("bool")),
                    });
                    nodes.register(Node::Expr(domain), fuel)?;
                    if category == 1 {
                        ExprKind::ContextualExprType(vec![], domain)
                    } else {
                        ExprKind::ContextualPatternType(domain, vec![])
                    }
                };
                let result = arena.alloc_expr(Expr {
                    hir_id: result_id,
                    span: DUMMY_SP,
                    kind: result_kind,
                });
                nodes.register(Node::Expr(result), fuel)?;
                let body_id = nodes.reserve(root, fuel)?;
                let body_kind = match category {
                    0 => ExprKind::Builtin(Symbol::intern("bool")),
                    1 => {
                        let value = arena.alloc_expr(Expr {
                            hir_id: nodes.reserve(body_id, fuel)?,
                            span: DUMMY_SP,
                            kind: ExprKind::Lit(Lit {
                                kind: LitKind::Bool(true),
                                span: DUMMY_SP,
                            }),
                        });
                        nodes.register(Node::Expr(value), fuel)?;
                        ExprKind::CodeQuote(value, vec![])
                    }
                    _ => {
                        let pattern = arena.alloc_pattern(Pattern {
                            hir_id: nodes.reserve(body_id, fuel)?,
                            span: DUMMY_SP,
                            kind: PatternKind::Wild,
                        });
                        nodes.register(Node::Pattern(pattern), fuel)?;
                        ExprKind::CodePatternQuote(pattern, vec![])
                    }
                };
                let body = arena.alloc_expr(Expr {
                    hir_id: body_id,
                    span: DUMMY_SP,
                    kind: body_kind,
                });
                Ok((
                    FnSig {
                        params: &[],
                        return_ty: Some(result),
                        return_bind: None,
                        return_binding: None,
                        modifiers: hir::item::FnModifiers {
                            is_pure: true,
                            is_comptime: true,
                            is_normal_form_head: category == 0,
                            ..Default::default()
                        },
                        clause_params: clauses,
                        clause_constraints: &[],
                        span: DUMMY_SP,
                    },
                    body,
                ))
            },
        )
        .unwrap();
        batch
            .define(
                definition.definition,
                Some(hir::common::DefId::new(0, owner.def_id.raw())),
                None,
            )
            .unwrap();
        batch.body(definition.body_id, definition.body).unwrap();
        let identities = BTreeMap::from([(0, 73)]);
        let before = batch.to_string();
        // Contextual functions prepare a generator checkpoint before checking
        // domains. Find the domain boundary without fixing its preparation
        // cost in this test; every rejected attempt must remain unpublished.
        let mut observed_domain_failure = false;
        for allowance in 0..1024 {
            let mut steps = allowance;
            let rejected = publish_with_invocation_budget(
                &mut batch,
                &mut 10_000,
                &mut steps,
                &identities,
                &Protocols::default(),
                None,
            );
            let reason = format!("{:?}", rejected.unwrap_err());
            assert!(reason.contains("budget exhausted"), "{reason}");
            assert!(steps <= allowance);
            assert_eq!(batch.to_string(), before);
            assert_eq!(batch.published().num_defs(), 0);
            if reason.contains("comptime input domain inspection budget exhausted") {
                observed_domain_failure = true;
                break;
            }
        }
        assert!(
            observed_domain_failure,
            "category {category}: domain check did not consume fuel"
        );
        let mut steps = 100_000;
        let (_, checked) = publish_with_invocation_budget(
            &mut batch,
            &mut 10_000,
            &mut steps,
            &identities,
            &Protocols::default(),
            None,
        )
        .unwrap();
        assert_eq!(batch.published().num_defs(), 1);
        if category == 0 {
            assert_eq!(checked.normal_forms.len(), 1);
        }
        assert!(steps < 100_000);
    }
}
