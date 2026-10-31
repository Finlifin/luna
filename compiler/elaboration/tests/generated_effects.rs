//! Generated effects use ordinary semantic publication, not function bodies.
use elaboration::{definitions::publish, program::Protocols};
use hir::{
    common::{FnParam, TyParam, TyParamKind},
    definitions::{DefinitionBatch, assemble_effect, assemble_effect_group},
    item::{EffectGroupDef, EffectModifiers},
    *,
};
use rustc_span::DUMMY_SP;
use std::collections::BTreeMap;

fn name(text: &str) -> Ident {
    Ident::new(Symbol::intern(text), DUMMY_SP)
}
fn operation<'a>(
    arena: &'a HirArena,
    owner: OwnerId,
    modifiers: EffectModifiers,
    valid_type: bool,
    parameter_index: u32,
    has_parameter: bool,
) -> Result<OwnerInfo<'a>, hir::definitions::Error> {
    assemble_effect(
        arena,
        owner,
        name(&format!("request{}", owner.def_id.raw())),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let root = nodes.root();
            let params = if has_parameter {
                let parameter = nodes.reserve(root, fuel)?;
                let ty = arena.alloc_expr(Expr {
                    hir_id: nodes.reserve(parameter, fuel)?,
                    kind: if valid_type {
                        ExprKind::Builtin(Symbol::intern("bool"))
                    } else {
                        ExprKind::Lit(Lit {
                            kind: LitKind::Bool(true),
                            span: DUMMY_SP,
                        })
                    },
                    span: DUMMY_SP,
                });
                nodes.register(Node::Expr(ty), fuel)?;
                vec![FnParam {
                    template_annotation: None,
                    hir_id: parameter,
                    name: name("value"),
                    index: parameter_index,
                    ty: TyParam::new(parameter, TyParamKind::Positional(ty), DUMMY_SP),
                    span: DUMMY_SP,
                }]
            } else {
                vec![]
            };
            let result = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(root, fuel)?,
                kind: ExprKind::Builtin(Symbol::intern("bool")),
                span: DUMMY_SP,
            });
            nodes.register(Node::Expr(result), fuel)?;
            Ok((
                FnSig {
                    params: arena.alloc_fn_param_slice(params),
                    return_ty: Some(result),
                    return_bind: None,
                    return_binding: None,
                    modifiers: Default::default(),
                    clause_params: &[],
                    clause_constraints: &[],
                    span: DUMMY_SP,
                },
                modifiers,
            ))
        },
    )
}

#[test]
fn generated_effect_groups_preserve_abi_and_repair_invalid_signatures() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let group = batch.reserve().unwrap();
    let operations: Vec<_> = (0..4).map(|_| batch.reserve().unwrap()).collect();
    let identity = |owner: OwnerId| Some(hir::common::DefId::new(0, owner.def_id.raw()));
    let group_info = assemble_effect_group(
        &arena,
        group,
        name("GeneratedEffects"),
        DUMMY_SP,
        &mut 100,
        |_, _| {
            Ok(EffectGroupDef {
                operations: operations.clone(),
                clause_params: &[],
                clause_constraints: &[],
            })
        },
    )
    .unwrap();
    batch.define(group_info, identity(group), None).unwrap();
    let modifiers = [
        EffectModifiers::default(),
        EffectModifiers {
            is_async: true,
            ..Default::default()
        },
        EffectModifiers {
            is_direct_passing: true,
            ..Default::default()
        },
        EffectModifiers {
            is_inline: true,
            ..Default::default()
        },
    ];
    for (&owner, &flags) in operations.iter().zip(&modifiers) {
        assert_eq!(
            operation(&arena, owner, flags, true, 1, true)
                .unwrap_err()
                .kind,
            "effect parameter identity or index is inconsistent"
        );
        batch
            .define(
                operation(&arena, owner, flags, false, 0, true).unwrap(),
                identity(owner),
                None,
            )
            .unwrap();
    }
    let identities = BTreeMap::from([(0, 73)]);
    let before = batch.to_string();
    assert!(matches!(
        publish(
            &mut batch,
            &mut 5000,
            &identities,
            &Protocols::default(),
            None
        ),
        Err(hir::definitions::PublicationError::Check(_))
    ));
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    for (&owner, &flags) in operations.iter().zip(&modifiers) {
        batch
            .define(
                operation(&arena, owner, flags, true, 0, !flags.is_direct_passing).unwrap(),
                identity(owner),
                None,
            )
            .unwrap();
    }
    let direct = operations[2];
    batch
        .define(
            operation(&arena, direct, modifiers[2], true, 0, true).unwrap(),
            identity(direct),
            None,
        )
        .unwrap();
    let invalid = publish(
        &mut batch,
        &mut 5000,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap_err();
    assert!(
        invalid
            .to_string()
            .contains("direct passing effects must be synchronous and have no parameters")
    );
    assert_eq!(batch.published().num_defs(), 0);
    batch
        .define(
            operation(&arena, direct, modifiers[2], true, 0, false).unwrap(),
            identity(direct),
            None,
        )
        .unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 5000,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap();
    let ItemKind::EffectGroup(descriptor) = &batch.published().item(group).unwrap().kind else {
        panic!("effect group kind lost")
    };
    assert_eq!(descriptor.operations, operations);
    assert!(
        batch
            .published()
            .item(group)
            .unwrap()
            .to_string()
            .contains("effect-group")
    );
    assert_eq!(checked.effects.len(), 4);
    assert!(
        checked.functions.is_empty(),
        "effect declarations must not create function bodies"
    );
    let group_id = nir::DeclarationId {
        package: 73,
        local: group.def_id.raw(),
    };
    for ((&owner, flags), abi) in operations.iter().zip(modifiers).zip([
        nir::effects::Abi::Direct,
        nir::effects::Abi::Async,
        nir::effects::Abi::DirectPassing,
        nir::effects::Abi::Direct,
    ]) {
        let effect = checked
            .effects
            .iter()
            .find(|e| e.declaration.local == owner.def_id.raw())
            .unwrap();
        assert_eq!(effect.group, Some(group_id));
        assert_eq!(effect.abi, abi);
        assert_eq!(effect.inline, flags.is_inline);
        assert_eq!(
            effect.signature,
            nir::Term::Function {
                parameters: if flags.is_direct_passing {
                    vec![]
                } else {
                    vec![nir::Parameter {
                        sort: nir::Sort::Runtime,
                        ty: nir::Term::Primitive(nir::Primitive::Bool),
                    }]
                },
                result: Box::new(nir::Term::Primitive(nir::Primitive::Bool)),
            }
        );
    }
}

#[test]
fn exhausted_effect_assembly_does_not_run_callbacks() {
    let arena = HirArena::new();
    let owner = OwnerId::new(LocalDefId::new(0));
    let mut called = false;
    assert!(
        assemble_effect(&arena, owner, name("request"), DUMMY_SP, &mut 0, |_, _| {
            called = true;
            unreachable!()
        })
        .is_err()
    );
    assert!(!called);
    assert!(
        assemble_effect_group(&arena, owner, name("Group"), DUMMY_SP, &mut 0, |_, _| {
            called = true;
            unreachable!()
        })
        .is_err()
    );
    assert!(!called);
}
