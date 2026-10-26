//! Generated Type factories keep opaque heads and declaration-side defaults.
use comptime_interpretor::{Budget, Interpreter, Value};
use elaboration::{definitions::publish, program::Protocols};
use hir::definitions::{
    AssembledFunction, DefinitionBatch, DefinitionNodes, Error, assemble_function,
};
use hir::*;
use rustc_span::DUMMY_SP;
use std::collections::BTreeMap;

fn name(text: &str) -> Ident {
    Ident::new(Symbol::intern(text), DUMMY_SP)
}
fn expr<'a>(
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
fn signature<'a>(result: &'a Expr<'a>) -> FnSig<'a> {
    FnSig {
        params: &[],
        return_ty: Some(result),
        return_bind: None,
        return_binding: None,
        modifiers: hir::item::FnModifiers {
            is_pure: true,
            is_comptime: true,
            ..Default::default()
        },
        clause_params: &[],
        clause_constraints: &[],
        span: DUMMY_SP,
    }
}
fn factory<'a>(arena: &'a HirArena, owner: OwnerId, wrong: bool) -> AssembledFunction<'a> {
    factory_with_ids(arena, owner, wrong, 10_001)
}
fn factory_with_ids<'a>(
    arena: &'a HirArena,
    owner: OwnerId,
    wrong: bool,
    first: u32,
) -> AssembledFunction<'a> {
    assemble_function(
        arena,
        owner,
        name("Identity"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let root = nodes.root();
            let first_id = hir::common::DefId::new(0, first);
            let second_id = hir::common::DefId::new(0, first + 1);
            let first = nodes.reserve(root, fuel)?;
            let second = nodes.reserve(root, fuel)?;
            let domain = expr(
                arena,
                nodes,
                second,
                ExprKind::Builtin(Symbol::intern("Type")),
                fuel,
            )?;
            let default = expr(
                arena,
                nodes,
                second,
                ExprKind::ResolvedClauseParam(Symbol::intern("T"), first_id),
                fuel,
            )?;
            let clauses = arena.alloc_clause_param_slice(vec![
                ClauseParam {
                    hir_id: first,
                    definition: Some(first_id),
                    name: name("T"),
                    kind: ClauseParamKind::Type(name("T")),
                    span: DUMMY_SP,
                },
                ClauseParam {
                    hir_id: second,
                    definition: Some(second_id),
                    name: name("U"),
                    kind: ClauseParamKind::Optional(name("U"), domain, default),
                    span: DUMMY_SP,
                },
            ]);
            for clause in clauses {
                nodes.register(Node::ClauseParam(clause), fuel)?;
            }
            let result = expr(
                arena,
                nodes,
                root,
                ExprKind::Builtin(Symbol::intern("Type")),
                fuel,
            )?;
            let body = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(root, fuel)?,
                span: DUMMY_SP,
                kind: if wrong {
                    ExprKind::Lit(Lit {
                        kind: LitKind::Bool(true),
                        span: DUMMY_SP,
                    })
                } else {
                    ExprKind::ResolvedClauseParam(Symbol::intern("U"), second_id)
                },
            });
            let mut sig = signature(result);
            sig.modifiers.is_normal_form_head = true;
            sig.clause_params = clauses;
            Ok((sig, body))
        },
    )
    .unwrap()
}
fn caller<'a>(
    arena: &'a HirArena,
    owner: OwnerId,
    factory: OwnerId,
    explicit: bool,
) -> AssembledFunction<'a> {
    assemble_function(
        arena,
        owner,
        name(if explicit { "Explicit" } else { "Defaulted" }),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let root = nodes.root();
            let result = expr(
                arena,
                nodes,
                root,
                ExprKind::Builtin(Symbol::intern("Type")),
                fuel,
            )?;
            let body = nodes.reserve(root, fuel)?;
            let head = expr(
                arena,
                nodes,
                body,
                ExprKind::ResolvedDef {
                    name: Symbol::intern("Identity"),
                    has_clause_param: true,
                    def_id: hir::common::DefId::new(0, factory.def_id.raw()),
                },
                fuel,
            )?;
            let first = expr(
                arena,
                nodes,
                body,
                ExprKind::Builtin(Symbol::intern("bool")),
                fuel,
            )?;
            let mut inputs = vec![hir::common::Arg::Positional(first)];
            if explicit {
                let second = expr(
                    arena,
                    nodes,
                    body,
                    ExprKind::Builtin(Symbol::intern("char")),
                    fuel,
                )?;
                inputs.push(hir::common::Arg::Named(name("U"), second));
            }
            let body = arena.alloc_expr(Expr {
                hir_id: body,
                kind: ExprKind::NFApplication(head, arena.alloc_arg_slice(inputs)),
                span: DUMMY_SP,
            });
            Ok((signature(result), body))
        },
    )
    .unwrap()
}
fn install<'hir>(batch: &mut DefinitionBatch<'_, 'hir>, definition: AssembledFunction<'hir>) {
    let owner = definition.definition.node.expect_item().owner_id;
    batch
        .define(
            definition.definition,
            Some(hir::common::DefId::new(0, owner.def_id.raw())),
            None,
        )
        .unwrap();
    batch.body(definition.body_id, definition.body).unwrap();
}

#[test]
fn generated_factory_defaults_keep_identity_and_never_invoke_the_head() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let head = batch.reserve().unwrap();
    let defaulted = batch.reserve().unwrap();
    let explicit = batch.reserve().unwrap();
    install(&mut batch, factory(&arena, head, true));
    install(&mut batch, caller(&arena, defaulted, head, false));
    install(&mut batch, caller(&arena, explicit, head, true));
    let identities = BTreeMap::from([(0, 73)]);
    let before = batch.to_string();
    assert!(
        publish(
            &mut batch,
            &mut 5000,
            &identities,
            &Protocols::default(),
            None
        )
        .is_err()
    );
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    install(&mut batch, factory(&arena, head, false));
    let (_, checked) = publish(
        &mut batch,
        &mut 5000,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap();
    let head_id = nir::DeclarationId {
        package: 73,
        local: head.def_id.raw(),
    };
    for (entry, second) in [
        (defaulted, nir::Primitive::Bool),
        (explicit, nir::Primitive::Char),
    ] {
        let mut vm = Interpreter::for_comptime(&checked, Budget::default()).unwrap();
        let value = vm
            .invoke(
                nir::DeclarationId {
                    package: 73,
                    local: entry.def_id.raw(),
                },
                vec![],
            )
            .unwrap();
        assert_eq!(
            value,
            Value::Type(nir::Term::NormalForm {
                head: head_id,
                sort: nir::Sort::Type,
                arguments: vec![
                    nir::Term::Primitive(nir::Primitive::Bool),
                    nir::Term::Primitive(second)
                ],
            })
        );
        assert!(!vm.events().iter().any(|event| matches!(event,
            comptime_interpretor::Event::EnterFunction(id) if *id == head_id)));
    }
}

#[test]
fn generated_factories_reject_shared_inputs_then_execute_independently() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let first = batch.reserve().unwrap();
    let second = batch.reserve().unwrap();
    let first_entry = batch.reserve().unwrap();
    let second_entry = batch.reserve().unwrap();
    install(&mut batch, factory_with_ids(&arena, first, false, 20_000));
    install(&mut batch, factory_with_ids(&arena, second, false, 20_000));
    install(&mut batch, caller(&arena, first_entry, first, false));
    install(&mut batch, caller(&arena, second_entry, second, true));
    let identities = BTreeMap::from([(0, 73)]);
    let before = batch.to_string();
    let error = publish(
        &mut batch,
        &mut 5000,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("clause definition identity is already declared")
    );
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    install(&mut batch, factory_with_ids(&arena, second, false, 30_000));
    let (_, checked) = publish(
        &mut batch,
        &mut 5000,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&checked, Budget::default()).unwrap();
    for (entry, head, tail) in [
        (first_entry, first, nir::Primitive::Bool),
        (second_entry, second, nir::Primitive::Char),
        (first_entry, first, nir::Primitive::Bool),
    ] {
        let id = |owner: OwnerId| nir::DeclarationId {
            package: 73,
            local: owner.def_id.raw(),
        };
        assert_eq!(
            vm.invoke(id(entry), vec![]).unwrap(),
            Value::Type(nir::Term::NormalForm {
                head: id(head),
                sort: nir::Sort::Type,
                arguments: vec![
                    nir::Term::Primitive(nir::Primitive::Bool),
                    nir::Term::Primitive(tail)
                ],
            })
        );
        assert!(!vm.events().iter().any(|event| matches!(event,
            comptime_interpretor::Event::EnterFunction(entered) if *entered == id(head))));
    }
}
