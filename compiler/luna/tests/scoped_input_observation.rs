//! Embedded callable binders do not become inputs of their enclosing default.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::Value;

#[test]
fn defaults_keep_nested_callable_slots_separate_from_the_call_telescope() {
    let source = "
        fn identity(comptime T: Type, value: T) -> T { value }
        comptime fn Pick(flag: bool) -> Type { if flag { bool } else { u8 } }
        fn choose(
            comptime .witness: bool = do {
                let callback: fn(comptime U: Type, value: U) -> U = identity;
                true
            },
            comptime flag: bool,
            value: defer Pick(flag)
        ) -> defer Pick(flag) { value }
        fn run() -> bool { choose(true, true) and choose(false, 9) == 9 }
    ";
    let package = sysroot::compile(source).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn defaults_keep_outer_type_dependencies_inside_callable_annotations() {
    let source = "
        fn identity(comptime T: Type, value: T) -> T { value }
        comptime fn Inspect(comptime T: Type, .U: Type = do {
            let callback: comptime fn(value: T) -> T = |value: T| value;
            let nested = || T;
            nested()
        }) -> Type { U }
        fn run() -> bool {
            const first = Inspect(bool);
            const second = Inspect(u8);
            first == bool and second == u8
        }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}
