#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use support::{compile_with_protocols, calls};
use comptime_interpretor::{Budget,Interpreter,Value};

const INSTRUCTIONS: &str = "typealias u32=.u32'builtin;typealias Bits5=.BitVec'builtin<5>;
    enum Instruction {
        .base_type u32,.width 32,.discrete_tag true,
        add:0b0000000 (rs2:Bits5) (rs1:Bits5) 000 (rd:Bits5) 0110011,
        sub:0b0100000 (rs2:Bits5) (rs1:Bits5) 000 (rd:Bits5) 0110011,
}";

#[test]
fn symbol_enum_encodings_preserve_groups_and_nominal_instances() {
    let source="typealias Symbol=.Symbol'builtin;
        comptime fn wire_ready()->Symbol {.wire_ready}
        enum Message where T {.base_type Symbol,.discrete_tag true,
            client.{ready=wire_ready(),waiting:.wire_waiting},closed=.wire_closed,}
        fn accepted(value:Message<T>)->bool where T {value match {.client=>true,.closed=>false}}
        fn run()->bool {accepted(Message<bool>.client.ready) and not accepted(Message<bool>.closed)}";
    let package=compile_with_protocols(source,&Default::default()).unwrap();
    let definition=&package.enums[0];
    assert!(definition.encoding.is_none());
    let symbols=definition.symbol_encoding.as_ref().unwrap();
    assert_eq!(symbols.encode(0),Some("wire_ready"));
    assert_eq!(symbols.encode(1),Some("wire_waiting"));
    assert_eq!(symbols.discriminate("wire_closed",&mut Default::default()).unwrap(),2);
    assert!(symbols.discriminate("missing",&mut Default::default()).is_err());
    assert!(definition.to_string().contains("(symbol-encoding"));
    let graph=luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let mut vm=Interpreter::from_iir(&graph,Budget::default()).unwrap();
    assert_eq!(vm.invoke_instance(&graph.roots[0],vec![]).unwrap(),Value::Bool(true));
    let staged=compile_with_protocols(&source.replace("fn accepted", "comptime fn accepted").replace("fn run", "comptime fn run"),&Default::default()).unwrap();
    assert_eq!(luna::invocation::invoke(&staged,"run",100_000).unwrap(),Value::Bool(true));
}

#[test]
fn symbol_enum_encodings_reject_overlap_wrong_domains_and_forged_metadata() {
    let prefix="typealias Symbol=.Symbol'builtin;";
    for body in [
        ".base_type Symbol,.discrete_tag true,first=.same,second=.same",
        ".base_type Symbol,.discrete_tag true,.width 8,first=.first",
        ".base_type Symbol,.discrete_tag true,first=7",
        ".base_type Symbol,.discrete_tag true,first=\"first\"",
        ".base_type Symbol,.discrete_tag true,first=.first,second",
        ".base_type Symbol,.discrete_tag true,first(bool)",
    ] {
        let source=format!("{prefix} enum Broken {{{body}}}");
        assert!(compile_with_protocols(&source,&Default::default()).is_err(),"{source}");
    }
    let mut package=compile_with_protocols(&format!("{prefix} enum Message {{.base_type Symbol,.discrete_tag true,ready=.wire_ready}} fn run()->Message {{Message.ready}}"),&Default::default()).unwrap();
    package.enums[0].symbol_encoding=Some(nir::enum_encoding::SymbolRepresentation::new(vec![
        nir::enum_encoding::SymbolVariant {name:"wrong".into(),value:"wire_ready".into()},
    ],&mut Default::default()).unwrap());
    assert!(package.enums[0].validate_encoding().is_err());
    assert!(Interpreter::new(&package,Budget::default()).is_err());
}

#[test]
fn instruction_constructors_use_the_declared_whole_encoding() {
    let source = format!("{INSTRUCTIONS}
        fn rs1()->Bits5 {{2}} fn rs2()->Bits5 {{3}} fn rd()->Bits5 {{1}}
        fn add()->u32 {{let code=Instruction.add {{rd:rd(),rs1:rs1(),rs2:rs2()}};(code'bitvec).as(u32)}}
        fn sub()->u32 {{let code=Instruction.sub {{rs2:3,rs1:2,rd:1}};(code'bitvec).as(u32)}}
        fn destination(code:Instruction)->u32 {{code match {{.add {{rd:rd}} => rd.as(u32),.sub {{rd:rd}} => rd.as(u32)}}}}
        fn run()->u32 {{destination(Instruction.sub {{rd:7,rs2:3,rs1:2}})}}");
    let package = compile_with_protocols(&source,&Default::default()).unwrap();
    assert!(package.to_string().contains("(enum-encoding (width 32)"));
    assert_eq!(package.enums[0].encoding.as_ref().unwrap().base_type,Some(nir::Primitive::U32));
    for (entry,expected) in [("add",0x003100b3),("sub",0x403100b3),("run",7)] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),Value::UnsignedInteger(expected));
    }
    let graph = luna::instantiation::instantiate(&package,"add",100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph,Budget::default()).unwrap();
    vm.invoke_instance(&graph.roots[0],vec![]).unwrap();
    assert_eq!(calls(&package,&vm),vec!["add","rs2","rs1","rd"]);
    let staged = compile_with_protocols(&source.replace("fn ","comptime fn "),&Default::default()).unwrap();
    assert_eq!(luna::invocation::invoke(&staged,"add",100_000).unwrap(),Value::UnsignedInteger(0x003100b3));
}

#[test]
fn invalid_encodings_fail_instead_of_acquiring_an_extra_tag() {
    for (source, diagnostic) in [
        (INSTRUCTIONS.replace("0b0100000","0b0000000"),"overlap"),
        (INSTRUCTIONS.replace(".width 32",".width 33"),"conflicts"),
        (INSTRUCTIONS.replace("(rd:Bits5)","(rd:u32)"),"pattern width"),
        (INSTRUCTIONS.replace("(rd:Bits5)","(rs1:Bits5)"),"duplicate name"),
        (INSTRUCTIONS.replace(".base_type u32",".base_type .i32'builtin"),"unsigned integer"),
        (INSTRUCTIONS.replace(".width 32",".width 0"),"positive"),
        (INSTRUCTIONS.replace("(rs2:Bits5)","(_:Bits5)"),"named unconstrained binding"),
    ] {
        let failure = compile_with_protocols(&source,&Default::default()).err().expect("invalid layout accepted");
        assert!(failure.contains(diagnostic),"{failure}");
    }
    let bad_payload = format!("{INSTRUCTIONS} fn bad()->Instruction {{Instruction.add {{rd:32,rs1:0,rs2:0}}}}");
    assert!(compile_with_protocols(&bad_payload,&Default::default()).is_err());
    let missing = format!("{INSTRUCTIONS} fn bad()->Instruction {{Instruction.add {{rd:1,rs1:0}}}}");
    let error = compile_with_protocols(&missing,&Default::default()).err().unwrap();
    assert!(error.contains("missing required enum payload field"),"{error}");
}

#[test]
fn width_only_encodings_preserve_wide_payloads_and_generic_instances() {
    let source = "typealias Wide=.BitVec'builtin<256>;typealias Byte=.u8'builtin;
        enum Packet where T {.width 257,.discrete_tag true,
            low:0b0 (payload:Wide), high:0b1 (payload:Wide),}
        fn bits(packet:Packet<T>)->.BitVec'builtin<257> where T {packet'bitvec}
        fn run()->bool {let value=bits(Packet<bool>.high {payload:3});
            value[256].as(Byte) == 1 and value[0..256] == 3}";
    let package = compile_with_protocols(source,&Default::default()).unwrap();
    assert_eq!(package.enums[0].encoding.as_ref().unwrap().base_type,None);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    let staged = compile_with_protocols(&source.replace("fn ","comptime fn "),&Default::default()).unwrap();
    assert_eq!(luna::invocation::invoke(&staged,"run",100_000).unwrap(),Value::Bool(true));
}

#[test]
fn computed_configurations_and_segments_form_one_stable_representation() {
    let source = "typealias Byte=.u8'builtin;
        comptime fn width()->.Integer'builtin {4 + 4}
        comptime fn prefix()->.BitVec'builtin<8> {0xa5}
        enum Code {.base_type Byte,.width width(),.discrete_tag true,
            selected:0b<prefix()>,}
        comptime fn encoding()->Byte {(Code.selected'bitvec).as(Byte)}
        const ENCODED=encoding();
        comptime fn run()->Byte {ENCODED}";
    let package = compile_with_protocols(source,&Default::default()).unwrap();
    assert_eq!(luna::invocation::invoke(&package,"run",100_000).unwrap(),Value::UnsignedInteger(0xa5));
}

#[test]
fn modified_encoding_metadata_is_rejected_at_nir_and_iir_boundaries() {
    let source = format!("{INSTRUCTIONS} fn run()->u32 {{let code=Instruction.add {{rs2:3,rs1:2,rd:1}};(code'bitvec).as(u32)}}");
    let package = compile_with_protocols(&source,&Default::default()).unwrap();
    let mut wrong_type = package.clone();
    wrong_type.enums[0].variants[0].fields[0] = nir::Term::Primitive(nir::Primitive::U8);
    assert!(nir::phases::validate(&wrong_type).is_err());
    assert!(Interpreter::new(&wrong_type,Budget::default()).is_err());
    let mut wrong_base = package.clone();
    wrong_base.enums[0].encoding.as_mut().unwrap().base_type = Some(nir::Primitive::U64);
    assert!(nir::phases::validate(&wrong_base).is_err());
    let mut graph = luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    for nominal in &mut graph.nominals {
        if let iir::NominalDefinition::Enum(definition) = &mut nominal.definition {
            definition.variants[0].name = "changed".into();
        }
    }
    assert!(Interpreter::from_iir(&graph,Budget::default()).is_err());
}

#[test]
fn early_exit_in_first_declared_payload_skips_remaining_producers() {
    let source = format!("{INSTRUCTIONS} fn later()->Bits5 {{3}}
        fn run()->bool {{let code=Instruction.add {{rd:later(),rs1:later(),rs2:do {{return true;}}}};false}}");
    let package = compile_with_protocols(&source,&Default::default()).unwrap();
    let graph = luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph,Budget::default()).unwrap();
    assert_eq!(vm.invoke_instance(&graph.roots[0],vec![]).unwrap(),Value::Bool(true));
    assert_eq!(calls(&package,&vm),vec!["run"]);
}

#[test]
fn scalar_payloads_preserve_bits_instead_of_numeric_casting() {
    let source = "typealias Byte=.i8'builtin;typealias Word=.u16'builtin;
        enum Signed {.base_type Word,.discrete_tag true,number:0xA (value:Byte) F,}
        fn signed()->Word {let value=Signed.number {value: -1};(value'bitvec).as(Word)}
        enum Float {.width 33,.discrete_tag true,number:0b1 (value:.f32'builtin),}
        fn float()->.u32'builtin {let value=Float.number {value:1.0};(value'bitvec)[0..32].as(.u32'builtin)}";
    let package = compile_with_protocols(source,&Default::default()).unwrap();
    assert_eq!(luna::execution::interpret(&package,"signed",100_000).unwrap(),Value::UnsignedInteger(0xafff));
    assert_eq!(luna::execution::interpret(&package,"float",100_000).unwrap(),Value::UnsignedInteger(0x3f800000));
}

#[test]
fn documented_development_fixture_executes() {
    let package = compile_with_protocols(include_str!("../../../development/examples/fixed-enum-encoding.fl"),&Default::default()).unwrap();
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
}

#[test]
fn expression_encodings_use_the_base_domain_and_keep_variant_identity() {
    let source = "typealias Byte=.u8'builtin;
        comptime fn ready_code()->Byte {40 + 2}
        enum Status {.base_type Byte,.discrete_tag true,ready=ready_code(),busy=7,}
        fn code(value:Status)->Byte {(value'bitvec).as(Byte)}
        fn is_ready(value:Status)->bool {value match {.ready=>true,.busy=>false}}
        fn run()->bool {code(Status.ready) == 42 and code(Status.busy) == 7
            and is_ready(.ready) and not is_ready(.busy)}";
    let package = compile_with_protocols(source,&Default::default()).unwrap();
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    let staged_source = source.replace("fn code", "comptime fn code")
        .replace("fn is_ready", "comptime fn is_ready").replace("fn run", "comptime fn run");
    let staged = compile_with_protocols(&staged_source,&Default::default()).unwrap();
    assert_eq!(luna::invocation::invoke(&staged,"run",100_000).unwrap(),Value::Bool(true));
    for invalid in [source.replace("busy=7", "busy=42"), source.replace("busy=7", "busy=256"),
        source.replace("busy=7", "busy= -1"), source.replace("busy=7", "busy=true"),
        source.replace("busy=7", "busy=bool"), source.replace("busy=7", "busy=.unbound"),
        source.replace("comptime fn ready_code", "fn ready_code"),
        source.replace("busy=7", "busy")]
    {
        assert!(compile_with_protocols(&invalid,&Default::default()).is_err(),"accepted {invalid}");
    }
}

#[test]
fn expression_and_payload_patterns_share_the_same_disjointness_check() {
    let source = "typealias Byte=.u8'builtin;typealias Nibble=.BitVec'builtin<4>;
        enum Packet {.base_type Byte,.discrete_tag true,empty=0,data:0xA (payload:Nibble),}
        fn code(value:Packet)->Byte {(value'bitvec).as(Byte)}
        fn run()->bool {code(Packet.empty)==0 and code(Packet.data {payload:5})==0xa5}";
    let package = compile_with_protocols(source,&Default::default()).unwrap();
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    let overlap = source.replace("empty=0", "empty=0xa5");
    let error = compile_with_protocols(&overlap,&Default::default()).err().unwrap();
    assert!(error.contains("overlap"),"{error}");
}

#[test]
fn width_only_expression_encodings_are_bitvec_values_including_wide_values() {
    let source = "typealias Bits=.BitVec'builtin<257>;
        comptime fn high()->Bits {let value:Bits=0;not value}
        enum Mask where T {.width 257,.discrete_tag true,empty=0,full=high(),}
        fn view(value:Mask<bool>)->Bits {value'bitvec}
        fn run()->bool {let value:Bits=0;view(Mask<bool>.full)==not value and view(.empty)==value}";
    let package = compile_with_protocols(source,&Default::default()).unwrap();
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    for invalid in [source.replace("empty=0", "empty=true"),
        source.replace("->Bits {let value:Bits=0;not value}", "->.BitVec'builtin<256> {0}")]
    {
        assert!(compile_with_protocols(&invalid,&Default::default()).is_err(),"accepted {invalid}");
    }
}

#[test]
fn equal_codes_in_different_enums_do_not_make_the_nominal_types_equal() {
    let source = "typealias Byte=.u8'builtin;
        enum A {.base_type Byte,.discrete_tag true,ready=1,}
        enum B {.base_type Byte,.discrete_tag true,ready=1,}
        fn accepts(value:A)->bool {value match {.ready=>true}}
        fn run()->bool {accepts(A.ready) and A.ready'bitvec == B.ready'bitvec}";
    let package = compile_with_protocols(source,&Default::default()).unwrap();
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    let invalid = source.replace("accepts(A.ready)", "accepts(B.ready)");
    assert!(compile_with_protocols(&invalid,&Default::default()).is_err());
    let raw_conversion = format!("{source} fn convert(raw:Byte)->A {{raw.as(A)}}");
    assert!(compile_with_protocols(&raw_conversion,&Default::default()).is_err());
}
