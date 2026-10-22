//! Partial result information through computed ordinary argument packets.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;
use support::compile;

#[test]
fn exiting_method_packets_keep_completed_peer_layouts() {
    for (inputs, order) in [
        (
            "flag:mark(),...do {return true;Source {}.first()},...Source {}.last()",
            vec!["run", "mark"],
        ),
        (
            "...Source {}.first(),flag:mark(),...do {return true;Source {}.last()}",
            vec!["run", "first", "mark"],
        ),
        (
            "...do {return true;Source {}.first()},flag:mark(),...Source {}.last()",
            vec!["run"],
        ),
    ] {
        let source=format!("struct Source {{}} impl Source {{fn first(self)->(bool,) {{(true,)}} fn last(self)->(bool,) {{(false,)}}}}
            fn mark()->bool {{true}}
            fn take(...items:(bool,bool),...properties:P)->(bool,bool) where P {{items}}
            fn run()->bool {{take {{{inputs}}};false}}");
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{inputs}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), order, "{inputs}");
        let invalid = source.replace("Source {}.last()", "Source {}.missing()");
        assert!(
            support::compile_with_protocols(&invalid, &Default::default()).is_err(),
            "unchecked unreachable input: {inputs}"
        );
    }
}

#[test]
fn checked_method_packets_supply_property_remapping_widths() {
    for (inputs, order) in [
        (
            "flag:mark(),...Source {}.first(),...Source {}.last()",
            vec!["mark", "first", "last"],
        ),
        (
            "...Source {}.first(),flag:mark(),...Source {}.last()",
            vec!["first", "mark", "last"],
        ),
        (
            "...Source {}.first(),...Source {}.last(),flag:mark()",
            vec!["first", "last", "mark"],
        ),
        (
            "...Source {}.empty(),flag:mark(),...Source {}.first(),...Source {}.last()",
            vec!["empty", "mark", "first", "last"],
        ),
        (
            "...Source {}.first(),flag:mark(),...Source {}.empty(),...Source {}.last()",
            vec!["first", "mark", "empty", "last"],
        ),
    ] {
        let source = format!(
            "struct Source {{}} impl Source {{
            fn first(self)->(bool,) {{(true,)}}
            fn last(self)->(bool,) {{(false,)}}
            fn empty(self,...items:T)->T where T {{items}}
        }}
        fn mark()->bool {{true}}
        fn take(...items:(bool,bool),...properties:P)->(bool,bool) where P {{items}}
        fn run()->bool {{let (a,b)=take {{{inputs}}};a and not b}}"
        );
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{inputs}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        let calls = support::calls(&package, &vm);
        assert_eq!(
            calls
                .iter()
                .filter(|name| order.contains(&name.as_str()))
                .map(String::as_str)
                .collect::<Vec<_>>(),
            order,
            "{inputs}"
        );
        let wrong = source.replace(
            "fn last(self)->(bool,) {(false,)}",
            "fn last(self)->(bool,bool) {(false,true)}",
        );
        assert!(
            support::compile_with_protocols(&wrong, &Default::default()).is_err(),
            "accepted an extra packet field: {inputs}"
        );
    }
}

#[test]
fn open_packets_share_peer_evidence_before_staged_arguments_exist() {
    for entry in [
        "fn run()->bool {apply(true,...callbacks(|value| value==9),...values())}",
        "fn relay(comptime flag:bool)->bool {apply(flag,...callbacks(|value| value==9),...values())} fn run()->bool {relay(true) and relay(false)}",
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            fn apply(comptime flag:bool,action:fn(T)->U,value:T)->U where T,U {{
                inline if flag {{action(value)}} else {{action(value)}}
            }}
            fn callbacks(value:T)->(T,) where T {{(value,)}}
            fn values()->(Byte,) {{(9,)}}
            {entry}"
        );
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{entry}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        let events = support::calls(&package, &vm);
        let producers = events
            .iter()
            .filter(|name| matches!(name.as_str(), "callbacks" | "values"))
            .map(String::as_str)
            .collect::<Vec<_>>();
        let expected = if entry.contains("relay") {
            vec!["callbacks", "values", "callbacks", "values"]
        } else {
            vec!["callbacks", "values"]
        };
        assert_eq!(producers, expected, "{entry}");
        let runtime = source
            .replace("fn run()->bool", "fn run(flag:bool)->bool")
            .replace("apply(true,", "apply(flag,")
            .replace("relay(true)", "relay(flag)");
        assert!(
            support::compile_with_protocols(&runtime, &Default::default()).is_err(),
            "accepted a runtime stage selector: {entry}"
        );
        let overflow = source.replace("value==9", "value==256");
        assert!(
            support::compile_with_protocols(&overflow, &Default::default()).is_err(),
            "accepted an out-of-range Byte: {entry}"
        );
    }
}

#[test]
fn ordinary_packets_share_later_type_evidence_without_reordering() {
    let source = "typealias Byte=.u8'builtin;
        fn apply(action:fn(T)->U,value:T)->U where T,U {action(value)}
        fn callbacks(value:T)->(T,) where T {(value,)}
        fn values()->(Byte,) {(9,)}
        fn run()->bool {apply(...callbacks(|value| value==9),...values())}";
    let package = compile(source);
    let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let events = support::calls(&package, &vm);
    assert_eq!(
        events
            .iter()
            .filter(|name| matches!(name.as_str(), "callbacks" | "values" | "apply"))
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["callbacks", "values", "apply"]
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn packet_evidence_follows_methods_collections_and_property_routing() {
    for (declaration, call) in [
        (
            "fn apply(action:fn(T)->U,value:T,.enabled:bool=true)->U where T,U {action(value)}",
            "apply(.enabled=flag(),...callbacks(|value| value==9),...values())",
        ),
        (
            "struct Holder {} impl Holder {fn apply(self,action:fn(T)->U,value:T)->U where T,U {action(value)}} fn make()->Holder {Holder {}}",
            "make().apply(...callbacks(|value| value==9),...values())",
        ),
        (
            "struct Holder {} trait Apply {fn apply(self,action:fn(T)->U,value:T)->U where T,U;} impl Apply for Holder {fn apply(self,action:fn(T)->U,value:T)->U where T,U {action(value)}} fn make()->Holder {Holder {}}",
            "make().as(Apply).apply(...callbacks(|value| value==9),...values())",
        ),
        (
            "fn apply(...items:(fn(T)->U,T))->U where T,U {let (action,value)=items;action(value)}",
            "apply(...callbacks(|value| value==9),...values())",
        ),
        (
            "fn apply(action:fn(T)->U,value:T,...properties:P)->U where T,U,P {action(value)}",
            "apply {enabled:flag(),...callbacks(|value| value==9),...values()}",
        ),
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            fn callbacks(value:T)->(T,) where T {{(value,)}}
            fn values()->(Byte,) {{(9,)}}
            fn flag()->bool {{true}}
            {declaration}
            fn run()->bool {{{call}}}"
        );
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{call}: {error}"));
        let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        let events = support::calls(&package, &vm);
        let expected = if call.starts_with("make") {
            vec!["run", "make", "callbacks", "values", "apply"]
        } else if call.contains("flag()") {
            vec!["run", "flag", "callbacks", "values", "apply"]
        } else {
            vec!["run", "callbacks", "values", "apply"]
        };
        // The callback may have a synthesized name; named producers and the
        // receiver must appear exactly once and in source order.
        assert_eq!(
            events
                .iter()
                .filter(|name| expected.contains(&name.as_str()))
                .map(String::as_str)
                .collect::<Vec<_>>(),
            expected,
            "{call}"
        );
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{call}"
        );
        let rejected = source.replace("value==9", "value==256");
        assert!(
            support::compile_with_protocols(&rejected, &Default::default()).is_err(),
            "{call}: accepted an out-of-range Byte"
        );
    }
}

#[test]
fn ordinary_spreads_preserve_partial_structural_result_context() {
    for value in [
        "keep((|value| value==9,9))",
        "keep(...wrap((|value| value==9,9)))",
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn keep(value:T)->T where T {{value}}
            fn wrap(value:T)->(T,) where T {{(value,)}}
            fn apply(values:(fn(Byte)->T,Byte))->T where T {{
                let (callback,value)=values;callback(value)
            }}
            fn run()->bool {{let result=apply({value});result}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{value}"
        );
    }
}

#[test]
fn open_spreads_preserve_partial_result_context_without_invoking_staged_inputs_early() {
    for call in [
        "keep(true,(|value| value==9,9))",
        "keep(true,...wrap((|value| value==9,9)))",
        "keep(...(true,),...wrap((|value| value==9,9)))",
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            fn keep(comptime enabled:bool,value:T)->T where T {{
                inline if enabled {{value}} else {{value}}
            }}
            fn wrap(value:T)->(T,) where T {{(value,)}}
            fn apply(values:(fn(Byte)->T,Byte))->T where T {{
                let (callback,value)=values;callback(value)
            }}
            fn run()->bool {{let result=apply({call});result}}"
        );
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{call}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let runtime_flag = source
            .replace("fn run()->bool", "fn run(flag:bool)->bool")
            .replace("keep(true,", "keep(flag,")
            .replace("...(true,)", "...(flag,)");
        assert!(support::compile_with_protocols(&runtime_flag, &Default::default()).is_err());
    }
}

#[test]
fn open_callable_result_context_reaches_direct_and_spread_inputs() {
    for call in [
        "keep(true,|value| value==9)",
        "keep(true,...wrap(|value| value==9))",
    ] {
        let source=format!(
            "typealias Byte=.u8'builtin;
            fn keep(comptime flag:bool,value:T)->T where T {{inline if flag {{value}} else {{value}}}}
            fn wrap(value:T)->(T,) where T {{(value,)}}
            fn apply(action:fn(Byte)->T)->T where T {{action(9)}}
            fn run()->bool {{let result=apply({call});result}}"
        );
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{call}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn open_callers_describe_contextual_packets_before_staged_values_exist() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn keep(comptime flag:bool,value:T)->T where T {inline if flag {value} else {value}}
        fn wrap(value:T)->(T,) where T {(value,)}
        fn apply(values:(fn(Byte)->T,Byte))->T where T {
            let (callback,value)=values;callback(value)
        }
        fn relay(comptime flag:bool)->bool {
            let result=apply(keep(flag,...wrap((|value| value==9,9))));result
        }
        fn run()->bool {relay(true) and relay(false)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_open_calls_preserve_callback_phase_and_partial_packet_context() {
    let source = "typealias Byte=.u8'builtin;
        comptime fn keep(comptime flag:bool,value:T)->T where T {if flag {value} else {value}}
        comptime fn wrap(value:T)->(T,) where T {(value,)}
        comptime fn apply(values:(comptime fn(Byte)->T,Byte))->T where T {
            let (callback,value)=values;callback(value)
        }
        comptime fn Run()->bool {
            let result=apply(keep(true,...wrap((|value| value==9,9))));result
        }";
    let package = compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        support::compile_with_protocols(
            &source.replace(
                "values:(comptime fn(Byte)->T,Byte)",
                "values:(fn(Byte)->T,Byte)"
            ),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn pending_open_result_is_not_replaced_by_the_callers_desired_shape() {
    let source = "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        comptime fn Select(flag:bool,T:Type)->Type {if flag {T} else {bool}}
        fn keep(comptime flag:bool,value:T)->defer Select(flag,T) where T {value}
        fn wrap(value:T)->(T,) where T {(value,)}
        fn apply(values:(fn(Byte)->T,Byte))->T where T {
            let (callback,value)=values;callback(value)
        }
        fn run()->bool {
            let number:Byte=9;
            let result=apply(keep(true,...wrap((|value:Byte| value==9,number))));result
        }";
    assert_eq!(
        luna::execution::interpret(&compile(source), "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        support::compile_with_protocols(
            &source.replace("keep(true,", "keep(false,"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn partial_packets_follow_method_optional_and_collection_routing() {
    for (declaration, value) in [
        (
            "fn keep(value:T,.enabled:bool=true)->T where T {value}",
            "keep(.enabled=true,...wrap((|value| value==9,9)))",
        ),
        (
            "fn collect(...values:T)->T where T {values}",
            "collect(...wrap(|value| value==9),9)",
        ),
        (
            "struct Holder {} impl Holder {fn keep(self,value:T)->T where T {value}}",
            "Holder {}.keep(...wrap((|value| value==9,9)))",
        ),
        (
            "struct Holder {} trait Keep {fn keep(self,value:T)->T where T;} impl Keep for Holder {fn keep(self,value:T)->T where T {value}}",
            "Holder {}.as(Keep).keep(...wrap((|value| value==9,9)))",
        ),
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            {declaration}
            fn wrap(value:T)->(T,) where T {{(value,)}}
            fn apply(values:(fn(Byte)->T,Byte))->T where T {{
                let (callback,value)=values;callback(value)
            }}
            fn run()->bool {{let result=apply({value});result}}"
        );
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{value}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{value}"
        );
        assert!(
            support::compile_with_protocols(
                &source.replace("value==9", "value==256"),
                &Default::default()
            )
            .is_err()
        );
    }
}

#[test]
fn effectful_generic_producers_preserve_partial_spread_results() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        effect tick()->bool;
        fn produce(value:T)->#tick T where T {tick()#;value}
        fn wrap(value:T)->(T,) where T {(value,)}
        fn apply(values:(fn(Byte)->T,Byte))->T where T {
            let (callback,value)=values;callback(value)
        }
        fn subject()->#tick bool {
            let result=apply(produce(...wrap((|value| value==9,9))));result
        }
        fn run()->bool {subject()# {tick()=>false,}}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn contextual_method_packets_preserve_receiver_and_named_input_order() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        struct Holder {}
        impl Holder {fn keep(self,value:T,.enabled:bool=true)->T where T {value}}
        fn make()->Holder {Holder {}}
        fn flag()->bool {true}
        fn wrap(value:T)->(T,) where T {(value,)}
        fn apply(values:(fn(Byte)->T,Byte))->T where T {
            let (callback,value)=values;callback(value)
        }
        fn run()->bool {
            let result=apply(make().keep(.enabled=flag(),...wrap((|value| value==9,9))));
            result
        }",
    );
    let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let events = support::calls(&package, &vm);
    assert_eq!(
        events
            .iter()
            .filter(|name| matches!(name.as_str(), "make" | "flag" | "wrap" | "keep" | "apply"))
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["make", "flag", "wrap", "keep", "apply"]
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn partial_variadic_packets_keep_normal_tuple_identity() {
    let package = support::compile_with_source_bindings(
        "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        typealias Byte=.u8'builtin;
        fn collect(...values:T)->T where T {values}
        fn wrap(value:T)->(T,) where T {(value,)}
        fn apply(values:(fn(Byte)->T,Byte))->T where T {
            let (callback,value)=values;callback(value)
        }
        fn run()->bool {let result=apply(collect(...wrap(|value| value==9),9));result}",
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_packet_contexts_preserve_record_fields() {
    let source = "typealias Byte=.u8'builtin;
        fn record(...fields:T)->T where T {fields}
        fn keep(value:T)->T where T {value}
        fn wrap(value:T)->(T,) where T {(value,)}
        fn exemplar(value:Byte)->T where T {exemplar<T>(value)}
        fn zero()->Byte {0}
        fn apply(...fields:(record {callback:exemplar<T>,value:zero()})'type)->T where T {
            fields.callback(fields.value)
        }
        fn run()->bool {
            let result=apply(.fields=keep(...wrap(record {callback:|value| value==9,value:9})));
            result
        }";
    assert_eq!(
        luna::execution::interpret(&compile(source), "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        support::compile_with_protocols(
            &source.replace("value==9,value:9", "value==9,value:256"),
            &Default::default()
        )
        .is_err()
    );
}
