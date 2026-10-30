use nir::comptime_values::{Error, freeze_admitted_list};
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn domain(element: Term) -> Term {
    Term::NormalForm {
        head: DeclarationId {
            package: 41,
            local: 7,
        },
        sort: Sort::Type,
        arguments: vec![element],
    }
}

#[test]
fn admitted_lists_share_one_encoding_and_preserve_domains_and_scopes() {
    let boolean = Term::Primitive(P::Bool);
    let types = domain(Term::Primitive(P::Type));
    for fields in [vec![], vec![boolean.clone()]] {
        assert_eq!(
            freeze_admitted_list(&types, fields.clone(), &mut 1000).unwrap(),
            Term::TypeListValue(fields)
        );
    }
    let bools = domain(boolean);
    let inner = freeze_admitted_list(&bools, vec![Term::Bool(true)], &mut 1000).unwrap();
    let nested = domain(bools.clone());
    let frozen = freeze_admitted_list(&nested, vec![inner.clone()], &mut 1000).unwrap();
    assert_eq!(
        frozen,
        Term::NominalValue {
            ty: Box::new(nested),
            variant: None,
            fields: vec![inner],
        }
    );
    assert!(frozen.check_scope(&[]).is_ok());
    assert!(frozen.to_string().starts_with("("));
    assert_ne!(
        freeze_admitted_list(&bools, vec![], &mut 1000).unwrap(),
        freeze_admitted_list(&domain(Term::Primitive(P::Char)), vec![], &mut 1000).unwrap()
    );
    let open = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let frozen = freeze_admitted_list(&types, vec![open], &mut 1000).unwrap();
    assert!(frozen.check_scope(&[vec![Sort::Type]]).is_ok());
    assert!(frozen.check_scope(&[]).is_err());
    assert_eq!(
        frozen
            .substitute_partial(&[Some(Term::Primitive(P::Bool))])
            .unwrap(),
        Term::TypeListValue(vec![Term::Primitive(P::Bool)])
    );
}

#[test]
fn list_encoding_rejects_malformed_shape_sort_and_budget_without_poisoning_retry() {
    let types = domain(Term::Primitive(P::Type));
    let bools = domain(Term::Primitive(P::Bool));
    for (ty, fields) in [
        (Term::Primitive(P::Bool), vec![]),
        (domain(Term::Bool(true)), vec![]),
        (types.clone(), vec![Term::Bool(true)]),
        (bools.clone(), vec![Term::Primitive(P::Bool)]),
        (
            Term::NormalForm {
                head: DeclarationId {
                    package: 41,
                    local: 7,
                },
                sort: Sort::Type,
                arguments: vec![],
            },
            vec![],
        ),
    ] {
        assert!(matches!(
            freeze_admitted_list(&ty, fields, &mut 1000),
            Err(Error::Invalid(_))
        ));
    }
    assert_eq!(
        freeze_admitted_list(&types, vec![], &mut 0),
        Err(Error::Budget)
    );
    let fields = vec![Term::Bool(true); 100];
    assert_eq!(
        freeze_admitted_list(&bools, fields.clone(), &mut 5),
        Err(Error::Budget)
    );
    assert!(freeze_admitted_list(&bools, fields, &mut 1000).is_ok());
}
