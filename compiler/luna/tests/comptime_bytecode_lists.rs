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

#[test]
fn list_indices_preserve_bounds_snapshots_and_type_values() {
    use comptime_interpretor::ErrorKind;
    let package = support::compile_with_tuple(
        "typealias Type = .Type'builtin;
        typealias Integer = .Integer'builtin;
        typealias List<T> = TupleElements<T>;
        comptime fn values() -> List<bool> { [true, false] }
        comptime fn types() -> List<Type> { [bool, Integer] }
        comptime fn at(values: List<bool>, index: Integer) -> bool { values[index] }
        comptime fn Element(values: List<Type>, index: Integer) -> Type { values[index] }
        comptime fn snapshot() -> bool {
            var values: List<bool> = [true];
            let old = values[do { values = [false]; 0 }];
            old and not values[0]
        }
        comptime fn early() -> bool {
            let values: List<bool> = [true];
            values[do { return true; 0 }]
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let values = vm.invoke(support::id(&package, "values"), vec![]).unwrap();
    let types = vm.invoke(support::id(&package, "types"), vec![]).unwrap();
    for (name, input, expected) in [
        ("at", values, Value::Bool(false)),
        (
            "Element",
            types,
            Value::Type(Term::Primitive(Primitive::Integer)),
        ),
    ] {
        for index in [-1, 2, i128::MAX] {
            let before = vm.bytecode_invocations();
            assert_eq!(
                vm.invoke(
                    support::id(&package, name),
                    vec![
                        input.clone(),
                        Value::Integer(comptime_runtime::Integer::from_i128(index))
                    ]
                )
                .unwrap_err()
                .kind,
                ErrorKind::IndexOutOfBounds
            );
            assert_eq!(vm.bytecode_invocations() - before, 1);
        }
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(
                support::id(&package, name),
                vec![
                    input,
                    Value::Integer(comptime_runtime::Integer::from_u128(1))
                ]
            )
            .unwrap(),
            expected
        );
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
    for name in ["snapshot", "early"] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(support::id(&package, name), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}

#[test]
fn repeated_append_keeps_copies_and_callback_snapshots() {
    let package = support::compile_with_tuple(
        "typealias Type = .Type'builtin;
        typealias Integer = .Integer'builtin;
        typealias List<T> = TupleElements<T>;
        const concat = .list_concat'builtin;
        comptime fn copies() -> List<bool> {
            var values: List<bool> = [true];
            let saved = values;
            values = concat(values, values);
            values = concat(values, [false]);
            concat(saved, values)
        }
        comptime fn snapshot() -> List<bool> {
            var values: List<bool> = [true];
            values = concat(values, do { values = [false]; [true] });
            values
        }
        comptime fn types() -> List<Type> {
            var values: List<Type> = [bool];
            values = concat(values, [Integer]);
            values
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (name, expected) in [
        ("copies", vec![true, true, true, false]),
        ("snapshot", vec![true, true]),
    ] {
        let before = vm.bytecode_invocations();
        let Value::List { values, .. } = vm.invoke(support::id(&package, name), vec![]).unwrap()
        else {
            panic!("list")
        };
        assert_eq!(
            values,
            expected.into_iter().map(Value::Bool).collect::<Vec<_>>()
        );
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
    assert_eq!(
        vm.invoke(support::id(&package, "types"), vec![]).unwrap(),
        Value::TypeList(vec![
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::Integer)
        ])
    );
}
