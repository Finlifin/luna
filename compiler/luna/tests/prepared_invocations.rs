#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
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

#[test]
fn source_invocation_preserves_type_dispatch_higher_order_values_and_fresh_frames() {
    let package = Arc::new(
        sysroot::compile(
            "
        pure comptime trait Flag {
            pure comptime fn flag() -> bool;
            assoc initial: bool;
        }
        comptime impl Flag for bool {
            pure comptime fn flag() -> bool { true }
            assoc initial: bool = true;
        }
        comptime impl Flag for u8 {
            pure comptime fn flag() -> bool { false }
            assoc initial: bool = false;
        }
        comptime fn selected(comptime T: Type) -> comptime fn() -> bool
        where requires T :- Flag
        {
            T.as(Flag).flag
        }
        comptime fn apply(action: comptime fn() -> bool) -> bool { action() }
        comptime fn direct(comptime T: Type) -> bool
        where requires T :- Flag
        {
            T.as(Flag).flag() == T.as(Flag).initial
        }
        comptime fn source_entry() -> bool {
            var state = false;
            state = not state;
            let first = selected(bool);
            let second = selected(u8);
            state and apply(first) and not apply(second) and direct(bool) and direct(u8)
        }
        ",
        )
        .unwrap(),
    );
    let mut call = PreparedInvocation::new(package.clone(), "source_entry", None).unwrap();
    assert!(call.invoke(1).is_err());
    for _ in 0..3 {
        assert_eq!(call.invoke(1_000_000).unwrap(), Value::Bool(true));
        assert!(call.last_stats().bytecode_invocations > 0);
    }
    assert_eq!(
        luna::invocation::invoke(&package, "source_entry", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}
