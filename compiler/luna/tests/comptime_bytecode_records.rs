//! Record construction uses ordinary checked values and preserves initialization order.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

#[test]
fn named_fields_keep_declaration_order_and_generic_arguments() {
    let package = support::compile(
        "typealias Integer = .Integer'builtin;
        struct Pair { left: bool, right: bool }
        struct Box where T { value: T }
        comptime fn run() -> bool {
            var trace: Integer = 0;
            let pair = Pair {
                right: do { trace = trace * 10 + 1; true },
                left: do { trace = trace * 10 + 2; false }
            };
            let boxed = Box<Pair> {value: pair};
            not boxed.value.left and boxed.value.right and trace == 21
        }",
    );
    let root = support::id(&package, "run");
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    // docs/type/user-defined-types/structs.md requires declaration order,
    // independently of the order of named construction inputs.
    assert_eq!(vm.invoke(root, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(tree.invoke(root, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(vm.bytecode_invocations(), 1);
    assert_eq!(vm.bytecode_compilations(), 1);
}

#[test]
fn anonymous_record_inputs_construct_and_update_nested_fields() {
    let package = support::compile(
        "comptime fn keep(...properties: T) -> T where T { properties }
        comptime fn run() -> bool {
            let value = keep {flag: false, nested: keep {flag: true}};
            value.flag = true;
            value.nested.flag = false;
            value.flag and not value.nested.flag
        }",
    );
    let root = support::id(&package, "run");
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(root, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(tree.invoke(root, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(vm.bytecode_invocations(), 3);
}

#[test]
fn explicit_fields_skip_defaults_and_empty_records_keep_identity() {
    let package = support::compile(
        "comptime fn supplied() -> bool { true }
        struct Config { first: bool = supplied(), second: bool }
        struct Empty {}
        comptime fn empty() -> Empty { Empty {} }
        comptime fn run() -> bool {
            let first = Config {second: false};
            let second = Config {first: false, second: true};
            first.first and not second.first and second.second
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(vm.bytecode_invocations(), 2);
    let supplied = support::id(&package, "supplied");
    assert_eq!(
        vm.events()
            .iter()
            .filter(|event| matches!(event,
        comptime_interpretor::Event::EnterFunction(id) if *id == supplied))
            .count(),
        1
    );
    let empty = package
        .structs
        .iter()
        .find(|s| s.name == "Empty")
        .unwrap()
        .declaration;
    assert_eq!(
        vm.invoke(support::id(&package, "empty"), vec![]).unwrap(),
        Value::Record {
            declaration: empty,
            arguments: vec![],
            fields: vec![]
        }
    );
    assert_eq!(vm.bytecode_invocations(), 3);
}
