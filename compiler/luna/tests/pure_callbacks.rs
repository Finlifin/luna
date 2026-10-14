//! Behavior evidence is specialized by delivered code identity, not its type.
use comptime_interpretor::Value;
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{compile, compile_with_protocols};

#[test]
fn known_named_callbacks_flow_through_helper_parameters() {
    let package = compile("fn invert(value:bool)->bool {not value} fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)} fn forward(callback:fn(bool)->bool,value:bool)->bool {apply(callback,value)} pure fn run()->bool {forward(invert,false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn finite_known_callback_chains_can_supply_defaults() {
    let package = compile("fn invert(value:bool)->bool {not value} fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)} fn take(value:bool,.opposite:bool=apply(invert,value))->bool {opposite} fn run()->bool {take(false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn unknown_callback_parameters_do_not_supply_a_purity_promise() {
    let error = compile_with_protocols("pure fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)}", &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn same_signature_does_not_reuse_another_callbacks_evidence() {
    let error = compile_with_protocols("fn apply(callback:fn(*bool)->bool,value:*bool)->bool {callback(value)} fn good(value:*bool)->bool {true} fn bad(value:*bool)->bool {value.*} pure fn run(value:*bool)->bool {apply(good,value) and apply(bad,value)}", &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn local_code_aliases_preserve_identity_through_preparation() {
    let package = compile("fn invert(value:bool)->bool {not value} fn apply(callback:fn(bool)->bool,value:bool)->bool {let alias=callback;alias(value)} pure fn run()->bool {let first=invert;let second=first;apply(second,false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn callback_cycles_do_not_supply_termination_evidence() {
    let error = compile_with_protocols("fn cycle(value:bool)->bool {cycle(value)} fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)} fn take(.value:bool=apply(cycle,false))->bool {value}", &Default::default()).unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn reassignment_cannot_retain_the_previous_callback_identity() {
    for body in ["callback=bad;callback(value)", "if true {callback=bad;} callback(value)", "while true {callback(value);callback=bad;}true"] {
        let source = format!("fn good(value:*bool)->bool {{true}} fn bad(value:*bool)->bool {{value.*}} fn apply(callback:fn(*bool)->bool,value:*bool)->bool {{{body}}} pure fn run(value:*bool)->bool {{apply(good,value)}}");
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("P4 evidence"), "{error}");
    }
}

#[test]
fn anonymous_callbacks_and_local_aliases_have_body_evidence() {
    let package = compile("fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)} pure fn run()->bool {let action=|value:bool| not value;apply(action,false) and apply(|value| value,true)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn anonymous_callbacks_can_supply_finite_defaults() {
    let package = compile("fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)} fn take(.value:bool=apply(|value| not value,false))->bool {value} fn run()->bool {take()}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn anonymous_code_identity_does_not_hide_effectful_or_divergent_bodies() {
    let error = compile_with_protocols("fn apply(callback:fn(*bool)->bool,value:*bool)->bool {callback(value)} pure fn run(value:*bool)->bool {apply(|p| true,value) and apply(|p| p.*,value)}", &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
    let error = compile_with_protocols("fn apply(callback:fn()->bool)->bool {callback()} fn take(.value:bool=apply(|| do {while true {} true}))->bool {value}", &Default::default()).unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn anonymous_higher_order_body_receives_the_same_code_premises() {
    let package = compile("fn compose(outer:fn(fn(bool)->bool,bool)->bool,inner:fn(bool)->bool,value:bool)->bool {outer(inner,value)} pure fn run()->bool {compose(|callback,value| callback(value),|value| not value,false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn anonymous_callback_cleanup_is_checked_after_invocation() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Owned {} impl Drop for Owned {fn drop(*self) {}} fn apply(callback:fn()->bool)->bool {callback()} pure fn run()->bool {apply(|| do {let owned=Owned {};true})}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
    let invalid = source.replace("struct Owned {}", "struct Owned {flag:bool}")
        .replace("fn drop(*self) {}", "fn drop(*self) {self.flag.* = true;}")
        .replace("owned=Owned {}", "owned=Owned {flag:false}");
    let error = execution_support::compile_with_source_bindings(&invalid, &Default::default(), selectors).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn readonly_value_captures_keep_their_environment_through_higher_order_calls() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} fn apply(callback:%{flag:bool} fn(bool)->bool,value:bool)->bool {callback(value)} pure fn run()->bool {let flag=true;let action=|value:bool| flag and value;apply(action,true)}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn owned_capture_updates_remain_private_through_by_value_parameters() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} fn apply(callback:%{flag:bool} fn()->bool)->bool {let first=callback();let second=callback();first and not second} pure fn run()->bool {var flag=false;let action=|| do {flag=not flag;flag};apply(action) and not flag}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn readonly_capture_is_a_snapshot_and_can_be_invoked_repeatedly() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} fn twice(callback:%{flag:bool} fn()->bool)->bool {callback() and callback()} pure fn run()->bool {var flag=true;let action=|| flag;flag=false;twice(action) and not flag}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn captured_pointer_representation_does_not_authorize_reading_its_target() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} fn apply(callback:%{p:*bool} fn()->bool)->bool {callback()} pure fn read(p:*bool)->bool {apply(|| p.*)}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let error = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn factories_return_named_and_anonymous_code_to_pure_callers() {
    for factory in ["fn factory()->fn(bool)->bool {invert}", "fn factory()->fn(bool)->bool {|value| not value}"] {
        let source = format!("fn invert(value:bool)->bool {{not value}} {factory} pure fn run()->bool {{let action=factory();action(false) and factory()(false)}}");
        let package = compile(&source);
        assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
    }
}

#[test]
fn returned_code_preserves_forwarded_callback_identity() {
    let package = compile("fn factory(callback:fn(bool)->bool)->fn(bool)->bool {callback} pure fn run()->bool {factory(|value| not value)(false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn factory_behavior_and_returned_behavior_are_checked_separately() {
    for source in [
        "fn factory(p:*bool)->fn()->bool {p.* = true;|| true} pure fn run(p:*bool)->bool {factory(p)()}",
        "fn factory()->fn(*bool)->bool {|p| p.*} pure fn run(p:*bool)->bool {factory()(p)}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(error.contains("P4 evidence"), "{error}");
    }
}

#[test]
fn finite_factory_and_returned_body_can_supply_a_default() {
    let package = compile("fn factory()->fn(bool)->bool {|value| not value} fn take(.value:bool=factory()(false))->bool {value} fn run()->bool {take()}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
    let error = compile_with_protocols("fn factory()->fn()->bool {factory()} fn take(.value:bool=factory()())->bool {value}", &Default::default()).unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn generic_factory_returns_specialized_code() {
    let package = compile("fn factory()->fn(T)->T where T {|value| value} pure fn run()->bool {factory<bool>()(true)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn factory_returns_a_readonly_capture_snapshot() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} fn factory(flag:bool)->%{flag:bool} fn()->bool {|| flag} pure fn run()->bool {let action=factory(true);action()}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn factory_candidate_sets_do_not_select_one_convenient_body() {
    let source = "fn good(p:*bool)->bool {true} fn bad(p:*bool)->bool {p.*} fn choose(flag:bool)->fn(*bool)->bool {if flag {good} else {bad}} pure fn run(flag:bool,p:*bool)->bool {choose(flag)(p)}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn all_named_candidates_are_checked_through_factories_and_parameters() {
    let package = compile("fn keep(value:bool)->bool {value} fn invert(value:bool)->bool {not value} fn choose(flag:bool)->fn(bool)->bool {if flag {keep} else {invert}} fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)} pure fn run()->bool {apply(choose(true),true) and apply(choose(false),false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn direct_conditional_and_mixed_anonymous_candidates_keep_all_bodies() {
    let package = compile("fn keep(value:bool)->bool {value} fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)} pure fn select(flag:bool,value:bool)->bool {let action=if flag {keep} else {|value:bool| not value};apply(action,value)} fn run()->bool {select(true,true) and select(false,false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn multiple_anonymous_factory_results_remain_distinct() {
    let package = compile("fn choose(flag:bool)->fn(bool)->bool {if flag {|value| value} else {|value| not value}} fn apply(callback:fn(bool)->bool,value:bool)->bool {callback(value)} pure fn run()->bool {apply(choose(true),true) and apply(choose(false),false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn default_suppliers_require_every_candidate_to_finish() {
    let prefix = "fn keep(value:bool)->bool {value} fn invert(value:bool)->bool {not value} fn choose(flag:bool)->fn(bool)->bool {if flag {keep} else {invert}}";
    let package = compile(&format!("{prefix} fn take(flag:bool,.value:bool=choose(flag)(flag))->bool {{value}} fn run()->bool {{take(true) and take(false)}}"));
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
    let source = format!("{} fn take(flag:bool,.value:bool=choose(flag)(flag))->bool {{value}}", prefix.replace("{not value}", "{while true {} true}"));
    let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn an_unknown_branch_cannot_be_dropped_from_code_evidence() {
    let error = compile_with_protocols("fn keep(value:bool)->bool {value} pure fn run(flag:bool,unknown:fn(bool)->bool)->bool {let action=if flag {keep} else {unknown};action(true)}", &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn match_results_merge_code_identities() {
    let package = compile("fn keep(value:bool)->bool {value} fn invert(value:bool)->bool {not value} pure fn select(flag:bool)->bool {let action=flag match {true=>keep,false=>invert};action(flag)} fn run()->bool {select(true) and select(false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn multiple_readonly_environments_remain_separate_resource_candidates() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} fn choose(flag:bool)->%{flag:bool} fn()->bool {if flag {|| flag} else {|| not flag}} fn apply(callback:%{flag:bool} fn()->bool)->bool {callback()} pure fn run()->bool {apply(choose(true)) and apply(choose(false))}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn direct_and_returned_forms_of_the_same_code_are_not_duplicate_candidates() {
    let package = compile("fn keep(value:bool)->bool {value} fn factory()->fn(bool)->bool {keep} fn apply(callback:fn(bool)->bool)->bool {callback(true)} pure fn select(flag:bool)->bool {apply(if flag {keep} else {factory()})} fn run()->bool {select(true) and select(false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn invariant_callbacks_can_be_invoked_in_runtime_loops() {
    let package = compile("fn repeat(callback:fn(bool)->bool,value:bool,count:.u8'builtin)->bool {var remaining=count;var result=value;while remaining > 0 {remaining=remaining - 1;result=callback(result);}result} pure fn run()->bool {repeat(|value| not value,true,2) and repeat(|value| not value,false,3)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn nested_loops_preserve_invariant_candidate_sets() {
    let package = compile("fn keep(value:bool)->bool {value} fn invert(value:bool)->bool {not value} fn twice(callback:fn(bool)->bool,value:bool)->bool {var outer:.u8'builtin=2;var result=value;while outer > 0 {outer=outer - 1;var inner:.u8'builtin=2;while inner > 0 {inner=inner - 1;result=callback(result);}}result} pure fn choose(flag:bool)->bool {twice(if flag {keep} else {invert},true)} fn run()->bool {choose(true) and choose(false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn loop_scan_finds_conditional_and_nested_code_assignments() {
    for update in ["if true {callback=bad;}", "while true {callback=bad;break;}"] {
        let source = format!("fn good(p:*bool)->bool {{true}} fn bad(p:*bool)->bool {{p.*}} fn apply(callback:fn(*bool)->bool,p:*bool)->bool {{var count:.u8'builtin=2;while count > 0 {{count=count - 1;callback(p);{update}}}true}} pure fn run(p:*bool)->bool {{apply(good,p)}}");
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("P4 evidence"), "{error}");
    }
}

#[test]
fn loop_invariant_code_does_not_prove_default_termination() {
    let source = "fn apply(callback:fn()->bool)->bool {while true {callback();}true} fn take(.value:bool=apply(|| true))->bool {value}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn loop_carried_named_code_is_closed_before_first_iteration() {
    let package = compile("fn keep(value:bool)->bool {value} fn invert(value:bool)->bool {not value} fn repeat(callback:fn(bool)->bool,count:.u8'builtin)->bool {var result=false;var remaining=count;while remaining > 0 {result=callback(result);callback=invert;remaining=remaining - 1;}result} pure fn run()->bool {repeat(keep,2) and not repeat(keep,3)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn loop_code_alias_equations_converge_across_multiple_bindings() {
    let package = compile("fn keep(value:bool)->bool {value} fn invert(value:bool)->bool {not value} pure fn run()->bool {var action=keep;var next=keep;var remaining:.u8'builtin=3;var value=false;while remaining > 0 {value=action(value);action=next;next=invert;remaining=remaining - 1;}value}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn callback_assignment_branches_keep_all_possible_bodies() {
    let package = compile("fn keep(value:bool)->bool {value} fn invert(value:bool)->bool {not value} pure fn select(flag:bool)->bool {var action=keep;if flag {action=invert;}action(flag)} fn run()->bool {not select(true) and not select(false)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn loop_carried_anonymous_code_keeps_both_iterations() {
    let package = compile("pure fn run()->bool {var action=|value:bool| value;var remaining:.u8'builtin=2;var value=false;while remaining > 0 {value=action(value);action=|value:bool| not value;remaining=remaining - 1;}value}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn loop_early_exits_do_not_erase_callback_candidates() {
    let source = "fn good(p:*bool)->bool {true} fn bad(p:*bool)->bool {p.*} pure fn run(p:*bool,flag:bool)->bool {var action=good;while true {if flag {action=bad;break;}action=good;break;}action(p)}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn unknown_loop_sources_remain_unknown_after_candidate_widening() {
    let source = "fn good(value:bool)->bool {value} pure fn run(unknown:fn(bool)->bool)->bool {var action=good;var remaining:.u8'builtin=2;var value=true;while remaining > 0 {value=action(value);action=unknown;remaining=remaining - 1;}value}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn loop_factory_updates_close_over_prior_callback_candidates() {
    let package = compile("fn keep(value:bool)->bool {value} fn invert(value:bool)->bool {not value} fn factory(previous:fn(bool)->bool,flag:bool)->fn(bool)->bool {if flag {previous} else {invert}} pure fn run()->bool {var action=keep;var value=false;var remaining:.u8'builtin=2;while remaining > 0 {value=action(value);action=factory(action,false);remaining=remaining - 1;}value}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn loop_factory_can_supply_anonymous_code() {
    let package = compile("fn factory()->fn(bool)->bool {|value| not value} pure fn run()->bool {var action=|value:bool| value;var value=false;var remaining:.u8'builtin=2;while remaining > 0 {value=action(value);action=factory();remaining=remaining - 1;}value}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn loop_identity_inference_does_not_authorize_factory_side_effects() {
    let source = "fn keep(value:bool)->bool {value} fn factory(p:*bool)->fn(bool)->bool {p.* = true;keep} pure fn run(p:*bool)->bool {var action=keep;var value=false;var remaining:.u8'builtin=2;while remaining > 0 {value=action(value);action=factory(p);remaining=remaining - 1;}value}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn loop_factory_results_with_unknown_code_do_not_reuse_the_entry_value() {
    let source = "fn keep(value:bool)->bool {value} fn factory(unknown:fn(bool)->bool)->fn(bool)->bool {unknown} pure fn run(unknown:fn(bool)->bool)->bool {var action=keep;var value=false;var remaining:.u8'builtin=2;while remaining > 0 {value=action(value);action=factory(unknown);remaining=remaining - 1;}value}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn loop_factory_itself_can_arrive_as_a_known_higher_order_parameter() {
    let package = compile("fn change(previous:fn(bool)->bool)->fn(bool)->bool {|value| not value} fn iterate(factory:fn(fn(bool)->bool)->fn(bool)->bool,action:fn(bool)->bool)->bool {var remaining:.u8'builtin=2;var value=false;while remaining > 0 {value=action(value);action=factory(action);remaining=remaining - 1;}value} pure fn run()->bool {iterate(change,|value| value)}");
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn affine_capture_can_transfer_through_one_higher_order_invocation() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Token {flag:bool} fn invoke(action:%{token:Token} fn()->Token)->Token {action()} pure fn run()->bool {let token=Token {flag:true};let action=|| token;let result=invoke(action);result.flag}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn repeated_owned_capture_consumption_cannot_be_declared_pure() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Token {flag:bool} fn invoke(action:%{token:Token} fn()->Token)->(Token,Token) {(action(),action())} pure fn run()->(Token,Token) {let token=Token {flag:true};invoke(|| token)}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let error = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn owned_capture_cleanup_cannot_hide_an_effectful_destructor() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Token {flag:bool} impl Drop for Token {fn drop(*self) {self.flag.* = false;}} fn invoke(action:%{token:Token} fn()->bool)->bool {action()} pure fn run()->bool {let token=Token {flag:true};invoke(|| token.flag)}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let error = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn owned_capture_factory_preserves_delivery_into_the_returned_environment() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Token {flag:bool} fn make(token:Token)->%{token:Token} fn()->Token {|| token} fn invoke(action:%{token:Token} fn()->Token)->Token {action()} pure fn run()->bool {let action=make(Token {flag:true});invoke(action).flag}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn partial_owned_capture_transfer_checks_remaining_environment_cleanup() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Token {flag:bool} impl Drop for Token {fn drop(*self) {}} struct Pair {first:Token,second:Token} fn invoke(action:%{pair:Pair} fn()->Token)->Token {action()} pure fn run()->bool {let pair=Pair {first:Token {flag:true},second:Token {flag:false}};invoke(|| pair.first).flag}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn local_mutable_closure_has_independent_value_storage() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} pure fn run()->bool {var flag=false;let left=|| do {flag=not flag;flag};let right=|| do {flag=not flag;flag};let first=left();let other=right();let second=left();first and other and not second and not flag}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn private_capture_storage_does_not_include_pointer_targets() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} fn apply(callback:%{p:*bool} fn()->bool)->bool {callback()} pure fn run(p:*bool)->bool {apply(|| do {p.* = true;true})}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let error = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn owned_struct_capture_can_update_private_scalar_fields() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct State {flag:bool} fn apply(callback:%{state:State} fn()->bool)->bool {let first=callback();let second=callback();first and not second} pure fn run()->bool {let state=State {flag:false};apply(|| do {state.flag=not state.flag;state.flag})}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn factory_created_mutable_environments_do_not_share_state() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} fn make(flag:bool)->%{flag:bool} fn()->bool {|| do {flag=not flag;flag}} pure fn run()->bool {let left=make(false);let right=make(true);let a=left();let b=right();let c=left();a and not b and not c}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
}

#[test]
fn owned_capture_field_replacement_validates_retired_and_new_values() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Token {flag:bool} impl Drop for Token {fn drop(*self) {}} struct State {item:Token} fn apply(callback:%{state:State} fn()->bool)->bool {callback() and callback()} pure fn run()->bool {let state=State {item:Token {flag:false}};apply(|| do {state.item=Token {flag:true};state.item.flag})}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
    let invalid = source.replace("fn drop(*self) {}", "fn drop(*self) {self.flag.* = false;}");
    let error = execution_support::compile_with_source_bindings(&invalid, &Default::default(), selectors).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn early_return_from_replacement_rhs_keeps_the_original_cleanup() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Token {} impl Drop for Token {fn drop(*self) {}} pure fn replace(value:Token)->bool {value=do {return true;};false} fn run()->bool {replace(Token {})}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
    use elaboration::verification::moves::{self, CleanupReason, Delivery};
    let function = package.functions.iter().find(|function| function.name == "replace").unwrap();
    let report = moves::analyze_in_package(&package,function,|ty| Ok(if matches!(ty,nir::Term::Nominal {..}) {Delivery::Move} else {Delivery::Copy}),100_000).unwrap();
    assert!(report.cleanup_sites.iter().flat_map(|site| &site.actions).any(|action|
        matches!(action, moves::CleanupAction::Owned { ty: nir::Term::Nominal { .. }, .. })));
    assert!(!report.cleanup_sites.iter().any(|site| site.reason == CleanupReason::Replacement));
}

#[test]
fn affine_self_replacement_does_not_retire_the_value_being_transferred() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Token {flag:bool} impl Drop for Token {fn drop(*self) {}} pure fn move_back(value:Token)->Token {value=value;value} fn run()->bool {move_back(Token {flag:true}).flag}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
    };
    let package = execution_support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(luna::execution::interpret_with_protocols_in_package(
        &package,0,"run","Copy","Drop",100_000).unwrap(), Value::Bool(true));
    use elaboration::verification::moves::{self, CleanupReason, Delivery};
    let function = package.functions.iter().find(|function| function.name == "move_back").unwrap();
    let report = moves::analyze_in_package(&package,function,|ty| Ok(if matches!(ty,nir::Term::Nominal {..}) {Delivery::Move} else {Delivery::Copy}),100_000).unwrap();
    assert!(!report.cleanup_sites.iter().any(|site| site.reason == CleanupReason::Replacement));
}
