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
    // Keep the small-budget rejection and also exercise exhaustion after code
    // has started. Cold compilation now consumes the same invocation budget.
    for (steps, executed) in [(80, 0), (Budget::default().steps, 1)] {
        let mut vm = Interpreter::for_comptime(&package, Budget { steps, depth: 128 }).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "forever"), vec![])
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(vm.bytecode_invocations(), executed);
        assert!(vm.bytecode_compilation_steps() > 0);
    }
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
    // Both the aggregate-building caller and its receiver body execute code.
    assert_eq!(vm.bytecode_invocations(), 2);
    assert_eq!(vm.bytecode_compilations(), 2);
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(vm.bytecode_invocations(), 4);
    // Both the compiled caller and callee reuse lowering decisions;
    // the impure receiver mutation must still execute on the second call.
    assert_eq!(vm.bytecode_compilations(), 2);
}

#[test]
fn calls_suspend_frames_and_preserve_argument_order() {
    let package = support::compile(
        "typealias Integer = .Integer'builtin;
        comptime fn digits(a: Integer, b: Integer) -> Integer { a * 10 + b }
        comptime fn countdown(n: Integer) -> Integer {
            if n == 0 { 0 } else { 1 + countdown(n - 1) }
        }
        comptime fn pattern(value: bool) -> bool {
            value match { true => digits(1, 2) == 12, false => false }
        }
        comptime fn bridge() -> bool { pattern(true) }
        comptime fn run(n: Integer) -> bool {
            var trace: Integer = 0;
            let result = digits(
                do { trace = trace * 10 + 1; digits(2, 3) },
                do { trace = trace * 10 + 2; countdown(n) }
            );
            result == 230 + n and trace == 12
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    let id = support::id(&package, "run");
    let args = |n| vec![Value::Integer(comptime_runtime::Integer::from_i128(n))];
    assert_eq!(vm.invoke(id, args(3)).unwrap(), Value::Bool(true));
    assert_eq!(tree.invoke(id, args(3)).unwrap(), Value::Bool(true));
    let before = vm.bytecode_invocations();
    assert_eq!(vm.invoke(id, args(20)).unwrap(), Value::Bool(true));
    assert_eq!(vm.bytecode_invocations() - before, 24);
    assert_eq!(vm.bytecode_compilations(), 3);
    // The migration evaluator also charges recursive expression depth. The
    // compiled path holds these suspended calls on its explicit frame stack.
    assert_eq!(
        tree.invoke(id, args(20)).unwrap_err().kind,
        ErrorKind::DepthExhausted
    );
    assert_eq!(
        vm.invoke(id, args(100)).unwrap_err().kind,
        ErrorKind::DepthExhausted
    );
    assert_eq!(vm.invoke(id, args(3)).unwrap(), Value::Bool(true));
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(support::id(&package, "bridge"), vec![]).unwrap(),
        Value::Bool(true)
    );
    // Structural bool matching is compiled too: bridge, pattern and digits
    // each execute once without replaying their callers.
    assert_eq!(vm.bytecode_invocations() - before, 3);
}

#[test]
fn nested_calls_check_contracts_and_recover_after_failure() {
    let mut package = support::compile(
        "comptime fn checked(value: bool) -> bool { value }
        comptime fn bad_exit(value: bool) -> result: bool
        where ensures result == value
        { value }
        comptime fn run(value: bool) -> bool { checked(value) }
        comptime fn fail_exit(value: bool) -> bool { bad_exit(value) }
        comptime fn identity(value: T) -> T where T { value }
        comptime fn generic(value: bool) -> bool { identity<bool>(value) }",
    );
    // Source verification rejects these invalid calls/returns. Alter the checked
    // carrier deliberately to exercise invocation admission and delivery too.
    let checked = support::id(&package, "checked");
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.declaration == checked)
        .unwrap();
    function.preconditions.push(function.body.clone());
    let broken = support::id(&package, "bad_exit");
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.declaration == broken)
        .unwrap();
    function.body.kind = nir::program::ExprKind::Not(Box::new(function.body.clone()));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for (entry, input, fails) in [
        ("run", false, true),
        ("run", true, false),
        ("fail_exit", true, true),
        ("fail_exit", false, true),
        ("generic", true, false),
    ] {
        let id = support::id(&package, entry);
        let before = vm.bytecode_invocations();
        let result = vm.invoke(id, vec![Value::Bool(input)]);
        let reference = tree.invoke(id, vec![Value::Bool(input)]);
        if fails {
            assert_eq!(result.unwrap_err().kind, ErrorKind::ProgramContractRejected);
            assert_eq!(
                reference.unwrap_err().kind,
                ErrorKind::ProgramContractRejected
            );
        } else {
            assert_eq!(result.unwrap(), Value::Bool(input));
            assert_eq!(reference.unwrap(), Value::Bool(input));
        }
        assert_eq!(
            vm.bytecode_invocations() - before,
            if entry == "run" && fails { 1 } else { 2 }
        );
    }
}
