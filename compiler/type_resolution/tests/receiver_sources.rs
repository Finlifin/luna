//! Receiver inference discovers interface inputs without granting applicability.
use nir::{DeclarationId, Primitive, Sort, Term, program::TypePredicate};
use std::collections::BTreeMap;
use type_resolution::{
    implementation::{ImplementationHead, TypeAssessment},
    ptc::{Budget, Context},
};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn ty(value: Primitive) -> Term {
    Term::Primitive(value)
}
fn input(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn head(subject: Term, parameters: &[u32], arguments: Vec<Term>) -> ImplementationHead {
    ImplementationHead {
        declaration: id(100),
        interface: Some(id(200)),
        subject,
        type_parameters: parameters.iter().map(|n| id(*n)).collect(),
        parameter_types: vec![],
        requirements: vec![],
        interface_arguments: arguments,
    }
}
fn discover(
    head: &ImplementationHead,
    subject: &Term,
    context: &Context,
) -> Result<nir::program::InterfaceInstance, TypeAssessment> {
    head.infer_interface_from_subject(subject, context, Budget::default(), &BTreeMap::new(), None)
}

#[test]
fn optional_receiver_recovers_interface_arguments_without_query_inputs() {
    let head = head(Term::Optional(Box::new(input(1))), &[1], vec![input(1)]);
    for primitive in [Primitive::Bool, Primitive::Char] {
        let subject = Term::Optional(Box::new(ty(primitive)));
        let found = discover(&head, &subject, &Context::default()).unwrap();
        assert_eq!(found.declaration, id(200));
        assert_eq!(found.arguments, [ty(primitive)]);
        assert!(matches!(
            head.assess_types(
                &subject,
                &found.arguments,
                &Context::default(),
                Budget::default()
            ),
            TypeAssessment::Applicable(_)
        ));
    }
}

#[test]
fn interface_outputs_can_reorder_inputs_and_build_ordinary_values() {
    let record = |a, b| Term::Record(vec![("first".into(), a), ("second".into(), b)]);
    let head = head(
        record(input(1), input(2)),
        &[1, 2],
        vec![
            input(2),
            Term::RecordValue(vec![
                ("element".into(), input(1)),
                ("enabled".into(), Term::Bool(true)),
            ]),
        ],
    );
    let found = discover(
        &head,
        &record(ty(Primitive::Bool), ty(Primitive::Char)),
        &Context::default(),
    )
    .unwrap();
    assert_eq!(
        found.arguments,
        [
            ty(Primitive::Char),
            Term::RecordValue(vec![
                ("element".into(), ty(Primitive::Bool)),
                ("enabled".into(), Term::Bool(true)),
            ])
        ]
    );
    assert!(found.to_string().starts_with("(interface-instance"));
}

#[test]
fn a_discovered_interface_is_not_evidence_of_its_implementation_requirements() {
    let mut head = head(Term::Optional(Box::new(input(1))), &[1], vec![input(1)]);
    head.requirements
        .push(TypePredicate::Equal(input(1), ty(Primitive::Bool)));
    let subject = Term::Optional(Box::new(ty(Primitive::Char)));
    let found = discover(&head, &subject, &Context::default()).unwrap();
    assert_eq!(found.arguments, [ty(Primitive::Char)]);
    assert!(matches!(
        head.assess_types(
            &subject,
            &found.arguments,
            &Context::default(),
            Budget::default()
        ),
        TypeAssessment::Excluded(_)
    ));
}

#[test]
fn parameters_absent_from_receiver_are_not_guessed_from_interface_outputs() {
    let head = head(ty(Primitive::Bool), &[1], vec![input(1)]);
    assert!(matches!(
        discover(&head, &ty(Primitive::Bool), &Context::default()),
        Err(TypeAssessment::Pending(_))
    ));
}

#[test]
fn opaque_factory_arguments_do_not_become_injective_during_source_discovery() {
    let opaque = |value| Term::NormalForm {
        sort: Sort::Type,
        head: id(300),
        arguments: vec![value],
    };
    let head = head(opaque(input(1)), &[1], vec![input(1)]);
    assert!(matches!(
        discover(&head, &opaque(ty(Primitive::Bool)), &Context::default()),
        Err(TypeAssessment::Pending(_))
    ));
}

#[test]
fn discovery_failure_does_not_publish_partial_inputs_or_poison_later_queries() {
    let head = head(Term::Optional(Box::new(input(1))), &[1], vec![input(1)]);
    let subject = Term::Optional(Box::new(ty(Primitive::Bool)));
    assert!(matches!(
        head.infer_interface_from_subject(
            &subject,
            &Context::default(),
            Budget {
                steps: 0,
                ..Budget::default()
            },
            &BTreeMap::new(),
            None
        ),
        Err(TypeAssessment::Pending(_))
    ));
    assert_eq!(
        discover(&head, &subject, &Context::default())
            .unwrap()
            .arguments,
        [ty(Primitive::Bool)]
    );
    let mut open = head.clone();
    open.interface = None;
    assert!(matches!(
        discover(&open, &subject, &Context::default()),
        Err(TypeAssessment::Pending(_))
    ));
    let other = Term::Optional(Box::new(ty(Primitive::Char)));
    assert_eq!(
        discover(&head, &other, &Context::default())
            .unwrap()
            .arguments,
        [ty(Primitive::Char)]
    );
}

#[test]
fn nominal_receiver_can_supply_ordinary_scalar_interface_inputs() {
    let value_input = Term::Rigid {
        declaration: id(1),
        sort: Sort::ComptimeValue,
    };
    let nominal = |value| Term::Nominal {
        declaration: id(400),
        arguments: vec![value],
    };
    let mut head = head(nominal(value_input.clone()), &[1], vec![value_input]);
    head.parameter_types = vec![ty(Primitive::Bool)];
    let mut context = Context::default();
    context.admit_nominal_constructor(id(400));
    for value in [true, false] {
        let subject = nominal(Term::Bool(value));
        let found = discover(&head, &subject, &context).unwrap();
        assert_eq!(found.arguments, [Term::Bool(value)]);
        assert!(matches!(
            head.assess_types(&subject, &found.arguments, &context, Budget::default()),
            TypeAssessment::Applicable(_)
        ));
    }
}

#[test]
fn discovery_keeps_distinct_interfaces_and_cannot_choose_between_them() {
    let first = head(Term::Optional(Box::new(input(1))), &[1], vec![input(1)]);
    let mut second = first.clone();
    second.interface = Some(id(201));
    second.declaration = id(101);
    let subject = Term::Optional(Box::new(ty(Primitive::Bool)));
    let context = Context::default();
    let a = discover(&first, &subject, &context).unwrap();
    let b = discover(&second, &subject, &context).unwrap();
    assert_ne!(a.declaration, b.declaration);
    assert_eq!(a.arguments, b.arguments);
    assert_eq!(first.interface_arguments, [input(1)]);
    assert_eq!(second.interface_arguments, [input(1)]);
}

#[test]
fn malformed_interface_output_cannot_escape_receiver_discovery() {
    let mut head = head(
        ty(Primitive::Bool),
        &[],
        vec![Term::Hole {
            index: 0,
            sort: Sort::Type,
        }],
    );
    assert!(matches!(
        discover(&head, &ty(Primitive::Bool), &Context::default()),
        Err(TypeAssessment::Invalid(_))
    ));
    head.interface_arguments = vec![ty(Primitive::Bool)];
    assert_eq!(
        discover(&head, &ty(Primitive::Bool), &Context::default())
            .unwrap()
            .arguments,
        [ty(Primitive::Bool)]
    );
}

#[test]
fn metered_discovery_returns_unused_fuel_and_charges_failed_queries() {
    let candidate = head(Term::Optional(Box::new(input(1))), &[1], vec![input(1)]);
    let subject = Term::Optional(Box::new(ty(Primitive::Bool)));
    let mut budget = Budget {
        steps: 10_000,
        ..Budget::default()
    };
    let found = candidate
        .infer_interface_from_subject_metered(
            &subject,
            &Context::default(),
            &mut budget,
            &BTreeMap::new(),
            None,
        )
        .unwrap();
    assert_eq!(found.arguments, [ty(Primitive::Bool)]);
    assert!(budget.steps > 0 && budget.steps < 10_000);
    let mut empty = Budget {
        steps: 0,
        ..Budget::default()
    };
    assert!(matches!(
        candidate.infer_interface_from_subject_metered(
            &subject,
            &Context::default(),
            &mut empty,
            &BTreeMap::new(),
            None,
        ),
        Err(TypeAssessment::Pending(_))
    ));
    assert_eq!(empty.steps, 0);
    let unresolved = head(Term::Optional(Box::new(input(1))), &[1, 2], vec![input(2)]);
    let before = budget.steps;
    assert!(matches!(
        unresolved.infer_interface_from_subject_metered(
            &subject,
            &Context::default(),
            &mut budget,
            &BTreeMap::new(),
            None,
        ),
        Err(TypeAssessment::Pending(_))
    ));
    assert!(budget.steps < before);
}
