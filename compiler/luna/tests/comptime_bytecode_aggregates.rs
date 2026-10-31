//! Tuple values and ordered Objects use ordinary aggregate evaluation rules.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

#[test]
fn tuple_construction_and_destructuring_preserve_order_and_early_return() {
    let package = support::compile_with_tuple(
        "comptime fn ordered() -> bool {
            var trace = false;
            let (first, second) = (do { trace = not trace; trace }, do { trace = not trace; trace });
            first and not second and not trace
        }
        comptime fn early() -> bool {
            let values = (do { return true; false }, do { while true {} false });
            false
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for name in ["ordered", "early"] {
        let before = vm.bytecode_invocations();
        let id = support::id(&package, name);
        assert_eq!(vm.invoke(id, vec![]).unwrap(), Value::Bool(true));
        assert_eq!(tree.invoke(id, vec![]).unwrap(), Value::Bool(true));
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}

#[test]
fn object_construction_preserves_duplicates_and_nested_values() {
    let package = support::compile(
        "typealias Object = .Object'builtin;
        comptime fn make(flag: bool) -> Object {
            var trace = false;
            let object = {item: do { trace = not trace; flag }, item: not trace, nested: {flag: trace}};
            object
        }
        comptime fn early() -> bool {
            let object = {first: do { return true; false }, second: do { while true {} false }};
            false
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let before = vm.bytecode_invocations();
    let Value::Object(object) = vm
        .invoke(support::id(&package, "make"), vec![Value::Bool(true)])
        .unwrap()
    else {
        panic!("Object")
    };
    let entries = object.entries();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].key(), Some("item"));
    assert_eq!(entries[1].key(), Some("item"));
    assert_eq!(entries[0].value().value, Value::Bool(true));
    assert_eq!(entries[1].value().value, Value::Bool(false));
    let Value::Object(nested) = &entries[2].value().value else {
        panic!("nested Object")
    };
    assert_eq!(nested.entries()[0].key(), Some("flag"));
    assert_eq!(nested.entries()[0].value().value, Value::Bool(true));
    assert_eq!(vm.bytecode_invocations() - before, 1);
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(support::id(&package, "early"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
}

#[test]
fn object_queries_and_edits_stay_in_bytecode_and_preserve_snapshots() {
    // Bootstrap bindings are explicit here so the invocation counter verifies
    // this body, without library wrapper invocations obscuring the route.
    let package = support::compile(
        "typealias Object = .Object'builtin;
        typealias Symbol = .Symbol'builtin;
        typealias Type = .Type'builtin;
        const length = .object_length'builtin;
        const key = .object_key'builtin;
        const EntryType = .object_entry_type'builtin;
        const value = .object_value'builtin;
        const get = .object_get'builtin;
        const append = .object_append'builtin;
        const replace = .object_replace'builtin;
        const remove = .object_remove'builtin;
        const concat = .object_concat'builtin;
        comptime fn Reflected() -> Type {
            let object = {flag: true};
            EntryType(object, 0)
        }
        comptime fn run() -> bool {
            var original = {flag: true, flag: false};
            let saved = original;
            let snapshot = get(bool, original, do {
                original = replace(original, 1, true);
                let name: Symbol = .flag;
                name
            });
            let absent: ?Symbol = null;
            let appended = append(original, absent, 'x');
            let replaced = replace(appended, 0, 'z');
            let removed = remove(replaced, 1);
            let joined = concat(removed, saved);
            length(joined) == 4
                and EntryType(joined, 0) == EntryType(joined, 1)
                and (key(joined, 0) matches .flag?)
                and (key(joined, 1) matches null)
                and (value(bool, joined, 2) matches true?)
                and (get(bool, joined, .flag) matches false?)
                and (value(bool, joined, 0) matches null)
                and (snapshot matches false?)
                and (get(bool, original, .flag) matches true?)
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(support::id(&package, "Reflected"), vec![])
            .unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
}
