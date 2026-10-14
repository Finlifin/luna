//! Default suppliers remain runtime computations with verified finite behavior.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{calls, compile, compile_with_protocols, id};

fn run(source: &str, expected: Value, order: &[&str]) {
    let package = compile(source);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(&package, "run"), vec![]).unwrap(), expected);
    assert_eq!(calls(&package, &vm), order);
    assert_eq!(luna::execution::interpret(&package, "run", 100_000).unwrap(), expected);
}

#[test]
fn finite_call_chains_supply_defaults_at_runtime_in_declaration_order() {
    run("fn choose(first:bool,.second:bool=opposite(first),.third:bool=opposite(second))->(bool,bool,bool) {(first,second,third)} fn opposite(value:bool)->bool {identity(not value)} fn identity(value:bool)->bool {value} fn input()->bool {true} fn run()->(bool,bool,bool) {choose(input())}", Value::Tuple(vec![Value::Bool(true),Value::Bool(false),Value::Bool(true)]), &["run","input","opposite","identity","opposite","identity","choose"]);
}

#[test]
fn explicit_supply_skips_only_its_default_and_keeps_later_dependencies() {
    run("fn choose(first:bool,.second:bool=opposite(first),.third:bool=opposite(second))->(bool,bool) {(second,third)} fn opposite(value:bool)->bool {not value} fn input()->bool {true} fn override_value()->bool {true} fn run()->(bool,bool) {choose(input(),.second=override_value())}", Value::Tuple(vec![Value::Bool(true),Value::Bool(false)]), &["run","input","override_value","opposite","choose"]);
}

#[test]
fn default_helpers_can_use_private_mutation_and_early_returns() {
    run("fn choose(input:bool,.flag:bool=bounded(input))->bool {flag} fn bounded(value:bool)->bool {var result=false;if value {result=true;}else{return false;}result} fn run()->bool {choose(true) and not choose(false)}", Value::Bool(true), &["run","bounded","choose","bounded","choose"]);
}

#[test]
fn generic_helpers_are_checked_after_type_substitution() {
    run("fn identity(value:T)->T where T {value} fn choose(value:bool,.other:bool=identity(value))->bool {other} fn run()->bool {choose(true)}", Value::Bool(true), &["run","identity","choose"]);
}

#[test]
fn nested_call_arguments_and_private_default_locals_are_stable() {
    run("fn invert(value:bool)->bool {not value} fn choose(.flag:bool=do {var local=true;local=invert(local);invert(local)})->bool {flag} fn run()->bool {choose()}", Value::Bool(true), &["run","invert","invert","choose"]);
}

#[test]
fn recursive_effectful_and_unknown_suppliers_need_stronger_evidence() {
    for source in [
        "fn cycle()->bool {cycle()} fn take(.value:bool=cycle())->bool {value}",
        "fn first()->bool {second()} fn second()->bool {first()} fn take(.value:bool=first())->bool {value}",
        "fn spin()->bool {while true {} true} fn take(.value:bool=spin())->bool {value}",
        "fn write(p:*bool)->bool {p.* = true;true} fn take(p:*bool,.value:bool=write(p))->bool {value}",
        "fn read(p:*bool)->bool {p.*} fn take(p:*bool,.value:bool=read(p))->bool {value}",
        "fn take(action:fn()->bool,.value:bool=action())->bool {value}",
        "struct Owned {} fn helper()->bool {let owned=Owned {};true} fn take(.value:bool=helper())->bool {value}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(error.contains("P3 stability/termination"), "{source}: {error}");
    }
}

#[test]
fn helper_admission_does_not_hide_call_contracts_or_invalid_arithmetic() {
    let error = compile_with_protocols("fn bounded(value:.u8'builtin)->.u8'builtin {value + 1} fn take(value:.u8'builtin,.next:.u8'builtin=bounded(value))->.u8'builtin {next}", &Default::default()).unwrap_err();
    assert!(!error.contains("not yet implemented"), "{error}");
    assert!(error.contains("range is not proven") || error.contains("precondition"), "{error}");
}

#[test]
fn finite_arithmetic_helpers_keep_entry_and_result_contracts() {
    run("typealias Byte=.u8'builtin;fn next(value:Byte)->result:Byte where requires value < 255, ensures result == value'old + 1 {value + 1} fn take(value:Byte,.following:Byte=if value < 255 {next(value)} else {value})->Byte where requires value < 255 {following} fn run()->Byte {take(40)}", Value::UnsignedInteger(41), &["run","next","take"]);
    let invalid = "typealias Byte=.u8'builtin;fn next(value:Byte)->Byte where requires value < 255 {value + 1} fn take(.following:Byte=next(255))->Byte {following} fn run()->Byte {take()}";
    let error = compile_with_protocols(invalid, &Default::default()).unwrap_err();
    assert!(error.contains("precondition"), "{error}");
}

#[test]
fn defaults_cannot_write_prior_inputs_or_hide_invalid_unused_suppliers() {
    for source in [
        "fn take(value:bool,.other:bool=do {value=false;value})->bool {other}",
        "fn cycle()->bool {cycle()} fn take(.value:bool=cycle())->bool {value} fn run()->bool {take(.value=true)}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(error.contains("P3 stability/termination"), "{error}");
    }
}

#[test]
fn tuple_supplier_analysis_preserves_the_bound_normal_form() {
    let source = "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T} pure comptime fn Tuple<...Es:List<Type>>->Type {bool} fn pair(value:bool)->(bool,bool) {(value,not value)} fn first(pair:(bool,bool))->bool {let (first,second)=pair;first} fn take(value:bool,.pair:(bool,bool)=pair(value),.flag:bool=first(pair))->bool {flag} fn run()->bool {take(true)}";
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::List"), tuple: Some("input::Tuple"), ..Default::default()
    }).unwrap();
    let pair = package.functions.iter().find(|f| f.name == "pair").unwrap();
    let nir::Term::Function { result, .. } = &pair.signature else { panic!() };
    assert!(matches!(result.as_ref(), nir::Term::NormalForm { head, .. } if *head == id(&package, "Tuple")));
    assert_eq!(luna::execution::interpret(&package, "run", 100_000).unwrap(), Value::Bool(true));
}
