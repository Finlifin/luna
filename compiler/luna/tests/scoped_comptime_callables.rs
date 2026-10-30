//! Staged callable identities preserve caller Type bindings without executing code.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn returned_runtime_function_identities_supply_scoped_defaults() {
    let source = "typealias Type=.Type'builtin;
        fn identity(value:T)->T where T {value}
        comptime fn callback(comptime T:Type)->fn(T)->T {identity<T>}
        fn dispatch(comptime T:Type,comptime .action:fn(T)->T=callback(T),value:T)->T {action(value)}
        fn run()->bool {dispatch(bool,true)}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn returned_comptime_function_instances_supply_scoped_defaults() {
    let source = "typealias Type=.Type'builtin;
        comptime fn identity(value:T)->T where T {value}
        comptime fn callback(comptime T:Type)->comptime fn(T)->T {identity<T>}
        comptime fn dispatch(comptime T:Type,comptime .action:comptime fn(T)->T=callback(T),value:T)->T {action(value)}
        const answer=dispatch(bool,true);
        comptime fn run()->bool {answer}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn runtime_function_values_preserve_scoped_normal_form_arguments() {
    let source = "typealias Type=.Type'builtin;typealias char=.char'builtin;
        fn identity(value:T)->T where T {value}
        comptime fn callback(comptime T:Type)->fn(T)->T {identity<T>}
        pure comptime fn Carrier<T,action:fn(T)->T>->Type {bool}
        fn generic(comptime T:Type,value:Carrier<T,callback(T)>)->bool {true}
        fn concrete(value:Carrier<bool,callback(bool)>)->bool {true}
        fn other(value:Carrier<char,callback(char)>)->bool {true}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    let parameter = |name: &str, index: usize| {
        let signature = &package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .signature;
        signature.check_scope(&[]).unwrap();
        let nir::Term::Function { parameters, .. } = signature else {
            panic!("function signature");
        };
        parameters[index].ty.clone()
    };
    let open = parameter("generic", 1);
    assert_eq!(
        open.substitute(&[nir::Term::Primitive(nir::Primitive::Bool)])
            .unwrap(),
        parameter("concrete", 0)
    );
    assert_eq!(
        open.substitute(&[nir::Term::Primitive(nir::Primitive::Char)])
            .unwrap(),
        parameter("other", 0)
    );
    assert_ne!(parameter("concrete", 0), parameter("other", 0));
}

#[test]
fn comptime_function_values_preserve_scoped_normal_form_arguments() {
    let source = "typealias Type=.Type'builtin;typealias char=.char'builtin;
        comptime fn identity(value:T)->T where T {value}
        comptime fn callback(comptime T:Type)->comptime fn(T)->T {identity<T>}
        pure comptime fn Carrier<T,action:comptime fn(T)->T>->Type {bool}
        fn generic(comptime T:Type,value:Carrier<T,callback(T)>)->bool {true}
        fn concrete(value:Carrier<bool,callback(bool)>)->bool {true}
        fn other(value:Carrier<char,callback(char)>)->bool {true}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    let input = |name: &str, index: usize| {
        let signature = &package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .signature;
        signature.check_scope(&[]).unwrap();
        let nir::Term::Function { parameters, .. } = signature else {
            panic!("function signature");
        };
        parameters[index].ty.clone()
    };
    let open = input("generic", 1);
    for (name, ty) in [
        ("concrete", nir::Primitive::Bool),
        ("other", nir::Primitive::Char),
    ] {
        assert_eq!(
            open.substitute(&[nir::Term::Primitive(ty)]).unwrap(),
            input(name, 0)
        );
    }
    assert_ne!(input("concrete", 0), input("other", 0));
}

#[test]
fn invocation_constructs_normal_forms_with_callable_inputs() {
    for qualifier in ["", "comptime", "unsafe"] {
        let source = "typealias Type=.Type'builtin;
        fn identity(value:T)->T where T {value}
        comptime fn callback(comptime T:Type)->fn(T)->T {identity<T>}
        pure comptime fn Carrier<T,action:fn(T)->T>->Type {bool}
        comptime fn Describe(comptime T:Type)->Type {Carrier<T,callback(T)>}
        comptime fn run()->bool {Describe(bool) == Carrier<bool,callback(bool)>}";
        let source = source
            .replace("fn identity", &format!("{qualifier} fn identity"))
            .replace("fn(T)", &format!("{qualifier} fn(T)"));
        let package = support::compile_with_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn runtime_instances_retain_function_valued_type_arguments() {
    for qualifier in ["", "comptime", "unsafe"] {
        let source = "typealias Type=.Type'builtin;
        fn identity(value:T)->T where T {value}
        pure comptime fn Carrier<T,action:fn(T)->T>->Type {bool}
        fn dispatch()->bool where T {true}
        fn run()->bool {dispatch<Carrier<bool,identity<bool>>>()}";
        let source = source
            .replace("fn identity", &format!("{qualifier} fn identity"))
            .replace("fn(T)", &format!("{qualifier} fn(T)"));
        let package = support::compile_with_protocols(&source, &Default::default()).unwrap();
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        assert_eq!(graph.callable_identities.len(), 1);
        let identity = &graph.callable_identities[0];
        assert!(
            !graph
                .functions
                .iter()
                .any(|function| function.key == identity.key)
        );
        assert!(
            graph
                .to_string()
                .contains("(callable-identity (instance-key")
        );
        let mut missing = graph.clone();
        missing.callable_identities.clear();
        assert!(missing.validate(&mut 100_000).is_err());
        let mut duplicate = graph.clone();
        duplicate.callable_identities.push(identity.clone());
        assert!(duplicate.validate(&mut 100_000).is_err());
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn invocation_preserves_callable_fields_inside_nominal_nf_inputs() {
    for qualifier in ["", "comptime"] {
        let source = "typealias Type=.Type'builtin;
        fn identity(value:T)->T where T {value}
        struct Descriptor where T {action:fn(T)->T}
        comptime fn describe(comptime T:Type)->Descriptor<T> {Descriptor<T> {action:identity<T>}}
        pure comptime fn Carrier<T,value:Descriptor<T>>->Type {bool}
        comptime fn DescribeType(comptime T:Type)->Type {Carrier<T,describe(T)>}
        fn generic(comptime T:Type,value:Carrier<T,describe(T)>)->bool {true}
        comptime fn run()->bool {DescribeType(bool) == Carrier<bool,describe(bool)>}";
        let source = source
            .replace("fn identity", &format!("{qualifier} fn identity"))
            .replace("fn(T)", &format!("{qualifier} fn(T)"));
        let package = support::compile_with_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn retained_callable_signatures_close_nominal_dependencies_without_bodies() {
    let source = "typealias Type=.Type'builtin;
        struct Payload {value:bool}
        fn identity(value:Payload)->Payload {value}
        pure comptime fn Carrier<action:fn(Payload)->Payload>->Type {bool}
        fn dispatch()->bool where T {true}
        fn run()->bool {dispatch<Carrier<identity>>()}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert_eq!(graph.callable_identities.len(), 1);
    assert_eq!(graph.nominals.len(), 1);
    assert!(
        !graph
            .functions
            .iter()
            .any(|function| function.function.name == "identity")
    );
    graph.validate(&mut 100_000).unwrap();
}
