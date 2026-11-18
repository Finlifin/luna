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

#[test]
fn value_clause_factory_inputs_wait_for_specialization() {
    assert_eq!(
        sysroot::run(include_str!(
            "../../../development/repros/staged-callable-value-input.fl"
        )),
        Value::Bool(true)
    );
}

#[test]
fn value_clause_identities_preserve_phase_and_chained_defaults() {
    for phase in ["", "comptime "] {
        let source = r#"
            PHASEfn selected() -> bool where flag: bool { flag }
            fn apply(comptime action: PHASEfn() -> bool,
                     comptime .alias: PHASEfn() -> bool = action) -> bool { alias() }
            fn forward() -> bool where flag: bool { apply(selected<flag>) }
            fn run() -> bool { forward<true>() and not forward<false>() }
        "#
        .replace("PHASE", phase);
        assert_eq!(sysroot::run(&source), Value::Bool(true), "phase: {phase}");
    }
}

#[test]
fn callback_factories_observe_outer_value_inputs() {
    let source = r#"
        struct Settings where flag: bool {}
        comptime fn selected() -> bool where flag: bool { flag }
        comptime fn make() -> comptime fn() -> bool where flag: bool {
            if flag { selected<true> } else { selected<false> }
        }
        fn apply(comptime action: comptime fn() -> bool) -> bool { action() }
        impl Settings<flag> where flag: bool {
            fn forward() -> bool { apply(make<flag>()) }
        }
        fn run() -> bool { Settings<true>.forward() and not Settings<false>.forward() }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn pending_value_inputs_do_not_hide_producer_contract_failures() {
    let source = r#"
        comptime fn selected() -> bool where flag: bool { flag }
        comptime fn make() -> comptime fn() -> bool where flag: bool {
            assumes flag;
            selected<flag>
        }
        fn discard(comptime action: comptime fn() -> bool) -> bool { true }
        fn forward() -> bool where flag: bool { discard(make<flag>()) }
        fn run() -> bool { forward<false>() }
    "#;
    let package = sysroot::compile(source).unwrap();
    let failure = luna::instantiation::instantiate(&package, "run", 1_000_000)
        .err()
        .expect("the factory must run even when its callback is unused");
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    assert_eq!(
        sysroot::run(&source.replace("forward<false>()", "forward<true>()")),
        Value::Bool(true)
    );
}

#[test]
fn deferred_identity_inputs_cannot_read_runtime_parameters() {
    let source = r#"
        comptime fn selected() -> bool where flag: bool { flag }
        fn apply(comptime action: comptime fn() -> bool) -> bool { action() }
        fn invalid(flag: bool) -> bool { apply(selected<flag>) }
    "#;
    let failure = sysroot::compile(source)
        .err()
        .expect("runtime input is unavailable");
    assert!(
        failure.contains("runtime") || failure.contains("comptime"),
        "{failure}"
    );
}

#[test]
fn staged_nominal_domains_retain_value_inputs_without_payload_reads() {
    let source = r#"
        struct Marker where flag: bool {}
        fn consume(comptime marker: Marker<flag>) -> bool where flag: bool { flag }
        fn forward() -> bool where flag: bool { consume<flag>(Marker<flag> {}) }
        fn run() -> bool { forward<true>() and not forward<false>() }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn dependent_callable_domains_survive_comptime_container_transport() {
    let source = r#"
        struct Packet where T { payload: T }
        typealias Reader = comptime fn(comptime T: Type, Packet<T>) -> T;
        comptime fn read(comptime T: Type, packet: Packet<T>) -> T { packet.payload }
        comptime fn evaluate() -> bool {
            let readers: meta.List<Reader> = [read];
            let domain: meta.List<Type> = [Reader];
            domain[0] == Reader and readers[0](bool, Packet<bool> { payload: true })
                and readers[0](u8, Packet<u8> { payload: 7 }) == 7
        }
        fn run() -> bool { evaluate() }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}
