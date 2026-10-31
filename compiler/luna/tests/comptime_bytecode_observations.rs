//! Metadata observations and shipped text wrappers execute as linear code.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

#[test]
fn text_and_names_use_bytecode_with_utf8_byte_boundaries() {
    let source = format!(
        "{}
        const symbol_byte=.symbol_byte'builtin;
        enum Event {{ready,stopped}}
        comptime fn length(value:str)->u64 {{value.byte_length()}}
        comptime fn read_byte(value:str,index:u64)->?u8 {{value.byte(index)}}
        comptime fn symbol(value:Symbol)->str {{value'tag_name}}
        comptime fn symbol_at(value:Symbol,index:u64)->?u8 {{symbol_byte(value,index)}}
        comptime fn early()->bool {{symbol_byte(do {{return true;}},0);false}}
        comptime fn event()->str {{Event.ready'tag_name}}",
        include!("support/text_source.rs")
    );
    let package = support::compile(&source);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::for_comptime_tree(&package, Budget::default()).unwrap();
    for (name, args, expected, count) in [
        (
            "length",
            vec![Value::String("a🦀".into())],
            Value::UnsignedInteger(5),
            2,
        ),
        (
            "length",
            vec![Value::String("".into())],
            Value::UnsignedInteger(0),
            2,
        ),
        (
            "symbol",
            vec![Value::Symbol("ready".into())],
            Value::String("ready".into()),
            1,
        ),
        ("event", vec![], Value::String("ready".into()), 1),
        ("early", vec![], Value::Bool(true), 1),
    ] {
        let before = vm.bytecode_invocations();
        let id = support::id(&package, name);
        assert_eq!(
            vm.invoke(id, args.clone())
                .unwrap_or_else(|e| panic!("compiled {name}: {e:?}")),
            expected
        );
        assert_eq!(vm.bytecode_invocations() - before, count, "{name}");
        assert_eq!(
            tree.invoke(id, args)
                .unwrap_or_else(|e| panic!("tree {name}: {e:?}")),
            expected
        );
    }
    for name in ["read_byte", "symbol_at"] {
        for (index, expected) in [(0, Some(195)), (1, Some(169)), (2, None), (u64::MAX, None)] {
            let text = if name == "read_byte" {
                Value::String("é".into())
            } else {
                Value::Symbol("é".into())
            };
            let args = vec![text, Value::UnsignedInteger(index as u128)];
            let before = vm.bytecode_invocations();
            let id = support::id(&package, name);
            let value = vm
                .invoke(id, args.clone())
                .unwrap_or_else(|e| panic!("compiled {name}: {e:?}"));
            assert_eq!(
                value,
                tree.invoke(id, args)
                    .unwrap_or_else(|e| panic!("tree {name}: {e:?}"))
            );
            let Value::Optional { value, .. } = value else {
                panic!("expected optional byte")
            };
            assert_eq!(value.map(|v| *v), expected.map(Value::UnsignedInteger));
            assert_eq!(
                vm.bytecode_invocations() - before,
                if name == "read_byte" { 2 } else { 1 }
            );
        }
    }
    assert_eq!(tree.bytecode_invocations(), 0);
    assert_eq!(tree.bytecode_staged_inputs(), 0);
}
