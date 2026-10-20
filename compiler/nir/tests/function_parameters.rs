use nir::{
    DeclarationId, Primitive, Sort, Term, function_parameters::Declaration, program::Package,
};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn metadata() -> Declaration {
    Declaration {
        declaration: id(10),
        parameters: vec![id(1), id(2)],
        domains: vec![
            Term::Primitive(Primitive::Type),
            Term::Rigid {
                declaration: id(1),
                sort: Sort::Type,
            },
        ],
    }
}
#[test]
fn callable_domains_preserve_dependent_value_sorts_without_bodies() {
    let declaration = metadata();
    let binders = declaration.binders().unwrap();
    assert_eq!(
        binders.iter().map(|b| b.sort).collect::<Vec<_>>(),
        vec![Sort::Type, Sort::ComptimeValue]
    );
    let package = Package {
        function_parameters: vec![declaration.clone()],
        ..Default::default()
    };
    let domains = package.declaration_parameter_domains(&mut 1000).unwrap();
    assert_eq!(domains[&id(2)], declaration.domains[1]);
    assert!(package.functions.is_empty());
    assert!(package.to_string().contains(&declaration.to_string()));
    assert_eq!(
        declaration.to_string(),
        "(function-parameters (declaration (declaration (package 1) (local 10))) (inputs (declaration (package 1) (local 1)) (declaration (package 1) (local 2))) (domains (primitive Type) (rigid (id (declaration (package 1) (local 1))) (sort type))))"
    );
}
#[test]
fn shared_captures_require_consistent_domains_and_unique_callable_entries() {
    let first = metadata();
    let mut second = first.clone();
    second.declaration = id(11);
    let mut package = Package {
        function_parameters: vec![first, second],
        ..Default::default()
    };
    assert!(package.declaration_parameter_domains(&mut 1000).is_ok());
    package.function_parameters[1].domains[1] = Term::Primitive(Primitive::Bool);
    assert!(package.declaration_parameter_domains(&mut 1000).is_err());
    package.function_parameters[1] = package.function_parameters[0].clone();
    assert!(package.declaration_parameter_domains(&mut 1000).is_err());
}
#[test]
fn callable_domain_validation_rejects_malformed_and_exhausted_inputs() {
    let good = metadata();
    for mutation in 0..4 {
        let mut invalid = good.clone();
        match mutation {
            0 => invalid.parameters[1] = invalid.parameters[0],
            1 => {
                invalid.domains.pop();
            }
            2 => invalid.domains[1] = Term::Bool(true),
            _ => {
                invalid.domains[1] = Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::Type,
                }
            }
        }
        let package = Package {
            function_parameters: vec![invalid],
            ..Default::default()
        };
        assert!(package.declaration_parameter_domains(&mut 1000).is_err());
    }
    let package = Package {
        function_parameters: vec![good],
        ..Default::default()
    };
    assert!(package.declaration_parameter_domains(&mut 0).is_err());
    assert!(package.declaration_parameter_domains(&mut 1000).is_ok());
}
