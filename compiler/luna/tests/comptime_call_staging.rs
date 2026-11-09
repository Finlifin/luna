//! Comptime calls keep their argument programs in the invoking environment.
use comptime_interpretor::{Budget, Event, Interpreter, Value};
#[path = "support/sysroot_execution.rs"]
mod sysroot;

fn check_both(source: &str, events: &[(&str, usize)]) {
    let package = sysroot::compile(source).unwrap();
    let id = |name: &str| {
        package
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
            .declaration
    };
    for tree in [false, true] {
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, Budget::default())
        } else {
            Interpreter::for_comptime(&package, Budget::default())
        }
        .unwrap();
        for invocation in 1..=2 {
            assert_eq!(
                vm.invoke(id("check"), vec![]).unwrap(),
                Value::Bool(true),
                "tree={tree}"
            );
            for (name, count) in events {
                assert_eq!(vm.events().iter().filter(|event| matches!(event, Event::EnterFunction(declaration) if *declaration == id(name))).count(), count * invocation, "{name}, tree={tree}");
            }
        }
    }
}

#[test]
fn explicit_stage_markers_keep_argument_order_and_caller_storage() {
    check_both(
        "
        comptime fn accept(comptime T: Type, comptime flag: bool, value: Integer) -> bool {
            T == bool and flag and value == 123
        }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let result = accept(
                do { trace = trace * 10 + 1; bool },
                do { trace = trace * 10 + 2; true },
                do { trace = trace * 10 + 3; trace }
            );
            result and trace == 123
        }
    ",
        &[("accept", 1)],
    );
}

#[test]
fn closed_argument_producers_run_per_call_even_when_code_is_warm() {
    check_both(
        "
        comptime fn produce() -> bool { true }
        comptime fn accept(comptime flag: bool) -> bool { flag }
        comptime fn check() -> bool { accept(produce()) and accept(produce()) }
    ",
        &[("accept", 2), ("produce", 2)],
    );
}

#[test]
fn callable_values_keep_the_same_argument_execution_boundary() {
    check_both(
        "
        comptime fn accept(comptime flag: bool) -> bool { flag }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let callable = accept;
            let result = callable(do { trace = trace + 1; true });
            result and trace == 1
        }
    ",
        &[("accept", 1)],
    );
}

#[test]
fn argument_return_does_not_invoke_the_callee() {
    check_both(
        "
        comptime fn accept(comptime flag: bool) -> bool { flag }
        comptime fn check() -> bool {
            accept(do { return true; false })
        }
    ",
        &[("accept", 0)],
    );
}

#[test]
fn ordinary_comptime_parameters_can_supply_explicit_comptime_parameters() {
    check_both(
        "
        comptime fn accept(comptime flag: bool) -> bool { flag }
        comptime fn forward(flag: bool) -> bool { accept(flag) }
        comptime fn check() -> bool { forward(true) and not forward(false) }
    ",
        &[("accept", 2), ("forward", 2)],
    );
}

#[test]
fn spread_producers_stay_in_the_callers_invocation() {
    check_both(
        "
        comptime fn accept(comptime flag: bool) -> bool { flag }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let result = accept(...do { trace = trace + 1; (true,) });
            result and trace == 1
        }
    ",
        &[("accept", 1)],
    );
}

#[test]
fn runtime_calls_still_require_staged_inputs_and_reject_runtime_capture() {
    assert_eq!(
        sysroot::run(
            "
        fn selected(comptime flag: bool) -> bool { flag }
        fn run() -> bool { selected(true) }
    "
        ),
        Value::Bool(true)
    );
    for signature in ["fn", "comptime fn"] {
        let source = format!(
            "
            {signature} selected(comptime flag: bool) -> bool {{ flag }}
            fn run(value: bool) -> bool {{ selected(value) }}
        "
        );
        let failure = sysroot::compile(&source).unwrap_err();
        assert!(
            failure.contains("runtime parameter")
                || failure.contains("runtime input")
                || failure.contains("comptime-only value cannot inhabit runtime storage"),
            "{failure}"
        );
    }
}
