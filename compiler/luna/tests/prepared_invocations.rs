#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;
use luna::invocation::PreparedInvocation;
use std::sync::Arc;

#[test]
fn prepared_calls_reuse_code_without_reusing_results_or_local_state() {
    let package = Arc::new(support::compile(
        "comptime fn run()->bool {
        var state=false; state=not state; state
    }",
    ));
    let mut invocation = PreparedInvocation::new(package, "run", None).unwrap();
    assert_eq!(invocation.invoke(100000).unwrap(), Value::Bool(true));
    assert!(invocation.last_stats().bytecode_compilations > 0);
    assert!(invocation.last_stats().bytecode_invocations > 0);
    assert_eq!(invocation.invoke(100000).unwrap(), Value::Bool(true));
    assert_eq!(invocation.last_stats().bytecode_compilations, 0);
    assert!(invocation.last_stats().bytecode_invocations > 0);
    assert!(invocation.invoke(1).is_err());
    assert_eq!(invocation.invoke(100000).unwrap(), Value::Bool(true));
    assert_eq!(invocation.last_stats().bytecode_compilations, 0);
    assert!(invocation.last_stats().bytecode_invocations > 0);
}

#[test]
fn prepared_default_type_inputs_preserve_checked_dispatch() {
    let package = Arc::new(support::compile(
        "typealias Type=.Type'builtin;
        trait Probe {comptime fn probe()->bool;}
        impl Probe for bool {comptime fn probe()->bool {true}}
        comptime fn run(.T:Type=bool)->bool {if T==bool {bool.as(Probe).probe()} else {false}}",
    ));
    let mut invocation = PreparedInvocation::new(package, "run", None).unwrap();
    assert_eq!(invocation.invoke(100000).unwrap(), Value::Bool(true));
    assert!(invocation.last_stats().bytecode_compilations > 0);
    assert_eq!(invocation.invoke(100000).unwrap(), Value::Bool(true));
    assert_eq!(invocation.last_stats().bytecode_compilations, 0);
    assert!(invocation.last_stats().bytecode_invocations > 0);
}
