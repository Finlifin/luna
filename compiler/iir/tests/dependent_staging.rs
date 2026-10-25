use nir::program::{
    Expr, ExprKind as K, Function, FunctionPhase, LocalId, SourceRange, TraitRequirement,
    TypePredicate,
};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term};
fn p(value: Primitive) -> Term {
    Term::Primitive(value)
}
fn id(index: u32) -> LocalId {
    LocalId { owner: 1, index }
}
fn expr(kind: K, ty: Term) -> Expr {
    Expr {
        source: SourceRange { start: 0, end: 1 },
        kind,
        ty,
    }
}

#[test]
fn dependent_inputs_rebind_body_contracts_defaults_and_trait_arguments() {
    let t = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let observation = expr(
        K::TypeEqual(
            Box::new(expr(K::TypeValue(t.clone()), p(Primitive::Type))),
            Box::new(expr(K::TypeValue(t.clone()), p(Primitive::Type))),
        ),
        p(Primitive::Bool),
    );
    let capability = TraitRequirement {
        interface: DeclarationId {
            package: 1,
            local: 40,
        },
        interface_arguments: vec![t.clone()],
        subject: t.clone(),
    };
    let template = Function {
        declaration: DeclarationId {
            package: 1,
            local: 1,
        },
        name: "identity".into(),
        phase: FunctionPhase::Comptime,
        inline: false,
        pure: false,
        is_unsafe: false,
        signature: Term::Function {
            parameters: vec![
                Parameter {
                    sort: Sort::Type,
                    ty: p(Primitive::Type),
                },
                Parameter {
                    sort: Sort::Runtime,
                    ty: t.clone(),
                },
                Parameter {
                    sort: Sort::Runtime,
                    ty: t.clone(),
                },
            ],
            result: Box::new(t.clone()),
        },
        parameters: vec![id(0), id(1), id(2)],
        body: expr(K::Local(id(2)), t.clone()),
        defaults: vec![(2, expr(K::Local(id(1)), t.clone()))],
        preconditions: vec![observation.clone()],
        postconditions: vec![observation],
        requirements: vec![TypePredicate::And(vec![
            TypePredicate::Equal(t.clone(), t),
            TypePredicate::Trait(capability.clone()),
        ])],
        capabilities: vec![capability],
        result_binding: None,
        entry_snapshots: vec![],
        type_parameters: vec![],
    };
    let original = template.clone();
    let inputs = vec![
        Some(expr(K::TypeValue(p(Primitive::Bool)), p(Primitive::Type))),
        None,
        None,
    ];
    assert!(iir::staging::bind_inputs(&template, &inputs, &mut 0).is_err());
    let bound = iir::staging::bind_inputs(&template, &inputs, &mut 10000).unwrap();
    assert_eq!(template, original);
    assert_eq!(bound.body.ty, p(Primitive::Bool));
    assert_eq!(bound.parameters, vec![id(1), id(2)]);
    assert_eq!(bound.defaults[0].0, 1);
    assert_eq!(bound.defaults[0].1.ty, p(Primitive::Bool));
    assert_eq!(bound.capabilities[0].subject, p(Primitive::Bool));
    assert_eq!(
        bound.capabilities[0].interface_arguments,
        vec![p(Primitive::Bool)]
    );
    assert_eq!(
        bound.requirements[0],
        TypePredicate::And(vec![
            TypePredicate::Equal(p(Primitive::Bool), p(Primitive::Bool)),
            TypePredicate::Trait(bound.capabilities[0].clone())
        ])
    );
    for condition in bound.preconditions.iter().chain(&bound.postconditions) {
        let K::TypeEqual(left, right) = &condition.kind else {
            panic!()
        };
        assert_eq!(left.kind, K::TypeValue(p(Primitive::Bool)));
        assert_eq!(right.kind, left.kind);
    }
}

#[test]
fn dependent_default_values_receive_concrete_types_before_evaluation_or_runtime_delivery() {
    for staged in [false, true] {
        let t = Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        };
        let template = Function {
            declaration: DeclarationId {
                package: 1,
                local: 2,
            },
            name: "copy".into(),
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![
                    Parameter {
                        sort: Sort::Type,
                        ty: p(Primitive::Type),
                    },
                    Parameter {
                        sort: if staged {
                            Sort::ComptimeValue
                        } else {
                            Sort::Runtime
                        },
                        ty: t.clone(),
                    },
                    Parameter {
                        sort: if staged {
                            Sort::ComptimeValue
                        } else {
                            Sort::Runtime
                        },
                        ty: t.clone(),
                    },
                ],
                result: Box::new(t.clone()),
            },
            parameters: vec![id(0), id(1), id(2)],
            body: expr(K::Local(id(2)), t.clone()),
            defaults: vec![(2, expr(K::Local(id(1)), t.clone()))],
            preconditions: vec![],
            postconditions: vec![],
            requirements: vec![],
            capabilities: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            type_parameters: vec![],
        };
        let values = [
            expr(K::TypeValue(p(Primitive::Bool)), p(Primitive::Type)),
            expr(
                K::Literal(nir::program::Literal::Bool(true)),
                p(Primitive::Bool),
            ),
            expr(K::Local(id(1)), t),
        ];
        let call = expr(
            K::Call {
                callee: Box::new(expr(
                    K::Function(template.declaration),
                    template.signature.clone(),
                )),
                arguments: values
                    .into_iter()
                    .enumerate()
                    .map(|(parameter, value)| nir::program::CallArgument {
                        parameter,
                        value,
                        is_default: parameter == 2,
                    })
                    .collect(),
                parameter_bindings: template.parameters.clone(),
            },
            p(Primitive::Bool),
        );
        for mismatch in 0..3 {
            let mut wrong = call.clone();
            let K::Call { callee, .. } = &mut wrong.kind else {
                panic!()
            };
            if mismatch == 0 {
                callee.ty = callee.ty.clone().with_comptime_phase(true);
            } else {
                let Term::Function { parameters, result } = &mut callee.ty else {
                    panic!()
                };
                if mismatch == 1 {
                    *result = Box::new(p(Primitive::Char));
                } else {
                    parameters[0].ty = p(Primitive::Char);
                }
            }
            assert!(
                iir::staging::bind_call(&template, &[], &wrong, &mut 10000).is_err(),
                "accepted incompatible entry signature {mismatch}"
            );
        }
        if staged {
            let mut wrong = call.clone();
            let K::Call { arguments, .. } = &mut wrong.kind else {
                panic!()
            };
            arguments[1].value = expr(
                K::Literal(nir::program::Literal::Char('x')),
                p(Primitive::Char),
            );
            // The dependent default expects Bool after T is supplied. A Char
            // input must not be substituted into it, even though both values
            // are canonical and available at compile time.
            assert!(
                iir::staging::bind_call(&template, &[], &wrong, &mut 10000)
                    .unwrap_err()
                    .contains("type mismatch")
            );
            assert!(
                iir::staging::bind_call_with(&template, &[], &call, &mut 10000, |_, _| Err(
                    "input evaluation failed".into()
                ))
                .unwrap_err()
                .contains("input evaluation failed")
            );
        }
        let (instance, call) = iir::staging::bind_call(&template, &[], &call, &mut 10000).unwrap();
        assert_eq!(instance.function.body.ty, p(Primitive::Bool));
        let K::Call { arguments, .. } = call.kind else {
            panic!()
        };
        assert_eq!(arguments.len(), if staged { 0 } else { 2 });
        assert!(
            arguments
                .iter()
                .all(|arg| arg.value.ty == p(Primitive::Bool))
        );
    }
}

#[test]
fn dependent_defaults_do_not_recharge_unrelated_completed_payloads() {
    let t = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let mut parameters = vec![
        Parameter {
            sort: Sort::Type,
            ty: p(Primitive::Type),
        },
        Parameter {
            sort: Sort::ComptimeValue,
            ty: p(Primitive::Str),
        },
    ];
    parameters.extend((0..32).map(|_| Parameter {
        sort: Sort::ComptimeValue,
        ty: t.clone(),
    }));
    let bindings: Vec<_> = (0..parameters.len())
        .map(|index| id(index as u32))
        .collect();
    let defaults: Vec<_> = (2..parameters.len())
        .map(|index| {
            (
                index,
                expr(K::Literal(nir::program::Literal::Bool(true)), t.clone()),
            )
        })
        .collect();
    let template = Function {
        declaration: DeclarationId {
            package: 1,
            local: 3,
        },
        name: "defaults".into(),
        phase: FunctionPhase::Runtime,
        inline: false,
        pure: false,
        is_unsafe: false,
        signature: Term::Function {
            parameters,
            result: Box::new(t.clone()),
        },
        parameters: bindings.clone(),
        body: expr(K::Local(*bindings.last().unwrap()), t),
        defaults: defaults.clone(),
        preconditions: vec![],
        postconditions: vec![],
        requirements: vec![],
        capabilities: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        type_parameters: vec![],
    };
    let payload = "x".repeat(8192);
    let mut arguments = vec![
        nir::program::CallArgument {
            parameter: 0,
            value: expr(K::TypeValue(p(Primitive::Bool)), p(Primitive::Type)),
            is_default: false,
        },
        nir::program::CallArgument {
            parameter: 1,
            value: expr(
                K::Literal(nir::program::Literal::String(payload)),
                p(Primitive::Str),
            ),
            is_default: false,
        },
    ];
    arguments.extend(
        defaults
            .into_iter()
            .map(|(parameter, value)| nir::program::CallArgument {
                parameter,
                value,
                is_default: true,
            }),
    );
    let call = expr(
        K::Call {
            callee: Box::new(expr(
                K::Function(template.declaration),
                template.signature.clone(),
            )),
            arguments,
            parameter_bindings: bindings,
        },
        p(Primitive::Bool),
    );
    let original = template.clone();
    let mut evaluations = 0;
    let (instance, call) =
        iir::staging::bind_call_with(&template, &[], &call, &mut 100_000, |input, _| {
            if evaluations >= 2 {
                assert_eq!(input.ty, p(Primitive::Bool));
            }
            evaluations += 1;
            Ok(input.clone())
        })
        .unwrap();
    assert_eq!(evaluations, 34, "each input must be evaluated exactly once");
    assert_eq!(instance.key.arguments.len(), 34);
    assert_eq!(instance.function.body.ty, p(Primitive::Bool));
    let K::Call { arguments, .. } = call.kind else {
        panic!()
    };
    assert!(arguments.is_empty());
    assert_eq!(template, original);
}

#[test]
fn dependent_type_value_domains_rebind_body_and_contracts_compositionally() {
    let head = DeclarationId {
        package: 1,
        local: 90,
    };
    let opaque = Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![Term::Bound {
            depth: 0,
            index: 1,
            sort: Sort::ComptimeValue,
        }],
    };
    let requirement = TraitRequirement {
        interface: DeclarationId {
            package: 1,
            local: 91,
        },
        interface_arguments: vec![],
        subject: opaque.clone(),
    };
    let template = Function {
        declaration: DeclarationId {
            package: 1,
            local: 4,
        },
        name: "Reify".into(),
        phase: FunctionPhase::Comptime,
        inline: false,
        pure: false,
        is_unsafe: false,
        signature: Term::Function {
            parameters: vec![
                Parameter {
                    sort: Sort::Type,
                    ty: p(Primitive::Type),
                },
                Parameter {
                    sort: Sort::ComptimeValue,
                    ty: Term::Bound {
                        depth: 0,
                        index: 0,
                        sort: Sort::Type,
                    },
                },
            ],
            result: Box::new(p(Primitive::Type)),
        },
        parameters: vec![id(0), id(1)],
        body: expr(K::TypeValue(opaque.clone()), p(Primitive::Type)),
        defaults: vec![],
        preconditions: vec![expr(
            K::TypeEqual(
                Box::new(expr(K::TypeValue(opaque.clone()), p(Primitive::Type))),
                Box::new(expr(K::TypeValue(opaque.clone()), p(Primitive::Type))),
            ),
            p(Primitive::Bool),
        )],
        postconditions: vec![],
        requirements: vec![TypePredicate::Equal(opaque.clone(), opaque)],
        capabilities: vec![requirement],
        result_binding: None,
        entry_snapshots: vec![],
        type_parameters: vec![],
    };
    let input_type = expr(K::TypeValue(p(Primitive::Type)), p(Primitive::Type));
    let input_value = expr(K::TypeValue(p(Primitive::Bool)), p(Primitive::Type));
    let partial =
        iir::staging::bind_inputs(&template, &[Some(input_type.clone()), None], &mut 10000)
            .unwrap();
    let split =
        iir::staging::bind_inputs(&partial, &[Some(input_value.clone())], &mut 10000).unwrap();
    let full = iir::staging::bind_inputs(
        &template,
        &[Some(input_type.clone()), Some(input_value)],
        &mut 10000,
    )
    .unwrap();
    assert_eq!(split, full);
    let expected = Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![p(Primitive::Bool)],
    };
    assert_eq!(full.body.kind, K::TypeValue(expected.clone()));
    assert_eq!(full.capabilities[0].subject, expected);
    assert_eq!(
        full.requirements,
        vec![TypePredicate::Equal(expected.clone(), expected.clone())]
    );
    let K::TypeEqual(left, right) = &full.preconditions[0].kind else {
        panic!()
    };
    assert_eq!(left.kind, K::TypeValue(expected));
    assert_eq!(left, right);
    assert!(
        iir::staging::bind_inputs(
            &template,
            &[
                Some(input_type),
                Some(expr(
                    K::Literal(nir::program::Literal::Bool(true)),
                    p(Primitive::Bool)
                ))
            ],
            &mut 10000
        )
        .is_err()
    );
}
