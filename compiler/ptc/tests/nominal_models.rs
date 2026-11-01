use nir::{DeclarationId, Primitive, Sort, Term};
use ptc::{Budget, Context, Formula as F, Outcome, TypeInputOutcome};

fn id(local: u32) -> DeclarationId {
    DeclarationId {
        package: 982,
        local,
    }
}
fn input(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn nominal(value: Term) -> Term {
    Term::Nominal {
        declaration: id(10),
        arguments: vec![value],
    }
}

#[test]
fn nominal_type_models_accept_completed_value_parameters() {
    let mut context = Context::default();
    context.admit_nominal_constructor(id(10));
    for value in [
        Term::Bool(true),
        Term::Char('x'),
        Term::TupleValue(vec![Term::Bool(false), Term::Primitive(Primitive::Bool)]),
        Term::RecordValue(vec![("flag".into(), Term::Bool(true))]),
    ] {
        let expected = nominal(value);
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
        let mut corrupted = solution.model;
        corrupted.types.insert(id(0), nominal(Term::Bool(false)));
        assert!(
            problem
                .verify_witness(&corrupted, Budget::default())
                .is_err()
        );
    }
}

#[test]
fn nominal_models_preserve_ambient_types_inside_value_arguments() {
    let mut context = Context::default();
    context.admit_nominal_constructor(id(10));
    let expected = nominal(Term::TupleValue(vec![input(9), Term::Bool(true)]));
    let constraint = F::equal(input(0), expected.clone());
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
    assert!(context.assumptions().is_empty());
}

#[test]
fn nominal_model_admission_keeps_unknown_domains_and_head_authority() {
    let goal = |argument| F::equal(input(0), nominal(argument));
    assert!(matches!(
        Context::default().prove(goal(Term::Bool(true)), Budget::default()),
        Outcome::Unknown { .. }
    ));
    let mut context = Context::default();
    context.admit_nominal_constructor(id(10));
    let outcome = context.prove(
        F::equal(nominal(Term::Bool(true)), nominal(Term::Bool(false))),
        Budget::default(),
    );
    let Outcome::NotEntailed(witness) = outcome else {
        panic!("{outcome}")
    };
    context.verify_witness(&witness, Budget::default()).unwrap();
    for value in [
        Term::Float64(0),
        Term::Value { store: 1, index: 0 },
        Term::TupleValue(vec![Term::Float64(0)]),
    ] {
        assert!(matches!(
            context.prove(goal(value), Budget::default()),
            Outcome::Unknown { .. }
        ));
    }
    let constraint = F::equal(input(0), nominal(Term::Bool(true)));
    assert!(matches!(
        context.search_type_inputs(
            &[id(0)],
            constraint.clone(),
            &mut Budget {
                steps: 0,
                ..Budget::default()
            }
        ),
        TypeInputOutcome::Unknown(ptc::UnknownReason::BudgetExhausted)
    ));
    assert!(matches!(
        context.search_type_inputs(&[id(0)], constraint, &mut Budget::default()),
        TypeInputOutcome::Unique(_)
    ));
}

fn payload(variant: Option<u32>, field: Term) -> Term {
    Term::NominalValue {
        ty: Box::new(Term::Nominal {
            declaration: id(11),
            arguments: vec![Term::Primitive(Primitive::Bool)],
        }),
        variant,
        fields: vec![field],
    }
}

#[test]
fn nominal_payload_models_preserve_identity_variant_and_type_fields() {
    let mut context = Context::default();
    context.admit_nominal_constructor(id(10));
    context.admit_nominal_constructor(id(11));
    context.admit_congruence(id(12));
    for variant in [None, Some(0), Some(1)] {
        let expected = Term::NormalForm {
            head: id(12),
            sort: Sort::Type,
            arguments: vec![payload(variant, input(9))],
        };
        let constraint = F::equal(input(0), expected.clone());
        let TypeInputOutcome::Unique(solution) =
            context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default())
        else {
            panic!("nominal payload should retain its ambient Type")
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
        let entry = forged
            .applications
            .iter_mut()
            .find(|entry| entry.head == id(12))
            .unwrap();
        let Term::NominalValue { variant, .. } = &mut entry.arguments[0] else {
            panic!()
        };
        *variant = Some(27);
        assert!(problem.verify_witness(&forged, Budget::default()).is_err());
    }
    for field in [Term::Float64(0), Term::Value { store: 1, index: 0 }] {
        assert!(matches!(
            context.prove(
                F::equal(input(0), nominal(payload(None, field))),
                Budget::default()
            ),
            Outcome::Unknown { .. }
        ));
    }
    let mut unregistered = Context::default();
    unregistered.admit_nominal_constructor(id(10));
    assert!(matches!(
        unregistered.prove(
            F::equal(input(0), nominal(payload(None, Term::Bool(true)))),
            Budget::default()
        ),
        Outcome::Unknown { .. }
    ));
}
