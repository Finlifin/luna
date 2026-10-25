//! Deferred source signatures: ordinary checking, specialization and runtime contracts.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{calls, compile, compile_with_protocols, compile_with_source_bindings, id};

#[test]
fn explicit_clause_inputs_specialize_deferred_open_annotations() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,value:defer Select(flag,T))->defer Select(flag,T) where T {value}
        fn byte()->Byte {choose<Byte>(true,9)}
        fn boolean()->bool {choose<bool>(true,true)}
        fn alternate()->bool {choose<Byte>(false,true)}",
    );
    for (entry, expected) in [
        ("byte", Value::UnsignedInteger(9)),
        ("boolean", Value::Bool(true)),
        ("alternate", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn explicit_open_clauses_specialize_known_domains_and_clause_defaults() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn known(comptime flag:bool,value:T)->defer Select(flag,T) where T {value}
        fn defaulted(comptime flag:bool,value:defer Select(flag,U))->defer Select(flag,U)
            where T,.U:Type=T {value}
        fn byte()->Byte {known<Byte>(true,defaulted<Byte>(true,9))}
        fn boolean()->bool {defaulted<Byte,.U=bool>(true,true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "byte", 100_000).unwrap(),
        Value::UnsignedInteger(9)
    );
    assert_eq!(
        luna::execution::interpret(&package, "boolean", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn explicit_open_clauses_admit_requirements_and_reject_wrong_domains() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,value:defer Select(flag,T))->defer Select(flag,T)
            where T,requires T == bool {value}";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{choose<bool>(true,true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for call in [
        "choose<Byte>(true,9)",
        "choose<bool>(true,9)",
        "choose<bool,bool>(true,true)",
    ] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->bool {{{call}}}"),
                &Default::default()
            )
            .is_err(),
            "{call}"
        );
    }
}

#[test]
fn explicit_open_clauses_keep_generic_caller_inputs_bound() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,value:defer Select(flag,T))->defer Select(flag,T) where T {value}
        fn forward(value:T)->T where T {choose<T>(true,value)}
        fn byte()->Byte {forward<Byte>(9)}
        fn boolean()->bool {forward<bool>(true)}",
    );
    for (entry, expected) in [
        ("byte", Value::UnsignedInteger(9)),
        ("boolean", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn open_generic_definitions_can_forward_to_other_open_generics() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,value:defer Select(flag,T))->defer Select(flag,T) where T {value}
        fn forward(comptime flag:bool,value:defer Select(flag,T))->defer Select(flag,T)
            where T {choose<T>(flag,value)}
        fn byte()->Byte {forward<Byte>(true,9)}
        fn boolean()->bool {forward<Byte>(false,true)}",
    );
    for (entry, expected) in [
        ("byte", Value::UnsignedInteger(9)),
        ("boolean", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn open_generic_forwarding_checks_definition_side_requirements() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,value:defer Select(flag,T))->defer Select(flag,T)
            where T,requires T == bool {value}";
    let forward = "fn forward(comptime flag:bool,value:defer Select(flag,T))->defer Select(flag,T)
        where T,requires T == bool {choose<T>(flag,value)}";
    let package = compile(&format!(
        "{prefix} {forward} fn run()->bool {{forward<bool>(true,true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let unconstrained = forward.replace("where T,requires T == bool", "where T");
    assert!(
        compile_with_protocols(&format!("{prefix} {unconstrained}"), &Default::default()).is_err()
    );
    let illegal = forward.replace("choose<T>(flag,value)", "choose<T>(flag,value).missing");
    assert!(compile_with_protocols(&format!("{prefix} {illegal}"), &Default::default()).is_err());
}

#[test]
fn open_generic_forwarding_does_not_invoke_pending_annotations() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn Loop()->Type {Loop()}
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {Loop()}}
        fn choose(comptime flag:bool,value:defer Select(flag,T))->defer Select(flag,T) where T {value}
        fn forward(comptime flag:bool,value:defer Select(flag,T))->defer Select(flag,T)
            where T {choose<T>(flag,value)}
        fn run()->bool {forward<bool>(true,true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn open_clause_inference_uses_formed_inputs_and_result_context() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn from_input(comptime flag:bool,value:T)->defer Select(flag,T) where T {value}
        fn from_result(comptime flag:bool,value:defer Select(flag,T))->T where T {value}
        fn byte()->Byte {let n:Byte=9;from_input(true,n)}
        fn boolean()->bool {from_input(true,true)}
        fn target()->Byte {from_result(true,9)}
        fn defaulted()->.i32'builtin {from_input(true,9)}",
    );
    for (entry, expected) in [
        ("byte", Value::UnsignedInteger(9)),
        ("boolean", Value::Bool(true)),
        ("target", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn open_clause_inference_preserves_argument_order_and_once_evaluation() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,first:T,second:T)->defer Select(flag,T) where T {first}
        fn mark(p:*bool)->bool {p.* = not p.*;p.*}
        fn run()->bool {var count=false;
            let result=choose(true,mark(count.ref),mark(count.ref));
            not count and result}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn open_clause_inference_checks_defaults_conflicts_and_missing_evidence() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,a:T,b:T)->defer Select(flag,U) where T,.U:Type=T {b}";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{choose(true,true,false)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
    for call in ["choose(true,true,9)", "choose(false,9,true)"] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->bool {{{call}}}"),
                &Default::default()
            )
            .is_err(),
            "{call}"
        );
    }
    let missing = "typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn unknown(comptime flag:bool,value:defer Select(flag,T))->defer Select(flag,T) where T {value}
        fn bad()->bool {unknown(true,true)}";
    let failure = compile_with_protocols(missing, &Default::default()).unwrap_err();
    assert!(failure.contains("inference evidence"), "{failure}");
}

#[test]
fn open_clause_inference_accepts_known_domain_comptime_operands() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        comptime fn number()->Byte {9}
        fn choose(comptime flag:bool,comptime value:T)->defer Select(flag,T) where T {value}
        fn byte()->Byte {choose(true,number())}
        fn boolean()->bool {choose(true,true)}",
    );
    for (entry, expected) in [
        ("byte", Value::UnsignedInteger(9)),
        ("boolean", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn inferred_comptime_domains_prepare_defaults_and_overrides() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        comptime fn number()->Byte {9}
        fn choose(comptime flag:bool,comptime value:T,comptime .other:T=value)
            ->defer Select(flag,T) where T {other}
        fn byte()->Byte {choose(true,number())}
        fn boolean()->bool {choose(true,true,.other=false)}",
    );
    for (entry, expected) in [
        ("byte", Value::UnsignedInteger(9)),
        ("boolean", Value::Bool(false)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn inferred_comptime_domains_do_not_admit_runtime_values() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,comptime value:T)->defer Select(flag,T) where T {value}";
    for source in [
        "fn bad(value:Byte)->Byte {choose(true,value)}",
        "fn number()->Byte {9} fn bad()->Byte {choose(true,number())}",
    ] {
        assert!(
            compile_with_protocols(&format!("{prefix} {source}"), &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn open_definitions_infer_forwarded_clauses_from_known_descriptions() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,value:T)->defer Select(flag,T) where T {value}
        fn forward(comptime flag:bool,value:T)->defer Select(flag,T) where T {choose(flag,value)}
        fn byte()->Byte {forward<Byte>(true,9)}
        fn boolean()->bool {forward<bool>(true,true)}",
    );
    for (entry, expected) in [
        ("byte", Value::UnsignedInteger(9)),
        ("boolean", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn inferred_open_descriptions_preserve_type_requirements() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,value:T)->defer Select(flag,T)
            where T,requires T == bool {value}";
    let forward = "fn forward(comptime flag:bool,value:T)->defer Select(flag,T)
        where T,requires T == bool {choose(flag,value)}";
    let package = compile(&format!(
        "{prefix} {forward} fn run()->bool {{forward<bool>(true,true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile_with_protocols(
        &format!(
            "{prefix} {}",
            forward.replace("where T,requires T == bool", "where T")
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        failure.contains("requirement") || failure.contains("constraint"),
        "{failure}"
    );
}

#[test]
fn inferred_open_descriptions_preserve_stages_and_pending_domains() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,comptime value:T)->defer Select(flag,T) where T {value}";
    let forward = "fn forward(comptime flag:bool,comptime value:T)->defer Select(flag,T)
        where T {choose(flag,value)}";
    let package = compile(&format!(
        "{prefix} {forward} fn run()->bool {{forward<bool>(true,true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let failure = compile_with_protocols(
        &format!(
            "{prefix} {}",
            forward.replace("comptime value:T", "value:T")
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("runtime parameter"), "{failure}");
    let pending = "fn pending(comptime flag:bool,value:defer Select(flag,T))->defer Select(flag,T)
        where T {choose(flag,value)}";
    let runtime_prefix = prefix
        .replace("comptime value:T", "value:T")
        .replace("where T {value}", "where T,Phantom {value}");
    let failure =
        compile_with_protocols(&format!("{runtime_prefix} {pending}"), &Default::default())
            .unwrap_err();
    assert!(failure.contains("inference evidence"), "{failure}");
}

#[test]
fn generic_open_recursion_keeps_nested_clause_environments_separate() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn recur(comptime flag:bool,value:T,n:Byte)->defer Select(flag,T) where T {
            if n == 0 {value} else {
                let other=recur<Byte>(true,9,0);
                recur(flag,value,n - 1)
            }
        }
        fn boolean()->bool {recur<bool>(true,true,3)}
        fn byte()->Byte {recur<Byte>(true,7,3)}";
    let package = compile(source);
    for (entry, expected) in [
        ("boolean", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(7)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    assert_eq!(
        package
            .functions
            .iter()
            .filter(|function| function.name == "recur")
            .count(),
        2
    );
    assert_eq!(package.to_string(), compile(source).to_string());
    let invalid = source.replace(
        "let other=recur<Byte>(true,9,0)",
        "let other=recur<Byte>(true,value,0)",
    );
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn open_generic_inference_waits_for_unknown_operand_domains() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Byte}}
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn inner(comptime flag:bool,value:T)->defer Select(flag,T) where T {value}
        fn outer(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {inner(true,value)}
        fn byte()->Byte {outer(false,9)}
        fn boolean()->bool {outer(true,true)}",
    );
    for (entry, expected) in [
        ("byte", Value::UnsignedInteger(9)),
        ("boolean", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn pending_open_inference_preserves_known_results_and_checks_other_operands() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Byte}}
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn accept(comptime flag:bool,value:T,other:defer Select(flag,T))->bool where T {true}";
    let outer = "fn outer(comptime flag:bool,value:defer Pick(flag))->bool {accept(true,value,value) and true}";
    let package = compile(&format!(
        "{prefix} {outer}
        fn run()->bool {{outer(false,9) and outer(true,true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let bad = outer.replace(
        "accept(true,value,value)",
        "accept(true,value,value.missing)",
    );
    assert!(compile_with_protocols(&format!("{prefix} {bad}"), &Default::default()).is_err());
    let constrained = prefix.replace("where T {true}", "where T,requires T == bool {true}");
    assert!(
        compile_with_protocols(&format!("{constrained} {outer}"), &Default::default()).is_err()
    );
}

#[test]
fn pending_open_inference_follows_defaults_but_rejects_phantom_parameters() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Byte}}
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn inner(comptime flag:bool,value:T)->defer Select(flag,U) where T,.U:Type=T {value}
        fn outer(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {inner(true,value)}";
    let package = compile(&format!("{prefix} fn run()->Byte {{outer(false,9)}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(9)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let phantom = prefix.replace("where T,.U:Type=T", "where T,Phantom,.U:Type=T");
    let failure = compile_with_protocols(&phantom, &Default::default()).unwrap_err();
    assert!(failure.contains("inference evidence"), "{failure}");
}

#[test]
fn open_clause_inference_uses_selected_default_type_evidence() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,.value:T=true)->defer Select(flag,T)
            where T,requires T == bool {value}
        fn run()->bool {choose(true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn open_default_inference_preserves_earlier_runtime_bindings() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,seed:bool,.value:T=seed)->defer Select(flag,T)
            where T,requires T == bool {value}
        fn run()->bool {not choose(true,false) and choose(true,false,.value=true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn open_default_inference_never_invokes_overridden_suppliers_or_self_seeds() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        comptime fn looping()->T where T {looping<T>()}
        fn choose(comptime flag:bool,comptime .value:T=looping<T>())->defer Select(flag,T)
            where T {value}";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{choose(true,.value=true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let failure = compile_with_protocols(
        &format!("{prefix} fn bad()->bool {{choose(true)}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("inference evidence"), "{failure}");
}

#[test]
fn open_inference_substitutes_prepared_type_inputs_into_callable_domains() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn number(value:bool)->Byte {if value {9} else {7}}
        fn choose(comptime U:Type,comptime flag:bool,callback:fn(U)->T,value:U)
            ->defer Select(flag,T) where T {callback(value)}
        fn run()->Byte {choose(bool,true,number,true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(9)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn suspended_type_blocks_invoke_staged_local_closures() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        fn choose(comptime flag:bool,comptime seed:defer Domain(flag),
            value:defer do {let callback=|| flag;if callback() {seed} else {bool}})
            ->defer do {let callback=|| flag;if callback() {seed} else {bool}} {value}
        fn flag()->bool {choose(true,bool,true)}
        fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}";
    for setup in [
        "let callback=|| flag",
        "var local=flag;let callback=|| local;local=false",
        "let make=|| do {let local=flag;|| local};let callback=make()",
    ] {
        let variant = source.replace("let callback=|| flag", setup);
        let package = compile(&variant);
        for (entry, expected) in [
            ("flag", Value::Bool(true)),
            ("byte", Value::UnsignedInteger(9)),
        ] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 100_000).unwrap(),
                expected
            );
            luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
        }
    }
    assert!(
        compile_with_protocols(
            &source.replace("comptime flag:bool", "flag:bool"),
            &Default::default()
        )
        .is_err()
    );
    let pointer = source.replace(
        "let callback=|| flag",
        "var local=false;let ptr=local.ref;let callback=|| do {ptr.* = flag;ptr.*}",
    );
    let package = compile(&pointer);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn suspended_type_loops_discover_inputs_and_resume_local_updates() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        fn choose(comptime flag:bool,comptime seed:defer Domain(flag),
            value:defer do {var saved=bool;while flag {saved=seed;break;} saved})
            ->defer do {var saved=bool;while flag {saved=seed;break;} saved} {value}
        fn flag()->bool {choose(true,bool,true)}
        fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}";
    for body in [
        "var saved=bool;while flag {saved=seed;break;} saved",
        "var saved=bool;var running=flag;while running {saved=seed;running=false;continue;} saved",
        "var saved=bool;while:outer flag {while true {saved=seed;break outer;}} saved",
    ] {
        let variant = source.replace("var saved=bool;while flag {saved=seed;break;} saved", body);
        let package = compile(&variant);
        for (entry, expected) in [
            ("flag", Value::Bool(true)),
            ("byte", Value::UnsignedInteger(9)),
        ] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 100_000).unwrap(),
                expected
            );
            luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
        }
    }
    assert!(
        compile_with_protocols(
            &format!("{source}fn bad()->bool {{choose(false,true,true)}}"),
            &Default::default()
        )
        .is_err()
    );
    for invalid in [
        source.replace("var saved", "let saved"),
        source.replace("saved=seed", "saved=true"),
        source.replace("comptime flag:bool", "flag:bool"),
    ] {
        assert!(
            compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn suspended_comptime_locals_check_mutability_and_concrete_assignment_domains() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        comptime fn identity(value:T)->T where T {value}
        fn choose(comptime flag:bool,comptime seed:defer Domain(flag),
            value:defer do {var saved=seed;saved=identity(seed);saved})
            ->defer do {var saved=seed;saved=identity(seed);saved} {value}
        fn flag()->bool {choose(true,bool,true)}
        fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}";
    let package = compile(source);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
    }
    let replaced = source
        .replace("saved=identity(seed)", "saved=bool")
        .replace(
            "fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}",
            "fn byte()->bool {choose(true,.u8'builtin,true)}",
        );
    let package = compile(&replaced);
    assert_eq!(
        luna::execution::interpret(&package, "byte", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "byte", 100_000).unwrap();
    for invalid in [
        source.replace("var saved", "let saved"),
        source.replace("saved=identity(seed)", "saved=true"),
        source.replace("comptime seed:", "seed:"),
    ] {
        assert!(
            compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn suspended_matches_preserve_staged_pattern_binding_provenance() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        fn choose(comptime flag:bool,comptime seed:defer Domain(flag),
            value:defer flag match {condition=>if not condition {bool} else {seed}})
            ->defer flag match {condition=>if not condition {bool} else {seed}} {value}
        fn flag()->bool {choose(true,bool,true)}
        fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}";
    for matched in [
        "flag match {condition=>if not condition {bool} else {seed}}",
        "(flag,false) match {(condition,other)=>if not condition or other {bool} else {seed}}",
        "flag match {condition if condition=>if not condition {bool} else {seed},other=>if not other {bool} else {seed}}",
        "flag match {outer=>outer match {inner=>if not inner {bool} else {seed}}}",
    ] {
        let variant = source.replace(
            "flag match {condition=>if not condition {bool} else {seed}}",
            matched,
        );
        let package = compile(&variant);
        for (entry, expected) in [
            ("flag", Value::Bool(true)),
            ("byte", Value::UnsignedInteger(9)),
        ] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 100_000).unwrap(),
                expected
            );
            luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
        }
    }
    for invalid in [
        source.replace("comptime flag:bool", "flag:bool"),
        source.replace("flag match", "seed match"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
    let runtime_guard = source
        .split("fn flag()->")
        .next()
        .unwrap()
        .replace("comptime flag:bool,", "comptime flag:bool,runtime:bool,")
        .replace(
            "flag match {condition=>if not condition {bool} else {seed}}",
            "flag match {condition if runtime=>seed,_=>bool}",
        );
    let failure = compile_with_protocols(&runtime_guard, &Default::default()).unwrap_err();
    assert!(failure.contains("runtime parameter"), "{failure}");
}

#[test]
fn suspended_type_blocks_check_staged_local_conditions() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        fn choose(comptime flag:bool,comptime seed:defer Domain(flag),
            value:defer do {let condition=flag;if not condition {bool} else {seed}})
            ->defer do {let condition=flag;if not condition {bool} else {seed}} {value}
        fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}
        fn flag()->bool {choose(true,bool,true)}";
    for binding in [
        "let condition=flag",
        "let outer=not flag;let condition=not outer",
        "let condition=do {let inner=not flag;not inner}",
        "let (condition,unused)=(flag,false)",
    ] {
        let variant = source.replace("let condition=flag", binding);
        let package = compile(&variant);
        for (entry, expected) in [
            ("flag", Value::Bool(true)),
            ("byte", Value::UnsignedInteger(9)),
        ] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 100_000).unwrap(),
                expected
            );
            luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
        }
    }
    for invalid in [
        source.replace("comptime flag:bool", "flag:bool"),
        source.replace("let condition=flag", "let condition=seed"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn suspended_generic_defaults_follow_unknown_operand_dependencies() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        comptime fn Identity(T:Type)->Type {T}
        comptime fn identity(value:T)->T where T, .U:Type=Identity(T), .V:Type=Identity(U) {value}
        fn choose(comptime flag:bool,comptime seed:defer Domain(flag),
            value:defer do {let saved=identity(seed);identity(saved)})
            ->defer do {let saved=identity(seed);identity(saved)} {value}
        fn flag()->bool {choose(true,bool,true)}
        fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}";
    let package = compile(source);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
    }
    let phantom = source.replace("where T, .U", "where T, Missing, .U");
    assert!(compile_with_protocols(&phantom, &Default::default()).is_err());
    let runtime = "typealias Type=.Type'builtin;
        comptime fn Identity(T:Type)->Type {T}
        fn identity(value:T)->T where T, .U:Type=Identity(T), .V:Type=Identity(U) {value}
        fn forward(comptime T:Type,value:defer Identity(T))->defer Identity(T) {
            let saved=identity(value);identity(saved)
        }
        fn flag()->bool {forward(bool,true)}
        fn byte()->.u8'builtin {forward(.u8'builtin,9)}";
    let package = compile(runtime);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let nonterminating = format!(
        "{} comptime fn Loop(T:Type)->Type {{Loop(T)}}",
        runtime.replace(".V:Type=Identity(U)", ".V:Type=Loop(U)")
    );
    assert!(compile_with_protocols(&nonterminating, &Default::default()).is_err());
}

#[test]
fn suspended_generic_locals_resume_with_their_concrete_domains() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        comptime fn identity(value:T)->T where T {value}
        fn choose(comptime flag:bool,comptime seed:defer Domain(flag),
            value:defer do {let saved=identity(seed);identity(saved)})
            ->defer do {let saved=identity(seed);identity(saved)} {value}
        fn flag()->bool {choose(true,bool,true)}
        fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}";
    let package = compile(source);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for invalid in [
        format!("{source}fn bad()->bool {{choose(false,true,true)}}"),
        source.replace("identity(saved)", "saved.missing"),
        source.replace("comptime seed:", "seed:"),
        source.replace("where T {value}", "where T, U {value}"),
        source.replace("where T {value}", "where T, requires T == Type {value}"),
        source.replace("comptime fn identity", "fn identity"),
    ] {
        assert!(
            compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn generic_forwarding_preserves_affine_operand_uses() {
    use elaboration::verification::moves::{Delivery, analyze};
    for (body, valid) in [
        ("let saved=identity(value);identity(saved)", true),
        ("identity(value);identity(value)", false),
        ("let saved=identity(value);identity(saved);saved", false),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;struct Ticket {{}}
            comptime fn Select(T:Type)->Type {{T}}
            fn identity(value:T)->T where T {{value}}
            fn forward(comptime T:Type,value:defer Select(T))->defer Select(T) {{{body}}}
            fn run()->Ticket {{forward(Ticket,Ticket {{}})}}"
        );
        let package = compile(&source);
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let function = &graph
            .functions
            .iter()
            .find(|instance| instance.function.name == "forward")
            .unwrap()
            .function;
        let report = analyze(
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100_000,
        );
        assert_eq!(report.is_ok(), valid, "{body}: {report:?}");
    }
}

#[test]
fn generic_aggregate_forwarding_defers_pointer_layout_until_instantiation() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Ticket {}impl Retire for Ticket {fn drop(*self) {}}
        struct Box where T {value:T}
        fn identity(value:T)->T where T {value}
        pure fn forward(value:Box<T>)->Box<T> where T {identity(value)}
        pure fn closed(value:Box<T>)->Box<T> where T {
            let action=|item:Box<T>| item;action(value)
        }";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let held=closed(forward(Box<Ticket> {{value:Ticket {{}}}}));true}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000)
            .unwrap(),
        Value::Bool(true)
    );
    let pointer = compile(&format!(
        "{prefix} fn run()->bool {{let cell=true;let held=closed(forward(Box<*bool> {{value:cell.ref}}));held.value.*}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&pointer, "run", "Duplicate", "Retire", 200_000)
            .unwrap(),
        Value::Bool(true)
    );
    let expired = compile(&format!(
        "{prefix} struct Cell {{flag:bool}}
        impl Retire for Cell {{fn drop(*self) {{}}}}
        fn run()->bool {{
            let cell=Cell {{flag:true}};
            let held=closed(forward(Box<*bool> {{value:cell.flag.ref}}));
            let moved=cell;held.value.*
        }}"
    ));
    let error =
        luna::execution::interpret_with_protocols(&expired, "run", "Duplicate", "Retire", 200_000)
            .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot prepare resource-checked execution"),
        "{error}"
    );
    let duplicate = prefix.replace("{identity(value)}", "{identity(value);identity(value)}");
    assert!(compile_with_protocols(&duplicate, &Default::default()).is_err());
}

#[test]
fn ownership_only_closure_analysis_does_not_require_nominal_layouts() {
    use elaboration::verification::moves::{Delivery, analyze};
    for (body, valid) in [
        ("let saved=action(value);saved", true),
        ("let saved=action(value);action(saved)", true),
        ("action(value);action(value)", false),
    ] {
        let package = compile(&format!(
            "struct Ticket {{}} fn forward(value:Ticket)->Ticket {{let action=|item:Ticket| item;{body}}}"
        ));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "forward")
            .unwrap();
        let report = analyze(
            function,
            |ty| {
                Ok(
                    if matches!(ty, nir::Term::Primitive(_) | nir::Term::Function { .. }) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    },
                )
            },
            100_000,
        );
        assert_eq!(report.is_ok(), valid, "{body}: {report:?}");
    }
}

#[test]
fn generic_calls_from_unknown_domains_preserve_contracts_and_memory_writes() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Byte}}
        fn put(value:T,p:*Byte,n:Byte)->result:Byte where T,
            requires n < 255, ensures p.* == n'old and result == p.* and result < 255 {
            p.* = n;p.*
        }
        fn relay(comptime flag:bool,value:defer Pick(flag),p:*Byte,n:Byte)->result:Byte
            where requires n < 255, ensures p.* == n'old and result == p.* and result < 255 {
            let result=put(value,p,n);result
        }
        fn first()->bool {true}
        fn second()->result:Byte where ensures result == 7 {7}
        fn run()->Byte {let cell:Byte=0;relay(true,first(),cell.ref,second());cell + 1}
        fn byte()->Byte {let cell:Byte=0;relay(false,9,cell.ref,7) + 1}";
    let package = compile(source);
    for entry in ["run", "byte"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::UnsignedInteger(8)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    vm.invoke(id(&package, "run"), vec![]).unwrap();
    assert_eq!(
        calls(&package, &vm),
        ["run", "first", "second", "relay", "put"]
    );
    for invalid in [
        source.replace("cell.ref,second()", "cell.ref,255"),
        source.replace("p.* = n;p.*", "p.* = 255;p.*"),
        source.replace("let result=put(value,p,n)", "let result=put(value,p,255)"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn runtime_generic_forwarding_keeps_unknown_descriptions_until_instantiation() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn identity(value:T)->T where T {value}
        fn accepts(value:T)->bool where T {true}
        fn choose(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {
            let saved=identity(value);identity(saved)
        }
        fn checked(comptime flag:bool,value:defer Pick(flag))->bool {
            let accepted=accepts(value);if accepted {true} else {false}
        }
        fn flag()->bool {choose(true,true)}
        fn checked_byte()->bool {checked(false,9)}
        fn byte()->.u8'builtin {choose(false,9)}";
    let package = compile(source);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
        ("checked_byte", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for body in [
        "let saved=identity(value);saved.missing",
        "let saved=identity(value);not saved",
    ] {
        let invalid = source.replace("let saved=identity(value);identity(saved)", body);
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
    let ambiguous = source.replace(
        "fn identity(value:T)->T where T",
        "fn identity(value:T)->T where T, U",
    );
    assert!(compile_with_protocols(&ambiguous, &Default::default()).is_err());
}

#[test]
fn runtime_generic_calls_describe_deferred_operands_from_result_context() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn identity(value:T)->T where T {value}
        fn choose(comptime flag:bool,value:defer Pick(flag))->bool {identity(identity(value))}
        fn run()->bool {choose(true,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        compile_with_protocols(
            &format!("{source}fn bad()->bool {{choose(false,9)}}"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn suspended_generic_value_defaults_keep_declaration_bindings() {
    let source="typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        comptime fn keep(value:T,.other:T=value)->T where T {other}
        fn choose(comptime flag:bool,comptime T:defer Domain(flag),value:defer keep(T))->defer keep(T) {value}
        fn explicit(comptime flag:bool,comptime T:defer Domain(flag),value:defer keep(T,.other=bool))->bool {value}
        fn flag()->bool {choose(true,bool,true)}
        fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}
        fn overridden()->bool {explicit(true,.u8'builtin,true)}";
    let package = compile(source);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
        ("overridden", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
    }
    for invalid in [
        "fn bad()->bool {choose(false,true,true)}",
        "fn bad()->bool {explicit(true,bool,9)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source}{invalid}"), &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn suspended_value_default_override_skips_recursive_supplier() {
    let source="typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        comptime fn looping()->T where T {looping<T>()}
        comptime fn keep(value:T,.other:T=looping<T>())->T where T {other}
        fn choose(comptime flag:bool,comptime T:defer Domain(flag),value:defer keep(T,.other=T))->defer keep(T,.other=T) {value}
        fn run()->bool {choose(true,bool,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let omitted = source.replace("keep(T,.other=T)", "keep(T)");
    assert!(compile_with_protocols(&omitted, &Default::default()).is_err());
}

#[test]
fn suspended_generic_type_defaults_settle_with_inference() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        pure comptime fn Identity(value:Type)->Type {value}";
    for declaration in [
        "comptime fn keep(value:T)->T where .T:Type=Type {value}",
        "comptime fn keep(value:U)->T where T,.U:Type=Identity(T),requires T == U {value}",
    ] {
        let source=format!("{prefix}{declaration}
            fn choose(comptime flag:bool,comptime T:defer Domain(flag),value:defer keep(T))->defer keep(T) {{value}}
            fn flag()->bool {{choose(true,bool,true)}}
            fn byte()->.u8'builtin {{choose(true,.u8'builtin,9)}}");
        let package = compile(&source);
        for (entry, expected) in [
            ("flag", Value::Bool(true)),
            ("byte", Value::UnsignedInteger(9)),
        ] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 100_000).unwrap(),
                expected
            );
        }
        assert!(
            compile_with_protocols(
                &format!("{source} fn bad()->bool {{choose(false,true,true)}}"),
                &Default::default()
            )
            .is_err()
        );
    }
}

#[test]
fn suspended_explicit_type_arguments_suppress_default_computation() {
    let source="typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        comptime fn Loop()->Type {Loop()}
        comptime fn keep(value:T)->T where .T:Type=Loop() {value}
        fn choose(comptime flag:bool,comptime T:defer Domain(flag),value:defer keep<.T=Type>(T))->defer keep<.T=Type>(T) {value}
        fn run()->bool {choose(true,bool,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let wrong = source.replace("keep<.T=Type>", "keep<.T=bool>");
    assert!(compile_with_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn suspended_generic_calls_infer_from_the_required_result() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        comptime fn identity(value:T)->T where T {value}
        fn choose(comptime flag:bool,comptime T:defer Domain(flag),value:defer identity(identity(T)))->defer identity(T) {value}
        fn flag()->bool {choose(true,bool,true)}
        fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}";
    let package = compile(source);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
    }
    for invalid in [
        "fn bad()->bool {choose(false,true,true)}",
        "fn bad()->bool {choose(true,bool,9)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source}{invalid}"), &Default::default()).is_err(),
            "{invalid}"
        );
    }
    let constrained = source.replace("where T {value}", "where T, requires T == Type {value}");
    compile(&constrained);
    let wrong = source.replace("where T {value}", "where T, requires T == bool {value}");
    assert!(compile_with_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn suspended_generic_calls_use_known_sibling_arguments() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        comptime fn identity(value:T)->T where T {value}
        comptime fn Select(first:T,second:T)->Type where T {bool}
        fn choose(comptime flag:bool,comptime seed:defer Pick(flag),value:defer Select(true,identity(seed)))->bool {value}
        fn run()->bool {choose(true,false,true)}";
    for call in ["Select(true,identity(seed))", "Select(identity(seed),true)"] {
        let source = source.replace("Select(true,identity(seed))", call);
        let package = compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let invalid = format!("{source} fn bad()->bool {{choose(false,9,true)}}");
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
    let dependent=source.replace("comptime fn Select(first:T,second:T)->Type where T {bool}",
        "pure comptime fn Identity(value:Type)->Type {value} comptime fn Select(first:T,second:U)->Type where T,.U:Type=Identity(T) {bool}");
    let package = compile(&dependent);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn suspended_value_dependencies_form_distinct_call_instances() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn choose(comptime flag:bool,comptime seed:defer Pick(flag),.value:defer Pick(seed)=true)->defer Pick(seed) {value}
        fn flag()->bool {choose(true,true)}
        fn byte()->.u8'builtin {choose(true,false,.value=9)}";
    let package = compile(source);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
        ("flag", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
    }
    for invalid in [
        "fn bad()->bool {choose(false,9)}",
        "fn bad()->.u8'builtin {choose(true,false)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source}{invalid}"), &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn suspended_annotation_calls_resume_after_staged_domain_formation() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        pure comptime fn Identity(value:Type)->Type {value}
        fn choose(comptime flag:bool,comptime T:defer Domain(flag),value:defer Identity(T))->defer Identity(T) {value}
        fn flag()->bool {choose(true,bool,true)}
        fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}";
    for expression in [
        "Identity(T)",
        "Identity(Identity(T))",
        "if flag {Identity(T)} else {bool}",
        "do {Identity(T)}",
        "do {let saved=T;Identity(saved)}",
        "when {flag=>Identity(T),else=>Identity(T)}",
        "flag match {true=>Identity(T),false=>Identity(T)}",
        "(flag,true) match {(yes,other) if yes and other=>Identity(T),_=>Identity(T)}",
    ] {
        let source = source.replace("defer Identity(T)", &format!("defer {expression}"));
        let package = compile(&source);
        for (entry, expected) in [
            ("flag", Value::Bool(true)),
            ("byte", Value::UnsignedInteger(9)),
        ] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 100_000).unwrap(),
                expected
            );
        }
        for invalid in [
            "fn bad()->bool {choose(false,true,true)}",
            "fn bad()->bool {choose(true,bool,9)}",
        ] {
            // The branch expression deliberately does not consume T when flag is false.
            if expression.starts_with("if ") && invalid.contains("choose(false") {
                continue;
            }
            assert!(
                compile_with_protocols(&format!("{source}{invalid}"), &Default::default()).is_err(),
                "{invalid}"
            );
        }
    }
    for expression in ["Identity(T.missing)", "Identity(if runtime {T} else {T})"] {
        let invalid = format!(
            "{source} fn unused(comptime flag:bool,comptime T:defer Domain(flag),runtime:bool,value:defer {expression})->bool {{true}}"
        );
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
    for expression in [
        "flag match {true=>Identity(T)}",
        "when {else=>Identity(T),flag=>Identity(T)}",
        "T match {true=>bool,false=>bool}",
    ] {
        let invalid = source.replace("defer Identity(T)", &format!("defer {expression}"));
        assert!(
            compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{expression}"
        );
    }
}

#[test]
fn deferred_type_inputs_feed_later_domains_without_placeholder_types() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        fn choose(comptime flag:bool,comptime T:defer Domain(flag),value:defer T)->defer T {value}
        fn flag()->bool {choose(true,bool,true)}
        fn byte()->.u8'builtin {choose(true,.u8'builtin,9)}";
    let package = compile(source);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
    }
    let non_type = compile_with_protocols(
        &format!("{source}fn bad()->bool {{choose(false,true,true)}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        non_type.contains("deferred type input did not produce a Type"),
        "{non_type}"
    );
    for invalid in [
        "fn bad()->bool {choose(false,true,true)}",
        "fn bad()->bool {choose(true,bool,9)}",
        "fn bad(flag:bool)->bool {choose(flag,bool,true)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source}{invalid}"), &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn deferred_type_input_chains_preserve_defaults_and_runtime_order() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        fn first()->bool {false}
        fn second()->bool {true}
        fn chain(comptime flag:bool,comptime T:defer Domain(flag),comptime U:defer T,left:defer U,right:defer U)->defer U {right}
        fn defaults(comptime flag:bool,comptime .T:defer Domain(flag)=Type,comptime .U:defer T=bool,.value:defer U=true)->defer U {value}
        fn run()->bool {chain(true,Type,bool,first(),second())}
        fn omitted()->bool {defaults(true)}
        fn explicit()->.u8'builtin {defaults(true,.U=.u8'builtin,.value=9)}";
    let package = compile(source);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let trace = calls(&package, &vm);
    assert_eq!(
        trace
            .iter()
            .filter(|name| ["first", "second", "chain"].contains(&name.as_str()))
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["first", "second", "chain"]
    );
    for (entry, expected) in [
        ("omitted", Value::Bool(true)),
        ("explicit", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
    }
    for invalid in [
        "fn bad()->bool {chain(true,bool,true,false,true)}",
        "fn bad()->.u8'builtin {defaults(true,.U=.u8'builtin)}",
        "fn bad(comptime flag:bool,T:defer Domain(flag),value:defer T)->bool {true}",
        "fn bad(comptime T:defer U,comptime U:Type)->bool {true}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source}{invalid}"), &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn deferred_comptime_domains_are_formed_before_argument_checking() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn choose(comptime flag:bool,comptime value:defer Pick(flag))->defer Pick(flag) {value}
        fn relay(comptime flag:bool,comptime value:defer Pick(flag))->defer Pick(flag) {choose(flag,value)}
        fn identity(value:T)->T where T {value}
        fn after(comptime flag:bool,comptime seed:defer Pick(flag),comptime T:Type,value:T)->T {identity(value)}
        fn flag()->bool {choose(true,true)}
        fn late()->bool {after(false,9,bool,true)}
        fn byte()->.u8'builtin {relay(false,9)}";
    let package = compile(source);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
        ("late", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
    }
    for invalid in [
        "fn bad()->bool {choose(true,9)}",
        "fn bad(value:bool)->bool {choose(true,value)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source}{invalid}"), &Default::default()).is_err()
        );
    }
}

#[test]
fn deferred_comptime_defaults_and_type_domains_are_instance_specific() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        fn choose(comptime flag:bool,comptime .value:defer Pick(flag)=true)->defer Pick(flag) {value}
        fn inspect(comptime flag:bool,comptime value:defer Domain(flag))->bool {true}
        fn flag()->bool {choose(true)}
        fn byte()->.u8'builtin {choose(false,.value=9)}
        fn type_input()->bool {inspect(true,bool)}
        fn value_input()->bool {inspect(false,false)}";
    let package = compile(source);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
        ("type_input", Value::Bool(true)),
        ("value_input", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
    }
    for invalid in [
        "fn bad()->.u8'builtin {choose(false)}",
        "fn bad()->bool {inspect(true,true)}",
        "fn bad()->bool {inspect(false,bool)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source}{invalid}"), &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn late_type_inputs_survive_normal_form_defaults() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn identity(value:T)->T where T {value}
        pure comptime fn Identity(value:Type)->Type {value}
        pure comptime fn Nested<T,.Fallback:Type=T>->Type {T}
        pure comptime fn Tagged<T,.Other:Type=T>->Type {T}
        fn inspect(comptime tag:Type,comptime expected:Type)->bool where requires tag == expected {true}
        fn choose(comptime flag:bool,unused:defer Pick(flag),comptime T:Type,value:T)->T {
            inspect(Tagged<T>,Tagged<T,.Other=DEFAULT>);
            identity(value)
        }
        fn run()->bool {choose(false,7,bool,true)}
        fn byte()->.u8'builtin {choose(true,false,.u8'builtin,9)}";
    for default in ["T", "Identity(T)", "Nested<T>"] {
        let source = source
            .replace(".Other:Type=T", &format!(".Other:Type={default}"))
            .replace("DEFAULT", default);
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("default {default}: {error}"));
        for (entry, expected) in [
            ("run", Value::Bool(true)),
            ("byte", Value::UnsignedInteger(9)),
        ] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 100_000).unwrap(),
                expected
            );
        }
        let wrong_head = source.replace(
            "Tagged<T>,Tagged<T,.Other=",
            "Tagged<T>,Nested<T,.Fallback=",
        );
        assert!(
            compile_with_protocols(&wrong_head, &Default::default()).is_err(),
            "default {default}: distinct normal-form heads must remain distinct"
        );
    }
}

#[test]
fn open_deferred_functions_check_any_definitions_and_each_concrete_delivery() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}";
    let source = format!("{prefix}
        fn identity(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {{let saved=value;saved}}
        fn returned(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {{return value;}}
        fn select(comptime flag:bool,left:defer Pick(flag),right:defer Pick(flag),choose:bool)->defer Pick(flag) {{
            if choose {{left}} else {{right}}
        }}
        fn byte()->.u8'builtin {{identity(false,7)}}
        fn flag()->bool {{returned(true,true)}}
        fn selected()->.u8'builtin {{select(false,3,9,false)}}");
    let package = compile(&source);
    for (entry, expected) in [
        ("byte", Value::UnsignedInteger(7)),
        ("flag", Value::Bool(true)),
        ("selected", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for declaration in [
        // Definition checking applies even when nobody calls the function.
        "fn bad(comptime flag:bool,value:defer Pick(flag))->bool {value.member}",
        "fn bad(comptime flag:bool,value:defer Pick(flag))->bool {value + value}",
        "fn bad(comptime flag:bool,value:defer Pick(flag))->bool {if value {true} else {false}}",
        "fn bad(comptime flag:bool,value:defer Pick(flag))->bool {9}",
    ] {
        let failure =
            compile_with_protocols(&format!("{prefix}{declaration}"), &Default::default())
                .unwrap_err();
        assert!(
            !failure.contains("not yet implemented"),
            "definition must be rejected for missing evidence or incompatibility: {failure}"
        );
    }
    for use_site in [
        "fn run()->bool {identity(false,true)}",
        "fn run()->bool {identity(true,7)}",
        "fn run()->bool {identity(false,7)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source}{use_site}"), &Default::default()).is_err(),
            "{use_site}"
        );
    }
    // Equal spelling of two deferred computations does not prove an equality
    // between unrelated actual Types; concrete result delivery is rechecked.
    let bad_result = format!(
        "{prefix}
        fn changed(comptime flag:bool,value:defer Pick(flag))->defer Pick(not flag) {{value}}
        fn run()->bool {{changed(false,7)}}"
    );
    assert!(compile_with_protocols(&bad_result, &Default::default()).is_err());
}

#[test]
fn open_deferred_instances_preserve_opaque_types_and_reject_failed_computations() {
    let prefix = "typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Identity<bool>}}
        fn pass(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}";
    let package = compile(&format!("{prefix} fn run()->bool {{pass(true,true)}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        compile_with_protocols(
            &format!("{prefix} fn bad()->bool {{pass(false,true)}}"),
            &Default::default()
        )
        .is_err()
    );
    let divergent = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Pick(flag)}}
        fn pass(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn run()->bool {pass(false,true)}";
    let error = compile_with_protocols(divergent, &Default::default()).unwrap_err();
    assert!(
        error.contains("comptime") && !error.contains("not yet implemented"),
        "{error}"
    );
}

#[test]
fn open_deferred_functions_forward_any_without_granting_operation_evidence() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn accept(value:bool)->bool {value}
        fn leaf(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn wrapper(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {leaf(flag,value)}
        fn narrow(comptime flag:bool,value:defer Pick(flag))->bool {accept(value)}
        fn run()->bool {wrapper(true,true) and narrow(true,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert!(
        graph
            .functions
            .iter()
            .any(|instance| instance.function.name == "leaf")
    );
    assert!(
        compile_with_protocols(
            &format!("{source} fn bad()->bool {{narrow(false,1)}}"),
            &Default::default()
        )
        .is_err()
    );
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}";
    for bad in [
        "comptime fn ct()->bool {true} fn bad(comptime flag:bool,value:defer Pick(flag))->bool {ct()}",
        "fn bad(comptime flag:bool,value:defer Pick(flag))->Type {bool}",
        "fn bad(comptime flag:bool,value:defer Pick(flag),illegal:Type)->defer Pick(flag) {value}",
        "fn other(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
            fn bad(comptime flag:bool,value:defer Pick(flag),runtime:bool)->defer Pick(flag) {other(runtime,value)}",
        "fn other(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
            fn bad(comptime flag:bool,value:defer Pick(flag),runtime:bool)->defer Pick(flag) {other(flag and runtime,value)}",
    ] {
        assert!(compile_with_protocols(&format!("{prefix}{bad}"),&Default::default()).is_err(),"unused definition must still reject: {bad}");
    }
}

#[test]
fn open_deferred_annotations_observe_concrete_type_inputs_and_keep_runtime_order() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(T:Type,flag:bool)->Type {if flag and T == bool {bool} else {.u8'builtin}}
        fn choose(comptime T:Type,comptime flag:bool,left:defer Pick(T,flag),right:defer Pick(T,flag))->defer Pick(T,flag) {left}
        fn first()->bool {true}
        fn second()->bool {false}
        fn run()->bool {choose(bool,true,first(),second())}
        fn byte()->.u8'builtin {choose(.u8'builtin,true,7,9)}";
    let package = compile(source);
    let mut interpreter = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &interpreter),
        ["run", "first", "second", "choose"]
    );
    assert_eq!(
        luna::execution::interpret(&package, "byte", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
    for entry in ["run", "byte"] {
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn open_deferred_recursion_reuses_instances_and_literals_use_concrete_context() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn recur(comptime flag:bool,value:defer Pick(flag),n:.u8'builtin)->defer Pick(flag) {
            if n == 0 {value} else {recur(flag,value,n - 1)}
        }
        fn make(comptime flag:bool)->defer Pick(flag) {7}
        fn run()->.u8'builtin {recur(false,make(false),3)}";
    let package = compile(source);
    assert_eq!(
        package
            .functions
            .iter()
            .filter(|function| function.name == "recur")
            .count(),
        1
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert!(
        compile_with_protocols(
            &format!("{source} fn bad()->bool {{make(true)}}"),
            &Default::default()
        )
        .is_err()
    );
    let second = compile(source);
    assert_eq!(
        package.to_string(),
        second.to_string(),
        "instance identities and dumps must be deterministic"
    );
}

#[test]
fn open_deferred_defaults_prepare_staged_chains_and_only_selected_suppliers() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        comptime fn invert(flag:bool)->bool {not flag}
        fn choose(comptime .seed:bool=false,comptime .flag:bool=invert(seed),.value:defer Pick(flag)=true)->defer Pick(flag) {value}
        fn flag()->bool {choose()}
        fn byte()->.u8'builtin {choose(.value=7,.seed=true)}
        fn override_flag()->bool {choose(.seed=true,.flag=true,.value=false)}";
    let package = compile(source);
    for (entry, result) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(7)),
        ("override_flag", Value::Bool(false)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            result
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    assert!(
        compile_with_protocols(
            &format!("{source} fn bad()->.u8'builtin {{choose(.seed=true)}}"),
            &Default::default()
        )
        .is_err()
    );
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        comptime fn forever()->bool {forever()}
        fn choose(comptime .flag:bool=defer forever(),.value:defer Pick(flag)=true)->defer Pick(flag) {value}
        fn run()->bool {choose(.flag=true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let error = compile_with_protocols(
        &format!("{source} fn bad()->bool {{choose()}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("comptime") && !error.contains("not yet implemented"),
        "{error}"
    );
}

#[test]
fn open_deferred_runtime_defaults_observe_prepared_parameters_in_order() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn first()->bool {true}
        fn second()->bool {false}
        fn choose(comptime .flag:bool=true,input:bool,.next:bool=input,.value:defer Pick(flag)=next)->defer Pick(flag) {value}
        fn run()->bool {choose(first(),.next=second())}
        fn omitted()->bool {choose(first())}
        fn scope()->bool {let next=true;choose(false)}
        fn previous(comptime .flag:bool=true,limit:.u8'builtin,.prior:.u8'builtin=if limit > 0 {limit - 1} else {0},.value:defer Pick(flag)=true)->.u8'builtin {prior}
        fn zero()->.u8'builtin {previous(0)}
        fn six()->.u8'builtin {previous(7)}";
    let package = compile(source);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(calls(&package, &vm), ["run", "first", "second", "choose"]);
    for (entry, result) in [
        ("omitted", Value::Bool(true)),
        ("scope", Value::Bool(false)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            result
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for (entry, result) in [
        ("zero", Value::UnsignedInteger(0)),
        ("six", Value::UnsignedInteger(6)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            result
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn open_deferred_defaults_check_unused_declarations_and_never_capture_future_inputs() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}";
    let package=compile(&format!("{prefix}
        fn finite()->bool {{true}}
        fn valid(comptime .flag:bool=true,.value:defer Pick(flag)=finite())->defer Pick(flag) {{value}}
        fn run()->bool {{valid()}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for bad in [
        "fn diverges()->bool {diverges()} fn bad(comptime .flag:bool=true,.value:defer Pick(flag)=diverges())->defer Pick(flag) {value}",
        "fn bad(comptime .flag:bool=true,.value:defer Pick(flag)=later,.later:bool=true)->defer Pick(flag) {value}",
        "fn bad(runtime:bool,comptime .flag:bool=runtime,.value:defer Pick(flag)=true)->defer Pick(flag) {value}",
        "fn bad(comptime .flag:bool=true,.value:defer Pick(flag)=do {return true;})->defer Pick(flag) {value}",
        "fn bad(comptime .flag:bool=9,.value:defer Pick(flag)=true)->defer Pick(flag) {value}",
        "fn bad(comptime .flag:bool=true,value:defer Pick(flag),.other:defer Pick(flag)=value)->defer Pick(flag) {other}",
        "fn bad(comptime .flag:bool=true,.count:.u8'builtin=255 + 1,.value:defer Pick(flag)=true)->defer Pick(flag) {value}",
    ] {
        assert!(
            compile_with_protocols(&format!("{prefix}{bad}"), &Default::default()).is_err(),
            "unused invalid default: {bad}"
        );
    }
}

#[test]
fn open_deferred_variadics_use_computed_tuple_identity_and_source_order() {
    let source="typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {List<T>}
        pure comptime fn Tuple<...Es:List<Type>>->Type {Tuple<...Es>}
        comptime fn Args(pair:bool)->Type {if pair {Tuple<bool,.u8'builtin>} else {Tuple<>}}
        fn collect(comptime pair:bool,...values:defer Args(pair),.marker:bool=false)->defer Args(pair) {values}
        fn first()->bool {true}
        fn second()->.u8'builtin {7}
        fn mark()->bool {false}
        fn run()->Tuple<bool,.u8'builtin> {collect(true,first(),.marker=mark(),second())}
        fn empty()->Tuple<> {collect(false)}
        fn named()->Tuple<bool,.u8'builtin> {collect(true,.values=(true,9))}
        fn packet()->Tuple<bool,.u8'builtin> {(first(),second())}
        fn spread()->Tuple<bool,.u8'builtin> {collect(true,...packet(),.marker=mark())}
        fn literal()->Tuple<bool,.u8'builtin> {collect(...(true,true,9))}";
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::List"),
        tuple: Some("input::Tuple"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Tuple(vec![Value::Bool(true), Value::UnsignedInteger(7)])
    );
    assert_eq!(
        calls(&package, &vm),
        ["run", "first", "mark", "second", "collect"]
    );
    for (entry, result) in [
        ("empty", Value::Tuple(vec![])),
        (
            "named",
            Value::Tuple(vec![Value::Bool(true), Value::UnsignedInteger(9)]),
        ),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            result
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let mut spread_vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        spread_vm.invoke(id(&package, "spread"), vec![]).unwrap(),
        Value::Tuple(vec![Value::Bool(true), Value::UnsignedInteger(7)])
    );
    assert_eq!(
        calls(&package, &spread_vm),
        ["spread", "packet", "first", "second", "mark", "collect"]
    );
    assert_eq!(
        luna::execution::interpret(&package, "literal", 100_000).unwrap(),
        Value::Tuple(vec![Value::Bool(true), Value::UnsignedInteger(9)])
    );
    for entry in ["spread", "literal"] {
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let collected = graph
        .functions
        .iter()
        .find(|instance| instance.function.name == "collect")
        .unwrap();
    let nir::Term::Function { parameters, result } =
        collected.function.signature.callable_signature()
    else {
        panic!("signature")
    };
    assert_eq!(parameters[0].ty, **result);
    assert!(
        matches!(result.as_ref(),nir::Term::NormalForm {head,..} if *head==id(&package,"Tuple"))
    );
    for use_site in [
        "fn bad()->Tuple<bool,.u8'builtin> {collect(true,true)}",
        "fn bad()->Tuple<> {collect(false,true)}",
        "fn bad()->Tuple<bool,.u8'builtin> {collect(true,true,7,.values=(true,9))}",
    ] {
        assert!(
            compile_with_source_bindings(
                &format!("{source}{use_site}"),
                &Default::default(),
                selectors
            )
            .is_err(),
            "{use_site}"
        );
    }
}

#[test]
fn open_deferred_variadics_combine_default_format_inputs_with_computed_packets() {
    let source="typealias Type=.Type'builtin;typealias Text=\"\"'type;
        pure comptime fn List<T>->Type {List<T>}
        pure comptime fn Tuple<...Es:List<Type>>->Type {Tuple<...Es>}
        comptime fn Args(format:Text)->Type {format match {\"pair\" => Tuple<bool,.u8'builtin>,\"none\" => Tuple<>,_=>Args(format)}}
        fn pack(comptime .format:Text=\"pair\",...values:defer Args(format))->defer Args(format) {values}
        fn pair()->Tuple<bool,.u8'builtin> {pack(true,7)}
        fn empty()->Tuple<> {pack(.format=\"none\")}";
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::List"),
        tuple: Some("input::Tuple"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    for (entry, result) in [
        (
            "pair",
            Value::Tuple(vec![Value::Bool(true), Value::UnsignedInteger(7)]),
        ),
        ("empty", Value::Tuple(vec![])),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            result
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for use_site in [
        "fn bad()->Tuple<> {pack(.format=\"none\",true)}",
        "fn bad()->Tuple<bool,.u8'builtin> {pack(\"wrong\",7)}",
        "fn bad()->Tuple<> {pack(.format=\"unknown\")}",
    ] {
        assert!(
            compile_with_source_bindings(
                &format!("{source}{use_site}"),
                &Default::default(),
                selectors
            )
            .is_err(),
            "{use_site}"
        );
    }
}

#[test]
fn open_deferred_program_contracts_survive_instance_generation_and_call_admission() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Byte}}
        fn step(comptime flag:bool,value:defer Pick(flag),n:Byte)->result:Byte
            where requires n < 255, ensures result == n + 1 {n + 1}
        fn relay(comptime flag:bool,value:defer Pick(flag),n:Byte)->defer Pick(flag)
            where requires n < 10, ensures n == n'old {value}
        fn run()->Byte {step(true,true,254)}
        fn other()->Byte {step(false,7,3)}
        fn forwarded()->bool {relay(true,true,5)}";
    let package = compile(source);
    for (entry, result) in [
        ("run", Value::UnsignedInteger(255)),
        ("other", Value::UnsignedInteger(4)),
        ("forwarded", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            result
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let steps = package
        .functions
        .iter()
        .filter(|function| function.name == "step")
        .collect::<Vec<_>>();
    assert_eq!(steps.len(), 2);
    assert!(
        steps
            .iter()
            .all(|function| function.preconditions.len() == 1
                && function.postconditions.len() == 1
                && function.result_binding.is_some())
    );
    let relay = package
        .functions
        .iter()
        .find(|function| function.name == "relay")
        .unwrap();
    assert_eq!(relay.entry_snapshots.len(), 1);
    for invalid in [
        format!("{source} fn bad()->Byte {{step(true,true,255)}}"),
        format!("{source} fn bad()->bool {{relay(true,true,10)}}"),
        source.replace("{n + 1}", "{n}"),
    ] {
        let error = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
        assert!(!error.contains("not yet implemented"), "{error}");
    }
}

#[test]
fn open_deferred_memory_contracts_export_writes_snapshots_and_results() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Byte}}
        fn put(comptime flag:bool,value:defer Pick(flag),p:*Byte,n:Byte)->result:Byte
            where requires n < 255, ensures p.* == n'old and result == p.* and p.* < 255 {
                p.* = n;p.*
            }
        fn run()->Byte {let cell:Byte=0;put(true,true,cell.ref,7);cell + 1}
        fn result()->Byte {let cell:Byte=0;put(false,9,cell.ref,7) + 1}";
    let package = compile(source);
    for entry in ["run", "result"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::UnsignedInteger(8)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    assert!(
        package
            .functions
            .iter()
            .filter(|function| function.name == "put")
            .all(|function| !function.entry_snapshots.is_empty()
                && !function.postconditions.is_empty())
    );
    let bad = source.replace("p.* = n;p.*", "p.* = 255;p.*");
    assert!(compile_with_protocols(&bad, &Default::default()).is_err());
}

#[test]
fn open_deferred_contracts_require_any_evidence_and_defined_entry_predicates() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Byte}}";
    for invalid in [
        "fn bad(comptime flag:bool,value:defer Pick(flag))->bool where requires value == true {true}",
        "fn bad(comptime flag:bool,value:defer Pick(flag))->result:defer Pick(flag) where ensures result == value {value}",
        "fn bad(comptime flag:bool,value:defer Pick(flag),n:Byte)->bool where requires n + 1 > 0 {true}",
        "fn noisy()->bool {true} fn bad(comptime flag:bool,value:defer Pick(flag))->bool where requires noisy() {true}",
    ] {
        assert!(
            compile_with_protocols(&format!("{prefix}{invalid}"), &Default::default()).is_err(),
            "unused invalid contract: {invalid}"
        );
    }
    let guarded = format!(
        "{prefix}
        fn good(comptime flag:bool,value:defer Pick(flag),n:Byte)->bool
            where requires n < 255, requires n + 1 > 0 {{true}}
        fn run()->bool {{good(true,true,7)}}"
    );
    let package = compile(&guarded);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn open_deferred_type_contracts_admit_before_running_type_computations() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {if T == bool {bool} else {Select(T)}}
        fn pass(comptime T:Type,value:defer Select(T))->defer Select(T)
            where requires T == bool {value}
        fn run()->bool {pass(bool,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let function = package
        .functions
        .iter()
        .find(|function| function.name == "pass")
        .unwrap();
    assert_eq!(function.requirements.len(), 1);
    for call in ["pass(Byte,7)", "pass(Identity<bool>,true)"] {
        let invalid =
            format!("{source} pure comptime fn Identity<T>->Type {{T}} fn bad()->bool {{{call}}}");
        let failure = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
        assert!(
            failure.contains("type requirement is not established"),
            "{failure}"
        );
    }
}

#[test]
fn open_deferred_type_contracts_require_definition_side_forwarding_evidence() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn pass(comptime T:Type,value:defer Select(T))->defer Select(T)
            where requires T == bool {value}
        fn relay(comptime T:Type,value:defer Select(T))->defer Select(T)
            where requires T == bool {pass(T,value)}
        fn run()->bool {relay(bool,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let invalid = source.replace(
        "where requires T == bool {pass(T,value)}",
        "{pass(T,value)}",
    );
    let failure = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("type requirement is not established"),
        "{failure}"
    );
}

#[test]
fn open_deferred_type_contracts_specialize_defaults_and_distinct_instances() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn pass(comptime .T:Type=bool,value:defer Select(T))->defer Select(T)
            where requires T == bool or T == Byte {value}
        fn first()->bool {pass(true)}
        fn second()->Byte {pass(.T=Byte,7)}";
    let package = compile(source);
    for (entry, value) in [
        ("first", Value::Bool(true)),
        ("second", Value::UnsignedInteger(7)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            value
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let instances = package
        .functions
        .iter()
        .filter(|function| function.name == "pass")
        .collect::<Vec<_>>();
    assert_eq!(instances.len(), 2);
    assert_ne!(instances[0].requirements, instances[1].requirements);
    for instance in instances {
        assert!(!instance.requirements.is_empty());
        for predicate in &instance.requirements {
            assert!(matches!(
                elaboration::ptc::Context::default().prove(
                    elaboration::ptc::Formula::from_predicate(predicate),
                    elaboration::ptc::Budget::default()
                ),
                elaboration::ptc::Outcome::Entailed(_)
            ));
        }
    }
}

#[test]
fn open_deferred_type_contracts_forward_dependent_defaults_without_running_overrides() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        comptime fn Loop()->Type {Loop()}
        fn pass(comptime .A:Type=Loop(),comptime .B:Type=A,value:defer Select(B))->defer Select(B)
            where requires B == bool {value}
        fn relay(comptime T:Type,value:defer Select(T))->defer Select(T)
            where requires T == bool {pass(.A=T,value)}
        fn run()->bool {relay(bool,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    // The caller's Type input sits at a different index from both default
    // dependencies in the callee; lexical indices must be substituted once.
    let shifted = source
        .replace(
            "fn relay(comptime T:Type,",
            "fn relay(comptime ignored:bool,comptime T:Type,",
        )
        .replace("relay(bool,true)", "relay(false,bool,true)");
    let shifted_package = compile(&shifted);
    assert_eq!(
        luna::execution::interpret(&shifted_package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&shifted_package, "run", 100_000).unwrap();
    let invalid = source.replace(
        "where requires T == bool {pass(.A=T,value)}",
        "{pass(.A=T,value)}",
    );
    let failure = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("type requirement is not established"),
        "{failure}"
    );
}

#[test]
fn open_deferred_calls_transport_known_domains_and_results_between_binders() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn identity(comptime T:Type,value:T)->T {value}
        fn open_identity(comptime T:Type,value:T,unused:defer Select(T))->T {value}
        fn relay(comptime ignored:bool,comptime U:Type,value:U,unknown:defer Select(U))->U {
            open_identity(U,identity(U,value),unknown)
        }
        fn run()->bool {relay(false,bool,true,false)}
        fn byte()->.u8'builtin {relay(true,.u8'builtin,7,9)}";
    let package = compile(source);
    for (entry, value) in [
        ("run", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(7)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            value
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let invalid = source.replace("identity(U,value)", "identity(bool,value)");
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn open_deferred_known_calls_use_defaults_literal_context_and_callee_requirements() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn identity(comptime .T:Type=Byte,value:T)->T {value}
        fn constrained(comptime T:Type,value:T)->T where requires T == bool {value}
        fn relay(comptime ignored:bool,comptime U:Type,value:U,unknown:defer Select(U))->U
            where requires U == bool {constrained(U,value)}
        fn literal(comptime U:Type,unknown:defer Select(U))->Byte {identity(9)}
        fn run()->bool {relay(false,bool,true,false)}
        fn byte()->Byte {literal(bool,true)}";
    let package = compile(source);
    for (entry, value) in [
        ("run", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            value
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let invalid = source.replace(
        "where requires U == bool {constrained(U,value)}",
        "{constrained(U,value)}",
    );
    let failure = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("type requirement is not established"),
        "{failure}"
    );
    let invalid = source.replace("identity(9)", "identity(256)");
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn open_deferred_bodies_infer_generic_calls_from_known_inputs_and_result_context() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn identity(value:T)->T where T {value}
        fn relay(comptime ignored:bool,comptime U:Type,value:U,unknown:defer Select(U))->U {identity(value)}
        fn literal(comptime U:Type,unknown:defer Select(U))->Byte {identity(9)}
        fn run()->bool {relay(false,bool,true,false)}
        fn byte()->Byte {literal(bool,true)}";
    let package = compile(source);
    for (entry, value) in [
        ("run", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            value
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let overflow = source.replace("identity(9)", "identity(256)");
    assert!(compile_with_protocols(&overflow, &Default::default()).is_err());
}

#[test]
fn open_deferred_generic_calls_preserve_trait_admission_and_definition_evidence() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        trait Allowed {} impl Allowed for bool {}
        comptime fn Select(T:Type)->Type {T}
        fn keep(value:T)->T where T :- Allowed {value}
        fn relay(comptime U:Type,unknown:defer Select(U),value:bool)->bool {keep(value)}
        fn run()->bool {relay(bool,false,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    // Check unused open definitions too: no later concrete instantiation may
    // compensate for a missing implementation or an unknown operand domain.
    for invalid in [
        source.replace("impl Allowed for bool {}", ""),
        source.replace("keep(value)", "keep(unknown)"),
        source
            .replace(
                "value:bool)->bool {keep(value)}",
                "value:Byte)->Byte {keep(value)}",
            )
            .replace("fn run()->bool {relay(bool,false,true)}", ""),
    ] {
        assert!(
            compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn open_deferred_bodies_destructure_known_inputs_with_ordinary_pattern_rules() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn split(comptime T:Type,unknown:defer Select(T),pair:Tuple<bool,Byte>)->Byte {
            let (flag,n)=pair;
            if flag {n} else {0}
        }
        fn run()->Byte {split(bool,true,(true,7))}
        fn other()->Byte {split(Byte,9,(false,7))}";
    let package = execution_support::compile_with_tuple(source);
    for (entry, value) in [
        ("run", Value::UnsignedInteger(7)),
        ("other", Value::UnsignedInteger(0)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            value
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for invalid in [
        source.replace("let (flag,n)=pair;", "let (flag,n)=unknown;"),
        source.replace("let (flag,n)=pair;", "let (flag,n)=pair;n = 9;"),
        source.replace("let (flag,n)=pair;", "let (flag,n,extra)=pair;"),
    ] {
        assert!(
            execution_support::compile_with_tuple_protocols(&invalid, &Default::default()).is_err()
        );
    }
}

#[test]
fn open_deferred_known_let_patterns_check_refutability_and_failure_scope() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn choose(comptime T:Type,unknown:defer Select(T),flag:bool)->bool {
            let value if flag = true else {return false;};value
        }
        fn yes()->bool {choose(bool,false,true)}
        fn no()->bool {choose(bool,true,false)}";
    let package = compile(source);
    for (entry, value) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(value)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for invalid in [
        source.replace("else {return false;}", ""),
        source.replace("else {return false;}", "else {false}"),
        source.replace("else {return false;}", "else {return value;}"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn open_deferred_known_regions_call_callbacks_closures_and_methods() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        trait Inspect {fn inspect(self)->bool;}
        impl Inspect for bool {fn inspect(self)->bool {self}}
        fn relay(comptime T:Type,unknown:defer Select(T),callback:fn(bool)->bool,flag:bool)->bool {
            let closure=|value:bool| callback(value);
            closure(flag).as(Inspect).inspect()
        }
        fn yes()->bool {relay(bool,false,|value| not value,false)}
        fn no()->bool {relay(.u8'builtin,7,|value| value,false)}";
    let package = compile(source);
    for (entry, value) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(value)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for invalid in [
        source.replace("callback(value)", "unknown(value)"),
        source.replace(
            "closure(flag).as(Inspect).inspect()",
            "unknown.as(Inspect).inspect()",
        ),
        source.replace("impl Inspect for bool {fn inspect(self)->bool {self}}", ""),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn open_deferred_trait_requirements_support_known_operations_and_forwarding() {
    let source = "typealias Type=.Type'builtin;
        trait Inspect {fn inspect(self)->bool;}
        impl Inspect for bool {fn inspect(self)->bool {self}}
        comptime fn Select(T:Type)->Type {T}
        fn inspect(comptime T:Type,value:T,unknown:defer Select(T))->bool
            where requires T :- Inspect {value.as(Inspect).inspect()}
        fn relay(comptime ignored:bool,comptime U:Type,value:U,unknown:defer Select(U))->bool
            where requires U :- Inspect {inspect(U,value,unknown)}
        fn run()->bool {relay(false,bool,true,false)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let instances=package.functions.iter().filter(|function|
        (function.name=="inspect" || function.name=="relay") && matches!(function.signature.callable_signature(),
            nir::Term::Function {parameters,..} if parameters.first().is_some_and(|parameter|parameter.sort!=nir::Sort::Runtime))).collect::<Vec<_>>();
    assert_eq!(instances.len(), 2);
    for function in instances {
        assert!(!function.capabilities.is_empty());
        for requirement in &function.capabilities {
            assert_eq!(
                requirement.subject,
                nir::Term::Primitive(nir::Primitive::Bool)
            );
        }
    }
    for invalid in [
        source.replace("impl Inspect for bool {fn inspect(self)->bool {self}}", ""),
        source.replace(
            "where requires U :- Inspect {inspect(U,value,unknown)}",
            "{inspect(U,value,unknown)}",
        ),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn open_deferred_trait_requirements_specialize_interface_arguments_and_defaults() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        trait Allowed where Item {} impl Allowed<bool> for bool {}
        comptime fn Select(T:Type)->Type {T}
        fn keep(comptime .A:Type=bool,comptime .B:Type=A,value:defer Select(A))->defer Select(A)
            where requires A :- Allowed<B> {value}
        fn relay(comptime T:Type,value:defer Select(T))->defer Select(T)
            where requires T :- Allowed<T> {keep(.A=T,value)}
        fn run()->bool {relay(bool,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for function in package
        .functions
        .iter()
        .filter(|function| function.name == "keep" || function.name == "relay")
    {
        assert_eq!(function.capabilities.len(), 1);
        assert_eq!(
            function.capabilities[0].interface_arguments,
            vec![nir::Term::Primitive(nir::Primitive::Bool)]
        );
    }
    let invalid = format!("{source} fn bad()->bool {{keep(.A=bool,.B=Byte,true)}}");
    let failure = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(
        failure.contains("type requirement is not established"),
        "{failure}"
    );
}

#[test]
fn open_deferred_match_arms_forward_unknown_values_with_known_pattern_evidence() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn choose(comptime T:Type,left:defer Select(T),right:defer Select(T),flag:bool)->defer Select(T) {
            flag match {true=>left,false=>right}
        }
        fn yes()->bool {choose(bool,true,false,true)}
        fn byte()->Byte {choose(Byte,7,9,false)}";
    let package = compile(source);
    for (entry, value) in [
        ("yes", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            value
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for invalid in [
        source.replace("true=>left,false=>right", "true=>left"),
        source.replace("flag match", "left match"),
        source.replace("true=>left,false=>right", "true=>left,false=>not right"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn open_deferred_match_patterns_bind_guard_and_scope_each_arm() {
    let source="typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn choose(comptime T:Type,left:defer Select(T),right:defer Select(T),pair:Tuple<bool,bool>)->defer Select(T) {
            pair match {(flag,other) if flag=>do {if other {return left;} right},_=>left}
        }
        fn first()->bool {choose(bool,true,false,(true,true))}
        fn second()->bool {choose(bool,true,false,(true,false))}
        fn fallback()->bool {choose(bool,true,false,(false,false))}";
    let package = execution_support::compile_with_tuple(source);
    for (entry, value) in [("first", true), ("second", false), ("fallback", true)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(value)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for invalid in [
        source.replace(",_=>left", ""),
        source.replace("if flag=>", "if left=>"),
        source.replace("(flag,other)", "(flag,other,extra)"),
    ] {
        assert!(
            execution_support::compile_with_tuple_protocols(&invalid, &Default::default()).is_err()
        );
    }
}

#[test]
fn open_deferred_when_branches_forward_values_and_keep_type_facts_scoped() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn choose(comptime T:Type,left:defer Select(T),right:defer Select(T),flag:bool)->defer Select(T) {
            when {flag=>left,else=>right}
        }
        fn type_branch(comptime T:Type,value:T,unknown:defer Select(T))->bool {
            when {T == bool=>value,else=>false}
        }
        fn discard(comptime T:Type,value:defer Select(T),flag:bool)->bool {
            when {flag=>value};true
        }
        fn discarded()->bool {discard(Byte,7,false)}
        fn yes()->bool {choose(bool,true,false,true)}
        fn byte()->Byte {choose(Byte,7,9,false)}
        fn typed()->bool {type_branch(bool,true,false)}
        fn fallback()->bool {type_branch(Byte,7,9)}";
    let package = compile(source);
    for (entry, value) in [
        ("yes", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
        ("typed", Value::Bool(true)),
        ("fallback", Value::Bool(false)),
        ("discarded", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            value
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for invalid in [
        source.replace(
            "when {flag=>left,else=>right}",
            "when {else=>right,flag=>left}",
        ),
        source.replace(
            "when {flag=>left,else=>right}",
            "when {left=>left,else=>right}",
        ),
        source.replace(
            "when {T == bool=>value,else=>false}",
            "when {T == bool=>value,else=>value}",
        ),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn open_deferred_if_type_facts_and_discard_match_when_semantics() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn narrow(comptime T:Type,value:T,unknown:defer Select(T))->bool {
            if T == bool {value} else {false}
        }
        fn discard(comptime T:Type,unknown:defer Select(T),flag:bool)->bool {
            if flag {unknown};true
        }
        fn first()->bool {narrow(bool,true,false)}
        fn other()->bool {narrow(Byte,7,9)}
        fn ignored()->bool {discard(Byte,7,true)}";
    let package = compile(source);
    for (entry, value) in [("first", true), ("other", false), ("ignored", true)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(value)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for branch in [
        "if T == bool {value} else {return false;}",
        "when {T == bool=>value,else=>return false}",
    ] {
        let early = compile(&source.replace("if T == bool {value} else {false}", branch));
        for (entry, value) in [("first", true), ("other", false)] {
            assert_eq!(
                luna::execution::interpret(&early, entry, 100_000).unwrap(),
                Value::Bool(value)
            );
            luna::instantiation::instantiate(&early, entry, 100_000).unwrap();
        }
    }
    for invalid in [
        source.replace(
            "if T == bool {value} else {false}",
            "if T == bool {value} else {value}",
        ),
        source.replace("if flag {unknown};true", "if unknown {unknown};true"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn open_deferred_mixed_type_conditions_preserve_short_circuit_evaluation() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn touch(p:*bool)->bool {p.* = true;true}
        fn choose(comptime T:Type,value:T,unknown:defer Select(T),p:*bool)->bool {
            if T == bool and value and touch(p) {value} else {false}
        }
        fn run()->bool {let touched=false;choose(bool,true,false,touched.ref) and touched}
        fn skip_value()->bool {let touched=false;choose(bool,false,true,touched.ref);not touched}
        fn skip_type()->bool {let touched=false;choose(Byte,7,9,touched.ref);not touched}";
    let package = compile(source);
    for entry in ["run", "skip_value", "skip_type"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(calls(&package, &vm), ["run", "choose", "touch"]);
    let reversed = compile(
        &source
            .replace(
                "T == bool and value and touch(p)",
                "touch(p) and T == bool and value",
            )
            .replace("not touched", "touched"),
    );
    for entry in ["run", "skip_value", "skip_type"] {
        assert_eq!(
            luna::execution::interpret(&reversed, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    let disjunction = compile(
        &source
            .replace(
                "if T == bool and value and touch(p) {value} else {false}",
                "if T != bool or value {false} else {value}",
            )
            .replace(
                "choose(bool,true,false,touched.ref) and touched",
                "not choose(bool,true,false,touched.ref)",
            ),
    );
    for entry in ["run", "skip_value", "skip_type"] {
        assert_eq!(
            luna::execution::interpret(&disjunction, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&disjunction, entry, 100_000).unwrap();
    }
    for invalid in [
        source.replace("T == bool and value", "value and T == bool"),
        source.replace("T == bool and value", "T == bool or value"),
        source.replace("else {false}", "else {value}"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn open_deferred_mutable_locals_preserve_delivery_and_concrete_assignment_types() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn replace(comptime T:Type,first:defer Select(T),second:defer Select(T),flag:bool)->defer Select(T) {
            var result=first;if flag {result = second;} result
        }
        fn yes()->bool {replace(bool,false,true,true)}
        fn byte()->Byte {replace(Byte,7,9,false)}";
    let package = compile(source);
    for (entry, value) in [
        ("yes", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(7)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            value
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let immutable = source.replace("var result=first", "let result=first");
    let error = compile_with_protocols(&immutable, &Default::default()).unwrap_err();
    assert!(error.contains("cannot reassign a let binding"), "{error}");
    let mismatch = source.replace("result = second", "result = true");
    assert!(compile_with_protocols(&mismatch, &Default::default()).is_err());
    let invalid = source.replace("result = second", "result = not second");
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn open_deferred_assignments_retain_concrete_move_and_replacement_checks() {
    use elaboration::verification::moves::{Delivery, analyze};
    let prefix = "typealias Type=.Type'builtin;struct Ticket {}
        comptime fn Select(T:Type)->Type {T}";
    for (body, valid, cleanup_count) in [
        ("var local=first;local=second;local", true, 1),
        ("var local=first;local=local;second;local", true, 0),
        ("var local=first;local=second;first", false, 0),
        ("var local=first;local=second;second", false, 0),
    ] {
        let package=compile(&format!("{prefix}
            fn replace(comptime T:Type,first:defer Select(T),second:defer Select(T))->defer Select(T) {{{body}}}
            fn run()->Ticket {{replace(Ticket,Ticket {{}},Ticket {{}})}}
            fn ordinary(first:Ticket,second:Ticket)->Ticket {{{body}}}"));
        // Runtime resource obligations start after staged parameter removal.
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let function = &graph
            .functions
            .iter()
            .find(|instance| instance.function.name == "replace")
            .unwrap()
            .function;
        let report = analyze(
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100_000,
        );
        assert_eq!(report.is_ok(), valid, "{body}: {report:?}");
        let ordinary = analyze(
            package
                .functions
                .iter()
                .find(|function| function.name == "ordinary")
                .unwrap(),
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100_000,
        );
        assert_eq!(ordinary.is_ok(), valid, "ordinary {body}: {ordinary:?}");
        if valid {
            assert_eq!(
                report
                    .as_ref()
                    .unwrap()
                    .cleanup_sites
                    .iter()
                    .map(|site| site.reason)
                    .collect::<Vec<_>>(),
                ordinary
                    .unwrap()
                    .cleanup_sites
                    .iter()
                    .map(|site| site.reason)
                    .collect::<Vec<_>>(),
                "{body}"
            );
            let report = report.unwrap();
            assert_eq!(
                report
                    .cleanup_sites
                    .iter()
                    .filter(|site| site.reason
                        == elaboration::verification::moves::CleanupReason::Replacement)
                    .count(),
                cleanup_count,
                "{body}: {report}"
            );
        }
    }
}

#[test]
fn open_deferred_loops_reassign_values_and_preserve_transfer_targets() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn choose(comptime T:Type,first:defer Select(T),second:defer Select(T),flag:bool)->defer Select(T) {
            var result=first;var running=flag;
            while running {result=second;running=false;continue;}
            result
        }
        fn nested(comptime T:Type,first:defer Select(T),second:defer Select(T))->defer Select(T) {
            var result=first;
            while true {while true {result=second;break;} break;}
            result
        }
        fn returned(comptime T:Type,value:defer Select(T))->defer Select(T) {
            while true {return value;} value
        }
        fn yes()->bool {choose(bool,false,true,true)}
        fn unchanged()->Byte {choose(Byte,7,9,false)}
        fn inner()->Byte {nested(Byte,7,9)}
        fn exit()->bool {returned(bool,true)}";
    let package = compile(source);
    for (entry, value) in [
        ("yes", Value::Bool(true)),
        ("unchanged", Value::UnsignedInteger(7)),
        ("inner", Value::UnsignedInteger(9)),
        ("exit", Value::Bool(true)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            value
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for invalid in [
        source.replace(
            "running=false;continue;",
            "running=false;continue;result=not second;",
        ),
        source.replace(
            "var result=first;var running=flag;",
            "let result=first;var running=flag;",
        ),
        source.replace("while running", "while first"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn open_deferred_loops_preserve_affine_backedge_checks_after_instantiation() {
    use elaboration::verification::moves::{Delivery, analyze};
    for (body, valid) in [
        ("while flag {local=second;break;}", true),
        ("while flag {local=second;continue;}", false),
        ("while true {local=second;break;}", true),
        ("while flag {local=local;break;}", true),
    ] {
        let package=compile(&format!("typealias Type=.Type'builtin;struct Ticket {{}}
            comptime fn Select(T:Type)->Type {{T}}
            fn cycle(comptime T:Type,first:defer Select(T),second:defer Select(T),flag:bool)->defer Select(T) {{
                var local=first;{body} local
            }}
            fn run()->Ticket {{cycle(Ticket,Ticket {{}},Ticket {{}},false)}}"));
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let function = &graph
            .functions
            .iter()
            .find(|instance| instance.function.name == "cycle")
            .unwrap()
            .function;
        let result = analyze(
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100_000,
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
    }
}

#[test]
fn open_deferred_named_loop_transfers_respect_nesting_and_closure_boundaries() {
    let source="typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn select(comptime T:Type,first:defer Select(T),second:defer Select(T))->defer Select(T) {
            var result=first;
            while:outer true {while true {result=second;break outer;} result=first;}
            result
        }
        fn continued(comptime T:Type,first:defer Select(T),second:defer Select(T))->defer Select(T) {
            var result=first;var running=true;
            while:outer running {running=false;while true {result=second;continue outer;} result=first;}
            result
        }
        fn run()->bool {select(bool,false,true)}
        fn again()->bool {continued(bool,false,true)}";
    let package = compile(source);
    for entry in ["run", "again"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    for invalid in [
        source.replace(
            "result=second;break outer;",
            "let action=|| {break outer;};action();",
        ),
        source.replace("result=second;break outer;", "let outer=true;break outer;"),
        source.replace("result=second;break outer;", "result=second;break absent;"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn open_deferred_type_assumptions_are_checked_before_instance_publication() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn narrowed(comptime T:Type,value:T,unknown:defer Select(T))->bool {
            assumes T == bool;not value
        }
        fn run()->bool {narrowed(bool,false,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for invalid in [
        source.replace("narrowed(bool,false,true)", "narrowed(Byte,7,9)"),
        source
            .replace(
                "unknown:defer Select(T))->bool",
                "unknown:defer Select(T),flag:bool)->bool",
            )
            .replace(
                "narrowed(bool,false,true)",
                "narrowed(bool,false,true,false)",
            )
            .replace(
                "assumes T == bool;not value",
                "if flag {assumes T == bool;};not value",
            ),
        source.replace("assumes T == bool;not value", "not value"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
    // Boolean program assumptions keep their runtime check rather than being
    // mistaken for PTC facts just because they occur inside an open definition.
    let checked = compile(
        "typealias Type=.Type'builtin;comptime fn Select(T:Type)->Type {T}
        fn guard(comptime T:Type,value:defer Select(T),flag:bool)->bool {assumes flag;flag}
        fn good()->bool {guard(bool,true,true)} fn bad()->bool {guard(bool,true,false)}",
    );
    assert_eq!(
        luna::execution::interpret(&checked, "good", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(luna::execution::interpret(&checked, "bad", 100_000).is_err());
}

#[test]
fn open_deferred_trait_assumptions_require_a_real_concrete_implementation() {
    let source="typealias Type=.Type'builtin;
        trait Inspect {fn inspect(self)->bool;} impl Inspect for bool {fn inspect(self)->bool {self}}
        comptime fn Select(T:Type)->Type {T}
        fn inspect(comptime T:Type,value:T,unknown:defer Select(T))->bool {
            assumes T :- Inspect;value.as(Inspect).inspect()
        }
        fn run()->bool {inspect(bool,true,false)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let invalid = source.replace("impl Inspect for bool {fn inspect(self)->bool {self}}", "");
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    for assumption in [
        "assumes (T :- Inspect) or T == .u8'builtin;",
        "assumes not (T :- Inspect);",
    ] {
        let invalid = source.replace("assumes T :- Inspect;", assumption);
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
    let conjunction = compile(&source.replace(
        "assumes T :- Inspect;",
        "assumes (T :- Inspect) and T == bool;",
    ));
    assert_eq!(
        luna::execution::interpret(&conjunction, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn open_type_assumptions_export_only_facts_of_completing_paths() {
    let prefix = "typealias Type=.Type'builtin;comptime fn Select(T:Type)->Type {T}";
    for statement in [
        "do {do {assumes T == bool;};};",
        "if flag {assumes T == bool;} else {assumes T == bool;}",
        "if flag {return false;} else {assumes T == bool;}",
    ] {
        let source = format!(
            "{prefix} fn narrow(comptime T:Type,value:T,unknown:defer Select(T),flag:bool)->bool {{
            {statement} not value
        }} fn run()->bool {{narrow(bool,false,true,false)}}"
        );
        let package = compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
    for statement in [
        "if flag {assumes T == bool;}",
        "while flag {assumes T == bool;break;}",
    ] {
        let source=format!("{prefix} fn invalid(comptime T:Type,value:T,unknown:defer Select(T),flag:bool)->bool {{
            {statement} not value
        }}");
        assert!(compile_with_protocols(&source, &Default::default()).is_err());
    }
}

#[test]
fn open_trait_assumptions_survive_unconditional_and_common_branch_completion() {
    let prefix = "typealias Type=.Type'builtin;trait Inspect {fn inspect(self)->bool;}
        impl Inspect for bool {fn inspect(self)->bool {self}}
        comptime fn Select(T:Type)->Type {T}";
    for statement in [
        "do {assumes T :- Inspect;};",
        "if flag {assumes T :- Inspect;} else {assumes T :- Inspect;}",
    ] {
        let source=format!("{prefix} fn inspect(comptime T:Type,value:T,unknown:defer Select(T),flag:bool)->bool {{
            {statement} value.as(Inspect).inspect()
        }} fn run()->bool {{inspect(bool,true,false,false)}}");
        let package = compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        assert!(
            compile_with_protocols(
                &source.replace("impl Inspect for bool {fn inspect(self)->bool {self}}", ""),
                &Default::default()
            )
            .is_err()
        );
    }
}

#[test]
fn staged_inputs_after_deferred_runtime_domains_keep_source_slots() {
    let source = "typealias Type=.Type'builtin;comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}fn select(comptime first:bool,a:defer Pick(first),comptime second:bool,b:defer Pick(second))->defer Pick(second) {b}fn run()->bool {let flag=select(false,7,true,true);let byte=select(true,false,false,9);flag and byte == 9}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn later_type_inputs_and_multiple_deferred_domains_are_specialized() {
    let source = "typealias Type=.Type'builtin;comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}fn identity(value:T)->T where T {value}fn choose(comptime flag:bool,unused:defer Pick(flag),comptime T:Type,value:T)->T {identity(value)}fn run()->bool {let a=choose(false,7,bool,true);let b=choose(true,false,.u8'builtin,9);a and b == 9}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn interleaved_staged_defaults_keep_phase_and_instance_checks() {
    let prefix = "typealias Type=.Type'builtin;comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}";
    let source = format!(
        "{prefix}fn choose(comptime first:bool,a:defer Pick(first),comptime .second:bool=not first,.value:defer Pick(second)=true)->defer Pick(second) {{value}}fn flag()->bool {{choose(false,7)}}fn byte()->.u8'builtin {{choose(true,false,.value=9)}}"
    );
    let package = compile(&source);
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
    }
    for declaration in [
        "fn bad(comptime first:bool,a:defer Pick(first),comptime .second:bool=a)->bool {second}",
        "fn bad(comptime first:bool,a:defer Pick(first),runtime:bool,comptime .second:bool=runtime)->bool {second}",
        "fn bad()->.u8'builtin {choose(true,false)}",
        "fn bad(flag:bool)->bool {choose(false,7,.second=flag)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source}{declaration}"), &Default::default()).is_err(),
            "{declaration}"
        );
    }
}

#[test]
fn later_staged_type_constraints_remain_in_the_original_scope() {
    let source = "typealias Type=.Type'builtin;comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}fn choose(comptime flag:bool,unused:defer Pick(flag),comptime T:Type,value:T)->bool where requires T == bool {value}fn run()->bool {choose(false,7,bool,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        compile_with_protocols(
            &format!("{source}fn bad()->bool {{choose(false,7,.u8'builtin,9)}}"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn later_type_inputs_dispatch_under_checked_trait_constraints() {
    let source = "typealias Type=.Type'builtin;comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}trait Inspect {fn inspect(self)->bool;}impl Inspect for bool {fn inspect(self)->bool {self}}fn choose(comptime flag:bool,unused:defer Pick(flag),comptime T:Type,value:T)->bool where requires T :- Inspect {value.as(Inspect).inspect()}fn run()->bool {choose(false,7,bool,true)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let invalid = source.replace("impl Inspect for bool {fn inspect(self)->bool {self}}", "");
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn later_type_inputs_keep_associated_result_identity() {
    let source = "typealias Type=.Type'builtin;comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}trait Source {assoc Item:Type;fn get(self)->Item;}impl Source for bool {assoc Item:Type=bool;fn get(self)->bool {self}}fn choose(comptime flag:bool,unused:defer Pick(flag),comptime T:Type,value:T)->T.as(Source).Item where requires T :- Source {value.as(Source).get()}fn run()->bool {choose(false,7,bool,true)}";
    let multiple = format!(
        "{source}impl Source for .u8'builtin {{assoc Item:Type=.u8'builtin;fn get(self)->.u8'builtin {{self}}}}fn byte()->.u8'builtin {{choose(true,false,.u8'builtin,9)}}"
    );
    let package = compile(&multiple);
    assert_eq!(
        luna::execution::interpret(&package, "byte", 100_000).unwrap(),
        Value::UnsignedInteger(9)
    );
    assert!(
        compile_with_protocols(
            &source.replace("where requires T :- Source", ""),
            &Default::default()
        )
        .is_err()
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        compile_with_protocols(
            &source.replace("fn run()->bool", "fn run()->.u8'builtin"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn late_type_inputs_keep_nested_closure_interfaces_and_call_order() {
    let source = "typealias Type=.Type'builtin;comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}fn first()->.u8'builtin {7}fn second()->bool {false}fn invert(value:bool)->bool {not value}fn choose(comptime flag:bool,unused:defer Pick(flag),comptime T:Type,value:T,callback:fn(T)->T)->T {let apply=|x:T| callback(x);apply(value)}fn run()->bool {choose(false,first(),bool,second(),invert)}";
    let source =
        format!("{source}fn spread()->bool {{choose(...(false,first(),bool,second(),invert))}}");
    let package = compile(&source);
    for entry in ["run", "spread"] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        let names = calls(&package, &vm);
        assert_eq!(
            names
                .iter()
                .filter(|name| ["first", "second", "choose", "invert"].contains(&name.as_str()))
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["first", "second", "choose", "invert"]
        );
    }
    let wrong = source.replace("bool,second(),invert", ".u8'builtin,first(),invert");
    assert!(compile_with_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn open_dependent_inference_separates_type_suppliers_and_rejects_bad_domains() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {bool} else {Byte}}
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn number(value:bool)->Byte {if value {9} else {7}}
        fn truth(value:Byte)->bool {value == 9}
        fn choose(comptime U:Type,comptime flag:bool,callback:fn(U)->T,value:U)
            ->defer Select(flag,T) where T {callback(value)}";
    let package = compile(&format!(
        "{source}
        fn run()->bool {{
            let n = choose(Domain(true),true,number,true);
            choose(Domain(false),true,truth,n)
        }}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for body in [
        "fn run()->Byte {choose(Byte,true,number,9)}",
        "fn run(flag:bool)->Byte {choose(Domain(flag),true,number,true)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source} {body}"), &Default::default()).is_err(),
            "{body}"
        );
    }
}

#[test]
fn open_inference_prepares_selected_staged_defaults() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        comptime fn Forever()->Type {Forever()}
        fn number(value:bool)->Byte {if value {9} else {7}}
        fn choose(comptime .U:Type=bool, comptime .flag:bool=true,
            callback:fn(U)->T,value:U)->defer Select(flag,T) where T {callback(value)}
        fn overridden(comptime .U:Type=Forever(),comptime .flag:bool=true,
            callback:fn(U)->T,value:U)->defer Select(flag,T) where T {callback(value)}";
    for body in [
        "fn run()->Byte {choose(number,true)}",
        "fn run()->Byte {overridden(number,true,.U=bool)}",
    ] {
        let package = compile(&format!("{source} {body}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::UnsignedInteger(9)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_inference_follows_default_input_dependency_chains() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        comptime fn Domain(flag:bool)->Type {if flag {bool} else {Byte}}
        comptime fn Forever()->Type {Forever()}
        fn number(value:bool)->Byte {if value {9} else {7}}
        fn choose(comptime .seed:bool=true,comptime .Base:Type=Domain(seed),
            comptime .U:Type=Base,comptime .flag:bool=true,
            callback:fn(U)->T,value:U)->defer Select(flag,T) where T {callback(value)}";
    for body in [
        "fn run()->Byte {choose(number,true)}",
        "fn run()->Byte {choose(number,true,.seed=false,.U=bool)}",
    ] {
        let package = compile(&format!("{source} {body}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::UnsignedInteger(9)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
    assert!(
        compile_with_protocols(
            &format!("{source} fn run()->Byte {{choose(number,true,.seed=false)}}"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn open_default_inference_preserves_unrelated_pending_prefix_slots() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn number(value:bool)->Byte {if value {9} else {7}}
        fn choose(comptime flag:bool, pending:defer Select(flag,T),
            comptime .U:Type=bool, callback:fn(U)->T, value:U)
            ->T where T {callback(value)}";
    let package = compile(&format!(
        "{source} fn run()->Byte {{let n=choose(true,9,number,true); n}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(9)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert!(
        compile_with_protocols(
            &format!("{source} fn run()->Byte {{choose(false,9,number,true)}}"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn open_default_evidence_substitutes_prepared_input_types() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime U:Type,comptime seed:U,comptime flag:bool,comptime .value:T=seed)
            ->defer Select(flag,T) where T,requires T == U {value}";
    let package = compile(&format!("{source} fn run()->Byte {{choose(Byte,9,true)}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(9)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut runtime = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        runtime.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::UnsignedInteger(9)
    );
    let failure = compile_with_protocols(
        &format!("{source} fn run()->bool {{choose(Byte,9,true,.value=true)}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("requirement"), "{failure}");
}

#[test]
fn open_default_dependencies_prepare_values_with_dependent_domains() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn TypeOf(value:T)->Type where T {T}
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn number(value:bool)->Byte {if value {9} else {7}}
        fn choose(comptime S:Type,comptime seed:S,comptime .U:Type=TypeOf(seed),
            comptime flag:bool,callback:fn(U)->T,value:U)
            ->defer Select(flag,T) where T {callback(value)}";
    let package = compile(&format!(
        "{source} fn run()->Byte {{choose(bool,true,true,number,true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(9)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut runtime = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        runtime.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::UnsignedInteger(9)
    );
    for body in [
        "fn run(seed:bool)->Byte {choose(bool,seed,true,number,true)}",
        "fn run()->Byte {choose(Byte,true,true,number,true)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source} {body}"), &Default::default()).is_err(),
            "{body}"
        );
    }
}

#[test]
fn deferred_callable_types_wait_for_staged_input_domains() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn number(value:bool)->Byte {if value {9} else {7}}
        fn choose(comptime flag:bool,comptime U:defer Domain(flag),
            callback:defer fn(U)->T,value:defer U)->bool
            where T {false}";
    let package = compile(&format!(
        "{source} fn run()->bool {{choose<Byte>(true,bool,number,true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert!(
        compile_with_protocols(
            &format!("{source} fn run()->bool {{choose<Byte>(false,true,number,true)}}"),
            &Default::default()
        )
        .is_err()
    );
    let wrong_callback = format!(
        "{source} fn wrong(value:bool)->bool {{value}} fn run()->bool {{choose<Byte>(true,bool,wrong,true)}}"
    );
    assert!(compile_with_protocols(&wrong_callback, &Default::default()).is_err());
    for invalid in ["fn(true)->T", "fn(U.missing)->T", "fn(U)->true"] {
        let invalid_source = source.replace("fn(U)->T", invalid);
        assert!(
            compile_with_protocols(&invalid_source, &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn open_inference_uses_newly_formed_deferred_callable_domains() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        fn number(value:bool)->Byte {if value {9} else {7}}
        fn choose(comptime flag:bool,comptime U:defer Domain(flag),
            callback:defer fn(U)->T,value:defer U)->bool where T {false}";
    let package = compile(&format!(
        "{source} fn run()->bool {{choose(true,bool,number,true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for body in [
        "fn run()->bool {choose(false,true,number,true)}",
        "fn run()->bool {choose(true,Byte,number,9)}",
        "fn run(flag:bool)->bool {choose(flag,bool,number,true)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{source} {body}"), &Default::default()).is_err(),
            "{body}"
        );
    }
}

#[test]
fn deferred_callable_inference_crosses_unrelated_pending_prefix() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn number(value:bool)->Byte {if value {9} else {7}}
        fn choose(comptime flag:bool,earlier:defer Select(flag,T),
            comptime U:defer Domain(flag),callback:defer fn(U)->T,
            value:defer U)->bool where T {false}";
    let package = compile(&format!(
        "{source} fn run()->bool {{choose(true,9,bool,number,true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert!(
        compile_with_protocols(
            &format!("{source} fn run()->bool {{choose(true,true,bool,number,true)}}"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn deferred_nested_structural_types_participate_in_inference() {
    for shape in ["*U", "?U", "*?U", "?*U"] {
        let callback_domain = shape.replace('U', "bool");
        let source=format!("typealias Type=.Type'builtin; typealias Byte=.u8'builtin; enum Maybe where E {{None,Some(E)}}
            comptime fn Domain(flag:bool)->Type {{if flag {{Type}} else {{bool}}}}
            fn number(value:{callback_domain})->Byte {{9}}
            fn choose(comptime flag:bool,comptime U:defer Domain(flag),
                callback:defer fn({shape})->T)->bool where T {{false}}
            fn run()->bool {{choose(true,bool,number)}}");
        let selectors = luna::protocols::type_constructors::Selectors {
            optional: Some("input::Maybe"),
            ..Default::default()
        };
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(false)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mismatch = source.replace("choose(true,bool,number)", "choose(true,Byte,number)");
        assert!(
            execution_support::compile_with_source_bindings(
                &mismatch,
                &Default::default(),
                selectors
            )
            .is_err(),
            "{shape}"
        );
        if shape.contains('?') {
            let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
            assert!(error.contains("bound optional declaration"), "{error}");
        }
        let invalid = source.replace("choose(true,bool,number)", "choose(false,true,number)");
        assert!(
            execution_support::compile_with_source_bindings(
                &invalid,
                &Default::default(),
                selectors
            )
            .is_err(),
            "{shape}"
        );
    }
}

#[test]
fn deferred_callable_inference_admits_ready_input_type_requirements() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        fn number(value:bool)->Byte {if value {9} else {7}}
        fn choose(comptime S:Type,comptime flag:bool,comptime U:defer Domain(flag),
            callback:defer fn(U)->T)->bool where T,requires S == bool {false}";
    let package = compile(&format!(
        "{source} fn run()->bool {{choose(bool,true,bool,number)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let failure = compile_with_protocols(
        &format!("{source} fn run()->bool {{choose(Byte,true,bool,number)}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("requirement"), "{failure}");
    let divergent = source.replace("if flag {Type} else {bool}", "Domain(flag)");
    let failure = compile_with_protocols(
        &format!("{divergent} fn run()->bool {{choose(Byte,true,bool,number)}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        failure.contains("requirement") && !failure.contains("budget"),
        "{failure}"
    );
}

#[test]
fn newly_formed_result_domains_contextualize_callback_inputs() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        fn factory(comptime flag:bool,comptime U:defer Domain(flag),
            callback:defer fn(U)->T)->defer fn(U)->T where T {callback}
        fn make()->fn(bool)->Byte {factory(true,bool,|value| if value {9} else {7})}
        fn run()->Byte {let callback=make();callback(true)}";
    let package = compile(&format!(
        "{source}
        fn make_bool()->fn(bool)->bool {{factory(true,bool,|value| not value)}}
        fn check()->bool {{let callback=make_bool();callback(false)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(9)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "check", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "check", 100_000).unwrap();
    for body in ["|value| true", "|value| 300"] {
        let invalid = source.replace("|value| if value {9} else {7}", body);
        assert!(
            compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{body}"
        );
    }
}

#[test]
fn deferred_callable_inference_preserves_named_type_components() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        typealias Result=Byte;
        comptime fn Domain(flag:bool)->Type {if flag {Type} else {bool}}
        fn number(value:bool,other:bool)->Result {if other {9} else {7}}
        fn choose(comptime flag:bool,comptime U:defer Domain(flag),
            callback:defer fn(U,T)->Result)->bool where T {false}
        fn run()->bool {choose(true,bool,number)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let invalid = source.replace(
        "fn number(value:bool,other:bool)->Result {if other {9} else {7}}",
        "fn number(value:bool,other:bool)->bool {other}",
    );
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    let opaque = source.replace(
        "typealias Result=Byte;",
        "pure comptime fn Identity<X>->Type {X} typealias Result=Identity<Byte>;",
    );
    let mismatch = opaque.replace(
        "->Result {if other {9} else {7}}",
        "->Byte {if other {9} else {7}}",
    );
    let failure = compile_with_protocols(&mismatch, &Default::default()).unwrap_err();
    assert!(!failure.contains("lack inference evidence"), "{failure}");
    let opaque = opaque.replace("{if other {9} else {7}}", "{number(value,other)}");
    let package = compile(&opaque);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn open_lambda_markers_route_tactics_post_do_and_explicit_callbacks() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn apply(comptime flag:bool,lambda action:fn(bool)->bool,value:bool)->defer Select(flag,bool) {action(value)}
        fn flip(value:bool)->bool {not value}
        fn input()->bool {false}
        fn tactic()->bool {apply(true,not _0,input())}
        fn post()->bool {apply(true,input()) do {not _0}}
        fn explicit()->bool {apply(true,|value| not value,input())}
        fn function()->bool {apply(true,flip,input())}");
    for entry in ["tactic", "post", "explicit", "function"] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        let trace = calls(&package, &vm);
        assert_eq!(trace.iter().filter(|name| **name == "input").count(), 1);
        assert!(
            trace.iter().position(|name| *name == "input").unwrap()
                < trace.iter().position(|name| *name == "apply").unwrap()
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn open_lambda_domains_form_before_tactic_elaboration() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn apply(comptime flag:bool,lambda action:defer fn(T)->T,value:T)->defer Select(flag,T) where T {action(value)}
        fn direct()->bool {apply(true,not _0,false)}
        fn post()->bool {apply(true,false) do {not _0}}
        fn nested()->bool {apply(true,_0 or apply(true,not _0,false),false)}");
    for entry in ["direct", "post", "nested"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn open_lambda_routing_rejects_invalid_placeholder_and_supply() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn apply(comptime flag:bool,lambda action:fn(bool)->bool,value:bool)->defer Select(flag,bool) {action(value)}";
    for body in [
        "apply(true,_1,true)",
        "apply(true,|x| x,true) do {_0}",
        "apply(true,7,true)",
    ] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->bool {{{body}}}"),
                &Default::default()
            )
            .is_err(),
            "{body}"
        );
    }
    let plain = prefix.replace("lambda action", "action");
    assert!(
        compile_with_protocols(
            &format!("{plain} fn bad()->bool {{apply(true,_0,true)}}"),
            &Default::default()
        )
        .is_err()
    );
    let ambiguous="typealias Type=.Type'builtin; comptime fn Result(flag:bool)->Type {bool}
        fn choose(comptime flag:bool,lambda left:fn()->bool,lambda right:fn()->bool)->defer Result(flag) {left() or right()}
        fn bad()->bool {choose(true) do {true}}";
    assert!(compile_with_protocols(ambiguous, &Default::default()).is_err());
}

#[test]
fn open_implicit_named_supply_preserves_runtime_order() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn first()->bool {true} fn supplied()->bool {false} fn last()->bool {true}
        fn choose(comptime flag:bool,first:bool,implicit supplied:bool,last:bool)->defer Select(flag,bool) {first and not supplied and last}
        fn run()->bool {choose(true,first(),.supplied=supplied(),last())}
        fn spread()->bool {choose(true,...(first(),),.supplied=supplied(),...(last(),))}");
    for entry in ["run", "spread"] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            calls(&package, &vm),
            vec![entry, "first", "supplied", "last", "choose"]
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn open_implicit_supply_supports_staged_types_and_dependent_defaults() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn typed(implicit comptime T:Type,value:defer T)->defer T {value}
        fn pick(comptime flag:bool,value:T,implicit supplied:T)->defer Select(flag,T) where T {supplied}
        fn direct()->bool {typed(true,.T=bool)}
        fn with_default(comptime flag:bool,implicit supplied:bool,.fallback:bool=supplied)->defer Select(flag,bool) {fallback}
        fn defaulted()->bool {with_default(true,.supplied=true)}
        fn forward(value:T,implicit supplied:T)->T where T {pick(true,value,.supplied=supplied)}
        fn generic()->bool {forward(false,.supplied=true)}");
    for entry in ["direct", "defaulted", "generic"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn open_implicit_supply_rejects_missing_duplicate_and_wrong_stage() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime flag:bool,value:bool,implicit supplied:bool)->defer Select(flag,bool) {supplied}";
    let missing = compile_with_protocols(
        &format!("{prefix} fn bad()->bool {{let supplied=true;choose(true,false)}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        missing.contains("implicit parameter requires explicit named supply"),
        "{missing}"
    );
    for args in [
        "true,false,.supplied=true,.supplied=false",
        "true,false,.supplied=7",
        "true,false,true",
    ] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->bool {{choose({args})}}"),
                &Default::default()
            )
            .is_err(),
            "{args}"
        );
    }
    assert!(compile_with_protocols("typealias Type=.Type'builtin; fn pick(implicit comptime T:Type,value:defer T)->defer T {value} fn bad(T:Type)->bool {pick(true,.T=T)}",&Default::default()).is_err());
}

#[test]
fn open_implicit_comptime_supply_cannot_capture_runtime_flags() {
    let failure = compile_with_protocols(
        "typealias Type=.Type'builtin;
        comptime fn Result(flag:bool)->Type {bool}
        fn choose(implicit comptime flag:bool,value:bool)->defer Result(flag) {value}
        fn bad(flag:bool)->bool {choose(true,.flag=flag)}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        failure.contains("runtime") || failure.contains("comptime"),
        "{failure}"
    );
}

#[test]
fn open_marked_callbacks_compose_with_implicit_and_variadic_inputs() {
    for (call, expected_calls) in [
        (
            "combine(true,first(),.bias=bias(),second()) do {body(_0,_1)}",
            vec!["run", "first", "bias", "second", "combine", "body"],
        ),
        (
            "combine(true,...(first(),),.bias=bias(),...(second(),)) do {body(_0,_1)}",
            vec!["run", "first", "bias", "second", "combine", "body"],
        ),
        (
            "combine(true,.items=packet(),.bias=bias()) do {body(_0,_1)}",
            vec!["run", "packet", "bias", "combine", "body"],
        ),
    ] {
        let package=execution_support::compile_with_tuple_protocols(&format!("typealias Type=.Type'builtin;
            comptime fn Select(flag:bool)->Type {{bool}}
            fn first()->bool {{false}} fn second()->bool {{true}} fn bias()->bool {{true}}
            fn packet()->Tuple<bool,bool> {{(false,true)}}
            fn body(a:bool,b:bool)->bool {{not a and b}}
            fn combine(comptime flag:bool,lambda action:fn(bool,bool)->bool,implicit bias:bool,...items:Tuple<bool,bool>)->defer Select(flag) {{let (a,b)=items;bias and action(a,b)}}
            fn run()->bool {{{call}}}"), &Default::default()).unwrap_or_else(|error| panic!("{call}: {error}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true),
            "{call}"
        );
        assert_eq!(calls(&package, &vm), expected_calls, "{call}");
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_extended_callbacks_route_distinct_tuple_and_property_collectors() {
    for call in [
        "apply {true,first(),flag:property(),last()} do {body(_0,_1)}",
        "apply {true,first(),flag:property(),last()} do |a,b| body(a,b)",
        "apply {true,...(first(),),flag:property(),...(last(),)} do {body(_0,_1)}",
    ] {
        let source = format!("typealias Type=.Type'builtin;
            comptime fn Select(flag:bool)->Type {{bool}}
            comptime fn schema(...properties:T)->T where T {{properties}}
            const SAMPLE=schema {{flag:true}};typealias Options=SAMPLE'type;
            fn first()->bool {{true}} fn property()->bool {{true}} fn last()->bool {{false}}
            fn body(a:bool,b:bool)->bool {{a and not b}}
            fn apply(comptime flag:bool,lambda action:fn(bool,bool)->bool,...items:Tuple<bool,bool>,...properties:Options)->defer Select(flag) {{let (a,b)=items;properties.flag and action(a,b)}}
            fn run()->bool {{{call}}}");
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{call}: {error}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            calls(&package, &vm),
            vec!["run", "first", "property", "last", "apply", "body"]
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_property_routing_rejects_ambiguous_entrances_and_implicit_substitution() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Select(flag:bool)->Type {bool}
        comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;";
    for parameters in [
        "...left:Options,...right:Options",
        "...left:defer Options,...right:defer Options",
    ] {
        let source = format!(
            "{prefix} fn apply(comptime flag:bool,{parameters})->defer Select(flag) {{true}} fn bad()->bool {{apply {{true,flag:true}}}}"
        );
        let failure = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            failure.contains("one variadic entrance"),
            "{parameters}: {failure}"
        );
    }
    let failure=compile_with_protocols(&format!("{prefix}
        fn apply(comptime flag:bool,implicit flag_value:bool,...properties:Options)->defer Select(flag) {{flag_value}}
        fn bad()->bool {{apply {{true,flag_value:true}}}}"),&Default::default()).unwrap_err();
    assert!(
        failure.contains("implicit parameter requires explicit named supply"),
        "{failure}"
    );
}

#[test]
fn explicit_open_extended_calls_preserve_clause_and_runtime_inputs() {
    for call in [
        "apply<bool> {true,first(),flag:property(),last()} do {_0 and not _1}",
        "apply<bool> {true,...(first(),),flag:property(),...(last(),)} do {_0 and not _1}",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Select(flag:bool)->Type {{bool}}
            comptime fn schema(...properties:T)->T where T {{properties}}
            const SAMPLE=schema {{flag:true}};typealias Options=SAMPLE'type;
            fn first()->bool {{true}} fn property()->bool {{true}} fn last()->bool {{false}}
            fn apply(comptime flag:bool,lambda action:fn(T,T)->bool,...items:Tuple<T,T>,...properties:Options)->defer Select(flag) where T {{let (a,b)=items;properties.flag and action(a,b)}}
            fn run()->bool {{{call}}}");
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{call}: {error}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            calls(&package, &vm),
            vec!["run", "first", "property", "last", "apply"]
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_extended_property_domains_form_from_explicit_comptime_inputs() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;
        fn property()->bool {true}
        fn collect(comptime T:Type,...properties:defer T)->defer T {properties}";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let result=collect {{Options,flag:property()}};result.flag}}"
    ));
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(calls(&package, &vm), vec!["run", "property", "collect"]);
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for call in [
        "collect {Options,flag:7}",
        "collect {Options,missing:true}",
        "collect {bool,flag:true}",
    ] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->bool {{let result={call};true}}"),
                &Default::default()
            )
            .is_err(),
            "{call}"
        );
    }
}

#[test]
fn explicit_open_extended_calls_check_clause_requirements() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Select(flag:bool)->Type {bool}
        comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;
        fn apply(comptime flag:bool,value:T,...properties:Options)->defer Select(flag) where T,requires T==bool {properties.flag}";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{apply<bool> {{true,false,flag:true}}}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for call in [
        "apply<bool> {true,7,flag:true}",
        "apply<bool> {true,false,flag:7}",
        "apply<.u8'builtin> {true,7,flag:true}",
        "apply<bool,bool> {true,false,flag:true}",
    ] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->bool {{{call}}}"),
                &Default::default()
            )
            .is_err(),
            "{call}"
        );
    }
}

#[test]
fn explicit_open_property_routing_uses_specialized_collector_domains() {
    for call in [
        "collect<Tuple<bool,bool>,Options> {true,first(),flag:property(),last()}",
        "collect<Tuple<bool,bool>,Options> {true,...(first(),),flag:property(),...(last(),)}",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Select(flag:bool,T:Type)->Type {{T}}
            comptime fn schema(...properties:T)->T where T {{properties}}
            const SAMPLE=schema {{flag:true}};typealias Options=SAMPLE'type;
            fn first()->bool {{true}} fn property()->bool {{true}} fn last()->bool {{false}}
            fn collect(comptime flag:bool,...items:T,...properties:U)->defer Select(flag,U) where T,U {{properties}}
            fn run()->bool {{let result={call};result.flag}}");
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{call}: {error}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            calls(&package, &vm),
            vec!["run", "first", "property", "last", "collect"]
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn specialized_open_property_entrances_do_not_leak_between_calls() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {T}
        comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;
        fn first()->bool {true} fn property()->bool {true} fn last()->bool {false}
        fn consume(comptime flag:bool,...left:T,...right:U)->defer Select(flag,bool) where T,U {true}";
    let package = execution_support::compile_with_tuple(&format!(
        "{prefix}
        fn run()->bool {{
            let a=consume<Tuple<bool,bool>,Options> {{true,first(),flag:property(),last()}};
            let b=consume<Options,Tuple<bool,bool>> {{true,first(),flag:property(),last()}};
            a and b
        }}"
    ));
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec![
            "run", "first", "property", "last", "consume", "first", "property", "last", "consume"
        ]
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for (types, expected) in [
        ("Options,Options", "one variadic entrance"),
        ("Tuple<bool,bool>,Tuple<bool,bool>", "one variadic entrance"),
    ] {
        let failure = execution_support::compile_with_tuple_protocols(
            &format!("{prefix} fn bad()->bool {{consume<{types}> {{true,flag:true}}}}"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(failure.contains(expected), "{failure}");
    }
}

#[test]
fn open_property_entrances_use_defaulted_explicit_type_clauses() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {T}
        comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;
        fn collect(comptime flag:bool,...items:T,...properties:U)->defer Select(flag,U) where T,.U:Type=Options {properties}
        fn run()->bool {let value=collect<Tuple<bool,bool>> {true,false,flag:true,true};value.flag}
        fn named()->bool {let value=collect<Tuple<bool,bool>,.U=Options> {true,false,flag:true,true};value.flag}");
    for entry in ["run", "named"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn open_property_preparation_preserves_early_returns() {
    for (call, expected_calls) in [
        (
            "consume<Tuple<bool,bool>,Options> {true,first(),flag:do {return false;},last()}",
            vec!["run", "first"],
        ),
        (
            "consume<Tuple<bool,bool>,Options> {true,do {return false;},flag:property(),last()}",
            vec!["run"],
        ),
        (
            "consume<Tuple<bool,bool>,Options> {true,...(first(),),flag:do {return false;},...(last(),)}",
            vec!["run", "first"],
        ),
    ] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Select(flag:bool,T:Type)->Type {{T}}
            comptime fn schema(...properties:T)->T where T {{properties}}
            const SAMPLE=schema {{flag:true}};typealias Options=SAMPLE'type;
            fn first()->bool {{true}} fn property()->bool {{true}} fn last()->bool {{false}}
            fn consume(comptime flag:bool,...left:T,...right:U)->defer Select(flag,bool) where T,U {{true}}
            fn run()->bool {{{call}}}");
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{call}: {error}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(calls(&package, &vm), expected_calls, "{call}");
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_spread_packets_receive_known_numeric_domains() {
    for packet in ["do {(1,2)}", "if true {(1,2)} else {(3,4)}"] {
        let source = format!(
            "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
            comptime fn Result(flag:bool)->Type {{Byte}}
            fn first(comptime flag:bool,a:Byte,b:Byte)->defer Result(flag) {{a}}
            fn run()->Byte {{first(true,...{packet})}}"
        );
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{packet}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::UnsignedInteger(1)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_spread_contexts_preserve_inference_and_reject_invalid_packets() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {T}
        fn first(comptime flag:bool,a:Byte,b:Byte)->defer Select(flag,Byte) {a}
        fn infer(comptime flag:bool,a:T)->defer Select(flag,T) where T {a}";
    let package = compile(&format!(
        "{prefix} fn run()->.i32'builtin {{infer(true,...do {{(1,)}})}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::SignedInteger(1)
    );
    for packet in ["do {(300,2)}", "do {(true,2)}", "do {(1,2,3)}", "do {(1,)}"] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->Byte {{first(true,...{packet})}}"),
                &Default::default()
            )
            .is_err(),
            "{packet}"
        );
    }
}

#[test]
fn open_property_spreads_use_instantiated_numeric_domains_once() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {T}
        comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;
        fn mark()->bool {true} fn property()->bool {true}
        fn collect(comptime flag:bool,...items:T,...properties:Options)->defer Select(flag,T) where T {items}
        fn run()->Tuple<Byte,Byte> {collect<Tuple<Byte,Byte>> {true,...do {let ignored=mark();(1,2)},flag:property()}}");
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Tuple(vec![Value::UnsignedInteger(1), Value::UnsignedInteger(2)])
    );
    assert_eq!(
        calls(&package, &vm),
        vec!["run", "mark", "property", "collect"]
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn explicit_open_calls_contextualize_computed_spread_packets() {
    for packet in ["do {(1,2)}", "if true {(1,2)} else {(3,4)}"] {
        let source = format!(
            "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
            comptime fn Select(flag:bool,T:Type)->Type {{T}}
            fn first(comptime flag:bool,a:T,b:T)->defer Select(flag,T) where T {{a}}
            fn run()->Byte {{first<Byte>(true,...{packet})}}"
        );
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{packet}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::UnsignedInteger(1)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn explicit_open_spreads_preserve_defaults_stages_and_runtime_order() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {T}
        fn mark()->bool {true}
        fn first(comptime flag:bool,a:T,b:U)->defer Select(flag,T) where T,.U:Type=T {a}";
    for (call, expected_calls, expected) in [
        (
            "first<Byte>(true,...do {let ignored=mark();(1,2)})",
            vec!["run", "mark", "first"],
            1,
        ),
        (
            "first<Byte,.U=Byte>(true,...do {let ignored=mark();(1,2)})",
            vec!["run", "mark", "first"],
            1,
        ),
        ("first<Byte>(...do {(true,1,2)})", vec!["run", "first"], 1),
        (
            "first<Byte>(true,...do {if mark() {return 9;};(1,2)})",
            vec!["run", "mark"],
            9,
        ),
    ] {
        let package = compile_with_protocols(
            &format!("{prefix} fn run()->Byte {{{call}}}"),
            &Default::default(),
        )
        .unwrap_or_else(|error| panic!("{call}: {error}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::UnsignedInteger(expected)
        );
        assert_eq!(calls(&package, &vm), expected_calls, "{call}");
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
    for call in [
        "first<Byte>(true,...do {(300,2)})",
        "first<Byte>(true,...do {(1,true)})",
        "first<Byte>(true,...do {(1,2,3)})",
        "first<Byte>(...do {(mark(),1,2)})",
    ] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->Byte {{{call}}}"),
                &Default::default()
            )
            .is_err(),
            "{call}"
        );
    }
}

#[test]
fn open_calls_read_multiple_staged_inputs_from_one_packet() {
    let prefix="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn choose(comptime first:bool,comptime second:bool,value:Byte)->defer Select(first and second,Byte) {value}";
    let package = compile(&format!(
        "{prefix} fn run()->Byte {{choose(...do {{(true,true,7)}})}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for fields in ["true,false,7", "false,true,7"] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->Byte {{choose(...do {{({fields})}})}}"),
                &Default::default()
            )
            .is_err(),
            "{fields}"
        );
    }
}

#[test]
fn open_spreads_retain_stages_when_input_domains_are_pending() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {T}
        fn identity(comptime flag:bool,comptime T:Type,value:defer Select(flag,T))->defer Select(flag,T) {value}
        fn run()->bool {identity(...do {(true,bool,true)})}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn partial_open_packet_contexts_preserve_independent_runtime_packets() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {T}
        fn mark()->bool {true}
        fn identity(comptime flag:bool,comptime T:Type,value:defer Select(flag,T))->defer Select(flag,T) {value}
        fn inferred(comptime flag:bool,value:T)->defer Select(flag,T) where T {value}";
    for (call, expected_calls) in [
        (
            "identity(...do {(true,bool)},...do {(mark(),)})",
            vec!["run", "mark", "identity"],
        ),
        ("inferred(...do {(true,true)})", vec!["run", "inferred"]),
    ] {
        let package = compile_with_protocols(
            &format!("{prefix} fn run()->bool {{{call}}}"),
            &Default::default(),
        )
        .unwrap_or_else(|error| panic!("{call}: {error}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(calls(&package, &vm), expected_calls);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
    for call in [
        "identity(...do {(true,bool,mark())})",
        "identity(...do {(true,bool,7)})",
        "identity(...do {(true,true,true)})",
        "identity(...do {(true,bool,true,false)})",
        "inferred(...do {(mark(),true)})",
    ] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->bool {{{call}}}"),
                &Default::default()
            )
            .is_err(),
            "{call}"
        );
    }
}

#[test]
fn property_spreads_preserve_stages_of_pending_open_domains() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {T}
        comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;
        fn property()->bool {true}
        fn collect(comptime flag:bool,comptime T:Type,value:defer Select(flag,T),...properties:Options)->defer Select(flag,T) {value}
        fn run()->bool {collect {...do {(true,bool,true)},flag:property()}}");
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(calls(&package, &vm), vec!["run", "property", "collect"]);
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn partially_known_open_packets_keep_numeric_field_contexts() {
    for packet in [
        "do {(true,bool,7,true)}",
        "if true {(true,bool,7,true)} else {(true,bool,9,false)}",
    ] {
        let source=format!("typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
            comptime fn Select(flag:bool,T:Type)->Type {{T}}
            fn choose(comptime flag:bool,comptime T:Type,tag:Byte,value:defer Select(flag,T))->defer Select(flag,T) {{value}}
            fn run()->bool {{choose(...{packet})}}");
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{packet}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn partial_open_packet_contexts_check_known_fields_in_all_branches() {
    let prefix="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {T}
        fn choose(comptime flag:bool,comptime T:Type,tag:Byte,value:defer Select(flag,T))->defer Select(flag,T) {value}";
    for packet in [
        "do {(true,bool,300,true)}",
        "if true {(true,bool,7,true)} else {(true,bool,300,true)}",
        "do {(true,bool,false,true)}",
        "do {(true,bool,7,9)}",
    ] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->bool {{choose(...{packet})}}"),
                &Default::default()
            )
            .is_err(),
            "{packet}"
        );
    }
}

#[test]
fn partial_open_packet_contexts_support_bound_tuple_protocols() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn List<T>->Type {List<T>}
        pure comptime fn Tuple<...Es:List<Type>>->Type {Tuple<...Es>}
        comptime fn Select(flag:bool,T:Type)->Type {T}
        fn choose(comptime flag:bool,comptime T:Type,tag:Byte,value:defer Select(flag,T))->defer Select(flag,T) {value}
        fn run()->bool {choose(...do {(true,bool,7,true)})}";
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::List"),
        tuple: Some("input::Tuple"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn partial_property_packet_contexts_preserve_known_numeric_fields() {
    let package=compile("typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {T}
        comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;
        fn property()->bool {true}
        fn collect(comptime flag:bool,comptime T:Type,tag:Byte,value:defer Select(flag,T),...properties:Options)->defer Select(flag,T) {value}
        fn run()->bool {collect {flag:property(),...do {(true,bool,7,true)}}}");
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(calls(&package, &vm), vec!["run", "property", "collect"]);
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn comptime_open_instances_participate_in_required_invocation() {
    let package=compile_with_source_bindings("typealias Type=.Type'builtin;
        trait Splice {assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;}
        impl Splice for bool {assoc RuntimeTarget:Type=bool;fn splice(comptime data:bool)->bool {data}}
        comptime fn Select(flag:bool)->Type {bool}
        comptime fn identity(comptime flag:bool,value:defer Select(flag))->defer Select(flag) {value}
        const RESULT=identity(true,true);
        fn run()->bool {RESULT}", &Default::default(), luna::protocols::type_constructors::Selectors {splice:Some("input::Splice"),..Default::default()}).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn comptime_open_calls_preserve_phase_and_splice_delivery() {
    let source="typealias Type=.Type'builtin;
        trait Splice {assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;}
        impl Splice for bool {assoc RuntimeTarget:Type=bool;fn splice(comptime data:bool)->bool {data}}
        comptime fn Select(flag:bool,T:Type)->Type {T}
        comptime fn ready()->bool {true}
        comptime fn choose(comptime flag:bool,value:T)->defer Select(flag,T) where T {let checked=ready();if checked {value} else {value}}
        comptime fn relay(comptime flag:bool,value:defer Select(flag,bool))->defer Select(flag,bool) {choose<bool>(true,value)}
        fn nested()->bool {relay(true,true)}
        fn direct()->bool {choose<bool>(true,true)}
        fn spread()->bool {choose<bool>(...do {(true,true)})}";
    let package = compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            splice: Some("input::Splice"),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        package
            .functions
            .iter()
            .any(|function| function.name == "choose")
    );
    assert!(
        package
            .functions
            .iter()
            .filter(|function| function.name == "choose")
            .all(|function| function.phase == nir::program::FunctionPhase::Comptime)
    );
    for entry in ["direct", "spread", "nested"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
        let body = &package
            .functions
            .iter()
            .find(|function| function.name == entry)
            .unwrap()
            .body;
        nir::visit::expression(body, &mut 100_000, |expression, _| {
            if let nir::program::ExprKind::Function(id) = expression.kind {
                assert!(
                    !package
                        .functions
                        .iter()
                        .any(|function| function.declaration == id && function.name == "choose")
                );
            }
            Ok(())
        })
        .unwrap();
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let failure = compile_with_source_bindings(
        &format!("{source} fn bad(flag:bool)->bool {{choose<bool>(flag,true)}}"),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            splice: Some("input::Splice"),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        failure.contains("runtime") && failure.contains("comptime"),
        "{failure}"
    );
}

#[test]
fn comptime_open_results_can_supply_types() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {T}
        comptime fn ValueType(comptime flag:bool,value:defer Select(flag,Type))->defer Select(flag,Type) {value}
        typealias Flag=ValueType(true,bool);
        fn run()->Flag {true}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn comptime_open_definitions_reject_runtime_operations_and_unspliced_results() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Select(flag:bool)->Type {bool}";
    for body in ["runtime()", "let ignored=runtime();value"] {
        let source = format!(
            "{prefix} fn runtime()->bool {{true}} comptime fn bad(comptime flag:bool,value:defer Select(flag))->defer Select(flag) {{{body}}}"
        );
        let failure = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            failure.contains("unavailable in this execution phase"),
            "{failure}"
        );
    }
    for use_site in [
        "fn bad()->bool {identity(true,true)}",
        "const BAD=identity(true,7);",
    ] {
        assert!(compile_with_protocols(&format!("{prefix} comptime fn identity(comptime flag:bool,value:defer Select(flag))->defer Select(flag) {{value}} {use_site}"),&Default::default()).is_err(),"{use_site}");
    }
}

#[test]
fn comptime_open_instances_keep_value_dependent_domains_distinct() {
    let package=compile("typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Selected(flag:bool)->Type {if flag {bool} else {Byte}}
        comptime fn identity(comptime flag:bool,value:defer Selected(flag))->defer Selected(flag) {value}
        const FLAG=identity(true,true);
        const NUMBER=identity(false,7);
        typealias FlagType=FLAG'type;
        typealias NumberType=NUMBER'type;
        fn flag()->FlagType {true}
        fn number()->NumberType {7}");
    assert_eq!(
        luna::execution::interpret(&package, "flag", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "number", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
    let instances: Vec<_> = package
        .functions
        .iter()
        .filter(|function| function.name == "identity")
        .collect();
    assert_eq!(instances.len(), 2);
    assert_ne!(instances[0].signature, instances[1].signature);
    for entry in ["flag", "number"] {
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn ordinary_comptime_parameters_can_form_open_domains() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Selected(flag:bool)->Type {if flag {bool} else {Byte}}
        comptime fn identity(flag:bool,value:defer Selected(flag))->defer Selected(flag) {value}
        const FLAG=identity(true,true);const NUMBER=identity(false,7);
        typealias FlagType=FLAG'type;typealias NumberType=NUMBER'type;
        fn flag()->FlagType {true} fn number()->NumberType {7}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "flag", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "number", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
    for entry in ["flag", "number"] {
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn ordinary_comptime_dependencies_preserve_source_prefix_failures() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Selected(flag:bool)->Type {bool}
        comptime fn checked(ok:bool)->bool {assumes ok;true}
        comptime fn spin()->bool {while true {} true}
        comptime fn choose(tag:bool,.flag:bool=true,value:bool)->defer Selected(flag) {value}";
    let failure = compile_with_protocols(
        &format!("{prefix} const BAD=choose(checked(false),true,.flag=spin());"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    assert!(!failure.contains("budget"), "{failure}");
    let package = compile(&format!(
        "{prefix} const GOOD=choose(checked(true),true);typealias Flag=GOOD'type;fn run()->Flag {{true}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_comptime_dependencies_preserve_parameter_sorts() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn Selected(flag:bool)->Type {bool}
        comptime fn producer()->bool {true}
        comptime fn choose(tag:bool,flag:bool,value:bool)->defer Selected(flag) {tag and value}
        const RESULT=choose(producer(),producer(),true);
        typealias Flag=RESULT'type;fn run()->Flag {true}",
    );
    let function = package
        .functions
        .iter()
        .find(|function| function.name == "choose")
        .unwrap();
    let nir::Term::Function { parameters, .. } = &function.signature else {
        panic!("signature")
    };
    assert!(
        parameters
            .iter()
            .all(|parameter| parameter.sort == nir::Sort::Runtime)
    );
    assert_eq!(function.phase, nir::program::FunctionPhase::Comptime);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_comptime_dependencies_accept_computed_packets() {
    for packet in [
        "do {(true,true)}",
        "if true {(true,true)} else {(false,false)}",
    ] {
        let package = compile(&format!("typealias Type=.Type'builtin;
            comptime fn Selected(flag:bool)->Type {{bool}}
            comptime fn identity(flag:bool,value:defer Selected(flag))->defer Selected(flag) {{value}}
            const RESULT=identity(...{packet});
            typealias Flag=RESULT'type;fn run()->Flag {{true}}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn ordinary_comptime_packets_reject_runtime_producers_and_wrong_domains() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Selected(flag:bool)->Type {bool}
        comptime fn identity(flag:bool,value:defer Selected(flag))->defer Selected(flag) {value}
        fn runtime()->bool {true}";
    for packet in [
        "do {(runtime(),true)}",
        "do {(true,7)}",
        "if true {(true,true)} else {(true,7)}",
    ] {
        let error = compile_with_protocols(
            &format!("{prefix} const BAD=identity(...{packet});"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            !error.contains("cyclic or excessively deep"),
            "{packet}: {error}"
        );
        assert!(error.contains("elaboration failed"), "{packet}: {error}");
    }
}

#[test]
fn ordinary_comptime_signature_dependencies_follow_default_chains() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Selected(flag:bool)->Type {bool}
        comptime fn choose(seed:bool,.first:bool=seed,.second:bool=first)->defer Selected(second) {second}
        comptime fn check(value:bool,expected:bool)->bool {assumes value == expected;true}
        const FLAG=check(choose(true),true);const OTHER=check(choose(false),false);
        const OVERRIDE=check(choose(false,.first=true),true);
        typealias Flag=FLAG'type;typealias Other=OTHER'type;typealias Override=OVERRIDE'type;
        fn flag()->Flag {true} fn other()->Other {true} fn override()->Override {true}");
    for entry in ["flag", "other", "override"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn ordinary_comptime_inputs_feed_staged_defaults_without_reordering_sources() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Selected(flag:bool)->Type {bool}
        comptime fn choose(seed:bool,comptime .flag:bool=seed)->defer Selected(flag) {flag}
        comptime fn check(value:bool,expected:bool)->bool {assumes value == expected;true}";
    let package = compile(&format!(
        "{prefix}
        const CHECK=check(choose(false),false);
        const OVERRIDE=check(choose(false,.flag=true),true);
        typealias Flag=CHECK'type;typealias Override=OVERRIDE'type;
        fn run()->Flag {{true}} fn overridden()->Override {{true}}"
    ));
    for entry in ["run", "overridden"] {
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    let failure = compile_with_protocols(
        &format!(
            "{prefix}
        comptime fn spin()->bool {{while true {{}} true}}
        const BAD=choose(check(false,true),.flag=spin());"
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    assert!(!failure.contains("budget"), "{failure}");
}

#[test]
fn ordinary_comptime_default_preparation_preserves_declaration_order() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Selected(flag:bool)->Type {bool}
        comptime fn checked(ok:bool)->bool {assumes ok;true}
        comptime fn spin()->bool {while true {} true}
        comptime fn choose(.first:bool=defer checked(false),.second:bool=defer spin())->defer Selected(second) {second}";
    let failure = compile_with_protocols(
        &format!("{prefix} const BAD=choose();"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    assert!(!failure.contains("budget"), "{failure}");
    let package = compile(&format!(
        "{prefix} const GOOD=choose(.first=true,.second=true);typealias Flag=GOOD'type;fn run()->Flag {{true}}"
    ));
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn ordinary_comptime_trailing_defaults_execute_only_when_omitted() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Selected(flag:bool)->Type {bool}
        comptime fn checked(ok:bool)->bool {assumes ok;true}
        comptime fn choose(flag:bool,.tail:bool=defer checked(false))->defer Selected(flag) {tail}";
    let failure = compile_with_protocols(
        &format!("{prefix} const BAD=choose(true);"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    let package = compile(&format!(
        "{prefix} const GOOD=choose(true,.tail=true);typealias Flag=GOOD'type;fn run()->Flag {{true}}"
    ));
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn ordinary_comptime_dependencies_prepare_interleaved_collections() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Selected(flag:bool)->Type {bool}
        comptime fn checked(value:bool)->bool {assumes value;value}
        comptime fn choose(.flag:bool=true,...values:Tuple<bool,bool>)->defer Selected(flag) {let (first,second)=values;first and second}
        comptime fn verify(value:bool)->bool {assumes value;true}";
    for arguments in [
        "checked(true),.flag=true,checked(true)",
        "checked(true),checked(true)",
        ".flag=true,checked(true),checked(true)",
    ] {
        let package = execution_support::compile_with_tuple(&format!(
            "{prefix} const GOOD=verify(choose({arguments}));typealias Flag=GOOD'type;fn run()->Flag {{true}}"
        ));
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
    let failure = execution_support::compile_with_tuple_protocols(
        &format!(
            "{prefix}
        comptime fn spin()->bool {{while true {{}} true}}
        const BAD=choose(checked(false),.flag=spin(),true);"
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
}

#[test]
fn ordinary_comptime_collected_values_can_form_signature_domains() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Selected(values:Tuple<bool,bool>)->Type {bool}
        comptime fn choose(.tag:bool=true,...values:Tuple<bool,bool>)->defer Selected(values) {let (first,second)=values;first and second}
        comptime fn checked(value:bool)->bool {assumes value;value}
        comptime fn spin()->bool {while true {} true}";
    let package = execution_support::compile_with_tuple(&format!(
        "{prefix} const GOOD=checked(choose(true,.tag=true,true));typealias Flag=GOOD'type;fn run()->Flag {{true}}"
    ));
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for arguments in [
        "checked(false),.tag=spin(),true",
        "true,.tag=checked(false),spin()",
    ] {
        let failure = execution_support::compile_with_tuple_protocols(
            &format!("{prefix} const BAD=choose({arguments});"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            failure.contains("ProgramContractRejected"),
            "{arguments}: {failure}"
        );
    }
    for arguments in ["true", "true,false,true", "true,.tag=true,7"] {
        assert!(
            execution_support::compile_with_tuple_protocols(
                &format!("{prefix} const BAD=choose({arguments});"),
                &Default::default()
            )
            .is_err(),
            "{arguments}"
        );
    }
}

#[test]
fn ordinary_comptime_collections_preserve_bound_tuple_layouts() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn List<T>->Type {List<T>}
        pure comptime fn Tuple<...Es:List<Type>>->Type {Tuple<...Es>}
        comptime fn Selected(flag:bool)->Type {bool}
        comptime fn pair(.flag:bool=true,...values:Tuple<bool,Byte>)->defer Selected(flag) {let (first,second)=values;first}
        comptime fn empty(.flag:bool=true,...values:Tuple<>)->defer Selected(flag) {flag}
        comptime fn checked(value:bool)->bool {assumes value;value}
        const PAIR=checked(pair(true,.flag=true,7));const EMPTY=checked(empty());
        typealias Pair=PAIR'type;typealias Empty=EMPTY'type;
        fn pair_result()->Pair {true} fn empty_result()->Empty {true}";
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::List"),
        tuple: Some("input::Tuple"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    for entry in ["pair_result", "empty_result"] {
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn ordinary_comptime_type_inputs_form_direct_deferred_domains() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn identity(T:Type,value:defer T)->defer T {value}
        const FLAG=identity(bool,true);const NUMBER=identity(Byte,7);
        typealias Flag=FLAG'type;typealias Number=NUMBER'type;
        fn flag()->Flag {true} fn number()->Number {7}",
    );
    for (entry, expected) in [
        ("flag", Value::Bool(true)),
        ("number", Value::UnsignedInteger(7)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn ordinary_comptime_type_inputs_support_pending_domains_defaults_and_packets() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn relay(T:Type,U:defer T,value:defer U)->defer U {value}
        comptime fn defaulted(T:Type,.U:Type=T,.value:defer U=true)->defer U {value}
        comptime fn identity(T:Type,value:defer T)->defer T {value}
        comptime fn checked(value:bool)->bool {assumes value;value}
        const NESTED=checked(relay(Type,bool,true));
        const DEFAULT=checked(defaulted(bool));
        const PACKET=checked(identity(...do {(bool,true)}));
        typealias Nested=NESTED'type;typealias Default=DEFAULT'type;typealias Packet=PACKET'type;
        fn nested()->Nested {true} fn defaulted_result()->Default {true} fn packet()->Packet {true}");
    for entry in ["nested", "defaulted_result", "packet"] {
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn ordinary_comptime_type_inputs_reject_value_domains_and_runtime_leaks() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn relay(T:Type,U:defer T,value:defer U)->defer U {value}";
    for source in [
        format!("{prefix} const BAD=relay(bool,true,true);"),
        format!("{prefix} const BAD=relay(Type,bool,7);"),
        format!("{prefix} fn bad(flag:bool)->bool {{relay(Type,bool,flag)}}"),
        "typealias Type=.Type'builtin;fn bad(T:Type,value:defer T)->defer T {value}".into(),
    ] {
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn ordinary_comptime_type_inputs_form_structural_function_domains() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn keep(T:Type,callback:defer fn(T)->T)->defer fn(T)->T {callback}
        fn identity(value:bool)->bool {value}
        const CALLBACK=keep(bool,identity);typealias Callback=CALLBACK'type;
        fn invoke(callback:Callback)->bool {callback(true)}
        fn run()->bool {invoke(identity)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn ordinary_comptime_structural_domains_support_nested_and_pending_types() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn keep(T:Type,U:defer T,callback:defer fn(fn(U)->U)->U)->defer fn(fn(U)->U)->U {callback}
        comptime fn pointer(T:Type,callback:defer fn(*T)->*T)->defer fn(*T)->*T {callback}
        fn higher(callback:fn(bool)->bool)->bool {callback(true)}
        fn identity(value:bool)->bool {value}
        fn pointer_identity(value:*bool)->*bool {value}
        const HIGHER=keep(Type,bool,higher);const POINTER=pointer(bool,pointer_identity);
        typealias Higher=HIGHER'type;typealias Pointer=POINTER'type;
        fn invoke(callback:Higher)->bool {callback(identity)}
        fn accept_pointer(callback:Pointer)->bool {true}
        fn run()->bool {invoke(higher) and accept_pointer(pointer_identity)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn ordinary_comptime_structural_domains_reject_mismatched_and_non_type_inputs() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn keep(T:Type,U:defer T,callback:defer fn(U)->U)->defer fn(U)->U {callback}
        fn identity(value:bool)->bool {value}
        fn wrong(value:Byte)->Byte {value}";
    for arguments in [
        "Type,bool,wrong",
        "bool,true,identity",
        "Type,Byte,identity",
    ] {
        let failure = compile_with_protocols(
            &format!("{prefix} const BAD=keep({arguments});"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            failure.contains("elaboration failed"),
            "{arguments}: {failure}"
        );
    }
}

#[test]
fn ordinary_comptime_inputs_enable_dependent_clause_inference() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn keep(U:Type,callback:defer fn(U)->T)->defer fn(U)->T where T {callback}
        fn identity(value:bool)->bool {value}
        const CALLBACK=keep(bool,identity);typealias Callback=CALLBACK'type;
        fn invoke(callback:Callback)->bool {callback(true)}
        fn run()->bool {invoke(identity)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn ordinary_comptime_clause_inference_preserves_prefix_failures() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn keep(tag:bool,U:Type,callback:defer fn(U)->T)->defer fn(U)->T where T {callback}
        comptime fn checked(value:bool)->bool {assumes value;value}
        comptime fn Spin()->Type {while true {} bool}
        fn identity(value:bool)->bool {value}";
    let failure = compile_with_protocols(
        &format!("{prefix} const BAD=keep(checked(false),Spin(),identity);"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    let package = compile(&format!(
        "{prefix} const GOOD=keep(checked(true),bool,identity);typealias Callback=GOOD'type;fn invoke(callback:Callback)->bool {{callback(true)}} fn run()->bool {{invoke(identity)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_comptime_clause_inference_can_postpone_defaults() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn keep(.U:Type=bool,callback:fn(bool)->T)->defer fn(U)->T where T {callback}
        fn identity(value:bool)->bool {value}
        const CALLBACK=keep(identity);typealias Callback=CALLBACK'type;
        fn invoke(callback:Callback)->bool {callback(true)}
        fn run()->bool {invoke(identity)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_comptime_clause_inference_keeps_result_types_distinct() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn keep(U:Type,callback:defer fn(U)->T)->defer fn(U)->T where T {callback}
        fn identity(value:bool)->bool {value}
        fn byte(value:bool)->Byte {7}
        fn wrong(value:Byte)->Byte {value}";
    let package = compile(&format!(
        "{prefix}
        const BOOLEAN=keep(bool,identity);const NUMBER=keep(bool,byte);
        typealias Boolean=BOOLEAN'type;typealias Number=NUMBER'type;
        fn boolean(callback:Boolean)->bool {{callback(true)}}
        fn numeric(callback:Number)->Byte {{callback(true)}}
        fn run_bool()->bool {{boolean(identity)}} fn run_byte()->Byte {{numeric(byte)}}"
    ));
    for (entry, value) in [
        ("run_bool", Value::Bool(true)),
        ("run_byte", Value::UnsignedInteger(7)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            value
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
    assert!(
        compile_with_protocols(
            &format!("{prefix} const BAD=keep(bool,wrong);"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn ordinary_comptime_type_inputs_form_bound_tuple_domains() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {List<T>}
        pure comptime fn Tuple<...Es:List<Type>>->Type {Tuple<...Es>}
        comptime fn pair(T:Type,...values:defer Tuple<T,T>)->defer Tuple<T,T> {values}
        const PAIR=pair(bool,true,false);typealias Pair=PAIR'type;
        fn run()->Pair {(true,false)}";
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::List"),
        tuple: Some("input::Tuple"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Tuple(vec![Value::Bool(true), Value::Bool(false)])
    );
    let pair = package
        .functions
        .iter()
        .find(|function| function.name == "pair")
        .unwrap();
    let nir::Term::Function { result, .. } = &pair.signature else {
        panic!("signature")
    };
    assert!(
        matches!(result.as_ref(),nir::Term::NormalForm {head,..} if *head==id(&package,"Tuple"))
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn ordinary_comptime_tuple_domains_resume_nested_pending_inputs() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn nested(T:Type,U:defer T,value:defer Tuple<U,Tuple<U,U>>)->defer Tuple<U,Tuple<U,U>> {value}";
    let package = execution_support::compile_with_tuple(&format!(
        "{prefix}
        const VALUE=nested(Type,bool,(true,(false,true)));typealias Nested=VALUE'type;
        fn run()->Nested {{(true,(false,true))}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Tuple(vec![
            Value::Bool(true),
            Value::Tuple(vec![Value::Bool(false), Value::Bool(true)])
        ])
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for arguments in [
        "Type,bool,(true,(true,7))",
        "Type,bool,(true,(true,))",
        "bool,true,(true,(true,true))",
    ] {
        let failure = execution_support::compile_with_tuple_protocols(
            &format!("{prefix} const BAD=nested({arguments});"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            failure.contains("elaboration failed"),
            "{arguments}: {failure}"
        );
    }
}

#[test]
fn ordinary_comptime_prefix_preserves_computed_function_values() {
    let package = compile(
        "typealias Type=.Type'builtin;
        fn identity(value:bool)->bool {value}
        comptime fn produce()->fn(bool)->bool {identity}
        comptime fn keep(.T:Type=bool,callback:fn(bool)->bool)->defer fn(T)->T {callback}
        const CALLBACK=keep(produce());typealias Callback=CALLBACK'type;
        fn invoke(callback:Callback)->bool {callback(true)}
        fn run()->bool {invoke(identity)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn ordinary_comptime_function_producers_fail_before_later_defaults() {
    let prefix = "typealias Type=.Type'builtin;
        fn identity(value:bool)->bool {value}
        comptime fn produce(ok:bool)->fn(bool)->bool {assumes ok;identity}
        comptime fn Spin()->Type {while true {} bool}
        comptime fn keep(.T:Type=defer Spin(),callback:fn(bool)->bool)->defer fn(T)->T {callback}";
    let failure = compile_with_protocols(
        &format!("{prefix} const BAD=keep(produce(false));"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    let package = compile(&format!(
        "{prefix} const GOOD=keep(produce(true),.T=bool);typealias Callback=GOOD'type;fn invoke(callback:Callback)->bool {{callback(true)}} fn run()->bool {{invoke(identity)}}"
    ));
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn ordinary_comptime_function_producers_preserve_generic_instances() {
    let package = compile(
        "typealias Type=.Type'builtin;
        fn identity(value:T)->T where T {value}
        comptime fn produce()->fn(bool)->bool {identity<bool>}
        comptime fn keep(.T:Type=bool,callback:fn(bool)->bool)->defer fn(T)->T {callback}
        const CALLBACK=keep(produce());typealias Callback=CALLBACK'type;
        fn invoke(callback:Callback)->bool {callback(true)}
        fn run()->bool {invoke(identity<bool>)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn ordinary_comptime_collections_deliver_computed_function_values() {
    let prefix="typealias Type=.Type'builtin;
        fn identity(value:bool)->bool {value}
        comptime fn produce(ok:bool)->fn(bool)->bool {assumes ok;identity}
        comptime fn choose(.T:Type=bool,...callbacks:Tuple<fn(bool)->bool,fn(bool)->bool>)->defer fn(T)->T {let (first,second)=callbacks;first}";
    for args in [
        "produce(true),produce(true)",
        "produce(true),.T=bool,produce(true)",
    ] {
        let package = execution_support::compile_with_tuple(&format!(
            "{prefix} const CALLBACK=choose({args});typealias Callback=CALLBACK'type;fn invoke(callback:Callback)->bool {{callback(true)}} fn run()->bool {{invoke(identity)}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
    let failure=execution_support::compile_with_tuple_protocols(&format!("{prefix} comptime fn Spin()->Type {{while true {{}} bool}} const BAD=choose(produce(false),.T=Spin(),produce(true));"),&Default::default()).unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
}

#[test]
fn ordinary_comptime_mixed_function_collections_survive_default_chains() {
    let source="typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {List<T>}
        pure comptime fn Tuple<...Es:List<Type>>->Type {Tuple<...Es>}
        fn identity(value:bool)->bool {value}
        comptime fn produce()->fn(bool)->bool {identity}
        comptime fn choose(.T:Type=bool,.U:Type=T,...values:Tuple<bool,fn(bool)->bool>)->defer fn(U)->U {let (flag,callback)=values;callback}
        const CALLBACK=choose(true,produce());typealias Callback=CALLBACK'type;
        fn invoke(callback:Callback)->bool {callback(true)}
        fn run()->bool {invoke(identity)}";
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::List"),
        tuple: Some("input::Tuple"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn ordinary_comptime_function_packets_supply_type_dependencies_and_values() {
    let package = execution_support::compile_with_tuple(
        "typealias Type=.Type'builtin;
        fn identity(value:bool)->bool {value}
        comptime fn packet()->Tuple<Type,fn(bool)->bool> {(bool,identity)}
        comptime fn keep(T:Type,callback:defer fn(T)->T)->defer fn(T)->T {callback}
        const CALLBACK=keep(...packet());typealias Callback=CALLBACK'type;
        fn invoke(callback:Callback)->bool {callback(true)}
        fn run()->bool {invoke(identity)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn staged_function_packets_deliver_the_selected_runtime_function() {
    let package = execution_support::compile_with_tuple(
        "typealias Type=.Type'builtin;
        fn negate(value:bool)->bool {not value}
        comptime fn packet()->Tuple<Type,fn(bool)->bool> {(bool,negate)}
        fn keep(comptime T:Type,callback:defer fn(T)->T)->defer fn(T)->T {callback}
        fn run()->bool {let callback=keep(...packet());callback(true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn ordinary_comptime_function_packets_precede_default_evaluation() {
    let prefix = "typealias Type=.Type'builtin;
        fn identity(value:bool)->bool {value}
        comptime fn packet(ok:bool)->Tuple<fn(bool)->bool> {assumes ok;(identity,)}
        comptime fn Spin()->Type {while true {} bool}
        comptime fn keep(.T:Type=defer Spin(),callback:fn(bool)->bool)->defer fn(T)->T {callback}";
    let failure = execution_support::compile_with_tuple_protocols(
        &format!("{prefix} const BAD=keep(...packet(false));"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    let package = execution_support::compile_with_tuple(&format!(
        "{prefix} const GOOD=keep(...packet(true),.T=bool);typealias Callback=GOOD'type;fn invoke(callback:Callback)->bool {{callback(true)}} fn run()->bool {{invoke(identity)}}"
    ));
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn staged_packets_preserve_nested_function_delivery() {
    let source="typealias Type=.Type'builtin;
        fn negate(value:bool)->bool {not value}
        comptime fn packet()->Tuple<Type,Tuple<fn(bool)->bool,bool>> {(bool,(negate,true))}
        fn keep(comptime T:Type,data:defer Tuple<fn(T)->T,bool>)->defer fn(T)->T {let (callback,flag)=data;callback}
        fn run()->bool {let callback=keep(...packet());callback(true)}";
    for bound in [false, true] {
        let selectors = if bound {
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input::List"),
                tuple: Some("input::Tuple"),
                ..Default::default()
            }
        } else {
            Default::default()
        };
        let source = if bound {
            format!(
                "pure comptime fn List<T>->Type {{List<T>}} pure comptime fn Tuple<...Es:List<Type>>->Type {{Tuple<...Es>}} {source}"
            )
        } else {
            format!(
                "pure comptime fn List<T>->Type {{List<T>}} pure comptime fn Tuple<...Es:List<Type>>->Type {{Tuple<...Es>}} {source}"
            )
        };
        let package = compile_with_source_bindings(&source, &Default::default(), selectors);
        if !bound {
            let error = package.unwrap_err();
            assert!(
                error.contains("bound") || error.contains("unresolved"),
                "{error}"
            );
            continue;
        }
        let package = package.unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(false)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn staged_packets_deliver_nominal_function_fields() {
    let package = execution_support::compile_with_tuple(
        "typealias Type=.Type'builtin;
        struct Holder {callback:fn(bool)->bool}
        fn negate(value:bool)->bool {not value}
        comptime fn packet()->Tuple<Type,Holder> {(bool,Holder {callback:negate})}
        fn invoke(comptime T:Type,holder:Holder)->bool {holder.callback(true)}
        fn run()->bool {invoke(...packet())}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn staged_packets_deliver_enum_and_generic_nominal_function_fields() {
    for (variant, expected) in [
        ("Choice.some(Holder<bool> {callback:negate})", false),
        ("Choice.none", true),
    ] {
        let package=execution_support::compile_with_tuple(&format!("typealias Type=.Type'builtin;
            struct Holder where T {{callback:fn(T)->T}}
            enum Choice {{none,some(Holder<bool>)}}
            fn negate(value:bool)->bool {{not value}}
            comptime fn packet()->Tuple<Type,Choice> {{(bool,{variant})}}
            fn invoke(comptime T:Type,choice:Choice)->bool {{choice match {{.none=>true,.some(holder)=>holder.callback(true)}}}}
            fn run()->bool {{invoke(...packet())}}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn comptime_packets_preserve_shared_closure_captures() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;typealias Callback=%{flag:bool} comptime fn()->bool;
        comptime fn make()->Callback {var flag=false;|| {flag=not flag;flag}}
        comptime fn packet()->Tuple<Callback,Callback> {let action=make();(action,action)}
        comptime fn Selected(T:Type)->Type {bool}
        comptime fn invoke(.T:Type=bool,left:Callback,right:Callback)->defer Selected(T) {left();right()}
        comptime fn checked(value:bool)->bool {assumes not value;true}
        const RESULT=checked(invoke(...packet()));typealias Result=RESULT'type;
        fn run()->Result {true}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn comptime_packets_preserve_nested_shared_closure_graphs_across_defaults() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        typealias Inner=%{flag:bool} comptime fn()->bool;
        typealias Outer=%{inner:Inner} comptime fn()->bool;
        comptime fn make()->Inner {var flag=false;|| {flag=not flag;flag}}
        comptime fn wrap(inner:Inner)->Outer {|| inner()}
        comptime fn packet()->Tuple<Outer,Outer> {let inner=make();(wrap(inner),wrap(inner))}
        comptime fn Selected(T:Type)->Type {bool}
        comptime fn invoke(.T:Type=bool,.U:Type=T,left:Outer,right:Outer)->defer Selected(U) {left();right()}
        comptime fn checked(value:bool)->bool {assumes not value;true}
        const RESULT=checked(invoke(...packet()));typealias Result=RESULT'type;
        fn run()->Result {true}");
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn comptime_closure_construction_does_not_erase_runtime_phase() {
    let failure = compile_with_protocols(
        "typealias Callback=%{flag:bool} fn()->bool;
        comptime fn make()->Callback {let flag=true;|| flag}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("different execution phase"), "{failure}");
}

#[test]
fn staged_packets_preserve_trait_method_function_identity() {
    let package = execution_support::compile_with_tuple(
        "typealias Type=.Type'builtin;
        trait Mapper {fn action(value:bool)->bool {not value}}
        struct Holder {} impl Mapper for Holder {}
        comptime fn packet()->Tuple<Type,fn(bool)->bool> {(bool,Holder.as(Mapper).action)}
        fn invoke(comptime T:Type,callback:fn(bool)->bool)->bool {callback(true)}
        fn run()->bool {invoke(...packet())}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn staged_method_packets_preserve_generic_and_inherited_origins() {
    for (definitions, access) in [
        (
            "trait Mapper {fn keep(value:T)->T where T {value}} struct Holder {} impl Mapper for Holder {}",
            "Holder.as(Mapper).keep<bool>",
        ),
        (
            "trait Mapper where A {fn keep(value:A)->A;} trait Child :- Mapper<A> where A {} struct Holder where X {} impl Mapper<X> for Holder<X> where X {fn keep(value:X)->X {value}} impl Child<X> for Holder<X> where X {}",
            "Holder<bool>.as(Child<bool>).keep",
        ),
    ] {
        let package = execution_support::compile_with_tuple(&format!(
            "typealias Type=.Type'builtin;{definitions}
            comptime fn packet()->Tuple<Type,fn(bool)->bool> {{(bool,{access})}}
            fn invoke(comptime T:Type,callback:fn(bool)->bool)->bool {{callback(true)}}
            fn run()->bool {{invoke(...packet())}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn comptime_method_collections_preserve_selection_across_default_chains() {
    let prefix = "typealias Type=.Type'builtin;
        trait Flip {fn action(value:bool)->bool {not value}}
        trait Keep {fn action(value:bool)->bool {value}}
        struct Holder {} impl Flip for Holder {} impl Keep for Holder {}
        comptime fn produce(ok:bool)->fn(bool)->bool {assumes ok;Holder.as(Flip).action}
        comptime fn choose(.T:Type=bool,.U:Type=T,...callbacks:Tuple<fn(bool)->bool,fn(bool)->bool>)->defer fn(U)->U {let (first,second)=callbacks;first}
        fn invoke(comptime T:Type,callback:fn(bool)->bool)->bool {callback(true)}";
    for (arguments, expected) in [
        ("Holder.as(Flip).action,Holder.as(Keep).action", false),
        ("Holder.as(Keep).action,Holder.as(Flip).action", true),
        ("produce(true),Holder.as(Keep).action", false),
        ("produce(true),.T=bool,Holder.as(Keep).action", false),
    ] {
        let package = execution_support::compile_with_tuple(&format!(
            "{prefix}
            comptime fn packet()->Tuple<Type,fn(bool)->bool> {{(bool,choose({arguments}))}}
            fn run()->bool {{invoke(...packet())}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(expected),
            "{arguments}"
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
    let failure = execution_support::compile_with_tuple_protocols(
        &format!(
            "{prefix}
        comptime fn Spin()->Type {{while true {{}} bool}}
        const BAD=choose(produce(false),.T=Spin(),Holder.as(Keep).action);"
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
}

#[test]
fn comptime_aggregate_inputs_deliver_functions_across_defaults() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        fn negate(value:bool)->bool {not value}
        comptime fn produce()->Tuple<bool,fn(bool)->bool> {(true,negate)}
        comptime fn choose(.T:Type=bool,data:Tuple<bool,fn(bool)->bool>)->defer fn(T)->T {let (flag,callback)=data;callback}
        comptime fn packet()->Tuple<Type,fn(bool)->bool> {(bool,choose(produce()))}
        fn invoke(comptime T:Type,callback:fn(bool)->bool)->bool {callback(true)}
        fn run()->bool {invoke(...packet())}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn comptime_nominal_delivery_inputs_preserve_functions_and_failure_order() {
    let prefix = "typealias Type=.Type'builtin;
        struct Holder {callback:fn(bool)->bool}
        fn negate(value:bool)->bool {not value}
        comptime fn produce(ok:bool)->Holder {assumes ok;Holder {callback:negate}}
        comptime fn choose(.T:Type=bool,.U:Type=T,data:Holder)->defer fn(U)->U {data.callback}
        fn invoke(comptime T:Type,callback:fn(bool)->bool)->bool {callback(true)}";
    let package = execution_support::compile_with_tuple(&format!(
        "{prefix}
        comptime fn packet()->Tuple<Type,fn(bool)->bool> {{(bool,choose(produce(true)))}}
        fn run()->bool {{invoke(...packet())}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let failure = execution_support::compile_with_tuple_protocols(
        &format!(
            "{prefix}
        comptime fn Spin()->Type {{while true {{}} bool}}
        const BAD=choose(produce(false),.T=Spin());"
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
}

#[test]
fn comptime_aggregate_packet_fields_preserve_shared_capture_graphs() {
    let source="typealias Type=.Type'builtin;
        typealias Callback=%{flag:bool} comptime fn()->bool;
        struct Holder {callback:Callback}
        comptime fn make()->Callback {var flag=false;|| {flag=not flag;flag}}
        comptime fn packet()->Tuple<Holder,Holder> {let action=make();(Holder {callback:action},Holder {callback:action})}
        comptime fn Selected(T:Type)->Type {bool}
        comptime fn invoke(.T:Type=bool,.U:Type=T,left:Holder,right:Holder)->defer Selected(U) {left.callback();right.callback()}
        comptime fn checked(value:bool)->bool {assumes not value;true}
        const RESULT=checked(invoke(...packet()));typealias Result=RESULT'type;
        fn run()->Result {true}";
    let tuple_source = source
        .replace(
            "struct Holder {callback:Callback}",
            "typealias Holder=Tuple<Callback>;",
        )
        .replace("Holder {callback:action}", "(action,)")
        .replace(
            "left.callback();right.callback()",
            "let (first,)=left;let (second,)=right;first();second()",
        );
    for source in [source, tuple_source.as_str()] {
        let package = execution_support::compile_with_tuple(source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn ordinary_comptime_type_inputs_form_normal_head_annotations() {
    let package=compile("typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        fn identity(value:Identity<bool>)->Identity<bool> {value}
        comptime fn keep(T:Type,callback:defer fn(Identity<T>)->Identity<T>)->defer fn(Identity<T>)->Identity<T> {callback}
        const CALLBACK=keep(bool,identity);
        typealias Callback=CALLBACK'type;
        fn run(callback:Callback,value:Identity<bool>)->Identity<bool> {callback(value)}");
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn suspended_normal_heads_keep_identity_and_application_requirements() {
    let prefix="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn keep(T:Type,callback:defer fn(Identity<T>)->Identity<T>)->defer fn(Identity<T>)->Identity<T> {callback}";
    for callback in [
        "fn wrong(value:bool)->bool {value}",
        "fn wrong(value:Identity<Byte>)->Identity<Byte> {value}",
    ] {
        let failure = compile_with_protocols(
            &format!("{prefix} {callback} const BAD=keep(bool,wrong);"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
    let bounded="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn Restricted<T>->Type where requires T == bool {T}
        comptime fn keep_restricted(T:Type,callback:defer fn(Restricted<T>)->Restricted<T>)->defer fn(Restricted<T>)->Restricted<T> {callback}";
    let package = compile(&format!(
        "{bounded}
        fn identity(value:Restricted<bool>)->Restricted<bool> {{value}}
        const CALLBACK=keep_restricted(bool,identity);typealias Callback=CALLBACK'type;
        fn run(callback:Callback,value:Restricted<bool>)->Restricted<bool> {{callback(value)}}"
    ));
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let failure = compile_with_protocols(
        &format!(
            "{bounded}
        fn identity(value:Restricted<bool>)->Restricted<bool> {{value}}
        const BAD=keep_restricted(Byte,identity);"
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
}

#[test]
fn ordinary_comptime_inputs_form_nominal_and_alias_annotations() {
    for (definition, input, result) in [
        (
            "struct Holder where T {value:T}",
            "Holder<bool> {value:true}",
            "result.value",
        ),
        (
            "newtype Holder<T> = T;",
            "true.as(Holder<bool>)",
            "result.as(bool)",
        ),
        (
            "enum Holder where T {some(T),none}",
            "Holder<bool>.some(true)",
            "result match {.some(value)=>value,.none=>false}",
        ),
        (
            "struct Storage where T {value:T} typealias Holder<T> = Storage<T>;",
            "Storage<bool> {value:true}",
            "result.value",
        ),
    ] {
        let source=format!("typealias Type=.Type'builtin;{definition}
            fn identity(value:Holder<bool>)->Holder<bool> {{value}}
            comptime fn keep(T:Type,callback:defer fn(Holder<T>)->Holder<T>)->defer fn(Holder<T>)->Holder<T> {{callback}}
            comptime fn packet()->Tuple<Type,fn(Holder<bool>)->Holder<bool>> {{(bool,keep(bool,identity))}}
            fn invoke(comptime T:Type,callback:fn(Holder<bool>)->Holder<bool>)->bool {{let result=callback({input});{result}}}
            fn run()->bool {{invoke(...packet())}}");
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|failure| panic!("{source}\n{failure}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn suspended_nominal_annotations_enforce_instance_identity_and_bounds() {
    let prefix="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        trait Allowed {} impl Allowed for bool {}
        struct Holder where T:-Allowed {value:T}
        comptime fn keep(T:Type,callback:defer fn(Holder<T>)->Holder<T>)->defer fn(Holder<T>)->Holder<T> {callback}
        fn identity(value:Holder<bool>)->Holder<bool> {value}";
    let package = compile(&format!(
        "{prefix} const GOOD=keep(bool,identity);typealias Callback=GOOD'type;
        fn run(callback:Callback,value:Holder<bool>)->Holder<bool> {{callback(value)}}"
    ));
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for extra in [
        "const BAD=keep(Byte,identity);",
        "fn wrong(value:bool)->bool {value} const BAD=keep(bool,wrong);",
    ] {
        let failure =
            compile_with_protocols(&format!("{prefix} {extra}"), &Default::default()).unwrap_err();
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
}

#[test]
fn suspended_normal_applications_route_named_types_and_defaults() {
    for application in ["Pair<T>", "Pair<T,.U=T>", "Pair<.U=T,T>", "Pair<.V=T,T>"] {
        let source=format!("typealias Type=.Type'builtin;
            pure comptime fn Pair<A,.U:Type=A,.V:Type=U>->Type {{A}}
            fn identity(value:Pair<bool>)->Pair<bool> {{value}}
            comptime fn keep(T:Type,callback:defer fn({application})->{application})->defer fn({application})->{application} {{callback}}
            const CALLBACK=keep(bool,identity);typealias Callback=CALLBACK'type;
            fn run(callback:Callback,value:Pair<bool>)->Pair<bool> {{callback(value)}}");
        let package = compile(&source);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn suspended_normal_type_routes_reject_invalid_labels_and_missing_inputs() {
    for (application, diagnostic) in [
        ("Pair<T,.U=T,.U=T>", "duplicate type argument"),
        ("Pair<.unknown=T,T>", "unknown named type argument"),
        ("Pair<.U=T>", "missing required type argument"),
        ("Pair<T,T>", "type argument count mismatch"),
    ] {
        let failure=compile_with_protocols(&format!("typealias Type=.Type'builtin;
            pure comptime fn Pair<A,.U:Type=A>->Type {{A}}
            comptime fn keep(T:Type,callback:defer fn({application})->{application})->defer fn({application})->{application} {{callback}}"),&Default::default()).unwrap_err();
        assert!(failure.contains(diagnostic), "{application}: {failure}");
    }
}

#[test]
fn suspended_type_list_applications_collect_type_inputs() {
    for (application, concrete) in [
        ("Pack<T>", "Pack<bool>"),
        ("Pack<T,T>", "Pack<bool,bool>"),
        ("Pack<T,bool,T>", "Pack<bool,bool,bool>"),
        ("Pack<...(T,),bool,T>", "Pack<bool,bool,bool>"),
    ] {
        let source=format!("typealias Type=.Type'builtin;
            pure comptime fn List<T>->Type {{List<T>}}
            pure comptime fn Pack<A,...Es:List<Type>>->Type {{Pack<A,...Es>}}
            fn identity(value:{concrete})->{concrete} {{value}}
            comptime fn keep(T:Type,callback:defer fn({application})->{application})->defer fn({application})->{application} {{callback}}
            const CALLBACK=keep(bool,identity);typealias Callback=CALLBACK'type;
            fn run(callback:Callback,value:{concrete})->{concrete} {{callback(value)}}");
        let package = compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input::List"),
                ..Default::default()
            },
        )
        .unwrap();
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn suspended_type_collections_reject_tuple_tails_and_different_elements() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn List<T>->Type {List<T>}
        pure comptime fn Pack<A,...Es:List<Type>>->Type {Pack<A,...Es>}";
    for suffix in [
        "comptime fn keep(T:Type,callback:defer fn(Pack<T,...(T,)>) -> Pack<T>)->bool {true}",
        "comptime fn keep(T:Type,callback:defer fn(Pack<...(T,T)>) -> Pack<T>)->bool {true}",
        "comptime fn keep(T:Type,callback:defer fn(Pack<T,T>)->Pack<T,T>)->defer fn(Pack<T,T>)->Pack<T,T> {callback}
         fn wrong(value:Pack<bool,Byte>)->Pack<bool,Byte> {value} const BAD=keep(bool,wrong);",
    ] {
        let failure=compile_with_source_bindings(&format!("{prefix}{suffix}"),&Default::default(),luna::protocols::type_constructors::Selectors {meta_list:Some("input::List"),..Default::default()}).unwrap_err();
        assert!(failure.contains("elaboration failed"),"{failure}");
    }
}

#[test]
fn suspended_normal_applications_accept_whole_and_spread_type_lists() {
    for (application, concrete) in [
        ("Pack<T,.Es=types>", "Pack<bool,bool>"),
        ("Pack<.Es=types,T>", "Pack<bool,bool>"),
        ("Pack<T,...types>", "Pack<bool,bool>"),
        ("Pack<bool,...types>", "Pack<bool,bool>"),
        ("Pack<T,...[],...types>", "Pack<bool,bool>"),
        ("Pack<T,bool,...types>", "Pack<bool,bool,bool>"),
        ("Pack<T,.Es=[T]>", "Pack<bool,bool>"),
    ] {
        let source=format!("typealias Type=.Type'builtin;
            pure comptime fn List<T>->Type {{List<T>}}
            pure comptime fn Pack<A,...Es:List<Type>>->Type {{Pack<A,...Es>}}
            fn identity(value:{concrete})->{concrete} {{value}}
            comptime fn keep(T:Type,types:List<Type>,callback:defer fn({application})->{application})->defer fn({application})->{application} {{callback}}
            const CALLBACK=keep(bool,[bool],identity);typealias Callback=CALLBACK'type;
            fn run(callback:Callback,value:{concrete})->{concrete} {{callback(value)}}");
        let package = compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input::List"),
                ..Default::default()
            },
        )
        .unwrap();
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn suspended_whole_type_lists_reject_mixed_supplies_and_wrong_domains() {
    for (application, diagnostic) in [
        (
            "Pack<T,T,.Es=types>",
            "cannot combine a complete type-list argument",
        ),
        (
            "Pack<T,.Es=types,.Es=types>",
            "duplicate type-list argument",
        ),
        (
            "Pack<T,.Unknown=types>",
            "unknown named normal-form argument",
        ),
        ("Pack<T,...T>", "elaboration failed"),
        ("Pack<T,.Es=(T,)>", "elaboration failed"),
    ] {
        let source=format!("typealias Type=.Type'builtin;
            pure comptime fn List<T>->Type {{List<T>}}
            pure comptime fn Pack<A,...Es:List<Type>>->Type {{Pack<A,...Es>}}
            comptime fn keep(T:Type,types:List<Type>,callback:defer fn({application})->{application})->bool {{true}}");
        let failure = compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input::List"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(failure.contains(diagnostic), "{application}: {failure}");
    }
}

#[test]
fn suspended_type_applications_invoke_list_producers() {
    for application in [
        "Pack<T,...types(T)>",
        "Pack<T,.Es=types(T)>",
        "Pack<.Es=types(T),T>",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            pure comptime fn List<T>->Type {{List<T>}}
            pure comptime fn Pack<A,...Es:List<Type>>->Type {{Pack<A,...Es>}}
            comptime fn types(T:Type)->List<Type> {{[T,bool]}}
            fn identity(value:Pack<bool,bool,bool>)->Pack<bool,bool,bool> {{value}}
            comptime fn keep(T:Type,callback:defer fn({application})->{application})->defer fn({application})->{application} {{callback}}
            const CALLBACK=keep(bool,identity);typealias Callback=CALLBACK'type;
            fn run(callback:Callback,value:Pack<bool,bool,bool>)->Pack<bool,bool,bool> {{callback(value)}}");
        let package = compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input::List"),
                ..Default::default()
            },
        )
        .unwrap();
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn suspended_list_producers_preserve_phase_and_invocation_failures() {
    let prefix = "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {List<T>}
        pure comptime fn Pack<A,...Es:List<Type>>->Type {Pack<A,...Es>}";
    let consumer =
        "comptime fn keep(T:Type,callback:defer fn(Pack<T,...types(T)>)->Pack<T>)->bool {true}";
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::List"),
        ..Default::default()
    };
    let dormant = format!(
        "{prefix} comptime fn types(T:Type)->List<Type> {{while true {{}} [T]}} {consumer}"
    );
    compile_with_source_bindings(&dormant, &Default::default(), selectors.clone()).unwrap();
    let runtime = format!("{prefix} fn types(T:Type)->List<Type> {{[bool]}} {consumer}");
    let failure =
        compile_with_source_bindings(&runtime, &Default::default(), selectors.clone()).unwrap_err();
    assert!(
        failure.contains("unavailable in this execution phase"),
        "{failure}"
    );
    let failing = format!(
        "{prefix} comptime fn types(T:Type)->List<Type> {{assumes false;[T]}} {consumer}
        fn callback(value:Pack<bool,bool>)->Pack<bool> {{while true {{}}}}
        const BAD=keep(bool,callback);"
    );
    let failure =
        compile_with_source_bindings(&failing, &Default::default(), selectors).unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
}

#[test]
fn ordinary_comptime_value_inputs_form_indexed_normal_annotations() {
    for (domain, argument) in [("bool", "true"), ("Byte", "9")] {
        let source=format!("typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
            pure comptime fn Indexed<index:{domain}>->Type {{bool}}
            fn identity(value:Indexed<{argument}>)->Indexed<{argument}> {{value}}
            comptime fn keep(index:{domain},callback:defer fn(Indexed<index>)->Indexed<index>)->defer fn(Indexed<index>)->Indexed<index> {{callback}}
            const CALLBACK=keep({argument},identity);typealias Callback=CALLBACK'type;
            fn run(callback:Callback,value:Indexed<{argument}>)->Indexed<{argument}> {{callback(value)}}");
        let package = compile(&source);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn suspended_value_indices_preserve_defaults_identity_and_domains() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn Indexed<T,flag:bool,.copy:bool=flag>->Type {T}
        fn identity(value:Indexed<bool,true>)->Indexed<bool,true> {value}";
    for application in [
        "Indexed<T,flag>",
        "Indexed<T,flag,.copy=flag>",
        "Indexed<.copy=flag,T,flag>",
    ] {
        let source=format!("{prefix}
            comptime fn keep(T:Type,flag:bool,callback:defer fn({application})->{application})->defer fn({application})->{application} {{callback}}
            const CALLBACK=keep(bool,true,identity);typealias Callback=CALLBACK'type;
            fn run(callback:Callback,value:Indexed<bool,true>)->Indexed<bool,true> {{callback(value)}}");
        let package = compile(&source);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let failure = compile_with_protocols(
            &source.replace("keep(bool,true,identity)", "keep(bool,false,identity)"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
    for signature in [
        "comptime fn keep(T:Type,flag:Byte,callback:defer fn(Indexed<T,flag>)->Indexed<T,flag>)->bool {true}",
        "fn keep(T:Type,flag:bool,callback:defer fn(Indexed<T,flag>)->Indexed<T,flag>)->bool {true}",
    ] {
        let failure = compile_with_protocols(&format!("{prefix}{signature}"), &Default::default())
            .unwrap_err();
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
}

#[test]
fn suspended_type_collections_accept_fixed_value_prefixes() {
    for application in [
        "Pack<flag,T>",
        "Pack<flag,...types>",
        "Pack<flag,.Es=types>",
        "Pack<.Es=types,flag>",
        "Pack<...(flag,),T>",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            pure comptime fn List<T>->Type {{List<T>}}
            pure comptime fn Pack<flag:bool,...Es:List<Type>>->Type {{bool}}
            fn identity(value:Pack<true,bool>)->Pack<true,bool> {{value}}
            comptime fn keep(flag:bool,T:Type,types:List<Type>,callback:defer fn({application})->{application})->defer fn({application})->{application} {{callback}}
            const CALLBACK=keep(true,bool,[bool],identity);typealias Callback=CALLBACK'type;
            fn run(callback:Callback,value:Pack<true,bool>)->Pack<true,bool> {{callback(value)}}");
        let selectors = luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            ..Default::default()
        };
        let package =
            compile_with_source_bindings(&source, &Default::default(), selectors.clone()).unwrap();
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let failure = compile_with_source_bindings(
            &source.replace(
                "keep(true,bool,[bool],identity)",
                "keep(false,bool,[bool],identity)",
            ),
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
}

#[test]
fn suspended_normal_inputs_resume_dependent_value_domains() {
    let package=compile("typealias Type=.Type'builtin;
        pure comptime fn Tagged<T,value:T>->Type {T}
        fn identity(value:Tagged<bool,true>)->Tagged<bool,true> {value}
        comptime fn keep(T:Type,value:defer T,callback:defer fn(Tagged<T,value>)->Tagged<T,value>)->defer fn(Tagged<T,value>)->Tagged<T,value> {callback}
        const CALLBACK=keep(bool,true,identity);typealias Callback=CALLBACK'type;
        fn run(callback:Callback,value:Tagged<bool,true>)->Tagged<bool,true> {callback(value)}");
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn suspended_dependent_head_domains_recheck_values_and_known_evidence() {
    let prefix="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn Tagged<T,value:T,.copy:T=value>->Type {T}
        fn identity(value:Tagged<bool,true>)->Tagged<bool,true> {value}
        comptime fn keep(T:Type,value:defer T,callback:defer fn(Tagged<T,value>)->Tagged<T,value>)->defer fn(Tagged<T,value>)->Tagged<T,value> {callback}";
    let package = compile(&format!(
        "{prefix} const GOOD=keep(bool,true,identity);typealias Callback=GOOD'type;
        fn run(callback:Callback,value:Tagged<bool,true>)->Tagged<bool,true> {{callback(value)}}"
    ));
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for call in [
        "keep(bool,false,identity)",
        "keep(bool,9,identity)",
        "keep(Byte,9,identity)",
    ] {
        let failure =
            compile_with_protocols(&format!("{prefix} const BAD={call};"), &Default::default())
                .unwrap_err();
        assert!(failure.contains("elaboration failed"), "{call}: {failure}");
    }
    for definition in [
        "comptime fn bad(value:Byte,callback:defer fn(Tagged<bool,value>)->Tagged<bool,value>)->bool {true}",
        "comptime fn bad(T:Type,value:defer T,callback:defer fn(Tagged<T,value(true)>)->Tagged<T,value(true)>)->bool {true}",
    ] {
        let failure = compile_with_protocols(&format!("{prefix}{definition}"), &Default::default())
            .unwrap_err();
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
}

#[test]
fn suspended_fixed_heads_accept_computed_type_packets() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        pure comptime fn Pair<A,B>->Type {A}
        comptime fn packet(T:Type)->Tuple<Type,Type> {(T,T)}
        fn identity(value:Pair<bool,bool>)->Pair<bool,bool> {value}
        comptime fn keep(T:Type,callback:defer fn(Pair<...packet(T)>)->Pair<...packet(T)>)->defer fn(Pair<...packet(T)>)->Pair<...packet(T)> {callback}
        const CALLBACK=keep(bool,identity);typealias Callback=CALLBACK'type;
        fn run(callback:Callback,value:Pair<bool,bool>)->Pair<bool,bool> {callback(value)}");
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn suspended_fixed_head_packets_check_shape_and_field_domains() {
    for (producer, diagnostic) in [
        (
            "comptime fn packet(T:Type)->Tuple<Type,bool> {(T,true)}",
            "elaboration failed",
        ),
        (
            "comptime fn packet(T:Type)->Tuple<Type> {(T,)}",
            "missing required type argument",
        ),
        (
            "comptime fn packet(T:Type)->Tuple<Type,Type,Type> {(T,T,T)}",
            "type argument count mismatch",
        ),
        (
            "comptime fn packet(T:Type)->bool {true}",
            "type argument spread requires a tuple",
        ),
    ] {
        let source=format!("typealias Type=.Type'builtin;
            pure comptime fn Pair<A,B>->Type {{A}} {producer}
            comptime fn keep(T:Type,callback:defer fn(Pair<...packet(T)>)->Pair<T,T>)->bool {{true}}");
        let failure = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_err();
        assert!(failure.contains(diagnostic), "{failure}");
    }
    let failure = execution_support::compile_with_tuple_protocols(
        "typealias Type=.Type'builtin;
        pure comptime fn Pair<A,B>->Type {A}
        comptime fn packet(T:Type)->Tuple<Type,Type> {assumes false;(T,T)}
        fn identity(value:Pair<bool,bool>)->Pair<bool,bool> {value}
        comptime fn keep(T:Type,callback:defer fn(Pair<...packet(T)>)->Pair<T,T>)->bool {true}
        const BAD=keep(bool,identity);",
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
}

#[test]
fn suspended_fixed_heads_preserve_computed_empty_packets() {
    let prefix="typealias Type=.Type'builtin;
        pure comptime fn Pair<A,B>->Type {A}
        comptime fn empty(T:Type,...items:Args)->Args where Args {items}
        fn identity(value:Pair<bool,bool>)->Pair<bool,bool> {value}
        comptime fn keep(T:Type,callback:defer fn(Pair<...empty(T),T,T>)->Pair<T,T>)->defer fn(Pair<T,T>)->Pair<T,T> {callback}
        const CALLBACK=keep(bool,identity);typealias Callback=CALLBACK'type;
        fn run(callback:Callback,value:Pair<bool,bool>)->Pair<bool,bool> {callback(value)}";
    let package = compile(prefix);
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let failure = compile_with_protocols(
        &prefix.replace(
            "empty(T:Type,...items:Args)->Args where Args {items}",
            "empty(T:Type,...items:Args)->Args where Args {assumes false;items}",
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
}

#[test]
fn suspended_empty_packets_precede_defaults_and_do_not_admit_void() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Spin()->Type {while true {} bool}
        pure comptime fn Pair<A,.B:Type=Spin()>->Type {A}
        comptime fn empty(T:Type,...items:Args)->Args where Args {assumes false;items}
        fn identity(value:Pair<bool,.B=bool>)->Pair<bool,.B=bool> {value}
        comptime fn keep(T:Type,callback:defer fn(Pair<...empty(T),T>)->Pair<T,.B=bool>)->bool {true}";
    let failure = execution_support::compile_with_tuple_protocols(
        &format!("{prefix} const BAD=keep(bool,identity);"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    let failure = execution_support::compile_with_tuple_protocols(
        &prefix.replace(
            "comptime fn empty(T:Type,...items:Args)->Args where Args {assumes false;items}",
            "comptime fn empty(T:Type)->.void'builtin {()}",
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        failure.contains("type argument spread requires a tuple"),
        "{failure}"
    );
}

#[test]
fn suspended_normal_indices_describe_boolean_computations() {
    for (expression, input, output) in [
        ("not flag", "true", "false"),
        ("flag and true", "true", "true"),
        ("flag or false", "false", "false"),
    ] {
        let source=format!("typealias Type=.Type'builtin;
            pure comptime fn Indexed<flag:bool>->Type {{bool}}
            fn identity(value:Indexed<{output}>)->Indexed<{output}> {{value}}
            comptime fn keep(flag:bool,callback:defer fn(Indexed<{expression}>)->Indexed<{expression}>)->defer fn(Indexed<{expression}>)->Indexed<{expression}> {{callback}}
            const CALLBACK=keep({input},identity);typealias Callback=CALLBACK'type;
            fn run(callback:Callback,value:Indexed<{output}>)->Indexed<{output}> {{callback(value)}}");
        let package = compile(&source);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn suspended_index_computations_keep_short_circuit_and_arithmetic_checks() {
    let source="typealias Type=.Type'builtin;
        pure comptime fn Indexed<flag:bool>->Type {bool}
        comptime fn fail(flag:bool)->bool {assumes false;flag}
        fn identity(value:Indexed<true>)->Indexed<true> {value}
        comptime fn keep(flag:bool,callback:defer fn(Indexed<flag or fail(flag)>)->Indexed<true>)->bool {true}
        const GOOD=keep(true,identity);typealias Result=GOOD'type;fn run()->Result {true}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile_with_protocols(
        &source.replace("keep(true,identity)", "keep(false,identity)"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    let arithmetic="typealias Type=.Type'builtin;typealias Int=.Integer'builtin;
        pure comptime fn Indexed<n:Int>->Type {bool}
        fn identity(value:Indexed<10>)->Indexed<10> {value}
        comptime fn keep(n:Int,callback:defer fn(Indexed<n + 1>)->Indexed<n + 1>)->defer fn(Indexed<n + 1>)->Indexed<n + 1> {callback}
        const CALLBACK=keep(9,identity);typealias Callback=CALLBACK'type;
        fn run(callback:Callback,value:Indexed<10>)->Indexed<10> {callback(value)}";
    let package = compile(arithmetic);
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let failure = compile_with_protocols(
        &arithmetic.replace(
            "typealias Int=.Integer'builtin;",
            "typealias Int=.u8'builtin;",
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn suspended_normal_indices_resume_conditional_branches() {
    for (input, output) in [("true", "false"), ("false", "true")] {
        let source=format!("typealias Type=.Type'builtin;
            pure comptime fn Indexed<flag:bool>->Type {{bool}}
            fn identity(value:Indexed<{output}>)->Indexed<{output}> {{value}}
            comptime fn keep(flag:bool,callback:defer fn(Indexed<if flag {{false}} else {{true}}>)->Indexed<{output}>)->bool {{true}}
            const GOOD=keep({input},identity);typealias Result=GOOD'type;
            fn run()->Result {{true}}");
        let package = compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn suspended_conditional_indices_check_both_domains_and_run_selected_branch() {
    let prefix="typealias Type=.Type'builtin;
        pure comptime fn Indexed<flag:bool>->Type {bool}
        comptime fn fail(flag:bool)->bool {assumes false;flag}
        fn identity(value:Indexed<true>)->Indexed<true> {value}
        comptime fn keep(flag:bool,callback:defer fn(Indexed<if flag {true} else {fail(flag)}>)->Indexed<true>)->bool {true}";
    compile(&format!("{prefix} const GOOD=keep(true,identity);"));
    let failure = compile_with_protocols(
        &format!("{prefix} const BAD=keep(false,identity);"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    let failure = compile_with_protocols(
        &prefix.replace("else {fail(flag)}", "else {7}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
    for (input, output) in [("true", "bool"), ("false", "Byte")] {
        let package=compile(&format!("typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
            pure comptime fn Identity<T>->Type {{T}}
            fn identity(value:Identity<{output}>)->Identity<{output}> {{value}}
            comptime fn keep(flag:bool,callback:defer fn(Identity<if flag {{bool}} else {{Byte}}>)->Identity<{output}>)->bool {{true}}
            const GOOD=keep({input},identity);typealias Result=GOOD'type;fn run()->Result {{true}}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn suspended_normal_indices_support_local_computation_blocks() {
    let package=compile("typealias Type=.Type'builtin;
        pure comptime fn Indexed<flag:bool>->Type {bool}
        fn identity(value:Indexed<false>)->Indexed<false> {value}
        comptime fn keep(flag:bool,callback:defer fn(Indexed<do {let inverted=not flag;inverted}>)->Indexed<false>)->bool {true}
        const GOOD=keep(true,identity);typealias Result=GOOD'type;
        fn run()->Result {true}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn suspended_index_blocks_keep_local_mutability_and_scope() {
    let prefix = "typealias Type=.Type'builtin;
        pure comptime fn Indexed<flag:bool>->Type {bool}
        fn identity(value:Indexed<false>)->Indexed<false> {value}";
    for block in [
        "do {var selected=flag;selected=not selected;selected}",
        "do {let selected=flag;do {let selected=not selected;selected}}",
    ] {
        let package=compile(&format!("{prefix}
            comptime fn keep(flag:bool,callback:defer fn(Indexed<{block}>)->Indexed<false>)->bool {{true}}
            const GOOD=keep(true,identity);typealias Result=GOOD'type;fn run()->Result {{true}}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    for block in [
        "do {let selected=flag;selected=not selected;selected}",
        "do {var selected=flag;selected=7;selected}",
    ] {
        let failure=compile_with_protocols(&format!("{prefix}
            comptime fn keep(flag:bool,callback:defer fn(Indexed<{block}>)->Indexed<false>)->bool {{true}}"),&Default::default()).unwrap_err();
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
}

#[test]
fn suspended_local_type_values_feed_nested_normal_heads() {
    let package=compile("typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        fn identity(value:Identity<bool>)->Identity<bool> {value}
        comptime fn keep(T:Type,callback:defer fn(do {let U=T;Identity<U>})->Identity<T>)->bool {true}
        const GOOD=keep(bool,identity);typealias Result=GOOD'type;fn run()->Result {true}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn suspended_local_dependencies_feed_nested_value_head_domains() {
    let prefix="typealias Type=.Type'builtin;
        pure comptime fn Tagged<T,value:T>->Type {T}
        fn identity(value:Tagged<bool,true>)->Tagged<bool,true> {value}
        comptime fn keep(T:Type,value:defer T,callback:defer fn(do {let U=T;let item=value;Tagged<U,item>})->Tagged<T,value>)->bool {true}";
    let package = compile(&format!(
        "{prefix} const GOOD=keep(bool,true,identity);typealias Result=GOOD'type;fn run()->Result {{true}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile_with_protocols(
        &format!("{prefix} const BAD=keep(bool,false,identity);"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
}

#[test]
fn suspended_normal_indices_execute_local_while_computations() {
    for (count, expected) in [(0, "false"), (1, "true"), (2, "false"), (3, "true")] {
        let source=format!("typealias Type=.Type'builtin;typealias Int=.Integer'builtin;
            pure comptime fn Indexed<flag:bool>->Type {{bool}}
            fn identity(value:Indexed<{expected}>)->Indexed<{expected}> {{value}}
            comptime fn keep(count:Int,callback:defer fn(Indexed<do {{var n=count;var flag=false;while n > 0 {{flag=not flag;n=n - 1;}} flag}}>)->Indexed<{expected}>)->bool {{true}}
            const GOOD=keep({count},identity);typealias Result=GOOD'type;fn run()->Result {{true}}");
        let package = compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn suspended_index_loops_keep_break_continue_and_body_checks() {
    for (count, expected) in [(0, "false"), (1, "true"), (3, "true")] {
        let source=format!("typealias Type=.Type'builtin;typealias Int=.Integer'builtin;
            pure comptime fn Indexed<flag:bool>->Type {{bool}}
            fn identity(value:Indexed<{expected}>)->Indexed<{expected}> {{value}}
            comptime fn keep(count:Int,callback:defer fn(Indexed<do {{var remaining=count;var seen=false;while remaining > 0 {{remaining=remaining - 1;if remaining > 0 {{continue;}} seen=true;break;}} seen}}>)->Indexed<{expected}>)->bool {{true}}
            const GOOD=keep({count},identity);typealias Result=GOOD'type;fn run()->Result {{true}}");
        let package = compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    let failure=compile_with_protocols("typealias Type=.Type'builtin;
        pure comptime fn Indexed<flag:bool>->Type {bool}
        comptime fn bad(flag:bool,callback:defer fn(Indexed<do {var result=flag;while false {result=7;} result}>)->Indexed<true>)->bool {true}",&Default::default()).unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
}

#[test]
fn suspended_match_indices_select_and_bind_patterns() {
    for (input, output) in [("true", "false"), ("false", "true")] {
        let package=compile(&format!("typealias Type=.Type'builtin;
            pure comptime fn Indexed<flag:bool>->Type {{bool}}
            fn identity(value:Indexed<{output}>)->Indexed<{output}> {{value}}
            comptime fn keep(flag:bool,callback:defer fn(Indexed<flag match {{true=>false,false=>true}}>)->Indexed<{output}>)->bool {{true}}
            const GOOD=keep({input},identity);typealias Result=GOOD'type;fn run()->Result {{true}}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn suspended_match_indices_preserve_enum_bindings_and_coverage() {
    for (input, output) in [
        ("Choice.none", "false"),
        ("Choice.some(true)", "false"),
        ("Choice.some(false)", "true"),
    ] {
        let package=compile(&format!("typealias Type=.Type'builtin;enum Choice {{none,some(bool)}}
            pure comptime fn Indexed<flag:bool>->Type {{bool}}
            fn identity(value:Indexed<{output}>)->Indexed<{output}> {{value}}
            comptime fn keep(choice:Choice,callback:defer fn(Indexed<choice match {{.none=>false,.some(value)=>not value}}>)->Indexed<{output}>)->bool {{true}}
            const GOOD=keep({input},identity);typealias Result=GOOD'type;fn run()->Result {{true}}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    for (arms, diagnostic) in [
        ("true=>false", "non-exhaustive"),
        ("true=>false,false=>7", "elaboration failed"),
    ] {
        let source=format!("typealias Type=.Type'builtin;
            pure comptime fn Indexed<flag:bool>->Type {{bool}}
            comptime fn bad(flag:bool,callback:defer fn(Indexed<flag match {{{arms}}}>)->Indexed<true>)->bool {{true}}");
        let failure = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(failure.contains(diagnostic), "{failure}");
    }
}

#[test]
fn higher_order_comptime_inputs_compute_deferred_domains() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        comptime fn keep(select:comptime fn(Type)->Type,T:Type,value:defer select(T))->defer select(T) {value}
        const RESULT=keep(Select,bool,true);typealias Result=RESULT'type;
        fn run()->Result {true}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn higher_order_comptime_domains_keep_callable_identity_and_dependencies() {
    for (selector, expected, literal) in [("Select", "bool", "true"), ("SelectByte", "Byte", "7")] {
        let package = compile(&format!("typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
            comptime fn Forward(T:Type)->Type {{T}}
            comptime fn Select(T:Type)->Type {{Forward(T)}}
            comptime fn SelectByte(T:Type)->Type {{Forward(Byte)}}
            comptime fn choose()->comptime fn(Type)->Type {{{selector}}}
            comptime fn keep(select:comptime fn(Type)->Type,T:Type,value:defer select(T))->defer select(T) {{value}}
            const RESULT=keep(choose(),bool,{literal});
            typealias Result=RESULT'type;
            fn run()->Result {{{literal}}}
            fn check(value:{expected})->Result {{value}}"));
        let result = luna::execution::interpret(&package, "run", 100_000).unwrap();
        assert_eq!(
            result,
            if expected == "bool" {
                Value::Bool(true)
            } else {
                Value::UnsignedInteger(7)
            }
        );
    }
}

#[test]
fn higher_order_comptime_domains_reject_wrong_values_and_execute_contracts() {
    for (body, argument) in [("Byte", "true"), ("assumes false; T", "true")] {
        let source = format!("typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
            comptime fn Select(T:Type)->Type {{{body}}}
            comptime fn keep(select:comptime fn(Type)->Type,T:Type,value:defer select(T))->defer select(T) {{value}}
            const RESULT=keep(Select,bool,{argument});typealias Result=RESULT'type;");
        let failure = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(failure.contains("elaboration failed"), "{failure}");
        if body.contains("assumes") {
            assert!(failure.contains("ProgramContractRejected"), "{failure}");
        }
    }
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        fn Select(T:Type)->Type {T}
        comptime fn keep(select:comptime fn(Type)->Type,T:Type,value:defer select(T))->defer select(T) {value}
        const RESULT=keep(Select,bool,true);typealias Result=RESULT'type;";
    assert!(compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn higher_order_comptime_domains_preserve_generic_instance_arguments() {
    let prefix="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(ignored:Type)->Type where T {T}
        comptime fn keep(select:comptime fn(Type)->Type,value:defer select(bool))->defer select(bool) {value}";
    for (instance, value) in [("bool", "true"), ("Byte", "7")] {
        let package = compile(&format!(
            "{prefix} const RESULT=keep(Select<{instance}>,{value});
            typealias Result=RESULT'type;fn run()->Result {{{value}}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            if instance == "bool" {
                Value::Bool(true)
            } else {
                Value::UnsignedInteger(7)
            }
        );
    }
    let failure = compile_with_protocols(
        &format!("{prefix} const BAD=keep(Select<Byte>,true);"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
}

#[test]
fn higher_order_comptime_instances_share_declaration_but_not_specialization() {
    let prefix="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(ignored:Type)->Type where T {T}
        comptime fn keep(select:comptime fn(Type)->Type,value:defer select(bool))->defer select(bool) {value}";
    let package = compile(&format!(
        "{prefix}
        const FIRST=keep(Select<bool>,true);typealias First=FIRST'type;
        const SECOND=keep(Select<Byte>,7);typealias Second=SECOND'type;
        const THIRD=keep(Select<bool>,false);typealias Third=THIRD'type;
        fn first(value:bool)->First {{value}}
        fn second(value:Byte)->Second {{value}}
        fn third(value:bool)->Third {{value}}
        fn run()->bool {{true}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let constrained = prefix.replace("where T {T}", "where T,requires T == bool {T}");
    compile(&format!(
        "{constrained} const GOOD=keep(Select<bool>,true);"
    ));
    let failure = compile_with_protocols(
        &format!("{constrained} const BAD=keep(Select<Byte>,7);"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
}

#[test]
fn captured_comptime_callbacks_compute_deferred_domains() {
    let package = compile(
        "typealias Type=.Type'builtin;
        typealias Selector=%{selected:Type} comptime fn()->Type;
        comptime fn make(selected:Type)->Selector {|| selected}
        comptime fn keep(select:Selector,value:defer select())->defer select() {value}
        const RESULT=keep(make(bool),true);typealias Result=RESULT'type;
        fn run()->Result {true}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn captured_comptime_domains_keep_snapshots_distinct_and_allow_local_updates() {
    let prefix="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        typealias Selector=%{selected:Type} comptime fn()->Type;
        comptime fn Forward(T:Type)->Type {T}
        comptime fn make(selected:Type)->Selector {|| {var local=selected;local=Forward(local);local}}
        comptime fn keep(select:Selector,value:defer select())->defer select() {value}";
    let package = compile(&format!(
        "{prefix}
        const FIRST=keep(make(bool),true);typealias First=FIRST'type;
        const SECOND=keep(make(Byte),7);typealias Second=SECOND'type;
        fn first(value:bool)->First {{value}}
        fn second(value:Byte)->Second {{value}}
        fn run()->Second {{7}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
    let failure = compile_with_protocols(
        &format!("{prefix} const BAD=keep(make(Byte),true);"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
}

#[test]
fn live_closure_dependencies_allow_capture_mutation() {
    for body in ["selected=bool;selected"] {
        let package = compile(&format!(
            "typealias Type=.Type'builtin;
            typealias Selector=%{{selected:Type}} comptime fn()->Type;
            comptime fn keep(select:Selector,value:defer select())->defer select() {{value}}
            comptime fn make()->Selector {{var selected=bool;|| {{{body}}}}}
            const RESULT=keep(make(),true);typealias Result=RESULT'type;
            fn run()->Result {{true}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn captured_comptime_domains_accept_computed_packet_inputs() {
    let package = execution_support::compile_with_tuple(
        "typealias Type=.Type'builtin;
        typealias Selector=%{selected:Type} comptime fn()->Type;
        comptime fn make(selected:Type)->Selector {|| selected}
        comptime fn packet()->Tuple<Selector,bool> {(make(bool),true)}
        comptime fn keep(select:Selector,value:defer select())->defer select() {value}
        const RESULT=keep(...packet());typealias Result=RESULT'type;
        fn run()->Result {true}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn packet_closure_domains_preserve_sibling_shared_state_and_defaults() {
    for (packet_type, packet_value, value_parameter) in [
        (
            "Tuple<Selector,Callback,Callback,bool>",
            "(make(bool),action,action,true)",
            "value:defer select()",
        ),
        (
            "Tuple<Selector,Callback,Callback>",
            "(make(bool),action,action)",
            ".value:defer select()=true",
        ),
    ] {
        let package=execution_support::compile_with_tuple(&format!("typealias Type=.Type'builtin;
            typealias Selector=%{{selected:Type}} comptime fn()->Type;
            typealias Callback=%{{flag:bool}} comptime fn()->bool;
            comptime fn make(selected:Type)->Selector {{|| selected}}
            comptime fn toggle()->Callback {{var flag=false;|| {{flag=not flag;flag}}}}
            comptime fn packet()->{packet_type} {{let action=toggle();{packet_value}}}
            comptime fn keep(select:Selector,left:Callback,right:Callback,{value_parameter})->defer select() {{left();let observed=right();assumes not observed;value}}
            const RESULT=keep(...packet());typealias Result=RESULT'type;
            fn run()->Result {{true}}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn packet_closure_domains_reject_wrong_fields_and_accept_stateful_selectors() {
    let prefix = "typealias Type=.Type'builtin;
        typealias Selector=%{selected:Type} comptime fn()->Type;
        comptime fn make()->Selector {var selected=.u8'builtin;|| selected}
        comptime fn packet()->Tuple<Selector,bool> {(make(),true)}
        comptime fn keep(select:Selector,value:defer select())->defer select() {value}
        const BAD=keep(...packet());typealias Result=BAD'type;";
    let failure =
        execution_support::compile_with_tuple_protocols(prefix, &Default::default()).unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
    let stateful = prefix.replace("|| selected", "|| {selected=bool;selected}");
    let package =
        execution_support::compile_with_tuple(&format!("{stateful} fn run()->Result {{true}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nested_readonly_closures_compute_deferred_domains() {
    let package = compile(
        "typealias Type=.Type'builtin;
        typealias Inner=%{selected:Type} comptime fn()->Type;
        typealias Outer=%{inner:Inner} comptime fn()->Type;
        comptime fn make(selected:Type)->Inner {|| selected}
        comptime fn wrap(inner:Inner)->Outer {|| inner()}
        comptime fn keep(select:Outer,value:defer select())->defer select() {value}
        const RESULT=keep(wrap(make(bool)),true);typealias Result=RESULT'type;
        fn run()->Result {true}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn readonly_closure_graphs_preserve_shared_nested_tuple_captures() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        typealias Inner=%{selected:Type} comptime fn()->Type;
        typealias Outer=%{pair:Tuple<Inner,Inner>} comptime fn()->Type;
        comptime fn make(selected:Type)->Inner {|| selected}
        comptime fn wrap(inner:Inner)->Outer {let pair=(inner,inner);|| {let (left,right)=pair;left();right()}}
        comptime fn keep(select:Outer,value:defer select())->defer select() {value}
        const RESULT=keep(wrap(make(.u8'builtin)),7);typealias Result=RESULT'type;
        fn run()->Result {7}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
}

#[test]
fn readonly_closure_graphs_accept_structural_record_captures() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {selected:bool};typealias Payload=SAMPLE'type;
        typealias Selector=%{payload:Payload} comptime fn()->Type;
        comptime fn make(payload:Payload)->Selector {|| payload.selected}
        comptime fn keep(select:Selector,value:defer select())->defer select() {value}";
    let package = compile(&format!(
        "{prefix}
        const RESULT=keep(make(schema {{selected:.u8'builtin}}),7);typealias Result=RESULT'type;
        fn run()->Result {{7}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
    let failure = compile_with_protocols(
        &format!("{prefix} const BAD=keep(make(schema {{selected:.u8'builtin}}),true);"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
}

#[test]
fn live_outer_closures_share_mutable_inner_captures() {
    let prefix = "typealias Type=.Type'builtin;
        typealias Inner=%{selected:Type} comptime fn()->Type;
        typealias Outer=%{inner:Inner} comptime fn()->Type;
        comptime fn wrap(inner:Inner)->Outer {|| inner()}
        comptime fn keep(select:Outer,value:defer select())->defer select() {value}";
    for body in ["selected=bool;selected"] {
        let source = format!(
            "{prefix} comptime fn make()->Inner {{var selected=bool;|| {{{body}}}}}
            const BAD=keep(wrap(make()),true);"
        );
        compile(&source);
    }
}

#[test]
fn readonly_closure_graphs_capture_generic_function_instances() {
    let package = compile(
        "typealias Type=.Type'builtin;
        typealias Selector=%{select:comptime fn(Type)->Type} comptime fn()->Type;
        comptime fn Select(ignored:Type)->Type where T {T}
        comptime fn wrap(select:comptime fn(Type)->Type)->Selector {|| select(bool)}
        comptime fn keep(select:Selector,value:defer select())->defer select() {value}
        const FIRST=keep(wrap(Select<bool>),true);typealias First=FIRST'type;
        const SECOND=keep(wrap(Select<.u8'builtin>),7);typealias Second=SECOND'type;
        fn first(value:bool)->First {value}
        fn run()->Second {7}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
}

#[test]
fn readonly_selector_can_construct_fresh_mutable_local_closures() {
    let source="typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {let flip=|| {flag=not flag;flag};if flip() {bool} else {.u8'builtin}}}
        comptime fn keep(select:Selector,value:defer select())->defer select() {value}
        const RESULT=keep(make(),true);typealias Result=RESULT'type;
        fn run()->Result {true}";
    for body in [
        "flag=not flag;flag",
        "let pointer=flag.ref;pointer.* = not pointer.*;flag",
    ] {
        let package = compile(&source.replace("flag=not flag;flag", body));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn readonly_closure_domains_preserve_generic_type_inputs() {
    let package = compile(
        "typealias Type=.Type'builtin;
        typealias Selector=%{selected:Type} comptime fn()->Type;
        comptime fn make(selected:Type)->Selector {|| selected}
        comptime fn keep(select:Selector,value:defer select())->defer select() {value}
        comptime fn forward(value:T)->T where T {keep(make(T),value)}
        const FIRST=forward<bool>(true);typealias First=FIRST'type;
        const SECOND=forward<.u8'builtin>(7);typealias Second=SECOND'type;
        fn first(value:bool)->First {value}
        fn run()->Second {7}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
}

#[test]
fn generic_readonly_closure_domains_survive_wrapping_and_packets() {
    for call in ["keep(wrap(make(T)),value)", "keep(...packet<T>(),value)"] {
        let package = execution_support::compile_with_tuple(&format!(
            "typealias Type=.Type'builtin;
            typealias Inner=%{{selected:Type}} comptime fn()->Type;
            typealias Outer=%{{inner:Inner}} comptime fn()->Type;
            comptime fn make(selected:Type)->Inner {{|| selected}}
            comptime fn wrap(inner:Inner)->Outer {{|| inner()}}
            comptime fn packet()->Tuple<Outer> where U {{(wrap(make(U)),)}}
            comptime fn keep(select:Outer,value:defer select())->defer select() {{value}}
            comptime fn forward(value:T)->T where T {{{call}}}
            const RESULT=forward<.u8'builtin>(7);typealias Result=RESULT'type;
            fn run()->Result {{7}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::UnsignedInteger(7)
        );
    }
}

#[test]
fn generic_closure_domains_retain_caller_type_premises() {
    let prefix="typealias Type=.Type'builtin;
        typealias Selector=%{selected:Type} comptime fn()->Type;
        comptime fn Fixed(ignored:Type)->Type where T,requires T == bool {T}
        comptime fn make()->Selector where T,requires T == bool {let selected=T;|| Fixed<T>(selected)}
        comptime fn keep(select:Selector,value:defer select())->defer select() {value}
        comptime fn forward(value:T)->T where T,requires T == bool {keep(make<T>(),value)}";
    let package = compile(&format!(
        "{prefix} const RESULT=forward<bool>(true);typealias Result=RESULT'type;fn run()->Result {{true}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile_with_protocols(
        &format!("{prefix} const BAD=forward<.u8'builtin>(7);"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
}

#[test]
fn live_selectors_share_state_across_parameter_and_return_annotations() {
    let package = compile(
        "typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {
            var flag=false;
            || {flag=not flag; if flag {bool} else {.u8'builtin}}
        }
        comptime fn choose(select:Selector,value:defer select())->defer select() {7}
        const RESULT=choose(make(),true);
        typealias Result=RESULT'type;
        fn run()->Result {7}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
}

#[test]
fn live_selector_delivers_annotation_state_to_final_comptime_body() {
    let prefix="typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn check(T:Type)->bool {if T == .u8'builtin {true} else {assumes false;false}}
        comptime fn consume(select:Selector,value:defer select())->bool {check(select())}";
    let package = compile(&format!(
        "{prefix} const RESULT=consume(make(),true);typealias Result=RESULT'type;fn run()->Result {{true}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn live_selector_instances_keep_distinct_formed_signatures() {
    let prefix = "typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make(flag:bool)->Selector {|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn inspect(select:Selector,value:defer select())->bool {true}";
    compile(&format!(
        "{prefix} const FIRST=inspect(make(false),true);const SECOND=inspect(make(true),7);"
    ));
    for call in ["inspect(make(false),7)", "inspect(make(true),true)"] {
        let failure =
            compile_with_protocols(&format!("{prefix} const BAD={call};"), &Default::default())
                .unwrap_err();
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
}

#[test]
fn live_packet_selectors_share_annotation_and_final_body_mutations() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn packet()->Tuple<Selector,Selector,bool> {let select=make();(select,select,true)}
        comptime fn expect_bool(T:Type)->bool {if T == bool {true} else {assumes false;false}}
        comptime fn expect_byte(T:Type)->bool {if T == .u8'builtin {true} else {assumes false;false}}
        comptime fn consume(left:Selector,right:Selector,value:defer left())->defer right() {
            expect_bool(left());expect_byte(right());7
        }
        const RESULT=consume(...packet());typealias Result=RESULT'type;
        fn run()->Result {7}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
}

#[test]
fn live_selector_can_update_primitive_capture_through_local_pointer() {
    let package = compile(
        "typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {
            let pointer=flag.ref;pointer.* = not pointer.*;
            if flag {bool} else {.u8'builtin}
        }}
        comptime fn consume(select:Selector,value:defer select())->defer select() {7}
        const RESULT=consume(make(),true);typealias Result=RESULT'type;
        fn run()->Result {7}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
    let failure = compile_with_protocols(
        "typealias Type=.Type'builtin;
        typealias Selector=%{selected:Type} comptime fn()->Type;
        comptime fn make()->Selector {var selected=bool;|| {let pointer=selected.ref;pointer.*}}
        comptime fn consume(select:Selector,value:defer select())->bool {true}
        const BAD=consume(make(),true);",
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        failure.contains("requires bound Copy and Drop protocols"),
        "{failure}"
    );
}

#[test]
fn live_annotation_is_not_replayed_when_preparing_its_staged_input() {
    let package=compile("typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn consume(select:Selector,comptime value:defer select())->bool {true}
        const RESULT=consume(make(),true);typealias Result=RESULT'type;
        fn run()->Result {true}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn live_annotation_preserves_aliases_in_undemanded_packet_fields() {
    execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn packet()->Tuple<Selector,Tuple<Selector>> {let select=make();(select,(select,))}
        comptime fn check(T:Type)->bool {if T == .u8'builtin {true} else {assumes false;false}}
        comptime fn consume(select:Selector,wrapped:Tuple<Selector>,.value:defer select()=true)->bool {
            let (other,)=wrapped;check(other())
        }
        const RESULT=consume(...packet());");
}

#[test]
fn live_default_selectors_preserve_state_and_explicit_override() {
    let prefix="typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn fail()->Selector {assumes false;make()}
        comptime fn consume(.select:Selector=make(),.value:defer select()=true)->defer select() {7}";
    let package = compile(&format!(
        "{prefix} const RESULT=consume();typealias Result=RESULT'type;fn run()->Result {{7}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
    let overridden = prefix.replace(".select:Selector=make()", ".select:Selector=fail()");
    compile(&format!(
        "{overridden} const RESULT=consume(.select=make());"
    ));
    let failure = compile_with_protocols(
        &format!("{overridden} const BAD=consume();"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
}

#[test]
fn live_defaults_read_and_alias_earlier_callback_state() {
    let prefix="typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn check(T:Type)->bool {if T == bool {true} else {assumes false;false}}
        comptime fn consume(select:Selector,.alias:Selector=select,.chosen:Type=alias(),.value:defer chosen=true)->defer select() {
            check(alias());7
        }";
    let package = compile(&format!(
        "{prefix} const RESULT=consume(make());typealias Result=RESULT'type;fn run()->Result {{7}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
}

#[test]
fn live_default_aliases_share_packet_and_nested_default_graphs() {
    let prefix="typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        typealias Wrapped=%{inner:Selector} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn wrap(inner:Selector)->Wrapped {|| inner()}
        comptime fn packet()->Tuple<Selector> {(make(),)}
        comptime fn check(T:Type)->bool {if T == bool {true} else {assumes false;false}}
        comptime fn consume(select:Selector,.alias:Wrapped=wrap(select),.chosen:Type=alias(),.value:defer chosen=true)->defer select() {
            check(alias());7
        }";
    let package = execution_support::compile_with_tuple(&format!(
        "{prefix} const RESULT=consume(...packet());typealias Result=RESULT'type;fn run()->Result {{7}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
}

#[test]
fn live_defaults_execute_earlier_unused_effects_before_demanded_defaults() {
    compile("typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn check(T:Type)->bool {if T == bool {true} else {assumes false;false}}
        comptime fn consume(select:Selector,.unused:Type=select(),.chosen:Type=select(),.value:defer chosen=7)->bool {
            check(select())
        }
        const RESULT=consume(make());");
}

#[test]
fn live_variadic_selectors_share_state_across_domains_and_body() {
    let prefix="typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn packet()->Tuple<Selector,Selector> {let select=make();(select,select)}
        comptime fn First(selectors:Tuple<Selector,Selector>)->Type {let (first,second)=selectors;first()}
        comptime fn Second(selectors:Tuple<Selector,Selector>)->Type {let (first,second)=selectors;second()}
        comptime fn check(T:Type)->bool {if T == bool {true} else {assumes false;false}}
        comptime fn consume(...selectors:Tuple<Selector,Selector>,.value:defer First(selectors)=true)->defer Second(selectors) {
            check(First(selectors));7
        }";
    let package = execution_support::compile_with_tuple(&format!(
        "{prefix} const RESULT=consume(...packet());typealias Result=RESULT'type;fn run()->Result {{7}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
    execution_support::compile_with_tuple(&format!("{prefix}
        comptime fn independent(...selectors:Tuple<Selector,Selector>,.value:defer First(selectors)=true)->bool {{check(Second(selectors))}}
        const SEPARATE=independent(make(),make());"));
    execution_support::compile_with_tuple(&format!("{prefix}
        comptime fn defaulted(...selectors:Tuple<Selector,Selector>,.alias:Tuple<Selector,Selector> = selectors,.value:defer First(alias)=true)->defer Second(selectors) {{check(First(alias));7}}
        const SHARED=defaulted(...packet());"));
    // Two independent producers must retain separate state rather than being
    // interned together merely because they share a body and capture values.
    let failure = execution_support::compile_with_tuple_protocols(
        &format!("{prefix} const BAD=consume(make(),make());"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
}

#[test]
fn live_variadic_graph_preserves_undemanded_fixed_packet_aliases() {
    execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn packet()->Tuple<Tuple<Selector>,Selector,Selector> {let select=make();((select,),select,select)}
        comptime fn First(selectors:Tuple<Selector,Selector>)->Type {let (first,second)=selectors;first()}
        comptime fn Second(selectors:Tuple<Selector,Selector>)->Type {let (first,second)=selectors;second()}
        comptime fn check(T:Type)->bool {if T == bool {true} else {assumes false;false}}
        comptime fn consume(wrapped:Tuple<Selector>,...selectors:Tuple<Selector,Selector>,.value:defer First(selectors)=true)->defer Second(selectors) {
            let (other,)=wrapped;check(other());7
        }
        const RESULT=consume(...packet());");
}

#[test]
fn live_variadic_producers_keep_interleaved_named_argument_order() {
    let failure=execution_support::compile_with_tuple_protocols("typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;bool}}
        comptime fn fail()->bool {assumes false;true}
        comptime fn spin()->Selector {while true {} make()}
        comptime fn First(selectors:Tuple<Selector,Selector>)->Type {let (first,second)=selectors;first()}
        comptime fn consume(...selectors:Tuple<Selector,Selector>,.marker:bool=true,.value:defer First(selectors)=true)->bool {true}
        const BAD=consume(make(),.marker=fail(),spin());",&Default::default()).unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    assert!(
        !failure.contains("StepsExhausted"),
        "later collection producer must not overtake named input: {failure}"
    );
}

#[test]
fn live_empty_collection_domains_run_once_in_declaration_order() {
    let source="typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {List<T>}
        pure comptime fn Tuple<...Es:List<Type>>->Type {Tuple<...Es>}
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn Empty(select:Selector)->Type {select();Tuple<>}
        comptime fn consume(select:Selector,...items:defer Empty(select))->defer select() {7}
        const RESULT=consume(make());typealias Result=RESULT'type;fn run()->Result {7}";
    let package = compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
}

#[test]
fn inferred_calls_retain_live_selectors_without_replaying_annotations() {
    let prefix="typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn check(T:Type)->bool {if T == bool {true} else {assumes false;false}}
        comptime fn consume(select:Selector,value:defer select(),marker:T)->defer select() where T {
            check(select());7
        }";
    let package = compile(&format!(
        "{prefix} const RESULT=consume(make(),true,true);const SECOND=consume(make(),true,false);typealias Result=RESULT'type;fn run()->Result {{7}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );
}

#[test]
fn inferred_calls_keep_live_default_and_packet_aliases() {
    let prefix="typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn packet()->Tuple<Selector,Selector> {let select=make();(select,select)}
        comptime fn check(T:Type)->bool {if T == bool {true} else {assumes false;false}}
        comptime fn First(selectors:Tuple<Selector,Selector>)->Type {let (first,second)=selectors;first()}
        comptime fn Second(selectors:Tuple<Selector,Selector>)->Type {let (first,second)=selectors;second()}
        comptime fn collected(...selectors:Tuple<Selector,Selector>,.value:defer First(selectors)=true,marker:T)->defer Second(selectors) where T {
            check(First(selectors));7
        }
        comptime fn defaulted(.select:Selector=make(),.alias:Selector=select,.value:defer alias()=true,marker:T)->defer select() where T {
            check(alias());7
        }";
    for call in ["collected(...packet(),true)", "defaulted(true)"] {
        let package = execution_support::compile_with_tuple(&format!(
            "{prefix} const RESULT={call};typealias Result=RESULT'type;fn run()->Result {{7}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::UnsignedInteger(7)
        );
    }
}

#[test]
fn result_inference_retains_already_prepared_live_default() {
    compile("typealias Type=.Type'builtin;
        typealias Selector=%{flag:bool} comptime fn()->Type;
        comptime fn make()->Selector {var flag=false;|| {flag=not flag;if flag {bool} else {.u8'builtin}}}
        comptime fn check(T:Type)->bool {if T == .u8'builtin {true} else {assumes false;false}}
        comptime fn consume(.select:Selector=make(),.value:defer select()=true)->T where T,requires T == bool {
            check(select())
        }
        comptime fn forward()->bool {consume()}
        const RESULT=forward();");
}

#[test]
fn inferred_live_default_waits_for_its_declaration_type_argument() {
    compile("typealias Type=.Type'builtin;
        typealias Selector=%{selected:Type} comptime fn()->Type;
        comptime fn make(selected:Type)->Selector {|| {selected=selected;selected}}
        comptime fn consume(.select:Selector=make(T),.alias:Selector=select,.value:defer alias()=true)->T where T,requires T == bool {value}
        comptime fn forward()->bool {consume()}
        const RESULT=forward();");
}

#[test]
fn runtime_staged_callbacks_form_domains_and_erase_inputs() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn run()->bool {consume(Boolean,true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn runtime_staged_callbacks_preserve_generic_identity_defaults_and_forwarding() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Selected()->Type where T {T}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn defaulted(comptime .select:comptime fn()->Type=Selected<bool>,.value:defer select()=true)->defer select() {value}
        fn forward(comptime select:comptime fn()->Type,value:defer select())->defer select() {consume(select,value)}";
    for (entry, body, expected) in [
        ("boolean", "consume(Selected<bool>,true)", Value::Bool(true)),
        (
            "byte",
            "consume(Selected<.u8'builtin>,9)",
            Value::UnsignedInteger(9),
        ),
        ("default_case", "defaulted()", Value::Bool(true)),
        (
            "override_case",
            "defaulted(.select=Selected<.u8'builtin>,.value=9)",
            Value::UnsignedInteger(9),
        ),
        (
            "forwarded",
            "forward(Selected<bool>,true)",
            Value::Bool(true),
        ),
    ] {
        let ty = if matches!(expected, Value::Bool(_)) {
            "bool"
        } else {
            ".u8'builtin"
        };
        let package = compile(&format!("{prefix} fn {entry}()->{ty} {{{body}}}"));
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            expected
        );
        let runtime = luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        for instance in &runtime.functions {
            let nir::Term::Function { parameters, .. } =
                instance.function.signature.callable_signature()
            else {
                panic!("function signature")
            };
            assert!(
                parameters.iter().all(|p| p.sort == nir::Sort::Runtime),
                "{}",
                instance.function.name
            );
        }
    }
}

#[test]
fn runtime_staged_callbacks_complete_producers_and_generic_factories() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn Selected()->Type where T {T}
        comptime fn make()->comptime fn()->Type {Selected<bool>}
        fn discard(comptime select:comptime fn()->Type,value:bool)->bool {value}
        fn run()->bool {discard(make(),true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn runtime_staged_callbacks_reject_wrong_domains_and_runtime_callbacks() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn runtime()->bool {true}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}";
    for body in ["consume(Boolean,9)", "consume(runtime,true)"] {
        assert!(
            compile_with_protocols(
                &format!("{prefix} fn bad()->bool {{{body}}}"),
                &Default::default()
            )
            .is_err()
        );
    }
}

#[test]
fn runtime_staged_callbacks_prepare_factories_after_generic_substitution() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn Selected()->Type where T {T}
        comptime fn make()->comptime fn()->Type where T {Selected<T>}
        fn discard(comptime select:comptime fn()->Type,value:T)->T where T {value}
        fn forward(value:T)->T where T {discard<T>(make<T>(),value)}
        fn run()->bool {forward<bool>(true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn runtime_staged_callbacks_do_not_skip_unused_producer_failures() {
    let failure = compile_with_protocols(
        "typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        comptime fn fail()->comptime fn()->Type {assumes false;Boolean}
        fn discard(comptime select:comptime fn()->Type)->bool {true}
        fn run()->bool {discard(fail())}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
}

#[test]
fn runtime_staged_callback_aggregates_form_domains_and_keep_dependencies() {
    let package = execution_support::compile_with_tuple(
        "typealias Type=.Type'builtin;
        typealias Pair=Tuple<comptime fn()->Type,comptime fn()->Type>;
        comptime fn Selected()->Type where T {T}
        comptime fn First(pair:Pair)->Type {let (first,second)=pair;first()}
        comptime fn packet()->Pair {(Selected<bool>,Selected<.u8'builtin>)}
        fn consume(comptime pair:Pair,value:defer First(pair))->defer First(pair) {value}
        fn run()->bool {consume(packet(),true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn runtime_staged_callback_aggregates_preserve_nominal_defaults() {
    let package=compile("typealias Type=.Type'builtin;
        struct Config {select:comptime fn()->Type}
        comptime fn Selected()->Type where T {T}
        comptime fn SelectedConfig(config:Config)->Type {config.select()}
        comptime fn make()->Config {Config {select:Selected<bool>}}
        fn consume(comptime .config:Config=make(),comptime .alias:Config=config,.value:defer SelectedConfig(alias)=true)->defer SelectedConfig(config) {value}
        fn run()->bool {consume()}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn runtime_staged_callback_aggregates_prepare_generic_producers() {
    let package = compile(
        "typealias Type=.Type'builtin;
        struct Config {select:comptime fn()->Type}
        comptime fn Selected()->Type where T {T}
        comptime fn make()->Config where T {Config {select:Selected<T>}}
        fn discard(comptime config:Config,value:T)->T where T {value}
        fn forward(value:T)->T where T {discard<T>(make<T>(),value)}
        fn run()->bool {forward<bool>(true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    let instance = graph
        .functions
        .iter()
        .find(|instance| instance.function.name == "discard")
        .unwrap();
    assert!(instance.key.arguments.iter().any(|argument| matches!(argument,
        nir::Term::NominalValue {variant, fields, ..}
        if *variant == None && matches!(fields.as_slice(),
            [nir::Term::FunctionValue {arguments, ..}] if arguments == &[nir::Term::Primitive(nir::Primitive::Bool)])
    )), "{}", instance.key);
}

#[test]
fn runtime_staged_callback_aggregates_prepare_enum_producers() {
    let package = compile(
        "typealias Type=.Type'builtin;
        enum Config {some(comptime fn()->Type),none}
        comptime fn Selected()->Type where T {T}
        comptime fn make()->Config where T {Config.some(Selected<T>)}
        fn discard(comptime config:Config,value:T)->T where T {value}
        fn forward(value:T)->T where T {discard<T>(make<T>(),value)}
        fn run()->bool {forward<bool>(true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    let instance = graph
        .functions
        .iter()
        .find(|instance| instance.function.name == "discard")
        .unwrap();
    assert!(instance.key.arguments.iter().any(|argument| matches!(argument,
        nir::Term::NominalValue {variant, fields, ..}
        if *variant == Some(0) && matches!(fields.as_slice(),
            [nir::Term::FunctionValue {arguments, ..}] if arguments == &[nir::Term::Primitive(nir::Primitive::Bool)])
    )), "{}", instance.key);
}

#[test]
fn runtime_staged_newtypes_keep_wrapped_value_identity() {
    let package = compile(
        "newtype Flag = bool;
        fn discard(comptime flag:Flag)->bool {true}
        fn run()->bool {discard(true.as(Flag))}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn runtime_staged_newtypes_preserve_defaults_nested_identity_and_packet_delivery() {
    let prefix="newtype Flag=bool;newtype Outer=Flag;
        comptime fn make()->Outer {false.as(Flag).as(Outer)}
        comptime fn packet()->Tuple<Outer> {(make(),)}
        fn read(comptime .outer:Outer=true.as(Flag).as(Outer),comptime .alias:Outer=outer)->bool {alias.as(Flag).as(bool)}
        fn positional(comptime outer:Outer)->bool {read(.outer=outer)}";
    for (body, expected) in [
        ("read()", true),
        ("read(.outer=make())", false),
        ("positional(...packet())", false),
    ] {
        let package =
            execution_support::compile_with_tuple(&format!("{prefix} fn run()->bool {{{body}}}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 200_000).unwrap(),
            Value::Bool(expected)
        );
        let graph = luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
        assert!(graph.functions.iter().any(|instance|instance.key.arguments.iter().any(|argument|matches!(argument,
            nir::Term::NominalValue {fields,..} if matches!(fields.as_slice(),[nir::Term::NominalValue {..}])
        ))));
    }
}

#[test]
fn runtime_staged_newtypes_prepare_generic_producers() {
    let package = compile(
        "newtype Wrapper<T> = bool;
        comptime fn make()->Wrapper<T> where T {true.as(Wrapper<T>)}
        fn discard(comptime wrapped:Wrapper<T>,value:T)->T where T {value}
        fn forward(value:T)->T where T {discard<T>(make<T>(),value)}
        fn run()->bool {forward<bool>(false)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn runtime_staged_newtypes_keep_callback_dependencies() {
    let package=compile("typealias Type=.Type'builtin;typealias Selector=comptime fn()->Type;
        newtype Callback=Selector;
        comptime fn Selected()->Type where T {T}
        comptime fn Wrapped(callback:Callback)->Type {callback.as(Selector)()}
        comptime fn make()->Callback {Selected<bool>.as(Callback)}
        fn consume(comptime callback:Callback,value:defer Wrapped(callback))->defer Wrapped(callback) {value}
        fn run()->bool {consume(make(),true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn fixed_runtime_signatures_forward_comptime_callbacks_into_open_bodies() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime select:comptime fn()->Type,value:bool)->bool {consume(select,value)}
        fn outer(comptime select:comptime fn()->Type,value:bool)->bool {forward(select,value)}
        fn run()->bool {outer(Boolean,true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn fixed_runtime_signatures_propagate_in_reverse_declaration_order_and_defaults() {
    let package=compile("typealias Type=.Type'builtin;
        fn outer(comptime .select:comptime fn()->Type=Boolean,.value:bool=true)->bool {forward(select,value)}
        fn forward(comptime select:comptime fn()->Type,value:bool)->bool {consume(select,value)}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        comptime fn Boolean()->Type {bool}
        fn run()->bool {outer()}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn fixed_runtime_signatures_check_actual_staged_domains() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(comptime flag:bool,value:bool)->bool {consume(flag,value)}";
    let package = compile(&format!("{prefix} fn run()->bool {{forward(true,true)}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    for entry in [
        "fn bad()->bool {forward(false,true)}",
        "fn bad(flag:bool)->bool {forward(flag,true)}",
    ] {
        assert!(
            compile_with_protocols(&format!("{prefix} {entry}"), &Default::default()).is_err(),
            "{entry}"
        );
    }
}

#[test]
fn fixed_runtime_signatures_do_not_skip_definition_errors() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {bool}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn unused(comptime flag:bool,value:bool)->bool {let wrong:.u8'builtin=true;consume(flag,value)}
        fn run()->bool {true}";
    assert!(compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn fixed_runtime_signatures_keep_independent_calls_available_as_values() {
    for (body, expected) in [
        ("consume(true,value)", true),
        ("consume(true,unused)", false),
    ] {
        let package = compile(&format!(
            "typealias Type=.Type'builtin;
            comptime fn Pick(flag:bool)->Type {{bool}}
            fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {{value}}
            fn independent(comptime unused:bool,value:bool)->bool {{{body}}}
            fn run()->bool {{let invoke=independent;invoke(false,true)}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    }
}

#[test]
fn fixed_runtime_signatures_keep_generic_and_runtime_control_flow() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn generic(comptime flag:bool,value:T)->T where T {consume(flag,value)}
        fn conditional(comptime flag:bool,value:bool)->bool {if value {generic<bool>(flag,value)} else {false}}
        fn yes()->bool {conditional(true,true)}
        fn no()->bool {conditional(true,false)}");
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_local_aliases_retain_completed_inputs_in_open_bodies() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {let alias=flag;consume(alias,value)}
        fn run()->bool {forward(true,true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_local_aliases_forward_through_fixed_signatures_and_scopes() {
    for body in [
        "let alias=flag;let again=alias;consume(again,value)",
        "let alias=flag;do {let alias=false;};consume(alias,value)",
    ] {
        let package = compile(&format!(
            "typealias Type=.Type'builtin;
            comptime fn Pick(flag:bool)->Type {{if flag {{bool}} else {{.u8'builtin}}}}
            fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {{value}}
            fn forward(comptime flag:bool,value:bool)->bool {{{body}}}
            fn outer(comptime flag:bool,value:bool)->bool {{forward(flag,value)}}
            fn run()->bool {{outer(true,true)}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 200_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    }
}

#[test]
fn staged_local_aliases_preserve_named_callable_identity() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime select:comptime fn()->Type,value:bool)->bool {let alias=select;let again=alias;consume(again,value)}
        fn run()->bool {forward(Boolean,true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_local_aliases_do_not_freeze_runtime_inputs() {
    let failure = compile_with_protocols(
        "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(flag:bool,value:bool)->bool {let alias=flag;consume(alias,value)}
        fn run()->bool {forward(true,true)}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.to_string().contains("comptime"), "{failure}");
}

#[test]
fn staged_local_aliases_invalidate_escaped_storage() {
    let failure=compile_with_protocols("typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(comptime flag:bool,value:bool)->bool {let alias=flag;let pointer=alias.ref;consume(alias,value)}
        fn run()->bool {forward(true,true)}",&Default::default()).unwrap_err();
    assert!(failure.to_string().contains("not available"), "{failure}");
}

#[test]
fn staged_local_aliases_destructure_completed_tuple_inputs() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(comptime pair:Tuple<bool,bool>,value:bool)->bool {let (first,second)=pair;consume(first,value)}
        fn run()->bool {forward((true,false),true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_local_aliases_reject_stale_fields_after_writes() {
    for mutation in [
        "alias.flag=false;",
        "do {alias.flag=false;};",
        "let pointer=alias.ref;",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            struct Config {{flag:bool}}
            comptime fn Pick(config:Config)->Type {{if config.flag {{bool}} else {{.u8'builtin}}}}
            fn consume(comptime config:Config,value:defer Pick(config))->defer Pick(config) {{value}}
            fn forward(comptime config:Config,value:bool)->bool {{let alias=config;{mutation}consume(alias,value)}}
            fn run()->bool {{forward(Config {{flag:true}},true)}}");
        let failure = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            failure.to_string().contains("not available"),
            "{mutation}: {failure}"
        );
    }
}

#[test]
fn staged_local_aliases_keep_copied_values_after_source_escape() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(comptime flag:bool,value:bool)->bool {let alias=flag;let pointer=flag.ref;consume(alias,value)}
        fn run()->bool {forward(true,true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_local_projections_feed_required_type_computations() {
    let package=compile("typealias Type=.Type'builtin;
        struct Inner {flag:bool}
        struct Config {inner:Inner}
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(comptime config:Config,value:bool)->bool {let flag=config.inner.flag;consume(flag,value)}
        fn run()->bool {forward(Config {inner:Inner {flag:true}},true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_local_projections_reject_stale_root_values() {
    let failure=compile_with_protocols("typealias Type=.Type'builtin;
        struct Config {flag:bool}
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(comptime config:Config,value:bool)->bool {config.flag=false;let flag=config.flag;consume(flag,value)}
        fn run()->bool {forward(Config {flag:true},true)}",&Default::default()).unwrap_err();
    assert!(failure.to_string().contains("not available"), "{failure}");
}

#[test]
fn staged_local_projections_preserve_callable_fields() {
    let package=compile("typealias Type=.Type'builtin;
        struct Config {select:comptime fn()->Type}
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Config,value:bool)->bool {let select=config.select;consume(select,value)}
        fn run()->bool {forward(Config {select:Boolean},true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_local_projections_preserve_anonymous_field_labels() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {left:false,right:true};
        typealias Config=SAMPLE'type;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(comptime config:Config,value:bool)->bool {let flag=config.right;consume(flag,value)}
        fn run()->bool {forward(SAMPLE,true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_configuration_alias_chains_keep_callback_fields() {
    for body in [
        "let alias=config;let select=alias.select;consume(select,value)",
        "let alias=config;let again=alias;let select=again.select;consume(select,value)",
        "let alias=config;do {let select=alias.select;consume(select,value)}",
    ] {
        let package=compile(&format!("typealias Type=.Type'builtin;
            struct Config {{select:comptime fn()->Type}}
            comptime fn Boolean()->Type {{bool}}
            fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {{value}}
            fn forward(comptime config:Config,value:bool)->bool {{{body}}}
            fn run()->bool {{forward(Config {{select:Boolean}},true)}}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 200_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    }
}

#[test]
fn staged_configuration_alias_chains_keep_actual_callback_domains() {
    let failure=compile_with_protocols("typealias Type=.Type'builtin;
        struct Config {select:comptime fn()->Type}
        comptime fn Byte()->Type {.u8'builtin}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Config,value:bool)->bool {let alias=config;let select=alias.select;consume(select,value)}
        fn run()->bool {forward(Config {select:Byte},true)}",&Default::default()).unwrap_err();
    assert!(failure.to_string().contains("type"), "{failure}");
}

#[test]
fn staged_input_destructuring_preserves_callable_phase() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>,value:bool)->bool {let (select,unused)=config;consume(select,value)}
        fn run()->bool {forward((Boolean,false),true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_input_destructuring_handles_nested_callback_packets() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<Tuple<comptime fn()->Type,bool>,bool>,value:bool)->bool {let ((select,unused),other)=config;consume(select,value)}
        fn run()->bool {forward(((Boolean,false),true),true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_input_destructuring_preserves_record_callback_fields() {
    let package=compile("typealias Type=.Type'builtin;
        struct Config {select:comptime fn()->Type}
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Config,value:bool)->bool {let {select}=config;consume(select,value)}
        fn run()->bool {forward(Config {select:Boolean},true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_input_destructuring_does_not_promote_runtime_packets() {
    let failure=execution_support::compile_with_tuple_protocols("typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(comptime unused:bool,config:Tuple<bool,bool>,value:bool)->bool {let (flag,other)=config;consume(flag,value)}
        fn run()->bool {forward(true,(true,false),true)}",&Default::default()).unwrap_err();
    assert!(failure.to_string().contains("comptime"), "{failure}");
}

#[test]
fn staged_mixed_destructuring_delivers_runtime_fields() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>)->bool {let (select,value)=config;consume(select,value)}
        fn yes()->bool {forward((Boolean,true))}
        fn no()->bool {forward((Boolean,false))}");
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_mixed_destructuring_keeps_runtime_writes_after_initialization() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>)->bool {let (select,value)=config;let pointer=value.ref;pointer.* = false;consume(select,value)}
        fn run()->bool {forward((Boolean,true))}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(false)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_mixed_destructuring_preserves_nominal_runtime_fields() {
    let package=compile("typealias Type=.Type'builtin;
        struct Payload {flag:bool}
        struct Config {select:comptime fn()->Type,payload:Payload}
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Config)->bool {let {select,payload}=config;consume(select,payload.flag)}
        fn run()->bool {forward(Config {select:Boolean,payload:Payload {flag:true}})}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_mixed_destructuring_preserves_nested_runtime_fields() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,Tuple<bool,bool>>)->bool {let (select,(left,right))=config;consume(select,left and not right)}
        fn run()->bool {forward((Boolean,(true,false)))}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_mixed_destructuring_keeps_runtime_closure_captures() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>)->bool {let (select,value)=config;let read=|| value;consume(select,read())}
        fn run()->bool {forward((Boolean,true))}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_mixed_destructuring_checks_actual_field_domains() {
    let failure=execution_support::compile_with_tuple_protocols("typealias Type=.Type'builtin;
        comptime fn Byte()->Type {.u8'builtin}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>)->bool {let (select,value)=config;consume(select,value)}
        fn run()->bool {forward((Byte,true))}",&Default::default()).unwrap_err();
    assert!(failure.to_string().contains("type"), "{failure}");
}

#[test]
fn staged_pattern_wildcard_consumes_completed_callback() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn forward(comptime select:comptime fn()->Type,value:defer select())->defer select() {let _=select;value}
        fn run()->bool {forward(Boolean,true)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn staged_mixed_runtime_subpatterns_keep_exhaustive_alternatives() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>,value:bool)->bool {let (select,(true or false))=config;consume(select,value)}
        fn yes()->bool {forward((Boolean,true),true)}
        fn no()->bool {forward((Boolean,false),false)}");
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_mixed_let_else_preserves_success_and_failure() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>)->bool {let (select,true)=config else {return false;};consume(select,true)}
        fn yes()->bool {forward((Boolean,true))}
        fn no()->bool {forward((Boolean,false))}");
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_mixed_let_else_keeps_failure_calls_at_runtime() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn failure()->bool {false}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>)->bool {let (select,true)=config else {return failure();};consume(select,true)}
        fn yes()->bool {forward((Boolean,true))}
        fn no()->bool {forward((Boolean,false))}");
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_mixed_let_else_checks_divergence_and_unused_definition_errors() {
    for failure in ["false", "let wrong:.u8'builtin=true;return false;"] {
        let source=format!("typealias Type=.Type'builtin;
            fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {{value}}
            fn unused(comptime config:Tuple<comptime fn()->Type,bool>)->bool {{let (select,true)=config else {{{failure}}};consume(select,true)}}
            fn run()->bool {{true}}");
        assert!(
            execution_support::compile_with_tuple_protocols(&source, &Default::default()).is_err(),
            "{failure}"
        );
    }
}

#[test]
fn staged_let_else_failure_storage_does_not_invalidate_success_inputs() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime enabled:bool,value:defer Pick(enabled))->defer Pick(enabled) {value}
        fn forward(comptime enabled:bool,ready:bool)->bool {
            let true=ready else {let pointer=enabled.ref;pointer.* = false;return false;};
            consume(enabled,true)
        }
        fn yes()->bool {forward(true,true)}
        fn no()->bool {forward(true,false)}",
    );
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_let_else_local_loop_does_not_escape_failure_scope() {
    let package=compile("typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime enabled:bool,value:defer Pick(enabled))->defer Pick(enabled) {value}
        fn forward(comptime enabled:bool,ready:bool)->bool {
            let true=ready else {while true {break;}let pointer=enabled.ref;pointer.* = false;return false;};
            consume(enabled,true)
        }
        fn yes()->bool {forward(true,true)}
        fn no()->bool {forward(true,false)}");
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_let_else_outward_loop_transfers_keep_invalidations() {
    for body in [
        "while true {let true=ready else {let pointer=enabled.ref;pointer.* = false;break;};break;}consume(enabled,true)",
        "var pending=ready;while true {let true=pending else {let pointer=enabled.ref;pointer.* = false;pending=true;continue;};break;}consume(enabled,true)",
        "let true=ready else {let pointer=enabled.ref;pointer.* = false;return consume(enabled,true);};consume(enabled,true)",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Pick(flag:bool)->Type {{if flag {{bool}} else {{.u8'builtin}}}}
            fn consume(comptime enabled:bool,value:defer Pick(enabled))->defer Pick(enabled) {{value}}
            fn forward(comptime enabled:bool,ready:bool)->bool {{{body}}}
            fn run()->bool {{forward(true,false)}}");
        let failure = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            failure.to_string().contains("not available"),
            "{body}: {failure}"
        );
    }
}

#[test]
fn staged_mixed_guards_preserve_runtime_conditions() {
    let package = execution_support::compile_with_tuple(
        "typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn accept(value:bool)->bool {value}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>,ready:bool)->bool {
            let (select,value) if accept(ready)=config else {return false;};consume(select,value)
        }
        fn yes()->bool {forward((Boolean,true),true)}
        fn no()->bool {forward((Boolean,true),false)}",
    );
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_mixed_guards_observe_candidates_and_staged_callbacks() {
    for guard in ["accept(value)", "consume(select,value)"] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Boolean()->Type {{bool}}
            fn accept(value:bool)->bool {{value}}
            fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {{value}}
            fn forward(comptime config:Tuple<comptime fn()->Type,bool>)->bool {{
                let (select,value) if {guard}=config else {{return false;}};consume(select,value)
            }}
            fn yes()->bool {{forward((Boolean,true))}}
            fn no()->bool {{forward((Boolean,false))}}");
        let package = execution_support::compile_with_tuple(&source);
        for (entry, expected) in [("yes", true), ("no", false)] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 200_000).unwrap(),
                Value::Bool(expected),
                "{guard}"
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn staged_mixed_guards_keep_phase_only_discard_control_flow() {
    let package = compile(
        "typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn accept(value:bool)->bool {value}
        fn forward(comptime select:comptime fn()->Type,ready:bool)->defer select() {
            let _ if accept(ready)=select else {return false;};true
        }
        fn yes()->bool {forward(Boolean,true)}
        fn no()->bool {forward(Boolean,false)}",
    );
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_mixed_guards_reject_invalid_unused_definitions() {
    for guard in ["missing", "7", "value.ref"] {
        let source=format!("typealias Type=.Type'builtin;
            fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {{value}}
            fn unused(comptime config:Tuple<comptime fn()->Type,bool>)->bool {{
                let (select,value) if {guard}=config else {{return false;}};consume(select,value)
            }}
            fn run()->bool {{true}}");
        assert!(
            execution_support::compile_with_tuple_protocols(&source, &Default::default()).is_err(),
            "{guard}"
        );
    }
}

#[test]
fn staged_mixed_pattern_computations_preserve_runtime_phase() {
    for pattern in [
        "(select,(value if accept(value)))",
        "(select,(_ and accept(ready) is true))",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Boolean()->Type {{bool}}
            fn accept(value:bool)->bool {{value}}
            fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {{value}}
            fn forward(comptime config:Tuple<comptime fn()->Type,bool>,ready:bool)->bool {{
                let {pattern}=config else {{return false;}};consume(select,true)
            }}
            fn yes()->bool {{forward((Boolean,true),true)}}
            fn no()->bool {{forward((Boolean,false),false)}}");
        let package = execution_support::compile_with_tuple(&source);
        for (entry, expected) in [("yes", true), ("no", false)] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 200_000).unwrap(),
                Value::Bool(expected),
                "{pattern}"
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn staged_mixed_pattern_computations_keep_computed_runtime_bindings() {
    let package = execution_support::compile_with_tuple(
        "typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn invert(value:bool)->bool {not value}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>)->bool {
            let (select,(value and invert(value) is result))=config;consume(select,result)
        }
        fn yes()->bool {forward((Boolean,false))}
        fn no()->bool {forward((Boolean,true))}",
    );
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_mixed_pattern_computations_do_not_promote_runtime_results() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        comptime fn Pick(value:bool)->Type {if value {bool} else {.u8'builtin}}
        fn observe(value:bool)->bool {value}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn selected(comptime ready:bool,value:defer Pick(ready))->defer Pick(ready) {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>)->bool {
            let (select,(value and observe(value) is computed))=config;
            selected(computed,consume(select,true))
        }
        fn run()->bool {forward((Boolean,true))}";
    for source in [
        source.to_string(),
        source.replace(
            "fn run()->bool {forward((Boolean,true))}",
            "fn run()->bool {true}",
        ),
    ] {
        let failure = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_err();
        assert!(
            failure.to_string().contains("comptime") || failure.to_string().contains("staged"),
            "{failure}"
        );
    }
}

#[test]
fn staged_mixed_pattern_computations_check_unused_definition_errors() {
    for computation in ["accept(7)", "accept(unknown)"] {
        let source=format!("typealias Type=.Type'builtin;
            fn accept(value:bool)->bool {{value}}
            fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {{value}}
            fn unused(comptime config:Tuple<comptime fn()->Type,bool>)->bool {{
                let (select,(value if {computation}))=config else {{return false;}};consume(select,true)
            }}
            fn run()->bool {{true}}");
        assert!(
            execution_support::compile_with_tuple_protocols(&source, &Default::default()).is_err(),
            "{computation}"
        );
    }
}

#[test]
fn staged_pattern_provenance_requires_every_alternative_to_supply_input() {
    for pattern in [
        "((value and observe(value) is computed) if ready) or (computed and observe(computed) is value)",
        "((computed and observe(computed) is value) if ready) or (value and observe(value) is computed)",
    ] {
        for use_value in ["computed", "do {let copied=computed;copied}"] {
            let source=format!("typealias Type=.Type'builtin;
                fn observe(value:bool)->bool {{value}}
                comptime fn Pick(value:bool)->Type {{if value {{bool}} else {{.u8'builtin}}}}
                fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {{value}}
                fn selected(comptime ready:bool,value:defer Pick(ready))->defer Pick(ready) {{value}}
                fn unused(comptime config:Tuple<comptime fn()->Type,bool>,ready:bool)->bool {{
                    let (select,({pattern}))=config;
                    selected({use_value},consume(select,true))
                }}
                fn run()->bool {{true}}");
            let failure =
                execution_support::compile_with_tuple_protocols(&source, &Default::default())
                    .unwrap_err();
            assert!(
                failure.to_string().contains("unstaged local"),
                "{pattern}: {failure}"
            );
        }
    }
}

#[test]
fn staged_pattern_provenance_keeps_structural_siblings_available() {
    let package=execution_support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn observe(value:bool)->bool {value}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<comptime fn()->Type,bool>,ready:bool)->bool {
            let (select,(((first and observe(first) is second) if ready) or (second and observe(second) is first)))=config;
            let copied=select;consume(copied,second)
        }
        fn yes()->bool {forward((Boolean,true),true)}
        fn no()->bool {forward((Boolean,false),false)}");
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_nested_packet_destructuring_preserves_callback_chains() {
    let package = execution_support::compile_with_tuple(
        "typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<Tuple<comptime fn()->Type>,bool>)->bool {
            let (callbacks,value)=config;
            let (select,)=callbacks;
            consume(select,value)
        }
        fn yes()->bool {forward(((Boolean,),true))}
        fn no()->bool {forward(((Boolean,),false))}",
    );
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_nested_packet_destructuring_supports_multiple_layers() {
    let package = execution_support::compile_with_tuple(
        "typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<Tuple<Tuple<comptime fn()->Type,comptime fn()->Type>>,bool>)->bool {
            let (nested,value)=config;
            let (callbacks,)=nested;
            let (first,second)=callbacks;
            let selected=second;
            consume(first,consume(selected,value))
        }
        fn yes()->bool {forward((((Boolean,Boolean),),true))}
        fn no()->bool {forward((((Boolean,Boolean),),false))}",
    );
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_nested_packet_destructuring_checks_callback_identity() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        comptime fn Byte()->Type {.u8'builtin}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<Tuple<comptime fn()->Type,comptime fn()->Type>,bool>)->bool {
            let (callbacks,value)=config;let (first,second)=callbacks;consume(second,value)
        }
        fn run()->bool {forward(((Boolean,Byte),true))}";
    let failure =
        execution_support::compile_with_tuple_protocols(source, &Default::default()).unwrap_err();
    assert!(failure.to_string().contains("type"), "{failure}");
}

#[test]
fn staged_nested_guarded_packets_preserve_control_flow() {
    let package = execution_support::compile_with_tuple(
        "typealias Type=.Type'builtin;
        comptime fn Boolean()->Type {bool}
        fn accept(value:bool)->bool {value}
        fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {value}
        fn forward(comptime config:Tuple<Tuple<comptime fn()->Type>,bool>,ready:bool)->bool {
            let (callbacks,value)=config;
            let (select,) if accept(ready)=callbacks else {return false;};
            consume(select,value)
        }
        fn yes()->bool {forward(((Boolean,),true),true)}
        fn no()->bool {forward(((Boolean,),true),false)}",
    );
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_nested_guarded_packets_retain_other_runtime_references() {
    for residual in ["callbacks;", "let copied=callbacks;copied;"] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Boolean()->Type {{bool}}
            fn accept(value:bool)->bool {{value}}
            fn consume(comptime select:comptime fn()->Type,value:defer select())->defer select() {{value}}
            fn forward(comptime config:Tuple<Tuple<comptime fn()->Type>,bool>,ready:bool)->bool {{
                let (callbacks,value)=config;
                let (select,) if accept(ready)=callbacks else {{return false;}};
                {residual}consume(select,value)
            }}
            fn run()->bool {{forward(((Boolean,),true),true)}}");
        let failure = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_err();
        assert!(
            failure.to_string().contains("comptime-only"),
            "{residual}: {failure}"
        );
    }
}

#[test]
fn staged_observed_pattern_bindings_keep_subject_values() {
    for pattern in [
        "(select,(flag if accept(flag)))",
        "(select,(flag and accept(flag) is true))",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Pick(value:bool)->Type {{if value {{bool}} else {{.u8'builtin}}}}
            fn accept(value:bool)->bool {{value}}
            fn consume(comptime select:comptime fn(bool)->Type,comptime flag:bool,value:defer select(flag))->defer select(flag) {{value}}
            fn forward(comptime config:Tuple<comptime fn(bool)->Type,bool>)->bool {{
                let {pattern}=config else {{return false;}};
                consume(select,flag,true)
            }}
            fn run()->bool {{forward((Pick,true))}}");
        let package = execution_support::compile_with_tuple(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 200_000).unwrap(),
            Value::Bool(true),
            "{pattern}"
        );
        luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    }
}

#[test]
fn staged_observed_pattern_bindings_preserve_failure_and_alias_storage() {
    for pattern in [
        "(select,(flag if accept(ready)))",
        "(select,(flag and accept(ready) is true))",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Pick(value:bool)->Type {{if value {{bool}} else {{.u8'builtin}}}}
            fn accept(value:bool)->bool {{value}}
            fn consume(comptime select:comptime fn(bool)->Type,comptime flag:bool,value:defer select(flag))->defer select(flag) {{value}}
            fn forward(comptime config:Tuple<comptime fn(bool)->Type,bool>,ready:bool)->bool {{
                let {pattern}=config else {{return false;}};
                let saved=flag;let pointer=flag.ref;pointer.* = false;
                consume(select,saved,true)
            }}
            fn yes()->bool {{forward((Pick,true),true)}}
            fn no()->bool {{forward((Pick,true),false)}}");
        let package = execution_support::compile_with_tuple(&source);
        for (entry, expected) in [("yes", true), ("no", false)] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 200_000).unwrap(),
                Value::Bool(expected),
                "{pattern}"
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
        let invalid = source.replace("consume(select,saved,true)", "consume(select,flag,true)");
        let failure =
            execution_support::compile_with_tuple_protocols(&invalid, &Default::default())
                .unwrap_err();
        assert!(
            failure.to_string().contains("not available"),
            "{pattern}: {failure}"
        );
    }
}

#[test]
fn staged_alternative_bindings_keep_agreed_completed_values() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime select:comptime fn(bool)->Type,comptime flag:bool,value:defer select(flag))->defer select(flag) {value}
        fn forward(comptime config:Tuple<comptime fn(bool)->Type,Tuple<bool,bool>>,ready:bool)->bool {
            let (select,(((flag,_) if ready) or (_,flag)))=config;
            consume(select,flag,true)
        }
        fn yes()->bool {forward((Pick,(true,true)),true)}
        fn other()->bool {forward((Pick,(true,true)),false)}";
    let package = execution_support::compile_with_tuple(source);
    for entry in ["yes", "other"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_alternative_bindings_remap_simultaneously_and_reject_conflicts() {
    for pattern in [
        "((flag,other) if ready) or (other,flag)",
        "((other,flag) if ready) or (flag,other)",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Pick(flag:bool)->Type {{if flag {{bool}} else {{.u8'builtin}}}}
            fn consume(comptime select:comptime fn(bool)->Type,comptime flag:bool,value:defer select(flag))->defer select(flag) {{value}}
            fn forward(comptime config:Tuple<comptime fn(bool)->Type,Tuple<bool,bool>>,ready:bool)->bool {{
                let (select,({pattern}))=config;consume(select,flag,other)
            }}
            fn yes()->bool {{forward((Pick,(true,true)),true)}}
            fn other()->bool {{forward((Pick,(true,true)),false)}}");
        let package = execution_support::compile_with_tuple(&source);
        for entry in ["yes", "other"] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 200_000).unwrap(),
                Value::Bool(true),
                "{pattern}"
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
        for fields in ["(true,false)", "(false,true)"] {
            let conflicting = source.replace("(true,true)", fields);
            let failure =
                execution_support::compile_with_tuple_protocols(&conflicting, &Default::default())
                    .unwrap_err();
            assert!(
                failure.to_string().contains("not available"),
                "{pattern}: {fields}: {failure}"
            );
        }
        let invalidated = source.replace(
            "consume(select,flag,other)",
            "let pointer=flag.ref;pointer.* = false;consume(select,flag,other)",
        );
        let failure =
            execution_support::compile_with_tuple_protocols(&invalidated, &Default::default())
                .unwrap_err();
        assert!(failure.to_string().contains("not available"), "{failure}");
    }
}

#[test]
fn staged_alternative_bindings_do_not_inherit_computed_candidate_values() {
    for pattern in [
        "(flag if ready) or (_ and observe(true) is flag)",
        "((_ and observe(true) is flag) if ready) or flag",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Pick(flag:bool)->Type {{if flag {{bool}} else {{.u8'builtin}}}}
            fn observe(value:bool)->bool {{value}}
            fn consume(comptime select:comptime fn(bool)->Type,comptime flag:bool,value:defer select(flag))->defer select(flag) {{value}}
            fn unused(comptime config:Tuple<comptime fn(bool)->Type,bool>,ready:bool)->bool {{
                let (select,({pattern}))=config;consume(select,flag,true)
            }}
            fn run()->bool {{true}}");
        let failure = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_err();
        assert!(
            failure.to_string().contains("unstaged local"),
            "{pattern}: {failure}"
        );
    }
}

#[test]
fn staged_alternative_discriminants_exclude_impossible_candidates() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime select:comptime fn(bool)->Type,comptime flag:bool,value:defer select(flag))->defer select(flag) {value}
        fn forward(comptime config:Tuple<comptime fn(bool)->Type,Tuple<bool,bool,bool>>,ready:bool)->bool {
            let (select,(((true,flag,_) if ready) or (false,_,flag)))=config else {return false;};
            consume(select,flag,true)
        }
        fn first()->bool {forward((Pick,(true,true,false)),true)}
        fn second()->bool {forward((Pick,(false,false,true)),true)}
        fn failed()->bool {forward((Pick,(true,true,false)),false)}";
    let package = execution_support::compile_with_tuple(source);
    for (entry, expected) in [("first", true), ("second", true), ("failed", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_alternative_discriminants_use_scalar_matching_equality() {
    for (ty, first, second, input) in [
        (".i32'builtin", "-1", "2", "-1"),
        (".u64'builtin", "1", "2", "1"),
        (".char'builtin", "'a'", "'b'", "'a'"),
        (".f32'builtin", "0.0", "1.0", "-0.0"),
        (".f64'builtin", "0.0", "1.0", "-0.0"),
    ] {
        let source=format!("typealias Type=.Type'builtin;typealias Tag={ty};
            comptime fn Pick(flag:bool)->Type {{if flag {{bool}} else {{.u8'builtin}}}}
            fn consume(comptime select:comptime fn(bool)->Type,comptime flag:bool,value:defer select(flag))->defer select(flag) {{value}}
            fn forward(comptime config:Tuple<comptime fn(bool)->Type,Tuple<Tag,bool,bool>>)->bool {{
                let (select,(({first},flag,_) or ({second},_,flag)))=config else {{return false;}};
                consume(select,flag,true)
            }}
            fn run()->bool {{forward((Pick,({input},true,false)))}}");
        let package = execution_support::compile_with_tuple(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 200_000).unwrap(),
            Value::Bool(true),
            "{ty}"
        );
        luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    }
}

#[test]
fn staged_alternative_discriminants_do_not_match_nan_with_itself() {
    for ty in [".f32'builtin", ".f64'builtin"] {
        let source=format!("typealias Type=.Type'builtin;typealias Tag={ty};
            comptime fn Pick(flag:bool)->Type {{if flag {{bool}} else {{.u8'builtin}}}}
            fn consume(comptime select:comptime fn(bool)->Type,comptime flag:bool,value:defer select(flag))->defer select(flag) {{value}}
            fn forward(comptime config:Tuple<comptime fn(bool)->Type,Tuple<Tag,bool,bool>>)->bool {{
                let (select,((Tag.NaN,flag,_) or (_,_,flag)))=config;
                consume(select,flag,true)
            }}
            fn run()->bool {{forward((Pick,(Tag.NaN,false,true)))}}");
        let package = execution_support::compile_with_tuple(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 200_000).unwrap(),
            Value::Bool(true),
            "{ty}"
        );
        luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    }
}

#[test]
fn staged_enum_payloads_preserve_selected_input_evidence() {
    let source="typealias Type=.Type'builtin;
        enum Choice {left(bool),right(bool)}
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime select:comptime fn(bool)->Type,comptime flag:bool,value:defer select(flag))->defer select(flag) {value}
        fn forward(comptime config:Tuple<comptime fn(bool)->Type,Choice>)->bool {
            let (select,(.left(flag) or .right(flag)))=config;consume(select,flag,true)
        }
        fn left()->bool {forward((Pick,Choice.left(true)))}
        fn right()->bool {forward((Pick,Choice.right(true)))}";
    let package = execution_support::compile_with_tuple(source);
    for entry in ["left", "right"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn staged_enum_payloads_preserve_generic_layout_and_reject_wrong_values() {
    let source = "typealias Type=.Type'builtin;
        enum Choice where T {left(T,bool),right(bool,T)}
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(comptime choice:Choice<bool>)->bool {
            let (.left(flag,_) or .right(_,flag))=choice;consume(flag,true)
        }
        fn left()->bool {forward(Choice<bool>.left(true,false))}
        fn right()->bool {forward(Choice<bool>.right(false,true))}";
    let package = compile(source);
    for entry in ["left", "right"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 200_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
    for invalid in [
        source.replace("left(true,false)", "left(false,true)"),
        source.replace("right(false,true)", "right(true,false)"),
        source.replace(
            "forward(Choice<bool>.left(true,false))",
            "forward(Choice<.u8'builtin>.left(1,false))",
        ),
    ] {
        let failure = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
        assert!(failure.to_string().contains("type"), "{failure}");
    }
}

#[test]
fn staged_enum_payloads_do_not_reuse_values_after_storage_escape() {
    let source = "typealias Type=.Type'builtin;
        enum Choice {left(bool),right(bool)}
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn consume(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn forward(comptime choice:Choice)->bool {
            let (.left(flag) or .right(flag))=choice;
            let pointer=flag.ref;pointer.* = false;consume(flag,true)
        }
        fn run()->bool {forward(Choice.left(true))}";
    let failure = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(failure.to_string().contains("not available"), "{failure}");
}

#[test]
fn open_runtime_spreads_do_not_force_unknown_staged_inputs() {
    for packet in [
        "do {(true,false)}",
        "if flag {(true,false)} else {(false,true)}",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
             comptime fn Select(flag:bool)->Type {{bool}}
             fn first(comptime choice:bool,a:bool,b:bool)->defer Select(choice) {{a}}
             fn forward(comptime choice:bool,flag:bool)->defer Select(choice) {{
                 first(choice,...{packet})
             }}
             fn run()->bool {{forward(true,true)}}"
        );
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{packet}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_runtime_spreads_preserve_producer_order_and_early_return() {
    for result in ["bool", "defer Select(choice)"] {
        for (packet, expected, events) in [
            (
                "do {left();(true,)}",
                true,
                vec!["run", "forward", "left", "right", "first"],
            ),
            (
                "do {left();return false;(true,)}",
                false,
                vec!["run", "forward", "left"],
            ),
        ] {
            let source = format!(
                "typealias Type=.Type'builtin;
             comptime fn Select(flag:bool)->Type {{bool}}
             fn left()->bool {{true}} fn right()->bool {{false}}
             fn first(comptime choice:bool,a:bool,b:bool)->defer Select(choice) {{a}}
             fn forward(comptime choice:bool)->{result} {{
                 first(choice,...{packet},...do {{(right(),)}})
             }}
             fn run()->bool {{forward(true)}}"
            );
            let package = compile_with_protocols(&source, &Default::default())
                .unwrap_or_else(|error| panic!("{packet}: {error}"));
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(id(&package, "run"), vec![]).unwrap(),
                Value::Bool(expected)
            );
            assert_eq!(calls(&package, &vm), events);
            luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        }
    }
}

#[test]
fn open_runtime_spreads_reject_invalid_unused_definitions() {
    for call in [
        "first(runtime,...do {(true,false)})",
        "first(choice,...do {(true,7)})",
        "first(choice,...do {(true,)})",
        "first(choice,...do {(true,false,true)})",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
             comptime fn Select(flag:bool)->Type {{bool}}
             fn first(comptime choice:bool,a:bool,b:bool)->defer Select(choice) {{a}}
             fn unused(comptime choice:bool,runtime:bool)->defer Select(choice) {{{call}}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{call}: {error}"
        );
    }
}

#[test]
fn deferred_packet_returns_wait_for_each_instances_result_domain() {
    let source =
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;typealias Word=.u16'builtin;
        comptime fn Select(flag:bool)->Type {if flag {Byte} else {Word}}
        fn consume(comptime flag:bool,a:bool,b:bool)->bool {inline if flag {a} else {b}}
        fn forward(comptime flag:bool)->defer Select(flag) {
            consume(flag,...do {
                inline if flag {return 7;} else {return 8;}
                (true,false)
            })
        }
        fn byte()->Byte {forward(true)} fn word()->Word {forward(false)}";
    let definition = source.split("fn byte()").next().unwrap();
    compile_with_protocols(definition, &Default::default()).expect("deferred definition");
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    for (entry, expected) in [
        ("byte", Value::UnsignedInteger(7)),
        ("word", Value::UnsignedInteger(8)),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(vm.invoke(id(&package, entry), vec![]).unwrap(), expected);
        assert_eq!(calls(&package, &vm), vec![entry, "forward"]);
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn deferred_packet_returns_are_checked_against_the_completed_result() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Select(flag:bool)->Type {bool}
        fn consume(comptime flag:bool,a:bool,b:bool)->bool {inline if flag {a} else {b}}
        fn forward(comptime flag:bool)->defer Select(flag) {
            consume(flag,...do {return 7;(true,false)})
        }";
    // An uninvoked deferred result supplies no return-type equality yet.
    compile_with_protocols(prefix, &Default::default()).unwrap();
    let error = compile_with_protocols(
        &format!("{prefix} fn bad()->bool {{forward(true)}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(!error.contains("return outside a function"), "{error}");
    assert!(!error.contains("parsing failed"), "{error}");
}

#[test]
fn deferred_packet_return_scope_does_not_escape_into_closures_or_defaults() {
    for (helper, body) in [
        ("", "let callback=|| {if runtime {return true;};7};"),
        (
            "fn helper(.value:bool=do {return true;false})->bool {value}",
            "helper();",
        ),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            comptime fn Select(flag:bool)->Type {{bool}}
            fn consume(comptime flag:bool,a:bool,b:bool)->bool {{inline if flag {{a}} else {{b}}}}
            {helper}
            fn unused(comptime flag:bool,runtime:bool)->defer Select(flag) {{
                consume(flag,...do {{{body} (true,false)}})
            }}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{error}"
        );
    }
}

#[test]
fn diverging_spread_producers_discard_only_the_call_result_expectation() {
    for (declarations, call) in [
        (
            "fn consume(a:bool,b:bool)->bool {a}",
            "consume(...do {mark();return 7;(true,false)})",
        ),
        (
            "fn consume(a:bool,b:bool)->bool {a}",
            "do {let callback=consume;callback(...do {mark();return 7;(true,false)})}",
        ),
        (
            "fn consume(a:T,b:T)->T where T {a}",
            "consume<bool>(...do {mark();return 7;(true,false)})",
        ),
        (
            "fn consume()->bool {true}",
            "consume(...do {mark();return 7;()})",
        ),
        (
            "enum Choice {some(bool)}",
            "Choice.some(...do {mark();return 7;(true,)})",
        ),
        (
            "comptime fn schema(...properties:T)->T where T {properties}
             const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;
             fn consume(a:bool,...properties:Options)->bool {a}",
            "consume {...do {mark();return 7;(true,)},flag:mark()}",
        ),
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;fn mark()->bool {{true}}
            {declarations} fn run()->Byte {{{call}}}"
        );
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{call}: {error}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::UnsignedInteger(7)
        );
        assert_eq!(calls(&package, &vm), vec!["run", "mark"]);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn diverging_spreads_still_check_later_inputs_and_completing_paths() {
    for call in [
        "consume(...do {return 7;(true,)},7)",
        "consume(...do {return 7;(true,)})",
        "consume(...do {return 7;(true,false,true)})",
        "consume(...do {if flag {return 7;};(true,false)})",
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            fn consume(a:bool,b:bool)->bool {{a}}
            fn bad(flag:bool)->Byte {{{call}}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{call}: {error}"
        );
    }
}

#[test]
fn explicit_generic_spreads_forward_unknown_staged_inputs() {
    for packet in [
        "do {(1,2)}",
        "do {inline if flag {(1,2)} else {(3,4)}}",
        "do {(1,)},...(),...do {(2,)}",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
            fn select(comptime flag:bool,a:T,b:U)->T where T,.U:Type=T {{
                inline if flag {{a}} else {{a}}
            }}
            fn forward(comptime flag:bool)->Byte {{select<Byte>(flag,...{packet})}}
            fn run()->Byte {{forward(true)}} fn other()->Byte {{forward(false)}}"
        );
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{packet}: {error}"));
        for (entry, expected) in [
            ("run", 1),
            ("other", if packet.contains("inline") { 3 } else { 1 }),
        ] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 100_000).unwrap(),
                Value::UnsignedInteger(expected)
            );
            luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
        }
    }
}

#[test]
fn explicit_generic_spread_contexts_do_not_leak_between_calls() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        fn select(comptime flag:bool,a:T,b:T)->T where T {inline if flag {a} else {b}}
        fn forward(comptime flag:bool)->Byte {
            let ignored=select<bool>(flag,...do {(true,false)});
            select<Byte>(flag,...do {(7,9)})
        }
        fn run()->Byte {forward(true)} fn other()->Byte {forward(false)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    for (entry, expected) in [("run", 7), ("other", 9)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::UnsignedInteger(expected)
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn explicit_generic_spreads_enforce_domains_stages_and_bounds() {
    for call in [
        "select<Byte>(flag,...do {(300,2)})",
        "select<Byte>(flag,...do {(true,2)})",
        "select<Byte>(runtime,...do {(1,2)})",
        "select<Byte>(flag,...do {(1,)})",
        "select<bool>(flag,...do {(true,false)})",
    ] {
        let source = format!("typealias Byte=.u8'builtin;
            trait Allowed {{}} impl Allowed for Byte {{}}
            fn select(comptime flag:bool,a:T,b:T)->T where T:-Allowed {{inline if flag {{a}} else {{b}}}}
            fn unused(comptime flag:bool,runtime:bool)->Byte {{{call}}}");
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{call}: {error}"
        );
    }
}

#[test]
fn explicit_generic_spreads_transport_the_callers_type_binder() {
    let source = "typealias Byte=.u8'builtin;
        trait Allowed {} impl Allowed for bool {} impl Allowed for Byte {}
        fn select(comptime flag:bool,a:T,b:T)->T where T:-Allowed {inline if flag {a} else {b}}
        fn forward(comptime flag:bool,a:T,b:T)->T where T:-Allowed {
            select<T>(flag,...do {(a,b)})
        }
        fn boolean()->bool {forward<bool>(true,true,false)}
        fn byte()->Byte {forward<Byte>(false,7,9)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    for (entry, expected) in [
        ("boolean", Value::Bool(true)),
        ("byte", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    }
}

#[test]
fn mixed_literal_and_computed_spreads_preserve_each_inputs_stage() {
    for (arguments, expected_calls) in [
        ("...(flag,),...do {(left(),right())}", vec!["left", "right"]),
        ("...(flag,left()),...do {(right(),)}", vec!["left", "right"]),
        (
            "...(),...(flag,),...do {(left(),)},...(),...do {(right(),)}",
            vec!["left", "right"],
        ),
    ] {
        for target in ["select", "generic<bool>"] {
            let source = format!("fn left()->bool {{true}} fn right()->bool {{false}}
                fn select(comptime flag:bool,a:bool,b:bool)->bool {{inline if flag {{a}} else {{b}}}}
                fn generic(comptime flag:bool,a:T,b:T)->T where T {{inline if flag {{a}} else {{b}}}}
                fn middle(comptime flag:bool)->bool {{{target}({arguments})}}
                fn outer(comptime flag:bool)->bool {{middle(...(flag,))}}
                fn run()->bool {{outer(true)}} fn other()->bool {{outer(false)}}");
            let package = compile_with_protocols(&source, &Default::default())
                .unwrap_or_else(|error| panic!("{target}({arguments}): {error}"));
            for (entry, expected) in [("run", true), ("other", false)] {
                let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
                assert_eq!(
                    vm.invoke(id(&package, entry), vec![]).unwrap(),
                    Value::Bool(expected)
                );
                let mut events = vec![entry, "outer", "middle"];
                events.extend(expected_calls.iter().copied());
                events.push(if target == "select" {
                    "select"
                } else {
                    "generic"
                });
                assert_eq!(calls(&package, &vm), events);
                luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
            }
        }
    }
}

#[test]
fn mixed_spreads_do_not_promote_runtime_inputs_to_comptime() {
    for arguments in [
        "...(runtime,),...do {(true,false)}",
        "...(flag,),...do {(true,7)}",
        "...do {(runtime,)},...(true,),...do {(false,)}",
    ] {
        let source=format!("fn select(comptime flag:bool,a:bool,b:bool)->bool {{inline if flag {{a}} else {{b}}}}
            fn unused(comptime flag:bool,runtime:bool)->bool {{inline if flag {{}} select({arguments})}}");
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{arguments}: {error}"
        );
    }
}

#[test]
fn inferred_generic_spreads_forward_unknown_stage_inputs() {
    for (domain, packet, expected) in [
        ("bool", "do {(true,false)}", Value::Bool(false)),
        ("Byte", "do {(7,9)}", Value::UnsignedInteger(9)),
        (
            "Byte",
            "do {inline if flag {(1,2)} else {(7,9)}}",
            Value::UnsignedInteger(9),
        ),
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            fn select(comptime flag:bool,a:T,b:T)->T where T {{inline if flag {{a}} else {{b}}}}
            fn forward(comptime flag:bool)->{domain} {{select(...(flag,),...{packet})}}
            fn run()->{domain} {{forward(false)}}"
        );
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            expected
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn inferred_generic_spreads_use_packet_evidence_with_a_deferred_result() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Result(flag:bool,T:Type)->Type {if flag {T} else {Result(flag,T)}}
        fn left()->bool {true} fn right()->bool {false}
        fn select(comptime flag:bool,a:T,b:U)->defer Result(flag,T) where T,.U:Type=T {a}
        fn forward(comptime flag:bool)->defer Result(flag,bool) {
            let result=select(flag,...do {(left(),right())});result
        }
        fn run()->bool {forward(true)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec!["run", "forward", "left", "right", "select"]
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn inferred_generic_spreads_keep_trait_and_stage_requirements() {
    for (domain, packet, flag) in [
        ("bool", "do {(true,false)}", "runtime"),
        ("bool", "do {(true,7)}", "flag"),
        ("Byte", "do {(7,9)}", "flag"),
    ] {
        let source=format!("typealias Byte=.u8'builtin;trait Allowed {{}} impl Allowed for bool {{}}
            fn select(comptime flag:bool,a:T,b:T)->T where T:-Allowed {{inline if flag {{a}} else {{b}}}}
            fn unused(comptime flag:bool,runtime:bool)->{domain} {{select({flag},...{packet})}}");
        if domain == "Byte" {
            let allowed = format!("{source} impl Allowed for Byte {{}}");
            compile_with_protocols(&allowed, &Default::default()).unwrap();
        }
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{source}: {error}"
        );
    }
}

#[test]
fn inferred_generic_spreads_do_not_constrain_an_unreachable_call_result() {
    for (result, body) in [
        ("bool", "inline if flag {a} else {a}"),
        ("Tuple<T,bool>", "inline if flag {(b,a)} else {(b,a)}"),
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
        fn later()->bool {{true}}
        fn consume(comptime flag:bool,a:bool,b:T)->{result} where T {{{body}}}
        fn forward(comptime flag:bool)->Byte {{
            consume(flag,...do {{return 7;(true,)}},later())
        }}
        fn run()->Byte {{forward(true)}}"
        );
        let package =
            execution_support::compile_with_tuple_protocols(&source, &Default::default()).unwrap();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::UnsignedInteger(7)
        );
        assert_eq!(calls(&package, &vm), vec!["run", "forward"]);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn inferred_spread_context_fallback_does_not_accept_completing_result_mismatches() {
    for packet in ["do {(true,)}", "do {if runtime {return 7;};(true,)}"] {
        let source=format!("typealias Byte=.u8'builtin;
            fn consume(comptime flag:bool,a:bool,b:T)->bool where T {{inline if flag {{a}} else {{a}}}}
            fn unused(comptime flag:bool,runtime:bool)->Byte {{consume(flag,...{packet},true)}}");
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{error}"
        );
    }
}

#[test]
fn diverging_inferred_spreads_do_not_bypass_later_trait_obligations() {
    let source="typealias Byte=.u8'builtin;
        trait Allowed {} impl Allowed for bool {}
        fn later()->Byte {9}
        fn consume(comptime flag:bool,a:bool,b:T)->bool where T:-Allowed {inline if flag {a} else {a}}
        fn unused(comptime flag:bool)->Byte {consume(flag,...do {return 7;(true,)},later())}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
        "{error}"
    );
    // Only the missing trait proof changes. The same unreachable call is
    // otherwise well typed, so this is not merely a result-mismatch rejection.
    let allowed = format!("{source} impl Allowed for Byte {{}}");
    compile_with_protocols(&allowed, &Default::default()).unwrap();
}

#[test]
fn open_generic_spread_contexts_use_peer_input_evidence() {
    let source="typealias Byte=.u8'builtin;
        fn peer()->Byte {3}
        fn consume(comptime flag:bool,first:T,second:T,third:T)->bool where T {inline if flag {true} else {false}}
        fn forward(comptime flag:bool)->bool {consume(flag,peer(),...do {(7,9)})}
        fn run()->bool {forward(true)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec!["run", "forward", "peer", "consume"]
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
}

#[test]
fn peer_inferred_spreads_preserve_source_order_and_early_exit() {
    for (call, expected, events) in [
        (
            "consume(flag,peer(),...do {mark();(7,9)})",
            true,
            vec!["run", "forward", "peer", "mark", "consume"],
        ),
        (
            "consume(flag,...do {mark();(7,9)},peer())",
            true,
            vec!["run", "forward", "mark", "peer", "consume"],
        ),
        (
            "consume(flag,...do {mark();return false;(7,9)},peer())",
            false,
            vec!["run", "forward", "mark"],
        ),
    ] {
        let source=format!("typealias Byte=.u8'builtin;
            fn peer()->Byte {{3}} fn mark()->bool {{true}}
            fn consume(comptime flag:bool,a:T,b:T,c:T)->bool where T {{inline if flag {{true}} else {{false}}}}
            fn forward(comptime flag:bool)->bool {{{call}}}
            fn run()->bool {{forward(true)}}");
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{call}: {e}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(expected)
        );
        assert_eq!(calls(&package, &vm), events);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn peer_inference_contextualizes_untyped_packet_closures() {
    let source = "typealias Byte=.u8'builtin;
        fn peer()->Byte {3}
        fn consume(comptime flag:bool,value:T,callback:fn(T)->T)->bool where T {
            inline if flag {let ignored=callback(value);true} else {false}
        }
        fn forward(comptime flag:bool)->bool {consume(flag,peer(),...do {(|value|value,)})}
        fn run()->bool {forward(true)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let invalid = source.replace("|value|value", "|value|false");
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn open_associated_input_and_result_views_wait_for_later_subject_evidence() {
    for call in [
        "select(flag,true,owner())",
        "select(flag,...do {(true,)},owner())",
    ] {
        let source=format!("typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
            trait Source {{assoc Element:Type;}}
            impl Source for Byte {{assoc Element:Type=bool;}}
            fn owner()->Byte {{7}}
            fn select(comptime flag:bool,value:T.as(Source).Element,subject:T)->T.as(Source).Element where T:-Source {{
                inline if flag {{value}} else {{value}}
            }}
            fn forward(comptime flag:bool)->bool {{{call}}}
            fn run()->bool {{forward(true)}}");
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{call}: {e}"));
        // Trait requirements are discharged by the concrete instance graph.
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
        );
        let wrong = source.replace("assoc Element:Type=bool", "assoc Element:Type=Byte");
        assert!(compile_with_protocols(&wrong, &Default::default()).is_err());
    }
}

#[test]
fn deferred_associated_equalities_do_not_invent_a_subject_type() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        trait Source {assoc Element:Type;}
        impl Source for bool {assoc Element:Type=bool;}
        impl Source for Byte {assoc Element:Type=bool;}
        fn select(comptime flag:bool,value:T.as(Source).Element)->bool where T:-Source {
            inline if flag {true} else {false}
        }
        fn unused(comptime flag:bool)->bool {select(flag,true)}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
        "{error}"
    );
    let explicit = source.replace("select(flag,true)", "select<Byte>(flag,true)");
    compile_with_protocols(&explicit, &Default::default()).unwrap();
}

#[test]
fn open_computed_spreads_anchor_post_do_outside_packet_slots() {
    for target in ["apply", "last", "generic<bool>", "generic"] {
        let source=format!("fn input()->bool {{false}}
            fn apply(comptime flag:bool,lambda action:fn(bool)->bool,value:bool)->bool {{inline if flag {{action(value)}} else {{false}}}}
            fn last(comptime flag:bool,value:bool,lambda action:fn(bool)->bool)->bool {{inline if flag {{action(value)}} else {{false}}}}
            fn generic(comptime flag:bool,lambda action:fn(T)->bool,value:T)->bool where T {{inline if flag {{action(value)}} else {{false}}}}
            fn forward(comptime flag:bool)->bool {{{target}(...(flag,),...do {{(input(),)}}) do {{not _0}}}}
            fn run()->bool {{forward(true)}}");
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{target}: {e}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            calls(&package, &vm)
                .iter()
                .filter(|name| **name == "input")
                .count(),
            1
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_post_do_spreads_keep_early_exit_and_reject_invalid_tactics() {
    let prefix = "fn input()->bool {false}
        fn callback(value:bool)->bool {not value}
        fn apply(comptime flag:bool,lambda action:fn(bool)->bool,value:bool)->bool {
            inline if flag {action(value)} else {false}
        }";
    let source=format!("{prefix}
        fn forward(comptime flag:bool)->bool {{apply(flag,...do {{input();return false;(false,)}}) do {{callback(_0)}}}}
        fn run()->bool {{forward(true)}}");
    let package = compile_with_protocols(&source, &Default::default()).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(calls(&package, &vm), vec!["run", "forward", "input"]);
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for call in [
        "apply(flag,...do {(false,)}) do {7}",
        "apply(flag,...do {return false;(false,)}) do {7}",
        "apply(flag,...do {(false,)}) do {_1}",
        "apply(runtime,...do {(false,)}) do {not _0}",
        "apply(flag,|value|value,...do {(false,)}) do {not _0}",
    ] {
        let invalid =
            format!("{prefix} fn unused(comptime flag:bool,runtime:bool)->bool {{{call}}}");
        let error = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{call}: {error}"
        );
    }
}

#[test]
fn open_computed_spreads_reserve_implicit_named_slots() {
    for (arguments, producer_order) in [
        (
            ".flag=flag,...do {(first(),)},.supplied=supplied(),...do {(last(),)}",
            vec!["first", "supplied", "last"],
        ),
        (
            "...do {(first(),last())},.supplied=supplied(),.flag=flag",
            vec!["first", "last", "supplied"],
        ),
        (
            ".supplied=supplied(),.flag=flag,...do {(first(),last())}",
            vec!["supplied", "first", "last"],
        ),
    ] {
        for target in ["choose", "generic", "generic<bool>"] {
            let source=format!("fn first()->bool {{true}} fn last()->bool {{false}} fn supplied()->bool {{false}}
                fn choose(implicit comptime flag:bool,first:bool,implicit supplied:bool,last:bool)->bool {{inline if flag {{first}} else {{last}}}}
                fn generic(implicit comptime flag:bool,first:T,implicit supplied:T,last:T)->T where T {{inline if flag {{first}} else {{last}}}}
                fn forward(comptime flag:bool)->bool {{{target}({arguments})}}
                fn run()->bool {{forward(true)}} fn other()->bool {{forward(false)}}");
            let package = compile_with_protocols(&source, &Default::default())
                .unwrap_or_else(|e| panic!("{target}({arguments}): {e}"));
            for (entry, expected) in [("run", true), ("other", false)] {
                let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
                assert_eq!(
                    vm.invoke(id(&package, entry), vec![]).unwrap(),
                    Value::Bool(expected)
                );
                let mut events = vec![entry, "forward"];
                events.extend(producer_order.iter().copied());
                events.push(if target == "choose" {
                    "choose"
                } else {
                    "generic"
                });
                assert_eq!(calls(&package, &vm), events);
                luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
            }
        }
    }
}

#[test]
fn open_implicit_spread_routing_preserves_missing_duplicate_and_stage_errors() {
    for args in [
        ".flag=flag,...do {(true,false)}",
        ".flag=flag,.supplied=true,.supplied=false,...do {(true,false)}",
        ".flag=runtime,.supplied=false,...do {(true,false)}",
        ".flag=flag,.supplied=7,...do {(true,false)}",
        ".flag=flag,.supplied=false,...do {(true,)}",
    ] {
        let source=format!("fn choose(implicit comptime flag:bool,first:bool,implicit supplied:bool,last:bool)->bool {{inline if flag {{first}} else {{last}}}}
            fn unused(comptime flag:bool,runtime:bool)->bool {{choose({args})}}");
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{args}: {error}"
        );
    }
}

#[test]
fn implicit_spreads_defaults_and_post_do_share_one_input_plan() {
    for (extra, result, default_name) in [
        ("", true, "defaulted"),
        (".fallback=overridden(),", false, "overridden"),
    ] {
        let source=format!("fn input()->bool {{true}} fn bias()->bool {{true}}
            fn defaulted(value:bool)->bool {{value}} fn overridden()->bool {{false}}
            fn apply(implicit comptime flag:bool,lambda action:fn(bool)->bool,implicit supplied:bool,.fallback:bool=defaulted(supplied),value:bool)->bool {{
                inline if flag {{action(value) and fallback}} else {{false}}
            }}
            fn forward(comptime flag:bool)->bool {{apply(.flag=flag,...do {{(input(),)}},.supplied=bias(),{extra}) do {{_0}}}}
            fn run()->bool {{forward(true)}}");
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{extra}: {e}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(result)
        );
        assert_eq!(
            calls(&package, &vm),
            vec!["run", "forward", "input", "bias", default_name, "apply"]
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_variadic_packets_preserve_interleaved_input_order() {
    for (arguments, events) in [
        (
            "flag,...do {(first(),)},.bias=bias(),...do {(last(),)}",
            vec!["first", "bias", "last"],
        ),
        (
            "flag,...do {(first(),last())},.bias=bias()",
            vec!["first", "last", "bias"],
        ),
        (
            "flag,.items=do {(first(),last())},.bias=bias()",
            vec!["first", "last", "bias"],
        ),
    ] {
        for target in ["choose", "generic<bool>", "generic"] {
            let source=format!("fn first()->bool {{true}} fn last()->bool {{false}} fn bias()->bool {{true}}
                fn choose(comptime flag:bool,...items:Tuple<bool,bool>,.bias:bool=false)->bool {{let (a,b)=items;inline if flag {{a and bias}} else {{b}}}}
                fn generic(comptime flag:bool,...items:Tuple<T,T>,.bias:bool=false)->T where T {{let (a,b)=items;inline if flag {{a}} else {{b}}}}
                fn forward(comptime flag:bool)->bool {{{target}({arguments})}}
                fn run()->bool {{forward(true)}}");
            let package =
                execution_support::compile_with_tuple_protocols(&source, &Default::default())
                    .unwrap_or_else(|e| panic!("{target}({arguments}): {e}"));
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(id(&package, "run"), vec![]).unwrap(),
                Value::Bool(true)
            );
            let mut expected = vec!["run", "forward"];
            expected.extend(events.iter().copied());
            expected.push(if target == "choose" {
                "choose"
            } else {
                "generic"
            });
            assert_eq!(calls(&package, &vm), expected);
            luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        }
    }
}

#[test]
fn open_variadic_packets_preserve_exit_and_reject_invalid_fields() {
    let prefix="fn first()->bool {true} fn bias()->bool {true} fn last()->bool {false}
        fn choose(comptime flag:bool,...items:Tuple<bool,bool>,.bias:bool=false)->bool {let (a,b)=items;inline if flag {a} else {b}}";
    let source = format!(
        "{prefix} fn forward(comptime flag:bool)->bool {{choose(flag,first(),.bias=bias(),...do {{return false;(last(),)}})}} fn run()->bool {{forward(true)}}"
    );
    let package =
        execution_support::compile_with_tuple_protocols(&source, &Default::default()).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        calls(&package, &vm),
        vec!["run", "forward", "first", "bias"]
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for args in [
        "flag,...do {(true,)}",
        "flag,...do {(true,false,true)}",
        "flag,...do {(true,7)}",
        "runtime,...do {(true,false)}",
        "flag,...do {(true,false)},.bias=7",
    ] {
        let source =
            format!("{prefix} fn unused(comptime flag:bool,runtime:bool)->bool {{choose({args})}}");
        let error = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{args}: {error}"
        );
    }
}

#[test]
fn open_variadic_packets_share_anchors_with_named_supply_and_post_do() {
    for (args, events) in [
        (
            "flag,...do {(first(),)},.bias=bias(),...do {(last(),tail())}",
            vec!["first", "bias", "last", "tail"],
        ),
        (
            "flag,...do {(tail(),)},.items=do {(first(),last())},.bias=bias()",
            vec!["tail", "first", "last", "bias"],
        ),
    ] {
        let source=format!("fn first()->bool {{true}} fn last()->bool {{false}} fn tail()->bool {{true}} fn bias()->bool {{true}}
            fn apply(comptime flag:bool,lambda action:fn(bool)->bool,...items:Tuple<bool,bool>,tail:bool,.bias:bool=false)->bool {{let (a,b)=items;inline if flag {{action(a) and tail and bias}} else {{b}}}}
            fn forward(comptime flag:bool)->bool {{apply({args}) do {{_0}}}}
            fn run()->bool {{forward(true)}}");
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{args}: {e}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        let mut expected = vec!["run", "forward"];
        expected.extend(events);
        expected.push("apply");
        assert_eq!(calls(&package, &vm), expected);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_variadic_domain_is_inferred_from_packet_remainder() {
    for arguments in [
        "flag,...do {(first(),last())},tail()",
        "flag,first(),...do {(last(),tail())}",
        "flag,...do {(first(),)},...do {(last(),)},tail()",
    ] {
        let source = format!(
            "fn first()->bool {{true}} fn last()->bool {{false}} fn tail()->bool {{true}}
            fn collect(comptime flag:bool,...items:Args,tail:bool)->Args where Args {{items}}
            fn forward(comptime flag:bool)->Tuple<bool,bool> {{collect({arguments})}}
            fn run()->bool {{let (a,b)=forward(true);a and not b}}"
        );
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{arguments}: {e}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            calls(&package, &vm),
            vec!["run", "forward", "first", "last", "tail", "collect"]
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_variadic_packet_evidence_does_not_need_a_result_hint() {
    for (packet, expected, events) in [
        (
            "do {(first(),false)}",
            true,
            vec!["run", "forward", "first", "tail", "collect"],
        ),
        (
            "do {first();return false;(true,false)}",
            false,
            vec!["run", "forward", "first"],
        ),
    ] {
        // Never supplies no field types; the later peer provides evidence
        // without being executed on the returning path.
        let (tail_type, tail_value, parameter_type) = if expected {
            ("bool", "false", "bool")
        } else {
            ("Tuple<bool,bool>", "(true,false)", "Args")
        };
        let source=format!("fn first()->bool {{true}} fn tail()->{tail_type} {{{tail_value}}}
            fn collect(comptime flag:bool,...items:Args,tail:{parameter_type})->bool where Args {{inline if flag {{true}} else {{false}}}}
            fn forward(comptime flag:bool)->bool {{collect(flag,...{packet},tail())}}
            fn run()->bool {{forward(true)}}");
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{packet}: {e}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(expected)
        );
        assert_eq!(calls(&package, &vm), events);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn open_variadic_remainder_keeps_domain_and_stage_checks() {
    for source in [
        "fn collect(comptime flag:bool,...items:bool)->bool {true} fn unused(comptime flag:bool)->bool {collect(flag,...do {(true,false)})}",
        "fn collect(comptime flag:bool,...items:Args,tail:bool)->bool where Args {true} fn unused(comptime flag:bool)->bool {collect(flag,...do {(true,false)},7)}",
        "fn collect(comptime flag:bool,comptime ...items:Args)->bool where Args {true} fn unused(comptime flag:bool,value:bool)->bool {collect(flag,...do {(value,false)})}",
        "trait Allowed {} fn collect(comptime flag:bool,...items:Args)->bool where Args:-Allowed {true} fn unused(comptime flag:bool)->bool {collect(flag,...do {(true,false)})}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{error}"
        );
    }
}

#[test]
fn collected_peer_evidence_contextualizes_computed_packets() {
    for args in ["flag,seed(),...do {(7,)}", "flag,...do {(7,)},seed()"] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            fn seed()->Byte {{9}}
            fn choose(comptime flag:bool,...items:Tuple<T,T>)->bool where T {{true}}
            fn forward(comptime flag:bool)->bool {{choose({args})}}
            fn run()->bool {{forward(true)}}"
        );
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{args}: {e}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            calls(&package, &vm),
            vec!["run", "forward", "seed", "choose"]
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let invalid = source.replace("(7,)", "(true,)");
        assert!(
            execution_support::compile_with_tuple_protocols(&invalid, &Default::default()).is_err()
        );
    }
}

#[test]
fn collected_peer_probing_preserves_early_exit_order() {
    for (args, events) in [
        (
            "flag,seed(),...do {return false;(7,)}",
            vec!["run", "forward", "seed"],
        ),
        (
            "flag,...do {return false;(7,)},seed()",
            vec!["run", "forward"],
        ),
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;fn seed()->Byte {{9}}
            fn choose(comptime flag:bool,...items:Tuple<T,T>)->bool where T {{true}}
            fn forward(comptime flag:bool)->bool {{choose({args})}}
            fn run()->bool {{forward(true)}}"
        );
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{args}: {e}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(calls(&package, &vm), events);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn collected_callback_context_infers_input_and_result_domains() {
    for args in [
        "flag,seed(),|value|not value,...do {(true,)}",
        "flag,...do {(seed(),)},|value|not value,true",
    ] {
        let source=format!("fn seed()->bool {{false}}
            fn apply(comptime flag:bool,...items:Tuple<T,fn(T)->U>,tail:bool)->U where T,U {{let (value,action)=items;action(value)}}
            fn forward(comptime flag:bool)->bool {{apply({args})}}
            fn run()->bool {{forward(true)}}");
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{args}: {e}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let wrong = source.replace("|value|not value", "|value|7");
        assert!(
            execution_support::compile_with_tuple_protocols(&wrong, &Default::default()).is_err()
        );
    }
}

#[test]
fn collected_local_evidence_contextualizes_direct_generic_callbacks() {
    let source =
        "fn apply(...items:Tuple<T,fn(T)->U>)->U where T,U {let (value,action)=items;action(value)}
        fn run()->bool {let value=false;apply(value,|input|not input)}";
    let package =
        execution_support::compile_with_tuple_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let wrong = source.replace("|input|not input", "|input|7");
    assert!(execution_support::compile_with_tuple_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn collected_callback_result_is_inferred_without_result_hint() {
    let source="fn seed()->bool {false}
        fn consume(comptime flag:bool,...items:Tuple<T,fn(T)->U>)->bool where T,U {let (value,action)=items;action(value);true}
        fn forward(comptime flag:bool)->bool {consume(flag,...do {(seed(),)},|value|not value)}
        fn run()->bool {forward(true)}";
    let package =
        execution_support::compile_with_tuple_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn collected_peer_evidence_refines_associated_callback_domains() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        trait Source {assoc Element:Type;}
        impl Source for Byte {assoc Element:Type=bool;}
        fn seed()->Byte {9}
        fn consume(comptime flag:bool,...items:Tuple<T,fn(T.as(Source).Element)->bool>,tail:bool)->bool where T:-Source {true}
        fn forward(comptime flag:bool)->bool {consume(flag,seed(),|value|not value,...do {(true,)})}
        fn run()->bool {forward(true)}";
    for source in [
        source.to_owned(),
        source.replace(
            "seed(),|value|not value,...do {(true,)}",
            "...do {(seed(),)},|value|not value,true",
        ),
    ] {
        let package =
            execution_support::compile_with_tuple_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let wrong = source.replace("assoc Element:Type=bool", "assoc Element:Type=Byte");
        assert!(
            execution_support::compile_with_tuple_protocols(&wrong, &Default::default()).is_err()
        );
        let missing = source.replace("impl Source for Byte {assoc Element:Type=bool;}", "");
        assert!(
            execution_support::compile_with_tuple_protocols(&missing, &Default::default()).is_err()
        );
    }
}

#[test]
fn direct_collections_infer_callbacks_from_computed_peer_elements() {
    let source = "fn seed()->bool {false}
        fn apply(...items:Tuple<T,fn(T)->U>)->U where T,U {let (value,action)=items;action(value)}
        fn run()->bool {apply(seed(),|value|not value)}";
    let package =
        execution_support::compile_with_tuple_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn direct_collection_inference_preserves_order_and_early_exit() {
    for (args, expected, events) in [
        (
            "seed(),.bias=bias(),|value|not value",
            true,
            vec!["run", "seed", "bias", "apply"],
        ),
        (
            "do {return false;seed()},.bias=bias(),|value:bool|not value",
            false,
            vec!["run"],
        ),
        (
            "seed(),.bias=do {return false;bias()},|value|not value",
            false,
            vec!["run", "seed"],
        ),
    ] {
        let source=format!("fn seed()->bool {{false}} fn bias()->bool {{true}}
            fn apply(...items:Tuple<T,fn(T)->U>,.bias:bool=false)->U where T,U {{let (value,action)=items;action(value)}}
            fn run()->bool {{apply({args})}}");
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{args}: {e}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(expected)
        );
        assert_eq!(calls(&package, &vm), events);
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    }
}

#[test]
fn direct_collection_defers_numeric_defaults_until_peer_evidence() {
    for args in ["7,seed()", "seed(),7"] {
        let source = format!(
            "typealias Byte=.u8'builtin;fn seed()->Byte {{9}}
            fn accept(...items:Tuple<T,T>)->bool where T {{true}}
            fn run()->bool {{accept({args})}}"
        );
        let package =
            execution_support::compile_with_tuple_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        assert!(
            execution_support::compile_with_tuple_protocols(
                &source.replace("7", "true"),
                &Default::default()
            )
            .is_err()
        );
    }
}

#[test]
fn open_property_peers_infer_callbacks_while_forwarding_stage_inputs() {
    let source="fn packet(...fields:T)->T where T {fields}
        fn exemplar(value:T)->U where T,U {exemplar<T,U>(value)}
        fn sample()->T where T {sample<T>()}
        fn seed()->bool {false}
        fn apply(comptime flag:bool,...fields:(packet {value:sample<T>(),callback:exemplar<T,U>})'type)->U where T,U {fields.callback(fields.value)}
        fn forward(comptime flag:bool)->bool {apply {flag,value:seed(),callback:|value|not value}}
        fn run()->bool {forward(true)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let deferred = format!(
        "typealias Type=.Type'builtin; comptime fn Select(flag:bool,T:Type)->Type {{if flag {{T}} else {{Select(flag,T)}}}} {}",
        source.replace(
            "->U where T,U {fields.callback",
            "->defer Select(flag,U) where T,U {fields.callback"
        )
    );
    let package = compile_with_protocols(&deferred, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn deferred_property_forwarding_preserves_aliases_and_stage_rejections() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Select(flag:bool)->Type {if flag {bool} else {Select(flag)}}
        fn packet(...fields:T)->T where T {fields}
        fn exemplar(value:T)->bool where T {true}
        fn sample()->T where T {sample<T>()}
        fn seed()->bool {false}
        fn apply(comptime flag:bool,...fields:(packet {value:sample<T>(),callback:exemplar<T>})'type)->defer Select(flag) where T {fields.callback(fields.value)}";
    let source=format!("{prefix}
        fn inner(comptime flag:bool)->bool {{let chosen=flag;apply {{chosen,value:seed(),callback:|value|not value}}}}
        fn outer(comptime flag:bool)->bool {{inner(flag)}}
        fn run()->bool {{outer(true)}}");
    let package = compile_with_protocols(&source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for args in [
        "runtime,value:seed(),callback:|value|not value",
        "flag,value:seed(),callback:|value|7",
        "flag,value:seed(),value:seed(),callback:|value|not value",
        "flag,value:seed()",
        "flag,unknown:seed(),callback:|value|not value",
    ] {
        let source = format!(
            "{prefix} fn unused(comptime flag:bool,runtime:bool)->bool {{apply {{{args}}}}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{args}: {error}"
        );
    }
}

#[test]
fn mixed_collector_forwarding_retains_deferred_result_dependencies() {
    for args in [
        "flag,first(),enabled:property(),last()",
        "flag,...do {(first(),)},enabled:property(),...do {(last(),)}",
    ] {
        let source=format!("typealias Type=.Type'builtin;
            comptime fn Select(flag:bool)->Type {{if flag {{bool}} else {{Select(flag)}}}}
            fn packet(...fields:T)->T where T {{fields}}
            fn first()->bool {{true}} fn property()->bool {{true}} fn last()->bool {{false}}
            fn apply(comptime flag:bool,...items:Tuple<bool,bool>,...properties:(packet {{enabled:true}})'type)->defer Select(flag) {{let (a,b)=items;a and not b and properties.enabled}}
            fn forward(comptime flag:bool)->bool {{apply {{{args}}}}}
            fn run()->bool {{forward(true)}}");
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{args}: {e}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            calls(&package, &vm),
            vec!["run", "forward", "first", "property", "last", "apply"]
        );
        luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let ambiguous = source
            .replace(
                "...items:Tuple<bool,bool>",
                "...items:(packet {enabled:true})'type",
            )
            .replace(
                "let (a,b)=items;a and not b and properties.enabled",
                "properties.enabled",
            );
        let error =
            execution_support::compile_with_tuple_protocols(&ambiguous, &Default::default())
                .unwrap_err();
        assert!(error.contains("one variadic entrance"), "{error}");
    }
}

#[test]
fn explicit_collector_domains_route_deferred_forwarding() {
    for (parameters, args) in [
        (
            "comptime flag:bool,...items:T,...properties:U",
            "flag,true,enabled:true,false",
        ),
        (
            "...items:T,...properties:U,comptime flag:bool",
            "true,enabled:true,false,flag",
        ),
        (
            "comptime flag:bool,...items:T,...properties:U",
            "flag,...do {(true,)},enabled:true,...do {(false,)}",
        ),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            comptime fn Select(flag:bool)->Type {{if flag {{bool}} else {{Select(flag)}}}}
            fn packet(...fields:T)->T where T {{fields}}
            typealias Options=(packet {{enabled:true}})'type;
            fn apply({parameters})->defer Select(flag) where T,U {{true}}
            fn forward(comptime flag:bool)->bool {{apply<Tuple<bool,bool>,Options> {{{args}}}}}
            fn run()->bool {{forward(true)}}"
        );
        let package = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{args}: {e}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let runtime = source.replace("forward(comptime flag:bool)", "forward(flag:bool)");
        assert!(
            execution_support::compile_with_tuple_protocols(&runtime, &Default::default()).is_err()
        );
        let wrong = source
            .replace("enabled:true,false", "enabled:7,false")
            .replace("enabled:true,...do", "enabled:7,...do");
        assert!(
            execution_support::compile_with_tuple_protocols(&wrong, &Default::default()).is_err()
        );
    }
}

#[test]
fn explicit_collection_substitutions_remain_per_call_with_defaults() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool)->Type {if flag {bool} else {Select(flag)}}
        fn packet(...fields:T)->T where T {fields}
        fn byte()->Byte {9}
        typealias Options=(packet {enabled:true})'type;
        typealias Numeric=(packet {enabled:byte()})'type;
        fn apply(comptime flag:bool,...items:T,...properties:U)->defer Select(flag) where T,.U:Type=Options {true}
        fn forward(comptime flag:bool)->bool {
            apply<Tuple<bool,bool>> {flag,true,enabled:true,false};
            apply<Tuple<Byte,Byte>,.U=Numeric> {flag,7,enabled:byte(),9};
            apply<Tuple<bool,bool>> {flag,false,enabled:false,true}
        }
        fn run()->bool {forward(true)}";
    let package =
        execution_support::compile_with_tuple_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for wrong in [
        source.replace("flag,7,enabled:byte(),9", "flag,true,enabled:byte(),9"),
        source.replace("flag,7,enabled:byte(),9", "flag,7,enabled:true,9"),
        source.replace(
            "flag,false,enabled:false,true",
            "flag,false,enabled:byte(),true",
        ),
    ] {
        let error = execution_support::compile_with_tuple_protocols(&wrong, &Default::default())
            .unwrap_err();
        assert!(
            !error.contains("parsing failed") && !error.contains("HIR lowering failed"),
            "{error}"
        );
    }
}
