use super::*;
const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
const INPUT: LocalId = LocalId { owner: 1, index: 0 };
const TRACE: LocalId = LocalId { owner: 1, index: 1 };
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty,
        kind,
    }
}
fn package() -> Package {
    let parameter = Term::Rigid {
        declaration: id(3),
        sort: nir::Sort::Type,
    };
    Package {
        structs: vec![nir::program::Struct {
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![id(3)],
            declaration: id(1),
            name: "Holder".into(),
            fields: vec![("item".into(), parameter.clone())],
        }],
        enums: vec![nir::program::Enum {
            is_intrinsic: false,
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![id(3)],
            declaration: id(2),
            name: "Choice".into(),
            variants: vec![
                nir::program::Variant {
                    name: "item".into(),
                    shape: nir::program::VariantShape::Tuple,
                    fields: vec![parameter],
                },
                nir::program::Variant {
                    name: "empty".into(),
                    shape: nir::program::VariantShape::Unit,
                    fields: vec![],
                },
            ],
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
        }],
        ..Default::default()
    }
}

#[test]
fn aggregates_observe_domains_before_fields_and_retain_completed_arguments() {
    let package = package();
    let boolean = Term::Primitive(Primitive::Bool);
    let projection = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(80),
            member: id(81),
        }),
        arguments: vec![],
        subject: Box::new(boolean.clone()),
    };
    let field = expr(
        projection.clone(),
        K::Block {
            statements: vec![expr(
                Term::Primitive(Primitive::Void),
                K::Assign {
                    place: nir::program::Place {
                        root: TRACE,
                        fields: vec![],
                    },
                    value: Box::new(expr(
                        boolean.clone(),
                        K::Literal(nir::program::Literal::Bool(true)),
                    )),
                },
            )],
            result: Some(Box::new(expr(projection.clone(), K::Local(INPUT)))),
        },
    );
    let bodies = [
        expr(
            Term::Record(vec![("item".into(), projection.clone())]),
            K::AnonymousRecord {
                fields: vec![field.clone()],
            },
        ),
        expr(
            Term::Nominal {
                declaration: id(1),
                arguments: vec![projection.clone()],
            },
            K::Record {
                declaration: id(1),
                fields: vec![field.clone()],
            },
        ),
        expr(
            Term::Nominal {
                declaration: id(2),
                arguments: vec![projection.clone()],
            },
            K::Variant {
                declaration: id(2),
                index: 0,
                fields: vec![field],
            },
        ),
        expr(
            Term::Nominal {
                declaration: id(2),
                arguments: vec![projection],
            },
            K::Variant {
                declaration: id(2),
                index: 1,
                fields: vec![],
            },
        ),
    ];
    for selected in [boolean.clone(), Term::Pointer(Box::new(boolean.clone()))] {
        let resolver = |_: &Expr, fuel: &mut u64| {
            *fuel -= 1;
            Ok(AssociatedSelection {
                initializer: nir::associated_initializers::AssociatedInitializer::new(
                    expr(
                        Term::Primitive(Primitive::Type),
                        K::TypeValue(selected.clone()),
                    ),
                    &mut 1000,
                )
                .unwrap(),
                requires_invocation: true,
            })
        };
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .with_associated_resolver(&resolver);
            for (index, body) in bodies.iter().enumerate() {
                let code = Arc::new(comptime_compiler::compile(body, &mut 10000).unwrap());
                assert!(code.to_string().contains("(prepare-aggregate %"));
                let run = |vm: &mut Interpreter<'_>, env: &mut Environment| {
                    if compiled {
                        vm.execute_code(&code, env)
                    } else {
                        vm.eval(body, env)
                    }
                };
                let mut env =
                    Environment::from([(INPUT, Value::Bool(true)), (TRACE, Value::Bool(false))]);
                if selected != boolean {
                    assert_eq!(
                        run(&mut vm, &mut env).unwrap_err().kind,
                        ErrorKind::PhaseMismatch
                    );
                    assert_eq!(env[&TRACE], Value::Bool(false));
                    continue;
                }
                vm.budget.steps = 0;
                assert_eq!(
                    run(&mut vm, &mut env).unwrap_err().kind,
                    ErrorKind::StepsExhausted
                );
                assert_eq!(env[&TRACE], Value::Bool(false));
                vm.budget.steps = Budget::default().steps;
                let expected = match index {
                    0 => Value::AnonymousRecord(vec![("item".into(), Value::Bool(true))]),
                    1 => Value::Record {
                        declaration: id(1),
                        arguments: vec![boolean.clone()],
                        fields: vec![Value::Bool(true)],
                    },
                    _ => Value::Variant {
                        declaration: id(2),
                        arguments: vec![boolean.clone()],
                        index: index - 2,
                        fields: if index == 2 {
                            vec![Value::Bool(true)]
                        } else {
                            vec![]
                        },
                    },
                };
                let Flow::Value(value) = run(&mut vm, &mut env).unwrap() else {
                    panic!()
                };
                assert_eq!(value, expected);
                assert_eq!(
                    vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                    Value::Bool(index != 3)
                );
                if index != 3 {
                    env.insert(INPUT, Value::Char('x'));
                    assert_eq!(run(&mut vm, &mut env).unwrap_err().kind, ErrorKind::Type);
                    env.insert(INPUT, Value::Bool(true));
                    let Flow::Value(value) = run(&mut vm, &mut env).unwrap() else {
                        panic!()
                    };
                    assert_eq!(value, expected);
                }
                assert!(vm.active_associated_initializers.is_empty());
            }
        }
    }
}

#[test]
fn compact_preparation_slots_isolate_nested_domains_and_failed_frames() {
    let package = Package::default();
    let boolean = Term::Primitive(Primitive::Bool);
    let optional = Term::Optional(Box::new(boolean.clone()));
    let record = Term::Record(vec![("item".into(), optional.clone())]);
    let outer = Term::Optional(Box::new(record.clone()));
    let body = expr(
        outer.clone(),
        K::OptionalPresent(Box::new(expr(
            record,
            K::AnonymousRecord {
                fields: vec![expr(
                    optional.clone(),
                    K::OptionalPresent(Box::new(expr(boolean.clone(), K::Local(INPUT)))),
                )],
            },
        ))),
    );
    let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
    assert_eq!(code.prepared_type_count(), 3);
    assert!(code.prepared_type_count() < code.registers());
    assert!(code.to_string().contains("(prepared-types"));
    let plain = comptime_compiler::compile(&expr(boolean, K::Local(INPUT)), &mut 1000).unwrap();
    assert_eq!(plain.prepared_type_count(), 0);
    assert!((0..plain.registers()).all(|register| plain.prepared_type_slot(register).is_none()));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for input in [Value::Bool(true), Value::Char('x'), Value::Bool(false)] {
        let mut env = Environment::from([(INPUT, input.clone())]);
        let result = vm.execute_code(&code, &mut env);
        if let Value::Bool(value) = input {
            let expected = Value::Optional {
                ty: outer.clone(),
                value: Some(Box::new(Value::AnonymousRecord(vec![(
                    "item".into(),
                    Value::Optional {
                        ty: optional.clone(),
                        value: Some(Box::new(Value::Bool(value))),
                    },
                )]))),
            };
            assert!(matches!(result.unwrap(), Flow::Value(actual) if actual == expected));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
        }
    }
}
