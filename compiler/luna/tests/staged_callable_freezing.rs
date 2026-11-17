//! Named callable values retain their phase and instance inputs across freezing.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::Value;

#[test]
fn frozen_factory_results_preserve_both_callable_phases_and_instance_arguments() {
    for phase in ["", "comptime "] {
        let source = r#"
            PHASEfn selected(value: bool) -> bool where T { not value }
            comptime fn make() -> PHASEfn(bool) -> bool where T { selected<T> }
            fn apply(comptime action: PHASEfn(bool) -> bool) -> bool { RESULT }
            fn forward() -> bool where T { apply(make<T>()) }
            fn run() -> bool { not forward<bool>() and not forward<?bool>() }
        "#
        .replace("PHASE", phase)
        .replace(
            "RESULT",
            if phase.is_empty() {
                "action(true)"
            } else {
                "const result = action(true); result"
            },
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(true),
            "callable phase: {phase}"
        );
    }
}

#[test]
fn frozen_comptime_callbacks_execute_after_nominal_field_delivery() {
    let source = r#"
        struct Config { action: comptime fn(bool) -> bool }
        comptime fn selected(value: bool) -> bool where T { not value }
        comptime fn make() -> Config where T { Config { action: selected<T> } }
        fn apply(comptime config: Config) -> bool { const answer = config.action(true); answer }
        fn forward() -> bool where T { apply(make<T>()) }
        fn run() -> bool { not forward<bool>() and not forward<?bool>() }
    "#;
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn freezing_an_unused_callback_does_not_execute_its_body() {
    let source = r#"
        comptime fn selected() -> bool where T { assumes false; true }
        comptime fn make() -> comptime fn() -> bool where T { selected<T> }
        fn discard(comptime action: comptime fn() -> bool) -> bool { true }
        fn forward() -> bool where T { discard(make<T>()) }
        fn run() -> bool { forward<bool>() }
    "#;
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn freezing_does_not_convert_a_runtime_callback_to_comptime() {
    let source = r#"
        fn selected(value: bool) -> bool where T { not value }
        comptime fn make() -> fn(bool) -> bool where T { selected<T> }
        fn apply(comptime action: comptime fn(bool) -> bool) -> bool { action(true) }
        fn forward() -> bool where T { apply(make<T>()) }
        fn run() -> bool { forward<bool>() }
    "#;
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn sysroot_splice_delivers_an_implicit_comptime_callback_result() {
    let source = r#"
        comptime fn selected(value: bool) -> bool where T { not value }
        comptime fn make() -> comptime fn(bool) -> bool where T { selected<T> }
        fn apply(comptime action: comptime fn(bool) -> bool) -> bool { action(true) }
        fn forward() -> bool where T { apply(make<T>()) }
        fn run() -> bool { forward<bool>() }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(false));
}
