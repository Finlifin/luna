//! Constant dependencies and module ownership survive generated publication.
use comptime_interpretor::{Interpreter, Value};
use elaboration::{definitions::publish, program::Protocols};
use hir::{
    definitions::{
        DefinitionBatch, DefinitionNodes, Error, assemble_constant, assemble_function,
        assemble_module,
    },
    *,
};
use rustc_span::DUMMY_SP;
use std::collections::BTreeMap;

fn name(text: &str) -> Ident {
    Ident::new(Symbol::intern(text), DUMMY_SP)
}
fn expression<'a>(
    arena: &'a HirArena,
    nodes: &mut DefinitionNodes<'a>,
    kind: ExprKind<'a>,
    fuel: &mut usize,
) -> Result<&'a Expr<'a>, Error> {
    let root = nodes.root();
    let value = arena.alloc_expr(Expr {
        hir_id: nodes.reserve(root, fuel)?,
        kind,
        span: DUMMY_SP,
    });
    nodes.register(Node::Expr(value), fuel)?;
    Ok(value)
}
fn reference(owner: OwnerId) -> ExprKind<'static> {
    ExprKind::ResolvedDef {
        name: Symbol::intern("value"),
        has_clause_param: false,
        def_id: hir::common::DefId::new(0, owner.def_id.raw()),
    }
}
fn constant<'a>(arena: &'a HirArena, owner: OwnerId, kind: ExprKind<'a>) -> OwnerInfo<'a> {
    assemble_constant(
        arena,
        owner,
        name(&format!("value{}", owner.def_id.raw())),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let annotation = expression(
                arena,
                nodes,
                ExprKind::Builtin(Symbol::intern("bool")),
                fuel,
            )?;
            let initializer = expression(arena, nodes, kind, fuel)?;
            Ok((annotation, initializer))
        },
    )
    .unwrap()
}

#[test]
fn generated_constants_check_dependencies_and_require_splice_for_runtime_use() {
    for comptime in [false, true] {
        let arena = HirArena::new();
        let mut package = Package::new();
        let mut batch = DefinitionBatch::new(&mut package);
        let module = batch.reserve().unwrap();
        let first = batch.reserve().unwrap();
        let second = batch.reserve().unwrap();
        let entry = batch.reserve().unwrap();
        let identity = |owner: OwnerId| Some(hir::common::DefId::new(0, owner.def_id.raw()));
        batch
            .define(
                assemble_module(
                    &arena,
                    module,
                    name("Generated"),
                    &[entry, first, second],
                    DUMMY_SP,
                    &mut 100,
                )
                .unwrap(),
                identity(module),
                None,
            )
            .unwrap();
        batch
            .define(
                constant(&arena, first, reference(second)),
                identity(first),
                None,
            )
            .unwrap();
        let function = assemble_function(
            &arena,
            entry,
            name("read"),
            DUMMY_SP,
            &mut 100,
            |nodes, fuel| {
                let result = expression(
                    &arena,
                    nodes,
                    ExprKind::Builtin(Symbol::intern("bool")),
                    fuel,
                )?;
                let root = nodes.root();
                let body = arena.alloc_expr(Expr {
                    hir_id: nodes.reserve(root, fuel)?,
                    kind: reference(first),
                    span: DUMMY_SP,
                });
                Ok((
                    FnSig {
                        params: &[],
                        return_ty: Some(result),
                        return_bind: None,
                        return_binding: None,
                        modifiers: hir::item::FnModifiers {
                            is_comptime: comptime,
                            is_pure: true,
                            ..Default::default()
                        },
                        clause_params: &[],
                        clause_constraints: &[],
                        span: DUMMY_SP,
                    },
                    body,
                ))
            },
        )
        .unwrap();
        batch
            .define(function.definition, identity(entry), None)
            .unwrap();
        batch.body(function.body_id, function.body).unwrap();
        let identities = BTreeMap::from([(0, 73)]);
        for (index, invalid) in [
            ExprKind::Lit(Lit {
                kind: LitKind::Char('x'),
                span: DUMMY_SP,
            }),
            reference(first),
        ]
        .into_iter()
        .enumerate()
        {
            batch
                .define(constant(&arena, second, invalid), identity(second), None)
                .unwrap();
            let before = batch.to_string();
            let result = publish(
                &mut batch,
                &mut 5000,
                &identities,
                &Protocols::default(),
                None,
            );
            assert!(
                matches!(result, Err(hir::definitions::PublicationError::Check(_))),
                "{result:?}"
            );
            if comptime && index == 1 {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("constant dependency")
                );
            }
            assert_eq!(batch.to_string(), before);
            assert_eq!(batch.published().num_defs(), 0);
        }
        batch
            .define(
                constant(
                    &arena,
                    second,
                    ExprKind::Lit(Lit {
                        kind: LitKind::Bool(true),
                        span: DUMMY_SP,
                    }),
                ),
                identity(second),
                None,
            )
            .unwrap();
        let result = publish(
            &mut batch,
            &mut 5000,
            &identities,
            &Protocols::default(),
            None,
        );
        if !comptime {
            let error = result.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("constant runtime use requires cross-stage Splice"),
                "{error}"
            );
            assert_eq!(batch.published().num_defs(), 0);
            continue;
        }
        let (_, checked) = result.unwrap();
        let ItemKind::Mod(definition) = &batch.published().item(module).unwrap().kind else {
            panic!("module kind lost")
        };
        assert_eq!(definition.items, [entry, first, second]);
        assert_eq!(checked.functions.len(), 1);
        let mut vm = Interpreter::for_comptime(&checked, Default::default()).unwrap();
        for _ in 0..2 {
            assert_eq!(
                vm.invoke(
                    nir::DeclarationId {
                        package: 73,
                        local: entry.def_id.raw()
                    },
                    vec![]
                )
                .unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn constant_assembly_requires_budget_and_owner_local_roots() {
    let arena = HirArena::new();
    let owner = OwnerId::new(LocalDefId::new(0));
    let mut called = false;
    assert!(
        assemble_constant(&arena, owner, name("value"), DUMMY_SP, &mut 0, |_, _| {
            called = true;
            unreachable!()
        })
        .is_err()
    );
    assert!(!called);
    assert!(assemble_module(&arena, owner, name("Group"), &[owner], DUMMY_SP, &mut 1).is_err());
    let foreign = arena.alloc_expr(Expr {
        hir_id: HirId::new(OwnerId::new(LocalDefId::new(1)), ItemLocalId::new(1)),
        kind: ExprKind::Builtin(Symbol::intern("bool")),
        span: DUMMY_SP,
    });
    let error = assemble_constant(
        &arena,
        owner,
        name("value"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let value = expression(
                &arena,
                nodes,
                ExprKind::Lit(Lit {
                    kind: LitKind::Bool(true),
                    span: DUMMY_SP,
                }),
                fuel,
            )?;
            Ok((foreign, value))
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        "constant roots must be reserved under their owner"
    );
}
