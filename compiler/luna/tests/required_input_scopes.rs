//! Nested type formation observes supplied clause inputs in their owning region.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Interpreter, Value};

#[test]
fn quoting_a_constant_does_not_supply_a_splice_position_to_its_initializer() {
    for (declaration, reference) in [
        ("const VALUE: bool = ~<'(true)>;", "VALUE"),
        (
            "struct Holder where T { pub const VALUE: bool = ~<'(true)>; }",
            "Holder<bool>.VALUE",
        ),
    ] {
        let witness = format!("const CHECK: bool = do {{ const code = '({reference}); true }};");
        for source in [
            format!("{declaration} {witness}"),
            format!("{witness} {declaration}"),
        ] {
            let error = sysroot::compile(&source).unwrap_err();
            assert!(
                error.contains("code splice requires a runtime position"),
                "{error}"
            );
        }
    }
}

#[test]
fn a_constant_and_its_requester_each_own_their_quote_hole() {
    let source = r#"
        const CHECK: bool = do { const code = '(VALUE and ~<'(true)>); true };
        const VALUE: bool = do { const code = '(~<'(true)>); true };
        fn run() -> bool { CHECK and VALUE }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

fn check_required_and_published(source: &str) -> nir::program::Package {
    let package = sysroot::compile(source).unwrap();
    let entry = package
        .functions
        .iter()
        .find(|function| function.name == "check")
        .unwrap()
        .declaration;
    for tree in [false, true] {
        sysroot::with_comptime(&package, Budget::default(), tree, |vm| {
            assert_eq!(vm.invoke(entry, vec![]).unwrap(), Value::Bool(true));
        });
    }
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    package
}

const SOURCE: &str = "
    struct Flag where enabled: bool {}
    comptime fn selected() -> <|-bool>
    where flag: bool, .witness: Flag<flag> = (Flag<flag>{})
    {
        if flag {'(true)} else {'(false)}
    }
    fn run() -> bool {
        const yes = selected<true>();
        const no = selected<false>();
        const again = selected<true>();
        ~<yes> and not ~<no> and ~<again>
    }
";

#[test]
fn nested_required_inputs_remain_separate_across_instances() {
    assert_eq!(sysroot::run(SOURCE), Value::Bool(true));
}

#[test]
fn nested_required_calls_capture_supplied_inputs_for_helpers() {
    let source = SOURCE
        .replace(
            "struct Flag",
            "pure comptime fn same(value: bool) -> bool { value }\nstruct Flag",
        )
        .replace("= (Flag<flag>{})", "= (Flag<same(flag)>{})");
    assert_eq!(sysroot::run(&source), Value::Bool(true));
}

#[test]
fn nested_required_domains_reject_wrong_and_unavailable_inputs() {
    for source in [
        SOURCE.replace("= (Flag<flag>{})", "= (Flag<not flag>{})"),
        SOURCE.replace(
            "flag: bool, .witness: Flag<flag> = (Flag<flag>{})",
            ".witness: Flag<flag> = (Flag<flag>{}), flag: bool",
        ),
        SOURCE
            .replace("fn run() -> bool", "fn run(value: bool) -> bool")
            .replace("selected<true>()", "selected<value>()"),
    ] {
        assert!(sysroot::compile(&source).is_err());
    }
    assert_eq!(sysroot::run(SOURCE), Value::Bool(true));
}

#[test]
fn ordinary_functions_use_the_same_input_scope() {
    let ordinary = SOURCE
        .replace("-> <|-bool>", "-> bool")
        .replace("if flag {'(true)} else {'(false)}", "flag")
        .replace(
            "~<yes> and not ~<no> and ~<again>",
            "yes and not no and again",
        );
    assert_eq!(sysroot::run(&ordinary), Value::Bool(true));
}

#[test]
fn library_collectors_use_the_same_input_scope() {
    let collected = SOURCE.replace(
        ".witness: Flag<flag> = (Flag<flag>{})",
        ".witness: meta.List<Flag<flag>> = [Flag<flag>{}]",
    );
    assert_eq!(sysroot::run(&collected), Value::Bool(true));
}

#[test]
fn forwarded_generic_clauses_keep_their_symbolic_identity() {
    let source = SOURCE
        .replace(
            "fn run()",
            "comptime fn forward() -> <|-bool> where value: bool { selected<value>() }\nfn run()",
        )
        .replace("selected<true>()", "forward<true>()")
        .replace("selected<false>()", "forward<false>()");
    assert_eq!(sysroot::run(&source), Value::Bool(true));
}

#[test]
fn observed_clause_identities_do_not_erase_input_effects() {
    assert_eq!(
        sysroot::run(
            "
        struct Flag where enabled: bool {}
        comptime fn select() -> bool
        where flag: bool, .witness: Flag<flag> = (Flag<flag>{})
        {
            flag
        }
        comptime fn check() -> bool {
            var ticks: Integer = 0;
            let value = select<do { ticks = ticks + 1; true }>();
            value and ticks == 1
        }
        fn run() -> bool { const value = check(); value }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn required_and_published_bodies_agree_on_type_inputs_and_defaults() {
    check_required_and_published(
        "
        comptime fn Pass(value: Type) -> Type { value }
        comptime fn digits(a: Integer, .b: Integer = a + 1) -> Integer {
            a * 10 + b
        }
        comptime fn check() -> bool {
            let types: meta.List<Type> = [Pass(u8), Pass(bool), Pass(u8)];
            var trace: Integer = 0;
            let supplied = digits(
                do { trace = trace * 10 + 1; 2 },
                .b = do { trace = trace * 10 + 2; 9 }
            );
            types.length() == 3 and types[0] == u8 and types[1] == bool
                and types[2] == u8 and digits(3) == 34
                and supplied == 29 and trace == 12
        }
        fn run() -> bool { const result = check(); result }
    ",
    );
}

#[test]
fn required_and_published_bodies_keep_entry_and_result_contracts() {
    let source = "
        comptime fn keep(value: u8) -> result: u8
        where requires value <= 10, ensures result == value'old
        {
            let original = value;
            value = 0;
            original
        }
        comptime fn check() -> bool { keep(9) == 9 and keep(7) == 7 }
        fn run() -> bool { const result = check(); result }
    ";
    let package = check_required_and_published(source);
    let function = package
        .functions
        .iter()
        .find(|function| function.name == "keep")
        .unwrap();
    assert!(!function.preconditions.is_empty());
    assert!(!function.postconditions.is_empty());
    assert!(!function.entry_snapshots.is_empty());
    for tree in [false, true] {
        sysroot::with_comptime(&package, Budget::default(), tree, |vm| {
            assert!(
                vm.invoke(function.declaration, vec![Value::UnsignedInteger(11)])
                    .is_err()
            );
            assert_eq!(
                vm.invoke(function.declaration, vec![Value::UnsignedInteger(9)])
                    .unwrap(),
                Value::UnsignedInteger(9)
            );
        });
    }
    assert!(sysroot::compile(&source.replace("keep(9)", "keep(11)")).is_err());
    assert!(sysroot::compile(&source.replace("original\n", "0\n")).is_err());
}

#[test]
fn wide_required_type_computation_retains_every_referenced_definition() {
    use std::fmt::Write;
    let mut source = String::new();
    for index in 0..260 {
        writeln!(source, "pure comptime fn flag{index}() -> bool {{ true }}").unwrap();
    }
    source.push_str("pure comptime fn Output() -> Type {\n");
    for index in 0..260 {
        writeln!(source, "if not flag{index}() {{ return u8; }}").unwrap();
    }
    source.push_str("bool } fn run() -> Output() { true }");
    assert_eq!(sysroot::run(&source), Value::Bool(true));
}

#[test]
fn deferred_type_calls_keep_callable_inputs_of_named_instances() {
    assert_eq!(
        sysroot::run(
            r#"
        comptime fn Flag() -> Type { bool }
        comptime fn Byte() -> Type { u8 }
        comptime fn Forward() -> Type where choose: comptime fn() -> Type { choose() }
        fn accept(comptime choose: comptime fn() -> Type, value: defer choose()) -> defer choose() { value }
        fn run() -> bool {
            let byte: u8 = 7;
            accept(Forward<Flag>, true) and accept(Forward<Byte>, byte) == 7
        }
    "#
        ),
        Value::Bool(true)
    );
}

#[test]
fn required_type_observation_checks_a_runtime_default_in_its_declaration() {
    let source = r#"
        typealias Answer = AnswerType();
        comptime fn AnswerType() -> Type { choose(false)'type }

        fn choose(value: bool, .apply: fn(bool) -> bool = |x: bool| not x) -> bool {
            apply(value)
        }
        fn run() -> Answer { choose(false) and not choose(true) }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn type_observation_preserves_runtime_callbacks_in_aggregate_defaults() {
    let source = r#"
        typealias Selected = SelectedType();
        typealias Case = CaseType();
        comptime fn SelectedType() -> Type { (Actions {})'type }
        comptime fn CaseType() -> Type { (Mode.direct {})'type }

        struct Actions { apply: fn(bool) -> bool = |x: bool| not x }
        enum Mode {
            direct { apply: fn(bool) -> bool = |x: bool| x },
            invert { apply: fn(bool) -> bool = |x: bool| not x },
        }
        fn invoke(mode: Case) -> bool {
            mode match {
                .direct { apply } => apply(true),
                .invert { apply } => apply(true),
            }
        }
        fn run() -> bool {
            let actions: Selected = Actions {};
            (actions.apply)(false) and invoke(Mode.direct {}) and not invoke(Mode.invert {})
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn erasing_an_alias_argument_cannot_expose_later_default_inputs() {
    let source = r#"
        typealias Ignore<T> = bool;
        typealias Example<.A: Type = bool, .B: Type = Ignore<A>> = B;
        fn run() -> Example<> { true }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let uncalled = source.replace("fn run() -> Example<> { true }", "fn run() {}");
    for unavailable in ["A", "B"] {
        let invalid = uncalled.replace(
            ".A: Type = bool",
            &format!(".A: Type = Ignore<{unavailable}>"),
        );
        let error = sysroot::compile(&invalid).unwrap_err();
        assert!(error.contains("itself or a later parameter"), "{error}");
    }
}

#[test]
fn required_method_calls_demand_only_receiver_compatible_bodies() {
    let source = r#"
        trait Source { comptime fn flag() -> bool; }
        struct First {}
        struct Second {}
        impl Source for First { comptime fn flag() -> bool { true } }
        impl Source for Second { comptime fn flag() -> bool { FLAG } }
        const FLAG: bool = First.as(Source).flag();
        fn run() -> bool { Second.as(Source).flag() }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let cycle = source.replace("= First.as", "= Second.as");
    let error = sysroot::compile(&cycle).unwrap_err();
    assert!(error.contains("cyclic"), "{error}");
    // Finishing the source package still checks the unused implementation.
    let invalid = source.replace("{ FLAG }", "{ First {} }").replace(
        "fn run() -> bool { Second.as(Source).flag() }",
        "fn run() -> bool { FLAG }",
    );
    let error = sysroot::compile(&invalid).unwrap_err();
    assert!(
        error.contains("expected") && error.contains("bool"),
        "{error}"
    );
}

#[test]
fn required_method_values_keep_interface_arguments_and_inherited_defaults() {
    let source = r#"
        trait Source where T { comptime fn flag() -> bool { FLAG } }
        struct Host {}
        impl Source<bool> for Host { comptime fn flag() -> bool { true } }
        impl Source<u8> for Host {}
        const FLAG: bool = do { let get = Host.as(Source<bool>).flag; get() };
        fn run() -> bool { Host.as(Source<u8>).flag() }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}
