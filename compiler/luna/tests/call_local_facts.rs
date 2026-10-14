//! Opaque calls invalidate reachable storage, not unaddressed scalar locals.
use comptime_interpretor::Value;
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{compile, compile_with_protocols};

#[test]
fn opaque_callbacks_preserve_private_loop_range_facts() {
    let package = compile("fn repeat(callback:fn(bool)->bool,value:bool,count:.u8'builtin)->bool {var remaining=count;var result=value;while remaining > 0 {result=callback(result);remaining=remaining - 1;}result} pure fn run()->bool {repeat(|value| not value,true,2) and repeat(|value| not value,false,3)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn arbitrary_side_effects_do_not_change_unaddressed_scalar_parameters() {
    let package = compile("fn next(callback:fn(*bool),p:*bool,value:.u8'builtin)->.u8'builtin where requires value < 255 {callback(p);value + 1} fn set(p:*bool) {p.* = true;} fn run()->bool {var flag=false;let result=next(set,flag.ref,40);flag and result == 41}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn addressed_scalar_ranges_do_not_survive_unknown_writes() {
    let source = "fn next(callback:fn(*.u8'builtin),value:.u8'builtin)->.u8'builtin where requires value < 255 {callback(value.ref);value + 1}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("range is not proven"), "{error}");
}

#[test]
fn external_pointee_observations_do_not_survive_unknown_calls() {
    let source = "fn next(callback:fn(*.u8'builtin),p:*.u8'builtin)->.u8'builtin {if p.* < 255 {callback(p);p.* + 1} else {0}}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("range is not proven"), "{error}");
}

#[test]
fn pointer_rebinding_cannot_reuse_a_previous_pointee_range() {
    let source = "typealias Byte=.u8'builtin;fn next(callback:fn(**Byte),p:*Byte)->Byte {if p.* < 255 {callback(p.ref);p.* + 1} else {0}}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("range is not proven"), "{error}");
}

#[test]
fn optimizer_retains_checks_on_callback_reachable_storage() {
    use comptime_interpretor::{Budget, ErrorKind, Interpreter};
    let baseline = compile("typealias Byte=.u8'builtin;fn check(callback:fn(*Byte)) {var value:Byte=0;callback(value.ref);assumes value == 0;} fn change(p:*Byte) {p.* = 1;} fn run() {check(change)}");
    let mut optimized = baseline.clone();
    elaboration::verification::optimization::elide_proven_checks(&mut optimized, &mut Default::default());
    for package in [&baseline, &optimized] {
        let mut vm = Interpreter::new(package, Budget::default()).unwrap();
        let error = vm.invoke(execution_support::id(package,"run"),vec![]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::ProgramContractRejected);
    }
}
