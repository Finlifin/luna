//! Bit computations share P4 evidence with finite suppliers; range and affine
//! obligations still pass through the ordinary checked execution pipeline.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{calls, compile, compile_with_protocols, id};

const TYPES: &str = "typealias Word=.u16'builtin;typealias Byte=.u8'builtin;typealias Bits=.BitVec'builtin<16>;typealias Nibble=.BitVec'builtin<4>;";

fn run(source: &str, entries: &[(&str, u128)]) {
    let package = compile(&format!("{TYPES}{source}"));
    for (entry, expected) in entries {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::UnsignedInteger(*expected),
            "{entry}"
        );
    }
}

#[test]
fn dynamic_reads_use_parameter_contracts() {
    run(
        "pure fn select(bits:Bits,i:Byte)->Nibble where requires i <= 12 {bits[i..(i + 4)]}
         pure fn bit(bits:Bits,i:Byte)->.BitVec'builtin<1> where requires i < 16 {bits[i]}
         pure fn run()->Word {select(0xABCD,4).as(Word)}
         pure fn one()->Word {bit(0xABCD,15).as(Word)}",
        &[("run", 12), ("one", 1)],
    );
}

#[test]
fn private_writeback_supports_static_dynamic_and_nested_selections() {
    run(
        "pure fn fixed()->Word {var word:Word=0x1234;word'bitvec[4..8]=0b1010;word}
         pure fn index(i:Byte)->Word where requires i < 16 {var word:Word=0;word'bitvec[i]=1;word}
         pure fn nested(a:Byte,b:Byte)->Word where requires a <= 8, requires b <= 4 {
             var word:Word=0;word'bitvec[a..(a + 8)][b..(b + 4)]=0b1010;word}
         pure fn dynamic()->Word {index(15)}
         pure fn run()->Word {nested(4,2)}",
        &[("fixed", 0x12A4), ("dynamic", 32768), ("run", 640)],
    );
}

#[test]
fn writeback_freezes_positions_but_preserves_rhs_storage_updates() {
    run(
        "pure fn run()->Word {var word:Word=0x1234;var i:Byte=4;
             word'bitvec[i..(i + 4)]=do {i=10;word=0xABCD;0b1010};word}
         pure fn overlap()->Word {var word:Word=0x1234;let i:Byte=4;
             word'bitvec[i..(i + 8)]=word'bitvec[0..8];word}
         pure fn empty()->Word {var word:Word=0x1234;let i:Byte=16;
             word'bitvec[i..i]=do {word=0xABCD;0};word}",
        &[("run", 0xABAD), ("overlap", 0x1344), ("empty", 0xABCD)],
    );
}

#[test]
fn read_and_write_operands_keep_source_order() {
    let package = compile(&format!(
        "{TYPES}
        fn low()->result:Byte where ensures result == 4 {{4}}
        fn high()->result:Byte where ensures result == 8 {{8}}
        fn payload()->Nibble {{0b1010}}
        fn operand()->Bits {{0xABCD}}
        pure fn write()->Word {{var word:Word=0x1234;word'bitvec[low()..high()]=payload();word}}
        pure fn read()->Word {{let part:Nibble=operand()[low()..high()];part.as(Word)}}"
    ));
    for (entry, expected, order) in [
        ("write", 0x12A4, vec!["write", "low", "high", "payload"]),
        ("read", 12, vec!["read", "operand", "low", "high"]),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, entry), vec![]).unwrap(),
            Value::UnsignedInteger(expected)
        );
        assert_eq!(calls(&package, &vm), order);
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::UnsignedInteger(expected)
        );
    }
}

#[test]
fn finite_bit_helpers_and_default_local_storage_are_admitted() {
    run(
        "fn build()->Word {var word:Word=0;let i:Byte=4;word'bitvec[i..(i + 4)]=0b1010;word}
         fn take(.word:Word=build())->Word {word}
         fn local(.word:Word=do {var bits:Bits=0;let i:Byte=4;bits[i]=1;bits.as(Word)})->Word {word}
         fn select(bits:Bits,.part:Nibble=do {let i:Byte=4;bits[i..(i + 4)]})->Word {part.as(Word)}
         fn run()->Word {take()}
         fn local_run()->Word {local()}
         fn read_run()->Word {select(0xABCD)}",
        &[("run", 160), ("local_run", 16), ("read_run", 12)],
    );
}

#[test]
fn defaults_cannot_modify_an_earlier_parameter_bit_view() {
    for body in ["bits[0..4]=0;bits", "let i:Byte=4;bits[i]=1;bits"] {
        let source = format!("{TYPES}fn take(bits:Bits,.other:Bits=do {{{body}}})->Bits {{other}}");
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("P3 stability/termination"), "{error}");
    }
}

#[test]
fn bit_reads_and_writes_do_not_hide_external_memory() {
    for body in [
        "pure fn bad(p:*Word)->Word {p.*'bitvec[0..4].as(Word)}",
        "pure fn bad(p:*Word) {p.*'bitvec[0..4]=0;}",
        "pure fn bad(p:*Word) {let i:Byte=4;p.*'bitvec[i]=1;}",
        "pure fn bad(p:*Word)->Word {var word:Word=0;let i:Byte=4;word'bitvec[i..(i + 4)]=p.*'bitvec[0..4];word}",
        "pure fn bad(p:*Byte)->Word {var word:Word=0;word'bitvec[do {p.* = 4;let i:Byte=4;i}]=1;word}",
        "pure fn bad(p:*Byte)->Word {let bits:Bits=0;let part:Nibble=bits[(do {p.* = 4;let i:Byte=4;i})..8];part.as(Word)}",
        "pure fn bad(p:*Byte)->Word {var word:Word=0;let i:Byte=4;let payload:Nibble=0;word'bitvec[i..do {p.* = 8;let end:Byte=8;end}]=payload;word}",
        "pure fn bad(p:*Byte)->Word {var word:Word=0;let i:Byte=4;word'bitvec[i..(i + 8)][do {p.* = 1;let j:Byte=1;j}]=1;word}",
    ] {
        let error =
            compile_with_protocols(&format!("{TYPES}{body}"), &Default::default()).unwrap_err();
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
}

#[test]
fn pure_bit_operations_still_require_range_evidence() {
    for body in [
        "pure fn bad(bits:Bits,i:Byte)->.BitVec'builtin<1> {bits[i]}",
        "pure fn bad(i:Byte)->Word {var word:Word=0;word'bitvec[i]=1;word}",
        "pure fn bad(i:Byte)->Word where requires i <= 13 {var word:Word=0;word'bitvec[i..(i + 4)]=0;word}",
    ] {
        let error =
            compile_with_protocols(&format!("{TYPES}{body}"), &Default::default()).unwrap_err();
        assert!(
            !error.contains("P4 evidence"),
            "purity should pass before the range rejection: {error}"
        );
        assert!(
            error.contains("bound") || error.contains("range"),
            "{error}"
        );
    }
}

#[test]
fn early_returns_from_positions_and_payloads_skip_writeback() {
    run(
        "pure fn position()->Word {var word:Word=0;word'bitvec[do {return 7;}]=1;word}
         pure fn payload()->Word {var word:Word=0;let i:Byte=4;word'bitvec[i]=do {return 9;};word}
         pure fn read()->Word {let bits:Bits=0;bits[do {return 11;}];0}",
        &[("position", 7), ("payload", 9), ("read", 11)],
    );
}

#[test]
fn pure_loops_build_bits_without_claiming_finite_default_evidence() {
    let source = "pure fn build()->Word {var word:Word=0;var i:Byte=0;while i < 16 {word'bitvec[i]=1;i=i + 1;}word}";
    run(source, &[("build", 65535)]);
    let error = compile_with_protocols(
        &format!("{TYPES}{source}fn take(.word:Word=build())->Word {{word}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn owned_fields_and_captured_bit_storage_use_resource_checked_execution() {
    let source=format!("{TYPES}trait Copy {{}} trait Drop {{fn drop(*self);}}
        struct Packet {{word:Word,other:Word}}
        pure fn field()->Word {{let packet=Packet {{word:0x1234,other:7}};let i:Byte=4;
            packet.word'bitvec[i..(i + 4)]=0b1010;packet.word}}
        fn apply(action:%{{packet:Packet}} fn()->Word)->Word {{action()}}
        pure fn captured()->Word {{let packet=Packet {{word:0,other:7}};
            apply(|| do {{let i:Byte=4;packet.word'bitvec[i]=1;packet.word}})}}
        pure fn snapshots()->Word {{var word:Word=0;let first=|| do {{let i:Byte=4;word'bitvec[i]=1;word}};
            let second=|| word;let changed=first();let original=second();if original == 0 {{changed}} else {{original}}}}");
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"),
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let package =
        execution_support::compile_with_source_bindings(&source, &Default::default(), selectors)
            .unwrap();
    for (entry, expected) in [("field", 0x12A4), ("captured", 16), ("snapshots", 16)] {
        assert_eq!(
            luna::execution::interpret_with_protocols_in_package(
                &package, 0, entry, "Copy", "Drop", 100_000
            )
            .unwrap(),
            Value::UnsignedInteger(expected)
        );
    }
}

#[test]
fn selection_callbacks_require_known_behavior_for_each_candidate() {
    run("fn low()->result:Byte where ensures result == 4 {4}
         fn edit(position:fn()->Byte)->Word {let i=position();if i < 16 {var word:Word=0;word'bitvec[i]=1;word} else {0}}
         pure fn run()->Word {edit(low)}",&[("run",16)]);
    let source = format!(
        "{TYPES}pure fn bad(position:fn()->Byte)->Word {{var word:Word=0;word'bitvec[position()]=1;word}}"
    );
    let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn value_snapshot_precedes_mutating_window_endpoints() {
    run(
        "pure fn run()->Word {var bits:Bits=0xABCD;let lo:Byte=4;
         let part:Nibble=bits[lo..do {bits=0;let hi:Byte=8;hi}];part.as(Word)}",
        &[("run", 12)],
    );
}

#[test]
fn pure_admission_does_not_remove_call_preconditions() {
    let source=format!("{TYPES}pure fn select(bits:Bits,i:Byte)->Nibble where requires i <= 12 {{bits[i..(i + 4)]}}
        pure fn bad()->Nibble {{select(0,13)}}");
    let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(error.contains("precondition"), "{error}");
}
