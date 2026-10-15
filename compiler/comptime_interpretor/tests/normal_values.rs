use comptime_interpretor::{ErrorKind, Value};
use comptime_runtime::{Budget, Integer};
use nir::{DeclarationId, Primitive, Term};

#[test]
fn symbol_freezing_preserves_identity_and_charges_names() {
    let name = "quoted\"\\\nλ";
    let value = Value::Symbol(name.into());
    assert_eq!(
        value
            .clone()
            .into_normal_term(&mut Budget::default())
            .unwrap(),
        Term::Symbol(name.into())
    );
    assert_eq!(value.to_string(), "(symbol \"quoted\\\"\\\\\\nλ\")");
    let large = Value::Symbol("x".repeat(1024).into());
    assert!(
        large
            .into_normal_term(&mut Budget {
                steps: 8,
                ..Budget::default()
            })
            .is_err()
    );
    assert_eq!(
        value.into_normal_term(&mut Budget::default()).unwrap(),
        Term::Symbol(name.into())
    );
}

#[test]
fn unresolved_associated_types_are_not_completed_comptime_type_values() {
    let projection = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: DeclarationId {
                package: 1,
                local: 1,
            },
            member: DeclarationId {
                package: 1,
                local: 2,
            },
        }),
        subject: Box::new(Term::Primitive(Primitive::Bool)),
        arguments: vec![],
    };
    for ty in [projection.clone(), Term::Pointer(Box::new(projection))] {
        assert_eq!(
            Value::Type(ty).into_normal_term(&mut Budget::default()),
            Err(ErrorKind::Type)
        );
    }
}

#[test]
fn freezing_nested_values_preserves_structure_and_type_identity() {
    let head = Term::NormalForm {
        head: DeclarationId {
            package: 1,
            local: 2,
        },
        sort: nir::Sort::Type,
        arguments: vec![Term::Primitive(Primitive::U8)],
    };
    let value = Value::Tuple(vec![
        Value::Type(head.clone()),
        Value::Tuple(vec![Value::String("λ\n".into()), Value::SignedInteger(-1)]),
    ]);
    assert_eq!(
        value.into_normal_term(&mut Budget::default()).unwrap(),
        Term::TupleValue(vec![
            head,
            Term::TupleValue(vec![
                Term::String("λ\n".into()),
                Term::Integer(Integer::from_i128(-1)),
            ])
        ])
    );
}

#[test]
fn freezing_charges_every_field_and_all_integer_representations() {
    for value in [
        Value::UnsignedInteger(u128::MAX),
        Value::SignedInteger(i128::MIN),
        Value::Integer(Integer::from_u128(u128::MAX)),
    ] {
        assert_eq!(
            value.into_normal_term(&mut Budget {
                steps: 100,
                max_limbs: 1
            }),
            Err(ErrorKind::RepresentationSizeLimit)
        );
    }
    let mut budget = Budget {
        steps: 3,
        max_limbs: 4,
    };
    assert_eq!(
        Value::Tuple(vec![Value::Bool(true), Value::String("λ".into())])
            .into_normal_term(&mut budget),
        Err(ErrorKind::StepsExhausted)
    );
    assert_eq!(budget.steps, 0);
    assert_eq!(
        Value::Bool(false).into_normal_term(&mut Budget::default()),
        Ok(Term::Bool(false))
    );
}

#[test]
fn freezing_rejects_deep_values_and_executable_values() {
    let mut value = Value::Bool(true);
    for _ in 0..140 {
        value = Value::Tuple(vec![value]);
    }
    assert_eq!(
        value.into_normal_term(&mut Budget::default()),
        Err(ErrorKind::DepthExhausted)
    );
    assert_eq!(
        Value::Tuple(vec![Value::Function(DeclarationId {
            package: 1,
            local: 2
        })])
        .into_normal_term(&mut Budget::default()),
        Err(ErrorKind::Unsupported)
    );
    assert_eq!(
        Value::Type(Term::Bool(true)).into_normal_term(&mut Budget::default()),
        Err(ErrorKind::Type)
    );
}

#[test]
fn bitvec_freezing_preserves_width_and_obeys_storage_budget() {
    let bits = Integer::from_u128(7)
        .to_bitvec(129, &mut Budget::default())
        .unwrap();
    assert_eq!(
        Value::BitVec(bits.clone()).into_normal_term(&mut Budget::default()),
        Ok(Term::BitVec(bits.clone()))
    );
    assert_eq!(
        Value::BitVec(bits.clone()).into_normal_term(&mut Budget {
            steps: 100,
            max_limbs: 4
        }),
        Err(ErrorKind::RepresentationSizeLimit)
    );
    assert_eq!(
        Value::BitVec(bits).into_normal_term(&mut Budget {
            steps: 5,
            max_limbs: 5
        }),
        Err(ErrorKind::StepsExhausted)
    );
}

#[test]
fn unit_freezing_is_canonical_and_charges_work() {
    assert_eq!(
        Value::Unit.into_normal_term(&mut Budget::default()),
        Ok(Term::Unit)
    );
    assert_eq!(
        Value::Tuple(vec![Value::Unit]).into_normal_term(&mut Budget::default()),
        Ok(Term::TupleValue(vec![Term::Unit]))
    );
    assert_eq!(
        Value::Unit.into_normal_term(&mut Budget {
            steps: 0,
            max_limbs: 0
        }),
        Err(ErrorKind::StepsExhausted)
    );
}

#[test]
fn freezing_records_preserves_names_and_bounds_label_work() {
    let value = Value::AnonymousRecord(vec![
        ("flag".into(), Value::Bool(true)),
        ("nested".into(), Value::Tuple(vec![Value::Unit])),
    ]);
    let expected = Term::RecordValue(vec![
        ("flag".into(), Term::Bool(true)),
        ("nested".into(), Term::TupleValue(vec![Term::Unit])),
    ]);
    assert_eq!(
        value.clone().into_normal_term(&mut Budget::default()),
        Ok(expected)
    );
    assert_eq!(
        value.into_normal_term(&mut Budget {
            steps: 3,
            max_limbs: 4
        }),
        Err(ErrorKind::StepsExhausted)
    );
    assert_eq!(
        Value::AnonymousRecord(vec![("x".into(), Value::Unit), ("x".into(), Value::Unit)])
            .into_normal_term(&mut Budget::default()),
        Err(ErrorKind::Type)
    );
    assert_eq!(
        Value::AnonymousRecord(vec![(
            "code".into(),
            Value::Function(DeclarationId {
                package: 1,
                local: 2
            })
        )])
        .into_normal_term(&mut Budget::default()),
        Err(ErrorKind::Unsupported)
    );
}

#[test]
fn type_list_freezing_preserves_list_identity_order_and_opaque_heads() {
    let opaque = Term::NormalForm {
        head: DeclarationId {
            package: 1,
            local: 8,
        },
        sort: nir::Sort::Type,
        arguments: vec![Term::TypeListValue(vec![Term::Primitive(Primitive::U8)])],
    };
    let elements = vec![opaque, Term::Primitive(Primitive::Bool)];
    let list = Value::TypeList(elements.clone());
    let expected = Term::TypeListValue(elements.clone());
    assert_eq!(list.to_string(), expected.to_string());
    assert_eq!(
        list.clone().into_normal_term(&mut Budget::default()),
        Ok(expected.clone())
    );
    assert_ne!(
        list,
        Value::Tuple(elements.iter().cloned().map(Value::Type).collect())
    );
    assert_ne!(expected, Term::TupleValue(elements));
    assert_eq!(
        Value::TypeList(vec![]).into_normal_term(&mut Budget::default()),
        Ok(Term::TypeListValue(vec![]))
    );
    assert_ne!(Value::TypeList(vec![]), Value::Tuple(vec![]));
    let nested = Value::Tuple(vec![list]);
    assert_eq!(
        nested.into_normal_term(&mut Budget::default()),
        Ok(Term::TupleValue(vec![expected]))
    );
}

#[test]
fn type_list_freezing_rejects_non_types_and_unfinished_inputs() {
    let identity = DeclarationId {
        package: 1,
        local: 1,
    };
    let invalid = [
        Term::Bool(true),
        Term::TypeListValue(vec![]),
        Term::Rigid {
            declaration: identity,
            sort: nir::Sort::Type,
        },
        Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::Type,
        },
        Term::Nominal {
            declaration: identity,
            arguments: vec![Term::Runtime(nir::RuntimeId {
                instance: 1,
                local: 0,
            })],
        },
        Term::NormalForm {
            head: identity,
            sort: nir::Sort::Type,
            arguments: vec![Term::TypeListValue(vec![Term::Bool(false)])],
        },
    ];
    for element in invalid {
        let value = Value::TypeList(vec![element]);
        assert_eq!(value.immutable_copy_work(), None);
        assert_eq!(
            value.into_normal_term(&mut Budget::default()),
            Err(ErrorKind::Type)
        );
    }
    let scoped = Term::Function {
        parameters: vec![nir::Parameter {
            sort: nir::Sort::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
        result: Box::new(Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::Type,
        }),
    };
    assert!(
        Value::TypeList(vec![scoped])
            .into_normal_term(&mut Budget::default())
            .is_ok()
    );
}

#[test]
fn type_list_copy_and_freezing_charge_contents_and_recover_after_budget_failure() {
    let small = Value::TypeList(vec![Term::Primitive(Primitive::Bool); 8]);
    let work = small.immutable_copy_work().unwrap();
    let mut enough = Budget {
        steps: work,
        ..Budget::default()
    };
    assert!(small.clone().into_normal_term(&mut enough).is_ok());
    assert_eq!(enough.steps, 0);
    assert_eq!(
        small.clone().into_normal_term(&mut Budget {
            steps: work - 1,
            ..Budget::default()
        }),
        Err(ErrorKind::StepsExhausted)
    );
    assert!(small.into_normal_term(&mut Budget::default()).is_ok());
    let wide = Value::TypeList(vec![Term::Primitive(Primitive::Bool); 100_001]);
    assert!(wide.immutable_copy_work().is_none());
    assert_eq!(
        wide.into_normal_term(&mut Budget {
            steps: 8,
            ..Budget::default()
        }),
        Err(ErrorKind::StepsExhausted)
    );
}

#[test]
fn completed_type_lists_do_not_pass_tuple_or_type_admission() {
    let package = nir::program::Package::default();
    let mut interpreter =
        comptime_interpretor::Interpreter::new(&package, comptime_interpretor::Budget::default())
            .unwrap();
    let types = vec![Term::Primitive(Primitive::Bool)];
    let list = Value::TypeList(types.clone());
    for ty in [
        Term::Primitive(Primitive::Type),
        Term::Tuple(vec![Term::Primitive(Primitive::Type)]),
    ] {
        assert!(interpreter.validate_immutable_input(&list, &ty).is_err());
    }
    let tuple = Value::Tuple(types.into_iter().map(Value::Type).collect());
    assert!(
        interpreter
            .validate_immutable_input(&tuple, &Term::Tuple(vec![Term::Primitive(Primitive::Type)]))
            .is_ok()
    );
}

#[test]
fn unreduced_type_list_concatenations_do_not_cross_completed_value_boundaries() {
    let head = DeclarationId {
        package: 1,
        local: 17,
    };
    let parts = vec![
        Term::TypeListValue(vec![Term::Primitive(Primitive::Bool)]),
        Term::TypeListValue(vec![Term::Primitive(Primitive::U8)]),
    ];
    let applied = |input| Term::NormalForm {
        head,
        sort: nir::Sort::Type,
        arguments: vec![input],
    };
    // Public IR constructors can bypass the canonical constructor. Scope-valid
    // does not imply reduced: this must not create a second completed identity.
    let unfinished = applied(Term::TypeListConcat(parts.clone()));
    unfinished.check_scope(&[]).unwrap();
    let completed = applied(Term::type_list_concat(parts));
    let package = nir::program::Package::default();
    let mut interpreter =
        comptime_interpretor::Interpreter::new(&package, comptime_interpretor::Budget::default())
            .unwrap();
    for value in [
        Value::Type(unfinished.clone()),
        Value::TypeList(vec![unfinished.clone()]),
        Value::Tuple(vec![Value::Type(unfinished.clone())]),
    ] {
        assert_eq!(value.immutable_copy_work(), None);
        assert_eq!(
            value.into_normal_term(&mut Budget::default()),
            Err(ErrorKind::Type)
        );
    }
    assert!(
        interpreter
            .validate_immutable_input(&Value::Type(unfinished), &Term::Primitive(Primitive::Type))
            .is_err()
    );
    assert!(
        interpreter
            .validate_immutable_input(
                &Value::Type(Term::Primitive(Primitive::Bool)),
                &Term::Primitive(Primitive::Type)
            )
            .is_ok()
    );
    assert_eq!(
        Value::Type(completed.clone()).into_normal_term(&mut Budget::default()),
        Ok(completed)
    );
}

#[test]
fn aggregate_freezing_reports_depth_on_a_two_mebibyte_stack_and_recovers() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            for (record, depth, exhausted) in [
                (false, 140, false),
                (true, 140, false),
                (false, 65536, false),
                (true, 65536, false),
                (false, 65536, true),
                (true, 65536, true),
            ] {
                let mut value = Value::Bool(true);
                for _ in 0..depth {
                    value = if record {
                        Value::AnonymousRecord(vec![("next".into(), value)])
                    } else {
                        Value::Tuple(vec![value])
                    };
                }
                let mut budget = Budget::default();
                if exhausted {
                    budget.steps = 0;
                }
                assert_eq!(
                    value.into_normal_term(&mut budget),
                    Err(if exhausted {
                        ErrorKind::StepsExhausted
                    } else {
                        ErrorKind::DepthExhausted
                    })
                );
                assert_eq!(
                    Value::Bool(true).into_normal_term(&mut Budget::default()),
                    Ok(Term::Bool(true))
                );
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn rejected_embedded_types_are_released_on_a_two_mebibyte_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            for list in [false, true] {
                let mut ty = Term::Primitive(Primitive::Bool);
                for _ in 0..65536 {
                    ty = Term::Pointer(Box::new(ty));
                }
                let value = if list {
                    Value::TypeList(vec![ty])
                } else {
                    Value::Type(ty)
                };
                assert_eq!(
                    value.into_normal_term(&mut Budget::default()),
                    Err(ErrorKind::Type)
                );
                assert_eq!(
                    Value::Bool(true).into_normal_term(&mut Budget::default()),
                    Ok(Term::Bool(true))
                );
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn newtype_freezing_is_bounded_and_rejects_non_nominal_identities() {
    let ty = Term::Nominal {
        declaration: DeclarationId {
            package: 1,
            local: 9,
        },
        arguments: vec![],
    };
    let value = Value::Newtype {
        ty: ty.clone(),
        value: Box::new(Value::Bool(true)),
    };
    assert_eq!(
        value
            .clone()
            .into_normal_term(&mut Budget::default())
            .unwrap(),
        Term::NominalValue {
            ty: Box::new(ty),
            variant: None,
            fields: vec![Term::Bool(true)]
        }
    );
    assert_eq!(
        value.into_normal_term(&mut Budget {
            steps: 0,
            ..Budget::default()
        }),
        Err(ErrorKind::StepsExhausted)
    );
    assert_eq!(
        Value::Newtype {
            ty: Term::Primitive(Primitive::Bool),
            value: Box::new(Value::Bool(true))
        }
        .into_normal_term(&mut Budget::default()),
        Err(ErrorKind::Type)
    );
}

#[test]
fn nested_newtype_freezing_rejects_depth_and_executable_payloads() {
    let ty = Term::Nominal {
        declaration: DeclarationId {
            package: 1,
            local: 9,
        },
        arguments: vec![],
    };
    let mut deep = Value::Bool(true);
    for _ in 0..140 {
        deep = Value::Newtype {
            ty: ty.clone(),
            value: Box::new(deep),
        };
    }
    assert_eq!(
        deep.into_normal_term(&mut Budget::default()),
        Err(ErrorKind::DepthExhausted)
    );
    let executable = Value::Newtype {
        ty: ty.clone(),
        value: Box::new(Value::Function(DeclarationId {
            package: 1,
            local: 10,
        })),
    };
    assert_eq!(
        executable.into_normal_term(&mut Budget::default()),
        Err(ErrorKind::Unsupported)
    );
    assert!(
        Value::Newtype {
            ty,
            value: Box::new(Value::Bool(true))
        }
        .into_normal_term(&mut Budget::default())
        .is_ok()
    );
}
