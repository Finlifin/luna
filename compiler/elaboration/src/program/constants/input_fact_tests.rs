//! Rollback and lexical exit must not confuse supplied values with provenance.
use super::*;
use crate::program::input_facts::State;
use rustc_span::DUMMY_SP;

fn id(index: u32) -> n::LocalId {
    n::LocalId { owner: 1, index }
}

fn facts() -> State {
    let mut facts = State::default();
    facts
        .specialized
        .push((id(1), p(P::Bool), Term::Bool(true)));
    facts.types.insert(id(2), p(P::U8));
    facts.values.insert(id(1), (Term::Bool(true), p(P::Bool)));
    facts
        .partial
        .insert(id(1), PartialStagedValue::known(Term::Bool(true)));
    facts.derived.insert(id(1));
    facts.supplied = facts
        .supplied
        .child(
            &[(id(1), p(P::Bool), Term::Bool(true))],
            &[],
            &mut 1000,
            DUMMY_SP,
        )
        .unwrap();
    facts
}

#[test]
fn speculative_invalidation_does_not_change_the_saved_supplied_region() {
    let parent = facts();
    let mut attempt = parent.clone();
    attempt
        .invalidate([id(1)].into(), true, &mut 1000, DUMMY_SP)
        .unwrap();
    assert!(
        attempt
            .supplied
            .lookup(id(1), &mut 100, DUMMY_SP)
            .unwrap()
            .unwrap()
            .1
            .is_none()
    );
    assert!(!attempt.partial.contains_key(&id(1)));
    assert_eq!(
        parent
            .supplied
            .lookup(id(1), &mut 100, DUMMY_SP)
            .unwrap()
            .unwrap()
            .1,
        Some(&Term::Bool(true))
    );
    assert!(parent.partial.contains_key(&id(1)));
    // Definition-side provenance and runtime specialization are separate from
    // the invocation-local observation invalidated by this staged write.
    assert!(attempt.derived.contains(&id(1)));
    assert_eq!(attempt.specialized, parent.specialized);
    assert_eq!(attempt.values, parent.values);
}

#[test]
fn lexical_exit_discards_inner_evidence_without_reviving_outer_snapshots() {
    let mut state = facts();
    let boundary = state.lexical_boundary();
    state
        .invalidate([id(1)].into(), false, &mut 1000, DUMMY_SP)
        .unwrap();
    state.specialized.push((id(3), p(P::Type), p(P::Bool)));
    state.types.insert(id(3), p(P::Bool));
    state.values.insert(id(3), (Term::Bool(false), p(P::Bool)));
    state
        .partial
        .insert(id(3), PartialStagedValue::known(Term::Bool(false)));
    state.derived.insert(id(3));
    state.leave_lexical(&boundary);
    assert!(state.specialized.is_empty());
    assert!(state.values.is_empty());
    assert!(state.partial.is_empty());
    assert_eq!(state.types, [(id(2), p(P::U8))].into());
    assert_eq!(state.derived, [id(1)].into());
    assert!(
        state
            .supplied
            .lookup(id(1), &mut 100, DUMMY_SP)
            .unwrap()
            .unwrap()
            .1
            .is_none()
    );
}

#[test]
fn open_body_inherits_lexical_evidence_without_mutating_the_callers_scope() {
    let parent = facts();
    let mut body = parent.for_open_body();
    assert!(body.specialized.is_empty() && body.partial.is_empty() && body.derived.is_empty());
    assert_eq!(body.types, parent.types);
    assert_eq!(body.values, parent.values);
    body.invalidate([id(1)].into(), true, &mut 1000, DUMMY_SP)
        .unwrap();
    assert!(
        body.supplied
            .lookup(id(1), &mut 100, DUMMY_SP)
            .unwrap()
            .unwrap()
            .1
            .is_none()
    );
    assert_eq!(
        parent
            .supplied
            .lookup(id(1), &mut 100, DUMMY_SP)
            .unwrap()
            .unwrap()
            .1,
        Some(&Term::Bool(true))
    );
}

#[test]
fn failed_invalidation_keeps_the_complete_evidence_owner_and_spends_its_budget() {
    let mut state = facts();
    let before = state.to_string();
    let mut fuel = 0;
    assert!(
        state
            .invalidate([id(1)].into(), false, &mut fuel, DUMMY_SP)
            .is_err()
    );
    assert_eq!(fuel, 0);
    assert_eq!(state.to_string(), before);
    state
        .invalidate([id(1)].into(), false, &mut 1000, DUMMY_SP)
        .unwrap();
    assert!(!state.values.contains_key(&id(1)));
}

#[test]
fn dump_distinguishes_partial_observations_supplied_values_and_provenance() {
    let mut state = facts();
    state.partial.insert(
        id(3),
        std::sync::Arc::new(PartialStagedValue {
            depth: 1,
            data: PartialStagedData::Aggregate(
                PartialStagedShape::AnonymousRecord(vec!["field\"(\n".into()]),
                vec![PartialStagedValue::unknown()],
            ),
        }),
    );
    let dump = state.to_string();
    assert!(dump.contains("(known (bool true))"), "{dump}");
    assert!(
        dump.contains("(names \"field\\\"(\\n\") (unknown)"),
        "{dump}"
    );
    assert!(dump.contains("(derived (local 1 1))"), "{dump}");
    assert_eq!(dump, state.to_string());
}
