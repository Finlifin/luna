use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Package, SourceRange};
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn p(value: P) -> Term {
    Term::Primitive(value)
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 1, end: 2 },
    }
}
fn value(ty: Term) -> Expr {
    expr(p(P::Type), K::TypeValue(ty))
}
fn function(local: u32, body: Expr) -> Function {
    Function {
        declaration: id(local),
        name: format!("Type{local}"),
        phase: FunctionPhase::Comptime,
        inline: false,
        pure: false,
        is_unsafe: false,
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(p(P::Type)),
        },
        parameters: vec![],
        defaults: vec![],
        body,
    }
}
fn call(local: u32) -> Expr {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(p(P::Type)),
    }
    .with_comptime_phase(true);
    expr(
        p(P::Type),
        K::Call {
            parameter_bindings: vec![],
            callee: Box::new(expr(signature, K::Function(id(local)))),
            arguments: vec![],
        },
    )
}

fn record(requirements: Vec<nir::program::TypePredicate>) -> nir::program::Struct {
    nir::program::Struct {
        parameter_types: vec![],
        declaration: id(20),
        name: "Box".into(),
        type_parameters: vec![id(9)],
        requirements,
        capabilities: vec![],
        fields: vec![(
            "value".into(),
            Term::Rigid {
                declaration: id(9),
                sort: Sort::Type,
            },
        )],
    }
}
fn nominal(argument: Expr) -> Expr {
    expr(
        p(P::Type),
        K::NominalType {
            preparations: vec![],
            bindings: None,
            order: None,
            declaration: id(20),
            arguments: vec![nir::program::NormalFormArgument::Value(argument)],
        },
    )
}

#[test]
fn evaluated_arguments_are_checked_against_the_nominal_contract_before_escape() {
    let package = Package {
        structs: vec![record(vec![nir::program::TypePredicate::Equal(
            Term::Rigid {
                declaration: id(9),
                sort: Sort::Type,
            },
            p(P::Bool),
        )])],
        functions: vec![
            function(0, nominal(call(1))),
            function(1, value(p(P::Char))),
            function(2, nominal(value(p(P::Bool)))),
        ],
        ..Default::default()
    };
    let mut interpreter = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::TypeContractUnresolved
    );
    let entered: Vec<_> = interpreter
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(entered, vec![id(0), id(1)]);
    assert_eq!(
        interpreter.invoke(id(2), vec![]).unwrap(),
        Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![p(P::Bool)]
        })
    );
}

#[test]
fn missing_descriptors_arity_and_non_type_inputs_do_not_construct_nominals() {
    for body in [
        expr(
            p(P::Type),
            K::NominalType {
                preparations: vec![],
                bindings: None,
                order: None,
                declaration: id(99),
                arguments: vec![nir::program::NormalFormArgument::Value(value(p(P::Bool)))],
            },
        ),
        expr(
            p(P::Type),
            K::NominalType {
                preparations: vec![],
                bindings: None,
                order: None,
                declaration: id(20),
                arguments: vec![],
            },
        ),
        nominal(expr(
            p(P::Bool),
            K::Literal(nir::program::Literal::Bool(true)),
        )),
        nominal(value(Term::Bool(true))),
    ] {
        let package = Package {
            structs: vec![record(vec![])],
            functions: vec![function(0, body), function(1, nominal(value(p(P::Bool))))],
            ..Default::default()
        };
        let mut interpreter = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            interpreter.invoke(id(0), vec![]).unwrap_err().kind,
            ErrorKind::Type
        );
        assert!(interpreter.invoke(id(1), vec![]).is_ok());
    }
}

#[test]
fn specialization_preserves_the_template_and_bounded_construction_recovers() {
    let body = nominal(value(Term::Rigid {
        declaration: id(9),
        sort: Sort::Type,
    }));
    let specialized = body.instantiate_types(&[id(9)], &[p(P::Char)]).unwrap();
    assert_ne!(body, specialized);
    let package = Package {
        structs: vec![record(vec![])],
        functions: vec![function(0, specialized)],
        ..Default::default()
    };
    let mut bounded = Interpreter::new(&package, Budget::default()).unwrap();
    bounded
        .charge_preparation((bounded.remaining_steps() - 1) as usize)
        .unwrap();
    assert_eq!(
        bounded.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(
        Interpreter::new(&package, Budget::default())
            .unwrap()
            .invoke(id(0), vec![])
            .unwrap(),
        Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![p(P::Char)]
        })
    );
}

#[test]
fn transparent_alias_guards_check_before_body_and_do_not_produce_nominal_values() {
    for allowed in [true, false] {
        let guard = expr(
            p(P::Void),
            K::CheckTypeAlias {
                preparations: vec![],
                bindings: None,
                order: None,
                body: None,
                declaration: id(20),
                arguments: vec![call(2)]
                    .into_iter()
                    .map(nir::program::NormalFormArgument::Value)
                    .collect(),
            },
        );
        let package = Package {
            alias_contracts: vec![nir::type_aliases::Contract {
                declaration: id(20),
                parameters: vec![id(9)],
                parameter_types: vec![],
                requirements: vec![nir::program::TypePredicate::Bool(allowed)],
                capabilities: vec![],
            }],
            functions: vec![
                function(
                    1,
                    expr(
                        p(P::Type),
                        K::Block {
                            statements: vec![guard],
                            result: Some(Box::new(call(3))),
                        },
                    ),
                ),
                function(2, value(p(P::Bool))),
                function(3, value(p(P::Char))),
            ],
            ..Default::default()
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let result = vm.invoke(id(1), vec![]);
        if allowed {
            assert_eq!(result.unwrap(), Value::Type(p(P::Char)));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractRejected);
        }
        let calls: Vec<_> = vm
            .events()
            .iter()
            .filter_map(|event| match event {
                Event::EnterFunction(id) => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            if allowed {
                vec![id(1), id(2), id(3)]
            } else {
                vec![id(1), id(2)]
            }
        );
    }
}

#[test]
fn alias_guards_reject_runtime_placement_and_malformed_inputs() {
    let contract = nir::type_aliases::Contract {
        declaration: id(20),
        parameters: vec![id(9)],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
    };
    let guard = expr(
        p(P::Void),
        K::CheckTypeAlias {
            preparations: vec![],
            bindings: None,
            order: None,
            body: None,
            declaration: id(20),
            arguments: vec![value(p(P::Bool))]
                .into_iter()
                .map(nir::program::NormalFormArgument::Value)
                .collect(),
        },
    );
    let mut package = Package {
        alias_contracts: vec![contract],
        functions: vec![function(
            1,
            expr(
                p(P::Type),
                K::Block {
                    statements: vec![guard],
                    result: Some(Box::new(value(p(P::Bool)))),
                },
            ),
        )],
        ..Default::default()
    };
    assert!(nir::phases::validate(&package).is_ok());
    package.functions[0].phase = FunctionPhase::Runtime;
    assert!(nir::phases::validate(&package).is_err());
    package.functions[0].phase = FunctionPhase::Comptime;
    let K::Block { statements, .. } = &mut package.functions[0].body.kind else {
        unreachable!()
    };
    let K::CheckTypeAlias { arguments, .. } = &mut statements[0].kind else {
        unreachable!()
    };
    arguments.clear();
    assert!(nir::phases::validate(&package).is_err());
    package.alias_contracts.clear();
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn nominal_constructor_checks_value_domains_before_export_and_recovers() {
    let mut descriptor = record(vec![]);
    descriptor.parameter_types = vec![p(P::U8)];
    descriptor.fields.clear();
    let integer = |n: u128| {
        expr(
            p(P::Type),
            K::NominalType {
                preparations: vec![],
                bindings: None,
                order: None,
                declaration: id(20),
                arguments: vec![nir::program::NormalFormArgument::Integer {
                    source: SourceRange { start: 1, end: 2 },
                    spelling: n.to_string(),
                    negative: false,
                }],
            },
        )
    };
    let package = Package {
        structs: vec![descriptor],
        functions: vec![function(0, integer(256)), function(1, integer(255))],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(0), vec![]).unwrap_err().kind, ErrorKind::Type);
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap(),
        Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::Integer(comptime_runtime::Integer::from_u128(255))]
        })
    );
}

#[test]
fn nominal_constructor_value_indices_keep_contract_substitution_sorts() {
    let mut descriptor = record(vec![nir::program::TypePredicate::Equal(
        Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::Rigid {
                declaration: id(9),
                sort: Sort::ComptimeValue,
            }],
        },
        Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::Bool(true)],
        },
    )]);
    descriptor.parameter_types = vec![p(P::Bool)];
    descriptor.fields.clear();
    let argument = |flag| expr(p(P::Bool), K::Literal(nir::program::Literal::Bool(flag)));
    let package = Package {
        structs: vec![descriptor],
        functions: vec![
            function(0, nominal(argument(false))),
            function(1, nominal(argument(true))),
        ],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::TypeContractUnresolved
    );
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap(),
        Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::Bool(true)]
        })
    );
}

#[test]
fn nominal_record_index_budget_boundaries_do_not_report_type_errors() {
    let name = "long_record_field_name".to_string();
    let domain = Term::Record(vec![(name, p(P::Bool))]);
    let mut descriptor = record(vec![]);
    descriptor.fields.clear();
    descriptor.parameter_types = vec![domain.clone()];
    let argument = expr(
        domain,
        K::AnonymousRecord {
            fields: vec![expr(
                p(P::Bool),
                K::Literal(nir::program::Literal::Bool(true)),
            )],
        },
    );
    let package = Package {
        structs: vec![descriptor],
        functions: vec![function(0, nominal(argument))],
        ..Default::default()
    };
    let mut failed = false;
    let mut succeeded = false;
    for steps in 1..512 {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.charge_preparation((vm.remaining_steps() - steps) as usize)
            .unwrap();
        match vm.invoke(id(0), vec![]) {
            Ok(Value::Type(_)) => succeeded = true,
            Err(error) => {
                assert_eq!(error.kind, ErrorKind::StepsExhausted, "budget {steps}");
                failed = true;
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
    assert!(failed && succeeded);
}

#[test]
fn contextual_nominal_input_waits_for_capture_without_replaying_effects() {
    use nir::program::NormalFormArgument as A;
    let source = SourceRange { start: 1, end: 2 };
    let argument = A::Block {
        source,
        statements: vec![call(2)],
        result: Some(Box::new(A::Integer {
            source,
            spelling: "7".into(),
            negative: false,
        })),
    };
    let body = expr(
        p(P::Type),
        K::NominalType {
            preparations: vec![],
            bindings: None,
            order: None,
            declaration: id(20),
            arguments: vec![argument, A::Value(call(1))],
        },
    );
    let dump = body.to_string();
    assert!(dump.contains("(nominal-type-value"));
    assert!(dump.contains("(contextual-block-input"));
    assert!(dump.contains("(contextual-integer-input"));
    let package = Package {
        newtypes: vec![nir::newtypes::Newtype {
            declaration: id(20),
            name: "Captured".into(),
            type_parameters: vec![id(9), id(10)],
            parameter_types: vec![
                Term::Rigid {
                    declaration: id(10),
                    sort: Sort::Type,
                },
                p(P::Type),
            ],
            capture_count: 1,
            representation: p(P::Bool),
            requirements: vec![],
            capabilities: vec![],
        }],
        functions: vec![
            function(0, body),
            function(1, value(p(P::U8))),
            function(2, value(p(P::Bool))),
        ],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(0), vec![]).unwrap(),
        Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![
                Term::Integer(comptime_runtime::Integer::from_u128(7)),
                p(P::U8)
            ]
        })
    );
    let entered = vm
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(entered, vec![id(0), id(2), id(1)]);
}

#[test]
fn nominal_input_order_rejects_malformed_permutations_before_effects_and_recovers() {
    for order in [vec![], vec![1], vec![0, 0], vec![usize::MAX]] {
        let package = Package {
            structs: vec![record(vec![])],
            functions: vec![
                function(
                    0,
                    expr(
                        p(P::Type),
                        K::NominalType {
                            preparations: vec![],
                            bindings: None,
                            order: Some(order),
                            declaration: id(20),
                            arguments: vec![nir::program::NormalFormArgument::Value(call(2))],
                        },
                    ),
                ),
                function(1, nominal(value(p(P::Bool)))),
                function(2, value(p(P::Bool))),
            ],
            ..Default::default()
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(vm.invoke(id(0), vec![]).unwrap_err().kind, ErrorKind::Type);
        assert!(
            !vm.events()
                .iter()
                .any(|event| matches!(event, Event::EnterFunction(callee) if *callee == id(2)))
        );
        assert!(matches!(vm.invoke(id(1), vec![]).unwrap(), Value::Type(_)));
    }
}

#[test]
fn nominal_input_bindings_allow_repeated_construction_and_invocation() {
    use nir::program::{LocalId, NormalFormArgument as A};
    let binding = LocalId {
        owner: 0,
        index: 99,
    };
    for argument in [
        A::Value(value(p(P::Bool))),
        A::Integer {
            source: SourceRange { start: 1, end: 2 },
            spelling: "7".into(),
            negative: false,
        },
        A::Value(expr(
            p(P::Never),
            K::Return(Some(Box::new(value(p(P::Bool))))),
        )),
    ] {
        let package = Package {
            structs: vec![record(vec![])],
            functions: vec![function(
                0,
                expr(
                    if argument.is_diverging() {
                        p(P::Never)
                    } else {
                        p(P::Type)
                    },
                    K::NominalType {
                        preparations: vec![],
                        bindings: Some(vec![binding]),
                        order: None,
                        declaration: id(20),
                        arguments: vec![argument],
                    },
                ),
            )],
            ..Default::default()
        };
        let mut package = package;
        let constructor = package.functions[0].body.clone();
        package.functions[0].body = expr(
            constructor.ty.clone(),
            K::Block {
                statements: vec![constructor.clone()],
                result: Some(Box::new(constructor)),
            },
        );
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        // The second constructor shares the first one's local environment;
        // repeated invocations also exercise recovery after a rejected input.
        let first = vm.invoke(id(0), vec![]);
        let second = vm.invoke(id(0), vec![]);
        match (first, second) {
            (Ok(left), Ok(right)) => assert_eq!(left, right),
            (Err(left), Err(right)) => {
                assert_eq!(left.kind, ErrorKind::Type);
                assert_eq!(right.kind, ErrorKind::Type);
            }
            other => panic!("inconsistent invocation: {other:?}"),
        }
    }
}

#[test]
fn alias_value_guards_instantiate_dependent_domains_and_preserve_transparency() {
    for abstract_annotation in [false, true] {
        for (domain, flag, expected_error) in [
            (P::Bool, true, None),
            (P::Bool, false, Some(ErrorKind::TypeContractUnresolved)),
            (P::Char, true, Some(ErrorKind::Type)),
        ] {
            let contract = nir::type_aliases::Contract {
                declaration: id(20),
                parameters: vec![id(8), id(9)],
                parameter_types: vec![
                    p(P::Type),
                    Term::Rigid {
                        declaration: id(8),
                        sort: Sort::Type,
                    },
                ],
                requirements: vec![nir::program::TypePredicate::Equal(
                    Term::Nominal {
                        declaration: id(40),
                        arguments: vec![Term::Rigid {
                            declaration: id(9),
                            sort: Sort::ComptimeValue,
                        }],
                    },
                    Term::Nominal {
                        declaration: id(40),
                        arguments: vec![Term::Bool(true)],
                    },
                )],
                capabilities: vec![],
            };
            let guard = expr(
                p(P::Void),
                K::CheckTypeAlias {
                    preparations: vec![],
                    bindings: None,
                    order: None,
                    body: None,
                    declaration: id(20),
                    arguments: vec![
                        value(p(domain)),
                        expr(
                            if abstract_annotation {
                                Term::Rigid {
                                    declaration: id(8),
                                    sort: Sort::Type,
                                }
                            } else {
                                p(P::Bool)
                            },
                            K::Literal(nir::program::Literal::Bool(flag)),
                        ),
                    ]
                    .into_iter()
                    .map(nir::program::NormalFormArgument::Value)
                    .collect(),
                },
            );
            let mut indexed = record(vec![]);
            indexed.declaration = id(40);
            // The indexed nominal and alias own distinct parameter identities.
            indexed.type_parameters = vec![id(41)];
            indexed.parameter_types = vec![p(P::Bool)];
            indexed.fields.clear();
            let package = Package {
                structs: vec![indexed],
                alias_contracts: vec![contract],
                functions: vec![
                    function(
                        1,
                        expr(
                            p(P::Type),
                            K::Block {
                                statements: vec![guard],
                                result: Some(Box::new(call(3))),
                            },
                        ),
                    ),
                    function(3, value(p(P::Char))),
                ],
                ..Default::default()
            };
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let result = vm.invoke(id(1), vec![]);
            if let Some(expected_error) = expected_error {
                assert_eq!(result.unwrap_err().kind, expected_error);
                assert!(
                    !vm.events().iter().any(
                        |event| matches!(event, Event::EnterFunction(found) if *found == id(3))
                    )
                );
            } else {
                assert_eq!(result.unwrap(), Value::Type(p(P::Char)));
            }
        }
    }
}

#[test]
fn alias_argument_specialization_charges_shared_closure_bodies() {
    use nir::program::{Literal, LocalId};
    let domain = Term::Rigid {
        declaration: id(8),
        sort: Sort::Type,
    };
    let closure = expr(
        Term::Function {
            parameters: vec![],
            result: Box::new(p(P::Str)),
        }
        .with_comptime_phase(true),
        K::Closure {
            parameters: vec![],
            captures: vec![],
            body: std::sync::Arc::new(expr(
                p(P::Str),
                K::Literal(Literal::String("x".repeat(4096))),
            )),
        },
    );
    let argument = expr(
        domain.clone(),
        K::Block {
            statements: vec![expr(
                p(P::Void),
                K::Let {
                    binding: LocalId {
                        owner: 1,
                        index: 99,
                    },
                    value: Box::new(closure),
                },
            )],
            result: Some(Box::new(expr(
                domain.clone(),
                K::Literal(Literal::Bool(true)),
            ))),
        },
    );
    let guard = expr(
        p(P::Void),
        K::CheckTypeAlias {
            preparations: vec![],
            bindings: None,
            order: None,
            body: None,
            declaration: id(20),
            arguments: vec![value(p(P::Bool)), argument]
                .into_iter()
                .map(nir::program::NormalFormArgument::Value)
                .collect(),
        },
    );
    let package = Package {
        alias_contracts: vec![nir::type_aliases::Contract {
            declaration: id(20),
            parameters: vec![id(8), id(9)],
            parameter_types: vec![p(P::Type), domain],
            requirements: vec![],
            capabilities: vec![],
        }],
        functions: vec![function(
            1,
            expr(
                p(P::Type),
                K::Block {
                    statements: vec![guard],
                    result: Some(Box::new(value(p(P::Bool)))),
                },
            ),
        )],
        ..Default::default()
    };
    let mut low = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    low.charge_preparation((low.remaining_steps() - 512) as usize)
        .unwrap();
    assert_eq!(
        low.invoke(id(1), vec![]).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    let mut enough = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for _ in 0..2 {
        assert_eq!(
            enough.invoke(id(1), vec![]).unwrap(),
            Value::Type(p(P::Bool))
        );
    }
}

#[test]
fn alias_body_specialization_happens_after_admission_and_obeys_budget() {
    use nir::program::{Literal, LocalId};
    for allowed in [false, true] {
        let closure = expr(
            Term::Function {
                parameters: vec![],
                result: Box::new(p(P::Str)),
            }
            .with_comptime_phase(true),
            K::Closure {
                parameters: vec![],
                captures: vec![],
                body: std::sync::Arc::new(expr(
                    p(P::Str),
                    K::Literal(Literal::String("x".repeat(4096))),
                )),
            },
        );
        let body = expr(
            p(P::Type),
            K::Block {
                statements: vec![expr(
                    p(P::Void),
                    K::Let {
                        binding: LocalId {
                            owner: 1,
                            index: 99,
                        },
                        value: Box::new(closure),
                    },
                )],
                result: Some(Box::new(value(p(P::Bool)))),
            },
        );
        let guard = expr(
            p(P::Type),
            K::CheckTypeAlias {
                preparations: vec![],
                bindings: None,
                order: None,
                declaration: id(20),
                arguments: vec![]
                    .into_iter()
                    .map(nir::program::NormalFormArgument::Value)
                    .collect(),
                body: Some(Box::new(body)),
            },
        );
        let package = Package {
            alias_contracts: vec![nir::type_aliases::Contract {
                declaration: id(20),
                parameters: vec![],
                parameter_types: vec![],
                requirements: vec![nir::program::TypePredicate::Bool(allowed)],
                capabilities: vec![],
            }],
            functions: vec![function(1, guard)],
            ..Default::default()
        };
        let mut low = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        low.charge_preparation((low.remaining_steps() - 512) as usize)
            .unwrap();
        assert_eq!(
            low.invoke(id(1), vec![]).unwrap_err().kind,
            if allowed {
                ErrorKind::StepsExhausted
            } else {
                ErrorKind::TypeContractRejected
            }
        );
        if allowed {
            let mut enough = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            for _ in 0..2 {
                assert_eq!(
                    enough.invoke(id(1), vec![]).unwrap(),
                    Value::Type(p(P::Bool))
                );
            }
        }
    }
}

#[test]
fn captured_value_reads_validate_domain_and_recover_after_failure() {
    let mut package = Package::default();
    for (index, domain, value) in [
        (70, p(P::Bool), Term::Bool(true)),
        (71, p(P::Bool), Term::Char('x')),
        (
            72,
            p(P::U8),
            Term::Integer(comptime_runtime::Integer::from_i128(256)),
        ),
        (
            73,
            p(P::Bool),
            Term::Rigid {
                declaration: id(99),
                sort: Sort::ComptimeValue,
            },
        ),
    ] {
        let mut f = function(index, expr(domain.clone(), K::ComptimeValue(value)));
        f.signature = Term::Function {
            parameters: vec![],
            result: Box::new(domain),
        };
        if matches!(index, 71 | 72) {
            let malformed = Package {
                functions: vec![f],
                ..Default::default()
            };
            assert!(Interpreter::for_comptime(&malformed, Budget::default()).is_err());
        } else {
            package.functions.push(f);
        }
    }
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for bad in [73, 73] {
        assert!(vm.invoke(id(bad), vec![]).is_err());
        assert_eq!(vm.invoke(id(70), vec![]).unwrap(), Value::Bool(true));
    }
}

#[test]
fn captured_values_check_specialized_domains_without_poisoning_later_calls() {
    let mut holder = record(vec![]);
    holder.parameter_types = vec![p(P::Bool)];
    holder.fields.clear();
    let mut read = function(
        80,
        expr(
            p(P::Bool),
            K::ComptimeValue(Term::Rigid {
                declaration: id(9),
                sort: Sort::ComptimeValue,
            }),
        ),
    );
    read.type_parameters = vec![id(9)];
    read.signature = Term::Function {
        parameters: vec![],
        result: Box::new(p(P::Bool)),
    };
    let mut unused = read.clone();
    unused.declaration = id(82);
    unused.body.kind = K::Literal(nir::program::Literal::Bool(true));
    let package = Package {
        structs: vec![holder],
        functions: vec![read, unused],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for bad in [Term::Char('x'), Term::Primitive(P::Bool)] {
        let events = vm.events().len();
        assert!(
            vm.invoke_instantiated(id(82), vec![bad.clone()], vec![])
                .is_err()
        );
        assert_eq!(vm.events().len(), events);
        assert!(vm.invoke_instantiated(id(80), vec![bad], vec![]).is_err());
        assert_eq!(
            vm.invoke_instantiated(id(80), vec![Term::Bool(true)], vec![])
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            vm.invoke_instantiated(id(80), vec![Term::Bool(false)], vec![])
                .unwrap(),
            Value::Bool(false)
        );
    }
}

#[test]
fn captured_value_payload_budget_is_not_a_type_error() {
    let mut read = function(
        81,
        expr(
            p(P::Str),
            K::ComptimeValue(Term::String("x".repeat(2048).into())),
        ),
    );
    read.signature = Term::Function {
        parameters: vec![],
        result: Box::new(p(P::Str)),
    };
    let package = Package {
        functions: vec![read],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 512,
            ..Budget::default()
        },
    )
    .unwrap();
    assert_eq!(
        vm.invoke(id(81), vec![]).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(81), vec![]).unwrap(),
        Value::String("x".repeat(2048).into())
    );
}

#[test]
fn unused_dependent_and_record_function_inputs_are_admitted_before_instantiation() {
    let byte = |n| Term::Integer(comptime_runtime::Integer::from_i128(n));
    let record_value =
        |flag, number| Term::RecordValue(vec![("flag".into(), flag), ("number".into(), number)]);
    for (domain, valid, invalid) in [
        (
            p(P::U8),
            byte(255),
            vec![byte(256), byte(-1), Term::Bool(true)],
        ),
        (
            Term::Record(vec![
                ("flag".into(), p(P::Bool)),
                ("number".into(), p(P::U8)),
            ]),
            record_value(Term::Bool(true), byte(255)),
            vec![
                record_value(Term::Char('x'), byte(255)),
                record_value(Term::Bool(true), byte(256)),
                Term::RecordValue(vec![]),
            ],
        ),
    ] {
        let mut holder = record(vec![]);
        holder.type_parameters = vec![id(8), id(9)];
        holder.parameter_types = vec![
            p(P::Type),
            Term::Rigid {
                declaration: id(8),
                sort: Sort::Type,
            },
        ];
        holder.fields.clear();
        let mut body = function(
            83,
            expr(p(P::Bool), K::Literal(nir::program::Literal::Bool(true))),
        );
        body.type_parameters = vec![id(8), id(9)];
        body.signature = Term::Function {
            parameters: vec![],
            result: Box::new(p(P::Bool)),
        };
        let package = Package {
            structs: vec![holder],
            functions: vec![body],
            ..Default::default()
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke_instantiated(id(83), vec![domain.clone(), valid.clone()], vec![])
                .unwrap(),
            Value::Bool(true)
        );
        for bad in invalid {
            let events = vm.events().len();
            assert!(
                vm.invoke_instantiated(id(83), vec![domain.clone(), bad], vec![])
                    .is_err()
            );
            assert_eq!(vm.events().len(), events);
            assert_eq!(
                vm.invoke_instantiated(id(83), vec![domain.clone(), valid.clone()], vec![])
                    .unwrap(),
                Value::Bool(true)
            );
        }
        assert_eq!(vm.events().iter().filter(|event| matches!(event,Event::InstantiateFunction {declaration,..} if *declaration==id(83))).count(),1);
    }
}

#[test]
fn newtype_only_metadata_admits_value_indexed_function_instances() {
    let mut body = function(
        84,
        expr(p(P::Bool), K::Literal(nir::program::Literal::Bool(true))),
    );
    body.type_parameters = vec![id(9)];
    body.signature = Term::Function {
        parameters: vec![],
        result: Box::new(p(P::Bool)),
    };
    let package = Package {
        newtypes: vec![nir::newtypes::Newtype {
            declaration: id(20),
            name: "Flag".into(),
            type_parameters: vec![id(9)],
            parameter_types: vec![p(P::Bool)],
            capture_count: 0,
            representation: p(P::Bool),
            requirements: vec![],
            capabilities: vec![],
        }],
        functions: vec![body],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for flag in [true, false, true] {
        let before = vm.events().len();
        assert!(
            vm.invoke_instantiated(id(84), vec![Term::Char('x')], vec![])
                .is_err()
        );
        assert_eq!(vm.events().len(), before);
        assert_eq!(
            vm.invoke_instantiated(id(84), vec![Term::Bool(flag)], vec![])
                .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn immutable_nominal_inputs_validate_mixed_domains_and_recover() {
    let mut descriptor = record(vec![]);
    descriptor.type_parameters = vec![id(8), id(9)];
    descriptor.parameter_types = vec![p(P::Type), p(P::U8)];
    descriptor.fields.clear();
    let package = Package {
        structs: vec![descriptor],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let good = vec![
        p(P::Bool),
        Term::Integer(comptime_runtime::Integer::from_u128(255)),
    ];
    for arguments in [
        vec![
            p(P::Bool),
            Term::Integer(comptime_runtime::Integer::from_u128(256)),
        ],
        vec![p(P::Bool), Term::Bool(true)],
        vec![Term::Bool(true), good[1].clone()],
        vec![p(P::Bool)],
        good.clone(),
    ] {
        let ty = Term::Nominal {
            declaration: id(20),
            arguments: arguments.clone(),
        };
        let expected = arguments == good;
        for (value, domain) in [
            (Value::Type(ty.clone()), p(P::Type)),
            (
                Value::Record {
                    declaration: id(20),
                    arguments,
                    fields: vec![],
                },
                ty,
            ),
        ] {
            let result = vm.validate_immutable_input(&value, &domain);
            if expected {
                result.unwrap();
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            }
        }
    }
}

#[test]
fn immutable_nominal_inputs_preserve_value_indexed_formation_contracts() {
    let mut descriptor = record(vec![nir::program::TypePredicate::Equal(
        Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::Rigid {
                declaration: id(9),
                sort: Sort::ComptimeValue,
            }],
        },
        Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::Bool(true)],
        },
    )]);
    descriptor.parameter_types = vec![p(P::Bool)];
    descriptor.fields.clear();
    let package = Package {
        structs: vec![descriptor],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for flag in [false, true, false, true] {
        let value = Value::Type(Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::Bool(flag)],
        });
        let result = vm.validate_immutable_input(&value, &p(P::Type));
        if flag {
            result.unwrap();
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractUnresolved);
        }
    }
}

#[test]
fn immutable_newtype_inputs_admit_captured_domains_before_own_values() {
    let package = Package {
        newtypes: vec![nir::newtypes::Newtype {
            declaration: id(20),
            name: "Captured".into(),
            type_parameters: vec![id(9), id(8)],
            parameter_types: vec![
                Term::Rigid {
                    declaration: id(8),
                    sort: Sort::Type,
                },
                p(P::Type),
            ],
            capture_count: 1,
            representation: p(P::Bool),
            requirements: vec![],
            capabilities: vec![],
        }],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (argument, domain, accepted) in [
        (Term::Bool(true), p(P::Bool), true),
        (Term::Bool(true), p(P::Char), false),
        (Term::Char('a'), p(P::Char), true),
    ] {
        let ty = Term::Nominal {
            declaration: id(20),
            arguments: vec![argument, domain],
        };
        for (value, expected) in [
            (Value::Type(ty.clone()), p(P::Type)),
            (
                Value::Newtype {
                    ty: ty.clone(),
                    value: Box::new(Value::Bool(true)),
                },
                ty,
            ),
        ] {
            let result = vm.validate_immutable_input(&value, &expected);
            if accepted {
                result.unwrap();
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            }
        }
    }
    assert!(vm.events().is_empty());
}

#[test]
fn immutable_captured_record_indices_check_payloads_and_inner_contracts() {
    for allowed in [true, false] {
        let mut config = record(vec![nir::program::TypePredicate::Bool(allowed)]);
        config.declaration = id(30);
        config.type_parameters.clear();
        config.fields = vec![("flag".into(), p(P::Bool))];
        let package = Package {
            structs: vec![config],
            newtypes: vec![nir::newtypes::Newtype {
                declaration: id(20),
                name: "Captured".into(),
                type_parameters: vec![id(9), id(8)],
                parameter_types: vec![
                    Term::Rigid {
                        declaration: id(8),
                        sort: Sort::Type,
                    },
                    p(P::Type),
                ],
                capture_count: 1,
                representation: p(P::Bool),
                requirements: vec![],
                capabilities: vec![],
            }],
            ..Default::default()
        };
        let config_ty = Term::Nominal {
            declaration: id(30),
            arguments: vec![],
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (fields, valid) in [
            (vec![Term::Char('x')], false),
            (vec![], false),
            (vec![Term::Bool(true)], true),
        ] {
            let ty = Term::Nominal {
                declaration: id(20),
                arguments: vec![
                    Term::NominalValue {
                        ty: Box::new(config_ty.clone()),
                        variant: None,
                        fields,
                    },
                    config_ty.clone(),
                ],
            };
            let result = vm.validate_immutable_input(&Value::Type(ty), &p(P::Type));
            if allowed && valid {
                result.unwrap();
            } else {
                assert_eq!(
                    result.unwrap_err().kind,
                    if valid {
                        ErrorKind::TypeContractRejected
                    } else {
                        ErrorKind::Type
                    }
                );
            }
        }
        assert!(vm.events().is_empty());
    }
}

#[test]
fn shared_alias_catalog_rechecks_proofs_before_executing_the_body() {
    use comptime_interpretor::{CodeSession, ContractOutcome};
    use nir::program::{TraitRequirement, TypePredicate};
    use std::sync::Arc;
    let guard = expr(
        p(P::Void),
        K::CheckTypeAlias {
            preparations: vec![],
            bindings: None,
            order: None,
            body: None,
            declaration: id(20),
            arguments: vec![],
        },
    );
    let package = Arc::new(Package {
        alias_contracts: vec![nir::type_aliases::Contract {
            declaration: id(20),
            parameters: vec![],
            parameter_types: vec![],
            requirements: vec![TypePredicate::Trait(TraitRequirement {
                interface: id(90),
                interface_arguments: vec![],
                subject: p(P::Bool),
            })],
            capabilities: vec![],
        }],
        functions: vec![
            function(
                1,
                expr(
                    p(P::Type),
                    K::Block {
                        statements: vec![guard],
                        result: Some(Box::new(call(3))),
                    },
                ),
            ),
            function(3, value(p(P::Char))),
        ],
        ..Default::default()
    });
    let mut session = CodeSession::new(package.clone());
    for outcome in [
        ContractOutcome::Satisfied,
        ContractOutcome::Rejected,
        ContractOutcome::Unknown,
        ContractOutcome::Satisfied,
    ] {
        let solver = |_: &TypePredicate| outcome;
        let mut vm =
            Interpreter::for_comptime_in_session(&package, &mut session, &mut Budget::default())
                .unwrap()
                .with_type_solver(&solver);
        let result = vm.with_code_session(&mut session, |vm| vm.invoke(id(1), vec![]));
        match outcome {
            ContractOutcome::Satisfied => assert_eq!(result.unwrap(), Value::Type(p(P::Char))),
            ContractOutcome::Rejected => {
                assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractRejected)
            }
            ContractOutcome::Unknown => {
                assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractUnresolved)
            }
        }
        let body_called = vm.events().iter().any(
            |event| matches!(event, Event::EnterFunction(declaration) if *declaration == id(3)),
        );
        assert_eq!(body_called, outcome == ContractOutcome::Satisfied);
    }
}
