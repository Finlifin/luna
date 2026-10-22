#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;
use support::{compile, compile_with_protocols};

#[test]
fn enum_tag_names_preserve_nominal_names_and_evaluate_once() {
    let source = "typealias str=.str'builtin;typealias Symbol=.Symbol'builtin;
        enum Message {.base_type Symbol,.discrete_tag true,ready=.wire_ready,closed=.wire_closed,}
        enum Event where T {network.{ready(T)},closed,}
        fn event()->Event<bool> {Event<bool>.network.ready(true)}
        fn run()->str {event()'tag_name}
        fn encoded()->str {Message.ready'tag_name}
        fn payload()->str {Event<bool>.closed'tag_name}";
    let package = compile(source);
    assert!(package.to_string().contains("(enum-name "));
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::String("network.ready".into())
    );
    assert_eq!(support::calls(&package, &vm), vec!["run", "event"]);
    for (name, expected) in [("encoded", "ready"), ("payload", "closed")] {
        assert_eq!(
            luna::execution::interpret(&package, name, 100_000).unwrap(),
            Value::String(expected.into())
        );
    }
    let staged = compile(&source.replace("fn ", "comptime fn "));
    assert_eq!(
        luna::invocation::invoke(&staged, "run", 100_000).unwrap(),
        Value::String("network.ready".into())
    );
    assert_eq!(
        luna::invocation::invoke(&staged, "encoded", 100_000).unwrap(),
        Value::String("ready".into())
    );
}

#[test]
fn enum_tag_names_feed_comptime_and_staged_parameters() {
    let package = compile(
        "typealias str=.str'builtin;typealias Type=.Type'builtin;
        enum State {ready,closed,}
        const name:str=State.ready'tag_name;
        pure comptime fn Named<name:str>->Type {bool}
        typealias First=Named<State.ready'tag_name>;typealias Second=Named<name>;
        comptime fn same()->bool {First==Second}
        fn pass(comptime name:str)->str {name}
        fn run()->str {pass(State.closed'tag_name)}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "same", 100_000).unwrap(),
        Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::String("closed".into())
    );
    assert!(
        compile_with_protocols(
            "typealias str=.str'builtin;struct S {} fn bad(s:S)->str {s'tag_name}",
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn symbol_tag_name_evaluates_once_in_both_stages_and_preserves_early_return() {
    let source = "typealias Symbol=.Symbol'builtin;typealias str=.str'builtin;
        fn tag()->Symbol {.ready}
        fn run()->str {tag()'tag_name}
        fn early()->str {(do {return \"early\";})'tag_name}";
    let package = compile(source);
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::String("ready".into())
    );
    assert_eq!(support::calls(&package, &vm), vec!["run", "tag"]);
    assert!(package.to_string().contains("(symbol-name "));
    assert_eq!(
        luna::execution::interpret(&package, "early", 100_000).unwrap(),
        Value::String("early".into())
    );
    let staged = compile(&source.replace("fn ", "comptime fn "));
    assert_eq!(
        luna::invocation::invoke(&staged, "run", 100_000).unwrap(),
        Value::String("ready".into())
    );
}

#[test]
fn symbol_tag_name_feeds_constants_normal_forms_and_staged_arguments() {
    let package = compile(
        "typealias Symbol=.Symbol'builtin;typealias str=.str'builtin;typealias Type=.Type'builtin;
        const name:str=.ready'tag_name;
        pure comptime fn Named<text:str>->Type {bool}
        comptime fn Make(tag:Symbol)->Type {Named<tag'tag_name>}
        typealias First=Named<.ready'tag_name>;typealias Second=Named<name>;
        typealias Third=Make(.ready);
        comptime fn same()->bool {First==Second and First==Third}
        fn pass(comptime tag:Symbol)->str {tag'tag_name}
        fn run()->str {pass(.ready)}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "same", 100_000).unwrap(),
        Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::String("ready".into())
    );
    for source in [
        "fn invalid()->bool {true'tag_name}",
        "typealias str=.str'builtin;fn invalid()->str {\"ready\"'tag_name}",
    ] {
        assert!(compile_with_protocols(source, &Default::default()).is_err());
    }
}

#[test]
fn symbols_are_distinct_values_in_both_stages() {
    let source="typealias Symbol=.Symbol'builtin;
        fn identity(value:Symbol)->Symbol {value}
        fn constrained()->T where T, requires T == Symbol {.ready}
        fn run()->bool {let value=.ready;identity(value)==.ready and value != .waiting and constrained<Symbol>() == .ready}
        fn literal()->Symbol {.ready}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let value = luna::execution::interpret(&package, "literal", 100_000).unwrap();
    assert_eq!(value, Value::Symbol("ready".into()));
    assert_ne!(value, Value::String("ready".into()));
    assert_eq!(value.to_string(), "(symbol \"ready\")");
    let staged = compile(&source.replace("fn ", "comptime fn "));
    assert_eq!(
        luna::invocation::invoke(&staged, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn symbol_host_values_keep_type_and_budget_boundaries() {
    use comptime_interpretor::{Budget, ErrorKind, Interpreter};
    let package = compile(
        "typealias Symbol=.Symbol'builtin;
        comptime fn identity(value:Symbol)->Symbol {value}
        comptime fn equal(a:Symbol,b:Symbol)->bool {a==b}",
    );
    let identity = package
        .functions
        .iter()
        .find(|f| f.name == "identity")
        .unwrap()
        .declaration;
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(identity, vec![Value::String("ready".into())])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert_eq!(
        vm.invoke(identity, vec![Value::Symbol("ready".into())])
            .unwrap(),
        Value::Symbol("ready".into())
    );
    let mut limited = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 10,
            ..Budget::default()
        },
    )
    .unwrap();
    let equal = package
        .functions
        .iter()
        .find(|f| f.name == "equal")
        .unwrap()
        .declaration;
    assert!(
        limited
            .invoke(
                equal,
                vec![
                    Value::Symbol("x".repeat(4096).into()),
                    Value::Symbol("x".repeat(4096).into())
                ]
            )
            .is_err()
    );
    assert_eq!(
        nir::Term::Symbol("ready".into()).sort(),
        nir::Sort::ComptimeValue
    );
    assert_ne!(
        nir::Term::Symbol("ready".into()),
        nir::Term::String("ready".into())
    );
    assert_eq!(
        elaboration::ptc::TypeKind::Symbol.evaluate(&nir::Term::Primitive(nir::Primitive::Symbol)),
        Some(true)
    );
    assert_eq!(
        elaboration::ptc::TypeKind::Symbol.evaluate(&nir::Term::Primitive(nir::Primitive::Str)),
        Some(false)
    );
    use elaboration::ptc::{Context, Formula, Outcome, TypeKind};
    let mut context = Context::default();
    for formula in [
        Formula::kind(
            nir::Term::Primitive(nir::Primitive::Symbol),
            TypeKind::Symbol,
        ),
        Formula::kind(nir::Term::Primitive(nir::Primitive::Str), TypeKind::Symbol).negate(),
    ] {
        assert!(matches!(
            context.prove(formula, Default::default()),
            Outcome::Entailed(_)
        ));
    }
}

#[test]
fn symbols_match_literals_and_preserve_enum_contexts() {
    let package = compile(
        "typealias Symbol=.Symbol'builtin;
        enum State {ready,waiting}
        fn classify(value:Symbol)->bool {value match {.ready=>true,_=>false}}
        fn state(value:State)->bool {value match {.ready=>true,.waiting=>false}}
        fn run()->bool {classify(.ready) and not classify(.waiting) and state(.ready)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for source in [
        "typealias Symbol=.Symbol'builtin;fn wrong()->Symbol {\"ready\"}",
        "typealias Symbol=.Symbol'builtin;fn wrong()->bool {.ready < .waiting}",
        "typealias Symbol=.Symbol'builtin;fn wrong(value:Symbol)->bool {value match {.ready=>true}}",
    ] {
        assert!(
            compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn symbols_materialize_staged_runtime_arguments() {
    let package = compile(
        "typealias Symbol=.Symbol'builtin;
        const selected:Symbol=.ready;
        comptime fn choose()->Symbol {selected}
        fn pass(comptime tag:Symbol)->Symbol {tag}
        fn run()->bool {pass(choose())==.ready}",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn symbols_survive_constants_and_opaque_normal_form_inputs() {
    let package = compile(
        "typealias Symbol=.Symbol'builtin;typealias Type=.Type'builtin;
        const selected:Symbol=.ready;
        pure comptime fn Tagged<name:Symbol>->Type {bool}
        typealias Ready=Tagged<selected>;
        typealias AlsoReady=Tagged<.ready>;
        typealias Waiting=Tagged<.waiting>;
        comptime fn run()->bool {Ready==AlsoReady}
        comptime fn name()->Symbol {Waiting match {Tagged<name> => name,_=>.missing}}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "name", 100_000).unwrap(),
        Value::Symbol("waiting".into())
    );
}

#[test]
fn negated_symbol_patterns_execute_in_both_phases_without_exporting_bindings() {
    let source = "typealias Symbol=.Symbol'builtin;
        fn inspect(value:Symbol)->bool {value match {not (.ready or .closed)=>true,_=>false}}
        fn run()->bool {inspect(.working) and not inspect(.ready) and not inspect(.closed)}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let staged = compile(&source.replace("fn ", "comptime fn "));
    assert_eq!(
        luna::invocation::invoke(&staged, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let invalid = "typealias Symbol=.Symbol'builtin;fn bad(value:Symbol)->Symbol {value match {not (.ready as hidden)=>hidden,_=>.closed}}";
    assert!(compile_with_protocols(invalid, &Default::default()).is_err());
}

#[test]
fn negated_symbol_products_preserve_subject_for_later_candidates() {
    let source="typealias Symbol=.Symbol'builtin;trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for (Symbol,bool) {}
        fn inspect(value:(Symbol,bool))->bool {value match {not (.ready,true)=>false,(tag,flag)=>tag==.ready and flag}}
        fn run()->bool {inspect((.ready,true)) and not inspect((.working,true))}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
    let no_copy = compile(&source.replace("impl Duplicate for (Symbol,bool) {}", ""));
    assert!(
        luna::execution::interpret_with_protocols(&no_copy, "run", "Duplicate", "Retire", 100_000)
            .is_err()
    );
}

#[test]
fn symbol_coverage_proves_complements_and_guard_decisions() {
    for arms in [
        ".ready=>true,not .ready=>false",
        "not (.ready or .closed)=>false,.ready=>true,.closed=>false",
        "tag if tag==.ready=>true,tag if tag!=.ready=>false",
        "tag if do {let selected=tag==.ready;selected}=>true,not .ready=>false",
        "tag if tag match {.ready=>true,_=>false}=>true,not .ready=>false",
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;fn inspect(tag:Symbol)->bool {{tag match {{{arms}}}}}
            fn run()->bool {{inspect(.ready) and not inspect(.unknown)}}"
        );
        assert_eq!(
            luna::execution::interpret(&compile(&source), "run", 100_000).unwrap(),
            Value::Bool(true),
            "{arms}"
        );
    }
}

#[test]
fn symbol_coverage_preserves_residual_values_and_tuple_correlations() {
    let source = "typealias Symbol=.Symbol'builtin;
        fn inspect(value:(Symbol,bool))->bool {value match {
            (.ready,true)=>true,(.ready,false)=>false,(not .ready,_)=>false
        }} fn run()->bool {inspect((.ready,true)) and not inspect((.other,true))}";
    assert_eq!(
        luna::execution::interpret(&compile(source), "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        source.replace("(.ready,false)=>false,", ""),
        "typealias Symbol=.Symbol'builtin;fn bad(value:Symbol)->bool {value match {.ready=>true,.closed=>false}}".into(),
        "typealias Symbol=.Symbol'builtin;fn unknown(value:Symbol)->bool {true} fn bad(value:Symbol)->bool {value match {tag if unknown(tag)=>true}}".into(),
    ] {
        assert!(compile_with_protocols(&invalid,&Default::default()).is_err(),"{invalid}");
    }
}

#[test]
fn symbol_coverage_short_circuit_does_not_turn_unknown_calls_into_facts() {
    let source = "typealias Symbol=.Symbol'builtin;fn unknown(value:Symbol)->bool {false}
        fn inspect(value:Symbol)->bool {value match {
            tag if tag==.ready or unknown(tag)=>true,not .ready=>false
        }} fn run()->bool {inspect(.ready) and not inspect(.other)}";
    assert_eq!(
        luna::execution::interpret(&compile(source), "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for arms in [
        "tag if tag==.ready or unknown(tag)=>true",
        "not (tag if unknown(tag))=>true",
        "tag if unknown(tag) or not unknown(tag)=>true",
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;fn unknown(value:Symbol)->bool {{true}} fn bad(value:Symbol)->bool {{value match {{{arms}}}}}"
        );
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn completed_symbol_constants_keep_coverage_through_code_generation() {
    let source = "typealias Symbol=.Symbol'builtin;
        const selected:Symbol=.ready;
        comptime fn build()-><input:Symbol|-bool> {
            '(input match {tag if tag==selected=>true,tag if tag!=selected=>false})
        }
        fn inspect(input:Symbol)->bool {~[input=input]<build()>}
        fn run()->bool {inspect(.ready) and not inspect(.other)}";
    assert_eq!(
        luna::execution::interpret(&compile(source), "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let invalid = source.replace("tag if tag==selected=>true,", "");
    assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn symbol_coverage_relates_runtime_keys_without_knowing_their_names() {
    let source = "typealias Symbol=.Symbol'builtin;
        fn compare(value:Symbol,key:Symbol)->bool {value match {
            tag if tag==key=>true,tag if tag!=key=>false
        }}
        fn run()->bool {compare(.ready,.ready) and not compare(.ready,.other)}";
    assert_eq!(
        luna::execution::interpret(&compile(source), "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn symbol_coverage_quantifies_aliasing_between_multiple_runtime_keys() {
    let source = "typealias Symbol=.Symbol'builtin;
        fn classify(value:Symbol,a:Symbol,b:Symbol)->Symbol {value match {
            tag if tag==a=>.one,
            tag if tag!=a and tag==b=>.two,
            tag if tag!=a and tag!=b=>.three
        }}
        fn run()->bool {
            classify(.x,.x,.x)==.one and classify(.x,.a,.x)==.two
            and classify(.x,.a,.b)==.three and classify(.x,.a,.a)==.three
        }";
    assert_eq!(
        luna::execution::interpret(&compile(source), "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for arms in [
        "tag if tag==a=>.one,tag if tag!=b=>.two",
        "tag if tag!=a=>.one,tag if tag!=b=>.two",
        "tag if tag==a=>.one,tag if tag==b=>.two",
    ] {
        let invalid = format!(
            "typealias Symbol=.Symbol'builtin; fn bad(value:Symbol,a:Symbol,b:Symbol)->Symbol {{value match {{{arms}}}}}"
        );
        assert!(
            compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{arms}"
        );
    }
}

#[test]
fn symbol_guards_partition_external_booleans_and_symbol_equalities() {
    let source = "typealias Symbol=.Symbol'builtin;
        fn compare(value:Symbol,key:Symbol,flag:bool)->bool {value match {
            tag if flag and tag==key=>true,
            tag if flag and tag!=key=>false,
            _ if not flag=>false
        }}
        fn run()->bool {compare(.x,.x,true) and not compare(.x,.y,true)
            and not compare(.x,.x,false) and not compare(.x,.y,false)}";
    assert_eq!(
        luna::execution::interpret(&compile(source), "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for arms in [
        "_ if flag=>true,_ if not other=>false",
        "tag if flag and tag==key=>true,_ if not flag=>false",
        "_ if unknown(flag)=>true,_ if not unknown(flag)=>false",
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;
            fn unknown(flag:bool)->bool {{flag}}
            fn bad(value:Symbol,key:Symbol,flag:bool,other:bool)->bool {{value match {{{arms}}}}}"
        );
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "{arms}"
        );
    }
}

#[test]
fn symbol_guard_boolean_aliases_preserve_lexical_binding_identity() {
    for guard in [
        "do {let flag=not flag;flag}",
        "flag match {true=>false,false=>true}",
        "if flag {false} else {true}",
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;
            fn compare(value:Symbol,flag:bool)->bool {{value match {{
                _ if flag=>true,_ if {guard}=>false
            }}}}
            fn run()->bool {{compare(.x,true) and not compare(.x,false)}}"
        );
        assert_eq!(
            luna::execution::interpret(&compile(&source), "run", 100_000).unwrap(),
            Value::Bool(true),
            "{guard}"
        );
    }
}

#[test]
fn symbol_guard_local_state_tracks_assignments_and_pattern_bindings() {
    for guard in [
        "do {var same=tag==key;same=not same;same}",
        "do {var name=tag;name=key;name==key and tag!=name}",
        "do {let _ as name=key;tag!=name}",
        "do {let name if true=key else {return false;};tag!=name}",
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;
            fn compare(value:Symbol,key:Symbol)->bool {{value match {{
                tag if tag==key=>true,tag if {guard}=>false
            }}}}
            fn run()->bool {{compare(.x,.x) and not compare(.x,.y)}}"
        );
        assert_eq!(
            luna::execution::interpret(&compile(&source), "run", 100_000).unwrap(),
            Value::Bool(true),
            "{guard}"
        );
        let staged = compile(&source.replace("fn ", "comptime fn "));
        assert_eq!(
            luna::invocation::invoke(&staged, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn symbol_guard_local_state_does_not_reuse_overwritten_or_unknown_values() {
    for guard in [
        "do {var same=tag!=key;same=false;same}",
        "do {var same=tag!=key;same=unknown(tag);same}",
        "do {flag=true;flag}",
        "do {let name if false=key else {return false;};tag!=name}",
        "do {let .fixed as name=key else {return false;};tag!=name}",
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;
            fn unknown(value:Symbol)->bool {{true}}
            fn bad(value:Symbol,key:Symbol)->bool {{var flag=false;value match {{
                tag if tag==key=>true,tag if {guard}=>false
            }}}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("non-exhaustive match"), "{guard}: {error}");
    }
}

#[test]
fn symbol_guards_match_and_destructure_constructed_tuples() {
    for guard in [
        "(tag,flag) matches (.ready,true)",
        "(tag,flag) match {(.ready,true)=>true,_=>false}",
        "do {let (name,enabled)=(tag,flag);name==.ready and enabled}",
        "do {let ((name,enabled),key)=((tag,flag),.ready);name==key and enabled}",
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;
            fn inspect(value:Symbol,flag:bool)->bool {{value match {{
                tag if {guard}=>true,tag if tag!=.ready or not flag=>false
            }}}}
            fn run()->bool {{inspect(.ready,true) and not inspect(.ready,false)
                and not inspect(.other,true) and not inspect(.other,false)}}"
        );
        assert_eq!(
            luna::execution::interpret(&compile(&source), "run", 100_000).unwrap(),
            Value::Bool(true),
            "{guard}"
        );
        let staged = compile(&source.replace("fn ", "comptime fn "));
        assert_eq!(
            luna::invocation::invoke(&staged, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let checked = compile(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}} impl Duplicate for (Symbol,bool) {{}} impl Duplicate for ((Symbol,bool),Symbol) {{}} {source}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &checked,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn tuple_guard_partitions_keep_field_correlations_and_unknown_elements() {
    for guard in [
        "(tag,flag) matches (.ready,true)",
        "not ((tag,flag) matches (.ready,_))",
        "(tag,flag) match {(.ready,true)=>true,(not .ready,false)=>true,_=>false}",
        "(tag,unknown()) matches (.ready,true)",
        "do {let (name,enabled)=(tag,flag);name==.ready or enabled}",
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;
            fn unknown()->bool {{true}}
            fn bad(value:Symbol,flag:bool)->bool {{value match {{tag if {guard}=>true}}}}"
        );
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "{guard}"
        );
    }
}

#[test]
fn symbol_guards_preserve_completed_record_labels_and_projections() {
    for guard in [
        "tag==CONFIG.key and CONFIG.enabled",
        "do {let {key:name,enabled:flag}=CONFIG;tag==name and flag}",
        "CONFIG match {{key:name,enabled:true}=>tag==name,_=>false}",
    ] {
        let source=format!("typealias Symbol=.Symbol'builtin;
            comptime fn schema(...properties:T)->T where T {{properties}}
            const CONFIG=schema {{enabled:true,key:.ready}};
            comptime fn inspect(value:Symbol)->bool {{value match {{tag if {guard}=>true,not .ready=>false}}}}
            comptime fn run()->bool {{inspect(.ready) and not inspect(.other)}}");
        let runtime = source
            .replace("comptime fn inspect", "fn inspect")
            .replace("comptime fn run", "fn run");
        assert!(
            compile_with_protocols(&runtime, &Default::default())
                .unwrap_err()
                .contains("Splice")
        );
        assert_eq!(
            luna::invocation::invoke(&compile(&source), "run", 100_000).unwrap(),
            Value::Bool(true),
            "{guard}"
        );
    }
}

#[test]
fn completed_record_guard_updates_preserve_copies_and_sibling_fields() {
    for (guard, selected) in [
        (
            "do {var item=CONFIG;item.key=.other;tag==item.key}",
            ".other",
        ),
        (
            "do {var item=CONFIG;let before=item;item.key=.other;tag==before.key}",
            ".ready",
        ),
        (
            "do {var item=CONFIG;item.nested.key=.other;tag==item.nested.key and item.enabled}",
            ".other",
        ),
        (
            "do {var item=CONFIG;let before=item.nested;item.nested.key=.other;tag==before.key}",
            ".ready",
        ),
    ] {
        let source=format!("typealias Symbol=.Symbol'builtin;
            comptime fn schema(...properties:T)->T where T {{properties}}
            const CONFIG=schema {{key:.ready,enabled:true,nested:schema {{key:.ready}}}};
            comptime fn inspect(value:Symbol)->bool {{value match {{tag if {guard}=>true,tag if tag!={selected}=>false}}}}
            comptime fn run()->bool {{inspect({selected}) and not inspect(.absent)}}");
        assert_eq!(
            luna::invocation::invoke(&compile(&source), "run", 100_000).unwrap(),
            Value::Bool(true),
            "{guard}"
        );
    }
}

#[test]
fn completed_record_guard_writes_do_not_keep_stale_field_facts() {
    for guard in [
        "do {var item=CONFIG;item.key=.other;tag==item.key}",
        "do {var item=CONFIG;item.enabled=false;tag==item.key and item.enabled}",
    ] {
        let source=format!("typealias Symbol=.Symbol'builtin;
            comptime fn schema(...properties:T)->T where T {{properties}}
            const CONFIG=schema {{key:.ready,enabled:true}};
            comptime fn bad(value:Symbol)->bool {{value match {{tag if {guard}=>true,not .ready=>false}}}}");
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("non-exhaustive match"), "{guard}: {error}");
    }
}

#[test]
fn symbol_guards_observe_direct_nominal_construction_and_variant_selection() {
    for guard in [
        "Entry {name:tag,enabled:flag} matches {name:.ready,enabled:true}",
        "do {let entry=Entry {name:tag,enabled:flag};entry.name==.ready and entry.enabled}",
        "Choice.item(tag,flag) match {Choice.item(.ready,true)=>true,_=>false}",
        "(if flag {Choice.item(tag,true)} else {Choice.empty}) match {Choice.item(.ready,true)=>true,_=>false}",
        "Choice.item(tag,flag) match {Choice.empty=>false,Choice.item(name,enabled)=>name==.ready and enabled}",
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;
            struct Entry {{name:Symbol,enabled:bool}}
            enum Choice {{empty,item(Symbol,bool),}}
            fn inspect(value:Symbol,flag:bool)->bool {{value match {{
                tag if {guard}=>true,tag if tag!=.ready or not flag=>false
            }}}}
            fn run()->bool {{inspect(.ready,true) and not inspect(.ready,false)
                and not inspect(.other,true) and not inspect(.other,false)}}"
        );
        let checked = compile(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}} impl Duplicate for Entry {{}} impl Duplicate for Choice {{}} {source}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &checked,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            Value::Bool(true),
            "{guard}"
        );
        assert_eq!(
            luna::execution::interpret(&compile(&source), "run", 100_000).unwrap(),
            Value::Bool(true),
            "{guard}"
        );
        assert_eq!(
            luna::invocation::invoke(
                &compile(&source.replace("fn ", "comptime fn ")),
                "run",
                100_000
            )
            .unwrap(),
            Value::Bool(true),
            "{guard}"
        );
    }
}

#[test]
fn nominal_guard_evidence_keeps_variant_and_payload_holes() {
    for guard in [
        "not (Choice.item(tag,true) matches Choice.item(.ready,_))",
        "Choice.item(tag,flag) matches Choice.item(.ready,true)",
        "(if flag {Choice.item(tag,true)} else {Choice.empty}) matches Choice.item(_,_)",
        "Entry {name:tag,enabled:unknown()} matches {enabled:true}",
    ] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;
            struct Entry {{name:Symbol,enabled:bool}}
            enum Choice {{empty,item(Symbol,bool),}}
            fn unknown()->bool {{true}}
            fn bad(value:Symbol,flag:bool)->bool {{value match {{tag if {guard}=>true}}}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("non-exhaustive match"), "{guard}: {error}");
    }
}

#[test]
fn nominal_guard_cleanup_does_not_erase_failing_destructors() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        struct Entry {name:Symbol}
        fn cleanup() {assumes false;}
        impl Retire for Entry {fn drop(*self) {cleanup();}}
        fn inspect(tag:Symbol)->bool {Entry {name:tag} matches {name:.ready}}
        fn run()->bool {inspect(.ready)}";
    let package = compile(source);
    let result =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000);
    assert!(
        result.is_err(),
        "a destructor must not disappear through release-only cleanup elision"
    );
}
