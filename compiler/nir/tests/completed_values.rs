use nir::program::{Capture, Expr, ExprKind as K, Literal, LocalId, SourceRange};
use nir::{Primitive, Term};
use std::sync::Arc;
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn id(index: u32) -> LocalId {
    LocalId { owner: 7, index }
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 0, end: 1 },
    }
}
fn signature(result: Term) -> Term {
    Term::ComptimeFunction(Box::new(Term::Function {
        parameters: vec![],
        result: Box::new(result),
    }))
}
fn frozen() -> Expr {
    let ty = Term::Closure {
        environment: vec![boolean()],
        signature: Box::new(signature(boolean())),
    };
    let closure = expr(
        ty.clone(),
        K::Closure {
            parameters: vec![],
            captures: vec![Capture {
                binding: id(1),
                ty: boolean(),
                value: Some(Box::new(expr(boolean(), K::Literal(Literal::Bool(true))))),
            }],
            body: Arc::new(expr(boolean(), K::Local(id(1)))),
        },
    );
    let body = expr(
        ty.clone(),
        K::Block {
            statements: vec![expr(
                Term::Primitive(Primitive::Void),
                K::Let {
                    binding: id(2),
                    value: Box::new(closure),
                },
            )],
            result: Some(Box::new(expr(ty.clone(), K::Local(id(2))))),
        },
    );
    expr(
        ty.clone(),
        K::Call {
            callee: Box::new(expr(
                signature(ty),
                K::Closure {
                    parameters: vec![],
                    captures: vec![],
                    body: Arc::new(body),
                },
            )),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    )
}
fn validate(value: &Expr) -> Result<(), &'static str> {
    nir::completed_values::validate(value, &mut 10000)
}

#[test]
fn target_ranges_reach_nested_values_and_closure_captures() {
    use nir::target::TargetData;
    let platform = Term::Primitive(Primitive::Usize);
    let value = expr(
        platform.clone(),
        K::Literal(Literal::UnsignedInteger(1u128 << 32)),
    );
    let pair = expr(
        Term::Tuple(vec![boolean(), platform.clone()]),
        K::Tuple(vec![
            expr(boolean(), K::Literal(Literal::Bool(true))),
            value.clone(),
        ]),
    );
    let closure_type = Term::Closure {
        environment: vec![pair.ty.clone()],
        signature: Box::new(signature(boolean())),
    };
    let closure = expr(
        closure_type,
        K::Closure {
            parameters: vec![],
            captures: vec![Capture {
                binding: id(1),
                ty: pair.ty.clone(),
                value: Some(Box::new(pair.clone())),
            }],
            body: Arc::new(expr(boolean(), K::Literal(Literal::Bool(true)))),
        },
    );
    let captured_term = expr(
        platform.clone(),
        K::ComptimeValue(Term::Integer(comptime_runtime::Integer::from_u128(
            1u128 << 32,
        ))),
    );
    let captured_record = expr(
        Term::Record(vec![("count".into(), platform)]),
        K::AnonymousRecord {
            fields: vec![captured_term.clone()],
        },
    );
    for candidate in [value, pair, closure, captured_term, captured_record] {
        assert!(validate(&candidate).is_err());
        let mut fuel = 10000;
        assert!(
            nir::completed_values::validate_in(&candidate, TargetData::new(32).unwrap(), &mut fuel)
                .is_err()
        );
        assert!(fuel < 10000);
        nir::completed_values::validate_in(&candidate, TargetData::new(64).unwrap(), &mut 10000)
            .unwrap();
        assert!(
            nir::completed_values::validate_in(&candidate, TargetData::new(64).unwrap(), &mut 0)
                .is_err()
        );
        // A failed narrow-target request cannot poison a later wider request.
        nir::completed_values::validate_in(&candidate, TargetData::new(64).unwrap(), &mut 10000)
            .unwrap();
    }
}
#[test]
fn frozen_closures_require_closed_explicit_captures_and_private_bindings() {
    let original = frozen();
    validate(&original).unwrap();
    for mutation in 0..4 {
        let mut value = original.clone();
        let K::Call { callee, .. } = &mut value.kind else {
            unreachable!()
        };
        let K::Closure { body, .. } = &mut callee.kind else {
            unreachable!()
        };
        let K::Block { statements, result } = &mut Arc::make_mut(body).kind else {
            unreachable!()
        };
        if mutation == 0 {
            result.as_mut().unwrap().kind = K::Local(id(99));
        } else if mutation == 1 {
            statements.push(statements[0].clone());
        } else {
            let K::Let { value, .. } = &mut statements[0].kind else {
                unreachable!()
            };
            let K::Closure { captures, .. } = &mut value.kind else {
                unreachable!()
            };
            if mutation == 2 {
                captures[0].value = None;
            } else {
                captures[0].value = Some(Box::new(expr(boolean(), K::Local(id(99)))));
            }
        }
        assert!(validate(&value).is_err(), "mutation {mutation}");
    }
    validate(&original).unwrap();
}
#[test]
fn reconstruction_cannot_execute_arbitrary_declarations_or_statements() {
    let mut value = frozen();
    let K::Call { callee, .. } = &mut value.kind else {
        unreachable!()
    };
    let original = callee.clone();
    callee.kind = K::Function(nir::DeclarationId {
        package: 0,
        local: 1,
    });
    assert!(validate(&value).is_err());
    let K::Call { callee, .. } = &mut value.kind else {
        unreachable!()
    };
    *callee = original;
    let K::Closure { body, .. } = &mut callee.kind else {
        unreachable!()
    };
    let K::Block { statements, .. } = &mut Arc::make_mut(body).kind else {
        unreachable!()
    };
    statements.push(expr(boolean(), K::Literal(Literal::Bool(true))));
    assert!(validate(&value).is_err());
}
#[test]
fn completed_graph_validation_recovers_after_budget_exhaustion() {
    let value = frozen();
    assert!(nir::completed_values::validate(&value, &mut 0).is_err());
    validate(&value).unwrap();
    let mut deep = expr(boolean(), K::Literal(Literal::Bool(true)));
    for _ in 0..70 {
        deep = expr(Term::Tuple(vec![deep.ty.clone()]), K::Tuple(vec![deep]));
    }
    assert!(validate(&deep).is_err());
    validate(&value).unwrap();
}

#[test]
fn reconstruction_wrapper_cannot_claim_an_implicit_environment() {
    let mut value = frozen();
    let K::Call { callee, .. } = &mut value.kind else {
        unreachable!()
    };
    callee.ty = Term::Closure {
        environment: vec![boolean()],
        signature: Box::new(callee.ty.clone()),
    };
    assert!(validate(&value).is_err());
}

#[test]
fn wide_graph_bindings_are_ordered_and_scoped() {
    let mut statements = Vec::new();
    for index in 0..2048 {
        let value = if index == 0 {
            K::Literal(Literal::Bool(true))
        } else {
            K::Local(id(index - 1))
        };
        statements.push(expr(
            Term::Primitive(Primitive::Void),
            K::Let {
                binding: id(index),
                value: Box::new(expr(boolean(), value)),
            },
        ));
    }
    let graph = expr(
        boolean(),
        K::Block {
            statements,
            result: Some(Box::new(expr(boolean(), K::Local(id(2047))))),
        },
    );
    nir::completed_values::validate(&graph, &mut 20000).unwrap();
    assert!(nir::completed_values::validate(&graph, &mut 100).is_err());
    // Adjacent blocks can reuse private identities, but neither exports them.
    let pair = expr(
        Term::Tuple(vec![boolean(), boolean()]),
        K::Tuple(vec![graph.clone(), graph]),
    );
    nir::completed_values::validate(&pair, &mut 40000).unwrap();
    let escaped = expr(
        Term::Tuple(vec![pair.ty.clone(), boolean()]),
        K::Tuple(vec![pair, expr(boolean(), K::Local(id(2047)))]),
    );
    assert!(nir::completed_values::validate(&escaped, &mut 50000).is_err());
}

#[test]
fn captured_data_values_preserve_domains_scope_and_budget_recovery() {
    let parameter = nir::DeclarationId {
        package: 71,
        local: 2,
    };
    let symbolic = expr(
        boolean(),
        K::ComptimeValue(Term::Rigid {
            declaration: parameter,
            sort: nir::Sort::ComptimeValue,
        }),
    );
    validate(&symbolic).unwrap();
    let specialized = symbolic
        .instantiate_parameters(
            &[nir::DeclarationParameter {
                declaration: parameter,
                sort: nir::Sort::ComptimeValue,
            }],
            &[Term::Bool(true)],
        )
        .unwrap();
    validate(&specialized).unwrap();
    assert!(nir::completed_values::validate(&specialized, &mut 1).is_err());
    validate(&specialized).unwrap();
    for invalid in [
        expr(boolean(), K::ComptimeValue(Term::Char('x'))),
        expr(
            boolean(),
            K::ComptimeValue(Term::Bound {
                depth: 0,
                index: 0,
                sort: nir::Sort::ComptimeValue,
            }),
        ),
        expr(
            Term::Primitive(Primitive::Type),
            K::ComptimeValue(Term::Bool(true)),
        ),
        expr(boolean(), K::ComptimeValue(boolean())),
    ] {
        assert!(validate(&invalid).is_err());
    }
}

#[test]
fn completed_error_payloads_validate_domains_and_remain_closed() {
    use nir::error_results::Conversion;
    let packet = Term::ErrorResult {
        errors: vec![boolean()],
        result: Box::new(Term::Primitive(Primitive::Char)),
    };
    for (conversion, payload) in [
        (
            Conversion::Success,
            expr(
                Term::Primitive(Primitive::Char),
                K::Literal(Literal::Char('x')),
            ),
        ),
        (
            Conversion::Failure,
            expr(boolean(), K::Literal(Literal::Bool(false))),
        ),
    ] {
        let value = expr(
            packet.clone(),
            K::ErrorConvert {
                conversion,
                value: Box::new(payload.clone()),
            },
        );
        validate(&value).unwrap();
        assert!(nir::completed_values::validate(&value, &mut 1).is_err());
        let wrong = expr(
            Term::ErrorResult {
                errors: vec![Term::Primitive(Primitive::Integer)],
                result: Box::new(boolean()),
            },
            value.kind.clone(),
        );
        assert!(validate(&wrong).is_err());
        let executable = expr(
            packet.clone(),
            K::ErrorConvert {
                conversion,
                value: Box::new(expr(
                    payload.ty.clone(),
                    K::Call {
                        callee: Box::new(expr(
                            signature(payload.ty.clone()),
                            K::Function(nir::DeclarationId {
                                package: 0,
                                local: 42,
                            }),
                        )),
                        arguments: vec![],
                        parameter_bindings: vec![],
                    },
                )),
            },
        );
        assert!(validate(&executable).is_err());
    }
}

#[test]
fn frozen_newtypes_require_the_exact_declared_representation() {
    let declaration = nir::DeclarationId {
        package: 3,
        local: 1,
    };
    let parameter = nir::DeclarationId {
        package: 3,
        local: 2,
    };
    let definition = nir::newtypes::Newtype {
        declaration,
        name: "Wrapper".into(),
        type_parameters: vec![parameter],
        parameter_types: vec![],
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation: Term::Rigid {
            declaration: parameter,
            sort: nir::Sort::Type,
        },
    };
    let wrapper = Term::Nominal {
        declaration,
        arguments: vec![boolean()],
    };
    let payload = expr(boolean(), K::Literal(Literal::Bool(true)));
    let value = expr(wrapper.clone(), K::NewtypeConvert(Box::new(payload)));
    let check = |value: &Expr| {
        nir::completed_values::validate_with_newtypes(
            value,
            std::iter::once(&definition),
            &mut 10000,
        )
    };
    check(&value).unwrap();
    assert!(
        validate(&value).is_err(),
        "a nominal name is not representation evidence"
    );
    assert!(
        nir::completed_values::validate_with_newtypes(&value, std::iter::once(&definition), &mut 1)
            .is_err()
    );
    let wrong = expr(
        wrapper.clone(),
        K::NewtypeConvert(Box::new(expr(
            Term::Primitive(Primitive::Char),
            K::Literal(Literal::Char('x')),
        ))),
    );
    assert!(check(&wrong).is_err());
    let unwrap = expr(boolean(), K::NewtypeConvert(Box::new(value.clone())));
    assert!(
        check(&unwrap).is_err(),
        "unwrapping is a computation, not reconstruction"
    );
    let packet = expr(
        Term::ErrorResult {
            errors: vec![boolean()],
            result: Box::new(wrapper),
        },
        K::ErrorConvert {
            conversion: nir::error_results::Conversion::Success,
            value: Box::new(value.clone()),
        },
    );
    check(&packet).unwrap();
    assert!(
        nir::completed_values::validate_with_newtypes(
            &value,
            [&definition, &definition].into_iter(),
            &mut 10000
        )
        .is_err()
    );
}

#[test]
fn completed_success_payloads_stay_completed_when_specialization_adds_errors() {
    let t = nir::DeclarationId {
        package: 9,
        local: 1,
    };
    let v = nir::DeclarationId {
        package: 9,
        local: 2,
    };
    let ty = Term::Rigid {
        declaration: t,
        sort: nir::Sort::Type,
    };
    let outer = Term::error_result(vec![Term::Primitive(Primitive::Char)], ty.clone());
    let template = expr(
        outer,
        K::ErrorConvert {
            conversion: nir::error_results::Conversion::Success,
            value: Box::new(expr(
                ty,
                K::ComptimeValue(Term::Rigid {
                    declaration: v,
                    sort: nir::Sort::ComptimeValue,
                }),
            )),
        },
    );
    validate(&template).unwrap();
    let inner = Term::error_result(vec![boolean()], boolean());
    for label in [None, Some(Box::new(boolean()))] {
        let value = Term::ErrorValue {
            ty: Box::new(inner.clone()),
            label,
            value: Box::new(Term::Bool(true)),
        };
        let completed = template
            .instantiate_parameters(
                &[
                    nir::DeclarationParameter {
                        declaration: t,
                        sort: nir::Sort::Type,
                    },
                    nir::DeclarationParameter {
                        declaration: v,
                        sort: nir::Sort::ComptimeValue,
                    },
                ],
                &[inner.clone(), value],
            )
            .unwrap();
        assert!(matches!(
            completed.kind,
            K::ErrorConvert {
                conversion: nir::error_results::Conversion::Embed,
                ..
            }
        ));
        validate(&completed).unwrap();
        let mut missing = completed.clone();
        missing.ty = Term::error_result(vec![Term::Primitive(Primitive::Char)], boolean());
        assert!(
            validate(&missing).is_err(),
            "a result cannot silently drop an error label"
        );
        let mut wrong = completed.clone();
        wrong.ty = Term::error_result(
            vec![boolean(), Term::Primitive(Primitive::Char)],
            Term::Primitive(Primitive::Integer),
        );
        assert!(validate(&wrong).is_err());
    }
}

#[test]
fn completed_product_transport_uses_registered_tuple_layouts() {
    let head = nir::DeclarationId {
        package: 11,
        local: 1,
    };
    let source = Term::ErrorResult {
        errors: vec![boolean(), Term::Primitive(Primitive::Char)],
        result: Box::new(boolean()),
    };
    let target = Term::ErrorResult {
        errors: vec![Term::Primitive(Primitive::Char), boolean()],
        result: Box::new(boolean()),
    };
    let payload = expr(
        source.clone(),
        K::ErrorConvert {
            conversion: nir::error_results::Conversion::Failure,
            value: Box::new(expr(boolean(), K::Literal(Literal::Bool(true)))),
        },
    );
    for nominal in [false, true] {
        let tuple = |field: Term| {
            if nominal {
                Term::NormalForm {
                    head,
                    arguments: vec![Term::TypeListValue(vec![field])],
                    sort: nir::Sort::Type,
                }
            } else {
                Term::Tuple(vec![field])
            }
        };
        let converted = expr(
            tuple(target.clone()),
            K::ErrorConvert {
                conversion: nir::error_results::Conversion::Product,
                value: Box::new(expr(tuple(source.clone()), K::Tuple(vec![payload.clone()]))),
            },
        );
        let checked = |value: &Expr, fuel: &mut usize| {
            nir::completed_values::validate_with_layouts(
                value,
                std::iter::empty(),
                &|id| id == head,
                fuel,
            )
        };
        checked(&converted, &mut 10000).unwrap();
        assert_eq!(
            validate(&converted).is_ok(),
            !nominal,
            "opaque heads require independent tuple evidence"
        );
        assert!(checked(&converted, &mut 1).is_err());
        let mut wrong = converted.clone();
        wrong.ty = tuple(Term::error_result(vec![boolean()], boolean()));
        assert!(checked(&wrong, &mut 10000).is_err());
        let mut wrong = converted.clone();
        wrong.ty = tuple(Term::error_result(
            vec![boolean(), Term::Primitive(Primitive::Char)],
            Term::Primitive(Primitive::Integer),
        ));
        assert!(checked(&wrong, &mut 10000).is_err());
    }
    let record = |field: Term| Term::Record(vec![("value".into(), field)]);
    let converted = expr(
        record(target),
        K::ErrorConvert {
            conversion: nir::error_results::Conversion::Product,
            value: Box::new(expr(
                record(source),
                K::AnonymousRecord {
                    fields: vec![payload],
                },
            )),
        },
    );
    validate(&converted).unwrap();
    let mut wrong = converted.clone();
    let Term::Record(fields) = &mut wrong.ty else {
        unreachable!()
    };
    fields[0].0 = "renamed".into();
    assert!(validate(&wrong).is_err());
}

#[test]
fn completed_literal_annotations_cannot_forge_payload_domains() {
    for (literal, domain) in [
        (Literal::Bool(true), Primitive::Bool),
        (Literal::Char('x'), Primitive::Char),
        (Literal::String("text".into()), Primitive::Str),
        (Literal::Symbol("tag".into()), Primitive::Symbol),
        (Literal::Integer("123".into()), Primitive::Integer),
        (Literal::Float("1.5".into()), Primitive::Float),
        (Literal::Float32(0), Primitive::F32),
        (Literal::Float64(0), Primitive::F64),
        (Literal::SignedInteger(-128), Primitive::I8),
        (Literal::UnsignedInteger(255), Primitive::U8),
        (
            Literal::BitVec {
                width: 1,
                words: vec![1],
            },
            Primitive::BitVec(1),
        ),
    ] {
        validate(&expr(Term::Primitive(domain), K::Literal(literal.clone()))).unwrap();
        for wrong in [Primitive::Bool, Primitive::Char, Primitive::Type] {
            if wrong != domain {
                assert!(
                    validate(&expr(Term::Primitive(wrong), K::Literal(literal.clone()))).is_err()
                );
            }
        }
    }
    for (literal, domain) in [
        (Literal::SignedInteger(128), Primitive::I8),
        (Literal::UnsignedInteger(256), Primitive::U8),
        (Literal::SignedInteger(1), Primitive::U8),
    ] {
        assert!(validate(&expr(Term::Primitive(domain), K::Literal(literal))).is_err());
    }
    assert!(validate(&expr(boolean(), K::Unit)).is_err());
    assert!(validate(&expr(boolean(), K::TypeValue(boolean()))).is_err());
    assert!(
        validate(&expr(
            Term::Primitive(Primitive::Type),
            K::TypeValue(Term::Bool(true))
        ))
        .is_err()
    );
}

#[test]
fn completed_literals_check_spelling_padding_and_shared_budget() {
    for spelling in ["0", "-0x123", "0b1010", "123456789012345678901234567890"] {
        validate(&expr(
            Term::Primitive(Primitive::Integer),
            K::Literal(Literal::Integer(spelling.into())),
        ))
        .unwrap();
    }
    for spelling in ["-1.25", "0.0", "1.0e12345678901234567890"] {
        validate(&expr(
            Term::Primitive(Primitive::Float),
            K::Literal(Literal::Float(spelling.into())),
        ))
        .unwrap();
    }
    for (literal, domain) in [
        (Literal::Integer("--1".into()), Primitive::Integer),
        (Literal::Integer("0xZ".into()), Primitive::Integer),
        (Literal::Float("1.0e".into()), Primitive::Float),
        (Literal::Float("NaN".into()), Primitive::Float),
        (
            Literal::BitVec {
                width: 1,
                words: vec![2],
            },
            Primitive::BitVec(1),
        ),
        (
            Literal::BitVec {
                width: 33,
                words: vec![1],
            },
            Primitive::BitVec(33),
        ),
        (
            Literal::BitVec {
                width: 0,
                words: vec![0],
            },
            Primitive::BitVec(0),
        ),
    ] {
        assert!(validate(&expr(Term::Primitive(domain), K::Literal(literal))).is_err());
    }
    for (width, words) in [(0, vec![]), (33, vec![u32::MAX, 1])] {
        validate(&expr(
            Term::Primitive(Primitive::BitVec(width)),
            K::Literal(Literal::BitVec { width, words }),
        ))
        .unwrap();
    }
    let value = expr(
        Term::Primitive(Primitive::Integer),
        K::Literal(Literal::Integer("9".repeat(1000))),
    );
    let mut fuel = 64;
    assert!(nir::completed_values::validate(&value, &mut fuel).is_err());
    assert!(fuel < 64, "failed validation must account for work");
    assert!(nir::completed_values::validate(&value, &mut 100000).is_ok());
}

#[test]
fn completed_structural_products_require_exact_layouts() {
    let flag = expr(boolean(), K::Literal(Literal::Bool(true)));
    let tuple = expr(Term::Tuple(vec![boolean()]), K::Tuple(vec![flag.clone()]));
    validate(&tuple).unwrap();
    for wrong in [
        expr(boolean(), tuple.kind.clone()),
        expr(Term::Tuple(vec![]), tuple.kind.clone()),
        expr(
            Term::Tuple(vec![Term::Primitive(Primitive::Char)]),
            tuple.kind.clone(),
        ),
        expr(tuple.ty.clone(), K::Tuple(vec![])),
    ] {
        assert!(validate(&wrong).is_err());
    }
    let record = expr(
        Term::Record(vec![("flag".into(), boolean())]),
        K::AnonymousRecord {
            fields: vec![flag.clone()],
        },
    );
    validate(&record).unwrap();
    for wrong in [
        expr(boolean(), record.kind.clone()),
        expr(Term::Record(vec![]), record.kind.clone()),
        expr(
            Term::Record(vec![("flag".into(), Term::Primitive(Primitive::Char))]),
            record.kind.clone(),
        ),
        expr(
            Term::Record(vec![("flag".into(), boolean()), ("flag".into(), boolean())]),
            K::AnonymousRecord {
                fields: vec![flag.clone(), flag.clone()],
            },
        ),
    ] {
        assert!(validate(&wrong).is_err());
    }
    let head = nir::DeclarationId {
        package: 12,
        local: 1,
    };
    let ty = Term::NormalForm {
        head,
        sort: nir::Sort::Type,
        arguments: vec![Term::TypeListValue(vec![boolean()])],
    };
    let formal = expr(ty, K::Tuple(vec![flag]));
    assert!(validate(&formal).is_err());
    nir::completed_values::validate_with_layouts(
        &formal,
        std::iter::empty(),
        &|id| id == head,
        &mut 10000,
    )
    .unwrap();
    assert!(
        nir::completed_values::validate_with_layouts(
            &formal,
            std::iter::empty(),
            &|_| false,
            &mut 10000
        )
        .is_err()
    );
}

#[test]
fn nominal_completed_values_check_instantiated_layouts_and_constructor_identity() {
    use nir::normal_forms::NominalValueDeclaration as D;
    use nir::program::{Enum, Struct, Variant, VariantShape};
    use nir::{DeclarationId, Sort};
    let decl = |local| DeclarationId { package: 1, local };
    let parameter = Term::Rigid {
        declaration: decl(3),
        sort: Sort::Type,
    };
    let record = Struct {
        declaration: decl(1),
        name: "Box".into(),
        type_parameters: vec![decl(3)],
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        fields: vec![("value".into(), parameter.clone())],
    };
    let enumeration = Enum {
        declaration: decl(2),
        name: "Choice".into(),
        type_parameters: vec![decl(3)],
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        groups: vec![],
        encoding: None,
        is_intrinsic: false,
        symbol_encoding: None,
        variants: vec![
            Variant {
                name: "empty".into(),
                shape: VariantShape::Unit,
                fields: vec![],
            },
            Variant {
                name: "some".into(),
                shape: VariantShape::Tuple,
                fields: vec![parameter],
            },
        ],
    };
    let lookup = |id, _: &mut usize| {
        if id == record.declaration {
            Some(D::Struct(&record))
        } else if id == enumeration.declaration {
            Some(D::Enum(&enumeration))
        } else {
            None
        }
    };
    let check = |value: &Expr, fuel: &mut usize| {
        nir::completed_values::validate_with_declarations(
            value,
            std::iter::empty(),
            &|_| false,
            &lookup,
            fuel,
        )
    };
    let nominal = |owner, argument| Term::Nominal {
        declaration: decl(owner),
        arguments: vec![argument],
    };
    let field = expr(boolean(), K::Literal(Literal::Bool(true)));
    let good_record = expr(
        nominal(1, boolean()),
        K::Record {
            declaration: decl(1),
            fields: vec![field.clone()],
        },
    );
    let good_variant = expr(
        nominal(2, boolean()),
        K::Variant {
            declaration: decl(2),
            index: 1,
            fields: vec![field.clone()],
        },
    );
    for good in [&good_record, &good_variant] {
        assert!(check(good, &mut 10000).is_ok());
        assert!(validate(good).is_err(), "layout evidence is mandatory");
        assert!(check(good, &mut 1).is_err());
        assert!(
            check(good, &mut 10000).is_ok(),
            "budget failure must not poison retry"
        );
    }
    let invalid = [
        expr(
            nominal(1, boolean()),
            K::Record {
                declaration: decl(2),
                fields: vec![field.clone()],
            },
        ),
        expr(
            nominal(1, boolean()),
            K::Record {
                declaration: decl(1),
                fields: vec![],
            },
        ),
        expr(
            nominal(1, Term::Primitive(Primitive::Char)),
            good_record.kind.clone(),
        ),
        expr(
            nominal(1, boolean()),
            K::Variant {
                declaration: decl(1),
                index: 0,
                fields: vec![field.clone()],
            },
        ),
        expr(
            nominal(2, boolean()),
            K::Record {
                declaration: decl(2),
                fields: vec![field.clone()],
            },
        ),
        expr(
            nominal(2, boolean()),
            K::Variant {
                declaration: decl(2),
                index: 2,
                fields: vec![],
            },
        ),
        expr(
            nominal(2, boolean()),
            K::Variant {
                declaration: decl(2),
                index: 0,
                fields: vec![field],
            },
        ),
        expr(
            nominal(2, Term::Bool(true)),
            K::Variant {
                declaration: decl(2),
                index: 0,
                fields: vec![],
            },
        ),
        expr(
            Term::Nominal {
                declaration: decl(1),
                arguments: vec![],
            },
            good_record.kind.clone(),
        ),
    ];
    for bad in invalid {
        assert!(check(&bad, &mut 10000).is_err(), "accepted {bad}");
    }
    let nested = expr(
        Term::Tuple(vec![good_record.ty.clone(), good_variant.ty.clone()]),
        K::Tuple(vec![good_record, good_variant]),
    );
    assert!(check(&nested, &mut 10000).is_ok());
}
