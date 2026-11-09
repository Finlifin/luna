//! Signature premises are formed to a fixed point without assuming their uses.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

const WORLD: &str = r#"
trait Source { assoc Item: Type; }
trait Allowed {}
impl Source for bool { assoc Item: Type = bool; }
impl Allowed for bool {}
"#;

fn source(clause_input: bool, bounds: &str, extra: &str) -> String {
    let (parameters, clauses, call) = if clause_input {
        (
            "value: A.as(Source).Item.as(Source).Item",
            format!("where A, {bounds}"),
            "accept<bool>(true)",
        )
    } else {
        (
            "comptime A: Type, value: A.as(Source).Item.as(Source).Item",
            format!("where {bounds}"),
            "accept(bool, true)",
        )
    };
    format!(
        "{WORLD}\n{extra}\nfn accept({parameters}) -> bool\n{clauses}\n{{ true }}\nfn run() -> bool {{ {call} }}"
    )
}

#[test]
fn every_nested_premise_order_uses_only_previously_formed_evidence() {
    let atoms = [
        "A :- Source",
        "A.as(Source).Item :- Source",
        "A.as(Source).Item.as(Source).Item :- Allowed",
    ];
    for clause in [false, true] {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let bounds = order.map(|index| format!("requires {}", atoms[index]));
            assert_eq!(
                sysroot::run(&source(clause, &bounds.join(", "), "")),
                Value::Bool(true),
                "clause={clause}, order={order:?}"
            );
        }
    }
}

#[test]
fn missing_and_self_supporting_premises_remain_errors_after_other_progress() {
    let complete = "requires A.as(Source).Item.as(Source).Item :- Allowed, \
        requires A.as(Source).Item :- Source, requires A :- Source";
    for clause in [false, true] {
        for bounds in [
            "requires A.as(Source).Item.as(Source).Item :- Allowed, requires A :- Source",
            "requires A.as(Source).Item :- Source",
            "requires A.as(Source).Item :- Source, requires A :- Source or A :- Allowed",
        ] {
            let failure = sysroot::compile(&source(clause, bounds, "")).unwrap_err();
            assert!(
                failure.contains("UnvalidatedModel")
                    || failure.contains("requires established trait evidence")
                    || failure.contains("given interface constraint"),
                "clause={clause}, {bounds}: {failure}"
            );
        }
        assert_eq!(
            sysroot::run(&source(clause, complete, "")),
            Value::Bool(true)
        );
    }
}

#[test]
fn an_invalid_atom_is_not_hidden_by_pending_or_completed_siblings() {
    for invalid_first in [false, true] {
        let valid = "requires A.as(Source).Item.as(Source).Item :- Allowed, \
            requires A.as(Source).Item :- Source, requires A :- Source";
        let invalid = "requires A :- Flag<bool>";
        let bounds = if invalid_first {
            format!("{invalid}, {valid}")
        } else {
            format!("{valid}, {invalid}")
        };
        let failure =
            sysroot::compile(&source(false, &bounds, "trait Flag where enabled: bool {}"))
                .unwrap_err();
        assert!(failure.contains("type mismatch"), "{failure}");
        assert!(!failure.contains("UnvalidatedModel"), "{failure}");
    }
}
