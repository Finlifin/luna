//! Stable finite call computations may supply a checked Boolean decision.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn finite_read_only_calls_supply_runtime_checks() {
    let source = "fn identity(value:bool)->bool {value}
        fn decide(left:bool,right:bool)->bool {left and not right}
        fn run()->bool {assumes decide(identity(true),false);true}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn predicate_calls_preserve_short_circuit_order_and_failed_checks() {
    let source = "fn left()->bool {true} fn right()->bool {false}
        fn skipped()->bool {false}
        fn decide(left:bool,right:bool)->bool {left and not right}
        fn run()->bool {assumes decide(left(),right()) or skipped();true}
        fn fail()->bool {assumes right();true}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    vm.invoke_instance(&graph.roots[0], vec![]).unwrap();
    let calls: Vec<_> = vm
        .events()
        .iter()
        .filter_map(|event| {
            if let comptime_interpretor::Event::EnterFunction(id) = event {
                package
                    .functions
                    .iter()
                    .find(|function| function.declaration == *id)
                    .map(|function| function.name.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(calls, ["run", "left", "right", "decide"]);
    assert!(
        luna::execution::interpret(&package, "fail", 100_000)
            .unwrap_err()
            .contains("ProgramContractRejected")
    );
}

#[test]
fn predicate_calls_require_stability_and_finite_completion() {
    for source in [
        "fn change(p:*bool)->bool {p.* = false;true} fn bad(p:*bool) {assumes change(p);}",
        "fn recur()->bool {recur()} fn bad() {assumes recur();}",
        "fn forever()->bool {while true {}true} fn bad() {assumes forever();}",
        "fn bad(action:fn()->bool) {assumes action();}",
        "fn good()->bool {true} fn bad() {var flag=true;assumes do {flag=false;good()};}",
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains("effect-free program predicate"),
            "{source}: {error}"
        );
    }
    assert!(
        support::compile_with_protocols(
            "fn good()->bool {true} fn bad()->bool {assumes good();}",
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn compound_checks_keep_arithmetic_facts_without_inventing_callee_facts() {
    let source = "typealias Byte=.u8'builtin;
        fn ready()->bool {true}
        fn next(value:Byte)->Byte {assumes value < 255 and ready();value + 1}
        fn run()->bool {next(7)==8}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let source = "typealias Byte=.u8'builtin;
        fn small(value:Byte)->bool {value < 255}
        fn bad(value:Byte)->Byte {assumes small(value);value + 1}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("range is not proven"), "{error}");
}

#[test]
#[ignore = "comptime call stability needs phase-aware value-reference evidence"]
fn comptime_decisions_check_their_results_before_returning_types() {
    let source = "typealias Type=.Type'builtin;
        comptime fn decide(value:bool)->bool {value}
        comptime fn Selected()->Type {assumes decide(true);bool}
        typealias Chosen=Selected();
        fn run()->Chosen {true}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let error = support::compile_with_protocols(
        &source.replace("decide(true)", "decide(false)"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("ProgramContractRejected"), "{error}");
}
