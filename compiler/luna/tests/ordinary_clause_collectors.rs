//! Ordinary clause collectors use public list values and preserve invocation order.
use comptime_interpretor::Value;
#[path = "support/sysroot_execution.rs"]
mod sysroot;
use sysroot::{compile, run};

#[test]
fn ordinary_clause_collectors_accept_non_type_element_domains() {
    assert_eq!(
        run("
        comptime fn first() -> bool
        where ...values: meta.List<bool>
        {
            values[0]
        }
        fn run() -> bool { const result = first<.values = [true, false]>(); result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn clause_collectors_keep_element_contexts_empty_tails_and_named_supply() {
    assert_eq!(
        run("
        comptime fn count() -> Integer where T, ...values: meta.List<T> {
            values.length()
        }
        comptime fn check() -> bool {
            let flags: meta.List<bool> = [true, false];
            count<u8, 1, 255>() == 2
                and count<bool>() == 0
                and count<u8, .values = [1, 255]>() == 2
                and count<.values = flags, bool>() == 2
        }
        fn run() -> bool { const result = check(); result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn clause_collectors_evaluate_elements_spreads_and_named_inputs_in_source_order() {
    assert_eq!(
        run("
        comptime fn check() -> bool {
            comptime fn packed() -> bool
            where .tag: bool = true, ...values: meta.List<bool>
            {
                tag and values.length() == 4
                    and values[0] and not values[1] and values[2] and not values[3]
            }
            var state = false;
            let result = packed<
                do { state = not state; state },
                .tag = do { state = not state; not state },
                ...do { state = not state; let part: meta.List<bool> = [not state, state]; part },
                do { state = not state; state }
            >();
            result and not state
        }
        fn run() -> bool { const result = check(); result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn clause_collectors_snapshot_whole_inputs_and_run_empty_spread_producers() {
    assert_eq!(
        run("
        comptime fn count() -> Integer where ...values: meta.List<bool> {
            values.length()
        }
        comptime fn check() -> bool {
            var state = false;
            let empty = count<...do { state = true; let items: meta.List<bool> = []; items }>();
            var original: meta.List<bool> = [true];
            let saved = count<.values = original>;
            original = [true, false];
            state and empty == 0 and saved() == 1 and count<.values = original>() == 2
        }
        fn run() -> bool { const result = check(); result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn clause_collectors_keep_defaults_and_lexical_clause_inputs() {
    assert_eq!(
        run("
        comptime fn outer() -> bool where seed: bool {
            comptime fn check() -> bool
            where first: bool, .copy: bool = first, ...values: meta.List<bool>
            {
                first == copy and values.length() == 2 and values[0] == seed and not values[1]
            }
            check<true, seed, false>() and check<false, .copy = false, .values = [seed, false]>()
        }
        fn run() -> bool { const result = outer<true>(); result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn clause_collectors_unpack_fixed_prefix_without_consuming_the_list_tail() {
    assert_eq!(
        run("
        comptime fn prefix() -> Tuple<bool, u8> { (true, 7) }
        comptime fn packed() -> bool
        where flag: bool, number: u8, ...values: meta.List<bool>
        {
            flag and number == 7 and values.length() == 1 and values[0]
        }
        comptime fn check() -> bool {
            packed<...(true, 7), true>() and packed<...prefix(), true>()
                and packed<.values = [true], ...prefix()>()
        }
        fn run() -> bool { const result = check(); result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn clause_collectors_reject_mixed_supply_wrong_domains_and_tuple_tails() {
    let source = "
        comptime fn count() -> Integer where ...values: meta.List<bool> {
            values.length()
        }
        comptime fn check() -> bool { count<INPUT>() == 2 }
        fn run() -> bool { const result = check(); result }
    ";
    for input in [
        ".values = [true], false",
        "true, .values = [false]",
        ".values = [true], .values = [false]",
        "...do { let xs: meta.List<bool> = []; xs }, .values = [true]",
        "...()",
        "...(true, false)",
        "1u8",
        ".values = [1u8]",
        "...do { let xs: meta.List<u8> = [1]; xs }",
        ".unknown = [true]",
    ] {
        assert!(
            compile(&source.replace("INPUT", input)).is_err(),
            "accepted {input}"
        );
    }
    assert_eq!(
        run(&source.replace("INPUT", "true, false")),
        Value::Bool(true)
    );
}

#[test]
fn clause_collectors_contextualize_literal_before_its_element_type_supply() {
    assert_eq!(
        run("
        comptime fn count() -> Integer where T, ...values: meta.List<T> { values.length() }
        fn run() -> bool { const result = count<.values = [true, false], bool>() == 2; result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn clause_collectors_recover_after_a_failed_invocation_and_keep_type_elements() {
    let package = compile(
        "
        comptime fn first() -> bool where ...values: meta.List<bool> { values[0] }
        comptime fn bad() -> bool { first<>() }
        comptime fn good() -> bool { first<true>() }
        comptime fn check_types() -> bool where ...types: meta.List<Type> {
            types.length() == 2 and types[0] == bool and types[1] == u8
        }
        comptime fn types() -> bool { check_types<bool, u8>() }
    ",
    )
    .unwrap();
    let id = |name| {
        package
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
            .declaration
    };
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert!(vm.invoke(id("bad"), vec![]).is_err());
    assert_eq!(vm.invoke(id("good"), vec![]).unwrap(), Value::Bool(true));
    assert_eq!(vm.invoke(id("types"), vec![]).unwrap(), Value::Bool(true));
}

#[test]
fn clause_collectors_preserve_early_exit_before_later_producers() {
    assert_eq!(
        run("
        comptime fn first() -> bool where ...values: meta.List<bool> { values[0] }
        comptime fn choose() -> bool {
            first<do { return true; false }, first<>()>()
        }
        fn run() -> bool { const result = choose(); result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn clause_collectors_also_supply_runtime_callable_identities() {
    assert_eq!(
        run("
        fn accepted() -> bool where ...values: meta.List<bool> { true }
        fn run() -> bool { accepted<true, false>() and accepted<.values = [true]>() }
    "),
        Value::Bool(true)
    );
}

#[test]
fn clause_collectors_combine_many_list_spreads() {
    let inputs = vec!["...[true, false]"; 128].join(", ");
    let source = format!(
        "
        comptime fn check() -> bool where ...values: meta.List<bool> {{
            values.length() == 256 and values[0] and not values[255]
        }}
        fn run() -> bool {{ const result = check<{inputs}>(); result }}
    "
    );
    assert_eq!(run(&source), Value::Bool(true));
}

#[test]
fn required_collector_inputs_drive_runtime_specialization() {
    assert_eq!(
        run("
        fn choose(input: bool) -> bool where ...flags: meta.List<bool> {
            const flip = flags.length() > 0 and flags[0];
            if flip { not input } else { input }
        }
        fn run() -> bool {
            choose<true>(false) and choose<>(true)
                and not choose<.flags = [false]>(false)
                and choose<...[true]>(false)
        }
    "),
        Value::Bool(true)
    );
}

#[test]
fn required_collector_inputs_keep_prefix_defaults_and_value_contexts() {
    assert_eq!(
        run("
        fn choose(input: bool) -> bool
        where T, seed: bool, .flip: bool = seed, ...flags: meta.List<T>
        {
            if flip { not input } else { input }
        }
        fn run() -> bool {
            choose<u8, true, 1, 255>(false)
                and choose<...(bool, false), .flags = [true]>(true)
                and choose<bool, false, .flip = true>(false)
        }
    "),
        Value::Bool(true)
    );
}

#[test]
fn required_collector_inputs_reject_runtime_reads_and_caller_control_exits() {
    for supplied in [
        "value",
        "do { return true; false }",
        "...do { let items: meta.List<bool> = [value]; items }",
    ] {
        let source = format!(
            "
            fn accepted() -> bool where ...values: meta.List<bool> {{ true }}
            fn run(value: bool) -> bool {{ accepted<{supplied}>() }}
        "
        );
        assert!(
            compile(&source).is_err(),
            "accepted runtime input {supplied}"
        );
    }
}

#[test]
fn collector_instance_formation_does_not_admit_runtime_execution_at_comptime() {
    let source = "
        fn accepted() -> bool where ...values: meta.List<bool> { true }
        fn run() -> bool { const result = accepted<true>(); result }
    ";
    assert!(compile(source).is_err());
}

#[test]
fn required_collectors_on_static_methods_use_the_same_clause_packet() {
    assert_eq!(
        run("
        struct Flag {}
        impl Flag {
            fn choose(input: bool) -> bool
            where .flip: bool = true, ...values: meta.List<u8>
            {
                if flip { not input } else { input }
            }
        }
        fn run() -> bool {
            Flag.choose<1, 255>(false) and Flag.choose<.values = [], .flip = false>(true)
        }
    "),
        Value::Bool(true)
    );
}

#[test]
fn required_collector_packets_can_be_nested_in_constant_callable_formation() {
    assert_eq!(
        run("
        fn choose(input: bool) -> bool where flip: bool, ...flags: meta.List<bool> {
            if flip { not input } else { input }
        }
        fn run() -> bool {
            const selected = choose<true, false>;
            const unchanged = choose<false, .flags = []>;
            selected(false) and unchanged(true)
        }
    "),
        Value::Bool(true)
    );
}

#[test]
fn required_collector_packets_still_reject_recursive_sources() {
    let error = compile(
        "
        const flags: meta.List<bool> = flags;
        fn accepted() -> bool where ...values: meta.List<bool> { true }
        fn run() -> bool { accepted<.values = flags>() }
    ",
    )
    .unwrap_err();
    assert!(
        error.contains("cyclic") || error.contains("recursive"),
        "{error}"
    );
}

#[test]
fn pending_clause_checks_keep_their_source_evaluation_positions() {
    assert_eq!(
        run("
        comptime fn verify() -> bool
        where T, .tag: bool = true, ...values: meta.List<T>
        {
            tag and values.length() == 1
        }
        comptime fn check() -> bool {
            var state = false;
            let selected = verify<
                .values = [do { state = not state; state }],
                .tag = do { state = not state; not state },
                bool
            >();
            selected and not state
        }
        fn run() -> bool { const result = check(); result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn pending_clause_checks_cover_nested_contexts_empty_lists_and_fixed_inputs() {
    assert_eq!(
        run("
        comptime fn accepted() -> bool where .T: Type = bool, .value: meta.List<T> = [] { value.length() == 1 }
        comptime fn count() -> Integer where T, ...values: meta.List<T> { values.length() }
        comptime fn check() -> bool {
            count<.values = [], u8>() == 0
                and count<.values = [[true], []], meta.List<bool>>() == 2
                and count<.values = [1, 255], u8>() == 2
                and accepted<.value = [255], .T = u8>()
        }
        fn run() -> bool { const result = check(); result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn pending_clause_checks_do_not_erase_early_return_or_accept_wrong_elements() {
    assert_eq!(
        run("
        comptime fn count() -> Integer where T, ...values: meta.List<T> { values.length() }
        comptime fn choose() -> bool {
            let result = count<.values = do { return true; [] }, bool>();
            false
        }
        fn run() -> bool { const result = choose(); result }
    "),
        Value::Bool(true)
    );
    let wrong = "
        comptime fn count() -> Integer where T, ...values: meta.List<T> { values.length() }
        fn run() -> bool { const result = count<.values = [true], u8>() == 1; result }
    ";
    let error = compile(wrong).unwrap_err();
    assert!(error.contains("type mismatch"), "{error}");
}

#[test]
fn required_runtime_collector_checks_use_later_type_evidence() {
    assert_eq!(
        run("
        fn selected(input: bool) -> bool where T, ...values: meta.List<T> {
            const count = values.length();
            const flip = count == 2;
            if flip { not input } else { input }
        }
        fn run() -> bool {
            selected<.values = [1, 255], u8>(false)
                and selected<.values = [], bool>(true)
        }
    "),
        Value::Bool(true)
    );
}

#[test]
fn pending_clause_checks_contextualize_control_flow_without_a_second_expression_tree() {
    assert_eq!(
        run("
        comptime fn count() -> Integer where T, ...values: meta.List<T> { values.length() }
        comptime fn choose(flag: bool) -> bool {
            let first = count<.values = if flag { [1, 255] } else { [] }, u8>();
            let second = count<.values = flag match { true => [7], false => [] }, u8>();
            first == 2 and second == 1
        }
        fn run() -> bool { const result = choose(true) and not choose(false); result }
    "),
        Value::Bool(true)
    );
}

#[test]
#[ignore = "ordinary clause domains still need invocation-time Type observations"]
fn pending_clause_domains_can_wait_for_a_type_producer() {
    assert_eq!(
        run("
        comptime fn Byte() -> Type { u8 }
        comptime fn count() -> Integer where T, ...values: meta.List<T> { values.length() }
        fn run() -> bool { const result = count<.values = [1, 255], Byte()>() == 2; result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn pending_clause_checks_keep_mutable_callable_reads_at_the_source_position() {
    assert_eq!(
        run("
        comptime fn yes() -> bool { true }
        comptime fn no() -> bool { false }
        comptime fn first() -> T
        where T, .tag: bool = true, ...values: meta.List<T>
        {
            values[0]
        }
        comptime fn check() -> bool {
            var action: comptime fn() -> bool = yes;
            let selected = first<
                .values = [action()],
                .tag = do { action = no; true },
                bool
            >();
            selected and not action()
        }
        fn run() -> bool { const result = check(); result }
    "),
        Value::Bool(true)
    );
}

#[test]
fn pending_clause_type_observations_keep_opaque_head_identity() {
    assert_eq!(
        run("
        pure comptime fn Identity<T> -> Type { T }
        comptime fn empty() -> bool where T, ...values: meta.List<T> {
            values.length() == 0 and T == Identity<u8>
        }
        fn run() -> bool {
            const result = empty<.values = [], Identity<u8>>();
            result
        }
    "),
        Value::Bool(true)
    );
}
