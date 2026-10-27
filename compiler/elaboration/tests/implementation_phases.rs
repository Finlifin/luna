use elaboration::implementation::{Error, ImplementationHead, Index, PhaseSelection, Selection};
use elaboration::ptc::{Budget, Context};
use hir::ImplementationPhase::{Comptime, Runtime};
use nir::{DeclarationId as Id, Primitive, Sort, Term};

fn id(local: u32) -> Id {
    Id { package: 0, local }
}
fn var(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn head(local: u32, binder: Option<u32>) -> ImplementationHead {
    ImplementationHead {
        declaration: id(local),
        interface: Some(id(100)),
        subject: binder.map(var).unwrap_or_else(boolean),
        parameter_types: vec![],
        type_parameters: binder.into_iter().map(id).collect(),
        interface_arguments: vec![],
        requirements: vec![],
    }
}
fn resolve(index: &Index) -> elaboration::implementation::TypeResolution {
    index
        .resolve_types(
            id(100),
            &[],
            &boolean(),
            &Context::default(),
            Budget::default(),
        )
        .unwrap()
}

#[test]
fn complementary_generic_heads_share_selection_and_coherence_in_either_order() {
    for (first, second) in [(Runtime, Comptime), (Comptime, Runtime)] {
        let mut index = Index::default();
        assert_eq!(
            index
                .insert_phase(head(200, Some(1)), first, Budget::default())
                .unwrap(),
            id(200)
        );
        let old = resolve(&index);
        let mut complement = head(201, Some(2));
        complement.parameter_types = vec![Term::Primitive(Primitive::Type)];
        assert_eq!(
            index
                .insert_phase(complement, second, Budget::default())
                .unwrap(),
            id(200)
        );
        assert_eq!(
            index.select_phase(&old, first),
            PhaseSelection::Unselected(Selection::StaleIndex)
        );
        let result = resolve(&index);
        assert_eq!(result.selection, Selection::Selected(id(200)));
        assert_eq!(
            index.select_phase(&result, second),
            PhaseSelection::Selected {
                relationship: id(200),
                contribution: id(201),
                phase: second,
            }
        );
        assert!(
            index
                .check_type_coherence(&Context::default(), Budget::default())
                .is_proved()
        );
        let dump = index.select_phase(&result, second).to_string();
        assert!(
            dump.starts_with("(phase-selection (relationship "),
            "{dump}"
        );
    }
}

#[test]
fn precise_relationship_missing_phase_never_falls_back() {
    let mut index = Index::default();
    index
        .insert_phase(head(200, Some(1)), Comptime, Budget::default())
        .unwrap();
    index
        .insert_phase(head(201, None), Runtime, Budget::default())
        .unwrap();
    let result = resolve(&index);
    assert_eq!(result.selection, Selection::Selected(id(201)));
    assert_eq!(
        index.select_phase(&result, Comptime),
        PhaseSelection::Missing {
            relationship: id(201),
            phase: Comptime,
        }
    );
    assert!(
        index
            .select_phase(&result, Comptime)
            .to_string()
            .starts_with("(missing-phase ")
    );
}

#[test]
fn same_phase_duplicates_remain_coherence_conflicts() {
    let mut index = Index::default();
    for n in [200, 201] {
        index
            .insert_phase(head(n, None), Runtime, Budget::default())
            .unwrap();
    }
    assert_eq!(
        resolve(&index).selection,
        Selection::Conflict(vec![id(200), id(201)])
    );
    assert!(
        !index
            .check_type_coherence(&Context::default(), Budget::default())
            .is_proved()
    );
    let revision = index.revision();
    assert_eq!(
        index.insert_phase(head(202, None), Comptime, Budget::default()),
        Err(Error::AmbiguousPhaseRelationship)
    );
    assert_eq!(index.revision(), revision);
}

#[test]
fn different_applicability_conditions_do_not_merge() {
    let mut index = Index::default();
    index
        .insert_phase(head(200, None), Runtime, Budget::default())
        .unwrap();
    let mut other = head(201, None);
    other
        .requirements
        .push(nir::program::TypePredicate::Bool(false));
    assert_eq!(
        index
            .insert_phase(other, Comptime, Budget::default())
            .unwrap(),
        id(201)
    );
    assert_eq!(
        index.select_phase(&resolve(&index), Comptime),
        PhaseSelection::Missing {
            relationship: id(200),
            phase: Comptime,
        }
    );
}

#[test]
fn requirement_binders_are_alpha_bound_without_capturing_free_identities() {
    let mut index = Index::default();
    let make = |declaration, binder, free| {
        let mut h = head(declaration, Some(binder));
        h.requirements
            .push(nir::program::TypePredicate::Equal(var(binder), var(free)));
        h
    };
    index
        .insert_phase(make(200, 1, 99), Runtime, Budget::default())
        .unwrap();
    assert_eq!(
        index
            .insert_phase(make(201, 2, 99), Comptime, Budget::default())
            .unwrap(),
        id(200)
    );
    let mut separate = Index::default();
    separate
        .insert_phase(make(200, 1, 2), Runtime, Budget::default())
        .unwrap();
    let revision = separate.revision();
    assert!(matches!(
        separate.insert_phase(make(201, 2, 1), Comptime, Budget::default()),
        Err(Error::UndeterminedPhaseRelationship(_))
    ));
    assert_eq!(separate.revision(), revision);
}

#[test]
fn budget_failure_is_atomic_and_allows_retry() {
    let mut succeeded = false;
    for steps in 0..200 {
        let mut index = Index::default();
        index
            .insert_phase(head(200, Some(1)), Runtime, Budget::default())
            .unwrap();
        let before = resolve(&index);
        match index.insert_phase(
            head(201, Some(2)),
            Comptime,
            Budget {
                steps,
                ..Budget::default()
            },
        ) {
            Ok(_) => {
                succeeded = true;
                break;
            }
            Err(Error::BudgetExhausted) => {
                assert!(index.type_resolution_is_current(&before));
                assert!(matches!(
                    index.select_phase(&before, Comptime),
                    PhaseSelection::Missing { .. }
                ));
                index
                    .insert_phase(head(201, Some(2)), Comptime, Budget::default())
                    .unwrap();
                assert!(matches!(
                    index.select_phase(&resolve(&index), Comptime),
                    PhaseSelection::Selected { .. }
                ));
            }
            result => panic!("unexpected admission: {result:?}"),
        }
    }
    assert!(succeeded);
}

#[test]
fn contribution_identity_cannot_be_reused_and_foreign_results_are_rejected() {
    let mut index = Index::default();
    index.insert(head(200, None), Budget::default()).unwrap();
    index
        .insert_phase(head(201, None), Comptime, Budget::default())
        .unwrap();
    let revision = index.revision();
    assert_eq!(
        index.insert(head(201, None), Budget::default()),
        Err(Error::DuplicateDeclaration(id(201)))
    );
    assert_eq!(index.revision(), revision);
    let mut other = Index::default();
    other.insert(head(200, None), Budget::default()).unwrap();
    assert_eq!(
        index.select_phase(&resolve(&other), Runtime),
        PhaseSelection::Unselected(Selection::StaleIndex)
    );
}

#[test]
fn open_interfaces_are_not_paired_before_their_identity_is_known() {
    let mut index = Index::default();
    for (n, phase) in [(200, Runtime), (201, Comptime)] {
        let mut h = head(n, None);
        h.interface = None;
        assert_eq!(
            index.insert_phase(h, phase, Budget::default()).unwrap(),
            id(n)
        );
    }
    assert_eq!(
        index
            .candidates(None, &[], &boolean(), Budget::default())
            .unwrap()
            .declarations()
            .len(),
        2
    );
}

#[test]
fn mixed_sort_domains_and_value_indices_participate_in_relationship_identity() {
    let make = |n, binder, domain| ImplementationHead {
        declaration: id(n),
        interface: Some(id(100)),
        subject: boolean(),
        type_parameters: vec![id(binder)],
        parameter_types: vec![domain],
        interface_arguments: vec![Term::Rigid {
            declaration: id(binder),
            sort: Sort::ComptimeValue,
        }],
        requirements: vec![],
    };
    let mut index = Index::default();
    index
        .insert_phase(make(200, 1, boolean()), Runtime, Budget::default())
        .unwrap();
    assert_eq!(
        index
            .insert_phase(make(201, 2, boolean()), Comptime, Budget::default())
            .unwrap(),
        id(200)
    );
    assert_eq!(
        index
            .insert_phase(
                make(202, 3, Term::Primitive(Primitive::Char)),
                Comptime,
                Budget::default()
            )
            .unwrap(),
        id(202)
    );
    let dump = index.to_string();
    assert!(dump.contains("(phases (runtime "), "{dump}");
    assert!(dump.contains("(comptime "), "{dump}");
}

#[test]
fn opaque_normal_form_heads_keep_their_identity() {
    let mut index = Index::default();
    let mut wrapped = head(200, None);
    wrapped.subject = Term::NormalForm {
        head: id(400),
        sort: Sort::Type,
        arguments: vec![boolean()],
    };
    index
        .insert_phase(wrapped, Runtime, Budget::default())
        .unwrap();
    assert_eq!(
        index
            .insert_phase(head(201, None), Comptime, Budget::default())
            .unwrap(),
        id(201)
    );
}

#[test]
fn unfinished_applicability_is_preserved_instead_of_becoming_a_phase_error() {
    let mut index = Index::default();
    let mut h = head(200, None);
    h.requirements
        .push(nir::program::TypePredicate::Equal(var(99), boolean()));
    index.insert_phase(h, Runtime, Budget::default()).unwrap();
    let resolution = resolve(&index);
    assert!(matches!(resolution.selection, Selection::Deferred { .. }));
    assert_eq!(
        index.select_phase(&resolution, Comptime),
        PhaseSelection::Unselected(resolution.selection)
    );
}

#[test]
fn unbound_requirement_slots_cannot_capture_the_relationship_binders() {
    let mut index = Index::default();
    let mut h = head(200, Some(1));
    h.requirements
        .push(nir::program::TypePredicate::Equal(var(1), boolean()));
    index.insert_phase(h, Runtime, Budget::default()).unwrap();
    let revision = index.revision();
    let mut bad = head(201, Some(2));
    bad.requirements.push(nir::program::TypePredicate::Equal(
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
        boolean(),
    ));
    assert!(matches!(
        index.insert_phase(bad, Comptime, Budget::default()),
        Err(Error::InvalidPhaseHead(_))
    ));
    assert_eq!(index.revision(), revision);
}

#[test]
fn logically_equivalent_conditions_share_a_relationship() {
    use nir::program::TypePredicate as Q;
    let mut index = Index::default();
    let mut runtime = head(200, Some(1));
    runtime.requirements = vec![Q::Equal(var(1), boolean()), Q::Bool(true)];
    index
        .insert_phase(runtime, Runtime, Budget::default())
        .unwrap();
    let mut comptime = head(201, Some(2));
    comptime.requirements = vec![Q::And(vec![
        Q::Bool(true),
        Q::Equal(boolean(), var(2)),
        Q::Equal(var(2), boolean()),
    ])];
    assert_eq!(
        index
            .insert_phase(comptime, Comptime, Budget::default())
            .unwrap(),
        id(200)
    );
    assert!(
        matches!(index.select_phase(&resolve(&index), Comptime), PhaseSelection::Selected { contribution, .. } if contribution == id(201))
    );
}

#[test]
fn implication_in_only_one_direction_does_not_merge_conditions() {
    use nir::program::TypePredicate as Q;
    for reverse in [false, true] {
        let mut index = Index::default();
        let mut weak = head(200, Some(1));
        weak.requirements = vec![Q::Bool(true)];
        let mut strong = head(201, Some(2));
        strong.requirements = vec![Q::Bool(false)];
        let (first, second) = if reverse {
            (strong, weak)
        } else {
            (weak, strong)
        };
        let second_id = second.declaration;
        index
            .insert_phase(first, Runtime, Budget::default())
            .unwrap();
        assert_eq!(
            index
                .insert_phase(second, Comptime, Budget::default())
                .unwrap(),
            second_id
        );
        assert_eq!(resolve(&index).selection, Selection::Selected(id(200)));
    }
}

#[test]
fn equivalence_solver_exhaustion_does_not_publish_a_separate_relationship() {
    use nir::program::TypePredicate as Q;
    let mut succeeded = false;
    for steps in 0..1500 {
        let mut index = Index::default();
        let mut first = head(200, Some(1));
        first.requirements = vec![Q::Equal(var(1), boolean())];
        index
            .insert_phase(first, Runtime, Budget::default())
            .unwrap();
        let revision = index.revision();
        let mut second = head(201, Some(2));
        second.requirements = vec![Q::Equal(boolean(), var(2))];
        match index.insert_phase(
            second.clone(),
            Comptime,
            Budget {
                steps,
                ..Budget::default()
            },
        ) {
            Ok(relationship) => {
                assert_eq!(relationship, id(200));
                succeeded = true;
                break;
            }
            Err(Error::BudgetExhausted) => {
                assert_eq!(index.revision(), revision);
                assert_eq!(
                    index
                        .insert_phase(second, Comptime, Budget::default())
                        .unwrap(),
                    id(200)
                );
            }
            result => panic!("unexpected result: {result:?}"),
        }
    }
    assert!(succeeded);
}

#[test]
fn trait_conditions_allow_boolean_reordering_without_assuming_satisfaction() {
    use nir::program::{TraitRequirement, TypePredicate as Q};
    let requirement = |binder| {
        Q::Trait(TraitRequirement {
            subject: var(binder),
            interface: id(400),
            interface_arguments: vec![],
        })
    };
    let mut index = Index::default();
    let mut first = head(200, Some(1));
    first.requirements = vec![requirement(1)];
    index
        .insert_phase(first, Runtime, Budget::default())
        .unwrap();
    let mut second = head(201, Some(2));
    second.requirements = vec![Q::Not(Box::new(Q::Not(Box::new(requirement(2)))))];
    assert_eq!(
        index
            .insert_phase(second, Comptime, Budget::default())
            .unwrap(),
        id(200)
    );
    assert!(matches!(
        resolve(&index).selection,
        Selection::Deferred { .. }
    ));
}

#[test]
fn unvalidated_countermodels_preserve_the_previous_relationship_world() {
    use nir::program::TypePredicate as Q;
    for reverse in [false, true] {
        let mut index = Index::default();
        let weak = head(200, Some(1));
        let mut strong = head(201, Some(2));
        strong.requirements = vec![Q::Equal(var(2), boolean())];
        let (first, second) = if reverse {
            (strong, weak)
        } else {
            (weak, strong)
        };
        index
            .insert_phase(first, Runtime, Budget::default())
            .unwrap();
        let previous = resolve(&index);
        assert!(matches!(
            index.insert_phase(second, Comptime, Budget::default()),
            Err(Error::UndeterminedPhaseRelationship(_))
        ));
        assert!(index.type_resolution_is_current(&previous));
        assert!(matches!(
            index.select_phase(&previous, Comptime),
            PhaseSelection::Missing { .. }
        ));
    }
}
