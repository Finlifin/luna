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
        "invalid struct parameter domains"
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
