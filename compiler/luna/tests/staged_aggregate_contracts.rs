//! Declaration-scoped staged aggregates participate in program contracts.
#[path = "support/execution.rs"]
mod support;
use support::compile_with_protocols;

#[test]
fn staged_record_fields_keep_contract_identity_across_calls() {
    for (declarations, ty, constant, field) in [
        (
            "struct Config {limit:Byte,other:Byte}",
            "Config",
            "Config {limit:19,other:7}",
            "config.limit",
        ),
        (
            "struct Inner {limit:Byte} struct Config {inner:Inner,other:Byte}",
            "Config",
            "Config {inner:Inner {limit:19},other:7}",
            "config.inner.limit",
        ),
        (
            "comptime fn schema(...fields:T)->T where T {fields} const SHAPE=schema {limit:do {let x:Byte=19;x},other:do {let x:Byte=7;x}};typealias Config=SHAPE'type;",
            "Config",
            "SHAPE",
            "config.limit",
        ),
    ] {
        for body in [
            format!("value <= {field}"),
            format!("helper(value,{field})"),
        ] {
            let source=format!(
                "typealias Byte=.u8'builtin;{declarations} const CONFIG:{ty}={constant};
                fn helper(value:Byte,limit:Byte)->result:bool where requires value <= limit, ensures result {{value <= limit}}
                fn bounded(comptime config:{ty},value:Byte)->result:bool where requires value <= {field}, ensures result {{{body}}}
                fn run()->bool {{bounded(CONFIG,11)}}"
            );
            for input in [11, 19] {
                let source =
                    source.replace("bounded(CONFIG,11)", &format!("bounded(CONFIG,{input})"));
                let package = compile_with_protocols(&source, &Default::default())
                    .unwrap_or_else(|error| panic!("{source}: {error}"));
                assert_eq!(
                    luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
                    comptime_interpretor::Value::Bool(true)
                );
            }
            let invalid = source.replace(&format!("{{{body}}}"), &format!("{{value < {field}}}"));
            let error = compile_with_protocols(&invalid, &Default::default())
                .err()
                .expect("strict comparison is not implied by a non-strict premise");
            assert!(error.contains("postcondition"), "{error}");
        }
    }
}

#[test]
fn splice_conversion_checks_staged_record_postconditions() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        struct Config {limit:Byte,other:Byte} const CONFIG:Config=Config {limit:19,other:7};
        trait Splice {assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;}
        impl Splice for Config {assoc RuntimeTarget:Type=Byte;
            fn splice(comptime data:Config)->result:Byte where ensures result==data.limit {data.limit}}
        fn run()->Byte {let converted=CONFIG;converted}";
    let package =
        support::compile_with_bindings(source, &Default::default(), Some("Splice")).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::UnsignedInteger(19)
    );
    let invalid = source.replace("{data.limit}", "{data.other}");
    let error = support::compile_with_bindings(&invalid, &Default::default(), Some("Splice"))
        .err()
        .expect("another field cannot discharge the declared postcondition");
    assert!(error.contains("postcondition"), "{error}");
}

#[test]
fn staged_fields_remain_immutable_across_opaque_callbacks() {
    let source = "typealias Byte=.u8'builtin;
        struct Config {limit:Byte,other:Byte}
        const CONFIG:Config=Config {limit:19,other:7};
        fn bounded(comptime config:Config,action:fn())->result:bool
          where requires config.other <= config.limit, ensures result {
            action(); config.other <= config.limit
          }
        fn noop() {}
        fn run()->bool {bounded(CONFIG,noop)}";
    let package = compile_with_protocols(source, &Default::default())
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn opaque_callback_writes_do_not_freeze_runtime_memory() {
    for (declarations, initial, comparison) in [
        (
            "struct Config {limit:Byte,other:Byte}",
            "Config {limit:19,other:7}",
            "config.other <= config.limit",
        ),
        (
            "struct Inner {limit:Byte,other:Byte} struct Config {inner:Inner}",
            "Config {inner:Inner {limit:19,other:7}}",
            "config.inner.other <= config.inner.limit",
        ),
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            {declarations} const CONFIG:Config={initial};
            fn helper(comptime config:Config,action:fn(*Byte),target:*Byte)->result:bool
              where requires {comparison}, ensures result {{
                action(target); {comparison}
              }}
            fn set(target:*Byte) {{target.* = 23;}}
            fn run()->bool {{var value:Byte=7;
                let accepted=helper(CONFIG,set,value.ref); accepted and value==23}}"
        );
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        let invalid = source
            .replace(
                &format!("requires {comparison}"),
                &format!("requires {comparison} and target.* == 7"),
            )
            .replace(
                &format!("action(target); {comparison}"),
                "action(target); target.* == 7",
            );
        let error = compile_with_protocols(&invalid, &Default::default())
            .err()
            .expect("opaque callback may overwrite runtime pointees");
        assert!(error.contains("postcondition"), "{error}");
    }
}

#[test]
fn staged_defaults_transport_field_relations_in_parameter_order() {
    let source="typealias Byte=.u8'builtin;
        struct Config {limit:Byte,other:Byte}
        const CONFIG:Config=Config {limit:19,other:7};
        fn checked(value:Byte,limit:Byte)->result:bool
          where requires value <= limit, ensures result {value <= limit}
        fn helper(comptime config:Config,.value:Byte=config.other,.limit:Byte=config.limit)->result:bool
          where requires value <= limit, ensures result {checked(value,limit)}
        fn outer(comptime config:Config)->result:bool
          where requires config.other <= config.limit, ensures result {helper(config)}
        fn run()->bool {outer(CONFIG)}";
    let package = compile_with_protocols(source, &Default::default())
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn staged_field_relations_survive_closure_capture_and_invocation() {
    for body in [
        "checked(config.other,config.limit)",
        "do {action();checked(config.other,config.limit)}",
        "do {let inner=|| checked(config.other,config.limit);inner()}",
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            struct Config {{limit:Byte,other:Byte}}
            const CONFIG:Config=Config {{limit:19,other:7}};
            fn checked(value:Byte,limit:Byte)->result:bool
              where requires value <= limit, ensures result {{value <= limit}}
            fn outer(comptime config:Config,action:fn())->bool
              where requires config.other <= config.limit {{
                let check=|| {body}; check()
              }}
            fn noop() {{}}
            fn run()->bool {{outer(CONFIG,noop)}}"
        );
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        let invalid = source.replace(
            "checked(config.other,config.limit)",
            "checked(config.limit,config.other)",
        );
        let error = compile_with_protocols(&invalid, &Default::default())
            .err()
            .expect("capturing a premise cannot reverse its inequality");
        assert!(error.contains("precondition"), "{error}");
    }
}

#[test]
fn mutable_runtime_capture_does_not_inherit_staged_field_stability() {
    let source = "typealias Byte=.u8'builtin;
        struct Config {limit:Byte,other:Byte}
        fn checked(value:Byte,limit:Byte)->bool where requires value <= limit {value <= limit}
        fn outer(comptime config:Config)->bool
          where requires config.other <= config.limit {
            var copy:Config=Config {limit:config.limit,other:config.other};
            let check=|| do {copy.other=255;checked(copy.other,copy.limit)};
            check()
          }";
    let error = compile_with_protocols(source, &Default::default())
        .err()
        .expect("mutation of a runtime copy must invalidate staged field evidence");
    assert!(error.contains("precondition"), "{error}");
}

#[test]
fn staged_enum_contracts_keep_variant_payloads_separate() {
    for initial in ["Config.none", "Config.pair(7,19)", "Config.single(19)"] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            enum Config {{none,pair(Byte,Byte),single(Byte)}}
            const CONFIG:Config={initial};
            fn checked(value:Byte,limit:Byte)->bool where requires value <= limit {{value <= limit}}
            fn bounded(comptime config:Config)->bool
              where requires config match {{.none=>true,.pair(a,b)=>a <= b,.single(a)=>a <= 19}} {{
                config match {{.none=>true,.pair(a,b)=>checked(a,b),.single(a)=>checked(a,19)}}
              }}
            fn run()->bool {{bounded(CONFIG)}}"
        );
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        for invalid in [
            source.replace("checked(a,b)", "checked(b,a)"),
            source.replace("checked(a,19)", "checked(a,7)"),
            source.replace(
                ".none=>true,.pair(a,b)=>checked",
                ".none=>checked(255,19),.pair(a,b)=>checked",
            ),
        ] {
            let error = compile_with_protocols(&invalid, &Default::default())
                .err()
                .expect("each tag authorizes only its own payload premise");
            assert!(error.contains("precondition"), "{error}");
        }
    }
}

#[test]
fn single_variant_staged_payloads_retain_observation_identity() {
    let source = "typealias Byte=.u8'builtin;
        enum Config {pair(Byte,Byte)}
        const CONFIG:Config=Config.pair(7,19);
        fn checked(value:Byte,limit:Byte)->bool where requires value <= limit {value <= limit}
        fn bounded(comptime config:Config)->bool
          where requires config match {.pair(a,b)=>a <= b} {
            config match {.pair(a,b)=>checked(a,b)}
          }
        fn run()->bool {bounded(CONFIG)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let invalid = source.replace("checked(a,b)", "checked(b,a)");
    let error = compile_with_protocols(&invalid, &Default::default())
        .err()
        .unwrap();
    assert!(error.contains("precondition"), "{error}");
}

#[test]
fn conditional_staged_field_contracts_activate_only_in_the_selected_branch() {
    for initial in [
        "Config {enabled:true,value:7}",
        "Config {enabled:true,value:19}",
        "Config {enabled:false,value:255}",
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            struct Config {{enabled:bool,value:Byte}}
            const CONFIG:Config={initial};
            fn checked(value:Byte)->bool where requires value <= 19 {{value <= 19}}
            fn bounded(comptime config:Config)->bool
              where requires not config.enabled or config.value <= 19 {{
                if config.enabled {{checked(config.value)}} else {{true}}
              }}
            fn run()->bool {{bounded(CONFIG)}}"
        );
        let package = compile_with_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        let invalid = source.replace("if config.enabled", "if not config.enabled");
        let error = compile_with_protocols(&invalid, &Default::default())
            .err()
            .expect("the inactive branch has no bound on value");
        assert!(error.contains("precondition"), "{error}");
    }
}

#[test]
fn repeated_enum_matches_exclude_impossible_tags() {
    for initial in ["Config.none", "Config.some(12)"] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            enum Config {{none,some(Byte)}} const CONFIG:Config={initial};
            fn checked(value:Byte)->bool where requires value <= 19 {{value <= 19}}
            fn bounded(comptime config:Config)->bool {{
                config match {{
                    .none=>config match {{.none=>true,.some(_)=>checked(255)}},
                    .some(_)=>config match {{.none=>checked(255),.some(_)=>true}}
                }}
            }}
            fn run()->bool {{bounded(CONFIG)}}"
        );
        let package = compile_with_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        let invalid = source.replace(".none=>true", ".none=>checked(255)");
        let error = compile_with_protocols(&invalid, &Default::default())
            .err()
            .unwrap();
        assert!(error.contains("precondition"), "{error}");
    }
}

#[test]
fn enum_contract_fallback_excludes_preceding_variants() {
    for prefix in ["", ".none=>true,"] {
        for initial in ["Config.none", "Config.pair(7,19)", "Config.single(255)"] {
            let source=format!("typealias Byte=.u8'builtin;
                enum Config {{none,pair(Byte,Byte),single(Byte)}} const CONFIG:Config={initial};
                fn checked(value:Byte,limit:Byte)->bool where requires value <= limit {{value <= limit}}
                fn bounded(comptime config:Config)->bool
                  where requires config match {{{prefix}.pair(a,b)=>a <= b,_=>true}} {{
                    config match {{{prefix}.pair(a,b)=>checked(a,b),_=>true}}
                  }}
                fn run()->bool {{bounded(CONFIG)}}");
            let package = compile_with_protocols(&source, &Default::default()).unwrap();
            assert_eq!(
                luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            for invalid in [
                source.replace("checked(a,b)", "checked(b,a)"),
                source.replace(
                    "checked(a,b),_=>true",
                    "checked(a,b),_=>config match {.single(a)=>checked(a,19),_=>true}",
                ),
            ] {
                let error = compile_with_protocols(&invalid, &Default::default())
                    .err()
                    .unwrap();
                assert!(error.contains("precondition"), "{error}");
            }
        }
    }
}

#[test]
fn enum_owned_payload_contracts_preserve_nested_field_relations() {
    for (declarations, ty, initial, predicate, body, call, reversed) in [
        (
            "struct Pair {low:Byte,high:Byte}",
            "Pair",
            "Pair {low:7,high:19}",
            "p.low <= p.high",
            "checked(p.low,p.high)",
            "checked(p.low,p.high)",
            "checked(p.high,p.low)",
        ),
        (
            "struct Pair {low:Byte,high:Byte} struct Outer {pair:Pair}",
            "Outer",
            "Outer {pair:Pair {low:7,high:19}}",
            "p.pair.low <= p.pair.high",
            "checked(p.pair.low,p.pair.high)",
            "checked(p.pair.low,p.pair.high)",
            "checked(p.pair.high,p.pair.low)",
        ),
        (
            "",
            "Tuple<Byte,Byte>",
            "(7,19)",
            "p match {(low,high)=>low <= high}",
            "do {let (low,high)=p;checked(low,high)}",
            "checked(low,high)",
            "checked(high,low)",
        ),
    ] {
        for constructor in [format!("Config.pair({initial})"), "Config.none".to_owned()] {
            let source = format!(
                "typealias Byte=.u8'builtin;{declarations}
                enum Config {{none,pair({ty})}} const CONFIG:Config={constructor};
                fn checked(low:Byte,high:Byte)->bool where requires low <= high {{low <= high}}
                fn bounded(comptime config:Config)->bool
                  where requires config match {{.pair(p)=>{predicate},_=>true}} {{
                    config match {{.pair(p)=>{body},_=>true}}
                  }}
                fn run()->bool {{bounded(CONFIG)}}"
            );
            let package = support::compile_with_tuple_protocols(&source, &Default::default())
                .unwrap_or_else(|error| panic!("{source}: {error}"));
            assert_eq!(
                luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            let invalid = source.replace(call, reversed);
            let error = support::compile_with_tuple_protocols(&invalid, &Default::default())
                .err()
                .unwrap();
            assert!(error.contains("precondition"), "{error}");
        }
    }
}

#[test]
fn mutable_match_scrutinee_keeps_the_selected_value_snapshot() {
    let source = "typealias Byte=.u8'builtin;
        fn bounded(pair:Tuple<Byte,Byte>)->result:bool
          where requires pair match {(low,high)=>low <= high}, ensures result {
            var current=pair;
            current match {(low,high)=>do {current=(255,0);low <= high}}
          }
        fn run()->bool {bounded((7,19))}";
    let package = support::compile_with_tuple_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let invalid = source.replace(
        "current=(255,0);low <= high",
        "current=(255,0);current match {(a,b)=>a <= b}",
    );
    let error = support::compile_with_tuple_protocols(&invalid, &Default::default())
        .err()
        .unwrap();
    assert!(error.contains("postcondition"), "{error}");
}

#[test]
fn nested_enum_contracts_keep_both_tags_and_payload_ordering() {
    for initial in [
        "Config.none",
        "Config.some(Inner.none)",
        "Config.some(Inner.pair(7,19))",
    ] {
        let source=format!("typealias Byte=.u8'builtin;
            enum Inner {{none,pair(Byte,Byte)}} enum Config {{none,some(Inner)}}
            const CONFIG:Config={initial};
            fn checked(low:Byte,high:Byte)->bool where requires low <= high {{low <= high}}
            fn bounded(comptime config:Config)->bool
              where requires config match {{.some(inner)=>inner match {{.pair(a,b)=>a <= b,_=>true}},_=>true}} {{
                config match {{.some(inner)=>inner match {{.pair(a,b)=>checked(a,b),_=>true}},_=>true}}
              }}
            fn run()->bool {{bounded(CONFIG)}}");
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        let invalid = source.replace("checked(a,b)", "checked(b,a)");
        let error = compile_with_protocols(&invalid, &Default::default())
            .err()
            .unwrap();
        assert!(error.contains("precondition"), "{error}");
    }
}

#[test]
fn guarded_enum_contracts_follow_guard_failure_continuations() {
    let source = "typealias Byte=.u8'builtin;enum Config {none,pair(Byte,Byte)}
        const CONFIG:Config=Config.pair(7,19);
        fn checked(low:Byte,high:Byte)->bool where requires low <= high {low <= high}
        fn bounded(comptime config:Config)->bool
          where requires config match {.pair(a,b) if a <= b=>true,_=>false} {
            config match {.pair(a,b)=>checked(a,b),_=>true}
          }
        fn run()->bool {bounded(CONFIG)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let invalid = source.replace("checked(a,b)", "checked(b,a)");
    let error = compile_with_protocols(&invalid, &Default::default())
        .err()
        .unwrap();
    assert!(error.contains("precondition"), "{error}");
}

#[test]
fn overlapping_enum_guards_preserve_first_match_order() {
    for initial in ["Config.pair(7,19)", "Config.pair(19,19)"] {
        let source = format!(
            "typealias Byte=.u8'builtin;enum Config {{none,pair(Byte,Byte)}}
            const CONFIG:Config={initial};
            fn checked(low:Byte,high:Byte)->bool where requires low <= high {{low <= high}}
            fn bounded(comptime config:Config)->bool
              where requires config match {{
                .pair(a,b) if a==0=>false,
                .pair(a,b) if a <= b=>true,
                _=>false
              }} {{config match {{.pair(a,b)=>checked(a,b),_=>true}}}}
            fn run()->bool {{bounded(CONFIG)}}"
        );
        let package = compile_with_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        for rejected in ["Config.pair(0,19)", "Config.pair(23,19)", "Config.none"] {
            let invalid = source.replace(initial, rejected);
            let error = compile_with_protocols(&invalid, &Default::default())
                .err()
                .expect("a later true arm cannot override an earlier false result");
            assert!(error.contains("precondition"), "{error}");
        }
    }
}

#[test]
fn literal_enum_payloads_constrain_ordered_contract_branches() {
    let source = "typealias Byte=.u8'builtin;enum Config {none,pair(Byte,Byte)}
        const CONFIG:Config=Config.pair(7,19);
        fn checked(a:Byte,b:Byte)->bool where requires a <= b {a <= b}
        fn bounded(comptime config:Config)->bool
          where requires config match {.pair(0,b)=>false,.pair(a,b)=>a <= b,_=>false} {
            config match {.pair(a,b)=>checked(a,b),_=>true}
          }
        fn run()->bool {bounded(CONFIG)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    for rejected in ["Config.pair(0,19)", "Config.pair(23,19)", "Config.none"] {
        let invalid = source.replace("Config.pair(7,19)", rejected);
        let error = compile_with_protocols(&invalid, &Default::default())
            .err()
            .unwrap();
        assert!(error.contains("precondition"), "{error}");
    }
}

#[test]
fn boolean_enum_payload_patterns_select_contract_facts() {
    let source = "typealias Byte=.u8'builtin;enum Config {none,flag(bool,Byte)}
        const CONFIG:Config=Config.flag(true,7);
        fn checked(a:Byte)->bool where requires a <= 19 {a <= 19}
        fn bounded(comptime config:Config)->bool
          where requires config match {.flag(true,a)=>a <= 19,_=>false} {
            config match {.flag(_,a)=>checked(a),_=>true}
          }
        fn run()->bool {bounded(CONFIG)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    for rejected in [
        "Config.flag(false,7)",
        "Config.flag(true,23)",
        "Config.none",
    ] {
        let invalid = source.replace("Config.flag(true,7)", rejected);
        let error = compile_with_protocols(&invalid, &Default::default())
            .err()
            .expect("the constructor and boolean payload must both select the true arm");
        assert!(error.contains("precondition"), "{error}");
    }
    let invalid = source.replace("checked(a)", "checked(23)");
    let error = compile_with_protocols(&invalid, &Default::default())
        .err()
        .unwrap();
    assert!(error.contains("precondition"), "{error}");
}

#[test]
fn nested_payload_patterns_preserve_selection_and_field_relations() {
    for (declarations, ty, initial, zero, reversed, first, second, body) in [
        (
            "",
            "Tuple<Byte,Byte>",
            "(7,19)",
            "(0,19)",
            "(23,19)",
            "(0,_)",
            "(a,b)",
            "do {let (a,b)=p;checked(a,b)}",
        ),
        (
            "struct Pair {low:Byte,high:Byte}",
            "Pair",
            "Pair {low:7,high:19}",
            "Pair {low:0,high:19}",
            "Pair {low:23,high:19}",
            "{low:0,high:_}",
            "{low:a,high:b}",
            "checked(p.low,p.high)",
        ),
        (
            "enum Inner {none,pair(Byte,Byte)}",
            "Inner",
            "Inner.pair(7,19)",
            "Inner.pair(0,19)",
            "Inner.pair(23,19)",
            ".pair(0,_)",
            ".pair(a,b)",
            "p match {.pair(a,b)=>checked(a,b),_=>true}",
        ),
    ] {
        let source = format!("typealias Byte=.u8'builtin;{declarations}
            enum Config {{none,some({ty})}} const CONFIG:Config=Config.some({initial});
            fn checked(a:Byte,b:Byte)->bool where requires a <= b {{a <= b}}
            fn bounded(comptime config:Config)->bool
              where requires config match {{.some({first})=>false,.some({second})=>a <= b,_=>false}} {{
                config match {{.some(p)=>{body},_=>true}}
              }}
            fn run()->bool {{bounded(CONFIG)}}");
        let package = support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        if ty == "Inner" {
            let invalid = source.replace(initial, "Inner.none");
            let error = support::compile_with_tuple_protocols(&invalid, &Default::default())
                .err()
                .unwrap();
            assert!(error.contains("precondition"), "{invalid}: {error}");
        }
        for invalid in [
            source.replace(initial, zero),
            source.replace(initial, reversed),
            source.replace(&format!("Config.some({initial})"), "Config.none"),
            source
                .replace("checked(a,b)", "checked(b,a)")
                .replace("checked(p.low,p.high)", "checked(p.high,p.low)"),
        ] {
            let error = support::compile_with_tuple_protocols(&invalid, &Default::default())
                .err()
                .expect("nested pattern constraints must be enforced");
            assert!(error.contains("precondition"), "{invalid}: {error}");
        }
    }
}

#[test]
fn structural_match_contracts_compose_ordered_patterns_without_enum_wrappers() {
    for (declarations, ty, initial, first, second, body) in [
        (
            "",
            "Tuple<Byte,Byte>",
            "(7,19)",
            "(0,_)",
            "(a,b)",
            "do {let (a,b)=config;checked(a,b)}",
        ),
        (
            "struct Pair {low:Byte,high:Byte}",
            "Pair",
            "Pair {low:7,high:19}",
            "{low:0,high:_}",
            "{low:a,high:b}",
            "checked(config.low,config.high)",
        ),
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;{declarations}
            const CONFIG:{ty} = {initial};
            fn checked(a:Byte,b:Byte)->bool where requires a <= b {{a <= b}}
            fn bounded(comptime config:{ty})->bool
              where requires config match {{{first}=>false,{second}=>a <= b}} {{{body}}}
            fn run()->bool {{bounded(CONFIG)}}"
        );
        let package = support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        for invalid in [
            source.replace("7,19", "0,19").replace("low:7", "low:0"),
            source.replace("7,19", "23,19").replace("low:7", "low:23"),
            source.replace("checked(a,b)", "checked(b,a)").replace(
                "checked(config.low,config.high)",
                "checked(config.high,config.low)",
            ),
        ] {
            let error = support::compile_with_tuple_protocols(&invalid, &Default::default())
                .err()
                .unwrap();
            assert!(error.contains("precondition"), "{invalid}: {error}");
        }
    }
}

#[test]
fn scalar_match_contracts_keep_literal_and_binding_arm_conditions() {
    let source = "typealias Byte=.u8'builtin;
        fn checked(value:Byte)->bool where requires value <= 19 {value <= 19}
        fn bounded(value:Byte)->bool
          where requires value match {0=>false,x=>x <= 19} {checked(value)}
        fn run()->bool {bounded(7)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    for invalid in [
        source.replace("bounded(7)", "bounded(0)"),
        source.replace("bounded(7)", "bounded(23)"),
        source.replace("checked(value)", "checked(23)"),
    ] {
        let error = compile_with_protocols(&invalid, &Default::default())
            .err()
            .unwrap();
        assert!(error.contains("precondition"), "{error}");
    }
}

#[test]
fn alternative_patterns_preserve_ordered_contract_selection() {
    for pattern in ["0 or 1", "(0 or 1) as excluded"] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            fn checked(value:Byte)->bool where requires value <= 19 {{value <= 19}}
            fn bounded(value:Byte)->bool
              where requires value match {{{pattern}=>false,x=>x <= 19}} {{checked(value)}}
            fn run()->bool {{bounded(7)}}"
        );
        let package = compile_with_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        for rejected in [0, 1, 23] {
            let invalid = source.replace("bounded(7)", &format!("bounded({rejected})"));
            let error = compile_with_protocols(&invalid, &Default::default())
                .err()
                .unwrap();
            assert!(error.contains("precondition"), "{error}");
        }
    }
}

#[test]
fn nested_alternative_patterns_preserve_payload_constraints() {
    let source = "typealias Byte=.u8'builtin;enum Config {none,pair(Byte,Byte)}
        const CONFIG:Config=Config.pair(7,19);
        fn checked(a:Byte,b:Byte)->bool where requires a <= b {a <= b}
        fn bounded(comptime config:Config)->bool
          where requires config match {.pair(0 or 1,_)=>false,.pair(a,b)=>a <= b,_=>false} {
            config match {.pair(a,b)=>checked(a,b),_=>true}
          }
        fn run()->bool {bounded(CONFIG)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    for rejected in [
        "Config.pair(0,19)",
        "Config.pair(1,19)",
        "Config.pair(23,19)",
        "Config.none",
    ] {
        let invalid = source.replace("Config.pair(7,19)", rejected);
        let error = compile_with_protocols(&invalid, &Default::default())
            .err()
            .unwrap();
        assert!(error.contains("precondition"), "{error}");
    }
}

#[test]
fn alternative_pattern_binders_keep_common_field_identity() {
    let source = "typealias Byte=.u8'builtin;
        fn checked(value:Byte)->bool where requires value <= 19 {value <= 19}
        fn bounded(pair:Tuple<Byte,Byte>)->bool
          where requires pair match {(0,x) or (1,x)=>x <= 19,_=>false} {
            let (_,value)=pair;checked(value)
          }
        fn run()->bool {bounded((1,7))}";
    for initial in ["(0,7)", "(1,7)", "(1,19)"] {
        let accepted = source.replace("(1,7)", initial);
        let package =
            support::compile_with_tuple_protocols(&accepted, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    for rejected in ["(2,7)", "(0,23)", "(1,23)"] {
        let invalid = source.replace("(1,7)", rejected);
        let error = support::compile_with_tuple_protocols(&invalid, &Default::default())
            .err()
            .unwrap();
        assert!(error.contains("precondition"), "{error}");
    }
}

#[test]
fn alternative_bindings_from_different_fields_keep_selected_relations() {
    let source = "typealias Byte=.u8'builtin;
        fn checked(value:Byte)->bool where requires value <= 19 {value <= 19}
        fn bounded(input:Tuple<bool,Byte,Byte>)->bool
          where requires input match {(true,x,_) or (false,_,x)=>x <= 19} {
            let (flag,left,right)=input;
            if flag {checked(left)} else {checked(right)}
          }
        fn run()->bool {bounded((true,7,255))}";
    for initial in ["(true,7,255)", "(false,255,7)"] {
        let accepted = source.replace("(true,7,255)", initial);
        let package =
            support::compile_with_tuple_protocols(&accepted, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    for invalid in [
        source.replace("(true,7,255)", "(true,23,7)"),
        source.replace("(true,7,255)", "(false,7,23)"),
        source.replace(
            "if flag {checked(left)} else {checked(right)}",
            "if flag {checked(right)} else {checked(left)}",
        ),
    ] {
        let error = support::compile_with_tuple_protocols(&invalid, &Default::default())
            .err()
            .unwrap();
        assert!(error.contains("precondition"), "{error}");
    }
}

#[test]
fn outer_guard_failure_does_not_retry_an_alternative_binding() {
    let source = "typealias Byte=.u8'builtin;
        fn bounded(input:Tuple<Byte,Byte>)->bool
          where requires input match {(x,_) or (_,x) if x <= 19=>true,_=>false} {true}
        fn run()->bool {bounded((7,23))}";
    let package = support::compile_with_tuple_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let invalid = source.replace("(7,23)", "(23,7)");
    let error = support::compile_with_tuple_protocols(&invalid, &Default::default())
        .err()
        .unwrap();
    assert!(error.contains("precondition"), "{error}");
}

#[test]
fn guarded_alternatives_keep_selected_field_bounds() {
    let source = "typealias Byte=.u8'builtin;
        fn checked(value:Byte)->bool where requires value <= 19 {value <= 19}
        fn bounded(input:Tuple<bool,Byte,Byte>)->bool
          where requires input match {(true,x,_) or (false,_,x) if x <= 19=>true,_=>false} {
            let (flag,left,right)=input;
            if flag {checked(left)} else {checked(right)}
          }
        fn run()->bool {bounded((true,7,255))}";
    for initial in ["(true,7,255)", "(false,255,7)"] {
        let accepted = source.replace("(true,7,255)", initial);
        let package =
            support::compile_with_tuple_protocols(&accepted, &Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    for invalid in [
        source.replace("(true,7,255)", "(true,23,7)"),
        source.replace("(true,7,255)", "(false,7,23)"),
        source.replace(
            "if flag {checked(left)} else {checked(right)}",
            "if flag {checked(right)} else {checked(left)}",
        ),
    ] {
        let error = support::compile_with_tuple_protocols(&invalid, &Default::default())
            .err()
            .unwrap();
        assert!(error.contains("precondition"), "{error}");
    }
}

#[test]
fn alternative_guard_calls_reach_fallback_exactly_once() {
    for (pattern, value) in [("(x,_) or (_,x)", "(23,7)"), ("(0,x) or (1,x)", "(1,23)")] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            fn gate(value:Byte)->bool {{value <= 19}}
            fn run()->bool {{
                let input:Tuple<Byte,Byte> = {value};
                input match {{({pattern}) if gate(x)=>true,_=>false}}
            }}"
        );
        let package = support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            comptime_interpretor::Value::Bool(false)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| *name == "gate")
                .count(),
            1
        );
    }
}

#[test]
fn alternative_guard_mutations_reach_fallback_without_retry() {
    for (pattern, value) in [("(x,_) or (_,x)", "(23,7)"), ("(0,x) or (1,x)", "(1,23)")] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            fn once(count:Byte)->bool where requires count==1 {{true}}
            fn run()->bool {{
                var count:Byte=0;
                let input:Tuple<Byte,Byte> = {value};
                let accepted=input match {{
                    ({pattern}) if do {{count=count + 1;x <= 19}}=>true,
                    _=>false
                }};
                once(count) and not accepted
            }}"
        );
        let package = support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        let invalid = source.replace("requires count==1", "requires count==2");
        let error = support::compile_with_tuple_protocols(&invalid, &Default::default())
            .err()
            .unwrap();
        assert!(error.contains("precondition"), "{error}");
    }
}

#[test]
fn guard_failure_uses_original_subject_and_updated_outer_state() {
    let source = "typealias Byte=.u8'builtin;
        fn once(count:Byte)->bool where requires count==1 {true}
        fn run()->bool {
            var count:Byte=0;
            var input:Tuple<Byte,Byte> = (23,7);
            let accepted=input match {
                ((x,_) or (_,x)) if do {count=count + 1;input=(0,0);x <= 19}=>true,
                (0,_)=>true,
                _=>false
            };
            let (left,right)=input;
            once(count) and not accepted and left==0 and right==0
        }";
    let package = support::compile_with_tuple_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let invalid = source.replace("requires count==1", "requires count==2");
    let error = support::compile_with_tuple_protocols(&invalid, &Default::default())
        .err()
        .unwrap();
    assert!(error.contains("precondition"), "{error}");
}
