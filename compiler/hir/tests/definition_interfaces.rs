use hir::definitions::{DefinitionNodes, Error, assemble_impl, assemble_trait};
use hir::*;
use rustc_span::DUMMY_SP;

fn owner(n: u32) -> OwnerId {
    OwnerId::new(LocalDefId::new(n))
}
fn name(text: &str) -> Ident {
    Ident::new(Symbol::intern(text), DUMMY_SP)
}
fn expression<'a>(
    arena: &'a HirArena,
    nodes: &mut DefinitionNodes<'a>,
    parent: HirId,
    kind: ExprKind<'a>,
    fuel: &mut usize,
) -> Result<&'a Expr<'a>, Error> {
    let value = arena.alloc_expr(Expr {
        hir_id: nodes.reserve(parent, fuel)?,
        kind,
        span: DUMMY_SP,
    });
    nodes.register(Node::Expr(value), fuel)?;
    Ok(value)
}

#[test]
fn trait_assembly_preserves_composite_super_traits_and_member_order() {
    let arena = HirArena::new();
    let info = assemble_trait(
        &arena,
        owner(1),
        name("Generated"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let root = nodes.root();
            let combined = nodes.reserve(root, fuel)?;
            let left = expression(
                &arena,
                nodes,
                combined,
                ExprKind::Builtin(Symbol::intern("A")),
                fuel,
            )?;
            let right = expression(
                &arena,
                nodes,
                combined,
                ExprKind::Builtin(Symbol::intern("B")),
                fuel,
            )?;
            let supers = arena.alloc_expr(Expr {
                hir_id: combined,
                kind: ExprKind::Binary(BinOp::Add, left, right),
                span: DUMMY_SP,
            });
            nodes.register(Node::Expr(supers), fuel)?;
            let parameters = arena.alloc_clause_param_slice(vec![ClauseParam {
                hir_id: nodes.reserve(root, fuel)?,
                definition: None,
                name: name("T"),
                kind: ClauseParamKind::Type(name("T")),
                span: DUMMY_SP,
            }]);
            nodes.register(Node::ClauseParam(&parameters[0]), fuel)?;
            let constraint = nodes.reserve(root, fuel)?;
            let condition = expression(
                &arena,
                nodes,
                constraint,
                ExprKind::Lit(Lit {
                    kind: LitKind::Bool(true),
                    span: DUMMY_SP,
                }),
                fuel,
            )?;
            let constraints = arena.alloc_clause_slice(vec![ClauseConstraint {
                hir_id: constraint,
                label: None,
                kind: ClauseConstraintKind::Requires(condition),
                span: DUMMY_SP,
            }]);
            nodes.register(Node::ClauseConstraint(&constraints[0]), fuel)?;
            Ok(TraitDef {
                super_traits: Some(supers),
                clause_params: parameters,
                clause_constraints: constraints,
                items: vec![owner(3), owner(2)],
            })
        },
    )
    .unwrap();
    let ItemKind::Trait(definition) = &info.node.expect_item().kind else {
        unreachable!()
    };
    assert_eq!(definition.items, [owner(3), owner(2)]);
    assert!(matches!(
        definition.clause_params[0].kind,
        ClauseParamKind::Type(_)
    ));
    let ClauseConstraintKind::Requires(condition) = definition.clause_constraints[0].kind else {
        unreachable!()
    };
    assert_eq!(
        info.nodes.get(condition.hir_id.local_id).unwrap().parent,
        definition.clause_constraints[0].hir_id.local_id
    );
    let super_traits = definition.super_traits.unwrap();
    let ExprKind::Binary(BinOp::Add, left, right) = &super_traits.kind else {
        unreachable!()
    };
    for value in [left, right] {
        assert_eq!(
            info.nodes.get(value.hir_id.local_id).unwrap().parent,
            super_traits.hir_id.local_id
        );
    }
    let dump = info.node.expect_item().to_string();
    assert!(dump.starts_with("("));
    assert!(dump.contains("Generated"));
}

#[test]
fn assembly_budget_and_unfinished_nodes_reject_without_publishing_a_descriptor() {
    let arena = HirArena::new();
    let mut called = false;
    let result = assemble_trait(
        &arena,
        owner(1),
        name("Generated"),
        DUMMY_SP,
        &mut 0,
        |_, _| {
            called = true;
            Ok(TraitDef {
                super_traits: None,
                clause_params: &[],
                clause_constraints: &[],
                items: vec![],
            })
        },
    );
    assert!(result.is_err());
    assert!(!called);
    let result = assemble_trait(
        &arena,
        owner(1),
        name("Generated"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            nodes.reserve(nodes.root(), fuel)?;
            Ok(TraitDef {
                super_traits: None,
                clause_params: &[],
                clause_constraints: &[],
                items: vec![],
            })
        },
    );
    assert_eq!(result.unwrap_err().kind, "unfinished node reservations");
    for fuel in [2, 3] {
        let result = assemble_trait(
            &arena,
            owner(1),
            name("Generated"),
            DUMMY_SP,
            &mut { fuel },
            |_, _| {
                Ok(TraitDef {
                    super_traits: None,
                    clause_params: &[],
                    clause_constraints: &[],
                    items: vec![owner(2)],
                })
            },
        );
        assert_eq!(result.is_ok(), fuel == 3);
    }
}

#[test]
fn implementation_assembly_preserves_receiver_and_optional_trait_reference() {
    let arena = HirArena::new();
    for is_trait in [false, true] {
        let info = assemble_impl(
            &arena,
            owner(1),
            name("generated_impl"),
            DUMMY_SP,
            &mut 100,
            |nodes, fuel| {
                let root = nodes.root();
                let self_ty = expression(
                    &arena,
                    nodes,
                    root,
                    ExprKind::Builtin(Symbol::intern("bool")),
                    fuel,
                )?;
                let trait_ref = if is_trait {
                    Some(expression(
                        &arena,
                        nodes,
                        root,
                        ExprKind::Builtin(Symbol::intern("Protocol")),
                        fuel,
                    )?)
                } else {
                    None
                };
                Ok(ImplDef {
                    self_ty,
                    trait_ref,
                    clause_params: &[],
                    clause_constraints: &[],
                    items: vec![owner(2)],
                })
            },
        )
        .unwrap();
        let ItemKind::Impl(definition) = &info.node.expect_item().kind else {
            unreachable!()
        };
        assert_eq!(definition.trait_ref.is_some(), is_trait);
        assert_eq!(definition.items, [owner(2)]);
        assert!(info.nodes.get(definition.self_ty.hir_id.local_id).is_some());
    }
}

#[test]
fn associated_assembly_registers_optional_parameters_and_rejects_bad_indices() {
    use hir::common::{FnParam, TyParam, TyParamKind};
    use hir::definitions::assemble_associated;
    use hir::item::AssociatedDecl;
    let arena = HirArena::new();
    for index in [1, 0] {
        let result = assemble_associated(
            &arena,
            owner(1),
            name("enabled"),
            DUMMY_SP,
            &mut 100,
            |nodes, fuel| {
                let root = nodes.root();
                let parameter = nodes.reserve(root, fuel)?;
                let ty = expression(
                    &arena,
                    nodes,
                    parameter,
                    ExprKind::Builtin(Symbol::intern("bool")),
                    fuel,
                )?;
                let default = expression(
                    &arena,
                    nodes,
                    parameter,
                    ExprKind::Lit(Lit {
                        kind: LitKind::Bool(true),
                        span: DUMMY_SP,
                    }),
                    fuel,
                )?;
                let parameters = arena.alloc_fn_param_slice(vec![FnParam {
                    hir_id: parameter,
                    name: name("value"),
                    index,
                    ty: TyParam::new(
                        parameter,
                        TyParamKind::Optional(name("value"), ty, default),
                        DUMMY_SP,
                    ),
                    span: DUMMY_SP,
                }]);
                let ty = expression(
                    &arena,
                    nodes,
                    root,
                    ExprKind::Builtin(Symbol::intern("bool")),
                    fuel,
                )?;
                Ok(AssociatedDecl {
                    parameters,
                    ty,
                    default: None,
                    clause_params: &[],
                    clause_constraints: &[],
                })
            },
        );
        if index != 0 {
            assert_eq!(
                result.unwrap_err().kind,
                "associated parameter identity or index is inconsistent"
            );
            continue;
        }
        let info = result.unwrap();
        let ItemKind::Associated(declaration) = &info.node.expect_item().kind else {
            unreachable!()
        };
        let parameter = &declaration.parameters[0];
        assert!(matches!(parameter.ty.kind, TyParamKind::Optional(..)));
        let entry = info.nodes.get(parameter.hir_id.local_id).unwrap();
        assert!(matches!(entry.node, Node::Param(_)));
        assert_eq!(entry.parent, ItemLocalId::new(0));
        assert!(declaration.default.is_none());
    }
}
