//! Ordinary staged Type values bind contextual signatures, without becoming
//! clause arguments or causing argument producers to execute during inference.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;
use support::run_iir as run;

#[test]
fn ordinary_types_flow_through_expression_pattern_and_data_calls() {
    let source = "typealias Type=.Type'builtin;
        comptime fn identity_code(comptime T:Type)-><x:T|-T> {'(x)}
        comptime fn binding_code(comptime T:Type)-><T -| value:T> {'<value>}
        comptime fn inspect(comptime T:Type, code:<x:T|-T>, pat:<T -| value:T>, value:T)->T {value}
        comptime fn emit(comptime T:Type,value:T)-><|-T> {
            const result=inspect(T,identity_code(T),binding_code(T),value);'(result)
        }
        fn pick(comptime T:Type, item:T)->T {
            const code=identity_code(T);let result=~[x=item]<code>;
            let ~<binding_code(T)> = result;value
        }
        fn run()->bool {pick(bool,true) and pick(.char'builtin,'x') == 'x'
            and ~<emit(bool,true)> and ~<emit(.char'builtin,'y')> == 'y'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn ordinary_type_arguments_follow_source_order_and_are_evaluated_once() {
    let source = "typealias Type=.Type'builtin;typealias Count=.Integer'builtin;
        comptime fn select(implicit comptime T:Type,implicit value:T)-><|-T> {'(value)}
        comptime fn emit()-><|-bool> {
            var step:Count=0;
            const first=select(.value=do {step=step * 10 + 1;true},.T=do {step=step * 10 + 2;bool});
            let first_ok=step == 12;
            const second=select(.T=do {step=step * 10 + 3;bool},.value=do {step=step * 10 + 4;true});
            let second_ok=step == 1234;
            let result=first_ok and second_ok;'(result)
        }
        fn run()->bool {~<emit()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn inferred_context_cannot_override_an_explicit_type_value() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn select(implicit comptime T:Type,implicit value:T)-><|-T> {'(value)}";
    for call in ["select(.T=bool,.value='a')", "select(.value='a',.T=bool)"] {
        let source = format!("{prefix} fn run()->bool {{const code={call};true}}");
        let failure = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
}

#[test]
fn contextual_type_parameters_do_not_allow_runtime_values_into_generation() {
    for body in [
        "fn run(T:Type)->bool {const code=select(T);true}",
        "fn run(value:bool)->bool {const code=select(if value {bool} else {.char'builtin});true}",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;comptime fn select(comptime T:Type)-><x:T|-T> {{'(x)}} {body}"
        );
        let failure = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
}

#[test]
fn repeated_type_parameters_and_clause_inputs_keep_distinct_identities() {
    let source="typealias Type=.Type'builtin;
        comptime fn choose(comptime T:Type,comptime U:Type,input:T,other:V)-><|-T> where V {'(input)}
        comptime fn forward(comptime U:Type,comptime T:Type,value:U)-><|-U> {choose(U,T,value,true)}
        fn run()->bool {~<forward(bool,.char'builtin,true)> and ~<forward(.char'builtin,bool,'z')> == 'z'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn trait_methods_transport_staged_type_inputs_to_selected_bodies() {
    let source = "typealias Type=.Type'builtin;
        trait Builder {
            comptime fn expression(comptime T:Type)-><x:T|-T>;
            comptime fn pattern(comptime T:Type)-><T -| value:T>;
            comptime fn inspect(comptime T:Type,code:<x:T|-T>)->bool;
        }
        struct Reader {}
        impl Builder for Reader {
            comptime fn expression(comptime U:Type)-><x:U|-U> {'(x)}
            comptime fn pattern(comptime U:Type)-><U -| value:U> {'<value>}
            comptime fn inspect(comptime U:Type,code:<x:U|-U>)->bool {true}
        }
        fn run()->bool {
            const code=Reader.as(Builder).expression(bool);
            const ok=Reader.as(Builder).inspect(bool,code);
            let input=true;let result=~[x=input]<code>;
            let ~<Reader.as(Builder).pattern(bool)> = result;
            ok and value
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn recursive_generators_freshen_type_inputs_without_capturing_outer_slots() {
    let source = "typealias Type=.Type'builtin;typealias Count=.Integer'builtin;
        comptime fn recurse(comptime T:Type,count:Count)-><x:T|-T> {
            if count == 0 {'(x)} else {recurse(T,count - 1)}
        }
        fn pick(comptime T:Type,item:T)->T {~[x=item]<recurse(T,3)>}
        fn run()->bool {pick(bool,true) and pick(.char'builtin,'r') == 'r'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_values_are_evaluated_through_calls_and_literal_spreads() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Choose(flag:bool)->Type {if flag {bool} else {.char'builtin}}
        comptime fn select_value(comptime T:Type,value:T)-><|-T> {'(value)}
        fn run()->bool {~<select_value(Choose(true),true)> and ~<select_value(...(Choose(false),'s'))> == 's'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn contextual_method_implementation_cannot_replace_a_dependent_interface() {
    let source = "typealias Type=.Type'builtin;
        trait Builder {comptime fn code(comptime T:Type)-><x:T|-T>;}
        struct Reader {}
        impl Builder for Reader {comptime fn code(comptime U:Type)-><x:bool|-bool> {'(x)}}
        fn run()->bool {true}";
    let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
}

#[test]
fn open_code_bindings_retain_callable_declaration_domains_for_phase_checks() {
    let source = "typealias Type=.Type'builtin;
        fn foreach(f:%c fn(bool)) where c:Type.vars.Closure {f(true);}
        comptime fn identity_code(comptime T:Type)-><x:T|-T> {'(x)}
        fn pick(comptime T:Type,item:T)->T {const code=identity_code(T);~[x=item]<code>}
        fn run()->bool {pick(bool,true)}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}
