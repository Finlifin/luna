use super::*;
use nir::program::{Function, FunctionPhase, MethodContract, MethodInstance, Trait, TraitPhase};
use std::cell::Cell;

#[test]
fn panicking_method_host_restores_native_depth_before_caller_reuse() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    for direct in [false, true] {
        let (package, _) = fixture(direct);
        let calls = Cell::new(0);
        let resolver = |_: &Expr, fuel: &mut u64| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                *fuel = u64::MAX;
                panic!("method resolution interrupted");
            }
            *fuel -= 1;
            Ok(MethodInstance {
                member: id(3),
                body: id(10),
                arguments: vec![],
                method_arguments: vec![],
            })
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_method_resolver(&resolver);
        vm.depth = 3;
        vm.effect_state.invocation = 41;
        vm.effect_state.resume = Some(17);
        assert!(catch_unwind(AssertUnwindSafe(|| vm.invoke(id(9), vec![]))).is_err());
        assert_eq!(vm.remaining_steps(), 0);
        assert_eq!(
            vm.depth, 3,
            "restore the enclosing native scope, not an assumed zero"
        );
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.effect_state.invocation, 41);
        assert_eq!(vm.effect_state.resume, Some(17));
        assert!(vm.resolved_method_bodies.is_empty());
        vm.budget.steps = Budget::default().steps;
        let result = vm.invoke(id(9), vec![]).unwrap();
        assert_eq!(
            result,
            if direct {
                Value::Bool(true)
            } else {
                Value::Function(id(10))
            }
        );
        assert_eq!(vm.depth, 3);
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(calls.get(), 2);
    }
}

#[test]
fn tree_and_staged_input_scopes_restore_depth_after_method_host_unwind() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let (package, _) = fixture(false);
    let expression = &package.functions[0].body;
    let input = NormalFormArgument::Value(expression.clone());
    for mode in 0..3 {
        let calls = Cell::new(0);
        let resolver = |_: &Expr, fuel: &mut u64| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                *fuel -= 1;
                panic!("method host interrupted within a native scope");
            }
            Ok(MethodInstance {
                member: id(3),
                body: id(10),
                arguments: vec![],
                method_arguments: vec![],
            })
        };
        let mut vm = Interpreter::for_comptime_tree(&package, Budget::default())
            .unwrap()
            .with_method_resolver(&resolver);
        vm.depth = 3;
        let mut env = Environment::default();
        let execute = |vm: &mut Interpreter<'_>, env: &mut Environment| -> Result<Value, Error> {
            let flow = match mode {
                0 => vm.eval(expression, env)?,
                1 => vm.eval_normal_argument(&input, &mut BTreeMap::new(), env)?,
                _ => {
                    let std::ops::ControlFlow::Continue(normal_inputs::StagedNormalInput::Value(
                        value,
                    )) = vm.interpret_staged_argument(&input, env, 0)?
                    else {
                        panic!("staged value");
                    };
                    return Ok(value);
                }
            };
            let Flow::Value(value) = flow else {
                panic!("method value");
            };
            Ok(value)
        };
        assert!(catch_unwind(AssertUnwindSafe(|| execute(&mut vm, &mut env))).is_err());
        assert_eq!(vm.remaining_steps(), 0);
        assert_eq!(vm.depth, 3, "scope mode {mode}");
        assert_eq!(vm.invocation_depth, 0);
        assert!(vm.resolved_method_bodies.is_empty());
        vm.budget.steps = Budget::default().steps;
        assert_eq!(execute(&mut vm, &mut env).unwrap(), Value::Function(id(10)));
        assert_eq!(vm.depth, 3);
        assert_eq!(calls.get(), 2);
    }
}

fn function(declaration: u32, result: Term, body: Expr) -> Function {
    Function {
        declaration: id(declaration),
        name: format!("method_test_{declaration}"),
        phase: FunctionPhase::Comptime,
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
            result: Box::new(result),
        },
        body,
    }
}

fn fixture(direct: bool) -> (Package, Expr) {
    let signature = Term::ComptimeFunction(Box::new(Term::Function {
        parameters: vec![],
        result: Box::new(ty(Primitive::Bool)),
    }));
    let method = expr(
        signature.clone(),
        K::TraitMethod {
            interface: id(1),
            interface_arguments: vec![],
            subject: ty(Primitive::Bool),
            member: id(3),
            method_arguments: vec![],
        },
    );
    let result = if direct {
        ty(Primitive::Bool)
    } else {
        signature.clone()
    };
    let body = if direct {
        expr(
            result.clone(),
            K::Call {
                callee: Box::new(method),
                arguments: vec![],
                parameter_bindings: vec![],
            },
        )
    } else {
        method
    };
    let package = Package {
        traits: vec![Trait {
            declaration: id(1),
            self_parameter: id(2),
            name: "Readable".into(),
            comptime_only: true,
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            associated: vec![],
            methods: vec![MethodContract {
                declaration: id(3),
                name: "read".into(),
                phase: TraitPhase::Comptime,
                is_quote: false,
                pure: false,
                is_unsafe: false,
                type_arguments: vec![],
                type_parameters: vec![],
                capabilities: vec![],
                program: Default::default(),
                parameter_bindings: vec![],
                by_value_receiver: false,
                by_reference_receiver: false,
                signature: signature.into(),
                requirements: vec![],
                default: None,
            }],
        }],
        functions: vec![
            function(9, result.clone(), body),
            function(
                10,
                ty(Primitive::Bool),
                expr(ty(Primitive::Bool), K::Literal(Literal::Bool(true))),
            ),
        ],
        ..Default::default()
    };
    let outer = expr(
        result.clone(),
        K::Block {
            statements: vec![mark()],
            result: Some(Box::new(expr(
                result.clone(),
                K::Call {
                    callee: Box::new(expr(
                        Term::ComptimeFunction(Box::new(Term::Function {
                            parameters: vec![],
                            result: Box::new(result),
                        })),
                        K::Function(id(9)),
                    )),
                    arguments: vec![],
                    parameter_bindings: vec![],
                },
            ))),
        },
    );
    (package, outer)
}

#[test]
fn method_preparation_is_owned_and_cancelled_at_every_driver_boundary() {
    for direct in [false, true] {
        let (package, body) = fixture(direct);
        let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
        let mut complete_polls = None;
        let mut pending_method = false;
        for limit in std::iter::once(usize::MAX).chain(0..64) {
            if complete_polls.is_some_and(|count| limit >= count) {
                break;
            }
            let calls = Cell::new(0);
            let resolver = |_: &Expr, fuel: &mut u64| {
                calls.set(calls.get() + 1);
                *fuel -= 1;
                Ok(MethodInstance {
                    member: id(3),
                    body: id(10),
                    arguments: vec![],
                    method_arguments: vec![],
                })
            };
            let mut vm = Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .with_method_resolver(&resolver);
            let mut env = Environment::from([(local(1), Value::Bool(false))]);
            let mut frame = vm.start_frame(&code, &mut env).unwrap();
            let mut session = Session::fragment(&mut vm, &mut frame);
            for polls in 1..=limit {
                if let Poll::Ready(flow) = session.poll().unwrap() {
                    let Some(Flow::Value(value)) = flow else {
                        panic!("value expected");
                    };
                    assert_eq!(
                        value,
                        if direct {
                            Value::Bool(true)
                        } else {
                            Value::Function(id(10))
                        }
                    );
                    assert_eq!(calls.get(), 1);
                    complete_polls = Some(polls);
                    break;
                }
            }
            let state = session.to_string();
            if matches!(session.request, Some(Request::Bytecode(_)))
                && session.vm.invocation_depth > 0
            {
                assert_eq!(
                    calls.get(),
                    0,
                    "host must wait for the next driver transition"
                );
                assert!(state.contains(if direct {
                    "(prepare-call "
                } else {
                    "(prepare-method "
                }));
                pending_method = true;
            }
            let before = (
                calls.get(),
                session.vm.remaining_steps(),
                session.vm.events().len(),
            );
            session.cancel();
            assert_eq!(
                before,
                (
                    calls.get(),
                    session.vm.remaining_steps(),
                    session.vm.events().len()
                )
            );
            assert_eq!(
                session.poll().unwrap_err().kind,
                ErrorKind::InvalidControlFlow
            );
            drop(session);
            assert_eq!(vm.depth, 0);
            assert_eq!(vm.invocation_depth, 0);
            env = frame.restore_environment();
            assert_eq!(env.len(), 1);
            assert_eq!(env[&local(1)], Value::Bool(limit != 0));
            let value = vm.invoke(id(9), vec![]).unwrap();
            assert_eq!(
                value,
                if direct {
                    Value::Bool(true)
                } else {
                    Value::Function(id(10))
                }
            );
            assert_eq!(
                calls.get(),
                1,
                "retry only selects if cancellation preceded the response"
            );
        }
        assert!(complete_polls.is_some());
        assert!(pending_method);
    }
}

#[test]
fn failed_method_preparation_unwinds_the_caller_and_does_not_cache_a_response() {
    for direct in [false, true] {
        let (package, body) = fixture(direct);
        let calls = Cell::new(0);
        let resolver = |_: &Expr, fuel: &mut u64| {
            calls.set(calls.get() + 1);
            *fuel -= 1;
            if calls.get() == 1 {
                return Err(Error {
                    kind: ErrorKind::UnmaterializedDispatch,
                    source: Some(SOURCE),
                });
            }
            Ok(MethodInstance {
                member: id(3),
                body: id(10),
                arguments: vec![],
                method_arguments: vec![],
            })
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_method_resolver(&resolver);
        let mut env = Environment::from([(local(1), Value::Bool(false))]);
        let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
        assert_eq!(
            vm.execute_code(&code, &mut env).unwrap_err().kind,
            ErrorKind::UnmaterializedDispatch
        );
        assert_eq!(env[&local(1)], Value::Bool(true));
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.invocation_depth, 0);
        assert!(vm.resolved_method_bodies.is_empty());
        assert!(matches!(
            vm.execute_code(&code, &mut env).unwrap(),
            Flow::Value(_)
        ));
        assert_eq!(calls.get(), 2);
    }
}
