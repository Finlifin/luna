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

#[test]
fn nominal_values_recheck_capabilities_in_the_receiving_world() {
    for (definition, construction, read) in [
        (
            "struct Packet where flag: bool, T :- Enabled<flag> { value: bool }",
            "Packet<flag, Subject> { value: true }",
            "value.value",
        ),
        (
            "enum Packet where flag: bool, T :- Enabled<flag> { ready(bool), }",
            "Packet<flag, Subject>.ready(true)",
            "value match { .ready(result) => result }",
        ),
        (
            "newtype Packet<flag: bool, T :- Enabled<flag>> = bool;",
            "true.as(Packet<flag, Subject>)",
            "value.as(bool)",
        ),
    ] {
        let source = format!(
            "trait Enabled where flag: bool {{}}
            struct Subject {{}}
            impl Enabled<flag> for Subject where flag: bool {{}}
            {definition}
            comptime fn make() -> Packet<flag, Subject>
            where flag: bool
            {{ {construction} }}
            comptime fn relay(value: Packet<flag, Subject>) -> Packet<flag, Subject>
            where flag: bool
            {{ value }}
            comptime fn read() -> bool
            where flag: bool
            {{ let value = relay<flag>(make<flag>()); {read} }}
            comptime fn dynamic_read(flag: bool) -> bool {{ read<flag>() }}
            comptime fn read_packet() -> bool {{ dynamic_read(true) }}"
        );
        let package = sysroot::compile(&source).unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "read_packet", 1_000_000)
                .unwrap_or_else(|failure| panic!("{definition}: {failure}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

fn invoke_dynamic_packet(body: &str) -> Result<comptime_interpretor::Value, String> {
    invoke_packet_program("", body)
}

fn invoke_packet_program(
    definitions: &str,
    body: &str,
) -> Result<comptime_interpretor::Value, String> {
    let source = format!(
        "trait Enabled where flag: bool {{}}
        struct Subject {{}}
        impl Enabled<flag> for Subject where flag: bool {{}}
        struct Packet where flag: bool, T :- Enabled<flag> {{ value: bool }}
        {definitions}
        comptime fn read() -> bool
        where flag: bool
        {{ {body} }}
        comptime fn dynamic_read(flag: bool) -> bool {{ read<flag>() }}
        comptime fn read_stored_packet() -> bool {{ dynamic_read(true) }}"
    );
    let package = sysroot::compile(&source)?;
    luna::invocation::invoke(&package, "read_stored_packet", 1_000_000)
}

#[test]
fn dynamic_nominal_values_survive_closure_capture() {
    assert_eq!(
        invoke_dynamic_packet(
            "
        let packet = Packet<flag, Subject> { value: true };
        let read: %{packet: Packet<flag, Subject>} comptime fn() -> bool = || packet.value;
        read()
    "
        )
        .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn dynamic_nominal_values_survive_object_edits() {
    assert_eq!(
        invoke_dynamic_packet(
            "
        var object = {packet: Packet<flag, Subject> { value: true }};
        let saved = object;
        object.set(0, Packet<flag, Subject> { value: false });
        (saved.get<Packet<flag, Subject>>(.packet) match {
            value? => value.value, null => false
        }) and (object.get<Packet<flag, Subject>>(.packet) match {
            value? => not value.value, null => false
        })
    "
        )
        .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn dynamic_nominal_associated_values_keep_their_formation_contracts() {
    assert_eq!(
        invoke_packet_program(
            "
        trait Source where flag: bool { assoc packet: Packet<flag, Subject>; }
        impl Source<flag> for bool where flag: bool {
            assoc packet: Packet<flag, Subject> = (Packet<flag, Subject> { value: true });
        }
    ",
            "bool.as(Source<flag>).packet.value"
        )
        .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn dynamic_nominal_clause_values_keep_their_formation_contracts() {
    assert_eq!(
        invoke_packet_program(
            "
        comptime fn selected() -> bool
        where flag: bool, packet: Packet<flag, Subject>
        { packet.value }
    ",
            "selected<flag, Packet<flag, Subject> { value: true }>()"
        )
        .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn dynamic_nominal_enum_names_keep_their_formation_contracts() {
    assert_eq!(
        invoke_packet_program(
            "
        enum Choice where flag: bool, T :- Enabled<flag> { ready, }
    ",
            "Choice<flag, Subject>.ready'tag_name == \"ready\""
        )
        .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}
