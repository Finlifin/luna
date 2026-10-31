use nir::{Primitive, Sort, Term};
use type_resolution::*;
fn boolean() -> InferenceTerm {
    InferenceTerm::Stable(Term::Primitive(Primitive::Bool))
}
fn frozen_types(fields: Vec<nir::Term>) -> nir::Term {
    nir::Term::NominalValue {
        ty: Box::new(nir::Term::NormalForm {
            head: nir::DeclarationId {
                package: 901,
                local: 1,
            },
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Type)],
        }),
        variant: None,
        fields,
    }
}

#[test]
fn abandoned_trial_leaves_parent_unassigned() {
    let mut parent = InferenceContext::default();
    let id = parent.fresh(Sort::Type);
    {
        let mut trial = parent.trial();
        trial.assign(id, boolean()).unwrap();
        assert!(trial.freeze(&InferenceTerm::Meta(id)).is_ok());
    }
    assert_eq!(
        parent.freeze(&InferenceTerm::Meta(id)),
        Err(InferenceError::Unresolved(id))
    );
}
#[test]
fn commit_publishes_assignments_and_invalidates_siblings() {
    let mut parent = InferenceContext::default();
    let id = parent.fresh(Sort::Type);
    let mut first = parent.trial();
    let second = parent.trial();
    first.assign(id, boolean()).unwrap();
    parent.commit(first).unwrap();
    assert_eq!(
        parent.freeze(&InferenceTerm::Meta(id)).unwrap(),
        Term::Primitive(Primitive::Bool)
    );
    assert_eq!(parent.commit(second), Err(InferenceError::StaleCandidate));
}
#[test]
fn changed_parent_and_unrelated_parent_reject_trials() {
    let mut parent = InferenceContext::default();
    let trial = parent.trial();
    parent.fresh(Sort::Type);
    assert_eq!(parent.commit(trial), Err(InferenceError::StaleCandidate));
    let other = InferenceContext::default();
    assert_eq!(
        parent.commit(other.trial()),
        Err(InferenceError::StaleCandidate)
    );
}
#[test]
fn child_ids_are_not_reused_or_visible_before_commit() {
    let mut parent = InferenceContext::default();
    let mut child = parent.trial();
    let id = child.fresh(Sort::Type);
    child.assign(id, boolean()).unwrap();
    assert_eq!(
        parent.freeze(&InferenceTerm::Meta(id)),
        Err(InferenceError::ForeignVariable(id))
    );
    parent.commit(child).unwrap();
    assert!(parent.freeze(&InferenceTerm::Meta(id)).is_ok());
}
#[test]
fn direct_and_indirect_occurs_checks_leave_store_unchanged() {
    let mut context = InferenceContext::default();
    let a = context.fresh(Sort::Type);
    let b = context.fresh(Sort::Type);
    assert_eq!(
        context.assign(a, InferenceTerm::Meta(a)),
        Err(InferenceError::Occurs(a))
    );
    context.assign(a, InferenceTerm::Meta(b)).unwrap();
    assert_eq!(
        context.assign(b, InferenceTerm::Meta(a)),
        Err(InferenceError::Occurs(b))
    );
    context.assign(b, boolean()).unwrap();
    assert_eq!(
        context.freeze(&InferenceTerm::Meta(a)).unwrap(),
        Term::Primitive(Primitive::Bool)
    );
}
#[test]
fn unresolved_variables_never_escape_freezing() {
    let mut context = InferenceContext::default();
    let id = context.fresh(Sort::Type);
    let term = InferenceTerm::Instantiate {
        template: Term::Pointer(Box::new(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        })),
        arguments: vec![InferenceTerm::Meta(id)],
    };
    assert_eq!(context.freeze(&term), Err(InferenceError::Unresolved(id)));
    context.assign(id, boolean()).unwrap();
    assert_eq!(
        context.freeze(&term).unwrap(),
        Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)))
    );
    assert!(term.to_string().starts_with("(instantiate (template "));
}
#[test]
fn incompatible_sorts_and_reassignments_are_rejected() {
    let mut context = InferenceContext::default();
    let id = context.fresh(Sort::Runtime);
    assert!(matches!(
        context.assign(id, boolean()),
        Err(InferenceError::WrongSort { .. })
    ));
    let value = InferenceTerm::Stable(Term::Runtime(nir::RuntimeId {
        instance: 1,
        local: 0,
    }));
    context.assign(id, value.clone()).unwrap();
    assert_eq!(
        context.assign(id, value),
        Err(InferenceError::AlreadyAssigned(id))
    );
}

#[test]
fn instantiation_waits_only_for_slots_referenced_by_its_template() {
    let mut context = InferenceContext::default();
    let used = context.fresh(Sort::Type);
    let unused = context.fresh(Sort::Type);
    let term = InferenceTerm::Instantiate {
        template: Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
        arguments: vec![InferenceTerm::Meta(used), InferenceTerm::Meta(unused)],
    };
    assert_eq!(
        context.unresolved(&term).unwrap(),
        std::collections::BTreeSet::from([used])
    );
    context.assign(used, boolean()).unwrap();
    assert!(context.unresolved(&term).unwrap().is_empty());
    assert_eq!(
        context.freeze(&term).unwrap(),
        Term::Primitive(Primitive::Bool)
    );
    assert_eq!(
        context.freeze(&InferenceTerm::Meta(unused)),
        Err(InferenceError::Unresolved(unused))
    );
}

#[test]
fn unused_substitution_slots_do_not_create_recursive_types() {
    let mut context = InferenceContext::default();
    let a = context.fresh(Sort::Type);
    let b = context.fresh(Sort::Type);
    context
        .assign(
            a,
            InferenceTerm::Instantiate {
                template: Term::Pointer(Box::new(Term::Bound {
                    depth: 0,
                    index: 1,
                    sort: Sort::Type,
                })),
                arguments: vec![InferenceTerm::Meta(a), InferenceTerm::Meta(b)],
            },
        )
        .unwrap();
    assert_eq!(
        context.unresolved(&InferenceTerm::Meta(a)).unwrap(),
        std::collections::BTreeSet::from([b])
    );
    assert_eq!(
        context.freeze(&InferenceTerm::Meta(a)),
        Err(InferenceError::Unresolved(b))
    );
    // Reversing the actual edge would still create a real recursive type.
    assert_eq!(
        context.assign(b, InferenceTerm::Meta(a)),
        Err(InferenceError::Occurs(b))
    );
    context.assign(b, boolean()).unwrap();
    assert_eq!(
        context.freeze(&InferenceTerm::Meta(a)).unwrap(),
        Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)))
    );
}

#[test]
fn long_inference_alias_chains_are_checked_and_frozen_without_recursion() {
    let mut context = InferenceContext::default();
    let ids: Vec<_> = (0..8192).map(|_| context.fresh(Sort::Type)).collect();
    for pair in ids.windows(2) {
        context
            .assign(pair[0], InferenceTerm::Meta(pair[1]))
            .unwrap();
    }
    let last = *ids.last().unwrap();
    assert_eq!(
        context.assign(last, InferenceTerm::Meta(ids[0])),
        Err(InferenceError::Occurs(last))
    );
    context.assign(last, boolean()).unwrap();
    assert!(
        context
            .unresolved(&InferenceTerm::Meta(ids[0]))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        context.freeze(&InferenceTerm::Meta(ids[0])).unwrap(),
        Term::Primitive(Primitive::Bool)
    );
    assert_eq!(
        context.resolve(&InferenceTerm::Meta(ids[0])).unwrap(),
        boolean()
    );
}

#[test]
fn referenced_slot_sorts_and_missing_inputs_are_checked_before_assignment() {
    let mut context = InferenceContext::default();
    let target = context.fresh(Sort::Type);
    let runtime = context.fresh(Sort::Runtime);
    let template = Term::Pointer(Box::new(Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    }));
    assert_eq!(
        context.assign(
            target,
            InferenceTerm::Instantiate {
                template: template.clone(),
                arguments: vec![InferenceTerm::Meta(runtime)]
            }
        ),
        Err(InferenceError::WrongSort {
            expected: Sort::Type,
            actual: Sort::Runtime
        })
    );
    assert_eq!(
        context.assign(
            target,
            InferenceTerm::Instantiate {
                template,
                arguments: vec![]
            }
        ),
        Err(InferenceError::InvalidSubstitution(
            nir::TypeError::MissingArgument(0)
        ))
    );
    context.assign(target, boolean()).unwrap();
    assert_eq!(
        context.freeze(&InferenceTerm::Meta(target)).unwrap(),
        Term::Primitive(Primitive::Bool)
    );
}

#[test]
fn iterative_freezing_preserves_sparse_slot_order_and_shared_subgraphs() {
    let mut context = InferenceContext::default();
    let a = context.fresh(Sort::Type);
    context.assign(a, boolean()).unwrap();
    let slot = |index| Term::Bound {
        depth: 0,
        index,
        sort: Sort::Type,
    };
    let term = InferenceTerm::Instantiate {
        template: Term::Tuple(vec![slot(2), Term::Pointer(Box::new(slot(0))), slot(2)]),
        arguments: vec![
            InferenceTerm::Meta(a),
            InferenceTerm::Meta(context.fresh(Sort::Type)),
            InferenceTerm::Stable(Term::Primitive(Primitive::Char)),
        ],
    };
    assert_eq!(
        context.freeze(&term).unwrap(),
        Term::Tuple(vec![
            Term::Primitive(Primitive::Char),
            Term::Pointer(Box::new(Term::Primitive(Primitive::Bool))),
            Term::Primitive(Primitive::Char)
        ])
    );
}

#[test]
fn metered_freezing_bounds_expansion_and_can_retry_without_assignments() {
    use nir::DeclarationId;
    let context = InferenceContext::default();
    let bound = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let head = DeclarationId {
        package: 99,
        local: 1,
    };
    let input = Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![frozen_types(vec![Term::Primitive(Primitive::Bool); 256])],
    };
    let expression = InferenceTerm::Instantiate {
        template: Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![frozen_types(vec![bound; 64])],
        },
        arguments: vec![InferenceTerm::Stable(input.clone())],
    };
    assert_eq!(
        context.freeze_metered(&expression, &mut 1000),
        Err(InferenceError::Budget)
    );
    let frozen = context.freeze_metered(&expression, &mut 100_000).unwrap();
    let Term::NormalForm {
        head: actual,
        arguments,
        ..
    } = frozen
    else {
        panic!("opaque head")
    };
    assert_eq!(actual, head);
    assert_eq!(arguments, vec![frozen_types(vec![input; 64])]);
}

#[test]
fn metered_inference_reads_only_referenced_slots_and_bounds_aliases() {
    let mut context = InferenceContext::default();
    let unused = context.fresh(Sort::Type);
    let mut last = context.fresh(Sort::Type);
    context.assign(last, boolean()).unwrap();
    for _ in 0..128 {
        let next = context.fresh(Sort::Type);
        context.assign(next, InferenceTerm::Meta(last)).unwrap();
        last = next;
    }
    let expression = InferenceTerm::Instantiate {
        template: Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
        arguments: vec![InferenceTerm::Meta(last), InferenceTerm::Meta(unused)],
    };
    assert_eq!(
        context.unresolved_metered(&expression, &mut 10),
        Err(InferenceError::Budget)
    );
    assert_eq!(
        context.freeze_metered(&expression, &mut 10),
        Err(InferenceError::Budget)
    );
    assert!(
        context
            .unresolved_metered(&expression, &mut 1000)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        context.freeze_metered(&expression, &mut 1000).unwrap(),
        Term::Primitive(Primitive::Bool)
    );
    assert_eq!(
        context.freeze(&InferenceTerm::Meta(unused)),
        Err(InferenceError::Unresolved(unused))
    );
}

#[test]
fn explicit_depth_limits_follow_aliases_and_ignore_unused_arguments() {
    let mut context = InferenceContext::default();
    let input = context.fresh(Sort::Type);
    let nested = Term::Pointer(Box::new(Term::Pointer(Box::new(Term::Primitive(
        Primitive::Bool,
    )))));
    context
        .assign(input, InferenceTerm::Stable(nested))
        .unwrap();
    assert_eq!(
        context.unresolved_with_limits(&InferenceTerm::Meta(input), &mut 1000, 2),
        Err(InferenceError::Budget)
    );
    assert_eq!(
        context.freeze_with_limits(&InferenceTerm::Meta(input), &mut 1000, 2),
        Err(InferenceError::Budget)
    );
    assert!(
        context
            .freeze_with_limits(&InferenceTerm::Meta(input), &mut 1000, 3)
            .is_ok()
    );
    let unused = InferenceTerm::Instantiate {
        template: Term::Primitive(Primitive::Bool),
        arguments: vec![InferenceTerm::Meta(input)],
    };
    assert!(
        context
            .unresolved_with_limits(&unused, &mut 1000, 1)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        context.freeze_with_limits(&unused, &mut 1000, 1).unwrap(),
        Term::Primitive(Primitive::Bool)
    );
}
