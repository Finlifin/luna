use nir::normal_forms::{ApplicationError, Head, Kind, Parameter, supported_input_type};
use nir::program::LocalId;
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn primitive(p: P) -> Term {
    Term::Primitive(p)
}
fn parameter(index: u32, kind: Kind, ty: Term) -> Parameter {
    Parameter {
        declaration: id(index),
        binding: LocalId { owner: 1, index },
        kind,
        ty,
    }
}
fn head(ty: Term) -> Head {
    Head {
        role: nir::normal_forms::Role::Opaque,
        declaration: id(0),
        parameters: vec![parameter(1, Kind::Value, ty)],
        requirements: vec![],
    }
}
fn record(name: &str, ty: Term) -> Term {
    Term::Record(vec![(name.into(), ty)])
}
fn value(name: &str, value: Term) -> Term {
    Term::RecordValue(vec![(name.into(), value)])
}

#[test]
fn symbol_normal_inputs_do_not_accept_strings_or_types() {
    let descriptor = head(primitive(P::Symbol));
    assert!(supported_input_type(&primitive(P::Symbol), &[]));
    assert!(
        descriptor
            .check_closed_arguments(&[Term::Symbol("ready".into())])
            .is_ok()
    );
    for invalid in [
        Term::String("ready".into()),
        primitive(P::Symbol),
        Term::Bool(true),
    ] {
        assert_eq!(
            descriptor.check_closed_arguments(&[invalid]),
            Err(ApplicationError::InputType { index: 0 })
        );
    }
    assert!(
        head(primitive(P::Str))
            .check_closed_arguments(&[Term::Symbol("ready".into())])
            .is_err()
    );
}

#[test]
fn nominal_domains_require_metadata_and_prior_type_inputs() {
    use nir::normal_forms::supported_input_type_with_layouts;
    let domain = Term::Nominal {
        declaration: id(7),
        arguments: vec![Term::Rigid {
            declaration: id(1),
            sort: Sort::Type,
        }],
    };
    assert!(!supported_input_type(&domain, &[id(1)]));
    assert!(!supported_input_type_with_layouts(
        &domain,
        &[],
        |_| false,
        |declaration| declaration == id(7)
    ));
    assert!(supported_input_type_with_layouts(
        &domain,
        &[id(1)],
        |_| false,
        |declaration| declaration == id(7)
    ));
    let descriptor = Head {
        role: nir::normal_forms::Role::Opaque,
        declaration: id(0),
        parameters: vec![
            parameter(1, Kind::Type, primitive(P::Type)),
            parameter(2, Kind::Value, domain),
        ],
        requirements: vec![],
    };
    let concrete = Term::Nominal {
        declaration: id(7),
        arguments: vec![primitive(P::Bool)],
    };
    let lookup = |ty: &Term, variant| {
        if *ty == concrete && variant == Some(0) {
            Some(vec![primitive(P::Bool)])
        } else {
            None
        }
    };
    let valid = Term::NominalValue {
        ty: Box::new(concrete.clone()),
        variant: Some(0),
        fields: vec![Term::Bool(true)],
    };
    assert!(
        descriptor
            .check_closed_arguments(&[primitive(P::Bool), valid.clone()])
            .is_err()
    );
    assert!(
        descriptor
            .check_closed_arguments_with_layouts(
                &[primitive(P::Bool), valid.clone()],
                |_| false,
                lookup
            )
            .is_ok()
    );
    assert!(
        descriptor
            .check_closed_arguments_with_layouts(
                &[primitive(P::F32), valid.clone()],
                |_| false,
                lookup
            )
            .is_err()
    );
    let invalid = Term::NominalValue {
        ty: Box::new(concrete.clone()),
        variant: Some(0),
        fields: vec![primitive(P::Bool)],
    };
    assert!(
        descriptor
            .check_closed_arguments_with_layouts(&[primitive(P::Bool), invalid], |_| false, lookup)
            .is_err()
    );
    assert!(
        descriptor
            .check_closed_arguments_with_layouts(&[primitive(P::Bool), valid], |_| false, lookup)
            .is_ok()
    );
}

#[test]
fn normal_form_input_budget_is_shared_across_the_telescope() {
    let descriptor = Head {
        role: nir::normal_forms::Role::Opaque,
        declaration: id(0),
        parameters: (1..=100)
            .map(|index| parameter(index, Kind::Value, primitive(P::Bool)))
            .collect(),
        requirements: vec![],
    };
    let arguments = vec![Term::Bool(true); 100];
    let mut low = 150;
    assert!(
        descriptor
            .check_closed_arguments_budgeted(&arguments, |_| false, |_, _, _| None, &mut low)
            .is_err()
    );
    assert!(low < 150);
    let mut enough = 1000;
    descriptor
        .check_closed_arguments_budgeted(&arguments, |_| false, |_, _, _| None, &mut enough)
        .unwrap();
    assert!(enough < 1000);
    let mut again = 1000;
    descriptor
        .check_closed_arguments_budgeted(&arguments, |_| false, |_, _, _| None, &mut again)
        .unwrap();
    assert_eq!(enough, again);
}

#[test]
fn record_normal_inputs_check_names_shape_and_nested_ranges() {
    let shape = record(
        "data",
        Term::Tuple(vec![primitive(P::U8), primitive(P::F32)]),
    );
    assert!(supported_input_type(&shape, &[]));
    let descriptor = head(shape);
    let fields = vec![
        Term::Integer(comptime_runtime::Integer::from_u128(255)),
        Term::Float32(0),
    ];
    let valid = value("data", Term::TupleValue(fields.clone()));
    assert!(descriptor.check_closed_arguments(&[valid.clone()]).is_ok());
    let mut overflow = fields.clone();
    overflow[0] = Term::Integer(comptime_runtime::Integer::from_u128(256));
    let mut wrong_width = fields.clone();
    wrong_width[1] = Term::Float64(0);
    for invalid in [
        value("other", Term::TupleValue(fields.clone())),
        value("data", Term::TupleValue(overflow)),
        value("data", Term::TupleValue(wrong_width)),
        Term::RecordValue(vec![]),
        Term::TupleValue(fields),
        primitive(P::Bool),
    ] {
        assert_eq!(
            descriptor.check_closed_arguments(&[invalid]),
            Err(ApplicationError::InputType { index: 0 })
        );
        assert!(descriptor.check_closed_arguments(&[valid.clone()]).is_ok());
    }
}

#[test]
fn record_normal_inputs_resolve_prior_type_arguments() {
    let variable = Term::Rigid {
        declaration: id(1),
        sort: Sort::Type,
    };
    let shape = record("payload", variable);
    assert!(!supported_input_type(&shape, &[]));
    assert!(supported_input_type(&shape, &[id(1)]));
    let descriptor = Head {
        role: nir::normal_forms::Role::Opaque,
        declaration: id(0),
        parameters: vec![
            parameter(1, Kind::Type, primitive(P::Type)),
            parameter(2, Kind::Value, shape),
        ],
        requirements: vec![],
    };
    assert!(
        descriptor
            .check_closed_arguments(&[primitive(P::Bool), value("payload", Term::Bool(true))])
            .is_ok()
    );
    assert_eq!(
        descriptor.check_closed_arguments(&[primitive(P::F32), value("payload", Term::Bool(true))]),
        Err(ApplicationError::InputType { index: 1 })
    );
}

#[test]
fn duplicate_and_oversized_record_labels_are_rejected_before_comparison() {
    let repeated = Term::Record(vec![("x".into(), primitive(P::Bool)); 2]);
    assert!(!supported_input_type(&repeated, &[]));
    assert!(
        head(repeated)
            .check_closed_arguments(&[Term::RecordValue(vec![("x".into(), Term::Bool(true)); 2])])
            .is_err()
    );
    let label = "x".repeat(100_001);
    assert!(!supported_input_type(
        &record(&label, primitive(P::Bool)),
        &[]
    ));
    assert!(
        head(record(&label, primitive(P::Bool)))
            .check_closed_arguments(&[value(&label, Term::Bool(true))])
            .is_err()
    );
    assert!(
        head(record("x", primitive(P::Bool)))
            .check_closed_arguments(&[value("x", Term::Bool(true))])
            .is_ok()
    );
}

#[test]
fn bound_tuple_value_domains_require_identity_and_check_nested_elements() {
    use nir::normal_forms::supported_input_type_with_tuple_heads;
    let tuple = |fields| Term::NormalForm {
        head: id(9),
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(fields)],
    };
    let shape = tuple(vec![primitive(P::Bool), tuple(vec![primitive(P::U8)])]);
    let trusted = |head| head == id(9);
    assert!(!supported_input_type(&shape, &[]));
    assert!(supported_input_type_with_tuple_heads(&shape, &[], trusted));
    let descriptor = head(shape.clone());
    let valid = Term::TupleValue(vec![
        Term::Bool(true),
        Term::TupleValue(vec![Term::Integer(comptime_runtime::Integer::from_u128(
            255,
        ))]),
    ]);
    assert!(descriptor.check_closed_arguments(&[valid.clone()]).is_err());
    assert!(
        descriptor
            .check_closed_arguments_with_tuple_heads(&[valid.clone()], trusted)
            .is_ok()
    );
    for bad in [
        Term::TupleValue(vec![]),
        Term::TupleValue(vec![
            Term::Bool(true),
            Term::TupleValue(vec![Term::Integer(comptime_runtime::Integer::from_u128(
                256,
            ))]),
        ]),
        Term::TupleValue(vec![
            Term::Bool(true),
            Term::TupleValue(vec![Term::Bool(false)]),
        ]),
    ] {
        assert!(
            descriptor
                .check_closed_arguments_with_tuple_heads(&[bad], trusted)
                .is_err()
        );
        assert!(
            descriptor
                .check_closed_arguments_with_tuple_heads(&[valid.clone()], trusted)
                .is_ok()
        );
    }
    let dependent = tuple(vec![Term::Rigid {
        declaration: id(7),
        sort: Sort::Type,
    }]);
    assert!(!supported_input_type_with_tuple_heads(
        &dependent,
        &[],
        trusted
    ));
    assert!(supported_input_type_with_tuple_heads(
        &dependent,
        &[id(7)],
        trusted
    ));
    assert!(!supported_input_type_with_tuple_heads(
        &tuple(vec![Term::Bool(true)]),
        &[],
        trusted
    ));
}

#[test]
fn exact_float_normal_inputs_do_not_accept_rounded_runtime_values() {
    let value = Term::Float(
        comptime_runtime::decimal::DecimalLiteral::parse(
            "0.10000000000000000000000000001",
            &mut Default::default(),
        )
        .unwrap()
        .exact_value(&mut Default::default())
        .unwrap(),
    );
    let descriptor = head(primitive(P::Float));
    assert!(supported_input_type(&primitive(P::Float), &[]));
    assert!(descriptor.check_closed_arguments(&[value.clone()]).is_ok());
    for wrong in [
        Term::Float64(0.1f64.to_bits()),
        Term::Float32(0.1f32.to_bits()),
        primitive(P::Float),
    ] {
        assert!(descriptor.check_closed_arguments(&[wrong]).is_err());
    }
    assert!(
        head(primitive(P::F64))
            .check_closed_arguments(&[value])
            .is_err()
    );
}
