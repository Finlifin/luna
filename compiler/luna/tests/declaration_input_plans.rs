//! NF, nominal, and alias inputs retain one source-order preparation contract.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Event, Interpreter, Value};

fn check_both(source: &str, calls: &[&str]) {
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
        let observed = vm
            .events()
            .iter()
            .filter_map(|event| match event {
                Event::EnterFunction(id) if id.package == entry.package => package
                    .functions
                    .iter()
                    .find(|f| f.declaration == *id)
                    .map(|f| f.name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(observed, calls, "tree={tree}");
    }
}

#[test]
fn static_domains_check_all_branches_for_nominal_and_alias_inputs() {
    for declaration in [
        "struct Packet where T, .values: meta.List<T> = [] {}",
        "typealias Packet<T, .values: meta.List<T> = []> = Tuple<T>;",
    ] {
        let source = format!(
            "
            {declaration}
            comptime fn mark() -> bool {{ true }}
            comptime fn Bad() -> Type {{
                Packet<.values = if true {{ [1] }} else {{ [256] }}, do {{ mark(); u8 }}>
            }}
        "
        );
        let failure = sysroot::compile(&source)
            .err()
            .expect("invalid branch was accepted");
        assert!(
            failure.contains("does not fit the requested fixed-width representation"),
            "{failure}"
        );
    }
}

#[test]
fn nominal_packet_producers_receive_their_known_result_context() {
    check_both(
        "
        struct Packet where T, flag: bool, value: T {}
        comptime fn prefix() -> R
        where R, requires R == Tuple<Type, bool>
        { (u8, true) }
        comptime fn check() -> bool {
            Packet<...prefix(), 7> == Packet<u8, true, 7>
        }
    ",
        &["check", "prefix"],
    );
}

const CONSTRUCTORS: &[&str] = &[
    "pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type { bool }",
    "struct Packet where T, .values: meta.List<T> = [] {}",
    "typealias Packet<T, .values: meta.List<T> = []> = Tuple<T>;",
];

#[test]
fn all_constructors_preserve_named_input_source_order() {
    for declaration in CONSTRUCTORS {
        check_both(
            &format!(
                "
            {declaration}
            comptime fn first() -> bool {{ true }}
            comptime fn last() -> bool {{ true }}
            comptime fn check() -> bool {{
                let value = Packet<
                    .values = do {{ first(); if true {{ [1, 255] }} else {{ [] }} }},
                    do {{ last(); let selected = u8; selected }}
                >;
                value == Packet<u8, .values = [1, 255]>
            }}
        "
            ),
            &["check", "first", "last"],
        );
    }
}

#[test]
fn all_constructors_retain_invocation_dependent_domains() {
    for declaration in CONSTRUCTORS {
        check_both(
            &format!(
                "
            {declaration}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn check() -> bool {{
                Packet<.values = [1, 255], Byte()> == Packet<u8, .values = [1, 255]>
            }}
        "
            ),
            &["check", "Byte"],
        );
    }
}

#[test]
fn all_constructors_keep_early_return_before_later_producers() {
    for declaration in CONSTRUCTORS {
        check_both(
            &format!(
                "
            {declaration}
            comptime fn later() -> bool {{ true }}
            comptime fn check() -> bool {{
                let value = Packet<
                    .values = do {{ return true; [1] }},
                    do {{ later(); u8 }}
                >;
                false
            }}
        "
            ),
            &["check"],
        );
    }
}
