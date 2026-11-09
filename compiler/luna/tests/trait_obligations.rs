//! A caller premise is one source of evidence, not a gate on other proofs.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

fn forwarding(phase: &str, clause_input: bool) -> String {
    let (parameters, bounds, concrete, forwarded) = if clause_input {
        (
            "value: T",
            "where T :- Inspect",
            "leaf<u8>(7)",
            "leaf<T>(value)",
        )
    } else {
        (
            "comptime T: Type, value: T",
            "where requires T :- Inspect",
            "leaf(u8, 7)",
            "leaf(T, value)",
        )
    };
    let call = if clause_input {
        "query<bool>(true)"
    } else {
        "query(bool, true)"
    };
    let entry = if phase.is_empty() {
        format!("fn run() -> bool {{ {call} }}")
    } else {
        format!("fn run() -> bool {{ const answer = {call}; answer }}")
    };
    format!(
        r#"
trait Inspect {{ {phase} fn inspect(self) -> bool; }}
impl Inspect for bool {{ {phase} fn inspect(self) -> bool {{ self }} }}
impl Inspect for u8 {{ {phase} fn inspect(self) -> bool {{ self == 7 }} }}
{phase} fn leaf({parameters}) -> bool
{bounds}
{{
    value.as(Inspect).inspect()
}}
{phase} fn query({parameters}) -> bool
{bounds}
{{
    {concrete} and {forwarded}
}}
{entry}
"#
    )
}

#[test]
fn ordinary_type_inputs_use_given_and_independent_implementation_evidence() {
    for phase in ["", "comptime"] {
        let source = forwarding(phase, false);
        assert_eq!(sysroot::run(&source), Value::Bool(true));
        let false_value = source.replace("query(bool, true)", "query(bool, false)");
        assert_eq!(sysroot::run(&false_value), Value::Bool(false));
    }
}

#[test]
fn clause_type_inputs_use_given_and_independent_implementation_evidence() {
    for phase in ["", "comptime"] {
        let source = forwarding(phase, true);
        assert_eq!(sysroot::run(&source), Value::Bool(true));
        let false_value = source.replace("query<bool>(true)", "query<bool>(false)");
        assert_eq!(sysroot::run(&false_value), Value::Bool(false));
    }
}

#[test]
fn unrelated_premises_cannot_supply_a_missing_implementation() {
    for phase in ["", "comptime"] {
        for clause_input in [false, true] {
            let source = forwarding(phase, clause_input);
            let missing = source.replace(
                &format!(
                    "impl Inspect for u8 {{ {phase} fn inspect(self) -> bool {{ self == 7 }} }}"
                ),
                "",
            );
            let failure = sysroot::compile(&missing).unwrap_err();
            assert!(
                failure.contains("trait bound") || failure.contains("Type domain equality"),
                "{phase}, clause={clause_input}: {failure}"
            );
            if !clause_input {
                assert!(
                    failure.contains("UnvalidatedModel"),
                    "a missing implementation cannot erase the unresolved premise: {failure}"
                );
            }
            assert_eq!(sysroot::run(&source), Value::Bool(true));
        }
    }
}

#[test]
fn inherited_premises_and_concrete_implementations_remain_separate_evidence() {
    let source = r#"
trait Inspect { comptime fn inspect(self) -> bool; }
trait Child :- Inspect {}
impl Inspect for bool { comptime fn inspect(self) -> bool { self } }
impl Child for bool {}
impl Inspect for u8 { comptime fn inspect(self) -> bool { self == 7 } }
comptime fn leaf(comptime T: Type, value: T) -> bool
where requires T :- Inspect
{
    value.as(Inspect).inspect()
}
comptime fn query(comptime T: Type, value: T) -> bool
where requires T :- Child
{
    leaf(T, value) and leaf(u8, 7)
}
fn run() -> bool { const answer = query(bool, true); answer }
"#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let missing = source.replace("trait Child :- Inspect {}", "trait Child {}");
    let failure = sysroot::compile(&missing).unwrap_err();
    assert!(failure.contains("trait bound"), "{failure}");
    assert_eq!(sysroot::run(source), Value::Bool(true));
}
