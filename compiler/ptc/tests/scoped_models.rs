use nir::{DeclarationId, Parameter, Primitive as P, Sort, Term};
use ptc::{Budget, Context, Formula as F, Outcome, ProofError, TypeBinder, TypeInputOutcome};

fn binder(depth: u32) -> TypeBinder {
    TypeBinder { depth, index: 0 }
}
fn bound(depth: u32) -> Term {
    Term::Bound {
        depth,
        index: 0,
        sort: Sort::Type,
    }
}
fn primitive(p: P) -> Term {
    Term::Primitive(p)
}
fn context() -> Context {
    Context::with_scope(vec![vec![Sort::Type], vec![Sort::Type]])
}
fn fixed_context() -> Context {
    let mut context = context();
    context.assume(F::equal(bound(1), primitive(P::Bool)));
    context.assume(F::equal(bound(0), primitive(P::U8)));
    context
}

#[test]
fn lexical_witnesses_replay_and_reject_missing_corrupt_or_out_of_scope_bindings() {
    let context = fixed_context();
    let result = context.prove(F::equal(bound(1), bound(0)), Budget::default());
    let Outcome::NotEntailed(witness) = &result else {
        panic!("{result}")
    };
    assert_eq!(witness.bound_types[&binder(1)], primitive(P::Bool));
    assert_eq!(witness.bound_types[&binder(0)], primitive(P::U8));
    assert!(
        result
            .to_string()
            .contains("(type-binder (depth 1) (index 0))")
    );
    context.verify_witness(witness, Budget::default()).unwrap();
    let mut missing = witness.clone();
    missing.bound_types.remove(&binder(0));
    assert_eq!(
        context.verify_witness(&missing, Budget::default()),
        Err(ProofError::Invalid)
    );
    for (key, value) in [
        (binder(0), primitive(P::Bool)),
        (binder(2), primitive(P::Bool)),
        (TypeBinder { depth: 0, index: 1 }, primitive(P::Bool)),
        (binder(0), bound(0)),
        (binder(0), Term::Bool(true)),
    ] {
        let mut corrupt = witness.clone();
        corrupt.bound_types.insert(key, value);
        assert_eq!(
            context.verify_witness(&corrupt, Budget::default()),
            Err(ProofError::Invalid)
        );
    }
    let mut bad_scope = witness.clone();
    bad_scope.scope.pop();
    assert_eq!(
        context.verify_witness(&bad_scope, Budget::default()),
        Err(ProofError::Invalid)
    );
    let mut fuel = Budget {
        steps: 1,
        ..Budget::default()
    };
    assert_eq!(
        context.verify_witness_metered(witness, &mut fuel),
        Err(ProofError::BudgetExhausted)
    );
    context.verify_witness(witness, Budget::default()).unwrap();
}

#[test]
fn symbolic_solutions_preserve_lexical_inputs_and_opaque_heads() {
    let input = DeclarationId {
        package: 77,
        local: 1,
    };
    let head = DeclarationId {
        package: 77,
        local: 2,
    };
    let mut context = context();
    context.admit_congruence(head);
    let expected = Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![Term::Tuple(vec![bound(1), bound(0)])],
    };
    let constraint = F::equal(
        Term::Rigid {
            declaration: input,
            sort: Sort::Type,
        },
        expected.clone(),
    );
    let outcome = context.search_type_inputs(&[input], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Unique(solution) = outcome else {
        panic!("{outcome}")
    };
    assert_eq!(solution.bindings[&input].0, expected);
    let problem = context
        .type_input_context(&[input], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    problem
        .verify_proof(&solution.bindings[&input].1, Budget::default())
        .unwrap();
}

#[test]
fn ambiguity_requires_two_models_with_the_same_lexical_environment() {
    let context = fixed_context();
    let input = DeclarationId {
        package: 77,
        local: 3,
    };
    let variable = Term::Rigid {
        declaration: input,
        sort: Sort::Type,
    };
    let constraint = F::Or(vec![
        F::equal(variable.clone(), bound(0)),
        F::equal(variable, bound(1)),
    ]);
    let outcome = context.search_type_inputs(&[input], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Ambiguous {
        candidate,
        alternative,
        ..
    } = outcome
    else {
        panic!("{outcome}")
    };
    assert_eq!(candidate.bound_types, alternative.bound_types);
    assert_ne!(candidate.types[&input], alternative.types[&input]);
    let problem = context
        .type_input_context(&[input], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&candidate, Budget::default())
        .unwrap();
    problem
        .verify_witness(&alternative, Budget::default())
        .unwrap();
}

#[test]
fn callable_local_binders_do_not_become_ambient_model_assignments() {
    let context = context();
    let callable = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: primitive(P::Type),
        }],
        result: Box::new(bound(0)),
    };
    // This binder belongs to the callable itself, despite sharing a coordinate
    // with an ambient input. Unsupported higher-order models must stay Unknown.
    assert!(matches!(
        context.prove(F::equal(callable, primitive(P::Bool)), Budget::default()),
        Outcome::Unknown { .. }
    ));
}
