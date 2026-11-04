//! One domain-membership service for ordinary, nominal and opaque inputs.
use nir::value_admission::{self, DomainQueries, Error};
use nir::{DeclarationId, Primitive as P, Sort, Term};
use std::borrow::Cow;

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 19, local }
}
fn p(value: P) -> Term {
    Term::Primitive(value)
}
fn list(element: Term) -> Term {
    Term::NormalForm {
        head: id(10),
        sort: Sort::Type,
        arguments: vec![element],
    }
}
fn signature() -> Term {
    Term::ComptimeFunction(Box::new(Term::Function {
        parameters: vec![],
        result: Box::new(p(P::Bool)),
    }))
}
fn check(
    value: &Term,
    domain: &Term,
    frames: &[Vec<Sort>],
    registered: bool,
    fuel: &mut usize,
) -> Result<(), Error> {
    value_admission::validate(
        value,
        domain,
        frames,
        fuel,
        &mut DomainQueries {
            layout: &mut |_, _, _| Err("missing layout evidence".into()),
            declared: &mut |value, domain, _| {
                registered
                    && *domain == p(P::Bool)
                    && matches!(
                        value,
                        Term::Bound {
                            sort: Sort::ComptimeValue,
                            depth: 0,
                            index: 0
                        }
                    )
            },
            collection: &mut |domain, fuel| {
                if registered && *domain == list(signature()) {
                    nir::type_computations::charge_term(&signature(), fuel)?;
                    Ok(Some(signature()))
                } else {
                    Ok(None)
                }
            },
            callable: &mut |value, _, _| {
                Ok(match value {
                    nir::CallableValueRef::Function {
                        declaration,
                        arguments,
                    } if registered && declaration == id(20) && arguments.is_empty() => {
                        Some(Cow::Owned(signature()))
                    }
                    _ => None,
                })
            },
            target: None,
        },
    )
}

#[test]
fn nested_collection_callables_require_receiving_world_evidence() {
    let domain = list(signature());
    let callable = Term::FunctionValue {
        declaration: id(20),
        arguments: vec![],
    };
    let value = Term::NominalValue {
        ty: Box::new(domain.clone()),
        variant: None,
        fields: vec![callable],
    };
    check(&value, &domain, &[], true, &mut 10000).unwrap();
    assert!(matches!(
        check(&value, &domain, &[], false, &mut 10000),
        Err(Error::Invalid(_))
    ));
    let mut wrong = value.clone();
    if let Term::NominalValue { fields, .. } = &mut wrong {
        fields[0] = Term::FunctionValue {
            declaration: id(21),
            arguments: vec![],
        };
    }
    assert!(matches!(
        check(&wrong, &domain, &[], true, &mut 10000),
        Err(Error::Invalid(_))
    ));
    // A matching payload never repairs a foreign collection identity.
    let other = Term::NormalForm {
        head: id(11),
        sort: Sort::Type,
        arguments: vec![signature()],
    };
    assert!(check(&value, &other, &[], true, &mut 10000).is_err());
    check(&value, &domain, &[], true, &mut 10000).unwrap();
    let mut succeeded = false;
    for allowance in 0..1000 {
        match check(&value, &domain, &[], true, &mut { allowance }) {
            Ok(()) => {
                succeeded = true;
                break;
            }
            Err(Error::Budget) => {}
            Err(error) => panic!("valid value became a rejection at budget {allowance}: {error}"),
        }
    }
    assert!(succeeded);
}

#[test]
fn scoped_values_need_both_a_frame_and_independent_domain_evidence() {
    let value = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::ComptimeValue,
    };
    let frames = [vec![Sort::ComptimeValue]];
    check(&value, &p(P::Bool), &frames, true, &mut 1000).unwrap();
    assert!(check(&value, &p(P::Bool), &[], true, &mut 1000).is_err());
    assert!(check(&value, &p(P::Bool), &frames, false, &mut 1000).is_err());
    assert!(check(&value, &p(P::Type), &frames, true, &mut 1000).is_err());
    let malformed = Term::ClosureContextValue(vec![Term::Bool(true)]);
    assert!(check(&malformed, &Term::ClosureContextType, &[], true, &mut 1000).is_err());
}

#[test]
fn records_keep_named_domains_across_nominal_and_nf_admission() {
    let domain = Term::Record(vec![("flag".into(), p(P::Bool)), ("byte".into(), p(P::U8))]);
    let value = Term::RecordValue(vec![
        (
            "byte".into(),
            Term::Integer(comptime_runtime::Integer::from_u128(7)),
        ),
        ("flag".into(), Term::Bool(true)),
    ]);
    let head = nir::normal_forms::Head {
        declaration: id(1),
        role: nir::normal_forms::Role::Opaque,
        collector: None,
        requirements: vec![],
        parameters: vec![nir::normal_forms::Parameter {
            declaration: id(2),
            binding: nir::program::LocalId { owner: 1, index: 0 },
            kind: nir::normal_forms::Kind::Value,
            ty: domain.clone(),
        }],
    };
    check(&value, &domain, &[], false, &mut 1000).unwrap();
    head.check_closed_arguments(&[value.clone()]).unwrap();
    nir::validate_declaration_arguments(&[id(2)], &[domain.clone()], &[value.clone()], &mut 1000)
        .unwrap();
    let mut bad = value.clone();
    if let Term::RecordValue(fields) = &mut bad {
        fields[0].1 = Term::Integer(comptime_runtime::Integer::from_u128(256));
    }
    assert!(check(&bad, &domain, &[], false, &mut 1000).is_err());
    assert!(head.check_closed_arguments(&[bad.clone()]).is_err());
    assert!(nir::validate_declaration_arguments(&[id(2)], &[domain], &[bad], &mut 1000).is_err());
    let failure = head
        .check_closed_arguments_budgeted(&[value], |_| false, |_, _, _| None, &mut 0)
        .unwrap_err();
    assert_eq!(
        failure,
        nir::normal_forms::ApplicationError::Budget { index: 0 }
    );
}
