#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Interpreter, Value};

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
            "struct State {flag:bool} pure comptime fn bad()->< |- bool> {let state=State {flag:true};state.ref.* = State {flag:false};'(true)}",
            "direct owned data place",
        ),
        (
            "struct State {bits:.u8'builtin} pure comptime fn bad()->< |- bool> {let state=State {bits:0};state.bits'bitvec[0..4]=16;'(true)}",
            "BitVec width",
        ),
        (
            "struct State {bits:.u8'builtin} pure comptime fn bad()->< |- bool> {let state=State {bits:0};state.bits'bitvec[0..4]+=1;'(true)}",
            "direct field path",
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
            "pure comptime fn build(pointer:*bool)->< |- bool> {'(true)}",
            "immutable value domain",
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

fn run(source: &str, entry: &str, inputs: Vec<Value>) -> Value {
    let package = support::compile(source);
    let graph = luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    vm.invoke_instance(&graph.roots[0], inputs).unwrap()
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
fn generator_multiple_unknown_packet_widths_are_not_guessed() {
    let source="pure comptime fn identity(value:T)->T where T {value}
        pure comptime fn select(a:bool,b:bool,c:bool,d:bool)->< |- bool> {'(true)}
        fn run()->bool {const code=select(...identity((true,false)),...identity((false,true)));~<code>}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("width"), "{error}");
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
