use nir::{DeclarationId, DeclarationParameter, Primitive as P, Sort, Term, TypeError};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 91, local }
}
fn domain(element: Term) -> Term {
    Term::NormalForm {
        head: id(1),
        sort: Sort::Type,
        arguments: vec![element],
    }
}
fn list(ty: &Term, fields: Vec<Term>) -> Term {
    nir::comptime_values::freeze_admitted_list(ty, fields, &mut 10000).unwrap()
}
fn variable() -> Term {
    Term::Rigid {
        declaration: id(2),
        sort: Sort::ComptimeValue,
    }
}

#[test]
fn ordinary_and_nested_lists_reduce_in_order_after_substitution() {
    let bools = domain(Term::Primitive(P::Bool));
    let nested = domain(bools.clone());
    for (ty, first, middle, last) in [
        (
            bools.clone(),
            Term::Bool(true),
            Term::Bool(false),
            Term::Bool(true),
        ),
        (
            nested,
            list(&bools, vec![]),
            list(&bools, vec![Term::Bool(false)]),
            list(&bools, vec![Term::Bool(true)]),
        ),
    ] {
        let residual = Term::list_concat(
            ty.clone(),
            vec![
                list(&ty, vec![first.clone()]),
                Term::ListConcat {
                    ty: Box::new(ty.clone()),
                    parts: vec![list(&ty, vec![]), variable(), list(&ty, vec![last.clone()])],
                },
            ],
        )
        .unwrap();
        residual.check_scope(&[]).unwrap();
        assert_eq!(residual.sort(), Sort::ComptimeValue);
        assert!(
            residual
                .to_string()
                .starts_with("(list-concat (type (normal-form")
        );
        assert!(residual.to_string().contains("(parts ("));
        for fields in [vec![], vec![middle.clone(), first.clone()]] {
            let actual = residual
                .instantiate_parameters(
                    &[DeclarationParameter {
                        declaration: id(2),
                        sort: Sort::ComptimeValue,
                    }],
                    &[list(&ty, fields.clone())],
                )
                .unwrap();
            let mut expected = vec![first.clone()];
            expected.extend(fields);
            expected.push(last.clone());
            assert_eq!(actual, list(&ty, expected));
        }
    }
}

#[test]
fn concatenation_rejects_foreign_domains_variants_scalars_and_runtime_pieces() {
    let ty = domain(Term::Primitive(P::Bool));
    let other = Term::NormalForm {
        head: id(3),
        sort: Sort::Type,
        arguments: vec![Term::Primitive(P::Bool)],
    };
    for part in [
        list(&other, vec![]),
        list(&domain(Term::Primitive(P::Char)), vec![]),
        Term::TypeListValue(vec![]),
        Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant: Some(0),
            fields: vec![],
        },
        Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant: None,
            fields: vec![Term::Primitive(P::Bool)],
        },
        Term::Bool(true),
        Term::Rigid {
            declaration: id(2),
            sort: Sort::Runtime,
        },
        Term::ListConcat {
            ty: Box::new(other),
            parts: vec![],
        },
    ] {
        assert_eq!(
            Term::list_concat(ty.clone(), vec![part.clone()]),
            Err(TypeError::InvalidListComponent)
        );
        let malformed = Term::ListConcat {
            ty: Box::new(ty.clone()),
            parts: vec![part],
        };
        assert!(malformed.check_scope(&[]).is_err());
    }
    let residual = Term::list_concat(
        ty.clone(),
        vec![list(&ty, vec![Term::Bool(true)]), variable()],
    )
    .unwrap();
    assert!(
        residual
            .instantiate_parameters(
                &[DeclarationParameter {
                    declaration: id(2),
                    sort: Sort::ComptimeValue
                }],
                &[Term::Bool(false)],
            )
            .is_err()
    );
    assert!(Term::list_concat(Term::Primitive(P::Bool), vec![]).is_err());
    assert_eq!(
        Term::list_concat(ty.clone(), vec![]).unwrap(),
        list(&ty, vec![])
    );
}

#[test]
fn list_domain_participates_in_dependencies_rewriting_and_budget() {
    let ty = domain(Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    });
    let residual = Term::ListConcat {
        ty: Box::new(ty),
        parts: vec![Term::Bound {
            depth: 0,
            index: 1,
            sort: Sort::ComptimeValue,
        }],
    };
    residual
        .check_scope(&[vec![Sort::Type, Sort::ComptimeValue]])
        .unwrap();
    assert!(residual.check_scope(&[]).is_err());
    assert_eq!(
        residual.substitution_parameters().unwrap(),
        std::collections::BTreeMap::from([(0, Sort::Type), (1, Sort::ComptimeValue)])
    );
    assert!(nir::type_computations::charge_term(&residual, &mut 2).is_err());
    nir::type_computations::charge_term(&residual, &mut 1000).unwrap();
    let bools = domain(Term::Primitive(P::Bool));
    assert_eq!(
        residual
            .substitute_partial(&[
                Some(Term::Primitive(P::Bool)),
                Some(list(&bools, vec![Term::Bool(true)])),
            ])
            .unwrap(),
        list(&bools, vec![Term::Bool(true)])
    );
    let runtime = nir::RuntimeId {
        instance: 5,
        local: 3,
    };
    let dependent = Term::ListConcat {
        ty: Box::new(domain(Term::Nominal {
            declaration: id(4),
            arguments: vec![Term::Runtime(runtime)],
        })),
        parts: vec![],
    };
    assert!(dependent.runtime_dependencies().contains(&runtime));
}

#[test]
fn type_list_composition_never_invents_a_missing_or_foreign_domain() {
    let ty = domain(Term::Primitive(P::Type));
    let other = Term::NormalForm {
        head: id(9),
        sort: Sort::Type,
        arguments: vec![Term::Primitive(P::Type)],
    };
    let residual = Term::list_concat(
        ty.clone(),
        vec![list(&ty, vec![Term::Primitive(P::Bool)]), variable()],
    )
    .unwrap();
    let original = residual.clone();
    let binder = DeclarationParameter {
        declaration: id(2),
        sort: Sort::ComptimeValue,
    };
    for fields in [vec![], vec![Term::Primitive(P::U8)]] {
        for bad in [
            Term::TypeListValue(fields.clone()),
            list(&other, fields.clone()),
        ] {
            assert_eq!(
                Term::list_concat(ty.clone(), vec![bad.clone()]),
                Err(TypeError::InvalidListComponent)
            );
            let malformed = Term::ListConcat {
                ty: Box::new(ty.clone()),
                parts: vec![bad.clone()],
            };
            assert!(malformed.check_scope(&[]).is_err());
            assert!(residual.instantiate_parameters(&[binder], &[bad]).is_err());
            assert_eq!(residual, original);
        }
        let mut expected = vec![Term::Primitive(P::Bool)];
        expected.extend(fields.clone());
        assert_eq!(
            residual
                .instantiate_parameters(&[binder], &[list(&ty, fields)])
                .unwrap(),
            list(&ty, expected)
        );
    }
}
