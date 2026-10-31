//! Frozen callable inputs require signature evidence from a declaration world.
use nir::normal_forms::{ApplicationError, Head, Kind, Parameter, Role, supported_input_type};
use nir::program::LocalId;
use nir::{DeclarationId, DeclarationSubstitution, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 7, local }
}
fn p(value: P) -> Term {
    Term::Primitive(value)
}
fn function(domain: Term) -> Term {
    Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Runtime,
            ty: domain.clone(),
        }],
        result: Box::new(domain),
    }
}
fn parameter(local: u32, kind: Kind, ty: Term) -> Parameter {
    Parameter {
        declaration: id(local),
        binding: LocalId {
            owner: 1,
            index: local,
        },
        kind,
        ty,
    }
}
fn head(domain: Term) -> Head {
    Head {
        declaration: id(1),
        collector: None,
        role: Role::Opaque,
        parameters: vec![parameter(2, Kind::Value, domain)],
        requirements: vec![],
    }
}
fn identity(arguments: Vec<Term>) -> Term {
    Term::FunctionValue {
        declaration: id(10),
        arguments,
    }
}
fn check(
    descriptor: &Head,
    inputs: &[Term],
    signature: &Term,
    frames: &[Vec<Sort>],
    fuel: &mut usize,
) -> Result<(), ApplicationError> {
    descriptor.check_arguments_in_scope_with_callables_budgeted_in(
        inputs,
        |_| false,
        |_| false,
        |_, _, _| None,
        |_, _| false,
        |declaration, arguments, caller_frames, fuel| {
            // This test registry contains one ordinary generic declaration.
            *fuel = fuel.checked_sub(1)?;
            if declaration != id(10) {
                return None;
            }
            let substitution = DeclarationSubstitution::bounded(
                &[id(11)],
                &[p(P::Type)],
                arguments,
                caller_frames,
                fuel,
            )
            .ok()?;
            substitution.instantiate_bounded(signature, fuel).ok()
        },
        frames,
        fuel,
        None,
    )
}
fn generic() -> Term {
    function(Term::Rigid {
        declaration: id(11),
        sort: Sort::Type,
    })
}

#[test]
fn declaration_signature_controls_domain_phase_and_safety() {
    let runtime = function(p(P::Bool));
    let comptime = Term::ComptimeFunction(Box::new(runtime.clone()));
    let unsafe_runtime = Term::UnsafeFunction(Box::new(runtime.clone()));
    let signatures = [runtime, comptime, unsafe_runtime];
    for (expected_index, expected) in signatures.iter().enumerate() {
        assert!(supported_input_type(expected, &[]));
        for (actual_index, actual) in signatures.iter().enumerate() {
            assert_eq!(
                check(
                    &head(expected.clone()),
                    &[identity(vec![p(P::Bool)])],
                    actual,
                    &[],
                    &mut 10000
                )
                .is_ok(),
                expected_index == actual_index,
            );
        }
    }
    let descriptor = head(function(p(P::Bool)));
    assert!(
        check(
            &descriptor,
            &[identity(vec![p(P::Char)])],
            &generic(),
            &[],
            &mut 10000
        )
        .is_err()
    );
    assert!(
        check(
            &descriptor,
            &[Term::FunctionValue {
                declaration: id(99),
                arguments: vec![p(P::Bool)]
            }],
            &generic(),
            &[],
            &mut 10000
        )
        .is_err()
    );
    for arguments in [vec![], vec![Term::Bool(true)], vec![p(P::Bool), p(P::Bool)]] {
        assert!(
            check(
                &descriptor,
                &[identity(arguments)],
                &generic(),
                &[],
                &mut 10000
            )
            .is_err()
        );
    }
    // Old callers supply no declaration authority and must continue to reject.
    assert!(
        descriptor
            .check_arguments_in_scope_with_collections_budgeted_in(
                &[identity(vec![p(P::Bool)])],
                |_| false,
                |_| false,
                |_, _, _| None,
                |_, _| true,
                &[],
                &mut 10000,
                None,
            )
            .is_err()
    );
}

#[test]
fn callable_domains_allow_runtime_payloads_but_reject_free_or_malformed_types() {
    assert!(supported_input_type(
        &function(Term::Pointer(Box::new(p(P::Bool)))),
        &[]
    ));
    assert!(supported_input_type(&generic(), &[id(11)]));
    assert!(!supported_input_type(&generic(), &[]));
    assert!(!supported_input_type(
        &Term::ComptimeFunction(Box::new(p(P::Bool))),
        &[]
    ));
    let escaping = Term::Bound {
        depth: 1,
        index: 0,
        sort: Sort::Type,
    };
    assert!(!supported_input_type(&function(escaping.clone()), &[]));
    assert!(
        check(
            &head(function(p(P::Bool))),
            &[identity(vec![escaping])],
            &generic(),
            &[],
            &mut 10000
        )
        .is_err()
    );
}

#[test]
fn dependent_inputs_restore_the_caller_frame_inside_function_binders() {
    let mut descriptor = head(function(Term::Rigid {
        declaration: id(2),
        sort: Sort::Type,
    }));
    descriptor.parameters = vec![
        parameter(2, Kind::Type, p(P::Type)),
        parameter(3, Kind::Value, descriptor.parameters[0].ty.clone()),
    ];
    let caller_type = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let inputs = [caller_type.clone(), identity(vec![caller_type])];
    assert!(
        check(
            &descriptor,
            &inputs,
            &generic(),
            &[vec![Sort::Type]],
            &mut 10000
        )
        .is_ok()
    );
    assert!(check(&descriptor, &inputs, &generic(), &[], &mut 10000).is_err());
    assert!(
        check(
            &descriptor,
            &[p(P::Bool), identity(vec![p(P::Char)])],
            &generic(),
            &[],
            &mut 10000
        )
        .is_err()
    );
}

#[test]
fn nested_callable_values_share_budget_and_failure_does_not_poison_recovery() {
    let signature = function(p(P::Bool));
    let descriptor = head(Term::Record(vec![(
        "callback".into(),
        Term::Optional(Box::new(signature.clone())),
    )]));
    let input = Term::RecordValue(vec![(
        "callback".into(),
        Term::OptionalValue {
            ty: Box::new(Term::Optional(Box::new(signature))),
            value: Some(Box::new(identity(vec![p(P::Bool)]))),
        },
    )]);
    let mut fuel = 10000;
    check(&descriptor, &[input.clone()], &generic(), &[], &mut fuel).unwrap();
    let used = 10000 - fuel;
    assert!(used > 10);
    assert!(
        check(
            &descriptor,
            &[input.clone()],
            &generic(),
            &[],
            &mut (used - 1)
        )
        .is_err()
    );
    let mut exact = used;
    check(&descriptor, &[input.clone()], &generic(), &[], &mut exact).unwrap();
    assert_eq!(exact, 0);
    assert!(check(&descriptor, &[input], &generic(), &[], &mut 10000).is_ok());
}

#[test]
fn normal_form_function_collector_is_part_of_signature_evidence() {
    let signature = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Type,
            ty: p(P::Type),
        }],
        result: Box::new(p(P::Type)),
    };
    let collected = Term::NormalFormFunction(Box::new(signature.clone()), Some(0));
    let ordinary = Term::NormalFormFunction(Box::new(signature), None);
    assert!(supported_input_type(&collected, &[]));
    assert!(
        check(
            &head(collected.clone()),
            &[identity(vec![p(P::Bool)])],
            &collected,
            &[],
            &mut 10000
        )
        .is_ok()
    );
    assert!(
        check(
            &head(collected),
            &[identity(vec![p(P::Bool)])],
            &ordinary,
            &[],
            &mut 10000
        )
        .is_err()
    );
}

#[test]
fn malformed_signature_evidence_is_rejected_even_when_it_matches_the_domain() {
    let malformed = Term::ComptimeFunction(Box::new(p(P::Bool)));
    assert!(
        check(
            &head(malformed.clone()),
            &[identity(vec![p(P::Bool)])],
            &malformed,
            &[],
            &mut 10000,
        )
        .is_err()
    );
}
