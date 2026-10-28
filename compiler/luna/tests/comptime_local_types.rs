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
