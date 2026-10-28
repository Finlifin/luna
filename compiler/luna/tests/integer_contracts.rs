//! Comptime integer specifications: inference, copies, expressions and versions.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use support::{compile, compile_with_protocols, id};

#[test]
fn established_branches_keep_condition_effects_and_selected_value_facts() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for .u8'builtin {}
        typealias Byte = .u8'builtin;
        fn run() -> bool {
            var calls: Byte = 0;
            let first: Byte = if do { calls = calls + 1; true } { 10 } else { 255 };
            let second: Byte = if do { calls = calls + 1; false } { 255 } else { first + 1 };
            let third: Byte = if second < 20 { second + 1 } else { 255 };
            calls == 2 and third == 12
        }";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500_000)
            .unwrap(),
        Value::Bool(true),
    );
    // Neither an unknown input nor a stale value before assignment establishes
    // which side is reached. A reachable overflowing operation must fail.
    for body in [
        "if flag { value + 1 } else { 0 }",
        "do { var local: Byte = 0; local = value; if flag { local + 1 } else { 0 } }",
        "do { var local: Byte = 0; if local < 1 { local = value; if local < 255 { 0 } else { local + 1 } } else { 0 } }",
    ] {
        let invalid = format!(
            "typealias Byte = .u8'builtin; fn bad(flag: bool, value: Byte) -> Byte {{ {body} }}"
        );
        let error = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
        assert!(error.contains("range"), "{error}");
    }
}

#[test]
fn checked_execution_revalidates_residual_empty_window_endpoints() {
    let mut package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         typealias Bits=.BitVec'builtin<16>;typealias Empty=.BitVec'builtin<0>;
         fn window(bits:Bits,start:.u8'builtin)->Empty {assumes start <= 16;bits[start..start]}
         fn run()->Empty {let bits:Bits=0xABCD;window(bits,4)}",
    );
    let Value::BitVec(value) =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500_000)
            .unwrap()
    else {
        panic!("expected bits")
    };
    assert!(value.words().is_empty());
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.name == "window")
        .unwrap();
    let mut removed = 0;
    iir::visit::expression_mut(&mut function.body, &mut 100_000, |expr, _| {
        if matches!(expr.kind, nir::program::ExprKind::CheckPredicate(_)) {
            expr.kind = nir::program::ExprKind::Unit;
            removed += 1;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(removed, 1);
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500_000)
            .unwrap_err();
    assert!(error.contains("residual program contract"), "{error}");
}

#[test]
fn checked_execution_revalidates_residual_arithmetic_evidence() {
    let mut package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         typealias Byte=.u8'builtin;
         fn increment(value:Byte)->Byte {assumes value < 255;value + 1}
         fn run()->Byte {increment(254)}",
    );
    let value =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500_000)
            .unwrap();
    assert_eq!(value, Value::UnsignedInteger(255));
    // Simulate a transformation dropping the required runtime check. Source
    // validation already ran; residual verification must catch the lost proof.
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.name == "increment")
        .unwrap();
    let mut removed = 0;
    iir::visit::expression_mut(&mut function.body, &mut 100_000, |expr, _| {
        if matches!(expr.kind, nir::program::ExprKind::CheckPredicate(_)) {
            expr.kind = nir::program::ExprKind::Unit;
            removed += 1;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(removed, 1);
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500_000)
            .unwrap_err();
    assert!(
        error.contains("residual program contract") && error.contains("range"),
        "{error}"
    );
}

#[test]
fn unbounded_integer_contract_facts_survive_local_copies_and_calls() {
    let source = "typealias Int=.Integer'builtin;
        comptime fn positive(n:Int)->bool where requires n > 0 {true}
        comptime fn forward(n:Int)->bool where requires n > 10 {let copied=n;positive(copied)}
        fn run()->bool {const result=forward(11);result}";
    let package = compile(source);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let invalid = source.replace("let copied=n", "let copied=0");
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn unbounded_integer_contract_facts_survive_return_values() {
    let source = "typealias Int=.Integer'builtin;
        comptime fn identity(n:Int)->result:Int where requires n > 10, ensures result > 0 {n}
        comptime fn positive(n:Int)->bool where requires n > 0 {true}
        fn run()->bool {const result=positive(identity(11));result}";
    let package = compile(source);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn unbounded_integer_contract_facts_survive_tuple_storage() {
    let package = compile(
        "typealias Int=.Integer'builtin;
        comptime fn positive(n:Int)->bool where requires n > 0 {true}
        comptime fn forward(n:Int)->bool where requires n > 10 {
            let pair=(n,n);let (a,b)=pair;positive(a) and positive(b)
        }
        fn run()->bool {const result=forward(11);result}",
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn unbounded_integer_contract_copies_do_not_follow_later_writes() {
    let source = "typealias Int=.Integer'builtin;
        comptime fn positive(n:Int)->bool where requires n > 0 {true}
        comptime fn forward(n:Int)->bool where requires n > 10 {
            var current=n;let saved=current;current=0;positive(saved)
        }
        fn run()->bool {const result=forward(11);result}";
    let package = compile(source);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let invalid = source.replace("positive(saved)", "positive(current)");
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn unbounded_integer_contract_copy_equality_keeps_operand_symmetry() {
    let package = compile(
        "typealias Int=.Integer'builtin;
        comptime fn identity(n:Int)->result:Int where ensures result == n {let copied=n;copied}
        fn run()->bool {const value=identity(11) == 11;value}",
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn unbounded_integer_contract_addressed_locals_lose_written_facts() {
    let source = "typealias Int=.Integer'builtin;
        comptime fn positive(n:Int)->bool where requires n > 0 {true}
        comptime fn clear(n:*Int) {n.* = 0;}
        comptime fn forward(n:Int)->bool where requires n > 10 {
            var current=n;clear(current.ref);positive(current)
        }
        fn run()->bool {const result=forward(11);result}";
    let failure = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn unbounded_integer_expression_contracts_survive_assignment_and_return() {
    for (expression, expected) in [("n + 1", "12"), ("n * n", "121"), ("n * n * n", "1331")] {
        let source = format!(
            "typealias Int=.Integer'builtin;
            comptime fn calculate(n:Int)->result:Int where ensures result == {expression} {{
                let calculated={expression};calculated
            }}
            fn run()->bool {{const correct=calculate(11) == {expected};correct}}"
        );
        let package = compile(&source);
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        let invalid = source.replace(
            &format!("let calculated={expression}"),
            "let calculated=n + 2",
        );
        let failure = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
        assert!(failure.contains("postcondition"), "{failure}");
    }
}

#[test]
fn unbounded_integer_expression_contracts_use_entry_snapshots_on_self_assignment() {
    let source = "typealias Int=.Integer'builtin;
        comptime fn advance(n:Int)->result:Int where ensures result == n'old + 1 {
            var current=n;current=current + 1;current
        }
        fn run()->bool {const correct=advance(11) == 12;correct}";
    let package = compile(source);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn unbounded_integer_expression_contracts_preserve_mixed_cubic_inputs() {
    let source = "typealias Int=.Integer'builtin;
        comptime fn calculate(x:Int,y:Int,z:Int)->result:Int where ensures result == x * y * z + x {
            let calculated=x * y * z + x;calculated
        }
        fn run()->bool {const correct=calculate(2,3,4) == 26;correct}";
    let package = compile(source);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let invalid = source.replace(
        "let calculated=x * y * z + x",
        "let calculated=x * y * z + y",
    );
    let failure = compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("postcondition"), "{failure}");
}

#[test]
fn unbounded_integer_expression_contracts_survive_tuple_components() {
    let source = "typealias Int=.Integer'builtin;
        comptime fn calculate(n:Int)->result:Int where ensures result == n * n {
            let pair=(n + 1,n * n);let (a,b)=pair;b
        }
        fn run()->bool {const correct=calculate(11) == 121;correct}";
    let package = compile(source);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn unbounded_integer_expression_contracts_reject_stale_input_versions() {
    let source = "typealias Int=.Integer'builtin;
        comptime fn calculate(n:Int)->result:Int where ensures result == n + 1 {
            let calculated=n + 1;n=0;calculated
        }
        fn run()->bool {const correct=calculate(11) == 12;correct}";
    let failure = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(failure.contains("postcondition"), "{failure}");
}

#[test]
fn unbounded_integer_cancellation_proves_polynomial_preconditions() {
    for expression in [
        "n + 1 > n",
        "n * n + 1 > n * n",
        "n * n * n + n == n + n * n * n",
    ] {
        let source = format!(
            "typealias Int=.Integer'builtin;
            comptime fn checked(n:Int)->bool where requires {expression} {{true}}
            comptime fn forward(n:Int)->bool {{checked(n)}}
            fn run()->bool {{const result=forward(11);result}}"
        );
        let package = compile(&source);
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
    let source = "typealias Int=.Integer'builtin;
        comptime fn checked(n:Int)->bool where requires n + 1 < n {true}
        comptime fn forward(n:Int)->bool {checked(n)}
        fn run()->bool {const result=forward(11);result}";
    let failure = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn residual_contract_validation_avoids_report_snapshots_without_losing_checks() {
    use elaboration::verification::{Limits, flow};
    let checks = "assumes value < 255;".repeat(24);
    let source = format!(
        "typealias Byte = .u8'builtin; fn increment(value: Byte) -> Byte {{ {checks} value + 1 }}"
    );
    let package = compile(&source);
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "increment")
        .unwrap();
    let budget = 2_000_000;
    let mut reporting = Limits::default();
    reporting.arithmetic.steps = budget;
    let report = flow::analyze_in_package(&package, function, &mut reporting).unwrap();
    assert_eq!(
        report
            .points
            .iter()
            .filter(|p| p.check_input.is_some())
            .count(),
        24
    );
    let mut validation = Limits::default();
    validation.arithmetic.steps = budget;
    flow::validate_residual_contracts(&package, function, &mut validation).unwrap();
    assert!(
        validation.arithmetic.steps > reporting.arithmetic.steps,
        "validation used {}, reporting used {}",
        budget - validation.arithmetic.steps,
        budget - reporting.arithmetic.steps
    );

    let mut invalid = function.clone();
    iir::visit::expression_mut(&mut invalid.body, &mut 100_000, |expr, _| {
        if matches!(expr.kind, nir::program::ExprKind::CheckPredicate(_)) {
            expr.kind = nir::program::ExprKind::Unit;
        }
        Ok(())
    })
    .unwrap();
    let reported = flow::analyze_in_package(&package, &invalid, &mut Limits::default())
        .err()
        .unwrap();
    let validated =
        flow::validate_residual_contracts(&package, &invalid, &mut Limits::default()).unwrap_err();
    assert_eq!(reported, validated);
    assert_eq!(validated.reason, flow::Reason::MissingMachineProof);

    let mut exhausted = Limits::default();
    exhausted.arithmetic.steps = 0;
    assert!(flow::validate_residual_contracts(&package, function, &mut exhausted).is_err());
    flow::validate_residual_contracts(&package, function, &mut Limits::default()).unwrap();
}
