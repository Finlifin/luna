use super::*;
use nir::program::Pattern;

fn id(index: u32) -> LocalId {
    LocalId { owner: 1, index }
}
fn binding(index: u32) -> Pattern {
    Pattern::Binding {
        binding: id(index),
        inner: Box::new(Pattern::Wild),
    }
}
fn remap(pattern: Pattern, bindings: &[(u32, u32)]) -> Pattern {
    Pattern::Remap {
        pattern: Box::new(pattern),
        bindings: bindings.iter().map(|(a, b)| (id(*a), id(*b))).collect(),
    }
}

#[test]
fn computed_local_cannot_reintroduce_a_removed_pattern_export() {
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let before = Environment::from([(id(0), Value::Bool(true))]);
    let mut environment = before.clone();
    let source = SourceRange { start: 1, end: 2 };
    let pattern = remap(
        Pattern::Then {
            first: Box::new(remap(binding(0), &[(0, 1)])),
            value: Box::new(Expr {
                ty: Term::Primitive(Primitive::Unit),
                source,
                kind: K::Let {
                    binding: id(0),
                    value: Box::new(Expr {
                        ty: Term::Primitive(Primitive::Bool),
                        source,
                        kind: K::Literal(Literal::Bool(false)),
                    }),
                },
            }),
            next: Box::new(Pattern::Wild),
        },
        &[(0, 2)],
    );
    let error = vm
        .match_pattern(&pattern, &Value::Bool(false), &mut environment)
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidCallPlan);
    assert_eq!(environment, before);
}

#[test]
fn remap_cannot_import_an_ambient_binding() {
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let mut environment = Environment::from([(id(0), Value::Bool(true))]);
    let error = vm
        .match_pattern(
            &remap(Pattern::Wild, &[(0, 1)]),
            &Value::Bool(false),
            &mut environment,
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidCallPlan);
    assert_eq!(environment, Environment::from([(id(0), Value::Bool(true))]));
}

#[test]
fn remap_rejects_duplicate_sources_and_merged_exports() {
    for mappings in [vec![(0, 2), (0, 3)], vec![(0, 2), (1, 2)], vec![(0, 1)]] {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let mut environment = Environment::new();
        let pattern = remap(Pattern::Tuple(vec![binding(0), binding(1)]), &mappings);
        let error = vm
            .match_pattern(
                &pattern,
                &Value::Tuple(vec![Value::Bool(false), Value::Bool(true)]),
                &mut environment,
            )
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidCallPlan);
        assert!(environment.is_empty());
    }
}

#[test]
fn remap_cannot_reuse_a_failed_candidates_restored_outer_binding() {
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let mut environment = Environment::from([(id(0), Value::Bool(true))]);
    let pattern = Pattern::Or(vec![
        Pattern::Tuple(vec![binding(0), Pattern::Literal(Literal::Bool(true))]),
        remap(Pattern::Wild, &[(0, 1)]),
    ]);
    assert_eq!(
        vm.match_pattern(
            &pattern,
            &Value::Tuple(vec![Value::Bool(false), Value::Bool(false)]),
            &mut environment
        )
        .unwrap_err()
        .kind,
        ErrorKind::InvalidCallPlan
    );
    assert_eq!(environment, Environment::from([(id(0), Value::Bool(true))]));
}

#[test]
fn remap_swaps_and_chains_use_the_original_candidate_values() {
    for (mappings, expected) in [
        (
            vec![(0, 1), (1, 0)],
            vec![(id(0), Value::Bool(true)), (id(1), Value::Bool(false))],
        ),
        (
            vec![(0, 1), (1, 2)],
            vec![(id(1), Value::Bool(false)), (id(2), Value::Bool(true))],
        ),
    ] {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let mut environment = Environment::new();
        let pattern = remap(Pattern::Tuple(vec![binding(0), binding(1)]), &mappings);
        assert!(matches!(
            vm.match_pattern(
                &pattern,
                &Value::Tuple(vec![Value::Bool(false), Value::Bool(true)]),
                &mut environment
            )
            .unwrap(),
            Flow::Value(Value::Bool(true))
        ));
        assert_eq!(environment, Environment::from_iter(expected));
    }
}

#[test]
fn failed_guard_restores_both_remap_sources_and_destinations() {
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let before = Environment::from([(id(0), Value::Bool(true)), (id(1), Value::Bool(true))]);
    let mut environment = before.clone();
    let pattern = Pattern::Guard {
        pattern: Box::new(remap(binding(0), &[(0, 1)])),
        condition: Box::new(Expr {
            ty: Term::Primitive(Primitive::Bool),
            source: SourceRange { start: 1, end: 2 },
            kind: K::Literal(Literal::Bool(false)),
        }),
    };
    assert!(matches!(
        vm.match_pattern(&pattern, &Value::Bool(false), &mut environment)
            .unwrap(),
        Flow::Value(Value::Bool(false))
    ));
    assert_eq!(environment, before);
}

#[test]
fn nested_remap_exports_only_its_live_destination() {
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let mut environment = Environment::new();
    let valid = remap(remap(binding(0), &[(0, 1)]), &[(1, 2)]);
    assert!(matches!(
        vm.match_pattern(&valid, &Value::Bool(true), &mut environment)
            .unwrap(),
        Flow::Value(Value::Bool(true))
    ));
    assert_eq!(environment, Environment::from([(id(2), Value::Bool(true))]));
    let invalid = remap(remap(binding(0), &[(0, 1)]), &[(0, 3)]);
    assert_eq!(
        vm.match_pattern(&invalid, &Value::Bool(false), &mut environment)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidCallPlan
    );
    assert_eq!(environment, Environment::from([(id(2), Value::Bool(true))]));
}

#[test]
fn exhausted_remap_restores_a_partially_matched_candidate() {
    let package = Package::default();
    let mut vm = Interpreter::new(
        &package,
        Budget {
            steps: 4,
            ..Budget::default()
        },
    )
    .unwrap();
    let before = Environment::from([(id(0), Value::Bool(true))]);
    let mut environment = before.clone();
    let pattern = remap(binding(0), &[(0, 1)]);
    assert_eq!(
        vm.match_pattern(&pattern, &Value::Bool(false), &mut environment)
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(environment, before);
    let mut fresh = Interpreter::new(&package, Budget::default()).unwrap();
    let mut recovered = Environment::new();
    assert!(matches!(
        fresh
            .match_pattern(&pattern, &Value::Bool(false), &mut recovered)
            .unwrap(),
        Flow::Value(Value::Bool(true))
    ));
    assert_eq!(recovered, Environment::from([(id(1), Value::Bool(false))]));
}

#[test]
fn pattern_receipt_tracks_only_successful_candidate_positions() {
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let mut environment = Environment::new();
    let pattern = Pattern::Or(vec![
        Pattern::Tuple(vec![binding(0), Pattern::Literal(Literal::Bool(true))]),
        remap(Pattern::Tuple(vec![Pattern::Wild, binding(1)]), &[(1, 0)]),
    ]);
    let (flow, receipt) = vm
        .match_pattern_with_bindings(
            &pattern,
            &Value::Tuple(vec![Value::Bool(true), Value::Bool(false)]),
            &mut environment,
        )
        .unwrap();
    assert!(matches!(flow, Flow::Value(Value::Bool(true))));
    assert_eq!(receipt.0, BTreeMap::from([(id(0), Some(vec![1]))]));
    assert_eq!(
        environment,
        Environment::from([(id(0), Value::Bool(false))])
    );
    assert!(receipt.to_string().contains("(subject 1)"));
}

#[test]
fn pattern_receipt_remaps_nested_paths_simultaneously() {
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let pattern = remap(
        Pattern::Tuple(vec![binding(0), Pattern::Tuple(vec![binding(1)])]),
        &[(0, 1), (1, 0)],
    );
    let (_, receipt) = vm
        .match_pattern_with_bindings(
            &pattern,
            &Value::Tuple(vec![
                Value::Bool(true),
                Value::Tuple(vec![Value::Bool(false)]),
            ]),
            &mut Environment::new(),
        )
        .unwrap();
    assert_eq!(
        receipt.0,
        BTreeMap::from([(id(0), Some(vec![1, 0])), (id(1), Some(vec![0]))])
    );
    assert_eq!(receipt.1, BTreeMap::from([(id(0), 1), (id(1), 0)]));
}

#[test]
fn pattern_receipt_does_not_treat_computed_values_as_subject_fields() {
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let pattern = Pattern::Then {
        first: Box::new(binding(0)),
        value: Box::new(Expr {
            ty: Term::Primitive(Primitive::Bool),
            source: SourceRange { start: 1, end: 2 },
            kind: K::Literal(Literal::Bool(true)),
        }),
        next: Box::new(binding(1)),
    };
    let (_, receipt) = vm
        .match_pattern_with_bindings(&pattern, &Value::Bool(false), &mut Environment::new())
        .unwrap();
    assert_eq!(
        receipt.0,
        BTreeMap::from([(id(0), Some(vec![])), (id(1), None)])
    );
    assert!(receipt.to_string().contains("(derived)"));
}

#[test]
fn pattern_receipt_is_discarded_when_external_guard_fails() {
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let guard = Expr {
        ty: Term::Primitive(Primitive::Bool),
        source: SourceRange { start: 1, end: 2 },
        kind: K::Literal(Literal::Bool(false)),
    };
    let (flow, receipt) = vm
        .match_arm_guard_with_bindings(
            &binding(0),
            Some(&guard),
            &Value::Bool(true),
            &mut Environment::new(),
        )
        .unwrap();
    assert!(matches!(flow, Flow::Value(Value::Bool(false))));
    assert!(receipt.0.is_empty());
    assert!(receipt.1.is_empty());
}

#[test]
fn abstract_pattern_code_cannot_match_or_export_runtime_bindings() {
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let mut environment = Environment::from([(id(0), Value::Bool(true))]);
    let before = environment.clone();
    let pattern = Pattern::CodeEvidence {
        outputs: vec![(id(1), Term::Primitive(Primitive::Bool))],
    };
    let error = vm
        .match_pattern(&pattern, &Value::Bool(false), &mut environment)
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::TypeContractUnresolved);
    assert_eq!(environment, before);
}
