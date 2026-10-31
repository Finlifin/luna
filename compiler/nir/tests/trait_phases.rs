use nir::function_parameters::equivalent_prefix_domains as equivalent;
use nir::{DeclarationId as Id, Primitive as P, Sort, Term};
use std::collections::BTreeMap;
fn id(local: u32) -> Id {
    Id { package: 1, local }
}
fn var(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}

#[test]
fn alpha_prefixes_preserve_dependent_value_domains() {
    let domains = BTreeMap::from([
        (id(1), Term::Primitive(P::Type)),
        (id(2), var(1)),
        (id(3), Term::Primitive(P::Type)),
        (id(4), var(3)),
    ]);
    assert!(equivalent(&[id(1), id(2)], &[id(3), id(4)], &domains, &mut 1000).unwrap());
    assert!(!equivalent(&[id(1), id(2)], &[id(4), id(3)], &domains, &mut 1000).unwrap());
    assert!(!equivalent(&[id(1)], &[id(3), id(4)], &domains, &mut 1000).unwrap());
}

#[test]
fn free_identities_and_missing_value_metadata_do_not_become_alpha_bindings() {
    let domains = BTreeMap::from([
        (id(2), var(3)),
        (id(4), var(1)),
        (id(5), Term::Primitive(P::Bool)),
    ]);
    assert!(!equivalent(&[id(1), id(2)], &[id(3), id(4)], &domains, &mut 1000).unwrap());
    assert!(!equivalent(&[id(5)], &[id(6)], &domains, &mut 1000).unwrap());
    assert!(equivalent(&[id(1)], &[id(3)], &domains, &mut 1000).unwrap());
}

#[test]
fn malformed_domains_and_exhausted_prefix_validation_are_rejected() {
    for invalid in [
        Term::Bool(true),
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
    ] {
        let domains = BTreeMap::from([(id(1), invalid.clone()), (id(2), invalid)]);
        assert!(!equivalent(&[id(1)], &[id(2)], &domains, &mut 1000).unwrap());
    }
    let domains = BTreeMap::new();
    assert!(equivalent(&[id(1)], &[id(2)], &domains, &mut 0).is_err());
    assert!(equivalent(&[id(1)], &[id(2)], &domains, &mut 1000).unwrap());
}
