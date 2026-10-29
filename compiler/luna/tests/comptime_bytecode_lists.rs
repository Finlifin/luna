//! Lists retain their registered declaration and ordinary element domain in VM code.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use nir::{Primitive, Term};

#[test]
fn list_construction_concat_and_length_execute_without_losing_snapshots() {
    let package = support::compile_with_tuple(
        "typealias Type = .Type'builtin;
        typealias Integer = .Integer'builtin;
        typealias List<T> = TupleElements<T>;
        const length = .list_length'builtin;
        const concat = .list_concat'builtin;
        comptime fn values() -> List<bool> {
            var flag = false;
            let first: List<bool> = [do { flag = true; flag }, do { flag = false; flag }];
            concat(first, [not flag])
        }
        comptime fn types() -> List<Type> { concat([bool], [Integer]) }
        comptime fn nested() -> List<List<bool>> { [[true], [false]] }
        comptime fn snapshots() -> List<bool> {
            var values: List<bool> = [false];
            let saved = values;
            values = concat(values, [true]);
            concat(saved, values)
        }
        comptime fn count(input: List<bool>) -> Integer { length(input) }
        comptime fn early() -> bool {
            let values: List<bool> = [do { return true; false }, do { while true {} false }];
            false
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for name in ["values", "types", "nested", "snapshots", "early"] {
        let before = vm.bytecode_invocations();
        let id = support::id(&package, name);
        let result = vm.invoke(id, vec![]).unwrap();
        assert_eq!(vm.bytecode_invocations() - before, 1, "{name}");
        match (name, result) {
            ("values", Value::List { values, .. }) => assert_eq!(
                values,
                vec![Value::Bool(true), Value::Bool(false), Value::Bool(true)]
            ),
            ("types", Value::TypeList(types)) => assert_eq!(
                types,
                vec![
                    Term::Primitive(Primitive::Bool),
                    Term::Primitive(Primitive::Integer)
                ]
            ),
            ("snapshots", Value::List { values, .. }) => assert_eq!(
                values,
                vec![Value::Bool(false), Value::Bool(false), Value::Bool(true)]
            ),
            ("early", Value::Bool(true)) => {}
            ("nested", Value::List { values, .. }) => assert_eq!(values.len(), 2),
            _ => panic!("wrong result for {name}"),
        }
    }
    let values = vm.invoke(support::id(&package, "values"), vec![]).unwrap();
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(support::id(&package, "count"), vec![values])
            .unwrap(),
        Value::Integer(comptime_runtime::Integer::from_u128(3))
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
}
