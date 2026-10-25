use nir::*;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

const SENSOR: DeclarationId = DeclarationId {
    package: 1,
    local: 10,
};
const CALIBRATION: DeclarationId = DeclarationId {
    package: 1,
    local: 11,
};
const IDENTITY: DeclarationId = DeclarationId {
    package: 1,
    local: 12,
};
fn primitive(p: Primitive) -> Term {
    Term::Primitive(p)
}
fn hole(index: usize, sort: Sort) -> Term {
    Term::Hole { index, sort }
}
fn bound(depth: u32, index: usize, sort: Sort) -> Term {
    Term::Bound { depth, index, sort }
}

#[test]
fn scoped_declaration_substitution_preserves_local_and_caller_frames() {
    let generic = Term::Rigid {
        declaration: SENSOR,
        sort: Sort::Type,
    };
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: primitive(Primitive::Type),
        }],
        result: Box::new(Term::Tuple(vec![bound(0, 0, Sort::Type), generic])),
    };
    let caller = bound(0, 0, Sort::Type);
    let replaced = signature
        .instantiate_types_in_scope(&[SENSOR], &[caller.clone()], &[vec![Sort::Type]])
        .unwrap();
    assert_eq!(
        replaced,
        Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Type,
                ty: primitive(Primitive::Type)
            }],
            result: Box::new(Term::Tuple(vec![
                bound(0, 0, Sort::Type),
                bound(1, 0, Sort::Type)
            ])),
        }
    );
    assert!(
        signature
            .instantiate_types(&[SENSOR], &[caller.clone()])
            .is_err()
    );
    assert!(
        signature
            .instantiate_types_in_scope(&[SENSOR], &[caller], &[vec![Sort::Runtime]])
            .is_err()
    );
    assert!(
        signature
            .instantiate_types_in_scope(&[SENSOR], &[bound(1, 0, Sort::Type)], &[vec![Sort::Type]])
            .is_err()
    );
    assert_eq!(
        replaced.substitute(&[primitive(Primitive::Bool)]).unwrap(),
        signature
            .instantiate_types(&[SENSOR], &[primitive(Primitive::Bool)])
            .unwrap()
    );
}
fn object(instance: u64) -> Term {
    Term::Runtime(RuntimeId { instance, local: 0 })
}

#[test]
fn function_input_domain_views_refine_sorts_before_binding_and_compose() {
    let projection = Term::AssociatedType {
        origin: Box::new(AssociatedTypeOrigin {
            interface: SENSOR,
            member: CALIBRATION,
        }),
        subject: Box::new(bound(0, 0, Sort::Type)),
        arguments: vec![],
    };
    let indexed = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: vec![bound(0, 1, Sort::ComptimeValue)],
    };
    let signature = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Type,
                ty: primitive(Primitive::Type),
            },
            Parameter {
                sort: Sort::ComptimeValue,
                ty: projection,
            },
            Parameter {
                sort: Sort::Runtime,
                ty: indexed.clone(),
            },
        ],
        result: Box::new(indexed.clone()),
    };
    let view = |ty: &Term, _: &[Parameter]| -> Result<Term, TypeError> {
        Ok(match ty {
            Term::AssociatedType { subject, .. } if **subject == primitive(Primitive::Bool) => {
                primitive(Primitive::Type)
            }
            _ => ty.clone(),
        })
    };
    let inputs = vec![
        Some(primitive(Primitive::Bool)),
        Some(primitive(Primitive::Char)),
        None,
    ];
    assert!(signature.prepare_function_inputs(&inputs).is_err());
    let full = signature
        .prepare_function_inputs_with_type_view(&inputs, view)
        .unwrap();
    let expected = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: vec![primitive(Primitive::Char)],
    };
    assert_eq!(
        full.signature(),
        &Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: expected.clone()
            }],
            result: Box::new(expected.clone()),
        }
    );
    assert_eq!(full.rebind(&indexed).unwrap(), expected);
    let partial = signature
        .prepare_function_inputs_with_type_view(&[inputs[0].clone(), None, None], view)
        .unwrap();
    let Term::Function { parameters, .. } = partial.signature() else {
        panic!()
    };
    assert_eq!(
        parameters[0],
        Parameter {
            sort: Sort::Type,
            ty: primitive(Primitive::Type)
        }
    );
    let split = partial
        .signature()
        .prepare_function_inputs_with_type_view(&[inputs[1].clone(), None], view)
        .unwrap();
    assert_eq!(split.signature(), full.signature());
    assert_eq!(
        split.rebind(&partial.rebind(&indexed).unwrap()).unwrap(),
        full.rebind(&indexed).unwrap()
    );
    assert!(full.to_string().contains("(type-slots 1)"));
    assert!(full.to_string().contains("(normal-form"));
}

#[test]
fn function_input_domain_views_reject_invalid_sorts_scopes_and_propagate_failure() {
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::ComptimeValue,
            ty: primitive(Primitive::Bool),
        }],
        result: Box::new(primitive(Primitive::Bool)),
    };
    let inputs = [Some(Term::Bool(true))];
    assert!(
        signature
            .prepare_function_inputs_with_type_view(&inputs, |_, _| Ok::<_, TypeError>(Term::Bool(
                true
            )))
            .is_err()
    );
    assert!(
        signature
            .prepare_function_inputs_with_type_view(&inputs, |_, _| Ok::<_, TypeError>(bound(
                0,
                0,
                Sort::Type
            )))
            .is_err()
    );
    assert!(
        signature
            .prepare_function_inputs_with_type_view(&inputs, |_, _| Err::<Term, _>(
                TypeError::InvalidFunctionSignature
            ))
            .is_err()
    );
    assert!(
        signature
            .prepare_function_inputs_with_type_view(
                &[Some(primitive(Primitive::Char))],
                |ty, _| Ok::<_, TypeError>(ty.clone())
            )
            .is_err()
    );
    assert!(signature.prepare_function_inputs(&inputs).is_ok());
    let opaque_domain = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::ComptimeValue,
            ty: Term::NormalForm {
                head: IDENTITY,
                sort: Sort::Type,
                arguments: vec![primitive(Primitive::Type)],
            },
        }],
        result: Box::new(primitive(Primitive::Bool)),
    };
    assert!(
        opaque_domain
            .prepare_function_inputs_with_type_view(
                &[Some(primitive(Primitive::Char))],
                |ty, _| Ok::<_, TypeError>(ty.clone())
            )
            .is_err(),
        "opaque Type-result heads are not the Type domain"
    );
}
fn calibration() -> Term {
    Term::Nominal {
        declaration: CALIBRATION,
        arguments: vec![],
    }
}
fn sensor_core(interner: &mut TypeInterner) -> Arc<TypeCore> {
    interner
        .intern(
            vec![Parameter {
                sort: Sort::Runtime,
                ty: calibration(),
            }],
            Term::Nominal {
                declaration: SENSOR,
                arguments: vec![hole(0, Sort::Runtime)],
            },
        )
        .unwrap()
}

#[test]
fn shared_core_does_not_merge_runtime_identities() {
    let mut interner = TypeInterner::default();
    let first_core = sensor_core(&mut interner);
    let second_core = sensor_core(&mut interner);
    assert!(Arc::ptr_eq(&first_core, &second_core));
    assert_eq!(interner.len(), 1);
    let first = TypeRef::bind(first_core, vec![object(1)]).unwrap();
    let second = TypeRef::bind(second_core, vec![object(2)]).unwrap();
    let refs: HashSet<_> = [first.clone(), second.clone()].into_iter().collect();
    assert_eq!(refs.len(), 2);
    assert_ne!(first.materialize().unwrap(), second.materialize().unwrap());
    assert_eq!(first.arguments(), &[object(1)]);
    assert_eq!(second.arguments(), &[object(2)]);
    // Runtime object identity is fixed; a surrounding substitution cannot
    // silently retarget either reference to a different object.
    assert_eq!(first.substitute(&[object(99)]).unwrap(), first);
}

#[test]
fn repeated_holes_keep_alias_relationships_and_availability_is_separate() {
    let mut interner = TypeInterner::default();
    let core = interner
        .intern(
            vec![Parameter {
                sort: Sort::Runtime,
                ty: calibration(),
            }],
            Term::Nominal {
                declaration: SENSOR,
                arguments: vec![hole(0, Sort::Runtime), hole(0, Sort::Runtime)],
            },
        )
        .unwrap();
    let ty = TypeRef::bind(core, vec![object(8)]).unwrap();
    let id = RuntimeId {
        instance: 8,
        local: 0,
    };
    assert_eq!(ty.runtime_dependencies().unwrap(), BTreeSet::from([id]));
    assert_eq!(
        ty.check_runtime_bindings(&BTreeSet::new()),
        Err(TypeError::UnboundRuntime(id))
    );
    assert!(ty.check_runtime_bindings(&BTreeSet::from([id])).is_ok());
    assert_eq!(
        ty.materialize().unwrap(),
        Term::Nominal {
            declaration: SENSOR,
            arguments: vec![object(8), object(8)],
        }
    );
}

#[test]
fn substitutions_compose_without_mutating_the_core_or_previous_reference() {
    let mut interner = TypeInterner::default();
    let core = sensor_core(&mut interner);
    let template = TypeRef::bind(core.clone(), vec![bound(0, 0, Sort::Runtime)]).unwrap();
    let a = template.substitute(&[object(41)]).unwrap();
    let b = template.substitute(&[object(42)]).unwrap();
    assert!(Arc::ptr_eq(a.core(), b.core()));
    assert!(Arc::ptr_eq(a.core(), &core));
    assert_eq!(template.arguments(), &[bound(0, 0, Sort::Runtime)]);
    assert_eq!(a.arguments(), &[object(41)]);
    assert_eq!(b.arguments(), &[object(42)]);
    let before = template.clone();
    assert!(template.substitute(&[primitive(Primitive::I32)]).is_err());
    assert_eq!(
        template, before,
        "failed candidate must not leave a partial assignment"
    );
}

#[test]
fn normal_form_head_survives_instantiation_and_cannot_become_its_argument() {
    let mut interner = TypeInterner::default();
    let core = interner
        .intern(
            vec![Parameter {
                sort: Sort::Type,
                ty: primitive(Primitive::Type),
            }],
            Term::NormalForm {
                head: IDENTITY,
                sort: Sort::Type,
                arguments: vec![hole(0, Sort::Type)],
            },
        )
        .unwrap();
    let i32 = primitive(Primitive::I32);
    let ty = TypeRef::bind(core, vec![i32.clone()]).unwrap();
    let result = ty.materialize().unwrap();
    assert_eq!(
        result,
        Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![i32.clone()]
        }
    );
    assert_ne!(result, i32);
    assert!(
        result
            .to_string()
            .starts_with("(normal-form (head (declaration")
    );
}

#[test]
fn ordered_contracts_see_earlier_substitutions_and_propagate_failure() {
    let mut interner = TypeInterner::default();
    let core = interner
        .intern(
            vec![
                Parameter {
                    sort: Sort::Type,
                    ty: primitive(Primitive::Type),
                },
                Parameter {
                    sort: Sort::Runtime,
                    ty: hole(0, Sort::Type),
                },
            ],
            Term::Nominal {
                declaration: SENSOR,
                arguments: vec![hole(1, Sort::Runtime)],
            },
        )
        .unwrap();
    let ty = TypeRef::bind(core, vec![calibration(), object(7)]).unwrap();
    let mut observed = Vec::new();
    ty.check_with::<()>(|index, argument, expected| {
        observed.push((index, argument.clone(), expected.clone()));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        observed,
        vec![
            (0, calibration(), primitive(Primitive::Type)),
            (1, object(7), calibration()),
        ]
    );
    assert_eq!(
        ty.check_with(|index, _, _| if index == 1 {
            Err("type mismatch")
        } else {
            Ok(())
        }),
        Err(ContractError::Contract("type mismatch"))
    );
}

#[test]
fn malformed_cores_and_references_are_rejected_before_interning() {
    let mut interner = TypeInterner::default();
    let type_parameter = Parameter {
        sort: Sort::Type,
        ty: primitive(Primitive::Type),
    };
    let runtime_parameter = Parameter {
        sort: Sort::Runtime,
        ty: calibration(),
    };
    let invalid = [
        (vec![], object(1)),
        (vec![], Term::Pointer(Box::new(object(1)))),
        (
            vec![],
            Term::Nominal {
                declaration: SENSOR,
                arguments: vec![object(1)],
            },
        ),
        (vec![], hole(0, Sort::Type)),
        (vec![type_parameter.clone()], hole(0, Sort::Runtime)),
        (
            vec![Parameter {
                sort: Sort::Runtime,
                ty: hole(0, Sort::Type),
            }],
            primitive(Primitive::Bool),
        ),
        (vec![], bound(0, 0, Sort::Type)),
        (vec![], Term::Tuple(vec![Term::Bool(true)])),
        (
            vec![],
            Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: bound(0, 0, Sort::Type),
                }],
                result: Box::new(calibration()),
            },
        ),
    ];
    for (holes, body) in invalid {
        assert!(interner.intern(holes, body).is_err());
    }
    assert!(interner.is_empty());
    let core = interner
        .intern(vec![runtime_parameter], calibration())
        .unwrap();
    assert!(TypeRef::bind(core.clone(), vec![]).is_err());
    assert!(TypeRef::bind(core.clone(), vec![Term::Bool(false)]).is_err());
    assert!(TypeRef::bind(core, vec![hole(0, Sort::Runtime)]).is_err());
}

#[test]
fn expansion_under_a_function_binder_does_not_capture_argument_references() {
    let mut interner = TypeInterner::default();
    let core = interner
        .intern(
            vec![Parameter {
                sort: Sort::Type,
                ty: primitive(Primitive::Type),
            }],
            Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Type,
                    ty: primitive(Primitive::Type),
                }],
                result: Box::new(Term::Tuple(vec![
                    bound(0, 0, Sort::Type),
                    hole(0, Sort::Type),
                ])),
            },
        )
        .unwrap();
    let reference = TypeRef::bind(core, vec![bound(0, 0, Sort::Type)]).unwrap();
    let Term::Function { result, .. } = reference.materialize().unwrap() else {
        panic!()
    };
    assert_eq!(
        *result,
        Term::Tuple(vec![bound(0, 0, Sort::Type), bound(1, 0, Sort::Type)])
    );
    let concrete = reference.substitute(&[primitive(Primitive::U32)]).unwrap();
    let Term::Function { result, .. } = concrete.materialize().unwrap() else {
        panic!()
    };
    assert_eq!(
        *result,
        Term::Tuple(vec![bound(0, 0, Sort::Type), primitive(Primitive::U32)])
    );
}

#[test]
fn substitution_shifts_outer_frames_and_preserves_rigid_parameters() {
    let rigid = Term::Rigid {
        declaration: IDENTITY,
        sort: Sort::Type,
    };
    let term = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: primitive(Primitive::Type),
        }],
        result: Box::new(Term::Tuple(vec![
            bound(0, 0, Sort::Type),
            bound(1, 0, Sort::Type),
            bound(2, 3, Sort::Type),
            rigid.clone(),
        ])),
    };
    let result = term.substitute(&[bound(0, 5, Sort::Type)]).unwrap();
    let Term::Function { result, .. } = result else {
        panic!()
    };
    assert_eq!(
        *result,
        Term::Tuple(vec![
            bound(0, 0, Sort::Type),
            bound(1, 5, Sort::Type),
            bound(1, 3, Sort::Type),
            rigid,
        ])
    );
    assert!(term.substitute(&[]).is_err());
    assert_eq!(
        term.substitute(&[bound(u32::MAX, 0, Sort::Type)]),
        Err(TypeError::BinderDepthOverflow)
    );
}

#[test]
fn dependent_function_parameter_types_observe_only_earlier_parameters() {
    let mut interner = TypeInterner::default();
    let parameters = vec![
        Parameter {
            sort: Sort::Type,
            ty: primitive(Primitive::Type),
        },
        Parameter {
            sort: Sort::Runtime,
            ty: bound(0, 0, Sort::Type),
        },
    ];
    let body = Term::Function {
        parameters: parameters.clone(),
        result: Box::new(bound(0, 0, Sort::Type)),
    };
    assert!(interner.intern(vec![], body).is_ok());
    let invalid = Term::Function {
        parameters,
        result: Box::new(bound(0, 1, Sort::Type)),
    };
    assert!(matches!(
        interner.intern(vec![], invalid),
        Err(TypeError::WrongSort { .. })
    ));
}

#[test]
fn shared_cores_outlive_the_interner_without_borrowing_another_arena() {
    let reference = {
        let mut a = TypeInterner::default();
        TypeRef::bind(sensor_core(&mut a), vec![object(7)]).unwrap()
    };
    assert_eq!(
        reference.materialize().unwrap(),
        Term::Nominal {
            declaration: SENSOR,
            arguments: vec![object(7)]
        }
    );
}

#[test]
fn structural_inequality_is_not_exposed_as_a_semantic_inequality_proof() {
    let mut first = TypeInterner::default();
    let mut second = TypeInterner::default();
    let a = sensor_core(&mut first);
    let b = sensor_core(&mut second);
    assert!(!Arc::ptr_eq(&a, &b));
    assert_eq!(
        a, b,
        "structural equality is independent of the allocating interner"
    );
    // The representation exposes no solver verdict: differing shapes require
    // the later semantic layer, rather than treating intern IDs as proofs.
}

#[test]
fn dumps_are_deterministic_complete_s_expressions() {
    let mut interner = TypeInterner::default();
    let core = sensor_core(&mut interner);
    let reference = TypeRef::bind(core, vec![object(4)]).unwrap();
    let expected = "(type-ref (core (type-core (holes ((parameter (sort runtime) (type (nominal (head (declaration (package 1) (local 11))) (arguments ())))))) (body (nominal (head (declaration (package 1) (local 10))) (arguments ((hole (index 0) (sort runtime)))))))) (substitution ((runtime (instance 4) (local 0)))))";
    assert_eq!(reference.to_string(), expected);
    assert_eq!(reference.to_string(), reference.clone().to_string());
    for term in [
        primitive(Primitive::Type),
        primitive(Primitive::F64),
        Term::Bool(true),
        Term::Value { store: 3, index: 9 },
        Term::Pointer(Box::new(calibration())),
        Term::Rigid {
            declaration: IDENTITY,
            sort: Sort::Type,
        },
        bound(2, 3, Sort::Runtime),
        hole(1, Sort::ComptimeValue),
        Term::Function {
            parameters: vec![],
            result: Box::new(primitive(Primitive::Unit)),
        },
        Term::NormalForm {
            head: IDENTITY,
            sort: Sort::ComptimeValue,
            arguments: vec![Term::Bool(false)],
        },
    ] {
        let dump = term.to_string();
        let mut depth = 0;
        for c in dump.chars() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0, "{dump}");
        }
        assert_eq!(depth, 0, "{dump}");
        assert!(dump.starts_with('(') && dump.ends_with(')'));
    }
}

#[test]
fn environment_checks_include_substitutions_absent_from_the_body() {
    let mut interner = TypeInterner::default();
    let core = interner
        .intern(
            vec![Parameter {
                sort: Sort::Runtime,
                ty: calibration(),
            }],
            primitive(Primitive::Bool),
        )
        .unwrap();
    let reference = TypeRef::bind(core, vec![object(81)]).unwrap();
    assert_eq!(reference.materialize().unwrap(), primitive(Primitive::Bool));
    let id = RuntimeId {
        instance: 81,
        local: 0,
    };
    assert_eq!(
        reference.runtime_dependencies().unwrap(),
        BTreeSet::from([id])
    );
    assert_eq!(
        reference.check_runtime_bindings(&BTreeSet::new()),
        Err(TypeError::UnboundRuntime(id))
    );
}

#[test]
fn use_site_scope_checks_depth_index_and_sort_without_rebinding() {
    let mut interner = TypeInterner::default();
    let reference =
        TypeRef::bind(sensor_core(&mut interner), vec![bound(1, 0, Sort::Runtime)]).unwrap();
    assert!(
        reference
            .check_scope(&[vec![Sort::Runtime], vec![Sort::Type]])
            .is_ok()
    );
    assert!(matches!(
        reference.check_scope(&[vec![Sort::Runtime]]),
        Err(TypeError::InvalidBound { depth: 1, index: 0 })
    ));
    assert!(matches!(
        reference.check_scope(&[vec![], vec![Sort::Runtime]]),
        Err(TypeError::InvalidBound { .. })
    ));
    assert!(matches!(
        reference.check_scope(&[vec![Sort::Type], vec![]]),
        Err(TypeError::WrongSort { .. })
    ));
    assert_eq!(reference.arguments(), &[bound(1, 0, Sort::Runtime)]);
}

#[test]
fn scope_and_runtime_checks_descend_into_function_argument_contracts() {
    let mut interner = TypeInterner::default();
    let core = interner
        .intern(
            vec![Parameter {
                sort: Sort::Type,
                ty: primitive(Primitive::Type),
            }],
            hole(0, Sort::Type),
        )
        .unwrap();
    let argument = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: Term::Nominal {
                declaration: SENSOR,
                arguments: vec![object(92), bound(1, 0, Sort::Runtime)],
            },
        }],
        result: Box::new(Term::Nominal {
            declaration: SENSOR,
            arguments: vec![bound(0, 0, Sort::Runtime)],
        }),
    };
    let reference = TypeRef::bind(core, vec![argument]).unwrap();
    assert!(reference.check_scope(&[vec![Sort::Runtime]]).is_ok());
    assert!(matches!(
        reference.check_scope(&[]),
        Err(TypeError::InvalidBound { depth: 1, .. })
    ));
    assert_eq!(
        reference.runtime_dependencies().unwrap(),
        BTreeSet::from([RuntimeId {
            instance: 92,
            local: 0
        }])
    );
}

#[test]
fn abstracting_runtime_objects_shares_structure_and_preserves_aliases() {
    let mut interner = TypeInterner::default();
    let make = |instance| {
        let id = RuntimeId { instance, local: 0 };
        (
            Term::Nominal {
                declaration: SENSOR,
                arguments: vec![Term::Runtime(id), Term::Runtime(id)],
            },
            id,
        )
    };
    let (first_body, first_id) = make(10);
    let (second_body, second_id) = make(20);
    let first = interner
        .abstract_runtime(&first_body, &[(first_id, calibration())])
        .unwrap();
    let second = interner
        .abstract_runtime(&second_body, &[(second_id, calibration())])
        .unwrap();
    assert!(Arc::ptr_eq(first.core(), second.core()));
    assert_eq!(first.materialize().unwrap(), first_body);
    assert_eq!(second.materialize().unwrap(), second_body);
    assert_ne!(first, second);
    assert_eq!(interner.len(), 1);
}

#[test]
fn abstraction_checks_ordered_contracts_and_does_not_intern_failures() {
    let mut interner = TypeInterner::default();
    let first = RuntimeId {
        instance: 1,
        local: 0,
    };
    let second = RuntimeId {
        instance: 1,
        local: 1,
    };
    let dependent = Term::Nominal {
        declaration: SENSOR,
        arguments: vec![Term::Runtime(first)],
    };
    let body = Term::Nominal {
        declaration: SENSOR,
        arguments: vec![Term::Runtime(second)],
    };
    assert!(matches!(
        interner.abstract_runtime(
            &body,
            &[(second, dependent.clone()), (first, calibration())]
        ),
        Err(TypeError::InvalidHole(1))
    ));
    assert!(interner.is_empty());
    assert!(
        matches!(interner.abstract_runtime(&body, &[(first, calibration())]), Err(TypeError::RuntimeInCore(id)) if id == second)
    );
    assert!(interner.is_empty());
    assert!(
        matches!(interner.abstract_runtime(&body, &[(first, calibration()), (first, calibration())]), Err(TypeError::DuplicateRuntime(id)) if id == first)
    );
    assert!(interner.is_empty());
    let reference = interner
        .abstract_runtime(
            &body,
            &[(first, calibration()), (second, dependent.clone())],
        )
        .unwrap();
    let mut observed = Vec::new();
    reference
        .check_with(|_, _, expected| {
            observed.push(expected.clone());
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(observed, vec![calibration(), dependent]);
    assert_eq!(reference.materialize().unwrap(), body);
}

#[test]
fn runtime_abstraction_rejects_holes_owned_by_another_template() {
    let mut interner = TypeInterner::default();
    let id = RuntimeId {
        instance: 3,
        local: 0,
    };
    let body = Term::Nominal {
        declaration: SENSOR,
        arguments: vec![hole(0, Sort::Runtime)],
    };
    assert_eq!(
        interner.abstract_runtime(&body, &[(id, calibration())]),
        Err(TypeError::InvalidHole(0))
    );
    assert!(interner.is_empty());
}

#[test]
fn closure_environment_dependencies_survive_abstraction_and_substitution() {
    let mut interner = TypeInterner::default();
    let id = RuntimeId {
        instance: 400,
        local: 0,
    };
    let ty = Term::Closure {
        environment: vec![Term::Nominal {
            declaration: SENSOR,
            arguments: vec![Term::Runtime(id)],
        }],
        signature: Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(primitive(Primitive::Bool)),
        }),
    };
    let reference = interner
        .abstract_runtime(&ty, &[(id, calibration())])
        .unwrap();
    assert_eq!(reference.materialize().unwrap(), ty);
    assert_eq!(
        reference.runtime_dependencies().unwrap(),
        BTreeSet::from([id])
    );
    assert!(reference.to_string().contains("(closure (environment"));
    assert!(matches!(
        interner.intern(
            vec![],
            Term::Closure {
                environment: vec![],
                signature: Box::new(primitive(Primitive::Bool))
            }
        ),
        Err(TypeError::InvalidClosureSignature)
    ));
}

#[test]
fn type_parameter_instantiation_preserves_binders_and_opaque_heads() {
    let rigid = Term::Rigid {
        declaration: SENSOR,
        sort: Sort::Type,
    };
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: rigid.clone(),
        }],
        result: Box::new(Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![rigid, bound(0, 0, Sort::Runtime)],
        }),
    };
    let bound_signature = signature.abstract_types(&[SENSOR]).unwrap();
    bound_signature.check_scope(&[vec![Sort::Type]]).unwrap();
    let instantiated = signature
        .instantiate_types(&[SENSOR], &[primitive(Primitive::Bool)])
        .unwrap();
    instantiated.check_scope(&[]).unwrap();
    let Term::Function { result, .. } = instantiated else {
        panic!()
    };
    assert_eq!(
        *result,
        Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![primitive(Primitive::Bool), bound(0, 0, Sort::Runtime)]
        }
    );
    assert!(signature.instantiate_types(&[SENSOR], &[]).is_err());
    assert!(
        signature
            .instantiate_types(&[SENSOR], &[Term::Bool(true)])
            .is_err()
    );
    assert!(
        signature
            .instantiate_types(&[SENSOR], &[bound(0, 0, Sort::Type)])
            .is_err()
    );
}

#[test]
fn partial_substitution_reads_only_referenced_slots_and_preserves_scope() {
    let template = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: bound(1, 1, Sort::Type),
        }],
        result: Box::new(bound(1, 1, Sort::Type)),
    };
    let args = [None, Some(primitive(Primitive::Bool)), None];
    let result = template.substitute_partial(&args).unwrap();
    result.check_scope(&[]).unwrap();
    assert!(template.substitute_partial(&[None, None]).is_err());
    assert!(
        template
            .substitute_partial(&[None, Some(Term::Bool(true))])
            .is_err()
    );
    assert_eq!(
        bound(1, 0, Sort::Type).substitute_partial(&args).unwrap(),
        bound(0, 0, Sort::Type)
    );
}

#[test]
fn unrestricted_scalar_representations_do_not_imply_general_cast_permission() {
    assert_eq!(Primitive::F32.unrestricted_scalar_width(), Some(32));
    assert_eq!(Primitive::F64.unrestricted_scalar_width(), Some(64));
    assert_eq!(Primitive::I128.unrestricted_scalar_width(), Some(128));
    assert_eq!(Primitive::U8.unrestricted_scalar_width(), Some(8));
    for primitive in [
        Primitive::Bool,
        Primitive::Char,
        Primitive::Str,
        Primitive::Isize,
        Primitive::Usize,
        Primitive::Integer,
        Primitive::Float,
    ] {
        assert_eq!(primitive.unrestricted_scalar_width(), None);
    }
    assert_eq!(Primitive::F32.fixed_integer_layout(), None);
    assert_eq!(Primitive::F64.fixed_integer_layout(), None);
}

#[test]
fn type_value_instantiation_rewrites_payload_without_unfolding_heads() {
    use nir::program::{Expr, ExprKind, SourceRange};
    let parameter = DeclarationId {
        package: 3,
        local: 41,
    };
    let template = Expr {
        ty: primitive(Primitive::Type),
        source: SourceRange { start: 1, end: 4 },
        kind: ExprKind::TypeValue(Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![Term::Rigid {
                declaration: parameter,
                sort: Sort::Type,
            }],
        }),
    };
    let instance = template
        .instantiate_types(&[parameter], &[primitive(Primitive::U8)])
        .unwrap();
    assert_eq!(
        instance.kind,
        ExprKind::TypeValue(Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![primitive(Primitive::U8)]
        })
    );
    assert!(instance.to_string().contains("(type-value (normal-form"));
}

#[test]
fn exact_integer_arguments_have_value_identity_and_survive_type_substitution() {
    fn integer(text: &str) -> Term {
        Term::Integer(
            comptime_runtime::Integer::parse_literal(
                text,
                &mut comptime_runtime::Budget::default(),
            )
            .unwrap(),
        )
    }
    let decimal = integer("18446744073709551616");
    let hexadecimal = integer("0x10000000000000000");
    assert_eq!(decimal.sort(), Sort::ComptimeValue);
    assert_eq!(HashSet::from([decimal.clone(), hexadecimal]).len(), 1);
    let ty = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: vec![
            Term::Rigid {
                declaration: SENSOR,
                sort: Sort::Type,
            },
            decimal.clone(),
        ],
    };
    let replaced = ty
        .instantiate_types(&[SENSOR], &[primitive(Primitive::U8)])
        .unwrap();
    assert_eq!(
        replaced,
        Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![primitive(Primitive::U8), decimal]
        }
    );
    assert!(replaced.check_scope(&[]).is_ok());
    assert!(
        replaced
            .to_string()
            .contains("(integer (sign positive) (hex \"10000000000000000\"))")
    );
}

#[test]
fn mixed_declaration_substitution_preserves_heads_sorts_and_scope() {
    use nir::program::TypePredicate;
    let ty = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: vec![
            Term::Rigid {
                declaration: SENSOR,
                sort: Sort::Type,
            },
            Term::Rigid {
                declaration: CALIBRATION,
                sort: Sort::ComptimeValue,
            },
        ],
    };
    let integer = Term::Integer(comptime_runtime::Integer::from_u128(8));
    let inputs = [primitive(Primitive::U8), integer.clone()];
    let expected = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: inputs.to_vec(),
    };
    assert_eq!(
        ty.instantiate_rigids(&[SENSOR, CALIBRATION], &inputs)
            .unwrap(),
        expected
    );
    let predicate = TypePredicate::Not(Box::new(TypePredicate::And(vec![TypePredicate::Equal(
        ty.clone(),
        expected.clone(),
    )])));
    assert_eq!(
        predicate
            .instantiate_rigids(&[SENSOR, CALIBRATION], &inputs)
            .unwrap(),
        TypePredicate::Not(Box::new(TypePredicate::And(vec![TypePredicate::Equal(
            expected.clone(),
            expected
        )])))
    );
    assert!(matches!(
        ty.instantiate_rigids(&[SENSOR], &[integer]),
        Err(TypeError::WrongSort { .. })
    ));
    assert!(matches!(
        ty.instantiate_rigids(&[CALIBRATION], &[primitive(Primitive::U8)]),
        Err(TypeError::WrongSort { .. })
    ));
    assert!(matches!(
        ty.instantiate_rigids(&[SENSOR], &[bound(0, 0, Sort::Type)]),
        Err(TypeError::InvalidBound { .. })
    ));
    assert!(matches!(
        ty.instantiate_rigids(&[SENSOR], &[]),
        Err(TypeError::ArgumentCount { .. })
    ));
    assert_eq!(ty.instantiate_rigids(&[], &[]).unwrap(), ty);
}

#[test]
fn trivial_type_contract_proofs_do_not_invent_opaque_disequalities() {
    use nir::program::TypePredicate as Predicate;
    let same = Predicate::Equal(calibration(), calibration());
    let unknown = Predicate::Equal(calibration(), primitive(Primitive::U8));
    assert_eq!(same.trivial_truth(), Some(true));
    assert_eq!(unknown.trivial_truth(), None);
    assert_eq!(
        Predicate::Not(Box::new(unknown.clone())).trivial_truth(),
        None
    );
    assert_eq!(
        Predicate::And(vec![same.clone(), unknown.clone()]).trivial_truth(),
        None
    );
    assert_eq!(
        Predicate::Or(vec![same, unknown.clone()]).trivial_truth(),
        Some(true)
    );
    assert_eq!(
        Predicate::And(vec![unknown, Predicate::Bool(false)]).trivial_truth(),
        Some(false)
    );
    assert_eq!(Predicate::And(vec![]).trivial_truth(), Some(true));
    assert_eq!(Predicate::Or(vec![]).trivial_truth(), Some(false));
    let mut deep = Predicate::Bool(true);
    for _ in 0..256 {
        deep = Predicate::Not(Box::new(deep));
    }
    assert_eq!(deep.trivial_truth(), None);
}

#[test]
fn text_value_identity_uses_contents_and_escaped_sexpressions() {
    let first = Term::String(Arc::from(String::from("name\"\\\n\tλ")));
    let second = Term::String(Arc::from(String::from("name\"\\\n\tλ")));
    assert_eq!(first.sort(), Sort::ComptimeValue);
    assert_eq!(HashSet::from([first.clone(), second]).len(), 1);
    assert_eq!(first.to_string(), "(string \"name\\\"\\\\\\n\\tλ\")");
    assert_eq!(Term::Char('λ').sort(), Sort::ComptimeValue);
    assert_eq!(Term::Char('λ').to_string(), "(char 955)");
    let application = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: vec![
            first.clone(),
            Term::Rigid {
                declaration: SENSOR,
                sort: Sort::Type,
            },
        ],
    };
    assert_eq!(
        application
            .instantiate_types(&[SENSOR], &[primitive(Primitive::U8)])
            .unwrap(),
        Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![first, primitive(Primitive::U8)]
        }
    );
}

#[test]
fn tuple_values_keep_sort_substitution_scope_and_escaped_structure() {
    let value = Term::TupleValue(vec![
        Term::Rigid {
            declaration: SENSOR,
            sort: Sort::Type,
        },
        Term::TupleValue(vec![Term::String("\"\nλ".into()), Term::Bool(true)]),
    ]);
    assert_eq!(value.sort(), Sort::ComptimeValue);
    let closed = value
        .instantiate_types(&[SENSOR], &[primitive(Primitive::U8)])
        .unwrap();
    assert_eq!(
        closed,
        Term::TupleValue(vec![
            primitive(Primitive::U8),
            Term::TupleValue(vec![Term::String("\"\nλ".into()), Term::Bool(true)])
        ])
    );
    assert!(closed.check_scope(&[]).is_ok());
    assert_ne!(Term::TupleValue(vec![]), Term::Tuple(vec![]));
    assert!(
        closed
            .to_string()
            .contains("(tuple-value ((string \"\\\"\\nλ\")")
    );
    assert!(Term::TupleValue(vec![object(1)]).check_scope(&[]).is_err());
    assert!(
        Term::TupleValue(vec![bound(0, 0, Sort::Type)])
            .check_scope(&[])
            .is_err()
    );
    assert!(
        Term::TupleValue(vec![bound(0, 0, Sort::Type)])
            .check_scope(&[vec![Sort::Type]])
            .is_ok()
    );
}

#[test]
fn bitvec_identity_includes_width_and_is_not_integer_identity() {
    let bits = |width| {
        Term::BitVec(
            comptime_runtime::Integer::from_u128(7)
                .to_bitvec(width, &mut comptime_runtime::Budget::default())
                .unwrap(),
        )
    };
    assert_eq!(bits(8).sort(), Sort::ComptimeValue);
    assert_eq!(HashSet::from([bits(8), bits(16), bits(8)]).len(), 2);
    assert_ne!(
        bits(8),
        Term::Integer(comptime_runtime::Integer::from_u128(7))
    );
    assert!(bits(8).check_scope(&[]).is_ok());
    assert!(bits(8).to_string().starts_with("(bitvec"));
}

#[test]
fn unit_value_is_a_closed_comptime_value_not_a_type_or_tuple() {
    assert_eq!(Term::Unit.sort(), Sort::ComptimeValue);
    assert_ne!(Term::Unit, primitive(Primitive::Void));
    assert_ne!(Term::Unit, Term::TupleValue(vec![]));
    assert_eq!(Term::Unit.to_string(), "(unit-value)");
    assert!(Term::Unit.check_scope(&[]).is_ok());
    assert_eq!(
        Term::TupleValue(vec![Term::Unit])
            .instantiate_types(&[], &[])
            .unwrap(),
        Term::TupleValue(vec![Term::Unit])
    );
}

#[test]
fn comptime_callable_substitution_keeps_phase_binder_and_display() {
    let generic = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: bound(1, 0, Sort::Type),
        }],
        result: Box::new(bound(1, 0, Sort::Type)),
    }
    .with_comptime_phase(true);
    assert_eq!(generic.check_scope(&[vec![Sort::Type]]), Ok(()));
    let specialized = generic.substitute(&[primitive(Primitive::U8)]).unwrap();
    assert_eq!(specialized.check_scope(&[]), Ok(()));
    assert_ne!(&specialized, specialized.callable_signature());
    assert!(
        specialized
            .to_string()
            .starts_with("(comptime-function (function ")
    );
    assert!(
        Term::ComptimeFunction(Box::new(primitive(Primitive::U8)))
            .check_scope(&[])
            .is_err()
    );
}

#[test]
fn closure_capture_initializers_are_specialized_without_mutating_the_template() {
    use nir::program::{Capture, Expr, ExprKind, LocalId, SourceRange};
    let variable = Term::Rigid {
        declaration: SENSOR,
        sort: Sort::Type,
    };
    let binding = LocalId { owner: 3, index: 4 };
    let source = SourceRange { start: 0, end: 1 };
    let type_ty = primitive(Primitive::Type);
    let expr = Expr {
        ty: Term::Closure {
            environment: vec![type_ty.clone()],
            signature: Box::new(
                Term::Function {
                    parameters: vec![],
                    result: Box::new(type_ty.clone()),
                }
                .with_comptime_phase(true),
            ),
        },
        source,
        kind: ExprKind::Closure {
            parameters: vec![],
            captures: vec![Capture {
                binding,
                ty: type_ty.clone(),
                value: Some(Box::new(Expr {
                    source,
                    ty: type_ty.clone(),
                    kind: ExprKind::TypeValue(variable.clone()),
                })),
            }],
            body: Arc::new(Expr {
                source,
                ty: type_ty,
                kind: ExprKind::Local(binding),
            }),
        },
    };
    assert_eq!(expr.ty.check_scope(&[]), Ok(()));
    let specialized = expr
        .instantiate_types(&[SENSOR], &[primitive(Primitive::Bool)])
        .unwrap();
    let value = |expr: &Expr| {
        let ExprKind::Closure { captures, .. } = &expr.kind else {
            panic!("closure")
        };
        let ExprKind::TypeValue(value) = &captures[0].value.as_ref().unwrap().kind else {
            panic!("type")
        };
        value.clone()
    };
    assert_eq!(value(&specialized), primitive(Primitive::Bool));
    assert_eq!(value(&expr), variable);
    assert!(specialized.ty.is_comptime_callable());
    assert!(specialized.to_string().contains("(value (expr"));
}

#[test]
fn replacing_callable_results_preserves_environment_phase_and_parameter_binders() {
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: Term::Primitive(Primitive::Bool),
        }],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    for comptime in [false, true] {
        let signature = signature.clone().with_comptime_phase(comptime);
        let original = Term::Closure {
            environment: vec![
                Term::Primitive(Primitive::Bool),
                Term::Primitive(Primitive::Char),
            ],
            signature: Box::new(signature.clone()),
        };
        let changed = original
            .clone()
            .with_callable_result(Term::Primitive(Primitive::Char))
            .unwrap();
        let Term::Closure {
            environment,
            signature: changed_signature,
        } = &changed
        else {
            panic!("lost environment")
        };
        assert_eq!(
            environment,
            &[
                Term::Primitive(Primitive::Bool),
                Term::Primitive(Primitive::Char)
            ]
        );
        assert_eq!(changed.is_comptime_callable(), comptime);
        let Term::Function { parameters, result } = changed_signature.callable_signature() else {
            panic!("lost function")
        };
        assert_eq!(
            parameters,
            &[Parameter {
                sort: Sort::Runtime,
                ty: Term::Primitive(Primitive::Bool)
            }]
        );
        assert_eq!(result.as_ref(), &Term::Primitive(Primitive::Char));
        assert_eq!(
            original,
            Term::Closure {
                environment: environment.clone(),
                signature: Box::new(signature)
            }
        );
    }
    assert!(
        Term::Primitive(Primitive::Bool)
            .with_callable_result(Term::Primitive(Primitive::Char))
            .is_none()
    );
}

#[test]
fn record_types_validate_fields_before_interning() {
    let mut interner = TypeInterner::default();
    let record = Term::Record(vec![
        ("enabled".into(), primitive(Primitive::Bool)),
        ("count".into(), primitive(Primitive::U8)),
    ]);
    let core = interner.intern(vec![], record.clone()).unwrap();
    assert!(Arc::ptr_eq(
        &core,
        &interner.intern(vec![], record.clone()).unwrap()
    ));
    assert_eq!(core.body(), &record);
    assert_ne!(
        record,
        Term::Tuple(vec![primitive(Primitive::Bool), primitive(Primitive::U8)])
    );
    assert_eq!(
        interner
            .intern(
                vec![],
                Term::Record(vec![
                    ("x".into(), primitive(Primitive::Bool)),
                    ("x".into(), primitive(Primitive::U8))
                ])
            )
            .unwrap_err(),
        TypeError::DuplicateRecordField("x".into())
    );
    assert!(matches!(
        interner.intern(vec![], Term::Record(vec![("bad".into(), Term::Bool(true))])),
        Err(TypeError::WrongSort { .. })
    ));
    assert_eq!(interner.len(), 1, "invalid records must not publish cores");
}

#[test]
fn record_type_substitution_reaches_fields_and_respects_nested_binders() {
    let term = Term::Record(vec![
        ("value".into(), bound(0, 0, Sort::Type)),
        (
            "callback".into(),
            Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Type,
                    ty: primitive(Primitive::Type),
                }],
                result: Box::new(Term::Tuple(vec![
                    bound(1, 0, Sort::Type),
                    bound(0, 0, Sort::Type),
                ])),
            },
        ),
    ]);
    let actual = term.substitute(&[primitive(Primitive::U8)]).unwrap();
    let expected = Term::Record(vec![
        ("value".into(), primitive(Primitive::U8)),
        (
            "callback".into(),
            Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Type,
                    ty: primitive(Primitive::Type),
                }],
                result: Box::new(Term::Tuple(vec![
                    primitive(Primitive::U8),
                    bound(0, 0, Sort::Type),
                ])),
            },
        ),
    ]);
    assert_eq!(actual, expected);
    actual.check_scope(&[]).unwrap();
    let rigid = Term::Rigid {
        declaration: SENSOR,
        sort: Sort::Type,
    };
    let record = Term::Record(vec![("value".into(), rigid)]);
    assert!(record.mentions_rigid(SENSOR));
    assert_eq!(
        record
            .instantiate_types(&[SENSOR], &[primitive(Primitive::Bool)])
            .unwrap(),
        Term::Record(vec![("value".into(), primitive(Primitive::Bool))])
    );
}

#[test]
fn record_runtime_dependencies_are_abstracted_without_merging_environments() {
    let mut interner = TypeInterner::default();
    let mut refs = Vec::new();
    for instance in [1, 2] {
        let runtime = RuntimeId { instance, local: 0 };
        let ty = Term::Record(vec![(
            "reading".into(),
            Term::NormalForm {
                head: IDENTITY,
                sort: Sort::Type,
                arguments: vec![Term::Runtime(runtime)],
            },
        )]);
        let reference = interner
            .abstract_runtime(&ty, &[(runtime, calibration())])
            .unwrap();
        assert_eq!(reference.materialize().unwrap(), ty);
        assert_eq!(
            reference.runtime_dependencies().unwrap(),
            BTreeSet::from([runtime])
        );
        assert!(reference.check_runtime_bindings(&BTreeSet::new()).is_err());
        refs.push(reference);
    }
    assert_ne!(refs[0], refs[1]);
    assert_eq!(interner.len(), 1);
}

#[test]
fn record_type_dump_escapes_names_and_retains_field_structure() {
    let term = Term::Record(vec![(
        "quote\"\n".into(),
        Term::Record(vec![("\\tail".into(), primitive(Primitive::Bool))]),
    )]);
    assert_eq!(
        term.to_string(),
        r#"(record-type (fields (field "quote\"\n" (record-type (fields (field "\\tail" (primitive bool)))))))"#
    );
    println!("RECORD_TYPE_DUMP={term}\nRECORD_TYPE_END");
}

#[test]
fn record_type_arguments_retain_nested_runtime_environment_requirements() {
    let runtime = RuntimeId {
        instance: 9,
        local: 2,
    };
    let argument = Term::Record(vec![(
        "sensor".into(),
        Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![Term::Runtime(runtime)],
        },
    )]);
    let mut interner = TypeInterner::default();
    let core = interner
        .intern(
            vec![Parameter {
                sort: Sort::Type,
                ty: primitive(Primitive::Type),
            }],
            hole(0, Sort::Type),
        )
        .unwrap();
    let reference = TypeRef::bind(core, vec![argument.clone()]).unwrap();
    assert_eq!(
        reference.runtime_dependencies().unwrap(),
        BTreeSet::from([runtime])
    );
    assert_eq!(
        reference.check_runtime_bindings(&BTreeSet::new()),
        Err(TypeError::UnboundRuntime(runtime))
    );
    reference
        .check_runtime_bindings(&BTreeSet::from([runtime]))
        .unwrap();
    assert_eq!(reference.materialize().unwrap(), argument);
}

#[test]
fn trait_predicates_preserve_identity_boolean_structure_and_substitution() {
    use nir::program::{TraitRequirement, TypePredicate as Q};
    let parameter = DeclarationId {
        package: 3,
        local: 1,
    };
    let interface = DeclarationId {
        package: 4,
        local: 9,
    };
    let rigid = Term::Rigid {
        declaration: parameter,
        sort: Sort::Type,
    };
    let atom = Q::Trait(TraitRequirement {
        interface,
        subject: rigid.clone(),
        interface_arguments: vec![Term::Pointer(Box::new(rigid))],
    });
    let predicate = Q::Not(Box::new(Q::Or(vec![atom.clone(), Q::Bool(false)])));
    assert!(predicate.mentions_rigid(parameter));
    assert_eq!(predicate.trivial_truth(), None);
    let boolean = Term::Primitive(Primitive::Bool);
    let expected = Q::Not(Box::new(Q::Or(vec![
        Q::Trait(TraitRequirement {
            interface,
            subject: boolean.clone(),
            interface_arguments: vec![Term::Pointer(Box::new(boolean.clone()))],
        }),
        Q::Bool(false),
    ])));
    assert_eq!(
        predicate
            .instantiate_types(&[parameter], &[boolean.clone()])
            .unwrap(),
        expected
    );
    assert_eq!(
        predicate
            .instantiate_rigids(&[parameter], &[boolean])
            .unwrap(),
        expected
    );
    assert!(!expected.mentions_rigid(parameter));
    let dump = expected.to_string();
    assert!(dump.contains("(trait-proposition"));
    assert!(dump.starts_with("(not (or "));
}

#[test]
fn mixed_parameter_abstraction_round_trips_under_function_binders() {
    use nir::DeclarationParameter as D;
    let runtime = DeclarationId {
        package: 9,
        local: 3,
    };
    let inputs = [
        D {
            declaration: SENSOR,
            sort: Sort::Type,
        },
        D {
            declaration: CALIBRATION,
            sort: Sort::ComptimeValue,
        },
        D {
            declaration: runtime,
            sort: Sort::Runtime,
        },
    ];
    let template = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: Term::Rigid {
                declaration: SENSOR,
                sort: Sort::Type,
            },
        }],
        result: Box::new(Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![
                Term::Rigid {
                    declaration: SENSOR,
                    sort: Sort::Type,
                },
                Term::Rigid {
                    declaration: CALIBRATION,
                    sort: Sort::ComptimeValue,
                },
                Term::Rigid {
                    declaration: runtime,
                    sort: Sort::Runtime,
                },
                bound(0, 0, Sort::Runtime),
            ],
        }),
    };
    let abstracted = template.abstract_parameters(&inputs).unwrap();
    abstracted
        .check_scope(&[vec![Sort::Type, Sort::ComptimeValue, Sort::Runtime]])
        .unwrap();
    let arguments = [
        primitive(Primitive::U8),
        Term::Integer(comptime_runtime::Integer::from_u128(8)),
        object(99),
    ];
    let concrete = template
        .instantiate_parameters(&inputs, &arguments)
        .unwrap();
    assert_eq!(abstracted.substitute(&arguments).unwrap(), concrete);
    let Term::Function { parameters, result } = concrete else {
        panic!("function")
    };
    assert_eq!(parameters[0].ty, primitive(Primitive::U8));
    assert_eq!(
        *result,
        Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: arguments
                .into_iter()
                .chain([bound(0, 0, Sort::Runtime)])
                .collect()
        }
    );
    assert_eq!(
        inputs[1].to_string(),
        format!("(declaration-parameter (identity {CALIBRATION}) (sort comptime-value))")
    );
}

#[test]
fn mixed_parameter_abstraction_preserves_existing_outer_frames() {
    use nir::DeclarationParameter;
    let parameters = [DeclarationParameter {
        declaration: SENSOR,
        sort: Sort::Type,
    }];
    let template = Term::Tuple(vec![
        bound(0, 0, Sort::Type),
        Term::Rigid {
            declaration: SENSOR,
            sort: Sort::Type,
        },
    ]);
    let abstracted = template.abstract_parameters(&parameters).unwrap();
    assert_eq!(
        abstracted,
        Term::Tuple(vec![bound(1, 0, Sort::Type), bound(0, 0, Sort::Type)])
    );
    abstracted
        .check_scope(&[vec![Sort::Type], vec![Sort::Type]])
        .unwrap();
    assert_eq!(
        abstracted
            .substitute(&[primitive(Primitive::Bool)])
            .unwrap(),
        template
            .instantiate_parameters(&parameters, &[primitive(Primitive::Bool)])
            .unwrap()
    );
    assert!(matches!(
        bound(u32::MAX, 0, Sort::Type).abstract_parameters(&parameters),
        Err(TypeError::BinderDepthOverflow)
    ));
}

#[test]
fn declared_input_sorts_are_checked_even_when_unused() {
    use nir::DeclarationParameter as D;
    let parameters = [D {
        declaration: SENSOR,
        sort: Sort::ComptimeValue,
    }];
    let unchanged = primitive(Primitive::Bool);
    assert!(matches!(
        unchanged.instantiate_parameters(&parameters, &[primitive(Primitive::U8)]),
        Err(TypeError::WrongSort {
            expected: Sort::ComptimeValue,
            actual: Sort::Type
        })
    ));
    assert_eq!(
        unchanged
            .instantiate_parameters(&parameters, &[Term::Bool(true)])
            .unwrap(),
        unchanged
    );
    let duplicates = [parameters[0], parameters[0]];
    assert!(matches!(
        unchanged.abstract_parameters(&duplicates),
        Err(TypeError::DuplicateDeclarationParameter(SENSOR))
    ));
    assert!(matches!(
        unchanged.instantiate_parameters(&duplicates, &[Term::Bool(true), Term::Bool(false)]),
        Err(TypeError::DuplicateDeclarationParameter(SENSOR))
    ));
    assert!(matches!(
        unchanged.instantiate_rigids(&[SENSOR, SENSOR], &[Term::Bool(true), Term::Bool(false)]),
        Err(TypeError::DuplicateDeclarationParameter(SENSOR))
    ));
    let malformed = Term::Rigid {
        declaration: SENSOR,
        sort: Sort::Type,
    };
    assert!(matches!(
        malformed.abstract_parameters(&parameters),
        Err(TypeError::WrongSort { .. })
    ));
    assert!(matches!(
        malformed.instantiate_parameters(&parameters, &[Term::Bool(true)]),
        Err(TypeError::WrongSort { .. })
    ));
    assert!(
        unchanged
            .instantiate_parameters(&parameters, &[bound(0, 0, Sort::ComptimeValue)])
            .is_err()
    );
}

#[test]
fn mixed_declaration_inputs_rewrite_nir_type_operands_without_erasing_heads() {
    use nir::DeclarationParameter;
    use nir::program::{Expr, ExprKind, SourceRange};
    let parameters = [DeclarationParameter {
        declaration: CALIBRATION,
        sort: Sort::ComptimeValue,
    }];
    let expr = Expr {
        ty: primitive(Primitive::Type),
        source: SourceRange { start: 0, end: 1 },
        kind: ExprKind::TypeValue(Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![Term::Rigid {
                declaration: CALIBRATION,
                sort: Sort::ComptimeValue,
            }],
        }),
    };
    let result = expr
        .instantiate_parameters(&parameters, &[Term::Bool(true)])
        .unwrap();
    assert_eq!(
        result.kind,
        ExprKind::TypeValue(Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![Term::Bool(true)]
        })
    );
    assert!(
        expr.instantiate_parameters(&parameters, &[primitive(Primitive::Bool)])
            .is_err()
    );
    assert!(
        matches!(expr.kind,ExprKind::TypeValue(Term::NormalForm {arguments,..}) if matches!(arguments[0],Term::Rigid {..}))
    );
}

#[test]
fn type_only_parameter_facade_keeps_outer_scope_and_rejects_duplicate_ids() {
    let ty = Term::Tuple(vec![
        bound(0, 0, Sort::Type),
        Term::Rigid {
            declaration: SENSOR,
            sort: Sort::Type,
        },
    ]);
    let bound_ty = ty.abstract_types(&[SENSOR]).unwrap();
    assert_eq!(
        bound_ty,
        Term::Tuple(vec![bound(1, 0, Sort::Type), bound(0, 0, Sort::Type)])
    );
    assert_eq!(
        bound_ty.substitute(&[primitive(Primitive::U8)]).unwrap(),
        ty.instantiate_types(&[SENSOR], &[primitive(Primitive::U8)])
            .unwrap()
    );
    assert!(matches!(
        ty.instantiate_types(
            &[SENSOR, SENSOR],
            &[primitive(Primitive::U8), primitive(Primitive::U16)]
        ),
        Err(TypeError::DuplicateDeclarationParameter(SENSOR))
    ));
}

#[test]
fn mixed_inputs_specialize_nested_type_contracts_without_unfolding() {
    use nir::program::{InterfaceInstance, TraitRequirement, TypePredicate as P};
    let parameters = [
        DeclarationParameter {
            declaration: SENSOR,
            sort: Sort::Type,
        },
        DeclarationParameter {
            declaration: CALIBRATION,
            sort: Sort::ComptimeValue,
        },
    ];
    let subject = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: vec![
            Term::Rigid {
                declaration: SENSOR,
                sort: Sort::Type,
            },
            Term::Rigid {
                declaration: CALIBRATION,
                sort: Sort::ComptimeValue,
            },
        ],
    };
    let requirement = TraitRequirement {
        interface: SENSOR,
        subject: subject.clone(),
        interface_arguments: vec![subject.clone()],
    };
    let input = P::And(vec![
        P::Trait(requirement.clone()),
        P::Not(Box::new(P::Or(vec![P::Equal(
            subject.clone(),
            primitive(Primitive::Bool),
        )]))),
    ]);
    let args = [primitive(Primitive::U8), Term::Bool(true)];
    let expected = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: args.to_vec(),
    };
    let result = input.instantiate_parameters(&parameters, &args).unwrap();
    assert_eq!(
        result,
        P::And(vec![
            P::Trait(TraitRequirement {
                interface: SENSOR,
                subject: expected.clone(),
                interface_arguments: vec![expected.clone()]
            }),
            P::Not(Box::new(P::Or(vec![P::Equal(
                expected.clone(),
                primitive(Primitive::Bool)
            )]))),
        ])
    );
    assert_eq!(
        requirement
            .interface_instance()
            .instantiate_parameters(&parameters, &args)
            .unwrap(),
        InterfaceInstance {
            declaration: SENSOR,
            arguments: vec![expected]
        }
    );
    assert!(input.mentions_rigid(CALIBRATION));
    assert!(!result.mentions_rigid(CALIBRATION));
}

#[test]
fn empty_contracts_validate_unused_declaration_inputs() {
    use nir::program::{InterfaceInstance, TypePredicate as P};
    let parameters = [DeclarationParameter {
        declaration: SENSOR,
        sort: Sort::ComptimeValue,
    }];
    let empty = InterfaceInstance {
        declaration: IDENTITY,
        arguments: vec![],
    };
    assert!(
        empty
            .instantiate_parameters(&parameters, &[primitive(Primitive::Bool)])
            .is_err()
    );
    assert_eq!(
        empty
            .instantiate_parameters(&parameters, &[Term::Bool(true)])
            .unwrap(),
        empty
    );
    for predicate in [P::Bool(true), P::And(vec![]), P::Or(vec![])] {
        assert!(predicate.instantiate_parameters(&parameters, &[]).is_err());
        assert!(
            predicate
                .instantiate_parameters(&parameters, &[primitive(Primitive::Bool)])
                .is_err()
        );
        assert_eq!(
            predicate
                .instantiate_parameters(&parameters, &[Term::Bool(true)])
                .unwrap(),
            predicate
        );
        assert!(
            predicate
                .instantiate_types(&[SENSOR], &[Term::Bool(true)])
                .is_err()
        );
        assert!(
            predicate
                .instantiate_types(
                    &[SENSOR, SENSOR],
                    &[primitive(Primitive::Bool), primitive(Primitive::Bool)]
                )
                .is_err()
        );
    }
}

#[test]
fn function_specialization_covers_contracts_defaults_and_preserves_local_identity() {
    use nir::program::*;
    let input = DeclarationParameter {
        declaration: CALIBRATION,
        sort: Sort::ComptimeValue,
    };
    let ty = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: vec![Term::Rigid {
            declaration: CALIBRATION,
            sort: Sort::ComptimeValue,
        }],
    };
    let concrete = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: vec![Term::Bool(true)],
    };
    let local = LocalId { owner: 3, index: 1 };
    let expression = Expr {
        source: SourceRange { start: 10, end: 20 },
        ty: ty.clone(),
        kind: ExprKind::Local(local),
    };
    let predicate = Expr {
        source: expression.source,
        ty: primitive(Primitive::Bool),
        kind: ExprKind::TypeEqual(
            Box::new(Expr {
                source: expression.source,
                ty: primitive(Primitive::Type),
                kind: ExprKind::TypeValue(ty.clone()),
            }),
            Box::new(Expr {
                source: expression.source,
                ty: primitive(Primitive::Type),
                kind: ExprKind::TypeValue(ty.clone()),
            }),
        ),
    };
    let template = Function {
        phase: FunctionPhase::Runtime,
        inline: false,
        pure: true,
        is_unsafe: false,
        declaration: IDENTITY,
        name: "template".into(),
        type_parameters: vec![],
        signature: Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: ty.clone(),
            }],
            result: Box::new(ty.clone()),
        },
        parameters: vec![local],
        body: expression.clone(),
        defaults: vec![(0, expression)],
        preconditions: vec![predicate.clone()],
        postconditions: vec![predicate],
        result_binding: Some(LocalId { owner: 3, index: 2 }),
        entry_snapshots: vec![EntrySnapshot {
            parameter: local,
            binding: LocalId { owner: 3, index: 3 },
        }],
        capabilities: vec![TraitRequirement {
            interface: SENSOR,
            subject: ty.clone(),
            interface_arguments: vec![ty.clone()],
        }],
        requirements: vec![TypePredicate::Equal(ty.clone(), ty)],
    };
    let original = template.clone();
    let specialized = template
        .instantiate_parameters(&[input], &[Term::Bool(true)])
        .unwrap();
    assert_eq!(specialized.body.ty, concrete);
    assert_eq!(specialized.defaults[0].1.ty, concrete);
    assert_eq!(specialized.capabilities[0].subject, concrete);
    assert_eq!(
        specialized.capabilities[0].interface_arguments,
        vec![concrete.clone()]
    );
    assert_eq!(
        specialized.requirements,
        vec![TypePredicate::Equal(concrete.clone(), concrete.clone())]
    );
    assert_eq!(
        specialized.signature,
        Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: concrete.clone()
            }],
            result: Box::new(concrete)
        }
    );
    for expression in specialized
        .preconditions
        .iter()
        .chain(&specialized.postconditions)
    {
        let ExprKind::TypeEqual(left, right) = &expression.kind else {
            panic!("predicate lost")
        };
        for operand in [left, right] {
            let ExprKind::TypeValue(ty) = &operand.kind else {
                panic!("type operand lost")
            };
            assert!(!ty.mentions_rigid(CALIBRATION));
        }
    }
    assert_eq!(specialized.body.kind, ExprKind::Local(local));
    assert_eq!(specialized.parameters, template.parameters);
    assert_eq!(specialized.entry_snapshots, template.entry_snapshots);
    assert_eq!(specialized.result_binding, template.result_binding);
    assert_eq!(specialized.body.source, template.body.source);
    assert_eq!(specialized.phase, template.phase);
    assert!(specialized.pure);
    assert!(
        template
            .instantiate_parameters(&[input], &[primitive(Primitive::Bool)])
            .is_err()
    );
    assert_eq!(template, original);
    let mut malformed = template.clone();
    malformed.postconditions[0].ty = Term::Rigid {
        declaration: CALIBRATION,
        sort: Sort::Type,
    };
    let before_failure = malformed.clone();
    assert!(matches!(
        malformed.instantiate_parameters(&[input], &[Term::Bool(true)]),
        Err(TypeError::WrongSort {
            expected: Sort::ComptimeValue,
            actual: Sort::Type
        })
    ));
    assert_eq!(malformed, before_failure);
}

#[test]
fn binding_function_inputs_erases_slots_and_reindexes_nested_dependencies() {
    let opaque = |args| Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: args,
    };
    let signature = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Type,
                ty: primitive(Primitive::Type),
            },
            Parameter {
                sort: Sort::Runtime,
                ty: bound(0, 0, Sort::Type),
            },
            Parameter {
                sort: Sort::ComptimeValue,
                ty: primitive(Primitive::Bool),
            },
        ],
        result: Box::new(Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: bound(1, 0, Sort::Type),
            }],
            result: Box::new(opaque(vec![
                bound(1, 1, Sort::Runtime),
                bound(1, 2, Sort::ComptimeValue),
                bound(0, 0, Sort::Runtime),
            ])),
        }),
    };
    let specialized = signature
        .bind_function_inputs(&[Some(primitive(Primitive::U8)), None, Some(Term::Bool(true))])
        .unwrap();
    assert_eq!(
        specialized,
        Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: primitive(Primitive::U8)
            }],
            result: Box::new(Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: primitive(Primitive::U8)
                }],
                result: Box::new(opaque(vec![
                    bound(1, 0, Sort::Runtime),
                    Term::Bool(true),
                    bound(0, 0, Sort::Runtime)
                ])),
            }),
        }
    );
    assert_eq!(
        signature.bind_function_inputs(&[None, None, None]).unwrap(),
        signature
    );
    // Repeated specialization must agree with binding all inputs at once,
    // including the runtime object's identity under the nested function.
    assert_eq!(
        specialized
            .bind_function_inputs(&[Some(object(77))])
            .unwrap(),
        signature
            .bind_function_inputs(&[
                Some(primitive(Primitive::U8)),
                Some(object(77)),
                Some(Term::Bool(true))
            ])
            .unwrap()
    );
    let wrapped = Term::Closure {
        environment: vec![primitive(Primitive::Bool)],
        signature: Box::new(Term::ComptimeFunction(Box::new(signature))),
    };
    assert_eq!(
        wrapped
            .bind_function_inputs(&[Some(primitive(Primitive::U8)), None, Some(Term::Bool(true))])
            .unwrap(),
        Term::Closure {
            environment: vec![primitive(Primitive::Bool)],
            signature: Box::new(Term::ComptimeFunction(Box::new(specialized)))
        }
    );
}

#[test]
fn binding_function_inputs_checks_unused_values_and_forward_references() {
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::ComptimeValue,
            ty: primitive(Primitive::Bool),
        }],
        result: Box::new(primitive(Primitive::Bool)),
    };
    assert!(signature.bind_function_inputs(&[]).is_err());
    assert!(
        signature
            .bind_function_inputs(&[Some(primitive(Primitive::U8))])
            .is_err()
    );
    assert!(
        signature
            .bind_function_inputs(&[Some(bound(0, 0, Sort::ComptimeValue))])
            .is_err()
    );
    assert_eq!(
        signature
            .bind_function_inputs(&[Some(Term::Bool(true))])
            .unwrap(),
        Term::Function {
            parameters: vec![],
            result: Box::new(primitive(Primitive::Bool))
        }
    );
    let invalid = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: bound(0, 0, Sort::Type),
        }],
        result: Box::new(primitive(Primitive::Bool)),
    };
    assert!(
        invalid
            .bind_function_inputs(&[Some(primitive(Primitive::Bool))])
            .is_err()
    );
    assert!(matches!(
        primitive(Primitive::Bool).bind_function_inputs(&[]),
        Err(TypeError::InvalidFunctionSignature)
    ));
}

#[test]
fn rebinding_a_function_scope_preserves_nested_and_distant_binders() {
    let nested = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Type,
                ty: primitive(Primitive::Type),
            },
            Parameter {
                sort: Sort::Runtime,
                ty: bound(1, 0, Sort::Type),
            },
        ],
        result: Box::new(Term::Tuple(vec![
            bound(0, 0, Sort::Type),
            bound(1, 0, Sort::Type),
            bound(2, 0, Sort::Type),
        ])),
    };
    let rebound = nested.rebind_scope(&[primitive(Primitive::Bool)]).unwrap();
    assert_eq!(
        rebound,
        Term::Function {
            parameters: vec![
                Parameter {
                    sort: Sort::Type,
                    ty: primitive(Primitive::Type)
                },
                Parameter {
                    sort: Sort::Runtime,
                    ty: primitive(Primitive::Bool)
                }
            ],
            result: Box::new(Term::Tuple(vec![
                bound(0, 0, Sort::Type),
                primitive(Primitive::Bool),
                bound(2, 0, Sort::Type)
            ])),
        }
    );
    assert!(
        bound(0, 0, Sort::Type)
            .rebind_scope(&[Term::Bool(true)])
            .is_err()
    );
    assert!(
        bound(0, 1, Sort::Type)
            .rebind_scope(&[primitive(Primitive::Bool)])
            .is_err()
    );
    let head = Term::NormalForm {
        head: IDENTITY,
        arguments: vec![bound(0, 0, Sort::Type)],
        sort: Sort::Type,
    };
    assert_eq!(
        head.rebind_scope(&[primitive(Primitive::Bool)]).unwrap(),
        Term::NormalForm {
            head: IDENTITY,
            arguments: vec![primitive(Primitive::Bool)],
            sort: Sort::Type
        }
    );
}

#[test]
fn scoped_call_binding_refines_domains_without_capturing_caller_inputs() {
    let signature = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Type,
                ty: primitive(Primitive::Type),
            },
            Parameter {
                sort: Sort::ComptimeValue,
                ty: bound(0, 0, Sort::Type),
            },
            Parameter {
                sort: Sort::Runtime,
                ty: primitive(Primitive::Bool),
            },
        ],
        result: Box::new(primitive(Primitive::Bool)),
    };
    let inputs = [
        Some(primitive(Primitive::Type)),
        Some(bound(0, 0, Sort::Type)),
        None,
    ];
    let binding = FunctionCallBinding::new(&signature, &inputs, &[vec![Sort::Type]], |ty| {
        Ok::<_, TypeError>(ty.clone())
    })
    .unwrap();
    let annotation = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: primitive(Primitive::Type),
        }],
        result: Box::new(Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![
                bound(1, 1, Sort::ComptimeValue),
                bound(0, 0, Sort::Type),
                bound(2, 0, Sort::Type),
            ],
        }),
    };
    assert_eq!(
        binding.substitute(&annotation).unwrap(),
        Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Type,
                ty: primitive(Primitive::Type)
            }],
            result: Box::new(Term::NormalForm {
                head: IDENTITY,
                sort: Sort::Type,
                arguments: vec![
                    bound(1, 0, Sort::Type),
                    bound(0, 0, Sort::Type),
                    bound(1, 0, Sort::Type)
                ]
            }),
        }
    );
    assert!(annotation.substitute_partial(&inputs).is_err());
    assert!(matches!(
        binding.substitute(&bound(0, 2, Sort::Runtime)),
        Err(TypeError::MissingArgument(2))
    ));
    assert!(binding.substitute(&bound(0, 1, Sort::Type)).is_err());
    assert!(binding.to_string().contains("(type-slots 1)"));
    assert!(binding.to_string().contains("(caller-frames (frame type))"));
    assert!(
        FunctionCallBinding::new(&signature, &inputs, &[], |ty| Ok::<_, TypeError>(
            ty.clone()
        ))
        .is_err()
    );
}

#[test]
fn scoped_call_binding_checks_view_results_and_does_not_unfold_opaque_domains() {
    let opaque = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: vec![],
    };
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::ComptimeValue,
            ty: opaque.clone(),
        }],
        result: Box::new(primitive(Primitive::Bool)),
    };
    let inputs = [Some(primitive(Primitive::Bool))];
    assert!(
        FunctionCallBinding::new(&signature, &inputs, &[], |ty| Ok::<_, &str>(ty.clone())).is_err()
    );
    let binding = FunctionCallBinding::new(&signature, &inputs, &[], |_| {
        Ok::<_, &str>(primitive(Primitive::Type))
    })
    .unwrap();
    assert_eq!(
        binding
            .substitute(&bound(0, 0, Sort::ComptimeValue))
            .unwrap(),
        primitive(Primitive::Bool)
    );
    for invalid in [Term::Bool(true), bound(0, 0, Sort::Type)] {
        assert!(
            FunctionCallBinding::new(&signature, &inputs, &[], |_| Ok::<_, &str>(invalid.clone()))
                .is_err()
        );
    }
    assert!(matches!(
        FunctionCallBinding::new(&signature, &inputs, &[], |_| Err("view failed")),
        Err(CallError::Contract {
            parameter: 0,
            error: "view failed"
        })
    ));
    assert!(
        FunctionCallBinding::new(&signature, &[], &[], |ty| Ok::<_, &str>(ty.clone())).is_err()
    );
    assert!(
        FunctionCallBinding::new(
            &signature,
            &[Some(Term::Bool(true))],
            &[],
            |_| Ok::<_, &str>(primitive(Primitive::Type))
        )
        .is_err()
    );
}

#[test]
fn type_list_values_preserve_opaque_heads_and_distinguish_tuple_representations() {
    let elements = vec![primitive(Primitive::U8), primitive(Primitive::Bool)];
    let list = Term::TypeListValue(elements.clone());
    assert_eq!(list.sort(), Sort::ComptimeValue);
    assert!(list.check_scope(&[]).is_ok());
    assert_ne!(list, Term::Tuple(elements.clone()));
    assert_ne!(list, Term::TupleValue(elements.clone()));
    assert_ne!(Term::TypeListValue(vec![]), Term::Unit);
    assert_ne!(Term::TypeListValue(vec![]), Term::TupleValue(vec![]));
    assert_eq!(
        list.to_string(),
        "(type-list-value ((primitive u8) (primitive bool)))"
    );
    assert_eq!(
        HashSet::from([list.clone(), Term::TypeListValue(elements)]).len(),
        1
    );

    let application = Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(vec![Term::Rigid {
            declaration: SENSOR,
            sort: Sort::Type,
        }])],
    };
    let instantiated = application
        .instantiate_types(&[SENSOR], &[primitive(Primitive::U8)])
        .unwrap();
    assert_eq!(
        instantiated,
        Term::NormalForm {
            head: IDENTITY,
            sort: Sort::Type,
            arguments: vec![Term::TypeListValue(vec![primitive(Primitive::U8)])],
        }
    );
    assert_ne!(instantiated, primitive(Primitive::U8));
    assert_ne!(instantiated, Term::Tuple(vec![primitive(Primitive::U8)]));
}

#[test]
fn type_list_values_validate_elements_and_substitute_without_adding_a_binder() {
    for invalid in [
        Term::Bool(true),
        Term::Unit,
        object(1),
        Term::TypeListValue(vec![]),
    ] {
        assert!(Term::TypeListValue(vec![invalid]).check_scope(&[]).is_err());
    }
    let open = Term::TypeListValue(vec![bound(0, 0, Sort::Type)]);
    assert!(open.check_scope(&[]).is_err());
    assert!(open.check_scope(&[vec![Sort::ComptimeValue]]).is_err());
    assert!(open.check_scope(&[vec![Sort::Type]]).is_ok());
    assert_eq!(
        open.substitute(&[primitive(Primitive::Char)]).unwrap(),
        Term::TypeListValue(vec![primitive(Primitive::Char)])
    );
    assert!(open.substitute(&[Term::Bool(true)]).is_err());
}

#[test]
fn type_list_inputs_retain_runtime_dependencies_inside_element_types() {
    let make = |element| Term::NormalForm {
        head: IDENTITY,
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(vec![element])],
    };
    let template = make(Term::Nominal {
        declaration: SENSOR,
        arguments: vec![hole(0, Sort::Runtime)],
    });
    let parameters = vec![Parameter {
        sort: Sort::Runtime,
        ty: calibration(),
    }];
    let mut interner = TypeInterner::default();
    let first = interner
        .intern(parameters.clone(), template.clone())
        .unwrap();
    let second = interner.intern(parameters, template).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let reference = TypeRef::bind(first, vec![object(8)]).unwrap();
    assert_eq!(
        reference.runtime_dependencies().unwrap(),
        BTreeSet::from([RuntimeId {
            instance: 8,
            local: 0
        }])
    );
    assert_eq!(
        reference.materialize().unwrap(),
        make(Term::Nominal {
            declaration: SENSOR,
            arguments: vec![object(8)],
        })
    );
    assert!(reference.check_runtime_bindings(&BTreeSet::new()).is_err());
}

#[test]
fn frozen_function_values_preserve_declaration_identity_and_value_sort() {
    let value = Term::FunctionValue {
        declaration: IDENTITY,
        arguments: vec![],
    };
    assert_eq!(value.sort(), Sort::ComptimeValue);
    assert_ne!(
        value,
        Term::FunctionValue {
            declaration: SENSOR,
            arguments: vec![]
        }
    );
    value.check_scope(&[]).unwrap();
    assert_eq!(value.substitute_partial(&[]).unwrap(), value);
    assert_eq!(
        value.to_string(),
        format!("(function-value {IDENTITY} (arguments))")
    );
}

#[test]
fn function_instance_values_rebind_and_charge_their_arguments() {
    let instance = Term::FunctionValue {
        declaration: IDENTITY,
        arguments: vec![bound(0, 0, Sort::Type)],
    };
    assert!(instance.check_scope(&[]).is_err());
    instance.check_scope(&[vec![Sort::Type]]).unwrap();
    let boolean = primitive(Primitive::Bool);
    let closed = instance
        .substitute_partial(&[Some(boolean.clone())])
        .unwrap();
    assert_eq!(
        closed,
        Term::FunctionValue {
            declaration: IDENTITY,
            arguments: vec![boolean]
        }
    );
    assert_ne!(
        closed,
        Term::FunctionValue {
            declaration: IDENTITY,
            arguments: vec![primitive(Primitive::U8)]
        }
    );
    let mut fuel = 1;
    assert!(nir::type_computations::charge_term(&closed, &mut fuel).is_err());
    let mut fuel = 2;
    nir::type_computations::charge_term(&closed, &mut fuel).unwrap();
    assert_eq!(fuel, 0);
    assert!(closed.to_string().contains("(arguments (primitive bool))"));
}

#[test]
fn closure_snapshot_is_a_value_with_distinct_factory_identity() {
    let value = Term::ClosureSnapshot {
        factory: IDENTITY,
        arguments: vec![],
    };
    assert_eq!(value.sort(), Sort::ComptimeValue);
    value.check_scope(&[]).unwrap();
    assert_ne!(
        value,
        Term::ClosureSnapshot {
            factory: SENSOR,
            arguments: vec![]
        }
    );
    assert_eq!(
        value.to_string(),
        format!("(closure-snapshot (factory {IDENTITY}) (arguments))")
    );
}

#[test]
fn closure_snapshot_arguments_keep_their_scope_and_substitution() {
    let snapshot = Term::ClosureSnapshot {
        factory: IDENTITY,
        arguments: vec![bound(0, 0, Sort::Type)],
    };
    assert!(snapshot.check_scope(&[]).is_err());
    snapshot.check_scope(&[vec![Sort::Type]]).unwrap();
    let closed = snapshot
        .substitute_partial(&[Some(primitive(Primitive::Bool))])
        .unwrap();
    assert_eq!(
        closed,
        Term::ClosureSnapshot {
            factory: IDENTITY,
            arguments: vec![primitive(Primitive::Bool)]
        }
    );
    assert_ne!(
        closed,
        Term::ClosureSnapshot {
            factory: IDENTITY,
            arguments: vec![primitive(Primitive::U8)]
        }
    );
    let mut fuel = 1;
    assert!(nir::type_computations::charge_term(&closed, &mut fuel).is_err());
    nir::type_computations::charge_term(&closed, &mut 2).unwrap();
    assert!(closed.to_string().contains("(arguments (primitive bool))"));
}

#[test]
fn closure_snapshot_rejects_non_type_factory_inputs() {
    let invalid = Term::ClosureSnapshot {
        factory: IDENTITY,
        arguments: vec![Term::Bool(true)],
    };
    assert!(invalid.check_scope(&[]).is_err());
}
