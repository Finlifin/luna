use nir::program::{CallArgument, Expr, ExprKind as K, Literal, LocalId, Pattern, SourceRange};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term};

fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn local(index: u32) -> LocalId {
    LocalId { owner: 1, index }
}
fn expr(kind: K, ty: Term) -> Expr {
    Expr {
        kind,
        ty,
        source: SourceRange { start: 0, end: 1 },
    }
}
fn read(index: u32) -> Expr {
    expr(K::Local(local(index)), boolean())
}
fn value(flag: bool) -> Expr {
    expr(K::Literal(Literal::Bool(flag)), boolean())
}
fn bind(index: u32, value: Expr) -> Expr {
    expr(
        K::Let {
            binding: local(index),
            value: Box::new(value),
        },
        Term::Primitive(Primitive::Void),
    )
}
fn callable(id: u32, stages: &[Sort], result: Term, ct: bool) -> Expr {
    let ty = Term::Function {
        parameters: stages
            .iter()
            .map(|&sort| Parameter {
                sort,
                ty: boolean(),
            })
            .collect(),
        result: Box::new(result),
    };
    expr(
        K::Function(DeclarationId {
            package: 1,
            local: id,
        }),
        if ct {
            Term::ComptimeFunction(Box::new(ty))
        } else {
            ty
        },
    )
}
fn call(callee: Expr, values: Vec<Expr>, ty: Term) -> Expr {
    expr(
        K::Call {
            callee: Box::new(callee),
            arguments: values
                .into_iter()
                .enumerate()
                .map(|(parameter, value)| CallArgument {
                    passing: Default::default(),
                    parameter,
                    value,
                    is_default: false,
                })
                .collect(),
            parameter_bindings: vec![],
        },
        ty,
    )
}

#[test]
fn selected_packet_has_one_producer_and_preserves_runtime_siblings() {
    let pair = Term::Tuple(vec![boolean(), boolean()]);
    let producer = call(
        callable(10, &[Sort::Runtime], pair.clone(), true),
        vec![read(0)],
        pair,
    );
    let unpack = expr(
        K::LetPattern {
            pattern: Pattern::Tuple(
                vec![1, 2]
                    .into_iter()
                    .map(|id| Pattern::Binding {
                        binding: local(id),
                        inner: Box::new(Pattern::Wild),
                    })
                    .collect(),
            ),
            value: Box::new(producer),
            guard: None,
            otherwise: None,
        },
        Term::Primitive(Primitive::Void),
    );
    let runtime = bind(
        3,
        call(callable(11, &[], boolean(), false), vec![], boolean()),
    );
    let mut input = expr(
        K::CallPreparation {
            statements: vec![
                bind(0, value(true)),
                unpack,
                runtime.clone(),
                bind(4, read(1)),
                bind(5, read(2)),
            ],
            result: Some(Box::new(call(
                callable(
                    12,
                    &[Sort::ComptimeValue, Sort::Runtime, Sort::Runtime],
                    boolean(),
                    false,
                ),
                vec![read(4), read(5), read(3)],
                boolean(),
            ))),
        },
        boolean(),
    );
    let plan = iir::staging::preparation(&input, &mut 10000)
        .unwrap()
        .unwrap();
    assert_eq!(plan.indices.into_iter().collect::<Vec<_>>(), vec![0, 1, 3]);
    let plan = iir::staging::preparation(&input, &mut 10000)
        .unwrap()
        .unwrap();
    let mut producers = 0;
    iir::visit::expression(&plan.expression, &mut 10000, |expr, _| {
        if matches!(expr.kind,K::Function(id) if id.local==10) {
            producers += 1;
        }
        assert!(!matches!(expr.kind,K::Function(id) if id.local==11));
        Ok(())
    })
    .unwrap();
    assert_eq!(producers, 1);
    let completed = expr(
        K::Tuple(vec![value(false), value(true)]),
        plan.expression.ty.clone(),
    );
    plan.apply(&mut input, completed, &mut 10000).unwrap();
    assert!(
        iir::staging::preparation(&input, &mut 10000)
            .unwrap()
            .is_none()
    );
    let K::CallPreparation {
        statements,
        result: Some(result),
    } = &input.kind
    else {
        panic!()
    };
    assert_eq!(statements.len(), 3);
    assert_eq!(statements[1], runtime);
    assert!(matches!(&statements[0].kind,K::Let {binding,..} if *binding==local(2)));
    let K::Call { arguments, .. } = &result.kind else {
        panic!()
    };
    assert_eq!(arguments[0].value, value(true));
    assert_eq!(arguments[1].value, read(5));
    assert!(iir::staging::preparation(&input, &mut 0).is_err());
}

#[test]
fn selection_only_type_bindings_do_not_become_runtime_storage() {
    let ty = Term::Primitive(Primitive::Type);
    let type_value = expr(K::TypeValue(boolean()), ty.clone());
    let callee = expr(
        K::Function(DeclarationId {
            package: 1,
            local: 30,
        }),
        Term::Function {
            parameters: vec![Parameter {
                sort: Sort::ComptimeValue,
                ty: ty.clone(),
            }],
            result: Box::new(boolean()),
        },
    );
    let mut input = expr(
        K::CallPreparation {
            statements: vec![bind(0, type_value.clone())],
            result: Some(Box::new(call(
                callee,
                vec![expr(K::Local(local(0)), ty.clone())],
                boolean(),
            ))),
        },
        boolean(),
    );
    let plan = iir::staging::preparation(&input, &mut 1000)
        .unwrap()
        .unwrap();
    let completed = expr(K::Tuple(vec![type_value]), Term::Tuple(vec![ty]));
    plan.apply(&mut input, completed, &mut 1000).unwrap();
    let K::CallPreparation { statements, .. } = input.kind else {
        panic!()
    };
    assert!(statements.is_empty());
}

#[test]
fn bound_tuple_preparation_uses_only_the_declared_layout_role() {
    use nir::normal_forms::{Head, Kind, Parameter as Input, Role};
    let id = DeclarationId {
        package: 1,
        local: 40,
    };
    let head = Head {
        collector: None,
        role: Role::Tuple,
        declaration: id,
        requirements: vec![],
        parameters: vec![Input {
            declaration: DeclarationId {
                package: 1,
                local: 42,
            },
            binding: local(42),
            kind: Kind::Value,
            ty: Term::NormalForm {
                head: DeclarationId {
                    package: 1,
                    local: 41,
                },
                sort: Sort::Type,
                arguments: vec![Term::Primitive(Primitive::Type)],
            },
        }],
    };
    let pair = Term::NormalForm {
        head: id,
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(vec![boolean(), boolean()])],
    };
    let unpack = expr(
        K::LetPattern {
            pattern: Pattern::Tuple(
                vec![1, 2]
                    .into_iter()
                    .map(|id| Pattern::Binding {
                        binding: local(id),
                        inner: Box::new(Pattern::Wild),
                    })
                    .collect(),
            ),
            value: Box::new(call(
                callable(10, &[], pair.clone(), true),
                vec![],
                pair.clone(),
            )),
            guard: None,
            otherwise: None,
        },
        Term::Primitive(Primitive::Void),
    );
    let mut input = expr(
        K::CallPreparation {
            statements: vec![unpack, bind(3, read(1))],
            result: Some(Box::new(call(
                callable(12, &[Sort::ComptimeValue, Sort::Runtime], boolean(), false),
                vec![read(3), read(2)],
                boolean(),
            ))),
        },
        boolean(),
    );
    let plan = iir::staging::preparation_with_layouts(&[head.clone()], &input, &mut 10000)
        .unwrap()
        .unwrap();
    assert_eq!(plan.indices.iter().copied().collect::<Vec<_>>(), vec![0, 1]);
    let mut producers = 0;
    iir::visit::expression(&plan.expression,&mut 10000,|expr,_| {
        if matches!(&expr.kind,K::Call {callee,..} if matches!(callee.kind,K::Function(id) if id.local==10)) {
            producers+=1;
            assert_eq!(expr.ty,pair);
        }
        Ok(())
    }).unwrap();
    assert_eq!(producers, 1);
    let mut opaque = head.clone();
    opaque.role = Role::Opaque;
    let opaque_plan = iir::staging::preparation_with_layouts(&[opaque], &input, &mut 10000)
        .unwrap()
        .unwrap();
    assert!(!opaque_plan.indices.contains(&0));
    assert!(iir::staging::preparation_with_layouts(&[head.clone()], &input, &mut 0).is_err());
    let mut malformed = input.clone();
    if let K::CallPreparation { statements, .. } = &mut malformed.kind {
        if let K::LetPattern {
            pattern: Pattern::Tuple(fields),
            ..
        } = &mut statements[0].kind
        {
            fields.push(Pattern::Wild);
        }
    }
    assert!(
        iir::staging::preparation_with_layouts(&[head.clone()], &malformed, &mut 10000).is_err()
    );
    let completed = expr(
        K::Tuple(vec![value(false), value(true)]),
        plan.expression.ty.clone(),
    );
    plan.apply_with_layouts(&[head], &mut input, completed, &mut 10000)
        .unwrap();
    let K::CallPreparation {
        statements,
        result: Some(result),
    } = &input.kind
    else {
        panic!("preparation")
    };
    assert_eq!(statements.len(), 1);
    let K::Call { arguments, .. } = &result.kind else {
        panic!("call")
    };
    assert_eq!(arguments[0].value, value(true));
    assert_eq!(arguments[1].value, read(2));
}

#[test]
fn nested_tuple_preparation_keeps_live_fields_and_discards_wildcards() {
    let nested = Term::Tuple(vec![boolean(), Term::Tuple(vec![boolean(), boolean()])]);
    let unpack = expr(
        K::LetPattern {
            pattern: Pattern::Tuple(vec![
                Pattern::Binding {
                    binding: local(1),
                    inner: Box::new(Pattern::Wild),
                },
                Pattern::Tuple(vec![
                    Pattern::Wild,
                    Pattern::Binding {
                        binding: local(2),
                        inner: Box::new(Pattern::Wild),
                    },
                ]),
            ]),
            value: Box::new(call(
                callable(10, &[], nested.clone(), true),
                vec![],
                nested,
            )),
            guard: None,
            otherwise: None,
        },
        Term::Primitive(Primitive::Void),
    );
    let mut input = expr(
        K::CallPreparation {
            statements: vec![unpack],
            result: Some(Box::new(call(
                callable(12, &[Sort::ComptimeValue, Sort::Runtime], boolean(), false),
                vec![read(1), read(2)],
                boolean(),
            ))),
        },
        boolean(),
    );
    for case in 0..3 {
        let mut invalid = input.clone();
        let K::CallPreparation { statements, .. } = &mut invalid.kind else {
            unreachable!()
        };
        let K::LetPattern {
            pattern: Pattern::Tuple(outer),
            ..
        } = &mut statements[0].kind
        else {
            unreachable!()
        };
        let Pattern::Tuple(inner) = &mut outer[1] else {
            unreachable!()
        };
        let expected = match case {
            0 => {
                inner.pop();
                "preparation tuple arity mismatch"
            }
            1 => {
                inner[1] = Pattern::Binding {
                    binding: local(1),
                    inner: Box::new(Pattern::Wild),
                };
                "duplicate preparation binding"
            }
            _ => {
                inner[0] = Pattern::Literal(Literal::Bool(true));
                "unsupported preparation subpattern"
            }
        };
        let failure = match iir::staging::preparation(&invalid, &mut 10000) {
            Err(error) => error,
            _ => panic!("invalid pattern accepted"),
        };
        assert_eq!(failure, expected);
    }
    assert!(iir::staging::preparation(&input, &mut 1).is_err());
    let plan = iir::staging::preparation(&input, &mut 10000)
        .unwrap()
        .unwrap();
    assert_eq!(plan.indices.iter().copied().collect::<Vec<_>>(), vec![0]);
    assert_eq!(plan.expression.ty, Term::Tuple(vec![boolean(), boolean()]));
    let completed = expr(
        K::Tuple(vec![value(true), value(false)]),
        plan.expression.ty.clone(),
    );
    plan.apply(&mut input, completed, &mut 10000).unwrap();
    let K::CallPreparation {
        statements,
        result: Some(result),
    } = &input.kind
    else {
        panic!("preparation")
    };
    assert_eq!(statements, &vec![bind(2, value(false))]);
    let K::Call { arguments, .. } = &result.kind else {
        panic!("call")
    };
    assert_eq!(arguments[0].value, value(true));
    assert_eq!(arguments[1].value, read(2));
}

#[test]
fn preparation_delivers_function_identity_but_rejects_unexecuted_calls() {
    let callback = callable(30, &[Sort::Runtime], boolean(), false);
    let tuple_ty = Term::Tuple(vec![boolean(), callback.ty.clone()]);
    let packet = expr(
        K::Tuple(vec![value(true), callback.clone()]),
        tuple_ty.clone(),
    );
    let mut callee = callable(31, &[Sort::ComptimeValue, Sort::Runtime], boolean(), false);
    let Term::Function { parameters, .. } = &mut callee.ty else {
        unreachable!()
    };
    parameters[1].ty = callback.ty.clone();
    let original = expr(
        K::CallPreparation {
            statements: vec![expr(
                K::LetPattern {
                    pattern: Pattern::Tuple(
                        vec![1, 2]
                            .into_iter()
                            .map(|index| Pattern::Binding {
                                binding: local(index),
                                inner: Box::new(Pattern::Wild),
                            })
                            .collect(),
                    ),
                    value: Box::new(packet),
                    guard: None,
                    otherwise: None,
                },
                Term::Primitive(Primitive::Void),
            )],
            result: Some(Box::new(call(
                callee,
                vec![read(1), expr(K::Local(local(2)), callback.ty.clone())],
                boolean(),
            ))),
        },
        boolean(),
    );
    let mut input = original.clone();
    let plan = iir::staging::preparation(&input, &mut 10000)
        .unwrap()
        .unwrap();
    let completed = expr(
        K::Tuple(vec![value(true), callback.clone()]),
        tuple_ty.clone(),
    );
    plan.apply(&mut input, completed, &mut 10000).unwrap();
    let K::CallPreparation {
        statements,
        result: Some(result),
    } = &input.kind
    else {
        unreachable!()
    };
    assert_eq!(statements.len(), 1);
    assert!(
        matches!(&statements[0].kind,K::Let {binding,value} if *binding==local(2) && matches!(value.kind,K::Function(_)))
    );
    let K::Call { arguments, .. } = &result.kind else {
        unreachable!()
    };
    assert!(matches!(
        arguments[0].value.kind,
        K::Literal(Literal::Bool(true))
    ));
    let mut input = original;
    let plan = iir::staging::preparation(&input, &mut 10000)
        .unwrap()
        .unwrap();
    let unevaluated = call(
        callable(32, &[], callback.ty.clone(), true),
        vec![],
        callback.ty.clone(),
    );
    let completed = expr(K::Tuple(vec![value(true), unevaluated]), tuple_ty);
    assert!(plan.apply(&mut input, completed, &mut 10000).is_err());
}
