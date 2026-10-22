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

#[test]
fn nominal_contextual_inputs_rewrite_nested_types_and_keep_literal_spelling() {
    use nir::program::{Expr, ExprKind as K, NormalFormArgument as A, SourceRange};
    let source = SourceRange { start: 7, end: 9 };
    let body = Expr {
        source,
        ty: primitive(P::Type),
        kind: K::NominalType {
            preparations: vec![],
            bindings: None,
            order: Some(vec![0]),
            declaration: id(20),
            arguments: vec![A::Block {
                source,
                statements: vec![Expr {
                    source,
                    ty: primitive(P::Type),
                    kind: K::TypeValue(Term::Rigid {
                        declaration: id(1),
                        sort: Sort::Type,
                    }),
                }],
                result: Some(Box::new(A::Integer {
                    source,
                    spelling: "0xff".into(),
                    negative: false,
                })),
            }],
        },
    };
    let rewritten = body
        .instantiate_types(&[id(1)], &[primitive(P::U8)])
        .unwrap();
    let K::NominalType { arguments, .. } = &rewritten.kind else {
        panic!("nominal constructor")
    };
    let A::Block {
        statements, result, ..
    } = &arguments[0]
    else {
        panic!("contextual block")
    };
    assert_eq!(statements[0].kind, K::TypeValue(primitive(P::U8)));
    assert!(matches!(result.as_deref(),Some(A::Integer {spelling,..}) if spelling=="0xff"));
    assert_ne!(body, rewritten);
    assert!(rewritten.to_string().contains("(order 0)"));
    assert!(rewritten.to_string().contains("(contextual-integer-input"));
}

#[test]
fn nominal_domains_preserve_value_prefix_sorts_and_instances() {
    let domain = Term::Nominal {
        declaration: id(7),
        arguments: vec![Term::Rigid {
            declaration: id(1),
            sort: Sort::ComptimeValue,
        }],
    };
    let scoped = |prior: &[nir::DeclarationParameter]| {
        nir::normal_forms::supported_input_type_in_scope(
            &domain,
            prior,
            |_| false,
            |id_| id_ == id(7),
        )
    };
    assert!(!scoped(&[]));
    assert!(!scoped(&[nir::DeclarationParameter {
        declaration: id(1),
        sort: Sort::Type
    }]));
    assert!(scoped(&[nir::DeclarationParameter {
        declaration: id(1),
        sort: Sort::ComptimeValue
    }]));
    let descriptor = Head {
        role: nir::normal_forms::Role::Opaque,
        declaration: id(0),
        requirements: vec![],
        parameters: vec![
            parameter(1, Kind::Value, primitive(P::Bool)),
            parameter(2, Kind::Value, domain),
        ],
    };
    let object = |flag| Term::NominalValue {
        ty: Box::new(Term::Nominal {
            declaration: id(7),
            arguments: vec![Term::Bool(flag)],
        }),
        variant: None,
        fields: vec![Term::Bool(true)],
    };
    for (index, value, expected) in [
        (true, true, true),
        (false, false, true),
        (true, false, false),
        (false, true, false),
    ] {
        let result = descriptor.check_closed_arguments_with_layouts(
            &[Term::Bool(index), object(value)],
            |_| false,
            |_, variant| variant.is_none().then(|| vec![primitive(P::Bool)]),
        );
        assert_eq!(result.is_ok(), expected);
    }
}

#[test]
fn residual_normal_inputs_require_exact_domains_without_weakening_closed_admission() {
    let symbol = Term::Rigid {
        declaration: id(10),
        sort: Sort::ComptimeValue,
    };
    let descriptor = Head {
        role: nir::normal_forms::Role::Opaque,
        declaration: id(0),
        requirements: vec![],
        parameters: vec![
            parameter(1, Kind::Type, primitive(P::Type)),
            parameter(
                2,
                Kind::Value,
                record(
                    "flag",
                    Term::Rigid {
                        declaration: id(1),
                        sort: Sort::Type,
                    },
                ),
            ),
        ],
    };
    let inputs = vec![primitive(P::Bool), value("flag", symbol.clone())];
    let check = |inputs: &[Term], fuel: &mut usize| {
        descriptor.check_arguments_with_evidence_budgeted(
            inputs,
            |_| false,
            |_, _, _| None,
            |input, domain| input == &symbol && domain == &primitive(P::Bool),
            fuel,
        )
    };
    assert!(check(&inputs, &mut 10000).is_ok());
    assert!(descriptor.check_closed_arguments(&inputs).is_err());
    assert!(
        check(
            &[primitive(P::Char), value("flag", symbol.clone())],
            &mut 10000
        )
        .is_err()
    );
    let unknown = Term::Rigid {
        declaration: id(11),
        sort: Sort::ComptimeValue,
    };
    assert!(check(&[primitive(P::Bool), value("flag", unknown)], &mut 10000).is_err());
    assert!(check(&inputs, &mut 0).is_err());
    assert!(check(&inputs, &mut 10000).is_ok());
    // Domain authority never changes the sort of a declaration input.
    let types = Head {
        parameters: vec![parameter(1, Kind::Type, primitive(P::Type))],
        ..descriptor
    };
    assert!(
        types
            .check_arguments_with_evidence_budgeted(
                &[symbol],
                |_| false,
                |_, _, _| None,
                |_, _| true,
                &mut 10000
            )
            .is_err()
    );
}
