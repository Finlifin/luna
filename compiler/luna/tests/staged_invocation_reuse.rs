#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_executor::Context;
use comptime_interpretor::Value;
use nir::{Primitive, Term};

fn prepare(source: &str) -> luna::memoization::Invocation {
    let package = support::compile(source);
    luna::memoization::Invocation::prepare(
        &package,
        &Default::default(),
        support::id(&package, "choose"),
        vec![],
        0,
        Default::default(),
    )
    .unwrap()
}
const SOURCE: &str = "typealias Type=.Type'builtin;
    pure comptime fn choose(comptime T:Type, value:bool)->bool {
        if T == bool {not value} else {value}
    }";

#[test]
fn prepared_graph_reuses_code_but_executes_fresh_arguments_and_replaces_type_keys() {
    let mut call = prepare(SOURCE);
    for (ty, value, expected, preparations, warm) in [
        (Primitive::Bool, true, false, 1, false),
        (Primitive::Bool, false, true, 1, true),
        (Primitive::Char, false, false, 2, false),
        (Primitive::Char, true, true, 2, true),
        (Primitive::Bool, true, false, 3, false),
    ] {
        let result = call
            .invoke(
                vec![Value::Type(Term::Primitive(ty)), Value::Bool(value)],
                &mut Context::new(1_000_000),
            )
            .unwrap();
        assert_eq!(*result.value, Value::Bool(expected));
        assert!(!result.reused);
        assert_eq!(call.cached_results(), 0);
        assert_eq!(call.staged_preparations(), preparations);
        let stats = call.last_execution_stats();
        assert!(stats.bytecode_invocations > 0);
        assert_eq!(stats.bytecode_compilations == 0, warm);
    }
}

#[test]
fn failed_calls_preserve_only_prepared_code_and_never_contract_results() {
    let mut call = prepare(
        "typealias Type=.Type'builtin;
        pure comptime fn choose(comptime T:Type, value:bool)->bool {
            assumes value;
            T == bool
        }",
    );
    let inputs = |value| {
        vec![
            Value::Type(Term::Primitive(Primitive::Bool)),
            Value::Bool(value),
        ]
    };
    assert!(call.invoke(inputs(true), &mut Context::new(1)).is_err());
    assert_eq!(call.staged_preparations(), 0);
    assert_eq!(
        *call
            .invoke(inputs(true), &mut Context::new(1_000_000))
            .unwrap()
            .value,
        Value::Bool(true)
    );
    assert!(
        call.invoke(inputs(false), &mut Context::new(1_000_000))
            .is_err()
    );
    assert_eq!(call.staged_preparations(), 1);
    assert_eq!(call.last_execution_stats().bytecode_compilations, 0);
    assert_eq!(
        *call
            .invoke(inputs(true), &mut Context::new(1_000_000))
            .unwrap()
            .value,
        Value::Bool(true)
    );
    assert_eq!(call.staged_preparations(), 1);
    assert_eq!(call.last_execution_stats().bytecode_compilations, 0);
    assert_eq!(call.cached_results(), 0);
}

#[test]
fn type_key_positions_do_not_capture_interleaved_ordinary_values() {
    let mut call = prepare(
        "typealias Type=.Type'builtin;
        pure comptime fn choose(comptime A:Type, value:bool, comptime B:Type)->bool {
            if A == B {value} else {not value}
        }",
    );
    for (a, b, value, expected, count) in [
        (Primitive::Bool, Primitive::Char, true, false, 1),
        (Primitive::Bool, Primitive::Char, false, true, 1),
        (Primitive::Char, Primitive::Bool, true, false, 2),
        (Primitive::Char, Primitive::Char, true, true, 3),
    ] {
        let result = call
            .invoke(
                vec![
                    Value::Type(Term::Primitive(a)),
                    Value::Bool(value),
                    Value::Type(Term::Primitive(b)),
                ],
                &mut Context::new(1_000_000),
            )
            .unwrap();
        assert_eq!(*result.value, Value::Bool(expected));
        assert_eq!(call.staged_preparations(), count);
        assert!(!result.reused);
    }
}

#[test]
fn dependent_type_domain_values_remain_live_inputs_to_a_warm_graph() {
    let mut call = prepare(
        "typealias Type=.Type'builtin;
        pure comptime fn choose(comptime kind:Type, comptime T:kind, value:bool)->bool
        where requires kind == Type
        {
            if T == bool {value} else {not value}
        }",
    );
    for (ty, expected) in [
        (Primitive::Bool, true),
        (Primitive::Char, false),
        (Primitive::Bool, true),
    ] {
        let result = call
            .invoke(
                vec![
                    Value::Type(Term::Primitive(Primitive::Type)),
                    Value::Type(Term::Primitive(ty)),
                    Value::Bool(true),
                ],
                &mut Context::new(1_000_000),
            )
            .unwrap();
        assert_eq!(*result.value, Value::Bool(expected));
        assert!(!result.reused);
    }
}
