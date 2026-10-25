use elaboration::{
    InferenceTerm as T,
    obligations::{Engine, Error, Requirement as R, State},
    ptc::{Budget, Formula as F, Outcome, TypeKind as K},
};
use nir::{Primitive as P, Sort, Term};
fn stable(p: P) -> T {
    T::Stable(Term::Primitive(p))
}
fn run(engine: &mut Engine) {
    engine.advance(100, Budget::default());
}

#[test]
fn waiters_follow_aliases_and_only_affected_obligations_wake() {
    let mut engine = Engine::default();
    let a = engine.fresh(Sort::Type);
    let b = engine.fresh(Sort::Type);
    let c = engine.fresh(Sort::Type);
    let first = engine.require(R::Equal(T::Meta(a), stable(P::Bool)));
    let second = engine.require(R::Kind(T::Meta(c), K::UnsignedInteger));
    run(&mut engine);
    assert!(matches!(engine.state(first), Some(State::Waiting(ids)) if ids.contains(&a)));
    assert_eq!(engine.advance(100, Budget::default()), 0);
    engine.assign(a, T::Meta(b)).unwrap();
    run(&mut engine);
    assert!(
        matches!(engine.state(first), Some(State::Waiting(ids)) if ids.contains(&b) && !ids.contains(&a))
    );
    assert_eq!(engine.attempts(second), Some(1));
    engine.assign(b, stable(P::Bool)).unwrap();
    run(&mut engine);
    assert!(engine.state(first).unwrap().is_proven());
    assert_eq!(engine.attempts(second), Some(1));
    engine.assign(c, stable(P::U32)).unwrap();
    run(&mut engine);
    assert!(engine.state(second).unwrap().is_proven());
    assert_eq!(engine.freeze(&T::Meta(a)), Ok(Term::Primitive(P::Bool)));
}

#[test]
fn one_requirement_tracks_all_dependencies_and_freezes_substitution() {
    let mut engine = Engine::default();
    let a = engine.fresh(Sort::Type);
    let b = engine.fresh(Sort::Type);
    let tuple = T::Instantiate {
        template: Term::Tuple(vec![
            Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type,
            },
            Term::Bound {
                depth: 0,
                index: 1,
                sort: Sort::Type,
            },
        ]),
        arguments: vec![T::Meta(a), T::Meta(b)],
    };
    let obligation = engine.require(R::Kind(tuple.clone(), K::Tuple));
    run(&mut engine);
    assert!(matches!(engine.state(obligation), Some(State::Waiting(ids)) if ids.len() == 2));
    engine.assign(a, stable(P::U32)).unwrap();
    run(&mut engine);
    assert!(
        matches!(engine.state(obligation), Some(State::Waiting(ids)) if ids.len() == 1 && ids.contains(&b))
    );
    engine.assign(b, stable(P::Bool)).unwrap();
    run(&mut engine);
    assert_eq!(
        engine.freeze(&tuple),
        Ok(Term::Tuple(vec![
            Term::Primitive(P::U32),
            Term::Primitive(P::Bool)
        ]))
    );
}

#[test]
fn candidate_failure_does_not_publish_assignment_or_evidence() {
    let mut parent = Engine::default();
    let a = parent.fresh(Sort::Type);
    let obligation = parent.require(R::Kind(T::Meta(a), K::UnsignedInteger));
    run(&mut parent);
    let mut bad = parent.trial();
    bad.assign(a, stable(P::Bool)).unwrap();
    run(&mut bad);
    assert!(matches!(
        bad.state(obligation),
        Some(State::Solved(Outcome::NotEntailed(_)))
    ));
    assert_eq!(parent.commit(bad), Err(Error::Unsettled(obligation)));
    assert!(matches!(parent.state(obligation), Some(State::Waiting(_))));
    let mut good = parent.trial();
    good.assign(a, stable(P::U32)).unwrap();
    run(&mut good);
    parent.commit(good).unwrap();
    assert_eq!(parent.freeze(&T::Meta(a)), Ok(Term::Primitive(P::U32)));
    assert!(parent.state(obligation).unwrap().is_proven());
}

#[test]
fn candidate_facts_are_private_until_accepted_and_revision_checks_include_facts() {
    let mut parent = Engine::default();
    let ty = Term::Rigid {
        declaration: nir::DeclarationId {
            package: 1,
            local: 0,
        },
        sort: Sort::Type,
    };
    let fact = F::kind(ty, K::UnsignedInteger);
    let obligation = parent.require(R::Stable(fact.clone()));
    run(&mut parent);
    let mut candidate = parent.trial();
    candidate.assume(fact.clone());
    run(&mut candidate);
    assert!(candidate.state(obligation).unwrap().is_proven());
    assert!(!parent.state(obligation).unwrap().is_proven());
    parent.assume(F::True);
    assert_eq!(parent.commit(candidate), Err(Error::StaleCandidate));
    run(&mut parent);
    assert!(!parent.state(obligation).unwrap().is_proven());
    let mut candidate = parent.trial();
    candidate.assume(fact);
    run(&mut candidate);
    parent.commit(candidate).unwrap();
    assert!(parent.state(obligation).unwrap().is_proven());
}

#[test]
fn stable_unknown_requires_explicit_retry_and_cannot_escape_freezing() {
    let mut engine = Engine::default();
    let obligation = engine.require(R::Equal(stable(P::Bool), stable(P::Bool)));
    engine.advance(
        1,
        Budget {
            steps: 0,
            max_depth: 128,
        },
    );
    assert!(matches!(
        engine.state(obligation),
        Some(State::Solved(Outcome::Unknown { .. }))
    ));
    assert_eq!(
        engine.freeze(&stable(P::Bool)),
        Err(Error::Unsettled(obligation))
    );
    assert_eq!(engine.advance(100, Budget::default()), 0);
    engine.retry_unknown();
    run(&mut engine);
    assert_eq!(
        engine.freeze(&stable(P::Bool)),
        Ok(Term::Primitive(P::Bool))
    );
}

#[test]
fn queue_limits_and_new_facts_invalidate_old_scope_evidence() {
    let mut engine = Engine::default();
    let first = engine.require(R::Stable(F::True));
    let second = engine.require(R::Stable(F::True));
    assert_eq!(engine.advance(1, Budget::default()), 1);
    assert!(engine.state(first).unwrap().is_proven());
    assert!(matches!(engine.state(second), Some(State::Ready)));
    engine.assume(F::True);
    assert!(matches!(engine.state(first), Some(State::Ready)));
    run(&mut engine);
    let Some(State::Solved(Outcome::Entailed(proof))) = engine.state(first) else {
        panic!("proof")
    };
    assert_eq!(proof.assumptions, vec![F::True]);
}

#[test]
fn foreign_variables_and_invalid_substitutions_are_failures_not_waiters() {
    let mut other = Engine::default();
    let foreign = other.fresh(Sort::Type);
    let mut engine = Engine::default();
    let id = engine.require(R::Kind(T::Meta(foreign), K::Tuple));
    run(&mut engine);
    assert!(matches!(engine.state(id), Some(State::InvalidInference(_))));
    let mut engine = Engine::default();
    let bad = T::Instantiate {
        template: Term::Bound {
            depth: 0,
            index: 5,
            sort: Sort::Type,
        },
        arguments: vec![],
    };
    let id = engine.require(R::Kind(bad, K::Tuple));
    run(&mut engine);
    assert!(matches!(engine.state(id), Some(State::InvalidInference(_))));
    assert_eq!(engine.freeze(&stable(P::Bool)), Err(Error::Unsettled(id)));
}

#[test]
fn nested_candidates_commit_to_the_correct_parent_and_dump_states() {
    let mut parent = Engine::default();
    let a = parent.fresh(Sort::Type);
    parent.require(R::Equal(T::Meta(a), stable(P::Bool)));
    run(&mut parent);
    let dump = parent.to_string();
    assert!(dump.starts_with("(obligations (obligation"));
    assert!(dump.contains("(waiting (meta"));
    assert_eq!(dump, parent.to_string());
    let mut candidate = parent.trial();
    let mut nested = candidate.trial();
    nested.assign(a, stable(P::Bool)).unwrap();
    run(&mut nested);
    candidate.commit(nested).unwrap();
    parent.commit(candidate).unwrap();
    assert_eq!(parent.freeze(&T::Meta(a)), Ok(Term::Primitive(P::Bool)));
    assert!(parent.to_string().contains("(entailed"));
}

#[test]
fn unused_substitution_slots_do_not_keep_a_proven_obligation_waiting() {
    let mut engine = Engine::default();
    let used = engine.fresh(Sort::Type);
    let unused = engine.fresh(Sort::Type);
    let term = T::Instantiate {
        template: Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
        arguments: vec![T::Meta(used), T::Meta(unused)],
    };
    let requirement = engine.require(R::Equal(term.clone(), stable(P::Bool)));
    run(&mut engine);
    assert!(
        matches!(engine.state(requirement), Some(State::Waiting(ids)) if ids.len() == 1 && ids.contains(&used))
    );
    engine.assign(used, stable(P::Bool)).unwrap();
    run(&mut engine);
    assert!(engine.state(requirement).unwrap().is_proven());
    assert_eq!(engine.freeze(&term).unwrap(), Term::Primitive(P::Bool));
    assert!(engine.freeze(&T::Meta(unused)).is_err());
}

#[test]
fn metered_advance_keeps_unprocessed_obligations_queued() {
    let mut engine = Engine::default();
    let ids = (0..8)
        .map(|_| engine.require(R::Equal(stable(P::Bool), stable(P::Bool))))
        .collect::<Vec<_>>();
    let mut empty = Budget {
        steps: 0,
        ..Default::default()
    };
    assert_eq!(engine.advance_metered(8, &mut empty), 0);
    let mut small = Budget {
        steps: 1,
        ..Default::default()
    };
    assert_eq!(engine.advance_metered(8, &mut small), 1);
    assert_eq!(small.steps, 0);
    assert!(ids[1..].iter().all(|id| engine.attempts(*id) == Some(0)));
    engine.retry_unknown();
    let mut enough = Budget::default();
    engine.advance_metered(8, &mut enough);
    assert!(ids.iter().all(|id| engine.state(*id).unwrap().is_proven()));
    assert!(enough.steps < Budget::default().steps);
}

#[test]
fn bounded_obligation_preparation_pauses_without_poisoning_inference() {
    let ty = Term::Tuple(vec![Term::Primitive(P::Bool); 256]);
    for requirement in [
        R::Equal(T::Stable(ty.clone()), T::Stable(ty.clone())),
        R::Stable(F::equal(ty.clone(), ty.clone())),
    ] {
        let mut engine = Engine::default();
        let id = engine.require(requirement);
        let mut small = Budget {
            steps: 16,
            ..Default::default()
        };
        assert_eq!(engine.advance_metered(1, &mut small), 1);
        assert!(matches!(engine.state(id), Some(State::Ready)));
        assert!(small.steps < 16);
        let mut enough = Budget::default();
        assert_eq!(engine.advance_metered(1, &mut enough), 1);
        assert!(engine.state(id).unwrap().is_proven());
        assert_eq!(engine.attempts(id), Some(2));
    }
}
