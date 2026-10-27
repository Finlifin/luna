//! Clause expressions execute in the caller's comptime invocation environment.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Event, Interpreter, Value};

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{{}}} {source}",
            include_str!("../../../library/meta/lib.fl")
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}
fn invoke(source: &str) -> Value {
    let package = compile(source).unwrap();
    luna::invocation::invoke(&package, "run", 100_000).unwrap()
}

#[test]
fn mutable_local_clauses_freeze_each_callable_and_reuse_complete_instances() {
    let package = compile(
        "
        comptime fn select()->bool where flag:bool {flag}
        comptime fn run()->bool {
            var flag=true;
            let first=select<flag>;
            flag=false;
            let second=select<flag>;
            let third=select<flag>;
            first() and not second() and not third()
        }
    ",
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let selected = support::id(&package, "select");
    assert_eq!(vm.events().iter().filter(|event|matches!(event,Event::InstantiateFunction{declaration,..} if *declaration==selected)).count(),2);
}

#[test]
fn clauses_execute_once_before_ordinary_arguments_in_source_order() {
    assert_eq!(
        invoke(
            "
        comptime fn mark(state:*.Integer'builtin,digit:.Integer'builtin)->bool {
            state.* = state.* * 10 + digit; true
        }
        comptime fn combine(value:bool)->bool where a:bool,b:bool {a and b and value}
        comptime fn run()->bool {
            var state:.Integer'builtin=0;
            let result=combine<mark(state.ref,1),mark(state.ref,2)>(mark(state.ref,3));
            result and state==123
        }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn clause_control_transfer_skips_following_clauses_and_arguments() {
    assert_eq!(
        invoke(
            "
        comptime fn combine(value:bool)->bool where a:bool,b:bool {a and b and value}
        comptime fn run()->bool {
            combine<do {return true;},do {while true {} false}>(do {while true {} false});
            false
        }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn object_and_list_clauses_retain_domains_and_edited_snapshots() {
    assert_eq!(
        invoke(
            "
        comptime fn enabled()->bool where metadata:meta.Object {
            meta.object_value<bool>(metadata,0) match {value?=>value,null=>false}
        }
        comptime fn first()->bool where values:meta.List<bool> {values[0]}
        comptime fn run()->bool {
            let original={flag:true};
            let changed=meta.object_replace(original,0,false);
            let values:meta.List<bool> = [true,false];
            enabled<original>() and not enabled<changed>() and first<values>()
        }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn fixed_type_clauses_specialize_value_domains_without_unfolding_heads() {
    assert_eq!(
        invoke(
            "
        comptime fn selected()->T where T,value:T {value}
        comptime fn run()->bool {let flag=true;selected<bool,flag>()}
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn runtime_inputs_and_wrong_clause_domains_remain_rejected() {
    for source in [
        "comptime fn select()->bool where flag:bool {flag} fn bad(flag:bool)->bool {select<flag>()}",
        "comptime fn select()->bool where flag:bool {flag} comptime fn run()->bool {let flag='x';select<flag>()}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn recorded_local_clause_reproduction_executes() {
    assert_eq!(
        invoke(include_str!(
            "../../../development/repros/comptime-local-clause-input.fl"
        )),
        Value::Bool(true)
    );
}

#[test]
fn pure_comptime_call_keeps_the_selected_declaration_behavior() {
    assert_eq!(
        invoke(
            "
        pure comptime fn select()->bool where flag:bool {flag}
        pure comptime fn run()->bool {let flag=true;select<flag>()}
    "
        ),
        Value::Bool(true)
    );
}
