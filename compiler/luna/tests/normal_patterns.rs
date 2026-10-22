use comptime_interpretor::Value;
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{compile, compile_with_protocols};

#[test]
fn nested_member_defaults_use_selected_parent_type_and_value_inputs() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        newtype Indexed<T,value:T> = bool;
        struct Outer where T,value:T {
            pub struct Inner where .other:T=value {
                pub typealias Result<.copy:T=other> = Indexed<T,copy>;
            }
        }
        comptime fn Build(value:Byte)->Type {Outer<Byte,value>.Inner<>.Result}
        comptime fn Seven()->Type {Build(7)}
        comptime fn Nine()->Type {Build(9)}
        comptime fn ExpectedSeven()->Type {Indexed<Byte,7>}
        comptime fn ExpectedNine()->Type {Indexed<Byte,9>}";
    let package = compile(source);
    for (entry, expected) in [("Seven", "ExpectedSeven"), ("Nine", "ExpectedNine")] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            luna::invocation::invoke(&package, expected, 100_000).unwrap()
        );
    }
    assert!(
        compile_with_protocols(
            &source.replace(".Inner<>.Result", ".Inner<.other=false>.Result"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn dynamic_nominal_member_projection_preserves_arithmetic_checks() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        struct Holder where key:Byte {pub typealias Payload=bool;}
        typealias Alias<key:Byte> = Holder<key>;
        comptime fn Build(key:Byte)->Type {
            if key < 255 {Alias<key + 1>.Payload} else {.char'builtin}
        }
        comptime fn Hit()->Type {Build(254)}
        comptime fn Boundary()->Type {Build(255)}";
    for source in [
        source.to_owned(),
        source.replace("struct Holder where key:Byte {", "enum Holder where key:Byte {empty,"),
        source.replace("pub typealias Payload=bool;", "pub struct Inner where inner:Byte {pub typealias Payload=bool;}")
            .replace(".Payload}", ".Inner<key>.Payload}"),
        source.replace("struct Holder", "struct Other where outer:Byte,inner:Byte {pub typealias Payload=bool;} struct Holder")
            .replacen("struct Holder where key:Byte {pub typealias Payload=bool;}", "struct Holder where key:Byte {pub typealias Inner<inner:Byte> = Other<key,inner>;}", 1)
            .replace(".Payload}", ".Inner<key>.Payload}"),
    ] {
        let package = compile(&source);
        for (entry, expected) in [("Hit", nir::Primitive::Bool), ("Boundary", nir::Primitive::Char)] {
            assert_eq!(luna::invocation::invoke(&package,entry,100_000).unwrap(),
                Value::Type(nir::Term::Primitive(expected)));
        }
        let error = compile_with_protocols(&source.replace("key < 255", "true"), &Default::default()).unwrap_err();
        assert!(error.contains("machine arithmetic range is not proven"), "{error}");
    }
}

#[test]
fn dynamic_member_inputs_execute_once_before_checked_arithmetic() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        struct Holder where T {pub typealias Payload=T;}
        typealias Alias<T> = Holder<T>;
        comptime fn choose(count:*Byte)->Type {count.* = 254;bool}
        comptime fn Run()->Byte {
            var count:Byte=0;
            let selected=Alias<choose(count.ref)>.Payload;
            if count < 255 {count + 1} else {0}
        }",
    );
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "Run")
        .unwrap()
        .declaration;
    let choose = package
        .functions
        .iter()
        .find(|f| f.name == "choose")
        .unwrap()
        .declaration;
    assert_eq!(vm.invoke(run, vec![]).unwrap(), Value::UnsignedInteger(255));
    assert_eq!(vm.events().iter().filter(|event| matches!(event,comptime_interpretor::Event::EnterFunction(id) if *id == choose)).count(),1);
}

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

#[test]
fn boolean_type_guards_contextualize_normal_form_value_patterns() {
    for guard in [
        "if enabled {Kind == bool} else {bool == Kind}",
        "if Kind == bool {true} else {false}",
        "(Kind == bool) == true",
        "false == (Kind != bool)",
        "(Kind == bool) != false",
        "bool == Kind",
        "Kind == bool and enabled",
        "enabled and bool == Kind",
        "(Kind == bool and enabled) or (bool == Kind and not enabled)",
        "not (not (Kind == bool))",
        "Kind == bool or false",
        "not (Kind != bool or not enabled)",
        "not (Kind != bool and Kind != bool)",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            pure comptime fn Box<T,value:T>->Type {{T}}
            comptime fn Inspect(input:Type,enabled:bool)->bool {{
                input match {{Box<(Kind if {guard}),item> => item,_ => false}}
            }}
            comptime fn Hit()->bool {{Inspect(Box<bool,true>,true)}}
            comptime fn Miss()->bool {{Inspect(Box<.u8'builtin,9>,true)}}
            comptime fn DisabledMiss()->bool {{Inspect(Box<.u8'builtin,9>,false)}}"
        );
        let package = compile(&source);
        for (entry, expected) in [("Hit", true), ("Miss", false), ("DisabledMiss", false)] {
            assert_eq!(
                luna::invocation::invoke(&package, entry, 100_000).unwrap(),
                Value::Bool(expected),
                "{guard}"
            );
        }
    }
}

#[test]
fn type_guard_evidence_rejects_alternative_domains_and_effectful_observation() {
    for guard in [
        "if enabled {Kind == bool} else {Kind == .u8'builtin}",
        "if enabled {Kind == bool} else {true}",
        "(Kind == bool) == enabled",
        "(Kind == bool) == (Kind == bool)",
        "Kind == bool or enabled",
        "Kind == bool or Kind == .u8'builtin",
        "not (Kind == bool)",
        "Kind == bool and unknown()",
        "do {Kind=bool;true} and Kind == bool",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            pure comptime fn Box<T,value:T>->Type {{T}}
            comptime fn unknown()->bool {{unknown()}}
            comptime fn Inspect(input:Type,enabled:bool)->bool {{
                input match {{Box<(Kind if {guard}),item> => item,_ => false}}
            }}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("dependent normal-form value patterns"),
            "{guard}: {error}"
        );
    }
}

#[test]
fn boolean_value_guards_form_dependent_normal_parameter_domains() {
    for (guard, selected) in [
        ("if enabled {flag} else {false}", true),
        ("if flag {false} else {true}", false),
        ("flag", true),
        ("not flag", false),
        ("flag == true", true),
        ("false == flag", false),
        ("flag and enabled", true),
        ("not (flag or not enabled)", false),
        ("(flag and enabled) or (flag and not enabled)", true),
    ] {
        let source=format!("typealias Type=.Type'builtin;
            struct Config where enabled:bool {{value:bool}}
            pure comptime fn Packet<enabled:bool,value:Config<enabled>>->Type {{bool}}
            pure comptime fn accepts(value:Config<{selected}>)->bool {{value.value}}
            comptime fn Inspect(input:Type,enabled:bool)->bool {{
                input match {{Packet<(flag if {guard}),item> => accepts(item),_=>false}}
            }}
            comptime fn Hit()->bool {{Inspect(Packet<{selected},Config<{selected}>{{value:true}}>,true)}}
            comptime fn Miss()->bool {{Inspect(Packet<{},Config<{}>{{value:true}}>,true)}}",!selected,!selected);
        let package = compile(&source);
        for (entry, expected) in [("Hit", true), ("Miss", false)] {
            assert_eq!(
                luna::invocation::invoke(&package, entry, 100_000).unwrap(),
                Value::Bool(expected),
                "{guard}"
            );
        }
    }
}

#[test]
fn boolean_value_guards_do_not_guess_domains_or_invoke_code() {
    for guard in [
        "if enabled {flag} else {not flag}",
        "if enabled {flag} else {true}",
        "enabled",
        "flag or enabled",
        "flag == enabled",
        "not (flag and false)",
        "flag and not flag",
        "flag and unknown()",
        "do {flag=true;true}",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            struct Config where enabled:bool {{value:bool}}
            pure comptime fn Packet<enabled:bool,value:Config<enabled>>->Type {{bool}}
            comptime fn unknown()->bool {{unknown()}}
            comptime fn Inspect(input:Type,enabled:bool)->bool {{
                input match {{Packet<(flag if {guard}),item> => item.value,_=>false}}
            }}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("dependent normal-form value patterns"),
            "{guard}: {error}"
        );
    }
}

#[test]
fn exact_scalar_guards_form_dependent_normal_parameter_domains() {
    for (domain, value, other) in [
        (".u8'builtin", "7", "8"),
        (".i16'builtin", "-7", "8"),
        (".Integer'builtin", "700", "800"),
        (".char'builtin", "'a'", "'b'"),
        (".Symbol'builtin", ".ready", ".waiting"),
        (".str'builtin", "\"ready\"", "\"waiting\""),
    ] {
        for guard in [
            format!("if allowed {{ key == {value} }} else {{ {value} == key }}"),
            format!("(key == {value}) == true"),
            format!("false == (key != {value})"),
            format!("key == {value}"),
            format!("{value} == key"),
            format!("key == {value} and allowed"),
            format!("(key == {value} and allowed) or ({value} == key and not allowed)"),
        ] {
            let source=format!("typealias Type=.Type'builtin;
                struct Config where key:{domain} {{value:bool}}
                pure comptime fn Packet<key:{domain},value:Config<key>>->Type {{bool}}
                pure comptime fn accepts(value:Config<{value}>)->bool {{value.value}}
                comptime fn inspect(input:Type,allowed:bool)->bool {{
                    input match {{Packet<(key if {guard}),item> => accepts(item),_=>false}}
                }}
                comptime fn Hit()->bool {{inspect(Packet<{value},Config<{value}>{{value:true}}>,true)}}
                comptime fn Miss()->bool {{inspect(Packet<{other},Config<{other}>{{value:true}}>,true)}}");
            let package = compile(&source);
            for (entry, expected) in [("Hit", true), ("Miss", false)] {
                assert_eq!(
                    luna::invocation::invoke(&package, entry, 100_000).unwrap(),
                    Value::Bool(expected),
                    "{domain}: {guard}"
                );
            }
        }
    }
}

#[test]
fn scalar_guard_evidence_preserves_unknown_and_float_identity_boundaries() {
    for (domain, guard) in [
        (".u8'builtin", "if allowed {key == 7} else {key == 8}"),
        (".u8'builtin", "if allowed {key == 7} else {true}"),
        (".u8'builtin", "if allowed {key == 7} else {unknown()}"),
        (".u8'builtin", "(key == 7) == allowed"),
        (".u8'builtin", "(key == 7) == (key == 7)"),
        (".u8'builtin", "(key == 7) == false"),
        (".u8'builtin", "key == 7 or key == 8"),
        (".u8'builtin", "key != 7"),
        (".u8'builtin", "key == 7 or allowed"),
        (".u8'builtin", "key == 7 and unknown()"),
        (".u8'builtin", "do {key=7;true} and key == 7"),
        (".f32'builtin", "key == 0.0"),
        (".f64'builtin", "0.0 == key"),
        (".Float'builtin", "key == 0.0"),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            struct Config where key:{domain} {{value:bool}}
            pure comptime fn Packet<key:{domain},value:Config<key>>->Type {{bool}}
            comptime fn unknown()->bool {{unknown()}}
            comptime fn inspect(input:Type,allowed:bool)->bool {{
                input match {{Packet<(key if {guard}),item> => item.value,_=>false}}
            }}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("dependent normal-form value patterns"),
            "{domain}, {guard}: {error}"
        );
    }
}

#[test]
fn mixed_domain_guard_comparisons_preserve_existing_identity_evidence() {
    for (domain, condition, good, bad) in [
        (".Integer'builtin", "limit > 1", "2", "0"),
        (".u8'builtin", "limit == 2", "2", "0"),
        (".char'builtin", "limit == 'a'", "'a'", "'b'"),
        (".f32'builtin", "limit > 0.0", "1.0", "0.0"),
        (".Symbol'builtin", "limit == .ready", ".ready", ".waiting"),
        (
            ".str'builtin",
            "limit == \"ready\"",
            "\"ready\"",
            "\"waiting\"",
        ),
    ] {
        let source=format!("typealias Type=.Type'builtin;
            pure comptime fn Box<T,value:T>->Type {{T}}
            struct Config where flag:bool {{value:bool}}
            pure comptime fn Packet<flag:bool,value:Config<flag>>->Type {{bool}}
            struct Sized where key:.u8'builtin {{value:bool}}
            pure comptime fn Layout<key:.u8'builtin,value:Sized<key>>->Type {{bool}}
            comptime fn inspectType(input:Type,limit:{domain})->bool {{
                input match {{Box<(Kind if Kind == bool and {condition}),item> => item,_=>false}}
            }}
            comptime fn inspectValue(input:Type,limit:{domain})->bool {{
                input match {{Packet<(flag if flag and {condition}),item> => item.value,_=>false}}
            }}
            comptime fn inspectScalar(input:Type,limit:{domain})->bool {{
                input match {{Layout<(key if key == 7 and {condition}),item> => item.value,_=>false}}
            }}
            comptime fn Hit()->bool {{inspectType(Box<bool,true>,{good}) and inspectValue(Packet<true,Config<true>{{value:true}}>,{good}) and inspectScalar(Layout<7,Sized<7>{{value:true}}>,{good})}}
            comptime fn Miss()->bool {{inspectType(Box<bool,true>,{bad}) or inspectValue(Packet<true,Config<true>{{value:true}}>,{bad}) or inspectScalar(Layout<7,Sized<7>{{value:true}}>,{bad})}}");
        let package = compile(&source);
        for (entry, expected) in [("Hit", true), ("Miss", false)] {
            assert_eq!(
                luna::invocation::invoke(&package, entry, 100_000).unwrap(),
                Value::Bool(expected),
                "{domain}: {condition}"
            );
        }
    }
}

#[test]
fn mixed_guard_unknown_comparisons_do_not_create_identity_or_hide_effects() {
    for guard in [
        "Kind == bool or limit > 0",
        "(Kind == bool) == (limit > 0)",
        "Kind == bool and observe() > 0",
        "Kind == bool and do {Kind=.u8'builtin;1} > 0",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            pure comptime fn Box<T,value:T>->Type {{T}}
            comptime fn observe()->.Integer'builtin {{observe()}}
            comptime fn inspect(input:Type,limit:.Integer'builtin)->bool {{
                input match {{Box<(Kind if {guard}),item> => item,_=>false}}
            }}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("dependent normal-form value patterns"),
            "{guard}: {error}"
        );
    }
    for guard in ["flag or limit > 0", "flag and observe() > 0"] {
        let source = format!(
            "typealias Type=.Type'builtin;
            struct Config where flag:bool {{value:bool}}
            pure comptime fn Packet<flag:bool,value:Config<flag>>->Type {{bool}}
            comptime fn observe()->.Integer'builtin {{observe()}}
            comptime fn inspect(input:Type,limit:.Integer'builtin)->bool {{
                input match {{Packet<(flag if {guard}),item> => item.value,_=>false}}
            }}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("dependent normal-form value patterns"),
            "{guard}: {error}"
        );
    }
}

#[test]
fn bitvector_equality_guards_preserve_exact_width_and_value_identity() {
    for (width, value, other) in [
        (0, "0", "0"),
        (1, "1", "0"),
        (8, "165", "166"),
        (65, "18446744073709551616", "0"),
        (129, "340282366920938463463374607431768211456", "0"),
    ] {
        for guard in [
            format!("key == {value}"),
            format!("{value} == key"),
            format!("allowed and key == {value}"),
        ] {
            let source=format!("typealias Type=.Type'builtin;typealias Bits=.BitVec'builtin<{width}>;
                struct Config where key:Bits {{value:bool}}
                pure comptime fn Packet<key:Bits,value:Config<key>>->Type {{bool}}
                pure comptime fn accepts(value:Config<{value}>)->bool {{value.value}}
                comptime fn inspect(input:Type,allowed:bool)->bool {{
                    input match {{Packet<(key if {guard}),item> => accepts(item),_=>false}}
                }}
                comptime fn Hit()->bool {{inspect(Packet<{value},Config<{value}>{{value:true}}>,true)}}
                comptime fn Other()->bool {{inspect(Packet<{other},Config<{other}>{{value:true}}>,true)}}");
            let package = compile(&source);
            for (entry, expected) in [("Hit", true), ("Other", width == 0)] {
                assert_eq!(
                    luna::invocation::invoke(&package, entry, 100_000).unwrap(),
                    Value::Bool(expected),
                    "{width}: {guard}"
                );
            }
        }
    }
}

#[test]
fn bitvector_guard_evidence_checks_widths_and_unknown_inputs() {
    let prefix = "typealias Type=.Type'builtin;typealias Bits=.BitVec'builtin<8>;
        struct Config where key:Bits {value:bool}
        pure comptime fn Packet<key:Bits,value:Config<key>>->Type {bool}
        comptime fn produce()->Bits {produce()}";
    for guard in [
        "key == 7 or key == 8",
        "key != 7",
        "key == produce()",
        "do {key=7;true} and key == 7",
    ] {
        let source = format!(
            "{prefix} comptime fn inspect(input:Type)->bool {{
            input match {{Packet<(key if {guard}),item> => item.value,_=>false}}
        }}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("dependent normal-form value patterns"),
            "{guard}: {error}"
        );
    }
    let wrong_width = format!(
        "{prefix}
        pure comptime fn other()->.BitVec'builtin<4> {{7}}
        comptime fn Bad()->Type {{Packet<other(),Config<7>{{value:true}}>}}"
    );
    assert!(compile_with_protocols(&wrong_width, &Default::default()).is_err());
    assert!(
        compile_with_protocols(
            &wrong_width.replace("builtin<4>", "builtin<8>"),
            &Default::default()
        )
        .is_ok()
    );
}

#[test]
fn computed_scalar_guard_constants_form_exact_dependent_domains() {
    for (domain, expression, expected) in [
        (".u8'builtin", "3 + 4", "7"),
        (".i16'builtin", "-(3 + 4)", "-7"),
        (".Integer'builtin", "350 * 2", "700"),
        (".BitVec'builtin<8>", "not 90", "165"),
        (".BitVec'builtin<8>", "160 or 5", "165"),
        (".BitVec'builtin<65>", "not 0", "36893488147419103231"),
    ] {
        let source=format!("typealias Type=.Type'builtin;typealias Key={domain};
            struct Config where key:Key {{value:bool}}
            pure comptime fn Packet<key:Key,value:Config<key>>->Type {{bool}}
            pure comptime fn accepts(value:Config<{expected}>)->bool {{value.value}}
            comptime fn inspect(input:Type)->bool {{
                input match {{Packet<(key if key == ({expression})),item> => accepts(item),_=>false}}
            }}
            comptime fn Hit()->bool {{inspect(Packet<{expected},Config<{expected}>{{value:true}}>)}}
            comptime fn Miss()->bool {{inspect(Packet<0,Config<0>{{value:true}}>)}}");
        let package = compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{domain}: {expression}: {error}"));
        for (entry, expected) in [("Hit", true), ("Miss", false)] {
            assert_eq!(
                luna::invocation::invoke(&package, entry, 100_000).unwrap(),
                Value::Bool(expected),
                "{domain}: {expression}"
            );
        }
    }
}

#[test]
fn computed_guard_evidence_preserves_checked_arithmetic() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        struct Config where key:Byte {value:bool}
        pure comptime fn Packet<key:Byte,value:Config<key>>->Type {bool}";
    // The final mathematical result is seven, but the original first addition
    // overflows. Extracting a witness must not replace executable checking.
    let source = format!(
        "{prefix}
        comptime fn inspect(input:Type)->bool {{input match {{
            Packet<(key if key == ((255 + 1) - 249)),item> => item.value,_=>false
        }}}}"
    );
    let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(
        error.contains("machine arithmetic range is not proven"),
        "{error}"
    );
    for expression in ["produce() + 0", "do {key=7;7} + 0", "255 + 1"] {
        let source = format!(
            "{prefix}
            comptime fn produce()->Byte {{produce()}}
            comptime fn inspect(input:Type)->bool {{input match {{
                Packet<(key if key == ({expression})),item> => item.value,_=>false
            }}}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("dependent normal-form value patterns"),
            "{expression}: {error}"
        );
    }
}

#[test]
fn normal_form_pattern_ranges_require_successful_guard_evidence() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn Tag<key:Byte>->Type {bool}";
    let source = format!(
        "{prefix}
        comptime fn inspect(input:Type)->Byte {{input match {{
            Tag<(key if key < 255)> => key + 1,_=>0
        }}}}
        comptime fn Hit()->Byte {{inspect(Tag<254>)}}
        comptime fn Miss()->Byte {{inspect(Tag<255>)}}
        comptime fn Other()->Byte {{inspect(bool)}}"
    );
    let package = compile(&source);
    for (entry, expected) in [("Hit", 255), ("Miss", 0), ("Other", 0)] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            Value::UnsignedInteger(expected)
        );
    }
    let error = compile_with_protocols(&source.replace("key < 255", "true"), &Default::default())
        .unwrap_err();
    assert!(
        error.contains("machine arithmetic range is not proven"),
        "{error}"
    );
}

#[test]
fn normal_form_failed_guard_effects_reach_the_fallback_proof() {
    let source = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn Tag<key:Byte>->Type {bool}
        comptime fn inspect(input:Type)->Byte {
            var count:Byte=0;
            input match {
                Tag<(key if do {count=254;false})> => 0,
                _=>count + 1
            }
        }
        comptime fn Hit()->Byte {inspect(Tag<7>)}
        comptime fn Other()->Byte {inspect(bool)}";
    let package = compile(source);
    for (entry, expected) in [("Hit", 255), ("Other", 1)] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            Value::UnsignedInteger(expected)
        );
    }
    let error = compile_with_protocols(
        &source.replace("count=254", "count=255"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("machine arithmetic range is not proven"),
        "{error}"
    );
}

#[test]
fn tuple_type_patterns_support_checked_arm_arithmetic() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn inspect(input:Type, count:Byte)->Byte {
            input match {
                ((_,_),_) if count < 255 => count + 1,
                _=>0
            }
        }
        comptime fn Hit()->Byte {inspect(((bool,bool),bool),254)}
        comptime fn Boundary()->Byte {inspect(((bool,bool),bool),255)}
        comptime fn Other()->Byte {inspect(bool,254)}",
    );
    for (entry, expected) in [("Hit", 255), ("Boundary", 0), ("Other", 0)] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            Value::UnsignedInteger(expected)
        );
    }
}
