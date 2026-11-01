//! Comptime storage checks inspect payloads, not runtime type descriptions.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn unused_comptime_domains_reject_stored_pointer_payloads() {
    for source in [
        "comptime fn unused(value:*bool)->bool {true}",
        "comptime fn unused(value:?*bool)->bool {true}",
        "comptime fn unused(value:%{pointer:*bool} comptime fn()->bool)->bool {true}",
        "struct Packet where T {value:T} comptime fn unused(value:Packet<*bool>)->bool {true}",
        "enum Packet {empty,some(*bool)} comptime fn unused(value:Packet)->bool {true}",
        "newtype Address=*bool; comptime fn unused(value:Address)->bool {true}",
        "comptime fn unused()->?*bool {null}",
        "fn unused(comptime value:*bool)->bool {true}",
    ] {
        match support::compile_with_protocols(source, &Default::default()) {
            Err(error) => assert!(
                error.contains("runtime pointer values cannot inhabit comptime storage"),
                "{source}: {error}"
            ),
            Ok(package) => {
                let error = nir::phases::validate(&package).unwrap_err();
                assert!(
                    error
                        .message
                        .contains("runtime pointer values cannot inhabit comptime storage"),
                    "{source}: {error:?}"
                );
            }
        }
    }
}

#[test]
fn comptime_domains_distinguish_phantom_types_and_runtime_signatures_from_payloads() {
    let source="typealias Type=.Type'builtin;
        struct Phantom where T {flag:bool}
        comptime fn inspect(value:Phantom<*bool>,code:<p:*bool|-*bool>,callback:fn(*bool)->*bool)->bool {value.flag}
        comptime fn Description()->Type {*bool}
        comptime fn run()->bool {true}";
    let package = support::compile(source);
    nir::phases::validate(&package).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn collected_domains_inspect_elements_but_not_type_value_contents() {
    for domain in ["Tuple<bool,*bool>", "TupleElements<*bool>"] {
        let source = format!("comptime fn unused(value:{domain})->bool {{true}}");
        match support::compile_with_tuple_protocols(&source, &Default::default()) {
            Err(error) => assert!(
                error.contains("runtime pointer values cannot inhabit comptime storage"),
                "{error}"
            ),
            Ok(package) => assert!(
                nir::phases::validate(&package)
                    .unwrap_err()
                    .message
                    .contains("runtime pointer values cannot inhabit comptime storage")
            ),
        }
    }
    let package=support::compile_with_tuple("typealias Type=.Type'builtin;
        comptime fn descriptions()->Tuple<Type,Type> {(bool,*bool)}
        comptime fn run()->bool {let (first,second)=descriptions();first == bool and second == *bool}");
    nir::phases::validate(&package).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
