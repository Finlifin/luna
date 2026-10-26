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
            declaration: id(1),
            name: "inspect".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: false,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
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
        vm.charge_preparation((vm.remaining_steps() - 2000) as usize)
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
    assert_eq!(
        Interpreter::for_comptime(&world, Budget::default())
            .err()
            .unwrap()
            .kind,
        ErrorKind::PhaseMismatch
    );
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
