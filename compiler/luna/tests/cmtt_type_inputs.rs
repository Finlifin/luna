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

#[test]
fn immutable_packets_preserve_type_values_in_contextual_calls() {
    let source = "typealias Type=.Type'builtin;
        comptime fn select_value(comptime T:Type,value:T)-><|-T> {'(value)}
        comptime fn forward(comptime T:Type,value:T)-><|-T> {
            const packet=(T,value);select_value(...packet)
        }
        fn run()->bool {~<forward(bool,true)> and ~<forward(.char'builtin,'p')> == 'p'}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn packet_type_evidence_flows_to_pattern_and_data_consumers() {
    let source = "typealias Type=.Type'builtin;
        comptime fn pattern(comptime T:Type)-><T -| value:T> {'<value>}
        comptime fn inspect(comptime T:Type,item:T,code:<|-bool>)->T {item}
        comptime fn forward(comptime T:Type,item:T)-><|-T> {
            const packet=(T,item);const result=inspect(...packet,'(true));'(result)
        }
        comptime fn binding(comptime T:Type)-><T -| value:T> {
            const packet=(T,);pattern(...packet)
        }
        fn run()->bool {let ~<binding(bool)> = ~<forward(bool,true)>;value}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn packet_evidence_does_not_execute_unknown_siblings() {
    let source = "typealias Type=.Type'builtin;typealias Count=.Integer'builtin;
        comptime fn select_value(comptime T:Type,value:T)-><|-T> {'(value)}
        comptime fn forward()-><|-bool> {
            var count:Count=0;
            const packet=(bool,do {count+=1;true});
            const code=select_value(...packet);
            let once=count == 1;'(~<code> and once)
        }
        fn run()->bool {~<forward()>}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn unknown_packet_type_values_are_not_inferred_from_peer_arguments() {
    let source = "typealias Type=.Type'builtin;
        comptime fn select_value(comptime T:Type,value:T)-><|-T> {'(value)}
        comptime fn forward(flag:bool)-><|-bool> {
            const packet=(if flag {bool} else {.char'builtin},true);
            select_value(...packet)
        }
        fn run()->bool {~<forward(true)>}";
    let failure = support::compile_with_tuple_protocols(source, &Default::default()).unwrap_err();
    assert!(
        failure.contains("dependent contextual input requires a completed Type value"),
        "{failure}"
    );
}

#[test]
fn ordinary_type_defaults_bind_expression_pattern_and_data_interfaces() {
    let source = "typealias Type=.Type'builtin;
        comptime fn identity(comptime .T:Type=bool)-><x:T|-T> {'(x)}
        comptime fn pattern(comptime .T:Type=bool)-><T -| value:T> {'<value>}
        comptime fn inspect(comptime .T:Type=bool,.code:<x:T|-T> = identity(.T=T))->bool {true}
        fn run()->bool {
            let input=true;let result=~[x=input]<identity()>;
            let ~<pattern()> = result;
            const ok=inspect();value and ok
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn ordinary_type_defaults_depend_on_preceding_inputs() {
    let source = "typealias Type=.Type'builtin;
        comptime fn identity(comptime A:Type,comptime .T:Type=A,
            comptime .U:Type=T)-><x:U|-U> {'(x)}
        comptime fn forward(comptime T:Type)-><x:T|-T> {identity(T)}
        fn pick(comptime T:Type,input:T)->T {~[x=input]<forward(T)>}
        fn run()->bool {pick(bool,true) and pick(.char'builtin,'d') == 'd'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_default_prefixes_execute_once_and_retain_updated_values() {
    let source = "typealias Type=.Type'builtin;typealias Count=.Integer'builtin;
        struct State {count:Count}
        comptime fn emit(state:State,
            .before:bool=do {state.count=state.count * 10 + 2;true},
            comptime .T:Type=do {state.count=state.count * 10 + 3;bool},
            .after:bool=do {state.count=state.count * 10 + 4;true})-><|-bool> {
            let result=state.count == 1234;'(result)
        }
        fn run()->bool {~<emit(State {count:1})>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn explicit_type_inputs_skip_their_defaults() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Forever()->Type {while true {} bool}
        comptime fn identity(comptime .T:Type=Forever())-><x:T|-T> {'(x)}
        fn run()->bool {let input=true;~[x=input]<identity(.T=bool)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_defaults_cannot_be_replaced_by_contextual_inference() {
    let source = "typealias Type=.Type'builtin;
        comptime fn identity(comptime .T:Type=bool,value:T)-><|-T> {'(value)}
        fn run()->bool {const code=identity('x');true}";
    let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
}

#[test]
fn code_defaults_before_type_inputs_preserve_caller_associations() {
    let source = "typealias Type=.Type'builtin;
        comptime fn forward(first:<x:bool|-bool>,
            .second:<x:bool|-bool> = first,comptime .T:Type=bool)-><x:bool|-bool> {second}
        fn run(flag:bool)->bool {
            const code=forward('[input=flag](flag));let flag=false;~<code>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
    assert_eq!(
        run(source, "run", vec![Value::Bool(false)]),
        Value::Bool(false)
    );
}

#[test]
fn trait_dispatch_uses_completed_ordinary_type_defaults() {
    let source = "typealias Type=.Type'builtin;
        trait Builder {
            comptime fn expression(comptime .T:Type=bool)-><x:T|-T>;
            comptime fn pattern(comptime .T:Type=bool)-><T -| value:T>;
            comptime fn inspect(comptime .T:Type=bool,
                .code:<x:T|-T> = '(x))->bool;
        }
        struct Reader {}
        impl Builder for Reader {
            comptime fn expression(comptime .U:Type=bool)-><x:U|-U> {'(x)}
            comptime fn pattern(comptime .U:Type=bool)-><U -| value:U> {'<value>}
            comptime fn inspect(comptime .U:Type=bool,
                .code:<x:U|-U> = '(x))->bool {true}
        }
        fn run()->bool {
            let input=true;
            let ~<Reader.as(Builder).pattern()> = ~[x=input]<Reader.as(Builder).expression()>;
            const ok=Reader.as(Builder).inspect();value and ok
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nested_required_calls_receive_ordinary_record_value_inputs() {
    let source = "typealias Type=.Type'builtin;
        struct Description {enabled:bool,kind:Type}
        pure comptime fn Marker<description:Description>->Type {bool}
        comptime fn inspect()->bool where T {T == Marker<Description {enabled:true,kind:bool}>}
        comptime fn emit()-><|-bool> {
            let description=Description {enabled:true,kind:bool};
            let result=inspect<Marker<description>>();'(result)
        }
        fn run()->bool {~<emit()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn contextual_inference_routes_ordinary_variadic_normal_form_values() {
    for (inputs, expected) in [
        ("bool,true,false", "bool,.items=[true,false]"),
        ("bool,...[true,false]", "bool,.items=[true,false]"),
        ("bool,.items=[true,false]", "bool,.items=[true,false]"),
        ("bool,.items=bits", "bool,.items=[true,false]"),
        ("bool", "bool,.items=[]"),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            pure comptime fn List<T>->Type {{T}}
            pure comptime fn Indexed<T,...items:List<T>>->Type {{while true {{}} bool}}
            struct Holder where T {{flag:bool}}
            comptime fn identity(value:Holder<T>)->Holder<T> where T {{value}}
            comptime fn build(lambda action:comptime fn(T)->bool,value:T)-><|-bool> where T {{
                let result=action(value);'(result)
            }}
            comptime fn outer()-><|-bool> {{
                let bits:List<bool> = [true,false];
                build(_0.flag, identity<Indexed<{inputs}>>(Holder<Indexed<{expected}>>{{flag:true}}))
            }}
            fn run()->bool {{~<outer()>}}"
        );
        let compile = |source: &str| {
            support::compile_with_source_bindings(
                source,
                &Default::default(),
                luna::protocols::type_constructors::Selectors {
                    meta_list: Some("input.List"),
                    ..Default::default()
                },
            )
        };
        let package = compile(&source).unwrap_or_else(|failure| panic!("{inputs}: {failure}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let wrong = source.replace(
            &format!("Holder<Indexed<{expected}>>{{flag:true}}"),
            "Holder<Indexed<bool,.items=[false,true]>>{flag:true}",
        );
        assert!(
            compile(&wrong).is_err(),
            "value indices must retain identity: {inputs}"
        );
    }
}
