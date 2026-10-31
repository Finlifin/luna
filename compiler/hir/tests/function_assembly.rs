use hir::{
    common::{FnParam, TyParam, TyParamKind},
    definitions::{DefinitionBatch, DefinitionNodes, Error, assemble_function},
    *,
};
use rustc_span::DUMMY_SP;

fn ident(text: &str) -> Ident {
    Ident::new(Symbol::intern(text), DUMMY_SP)
}
fn registered<'a>(
    arena: &'a HirArena,
    nodes: &mut DefinitionNodes<'a>,
    parent: HirId,
    kind: ExprKind<'a>,
    fuel: &mut usize,
) -> Result<&'a Expr<'a>, Error> {
    let hir_id = nodes.reserve(parent, fuel)?;
    let node = arena.alloc_expr(Expr {
        hir_id,
        kind,
        span: DUMMY_SP,
    });
    nodes.register(Node::Expr(node), fuel)?;
    Ok(node)
}
fn construct<'a>(
    arena: &'a HirArena,
    nodes: &mut DefinitionNodes<'a>,
    fuel: &mut usize,
    invalid: u8,
) -> Result<(FnSig<'a>, &'a Expr<'a>), Error> {
    let root = nodes.root();
    let type_id = nodes.reserve(root, fuel)?;
    let value_id = nodes.reserve(root, fuel)?;
    let type_type = registered(arena, nodes, type_id, ExprKind::TyType, fuel)?;
    let value_type = registered(
        arena,
        nodes,
        value_id,
        ExprKind::ResolvedFnParam(Symbol::intern("T"), type_id),
        fuel,
    )?;
    let result = registered(
        arena,
        nodes,
        root,
        ExprKind::ResolvedFnParam(Symbol::intern("T"), type_id),
        fuel,
    )?;
    let params = arena.alloc_fn_param_slice(vec![
        FnParam {
            template_annotation: None,
            hir_id: type_id,
            name: ident("T"),
            index: 0,
            ty: TyParam::new(type_id, TyParamKind::Positional(type_type), DUMMY_SP).with_comptime(),
            span: DUMMY_SP,
        },
        FnParam {
            template_annotation: None,
            hir_id: value_id,
            name: ident("value"),
            index: if invalid == 1 { 0 } else { 1 },
            ty: TyParam::new(
                if invalid == 2 { type_id } else { value_id },
                TyParamKind::Positional(value_type),
                DUMMY_SP,
            ),
            span: DUMMY_SP,
        },
    ]);
    if invalid == 3 {
        nodes.reserve(root, fuel)?;
    }
    let body = arena.alloc_expr(Expr {
        hir_id: nodes.reserve(if invalid == 4 { type_id } else { root }, fuel)?,
        kind: ExprKind::ResolvedFnParam(Symbol::intern("value"), value_id),
        span: DUMMY_SP,
    });
    if invalid == 5 {
        nodes.register(Node::Expr(body), fuel)?;
    }
    Ok((
        FnSig {
            params,
            return_ty: Some(result),
            return_bind: None,
            return_binding: None,
            modifiers: hir::item::FnModifiers {
                is_pure: true,
                is_unsafe: true,
                ..Default::default()
            },
            clause_params: &[],
            clause_constraints: &[],
            span: DUMMY_SP,
        },
        body,
    ))
}

#[test]
fn signature_dependencies_and_body_bindings_share_one_complete_index() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let built = assemble_function(
        &arena,
        owner,
        ident("generated\n\""),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| construct(&arena, nodes, fuel, 0),
    )
    .unwrap();
    let ItemKind::Fn(signature, body_id) = &built.definition.node.expect_item().kind else {
        panic!()
    };
    assert_eq!(*body_id, built.body_id);
    assert_eq!(signature.params.len(), built.body.params.len());
    for (parameter, body) in signature.params.iter().zip(built.body.params) {
        assert_eq!(parameter.hir_id, body.hir_id);
        assert_eq!(parameter.ty, body.ty);
        assert!(matches!(
            built
                .definition
                .nodes
                .get(parameter.hir_id.local_id)
                .unwrap()
                .node,
            Node::Param(_)
        ));
    }
    assert!(signature.params[0].ty.is_comptime());
    assert!(signature.modifiers.is_unsafe);
    assert!(
        matches!(signature.return_ty.unwrap().kind, ExprKind::ResolvedFnParam(_, id) if id == signature.params[0].hir_id)
    );
    assert!(built.to_string().contains("generated\\n\\\""));
    assert_eq!(built.to_string(), built.to_string());
    batch.body(built.body_id, built.body).unwrap();
    batch.define(built.definition, None, None).unwrap();
    batch.publish(&mut 1000).unwrap();
    assert_eq!(batch.published().num_defs(), 1);
    assert_eq!(batch.published().num_bodies(), 1);
}

#[test]
fn invalid_indices_mirrors_parents_and_pending_nodes_do_not_escape_assembly() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    for invalid in 1..=5 {
        assert!(
            assemble_function(
                &arena,
                owner,
                ident("generated"),
                DUMMY_SP,
                &mut 100,
                |nodes, fuel| construct(&arena, nodes, fuel, invalid)
            )
            .is_err()
        );
        assert_eq!(batch.published().num_defs(), 0);
    }
    assert!(
        assemble_function(
            &arena,
            owner,
            ident("generated"),
            DUMMY_SP,
            &mut 100,
            |nodes, fuel| construct(&arena, nodes, fuel, 0)
        )
        .is_ok()
    );
}

#[test]
fn exhausted_budget_does_not_invoke_the_callback_and_can_be_replenished() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let failed = assemble_function(
        &arena,
        owner,
        ident("generated"),
        DUMMY_SP,
        &mut 0,
        |_, _| panic!("no construction fuel"),
    );
    assert_eq!(failed.unwrap_err().kind, "budget exhausted");
    assert!(
        assemble_function(
            &arena,
            owner,
            ident("generated"),
            DUMMY_SP,
            &mut 2,
            |nodes, fuel| construct(&arena, nodes, fuel, 0)
        )
        .is_err()
    );
    assert!(
        assemble_function(
            &arena,
            owner,
            ident("generated"),
            DUMMY_SP,
            &mut 100,
            |nodes, fuel| construct(&arena, nodes, fuel, 0)
        )
        .is_ok()
    );
}

#[test]
fn bodyless_contract_indexes_parameters_without_creating_a_body() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let mut fuel = 9;
    let definition = hir::definitions::assemble_function_declaration(
        &arena,
        owner,
        ident("required"),
        DUMMY_SP,
        &mut fuel,
        |nodes, fuel| {
            let root = nodes.root();
            let parameter = nodes.reserve(root, fuel)?;
            let ty = registered(
                &arena,
                nodes,
                parameter,
                ExprKind::Builtin(Symbol::intern("bool")),
                fuel,
            )?;
            let result = registered(
                &arena,
                nodes,
                root,
                ExprKind::Builtin(Symbol::intern("bool")),
                fuel,
            )?;
            let params = arena.alloc_fn_param_slice(vec![FnParam {
                template_annotation: None,
                hir_id: parameter,
                name: ident("value"),
                index: 0,
                ty: TyParam::new(parameter, TyParamKind::Positional(ty), DUMMY_SP),
                span: DUMMY_SP,
            }]);
            Ok(FnSig {
                params,
                return_ty: Some(result),
                return_bind: None,
                return_binding: None,
                modifiers: Default::default(),
                clause_params: &[],
                clause_constraints: &[],
                span: DUMMY_SP,
            })
        },
    )
    .unwrap();
    assert_eq!(fuel, 0);
    let ItemKind::FnDeclaration(signature) = &definition.node.expect_item().kind else {
        panic!()
    };
    let Node::Param(indexed) = definition
        .nodes
        .get(signature.params[0].hir_id.local_id)
        .unwrap()
        .node
    else {
        panic!()
    };
    assert_eq!(indexed.ty, signature.params[0].ty);
    batch.define(definition, None, None).unwrap();
    // Structural assembly is independent from the enclosing trait check.
    batch.publish(&mut 100).unwrap();
    assert_eq!(batch.published().num_bodies(), 0);
}
