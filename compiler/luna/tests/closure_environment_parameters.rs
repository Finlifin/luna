//! Baselines for migrating fixed closure environments to context parameters.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn effects_deliver_open_environment_callbacks() {
    let source = "typealias Type=.Type'builtin;
        inline effect relay(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure;
        fn forward(callback:%c fn()->bool)->#relay<c> %c fn()->bool where c:Type.vars.Closure {relay(callback)#}
        fn install(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure {
            forward(callback)# {relay<c>(callback)=>callback,}
        }
        fn run()->bool {let yes=true;let no=false;
            let first=install(|| yes);
            let second=install(|| no);
            first() and not second()
        }";
    for source in [source.to_owned(), source.replace("inline effect", "effect")] {
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn inline_effect_delivered_callbacks_preserve_pointer_lifetimes() {
    check_effect_delivered_pointer_lifetimes(true);
}

#[test]
fn ordinary_effect_delivered_callbacks_preserve_pointer_lifetimes() {
    check_effect_delivered_pointer_lifetimes(false);
}

#[test]
fn ordinary_effect_pointer_callbacks_cross_forwarders_and_resume() {
    for body in [
        "callback",
        "do {let saved=callback;saved}",
        "do {resume callback;}",
    ] {
        let declarations = format!("typealias Type=.Type'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
            effect relay(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure;
            fn forward(callback:%c fn()->bool)->#relay<c> %c fn()->bool where c:Type.vars.Closure {{relay(callback)#}}
            fn install(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure {{
                forward(callback)# {{relay<c>(callback)=>{body},}}
            }}");
        let valid = format!(
            "{declarations}
            fn run()->bool {{var first=false;var second=true;
                let p=first.ref;let q=second.ref;
                let a=install(|| do {{p.* = not p.*;p.*}});
                let b=install(|| do {{q.* = not q.*;q.*}});
                a() and not b() and first and not second
            }}"
        );
        let package = support::compile(&valid);
        elaboration::verification::memory::infer_in_package(
            &package,
            support::id(&package, "run"),
            &[],
            &mut 200_000,
        )
        .unwrap_or_else(|reason| panic!("{body}: {reason}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000
            )
            .unwrap_or_else(|reason| panic!("{body}: {reason}")),
            Value::Bool(true)
        );
        let invalid = format!("{declarations}
            fn run()->bool {{let callback=do {{var flag=false;let p=flag.ref;install(|| p.*)}};callback()}}");
        let package = support::compile(&invalid);
        let failure = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            200_000,
        )
        .unwrap_err();
        assert!(
            failure.starts_with("cannot prepare resource-checked execution"),
            "{failure}"
        );
    }
}

fn check_effect_delivered_pointer_lifetimes(inline: bool) {
    let declarations = "typealias Type=.Type'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        inline effect relay(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure;
        fn install(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure {
            relay(callback)# {relay<c>(callback)=>callback,}
        }";
    let declarations = if inline {
        declarations.to_owned()
    } else {
        declarations.replace("inline effect", "effect")
    };
    let valid = format!(
        "{declarations}
        fn run()->bool {{var flag=false;let target=flag.ref;
            let callback=install(|| do {{target.* = not target.*;target.*}});
            callback() and not callback() and not flag
        }}"
    );
    let package = support::compile(&valid);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
    let invalid = format!("{declarations}
        fn run()->bool {{let callback=do {{var flag=false;let target=flag.ref;install(|| target.*)}};callback()}}");
    let package = support::compile(&invalid);
    assert!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .is_err()
    );
}

#[test]
fn inline_effect_delivered_callbacks_retain_capture_consumption() {
    check_effect_delivered_capture_consumption(true);
}

#[test]
fn ordinary_effect_delivered_callbacks_retain_capture_consumption() {
    check_effect_delivered_capture_consumption(false);
}

fn check_effect_delivered_capture_consumption(inline: bool) {
    for (calls, accepted) in [("callback()", true), ("callback() and callback()", false)] {
        let source = format!(
            "typealias Type=.Type'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}
            struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}}
            fn take(value:Ticket)->bool {{true}}
            inline effect relay(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure;
            fn install(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure {{
                relay(callback)# {{relay<c>(callback)=>callback,}}
            }}
            fn run()->bool {{let ticket=Ticket{{}};let callback=install(|| take(ticket));{calls}}}"
        );
        let source = if inline {
            source
        } else {
            source.replace("inline effect", "effect")
        };
        let package = support::compile(&source);
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        );
        assert_eq!(result.is_ok(), accepted, "{calls}: {result:?}");
        if accepted {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert!(result.unwrap_err().contains("unavailable"));
        }
    }
}

#[test]
fn ordinary_effect_transfer_merges_matching_branch_and_fallback_origins() {
    for handler in [
        "relay(flag,callback)# {relay<c>(true,callback)=>callback,relay<c>(false,callback)=>do {resume callback;},}",
        "relay(flag,callback)# {relay<c>(true,callback)=>callback,}# {relay<c>(flag,callback)=>callback,}",
        "relay(flag,callback)# {relay<c>(flag,callback)=>if flag {callback} else {callback},}",
        "relay(flag,callback)# {relay<c>(flag,callback)=>flag match {true=>callback,false=>do {resume callback;}},}",
        "(relay(flag,callback)# {relay<c>(flag,callback) if flag=>callback,})# {relay<c>(flag,callback)=>callback,}",
    ] {
        for flag in ["true", "false"] {
            let source=format!("typealias Type=.Type'builtin;
                trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                impl Duplicate for bool {{}}
                struct Ticket {{}} impl Retire for Ticket {{fn drop(*self) {{}}}}
                fn take(value:Ticket)->bool {{true}}
                effect relay(flag:bool,callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure;
                fn install(flag:bool,callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure {{{handler}}}
                fn run()->bool {{let ticket=Ticket{{}};let callback=install({flag},|| take(ticket));callback()}}");
            let package = support::compile(&source);
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000
                )
                .unwrap(),
                Value::Bool(true),
                "{flag}: {handler}"
            );
        }
    }
}

#[test]
fn ordinary_effect_transfer_preserves_local_aliases() {
    let source = "typealias Type=.Type'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        fn take(value:Ticket)->bool {true}
        effect relay(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure;
        fn install(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure {
            relay(callback)# {relay<c>(callback)=>do {let retained=callback;retained},}
        }
        fn run()->bool {let ticket=Ticket{};let callback=install(|| take(ticket));callback()}";
    for body in [
        "let retained=callback;retained",
        "let first=callback;let retained=first;retained",
        "let retained=do {let inner=callback;inner};retained",
        "let retained=callback;resume retained;",
        "resume callback;",
    ] {
        let source = source.replace("let retained=callback;retained", body);
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn ordinary_effect_transfer_does_not_restore_consumed_captures() {
    let source = "typealias Type=.Type'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {}
        struct Ticket {} impl Retire for Ticket {fn drop(*self) {}}
        fn take(value:Ticket)->bool {true}
        effect relay(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure;
        fn install(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure {
            relay(callback)# {relay<c>(callback)=>callback,}
        }
        fn run()->bool {let ticket=Ticket{};let callback=|| take(ticket);callback();let returned=install(callback);returned()}";
    let package = support::compile(source);
    let failure =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        failure.starts_with("cannot prepare resource-checked execution"),
        "{failure}"
    );
}

#[test]
fn returned_open_environment_does_not_extend_pointee_lifetime() {
    let source = "typealias Type=.Type'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        fn retain(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure {callback}
        fn run()->bool {
            let callback=do {var flag=false;let target=flag.ref;retain(|| target.*)};
            callback()
        }";
    let package = support::compile(source);
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let result = elaboration::verification::moves::analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    elaboration::verification::moves::Delivery::Copy
                } else {
                    elaboration::verification::moves::Delivery::Move
                },
            )
        },
        100_000,
    );
    assert!(result.is_err(), "dangling returned environment accepted");
    let error = result.unwrap_err().to_string();
    assert!(!error.contains("expected type"), "{error}");
    assert!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .is_err()
    );
}

#[test]
fn open_environment_preserves_pointer_identity_and_mutation() {
    let source = "typealias Type=.Type'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        fn retain(callback:%c fn()->bool)->%c fn()->bool where c:Type.vars.Closure {callback}
        fn run()->bool {
            var flag=false;let target=flag.ref;
            let callback=retain(|| do {target.* = not target.*;target.*});
            callback() and not callback() and not flag
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let package = support::compile(source);
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    elaboration::verification::moves::analyze_in_package(
        &package,
        run,
        |ty| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    elaboration::verification::moves::Delivery::Copy
                } else {
                    elaboration::verification::moves::Delivery::Move
                },
            )
        },
        100_000,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn open_environment_checks_consumed_capture_before_repeated_invocation() {
    for (calls, accepted) in [("callback()", true), ("callback() and callback()", false)] {
        let source = format!(
            "typealias Type=.Type'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}
            struct Ticket {{}}
            impl Retire for Ticket {{fn drop(*self) {{}}}}
            fn take(value:Ticket)->bool {{true}}
            fn invoke(callback:%c fn()->bool)->bool where c:Type.vars.Closure {{{calls}}}
            fn run()->bool {{let ticket=Ticket {{}};invoke(|| take(ticket))}}"
        );
        let package = support::compile(&source);
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        );
        assert_eq!(result.is_ok(), accepted, "{calls}: {result:?}");
        if accepted {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn open_environment_type_values_complete_during_comptime_invocation() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        comptime fn Describe(callback:%c comptime fn()->T)->Type where c:Type.vars.Closure,T {
            %c comptime fn()->T
        }
        comptime fn captured()->Type {let flag=true;Describe(|| flag)}
        comptime fn empty()->Type {Describe(|| true)}",
    );
    let mut interpreter =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let signature = nir::Term::ComptimeFunction(Box::new(nir::Term::Function {
        parameters: vec![],
        result: Box::new(nir::Term::Primitive(nir::Primitive::Bool)),
    }));
    assert_eq!(
        interpreter
            .invoke(support::id(&package, "empty"), vec![])
            .unwrap(),
        Value::Type(signature.clone())
    );
    assert_eq!(
        interpreter
            .invoke(support::id(&package, "captured"), vec![])
            .unwrap(),
        Value::Type(nir::Term::Closure {
            environment: vec![nir::Term::Primitive(nir::Primitive::Bool)],
            signature: Box::new(signature)
        })
    );
}

#[test]
fn open_environment_preserves_returned_callbacks() {
    let source = "typealias Type=.Type'builtin;
        fn retain(callback:%c fn()->T)->%c fn()->T where c:Type.vars.Closure,T {
            let retained:%c fn()->T = callback;
            retained
        }
        fn run()->bool {
            let yes=true;let no=false;
            let first=retain(|| yes);let second=retain(|| no);let empty=retain(|| true);
            first() and not second() and empty()
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn open_environment_source_infers_empty_and_captured_callbacks() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        fn invoke(callback:%c fn()->T)->T where c:Type.vars.Closure,T {callback()}
        fn run()->bool {let flag=true;invoke(|| flag) and invoke(|| true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn open_environment_forwards_through_generic_callers_to_iir() {
    let source = "typealias Type=.Type'builtin;
        fn invoke(callback:%c fn()->T)->T where c:Type.vars.Closure,T {callback()}
        fn forward(callback:%c fn()->T)->T where c:Type.vars.Closure,T {invoke(callback)}
        fn run()->bool {
            let yes=true;let no=false;
            forward(|| yes) and not forward(|| no) and forward(|| 'x')=='x'
                and forward(|| true)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn open_environment_domain_uses_type_identity_and_rejects_wrong_domains() {
    let source = "typealias Universe=.Type'builtin;
        fn invoke(callback:%c fn()->bool)->bool where c:Universe.vars.Closure {callback()}
        fn run()->bool {let yes=true;invoke(|| yes)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for domain in ["bool", "Type", "Type.vars.Closure"] {
        let source = format!(
            "typealias Type=bool;
            fn invoke(callback:%c fn()->bool)->bool where c:{domain} {{callback()}}
            fn run()->bool {{invoke(|| true)}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{domain}"
        );
    }
}

#[test]
fn fixed_environment_parameters_infer_capture_and_result_types_independently() {
    for body in [
        "let flag=true;invoke(|| flag)",
        "let flag=true;invoke(|| if flag {'x'} else {'y'})=='x'",
        "let flag='x';invoke(|| flag=='x')",
    ] {
        let source = format!(
            "fn invoke(callback:%{{flag:T}} fn()->U)->U where T,U {{callback()}}
            fn run()->bool {{{body}}}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn fixed_environment_keeps_capture_values_and_body_identity_separate() {
    let package = support::compile(
        "fn invoke(callback:%{flag:T} fn()->U)->U where T,U {callback()}
        fn run()->bool {
            let yes=true; let no=false;
            invoke(|| yes) and not invoke(|| no) and invoke(|| not no)
        }",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn fixed_environment_does_not_accept_missing_or_extra_captures() {
    for callback in ["|| true", "|| first and second"] {
        let source = format!(
            "fn invoke(callback:%{{flag:T}} fn()->bool)->bool where T {{callback()}}
            fn run()->bool {{let first=true;let second=false;invoke({callback})}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{callback}"
        );
    }
}
