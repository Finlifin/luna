//! Nominal formation uses the same capability world as ordinary invocation.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

#[test]
fn dynamic_nominal_formation_queries_unused_generic_capabilities() {
    for definition in [
        "struct Packet where flag: bool, T :- Enabled<flag> {}",
        "enum Packet where flag: bool, T :- Enabled<flag> { ready, }",
        "newtype Packet<flag: bool, T :- Enabled<flag>> = bool;",
        "typealias Packet<flag: bool, T :- Enabled<flag>> = bool;",
    ] {
        let source = format!(
            "trait Enabled where flag: bool {{}}
            struct Subject {{}}
            impl Enabled<flag> for Subject where flag: bool {{}}
            {definition}
            comptime fn Make(flag: bool) -> Type {{ Packet<flag,Subject> }}
            comptime fn CapabilityPacketType() -> Type {{ Make(true) }}
            comptime fn ExpectedCapabilityPacketType() -> Type {{ Packet<true,Subject> }}"
        );
        let package = sysroot::compile(&source).unwrap();
        let actual = luna::invocation::invoke(&package, "CapabilityPacketType", 1_000_000)
            .unwrap_or_else(|failure| panic!("{definition}: {failure}"));
        let expected =
            luna::invocation::invoke(&package, "ExpectedCapabilityPacketType", 1_000_000).unwrap();
        assert_eq!(actual, expected, "{definition}");

        let missing = source
            .replace(
                "impl Enabled<flag> for Subject where flag: bool {}",
                "impl Enabled<true> for Subject {}",
            )
            .replace("Make(true)", "Make(false)");
        let package = sysroot::compile(&missing).unwrap();
        let failure = luna::invocation::invoke(&package, "CapabilityPacketType", 1_000_000)
            .expect_err("forming a Type must not grant a missing capability");
        assert!(
            failure.contains("TypeContractRejected") || failure.contains("TypeContractUnresolved"),
            "{definition}: {failure}"
        );
    }
}
