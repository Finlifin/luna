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
fn object(instance: u64) -> Term {
    Term::Runtime(RuntimeId { instance, local: 0 })
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
