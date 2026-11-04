//! Runtime pointer descriptions and comptime pointer execution are distinct.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn comptime_pointer_operations_are_rejected_before_invocation() {
    for source in [
        "comptime fn run()->bool {let value=true;value.ref.*}",
        "comptime fn run()->bool {let value=false;value.ref.* =true;value}",
        "comptime fn run()->bool {let value=false;if false {value.ref.* =true;} value}",
        "comptime fn run()->bool {let value=false;let action=|| {value.ref.*};action()}",
        "comptime fn run(p:*bool)->bool {p.*}",
        "struct Cell {value:bool} comptime fn run(p:*Cell)->*bool {p.value}",
        "comptime fn run(p:*Word) {p.*'bitvec[0]=1;}",
    ] {
        let source = format!("typealias Word=.u8'builtin;{source}");
        match support::compile_with_protocols(&source, &Default::default()) {
            Err(message) => assert!(
                message.contains("pointer operations cannot execute at comptime"),
                "{source}: {message}"
            ),
            Ok(package) => {
                let error = nir::phases::validate(&package).unwrap_err();
                assert!(
                    error
                        .message
                        .contains("pointer operations cannot execute at comptime"),
                    "{source}: {error:?}"
                );
                let error =
                    comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
                        .err()
                        .expect("accepted comptime pointer operation");
                let comptime_interpretor::ErrorKind::InvalidPhase(reason) = &error.kind else {
                    panic!("expected the source phase failure: {error}");
                };
                assert!(
                    reason
                        .message
                        .contains("pointer operations cannot execute at comptime")
                );
                assert_eq!(error.source, Some(reason.source));
            }
        }
    }
}

#[test]
fn comptime_can_construct_runtime_pointer_types() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        comptime fn Pointer(input:Type)->Type {*input}
        comptime fn run()->Type {Pointer(bool)}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Type(nir::Term::Pointer(Box::new(nir::Term::Primitive(
            nir::Primitive::Bool
        ))))
    );
}

#[test]
fn runtime_pointer_execution_in_a_mixed_package_is_unchanged() {
    let package = support::compile(
        "fn runtime()->bool {let value=false;value.ref.* =true;value}
        comptime fn ready()->bool {true}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "ready", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "runtime", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn quoted_runtime_pointer_operations_remain_legal() {
    let package = support::compile(
        "comptime fn build()-><|-bool> {
            '(do {let value=true;value.ref.*})
        }
        fn run()->bool {~<build()>}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pointer_operations_in_quote_producers_are_not_runtime_code() {
    let source = "comptime fn build()-><|-bool> {
            let value=true;let observed=value.ref.*;'(observed)
        }
        fn run()->bool {~<build()>}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("pointer operations cannot execute at comptime"),
        "{error}"
    );
}
