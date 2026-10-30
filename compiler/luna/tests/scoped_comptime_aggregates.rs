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

#[test]
fn type_contracts_can_invoke_descriptors_with_explicit_type_inputs() {
    let source = "typealias Type=.Type'builtin;
        struct Descriptor {item:Type}
        pure comptime fn Carrier<value:Descriptor>->Type {bool}
        comptime fn describe(T:Type)->Descriptor {Descriptor {item:T}}
        fn identity(comptime T:Type,value:T)->T
        where requires Carrier<describe(T)> == Carrier<Descriptor {item:T}>
        {value}
        fn run()->bool {identity(bool,true)}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn descriptor_contracts_preserve_boolean_structure_and_reject_wrong_types() {
    for (predicate, good_ty, good_value, bad_ty, bad_value) in [
        (
            "Carrier<describe(T)> == Carrier<describe(bool)>",
            "bool",
            "true",
            "char",
            "'x'",
        ),
        (
            "not (T == bool) and Carrier<describe(T)> == Carrier<describe(T)>",
            "char",
            "'x'",
            "bool",
            "true",
        ),
        (
            "Carrier<describe(T)> == Carrier<describe(bool)> or T == char",
            "char",
            "'x'",
            "Byte",
            "7",
        ),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;typealias char=.char'builtin;typealias Byte=.u8'builtin;
            struct Descriptor {{item:Type}}
            pure comptime fn Carrier<value:Descriptor>->Type {{bool}}
            comptime fn describe(T:Type)->Descriptor {{Descriptor {{item:T}}}}
            fn selected(comptime T:Type,value:T)->T where requires {predicate} {{value}}
            fn run()->{good_ty} {{selected({good_ty},{good_value})}}"
        );
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|e| panic!("{source}\n{e}"));
        let selected = package
            .functions
            .iter()
            .find(|f| f.name == "selected")
            .unwrap();
        assert!(!selected.requirements.is_empty());
        assert!(selected.preconditions.is_empty());
        assert!(luna::execution::interpret(&package, "run", 100_000).is_ok());
        let rejected = source.replace(
            &format!("run()->{good_ty} {{selected({good_ty},{good_value})}}"),
            &format!("run()->{bad_ty} {{selected({bad_ty},{bad_value})}}"),
        );
        assert!(
            support::compile_with_protocols(&rejected, &Default::default()).is_err(),
            "{rejected}"
        );
    }
}

#[test]
fn descriptor_contracts_keep_distinct_parameters_and_method_receiver_positions() {
    let source = "typealias Type=.Type'builtin;typealias char=.char'builtin;
        struct Descriptor {item:Type} struct Gate {}
        pure comptime fn Carrier<value:Descriptor>->Type {bool}
        comptime fn describe(T:Type)->Descriptor {Descriptor {item:T}}
        impl Gate {
            fn same(self,comptime A:Type,comptime B:Type)->bool
            where requires Carrier<describe(A)> == Carrier<describe(B)>
            {true}
        }
        fn run()->bool {(Gate {}).same(bool,bool)}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert!(
        support::compile_with_protocols(
            &source.replace("same(bool,bool)", "same(bool,char)"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn descriptor_contracts_do_not_assume_the_condition_being_computed() {
    let source = "typealias Type=.Type'builtin;
        struct Descriptor {item:Type}
        pure comptime fn Carrier<value:Descriptor>->Type {bool}
        comptime fn describe(T:Type)->Descriptor {
            if T == bool {Descriptor {item:bool}} else {Descriptor {item:T}}
        }
        fn selected(comptime T:Type,value:T)->T
        where requires Carrier<describe(T)> == Carrier<describe(bool)>
        {value}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("TypeContractUnresolved"), "{error}");
    let runtime = source.replace("comptime T:Type,value", "T:Type,value");
    assert!(support::compile_with_protocols(&runtime, &Default::default()).is_err());
}

#[test]
fn descriptor_contracts_do_not_infer_factory_injectivity() {
    // Different arguments do not prove unequal results for an opaque factory.
    let source = "typealias Type=.Type'builtin;typealias char=.char'builtin;
        struct Descriptor {item:Type}
        pure comptime fn Carrier<value:Descriptor>->Type {bool}
        comptime fn describe(T:Type)->Descriptor {Descriptor {item:T}}
        fn selected(comptime T:Type,value:T)->T
        where requires Carrier<describe(T)> != Carrier<describe(bool)>
        {value}
        fn run()->char {selected(char,'x')}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("requirement is not established"), "{error}");
    assert!(error.contains("unknown"), "{error}");
}
