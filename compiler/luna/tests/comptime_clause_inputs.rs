//! Clause expressions execute in the caller's comptime invocation environment.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Event, Interpreter, Value};

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{{}}} {source}",
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
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
            meta.Object.value<bool>(metadata,0) match {value?=>value,null=>false}
        }
        comptime fn first()->bool where values:meta.List<bool> {values[0]}
        comptime fn run()->bool {
            let original={flag:true};
            let changed=meta.Object.replace(original,0,false);
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

#[test]
fn ordinary_defaults_use_the_selected_comptime_clause_snapshot() {
    assert_eq!(
        invoke(
            "
        comptime fn choose(.value:bool=flag)->bool where flag:bool {value}
        comptime fn run()->bool {
            var flag=true;
            let first=choose<flag>();
            flag=false;
            first and not choose<flag>() and choose<flag>(.value=true)
        }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_defaults_specialize_type_and_value_clauses() {
    assert_eq!(
        invoke(
            "
        comptime fn choose(.value:T=seed)->T where T,seed:T {value}
        comptime fn run()->bool {let flag=true;choose<bool,flag>()}
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_defaults_see_prepared_arguments_and_inherited_clauses() {
    assert_eq!(invoke("
        comptime fn outer()->bool where inherited:bool {
            comptime fn inner(value:bool,.result:bool=value and flag and inherited)->bool where flag:bool {result}
            let local=true;inner<local>(true)
        }
        comptime fn run()->bool {outer<true>() and not outer<false>()}
    "),Value::Bool(true));
}

#[test]
fn explicit_ordinary_input_runs_after_clause_snapshot() {
    assert_eq!(invoke("
        comptime fn change(flag:*bool)->bool {flag.* = false;true}
        comptime fn choose(input:bool,.value:bool=flag and input)->bool where flag:bool {value}
        comptime fn run()->bool {var flag=true;let result=choose<flag>(change(flag.ref));result and not flag}
    "),Value::Bool(true));
}

#[test]
fn ordinary_type_requirements_use_dynamic_instances_clause_types() {
    assert_eq!(invoke("
        comptime fn choose(comptime Actual:.Type'builtin)->bool where Expected,flag:bool,requires Actual==Expected {flag}
        comptime fn run()->bool {let flag=true;choose<bool,flag>(bool)}
    "),Value::Bool(true));
    let error=compile("comptime fn choose(comptime Actual:.Type'builtin)->bool where Expected,flag:bool,requires Actual==Expected {flag} comptime fn unused()->bool {let flag=true;choose<bool,flag>(.char'builtin)}").unwrap_err();
    assert!(!error.contains("arity"), "{error}");
}

#[test]
fn ordinary_capabilities_use_dynamic_clause_types() {
    let declarations = "trait Accept where Expected {} impl Accept<bool> for bool {} comptime fn choose(comptime Actual:.Type'builtin)->bool where Expected,flag:bool,requires Actual:-Accept<Expected> {flag}";
    assert_eq!(
        invoke(&format!(
            "{declarations} comptime fn run()->bool {{let flag=true;choose<bool,flag>(bool)}}"
        )),
        Value::Bool(true)
    );
    let error=compile(&format!("{declarations} comptime fn unused()->bool {{let flag=true;choose<bool,flag>(.char'builtin)}}")).unwrap_err();
    assert!(!error.contains("expected 2 arguments, got 0"), "{error}");
}

#[test]
fn inherited_types_are_retained_for_ordinary_type_requirements() {
    assert_eq!(invoke("
        comptime fn outer()->bool where Expected {
            comptime fn choose(comptime Actual:.Type'builtin,.value:bool=flag)->bool where flag:bool,requires Actual==Expected {value}
            let flag=true;choose<flag>(Expected)
        }
        comptime fn run()->bool {outer<bool>()}
    "),Value::Bool(true));
}

#[test]
fn fixed_clause_requirements_reject_unused_invalid_instances() {
    let declarations = "trait Mark {} impl Mark for bool {} comptime fn choose()->bool where T:-Mark,flag:bool {flag}";
    assert_eq!(
        invoke(&format!(
            "{declarations} comptime fn run()->bool {{let flag=true;choose<bool,flag>()}}"
        )),
        Value::Bool(true)
    );
    let invalid = format!(
        "{declarations} comptime fn unused()->bool {{let flag=true;choose<.char'builtin,flag>()}}"
    );
    assert!(
        compile(&invalid).is_err(),
        "invalid fixed Type clause accepted in unused function"
    );
}

#[test]
fn fixed_type_equalities_and_defaults_are_checked_without_invocation() {
    for clauses in [
        "T,flag:bool,requires T==bool",
        ".T:.Type'builtin=.char'builtin,flag:bool,requires T==bool",
    ] {
        let inputs = if clauses.starts_with('.') {
            "flag"
        } else {
            ".char'builtin,flag"
        };
        let source = format!(
            "comptime fn choose()->bool where {clauses} {{flag}} comptime fn unused()->bool {{let flag=true;choose<{inputs}>()}}"
        );
        assert!(compile(&source).is_err(), "{source}");
    }
}

#[test]
fn value_indexed_capabilities_are_retained_until_invocation() {
    let declarations = "trait Enabled where flag:bool {} impl Enabled<true> for bool {} comptime fn choose()->bool where flag:bool,requires bool:-Enabled<flag> {flag}";
    let accepted =
        format!("{declarations} comptime fn run()->bool {{let flag=true;choose<flag>()}}");
    assert_eq!(invoke(&accepted), Value::Bool(true));
    let rejected =
        format!("{declarations} comptime fn run()->bool {{let flag=false;choose<flag>()}}");
    let package = compile(&rejected).unwrap();
    assert!(luna::invocation::invoke(&package, "run", 100_000).is_err());
}

#[test]
fn inherent_static_methods_accept_local_clause_values() {
    assert_eq!(
        invoke(
            "
        struct Factory {}
        impl Factory {comptime fn choose()->bool where flag:bool {flag}}
        comptime fn run()->bool {let flag=true;Factory.choose<flag>()}
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn generic_inherent_methods_retain_owner_types_and_defaults() {
    assert_eq!(
        invoke(
            "
        struct Factory where T {}
        impl Factory<T> where T {
            comptime fn choose(.value:T=seed)->T where seed:T {value}
        }
        comptime fn run()->bool {let seed=true;Factory<bool>.choose<seed>()}
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn stored_inherent_method_instances_keep_their_snapshot() {
    assert_eq!(
        invoke(
            "
        struct Factory {}
        impl Factory {comptime fn choose()->bool where flag:bool {flag}}
        comptime fn run()->bool {
            var flag=true;let selected=Factory.choose<flag>;flag=false;selected()
        }
    "
        ),
        Value::Bool(true)
    );
}

#[test]
fn receiver_runs_before_clause_and_ordinary_argument_producers() {
    assert_eq!(invoke("
        struct Factory {flag:bool}
        impl Factory {comptime fn choose(self,input:bool,.extra:bool=flag)->bool where flag:bool {self.flag and input and extra}}
        comptime fn mark(trace:*.Integer'builtin,digit:.Integer'builtin)->bool {trace.* = trace.* * 10 + digit;true}
        comptime fn make(trace:*.Integer'builtin)->Factory {Factory {flag:mark(trace,1)}}
        comptime fn run()->bool {
            var trace:.Integer'builtin=0;
            let result=make(trace.ref).choose<mark(trace.ref,2)>(mark(trace.ref,3));
            result and trace==123
        }
    "),Value::Bool(true));
}

#[test]
fn receiver_spread_and_defaults_keep_instance_identity_and_order() {
    assert_eq!(invoke("
        struct Factory {flag:bool}
        impl Factory {comptime fn choose(self,input:bool,.extra:bool=flag)->bool where flag:bool {self.flag and input and extra}}
        comptime fn mark(trace:*.Integer'builtin,digit:.Integer'builtin)->bool {trace.* = trace.* * 10 + digit;true}
        comptime fn make(trace:*.Integer'builtin)->Factory {Factory {flag:mark(trace,1)}}
        comptime fn run()->bool {
            var trace:.Integer'builtin=0;
            let result=make(trace.ref).choose<mark(trace.ref,2)>(...do {(mark(trace.ref,3),)});
            result and trace==123
        }
    "),Value::Bool(true));
}

#[test]
fn static_method_fixed_contracts_reject_invalid_unused_calls() {
    let source = "struct Factory {} impl Factory {comptime fn choose()->bool where T,flag:bool,requires T==bool {flag}} comptime fn unused()->bool {let flag=true;Factory.choose<.char'builtin,flag>()}";
    assert!(compile(source).is_err());
}

#[test]
fn closed_comptime_signature_delays_inline_selection_until_staged_input_supply() {
    assert_eq!(
        invoke(
            "comptime fn choose(comptime flag:bool)->bool {inline if flag {true} else {false}}
             comptime fn run()->bool {choose(true) and not choose(false)}",
        ),
        Value::Bool(true)
    );
}

#[test]
fn closed_comptime_staged_body_checks_unselected_operations() {
    let error = compile(
        "fn runtime_value()->bool {true}
         comptime fn choose(comptime flag:bool)->bool {inline if flag {true} else {runtime_value()}}
         const value=choose(true);",
    )
    .unwrap_err();
    assert!(
        error.contains("unavailable in this execution phase"),
        "{error}"
    );
}

#[test]
fn ordinary_comptime_inputs_select_inline_branches_per_invocation() {
    assert_eq!(
        invoke(
            "comptime fn choose(flag:bool)->bool {inline if flag {true} else {false}}
         comptime fn run()->bool {choose(true) and not choose(false)}"
        ),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_comptime_inline_calls_snapshot_mutable_arguments_once() {
    assert_eq!(
        invoke(
            "comptime fn choose(flag:bool)->bool {inline if flag {true} else {false}}
         comptime fn forward(flag:bool)->bool {choose(flag)}
         comptime fn mark(state:*.Integer'builtin,digit:.Integer'builtin,value:bool)->bool {
             state.* = state.* * 10 + digit; value
         }
         comptime fn run()->bool {
             var trace:.Integer'builtin=0;
             var flag=true;
             let first=forward(mark(trace.ref,1,flag));
             flag=false;
             let second=forward(mark(trace.ref,2,flag));
             first and not second and trace==12
         }"
        ),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_comptime_inline_when_and_match_use_invocation_values() {
    for body in [
        "inline when {flag=>true,else=>false}",
        "inline if flag is do {true=>true,false=>false}",
        "var state=flag; state=not state; inline if state {false} else {true}",
    ] {
        assert_eq!(
            invoke(&format!(
                "comptime fn choose(flag:bool)->bool {{{body}}}
             comptime fn run()->bool {{choose(true) and not choose(false)}}"
            )),
            Value::Bool(true),
            "{body}"
        );
    }
}

#[test]
fn ordinary_comptime_inline_when_short_circuits_selector_effects() {
    assert_eq!(
        invoke(
            "comptime fn mark(state:*.Integer'builtin,digit:.Integer'builtin,value:bool)->bool {
             state.* = state.* * 10 + digit; value
         }
         comptime fn choose(trace:*.Integer'builtin)->bool {
             inline when {
                 mark(trace,1,false)=>false,
                 mark(trace,2,true)=>true,
                 mark(trace,3,true)=>false,
                 else=>false
             }
         }
         comptime fn run()->bool {
             var trace:.Integer'builtin=0;
             let result=choose(trace.ref);
             result and trace==12
         }"
        ),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_comptime_inline_controls_keep_definition_checks() {
    for body in [
        "inline if flag {true} else {runtime_value()}",
        "inline when {flag=>true,else=>runtime_value()}",
        "inline if flag is do {true=>true,false=>runtime_value()}",
        "inline if flag {true} else {()}",
    ] {
        let source = format!(
            "fn runtime_value()->bool {{true}} comptime fn choose(flag:bool)->bool {{{body}}}"
        );
        assert!(
            compile(&source).is_err(),
            "accepted unused invalid body: {body}"
        );
    }
    assert!(compile("fn choose(flag:bool)->bool {inline if flag {true} else {false}}").is_err());
}
