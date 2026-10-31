//! Structural matches and propagation use the same compiled control flow.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

#[test]
fn error_propagation_returns_before_later_computation() {
    let package = support::compile(
        "enum Failure { bad(bool) }
        comptime fn checked(flag: bool) -> ![Failure] bool {
            if flag { true } else { error Failure.bad(false) }
        }
        comptime fn propagate(flag: bool) -> ![Failure] bool {
            let value = checked(flag)!;
            not value
        }
        comptime fn observe(flag: bool) -> bool {
            propagate(flag) match {
                value! => value,
                error .bad(payload) => payload,
            }
        }",
    );
    let root = support::id(&package, "observe");
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for flag in [false, true] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(root, vec![Value::Bool(flag)]).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            tree.invoke(root, vec![Value::Bool(flag)]).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(vm.bytecode_invocations() - before, 3);
    }
}

#[test]
fn optional_match_preserves_layers_and_selected_arm() {
    let package = support::compile(
        "comptime fn classify(value: ?bool) -> bool {
            value match {
                null => false,
                payload? => payload,
            }
        }
        comptime fn present() -> bool { classify(true) }
        comptime fn absent() -> bool { classify(null) }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for (name, expected) in [("present", true), ("absent", false)] {
        let root = support::id(&package, name);
        let before = vm.bytecode_invocations();
        assert_eq!(vm.invoke(root, vec![]).unwrap(), Value::Bool(expected));
        assert_eq!(tree.invoke(root, vec![]).unwrap(), Value::Bool(expected));
        assert_eq!(vm.bytecode_invocations() - before, 2);
    }
}

#[test]
fn guard_failure_preserves_effects_and_exits_unwind_match_scopes() {
    let package = support::compile(
        "comptime fn guard_effects() -> bool {
            var trace = false;
            let chosen = (do { trace = not trace; true }) match {
                first if (do { trace = not trace; false }) => do { while true {} false },
                second => second,
            };
            chosen and not trace
        }
        comptime fn exits() -> bool {
            var trace = false;
            while true {
                true match {
                    bound => { trace = bound; break; },
                };
            };
            trace
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for name in ["guard_effects", "exits"] {
        let root = support::id(&package, name);
        let before = vm.bytecode_invocations();
        assert_eq!(vm.invoke(root, vec![]).unwrap(), Value::Bool(true));
        assert_eq!(tree.invoke(root, vec![]).unwrap(), Value::Bool(true));
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}

#[test]
fn alternatives_remap_payload_bindings_without_leaking_failed_candidates() {
    let package = support::compile(
        "enum Choice { left(bool, bool), right(bool, bool) }
        comptime fn select(value: Choice) -> bool {
            value match {
                .left(payload, _) or .right(_, payload) => payload,
            }
        }
        comptime fn left() -> bool { select(.left(true, false)) }
        comptime fn right() -> bool { select(.right(false, true)) }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for name in ["left", "right"] {
        let root = support::id(&package, name);
        let before = vm.bytecode_invocations();
        assert_eq!(vm.invoke(root, vec![]).unwrap(), Value::Bool(true));
        assert_eq!(tree.invoke(root, vec![]).unwrap(), Value::Bool(true));
        assert_eq!(vm.bytecode_invocations() - before, 2);
    }
}
