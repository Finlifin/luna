use comptime_runtime::Budget;
use elaboration::program::attributes::{EvaluatedAttribute, merge};
use hir::{AttributeTarget, HirId, ItemLocalId, LocalDefId, OwnerId};
use nir::program::{Expr, ExprKind as K, Literal, SourceRange};
use nir::{Primitive, Term};
fn target(id: u32) -> AttributeTarget {
    AttributeTarget::Definition(OwnerId {
        def_id: LocalDefId::new(id),
    })
}
fn flag(value: bool) -> Expr {
    Expr {
        ty: Term::Primitive(Primitive::Bool),
        source: SourceRange { start: 1, end: 2 },
        kind: K::Literal(Literal::Bool(value)),
    }
}
fn attribute(
    target: AttributeTarget,
    key: Option<&str>,
    fields: &[(&str, bool)],
    offset: u32,
) -> EvaluatedAttribute {
    EvaluatedAttribute {
        target,
        lexical_owner: OwnerId {
            def_id: LocalDefId::new(0),
        },
        source: SourceRange {
            start: offset,
            end: offset + 1,
        },
        configuration_key: key.map(str::to_owned),
        value: if key.is_some() {
            flag(fields[0].1)
        } else {
            Expr {
                ty: Term::Primitive(Primitive::Object),
                source: SourceRange {
                    start: offset,
                    end: offset + 1,
                },
                kind: K::Object {
                    names: fields.iter().map(|(key, _)| Some((*key).into())).collect(),
                    fields: fields.iter().map(|(_, value)| flag(*value)).collect(),
                },
            }
        },
    }
}
#[test]
fn configurations_precede_prefixes_and_targets_keep_separate_provenance() {
    let attributes = vec![
        attribute(target(1), None, &[("key", false), ("key", true)], 10),
        attribute(target(2), Some("key"), &[("key", false)], 20),
        attribute(target(1), Some("key"), &[("key", false)], 30),
        attribute(target(1), None, &[("key", false)], 40),
    ];
    let result = merge(&attributes, 10, &mut Budget::default()).unwrap();
    assert_eq!(result.len(), 2);
    let selected = result[0].get("key").unwrap();
    assert!(std::ptr::eq(selected.origin, &attributes[3]));
    assert_eq!(selected.entry, Some(0));
    assert_eq!(selected.value, &flag(false));
    assert!(std::ptr::eq(
        result[1].get("key").unwrap().origin,
        &attributes[1]
    ));
    assert!(result[0].to_string().contains("(source 40 41)"));
    let duplicate = merge(&attributes[..1], 10, &mut Budget::default()).unwrap();
    assert_eq!(duplicate[0].get("key").unwrap().entry, Some(1));
}
#[test]
fn scopes_and_member_targets_survive_without_copying_payloads() {
    let mut attribute = attribute(target(3), None, &[("quote\"\\\n", true)], 12);
    let owner = OwnerId {
        def_id: LocalDefId::new(3),
    };
    attribute.target = AttributeTarget::Member(HirId::new(owner, ItemLocalId::new(1)));
    let constructor = attribute.value.clone();
    attribute.value.kind = K::Block {
        statements: vec![],
        result: Some(Box::new(constructor)),
    };
    let attributes = vec![attribute];
    let result = merge(&attributes, 1, &mut Budget::default()).unwrap();
    assert_eq!(result[0].target, attributes[0].target);
    let selected = result[0].get("quote\"\\\n").unwrap();
    assert!(std::ptr::eq(selected.origin, &attributes[0]));
    assert!(matches!(selected.origin.value.kind, K::Block { .. }));
    assert!(result[0].to_string().contains("quote\\\"\\\\\\n"));
}
#[test]
fn malformed_positional_and_budget_failures_preserve_inputs_and_allow_retry() {
    let mut attributes = vec![attribute(target(1), None, &[("x", true), ("y", false)], 5)];
    assert!(merge(&attributes, 1, &mut Budget::default()).is_err());
    let before = attributes[0].to_string();
    assert!(
        merge(
            &attributes,
            2,
            &mut Budget {
                steps: 0,
                ..Budget::default()
            }
        )
        .is_err()
    );
    assert_eq!(attributes[0].to_string(), before);
    assert_eq!(
        merge(&attributes, 2, &mut Budget::default()).unwrap()[0]
            .iter()
            .count(),
        2
    );
    let K::Object { names, .. } = &mut attributes[0].value.kind else {
        panic!()
    };
    names[1] = None;
    let failure = merge(&attributes, 2, &mut Budget::default()).unwrap_err();
    assert_eq!(failure.source, attributes[0].source);
    assert!(failure.message.contains("positional Object entry 1"));
    let K::Object { names, .. } = &mut attributes[0].value.kind else {
        panic!()
    };
    names.pop();
    assert!(
        merge(&attributes, 2, &mut Budget::default())
            .unwrap_err()
            .message
            .contains("shape")
    );
}

#[test]
fn every_insufficient_budget_is_atomic_and_reports_a_real_origin() {
    for attributes in [
        vec![attribute(target(1), None, &[], 100)],
        vec![attribute(
            target(1),
            None,
            &[("key", true), ("key", false)],
            100,
        )],
    ] {
        let before = attributes[0].to_string();
        let mut budget = Budget::default();
        let initial = budget.steps;
        merge(&attributes, 10, &mut budget).unwrap();
        let required = initial - budget.steps;
        for steps in 0..required {
            let failure = merge(
                &attributes,
                10,
                &mut Budget {
                    steps,
                    ..Budget::default()
                },
            )
            .unwrap_err();
            assert_eq!(failure.source, attributes[0].source, "budget {steps}");
            assert_eq!(attributes[0].to_string(), before);
        }
        merge(
            &attributes,
            10,
            &mut Budget {
                steps: required,
                ..Budget::default()
            },
        )
        .unwrap();
    }
}

#[test]
fn repeated_overrides_keep_only_final_origin_and_limit_failure_names_its_input() {
    let mut attributes = (0..1000)
        .map(|offset| attribute(target(1), None, &[("key", offset % 2 == 0)], offset))
        .collect::<Vec<_>>();
    let selected = merge(&attributes, 1, &mut Budget::default()).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].iter().count(), 1);
    assert!(std::ptr::eq(
        selected[0].get("key").unwrap().origin,
        &attributes[999]
    ));
    attributes.push(attribute(target(1), None, &[("new", true)], 2000));
    let failure = merge(&attributes, 1, &mut Budget::default()).unwrap_err();
    assert_eq!(failure.source, attributes[1000].source);
    assert!(failure.message.contains("entry-limit"));
    assert_eq!(
        merge(&attributes, 2, &mut Budget::default()).unwrap()[0]
            .iter()
            .count(),
        2
    );
}
