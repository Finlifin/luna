use nir::normal_forms::{ApplicationError, Head, Kind, Parameter, supported_input_type};
use nir::program::LocalId;
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn primitive(p: P) -> Term {
    Term::Primitive(p)
}
fn type_list(fields: Vec<Term>) -> Term {
    Term::NominalValue {
        ty: Box::new(Term::NormalForm {
            head: id(90),
            sort: Sort::Type,
            arguments: vec![primitive(P::Type)],
        }),
        variant: None,
        fields,
    }
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
        collector: None,
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
fn callable_metadata_abstracts_dependent_domains_and_checks_collectors() {
    let mut descriptor = Head {
        collector: Some(1),
        role: nir::normal_forms::Role::Opaque,
        declaration: id(0),
        parameters: vec![
            parameter(1, Kind::Type, primitive(P::Type)),
            parameter(
                2,
                Kind::Value,
                Term::Rigid {
                    declaration: id(1),
                    sort: Sort::Type,
                },
            ),
        ],
        requirements: vec![],
    };
    let callable = descriptor.callable_type().unwrap();
    let specialized = callable
        .bind_function_inputs(&[Some(primitive(P::Bool)), None])
        .unwrap();
    assert_eq!(
        specialized,
        Term::NormalFormFunction(
            Box::new(Term::Function {
                parameters: vec![nir::Parameter {
                    sort: Sort::ComptimeValue,
                    ty: primitive(P::Bool)
                }],
                result: Box::new(primitive(P::Type)),
            }),
            Some(0)
        )
    );
    assert!(descriptor.to_string().contains("(collector 1)"));
    for invalid in [0, 2, usize::MAX] {
        descriptor.collector = Some(invalid);
        assert!(descriptor.callable_type().is_err());
    }
    descriptor.collector = None;
    assert!(descriptor.callable_type().is_ok());
}

#[test]
fn dependent_list_parameters_use_the_same_value_contract_for_types_and_bools() {
    let list = |element| Term::NormalForm {
        head: id(90),
        sort: Sort::Type,
        arguments: vec![element],
    };
    let descriptor = Head {
        collector: None,
        role: nir::normal_forms::Role::Opaque,
        declaration: id(0),
        parameters: vec![
            parameter(1, Kind::Type, primitive(P::Type)),
            parameter(
                2,
                Kind::Value,
                list(Term::Rigid {
                    declaration: id(1),
                    sort: Sort::Type,
                }),
            ),
        ],
        requirements: vec![],
    };
    let check = |arguments: &[Term], trusted: bool, fuel: &mut usize| {
        descriptor.check_arguments_in_scope_with_collections_budgeted_in(
            arguments,
            |_| false,
            |head| trusted && head == id(90),
            |_, _, _| None,
            |_, _| false,
            &[],
            fuel,
            None,
        )
    };
    let types = vec![primitive(P::Type), type_list(vec![primitive(P::Bool)])];
    let bools = vec![
        primitive(P::Bool),
        Term::NominalValue {
            ty: Box::new(list(primitive(P::Bool))),
            variant: None,
            fields: vec![Term::Bool(true)],
        },
    ];
    for arguments in [&types, &bools] {
        assert!(check(arguments, true, &mut 10_000).is_ok());
        assert!(check(arguments, false, &mut 10_000).is_err());
        assert!(check(arguments, true, &mut 0).is_err());
        assert!(check(arguments, true, &mut 10_000).is_ok());
    }
    assert!(check(&[primitive(P::Bool), types[1].clone()], true, &mut 10_000).is_err());
    assert!(check(&[primitive(P::Type), bools[1].clone()], true, &mut 10_000).is_err());
    assert!(
        check(
            &[primitive(P::Type), type_list(vec![Term::Bool(true)])],
            true,
            &mut 10_000
        )
        .is_err()
    );
}

#[test]
fn frozen_collection_inputs_require_identity_authority_and_element_domains() {
    let list = |element| Term::NormalForm {
        head: id(90),
        sort: Sort::Type,
        arguments: vec![element],
    };
    let frozen = |ty, fields| Term::NominalValue {
        ty: Box::new(ty),
        variant: None,
        fields,
    };
    let bools = list(primitive(P::Bool));
    let nested = list(bools.clone());
    let descriptor = head(nested.clone());
    let good = frozen(
        nested.clone(),
        vec![
            frozen(bools.clone(), vec![Term::Bool(true), Term::Bool(false)]),
            frozen(bools.clone(), vec![]),
        ],
    );
    let check = |argument: &Term, trusted: bool, fuel: &mut usize| {
        descriptor.check_arguments_in_scope_with_collections_budgeted_in(
            std::slice::from_ref(argument),
            |_| false,
            |declaration| trusted && declaration == id(90),
            |_, _, _| None,
            |_, _| false,
            &[],
            fuel,
            None,
        )
    };
    assert!(good.check_scope(&[]).is_ok());
    assert!(
        descriptor
            .check_closed_arguments(std::slice::from_ref(&good))
            .is_err()
    );
    assert!(check(&good, false, &mut 100_000).is_err());
    assert!(check(&good, true, &mut 100_000).is_ok());
    for bad in [
        frozen(
            nested.clone(),
            vec![frozen(bools.clone(), vec![Term::Char('x')])],
        ),
        frozen(
            nested.clone(),
            vec![frozen(list(primitive(P::Char)), vec![Term::Char('x')])],
        ),
        frozen(nested.clone(), vec![Term::TupleValue(vec![])]),
        Term::NominalValue {
            ty: Box::new(nested.clone()),
            variant: Some(0),
            fields: vec![],
        },
    ] {
        assert!(check(&bad, true, &mut 100_000).is_err());
        assert!(check(&good, true, &mut 100_000).is_ok());
    }
    let mut low = 1;
    assert!(check(&good, true, &mut low).is_err());
    let empty = frozen(nested, vec![]);
    assert!(check(&empty, true, &mut 100_000).is_ok());
}

#[test]
fn dependent_collection_domains_validate_ordinary_type_list_payloads() {
    let list = |element| Term::NormalForm {
        head: id(90),
        sort: Sort::Type,
        arguments: vec![element],
    };
    let mut descriptor = head(primitive(P::Bool));
    descriptor.parameters = vec![
        parameter(1, Kind::Type, primitive(P::Type)),
        parameter(
            2,
            Kind::Value,
            list(Term::Rigid {
                declaration: id(1),
                sort: Sort::Type,
            }),
        ),
    ];
    let check = |element, value| {
        descriptor.check_arguments_in_scope_with_collections_budgeted_in(
            &[element, value],
            |_| false,
            |declaration| declaration == id(90),
            |_, _, _| None,
            |_, _| false,
            &[],
            &mut 100_000,
            None,
        )
    };
    assert!(check(primitive(P::Type), type_list(vec![primitive(P::Bool)])).is_ok());
    assert!(
        check(
            primitive(P::Bool),
            Term::NominalValue {
                ty: Box::new(list(primitive(P::Bool))),
                variant: None,
                fields: vec![Term::Bool(true)],
            }
        )
        .is_ok()
    );
    assert!(check(primitive(P::Bool), type_list(vec![primitive(P::Bool)])).is_err());
    assert!(
        check(
            primitive(P::Type),
            Term::NominalValue {
                ty: Box::new(list(primitive(P::Type))),
                variant: None,
                fields: vec![primitive(P::Bool)],
            }
        )
        .is_ok()
    );
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
        collector: None,
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
        collector: None,
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
        collector: None,
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
    let supported = |ty: &Term, prior: &[DeclarationId], trusted| {
        nir::normal_forms::supported_input_type_with_collections_in(
            ty,
            &prior
                .iter()
                .map(|declaration| nir::DeclarationParameter {
                    declaration: *declaration,
                    sort: Sort::Type,
                })
                .collect::<Vec<_>>(),
            trusted,
            |head| head == id(90),
            |_| false,
            None,
        )
    };
    let tuple = |fields| Term::NormalForm {
        head: id(9),
        sort: Sort::Type,
        arguments: vec![type_list(fields)],
    };
    let shape = tuple(vec![primitive(P::Bool), tuple(vec![primitive(P::U8)])]);
    let trusted = |head| head == id(9);
    assert!(!supported_input_type(&shape, &[]));
    assert!(supported(&shape, &[], trusted));
    let descriptor = head(shape.clone());
    let check = |values: &[Term]| {
        descriptor.check_arguments_in_scope_with_collections_budgeted_in(
            values,
            trusted,
            |head| head == id(90),
            |_, _, _| None,
            |_, _| false,
            &[],
            &mut 100_000,
            None,
        )
    };
    let valid = Term::TupleValue(vec![
        Term::Bool(true),
        Term::TupleValue(vec![Term::Integer(comptime_runtime::Integer::from_u128(
            255,
        ))]),
    ]);
    assert!(descriptor.check_closed_arguments(&[valid.clone()]).is_err());
    assert!(check(&[valid.clone()]).is_ok());
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
        assert!(check(&[bad]).is_err());
        assert!(check(&[valid.clone()]).is_ok());
    }
    let dependent = tuple(vec![Term::Rigid {
        declaration: id(7),
        sort: Sort::Type,
    }]);
    assert!(!supported(&dependent, &[], trusted));
    assert!(supported(&dependent, &[id(7)], trusted));
    assert!(!supported(&tuple(vec![Term::Bool(true)]), &[], trusted));
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
        collector: None,
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
        collector: None,
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

#[test]
fn error_inputs_validate_payload_domain_and_full_branch_membership() {
    let ty = Term::error_result(vec![primitive(P::Char)], primitive(P::Bool));
    assert!(supported_input_type(&ty, &[]));
    let descriptor = head(ty.clone());
    let input = |label, value| Term::ErrorValue {
        ty: Box::new(ty.clone()),
        label,
        value: Box::new(value),
    };
    for value in [
        input(None, Term::Bool(true)),
        input(Some(Box::new(primitive(P::Char))), Term::Char('a')),
    ] {
        descriptor.check_closed_arguments(&[value]).unwrap();
    }
    for value in [
        input(None, Term::Char('a')),
        input(Some(Box::new(primitive(P::Char))), Term::Bool(true)),
        input(Some(Box::new(primitive(P::Bool))), Term::Bool(true)),
    ] {
        assert!(descriptor.check_closed_arguments(&[value]).is_err());
    }
    assert!(!supported_input_type(
        &Term::error_result(
            vec![primitive(P::Char)],
            Term::Pointer(Box::new(primitive(P::Bool)))
        ),
        &[]
    ));
}

#[test]
fn residual_error_success_requires_the_unflattened_inner_domain() {
    let boolean = primitive(P::Bool);
    let inner = Term::error_result(vec![boolean.clone()], boolean.clone());
    let outer = Term::error_result(vec![primitive(P::Char)], inner.clone());
    let descriptor = head(outer);
    let pending = Term::Rigid {
        declaration: id(100),
        sort: Sort::ComptimeValue,
    };
    let argument = Term::error_value(
        vec![primitive(P::Char)],
        inner.clone(),
        None,
        pending.clone(),
    )
    .unwrap();
    assert!(argument.has_pending_error_branch());
    assert!(
        descriptor
            .check_closed_arguments(std::slice::from_ref(&argument))
            .is_err()
    );
    let check = |domain: &Term| {
        descriptor.check_arguments_with_evidence_budgeted(
            std::slice::from_ref(&argument),
            |_| false,
            |_, _, _| None,
            |value, expected| value == &pending && expected == domain,
            &mut 10000,
        )
    };
    check(&inner).unwrap();
    assert!(check(&boolean).is_err());
}

#[test]
fn associated_input_domains_preserve_type_and_value_binder_scope() {
    let domain = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(7),
            member: id(8),
        }),
        subject: Box::new(Term::Rigid {
            declaration: id(1),
            sort: Sort::Type,
        }),
        arguments: vec![Term::Rigid {
            declaration: id(2),
            sort: Sort::ComptimeValue,
        }],
    };
    let supported = |prior: &[nir::DeclarationParameter]| {
        nir::normal_forms::supported_input_type_in_scope(&domain, prior, |_| false, |_| false)
    };
    let subject = nir::DeclarationParameter {
        declaration: id(1),
        sort: Sort::Type,
    };
    let index = nir::DeclarationParameter {
        declaration: id(2),
        sort: Sort::ComptimeValue,
    };
    assert!(supported(&[subject, index]));
    assert!(!supported(&[]));
    assert!(!supported(&[subject]));
    assert!(!supported(&[index]));
    assert!(!supported(&[
        subject,
        nir::DeclarationParameter {
            declaration: id(2),
            sort: Sort::Type
        }
    ]));
}

#[test]
fn platform_normal_inputs_validate_target_ranges_recursively() {
    use nir::target::TargetData;
    for (domain, integer, accepted32, accepted64) in [
        (
            P::Usize,
            comptime_runtime::Integer::from_u128(1u128 << 32),
            false,
            true,
        ),
        (
            P::Isize,
            comptime_runtime::Integer::from_i128(-2147483649),
            false,
            true,
        ),
        (
            P::Usize,
            comptime_runtime::Integer::from_i128(-1),
            false,
            false,
        ),
        (
            P::Isize,
            comptime_runtime::Integer::from_i128(-2147483648),
            true,
            true,
        ),
    ] {
        let descriptor = head(record("count", primitive(domain)));
        let arguments = [value("count", Term::Integer(integer))];
        assert!(descriptor.check_closed_arguments(&arguments).is_err());
        for (width, accepted) in [(32, accepted32), (64, accepted64)] {
            let target = Some(TargetData::new(width).unwrap());
            let mut exhausted = 0;
            assert!(
                descriptor
                    .check_closed_arguments_budgeted_in(
                        &arguments,
                        |_| false,
                        |_, _, _| None,
                        &mut exhausted,
                        target
                    )
                    .is_err()
            );
            let mut fuel = 10000;
            assert_eq!(
                descriptor
                    .check_closed_arguments_budgeted_in(
                        &arguments,
                        |_| false,
                        |_, _, _| None,
                        &mut fuel,
                        target
                    )
                    .is_ok(),
                accepted
            );
            assert!(fuel < 10000);
            // Reusing a descriptor after checking another target cannot cache its range.
            assert!(descriptor.check_closed_arguments(&arguments).is_err());
        }
    }
}

#[test]
fn optional_normal_inputs_preserve_layer_and_validate_payload_domain() {
    let boolean = primitive(P::Bool);
    let domain = Term::Optional(Box::new(boolean.clone()));
    let descriptor = head(domain.clone());
    assert!(supported_input_type(&domain, &[]));
    for payload in [None, Some(Box::new(Term::Bool(true)))] {
        descriptor
            .check_closed_arguments(&[Term::OptionalValue {
                ty: Box::new(domain.clone()),
                value: payload,
            }])
            .unwrap();
    }
    for value in [
        Term::OptionalValue {
            ty: Box::new(domain.clone()),
            value: Some(Box::new(Term::Char('x'))),
        },
        Term::OptionalValue {
            ty: Box::new(Term::Optional(Box::new(domain))),
            value: None,
        },
    ] {
        assert!(descriptor.check_closed_arguments(&[value]).is_err());
    }
    assert!(!supported_input_type(
        &Term::Optional(Box::new(Term::Pointer(Box::new(boolean)))),
        &[]
    ));
    let empty = Term::Optional(Box::new(primitive(P::Never)));
    assert!(supported_input_type(&empty, &[]));
    head(empty.clone())
        .check_closed_arguments(&[Term::OptionalValue {
            ty: Box::new(empty),
            value: None,
        }])
        .unwrap();
}

#[test]
fn frozen_objects_check_every_stored_domain_and_preserve_duplicate_keys() {
    let object = primitive(P::Object);
    assert!(supported_input_type(&object, &[]));
    let descriptor = head(object.clone());
    let make =
        |domain, value| nir::objects::value(vec![nir::objects::entry(Some("flag"), domain, value)]);
    let good = nir::objects::value(vec![
        nir::objects::entry(Some("flag"), primitive(P::Bool), Term::Bool(true)),
        nir::objects::entry(
            Some("flag"),
            object.clone(),
            make(primitive(P::Char), Term::Char('x')),
        ),
        nir::objects::entry(None, primitive(P::Type), primitive(P::Bool)),
    ]);
    assert!(good.check_scope(&[]).is_ok());
    assert!(
        descriptor
            .check_closed_arguments(std::slice::from_ref(&good))
            .is_ok()
    );
    for bad in [
        make(primitive(P::Bool), Term::Char('x')),
        make(primitive(P::Type), Term::Bool(true)),
        make(object.clone(), Term::TupleValue(vec![])),
        nir::objects::value(vec![Term::TupleValue(vec![
            Term::Bool(true),
            primitive(P::Bool),
            Term::Bool(true),
        ])]),
        Term::NominalValue {
            ty: Box::new(object),
            variant: Some(0),
            fields: vec![],
        },
    ] {
        assert!(descriptor.check_closed_arguments(&[bad]).is_err());
        assert!(
            descriptor
                .check_closed_arguments(std::slice::from_ref(&good))
                .is_ok()
        );
    }
    assert!(
        descriptor
            .check_arguments_in_scope_with_collections_budgeted_in(
                &[good],
                |_| false,
                |_| false,
                |_, _, _| None,
                |_, _| false,
                &[],
                &mut 1,
                None,
            )
            .is_err()
    );
}

#[test]
fn registered_layout_views_read_typed_list_payloads_without_unfolding_heads() {
    use nir::normal_forms::Role;
    let list = Head {
        declaration: id(90),
        role: Role::TypeList,
        collector: None,
        parameters: vec![parameter(91, Kind::Type, primitive(P::Type))],
        requirements: vec![],
    };
    let domain = Term::NormalForm {
        head: list.declaration,
        sort: Sort::Type,
        arguments: vec![primitive(P::Type)],
    };
    let fields = vec![primitive(P::Bool), primitive(P::U8)];
    let payload = Term::NominalValue {
        ty: Box::new(domain.clone()),
        variant: None,
        fields: fields.clone(),
    };
    assert_eq!(
        list.list_value_elements(&domain, &payload),
        Some(fields.as_slice())
    );
    let mut opaque = list.clone();
    opaque.role = Role::Opaque;
    assert!(opaque.list_value_elements(&domain, &payload).is_none());
    let tuple = Head {
        declaration: id(80),
        role: Role::Tuple,
        collector: Some(0),
        parameters: vec![parameter(81, Kind::Value, domain.clone())],
        requirements: vec![],
    };
    let application = |payload| Term::NormalForm {
        head: tuple.declaration,
        sort: Sort::Type,
        arguments: vec![payload],
    };
    assert_eq!(
        tuple.tuple_elements(&application(payload.clone())),
        Some(fields.as_slice())
    );
    assert!(
        tuple
            .tuple_elements(&application(Term::TupleValue(fields.clone())))
            .is_none()
    );
    for fields in [vec![], fields.clone()] {
        let untyped = Term::TupleValue(fields);
        assert!(list.list_value_elements(&domain, &untyped).is_none());
        assert!(tuple.tuple_elements(&application(untyped)).is_none());
    }
    let mut missing = tuple.clone();
    missing.parameters.clear();
    assert!(
        missing
            .tuple_elements(&application(payload.clone()))
            .is_none()
    );
    assert!(
        missing
            .tuple_elements(&application(Term::TupleValue(vec![])))
            .is_none()
    );
    for wrong in [
        Term::NominalValue {
            ty: Box::new(primitive(P::Type)),
            variant: None,
            fields: fields.clone(),
        },
        Term::NominalValue {
            ty: Box::new(domain.clone()),
            variant: Some(0),
            fields: fields.clone(),
        },
        Term::NominalValue {
            ty: Box::new(Term::NormalForm {
                head: id(92),
                sort: Sort::Type,
                arguments: vec![primitive(P::Type)],
            }),
            variant: None,
            fields: fields.clone(),
        },
        Term::NominalValue {
            ty: Box::new(domain.clone()),
            variant: None,
            fields: vec![Term::Bool(true)],
        },
    ] {
        assert!(tuple.tuple_elements(&application(wrong)).is_none());
    }
    assert!(
        tuple
            .tuple_elements(&application(type_list(vec![Term::Bool(true)])))
            .is_none()
    );
    assert!(
        tuple
            .tuple_elements(&Term::NormalForm {
                head: id(79),
                sort: Sort::Type,
                arguments: vec![payload]
            })
            .is_none()
    );
}

#[test]
fn collection_admission_never_supplies_a_missing_value_domain() {
    let domain = Term::NormalForm {
        head: id(90),
        sort: Sort::Type,
        arguments: vec![primitive(P::Type)],
    };
    for fields in [vec![], vec![primitive(P::Bool)]] {
        for nested in [false, true] {
            let descriptor = head(if nested {
                record("items", domain.clone())
            } else {
                domain.clone()
            });
            let wrap = |input| if nested { value("items", input) } else { input };
            let check = |input, fuel: &mut usize| {
                descriptor.check_arguments_in_scope_with_collections_budgeted_in(
                    &[wrap(input)],
                    |_| false,
                    |declaration| declaration == id(90),
                    |_, _, _| None,
                    |_, _| false,
                    &[],
                    fuel,
                    None,
                )
            };
            let valid = type_list(fields.clone());
            assert!(check(valid.clone(), &mut 10000).is_ok());
            assert!(check(Term::TupleValue(fields.clone()), &mut 10000).is_err());
            let foreign = Term::NominalValue {
                ty: Box::new(Term::NormalForm {
                    head: id(91),
                    sort: Sort::Type,
                    arguments: vec![primitive(P::Type)],
                }),
                variant: None,
                fields: fields.clone(),
            };
            assert!(check(foreign, &mut 10000).is_err());
            assert!(check(valid.clone(), &mut 0).is_err());
            assert!(check(valid, &mut 10000).is_ok());
        }
    }
}
