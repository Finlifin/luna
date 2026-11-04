//! Prepared invocation retains live comptime value inputs and resolves their uses.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_executor::Context;
use comptime_interpretor::Value;
use nir::{DeclarationId, Primitive, Term, program::Package};

fn id(package: &Package, name: &str) -> DeclarationId {
    let mut matches = package.functions.iter().filter(|f| f.name == name);
    let declaration = matches.next().expect("named test entry").declaration;
    assert!(matches.next().is_none(), "ambiguous test entry: {name}");
    declaration
}
fn prepare(package: &Package, name: &str, arguments: Vec<Term>) -> luna::memoization::Invocation {
    luna::memoization::Invocation::prepare(
        package,
        &Default::default(),
        id(package, name),
        arguments,
        8,
        Default::default(),
    )
    .unwrap()
}

const SOURCE: &str = "
    struct Key where tag: bool { value: bool }
    pure comptime trait Reading {
        pure comptime fn read(self) -> bool;
        assoc initial: bool;
    }
    comptime impl Reading for Key<tag> where tag: bool {
        pure comptime fn read(self) -> bool { self.value == tag }
        assoc initial: bool = tag;
    }
    pure comptime fn inspect_key(value: Key<tag>) -> bool
    where tag: bool
    {
        value.value
    }
    pure comptime fn read_key(comptime enabled: bool, value: Key<tag>) -> bool
    where tag: bool
    {
        if enabled { value.as(Reading).read() } else { Key<tag>.as(Reading).initial }
    }
    pure comptime fn checked_key(comptime enabled: bool, value: Key<tag>) -> bool
    where tag: bool
    {
        assumes enabled;
        value.as(Reading).read()
    }
";

fn key(package: &Package, tag: bool, value: bool) -> Value {
    Value::Record {
        declaration: package
            .structs
            .iter()
            .find(|s| s.name == "Key")
            .unwrap()
            .declaration,
        arguments: vec![Term::Bool(tag)],
        fields: vec![Value::Bool(value)],
    }
}

#[test]
fn live_value_inputs_select_methods_and_associated_values_without_result_caching() {
    let package = sysroot::compile(SOURCE).unwrap();
    for tag in [false, true] {
        let mut inspect = prepare(&package, "inspect_key", vec![Term::Bool(tag)]);
        let mut read = prepare(&package, "read_key", vec![Term::Bool(tag)]);
        for (enabled, value) in [(true, false), (false, true), (true, true), (false, false)] {
            let outcome = inspect
                .invoke(
                    vec![key(&package, tag, value)],
                    &mut Context::new(1_000_000),
                )
                .unwrap();
            assert_eq!(*outcome.value, Value::Bool(value));
            assert!(!outcome.reused);
            let outcome = read
                .invoke(
                    vec![Value::Bool(enabled), key(&package, tag, value)],
                    &mut Context::new(1_000_000),
                )
                .unwrap();
            assert_eq!(
                *outcome.value,
                Value::Bool(if enabled { value == tag } else { tag })
            );
            assert!(!outcome.reused);
            assert_eq!(read.cached_results(), 0);
            assert_eq!(inspect.cached_results(), 0);
        }
        assert!(
            read.invoke(
                vec![Value::Bool(true), key(&package, !tag, true)],
                &mut Context::new(1_000_000)
            )
            .is_err()
        );
        assert_eq!(
            *read
                .invoke(
                    vec![Value::Bool(true), key(&package, tag, tag)],
                    &mut Context::new(1_000_000)
                )
                .unwrap()
                .value,
            Value::Bool(true)
        );
    }
}

#[test]
fn prepared_value_templates_recheck_contracts_and_recover_after_failures() {
    let package = sysroot::compile(SOURCE).unwrap();
    let mut call = prepare(&package, "checked_key", vec![Term::Bool(true)]);
    let inputs = |enabled| vec![Value::Bool(enabled), key(&package, true, true)];
    assert!(call.invoke(inputs(true), &mut Context::new(1)).is_err());
    for enabled in [true, false, true, false, true] {
        let outcome = call.invoke(inputs(enabled), &mut Context::new(1_000_000));
        if enabled {
            let outcome = outcome.unwrap();
            assert_eq!(*outcome.value, Value::Bool(true));
            assert!(!outcome.reused);
        } else {
            let failure = outcome
                .err()
                .expect("assumes must be checked on every call")
                .to_string();
            assert!(failure.contains("ProgramContractRejected"), "{failure}");
        }
        assert_eq!(call.cached_results(), 0);
    }
}

#[test]
fn type_results_use_real_meta_key_protocols_with_comptime_value_inputs() {
    let package = sysroot::compile(
        "pure comptime fn Select(comptime flag: bool) -> Type { if flag {bool} else {u8} }",
    )
    .unwrap();
    let items = key_protocols(&package);
    let mut call = luna::memoization::Invocation::prepare(
        &package,
        &items,
        id(&package, "Select"),
        vec![],
        8,
        Default::default(),
    )
    .unwrap();
    for (flag, reused) in [(false, false), (true, false), (false, true), (true, true)] {
        let outcome = call
            .invoke(vec![Value::Bool(flag)], &mut Context::new(1_000_000))
            .unwrap();
        assert_eq!(
            *outcome.value,
            Value::Type(Term::Primitive(if flag {
                Primitive::Bool
            } else {
                Primitive::U8
            }))
        );
        assert_eq!(outcome.reused, reused);
    }
    assert_eq!(call.cached_results(), 2);
}

fn key_protocols(package: &Package) -> intrinsic::LangItems {
    let mut items = intrinsic::LangItems::new();
    for (item, name) in [
        (intrinsic::LangItem::Eq, "ComptimeEq"),
        (intrinsic::LangItem::Hash, "ComptimeHash"),
    ] {
        let declaration = package
            .traits
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .declaration;
        items
            .bind_source_traits(package, &[(item, declaration)])
            .unwrap();
    }
    items
}

#[test]
fn type_input_dispatch_uses_source_methods_and_real_meta_cache_protocols() {
    let package = sysroot::compile(
        "pure comptime trait Choice {
            pure comptime fn Select() -> Type;
            assoc selected: Type;
        }
        comptime impl Choice for bool {
            pure comptime fn Select() -> Type { u8 }
            assoc selected: Type = u8;
        }
        comptime impl Choice for u8 {
            pure comptime fn Select() -> Type { bool }
            assoc selected: Type = bool;
        }
        pure comptime fn SelectFor(comptime T: Type) -> Type
        where requires T :- Choice
        {
            let select = T.as(Choice).Select;
            let actual = select();
            if actual == T.as(Choice).selected { actual } else { char }
        }
        pure comptime fn Resolve(comptime flag: bool) -> Type {
            if flag { SelectFor(bool) } else { SelectFor(u8) }
        }",
    )
    .unwrap();
    let items = key_protocols(&package);
    let prepare = |capacity| {
        luna::memoization::Invocation::prepare(
            &package,
            &items,
            id(&package, "Resolve"),
            vec![],
            capacity,
            Default::default(),
        )
        .unwrap()
    };
    let inputs = |flag| vec![Value::Bool(flag)];
    let mut call = prepare(8);
    for (input, output, reused) in [
        (true, Primitive::U8, false),
        (false, Primitive::Bool, false),
        (true, Primitive::U8, true),
        (false, Primitive::Bool, true),
    ] {
        let outcome = call
            .invoke(inputs(input), &mut Context::new(1_000_000))
            .unwrap();
        assert_eq!(*outcome.value, Value::Type(Term::Primitive(output)));
        assert_eq!(outcome.reused, reused);
        if reused {
            assert_eq!(call.last_execution_stats().bytecode_invocations, 0);
        } else {
            assert!(call.last_execution_stats().bytecode_invocations > 0);
        }
    }
    assert_eq!(call.cached_results(), 2);
    assert!(
        call.invoke(
            vec![Value::UnsignedInteger(1)],
            &mut Context::new(1_000_000)
        )
        .is_err()
    );
    assert_eq!(call.cached_results(), 2);
    assert!(
        call.invoke(inputs(true), &mut Context::new(1_000_000))
            .unwrap()
            .reused
    );

    // With result retention disabled, every success includes execution and
    // source result admission. Insufficient allowance must never publish a result.
    let mut uncached = prepare(0);
    uncached
        .invoke(inputs(true), &mut Context::new(1_000_000))
        .unwrap();
    let mut warm = Context::new(1_000_000);
    uncached.invoke(inputs(true), &mut warm).unwrap();
    let work = 1_000_000 - warm.remaining();
    assert!(work > 1);
    for allowance in [0, 1, work / 2, work - 1] {
        let mut limited = Context::new(allowance);
        assert!(uncached.invoke(inputs(true), &mut limited).is_err());
        assert_eq!(uncached.cached_results(), 0);
        let mut recovered = Context::new(1_000_000);
        let outcome = uncached.invoke(inputs(true), &mut recovered).unwrap();
        assert_eq!(*outcome.value, Value::Type(Term::Primitive(Primitive::U8)));
        assert!(!outcome.reused);
        assert_eq!(1_000_000 - recovered.remaining(), work);
    }
}
