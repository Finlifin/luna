use nir::program::{Enum, Struct, Variant, VariantShape};
use nir::{DeclarationId, Primitive as P, Sort, Term, TypeError};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 91, local }
}
fn rigid(local: u32, sort: Sort) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort,
    }
}

#[test]
fn full_nominal_instances_preserve_metadata_and_specialize_contracts() {
    use nir::program::{TraitRequirement, TypePredicate};
    let parameter = rigid(1, Sort::Type);
    let normal = Term::NormalForm {
        head: id(90),
        sort: Sort::Type,
        arguments: vec![parameter.clone()],
    };
    let declaration = Struct {
        declaration: id(2),
        name: "Container".into(),
        type_parameters: vec![id(1)],
        parameter_types: vec![Term::Primitive(P::Type)],
        fields: vec![("value".into(), normal)],
        requirements: vec![TypePredicate::Equal(parameter.clone(), parameter.clone())],
        capabilities: vec![TraitRequirement {
            interface: id(3),
            subject: parameter.clone(),
            interface_arguments: vec![parameter],
        }],
    };
    let original = declaration.clone();
    let argument = Term::Primitive(P::Bool);
    let instance = declaration
        .instantiate_definition_bounded(&[argument.clone()], &[], &mut 10_000)
        .unwrap();
    assert!(instance.type_parameters.is_empty() && instance.parameter_types.is_empty());
    assert_eq!(
        instance.fields,
        vec![(
            "value".into(),
            Term::NormalForm {
                head: id(90),
                sort: Sort::Type,
                arguments: vec![argument.clone()]
            }
        )]
    );
    assert_eq!(
        instance.requirements,
        vec![TypePredicate::Equal(argument.clone(), argument.clone())]
    );
    assert_eq!(instance.capabilities[0].subject, argument);
    assert_eq!(
        instance.capabilities[0].interface_arguments,
        vec![argument.clone()]
    );
    assert_eq!(declaration, original);
    let enumeration = Enum {
        declaration: id(4),
        name: "Choices".into(),
        type_parameters: declaration.type_parameters.clone(),
        parameter_types: declaration.parameter_types.clone(),
        requirements: declaration.requirements.clone(),
        capabilities: declaration.capabilities.clone(),
        groups: vec!["nested".into()],
        encoding: None,
        symbol_encoding: None,
        is_intrinsic: true,
        variants: vec![
            Variant {
                name: "empty".into(),
                shape: VariantShape::Unit,
                fields: vec![],
            },
            Variant {
                name: "nested.value".into(),
                shape: VariantShape::Named(vec!["value".into()]),
                fields: vec![rigid(1, Sort::Type)],
            },
        ],
    };
    let specialized = enumeration
        .instantiate_definition_bounded(&[argument.clone()], &[], &mut 10_000)
        .unwrap();
    assert_eq!(specialized.groups, enumeration.groups);
    assert_eq!(specialized.variants[1].shape, enumeration.variants[1].shape);
    assert!(specialized.variants[0].fields.is_empty());
    assert_eq!(specialized.variants[1].fields, vec![argument.clone()]);
    assert_eq!(specialized.requirements, instance.requirements);
    assert_eq!(specialized.capabilities, instance.capabilities);
    assert!(specialized.is_intrinsic);
    assert!(
        enumeration
            .instantiate_definition_bounded(&[Term::Bool(true)], &[], &mut 10_000)
            .is_err()
    );
    enumeration
        .instantiate_definition_bounded(&[argument], &[], &mut 10_000)
        .unwrap();
}

#[test]
fn full_enum_instances_charge_encoding_payloads_before_copying() {
    use comptime_runtime::{Budget, bitvec::BitVec};
    use nir::enum_encoding::{Layout, Representation, SymbolRepresentation, SymbolVariant};
    use nir::type_computations::Error;
    let mut declaration = Enum {
        declaration: id(2),
        name: "Encoded".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        groups: vec![],
        encoding: None,
        symbol_encoding: None,
        is_intrinsic: false,
        variants: vec![Variant {
            name: "only".into(),
            shape: VariantShape::Unit,
            fields: vec![],
        }],
    };
    for symbols in [false, true] {
        if symbols {
            declaration.encoding = None;
            declaration.symbol_encoding = Some(
                SymbolRepresentation::new(
                    vec![SymbolVariant {
                        name: "only".into(),
                        value: "s".repeat(8192),
                    }],
                    &mut Budget::default(),
                )
                .unwrap(),
            );
        } else {
            let mut budget = Budget::default();
            let value = BitVec::from_words(8192, &[0; 256], &mut budget).unwrap();
            let variant = nir::enum_encoding::Variant::new(
                "only".into(),
                value.not(&mut budget).unwrap(),
                value,
                vec![],
                &mut budget,
            )
            .unwrap();
            declaration.encoding = Some(Representation {
                base_type: None,
                layout: Layout::new(8192, vec![variant], &mut budget).unwrap(),
            });
        }
        assert!(matches!(
            declaration.instantiate_definition_bounded(&[], &[], &mut 500),
            Err(Error::Budget)
        ));
        // Payload-only observation has no reason to copy the encoding plan.
        assert_eq!(
            declaration
                .instantiate_variant_fields(&[], &[], &mut 10)
                .unwrap(),
            vec![Vec::<Term>::new()]
        );
        let copy = declaration
            .instantiate_definition_bounded(&[], &[], &mut 100_000)
            .unwrap();
        assert_eq!(copy, declaration);
    }
}

fn frozen_types(fields: Vec<nir::Term>) -> nir::Term {
    nir::Term::NominalValue {
        ty: Box::new(nir::Term::NormalForm {
            head: nir::DeclarationId {
                package: 901,
                local: 1,
            },
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Type)],
        }),
        variant: None,
        fields,
    }
}

#[test]
fn completed_value_domains_can_use_captured_type_suffixes() {
    let parameters = [id(1), id(2)];
    let domains = [rigid(2, Sort::Type), Term::Primitive(P::Type)];
    let valid = [Term::Bool(true), Term::Primitive(P::Bool)];
    nir::validate_declaration_arguments(&parameters, &domains, &valid, &mut 1000).unwrap();
    // A completed Type domain admits a Type value, even when the source
    // declaration had to retain a dependent value binder before completion.
    let type_value = [Term::Primitive(P::Bool), Term::Primitive(P::Type)];
    nir::validate_declaration_arguments(&parameters, &domains, &type_value, &mut 1000).unwrap();
    for arguments in [
        [Term::Bool(true), Term::Primitive(P::U8)],
        [Term::Bool(true), Term::Bool(false)],
    ] {
        assert!(
            nir::validate_declaration_arguments(&parameters, &domains, &arguments, &mut 1000)
                .is_err()
        );
    }
    assert!(matches!(
        nir::validate_declaration_arguments(&parameters, &domains, &valid, &mut 0),
        Err(nir::NominalAdmissionError::Budget)
    ));
    // Forward substitution cannot manufacture evidence for a symbolic value.
    let symbolic = [rigid(3, Sort::ComptimeValue), Term::Primitive(P::Bool)];
    assert!(
        nir::validate_declaration_arguments(&parameters, &domains, &symbolic, &mut 1000).is_err()
    );
}
fn record(domains: Vec<Term>) -> Struct {
    Struct {
        parameter_types: domains,
        type_parameters: vec![id(1), id(2)],
        declaration: id(0),
        name: "Indexed".into(),
        capabilities: vec![],
        requirements: vec![],
        fields: vec![
            ("value".into(), rigid(1, Sort::Type)),
            (
                "indexed".into(),
                Term::NormalForm {
                    sort: Sort::Type,
                    head: id(3),
                    arguments: vec![rigid(2, Sort::ComptimeValue)],
                },
            ),
        ],
    }
}

#[test]
fn borrowed_value_layouts_require_matching_declarations_and_domains() {
    use nir::normal_forms::{
        NominalValueDeclaration as D, nominal_value_fields_with_lookup as fields,
    };
    let definition = record(vec![Term::Primitive(P::Type), Term::Primitive(P::Bool)]);
    let lookup = |_: DeclarationId, fuel: &mut usize| {
        *fuel = fuel.checked_sub(1)?;
        Some(D::Struct(&definition))
    };
    let ty = Term::Nominal {
        declaration: id(0),
        arguments: vec![Term::Primitive(P::U8), Term::Bool(true)],
    };
    assert_eq!(
        fields(&ty, None, &mut 1000, &lookup).unwrap(),
        vec![
            Term::Primitive(P::U8),
            Term::NormalForm {
                head: id(3),
                sort: Sort::Type,
                arguments: vec![Term::Bool(true)]
            }
        ]
    );
    assert!(fields(&ty, Some(0), &mut 1000, &lookup).is_none());
    assert!(fields(&ty, None, &mut 0, &lookup).is_none());
    let wrong_identity = Term::Nominal {
        declaration: id(5),
        arguments: vec![Term::Primitive(P::U8), Term::Bool(true)],
    };
    assert!(fields(&wrong_identity, None, &mut 1000, &lookup).is_none());
    let wrong_domain = Term::Nominal {
        declaration: id(0),
        arguments: vec![Term::Primitive(P::U8), Term::Unit],
    };
    assert!(fields(&wrong_domain, None, &mut 1000, &lookup).is_none());
    assert!(fields(&ty, None, &mut 1000, &|_, _| None).is_none());
}

#[test]
fn typed_layout_queries_preserve_lookup_errors_and_failed_reservations() {
    use nir::NominalAdmissionError as Error;
    use nir::normal_forms::{
        NominalValueDeclaration as D, query_nominal_value_fields_with as fields,
    };
    let definition = record(vec![Term::Primitive(P::Type), Term::Primitive(P::Bool)]);
    let ty = Term::Nominal {
        declaration: id(0),
        arguments: vec![Term::Primitive(P::U8), Term::Bool(true)],
    };
    let lookup = |requested, fuel: &mut usize| {
        *fuel = fuel.checked_sub(3).ok_or(Error::Budget)?;
        Ok((requested == definition.declaration).then_some(D::Struct(&definition)))
    };
    let check =
        |ty: &Term, fuel: &mut usize| fields(ty, None, fuel, &lookup, &|_, _| Ok(None), None);
    let mut fuel = 2;
    assert_eq!(check(&ty, &mut fuel), Err(Error::Budget));
    assert_eq!(fuel, 2, "failed lookup reservation retains unspent fuel");
    let mut completed = false;
    for allowance in 0..1000 {
        let mut fuel = allowance;
        match check(&ty, &mut fuel) {
            Err(Error::Budget) => {}
            Ok(layout) => {
                assert_eq!(layout[0], Term::Primitive(P::U8));
                completed = true;
                break;
            }
            other => panic!("valid layout lost its outcome: {other:?}"),
        }
    }
    assert!(completed);
    let foreign = Term::Nominal {
        declaration: id(99),
        arguments: vec![],
    };
    assert!(matches!(check(&foreign, &mut 1000), Err(Error::Invalid(_))));
    let wrong = Term::Nominal {
        declaration: id(0),
        arguments: vec![Term::Primitive(P::U8), Term::Char('x')],
    };
    assert!(matches!(check(&wrong, &mut 1000), Err(Error::Invalid(_))));
    let invalid = Error::Invalid("lookup evidence belongs to a retired world".into());
    assert_eq!(
        fields(
            &ty,
            None,
            &mut 1000,
            &|_, _| Err(invalid.clone()),
            &|_, _| Ok(None),
            None
        ),
        Err(invalid)
    );
    assert!(check(&ty, &mut 1000).is_ok());
}

#[test]
fn selected_field_instantiation_keeps_budget_and_domain_failures_distinct() {
    use nir::NominalAdmissionError as Error;
    use nir::normal_forms::instantiate_value_fields_checked as fields;
    let parameters = [id(1)];
    let domains = [Term::Primitive(P::Type)];
    let arguments = [Term::Primitive(P::Bool)];
    let layout = [rigid(1, Sort::Type)];
    let mut fuel = 2;
    assert_eq!(
        fields(&parameters, &domains, &arguments, layout.iter(), &mut fuel),
        Err(Error::Budget)
    );
    assert_eq!(fuel, 2);
    assert!(matches!(
        fields(&parameters, &domains, &[], layout.iter(), &mut 1000),
        Err(Error::Invalid(_))
    ));
    assert_eq!(
        fields(&parameters, &domains, &arguments, layout.iter(), &mut 1000),
        Ok(vec![Term::Primitive(P::Bool)])
    );
}

#[test]
fn nominal_layout_substitution_preserves_value_indices_and_opaque_heads() {
    let record = record(vec![Term::Primitive(P::Type), Term::Primitive(P::Bool)]);
    let fields = record
        .instantiate_fields(&[Term::Primitive(P::U8), Term::Bool(true)])
        .unwrap();
    assert_eq!(fields[0].1, Term::Primitive(P::U8));
    assert_eq!(
        fields[1].1,
        Term::NormalForm {
            sort: Sort::Type,
            head: id(3),
            arguments: vec![Term::Bool(true)]
        }
    );
    let other = record
        .instantiate_fields(&[Term::Primitive(P::U8), Term::Bool(false)])
        .unwrap();
    assert_ne!(fields[1].1, other[1].1);
    let text = record.to_string();
    assert!(text.contains("(parameter-types"), "{text}");
    assert!(text.contains("(fields"), "{text}");
}

#[test]
fn nominal_domains_can_depend_on_earlier_type_parameters() {
    let record = record(vec![Term::Primitive(P::Type), rigid(1, Sort::Type)]);
    let binders = record.parameter_binders().unwrap();
    assert_eq!(binders[0].sort, Sort::Type);
    assert_eq!(binders[1].sort, Sort::ComptimeValue);
    assert!(
        record
            .instantiate_fields(&[Term::Primitive(P::Bool), Term::Bool(true)])
            .is_ok()
    );
    assert!(matches!(
        record.instantiate_fields(&[Term::Primitive(P::Bool), Term::Primitive(P::Bool)]),
        Err(TypeError::WrongSort { .. })
    ));
}

#[test]
fn nominal_domains_reject_malformed_binder_metadata() {
    assert!(matches!(
        record(vec![Term::Primitive(P::Type)]).parameter_binders(),
        Err(TypeError::ArgumentCount { .. })
    ));
    assert!(matches!(
        record(vec![Term::Primitive(P::Type), Term::Bool(true)]).parameter_binders(),
        Err(TypeError::WrongSort { .. })
    ));
    let mut duplicate = record(vec![]);
    duplicate.type_parameters[1] = id(1);
    assert!(matches!(
        duplicate.parameter_binders(),
        Err(TypeError::DuplicateDeclarationParameter(_))
    ));
}

#[test]
fn enum_payload_substitution_uses_the_same_value_binders() {
    let fields = record(vec![Term::Primitive(P::Type), Term::Primitive(P::Bool)]);
    let enumeration = Enum {
        parameter_types: fields.parameter_types,
        type_parameters: fields.type_parameters,
        declaration: id(4),
        name: "Choice".into(),
        capabilities: vec![],
        requirements: vec![],
        variants: vec![Variant {
            name: "some".into(),
            shape: VariantShape::Tuple,
            fields: fields.fields.into_iter().map(|(_, ty)| ty).collect(),
        }],
        groups: vec![],
        encoding: None,
        is_intrinsic: false,
        symbol_encoding: None,
    };
    let variants = enumeration
        .instantiate_variants(&[Term::Primitive(P::Bool), Term::Bool(false)])
        .unwrap();
    assert_eq!(
        variants[0].fields[1],
        Term::NormalForm {
            sort: Sort::Type,
            head: id(3),
            arguments: vec![Term::Bool(false)]
        }
    );
    assert!(enumeration.to_string().contains("(parameter-types"));
    assert!(
        enumeration
            .instantiate_variants(&[Term::Bool(false), Term::Bool(false)])
            .is_err()
    );
}

#[test]
fn legacy_type_only_nominals_and_open_scope_substitution_remain_distinct() {
    let mut record = record(vec![]);
    record.fields = vec![("value".into(), rigid(1, Sort::Type))];
    let argument = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let arguments = [argument.clone(), Term::Primitive(P::Bool)];
    assert!(record.instantiate_fields(&arguments).is_err());
    let fields = record
        .instantiate_fields_in_scope(&arguments, &[vec![Sort::Type]])
        .unwrap();
    assert_eq!(fields[0].1, argument);
    assert!(
        record
            .instantiate_fields(&[Term::Primitive(P::Bool), Term::Bool(false)])
            .is_err()
    );
}

#[test]
fn phase_validation_checks_unused_nominal_parameter_domains() {
    let mut package = nir::program::Package::default();
    package.structs.push(record(vec![Term::Primitive(P::Type)]));
    assert_eq!(
        nir::phases::validate(&package).unwrap_err().message,
        "invalid declaration parameter domains"
    );
    package.structs[0].parameter_types = vec![Term::Primitive(P::Type), Term::Primitive(P::Bool)];
    nir::phases::validate(&package).unwrap();
}

#[test]
fn completed_nominal_indices_check_declared_domains_not_just_sorts() {
    let nominal = record(vec![Term::Primitive(P::Type), rigid(1, Sort::Type)]);
    nominal
        .validate_completed_arguments(&[Term::Primitive(P::Bool), Term::Bool(true)], &mut 10000)
        .unwrap();
    assert!(
        nominal
            .validate_completed_arguments(&[Term::Primitive(P::Char), Term::Bool(true)], &mut 10000)
            .is_err()
    );
    assert!(
        nominal
            .validate_completed_arguments(
                &[Term::Primitive(P::Bool), rigid(2, Sort::ComptimeValue)],
                &mut 10000
            )
            .is_err()
    );
    assert!(
        nominal
            .validate_completed_arguments(&[Term::Primitive(P::Bool), Term::Bool(true)], &mut 0)
            .is_err()
    );
}

#[test]
fn completed_nominal_integer_indices_must_fit_without_truncation() {
    let nominal = record(vec![Term::Primitive(P::Type), Term::Primitive(P::U8)]);
    for (input, accepted) in [(0, true), (255, true), (256, false), (-1, false)] {
        let arguments = [
            Term::Primitive(P::Bool),
            Term::Integer(comptime_runtime::Integer::from_i128(input)),
        ];
        assert_eq!(
            nominal
                .validate_completed_arguments(&arguments, &mut 10000)
                .is_ok(),
            accepted
        );
    }
}

#[test]
fn synthetic_impls_share_container_binders_only_with_identical_domains() {
    let declaration = record(vec![Term::Primitive(P::Type), Term::Primitive(P::Bool)]);
    let implementation = nir::program::Implementation {
        phases: Default::default(),
        associated_initializers: vec![],
        parameter_types: declaration.parameter_types.clone(),
        type_parameters: declaration.type_parameters.clone(),
        declaration: id(10),
        subject: Term::Nominal {
            declaration: declaration.declaration,
            arguments: vec![rigid(1, Sort::Type), rigid(2, Sort::ComptimeValue)],
        },
        interface: None,
        interface_arguments: vec![],
        capabilities: vec![],
        requirements: vec![],
        methods: vec![],
        associated: vec![],
    };
    let mut package = nir::program::Package {
        structs: vec![declaration],
        implementations: vec![implementation],
        ..Default::default()
    };
    let sorts = package.declaration_parameter_sorts(&mut 1000).unwrap();
    assert_eq!(sorts[&id(1)], Sort::Type);
    assert_eq!(sorts[&id(2)], Sort::ComptimeValue);
    assert!(package.declaration_parameter_sorts(&mut 0).is_err());
    package.implementations[0].parameter_types[1] = Term::Primitive(P::Char);
    assert!(
        package
            .declaration_parameter_sorts(&mut 1000)
            .unwrap_err()
            .contains("conflicting")
    );
    package.implementations[0].parameter_types[1] = Term::Primitive(P::Bool);
    assert_eq!(
        package.declaration_parameter_sorts(&mut 1000).unwrap(),
        sorts
    );
}

#[test]
fn completed_nominal_value_layouts_check_domains_even_without_payloads() {
    let mut declaration = record(vec![Term::Primitive(P::Type), Term::Primitive(P::Bool)]);
    declaration.fields.clear();
    let package = nir::program::Package {
        structs: vec![declaration],
        ..Default::default()
    };
    let nominal = |index| Term::Nominal {
        declaration: id(0),
        arguments: vec![Term::Primitive(P::U8), index],
    };
    assert_eq!(
        nir::normal_forms::nominal_value_fields(&package, &nominal(Term::Bool(true)), None),
        Some(vec![])
    );
    assert_eq!(
        nir::normal_forms::nominal_value_fields(&package, &nominal(Term::Char('x')), None),
        None
    );
    assert_eq!(
        nir::normal_forms::nominal_value_fields(&package, &nominal(Term::Primitive(P::Bool)), None),
        None
    );
    assert!(
        nir::normal_forms::nominal_value_fields_budgeted(
            &package,
            &nominal(Term::Bool(true)),
            None,
            &mut 0
        )
        .is_none()
    );
    assert_eq!(
        nir::normal_forms::nominal_value_fields(&package, &nominal(Term::Bool(false)), None),
        Some(vec![])
    );
}

#[test]
fn nominal_field_projection_preserves_abstract_value_binders() {
    let declaration = record(vec![Term::Primitive(P::Type), Term::Primitive(P::Bool)]);
    let fields = nir::normal_forms::instantiate_value_fields(
        &declaration.type_parameters,
        &declaration.parameter_types,
        &[rigid(10, Sort::Type), rigid(11, Sort::ComptimeValue)],
        declaration.fields.iter().map(|(_, ty)| ty),
        &mut 1000,
    )
    .unwrap();
    assert_eq!(fields[0], rigid(10, Sort::Type));
    assert_eq!(
        fields[1],
        Term::NormalForm {
            head: id(3),
            sort: Sort::Type,
            arguments: vec![rigid(11, Sort::ComptimeValue)]
        }
    );
}

#[test]
fn admission_budget_failures_are_not_domain_failures() {
    use nir::{
        NominalAdmissionError as AdmissionError,
        validate_declaration_arguments as validate_completed,
    };
    let name = "long_record_field_name".to_string();
    let domains = [Term::Record(vec![(name.clone(), Term::Primitive(P::Bool))])];
    let arguments = [Term::RecordValue(vec![(name, Term::Bool(true))])];
    let mut exhausted = false;
    let mut completed = false;
    for budget in 0..512 {
        match validate_completed(&[id(1)], &domains, &arguments, &mut { budget }) {
            Ok(()) => completed = true,
            Err(AdmissionError::Budget) => exhausted = true,
            Err(error) => panic!("valid input failed at budget {budget}: {error}"),
        }
    }
    assert!(exhausted && completed);
    assert!(matches!(
        validate_completed(&[], &[], &arguments, &mut 0),
        Err(AdmissionError::Invalid(_))
    ));
}

#[test]
fn package_parameter_domains_preserve_dependent_telescope_evidence() {
    let package = nir::program::Package {
        structs: vec![record(vec![Term::Primitive(P::Type), rigid(1, Sort::Type)])],
        ..Default::default()
    };
    let domains = package.declaration_parameter_domains(&mut 1000).unwrap();
    assert_eq!(domains[&id(1)], Term::Primitive(P::Type));
    assert_eq!(domains[&id(2)], rigid(1, Sort::Type));
    assert!(package.declaration_parameter_domains(&mut 0).is_err());
    assert_eq!(
        package.declaration_parameter_domains(&mut 1000).unwrap(),
        domains
    );
    let sorts = package.declaration_parameter_sorts(&mut 1000).unwrap();
    assert_eq!(sorts[&id(1)], Sort::Type);
    assert_eq!(sorts[&id(2)], Sort::ComptimeValue);
}

#[test]
fn newtype_and_alias_domains_include_shared_captured_inputs() {
    let mut package = nir::program::Package {
        structs: vec![record(vec![Term::Primitive(P::Type), rigid(1, Sort::Type)])],
        newtypes: vec![nir::newtypes::Newtype {
            declaration: id(30),
            name: "Member".into(),
            capture_count: 2,
            type_parameters: vec![id(3), id(1), id(2)],
            parameter_types: vec![
                Term::Primitive(P::Bool),
                Term::Primitive(P::Type),
                rigid(1, Sort::Type),
            ],
            representation: Term::Primitive(P::Bool),
            requirements: vec![],
            capabilities: vec![],
        }],
        alias_contracts: vec![nir::type_aliases::Contract {
            declaration: id(40),
            parameters: vec![id(1), id(2), id(4)],
            parameter_types: vec![
                Term::Primitive(P::Type),
                rigid(1, Sort::Type),
                Term::Primitive(P::U8),
            ],
            requirements: vec![],
            capabilities: vec![],
        }],
        ..Default::default()
    };
    let domains = package.declaration_parameter_domains(&mut 1000).unwrap();
    assert_eq!(domains.get(&id(3)), Some(&Term::Primitive(P::Bool)));
    assert_eq!(domains.get(&id(4)), Some(&Term::Primitive(P::U8)));
    assert_eq!(domains.get(&id(2)), Some(&rigid(1, Sort::Type)));
    package.newtypes[0].parameter_types[2] = Term::Primitive(P::Char);
    assert!(package.declaration_parameter_domains(&mut 1000).is_err());
    package.newtypes[0].parameter_types[2] = rigid(1, Sort::Type);
    package.alias_contracts[0].parameter_types[1] = Term::Primitive(P::Bool);
    assert!(package.declaration_parameter_domains(&mut 1000).is_err());
    package.alias_contracts[0].parameter_types[1] = rigid(1, Sort::Type);
    assert_eq!(
        package.declaration_parameter_domains(&mut 1000).unwrap(),
        domains
    );
}

#[test]
fn nominal_payload_admission_requires_layout_and_checks_every_field() {
    use nir::{NominalAdmissionError as E, validate_declaration_arguments_with_layouts as admit};
    let ty = Term::Nominal {
        declaration: id(70),
        arguments: vec![Term::Bool(true)],
    };
    let value = Term::NominalValue {
        ty: Box::new(ty.clone()),
        variant: None,
        fields: vec![Term::Bool(true)],
    };
    let check = |value: Term, domain: Term, budget: usize| {
        admit(
            &[id(1)],
            &[domain],
            &[value],
            &mut { budget },
            |actual, variant, fuel| {
                *fuel = fuel.checked_sub(1).ok_or(E::Budget)?;
                if actual != &ty || variant.is_some() {
                    return Err(E::Invalid("unknown layout".into()));
                }
                Ok(vec![Term::Primitive(P::Bool)])
            },
        )
    };
    assert!(check(value.clone(), ty.clone(), 1000).is_ok());
    assert!(
        nir::validate_declaration_arguments(&[id(1)], &[ty.clone()], &[value.clone()], &mut 1000)
            .is_err()
    );
    for forged in [
        Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant: Some(0),
            fields: vec![Term::Bool(true)],
        },
        Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant: None,
            fields: vec![],
        },
        Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant: None,
            fields: vec![Term::Char('x')],
        },
        Term::NominalValue {
            ty: Box::new(Term::Nominal {
                declaration: id(70),
                arguments: vec![Term::Bool(false)],
            }),
            variant: None,
            fields: vec![Term::Bool(true)],
        },
    ] {
        assert!(matches!(
            check(forged, ty.clone(), 1000),
            Err(E::Invalid(_))
        ));
    }
    let domain = Term::Record(vec![("nested".into(), Term::Tuple(vec![ty.clone()]))]);
    let nested = Term::RecordValue(vec![(
        "nested".into(),
        Term::TupleValue(vec![value.clone()]),
    )]);
    assert!(check(nested.clone(), domain.clone(), 1000).is_ok());
    let mut succeeded = false;
    for budget in 0..1000 {
        match check(nested.clone(), domain.clone(), budget) {
            Ok(()) => {
                succeeded = true;
                break;
            }
            Err(E::Budget) => {}
            Err(error) => panic!("{budget}: {error}"),
        }
    }
    assert!(succeeded);
}

#[test]
fn layout_lookup_preserves_exhaustion_evidence_for_valid_indices() {
    let mut package = nir::program::Package {
        structs: vec![record(vec![
            Term::Primitive(P::Type),
            Term::Primitive(P::U8),
        ])],
        newtypes: vec![nir::newtypes::Newtype {
            declaration: id(5),
            name: "Captured".into(),
            type_parameters: vec![id(6), id(7)],
            parameter_types: vec![rigid(7, Sort::Type), Term::Primitive(P::Type)],
            capture_count: 1,
            representation: Term::Primitive(P::Bool),
            requirements: vec![],
            capabilities: vec![],
        }],
        ..Default::default()
    };
    let inner = Term::Nominal {
        declaration: id(5),
        arguments: vec![Term::Bool(true), Term::Primitive(P::Bool)],
    };
    package.structs.push(Struct {
        declaration: id(10),
        name: "Nested".into(),
        type_parameters: vec![id(11)],
        parameter_types: vec![inner.clone()],
        fields: vec![],
        requirements: vec![],
        capabilities: vec![],
    });
    let mut invalid_fuel = 2000;
    let invalid = Term::Nominal {
        declaration: id(5),
        arguments: vec![Term::Char('x'), Term::Primitive(P::Bool)],
    };
    assert!(
        nir::normal_forms::nominal_value_fields_budgeted(
            &package,
            &invalid,
            None,
            &mut invalid_fuel
        )
        .is_none()
    );
    assert!(invalid_fuel > 0);
    for ty in [
        Term::Nominal {
            declaration: id(10),
            arguments: vec![Term::NominalValue {
                ty: Box::new(inner),
                variant: None,
                fields: vec![Term::Bool(true)],
            }],
        },
        Term::Nominal {
            declaration: id(0),
            arguments: vec![
                Term::Primitive(P::Bool),
                Term::Integer(comptime_runtime::Integer::from_u128(255)),
            ],
        },
        Term::Nominal {
            declaration: id(5),
            arguments: vec![Term::Bool(true), Term::Primitive(P::Bool)],
        },
    ] {
        let mut succeeded = false;
        for budget in 0..2000 {
            let mut fuel = budget;
            match nir::normal_forms::nominal_value_fields_budgeted(&package, &ty, None, &mut fuel) {
                Some(_) => {
                    succeeded = true;
                    break;
                }
                None => assert_eq!(fuel, 0, "valid layout reported invalid at budget {budget}"),
            }
        }
        assert!(succeeded);
    }
}

#[test]
fn symbolic_tuple_indices_require_registered_layout_and_leaf_evidence() {
    let boolean = Term::Primitive(P::Bool);
    let tuple = Term::NormalForm {
        head: id(70),
        sort: Sort::Type,
        arguments: vec![frozen_types(vec![boolean.clone(), boolean.clone()])],
    };
    let symbolic = rigid(71, Sort::ComptimeValue);
    let check = |domain: &Term, fields: Vec<Term>, registered: bool, evidence: bool, budget| {
        nir::validate_declaration_arguments_with_evidence(
            &[id(72)],
            &[domain.clone()],
            &[Term::TupleValue(fields)],
            &mut { budget },
            |requested, variant, _| {
                if registered && requested == &tuple && variant.is_none() {
                    Ok(vec![boolean.clone(), boolean.clone()])
                } else {
                    Err(nir::NominalAdmissionError::Invalid(
                        "tuple layout unavailable".into(),
                    ))
                }
            },
            |value, domain, _| evidence && value == &symbolic && domain == &boolean,
        )
    };
    let fields = vec![symbolic.clone(), Term::Bool(false)];
    check(&tuple, fields.clone(), true, true, 1000).unwrap();
    assert!(check(&tuple, fields.clone(), false, true, 1000).is_err());
    assert!(check(&tuple, fields.clone(), true, false, 1000).is_err());
    assert!(check(&tuple, vec![symbolic.clone()], true, true, 1000).is_err());
    assert!(
        check(
            &tuple,
            vec![symbolic.clone(), Term::Primitive(P::Bool)],
            true,
            true,
            1000
        )
        .is_err()
    );
    let mut unrelated = tuple.clone();
    if let Term::NormalForm { head, .. } = &mut unrelated {
        *head = id(73);
    }
    assert!(check(&unrelated, fields.clone(), true, true, 1000).is_err());
    assert!(matches!(
        check(&tuple, fields, true, true, 0),
        Err(nir::NominalAdmissionError::Budget)
    ));
}

#[test]
fn platform_index_admission_preserves_recursive_target_and_budget() {
    let parameters = [id(1)];
    let domains = [Term::Record(vec![(
        "count".into(),
        Term::Primitive(P::Usize),
    )])];
    let arguments = [Term::RecordValue(vec![(
        "count".into(),
        Term::Integer(comptime_runtime::Integer::from_u128(1u128 << 32)),
    )])];
    for (target, accepted) in [
        (None, false),
        (Some(nir::target::TargetData::new(32).unwrap()), false),
        (Some(nir::target::TargetData::new(64).unwrap()), true),
    ] {
        assert!(matches!(
            nir::validate_declaration_arguments_in(
                &parameters,
                &domains,
                &arguments,
                &mut 0,
                target
            ),
            Err(nir::NominalAdmissionError::Budget)
        ));
        assert_eq!(
            nir::validate_declaration_arguments_in(
                &parameters,
                &domains,
                &arguments,
                &mut 1000,
                target
            )
            .is_ok(),
            accepted
        );
    }
    assert!(
        nir::validate_declaration_arguments(&parameters, &domains, &arguments, &mut 1000).is_err()
    );
}

#[test]
fn object_indices_validate_nested_domains_without_nominal_layout_guessing() {
    let domain = Term::Primitive(P::Object);
    let good = nir::objects::value(vec![nir::objects::entry(
        Some("flag"),
        Term::Primitive(P::Bool),
        Term::Bool(true),
    )]);
    nir::validate_declaration_arguments(
        &[id(1)],
        std::slice::from_ref(&domain),
        std::slice::from_ref(&good),
        &mut 10000,
    )
    .unwrap();
    let bad = nir::objects::value(vec![nir::objects::entry(
        Some("flag"),
        Term::Primitive(P::Bool),
        Term::Char('x'),
    )]);
    assert!(
        nir::validate_declaration_arguments(
            &[id(1)],
            std::slice::from_ref(&domain),
            &[bad],
            &mut 10000
        )
        .is_err()
    );
    assert!(nir::validate_declaration_arguments(&[id(1)], &[domain], &[good], &mut 1).is_err());
}

#[test]
fn type_valued_dependent_indices_substitute_all_nominal_layouts() {
    let record = record(vec![Term::Primitive(P::Type), rigid(1, Sort::Type)]);
    let arguments = [Term::Primitive(P::Type), Term::Primitive(P::Bool)];
    let fields = record.instantiate_fields(&arguments).unwrap();
    assert_eq!(
        fields[1].1,
        Term::NormalForm {
            head: id(3),
            sort: Sort::Type,
            arguments: vec![Term::Primitive(P::Bool)],
        }
    );
    let enumeration = Enum {
        declaration: id(4),
        name: "IndexedChoice".into(),
        type_parameters: record.type_parameters.clone(),
        parameter_types: record.parameter_types.clone(),
        requirements: vec![],
        capabilities: vec![],
        variants: vec![Variant {
            name: "some".into(),
            shape: VariantShape::Tuple,
            fields: record.fields.iter().map(|(_, ty)| ty.clone()).collect(),
        }],
        groups: vec![],
        encoding: None,
        is_intrinsic: false,
        symbol_encoding: None,
    };
    assert_eq!(
        enumeration.instantiate_variants(&arguments).unwrap()[0].fields[1],
        fields[1].1
    );
    let wrapper = nir::newtypes::Newtype {
        declaration: id(5),
        name: "IndexedWrapper".into(),
        type_parameters: record.type_parameters.clone(),
        parameter_types: record.parameter_types.clone(),
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation: record.fields[1].1.clone(),
    };
    assert_eq!(
        wrapper
            .representation_budgeted(&arguments, &[], &mut 10000)
            .unwrap(),
        fields[1].1
    );
    assert!(
        wrapper
            .representation_budgeted(&arguments, &[], &mut 0)
            .is_err()
    );
    let layout = nir::normal_forms::nominal_value_fields_with_lookup(
        &Term::Nominal {
            declaration: record.declaration,
            arguments: arguments.to_vec(),
        },
        None,
        &mut 10000,
        &|_, _| Some(nir::normal_forms::NominalValueDeclaration::Struct(&record)),
    )
    .unwrap();
    assert_eq!(
        layout,
        fields.into_iter().map(|(_, ty)| ty).collect::<Vec<_>>()
    );
    for wrong in [
        [Term::Primitive(P::Type), Term::Bool(true)],
        [Term::Primitive(P::Bool), Term::Primitive(P::Bool)],
    ] {
        assert!(record.instantiate_fields(&wrong).is_err());
        assert!(enumeration.instantiate_variants(&wrong).is_err());
        assert!(
            wrapper
                .representation_budgeted(&wrong, &[], &mut 10000)
                .is_err()
        );
    }
}

#[test]
fn nominal_collection_layouts_use_registered_domains_and_capture_order() {
    use nir::normal_forms::{Head, Kind, Parameter, Role, nominal_value_fields_budgeted as fields};
    let list = |element| Term::NormalForm {
        head: id(90),
        sort: Sort::Type,
        arguments: vec![element],
    };
    let domain = list(Term::Primitive(P::Bool));
    let value = Term::NominalValue {
        ty: Box::new(domain.clone()),
        variant: None,
        fields: vec![Term::Bool(true)],
    };
    // The first input depends on a later captured Type input. Stored order
    // remains unchanged even though newtype admission processes captures first.
    let domains = vec![list(rigid(2, Sort::Type)), Term::Primitive(P::Type)];
    let mut structure = record(domains.clone());
    structure.fields = vec![("value".into(), rigid(2, Sort::Type))];
    let mut package = nir::program::Package {
        normal_forms: vec![Head {
            declaration: id(90),
            role: Role::TypeList,
            collector: None,
            parameters: vec![Parameter {
                declaration: id(91),
                binding: nir::program::LocalId {
                    owner: 90,
                    index: 0,
                },
                kind: Kind::Type,
                ty: Term::Primitive(P::Type),
            }],
            requirements: vec![],
        }],
        structs: vec![structure],
        enums: vec![Enum {
            declaration: id(10),
            name: "Choice".into(),
            type_parameters: vec![id(1), id(2)],
            parameter_types: domains.clone(),
            capabilities: vec![],
            requirements: vec![],
            is_intrinsic: false,
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
            variants: vec![Variant {
                name: "value".into(),
                shape: VariantShape::Tuple,
                fields: vec![rigid(2, Sort::Type)],
            }],
        }],
        newtypes: vec![nir::newtypes::Newtype {
            declaration: id(20),
            name: "Wrapped".into(),
            type_parameters: vec![id(1), id(2)],
            parameter_types: domains,
            capture_count: 1,
            requirements: vec![],
            capabilities: vec![],
            representation: rigid(2, Sort::Type),
        }],
        ..Default::default()
    };
    for (declaration, variant) in [(0, None), (10, Some(0)), (20, None)] {
        let ty = |value| Term::Nominal {
            declaration: id(declaration),
            arguments: vec![value, Term::Primitive(P::Bool)],
        };
        let valid = ty(value.clone());
        assert_eq!(
            fields(&package, &valid, variant, &mut 10000),
            Some(vec![Term::Primitive(P::Bool)])
        );
        for invalid in [
            Term::NominalValue {
                ty: Box::new(domain.clone()),
                variant: None,
                fields: vec![Term::Unit],
            },
            Term::NominalValue {
                ty: Box::new(list(Term::Primitive(P::U8))),
                variant: None,
                fields: vec![],
            },
            Term::NominalValue {
                ty: Box::new(domain.clone()),
                variant: Some(0),
                fields: vec![],
            },
        ] {
            assert!(fields(&package, &ty(invalid), variant, &mut 10000).is_none());
        }
        assert!(fields(&package, &valid, variant, &mut 0).is_none());
        package.normal_forms[0].role = Role::Opaque;
        assert!(fields(&package, &valid, variant, &mut 10000).is_none());
        package.normal_forms[0].role = Role::TypeList;
        assert!(fields(&package, &valid, variant, &mut 10000).is_some());
        package.normal_forms.push(package.normal_forms[0].clone());
        assert!(fields(&package, &valid, variant, &mut 10000).is_none());
        package.normal_forms.pop();
    }
    // Recursing into a nominal value must retain the same collection evidence.
    let inner = Term::Nominal {
        declaration: id(0),
        arguments: vec![value.clone(), Term::Primitive(P::Bool)],
    };
    package.structs.push(Struct {
        declaration: id(30),
        name: "Outer".into(),
        type_parameters: vec![id(31)],
        parameter_types: vec![inner.clone()],
        fields: vec![],
        requirements: vec![],
        capabilities: vec![],
    });
    let outer = |payload| Term::Nominal {
        declaration: id(30),
        arguments: vec![Term::NominalValue {
            ty: Box::new(inner.clone()),
            variant: None,
            fields: vec![payload],
        }],
    };
    assert_eq!(
        fields(&package, &outer(Term::Bool(false)), None, &mut 10000),
        Some(vec![])
    );
    assert!(fields(&package, &outer(Term::Unit), None, &mut 10000).is_none());
    let valid = Term::Nominal {
        declaration: id(0),
        arguments: vec![value, Term::Primitive(P::Bool)],
    };
    assert!(
        nir::normal_forms::nominal_value_fields_with_lookup(
            &valid,
            None,
            &mut 10000,
            &|_, _| Some(nir::normal_forms::NominalValueDeclaration::Struct(
                &package.structs[0]
            )),
        )
        .is_none()
    );
}

#[test]
fn stored_fields_share_bounded_substitution_and_preserve_nested_scope() {
    use nir::type_computations::Error;
    let mut definition = record(vec![Term::Primitive(P::Type), Term::Primitive(P::Bool)]);
    definition.fields.push((
        "callback".into(),
        Term::Function {
            parameters: vec![],
            result: Box::new(rigid(1, Sort::Type)),
        },
    ));
    let outer = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let arguments = [outer.clone(), Term::Bool(false)];
    let frames = [vec![Sort::Type]];
    let expected = vec![
        outer,
        Term::NormalForm {
            head: id(3),
            sort: Sort::Type,
            arguments: vec![Term::Bool(false)],
        },
        Term::Function {
            parameters: vec![],
            result: Box::new(Term::Bound {
                depth: 1,
                index: 0,
                sort: Sort::Type,
            }),
        },
    ];
    let mut completed = false;
    for mut allowance in 0..1000 {
        match definition.instantiate_stored_fields(&arguments, &frames, &mut allowance) {
            Err(Error::Budget) => {}
            Ok(fields) => {
                assert_eq!(fields, expected);
                completed = true;
                break;
            }
            Err(error) => panic!("valid scoped fields rejected: {error}"),
        }
    }
    assert!(completed);
    assert!(matches!(
        definition.instantiate_stored_fields(&arguments, &[], &mut 1000),
        Err(Error::Scope(_))
    ));
    let mut fuel = 1000;
    definition
        .instantiate_stored_fields(&arguments, &frames, &mut fuel)
        .unwrap();
    for (name, _) in &mut definition.fields {
        *name = "unused-label".repeat(1000);
    }
    let mut renamed_fuel = 1000;
    assert_eq!(
        definition
            .instantiate_stored_fields(&arguments, &frames, &mut renamed_fuel)
            .unwrap(),
        expected
    );
    assert_eq!(
        fuel, renamed_fuel,
        "storage queries must not copy discarded field labels"
    );
    definition.fields.clear();
    assert!(matches!(
        definition.instantiate_stored_fields(&arguments[..1], &frames, &mut 1000),
        Err(Error::Scope(_))
    ));
    definition
        .instantiate_stored_fields(&arguments, &frames, &mut 1000)
        .unwrap();
}

#[test]
fn all_variant_storage_fields_charge_empty_alternatives_and_validate_empty_frames() {
    use nir::type_computations::Error;
    let mut definition = Enum {
        declaration: id(4),
        name: "Choice".into(),
        type_parameters: vec![id(1)],
        parameter_types: vec![Term::Primitive(P::Type)],
        requirements: vec![],
        capabilities: vec![],
        groups: vec![],
        encoding: None,
        symbol_encoding: None,
        is_intrinsic: false,
        variants: (0..128)
            .map(|_| Variant {
                name: "empty".into(),
                shape: VariantShape::Unit,
                fields: vec![],
            })
            .collect(),
    };
    let args = [Term::Primitive(P::Bool)];
    let mut fuel = 16;
    assert_eq!(
        definition.instantiate_stored_fields(&args, &[], &mut fuel),
        Err(Error::Budget)
    );
    assert_eq!(fuel, 16);
    assert_eq!(
        definition
            .instantiate_stored_fields(&args, &[], &mut 1000)
            .unwrap(),
        vec![]
    );
    assert!(matches!(
        definition.instantiate_stored_fields(&[], &[], &mut 1000),
        Err(Error::Scope(_))
    ));
    definition.variants.push(Variant {
        name: "payload".into(),
        shape: VariantShape::Tuple,
        fields: vec![Term::Pointer(Box::new(rigid(1, Sort::Type)))],
    });
    assert_eq!(
        definition
            .instantiate_stored_fields(&args, &[], &mut 1000)
            .unwrap(),
        vec![Term::Pointer(Box::new(Term::Primitive(P::Bool)))]
    );
    let mut fuel = 1000;
    definition
        .instantiate_stored_fields(&args, &[], &mut fuel)
        .unwrap();
    for variant in &mut definition.variants {
        variant.name = "unused-label".repeat(1000);
    }
    let mut renamed_fuel = 1000;
    definition
        .instantiate_stored_fields(&args, &[], &mut renamed_fuel)
        .unwrap();
    assert_eq!(fuel, renamed_fuel);
    let outer = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let groups = definition
        .instantiate_variant_fields(&[outer.clone()], &[vec![Sort::Type]], &mut 1000)
        .unwrap();
    assert_eq!(groups.len(), 129);
    assert!(groups[..128].iter().all(Vec::is_empty));
    assert_eq!(groups[128], vec![Term::Pointer(Box::new(outer))]);
    assert!(matches!(
        definition.instantiate_variant_fields(&args, &[], &mut 16),
        Err(Error::Budget)
    ));
    assert!(matches!(
        definition.instantiate_variant_fields(&[], &[], &mut 1000),
        Err(Error::Scope(_))
    ));
    assert!(matches!(
        definition.instantiate_variant_fields(
            &[Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type
            }],
            &[],
            &mut 1000,
        ),
        Err(Error::Scope(_))
    ));
    definition
        .instantiate_variant_fields(&args, &[], &mut 1000)
        .unwrap();
}
