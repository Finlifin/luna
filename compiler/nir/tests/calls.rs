use nir::*;

fn p(primitive: Primitive) -> Term {
    Term::Primitive(primitive)
}
fn b(depth: u32, index: usize, sort: Sort) -> Term {
    Term::Bound { depth, index, sort }
}
fn identity_signature() -> Term {
    Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Type,
                ty: p(Primitive::Type),
            },
            Parameter {
                sort: Sort::Runtime,
                ty: b(0, 0, Sort::Type),
            },
        ],
        result: Box::new(b(0, 0, Sort::Type)),
    }
}

#[test]
fn dependent_contract_and_result_use_the_same_type_argument() {
    let signature = identity_signature();
    let value = Term::Runtime(RuntimeId {
        instance: 10,
        local: 0,
    });
    let mut checks = Vec::new();
    let result = signature
        .instantiate_function(
            &[p(Primitive::U32), value.clone()],
            |index, actual, expected| {
                checks.push((index, actual.clone(), expected.clone()));
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    assert_eq!(
        checks,
        vec![
            (0, p(Primitive::U32), p(Primitive::Type)),
            (1, value, p(Primitive::U32))
        ]
    );
    assert_eq!(result, p(Primitive::U32));
    assert_eq!(signature, identity_signature());
}

#[test]
fn return_normal_form_preserves_head_and_runtime_identity() {
    let head = DeclarationId {
        package: 1,
        local: 1,
    };
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: p(Primitive::U32),
        }],
        result: Box::new(Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![b(0, 0, Sort::Runtime)],
        }),
    };
    for instance in [7, 8] {
        let argument = Term::Runtime(RuntimeId { instance, local: 0 });
        let result = signature
            .instantiate_function(&[argument.clone()], |_, _, _| Ok::<_, ()>(()))
            .unwrap();
        assert_eq!(
            result,
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments: vec![argument]
            }
        );
    }
}

#[test]
fn nested_result_does_not_capture_the_callers_type_binder() {
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: p(Primitive::Type),
        }],
        result: Box::new(Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: b(1, 0, Sort::Type),
            }],
            result: Box::new(b(1, 0, Sort::Type)),
        }),
    };
    let result = signature
        .instantiate_function(&[b(0, 2, Sort::Type)], |_, _, _| Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(
        result,
        Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: b(1, 2, Sort::Type)
            }],
            result: Box::new(b(1, 2, Sort::Type)),
        }
    );
}

#[test]
fn shape_errors_do_not_invoke_semantic_checking() {
    let signature = identity_signature();
    let forbidden = |_: usize, _: &Term, _: &Term| -> Result<(), ()> {
        panic!("invalid shape reached semantic checking")
    };
    assert!(matches!(
        signature.instantiate_function(&[], forbidden),
        Err(CallError::Shape(TypeError::ArgumentCount { .. }))
    ));
    assert!(matches!(
        signature.instantiate_function(&[p(Primitive::U32), Term::Bool(true)], forbidden),
        Err(CallError::Shape(TypeError::WrongSort { .. }))
    ));
    assert_eq!(
        p(Primitive::Bool).instantiate_function(&[], forbidden),
        Err(CallError::NotFunction)
    );
    let forward = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: b(0, 0, Sort::Type),
        }],
        result: Box::new(p(Primitive::Bool)),
    };
    assert!(matches!(
        forward.instantiate_function(&[p(Primitive::U32)], forbidden),
        Err(CallError::Shape(TypeError::InvalidBound { .. }))
    ));
}

#[test]
fn failed_contract_reports_its_parameter_and_does_not_publish_a_result() {
    let signature = identity_signature();
    let arguments = [
        p(Primitive::U32),
        Term::Runtime(RuntimeId {
            instance: 1,
            local: 0,
        }),
    ];
    let mut checks = Vec::new();
    let error = signature
        .instantiate_function(&arguments, |index, _, _| {
            checks.push(index);
            Err("type mismatch")
        })
        .unwrap_err();
    assert_eq!(checks, vec![0]);
    assert_eq!(
        error,
        CallError::Contract {
            parameter: 0,
            error: "type mismatch"
        }
    );
    assert_eq!(error.to_string(), "parameter 0: type mismatch");
    assert_eq!(
        signature
            .instantiate_function(&arguments, |_, _, _| Ok::<_, ()>(()))
            .unwrap(),
        p(Primitive::U32)
    );
}

#[test]
fn method_contract_summary_preserves_and_instantiates_local_binders() {
    use nir::program::{MethodContract, TraitRequirement};
    let id = |local| DeclarationId { package: 1, local };
    let rigid = Term::Rigid {
        declaration: id(1),
        sort: Sort::Type,
    };
    let contract = MethodContract {
        phase: Default::default(),
        is_quote: false,
        pure: true,
        is_unsafe: false,
        type_arguments: vec![],
        type_parameters: vec![id(1)],
        capabilities: vec![TraitRequirement {
            subject: rigid.clone(),
            interface: id(2),
            interface_arguments: vec![],
        }],
        program: Default::default(),
        parameter_bindings: vec![],
        by_value_receiver: false,
        by_reference_receiver: false,
        declaration: id(3),
        name: "generic\"method".into(),
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(rigid),
        }
        .into(),
        requirements: vec![],
        default: None,
    };
    let open = contract.program_summary(&[], &[]).unwrap();
    assert!(open.pure);
    assert_eq!(open.type_parameters, vec![id(1)]);
    assert_eq!(open.capabilities, contract.capabilities);
    let closed = contract
        .program_summary(&[id(1)], &[p(Primitive::Bool)])
        .unwrap();
    assert!(closed.pure);
    assert!(closed.type_parameters.is_empty());
    assert_eq!(closed.capabilities[0].subject, p(Primitive::Bool));
    assert_eq!(
        closed.signature,
        Term::Function {
            parameters: vec![],
            result: Box::new(p(Primitive::Bool))
        }
    );
    let mut indexed = contract.clone();
    indexed.capabilities[0]
        .interface_arguments
        .push(Term::Rigid {
            declaration: id(4),
            sort: Sort::ComptimeValue,
        });
    let binders = [
        nir::DeclarationParameter {
            declaration: id(1),
            sort: Sort::Type,
        },
        nir::DeclarationParameter {
            declaration: id(4),
            sort: Sort::ComptimeValue,
        },
    ];
    let arguments = [p(Primitive::Bool), Term::Symbol("px".into())];
    let summary = indexed
        .program_summary_parameters(&binders, &arguments)
        .unwrap();
    assert_eq!(
        summary.capabilities[0].interface_arguments,
        vec![Term::Symbol("px".into())]
    );
    assert!(summary.type_parameters.is_empty());
    assert!(
        indexed
            .program_summary(&[id(1), id(4)], &arguments)
            .is_err()
    );
    // The caller's Type input and the method's own input occupy distinct frames.
    let mut dependent = indexed.clone();
    dependent.signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: p(Primitive::Type),
        }],
        result: Box::new(p(Primitive::Bool)),
    }
    .into();
    let domains = [
        p(Primitive::Type),
        Term::Rigid {
            declaration: id(1),
            sort: Sort::Type,
        },
    ];
    let inputs = [p(Primitive::Type), b(0, 0, Sort::Type)];
    let caller_frames = [vec![Sort::Type]];
    let type_value = |input| nir::program::Expr {
        source: nir::program::SourceRange { start: 3, end: 7 },
        ty: p(Primitive::Type),
        kind: nir::program::ExprKind::TypeValue(Term::Nominal {
            declaration: id(9),
            arguments: vec![input],
        }),
    };
    dependent.program.preconditions.push(nir::program::Expr {
        source: nir::program::SourceRange { start: 3, end: 7 },
        ty: p(Primitive::Bool),
        kind: nir::program::ExprKind::TypeEqual(
            Box::new(type_value(Term::Rigid {
                declaration: id(4),
                sort: Sort::ComptimeValue,
            })),
            Box::new(type_value(b(0, 0, Sort::Type))),
        ),
    });
    let summary = dependent
        .program_summary_declaration_inputs(
            &[id(1), id(4)],
            &domains,
            &inputs,
            &caller_frames,
            &mut 1000,
        )
        .unwrap();
    assert_eq!(summary.capabilities[0].subject, p(Primitive::Type));
    assert_eq!(
        summary.capabilities[0].interface_arguments,
        vec![b(1, 0, Sort::Type)]
    );
    assert!(summary.type_parameters.is_empty());
    assert_eq!(
        summary.preconditions[0].kind,
        nir::program::ExprKind::TypeEqual(
            Box::new(type_value(b(1, 0, Sort::Type))),
            Box::new(type_value(b(0, 0, Sort::Type))),
        )
    );
    assert!(
        dependent
            .program_summary_declaration_inputs(&[id(1), id(4)], &domains, &inputs, &[], &mut 1000,)
            .is_err()
    );
    assert!(
        dependent
            .program_summary_declaration_inputs(
                &[id(1), id(4)],
                &domains,
                &inputs,
                &caller_frames,
                &mut 0,
            )
            .is_err()
    );
    assert_eq!(
        dependent
            .program_summary_declaration_inputs(
                &[id(1), id(4)],
                &domains,
                &inputs,
                &caller_frames,
                &mut 1000,
            )
            .unwrap(),
        summary
    );
    let dump = contract.to_string();
    assert!(dump.contains("(pure true)"));
    assert!(dump.contains("(type-parameters "));
    assert!(dump.contains("(capabilities "));
    assert!(dump.contains("generic\\\"method"));
}

#[test]
fn dependent_comptime_value_domains_specialize_to_type_only_with_type_evidence() {
    let head = DeclarationId {
        package: 7,
        local: 9,
    };
    let signature = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Type,
                ty: p(Primitive::Type),
            },
            Parameter {
                sort: Sort::ComptimeValue,
                ty: b(0, 0, Sort::Type),
            },
        ],
        result: Box::new(Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![b(0, 0, Sort::Type), b(0, 1, Sort::ComptimeValue)],
        }),
    };
    let inputs = [Some(p(Primitive::Type)), Some(p(Primitive::Bool))];
    let partial = signature
        .prepare_function_inputs(&[inputs[0].clone(), None])
        .unwrap();
    let Term::Function { parameters, .. } = partial.signature() else {
        panic!()
    };
    assert_eq!(
        parameters,
        &vec![Parameter {
            sort: Sort::Type,
            ty: p(Primitive::Type)
        }]
    );
    assert_eq!(
        partial.rebind(&b(0, 1, Sort::ComptimeValue)).unwrap(),
        b(0, 0, Sort::Type)
    );
    assert!(partial.to_string().contains("(type-slots 1)"));
    let full = signature.prepare_function_inputs(&inputs).unwrap();
    assert_eq!(
        partial
            .signature()
            .bind_function_inputs(&[inputs[1].clone()])
            .unwrap(),
        *full.signature()
    );
    let nested = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: p(Primitive::Type),
        }],
        result: Box::new(Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![b(0, 0, Sort::Type), b(1, 1, Sort::ComptimeValue)],
        }),
    };
    assert_eq!(
        full.rebind(&nested).unwrap(),
        Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Type,
                ty: p(Primitive::Type)
            }],
            result: Box::new(Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments: vec![b(0, 0, Sort::Type), p(Primitive::Bool)]
            })
        }
    );
    assert!(
        signature
            .bind_function_inputs(&[Some(p(Primitive::Bool)), Some(p(Primitive::Bool))])
            .is_err()
    );
    assert!(
        signature
            .bind_function_inputs(&[Some(p(Primitive::Type)), Some(Term::Bool(true))])
            .is_err()
    );
    assert!(
        b(0, 1, Sort::ComptimeValue)
            .substitute(&[p(Primitive::Type), p(Primitive::Bool)])
            .is_err()
    );
    let mut checks = Vec::new();
    let result = signature
        .instantiate_function(
            &[p(Primitive::Type), p(Primitive::Bool)],
            |index, _, expected| {
                checks.push((index, expected.clone()));
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    assert_eq!(
        checks,
        vec![(0, p(Primitive::Type)), (1, p(Primitive::Type))]
    );
    assert_eq!(
        result,
        Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![p(Primitive::Type), p(Primitive::Bool)]
        }
    );
    let mut called = false;
    assert!(
        signature
            .instantiate_function(&[p(Primitive::Type), Term::Bool(true)], |_, _, _| {
                called = true;
                Ok::<_, ()>(())
            })
            .is_err()
    );
    assert!(
        !called,
        "invalid input domains must fail before contract callbacks"
    );
    let opaque_parameter = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::ComptimeValue,
            ty: Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments: vec![p(Primitive::Type)],
            },
        }],
        result: Box::new(p(Primitive::Void)),
    };
    assert!(
        opaque_parameter
            .bind_function_inputs(&[Some(p(Primitive::Bool))])
            .is_err(),
        "an opaque head is not evidence that a value parameter has type Type"
    );
    let text_signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::ComptimeValue,
            ty: p(Primitive::Str),
        }],
        result: Box::new(p(Primitive::Void)),
    };
    let display = text_signature
        .prepare_function_inputs(&[Some(Term::String("a\"b\nc".into()))])
        .unwrap()
        .to_string();
    assert!(display.starts_with("(function-input-binding (signature (function"));
    assert!(display.contains("(string \"a\\\"b\\nc\")"), "{display}");
}

#[test]
fn program_contract_substitution_preserves_scope_for_type_valued_indices() {
    use nir::contracts::ProgramContract;
    use nir::program::{Expr, ExprKind, SourceRange};
    let ids = [
        DeclarationId {
            package: 91,
            local: 1,
        },
        DeclarationId {
            package: 91,
            local: 2,
        },
    ];
    let domains = [
        p(Primitive::Type),
        Term::Rigid {
            declaration: ids[0],
            sort: Sort::Type,
        },
    ];
    let arguments = [p(Primitive::Type), p(Primitive::Bool)];
    let make_type = |index| Term::Nominal {
        declaration: DeclarationId {
            package: 91,
            local: 3,
        },
        arguments: vec![index, b(0, 0, Sort::Type)],
    };
    let source = SourceRange { start: 1, end: 2 };
    let value = Expr {
        source,
        ty: p(Primitive::Type),
        kind: ExprKind::TypeValue(make_type(Term::Rigid {
            declaration: ids[1],
            sort: Sort::ComptimeValue,
        })),
    };
    let predicate = Expr {
        source,
        ty: p(Primitive::Bool),
        kind: ExprKind::TypeEqual(Box::new(value.clone()), Box::new(value)),
    };
    let contract = ProgramContract {
        preconditions: vec![predicate.clone()],
        postconditions: vec![predicate],
        ..Default::default()
    };
    let frames = [vec![Sort::Type]];
    let result = contract
        .instantiate_declaration_parameters(&ids, &domains, &arguments, &frames, &mut 1000)
        .unwrap();
    for predicate in result.preconditions.iter().chain(&result.postconditions) {
        let ExprKind::TypeEqual(left, right) = &predicate.kind else {
            panic!("lost predicate");
        };
        assert_eq!(
            left.kind,
            ExprKind::TypeValue(make_type(p(Primitive::Bool)))
        );
        assert_eq!(left, right);
        assert_eq!(predicate.source, source);
    }
    assert!(
        contract
            .instantiate_declaration_parameters(&ids, &domains, &arguments, &[], &mut 1000)
            .is_err()
    );
    assert!(
        contract
            .instantiate_declaration_parameters(
                &ids,
                &domains,
                &[p(Primitive::Type), Term::Bool(true)],
                &frames,
                &mut 1000
            )
            .is_err()
    );
    assert!(
        contract
            .instantiate_declaration_parameters(&ids, &domains, &arguments, &frames, &mut 0)
            .is_err()
    );
    assert_eq!(
        contract
            .instantiate_declaration_parameters(&ids, &domains, &arguments, &frames, &mut 1000)
            .unwrap(),
        result
    );
}

#[test]
fn dependent_value_expressions_become_type_values_after_domain_completion() {
    use nir::program::{Expr, ExprKind, SourceRange};
    let ids = [
        DeclarationId {
            package: 1,
            local: 91,
        },
        DeclarationId {
            package: 1,
            local: 92,
        },
    ];
    let domain = Term::Rigid {
        declaration: ids[0],
        sort: Sort::Type,
    };
    let template = Expr {
        ty: domain.clone(),
        source: SourceRange { start: 0, end: 0 },
        kind: ExprKind::ComptimeValue(Term::Rigid {
            declaration: ids[1],
            sort: Sort::ComptimeValue,
        }),
    };
    let domains = [p(Primitive::Type), domain];
    let typed = template
        .instantiate_declaration_parameters_in_scope(
            &ids,
            &domains,
            &[p(Primitive::Type), p(Primitive::Bool)],
            &[],
        )
        .unwrap();
    assert_eq!(typed.ty, p(Primitive::Type));
    assert!(matches!(typed.kind, ExprKind::TypeValue(ty) if ty == p(Primitive::Bool)));
    assert!(
        template
            .instantiate_declaration_parameters_in_scope(
                &ids,
                &domains,
                &[p(Primitive::Bool), p(Primitive::Bool)],
                &[]
            )
            .is_err()
    );
    assert!(matches!(template.kind, ExprKind::ComptimeValue(_)));
}
