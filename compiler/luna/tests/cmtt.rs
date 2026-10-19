#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Interpreter, Value};
use support::run_iir as run;

#[test]
fn generator_match_selects_code_and_lifts_selected_pattern_bindings() {
    let source="enum Mode {off,on(bool),}
        pure comptime fn build(mode:Mode)-><x:bool |- bool> {
            mode match {
                .on(flag) if flag => '(x and flag),
                .on(flag) => '(not x and not flag),
                .off => '(false),
            }
        }
        fn run(value:bool)->bool {
            const enabled=build(Mode.on(true));const inverted=build(Mode.on(false));const off=build(Mode.off);
            ~[x=value]<enabled> and not ~[x=value]<inverted> and not ~[x=value]<off>
        }";
    let package = support::compile(source);
    assert!(
        !package
            .enums
            .iter()
            .any(|item| item.name == "<generator-match-packet>")
    );
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_match_skips_unmatched_guards_and_keeps_candidate_order() {
    let source = "enum Mode {off,on(bool),}
        comptime fn never()->bool {while true {} false}
        pure comptime fn build(mode:Mode)->< |- bool> {
            mode match {
                .on(flag) if never() => '(flag),
                .off => '(true),
                _ => '(false),
            }
        }
        pure comptime fn choice(pair:(bool,bool))->< |- bool> {
            pair match {
                (chosen,false) or (true,chosen) if chosen => '(chosen),
                (left,right) => '(left or right),
            }
        }
        fn run()->bool {~<build(Mode.off)> and ~<choice((true,true))>
            and ~<choice((false,true))> and not ~<choice((false,false))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_match_checks_coverage_unselected_bodies_and_guard_phase() {
    for (source, anchor) in [
        (
            "enum Mode {off,on(bool),} pure comptime fn build(mode:Mode)->< |- bool> {mode match {.off=>'(true)}}",
            "non-exhaustive generator match",
        ),
        (
            "pure comptime fn build(flag:bool)->< |- bool> {flag match {true=>'(true),false=>'( 'x')}}",
            "type",
        ),
        (
            "fn runtime()->bool {true} pure comptime fn build(flag:bool)->< |- bool> {flag match {true if runtime()=>'(true),_=>'(false)}}",
            "execution phase",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
}

#[test]
fn generator_nested_field_updates_drive_code_assembly() {
    let source = "struct Counter {remaining:.u8'builtin}
        struct State {counter:Counter,untouched:bool}
        pure comptime fn build(input:State)-><x:bool |- bool> {
            let state=input;
            var code:<x:bool |- bool> = '(x);
            while state.counter.remaining > 0 {
                code='(not ~<code>);
                state.counter.remaining -= 1;
            }
            '(state.untouched and ~<code>)
        }
        fn run(value:bool)->bool {
            const code=build(State {counter:Counter {remaining:3},untouched:true});
            ~[x=value]<code>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(false)]),
        Value::Bool(true)
    );
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(false)
    );
}

#[test]
fn generator_field_updates_preserve_previously_quoted_snapshots() {
    let source = "struct State {flag:bool}
        pure comptime fn build()->< |- bool> {
            let state=State {flag:true};
            const original='(state.flag);
            state.flag=not state.flag;
            if state.flag {'(false)} else {original}
        }
        fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_anonymous_record_fields_use_the_declared_schema() {
    let source = "comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {enabled:false,stable:true};typealias Config=SAMPLE'type;
        pure comptime fn build(config:Config)->< |- bool> {
            config.enabled=not config.enabled;
            '(config.enabled and config.stable)
        }
        fn run()->bool {~<build(schema {enabled:false,stable:true})>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_bit_view_writeback_keeps_sibling_fields() {
    let source = "struct State {bits:.u8'builtin,other:bool}
        pure comptime fn build()->< |- bool> {
            let state=State {bits:0,other:true};
            state.bits'bitvec[0..4]=9;
            '(state.bits == 9 and state.other)
        }
        fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_dynamic_bit_places_check_bounds_and_recover() {
    let prefix = "struct State {bits:.u8'builtin,index:.u8'builtin}
        pure comptime fn build(index:.u8'builtin)->< |- bool> {
            let state=State {bits:0,index:index};
            state.bits'bitvec[state.index]=1;
            '(state.bits == 2)
        }";
    let failure = support::compile_with_protocols(
        &format!("{prefix} fn run()->bool {{~<build(8)>}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("IndexOutOfBounds"), "{failure}");
    assert_eq!(
        run(
            &format!("{prefix} fn run()->bool {{~<build(1)>}}"),
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn generator_places_reject_wrong_fields_types_and_stage_escapes() {
    for (source, anchor) in [
        (
            "struct State {flag:bool} pure comptime fn bad()->< |- bool> {let state=State {flag:true};state.missing=false;'(true)}",
            "field",
        ),
        (
            "struct State {flag:bool} pure comptime fn bad()->< |- bool> {let state=State {flag:true};state.flag='x';'(true)}",
            "type",
        ),
        (
            "struct State {flag:bool} pure comptime fn bad()-><state:State |- bool> {state.flag=false;'(true)}",
            "runtime context",
        ),
        (
            "struct State {bits:.u8'builtin} pure comptime fn bad()->< |- bool> {let state=State {bits:0};state.bits'bitvec[0..4]=16;'(true)}",
            "BitVec width",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
}

#[test]
fn generator_all_returning_branches_and_loops_need_no_dummy_tail() {
    let source = "pure comptime fn branches(flag:bool)->< |- bool> {
            if flag {return '(true);} else {return '(false);};
        }
        pure comptime fn looping(flag:bool)->< |- bool> {
            while true {
                if false {break;}
                if flag {return '(true);}
                return '(false);
            }
        }
        fn run()->bool {~<branches(true)> and not ~<branches(false)>
            and ~<looping(true)> and not ~<looping(false)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_nested_loop_exits_preserve_the_target_path() {
    let source = "pure comptime fn build(flag:bool)->< |- bool> {
            while:outer true {
                while true {
                    if flag {break outer;}
                    return '(false);
                }
            }
            return '(true);
        }
        fn run()->bool {~<build(true)> and not ~<build(false)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_flow_rejects_any_reachable_missing_result() {
    for source in [
        "pure comptime fn bad(flag:bool)->< |- bool> {if flag {return '(true);};}",
        "pure comptime fn bad(flag:bool)->< |- bool> {while true {if flag {break;}return '(true);}}",
        "pure comptime fn bad(flag:bool)->< |- bool> {while flag {return '(true);}}",
        "pure comptime fn bad(flag:bool)->< |- bool> {while:outer true {while true {if flag {break outer;}return '(true);}}}",
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            failure.contains("without returning a code result"),
            "{source}: {failure}"
        );
    }
}

#[test]
fn generator_divergence_is_not_a_missing_result_or_a_completed_code_value() {
    let prefix = "pure comptime fn forever()->< |- bool> {while true {}}";
    support::compile(&format!("{prefix} fn run()->bool {{true}}"));
    let failure = support::compile_with_protocols(
        &format!("{prefix} fn run()->bool {{~<forever()>}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("budget exhausted"), "{failure}");
    // Unreachable code is still checked, even though its exit cannot make the
    // enclosing generator appear to fall through.
    let failure = support::compile_with_protocols(
        "pure comptime fn bad()->< |- bool> {while true {} return '( 'x');}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("type"), "{failure}");
    assert_eq!(
        run(
            "pure comptime fn maybe(flag:bool)->< |- bool> {while true {if flag {return '(true);}}} fn run()->bool {~<maybe(true)>}",
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn generator_while_assembles_code_and_handles_zero_iterations() {
    let source = "pure comptime fn repeat(count:.u8'builtin)-><x:bool |- bool> {
            var remaining=count;
            var code:<x:bool |- bool> = '(x);
            while remaining > 0 {
                const previous=code;
                code='(not ~<previous>);
                remaining -= 1;
            }
            code
        }
        fn run(value:bool)->bool {
            const zero=repeat(0);const odd=repeat(3);const even=repeat(4);
            ~[x=value]<zero> and not ~[x=value]<odd> and ~[x=value]<even>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_nested_loops_break_continue_and_early_return_have_local_targets() {
    let source = "pure comptime fn build(stop:bool)->< |- bool> {
            var code='(false);
            var outer=true;
            while:again outer {
                outer=false;
                while true {
                    if stop {return '(true);}
                    code='(true);
                    continue again;
                }
                code='(false);
            }
            while:done true {while true {break done;}code='(false);}
            code
        }
        fn run()->bool {~<build(true)> and ~<build(false)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn unused_generator_loops_check_bodies_without_running_them() {
    support::compile(
        "pure comptime fn unused()->< |- bool> {while true {} '(true)} fn run()->bool {true}",
    );
    let failure=support::compile_with_protocols("fn runtime()->bool {true} pure comptime fn unused()->< |- bool> {while false {let value=runtime();} '(true)}",&Default::default()).unwrap_err();
    assert!(failure.contains("execution phase"), "{failure}");
    let failure=support::compile_with_protocols("pure comptime fn endless()->< |- bool> {while true {} '(true)} fn run()->bool {~<endless()>}",&Default::default()).unwrap_err();
    assert!(failure.contains("budget exhausted"), "{failure}");
    assert_eq!(
        run(
            "pure comptime fn good()->< |- bool> {var code='(false);while true {code='(true);break;}code} fn run()->bool {~<good()>}",
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
    for (source, anchor) in [
        (
            "pure comptime fn bad()-><x:bool |- bool> {while x {} '(x)}",
            "runtime context binding",
        ),
        (
            "pure comptime fn bad()->< |- bool> {while true {const code='(do {break;});break;} '(true)}",
            "loop",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
}

#[test]
fn generator_mutable_code_preserves_snapshots_and_open_context() {
    let source = "pure comptime fn build()-><x:bool |- bool> {
            var code:<x:bool |- bool> = '(x);
            const original=code;
            code='(not ~<original>);
            const inverted=code;
            code='(not ~<inverted>);
            code
        }
        fn run(value:bool)->bool {const code=build();~[x=value]<code>}";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
    assert_eq!(
        run(source, "run", vec![Value::Bool(false)]),
        Value::Bool(false)
    );
    assert_eq!(
        run(
            "pure comptime fn build()->< |- bool> {var code='(true);const old=code;code='(false);old} fn run()->bool {~<build()>}",
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn generator_mutable_data_and_nested_blocks_keep_only_outer_updates() {
    let source = "pure comptime fn build()->< |- bool> {
            var enabled=false;
            var code='(false);
            let selected=do {enabled=true;code='(true);let hidden=false;code};
            code='(enabled and ~<selected>);
            code
        }
        fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn code_assignment_transports_new_defaults_without_retargeting_old_aliases() {
    let source = "fn run(left:bool,right:bool)->bool {
        const current=do {
            var code='[port=left](left);
            code='[renamed=right](right);
            code
        };
        const snapshot=do {
            var code='[port=left](left);
            const old=code;
            code='[renamed=right](right);
            old
        };
        ~<snapshot> and not ~<current>
    }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true), Value::Bool(false)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_assignment_rejects_immutable_stage_and_interface_violations() {
    for (source, anchor) in [
        (
            "pure comptime fn build()->< |- bool> {let code='(true);code='(false);code}",
            "immutable",
        ),
        (
            "pure comptime fn build(flag:bool)->< |- bool> {flag=false;'(flag)}",
            "immutable",
        ),
        (
            "pure comptime fn build()-><x:bool |- bool> {x=false;'(x)}",
            "runtime context binding",
        ),
        (
            "pure comptime fn build()->< |- bool> {var code='(true);code='( 'x');code}",
            "type",
        ),
        (
            "pure comptime fn build()-><x:bool |- bool> {var code='(true);code='(x);code}",
            "context connection",
        ),
        (
            "fn run(left:bool,right:bool)->bool {const code=do {var current='(left);current='(right);current};~<code>}",
            "context connection",
        ),
        (
            "fn run(left:bool,right:bool)->bool {const code=do {var current='(left);current='[port=right](right);current};~<code>}",
            "unknown context port",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
}

#[test]
fn generator_let_patterns_unpack_nested_data_and_keep_bindings_private() {
    let source = "struct Options {flags:(bool,bool)}
        pure comptime fn selected(options:Options)-><x:bool |- bool> {
            let _=options;
            let {flags:(enabled,inverted)}=options;
            if enabled {
                let inverted=not inverted;
                '(x and inverted)
            } else {'(false)}
        }
        fn run(value:bool)->bool {
            const code=selected(Options {flags:(true,false)});
            let inverted=false;
            ~[x=value]<code> and not inverted
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_patterns_reuse_alternative_remapping_and_sequential_computation() {
    let source = "pure comptime fn selected(pair:(bool,bool))->< |- bool> {
            let (chosen,true) or (false,chosen) or (chosen,false)=pair;
            let first and not first is opposite=chosen;
            '(first and not opposite)
        }
        fn run()->bool {~<selected((true,true))> and ~<selected((true,false))>
            and not ~<selected((false,true))> and not ~<selected((false,false))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_destructuring_rejects_refutable_and_ill_typed_unused_definitions() {
    for (source, anchor) in [
        (
            "pure comptime fn build(pair:(bool,bool))->< |- bool> {let (true,flag)=pair;'(flag)}",
            "refutable generator let",
        ),
        (
            "struct Config {flag:bool} pure comptime fn build(config:Config)->< |- bool> {let {missing}=config;'(true)}",
            "field",
        ),
        (
            "pure comptime fn build(pair:(bool,'x''type))->< |- bool> {let (flag,_) or (_,flag)=pair;'(true)}",
            "type",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
    assert_eq!(
        run(
            "pure comptime fn build(pair:(bool,bool))->< |- bool> {let (left,right)=pair;'(left and right)} fn run()->bool {~<build((true,true))>}",
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn unused_generator_computations_cannot_read_context_or_call_runtime_functions() {
    for (source, anchor) in [
        (
            "fn runtime()->bool {true} pure comptime fn build()->< |- bool> {let flag=runtime();'(flag)}",
            "execution phase",
        ),
        (
            "fn runtime()->bool {true} pure comptime fn build()->< |- bool> {if runtime() {'(true)} else {'(false)}}",
            "execution phase",
        ),
        (
            "fn runtime()->bool {true} pure comptime fn build(flag:bool)->< |- bool> {let first and runtime() is second=flag;'(second)}",
            "execution phase",
        ),
        (
            "pure comptime fn build()-><x:bool |- bool> {let flag=x;'(flag)}",
            "runtime context binding",
        ),
        (
            "pure comptime fn build()-><x:bool |- bool> {if x {'(true)} else {'(false)}}",
            "runtime context binding",
        ),
        (
            "pure comptime fn leaf(flag:bool)->< |- bool> {'(flag)} pure comptime fn build()-><x:bool |- bool> {leaf(x)}",
            "runtime context binding",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
    // The runtime computation belongs inside the quote, after context supply.
    assert_eq!(
        run(
            "fn runtime(value:bool)->bool {value} pure comptime fn build()-><x:bool |- bool> {'(runtime(x))} fn run(value:bool)->bool {const code=build();~[x=value]<code>}",
            "run",
            vec![Value::Bool(true)]
        ),
        Value::Bool(true)
    );
    // Carrying a runtime callable to a comptime helper does not execute its
    // body while transporting the declaration identity.
    assert_eq!(
        run(
            "fn runtime()->bool {true} comptime fn accepts(action:fn()->bool)->bool {true} pure comptime fn build()->< |- bool> {let flag=accepts(runtime);'(flag)} fn run()->bool {~<build()>}",
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn structured_generator_inputs_drive_code_and_lift_only_completed_data() {
    let source = "struct Options {invert:bool}
        struct Config {options:Options,value:bool}
        pure comptime fn configured(config:Config)-><x:bool |- bool> {
            let options=config.options;
            if options.invert {'(not x)} else {'(x and config.value)}
        }
        pure comptime fn forward(config:Config)-><x:bool |- bool> {configured(config)}
        fn run(value:bool)->bool {
            const inverted=forward(Config {options:Options {invert:true},value:false});
            const direct=configured(Config {options:Options {invert:false},value:true});
            not ~[x=value]<inverted> and ~[x=value]<direct>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_enum_data_can_be_inspected_and_returned_as_runtime_payload() {
    let source = "enum Mode {off,on(bool),}
        pure comptime fn selected(mode:Mode)->< |- bool> {
            let enabled=mode match {.on(flag)=>flag,_=>false,};
            if enabled {'(true)} else {'(false)}
        }
        pure comptime fn payload(mode:Mode)->< |- Mode> {'(mode)}
        fn run()->bool {
            const code=payload(Mode.on(true));
            ~<selected(Mode.on(true))> and not ~<selected(Mode.off)>
                and (~<code> match {.on(flag)=>flag,_=>false,})
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_tuple_data_preserves_heterogeneous_fields() {
    let source = "pure comptime fn selected(config:(bool,'x''type))->< |- bool> {
            let enabled=config match {(flag,'x')=>flag,_=>false,};
            '(enabled)
        }
        fn run()->bool {~<selected((true,'x'))> and not ~<selected((true,'y'))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_type_data_stays_at_comptime() {
    let source = "pure comptime fn is_bool(description:.Type'builtin)->< |- bool> {
            let (description,enabled)=(description,true);
            let matched=description == bool;
            '(matched and enabled)
        }
        fn run()->bool {~<is_bool(bool)> and not ~<is_bool('x''type)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_data_uses_the_bound_tuple_head_and_anonymous_record_schema() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        comptime fn schema(...properties:T)->T where T {properties}
        const SAMPLE=schema {enabled:true};typealias Options=SAMPLE'type;
        pure comptime fn selected(config:Tuple<Options,bool>)->< |- bool> {
            let ({enabled},flag)=config;
            '(enabled and flag)
        }
        fn run()->bool {~<selected((schema {enabled:true},true))>
            and not ~<selected((schema {enabled:true},false))>}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nominal_generator_data_cannot_hide_callable_identity() {
    let prefix = "enum Data {plain(bool),callback(fn()->bool),}
        pure comptime fn build(data:Data)->< |- bool> {'(true)}
        fn action()->bool {true}";
    let failure = support::compile_with_protocols(
        &format!("{prefix} fn run()->bool {{~<build(Data.callback(action))>}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("cannot be frozen"), "{failure}");
    assert_eq!(
        run(
            &format!("{prefix} fn run()->bool {{~<build(Data.plain(true))>}}"),
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn generator_data_rejects_runtime_inputs_wrong_identities_and_opaque_domains() {
    for (source, anchor) in [
        (
            "struct Config {flag:bool} pure comptime fn build(config:Config)->< |- bool> {'(config.flag)} fn run(config:Config)->bool {~<build(config)>}",
            "runtime parameter",
        ),
        (
            "struct Config {flag:bool} struct Other {flag:bool} pure comptime fn build(config:Config)->< |- bool> {'(config.flag)} fn run()->bool {~<build(Other {flag:true})>}",
            "type",
        ),
        (
            "pure comptime fn Identity<T>->.Type'builtin {T} pure comptime fn build(config:Identity<bool>)->< |- bool> {'(true)}",
            "immutable value domain",
        ),
        (
            "pure comptime fn build(pointer:*bool)->< |- bool> {'(true)} fn run(pointer:*bool)->bool {~<build(pointer)>}",
            "runtime parameter",
        ),
        (
            "struct Description {ty:.Type'builtin} pure comptime fn build(description:Description)->< |- Description> {'(description)}",
            "quoted runtime interface",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
    assert_eq!(
        run(
            "struct Config {flag:bool} pure comptime fn build(config:Config)->< |- bool> {'(config.flag)} fn run()->bool {~<build(Config {flag:true})>}",
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn comptime_generators_select_return_and_compose_expression_code() {
    let source = "pure comptime fn choose(flag:bool)->< |- bool> {
            if flag {'(true)} else {'(false)}
        }
        pure comptime fn invert(code:<x:bool |- bool>)-><x:bool |- bool> {
            '(not ~<code>)
        }
        fn run(value:bool)->bool {
            const original:<x:bool |- bool> = '(x);
            const generated=invert(invert(original));
            ~<choose(true)> and not ~<choose(false)> and ~[x=value]<generated>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_scalar_parameters_and_locals_are_lifted_without_runtime_inputs() {
    let source = "pure comptime fn flag(value:bool)->< |- bool> {
            let inverse=not value;
            let selected=if inverse {'(false)} else {'(value)};
            return selected;
        }
        fn run()->bool {~<flag(true)> and not ~<flag(false)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn annotated_aliases_preserve_context_and_original_default_association() {
    let source = "fn run(value:bool)->bool {
        const original:<value:bool |- bool> = '(value);
        const renamed:<port:bool |- bool> = original;
        let value=false;
        ~<renamed>
    }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_inputs_receive_contextual_results_without_rebinding_source_names() {
    let source = "typealias u32=.u32'builtin;
        pure comptime fn keep(code:<x:bool |- bool>)-><x:bool |- bool> {code}
        pure comptime fn number(code:< |- u32>)->< |- u32> {code}
        fn run(value:bool)->bool {
            const code=keep('[port=value](value));
            let value=false;
            ~<code> and ~<number('(41))> == 41
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_return_alpha_renaming_does_not_swap_original_objects() {
    let source =
        "pure comptime fn keep(code:<a:bool,b:bool |- bool>)-><b:bool,a:bool |- bool> {code}
        fn run(left:bool,right:bool)->bool {
            const original:<left:bool,right:bool |- bool> = '(left and not right);
            const result=keep(original);
            ~<result>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true), Value::Bool(false)]),
        Value::Bool(true)
    );
}

#[test]
fn recursive_generators_are_bounded_and_unused_body_checks_do_not_invoke_them() {
    let definition = "pure comptime fn again()->< |- bool> {again()}";
    support::compile(&format!("{definition} fn run()->bool {{true}}"));
    let failure = support::compile_with_protocols(
        &format!("{definition} fn run()->bool {{~<again()>}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("depth exhausted"), "{failure}");
    assert_eq!(
        run(
            "pure comptime fn good()->< |- bool> {'(true)} fn run()->bool {~<good()>}",
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn invalid_unused_generator_branches_and_code_interfaces_are_rejected() {
    for (source, anchor) in [
        (
            "pure comptime fn bad(flag:bool)->< |- bool> {if flag {'(true)} else {'(do {})}} fn run() {}",
            "type",
        ),
        ("fn bad()->< |- bool> {'(true)}", "explicit comptime fn"),
        (
            "pure comptime fn build(flag:bool)->< |- bool> {'(flag)} fn run(flag:bool)->bool {~<build(flag)>}",
            "runtime",
        ),
        (
            "fn run(value:bool)->bool {const original='(value);const code:<x:bool |- bool> = original;~[x=value]<code>}",
            "fixed code dependency",
        ),
        (
            "pure comptime fn build()->< |- bool> {'(true)} fn run()->bool {build()}",
            "comptime code position",
        ),
        (
            "struct Box {value:.Integer'builtin} pure comptime fn bad(code:<x:Box |- bool>)-><x:Box |- bool> {code}",
            "quoted runtime interface",
        ),
        (
            "pure comptime fn bad(code:< |- .Integer'builtin>)->< |- .Integer'builtin> {code}",
            "quoted runtime interface",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
}

#[test]
fn quotes_preserve_runtime_identity_and_only_execute_at_splice() {
    let source = "fn run(value:bool)->bool {
        var changed=false;
        const unused='(do {changed=true;});
        const code='(value);
        let value=false;
        ~<code> and not changed
    }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn splice_redirects_only_explicit_ports_simultaneously() {
    let source = "fn run(left:bool,right:bool)->bool {
        const code='[a=left,b=right](left and not right);
        ~[a=right,b=left]<code>
    }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(false), Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn contextual_types_supply_virtual_ports_and_preserve_default_identity() {
    for source in [
        "fn run(value:bool)->bool {const code:<input:bool |- bool> = '(input);~[input=value]<code>}",
        "fn run(value:bool)->bool {const code:<value:bool |- bool> = '(value);let value=false;~<code>}",
        "fn run(value:bool)->bool {const code:< |- bool> = '(true);~<code>}",
    ] {
        assert_eq!(
            run(source, "run", vec![Value::Bool(true)]),
            Value::Bool(true)
        );
    }
    for (source, anchor) in [
        (
            "fn bad()->bool {const code:<missing:bool |- bool> = '(missing);~<code>}",
            "missing a required runtime binding",
        ),
        (
            "fn bad(value:bool)->bool {const code:< |- bool> = '(value);~<code>}",
            "absent from its declared context",
        ),
        (
            "typealias Type=.Type'builtin;fn bad()->bool {const code:<T:Type |- bool> = '(true);false}",
            "only runtime bindings",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{failure}");
    }
}

#[test]
fn nested_construction_and_repeated_connection_keep_private_bindings_fresh() {
    let source = "fn run(value:bool)->bool {
        const original='(do {let local=not value;local});
        const alias=original;
        const combined='(not ~<alias>);
        ~<combined> and ~<combined>
    }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn construction_can_choose_code_without_reading_runtime_condition() {
    assert_eq!(
        run(
            "fn run(value:bool)->bool {const code=if true {'(value)} else {'(false)};~<code>}",
            "run",
            vec![Value::Bool(true)]
        ),
        Value::Bool(true)
    );
    let failure = support::compile_with_protocols(
        "fn bad(flag:bool)->bool {const code=if flag {'(true)} else {'(false)};~<code>}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("runtime"), "{failure}");
}

#[test]
fn immediate_splice_and_runtime_branches_remain_runtime_code() {
    let source = "fn run(flag:bool)->bool {~<'(if flag {true} else {false})>}";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
    assert_eq!(
        run(source, "run", vec![Value::Bool(false)]),
        Value::Bool(false)
    );
}

#[test]
fn splice_imports_runtime_dependencies_into_a_closure() {
    let source =
        "fn run(value:bool)->bool {const code='(value);let callback=|| ~<code>;callback()}";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
    let source = "fn run(value:bool)->bool {const code='(|| value);let first=~<code>;let second=~<code>;first() and second()}";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
    let source = "fn run(value:bool)->bool {let left=false;let right=false;const code='[a=left,b=right](|| left and right);let callback=~[a=value,b=value]<code>;callback()}";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn code_expansion_keeps_internal_loops_patterns_and_default_frames_separate() {
    let source = "fn identity(value:bool,.other:bool=value)->bool {other}
        fn run(value:bool)->bool {
            const code='(do {
                var flag=true;
                while flag {flag=false;}
                value match {matched if matched=>identity(matched),_=>false,}
            });
            ~<code> and ~<code>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn substitution_preserves_writability_and_actual_interface_type() {
    for (source, anchor) in [
        (
            "fn bad()->bool {var flag=false;const code='[p=flag](do {flag=true;});let fixed=false;~[p=fixed]<code>;fixed}",
            "immutable",
        ),
        (
            "typealias str=.str'builtin;fn bad(value:bool,text:str)->bool {const code='[p=value](value);~[p=text]<code>}",
            "type",
        ),
        (
            "fn bad()->bool {const code:< |- bool> = '(do {});~<code>}",
            "type",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{failure}");
    }
    assert_eq!(
        run(
            "fn run()->bool {var original=false;var replacement=false;const code='[p=original](do {original=true;});~[p=replacement]<code>;replacement and not original}",
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn rejects_stage_interface_and_control_mismatches() {
    for (source, anchor) in [
        ("fn bad()->bool {const code='(true);code}", "runtime value"),
        ("fn bad(value:bool)->bool {~<value>}", "comptime code"),
        (
            "fn bad(value:bool)->bool {const code='(value);~[value=value]<code>}",
            "fixed dependency",
        ),
        (
            "fn bad(value:bool)->bool {const code='[p=value](value);~[p=value,p=value]<code>}",
            "duplicate splice",
        ),
        (
            "fn bad(value:bool)->bool {const code='[p=value,p=value](value);~<code>}",
            "duplicate quote",
        ),
        (
            "fn bad()->bool {const code='(do {return true;});false}",
            "return",
        ),
        (
            "fn bad()->bool {const code='( '(true) );false}",
            "comptime value",
        ),
        ("comptime fn bad()->bool {~<'(true)>}", "runtime position"),
        (
            "fn bad()->bool {const code='(true);const invalid='(if true {~<code>} else {~<code>});false}",
            "parsing failed",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
}

#[test]
fn expanded_sources_preserve_origins_through_instantiation() {
    let package = support::compile(
        "fn run()->bool {const code='(do {let local=true;local});~<code> and ~<code>}",
    );
    assert!(!package.expansion_sources.is_empty());
    for (index, origin) in package.expansion_sources.iter().enumerate() {
        assert!(
            origin
                .original
                .expansion_index()
                .is_none_or(|original| (original as usize) < index)
        );
        assert!(origin.site.start < origin.site.end);
    }
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert_eq!(package.expansion_sources, graph.expansion_sources);
    assert!(format!("{graph}").contains("(expansion-source"));
    let mut locals = std::collections::BTreeSet::new();
    nir::visit::expression(
        &graph
            .functions
            .iter()
            .find(|f| f.function.name == "run")
            .unwrap()
            .function
            .body,
        &mut 1000,
        |expr, _| {
            if let nir::program::ExprKind::Let { binding, .. } = &expr.kind {
                assert!(locals.insert(*binding));
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(locals.len(), 2);
}

#[test]
fn generator_match_controls_accumulation_and_loop_transfers() {
    let source = "enum Step {skip,wrap(bool),stop}
        pure comptime fn build(first:Step)-><x:bool|-bool> {
            var step=first;
            var code:<x:bool|-bool> = '(x);
            while true {
                step match {
                    .skip => {step=.wrap(true);continue;},
                    .wrap(invert) => {
                        const previous=code;
                        code=if invert {'(not ~<previous>)} else {previous};
                        step=.stop;
                    },
                    .stop => {break;},
                };
            }
            code
        }
        fn run(value:bool)->bool {
            ~[x=value]<build(.stop)> and not ~[x=value]<build(.skip)>
                and ~[x=value]<build(.wrap(false))>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_terminal_match_propagates_returns_and_checks_fallthrough() {
    let source="enum Choice {flag(bool),letter(.char'builtin)}
        pure comptime fn build(choice:Choice)-><|-bool> {
            choice match {
                .flag(value) => {return '(value);},
                .letter(value) => {return '(value == 'a');},
            }
        }
        fn run()->bool {~<build(.flag(true))> and ~<build(.letter('a'))> and not ~<build(.letter('b'))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    for (source, anchor) in [
        (
            "pure comptime fn bad(flag:bool)-><|-bool> {flag match {true=>{return '(true);},false=>{},}}",
            "without returning",
        ),
        (
            "pure comptime fn bad(flag:bool)-><|-bool> {flag match {true=>{return '(true);},false=>{return '( 'x');},}}",
            "type",
        ),
        (
            "pure comptime fn bad(flag:bool)-><|-bool> {flag match {true=>{return '(true);},}}",
            "non-exhaustive",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{failure}");
    }
}

#[test]
fn generator_let_else_exports_success_and_returns_on_failure() {
    let source = "enum Input {none,some(bool)}
        pure comptime fn build(input:Input)-><x:bool|-bool> {
            let .some(flag):Input = input else {return '(x);};
            '(x and flag)
        }
        pure comptime fn mutable(input:Input)-><|-bool> {
            var .some(flag)=input else {return '(false);};
            flag=not flag;
            '(flag)
        }
        fn run(value:bool)->bool {
            ~[x=value]<build(.none)> and ~[x=value]<build(.some(true))>
                and not ~[x=value]<build(.some(false))> and ~<mutable(.some(false))>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_let_else_transfers_to_construction_loops() {
    let source="enum Input {none,some(bool)}
        pure comptime fn build(first:Input)-><x:bool|-bool> {
            var input=first;
            var code:<x:bool|-bool> = '(x);
            while true {
                let .some(flag)=input else {input=.some(true);continue;};
                code='(not ~<code>);
                let false=flag else {break;};
                return code;
            }
            code
        }
        fn run(value:bool)->bool {not ~[x=value]<build(.none)> and not ~[x=value]<build(.some(false))>}";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_let_else_checks_failure_even_when_unused() {
    for (source, anchor) in [
        (
            "enum E {a(bool),b} pure comptime fn bad(e:E)-><|-bool> {let .a(x)=e else {}; '(x)}",
            "must diverge",
        ),
        (
            "enum E {a(bool),b} pure comptime fn bad(e:E)-><|-bool> {let .a(x)=e else {return '( 'x');}; '(x)}",
            "type",
        ),
        (
            "fn runtime()->bool {true} enum E {a(bool),b} pure comptime fn bad(e:E)-><|-bool> {let .a(x)=e else {let y=runtime();return '(y);}; '(x)}",
            "execution phase",
        ),
        (
            "enum E {a(bool),b} pure comptime fn bad(e:E)-><|-bool> {let .a(x)=e else {return '(false);};x=false;'(x)}",
            "immutable",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
}

#[test]
fn generator_let_else_keeps_guard_candidates_and_outer_bindings_separate() {
    let source="pure comptime fn build(pair:(bool,bool))-><|-bool> {
            let outer=true;
            let (chosen,false) or (true,chosen) if chosen = pair else {return '(outer);};
            '(chosen)
        }
        fn run()->bool {~<build((true,true))> and ~<build((false,false))> and ~<build((false,true))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    for (source, anchor) in [
        (
            "pure comptime fn bad(input:bool)-><|-bool> {let true:.char'builtin=input else {return '(false);};'(true)}",
            "type",
        ),
        (
            "pure comptime fn bad(input:bool)-><x:bool|-bool> {let value if x=input else {return '(false);};'(value)}",
            "runtime context",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{failure}");
    }
}

#[test]
fn generator_let_else_checks_divergence_without_executing_unselected_else() {
    let source = "pure comptime fn build(input:bool)-><|-bool> {
            let true=input else {while true {}};
            '(true)
        } fn run()->bool {~<build(true)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let failure = support::compile_with_protocols(
        &source.replace("build(true)", "build(false)"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("budget"), "{failure}");
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_code_generators_specialize_contexts_results_and_nested_calls() {
    let source = "pure comptime fn identity(code:<x:T |- T>)-><x:T |- T> where T {code}
        pure comptime fn closed(code:< |- T>)->< |- T> where T {code}
        pure comptime fn wrap(code:<x:T |- T>)-><x:T |- T> where T {identity<T>(code)}
        fn run(value:bool)->bool {
            const first=wrap<bool>('[x=value](value));const second=closed<.char'builtin>('( 'a'));
            ~<first> and ~<second> == 'a'
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generic_code_generators_check_definitions_and_type_requirements() {
    for source in [
        "pure comptime fn bad()-><x:T |- T> where T {'(not x)}",
        "pure comptime fn code(input:< |- T>)->< |- T> where T, requires T == bool {input} fn run()->bool {const code=code<.char'builtin>('( 'a'));true}",
        "trait Allowed {} impl Allowed for bool {} pure comptime fn code(input:< |- T>)->< |- T> where T :- Allowed {input} fn run()->bool {const code=code<.char'builtin>('( 'a'));true}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
    let source = "trait Allowed {} impl Allowed for bool {} pure comptime fn code(input:< |- T>)->< |- T> where T :- Allowed {input} fn run()->bool {const code=code<bool>('(true));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_code_generator_defaults_keep_declared_type_identity() {
    let source = "pure comptime fn code(input:< |- T>)->< |- T> where .T:.Type'builtin=bool {input} fn run()->bool {const first=code('(true));const second=code<.T=.char'builtin>('( 'a'));~<first> and ~<second> == 'a'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_code_generators_infer_types_without_erasing_context_evidence() {
    let source="pure comptime fn forward(code:<x:T |- T>)-><x:T |- T> where T {code}
        pure comptime fn compose(code:<x:T |- T>)-><x:T |- T> where T {let saved=forward(code);saved}
        fn run(flag:bool)->bool {const code=compose('[input=flag](flag));let flag=false;~<code>}";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
    assert_eq!(
        run(source, "run", vec![Value::Bool(false)]),
        Value::Bool(false)
    );
    let invalid = source.replace("'[input=flag](flag)", "'(flag)");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generic_generator_result_context_infers_otherwise_unconstrained_types() {
    let source = "pure comptime fn build()-><x:T |- T> where T {'(x)}
        fn run(flag:bool)->bool {const code:<input:bool |- bool> = build();~[input=flag]<code>}";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generic_generator_data_and_instance_scopes_stay_independent() {
    let source="pure comptime fn lift_value(value:T)->< |- T> where T {'(value)}
        pure comptime fn wrap(value:T)->< |- T> where T {lift_value(value)}
        fn run()->bool {const first=wrap(true);const second=wrap('a');const third=wrap(false);~<first> and ~<second> == 'a' and not ~<third>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_generator_context_only_parameters_participate_in_inference() {
    let source = "pure comptime fn discard(code:<x:T |- bool>)-><x:T |- bool> where T {code}
        fn run(value:.char'builtin)->bool {const code=discard('[x=value](value == 'a'));~<code>}";
    assert_eq!(
        run(source, "run", vec![Value::Char('a')]),
        Value::Bool(true)
    );
}

#[test]
fn generic_generator_type_constraints_remain_available_in_quoted_bodies() {
    let source = "trait Read {fn read(self)->bool;} impl Read for bool {fn read(self)->bool {self}}
        pure comptime fn build()-><x:T |- bool> where T :- Read {'(x.read())}
        fn run(value:bool)->bool {const code=build<bool>();~[x=value]<code>}";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generic_generator_type_default_dependencies_follow_inferred_arguments() {
    let source="pure comptime fn choose(first:< |- T>,second:< |- U>)->< |- U> where T,.U:.Type'builtin=T {second}
        fn run()->bool {const code=choose('(false),'(true));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("'(true)", "'( 'a')");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generic_generators_preserve_runtime_only_interfaces_and_reject_unresolved_types() {
    for source in [
        "pure comptime fn build()-><x:T |- bool> where T {'(true)} fn run()->bool {const code=build<.Type'builtin>();false}",
        "pure comptime fn build()-><x:T |- bool> where T {'(true)} fn run()->bool {const code=build();false}",
        "pure comptime fn build(value:T)->< |- bool> where T {'(true)} fn run(flag:bool)->bool {const code=build(flag);false}",
        "pure comptime fn build()->< |- bool> where .T:.Type'builtin=T {'(true)}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generic_generator_result_context_guides_literal_code_before_defaulting() {
    let source = "pure comptime fn identity(code:< |- T>)->< |- T> where T {code}
        fn run()->bool {const code:< |- .u8'builtin> = identity('(17));~<code> == 17}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_code_generator_instantiation_keeps_normal_form_heads_opaque() {
    let source = "typealias Type=.Type'builtin;pure comptime fn Wrap<T> -> Type {T}
        pure comptime fn forward(code:<x:T |- T>)-><x:T |- T> where T {code}
        fn run(value:Wrap<bool>)->Wrap<bool> {const code=forward('[x=value](value));~<code>}";
    let package = support::compile(source);
    let function = package
        .functions
        .iter()
        .find(|function| function.name == "run")
        .unwrap();
    assert!(matches!(function.body.ty, nir::Term::NormalForm { .. }));
    let invalid = source.replace("->Wrap<bool> {const", "->bool {const");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn code_generator_defaults_observe_prior_data_and_named_overrides() {
    let source="pure comptime fn build(value:bool,.invert:bool=value,.answer:bool=not invert)->< |- bool> {'(answer)}
        fn run()->bool {const first=build(false);const second=build(false,.invert=true);const third=build(true,.answer=true);~<first> and not ~<second> and ~<third>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn code_generator_defaults_can_reuse_prior_code_and_preserve_connections() {
    let source="pure comptime fn build(first:<x:T |- T>,.second:<x:T |- T> = first)-><x:T |- T> where T {second}
        fn run(flag:bool)->bool {const code=build('[x=flag](flag));let flag=false;~<code>}";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn code_generator_default_quotes_lift_prior_data_after_explicit_supply() {
    let source="pure comptime fn build(value:bool,.code:< |- bool> = '(value))->< |- bool> {code}
        fn run()->bool {const first=build(true);const second=build(false,.code='(true));~<first> and ~<second>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn code_generator_defaults_are_checked_even_when_overridden_or_unused() {
    for source in [
        "pure comptime fn build(.value:bool='a')->< |- bool> {'(value)}",
        "pure comptime fn build(.value:bool=later,later:bool)->< |- bool> {'(value)}",
        "pure comptime fn build(.value:bool=value)->< |- bool> {'(value)}",
        "pure comptime fn build(.code:< |- bool> = '( 'a'))->< |- bool> {code} fn run()->bool {const code=build(.code='(true));~<code>}",
        "fn runtime()->bool {true} pure comptime fn build(.value:bool=runtime())->< |- bool> {'(value)}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn code_generator_named_routing_rejects_duplicates_unknowns_and_missing_anchors() {
    let prefix =
        "pure comptime fn build(value:bool,.other:bool=false)->< |- bool> {'(value or other)} ";
    for call in [
        "build()",
        "build(true,false)",
        "build(true,.missing=false)",
        "build(true,.other=false,.other=true)",
    ] {
        let source = format!("{prefix} fn run()->bool {{const code={call};~<code>}}");
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn code_generator_explicit_override_skips_divergent_comptime_default() {
    let source = "pure comptime fn forever()->bool {while true {} false}
        pure comptime fn build(.value:bool=forever())->< |- bool> {'(value)}
        fn run()->bool {const code=build(.value=true);~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert!(
        support::compile_with_protocols(
            &source.replace("build(.value=true)", "build()"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn code_generator_named_arguments_evaluate_in_source_order_once() {
    let source="pure comptime fn choose(.first:< |- bool> = '(false),.second:< |- bool> = '(false))->< |- bool> {first}
        pure comptime fn build()->< |- bool> {
            var state=false;
            let selected=choose(.second=do {state=true;'(state)},.first=do {state=not state;'(state)});
            selected
        }
        fn run()->bool {const code=build();not ~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn code_generator_default_quote_declares_its_own_virtual_context() {
    let source =
        "pure comptime fn build(.code:<x:bool |- bool> = '(not x))-><x:bool |- bool> {code}
        fn run(value:bool)->bool {const code=build();~[x=value]<code>}";
    assert_eq!(
        run(source, "run", vec![Value::Bool(false)]),
        Value::Bool(true)
    );
}

#[test]
fn code_generator_default_virtual_names_do_not_escape_or_read_runtime_state() {
    for source in [
        "pure comptime fn bad(.code:<x:bool |- bool> = '(x),.data:bool=x)->< |- bool> {'(data)}",
        "pure comptime fn bad(.code:<x:bool |- bool> = if x {'(true)} else {'(false)})-><x:bool |- bool> {code}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generator_variadic_data_collects_heterogeneous_and_empty_packets() {
    let source="pure comptime fn pack(...items:T)->< |- T> where T {'(items)}
        fn run()->bool {const code=pack(true,'a');const empty=pack();let (flag,letter)=~<code>;let unused=~<empty>;flag and letter == 'a'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let supplied = source.replace("pack(true,'a')", "pack(.items=(true,'a'))");
    assert_eq!(run(&supplied, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_variadic_elements_observe_interleaved_code_arguments_in_source_order() {
    let source =
        "pure comptime fn consume(...items:(bool,bool),.unused:< |- bool> = '(false))->< |- bool> {
            let (first,second)=items;'(not first and second)
        }
        pure comptime fn build()->< |- bool> {
            var state=false;
            consume(state,.unused=do {state=true;'(false)},state)
        }
        fn run()->bool {const code=build();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_variadic_domains_remain_whole_packet_types() {
    for source in [
        "pure comptime fn build(...items:bool)->< |- bool> {'(true)} fn run()->bool {const code=build(true);~<code>}",
        "pure comptime fn build(...items:.Type'builtin)->< |- bool> {'(true)} fn run()->bool {const code=build(bool);~<code>}",
        "pure comptime fn build(...items:(bool,bool))->< |- bool> {'(true)} fn run()->bool {const code=build(true);~<code>}",
        "pure comptime fn build(...items:< |- bool>)->< |- bool> {items}",
        "pure comptime fn build(...items:(bool,bool))->< |- bool> {'(true)} fn run()->bool {const code=build(.items=(true,false),.items=(false,true));~<code>}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generator_variadic_collection_uses_the_registered_tuple_constructor() {
    let source="typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        pure comptime fn build(...items:Tuple<bool,.char'builtin>)->< |- bool> {let (flag,letter)=items;'(flag and letter == 'a')}
        fn run()->bool {const code=build(true,'a');~<code>}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_generator_completed_code_arguments_guide_earlier_literals() {
    let source = "pure comptime fn select(first:< |- T>,second:< |- T>)->< |- T> where T {first}
        fn run()->bool {
            const typed:< |- .u8'builtin> = '(42);
            const generated=select('(17),typed);
            ~<generated> == 17
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_generator_completed_data_types_supply_code_literal_context() {
    let source = "pure comptime fn select(code:< |- T>,value:T)->< |- T> where T {code}
        pure comptime fn build(value:.u8'builtin)->< |- bool> {
            const code=select('(17),value);
            '(~<code> == 17)
        }
        fn run()->bool {const code=build(42);~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_generator_earlier_computed_code_guides_later_literal() {
    let source="pure comptime fn select(first:< |- T>,second:< |- T>)->< |- T> where T {second}
        fn run()->bool {const code=select('(do {let value:.u8'builtin=42;value}),'(17));~<code> == 17}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_generator_interface_evidence_does_not_bypass_staging_or_binding_identity() {
    for source in [
        "pure comptime fn build(code:< |- T>,value:T)->< |- T> where T {code} fn run(value:.u8'builtin)->bool {const code=build('(17),value);true}",
        "pure comptime fn build(first:<x:T |- T>,second:<x:T |- T>)-><x:T |- T> where T {first} fn run(value:bool)->bool {const known='[x=value](value);const code=build('(value),known);~<code>}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generic_generator_computed_interface_guides_later_variadic_elements() {
    let source = "pure comptime fn collect(marker:< |- T>,...items:T)->< |- T> where T {'(items)}
        fn run()->bool {
            const code=collect('(do {let byte:.u8'builtin=1;(byte,byte)}),2,3);
            let (first,second)=~<code>;first == 2 and second == 3
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_literal_spreads_keep_destination_contexts_and_defaults() {
    let source="pure comptime fn build(first:.u8'builtin,second:bool,.enabled:bool=second)->< |- bool> {'(first == 17 and enabled)}
        fn run()->bool {const code=build(...(17,true));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_literal_spreads_join_variadic_collection_in_source_order() {
    let source="pure comptime fn build(...items:(bool,.char'builtin,bool),.unused:< |- bool> = '(false))->< |- bool> {
            let (first,letter,last)=items;'(not first and letter == 'a' and last)
        }
        pure comptime fn generate()->< |- bool> {
            var state=false;
            build(...(state,'a'),.unused=do {state=true;'(false)},state)
        }
        fn run()->bool {const code=generate();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_literal_spreads_do_not_silently_discard_elements() {
    let prefix =
        "pure comptime fn build(first:bool,second:bool)->< |- bool> {'(first and second)} ";
    for call in [
        "build(...(true,'a'))",
        "build(...(true,true),false)",
        "build(...(true,true),.unknown=true)",
    ] {
        let source = format!("{prefix} fn run()->bool {{const code={call};~<code>}}");
        assert!(support::compile_with_protocols(&source, &Default::default()).is_err());
    }
}

#[test]
fn generator_spreads_completed_and_computed_packets() {
    let source="pure comptime fn packet()->(bool,.char'builtin) {(true,'a')}
        pure comptime fn select(flag:bool,letter:.char'builtin)->< |- bool> {'(flag and letter == 'a')}
        pure comptime fn build()->< |- bool> {let pair=packet();select(...pair)}
        fn run()->bool {const first=select(...packet());const second=build();~<first> and ~<second>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_spread_packet_reads_state_at_its_original_source_position() {
    let source="pure comptime fn select(...items:(bool,bool),.unused:< |- bool> = '(false))->< |- bool> {let (a,b)=items;'(a and b)}
        pure comptime fn build()->< |- bool> {
            var state=false;
            select(.unused=do {state=true;'(false)},...do {(state,state)})
        }
        fn run()->bool {const code=build();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_packet_spreads_preserve_generic_whole_collection_inference() {
    let source="pure comptime fn packet()->(bool,.char'builtin) {(true,'a')}
        pure comptime fn pack(...items:T)->< |- T> where T {'(items)}
        fn run()->bool {const code=pack(...packet(),false);let (a,b,c)=~<code>;a and b == 'a' and not c}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_spread_packet_gets_element_contexts_from_destination_parameters() {
    let source="pure comptime fn select(first:.u8'builtin,second:.u8'builtin)->< |- bool> {'(first == 17 and second == 42)}
        fn run()->bool {const code=select(...do {(17,42)});~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_empty_spread_packets_still_execute_checks() {
    let prefix = "pure comptime fn collect(...items:T)->T where T {items}
        pure comptime fn select(.flag:bool=true)->< |- bool> {'(flag)} ";
    let source = format!(
        "{prefix} pure comptime fn build()->< |- bool> {{let empty=collect();select(...do {{empty}})}} fn run()->bool {{const code=build();~<code>}}"
    );
    assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("...do {empty}", "...do {assumes false;empty}");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generator_spread_rejects_runtime_producers_and_missing_code_evidence() {
    for source in [
        "fn packet()->(bool,bool) {(true,false)} pure comptime fn select(a:bool,b:bool)->< |- bool> {'(a)} fn run()->bool {const code=select(...packet());~<code>}",
        "pure comptime fn packet()->(bool,bool) {(true,false)} pure comptime fn select(a:< |- bool>,b:bool)->< |- bool> {a} fn run()->bool {const code=select(...packet());~<code>}",
        "pure comptime fn packet()->(bool,bool) {(true,false)} pure comptime fn select(a:bool)->< |- bool> {'(a)} fn run()->bool {const code=select(...packet());~<code>}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generator_spread_generic_producer_uses_unique_destination_layout() {
    let source = "pure comptime fn identity(value:T)->T where T {value}
        pure comptime fn select(a:.u8'builtin,b:bool)->< |- bool> {'(a == 17 and b)}
        fn run()->bool {const code=select(...identity((17,true)));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_empty_generic_spread_has_a_real_evaluation_event() {
    let source = "pure comptime fn collect(...items:T)->T where T {items}
        pure comptime fn select(.flag:bool=true)->< |- bool> {'(flag)}
        fn run()->bool {const code=select(...collect());~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_multiple_type_preserving_packets_determine_layout() {
    let source="pure comptime fn identity(value:T)->T where T {value}
        pure comptime fn select(a:bool,b:bool,c:bool,d:bool)->< |- bool> {'(a and not b and not c and d)}
        fn run()->bool {const code=select(...identity((true,false)),...identity((false,true)));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let reordered = source.replace("identity((false,true))", "identity((true,false))");
    assert_eq!(run(&reordered, "run", vec![]), Value::Bool(false));
}

#[test]
fn generator_multiple_result_inferred_packets_use_required_type_equalities() {
    let source="pure comptime fn packet()->T where T, requires T == (bool,bool) {(true,false)}
        pure comptime fn select(a:bool,b:bool,c:bool,d:bool)->< |- bool> {'(a and not b and c and not d)}
        fn run()->bool {const code=select(...packet(),...packet());~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let reversed = source.replace("T == (bool,bool)", "(bool,bool) == T");
    assert_eq!(run(&reversed, "run", vec![]), Value::Bool(true));
    let chained = source
        .replace(
            "packet()->T where T, requires T == (bool,bool) {(true,false)}",
            "packet(value:U)->T where T,U, requires T == U and U == (bool,bool) {value}",
        )
        .replace("...packet()", "...packet((true,false))");
    assert_eq!(run(&chained, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_spread_does_not_choose_a_disjunctive_packet_width() {
    let source="pure comptime fn packet()->T where T, requires T == (bool,bool) or T == (bool,bool,bool) {packet<T>()}
        pure comptime fn select(a:bool,b:bool,c:bool,d:bool)->< |- bool> {'(true)}
        fn run()->bool {const code=select(...packet(),...packet());~<code>}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("width"), "{error}");
}

#[test]
fn generator_explicit_result_types_determine_packet_layout() {
    for (declaration, call) in [
        ("T", "packet<(bool,bool)>"),
        ("T", "packet<...((bool,bool),)>"),
        (".T:.Type'builtin=(bool,bool)", "packet<.T=(bool,bool)>"),
    ] {
        let source = format!(
            "pure comptime fn packet(value:T)->T where {declaration} {{value}}
            pure comptime fn select(a:bool,b:bool,c:bool,d:bool)->< |- bool> {{'(a and not b and c and not d)}}
            fn run()->bool {{const code=select(...{call}(do {{let pair=(true,false);pair}}),...{call}(do {{let pair=(true,false);pair}}));~<code>}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
        for invalid in [
            source.replace("(true,false)", "(true,'a')"),
            source.replace("(bool,bool)", "(bool,)"),
        ] {
            assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
        }
        if call.contains(".T=") {
            for invalid in [
                source.replace("<.T=", "<.unknown="),
                source.replace("<.T=(bool,bool)>", "<.T=(bool,bool),.T=(bool,bool)>"),
            ] {
                assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
            }
        }
    }
}

#[test]
fn generator_explicit_registered_tuple_results_determine_layout() {
    for ty in [
        "Tuple<bool,bool>",
        "Tuple<do {bool},bool>",
        "Tuple<...[bool,bool]>",
        "Tuple<.Es=[bool,bool]>",
        "Tuple<...fields>",
        "Tuple<.Es=fields>",
        "Tuple<...if true {[bool,bool]} else {fields}>",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            pure comptime fn List<T>->Type {{T}}
            pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
            const fields:List<Type> = [bool,bool];
            pure comptime fn packet(value:T)->T where T {{value}}
            pure comptime fn select(a:bool,b:bool,c:bool,d:bool)->< |- bool> {{'(a and not b and c and not d)}}
            fn run()->bool {{const code=select(
                ...packet<{ty}>(do {{let pair=(true,false);pair}}),
                ...packet<{ty}>(do {{let pair=(true,false);pair}}));~<code>}}"
        );
        let package = support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input::List"),
                tuple: Some("input::Tuple"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn generator_empty_packets_stay_between_interleaved_code_arguments() {
    let source="pure comptime fn collect(...items:T)->T where T {items}
        pure comptime fn select(.first:< |- bool> = '(true),.second:< |- bool> = '(true))->< |- bool> {first}
        pure comptime fn build()->< |- bool> {
            let empty=collect();var state=false;
            select(...do {assumes not state;empty},
                .first=do {state=true;'(true)},
                ...do {assumes state;empty},
                .second=do {state=false;'(true)},
                ...do {assumes not state;empty})
        }
        fn run()->bool {const code=build();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_spread_packets_retain_registered_tuple_identity() {
    let source="typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        pure comptime fn packet()->Tuple<bool,.char'builtin> {(true,'a')}
        pure comptime fn select(...values:Tuple<bool,.char'builtin>)->< |- bool> {let (a,b)=values;'(a and b == 'a')}
        fn run()->bool {const code=select(...packet());~<code>}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generator_property_collection_builds_anonymous_record_values() {
    let source="pure comptime fn pack(...properties:T)->< |- T> where T {'(properties)}
        fn run()->bool {const code=pack {enabled:true,letter:'a'};let config=~<code>;config.enabled and config.letter == 'a'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_property_collection_combines_with_positional_tuple_spreads() {
    let source="pure comptime fn pair()->(bool,bool) {(true,false)}
        pure comptime fn pack(...items:(bool,bool),...properties:T)->< |- T> where T {'(properties)}
        fn run()->bool {const code=pack {...pair(),enabled:true,letter:'a'};let config=~<code>;config.enabled and config.letter == 'a'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_properties_keep_interleaved_code_argument_evaluation_order() {
    let source =
        "pure comptime fn pack(code:< |- bool>,...properties:T)->< |- T> where T {'(properties)}
        pure comptime fn build()->< |- bool> {
            var state=false;
            const data=pack {before:state,do {state=true;'(true)},after:state};
            '(do {let data=~<data>;not data.before and data.after})
        }
        fn run()->bool {const code=build();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_property_duplicates_and_ambiguous_collectors_are_rejected() {
    for source in [
        "pure comptime fn pack(...properties:T)->< |- T> where T {'(properties)} fn run()->bool {const code=pack {flag:true,flag:false};true}",
        "pure comptime fn pack(.flag:bool=true)->< |- bool> {'(flag)} fn run()->bool {const code=pack {flag:false};~<code>}",
        "pure comptime fn pack(...first:T,...second:U)->< |- T> where T,U {'(first)} fn run()->bool {const code=pack {flag:true};true}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generator_property_collector_uses_explicit_type_arguments_without_replaying_them() {
    let source="pure comptime fn schema(...fields:T)->T where T {fields}
        const SAMPLE=schema {enabled:true};typealias Options=SAMPLE'type;
        pure comptime fn pack(...first:A,...props:B)->< |- B> where A,B {'(props)}
        fn run()->bool {const code=pack<(bool,bool),Options> {true,false,enabled:true};let data=~<code>;data.enabled}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_property_fields_receive_the_declared_record_schema() {
    let source="pure comptime fn schema(...fields:T)->T where T {fields}
        pure comptime fn number()->.u8'builtin {17}
        const SAMPLE=schema {size:number(),enabled:true};typealias Options=SAMPLE'type;
        pure comptime fn pack(...props:Options)->< |- Options> {'(props)}
        fn run()->bool {const code=pack {size:17,enabled:true};let data=~<code>;data.size == 17 and data.enabled}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_property_calls_and_named_whole_records_have_equivalent_contents() {
    let source = "pure comptime fn schema(...fields:T)->T where T {fields}
        pure comptime fn pack(...properties:T)->< |- T> where T {'(properties)}
        pure comptime fn build()->< |- bool> {
            let config=schema {enabled:true};
            const whole=pack(.properties=config);
            '(do {let value=~<whole>;value.enabled})
        }
        fn run()->bool {const code=build();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_property_values_cannot_read_runtime_state() {
    let source = "pure comptime fn pack(...properties:T)->< |- T> where T {'(properties)}
        fn run(flag:bool)->bool {const code=pack {enabled:flag};true}";
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn generator_property_calls_cannot_escape_into_runtime_call_elaboration() {
    for definition in [
        "pure comptime fn pack(...properties:T)->< |- T> where T {'(properties)}",
        "pure comptime fn pack(...first:A,...properties:B)->< |- B> where A,B {'(properties)}",
    ] {
        let source = format!("{definition} fn run() {{let value=pack {{enabled:true}};}}");
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("comptime code position"), "{error}");
    }
}

#[test]
fn local_data_constants_evaluate_and_feed_runtime_values_and_quotes() {
    let source = "pure comptime fn invert(value:bool)->bool {not value}
        fn run()->bool {const first=invert(false);const second=not first;
            const code='(first and not second);~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_type_constants_and_aggregate_patterns_are_completed_before_runtime() {
    let source = "fn run()->bool {const T=bool;const (flag,letter):(T,.char'builtin)=(true,'a');
            let value:T=flag;value and letter == 'a'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_data_constants_respect_lexical_shadowing_and_closure_capture() {
    let source = "fn run()->bool {const flag=true;let f=|| {const other=not flag;not other};
            let inner=do {const flag=false;not flag};flag and inner and f()}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_constants_cannot_read_runtime_parameters_locals_or_call_runtime_functions() {
    for source in [
        "fn run(value:bool)->bool {const saved=value;saved}",
        "fn run()->bool {var value=true;const saved=value;saved}",
        "fn make()->bool {true} fn run()->bool {const saved=make();saved}",
        "fn run()->bool {const flag=true;flag=false;flag}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn local_data_constants_and_code_alias_blocks_keep_distinct_result_kinds() {
    let source = "fn run()->bool {const code=do {const first='(true);const second=first;second};
        const flag=do {const inner=true;not not inner};
        const early=do {return '(true);};flag and ~<code> and ~<early>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_constants_inside_generated_runtime_code_keep_construction_stage() {
    let source = "pure comptime fn build(flag:bool)->< |- bool> {
            '(do {const saved=flag;saved})
        }
        fn run()->bool {const code=build(true);~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_constants_in_comptime_functions_bind_at_each_invocation() {
    let source = "pure comptime fn invert(value:bool)->bool {const result=not value;result}
        fn run()->bool {const a=invert(false);const b=invert(true);a and not b}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nested_local_constants_use_their_current_comptime_block_bindings() {
    let source = "fn run()->bool {const value=do {let a=true;const b=not a;not b};value}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_type_constants_preserve_generic_parameters_without_unfolding_heads() {
    let source = "fn identity(value:T)->T where T {const U=T;let result:U=value;result}
        fn run()->bool {identity(true)}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_type_constants_keep_normal_form_heads_opaque() {
    let source = "pure comptime fn Identity<T>->.Type'builtin {T}
        fn bad(value:T)->T where T {const U=Identity<T>;let result:U=value;result}";
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn local_constants_preserve_runtime_function_identity_without_invoking_it() {
    let source = "fn invert(value:bool)->bool {not value}
        fn run()->bool {const action=invert;action(false)}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_constant_function_aggregates_destructure_and_keep_generic_instances() {
    let source="fn identity(value:T)->T where T {value}
        fn invert(value:bool)->bool {not value}
        fn run()->bool {const (first,second)=(identity<bool>,invert);first(true) and second(false)}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_constants_can_select_runtime_functions_during_comptime() {
    let source = "fn yes()->bool {true} fn no()->bool {false}
        fn run()->bool {const action=if true {yes} else {no};action()}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_constant_callbacks_survive_nominal_storage_and_runtime_closures() {
    let source = "struct Callback {apply:fn(bool)->bool}
        fn invert(value:bool)->bool {not value}
        fn run()->bool {const callback=Callback {apply:invert};
            let run_callback=|| callback.apply(false);run_callback()}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_callable_constants_cannot_erase_phase_or_code_evidence() {
    for source in [
        "pure comptime fn invert(value:bool)->bool {not value} fn run()->bool {const action=invert;action(false)}",
        "quote fn quoted()->bool {true} fn run()->fn()->bool {const action=quoted;action}",
        "fn run(){const pointer=do {var value=false;value.ref};}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn local_constant_closures_reconstruct_immutable_environments_hygienically() {
    let source = "fn run()->bool {const action=do {let flag=true;|value:bool| value and flag};
            const first=action(true);const second=action(false);first and not second}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_constant_closure_graphs_preserve_shared_members_inside_each_copy() {
    let source="fn run()->bool {const actions=do {let flag=true;let action=|value:bool| value and flag;(action,action)};
            const (first,second)=actions;const (third,fourth)=actions;
            const result=first(true) and not second(false) and third(true) and not fourth(false);result}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_constant_distinct_closures_keep_independent_owned_captures() {
    let source = "fn run()->bool {const actions=do {var flag=false;
        let set=|| {flag=true;};let get=|| flag;(set,get)};
        const result=do {let (set,get)=actions;set();get()};result}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(false));
}

#[test]
fn local_constant_repeated_closure_references_share_owned_capture_updates() {
    let source = "fn run()->bool {const actions=do {var flag=false;
        let toggle=|| {flag=not flag;flag};(toggle,toggle)};
        const result=do {let (first,second)=actions;first() and not second()};result}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_constant_mutable_capture_snapshots_preserve_current_value_and_copy_independence() {
    let source = "fn run()->bool {const actions=do {var flag=false;
        let toggle=|| {flag=not flag;flag};toggle();(toggle,toggle)};
        const result=do {let (first,second)=actions;let (third,fourth)=actions;
            not first() and second() and not third() and fourth()};result}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_constant_closures_still_reject_external_pointer_capture() {
    let source =
        "fn run(){const action=do {var flag=false;let pointer=flag.ref;|| {pointer.* = true;}};}";
    let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(failure.contains("pointer"), "{failure}");
}

#[test]
fn local_constant_nested_closure_graph_retains_shared_mutable_child_identity() {
    for packet in ["(toggle,indirect)", "(indirect,toggle)"] {
        let source = format!(
            "fn run()->bool {{const actions=do {{var flag=false;
            let toggle=|| {{flag=not flag;flag}};let indirect=|| toggle();{packet}}};
            const result=do {{let (first,second)=actions;first() and not second()}};result}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn generator_preconditions_check_completed_inputs_before_construction() {
    let source = "pure comptime fn build(allowed:bool,.flag:bool=allowed)-><|-bool> where requires allowed and flag {'(flag)}
        pure comptime fn forward(allowed:bool)-><|-bool> {build(allowed)}
        fn run()->bool {~<forward(true)>}";
    for predicate in [
        "allowed and flag",
        "(if allowed {flag} else {false})",
        "do {let ready=allowed and flag;ready}",
    ] {
        let source = source.replace("allowed and flag", predicate);
        let source = source.as_str();
        assert_eq!(run(source, "run", vec![]), Value::Bool(true));
        for rejected in [
            source.replace("forward(true)", "forward(false)"),
            source.replace("build(allowed)", "build(allowed,.flag=false)"),
            source
                .replace("'(flag)", "while true {}")
                .replace("forward(true)", "forward(false)"),
        ] {
            let error =
                support::compile_with_protocols(&rejected, &Default::default()).unwrap_err();
            assert!(
                error.contains("code generator precondition rejected"),
                "{error}"
            );
        }
        assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn generator_preconditions_reject_runtime_calls_even_in_unused_definitions() {
    let source = "fn runtime(value:bool)->bool {value}
        pure comptime fn build(allowed:bool)-><|-bool> where requires runtime(allowed) {'(true)}
        fn run()->bool {true}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("phase")
            || error.contains("comptime")
            || error.contains("effect-free program predicate"),
        "{error}"
    );
}

#[test]
fn generator_preconditions_cannot_observe_quoted_runtime_context() {
    let source =
        "pure comptime fn build(allowed:bool)-><x:bool|-bool> where requires allowed and x {'(x)}
        fn run()->bool {true}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("Unresolved identifier `x`"), "{error}");
}

#[test]
fn generator_postconditions_guard_tail_and_early_code_returns() {
    for body in [
        "'(allowed)",
        "return '(allowed);",
        "if allowed {return '(true);} else {return '(false);}",
    ] {
        let source = format!("pure comptime fn build(allowed:bool,.copy:bool=allowed)-><|-bool> where ensures copy and allowed == allowed'old {{{body}}}
            fn run()->bool {{~<build(true)>}}");
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
        for bad in [
            source.replace("build(true)", "build(false)"),
            source.replace("build(true)", "build(true,.copy=false)"),
        ] {
            let error = support::compile_with_protocols(&bad, &Default::default()).unwrap_err();
            assert!(
                error.contains("code generator postcondition rejected"),
                "{error}"
            );
        }
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn generator_postconditions_are_checked_without_running_unused_bodies() {
    let source =
        "pure comptime fn build(allowed:bool)-><|-bool> where ensures not allowed {while true {}}
        fn run()->bool {true}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let error = support::compile_with_protocols(
        &source.replace("fn run()->bool {true}", "fn run()->bool {~<build(true)>}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("budget"), "{error}");
    assert!(!error.contains("postcondition rejected"), "{error}");
    let bad = source.replace("ensures not allowed", "ensures 'x'");
    assert!(support::compile_with_protocols(&bad, &Default::default()).is_err());
}

#[test]
fn generator_contracts_observe_completed_record_inputs() {
    for contract in [
        "requires input.flag",
        "ensures input.flag",
        "ensures input.flag == input'old.flag and input.flag",
    ] {
        let source = format!(
            "struct Input {{flag:bool}}
            pure comptime fn build(input:Input)-><|-bool> where {contract} {{'(true)}}
            fn run()->bool {{~<build(Input {{flag:true}})>}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true), "{contract}");
        let error = support::compile_with_protocols(
            &source.replace("flag:true", "flag:false"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.contains("condition rejected"), "{contract}: {error}");
    }
}

#[test]
fn generator_contracts_keep_enum_identity_and_default_record_snapshots() {
    let source = "typealias str=.str'builtin;enum State {ready,closed}
        struct Input {flag:bool}
        pure comptime fn build(state:State,input:Input,.copy:Input=input)-><|-bool>
            where requires state'tag_name == \"ready\", ensures copy.flag and copy'old.flag {'(true)}
        fn run()->bool {~<build(State.ready,Input {flag:true})>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    for (bad, expected) in [
        (
            source.replace("build(State.ready", "build(State.closed"),
            "precondition rejected",
        ),
        (
            source.replace("flag:true", "flag:false"),
            "postcondition rejected",
        ),
        (
            source.replace(
                "Input {flag:true})",
                "Input {flag:true},.copy=Input {flag:false})",
            ),
            "postcondition rejected",
        ),
    ] {
        let error = support::compile_with_protocols(&bad, &Default::default()).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn generic_generator_contracts_follow_variadic_collection_and_packet_forwarding() {
    let prefix = "pure comptime fn build(...items:T,.copy:T=items)-><|-bool> where T,
        requires T == (bool,bool),
        requires do {let (first,second)=items;first and second},
        ensures do {let (first,second)=copy'old;first and second} {'(true)}
        pure comptime fn forward(items:(bool,bool))-><|-bool> {build(...items)}";
    for call in [
        "build(true,true)",
        "build(...(true,true))",
        "build(.items=(true,true))",
        "forward((true,true))",
    ] {
        let source = format!("{prefix} fn run()->bool {{~<{call}>}}");
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true), "{call}");
        let error = support::compile_with_protocols(
            &source.replace("true,true", "true,false"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.contains("precondition rejected"), "{call}: {error}");
    }
    let source = format!("{prefix} fn run()->bool {{~<build(true,true,.copy=(true,false))>}}");
    let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(error.contains("postcondition rejected"), "{error}");
}

#[test]
fn generator_contracts_run_after_interleaved_argument_collection() {
    let source =
        "pure comptime fn collect(...items:(bool,bool),.code:<|-bool> = '(false))-><|-bool>
        where requires do {let (first,second)=items;not first and second} {code}
        pure comptime fn build()-><|-bool> {
            var state=false;
            collect(state,.code=do {state=true;'(true)},state)
        }
        fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_contracts_match_enum_payloads_and_guards() {
    for contract in [
        "requires (input match {.ready(flag) if flag=>true,.ready(_)=>false,.closed=>false})",
        "ensures (input'old match {.ready(flag)=>flag,.closed=>false})",
    ] {
        let source = format!(
            "enum State {{ready(bool),closed}}
            pure comptime fn build(input:State)-><|-bool> where {contract} {{'(true)}}
            fn run()->bool {{~<build(State.ready(true))>}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true), "{contract}");
        for bad in [
            source.replace("State.ready(true)", "State.ready(false)"),
            source.replace("State.ready(true)", "State.closed"),
        ] {
            let error = support::compile_with_protocols(&bad, &Default::default()).unwrap_err();
            assert!(error.contains("condition rejected"), "{contract}: {error}");
        }
    }
}

#[test]
fn generator_contract_or_patterns_preserve_payload_bindings() {
    for contract in [
        "requires (input match {.left(flag) or .right(flag) if flag=>true,_=>false})",
        "ensures (input'old match {.left(flag) or .right(flag)=>flag,.closed=>false})",
    ] {
        let prefix = format!(
            "enum State {{left(bool),right(bool),closed}}
            pure comptime fn build(input:State)-><|-bool> where {contract} {{'(true)}}"
        );
        for value in ["State.left(true)", "State.right(true)"] {
            let source = format!("{prefix} fn run()->bool {{~<build({value})>}}");
            assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
        }
        for value in ["State.left(false)", "State.right(false)", "State.closed"] {
            let source = format!("{prefix} fn run()->bool {{~<build({value})>}}");
            let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
            assert!(error.contains("condition rejected"), "{value}: {error}");
        }
    }
}

#[test]
fn generator_contract_pattern_computations_keep_candidate_bindings() {
    for contract in [
        "requires do {let flag and (not flag,true) is (inverted,enabled)=input;inverted and enabled}",
        "ensures do {let flag and (not flag,true) is (inverted,enabled)=input'old;inverted and enabled}",
        "requires do {let flag and (not flag) is inverted=input;inverted}",
        "ensures do {let flag and (not flag) is inverted=input'old;inverted}",
        "requires (input match {flag and (not flag) is inverted=>inverted})",
        "requires (true match {_ and (not input) is inverted=>inverted})",
        "ensures (input'old match {flag and (not flag) is true=>true,_=>false})",
    ] {
        let source = format!(
            "pure comptime fn build(input:bool)-><|-bool> where {contract} {{'(true)}}
            fn run()->bool {{~<build(false)>}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
        let error = support::compile_with_protocols(
            &source.replace("build(false)", "build(true)"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.contains("condition rejected"), "{error}");
    }
    let source="fn observed(flag:bool)->bool {flag}
        pure comptime fn build(input:bool)-><|-bool> where requires (input match {flag and observed(flag) is result=>result}) {'(true)}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("effect-free program predicate"), "{error}");
}

#[test]
fn generator_contract_computed_tuple_patterns_cover_integer_inputs() {
    let source = "typealias u32=.u32'builtin; pure comptime fn build(input:u32)-><|-bool>
        where requires do {let value and (value > 0,true) is (positive,enabled)=input;positive and enabled}
        {'(true)}
        fn run()->bool {~<build(1)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let error = support::compile_with_protocols(
        &source.replace("build(1)", "build(0)"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("precondition rejected"), "{error}");
}

#[test]
fn generator_when_selects_code_and_checks_all_arms() {
    let prefix = "pure comptime fn build(first:bool,second:bool)-><|-bool> {
        when { first=>'(false),second=>'(true),else=>'(false) }
    }";
    for (first, second, expected) in [
        (true, true, false),
        (false, true, true),
        (false, false, false),
    ] {
        let source = format!("{prefix} fn run()->bool {{~<build({first},{second})>}}");
        assert_eq!(run(&source, "run", vec![]), Value::Bool(expected));
    }
    let invalid = prefix.replace("second=>'(true)", "second=>'(missing)");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(error.contains("missing"), "{error}");
    let invalid = "pure comptime fn build()-><|-bool> {when {true=>'(true)}}";
    assert!(support::compile_with_protocols(invalid, &Default::default()).is_err());
}

#[test]
fn generator_when_controls_returns_and_loops() {
    for body in [
        "when {false=>return '(false),true=>return '(true)}",
        "when {flag=>return '(true)} '(false)",
        "var done=false;while true {when {not done=>{done=true;continue;},else=>return '(true)}}",
        "var code='(false);when {flag=>{let chosen='(true);code=chosen;},else=>{}} code",
        "let code=when {flag=>'(true),else=>'(false)};code",
        "var code='(false);while true {when {flag=>{code='(true);break;},else=>break}} code",
    ] {
        let source = format!(
            "pure comptime fn build(flag:bool)-><|-bool> {{{body}}}
            fn run()->bool {{~<build(true)>}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true), "{body}");
    }
    let invalid = "pure comptime fn build(flag:bool)-><|-bool> {when {flag=>return '(true)}}";
    let error = support::compile_with_protocols(invalid, &Default::default()).unwrap_err();
    assert!(error.contains("without returning a code result"), "{error}");
}

#[test]
fn generator_when_skips_later_conditions_but_checks_unselected_bodies() {
    let source = "pure comptime fn stall()->bool {while true {} false}
        pure comptime fn build()-><|-bool> {
            when {true=>'(true),stall()=>'(false),else=>'(false)}
        }
        fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let source = "pure comptime fn build()-><|-bool> {
        when {true=>return '(true),else=>{let wrong:bool='x';}}
    }";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("elaboration failed"), "{error}");
    let source = "pure comptime fn build(flag:bool)-><|-bool> {
        when {flag=>return '(true)} '(false)
    }
    fn run()->bool {~<build(false)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(false));
}

#[test]
fn generator_when_rejects_invalid_arm_types_and_scopes() {
    for source in [
        "pure comptime fn build()-><|-bool> {when {true=>'(true),else=>'(())}}",
        "pure comptime fn build()-><|-bool> {when {()=>'(true),else=>'(false)}}",
        "pure comptime fn build()-><|-bool> {when {true=>break,else=>return '(true)}}",
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(error.contains("elaboration failed"), "{error}");
    }
    let source = "pure comptime fn build()-><|-bool> {
        when {true=>{let private_code='(true);},else=>{}} private_code
    }";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("Unresolved identifier"), "{error}");
}

#[test]
fn generator_inline_range_accumulates_code_in_order() {
    for (range, expected) in [
        ("0..3", false),
        ("0..4", true),
        ("3..3", true),
        ("4..2", true),
        ("2..=2", false),
    ] {
        let source = format!(
            "pure comptime fn build()-><|-bool> {{
            var code='(true);
            inline for index in {range} {{code='(not ~<code>);}}
            code
        }} fn run()->bool {{~<build()>}}"
        );
        assert_eq!(
            run(&source, "run", vec![]),
            Value::Bool(expected),
            "{range}"
        );
    }
}

#[test]
fn generator_inline_range_handles_filter_and_transfers() {
    for body in [
        "inline for index in 0..4 {when {index == 0=>continue,index == 2=>break,else=>{code='(true);}}}",
        "inline for index if index == 1 in 0..4 {code='(true);}",
        "inline for index in 0..4 {if index == 1 {return '(true);}}",
        "inline for:outer i in 0..2 {inline for j in 0..2 {code='(true);break outer;}}",
        "inline for (index and (index == 1) is selected) in 0..3 {if selected {code='(true);}}",
    ] {
        let source = format!(
            "pure comptime fn build()-><|-bool> {{
            var code='(false);{body} code
        }} fn run()->bool {{~<build()>}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true), "{body}");
    }
}

#[test]
fn generator_inline_ranges_check_empty_bodies_and_bound_expansion() {
    for body in [
        "inline for index in 0..0 {let wrong:bool=0;}",
        "inline for index if false in 0..3 {let wrong:bool=0;}",
        "inline for index in true..3 {}",
        "inline for 1 in 0..3 {}",
        "inline for index in 0..3 {index=1;}",
    ] {
        let source = format!("pure comptime fn unused()-><|-bool> {{{body} '(true)}}");
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("elaboration failed"), "{body}: {error}");
    }
    let source = "pure comptime fn build()-><|-bool> {
        inline for index in 0..100001 {} '(true)
    } fn run()->bool {~<build()>}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("budget exhausted"), "{error}");
}

#[test]
fn generator_inline_range_uses_completed_endpoint_parameters() {
    for (start, end, expected) in [(-2, 1, false), (1, 1, true), (3, 1, true)] {
        let source = format!(
            "typealias Integer=.Integer'builtin;
            pure comptime fn build(start:Integer,end:Integer)-><|-bool> {{
                var code='(true);
                inline for index in start..end {{code='(not ~<code>);}}
                code
            }}
            fn run()->bool {{~<build({start},{end})>}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::Bool(expected));
    }
}

#[test]
fn generator_empty_inline_range_does_not_evaluate_guard_or_body() {
    let source = "pure comptime fn forever()->bool {while true {} false}
        pure comptime fn build()-><|-bool> {
            inline for index if forever() in 0..0 {while true {}}
            '(true)
        }
        fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_inline_patterns_check_phase_even_without_iterations_or_invocation() {
    for range in ["0..0", "0..2"] {
        let source = format!(
            "fn runtime(flag:bool)->bool {{flag}}
            pure comptime fn unused()-><|-bool> {{
                inline for (index and runtime(index == 0) is observed) in {range} {{}}
                '(true)
            }}"
        );
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("wrong execution phase"), "{error}");
    }
}

#[test]
fn generator_inline_patterns_allow_comptime_calls_and_keep_bound_results() {
    let source = "pure comptime fn selected(flag:bool)->bool {flag}
        pure comptime fn build()-><|-bool> {
            var code='(false);
            inline for (index and selected(index == 1) is observed) in 0..3 {
                if observed {code='(true);}
            }
            code
        }
        fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let empty = "pure comptime fn forever(flag:bool)->bool {while true {} flag}
        pure comptime fn build()-><|-bool> {
            inline for (index and forever(index == 0) is observed) in 0..0 {}
            '(true)
        }
        fn run()->bool {~<build()>}";
    assert_eq!(run(empty, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_local_code_selection_cannot_hide_an_open_branch() {
    for selection in [
        "if flag {closed} else {open}",
        "when {flag=>closed,else=>open}",
        "flag match {true=>closed,false=>open}",
    ] {
        let source = format!(
            "pure comptime fn unused(flag:bool,closed:<|-bool>,open:<x:bool|-bool>)-><|-bool> {{
            let selected={selection};selected
        }}"
        );
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("code") || error.contains("context"),
            "{error}"
        );
    }
}

#[test]
fn generator_code_branches_preserve_compatible_context_renaming() {
    for selection in [
        "if flag {left} else {right}",
        "when {flag=>left,else=>right}",
        "flag match {true=>left,false=>right}",
    ] {
        for flag in [false, true] {
            let source=format!("pure comptime fn choose(flag:bool,left:<x:bool|-bool>,right:<y:bool|-bool>)-><out:bool|-bool> {{
                let selected={selection};selected
            }}
            fn run(value:bool)->bool {{
                const left:<x:bool|-bool> = '(x);
                const right:<y:bool|-bool> = '(not y);
                const selected=choose({flag},left,right);
                ~[out=value]<selected>
            }}");
            for value in [false, true] {
                assert_eq!(
                    run(&source, "run", vec![Value::Bool(value)]),
                    Value::Bool(if flag { value } else { !value }),
                    "{selection}, {flag}, {value}"
                );
            }
        }
    }
}

#[test]
fn generator_inline_conditionals_select_typed_code_values() {
    for selection in [
        "inline if flag {'(true)} else {'(false)}",
        "inline when {flag=>'(true),else=>'(false)}",
        "inline if flag is do {true=>'(true),false=>'(false)}",
    ] {
        for flag in [false, true] {
            let source = format!(
                "pure comptime fn build(flag:bool)-><|-bool> {{
                let code={selection};code
            }} fn run()->bool {{~<build({flag})>}}"
            );
            assert_eq!(
                run(&source, "run", vec![]),
                Value::Bool(flag),
                "{selection}"
            );
        }
    }
}

#[test]
fn generator_inline_conditionals_preserve_statement_transfers() {
    for body in [
        "inline if true {return '(true);}",
        "inline when {false=>return '(false),true=>return '(true)}",
        "inline if flag {return '(true);} '(false)",
        "inline when {flag=>return '(true)} '(false)",
        "inline if flag is do {true=>return '(true),false=>return '(false)}",
        "var code='(false);inline for i in 0..3 {inline if i==0 {continue;} inline when {i==2=>break} code='(true);} code",
    ] {
        let source = format!(
            "pure comptime fn build(flag:bool)-><|-bool> {{{body}}}
            fn run()->bool {{~<build(true)>}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true), "{body}");
    }
}

#[test]
fn generator_inline_conditionals_validate_unselected_code_without_running_it() {
    let source = "pure comptime fn forever()->bool {while true {} false}
        pure comptime fn build()-><|-bool> {
            inline if true {'(true)} else {
                inline when {forever()=>'(false),else=>'(true)}
            }
        }
        fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    for body in [
        "inline if true {'(true)} else {'(())}",
        "inline when {true=>'(true),else=>'(())}",
        "inline if true is do {true=>'(true),false=>'(())}",
        "inline if runtime() {'(true)} else {'(false)}",
    ] {
        let source = format!(
            "fn runtime()->bool {{true}}
            pure comptime fn unused()-><|-bool> {{{body}}}"
        );
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("elaboration failed"), "{body}: {error}");
    }
}

#[test]
fn generator_inline_selection_can_supply_a_code_argument() {
    let source = "pure comptime fn invert(code:<|-bool>)-><|-bool> {'(not ~<code>)}
        pure comptime fn build(flag:bool)-><|-bool> {
            invert(inline if flag {'(false)} else {'(true)})
        }
        fn run()->bool {~<build(true)> and not ~<build(false)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_nonempty_literal_ranges_can_supply_the_entire_return() {
    for range in [
        "0..1",
        "(-2)..0",
        "2..=2",
        "340282366920938463463374607431768211456..=340282366920938463463374607431768211456",
    ] {
        let source = format!(
            "pure comptime fn build()-><|-bool> {{
            inline for index in {range} {{return '(true);}}
        }} fn run()->bool {{~<build()>}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true), "{range}");
    }
    for body in [
        "inline for index in 0..0 {return '(true);}",
        "inline for index in 2..1 {return '(true);}",
        "inline for index in 2..2 {return '(true);}",
        "inline for index if false in 0..1 {return '(true);}",
        "inline for index in 0..1 {continue;}",
        "inline for index in 0..1 {break;}",
        "inline for index in 0..1 {if flag {return '(true);}}",
        "inline for index if flag in 0..1 {return '(true);}",
    ] {
        let source = format!("pure comptime fn unused(flag:bool)-><|-bool> {{{body}}}");
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("without returning a code result"),
            "{body}: {error}"
        );
    }
}

#[test]
fn generator_range_flow_does_not_invoke_endpoints_and_preserves_outer_exits() {
    let source = "typealias Integer=.Integer'builtin;
        pure comptime fn forever()->Integer {while true {} 1}
        pure comptime fn unused()-><|-bool> {
            inline for index in 0..forever() {return '(true);}
            '(false)
        }";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    let unknown = "typealias Integer=.Integer'builtin;
        pure comptime fn unused(end:Integer)-><|-bool> {
            inline for index in 0..end {return '(true);}
        }";
    let error = support::compile_with_protocols(unknown, &Default::default()).unwrap_err();
    assert!(error.contains("without returning a code result"), "{error}");
    let source = "pure comptime fn build()-><|-bool> {
        inline for:outer i in 0..1 {
            inline for j in 0..1 {break outer;}
            return '(false);
        }
        '(true)
    } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let source = "pure comptime fn build()-><|-bool> {
        inline for i if true in 0..1 {return '(true);}
    } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_tuple_enumeration_builds_code_from_completed_fields() {
    let source = "pure comptime fn build(values:(bool,bool,bool))-><|-bool> {
        var code='(true);
        inline for (index,element) in values'tuple_enumerate {
            if element {code='(not ~<code>);}
        }
        code
    }
    fn run()->bool {~<build((true,false,true))> and not ~<build((false,true,false))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_tuple_enumeration_preserves_each_element_type() {
    let source = "typealias Word=.u32'builtin;
        pure comptime fn build(values:(bool,Word))-><|-bool> {
            var code='(false);
            inline for (index,element) in values'tuple_enumerate {
                if element'type == Word {code='(true);}
            }
            code
        }
        fn run()->bool {~<build((false,9))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_tuple_enumeration_filters_and_transfers_control() {
    let source = "pure comptime fn build(values:(bool,bool,bool))-><|-bool> {
        var code='(false);
        inline for (index,element) if index != 0 in values'tuple_enumerate {
            if not element {continue;}
            code='(not ~<code>);
            break;
        }
        code
    } fn run()->bool {~<build((true,false,true))> and not ~<build((true,false,false))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let source = "pure comptime fn build(values:(bool,bool))-><|-bool> {
        inline for (index,element) in values'tuple_enumerate {return '(true);}
    } fn run()->bool {~<build((false,false))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let source = "pure comptime fn build(values:(bool,bool))-><|-bool> {
        inline for:outer i in 0..2 {
            inline for (index,element) in values'tuple_enumerate {break outer;}
            return '(false);
        }
        '(true)
    } fn run()->bool {~<build((false,false))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_tuple_enumeration_checks_filtered_and_unreachable_elements() {
    for body in [
        "inline for (index,element) if false in values'tuple_enumerate {let invalid=not element;}",
        "inline for (index,element) in values'tuple_enumerate {return '(true); let invalid=not element;}",
        "inline for (index,element) in values'tuple_enumerate {index=1;}",
        "inline for (index,element) in values'tuple_enumerate {element=element;}",
        "inline for element in values'tuple_enumerate {}",
        "inline for (index,element) if false in values'tuple_enumerate {runtime();}",
    ] {
        let source = format!(
            "typealias Word=.u32'builtin;
            fn runtime()->bool {{true}}
            pure comptime fn unused(values:(bool,Word))-><|-bool> {{{body} '(true)}}"
        );
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("elaboration failed"), "{body}: {error}");
    }
    for body in [
        "inline for (index,element) in values'tuple_enumerate {break;}",
        "inline for (index,element) in values'tuple_enumerate {continue;}",
        "inline for (index,element) if flag in values'tuple_enumerate {return '(true);}",
    ] {
        let source =
            format!("pure comptime fn unused(flag:bool,values:(bool,bool))-><|-bool> {{{body}}}");
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("without returning a code result"),
            "{body}: {error}"
        );
    }
}

#[test]
fn generator_bit_compound_writeback_preserves_snapshots_and_siblings() {
    let source = "struct State {bits:.u8'builtin,other:bool}
        pure comptime fn build()-><|-bool> {
            let state=State {bits:33,other:true};
            const original='(state.bits == 33);
            state.bits'bitvec[0..4]+=2;
            state.bits'bitvec[4..8]+=4;
            if state.bits == 99 and state.other {original} else {'(false)}
        } fn run()->bool {~<build()>}";
    assert_eq!(run_bit_generator(source), Value::Bool(true));
}

#[test]
fn generator_bit_compound_writeback_checks_bounds_and_phase() {
    let prefix = "struct State {bits:.u8'builtin,index:.u8'builtin}
        pure comptime fn build(index:.u8'builtin)-><|-bool> {
            let state=State {bits:0,index:index};
            state.bits'bitvec[state.index]+=1;
            '(state.bits == 2)
        }";
    let failure =
        compile_bit_generator(&format!("{prefix} fn run()->bool {{~<build(8)>}}")).unwrap_err();
    assert!(failure.contains("IndexOutOfBounds"), "{failure}");
    assert_eq!(
        run_bit_generator(&format!("{prefix} fn run()->bool {{~<build(1)>}}")),
        Value::Bool(true)
    );
    let source = "fn runtime()->.u8'builtin {0}
        struct State {bits:.u8'builtin}
        pure comptime fn unused()-><|-bool> {
            let state=State {bits:0};state.bits'bitvec[runtime()]+=1;'(true)
        }";
    let failure = compile_bit_generator(source).unwrap_err();
    assert!(failure.contains("wrong execution phase"), "{failure}");
}

fn compile_bit_generator(source: &str) -> Result<nir::program::Package, String> {
    let prelude = "typealias Bits4=.BitVec'builtin<4>;typealias Bits1=.BitVec'builtin<1>;
        trait Operation where Rhs {assoc Output:.Type'builtin;pure comptime fn add(self,rhs:Rhs)->Output;}
        impl Operation<Bits4> for Bits4 {assoc Output:.Type'builtin=Bits4;
            pure comptime fn add(self,rhs:Bits4)->Bits4 {self or rhs}}
        impl Operation<Bits1> for Bits1 {assoc Output:.Type'builtin=Bits1;
            pure comptime fn add(self,rhs:Bits1)->Bits1 {self or rhs}}";
    let mut selectors = luna::protocols::type_constructors::Selectors::default();
    selectors.arithmetic[elaboration::arithmetic::Operator::Add as usize] =
        Some("input::Operation");
    support::compile_with_source_bindings(
        &format!("{prelude} {source}"),
        &Default::default(),
        selectors,
    )
}

fn run_bit_generator(source: &str) -> Value {
    let package = compile_bit_generator(source).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    vm.invoke_instance(&graph.roots[0], vec![]).unwrap()
}

#[test]
fn generator_bit_compound_writeback_evaluates_selector_once_before_rhs() {
    let source = "struct State {bits:.u8'builtin,index:.u8'builtin}
        pure comptime fn build()-><|-bool> {
            let state=State {bits:0,index:0};
            state.bits'bitvec[do {state.index=if state.index == 0 {1} else {2};state.index}]+=do {
                state.bits=16;1
            };
            '(state.index == 1 and state.bits == 18)
        } fn run()->bool {~<build()>}";
    assert_eq!(run_bit_generator(source), Value::Bool(true));
}

#[test]
fn generator_bit_compound_writeback_handles_nested_slices() {
    let source = "struct State {bits:.u8'builtin}
        pure comptime fn build()-><|-bool> {
            let state=State {bits:160};
            state.bits'bitvec[0..8][0..4]+=3;
            '(state.bits == 163)
        } fn run()->bool {~<build()>}";
    assert_eq!(run_bit_generator(source), Value::Bool(true));
    let source = "pure comptime fn unused()-><|-bool> {
        let bits:.u8'builtin=0;bits'bitvec[0..4]+=1;'(true)
    }";
    let failure = compile_bit_generator(source).unwrap_err();
    assert!(
        failure.contains("cannot reassign a let binding"),
        "{failure}"
    );
}

#[test]
fn code_binding_wildcards_and_aliases_preserve_context() {
    let source = "fn run(flag:bool)->bool {
        const _ = '(not flag);
        const (_ as first) as second: <x:bool|-bool> = '(x);
        ~[x=flag]<first> == ~[x=flag]<second>
    }";
    for flag in [false, true] {
        assert_eq!(
            run(source, "run", vec![Value::Bool(flag)]),
            Value::Bool(true)
        );
    }
    let source = "pure comptime fn build(flag:bool)-><|-bool> {
        let _ = '(false);
        var first as second = '(false);
        first = '(true);
        if true {let _ as hidden = '(false);}
        if flag {first} else {second}
    } fn run()->bool {~<build(true)> and not ~<build(false)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn discarded_code_still_checks_construction_and_interfaces() {
    for source in [
        "pure comptime fn build(flag:bool)-><|-bool> where requires flag {'(true)} fn run()->bool {const _=build(false);true}",
        "fn run()->bool {const _: <|-bool> = '(1);true}",
        "pure comptime fn unused()-><x:bool|-bool> {let _=if x {'(true)} else {'(false)};'(true)}",
        "pure comptime fn unused()-><|-bool> {let _ as alias='(false);alias='(true);alias}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn discarded_code_executes_constructor_but_not_quoted_body() {
    let source = "pure comptime fn checked(flag:bool)-><|-bool> where ensures flag {'(true)}
        fn run()->bool {const _=checked(false);true}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("postcondition"), "{error}");
    let source = "fn forever()->bool {while true {} false}
        fn run()->bool {const _='(forever());true}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let source =
        "fn run()->bool {const (_ as first) as second: <|-bool> = '(true);~<first> and ~<second>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn local_code_interfaces_classify_diverging_construction_without_invocation() {
    let source = "pure comptime fn unused()-><|-bool> {
        let code:<|-bool> = do {while true {}};
        code
    }";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    for body in ["do {true}", "do {while false {} }"] {
        let source =
            format!("pure comptime fn unused()-><|-bool> {{let code:<|-bool> = {body};code}}");
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{body}"
        );
    }
}

#[test]
fn local_code_construction_is_recognized_through_loop_returns() {
    for body in [
        "do {while true {return '(true);}}",
        "do {inline for index in 0..1 {return '(true);}}",
        "do {inline for (index,element) in (true,false)'tuple_enumerate {return '(element);}}",
    ] {
        let source = format!(
            "pure comptime fn build()-><|-bool> {{let code={body};code}} fn run()->bool {{~<build()>}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true), "{body}");
    }
}

#[test]
fn nested_code_binding_classification_uses_declared_interfaces() {
    let source = "pure comptime fn unused()-><|-bool> {
        let outer=do {
            let (_ as first) as second:<|-bool> = do {while true {}};
            second
        };
        outer
    }";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    for body in [
        "do {inline for index in 0..0 {return '(true);}}",
        "do {while false {return '(true);}}",
        "do {while flag {return '(true);}}",
    ] {
        let source =
            format!("pure comptime fn unused(flag:bool)-><|-bool> {{let code={body};code}}");
        let failure = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            failure.contains("without returning a code result"),
            "{body}: {failure}"
        );
    }
}

#[test]
fn generator_implicit_inputs_use_named_supply_and_skip_positional_slots() {
    let source = "pure comptime fn build(implicit enabled:bool,comptime value:bool)-><|-bool> {
        '(enabled and value)
    } fn run()->bool {
        ~<build(true,.enabled=true)> and not ~<build(.enabled=false,true)>
    }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let source="pure comptime fn build(implicit code:<x:bool|-bool>,value:bool,.copy:bool=value)-><x:bool|-bool> {
        if copy {code} else {'(not x)}
    } fn run(value:bool)->bool {
        const original:<x:bool|-bool> = '(x);
        const selected=build(true,.code=original);
        ~[x=value]<selected> == value
    }";
    for value in [false, true] {
        assert_eq!(
            run(source, "run", vec![Value::Bool(value)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn generator_implicit_inputs_preserve_spreads_and_forwarding() {
    let source="pure comptime fn identity(value:T)->T where T {value}
        pure comptime fn build(implicit enabled:bool,a:bool,b:bool)-><|-bool> {'(enabled and a and not b)}
        pure comptime fn forward(comptime enabled:bool)-><|-bool> {build(...identity((true,false)),.enabled=enabled)}
        fn run()->bool {~<forward(true)> and not ~<forward(false)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_implicit_inputs_reject_missing_duplicate_and_runtime_supply() {
    for (source, anchor) in [
        (
            "pure comptime fn build(implicit enabled:bool)-><|-bool> {'(enabled)} fn run()->bool {~<build()>}",
            "implicit parameter requires explicit named supply",
        ),
        (
            "pure comptime fn build(implicit enabled:bool)-><|-bool> {'(enabled)} fn run()->bool {~<build(true)>}",
            "argument count mismatch",
        ),
        (
            "pure comptime fn build(implicit enabled:bool)-><|-bool> {'(enabled)} fn run()->bool {~<build(.enabled=true,.enabled=false)>}",
            "more than once",
        ),
        (
            "pure comptime fn build(implicit enabled:bool)-><|-bool> {'(enabled)} fn run(value:bool)->bool {~<build(.enabled=value)>}",
            "runtime",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
}

#[test]
fn generator_implicit_inputs_feed_defaults_and_contracts() {
    let source="pure comptime fn build(implicit comptime enabled:bool,.code:<|-bool> = '(enabled))-><|-bool>
        where requires enabled, ensures enabled {code}
        fn run()->bool {~<build(.enabled=true)> and not ~<build(.code='(false),.enabled=true)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let source = "pure comptime fn build(implicit enabled:bool)-><|-bool>
        where requires enabled {while true {}}
        fn run()->bool {~<build(.enabled=false)>}";
    let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn generator_place_rhs_preserves_updates_to_other_owned_objects() {
    let source = "struct State {flag:bool}
        pure comptime fn build()-><|-bool> {
            let first=State {flag:false};
            let second=State {flag:false};
            first.flag=do {second.flag=true;second.flag};
            '(first.flag and second.flag)
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_assignment_frames_preserve_scalar_updates_and_old_quotes() {
    let source = "struct State {flag:bool}
        pure comptime fn build()-><|-bool> {
            var flag=false;
            let state=State {flag:false};
            const old='(not state.flag and not flag);
            flag=do {state.flag=true;state.flag};
            if state.flag and flag {old} else {'(false)}
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let source = "struct State {flag:bool}
        pure comptime fn build()-><|-bool> {
            let state=State {flag:false};var count:.u8'builtin=0;
            state.flag=do {count+=1;true};
            '(state.flag and count == 1)
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_bit_update_frame_preserves_selector_and_rhs_objects() {
    let source="struct State {bits:.u8'builtin}
        struct Counter {index:.u8'builtin,called:bool}
        pure comptime fn build()-><|-bool> {
            let state=State {bits:0};
            let counter=Counter {index:0,called:false};
            state.bits'bitvec[do {counter.index=if counter.index == 0 {1} else {2};counter.index}]+=do {
                counter.called=true;1
            };
            '(state.bits == 2 and counter.index == 1 and counter.called)
        } fn run()->bool {~<build()>}";
    assert_eq!(run_bit_generator(source), Value::Bool(true));
}

#[test]
fn generator_update_frames_keep_nested_write_mutability_and_phase_checks() {
    for source in [
        "struct State {flag:bool} pure comptime fn bad()-><|-bool> {let state=State {flag:false};let flag=false;state.flag=do {flag=true;true};'(true)}",
        "struct State {flag:bool} pure comptime fn bad()-><flag:bool|-bool> {let state=State {flag:false};state.flag=do {flag=true;true};'(true)}",
        "fn runtime()->bool {true} struct State {flag:bool} pure comptime fn bad()-><|-bool> {let state=State {flag:false};state.flag=do {runtime()};'(true)}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generator_data_initializers_and_conditions_preserve_owned_state() {
    let source = "struct State {count:.u8'builtin}
        pure comptime fn build()-><|-bool> {
            let state=State {count:0};
            let chosen=do {state.count+=1;state.count == 1};
            if do {state.count+=1;chosen} {
                while do {state.count+=1;state.count < 4} {}
            }
            '(state.count == 4)
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_data_arguments_and_packets_observe_prior_updates() {
    let source = "struct State {count:.u8'builtin}
        pure comptime fn consume(a:.u8'builtin,b:.u8'builtin)-><|-bool> {'(a == 1 and b == 2)}
        pure comptime fn build()-><|-bool> {
            let state=State {count:0};
            let code=consume(do {state.count+=1;state.count},do {state.count+=1;state.count});
            if state.count == 2 {code} else {'(false)}
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let source = "struct State {count:.u8'builtin}
        pure comptime fn consume(a:.u8'builtin,b:.u8'builtin)-><|-bool> {'(a == 1 and b == 2)}
        pure comptime fn build()-><|-bool> {
            let state=State {count:0};
            let code=consume(...do {state.count+=1;(state.count,)},do {state.count+=1;state.count});
            if state.count == 2 {code} else {'(false)}
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_match_subject_and_range_endpoints_update_state_once() {
    let source = "struct State {count:.u8'builtin}
        pure comptime fn build()-><|-bool> {
            let state=State {count:0};
            let code=do {state.count+=1;true} match {true=>'(true),false=>'(false)};
            inline for index in do {state.count+=1;0}..do {state.count+=1;2} {}
            if state.count == 3 {code} else {'(false)}
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_data_frames_preserve_short_circuit_and_unselected_conditions() {
    let source = "struct State {count:.u8'builtin}
        pure comptime fn build()-><|-bool> {
            let state=State {count:0};
            let ignored=false and do {state.count+=1;true};
            let chosen=true or do {state.count+=1;false};
            let code=when {
                do {state.count+=1;true}=>'(true),
                do {state.count+=1;true}=>'(false),
                else=>'(false)
            };
            if state.count == 1 and chosen and not ignored {code} else {'(false)}
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_local_completed_type_values_drive_data_instantiation() {
    let source = "pure comptime fn identity(value:T)->T where T {value}
        pure comptime fn build()-><|-bool> {
            let Word=.u8'builtin;
            let value:Word=17;
            let copied=identity<Word>(value);
            '(copied == 17)
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_type_evidence_survives_nested_patterns_aliases_and_updates() {
    let source = "struct State {flag:bool}
        pure comptime fn identity(value:T)->T where T {value}
        pure comptime fn build()-><|-bool> {
            let ((Word as Alias),(_,Flag))=(.u8'builtin,(false,bool));
            let state=State {flag:false};
            let copied=do {state.flag=true;identity<Alias>(17)};
            let enabled:Flag=state.flag;
            let original:Word=copied;
            '(enabled and original == 17)
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_local_type_evidence_keeps_normal_form_identity() {
    let source = "pure comptime fn Identity<T>->.Type'builtin {T}
        pure comptime fn build()-><|-bool> {
            let Word=.u8'builtin;
            let Nominal=Identity<Word>;
            let same=Nominal == Identity<Word>;
            '(same)
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let opaque = source.replace("Nominal == Identity<Word>", "Nominal == Word");
    let failure = support::compile_with_protocols(&opaque, &Default::default()).unwrap_err();
    assert!(failure.contains("TypeContractUnresolved"), "{failure}");
}

#[test]
fn generator_type_evidence_does_not_invoke_unknown_producers() {
    let source = "pure comptime fn unknown()->.Type'builtin {while true {} bool}
        pure comptime fn unused()-><|-bool> {
            let T=unknown();let value:T=true;'(true)
        }";
    let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        failure.contains("must contain a completed Type value"),
        "{failure}"
    );
    let source = "pure comptime fn unused()-><|-bool> {
        var T=bool;T=.u8'builtin;let value:T=1;'(true)
    }";
    let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        failure.contains("must contain a completed Type value"),
        "{failure}"
    );
}

#[test]
fn generator_record_destructuring_retains_completed_type_fields() {
    let source = "struct Types {word:.Type'builtin,flag:.Type'builtin}
        pure comptime fn identity(value:T)->T where T {value}
        pure comptime fn build()-><|-bool> {
            let {flag:Flag,word:Word}=Types {flag:bool,word:.u8'builtin};
            let copied=identity<Word>(17);
            let enabled:Flag=true;
            '(enabled and copied == 17)
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_type_evidence_projects_record_literals_and_constant_records() {
    let source = "struct Types {word:.Type'builtin,flag:.Type'builtin}
        pure comptime fn build()-><|-bool> {
            let Word=Types {word:.u8'builtin,flag:bool}.word;
            let value:Word=17;
            '(value == 17)
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let source = "pure comptime fn schema(...fields:T)->T where T {fields}
        const TYPES=schema {word:.u8'builtin,flag:bool};
        pure comptime fn build()-><|-bool> {
            let {flag:Flag,word:Word}=TYPES;
            let Other=TYPES.word;
            let value:Word=17;let other:Other=value;let enabled:Flag=true;
            '(enabled and other == 17)
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_type_projection_does_not_skip_constructor_execution() {
    let prefix = "struct Types {word:.Type'builtin,enabled:bool}
        pure comptime fn checked(flag:bool)->bool where requires flag {flag}
        pure comptime fn build()-><|-bool> {
            let Word=Types {word:.u8'builtin,enabled:checked(false)}.word;
            let value:Word=17;'(value == 17)
        }";
    support::compile_with_protocols(prefix, &Default::default()).unwrap();
    let failure = support::compile_with_protocols(
        &format!("{prefix} fn run()->bool {{~<build()>}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn ordinary_comptime_generators_construct_and_forward_code() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        comptime fn build(value:bool)-><x:bool|-bool> {
            let enabled=invert(value);
            '(x and enabled)
        }
        comptime fn forward(value:bool)-><x:bool|-bool> {build(value)}
        fn run(value:bool)->bool {
            const enabled=forward(false);const disabled=forward(true);
            ~[x=value]<enabled> == value and not ~[x=value]<disabled>
        }";
    for value in [false, true] {
        assert_eq!(
            run(source, "run", vec![Value::Bool(value)]),
            Value::Bool(true)
        );
    }
}

#[test]
fn ordinary_comptime_generators_use_defaults_contracts_and_mutable_state() {
    let source = "struct State {count:.u8'builtin}
        comptime fn check(flag:bool)->bool where requires flag {flag}
        comptime fn build(implicit enabled:bool,.initial:bool=check(enabled))-><|-bool>
            where requires enabled, ensures enabled {
            let state=State {count:0};var code='(initial);
            while do {state.count+=1;state.count < 3} {code='(not ~<code>);}
            code
        }
        fn run()->bool {~<build(.enabled=true)> and not ~<build(.enabled=true,.initial=false)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn ordinary_comptime_generator_definition_checks_do_not_execute_bodies() {
    let source = "comptime fn forever()->bool {while true {} false}
        comptime fn unused()-><|-bool> {let value=forever();'(value)}";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    for source in [
        "fn runtime()->bool {true} comptime fn unused()-><|-bool> {let value=runtime();'(value)}",
        "comptime fn unused()-><x:bool|-bool> {let value=x;'(value)}",
        "comptime fn build(flag:bool)-><|-bool> where requires flag {'(true)} fn run()->bool {~<build(false)>}",
        "comptime fn build(flag:bool)-><|-bool> where ensures flag {'(true)} fn run()->bool {~<build(false)>}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generator_local_type_lists_drive_registered_tuple_instantiation() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        pure comptime fn identity(value:T)->T where T {value}
        comptime fn build()-><|-bool> {
            let fields:List<Type> = [bool,.u8'builtin];
            let Product=Tuple<...fields>;
            let pair:Product=(true,17);
            let (flag,number)=identity<Tuple<...fields>>(pair);
            '(flag and number == 17)
        } fn run()->bool {~<build()>}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generator_type_list_aliases_support_named_and_empty_collection_inputs() {
    for (fields, application, expected) in [
        (
            "[bool,.u8'builtin]",
            "Tuple<.Es=alias>",
            "Tuple<bool,.u8'builtin>",
        ),
        ("[]", "Tuple<...alias>", "Tuple<>"),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            pure comptime fn List<T>->Type {{T}}
            pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
            comptime fn build()-><|-bool> {{
                let fields:List<Type> = {fields};let alias=fields;
                let Product={application};let same=Product == {expected};'(same)
            }} fn run()->bool {{~<build()>}}"
        );
        let package = support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input::List"),
                tuple: Some("input::Tuple"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn generator_type_list_evidence_does_not_run_unknown_element_functions() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        comptime fn unknown()->Type {while true {} bool}
        comptime fn unused()-><|-bool> {
            let fields:List<Type> = [unknown()];
            let Product=Tuple<...fields>;let value:Product=(true,);'(true)
        }";
    let failure = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(failure.contains("elaboration failed"), "{failure}");
    assert!(
        !failure.contains("BudgetExhausted"),
        "definition checking invoked unknown input: {failure}"
    );
}

#[test]
fn generator_named_comptime_callbacks_compute_data() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        comptime fn identity(value:T)->T where T {value}
        comptime fn build(action:comptime fn(bool)->bool,value:bool)-><|-bool> {
            let result=action(value);'(result)
        } fn run()->bool {~<build(invert,false)> and ~<build(identity<bool>,true)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_named_callbacks_can_update_invocation_local_storage() {
    let source = "comptime fn mark(value:*bool)->bool {value.* = true;true}
        comptime fn build(action:comptime fn(*bool)->bool)-><|-bool> {
            var marked=false;let result=action(marked.ref);'(marked and result)
        } fn run()->bool {~<build(mark)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callback_defaults_and_implicit_supply_use_comptime_phase() {
    let source="comptime fn invert(value:bool)->bool {not value}
        comptime fn identity(value:bool)->bool {value}
        comptime fn build(value:bool,.action:comptime fn(bool)->bool=invert)-><|-bool> {
            let result=action(value);'(result)
        }
        comptime fn forward(implicit action:comptime fn(bool)->bool)-><|-bool> {build(true,.action=action)}
        fn run()->bool {~<build(false)> and ~<forward(.action=identity)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callbacks_keep_stage_and_freezing_boundaries() {
    for source in [
        "fn runtime(value:bool)->bool {value} comptime fn build(action:comptime fn(bool)->bool)-><|-bool> {let result=action(true);'(result)} fn run()->bool {~<build(runtime)>}",
        "comptime fn unused(action:fn(bool)->bool)-><|-bool> {'(true)}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
    let source="comptime fn checked(flag:bool)->bool where requires flag {flag}
        comptime fn build(action:comptime fn(bool)->bool)-><|-bool> {let result=action(false);'(result)}
        fn run()->bool {~<build(checked)>}";
    let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn generator_captureless_lambda_callbacks_and_post_do_are_executable() {
    let source="comptime fn build(action:comptime fn(bool)->bool)-><|-bool> {let result=action(false);'(result)}
        fn run()->bool {~<build(|value| not value)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let source = "comptime fn build(value:bool,lambda action:comptime fn(bool)->bool)-><|-bool> {
            let result=action(value);'(result)
        } fn run()->bool {const code=build(false) do {not _0};~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_post_do_composes_with_computed_spreads_and_implicit_inputs() {
    let source="comptime fn identity(value:T)->T where T {value}
        comptime fn build(lambda action:comptime fn(bool)->bool,value:bool,implicit enabled:bool)-><|-bool> {
            let result=action(value);'(enabled and result)
        }
        fn run()->bool {const code=build(...identity((false,)),.enabled=true) do {not _0};~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_lambda_parameters_accept_explicit_tactics_and_function_values() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        comptime fn build(value:bool,lambda action:comptime fn(bool)->bool)-><|-bool> {
            let result=action(value);'(result)
        } fn run()->bool {~<build(false,not _0)> and ~<build(false,invert)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_lambda_routing_rejects_ambiguity_bad_placeholders_and_stage_leaks() {
    for source in [
        "comptime fn build(lambda action:comptime fn(bool)->bool)-><|-bool> {'(true)} fn run()->bool {const code=build() do {_1};~<code>}",
        "comptime fn build(lambda first:comptime fn()->bool,lambda second:comptime fn()->bool)-><|-bool> {'(true)} fn run()->bool {const code=build() do {true};~<code>}",
        "fn runtime()->bool {true} comptime fn build(lambda action:comptime fn()->bool)-><|-bool> {'(true)} fn run()->bool {const code=build() do {runtime()};~<code>}",
        "comptime fn build(lambda action:comptime fn()->bool)-><|-bool> {'(true)} fn run(flag:bool)->bool {const code=build() do {flag};~<code>}",
        "comptime fn unused(lambda code:<|-bool>)-><|-bool> {code}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn unused_generator_callbacks_are_checked_without_being_invoked() {
    let source = "comptime fn build(lambda action:comptime fn()->bool)-><|-bool> {'(true)}
        fn run()->bool {const code=build() do {while true {} false};~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_generic_lambda_uses_later_literal_type_evidence() {
    let source = "comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        } fn run()->bool {const code=build(not _0,false);~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_generic_lambda_literal_evidence_respects_expected_numeric_types() {
    let source = "comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const letter=build(_0,'a');
            const number:<|-.u8'builtin> = build(_0,17);
            ~<letter> == 'a' and ~<number> == 17
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callback_inference_does_not_replay_argument_computations() {
    let source = "struct State {count:.u8'builtin}
        comptime fn build(action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {
            let state=State {count:0};
            let code=build(do {state.count+=1;|value| not value},false);
            if state.count == 1 {code} else {'(false)}
        } fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callback_context_can_come_from_a_later_expanded_literal() {
    let source="comptime fn build(lambda action:comptime fn(T)->T,value:T,implicit enabled:bool)-><|-T> where T {
            let result=action(value);'(result)
        } fn run()->bool {const code=build(not _0,...(false,),.enabled=true);~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_generic_callback_uses_later_packet_signature_without_invoking_it() {
    let source = "comptime fn packet()->(bool,) {(false,)}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        } fn run()->bool {const code=build(not _0,...packet());~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_packet_signature_hints_preserve_argument_order_and_state() {
    let source = "typealias Byte=.u8'builtin;
        comptime fn packet(count:*Byte)->(bool,) where requires count.* == 1 {
            count.* += 1;(false,)
        }
        comptime fn build(action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {
            var count:Byte=0;
            let code=build(do {count+=1;|value| not value},...packet(count.ref));
            if count == 2 {code} else {'(false)}
        } fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_packet_type_hints_do_not_execute_unused_producers_or_hide_phase_errors() {
    let source = "comptime fn packet()->(bool,) {while true {} (false,)}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn unused()-><|-bool> {build(not _0,...packet())}";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    let runtime = source.replace("comptime fn packet()", "fn packet()");
    let failure = support::compile_with_protocols(&runtime, &Default::default()).unwrap_err();
    assert!(failure.contains("wrong execution phase"), "{failure}");
}

#[test]
fn generator_existing_packet_types_supply_earlier_callback_context() {
    let source = "comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {let packet=(false,);build(not _0,...packet)}
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callback_inference_uses_direct_producer_signatures() {
    let source = "comptime fn input()->bool {false}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        } fn run()->bool {const code=build(not _0,input());~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_direct_producer_evidence_preserves_state_order_and_contracts() {
    let source = "typealias Byte=.u8'builtin;
        comptime fn input(count:*Byte)->bool where requires count.* == 2 {
            count.* += 1;false
        }
        comptime fn build(action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {
            var count:Byte=0;
            let code=build(do {count+=1;|value| not value},do {count+=1;input(count.ref)});
            if count == 3 {code} else {'(false)}
        } fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("requires count.* == 2", "requires count.* == 3");
    let failure = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn generator_direct_producer_evidence_neither_invokes_unused_bodies_nor_erases_phase() {
    let source = "comptime fn input()->bool {while true {} false}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn unused()-><|-bool> {build(not _0,input())}";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    let runtime = source.replace("comptime fn input()", "fn input()");
    let failure = support::compile_with_protocols(&runtime, &Default::default()).unwrap_err();
    assert!(failure.contains("wrong execution phase"), "{failure}");
}

#[test]
fn generator_callback_inference_uses_callable_parameter_results() {
    let source = "comptime fn input()->bool {false}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn outer(producer:comptime fn()->bool)-><|-bool> {
            let code=build(not _0,producer());code
        }
        fn run()->bool {~<outer(input)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let anonymous = source.replace("outer(input)", "outer(|| false)");
    assert_eq!(run(&anonymous, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callback_inference_uses_fixed_results_of_generic_producers() {
    let source = "comptime fn input(ignored:U)->bool where U {false}
        comptime fn packet(ignored:U)->(bool,) where U {(false,)}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const first=build(not _0,input(17));
            const second=build(not _0,...packet('x'));
            ~<first> and ~<second>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callback_context_flows_through_data_control_branches() {
    for expression in [
        "if flag {input()} else {input()}",
        "when {flag=>input(),else=>input()}",
        "flag match {true=>input(),false=>input()}",
    ] {
        let source = format!(
            "comptime fn input()->bool {{false}}
            comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {{
                let result=action(value);'(result)
            }}
            comptime fn outer(flag:bool)-><|-bool> {{let code=build(not _0,{expression});code}}
            fn run()->bool {{~<outer(true)> and ~<outer(false)>}}"
        );
        assert_eq!(
            run(&source, "run", vec![]),
            Value::Bool(true),
            "{expression}"
        );
    }
}

#[test]
fn generator_branch_packet_evidence_does_not_execute_selectors_or_unselected_arms() {
    let source = "typealias Byte=.u8'builtin;
        comptime fn packet(count:*Byte)->(bool,) where requires count.* == 2 {
            count.* += 1;(false,)
        }
        comptime fn never()->(bool,) {while true {} (true,)}
        comptime fn build(action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {
            var count:Byte=0;
            let code=build(do {count+=1;|value| not value},
                ...if do {count+=1;true} {packet(count.ref)} else {never()});
            if count == 3 {code} else {'(false)}
        } fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_branch_type_evidence_does_not_hide_invalid_selectors_or_missing_coverage() {
    for expression in [
        "if runtime() {input()} else {input()}",
        "true match {false=>input()}",
        "when {runtime()=>input(),else=>input()}",
    ] {
        let source = format!(
            "fn runtime()->bool {{true}}
            comptime fn input()->bool {{false}}
            comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {{
                let result=action(value);'(result)
            }}
            comptime fn unused()-><|-bool> {{build(not _0,{expression})}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{expression}"
        );
    }
}

#[test]
fn generator_callbacks_use_explicit_static_producer_type_arguments() {
    let source = "comptime fn identity(value:T)->T where T {value}
        comptime fn packet(value:T)->(T,) where T {(value,)}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const direct=build(not _0,identity<bool>(false));
            const spread=build(not _0,...packet<bool>(false));
            ~<direct> and ~<spread>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callbacks_use_immutable_type_values_in_explicit_producer_arguments() {
    let source = "comptime fn identity(value:T)->T where T {value}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {
            let Flag=bool;
            let code=build(not _0,identity<Flag>(false));code
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_explicit_producer_hints_support_named_and_tuple_types() {
    let source = "comptime fn identity(value:T)->T where .T:.Type'builtin=bool {value}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const direct=build(not _0,identity<.T=bool>(false));
            const spread=build(not _0,...identity<.T=(bool,)>((false,)));
            ~<direct> and ~<spread>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_explicit_producer_hints_keep_constraints_and_execution_phase() {
    for declaration in [
        "fn input(value:T)->T where T {value}",
        "comptime fn input(value:T)->T where T, requires T == .char'builtin {value}",
    ] {
        let source = format!(
            "{declaration}
            comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {{
                let result=action(value);'(result)
            }}
            fn run()->bool {{const code=build(not _0,input<bool>(false));~<code>}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{declaration}"
        );
    }
}

#[test]
fn generator_explicit_producer_hints_keep_argument_order_and_single_evaluation() {
    let source = "typealias Byte=.u8'builtin;
        comptime fn input(value:T,count:*Byte)->T where T, requires count.* == 1 {
            count.* += 1;value
        }
        comptime fn build(action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {
            var count:Byte=0;
            let code=build(do {count+=1;|value| not value},input<bool>(false,count.ref));
            if count == 2 {code} else {'(false)}
        } fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callbacks_infer_nested_generic_producer_results() {
    let source = "comptime fn identity(value:T)->T where T {value}
        comptime fn packet(value:T)->(T,) where T {(value,)}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const direct=build(not _0,identity(identity(false)));
            const spread=build(not _0,...packet(identity(false)));
            ~<direct> and ~<spread>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn inferred_generic_producer_results_keep_evaluation_order() {
    let source = "typealias Byte=.u8'builtin;
        comptime fn input(value:T,count:*Byte)->T where T, requires count.* == 1 {
            count.* += 1;value
        }
        comptime fn build(action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {
            var count:Byte=0;
            let flag=false;
            let code=build(do {count+=1;|value| not value},input(flag,count.ref));
            if count == 2 {code} else {'(false)}
        } fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn inferred_generic_producer_hints_preserve_numeric_context() {
    let source = "comptime fn identity(value:T)->T where T {value}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const number:<|-.u8'builtin> = build(_0,identity(17));
            ~<number> == 17
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn inferred_generic_producer_hints_do_not_invoke_unused_code_or_hide_phase() {
    let source = "comptime fn identity(value:T)->T where T {while true {} value}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn unused()-><|-bool> {let code=build(not _0,identity(false));code}";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    let runtime = source.replace("comptime fn identity", "fn identity");
    let failure = support::compile_with_protocols(&runtime, &Default::default()).unwrap_err();
    assert!(failure.contains("wrong execution phase"), "{failure}");
}

#[test]
fn generic_variadic_producer_results_supply_callback_context() {
    let source = "comptime fn pack(...items:T)->T where T {items}
        comptime fn build(lambda action:comptime fn(T)->bool,value:T)-><|-bool> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const code=build(do {let (flag,letter)=_0;not flag and letter == 'x'},pack(false,'x'));
            ~<code>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let expanded = source.replace("pack(false,'x')", "pack(...(false,'x'))");
    assert_eq!(run(&expanded, "run", vec![]), Value::Bool(true));
    let named = source.replace("pack(false,'x')", "pack(.items=(false,'x'))");
    assert_eq!(run(&named, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_variadic_producer_hints_preserve_registered_tuple_heads() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        comptime fn pack(...items:T)->T where T {items}
        comptime fn build(lambda action:comptime fn(T)->bool,value:T)-><|-bool> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const code=build(do {let (flag,letter)=_0;not flag and letter == 'x'},pack(false,'x'));
            ~<code>
        }";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_variadic_producer_hints_cover_empty_collection() {
    let source = "comptime fn pack(...items:T)->T where T {items}
        comptime fn build(lambda action:comptime fn(T)->bool,value:T)-><|-bool> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {const code=build(|value| true,pack());~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_variadic_producer_hints_do_not_reorder_element_producers() {
    let source = "typealias Count=.Integer'builtin;
        comptime fn next(count:*Count)->bool {
            let first=count.* == 1;count.* += 1;first
        }
        comptime fn pack(...items:T)->T where T {items}
        comptime fn build(action:comptime fn(T)->bool,value:T)-><|-bool> where T {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=build(do {count+=1;|value| {let (first,second)=value;first and not second}},
                pack(next(count.ref),next(count.ref)));
            if count == 3 {code} else {'(false)}
        } fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_property_producers_supply_callback_record_context() {
    let source = "comptime fn schema(...properties:T)->T where T {properties}
        comptime fn build(lambda action:comptime fn(T)->bool,value:T)-><|-bool> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const code=build(_0.enabled and _0.letter == 'x',schema {enabled:true,letter:'x'});
            ~<code>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_property_producer_hints_preserve_field_evaluation_order() {
    let source = "typealias Count=.Integer'builtin;
        comptime fn next(count:*Count)->bool {
            let first=count.* == 1;count.* += 1;first
        }
        comptime fn schema(...properties:T)->T where T {properties}
        comptime fn build(action:comptime fn(T)->bool,value:T)-><|-bool> where T {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=build(do {count+=1;|value| value.first and not value.second},
                schema {first:next(count.ref),second:next(count.ref)});
            if count == 3 {code} else {'(false)}
        } fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn property_producer_evidence_does_not_hide_invalid_fields_or_phase() {
    let prefix = "comptime fn schema(...properties:T)->T where T {properties}
        comptime fn build(lambda action:comptime fn(T)->bool,value:T)-><|-bool> where T {
            let result=action(value);'(result)
        }";
    for invocation in [
        "build(_0.enabled,schema {enabled:true,enabled:false})",
        "build(_0.missing,schema {enabled:true})",
        "build(not _0.enabled,schema {enabled:'x'})",
    ] {
        let source = format!("{prefix} fn run()->bool {{const code={invocation};~<code>}}");
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{invocation}"
        );
    }
    let source=format!("{prefix} fn runtime()->bool {{true}}
        comptime fn unused()-><|-bool> {{let code=build(_0.enabled,schema {{enabled:runtime()}});code}}");
    let failure = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(failure.contains("wrong execution phase"), "{failure}");
}

#[test]
fn property_producer_evidence_does_not_execute_unused_fields() {
    let source="comptime fn forever()->bool {while true {} false}
        comptime fn schema(...properties:T)->T where T {properties}
        comptime fn build(lambda action:comptime fn(T)->bool,value:T)-><|-bool> where T {
            let result=action(value);'(result)
        }
        comptime fn unused()-><|-bool> {let code=build(_0.enabled,schema {enabled:forever()});code}";
    support::compile_with_protocols(source, &Default::default()).unwrap();
}

#[test]
fn generator_producer_inference_respects_static_type_defaults() {
    let source = "comptime fn identity(value:T)->T where .T:.Type'builtin=bool {value}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {const code=build(not _0,identity(false));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("=bool {value}", "=.char'builtin {value}");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generator_producer_inference_resolves_chained_dependent_type_defaults() {
    let source="comptime fn second(first:T,second:V)->V where T,.U:.Type'builtin=T,.V:.Type'builtin=U {second}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {const code=build(not _0,second(true,false));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_explicit_producer_types_complete_static_defaults() {
    let source = "comptime fn second(first:T,second:U)->U where T,.U:.Type'builtin=T {second}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {const code=build(not _0,second<bool>(true,false));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_producer_type_override_skips_divergent_default() {
    let source = "comptime fn Never()->.Type'builtin {while true {} bool}
        comptime fn identity(value:T)->T where .T:.Type'builtin=Never() {value}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {const code=build(not _0,identity<.T=bool>(false));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_producer_inference_uses_computed_spread_packet_types() {
    let source = "comptime fn packet()->(bool,) {(false,)}
        comptime fn identity(value:T)->T where T {value}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {const code=build(not _0,identity(...packet()));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_collection_inference_combines_computed_spread_packets() {
    let source="comptime fn first()->(bool,) {(false,)}
        comptime fn second()->(.char'builtin,) {('x',)}
        comptime fn pack(...items:T)->T where T {items}
        comptime fn build(lambda action:comptime fn(T)->bool,value:T)-><|-bool> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const code=build(do {let (flag,letter)=_0;not flag and letter == 'x'},pack(...first(),...second()));
            ~<code>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn computed_packet_type_evidence_keeps_nested_producer_evaluation_order() {
    let source = "typealias Count=.Integer'builtin;
        comptime fn packet(count:*Count)->(bool,) {
            let first=count.* == 1;count.* += 1;(first,)
        }
        comptime fn identity(value:T)->T where T {value}
        comptime fn build(action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=build(do {count+=1;|value| value},identity(...packet(count.ref)));
            if count == 2 {code} else {'(false)}
        } fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nested_packet_type_evidence_does_not_execute_unused_producers_or_hide_phase() {
    let source = "comptime fn packet()->(bool,) {while true {} (false,)}
        comptime fn identity(value:T)->T where T {value}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn unused()-><|-bool> {let code=build(not _0,identity(...packet()));code}";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    let runtime = source.replace("comptime fn packet()", "fn packet()");
    let failure = support::compile_with_protocols(&runtime, &Default::default()).unwrap_err();
    assert!(failure.contains("wrong execution phase"), "{failure}");
}

#[test]
fn static_alias_chains_supply_producer_type_arguments_and_defaults() {
    let source = "typealias Flag=bool;typealias Enabled=Flag;
        comptime fn identity(value:T)->T where .T:.Type'builtin=Enabled {value}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const first=build(not _0,identity(false));
            const second=build(not _0,identity<.T=Enabled>(false));
            ~<first> and ~<second>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_producer_type_arguments_forward_caller_type_parameters() {
    let source = "comptime fn identity(value:T)->T where T {value}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        comptime fn outer(value:U)-><|-U> where U {
            let code=build(_0,identity<U>(value));code
        }
        fn run()->bool {~<outer(true)> and ~<outer('x')> == 'x'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn static_normal_tuple_inputs_support_generic_producer_inference() {
    let source="typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        typealias Pair=Tuple<bool,.char'builtin>;
        comptime fn identity(value:T)->T where .T:Type=Pair {value}
        comptime fn build(lambda action:comptime fn(T)->bool,value:T)-><|-bool> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {
            const code=build(do {let (flag,letter)=_0;flag and letter == 'x'},identity((true,'x')));
            const explicit=build(do {let (flag,letter)=_0;flag and letter == 'x'},identity<.T=Tuple<bool,.char'builtin>>((true,'x')));
            ~<code> and ~<explicit>
        }";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn static_normal_form_producer_evidence_never_unfolds_the_head() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn identity(value:T)->T where .T:Type=Identity<bool> {value}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {
            let result=action(value);'(result)
        }
        fn run()->bool {const code=build(_0,identity(false));~<code>}";
    let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        failure.contains("normal") && failure.contains("bool"),
        "{failure}"
    );
}

#[test]
fn static_normal_tuple_evidence_uses_completed_type_lists() {
    for argument in ["...fields", ".Es=fields", "...[bool,.char'builtin]"] {
        let source = format!(
            "typealias Type=.Type'builtin;
            pure comptime fn List<T>->Type {{T}}
            pure comptime fn Tuple<...Es:List<Type>>->Type {{while true {{}} bool}}
            comptime fn identity(value:T)->T where T {{value}}
            comptime fn build(lambda action:comptime fn(T)->bool,value:T)-><|-bool> where T {{
                let result=action(value);'(result)
            }}
            comptime fn outer()-><|-bool> {{
                let fields:List<Type> = [bool,.char'builtin];
                let code=build(do {{let (flag,letter)=_0;flag and letter == 'x'}},
                    identity<Tuple<{argument}>>((true,'x')));code
            }}
            fn run()->bool {{~<outer()>}}"
        );
        let package = support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input::List"),
                tuple: Some("input::Tuple"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn generator_temporary_pointer_writes_preserve_state_and_quote_snapshots() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct State {flag:bool,count:.u8'builtin} impl Duplicate for State {}
        pure comptime fn build()-><|-bool> {
            let state=State {flag:false,count:0};
            const original='(not state.flag and state.count == 0);
            state.ref.* = State {flag:true,count:1};
            state.ref.*.count += 2;
            state.flag.ref.* = false;
            '(~<original> and not state.flag and state.count == 3)
        } fn run()->bool {~<build()>}";
    let compile = |source: &str| {
        support::compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("Duplicate"),
                drop: Some("Retire"),
                ..Default::default()
            },
        )
    };
    let package = compile(source).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for rejected in [
        source.replace("impl Duplicate for State {}", ""),
        format!("{source} impl Retire for State {{fn drop(*self) {{}}}}"),
    ] {
        let failure = compile(&rejected).unwrap_err();
        assert!(failure.contains("pointer Copy evidence"), "{failure}");
    }
}

#[test]
fn generator_temporary_pointer_selection_runs_once_before_rhs() {
    let source = "struct State {count:.u8'builtin,other:bool}
        pure comptime fn build()-><|-bool> {
            let state=State {count:1,other:false};
            var calls:.u8'builtin=0;
            (do {calls+=1;state.count.ref}).* += do {
                state.other=true;calls+=2;calls
            };
            '(state.count == 4 and state.other and calls == 3)
        } fn run()->bool {~<build()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_temporary_pointer_selection_can_choose_between_owned_roots() {
    let source = "struct State {flag:bool}
        pure comptime fn build(select:bool)-><|-bool> {
            let first=State {flag:false};let second=State {flag:false};
            (if select {first.flag.ref} else {second.flag.ref}).* = true;
            '(first.flag == select and second.flag != select)
        } fn run()->bool {~<build(true)> and ~<build(false)>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_temporary_pointer_writes_reject_stage_leaks_and_wrong_pointees() {
    for (source, anchor) in [
        (
            "pure comptime fn bad()-><value:bool|-bool> {value.ref.* = false;'(value)}",
            "runtime context",
        ),
        (
            "pure comptime fn bad()-><|-bool> {var value=false;value.ref.* = 'x';'(value)}",
            "type",
        ),
        (
            "pure comptime fn bad()-><|-*bool> {var value=false;let pointer=value.ref;'(pointer)} fn run()->*bool {~<bad()>}",
            "runtime binding",
        ),
        (
            "fn select(p:*bool)->*bool {p} pure comptime fn bad()-><|-bool> {var value=false;select(value.ref).* = true;'(value)}",
            "execution phase",
        ),
    ] {
        let failure =
            support::compile_with_protocols(source, &Default::default()).expect_err(source);
        assert!(failure.contains(anchor), "{source}: {failure}");
    }
}

#[test]
fn generator_temporary_pointer_bit_writes_preserve_siblings_and_check_bounds() {
    let prefix = "struct State {bits:.u8'builtin,other:bool}
        pure comptime fn build(index:.u8'builtin)-><|-bool> {
            let state=State {bits:0,other:false};
            state.ref.*.bits'bitvec[index]=do {state.other=true;1};
            '(state.bits == 2 and state.other)
        }";
    let failure = support::compile_with_protocols(
        &format!("{prefix} fn run()->bool {{~<build(8)>}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(failure.contains("IndexOutOfBounds"), "{failure}");
    assert_eq!(
        run(
            &format!("{prefix} fn run()->bool {{~<build(1)>}}"),
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn generator_explicit_type_clause_domains_preserve_inference_and_defaults() {
    let source = "typealias Kind=.Type'builtin;
        pure comptime fn forward(code:<x:T|-T>)-><x:T|-T> where T:Kind {code}
        pure comptime fn compose(code:<x:T|-T>)-><x:T|-T> where T:Kind {forward(code)}
        pure comptime fn choose(first:T,second:U)-><|-U> where T:Kind,.U:Kind=T {'(second)}
        fn run()->bool {
            let flag=true;const code=compose('[x=flag](flag));
            ~<code> and ~<forward<bool>('[x=flag](flag))>
                and not ~<choose(true,false)>
                and ~<choose<bool,.U=.char'builtin>(true,'a')> == 'a'
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_explicit_type_clause_domains_check_constraints_and_result_domains() {
    let prefix = "typealias Kind=.Type'builtin;
        trait Flag {} impl Flag for bool {}
        pure comptime fn forward(code:<|-T>)-><|-T> where T:Kind, requires T :- Flag {code}";
    assert_eq!(
        run(
            &format!("{prefix} fn run()->bool {{~<forward('(true))>}}"),
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
    for source in [
        format!("{prefix} fn run()->.char'builtin {{~<forward('( 'a'))>}}"),
        "pure comptime fn invalid()-><|-bool> where value:.char'builtin {'(value)}".to_string(),
        "pure comptime fn keep(code:<|-T>)-><|-T> where T:.Type'builtin {code} fn run()->bool {~<keep<bool>('( 'x'))>}".to_string(),
    ] {
        let failure = support::compile_with_protocols(&source, &Default::default()).expect_err(&source);
        assert!(failure.contains("elaboration failed"), "{source}: {failure}");
    }
}

#[test]
fn generator_variant_payload_patterns_read_prior_candidate_bindings() {
    let source = "enum Pair {pair(bool,bool)}
        pure comptime fn build(input:Pair)-><|-bool> {
            input match {
                .pair(first,(second if first == second))=>'(first),
                _=>'(false),
            }
        }
        fn run()->bool {~<build(Pair.pair(true,true))>
            and not ~<build(Pair.pair(true,false))>
            and not ~<build(Pair.pair(false,false))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_named_field_candidates_read_prior_bindings() {
    let source = "struct Pair {first:bool,second:bool}
        enum Choice {pair {first:bool,second:bool}}
        pure comptime fn build(input:Pair)-><|-bool> {
            input match {
                {first:a,second:(b if a == b)}=>'(a),
                _=>'(false),
            }
        }
        pure comptime fn variant(input:Choice)-><|-bool> {
            input match {
                .pair {first:a,second:(b if a == b)}=>'(a),
                _=>'(false),
            }
        }
        fn run()->bool {
            ~<build(Pair {first:true,second:true})>
            and not ~<build(Pair {first:true,second:false})>
            and ~<variant(Choice.pair {first:true,second:true})>
            and not ~<variant(Choice.pair {first:true,second:false})>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}
#[test]
fn generator_value_clauses_select_code_without_runtime_capture() {
    let source = "comptime fn choose()-><x:bool |- bool> where flag:bool {
        if flag {'(x)} else {'(not x)}
    }
    fn run(value:bool)->(bool,bool) {
        const yes=choose<true>();const no=choose<false>();
        (~[x=value]<yes>,~[x=value]<no>)
    }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Tuple(vec![Value::Bool(true), Value::Bool(false)])
    );
}

#[test]
fn generator_value_clauses_mix_types_defaults_and_nested_calls() {
    let source = "typealias Type=.Type'builtin;
        comptime fn leaf(code:<x:T |- T>)-><x:T |- T> where T,flag:bool {code}
        comptime fn forward(code:<x:T |- T>)-><x:T |- T> where T,flag:bool {leaf<T,flag>(code)}
        comptime fn configured()-><x:bool |- bool> where .flag:bool=true,.copy:bool=flag {
            '(x and copy)
        }
        fn run(value:bool)->(bool,bool,bool) {
            const input:<x:bool |- bool> = '(x);
            const identity=forward<bool,false>(input);
            const enabled=configured();const disabled=configured<.flag=false>();
            (~[x=value]<identity>,~[x=value]<enabled>,~[x=value]<disabled>)
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Tuple(vec![
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(false)
        ])
    );
}

#[test]
fn generator_value_clauses_reconstruct_aggregate_inputs() {
    let source = "struct Config {enabled:bool}
        comptime fn configured()-><x:bool |- bool> where config:Config {
            if config.enabled {'(x)} else {'(not x)}
        }
        fn run(value:bool)->bool {
            const code=configured<Config {enabled:true}>();~[x=value]<code>
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_value_clauses_allow_type_inference_with_fixed_defaults() {
    let source = "comptime fn configured(value:T)-><x:bool |- bool> where T,.enabled:bool=true {
        '(x and enabled)
    }
    fn run(value:bool)->bool {
        const first=configured(false);const second=configured('a');
        ~[x=value]<first> and ~[x=value]<second>
    }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Bool(true)
    );
}

#[test]
fn generator_value_clauses_compute_defaults_and_check_contracts() {
    let source = "comptime fn invert(flag:bool)->bool {not flag}
        comptime fn configured()-><x:bool |- bool>
            where .flag:bool=true,.opposite:bool=invert(flag),ensures opposite != flag {
            '(x and opposite)
        }
        fn run(value:bool)->(bool,bool) {
            const first=configured();const second=configured<.flag=false>();
            (~[x=value]<first>,~[x=value]<second>)
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Tuple(vec![Value::Bool(false), Value::Bool(true)])
    );
}

#[test]
fn recursive_generator_value_defaults_use_the_new_call_frame() {
    let source = "comptime fn configured()->< |- bool>
        where .flag:bool=true,.copy:bool=not flag {
        if flag {configured<.flag=false>()} else {'(copy)}
    }
    fn run()->bool {const code=configured();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_value_clauses_substitute_dependent_domains() {
    let source = "comptime fn constant()-><x:T |- T> where T,value:T {'(value)}
        fn run(value:bool)->(bool,.u8'builtin) {
            const flag=constant<bool,false>();
            const byte=constant<.u8'builtin,200>();
            let zero:.u8'builtin=0;
            (~[x=value]<flag>,~[x=zero]<byte>)
        }";
    assert_eq!(
        run(source, "run", vec![Value::Bool(true)]),
        Value::Tuple(vec![Value::Bool(false), Value::UnsignedInteger(200)])
    );
}

#[test]
fn generator_value_clauses_skip_overridden_defaults_and_reject_failed_contracts() {
    let source = "comptime fn never()->bool {never()}
        comptime fn configured()->< |- bool> where .flag:bool=never() {'(flag)}
        fn run()->bool {const code=configured<.flag=true>();~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let error = support::compile_with_protocols(
        "comptime fn configured()->< |- bool> where flag:bool,ensures flag {'(flag)}
        fn run()->bool {const code=configured<false>();~<code>}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("contract") || error.contains("postcondition"),
        "{error}"
    );
}

#[test]
fn generator_value_clauses_reject_invalid_inputs_and_unused_bodies() {
    for source in [
        "comptime fn build()->< |- bool> where flag:bool {'(flag)} fn run(value:bool)->bool {const code=build<value>();~<code>}",
        "comptime fn build()->< |- bool> where flag:bool {'(flag)} fn run()->bool {const code=build<'a'>();~<code>}",
        "comptime fn build()->< |- bool> where flag:bool {'(flag)} fn run()->bool {const code=build();~<code>}",
        "comptime fn build()->< |- bool> where .flag:bool='a' {'(flag)} fn run()->bool {true}",
        "comptime fn build()->< |- bool> where .flag:bool=flag {'(flag)} fn run()->bool {true}",
        "comptime fn build()-><x:bool |- bool> where flag:bool {if flag {'(x)} else {let bad=x;'(bad)}} fn run()->bool {true}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}
#[test]
fn generator_value_defaults_wait_for_inferred_types_and_previous_defaults() {
    let source="typealias Type=.Type'builtin;
        comptime fn is_bool(kind:Type)->bool {kind == bool}
        comptime fn configured(value:T)->< |- bool>
            where T,.flag:bool=is_bool(T),.opposite:bool=not flag {'(opposite)}
        comptime fn forward(value:T)->< |- bool> where T {configured(value)}
        fn run()->(bool,bool) {const flag=forward(true);const character=forward('a');(~<flag>,~<character>)}";
    assert_eq!(
        run(source, "run", vec![]),
        Value::Tuple(vec![Value::Bool(false), Value::Bool(true)])
    );
}

#[test]
fn generator_dependent_value_defaults_dispatch_after_target_type_inference() {
    let source = "trait Initial {comptime fn initial()->Self;}
        impl Initial for bool {comptime fn initial()->bool {true}}
        impl Initial for .char'builtin {comptime fn initial()->.char'builtin {'a'}}
        comptime fn build()->< |- T>
            where T :- Initial,.value:T=T.as(Initial).initial() {'(value)}
        fn run()->(bool,.char'builtin) {
            const flag:< |- bool> = build();
            const character:< |- .char'builtin> = build();
            (~<flag>,~<character>)
        }";
    assert_eq!(
        run(source, "run", vec![]),
        Value::Tuple(vec![Value::Bool(true), Value::Char('a')])
    );
}

#[test]
fn generator_deferred_value_defaults_keep_contract_rejection() {
    let source = "typealias Type=.Type'builtin;
        comptime fn is_bool(kind:Type)->bool {kind == bool}
        comptime fn build(value:T)->< |- bool>
            where T,.flag:bool=is_bool(T),ensures flag {'(flag)}
        fn run()->bool {const code=build('a');~<code>}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("contract") || error.contains("postcondition"),
        "{error}"
    );
}
#[test]
fn generator_deferred_defaults_keep_type_and_value_declaration_order() {
    let source="typealias Type=.Type'builtin;
        comptime fn is_bool(kind:Type)->bool {kind == bool}
        comptime fn SelectType(flag:bool)->Type {if flag {bool} else {.char'builtin}}
        comptime fn configured(value:T)->< |- bool>
            where T,.flag:bool=is_bool(T),.U:Type=SelectType(flag),.result:bool=is_bool(U) {'(result)}
        fn run()->(bool,bool) {const first=configured(true);const second=configured('a');(~<first>,~<second>)}";
    assert_eq!(
        run(source, "run", vec![]),
        Value::Tuple(vec![Value::Bool(true), Value::Bool(false)])
    );
}
#[test]
fn generator_structural_type_defaults_keep_literal_context_before_value_defaults() {
    let source = "typealias Type=.Type'builtin;
        comptime fn is_bool(kind:Type)->bool {kind == bool}
        comptime fn choose(first:T,second:U)->< |- U>
            where T,.U:Type=T,.flag:bool=is_bool(T) {'(second)}
        fn run()->.u8'builtin {
            const first:.u8'builtin=5;const code=choose(first,200);~<code>
        }";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}
#[test]
fn unused_generator_value_defaults_still_check_execution_phase() {
    for body in [
        "fn run()->bool {true}",
        "fn run()->bool {const code=build<bool,.flag=true>(false);~<code>}",
    ] {
        let source = format!(
            "fn runtime(value:bool)->bool {{value}}
            comptime fn build(value:T)->< |- bool>
                where T,.flag:bool=runtime(T == bool) {{'(flag)}} {body}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}
#[test]
fn generator_deferred_type_defaults_cannot_be_overridden_by_argument_inference() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn is_bool(kind:Type)->bool {kind == bool}
        comptime fn SelectType(flag:bool)->Type {if flag {bool} else {.char'builtin}}
        comptime fn choose(first:T,second:U)->< |- U>
            where T,.flag:bool=is_bool(T),.U:Type=SelectType(flag) {'(second)}";
    assert_eq!(
        run(
            &format!("{prefix} fn run()->bool {{const code=choose(true,false);~<code>}}"),
            "run",
            vec![]
        ),
        Value::Bool(false)
    );
    let error = support::compile_with_protocols(
        &format!("{prefix} fn run()->.char'builtin {{const code=choose(true,'a');~<code>}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("elaboration failed"), "{error}");
}
#[test]
fn generator_ready_defaults_supply_narrow_literal_contexts_before_arguments() {
    let source = "typealias Type=.Type'builtin;
        comptime fn is_bool(kind:Type)->bool {kind == bool}
        comptime fn Width(flag:bool)->Type {if flag {.u8'builtin} else {.u16'builtin}}
        comptime fn choose(first:T,second:U)->< |- U>
            where T,.flag:bool=is_bool(T),.U:Type=Width(flag) {'(second)}
        fn run()->(.u8'builtin,.u16'builtin) {
            const narrow=choose(true,200);const wide=choose('a',500);
            (~<narrow>,~<wide>)
        }";
    for source in [
        source.to_owned(),
        source
            .replace("choose(true,200)", "choose(...(true,200))")
            .replace("choose('a',500)", "choose(...('a',500))"),
    ] {
        assert_eq!(
            run(&source, "run", vec![]),
            Value::Tuple(vec![
                Value::UnsignedInteger(200),
                Value::UnsignedInteger(500)
            ])
        );
    }
}

#[test]
fn generator_ready_defaults_contextualize_code_before_later_type_evidence() {
    let source = "typealias Type=.Type'builtin;
        comptime fn is_bool(kind:Type)->bool {kind == bool}
        comptime fn Width(flag:bool)->Type {if flag {.u8'builtin} else {.u16'builtin}}
        comptime fn choose(code:< |- U>,first:T)->< |- U>
            where T,.flag:bool=is_bool(T),.U:Type=Width(flag) {code}
        fn run()->(.u8'builtin,.u16'builtin) {
            const narrow=choose('(200),true);const wide=choose('(500),'a');
            (~<narrow>,~<wide>)
        }";
    assert_eq!(
        run(source, "run", vec![]),
        Value::Tuple(vec![
            Value::UnsignedInteger(200),
            Value::UnsignedInteger(500)
        ])
    );
}

#[test]
fn generator_ready_defaults_reject_literals_outside_selected_width() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn is_bool(kind:Type)->bool {kind == bool}
        comptime fn Width(flag:bool)->Type {if flag {.u8'builtin} else {.u16'builtin}}
        comptime fn choose(first:T,second:U)->< |- U>
            where T,.flag:bool=is_bool(T),.U:Type=Width(flag) {'(second)}";
    for call in ["choose(true,256)", "choose('a',65536)"] {
        let error = support::compile_with_protocols(
            &format!("{prefix} fn run() {{const code={call};}}"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.contains("elaboration failed"), "{error}");
    }
}

#[test]
fn generator_collections_supply_context_to_earlier_callbacks() {
    let source =
        "comptime fn build(lambda action:comptime fn(T)->bool,...items:T)-><|-bool> where T {
            let result=action(items);'(result)
        }
        fn run()->bool {
            const code=build(do {let (flag,letter)=_0;not flag and letter == 'x'},false,'x');
            ~<code>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    let expanded = source.replace(",false,'x');", ",...(false,'x'));");
    assert_eq!(run(&expanded, "run", vec![]), Value::Bool(true));
    let named = source.replace(",false,'x');", ",.items=(false,'x'));");
    assert_eq!(run(&named, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_collection_evidence_keeps_producer_order_and_packet_evaluation_once() {
    let prefix = "typealias Count=.Integer'builtin;
        comptime fn next(count:*Count)->bool {let first=count.* == 1;count.* += 1;first}
        comptime fn packet(count:*Count)->(bool,bool) {(next(count),next(count))}
        comptime fn build(action:comptime fn(T)->bool,...items:T)-><|-bool> where T {
            let result=action(items);'(result)
        }";
    for inputs in ["next(count.ref),next(count.ref)", "...packet(count.ref)"] {
        let source=format!("{prefix}
            comptime fn outer()-><|-bool> {{
                var count:Count=0;
                let code=build(do {{count+=1;|value| {{let (first,second)=value;first and not second}}}},{inputs});
                if count == 3 {{code}} else {{'(false)}}
            }} fn run()->bool {{~<outer()>}}");
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn generator_collection_evidence_checks_phase_without_invoking_unused_producers() {
    let source="comptime fn forever()->bool {while true {} false}
        comptime fn build(lambda action:comptime fn(T)->bool,...items:T)-><|-bool> where T {
            let result=action(items);'(result)
        }
        comptime fn unused()-><|-bool> {let code=build(do {let (flag,)=_0;not flag},forever());code}";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    let error = support::compile_with_protocols(
        &source.replace("comptime fn forever", "fn forever"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("wrong execution phase"), "{error}");
}

#[test]
fn generator_collection_evidence_preserves_numeric_destination_contexts() {
    let source = "comptime fn build(lambda action:comptime fn(T)->T,...items:T)-><|-T> where T {
            let result=action(items);'(result)
        }
        fn run()->(.u8'builtin,.u16'builtin) {
            const code:<|-(.u8'builtin,.u16'builtin)> = build(_0,200,500);~<code>
        }";
    assert_eq!(
        run(source, "run", vec![]),
        Value::Tuple(vec![
            Value::UnsignedInteger(200),
            Value::UnsignedInteger(500)
        ])
    );
}

#[test]
fn generator_collection_evidence_preserves_registered_tuple_identity() {
    let source="typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        comptime fn build(lambda action:comptime fn(T)->bool,...items:T)-><|-bool> where T {
            let result=action(items);'(result)
        }
        fn run()->bool {const code=build(do {let (flag,letter)=_0;flag and letter == 'a'},true,'a');~<code>}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generator_collection_evidence_supports_empty_and_property_collections() {
    let source="comptime fn build(lambda action:comptime fn(T)->bool,...items:T)-><|-bool> where T {
            let result=action(items);'(result)
        }
        fn run()->bool {const empty=build(|value| true);const record=build {_0.enabled and _0.letter == 'a',enabled:true,letter:'a'};~<empty> and ~<record>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_partial_collections_infer_types_without_defaulting_numeric_fields() {
    let source="comptime fn build(lambda action:comptime fn(T)->bool,...items:(T,.u8'builtin))-><|-bool> where T {
            let (flag,number)=items;let result=action(flag) and number == 200;'(result)
        }
        fn run()->bool {const code=build(not _0,false,200);~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
    assert_eq!(
        run(
            &source.replace("false,200", ".items=(false,200)"),
            "run",
            vec![]
        ),
        Value::Bool(true)
    );
}

#[test]
fn generator_partial_collections_propagate_nested_evidence_and_keep_tuple_heads() {
    let source="typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        comptime fn build(lambda action:comptime fn(T)->bool,...items:((T,.u8'builtin),.u16'builtin))-><|-bool> where T {
            let ((flag,small),large)=items;
            let result=action(flag) and small == 200 and large == 500;'(result)
        }
        fn run()->bool {const code=build(not _0,(false,200),500);~<code>}";
    for registered in [false, true] {
        let source = if registered {
            source.to_owned()
        } else {
            source.replace("pure comptime fn Tuple<...Es:List<Type>>->Type {bool}", "")
        };
        let package = support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: registered.then_some("input::List"),
                tuple: registered.then_some("input::Tuple"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn generator_partial_collections_unlock_default_computation_before_code_inputs() {
    let source = "typealias Type=.Type'builtin;
        comptime fn is_bool(kind:Type)->bool {kind == bool}
        comptime fn Width(flag:bool)->Type {if flag {.u8'builtin} else {.u16'builtin}}
        comptime fn build(code:<|-U>,...items:(T,.u8'builtin))-><|-U>
            where T,.flag:bool=is_bool(T),.U:Type=Width(flag) {code}
        fn run()->(.u8'builtin,.u16'builtin) {
            const narrow=build('(200),false,100);const wide=build('(500),'a',200);
            (~<narrow>,~<wide>)
        }";
    assert_eq!(
        run(source, "run", vec![]),
        Value::Tuple(vec![
            Value::UnsignedInteger(200),
            Value::UnsignedInteger(500)
        ])
    );
}

#[test]
fn generator_partial_collections_keep_shape_range_and_callback_rejections() {
    let prefix="comptime fn build(lambda action:comptime fn(T)->bool,...items:(T,.u8'builtin))-><|-bool> where T {
        let (value,number)=items;let result=action(value);'(result)
    }";
    for arguments in [
        "not _0,false,256",
        "not _0,'a',200",
        "not _0,false,200,true",
    ] {
        let source = format!("{prefix} fn run()->bool {{const code=build({arguments});~<code>}}");
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generator_callbacks_infer_results_from_partial_function_contexts() {
    let source = "comptime fn transform(lambda action:comptime fn(T)->U,value:T)-><|-U> where T,U {
        let result=action(value);'(result)
    }
    fn run()->.char'builtin {
        const code=transform(if _0 {'a'} else {'b'},true);~<code>
    }";
    assert_eq!(run(source, "run", vec![]), Value::Char('a'));
}

#[test]
fn generator_collected_callbacks_use_partial_function_contexts() {
    let source = "comptime fn transform(...items:(comptime fn(bool)->U,bool))-><|-U> where U {
        let (action,value)=items;let result=action(value);'(result)
    }
    fn run()->.char'builtin {
        const code=transform(|value| if value {'a'} else {'b'},true);~<code>
    }";
    assert_eq!(run(source, "run", vec![]), Value::Char('a'));
    assert_eq!(
        run(
            &source.replace(
                "transform(|value| if value {'a'} else {'b'},true)",
                "transform(.items=(|value| if value {'a'} else {'b'},true))"
            ),
            "run",
            vec![]
        ),
        Value::Char('a')
    );
}

#[test]
fn generator_partial_callbacks_keep_owned_state_and_source_order() {
    let source = "typealias Count=.Integer'builtin;
        comptime fn next(count:*Count)->bool {let first=count.* == 1;count.* += 1;first}
        comptime fn transform(action:comptime fn(T)->U,value:T)-><|-U> where T,U {
            let result=action(value);'(result)
        }
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=transform(do {count+=1;|value| if value {'a'} else {'b'}},next(count.ref));
            if count == 2 {'(~<code> == 'a')} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_partial_callbacks_check_unused_bodies_and_execution_phase() {
    let source = "comptime fn transform(lambda action:comptime fn(T)->U,value:T)-><|-U> where T,U {
        let result=action(value);'(result)
    }
    comptime fn unused()-><|-.char'builtin> {
        let code=transform(do {while true {} if _0 {'a'} else {'b'}},true);code
    }";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    let invalid = source.replace("while true {} if _0 {'a'} else {'b'}", "not _0 + 'a'");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    let runtime = source
        .replace(
            "comptime fn unused",
            "fn runtime()->.char'builtin {'a'} comptime fn unused",
        )
        .replace(
            "while true {} if _0 {'a'} else {'b'}",
            "if _0 {runtime()} else {'b'}",
        );
    let error = support::compile_with_protocols(&runtime, &Default::default()).unwrap_err();
    assert!(error.contains("wrong execution phase"), "{error}");
}

#[test]
fn generator_callback_containers_reject_captured_graphs() {
    let prefix = "comptime fn transform(...items:(comptime fn(bool)->bool,bool))-><|-bool> {
        let (action,value)=items;let result=action(value);'(result)
    }";
    for callback in [
        "do {let captured=true;|value:bool| value and captured}",
        "do {var captured=true;|value:bool| {captured=not captured;value and captured}}",
    ] {
        let source =
            format!("{prefix} fn run()->bool {{const code=transform({callback},true);~<code>}}");
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("cannot be frozen")
                || (error.contains("type mismatch") && error.contains("closure (environment")),
            "{error}"
        );
    }
}

#[test]
fn generator_callback_containers_support_nested_captureless_values() {
    let source = "comptime fn transform(...items:((comptime fn(bool)->U,),bool))-><|-U> where U {
        let ((action,),value)=items;let result=action(value);'(result)
    }
    fn run()->.char'builtin {
        const code=transform((|value| if value {'a'} else {'b'},),true);~<code>
    }";
    assert_eq!(run(source, "run", vec![]), Value::Char('a'));
}

#[test]
fn generator_collection_fields_propagate_callback_results_to_later_literals() {
    let source = "comptime fn narrow(value:bool)->.u8'builtin {if value {200} else {0}}
        comptime fn build(...items:(comptime fn(bool)->U,U))-><|-U> where U {
            let (action,value)=items;let result=action(true);'(value)
        }
        fn run()->.u8'builtin {const code=build(|value| narrow(value),200);~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_collection_fields_unlock_type_defaults_between_elements() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Width(kind:Type)->Type {if kind == bool {.u8'builtin} else {.u16'builtin}}
        comptime fn build(...items:(comptime fn(bool)->T,U))-><|-U> where T,.U:Type=Width(T) {
            let (action,value)=items;let result=action(true);'(value)
        }
        fn run()->(.u8'builtin,.u16'builtin) {
            const small=build(|value| not value,200);
            const large=build(|value| 'a',500);
            (~<small>,~<large>)
        }";
    assert_eq!(
        run(source, "run", vec![]),
        Value::Tuple(vec![
            Value::UnsignedInteger(200),
            Value::UnsignedInteger(500)
        ])
    );
    let invalid = source.replace("not value,200", "not value,256");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generator_collection_fields_update_interleaved_code_contexts() {
    let source = "comptime fn narrow(value:bool)->.u8'builtin {if value {200} else {0}}
        comptime fn build(...items:(comptime fn(bool)->U,U),implicit code:<|-U>)-><|-U> where U {code}
        fn run()->.u8'builtin {const code=build(|value| narrow(value),.code='(200),200);~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_collection_fields_preserve_source_order_and_registered_tuple_heads() {
    let source = "typealias Type=.Type'builtin;
        typealias Count=.Integer'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        comptime fn narrow(value:bool)->.u8'builtin {if value {200} else {0}}
        comptime fn build(...items:(comptime fn(bool)->U,U))-><|-U> where U {
            let (action,value)=items;let result=action(true);'(value)
        }
        comptime fn outer()-><|-.u8'builtin> {
            var count:Count=0;
            let code=build(do {count+=1;|value| narrow(value)},do {count+=1;200});
            if count == 2 {code} else {'(0)}
        }
        fn run()->.u8'builtin {~<outer()>}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(200)
    );
}

#[test]
fn generator_named_literal_packets_propagate_types_between_fields() {
    let source = "comptime fn narrow(value:bool)->.u8'builtin {if value {200} else {0}}
        comptime fn build(...items:(comptime fn(bool)->U,U))-><|-U> where U {
            let (action,value)=items;let result=action(true);'(value)
        }
        fn run()->.u8'builtin {const code=build(.items=(|value| narrow(value),200));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_named_literal_packets_advance_defaults_and_reject_overflow() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Width(kind:Type)->Type {if kind == bool {.u8'builtin} else {.u16'builtin}}
        comptime fn build(...items:(comptime fn(bool)->T,U))-><|-U> where T,.U:Type=Width(T) {
            let (action,value)=items;let result=action(true);'(value)
        }
        fn run()->(.u8'builtin,.u16'builtin) {
            const small=build(.items=(|value| not value,200));
            const large=build(.items=(|value| 'a',500));(~<small>,~<large>)
        }";
    assert_eq!(
        run(source, "run", vec![]),
        Value::Tuple(vec![
            Value::UnsignedInteger(200),
            Value::UnsignedInteger(500)
        ])
    );
    let invalid = source.replace("not value,200", "not value,256");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generator_named_literal_packets_keep_order_and_tuple_identity() {
    let source = "typealias Type=.Type'builtin;
        typealias Count=.Integer'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        comptime fn narrow(value:bool)->.u8'builtin {if value {200} else {0}}
        comptime fn build(...items:(comptime fn(bool)->U,U))-><|-U> where U {
            let (action,value)=items;let result=action(true);'(value)
        }
        comptime fn outer()-><|-.u8'builtin> {
            var count:Count=0;
            let code=build(.items=(do {count+=1;|value| narrow(value)},do {count = count * 10 + 2;200}));
            if count == 12 {code} else {'(0)}
        }
        fn run()->.u8'builtin {~<outer()>}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(200)
    );
}

#[test]
fn generator_named_computed_packets_keep_their_single_invocation_boundary() {
    let prefix="typealias Count=.Integer'builtin;
        comptime fn packet(count:*Count)->(bool,bool) {count.*+=1;(true,true)}
        comptime fn build(...items:(bool,bool))-><|-bool> {let (first,second)=items;'(first and second)}";
    for input in ["packet(count.ref)", "do {count+=1;(true,true)}"] {
        let source=format!("{prefix}
            comptime fn outer()-><|-bool> {{var count:Count=0;let code=build(.items={input});if count == 1 {{code}} else {{'(false)}}}}
            fn run()->bool {{~<outer()>}}");
        assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn generator_nominal_callback_data_reconstructs_record_and_variant_layouts() {
    let source = "struct Action {callback:comptime fn(bool)->bool}
        enum Choice {none,some(Action),}
        comptime fn invert(value:bool)->bool {not value}
        comptime fn build(choice:Choice)-><|-bool> {
            let result=choice match {.some(action)=>action.callback(false),.none=>false};'(result)
        }
        fn run()->bool {
            const named=build(Choice.some(Action {callback:invert}));
            const anonymous=build(Choice.some(Action {callback:|value| not value}));
            ~<named> and ~<anonymous> and not ~<build(Choice.none)>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_nominal_callback_data_instantiates_generic_field_layouts() {
    let source="struct Action where U {callback:comptime fn(bool)->U}
        comptime fn build(action:Action<U>)-><|-U> where U {let result=action.callback(true);'(result)}
        comptime fn letter(value:bool)->.char'builtin {if value {'a'} else {'b'}}
        comptime fn flag(value:bool)->bool {not value}
        fn run()->bool {
            const character=build(Action<.char'builtin>{callback:letter});
            const boolean=build(Action<bool>{callback:flag});
            ~<character> == 'a' and not ~<boolean>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_nominal_callback_data_reconstructs_newtype_representations() {
    let source="typealias Callback=comptime fn(bool)->bool;
        newtype Wrapped=Callback;
        struct Action {callback:Wrapped}
        comptime fn invert(value:bool)->bool {not value}
        comptime fn build(action:Action)-><|-bool> {let callback=action.callback.as(Callback);let result=callback(false);'(result)}
        fn run()->bool {const code=build(Action {callback:invert.as(Wrapped)});~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_nominal_callback_data_keeps_capture_and_pointer_boundaries() {
    for source in [
        "struct Action {callback:comptime fn(bool)->bool}
            comptime fn build(action:Action)-><|-Action> {'(action)}",

        "struct Action {callback:comptime fn(bool)->bool}
            comptime fn build(action:Action)-><|-bool> {'(true)}
            fn run()->bool {const code=build(Action {callback:do {let captured=true;|value:bool| value and captured}});~<code>}",
        "struct Data {pointer:*bool}
            comptime fn build(data:Data)-><|-Data> {'(data)}
            comptime fn outer()-><|-Data> {let flag=true;build(Data {pointer:flag.ref})}
            fn run()->Data {~<outer()>}",
        "typealias Callback=fn(bool)->bool;newtype Wrapped=Callback;
            comptime fn build(callback:Wrapped)-><|-bool> {'(true)}
            fn runtime(value:bool)->bool {value}
            fn run()->bool {~<build(runtime.as(Wrapped))>}",
    ] {
        assert!(support::compile_with_protocols(source,&Default::default()).is_err(),"{source}");
    }
}

#[test]
fn generator_readonly_captures_survive_calls_and_generator_forwarding() {
    let source="typealias Reader=%{flag:bool} comptime fn()->bool;
        comptime fn make(flag:bool)->Reader {|| flag}
        comptime fn build(action:Reader)-><|-bool> {let first=action();let second=action();'(first and second)}
        comptime fn forward(action:Reader)-><|-bool> {build(action)}
        fn run()->bool {~<forward(make(true))> and not ~<forward(make(false))>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_readonly_captures_preserve_nested_callbacks_and_nominal_wrappers() {
    let source="typealias Reader=%{flag:bool} comptime fn()->bool;
        typealias Forward=%{read:Reader} comptime fn()->bool;
        struct Action {callback:Forward}
        comptime fn make(flag:bool)->Reader {|| flag}
        comptime fn wrap(read:Reader)->Forward {|| read()}
        comptime fn build(action:Action)-><|-bool> {let first=action.callback();let second=action.callback();'(first and second)}
        fn run()->bool {
            const yes=build(Action {callback:wrap(make(true))});
            const no=build(Action {callback:wrap(make(false))});~<yes> and not ~<no>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_readonly_captures_keep_creation_snapshots() {
    let source = "typealias Reader=%{flag:bool} comptime fn()->bool;
        comptime fn build(...actions:(Reader,Reader))-><|-bool> {
            let (first,second)=actions;let a=first();let b=second();'(a and not b)
        }
        comptime fn outer()-><|-bool> {
            var flag=true;let first:Reader=|| flag;flag=false;let second:Reader=|| flag;
            build(first,second)
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_readonly_captures_reject_mutation_and_exposed_storage() {
    let prefix="typealias Reader=%{flag:bool} comptime fn()->bool;
        comptime fn build(action:Reader)-><|-bool> {let first=action();let second=action();'(first and second)}";
    for (factory, anchor) in [
        (
            "comptime fn make()->Reader {var flag=false;|| {flag=not flag;flag}}",
            "mutates captured state",
        ),
        (
            "comptime fn touch(pointer:*bool)->bool {pointer.* = not pointer.*;pointer.*}
          comptime fn make()->Reader {var flag=false;|| touch(flag.ref)}",
            "exposes captured storage",
        ),
    ] {
        let source = format!("{prefix} {factory} fn run()->bool {{~<build(make())>}}");
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains(anchor), "{error}");
    }
}

#[test]
fn generator_readonly_nominal_captures_preserve_nested_values() {
    let source="newtype Flag=bool;struct Config {enabled:Flag} enum Mode {off,on(Config),}
        typealias Reader=%{mode:Mode} comptime fn()->bool;
        comptime fn make(mode:Mode)->Reader {|| mode match {.off=>false,.on(config)=>config.enabled.as(bool)}}
        comptime fn build(action:Reader)-><|-bool> {let first=action();let second=action();'(first and second)}
        fn run()->bool {
            const yes=build(make(Mode.on(Config {enabled:true.as(Flag)})));
            const no=build(make(Mode.off));~<yes> and not ~<no>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_readonly_nominal_captures_keep_generic_type_arguments() {
    let source="struct Box where T {value:T}
        comptime fn make(value:T)->%{data:Box<T>} comptime fn()->T where T {
            let data=Box<T>{value:value};|| data.value
        }
        comptime fn build(action:%{data:Box<T>} comptime fn()->T)-><|-T> where T {
            let value=action();'(value)
        }
        fn run()->bool {const flag=build(make(true));const letter=build(make('a'));~<flag> and ~<letter> == 'a'}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_readonly_nominal_captures_reject_nested_mutable_callbacks() {
    let source="typealias Counter=%{flag:bool} comptime fn()->bool;
        struct Action {next:Counter}
        typealias Reader=%{action:Action} comptime fn()->bool;
        comptime fn make_counter()->Counter {var flag=false;|| {flag=not flag;flag}}
        comptime fn make_reader(action:Action)->Reader {|| action.next()}
        comptime fn build(reader:Reader)-><|-bool> {let first=reader();let second=reader();'(first and second)}
        fn run()->bool {~<build(make_reader(Action {next:make_counter()}))>}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("mutates captured state"), "{error}");
}

#[test]
fn generator_readonly_nominal_captures_reject_pointer_fields_and_field_addresses() {
    for (source, anchor) in [
        (
            "struct Data {pointer:*bool}
            typealias Reader=%{data:Data} comptime fn()->bool;
            comptime fn make(data:Data)->Reader {|| data.pointer.*}
            comptime fn build(reader:Reader)-><|-bool> {let value=reader();'(value)}
            comptime fn outer()-><|-bool> {let flag=true;build(make(Data {pointer:flag.ref}))}
            fn run()->bool {~<outer()>}",
            "immutable capture graph",
        ),
        (
            "struct Data {flag:bool}
            typealias Reader=%{data:Data} comptime fn()->bool;
            comptime fn touch(pointer:*bool)->bool {pointer.* = not pointer.*;pointer.*}
            comptime fn make(data:Data)->Reader {|| touch(data.flag.ref)}
            comptime fn build(reader:Reader)-><|-bool> {let value=reader();'(value)}
            fn run()->bool {~<build(make(Data {flag:true}))>}",
            "exposes captured storage",
        ),
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(error.contains(anchor), "{error}");
    }
}

#[test]
fn generator_named_function_values_supply_earlier_code_contexts() {
    let source = "comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        fn run()->.u8'builtin {const code=build('(200),narrow);~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_named_function_values_instantiate_static_type_inputs() {
    let prefix = "typealias Type=.Type'builtin;
        typealias Narrow=.u8'builtin;
        comptime fn identity(value:T)->T where T {value}
        comptime fn defaulted(value:T)->T where .T:Type=Narrow {value}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}";
    for function in ["identity<Narrow>", "defaulted", "defaulted<.T=Narrow>"] {
        let source =
            format!("{prefix} fn run()->Narrow {{const code=build('(200),{function});~<code>}}");
        assert_eq!(run(&source, "run", vec![]), Value::UnsignedInteger(200));
    }
}

#[test]
fn generator_named_function_value_evidence_keeps_source_order() {
    let source = "typealias Count=.Integer'builtin;
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        comptime fn outer()-><|-.u8'builtin> {
            var count:Count=0;
            let code=build(do {count+=1;'(200)},do {count = count * 10 + 2;narrow});
            if count == 12 {code} else {'(0)}
        }
        fn run()->.u8'builtin {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_named_function_value_evidence_does_not_invoke_bodies_or_skipped_defaults() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Never()->Type {while true {} bool}
        comptime fn never(value:.u8'builtin)->.u8'builtin {while true {} value}
        comptime fn defaulted(value:T)->T where .T:Type=Never() {value}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        fn run()->(.u8'builtin,.u8'builtin) {
            const first=build('(200),never);const second=build('(201),defaulted<.T=.u8'builtin>);
            (~<first>,~<second>)
        }";
    assert_eq!(
        run(source, "run", vec![]),
        Value::Tuple(vec![
            Value::UnsignedInteger(200),
            Value::UnsignedInteger(201)
        ])
    );
}

#[test]
fn generator_named_function_value_evidence_preserves_phase_and_requirements() {
    let prefix = "comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}";
    for (declaration, function) in [
        (
            "fn runtime(value:.u8'builtin)->.u8'builtin {value}",
            "runtime",
        ),
        (
            "comptime fn constrained(value:T)->T where T, requires T == bool {value}",
            "constrained<.u8'builtin>",
        ),
    ] {
        let source = format!(
            "{prefix} {declaration} fn run()->.u8'builtin {{const code=build('(200),{function});~<code>}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generator_field_type_evidence_supplies_earlier_code_contexts() {
    let source = "struct Config {action:comptime fn(.u8'builtin)->.u8'builtin}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn config()->Config {Config {action:narrow}}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        fn run()->.u8'builtin {const code=build('(200),config().action);~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_field_type_evidence_instantiates_generic_and_anonymous_records() {
    let prefix = "struct Config where T {action:comptime fn(T)->T}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn config(action:comptime fn(T)->T)->Config<T> where T {Config<T>{action:action}}
        comptime fn schema(...fields:T)->T where T {fields}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}";
    for action in [
        "config<.u8'builtin>(narrow).action",
        "schema {action:narrow}.action",
    ] {
        let source =
            format!("{prefix} fn run()->.u8'builtin {{const code=build('(200),{action});~<code>}}");
        assert_eq!(run(&source, "run", vec![]), Value::UnsignedInteger(200));
    }
}

#[test]
fn generator_field_type_evidence_keeps_packet_and_callback_source_order() {
    let source="typealias Count=.Integer'builtin;
        struct Config {packet:(bool,)}
        comptime fn config(count:*Count)->Config {let flag=count.* == 1;count.*+=1;Config {packet:(flag,)}}
        comptime fn build(action:comptime fn(T)->T,value:T)-><|-T> where T {let result=action(value);'(result)}
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=build(do {count+=1;|value| value},...config(count.ref).packet);
            if count == 2 {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_field_type_evidence_checks_unused_producer_phase_and_actual_contracts() {
    let prefix="struct Config {action:comptime fn(.u8'builtin)->.u8'builtin}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn config(flag:bool)->Config where requires flag {while true {} Config {action:narrow}}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}";
    let unused = format!(
        "{prefix} comptime fn unused()-><|-.u8'builtin> {{let code=build('(200),config(true).action);code}}"
    );
    support::compile_with_protocols(&unused, &Default::default()).unwrap();
    let runtime = unused.replace("comptime fn config", "fn config");
    let error = support::compile_with_protocols(&runtime, &Default::default()).unwrap_err();
    assert!(error.contains("wrong execution phase"), "{error}");
    let invalid = format!(
        "{} fn run()->.u8'builtin {{const code=build('(200),config(false).action);~<code>}}",
        prefix.replace("while true {} ", "")
    );
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(error.contains("precondition"), "{error}");
}

#[test]
fn generator_callback_call_results_supply_earlier_lambda_contexts() {
    let source="struct Config {action:comptime fn(bool)->bool}
        comptime fn invert(value:bool)->bool {not value}
        comptime fn config()->Config {Config {action:invert}}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {let result=action(value);'(result)}
        fn run()->bool {const code=build(not _0,config().action(true));~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callback_call_results_cover_factories_and_packets() {
    let source="typealias Reader=%{flag:bool} comptime fn()->bool;
        comptime fn reader(flag:bool)->Reader {|| flag}
        comptime fn packet()->(bool,) {(false,)}
        comptime fn factory()->comptime fn()->(bool,) {packet}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {let result=action(value);'(result)}
        fn run()->bool {
            const first=build(not _0,reader(false)());
            const second=build(not _0,...factory()());~<first> and ~<second>
        }";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callback_call_result_evidence_does_not_select_branches_early() {
    let source="typealias Count=.Integer'builtin;
        comptime fn invert(value:bool)->bool {not value}
        comptime fn identity(value:bool)->bool {value}
        comptime fn build(action:comptime fn(T)->T,value:T)-><|-T> where T {let result=action(value);'(result)}
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=build(do {count+=1;|value| not value},(if count == 1 {invert} else {identity})(true));
            if count == 1 {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_callback_call_result_evidence_retains_call_validation() {
    let prefix="struct Config {action:comptime fn(bool)->bool}
        comptime fn checked(value:bool)->bool where requires value {value}
        comptime fn config()->Config {Config {action:checked}}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {let result=action(value);'(result)}";
    for invocation in [
        "config().action('a')",
        "config().action()",
        "config().action(false)",
    ] {
        let source =
            format!("{prefix} fn run()->bool {{const code=build(not _0,{invocation});~<code>}}");
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
    let runtime="fn action(value:bool)->bool {value}
        fn factory()->fn(bool)->bool {action}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {let result=action(value);'(result)}
        comptime fn unused()-><|-bool> {let code=build(not _0,factory()(true));code}";
    let error = support::compile_with_protocols(runtime, &Default::default()).unwrap_err();
    assert!(error.contains("wrong execution phase"), "{error}");
}

#[test]
fn generator_callback_call_result_evidence_does_not_invoke_unused_factories() {
    let source="comptime fn action(value:bool)->bool {while true {} value}
        comptime fn factory()->comptime fn(bool)->bool {while true {} action}
        comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {let result=action(value);'(result)}
        comptime fn unused()-><|-bool> {let code=build(not _0,factory()(true));code}";
    support::compile_with_protocols(source, &Default::default()).unwrap();
}

#[test]
fn generator_block_binding_evidence_preserves_named_function_context() {
    let source = "comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        fn run()->.u8'builtin {const code=build('(200),do {let action=narrow;action});~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_block_binding_evidence_handles_annotations_and_shadowing() {
    let source="comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn wide(value:.u16'builtin)->.u16'builtin {value}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        comptime fn data(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {let result=action(value);'(result)}
        fn run()->(.u8'builtin,.u16'builtin,.u8'builtin) {
            const first=build('(200),do {let action=narrow;action});
            const second=build('(500),do {let action=narrow;do {let action=wide;action}});
            const third=data(_0,do {let value:.u8'builtin=201;value});
            (~<first>,~<second>,~<third>)
        }";
    assert_eq!(
        run(source, "run", vec![]),
        Value::Tuple(vec![
            Value::UnsignedInteger(200),
            Value::UnsignedInteger(500),
            Value::UnsignedInteger(201)
        ])
    );
}

#[test]
fn generator_block_binding_evidence_keeps_destructuring_producer_order() {
    let source="typealias Count=.Integer'builtin;
        comptime fn packet(count:*Count)->(bool,.char'builtin) {let ready=count.* == 1;count.*+=1;(ready,'a')}
        comptime fn build(action:comptime fn(T)->T,value:T)-><|-T> where T {let result=action(value);'(result)}
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=build(do {count+=1;|value| value},do {let (ready,letter)=packet(count.ref);ready});
            if count == 2 {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_block_binding_evidence_keeps_initializer_validation_and_phase() {
    let prefix = "comptime fn build(lambda action:comptime fn(T)->T,value:T)-><|-T> where T {let result=action(value);'(result)}";
    for body in [
        "let value:.u8'builtin='a';value",
        "let value:.u8'builtin=256;value",
    ] {
        let source = format!(
            "{prefix} fn run()->.u8'builtin {{const code=build(_0,do {{{body}}});~<code>}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
    let unused=format!("{prefix} comptime fn forever()->bool {{while true {{}} false}}
        comptime fn unused()-><|-bool> {{let code=build(not _0,do {{let value=forever();value}});code}}");
    support::compile_with_protocols(&unused, &Default::default()).unwrap();
    let error = support::compile_with_protocols(
        &unused.replace("comptime fn forever", "fn forever"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("wrong execution phase"), "{error}");
}

#[test]
fn generator_record_binding_evidence_preserves_callback_context() {
    let source = "struct Config where T {action:comptime fn(T)->T}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn config()->Config<.u8'builtin> {Config<.u8'builtin>{action:narrow}}
        comptime fn schema(...fields:T)->T where T {fields}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        fn run()->(.u8'builtin,.u8'builtin) {
            const first=build('(200),do {let {action}=config();action});
            const second=build('(201),do {let {action:selected}=schema {action:narrow};selected});
            (~<first>,~<second>)
        }";
    assert_eq!(
        run(source, "run", vec![]),
        Value::Tuple(vec![
            Value::UnsignedInteger(200),
            Value::UnsignedInteger(201)
        ])
    );
}

#[test]
fn generator_record_binding_evidence_keeps_nested_producer_order() {
    let source="typealias Count=.Integer'builtin;
        struct Config {action:comptime fn(.u8'builtin)->.u8'builtin}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn bundle(count:*Count)->(Config,bool) {count.* = count.* * 10 + 2;(Config {action:narrow},true)}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        comptime fn outer()-><|-.u8'builtin> {
            var count:Count=0;
            let code=build(do {count+=1;'(200)},do {let ({action},_)=bundle(count.ref);action});
            if count == 12 {code} else {'(0)}
        }
        fn run()->.u8'builtin {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_record_binding_evidence_retains_pattern_and_phase_validation() {
    let prefix = "struct Config {action:comptime fn(.u8'builtin)->.u8'builtin}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn config()->Config {while true {} Config {action:narrow}}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}";
    let source = format!(
        "{prefix} comptime fn unused()-><|-.u8'builtin> {{let code=build('(200),do {{let {{action}}=config();action}});code}}"
    );
    support::compile_with_protocols(&source, &Default::default()).unwrap();
    let error = support::compile_with_protocols(
        &source.replace("comptime fn config", "fn config"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("wrong execution phase"), "{error}");
    for pattern in ["{missing:action}", "{action,action:other}"] {
        let invalid = source.replace("let {action}=config()", &format!("let {pattern}=config()"));
        assert!(
            support::compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn generator_match_binding_evidence_preserves_callback_types() {
    let source="comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn pair()->(comptime fn(.u8'builtin)->.u8'builtin,bool) {(narrow,true)}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        fn run()->.u8'builtin {const code=build('(200),pair() match {(action,true)=>action,(action,false)=>action});~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_match_binding_evidence_preserves_record_field_types() {
    let source="struct Config where T {action:comptime fn(T)->T}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn config()->Config<.u8'builtin> {Config<.u8'builtin>{action:narrow}}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        fn run()->.u8'builtin {const code=build('(200),config() match {{action:selected}=>selected});~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_match_binding_evidence_keeps_subject_and_guard_order() {
    let source="typealias Count=.Integer'builtin;
        comptime fn narrow()->.u8'builtin {200}
        comptime fn wrong()->.u8'builtin {0}
        comptime fn pair(count:*Count)->(comptime fn()->.u8'builtin,bool) {count.*+=1;(narrow,true)}
        comptime fn build(transform:comptime fn(T)->bool,action:comptime fn()->T)-><|-bool> where T {let value=action();let result=transform(value);'(result)}
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=build(do {count+=1;|value| value == 200},pair(count.ref) match {
                (action,true) if count == 2=>action,
                (action,_)=>wrong,
            });
            if count == 2 {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_match_binding_evidence_retains_coverage_and_branch_validation() {
    let prefix = "comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn wide(value:.u16'builtin)->.u16'builtin {value}
        comptime fn pair()->(comptime fn(.u8'builtin)->.u8'builtin,bool) {(narrow,true)}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}";
    for arms in [
        "(action,true)=>action",
        "(action,true)=>action,(action,false)=>wide",
        "(action,'a')=>action,(action,_)=>action",
    ] {
        let source = format!(
            "{prefix} fn run()->.u8'builtin {{const code=build('(200),pair() match {{{arms}}});~<code>}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
    let source = format!(
        "{} comptime fn unused()-><|-.u8'builtin> {{let code=build('(200),pair() match {{(action,_)=>action}});code}}",
        prefix.replace("{(narrow,true)}", "{while true {} (narrow,true)}")
    );
    support::compile_with_protocols(&source, &Default::default()).unwrap();
}

#[test]
fn generator_enum_binding_evidence_preserves_generic_payload_types() {
    let source="enum Choice where T {first(comptime fn(T)->T),second(comptime fn(T)->T),}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn choice()->Choice<.u8'builtin> {Choice<.u8'builtin>.first(narrow)}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        fn run()->.u8'builtin {const code=build('(200),choice() match {.first(action)=>action,.second(action)=>action});~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_enum_binding_evidence_handles_named_nested_payloads() {
    let prefix = "enum Inner where T {item(comptime fn(T)->T)}
        enum Choice where T {first {payload:Inner<T>,flag:bool},second {action:comptime fn(T)->T}}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}";
    for selected in [
        "Choice<.u8'builtin>.first {flag:true,payload:Inner<.u8'builtin>.item(narrow)}",
        "Choice<.u8'builtin>.second {action:narrow}",
    ] {
        let source = format!(
            "{prefix}
            comptime fn choice()->Choice<.u8'builtin> {{{selected}}}
            fn run()->.u8'builtin {{const code=build('(200),choice() match {{
                .first {{payload:.item(action)}}=>action,
                .second {{action}}=>action
            }});~<code>}}"
        );
        assert_eq!(run(&source, "run", vec![]), Value::UnsignedInteger(200));
    }
}

#[test]
fn generator_enum_binding_evidence_keeps_invocation_and_guard_order() {
    let source="typealias Count=.Integer'builtin;
        enum Choice {item(comptime fn()->.u8'builtin)}
        comptime fn narrow()->.u8'builtin {200}
        comptime fn wrong()->.u8'builtin {0}
        comptime fn choice(count:*Count)->Choice {count.*+=1;Choice.item(narrow)}
        comptime fn build(transform:comptime fn(T)->bool,action:comptime fn()->T)-><|-bool> where T {let value=action();let result=transform(value);'(result)}
        comptime fn outer()-><|-bool> {
            var count:Count=0;
            let code=build(do {count+=1;|value| value == 200},choice(count.ref) match {
                .item(action) if count == 2=>action,
                .item(_)=>wrong,
            });
            if count == 2 {code} else {'(false)}
        }
        fn run()->bool {~<outer()>}";
    assert_eq!(run(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_enum_binding_evidence_retains_pattern_validation() {
    let prefix = "enum Choice where T {first {action:comptime fn(T)->T},second(comptime fn(T)->T)}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn choice()->Choice<.u8'builtin> {Choice<.u8'builtin>.second(narrow)}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}";
    for arms in [
        ".first {action}=>action",
        ".first {missing:action}=>action,.second(action)=>action",
        ".first {action,action:other}=>action,.second(action)=>action",
        ".first {action}=>action,.second(action,extra)=>action",
        ".first(action)=>action,.second(action)=>action",
    ] {
        let source = format!(
            "{prefix} fn run()->.u8'builtin {{const code=build('(200),choice() match {{{arms}}});~<code>}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
    let source = format!(
        "{} comptime fn unused()-><|-.u8'builtin> {{build('(200),choice() match {{.first {{action}}=>action,.second(action)=>action}})}}",
        prefix.replace(
            "{Choice<.u8'builtin>.second(narrow)}",
            "{while true {} Choice<.u8'builtin>.second(narrow)}"
        )
    );
    support::compile_with_protocols(&source, &Default::default()).unwrap();
}

#[test]
fn generator_combined_pattern_evidence_exports_common_callbacks() {
    let prefix = "enum Choice where T {first(comptime fn(T)->T),second(comptime fn(T)->T)}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}";
    for variant in ["first", "second"] {
        for arms in [
            ".first(action) or .second(action)=>action",
            "(.first(action) or .second(action)) as whole=>action",
            ".first(_ as action) or .second(_ as action)=>action",
        ] {
            let source=format!("{prefix} comptime fn choice()->Choice<.u8'builtin> {{Choice<.u8'builtin>.{variant}(narrow)}}
                fn run()->.u8'builtin {{const code=build('(200),choice() match {{{arms}}});~<code>}}");
            assert_eq!(run(&source, "run", vec![]), Value::UnsignedInteger(200));
        }
    }
}

#[test]
fn generator_combined_pattern_evidence_checks_alternatives() {
    let prefix="enum Choice {first(comptime fn(.u8'builtin)->.u8'builtin),second(comptime fn(.u16'builtin)->.u16'builtin)}
        comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn choice()->Choice {Choice.first(narrow)}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}";
    for arms in [
        ".first(action) or .second(action)=>action",
        ".first(action) or .second(other)=>action",
    ] {
        let source = format!(
            "{prefix} fn run()->.u8'builtin {{const code=build('(200),choice() match {{{arms}}});~<code>}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generator_whole_binding_evidence_preserves_inner_and_whole_types() {
    let source="comptime fn narrow(value:.u8'builtin)->.u8'builtin {value}
        comptime fn pair()->(comptime fn(.u8'builtin)->.u8'builtin,bool) {(narrow,true)}
        comptime fn build(code:<|-T>,action:comptime fn(T)->T)-><|-T> where T {code}
        fn run()->.u8'builtin {const code=build('(200),do {let (action,_) as whole=pair();let (selected,_)=whole;selected});~<code>}";
    assert_eq!(run(source, "run", vec![]), Value::UnsignedInteger(200));
}

#[test]
fn generator_guarded_pattern_evidence_preserves_runtime_selection() {
    let prefix="comptime fn narrow()->.u8'builtin {200}
        comptime fn wrong()->.u8'builtin {0}
        comptime fn pair()->(comptime fn()->.u8'builtin,bool) {(narrow,true)}
        comptime fn build(transform:comptime fn(T)->bool,action:comptime fn()->T)-><|-bool> where T {let value=action();let result=transform(value);'(result)}";
    let source = format!(
        "{prefix} comptime fn outer()-><|-bool> {{
        var ready=false;
        build(do {{ready=true;|value| value == 200}},pair() match {{
            ((action if ready),_)=>action,
            (_,_)=>wrong,
        }})
    }} fn run()->bool {{~<outer()>}}"
    );
    assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
    let invalid = format!(
        "{prefix} fn runtime()->bool {{true}}
        fn run()->bool {{~<build(|value| value == 200,pair() match {{
            ((action if runtime()),_)=>action,(_,_)=>wrong
        }})>}}"
    );
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generator_owned_float_frames_preserve_nan_and_finite_snapshots() {
    for width in ["f32", "f64"] {
        for (initial, place) in [
            ("var value:Real=1.5;", "value"),
            ("var state=State {value:1.5};", "state.value"),
        ] {
            let source = format!(
                "typealias Real=.{width}'builtin;
                struct State {{value:Real}}
                comptime fn build()-><|-bool> {{
                    {initial}
                    let finite={place} == 1.5;
                    {place}=Real.NaN;
                    let unordered=not ({place} == {place});
                    {place}=Real.Infinity;
                    let infinite={place} == Real.Infinity;
                    {place}= -0.0;
                    '(finite and unordered and infinite and {place} == 0.0)
                }}
                fn run()->bool {{~<build()>}}"
            );
            assert_eq!(run(&source, "run", vec![]), Value::Bool(true));
        }
    }
}
