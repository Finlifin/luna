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
fn prepared_source_reuses_input_independent_code_across_type_values() {
    let mut call = prepare(SOURCE);
    for (ty, value, expected, warm) in [
        (Primitive::Bool, true, false, false),
        (Primitive::Bool, false, true, true),
        (Primitive::Char, false, false, true),
        (Primitive::Char, true, true, true),
        (Primitive::Bool, true, false, true),
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
    assert_eq!(call.last_execution_stats().bytecode_compilations, 0);
    assert_eq!(
        *call
            .invoke(inputs(true), &mut Context::new(1_000_000))
            .unwrap()
            .value,
        Value::Bool(true)
    );
    assert_eq!(call.last_execution_stats().bytecode_compilations, 0);
    assert_eq!(call.cached_results(), 0);
}

#[test]
fn type_inputs_do_not_capture_interleaved_ordinary_values() {
    let mut call = prepare(
        "typealias Type=.Type'builtin;
        pure comptime fn choose(comptime A:Type, value:bool, comptime B:Type)->bool {
            if A == B {value} else {not value}
        }",
    );
    for (a, b, value, expected) in [
        (Primitive::Bool, Primitive::Char, true, false),
        (Primitive::Bool, Primitive::Char, false, true),
        (Primitive::Char, Primitive::Bool, true, false),
        (Primitive::Char, Primitive::Char, true, true),
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
        assert!(!result.reused);
    }
}

#[test]
fn dependent_type_domain_values_remain_live_inputs_to_the_source_vm() {
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

#[test]
fn dependent_declaration_domains_govern_cache_preparation_and_execution() {
    let package = support::compile("pure comptime fn project() -> T where T, X: T { X }");
    let declaration = support::id(&package, "project");
    let prepare = |arguments| {
        luna::memoization::Invocation::prepare(
            &package,
            &Default::default(),
            declaration,
            arguments,
            8,
            Default::default(),
        )
    };
    let ty = Term::Primitive(Primitive::Type);
    let boolean = Term::Primitive(Primitive::Bool);

    for arguments in [
        vec![ty.clone(), Term::Bool(true)],
        vec![boolean.clone(), boolean.clone()],
        vec![
            boolean.clone(),
            Term::Integer(comptime_runtime::Integer::from_u128(1)),
        ],
    ] {
        assert!(prepare(arguments).is_err());
    }
    for (arguments, expected, cacheable) in [
        (
            vec![ty, boolean.clone()],
            Value::Type(boolean.clone()),
            true,
        ),
        (vec![boolean, Term::Bool(false)], Value::Bool(false), false),
    ] {
        let mut call = prepare(arguments).unwrap();
        let mut context = Context::new(1_000_000);
        for reused in [false, cacheable] {
            let result = call.invoke(vec![], &mut context).unwrap();
            assert_eq!(*result.value, expected);
            assert_eq!(result.reused, reused);
        }
        assert_eq!(call.cached_results(), usize::from(cacheable));
    }
}

#[test]
fn reusable_call_checks_trait_arguments_in_the_admitted_declaration_frame() {
    let package = support::compile(
        "trait Allows where T, X: T {}
         impl Allows<bool'type, bool> for bool {}
         pure comptime fn allowed() -> bool
         where T, X: T, requires bool :- Allows<T, X>
         {
             true
         }",
    );
    let prepare = |arguments| {
        luna::memoization::Invocation::prepare(
            &package,
            &Default::default(),
            support::id(&package, "allowed"),
            arguments,
            8,
            Default::default(),
        )
    };
    let boolean = Term::Primitive(Primitive::Bool);
    let reject = prepare(vec![boolean.clone(), Term::Bool(true)])
        .err()
        .unwrap();
    assert!(
        reject.contains("type contract is not established"),
        "{reject}"
    );
    let mut call = prepare(vec![Term::Primitive(Primitive::Type), boolean]).unwrap();
    assert_eq!(
        *call
            .invoke(vec![], &mut Context::new(1_000_000))
            .unwrap()
            .value,
        Value::Bool(true)
    );
}
