//! Associated initialization uses isolated environments and explicit activations.
use super::*;
use nir::program::Literal;
const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn key(member: u32) -> (InterfaceInstance, Term, DeclarationId) {
    (
        InterfaceInstance {
            declaration: id(1),
            arguments: vec![],
        },
        Term::Primitive(Primitive::Bool),
        id(member),
    )
}
fn expr(kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty: Term::Primitive(Primitive::Bool),
        kind,
    }
}
fn read(member: u32) -> Expr {
    let (interface, subject, member) = key(member);
    expr(K::AssociatedValue {
        interface,
        subject,
        member,
    })
}
fn run(
    vm: &mut Interpreter<'_>,
    body: &Expr,
    env: &mut Environment,
    compiled: bool,
) -> Result<Flow, Error> {
    if compiled {
        let code = Arc::new(comptime_compiler::compile(body, &mut 10000).unwrap());
        assert!(code.to_string().contains("(read-associated "));
        vm.execute_code(&code, env)
    } else {
        vm.eval(body, env)
    }
}
#[test]
fn associated_cycles_and_failed_initializers_release_every_active_key() {
    let package = Package::default();
    let chain = [read(3), read(4), read(2)];
    let good = expr(K::Literal(Literal::Bool(true)));
    let caller = LocalId { owner: 7, index: 0 };
    let missing = expr(K::Local(caller));
    let wrong = expr(K::Literal(Literal::Char('x')));
    let returned = expr(K::Return(Some(Box::new(good.clone()))));
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (i, initializer) in chain.iter().enumerate() {
            vm.associated_initializers
                .insert(key(i as u32 + 2), initializer);
        }
        let mut env = Environment::from([(caller, Value::Bool(true))]);
        for (initializer, error) in [
            (&chain[2], ErrorKind::InvalidControlFlow),
            (&missing, ErrorKind::MissingBinding),
            (&wrong, ErrorKind::Type),
            (&returned, ErrorKind::InvalidControlFlow),
        ] {
            vm.associated_initializers.insert(key(4), initializer);
            assert_eq!(
                run(&mut vm, &read(2), &mut env, compiled).unwrap_err().kind,
                error
            );
            assert!(vm.active_associated_initializers.is_empty());
            assert_eq!(env[&caller], Value::Bool(true));
            vm.associated_initializers.insert(key(4), &good);
            assert!(matches!(
                run(&mut vm, &read(2), &mut env, compiled).unwrap(),
                Flow::Value(Value::Bool(true))
            ));
            assert!(vm.active_associated_initializers.is_empty());
        }
    }
}
#[test]
fn associated_chains_use_the_explicit_stack_and_clean_up_budget_failure() {
    let package = Package::default();
    let mut chain: Vec<_> = (3..67).map(read).collect();
    chain.push(expr(K::Literal(Literal::Bool(true))));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (i, initializer) in chain.iter().enumerate() {
        vm.associated_initializers
            .insert(key(i as u32 + 2), initializer);
    }
    vm.budget.depth = 8;
    let mut env = Environment::default();
    assert!(matches!(
        run(&mut vm, &read(2), &mut env, true).unwrap(),
        Flow::Value(Value::Bool(true))
    ));
    assert!(vm.active_associated_initializers.is_empty());
    vm.budget.steps = 100;
    assert_eq!(
        run(&mut vm, &read(2), &mut env, true).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.active_associated_initializers.is_empty());
    vm.budget.steps = Budget::default().steps;
    assert!(matches!(
        run(&mut vm, &read(2), &mut env, true).unwrap(),
        Flow::Value(Value::Bool(true))
    ));
}

#[test]
fn associated_code_reuse_does_not_cache_selection_results_or_execution() {
    let package = Package::default();
    let yes = expr(K::Literal(Literal::Bool(true)));
    // Deliberately use identical source coordinates and result types: those are
    // not code identity, nor is the associated member's declaration alone.
    let no = expr(K::Literal(Literal::Bool(false)));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut env = Environment::default();
    for (initializer, expected, compilations) in [
        (&yes, true, 1),
        (&yes, true, 1),
        (&no, false, 2),
        (&yes, true, 2),
    ] {
        vm.associated_initializers.insert(key(2), initializer);
        let before = vm.remaining_steps();
        assert!(matches!(run(&mut vm, &read(2), &mut env, true).unwrap(),
            Flow::Value(Value::Bool(value)) if value == expected));
        assert_eq!(vm.bytecode_compilations(), compilations);
        assert!(vm.remaining_steps() < before);
        assert!(vm.active_associated_initializers.is_empty());
    }
    // A warmed body cannot bypass the current cycle guard or missing evidence.
    vm.active_associated_initializers.insert(key(2));
    assert_eq!(
        run(&mut vm, &read(2), &mut env, true).unwrap_err().kind,
        ErrorKind::InvalidControlFlow
    );
    vm.active_associated_initializers.clear();
    vm.associated_initializers.clear();
    assert_eq!(
        run(&mut vm, &read(2), &mut env, true).unwrap_err().kind,
        ErrorKind::UnmaterializedDispatch
    );
}

#[test]
fn borrowed_code_cache_does_not_poison_retries_after_compilation_budget_failure() {
    let package = Package::default();
    let body = expr(K::Literal(Literal::Bool(true)));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    vm.budget.steps = 0;
    assert_eq!(
        vm.borrowed_invocation_code(&body).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    vm.budget.steps = Budget::default().steps;
    let program = vm.borrowed_invocation_code(&body).unwrap().unwrap();
    let count = vm.bytecode_compilations();
    let reused = vm.borrowed_invocation_code(&body).unwrap().unwrap();
    assert!(Arc::ptr_eq(&program, &reused));
    assert_eq!(vm.bytecode_compilations(), count);
    vm.bytecode_enabled = false;
    assert!(vm.borrowed_invocation_code(&body).unwrap().is_none());
}

#[test]
fn owned_associated_selection_checks_type_phase_and_budget_before_publication() {
    use std::cell::Cell;
    let package = Package::default();
    for compiled in [false, true] {
        let calls = Cell::new(0);
        let resolver = |_: &Expr, fuel: &mut u64| {
            *fuel -= 1;
            calls.set(calls.get() + 1);
            let mut value = expr(K::Literal(Literal::Bool(true)));
            if calls.get() == 1 {
                value.ty = Term::Primitive(Primitive::Char);
            }
            Ok(AssociatedSelection {
                initializer: nir::associated_initializers::AssociatedInitializer::new(
                    value, &mut 1000,
                )
                .unwrap(),
                requires_invocation: true,
            })
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_associated_resolver(&resolver);
        let mut env = Environment::default();
        assert_eq!(
            run(&mut vm, &read(2), &mut env, compiled).unwrap_err().kind,
            ErrorKind::Type
        );
        assert!(!vm.associated_initializers.contains_key(&key(2)));
        for _ in 0..2 {
            assert!(matches!(
                run(&mut vm, &read(2), &mut env, compiled).unwrap(),
                Flow::Value(Value::Bool(true))
            ));
        }
        assert_eq!(calls.get(), 2);
        assert!(vm.active_associated_initializers.is_empty());
        let mut runtime = Interpreter::new(&package, Budget::default())
            .unwrap()
            .with_associated_resolver(&resolver);
        assert_eq!(
            run(&mut runtime, &read(2), &mut env, compiled)
                .unwrap_err()
                .kind,
            ErrorKind::UnmaterializedDispatch
        );
        assert_eq!(calls.get(), 2);
        let refill = |_: &Expr, fuel: &mut u64| {
            *fuel += 1;
            Err(Error {
                kind: ErrorKind::UnmaterializedDispatch,
                source: None,
            })
        };
        let mut rejected = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_associated_resolver(&refill);
        assert_eq!(
            run(&mut rejected, &read(2), &mut env, compiled)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(rejected.remaining_steps(), 0);
    }
}

#[test]
fn owned_associated_cycles_and_budget_failures_release_active_keys() {
    let package = Package::default();
    let resolver = |access: &Expr, fuel: &mut u64| {
        *fuel -= 1;
        let K::AssociatedValue { member, .. } = &access.kind else {
            unreachable!()
        };
        let value = if member.local < 34 {
            read(member.local + 1)
        } else {
            expr(K::Literal(Literal::Bool(true)))
        };
        Ok(AssociatedSelection {
            initializer: nir::associated_initializers::AssociatedInitializer::new(value, &mut 1000)
                .unwrap(),
            requires_invocation: true,
        })
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_associated_resolver(&resolver);
    vm.budget.depth = 8;
    let mut env = Environment::default();
    assert!(matches!(
        run(&mut vm, &read(2), &mut env, true).unwrap(),
        Flow::Value(Value::Bool(true))
    ));
    vm.budget.steps = 100;
    assert_eq!(
        run(&mut vm, &read(2), &mut env, true).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.active_associated_initializers.is_empty());
    vm.budget.steps = Budget::default().steps;
    assert!(matches!(
        run(&mut vm, &read(2), &mut env, true).unwrap(),
        Flow::Value(Value::Bool(true))
    ));
    vm.associated_initializers
        .insert_owned(key(34), Arc::new(read(2)));
    assert_eq!(
        run(&mut vm, &read(2), &mut env, true).unwrap_err().kind,
        ErrorKind::InvalidControlFlow
    );
    assert!(vm.active_associated_initializers.is_empty());
}

#[test]
fn associated_host_unwind_exhausts_allowance_without_publishing_a_selection() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let package = Package::default();
    for compiled in [false, true] {
        let calls = std::cell::Cell::new(0);
        let resolver = |_: &Expr, fuel: &mut u64| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                *fuel = u64::MAX;
                panic!("associated host interrupted");
            }
            *fuel -= 1;
            Ok(AssociatedSelection {
                initializer: nir::associated_initializers::AssociatedInitializer::new(
                    expr(K::Literal(Literal::Bool(true))),
                    &mut 1000,
                )
                .unwrap(),
                requires_invocation: true,
            })
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_associated_resolver(&resolver);
        let mut env = Environment::default();
        assert!(
            catch_unwind(AssertUnwindSafe(|| run(
                &mut vm,
                &read(2),
                &mut env,
                compiled
            )))
            .is_err()
        );
        assert_eq!(vm.remaining_steps(), 0);
        assert!(!vm.associated_initializers.contains_key(&key(2)));
        assert!(vm.active_associated_initializers.is_empty());
        assert_eq!(calls.get(), 1);
        vm.budget.steps = Budget::default().steps;
        assert!(matches!(
            run(&mut vm, &read(2), &mut env, compiled).unwrap(),
            Flow::Value(Value::Bool(true))
        ));
        assert_eq!(calls.get(), 2);
    }
}

#[test]
fn initialized_type_views_are_query_local_and_preserve_nested_projection_structure() {
    let package = Package::default();
    let resolver = |_: &Expr, fuel: &mut u64| {
        *fuel -= 1;
        Ok(AssociatedSelection {
            initializer: nir::associated_initializers::AssociatedInitializer::new(
                Expr {
                    source: SOURCE,
                    ty: Term::Primitive(Primitive::Type),
                    kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
                },
                &mut 1000,
            )
            .unwrap(),
            requires_invocation: true,
        })
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_associated_resolver(&resolver);
    let projection = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(1),
            member: id(2),
        }),
        arguments: vec![],
        subject: Box::new(Term::Primitive(Primitive::Bool)),
    };
    let nested = Term::Optional(Box::new(projection));
    for _ in 0..2 {
        let before = vm.remaining_steps();
        assert_eq!(
            vm.selected_type_view(&nested, Some(SOURCE)).unwrap(),
            Term::Optional(Box::new(Term::Primitive(Primitive::Bool)))
        );
        assert!(vm.remaining_steps() < before);
        assert!(!vm.associated_values.contains_key(&key(2)));
        assert!(vm.associated_initializers.contains_key(&key(2)));
        assert!(vm.active_associated_initializers.is_empty());
    }
}

#[test]
fn associated_result_domains_survive_query_local_type_observations() {
    let package = Package::default();
    let projection = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(1),
            member: id(3),
        }),
        arguments: vec![],
        subject: Box::new(Term::Primitive(Primitive::Bool)),
    };
    let resolver = |access: &Expr, fuel: &mut u64| {
        *fuel -= 1;
        let K::AssociatedValue { member, .. } = access.kind else {
            unreachable!()
        };
        let value = if member == id(3) {
            Expr {
                source: SOURCE,
                ty: Term::Primitive(Primitive::Type),
                kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
            }
        } else {
            expr(K::Literal(Literal::Bool(true)))
        };
        Ok(AssociatedSelection {
            initializer: nir::associated_initializers::AssociatedInitializer::new(value, &mut 1000)
                .unwrap(),
            requires_invocation: true,
        })
    };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_associated_resolver(&resolver);
        let mut access = read(2);
        access.ty = projection.clone();
        for _ in 0..2 {
            assert!(matches!(
                run(&mut vm, &access, &mut Environment::default(), compiled).unwrap(),
                Flow::Value(Value::Bool(true))
            ));
            assert!(!vm.associated_values.contains_key(&key(3)));
            assert!(vm.active_associated_initializers.is_empty());
        }
        access.ty = Term::Primitive(Primitive::Char);
        assert_eq!(
            run(&mut vm, &access, &mut Environment::default(), compiled)
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        assert!(vm.active_associated_initializers.is_empty());
    }
}

#[test]
fn invocation_type_comparison_preserves_query_failures_and_opaque_identity() {
    use std::cell::Cell;
    let package = Package::default();
    let mode = Cell::new(0);
    let calls = Cell::new(0);
    let resolver = |_: &Expr, fuel: &mut u64| {
        calls.set(calls.get() + 1);
        *fuel -= 1;
        let kind = match mode.get() {
            0 => Some(ErrorKind::TypeContractUnresolved),
            1 => Some(ErrorKind::StepsExhausted),
            2 => Some(ErrorKind::Type),
            _ => None,
        };
        if let Some(kind) = kind {
            return Err(Error {
                kind,
                source: Some(SOURCE),
            });
        }
        Ok(AssociatedSelection {
            initializer: nir::associated_initializers::AssociatedInitializer::new(
                Expr {
                    source: SOURCE,
                    ty: Term::Primitive(Primitive::Type),
                    kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
                },
                &mut 1000,
            )
            .unwrap(),
            requires_invocation: true,
        })
    };
    let projection = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(1),
            member: id(2),
        }),
        arguments: vec![],
        subject: Box::new(Term::Primitive(Primitive::Bool)),
    };
    let callable = |result| Term::Function {
        parameters: vec![],
        result: Box::new(result),
    };
    let expected = callable(Term::Primitive(Primitive::Bool));
    let output = callable(projection.clone());
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_associated_resolver(&resolver);
    for (state, error) in [
        (0, ErrorKind::TypeContractUnresolved),
        (1, ErrorKind::StepsExhausted),
        (2, ErrorKind::Type),
    ] {
        mode.set(state);
        let before = vm.remaining_steps();
        let failed = vm
            .observe_same_type(&expected, &output, SOURCE)
            .unwrap_err();
        assert_eq!(failed.kind, error);
        assert_eq!(failed.source, Some(SOURCE));
        assert!(vm.remaining_steps() < before);
        assert!(!vm.associated_values.contains_key(&key(2)));
        assert!(!vm.associated_initializers.contains_key(&key(2)));
        assert!(vm.active_associated_initializers.is_empty());
    }
    mode.set(3);
    for _ in 0..2 {
        assert!(vm.observe_same_type(&expected, &output, SOURCE).unwrap());
        assert!(!vm.associated_values.contains_key(&key(2)));
        assert!(vm.active_associated_initializers.is_empty());
    }
    assert_eq!(calls.get(), 4, "only the admitted initializer is retained");
    assert!(
        !vm.observe_same_type(&callable(Term::Primitive(Primitive::Char)), &output, SOURCE)
            .unwrap()
    );
    let opaque = Term::NormalForm {
        head: id(9),
        sort: nir::Sort::Type,
        arguments: vec![projection.clone()],
    };
    assert!(
        !vm.observe_same_type(&opaque, &Term::Primitive(Primitive::Bool), SOURCE)
            .unwrap()
    );
    assert!(
        vm.observe_same_type(
            &opaque,
            &Term::NormalForm {
                head: id(9),
                sort: nir::Sort::Type,
                arguments: vec![Term::Primitive(Primitive::Bool)],
            },
            SOURCE
        )
        .unwrap()
    );
    vm.budget.steps = 0;
    assert_eq!(
        vm.observe_same_type(&expected, &expected, SOURCE)
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
}

#[test]
fn runtime_identity_observes_associated_output_without_an_executable_body() {
    use std::cell::Cell;
    let signature = |result| Term::Function {
        parameters: vec![],
        result: Box::new(result),
    };
    let package = Package {
        runtime_function_identities: vec![nir::program::RuntimeFunctionIdentity {
            declaration: id(9),
            type_parameters: vec![],
            signature: signature(Term::AssociatedType {
                origin: Box::new(nir::AssociatedTypeOrigin {
                    interface: id(1),
                    member: id(2),
                }),
                subject: Box::new(Term::Primitive(Primitive::Bool)),
                arguments: vec![],
            }),
        }],
        ..Default::default()
    };
    let ready = Cell::new(false);
    let resolver = |_: &Expr, fuel: &mut u64| {
        *fuel -= 1;
        if !ready.get() {
            return Err(Error {
                kind: ErrorKind::TypeContractUnresolved,
                source: Some(SOURCE),
            });
        }
        Ok(AssociatedSelection {
            initializer: nir::associated_initializers::AssociatedInitializer::new(
                Expr {
                    source: SOURCE,
                    ty: Term::Primitive(Primitive::Type),
                    kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
                },
                &mut 1000,
            )
            .unwrap(),
            requires_invocation: true,
        })
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_associated_resolver(&resolver);
    let output = signature(Term::Primitive(Primitive::Bool));
    assert_eq!(
        vm.function_instance_value(id(9), &[], &output, SOURCE)
            .unwrap_err()
            .kind,
        ErrorKind::TypeContractUnresolved
    );
    ready.set(true);
    assert_eq!(
        vm.function_instance_value(id(9), &[], &output, SOURCE)
            .unwrap(),
        Value::FunctionInstance {
            declaration: id(9),
            arguments: vec![],
        }
    );
    assert!(vm.instances.is_empty());
    assert!(
        !vm.events
            .iter()
            .any(|event| matches!(event, Event::InstantiateFunction { .. }))
    );
    assert!(!vm.associated_values.contains_key(&key(2)));
    assert_eq!(
        vm.invoke(id(9), vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
}

#[test]
fn associated_input_domains_admit_values_and_reject_mismatches() {
    let package = Package::default();
    let reference = ValueReference::owned(Value::Bool(true));
    let resolver = |_: &Expr, fuel: &mut u64| {
        *fuel -= 1;
        // Resolving a domain must not hold the actual argument's storage lock.
        assert!(matches!(
            reference.borrow(fuel).unwrap().value(),
            Value::Bool(true)
        ));
        Ok(AssociatedSelection {
            initializer: nir::associated_initializers::AssociatedInitializer::new(
                Expr {
                    source: SOURCE,
                    ty: Term::Primitive(Primitive::Type),
                    kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
                },
                &mut 1000,
            )
            .unwrap(),
            requires_invocation: true,
        })
    };
    for sort in [nir::Sort::Runtime, nir::Sort::ComptimeValue] {
        let signature = Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![
                nir::Parameter {
                    sort: nir::Sort::Type,
                    ty: Term::Primitive(Primitive::Type),
                },
                nir::Parameter {
                    sort,
                    ty: Term::AssociatedType {
                        origin: Box::new(nir::AssociatedTypeOrigin {
                            interface: id(1),
                            member: id(3),
                        }),
                        arguments: vec![],
                        subject: Box::new(Term::Bound {
                            depth: 0,
                            index: 0,
                            sort: nir::Sort::Type,
                        }),
                    },
                },
            ],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        }));
        let parameters = [
            LocalId { owner: 7, index: 0 },
            LocalId { owner: 7, index: 1 },
        ];
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_associated_resolver(&resolver);
        let mut environment = Environment::default();
        vm.bind_invocation_inputs(
            &parameters,
            vec![
                Value::Type(Term::Primitive(Primitive::Bool)),
                Value::ComptimeReference(reference.clone()),
            ],
            &signature,
            &mut environment,
            SOURCE,
        )
        .unwrap();
        assert_eq!(
            environment[&parameters[1]],
            Value::ComptimeReference(reference.clone())
        );
        for (value, valid) in [
            (Value::Bool(true), true),
            (Value::Char('x'), false),
            (Value::Bool(false), true),
        ] {
            let mut environment = Environment::default();
            let result = vm.bind_invocation_inputs(
                &parameters,
                vec![Value::Type(Term::Primitive(Primitive::Bool)), value],
                &signature,
                &mut environment,
                SOURCE,
            );
            if valid {
                result.unwrap();
                assert!(environment.contains_key(&parameters[1]));
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                assert!(!environment.contains_key(&parameters[1]));
            }
            assert!(!vm.associated_values.contains_key(&key(3)));
            assert!(vm.active_associated_initializers.is_empty());
        }
    }
}

#[test]
fn selected_associated_body_observes_its_domain_once_before_execution() {
    let type_type = Term::Primitive(Primitive::Type);
    let function = nir::program::Function {
        declaration: id(9),
        name: "Domain".into(),
        phase: nir::program::FunctionPhase::Comptime,
        inline: false,
        pure: false,
        is_unsafe: false,
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        parameters: vec![],
        defaults: vec![],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(type_type.clone()),
        },
        body: Expr {
            source: SOURCE,
            ty: type_type.clone(),
            kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
        },
    };
    let callable = function.callable_type();
    let package = Package {
        functions: vec![function],
        ..Default::default()
    };
    let projection = Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(1),
            member: id(3),
        }),
        arguments: vec![],
        subject: Box::new(Term::Primitive(Primitive::Bool)),
    };
    let resolver = |access: &Expr, fuel: &mut u64| {
        *fuel -= 1;
        let K::AssociatedValue { member, .. } = access.kind else {
            unreachable!()
        };
        let value = if member == id(3) {
            Expr {
                source: SOURCE,
                ty: type_type.clone(),
                kind: K::Call {
                    callee: Box::new(Expr {
                        source: SOURCE,
                        ty: callable.clone(),
                        kind: K::Function(id(9)),
                    }),
                    arguments: vec![],
                    parameter_bindings: vec![],
                },
            }
        } else {
            Expr {
                source: SOURCE,
                ty: projection.clone(),
                kind: K::Block {
                    statements: vec![],
                    result: Some(Box::new(expr(K::Literal(Literal::Bool(true))))),
                },
            }
        };
        Ok(AssociatedSelection {
            initializer: {
                let mut construction_fuel = usize::try_from(*fuel).unwrap();
                let initializer = nir::associated_initializers::AssociatedInitializer::new(
                    value,
                    &mut construction_fuel,
                )
                .unwrap();
                *fuel = construction_fuel as u64;
                initializer
            },
            requires_invocation: true,
        })
    };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_associated_resolver(&resolver);
        vm.bytecode_enabled = compiled;
        for _ in 0..3 {
            let before = vm.events.len();
            assert!(matches!(
                run(&mut vm, &read(2), &mut Environment::default(), compiled).unwrap(),
                Flow::Value(Value::Bool(true))
            ));
            let entered: Vec<_> = vm.events[before..]
                .iter()
                .filter_map(|event| match event {
                    Event::EnterFunction(declaration) => Some(*declaration),
                    _ => None,
                })
                .collect();
            assert_eq!(entered, [id(9)], "one fresh domain observation per access");
            assert!(!vm.associated_values.contains_key(&key(3)));
            assert!(vm.active_associated_initializers.is_empty());
        }
    }
}
