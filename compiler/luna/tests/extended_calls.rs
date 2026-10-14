//! Extended property collection and positional expansion share one call plan.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{calls, compile, compile_with_protocols, id};

fn run(source: &str, expected: Value, order: &[&str]) {
    let package = compile(source);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(&package, "run"), vec![]).unwrap(), expected);
    assert_eq!(calls(&package, &vm), order);
    // Also execute the completed runtime artifact, including phase and
    // materialization checks that direct NIR interpretation doesn't provide.
    assert_eq!(luna::execution::interpret(&package, "run", 100_000).unwrap(), expected);
}

#[test]
fn mixed_property_and_tuple_spreads_preserve_every_producer_in_source_order() {
    for signature in ["...items:(bool,bool),...properties:T", "...properties:T,...items:(bool,bool)"] {
        for (inputs, order) in [
            ("...packet(),flag:mark()", vec!["run", "packet", "mark", "take"]),
            ("flag:mark(),...packet()", vec!["run", "mark", "packet", "take"]),
            ("...do {(first(),)},flag:mark(),...do {(last(),)}", vec!["run", "first", "mark", "last", "take"]),
            ("...(first(),),flag:mark(),...(last(),)", vec!["run", "first", "mark", "last", "take"]),
        ] {
            run(&format!("fn packet()->(bool,bool) {{(true,false)}} fn first()->bool {{true}} fn last()->bool {{false}} fn mark()->bool {{true}} fn take({signature})->(T,(bool,bool)) where T {{(properties,items)}} fn run()->bool {{let (properties,(a,b))=take {{{inputs}}};properties.flag and a and not b}}"), Value::Bool(true), &order);
        }
    }
}

#[test]
fn empty_packets_still_run_between_properties_and_before_defaults() {
    for (inputs, order) in [
        ("...empty(),first:left(),second:right()", vec!["run", "empty", "left", "right", "take"]),
        ("first:left(),...empty(),second:right()", vec!["run", "left", "empty", "right", "take"]),
        ("first:left(),second:right(),...empty()", vec!["run", "left", "right", "empty", "take"]),
    ] {
        run(&format!("fn empty(...items:T)->T where T {{items}} fn left()->bool {{true}} fn right()->bool {{false}} fn take(...properties:T,.flag:bool=true)->T where T {{properties}} fn run()->bool {{let p=take {{{inputs}}};p.first and not p.second}}"), Value::Bool(true), &order);
    }
}

#[test]
fn packet_context_reaches_numeric_literals_and_property_callbacks() {
    run("comptime fn schema(...properties:T)->T where T {properties} fn negate(x:bool)->bool {not x} const SAMPLE=schema {action:negate};typealias Options=SAMPLE'type;fn take(...items:(.u8'builtin,bool),...properties:Options)->.u8'builtin {let (n,b)=items;if properties.action(b) {n} else {0}} fn run()->.u8'builtin {take {action:|x| not x,...do {(7,false)}}}", Value::UnsignedInteger(7), &["run", "take"]);
}

#[test]
fn peer_property_inference_is_retained_without_replaying_producers() {
    run("fn keep(...properties:T)->T where T {properties} fn select(...properties:T,model:T,flag:bool)->T where T {properties} fn mark()->bool {true} fn pair()->(bool,) {(true,)} fn run()->bool {let model=keep {flag:false};let result=select {flag:mark(),model,...pair()};result.flag}", Value::Bool(true), &["run", "keep", "mark", "pair", "select"]);
}

#[test]
fn method_receivers_precede_mixed_inputs() {
    for declaration in [
        "impl Holder {fn take(self,...items:(.u8'builtin,bool),...properties:Options)->.u8'builtin {let (n,b)=items;if b and properties.flag {n} else {0}}}",
        "trait Take {fn take(self,...items:(.u8'builtin,bool),...properties:Options)->.u8'builtin;} impl Take for Holder {fn take(self,...items:(.u8'builtin,bool),...properties:Options)->.u8'builtin {let (n,b)=items;if b and properties.flag {n} else {0}}}",
    ] {
        let access = if declaration.starts_with("trait") { "make().as(Take).take" } else { "make().take" };
        run(&format!("comptime fn schema(...properties:T)->T where T {{properties}} const SAMPLE=schema {{flag:true}};typealias Options=SAMPLE'type;struct Holder {{}} {declaration} fn make()->Holder {{Holder {{}}}} fn mark()->bool {{true}} fn run()->.u8'builtin {{{access} {{flag:mark(),...do {{(7,true)}}}}}}"), Value::UnsignedInteger(7), &["run", "make", "mark", "take"]);
    }
}

#[test]
fn malformed_mixed_inputs_reject_instead_of_changing_routes() {
    for inputs in ["flag:true,...(true,false,true)", "flag:true,...false", "flag:true,...(true,false),flag:false"] {
        let error = compile_with_protocols(&format!("fn take(...items:(bool,bool),...properties:T)->T where T {{properties}} fn run() {{take {{{inputs}}};}}"), &Default::default()).unwrap_err();
        assert!(error.contains("positional") || error.contains("type mismatch") || error.contains("more than once"), "{error}");
    }
}

#[test]
fn expanded_collections_preserve_affine_packet_and_property_moves() {
    use elaboration::verification::moves::{analyze_in_package, Delivery};
    for (body, valid) in [
        ("take {...packet,item:third};", true),
        ("take {item:third,...packet};", true),
        ("take {...packet,item:third};let again=packet;", false),
        ("take {item:third,...packet};let again=third;", false),
        ("take {...packet,item:first};", false),
    ] {
        let package = compile(&format!("struct Ticket {{}} fn take(...items:(Ticket,Ticket),...properties:T) where T {{}} fn run(first:Ticket,second:Ticket,third:Ticket) {{let packet=(first,second);{body}}}"));
        let function = package.functions.iter().find(|f| f.name == "run").unwrap();
        let result = analyze_in_package(&package, function, |ty| Ok(if matches!(ty, nir::Term::Primitive(_)) { Delivery::Copy } else { Delivery::Move }), 100_000);
        assert_eq!(result.is_ok(), valid, "{body}: {result:?}");
        if let Err(failure) = result { assert!(failure.reason.contains("unavailable"), "{failure}"); }
    }
}

#[test]
fn exits_inside_a_packet_skip_later_properties_and_the_call() {
    run("fn first()->bool {true} fn later()->bool {false} fn take(...items:(bool,bool),...properties:T)->bool where T {false} fn run()->bool {take {flag:first(),...do {return true;(false,false)},tail:later()}}", Value::Bool(true), &["run", "first"]);
    run("fn packet()->(bool,bool) {(true,false)} fn later()->bool {false} fn take(...items:(bool,bool),...properties:T)->bool where T {false} fn run()->bool {take {flag:do {return true;false},...packet(),tail:later()}}", Value::Bool(true), &["run"]);
}

#[test]
fn contextual_result_and_explicit_type_arguments_choose_collectors() {
    for call in ["gather {flag:true,...packet()}", "gather<(bool,bool),Options> {flag:true,...packet()}"] {
        let package = compile(&format!("comptime fn schema(...properties:T)->T where T {{properties}} const SAMPLE=schema {{flag:true}};typealias Options=SAMPLE'type;fn packet()->(bool,bool) {{(true,false)}} fn gather(...items:A,...properties:B)->(A,B) where A,B {{(items,properties)}} fn configured()->((bool,bool),Options) {{{call}}} fn run()->bool {{let ((a,b),p)=configured();a and not b and p.flag}}"));
        assert_eq!(luna::execution::interpret(&package, "run", 100_000).unwrap(), Value::Bool(true));
    }
}

#[test]
fn staged_collections_retain_dependent_default_binding() {
    let package = compile("comptime fn schema(...properties:T)->T where T {properties} const SAMPLE=schema {flag:true};typealias Options=SAMPLE'type;fn take(comptime ...properties:Options,first:bool,second:bool,comptime .flag:bool=properties.flag)->bool {flag and first and not second} fn packet()->(bool,bool) {(true,false)} fn run()->bool {take {...packet(),flag:true}}");
    assert_eq!(luna::execution::interpret(&package, "run", 100_000).unwrap(), Value::Bool(true));
}

#[test]
fn mixed_calls_preserve_the_bound_tuple_normal_form() {
    let package = execution_support::compile_with_source_bindings(
        "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T} pure comptime fn Tuple<...Es:List<Type>>->Type {bool} fn packet()->(bool,bool) {(true,false)} fn take(...items:(bool,bool),...properties:T)->((bool,bool),T) where T {(items,properties)} fn run()->bool {let ((a,b),p)=take {...packet(),flag:true};a and not b and p.flag}",
        &Default::default(), luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"), tuple: Some("input::Tuple"), ..Default::default()
        },
    ).unwrap();
    let tuple = id(&package, "Tuple");
    let packet = package.functions.iter().find(|f| f.name == "packet").unwrap();
    let nir::Term::Function { result, .. } = &packet.signature else { panic!() };
    assert!(matches!(result.as_ref(), nir::Term::NormalForm { head, arguments, .. }
        if *head == tuple && matches!(arguments.as_slice(), [nir::Term::TypeListValue(fields)] if fields.len() == 2)));
    assert_eq!(luna::execution::interpret(&package, "run", 100_000).unwrap(), Value::Bool(true));
}
