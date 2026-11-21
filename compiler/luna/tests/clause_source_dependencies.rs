//! Clause defaults wait for free declaration inputs, including captured inputs.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

const DEFAULTS: &[&str] = &[
    "do { let capture = || T == bool; capture() }",
    "(|candidate: Type| candidate == T)(bool)",
    "do { pure comptime fn capture() -> bool { T == bool } capture() }",
    "do { pure comptime fn compare(candidate: Type) -> bool where U { candidate == U and U == T } compare<bool>(bool) }",
];

#[test]
fn ordinary_value_defaults_wait_for_captured_type_inference() {
    for default in DEFAULTS {
        let source = format!(
            "comptime fn choose(value: T) -> bool
            where T, .flag: bool = {default}, .opposite: bool = not flag
            {{ opposite }}
            const yes = choose(true);
            const no = choose('x');
            const override = choose<bool, .flag = false>(true);
            comptime fn run() -> bool {{ not yes and no and override }}"
        );
        let package =
            sysroot::compile(&source).unwrap_or_else(|error| panic!("{default}: {error}"));
        let run = package
            .functions
            .iter()
            .find(|function| function.name == "run")
            .unwrap()
            .declaration;
        for tree in [false, true] {
            let mut vm = if tree {
                comptime_interpretor::Interpreter::for_comptime_tree(
                    &package,
                    sysroot::COMPTIME_BUDGET,
                )
                .unwrap()
            } else {
                comptime_interpretor::Interpreter::for_comptime(&package, sysroot::COMPTIME_BUDGET)
                    .unwrap()
            };
            assert_eq!(
                vm.invoke(run, vec![]).unwrap(),
                Value::Bool(true),
                "{default}"
            );
        }
    }
}

#[test]
fn code_generator_defaults_wait_for_captured_type_inference() {
    for default in DEFAULTS {
        let source = format!(
            "comptime fn choose(value: T) -> < |- bool>
            where T, .flag: bool = {default}, .opposite: bool = not flag
            {{ '(opposite) }}
            fn run() -> bool {{
                const yes = choose(true);
                const no = choose('x');
                const override = choose<bool, .flag = false>(true);
                not ~<yes> and ~<no> and ~<override>
            }}"
        );
        assert_eq!(sysroot::run(&source), Value::Bool(true), "{default}");
    }
}

#[test]
fn explicit_overrides_do_not_hide_invalid_closure_defaults() {
    for generator in [false, true] {
        let (output, body, call) = if generator {
            (
                "< |- bool>",
                "'(flag)",
                "fn run() -> bool { const code = choose<bool, .flag = true>(true); ~<code> }",
            )
        } else {
            (
                "bool",
                "flag",
                "const value = choose<bool, .flag = true>(true);",
            )
        };
        let source = format!(
            "comptime fn choose(value: T) -> {output}
            where T, .flag: bool = do {{ let capture = || 'x'; capture() }}
            {{ {body} }}
            {call}"
        );
        let failure = sysroot::compile(&source)
            .err()
            .expect("invalid default accepted");
        assert!(failure.contains("type"), "{failure}");
    }
}

#[test]
fn named_clause_overrides_keep_caller_identity_and_infer_missing_slots() {
    let source = r#"
        fn inspect(x: T, witness: *Flag, done: bool) -> bool
        where T, .Flag: Type = bool
        {
            if done { true }
            else {
                let byte: u8 = 7;
                inspect<.Flag = T>(byte, x.ref, true)
            }
        }

        fn validate(value: T) -> bool
        where T, .Marker: Type = u32, .chosen: bool = do { let capture = || T == bool; capture() }
        {
            chosen
        }

        fn run() -> bool {
            let flag = true;
            inspect(flag, flag.ref, false)
                and validate<.Marker = u16>(flag)
                and not validate<.Marker = u16>('x')
                and validate<.chosen = true>('x')
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let invalid = source.replace("byte, x.ref, true", "byte, byte.ref, true");
    let error = sysroot::compile(&invalid).unwrap_err();
    assert!(
        error.contains("type constraint") || error.contains("type mismatch"),
        "{error}"
    );
}

#[test]
fn open_calls_infer_missing_clauses_alongside_explicit_overrides() {
    let source = r#"
        fn choose(value: T, comptime keep: bool) -> T
        where T, .Witness: Type = bool
        {
            inline if keep { value } else { value }
        }

        fn wrapper(value: T, comptime keep: bool) -> T where T {
            choose<.Witness = u16>(value, keep)
        }

        fn run() -> bool {
            let byte: u8 = 42;
            wrapper(byte, true) == byte and choose<.Witness = char>(byte, false) == byte
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn named_clause_literals_wait_for_their_inferred_domain() {
    let source = r#"
        fn present(value: T) -> bool
        where T, .tag: ?T = null, .flag: bool = tag matches _?
        {
            flag
        }

        fn tagged(value: T, comptime keep: bool) -> bool
        where T, .tag: ?T = null
        {
            inline if keep { tag matches _? } else { false }
        }

        fn run() -> bool {
            let byte: u8 = 1;
            not present<.tag = null>(byte)
                and present<.tag = 255>(byte)
                and not present<.tag = if true { null } else { 1 }>(byte)
                and tagged<.tag = 7>(byte, true)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let invalid = source.replace("else { 1 }", "else { 256 }");
    let error = sysroot::compile(&invalid).unwrap_err();
    assert!(error.contains("does not fit"), "{error}");
}

#[test]
fn computed_clause_operands_keep_their_value_until_delivery() {
    let source = r#"
        enum TestErr { Missing }
        comptime fn make() -> u8 { 42 }
        comptime fn absent() -> ?u8 { null }
        comptime fn fail() -> !TestErr u8 { error TestErr.Missing }

        fn present(value: T) -> bool where T, .tag: ?T = null { tag matches _? }
        fn failed(value: T) -> bool
        where T, .out: !TestErr T = error TestErr.Missing
        {
            out matches error _
        }

        fn forward(value: U) -> bool
        where U, .outer: !TestErr U = error TestErr.Missing
        {
            failed<.out = outer>(value)
        }

        fn run() -> bool {
            let byte: u8 = 1;
            present<.tag = make()>(byte)
                and not present<.tag = absent()>(byte)
                and not failed<.out = make()>(byte)
                and failed<.out = fail()>(byte)
                and forward(byte)
                and not forward<.outer = make()>(byte)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let error = sysroot::compile(&source.replace("make() -> u8", "make() -> u16")).unwrap_err();
    assert!(
        error.contains("expected (primitive u8), found (primitive u16)"),
        "{error}"
    );
}

#[test]
fn contextual_clause_control_runs_in_order_before_ordinary_type_inference() {
    let source = r#"
        comptime fn choose(name: Symbol, result: bool) -> bool {
            let generated = meta.StructBuilder.new(.name = name).Build();
            result
        }

        fn present(value: T) -> bool
        where T, .tag: ?T = null, .stamp: bool = false
        {
            stamp and (tag matches _?)
        }

        fn run() -> bool {
            let byte: u8 = 0;
            present<.tag = do {
                let flag = choose(.FirstSelector, false);
                flag match {
                    true => null,
                    false => when {
                        choose(.SecondSelector, true) => 255,
                        choose(.SkippedSelector, false) => 1,
                        else => null,
                    },
                }
            }, .stamp = choose(.FollowingInput, true)>(byte)
        }
    "#;
    let package = sysroot::compile(source).unwrap();
    let mut events: Vec<_> = package
        .structs
        .iter()
        .filter_map(|definition| {
            let name = definition.name.as_str();
            matches!(
                name,
                "FirstSelector" | "SecondSelector" | "SkippedSelector" | "FollowingInput"
            )
            .then_some((definition.declaration, name))
        })
        .collect();
    events.sort_by_key(|(id, _)| *id);
    assert_eq!(
        events.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ["FirstSelector", "SecondSelector", "FollowingInput"]
    );
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let wrong = source.replace("true => null", "true => 256");
    assert!(
        sysroot::compile(&wrong)
            .unwrap_err()
            .contains("does not fit")
    );
}

#[test]
fn runtime_wrappers_specialize_clause_operands_from_staged_parameters() {
    let source = r#"
        fn outer(value: u8, comptime flag: bool) -> bool { nested(value, flag) }
        fn nested(value: u8, comptime flag: bool) -> bool {
            let alias = flag;
            present<.tag = if choose(alias) { null } else { 42 }>(value)
        }
        fn explicit(value: u8, comptime flag: bool) -> bool {
            present<u8, .tag = if choose(flag) { null } else { 42 }>(value)
        }
        comptime fn choose(value: bool) -> bool { value }
        fn present(value: T) -> bool where T, .tag: ?T = null { tag matches _? }
        fn run() -> bool {
            let byte: u8 = 0;
            not outer(byte, true) and outer(byte, false)
                and not explicit(byte, true) and explicit(byte, false)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    for (from, to, diagnostic) in [
        ("choose(alias)", "choose(value == 0)", "runtime"),
        ("else { 42 }", "else { 256 }", "does not fit"),
    ] {
        let error = sysroot::compile(&source.replace(from, to)).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn a_returned_specialization_retains_its_own_clause_inputs() {
    let source = r#"
        fn present(value: T) -> bool where T, .tag: ?T = null { tag matches _? }
        comptime fn choose(flag: bool) -> bool { flag }
        fn select(comptime flag: bool) -> fn(value: u8) -> bool {
            present<u8, .tag = if choose(flag) { null } else { 42 }>
        }
        fn run() -> bool {
            let byte: u8 = 0;
            let empty = select(true);
            let full = select(false);
            not empty(byte) and full(byte) and not empty(byte)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn mixed_clause_results_keep_lexical_values_and_check_unselected_producers() {
    let source = r#"
        comptime fn event(name: Symbol, value: u8) -> u8 {
            let generated = meta.StructBuilder.new(.name = name).Build();
            value
        }
        fn check(value: T) -> ?T where T, .tag: ?T = null, .stamp: u8 = 0 {
            if stamp == 9 { tag } else { null }
        }
        fn run() -> bool {
            let byte: u8 = 0;
            let result = check<.tag = do {
                let value = event(.BeforeMatch, 42);
                value match {
                    0 => null,
                    1 => event(.Skipped, value),
                    payload => event(.Selected, payload),
                }
            }, .stamp = event(.AfterMatch, 9)>(byte);
            result matches 42?
        }
    "#;
    let package = sysroot::compile(source).unwrap();
    let mut events: Vec<_> = package
        .structs
        .iter()
        .filter_map(|def| {
            let name = def.name.as_str();
            matches!(name, "BeforeMatch" | "Skipped" | "Selected" | "AfterMatch")
                .then_some((def.declaration, name))
        })
        .collect();
    events.sort_unstable();
    assert_eq!(
        events.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ["BeforeMatch", "Selected", "AfterMatch"]
    );
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let bad = source.replace("event(.Skipped, value)", "event(.Skipped, value) == 1");
    let error = sysroot::compile(&bad).unwrap_err();
    assert!(error.contains("bool") && error.contains("u8"), "{error}");
}

#[test]
fn mixed_clause_results_distinguish_absence_from_an_absent_payload() {
    let source = r#"
        comptime fn choose(value: bool) -> bool { value }
        comptime fn absent() -> ?u8 { null }
        fn read(value: T) -> ?T where T, .tag: ?T = null { tag }
        fn staged(value: ?u8, comptime flag: bool) -> ??u8 {
            read<.tag = if choose(flag) { null } else { absent() }>(value)
        }
        fn run() -> bool {
            let witness: ?u8 = null;
            let outer = staged(witness, true);
            let inner = staged(witness, false);
            (outer matches null) and (inner matches null?)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn computed_clause_delivery_preserves_error_tags_and_optional_success() {
    let source = r#"
        enum ReadErr { Missing }
        comptime fn failed() -> !ReadErr ?u8 { error .Missing }
        comptime fn byte() -> u8 { 42 }
        fn read(value: T) -> !ReadErr ?T where T, .tag: !ReadErr ?T = null { tag }
        fn staged(value: u8, comptime branch: u8) -> !ReadErr ?u8 {
            read<.tag = branch match {
                0 => null,
                1 => failed(),
                _ => byte(),
            }>(value)
        }
        fn run() -> bool {
            let value: u8 = 0;
            (staged(value, 0) matches null!) and (staged(value, 1) matches error .Missing)
                and (staged(value, 2) matches 42?!)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}
