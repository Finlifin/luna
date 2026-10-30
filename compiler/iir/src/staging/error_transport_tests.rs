use super::*;
use nir::error_results::Conversion as C;

fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: nir::program::SourceRange { start: 1, end: 2 },
    }
}
fn error_type(reverse: bool) -> Term {
    let mut errors = vec![
        Term::Primitive(Primitive::Bool),
        Term::Primitive(Primitive::Char),
    ];
    if reverse {
        errors.reverse();
    }
    // Exercise differing physical layouts; source type construction now uses
    // canonical rows and would make this permutation an identity conversion.
    Term::ErrorResult {
        errors,
        result: Box::new(Term::Primitive(Primitive::Bool)),
    }
}
fn input() -> Expr {
    let boolean = expr(
        Term::Primitive(Primitive::Bool),
        K::Literal(Literal::Bool(true)),
    );
    let error = expr(
        error_type(false),
        K::ErrorConvert {
            conversion: C::Failure,
            value: Box::new(boolean.clone()),
        },
    );
    expr(
        Term::Tuple(vec![error.ty.clone(), boolean.ty.clone()]),
        K::Tuple(vec![error, boolean]),
    )
}

#[test]
fn staged_product_transport_freezes_exact_layout_without_invocation() {
    let source = input();
    let target = Term::Tuple(vec![error_type(true), Term::Primitive(Primitive::Bool)]);
    let converted = expr(
        target.clone(),
        K::ErrorConvert {
            conversion: C::Product,
            value: Box::new(source),
        },
    );
    let expected = Term::TupleValue(vec![
        Term::ErrorValue {
            ty: Box::new(error_type(true)),
            label: Some(Box::new(Term::Primitive(Primitive::Bool))),
            value: Box::new(Term::Bool(true)),
        },
        Term::Bool(true),
    ]);
    assert_eq!(constant(&[], &converted, &mut 10000, 0).unwrap(), expected);
    canonical_input_with_layouts(Layouts::new(&[], &[]), &converted, &mut 10000).unwrap();
    assert!(constant(&[], &converted, &mut 0, 0).is_err());
    let widened = Term::Tuple(vec![
        Term::error_result(
            vec![
                Term::Primitive(Primitive::Bool),
                Term::Primitive(Primitive::Char),
                Term::Primitive(Primitive::Str),
            ],
            Term::Primitive(Primitive::Bool),
        ),
        Term::Primitive(Primitive::Bool),
    ]);
    assert!(constant(&[], &expr(widened, converted.kind.clone()), &mut 10000, 0).is_err());
}

#[test]
fn staged_product_transport_does_not_invoke_nonconstant_producers() {
    let value = input();
    let producer = expr(value.ty, K::Local(LocalId { owner: 1, index: 0 }));
    let converted = expr(
        Term::Tuple(vec![error_type(true), Term::Primitive(Primitive::Bool)]),
        K::ErrorConvert {
            conversion: C::Product,
            value: Box::new(producer),
        },
    );
    assert!(constant(&[], &converted, &mut 10000, 0).is_err());
}

#[test]
fn staged_error_match_after_product_transport_keeps_guard_order_and_bindings() {
    use nir::program::{MatchArm, Pattern};
    let binding = LocalId { owner: 2, index: 0 };
    let boolean = Term::Primitive(Primitive::Bool);
    let selected = Pattern::Tuple(vec![
        Pattern::ErrorResult {
            label: Some(boolean.clone()),
            inner: Box::new(Pattern::Binding {
                binding,
                inner: Box::new(Pattern::Wild),
            }),
        },
        Pattern::Wild,
    ]);
    let converted = expr(
        Term::Tuple(vec![error_type(true), boolean.clone()]),
        K::ErrorConvert {
            conversion: C::Product,
            value: Box::new(input()),
        },
    );
    let unavailable = expr(
        boolean.clone(),
        K::Local(LocalId {
            owner: 99,
            index: 99,
        }),
    );
    let matched = expr(
        boolean.clone(),
        K::Match {
            value: Box::new(converted),
            arms: vec![
                MatchArm {
                    pattern: selected.clone(),
                    guard: Some(expr(boolean.clone(), K::Literal(Literal::Bool(false)))),
                    body: unavailable.clone(),
                },
                MatchArm {
                    pattern: selected,
                    guard: None,
                    body: expr(boolean.clone(), K::Local(binding)),
                },
                MatchArm {
                    pattern: Pattern::Wild,
                    guard: None,
                    body: unavailable,
                },
            ],
        },
    );
    assert_eq!(
        fold_value(&matched, &mut 10000).unwrap().kind,
        K::Literal(Literal::Bool(true))
    );
    assert!(fold_value(&matched, &mut 0).is_err());
}

#[test]
fn staged_error_success_matching_and_embed_retain_payload() {
    use nir::program::{MatchArm, Pattern};
    let boolean = Term::Primitive(Primitive::Bool);
    for (conversion, selected) in [(C::Success, None), (C::Failure, Some(boolean.clone()))] {
        let value = expr(
            Term::error_result(vec![boolean.clone()], boolean.clone()),
            K::ErrorConvert {
                conversion,
                value: Box::new(expr(boolean.clone(), K::Literal(Literal::Bool(true)))),
            },
        );
        let converted = expr(
            error_type(true),
            K::ErrorConvert {
                conversion: C::Embed,
                value: Box::new(value),
            },
        );
        let matched = expr(
            boolean.clone(),
            K::Match {
                value: Box::new(converted),
                arms: vec![
                    MatchArm {
                        pattern: Pattern::ErrorResult {
                            label: Some(Term::Primitive(Primitive::Char)),
                            inner: Box::new(Pattern::Wild),
                        },
                        guard: None,
                        body: expr(
                            boolean.clone(),
                            K::Local(LocalId {
                                owner: 99,
                                index: 0,
                            }),
                        ),
                    },
                    MatchArm {
                        pattern: Pattern::ErrorResult {
                            label: selected,
                            inner: Box::new(Pattern::Wild),
                        },
                        guard: None,
                        body: expr(boolean.clone(), K::Literal(Literal::Bool(true))),
                    },
                ],
            },
        );
        assert_eq!(
            fold_value(&matched, &mut 10000).unwrap().kind,
            K::Literal(Literal::Bool(true))
        );
    }
}

#[test]
fn staged_error_nesting_exhausts_logical_depth_without_invocation() {
    let ty = error_type(false);
    let mut value = expr(
        ty.clone(),
        K::ErrorConvert {
            conversion: C::Success,
            value: Box::new(expr(
                Term::Primitive(Primitive::Bool),
                K::Literal(Literal::Bool(true)),
            )),
        },
    );
    for _ in 0..150 {
        value = expr(
            ty.clone(),
            K::ErrorConvert {
                conversion: C::Embed,
                value: Box::new(value),
            },
        );
    }
    assert!(
        fold_value(&value, &mut 100000)
            .unwrap_err()
            .contains("depth")
    );
}

#[test]
fn staged_typed_tuple_transport_uses_declared_collector_domain() {
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    let id = |local| nir::DeclarationId { package: 4, local };
    let list = Term::NormalForm {
        head: id(1),
        sort: nir::Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Type)],
    };
    let heads = [Head {
        declaration: id(2),
        role: Role::Tuple,
        collector: None,
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(3),
            binding: LocalId { owner: 2, index: 0 },
            kind: Kind::Value,
            ty: list.clone(),
        }],
    }];
    let tuple = |fields| Term::NormalForm {
        head: id(2),
        sort: nir::Sort::Type,
        arguments: vec![Term::NominalValue {
            ty: Box::new(list.clone()),
            variant: None,
            fields,
        }],
    };
    let mut source = input();
    source.ty = tuple(vec![error_type(false), Term::Primitive(Primitive::Bool)]);
    let converted = expr(
        tuple(vec![error_type(true), Term::Primitive(Primitive::Bool)]),
        K::ErrorConvert {
            conversion: C::Product,
            value: Box::new(source),
        },
    );
    let layouts = Layouts::new(&heads, &[]);
    let frozen = constant(layouts, &converted, &mut 10000, 0).unwrap();
    let expected = Term::TupleValue(vec![
        Term::ErrorValue {
            ty: Box::new(error_type(true)),
            label: Some(Box::new(Term::Primitive(Primitive::Bool))),
            value: Box::new(Term::Bool(true)),
        },
        Term::Bool(true),
    ]);
    assert_eq!(frozen, expected);
    canonical_input_with_layouts(layouts, &converted, &mut 10000).unwrap();
    let folded = fold_input(layouts, &converted, &mut 10000, 0).unwrap();
    assert_eq!(folded.ty, converted.ty);
    assert_eq!(constant(layouts, &folded, &mut 10000, 0).unwrap(), expected);
    assert!(constant(&[], &converted, &mut 10000, 0).is_err());
    assert!(constant(layouts, &converted, &mut 0, 0).is_err());
    let mut invalid = heads.clone();
    invalid[0].parameters[0].ty = Term::NormalForm {
        head: id(99),
        sort: nir::Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Type)],
    };
    assert!(constant(Layouts::new(&invalid, &[]), &converted, &mut 10000, 0).is_err());
    assert_eq!(
        constant(layouts, &converted, &mut 10000, 0).unwrap(),
        expected
    );
}
