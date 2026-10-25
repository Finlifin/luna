//! Staged parameters in first-class callable type descriptions.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use nir::{Primitive as P, Sort, Term};

#[test]
fn comptime_callable_annotations_preserve_parameter_stages() {
    let package = support::compile(
        "typealias Callback=comptime fn(comptime bool,bool)->bool;
        comptime fn choose(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        comptime fn apply(callback:Callback)->bool {callback(true,true)}
        comptime fn run()->bool {apply(choose)}",
    );
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(support::id(&package, "run"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn computed_callable_domains_determine_staged_input_sorts() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        comptime fn Domain()->Type {bool}
        comptime fn Meta()->Type {Type}
        comptime fn Build()->Type {comptime fn(comptime Domain(),comptime Meta(),bool)->bool}",
    );
    let value = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .invoke(support::id(&package, "Build"), vec![])
        .unwrap();
    assert_eq!(
        value,
        Value::Type(Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![
                nir::Parameter {
                    sort: Sort::ComptimeValue,
                    ty: Term::Primitive(P::Bool)
                },
                nir::Parameter {
                    sort: Sort::Type,
                    ty: Term::Primitive(P::Type)
                },
                nir::Parameter {
                    sort: Sort::Runtime,
                    ty: Term::Primitive(P::Bool)
                }
            ],
            result: Box::new(Term::Primitive(P::Bool)),
        })))
    );
}

#[test]
fn callback_parameter_stage_mismatch_is_rejected() {
    for (expected, actual) in [("comptime ", ""), ("", "comptime ")] {
        let source = format!(
            "typealias Callback=comptime fn({expected}bool)->bool;
            comptime fn choose({actual}flag:bool)->bool {{flag}}
            comptime fn apply(callback:Callback)->bool {{callback(true)}}
            comptime fn run()->bool {{apply(choose)}}"
        );
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("type") || error.contains("Type"), "{error}");
    }
}

#[test]
fn runtime_callable_type_construction_preserves_staged_inputs() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        comptime fn Domain()->Type {bool}
        comptime fn Build()->Type {fn(comptime Domain(),bool)->bool}",
    );
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(support::id(&package, "Build"), vec![])
            .unwrap(),
        Value::Type(Term::Function {
            parameters: vec![
                nir::Parameter {
                    sort: Sort::ComptimeValue,
                    ty: Term::Primitive(P::Bool)
                },
                nir::Parameter {
                    sort: Sort::Runtime,
                    ty: Term::Primitive(P::Bool)
                },
            ],
            result: Box::new(Term::Primitive(P::Bool)),
        })
    );
}

#[test]
fn runtime_callbacks_with_staged_inputs_execute_in_iir() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn choose(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn apply(callback:Callback,value:bool)->bool {callback(false,value)}
        fn run()->bool {apply(choose,false)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn runtime_callback_cannot_supply_runtime_input_to_staged_parameter() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn apply(callback:Callback,flag:bool)->bool {callback(flag,true)}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("comptime") || error.contains("stage"),
        "{error}"
    );
}

#[test]
fn staged_callback_repeated_calls_and_nested_forwarders_have_distinct_instances() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn choose(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn apply(callback:Callback,value:bool)->bool {
            let first=callback(true,value);
            let second=callback(false,value);
            first and not second
        }
        fn relay(callback:Callback,value:bool)->bool {apply(callback,value)}
        fn run()->bool {relay(choose,true)}";
    let package = support::compile(source);
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let instances: Vec<_> = graph
        .functions
        .iter()
        .filter(|instance| instance.function.name == "choose")
        .collect();
    assert_eq!(instances.len(), 2);
    assert_ne!(instances[0].key, instances[1].key);
    assert_eq!(
        Interpreter::from_iir(&graph, Budget::default())
            .unwrap()
            .invoke_instance(&graph.roots[0], vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn staged_callback_expansion_evaluates_runtime_arguments_once_in_source_order() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn choose(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn flip(state:*bool)->bool {let old=state.*;state.* = not old;old}
        fn apply(callback:Callback,first:bool,second:bool)->bool {
            callback(true,first) and callback(false,second)
        }
        fn run()->bool {
            var state=true;
            let result=apply(choose,flip(state.ref),flip(state.ref));
            result and state
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn runtime_selected_staged_callbacks_keep_selection_before_argument_effects() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn flip(state:*bool)->bool {let old=state.*;state.* = not old;old}
        fn apply(callback:Callback,value:bool)->bool {
            let first=callback(true,value);
            let second=callback(false,value);
            first and not second
        }
        fn run(start:bool)->bool {
            var state=start;
            let callback=if flip(state.ref) {keep} else {invert};
            let result=apply(callback,flip(state.ref));
            not result and state == start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn matched_staged_callback_selection_preserves_guards_and_aliases() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn flip(state:*bool)->bool {let old=state.*;state.* = not old;old}
        fn apply(callback:Callback)->bool {
            let a=callback(true,true);
            let b=callback(false,true);
            a and not b
        }
        fn run(start:bool)->bool {
            var state=start;
            let callback=true match {
                true if flip(state.ref)=>{keep},
                _=>{invert},
            };
            let alias=callback;
            let result=apply(alias);
            result == start and state == not start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn staged_callback_selection_keeps_early_return_before_arguments() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn flip(state:*bool)->bool {let old=state.*;state.* = not old;old}
        fn apply(callback:Callback,value:bool)->bool {callback(true,value)}
        fn run(stop:bool)->bool {
            var state=true;
            let callback=if stop {return false} else {keep};
            apply(callback,flip(state.ref)) and not state
        }";
    for stop in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(stop)]),
            Value::Bool(!stop)
        );
    }
}

#[test]
fn staged_callback_specialization_preserves_target_and_wrapper_contracts() {
    let prefix = "typealias Callback=fn(comptime bool,bool)->bool;
        fn checked(comptime flag:bool,value:bool)->result:bool
            where ensures result == value'old {value}
        fn apply(callback:Callback,value:bool)->result:bool
            where requires value, ensures result == value'old {
                callback(true,value);value
            }";
    let valid = format!("{prefix} fn run()->bool {{apply(checked,true)}}");
    assert_eq!(support::run_iir(&valid, "run", vec![]), Value::Bool(true));
    for source in [
        format!("{prefix} fn run()->bool {{apply(checked,false)}}"),
        "typealias Callback=fn(comptime bool,bool)->bool;
        fn checked(comptime flag:bool,value:bool)->bool where requires value == flag {value}
        fn apply(callback:Callback)->bool {callback(true,false)}
        fn run()->bool {apply(checked)}"
            .to_owned(),
    ] {
        let failure = match support::compile_with_protocols(&source, &Default::default()) {
            Err(error) => error,
            Ok(package) => luna::execution::interpret(&package, "run", 100_000).unwrap_err(),
        };
        assert!(
            failure.contains("precondition")
                || failure.contains("contract")
                || failure.contains("ProgramContractRejected"),
            "{failure}"
        );
    }
}

#[test]
fn conditional_template_targets_can_reference_immutable_local_aliases() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn run(start:bool)->bool {
            let named=keep;
            let alias=named;
            let callback=if start {alias} else {invert};
            let first=callback(true,true);
            let second=callback(false,true);
            first == start and second == not start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn nested_template_selections_keep_each_original_selection_time() {
    let source = "fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn flip(state:*bool)->bool {let old=state.*;state.* = not old;old}
        fn run(start:bool)->bool {
            var state=start;
            let first=if flip(state.ref) {keep} else {invert};
            let selected=if flip(state.ref) {first} else {keep};
            let a=selected(true,true);
            let b=selected(false,true);
            a == start and b == not start and state == start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn staged_trait_method_callbacks_keep_selection_and_repeated_instances() {
    let source = "trait Select {fn select(comptime flag:bool,value:bool)->bool;}
        struct Host {}
        impl Select for Host {
            fn select(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        }
        fn apply(callback:fn(comptime bool,bool)->bool)->bool {
            let a=callback(true,true);
            let b=callback(false,true);
            a and not b
        }
        fn run()->bool {
            let callback=Host.as(Select).select;
            apply(callback)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn value_indexed_trait_callback_selection_preserves_interface_identity() {
    let source = "trait Select where enabled:bool {
            fn select(comptime flag:bool,value:bool)->bool {
                if flag {value == enabled} else {value != enabled}
            }
        }
        struct Host {}
        impl Select<true> for Host {}
        impl Select<false> for Host {}
        fn apply(callback:fn(comptime bool,bool)->bool)->bool {
            let a=callback(true,true);
            let b=callback(false,true);
            a and not b
        }
        fn run(start:bool)->bool {
            let yes=Host.as(Select<true>).select;
            let no=Host.as(Select<false>).select;
            let callback=if start {yes} else {no};
            apply(callback) == start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn staged_method_callback_keeps_runtime_receiver_before_staged_input() {
    let source = "trait Select {fn select(self,comptime flag:bool,value:bool)->bool;}
        struct Host {enabled:bool}
        impl Select for Host {
            fn select(self,comptime flag:bool,value:bool)->bool {
                if flag {value == self.enabled} else {value != self.enabled}
            }
        }
        fn apply(callback:fn(Host,comptime bool,bool)->bool,host:Host)->bool {
            callback(host,true,true)
        }
        fn run(start:bool)->bool {
            let callback=Host.as(Select).select;
            apply(callback,Host {enabled:start}) == start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn staged_trait_callback_does_not_erase_interface_preconditions() {
    let source = "trait Select {
            fn select(comptime flag:bool,value:bool)->bool where requires value;
        }
        struct Host {}
        impl Select for Host {
            fn select(comptime flag:bool,value:bool)->bool {value}
        }
        fn run()->bool {
            let callback=Host.as(Select).select;
            callback(true,false)
        }";
    let package = support::compile(source);
    let failure = luna::execution::interpret(&package, "run", 100_000).unwrap_err();
    assert!(
        failure.contains("precondition") || failure.contains("ProgramContractRejected"),
        "{failure}"
    );
    let accepted = source.replace("callback(true,false)", "callback(true,true)");
    assert_eq!(
        support::run_iir(&accepted, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn mutable_staged_callbacks_preserve_old_aliases_after_reassignment() {
    let source = "fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn run(start:bool)->bool {
            var callback=if start {keep} else {invert};
            let saved=callback;
            callback = if start {invert} else {keep};
            let old=saved(true,true);
            let current=callback(true,true);
            old == start and current == not start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn mutable_staged_callback_callee_is_selected_before_argument_assignment() {
    let source = "fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn run()->bool {
            var callback=keep;
            let first=callback(true,do {callback=invert;true});
            let second=callback(true,true);
            first and not second
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn mutable_staged_callback_loop_rebinding_evaluates_each_selection_once() {
    let source = "fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn flip(state:*bool)->bool {let old=state.*;state.* = not old;old}
        fn run(start:bool)->bool {
            var state=start;
            var callback=keep;
            let saved=callback;
            var remaining=true;
            while remaining {
                callback=if flip(state.ref) {invert} else {keep};
                remaining=false;
            }
            let actual=callback(true,true);
            actual == not start and saved(true,true) and state == not start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn mutable_staged_callbacks_transfer_current_choices_between_locals() {
    let source = "fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn run(start:bool)->bool {
            var left=if start {keep} else {invert};
            var right=invert;
            let saved=left;
            right=left;
            left=invert;
            left=right;
            right=saved;
            let a=left(true,true);
            let b=right(false,true);
            a == start and b == not start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn mutable_callback_alias_sources_preserve_callee_and_conditional_assignment_order() {
    let source = "fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn run(start:bool)->bool {
            var left=keep;
            var right=invert;
            let original=left;
            let first=left(true,do {left=right;true});
            right=original;
            left=if start {left} else {right};
            left=left;
            let current=left(true,true);
            first and current == not start and right(true,true)
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn returned_staged_callback_keeps_runtime_factory_selection() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(choice:bool)->Callback {if choice {keep} else {invert}}
        fn run(start:bool)->bool {
            let callback=factory(start);
            let a=callback(true,true);
            let b=callback(false,true);
            a == start and b == not start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn staged_callback_factories_preserve_effects_forwarding_and_reassignment() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(state:*bool)->Callback {
            let choice=state.*;
            state.* = not choice;
            if choice {keep} else {invert}
        }
        fn forward(callback:Callback)->Callback {callback}
        fn run(start:bool)->bool {
            var state=start;
            var callback=forward(factory(state.ref));
            let saved=callback;
            callback=factory(state.ref);
            let a=saved(true,true);
            let b=callback(true,true);
            a == start and b == not start and state == start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn staged_callback_factories_preserve_explicit_return() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(choice:bool)->Callback {
            if choice {return keep;}
            invert
        }
        fn run(start:bool)->bool {
            let callback=factory(start);
            callback(true,true) == start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn early_returning_template_factory_keeps_effects_and_wrapper_contracts() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(state:*bool,allowed:bool)->Callback where requires allowed {
            let choice=state.*;
            state.* = not choice;
            if choice {return keep;}
            invert
        }
        fn run(start:bool)->bool {
            var state=start;
            let callback=factory(state.ref,true);
            let result=callback(true,true);
            result == start and state == not start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
    let invalid = source.replace("factory(state.ref,true)", "factory(state.ref,false)");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn staged_factory_inputs_select_returned_template_without_running_runtime_body_early() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(comptime choice:bool,state:*bool)->Callback {
            state.* = not state.*;
            if choice {return keep;}
            invert
        }
        fn run()->bool {
            var state=false;
            let first=factory(true,state.ref);
            let second=factory(false,state.ref);
            first(true,true) and not second(true,true) and not state
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn trait_factory_methods_return_staged_callbacks_through_interface_selection() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        trait Factory where enabled:bool {
            fn make()->Callback {
                if enabled {return keep;}
                invert
            }
        }
        struct Host {}
        impl Factory<true> for Host {}
        impl Factory<false> for Host {}
        fn run()->bool {
            let yes=Host.as(Factory<true>).make();
            let no=Host.as(Factory<false>).make();
            yes(true,true) and not no(true,true)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_trait_factory_forwarding_retains_receiver_and_interface_preconditions() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        trait Factory {fn make(self,allowed:bool)->Callback where requires allowed;}
        struct Host {choice:bool}
        impl Factory for Host {
            fn make(self,allowed:bool)->Callback {
                if self.choice {return keep;}
                invert
            }
        }
        fn relay(host:T,allowed:bool)->Callback
            where T:-Factory,requires allowed {host.as(Factory).make(allowed)}
        fn run(start:bool)->bool {
            let callback=relay(Host {choice:start},true);
            callback(true,true) == start
        }";
    for start in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(start)]),
            Value::Bool(true)
        );
    }
    let invalid = source.replace(
        "relay(Host {choice:start},true)",
        "Host {choice:start}.as(Factory).make(false)",
    );
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn staged_trait_factory_inputs_reach_returned_callback_instances() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        trait Factory {fn make(comptime enabled:bool)->Callback;}
        struct Host {}
        impl Factory for Host {
            fn make(comptime enabled:bool)->Callback {
                if enabled {return keep;}
                invert
            }
        }
        fn run()->bool {
            let first=Host.as(Factory).make(true);
            let second=Host.as(Factory).make(false);
            first(true,true) and not second(true,true)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn factory_early_returns_preserve_mutable_callback_and_saved_alias_choices() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(choice:bool,early:bool)->Callback {
            var callback=if choice {keep} else {invert};
            let saved=callback;
            callback=if choice {invert} else {keep};
            if early {return saved;}
            callback
        }
        fn run(choice:bool,early:bool)->bool {
            let callback=factory(choice,early);
            callback(true,true) == if early {choice} else {not choice}
        }";
    for choice in [false, true] {
        for early in [false, true] {
            assert_eq!(
                support::run_iir(source, "run", vec![Value::Bool(choice), Value::Bool(early)]),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn early_returned_local_template_preserves_factory_resource_cleanup() {
    for early in [false, true] {
        let source = format!(
            "typealias Callback=fn(comptime bool,bool)->bool;
            trait Duplicate {{}}
            trait Retire {{fn drop(*self);}}
            struct Ticket {{target:*bool}}
            impl Retire for Ticket {{
                fn drop(*self) {{let p=self.target.*;p.* = not p.*;}}
            }}
            fn keep(comptime flag:bool,value:bool)->bool {{if flag {{value}} else {{not value}}}}
            fn invert(comptime flag:bool,value:bool)->bool {{if flag {{not value}} else {{value}}}}
            fn factory(state:*bool,early:bool)->Callback {{
                let ticket=Ticket {{target:state}};
                var callback=keep;
                let saved=callback;
                callback=invert;
                if early {{return saved;}}
                callback
            }}
            fn run()->bool {{
                var state=false;
                let callback=factory(state.ref,{early});
                let result=callback(true,true);
                state and result == {early}
            }}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn template_factory_budget_failure_does_not_publish_partial_instances() {
    let package = support::compile(
        "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(choice:bool)->Callback {if choice {return keep;}invert}
        fn run()->bool {
            var callback=factory(true);
            let saved=callback;
            callback=factory(false);
            saved(true,true) and not callback(true,true)
        }",
    );
    let baseline = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for fuel in [0, 1, 16, 128] {
        assert!(luna::instantiation::instantiate(&package, "run", fuel).is_err());
        let recovered = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        assert_eq!(recovered.to_string(), baseline.to_string());
        assert_eq!(
            Interpreter::from_iir(&recovered, Budget::default())
                .unwrap()
                .invoke_instance(&recovered.roots[0], vec![])
                .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn recursive_template_factory_executes_with_finite_helper_graph() {
    let package = support::compile(
        "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn factory(again:bool)->Callback {if again {factory(false)} else {keep}}
        fn unresolved()->bool {let callback=factory(true);callback(true,true)}
        fn run()->bool {keep(true,true)}",
    );
    let recursive = luna::instantiation::instantiate(&package, "unresolved", 100_000).unwrap();
    let helpers: Vec<_> = recursive
        .functions
        .iter()
        .filter(|instance| instance.function.name.ends_with("::template-target"))
        .collect();
    assert_eq!(helpers.len(), 1);
    let mut vm = Interpreter::from_iir(&recursive, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&recursive.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        vm.events()
            .iter()
            .filter(|event| matches!(event,
        comptime_interpretor::Event::EnterFunction(id) if *id == helpers[0].function.declaration))
            .count(),
        2
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert_eq!(
        Interpreter::from_iir(&graph, Budget::default())
            .unwrap()
            .invoke_instance(&graph.roots[0], vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn mutual_template_factories_keep_recursive_effects_and_early_returns() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn first(again:bool,state:*bool)->Callback {
            state.* = not state.*;
            if again {return second(false,state);}
            keep
        }
        fn second(again:bool,state:*bool)->Callback {
            state.* = not state.*;
            if again {first(false,state)} else {invert}
        }
        fn run(again:bool)->bool {
            var state=false;
            let callback=first(again,state.ref);
            let value=callback(true,true);
            value == not again and state == not again
        }";
    for again in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(again)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn recursive_template_factories_keep_distinct_generic_instances() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn factory(again:bool,value:T)->Callback where T {
            if again {factory(false,value)} else {keep}
        }
        fn run()->bool {
            let first=factory(true,true);
            let second=factory(true,'x');
            first(true,true) and second(true,true)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn recursive_factory_nontermination_is_a_runtime_budget_failure_and_recovers() {
    let package = support::compile(
        "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn factory(forever:bool)->Callback {if forever {factory(true)} else {keep}}
        fn run(forever:bool)->bool {let callback=factory(forever);callback(true,true)}",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(
        &graph,
        Budget {
            steps: 100_000,
            depth: 32,
        },
    )
    .unwrap();
    let failure = vm
        .invoke_instance(&graph.roots[0], vec![Value::Bool(true)])
        .unwrap_err();
    assert_eq!(
        failure.kind,
        comptime_interpretor::ErrorKind::DepthExhausted
    );
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![Value::Bool(false)])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn recursive_factory_helpers_preserve_resource_cleanup_per_runtime_frame() {
    for (again, saved) in [(false, false), (true, false), (false, true), (true, true)] {
        let source = format!(
            "typealias Callback=fn(comptime bool,bool)->bool;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Ticket {{target:*bool}}
            impl Retire for Ticket {{fn drop(*self) {{let p=self.target.*;p.* = not p.*;}}}}
            fn keep(comptime flag:bool,value:bool)->bool {{if flag {{value}} else {{not value}}}}
            fn factory(again:bool,state:*bool)->Callback {{
                let ticket=Ticket {{target:state}};
                if again {{return factory(false,state);}}
                keep
            }}
            fn run()->bool {{
                var state=false;
                let callback=factory({again},state.ref);
                callback(true,true) and state == not {again}
            }}"
        );
        let source = if saved {
            source.replace("if again {return factory(false,state);}\n                keep",
                "var selected=if again {factory(false,state)} else {keep}; let saved=selected; selected=keep; selected=saved; return selected; keep")
        } else {
            source
        };
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                200_000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn recursive_factory_defaults_preserve_parameter_dependencies_and_overrides() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(again:bool,choice:bool,.selected:bool=choice)->Callback {
            if again {factory(false,not choice,.selected=selected)}
            else {if selected {keep} else {invert}}
        }
        fn run(choice:bool)->bool {
            let defaulted=factory(true,choice);
            let explicit=factory(true,choice,.selected=not choice);
            defaulted(true,true) == choice and explicit(true,true) == not choice
        }";
    for choice in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(choice)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn recursive_factory_preserves_entry_snapshot_postconditions() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        pure fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn factory(again:bool,.allowed:bool=true)->result:Callback where requires allowed, ensures allowed'old {
            allowed=false;
            if again {factory(false)} else {keep}
        }
        fn run()->bool {let callback=factory(true);callback(true,true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn recursive_factory_can_store_and_use_recursive_results_before_returning() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(again:bool,choice:bool)->Callback {
            let selected=if again {factory(false,choice)} else {if choice {keep} else {invert}};
            let saved=selected;
            if saved(true,true) {return saved;}
            selected
        }
        fn run(choice:bool)->bool {let callback=factory(true,choice);callback(true,true)==choice and callback(false,false)==choice}";
    let package = support::compile(source);
    let graph = luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    let helper = graph
        .functions
        .iter()
        .find(|instance| instance.function.name.ends_with("::template-target"))
        .unwrap()
        .function
        .declaration;
    for choice in [false, true] {
        let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![Value::Bool(choice)])
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(vm.events().iter().filter(|event| matches!(event, comptime_interpretor::Event::EnterFunction(id) if *id == helper)).count(), 2);
    }
}

#[test]
fn unreachable_template_sequences_after_loop_transfers_do_not_materialize() {
    let source = "fn staged_value(comptime flag:bool)->bool {flag}
        fn run()->bool {
            var again=true;
            while again {
                again=false;
                continue;
                let unused=staged_value;
                unused;
            }
            while true {
                break;
                let unused=staged_value;
                unused;
            }
            not again
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn only_definitely_terminated_sequence_suffixes_are_removed() {
    let source = "fn staged_value(comptime flag:bool)->bool {flag}
        fn run(early:bool,state:*bool)->bool {
            if early {return true;}
            state.* = true;
            return false;
            state.* = false;
            let unused=staged_value;
            unused;
            false
        }
        fn check(early:bool)->bool {
            var state=false;
            let answer=run(early,state.ref);
            answer == early and state == not early
        }";
    for early in [false, true] {
        assert_eq!(
            support::run_iir(source, "check", vec![Value::Bool(early)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn recursive_factory_mutable_choices_preserve_saved_aliases_and_reassignment() {
    let source="typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(again:bool,choice:bool,restore:bool)->Callback {
            var selected=if again {factory(false,choice,false)} else {if choice {keep} else {invert}};
            let saved=selected;
            if again {
                selected=if choice {invert} else {keep};
                if restore {selected=saved;}
            }
            selected
        }
        fn run(choice:bool,restore:bool)->bool {
            let callback=factory(true,choice,restore);
            callback(true,true)==if restore {choice} else {not choice}
        }";
    for choice in [false, true] {
        for restore in [false, true] {
            assert_eq!(
                support::run_iir(
                    source,
                    "run",
                    vec![Value::Bool(choice), Value::Bool(restore)]
                ),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn recursive_factory_mutable_callee_is_selected_before_argument_updates() {
    let source="typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(again:bool,choice:bool)->Callback {
            var selected=if again {factory(false,choice)} else {if choice {keep} else {invert}};
            let saved=selected;
            if again {
                let observed=selected(true,do {selected=if choice {invert} else {keep};true});
                if observed==choice {return selected;}
                return saved;
            }
            selected
        }
        fn run(choice:bool)->bool {let callback=factory(true,choice);callback(true,true)==not choice}";
    for choice in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(choice)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn recursive_factory_mutable_loop_transfers_copy_current_tags() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn factory(again:bool,choice:bool)->Callback {
            var first=if again {factory(false,choice)} else {if choice {keep} else {invert}};
            var second=if choice {invert} else {keep};
            var repeat=again;
            while repeat {
                let saved=first;
                first=second;
                second=saved;
                first=first;
                repeat=false;
            }
            if again {second} else {first}
        }
        fn run(choice:bool)->bool {let callback=factory(true,choice);callback(true,true)==choice}";
    for choice in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(choice)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn recursive_factory_does_not_represent_addressed_callback_storage_as_a_tag() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn factory(again:bool)->Callback {
            var selected=if again {factory(false)} else {keep};
            let address=selected.ref;
            selected=keep;
            address.*
        }
        fn run()->bool {let callback=factory(true);callback(true,true)}
        fn independent()->bool {keep(true,true)}";
    let package = support::compile(source);
    assert!(luna::instantiation::instantiate(&package, "run", 100_000).is_err());
    let graph = luna::instantiation::instantiate(&package, "independent", 100_000).unwrap();
    assert_eq!(
        Interpreter::from_iir(&graph, Budget::default())
            .unwrap()
            .invoke_instance(&graph.roots[0], vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn recursive_factory_can_discard_recursive_result_before_returning() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn factory(again:bool,state:*bool)->Callback {
            state.* = not state.*;
            if again {factory(false,state);}
            keep
        }
        fn run()->bool {
            var state=false;
            let callback=factory(true,state.ref);
            callback(true,true) and not state
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn recursive_factory_can_forward_saved_choice_to_higher_order_wrapper() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn invert(comptime flag:bool,value:bool)->bool {if flag {not value} else {value}}
        fn apply(callback:Callback)->bool {callback(true,true)}
        fn factory(again:bool,choice:bool)->Callback {
            let selected=if again {factory(false,choice)} else {if choice {keep} else {invert}};
            if apply(selected) {return keep;}
            invert
        }
        fn run(choice:bool)->bool {let callback=factory(true,choice);callback(true,true)==choice}";
    for choice in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(choice)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn discarded_template_choices_keep_selection_effects_and_early_returns() {
    let source = "fn staged_value(comptime flag:bool)->bool {flag}
        fn choose(early:bool,state:*bool)->bool {
            let saved=staged_value;
            saved;
            (if early {return false;} else {state.* = true;staged_value});
            true
        }
        fn run(early:bool)->bool {
            var state=false;
            let result=choose(early,state.ref);
            result == not early and state == not early
        }";
    for early in [false, true] {
        assert_eq!(
            support::run_iir(source, "run", vec![Value::Bool(early)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn discarded_recursive_factory_results_keep_per_frame_resource_cleanup() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        trait Duplicate {} trait Retire {fn drop(*self);}
        struct Ticket {target:*bool}
        impl Retire for Ticket {fn drop(*self) {let p=self.target.*;p.* = not p.*;}}
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn factory(again:bool,state:*bool)->Callback {
            let ticket=Ticket {target:state};
            if again {factory(false,state);}
            keep
        }
        fn run()->bool {
            var state=false;
            factory(true,state.ref);
            not state
        }";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 200_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn finite_factory_graphs_are_limited_by_work_budget_not_a_fixed_node_count() {
    const COUNT: usize = 70;
    let mut source = String::from(
        "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}\n",
    );
    for index in 0..COUNT {
        source.push_str(&format!("fn factory{index}(again:bool)->Callback {{if again {{factory{}(false)}} else {{keep}}}}\n",(index+1)%COUNT));
    }
    source.push_str("fn run()->bool {let callback=factory0(true);callback(true,true)}");
    let package = support::compile(&source);
    assert!(luna::instantiation::instantiate(&package, "run", 500).is_err());
    let graph = luna::instantiation::instantiate(&package, "run", 2_000_000).unwrap();
    assert_eq!(
        graph
            .functions
            .iter()
            .filter(|instance| instance.function.name.ends_with("::template-target"))
            .count(),
        COUNT
    );
    let mut vm = Interpreter::from_iir(
        &graph,
        Budget {
            steps: 2_000_000,
            depth: 128,
        },
    )
    .unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        graph.to_string(),
        luna::instantiation::instantiate(&package, "run", 2_000_000)
            .unwrap()
            .to_string()
    );
}

#[test]
fn shared_factory_dependencies_do_not_create_false_recursion_cycles() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn leaf()->Callback {keep}
        fn left()->Callback {leaf()}
        fn right()->Callback {leaf()}
        fn factory(choice:bool)->Callback {if choice {left()} else {right()}}
        fn run(choice:bool)->bool {let callback=factory(choice);callback(true,true)}";
    let package = support::compile(source);
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert!(
        !graph
            .functions
            .iter()
            .any(|instance| instance.function.name.ends_with("::template-target"))
    );
    for choice in [false, true] {
        assert_eq!(
            Interpreter::from_iir(&graph, Budget::default())
                .unwrap()
                .invoke_instance(&graph.roots[0], vec![Value::Bool(choice)])
                .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn growing_recursive_factory_instances_exhaust_budget_without_poisoning_other_roots() {
    let source = "typealias Callback=fn(comptime bool,bool)->bool;
        fn keep(comptime flag:bool,value:bool)->bool {if flag {value} else {not value}}
        fn factory(again:bool,value:T)->Callback where T {
            if again {factory(false,value.ref)} else {keep}
        }
        fn run()->bool {let callback=factory(true,true);callback(true,true)}
        fn independent()->bool {keep(true,true)}";
    let package = support::compile(source);
    let failure = luna::instantiation::instantiate(&package, "run", 5_000).unwrap_err();
    assert!(
        failure.contains("budget") || failure.contains("exhausted"),
        "{failure}"
    );
    let graph = luna::instantiation::instantiate(&package, "independent", 100_000).unwrap();
    assert_eq!(
        Interpreter::from_iir(&graph, Budget::default())
            .unwrap()
            .invoke_instance(&graph.roots[0], vec![])
            .unwrap(),
        Value::Bool(true)
    );
}
