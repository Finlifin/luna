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

#[test]
fn ordinary_matching_uses_models_after_structural_proposals_stall() {
    let alias = Term::Rigid {
        declaration: DeclarationId {
            package: 10,
            local: 1,
        },
        sort: Sort::Type,
    };
    let optional = |ty| Term::Optional(Box::new(ty));
    let mut context = Context::default();
    // Only the outer shapes occur as assumption operands. Their injective
    // structure fixes Alias, so the original match is valid for this region.
    context.assume(F::equal(
        optional(alias.clone()),
        optional(optional(Term::Primitive(P::Bool))),
    ));
    // Establish that the direct matcher itself cannot fill this input.
    let mut direct = type_resolution::InferenceContext::default();
    let direct_input = direct.fresh(Sort::Type);
    let direct_pattern = I::Instantiate {
        template: optional(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }),
        arguments: vec![I::Meta(direct_input)],
    };
    type_resolution::matching::propose(
        &mut direct,
        &direct_pattern,
        &I::Stable(alias.clone()),
        &context,
        &[],
        Budget::default(),
    )
    .unwrap();
    assert_eq!(
        direct.resolve(&I::Meta(direct_input)).unwrap(),
        I::Meta(direct_input)
    );
    let mut parent = Engine::with_context(context);
    let input = parent.fresh(Sort::Type);
    let pattern = I::Instantiate {
        template: optional(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }),
        arguments: vec![I::Meta(input)],
    };
    let (mut candidate, obligation) = parent
        .match_types(pattern, I::Stable(alias), Budget::default())
        .unwrap();
    assert_eq!(parent.inspect(&I::Meta(input)).unwrap(), I::Meta(input));
    assert_eq!(candidate.inspect(&I::Meta(input)).unwrap(), stable(P::Bool));
    candidate.advance(100, Budget::default());
    assert!(candidate.state(obligation).unwrap().is_proven());
    parent.commit(candidate).unwrap();
    assert_eq!(
        parent.freeze(&I::Meta(input)).unwrap(),
        Term::Primitive(P::Bool)
    );
}

#[test]
fn model_matching_cannot_turn_a_conditional_shape_into_a_regional_fact() {
    let alias = Term::Rigid {
        declaration: DeclarationId {
            package: 10,
            local: 1,
        },
        sort: Sort::Type,
    };
    let optional = |ty| Term::Optional(Box::new(ty));
    let mut context = Context::default();
    context.assume(F::Or(vec![
        F::equal(alias.clone(), optional(Term::Primitive(P::Bool))),
        F::equal(
            alias.clone(),
            Term::Pointer(Box::new(Term::Primitive(P::U8))),
        ),
    ]));
    let mut parent = Engine::with_context(context);
    let input = parent.fresh(Sort::Type);
    let pattern = I::Instantiate {
        template: optional(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }),
        arguments: vec![I::Meta(input)],
    };
    let (mut candidate, obligation) = parent
        .match_types(pattern, I::Stable(alias), Budget::default())
        .unwrap();
    candidate.advance(100, Budget::default());
    assert!(!candidate.state(obligation).unwrap().is_proven());
    assert!(parent.commit(candidate).is_err());
    assert_eq!(parent.inspect(&I::Meta(input)).unwrap(), I::Meta(input));
}
