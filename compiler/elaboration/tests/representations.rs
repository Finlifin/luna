use elaboration::representations::Checker;
use nir::{DeclarationId, Primitive, Sort, Term, program::Struct};
use std::collections::BTreeMap;
use std::sync::Arc;

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 71, local }
}
fn nominal(local: u32) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![],
    }
}
fn definition(local: u32, field: Term) -> Struct {
    Struct {
        declaration: id(local),
        name: format!("S{local}"),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        fields: vec![("field".into(), field)],
    }
}
fn enumeration(local: u32, field: Term) -> nir::program::Enum {
    nir::program::Enum {
        declaration: id(local),
        name: format!("E{local}"),
        is_intrinsic: false,
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        groups: vec![],
        encoding: None,
        symbol_encoding: None,
        variants: vec![nir::program::Variant {
            name: "Value".into(),
            shape: nir::program::VariantShape::Tuple,
            fields: vec![field],
        }],
    }
}

#[test]
fn mixed_representation_batches_reject_cross_kind_cycles_without_leaking_proofs() {
    use elaboration::representations::{FailureKind, IncrementalChecker};
    let package = nir::program::Package::default();
    let mut checker = IncrementalChecker::new(&package, &mut 100_000).unwrap();
    let failure = checker
        .prepare_batch(
            vec![definition(4, nominal(3)), definition(1, nominal(2))].into(),
            vec![
                enumeration(3, Term::Primitive(Primitive::Bool)),
                enumeration(2, nominal(1)),
            ]
            .into(),
            &mut 100_000,
        )
        .err()
        .unwrap();
    assert_eq!(failure.kind, FailureKind::Invalid);
    // Neither a successful root nor the failed root may survive preparation.
    assert!(
        checker
            .prepare_batch(
                Arc::from([]),
                vec![enumeration(3, nominal(3))].into(),
                &mut 100_000
            )
            .is_err()
    );
    let prepared = checker
        .prepare_batch(
            vec![definition(1, nominal(2))].into(),
            vec![enumeration(2, Term::Pointer(Box::new(nominal(1))))].into(),
            &mut 100_000,
        )
        .unwrap();
    assert!(prepared.to_string().contains("(enums"));
    checker.commit(prepared).unwrap();
    let later = checker
        .prepare(vec![definition(4, nominal(2))].into(), &mut 100_000)
        .unwrap();
    checker.commit(later).unwrap();
}

#[test]
fn enum_representation_receipts_share_revision_and_identity_with_structs() {
    use elaboration::representations::{FailureKind, IncrementalChecker};
    let package = nir::program::Package::default();
    let mut checker = IncrementalChecker::new(&package, &mut 100_000).unwrap();
    let mut foreign = IncrementalChecker::new(&package, &mut 100_000).unwrap();
    let enums: Arc<[nir::program::Enum]> =
        vec![enumeration(2, Term::Primitive(Primitive::Bool))].into();
    let weak = Arc::downgrade(&enums);
    let stale = checker
        .prepare_batch(Arc::from([]), Arc::clone(&enums), &mut 100_000)
        .unwrap();
    let receipt = checker
        .prepare_batch(Arc::from([]), Arc::clone(&enums), &mut 100_000)
        .unwrap();
    assert_eq!(
        foreign.commit(receipt).unwrap_err().kind,
        FailureKind::Stale
    );
    assert!(
        checker
            .prepare_batch(
                vec![definition(2, Term::Primitive(Primitive::Bool))].into(),
                Arc::clone(&enums),
                &mut 100_000
            )
            .is_err()
    );
    let first = checker
        .prepare(
            vec![definition(1, Term::Primitive(Primitive::Bool))].into(),
            &mut 100_000,
        )
        .unwrap();
    checker.commit(first).unwrap();
    assert_eq!(checker.commit(stale).unwrap_err().kind, FailureKind::Stale);
    let receipt = checker
        .prepare_batch(Arc::from([]), enums, &mut 100_000)
        .unwrap();
    checker.commit(receipt).unwrap();
    assert!(weak.upgrade().is_some());
    assert!(
        checker
            .prepare(
                vec![definition(2, Term::Primitive(Primitive::Bool))].into(),
                &mut 100_000
            )
            .is_err()
    );
    drop(checker);
    assert!(weak.upgrade().is_none());
}

#[test]
fn alternating_enum_and_struct_publication_reuses_prior_representation_proofs() {
    use elaboration::representations::IncrementalChecker;
    let package = nir::program::Package::default();
    let mut checker = IncrementalChecker::new(&package, &mut 100_000).unwrap();
    let mut costs = [None, None];
    for local in 1..=256 {
        let field = if local == 1 {
            Term::Primitive(Primitive::Bool)
        } else {
            nominal(local - 1)
        };
        let is_enum = local % 2 == 0;
        let mut fuel = 100_000;
        let receipt = if is_enum {
            checker.prepare_batch(
                Arc::from([]),
                vec![enumeration(local, field)].into(),
                &mut fuel,
            )
        } else {
            checker.prepare(vec![definition(local, field)].into(), &mut fuel)
        }
        .unwrap();
        if local > 1 {
            let cost = 100_000 - fuel;
            let previous = &mut costs[usize::from(is_enum)];
            if let Some(expected) = previous {
                assert_eq!(cost, *expected);
            } else {
                *previous = Some(cost);
            }
        }
        checker.commit(receipt).unwrap();
    }
}

#[test]
fn mixed_representation_budget_failures_are_retryable_and_do_not_publish() {
    use elaboration::representations::{FailureKind, IncrementalChecker};
    let package = nir::program::Package::default();
    let mut checker = IncrementalChecker::new(&package, &mut 100_000).unwrap();
    let structs: Arc<[Struct]> = vec![definition(1, nominal(2))].into();
    let enums: Arc<[nir::program::Enum]> =
        vec![enumeration(2, Term::Primitive(Primitive::Bool))].into();
    let mut fuel = 100_000;
    drop(
        checker
            .prepare_batch(Arc::clone(&structs), Arc::clone(&enums), &mut fuel)
            .unwrap(),
    );
    let cost = 100_000 - fuel;
    for budget in 0..cost {
        let mut remaining = budget;
        assert_eq!(
            checker
                .prepare_batch(Arc::clone(&structs), Arc::clone(&enums), &mut remaining)
                .err()
                .unwrap()
                .kind,
            FailureKind::Budget
        );
        assert!(
            checker
                .prepare_batch(
                    Arc::from([]),
                    vec![enumeration(2, nominal(2))].into(),
                    &mut 100_000
                )
                .is_err()
        );
    }
    let receipt = checker.prepare_batch(structs, enums, &mut 100_000).unwrap();
    checker.commit(receipt).unwrap();
}
fn check(
    field: Term,
    tuple: Option<DeclarationId>,
) -> Result<(), elaboration::representations::Failure> {
    Checker::new(
        tuple,
        &BTreeMap::from([(id(0), definition(0, field))]),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .check(&nominal(0), &mut 100_000)
}

#[test]
fn incremental_dropped_and_failed_candidates_do_not_publish_proofs() {
    use elaboration::representations::IncrementalChecker;
    let package = nir::program::Package::default();
    let checker = IncrementalChecker::new(&package, &mut 100_000).unwrap();
    let candidate: Arc<[Struct]> = vec![definition(1, Term::Primitive(Primitive::Bool))].into();
    let weak = Arc::downgrade(&candidate);
    let prepared = checker.prepare(candidate, &mut 100_000).unwrap();
    assert!(
        prepared
            .to_string()
            .starts_with("(prepared-representations (revision 0)")
    );
    drop(prepared);
    assert!(weak.upgrade().is_none());
    assert!(
        checker
            .prepare(vec![definition(1, nominal(1))].into(), &mut 100_000)
            .is_err()
    );
    // The first root succeeds before the second root fails. Neither proof may
    // survive the failed batch, even if its identity is reused for a new layout.
    assert!(
        checker
            .prepare(
                vec![
                    definition(1, Term::Primitive(Primitive::Bool)),
                    definition(2, nominal(2))
                ]
                .into(),
                &mut 100_000
            )
            .is_err()
    );
    assert!(
        checker
            .prepare(vec![definition(1, nominal(1))].into(), &mut 100_000)
            .is_err()
    );
    assert_eq!(
        checker.to_string(),
        "(representation-world (revision 0) (proved 0))"
    );
}

#[test]
fn incremental_receipts_are_revision_and_world_bound() {
    use elaboration::representations::IncrementalChecker;
    let package = nir::program::Package::default();
    let mut first = IncrementalChecker::new(&package, &mut 100_000).unwrap();
    let mut other = IncrementalChecker::new(&package, &mut 100_000).unwrap();
    let make = || vec![definition(1, Term::Primitive(Primitive::Bool))].into();
    let foreign = first.prepare(make(), &mut 100_000).unwrap();
    assert!(other.commit(foreign).is_err());
    let current = first.prepare(make(), &mut 100_000).unwrap();
    let stale = first
        .prepare(
            vec![definition(2, Term::Primitive(Primitive::Bool))].into(),
            &mut 100_000,
        )
        .unwrap();
    first.commit(current).unwrap();
    assert!(first.commit(stale).is_err());
    assert!(
        first
            .prepare(vec![definition(3, nominal(2))].into(), &mut 100_000)
            .is_err()
    );
    assert!(
        first
            .prepare(vec![definition(1, nominal(1))].into(), &mut 100_000)
            .is_err()
    );
    let valid = first
        .prepare(vec![definition(2, nominal(1))].into(), &mut 100_000)
        .unwrap();
    first.commit(valid).unwrap();
    assert_eq!(
        first.to_string(),
        "(representation-world (revision 2) (proved 2))"
    );
}

#[test]
fn incremental_checks_reuse_committed_chain_proofs_without_rescanning_the_world() {
    use elaboration::representations::IncrementalChecker;
    let package = nir::program::Package {
        structs: (0..256)
            .map(|n| definition(n, Term::Primitive(Primitive::Bool)))
            .collect(),
        ..Default::default()
    };
    let mut checker = IncrementalChecker::new(&package, &mut 100_000).unwrap();
    let mut steady_cost = None;
    for index in 256..512 {
        let mut fuel = 200;
        let prepared = checker
            .prepare(
                vec![definition(index, nominal(index - 1))].into(),
                &mut fuel,
            )
            .unwrap();
        // Once the baseline leaf is proved, each new wrapper checks one new
        // layout and a cached child; chain depth and world size do not add cost.
        if index > 256 {
            let cost = 200 - fuel;
            assert_eq!(*steady_cost.get_or_insert(cost), cost);
        }
        checker.commit(prepared).unwrap();
    }
}

#[test]
fn incremental_exhaustion_and_recursive_pointer_retry_leave_world_unchanged() {
    use elaboration::representations::IncrementalChecker;
    let package = nir::program::Package::default();
    let mut checker = IncrementalChecker::new(&package, &mut 100_000).unwrap();
    let large: Arc<[Struct]> = vec![definition(
        1,
        Term::Tuple(vec![Term::Primitive(Primitive::Bool); 512]),
    )]
    .into();
    for allowance in [0, 1, 10, 100] {
        let mut fuel = allowance;
        assert!(checker.prepare(Arc::clone(&large), &mut fuel).is_err());
        assert!(fuel <= allowance);
        assert_eq!(
            checker.to_string(),
            "(representation-world (revision 0) (proved 0))"
        );
    }
    let pointer = checker
        .prepare(
            vec![definition(1, Term::Pointer(Box::new(nominal(1))))].into(),
            &mut 100_000,
        )
        .unwrap();
    checker.commit(pointer).unwrap();
    let wrapper = checker
        .prepare(vec![definition(2, nominal(1))].into(), &mut 100_000)
        .unwrap();
    checker.commit(wrapper).unwrap();
}

#[test]
fn incremental_batch_forward_edges_are_checked_before_any_proof_is_committed() {
    use elaboration::representations::IncrementalChecker;
    let package = nir::program::Package::default();
    let mut checker = IncrementalChecker::new(&package, &mut 100_000).unwrap();
    let cycle = vec![definition(1, nominal(2)), definition(2, nominal(1))];
    assert!(checker.prepare(cycle.into(), &mut 100_000).is_err());
    let valid = vec![
        definition(1, nominal(2)),
        definition(2, Term::Primitive(Primitive::Bool)),
    ];
    let prepared = checker.prepare(valid.into(), &mut 100_000).unwrap();
    checker.commit(prepared).unwrap();
    let downstream = checker
        .prepare(vec![definition(3, nominal(1))].into(), &mut 100_000)
        .unwrap();
    checker.commit(downstream).unwrap();
    assert_eq!(
        checker.to_string(),
        "(representation-world (revision 2) (proved 3))"
    );
}

#[test]
fn inline_storage_wrappers_cannot_hide_recursive_payloads() {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    for field in [
        nominal(0),
        Term::Optional(Box::new(nominal(0))),
        Term::Record(vec![("nested".into(), nominal(0))]),
        Term::Tuple(vec![nominal(0)]),
        Term::Closure {
            environment: vec![nominal(0)],
            signature: Box::new(signature),
        },
        Term::ErrorResult {
            errors: vec![nominal(0)],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        },
        Term::ErrorResult {
            errors: vec![Term::Primitive(Primitive::Bool)],
            result: Box::new(nominal(0)),
        },
    ] {
        assert!(
            check(field.clone(), None)
                .unwrap_err()
                .to_string()
                .contains("recursive by-value"),
            "{field}"
        );
    }
}

#[test]
fn pointer_boundaries_and_opaque_heads_remain_finite() {
    for field in [
        Term::Optional(Box::new(Term::Pointer(Box::new(nominal(0))))),
        Term::Function {
            parameters: vec![],
            result: Box::new(nominal(0)),
        },
        Term::NormalForm {
            head: id(5),
            sort: Sort::Type,
            arguments: vec![nominal(0)],
        },
    ] {
        check(field, None).unwrap();
    }
}

#[test]
fn only_bound_tuple_head_exposes_its_type_list_storage() {
    let field = Term::NormalForm {
        head: id(5),
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(vec![nominal(0)])],
    };
    check(field.clone(), None).unwrap();
    assert!(
        check(field, Some(id(5)))
            .unwrap_err()
            .to_string()
            .contains("recursive by-value")
    );
}

#[test]
fn forward_edges_and_failed_roots_do_not_poison_completed_roots() {
    let structs = BTreeMap::from([
        (id(0), definition(0, Term::Optional(Box::new(nominal(1))))),
        (id(1), definition(1, Term::Primitive(Primitive::Bool))),
        (id(2), definition(2, nominal(2))),
    ]);
    let enums = BTreeMap::new();
    let newtypes = BTreeMap::new();
    let mut checker = Checker::new(None, &structs, &enums, &newtypes);
    let mut fuel = 100_000;
    assert!(checker.check(&nominal(2), &mut fuel).is_err());
    checker.check(&nominal(0), &mut fuel).unwrap();
    checker.check(&nominal(1), &mut fuel).unwrap();
    assert!(checker.check(&nominal(2), &mut fuel).is_err());
    assert!(
        checker
            .check(&nominal(99), &mut fuel)
            .unwrap_err()
            .to_string()
            .contains("missing")
    );
}

#[test]
fn expanding_generic_recursion_is_bounded_before_materialization() {
    let parameter = id(8);
    let mut def = definition(
        0,
        Term::Nominal {
            declaration: id(0),
            arguments: vec![Term::Optional(Box::new(Term::Rigid {
                declaration: parameter,
                sort: Sort::Type,
            }))],
        },
    );
    def.type_parameters.push(parameter);
    let structs = BTreeMap::from([(id(0), def)]);
    let enums = BTreeMap::new();
    let newtypes = BTreeMap::new();
    let mut checker = Checker::new(None, &structs, &enums, &newtypes);
    let root = Term::Nominal {
        declaration: id(0),
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    assert!(
        checker
            .check(&root, &mut 300)
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
    assert!(
        checker
            .check(&root, &mut 100_000)
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
}

#[test]
fn exhausted_attempt_can_be_retried_with_fresh_allowance() {
    let structs = BTreeMap::from([(id(0), definition(0, Term::Primitive(Primitive::Bool)))]);
    let enums = BTreeMap::new();
    let newtypes = BTreeMap::new();
    let mut checker = Checker::new(None, &structs, &enums, &newtypes);
    assert!(
        checker
            .check(&nominal(0), &mut 0)
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
    checker.check(&nominal(0), &mut 1000).unwrap();
}

#[test]
fn mixed_struct_enum_newtype_cycle_follows_payload_storage() {
    use nir::program::{Enum, Variant, VariantShape};
    let structs = BTreeMap::from([(id(0), definition(0, nominal(1)))]);
    let enums = BTreeMap::from([(
        id(1),
        Enum {
            is_intrinsic: false,
            declaration: id(1),
            name: "Branch".into(),
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
            variants: vec![Variant {
                name: "payload".into(),
                shape: VariantShape::Tuple,
                fields: vec![nominal(2)],
            }],
        },
    )]);
    let mut wrapper = nir::newtypes::Newtype {
        declaration: id(2),
        name: "Wrapper".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation: Term::Optional(Box::new(nominal(0))),
    };
    assert!(
        Checker::new(
            None,
            &structs,
            &enums,
            &BTreeMap::from([(id(2), wrapper.clone())])
        )
        .check(&nominal(0), &mut 100_000)
        .unwrap_err()
        .to_string()
        .contains("recursive by-value")
    );
    wrapper.representation = Term::Pointer(Box::new(nominal(0)));
    Checker::new(None, &structs, &enums, &BTreeMap::from([(id(2), wrapper)]))
        .check(&nominal(0), &mut 100_000)
        .unwrap();
}

#[test]
fn representation_failures_distinguish_limits_invalid_layouts_and_stale_receipts() {
    use elaboration::representations::{FailureKind, IncrementalChecker};
    let package = nir::program::Package::default();
    assert_eq!(
        IncrementalChecker::new(&package, &mut 0)
            .err()
            .unwrap()
            .kind,
        FailureKind::Budget
    );
    let mut checker = IncrementalChecker::new(&package, &mut 1000).unwrap();
    let simple = || vec![definition(1, Term::Primitive(Primitive::Bool))].into();
    assert_eq!(
        checker.prepare(simple(), &mut 0).err().unwrap().kind,
        FailureKind::Budget
    );
    assert_eq!(
        checker
            .prepare(vec![definition(1, nominal(1))].into(), &mut 1000)
            .err()
            .unwrap()
            .kind,
        FailureKind::Invalid
    );
    let first = checker.prepare(simple(), &mut 1000).unwrap();
    let stale = checker.prepare(simple(), &mut 1000).unwrap();
    checker.commit(first).unwrap();
    assert_eq!(checker.commit(stale).unwrap_err().kind, FailureKind::Stale);
}

#[test]
fn newtype_expansion_preserves_budget_and_type_failure_categories() {
    use elaboration::representations::FailureKind;
    let wrapper = nir::newtypes::Newtype {
        declaration: id(1),
        name: "Wide".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation: Term::Tuple(vec![Term::Primitive(Primitive::Bool); 128]),
    };
    let structs = BTreeMap::new();
    let enums = BTreeMap::new();
    let newtypes = BTreeMap::from([(id(1), wrapper)]);
    let mut checker = Checker::new(None, &structs, &enums, &newtypes);
    assert_eq!(
        checker.check(&nominal(1), &mut 50).unwrap_err().kind,
        FailureKind::Budget
    );
    checker.check(&nominal(1), &mut 100_000).unwrap();
    let malformed = Term::Nominal {
        declaration: id(1),
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    assert_eq!(
        checker.check(&malformed, &mut 100_000).unwrap_err().kind,
        FailureKind::Invalid
    );
}
