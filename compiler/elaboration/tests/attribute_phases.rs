use comptime_runtime::Budget;
use elaboration::program::attributes::{EvaluatedAttribute, member_phases, merge};
use hir::*;
use nir::program::{Expr as NExpr, ExprKind as K, Literal, SourceRange};
use nir::{Primitive, Term};
use rustc_span::DUMMY_SP;

fn owner(id: u32) -> OwnerId {
    OwnerId::new(LocalDefId::new(id))
}
fn package(arena: &HirArena) -> Package<'_> {
    let mut package = Package::new();
    for id in 0..4 {
        let kind = if id == 0 {
            ItemKind::Trait(TraitDef {
                comptime_only: false,
                super_traits: None,
                clause_params: &[],
                clause_constraints: &[],
                items: vec![owner(1), owner(2)],
            })
        } else if id == 2 {
            let ty = arena.alloc_expr(Expr {
                hir_id: HirId::new(owner(id), ItemLocalId::new(1)),
                kind: ExprKind::Builtin(Symbol::intern("Type")),
                span: DUMMY_SP,
            });
            ItemKind::Associated(hir::item::AssociatedDecl {
                parameters: &[],
                ty,
                default: None,
                clause_params: &[],
                clause_constraints: &[],
            })
        } else {
            ItemKind::FnDeclaration(FnSig {
                params: &[],
                return_ty: None,
                return_bind: None,
                return_binding: None,
                modifiers: Default::default(),
                clause_params: &[],
                clause_constraints: &[],
                span: DUMMY_SP,
            })
        };
        let item = arena.alloc_item(Item {
            owner_id: owner(id),
            ident: Ident::new(Symbol::intern("member"), DUMMY_SP),
            kind,
            span: DUMMY_SP,
        });
        package.insert_owner(
            owner(id),
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes: OwnerNodes::new(),
            },
        );
    }
    package
}
fn metadata(target: u32, enabled: bool, offset: u32) -> EvaluatedAttribute {
    EvaluatedAttribute {
        target: AttributeTarget::Definition(owner(target)),
        lexical_owner: owner(0),
        source: SourceRange {
            start: offset,
            end: offset + 1,
        },
        configuration_key: Some("comptime_phase".into()),
        value: NExpr {
            source: SourceRange {
                start: offset,
                end: offset + 1,
            },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(Literal::Bool(enabled)),
        },
    }
}
#[test]
fn selected_phase_preserves_order_target_and_provenance() {
    let arena = HirArena::new();
    let package = package(&arena);
    let values = vec![
        metadata(2, true, 10),
        metadata(1, true, 20),
        metadata(1, false, 30),
    ];
    let merged = merge(&values, 4, &mut Budget::default()).unwrap();
    let phases = member_phases(&package, &merged, &mut Budget::default()).unwrap();
    assert_eq!(phases.len(), 2);
    assert_eq!(phases[0].owner, owner(1));
    assert_eq!(phases[0].phase, ImplementationPhase::Runtime);
    assert_eq!(phases[0].source.start, 30);
    assert_eq!(phases[1].phase, ImplementationPhase::Comptime);
    assert!(phases[1].to_string().contains("(phase comptime)"));
    assert!(phases[1].to_string().contains("(source 10 11)"));
}
#[test]
fn phase_targets_reject_free_functions_containers_and_fields() {
    let arena = HirArena::new();
    let package = package(&arena);
    for target in [
        AttributeTarget::Definition(owner(0)),
        AttributeTarget::Definition(owner(3)),
        AttributeTarget::Member(HirId::new(owner(1), ItemLocalId::new(1))),
    ] {
        let mut value = metadata(1, true, 10);
        value.target = target;
        let values = [value];
        let merged = merge(&values, 4, &mut Budget::default()).unwrap();
        let error = member_phases(&package, &merged, &mut Budget::default()).unwrap_err();
        assert_eq!(error.source.start, 10);
        assert!(error.message.contains("trait method or associated"));
    }
}
#[test]
fn bool_payload_and_original_domain_are_both_required() {
    let arena = HirArena::new();
    let package = package(&arena);
    for forged_domain in [false, true] {
        let mut value = metadata(1, true, 10);
        if forged_domain {
            value.value.ty = Term::Primitive(Primitive::Char);
        } else {
            value.value.kind = K::Literal(Literal::Char('x'));
        }
        let values = [value];
        let merged = merge(&values, 4, &mut Budget::default()).unwrap();
        assert!(member_phases(&package, &merged, &mut Budget::default()).is_err());
    }
}
#[test]
fn exhausted_phase_budget_can_retry_without_mutating_metadata() {
    let arena = HirArena::new();
    let package = package(&arena);
    let values = [metadata(1, true, 10)];
    let merged = merge(&values, 4, &mut Budget::default()).unwrap();
    let mut budget = Budget::default();
    budget.steps = 0;
    assert!(member_phases(&package, &merged, &mut budget).is_err());
    assert_eq!(
        member_phases(&package, &merged, &mut Budget::default()).unwrap()[0].phase,
        ImplementationPhase::Comptime
    );
}

#[test]
fn unrelated_metadata_is_left_for_its_own_consumer() {
    let arena = HirArena::new();
    let package = package(&arena);
    let mut value = metadata(3, true, 10);
    value.configuration_key = Some("abi".into());
    let values = [value];
    let merged = merge(&values, 4, &mut Budget::default()).unwrap();
    assert!(
        member_phases(&package, &merged, &mut Budget::default())
            .unwrap()
            .is_empty()
    );
    assert!(merged[0].get("abi").is_some());
}

#[test]
fn default_markers_validate_both_payload_and_domain_without_partial_results() {
    use elaboration::program::attributes::default_strategy_markers;
    let arena = HirArena::new();
    let mut package = package(&arena);
    let mut item = package.item(owner(3)).unwrap().clone();
    let expression = arena.alloc_expr(Expr {
        hir_id: HirId::new(owner(3), ItemLocalId::new(1)),
        span: DUMMY_SP,
        kind: ExprKind::TyVoid,
    });
    item.kind = ItemKind::Deriver(hir::item::DeriverDef {
        parameters: &[],
        trait_ref: expression,
        clause_params: &[],
        clause_constraints: &[],
        body: BodyId::new(HirId::make_owner(owner(3))),
    });
    package.insert_owner(
        owner(3),
        OwnerInfo {
            node: OwnerNode::Item(arena.alloc_item(item)),
            nodes: OwnerNodes::new(),
        },
    );
    let mut valid = metadata(3, true, 40);
    valid.configuration_key = Some("derivation_default".into());
    for invalid in 0..3 {
        let mut bad = valid.clone();
        match invalid {
            0 => bad.value.ty = Term::Primitive(Primitive::Char),
            1 => bad.value.kind = K::Literal(Literal::Char('x')),
            _ => bad.target = AttributeTarget::Member(HirId::new(owner(3), ItemLocalId::new(1))),
        }
        let values = [bad];
        let merged = merge(&values, 4, &mut Budget::default()).unwrap();
        let error =
            default_strategy_markers(&package, &merged, &mut Budget::default()).unwrap_err();
        assert_eq!(error.source.start, 40);
    }
    let values = [valid];
    let merged = merge(&values, 4, &mut Budget::default()).unwrap();
    let mut exhausted = Budget {
        steps: 0,
        ..Budget::default()
    };
    assert!(default_strategy_markers(&package, &merged, &mut exhausted).is_err());
    let markers = default_strategy_markers(&package, &merged, &mut Budget::default()).unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].owner, owner(3));
    assert!(markers[0].enabled);
    assert!(markers[0].to_string().contains("(source 40 41)"));
}
