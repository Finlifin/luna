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
