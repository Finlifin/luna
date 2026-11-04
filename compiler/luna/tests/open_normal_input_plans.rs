//! Suspended NF descriptions share concrete input routing and ordinary domains.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Interpreter};

fn check_application(head: &str, application: &str, concrete: &str) {
    let source = format!(
        "{head}
        fn identity(value: {concrete}) -> {concrete} {{ value }}
        comptime fn keep(T: Type, value: u8, items: meta.List<u8>, tag: bool,
            callback: defer fn({application}) -> {application})
            -> defer fn({application}) -> {application}
        {{ callback }}
        const callback = keep(u8, 7, [8, 9], true, identity);
        typealias Callback = callback'type;
        fn run(selected: Callback, value: {concrete}) -> {concrete} {{ selected(value) }}
        comptime fn Actual() -> Type {{ Callback }}
        comptime fn Expected() -> Type {{ fn({concrete}) -> {concrete} }}
        "
    );
    let package = sysroot::compile(&source).unwrap_or_else(|e| panic!("{application}: {e}"));
    luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let id = |name| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    for tree in [false, true] {
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, Budget::default())
        } else {
            Interpreter::for_comptime(&package, Budget::default())
        }
        .unwrap();
        let actual = vm.invoke(id("Actual"), vec![]).unwrap();
        assert_eq!(
            actual,
            vm.invoke(id("Expected"), vec![]).unwrap(),
            "{application}"
        );
    }
}

#[test]
fn suspended_collections_use_actual_element_domains_with_optional_inputs() {
    let head =
        "pure comptime fn Packet<T, .tag: bool = false, ...values: meta.List<u8>> -> Type { bool }";
    for (application, concrete) in [
        ("Packet<T, value>", "Packet<u8, 7>"),
        ("Packet<T, value, ...items>", "Packet<u8, 7, 8, 9>"),
        (
            "Packet<T, ...items, .tag = tag>",
            "Packet<u8, 8, 9, .tag = true>",
        ),
        (
            "Packet<.values = items, .tag = tag, T>",
            "Packet<u8, 8, 9, .tag = true>",
        ),
        (
            "Packet<...(T,), value, .tag = tag>",
            "Packet<u8, 7, .tag = true>",
        ),
        ("Packet<T>", "Packet<u8>"),
    ] {
        check_application(head, application, concrete);
    }
}

#[test]
fn suspended_mixed_prefix_and_dependent_collections_resume_concrete_checks() {
    let head = "pure comptime fn Packet<T, first: T, .tag: bool = false, ...values: meta.List<T>> -> Type { bool }";
    for (application, concrete) in [
        ("Packet<T, value, value>", "Packet<u8, 7, 7>"),
        ("Packet<T, value, ...items>", "Packet<u8, 7, 8, 9>"),
        (
            "Packet<.values = items, T, value, .tag = tag>",
            "Packet<u8, 7, 8, 9, .tag = true>",
        ),
        ("Packet<...(T, value), ...items>", "Packet<u8, 7, 8, 9>"),
    ] {
        check_application(head, application, concrete);
    }
}

#[test]
fn suspended_collection_routes_reject_conflicts_and_known_wrong_domains() {
    let head =
        "pure comptime fn Packet<T, .tag: bool = false, ...values: meta.List<u8>> -> Type { bool }";
    for (application, diagnostic) in [
        ("Packet<T, T>", "type"),
        ("Packet<T, .values = value>", "type"),
        ("Packet<T, ...value>", "type"),
        (
            "Packet<T, value, .values = items>",
            "cannot combine a complete",
        ),
        (
            "Packet<.values = items, T, value>",
            "cannot combine a complete",
        ),
        (
            "Packet<T, .values = items, .values = items>",
            "duplicate normal-form argument",
        ),
        (
            "Packet<T, .tag = true, .tag = false>",
            "duplicate normal-form argument",
        ),
        (
            "Packet<T, .missing = items>",
            "unknown named normal-form argument",
        ),
        (
            "Packet<.values = items>",
            "normal-form argument count mismatch",
        ),
        ("Packet<T, ... (value,)>", "type"),
    ] {
        let source = format!(
            "{head}
            comptime fn reject(T: Type, value: u8, items: meta.List<u8>,
                callback: defer fn({application}) -> bool) -> bool {{ true }}"
        );
        let error = sysroot::compile(&source).unwrap_err();
        assert!(error.contains(diagnostic), "{application}: {error}");
    }
}

#[test]
fn suspended_collectors_follow_late_optional_type_inputs() {
    let head = "pure comptime fn Packet<.T: Type = u8, .tag: bool = false, ...values: meta.List<T>> -> Type { bool }";
    for (application, concrete) in [
        ("Packet<value, .T = T>", "Packet<7>"),
        (
            "Packet<...items, .tag = tag, .T = T>",
            "Packet<8, 9, .tag = true>",
        ),
        ("Packet<.values = items, .T = T>", "Packet<8, 9>"),
    ] {
        check_application(head, application, concrete);
    }
}

#[test]
fn suspended_collection_domains_are_checked_again_after_type_inputs_arrive() {
    for (ty, element, valid) in [("u8", "1", true), ("bool", "true", false)] {
        let source = format!(
            "pure comptime fn Packet<T, ...values: meta.List<T>> -> Type {{ bool }}
            fn identity(value: Packet<{ty}, {element}>) -> Packet<{ty}, {element}> {{ value }}
            comptime fn keep(T: Type, items: meta.List<u8>,
                callback: defer fn(Packet<T, ...items>) -> Packet<T, ...items>)
                -> defer fn(Packet<T, ...items>) -> Packet<T, ...items>
            {{ callback }}
            const selected = keep({ty}, [1], identity);"
        );
        let result = sysroot::compile(&source);
        if valid {
            result.unwrap();
        } else {
            assert!(
                result.is_err(),
                "dependent list element mismatch was accepted"
            );
        }
    }
}
