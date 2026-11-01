use nir::{DeclarationId, Primitive, Sort, Term};
use ptc::{Budget, Context, Formula as F, Outcome, TypeInputOutcome};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 7, local }
}
fn var(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn head(arg: Term) -> Term {
    Term::NormalForm {
        head: id(100),
        sort: Sort::Type,
        arguments: vec![arg],
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}

#[test]
fn opaque_models_supply_checked_symbolic_type_input_solutions() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let constraint = F::And(vec![
        F::equal(var(0), head(var(1))),
        F::equal(var(1), var(9)),
    ]);
    let inputs = [id(0), id(1)];
    let outcome = context.search_type_inputs(&inputs, constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Unique(solution) = outcome else {
        panic!("{outcome}")
    };
    assert_eq!(solution.bindings[&id(0)].0, head(var(9)));
    assert_eq!(solution.bindings[&id(1)].0, var(9));
    let problem = context
        .type_input_context(&inputs, constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    for (_, proof) in solution.bindings.values() {
        problem.verify_proof(proof, Budget::default()).unwrap();
    }
    assert!(!solution.model.applications.is_empty());
    assert!(context.assumptions().is_empty());
}

#[test]
fn opaque_model_tables_are_checked_and_do_not_grant_injectivity() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    context.assume(F::equal(head(var(0)), head(var(1))));
    let outcome = context.prove(F::equal(var(0), var(1)), Budget::default());
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    context.verify_witness(&witness, Budget::default()).unwrap();
    assert_ne!(witness.types[&id(0)], witness.types[&id(1)]);
    let mut missing = witness.clone();
    missing.applications.clear();
    assert!(context.verify_witness(&missing, Budget::default()).is_err());
    let mut duplicate = witness.clone();
    duplicate
        .applications
        .push(duplicate.applications[0].clone());
    assert!(
        context
            .verify_witness(&duplicate, Budget::default())
            .is_err()
    );
    let mut changed = witness.clone();
    changed.applications[0].result = boolean();
    assert!(context.verify_witness(&changed, Budget::default()).is_err());
    let mut foreign = witness.clone();
    foreign.applications[0].head = id(101);
    assert!(context.verify_witness(&foreign, Budget::default()).is_err());
    let dump = Outcome::NotEntailed(witness).to_string();
    assert!(dump.contains("(applications (type-application (head"));
}

#[test]
fn nested_opaque_calls_replay_and_unadmitted_heads_remain_unknown() {
    let goal = F::equal(head(head(var(0))), boolean());
    assert!(matches!(
        Context::default().prove(goal.clone(), Budget::default()),
        Outcome::Unknown { .. }
    ));
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let outcome = context.prove(goal, Budget::default());
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    context.verify_witness(&witness, Budget::default()).unwrap();
    assert_eq!(witness.applications.len(), 2);
}

#[test]
fn model_interpretations_obey_budgets_and_do_not_escape_failed_search() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let original = context.clone();
    for steps in [0, 1, 10, 100, 1000, 100_000] {
        let outcome = context.prove(
            F::equal(head(var(0)), boolean()),
            Budget {
                steps,
                ..Budget::default()
            },
        );
        match outcome {
            Outcome::NotEntailed(witness) => {
                context.verify_witness(&witness, Budget::default()).unwrap();
                assert!(matches!(
                    context.verify_witness(
                        &witness,
                        Budget {
                            steps: 1,
                            ..Budget::default()
                        }
                    ),
                    Err(ptc::ProofError::BudgetExhausted)
                ));
            }
            Outcome::Unknown {
                reason: ptc::UnknownReason::BudgetExhausted,
                ..
            } => {}
            other => panic!("{steps}: {other}"),
        }
        assert_eq!(context, original);
    }
}

#[test]
fn cyclic_opaque_equations_have_finite_interpretations_not_recursive_types() {
    for wrapped in [false, true] {
        let mut context = Context::default();
        context.admit_congruence(id(100));
        let value = head(var(0));
        let value = if wrapped {
            Term::Pointer(Box::new(value))
        } else {
            value
        };
        context.assume(F::equal(var(0), value));
        let outcome = context.prove(F::False, Budget::default());
        let Outcome::NotEntailed(witness) = outcome else {
            panic!("{outcome}")
        };
        context.verify_witness(&witness, Budget::default()).unwrap();
        assert_eq!(witness.applications.len(), 1);
        assert_eq!(
            witness.applications[0].arguments,
            vec![witness.types[&id(0)].clone()]
        );
    }
}
