//! Signature premises use free source inputs rather than syntax whitelists.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

fn source(subject: &str, bound: &str) -> String {
    format!(
        "trait Source where U {{ assoc Item: Type; }}
        impl Source<bool> for bool {{ assoc Item: Type = bool; }}
        fn take(comptime T: Type, value: T.as(Source<bool>).Item) -> T.as(Source<bool>).Item
        where requires {subject} :- {bound}
        {{ value }}
        fn run() -> bool {{ take(bool, true) }}"
    )
}

#[test]
fn signature_capabilities_accept_internal_bindings_and_captured_inputs() {
    for (subject, bound) in [
        ("do { let subject = T; subject }", "Source<bool>"),
        ("do { let select = || T; select() }", "Source<bool>"),
        ("T", "Source<do { let selected = bool; selected }>"),
        ("T", "Source<do { let select = || bool; select() }>"),
    ] {
        assert_eq!(
            sysroot::run(&source(subject, bound)),
            Value::Bool(true),
            "{subject} :- {bound}"
        );
    }
}

#[test]
fn source_readiness_does_not_supply_missing_capability_evidence() {
    let source = source("do { let subject = T; subject }", "Source<bool>")
        .replace("take(bool, true)", "take(u8, 9); true");
    let failure = sysroot::compile(&source)
        .err()
        .expect("missing capability accepted");
    assert!(
        failure.contains("requirement") || failure.contains("trait"),
        "{failure}"
    );
}

#[test]
fn a_capability_operand_cannot_use_the_capability_it_is_forming() {
    let source = source(
        "T",
        "Source<do { let selected = T.as(Source<bool>).Item; selected }>",
    );
    let failure = sysroot::compile(&source)
        .err()
        .expect("circular capability evidence accepted");
    assert!(
        failure.contains("requires established trait evidence"),
        "{failure}"
    );
}
