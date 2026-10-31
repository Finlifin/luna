use nir::{DeclarationId, Primitive as P, Sort, Term};
use type_resolution::{
    InferenceError, InferenceTerm as I,
    obligations::{Engine, Error, Requirement as R, State, TypeInputSearch as Search},
    ptc::{Budget, Context, Formula as F, TypeInputOutcome},
};
fn stable(p: P) -> I {
    I::Stable(Term::Primitive(p))
}
fn determined(input: I) -> R {
    R::And(vec![
        R::Or(vec![
            R::Equal(input.clone(), stable(P::Bool)),
            R::Equal(input.clone(), stable(P::U8)),
        ]),
        R::Not(Box::new(R::Equal(input, stable(P::U8)))),
    ])
}
#[test]
fn unique_models_propose_isolated_candidates_with_original_obligations() {
    let mut parent = Engine::default();
    let input = parent.fresh(Sort::Type);
    let problem = determined(I::Meta(input));
    let Search::Proposed {
        mut candidate,
        obligation,
        evidence,
    } = parent
        .search_type_inputs(&[input], &problem, &mut Budget::default())
        .unwrap()
    else {
        panic!("determined input")
    };
    assert_eq!(evidence.bindings.len(), 1);
    assert_eq!(parent.inspect(&I::Meta(input)).unwrap(), I::Meta(input));
    assert!(matches!(candidate.state(obligation), Some(State::Ready)));
    assert!(candidate.freeze(&I::Meta(input)).is_err());
    candidate.advance(100, Budget::default());
    assert!(candidate.state(obligation).unwrap().is_proven());
    parent.commit(candidate).unwrap();
    assert_eq!(
        parent.freeze(&I::Meta(input)).unwrap(),
        Term::Primitive(P::Bool)
    );
}
#[test]
fn existing_requires_neither_supply_search_facts_nor_disappear_on_commit() {
    let mut parent = Engine::default();
    let input = parent.fresh(Sort::Type);
    let required = parent.require(R::Equal(I::Meta(input), stable(P::U8)));
    parent.advance(100, Budget::default());
    assert!(matches!(
        parent
            .search_type_inputs(&[input], &R::Stable(F::True), &mut Budget::default())
            .unwrap(),
        Search::Unresolved(TypeInputOutcome::Ambiguous { .. })
    ));
    let Search::Proposed { mut candidate, .. } = parent
        .search_type_inputs(
            &[input],
            &determined(I::Meta(input)),
            &mut Budget::default(),
        )
        .unwrap()
    else {
        panic!("determined input")
    };
    candidate.advance(100, Budget::default());
    assert!(!candidate.state(required).unwrap().is_proven());
    assert!(matches!(parent.commit(candidate), Err(Error::Unsettled(_))));
    assert_eq!(parent.inspect(&I::Meta(input)).unwrap(), I::Meta(input));
}
#[test]
fn failure_does_not_assign_inputs_and_excluded_variables_are_not_inferred() {
    let mut parent = Engine::default();
    let input = parent.fresh(Sort::Type);
    let excluded = parent.fresh(Sort::Type);
    let problem = R::And(vec![
        determined(I::Meta(input)),
        R::Equal(I::Meta(excluded), stable(P::U8)),
    ]);
    assert!(
        matches!(parent.search_type_inputs(&[input], &problem, &mut Budget::default()),
        Err(Error::Inference(InferenceError::Unresolved(id))) if id == excluded)
    );
    assert_eq!(parent.inspect(&I::Meta(input)).unwrap(), I::Meta(input));
    let mut budget = Budget {
        steps: 1,
        ..Budget::default()
    };
    assert!(
        parent
            .search_type_inputs(&[input], &determined(I::Meta(input)), &mut budget)
            .is_err()
    );
    assert_eq!(parent.inspect(&I::Meta(input)).unwrap(), I::Meta(input));
    let Search::Proposed { mut candidate, .. } = parent
        .search_type_inputs(&[input, excluded], &problem, &mut Budget::default())
        .unwrap()
    else {
        panic!("explicit inputs")
    };
    candidate.advance(100, Budget::default());
    parent.commit(candidate).unwrap();
    assert_eq!(
        parent.freeze(&I::Meta(excluded)).unwrap(),
        Term::Primitive(P::U8)
    );
}
#[test]
fn temporary_input_identities_do_not_capture_existing_region_parameters() {
    let existing = Term::Rigid {
        declaration: DeclarationId {
            package: u64::MAX,
            local: 0,
        },
        sort: Sort::Type,
    };
    let mut context = Context::default();
    context.assume(F::equal(existing.clone(), Term::Primitive(P::U8)));
    let mut parent = Engine::with_context(context);
    let input = parent.fresh(Sort::Type);
    let Search::Proposed {
        mut candidate,
        evidence,
        ..
    } = parent
        .search_type_inputs(
            &[input],
            &determined(I::Meta(input)),
            &mut Budget::default(),
        )
        .unwrap()
    else {
        panic!("fresh input identity")
    };
    assert!(!evidence.bindings.contains_key(&DeclarationId {
        package: u64::MAX,
        local: 0
    }));
    candidate.advance(100, Budget::default());
    parent.commit(candidate).unwrap();
    assert_eq!(
        parent.freeze(&I::Meta(input)).unwrap(),
        Term::Primitive(P::Bool)
    );
}
#[test]
fn wrong_sort_assigned_and_stale_candidates_follow_existing_transaction_rules() {
    let mut parent = Engine::default();
    let input = parent.fresh(Sort::Type);
    let other = parent.fresh(Sort::ComptimeValue);
    assert!(matches!(
        parent.search_type_inputs(&[other], &R::Stable(F::True), &mut Budget::default()),
        Err(Error::Inference(InferenceError::WrongSort { .. }))
    ));
    let Search::Proposed { mut candidate, .. } = parent
        .search_type_inputs(
            &[input],
            &determined(I::Meta(input)),
            &mut Budget::default(),
        )
        .unwrap()
    else {
        panic!("proposal")
    };
    candidate.advance(100, Budget::default());
    parent.assign(input, stable(P::U8)).unwrap();
    assert!(matches!(
        parent.commit(candidate),
        Err(Error::StaleCandidate)
    ));
    assert!(matches!(
        parent.search_type_inputs(&[input], &R::Stable(F::True), &mut Budget::default()),
        Err(Error::Inference(InferenceError::AlreadyAssigned(_)))
    ));
}
