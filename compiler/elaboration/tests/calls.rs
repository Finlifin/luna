use elaboration::{
    calls::{Argument, Scheme},
    ptc::Budget,
};
use nir::{DeclarationId, Parameter, Primitive as P, RuntimeId, Sort, Term};
fn generic(index: usize) -> Term {
    Term::Bound {
        depth: 1,
        index,
        sort: Sort::Type,
    }
}
fn scheme(parameters: Vec<Term>, result: Term, types: usize) -> Scheme {
    Scheme {
        parameter_sorts: vec![],
        type_parameters: types,
        signature: Term::Function {
            parameters: parameters
                .into_iter()
                .map(|ty| Parameter {
                    sort: Sort::Runtime,
                    ty,
                })
                .collect(),
            result: Box::new(result),
        },
    }
}
fn argument(ty: Term, index: u32) -> Argument {
    Argument {
        ty,
        value: Term::Runtime(RuntimeId {
            instance: 42,
            local: index,
        }),
    }
}
fn primitive(p: P) -> Term {
    Term::Primitive(p)
}

#[test]
fn scoped_inference_keeps_caller_bindings_and_rejects_escape() {
    let caller = vec![Parameter {
        sort: Sort::Type,
        ty: primitive(P::Type),
    }];
    let local = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let identity = scheme(vec![generic(0)], generic(0), 1);
    let context = elaboration::ptc::Context::default();
    let candidate = identity
        .infer_scoped_types(
            &[Some(local.clone())],
            Some(&local),
            &context,
            Budget::default(),
            &caller,
        )
        .unwrap();
    assert!(
        candidate
            .engine()
            .to_string()
            .contains("(scope (parameter type ")
    );
    let instance = candidate.finish().unwrap();
    assert_eq!(instance.type_arguments, vec![local.clone()]);
    assert_eq!(instance.result, local.clone());
    assert!(instance.signature.check_scope(&[vec![Sort::Type]]).is_ok());
    assert!(instance.signature.check_scope(&[]).is_err());
    assert!(
        identity
            .infer_types(&[local.clone()], None, Budget::default())
            .is_err()
    );
    let outside = Term::Bound {
        depth: 1,
        index: 0,
        sort: Sort::Type,
    };
    assert!(
        identity
            .infer_scoped_types(&[Some(outside)], None, &context, Budget::default(), &caller)
            .is_err()
    );
    let wrong_sort = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Runtime,
    };
    assert!(
        identity
            .infer_scoped_types(
                &[Some(wrong_sort)],
                None,
                &context,
                Budget::default(),
                &caller
            )
            .is_err()
    );
    assert!(
        identity
            .infer_scoped_types(
                &[Some(local.clone())],
                Some(&primitive(P::Bool)),
                &context,
                Budget::default(),
                &caller
            )
            .and_then(|candidate| candidate.finish())
            .is_err()
    );
    assert_eq!(
        identity
            .infer_scoped_types(
                &[Some(local.clone())],
                None,
                &context,
                Budget::default(),
                &caller
            )
            .unwrap()
            .finish()
            .unwrap()
            .result,
        local
    );
}

#[test]
fn scoped_inference_distinguishes_callback_local_and_enclosing_bindings() {
    let caller = vec![Parameter {
        sort: Sort::Type,
        ty: primitive(P::Type),
    }];
    let context = elaboration::ptc::Context::default();
    let callback = |result| Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: primitive(P::Type),
        }],
        result: Box::new(result),
    };
    let generic_callback = callback(Term::Bound {
        depth: 2,
        index: 0,
        sort: Sort::Type,
    });
    let s = scheme(vec![generic_callback], generic(0), 1);
    let caller_callback = callback(Term::Bound {
        depth: 1,
        index: 0,
        sort: Sort::Type,
    });
    let instance = s
        .infer_scoped_types(
            &[Some(caller_callback)],
            None,
            &context,
            Budget::default(),
            &caller,
        )
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(
        instance.type_arguments,
        vec![Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type
        }]
    );
    let own_callback = callback(Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    });
    assert!(
        s.infer_scoped_types(
            &[Some(own_callback)],
            None,
            &context,
            Budget::default(),
            &caller
        )
        .and_then(|candidate| candidate.finish())
        .is_err()
    );
    let ordinary_callback = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: Term::Bound {
                depth: 1,
                index: 0,
                sort: Sort::Type,
            },
        }],
        result: Box::new(Term::Bound {
            depth: 1,
            index: 0,
            sort: Sort::Type,
        }),
    };
    let identity = scheme(vec![generic(0)], generic(0), 1);
    let candidate = identity
        .infer_scoped_types(
            &[Some(ordinary_callback)],
            None,
            &context,
            Budget::default(),
            &caller,
        )
        .unwrap();
    let view = candidate.parameter_callable_context(0).unwrap().unwrap();
    assert_eq!(
        view.parameters,
        vec![Some(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type
        })]
    );
    assert_eq!(
        view.result,
        Some(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type
        })
    );
}

#[test]
fn generic_identity_infers_type_and_result_without_mutating_scheme() {
    let scheme = scheme(vec![generic(0)], generic(0), 1);
    let original = scheme.signature.clone();
    for p in [P::Bool, P::U32] {
        let call = scheme
            .infer(
                &[argument(primitive(p), 0)],
                &[None],
                None,
                Budget::default(),
            )
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(call.type_arguments, vec![primitive(p)]);
        assert_eq!(call.result, primitive(p));
        assert!(
            call.to_string()
                .starts_with("(call-instance (type-arguments")
        );
    }
    assert_eq!(scheme.signature, original);
}

#[test]
fn whole_call_inference_accepts_later_information_for_earlier_opaque_contract() {
    let head = DeclarationId {
        package: 5,
        local: 1,
    };
    let opaque = |ty| Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![ty],
    };
    let scheme = scheme(vec![opaque(generic(0)), generic(0)], generic(0), 1);
    let args = [
        argument(opaque(primitive(P::Bool)), 0),
        argument(primitive(P::Bool), 1),
    ];
    let call = scheme
        .infer(&args, &[None], None, Budget::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(call.type_arguments, vec![primitive(P::Bool)]);
    assert_eq!(call.result, primitive(P::Bool));
}

#[test]
fn expected_result_can_determine_an_otherwise_unused_parameter() {
    let scheme = scheme(vec![], generic(0), 1);
    assert!(
        scheme
            .infer(&[], &[None], None, Budget::default())
            .unwrap()
            .finish()
            .is_err()
    );
    let call = scheme
        .infer(&[], &[None], Some(&primitive(P::Char)), Budget::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(call.result, primitive(P::Char));
}

#[test]
fn conflicting_arguments_or_explicit_type_never_publish_an_instance() {
    let scheme = scheme(vec![generic(0), generic(0)], generic(0), 1);
    let bad = [
        argument(primitive(P::Bool), 0),
        argument(primitive(P::Char), 1),
    ];
    assert!(
        scheme
            .infer(&bad, &[None], None, Budget::default())
            .unwrap()
            .finish()
            .is_err()
    );
    let good = [
        argument(primitive(P::Bool), 0),
        argument(primitive(P::Bool), 1),
    ];
    assert!(
        scheme
            .infer(&good, &[Some(primitive(P::Char))], None, Budget::default())
            .unwrap()
            .finish()
            .is_err()
    );
    assert!(
        scheme
            .infer(&good, &[None], None, Budget::default())
            .unwrap()
            .finish()
            .is_ok()
    );
}

#[test]
fn dependent_contract_and_result_retain_runtime_object_identity() {
    let head = DeclarationId {
        package: 7,
        local: 1,
    };
    let dep = |object| Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![object],
    };
    let bound = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Runtime,
    };
    let scheme = scheme(vec![primitive(P::Bool), dep(bound.clone())], dep(bound), 0);
    let first = argument(primitive(P::Bool), 0);
    let second = argument(dep(first.value.clone()), 1);
    let call = scheme
        .infer(&[first, second], &[], None, Budget::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(
        call.result,
        dep(Term::Runtime(RuntimeId {
            instance: 42,
            local: 0
        }))
    );
    let wrong = argument(
        dep(Term::Runtime(RuntimeId {
            instance: 43,
            local: 0,
        })),
        1,
    );
    assert!(
        scheme
            .infer(
                &[argument(primitive(P::Bool), 0), wrong],
                &[],
                None,
                Budget::default()
            )
            .unwrap()
            .finish()
            .is_err()
    );
}

#[test]
fn forward_dependencies_and_wrong_argument_domains_are_rejected() {
    let bad = scheme(
        vec![
            Term::NormalForm {
                head: DeclarationId {
                    package: 1,
                    local: 0,
                },
                sort: Sort::Type,
                arguments: vec![Term::Bound {
                    depth: 0,
                    index: 1,
                    sort: Sort::Runtime,
                }],
            },
            primitive(P::Bool),
        ],
        primitive(P::Void),
        0,
    );
    assert!(
        bad.infer(
            &[
                argument(primitive(P::Bool), 0),
                argument(primitive(P::Bool), 1)
            ],
            &[],
            None,
            Budget::default()
        )
        .is_err()
    );
    let good = scheme(vec![primitive(P::Bool)], primitive(P::Bool), 0);
    assert!(good.infer(&[], &[], None, Budget::default()).is_err());
    assert!(
        good.infer(
            &[Argument {
                ty: primitive(P::Bool),
                value: Term::Bool(true)
            }],
            &[],
            None,
            Budget::default()
        )
        .is_err()
    );
}

#[test]
fn type_only_inference_rejects_runtime_dependent_signatures() {
    let independent = scheme(vec![generic(0)], generic(0), 1);
    let instance = independent
        .infer_types(&[primitive(P::Bool)], None, Budget::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(instance.type_arguments, vec![primitive(P::Bool)]);
    let dependent = scheme(
        vec![primitive(P::Bool)],
        Term::NormalForm {
            head: DeclarationId {
                package: 0,
                local: 88,
            },
            sort: Sort::Type,
            arguments: vec![Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Runtime,
            }],
        },
        0,
    );
    assert!(
        dependent
            .infer_types(&[primitive(P::Bool)], None, Budget::default())
            .is_err()
    );
}

#[test]
fn partial_context_does_not_require_unrelated_parameters_to_be_solved() {
    let callback = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: Term::Bound {
                depth: 2,
                index: 0,
                sort: Sort::Type,
            },
        }],
        result: Box::new(Term::Bound {
            depth: 2,
            index: 0,
            sort: Sort::Type,
        }),
    };
    let scheme = scheme(vec![generic(0), callback, generic(1)], generic(1), 2);
    let candidate = scheme
        .infer_partial_types(
            &[Some(primitive(P::Bool)), None, None],
            None,
            Budget::default(),
        )
        .unwrap();
    assert_eq!(
        candidate.parameter_type(0).unwrap(),
        Some(primitive(P::Bool))
    );
    assert_eq!(
        candidate.parameter_type(1).unwrap(),
        Some(Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: primitive(P::Bool)
            }],
            result: Box::new(primitive(P::Bool))
        })
    );
    assert_eq!(candidate.parameter_type(2).unwrap(), None);
    assert!(candidate.finish().is_err());
}

#[test]
fn structural_result_contexts_keep_holes_private_and_recover_from_failure() {
    use elaboration::calls::InputContext as C;
    let scheme = scheme(vec![generic(0)], generic(0), 1);
    let context = C::Tuple(vec![C::Known(primitive(P::U8)), C::Unknown]);
    let mut candidate = scheme
        .infer_partial_types(&[None], None, Budget::default())
        .unwrap();
    assert!(
        candidate
            .constrain_result_context(
                &context,
                Budget {
                    steps: 1,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    assert_eq!(candidate.parameter_type(0).unwrap(), None);
    candidate
        .constrain_result_context(&context, Budget::default())
        .unwrap();
    let C::Tuple(fields) = candidate.parameter_input_context(0).unwrap() else {
        panic!("tuple context")
    };
    assert!(matches!(&fields[0], C::Known(ty) if *ty == primitive(P::U8)));
    assert!(matches!(&fields[1], C::Unknown));
    assert!(candidate.finish().is_err());

    for (first, accepted) in [(P::U8, true), (P::Bool, false)] {
        let actual = Term::Tuple(vec![primitive(first), primitive(P::Char)]);
        let mut candidate = scheme
            .infer_types(&[actual.clone()], None, Budget::default())
            .unwrap();
        let result = candidate
            .constrain_result_context(&context, Budget::default())
            .and_then(|_| candidate.finish());
        if accepted {
            let instance = result.unwrap();
            assert_eq!(instance.result, actual);
            assert_eq!(instance.type_arguments, vec![actual]);
        } else {
            assert!(result.is_err());
        }
    }
    let mut candidate = scheme
        .infer_partial_types(&[None], None, Budget::default())
        .unwrap();
    let invalid = C::Known(Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    });
    assert!(
        candidate
            .constrain_result_context(&invalid, Budget::default())
            .is_err()
    );
    // Invalid caller-relative data never changes the existing candidate.
    candidate
        .constrain_result_context(&C::Known(primitive(P::Bool)), Budget::default())
        .unwrap();
    assert_eq!(candidate.finish().unwrap().result, primitive(P::Bool));
}

#[test]
fn record_callback_context_preserves_names_and_binder_frames() {
    use elaboration::calls::InputContext as C;
    let callback = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: primitive(P::U8),
        }],
        result: Box::new(Term::Bound {
            depth: 2,
            index: 0,
            sort: Sort::Type,
        }),
    };
    let scheme = scheme(
        vec![Term::Record(vec![
            ("callback".into(), callback),
            ("value".into(), primitive(P::U8)),
        ])],
        generic(0),
        1,
    );
    let candidate = scheme
        .infer_partial_types(&[None], None, Budget::default())
        .unwrap();
    let C::Record(fields) = candidate.parameter_input_context(0).unwrap() else {
        panic!("tuple")
    };
    assert_eq!(
        fields
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["callback", "value"]
    );
    assert!(matches!(&fields[1].1, C::Known(ty) if *ty == primitive(P::U8)));
    let C::Callable(callback) = &fields[0].1 else {
        panic!("callback")
    };
    assert_eq!(callback.parameters, vec![Some(primitive(P::U8))]);
    assert_eq!(callback.result, None);
    assert!(!callback.comptime);
    assert_eq!(callback.environment, None);
    assert!(candidate.finish().is_err());
}

#[test]
fn tuple_callback_context_preserves_its_own_binder_frame() {
    use elaboration::calls::InputContext as C;
    let callback = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: primitive(P::U8),
        }],
        result: Box::new(Term::Bound {
            depth: 2,
            index: 0,
            sort: Sort::Type,
        }),
    };
    let scheme = scheme(
        vec![Term::Tuple(vec![callback, primitive(P::U8)])],
        generic(0),
        1,
    );
    let candidate = scheme
        .infer_partial_types(&[None], None, Budget::default())
        .unwrap();
    let C::Tuple(fields) = candidate.parameter_input_context(0).unwrap() else {
        panic!("tuple")
    };
    let C::Callable(callback) = &fields[0] else {
        panic!("callback")
    };
    assert_eq!(callback.parameters, vec![Some(primitive(P::U8))]);
    assert_eq!(callback.result, None);
    assert!(!callback.comptime);
    assert_eq!(callback.environment, None);
    assert!(candidate.finish().is_err());
}

#[test]
fn tuple_context_keeps_unknown_components_private() {
    use elaboration::calls::InputContext as C;
    let scheme = scheme(
        vec![Term::Tuple(vec![
            generic(0),
            Term::Tuple(vec![primitive(P::U8), generic(1)]),
        ])],
        primitive(P::Bool),
        2,
    );
    let candidate = scheme
        .infer_partial_types(&[None], None, Budget::default())
        .unwrap();
    assert_eq!(candidate.parameter_type(0).unwrap(), None);
    let C::Tuple(fields) = candidate.parameter_input_context(0).unwrap() else {
        panic!("tuple context")
    };
    assert!(matches!(fields[0], C::Unknown));
    let C::Tuple(nested) = &fields[1] else {
        panic!("nested context")
    };
    assert!(matches!(&nested[0],C::Known(ty) if *ty==primitive(P::U8)));
    assert!(matches!(nested[1], C::Unknown));
    assert!(candidate.finish().is_err());
}

#[test]
fn callback_context_freezes_inputs_independently_of_its_result() {
    let nested = |index| Term::Bound {
        depth: 2,
        index,
        sort: Sort::Type,
    };
    let callback = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: nested(0),
        }],
        result: Box::new(nested(1)),
    };
    let scheme = scheme(vec![generic(0), callback], generic(1), 2);
    let candidate = scheme
        .infer_partial_types(&[Some(primitive(P::Bool)), None], None, Budget::default())
        .unwrap();
    assert_eq!(candidate.parameter_type(1).unwrap(), None);
    let context = candidate.parameter_callable_context(1).unwrap().unwrap();
    assert_eq!(context.parameters, [Some(primitive(P::Bool))]);
    assert_eq!(context.result, None);
    assert!(candidate.finish().is_err());
    let actual_callback = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: primitive(P::Bool),
        }],
        result: Box::new(primitive(P::Char)),
    };
    let complete = scheme
        .infer_types(
            &[primitive(P::Bool), actual_callback],
            None,
            Budget::default(),
        )
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(
        complete.type_arguments,
        [primitive(P::Bool), primitive(P::Char)]
    );
}

#[test]
fn partial_result_shape_passes_through_a_generic_identity() {
    let scheme = scheme(vec![generic(0)], generic(0), 1);
    let context = elaboration::calls::CallableContext {
        comptime: false,
        environment: None,
        result_context: None,
        parameters: vec![Some(primitive(P::Bool))],
        result: None,
    };
    let candidate = scheme
        .infer_contextual_types(
            &[None],
            None,
            Some(&context),
            &elaboration::ptc::Context::default(),
            Budget::default(),
        )
        .unwrap();
    assert_eq!(candidate.parameter_type(0).unwrap(), None);
    let parameter = candidate.parameter_callable_context(0).unwrap().unwrap();
    assert_eq!(parameter.parameters, context.parameters);
    assert_eq!(parameter.result, None);
    // Inspecting the input shape never authorizes an unresolved instance.
    assert!(candidate.finish().is_err());
    let callback = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: primitive(P::Bool),
        }],
        result: Box::new(primitive(P::Char)),
    };
    let complete = scheme
        .infer_contextual_types(
            &[Some(callback.clone())],
            None,
            Some(&context),
            &elaboration::ptc::Context::default(),
            Budget::default(),
        )
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(complete.result, callback);
}

#[test]
fn inspecting_candidate_context_cannot_export_a_conflicting_call() {
    let scheme = scheme(vec![generic(0)], generic(0), 1);
    let context = elaboration::calls::CallableContext {
        comptime: false,
        environment: None,
        result_context: None,
        parameters: vec![Some(primitive(P::Bool))],
        result: Some(primitive(P::Char)),
    };
    let callback = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: primitive(P::Bool),
        }],
        result: Box::new(primitive(P::Bool)),
    };
    let candidate = scheme
        .infer_contextual_types(
            &[Some(callback.clone())],
            None,
            Some(&context),
            &elaboration::ptc::Context::default(),
            Budget::default(),
        )
        .unwrap();
    assert_eq!(candidate.parameter_type(0).unwrap(), Some(callback));
    assert!(candidate.finish().is_err());
}

#[test]
fn pending_context_does_not_hide_an_independent_type_conflict() {
    let scheme = scheme(vec![generic(0), generic(0)], generic(1), 2);
    let partial = elaboration::calls::CallableContext {
        comptime: false,
        environment: None,
        result_context: None,
        parameters: vec![Some(primitive(P::Bool))],
        result: None,
    };
    let candidate = scheme
        .infer_contextual_types(
            &[Some(primitive(P::Bool)), Some(primitive(P::Char))],
            None,
            Some(&partial),
            &elaboration::ptc::Context::default(),
            Budget::default(),
        )
        .unwrap();
    assert!(matches!(
        candidate.finish(),
        Err(elaboration::calls::Error::Obligations(_))
    ));
}

#[test]
fn partial_comptime_closure_context_preserves_environment_and_phase() {
    let scheme = scheme(vec![generic(0)], generic(0), 1);
    let context = elaboration::calls::CallableContext {
        comptime: true,
        environment: Some(vec![Some(primitive(P::Bool)), None]),
        result_context: None,
        parameters: vec![Some(primitive(P::Bool))],
        result: None,
    };
    let candidate = scheme
        .infer_contextual_types(
            &[None],
            None,
            Some(&context),
            &elaboration::ptc::Context::default(),
            Budget::default(),
        )
        .unwrap();
    let forwarded = candidate.parameter_callable_context(0).unwrap().unwrap();
    assert!(forwarded.comptime);
    assert_eq!(forwarded.environment, context.environment);
    assert_eq!(forwarded.parameters, context.parameters);
    assert_eq!(forwarded.result, None);
    assert!(candidate.finish().is_err());
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: primitive(P::Bool),
        }],
        result: Box::new(primitive(P::Char)),
    }
    .with_comptime_phase(true);
    let closure = Term::Closure {
        environment: vec![primitive(P::Bool), primitive(P::U32)],
        signature: Box::new(signature.clone()),
    };
    let complete = scheme
        .infer_contextual_types(
            &[Some(closure.clone())],
            None,
            Some(&context),
            &elaboration::ptc::Context::default(),
            Budget::default(),
        )
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(complete.result, closure);
    // A matching plain signature still cannot discard the required environment.
    assert!(
        scheme
            .infer_contextual_types(
                &[Some(signature)],
                None,
                Some(&context),
                &elaboration::ptc::Context::default(),
                Budget::default(),
            )
            .and_then(|c| c.finish())
            .is_err()
    );
}

#[test]
fn stable_type_views_resume_obligations_transactionally_and_remain_candidate_local() {
    let projection = |subject| Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: DeclarationId {
                package: 7,
                local: 1,
            },
            member: DeclarationId {
                package: 7,
                local: 2,
            },
        }),
        arguments: vec![],
        subject: Box::new(subject),
    };
    let scheme = scheme(
        vec![generic(0), projection(generic(0))],
        projection(generic(0)),
        1,
    );
    let actual = vec![Some(primitive(P::Bool)), Some(primitive(P::Bool))];
    let mut candidate = scheme
        .infer_partial_types(&actual, None, Budget::default())
        .unwrap();
    let before = projection(primitive(P::Bool));
    assert_eq!(candidate.parameter_type(1).unwrap(), Some(before.clone()));
    let mut queries = 0;
    assert!(
        candidate
            .refine_type_views(
                |ty, _| {
                    queries += 1;
                    if queries == 3 {
                        return Err("failed dependent query".into());
                    }
                    Ok(if *ty == before {
                        primitive(P::Bool)
                    } else {
                        ty.clone()
                    })
                },
                Budget::default()
            )
            .is_err()
    );
    assert_eq!(candidate.parameter_type(1).unwrap(), Some(before.clone()));
    candidate
        .refine_type_views(
            |ty, _| {
                Ok(if *ty == before {
                    primitive(P::Bool)
                } else {
                    ty.clone()
                })
            },
            Budget::default(),
        )
        .unwrap();
    assert_eq!(
        candidate.parameter_type(1).unwrap(),
        Some(primitive(P::Bool))
    );
    let instance = candidate.finish().unwrap();
    assert_eq!(instance.type_arguments, vec![primitive(P::Bool)]);
    // The selected view does not rewrite the original declaration signature.
    assert_eq!(instance.result, before);
    assert!(
        scheme
            .infer_partial_types(&actual, None, Budget::default())
            .unwrap()
            .finish()
            .is_err()
    );
    let wrong = vec![Some(primitive(P::Bool)), Some(primitive(P::Char))];
    let mut candidate = scheme
        .infer_partial_types(&wrong, None, Budget::default())
        .unwrap();
    candidate
        .refine_type_views(
            |ty, _| {
                Ok(if *ty == before {
                    primitive(P::Bool)
                } else {
                    ty.clone()
                })
            },
            Budget::default(),
        )
        .unwrap();
    assert!(candidate.finish().is_err());
}

#[test]
fn bound_tuple_inference_proposes_fields_and_preserves_context_identity() {
    use elaboration::{calls::InputContext as C, ptc::Context};
    use nir::normal_forms::{Head, Kind, Parameter as Input, Role};
    let head = DeclarationId {
        package: 55,
        local: 1,
    };
    let tuple = |fields| Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(fields)],
    };
    let mut package = nir::program::Package {
        normal_forms: vec![Head {
            declaration: head,
            role: Role::Tuple,
            requirements: vec![],
            parameters: vec![Input {
                declaration: DeclarationId {
                    package: 55,
                    local: 2,
                },
                binding: nir::program::LocalId { owner: 1, index: 0 },
                kind: Kind::TypeList,
                ty: Term::Nominal {
                    declaration: DeclarationId {
                        package: 55,
                        local: 3,
                    },
                    arguments: vec![primitive(P::Type)],
                },
            }],
        }],
        ..Default::default()
    };
    let context = Context::from_package(&package).unwrap();
    let definition = scheme(
        vec![tuple(vec![generic(0), primitive(P::U8)])],
        generic(0),
        1,
    );
    let pending = definition
        .infer_partial_types_in_context(&[None], None, &context, Budget::default())
        .unwrap();
    let C::NormalTuple {
        head: actual,
        fields,
    } = pending.parameter_input_context(0).unwrap()
    else {
        panic!("normal tuple context");
    };
    assert_eq!(actual, head);
    assert!(matches!(&fields[0], C::Unknown));
    assert!(matches!(&fields[1], C::Known(ty) if *ty == primitive(P::U8)));
    let actual = tuple(vec![primitive(P::Bool), primitive(P::U8)]);
    let instance = definition
        .infer_partial_types_in_context(&[Some(actual.clone())], None, &context, Budget::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(instance.type_arguments, vec![primitive(P::Bool)]);
    assert_eq!(instance.result, primitive(P::Bool));
    let producer = scheme(vec![generic(0)], generic(0), 1);
    let mut candidate = producer
        .infer_partial_types_in_context(&[None], None, &context, Budget::default())
        .unwrap();
    let partial = C::NormalTuple {
        head,
        fields: vec![C::Known(primitive(P::U8)), C::Unknown],
    };
    assert!(
        candidate
            .constrain_result_context(
                &partial,
                Budget {
                    steps: 1,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    candidate
        .constrain_result_context(&partial, Budget::default())
        .unwrap();
    let C::NormalTuple {
        head: retained,
        fields,
    } = candidate.parameter_input_context(0).unwrap()
    else {
        panic!("retained tuple head");
    };
    assert_eq!(retained, head);
    assert!(matches!(&fields[0], C::Known(ty) if *ty == primitive(P::U8)));
    assert!(matches!(&fields[1], C::Unknown));
    assert!(candidate.finish().is_err());
    let invalid = tuple(vec![primitive(P::Bool), primitive(P::Char)]);
    assert!(
        definition
            .infer_partial_types_in_context(&[Some(invalid)], None, &context, Budget::default())
            .and_then(|candidate| candidate.finish())
            .is_err()
    );
    package.normal_forms[0].role = Role::Opaque;
    let opaque = Context::from_package(&package).unwrap();
    assert!(
        definition
            .infer_partial_types_in_context(&[Some(actual)], None, &opaque, Budget::default())
            .and_then(|candidate| candidate.finish())
            .is_err()
    );
}

#[test]
fn partial_nominal_contexts_preserve_known_arguments_without_exporting_holes() {
    use elaboration::{calls::InputContext as C, ptc::Context};
    let id = DeclarationId {
        package: 77,
        local: 1,
    };
    let nominal = |arguments| Term::Nominal {
        declaration: id,
        arguments,
    };
    let mut world = Context::default();
    world.admit_nominal_constructor(id);
    let consumer = scheme(
        vec![nominal(vec![generic(0), primitive(P::U8)])],
        generic(0),
        1,
    );
    let pending = consumer
        .infer_partial_types_in_context(&[None], None, &world, Budget::default())
        .unwrap();
    let shape = pending.parameter_input_context(0).unwrap();
    let C::Nominal {
        declaration,
        arguments,
    } = &shape
    else {
        panic!("partial nominal context")
    };
    assert_eq!(*declaration, id);
    assert!(matches!(&arguments[..],[C::Unknown,C::Known(ty)] if *ty==primitive(P::U8)));
    assert!(pending.finish().is_err());

    let producer = scheme(vec![generic(0)], generic(0), 1);
    let mut trial = producer
        .infer_partial_types_in_context(&[None], None, &world, Budget::default())
        .unwrap();
    assert!(
        trial
            .constrain_result_context(
                &shape,
                Budget {
                    steps: 1,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    trial
        .constrain_result_context(&shape, Budget::default())
        .unwrap();
    assert!(
        matches!(trial.parameter_input_context(0).unwrap(),C::Nominal {declaration,..} if declaration==id)
    );
    assert!(trial.finish().is_err());

    let completed = nominal(vec![primitive(P::Bool), primitive(P::U8)]);
    let mut trial = producer
        .infer_partial_types_in_context(&[Some(completed.clone())], None, &world, Budget::default())
        .unwrap();
    trial
        .constrain_result_context(&shape, Budget::default())
        .unwrap();
    assert_eq!(trial.finish().unwrap().result, completed);
    let mut mismatch = producer
        .infer_partial_types_in_context(
            &[Some(nominal(vec![primitive(P::Bool), primitive(P::Char)]))],
            None,
            &world,
            Budget::default(),
        )
        .unwrap();
    assert!(
        mismatch
            .constrain_result_context(&shape, Budget::default())
            .and_then(|_| mismatch.finish())
            .is_err()
    );

    let unknown = Context::default();
    let pending = consumer
        .infer_partial_types_in_context(&[None], None, &unknown, Budget::default())
        .unwrap();
    assert!(matches!(
        pending.parameter_input_context(0).unwrap(),
        C::Unknown
    ));
    let mut trial = producer
        .infer_partial_types_in_context(&[None], None, &unknown, Budget::default())
        .unwrap();
    assert!(
        trial
            .constrain_result_context(&shape, Budget::default())
            .is_err()
    );
}

#[test]
fn partial_callable_nominal_results_survive_forwarding_and_reject_conflicts() {
    use elaboration::{calls::InputContext as C, ptc::Context};
    let id = DeclarationId {
        package: 78,
        local: 1,
    };
    let nominal = |arguments| Term::Nominal {
        declaration: id,
        arguments,
    };
    let callback = |result| Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: primitive(P::Bool),
        }],
        result: Box::new(result),
    };
    let mut world = Context::default();
    world.admit_nominal_constructor(id);
    let consumer = scheme(
        vec![callback(nominal(vec![
            Term::Bound {
                depth: 2,
                index: 0,
                sort: Sort::Type,
            },
            primitive(P::U8),
        ]))],
        generic(0),
        1,
    );
    let pending = consumer
        .infer_partial_types_in_context(&[None], None, &world, Budget::default())
        .unwrap();
    let shape = pending.parameter_input_context(0).unwrap();
    let C::Callable(context) = &shape else {
        panic!("callable shape")
    };
    assert_eq!(context.parameters, vec![Some(primitive(P::Bool))]);
    assert!(context.result.is_none());
    let Some(C::Nominal {
        declaration,
        arguments,
    }) = context.result_context.as_deref()
    else {
        panic!("nominal result")
    };
    assert_eq!(*declaration, id);
    assert!(matches!(&arguments[..],[C::Unknown,C::Known(ty)] if *ty==primitive(P::U8)));
    assert!(pending.finish().is_err());
    let identity = scheme(vec![generic(0)], generic(0), 1);
    let mut forwarding = identity
        .infer_partial_types_in_context(&[None], None, &world, Budget::default())
        .unwrap();
    assert!(
        forwarding
            .constrain_result_context(
                &shape,
                Budget {
                    steps: 1,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    forwarding
        .constrain_result_context(&shape, Budget::default())
        .unwrap();
    let C::Callable(forwarded) = forwarding.parameter_input_context(0).unwrap() else {
        panic!("forwarded callable")
    };
    assert!(
        matches!(forwarded.result_context.as_deref(),Some(C::Nominal {declaration,..}) if *declaration==id)
    );
    assert!(forwarding.finish().is_err());
    for (error_type, accepted) in [(P::U8, true), (P::Char, false)] {
        let completed = callback(nominal(vec![primitive(P::Bool), primitive(error_type)]));
        let mut candidate = identity
            .infer_partial_types_in_context(
                &[Some(completed.clone())],
                None,
                &world,
                Budget::default(),
            )
            .unwrap();
        let result = candidate
            .constrain_result_context(&shape, Budget::default())
            .and_then(|_| candidate.finish());
        assert_eq!(result.is_ok(), accepted);
        if let Ok(instance) = result {
            assert_eq!(instance.result, completed);
        }
    }
}

#[test]
fn candidate_type_equations_are_transactional_and_do_not_invert_opaque_heads() {
    let template = scheme(vec![], generic(0), 1);
    let bound = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let mut candidate = template.infer_types(&[], None, Budget::default()).unwrap();
    assert!(
        candidate
            .constrain_type_equations(
                &[(bound.clone(), primitive(P::Bool))],
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    candidate
        .constrain_type_equations(&[(bound.clone(), primitive(P::Char))], Budget::default())
        .unwrap();
    assert_eq!(candidate.finish().unwrap().result, primitive(P::Char));
    let opaque = |argument| Term::NormalForm {
        head: DeclarationId {
            package: 77,
            local: 0,
        },
        arguments: vec![argument],
        sort: Sort::Type,
    };
    let mut candidate = template.infer_types(&[], None, Budget::default()).unwrap();
    candidate
        .constrain_type_equations(
            &[(opaque(bound), opaque(primitive(P::Bool)))],
            Budget::default(),
        )
        .unwrap();
    assert!(candidate.finish().is_err());
}

#[test]
fn sparse_caller_inference_retains_slots_and_lexical_assumptions() {
    use elaboration::ptc::{Context, Formula};
    let scope = vec![
        (Sort::Runtime, None),
        (Sort::Type, Some(primitive(P::Type))),
    ];
    let local = Term::Bound {
        depth: 0,
        index: 1,
        sort: Sort::Type,
    };
    let identity = scheme(vec![generic(0)], generic(0), 1);
    let candidate = identity
        .infer_sparse_scoped_types(
            &[Some(local.clone())],
            Some(&local),
            &Context::default(),
            Budget::default(),
            &scope,
        )
        .unwrap();
    assert!(
        candidate
            .engine()
            .to_string()
            .contains("(parameter runtime (unknown-domain))")
    );
    let instance = candidate.finish().unwrap();
    assert_eq!(instance.type_arguments, vec![local.clone()]);
    instance
        .signature
        .check_scope(&[vec![Sort::Runtime, Sort::Type]])
        .unwrap();

    let mut context = Context::with_scope(vec![vec![Sort::Runtime, Sort::Type]]);
    context.assume(Formula::equal(local.clone(), primitive(P::Bool)));
    identity
        .infer_sparse_scoped_types(
            &[Some(local.clone())],
            Some(&primitive(P::Bool)),
            &context,
            Budget::default(),
            &scope,
        )
        .unwrap()
        .finish()
        .unwrap();
    let wrong_scope = Context::with_scope(vec![vec![Sort::Type]]);
    assert!(
        identity
            .infer_sparse_scoped_types(
                &[Some(local.clone())],
                None,
                &wrong_scope,
                Budget::default(),
                &scope
            )
            .is_err()
    );
    // An invalid closed assumption must not become valid merely by adding slots.
    let mut invalid = Context::default();
    invalid.assume(Formula::equal(local.clone(), primitive(P::Bool)));
    assert!(
        identity
            .infer_sparse_scoped_types(&[Some(local)], None, &invalid, Budget::default(), &scope)
            .is_err()
    );
    assert!(
        identity
            .infer_sparse_scoped_types(
                &[Some(primitive(P::Bool))],
                None,
                &Context::default(),
                Budget::default(),
                &[(Sort::Type, None)]
            )
            .is_err()
    );
}

#[test]
fn sparse_caller_inference_preserves_closed_assumptions() {
    use elaboration::ptc::{Context, Formula};
    let left = Term::Rigid {
        declaration: DeclarationId {
            package: 7,
            local: 1,
        },
        sort: Sort::Type,
    };
    let right = Term::Rigid {
        declaration: DeclarationId {
            package: 7,
            local: 2,
        },
        sort: Sort::Type,
    };
    let mut context = Context::default();
    context.assume(Formula::equal(left.clone(), right.clone()));
    let identity = scheme(vec![generic(0)], generic(0), 1);
    identity
        .infer_sparse_scoped_types(
            &[Some(left)],
            Some(&right),
            &context,
            Budget::default(),
            &[(Sort::Runtime, None)],
        )
        .unwrap()
        .finish()
        .unwrap();
}

#[test]
fn partial_nominal_value_indices_keep_identity_scope_and_budget_recovery() {
    use elaboration::{calls::InputContext as C, ptc::Context};
    let id = DeclarationId {
        package: 79,
        local: 1,
    };
    let nominal = |ty, flag| Term::Nominal {
        declaration: id,
        arguments: vec![ty, flag],
    };
    let mut world = Context::default();
    world.admit_nominal_constructor(id);
    let consumer = scheme(vec![nominal(generic(0), Term::Bool(true))], generic(0), 1);
    let pending = consumer
        .infer_partial_types_in_context(&[None], None, &world, Budget::default())
        .unwrap();
    let shape = pending.parameter_input_context(0).unwrap();
    assert!(matches!(&shape, C::Nominal { arguments, .. }
        if matches!(arguments.as_slice(), [C::Unknown, C::Known(Term::Bool(true))])));
    assert!(pending.finish().is_err());
    let producer = scheme(vec![generic(0)], generic(0), 1);
    let complete = nominal(primitive(P::Bool), Term::Bool(true));
    let mut trial = producer
        .infer_partial_types_in_context(&[Some(complete.clone())], None, &world, Budget::default())
        .unwrap();
    assert!(
        trial
            .constrain_result_context(
                &shape,
                Budget {
                    steps: 1,
                    ..Budget::default()
                }
            )
            .is_err()
    );
    trial
        .constrain_result_context(&shape, Budget::default())
        .unwrap();
    assert_eq!(trial.finish().unwrap().result, complete);
    let mut wrong = producer
        .infer_partial_types_in_context(
            &[Some(nominal(primitive(P::Bool), Term::Bool(false)))],
            None,
            &world,
            Budget::default(),
        )
        .unwrap();
    assert!(
        wrong
            .constrain_result_context(&shape, Budget::default())
            .and_then(|_| wrong.finish().map(|_| ()))
            .is_err()
    );
    let unbound = C::Nominal {
        declaration: id,
        arguments: vec![
            C::Unknown,
            C::Known(Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::ComptimeValue,
            }),
        ],
    };
    let mut trial = producer
        .infer_partial_types_in_context(&[Some(complete.clone())], None, &world, Budget::default())
        .unwrap();
    assert!(
        trial
            .constrain_result_context(&unbound, Budget::default())
            .is_err()
    );
    trial
        .constrain_result_context(&shape, Budget::default())
        .unwrap();
    assert_eq!(trial.finish().unwrap().result, complete);
}

#[test]
fn mixed_clause_sorts_infer_values_without_turning_them_into_types() {
    let head = DeclarationId {
        package: 4,
        local: 1,
    };
    let nominal = |arguments| Term::Nominal {
        declaration: head,
        arguments,
    };
    let mut mixed = scheme(
        vec![nominal(vec![
            generic(0),
            Term::Bound {
                depth: 1,
                index: 1,
                sort: Sort::ComptimeValue,
            },
        ])],
        generic(0),
        2,
    );
    mixed.parameter_sorts = vec![Sort::Type, Sort::ComptimeValue];
    let mut context = elaboration::ptc::Context::default();
    context.admit_nominal_constructor(head);
    for value in [true, false] {
        let candidate = mixed
            .infer_sparse_scoped_types(
                &[Some(nominal(vec![primitive(P::Bool), Term::Bool(value)]))],
                Some(&primitive(P::Bool)),
                &context,
                Budget::default(),
                &[],
            )
            .unwrap()
            .finish()
            .unwrap();
        assert_eq!(
            candidate.type_arguments,
            vec![primitive(P::Bool), Term::Bool(value)]
        );
        assert_eq!(candidate.result, primitive(P::Bool));
    }
    let malformed = nominal(vec![primitive(P::Bool), primitive(P::Char)]);
    assert!(
        mixed
            .infer_sparse_scoped_types(&[Some(malformed)], None, &context, Budget::default(), &[])
            .and_then(|candidate| candidate.finish())
            .is_err()
    );
    mixed.parameter_sorts = vec![Sort::Type, Sort::Runtime];
    assert!(
        mixed
            .infer_sparse_scoped_types(&[None], None, &context, Budget::default(), &[])
            .is_err()
    );
}

#[test]
fn unknown_nominal_value_contexts_keep_their_sort_across_calls() {
    use elaboration::{calls::InputContext as C, ptc::Context};
    let id = DeclarationId {
        package: 80,
        local: 1,
    };
    let nominal = |index| Term::Nominal {
        declaration: id,
        arguments: vec![index],
    };
    let mut context = Context::default();
    context.admit_nominal_constructor(id);
    let mut consumer = scheme(
        vec![nominal(Term::Bound {
            depth: 1,
            index: 0,
            sort: Sort::ComptimeValue,
        })],
        primitive(P::Bool),
        1,
    );
    consumer.parameter_sorts = vec![Sort::ComptimeValue];
    let pending = consumer
        .infer_partial_types_in_context(&[None], None, &context, Budget::default())
        .unwrap();
    let hint = pending.parameter_input_context(0).unwrap();
    assert!(
        matches!(&hint, C::Nominal { arguments, .. } if matches!(arguments.as_slice(), [C::UnknownValue]))
    );
    assert!(pending.finish().is_err());
    let producer = scheme(vec![generic(0)], generic(0), 1);
    for flag in [true, false] {
        let complete = nominal(Term::Bool(flag));
        let mut candidate = producer
            .infer_partial_types_in_context(
                &[Some(complete.clone())],
                None,
                &context,
                Budget::default(),
            )
            .unwrap();
        assert!(
            candidate
                .constrain_result_context(
                    &hint,
                    Budget {
                        steps: 1,
                        ..Budget::default()
                    }
                )
                .is_err()
        );
        candidate
            .constrain_result_context(&hint, Budget::default())
            .unwrap();
        assert_eq!(candidate.finish().unwrap().result, complete);
    }
    let mut wrong = producer
        .infer_partial_types_in_context(
            &[Some(nominal(primitive(P::Bool)))],
            None,
            &context,
            Budget::default(),
        )
        .unwrap();
    assert!(
        wrong
            .constrain_result_context(&hint, Budget::default())
            .and_then(|_| wrong.finish().map(|_| ()))
            .is_err()
    );
}
