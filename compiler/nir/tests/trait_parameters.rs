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
