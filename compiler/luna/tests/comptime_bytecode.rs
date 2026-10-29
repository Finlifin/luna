//! These tests require actual invocation routing, not just successful lowering.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};

#[test]
fn arithmetic_loop_executes_bytecode_with_ordered_operand_effects() {
    let package = support::compile(
        "typealias Integer = .Integer'builtin;
        comptime fn sum(limit: Integer) -> Integer {
            var i: Integer = 0;
            var total: Integer = 0;
            while i < limit {
                i = i + 1;
                total = total + i * i;
            }
            -total
        }
        comptime fn order() -> bool {
            var trace: Integer = 0;
            let sum = do { trace = trace * 10 + 1; 7 }
                + do { trace = trace * 10 + 2; 3 };
            sum == 10 and trace == 12
        }",
    );
    let integer = |value| Value::Integer(comptime_runtime::Integer::from_i128(value));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for (entry, args, expected) in [
        ("sum", vec![integer(0)], integer(0)),
        ("sum", vec![integer(10)], integer(-385)),
        ("sum", vec![integer(100)], integer(-338350)),
        ("order", vec![], Value::Bool(true)),
    ] {
        let before = vm.bytecode_invocations();
        let id = support::id(&package, entry);
        assert_eq!(vm.invoke(id, args.clone()).unwrap(), expected);
        assert_eq!(vm.bytecode_invocations(), before + 1);
        assert_eq!(tree.invoke(id, args).unwrap(), expected);
    }
    assert_eq!(vm.bytecode_compilations(), 2);
}

#[test]
fn source_invocation_executes_linear_control_and_preserves_type_values() {
    let package = support::compile(
        "typealias Type = .Type'builtin;
        typealias u8 = .u8'builtin;
        comptime fn Choose(flag: bool) -> Type {
            var chosen = bool;
            if flag { chosen = u8; }
            chosen
        }
        comptime fn run() -> bool {
            var flag = true;
            while flag {
                let local = not flag;
                flag = local;
                continue;
            }
            not flag
        }
        comptime fn early() -> bool {
            true and do { return false; }
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for (entry, args, expected) in [
        (
            "Choose",
            vec![Value::Bool(true)],
            Value::Type(nir::Term::Primitive(nir::Primitive::U8)),
        ),
        (
            "Choose",
            vec![Value::Bool(false)],
            Value::Type(nir::Term::Primitive(nir::Primitive::Bool)),
        ),
        ("run", vec![], Value::Bool(true)),
        ("early", vec![], Value::Bool(false)),
    ] {
        let id = support::id(&package, entry);
        let before = vm.bytecode_invocations();
        assert_eq!(vm.invoke(id, args.clone()).unwrap(), expected);
        assert_eq!(
            vm.bytecode_invocations(),
            before + 1,
            "{entry} must execute bytecode"
        );
        assert_eq!(tree.invoke(id, args).unwrap(), expected);
    }
    assert_eq!(tree.bytecode_invocations(), 0);
}

#[test]
fn bytecode_budget_exhaustion_and_fresh_invocation_are_reported() {
    let package = support::compile(
        "comptime fn forever() -> bool { while true {} false }
        comptime fn ready() -> bool { true }",
    );
    let mut vm = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 80,
            depth: 128,
        },
    )
    .unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "forever"), vec![])
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(vm.bytecode_invocations(), 1);
    let mut recovered = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        recovered
            .invoke(support::id(&package, "ready"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(recovered.bytecode_invocations(), 1);
}

#[test]
fn compiled_receiver_updates_storage_without_runtime_copy_evidence() {
    let package = support::compile(
        "struct Cell { value:bool }
        impl Cell { comptime fn replace(self, other:Cell) { self = other; } }
        comptime fn run() -> bool {
            var cell = Cell {value:false};
            cell.replace(Cell {value:true});
            cell.value
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    // The caller's aggregate construction still uses the migration path; the
    // receiver body itself must execute the bytecode read and storage write.
    assert_eq!(vm.bytecode_invocations(), 1);
    assert_eq!(vm.bytecode_compilations(), 2);
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(vm.bytecode_invocations(), 2);
    // Both the unsupported caller and compiled callee reuse lowering decisions;
    // the impure receiver mutation must still execute on the second call.
    assert_eq!(vm.bytecode_compilations(), 2);
}
