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
        is_quote: false,
        pure: true,
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
        },
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
