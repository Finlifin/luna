//! Native checks against a concrete IIR handoff. Altering only that handoff
//! isolates interface emission from source verification and inherited body checks.
use super::*;
use nir::program::ExprKind as K;

fn source_package(source: &str) -> nir::program::Package {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "luna-host-interface-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    // These isolated compiler fixtures do not load the public sysroot.
    // Primitive names therefore need explicit bindings in the source package.
    std::fs::write(&path, format!("typealias bool = .bool'builtin;\n{source}")).unwrap();
    let crate::cli::Command::Compile(options) =
        crate::cli::parse([path.clone().into_os_string(), "--emit".into(), "nir".into()]).unwrap()
    else {
        panic!("compile options")
    };
    let artifact = crate::frontend::compile(&options);
    std::fs::remove_file(path).unwrap();
    let crate::frontend::Artifact::Nir(package) = artifact.unwrap() else {
        panic!("NIR")
    };
    package
}

#[test]
fn interface_resource_query_retains_callback_premises() {
    use elaboration::verification::moves::CallableParameter;
    for callback in ["yes", "|| true"] {
        let package = source_package(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
             impl Duplicate for bool {{}}
             fn yes()->bool {{true}}
             fn wrong() {{}}
             trait Apply {{fn apply(f:fn()->bool,flag:bool)->result:bool
                 where requires flag, ensures flag'old;}}
             struct Host {{}} impl Apply for Host {{
                 fn apply(f:fn()->bool,flag:bool)->bool {{f()}} }}
             fn run()->bool {{Host.as(Apply).apply({callback},true)}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let mut call = None;
        nir::visit::expressions(run, &mut 10000, |expr, _| {
            if matches!(&expr.kind,K::Call {callee,..} if matches!(callee.kind,K::TraitMethod {..}))
            {
                call = Some(expr.clone());
            }
            Ok(())
        })
        .unwrap();
        let call = call.unwrap();
        let K::Call {
            callee, arguments, ..
        } = &call.kind
        else {
            unreachable!()
        };
        let input = CallableParameter {
            choices: vec![],
            enum_captures: Default::default(),
            parameter: 0,
            fields: vec![],
            definition: arguments
                .iter()
                .find(|a| a.parameter == 0)
                .unwrap()
                .value
                .clone(),
            alternatives: vec![],
            captures: Default::default(),
        };
        let context = crate::resources::ProtocolContext::new(
            &package,
            "Duplicate",
            Some("Retire"),
            Default::default(),
        )
        .unwrap();
        let world = context.world(&package);
        let query = |inputs: &[CallableParameter], fuel| {
            crate::protocols::analyze_method_cleanup_with_context(
                &package,
                &world,
                &context.items,
                callee,
                fuel,
                Default::default(),
                &[],
                inputs,
                &[],
                vec![],
                &[],
                &[],
            )
        };
        let unknown = query(&[], 100000).unwrap();
        assert!(!unknown.report.deferred_calls.is_empty());
        assert!(crate::cleanup::lower_method_call_cleanup(&unknown, &call, &mut 100000).is_err());
        let report = query(std::slice::from_ref(&input), 100000).unwrap();
        assert_eq!(report.report.callable_parameters, vec![input.clone()]);
        assert!(report.report.deferred_calls.is_empty());
        assert!(
            report
                .report
                .named_calls
                .iter()
                .any(|call| call.callables == vec![input.clone()])
        );
        assert!(report.to_string().contains("(callable-parameter"));
        assert!(
            crate::cleanup::lower_method_call_cleanup(&report, &call, &mut 100000)
                .unwrap_err()
                .contains("completed invocation contexts")
        );
        assert!(query(std::slice::from_ref(&input), 0).is_err());
        let mut bad = input.clone();
        bad.parameter = 2;
        assert!(query(&[bad], 100000).is_err());
        let wrong = package
            .functions
            .iter()
            .find(|f| f.name == "wrong")
            .unwrap();
        let mut bad = input.clone();
        bad.definition.kind = K::Function(wrong.declaration);
        bad.definition.ty = wrong.signature.clone();
        assert!(query(&[bad], 100000).is_err());
        assert_eq!(
            query(&[input], 100000).unwrap().to_string(),
            report.to_string()
        );
    }
}

fn graph(source: &str) -> iir::Package {
    let package = source_package(source);
    let mut graph = crate::instantiation::instantiate(&package, "run", 100000).unwrap();
    // A backend must enforce the interface even when a selected implementation
    // has a weaker entry domain and contains no inherited predicate copy.
    for instance in &mut graph.functions {
        instance.function.preconditions.clear();
        instance.function.postconditions.clear();
    }
    graph
}

#[test]
fn interface_resource_query_preserves_affine_callback_for_implementation_check() {
    use elaboration::verification::moves::CallableParameter;
    for twice in [false, true] {
        let body = if twice { "f();f()" } else { "f()" };
        let package = source_package(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
             impl Duplicate for bool {{}}
             struct Token {{}} impl Retire for Token {{fn drop(*self) {{}}}}
             fn take(token:Token) {{}}
             trait Apply {{fn apply(f:%{{token:Token}} fn()->bool,flag:bool)->bool
                 where requires flag, ensures flag'old;}}
             struct Host {{}} impl Apply for Host {{
                 fn apply(f:%{{token:Token}} fn()->bool,flag:bool)->bool {{{body}}} }}
             fn run()->bool {{let token=Token {{}};
                 Host.as(Apply).apply(|| {{take(token);true}},true)}}"
        ));
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let mut call = None;
        nir::visit::expressions(run, &mut 10000, |expr, _| {
            if matches!(&expr.kind,K::Call {callee,..} if matches!(callee.kind,K::TraitMethod {..}))
            {
                call = Some(expr.clone());
            }
            Ok(())
        })
        .unwrap();
        let call = call.unwrap();
        let K::Call {
            callee, arguments, ..
        } = &call.kind
        else {
            unreachable!()
        };
        let input = CallableParameter {
            parameter: 0,
            definition: arguments
                .iter()
                .find(|a| a.parameter == 0)
                .unwrap()
                .value
                .clone(),
            fields: vec![],
            choices: vec![],
            alternatives: vec![],
            captures: Default::default(),
            enum_captures: Default::default(),
        };
        let context = crate::resources::ProtocolContext::new(
            &package,
            "Duplicate",
            Some("Retire"),
            Default::default(),
        )
        .unwrap();
        let result = crate::protocols::analyze_method_cleanup_with_context(
            &package,
            &context.world(&package),
            &context.items,
            callee,
            100000,
            Default::default(),
            &[],
            &[input.clone()],
            &[],
            vec![],
            &[],
            &[],
        );
        let result = result.unwrap();
        assert_eq!(result.report.callable_parameters, vec![input]);
        assert!(result.report.deferred_calls.is_empty());
        let nested = result
            .report
            .named_calls
            .iter()
            .find(|call| call.source == result.invocation.function.body.source)
            .unwrap();
        let (body, arguments) = package
            .runtime_body(
                result.invocation.implementation.declaration,
                &result.invocation.implementation.arguments,
            )
            .unwrap();
        assert!(arguments.is_empty());
        // The frame report records a contextual child, not a certificate for
        // its body. Discharge that child exactly as the schedule publisher does.
        let checked = crate::protocols::analyze_cleanup_with_borrowed_code_context(
            &package,
            &context.world(&package),
            &context.items,
            body,
            100000,
            Default::default(),
            &[],
            &nested.callables,
            &nested.enums,
            nested.effects.clone(),
            &[],
            &nested.borrowed_code,
        );
        if twice {
            let error = checked.unwrap_err();
            assert!(
                error.reason.contains("unavailable") || error.reason.contains("move"),
                "{error}"
            );
        } else {
            let checked = checked.unwrap();
            assert!(checked.deferred_calls.is_empty());
            assert_eq!(
                checked
                    .closures
                    .iter()
                    .filter(|closure| closure.invoked)
                    .count(),
                1
            );
        }
    }
}

#[test]
fn native_interface_entry_checks_survive_without_inherited_body_checks() {
    let graph = graph(
        "trait Gate {fn enter(flag:bool)->bool where requires flag;}
        struct Host {} impl Gate for Host {fn enter(flag:bool)->bool {true}}
        fn run()->bool {Host.as(Gate).enter(true)}",
    );
    assert_eq!(graph.interfaces.len(), 1);
    let good = llvm::emit(&graph, 100000, false, None, None).unwrap();
    let mut rejected = graph.clone();
    let predicate = &mut rejected.interfaces[0].methods[0].program.preconditions[0];
    predicate.kind = K::Literal(nir::program::Literal::Bool(false));
    let bad = llvm::emit(&rejected, 100000, false, None, None).unwrap();
    for (input, succeeds) in [(&graph, true), (&rejected, false)] {
        let mut interpreter =
            comptime_interpretor::Interpreter::from_iir(input, Default::default()).unwrap();
        let result = interpreter.invoke_instance(&input.roots[0], vec![]);
        if succeeds {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert_eq!(
                result.unwrap_err().kind,
                comptime_interpretor::ErrorKind::ProgramContractRejected
            );
        }
    }

    for optimization in [OptimizationLevel::O0, OptimizationLevel::O3] {
        let tools = Toolchain {
            optimization,
            ..Default::default()
        };
        let success = tools.execute_test_ir(&good).unwrap();
        assert!(success.status.success(), "{success:?}");
        assert!(success.stderr.is_empty());
        let failure = tools.execute_test_ir(&bad).unwrap();
        assert!(
            !failure.status.success(),
            "interface predicate was skipped: {failure:?}"
        );
        let source = graph.interfaces[0].methods[0].program.preconditions[0]
            .source
            .original(&graph.expansion_sources)
            .unwrap();
        let diagnostic = String::from_utf8(failure.stderr).unwrap();
        assert!(
            diagnostic.contains(&format!("(predicate {} {})", source.start, source.end)),
            "{diagnostic}"
        );
        assert_eq!(diagnostic.matches("runtime-check-failure").count(), 1);
    }
    let mut missing = graph.clone();
    missing.interfaces.clear();
    let error = llvm::emit(&missing, 100000, false, None, None).unwrap_err();
    assert!(
        error.contains("IIR dispatch has no interface contract"),
        "{error}"
    );
    let mut malformed = graph.clone();
    malformed.interfaces[0].methods[0]
        .parameter_bindings
        .clear();
    let error = llvm::emit(&malformed, 100000, false, None, None).unwrap_err();
    assert!(
        error.contains("IIR interface parameter bindings do not match its signature"),
        "{error}"
    );
    assert_eq!(llvm::emit(&graph, 100000, false, None, None).unwrap(), good);
}

#[test]
fn interface_predicates_cannot_read_unbound_caller_locals() {
    let graph = graph(
        "trait Gate {fn enter(flag:bool)->bool where requires flag, ensures flag'old;}
         struct Host {} impl Gate for Host {fn enter(flag:bool)->bool {true}}
         fn run()->bool {let secret=true;Host.as(Gate).enter(true)}",
    );
    let run = graph
        .functions
        .iter()
        .find(|f| f.function.name == "run")
        .unwrap();
    let mut secret = None;
    nir::visit::expressions(&run.function, &mut 10000, |expr, _| {
        if let K::Let { binding, .. } = &expr.kind {
            secret = Some(*binding);
        }
        Ok(())
    })
    .unwrap();
    let secret = secret.unwrap();
    // Well-formed interface inputs still execute with a caller-local present.
    llvm::emit(&graph, 100000, false, None, None).unwrap();
    for exit in [false, true] {
        let mut input = graph.clone();
        let contract = &mut input.interfaces[0].methods[0];
        assert!(!contract.parameter_bindings.contains(&secret));
        let conditions = if exit {
            &mut contract.program.postconditions
        } else {
            &mut contract.program.preconditions
        };
        conditions[0].kind = K::Local(secret);
        let error = llvm::emit(&input, 100000, false, None, None).unwrap_err();
        assert!(
            error.ends_with("host local binding is unavailable"),
            "{error}"
        );
        let mut interpreter =
            comptime_interpretor::Interpreter::from_iir(&input, Default::default()).unwrap();
        assert_eq!(
            interpreter
                .invoke_instance(&input.roots[0], vec![])
                .unwrap_err()
                .kind,
            comptime_interpretor::ErrorKind::MissingBinding
        );
    }
    // Failed emission must not poison the next independent invocation frame.
    llvm::emit(&graph, 100000, false, None, None).unwrap();
}

#[test]
fn generated_interface_schedule_unwinds_result_and_caller_once() {
    for default in [false, true] {
        let source="trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Token {} impl Retire for Token {fn drop(*self) {}}
        trait Factory {fn make(token:Token,flag:bool)->result:Token where requires flag, ensures flag'old;}
        struct Host {} impl Factory for Host {fn make(token:Token,flag:bool)->Token {token}}
        fn run() {let outer=Token {};Host.as(Factory).make(Token {},true);}";
        let source = if default {
            format!(
                "{} fn consume(.flag:bool=Host.as(Factory).make(true,true)) {{}}",
                source
                    .replace("Host.as(Factory).make(Token {},true);", "consume();")
                    .replace("token:Token", "token:bool")
                    .replace("result:Token", "result:bool")
                    .replace("->Token", "->bool")
            )
        } else {
            source.to_owned()
        };
        let package = source_package(&source);
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let context = crate::resources::ProtocolContext::new(
            &package,
            "Duplicate",
            Some("Retire"),
            Default::default(),
        )
        .unwrap();
        let executable = crate::cleanup::prepare_source(
            &package,
            &context.world(&package),
            &context.items,
            iir::InstanceKey {
                declaration: run.declaration,
                arguments: vec![],
            },
            &mut 100000,
            Default::default(),
        )
        .unwrap();
        let caller = executable
            .schedule
            .functions
            .iter()
            .find(|f| f.instance == executable.entry && f.context.is_empty())
            .unwrap();
        let exit = executable
            .schedule
            .functions
            .iter()
            .flat_map(|f| &f.sites)
            .find(|s| s.boundary == iir::cleanup::Boundary::InterfacePostconditionFailure)
            .unwrap();
        assert!(
            default
                || exit
                    .actions
                    .iter()
                    .any(|a| matches!(a.target, iir::cleanup::Target::Prepared { slot: 2, .. }))
        );
        let owners = if default {
            &caller
                .sites
                .iter()
                .find(|s| matches!(s.boundary, iir::cleanup::Boundary::DefaultFailure(0)))
                .unwrap()
                .actions
        } else {
            &exit.actions
        };
        assert_eq!(
            owners
                .iter()
                .filter(|a| matches!(a.target, iir::cleanup::Target::Local { .. }))
                .count(),
            1
        );
        for failure in [None, Some(false), Some(true)] {
            let mut input = executable.graph.clone();
            if let Some(exit) = failure {
                let program = &mut input.interfaces[0].methods[0].program;
                let checks = if exit {
                    &mut program.postconditions
                } else {
                    &mut program.preconditions
                };
                checks[0].kind = K::Literal(nir::program::Literal::Bool(false));
            }
            let mut vm = comptime_interpretor::Interpreter::from_iir(&input, Default::default())
                .unwrap()
                .with_cleanup_schedule(&executable.schedule)
                .unwrap();
            let result = vm.invoke_instance(&executable.entry, vec![]);
            if failure.is_some() {
                assert_eq!(
                    result.unwrap_err().kind,
                    comptime_interpretor::ErrorKind::ProgramContractRejected
                );
            } else {
                assert!(result.is_ok(), "{result:?}");
            }
            let count = vm
                .events()
                .iter()
                .filter(|e| matches!(e, comptime_interpretor::Event::EnterFunction(_)))
                .count();
            assert_eq!(
                count,
                if default {
                    if failure == Some(false) {
                        2
                    } else if failure.is_some() {
                        3
                    } else {
                        4
                    }
                } else if failure == Some(false) {
                    3
                } else {
                    4
                },
                "{:?}",
                vm.events()
            );
            llvm::validate_cleanup(&input, &executable.schedule, &mut 100000).unwrap();
            let ir = llvm::emit(
                &input,
                100000,
                true,
                Some(&executable.schedule),
                Some(&executable.entry),
            )
            .unwrap();
            for optimization in [OptimizationLevel::O0, OptimizationLevel::O3] {
                let out = Toolchain {
                    optimization,
                    ..Default::default()
                }
                .execute_test_ir(&ir)
                .unwrap();
                assert_eq!(
                    out.status.success(),
                    failure.is_none(),
                    "{failure:?}: {out:?}"
                );
            }
        }
    }
}

#[test]
fn closure_failure_unwinds_persistent_capture_state_once() {
    for (invocation, argument_check) in [
        ("callback(take,passed);", false),
        ("(do {callback})(take,passed);", false),
        ("(do {callback})(take,do {assumes passed;passed});", true),
    ] {
        for rebuild in [false, true] {
            let repair = if rebuild {
                "pair.first=Token {target:count};"
            } else {
                ""
            };
            for take in [false, true] {
                for passed in [false, true] {
                    let expected = if rebuild && !(argument_check && !passed) {
                        3
                    } else {
                        2
                    };
                    let package=source_package(&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                typealias Count=.u32'builtin;impl Duplicate for bool {{}} impl Duplicate for Count {{}}
                struct Token {{target:*Count}}
                impl Retire for Token {{fn drop(*self) {{let n=self.target.*.*;if n < 10 {{self.target.*.* = n + 1;}}}}}}
                struct Pair {{first:Token,second:Token}}
                fn consume(token:Token) {{}}
                fn trial(count:*Count,take:bool,passed:bool) {{
                    var pair=Pair {{first:Token {{target:count}},second:Token {{target:count}}}};
                    let callback=|take:bool,passed:bool| {{if take {{consume(pair.first);}} {repair} assumes passed;}};
                    {invocation}
                }}
                fn run()->bool {{let count:Count=0;trial(count.ref,{take},{passed});count == {expected}}}"));
                    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
                    let drop = package
                        .functions
                        .iter()
                        .find(|f| f.name == "drop")
                        .unwrap()
                        .declaration;
                    let context = crate::resources::ProtocolContext::new(
                        &package,
                        "Duplicate",
                        Some("Retire"),
                        Default::default(),
                    )
                    .unwrap();
                    let executable = crate::cleanup::prepare_source(
                        &package,
                        &context.world(&package),
                        &context.items,
                        iir::InstanceKey {
                            declaration: run.declaration,
                            arguments: vec![],
                        },
                        &mut 100000,
                        Default::default(),
                    )
                    .unwrap();
                    let mut vm = comptime_interpretor::Interpreter::from_iir(
                        &executable.graph,
                        Default::default(),
                    )
                    .unwrap()
                    .with_cleanup_schedule(&executable.schedule)
                    .unwrap();
                    let result = vm.invoke_instance(&executable.entry, vec![]);
                    if passed {
                        assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(true));
                    } else {
                        assert_eq!(
                            result.unwrap_err().kind,
                            comptime_interpretor::ErrorKind::ProgramContractRejected
                        );
                    }
                    assert_eq!(vm.events().iter().filter(|e|matches!(e,comptime_interpretor::Event::EnterFunction(id) if *id==drop)).count(),expected);
                    llvm::validate_cleanup(&executable.graph, &executable.schedule, &mut 100000)
                        .unwrap();
                    let ir = llvm::emit(
                        &executable.graph,
                        100000,
                        true,
                        Some(&executable.schedule),
                        Some(&executable.entry),
                    )
                    .unwrap();
                    let trial = executable
                        .graph
                        .functions
                        .iter()
                        .position(|f| f.function.name == "trial")
                        .unwrap();
                    let (module, _) = ir.split_once("define i32 @main()").unwrap();
                    let ir = format!(
                        "{module}\n@observed=internal global i32 0\ndefine i32 @main() {{\nentry:\n call void @flurry_{trial}(ptr @observed,i1 {take},i1 {passed})\n %count=load i32,ptr @observed\n %failed=load i1,ptr @flurry_failed\n %a=icmp eq i32 %count,{expected}\n %b=icmp eq i1 %failed,{}\n %ok=and i1 %a,%b\n %bad=xor i1 %ok,true\n %code=zext i1 %bad to i32\n ret i32 %code\n}}",
                        !passed
                    );
                    for optimization in [OptimizationLevel::O0, OptimizationLevel::O3] {
                        let out = Toolchain {
                            optimization,
                            ..Default::default()
                        }
                        .execute_test_ir(&ir)
                        .unwrap();
                        assert!(
                            out.status.success(),
                            "take={take}, passed={passed}: {out:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn native_interface_exit_observes_return_value_and_entry_snapshot() {
    let graph = graph(
        "trait Echo {fn echo(flag:bool)->answer:bool where ensures answer == flag'old;}
        struct Host {} impl Echo for Host {fn echo(flag:bool)->bool {flag}}
        fn run()->bool {Host.as(Echo).echo(true)}",
    );
    let method = &graph.interfaces[0].methods[0];
    assert!(method.program.result_binding.is_some());
    assert_eq!(method.program.entry_snapshots.len(), 1);
    let good = llvm::emit(&graph, 100000, false, None, None).unwrap();
    let mut rejected = graph.clone();
    // Change only the implementation's returned value. No body postcondition
    // remains, so failure must be the interface observing this actual result.
    let implementation = rejected
        .functions
        .iter_mut()
        .find(|f| f.function.name == "echo")
        .unwrap();
    implementation.function.body.kind = K::Literal(nir::program::Literal::Bool(false));
    let bad = llvm::emit(&rejected, 100000, false, None, None).unwrap();
    for (input, succeeds) in [(&graph, true), (&rejected, false)] {
        let mut interpreter =
            comptime_interpretor::Interpreter::from_iir(input, Default::default()).unwrap();
        let result = interpreter.invoke_instance(&input.roots[0], vec![]);
        if succeeds {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert_eq!(
                result.unwrap_err().kind,
                comptime_interpretor::ErrorKind::ProgramContractRejected
            );
        }
    }

    for optimization in [OptimizationLevel::O0, OptimizationLevel::O3] {
        let tools = Toolchain {
            optimization,
            ..Default::default()
        };
        assert!(tools.execute_test_ir(&good).unwrap().status.success());
        let failure = tools.execute_test_ir(&bad).unwrap();
        assert!(!failure.status.success());
        let source = method.program.postconditions[0]
            .source
            .original(&graph.expansion_sources)
            .unwrap();
        let diagnostic = String::from_utf8(failure.stderr).unwrap();
        assert!(
            diagnostic.contains(&format!("(predicate {} {})", source.start, source.end)),
            "{diagnostic}"
        );
        assert_eq!(diagnostic.matches("runtime-check-failure").count(), 1);
    }
    // Missing snapshot input must not silently bind a caller local.
    let mut malformed = graph.clone();
    malformed.interfaces[0].methods[0].program.entry_snapshots[0]
        .parameter
        .index = u32::MAX;
    assert!(
        llvm::emit(&malformed, 100000, false, None, None)
            .unwrap_err()
            .contains("snapshot target missing")
    );
}

#[test]
fn interface_exit_cleanup_owns_only_the_pending_result() {
    use iir::cleanup::{Action, Boundary, FunctionSchedule, Program, Schedule, Site, Step, Target};
    let graph = graph(
        "typealias Count=.u32'builtin;
        trait Echo {fn echo(p:*Count,flag:bool)->result:*Count where requires p.* == 0, ensures result.* == 0;}
        struct Host {} impl Echo for Host {fn echo(p:*Count,flag:bool)->*Count {p}}
        fn release(p:**Count) {let n=p.*.*;if n < 10 {p.*.* = n + 1;}}
        fn trial(p:*Count)->*Count where requires p.* == 0 {Host.as(Echo).echo(p,true)}
        fn run()->bool {let count:Count=0;let cleanup=release;trial(count.ref).* == 0}",
    );
    let trial = graph
        .functions
        .iter()
        .position(|f| f.function.name == "trial")
        .unwrap();
    let release = graph
        .functions
        .iter()
        .find(|f| f.function.name == "release")
        .unwrap()
        .key
        .clone();
    let mut call = None;
    nir::visit::expressions(&graph.functions[trial].function, &mut 10000, |expr, _| {
        if matches!(&expr.kind, K::Call {callee, ..} if matches!(callee.kind, K::TraitMethod {..}))
        {
            call = Some(expr.source);
        }
        Ok(())
    })
    .unwrap();
    let source = call.unwrap();
    assert_eq!(
        Boundary::InterfacePostconditionFailure.to_string(),
        "interface-postcondition-failure"
    );
    let mut schedule = Schedule::default();
    for (index, function) in graph.functions.iter().enumerate() {
        schedule.functions.push(FunctionSchedule {
            instance: function.key.clone(),
            context: vec![],
            discards: vec![],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
            sites: if index == trial {
                vec![Site {
                    source,
                    boundary: Boundary::InterfacePostconditionFailure,
                    actions: vec![Action {
                        target: Target::Prepared {
                            construction: source,
                            slot: 2,
                            value_source: source,
                        },
                        program: Program {
                            ty: nir::Term::Pointer(Box::new(nir::Term::Primitive(
                                nir::Primitive::U32,
                            ))),
                            guards: vec![],
                            steps: vec![Step::Invoke {
                                path: vec![],
                                subject: nir::Term::Pointer(Box::new(nir::Term::Primitive(
                                    nir::Primitive::U32,
                                ))),
                                method: release.clone(),
                            }],
                        },
                    }],
                }]
            } else {
                vec![]
            },
        });
    }
    let absent = Schedule {
        functions: schedule
            .functions
            .iter()
            .cloned()
            .map(|mut f| {
                f.sites.clear();
                f
            })
            .collect(),
        ..Default::default()
    };
    assert!(
        llvm::emit(&graph, 100000, true, Some(&absent), None)
            .unwrap_err()
            .contains("unique result cleanup boundary")
    );
    let mut duplicate = schedule.clone();
    let site = duplicate.functions[trial].sites[0].clone();
    duplicate.functions[trial].sites.push(site);
    assert!(
        llvm::emit(&graph, 100000, true, Some(&duplicate), None)
            .unwrap_err()
            .contains("unique result cleanup boundary")
    );
    for invalid in [&absent, &duplicate] {
        let vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default())
            .unwrap()
            .with_cleanup_schedule(invalid);
        let result = match vm {
            Ok(mut vm) => vm.invoke_instance(&graph.roots[0], vec![]),
            Err(error) => Err(error),
        };
        assert_eq!(
            result.unwrap_err().kind,
            comptime_interpretor::ErrorKind::InvalidCallPlan
        );
    }
    for caller_state in 0..3 {
        let caller_cleanup = caller_state > 0;
        let transferred = caller_state == 2;
        for failure in [None, Some(false), Some(true)] {
            let fails = failure.is_some();
            let mut schedule = schedule.clone();
            let mut input = graph.clone();
            let caller_binding = input.functions[trial].function.parameters[0];
            let mut entry = Site {
                source,
                boundary: Boundary::InterfacePreconditionFailure,
                actions: vec![],
            };
            if caller_cleanup {
                // Deliberately overlap a bool observation binding with the caller's
                // pointer owner. Predicates do not refer to this otherwise-unused flag.
                input.interfaces[0].methods[0].parameter_bindings[1] = caller_binding;
                let mut argument_source = None;
                nir::visit::expression_mut(
                    &mut input.functions[trial].function.body,
                    &mut 10000,
                    |expr, _| {
                        if expr.source == source {
                            if let K::Call {
                                parameter_bindings,
                                arguments,
                                ..
                            } = &mut expr.kind
                            {
                                parameter_bindings[1] = caller_binding;
                                argument_source = Some(arguments[0].value.source);
                            }
                        }
                        Ok(())
                    },
                )
                .unwrap();
                let mut action = schedule.functions[trial].sites[0].actions[0].clone();
                action.target = Target::Local {
                    binding: caller_binding,
                    fields: vec![],
                };
                if transferred {
                    schedule.functions[trial]
                        .transfers
                        .push(iir::cleanup::Transfer {
                            source: argument_source.unwrap(),
                            binding: caller_binding,
                            fields: vec![],
                        });
                    action.program.steps = vec![Step::IfConstructed {
                        path: vec![],
                        steps: std::mem::take(&mut action.program.steps),
                    }];
                }
                entry.actions.push(action.clone());
                schedule.functions[trial].sites[0].actions.push(action);
            }
            schedule.functions[trial].sites.push(entry);
            if let Some(exit) = failure {
                let program = &mut input.interfaces[0].methods[0].program;
                let conditions = if exit {
                    &mut program.postconditions
                } else {
                    &mut program.preconditions
                };
                conditions[0].kind = K::Literal(nir::program::Literal::Bool(false));
            }
            let drops = usize::from(failure == Some(true))
                + usize::from(fails && caller_cleanup && !transferred);
            let mut interpreter =
                comptime_interpretor::Interpreter::from_iir(&input, Default::default())
                    .unwrap()
                    .with_cleanup_schedule(&schedule)
                    .unwrap();
            let result = interpreter.invoke_instance(&input.roots[0], vec![]);
            if fails {
                assert_eq!(
                    result.unwrap_err().kind,
                    comptime_interpretor::ErrorKind::ProgramContractRejected
                );
            } else {
                assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(true));
            }
            let entered = interpreter
                .events()
                .iter()
                .filter(|event| matches!(event, comptime_interpretor::Event::EnterFunction(_)))
                .count();
            assert_eq!(
                entered,
                3 - usize::from(failure == Some(false)) + drops,
                "{:?}",
                interpreter.events()
            );
            let ir = llvm::emit(&input, 100000, true, Some(&schedule), None).unwrap();
            let (module, _) = ir.split_once("define i32 @main()").unwrap();
            // Replace only the development entry adapter to inspect memory and
            // failure state after the generated function has unwound.
            let ir = format!(
                "{module}\n@observed = internal global i32 0\ndefine i32 @main() {{\nentry:\n %result = call ptr @flurry_{trial}(ptr @observed)\n %count = load i32, ptr @observed\n %failed = load i1, ptr @flurry_failed\n %a = icmp eq i32 %count, {}\n %b = icmp eq i1 %failed, {}\n %ok = and i1 %a, %b\n %bad = xor i1 %ok, true\n %status = zext i1 %bad to i32\n ret i32 %status\n}}\n",
                drops, fails
            );
            for optimization in [OptimizationLevel::O0, OptimizationLevel::O3] {
                let tools = Toolchain {
                    optimization,
                    ..Default::default()
                };
                let result = tools.execute_test_ir(&ir).unwrap();
                assert!(result.status.success(), "fails={fails}: {result:?}");
            }
        }
    }
}

#[test]
fn interface_invocation_resource_report_keeps_entry_and_result_owners_separate() {
    use elaboration::verification::moves::{CleanupAction, CleanupReason};
    let package = source_package("trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
        struct Token {} impl Retire for Token {fn drop(*self) {}}
        trait Factory {fn make(token:Token,flag:bool)->result:Token where requires flag, requires flag, ensures flag'old, ensures flag'old;}
        struct Host {} impl Factory for Host {fn make(token:Token,flag:bool)->Token {token}}
        fn run() {Host.as(Factory).make(Token {},true);}");
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let mut callee = None;
    nir::visit::expressions(run, &mut 10000, |expr, _| {
        if matches!(expr.kind, K::TraitMethod { .. }) {
            callee = Some(expr.clone());
        }
        Ok(())
    })
    .unwrap();
    let callee = callee.unwrap();
    let context = crate::resources::ProtocolContext::new(
        &package,
        "Duplicate",
        Some("Retire"),
        Default::default(),
    )
    .unwrap();
    let evidence = crate::protocols::analyze_method_cleanup(
        &package,
        &context.world(&package),
        &context.items,
        &callee,
        100000,
        Default::default(),
    )
    .unwrap();
    assert!(
        evidence
            .to_string()
            .starts_with("(method-cleanup (method-invocation (callee")
    );
    assert!(
        crate::protocols::analyze_method_cleanup(
            &package,
            &context.world(&package),
            &context.items,
            &callee,
            0,
            Default::default()
        )
        .is_err()
    );
    let mut actual_call = None;
    nir::visit::expressions(run, &mut 10000, |expr, _| {
        if matches!(&expr.kind,K::Call {callee,..} if matches!(callee.kind,K::TraitMethod {..})) {
            actual_call = Some(expr.clone());
        }
        Ok(())
    })
    .unwrap();
    let actual_call = actual_call.unwrap();
    let mapped =
        crate::cleanup::lower_method_call_cleanup(&evidence, &actual_call, &mut 100000).unwrap();
    assert!(
        mapped
            .to_string()
            .starts_with("(method-call-cleanup (callee")
    );
    assert!(mapped.sites.iter().any(|site|site.boundary == iir::cleanup::Boundary::InterfacePreconditionFailure
        && site.actions.iter().any(|a|matches!(a.target,iir::cleanup::Target::Prepared {construction,slot:0,..} if construction==actual_call.source))));
    assert!(mapped.sites.iter().any(|site|site.boundary == iir::cleanup::Boundary::InterfacePostconditionFailure
        && site.actions.iter().any(|a|matches!(a.target,iir::cleanup::Target::Prepared {construction,slot:2,value_source} if construction==actual_call.source && value_source==actual_call.source))));
    assert_eq!(mapped.roots.len(), 1);
    assert!(crate::cleanup::lower_method_call_cleanup(&evidence, &actual_call, &mut 0).is_err());
    let mut wrong = actual_call.clone();
    let K::Call { arguments, .. } = &mut wrong.kind else {
        unreachable!()
    };
    arguments[1].parameter = 0;
    assert!(crate::cleanup::lower_method_call_cleanup(&evidence, &wrong, &mut 100000).is_err());
    for missing in [false, true] {
        let mut wrong = actual_call.clone();
        let K::Call {
            parameter_bindings, ..
        } = &mut wrong.kind
        else {
            unreachable!()
        };
        if missing {
            parameter_bindings.pop();
        } else {
            parameter_bindings[1] = parameter_bindings[0];
        }
        assert!(
            crate::cleanup::lower_method_call_cleanup(&evidence, &wrong, &mut 100000)
                .unwrap_err()
                .contains("complete unique parameter binding frame")
        );
    }
    let mut wrong = actual_call.clone();
    let K::Call { arguments, .. } = &mut wrong.kind else {
        unreachable!()
    };
    arguments[0].is_default = true;
    assert!(
        crate::cleanup::lower_method_call_cleanup(&evidence, &wrong, &mut 100000)
            .unwrap_err()
            .contains("explicit argument follows a default")
    );
    let K::Call { arguments, .. } = &mut wrong.kind else {
        unreachable!()
    };
    arguments.swap(0, 1);
    arguments[0].is_default = true;
    assert!(
        crate::cleanup::lower_method_call_cleanup(&evidence, &wrong, &mut 100000)
            .unwrap_err()
            .contains("defaults are not in declaration order")
    );
    // Explicit argument evaluation follows source order, which need not be
    // parameter order. Relocation still uses parameter slots and value sources.
    let mut reordered = actual_call.clone();
    let K::Call { arguments, .. } = &mut reordered.kind else {
        unreachable!()
    };
    arguments.swap(0, 1);
    assert_eq!(
        crate::cleanup::lower_method_call_cleanup(&evidence, &reordered, &mut 100000)
            .unwrap()
            .to_string(),
        mapped.to_string()
    );
    let mut wrong = actual_call.clone();
    let K::Call { callee, .. } = &mut wrong.kind else {
        unreachable!()
    };
    callee.kind = K::Function(evidence.invocation.implementation.declaration);
    assert!(crate::cleanup::lower_method_call_cleanup(&evidence, &wrong, &mut 100000).is_err());
    assert_eq!(
        crate::cleanup::lower_method_call_cleanup(&evidence, &actual_call, &mut 100000)
            .unwrap()
            .to_string(),
        mapped.to_string()
    );
    let mut discard_source = None;
    nir::visit::expressions(run, &mut 10000, |expr, _| {
        if matches!(&expr.kind,K::Discard(value) if value.source==actual_call.source) {
            discard_source = Some(expr.source);
        }
        Ok(())
    })
    .unwrap();
    let discard = iir::cleanup::DiscardCleanup {
        source: discard_source.unwrap(),
        value_source: actual_call.source,
        program: mapped
            .sites
            .iter()
            .find(|s| s.boundary == iir::cleanup::Boundary::InterfacePostconditionFailure)
            .unwrap()
            .actions[0]
            .program
            .clone(),
    };
    let root = iir::InstanceKey {
        declaration: run.declaration,
        arguments: vec![],
    };
    let mut roots = vec![root.clone()];
    roots.extend(mapped.roots.iter().cloned());
    let graph = elaboration::materialization::instantiate(
        &package,
        &roots,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let mut schedule = iir::cleanup::Schedule::default();
    for instance in &graph.functions {
        schedule.functions.push(iir::cleanup::FunctionSchedule {
            instance: instance.key.clone(),
            context: vec![],
            sites: if instance.key == root {
                mapped.sites.clone()
            } else {
                vec![]
            },
            discards: if instance.key == root {
                vec![discard.clone()]
            } else {
                vec![]
            },
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
        });
    }
    for failure in [None, Some(false), Some(true)] {
        let mut input = graph.clone();
        let method = input
            .interfaces
            .iter_mut()
            .flat_map(|i| &mut i.methods)
            .find(|m| m.name == "make")
            .unwrap();
        if let Some(at_exit) = failure {
            let conditions = if at_exit {
                &mut method.program.postconditions
            } else {
                &mut method.program.preconditions
            };
            conditions[0].kind = K::Literal(nir::program::Literal::Bool(false));
        }
        let mut vm = comptime_interpretor::Interpreter::from_iir(&input, Default::default())
            .unwrap()
            .with_cleanup_schedule(&schedule)
            .unwrap();
        let result = vm.invoke_instance(&root, vec![]);
        if failure.is_none() {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert_eq!(
                result.unwrap_err().kind,
                comptime_interpretor::ErrorKind::ProgramContractRejected
            );
        }
        let calls = vm
            .events()
            .iter()
            .filter(|event| matches!(event, comptime_interpretor::Event::EnterFunction(_)))
            .count();
        assert_eq!(
            calls,
            match failure {
                None => 3,
                Some(false) => 2,
                Some(true) => 3,
            },
            "{:?}",
            vm.events()
        );
        let ir = llvm::emit(&input, 100000, true, Some(&schedule), Some(&root)).unwrap();
        for optimization in [OptimizationLevel::O0, OptimizationLevel::O3] {
            let tools = Toolchain {
                optimization,
                ..Default::default()
            };
            assert_eq!(
                tools.execute_test_ir(&ir).unwrap().status.success(),
                failure.is_none()
            );
        }
    }
    let invocation = evidence.invocation;
    let report = evidence.report;
    let result = invocation.function.result_binding.unwrap();
    let token = invocation.function.parameters[0];
    let owned = |action: &CleanupAction, binding: nir::program::LocalId| {
        let CleanupAction::Planned { candidate, .. } = action else {
            return false;
        };
        matches!(candidate.as_ref(), CleanupAction::Owned {remaining,..}
            if remaining.place.root.owner == u64::from(binding.owner) && remaining.place.root.local == binding.index)
    };
    assert!(
        report
            .cleanup_sites
            .iter()
            .any(|site| site.reason == CleanupReason::PredicateFailure
                && site.result_origin.is_none()
                && site.actions.iter().any(|a| owned(a, token))),
        "{report}"
    );
    let exits: Vec<_> = report
        .cleanup_sites
        .iter()
        .filter(|site| site.result_origin.is_some())
        .collect();
    assert!(!exits.is_empty(), "{report}");
    assert!(
        exits
            .iter()
            .any(|site| site.actions.iter().any(|a| owned(a, result))),
        "{report}"
    );
    assert!(
        exits
            .iter()
            .all(|site| !site.actions.iter().any(|a| owned(a, token))),
        "{report}"
    );
    assert!(exits.iter().flat_map(|site| &site.actions).any(|action| {
        let CleanupAction::Planned {candidate, plan} = action else {return false;};
        matches!(candidate.as_ref(),CleanupAction::Owned {remaining,..} if remaining.place.root.owner == u64::from(result.owner) && remaining.place.root.local == result.index)
            && plan.steps.iter().any(|step|matches!(step,elaboration::verification::drop::CleanupStep::Invoke {..}))
    }), "result must carry the acquired user Drop plan: {report}");
    let K::Call { callee, .. } = &invocation.function.body.kind else {
        panic!("implementation call boundary")
    };
    assert!(
        matches!(&callee.kind,K::FunctionInstance {declaration,arguments}
        if *declaration == invocation.implementation.declaration && *arguments == invocation.implementation.arguments)
    );
}

#[test]
fn closure_cleanup_definitions_require_unique_concrete_evidence() {
    let package = source_package(
        "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {}
         fn make(value:T)->%{value:T} fn()->T where T {||value}
         fn run()->bool {let n:.u8'builtin=1;let first=make(true);let second=make(n);first() and second()==1}",
    );
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let context = crate::resources::ProtocolContext::new(
        &package,
        "Duplicate",
        Some("Retire"),
        Default::default(),
    )
    .unwrap();
    let executable = crate::cleanup::prepare_source(
        &package,
        &context.world(&package),
        &context.items,
        iir::InstanceKey {
            declaration: run.declaration,
            arguments: vec![],
        },
        &mut 100000,
        Default::default(),
    )
    .unwrap();
    let schedule = &executable.schedule;
    assert_eq!(schedule.closure_definitions.len(), 2);
    let emit = |schedule: &iir::cleanup::Schedule, fuel| {
        llvm::emit(
            &executable.graph,
            fuel,
            true,
            Some(schedule),
            Some(&executable.entry),
        )
    };
    let original = emit(schedule, 100000).unwrap();
    assert!(emit(schedule, 0).is_err());
    let mut missing = schedule.clone();
    missing.closure_definitions.clear();
    assert!(emit(&missing, 100000).is_err());
    let mut duplicate = schedule.clone();
    duplicate
        .closure_definitions
        .push(duplicate.closure_definitions[0].clone());
    assert!(emit(&duplicate, 100000).is_err());
    let mut forged = schedule.clone();
    forged.closure_definitions[0].definition.kind = K::Unit;
    assert!(emit(&forged, 100000).is_err());
    assert_eq!(emit(schedule, 100000).unwrap(), original);
    let dump = schedule.to_string();
    assert!(dump.contains("(closure-definition (instance "));
    assert!(dump.contains("(definition "));
    assert_eq!(
        dump.chars().filter(|c| *c == '(').count(),
        dump.chars().filter(|c| *c == ')').count()
    );
}

#[test]
fn destructor_receiver_cannot_be_rebound_to_a_layout_equivalent_newtype() {
    for generic in [false, true] {
        let declarations = if generic {
            "newtype Wrapped<T> = bool;impl Retire for Wrapped<T> where T {fn drop(*self) {}}
         fn run()->bool {true.as(Wrapped<bool>);false.as(Wrapped<.u32'builtin>);true}"
        } else {
            "newtype First=bool;impl Retire for First {fn drop(*self) {}}
         newtype Second=bool;impl Retire for Second {fn drop(*self) {}}
         fn run()->bool {true.as(First);false.as(Second);true}"
        };
        let package = source_package(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}impl Duplicate for bool {{}} {declarations}"
        ));
        let identity = |name| {
            package
                .traits
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .declaration
        };
        let prepared = prepare_with_protocols(
            &package,
            "run",
            100_000,
            identity("Duplicate"),
            identity("Retire"),
        )
        .unwrap();
        let valid = llvm_ir_prepared(&prepared, 100_000).unwrap();
        let mut malformed = prepared.graph().clone();
        let nominal = |name| nir::Term::Nominal {
            declaration: package
                .newtypes
                .iter()
                .find(|t| t.name == if generic { "Wrapped" } else { name })
                .unwrap()
                .declaration,
            arguments: if generic {
                vec![nir::Term::Primitive(if name == "First" {
                    nir::Primitive::Bool
                } else {
                    nir::Primitive::U32
                })]
            } else {
                vec![]
            },
        };
        let first = nir::Term::Pointer(Box::new(nominal("First")));
        let second = nir::Term::Pointer(Box::new(nominal("Second")));
        let destructor = malformed
            .functions
            .iter_mut()
            .find(|f| {
                f.function.name == "drop"
                    && matches!(&f.function.signature,
            nir::Term::Function {parameters, ..} if parameters[0].ty == first)
            })
            .unwrap();
        let nir::Term::Function { parameters, .. } = &mut destructor.function.signature else {
            unreachable!()
        };
        parameters[0].ty = second;
        let error = llvm::emit(
            &malformed,
            100_000,
            true,
            Some(prepared.schedule()),
            Some(&prepared.executable.entry),
        )
        .unwrap_err();
        assert!(
            error.contains("host destructor ABI requires a subject pointer"),
            "{error}"
        );
        assert_eq!(llvm_ir_prepared(&prepared, 100_000).unwrap(), valid);
    }
}

#[test]
fn specialized_call_failures_preserve_prepared_and_callee_cleanup_order() {
    for closure in [false, true] {
        for argument_check in [false, true] {
            for passed in [false, true] {
                let body = "
                    let local=Token {count:count,digit:3};
                    value match {
                        .none=>(),
                        .some(token)=>do {
                            let active=Token {count:count,digit:4};
                            assumes passed;
                        },
                    };
                ";
                let (definition, setup, empty, full) = if closure {
                    (
                        String::new(),
                        format!("let consume=|value:Choice,passed:bool| {{{body}}};"),
                        "consume(Choice.none,true)".to_owned(),
                        "consume(Choice.some(Token {count:count,digit:2}),CHECK)".to_owned(),
                    )
                } else {
                    (
                        format!("fn consume(value:Choice,count:*Count,passed:bool) {{{body}}}"),
                        String::new(),
                        "consume(Choice.none,count,true)".to_owned(),
                        "consume(Choice.some(Token {count:count,digit:2}),count,CHECK)".to_owned(),
                    )
                };
                let full = full.replace(
                    "CHECK",
                    if argument_check {
                        "do {assumes passed;true}"
                    } else {
                        "passed"
                    },
                );
                let expected = if argument_check && !passed {
                    321
                } else {
                    34231
                };
                let drops = if argument_check && !passed { 3 } else { 5 };
                let package = source_package(&format!(
                    "
                    trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                    typealias Count=.u32'builtin;
                    impl Duplicate for Count {{}} impl Duplicate for bool {{}}
                    struct Token {{count:*Count,digit:Count}}
                    impl Retire for Token {{fn drop(*self) {{
                        let n=self.count.*.*;let digit=self.digit.*;
                        if n < 100000 and digit <= 9 {{self.count.*.* = n * 10 + digit;}}
                    }}}}
                    enum Choice {{none,some(Token)}}
                    {definition}
                    fn trial(count:*Count,passed:bool) {{
                        let outer=Token {{count:count,digit:1}};
                        {setup}
                        {empty};
                        {full};
                    }}
                    fn run()->bool {{let count:Count=0;trial(count.ref,{passed});count=={expected}}}
                "
                ));
                let run = package.functions.iter().find(|f| f.name == "run").unwrap();
                let drop = package
                    .functions
                    .iter()
                    .find(|f| f.name == "drop")
                    .unwrap()
                    .declaration;
                let context = crate::resources::ProtocolContext::new(
                    &package,
                    "Duplicate",
                    Some("Retire"),
                    Default::default(),
                )
                .unwrap();
                let executable = crate::cleanup::prepare_source(
                    &package,
                    &context.world(&package),
                    &context.items,
                    iir::InstanceKey {
                        declaration: run.declaration,
                        arguments: vec![],
                    },
                    &mut 1000000,
                    Default::default(),
                )
                .unwrap();
                let mut vm = comptime_interpretor::Interpreter::from_iir(
                    &executable.graph,
                    Default::default(),
                )
                .unwrap()
                .with_cleanup_schedule(&executable.schedule)
                .unwrap();
                let result = vm.invoke_instance(&executable.entry, vec![]);
                if passed {
                    assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(true));
                } else {
                    assert_eq!(
                        result.unwrap_err().kind,
                        comptime_interpretor::ErrorKind::ProgramContractRejected
                    );
                }
                assert_eq!(vm.events().iter().filter(|event|matches!(event,comptime_interpretor::Event::EnterFunction(id) if *id==drop)).count(),drops);
                let ir = llvm::emit(
                    &executable.graph,
                    1000000,
                    true,
                    Some(&executable.schedule),
                    Some(&executable.entry),
                )
                .unwrap();
                let trial = executable
                    .graph
                    .functions
                    .iter()
                    .position(|f| f.function.name == "trial")
                    .unwrap();
                let (module, _) = ir.split_once("define i32 @main()").unwrap();
                // Only replace the development entry adapter. Inspect actual
                // cleanup writes after generated failure propagation completes.
                let ir = format!(
                    "{module}
                    @observed = internal global i32 0
                    define i32 @main() {{
                    entry:
                        call void @flurry_{trial}(ptr @observed, i1 {passed})
                        %count = load i32, ptr @observed
                        %failed = load i1, ptr @flurry_failed
                        %a = icmp eq i32 %count, {expected}
                        %b = icmp eq i1 %failed, {}
                        %ok = and i1 %a, %b
                        %bad = xor i1 %ok, true
                        %status = zext i1 %bad to i32
                        ret i32 %status
                    }}",
                    !passed
                );
                for optimization in [OptimizationLevel::O0, OptimizationLevel::O3] {
                    let output = Toolchain {
                        optimization,
                        ..Default::default()
                    }
                    .execute_test_ir(&ir)
                    .unwrap();
                    assert!(
                        output.status.success(),
                        "closure={closure}, argument_check={argument_check}, passed={passed}: {output:?}"
                    );
                }
            }
        }
    }
}
