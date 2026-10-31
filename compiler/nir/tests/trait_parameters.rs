use nir::{DeclarationId, Primitive, Sort, Term, program::Trait};

#[test]
fn record_index_type_fields_are_domains_not_scalar_identities() {
    let mut contract = declaration();
    contract.type_parameters.truncate(1);
    contract.parameter_types = vec![Term::Record(vec![(
        "element".into(),
        Term::Primitive(Primitive::Type),
    )])];
    let index = |ty| Term::RecordValue(vec![("element".into(), ty)]);
    let left = index(Term::Primitive(Primitive::Bool));
    let right = index(Term::Primitive(Primitive::Char));
    for value in [&left, &right] {
        assert!(value.is_structural_index_value());
        contract
            .validate_arguments(std::slice::from_ref(value), &[], &mut 10_000)
            .unwrap();
    }
    assert_eq!(
        left.index_type_relations(&right),
        Some(vec![(
            &Term::Primitive(Primitive::Bool),
            &Term::Primitive(Primitive::Char)
        )])
    );
    assert!(
        left.index_type_relations(&index(Term::Bool(true)))
            .is_none()
    );
    assert!(
        contract
            .validate_arguments(&[index(Term::Bool(true))], &[], &mut 10_000)
            .is_err()
    );
    assert!(!Term::Primitive(Primitive::Bool).is_structural_index_value());
}

#[test]
fn record_indices_validate_nested_scalar_domains_and_recover_after_failure() {
    let mut contract = declaration();
    contract.type_parameters.truncate(1);
    contract.parameter_types = vec![Term::Record(vec![
        ("flag".into(), Term::Primitive(Primitive::Bool)),
        ("count".into(), Term::Primitive(Primitive::U8)),
        (
            "nested".into(),
            Term::Record(vec![("tag".into(), Term::Primitive(Primitive::Symbol))]),
        ),
    ])];
    let value = Term::RecordValue(vec![
        ("flag".into(), Term::Bool(true)),
        (
            "count".into(),
            Term::Integer(comptime_runtime::Integer::from_u128(255)),
        ),
        (
            "nested".into(),
            Term::RecordValue(vec![("tag".into(), Term::Symbol("fast".into()))]),
        ),
    ]);
    assert!(value.is_structural_index_value());
    assert!(
        contract
            .validate_arguments(std::slice::from_ref(&value), &[], &mut 1)
            .is_err()
    );
    contract
        .validate_arguments(std::slice::from_ref(&value), &[], &mut 10_000)
        .unwrap();
    for invalid in [
        Term::Integer(comptime_runtime::Integer::from_u128(256)),
        Term::Integer(comptime_runtime::Integer::from_i128(-1)),
        Term::Bool(true),
        Term::Primitive(Primitive::U8),
    ] {
        let mut changed = value.clone();
        let Term::RecordValue(fields) = &mut changed else {
            unreachable!()
        };
        fields[1].1 = invalid;
        assert!(
            contract
                .validate_arguments(&[changed], &[], &mut 10_000)
                .is_err()
        );
    }
    let mut changed = value.clone();
    let Term::RecordValue(fields) = &mut changed else {
        unreachable!()
    };
    fields[0].0 = "other".into();
    assert!(
        contract
            .validate_arguments(&[changed], &[], &mut 10_000)
            .is_err()
    );
    contract
        .validate_arguments(&[value], &[], &mut 10_000)
        .unwrap();
}

fn declaration() -> Trait {
    Trait {
        comptime_only: false,
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
fn record_index_fields_check_scalar_domains_and_signed_boundaries() {
    let mut contract = declaration();
    contract.type_parameters.truncate(1);
    for (domain, value) in [
        (Primitive::Bool, Term::Bool(false)),
        (Primitive::Symbol, Term::Symbol("tag".into())),
        (Primitive::Str, Term::String("tag".into())),
        (Primitive::Char, Term::Char('界')),
        (Primitive::Void, Term::Unit),
        (Primitive::F32, Term::Float32(0x7fc00042)),
        (Primitive::F64, Term::Float64(0x8000000000000000)),
        (
            Primitive::I8,
            Term::Integer(comptime_runtime::Integer::from_i128(-128)),
        ),
        (
            Primitive::I8,
            Term::Integer(comptime_runtime::Integer::from_i128(127)),
        ),
        (
            Primitive::U128,
            Term::Integer(comptime_runtime::Integer::from_u128(u128::MAX)),
        ),
    ] {
        contract.parameter_types = vec![Term::Record(vec![(
            "value".into(),
            Term::Primitive(domain),
        )])];
        let value = Term::RecordValue(vec![("value".into(), value)]);
        assert!(value.is_structural_index_value());
        contract
            .validate_arguments(std::slice::from_ref(&value), &[], &mut 10_000)
            .unwrap();
        contract.parameter_types = vec![Term::Record(vec![(
            "value".into(),
            Term::Primitive(Primitive::Type),
        )])];
        assert!(
            contract
                .validate_arguments(&[value], &[], &mut 10_000)
                .is_err()
        );
    }
    contract.parameter_types = vec![Term::Record(vec![(
        "value".into(),
        Term::Primitive(Primitive::I8),
    )])];
    for value in [-129, 128] {
        assert!(
            contract
                .validate_arguments(
                    &[Term::RecordValue(vec![(
                        "value".into(),
                        Term::Integer(comptime_runtime::Integer::from_i128(value))
                    )])],
                    &[],
                    &mut 10_000
                )
                .is_err()
        );
    }
    contract.parameter_types = vec![Term::Record(vec![])];
    contract
        .validate_arguments(&[Term::RecordValue(vec![])], &[], &mut 10_000)
        .unwrap();
    assert!(
        contract
            .validate_arguments(&[Term::Unit], &[], &mut 10_000)
            .is_err()
    );
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
    let sorts = package.declaration_parameter_sorts(&mut 1000).unwrap();
    assert_eq!(sorts[&contract.self_parameter], Sort::Type);
    assert_eq!(sorts[&contract.type_parameters[0]], Sort::Type);
    assert_eq!(sorts[&contract.type_parameters[1]], Sort::ComptimeValue);
    assert!(package.declaration_parameter_sorts(&mut 0).is_err());
    assert_eq!(
        package.declaration_parameter_sorts(&mut 1000).unwrap(),
        sorts
    );
    let mut conflicting = contract.clone();
    conflicting.declaration.local = 5;
    conflicting.self_parameter.local = 5;
    package.traits.push(conflicting);
    assert!(
        package
            .declaration_parameter_sorts(&mut 1000)
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

#[test]
fn frozen_integer_indices_require_exact_declared_proxy_ranges() {
    use comptime_runtime::Integer;
    let contract = declaration();
    for (signed, domains) in [
        (
            false,
            [
                Primitive::U8,
                Primitive::U16,
                Primitive::U32,
                Primitive::U64,
                Primitive::U128,
            ],
        ),
        (
            true,
            [
                Primitive::I8,
                Primitive::I16,
                Primitive::I32,
                Primitive::I64,
                Primitive::I128,
            ],
        ),
    ] {
        for domain in domains {
            let (_, width) = domain.fixed_integer_layout().unwrap();
            let (min, max, overflow) = if signed {
                let min = if width == 128 {
                    i128::MIN
                } else {
                    -(1i128 << (width - 1))
                };
                let max = if width == 128 {
                    i128::MAX
                } else {
                    (1i128 << (width - 1)) - 1
                };
                (
                    Integer::from_i128(min),
                    Integer::from_i128(max),
                    Integer::from_u128(1u128 << (width - 1)),
                )
            } else {
                let max = if width == 128 {
                    u128::MAX
                } else {
                    (1u128 << width) - 1
                };
                (
                    Integer::from_u128(0),
                    Integer::from_u128(max),
                    Integer::from_i128(-1),
                )
            };
            for value in [min, max] {
                let args = [Term::Primitive(domain), Term::Integer(value)];
                contract.validate_arguments(&args, &[], &mut 1000).unwrap();
                assert!(contract.validate_arguments(&args, &[], &mut 0).is_err());
                contract.validate_arguments(&args, &[], &mut 1000).unwrap();
            }
            let args = [Term::Primitive(domain), Term::Integer(overflow)];
            assert!(
                contract
                    .validate_arguments(&args, &[], &mut 1000)
                    .unwrap_err()
                    .contains("outside its declared domain")
            );
        }
    }
    for domain in [Primitive::Usize, Primitive::Isize, Primitive::Bool] {
        assert!(
            contract
                .validate_arguments(
                    &[
                        Term::Primitive(domain),
                        Term::Integer(Integer::from_u128(1))
                    ],
                    &[],
                    &mut 1000
                )
                .is_err()
        );
    }
}

#[test]
fn self_dependent_domains_require_an_explicit_valid_subject() {
    let mut contract = declaration();
    contract.type_parameters.truncate(1);
    contract.parameter_types = vec![Term::Rigid {
        declaration: contract.self_parameter,
        sort: Sort::Type,
    }];
    let arguments = [Term::Bool(true)];
    let subject = Term::Primitive(Primitive::Bool);
    assert!(
        contract
            .validate_arguments(&arguments, &[], &mut 1000)
            .is_err()
    );
    contract
        .validate_arguments_for_subject(&subject, &arguments, &[], &[], &mut 1000)
        .unwrap();
    for wrong in [Term::Bool(true), Term::Primitive(Primitive::Char)] {
        assert!(
            contract
                .validate_arguments_for_subject(&wrong, &arguments, &[], &[], &mut 1000)
                .is_err()
        );
    }
    assert!(
        contract
            .validate_arguments_for_subject(&subject, &arguments, &[], &[], &mut 0)
            .is_err()
    );
    contract
        .validate_arguments_for_subject(&subject, &arguments, &[], &[], &mut 1000)
        .unwrap();
}

#[test]
fn captured_newtype_and_alias_inputs_reuse_trait_domains_without_erasing_them() {
    let contract = declaration();
    let mut parameters = vec![contract.self_parameter];
    parameters.extend(&contract.type_parameters);
    let mut domains = vec![Term::Primitive(Primitive::Type)];
    domains.extend(contract.parameter_types.clone());
    let mut package = nir::program::Package {
        traits: vec![contract],
        newtypes: vec![nir::newtypes::Newtype {
            declaration: DeclarationId {
                package: 92,
                local: 100,
            },
            name: "Captured".into(),
            capture_count: parameters.len(),
            type_parameters: parameters.clone(),
            parameter_types: domains.clone(),
            representation: Term::Primitive(Primitive::Bool),
            requirements: vec![],
            capabilities: vec![],
        }],
        alias_contracts: vec![nir::type_aliases::Contract {
            declaration: DeclarationId {
                package: 92,
                local: 101,
            },
            parameters: parameters.clone(),
            parameter_types: domains.clone(),
            requirements: vec![],
            capabilities: vec![],
        }],
        ..Default::default()
    };
    let evidence = package.declaration_parameter_domains(&mut 10000).unwrap();
    for (id, domain) in parameters.iter().zip(&domains) {
        assert_eq!(&evidence[id], domain);
    }
    let last = domains.len() - 1;
    package.newtypes[0].parameter_types[last] = Term::Primitive(Primitive::Char);
    assert!(
        package
            .declaration_parameter_domains(&mut 10000)
            .unwrap_err()
            .contains("conflicting")
    );
    package.newtypes[0].parameter_types[last] = domains[last].clone();
    package.alias_contracts[0].parameter_types[last] = Term::Primitive(Primitive::Char);
    assert!(
        package
            .declaration_parameter_domains(&mut 10000)
            .unwrap_err()
            .contains("conflicting")
    );
    package.alias_contracts[0].parameter_types[last] = domains[last].clone();
    assert_eq!(
        package.declaration_parameter_domains(&mut 10000).unwrap(),
        evidence
    );
}

#[test]
fn external_rigid_domains_are_checked_without_guessing_the_interface_slot() {
    let mut contract = declaration();
    contract.parameter_types[1] = Term::Primitive(Primitive::Symbol);
    let id = DeclarationId {
        package: 9,
        local: 42,
    };
    let arguments = [
        Term::Primitive(Primitive::Bool),
        Term::Rigid {
            declaration: id,
            sort: Sort::ComptimeValue,
        },
    ];
    let subject = Term::Primitive(Primitive::Bool);
    let mut domains = std::collections::BTreeMap::new();
    for domain in [
        None,
        Some(Term::Primitive(Primitive::Bool)),
        Some(Term::Bool(true)),
    ] {
        domains.clear();
        if let Some(domain) = domain {
            domains.insert(id, domain);
        }
        assert!(
            contract
                .validate_arguments_with_domains(
                    &subject,
                    &arguments,
                    &[],
                    &[],
                    &domains,
                    &mut 1000
                )
                .is_err()
        );
    }
    domains.insert(id, Term::Primitive(Primitive::Symbol));
    assert!(
        contract
            .validate_arguments_with_domains(&subject, &arguments, &[], &[], &domains, &mut 0)
            .is_err()
    );
    contract
        .validate_arguments_with_domains(&subject, &arguments, &[], &[], &domains, &mut 1000)
        .unwrap();
    let mut conflicting = contract.clone();
    conflicting.type_parameters[1] = id;
    conflicting.parameter_types[1] = Term::Primitive(Primitive::Bool);
    assert!(
        contract
            .validate_arguments_with_domains(
                &subject,
                &arguments,
                &[],
                &[conflicting],
                &domains,
                &mut 1000
            )
            .is_err()
    );
}

#[test]
fn symbolic_record_leaves_require_domains_without_becoming_closed_values() {
    let mut contract = declaration();
    contract.type_parameters.truncate(1);
    contract.parameter_types = vec![Term::Record(vec![(
        "nested".into(),
        Term::Record(vec![("flag".into(), Term::Primitive(Primitive::Bool))]),
    )])];
    let id = DeclarationId {
        package: 9,
        local: 77,
    };
    let value = Term::RecordValue(vec![(
        "nested".into(),
        Term::RecordValue(vec![(
            "flag".into(),
            Term::Rigid {
                declaration: id,
                sort: Sort::ComptimeValue,
            },
        )]),
    )]);
    assert!(value.is_symbolic_index_value());
    assert!(!value.is_structural_index_value());
    let mut domains = std::collections::BTreeMap::new();
    for domain in [None, Some(Term::Primitive(Primitive::Char))] {
        domains.clear();
        if let Some(domain) = domain {
            domains.insert(id, domain);
        }
        assert!(
            contract
                .validate_arguments_with_domains(
                    &Term::Primitive(Primitive::Bool),
                    std::slice::from_ref(&value),
                    &[],
                    &[],
                    &domains,
                    &mut 10000
                )
                .is_err()
        );
    }
    domains.insert(id, Term::Primitive(Primitive::Bool));
    assert!(
        contract
            .validate_arguments_with_domains(
                &Term::Primitive(Primitive::Bool),
                std::slice::from_ref(&value),
                &[],
                &[],
                &domains,
                &mut 0
            )
            .is_err()
    );
    contract
        .validate_arguments_with_domains(
            &Term::Primitive(Primitive::Bool),
            &[value],
            &[],
            &[],
            &domains,
            &mut 10000,
        )
        .unwrap();
}

#[test]
fn record_index_refutation_uses_known_leaves_and_respects_limits() {
    let unknown = Term::Rigid {
        declaration: DeclarationId {
            package: 9,
            local: 90,
        },
        sort: Sort::ComptimeValue,
    };
    let record = |tag, flag, ty| {
        Term::RecordValue(vec![
            (
                "nested".into(),
                Term::RecordValue(vec![("tag".into(), Term::Bool(tag))]),
            ),
            ("unknown".into(), flag),
            ("type".into(), ty),
        ])
    };
    let left = record(true, unknown.clone(), Term::Primitive(Primitive::Bool));
    let right = record(false, Term::Bool(true), Term::Primitive(Primitive::Char));
    for (a, b) in [(&left, &right), (&right, &left)] {
        assert!(!a.index_values_definitely_differ(b, &mut 0));
        assert!(a.index_values_definitely_differ(b, &mut 1000));
    }
    let uncertain = record(true, Term::Bool(true), Term::Primitive(Primitive::Char));
    assert!(!left.index_values_definitely_differ(&uncertain, &mut 1000));
    assert!(
        !Term::Primitive(Primitive::Bool)
            .index_values_definitely_differ(&Term::Primitive(Primitive::Char), &mut 1000)
    );
    assert!(!unknown.index_values_definitely_differ(&Term::Bool(true), &mut 1000));
    assert!(Term::Bool(true).index_values_definitely_differ(&Term::Bool(false), &mut 1000));
    assert!(!left.index_values_definitely_differ(&left, &mut 1000));
}

#[test]
fn nominal_trait_indices_require_full_payload_layout_evidence() {
    let mut contract = declaration();
    contract.type_parameters.truncate(1);
    let config_id = DeclarationId {
        package: 7,
        local: 90,
    };
    let config = Term::Nominal {
        declaration: config_id,
        arguments: vec![],
    };
    contract.parameter_types = vec![config.clone()];
    let value = Term::NominalValue {
        ty: Box::new(config),
        variant: None,
        fields: vec![Term::Bool(true)],
    };
    let mut package = nir::program::Package::default();
    package.structs.push(nir::program::Struct {
        declaration: config_id,
        name: "Config".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        fields: vec![("flag".into(), Term::Primitive(Primitive::Bool))],
        requirements: vec![],
        capabilities: vec![],
    });
    let subject = Term::Primitive(Primitive::Bool);
    let domains = Default::default();
    assert!(
        contract
            .validate_arguments(std::slice::from_ref(&value), &[], &mut 10000)
            .is_err()
    );
    let validate = |value: &Term, package: &nir::program::Package, fuel: &mut usize| {
        contract.validate_arguments_with_layouts(
            &subject,
            std::slice::from_ref(value),
            &[],
            &[],
            &domains,
            fuel,
            |_, _, _| false,
            Some(package),
        )
    };
    validate(&value, &package, &mut 10000).unwrap();
    assert!(validate(&value, &package, &mut 0).is_err());
    for fields in [
        vec![],
        vec![Term::Char('x')],
        vec![Term::Bool(true), Term::Bool(false)],
    ] {
        let mut forged = value.clone();
        let Term::NominalValue {
            fields: payload, ..
        } = &mut forged
        else {
            unreachable!()
        };
        *payload = fields;
        assert!(validate(&forged, &package, &mut 10000).is_err());
    }
    let mut forged = value.clone();
    let Term::NominalValue { variant, .. } = &mut forged else {
        unreachable!()
    };
    *variant = Some(0);
    assert!(validate(&forged, &package, &mut 10000).is_err());
    package.structs[0].fields[0].1 = Term::Primitive(Primitive::Char);
    assert!(validate(&value, &package, &mut 10000).is_err());
}

#[test]
fn nominal_index_relations_preserve_variants_and_type_leaves() {
    let domain = Term::Nominal {
        declaration: DeclarationId {
            package: 1,
            local: 20,
        },
        arguments: vec![],
    };
    let value = |variant, leaf| Term::NominalValue {
        ty: Box::new(domain.clone()),
        variant,
        fields: vec![Term::TupleValue(vec![Term::Bool(true), leaf])],
    };
    let left = value(Some(0), Term::Primitive(Primitive::Bool));
    let right = value(Some(0), Term::Primitive(Primitive::Char));
    assert!(left.is_structural_index_value());
    let relations = left.index_type_relations(&right).unwrap();
    assert_eq!(relations.len(), 2);
    assert!(relations.contains(&(
        &Term::Primitive(Primitive::Bool),
        &Term::Primitive(Primitive::Char)
    )));
    assert!(
        left.index_type_relations(&value(Some(1), Term::Primitive(Primitive::Bool)))
            .is_none()
    );
    assert!(
        left.index_type_relations(&value(Some(0), Term::Bool(true)))
            .is_none()
    );
    let symbolic = value(
        None,
        Term::Rigid {
            declaration: DeclarationId {
                package: 1,
                local: 21,
            },
            sort: Sort::ComptimeValue,
        },
    );
    assert!(symbolic.is_symbolic_index_value());
    assert!(!symbolic.is_structural_index_value());
}

#[test]
fn nominal_index_refutation_uses_known_leaves_and_respects_limits() {
    let unknown = Term::Rigid {
        declaration: DeclarationId {
            package: 9,
            local: 90,
        },
        sort: Sort::ComptimeValue,
    };
    let record = |tag, flag, ty| Term::NominalValue {
        ty: Box::new(Term::Nominal {
            declaration: DeclarationId {
                package: 9,
                local: 91,
            },
            arguments: vec![],
        }),
        variant: Some(0),
        fields: vec![Term::TupleValue(vec![Term::Bool(tag), flag, ty])],
    };
    let left = record(true, unknown.clone(), Term::Primitive(Primitive::Bool));
    let right = record(false, Term::Bool(true), Term::Primitive(Primitive::Char));
    for (a, b) in [(&left, &right), (&right, &left)] {
        assert!(!a.index_values_definitely_differ(b, &mut 0));
        assert!(a.index_values_definitely_differ(b, &mut 1000));
    }
    let uncertain = record(true, Term::Bool(true), Term::Primitive(Primitive::Char));
    assert!(!left.index_values_definitely_differ(&uncertain, &mut 1000));
    assert!(
        !Term::Primitive(Primitive::Bool)
            .index_values_definitely_differ(&Term::Primitive(Primitive::Char), &mut 1000)
    );
    assert!(!unknown.index_values_definitely_differ(&Term::Bool(true), &mut 1000));
    assert!(Term::Bool(true).index_values_definitely_differ(&Term::Bool(false), &mut 1000));
    assert!(!left.index_values_definitely_differ(&left, &mut 1000));
}

#[test]
fn symbolic_nominal_payloads_require_independent_field_domains() {
    let mut contract = declaration();
    contract.type_parameters.truncate(1);
    let config_id = DeclarationId {
        package: 8,
        local: 1,
    };
    let field_id = DeclarationId {
        package: 8,
        local: 2,
    };
    let config = Term::Nominal {
        declaration: config_id,
        arguments: vec![],
    };
    contract.parameter_types = vec![config.clone()];
    let value = Term::NominalValue {
        ty: Box::new(config.clone()),
        variant: None,
        fields: vec![Term::Rigid {
            declaration: field_id,
            sort: Sort::ComptimeValue,
        }],
    };
    let mut package = nir::program::Package::default();
    package.structs.push(nir::program::Struct {
        declaration: config_id,
        name: "Config".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        fields: vec![("flag".into(), Term::Primitive(Primitive::Bool))],
        requirements: vec![],
        capabilities: vec![],
    });
    for (domain, accepted) in [
        (None, false),
        (Some(Term::Primitive(Primitive::Char)), false),
        (Some(Term::Primitive(Primitive::Bool)), true),
    ] {
        let domains = domain
            .map(|domain| (field_id, domain))
            .into_iter()
            .collect();
        let result = contract.validate_arguments_with_layouts(
            &Term::Primitive(Primitive::Bool),
            std::slice::from_ref(&value),
            &[],
            &[],
            &domains,
            &mut 10000,
            |_, _, _| false,
            Some(&package),
        );
        assert_eq!(result.is_ok(), accepted, "{result:?}");
    }
    // The completed-value entry point must not inherit symbolic admission.
    assert!(
        nir::validate_declaration_arguments_with_layouts(
            &contract.type_parameters,
            &[config],
            &[value],
            &mut 10000,
            |ty, variant, fuel| {
                nir::normal_forms::nominal_value_fields_budgeted(&package, ty, variant, fuel)
                    .ok_or_else(|| nir::NominalAdmissionError::from("invalid layout"))
            }
        )
        .is_err()
    );
}

#[test]
fn record_trait_indices_admit_nested_error_and_tuple_values_with_domains() {
    let boolean = Term::Primitive(Primitive::Bool);
    let error = Term::error_result(vec![Term::Primitive(Primitive::Char)], boolean.clone());
    let mut contract = declaration();
    contract.type_parameters.truncate(1);
    contract.parameter_types = vec![Term::Record(vec![(
        "result".into(),
        Term::Tuple(vec![error.clone(), boolean.clone()]),
    )])];
    let input = |label, payload| {
        Term::RecordValue(vec![(
            "result".into(),
            Term::TupleValue(vec![
                Term::ErrorValue {
                    ty: Box::new(error.clone()),
                    label,
                    value: Box::new(payload),
                },
                Term::Bool(true),
            ]),
        )])
    };
    for value in [
        input(None, Term::Bool(true)),
        input(
            Some(Box::new(Term::Primitive(Primitive::Char))),
            Term::Char('a'),
        ),
    ] {
        contract
            .validate_arguments(std::slice::from_ref(&value), &[], &mut 10000)
            .unwrap();
        assert!(
            contract
                .validate_arguments(std::slice::from_ref(&value), &[], &mut 1)
                .is_err()
        );
    }
    for value in [
        input(None, Term::Char('a')),
        input(Some(Box::new(boolean)), Term::Bool(true)),
        input(
            Some(Box::new(Term::Primitive(Primitive::Char))),
            Term::Bool(true),
        ),
    ] {
        assert!(
            contract
                .validate_arguments(&[value], &[], &mut 10000)
                .is_err()
        );
    }
}

#[test]
fn record_error_nominal_payloads_require_layouts_and_reject_forged_fields() {
    let config_id = DeclarationId {
        package: 1,
        local: 201,
    };
    let nominal = Term::Nominal {
        declaration: config_id,
        arguments: vec![],
    };
    let result = Term::error_result(vec![nominal.clone()], Term::Primitive(Primitive::Bool));
    let mut contract = declaration();
    contract.type_parameters.truncate(1);
    contract.parameter_types = vec![Term::Record(vec![("result".into(), result.clone())])];
    let input = |payload| {
        Term::RecordValue(vec![(
            "result".into(),
            Term::ErrorValue {
                ty: Box::new(result.clone()),
                label: Some(Box::new(nominal.clone())),
                value: Box::new(Term::NominalValue {
                    ty: Box::new(nominal.clone()),
                    variant: None,
                    fields: vec![payload],
                }),
            },
        )])
    };
    let mut package = nir::program::Package::default();
    package.structs.push(nir::program::Struct {
        declaration: config_id,
        name: "Config".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        fields: vec![("flag".into(), Term::Primitive(Primitive::Bool))],
        requirements: vec![],
        capabilities: vec![],
    });
    let valid = input(Term::Bool(true));
    assert!(
        contract
            .validate_arguments(std::slice::from_ref(&valid), &[], &mut 10000)
            .is_err()
    );
    let validate = |value: &Term, fuel: &mut usize| {
        contract.validate_arguments_with_layouts(
            &Term::Primitive(Primitive::Bool),
            std::slice::from_ref(value),
            &[],
            &[],
            &Default::default(),
            fuel,
            |_, _, _| false,
            Some(&package),
        )
    };
    validate(&valid, &mut 10000).unwrap();
    assert!(validate(&input(Term::Char('a')), &mut 10000).is_err());
    assert!(validate(&valid, &mut 1).is_err());
    validate(&valid, &mut 10000).unwrap();
}

#[test]
fn platform_trait_indices_use_layout_target_for_scalar_and_record_leaves() {
    use std::collections::BTreeMap;
    let mut contract = declaration();
    contract.type_parameters.truncate(1);
    let scalar = Term::Integer(comptime_runtime::Integer::from_u128(1u128 << 32));
    for nested in [false, true] {
        let domain = Term::Primitive(Primitive::Usize);
        contract.parameter_types = vec![if nested {
            Term::Record(vec![("count".into(), domain)])
        } else {
            domain
        }];
        let argument = if nested {
            Term::RecordValue(vec![("count".into(), scalar.clone())])
        } else {
            scalar.clone()
        };
        for (width, accepted) in [(None, false), (Some(32), false), (Some(64), true)] {
            let package = nir::program::Package {
                target: width.map(|width| nir::target::TargetData::new(width).unwrap()),
                ..Default::default()
            };
            let validate = |fuel: &mut usize| {
                contract.validate_arguments_with_layouts(
                    &Term::Primitive(Primitive::Bool),
                    std::slice::from_ref(&argument),
                    &[],
                    &[],
                    &BTreeMap::new(),
                    fuel,
                    |_, _, _| false,
                    Some(&package),
                )
            };
            assert!(validate(&mut 0).is_err());
            assert_eq!(validate(&mut 10000).is_ok(), accepted);
        }
    }
}

#[test]
fn type_valued_trait_indices_follow_completed_parameter_and_self_domains() {
    let mut contract = declaration();
    let type_domain = Term::Primitive(Primitive::Type);
    let boolean = Term::Primitive(Primitive::Bool);
    let arguments = [type_domain.clone(), boolean.clone()];
    contract
        .validate_arguments(&arguments, &[], &mut 1000)
        .unwrap();
    for invalid in [
        vec![type_domain.clone(), Term::Bool(true)],
        vec![boolean.clone(), boolean.clone()],
    ] {
        assert!(
            contract
                .validate_arguments(&invalid, &[], &mut 1000)
                .is_err()
        );
    }
    assert!(
        contract
            .validate_arguments(&arguments, &[], &mut 0)
            .is_err()
    );
    contract
        .validate_arguments(&arguments, &[], &mut 1000)
        .unwrap();
    contract.type_parameters.truncate(1);
    contract.parameter_types = vec![Term::Rigid {
        declaration: contract.self_parameter,
        sort: Sort::Type,
    }];
    contract
        .validate_arguments_for_subject(&type_domain, &[boolean.clone()], &[], &[], &mut 1000)
        .unwrap();
    assert!(
        contract
            .validate_arguments_for_subject(&boolean, &[boolean.clone()], &[], &[], &mut 1000)
            .is_err()
    );
}

#[test]
fn value_difference_reports_exhaustion_separately_from_unknown_types() {
    let left = Term::Symbol("x".repeat(512).into());
    let right = Term::Symbol("y".repeat(512).into());
    // Charging a payload can fail while leaving nonzero fuel; a boolean
    // non-refutation must not be mistaken for a completed candidate search.
    for budget in [0, 10, 600] {
        assert!(matches!(
            left.index_values_definitely_differ_budgeted(&right, &mut { budget }),
            Err(nir::type_computations::Error::Budget)
        ));
        assert!(!left.index_values_definitely_differ(&right, &mut { budget }));
    }
    assert_eq!(
        left.index_values_definitely_differ_budgeted(&right, &mut 10000),
        Ok(true)
    );
    assert_eq!(
        left.index_values_definitely_differ_budgeted(&left, &mut 10000),
        Ok(false)
    );
    assert_eq!(
        Term::Primitive(nir::Primitive::Bool).index_values_definitely_differ_budgeted(
            &Term::Primitive(nir::Primitive::Char),
            &mut 1000
        ),
        Ok(false)
    );
}
