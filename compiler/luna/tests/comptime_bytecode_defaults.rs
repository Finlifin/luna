//! Defaults retain declaration-side bindings while the caller is suspended.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};

#[test]
fn defaults_see_prepared_inputs_after_all_explicit_effects() {
    let package = support::compile(
        "typealias Integer = .Integer'builtin;
        comptime fn next(value: Integer) -> Integer { value + 1 }
        comptime fn digits(a: Integer, .b: Integer = a + 1, .c: Integer = next(b)) -> Integer {
            a * 100 + b * 10 + c
        }
        comptime fn run() -> bool {
            let a: Integer = 90;
            let b: Integer = 80;
            var trace: Integer = 0;
            let result = digits(do { trace = trace * 10 + 1; 2 },
                .c = do { trace = trace * 10 + 2; 9 });
            result == 239 and trace == 12 and digits(3) == 345
                and digits(2, .b = 4, .c = 6) == 246 and a == 90 and b == 80
        }",
    );
    let id = support::id(&package, "run");
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(tree.invoke(id, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(vm.bytecode_invocations(), 5);
    assert_eq!(vm.bytecode_default_evaluations(), 3);
}

#[test]
fn nested_and_generic_defaults_use_independent_frames() {
    let package = support::compile(
        "typealias Integer = .Integer'builtin;
        comptime fn increment(value: Integer, .amount: Integer = 1) -> Integer { value + amount }
        comptime fn choose(value: Integer, .result: Integer = increment(value)) -> Integer { result }
        comptime fn identity(input: T, .result: T = input) -> T where T { result }
        comptime fn run() -> bool { choose(40) == 41 and identity<bool>(true) }",
    );
    let id = support::id(&package, "run");
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(tree.invoke(id, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(vm.bytecode_invocations(), 4);
    assert_eq!(vm.bytecode_default_evaluations(), 3);
}

#[test]
fn default_failure_prevents_entry_and_restores_the_caller() {
    let package = support::compile(
        "typealias Integer = .Integer'builtin;
        comptime fn descend(depth: Integer) -> bool {
            if depth == 0 { true } else { descend(depth - 1) }
        }
        comptime fn choose(depth: Integer, .result: bool = descend(depth)) -> bool { result }
        comptime fn run(depth: Integer) -> bool { choose(depth) }",
    );
    let id = support::id(&package, "run");
    let selected = support::id(&package, "choose");
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    let integer = |value| Value::Integer(comptime_runtime::Integer::from_i128(value));
    assert_eq!(
        vm.invoke(id, vec![integer(100)]).unwrap_err().kind,
        ErrorKind::DepthExhausted
    );
    assert_eq!(
        tree.invoke(id, vec![integer(100)]).unwrap_err().kind,
        ErrorKind::DepthExhausted
    );
    assert!(!vm.events().iter().any(|event| matches!(event,
        comptime_interpretor::Event::EnterFunction(id) if *id == selected)));
    assert_eq!(vm.bytecode_default_evaluations(), 1);
    assert_eq!(vm.invoke(id, vec![integer(2)]).unwrap(), Value::Bool(true));
    assert_eq!(
        tree.invoke(id, vec![integer(2)]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(vm.bytecode_default_evaluations(), 2);
    assert_eq!(
        vm.events()
            .iter()
            .filter(|event| matches!(event,
        comptime_interpretor::Event::EnterFunction(id) if *id == selected))
            .count(),
        1
    );
}

#[test]
fn unsupported_default_selects_its_carrier_without_replaying_the_caller() {
    let package = support::compile(
        "comptime fn choose(value: bool, .result: bool = value match {true => false, false => true}) -> bool { result }
        comptime fn run() -> bool { var value = false; choose(do { value = not value; value }) or not value }",
    );
    let id = support::id(&package, "run");
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id, vec![]).unwrap(), Value::Bool(false));
    assert_eq!(tree.invoke(id, vec![]).unwrap(), Value::Bool(false));
    assert_eq!(vm.bytecode_invocations(), 2);
    assert_eq!(vm.bytecode_default_evaluations(), 0);
}

#[test]
fn local_instance_defaults_substitute_the_selected_type_inputs() {
    use nir::program::{Expr, ExprKind as K, LocalId};
    use nir::{Primitive, Term};
    let mut package = support::compile(
        "typealias Type = .Type'builtin;
        comptime fn Select(.result: Type = T) -> Type where T { result }
        comptime fn Run() -> Type { Select<bool>() }",
    );
    let selected = support::id(&package, "Select");
    let root = support::id(&package, "Run");
    let definition = package
        .functions
        .iter()
        .find(|f| f.declaration == selected)
        .unwrap();
    let template = definition.defaults[0].1.clone();
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.declaration == root)
        .unwrap();
    let mut expression = &mut function.body;
    while matches!(
        expression.kind,
        K::Block {
            result: Some(_),
            ..
        }
    ) {
        let K::Block {
            result: Some(result),
            ..
        } = &mut expression.kind
        else {
            unreachable!()
        };
        expression = result;
    }
    let mut call = expression.clone();
    let K::Call {
        callee, arguments, ..
    } = &mut call.kind
    else {
        panic!("expected call")
    };
    assert!(matches!(callee.kind, K::FunctionInstance { .. }));
    // Retaining a selected instance in a local is the call-preparation carrier
    // used by dynamic input/spread plans. Its defaults retain declaration types.
    let local = LocalId {
        owner: u32::MAX,
        index: 0,
    };
    let instance = callee.clone();
    callee.kind = K::Local(local);
    arguments[0].value = template;
    expression.kind = K::Block {
        statements: vec![Expr {
            source: expression.source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::Let {
                binding: local,
                value: instance,
            },
        }],
        result: Some(Box::new(call)),
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    let expected = Value::Type(Term::Primitive(Primitive::Bool));
    assert_eq!(vm.invoke(root, vec![]).unwrap(), expected);
    assert_eq!(tree.invoke(root, vec![]).unwrap(), expected);
    assert_eq!(vm.bytecode_invocations(), 2);
    assert_eq!(vm.bytecode_default_evaluations(), 1);
}

fn tail_expression(mut expression: &mut nir::program::Expr) -> &mut nir::program::Expr {
    use nir::program::ExprKind as K;
    while matches!(
        expression.kind,
        K::Block {
            result: Some(_),
            ..
        }
    ) {
        let K::Block {
            result: Some(result),
            ..
        } = &mut expression.kind
        else {
            unreachable!()
        };
        expression = result;
    }
    expression
}

#[test]
fn malformed_default_plans_are_rejected_before_execution() {
    use nir::program::ExprKind as K;
    let package = support::compile(
        "comptime fn choose(.a: bool = true, .b: bool = false) -> bool { a and b }
        comptime fn run() -> bool { choose() }",
    );
    let root = support::id(&package, "run");
    for fault in 0..3 {
        let mut package = package.clone();
        let function = package
            .functions
            .iter_mut()
            .find(|f| f.declaration == root)
            .unwrap();
        let K::Call {
            arguments,
            parameter_bindings,
            ..
        } = &mut tail_expression(&mut function.body).kind
        else {
            panic!("expected call")
        };
        match fault {
            0 => arguments.reverse(),
            1 => arguments[1].is_default = false,
            2 => parameter_bindings.clear(),
            _ => unreachable!(),
        }
        // Package admission must continue rejecting malformed plans even though
        // the bytecode executor also validates them defensively.
        let Err(error) = Interpreter::for_comptime(&package, Budget::default()) else {
            panic!("malformed default plan admitted");
        };
        assert_eq!(error.kind, ErrorKind::PhaseMismatch);
        let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            tree.invoke(root, vec![]).unwrap_err().kind,
            ErrorKind::InvalidCallPlan
        );
    }
}

#[test]
fn default_control_flow_exits_the_caller_without_entering_the_callee() {
    use nir::program::ExprKind as K;
    let mut package = support::compile(
        "comptime fn choose(.value: bool = true) -> bool { not value }
        comptime fn run() -> bool { choose() }",
    );
    let root = support::id(&package, "run");
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.declaration == root)
        .unwrap();
    let K::Call { arguments, .. } = &mut tail_expression(&mut function.body).kind else {
        panic!("expected call")
    };
    // Exercise the carrier's control-flow delivery independently of which
    // source constructs are permitted to emit an early exit in a default.
    let value = arguments[0].value.clone();
    arguments[0].value.kind = K::Return(Some(Box::new(value)));
    arguments[0].value.ty = nir::Term::Primitive(nir::Primitive::Never);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(root, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(tree.invoke(root, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(vm.bytecode_invocations(), 1);
    assert_eq!(vm.bytecode_default_evaluations(), 1);
}
