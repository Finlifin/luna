//! Prepared annotations retain their inputs until specialization supplies them.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

const DECLARATIONS: &str = "
comptime fn Select(flag: bool) -> Type {
    let select = || if flag { bool } else { u8 };
    select()
}
fn accept(value: defer Select(flag)) -> defer Select(flag)
where flag: bool
{
    value
}
fn forward(value: defer Select(flag)) -> defer Select(flag)
where flag: bool
{
    accept<flag>(value)
}
";

#[test]
fn prepared_closure_annotations_wait_through_generic_forwarding() {
    let source = format!(
        "{DECLARATIONS}
        fn run() -> bool {{ forward<true>(true) and forward<false>(9) == 9 }}"
    );
    assert_eq!(sysroot::run(&source), Value::Bool(true));
}

#[test]
fn prepared_annotation_observation_does_not_admit_the_wrong_completed_domain() {
    let source = format!(
        "{DECLARATIONS}
        fn run() -> bool {{ forward<false>(true); true }}"
    );
    let failure = sysroot::compile(&source)
        .err()
        .expect("wrong domain accepted");
    assert!(failure.contains("type"), "{failure}");
}
