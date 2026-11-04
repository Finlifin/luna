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
                comptime_interpretor::Interpreter::for_comptime_tree(&package, Default::default())
                    .unwrap()
            } else {
                comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
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
