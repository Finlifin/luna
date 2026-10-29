//! Refutable declarations commit bindings only on success and preserve else exits.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

#[test]
fn destructuring_declarations_publish_mutable_bindings_and_return_on_failure() {
    let package = support::compile(
        "enum Envelope { present(bool), absent }
        comptime fn unpack(input: Envelope) -> bool {
            var .present(payload) = input else { return false; };
            payload = not payload;
            payload
        }
        comptime fn optional(input: ?bool) -> bool {
            let payload? = input else { return false; };
            payload
        }
        comptime fn present() -> bool { unpack(.present(false)) }
        comptime fn absent() -> bool { unpack(.absent) }
        comptime fn some() -> bool { optional(true) }
        comptime fn none() -> bool { optional(null) }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for (name, expected) in [
        ("present", true),
        ("absent", false),
        ("some", true),
        ("none", false),
    ] {
        let root = support::id(&package, name);
        let before = vm.bytecode_invocations();
        assert_eq!(vm.invoke(root, vec![]).unwrap(), Value::Bool(expected));
        assert_eq!(tree.invoke(root, vec![]).unwrap(), Value::Bool(expected));
        assert_eq!(vm.bytecode_invocations() - before, 2);
    }
}

#[test]
fn declaration_guards_preserve_outer_effects_before_else_returns() {
    let package = support::compile(
        "comptime fn guarded() -> bool {
            var trace = false;
            let payload if (do { trace = not trace; false }) = true else { return trace; };
            payload
        }
        comptime fn iteration() -> bool {
            var again = true;
            var result = false;
            while again {
                again = false;
                let input: ?bool = null;
                let payload? = input else { result = true; continue; };
                result = payload;
            };
            result
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for name in ["guarded", "iteration"] {
        let root = support::id(&package, name);
        let before = vm.bytecode_invocations();
        assert_eq!(vm.invoke(root, vec![]).unwrap(), Value::Bool(true));
        assert_eq!(tree.invoke(root, vec![]).unwrap(), Value::Bool(true));
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}
