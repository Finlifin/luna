//! Clause packets retain ordinary domains and execute their producers once.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

fn check(source: &str, calls: &[&str]) {
    let package = sysroot::compile(source).unwrap();
    let id = |name| {
        package
            .functions
            .iter()
            .find(|function| function.name == name)
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
        let actual = vm.invoke(id("check"), vec![]).unwrap();
        let observed = vm
            .events()
            .iter()
            .filter_map(|event| match event {
                comptime_interpretor::Event::EnterFunction(declaration)
                    if declaration.package == id("check").package =>
                {
                    package
                        .functions
                        .iter()
                        .find(|function| function.declaration == *declaration)
                        .map(|function| function.name.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(observed, calls, "tree={tree}");
        assert_eq!(actual, Value::Bool(true), "tree={tree}");
    }
}

#[test]
fn mixed_clause_packets_receive_result_context_and_preserve_empty_producers() {
    check(
        "
        comptime fn packet() -> T
        where T, requires T == Tuple<u8, bool>
        {
            (7, true)
        }
        comptime fn empty() -> Tuple<> { () }
        comptime fn default_flag() -> bool { true }
        comptime fn take() -> bool
        where value: u8, .tag: bool = default_flag(), flag: bool
        {
            value == 7 and tag and flag
        }
        comptime fn check() -> bool { take<...empty(), ...packet(), ...empty()>() }
        ",
        &["check", "empty", "packet", "empty", "default_flag", "take"],
    );
}

#[test]
fn type_only_nominal_packets_share_preparation_without_reordering_defaults() {
    check(
        "
        comptime fn packet() -> T
        where T, requires T == Tuple<Type, Type>
        {
            (u8, bool)
        }
        comptime fn empty() -> Tuple<> { () }
        comptime fn Default() -> Type { u16 }
        newtype Marker<A, .C: Type = Default(), B> = bool;
        comptime fn check() -> bool {
            let ty = Marker<...empty(), ...packet(), ...empty()>;
            ty == Marker<u8, .C = u16, bool>
        }
        ",
        &["check", "empty", "packet", "empty", "Default"],
    );
}

#[test]
fn returning_packet_skips_its_tail_and_remaining_clause_producers() {
    check(
        "
        comptime fn packet() -> T
        where T, requires T == Tuple<u8, bool>
        {
            (7, true)
        }
        comptime fn later() -> bool { false }
        comptime fn take() -> bool
        where value: u8, flag: bool, .tag: bool = later()
        {
            flag
        }
        comptime fn check() -> bool {
            take<...do { return true; packet() }, .tag = later()>()
        }
        ",
        &["check"],
    );
}

#[test]
fn packet_context_does_not_override_producer_requirements_or_field_domains() {
    let source = "
        comptime fn packet() -> T
        where T, requires T == Tuple<u8, bool>
        {
            BODY
        }
        comptime fn take() -> bool where value: u8, flag: bool { flag }
        comptime fn check() -> bool { take<...packet()>() }
    ";
    for body in ["(256, true)", "(7, 8)", "(7, true, false)"] {
        assert!(
            sysroot::compile(&source.replace("BODY", body)).is_err(),
            "{body}"
        );
    }
    let wrong_requirement = source
        .replace("BODY", "(7, true)")
        .replace("T == Tuple<u8, bool>", "T == Tuple<bool, u8>");
    assert!(sysroot::compile(&wrong_requirement).is_err());
}
