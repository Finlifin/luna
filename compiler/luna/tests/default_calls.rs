//! Default suppliers remain runtime computations with verified finite behavior.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{
    calls, compile_with_tuple as compile, compile_with_tuple_protocols as compile_with_protocols,
    id,
};

fn run(source: &str, expected: Value, order: &[&str]) {
    let package = compile(source);
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(&package, "run"), vec![]).unwrap(), expected);
    assert_eq!(calls(&package, &vm), order);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        expected
    );
}

#[test]
fn inline_control_in_runtime_defaults_keeps_selector_dependencies_and_all_branches() {
    for expression in [
        "inline if mode {not value} else {value}",
        "inline when {mode=>not value,else=>value}",
        "inline if mode is do {true=>not value,false=>value}",
        "inline if true is do {'mode=>not value,_=>value}",
        "inline if true is do { < mode > =>not value,_=>value}",
        "do {let selected=mode; inline if selected {not value} else {value}}",
        "do {let (first and (inline if mode {not first} else {first}) is result)=value; result}",
    ] {
        let source = format!(
            "fn take(comptime .mode:bool=true,value:bool,.next:bool={expression})->bool {{next}}
            fn run()->bool {{take(false) and not take(false,.mode=false)}}"
        );
        let package = execution_support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{expression}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let unused = source.replace("take(false) and not take(false,.mode=false)", "true");
        execution_support::compile_with_protocols(&unused, &Default::default()).unwrap();
        let bad = unused
            .replace("not value", "value + value")
            .replace("not first", "first + first");
        assert!(execution_support::compile_with_protocols(&bad, &Default::default()).is_err());
        let unstaged = unused.replace("comptime .mode", ".mode");
        assert!(
            execution_support::compile_with_protocols(&unstaged, &Default::default()).is_err(),
            "runtime selector was accepted: {expression}"
        );
    }
    let bad = "fn take(comptime seed:bool,mode:bool,.next:bool=inline if mode {seed} else {false})->bool {next}
        fn run()->bool {take(true,true,.next=true)}";
    assert!(execution_support::compile_with_protocols(bad, &Default::default()).is_err());
}

#[test]
fn constant_inline_default_does_not_require_an_open_method_signature() {
    for value in [
        "inline if true {true} else {false}",
        "inline if true {mode} else {false}",
        "inline when {true=>mode,else=>false}",
        "inline if true is do {true=>mode,false=>false}",
    ] {
        let source = format!(
            "struct Holder {{fn take(comptime mode:bool,.next:bool={value})->bool {{next}}}}
            fn run()->bool {{Holder.take(true)}}"
        );
        let package =
            execution_support::compile_with_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn inline_suppliers_in_defaults_are_checked_before_inputs_are_available() {
    let source = include_str!("../../../development/repros/default-forwarded-comptime-input.fl");
    let package = execution_support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let unused = source.replace("take(true, false) and not take(false, false)", "true");
    execution_support::compile_with_protocols(&unused, &Default::default()).unwrap();
    let with_resources = format!(
        "mod resources {{ {} }}\n{unused}",
        include_str!("../../../library/builtin/protocols/resources.fl")
    );
    execution_support::compile_with_source_bindings(
        &with_resources,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.resources.Copy"),
            drop: Some("input.resources.Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    // Explicit override cannot hide a divergent or ill-typed supplier branch.
    let overridden = source
        .replace("take(true, false)", "take(true, false, .next = true)")
        .replace("take(false, false)", "take(false, false, .next = false)");
    for invalid in [
        overridden.replace("else { value }", "else { invert(mode, value) }"),
        overridden.replace("else { value }", "else { value + value }"),
        source.replace("fn take(comptime mode", "fn take(mode"),
    ] {
        assert!(
            execution_support::compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn single_iteration_defaults_require_no_fallthrough_or_backedge() {
    let source = "fn take(flag:bool,.next:bool=do {var result=false;while true {if flag {result=true;break;} break;} result})->bool {next} fn run()->bool {take(true) and not take(false)}";
    for candidate in [
        source.to_owned(),
        source.replace("if flag", "while true {break;} if flag"),
    ] {
        let package =
            execution_support::compile_with_protocols(&candidate, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    for candidate in [
        source.replace("result=true;break;", "result=true;continue;"),
        source.replace("} break;", "}"),
        source.replace("if flag", "while true {} if flag"),
        source
            .replace("result=true;break;", "result=spin();break;")
            .replace("fn take", "fn spin()->bool {spin()} fn take"),
    ] {
        assert!(
            execution_support::compile_with_protocols(&candidate, &Default::default()).is_err(),
            "{candidate}"
        );
    }
}

#[test]
fn conditional_constants_in_defaults_wait_for_guard_inputs() {
    let source = include_str!("../../../development/repros/default-conditional-constant.fl");
    for candidate in [
        source.to_owned(),
        source.replace("found if mode = true", "true as found = mode"),
        source.replace("found if mode = true", "'mode as found = true"),
        source.replace("if mode", "if found and mode"),
        source.replace(
            "const found if mode = true else { break; };",
            "const T if mode = bool else { break; }; let found:T = true;",
        ),
        format!(
            "comptime fn enabled(flag:bool)->bool {{flag}} {}",
            source.replace("if mode", "if enabled(mode)")
        ),
    ] {
        let package = execution_support::compile_with_protocols(&candidate, &Default::default())
            .unwrap_or_else(|error| panic!("{candidate}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let unused = candidate.replace("take(true) and not take(false)", "true");
        execution_support::compile_with_protocols(&unused, &Default::default()).unwrap();
    }
    for invalid in [
        source.replace("comptime mode", "mode"),
        source.replace("else { break; }", "else { false; }"),
        source.replace("else { break; }", "else { selected = found; break; }"),
        source.replace("if mode", "if mode + mode"),
        format!(
            "fn enabled(flag:bool)->bool {{flag}} {}",
            source.replace("if mode", "if enabled(mode)")
        ),
        source.replace("else { break; }", "else { continue; }"),
        source.replace("else { break; }", "else { while true {} }"),
    ] {
        assert!(
            execution_support::compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn generic_open_function_constants_preserve_explicit_and_contextual_clauses() {
    let prefix = "typealias Byte=.u8'builtin; fn choose(comptime mode:bool,first:T,second:T)->T where T {inline if mode {first} else {second}}";
    for binding in [
        "const operation=choose<bool>;",
        "const operation:fn(comptime bool,bool,bool)->bool=choose;",
    ] {
        let source = format!(
            "{prefix} fn run()->bool {{{binding} const bytes=choose<Byte>; operation(true,true,false) and not operation(false,true,false) and bytes(true,7,9)==7}}"
        );
        let package = execution_support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{binding}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let bad = source.replace("operation(true,true,false)", "operation(true,1,false)");
        assert!(execution_support::compile_with_protocols(&bad, &Default::default()).is_err());
        let runtime = source
            .replace("fn run()->bool", "fn run(mode:bool)->bool")
            .replace("operation(true,true,false)", "operation(mode,true,false)");
        assert!(execution_support::compile_with_protocols(&runtime, &Default::default()).is_err());
    }
    let source = "fn invert(comptime mode:bool,value:bool)->bool where flip:bool {inline if mode==flip {not value} else {value}}
        fn run()->bool {const operation=invert<true>;operation(true,false) and not operation(false,false)}";
    let package = execution_support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        execution_support::compile_with_protocols(
            &source.replace("invert<true>", "invert<bool>"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn open_function_constants_in_defaults_prepare_each_staged_call() {
    let source = include_str!("../../../development/repros/default-open-function-constant.fl");
    let package = execution_support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let unused = source.replace("take(false, false) and not take(true, false)", "true");
    execution_support::compile_with_protocols(&unused, &Default::default()).unwrap();
    for invalid in [
        unused.replace("comptime mode", "mode"),
        unused.replace("else { value }", "else { invert(mode, value) }"),
        unused.replace("else { value }", "else { value + value }"),
        unused.replace("value: bool) -> bool", ".value: bool = false) -> bool"),
    ] {
        assert!(execution_support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn local_constants_in_defaults_wait_for_invocation_inputs() {
    let source = include_str!("../../../development/repros/default-local-comptime-binding.fl");
    let package = execution_support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for binding in [
        "const (selected, ignored) = (not mode, mode);",
        "const original = mode; const selected = not original;",
        "const Flag = bool; const selected:Flag = not mode;",
        "const operation = ordinary; const selected = not mode;",
    ] {
        let mut variant = source.replace("const selected = not mode;", binding);
        if binding.contains("const operation") {
            variant.push_str("\nfn ordinary(value:bool)->bool {value}");
            variant = variant.replace(
                "invert(selected, value)",
                "invert(selected, operation(value))",
            );
        }
        let package = compile_with_protocols(&variant, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    let unused = source.replace("take(false, false) and not take(true, false)", "true");
    execution_support::compile_with_protocols(&unused, &Default::default()).unwrap();
    for invalid in [
        unused.replace("comptime mode", "mode"),
        unused.replace("not mode;", "not value;"),
        unused.replace("else { value }", "else { value + value }"),
    ] {
        assert!(execution_support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn computed_staged_inputs_in_defaults_keep_phase_and_finiteness_checks() {
    for input in [
        "not mode",
        "mode == false",
        "do {let inverse=not mode; inverse}",
    ] {
        let source = format!(
            "fn invert(comptime flag:bool,value:bool)->bool {{inline if flag {{not value}} else {{value}}}}
            fn take(comptime mode:bool,value:bool,.next:bool=invert({input},value))->bool {{next}}
            fn run()->bool {{take(false,false) and not take(true,false)}}"
        );
        let package = execution_support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{input}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let unused = source.replace("take(false,false) and not take(true,false)", "true");
        execution_support::compile_with_protocols(&unused, &Default::default()).unwrap();
        for invalid in [
            unused.replace("comptime mode", "mode"),
            unused.replace("else {value}", "else {invert(flag,value)}"),
            unused.replace("else {value}", "else {value + value}"),
        ] {
            assert!(
                execution_support::compile_with_protocols(&invalid, &Default::default()).is_err(),
                "{invalid}"
            );
        }
    }
}

#[test]
fn runtime_defaults_forward_declared_staged_inputs_without_guessing_values() {
    let source = "fn invert(comptime mode:bool,value:bool)->bool {if mode {not value} else {value}}
        fn take(comptime mode:bool,value:bool,.next:bool=invert(mode,value))->bool {next}
        fn run()->bool {take(true,false) and not take(false,false)}";
    let package = execution_support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        source.replace("fn take(comptime mode", "fn take(mode"),
        source.replace(
            "not value} else {value}",
            "not value} else {invert(mode,value)}",
        ),
    ] {
        assert!(
            execution_support::compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn computed_code_patterns_in_defaults_require_known_finite_behavior() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    let source = "trait Copy {} trait Drop {fn drop(*self);}
        typealias Action=fn(bool)->bool;impl Copy for Action {}
        fn invert(value:bool)->bool {not value}
        fn produce()->Action {invert}
        fn take(.value:bool=false match {_ and produce() is action=>action(false)})->bool {value}
        fn run()->bool {take()}";
    for expression in [
        "false match {_ and produce() is action=>action(false)}",
        "false match {_ if (true match {true=>produce(),false=>invert})(false)=>true,_=>false}",
        "false match {_ and (produce() match {action=>action}) is next=>next(false)}",
        "false match {_ and do {let _ and produce() is action=false;action} is next=>next(false)}",
    ] {
        let source = source.replace(
            "false match {_ and produce() is action=>action(false)}",
            expression,
        );
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1000000)
                .unwrap(),
            Value::Bool(true)
        );
        let cyclic = source.replace(
            "fn produce()->Action {invert}",
            "fn produce()->Action {produce()}",
        );
        let error = execution_support::compile_with_source_bindings(
            &cyclic,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(error.contains("P3 stability/termination"), "{error}");
    }
}

#[test]
fn finite_call_chains_supply_defaults_at_runtime_in_declaration_order() {
    run(
        "fn choose(first:bool,.second:bool=opposite(first),.third:bool=opposite(second))->Tuple<bool,bool,bool> {(first,second,third)} fn opposite(value:bool)->bool {identity(not value)} fn identity(value:bool)->bool {value} fn input()->bool {true} fn run()->Tuple<bool,bool,bool> {choose(input())}",
        Value::Tuple(vec![
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(true),
        ]),
        &[
            "run", "input", "opposite", "identity", "opposite", "identity", "choose",
        ],
    );
}

#[test]
fn guard_default_code_values_preserve_their_own_identity() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    let source = "trait Copy {} trait Drop {fn drop(*self);}
        typealias Action=fn(bool)->bool;impl Copy for Action {}
        fn invert(value:bool)->bool {not value}
        fn apply(value:bool,.action:Action=invert)->bool {action(value)}
        pure fn run()->bool {false match {_ if apply(false)=>true,_=>false}}";
    let package =
        execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1000000)
            .unwrap(),
        Value::Bool(true)
    );
    let unknown = source
        .replace("pure fn run()->bool", "pure fn run(unknown:Action)->bool")
        .replace("apply(false)=>", "apply(false,.action=unknown)=>");
    let error =
        execution_support::compile_with_source_bindings(&unknown, &Default::default(), selectors)
            .unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn explicit_supply_skips_only_its_default_and_keeps_later_dependencies() {
    run(
        "fn choose(first:bool,.second:bool=opposite(first),.third:bool=opposite(second))->Tuple<bool,bool> {(second,third)} fn opposite(value:bool)->bool {not value} fn input()->bool {true} fn override_value()->bool {true} fn run()->Tuple<bool,bool> {choose(input(),.second=override_value())}",
        Value::Tuple(vec![Value::Bool(true), Value::Bool(false)]),
        &["run", "input", "override_value", "opposite", "choose"],
    );
}

#[test]
fn default_helpers_can_use_private_mutation_and_early_returns() {
    run(
        "fn choose(input:bool,.flag:bool=bounded(input))->bool {flag} fn bounded(value:bool)->bool {var result=false;if value {result=true;}else{return false;}result} fn run()->bool {choose(true) and not choose(false)}",
        Value::Bool(true),
        &["run", "bounded", "choose", "bounded", "choose"],
    );
}

#[test]
fn generic_helpers_are_checked_after_type_substitution() {
    run(
        "fn identity(value:T)->T where T {value} fn choose(value:bool,.other:bool=identity(value))->bool {other} fn run()->bool {choose(true)}",
        Value::Bool(true),
        &["run", "identity", "choose"],
    );
}

#[test]
fn nested_call_arguments_and_private_default_locals_are_stable() {
    run(
        "fn invert(value:bool)->bool {not value} fn choose(.flag:bool=do {var local=true;local=invert(local);invert(local)})->bool {flag} fn run()->bool {choose()}",
        Value::Bool(true),
        &["run", "invert", "invert", "choose"],
    );
}

#[test]
fn recursive_effectful_and_unknown_suppliers_need_stronger_evidence() {
    for source in [
        "fn cycle()->bool {cycle()} fn take(.value:bool=cycle())->bool {value}",
        "fn first()->bool {second()} fn second()->bool {first()} fn take(.value:bool=first())->bool {value}",
        "fn spin()->bool {while true {} true} fn take(.value:bool=spin())->bool {value}",
        "fn write(p:*bool)->bool {p.* = true;true} fn take(p:*bool,.value:bool=write(p))->bool {value}",
        "fn take(action:fn()->bool,.value:bool=action())->bool {value}",
        "struct Owned {} fn helper()->bool {let owned=Owned {};true} fn take(.value:bool=helper())->bool {value}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains("P3 stability/termination"),
            "{source}: {error}"
        );
    }
}

#[test]
fn pointer_defaults_observe_current_contents_after_explicit_inputs() {
    run(
        "fn read(p:*bool)->bool {p.*}
        fn mark(p:*bool)->bool {p.* = true;false}
        fn take(p:*bool,flag:bool,.observed:bool=read(p))->bool {observed}
        fn run()->bool {
            let value=false;let pointer=value.ref;
            let first=take(pointer,true);
            let second=take(pointer,mark(pointer));
            let third=take(pointer,true,.observed=false);
            not first and second and not third
        }",
        Value::Bool(true),
        &["run", "read", "take", "mark", "read", "take", "take"],
    );
}

#[test]
fn pointer_defaults_allow_field_observations_and_transitive_generic_helpers() {
    run(
        "struct Cell {flag:bool}
        fn identity(value:T)->T where T {value}
        fn read(p:*bool)->bool {identity(p).*}
        fn take(p:*Cell,.observed:bool=read(p.flag))->bool {observed}
        fn run()->bool {let cell=Cell {flag:true};take(cell.ref)}",
        Value::Bool(true),
        &["run", "read", "identity", "take"],
    );
}

#[test]
fn pointer_observation_does_not_hide_writes_divergence_or_unknown_calls() {
    for source in [
        "fn change(p:*bool)->*bool {p.* = true;p} fn take(p:*bool,.value:bool=change(p).*)->bool {value}",
        "fn cycle(p:*bool)->bool {p.* and cycle(p)} fn take(p:*bool,.value:bool=cycle(p))->bool {value}",
        "fn take(p:*bool,action:fn(*bool)->*bool,.value:bool=action(p).*)->bool {value}",
        "fn take(p:*bool,.value:bool=do {p.* = true;p.*})->bool {value}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains("P3 stability/termination"),
            "{source}: {error}"
        );
    }
    let error = compile_with_protocols(
        "fn read(p:*bool)->bool {p.*} pure fn bad(p:*bool)->bool {read(p)}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("P4"), "{error}");
}

#[test]
fn immediate_captured_defaults_observe_pointer_and_scalar_snapshots() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn take(p:*bool,flag:bool,.observed:bool=(|| p.* and flag)())->bool {observed}
        fn run()->bool {let value=true;take(value.ref,true) and not take(value.ref,false)}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1000000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn captured_defaults_preserve_purity_and_actual_owner_requirements() {
    for body in ["(|| do {p.* = false;true})()", "(|| action(p))()"] {
        let source = format!(
            "fn take(p:*bool,action:fn(*bool)->bool,.observed:bool={body})->bool {{observed}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("P3 stability/termination"), "{error}");
    }
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        struct Cell {flag:bool}
        fn take(p:*bool,flag:bool,.observed:bool=(|| p.*)())->bool {observed}
        fn run()->bool {let cell=Cell {flag:true};let p=cell.flag.ref;take(p,do {let moved=cell;true})}");
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1000000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn immediate_default_closures_accept_known_callback_arguments() {
    let package=compile("trait Duplicate {} trait Retire {fn drop(*self);}
        fn read(p:*bool)->bool {p.*}
        fn take(p:*bool,.observed:bool=(|callback:fn(*bool)->bool|callback(p))(read))->bool {observed}
        fn run()->bool {let flag=false;not take(flag.ref)}");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1000000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn default_code_captures_use_the_construction_snapshot() {
    let package = compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        fn read(p:*bool)->bool {p.*}
        fn write(p:*bool)->bool {p.* = false;false}
        fn take(p:*bool,.observed:bool=do {
            var action=read;
            (|ignored:bool|action(p))(do {action=write;true})
        })->bool {observed}
        fn run()->bool {let flag=true;take(flag.ref) and flag}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1000000)
            .unwrap(),
        Value::Bool(true)
    );
    let source="fn read(p:*bool)->bool {p.*} fn write(p:*bool)->bool {p.* = false;false}
        fn take(p:*bool,.observed:bool=do {var action=write;(|ignored:bool|action(p))(do {action=read;true})})->bool {observed}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn helper_admission_does_not_hide_call_contracts_or_invalid_arithmetic() {
    let error = compile_with_protocols("fn bounded(value:.u8'builtin)->.u8'builtin {value + 1} fn take(value:.u8'builtin,.next:.u8'builtin=bounded(value))->.u8'builtin {next}", &Default::default()).unwrap_err();
    assert!(!error.contains("not yet implemented"), "{error}");
    assert!(
        error.contains("range is not proven") || error.contains("precondition"),
        "{error}"
    );
}

#[test]
fn finite_arithmetic_helpers_keep_entry_and_result_contracts() {
    run(
        "typealias Byte=.u8'builtin;fn next(value:Byte)->result:Byte where requires value < 255, ensures result == value'old + 1 {value + 1} fn take(value:Byte,.following:Byte=if value < 255 {next(value)} else {value})->Byte where requires value < 255 {following} fn run()->Byte {take(40)}",
        Value::UnsignedInteger(41),
        &["run", "next", "take"],
    );
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
    let source = "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T} pure comptime fn Tuple<...Es:List<Type>>->Type {bool} fn pair(value:bool)->Tuple<bool,bool> {(value,not value)} fn first(pair:Tuple<bool,bool>)->bool {let (first,second)=pair;first} fn take(value:bool,.pair:Tuple<bool,bool> = pair(value),.flag:bool=first(pair))->bool {flag} fn run()->bool {take(true)}";
    let package = execution_support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            tuple: Some("input.Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    let pair = package.functions.iter().find(|f| f.name == "pair").unwrap();
    let nir::Term::Function { result, .. } = &pair.signature else {
        panic!()
    };
    assert!(
        matches!(result.as_ref(), nir::Term::NormalForm { head, .. } if *head == id(&package, "Tuple"))
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn constructor_defaults_keep_field_behavior_and_partial_cleanup_checks() {
    for source in [
        "struct Config {flag:bool} fn recurse(value:bool)->bool {recurse(value)} fn take(value:bool,.config:Config=(Config {flag:recurse(value)}))->bool {config.flag}",
        "struct Config {flag:bool} typealias Action=fn(bool)->bool; fn take(action:Action,value:bool,.config:Config=(Config {flag:action(value)}))->bool {config.flag}",
        "struct Config {flag:bool} fn write(pointer:*bool)->bool {pointer.* = true;true} fn take(pointer:*bool,.config:Config=(Config {flag:write(pointer)}))->bool {config.flag}",
        "struct Held {} struct Config {held:Held,flag:bool} fn compute(value:bool)->bool {not value} fn take(value:bool,.config:Config=(Config {held:Held {},flag:compute(value)}))->bool {config.flag}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains("P3 stability/termination"),
            "{source}: {error}"
        );
    }
}

#[test]
fn constructor_default_fields_preserve_order_and_explicit_override() {
    run("struct Config {first:bool,second:bool}
        fn first(value:bool)->bool {not value} fn second(value:bool)->bool {value}
        fn take(value:bool,.config:Config=(Config {first:first(value),second:second(value)}))->bool {config.first and not config.second}
        fn run()->bool {take(false)}",Value::Bool(true),&["run","first","second","take"]);
    run(
        "struct Config {flag:bool} fn compute(value:bool)->bool {not value}
        fn take(value:bool,.config:Config=(Config {flag:compute(value)}))->bool {config.flag}
        fn run()->bool {take(false,.config=Config {flag:false})}",
        Value::Bool(false),
        &["run", "take"],
    );
}

#[test]
fn finite_owned_factories_do_not_hide_cleanup_or_recursive_work() {
    for body in [
        "let local=Config {flag:value};Config {flag:value}",
        "make(value);Config {flag:value}",
        "let flag=make(value).flag;Config {flag:flag}",
        "build(value)",
    ] {
        let source = format!(
            "struct Config {{flag:bool}} fn make(value:bool)->Config {{Config {{flag:value}}}}
            fn build(value:bool)->Config {{{body}}}
            fn take(value:bool,.config:Config=build(value))->bool {{config.flag}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("P3 stability/termination"),
            "{body}: {error}"
        );
    }
    let source = "struct Config {flag:bool}
        fn build(value:bool)->result:Config where ensures result.flag {Config {flag:true}}
        fn take(value:bool,.config:Config=build(value))->bool {config.flag}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn finite_factory_defaults_preserve_generic_and_callable_results() {
    run(
        "struct Config where T {flag:T}
        fn make(value:T)->Config<T> where T {Config<T> {flag:value}}
        fn take(value:bool,.config:Config<bool> = make(value))->bool {config.flag}
        fn run()->bool {take(true)}",
        Value::Bool(true),
        &["run", "make", "take"],
    );
    run(
        "struct Config {flag:bool} typealias Factory=fn(bool)->Config;
        fn make(value:bool)->Config {Config {flag:not value}}
        fn choose()->Factory {make}
        fn take(value:bool,.config:Config=choose()(value))->bool {config.flag}
        fn run()->bool {take(false)}",
        Value::Bool(true),
        &["run", "choose", "make", "take"],
    );
    run("struct Config {flag:bool}
        fn take(value:bool,.config:Config=(|flag:bool| Config {flag:not flag})(value))->bool {config.flag}
        fn run()->bool {take(false)}",Value::Bool(true),&["run","take"]);
}

#[test]
fn finite_factory_locals_have_checked_cleanup() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    for body in [
        "let scratch=Scratch {};let result=Config {flag:value};result",
        "Scratch {};Config {flag:value}",
        "let flag=Config {flag:value}.flag;Config {flag:flag}",
        "var result=Config {flag:not value};result=Config {flag:value};result",
        "let compute=|flag:bool| {let scratch=Scratch {};flag};Config {flag:compute(value)}",
    ] {
        let source = format!(
            "trait Copy {{}} trait Drop {{fn drop(*self);}}
            struct Scratch {{}} struct Config {{flag:bool}}
            impl Drop for Scratch {{fn drop(*self) {{}}}}
            impl Drop for Config {{fn drop(*self) {{}}}}
            inline fn factory(value:bool)->Config {{{body}}}
            fn take(.config:Config=factory(true))->bool {{config.flag}}
            fn run()->bool {{take()}}"
        );
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_or_else(|e| panic!("{body}: {e}"));
        let mut expanded = package.clone();
        assert!(
            elaboration::program::inlining::expand(&mut expanded, &mut 1000000)
                .unwrap()
                .expanded
                > 0
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(&expanded, "run", "Copy", "Drop", 1000000)
                .unwrap(),
            Value::Bool(true),
            "expanded: {body}"
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1000000)
                .unwrap(),
            Value::Bool(true),
            "{body}"
        );
    }
}

#[test]
fn finite_factory_cleanup_rejects_mutation_divergence_and_invalid_moves() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    for (drop_body, factory_body) in [
        (
            "self.target.*.* = true;",
            "let scratch=Scratch {target:pointer};Config {flag:true}",
        ),
        (
            "spin();",
            "let scratch=Scratch {target:pointer};Config {flag:true}",
        ),
        ("", "let result=Config {flag:true};let taken=result;result"),
    ] {
        let source = format!(
            "trait Copy {{}} trait Drop {{fn drop(*self);}}
            struct Scratch {{target:*bool}} struct Config {{flag:bool}}
            fn spin() {{spin();}}
            impl Drop for Scratch {{fn drop(*self) {{{drop_body}}}}}
            impl Drop for Config {{fn drop(*self) {{}}}}
            fn factory(pointer:*bool)->Config {{{factory_body}}}
            fn take(pointer:*bool,.config:Config=factory(pointer))->bool {{config.flag}}"
        );
        let error = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(
            error.contains("P3 stability/termination"),
            "{drop_body}, {factory_body}: {error}"
        );
    }
}

#[test]
fn finite_factories_transfer_private_owners_to_helpers() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    for body in [
        "inspect(Config {flag:value})",
        "let config=Config {flag:value};inspect(config)",
        "let callback=inspect;callback(Config {flag:value})",
        "let callback=|config:Config| config.flag;callback(Config {flag:value})",
        "apply(Config {flag:value},inspect)",
        "let config=Config {flag:value};inspect(pass(config))",
        "let config=Config {flag:value};inspect(pass_generic(config))",
    ] {
        let source = format!(
            "trait Copy {{}} trait Drop {{fn drop(*self);}}
            struct Config {{flag:bool}} impl Drop for Config {{fn drop(*self) {{}}}}
            typealias Inspect=fn(Config)->bool;impl Copy for Inspect {{}}
            fn inspect(config:Config)->bool {{config.flag}}
            fn pass(config:Config)->Config {{config}}
            fn pass_generic(value:T)->T where T {{value}}
            fn apply(config:Config,callback:Inspect)->bool {{callback(config)}}
            inline fn factory(value:bool)->bool {{{body}}}
            fn take(.flag:bool=factory(true))->bool {{flag}}
            fn run()->bool {{take()}}"
        );
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_or_else(|e| panic!("{body}: {e}"));
        for expanded in [false, true] {
            let mut package = package.clone();
            if expanded {
                assert!(
                    elaboration::program::inlining::expand(&mut package, &mut 1000000)
                        .unwrap()
                        .expanded
                        > 0
                );
            }
            assert_eq!(
                luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1000000)
                    .unwrap(),
                Value::Bool(true),
                "{body}"
            );
        }
    }
}

#[test]
fn finite_owned_calls_preserve_caller_and_affine_boundaries() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    for suffix in [
        "fn take(config:Config,.flag:bool=inspect(config))->bool {flag}",
        "fn factory()->bool {let config=Config {flag:true};let first=inspect(config);inspect(config)} fn take(.flag:bool=factory())->bool {flag}",
        "fn factory()->bool {let config=Config {flag:true};let first=inspect(config);config.flag} fn take(.flag:bool=factory())->bool {flag}",
    ] {
        let source = format!(
            "trait Copy {{}} trait Drop {{fn drop(*self);}}
            struct Config {{flag:bool}} impl Drop for Config {{fn drop(*self) {{}}}}
            fn inspect(config:Config)->bool {{config.flag}} {suffix}"
        );
        let error = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(
            error.contains("P3 stability/termination"),
            "{suffix}: {error}"
        );
    }
    let source = "trait Copy {} trait Drop {fn drop(*self);}
        struct Config {target:*bool} impl Drop for Config {fn drop(*self) {self.target.*.* = true;}}
        fn inspect(config:Config)->bool {true}
        fn factory(pointer:*bool)->bool {inspect(Config {target:pointer})}
        fn take(pointer:*bool,.flag:bool=factory(pointer))->bool {flag}";
    let error =
        execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
            .unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn finite_factories_can_capture_private_owners() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    for body in [
        "let config=Config {flag:value};let compute=|| config.flag;compute()",
        "let compute=make_thunk(value);compute()",
        "let config=Config {flag:value};let compute=|| inspect(config);compute()",
        "let config=Config {flag:value};let unused=|| {spin();config.flag};value",
        "let holder=Holder {config:Config {flag:not value},action:invert};let compute=|| holder.action(holder.config.flag);compute()",
    ] {
        let source = format!(
            "trait Copy {{}} trait Drop {{fn drop(*self);}}
            struct Config {{flag:bool}} impl Drop for Config {{fn drop(*self) {{}}}}
            typealias Action=fn(bool)->bool;impl Copy for Action {{}}
            struct Holder {{config:Config,action:Action}}
            typealias Thunk=%{{config:Config}} fn()->bool;
            fn make_thunk(value:bool)->Thunk {{let config=Config {{flag:value}};|| config.flag}}
            fn inspect(config:Config)->bool {{config.flag}}
            fn invert(value:bool)->bool {{not value}} fn spin() {{spin();}}
            inline fn factory(value:bool)->bool {{{body}}}
            fn take(.flag:bool=factory(true))->bool {{flag}}
            fn run()->bool {{take()}}"
        );
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_or_else(|e| panic!("{body}: {e}"));
        for expanded in [false, true] {
            let mut package = package.clone();
            if expanded {
                assert!(
                    elaboration::program::inlining::expand(&mut package, &mut 1000000)
                        .unwrap()
                        .expanded
                        > 0
                );
            }
            assert_eq!(
                luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1000000)
                    .unwrap(),
                Value::Bool(true),
                "{body}"
            );
        }
    }
}

#[test]
fn finite_owned_capture_checks_transfer_invocation_and_retirement() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    for body in [
        "let config=Config {flag:value};let compute=|| config.flag;config.flag",
        "let config=Config {flag:value};let compute=|| {spin();config.flag};compute()",
    ] {
        let source = format!(
            "trait Copy {{}} trait Drop {{fn drop(*self);}}
            struct Config {{flag:bool}} impl Drop for Config {{fn drop(*self) {{}}}}
            fn spin() {{spin();}}
            fn factory(value:bool)->bool {{{body}}}
            fn take(.flag:bool=factory(true))->bool {{flag}}"
        );
        let error = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(
            error.contains("P3 stability/termination"),
            "{body}: {error}"
        );
    }
    let source="trait Copy {} trait Drop {fn drop(*self);}
        struct Config {target:*bool} impl Drop for Config {fn drop(*self) {self.target.*.* = true;}}
        fn factory(pointer:*bool)->bool {let config=Config {target:pointer};let unused=||config.target.*;true}
        fn take(pointer:*bool,.flag:bool=factory(pointer))->bool {flag}";
    let error =
        execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
            .unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn finite_factories_verify_owned_result_postcondition_cleanup() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    for body in [
        "Config {flag:true}",
        "return Config {flag:true};",
        "let config=Config {flag:true};config",
    ] {
        let source = format!(
            "trait Copy {{}} trait Drop {{fn drop(*self);}}
            struct Config {{flag:bool}} impl Drop for Config {{fn drop(*self) {{}}}}
            inline fn factory()->result:Config where ensures result.flag {{{body}}}
            fn take(.config:Config=factory())->bool {{config.flag}}
            fn run()->bool {{take()}}"
        );
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_or_else(|e| panic!("{body}: {e}"));
        for expanded in [false, true] {
            let mut package = package.clone();
            if expanded {
                assert!(
                    elaboration::program::inlining::expand(&mut package, &mut 1000000)
                        .unwrap()
                        .expanded
                        > 0
                );
            }
            assert_eq!(
                luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1000000)
                    .unwrap(),
                Value::Bool(true),
                "{body}"
            );
        }
    }
}

#[test]
fn finite_postconditions_require_safe_result_retirement() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    for drop_body in ["self.target.*.* = true;", "spin();"] {
        let source=format!("trait Copy {{}} trait Drop {{fn drop(*self);}}
            struct Config {{flag:bool,target:*bool}}
            fn spin() {{spin();}}
            impl Drop for Config {{fn drop(*self) {{{drop_body}}}}}
            fn factory(pointer:*bool)->result:Config where ensures result.flag {{Config {{flag:true,target:pointer}}}}
            fn take(pointer:*bool,.config:Config=factory(pointer))->bool {{config.flag}}");
        let error = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(
            error.contains("P3 stability/termination"),
            "{drop_body}: {error}"
        );
    }
}

#[test]
fn finite_postcondition_cleanup_keeps_generic_instances_separate() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        ..Default::default()
    };
    let prefix="trait Copy {} trait Drop {fn drop(*self);}
        struct Safe {} struct Bad {} struct Config where T {flag:bool,target:*bool}
        impl Drop for Config<Safe> {fn drop(*self) {}}
        impl Drop for Config<Bad> {fn drop(*self) {self.target.*.* = true;}}
        fn factory(pointer:*bool)->result:Config<T> where T, ensures result.flag {Config<T> {flag:true,target:pointer}}";
    let valid = format!(
        "{prefix} fn take(pointer:*bool,.first:Config<Safe> = factory<Safe>(pointer))->bool {{first.flag}}"
    );
    execution_support::compile_with_source_bindings(&valid, &Default::default(), selectors)
        .unwrap();
    for (first, second) in [("Safe", "Bad"), ("Bad", "Safe")] {
        let source=format!("{prefix}
            fn take(pointer:*bool,.first:Config<{first}> = factory<{first}>(pointer),.second:Config<{second}> = factory<{second}>(pointer))->bool {{first.flag and second.flag}}");
        let error = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(
            error.contains("P3 stability/termination"),
            "{first}/{second}: {error}"
        );
    }
}

#[test]
fn finite_closure_factories_preserve_returned_code() {
    for body in [
        "let maker=|| invert;maker()(false)",
        "let maker=|| {return invert;};maker()(false)",
        "let maker=|| {if true {return invert;} identity};maker()(false)",
        "let selected:fn(bool)->bool=invert;let maker=|| selected;maker()(false)",
        "let maker=|| Holder {action:invert};maker().action(false)",
        "let maker=|| {let chosen:fn(bool)->bool=invert;|| chosen(false)};maker()()",
    ] {
        let source = format!(
            "trait Copy {{}} trait Drop {{fn drop(*self);}}
            struct Holder {{action:fn(bool)->bool}}
            fn invert(value:bool)->bool {{not value}}
            fn identity(value:bool)->bool {{value}}
            fn factory()->bool {{{body}}}
            fn take(.value:bool=factory())->bool {{value}}
            fn run()->bool {{take()}}"
        );
        let selectors = luna::protocols::type_constructors::Selectors {
            copy: Some("input.Copy"),
            drop: Some("input.Drop"),
            ..Default::default()
        };
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_or_else(|error| panic!("{body}: {error}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1000000)
                .unwrap(),
            Value::Bool(true),
            "{body}"
        );
        let divergent = source.replace("{not value}", "{invert(value)}");
        let error = execution_support::compile_with_source_bindings(
            &divergent,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(
            error.contains("P3 stability/termination"),
            "{body}: {error}"
        );
    }
}
