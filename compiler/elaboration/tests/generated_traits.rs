use elaboration::{definitions::publish, program::Protocols};
use hir::{
    definitions::{
        DefinitionBatch, DefinitionNodes, Error, assemble_function, assemble_function_declaration,
        assemble_impl, assemble_trait,
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
    for phase in [ImplementationPhase::Runtime, ImplementationPhase::Comptime] {
        generated_phase_publication_after_repair(phase);
    }
}

fn generated_phase_publication_after_repair(phase: ImplementationPhase) {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let protocol = batch.reserve().unwrap();
    let contract = batch.reserve().unwrap();
    let implementation = batch.reserve().unwrap();
    let method = batch.reserve().unwrap();
    let caller = batch.reserve().unwrap();
    let definition = assemble_trait(
        &arena,
        protocol,
        name("GeneratedProtocol"),
        DUMMY_SP,
        &mut 100,
        |_, _| {
            Ok(TraitDef {
                comptime_only: false,
                super_traits: None,
                clause_params: &[],
                clause_constraints: &[],
                items: vec![contract],
            })
        },
    )
    .unwrap();
    batch
        .define(
            definition,
            Some(hir::common::DefId::new(0, protocol.def_id.raw())),
            None,
        )
        .unwrap();
    batch
        .define(declaration(&arena, contract, "char"), None, None)
        .unwrap();

    let definition = assemble_impl(
        &arena,
        implementation,
        name("generated_impl"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let root = nodes.root();
            let subject = expr(
                &arena,
                nodes,
                root,
                ExprKind::Builtin(Symbol::intern("bool")),
                fuel,
            )?;
            let target = expr(&arena, nodes, root, reference(protocol), fuel)?;
            Ok(ImplDef {
                generation_trait: None,
                phase,
                self_ty: subject,
                trait_ref: Some(target),
                clause_params: &[],
                clause_constraints: &[],
                items: vec![method],
            })
        },
    )
    .unwrap();
    batch.define(definition, None, None).unwrap();
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
    let flag = Expr {
        hir_id: HirId::new(contract, ItemLocalId::new(100)),
        kind: ExprKind::Lit(Lit {
            kind: LitKind::Bool(true),
            span: DUMMY_SP,
        }),
        span: DUMMY_SP,
    };
    if phase == ImplementationPhase::Comptime {
        batch.set_attributes(vec![DeferredDefinitionAttribute {
            target: AttributeTarget::Definition(contract),
            lexical_owner: contract,
            expression: "generated phase metadata".into(),
            value: &flag,
            configuration_key: Some("comptime_phase".into()),
            span: DUMMY_SP,
        }]);
        let complement = batch.reserve().unwrap();
        let definition = assemble_impl(
            &arena,
            complement,
            name("runtime_complement"),
            DUMMY_SP,
            &mut 100,
            |nodes, fuel| {
                let root = nodes.root();
                let subject = expr(
                    &arena,
                    nodes,
                    root,
                    ExprKind::Builtin(Symbol::intern("bool")),
                    fuel,
                )?;
                let target = expr(&arena, nodes, root, reference(protocol), fuel)?;
                Ok(ImplDef {
                    generation_trait: None,
                    phase: ImplementationPhase::Runtime,
                    self_ty: subject,
                    trait_ref: Some(target),
                    clause_params: &[],
                    clause_constraints: &[],
                    items: vec![],
                })
            },
        )
        .unwrap();
        batch.define(definition, None, None).unwrap();
    }
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
    assert!(batch.published().definition_attributes.is_empty());
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
    assert_eq!(
        batch.published().num_defs(),
        if phase == ImplementationPhase::Comptime {
            6
        } else {
            5
        }
    );
    assert_eq!(checked.implementations.len(), 1);
    assert!(checked.implementations[0].phases.runtime);
    assert_eq!(
        checked.implementations[0].phases.comptime,
        phase == ImplementationPhase::Comptime
    );
    assert_eq!(
        checked.traits[0].methods[0].phase,
        if phase == ImplementationPhase::Comptime {
            nir::program::TraitPhase::Comptime
        } else {
            nir::program::TraitPhase::Runtime
        }
    );
    assert_eq!(
        batch.published().num_bodies(),
        2,
        "bodyless contract must not create a phantom body"
    );
    assert!(
        checked
            .functions
            .iter()
            .all(|function| function.phase == nir::program::FunctionPhase::Runtime)
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

fn associated_value<'a>(arena: &'a HirArena, owner: OwnerId, value: LitKind) -> OwnerInfo<'a> {
    hir::definitions::assemble_associated(
        arena,
        owner,
        name("enabled"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let root = nodes.root();
            let ty = expr(
                arena,
                nodes,
                root,
                ExprKind::Builtin(Symbol::intern("bool")),
                fuel,
            )?;
            let default = expr(
                arena,
                nodes,
                root,
                ExprKind::Lit(Lit {
                    kind: value,
                    span: DUMMY_SP,
                }),
                fuel,
            )?;
            Ok(hir::item::AssociatedDecl {
                parameters: &[],
                ty,
                default: Some(default),
                clause_params: &[],
                clause_constraints: &[],
            })
        },
    )
    .unwrap()
}

#[test]
fn generated_associated_defaults_and_overrides_use_atomic_semantic_publication() {
    check_associated_publication(true);
}

#[test]
fn generated_runtime_pure_associated_values_keep_resource_evidence() {
    check_associated_publication(false);
}

fn check_associated_publication(comptime: bool) {
    for override_default in [false, true] {
        let arena = HirArena::new();
        let mut package = Package::new();
        let mut batch = DefinitionBatch::new(&mut package);
        let protocol = batch.reserve().unwrap();
        let contract = batch.reserve().unwrap();
        let implementation = batch.reserve().unwrap();
        let caller = batch.reserve().unwrap();
        let value = override_default.then(|| batch.reserve().unwrap());
        let definition = assemble_trait(
            &arena,
            protocol,
            name("GeneratedProtocol"),
            DUMMY_SP,
            &mut 100,
            |_, _| {
                Ok(TraitDef {
                    comptime_only: false,
                    super_traits: None,
                    clause_params: &[],
                    clause_constraints: &[],
                    items: vec![contract],
                })
            },
        )
        .unwrap();
        batch
            .define(
                definition,
                Some(hir::common::DefId::new(0, protocol.def_id.raw())),
                None,
            )
            .unwrap();
        batch
            .define(
                associated_value(&arena, contract, LitKind::Bool(true)),
                None,
                None,
            )
            .unwrap();
        let definition = assemble_impl(
            &arena,
            implementation,
            name("generated_impl"),
            DUMMY_SP,
            &mut 100,
            |nodes, fuel| {
                let root = nodes.root();
                let self_ty = expr(
                    &arena,
                    nodes,
                    root,
                    ExprKind::Builtin(Symbol::intern("bool")),
                    fuel,
                )?;
                let trait_ref = Some(expr(&arena, nodes, root, reference(protocol), fuel)?);
                Ok(ImplDef {
                    generation_trait: None,
                    phase: hir::ImplementationPhase::Runtime,
                    self_ty,
                    trait_ref,
                    clause_params: &[],
                    clause_constraints: &[],
                    items: value.into_iter().collect(),
                })
            },
        )
        .unwrap();
        batch.define(definition, None, None).unwrap();
        if let Some(value) = value {
            batch
                .define(
                    associated_value(&arena, value, LitKind::Char('x')),
                    None,
                    None,
                )
                .unwrap();
        }
        let generated = assemble_function(
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
                let body = nodes.reserve(root, fuel)?;
                let cast = nodes.reserve(body, fuel)?;
                let subject = expr(
                    &arena,
                    nodes,
                    cast,
                    ExprKind::Builtin(Symbol::intern("bool")),
                    fuel,
                )?;
                let target = expr(&arena, nodes, cast, reference(protocol), fuel)?;
                let cast = arena.alloc_expr(Expr {
                    hir_id: cast,
                    kind: ExprKind::Cast(subject, target),
                    span: DUMMY_SP,
                });
                nodes.register(Node::Expr(cast), fuel)?;
                let body = arena.alloc_expr(Expr {
                    hir_id: body,
                    kind: ExprKind::Projection(cast, name("enabled")),
                    span: DUMMY_SP,
                });
                let mut signature = sig(result);
                signature.modifiers.is_comptime = comptime;
                Ok((signature, body))
            },
        )
        .unwrap();
        batch.define(generated.definition, None, None).unwrap();
        batch.body(generated.body_id, generated.body).unwrap();
        let identities = BTreeMap::from([(0, 73)]);
        if let Some(value) = value {
            let before = batch.to_string();
            assert!(
                publish(
                    &mut batch,
                    &mut 3000,
                    &identities,
                    &Protocols::default(),
                    None
                )
                .is_err()
            );
            assert_eq!(batch.to_string(), before);
            assert_eq!(batch.published().num_defs(), 0);
            assert_eq!(batch.published().num_bodies(), 0);
            batch
                .define(
                    associated_value(&arena, value, LitKind::Bool(false)),
                    None,
                    None,
                )
                .unwrap();
        }
        let (_, mut checked) = publish(
            &mut batch,
            &mut 3000,
            &identities,
            &Protocols::default(),
            None,
        )
        .unwrap();
        assert_eq!(batch.published().num_bodies(), 1);
        checked.dispatch_instances =
            elaboration::materialization::dispatch_instances_for_invocation(
                &checked,
                &[iir::InstanceKey {
                    declaration: nir::DeclarationId {
                        package: 73,
                        local: caller.def_id.raw(),
                    },
                    arguments: vec![].into(),
                }],
                Default::default(),
            )
            .unwrap();
        let mut vm = if comptime {
            comptime_interpretor::Interpreter::for_comptime(&checked, Default::default())
        } else {
            comptime_interpretor::Interpreter::new(&checked, Default::default())
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
            comptime_interpretor::Value::Bool(!override_default)
        );
    }
}

#[test]
fn generated_comptime_only_trait_preserves_identity_on_publication() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let protocol = batch.reserve().unwrap();
    let definition = assemble_trait(
        &arena,
        protocol,
        name("Stage"),
        DUMMY_SP,
        &mut 100,
        |_, _| {
            Ok(TraitDef {
                comptime_only: true,
                super_traits: None,
                clause_params: &[],
                clause_constraints: &[],
                items: vec![],
            })
        },
    )
    .unwrap();
    batch.define(definition, None, None).unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 1000,
        &BTreeMap::from([(0, 73)]),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert!(checked.traits[0].comptime_only);
    assert_eq!(batch.published().num_defs(), 1);
}
