use elaboration::derivation::{Applicability as A, DefaultStrategy, Registry, Selection};
use elaboration::implementation::{Error, Ownership, OwnershipError, default_strategy_ownership};
use elaboration::ptc::Budget;
use nir::{DeclarationId as Id, Sort, Term};
use std::collections::BTreeSet;
fn id(package: u64, local: u32) -> Id {
    Id { package, local }
}
fn nominal(package: u64) -> Term {
    Term::Nominal {
        declaration: id(package, 9),
        arguments: vec![],
    }
}
fn open() -> Term {
    Term::Rigid {
        declaration: id(1, 99),
        sort: Sort::Type,
    }
}
fn strategy(local: u32) -> DefaultStrategy {
    DefaultStrategy {
        declaration: id(1, local),
        interface: id(1, 1),
        interface_arguments: vec![Term::Bool(local % 2 == 0)],
    }
}
fn registry() -> Registry {
    let mut registry = Registry::default();
    for local in [4, 2, 3] {
        registry
            .register(strategy(local), Budget::default())
            .unwrap();
    }
    registry
}
fn visible() -> BTreeSet<Id> {
    [id(1, 2), id(1, 3), id(1, 4)].into_iter().collect()
}

#[test]
fn registration_uses_trait_arguments_not_a_future_subject() {
    assert_eq!(
        default_strategy_ownership(id(1, 2), id(1, 1), &[open()], Budget::default()).unwrap(),
        Ownership::LocalInterface(id(1, 1))
    );
    assert_eq!(
        default_strategy_ownership(id(1, 2), id(2, 1), &[], Budget::default()),
        Err(Error::Ownership(OwnershipError::NoLocalIdentity))
    );
    assert_eq!(
        default_strategy_ownership(id(1, 2), id(2, 1), &[nominal(1), open()], Budget::default())
            .unwrap(),
        Ownership::LocalNominal(id(1, 9))
    );
    assert_eq!(
        default_strategy_ownership(id(1, 2), id(2, 1), &[open(), nominal(1)], Budget::default()),
        Err(Error::Ownership(OwnershipError::OpenBeforeLocalIdentity))
    );
}

#[test]
fn foreign_wrappers_preserve_order_and_opaque_heads_do_not_grant_ownership() {
    let wrapped = Term::Nominal {
        declaration: id(2, 8),
        arguments: vec![nominal(1), open()],
    };
    assert!(default_strategy_ownership(id(1, 2), id(2, 1), &[wrapped], Budget::default()).is_ok());
    let opaque = Term::NormalForm {
        sort: Sort::Type,
        head: id(1, 10),
        arguments: vec![nominal(1)],
    };
    assert_eq!(
        default_strategy_ownership(id(1, 2), id(2, 1), &[opaque], Budget::default()),
        Err(Error::Ownership(OwnershipError::OpenBeforeLocalIdentity))
    );
}

#[test]
fn registration_is_atomic_and_import_identity_is_deduplicated() {
    let mut registry = Registry::default();
    registry.register(strategy(2), Budget::default()).unwrap();
    registry.register(strategy(2), Budget::default()).unwrap();
    let mut conflict = strategy(2);
    conflict.interface_arguments = vec![Term::Bool(false)];
    assert_eq!(
        registry.register(conflict, Budget::default()),
        Err(Error::DuplicateDeclaration(id(1, 2)))
    );
    let mut invalid = strategy(3);
    invalid.interface = id(2, 1);
    assert!(registry.register(invalid, Budget::default()).is_err());
    let mut calls = 0;
    assert_eq!(
        registry
            .select(id(1, 1), &visible(), &mut 100, |head, _| {
                calls += 1;
                assert_eq!(head.interface_arguments, vec![Term::Bool(true)]);
                Ok(A::Applicable)
            })
            .unwrap(),
        Selection::Unique(id(1, 2))
    );
    assert_eq!(calls, 1);
}

#[test]
fn visibility_and_complete_interface_assessment_control_selection() {
    let registry = registry();
    let visible = [id(1, 2), id(1, 3)].into_iter().collect();
    assert_eq!(
        registry
            .select(id(1, 1), &visible, &mut 100, |head, _| Ok(
                if head.interface_arguments == vec![Term::Bool(true)] {
                    A::Applicable
                } else {
                    A::Rejected
                }
            ))
            .unwrap(),
        Selection::Unique(id(1, 2))
    );
    assert_eq!(
        registry
            .select(id(2, 1), &visible, &mut 100, |_, _| panic!(
                "wrong interface bucket"
            ))
            .unwrap(),
        Selection::Missing
    );
    assert_eq!(
        registry
            .select(id(1, 1), &BTreeSet::new(), &mut 100, |_, _| panic!(
                "invisible candidate"
            ))
            .unwrap(),
        Selection::Missing
    );
}

#[test]
fn unresolved_competitors_prevent_early_selection_and_can_be_retried() {
    let registry = registry();
    assert_eq!(
        registry
            .select(id(1, 1), &visible(), &mut 100, |head, _| Ok(
                match head.declaration.local {
                    2 => A::Applicable,
                    3 => A::Pending,
                    _ => A::Rejected,
                }
            ))
            .unwrap(),
        Selection::Pending {
            applicable: vec![id(1, 2)],
            pending: vec![id(1, 3)]
        }
    );
    assert_eq!(
        registry
            .select(id(1, 1), &visible(), &mut 100, |head, _| Ok(
                if head.declaration.local == 2 {
                    A::Applicable
                } else {
                    A::Rejected
                }
            ))
            .unwrap(),
        Selection::Unique(id(1, 2))
    );
}

#[test]
fn multiple_applicable_strategies_are_ambiguous_without_priority() {
    let registry = registry();
    let selected = registry
        .select(id(1, 1), &visible(), &mut 100, |_, _| Ok(A::Applicable))
        .unwrap();
    assert_eq!(
        selected,
        Selection::Ambiguous(vec![id(1, 2), id(1, 3), id(1, 4)])
    );
    assert!(
        selected
            .to_string()
            .starts_with("(default-strategy-selection (ambiguous")
    );
    assert!(
        strategy(2)
            .to_string()
            .starts_with("(default-strategy (declaration")
    );
}

#[test]
fn exhausted_registration_and_selection_leave_no_partial_publication() {
    let mut registry = Registry::default();
    assert_eq!(
        registry.register(
            strategy(2),
            Budget {
                steps: 0,
                ..Budget::default()
            }
        ),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(
        registry
            .select(id(1, 1), &visible(), &mut 100, |_, _| Ok(A::Applicable))
            .unwrap(),
        Selection::Missing
    );
    registry.register(strategy(2), Budget::default()).unwrap();
    assert_eq!(
        registry.select(id(1, 1), &visible(), &mut 1, |_, _| Ok(A::Applicable)),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(
        registry.select(id(1, 1), &visible(), &mut 100, |_, fuel| {
            *fuel = 0;
            Err(Error::BudgetExhausted)
        }),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(
        registry
            .select(id(1, 1), &visible(), &mut 100, |_, _| Ok(A::Applicable))
            .unwrap(),
        Selection::Unique(id(1, 2))
    );
}

#[test]
fn registration_honors_depth_before_comparing_duplicate_payloads() {
    let mut registry = Registry::default();
    let mut head = strategy(2);
    head.interface_arguments = vec![Term::Pointer(Box::new(Term::Pointer(Box::new(nominal(1)))))];
    let tight = Budget {
        steps: 100,
        max_depth: 1,
    };
    assert_eq!(
        registry.register(head.clone(), tight),
        Err(Error::BudgetExhausted)
    );
    registry.register(head.clone(), Budget::default()).unwrap();
    assert_eq!(registry.register(head, tight), Err(Error::BudgetExhausted));
}

#[test]
fn pending_only_and_rejected_sets_do_not_publish_a_candidate() {
    let registry = registry();
    assert_eq!(
        registry
            .select(id(1, 1), &visible(), &mut 100, |_, _| Ok(A::Pending))
            .unwrap(),
        Selection::Pending {
            applicable: vec![],
            pending: vec![id(1, 2), id(1, 3), id(1, 4)]
        }
    );
    assert_eq!(
        registry
            .select(id(1, 1), &visible(), &mut 100, |_, _| Ok(A::Rejected))
            .unwrap(),
        Selection::Missing
    );
}

#[test]
fn dumps_and_ambiguity_are_stable_across_registration_order() {
    let left = registry();
    let mut right = Registry::default();
    for local in [2, 3, 4] {
        right.register(strategy(local), Budget::default()).unwrap();
    }
    assert_eq!(left.to_string(), right.to_string());
    assert_eq!(
        Selection::Unique(id(1, 2)).to_string(),
        format!("(default-strategy-selection (unique {}))", id(1, 2))
    );
    assert_eq!(
        Selection::Pending {
            applicable: vec![id(1, 2)],
            pending: vec![id(1, 3)]
        }
        .to_string(),
        format!(
            "(default-strategy-selection (applicable {}) (pending {}))",
            id(1, 2),
            id(1, 3)
        )
    );
    for registry in [&left, &right] {
        assert_eq!(
            registry
                .select(id(1, 1), &visible(), &mut 100, |head, _| Ok(
                    if head.declaration.local == 4 {
                        A::Pending
                    } else {
                        A::Applicable
                    }
                ))
                .unwrap(),
            Selection::Ambiguous(vec![id(1, 2), id(1, 3)])
        );
    }
}
