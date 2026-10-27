#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn representation_queries_declare_without_target_but_execution_requires_it() {
    let package = support::compile(
        "typealias Word=.usize'builtin; typealias Wide=.u128'builtin; fn observe(value:Word)->Wide {.integer_representation'builtin(value)} fn independent()->bool {true}",
    );
    assert_eq!(package.target, None);
    assert!(format!("{package}").contains("(integer-representation"));
    assert_eq!(
        luna::execution::interpret(&package, "independent", 100000).unwrap(),
        Value::Bool(true)
    );
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "observe")
        .unwrap()
        .declaration;
    let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    assert!(vm.invoke(entry, vec![Value::UnsignedInteger(1)]).is_err());
}

#[test]
fn representation_queries_evaluate_once_and_preserve_early_exit() {
    let package = support::compile(
        "typealias U=.u64'builtin;typealias Wide=.u128'builtin;fn next(state:*U)->U {let old=state.*;assumes old == 0;state.* = 1;255} fn run()->bool {let state:U=0;let result=.integer_representation'builtin(next(state.ref));result == 255 and state == 1} comptime fn early()->bool {.integer_representation'builtin(do {return true;});false} fn runtime_early()->bool {.integer_representation'builtin(do {return true;});false} fn verify() {let ok=runtime_early();assumes ok;}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "early", 100000).unwrap(),
        Value::Bool(true)
    );
    let ir = luna::host::llvm_ir(&package, "verify", 100000).unwrap();
    luna::host::Toolchain::default()
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
}

#[test]
fn representation_queries_reject_non_integer_domains_and_wrong_arity() {
    for input in ["true", "'A'", "1.0", "", "true,false"] {
        assert!(
            support::compile_with_protocols(
                &format!(
                    "fn invalid()->.u128'builtin {{.integer_representation'builtin({input})}}"
                ),
                &Default::default()
            )
            .is_err(),
            "{input}"
        );
    }
}

#[test]
fn representation_queries_materialize_required_comptime_values() {
    let package = support::compile(
        "typealias Byte=.i8'builtin;typealias Wide=.u128'builtin;pure comptime fn widen(value:Byte)->Wide {.integer_representation'builtin(value)} const bits:Wide=widen(-1);comptime fn run()->bool {bits == 255}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100000).unwrap(),
        Value::Bool(true)
    );
}
