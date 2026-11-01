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

#[test]
fn completed_generic_result_domains_cannot_hide_pointer_storage() {
    for expression in ["absent<*bool>()", "absent<Wrapper<*bool>>()"] {
        let source = format!(
            "struct Wrapper where T {{value:T}}
            comptime fn absent()->?T where T {{null}}
            comptime fn run()->bool {{let value={expression};true}}"
        );
        let package = support::compile(&source);
        for tree in [false, true] {
            let mut vm = if tree {
                comptime_interpretor::Interpreter::for_comptime_tree(&package, Default::default())
            } else {
                comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
            }
            .unwrap();
            let error = vm.invoke(support::id(&package, "run"), vec![]).unwrap_err();
            assert_eq!(
                error.kind,
                comptime_interpretor::ErrorKind::PhaseMismatch,
                "{expression}: {error:?}"
            );
        }
    }
}

#[test]
fn dependent_input_domains_reject_absent_pointer_payloads_and_recover() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        comptime fn accept(comptime T:Type,value:T)->bool {true}
        comptime fn good()->bool {true}",
    );
    for tree in [false, true] {
        let mut vm = if tree {
            comptime_interpretor::Interpreter::for_comptime_tree(&package, Default::default())
        } else {
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
        }
        .unwrap();
        let domain = nir::Term::Optional(Box::new(nir::Term::Pointer(Box::new(
            nir::Term::Primitive(nir::Primitive::Bool),
        ))));
        let error = vm
            .invoke(
                support::id(&package, "accept"),
                vec![
                    Value::Type(domain.clone()),
                    Value::Optional {
                        ty: domain,
                        value: None,
                    },
                ],
            )
            .unwrap_err();
        assert_eq!(error.kind, comptime_interpretor::ErrorKind::PhaseMismatch);
        assert_eq!(
            vm.invoke(support::id(&package, "good"), vec![]).unwrap(),
            Value::Bool(true)
        );
        let domain = nir::Term::Optional(Box::new(nir::Term::Primitive(nir::Primitive::Bool)));
        assert_eq!(
            vm.invoke(
                support::id(&package, "accept"),
                vec![
                    Value::Type(domain.clone()),
                    Value::Optional {
                        ty: domain,
                        value: None
                    }
                ]
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn generic_local_error_domains_check_inactive_branches_and_recover() {
    let package = support::compile(
        "comptime fn success()->bool where E {let value: !E bool = true; true}
        comptime fn failure()->bool where T {let value: !bool T = error false; true}
        comptime fn bad_success()->bool {success<*bool>()}
        comptime fn bad_failure()->bool {failure<*bool>()}
        comptime fn good()->bool {success<bool>() and failure<bool>()}",
    );
    for tree in [false, true] {
        let mut vm = if tree {
            comptime_interpretor::Interpreter::for_comptime_tree(&package, Default::default())
        } else {
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
        }
        .unwrap();
        for name in ["bad_success", "bad_failure"] {
            let error = vm.invoke(support::id(&package, name), vec![]).unwrap_err();
            assert_eq!(error.kind, comptime_interpretor::ErrorKind::PhaseMismatch);
            assert_eq!(
                vm.invoke(support::id(&package, "good"), vec![]).unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn generic_local_enum_domains_check_inactive_variants_and_recover() {
    let package = support::compile(
        "enum Packet where T {empty, some(T)}
        enum Phantom where T {empty, some(bool)}
        comptime fn discard()->bool where T {let value=Packet<T>.empty; true}
        comptime fn bad()->bool {discard<*bool>()}
        comptime fn good()->bool {let phantom=Phantom<*bool>.empty; discard<bool>()}
        fn runtime()->bool {let value=Packet<*bool>.empty; true}",
    );
    let mut runtime = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    assert_eq!(
        runtime
            .invoke(support::id(&package, "runtime"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
    for tree in [false, true] {
        let mut vm = if tree {
            comptime_interpretor::Interpreter::for_comptime_tree(&package, Default::default())
        } else {
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
        }
        .unwrap();
        let error = vm.invoke(support::id(&package, "bad"), vec![]).unwrap_err();
        assert_eq!(error.kind, comptime_interpretor::ErrorKind::PhaseMismatch);
        assert_eq!(
            vm.invoke(support::id(&package, "good"), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}
