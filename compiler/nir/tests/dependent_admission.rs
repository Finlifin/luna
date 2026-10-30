use nir::{DeclarationId, DeclarationParameter, NominalAdmissionError, Primitive, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 99, local }
}
fn rigid(local: u32, sort: Sort) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort,
    }
}
fn p(primitive: Primitive) -> Term {
    Term::Primitive(primitive)
}
fn admit<'a>(
    ids: &[DeclarationId],
    domains: &[Term],
    arguments: &'a [Term],
    fuel: &mut usize,
) -> Result<nir::AdmittedArguments<'a>, NominalAdmissionError> {
    nir::admit_declaration_arguments_with_evidence_in(
        ids,
        domains,
        arguments,
        fuel,
        |_, _, _| Err("unexpected layout request".into()),
        |_, _, _| false,
        None,
    )
}

#[test]
fn completed_domains_authorize_type_values_without_weakening_fixed_sort_substitution() {
    let arguments = [p(Primitive::Type), p(Primitive::Bool)];
    let domains = [p(Primitive::Type), rigid(1, Sort::Type)];
    let parameters = [
        DeclarationParameter {
            declaration: id(1),
            sort: Sort::Type,
        },
        DeclarationParameter {
            declaration: id(2),
            sort: Sort::ComptimeValue,
        },
    ];
    let body = Term::NormalForm {
        head: id(3),
        sort: Sort::Type,
        arguments: vec![rigid(1, Sort::Type), rigid(2, Sort::ComptimeValue)],
    };
    assert!(
        body.instantiate_parameters(&parameters, &arguments)
            .is_err()
    );
    let admitted = admit(&[id(1), id(2)], &domains, &arguments, &mut 10000).unwrap();
    assert_eq!(
        admitted.instantiate(&body, &mut 10000).unwrap(),
        Term::NormalForm {
            head: id(3),
            sort: Sort::Type,
            arguments: arguments.to_vec(),
        }
    );
    assert!(
        admitted
            .to_string()
            .starts_with("(admitted-arguments (input ")
    );
    // The original occurrence sort remains part of the definition contract.
    assert!(
        admitted
            .instantiate(&rigid(2, Sort::Type), &mut 10000)
            .is_err()
    );
    let predicate = nir::program::TypePredicate::Equal(body.clone(), body.clone());
    let expected = admitted.instantiate(&body, &mut 10000).unwrap();
    assert_eq!(
        admitted
            .instantiate_predicate(&predicate, &mut 10000)
            .unwrap(),
        nir::program::TypePredicate::Equal(expected.clone(), expected)
    );
    assert!(admitted.instantiate_predicate(&predicate, &mut 0).is_err());
    assert!(admitted.instantiate(&body, &mut 0).is_err());
}

#[test]
fn captured_domains_are_resolved_before_dependent_value_inputs() {
    let domains = [rigid(1, Sort::Type), p(Primitive::Type)];
    let arguments = [p(Primitive::Bool), p(Primitive::Type)];
    let admitted = admit(&[id(2), id(1)], &domains, &arguments, &mut 10000).unwrap();
    assert_eq!(
        admitted
            .instantiate(&rigid(2, Sort::ComptimeValue), &mut 10000)
            .unwrap(),
        p(Primitive::Bool)
    );
}

#[test]
fn actual_domains_reject_mismatches_and_exhaustion_is_not_rejection() {
    let domains = [p(Primitive::Type), rigid(1, Sort::Type)];
    for arguments in [
        [p(Primitive::Bool), p(Primitive::Bool)],
        [p(Primitive::Type), Term::Bool(true)],
        [
            p(Primitive::Bool),
            Term::Integer(comptime_runtime::Integer::from_u128(1)),
        ],
    ] {
        assert!(matches!(
            admit(&[id(1), id(2)], &domains, &arguments, &mut 10000),
            Err(NominalAdmissionError::Invalid(_))
        ));
    }
    assert!(matches!(
        admit(
            &[id(1)],
            &[p(Primitive::Type)],
            &[p(Primitive::Bool)],
            &mut 0
        ),
        Err(NominalAdmissionError::Budget)
    ));
    let cycle = [2, 1].map(|input| Term::Nominal {
        declaration: id(9),
        arguments: vec![rigid(input, Sort::ComptimeValue)],
    });
    assert!(
        admit(
            &[id(1), id(2)],
            &cycle,
            &[Term::Bool(true), Term::Bool(false)],
            &mut 10000
        )
        .is_err()
    );
}

#[test]
fn structural_substitution_checks_completed_sorts_and_preserves_caller_scope() {
    let ids = [id(1), id(2)];
    let domains = [p(Primitive::Type), rigid(1, Sort::Type)];
    let frames = [vec![Sort::Type]];
    let arguments = [
        p(Primitive::Type),
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
    ];
    let substitution =
        nir::DeclarationSubstitution::bounded(&ids, &domains, &arguments, &frames, &mut 10000)
            .unwrap();
    let body = Term::NormalForm {
        head: id(3),
        sort: Sort::Type,
        arguments: vec![rigid(2, Sort::ComptimeValue)],
    };
    assert_eq!(
        substitution.instantiate_bounded(&body, &mut 10000).unwrap(),
        Term::NormalForm {
            head: id(3),
            sort: Sort::Type,
            arguments: vec![arguments[1].clone()]
        }
    );
    let nested = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Type,
            ty: p(Primitive::Type),
        }],
        result: Box::new(body.clone()),
    };
    let expected_nested = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Type,
            ty: p(Primitive::Type),
        }],
        result: Box::new(Term::NormalForm {
            head: id(3),
            sort: Sort::Type,
            arguments: vec![Term::Bound {
                depth: 1,
                index: 0,
                sort: Sort::Type,
            }],
        }),
    };
    assert_eq!(substitution.instantiate(&nested).unwrap(), expected_nested);
    assert!(substitution.to_string().contains("(frames (frame type))"));
    assert!(nir::DeclarationSubstitution::new(&ids, &domains, &arguments, &[]).is_err());
    assert!(
        nir::DeclarationSubstitution::bounded(&ids, &domains, &arguments, &frames, &mut 0).is_err()
    );
    for arguments in [
        [p(Primitive::Type), Term::Bool(true)],
        [p(Primitive::Bool), p(Primitive::Bool)],
    ] {
        assert!(nir::DeclarationSubstitution::new(&ids, &domains, &arguments, &[]).is_err());
    }
    // An opaque domain does not become Type by looking at the supplied value.
    let opaque = [p(Primitive::Type), body];
    assert!(
        nir::DeclarationSubstitution::new(
            &ids,
            &opaque,
            &[p(Primitive::Type), p(Primitive::Bool)],
            &[]
        )
        .is_err()
    );
}
