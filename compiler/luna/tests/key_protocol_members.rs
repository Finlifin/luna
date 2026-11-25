//! Key protocol identity selects the phase; method names remain eq and hash.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_executor::{Context, KeyProtocol};
use comptime_interpretor::Value;
use intrinsic::{LangItem, LangItems};
use luna::comptime_keys::Protocol;
use nir::{Primitive, Term, program::Package};

fn prepare(package: &Package, equality: &str, hashing: &str) -> Result<Protocol, String> {
    let mut items = LangItems::new();
    for (item, name) in [(LangItem::Eq, equality), (LangItem::Hash, hashing)] {
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
    Protocol::prepare(
        package,
        &items,
        &Term::Primitive(Primitive::Bool),
        elaboration::ptc::Budget {
            steps: 1_000_000,
            ..Default::default()
        },
    )
}

#[test]
fn canonical_key_members_are_not_shadowed_by_legacy_spellings() {
    let package = sysroot::compile(
        "pure comptime trait KeyEquality {
            comptime fn eq(self, other: Self) -> bool;
            comptime fn comptime_eq(self, other: Self) -> bool;
        }
        pure comptime trait KeyHashing {
            comptime fn hash(self) -> u64;
            comptime fn comptime_hash(self) -> u64;
        }
        comptime impl KeyEquality for bool {
            comptime fn eq(self, other: Self) -> bool { self == other }
            comptime fn comptime_eq(self, other: Self) -> bool { false }
        }
        comptime impl KeyHashing for bool {
            comptime fn hash(self) -> u64 { if self { 1 } else { 0 } }
            comptime fn comptime_hash(self) -> u64 { 0 }
        }",
    )
    .unwrap();
    let mut protocol = prepare(&package, "KeyEquality", "KeyHashing").unwrap();
    let mut context = Context::new(1_000_000);
    let yes = protocol.admit(Value::Bool(true), &mut context).unwrap();
    let no = protocol.admit(Value::Bool(false), &mut context).unwrap();
    assert!(protocol.equivalent(&yes, &yes, &mut context).unwrap());
    assert!(!protocol.equivalent(&yes, &no, &mut context).unwrap());
    assert_ne!(
        protocol.hash(&yes, &mut context).unwrap(),
        protocol.hash(&no, &mut context).unwrap()
    );
}

#[test]
fn prefixed_members_cannot_supply_missing_protocol_operations() {
    let package = sysroot::compile(
        "pure comptime trait LegacyEquality {
            comptime fn comptime_eq(self, other: Self) -> bool;
        }
        pure comptime trait LegacyHashing {
            comptime fn comptime_hash(self) -> u64;
        }
        comptime impl LegacyEquality for bool {
            comptime fn comptime_eq(self, other: Self) -> bool { self == other }
        }
        comptime impl LegacyHashing for bool {
            comptime fn comptime_hash(self) -> u64 { 0 }
        }",
    )
    .unwrap();
    for (equality, hashing) in [
        ("LegacyEquality", "ComptimeHash"),
        ("ComptimeEq", "LegacyHashing"),
    ] {
        let error = prepare(&package, equality, hashing)
            .err()
            .expect("canonical member is absent");
        assert!(error.contains("protocol method is missing"), "{error}");
    }
}

#[test]
fn prefixed_comptime_members_cannot_bypass_canonical_member_phase() {
    let package = sysroot::compile(
        "trait MixedEquality {
            fn eq(self, other: Self) -> bool;
            ^attrs.comptime_phase
            comptime fn comptime_eq(self, other: Self) -> bool;
        }
        trait MixedHashing {
            fn hash(self) -> u64;
            ^attrs.comptime_phase
            comptime fn comptime_hash(self) -> u64;
        }
        impl MixedEquality for bool {
            fn eq(self, other: Self) -> bool { self == other }
        }
        comptime impl MixedEquality for bool {
            comptime fn comptime_eq(self, other: Self) -> bool { self == other }
        }
        impl MixedHashing for bool {
            fn hash(self) -> u64 { 0 }
        }
        comptime impl MixedHashing for bool {
            comptime fn comptime_hash(self) -> u64 { 0 }
        }",
    )
    .unwrap();
    for (equality, hashing) in [
        ("MixedEquality", "ComptimeHash"),
        ("ComptimeEq", "MixedHashing"),
    ] {
        let error = prepare(&package, equality, hashing)
            .err()
            .expect("canonical member has runtime membership");
        assert!(error.contains("comptime trait membership"), "{error}");
    }
}
