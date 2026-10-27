#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

fn source(body: &str) -> String {
    format!(
        "{} struct String {{}}\nmod comparison {{ {} }}\nmod resources {{ {} }}\nmod conversion {{ {} }}\nmod traits {{ {} }}\npub use traits.*;\n{body}",
        include_str!("../../../library/builtin/primitives.fl"),
        include_str!("../../../library/builtin/comparison.fl"),
        include_str!("../../../library/builtin/resources.fl"),
        include_str!("../../../library/builtin/conversion.fl"),
        include_str!("../../../library/builtin/traits.fl"),
    )
}

#[test]
fn scalar_hashes_agree_across_execution_phases_and_integer_boundaries() {
    for (ty, literal, expected) in [
        ("bool", "false", 0),
        ("bool", "true", 1),
        ("char", "'A'", 65),
        ("char", "'\u{d7ff}'", 0xd7ff),
        ("char", "'\u{e000}'", 0xe000),
        ("char", "'🦀'", 0x1f980),
        ("char", "'\u{10ffff}'", 0x10ffff),
        ("u8", "255", 255),
        ("u16", "65535", 65535),
        ("u32", "4294967295", 4294967295),
        ("u64", "18446744073709551615", u64::MAX),
        ("i8", "-128", 128),
        ("i16", "-32768", 32768),
        ("i32", "-2147483648", 2147483648),
        ("i64", "-9223372036854775808", 1u64 << 63),
        ("u128", "18446744073709551616", 1),
        ("u128", "340282366920938463463374607431768211455", 0),
        (
            "i128",
            "-170141183460469231731687303715884105728",
            1u64 << 63,
        ),
        ("i128", "-1", 0),
    ] {
        let package = support::compile(&source(&format!(
            "fn run()->u64 {{let value:{ty}={literal};value.as(Hash).hash()}}\ncomptime fn staged()->u64 {{let value:{ty}={literal};value.as(Hash).comptime_hash()}}"
        )));
        let expected = Value::UnsignedInteger(expected.into());
        assert_eq!(
            luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
            expected,
            "{ty} {literal}"
        );
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            expected,
            "{ty} {literal}"
        );
    }
}

#[test]
fn wide_hash_collisions_still_require_equality() {
    use comptime_executor::{Context, KeyProtocol};
    let package = support::compile(&source(""));
    let mut items = intrinsic::LangItems::new();
    let bindings = [
        (intrinsic::LangItem::Eq, "Eq"),
        (intrinsic::LangItem::Hash, "Hash"),
    ]
    .map(|(item, name)| {
        (
            item,
            package
                .traits
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .declaration,
        )
    });
    items.bind_source_traits(&package, &bindings).unwrap();
    let mut protocol = luna::comptime_keys::Protocol::prepare(
        &package,
        &items,
        &nir::Term::Primitive(nir::Primitive::U128),
        Default::default(),
    )
    .unwrap();
    let low = Value::UnsignedInteger(1);
    let high = Value::UnsignedInteger(1u128 << 64);
    let mut context = Context::new(1_000_000);
    assert_eq!(
        protocol.hash(&low, &mut context).unwrap(),
        protocol.hash(&high, &mut context).unwrap()
    );
    assert!(!protocol.equivalent(&low, &high, &mut context).unwrap());
    assert!(protocol.equivalent(&high, &high, &mut context).unwrap());
}

#[test]
fn scalar_hashes_lower_to_host_code_without_comptime_dispatch() {
    let package = support::compile(&source(
        "fn verify() {\nlet wide:u128=18446744073709551616;\nlet signed:i128=-170141183460469231731687303715884105728;\nlet wide_hash=wide.as(Hash).hash();assumes wide_hash == 1;\nlet signed_hash=signed.as(Hash).hash();assumes signed_hash == 9223372036854775808;\nlet bool_hash=true.as(Hash).hash();assumes bool_hash == 1;\n}",
    ));
    let ir = luna::host::llvm_ir(&package, "verify", 1_000_000).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn character_bit_reads_do_not_authorize_reconstruction_or_writeback() {
    for body in [
        "let value:u32=65;value'bitvec.as(char)",
        "var value:char='A';value'bitvec[0..32]=55296;value",
    ] {
        let error = support::compile_with_protocols(
            &source(&format!("fn invalid()->char {{{body}}}")),
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            error.contains("representation") || error.contains("unrestricted"),
            "{error}"
        );
    }
    let package = support::compile(&source(
        "fn read(value:char)->u64 {value'bitvec.as(u64)} fn verify() {let value=read('🦀');assumes value == 129408;}",
    ));
    let ir = luna::host::llvm_ir(&package, "verify", 1_000_000).unwrap();
    luna::host::Toolchain::default()
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
}

#[test]
fn symbol_hashes_follow_utf8_content_in_both_phases_and_native_code() {
    // Published FNV-1a vectors, plus a separately computed UTF-8 vector.
    for (name, expected) in [
        ("a", 0xaf63dc4c8601ec8cu64),
        ("foobar", 0x85944171f73967e8u64),
        ("é", 0x0ac21707b7181e01u64),
    ] {
        let package = support::compile(&source(&format!(
            "fn hash(value:Symbol)->u64 {{value.as(Hash).hash()}} fn run()->u64 {{hash(.{name})}} comptime fn staged()->u64 {{let value:Symbol=.{name};value.as(Hash).comptime_hash()}} fn verify() {{let value=run();assumes value == {expected};}}"
        )));
        assert_eq!(
            luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
            Value::UnsignedInteger(expected.into()),
            "{name}"
        );
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            Value::UnsignedInteger(expected.into()),
            "{name}"
        );
        let ir = luna::host::llvm_ir(&package, "verify", 1_000_000).unwrap();
        for optimization in [
            luna::host::OptimizationLevel::O0,
            luna::host::OptimizationLevel::O3,
        ] {
            luna::host::Toolchain {
                optimization,
                ..Default::default()
            }
            .execute_test(&ir, std::time::Duration::from_secs(5))
            .unwrap();
        }
    }
}

#[test]
fn symbol_cache_keys_compare_content_and_recover_after_budget_exhaustion() {
    use comptime_executor::{Context, KeyProtocol};
    let package = support::compile(&source(""));
    let mut items = intrinsic::LangItems::new();
    let bindings = [
        (intrinsic::LangItem::Eq, "Eq"),
        (intrinsic::LangItem::Hash, "Hash"),
    ]
    .map(|(item, name)| {
        (
            item,
            package
                .traits
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .declaration,
        )
    });
    items.bind_source_traits(&package, &bindings).unwrap();
    let mut protocol = luna::comptime_keys::Protocol::prepare(
        &package,
        &items,
        &nir::Term::Primitive(nir::Primitive::Symbol),
        Default::default(),
    )
    .unwrap();
    let first = Value::Symbol("é".repeat(128).into());
    let same = Value::Symbol("é".repeat(128).into());
    let different = Value::Symbol("e".repeat(128).into());
    assert!(protocol.hash(&first, &mut Context::new(1)).is_err());
    let mut context = Context::new(4_000_000);
    let hash = protocol.hash(&first, &mut context).unwrap();
    assert_eq!(hash, protocol.hash(&same, &mut context).unwrap());
    assert!(protocol.equivalent(&first, &same, &mut context).unwrap());
    assert!(
        !protocol
            .equivalent(&first, &different, &mut context)
            .unwrap()
    );
    // The empty name is admitted by the VM Symbol domain even though a source
    // symbol literal normally spells an identifier.
    assert_eq!(
        protocol
            .hash(&Value::Symbol("".into()), &mut context)
            .unwrap(),
        protocol
            .hash(&Value::Symbol(String::new().into()), &mut context)
            .unwrap()
    );
}
