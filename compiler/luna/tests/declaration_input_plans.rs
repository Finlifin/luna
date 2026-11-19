//! Type constructors and callables share source-order declaration input plans.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Event, Value};

fn check_both(source: &str, calls: &[&str]) {
    check_entry(&sysroot::compile(source).unwrap(), "check", calls);
}

fn check_entry(package: &nir::program::Package, name: &str, calls: &[&str]) {
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap()
        .declaration;
    for tree in [false, true] {
        sysroot::with_comptime(package, sysroot::COMPTIME_BUDGET, tree, |vm| {
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
        });
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

#[test]
fn spread_field_domains_follow_declaration_slots_across_named_defaults() {
    let source = "
        pure comptime fn Packet<.extra: bool = false, T, .small: u8 = 7, value: T, tail: u8> -> Type { T }
        comptime fn mark() -> bool { true }
        comptime fn check() -> bool {
            let packet = Packet<...(bool, mark(), 255)>;
            packet == Packet<bool, true, 255>
        }
    ";
    check_both(source, &["check", "mark"]);
    let invalid = source.replace("mark(), 255", "mark(), 256");
    let failure = sysroot::compile(&invalid).unwrap_err();
    assert!(
        failure.contains("does not fit the requested fixed-width representation"),
        "{failure}"
    );
}

#[test]
fn checked_packet_width_routes_subsequent_spreads_into_the_collector() {
    let source = "
        pure comptime fn Packet<a: u8, b: u8, .flag: bool = false, ...tail: meta.List<u8>> -> Type { bool }
        comptime fn verify() -> bool
        where a: u8, b: u8, .flag: bool = false, ...tail: meta.List<u8>
        {
            a == 1 and b == 2 and flag and tail.length() == 3
                and tail[0] == 3 and tail[1] == 255 and tail[2] == 4
        }
        comptime fn pair() -> Tuple<u8, u8> { (1, 2) }
        comptime fn field() -> u8 { 3 }
        comptime fn flag() -> bool { true }
        comptime fn last() -> meta.List<u8> { [4] }
        comptime fn check() -> bool {
            let head = Packet<INPUT>;
            head == Packet<1, 2, .flag = true, .tail = [3, 255, 4]> and verify<INPUT>()
        }
    ".replace("INPUT", "
        ...do { let local_pair = pair(); local_pair },
        ...[field(), 255], .flag = flag(), ...last()
    ");
    check_both(
        &source,
        &[
            "check", "pair", "field", "flag", "last", "pair", "field", "flag", "last", "verify",
        ],
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
fn nested_domain_prechecks_resume_their_consumer_without_reordering_execution() {
    for declaration in CONSTRUCTORS {
        check_both(
            &format!(
                "
                {declaration}
                comptime fn first() -> bool {{ true }}
                comptime fn inner_first() -> bool {{ true }}
                comptime fn inner_last() -> bool {{ true }}
                comptime fn last() -> bool {{ true }}
                comptime fn take() -> bool
                where T, .values: meta.List<T> = []
                {{ values.length() == 1 }}
                comptime fn check() -> bool {{
                    let selected = Packet<
                        .values = do {{ first(); [1, 255] }},
                        do {{
                            let inner = take<
                                .values = do {{ inner_first(); [true] }},
                                do {{ inner_last(); bool }}
                            >;
                            let valid = inner();
                            last();
                            u8
                        }}
                    >;
                    selected == Packet<u8, .values = [1, 255]>
                }}
                "
            ),
            &[
                "check",
                "first",
                "inner_first",
                "inner_last",
                "take",
                "last",
            ],
        );
    }
}

#[test]
fn nested_dependency_failure_is_not_hidden_by_an_early_outer_return() {
    for declaration in CONSTRUCTORS {
        let failure = sysroot::compile(&format!(
            "
            {declaration}
            comptime fn take() -> bool
            where T, .values: meta.List<T> = []
            {{ values.length() == 1 }}
            comptime fn check() -> bool {{
                let selected = Packet<
                    .values = do {{ return true; [1] }},
                    do {{ let inner = take<.values = [256], u8>; u8 }}
                >;
                false
            }}
            "
        ))
        .err()
        .expect("unreachable execution must not hide an invalid dependency input");
        assert!(
            failure.contains("does not fit the requested fixed-width representation"),
            "{failure}"
        );
    }
}

#[test]
fn nested_slot_defaults_keep_each_prefix_and_caller_names_separate() {
    check_both(
        "
        comptime fn copied() -> bool
        where value: bool, .copy: bool = value
        { copy }
        comptime fn select() -> bool
        where first: bool, .second: bool = copied<first>(), .third: bool = second
        { first and second and third }
        struct Mark
        where first: bool, .second: bool = copied<first>(), .third: bool = second
        {}
        comptime fn check() -> bool {
            let first = false;
            let second = false;
            let third = false;
            let yes = select<true>;
            let no = select<false>;
            let positive = yes();
            let negative = no();
            let marker = Mark<true>;
            positive and not negative
                and marker == Mark<true, .second = true, .third = true>
                and not first and not second and not third
        }
        ",
        &["check", "copied", "copied", "select", "select", "copied"],
    );
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
fn sibling_input_preparations_retain_distinct_selected_parent_environments() {
    check_both(
        "
        comptime fn copied() -> bool
        where flag: bool, .copy: bool = flag
        { copy }
        struct Holder where seed: bool {
            pub newtype Packet<value: bool, .copy: bool = copied<seed>(),
                               ...flags: meta.List<bool>> = bool;
            pub comptime fn Build() -> Type {
                let first = Holder<false>.Packet<seed, ...[seed]>;
                let second = Holder<true>.Packet<seed, ...[seed]>;
                if seed { first } else { second }
            }
        }
        comptime fn check() -> bool {
            let first = Holder<true>.Build();
            let second = Holder<false>.Build();
            first == Holder<false>.Packet<true, .copy = false, .flags = [true]>
                and second == Holder<true>.Packet<false, .copy = true, .flags = [false]>
        }
        ",
        &[
            "check", "Build", "copied", "copied", "Build", "copied", "copied",
        ],
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

#[test]
fn dependent_nominal_domains_receive_collection_and_callable_evidence() {
    check_both(
        "
        struct Entry where items: meta.List<u8>, callback: comptime fn(bool) -> bool,
                           T, field: T, flag: bool {}
        comptime fn items() -> meta.List<u8> { [1, 255] }
        comptime fn keep(value: bool) -> bool { value }
        comptime fn Domain() -> Type { Type }
        comptime fn check() -> bool {
            Entry<items(), keep, Domain(), bool, true> == Entry<[1, 255], keep, Type, bool, true>
        }
        ",
        &["check", "items", "Domain"],
    );
}

#[test]
fn ascribed_normal_form_constants_use_public_slots_and_implementation_contracts() {
    let declarations = r#"
        pure comptime fn Head<T, ...Actual: meta.List<T>> -> Type
        where requires T == bool
        { T }
        const Factory: pure comptime fn<U: Type, ...Public: meta.List<U>> -> Type = Head;
        pure comptime fn Defaulted<.hidden: Type = u8> -> Type { bool }
        const Fixed: pure comptime fn<Type> -> Type = Defaulted;
    "#;
    let source = format!(
        r#"{declarations}
        comptime fn select() -> pure comptime fn<V: Type, ...Inner: meta.List<V>> -> Type {{ Head }}
        const Computed: pure comptime fn<W: Type, ...Fields: meta.List<W>> -> Type = select();
        typealias Selected = Factory<bool, .Public = [true, false]>;
        typealias Explicit = Fixed<bool>;
        comptime fn check() -> bool {{
            Selected == Head<bool, true, false>
                and Factory<bool, .Public = []> == Head<bool>
                and Computed<bool, .Fields = [true, false]> == Selected
                and Explicit == Defaulted<.hidden = bool>
                and Fixed<u8> == Defaulted<>
        }}
    "#
    );
    check_both(&source, &["check"]);
    for (application, diagnostic) in [
        (
            "Factory<bool, .Actual = [true]>",
            "unknown named normal-form argument",
        ),
        ("Factory<bool, .Public = [7]>", "fixed-width integer target"),
        (
            "Factory<u8, .Public = [7]>",
            "type requirement is not satisfied",
        ),
        ("Fixed<>", "argument count mismatch"),
        ("Fixed<.hidden = u8>", "unknown named normal-form argument"),
    ] {
        for context in [
            "typealias Bad = CALL;",
            "comptime fn Bad() -> Type { CALL }",
        ] {
            let invalid = format!("{declarations} {}", context.replace("CALL", application));
            let failure = sysroot::compile(&invalid).unwrap_err();
            assert!(failure.contains(diagnostic), "{application}: {failure}");
        }
    }
}

#[test]
fn ascription_survives_type_aliases_reexports_and_declared_call_results() {
    let declarations = r#"
        pure comptime fn Head<T, ...Actual: meta.List<T>> -> Type { T }
        typealias Factory = pure comptime fn<T: Type, ...Public: meta.List<T>> -> Type;
        typealias Alias = Factory;
        const First: Alias = Head;
        const Second = First;
        comptime fn factory() -> Factory { Head }
        const Selected = factory();
        pure comptime fn Defaulted<.hidden: Type = u8> -> Type { bool }
        typealias Required = pure comptime fn<Type> -> Type;
        const Fixed: Required = Defaulted;
    "#;
    let accepted = format!(
        r#"{declarations}
        typealias Result = First<bool, .Public = [true]>;
        comptime fn check() -> bool {{
            Second<bool, .Public = [true]> == Result
                and Selected<bool, .Public = [true]> == Head<bool, true>
                and Fixed<bool> == Defaulted<.hidden = bool>
        }}
    "#
    );
    check_both(&accepted, &["check"]);
    for application in [
        "Second<bool, .Actual = [true]>",
        "Selected<bool, .Actual = [true]>",
        "Fixed<.hidden = bool>",
        "Fixed<>",
    ] {
        let invalid = format!("{declarations} typealias Bad = {application};");
        let failure = sysroot::compile(&invalid).unwrap_err();
        assert!(
            failure.contains("unknown named normal-form argument")
                || failure.contains("argument count mismatch"),
            "{application}: {failure}"
        );
    }
}

#[test]
fn higher_order_normal_heads_use_the_receiving_collector_name() {
    let source = include_str!("../../../development/repros/dynamic-nf-interface-forwarding.fl");
    let package = sysroot::compile(source).unwrap();
    check_entry(
        &package,
        "verify",
        &[
            "verify", "select", "forward", "Apply", "select", "forward", "Apply",
        ],
    );
    let names: Vec<_> = package
        .normal_forms
        .iter()
        .filter_map(|head| head.collector.as_ref())
        .filter(|entry| entry.slot == 0)
        .filter_map(|entry| entry.name.as_deref())
        .collect();
    for required in ["Left", "Right", "Es"] {
        assert!(
            names.contains(&required),
            "missing published collector {required}: {names:?}"
        );
    }
    for label in ["Left", "Right"] {
        let invalid = source.replace("factory<.Items", &format!("factory<.{label}"));
        let error = sysroot::compile(&invalid).unwrap_err();
        assert!(
            error.contains("unknown named normal-form argument"),
            "{error}"
        );
    }
    let builtin = include_str!("../../../development/repros/dynamic-nf-builtin-input.fl");
    assert_eq!(sysroot::run(builtin), Value::Bool(true));
}
