use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Literal, Package, SourceRange};
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn expression(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 1, end: 2 },
    }
}

fn package(body: Expr) -> Package {
    Package {
        functions: vec![Function {
            interface: nir::program::FunctionDeclaration {
                source: nir::program::SourceRange { start: 0, end: 0 },
                declaration: id(1),
                name: "inspect".into(),
                phase: FunctionPhase::Comptime,
                inline: false,
                pure: false,
                is_unsafe: false,
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![id(2)],
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Primitive(P::Bool)),
                },
                parameters: vec![],
                defaults: vec![],
            },
            body,
        }],
        ..Default::default()
    }
}

#[test]
fn expanded_instance_copies_fail_before_cache_publication() {
    let body = expression(
        Term::Primitive(P::Bool),
        K::Block {
            statements: (0..40)
                .map(|_| {
                    expression(
                        Term::Primitive(P::Type),
                        K::TypeValue(Term::Rigid {
                            declaration: id(2),
                            sort: Sort::Type,
                        }),
                    )
                })
                .collect(),
            result: Some(Box::new(expression(
                Term::Primitive(P::Bool),
                K::Literal(Literal::Bool(true)),
            ))),
        },
    );
    let package = package(body);
    let input = Term::Tuple(vec![Term::Primitive(P::Bool); 128]);
    let before = package.to_string();
    let mut bounded = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 3000,
            depth: 128,
        },
    )
    .unwrap();
    assert_eq!(
        bounded
            .invoke_instantiated(id(1), vec![], vec![])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert_eq!(
        bounded
            .invoke_instantiated(id(1), vec![input.clone()], vec![])
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert!(
        !bounded
            .events()
            .iter()
            .any(|event| matches!(event, Event::InstantiateFunction { .. }))
    );
    let mut complete = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 100_000,
            depth: 128,
        },
    )
    .unwrap();
    for _ in 0..2 {
        assert_eq!(
            complete
                .invoke_instantiated(id(1), vec![input.clone()], vec![])
                .unwrap(),
            Value::Bool(true)
        );
    }
    assert_eq!(
        complete
            .events()
            .iter()
            .filter(|event| matches!(event, Event::InstantiateFunction { .. }))
            .count(),
        1
    );
    assert_eq!(package.to_string(), before);
}

#[test]
fn cached_instances_preflight_program_contract_copies_on_each_call() {
    for postcondition in [false, true] {
        let mut package = package(expression(
            Term::Primitive(P::Bool),
            K::Literal(Literal::Bool(true)),
        ));
        let input = Term::Tuple(vec![Term::Primitive(P::Bool); 128]);
        let value_binding = nir::program::LocalId { owner: 1, index: 0 };
        let domain = Term::Rigid {
            declaration: id(2),
            sort: Sort::Type,
        };
        package.functions[0].parameters = vec![value_binding];
        package.functions[0].signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: domain.clone(),
            }],
            result: Box::new(Term::Primitive(P::Bool)),
        };
        // Ordinary value reads in a program predicate, not Type propositions.
        let mut condition = expression(
            Term::Primitive(P::Bool),
            K::Block {
                statements: (0..20)
                    .map(|_| expression(domain.clone(), K::Local(value_binding)))
                    .collect(),
                result: Some(Box::new(expression(
                    Term::Primitive(P::Bool),
                    K::Literal(Literal::Bool(true)),
                ))),
            },
        );
        condition.source = SourceRange { start: 10, end: 11 };
        if postcondition {
            package.functions[0].postconditions.push(condition);
        } else {
            package.functions[0].preconditions.push(condition);
        }
        nir::phases::validate(&package).unwrap();
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 100_000,
                depth: 128,
            },
        )
        .unwrap();
        let value = Value::Tuple(vec![Value::Bool(true); 128]);
        assert_eq!(
            vm.invoke_instantiated(id(1), vec![input.clone()], vec![value.clone()])
                .unwrap(),
            Value::Bool(true)
        );
        // Measure admission and input checking without program predicates.
        // A fixed allowance can expire earlier as those checks evolve, hiding
        // whether the cached invocation still preflights its contract copies.
        let mut baseline_package = package.clone();
        baseline_package.functions[0].preconditions.clear();
        baseline_package.functions[0].postconditions.clear();
        let mut baseline = Interpreter::for_comptime(
            &baseline_package,
            Budget {
                steps: 100_000,
                depth: 128,
            },
        )
        .unwrap();
        baseline
            .invoke_instantiated(id(1), vec![input.clone()], vec![value.clone()])
            .unwrap();
        let before = baseline.remaining_steps();
        baseline
            .invoke_instantiated(id(1), vec![input.clone()], vec![value.clone()])
            .unwrap();
        let admission = before - baseline.remaining_steps();
        vm.charge_preparation((vm.remaining_steps() - admission - 100) as usize)
            .unwrap();
        let error = vm
            .invoke_instantiated(id(1), vec![input], vec![value])
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(error.source, Some(SourceRange { start: 10, end: 11 }));
        assert_eq!(vm.events().iter().filter(|event| matches!(event,Event::InstantiateFunction {declaration,..} if *declaration==id(1))).count(),1);
    }
}

#[test]
fn scoped_worlds_keep_consumption_and_do_not_reuse_previous_instances() {
    let first = package(expression(
        Term::Primitive(P::Bool),
        K::Literal(Literal::Bool(true)),
    ));
    let second = package(expression(
        Term::Primitive(P::Bool),
        K::Literal(Literal::Bool(false)),
    ));
    let mut budget = Budget {
        steps: 100_000,
        depth: 42,
    };
    for (world, expected) in [(&first, true), (&second, false), (&first, true)] {
        let before = budget.steps;
        let mut remaining = 0;
        let value = Interpreter::with_comptime_in_budget(world, &[], &mut budget, |vm| {
            assert!(vm.events().is_empty());
            let result = vm.invoke_instantiated(id(1), vec![Term::Primitive(P::Bool)], vec![])?;
            remaining = vm.remaining_steps();
            Ok(result)
        })
        .unwrap();
        assert_eq!(value, Value::Bool(expected));
        assert_eq!(budget.steps, remaining);
        assert!(budget.steps < before);
        assert_eq!(budget.depth, 42);
    }
}

#[test]
fn scoped_invocation_errors_and_unwinding_keep_spent_fuel() {
    let world = package(expression(
        Term::Primitive(P::Bool),
        K::Literal(Literal::Bool(true)),
    ));
    let mut budget = Budget::default();
    let mut remaining = 0;
    let error = Interpreter::with_comptime_in_budget(&world, &[], &mut budget, |vm| {
        vm.charge_preparation(23)?;
        let result = vm.invoke_instantiated(id(1), vec![], vec![]);
        remaining = vm.remaining_steps();
        result
    })
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Type);
    assert_eq!(budget.steps, remaining);
    let before = budget.steps;
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Interpreter::with_comptime_in_budget::<()>(&world, &[], &mut budget, |vm| {
            vm.charge_preparation(31)?;
            remaining = vm.remaining_steps();
            panic!("host failure after preparation")
        })
    }));
    assert!(panicked.is_err());
    assert_eq!(budget.steps, remaining);
    assert!(budget.steps < before);
    assert_eq!(
        Interpreter::with_comptime_in_budget(&world, &[], &mut budget, |vm| vm
            .invoke_instantiated(id(1), vec![Term::Primitive(P::Bool)], vec![]))
        .unwrap(),
        Value::Bool(true)
    );
    let mut exhausted = Budget {
        steps: 0,
        depth: 42,
    };
    let mut invalid = world.clone();
    invalid.functions.push(invalid.functions[0].clone());
    let mut called = false;
    assert!(
        Interpreter::with_comptime_in_budget::<()>(&invalid, &[], &mut exhausted, |_| {
            called = true;
            Ok(())
        })
        .is_err()
    );
    assert!(!called);
    assert_eq!(exhausted.steps, 0);
    assert_eq!(exhausted.depth, 42);
}

#[test]
fn declaration_tables_are_bounded_before_world_indexing_and_phase_validation() {
    let mut world = package(expression(
        Term::Primitive(P::Bool),
        K::Literal(Literal::Bool(true)),
    ));
    world.functions[0].type_parameters.clear();
    let template = world.functions[0].clone();
    world.functions = (1..=128)
        .map(|index| {
            let mut function = template.clone();
            function.declaration = id(index);
            function
        })
        .collect();
    let before = world.to_string();
    for comptime in [false, true] {
        let budget = Budget {
            steps: 64,
            depth: 42,
        };
        let result = if comptime {
            Interpreter::for_comptime(&world, budget)
        } else {
            Interpreter::new(&world, budget)
        };
        assert_eq!(result.err().unwrap().kind, ErrorKind::StepsExhausted);
    }
    let mut budget = Budget {
        steps: 10_000,
        depth: 42,
    };
    let value = Interpreter::with_comptime_in_budget(&world, &[], &mut budget, |vm| {
        vm.invoke(id(128), vec![])
    })
    .unwrap();
    assert_eq!(value, Value::Bool(true));
    assert!(budget.steps < 10_000 - 128);
    assert_eq!(budget.depth, 42);
    assert_eq!(world.to_string(), before);

    // With a bounded allowance the malformed duplicate is diagnosed normally.
    // With no allowance neither indexing nor the phase walker should begin.
    world.functions.push(template);
    assert_eq!(
        Interpreter::for_comptime(
            &world,
            Budget {
                steps: 0,
                depth: 42
            }
        )
        .err()
        .unwrap()
        .kind,
        ErrorKind::StepsExhausted
    );
    let error = Interpreter::for_comptime(&world, Budget::default())
        .err()
        .unwrap();
    let ErrorKind::InvalidPhase(reason) = &error.kind else {
        panic!("construction lost its phase diagnostic: {error}");
    };
    assert_eq!(reason.message, "duplicate function declaration");
    assert_eq!(error.source, Some(reason.source));
    assert!(error.to_string().contains(reason.message));
    assert_eq!(
        Interpreter::new(&world, Budget::default())
            .err()
            .unwrap()
            .kind,
        ErrorKind::InvalidCallPlan
    );
}

#[test]
fn zero_budget_cannot_create_even_an_empty_scoped_world() {
    let world = Package::default();
    let mut budget = Budget {
        steps: 0,
        depth: 42,
    };
    let mut called = false;
    let error = Interpreter::with_comptime_in_budget(&world, &[], &mut budget, |_| {
        called = true;
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::StepsExhausted);
    assert!(!called);
    assert_eq!(budget.steps, 0);
    assert_eq!(budget.depth, 42);
}

#[test]
fn code_sessions_retain_generic_code_and_recheck_current_contracts() {
    use comptime_interpretor::{CodeSession, ContractOutcome};
    use nir::program::{TraitRequirement, TypePredicate};
    use std::sync::Arc;
    let mut source = package(expression(
        Term::Primitive(P::Type),
        K::TypeValue(Term::Rigid {
            declaration: id(2),
            sort: Sort::Type,
        }),
    ));
    source.functions[0].signature = Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(P::Type)),
    };
    source.functions[0]
        .requirements
        .push(TypePredicate::Trait(TraitRequirement {
            interface: id(90),
            interface_arguments: vec![],
            subject: Term::Primitive(P::Bool),
        }));
    let source = Arc::new(source);
    let mut session = CodeSession::new(source.clone());
    let allow = |_: &TypePredicate| ContractOutcome::Satisfied;
    for (input, fresh) in [
        (P::Bool, true),
        (P::Bool, false),
        (P::U8, true),
        (P::Bool, false),
    ] {
        let mut vm = Interpreter::for_comptime(&source, Budget::default())
            .unwrap()
            .with_type_solver(&allow);
        let result = vm
            .with_code_session(&mut session, |vm| {
                vm.invoke_instantiated(id(1), vec![Term::Primitive(input)], vec![])
            })
            .unwrap();
        assert_eq!(result, Value::Type(Term::Primitive(input)));
        assert_eq!(vm.bytecode_compilations() > 0, fresh);
        assert_eq!(
            vm.events()
                .iter()
                .any(|event| matches!(event, Event::InstantiateFunction { .. })),
            fresh
        );
    }
    for (outcome, expected) in [
        (ContractOutcome::Rejected, ErrorKind::TypeContractRejected),
        (ContractOutcome::Unknown, ErrorKind::TypeContractUnresolved),
    ] {
        let deny = |_: &TypePredicate| outcome;
        let mut vm = Interpreter::for_comptime(&source, Budget::default())
            .unwrap()
            .with_type_solver(&deny);
        let error = vm
            .with_code_session(&mut session, |vm| {
                vm.invoke_instantiated(id(1), vec![Term::Primitive(P::Bool)], vec![])
            })
            .unwrap_err();
        assert_eq!(error.kind, expected);
        assert_eq!(vm.bytecode_invocations(), 0);
    }
    let mut vm = Interpreter::for_comptime(&source, Budget::default())
        .unwrap()
        .with_type_solver(&allow);
    vm.with_code_session(&mut session, |vm| {
        vm.invoke_instantiated(id(1), vec![Term::Primitive(P::Bool)], vec![])
    })
    .unwrap();
    assert_eq!(vm.bytecode_compilations(), 0);
}

#[test]
fn generic_code_session_eviction_recompiles_without_changing_results() {
    use comptime_interpretor::CodeSession;
    use std::sync::Arc;
    let source = Arc::new(package(expression(
        Term::Primitive(P::Bool),
        K::Literal(Literal::Bool(true)),
    )));
    let mut session = CodeSession::new(source.clone());
    for count in 0..70 {
        let mut vm = Interpreter::for_comptime(&source, Budget::default()).unwrap();
        let input = Term::Tuple(vec![Term::Primitive(P::Bool); count]);
        assert_eq!(
            vm.with_code_session(&mut session, |vm| {
                vm.invoke_instantiated(id(1), vec![input], vec![])
            })
            .unwrap(),
            Value::Bool(true)
        );
    }
    for fresh in [true, false] {
        let mut vm = Interpreter::for_comptime(&source, Budget::default()).unwrap();
        assert_eq!(
            vm.with_code_session(&mut session, |vm| {
                vm.invoke_instantiated(id(1), vec![Term::Tuple(vec![])], vec![])
            })
            .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(vm.bytecode_compilations() > 0, fresh);
    }
}

#[test]
fn source_calls_prove_closed_and_ordinary_input_requirements_once_per_frame() {
    use comptime_interpretor::ContractOutcome;
    use nir::program::{LocalId, TraitRequirement, TypePredicate};
    use std::cell::RefCell;

    let mut world = package(expression(
        Term::Primitive(P::Bool),
        K::Literal(Literal::Bool(true)),
    ));
    let function = &mut world.functions[0];
    function.parameters.push(LocalId { owner: 1, index: 0 });
    function.signature = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Type,
            ty: Term::Primitive(P::Type),
        }],
        result: Box::new(Term::Primitive(P::Bool)),
    };
    let obligation = |interface, subject| TraitRequirement {
        interface: id(interface),
        subject,
        interface_arguments: vec![],
    };
    function.requirements.push(TypePredicate::Trait(obligation(
        30,
        Term::Rigid {
            declaration: id(2),
            sort: Sort::Type,
        },
    )));
    function.capabilities.push(obligation(
        31,
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
    ));
    let seen = RefCell::new(Vec::new());
    let solver = |predicate: &TypePredicate| {
        let TypePredicate::Trait(requirement) = predicate else {
            panic!("{predicate}")
        };
        seen.borrow_mut()
            .push((requirement.interface, requirement.subject.clone()));
        if requirement.subject == Term::Primitive(P::Char) {
            ContractOutcome::Rejected
        } else {
            ContractOutcome::Satisfied
        }
    };
    for reference in [false, true] {
        let mut vm = Interpreter::for_comptime(&world, Budget::default())
            .unwrap()
            .with_type_solver(&solver);
        if reference {
            vm = vm.with_reference_execution();
        }
        for ordinary in [P::Char, P::U8, P::U8] {
            let inputs = vec![Term::Primitive(P::Bool)];
            let values = vec![Value::Type(Term::Primitive(ordinary))];
            for inspect in [false, true] {
                let mut fuel = vm.remaining_steps() as usize;
                let result = if inspect {
                    vm.admit_source_call(id(1), inputs.clone(), values.clone(), &mut fuel)
                        .and_then(|call| call.resume(&mut fuel))
                } else {
                    vm.invoke_instantiated(id(1), inputs.clone(), values.clone())
                };
                if ordinary == P::Char {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractRejected);
                    assert!(
                        !vm.events()
                            .iter()
                            .any(|e| matches!(e, Event::EnterFunction(_)))
                    );
                } else {
                    assert_eq!(result.unwrap(), Value::Bool(true));
                }
                assert_eq!(
                    seen.take(),
                    [
                        (id(30), Term::Primitive(P::Bool)),
                        (id(31), Term::Primitive(ordinary))
                    ]
                );
            }
        }
    }
}

#[test]
fn explicit_entry_reports_input_representation_limits_and_rejects_foreign_binders() {
    let world = package(expression(
        Term::Primitive(P::Bool),
        K::Literal(Literal::Bool(true)),
    ));
    let mut vm = Interpreter::for_comptime(&world, Budget::default()).unwrap();
    let oversized = Term::Tuple(vec![Term::Primitive(P::Bool); 100_001]);
    let error = vm
        .invoke_instantiated(id(1), vec![oversized], vec![])
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::RepresentationSizeLimit);
    let binder = Term::Rigid {
        declaration: id(99),
        sort: Sort::Type,
    };
    let value = Value::FunctionInstance {
        declaration: id(1),
        arguments: vec![binder.clone()],
    };
    assert_eq!(
        vm.start_invocation(value, vec![]).err().unwrap().kind,
        ErrorKind::Type
    );
    assert!(vm.events().is_empty());
    vm.authorize_abstract_type_parameters(&[id(99)]).unwrap();
    assert_eq!(
        vm.invoke_instantiated(id(1), vec![binder], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn named_admission_spends_the_invokers_budget_on_nested_head_queries() {
    use nir::normal_forms::{Head, Kind, Parameter, Role};
    let mut world = package(expression(
        Term::Primitive(P::Bool),
        K::Literal(Literal::Bool(true)),
    ));
    world.normal_forms.push(Head {
        declaration: id(20),
        role: Role::Opaque,
        collector: None,
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(21),
            binding: nir::program::LocalId {
                owner: 20,
                index: 0,
            },
            kind: Kind::Type,
            ty: Term::Primitive(P::Type),
        }],
    });

    // The representation fits the per-term size limit. Each application also
    // needs domain evidence from its registered head in this source world.
    let argument = Term::Tuple(vec![
        Term::NormalForm {
            head: id(20),
            sort: Sort::Type,
            arguments: vec![Term::Primitive(P::Bool)],
        };
        20_000
    ]);
    for steps in [100_000, 5_000_000] {
        let mut vm = Interpreter::for_comptime(
            &world,
            Budget {
                steps,
                ..Default::default()
            },
        )
        .unwrap();
        let result = vm.invoke_instantiated(id(1), vec![argument.clone()], vec![]);
        if steps == 100_000 {
            assert_eq!(result.unwrap_err().kind, ErrorKind::StepsExhausted);
            assert!(vm.events().is_empty());
        } else {
            assert_eq!(result.unwrap(), Value::Bool(true));
            assert!(steps - vm.remaining_steps() > 100_000);
        }
    }
}
