//! Staged callable identities preserve caller Type bindings without executing code.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn returned_runtime_function_identities_supply_scoped_defaults() {
    let source = "typealias Type=.Type'builtin;
        fn identity(value:T)->T where T {value}
        comptime fn callback(comptime T:Type)->fn(T)->T {identity<T>}
        fn dispatch(comptime T:Type,comptime .action:fn(T)->T=callback(T),value:T)->T {action(value)}
        fn run()->bool {dispatch(bool,true)}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn returned_comptime_function_instances_supply_scoped_defaults() {
    let source = "typealias Type=.Type'builtin;
        comptime fn identity(value:T)->T where T {value}
        comptime fn callback(comptime T:Type)->comptime fn(T)->T {identity<T>}
        comptime fn dispatch(comptime T:Type,comptime .action:comptime fn(T)->T=callback(T),value:T)->T {action(value)}
        const answer=dispatch(bool,true);
        comptime fn run()->bool {answer}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
