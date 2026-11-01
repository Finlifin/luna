//! Qualified values are ordinary NF inputs, not standalone PTC propositions.
use nir::{DeclarationId, Primitive as P, Sort, Term};
use ptc::{Budget, Context, Formula as F, Outcome, TypeInputOutcome};

fn id(local: u32) -> DeclarationId {
    DeclarationId {
        package: 983,
        local,
    }
}
fn input(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn apply(value: Term) -> Term {
    Term::NormalForm {
        head: id(10),
        sort: Sort::Type,
        arguments: vec![value],
    }
}
fn optional(value: Option<Term>) -> Term {
    Term::OptionalValue {
        ty: Box::new(Term::Optional(Box::new(Term::Primitive(P::Type)))),
        value: value.map(Box::new),
    }
}
fn error(label: Option<Term>, value: Term) -> Term {
    Term::ErrorValue {
        ty: Box::new(Term::error_result(
            vec![Term::Primitive(P::Type)],
            Term::Primitive(P::Type),
        )),
        label: label.map(Box::new),
        value: Box::new(value),
    }
}

#[test]
fn qualified_inputs_reify_ambient_types_and_verify_the_original_equations() {
    let mut context = Context::default();
    context.admit_congruence(id(10));
    for value in [
        optional(None),
        optional(Some(input(9))),
        error(None, input(9)),
        error(Some(Term::Primitive(P::Type)), input(9)),
    ] {
        let expected = apply(value.clone());
        let constraint = F::equal(input(0), expected.clone());
        let outcome =
            context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
        let TypeInputOutcome::Unique(solution) = outcome else {
            panic!("{expected}: {outcome}")
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
        let packet = &mut forged
            .applications
            .iter_mut()
            .find(|entry| entry.head == id(10))
            .unwrap()
            .arguments[0];
        match packet {
            Term::OptionalValue { value, .. } => {
                *value = if value.is_some() {
                    None
                } else {
                    Some(Box::new(Term::Primitive(P::U8)))
                }
            }
            Term::ErrorValue { label, .. } => {
                *label = if label.is_some() {
                    None
                } else {
                    Some(Box::new(Term::Primitive(P::Type)))
                }
            }
            _ => panic!(),
        }
        assert!(problem.verify_witness(&forged, Budget::default()).is_err());
        assert!(matches!(
            context.prove(F::equal(value.clone(), value), Budget::default()),
            Outcome::IllFormed(_)
        ));
    }
}

#[test]
fn qualified_payloads_do_not_grant_opaque_inversion_or_unknown_value_identity() {
    let mut context = Context::default();
    context.admit_congruence(id(10));
    context.assume(F::equal(
        apply(optional(Some(input(1)))),
        apply(optional(Some(input(2)))),
    ));
    let outcome = context.prove(F::equal(input(1), input(2)), Budget::default());
    let Outcome::NotEntailed(model) = outcome else {
        panic!("{outcome}")
    };
    context.verify_witness(&model, Budget::default()).unwrap();
    let mut admitted = Context::default();
    admitted.admit_congruence(id(10));
    for value in [Term::Float64(0), Term::Value { store: 1, index: 0 }] {
        let domain = if matches!(value, Term::Float64(_)) {
            P::F64
        } else {
            P::Type
        };
        for packet in [
            Term::OptionalValue {
                ty: Box::new(Term::Optional(Box::new(Term::Primitive(domain)))),
                value: Some(Box::new(value.clone())),
            },
            Term::ErrorValue {
                ty: Box::new(Term::error_result(
                    vec![Term::Primitive(P::Type)],
                    Term::Primitive(domain),
                )),
                label: None,
                value: Box::new(value),
            },
        ] {
            assert!(matches!(
                admitted.search_type_inputs(
                    &[id(0)],
                    F::equal(input(0), apply(packet)),
                    &mut Budget::default()
                ),
                TypeInputOutcome::Unknown(_)
            ));
        }
    }
    assert!(matches!(
        context.search_type_inputs(
            &[id(0)],
            F::equal(input(0), apply(optional(None))),
            &mut Budget {
                steps: 0,
                ..Budget::default()
            }
        ),
        TypeInputOutcome::Unknown(ptc::UnknownReason::BudgetExhausted)
    ));
}
