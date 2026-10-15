use nir::{DeclarationId, Primitive, Sort, Term, program::Trait};

fn declaration() -> Trait {
    Trait {
        parameter_types: vec![
            Term::Primitive(Primitive::Type),
            Term::Rigid {
                declaration: DeclarationId {
                    package: 1,
                    local: 1,
                },
                sort: Sort::Type,
            },
        ],
        type_parameters: vec![
            DeclarationId {
                package: 1,
                local: 1,
            },
            DeclarationId {
                package: 1,
                local: 2,
            },
        ],
        declaration: DeclarationId {
            package: 1,
            local: 0,
        },
        self_parameter: DeclarationId {
            package: 1,
            local: 0,
        },
        name: "Indexed".into(),
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        parents: vec![],
        methods: vec![],
    }
}

#[test]
fn declaration_domains_substitute_earlier_types_and_reject_wrong_values() {
    let contract = declaration();
    for (domain, value) in [
        (Primitive::Symbol, Term::Symbol("px".into())),
        (Primitive::Bool, Term::Bool(true)),
    ] {
        let args = [Term::Primitive(domain), value];
        contract.validate_arguments(&args, &[], &mut 1000).unwrap();
        assert!(contract.validate_arguments(&args, &[], &mut 0).is_err());
        contract.validate_arguments(&args, &[], &mut 1000).unwrap();
    }
    assert!(
        contract
            .validate_arguments(
                &[
                    Term::Primitive(Primitive::Symbol),
                    Term::String("px".into())
                ],
                &[],
                &mut 1000
            )
            .is_err()
    );
    assert!(
        contract
            .validate_arguments(
                &[
                    Term::Primitive(Primitive::Symbol),
                    Term::Value { store: 1, index: 0 }
                ],
                &[],
                &mut 1000
            )
            .is_err()
    );
    let binders = contract.binders_with_self(&mut 1000).unwrap();
    assert_eq!(
        binders.iter().map(|p| p.sort).collect::<Vec<_>>(),
        vec![Sort::Type, Sort::Type, Sort::ComptimeValue]
    );
    assert!(
        contract
            .to_string()
            .contains("(parameter-types (primitive Type)")
    );
}

#[test]
fn compact_type_domains_and_malformed_metadata_stay_distinct() {
    let mut contract = declaration();
    contract.parameter_types.clear();
    assert!(
        contract
            .validate_arguments(
                &[
                    Term::Primitive(Primitive::Symbol),
                    Term::Symbol("px".into())
                ],
                &[],
                &mut 1000
            )
            .is_err()
    );
    contract
        .validate_arguments(
            &[
                Term::Primitive(Primitive::Symbol),
                Term::Primitive(Primitive::Bool),
            ],
            &[],
            &mut 1000,
        )
        .unwrap();
    contract.type_parameters[1] = contract.type_parameters[0];
    assert!(contract.parameter_binders(&mut 1000).is_err());
}

#[test]
fn package_inherited_sorts_are_declared_bounded_and_unambiguous() {
    let contract = declaration();
    let mut package = nir::program::Package {
        traits: vec![contract.clone()],
        ..Default::default()
    };
    let sorts = package.trait_parameter_sorts(&mut 1000).unwrap();
    assert_eq!(sorts[&contract.self_parameter], Sort::Type);
    assert_eq!(sorts[&contract.type_parameters[0]], Sort::Type);
    assert_eq!(sorts[&contract.type_parameters[1]], Sort::ComptimeValue);
    assert!(package.trait_parameter_sorts(&mut 0).is_err());
    assert_eq!(package.trait_parameter_sorts(&mut 1000).unwrap(), sorts);
    let mut conflicting = contract.clone();
    conflicting.declaration.local = 5;
    conflicting.self_parameter.local = 5;
    package.traits.push(conflicting);
    assert!(
        package
            .trait_parameter_sorts(&mut 1000)
            .unwrap_err()
            .contains("duplicate")
    );
}

#[test]
fn rigid_value_indices_require_unique_matching_declaration_domains() {
    let mut owner = declaration();
    owner.parameter_types[1] = Term::Primitive(Primitive::Symbol);
    let args = [
        Term::Primitive(Primitive::Bool),
        Term::Rigid {
            declaration: owner.type_parameters[1],
            sort: Sort::ComptimeValue,
        },
    ];
    assert!(owner.validate_arguments(&args, &[], &mut 1000).is_err());
    owner
        .validate_arguments_in_declarations(&args, &[], &[owner.clone()], &mut 1000)
        .unwrap();
    assert!(
        owner
            .validate_arguments_in_declarations(&args, &[], &[owner.clone()], &mut 0)
            .is_err()
    );
    let mut wrong_domain = owner.clone();
    wrong_domain.parameter_types[1] = Term::Primitive(Primitive::Bool);
    assert!(
        owner
            .validate_arguments_in_declarations(&args, &[], &[wrong_domain], &mut 1000)
            .is_err()
    );
    assert!(
        owner
            .validate_arguments_in_declarations(
                &args,
                &[],
                &[owner.clone(), owner.clone()],
                &mut 1000
            )
            .is_err()
    );
    let unknown = [
        args[0].clone(),
        Term::Rigid {
            declaration: DeclarationId {
                package: 1,
                local: 99,
            },
            sort: Sort::ComptimeValue,
        },
    ];
    assert!(
        owner
            .validate_arguments_in_declarations(&unknown, &[], &[owner.clone()], &mut 1000)
            .is_err()
    );
    owner
        .validate_arguments_in_declarations(&args, &[], &[owner.clone()], &mut 1000)
        .unwrap();
}
