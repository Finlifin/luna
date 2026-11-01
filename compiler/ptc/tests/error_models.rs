use nir::{DeclarationId, Primitive as P, Sort, Term};
use ptc::{Budget, Context, Formula as F, Outcome, ProofError, TypeInputOutcome};

fn id() -> DeclarationId {
    DeclarationId {
        package: 94,
        local: 1,
    }
}
fn variable() -> Term {
    Term::Rigid {
        declaration: id(),
        sort: Sort::Type,
    }
}
fn ty(p: P) -> Term {
    Term::Primitive(p)
}
fn row() -> Term {
    Term::error_result(vec![ty(P::Bool), ty(P::Char)], ty(P::U8))
}

#[test]
fn explicit_error_result_inputs_have_proved_models() {
    let context = Context::default();
    let constraint = F::equal(variable(), row());
    let outcome = context.search_type_inputs(&[id()], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Unique(solution) = outcome else {
        panic!("{outcome}")
    };
    assert_eq!(solution.bindings[&id()].0, row());
    let problem = context
        .type_input_context(&[id()], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    problem
        .verify_proof(&solution.bindings[&id()].1, Budget::default())
        .unwrap();
}

#[test]
fn error_model_replay_rejects_noncanonical_assignment_payloads() {
    let mut context = Context::default();
    context.assume(F::equal(variable(), row()));
    let outcome = context.prove(F::False, Budget::default());
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    context.verify_witness(&witness, Budget::default()).unwrap();
    for invalid in [
        Term::ErrorResult {
            errors: vec![ty(P::Char), ty(P::Bool)],
            result: Box::new(ty(P::U8)),
        },
        Term::ErrorResult {
            errors: vec![ty(P::Bool), ty(P::Bool), ty(P::Char)],
            result: Box::new(ty(P::U8)),
        },
        Term::ErrorResult {
            errors: vec![ty(P::Bool)],
            result: Box::new(Term::error_result(vec![ty(P::Char)], ty(P::U8))),
        },
        Term::ErrorResult {
            errors: vec![],
            result: Box::new(row()),
        },
    ] {
        let mut corrupt = witness.clone();
        corrupt.types.insert(id(), invalid);
        assert_eq!(
            context.verify_witness(&corrupt, Budget::default()),
            Err(ProofError::Invalid)
        );
    }
}

#[test]
fn raw_input_rows_are_normalized_before_model_equality() {
    let context = Context::default();
    let raw = Term::ErrorResult {
        errors: vec![ty(P::Char), ty(P::Bool), ty(P::Bool)],
        result: Box::new(Term::ErrorResult {
            errors: vec![ty(P::Char)],
            result: Box::new(ty(P::U8)),
        }),
    };
    let mut problem = context.fork();
    problem.assume(F::equal(variable(), raw));
    let outcome = problem.prove(F::equal(variable(), ty(P::Bool)), Budget::default());
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    assert_eq!(witness.types[&id()], row());
    problem.verify_witness(&witness, Budget::default()).unwrap();
}

#[test]
fn error_result_ambiguity_has_two_independently_checked_models() {
    let context = Context::default();
    let other = Term::error_result(vec![ty(P::Char)], ty(P::Bool));
    let constraint = F::Or(vec![
        F::equal(variable(), row()),
        F::equal(variable(), other),
    ]);
    let outcome = context.search_type_inputs(&[id()], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Ambiguous {
        candidate,
        alternative,
        ..
    } = outcome
    else {
        panic!("{outcome}")
    };
    let problem = context
        .type_input_context(&[id()], constraint, &mut Budget::default())
        .unwrap();
    assert_ne!(candidate.types[&id()], alternative.types[&id()]);
    problem
        .verify_witness(&candidate, Budget::default())
        .unwrap();
    problem
        .verify_witness(&alternative, Budget::default())
        .unwrap();
}

#[test]
fn scoped_error_solutions_preserve_symbolic_labels_and_residuals() {
    let context = Context::with_scope(vec![vec![Sort::Type], vec![Sort::Type]]);
    let bound = |depth| Term::Bound {
        depth,
        index: 0,
        sort: Sort::Type,
    };
    let expected = Term::error_result(vec![bound(0)], bound(1));
    let constraint = F::equal(variable(), expected.clone());
    let outcome = context.search_type_inputs(&[id()], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Unique(solution) = outcome else {
        panic!("{outcome}")
    };
    assert_eq!(solution.bindings[&id()].0, expected);
    let problem = context
        .type_input_context(&[id()], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    problem
        .verify_proof(&solution.bindings[&id()].1, Budget::default())
        .unwrap();
}

#[test]
fn opaque_application_models_accept_canonical_error_arguments_without_unfolding() {
    let head = DeclarationId {
        package: 94,
        local: 2,
    };
    let mut context = Context::default();
    context.admit_congruence(head);
    let application = Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![row()],
    };
    let constraint = F::equal(variable(), application.clone());
    let outcome = context.search_type_inputs(&[id()], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Unique(solution) = outcome else {
        panic!("{outcome}")
    };
    assert_eq!(solution.bindings[&id()].0, application);
    let problem = context
        .type_input_context(&[id()], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    let mut corrupt = solution.model;
    let Term::ErrorResult { errors, .. } = &mut corrupt.applications[0].arguments[0] else {
        panic!()
    };
    errors.reverse();
    assert_eq!(
        problem.verify_witness(&corrupt, Budget::default()),
        Err(ProofError::Invalid)
    );
}

#[test]
fn error_model_budget_exhaustion_does_not_publish_partial_solutions() {
    let context = Context::default();
    let constraint = F::equal(variable(), row());
    for steps in [0, 1, 10] {
        assert!(matches!(
            context.search_type_inputs(
                &[id()],
                constraint.clone(),
                &mut Budget {
                    steps,
                    ..Budget::default()
                }
            ),
            TypeInputOutcome::Unknown(_)
        ));
    }
    assert!(matches!(
        context.search_type_inputs(&[id()], constraint, &mut Budget::default()),
        TypeInputOutcome::Unique(_)
    ));
    let mut cyclic = context.fork();
    cyclic.assume(F::equal(
        variable(),
        Term::error_result(vec![ty(P::Bool)], variable()),
    ));
    assert!(matches!(
        cyclic.prove(F::False, Budget::default()),
        Outcome::Unknown { .. }
    ));
}
