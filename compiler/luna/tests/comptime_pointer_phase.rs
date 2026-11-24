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

#[test]
fn generator_pointer_receivers_and_ops_are_rejected_and_value_self_works() {
    for (source, anchor) in [
        (
            "struct Cell { value: bool }
             impl Cell { comptime fn build(*self) -> <|- bool> { '(true) } }",
            "comptime methods use value-reference self",
        ),
        (
            "trait Build { comptime fn build(*self) -> <|- bool>; }",
            "comptime methods use value-reference self",
        ),
        (
            "struct Cell { value: bool }
             impl Cell { comptime fn pat(*self) -> <bool -| v: bool> { '<v> } }",
            "comptime methods use value-reference self",
        ),
        (
            "comptime fn bad(flag: bool) -> <|- bool> {
                if flag { var x = false; x.ref.* = true; }
                '(true)
            }",
            "pointer operations cannot execute at comptime",
        ),
        (
            "comptime fn bad(p: *bool) -> <|- bool> { '(true) }",
            "immutable value domain",
        ),
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(error.contains(anchor), "{source}: {error}");
    }

    let valid = support::compile(
        "struct Cell { value: bool }
         impl Cell {
             comptime fn build(self) -> <|- bool> {
                 self.value = true;
                 let snapshot = self.value;
                 '(snapshot)
             }
             comptime fn pat(self) -> <bool -| matched: bool> {
                 let wanted = self.value;
                 self.value = not wanted;
                 '<matched if matched == wanted>
             }
         }
         comptime fn produce_code() -> <|- bool> {
             var cell = Cell { value: false };
             let saved = cell;
             const code = cell.build();
             let ok = not saved.value and cell.value;
             '(ok and ~<code>)
         }
         comptime fn produce_pat() -> <|- bool> {
             var cell = Cell { value: true };
             let saved = cell;
             const pattern: <bool -| matched: bool> = cell.pat();
             let ok = saved.value and not cell.value;
             '(ok and (true match { ~<pattern> => matched, _ => false }))
         }
         fn run() -> bool { ~<produce_code()> and ~<produce_pat()> }",
    );
    assert_eq!(
        luna::execution::interpret(&valid, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
