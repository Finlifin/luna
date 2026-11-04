//! Preserve type-query failure categories through the invocation boundary.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_executor::{Context, Error};
use comptime_interpretor::Value;
use nir::{Primitive, Term};

#[test]
fn invocation_contract_exhaustion_is_not_a_protocol_error_or_a_cached_result() {
    let package = sysroot::compile(
        "trait Marker {}
        impl Marker for bool {}
        pure comptime fn accept(comptime T: Type) -> bool
        where requires T :- Marker
        {
            true
        }",
    )
    .unwrap();
    let declaration = package
        .functions
        .iter()
        .find(|f| f.name == "accept")
        .unwrap()
        .declaration;
    let mut call = luna::memoization::Invocation::prepare(
        &package,
        &intrinsic::LangItems::new(),
        declaration,
        vec![],
        8,
        Default::default(),
    )
    .unwrap();
    for steps in [0, 1, 8, 16, 32] {
        let mut budget = Context::new(steps);
        let result = call.invoke(
            vec![Value::Type(Term::Primitive(Primitive::Bool))],
            &mut budget,
        );
        let error = result
            .err()
            .expect("insufficient contract budget must reject");
        assert!(
            matches!(error, Error::BudgetExhausted),
            "steps {steps}: {error:?}"
        );
        assert_eq!(call.last_execution_stats().bytecode_invocations, 0);
        assert_eq!(call.cached_results(), 0);
        assert!(budget.remaining() <= steps);
    }
    let rejected = call.invoke(
        vec![Value::Type(Term::Primitive(Primitive::Char))],
        &mut Context::new(1_000_000),
    );
    let error = rejected.err().expect("unimplemented Marker must reject");
    assert!(matches!(error, Error::Protocol(_)), "{error:?}");
    assert_eq!(call.last_execution_stats().bytecode_invocations, 0);
    assert_eq!(call.cached_results(), 0);
    for _ in 0..2 {
        let value = call
            .invoke(
                vec![Value::Type(Term::Primitive(Primitive::Bool))],
                &mut Context::new(1_000_000),
            )
            .unwrap();
        assert_eq!(*value.value, Value::Bool(true));
        assert!(!value.reused);
        assert!(call.last_execution_stats().bytecode_invocations > 0);
        assert_eq!(call.cached_results(), 0);
    }
}
