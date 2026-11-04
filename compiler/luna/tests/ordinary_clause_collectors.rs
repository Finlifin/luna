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
#[ignore = "ordinary input preparation needs a contextual expression plan for later Type suppliers"]
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
#[ignore = "required clause invocation still uses the separate trait input routing path"]
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
