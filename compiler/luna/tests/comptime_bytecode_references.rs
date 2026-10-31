//! Value-reference transport is invocation storage, not a runtime pointer.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
use nir::program::{ArgumentPassing, Expr, ExprKind as K, Package};

fn compile() -> Package {
    support::compile(
        "struct Cell { value: bool }
        struct Envelope { left: Cell, right: Cell }
        impl Cell {
            comptime fn set(self, value: bool) { self.value = value; }
            comptime fn replace(self, other: Cell) { self = other; }
            comptime fn forward(self, other: Cell) { self.replace(other); }
            comptime fn snapshot(self) -> Cell { self }
            comptime fn swap(self, other: Cell) -> Cell { let previous = self; self = other; previous }
            comptime fn retain(self, .other: Cell = self) { self = other; }
        }
        impl Envelope {
            comptime fn set_left(self, other: Cell) { self.left.replace(other); }
        }
        comptime fn field_of_let(input: Envelope, other: Cell) -> Envelope {
            let current = input; current.set_left(other); current
        }
        comptime fn fields(input: Envelope) -> bool {
            var current = input;
            current.left.set(true);
            current.left.value and not current.right.value
        }
        comptime fn field_order(input: Envelope) -> bool {
            var current = input;
            current.left.value = do { current.right.set(true); false };
            not current.left.value and current.right.value
        }
        comptime fn same_envelope(input: Envelope) -> Envelope { input }
        comptime fn computed_field(input: Envelope) -> bool { same_envelope(input).left.value }
        comptime fn identity(value: Cell) -> Cell { value }
        comptime fn run(input: Cell, other: Cell) -> Cell {
            var current = input; current.replace(other); current
        }
        comptime fn projected(input: Envelope, other: Cell) -> Envelope {
            var current = input; current.left.replace(other); current
        }
        comptime fn forwarded(input: Cell, other: Cell) -> Cell {
            var current = input; current.forward(other); current
        }
        comptime fn observed(input: Cell) -> Cell { let current = input; current.snapshot() }
        comptime fn temporary(input: Cell, other: Cell) -> Cell { identity(input).replace(other); input }
        comptime fn order(input: Cell, other: Cell) -> Cell {
            var current = input;
            current.swap(do { current = other; input })
        }
        comptime fn retained(input: Cell) -> Cell { var current = input; current.retain(); current }",
    )
}

fn record(package: &Package, name: &str, fields: Vec<Value>) -> Value {
    Value::Record {
        declaration: package
            .structs
            .iter()
            .find(|s| s.name == name)
            .unwrap()
            .declaration,
        arguments: vec![],
        fields,
    }
}

#[test]
fn compiled_receiver_calls_write_back_and_forward_the_same_storage() {
    let package = compile();
    let old = record(&package, "Cell", vec![Value::Bool(false)]);
    let new = record(&package, "Cell", vec![Value::Bool(true)]);
    let envelope = record(&package, "Envelope", vec![old.clone(), old.clone()]);
    let changed = record(&package, "Envelope", vec![new.clone(), old.clone()]);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (name, args, expected, calls) in [
        ("run", vec![old.clone(), new.clone()], new.clone(), 2),
        (
            "projected",
            vec![envelope.clone(), new.clone()],
            changed.clone(),
            2,
        ),
        ("field_of_let", vec![envelope, new.clone()], changed, 3),
        ("forwarded", vec![old.clone(), new.clone()], new.clone(), 3),
        ("observed", vec![old.clone()], old.clone(), 2),
        ("order", vec![old.clone(), new.clone()], new, 2),
        ("retained", vec![old.clone()], old, 2),
    ] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(support::id(&package, name), args).unwrap(),
            expected,
            "{name}"
        );
        assert_eq!(
            vm.bytecode_invocations() - before,
            calls,
            "{name} must compile the caller too"
        );
    }
    assert_eq!(vm.bytecode_default_evaluations(), 1);
}

#[test]
fn computed_receiver_is_evaluated_once_into_private_storage() {
    let package = compile();
    let old = record(&package, "Cell", vec![Value::Bool(false)]);
    let new = record(&package, "Cell", vec![Value::Bool(true)]);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "temporary"), vec![old.clone(), new])
            .unwrap(),
        old
    );
    assert_eq!(vm.bytecode_invocations(), 3);
    let identity = support::id(&package, "identity");
    assert_eq!(
        vm.events()
            .iter()
            .filter(|event| matches!(event,
        Event::EnterFunction(id) if *id == identity))
            .count(),
        1
    );
}

fn prohibit_receiver_rebind(expression: &mut Expr) -> bool {
    match &mut expression.kind {
        K::Call { arguments, .. } => {
            if let Some(argument) = arguments
                .iter_mut()
                .find(|a| a.passing == ArgumentPassing::ComptimeReference)
            {
                argument.passing = ArgumentPassing::ComptimeReferenceNoRebind;
                true
            } else {
                false
            }
        }
        K::Block { statements, result } => {
            statements.iter_mut().any(prohibit_receiver_rebind)
                || result.as_deref_mut().is_some_and(prohibit_receiver_rebind)
        }
        K::Discard(value) => prohibit_receiver_rebind(value),
        _ => false,
    }
}

#[test]
fn no_rebind_permission_survives_compiled_forwarding() {
    let mut package = compile();
    let id = support::id(&package, "forwarded");
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.declaration == id)
        .unwrap();
    // A let receiver uses this transport permission. Restrict a valid call plan
    // directly so execution must enforce it even across the forwarding method.
    assert!(prohibit_receiver_rebind(&mut function.body));
    let old = record(&package, "Cell", vec![Value::Bool(false)]);
    let new = record(&package, "Cell", vec![Value::Bool(true)]);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id, vec![old.clone(), new.clone()])
            .unwrap_err()
            .kind,
        ErrorKind::InvalidValueReference
    );
    assert_eq!(vm.bytecode_invocations(), 3);
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![old, new.clone()])
            .unwrap(),
        new
    );
}

#[test]
fn compiled_fields_preserve_sibling_writes_and_read_computed_bases() {
    let package = compile();
    let cell = record(&package, "Cell", vec![Value::Bool(false)]);
    let input = record(&package, "Envelope", vec![cell.clone(), cell]);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (entry, expected) in [
        ("fields", true),
        ("field_order", true),
        ("computed_field", false),
    ] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![input.clone()])
                .unwrap(),
            Value::Bool(expected)
        );
        assert_eq!(vm.bytecode_invocations() - before, 2);
    }
}
