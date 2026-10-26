use comptime_interpretor::{Budget, Interpreter, Value};
use elaboration::{definitions::publish, program::Protocols};
use hir::{
    common::{FnParam, TyParam, TyParamKind},
    definitions::{AssembledFunction, DefinitionBatch, DefinitionNodes, Error, assemble_function},
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
    parent: HirId,
    kind: ExprKind<'a>,
    fuel: &mut usize,
) -> Result<&'a Expr<'a>, Error> {
    let hir_id = nodes.reserve(parent, fuel)?;
    let value = arena.alloc_expr(Expr {
        hir_id,
        kind,
        span: DUMMY_SP,
    });
    nodes.register(Node::Expr(value), fuel)?;
    Ok(value)
}
fn signature<'a>(params: &'a [FnParam<'a>], result: &'a Expr<'a>, comptime: bool) -> FnSig<'a> {
    FnSig {
        params,
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
    }
}
fn generated<'a>(
    arena: &'a HirArena,
    owner: OwnerId,
    comptime: bool,
    wrong: bool,
) -> AssembledFunction<'a> {
    assemble_function(
        arena,
        owner,
        name("select"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let root = nodes.root();
            let parameter = nodes.reserve(root, fuel)?;
            let ty = expression(
                arena,
                nodes,
                parameter,
                ExprKind::Builtin(Symbol::intern("bool")),
                fuel,
            )?;
            let default = expression(
                arena,
                nodes,
                parameter,
                ExprKind::Lit(Lit {
                    kind: LitKind::Bool(true),
                    span: DUMMY_SP,
                }),
                fuel,
            )?;
            let params = arena.alloc_fn_param_slice(vec![FnParam {
                hir_id: parameter,
                name: name("value"),
                index: 0,
                ty: TyParam::new(
                    parameter,
                    TyParamKind::Optional(name("value"), ty, default),
                    DUMMY_SP,
                ),
                span: DUMMY_SP,
            }]);
            let result = expression(
                arena,
                nodes,
                root,
                ExprKind::Builtin(Symbol::intern("bool")),
                fuel,
            )?;
            let body = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(root, fuel)?,
                span: DUMMY_SP,
                kind: if wrong {
                    ExprKind::Lit(Lit {
                        kind: LitKind::Char('x'),
                        span: DUMMY_SP,
                    })
                } else {
                    ExprKind::ResolvedFnParam(Symbol::intern("value"), parameter)
                },
            });
            Ok((signature(params, result, comptime), body))
        },
    )
    .unwrap()
}

#[test]
fn generated_runtime_and_comptime_functions_check_retry_and_invoke_defaults() {
    for comptime in [false, true] {
        let arena = HirArena::new();
        let mut package = Package::new();
        let mut batch = DefinitionBatch::new(&mut package);
        let function = batch.reserve().unwrap();
        let caller = batch.reserve().unwrap();
        let function_id = hir::common::DefId::new(0, function.def_id.raw());
        let assembled_caller = assemble_function(
            &arena,
            caller,
            name("run"),
            DUMMY_SP,
            &mut 100,
            |nodes, fuel| {
                let root = nodes.root();
                let result = expression(
                    &arena,
                    nodes,
                    root,
                    ExprKind::Builtin(Symbol::intern("bool")),
                    fuel,
                )?;
                let body_id = nodes.reserve(root, fuel)?;
                let callee = expression(
                    &arena,
                    nodes,
                    body_id,
                    ExprKind::ResolvedDef {
                        name: Symbol::intern("select"),
                        has_clause_param: false,
                        def_id: function_id,
                    },
                    fuel,
                )?;
                let body = arena.alloc_expr(Expr {
                    hir_id: body_id,
                    kind: ExprKind::Application(callee, &[]),
                    span: DUMMY_SP,
                });
                Ok((signature(&[], result, comptime), body))
            },
        )
        .unwrap();
        batch
            .define(assembled_caller.definition, None, None)
            .unwrap();
        batch
            .body(assembled_caller.body_id, assembled_caller.body)
            .unwrap();
        let identities = BTreeMap::from([(0, 73)]);
        let wrong = generated(&arena, function, comptime, true);
        batch
            .define(wrong.definition, Some(function_id), None)
            .unwrap();
        batch.body(wrong.body_id, wrong.body).unwrap();
        let before = batch.to_string();
        assert!(
            publish(
                &mut batch,
                &mut 1000,
                &identities,
                &Protocols::default(),
                None
            )
            .is_err()
        );
        assert_eq!(before, batch.to_string());
        assert_eq!(batch.published().num_defs(), 0);
        assert_eq!(batch.published().num_bodies(), 0);
        let repaired = generated(&arena, function, comptime, false);
        batch
            .define(repaired.definition, Some(function_id), None)
            .unwrap();
        batch.body(repaired.body_id, repaired.body).unwrap();
        let (_, checked) = publish(
            &mut batch,
            &mut 1000,
            &identities,
            &Protocols::default(),
            None,
        )
        .unwrap();
        let mut vm = if comptime {
            Interpreter::for_comptime(&checked, Budget::default())
        } else {
            Interpreter::new(&checked, Budget::default())
        }
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
            Value::Bool(true)
        );
        assert_eq!(
            vm.invoke(
                nir::DeclarationId {
                    package: 73,
                    local: function.def_id.raw()
                },
                vec![Value::Bool(false)]
            )
            .unwrap(),
            Value::Bool(false)
        );
    }
}
