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
fn symbolic_model_proposals_preserve_the_ambient_type_and_transaction_boundary() {
    let mut parent = Engine::default();
    let input = parent.fresh(Sort::Type);
    let ambient = Term::Rigid {
        declaration: DeclarationId {
            package: 921,
            local: 7,
        },
        sort: Sort::Type,
    };
    let problem = R::Equal(
        I::Meta(input),
        I::Stable(Term::Pointer(Box::new(ambient.clone()))),
    );
    let Search::Proposed {
        mut candidate,
        obligation,
        evidence,
    } = parent
        .search_type_inputs(&[input], &problem, &mut Budget::default())
        .unwrap()
    else {
        panic!("the input is determined relative to the ambient declaration");
    };
    assert_eq!(parent.inspect(&I::Meta(input)).unwrap(), I::Meta(input));
    let expected = Term::Pointer(Box::new(ambient));
    assert_eq!(evidence.bindings.values().next().unwrap().0, expected);
    candidate.advance(100, Budget::default());
    assert!(candidate.state(obligation).unwrap().is_proven());
    parent.commit(candidate).unwrap();
    assert_eq!(parent.freeze(&I::Meta(input)).unwrap(), expected);
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
fn ordinary_matching_recovers_nested_regional_structure() {
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
    // Regional subterm discovery now lets direct matching retain this shape.
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
        stable(P::Bool)
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

#[test]
fn dependent_symbolic_proposals_commit_without_temporary_search_identities() {
    let mut parent = Engine::default();
    let outer = parent.fresh(Sort::Type);
    let inner = parent.fresh(Sort::Type);
    let ambient = Term::Rigid {
        declaration: DeclarationId {
            package: 921,
            local: 7,
        },
        sort: Sort::Type,
    };
    let problem = R::And(vec![
        R::Equal(
            I::Meta(outer),
            I::Instantiate {
                template: Term::Pointer(Box::new(Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::Type,
                })),
                arguments: vec![I::Meta(inner)],
            },
        ),
        R::Equal(I::Meta(inner), I::Stable(ambient.clone())),
    ]);
    let Search::Proposed {
        mut candidate,
        obligation,
        evidence,
    } = parent
        .search_type_inputs(&[outer, inner], &problem, &mut Budget::default())
        .unwrap()
    else {
        panic!("dependent symbolic input should have a proposal")
    };
    assert_eq!(evidence.bindings.len(), 2);
    assert_eq!(parent.inspect(&I::Meta(outer)).unwrap(), I::Meta(outer));
    assert_eq!(parent.inspect(&I::Meta(inner)).unwrap(), I::Meta(inner));
    candidate.advance(100, Budget::default());
    assert!(candidate.state(obligation).unwrap().is_proven());
    parent.commit(candidate).unwrap();
    assert_eq!(
        parent.freeze(&I::Meta(outer)).unwrap(),
        Term::Pointer(Box::new(ambient.clone()))
    );
    assert_eq!(parent.freeze(&I::Meta(inner)).unwrap(), ambient);
}

#[test]
fn model_search_late_budget_failures_preserve_assignments_and_waiters() {
    let mut parent = Engine::default();
    let outer = parent.fresh(Sort::Type);
    let inner = parent.fresh(Sort::Type);
    let waiting = parent.require(R::Equal(I::Meta(inner), stable(P::Bool)));
    parent.advance(100, Budget::default());
    assert!(matches!(parent.state(waiting), Some(State::Waiting(_))));
    let problem = R::And(vec![
        determined(I::Meta(inner)),
        R::Equal(
            I::Meta(outer),
            I::Instantiate {
                template: Term::Optional(Box::new(Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::Type,
                })),
                arguments: vec![I::Meta(inner)],
            },
        ),
    ]);
    let inputs = [outer, inner];
    let mut measured = Budget::default();
    let before = measured.steps;
    assert!(matches!(
        parent
            .search_type_inputs(&inputs, &problem, &mut measured)
            .unwrap(),
        Search::Proposed { .. }
    ));
    let used = before - measured.steps;
    let mut interrupted = 0;
    let mut completed = 0;
    // Search strategies divide available fuel, so a smaller allowance need not
    // reproduce the exact work count of the full-budget run. Accept only a
    // complete checked proposal or an isolated budget failure at every point.
    for steps in 0..=used {
        let mut budget = Budget {
            steps,
            ..Budget::default()
        };
        match parent.search_type_inputs(&inputs, &problem, &mut budget) {
            Err(Error::Inference(InferenceError::Budget))
            | Ok(Search::Unresolved(TypeInputOutcome::Unknown(
                type_resolution::ptc::UnknownReason::BudgetExhausted,
            ))) => interrupted += 1,
            Ok(Search::Proposed {
                mut candidate,
                obligation,
                ..
            }) => {
                completed += 1;
                assert_eq!(candidate.inspect(&I::Meta(inner)).unwrap(), stable(P::Bool));
                assert_eq!(
                    candidate.inspect(&I::Meta(outer)).unwrap(),
                    I::Stable(Term::Optional(Box::new(Term::Primitive(P::Bool))))
                );
                assert!(candidate.freeze(&I::Meta(outer)).is_err());
                candidate.advance(100, Budget::default());
                assert!(candidate.state(waiting).unwrap().is_proven());
                assert!(candidate.state(obligation).unwrap().is_proven());
                assert_eq!(
                    candidate.freeze(&I::Meta(inner)).unwrap(),
                    Term::Primitive(P::Bool)
                );
            }
            other => panic!(
                "search must not publish a partially staged proposal at {steps}/{used}: {}",
                match other {
                    Ok(result) => result.to_string(),
                    Err(error) => format!("{error:?}"),
                }
            ),
        }
        assert!(budget.steps <= steps);
        assert!(matches!(parent.state(waiting), Some(State::Waiting(_))));
        for input in inputs {
            assert_eq!(parent.inspect(&I::Meta(input)).unwrap(), I::Meta(input));
        }
    }
    assert!(interrupted > 0);
    assert!(completed > 0);
    let Search::Proposed {
        mut candidate,
        obligation,
        ..
    } = parent
        .search_type_inputs(&inputs, &problem, &mut Budget::default())
        .unwrap()
    else {
        panic!("a fresh allowance must recover");
    };
    assert!(candidate.freeze(&I::Meta(outer)).is_err());
    candidate.advance(100, Budget::default());
    assert!(candidate.state(waiting).unwrap().is_proven());
    assert!(candidate.state(obligation).unwrap().is_proven());
    parent.commit(candidate).unwrap();
    assert_eq!(
        parent.freeze(&I::Meta(inner)).unwrap(),
        Term::Primitive(P::Bool)
    );
    assert_eq!(
        parent.freeze(&I::Meta(outer)).unwrap(),
        Term::Optional(Box::new(Term::Primitive(P::Bool)))
    );
}

#[test]
fn fixed_environment_ambiguity_does_not_assign_inference_inputs() {
    let mut parent = Engine::default();
    let input = parent.fresh(Sort::Type);
    let ambient = DeclarationId {
        package: 927,
        local: 1,
    };
    let ambient_type = I::Stable(Term::Rigid {
        declaration: ambient,
        sort: Sort::Type,
    });
    let problem = R::Not(Box::new(R::Equal(I::Meta(input), ambient_type)));
    let Search::Unresolved(TypeInputOutcome::Ambiguous {
        candidate,
        alternative,
        ..
    }) = parent
        .search_type_inputs(&[input], &problem, &mut Budget::default())
        .unwrap()
    else {
        panic!("inequality cannot determine the requested input");
    };
    assert_eq!(candidate.types[&ambient], alternative.types[&ambient]);
    assert_eq!(parent.inspect(&I::Meta(input)).unwrap(), I::Meta(input));
    let Search::Proposed { mut candidate, .. } = parent
        .search_type_inputs(
            &[input],
            &determined(I::Meta(input)),
            &mut Budget::default(),
        )
        .unwrap()
    else {
        panic!("an independent determined problem must remain solvable");
    };
    candidate.advance(100, Budget::default());
    parent.commit(candidate).unwrap();
    assert_eq!(
        parent.freeze(&I::Meta(input)).unwrap(),
        Term::Primitive(P::Bool)
    );
}
