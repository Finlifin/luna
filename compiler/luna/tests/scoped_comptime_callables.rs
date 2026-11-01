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

#[test]
fn scoped_default_restores_captured_type_and_executable_body() {
    let source = "typealias Type = .Type'builtin;
        comptime fn callback(comptime T:Type)->%{saved:Type} comptime fn(T)->T {
            let saved=T;
            |value:T| {let observed=saved; value}
        }
        comptime fn dispatch(comptime T:Type,
            comptime .action:%{saved:Type} comptime fn(T)->T=callback(T), value:T)->T {
            action(value)
        }
        const answer=dispatch(bool,true);
        comptime fn run()->bool {answer}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn scoped_default_restores_captured_types_and_keeps_mutable_state_between_calls() {
    let source = r#"typealias Type = .Type'builtin;
        typealias char = .char'builtin;

comptime fn capturing_callback(comptime T: Type)
    -> %{flag: bool, saved: Type} comptime fn() -> Type {
    var flag = false;
    let saved = T;
    || { flag = not flag; if flag { saved } else { bool } }
}

comptime fn check_captured(comptime T: Type,
    comptime .action: %{flag: bool, saved: Type} comptime fn() -> Type = capturing_callback(T)) -> bool {
    action() == T and action() == bool and action() == T
}

        const a = check_captured(bool);
        const b = check_captured(char);
        comptime fn run()->bool {a and b}
    "#;
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn materialized_callable_signatures_share_cores_without_merging_identities() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        fn first(value:bool)->bool {value}
        fn second(value:bool)->bool {not value}
        pure comptime fn Carrier<action:fn(bool)->bool>->Type {bool}
        fn dispatch()->bool where T {true}
        fn run()->bool {dispatch<Carrier<first>>() and dispatch<Carrier<second>>()}",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert_eq!(graph.callable_identities.len(), 2);
    let [first, second] = graph.callable_identities.as_slice() else {
        unreachable!()
    };
    assert_ne!(first.key, second.key);
    assert!(std::sync::Arc::ptr_eq(
        first.signature.core(),
        second.signature.core()
    ));
    assert!(first.signature.arguments().is_empty());
    let cloned = graph.clone();
    assert!(std::sync::Arc::ptr_eq(
        first.signature.core(),
        cloned.callable_identities[0].signature.core()
    ));
    graph.validate(&mut 100_000).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn staged_method_defaults_keep_interface_identity_through_freezing() {
    let source = include_str!("../../../development/repros/staged-method-default-freeze.fl");
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let key = iir::InstanceKey {
        declaration: support::id(&package, "run"),
        arguments: vec![].into(),
    };
    let mut graph = elaboration::materialization::instantiate(
        &package,
        &[key],
        Default::default(),
        elaboration::ptc::Budget::default(),
    )
    .unwrap();
    assert!(
        !graph.templates.is_empty(),
        "frozen method targets must survive specialization"
    );
    graph.dispatch.clear();
    assert!(
        graph.validate(&mut 100_000).is_err(),
        "a frozen method cannot outlive its dispatch evidence"
    );
}

#[test]
fn unstaged_runtime_method_defaults_preserve_the_selected_contract() {
    let source = "typealias Type = .Type'builtin;
        trait Selector { fn select() -> bool; }
        impl Selector for bool { fn select() -> bool { true } }
        comptime fn callback() -> fn() -> bool where T { bool.select }
        fn dispatch(comptime T: Type, comptime .action: fn() -> bool = callback<T>()) -> bool { action() }
        fn run() -> bool { dispatch(bool) }";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn normal_form_method_inputs_keep_their_interface_origin() {
    let source = "typealias Type = .Type'builtin;
        trait First { fn first(comptime value: bool) -> bool; }
        trait Second { fn second(comptime value: bool) -> bool; }
        impl First for bool { fn first(comptime value: bool) -> bool { true } }
        impl Second for bool { fn second(comptime value: bool) -> bool { true } }
        pure comptime fn Carrier<action: fn(comptime bool) -> bool> -> Type { bool }
        comptime fn selected() -> fn(comptime bool) -> bool { bool.first }
        comptime fn run() -> bool { Carrier<selected()> == Carrier<bool.first> }
        fn consume_first(value: Carrier<bool.first>) -> bool { true }
        fn consume_second(value: Carrier<bool.second>) -> bool { true }";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let parameter = |name: &str| {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let nir::Term::Function { parameters, .. } = &function.signature else {
            panic!("signature")
        };
        parameters[0].ty.clone()
    };
    assert_ne!(parameter("consume_first"), parameter("consume_second"));
    for name in ["consume_first", "consume_second"] {
        let ty = parameter(name);
        let nir::Term::NormalForm { arguments, .. } = ty else {
            panic!("NF head was lost")
        };
        assert!(matches!(&arguments[0], nir::Term::MethodValue(_)));
        let key = iir::InstanceKey {
            declaration: support::id(&package, name),
            arguments: vec![].into(),
        };
        elaboration::materialization::instantiate(
            &package,
            &[key],
            Default::default(),
            elaboration::ptc::Budget::default(),
        )
        .unwrap();
    }
}

#[test]
fn method_normal_form_inputs_restore_caller_type_arguments() {
    let source = "typealias Type = .Type'builtin; typealias char = .char'builtin;
        trait Mapper { fn map() -> bool where T; }
        impl Mapper for bool { fn map() -> bool where T { true } }
        pure comptime fn Carrier<action: fn() -> bool> -> Type { bool }
        fn generic(comptime T: Type, value: Carrier<bool.map<T>>) -> bool { true }
        fn concrete(value: Carrier<bool.map<bool>>) -> bool { true }
        fn other(value: Carrier<bool.map<char>>) -> bool { true }";
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
            panic!("signature")
        };
        parameters[index].ty.clone()
    };
    for (name, primitive) in [
        ("concrete", nir::Primitive::Bool),
        ("other", nir::Primitive::Char),
    ] {
        assert_eq!(
            input("generic", 1)
                .substitute(&[nir::Term::Primitive(primitive)])
                .unwrap(),
            input(name, 0)
        );
    }
    assert_ne!(input("concrete", 0), input("other", 0));
}

#[test]
fn ordinary_type_inputs_prepare_demanded_method_identities() {
    let source = include_str!("../../../development/repros/staged-type-input-method-result.fl");
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_type_inputs_select_methods_for_each_receiver_type() {
    let source = "typealias Type = .Type'builtin; typealias char = .char'builtin;
        trait Selector { fn select(comptime value: bool) -> bool; }
        impl Selector for bool { fn select(comptime value: bool) -> bool { true } }
        impl Selector for char { fn select(comptime value: bool) -> bool { false } }
        comptime fn callback(comptime T: Type) -> fn(comptime bool) -> bool
        where requires T :- Selector
        { T.as(Selector).select }
        fn dispatch_bool(comptime .action: fn(comptime bool) -> bool = callback(bool)) -> bool { action(true) }
        fn dispatch_char(comptime .action: fn(comptime bool) -> bool = callback(char)) -> bool { action(true) }
        fn run() -> bool { dispatch_bool() and not dispatch_char() and dispatch_bool() }";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
