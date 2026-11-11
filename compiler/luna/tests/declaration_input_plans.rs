//! Type constructors and callables share source-order declaration input plans.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Event, Interpreter, Value};
use elaboration::materialization::SourceInvocationQueries;

fn check_both(source: &str, calls: &[&str]) {
    let package = sysroot::compile(source).unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "check")
        .unwrap()
        .declaration;
    let context = elaboration::ptc::Context::from_package(&package).unwrap();
    let queries = SourceInvocationQueries::new(&package, &context, &[]);
    let solver = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
        queries.check(predicate, Default::default(), fuel)
    };
    let methods = |callee: &nir::program::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
    let associated = |access: &nir::program::Expr, fuel: &mut u64| queries.associated(access, fuel);
    for tree in [false, true] {
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, Budget::default())
        } else {
            Interpreter::for_comptime(&package, Budget::default())
        }
        .unwrap()
        .with_budgeted_type_solver(&solver)
        .with_method_resolver(&methods)
        .with_associated_resolver(&associated);
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

#[test]
fn callable_packet_prefix_receives_context_before_a_list_collector() {
    check_both(
        "
        comptime fn prefix() -> R
        where R, requires R == Tuple<u8, bool>
        { (7, true) }
        comptime fn empty() -> Tuple<> { () }
        comptime fn flag() -> bool { true }
        comptime fn take() -> bool
        where value: u8, enabled: bool, ...flags: meta.List<bool>
        { value == 7 and enabled and flags.length() == 1 }
        comptime fn check() -> bool {
            let selected = take<...empty(), ...prefix(), flag()>;
            selected() and selected()
        }
        ",
        &["check", "empty", "prefix", "flag", "take", "take"],
    );
}

#[test]
fn callable_packet_defaults_follow_all_explicit_sources_and_keep_prefix_values() {
    check_both(
        "
        comptime fn prefix() -> R
        where R, requires R == Tuple<u8, bool>
        { (7, true) }
        comptime fn tail() -> meta.List<bool> { [true, false] }
        comptime fn mark() -> bool { true }
        comptime fn outer() -> bool where seed: bool {
            comptime fn take() -> bool
            where value: u8, .copy: u8 = do { mark(); value }, enabled: bool,
                  ...flags: meta.List<bool>
            { value == copy and enabled and seed and flags.length() == 2 }
            let selected = take<...prefix(), ...tail()>;
            selected() and selected()
        }
        comptime fn check() -> bool { outer<true>() }
        ",
        &["check", "outer", "prefix", "tail", "mark", "take", "take"],
    );
}

#[test]
fn callable_packet_prefix_exit_skips_collector_defaults_and_formation() {
    check_both(
        "
        comptime fn prefix() -> R
        where R, requires R == Tuple<u8, bool>
        { (7, true) }
        comptime fn later() -> bool { true }
        comptime fn take() -> bool
        where value: u8, .tag: bool = later(), enabled: bool, ...flags: meta.List<bool>
        { enabled }
        comptime fn check() -> bool {
            let selected = take<...do { return true; prefix() }, later()>;
            false
        }
        ",
        &["check"],
    );
}

#[test]
fn callable_collector_packet_context_keeps_producer_requirements_and_range_checks() {
    let source = "
        comptime fn prefix() -> R
        where R, requires R == Tuple<u8, bool>
        { BODY }
        comptime fn take() -> bool
        where value: u8, enabled: bool, ...flags: meta.List<bool>
        { enabled }
        comptime fn check() -> bool { take<...prefix(), true>() }
    ";
    for body in ["(256, true)", "(7, 8)", "(7, true, false)"] {
        assert!(
            sysroot::compile(&source.replace("BODY", body)).is_err(),
            "{body}"
        );
    }
    assert!(
        sysroot::compile(
            &source
                .replace("BODY", "(7, true)")
                .replace("R == Tuple<u8, bool>", "R == Tuple<bool, u8>")
        )
        .is_err()
    );
}

#[test]
fn nominal_packet_prefix_receives_context_before_a_list_collector() {
    for declaration in [
        "struct Packet where value: u8, enabled: bool, ...flags: meta.List<bool> {}",
        "newtype Packet<value: u8, enabled: bool, ...flags: meta.List<bool>> = bool;",
    ] {
        check_both(
            &format!(
                "
                {declaration}
                comptime fn prefix() -> R
                where R, requires R == Tuple<u8, bool>
                {{ (7, true) }}
                comptime fn empty() -> Tuple<> {{ () }}
                comptime fn flag() -> bool {{ true }}
                comptime fn check() -> bool {{
                    let selected = Packet<...empty(), ...prefix(), flag()>;
                    selected == Packet<7, true, .flags = [true]>
                }}
                "
            ),
            &["check", "empty", "prefix", "flag"],
        );
    }
}

#[test]
fn member_input_plans_keep_caller_arguments_and_selected_parent_defaults() {
    check_both(
        "
        comptime fn prefix(flag: bool) -> R
        where R, requires R == Tuple<u8, bool>
        { (7, flag) }
        comptime fn tail() -> meta.List<bool> { [true, false] }
        comptime fn mark() -> bool { true }
        struct Holder where seed: bool {
            pub newtype Packet<value: u8, .copy: bool = do { mark(); seed },
                               enabled: bool, ...flags: meta.List<bool>> = bool;
            pub comptime fn Build() -> Type {
                Holder<false>.Packet<...prefix(seed), ...tail()>
            }
        }
        comptime fn check() -> bool {
            Holder<true>.Build() ==
                Holder<false>.Packet<7, .copy = false, true, .flags = [true, false]>
        }
        ",
        &["check", "Build", "prefix", "tail", "mark"],
    );
}

#[test]
fn nominal_collectors_reject_wrong_or_mixed_supplies_and_keep_opaque_identity() {
    let source = "
        newtype Flags<...values: meta.List<bool>> = bool;
        comptime fn check() -> bool { Flags<INPUT> == Flags<true, false> }
    ";
    for input in [
        "1u8",
        ".values = [1u8]",
        "true, .values = [false]",
        ".values = [true], false",
        ".values = [true], .values = [false]",
        "...(true, false)",
        "...()",
        ".missing = [true]",
    ] {
        assert!(
            sysroot::compile(&source.replace("INPUT", input)).is_err(),
            "accepted {input}"
        );
    }
    check_both(
        "
        newtype Flags<...values: meta.List<bool>> = bool;
        comptime fn check() -> bool {
            Flags<true, false> == Flags<.values = [true, false]>
                and Flags<> != Flags<true>
        }
        ",
        &["check"],
    );
}

#[test]
fn nominal_collectors_use_ordinary_values_in_dependent_and_type_domains() {
    check_both(
        "
        newtype Entries<T, ...items: meta.List<T>> = bool;
        comptime fn Byte() -> Type { u8 }
        comptime fn check() -> bool {
            Entries<.items = [1, 255], Byte()> == Entries<u8, .items = [1, 255]>
                and Entries<Type, bool, u8> == Entries<Type, .items = [bool, u8]>
        }
        ",
        &["check", "Byte"],
    );
}
