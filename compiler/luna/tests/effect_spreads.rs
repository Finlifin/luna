//! Effect requests retain ordinary tuple input evaluation and effect identity.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use support::{calls, compile, compile_with_protocols, id};

#[test]
fn effect_packet_defaults_provide_callback_context() {
    for input in [
        "...packet(true), |value| false",
        "true, ...packet(|value| false)",
    ] {
        let source = format!("effect enum Actions where T,.U:.Type'builtin=T {{apply(value:T,action:fn(U)->bool)->bool,}}
            fn packet(value:T)->(T,) where T {{(value,)}}
            fn run()->bool {{Actions.apply({input})# {{Actions<bool>.apply(value,action)=>not action(value),}}}}");
        let package = compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{input}"
        );
    }
}

#[test]
fn ordinary_generic_spread_calls_discharge_effects_in_result_context() {
    for call in ["echo(true)", "echo(...packet())", "echo(...(true,))"] {
        let package = compile(&format!(
            "effect read()->bool;
            fn echo(value:T)->#read T where T {{read()#;value}}
            fn packet()->(bool,) {{(true,)}}
            fn request()->#read bool {{{call}}}
            fn run()->bool {{request()# {{read()=>false,}}}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{call}"
        );
    }
}

#[test]
fn effect_packets_receive_partial_nested_callback_context() {
    for input in [
        "...((|value| value == 9,9),)",
        "...packet((|value| value == 9,9))",
        "...do {packet((|value| value == 9,9))}",
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            typealias Arguments=(fn(Byte)->bool,Byte);
            fn packet(value:T)->(T,) where T {{(value,)}}
            effect echo(value:T)->T where T;
            fn apply(values:(fn(Byte)->T,Byte))->T where T {{
                let (callback,value)=values;callback(value)
            }}
            fn subject()->#[echo<Arguments>] bool {{
                let result=apply(echo({input})#);result
            }}
            fn run()->bool {{subject()# {{echo<Arguments>(value)=>value,}}}}"
        );
        let package = compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{input}"
        );
    }
}

#[test]
fn later_packet_evidence_checks_early_closures_without_reordering_producers() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        effect apply(action:fn(T)->U,value:T)->U where T,U;
        fn callbacks(value:T)->(T,) where T {(value,)}
        fn values()->(Byte,) {(9,)}
        fn run()->bool {
            apply(...callbacks(|value| value==9), ...values())#
                {apply<Byte,bool>(action,value)=>action(value),}
        }",
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let events = calls(&package, &vm);
    assert_eq!(
        events
            .iter()
            .filter(|name| matches!(name.as_str(), "callbacks" | "values"))
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["callbacks", "values"]
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn partial_effect_packet_context_rejects_wrong_domains_and_literal_overflow() {
    for value in ["(|value:bool| value,9)", "(|value| value==9,256)"] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            typealias Arguments=(fn(Byte)->bool,Byte);
            fn packet(value:T)->(T,) where T {{(value,)}}
            effect echo(value:T)->T where T;
            fn apply(values:(fn(Byte)->T,Byte))->T where T {{
                let (callback,value)=values;callback(value)
            }}
            fn subject()->#[echo<Arguments>] bool {{
                let result=apply(echo(...packet({value}))#);result
            }}"
        );
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "{value}"
        );
    }
}

#[test]
fn effect_packets_keep_partial_record_field_contexts() {
    let source = "typealias Byte=.u8'builtin;
        fn record(...fields:T)->T where T {fields}
        fn wrap(value:T)->(T,) where T {(value,)}
        fn exemplar(value:Byte)->T where T {exemplar<T>(value)}
        fn check(value:Byte)->bool {value==9}
        fn zero()->Byte {0}
        typealias Arguments=(record {callback:check,value:zero()})'type;
        effect echo(value:T)->T where T;
        fn apply(...fields:(record {callback:exemplar<T>,value:zero()})'type)->T where T {
            fields.callback(fields.value)
        }
        fn subject()->#[echo<Arguments>] bool {
            let result=apply(.fields=echo(...wrap(record {callback:|value| value==9,value:9}))#);
            result
        }
        fn run()->bool {subject()# {echo<Arguments>(value)=>value,}}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        source.replace("value==9,value:9", "value==9,value:256"),
        source.replace("callback:|value| value==9", "callback:|value:bool| value"),
    ] {
        assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn operation_spreads_preserve_once_only_source_order_including_empty_packets() {
    let package = compile(
        "effect combine(a:bool,b:bool,c:bool)->bool;
        fn first()->bool {true}
        fn packet()->(bool,bool) {(true,false)}
        fn empty(...items:T)->T where T {items}
        fn handle(a:bool,b:bool,c:bool)->bool {a and b and not c}
        fn run()->bool {
            combine(...empty(), first(), ...packet(), ...empty())#
                {combine(a,b,c)=>handle(a,b,c),}
        }",
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        ["run", "empty", "first", "packet", "empty", "handle"]
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_operations_and_groups_infer_spread_inputs_and_callback_contexts() {
    for head in [
        "apply",
        "apply<bool,bool>",
        "Channel.apply",
        "Channel<bool,bool>.apply",
    ] {
        let package = compile(&format!(
            "effect apply(value:T,action:fn(T)->U)->U where T,U;
            effect enum Channel where T,U {{apply(value:T,action:fn(T)->U)->U,}}
            fn packet()->(bool,) {{(false,)}}
            fn run()->bool {{
                {head}(...packet(), |value| not value)#
                    {{apply<bool,bool>(value,action)=>action(value),
                      Channel<bool,bool>.apply(value,action)=>action(value),}}
            }}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{head}"
        );
    }
}

#[test]
fn literal_and_computed_packets_receive_numeric_result_context() {
    for packet in ["(7,)", "packet(7)"] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;
            effect echo(value:T)->T where T;
            fn packet(value:T)->(T,) where T {{(value,)}}
            fn request()->#[echo<Byte>] Byte {{echo(...{packet})#}}
            fn run()->bool {{(request()# {{echo<Byte>(value)=>value,}})==7}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{packet}"
        );
    }
}

#[test]
fn operation_packets_preserve_bound_tuple_identity() {
    let package = support::compile_with_source_bindings(
        "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        effect combine(a:bool,b:bool)->bool;
        fn packet()->(bool,bool) {(true,false)}
        fn run()->bool {combine(...packet())# {combine(a,b)=>a and not b,}}",
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    let packet = package
        .functions
        .iter()
        .find(|f| f.name == "packet")
        .unwrap();
    let nir::Term::Function { result, .. } = packet.signature.callable_signature() else {
        panic!("packet signature");
    };
    assert!(matches!(result.as_ref(), nir::Term::NormalForm { .. }));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn operation_spreads_reject_wrong_width_types_and_unavailable_instances() {
    for input in [
        "...(true, false)",
        "...true",
        "...('x',)",
        ".value=true, ...()",
    ] {
        assert!(
            compile_with_protocols(
                &format!("effect echo(value:bool)->bool; fn run()->#echo bool {{echo({input})#}}"),
                &Default::default()
            )
            .is_err(),
            "{input}"
        );
    }
    assert!(
        compile_with_protocols(
            "effect echo(value:T)->T where T;
         fn packet()->(bool,) {(true,)}
         fn run()->#[echo<'x''type>] bool {echo(...packet())#}",
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn exiting_input_skips_later_producers_and_the_request() {
    for input in [
        "do {return true; false}, false",
        "...do {return true; (false,false)}",
        "...do {return true; (false,)}, later()",
    ] {
        let package = compile(&format!(
            "effect combine(a:bool,b:bool)->bool;
            fn later()->bool {{false}}
            fn handled()->bool {{false}}
            fn request()->#combine bool {{combine({input})#}}
            fn run()->bool {{request()# {{combine(a,b)=>handled(),}}}}"
        ));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true),
            "{input}"
        );
        assert_eq!(calls(&package, &vm), ["run", "request"], "{input}");
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{input}"
        );
    }
}

#[test]
fn spread_fields_preserve_computation_types_in_nir() {
    let package = compile(
        "effect read()->bool;
        effect retain(value:#read bool)->#read bool;
        fn source()->#read bool {read()#}
        fn packet()->(#read bool,) {(source(),)}
        fn request()->#[retain,read] bool {retain(...packet())#}
        fn run()->bool {
            request()# {retain(value)=>value,read()=>true,}
        }",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "request")
        .unwrap();
    let mut found = false;
    iir::visit::expressions(function, &mut 10_000, |value, _| {
        if let nir::program::ExprKind::EffectRequest { arguments, .. } = &value.kind {
            assert!(matches!(arguments[0].ty, nir::Term::Computation { .. }));
            assert!(matches!(value.ty, nir::Term::Computation { .. }));
            found = true;
        }
        Ok(())
    })
    .unwrap();
    assert!(found);
    // Stored computation values have no executable evidence representation
    // yet, including without spreading. Preserve that explicit boundary.
    for package in [
        package,
        compile(
            "effect read()->bool; effect retain(value:#read bool)->#read bool;
            fn source()->#read bool {read()#}
            fn request()->#[retain,read] bool {retain(source())#}
            fn run()->bool {request()# {retain(value)=>value,read()=>true,}}",
        ),
    ] {
        assert!(
            luna::execution::interpret(&package, "run", 100_000)
                .unwrap_err()
                .contains("stored computation values")
        );
    }
}

#[test]
fn generic_spread_result_discharge_keeps_computation_inputs() {
    let package = compile(
        "effect read()->bool;
        effect retain(value:#read T)->#read T where T;
        fn source()->#read bool {read()#}
        fn packet()->(#read bool,) {(source(),)}
        fn request()->#[retain<bool>,read] bool {retain(...packet())#}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "request")
        .unwrap();
    let mut found = false;
    iir::visit::expressions(function, &mut 10_000, |value, _| {
        if let nir::program::ExprKind::EffectRequest {
            arguments,
            operation,
            ..
        } = &value.kind
        {
            assert_eq!(
                operation.arguments,
                [nir::Term::Primitive(nir::Primitive::Bool)]
            );
            assert!(matches!(arguments[0].ty, nir::Term::Computation { .. }));
            found = true;
        }
        Ok(())
    })
    .unwrap();
    assert!(found);
}

#[test]
fn affine_packets_deliver_and_clean_each_field_once_with_and_without_inlining() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Left {} struct Right {}
        fn left_clean() {} fn right_clean() {}
        impl Retire for Left {fn drop(*self) {left_clean();}}
        impl Retire for Right {fn drop(*self) {right_clean();}}
        inline effect receive(left:Left,right:Right)->bool;
        fn packet()->(Left,Right) {(Left {},Right {})}
        fn run()->bool {
            receive(...packet())# {receive(first,second)=>true,}
        }";
    for inline in [false, true] {
        let events = cleanup_events(source, inline);
        assert_eq!(
            events
                .iter()
                .filter(|name| name.ends_with("_clean"))
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["right_clean", "left_clean"],
            "inline={inline}: {events:?}"
        );
        assert_eq!(events.iter().filter(|name| *name == "packet").count(), 1);
    }
}

#[test]
fn exiting_packet_cleans_prepared_affine_inputs_without_dispatch() {
    for input in [
        "make_left(), ...do {return true; (Right {},)}",
        "...(make_left(),), do {return true; Right {}}",
    ] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Left {{}} struct Right {{}}
            fn left_clean() {{}} fn right_clean() {{}} fn handled() {{}}
            impl Retire for Left {{fn drop(*self) {{left_clean();}}}}
            impl Retire for Right {{fn drop(*self) {{right_clean();}}}}
            fn make_left()->Left {{Left {{}}}}
            inline effect receive(left:Left,right:Right)->bool;
            inline fn request()->#receive bool {{receive({input})#}}
            fn run()->bool {{request()# {{receive(left,right)=>{{handled();false}},}}}}"
        );
        for inline in [false, true] {
            let events = cleanup_events(&source, inline);
            assert_eq!(
                events
                    .iter()
                    .filter(|name| name.ends_with("_clean"))
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                ["left_clean"],
                "inline={inline}, {input}: {events:?}"
            );
            assert!(!events.iter().any(|name| name == "handled"));
            assert_eq!(events.iter().filter(|name| *name == "make_left").count(), 1);
        }
    }
}

fn cleanup_events(source: &str, inline: bool) -> Vec<String> {
    use elaboration::{
        implementation::{ImplementationHead, Index},
        ptc::Context,
    };
    let mut package = compile(source);
    if inline {
        elaboration::program::inlining::expand(&mut package, &mut 100_000).unwrap();
    }
    let context = Context::from_package(&package).unwrap();
    let mut index = Index::default();
    for implementation in package
        .implementations
        .iter()
        .filter(|i| i.interface.is_some())
    {
        index
            .insert(
                ImplementationHead {
                    parameter_types: implementation.parameter_types.clone(),
                    declaration: implementation.declaration,
                    interface: implementation.interface,
                    interface_arguments: implementation.interface_arguments.clone(),
                    subject: implementation.subject.clone(),
                    type_parameters: implementation.type_parameters.clone(),
                    requirements: implementation.requirements.clone(),
                },
                Default::default(),
            )
            .unwrap();
    }
    let world = elaboration::capabilities::World {
        index: &index,
        context: &context,
        implementations: &package.implementations,
        traits: &package.traits,
        assumptions: &[],
    };
    let mut items = intrinsic::LangItems::new();
    let protocol = |name| {
        package
            .traits
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .declaration
    };
    items
        .bind_source_traits(
            &package,
            &[
                (intrinsic::LangItem::Copy, protocol("Duplicate")),
                (intrinsic::LangItem::Drop, protocol("Retire")),
            ],
        )
        .unwrap();
    let executable = luna::cleanup::prepare_source(
        &package,
        &world,
        &items,
        iir::InstanceKey {
            declaration: id(&package, "run"),
            arguments: vec![],
        },
        &mut 100_000,
        Default::default(),
    )
    .unwrap();
    let mut vm = Interpreter::from_iir(&executable.graph, Budget::default())
        .unwrap()
        .with_cleanup_schedule(&executable.schedule)
        .unwrap();
    assert_eq!(
        vm.invoke_instance(&executable.entry, vec![]).unwrap(),
        Value::Bool(true)
    );
    calls(&package, &vm)
}

#[test]
fn contextual_constructor_exit_cleans_earlier_payload_without_calling_consumer() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Byte=.u8'builtin;
        struct Held {}
        fn held_clean() {}
        impl Retire for Held {fn drop(*self) {held_clean();}}
        enum Packet where T,E {pair(T,T,E)}
        fn first()->Held {Held {}}
        fn accept(value:Packet<T,Byte>)->bool where T {false}
        fn run()->bool {
            accept(.pair(first(),...do {return true; (Held {},9)}))
        }";
    for inline in [false, true] {
        let events = cleanup_events(source, inline);
        assert_eq!(events.iter().filter(|name| *name == "first").count(), 1);
        assert_eq!(
            events.iter().filter(|name| *name == "held_clean").count(),
            1
        );
        assert!(!events.iter().any(|name| name == "accept"));
    }
}

#[test]
fn fabricated_never_result_does_not_bypass_effect_signature_validation() {
    let mut package = compile(
        "effect echo(value:bool)->bool;
        fn request()->#echo bool {echo(true)#}
        fn run()->bool {request()# {echo(value)=>value,}}",
    );
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.name == "request")
        .unwrap();
    let mut changed = false;
    nir::visit::expression_mut_pruned(&mut function.body, &mut 10_000, |value, _| {
        if matches!(value.kind, nir::program::ExprKind::EffectRequest { .. }) {
            value.ty = nir::Term::Primitive(nir::Primitive::Never);
            changed = true;
        }
        Ok(true)
    })
    .unwrap();
    assert!(changed);
    assert!(luna::execution::interpret(&package, "run", 100_000).is_err());
}

#[test]
fn prepared_never_pattern_binding_cleans_only_initialized_inputs() {
    for body in [
        "let (held,flag)=pair(first(),...do {return true;(false,)}) else {return false;};guard()",
        "pair(first(),...do {return true;(false,)}) match {(held,flag) if guard()=>false,_=>false}",
    ] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Held {{}} fn held_clean() {{}}
            impl Retire for Held {{fn drop(*self) {{held_clean();}}}}
            fn first()->Held {{Held {{}}}}
            inline fn pair(a:Held,b:bool)->(Held,bool) {{(a,b)}}
            fn guard()->bool {{false}}
            fn run()->bool {{{body}}}"
        );
        for inline in [false, true] {
            let events = cleanup_events(&source, inline);
            assert_eq!(events.iter().filter(|name| *name == "first").count(), 1);
            assert_eq!(
                events.iter().filter(|name| *name == "held_clean").count(),
                1
            );
            assert!(!events.iter().any(|name| name == "pair" || name == "guard"));
        }
    }
}
