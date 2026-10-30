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
