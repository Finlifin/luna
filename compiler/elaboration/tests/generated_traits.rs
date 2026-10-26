use elaboration::{definitions::publish, program::Protocols};
use hir::{
    definitions::{
        DefinitionBatch, DefinitionNodes, Error, assemble_function, assemble_function_declaration,
    },
    *,
};
use rustc_span::DUMMY_SP;
use std::collections::BTreeMap;

fn name(s: &str) -> Ident {
    Ident::new(Symbol::intern(s), DUMMY_SP)
}
fn expr<'a>(
    arena: &'a HirArena,
    nodes: &mut DefinitionNodes<'a>,
    parent: HirId,
    kind: ExprKind<'a>,
    fuel: &mut usize,
) -> Result<&'a Expr<'a>, Error> {
    let node = arena.alloc_expr(Expr {
        hir_id: nodes.reserve(parent, fuel)?,
        kind,
        span: DUMMY_SP,
    });
    nodes.register(Node::Expr(node), fuel)?;
    Ok(node)
}
fn sig<'a>(result: &'a Expr<'a>) -> FnSig<'a> {
    FnSig {
        params: &[],
        return_ty: Some(result),
        return_bind: None,
        return_binding: None,
        modifiers: hir::item::FnModifiers {
            is_pure: true,
            ..Default::default()
        },
        clause_params: &[],
        clause_constraints: &[],
        span: DUMMY_SP,
    }
}
fn reference(owner: OwnerId) -> ExprKind<'static> {
    ExprKind::ResolvedDef {
        name: Symbol::intern("GeneratedProtocol"),
        has_clause_param: false,
        def_id: hir::common::DefId::new(0, owner.def_id.raw()),
    }
}
fn declaration<'a>(arena: &'a HirArena, owner: OwnerId, ty: &str) -> OwnerInfo<'a> {
    assemble_function_declaration(
        arena,
        owner,
        name("enabled"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let root = nodes.root();
            let result = expr(
                arena,
                nodes,
                root,
                ExprKind::Builtin(Symbol::intern(ty)),
                fuel,
            )?;
            Ok(sig(result))
        },
    )
    .unwrap()
}

#[test]
fn generated_trait_contract_impl_and_dispatch_publish_together_after_repair() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let protocol = batch.reserve().unwrap();
    let contract = batch.reserve().unwrap();
    let implementation = batch.reserve().unwrap();
    let method = batch.reserve().unwrap();
    let caller = batch.reserve().unwrap();
    let mut nodes = DefinitionNodes::new(protocol).unwrap();
    let item = arena.alloc_item(Item {
        owner_id: protocol,
        ident: name("GeneratedProtocol"),
        span: DUMMY_SP,
        kind: ItemKind::Trait(TraitDef {
            super_traits: None,
            clause_params: &[],
            clause_constraints: &[],
            items: vec![contract],
        }),
    });
    batch
        .define(
            nodes.finish(item, &mut 100).unwrap(),
            Some(hir::common::DefId::new(0, protocol.def_id.raw())),
            None,
        )
        .unwrap();
    batch
        .define(declaration(&arena, contract, "char"), None, None)
        .unwrap();

    let mut nodes = DefinitionNodes::new(implementation).unwrap();
    let root = nodes.root();
    let subject = expr(
        &arena,
        &mut nodes,
        root,
        ExprKind::Builtin(Symbol::intern("bool")),
        &mut 100,
    )
    .unwrap();
    let target = expr(&arena, &mut nodes, root, reference(protocol), &mut 100).unwrap();
    let item = arena.alloc_item(Item {
        owner_id: implementation,
        ident: name("generated_impl"),
        span: DUMMY_SP,
        kind: ItemKind::Impl(ImplDef {
            self_ty: subject,
            trait_ref: Some(target),
            clause_params: &[],
            clause_constraints: &[],
            items: vec![method],
        }),
    });
    batch
        .define(nodes.finish(item, &mut 100).unwrap(), None, None)
        .unwrap();
    let method = assemble_function(
        &arena,
        method,
        name("enabled"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let root = nodes.root();
            let result = expr(
                &arena,
                nodes,
                root,
                ExprKind::Builtin(Symbol::intern("bool")),
                fuel,
            )?;
            let body = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(root, fuel)?,
                span: DUMMY_SP,
                kind: ExprKind::Lit(Lit {
                    kind: LitKind::Bool(true),
                    span: DUMMY_SP,
                }),
            });
            Ok((sig(result), body))
        },
    )
    .unwrap();
    batch.define(method.definition, None, None).unwrap();
    batch.body(method.body_id, method.body).unwrap();

    let generated_caller = assemble_function(
        &arena,
        caller,
        name("run"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let root = nodes.root();
            let result = expr(
                &arena,
                nodes,
                root,
                ExprKind::Builtin(Symbol::intern("bool")),
                fuel,
            )?;
            let body_id = nodes.reserve(root, fuel)?;
            let projection_id = nodes.reserve(body_id, fuel)?;
            let cast_id = nodes.reserve(projection_id, fuel)?;
            let subject = expr(
                &arena,
                nodes,
                cast_id,
                ExprKind::Builtin(Symbol::intern("bool")),
                fuel,
            )?;
            let protocol = expr(&arena, nodes, cast_id, reference(protocol), fuel)?;
            let cast = arena.alloc_expr(Expr {
                hir_id: cast_id,
                kind: ExprKind::Cast(subject, protocol),
                span: DUMMY_SP,
            });
            nodes.register(Node::Expr(cast), fuel)?;
            let projected = arena.alloc_expr(Expr {
                hir_id: projection_id,
                kind: ExprKind::Projection(cast, name("enabled")),
                span: DUMMY_SP,
            });
            nodes.register(Node::Expr(projected), fuel)?;
            let body = arena.alloc_expr(Expr {
                hir_id: body_id,
                kind: ExprKind::Application(projected, &[]),
                span: DUMMY_SP,
            });
            Ok((sig(result), body))
        },
    )
    .unwrap();
    batch
        .define(generated_caller.definition, None, None)
        .unwrap();
    batch
        .body(generated_caller.body_id, generated_caller.body)
        .unwrap();
    let identities = BTreeMap::from([(0, 73)]);
    let before = batch.to_string();
    let rejected = publish(
        &mut batch,
        &mut 2000,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap_err();
    assert!(
        rejected
            .to_string()
            .contains("does not match its trait signature"),
        "{rejected}"
    );
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    assert_eq!(batch.published().num_bodies(), 0);
    batch
        .define(declaration(&arena, contract, "bool"), None, None)
        .unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 2000,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(batch.published().num_defs(), 5);
    assert_eq!(
        batch.published().num_bodies(),
        2,
        "bodyless contract must not create a phantom body"
    );
    let mut vm =
        comptime_interpretor::Interpreter::new(&checked, comptime_interpretor::Budget::default())
            .unwrap();
    assert_eq!(
        vm.invoke(
            nir::DeclarationId {
                package: 73,
                local: caller.def_id.raw()
            },
            vec![]
        )
        .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn assembling_a_bodyless_function_does_not_authorize_a_free_standing_contract() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let function = batch.reserve().unwrap();
    batch
        .define(declaration(&arena, function, "bool"), None, None)
        .unwrap();
    let error = publish(
        &mut batch,
        &mut 1000,
        &BTreeMap::from([(0, 73)]),
        &Protocols::default(),
        None,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("bodyless function requires a trait contract"),
        "{error}"
    );
    assert_eq!(batch.published().num_defs(), 0);
    assert_eq!(batch.published().num_bodies(), 0);
}
