use nir::{DeclarationId, Primitive as P, Sort, Term, effects::Operation};
use ptc::{Budget, Context, Formula as F, Outcome, ProofError, TypeInputOutcome};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 95, local }
}
fn variable() -> Term {
    Term::Rigid {
        declaration: id(0),
        sort: Sort::Type,
    }
}
fn op(local: u32, arguments: Vec<Term>) -> Operation {
    Operation {
        declaration: id(local),
        arguments,
    }
}
fn ty(p: P) -> Term {
    Term::Primitive(p)
}
fn row() -> Term {
    Term::computation(
        vec![
            op(1, vec![ty(P::Bool)]),
            op(2, vec![]),
            op(1, vec![ty(P::Bool)]),
        ],
        ty(P::U8),
    )
}

#[test]
fn explicit_effect_type_models_retain_repeated_occurrences() {
    let context = Context::default();
    let constraint = F::equal(variable(), row());
    let outcome = context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Unique(solution) = outcome else {
        panic!("{outcome}")
    };
    let Term::Computation { effects, .. } = &solution.bindings[&id(0)].0 else {
        panic!()
    };
    assert_eq!(effects.len(), 3);
    assert_eq!(
        effects
            .iter()
            .filter(|operation| operation.declaration == id(1))
            .count(),
        2
    );
    let problem = context
        .type_input_context(&[id(0)], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_proof(&solution.bindings[&id(0)].1, Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    let mut corrupt = solution.model;
    let Term::Computation { effects, .. } = corrupt.types.get_mut(&id(0)).unwrap() else {
        panic!()
    };
    effects.remove(0);
    assert_eq!(
        problem.verify_witness(&corrupt, Budget::default()),
        Err(ProofError::Invalid)
    );
}

#[test]
fn effect_model_replay_checks_order_flattening_and_closed_inputs() {
    let mut context = Context::default();
    context.assume(F::equal(variable(), row()));
    let outcome = context.prove(F::False, Budget::default());
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    context.verify_witness(&witness, Budget::default()).unwrap();
    let mut reversed = witness.clone();
    let Term::Computation { effects, .. } = reversed.types.get_mut(&id(0)).unwrap() else {
        panic!()
    };
    effects.reverse();
    assert_eq!(
        context.verify_witness(&reversed, Budget::default()),
        Err(ProofError::Invalid)
    );
    let mut raw = witness.clone();
    raw.types.insert(
        id(0),
        Term::Computation {
            effects: vec![],
            result: Box::new(witness.types[&id(0)].clone()),
        },
    );
    assert_eq!(
        context.verify_witness(&raw, Budget::default()),
        Err(ProofError::Invalid)
    );
}

#[test]
fn effect_models_preserve_completed_scalar_label_inputs() {
    let context = Context::default();
    let expected = Term::computation(
        vec![
            op(1, vec![Term::Bool(true)]),
            op(1, vec![Term::Bool(false)]),
        ],
        ty(P::Bool),
    );
    let constraint = F::equal(variable(), expected);
    let outcome = context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Unique(solution) = outcome else {
        panic!("{outcome}")
    };
    let problem = context
        .type_input_context(&[id(0)], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    problem
        .verify_proof(&solution.bindings[&id(0)].1, Budget::default())
        .unwrap();
}

#[test]
fn symbolic_effect_solutions_do_not_export_model_assignments_for_lexical_inputs() {
    let context = Context::with_scope(vec![vec![Sort::Type], vec![Sort::Type]]);
    let bound = |depth| Term::Bound {
        depth,
        index: 0,
        sort: Sort::Type,
    };
    let expected = Term::computation(
        vec![
            op(1, vec![bound(1)]),
            op(1, vec![bound(0)]),
            op(1, vec![bound(1)]),
        ],
        bound(0),
    );
    let constraint = F::equal(variable(), expected.clone());
    let outcome = context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Unique(solution) = outcome else {
        panic!("{outcome}")
    };
    assert_eq!(solution.bindings[&id(0)].0, expected);
    let problem = context
        .type_input_context(&[id(0)], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    problem
        .verify_proof(&solution.bindings[&id(0)].1, Budget::default())
        .unwrap();
}

#[test]
fn occurrence_multiplicity_distinguishes_ambiguous_models() {
    let context = Context::default();
    let single = Term::computation(vec![op(1, vec![])], ty(P::Bool));
    let double = Term::computation(vec![op(1, vec![]), op(1, vec![])], ty(P::Bool));
    let constraint = F::Or(vec![
        F::equal(variable(), single),
        F::equal(variable(), double),
    ]);
    let outcome = context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Ambiguous {
        candidate,
        alternative,
        ..
    } = outcome
    else {
        panic!("{outcome}")
    };
    assert_ne!(candidate.types[&id(0)], alternative.types[&id(0)]);
    let problem = context
        .type_input_context(&[id(0)], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&candidate, Budget::default())
        .unwrap();
    problem
        .verify_witness(&alternative, Budget::default())
        .unwrap();
}

#[test]
fn nested_model_rows_flatten_without_erasing_occurrences() {
    let raw = Term::Computation {
        effects: vec![op(2, vec![])],
        result: Box::new(row()),
    };
    let mut context = Context::default();
    context.assume(F::equal(variable(), raw));
    let outcome = context.prove(F::False, Budget::default());
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    let Term::Computation { effects, result } = &witness.types[&id(0)] else {
        panic!()
    };
    assert_eq!(effects.len(), 4);
    assert_eq!(result.as_ref(), &ty(P::U8));
    context.verify_witness(&witness, Budget::default()).unwrap();
    assert_eq!(
        context.verify_witness(
            &witness,
            Budget {
                steps: 1,
                ..Budget::default()
            }
        ),
        Err(ProofError::BudgetExhausted)
    );
    context.verify_witness(&witness, Budget::default()).unwrap();
}

#[test]
fn unsupported_label_values_and_cyclic_rows_do_not_acquire_models() {
    for expected in [
        Term::computation(vec![op(1, vec![Term::Float32(0)])], ty(P::Bool)),
        Term::computation(vec![op(1, vec![])], variable()),
    ] {
        let mut context = Context::default();
        context.assume(F::equal(variable(), expected));
        assert!(matches!(
            context.prove(F::False, Budget::default()),
            Outcome::Unknown { .. }
        ));
    }
}
