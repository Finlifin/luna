//! Aggregate indices use ordinary contextual checking and required invocation.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Interpreter, Value};

fn check_comptime_and_runtime(source: &str) {
    let package = sysroot::compile(source).unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "check")
        .unwrap()
        .declaration;
    for tree in [false, true] {
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, Budget::default())
        } else {
            Interpreter::for_comptime(&package, Budget::default())
        }
        .unwrap();
        assert_eq!(
            vm.invoke(entry, vec![]).unwrap(),
            Value::Bool(true),
            "tree={tree}"
        );
    }
    let runtime = source.replace("comptime fn check()", "fn run()");
    assert_eq!(sysroot::run(&runtime), Value::Bool(true));
}

#[test]
fn aggregate_annotations_retain_generic_inputs_and_check_the_whole_domain() {
    for (domain, generic, concrete, wrong) in [
        (
            "Tuple<bool, bool>",
            "(flag, false)",
            "(true, false)",
            "(false, false)",
        ),
        (
            "meta.List<bool>",
            "[flag, false]",
            "[true, false]",
            "[false, false]",
        ),
        (
            "State",
            "State { flag: flag }",
            "State { flag: true }",
            "State { flag: false }",
        ),
        (
            "Tuple<meta.List<bool>, bool>",
            "([flag], false)",
            "([true], false)",
            "([false], false)",
        ),
        (
            "meta.List<Tuple<bool, bool>>",
            "[(flag, false)]",
            "[(true, false)]",
            "[(false, false)]",
        ),
    ] {
        let source = format!(
            "
            struct State {{ flag: bool }}
            struct Packet where state: {domain} {{ payload: bool }}
            struct Holder where flag: bool {{ value: Packet<{generic}> }}
            comptime fn check() -> bool {{
                let holder = Holder<true> {{ value: Packet<{concrete}> {{ payload: true }} }};
                holder.value.payload
            }}
        "
        );
        check_comptime_and_runtime(&source);
        let invalid = source.replace(&format!("Packet<{concrete}>"), &format!("Packet<{wrong}>"));
        let error = sysroot::compile(&invalid).unwrap_err();
        assert!(
            error.contains("type mismatch") || error.contains("unresolved type equality"),
            "{domain}: {error}"
        );
    }
}

#[test]
fn aggregate_annotations_check_type_valued_leaves_without_unfolding_heads() {
    for (domain, generic, concrete, wrong) in [
        (
            "meta.List<Type>",
            "[T, Identity<T>]",
            "[bool, Identity<bool>]",
            "[bool, bool]",
        ),
        (
            "Tuple<Type, Type>",
            "(T, Identity<T>)",
            "(bool, Identity<bool>)",
            "(bool, bool)",
        ),
    ] {
        let source = format!(
            "
        pure comptime fn Identity<T> -> Type {{ T }}
        struct Packet where types: {domain} {{ payload: bool }}
        struct Holder where T {{ value: Packet<{generic}> }}
        comptime fn check() -> bool {{
            let holder = Holder<bool> {{ value: Packet<{concrete}> {{ payload: true }} }};
            holder.value.payload
        }}
    "
        );
        check_comptime_and_runtime(&source);
        let wrong = source.replace(&format!("Packet<{concrete}>"), &format!("Packet<{wrong}>"));
        let error = sysroot::compile(&wrong).unwrap_err();
        assert!(error.contains("type mismatch"), "{error}");
    }
}
