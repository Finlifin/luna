//! Destination delivery retains presence layers, source effects and exits.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

fn payload(mut value: Value, layers: usize) -> Value {
    let Value::ErrorResult {
        error: None,
        value: success,
        ..
    } = value
    else {
        panic!("expected success, got {value:?}");
    };
    value = *success;
    for _ in 0..layers {
        let Value::Optional {
            value: Some(inner), ..
        } = value
        else {
            panic!("expected another presence layer, got {value:?}");
        };
        value = *inner;
    }
    value
}

#[test]
fn nested_destinations_preserve_contextual_values_calls_and_control_exits() {
    let package = sysroot::compile(
        "
        typealias Delivered = !bool ???u8;
        comptime fn literal() -> Delivered { 7 }
        comptime fn nested() -> Delivered { do { do { 7 } } }
        comptime fn nested_early() -> Delivered { do { do { return 9; 7 } } }
        comptime fn nested_void() -> !bool void { do { do {} } }
        comptime fn empty() -> ?u8 { null }
        comptime fn absence() -> Delivered { empty() }
        comptime fn early() -> Delivered {
            let value: Delivered = do { return 9; 7 };
            value
        }
        comptime fn failure() -> Delivered { error true }
        comptime fn branch(flag: bool) -> Delivered { if flag { 7 } else { null } }

        struct Counter { count: Integer }
        impl Counter {
            comptime fn next(self) -> u8 { self.count += 1; 7 }
        }
        comptime fn once() -> Tuple<Integer, Delivered> {
            var counter = Counter { count: 0 };
            let value: Delivered = counter.next();
            (counter.count, value)
        }
    ",
    )
    .unwrap();
    for tree in [false, true] {
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, Budget::default())
        } else {
            Interpreter::for_comptime(&package, Budget::default())
        }
        .unwrap();
        let id = |name| {
            package
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .declaration
        };
        for (name, expected) in [
            ("literal", 7),
            ("early", 9),
            ("nested", 7),
            ("nested_early", 9),
        ] {
            let value = vm.invoke(id(name), vec![]).unwrap();
            assert_eq!(
                payload(value, 3),
                Value::UnsignedInteger(expected),
                "{name}, tree={tree}"
            );
        }
        assert_eq!(
            payload(vm.invoke(id("nested_void"), vec![]).unwrap(), 0),
            Value::Unit
        );
        let value = vm.invoke(id("absence"), vec![]).unwrap();
        assert!(matches!(
            payload(value, 2),
            Value::Optional { value: None, .. }
        ));
        let value = vm.invoke(id("failure"), vec![]).unwrap();
        let Value::ErrorResult {
            error: Some(_),
            value,
            ..
        } = value
        else {
            panic!("{value:?}")
        };
        assert_eq!(*value, Value::Bool(true));
        let present = vm.invoke(id("branch"), vec![Value::Bool(true)]).unwrap();
        assert_eq!(payload(present, 3), Value::UnsignedInteger(7));
        let absent = vm.invoke(id("branch"), vec![Value::Bool(false)]).unwrap();
        assert!(matches!(
            payload(absent, 0),
            Value::Optional { value: None, .. }
        ));
        let value = vm.invoke(id("once"), vec![]).unwrap();
        let Value::Tuple(mut fields) = value else {
            panic!("{value:?}")
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(payload(fields.pop().unwrap(), 3), Value::UnsignedInteger(7));
        assert_eq!(
            fields.pop().unwrap(),
            Value::Integer(comptime_runtime::Integer::from_u128(1))
        );
        if !tree {
            assert!(vm.bytecode_invocations() > 0);
        }
    }
}

#[test]
fn pending_delivery_does_not_weaken_the_child_domain() {
    for body in ["256", "true", "error 'x'"] {
        let source = format!("comptime fn invalid() -> !bool ???u8 {{ {body} }}");
        assert!(sysroot::compile(&source).is_err(), "accepted {source}");
    }
}
