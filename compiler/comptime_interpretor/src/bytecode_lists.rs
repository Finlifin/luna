//! List checks use the declaration and element domain, preserving effect order.
use super::*;
use nir::normal_forms::{Head, Kind, Parameter, Role};
use nir::program::{Literal, Place};
const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn list(element: Primitive) -> Term {
    Term::NormalForm {
        head: id(2),
        sort: nir::Sort::Type,
        arguments: vec![Term::Primitive(element)],
    }
}
fn package() -> Package {
    Package {
        normal_forms: vec![Head {
            collector: None,
            role: Role::TypeList,
            declaration: id(2),
            requirements: vec![],
            parameters: vec![Parameter {
                declaration: id(3),
                binding: LocalId { owner: 2, index: 0 },
                kind: Kind::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
        }],
        ..Default::default()
    }
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty,
        kind,
    }
}
fn run(
    vm: &mut Interpreter<'_>,
    body: &Expr,
    env: &mut Environment,
    compiled: bool,
) -> Result<Flow, Error> {
    if compiled {
        let code = Arc::new(comptime_compiler::compile(body, &mut 10000).unwrap());
        vm.execute_code(&code, env)
    } else {
        vm.eval(body, env)
    }
}

#[test]
fn list_identity_checks_precede_concat_suffix_and_length_reads() {
    let package = package();
    let input = LocalId { owner: 1, index: 0 };
    let trace = LocalId { owner: 1, index: 1 };
    let left = expr(list(Primitive::Bool), K::Local(input));
    let right = expr(
        list(Primitive::Bool),
        K::Block {
            statements: vec![expr(
                Term::Primitive(Primitive::Void),
                K::Assign {
                    place: Place {
                        root: trace,
                        fields: vec![],
                    },
                    value: Box::new(expr(
                        Term::Primitive(Primitive::Bool),
                        K::Literal(Literal::Bool(true)),
                    )),
                },
            )],
            result: Some(Box::new(expr(list(Primitive::Bool), K::TypeList(vec![])))),
        },
    );
    let concat = expr(
        list(Primitive::Bool),
        K::ListConcat(Box::new(left.clone()), Box::new(right)),
    );
    let length = expr(
        Term::Primitive(Primitive::Integer),
        K::ListLength(Box::new(left)),
    );
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for valid in [false, true] {
            let original = if valid {
                Value::List {
                    ty: list(Primitive::Bool),
                    values: vec![Value::Bool(false)],
                }
            } else {
                Value::List {
                    ty: list(Primitive::Char),
                    values: vec![Value::Char('x')],
                }
            };
            let mut env = Environment::new();
            env.insert(input, original.clone());
            env.insert(trace, Value::Bool(false));
            let result = run(&mut vm, &concat, &mut env, compiled);
            if valid {
                assert!(matches!(result.unwrap(), Flow::Value(Value::List { .. })));
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            }
            assert_eq!(env.get(&trace), Some(&Value::Bool(valid)));
            assert_eq!(env.get(&input), Some(&original));
            let result = run(&mut vm, &length, &mut env, compiled);
            if valid {
                assert!(matches!(result.unwrap(), Flow::Value(Value::Integer(_))));
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            }
        }
    }
}

#[test]
fn construction_keeps_type_conversion_and_aggregate_validation_boundaries() {
    let package = package();
    let input = LocalId { owner: 1, index: 0 };
    let trace = LocalId { owner: 1, index: 1 };
    for compiled in [false, true] {
        for element in [Primitive::Bool, Primitive::Type] {
            let ty = Term::Primitive(element);
            let second = expr(
                ty.clone(),
                K::Block {
                    statements: vec![expr(
                        Term::Primitive(Primitive::Void),
                        K::Assign {
                            place: Place {
                                root: trace,
                                fields: vec![],
                            },
                            value: Box::new(expr(
                                Term::Primitive(Primitive::Bool),
                                K::Literal(Literal::Bool(true)),
                            )),
                        },
                    )],
                    result: Some(Box::new(expr(
                        ty.clone(),
                        if element == Primitive::Type {
                            K::TypeValue(Term::Primitive(Primitive::Bool))
                        } else {
                            K::Literal(Literal::Bool(true))
                        },
                    ))),
                },
            );
            let body = expr(
                list(element),
                K::TypeList(vec![expr(ty, K::Local(input)), second]),
            );
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env = Environment::new();
            env.insert(input, Value::Char('x'));
            env.insert(trace, Value::Bool(false));
            assert_eq!(
                run(&mut vm, &body, &mut env, compiled).unwrap_err().kind,
                ErrorKind::Type
            );
            assert_eq!(
                env.get(&trace),
                Some(&Value::Bool(element != Primitive::Type))
            );
            env.insert(
                input,
                if element == Primitive::Type {
                    Value::Type(Term::Primitive(Primitive::Bool))
                } else {
                    Value::Bool(false)
                },
            );
            assert!(matches!(
                run(&mut vm, &body, &mut env, compiled).unwrap(),
                Flow::Value(_)
            ));
        }
    }
}

#[test]
fn list_indices_read_sibling_references_after_releasing_receiver_locks() {
    let package = package();
    let receiver = LocalId { owner: 1, index: 0 };
    let index = LocalId { owner: 1, index: 1 };
    for element in [Primitive::Bool, Primitive::Type] {
        let value = if element == Primitive::Type {
            Value::List {
                ty: list(Primitive::Type),
                values: vec![Value::Type(Term::Primitive(Primitive::Bool))],
            }
        } else {
            Value::List {
                ty: list(element),
                values: vec![Value::Bool(true)],
            }
        };
        let storage = Storage::new(Value::AnonymousRecord(vec![
            ("list".into(), value),
            (
                "index".into(),
                Value::Integer(comptime_runtime::Integer::from_u128(0)),
            ),
        ]));
        let mut env = Environment::new();
        for (binding, field) in [(receiver, 0), (index, 1)] {
            let reference = value_reference::ValueReference::projected(
                storage.clone(),
                &[(nir::program::FieldOwner::Anonymous, field)],
                &mut 1000,
            )
            .unwrap();
            env.insert(binding, Value::ComptimeReference(reference));
        }
        let body = expr(
            Term::Primitive(element),
            K::TypeListIndex(
                Box::new(expr(list(element), K::Local(receiver))),
                Box::new(expr(Term::Primitive(Primitive::Integer), K::Local(index))),
            ),
        );
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let Flow::Value(value) = run(&mut vm, &body, &mut env, compiled).unwrap() else {
                panic!("flow")
            };
            assert_eq!(
                value,
                if element == Primitive::Type {
                    Value::Type(Term::Primitive(Primitive::Bool))
                } else {
                    Value::Bool(true)
                }
            );
        }
    }
}

#[test]
fn indexed_list_domains_are_checked_before_computed_index_effects() {
    let package = package();
    let input = LocalId { owner: 1, index: 0 };
    let trace = LocalId { owner: 1, index: 1 };
    let index = expr(
        Term::Primitive(Primitive::Integer),
        K::Block {
            statements: vec![expr(
                Term::Primitive(Primitive::Void),
                K::Assign {
                    place: Place {
                        root: trace,
                        fields: vec![],
                    },
                    value: Box::new(expr(
                        Term::Primitive(Primitive::Bool),
                        K::Literal(Literal::Bool(true)),
                    )),
                },
            )],
            result: Some(Box::new(expr(
                Term::Primitive(Primitive::Integer),
                K::Literal(Literal::Integer("0".into())),
            ))),
        },
    );
    let body = expr(
        Term::Primitive(Primitive::Bool),
        K::TypeListIndex(
            Box::new(expr(list(Primitive::Bool), K::Local(input))),
            Box::new(index),
        ),
    );
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        env.insert(
            input,
            Value::List {
                ty: list(Primitive::Char),
                values: vec![Value::Char('x')],
            },
        );
        env.insert(trace, Value::Bool(false));
        assert_eq!(
            run(&mut vm, &body, &mut env, compiled).unwrap_err().kind,
            ErrorKind::Type
        );
        assert_eq!(env.get(&trace), Some(&Value::Bool(false)));
        env.insert(
            input,
            Value::List {
                ty: list(Primitive::Bool),
                values: vec![Value::Bool(true)],
            },
        );
        assert!(matches!(
            run(&mut vm, &body, &mut env, compiled).unwrap(),
            Flow::Value(Value::Bool(true))
        ));
        assert_eq!(env.get(&trace), Some(&Value::Bool(true)));
    }
}

#[test]
fn fused_append_preserves_aliases_and_reuses_owned_buffers() {
    let package = package();
    let input = LocalId { owner: 1, index: 0 };
    let suffix = LocalId { owner: 1, index: 1 };
    for element in [Primitive::Bool, Primitive::Type] {
        let ty = list(element);
        for frozen in 0..3 {
            let right = if frozen == 1 {
                expr(
                    ty.clone(),
                    K::ComptimeValue(Term::NominalValue {
                        ty: Box::new(ty.clone()),
                        variant: None,
                        fields: vec![if element == Primitive::Type {
                            Term::Primitive(Primitive::Bool)
                        } else {
                            Term::Bool(true)
                        }],
                    }),
                )
            } else if frozen == 2 {
                let field = if element == Primitive::Type {
                    K::TypeValue(Term::Primitive(Primitive::Bool))
                } else {
                    K::ComptimeValue(Term::Bool(true))
                };
                expr(
                    ty.clone(),
                    K::TypeList(vec![expr(Term::Primitive(element), field)]),
                )
            } else {
                expr(ty.clone(), K::Local(suffix))
            };
            let body = expr(
                Term::Primitive(Primitive::Void),
                K::Assign {
                    place: Place {
                        root: input,
                        fields: vec![],
                    },
                    value: Box::new(expr(
                        ty.clone(),
                        K::ListConcat(Box::new(expr(ty.clone(), K::Local(input))), Box::new(right)),
                    )),
                },
            );
            let code = comptime_compiler::compile(&body, &mut 1000)
                .unwrap()
                .to_string();
            assert!(code.contains("(list-append "));
            assert_eq!(
                code,
                comptime_compiler::compile(&body, &mut 1000)
                    .unwrap()
                    .to_string()
            );
            for compiled in [false, true] {
                for carrier in 0..3 {
                    let item = if element == Primitive::Type {
                        Value::Type(Term::Primitive(Primitive::Bool))
                    } else {
                        Value::Bool(true)
                    };
                    let mut values = Vec::with_capacity(8);
                    values.push(item.clone());
                    let original = Value::List {
                        ty: ty.clone(),
                        values,
                    };
                    let buffer = match &original {
                        Value::List { values, .. } => values.as_ptr() as usize,
                        _ => unreachable!(),
                    };
                    let mut env = Environment::new();
                    let storage = if carrier == 0 {
                        env.insert(suffix, original.clone());
                        env.insert(input, original);
                        None
                    } else {
                        let storage =
                            Storage::new(Value::AnonymousRecord(vec![("items".into(), original)]));
                        let reference = value_reference::ValueReference::projected(
                            storage.clone(),
                            &[(nir::program::FieldOwner::Anonymous, 0)],
                            &mut 1000,
                        )
                        .unwrap();
                        env.insert(input, Value::ComptimeReference(reference.clone()));
                        env.insert(suffix, Value::ComptimeReference(reference));
                        Some(storage)
                    };
                    // A root storage carrier uses the same append mechanism as a projected receiver.
                    if carrier == 2 {
                        let value = match &env[&input] {
                            Value::ComptimeReference(reference) => {
                                reference.borrow(&mut 1000).unwrap().value().clone()
                            }
                            _ => unreachable!(),
                        };
                        let root = Storage::new(value);
                        env.insert(input, Value::Storage(root.clone()));
                        env.insert(suffix, Value::Storage(root));
                    }
                    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                    assert!(matches!(
                        run(&mut vm, &body, &mut env, compiled).unwrap(),
                        Flow::Value(Value::Unit)
                    ));
                    let actual = vm
                        .invocation_slot_value(&env[&input], Some(SOURCE))
                        .unwrap();
                    match actual {
                        Value::List { values, .. } => assert_eq!(values, vec![item; 2]),
                        _ => panic!("list expected"),
                    }
                    if carrier == 0 {
                        let after = match &env[&input] {
                            Value::List { values, .. } => values.as_ptr() as usize,
                            _ => unreachable!(),
                        };
                        assert_eq!(
                            after, buffer,
                            "owned list must reuse its existing allocation"
                        );
                    }
                    drop(storage);
                }
            }
        }
    }
}

#[test]
fn fused_append_rolls_back_budget_failures_and_recovers() {
    let package = package();
    let ty = list(Primitive::Bool);
    let original = Value::List {
        ty: ty.clone(),
        values: vec![Value::Bool(false)],
    };
    let suffix = Value::List {
        ty: ty.clone(),
        values: vec![Value::Bool(true)],
    };
    let mut failed = 0;
    let mut succeeded = 0;
    for steps in 0..100 {
        for shared in [false, true] {
            let mut target = if shared {
                Value::Storage(Storage::new(original.clone()))
            } else {
                original.clone()
            };
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            vm.budget.steps = steps;
            let result = vm.complete_list_append(&mut target, suffix.clone(), &ty, SOURCE);
            vm.budget = Budget::default();
            let actual = vm.invocation_slot_value(&target, Some(SOURCE)).unwrap();
            if let Err(error) = result {
                assert_eq!(error.kind, ErrorKind::StepsExhausted);
                failed += 1;
                assert_eq!(actual, original);
                vm.complete_list_append(&mut target, suffix.clone(), &ty, SOURCE)
                    .unwrap();
            } else {
                succeeded += 1;
            }
            assert_eq!(
                vm.invocation_slot_value(&target, Some(SOURCE)).unwrap(),
                Value::List {
                    ty: ty.clone(),
                    values: vec![Value::Bool(false), Value::Bool(true)],
                }
            );
        }
    }
    assert!(failed > 0 && succeeded > 0);
}

#[test]
fn fused_append_rejects_invalid_suffix_without_replacing_receiver() {
    let package = package();
    let input = LocalId { owner: 1, index: 0 };
    let suffix = LocalId { owner: 1, index: 1 };
    let ty = list(Primitive::Bool);
    let original = Value::List {
        ty: ty.clone(),
        values: vec![Value::Bool(false)],
    };
    let body = expr(
        Term::Primitive(Primitive::Void),
        K::Assign {
            place: Place {
                root: input,
                fields: vec![],
            },
            value: Box::new(expr(
                ty.clone(),
                K::ListConcat(
                    Box::new(expr(ty.clone(), K::Local(input))),
                    Box::new(expr(ty.clone(), K::Local(suffix))),
                ),
            )),
        },
    );
    for compiled in [false, true] {
        for malformed in [
            Value::List {
                ty: list(Primitive::Char),
                values: vec![Value::Char('x')],
            },
            Value::List {
                ty: ty.clone(),
                values: vec![Value::Char('x')],
            },
        ] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env = Environment::from([(input, original.clone()), (suffix, malformed)]);
            assert_eq!(
                run(&mut vm, &body, &mut env, compiled).unwrap_err().kind,
                ErrorKind::Type
            );
            assert_eq!(env[&input], original);
            env.insert(suffix, original.clone());
            run(&mut vm, &body, &mut env, compiled).unwrap();
            assert_eq!(
                env[&input],
                Value::List {
                    ty: ty.clone(),
                    values: vec![Value::Bool(false); 2]
                }
            );
        }
    }
}

#[test]
fn frozen_append_checks_suffix_before_mutation_and_recovers_at_every_budget_boundary() {
    let package = package();
    let ty = list(Primitive::Bool);
    let input = LocalId { owner: 1, index: 0 };
    let original = Value::List {
        ty: ty.clone(),
        values: vec![Value::Bool(false)],
    };
    let body = |fields| {
        expr(
            Term::Primitive(Primitive::Void),
            K::Assign {
                place: Place {
                    root: input,
                    fields: vec![],
                },
                value: Box::new(expr(
                    ty.clone(),
                    K::ListConcat(
                        Box::new(expr(ty.clone(), K::Local(input))),
                        Box::new(expr(
                            ty.clone(),
                            K::ComptimeValue(Term::NominalValue {
                                ty: Box::new(ty.clone()),
                                variant: None,
                                fields,
                            }),
                        )),
                    ),
                )),
            },
        )
    };
    let valid = body(vec![Term::Bool(true)]);
    let malformed = body(vec![Term::Char('x')]);
    for compiled in [false, true] {
        for shared in [false, true] {
            let mut failed = 0;
            let mut succeeded = 0;
            for steps in 0..500 {
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let target = if shared {
                    Value::Storage(Storage::new(original.clone()))
                } else {
                    original.clone()
                };
                let mut env = Environment::from([(input, target)]);
                assert_eq!(
                    run(&mut vm, &malformed, &mut env, compiled)
                        .unwrap_err()
                        .kind,
                    ErrorKind::Type
                );
                assert_eq!(
                    vm.invocation_slot_value(&env[&input], Some(SOURCE))
                        .unwrap(),
                    original
                );
                vm.budget.steps = steps;
                let result = run(&mut vm, &valid, &mut env, compiled);
                vm.budget = Budget::default();
                match result {
                    Err(error) => {
                        assert_eq!(error.kind, ErrorKind::StepsExhausted);
                        failed += 1;
                        let actual = vm
                            .invocation_slot_value(&env[&input], Some(SOURCE))
                            .unwrap();
                        let completed = Value::List {
                            ty: ty.clone(),
                            values: vec![Value::Bool(false), Value::Bool(true)],
                        };
                        // Bytecode can exhaust its budget after committing the
                        // append, while producing the enclosing Unit result.
                        // The operation is atomic; the invocation is not a transaction.
                        assert!(actual == original || actual == completed);
                        env.insert(
                            input,
                            if shared {
                                Value::Storage(Storage::new(original.clone()))
                            } else {
                                original.clone()
                            },
                        );
                        run(&mut vm, &valid, &mut env, compiled).unwrap();
                    }
                    Ok(Flow::Value(Value::Unit)) => succeeded += 1,
                    other => panic!("unexpected append result: {other:?}"),
                }
                assert_eq!(
                    vm.invocation_slot_value(&env[&input], Some(SOURCE))
                        .unwrap(),
                    Value::List {
                        ty: ty.clone(),
                        values: vec![Value::Bool(false), Value::Bool(true)]
                    }
                );
            }
            assert!(failed > 0 && succeeded > 0);
        }
    }
}

#[test]
fn append_rejects_malformed_prefix_before_reading_suffix_for_every_carrier() {
    let package = package();
    let input = LocalId { owner: 1, index: 0 };
    let suffix = LocalId { owner: 1, index: 1 };
    let ty = list(Primitive::Bool);
    let body = expr(
        Term::Primitive(Primitive::Void),
        K::Assign {
            place: Place {
                root: input,
                fields: vec![],
            },
            value: Box::new(expr(
                ty.clone(),
                K::ListConcat(
                    Box::new(expr(ty.clone(), K::Local(input))),
                    Box::new(expr(ty.clone(), K::Local(suffix))),
                ),
            )),
        },
    );
    for compiled in [false, true] {
        for carrier in 0..3 {
            let wrap = |value| match carrier {
                0 => value,
                1 => Value::Storage(Storage::new(value)),
                _ => Value::ComptimeReference(
                    value_reference::ValueReference::projected(
                        Storage::new(Value::AnonymousRecord(vec![("items".into(), value)])),
                        &[(nir::program::FieldOwner::Anonymous, 0)],
                        &mut 1000,
                    )
                    .unwrap(),
                ),
            };
            let malformed = Value::List {
                ty: ty.clone(),
                values: vec![Value::Char('x')],
            };
            let mut env = Environment::from([(input, wrap(malformed.clone()))]);
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            // The missing suffix binding would produce MissingBinding if read.
            assert_eq!(
                run(&mut vm, &body, &mut env, compiled).unwrap_err().kind,
                ErrorKind::Type
            );
            assert_eq!(
                vm.invocation_slot_value(&env[&input], Some(SOURCE))
                    .unwrap(),
                malformed
            );
            let valid = Value::List {
                ty: ty.clone(),
                values: vec![Value::Bool(true)],
            };
            env.insert(input, wrap(valid.clone()));
            env.insert(suffix, valid);
            run(&mut vm, &body, &mut env, compiled).unwrap();
            assert_eq!(
                vm.invocation_slot_value(&env[&input], Some(SOURCE))
                    .unwrap(),
                Value::List {
                    ty: ty.clone(),
                    values: vec![Value::Bool(true); 2]
                }
            );
        }
    }
}
