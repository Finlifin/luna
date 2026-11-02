//! Definition-time template inputs retain ordinary clause binding semantics.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn explicit_template_types_are_fixed_at_definition_formation() {
    let source = "template fn identity(value:T)->T where T;
        fn keep use identity<bool> {value}
        quote fn invert use identity<bool> {not value}
        fn run()->bool {keep(true) and invert(false)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("keep(true)", "keep<bool>(true)");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn applied_value_inputs_and_dependent_defaults_stay_instance_local() {
    let source = "template fn choice(value:bool)->bool
        where .inverted:bool=false, .again:bool=inverted;
        fn flip use choice<.inverted=true> {value != again}
        fn keep use choice<> {value != again}
        fn run()->bool {flip(false) and keep(true) and not flip(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn applied_types_reach_shared_body_contexts_and_defaults() {
    let source = "template fn shared(value:T,quote lambda body:<current:T |- T>)->T
        where T
        {let current=value;~<body>}
        fn first use shared<bool> {not current}
        fn second use shared<bool> {current}
        fn run()->bool {first(false) and second(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn applied_template_requirements_are_checked_even_for_unused_instances() {
    let valid = "template fn interface(value:T)->T where T, requires T == bool;
        fn keep use interface<bool> {value}
        fn run()->bool {keep(true)}";
    assert_eq!(support::run_iir(valid, "run", vec![]), Value::Bool(true));
    let invalid = "typealias char=.char'builtin;
        template fn interface(value:T)->T where T, requires T == bool;
        fn unused use interface<char> {value}";
    let error = support::compile_with_protocols(invalid, &Default::default()).unwrap_err();
    assert!(
        error.contains("precondition") || error.contains("requirement"),
        "{error}"
    );
}

#[test]
fn computed_template_inputs_cannot_assume_the_unformed_interface_contract() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Flag()->Type {bool}
        template fn identity(value:T)->U where T,.U:Type=T,requires T == U;
        fn keep use identity<Flag()> {value}
        fn run()->bool {keep(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nested_template_applications_use_the_enclosing_staged_inputs() {
    let source = "typealias Type=.Type'builtin;
        template fn identity(value:T)->T where T;
        fn outer(comptime U:Type, input:U)->U {
            fn nested use identity<U> {value}
            nested(input)
        }
        fn run()->bool {outer(bool,true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nested_template_inputs_preserve_value_domains_and_outer_evidence() {
    let source = "typealias Type=.Type'builtin;
        template fn choose(value:T)->T where T,fallback:T;
        fn outer(comptime U:Type, comptime fallback:U, input:U)->U {
            fn nested use choose<U,fallback> {fallback}
            nested(input)
        }
        template fn constrained(value:T)->T where T, requires T == bool;
        fn checked(comptime U:Type, input:U)->U
        where requires U == bool
        {
            fn nested use constrained<U> {value}
            nested(input)
        }
        fn run()->bool {
            outer(bool,true,false) and not outer(bool,false,true)
                and checked(bool,true)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for invalid in [
        source.replace("comptime fallback:U", "fallback:U"),
        source.replace("where requires U == bool", ""),
    ] {
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn template_application_reuses_type_defaults_and_preserves_generic_instances() {
    let source = "typealias Type=.Type'builtin;
        template fn pair(first:T, second:U)->U where T, .U:Type=T;
        fn fixed use pair<bool> {second}
        fn generic use pair {second}
        fn run()->bool {fixed(false,true) and generic<bool>(false,true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn template_value_inputs_include_callable_and_optional_values() {
    let source = "fn negate(value:bool)->bool {not value}
        template fn apply(value:bool)->bool where .action:fn(bool)->bool=negate;
        fn flip use apply<> {action(value)}
        template fn option()->bool where .flag:?bool=null;
        fn absent use option<> {flag match {value? => value,null => false}}
        fn present use option<.flag=true> {flag match {value? => value,null => false}}
        fn run()->bool {flip(false) and not absent() and present()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn template_inputs_reject_bad_binding_and_preserve_opaque_heads() {
    let prefix = "typealias Type=.Type'builtin;typealias char=.char'builtin;
        pure comptime fn Identity<T>->Type {T}
        template fn interface(value:T)->T where T;";
    for body in [
        "fn bad use interface<> {value}",
        "fn bad use interface<true> {value}",
        "fn bad use interface<bool,bool> {value}",
        "fn bad use interface<.missing=bool> {value}",
        "fn bad use interface<bool,.T=bool> {value}",
        "fn bad use interface<Identity<bool>> {true}",
    ] {
        let source = format!("{prefix}{body}");
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{body}"
        );
    }
    let source = format!("{prefix}fn keep use interface<Identity<bool>> {{value}}");
    let package = support::compile_with_protocols(&source, &Default::default()).unwrap();
    let function = package
        .functions
        .iter()
        .find(|function| function.name == "keep")
        .unwrap();
    let nir::Term::Function { parameters, result } = &function.signature else {
        panic!("function")
    };
    assert!(matches!(result.as_ref(), nir::Term::NormalForm { .. }));
    assert_eq!(&parameters[0].ty, result.as_ref());
    assert!(function.type_parameters.is_empty());
}
