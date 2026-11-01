use nir::{DeclarationId, Primitive, Sort, Term, effects::Operation};
use ptc::{Budget, Context, Formula, Outcome, ProofError};

fn bound(depth: u32) -> Term {
    Term::Bound {
        depth,
        index: 0,
        sort: Sort::Type,
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn context() -> Context {
    Context::with_scope(vec![vec![Sort::Type], vec![Sort::Type]])
}
fn effect(argument: Term) -> Operation {
    Operation {
        declaration: DeclarationId {
            package: 77,
            local: 1,
        },
        arguments: vec![argument],
    }
}
fn effects(arguments: Vec<Term>, result: Term) -> Term {
    Term::computation(arguments.into_iter().map(effect).collect(), result)
}
fn prove(context: &Context, goal: Formula) {
    let outcome = context.prove(goal, Budget::default());
    let Outcome::Entailed(proof) = outcome else {
        panic!("{outcome}")
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
    let mut wrong_scope = Context::with_scope(vec![vec![Sort::Type]]);
    for assumption in context.assumptions() {
        wrong_scope.assume(assumption.clone());
    }
    assert_eq!(
        wrong_scope.verify_proof(&proof, Budget::default()),
        Err(ProofError::Invalid)
    );
}

#[test]
fn scoped_error_labels_coalesce_only_with_equality_evidence() {
    let mut context = context();
    let goal = Formula::equal(
        Term::error_result(vec![bound(0), bound(1)], boolean()),
        Term::error_result(vec![bound(0)], boolean()),
    );
    assert!(!matches!(
        context.prove(goal.clone(), Budget::default()),
        Outcome::Entailed(_)
    ));
    context.assume(Formula::equal(bound(0), bound(1)));
    prove(&context, goal);
}

#[test]
fn scoped_effect_labels_commute_only_with_distinctness_evidence() {
    let mut context = context();
    let goal = Formula::equal(
        effects(vec![bound(0), bound(1)], boolean()),
        effects(vec![bound(1), bound(0)], boolean()),
    );
    assert!(!matches!(
        context.prove(goal.clone(), Budget::default()),
        Outcome::Entailed(_)
    ));
    context.assume(Formula::equal(bound(0), bound(1)).negate());
    prove(&context, goal);
    prove(
        &context,
        Formula::equal(
            effects(vec![bound(0), bound(0)], boolean()),
            effects(vec![bound(0)], boolean()),
        )
        .negate(),
    );
}

#[test]
fn row_congruence_substitutes_lexical_labels_and_result_aliases() {
    let mut context = context();
    context.assume(Formula::equal(bound(0), boolean()));
    context.assume(Formula::equal(
        bound(1),
        effects(vec![boolean()], boolean()),
    ));
    prove(
        &context,
        Formula::equal(
            effects(vec![bound(0)], bound(1)),
            effects(vec![boolean(), boolean()], boolean()),
        ),
    );
    prove(
        &context,
        Formula::equal(
            Term::error_result(vec![bound(0)], boolean()),
            Term::error_result(vec![boolean()], boolean()),
        ),
    );
}

#[test]
fn scoped_unknown_labels_do_not_prove_absence_or_open_tail_length() {
    let context = context();
    for goal in [
        Formula::equal(
            Term::error_result(vec![bound(0)], boolean()),
            Term::error_result(vec![bound(1)], boolean()),
        )
        .negate(),
        Formula::equal(
            effects(vec![bound(0)], bound(1)),
            effects(vec![bound(0), bound(0)], boolean()),
        )
        .negate(),
    ] {
        assert!(!matches!(
            context.prove(goal, Budget::default()),
            Outcome::Entailed(_)
        ));
    }
}

#[test]
fn row_labels_do_not_capture_callable_local_binders() {
    let mut context = context();
    context.assume(Formula::equal(bound(0), boolean()));
    let callable = |result| Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
        result: Box::new(result),
    };
    let generic = callable(bound(0));
    let constant = callable(boolean());
    for goal in [
        Formula::equal(
            effects(vec![generic.clone()], boolean()),
            effects(vec![constant.clone()], boolean()),
        ),
        Formula::equal(
            Term::error_result(vec![generic], boolean()),
            Term::error_result(vec![constant], boolean()),
        ),
    ] {
        assert!(!matches!(
            context.prove(goal, Budget::default()),
            Outcome::Entailed(_)
        ));
    }
}

#[test]
fn interrupted_scoped_row_proofs_can_be_retried_without_foreign_facts() {
    let mut context = context();
    let goal = Formula::equal(
        effects(vec![bound(0), bound(1)], boolean()),
        effects(vec![bound(1), bound(0)], boolean()),
    );
    context.assume(Formula::equal(bound(0), bound(1)).negate());
    for steps in [0, 1, 10, 100] {
        let _ = context.prove(
            goal.clone(),
            Budget {
                steps,
                ..Budget::default()
            },
        );
        prove(&context, goal.clone());
    }
    assert!(matches!(
        Context::default().prove(goal, Budget::default()),
        Outcome::IllFormed(_)
    ));
}
