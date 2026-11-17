//! Ordinary parameter dependencies retain checking progress and source producers.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Event, Value};

#[test]
fn long_default_dependency_chains_are_acyclic_and_keep_producers_once() {
    let count = 72;
    let mut parameters = vec!["comptime .T0: Type = do { mark(); bool }".to_owned()];
    for slot in 1..count {
        parameters.push(format!("comptime .T{slot}: Type = T{}", slot - 1));
    }
    let source = format!(
        "
        comptime fn mark() -> bool {{ true }}
        comptime fn accept({}) -> bool
        where requires T{} == bool
        {{ true }}
        comptime fn check() -> bool {{ accept() and accept() }}
        ",
        parameters.join(", "),
        count - 1,
    );
    let package = sysroot::compile(&source).unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "check")
        .unwrap()
        .declaration;
    let marker = package
        .functions
        .iter()
        .find(|f| f.name == "mark")
        .unwrap()
        .declaration;
    for tree in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, tree, |vm| {
            assert_eq!(
                vm.invoke(entry, vec![]).unwrap(),
                Value::Bool(true),
                "tree={tree}"
            );
            let enters = vm
                .events()
                .iter()
                .filter(|event| matches!(event, Event::EnterFunction(id) if *id == marker))
                .count();
            assert_eq!(
                enters, 2,
                "tree={tree}: each invocation executes its root default once"
            );
        });
    }
}

#[test]
fn value_index_dependencies_preserve_source_order_and_default_forwarding() {
    let source = "
        struct Flag where enabled: bool {}
        comptime fn accept(comptime enabled: bool, comptime .copy: bool = enabled, value: Flag<copy>) -> bool {
            copy
        }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let first = accept(do { trace = trace * 10 + 1; true }, Flag<true>{});
            let second = accept(do { trace = trace * 10 + 2; false }, Flag<false>{});
            first and not second and trace == 12
        }
    ";
    let package = sysroot::compile(source).unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "check")
        .unwrap()
        .declaration;
    for tree in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, tree, |vm| {
            assert_eq!(
                vm.invoke(entry, vec![]).unwrap(),
                Value::Bool(true),
                "tree={tree}"
            );
        });
    }
    let invalid = source.replace("false }, Flag<false>", "false }, Flag<true>");
    assert!(sysroot::compile(&invalid).is_err());
}
