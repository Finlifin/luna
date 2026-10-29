//! Variant construction retains the enum head, inputs and selected payload.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::program::{Expr, ExprKind as K};
use nir::{Primitive, Term};

fn package() -> nir::program::Package {
    support::compile(
        "typealias Integer = .Integer'builtin;
        enum Message where T {
            none,
            one(T),
            two(T, T),
            named { left: T, right: T },
        }
        comptime fn empty() -> Message<bool> { .none }
        comptime fn single() -> Message<bool> { .one(true) }
        comptime fn positional() -> Message<Integer> {
            var trace: Integer = 0;
            .two(do { trace = trace * 10 + 1; trace }, do { trace = trace * 10 + 2; trace })
        }
        comptime fn named() -> Message<Integer> {
            var trace: Integer = 0;
            .named { right: do { trace = trace * 10 + 2; trace }, left: do { trace = trace * 10 + 1; trace } }
        }
        comptime fn early() -> Message<bool> {
            .two(do { return .none; }, do { while true {} false })
        }",
    )
}

#[test]
fn compiled_variants_preserve_identity_arguments_and_initialization_order() {
    let package = package();
    let declaration = package
        .enums
        .iter()
        .find(|e| e.name == "Message")
        .unwrap()
        .declaration;
    let integer = |value| Value::Integer(comptime_runtime::Integer::from_i128(value));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for (entry, parameter, index, fields) in [
        ("empty", Primitive::Bool, 0, vec![]),
        ("single", Primitive::Bool, 1, vec![Value::Bool(true)]),
        (
            "positional",
            Primitive::Integer,
            2,
            vec![integer(1), integer(12)],
        ),
        (
            "named",
            Primitive::Integer,
            3,
            vec![integer(1), integer(12)],
        ),
        ("early", Primitive::Bool, 0, vec![]),
    ] {
        let id = support::id(&package, entry);
        let expected = Value::Variant {
            declaration,
            arguments: vec![Term::Primitive(parameter)],
            index,
            fields,
        };
        let before = vm.bytecode_invocations();
        assert_eq!(vm.invoke(id, vec![]).unwrap(), expected, "{entry}");
        assert_eq!(tree.invoke(id, vec![]).unwrap(), expected, "{entry}");
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}

fn tail(mut value: &mut Expr) -> &mut Expr {
    while matches!(
        value.kind,
        K::Block {
            result: Some(_),
            ..
        }
    ) {
        let K::Block {
            result: Some(result),
            ..
        } = &mut value.kind
        else {
            unreachable!()
        };
        value = result;
    }
    value
}

#[test]
fn malformed_variant_index_payload_and_type_are_rejected() {
    let package = package();
    let root = support::id(&package, "single");
    for fault in 0..3 {
        let mut package = package.clone();
        let function = package
            .functions
            .iter_mut()
            .find(|f| f.declaration == root)
            .unwrap();
        let expression = tail(&mut function.body);
        let K::Variant { index, fields, .. } = &mut expression.kind else {
            panic!("expected variant")
        };
        match fault {
            0 => *index = usize::MAX,
            1 => fields.clear(),
            2 => fields[0].kind = K::Unit,
            _ => unreachable!(),
        }
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(vm.invoke(root, vec![]).unwrap_err().kind, ErrorKind::Type);
        assert_eq!(tree.invoke(root, vec![]).unwrap_err().kind, ErrorKind::Type);
        assert_eq!(vm.bytecode_invocations(), 1);
        assert!(vm.invoke(support::id(&package, "empty"), vec![]).is_ok());
    }
}
