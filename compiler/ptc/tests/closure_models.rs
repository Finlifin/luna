use nir::{DeclarationId, Primitive as P, Sort, Term};
use ptc::{Budget, Context, Formula as F, Outcome, ProofError, TypeInputOutcome};

fn bound(depth: u32) -> Term {
    Term::Bound {
        depth,
        index: 0,
        sort: Sort::Type,
    }
}
fn ty(value: P) -> Term {
    Term::Primitive(value)
}
fn closure(environment: Vec<Term>) -> Term {
    Term::Closure {
        environment,
        signature: Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(ty(P::Bool)),
        }),
    }
}
fn id() -> DeclarationId {
    DeclarationId {
        package: 95,
        local: 1,
    }
}
fn variable() -> Term {
    Term::Rigid {
        declaration: id(),
        sort: Sort::Type,
    }
}

#[test]
fn closure_environment_equalities_preserve_ambient_scope() {
    let mut context = Context::with_scope(vec![vec![Sort::Type], vec![Sort::Type]]);
    context.assume(F::equal(
        closure(vec![bound(1), bound(0)]),
        closure(vec![ty(P::Bool), ty(P::Char)]),
    ));
    for goal in [
        F::equal(bound(1), ty(P::Bool)),
        F::equal(bound(0), ty(P::Char)),
    ] {
        let outcome = context.prove(goal, Budget::default());
        let Outcome::Entailed(proof) = outcome else {
            panic!("{outcome}")
        };
        context.verify_proof(&proof, Budget::default()).unwrap();
    }
}

#[test]
fn explicit_closure_inputs_keep_symbolic_environments_and_checked_models() {
    let context = Context::with_scope(vec![vec![Sort::Type], vec![Sort::Type]]);
    let expected = closure(vec![bound(1), Term::Optional(Box::new(bound(0)))]);
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
        .verify_proof(&solution.bindings[&id()].1, Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    let mut corrupt = solution.model.clone();
    corrupt.types.insert(id(), closure(vec![]));
    assert_eq!(
        problem.verify_witness(&corrupt, Budget::default()),
        Err(ProofError::Invalid)
    );
    let mut budget = Budget {
        steps: 1,
        ..Budget::default()
    };
    assert_eq!(
        problem.verify_witness_metered(&solution.model, &mut budget),
        Err(ProofError::BudgetExhausted)
    );
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
}

#[test]
fn callable_local_binders_inside_closures_are_not_ambient_inputs() {
    let mut context = Context::with_scope(vec![vec![Sort::Type]]);
    context.assume(F::equal(bound(0), ty(P::Bool)));
    let dependent = Term::Closure {
        environment: vec![bound(0)],
        signature: Box::new(Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Type,
                ty: ty(P::Type),
            }],
            result: Box::new(bound(0)),
        }),
    };
    let outcome = context.search_type_inputs(
        &[id()],
        F::equal(variable(), dependent),
        &mut Budget::default(),
    );
    assert!(matches!(outcome, TypeInputOutcome::Unknown(_)), "{outcome}");
    let retry = context.search_type_inputs(
        &[id()],
        F::equal(variable(), closure(vec![bound(0)])),
        &mut Budget::default(),
    );
    assert!(matches!(retry, TypeInputOutcome::Unique(_)), "{retry}");
}

#[test]
fn closure_choices_remain_ambiguous_with_the_same_ambient_assignment() {
    let mut context = Context::with_scope(vec![vec![Sort::Type]]);
    context.assume(F::equal(bound(0), ty(P::Char)));
    let constraint = F::Or(vec![
        F::equal(variable(), closure(vec![bound(0)])),
        F::equal(variable(), closure(vec![ty(P::Bool)])),
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
    assert_eq!(candidate.bound_types, alternative.bound_types);
    assert_ne!(candidate.types[&id()], alternative.types[&id()]);
    let problem = context
        .type_input_context(&[id()], constraint, &mut Budget::default())
        .unwrap();
    for model in [&candidate, &alternative] {
        problem.verify_witness(model, Budget::default()).unwrap();
    }
}

#[test]
fn empty_environments_disappear_in_theory_and_cannot_forge_distinct_models() {
    let empty = closure(vec![]);
    let Term::Closure { signature, .. } = &empty else {
        unreachable!()
    };
    let function = signature.as_ref().clone();
    let context = Context::default();
    let outcome = context.prove(F::equal(empty.clone(), function.clone()), Budget::default());
    let Outcome::Entailed(proof) = outcome else {
        panic!("{outcome}")
    };
    context.verify_proof(&proof, Budget::default()).unwrap();
    let constraint = F::equal(variable(), empty.clone());
    let outcome = context.search_type_inputs(&[id()], constraint.clone(), &mut Budget::default());
    let TypeInputOutcome::Unique(solution) = outcome else {
        panic!("{outcome}")
    };
    assert_eq!(solution.model.types[&id()], function);
    let problem = context
        .type_input_context(&[id()], constraint, &mut Budget::default())
        .unwrap();
    problem
        .verify_witness(&solution.model, Budget::default())
        .unwrap();
    let mut corrupt = solution.model.clone();
    corrupt.types.insert(id(), empty);
    assert_eq!(
        problem.verify_witness(&corrupt, Budget::default()),
        Err(ProofError::Invalid)
    );
    let scoped = Context::with_scope(vec![vec![Sort::Type]]);
    let outcome = scoped.prove(
        F::equal(closure(vec![bound(0)]), function),
        Budget::default(),
    );
    let Outcome::NotEntailed(model) = outcome else {
        panic!("{outcome}")
    };
    scoped.verify_witness(&model, Budget::default()).unwrap();
}
