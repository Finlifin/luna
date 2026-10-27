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

#[test]
fn named_clause_values_keep_source_order_before_ordinary_arguments() {
    assert_eq!(invoke("
        comptime fn mark(state:*.Integer'builtin,digit:.Integer'builtin)->bool {
            state.* = state.* * 10 + digit; true
        }
        comptime fn choose(value:bool)->bool where .first:bool=false,.second:bool=false {
            first and second and value
        }
        comptime fn run()->bool {
            var trace:.Integer'builtin=0;
            let result=choose<.second=mark(trace.ref,1),.first=mark(trace.ref,2)>(mark(trace.ref,3));
            result and trace==123
        }
    "),Value::Bool(true));
}

#[test]
fn missing_clause_defaults_reuse_prior_snapshots_in_declaration_scope() {
    assert_eq!(invoke("
        comptime fn choose()->bool where seed:bool,.inverse:bool=not seed,.answer:bool=not inverse {answer}
        comptime fn run()->bool {
            var seed=true;
            let inverse=true;
            let first=choose<seed>;
            seed=false;
            let second=choose<seed>;
            first() and not second() and choose<seed,.answer=true>()
        }
    "),Value::Bool(true));
}

#[test]
fn explicit_override_does_not_evaluate_omitted_clause_default() {
    assert_eq!(
        invoke(
            "
        comptime fn missing()->bool {while true {} false}
        comptime fn choose()->bool where .flag:bool=missing() {flag}
        comptime fn run()->bool {let value=true;choose<.flag=value>()}
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn clause_spread_producer_executes_once_and_defaults_see_its_fields() {
    assert_eq!(invoke("
        comptime fn choose()->bool where first:bool,second:bool,.answer:bool=first and not second {answer}
        comptime fn run()->bool {
            var trace:.Integer'builtin=0;
            let result=choose<...do {trace = trace + 1;(true,false)}>();
            result and trace==1
        }
    "),Value::Bool(true));
}

#[test]
fn invalid_named_clause_routing_is_rejected() {
    for inputs in [
        ".flag=true,.flag=false",
        ".missing=true",
        "true",
        ".flag='x'",
    ] {
        let source = format!(
            "comptime fn choose()->bool where .flag:bool=false {{flag}} comptime fn run()->bool {{choose<{inputs}>()}}"
        );
        assert!(compile(&source).is_err(), "{inputs}");
    }
}

#[test]
fn default_type_clause_specializes_a_local_value_and_callable_result() {
    assert_eq!(
        invoke(
            "
        comptime fn choose()->T where .T:.Type'builtin=bool,value:T {value}
        comptime fn run()->bool {let flag=false;not choose<flag>()}
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn returning_named_input_skips_later_inputs_defaults_and_call() {
    assert_eq!(
        invoke(
            "
        comptime fn missing()->bool {while true {} false}
        comptime fn choose(value:bool)->bool where .first:bool=missing(),.second:bool=false {value}
        comptime fn run()->bool {
            choose<.second=do {return true;}>(missing());false
        }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn later_named_type_inputs_still_check_unused_function_bodies() {
    for inputs in ["true,.T=.char'builtin", ".T=.char'builtin,true"] {
        let source = format!(
            "comptime fn choose()->T where .T:.Type'builtin=bool,value:T {{value}} comptime fn unused()->bool {{choose<{inputs}>();true}}"
        );
        let error = compile(&source).unwrap_err();
        assert!(error.contains("type mismatch"), "{inputs}: {error}");
    }
    assert_eq!(
        invoke(
            "comptime fn choose()->T where .T:.Type'builtin=bool,value:T {value} comptime fn run()->bool {let flag=true;choose<flag,.T=bool>()}"
        ),
        Value::Bool(true)
    );
}

#[test]
fn nested_functions_freeze_lexical_and_local_clause_inputs_together() {
    assert_eq!(
        invoke(
            "
        comptime fn outer()->bool where inherited:bool {
            comptime fn inner()->bool where local:bool {inherited and local}
            var value=true;
            let first=inner<value>;
            value=false;
            let second=inner<value>;
            first() and not second()
        }
        comptime fn run()->bool {outer<true>() and not outer<false>()}
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn nested_defaults_reuse_lexical_types_and_values() {
    assert_eq!(
        invoke(
            "
        comptime fn outer()->T where T,seed:T {
            comptime fn inner()->T where .value:T=seed {value}
            inner<>()
        }
        comptime fn run()->bool {outer<bool,true>() and not outer<bool,false>()}
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn nested_invocations_reuse_the_current_lexical_instance() {
    assert_eq!(
        invoke(
            "
        comptime fn outer()->bool where inherited:bool {
            comptime fn inner()->bool where value:bool,.last:bool=inherited {
                if value {let next=false;inner<next>()} else {last}
            }
            let value=true;inner<value>()
        }
        comptime fn run()->bool {outer<true>() and not outer<false>()}
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn inherited_clause_slots_cannot_be_supplied_by_the_caller() {
    for inputs in ["true,true", ".inherited=false,true"] {
        let source = format!(
            "comptime fn outer()->bool where inherited:bool {{comptime fn inner()->bool where value:bool {{inherited and value}} inner<{inputs}>()}} comptime fn run()->bool {{outer<true>()}}"
        );
        let error = compile(&source).unwrap_err();
        assert!(error.contains("argument"), "{inputs}: {error}");
    }
}
