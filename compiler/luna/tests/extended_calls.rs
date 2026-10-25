//! Extended property collection and positional expansion share one call plan.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{
    calls, compile_splice_fixture, compile_with_tuple as compile,
    compile_with_tuple_protocols as compile_with_protocols, id,
};

fn run(source: &str, expected: Value, order: &[&str]) {
    let package = compile_with_protocols(source, &Default::default())
        .unwrap_or_else(|error| panic!("{source}: {error}"));
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(&package, "run"), vec![]).unwrap(), expected);
    assert_eq!(calls(&package, &vm), order);
    // Also execute the completed runtime artifact, including phase and
    // materialization checks that direct NIR interpretation doesn't provide.
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        expected
    );
}

#[test]
fn mixed_property_and_tuple_spreads_preserve_every_producer_in_source_order() {
    for signature in [
        "...items:Tuple<bool,bool>,...properties:T",
        "...properties:T,...items:Tuple<bool,bool>",
    ] {
        for (inputs, order) in [
            (
                "...packet(),flag:mark()",
                vec!["run", "packet", "mark", "take"],
            ),
            (
                "flag:mark(),...packet()",
                vec!["run", "mark", "packet", "take"],
            ),
            (
                "...do {(first(),)},flag:mark(),...do {(last(),)}",
                vec!["run", "first", "mark", "last", "take"],
            ),
            (
                "...(first(),),flag:mark(),...(last(),)",
                vec!["run", "first", "mark", "last", "take"],
            ),
        ] {
            run(
                &format!(
                    "fn packet()->Tuple<bool,bool> {{(true,false)}} fn first()->bool {{true}} fn last()->bool {{false}} fn mark()->bool {{true}} fn take({signature})->Tuple<T,Tuple<bool,bool>> where T {{(properties,items)}} fn run()->bool {{let (properties,(a,b))=take {{{inputs}}};properties.flag and a and not b}}"
                ),
                Value::Bool(true),
                &order,
            );
        }
    }
}

#[test]
fn empty_packets_still_run_between_properties_and_before_defaults() {
    for (inputs, order) in [
        (
            "...empty(),first:left(),second:right()",
            vec!["run", "empty", "left", "right", "take"],
        ),
        (
            "first:left(),...empty(),second:right()",
            vec!["run", "left", "empty", "right", "take"],
        ),
        (
            "first:left(),second:right(),...empty()",
            vec!["run", "left", "right", "empty", "take"],
        ),
    ] {
        run(
            &format!(
                "fn empty(...items:T)->T where T {{items}} fn left()->bool {{true}} fn right()->bool {{false}} fn take(...properties:T,.flag:bool=true)->T where T {{properties}} fn run()->bool {{let p=take {{{inputs}}};p.first and not p.second}}"
            ),
            Value::Bool(true),
            &order,
        );
    }
}

#[test]
fn packet_context_reaches_numeric_literals_and_property_callbacks() {
    run(
        "comptime fn schema(...properties:T)->T where T {properties} fn negate(x:bool)->bool {not x} const SAMPLE=schema {action:negate};typealias Options=SAMPLE'type;fn take(...items:Tuple<.u8'builtin,bool>,...properties:Options)->.u8'builtin {let (n,b)=items;if properties.action(b) {n} else {0}} fn run()->.u8'builtin {take {action:|x| not x,...do {(7,false)}}}",
        Value::UnsignedInteger(7),
        &["run", "take"],
    );
}

#[test]
fn peer_property_inference_is_retained_without_replaying_producers() {
    run(
        "fn keep(...properties:T)->T where T {properties} fn select(...properties:T,model:T,flag:bool)->T where T {properties} fn mark()->bool {true} fn pair()->Tuple<bool> {(true,)} fn run()->bool {let model=keep {flag:false};let result=select {flag:mark(),model,...pair()};result.flag}",
        Value::Bool(true),
        &["run", "keep", "mark", "pair", "select"],
    );
}

#[test]
fn method_receivers_precede_mixed_inputs() {
    for declaration in [
        "impl Holder {fn take(self,...items:Tuple<.u8'builtin,bool>,...properties:Options)->.u8'builtin {let (n,b)=items;if b and properties.flag {n} else {0}}}",
        "trait Take {fn take(self,...items:Tuple<.u8'builtin,bool>,...properties:Options)->.u8'builtin;} impl Take for Holder {fn take(self,...items:Tuple<.u8'builtin,bool>,...properties:Options)->.u8'builtin {let (n,b)=items;if b and properties.flag {n} else {0}}}",
    ] {
        let access = if declaration.starts_with("trait") {
            "make().as(Take).take"
        } else {
            "make().take"
        };
        run(
            &format!(
                "comptime fn schema(...properties:T)->T where T {{properties}} const SAMPLE=schema {{flag:true}};typealias Options=SAMPLE'type;struct Holder {{}} {declaration} fn make()->Holder {{Holder {{}}}} fn mark()->bool {{true}} fn run()->.u8'builtin {{{access} {{flag:mark(),...do {{(7,true)}}}}}}"
            ),
            Value::UnsignedInteger(7),
            &["run", "make", "mark", "take"],
        );
    }
}

#[test]
fn malformed_mixed_inputs_reject_instead_of_changing_routes() {
    for (inputs, diagnostic) in [
        (
            "flag:true,...(true,false,true)",
            "variadic collection argument count mismatch",
        ),
        ("flag:true,...false", "type mismatch"),
        ("flag:true,...(true,false),flag:false", "more than once"),
    ] {
        let error = compile_with_protocols(&format!("fn take(...items:Tuple<bool,bool>,...properties:T)->T where T {{properties}} fn run() {{take {{{inputs}}};}}"), &Default::default()).unwrap_err();
        assert!(error.contains(diagnostic), "{inputs}: {error}");
    }
}

#[test]
fn expanded_collections_preserve_affine_packet_and_property_moves() {
    use elaboration::verification::moves::{Delivery, analyze_in_package};
    for (body, valid) in [
        ("take {...packet,item:third};", true),
        ("take {item:third,...packet};", true),
        ("take {...packet,item:third};let again=packet;", false),
        ("take {item:third,...packet};let again=third;", false),
        ("take {...packet,item:first};", false),
    ] {
        let package = compile(&format!(
            "struct Ticket {{}} fn take(...items:Tuple<Ticket,Ticket>,...properties:T) where T {{}} fn run(first:Ticket,second:Ticket,third:Ticket) {{let packet=(first,second);{body}}}"
        ));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(
            &package,
            function,
            |ty| {
                Ok(if matches!(ty, nir::Term::Primitive(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                })
            },
            100_000,
        );
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
        if let Err(failure) = result {
            assert!(failure.reason.contains("unavailable"), "{failure}");
        }
    }
}

#[test]
fn exits_inside_a_packet_skip_later_properties_and_the_call() {
    run(
        "fn first()->bool {true} fn later()->bool {false} fn take(...items:Tuple<bool,bool>,...properties:T)->bool where T {false} fn run()->bool {take {flag:first(),...do {return true;(false,false)},tail:later()}}",
        Value::Bool(true),
        &["run", "first"],
    );
    run(
        "fn packet()->Tuple<bool,bool> {(true,false)} fn later()->bool {false} fn take(...items:Tuple<bool,bool>,...properties:T)->bool where T {false} fn run()->bool {take {flag:do {return true;false},...packet(),tail:later()}}",
        Value::Bool(true),
        &["run"],
    );
}

#[test]
fn contextual_result_and_explicit_type_arguments_choose_collectors() {
    for call in [
        "gather {flag:true,...packet()}",
        "gather<Tuple<bool,bool>,Options> {flag:true,...packet()}",
    ] {
        let package = compile(&format!(
            "comptime fn schema(...properties:T)->T where T {{properties}} const SAMPLE=schema {{flag:true}};typealias Options=SAMPLE'type;fn packet()->Tuple<bool,bool> {{(true,false)}} fn gather(...items:A,...properties:B)->Tuple<A,B> where A,B {{(items,properties)}} fn configured()->Tuple<Tuple<bool,bool>,Options> {{{call}}} fn run()->bool {{let ((a,b),p)=configured();a and not b and p.flag}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn staged_collections_retain_dependent_default_binding() {
    let package = compile(
        "comptime fn schema(...properties:T)->T where T {properties} const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;fn take(comptime ...properties:Options,first:bool,second:bool,comptime .flag:bool=properties.flag)->bool {flag and first and not second} fn packet()->Tuple<bool,bool> {(true,false)} fn run()->bool {take {...packet(),flag:true}}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn mixed_calls_preserve_the_bound_tuple_normal_form() {
    let package = execution_support::compile_with_source_bindings(
        "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T} pure comptime fn Tuple<...Es:List<Type>>->Type {bool} fn packet()->Tuple<bool,bool> {(true,false)} fn take(...items:Tuple<bool,bool>,...properties:T)->Tuple<Tuple<bool,bool>,T> where T {(items,properties)} fn run()->bool {let ((a,b),p)=take {...packet(),flag:true};a and not b and p.flag}",
        &Default::default(), luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"), tuple: Some("input::Tuple"), ..Default::default()
        },
    ).unwrap();
    let tuple = id(&package, "Tuple");
    let packet = package
        .functions
        .iter()
        .find(|f| f.name == "packet")
        .unwrap();
    let nir::Term::Function { result, .. } = &packet.signature else {
        panic!()
    };
    assert!(
        matches!(result.as_ref(), nir::Term::NormalForm { head, arguments, .. }
        if *head == tuple && matches!(arguments.as_slice(), [nir::Term::TypeListValue(fields)] if fields.len() == 2))
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn computed_callees_accept_braced_inputs_and_run_before_producers() {
    for (callee, prefix) in [
        ("factory()", vec!["run", "factory"]),
        ("do {factory()}", vec!["run", "factory"]),
        (
            "(if flag() {factory()} else {target})",
            vec!["run", "flag", "factory"],
        ),
    ] {
        for (inputs, producers) in [
            ("first(),second()", vec!["first", "second"]),
            ("...(first(),second())", vec!["first", "second"]),
            ("...packet()", vec!["packet", "first", "second"]),
        ] {
            let mut order = prefix.clone();
            order.extend(producers);
            order.push("target");
            run(
                &format!(
                    "fn target(a:bool,b:bool)->bool {{a and not b}} const ACTION=target;
                fn factory()->fn(bool,bool)->bool {{target}} fn flag()->bool {{true}}
                fn first()->bool {{true}} fn second()->bool {{false}}
                fn packet()->Tuple<bool,bool> {{(first(),second())}}
                fn run()->bool {{{callee} {{{inputs}}}}}"
                ),
                Value::Bool(true),
                &order,
            );
        }
    }
}

#[test]
fn computed_type_callees_still_construct_structs() {
    for callee in [
        "Choose()",
        "do {Choose()}",
        "(if true {Box} else {Box})",
        "ALIAS",
    ] {
        run(
            &format!(
                "typealias Type=.Type'builtin;struct Box {{value:bool}} comptime fn Choose()->Type {{Box}} const ALIAS=Box;
            fn mark()->bool {{true}} fn run()->bool {{let b={callee} {{value:mark()}};b.value}}"
            ),
            Value::Bool(true),
            &["run", "mark"],
        );
    }
}

#[test]
fn computed_callees_do_not_recover_declaration_parameter_names() {
    for callee in ["factory()", "do {factory()}"] {
        for inputs in ["value:true"] {
            let error = compile_with_protocols(
                &format!(
                    "fn target(value:bool)->bool {{value}}
                const ACTION=target; fn factory()->fn(bool)->bool {{target}}
                fn run()->bool {{{callee} {{{inputs}}}}}"
                ),
                &Default::default(),
            )
            .unwrap_err();
            assert!(
                error.contains("metadata") || error.contains("named"),
                "{error}"
            );
        }
    }
}

#[test]
fn computed_callees_preserve_captures_and_input_exit_order() {
    run(
        "fn first()->bool {true} fn run()->bool {(do {let captured=true;|value:bool| captured and value}) {...do {(first(),)}}}",
        Value::Bool(true),
        &["run", "first"],
    );
    for input in ["do {return true;false}", "...do {return true;(false,)}"] {
        run(
            &format!(
                "fn target(value:bool)->bool {{false}} fn factory()->fn(bool)->bool {{target}}
            fn run()->bool {{factory() {{{input}}}}}"
            ),
            Value::Bool(true),
            &["run", "factory"],
        );
    }
    run(
        "fn target(value:bool)->bool {value} fn mark()->bool {false}
        fn leave()->bool {true} fn run()->bool {(do {if leave() {return true;} target}) {...do {(mark(),)}}}",
        Value::Bool(true),
        &["run", "leave"],
    );
}

#[test]
fn braced_stored_comptime_calls_preserve_phase_and_type_checks() {
    let source = "comptime fn target(a:bool,b:bool)->bool {a and not b}
        const ACTION=target;const ANSWER=ACTION {...(true,false)};
        fn select(comptime value:bool)->bool {inline if value {true} else {false}}
        fn run()->bool {select(ANSWER)}";
    run(source, Value::Bool(true), &["run", "select"]);
    let error = compile_with_protocols(
        &source.replace("...(true,false)", "...(true,7)"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("type mismatch") || error.contains("integer"),
        "{error}"
    );
    let error = compile_with_protocols(
        "fn target(value:bool)->bool {value} const ACTION=target;
        fn run()->bool {ACTION {true}}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("Splice"), "{error}");
}

#[test]
fn computed_constructor_types_do_not_require_runtime_splice() {
    let source = "typealias Type=.Type'builtin;
        trait Splice {assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;}
        struct Box {value:bool} comptime fn Choose()->Type {Box}
        fn run()->bool {let b=Choose() {value:true};b.value}";
    let package =
        execution_support::compile_with_bindings(source, &Default::default(), Some("Splice"))
            .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn selected_comptime_callees_deliver_only_the_invocation_result_to_runtime() {
    let prefix = include_str!("../../../development/examples/splice-comptime-calls.fl");
    for (callee, expected) in [
        ("(if true {make()} else {other()})", true),
        ("(if false {make()} else {other()})", false),
        ("(do {inline if true {make()} else {other()}})", true),
        ("(do {inline if false {make()} else {other()}})", false),
        (
            "(do {inline when {false => make(),else => other()}})",
            false,
        ),
        (
            "(do {inline if true is do {true => make(),false => other()}})",
            true,
        ),
        ("(when {true => make(),else => other()})", true),
        ("(when {false => make(),else => other()})", false),
        ("(true match {true => make(),false => other()})", true),
        ("(false match {true => make(),false => other()})", false),
        ("(do {if true {do {make()}} else {other()}})", true),
    ] {
        for input in ["(false)", "{false}", "{...do {(false,)}}"] {
            let source = format!(
                "{prefix} comptime fn make()->comptime fn(bool)->bool {{invert}}
                comptime fn other()->comptime fn(bool)->bool {{identity}} fn entry()->Tuple<bool,bool> {{{callee}{input}}}"
            );
            let package = compile_splice_fixture(&source, &Default::default(), Some("Splice"))
                .unwrap_or_else(|error| panic!("{callee}{input}: {error}"));
            let graph = luna::instantiation::instantiate(&package, "entry", 100_000).unwrap();
            let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                Value::Tuple(vec![Value::Bool(expected), Value::Bool(!expected)])
            );
            let events = calls(&package, &vm);
            assert_eq!(
                events
                    .iter()
                    .filter(|name| name.as_str() == "splice")
                    .count(),
                1
            );
            assert!(
                !events
                    .iter()
                    .any(|name| ["make", "other", "invert", "identity"].contains(&name.as_str())),
                "{events:?}"
            );
        }
    }
}

#[test]
fn callee_delivery_context_does_not_stage_conditions_or_discarded_values() {
    let prefix = include_str!("../../../development/examples/splice-comptime-calls.fl");
    for callee in [
        "(if flag {make()} else {make()})",
        "(when {flag => make(),else => make()})",
        "(flag match {true => make(),false => make()})",
        "(if true {make();make()} else {make()})",
    ] {
        let source = format!(
            "{prefix} comptime fn make()->comptime fn(bool)->bool {{invert}}
            fn entry(flag:bool)->Tuple<bool,bool> {{{callee}(false)}}"
        );
        let error =
            compile_splice_fixture(&source, &Default::default(), Some("Splice")).unwrap_err();
        assert!(!error.contains("parsing failed"), "{error}");
        if callee.contains("make();") {
            // The discarded result still requests Splice, for which this
            // function type deliberately has no implementation.
            assert!(error.contains("no-applicable-candidate"), "{error}");
        } else {
            assert!(
                error.contains("comptime") || error.contains("runtime"),
                "{callee}: {error}"
            );
        }
    }
}

#[test]
fn selected_comptime_callees_keep_open_inputs_and_closure_environments() {
    let prefix = include_str!("../../../development/examples/splice-comptime-calls.fl");
    for callee in [
        "(if flag {make(true)} else {make(false)})",
        "(do {inline if flag {make(true)} else {make(false)}})",
        "(when {flag=>make(true),else=>make(false)})",
        "(do {inline when {flag=>make(true),else=>make(false)}})",
        "(flag match {true=>make(true),false=>make(false)})",
        "(do {inline if flag is do {true=>make(true),false=>make(false)}})",
    ] {
        for invocation in ["()", "{}"] {
            let source = format!(
                "{prefix}
            typealias Callback=%{{flag:bool}} comptime fn()->bool;
            comptime fn make(flag:bool)->Callback {{|| flag}}
            fn choose(comptime flag:bool)->Tuple<bool,bool> {{{callee}{invocation}}}
            fn entry()->Tuple<Tuple<bool,bool>,Tuple<bool,bool>> {{(choose(true),choose(false))}}"
            );
            let package = compile_splice_fixture(&source, &Default::default(), Some("Splice"))
                .unwrap_or_else(|error| panic!("{callee}: {error}"));
            assert_eq!(
                luna::execution::interpret(&package, "entry", 100_000).unwrap(),
                Value::Tuple(vec![
                    Value::Tuple(vec![Value::Bool(true), Value::Bool(false)]),
                    Value::Tuple(vec![Value::Bool(false), Value::Bool(true)])
                ]),
                "{callee}"
            );
        }
    }
}

#[test]
fn inline_callee_selection_checks_unselected_branches_and_runtime_dependencies() {
    let prefix = include_str!("../../../development/examples/splice-comptime-calls.fl");
    for (parameter, callee) in [
        ("flag:bool", "(do {inline if flag {make()} else {make()}})"),
        (
            "flag:bool",
            "(do {inline when {flag=>make(),else=>make()}})",
        ),
        (
            "flag:bool",
            "(do {inline if flag is do {true=>make(),false=>make()}})",
        ),
        (
            "comptime flag:bool",
            "(do {inline if flag {make()} else {invert(7)}})",
        ),
    ] {
        let source = format!("{prefix} comptime fn make()->comptime fn(bool)->bool {{invert}}
            fn entry({parameter})->Tuple<bool,bool> {{{callee}(false)}} fn run_entry()->Tuple<bool,bool> {{entry(true)}}");
        let error =
            compile_splice_fixture(&source, &Default::default(), Some("Splice")).unwrap_err();
        assert!(!error.contains("parsing failed"), "{error}");
        assert!(
            error.contains("runtime")
                || error.contains("comptime")
                || error.contains("type mismatch")
                || error.contains("integer"),
            "{callee}: {error}"
        );
    }
}

#[test]
fn open_runtime_callee_factories_preserve_explicit_implicit_inputs() {
    for selection in [
        "inline if flag {first(bias)} else {second(bias)}",
        "inline when {flag=>first(bias),else=>second(bias)}",
        "inline if flag is do {true=>first(bias),false=>second(bias)}",
    ] {
        for invocation in ["(input())", "{...do {(input(),)}}"] {
            run(&format!("typealias Callback=%{{bias:bool}} fn(bool)->bool;
                fn first(bias:bool)->Callback {{|value:bool| bias and value}}
                fn second(bias:bool)->Callback {{|value:bool| bias or value}}
                fn input()->bool {{false}} fn supply()->bool {{true}}
                fn choose(comptime flag:bool,implicit bias:bool)->bool {{(do {{{selection}}}){invocation}}}
                fn run()->bool {{let a=choose(true,.bias=supply());let b=choose(false,.bias=supply());not a and b}}"),
                Value::Bool(true), &["run","supply","choose","first","input","supply","choose","second","input"]);
        }
    }
}

#[test]
fn open_callee_factories_forward_unknown_staged_and_implicit_inputs() {
    for selection in [
        "factory(flag,.bias=bias)",
        "inline if flag {factory(flag,.bias=bias)} else {factory(flag,.bias=bias)}",
        "inline when {flag=>factory(flag,.bias=bias),else=>factory(flag,.bias=bias)}",
    ] {
        run(&format!("typealias Callback=%{{bias:bool}} fn(bool)->bool;
            fn factory(comptime mode:bool,implicit bias:bool)->Callback {{inline if mode {{|value:bool| bias and value}} else {{|value:bool| bias or value}}}}
            fn input()->bool {{false}} fn supply()->bool {{true}}
            fn choose(comptime flag:bool,implicit bias:bool)->bool {{(do {{{selection}}})(input())}}
            fn run()->bool {{let a=choose(true,.bias=supply());let b=choose(false,.bias=supply());not a and b}}"),
            Value::Bool(true), &["run","supply","choose","factory","input","supply","choose","factory","input"]);
    }
}

#[test]
fn described_open_callee_factories_still_check_stage_and_implicit_supply() {
    for (parameter, factory_call, diagnostic) in [
        ("flag:bool", "factory(flag,.bias=bias)", "runtime"),
        ("comptime flag:bool", "factory(flag)", "implicit"),
        ("comptime flag:bool", "factory(flag,.bias=7)", "integer"),
    ] {
        let source = format!("typealias Callback=%{{bias:bool}} fn(bool)->bool;
            fn factory(comptime mode:bool,implicit bias:bool)->Callback {{inline if mode {{|value:bool| bias and value}} else {{|value:bool| bias or value}}}}
            fn choose({parameter},bias:bool)->bool {{(do {{{factory_call}}})(false)}}");
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(!error.contains("parsing failed"), "{error}");
        assert!(
            error.contains(diagnostic) || error.contains("type mismatch"),
            "{factory_call}: {error}"
        );
    }
}

#[test]
fn generic_open_factories_accept_computed_packets_without_staging_the_producer() {
    for factory_call in [
        "factory(flag,...packet())",
        "factory<bool>(flag,...packet())",
        "factory<bool>(...(flag,),...packet())",
    ] {
        run(&format!("typealias Callback<T> = %{{value:T}} fn()->T;
            fn factory(comptime mode:bool,value:T)->Callback<T> where T {{inline if mode {{|| value}} else {{|| value}}}}
            fn packet()->Tuple<bool> {{(true,)}}
            fn choose(comptime flag:bool)->bool {{(do {{{factory_call}}})()}}
            fn run()->bool {{choose(true)}}"), Value::Bool(true), &["run","choose","packet","factory"]);
    }
}

#[test]
fn explicit_open_factory_packet_types_remain_scoped_to_each_invocation() {
    for (parameter, binding) in [
        ("value:T", ""),
        ("...items:Tuple<T>", "let (value,)=items;"),
    ] {
        run(&format!("typealias Callback<T> = %{{value:T}} fn()->T;
            fn factory(comptime mode:bool,{parameter})->Callback<T> where T {{{binding} inline if mode {{|| value}} else {{|| value}}}}
            fn packet(value:T)->Tuple<T> where T {{(value,)}}
            fn choose(comptime flag:bool,value:T)->T where T {{(do {{factory<T>(flag,...packet(value))}})()}}
            fn run()->bool {{let a=choose<bool>(true,true);let b=choose<.char'builtin>(false,'x');let c=choose<bool>(false,false);a and b=='x' and not c}}"),
            Value::Bool(true), &["run","choose","packet","factory","choose","packet","factory","choose","packet","factory"]);
    }
}

#[test]
fn explicit_open_factory_packets_reject_conflicting_types_and_stage_inputs() {
    for (parameter, input, diagnostic) in [
        (
            "comptime flag:bool",
            "factory<.char'builtin>(flag,...packet())",
            "type",
        ),
        ("flag:bool", "factory<bool>(flag,...packet())", "runtime"),
    ] {
        let source = format!("typealias Callback<T> = %{{value:T}} fn()->T;
            fn factory(comptime mode:bool,value:T)->Callback<T> where T {{inline if mode {{|| value}} else {{|| value}}}}
            fn packet()->Tuple<bool> {{(true,)}}
            fn choose({parameter})->bool {{(do {{{input}}})()}}");
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(!error.contains("parsing failed"), "{error}");
        assert!(error.contains(diagnostic), "{input}: {error}");
    }
}

#[test]
fn open_callee_factories_keep_effect_handling_before_callback_inputs() {
    let source = "^attrs.effects.direct_passing effect setting()->bool;
        fn positive(value:bool)->bool {value} fn negative(value:bool)->bool {not value}
        typealias Action=fn(bool)->bool; fn factory(comptime flag:bool)->#setting Action {let observed=setting()#;inline if flag {positive} else {negative}}
        fn handler()->bool {true} fn input()->bool {true}
        fn choose(comptime flag:bool)->bool {(do {factory(flag)# {setting()=>handler(),}})(input())}
        fn run()->bool {choose(true) and not choose(false)}";
    run(
        source,
        Value::Bool(true),
        &[
            "run", "choose", "handler", "factory", "input", "positive", "choose", "handler",
            "factory", "input", "negative",
        ],
    );
    for modifier in ["", "inline "] {
        run(
            &source.replace("^attrs.effects.direct_passing ", modifier),
            Value::Bool(true),
            &[
                "run", "choose", "factory", "handler", "input", "positive", "choose", "factory",
                "handler", "input", "negative",
            ],
        );
    }
}

#[test]
fn open_effectful_calls_deliver_plain_values_inside_the_declared_row() {
    run("effect read()->bool;
        fn leaf(comptime flag:bool)->#read bool {inline if flag {read()#} else {false}}
        fn forward(comptime flag:bool)->#read bool {let value=leaf(flag);if value {true} else {false}}
        fn handler()->bool {true} fn run()->bool {forward(true)# {read()=>handler(),}}",
        Value::Bool(true), &["run","forward","leaf","handler"]);
}

#[test]
fn open_effect_rows_do_not_authorize_other_requests_or_staged_selectors() {
    for body in [
        "inline if flag {write()#} else {true}",
        "inline if (read()#) {true} else {false}",
    ] {
        let source = format!(
            "effect read()->bool;effect write()->bool;
            fn leaf(comptime flag:bool)->#read bool {{{body}}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(!error.contains("parsing failed"), "{error}");
        assert!(
            error.contains("effect") || error.contains("comptime"),
            "{body}: {error}"
        );
    }
    let error=compile_with_protocols("effect read()->bool;
        fn leaf(comptime flag:bool,.value:bool=read()#)->#read bool {inline if flag {value} else {false}}",
        &Default::default()).unwrap_err();
    assert!(error.contains("effect"), "{error}");
}

#[test]
fn generic_open_effect_calls_infer_payloads_inside_a_known_row() {
    for call in [
        "leaf(flag,true)",
        "leaf<bool>(flag,true)",
        "leaf(flag,...do {(true,)})",
    ] {
        run(&format!("effect obtain()->T where T;
            fn leaf(comptime flag:bool,value:T)->#obtain<T> T where T {{inline if flag {{obtain<T>()#}} else {{value}}}}
            fn forward(comptime flag:bool)->#obtain<bool> bool {{{call}}}
            fn handler()->bool {{true}} fn run()->bool {{forward(true)# {{obtain<bool>()=>handler(),}}}}"),
            Value::Bool(true), &["run","forward","leaf","handler"]);
    }
}

#[test]
fn open_effect_context_can_supply_the_only_generic_payload_evidence() {
    for (declaration, call) in [
        (
            "fn leaf(comptime flag:bool)->#obtain<T> T where T {inline if flag {obtain<T>()#} else {obtain<T>()#}}",
            "leaf(flag)",
        ),
        ("fn leaf()->#obtain<T> T where T {obtain<T>()#}", "leaf()"),
    ] {
        run(&format!("effect obtain()->T where T;{declaration}
            fn forward(comptime flag:bool)->#obtain<bool> bool {{inline if flag {{{call}}} else {{{call}}}}}
            fn handler()->bool {{true}} fn run()->bool {{forward(true)# {{obtain<bool>()=>handler(),}}}}"),
            Value::Bool(true), &["run","forward","leaf","handler"]);
    }
}

#[test]
fn generic_open_effect_inference_retains_operation_identity_and_type_arguments() {
    for row in ["obtain<.char'builtin>", "other<bool>", "[]"] {
        for call in [
            "leaf(flag,true)",
            "leaf(flag,...do {(true,)})",
            "leaf<bool>(flag,true)",
        ] {
            let source=format!("effect obtain()->T where T;effect other()->T where T;
                fn leaf(comptime flag:bool,value:T)->#obtain<T> bool where T {{let ignored=obtain<T>()#;inline if flag {{true}} else {{false}}}}
                fn forward(comptime flag:bool)->#{row} bool {{{call}}}");
            let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
            assert!(!error.contains("parsing failed"), "{error}");
            assert!(
                error.contains("effect")
                    || error.contains("Row")
                    || error.contains("Operation")
                    || (row == "[]" && error.contains("generic call inference failed")),
                "{row}/{call}: {error}"
            );
        }
    }
}

#[test]
fn generic_open_calls_do_not_erase_explicit_computation_targets() {
    for target in ["other<bool>", "obtain<.char'builtin>"] {
        let source=format!("effect obtain()->T where T;effect other()->T where T;
            fn leaf(comptime flag:bool,value:T)->#obtain<T> T where T {{inline if flag {{obtain<T>()#}} else {{value}}}}
            fn forward(comptime flag:bool)->#obtain<bool> bool {{let pending:#{target} bool=leaf(flag,true);true}}");
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(!error.contains("parsing failed"), "{error}");
        assert!(
            error.contains("inference") || error.contains("type mismatch"),
            "{error}"
        );
    }
}

#[test]
fn open_effect_results_keep_contracts_across_returns_and_handler_completion() {
    let source = "effect poll()->bool;
        fn keep(comptime mode:bool,flag:bool)->result:#poll bool where ensures result==flag'old {
            poll()#;inline if mode {return flag;} else {flag}
        }
        fn forward(comptime mode:bool)->result:bool where ensures result {
            keep(mode,true)# {poll()=>handler(),}
        }
        fn handler()->bool {false} fn run()->bool {forward(true) and forward(false)}";
    run(
        source,
        Value::Bool(true),
        &[
            "run", "forward", "keep", "handler", "forward", "keep", "handler",
        ],
    );
    let early_exit = source.replace("poll()=>handler(),", "poll()=>{return false;},");
    let error = compile_with_protocols(&early_exit, &Default::default()).unwrap_err();
    assert!(
        error.contains("postcondition") || error.contains("ensures"),
        "{error}"
    );
    let error = compile_with_protocols(
        &source.replace("ensures result==flag'old", "ensures result!=flag'old"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("postcondition") || error.contains("ensures"),
        "{error}"
    );
}

#[test]
fn diverging_blocks_check_tails_without_requiring_the_delivered_result_type() {
    for body in [
        "return true; unused()",
        "return true; unused",
        "do {return true; unused()}",
        "let ignored:bool=do {return true; unused()}; false",
        "let ignored:Word=do {return true; false}; false",
        "sink(do {return true; unused()})",
    ] {
        let source = format!(
            "typealias Word=.u8'builtin; fn unused()->Word {{7}} fn sink(value:bool)->bool {{value}} fn run()->bool {{{body}}}"
        );
        run(&source, Value::Bool(true), &["run"]);
        let package = compile(&source);
        let ir = luna::host::llvm_ir(&package, "run", 100_000).unwrap();
        for optimization in [
            luna::host::OptimizationLevel::O0,
            luna::host::OptimizationLevel::O3,
        ] {
            luna::host::Toolchain {
                optimization,
                ..Default::default()
            }
            .execute_test(&ir, std::time::Duration::from_secs(5))
            .unwrap();
        }
    }
}

#[test]
fn divergence_does_not_invent_callable_signatures_or_skip_invalid_tail_checks() {
    let error = compile_with_protocols(
        "fn unused(value:bool)->bool {value} fn run()->bool {(do {return true; unused})(false)}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("diverging callee has no callable signature"),
        "{error}"
    );
    for source in [
        "fn run()->bool {return true; missing()}",
        "fn unused(value:bool)->bool {value} fn run()->bool {return true; unused(1)}",
        "unsafe fn unused()->bool {true} fn run()->bool {return true; unused()}",
        "fn unused()->.u8'builtin {7} fn run(flag:bool)->bool {if flag {return true;} unused()}",
    ] {
        assert!(
            compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}
