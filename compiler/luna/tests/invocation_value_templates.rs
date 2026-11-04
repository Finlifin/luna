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
            .bind_source_traits(&package, &[(item, declaration)])
            .unwrap();
    }
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
