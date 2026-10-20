use nir::program::TypePredicate;
use nir::type_aliases::{Contract, validate};
use nir::{DeclarationId, Primitive, Sort, Term};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn contract() -> Contract {
    Contract {
        declaration: id(1),
        parameters: vec![id(2)],
        parameter_types: vec![],
        requirements: vec![TypePredicate::Equal(
            Term::Rigid {
                declaration: id(2),
                sort: Sort::Type,
            },
            Term::Primitive(Primitive::Bool),
        )],
        capabilities: vec![],
    }
}
#[test]
fn alias_contract_metadata_requires_unique_closed_type_parameters() {
    let mut value = contract();
    assert!(validate(&[value.clone()]).is_ok());
    assert!(
        validate(&[value.clone(), value.clone()])
            .unwrap_err()
            .contains("duplicate alias contract identity")
    );
    value.parameters.push(id(2));
    assert!(
        validate(&[value])
            .unwrap_err()
            .contains("duplicate alias contract parameter")
    );
    for term in [
        Term::Rigid {
            declaration: id(3),
            sort: Sort::Type,
        },
        Term::Rigid {
            declaration: id(2),
            sort: Sort::ComptimeValue,
        },
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
    ] {
        let mut value = contract();
        value.requirements = vec![TypePredicate::Equal(term, Term::Primitive(Primitive::Bool))];
        assert!(validate(&[value]).is_err());
    }
}
#[test]
fn alias_contract_validation_is_bounded_and_failures_do_not_persist() {
    let mut value = contract();
    value.requirements = vec![TypePredicate::Or(vec![TypePredicate::Bool(true); 100_001])];
    assert_eq!(
        validate(&[value]),
        Err("alias contract validation budget exhausted")
    );
    assert!(validate(&[contract()]).is_ok());
}
#[test]
fn alias_contract_dump_exposes_telescope_and_predicate_structure() {
    let dump = contract().to_string();
    assert!(dump.starts_with("(type-alias-contract (declaration "));
    assert!(dump.contains("(parameters "));
    assert!(dump.contains("(requires "));
    assert!(dump.ends_with("(capabilities))"));
    let mut depth = 0;
    for c in dump.chars() {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            assert!(depth >= 0);
        }
    }
    assert_eq!(depth, 0);
}

#[test]
fn alias_contract_domains_are_a_typed_telescope() {
    let mut value = contract();
    value.parameters.push(id(3));
    value.parameter_types = vec![
        Term::Primitive(Primitive::Type),
        Term::Rigid {
            declaration: id(2),
            sort: Sort::Type,
        },
    ];
    let index = Term::Rigid {
        declaration: id(3),
        sort: Sort::ComptimeValue,
    };
    let indexed_type = Term::Nominal {
        declaration: id(40),
        arguments: vec![index],
    };
    value.requirements = vec![TypePredicate::Equal(indexed_type.clone(), indexed_type)];
    assert!(validate(&[value.clone()]).is_ok());
    assert_eq!(
        value.parameter_binders().unwrap()[1].sort,
        Sort::ComptimeValue
    );
    assert!(value.to_string().contains("(domains "));
    value.parameter_types[1] = Term::Rigid {
        declaration: id(3),
        sort: Sort::Type,
    };
    assert!(validate(&[value.clone()]).is_err());
    value.parameter_types[1] = Term::Rigid {
        declaration: id(2),
        sort: Sort::ComptimeValue,
    };
    assert!(validate(&[value.clone()]).is_err());
    value.parameter_types.pop();
    assert!(validate(&[value]).is_err());
}
