//! Bit-value bytecode preserves widths, boundaries and expression ordering.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

fn bits(width: usize, word: u32) -> Value {
    Value::BitVec(
        comptime_runtime::bitvec::BitVec::from_words(
            width,
            if width == 0 {
                &[]
            } else {
                std::slice::from_ref(&word)
            },
            &mut Default::default(),
        )
        .unwrap(),
    )
}

#[test]
fn bit_selections_and_concatenation_use_bytecode_with_checked_boundaries() {
    let package = support::compile(
        "typealias Bits=.BitVec'builtin<8>;
        typealias Part=.BitVec'builtin<4>;
        typealias Bit=.BitVec'builtin<1>;
        typealias Empty=.BitVec'builtin<0>;
        typealias Wide=.BitVec'builtin<512>;
        typealias Integer=.Integer'builtin;
        comptime fn slice(value:Bits)->Part {value[2..6]}
        comptime fn index(value:Bits,position:Integer)->Bit {value[position]}
        comptime fn window(value:Bits,start:Integer)->Part {value[start..(start + 4)]}
        comptime fn empty(value:Bits,start:Integer)->Empty {value[start..start]}
        comptime fn join(high:Part,low:Part)->Bits {high ++ low}
        comptime fn join_empty(high:Empty,low:Empty)->Empty {high ++ low}
        comptime fn empty_slice(value:Bits)->Empty {value[8..8]}
        comptime fn tail(value:Wide)->Part {value[508..512]}",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    let integer = |value| Value::Integer(comptime_runtime::Integer::from_i128(value));
    let mut words = vec![0; 16];
    words[15] = 0xA0000000;
    let wide = Value::BitVec(
        comptime_runtime::bitvec::BitVec::from_words(512, &words, &mut Default::default()).unwrap(),
    );
    for (name, arguments, expected) in [
        ("slice", vec![bits(8, 0b10110100)], bits(4, 13)),
        ("index", vec![bits(8, 0b10110100), integer(2)], bits(1, 1)),
        ("window", vec![bits(8, 0b10110100), integer(4)], bits(4, 11)),
        ("empty", vec![bits(8, 0b10110100), integer(8)], bits(0, 0)),
        ("join", vec![bits(4, 10), bits(4, 5)], bits(8, 165)),
        ("join_empty", vec![bits(0, 0), bits(0, 0)], bits(0, 0)),
        ("empty_slice", vec![bits(8, 255)], bits(0, 0)),
        ("tail", vec![wide], bits(4, 10)),
    ] {
        let before = vm.bytecode_invocations();
        let id = support::id(&package, name);
        assert_eq!(vm.invoke(id, arguments.clone()).unwrap(), expected);
        assert_eq!(vm.bytecode_invocations() - before, 1, "{name}");
        assert_eq!(tree.invoke(id, arguments).unwrap(), expected);
    }
    for (name, positions) in [
        ("index", vec![-1, 8, 0]),
        ("window", vec![-1, 5, 0]),
        ("empty", vec![-1, 9, 8]),
    ] {
        for (trial, position) in positions.into_iter().enumerate() {
            let arguments = vec![bits(8, 0b10110100), integer(position)];
            let id = support::id(&package, name);
            let result = vm.invoke(id, arguments.clone());
            let reference = tree.invoke(id, arguments);
            if trial < 2 {
                assert_eq!(result.unwrap_err().kind, reference.unwrap_err().kind);
            } else {
                assert_eq!(result.unwrap(), reference.unwrap());
            }
        }
    }
}

#[test]
fn bit_conversions_preserve_encodings_and_equality_uses_bytecode() {
    let package = support::compile(
        "typealias Bits=.BitVec'builtin<8>;
        typealias Wide=.BitVec'builtin<32>;
        typealias Signed=.i8'builtin;
        typealias Unsigned=.u32'builtin;
        typealias Float=.f32'builtin;
        comptime fn widen(value:Bits)->Wide {value.as(Wide)}
        comptime fn unsigned(value:Bits)->Unsigned {value.as(Unsigned)}
        comptime fn signed(value:Bits)->Signed {value.as(Signed)}
        comptime fn float(value:Wide)->Float {value.as(Float)}
        comptime fn equal(left:Bits,right:Bits)->bool {left == right}",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    let mut cases = Vec::new();
    for value in [0, 127, 128, 255] {
        cases.push(("widen", vec![bits(8, value)], bits(32, value)));
        cases.push((
            "unsigned",
            vec![bits(8, value)],
            Value::UnsignedInteger(value as u128),
        ));
        cases.push((
            "signed",
            vec![bits(8, value)],
            Value::SignedInteger(value as u8 as i8 as i128),
        ));
    }
    for raw in [0, 0x80000000, 1, 0x7f800000, 0x7fc00042, 0x7f800001] {
        cases.push(("float", vec![bits(32, raw)], Value::Float32(raw)));
    }
    cases.push(("equal", vec![bits(8, 255), bits(8, 255)], Value::Bool(true)));
    cases.push((
        "equal",
        vec![bits(8, 255), bits(8, 127)],
        Value::Bool(false),
    ));
    for (name, arguments, expected) in cases {
        let before = vm.bytecode_invocations();
        let id = support::id(&package, name);
        assert_eq!(
            vm.invoke(id, arguments.clone()).unwrap(),
            expected,
            "{name}"
        );
        assert_eq!(vm.bytecode_invocations() - before, 1, "{name}");
        assert_eq!(tree.invoke(id, arguments).unwrap(), expected, "{name}");
    }
}

#[test]
fn taking_bits_compiles_scalar_and_encoded_enum_representations() {
    let package = support::compile(
        "typealias Byte=.i8'builtin;
        typealias Word=.u16'builtin;
        typealias Half=.f16'builtin;
        typealias Brain=.bf16'builtin;
        typealias Bits=.BitVec'builtin<16>;
        enum Signed {.base_type Word,.discrete_tag true,number:0xA (value:Byte) F,}
        comptime fn signed(value:Byte)->Word {
            let code=Signed.number {value:value};code'bitvec.as(Word)
        }
        comptime fn half(value:Half)->Bits {value'bitvec}
        comptime fn brain(value:Brain)->Bits {value'bitvec}
        comptime fn identity(value:Bits)->Bits {value'bitvec}",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for (name, argument, expected) in [
        (
            "signed",
            Value::SignedInteger(-1),
            Value::UnsignedInteger(0xAFFF),
        ),
        (
            "signed",
            Value::SignedInteger(0),
            Value::UnsignedInteger(0xA00F),
        ),
        ("half", Value::Float16(0x8000), bits(16, 0x8000)),
        ("half", Value::Float16(0x7e42), bits(16, 0x7e42)),
        ("brain", Value::BFloat16(0x7fc1), bits(16, 0x7fc1)),
        ("identity", bits(16, 0x1234), bits(16, 0x1234)),
    ] {
        let before = vm.bytecode_invocations();
        let id = support::id(&package, name);
        assert_eq!(
            vm.invoke(id, vec![argument.clone()]).unwrap(),
            expected,
            "{name}"
        );
        assert_eq!(vm.bytecode_invocations() - before, 1, "{name}");
        assert_eq!(tree.invoke(id, vec![argument]).unwrap(), expected, "{name}");
    }
}
