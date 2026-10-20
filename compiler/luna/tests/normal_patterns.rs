use comptime_interpretor::Value;
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{compile, compile_with_protocols};

#[test]
fn wrapped_type_patterns_form_later_normal_value_domains() {
    let prefix = "typealias Type=.Type'builtin; typealias Bool=bool; typealias Byte=.u8'builtin; typealias Alias=Bool;
        pure comptime fn Box<T,value:T>->Type {T}";
    for (pattern, expected) in [
        ("('Bool as Kind)", true),
        ("('Bool if true)", true),
        ("('Bool or 'Alias)", true),
        ("('Bool if false)", false),
        ("('Bool and Byte is _)", true),
        ("('Bool and false is true)", false),
        ("(Kind and Kind is 'Bool)", true),
    ] {
        let body = if pattern.contains("Kind") {
            "Kind == Bool and item"
        } else {
            "item"
        };
        let source=format!("{prefix}
            comptime fn Inspect(value:Type)->Bool {{value match {{Box<{pattern},item> =>{body},_=>false}}}}
            comptime fn Run()->Bool {{Inspect(Box<Bool,true>)}}
            comptime fn Miss()->Bool {{Inspect(Box<Byte,9>)}}");
        let package = compile(&source);
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            Value::Bool(expected),
            "{pattern}"
        );
        assert_eq!(
            luna::invocation::invoke(&package, "Miss", 100_000).unwrap(),
            Value::Bool(false),
            "{pattern}"
        );
    }
}

#[test]
fn normal_pattern_type_evidence_does_not_cross_alternatives() {
    let prefix = "typealias Type=.Type'builtin; typealias Bool=bool; typealias Byte=.u8'builtin;
        pure comptime fn Identity<X>->Type {X} typealias Opaque=Identity<Bool>;
        pure comptime fn Box<T,value:T>->Type {T}";
    for arms in [
        "Box<('Bool or 'Byte),item> =>item,_=>false",
        "Box<('Bool or 'Opaque),item> =>item,_=>false",
        "Box<('Bool if false),item> =>item,Box<T,item> =>false,_=>false",
        "Box<(_ if Bool == Bool),item> =>item,_=>false",
        "Box<(_ and Bool is 'Bool),item> =>item,_=>false",
        "Box<(Kind and Bool is 'Bool),item> =>item,_=>false",
    ] {
        let failure = compile_with_protocols(
            &format!("{prefix} comptime fn Bad(value:Type)->Bool {{value match {{ {arms} }} }}"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            failure.contains("dependent normal-form value patterns"),
            "{failure}"
        );
    }
}

#[test]
fn chained_type_observation_retains_generic_identity_and_fallback() {
    let package = compile(
        "typealias Type=.Type'builtin; typealias Byte=.u8'builtin;
        pure comptime fn Box<T,value:T>->Type {T}
        comptime fn Contents(value:Type,fallback:T)->T where T {
            value match {Box<(Kind and Kind is 'T),item> => item,_ => fallback}
        }
        comptime fn Run()->bool {Contents<bool>(Box<bool,true>,false)}
        comptime fn Miss()->bool {Contents<bool>(Box<Byte,9>,false)}
        comptime fn Number()->Byte {Contents<Byte>(Box<Byte,9>,7)}",
    );
    for (entry, expected) in [
        ("Run", Value::Bool(true)),
        ("Miss", Value::Bool(false)),
        ("Number", Value::UnsignedInteger(9)),
    ] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            expected
        );
    }
}

#[test]
fn dependent_type_pattern_guards_preserve_effects_and_candidate_order() {
    let package = compile(
        "typealias Type=.Type'builtin; typealias Bool=bool; typealias Int=.Integer'builtin;
        pure comptime fn Box<T,value:T>->Type {T}
        comptime fn Tick(counter:*Int,result:Bool)->Bool {counter.* = counter.* + 1;result}
        comptime fn Probe(value:Type)->Int {
            var count:Int=0;
            let matched:Int=value match {
                Box<('Bool if Tick(count.ref,false)),item> => if item {100} else {101},
                Box<('Bool and Tick(count.ref,true) is true),item> => if item {10} else {20},
                _ => 30,
            };
            count + matched
        }
        comptime fn Hit()->Int {Probe(Box<Bool,true>)}
        comptime fn OtherValue()->Int {Probe(Box<Bool,false>)}
        comptime fn WrongType()->Int {Probe(Box<Int,9>)}
        comptime fn WrongHead()->Int {Probe(Bool)}",
    );
    for (entry, expected) in [
        ("Hit", 12),
        ("OtherValue", 22),
        ("WrongType", 30),
        ("WrongHead", 30),
    ] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            Value::Integer(comptime_runtime::Integer::from_u128(expected)),
            "{entry}"
        );
    }
}

#[test]
fn observed_normal_form_arguments_contextualize_dependent_patterns() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn Box<T,value:T>->Type {T}
        pure comptime fn Wrap<T>->Type {T}
        comptime fn Direct()->bool {
            Box<bool,true> match {Box<Kind,item> => Kind == bool and item,_ => false}
        }
        comptime fn Nested()->bool {
            Wrap<Box<bool,true>> match {Wrap<Box<Kind,item>> => Kind == bool and item,_ => false}
        }
        comptime fn Binding()->bool {
            let Box<Kind,item> = Box<bool,true> else {return false;};
            Kind == bool and item
        }
        comptime fn Chained()->bool {
            true match {
                _ and Box<bool,true> is Box<Kind,item> => Kind == bool and item,
                _ => false
            }
        }
        comptime fn Named()->bool {
            Box<bool,true> match {Box<value: item,T: Kind> => Kind == bool and item,_ => false}
        }";
    let package = compile(source);
    for entry in ["Direct", "Nested", "Binding", "Chained", "Named"] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            Value::Bool(true),
            "{entry}"
        );
    }
}

#[test]
fn observed_normal_patterns_do_not_unfold_other_heads() {
    let prefix = "typealias Type=.Type'builtin;
        pure comptime fn Box<T,value:T>->Type {T}
        pure comptime fn Other<T,value:T>->Type {T}
        pure comptime fn Identity<T>->Type {T}";
    for value in ["Other<bool,true>", "Identity<Box<bool,true>>"] {
        let failure=compile_with_protocols(&format!("{prefix} comptime fn Bad()->bool {{ {value} match {{Box<Kind,item> => item,_ => false}} }}"),&Default::default()).unwrap_err();
        assert!(
            failure.contains("dependent normal-form value patterns"),
            "{failure}"
        );
    }
}

#[test]
fn tuple_type_patterns_preserve_observed_dependent_domains() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn Box<T,value:T>->Type {T}
        pure comptime fn Wrap<T>->Type {T}
        comptime fn Direct()->bool {
            Wrap<(Box<bool,true>,bool)> match {
                Wrap<(Box<Kind,item>,_)> => Kind == bool and item,
                _ => false
            }
        }
        comptime fn Nested()->bool {
            Wrap<((bool,Box<bool,true>),bool)> match {
                Wrap<((_,Box<Kind,item>),_)> => Kind == bool and item,
                _ => false
            }
        }";
    let structural = compile(source);
    let bound = execution_support::compile_with_source_bindings(
        &format!("{source} pure comptime fn List<T>->Type {{T}} pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}"),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    ).unwrap();
    for package in [&structural, &bound] {
        for entry in ["Direct", "Nested"] {
            assert_eq!(
                luna::invocation::invoke(package, entry, 100_000).unwrap(),
                Value::Bool(true)
            );
        }
    }
    let failure=compile_with_protocols("typealias Type=.Type'builtin; pure comptime fn Box<T,value:T>->Type {T} pure comptime fn Wrap<T>->Type {T} comptime fn Bad()->bool {Wrap<(Box<bool,true>,bool)> match {Wrap<(Box<Kind,item>,_,_)> => item,_ => false}}", &Default::default()).unwrap_err();
    assert!(
        failure.contains("dependent normal-form value patterns"),
        "{failure}"
    );
}

#[test]
fn normal_arguments_share_only_successful_prior_candidate_bindings() {
    for pattern in ["Pair<A,(B if A == B)>", "Pair<Left:A,Right:(B if A == B)>"] {
        let package = compile(&format!(
            "typealias Type=.Type'builtin;
            pure comptime fn Pair<Left,Right>->Type {{Left}}
            comptime fn Inspect(value:Type)->bool {{
                value match {{{pattern} => A == bool,_ => false}}
            }}
            comptime fn Hit()->bool {{Inspect(Pair<bool,bool>)}}
            comptime fn Miss()->bool {{Inspect(Pair<bool,.u8'builtin>)}}
            comptime fn Opaque()->bool {{Inspect(bool)}}"
        ));
        for (entry, expected) in [("Hit", true), ("Miss", false), ("Opaque", false)] {
            assert_eq!(
                luna::invocation::invoke(&package, entry, 100_000).unwrap(),
                Value::Bool(expected),
                "{pattern}: {entry}"
            );
        }
    }
}

#[test]
fn normal_argument_candidates_reject_forward_and_escaped_references() {
    for arms in [
        "Pair<(A if A == B),B> => true,_ => false",
        "Pair<A,(B if A == B)> => true,_ => A == bool",
        "Pair<Right:B,Left:(A if A == B)> => true,_ => false",
    ] {
        let failure = compile_with_protocols(
            &format!(
                "typealias Type=.Type'builtin;
                pure comptime fn Pair<Left,Right>->Type {{Left}}
                comptime fn Inspect(value:Type)->bool {{value match {{{arms}}}}}"
            ),
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            failure.contains("HIR lowering") || failure.contains("elaboration failed"),
            "{arms}: {failure}"
        );
    }
}

#[test]
fn normal_value_arguments_and_nested_heads_preserve_candidate_order() {
    let package = compile(
        "typealias Type=.Type'builtin;
        pure comptime fn Flags<first:bool,second:bool>->Type {bool}
        pure comptime fn Wrap<T>->Type {T}
        comptime fn Inspect(value:Type)->bool {
            value match {
                Wrap<Flags<a,(b if a == b)>> => a,
                Flags<a,(b if a == b)> => a,
                Flags<a,b> => not a and b,
                _ => false,
            }
        }
        comptime fn Equal()->bool {Inspect(Flags<true,true>)}
        comptime fn Unequal()->bool {Inspect(Flags<false,true>)}
        comptime fn Reject()->bool {Inspect(Flags<true,false>)}
        comptime fn Nested()->bool {Inspect(Wrap<Flags<true,true>>)}
        comptime fn NestedMiss()->bool {Inspect(Wrap<Flags<true,false>>)}",
    );
    for (entry, expected) in [
        ("Equal", true),
        ("Unequal", true),
        ("Reject", false),
        ("Nested", true),
        ("NestedMiss", false),
    ] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            Value::Bool(expected),
            "{entry}"
        );
    }
}

#[test]
fn normal_value_prefixes_form_later_indexed_domains() {
    let source = "typealias Type=.Type'builtin;
        struct Config where enabled:bool {flag:bool}
        pure comptime fn Packet<enabled:bool,value:Config<enabled>>->Type {bool}
        comptime fn Inspect(value:Type)->bool {value match {
            Packet<true,item> => item.flag,
            _ => false
        }}
        comptime fn Run()->bool {Inspect(Packet<true,Config<true>{flag:true}>)}";
    let package = compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for pattern in ["(true as selected)", "(true if true)", "(true or true)"] {
        let wrapped = source.replace("Packet<true,item>", &format!("Packet<{pattern},item>"));
        let package = compile(&wrapped);
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    let integer = source
        .replace("enabled:bool", "enabled:.u8'builtin")
        .replace("Packet<true,item>", "Packet<7,item>")
        .replace("Packet<true,Config<true>", "Packet<7,Config<7>");
    assert_eq!(
        luna::invocation::invoke(&compile(&integer), "Run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let observed = source.replace("comptime fn Run()->bool {Inspect(Packet<true,Config<true>{flag:true}>)}", "comptime fn Run()->bool {Packet<true,Config<true>{flag:true}> match {Packet<enabled,item> => enabled and item.flag,_ => false}}");
    assert_eq!(
        luna::invocation::invoke(&compile(&observed), "Run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let miss = source.replace(
        "Packet<true,Config<true>{flag:true}>",
        "Packet<false,Config<false>{flag:true}>",
    );
    assert_eq!(
        luna::invocation::invoke(&compile(&miss), "Run", 100_000).unwrap(),
        Value::Bool(false)
    );
    let named = source
        .replace(
            "Packet<enabled:bool,value:Config<enabled>>",
            "Packet<.enabled:bool=true,value:Config<enabled>>",
        )
        .replace(
            "Packet<true,Config<true>{flag:true}>",
            "Packet<Config<true>{flag:true},.enabled=true>",
        );
    assert_eq!(
        luna::invocation::invoke(
            &compile_with_protocols(&named, &Default::default())
                .unwrap_or_else(|e| panic!("{named}\n{e}")),
            "Run",
            100_000
        )
        .unwrap(),
        Value::Bool(true)
    );
    let defaulted = named.replace(",.enabled=true>", ">");
    assert_eq!(
        luna::invocation::invoke(&compile(&defaulted), "Run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let dynamic = source
        .replace(
            "comptime fn Run()",
            "comptime fn enabled()->bool {true} comptime fn Run()",
        )
        .replace(
            "Packet<true,Config<true>{flag:true}>",
            "Packet<enabled(),Config<true>{flag:true}>",
        );
    let package = compile(&dynamic);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "Run")
        .unwrap()
        .declaration;
    assert_eq!(vm.invoke(entry, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(
        execution_support::calls(&package, &vm),
        vec!["Run", "enabled", "Inspect"]
    );
    let ambiguous = source.replace("Packet<true,item>", "Packet<(true or false),item>");
    assert!(compile_with_protocols(&ambiguous, &Default::default()).is_err());
    let wrong = source.replace(
        "Packet<true,Config<true>{flag:true}>",
        "Packet<true,Config<false>{flag:true}>",
    );
    assert!(compile_with_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn normal_optional_record_defaults_preserve_dependent_instances() {
    let source = "typealias Type=.Type'builtin;
        struct Config where enabled:bool {flag:bool}
        pure comptime fn Packet<.enabled:bool=true,.value:Config<enabled> = defer Config<enabled>{flag:enabled}>->Type {bool}
        comptime fn Run()->bool {Packet<> match {Packet<true,item> => item.flag,_=>false}}
        comptime fn Disabled()->bool {Packet<.enabled=false> match {Packet<false,item> => not item.flag,_=>false}}
        comptime fn Override()->bool {Packet<.value=Config<true>{flag:false}> match {Packet<true,item> => not item.flag,_=>false}}";
    let package = compile(source);
    for entry in ["Run", "Disabled", "Override"] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            Value::Bool(true),
            "{entry}"
        );
    }
    let wrong = source.replace(
        ".value=Config<true>{flag:false}",
        ".value=Config<false>{flag:false}",
    );
    let package = compile(&wrong);
    assert!(luna::invocation::invoke(&package, "Override", 100_000).is_err());
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_optional_defaults_construct_records_and_respect_overrides() {
    let package = compile(
        "struct Config {flag:bool}
        comptime fn read(.config:Config=Config {flag:true})->bool {config.flag}
        comptime fn Run()->bool {read() and not read(.config=Config {flag:false})}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn normal_record_defaults_follow_evaluated_value_prefixes_once() {
    let package = compile("typealias Type=.Type'builtin;
        struct Config where enabled:bool {flag:bool}
        comptime fn enabled()->bool {true}
        pure comptime fn Packet<.enabled:bool=true,.value:Config<enabled> = defer Config<enabled>{flag:enabled}>->Type {bool}
        comptime fn Run()->bool {Packet<.enabled=enabled()> match {Packet<true,item> => item.flag,_=>false}}");
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "Run")
        .unwrap()
        .declaration;
    assert_eq!(vm.invoke(entry, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(
        execution_support::calls(&package, &vm),
        vec!["Run", "enabled"]
    );
}
