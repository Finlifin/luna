#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use support::{compile, compile_with_protocols};
use comptime_interpretor::Value;

#[test]
fn enum_tag_names_preserve_nominal_names_and_evaluate_once() {
    let source = "typealias str=.str'builtin;typealias Symbol=.Symbol'builtin;
        enum Message {.base_type Symbol,.discrete_tag true,ready=.wire_ready,closed=.wire_closed,}
        enum Event where T {network.{ready(T)},closed,}
        fn event()->Event<bool> {Event<bool>.network.ready(true)}
        fn run()->str {event()'tag_name}
        fn encoded()->str {Message.ready'tag_name}
        fn payload()->str {Event<bool>.closed'tag_name}";
    let package=compile(source);
    assert!(package.to_string().contains("(enum-name "));
    let graph=luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let mut vm=comptime_interpretor::Interpreter::from_iir(&graph,Default::default()).unwrap();
    assert_eq!(vm.invoke_instance(&graph.roots[0],vec![]).unwrap(),Value::String("network.ready".into()));
    assert_eq!(support::calls(&package,&vm),vec!["run","event"]);
    for (name, expected) in [("encoded","ready"),("payload","closed")] {
        assert_eq!(luna::execution::interpret(&package,name,100_000).unwrap(),Value::String(expected.into()));
    }
    let staged=compile(&source.replace("fn ","comptime fn "));
    assert_eq!(luna::invocation::invoke(&staged,"run",100_000).unwrap(),Value::String("network.ready".into()));
    assert_eq!(luna::invocation::invoke(&staged,"encoded",100_000).unwrap(),Value::String("ready".into()));
}

#[test]
fn enum_tag_names_feed_comptime_and_staged_parameters() {
    let package=compile("typealias str=.str'builtin;typealias Type=.Type'builtin;
        enum State {ready,closed,}
        const name:str=State.ready'tag_name;
        pure comptime fn Named<name:str>->Type {bool}
        typealias First=Named<State.ready'tag_name>;typealias Second=Named<name>;
        comptime fn same()->bool {First==Second}
        fn pass(comptime name:str)->str {name}
        fn run()->str {pass(State.closed'tag_name)}");
    assert_eq!(luna::invocation::invoke(&package,"same",100_000).unwrap(),Value::Bool(true));
    let graph=luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let mut vm=comptime_interpretor::Interpreter::from_iir(&graph,Default::default()).unwrap();
    assert_eq!(vm.invoke_instance(&graph.roots[0],vec![]).unwrap(),Value::String("closed".into()));
    assert!(compile_with_protocols("typealias str=.str'builtin;struct S {} fn bad(s:S)->str {s'tag_name}",&Default::default()).is_err());
}


#[test]
fn symbol_tag_name_evaluates_once_in_both_stages_and_preserves_early_return() {
    let source="typealias Symbol=.Symbol'builtin;typealias str=.str'builtin;
        fn tag()->Symbol {.ready}
        fn run()->str {tag()'tag_name}
        fn early()->str {(do {return \"early\";})'tag_name}";
    let package=compile(source);
    let graph=luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let mut vm=comptime_interpretor::Interpreter::from_iir(&graph,Default::default()).unwrap();
    assert_eq!(vm.invoke_instance(&graph.roots[0],vec![]).unwrap(),Value::String("ready".into()));
    assert_eq!(support::calls(&package,&vm),vec!["run","tag"]);
    assert!(package.to_string().contains("(symbol-name "));
    assert_eq!(luna::execution::interpret(&package,"early",100_000).unwrap(),Value::String("early".into()));
    let staged=compile(&source.replace("fn ","comptime fn "));
    assert_eq!(luna::invocation::invoke(&staged,"run",100_000).unwrap(),Value::String("ready".into()));
}

#[test]
fn symbol_tag_name_feeds_constants_normal_forms_and_staged_arguments() {
    let package=compile("typealias Symbol=.Symbol'builtin;typealias str=.str'builtin;typealias Type=.Type'builtin;
        const name:str=.ready'tag_name;
        pure comptime fn Named<text:str>->Type {bool}
        comptime fn Make(tag:Symbol)->Type {Named<tag'tag_name>}
        typealias First=Named<.ready'tag_name>;typealias Second=Named<name>;
        typealias Third=Make(.ready);
        comptime fn same()->bool {First==Second and First==Third}
        fn pass(comptime tag:Symbol)->str {tag'tag_name}
        fn run()->str {pass(.ready)}");
    assert_eq!(luna::invocation::invoke(&package,"same",100_000).unwrap(),Value::Bool(true));
    let graph=luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let mut vm=comptime_interpretor::Interpreter::from_iir(&graph,Default::default()).unwrap();
    assert_eq!(vm.invoke_instance(&graph.roots[0],vec![]).unwrap(),Value::String("ready".into()));
    for source in ["fn invalid()->bool {true'tag_name}", "typealias str=.str'builtin;fn invalid()->str {\"ready\"'tag_name}"] {
        assert!(compile_with_protocols(source,&Default::default()).is_err());
    }
}

#[test]
fn symbols_are_distinct_values_in_both_stages() {
    let source="typealias Symbol=.Symbol'builtin;
        fn identity(value:Symbol)->Symbol {value}
        fn constrained()->T where T, requires T == Symbol {.ready}
        fn run()->bool {let value=.ready;identity(value)==.ready and value != .waiting and constrained<Symbol>() == .ready}
        fn literal()->Symbol {.ready}";
    let package=compile(source);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    let value=luna::execution::interpret(&package,"literal",100_000).unwrap();
    assert_eq!(value,Value::Symbol("ready".into()));
    assert_ne!(value,Value::String("ready".into()));
    assert_eq!(value.to_string(),"(symbol \"ready\")");
    let staged=compile(&source.replace("fn ","comptime fn "));
    assert_eq!(luna::invocation::invoke(&staged,"run",100_000).unwrap(),Value::Bool(true));
}

#[test]
fn symbol_host_values_keep_type_and_budget_boundaries() {
    use comptime_interpretor::{Interpreter, Budget, ErrorKind};
    let package=compile("typealias Symbol=.Symbol'builtin;
        comptime fn identity(value:Symbol)->Symbol {value}
        comptime fn equal(a:Symbol,b:Symbol)->bool {a==b}");
    let identity=package.functions.iter().find(|f| f.name=="identity").unwrap().declaration;
    let mut vm=Interpreter::for_comptime(&package,Budget::default()).unwrap();
    assert_eq!(vm.invoke(identity,vec![Value::String("ready".into())]).unwrap_err().kind,ErrorKind::Type);
    assert_eq!(vm.invoke(identity,vec![Value::Symbol("ready".into())]).unwrap(),Value::Symbol("ready".into()));
    let mut limited=Interpreter::for_comptime(&package,Budget {steps:10,..Budget::default()}).unwrap();
    let equal=package.functions.iter().find(|f| f.name=="equal").unwrap().declaration;
    assert!(limited.invoke(equal,vec![Value::Symbol("x".repeat(4096).into()),Value::Symbol("x".repeat(4096).into())]).is_err());
    assert_eq!(nir::Term::Symbol("ready".into()).sort(),nir::Sort::ComptimeValue);
    assert_ne!(nir::Term::Symbol("ready".into()),nir::Term::String("ready".into()));
    assert_eq!(elaboration::ptc::TypeKind::Symbol.evaluate(&nir::Term::Primitive(nir::Primitive::Symbol)),Some(true));
    assert_eq!(elaboration::ptc::TypeKind::Symbol.evaluate(&nir::Term::Primitive(nir::Primitive::Str)),Some(false));
    use elaboration::ptc::{Context,Formula,TypeKind,Outcome};
    let mut context=Context::default();
    for formula in [
        Formula::kind(nir::Term::Primitive(nir::Primitive::Symbol),TypeKind::Symbol),
        Formula::kind(nir::Term::Primitive(nir::Primitive::Str),TypeKind::Symbol).negate(),
    ] {assert!(matches!(context.prove(formula,Default::default()),Outcome::Entailed(_)));}
}

#[test]
fn symbols_match_literals_and_preserve_enum_contexts() {
    let package=compile("typealias Symbol=.Symbol'builtin;
        enum State {ready,waiting}
        fn classify(value:Symbol)->bool {value match {.ready=>true,_=>false}}
        fn state(value:State)->bool {value match {.ready=>true,.waiting=>false}}
        fn run()->bool {classify(.ready) and not classify(.waiting) and state(.ready)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    for source in [
        "typealias Symbol=.Symbol'builtin;fn wrong()->Symbol {\"ready\"}",
        "typealias Symbol=.Symbol'builtin;fn wrong()->bool {.ready < .waiting}",
        "typealias Symbol=.Symbol'builtin;fn wrong(value:Symbol)->bool {value match {.ready=>true}}",
    ] {assert!(compile_with_protocols(source,&Default::default()).is_err(),"{source}");}
}

#[test]
fn symbols_materialize_staged_runtime_arguments() {
    let package=compile("typealias Symbol=.Symbol'builtin;
        const selected:Symbol=.ready;
        comptime fn choose()->Symbol {selected}
        fn pass(comptime tag:Symbol)->Symbol {tag}
        fn run()->bool {pass(choose())==.ready}");
    let graph=luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let mut vm=comptime_interpretor::Interpreter::from_iir(&graph,Default::default()).unwrap();
    assert_eq!(vm.invoke_instance(&graph.roots[0],vec![]).unwrap(),Value::Bool(true));
}

#[test]
fn symbols_survive_constants_and_opaque_normal_form_inputs() {
    let package=compile("typealias Symbol=.Symbol'builtin;typealias Type=.Type'builtin;
        const selected:Symbol=.ready;
        pure comptime fn Tagged<name:Symbol>->Type {bool}
        typealias Ready=Tagged<selected>;
        typealias AlsoReady=Tagged<.ready>;
        typealias Waiting=Tagged<.waiting>;
        comptime fn run()->bool {Ready==AlsoReady}
        comptime fn name()->Symbol {Waiting match {Tagged<name> => name,_=>.missing}}");
    assert_eq!(luna::invocation::invoke(&package,"run",100_000).unwrap(),Value::Bool(true));
    assert_eq!(luna::invocation::invoke(&package,"name",100_000).unwrap(),Value::Symbol("waiting".into()));
}
