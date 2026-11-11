//! Publication rights survive unrelated inference and cannot escape to trials.
use nir::{Primitive, Sort, Term};
use type_resolution::{InferenceContext, InferenceError as E, InferenceTerm as I};

fn ty() -> I {
    I::Stable(Term::Primitive(Primitive::Bool))
}

#[test]
fn reserved_outputs_remain_observable_but_not_assignable_by_inference() {
    let mut context = InferenceContext::default();
    let output = context.fresh(Sort::Type);
    let permit = context.reserve_assignment(output).unwrap();
    assert_eq!(context.freeze(&I::Meta(output)), Err(E::Unresolved(output)));
    assert_eq!(
        context.unresolved(&I::Meta(output)).unwrap(),
        [output].into()
    );
    assert_eq!(
        context.assign(output, ty()),
        Err(E::ReservedVariable(output))
    );
    assert_eq!(
        context.assign_with_limits(output, ty(), &mut 100, 128),
        Err(E::ReservedVariable(output))
    );
    assert!(
        matches!(context.reserve_assignment(output), Err(E::ReservedVariable(id)) if id == output)
    );
    assert!(permit.to_string().contains(&format!("(output {output})")));
    context.assign_reserved(&permit, ty()).unwrap();
    assert_eq!(
        context.freeze(&I::Meta(output)).unwrap(),
        Term::Primitive(Primitive::Bool)
    );
    assert_eq!(
        context.assign_reserved(&permit, ty()),
        Err(E::AlreadyAssigned(output))
    );
    assert_eq!(
        context.assign(output, ty()),
        Err(E::AlreadyAssigned(output))
    );
}

#[test]
fn unrelated_candidate_commit_preserves_the_original_publication_authority() {
    let mut context = InferenceContext::default();
    let output = context.fresh(Sort::Type);
    let other = context.fresh(Sort::Type);
    let before_reservation = context.trial();
    let permit = context.reserve_assignment(output).unwrap();
    assert_eq!(context.commit(before_reservation), Err(E::StaleCandidate));
    let mut candidate = context.trial();
    assert_eq!(
        candidate.assign(output, ty()),
        Err(E::ReservedVariable(output))
    );
    assert_eq!(
        candidate.assign_with_limits(output, ty(), &mut 100, 128),
        Err(E::ReservedVariable(output))
    );
    assert_eq!(
        candidate.assign_reserved(&permit, ty()),
        Err(E::SpeculativeAssignment)
    );
    assert!(matches!(
        candidate.reserve_assignment(other),
        Err(E::SpeculativeAssignment)
    ));
    candidate.assign(other, ty()).unwrap();
    context.commit(candidate).unwrap();
    assert!(!context.is_assigned(output).unwrap());
    assert!(context.is_assigned(other).unwrap());
    assert_eq!(
        context.assign(output, ty()),
        Err(E::ReservedVariable(output))
    );
    let before_publication = context.trial();
    context.assign_reserved(&permit, ty()).unwrap();
    assert_eq!(context.commit(before_publication), Err(E::StaleCandidate));
    assert!(context.is_assigned(output).unwrap());
}

#[test]
fn foreign_or_invalid_publication_does_not_consume_the_permit() {
    let mut context = InferenceContext::default();
    let output = context.fresh(Sort::Type);
    let permit = context.reserve_assignment(output).unwrap();
    let mut foreign = InferenceContext::default();
    assert_eq!(
        foreign.assign_reserved(&permit, ty()),
        Err(E::ForeignAssignment)
    );
    assert_eq!(
        context.assign_reserved(&permit, I::Meta(output)),
        Err(E::Occurs(output))
    );
    assert!(matches!(
        context.assign_reserved(&permit, I::Stable(Term::Bool(true))),
        Err(E::WrongSort { .. })
    ));
    assert!(!context.is_assigned(output).unwrap());
    context.assign_reserved(&permit, ty()).unwrap();
}

#[test]
fn abandoning_a_producer_never_reopens_its_output() {
    let mut context = InferenceContext::default();
    let output = context.fresh(Sort::Type);
    drop(context.reserve_assignment(output).unwrap());
    assert_eq!(
        context.assign(output, ty()),
        Err(E::ReservedVariable(output))
    );
    assert!(
        matches!(context.reserve_assignment(output), Err(E::ReservedVariable(id)) if id == output)
    );
    assert!(!context.is_assigned(output).unwrap());
    let next = context.fresh(Sort::Type);
    let permit = context.reserve_assignment(next).unwrap();
    context.assign_reserved(&permit, ty()).unwrap();
    assert!(context.is_assigned(next).unwrap());
}
