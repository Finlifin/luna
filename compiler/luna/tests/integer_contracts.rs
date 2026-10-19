//! Comptime integer specifications: inference, copies, expressions and versions.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use support::{compile, compile_with_protocols, id};

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
