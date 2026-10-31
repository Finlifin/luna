//! Ordinary immutable comptime bindings carry structural type evidence while
//! retaining their initializer execution and local storage.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Event, Interpreter, Value};

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_protocols(
        &format!("typealias Type=.Type'builtin; typealias Byte=.u8'builtin; {source}"),
        &Default::default(),
    )
}

#[test]
fn immutable_type_aliases_support_chains_destructuring_and_shadowing() {
    for body in [
        "let T=bool; let value:T=true; value",
        "let T=bool; let U=T; let value:U=true; value",
        "let (T, flag)=(bool,true); let value:T=flag; value",
        "let ((T, flag), other)=((bool,true),false); let value:T=flag; value and not other",
        "let T=bool; do {let T=Byte; let value:T=7; value == 7}; let value:T=true; value",
        "let T=Box; let value:T=Box {flag:true}; value.flag",
    ] {
        let source = format!("struct Box {{flag:bool}} comptime fn run()->bool {{{body}}}");
        let package = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{source}"
        );
    }
}

#[test]
fn partial_aggregates_keep_type_evidence_and_execute_unknown_fields() {
    for body in [
        "let (T, flag)=(bool, observe(true)); let value:T=flag; value",
        "let descriptor=Descriptor {ty:bool, flag:observe(true)}; let T=descriptor.ty; let value:T=descriptor.flag; value",
    ] {
        let source = format!(
            "struct Descriptor {{ty:Type,flag:bool}} comptime fn observe(flag:bool)->bool {{flag}} comptime fn run()->bool {{{body}}}"
        );
        let package = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        // The initializer's ordinary call must survive observation and execute
        // exactly once on each invocation, rather than being folded away.
        let observed = support::id(&package, "observe");
        assert_eq!(
            vm.events()
                .iter()
                .filter(|event| matches!(event, Event::EnterFunction(id) if *id == observed))
                .count(),
            1
        );
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            vm.events()
                .iter()
                .filter(|event| matches!(event, Event::EnterFunction(id) if *id == observed))
                .count(),
            2
        );
    }
}

#[test]
fn type_observation_does_not_freeze_mutable_or_runtime_bindings() {
    for source in [
        "comptime fn run()->bool {var T=bool; T=Byte; let value:T=true; true}",
        "fn run()->bool {let T=bool; let value:T=true; value}",
        "comptime fn run()->bool {let T=Byte; let value:T=true; true}",
        "pure comptime fn Identity<T>->Type {T} comptime fn run()->bool {let T=Identity<bool>; let value:T=true; true}",
    ] {
        assert!(compile(source).is_err(), "unexpectedly accepted {source}");
    }
}

#[test]
fn unknown_control_flow_retains_only_common_type_evidence() {
    for body in [
        "let T=if flag {bool} else {bool}; let value:T=flag; value",
        "let T=flag match {true=>bool,false=>bool}; let value:T=flag; value",
        "let (T,value)=if flag {(bool,true)} else {(bool,false)}; let result:T=value; result",
        "let descriptor=if flag {Descriptor {ty:bool,flag:true}} else {Descriptor {ty:bool,flag:observe(false)}}; let result:descriptor.ty=descriptor.flag; result",
        "let descriptor=flag match {true=>Descriptor {ty:bool,flag:observe(true)},false=>Descriptor {ty:bool,flag:observe(false)}}; let T=descriptor.ty; let result:T=descriptor.flag; result",
        "let .item(T,value)=if flag {Choice.item(bool,true)} else {Choice.item(bool,observe(false))}; let result:T=value; result",
        "let T=if true {bool} else {Byte}; let result:T=flag; result",
    ] {
        let source = format!(
            "struct Descriptor {{ty:Type,flag:bool}} enum Choice {{item(Type,bool)}} comptime fn observe(value:bool)->bool {{value}} comptime fn choose(flag:bool)->bool {{{body}}} comptime fn run()->bool {{choose(true) and not choose(false)}}"
        );
        let package = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{source}"
        );
    }
}

#[test]
fn branch_observation_keeps_condition_and_selected_calls() {
    let package = compile("comptime fn condition(flag:bool)->bool {flag} comptime fn left()->bool {true} comptime fn right()->bool {false}
        comptime fn choose(flag:bool)->bool {let (T,value)=if condition(flag) {(bool,left())} else {(bool,right())}; let result:T=value;result}
        comptime fn run()->bool {choose(true) and not choose(false)}").unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let names = ["condition", "left", "right"];
    let ids = names.map(|name| support::id(&package, name));
    let calls = vm
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => ids.iter().position(|expected| id == expected),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(calls, [0, 1, 0, 2]);
}

#[test]
fn divergent_branch_types_do_not_become_fixed_annotations() {
    for value in [
        "if flag {bool} else {Byte}",
        "flag match {true=>bool,false=>Byte}",
    ] {
        let source = format!(
            "comptime fn choose(flag:bool)->bool {{let T={value};let result:T=true;result}}"
        );
        assert!(compile(&source).is_err(), "{source}");
    }
    assert!(compile("struct Descriptor {ty:Type,flag:bool} comptime fn choose(flag:bool)->bool {let d=if flag {Descriptor {ty:bool,flag:true}} else {Descriptor {ty:Byte,flag:false}};let result:d.ty=true;result}").is_err());
}

#[test]
fn match_common_evidence_does_not_execute_guards_during_observation() {
    let source = "comptime fn guard(flag:bool)->bool {flag} comptime fn choose(flag:bool)->bool {let T=flag match {value if guard(value)=>bool,_=>bool}; let result:T=flag; result} comptime fn run()->bool {choose(true) and not choose(false)}";
    for binding in ["let", "const"] {
        let package = compile(&source.replace("let T=", &format!("{binding} T="))).unwrap();
        let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        let guard = support::id(&package, "guard");
        assert_eq!(
            vm.events()
                .iter()
                .filter(|event| matches!(event, Event::EnterFunction(id) if *id == guard))
                .count(),
            2,
            "{binding}"
        );
    }
    assert!(compile("fn choose(flag:bool)->bool {const T=if flag {bool} else {bool}; let result:T=true; result}").is_err());
}

#[test]
fn local_blocks_preserve_partial_types_without_replaying_initializers() {
    for body in [
        "let (T,flag)=do {let descriptor=(bool,observe(true)); descriptor}; let result:T=flag; result",
        "let (T,flag)=do {let T=bool; let flag=observe(true); let Alias=T; (Alias,flag)}; let result:T=flag; result",
        "let (T,flag)=do {let (T,flag)=(bool,observe(true)); (T,flag)}; let result:T=flag; result",
        "let descriptor=do {let descriptor=Descriptor {ty:bool,flag:observe(true)}; descriptor}; let result:descriptor.ty=descriptor.flag; result",
        "let (T,flag)=do {let selected=true; let T=if selected {bool} else {Byte}; let flag=observe(true); (T,flag)}; let result:T=flag; result",
        "let (T,flag)=do {let T=bool; let flag=do {let T=Byte; let value:T=7; observe(value==7)}; (T,flag)}; let result:T=flag; result",
    ] {
        let source = format!(
            "struct Descriptor {{ty:Type,flag:bool}} comptime fn observe(flag:bool)->bool {{flag}} comptime fn run()->bool {{{body}}}"
        );
        let package = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
        let observed = support::id(&package, "observe");
        for expected_count in [1, 2] {
            assert_eq!(
                vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
                Value::Bool(true),
                "{source}"
            );
            assert_eq!(
                vm.events()
                    .iter()
                    .filter(|event| matches!(event, Event::EnterFunction(id) if *id == observed))
                    .count(),
                expected_count,
                "{source}"
            );
        }
    }
}

#[test]
fn branch_local_aliases_preserve_common_type_fields() {
    let source = "comptime fn observe(flag:bool)->bool {flag} comptime fn choose(flag:bool)->bool {
        let (T,value)=if flag {let descriptor=(bool,observe(true));descriptor} else {let T=bool;let value=observe(false);(T,value)};
        let result:T=value;result
    } comptime fn run()->bool {choose(true) and not choose(false)}";
    let package = compile(source).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn block_observation_does_not_replay_mutation_or_promote_unknown_calls() {
    for source in [
        "comptime fn run()->bool {let T=do {var T=bool;T=Byte;T};let result:T=true;result}",
        "comptime fn Choose(flag:bool)->Type {if flag {bool} else {Byte}} comptime fn run()->bool {let T=do {let T=Choose(true);T};let result:T=true;result}",
        "fn run(flag:bool)->bool {const T=do {let flag=flag; if flag {bool} else {bool}}; let result:T=true;result}",
    ] {
        assert!(compile(source).is_err(), "unexpectedly accepted {source}");
    }
}
