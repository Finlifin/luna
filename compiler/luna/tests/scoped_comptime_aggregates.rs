//! Required invocation results retain the caller's type telescope in data.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use nir::{Primitive, Term};

#[test]
fn required_aggregate_results_restore_the_caller_telescope() {
    for (definition, domain, value) in [
        (
            "struct Descriptor { item:Type }",
            "Descriptor",
            "Descriptor {item:T}",
        ),
        (
            "struct Descriptor where U { item:Type }",
            "Descriptor<T>",
            "Descriptor<T> {item:T}",
        ),
        (
            "enum Descriptor {empty, item(Type)}",
            "Descriptor",
            "Descriptor.item(T)",
        ),
        ("newtype Descriptor=Type;", "Descriptor", "T.as(Descriptor)"),
        ("", "?Type", "T"),
        ("", "!bool Type", "T"),
        (
            "struct Descriptor { item:?Type }",
            "Descriptor",
            "Descriptor {item:T}",
        ),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;typealias char=.char'builtin;
            {definition}
            pure comptime fn Carrier<T,value:{domain}>->Type {{bool}}
            comptime fn describe(comptime T:Type)->{domain} {{{value}}}
            fn generic(comptime T:Type,value:Carrier<T,describe(T)>)->bool {{true}}
            fn concrete(value:Carrier<bool,describe(bool)>)->bool {{true}}
            fn other(value:Carrier<char,describe(char)>)->bool {{true}}"
        );
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{source}\n{error}"));
        let parameter = |name: &str, index: usize| {
            let signature = &package
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .signature;
            signature.check_scope(&[]).unwrap();
            let Term::Function { parameters, .. } = signature else {
                panic!("function signature");
            };
            parameters[index].ty.clone()
        };
        let open = parameter("generic", 1);
        assert_eq!(
            open.substitute(&[Term::Primitive(Primitive::Bool)])
                .unwrap(),
            parameter("concrete", 0)
        );
        assert_eq!(
            open.substitute(&[Term::Primitive(Primitive::Char)])
                .unwrap(),
            parameter("other", 0)
        );
        assert_ne!(parameter("concrete", 0), parameter("other", 0));
    }
}

#[test]
fn required_aggregate_observation_does_not_guess_unknown_types() {
    let source = "typealias Type=.Type'builtin;
        struct Descriptor {item:Type}
        pure comptime fn Carrier<value:Descriptor>->Type {bool}
        comptime fn describe(T:Type)->Descriptor {
            if T == bool {Descriptor {item:bool}} else {Descriptor {item:T}}
        }
        fn generic(comptime T:Type,value:Carrier<describe(T)>)->bool {true}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("TypeContractUnresolved"), "{error}");
    let runtime = source.replace("comptime T:Type,value", "T:Type,value");
    assert!(support::compile_with_protocols(&runtime, &Default::default()).is_err());
}

#[test]
fn aggregate_defaults_specialize_through_runtime_calls() {
    let source = "typealias Type=.Type'builtin;
        struct Descriptor {item:Type}
        pure comptime fn Carrier<value:Descriptor>->Type {bool}
        comptime fn describe(T:Type)->Descriptor {Descriptor {item:T}}
        fn identity(comptime T:Type,comptime .description:Descriptor=defer describe(T),value:T)->T
        {value}
        fn run()->bool {identity(bool,true)}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let identity = graph
        .functions
        .iter()
        .find(|f| f.function.name == "identity")
        .unwrap();
    assert!(
        identity
            .key
            .arguments
            .iter()
            .any(|argument| matches!(argument,
        Term::NominalValue { fields, .. } if fields == &[Term::Primitive(Primitive::Bool)]))
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}
