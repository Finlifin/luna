//! Bit computations share P4 evidence with finite suppliers; range and affine
//! obligations still pass through the ordinary checked execution pipeline.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{calls, compile, compile_with_protocols, id};

const TYPES: &str = "typealias Word=.u16'builtin;typealias Byte=.u8'builtin;typealias Bits=.BitVec'builtin<16>;typealias Nibble=.BitVec'builtin<4>;";

#[test]
fn abstract_bitvec_widths_report_a_diagnostic_instead_of_panicking() {
    for source in [
        "trait Marker {} impl Marker for .BitVec'builtin<N> where N:.Integer'builtin {}",
        "struct Packet where N:.Integer'builtin {bits:.BitVec'builtin<N>}",
    ] {
        let failure = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            failure.contains("BitVec width is still an abstract comptime value"),
            "{failure}"
        );
    }
    // A failed declaration must not poison later concrete type formation.
    let package = compile("fn run()->bool {let bits:.BitVec'builtin<1024> = not 0;bits == not 0}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn deferred_bitvec_widths_form_concrete_instances_at_the_call_site() {
    let definition = "fn make()->defer .BitVec'builtin<N> where N:.Integer'builtin {0}
        fn ordinary(comptime width:.Integer'builtin)->defer .BitVec'builtin<width> {0}";
    let package = compile(&format!(
        "{definition} fn run()->bool {{
            let empty=make<0>();
            let bit=make<1>();
            let wide=make<1024>();
            empty == 0 and bit == 0 and wide == 0 and ordinary(65) == 0
        }}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "make<-1>()",
        "make<true>()",
        "let bits:.BitVec'builtin<8> = make<9>();bits",
    ] {
        assert!(
            compile_with_protocols(
                &format!("{definition} fn bad() {{{body};}}"),
                &Default::default()
            )
            .is_err(),
            "{body}"
        );
    }
}

#[test]
fn contextual_literal_inversion_waits_for_the_completed_bitvec_domain() {
    let definition = "fn ones()->defer .BitVec'builtin<N> where N:.Integer'builtin {not 0}
        fn twice()->defer .BitVec'builtin<N> where N:.Integer'builtin {not not 1}
        fn mask()->defer .BitVec'builtin<N> where N:.Integer'builtin {not 256}";
    let package = compile(&format!(
        "{definition}
        fn run()->bool {{ones<0>() == 0 and ones<1>() == 1 and ones<65>() == not 0
            and ones<1024>() == not 0 and twice<8>() == 1 and mask<9>() == 255}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for source in [
        format!("{definition} fn bad() {{mask<8>();}}"),
        "comptime fn Pick(flag:bool)->.Type'builtin {if flag {bool} else {.BitVec'builtin<8>}}
         fn invert()->defer Pick(flag) where flag:bool {not 0}
         fn bad()->bool {invert<true>()}".into(),
        "fn bad(value:defer .BitVec'builtin<N>)->defer .BitVec'builtin<N> where N:.Integer'builtin {not value}".into(),
        "fn bad()->defer .BitVec'builtin<N> where N:.Integer'builtin {not 1.5}".into(),
        "fn bad()->defer .BitVec'builtin<N> where N:.Integer'builtin {let unused=not 0;0}".into(),
    ] {
        assert!(compile_with_protocols(&source, &Default::default()).is_err(), "{source}");
    }
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn deferred_structural_types_preserve_nested_bitvec_computations() {
    let definition = "fn ones()->defer .BitVec'builtin<N> where N:.Integer'builtin {not 0}
        fn relay(comptime width:.Integer'builtin, callback:defer (fn()->.BitVec'builtin<width>))
            ->defer (fn()->.BitVec'builtin<width>) {callback}
        fn forward(comptime width:.Integer'builtin, pointer:defer *.BitVec'builtin<width>)
            ->defer *.BitVec'builtin<width> {pointer}";
    let package = compile(&format!(
        "{definition}
        fn run()->bool {{
            let callback=relay(65,ones<65>);
            let nested=|| callback();
            let empty=relay(0,ones<0>);
            let bits:.BitVec'builtin<65> = 42;
            let pointer=forward(65,bits.ref);
            callback() == not 0 and nested() == not 0 and empty() == 0 and pointer.* == 42
        }}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "relay(8,ones<9>);",
        "relay(-1,ones<8>);",
        "let bits:.BitVec'builtin<8> = 0;forward(9,bits.ref);",
    ] {
        assert!(
            compile_with_protocols(
                &format!("{definition} fn bad() {{{body}}}"),
                &Default::default()
            )
            .is_err(),
            "{body}"
        );
    }
}

#[test]
fn deferred_optional_payloads_use_native_type_and_check_width() {
    let definitions =
        "fn forward(comptime width:.Integer'builtin,value:defer ?.BitVec'builtin<width>)
            ->defer ?.BitVec'builtin<width> {value}";
    let source = format!(
        "{definitions} fn run()->bool {{
        let first=forward(2,3);
        let empty=forward(0,null);
        (first match {{bits?=>bits == 3,null=>false}})
            and (empty match {{_?=>false,null=>true}})
    }}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        ..Default::default()
    };
    let package =
        execution_support::compile_with_source_bindings(&source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let invalid = format!("{definitions} fn bad() {{forward(2,4);}}");
    assert!(
        execution_support::compile_with_source_bindings(&invalid, &Default::default(), selectors)
            .is_err()
    );
}

#[test]
fn abstract_width_factories_form_callable_values_after_specialization() {
    let definition = "fn ones()->defer .BitVec'builtin<N> where N:.Integer'builtin {not 0}
        fn factory()->defer (fn()->.BitVec'builtin<N>) where N:.Integer'builtin {ones<N>}";
    let package = compile(&format!(
        "{definition} fn run()->bool {{
        let empty=factory<0>();let small=factory<8>();let large=factory<1024>();
        let captured=|| large();
        empty() == 0 and small() == 255 and captured() == not 0
    }}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for source in [
        format!("{definition} fn bad() {{factory<-1>();}}"),
        format!("{definition} fn bad() {{let callback:fn()->.BitVec'builtin<9> = factory<8>();}}"),
        "fn target()->defer .BitVec'builtin<N> where N:.Integer'builtin {0}
         fn bad() where flag:bool {target<flag>; }"
            .into(),
        "fn target(.value:bool=false)->defer .BitVec'builtin<N> where N:.Integer'builtin {0}
         fn bad() where N:.Integer'builtin {target<N>; }"
            .into(),
        "fn target(comptime value:bool)->defer .BitVec'builtin<N> where N:.Integer'builtin {0}
         fn bad() where N:.Integer'builtin {target<N>; }"
            .into(),
        "unsafe fn target()->defer .BitVec'builtin<N> where N:.Integer'builtin {0}
         fn factory()->defer (fn()->.BitVec'builtin<N>) where N:.Integer'builtin {target<N>}
         fn bad() {factory<8>();}"
            .into(),
        "fn target(gate:bool)->defer .BitVec'builtin<N> where N:.Integer'builtin, requires gate {0}
         fn factory()->defer (fn(bool)->.BitVec'builtin<N>) where N:.Integer'builtin {target<N>}
         fn bad() {factory<8>();}"
            .into(),
    ] {
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn ordinary_comptime_width_factories_preserve_input_slots_and_defaults() {
    let definitions = "fn ones()->defer .BitVec'builtin<N> where N:.Integer'builtin {not 0}
        fn factory(comptime width:.Integer'builtin)->defer (fn()->.BitVec'builtin<width>) {ones<width>}
        fn forwarded(comptime width:.Integer'builtin)->defer (fn()->.BitVec'builtin<width>) {factory(width)}
        fn defaulted(comptime .width:.Integer'builtin=8)->defer (fn()->.BitVec'builtin<width>) {ones<width>}
        comptime fn failing()->.Integer'builtin {assumes false;8}
        fn guarded(comptime .width:.Integer'builtin=failing())->defer (fn()->.BitVec'builtin<width>) {ones<width>}";
    let package = compile(&format!(
        "{definitions} fn run()->bool {{
        let empty=factory(0);let wide=forwarded(1024);let fallback=defaulted();
        let explicit=defaulted(.width=65);let skipped=guarded(.width=1);
        let captured=|| wide();
        empty() == 0 and captured() == not 0 and fallback() == 255
            and explicit() == not 0 and skipped() == 1
    }}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "factory(-1);",
        "factory(true);",
        "guarded();",
        "let callback:fn()->.BitVec'builtin<8> = factory(9);",
    ] {
        assert!(
            compile_with_protocols(
                &format!("{definitions} fn bad() {{{body}}}"),
                &Default::default()
            )
            .is_err(),
            "{body}"
        );
    }
    assert!(
        compile_with_protocols(
            &format!("{definitions} fn bad(flag:bool) {{factory(if flag {{8}} else {{9}});}}"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn computed_clause_inputs_wait_for_factory_specialization() {
    let definitions = "fn ones()->defer .BitVec'builtin<N> where N:.Integer'builtin {not 0}
        fn named()->defer .BitVec'builtin<N> where .N:.Integer'builtin=8 {not 0}
        comptime fn increment(width:.Integer'builtin)->.Integer'builtin {width + 1}
        fn factory(comptime width:.Integer'builtin)->defer (fn()->.BitVec'builtin<width + 1>) {ones<width + 1>}
        fn through_call(comptime width:.Integer'builtin)->defer (fn()->.BitVec'builtin<width + 1>) {named<.N=increment(width)>}
        fn clause_factory()->defer (fn()->.BitVec'builtin<N + 1>) where N:.Integer'builtin {ones<N + 1>}";
    let package = compile(&format!("{definitions} fn run()->bool {{
        let first=factory(0);let second=factory(64);let third=through_call(7);let fourth=clause_factory<1023>();
        first() == 1 and second() == not 0 and third() == 255 and fourth() == not 0
    }}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "fn bad(comptime width:.Integer'builtin) {ones<width + true>;}",
        "fn bad(comptime width:.Integer'builtin) {ones<width + 1,0>;}",
        "fn bad(comptime width:.Integer'builtin) {named<.N=width + 1,.N=1>;}",
        "fn bad(flag:bool) {ones<if flag {8} else {9}>;}",
        "fn bad() {factory(-2);}",
        "fn bad() {let callback:fn()->.BitVec'builtin<8> = factory(8);}",
    ] {
        assert!(
            compile_with_protocols(&format!("{definitions} {body}"), &Default::default()).is_err(),
            "{body}"
        );
    }
}

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
fn leading_bit_masks_use_peer_context_without_short_circuiting_or_reordering() {
    let package = compile(&format!(
        "{TYPES}
        fn operand()->Bits {{0xABCD}}
        fn left()->Bits {{0}}
        fn right()->Bits {{0xFFFF}}
        fn masked()->Word {{(0xFF00 and operand()).as(Word)}}
        fn forced()->Word {{(0xFFFF or operand()).as(Word)}}
        fn ordered()->Word {{(left() and right()).as(Word)}}"
    ));
    for (entry, expected, order) in [
        ("masked", 0xAB00, vec!["masked", "operand"]),
        ("forced", 0xFFFF, vec!["forced", "operand"]),
        ("ordered", 0, vec!["ordered", "left", "right"]),
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
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
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

#[test]
fn guard_bit_helpers_still_require_machine_range_proofs() {
    for body in [
        "let bits:Bits=0;let selected=bits[i];p.*",
        "var packet=Packet {word:0};packet.word'bitvec[i]=1;p.*",
        "var packet=Packet {word:0};packet.word'bitvec[i..(i + 4)]=0;p.*",
    ] {
        for inline in ["", "inline "] {
            let source = format!(
                "{TYPES}struct Packet {{word:Word}}{inline}fn observe(i:Byte,p:*bool)->bool {{{body}}}fn entry(i:Byte)->bool {{let flag=true;false match {{_ if observe(i,flag.ref)=>true,_=>false}}}}"
            );
            let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
            assert!(
                error.contains("bound") || error.contains("range"),
                "{inline}{body}: {error}"
            );
        }
    }
}

#[test]
fn guard_bit_range_assumptions_keep_runtime_rejection_obligations() {
    for body in [
        "assumes i < 16;let bits:Bits=0;let selected=bits[i];p.*",
        "assumes i < 16;var word:Word=0;word'bitvec[i]=1;p.*",
    ] {
        for index in [4, 20] {
            let package = compile(&format!(
                "{TYPES}fn observe(i:Byte,p:*bool)->bool {{{body}}}fn entry()->bool {{let flag=true;false match {{_ if observe({index},flag.ref)=>true,_=>false}}}}"
            ));
            let result = luna::execution::interpret(&package, "entry", 100_000);
            if index == 4 {
                assert_eq!(result.unwrap(), Value::Bool(true));
            } else {
                let error = result.unwrap_err().to_string();
                assert!(error.contains("ProgramContractRejected"), "{body}: {error}");
            }
        }
    }
}

#[test]
fn field_writeback_preserves_rhs_object_replacement_and_siblings() {
    for (selection, payload, expected) in [
        ("4..8", "0b1010", 0xABAD),
        ("i..(i + 4)", "0b1010", 0xABAD),
        ("i..i", "0", 0xABCD),
    ] {
        let source = format!(
            "{TYPES}trait Copy {{}}trait Drop {{fn drop(*self);}}struct Packet {{word:Word,other:Word}}inline fn transform()->Word {{var packet=Packet {{word:0x1234,other:7}};var i:Byte=4;packet.word'bitvec[{selection}]=do {{i=10;packet=Packet {{word:0xABCD,other:9}};{payload}}};if packet.other == 9 {{packet.word}} else {{0}}}}fn entry()->Word {{transform()}}"
        );
        let original = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("input.Copy"),
                drop: Some("input.Drop"),
                ..Default::default()
            },
        )
        .unwrap();
        for expanded in [false, true] {
            let mut package = original.clone();
            if expanded {
                assert!(
                    elaboration::program::inlining::expand(&mut package, &mut 1_000_000)
                        .unwrap()
                        .expanded
                        > 0
                );
            }
            assert_eq!(
                luna::execution::interpret_with_protocols_in_package(
                    &package, 0, "entry", "Copy", "Drop", 1_000_000
                )
                .unwrap_or_else(|e| panic!("{selection}/expanded={expanded}: {e}")),
                Value::UnsignedInteger(expected)
            );
        }
    }
}

#[test]
fn diverging_logical_prefix_uses_the_remaining_operand_domain() {
    for operator in ["and", "or"] {
        for right in ["7", "do {7}", "bits()"] {
            let source = format!("{TYPES}
                fn bits()->Bits {{7}}
                fn run()->bool {{let ignored:Bits=(do {{return true;}}) {operator} ({right}); false}}");
            let package = compile(&source);
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(id(&package, "run"), vec![]).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(calls(&package, &vm), vec!["run"]);
            assert_eq!(
                luna::execution::interpret(&package, "run", 100_000).unwrap(),
                Value::Bool(true)
            );
            let ir = luna::host::llvm_ir(&package, "run", 100_000).unwrap();
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
}

#[test]
fn diverging_logical_chains_and_boolean_short_circuit_preserve_control() {
    for operator in ["and", "or"] {
        for body in [
            format!("(do {{return true;}}) {operator} bits()"),
            format!(
                "let ignored:Bits=(do {{return true;}}) {operator} bits() {operator} bits(); false"
            ),
            format!(
                "let ignored:Bits=(do {{return true;}}) {operator} (do {{return false;}}); false"
            ),
            format!("let ignored:bool=(do {{return true;}}) {operator} true; false"),
            format!(
                "let ignored:bool={} {operator} (do {{return false;}}); true",
                if operator == "and" { "false" } else { "true" }
            ),
        ] {
            let package = compile(&format!(
                "{TYPES} fn bits()->Bits {{7}} fn run()->bool {{{body}}}"
            ));
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(id(&package, "run"), vec![]).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(calls(&package, &vm), vec!["run"]);
            assert_eq!(
                luna::execution::interpret(&package, "run", 100_000).unwrap(),
                Value::Bool(true)
            );
            let ir = luna::host::llvm_ir(&package, "run", 100_000).unwrap();
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
}

#[test]
fn diverging_left_operand_does_not_admit_an_unrelated_logical_domain() {
    for operator in ["and", "or"] {
        let source = format!(
            "{TYPES} fn wrong()->Word {{7}} fn run()->bool {{(do {{return true;}}) {operator} wrong()}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("and/or require bool or equal-width BitVec operands"),
            "{error}"
        );
        for left in ["true", "false"] {
            let source = format!(
                "{TYPES} fn bits()->Bits {{7}} fn run()->bool {{{left} {operator} bits()}}"
            );
            assert!(compile_with_protocols(&source, &Default::default()).is_err());
        }
    }
}
