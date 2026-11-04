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
    with_queries(registered, |queries| {
        value_admission::validate(value, domain, frames, fuel, queries)
    })
}

fn with_queries<R>(registered: bool, check: impl FnOnce(&mut DomainQueries<'_, '_>) -> R) -> R {
    check(&mut DomainQueries {
        layout: &mut |_, _, _| Err("missing layout evidence".into()),
        declared: &mut |value, domain, _| {
            Ok(registered
                && *domain == p(P::Bool)
                && matches!(
                    value,
                    Term::Bound {
                        sort: Sort::ComptimeValue,
                        depth: 0,
                        index: 0
                    }
                ))
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
    })
}

#[test]
fn normal_input_queries_preserve_unresolved_prerequisites_and_recover() {
    use nir::normal_forms::{ApplicationError, Head, Kind, Parameter};
    let descriptor = Head {
        declaration: id(30),
        role: Default::default(),
        collector: None,
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(31),
            binding: nir::program::LocalId {
                owner: 30,
                index: 0,
            },
            kind: Kind::Value,
            ty: signature(),
        }],
    };
    let input = Term::FunctionValue {
        declaration: id(20),
        arguments: vec![],
    };
    let mut fuel = 10_000;
    let result = with_queries(true, |queries| {
        let mut local = DomainQueries {
            layout: &mut *queries.layout,
            declared: &mut *queries.declared,
            collection: &mut *queries.collection,
            callable: &mut |_, _, _| Err(Error::Unresolved("callable contract pending".into())),
            target: queries.target,
        };
        descriptor.check_arguments(std::slice::from_ref(&input), &[], &mut fuel, &mut local)
    });
    assert_eq!(
        result,
        Err(ApplicationError::Unresolved {
            index: 0,
            reason: "callable contract pending".into(),
        })
    );
    assert!(fuel > 0 && fuel < 10_000);
    with_queries(true, |queries| {
        descriptor.check_arguments(&[input], &[], &mut fuel, queries)
    })
    .unwrap();
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
    with_queries(false, |queries| {
        head.check_arguments(&[value.clone()], &[], &mut 100_000, queries)
    })
    .unwrap();
    nir::validate_declaration_arguments(&[id(2)], &[domain.clone()], &[value.clone()], &mut 1000)
        .unwrap();
    let mut bad = value.clone();
    if let Term::RecordValue(fields) = &mut bad {
        fields[0].1 = Term::Integer(comptime_runtime::Integer::from_u128(256));
    }
    assert!(check(&bad, &domain, &[], false, &mut 1000).is_err());
    assert!(
        with_queries(false, |queries| head.check_arguments(
            &[bad.clone()],
            &[],
            &mut 100_000,
            queries
        ))
        .is_err()
    );
    assert!(nir::validate_declaration_arguments(&[id(2)], &[domain], &[bad], &mut 1000).is_err());
    let failure = with_queries(false, |queries| {
        head.check_arguments(&[value], &[], &mut 0, queries)
    })
    .unwrap_err();
    assert_eq!(
        failure,
        nir::normal_forms::ApplicationError::Budget { index: 0 }
    );
}

#[test]
fn nf_inputs_consume_the_same_collection_and_callable_evidence() {
    use nir::normal_forms::{ApplicationError, Head, Kind, Parameter, Role};
    let descriptor = Head {
        declaration: id(30),
        role: Role::Opaque,
        collector: None,
        requirements: vec![],
        parameters: vec![
            Parameter {
                declaration: id(31),
                binding: nir::program::LocalId {
                    owner: 30,
                    index: 0,
                },
                kind: Kind::Type,
                ty: p(P::Type),
            },
            Parameter {
                declaration: id(32),
                binding: nir::program::LocalId {
                    owner: 30,
                    index: 1,
                },
                kind: Kind::Value,
                ty: Term::Rigid {
                    declaration: id(31),
                    sort: Sort::Type,
                },
            },
        ],
    };
    let domain = list(signature());
    let value = |declaration| Term::NominalValue {
        ty: Box::new(domain.clone()),
        variant: None,
        fields: vec![Term::FunctionValue {
            declaration,
            arguments: vec![],
        }],
    };
    let run = |value, registered, fuel: &mut usize| {
        with_queries(registered, |queries| {
            descriptor.check_arguments(&[domain.clone(), value], &[], fuel, queries)
        })
    };
    run(value(id(20)), true, &mut 10000).unwrap();
    assert_eq!(
        run(value(id(20)), false, &mut 10000),
        Err(ApplicationError::InputType { index: 1 })
    );
    assert_eq!(
        run(value(id(21)), true, &mut 10000),
        Err(ApplicationError::InputType { index: 1 })
    );
    for allowance in 0..1000 {
        let mut fuel = allowance;
        match run(value(id(20)), true, &mut fuel) {
            Err(ApplicationError::Budget { .. }) => {}
            Ok(()) => break,
            other => panic!("budget exhaustion became a domain rejection: {other:?}"),
        }
    }
    run(value(id(20)), true, &mut 10000).unwrap();
}

#[test]
fn nf_query_budget_errors_do_not_need_an_exhausted_fuel_sentinel() {
    use nir::normal_forms::{ApplicationError, Head, Kind, Parameter, Role};
    let descriptor = Head {
        declaration: id(30),
        role: Role::Opaque,
        collector: None,
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(31),
            binding: nir::program::LocalId {
                owner: 30,
                index: 0,
            },
            kind: Kind::Value,
            ty: signature(),
        }],
    };
    let input = Term::FunctionValue {
        declaration: id(20),
        arguments: vec![],
    };
    let mut fuel = 10000;
    let error = with_queries(true, |queries| {
        // A host query may report its own budget exhaustion while the caller
        // still has fuel. The typed outcome, not a zero sentinel, is authority.
        let mut exhausted = |_: nir::CallableValueRef<'_>, _: &[Vec<Sort>], fuel: &mut usize| {
            assert!(*fuel > 0);
            Err(Error::Budget)
        };
        let mut local = DomainQueries {
            layout: &mut *queries.layout,
            declared: &mut *queries.declared,
            collection: &mut *queries.collection,
            callable: &mut exhausted,
            target: queries.target,
        };
        descriptor.check_arguments(std::slice::from_ref(&input), &[], &mut fuel, &mut local)
    });
    assert_eq!(error, Err(ApplicationError::Budget { index: 0 }));
    assert_eq!(fuel, 0);
    with_queries(true, |queries| {
        descriptor
            .check_arguments(&[input], &[], &mut 10000, queries)
            .unwrap();
    });
}

#[test]
fn tuple_representation_requires_its_exact_registered_collection_domain() {
    use nir::normal_forms::{Head, Kind, Parameter, Role, registered_tuple_fields};
    let list_head = Head {
        declaration: id(10),
        role: Role::TypeList,
        collector: None,
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(11),
            binding: nir::program::LocalId {
                owner: 10,
                index: 0,
            },
            kind: Kind::Type,
            ty: p(P::Type),
        }],
    };
    let tuple_head = Head {
        declaration: id(30),
        role: Role::Tuple,
        collector: Some(0),
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(31),
            binding: nir::program::LocalId {
                owner: 30,
                index: 0,
            },
            kind: Kind::Value,
            ty: list(p(P::Type)),
        }],
    };
    let ty = Term::NormalForm {
        head: id(30),
        sort: Sort::Type,
        arguments: vec![Term::NominalValue {
            ty: Box::new(list(p(P::Type))),
            variant: None,
            fields: vec![p(P::Bool)],
        }],
    };
    let check = |tuple: &Head, list: Option<&Head>| {
        registered_tuple_fields(&ty, |id| {
            if id == tuple.declaration {
                Some(tuple)
            } else {
                list.filter(|list| list.declaration == id)
            }
        })
        .map(<[Term]>::to_vec)
    };
    assert_eq!(check(&tuple_head, Some(&list_head)), Some(vec![p(P::Bool)]));
    assert_eq!(check(&tuple_head, None), None);
    for invalid in [
        Head {
            role: Role::Opaque,
            ..list_head.clone()
        },
        Head {
            declaration: id(12),
            ..list_head.clone()
        },
        Head {
            parameters: vec![],
            ..list_head.clone()
        },
    ] {
        assert_eq!(check(&tuple_head, Some(&invalid)), None);
    }
    let mut different_domain = tuple_head.clone();
    different_domain.parameters[0].ty = list(p(P::Bool));
    assert_eq!(check(&different_domain, Some(&list_head)), None);
    assert_eq!(check(&tuple_head, Some(&list_head)), Some(vec![p(P::Bool)]));
}
