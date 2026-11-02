//! Opaque payload domains are Type identities, not permission to unfold layouts.
use nir::{DeclarationId, Primitive as P, Sort, Term};
use ptc::{Budget, Context, Formula as F, Outcome, TypeInputOutcome as Search};

fn id(local: u32) -> DeclarationId {
    DeclarationId {
        package: 986,
        local,
    }
}
fn input(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn application(head: u32, arguments: Vec<Term>) -> Term {
    Term::NormalForm {
        head: id(head),
        sort: Sort::Type,
        arguments,
    }
}
fn packet(domain: Term, field: Term) -> Term {
    Term::NominalValue {
        ty: Box::new(domain),
        variant: None,
        fields: vec![field],
    }
}
fn context() -> Context {
    let mut context = Context::default();
    context.admit_congruence(id(10));
    context.admit_congruence(id(11));
    context.admit_nominal_constructor(id(30));
    context
}

#[test]
fn opaque_payload_domains_preserve_source_types_and_replay_models() {
    let context = context();
    let domain = application(11, vec![input(9)]);
    let expected = application(10, vec![packet(domain, input(9))]);
    let constraint = F::equal(input(0), expected.clone());
    let outcome = context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
    let Search::Unique(solution) = outcome else {
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
    for change in 0..4 {
        let mut forged = solution.model.clone();
        let argument = &mut forged
            .applications
            .iter_mut()
            .find(|entry| entry.head == id(10))
            .unwrap()
            .arguments[0];
        let Term::NominalValue {
            ty,
            variant,
            fields,
        } = argument
        else {
            panic!()
        };
        match change {
            0 => *ty = Box::new(Term::Bool(true)),
            1 => *variant = Some(2),
            2 => fields.push(Term::Bool(false)),
            _ => *ty = Box::new(Term::Pointer(Box::new(Term::Primitive(P::Char)))),
        }
        assert!(problem.verify_witness(&forged, Budget::default()).is_err());
    }
    // Model validation is independent of source value formation, but callable
    // shapes, binder closure and error labels still need complete validation.
    for invalid in [
        Term::ComptimeFunction(Box::new(Term::Primitive(P::Bool))),
        Term::UnsafeFunction(Box::new(Term::Primitive(P::Bool))),
        Term::NormalFormFunction(
            Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(P::Bool)),
            }),
            Some(0),
        ),
        Term::NormalFormFunction(
            Box::new(Term::Function {
                parameters: vec![nir::Parameter {
                    sort: Sort::Runtime,
                    ty: Term::Primitive(P::Bool),
                }],
                result: Box::new(Term::Primitive(P::Bool)),
            }),
            None,
        ),
        Term::Closure {
            environment: vec![Term::Primitive(P::Bool)],
            signature: Box::new(Term::Primitive(P::Bool)),
        },
        Term::Nominal {
            declaration: id(30),
            arguments: vec![Term::ErrorValue {
                ty: Box::new(Term::error_result(
                    vec![Term::Primitive(P::Bool)],
                    Term::Primitive(P::Bool),
                )),
                label: Some(Box::new(Term::Primitive(P::Char))),
                value: Box::new(Term::Bool(true)),
            }],
        },
        Term::Pointer(Box::new(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        })),
    ] {
        let mut forged = solution.model.clone();
        forged.types.insert(id(99), invalid);
        assert!(problem.verify_witness(&forged, Budget::default()).is_err());
    }
    assert!(context.assumptions().is_empty());
}

#[test]
fn payload_domain_equalities_lift_forward_without_opaque_injectivity() {
    let left = application(11, vec![input(1)]);
    let right = application(11, vec![input(2)]);
    let outer = |domain| application(10, vec![packet(domain, Term::Bool(true))]);
    let mut forward = context();
    forward.assume(F::equal(left.clone(), right.clone()));
    let outcome = forward.prove(
        F::equal(outer(left.clone()), outer(right.clone())),
        Budget::default(),
    );
    let Outcome::Entailed(proof) = outcome else {
        panic!("{outcome}")
    };
    forward.verify_proof(&proof, Budget::default()).unwrap();

    let mut inverse = context();
    inverse.assume(F::equal(outer(left.clone()), outer(right.clone())));
    let outcome = inverse.prove(F::equal(left, right), Budget::default());
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    inverse.verify_witness(&witness, Budget::default()).unwrap();
}

#[test]
fn opaque_payload_models_keep_authority_and_budget_boundaries() {
    let domain = application(11, vec![Term::Primitive(P::Type)]);
    let expected = application(10, vec![packet(domain.clone(), Term::Primitive(P::Bool))]);
    let constraint = F::equal(input(0), expected);
    let context = context();
    assert!(matches!(
        context.search_type_inputs(
            &[id(0)],
            constraint.clone(),
            &mut Budget {
                steps: 0,
                ..Budget::default()
            }
        ),
        Search::Unknown(ptc::UnknownReason::BudgetExhausted)
    ));
    assert!(matches!(
        context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default()),
        Search::Unique(_)
    ));
    let mut missing = Context::default();
    missing.admit_congruence(id(10));
    assert!(matches!(
        missing.search_type_inputs(&[id(0)], constraint, &mut Budget::default()),
        Search::Unknown(_)
    ));
    for field in [Term::Float64(0), Term::Value { store: 1, index: 0 }] {
        let constraint = F::equal(
            input(0),
            application(10, vec![packet(domain.clone(), field)]),
        );
        assert!(matches!(
            context.search_type_inputs(&[id(0)], constraint, &mut Budget::default()),
            Search::Unknown(_)
        ));
    }
    let value = packet(domain, Term::Bool(true));
    assert!(matches!(
        context.prove(F::equal(value.clone(), value), Budget::default()),
        Outcome::IllFormed(_)
    ));
    let malformed_source =
        application(10, vec![packet(Term::Primitive(P::Bool), Term::Bool(true))]);
    assert!(matches!(
        context.prove(F::equal(input(0), malformed_source), Budget::default()),
        Outcome::IllFormed(_)
    ));
}

#[test]
fn opaque_payload_domains_preserve_lexical_type_bindings() {
    let mut context = Context::with_scope(vec![vec![Sort::Type]]);
    context.admit_congruence(id(10));
    context.admit_congruence(id(11));
    let lexical = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let expected = application(
        10,
        vec![packet(application(11, vec![lexical.clone()]), lexical)],
    );
    let constraint = F::equal(input(0), expected.clone());
    let outcome = context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
    let Search::Unique(solution) = outcome else {
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
    let mut forged = solution.model;
    forged.bound_types.clear();
    assert!(problem.verify_witness(&forged, Budget::default()).is_err());
}
