//! Stable list folding uses trusted collection metadata and complete elements.
use iir::staging::{Layouts, fold_value_with_layouts};
use nir::normal_forms::{Head, Kind, Parameter, Role};
use nir::program::{Expr, ExprKind as K, Literal, LocalId, SourceRange};
use nir::{DeclarationId, Primitive, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 51, local }
}
fn head() -> Head {
    Head {
        collector: None,
        declaration: id(1),
        role: Role::TypeList,
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(2),
            binding: LocalId { owner: 1, index: 0 },
            kind: Kind::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
    }
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 2, end: 7 },
    }
}
fn boolean(value: bool) -> Expr {
    expr(
        Term::Primitive(Primitive::Bool),
        K::Literal(Literal::Bool(value)),
    )
}
fn list(element: Term, fields: Vec<Expr>) -> Expr {
    expr(
        Term::NormalForm {
            head: id(1),
            sort: Sort::Type,
            arguments: vec![element],
        },
        K::TypeList(fields),
    )
}
fn index(values: Expr, element: Term, position: &str) -> Expr {
    expr(
        element,
        K::TypeListIndex(
            Box::new(values),
            Box::new(expr(
                Term::Primitive(Primitive::Integer),
                K::Literal(Literal::Integer(position.into())),
            )),
        ),
    )
}

#[test]
fn nested_list_folding_keeps_identity_and_selects_the_exact_element() {
    let heads = [head()];
    let layouts = Layouts::new(&heads, &[]);
    let bool_ty = Term::Primitive(Primitive::Bool);
    let values = list(bool_ty.clone(), vec![boolean(true), boolean(false)]);
    let folded = fold_value_with_layouts(layouts, &values, &mut 100_000).unwrap();
    assert_eq!(folded.ty, values.ty);
    let nested = list(values.ty.clone(), vec![values.clone()]);
    let selected = index(index(nested, values.ty.clone(), "0"), bool_ty.clone(), "1");
    let folded = fold_value_with_layouts(layouts, &selected, &mut 100_000).unwrap();
    assert!(matches!(folded.kind, K::Literal(Literal::Bool(false))));
    assert_eq!(folded.ty, bool_ty);
    assert_eq!(folded.source, selected.source);
    for position in ["-1", "2", "999999999999999999999999999999999999999999"] {
        assert!(
            fold_value_with_layouts(
                layouts,
                &index(values.clone(), bool_ty.clone(), position),
                &mut 100_000
            )
            .is_err()
        );
    }
    assert!(fold_value_with_layouts(layouts, &values, &mut 0).is_err());
    assert!(fold_value_with_layouts(layouts, &values, &mut 100_000).is_ok());
}

#[test]
fn folding_rejects_unbound_heads_and_does_not_skip_unfinished_elements() {
    let bool_ty = Term::Primitive(Primitive::Bool);
    let values = list(bool_ty.clone(), vec![boolean(true)]);
    let mut unbound = head();
    unbound.role = Role::Opaque;
    assert!(fold_value_with_layouts(Layouts::new(&[unbound], &[]), &values, &mut 100_000).is_err());
    let heads = [head()];
    let layouts = Layouts::new(&heads, &[]);
    let unfinished = list(
        bool_ty.clone(),
        vec![
            boolean(true),
            expr(bool_ty.clone(), K::Local(LocalId { owner: 9, index: 0 })),
        ],
    );
    assert!(
        fold_value_with_layouts(
            layouts,
            &index(unfinished, bool_ty.clone(), "0"),
            &mut 100_000
        )
        .is_err()
    );
    let wrong = list(
        bool_ty.clone(),
        vec![expr(
            Term::Primitive(Primitive::Type),
            K::TypeValue(bool_ty.clone()),
        )],
    );
    assert!(fold_value_with_layouts(layouts, &wrong, &mut 100_000).is_err());
    let empty = list(bool_ty.clone(), vec![]);
    assert!(fold_value_with_layouts(layouts, &empty, &mut 100_000).is_ok());
    assert!(fold_value_with_layouts(layouts, &index(empty, bool_ty, "0"), &mut 100_000).is_err());
}

#[test]
fn list_length_and_concat_fold_with_the_original_nominal_identity() {
    let heads = [head()];
    let layouts = Layouts::new(&heads, &[]);
    for element in [Primitive::Bool, Primitive::Type] {
        let field = if element == Primitive::Type {
            expr(
                Term::Primitive(element),
                K::TypeValue(Term::Primitive(Primitive::Bool)),
            )
        } else {
            boolean(true)
        };
        let values = list(Term::Primitive(element), vec![field]);
        let joined = expr(
            values.ty.clone(),
            K::ListConcat(Box::new(values.clone()), Box::new(values.clone())),
        );
        let folded = fold_value_with_layouts(layouts, &joined, &mut 100_000).unwrap();
        assert_eq!(folded.ty, values.ty);
        let length = expr(
            Term::Primitive(Primitive::Integer),
            K::ListLength(Box::new(joined.clone())),
        );
        let folded = fold_value_with_layouts(layouts, &length, &mut 100_000).unwrap();
        assert!(matches!(folded.kind,K::Literal(Literal::Integer(ref n)) if n=="2"));
        assert!(joined.to_string().contains("(list-concat"));
        assert!(length.to_string().contains("(list-length"));
        assert!(fold_value_with_layouts(layouts, &joined, &mut 1).is_err());
        let mut bad = length;
        bad.ty = Term::Primitive(Primitive::Bool);
        assert!(fold_value_with_layouts(layouts, &bad, &mut 100_000).is_err());
    }
}

#[test]
fn mixed_completed_list_encodings_share_nested_operations_and_domain_checks() {
    let heads = [head()];
    let layouts = Layouts::new(&heads, &[]);
    let type_domain = Term::Primitive(Primitive::Type);
    let bool_type = Term::Primitive(Primitive::Bool);
    let char_type = Term::Primitive(Primitive::Char);
    let ordinary = list(
        type_domain.clone(),
        vec![expr(type_domain.clone(), K::TypeValue(bool_type.clone()))],
    );
    let frozen = Expr {
        kind: K::ComptimeValue(Term::NominalValue {
            ty: Box::new(ordinary.ty.clone()),
            variant: None,
            fields: vec![char_type.clone()],
        }),
        ..ordinary.clone()
    };
    for (left, right, expected) in [
        (ordinary.clone(), frozen.clone(), char_type.clone()),
        (frozen.clone(), ordinary.clone(), bool_type.clone()),
    ] {
        let joined = expr(
            ordinary.ty.clone(),
            K::ListConcat(Box::new(left), Box::new(right)),
        );
        let nested = list(ordinary.ty.clone(), vec![joined.clone()]);
        let selected = index(
            index(nested, ordinary.ty.clone(), "0"),
            type_domain.clone(),
            "1",
        );
        let folded = fold_value_with_layouts(layouts, &selected, &mut 100_000).unwrap();
        assert_eq!(folded.ty, type_domain);
        assert_eq!(folded.kind, K::TypeValue(expected));
        let length = expr(
            Term::Primitive(Primitive::Integer),
            K::ListLength(Box::new(joined)),
        );
        assert_eq!(
            fold_value_with_layouts(layouts, &length, &mut 100_000)
                .unwrap()
                .kind,
            K::Literal(Literal::Integer("2".into()))
        );
    }
    let empty = list(type_domain.clone(), vec![]);
    let joined = expr(
        ordinary.ty.clone(),
        K::ListConcat(Box::new(empty), Box::new(frozen.clone())),
    );
    let selected = index(joined, type_domain.clone(), "0");
    assert_eq!(
        fold_value_with_layouts(layouts, &selected, &mut 100_000)
            .unwrap()
            .kind,
        K::TypeValue(char_type)
    );
    for bad in [
        index(frozen.clone(), bool_type.clone(), "0"),
        index(frozen.clone(), type_domain.clone(), "1"),
        expr(
            ordinary.ty.clone(),
            K::ListConcat(
                Box::new(ordinary.clone()),
                Box::new(list(bool_type.clone(), vec![boolean(true)])),
            ),
        ),
        index(
            Expr {
                kind: K::ComptimeValue(Term::NominalValue {
                    ty: Box::new(ordinary.ty.clone()),
                    variant: None,
                    fields: vec![Term::Bool(true)],
                }),
                ..frozen
            },
            type_domain.clone(),
            "0",
        ),
        index(
            list(
                type_domain.clone(),
                vec![
                    expr(type_domain, K::TypeValue(bool_type.clone())),
                    expr(
                        Term::Primitive(Primitive::Type),
                        K::Local(LocalId {
                            owner: 99,
                            index: 0,
                        }),
                    ),
                ],
            ),
            Term::Primitive(Primitive::Type),
            "0",
        ),
    ] {
        assert!(fold_value_with_layouts(layouts, &bad, &mut 100_000).is_err());
    }
    assert!(fold_value_with_layouts(layouts, &selected, &mut 1).is_err());
    assert!(fold_value_with_layouts(layouts, &selected, &mut 100_000).is_ok());
}

fn frozen_list(element: Term, fields: Vec<Term>) -> Expr {
    let ty = Term::NormalForm {
        head: id(1),
        sort: Sort::Type,
        arguments: vec![element],
    };
    expr(
        ty.clone(),
        K::ComptimeValue(Term::NominalValue {
            ty: Box::new(ty),
            variant: None,
            fields,
        }),
    )
}

#[test]
fn frozen_lists_fold_index_length_concat_and_nested_values_without_losing_domains() {
    let heads = [head()];
    let layouts = Layouts::new(&heads, &[]);
    let bool_ty = Term::Primitive(Primitive::Bool);
    let values = frozen_list(bool_ty.clone(), vec![Term::Bool(true), Term::Bool(false)]);
    let selected = index(values.clone(), bool_ty.clone(), "1");
    assert_eq!(
        fold_value_with_layouts(layouts, &selected, &mut 100_000).unwrap(),
        boolean(false)
    );
    let length = expr(
        Term::Primitive(Primitive::Integer),
        K::ListLength(Box::new(values.clone())),
    );
    assert!(
        matches!(fold_value_with_layouts(layouts, &length, &mut 100_000).unwrap().kind,
        K::Literal(Literal::Integer(value)) if value == "2")
    );
    let joined = expr(
        values.ty.clone(),
        K::ListConcat(
            Box::new(values.clone()),
            Box::new(list(bool_ty.clone(), vec![boolean(true)])),
        ),
    );
    assert_eq!(
        fold_value_with_layouts(layouts, &joined, &mut 100_000).unwrap(),
        list(
            bool_ty.clone(),
            vec![boolean(true), boolean(false), boolean(true)]
        )
    );
    let K::ComptimeValue(payload) = &values.kind else {
        unreachable!()
    };
    let nested = frozen_list(values.ty.clone(), vec![payload.clone()]);
    let selected = index(index(nested, values.ty.clone(), "0"), bool_ty.clone(), "1");
    assert_eq!(
        fold_value_with_layouts(layouts, &selected, &mut 100_000).unwrap(),
        boolean(false)
    );
    let opaque = Term::NormalForm {
        head: id(77),
        sort: Sort::Type,
        arguments: vec![bool_ty],
    };
    let types = frozen_list(Term::Primitive(Primitive::Type), vec![opaque.clone()]);
    let selected = index(types, Term::Primitive(Primitive::Type), "0");
    assert_eq!(
        fold_value_with_layouts(layouts, &selected, &mut 100_000)
            .unwrap()
            .kind,
        K::TypeValue(opaque)
    );
}

#[test]
fn frozen_list_admission_checks_all_fields_identity_scope_and_budget_before_selection() {
    let heads = [head()];
    let layouts = Layouts::new(&heads, &[]);
    let bool_ty = Term::Primitive(Primitive::Bool);
    let valid = frozen_list(bool_ty.clone(), vec![Term::Bool(true)]);
    let mut foreign = valid.clone();
    let K::ComptimeValue(Term::NominalValue { ty, .. }) = &mut foreign.kind else {
        unreachable!()
    };
    let Term::NormalForm { head, .. } = ty.as_mut() else {
        unreachable!()
    };
    *head = id(99);
    for values in [
        foreign,
        frozen_list(bool_ty.clone(), vec![Term::Bool(true), Term::Char('x')]),
        frozen_list(
            bool_ty.clone(),
            vec![
                Term::Bool(true),
                Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::ComptimeValue,
                },
            ],
        ),
    ] {
        assert!(
            fold_value_with_layouts(layouts, &index(values, bool_ty.clone(), "0"), &mut 100_000)
                .is_err()
        );
    }
    assert!(fold_value_with_layouts(Layouts::new(&[], &[]), &valid, &mut 100_000).is_err());
    for position in ["-1", "1", "999999999999999999999999999999999"] {
        assert!(
            fold_value_with_layouts(
                layouts,
                &index(valid.clone(), bool_ty.clone(), position),
                &mut 100_000
            )
            .is_err()
        );
    }
    assert!(fold_value_with_layouts(layouts, &valid, &mut 1).is_err());
    assert!(fold_value_with_layouts(layouts, &valid, &mut 100_000).is_ok());
    let empty = frozen_list(bool_ty, vec![]);
    assert!(fold_value_with_layouts(layouts, &empty, &mut 100_000).is_ok());
}

#[test]
fn frozen_list_fields_reconstruct_optional_record_tuple_and_machine_scalar_data() {
    let heads = [head()];
    let layouts = Layouts::new(&heads, &[]);
    let byte = Term::Primitive(Primitive::U8);
    let tuple = Term::Tuple(vec![byte.clone(), Term::Primitive(Primitive::Bool)]);
    let record = Term::Record(vec![("pair".into(), tuple.clone())]);
    let optional = Term::Optional(Box::new(record.clone()));
    let value = Term::OptionalValue {
        ty: Box::new(optional.clone()),
        value: Some(Box::new(Term::RecordValue(vec![(
            "pair".into(),
            Term::TupleValue(vec![
                Term::Integer(comptime_runtime::Integer::from_u128(255)),
                Term::Bool(true),
            ]),
        )]))),
    };
    let values = frozen_list(optional.clone(), vec![value]);
    let folded =
        fold_value_with_layouts(layouts, &index(values, optional.clone(), "0"), &mut 100_000)
            .unwrap();
    let expected = expr(
        optional,
        K::OptionalPresent(Box::new(expr(
            record,
            K::AnonymousRecord {
                fields: vec![expr(
                    tuple,
                    K::Tuple(vec![
                        expr(byte.clone(), K::Literal(Literal::UnsignedInteger(255))),
                        boolean(true),
                    ]),
                )],
            },
        ))),
    );
    assert_eq!(folded, expected);
    let invalid = frozen_list(
        byte.clone(),
        vec![Term::Integer(comptime_runtime::Integer::from_u128(256))],
    );
    assert!(fold_value_with_layouts(layouts, &index(invalid, byte, "0"), &mut 100_000).is_err());
}

#[test]
fn canonical_frozen_inputs_check_machine_domains_and_recover_after_failure() {
    use iir::staging::canonical_input_with_layouts;
    let heads = [head()];
    let layouts = Layouts::new(&heads, &[]);
    for (value, primitive, expected) in [
        (Term::Bool(true), Primitive::Bool, Literal::Bool(true)),
        (Term::Char('x'), Primitive::Char, Literal::Char('x')),
        (
            Term::Float16(0x3c00),
            Primitive::F16,
            Literal::Float16(0x3c00),
        ),
        (
            Term::BFloat16(0x3f80),
            Primitive::Bf16,
            Literal::BFloat16(0x3f80),
        ),
        (
            Term::Float32(0x80000000),
            Primitive::F32,
            Literal::Float32(0x80000000),
        ),
        (
            Term::Float64(0x7ff0000000000000),
            Primitive::F64,
            Literal::Float64(0x7ff0000000000000),
        ),
    ] {
        let value = expr(Term::Primitive(primitive), K::ComptimeValue(value));
        canonical_input_with_layouts(layouts, &value, &mut 100_000).unwrap();
        assert_eq!(
            fold_value_with_layouts(layouts, &value, &mut 100_000)
                .unwrap()
                .kind,
            K::Literal(expected)
        );
        let wrong = Expr {
            ty: Term::Primitive(Primitive::Symbol),
            ..value
        };
        assert!(canonical_input_with_layouts(layouts, &wrong, &mut 100_000).is_err());
    }
    let bool_ty = Term::Primitive(Primitive::Bool);
    let bad = frozen_list(bool_ty.clone(), vec![Term::Bool(true), Term::Char('x')]);
    assert!(canonical_input_with_layouts(layouts, &bad, &mut 100_000).is_err());
    let valid = frozen_list(bool_ty, vec![Term::Bool(true)]);
    assert!(canonical_input_with_layouts(layouts, &valid, &mut 1).is_err());
    canonical_input_with_layouts(layouts, &valid, &mut 100_000).unwrap();
}
