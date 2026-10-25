use nir::type_computations::{Error, Input, Invocation};
use nir::{DeclarationId, Primitive, Sort, Term};
fn target() -> DeclarationId {
    DeclarationId {
        package: 1,
        local: 2,
    }
}
fn slot(index: usize, sort: Sort) -> Term {
    Term::Bound {
        depth: 0,
        index,
        sort,
    }
}
fn input(value: Term) -> Input {
    Input {
        ty: Term::Primitive(Primitive::Type),
        value,
    }
}

#[test]
fn residuals_keep_stable_binders_and_nested_normal_form_heads() {
    let head = DeclarationId {
        package: 1,
        local: 3,
    };
    let ty = Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(vec![slot(0, Sort::Type)])],
    };
    let plan = Invocation::new(
        target(),
        vec![Sort::Type, Sort::Runtime],
        vec![input(ty.clone())],
        &mut 100,
    )
    .unwrap();
    assert_eq!(
        plan.dependencies().iter().copied().collect::<Vec<_>>(),
        vec![0]
    );
    assert_eq!(plan.inputs()[0].value, ty);
    let bound = plan.inputs()[0]
        .value
        .substitute_partial(&[Some(Term::Primitive(Primitive::Bool)), None])
        .unwrap();
    assert_eq!(
        bound,
        Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![Term::TypeListValue(vec![Term::Primitive(Primitive::Bool)])]
        }
    );
    assert!(
        plan.to_string()
            .starts_with("(residual-type-invocation (target")
    );
    assert!(plan.to_string().ends_with("(dependencies 0))"));
    let callback = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
        result: Box::new(Term::Bound {
            depth: 1,
            index: 0,
            sort: Sort::Type,
        }),
    };
    let nested =
        Invocation::new(target(), vec![Sort::Type], vec![input(callback)], &mut 100).unwrap();
    assert_eq!(
        nested.dependencies().iter().copied().collect::<Vec<_>>(),
        vec![0]
    );
}

#[test]
fn residuals_reject_invalid_scope_runtime_reads_and_excessive_structure() {
    assert!(matches!(
        Invocation::new(
            target(),
            vec![Sort::Runtime],
            vec![input(slot(0, Sort::Runtime))],
            &mut 100
        ),
        Err(Error::RuntimeInput(0))
    ));
    assert!(matches!(
        Invocation::new(
            target(),
            vec![Sort::Type],
            vec![input(slot(1, Sort::Type))],
            &mut 100
        ),
        Err(Error::Scope(_))
    ));
    assert!(matches!(
        Invocation::new(
            target(),
            vec![Sort::Type],
            vec![input(slot(0, Sort::ComptimeValue))],
            &mut 100
        ),
        Err(Error::Scope(_))
    ));
    assert!(matches!(
        Invocation::new(
            target(),
            vec![],
            vec![Input {
                ty: Term::Bool(true),
                value: Term::Bool(true)
            }],
            &mut 100
        ),
        Err(Error::Scope(_))
    ));
    let mut deep = Term::Primitive(Primitive::Bool);
    for _ in 0..140 {
        deep = Term::Pointer(Box::new(deep));
    }
    assert!(matches!(
        Invocation::new(target(), vec![], vec![input(deep)], &mut 1000),
        Err(Error::Budget)
    ));
    assert!(matches!(
        Invocation::new(
            target(),
            vec![],
            vec![input(Term::TypeListValue(vec![
                Term::Primitive(
                    Primitive::Bool
                );
                100
            ]))],
            &mut 10
        ),
        Err(Error::Budget)
    ));
}

#[test]
fn capture_rejects_ambiguous_routes_unknown_bindings_and_effectful_operands() {
    use nir::program::{CallArgument, Expr, ExprKind as K, LocalId, SourceRange};
    let source = SourceRange { start: 10, end: 20 };
    let boolean = Term::Primitive(Primitive::Bool);
    let parameter = nir::Parameter {
        sort: Sort::Runtime,
        ty: boolean.clone(),
    };
    let local = LocalId { owner: 1, index: 0 };
    let captures = vec![(
        local,
        nir::Parameter {
            sort: Sort::ComptimeValue,
            ty: boolean.clone(),
        },
    )];
    let call = Expr {
        source,
        ty: Term::Primitive(Primitive::Type),
        kind: K::Call {
            callee: Box::new(Expr {
                source,
                ty: Term::Function {
                    parameters: vec![parameter.clone(), parameter],
                    result: Box::new(Term::Primitive(Primitive::Type)),
                }
                .with_comptime_phase(true),
                kind: K::Function(target()),
            }),
            arguments: vec![
                CallArgument {
                    parameter: 1,
                    value: Expr {
                        source,
                        ty: boolean.clone(),
                        kind: K::Literal(nir::program::Literal::Bool(false)),
                    },
                    is_default: false,
                },
                CallArgument {
                    parameter: 0,
                    value: Expr {
                        source,
                        ty: boolean.clone(),
                        kind: K::Local(local),
                    },
                    is_default: false,
                },
            ],
            parameter_bindings: vec![
                LocalId { owner: 2, index: 0 },
                LocalId { owner: 2, index: 1 },
            ],
        },
    };
    let plan = Invocation::from_call(&call, &captures, &mut 100).unwrap();
    assert_eq!(plan.inputs()[0].value, slot(0, Sort::ComptimeValue));
    assert_eq!(plan.inputs()[1].value, Term::Bool(false));
    for value in [
        Term::Bool(true),
        Term::Char('x'),
        Term::Primitive(Primitive::Bool),
        slot(0, Sort::ComptimeValue),
    ] {
        let mut captured = call.clone();
        let K::Call { arguments, .. } = &mut captured.kind else {
            unreachable!()
        };
        arguments[0].value.kind = K::ComptimeValue(value.clone());
        let result = Invocation::from_call(&captured, &captures, &mut 1000);
        assert_eq!(result.is_ok(), value == Term::Bool(true), "{value}");
    }
    for mode in 0..4 {
        let mut invalid = call.clone();
        let K::Call { arguments, .. } = &mut invalid.kind else {
            unreachable!()
        };
        match mode {
            0 => arguments[0].parameter = 0,
            1 => arguments[0].parameter = 3,
            2 => {
                arguments[1].value.kind = K::Local(LocalId {
                    owner: 99,
                    index: 0,
                })
            }
            _ => {
                arguments[1].value.kind = K::Block {
                    statements: vec![],
                    result: Some(Box::new(Expr {
                        source,
                        ty: boolean.clone(),
                        kind: K::Local(local),
                    })),
                }
            }
        }
        assert!(
            Invocation::from_call(&invalid, &captures, &mut 100).is_err(),
            "mode {mode}"
        );
    }
    assert!(matches!(
        Invocation::from_call(&call, &[captures[0].clone(), captures[0].clone()], &mut 100),
        Err(Error::DuplicateBinding(_))
    ));
    assert!(matches!(
        Invocation::from_call(&call, &captures, &mut 0),
        Err(Error::Budget)
    ));
    let mut runtime = captures.clone();
    runtime[0].1.sort = Sort::Runtime;
    assert!(matches!(
        Invocation::from_call(&call, &runtime, &mut 100),
        Err(Error::RuntimeInput(0))
    ));
}

#[test]
fn expression_entries_validate_captures_annotations_and_preserve_owned_code() {
    use nir::program::{Expr, ExprKind as K, FunctionPhase, LocalId, SourceRange};
    use nir::type_computations::expression_entry;
    let body = Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
    };
    let binding = LocalId { owner: 1, index: 0 };
    let parameter = nir::Parameter {
        sort: Sort::Runtime,
        ty: Term::Primitive(Primitive::Bool),
    };
    let entry = expression_entry(
        target(),
        "defer\"entry".into(),
        body.clone(),
        vec![(binding, parameter.clone())],
        &mut 100,
    )
    .unwrap();
    assert_eq!(entry.phase, FunctionPhase::Comptime);
    assert_eq!(entry.body, body);
    assert_eq!(entry.parameters, vec![binding]);
    assert!(entry.to_string().contains("defer\\\"entry"));
    assert!(
        matches!(expression_entry(target(),"duplicate".into(),body.clone(),vec![(binding,parameter.clone()),(binding,parameter)],&mut 100),Err(Error::DuplicateBinding(id)) if id==binding)
    );
    let malformed = Expr {
        source: body.source,
        ty: Term::Primitive(Primitive::Type),
        kind: K::Block {
            statements: vec![Expr {
                source: body.source,
                ty: Term::Bool(false),
                kind: K::Unit,
            }],
            result: Some(Box::new(body.clone())),
        },
    };
    assert!(matches!(
        expression_entry(target(), "invalid".into(), malformed, vec![], &mut 100),
        Err(Error::Structure(_))
    ));
    assert!(expression_entry(target(), "budget".into(), body, vec![], &mut 0).is_err());
}

#[test]
fn compact_entries_keep_transitive_domains_and_nested_binder_identity() {
    use nir::program::{Expr, ExprKind as K, LocalId, SourceRange};
    use nir::type_computations::capture_entry;
    let binding = |index| LocalId { owner: 9, index };
    let parameter = |sort, ty| nir::Parameter { sort, ty };
    let available = vec![
        (
            binding(0),
            parameter(Sort::Runtime, Term::Primitive(Primitive::Bool)),
        ),
        (
            binding(1),
            parameter(Sort::Type, Term::Primitive(Primitive::Type)),
        ),
        (
            binding(2),
            parameter(Sort::ComptimeValue, slot(1, Sort::Type)),
        ),
    ];
    let body = Expr {
        source: SourceRange { start: 0, end: 1 },
        ty: slot(1, Sort::Type),
        kind: K::Local(binding(2)),
    };
    let captured = capture_entry(
        target(),
        "compact".into(),
        body.clone(),
        &available,
        &mut 1000,
    )
    .unwrap();
    assert_eq!(captured.slots, vec![1, 2]);
    assert_eq!(captured.function.parameters, vec![binding(1), binding(2)]);
    assert_eq!(captured.function.body.ty, slot(0, Sort::Type));
    let Term::Function { parameters, .. } = captured.function.signature.callable_signature() else {
        panic!("function")
    };
    assert_eq!(parameters[1].ty, slot(0, Sort::Type));
    assert!(
        captured
            .to_string()
            .starts_with("(captured-comptime-entry (slots 1 2)")
    );
    // Slot 1 occurs only in slot 2's domain, not anywhere in this body.
    let dependent = Expr {
        source: body.source,
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(Term::NormalForm {
            head: target(),
            sort: Sort::Type,
            arguments: vec![slot(2, Sort::ComptimeValue)],
        }),
    };
    let captured = capture_entry(
        target(),
        "domain-dependency".into(),
        dependent,
        &available,
        &mut 1000,
    )
    .unwrap();
    assert_eq!(captured.slots, vec![1, 2]);
    let call = captured.invocation(&mut 1000).unwrap();
    assert_eq!(
        call.scope(),
        &[Sort::Runtime, Sort::Type, Sort::ComptimeValue]
    );
    assert_eq!(
        call.dependencies().iter().copied().collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(call.inputs()[0].value, slot(1, Sort::Type));
    assert_eq!(call.inputs()[1].ty, slot(1, Sort::Type));
    assert_eq!(call.inputs()[1].value, slot(2, Sort::ComptimeValue));
    assert!(matches!(captured.invocation(&mut 0), Err(Error::Budget)));
    for slots in [vec![1], vec![1, 1], vec![1, 9], vec![0, 2]] {
        let mut invalid = captured.clone();
        invalid.slots = slots;
        assert!(invalid.invocation(&mut 1000).is_err());
    }
    let K::TypeValue(Term::NormalForm { arguments, .. }) = captured.function.body.kind else {
        panic!("normal form")
    };
    assert_eq!(arguments, vec![slot(1, Sort::ComptimeValue)]);
    let nested = Term::Function {
        parameters: vec![parameter(Sort::Type, Term::Primitive(Primitive::Type))],
        result: Box::new(Term::NormalForm {
            head: target(),
            sort: Sort::Type,
            arguments: vec![Term::TypeListValue(vec![
                slot(0, Sort::Type),
                Term::Bound {
                    depth: 1,
                    index: 1,
                    sort: Sort::Type,
                },
            ])],
        }),
    };
    let body = Expr {
        source: body.source,
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(nested.clone()),
    };
    let captured = capture_entry(target(), "nested".into(), body, &available, &mut 1000).unwrap();
    assert_eq!(captured.slots, vec![1]);
    let K::TypeValue(actual) = captured.function.body.kind else {
        panic!("type value")
    };
    let expected = nested
        .rebind_scope_partial(&[None, Some(slot(0, Sort::Type)), None])
        .unwrap();
    assert_eq!(actual, expected);
    let Term::Function { result, .. } = actual else {
        panic!("callback")
    };
    let Term::NormalForm {
        head, arguments, ..
    } = *result
    else {
        panic!("opaque head")
    };
    assert_eq!(head, target());
    assert_eq!(
        arguments,
        vec![Term::TypeListValue(vec![
            slot(0, Sort::Type),
            Term::Bound {
                depth: 1,
                index: 0,
                sort: Sort::Type
            }
        ])]
    );
    assert!(nested.rebind_scope_partial(&[None, None, None]).is_err());
}

#[test]
fn compact_entries_reject_missing_runtime_and_ill_scoped_inputs() {
    use nir::program::{Expr, ExprKind as K, LocalId, SourceRange};
    use nir::type_computations::capture_entry;
    let id = LocalId { owner: 7, index: 0 };
    let parameter = nir::Parameter {
        sort: Sort::Runtime,
        ty: Term::Primitive(Primitive::Bool),
    };
    let body = Expr {
        source: SourceRange { start: 0, end: 1 },
        ty: parameter.ty.clone(),
        kind: K::Local(id),
    };
    assert!(matches!(
        capture_entry(
            target(),
            "runtime".into(),
            body.clone(),
            &[(id, parameter.clone())],
            &mut 1000
        ),
        Err(Error::RuntimeInput(0))
    ));
    assert!(
        matches!(capture_entry(target(),"missing".into(),body.clone(),&[],&mut 1000),Err(Error::MissingCapture(found)) if found==id)
    );
    assert!(matches!(
        capture_entry(
            target(),
            "duplicate".into(),
            body.clone(),
            &[(id, parameter.clone()), (id, parameter)],
            &mut 1000
        ),
        Err(Error::DuplicateBinding(_))
    ));
    assert!(capture_entry(target(), "budget".into(), body.clone(), &[], &mut 0).is_err());
    let malformed = Expr {
        ty: slot(2, Sort::Type),
        ..body
    };
    assert!(matches!(
        capture_entry(
            target(),
            "scope".into(),
            malformed,
            &[(
                id,
                nir::Parameter {
                    sort: Sort::Type,
                    ty: Term::Primitive(Primitive::Type)
                }
            )],
            &mut 1000
        ),
        Err(Error::Scope(_))
    ));
}

#[test]
fn term_depth_limits_can_be_tightened_but_not_raised_past_the_ceiling() {
    use nir::type_computations::charge_term_with_depth;
    let boolean = Term::Primitive(Primitive::Bool);
    assert!(matches!(
        charge_term_with_depth(&boolean, &mut 1000, 0),
        Err(Error::Budget)
    ));
    assert!(charge_term_with_depth(&boolean, &mut 1000, 1).is_ok());
    let pointer = Term::Pointer(Box::new(boolean.clone()));
    assert!(matches!(
        charge_term_with_depth(&pointer, &mut 1000, 1),
        Err(Error::Budget)
    ));
    assert!(charge_term_with_depth(&pointer, &mut 1000, 2).is_ok());
    let mut deep = boolean;
    for _ in 0..128 {
        deep = Term::Pointer(Box::new(deep));
    }
    assert!(matches!(
        charge_term_with_depth(&deep, &mut 1000, usize::MAX),
        Err(Error::Budget)
    ));
}

#[test]
fn exact_float_capture_preserves_decimal_identity_and_checks_domain_and_budget() {
    use nir::program::{CallArgument, Expr, ExprKind as K, Literal, LocalId, SourceRange};
    let source = SourceRange { start: 1, end: 2 };
    let floating = Term::Primitive(Primitive::Float);
    let make = |spelling: &str, ty: Term| Expr {
        source,
        ty: Term::Primitive(Primitive::Type),
        kind: K::Call {
            callee: Box::new(Expr {
                source,
                ty: Term::Function {
                    parameters: vec![nir::Parameter {
                        sort: Sort::Runtime,
                        ty: ty.clone(),
                    }],
                    result: Box::new(Term::Primitive(Primitive::Type)),
                }
                .with_comptime_phase(true),
                kind: K::Function(target()),
            }),
            arguments: vec![CallArgument {
                parameter: 0,
                is_default: false,
                value: Expr {
                    source,
                    ty,
                    kind: K::Literal(Literal::Float(spelling.into())),
                },
            }],
            parameter_bindings: vec![LocalId { owner: 2, index: 0 }],
        },
    };
    let a = Invocation::from_call(
        &make("1.0000000000000000000000000000001", floating.clone()),
        &[],
        &mut 10000,
    )
    .unwrap();
    let equivalent = Invocation::from_call(
        &make("10.000000000000000000000000000001e-1", floating.clone()),
        &[],
        &mut 10000,
    )
    .unwrap();
    let b = Invocation::from_call(
        &make("1.0000000000000000000000000000002", floating.clone()),
        &[],
        &mut 10000,
    )
    .unwrap();
    assert_eq!(a.inputs()[0].value, equivalent.inputs()[0].value);
    assert_ne!(a.inputs()[0].value, b.inputs()[0].value);
    assert_eq!(a.inputs()[0].value.sort(), Sort::ComptimeValue);
    assert!(a.inputs()[0].value.check_scope(&[]).is_ok());
    assert!(a.to_string().contains("(comptime-float (decimal-value"));
    assert!(
        Invocation::from_call(
            &make("1.0", Term::Primitive(Primitive::F64)),
            &[],
            &mut 10000
        )
        .is_err()
    );
    assert!(Invocation::from_call(&make("NaN", floating.clone()), &[], &mut 10000).is_err());
    assert!(Invocation::from_call(&make("1.0", floating.clone()), &[], &mut 0).is_err());
    assert!(Invocation::from_call(&make("1.0", floating), &[], &mut 10000).is_ok());
    assert!(nir::type_computations::charge_term(&a.inputs()[0].value, &mut 2).is_err());
    assert_eq!(
        a.inputs()[0].value.instantiate_types(&[], &[]).unwrap(),
        a.inputs()[0].value
    );
}

#[test]
fn sparse_capture_keeps_unknown_runtime_positions_without_inventing_domains() {
    use nir::program::{Expr, ExprKind as K, LocalId, SourceRange};
    use nir::type_computations::capture_sparse_entry;
    let binding = |index| LocalId { owner: 12, index };
    let available = vec![
        (binding(0), Sort::Runtime, None),
        (
            binding(1),
            Sort::Type,
            Some(Term::Primitive(Primitive::Type)),
        ),
        (binding(2), Sort::Runtime, None),
        (binding(3), Sort::ComptimeValue, Some(slot(1, Sort::Type))),
    ];
    let body = Expr {
        source: SourceRange { start: 0, end: 1 },
        ty: slot(1, Sort::Type),
        kind: K::Local(binding(3)),
    };
    assert!(matches!(
        capture_sparse_entry(target(), "sparse".into(), body.clone(), &available, &mut 0),
        Err(Error::Budget)
    ));
    let captured = capture_sparse_entry(
        target(),
        "sparse".into(),
        body.clone(),
        &available,
        &mut 1000,
    )
    .unwrap();
    assert_eq!(captured.slots, vec![1, 3]);
    assert_eq!(
        captured.source_scope(),
        vec![
            Sort::Runtime,
            Sort::Type,
            Sort::Runtime,
            Sort::ComptimeValue
        ]
    );
    assert_eq!(captured.function.parameters, vec![binding(1), binding(3)]);
    assert_eq!(captured.function.body.ty, slot(0, Sort::Type));
    captured.function.signature.check_scope(&[]).unwrap();
    assert!(captured.to_string().starts_with("(captured-comptime-entry"));
    let runtime = Expr {
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Local(binding(2)),
        ..body.clone()
    };
    assert!(matches!(
        capture_sparse_entry(target(), "runtime".into(), runtime, &available, &mut 1000),
        Err(Error::RuntimeInput(2))
    ));
    let mut missing = available.clone();
    missing[3].2 = None;
    assert!(
        capture_sparse_entry(
            target(),
            "missing".into(),
            body.clone(),
            &missing,
            &mut 1000
        )
        .is_err()
    );
    // A pending staged domain can retain its position, but may not enter the
    // executable capture until its domain is known.
    let independent = Expr {
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Literal(nir::program::Literal::Bool(true)),
        ..body
    };
    let captured = capture_sparse_entry(
        target(),
        "unused-staged".into(),
        independent,
        &missing,
        &mut 1000,
    )
    .unwrap();
    assert!(captured.slots.is_empty());
    assert_eq!(captured.source_scope().len(), missing.len());
}

#[test]
fn bounded_parameter_substitution_checks_expansion_before_allocating() {
    use nir::type_computations::instantiate_parameters_bounded as instantiate;
    let parameter = nir::DeclarationParameter {
        declaration: target(),
        sort: Sort::Type,
    };
    let rigid = Term::Rigid {
        declaration: target(),
        sort: Sort::Type,
    };
    let input = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 100]);
    let root = Term::NormalForm {
        head: DeclarationId {
            package: 2,
            local: 0,
        },
        sort: Sort::Type,
        arguments: vec![rigid; 20],
    };
    let args = [input.clone()];
    assert!(matches!(
        instantiate(&root, &[parameter], &args, &[], &mut 500),
        Err(Error::Budget)
    ));
    let result = instantiate(&root, &[parameter], &args, &[], &mut 10_000).unwrap();
    let Term::NormalForm {
        head, arguments, ..
    } = result
    else {
        panic!("opaque head")
    };
    assert_eq!(
        head,
        DeclarationId {
            package: 2,
            local: 0
        }
    );
    assert_eq!(arguments, vec![input; 20]);
    assert!(matches!(
        instantiate(&root, &[parameter], &[Term::Bool(true)], &[], &mut 10_000),
        Err(Error::Scope(_))
    ));
}

#[test]
fn bounded_parameter_substitution_is_simultaneous_and_checks_result_depth() {
    use nir::type_computations::instantiate_parameters_bounded as instantiate;
    let first = nir::DeclarationParameter {
        declaration: target(),
        sort: Sort::Type,
    };
    let second = nir::DeclarationParameter {
        declaration: DeclarationId {
            package: 1,
            local: 3,
        },
        sort: Sort::Type,
    };
    let rigid = |binder: nir::DeclarationParameter| Term::Rigid {
        declaration: binder.declaration,
        sort: binder.sort,
    };
    let root = Term::Tuple(vec![rigid(first), rigid(second)]);
    assert_eq!(
        instantiate(
            &root,
            &[first, second],
            &[rigid(second), Term::Primitive(Primitive::Bool)],
            &[],
            &mut 1000
        )
        .unwrap(),
        Term::Tuple(vec![rigid(second), Term::Primitive(Primitive::Bool)])
    );
    let mut outer = rigid(first);
    for _ in 0..80 {
        outer = Term::Pointer(Box::new(outer));
    }
    let mut replacement = Term::Primitive(Primitive::Bool);
    for _ in 0..80 {
        replacement = Term::Pointer(Box::new(replacement));
    }
    assert!(matches!(
        instantiate(&outer, &[first], &[replacement], &[], &mut 10_000),
        Err(Error::Budget)
    ));
    assert!(
        instantiate(
            &outer,
            &[first],
            &[Term::Primitive(Primitive::Bool)],
            &[],
            &mut 10_000
        )
        .is_ok()
    );
}

#[test]
fn rigid_dependency_inspection_is_bounded_and_keeps_opaque_heads() {
    use nir::type_computations::has_rigid_parameters_bounded;
    let open = Term::NormalForm {
        head: target(),
        sort: Sort::Type,
        arguments: vec![Term::Tuple(vec![Term::Rigid {
            declaration: DeclarationId {
                package: 1,
                local: 3,
            },
            sort: Sort::Type,
        }])],
    };
    assert!(has_rigid_parameters_bounded(&open, &mut 32).unwrap());
    assert!(matches!(
        has_rigid_parameters_bounded(&open, &mut 1),
        Err(Error::Budget)
    ));
    let closed = Term::NormalForm {
        head: target(),
        sort: Sort::Type,
        arguments: vec![Term::Primitive(Primitive::U8)],
    };
    assert!(!has_rigid_parameters_bounded(&closed, &mut 32).unwrap());
    let mut deep = open.clone();
    for _ in 0..129 {
        deep = Term::Tuple(vec![deep]);
    }
    assert!(matches!(
        has_rigid_parameters_bounded(&deep, &mut 10_000),
        Err(Error::Budget)
    ));
    assert!(has_rigid_parameters_bounded(&open, &mut 32).unwrap());
}

#[test]
fn signature_fragment_copy_budget_does_not_certify_its_enclosing_scope() {
    use nir::type_computations::{charge_parameter_substitution, instantiate_parameters_bounded};
    let parameter = nir::DeclarationParameter {
        declaration: target(),
        sort: Sort::Type,
    };
    let outer = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let mut fields = vec![
        Term::Rigid {
            declaration: target(),
            sort: Sort::Type
        };
        20
    ];
    fields.push(outer.clone());
    let fragment = Term::Tuple(fields);
    let arguments = [Term::Tuple(vec![Term::Primitive(Primitive::Bool); 100])];
    assert!(matches!(
        charge_parameter_substitution(&fragment, &[parameter], &arguments, &mut 500),
        Err(Error::Budget)
    ));
    charge_parameter_substitution(&fragment, &[parameter], &arguments, &mut 10000).unwrap();
    // Allocation admission preserves the open fragment; only its owner can
    // supply the binder frame, and the closed API must continue to reject it.
    assert!(
        instantiate_parameters_bounded(&fragment, &[parameter], &arguments, &[], &mut 10000)
            .is_err()
    );
    let result = instantiate_parameters_bounded(
        &fragment,
        &[parameter],
        &arguments,
        &[vec![Sort::Type]],
        &mut 10000,
    )
    .unwrap();
    let Term::Tuple(fields) = result else {
        panic!("tuple")
    };
    assert_eq!(fields.len(), 21);
    assert_eq!(fields.last(), Some(&outer));
    assert!(fields[..20].iter().all(|field| field == &arguments[0]));
}
