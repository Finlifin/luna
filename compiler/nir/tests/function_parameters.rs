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

#[test]
fn callable_list_inputs_require_unique_representation_evidence_and_recover() {
    use nir::function_parameters::validate_arguments;
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    let domain = Term::NormalForm {
        head: id(20),
        sort: Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Type)],
    };
    let head = Head {
        collector: None,
        declaration: id(20),
        role: Role::TypeList,
        parameters: vec![Parameter {
            declaration: id(21),
            binding: nir::program::LocalId {
                owner: 20,
                index: 0,
            },
            kind: Kind::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
        requirements: vec![],
    };
    let package = Package {
        normal_forms: vec![head.clone()],
        ..Default::default()
    };
    let frozen = |fields| Term::NominalValue {
        ty: Box::new(domain.clone()),
        variant: None,
        fields,
    };
    let valid = frozen(vec![Term::Primitive(Primitive::Bool)]);
    let check = |package: &Package, value: Term, fuel: &mut usize| {
        validate_arguments(package, &[id(1)], &[domain.clone()], &[value], fuel)
    };
    assert!(check(&package, valid.clone(), &mut 1000).is_ok());
    assert!(check(&package, frozen(vec![]), &mut 1000).is_ok());
    for heads in [
        vec![],
        vec![Head {
            collector: None,
            role: Role::Opaque,
            ..head.clone()
        }],
        vec![head.clone(), head.clone()],
    ] {
        assert!(
            check(
                &Package {
                    normal_forms: heads,
                    ..Default::default()
                },
                valid.clone(),
                &mut 1000
            )
            .is_err()
        );
    }
    for value in [
        Term::TupleValue(vec![]),
        Term::TupleValue(vec![Term::Primitive(Primitive::Bool)]),
        frozen(vec![Term::Bool(true)]),
        frozen(vec![Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }]),
        Term::Bool(true),
    ] {
        assert!(check(&package, value, &mut 1000).is_err());
    }
    assert!(check(&package, valid.clone(), &mut 0).is_err());
    assert!(check(&package, valid.clone(), &mut 1000).is_ok());
    // Resolve a domain supplied through another input, not merely a literal
    // List<Type> annotation. Full-vector substitution must retain its identity.
    assert!(
        validate_arguments(
            &package,
            &[id(1), id(2)],
            &[
                Term::Primitive(Primitive::Type),
                Term::Rigid {
                    declaration: id(1),
                    sort: Sort::Type
                },
            ],
            &[domain, valid],
            &mut 1000
        )
        .is_ok()
    );
}

#[test]
fn callable_collections_preserve_dependent_domains_and_scalar_ranges() {
    use nir::function_parameters::validate_arguments;
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    let list = |element| Term::NormalForm {
        head: id(20),
        sort: Sort::Type,
        arguments: vec![element],
    };
    let frozen = |domain: &Term, fields| Term::NominalValue {
        ty: Box::new(domain.clone()),
        variant: None,
        fields,
    };
    let package = Package {
        normal_forms: vec![Head {
            declaration: id(20),
            role: Role::TypeList,
            collector: None,
            parameters: vec![Parameter {
                declaration: id(21),
                binding: nir::program::LocalId {
                    owner: 20,
                    index: 0,
                },
                kind: Kind::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            requirements: vec![],
        }],
        ..Default::default()
    };
    let bools = list(Term::Primitive(Primitive::Bool));
    let types = list(Term::Primitive(Primitive::Type));
    let nested = list(bools.clone());
    for (domain, value) in [
        (bools.clone(), frozen(&bools, vec![Term::Bool(true)])),
        (
            types.clone(),
            frozen(&types, vec![Term::Primitive(Primitive::Bool)]),
        ),
        (
            nested.clone(),
            frozen(&nested, vec![frozen(&bools, vec![])]),
        ),
    ] {
        let domains = [
            Term::Rigid {
                declaration: id(2),
                sort: Sort::Type,
            },
            Term::Primitive(Primitive::Type),
        ];
        let arguments = [value, domain];
        validate_arguments(&package, &[id(1), id(2)], &domains, &arguments, &mut 10000).unwrap();
        assert!(
            validate_arguments(&package, &[id(1), id(2)], &domains, &arguments, &mut 0).is_err()
        );
        validate_arguments(&package, &[id(1), id(2)], &domains, &arguments, &mut 10000).unwrap();
    }
    let bytes = list(Term::Primitive(Primitive::U8));
    for (number, accepted) in [(255, true), (256, false)] {
        let input = frozen(
            &bytes,
            vec![Term::Integer(comptime_runtime::Integer::from_u128(number))],
        );
        assert_eq!(
            validate_arguments(&package, &[id(1)], &[bytes.clone()], &[input], &mut 10000).is_ok(),
            accepted
        );
    }
    let invalid = frozen(
        &bools,
        vec![Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::ComptimeValue,
        }],
    );
    assert!(validate_arguments(&package, &[id(1)], &[bools], &[invalid], &mut 10000).is_err());
}
