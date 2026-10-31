use nir::{DeclarationId, Primitive, Sort, Term};
use ptc::{Budget, Context, Formula as F, TypeInputOutcome as O, UnknownReason};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 7, local }
}
fn var(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn byte() -> Term {
    Term::Primitive(Primitive::U8)
}

#[test]
fn disjunction_search_extracts_only_determined_requested_inputs() {
    let context = Context::default();
    let constraint = F::And(vec![
        F::Or(vec![F::equal(var(0), boolean()), F::equal(var(0), byte())]),
        F::equal(var(0), byte()).negate(),
        F::equal(var(1), Term::Optional(Box::new(var(0)))),
        F::equal(var(9), boolean()),
    ]);
    let inputs = [id(0), id(1)];
    let O::Unique(solution) =
        context.search_type_inputs(&inputs, constraint.clone(), &mut Budget::default())
    else {
        panic!("the disjunction and exclusion determine both requested inputs");
    };
    assert_eq!(solution.bindings.len(), 2);
    assert_eq!(solution.bindings[&id(0)].0, boolean());
    assert_eq!(
        solution.bindings[&id(1)].0,
        Term::Optional(Box::new(boolean()))
    );
    let problem = context
        .type_input_context(&inputs, constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    for (variable, (value, proof)) in &solution.bindings {
        assert_eq!(
            proof.goal,
            F::equal(
                Term::Rigid {
                    declaration: *variable,
                    sort: Sort::Type
                },
                value.clone()
            )
        );
        problem.verify_proof(proof, Budget::default()).unwrap();
        assert!(context.verify_proof(proof, Budget::default()).is_err());
    }
    assert!(context.assumptions().is_empty());
}

#[test]
fn ambiguous_and_unconstrained_inputs_retain_two_checked_models() {
    let context = Context::default();
    for constraint in [
        F::True,
        F::Or(vec![F::equal(var(0), boolean()), F::equal(var(0), byte())]),
    ] {
        let outcome =
            context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
        let O::Ambiguous {
            variable,
            candidate,
            alternative,
        } = outcome
        else {
            panic!("{outcome}")
        };
        assert_eq!(variable, id(0));
        assert_ne!(candidate.types[&variable], alternative.types[&variable]);
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
}

#[test]
fn inconsistency_exhaustion_and_bad_requests_never_become_solutions() {
    let context = Context::default();
    let constraint = F::And(vec![F::equal(var(0), boolean()), F::equal(var(0), byte())]);
    let outcome = context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
    let O::Inconsistent(proof) = outcome else {
        panic!("{outcome}")
    };
    let problem = context
        .type_input_context(&[id(0)], constraint, &mut Budget::default())
        .unwrap();
    assert_eq!(proof.goal, F::False);
    problem.verify_proof(&proof, Budget::default()).unwrap();
    let mut budget = Budget {
        steps: 1,
        ..Budget::default()
    };
    assert!(matches!(
        context.search_type_inputs(&[id(0)], F::equal(var(0), boolean()), &mut budget),
        O::Unknown(UnknownReason::BudgetExhausted)
    ));
    assert!(budget.steps <= 1);
    assert!(matches!(
        context.search_type_inputs(&[id(0), id(0)], F::True, &mut Budget::default()),
        O::IllFormed(_)
    ));
    assert!(matches!(
        context.search_type_inputs(
            &[id(0)],
            F::equal(var(0), boolean()),
            &mut Budget::default()
        ),
        O::Unique(_)
    ));
}

#[test]
fn opaque_head_equality_does_not_solve_its_argument_by_inversion() {
    let mut context = Context::default();
    context.admit_congruence(id(100));
    let head = |arg| Term::NormalForm {
        head: id(100),
        sort: Sort::Type,
        arguments: vec![arg],
    };
    let constraint = F::equal(head(var(0)), head(boolean()));
    let result = context.search_type_inputs(&[id(0)], constraint, &mut Budget::default());
    assert!(
        !matches!(result, O::Unique(_) | O::Inconsistent(_)),
        "{result}"
    );
}

#[test]
fn free_model_elements_avoid_explicit_pointer_types() {
    let context = Context::default();
    let mut concrete = boolean();
    let mut constraints = Vec::new();
    for _ in 0..5 {
        concrete = Term::Pointer(Box::new(concrete));
        constraints.push(F::equal(var(0), concrete.clone()).negate());
    }
    let constraint = F::And(constraints);
    let outcome = context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
    let O::Ambiguous {
        candidate,
        alternative,
        ..
    } = outcome
    else {
        panic!("{outcome}")
    };
    let problem = context
        .type_input_context(&[id(0)], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&candidate, Budget::default())
        .unwrap();
    problem
        .verify_witness(&alternative, Budget::default())
        .unwrap();
    assert_ne!(candidate.types[&id(0)], alternative.types[&id(0)]);
}

#[test]
fn explicit_input_search_does_not_bind_ambient_variables_or_leak_across_branches() {
    let mut context = Context::default();
    context.assume(F::equal(var(9), boolean()));
    let original = context.clone();
    for value in [boolean(), byte()] {
        let constraint = F::equal(var(0), value.clone());
        let outcome = context.search_type_inputs(&[id(0)], constraint, &mut Budget::default());
        let O::Unique(solution) = outcome else {
            panic!("{outcome}")
        };
        assert_eq!(solution.bindings.len(), 1);
        assert_eq!(solution.bindings[&id(0)].0, value);
        assert_eq!(context, original);
    }
    let malformed = F::equal(
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
        boolean(),
    );
    assert!(matches!(
        context.search_type_inputs(&[id(0)], malformed, &mut Budget::default()),
        O::IllFormed(_)
    ));
}
