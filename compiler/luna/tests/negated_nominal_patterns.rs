#[path = "support/execution.rs"]
mod support;

use comptime_interpretor::Value;

const SOURCE: &str = "trait Duplicate {} trait Retire {fn drop(*self);}
    struct Item {flag:bool}
    enum Choice {none,some(Item)}
    impl Duplicate for Item {}
    impl Duplicate for Choice {}
    fn inspect(value:Choice)->bool {value match {
        not .some({flag:true}) => true,
        _=>false,
    }}
    fn nested(value:Item)->bool {value match {
        not (not {flag:true}) => true,
        _=>false,
    }}
    fn entry()->bool {
        inspect(.none) and inspect(.some(Item {flag:false}))
        and not inspect(.some(Item {flag:true}))
        and nested(Item {flag:true}) and not nested(Item {flag:false})
    }";

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("Duplicate"),
            drop: Some("Retire"),
            ..Default::default()
        },
    )
}

fn run(source: &str) -> Result<Value, String> {
    let package = compile(source)?;
    luna::execution::interpret_with_protocols(&package, "entry", "Duplicate", "Retire", 300_000)
}

#[test]
fn nominal_negation_observes_nested_variants_without_unfolding_types() {
    assert_eq!(run(SOURCE).unwrap(), Value::Bool(true));
}

#[test]
fn nominal_negation_requires_copy_and_trivial_cleanup_recursively() {
    for source in [
        SOURCE.replace("impl Duplicate for Choice {}", ""),
        SOURCE.replace("impl Duplicate for Item {}", ""),
        format!("{SOURCE} impl Retire for Item {{fn drop(*self) {{}}}}"),
    ] {
        // The type-level shape is legal; rejection must come from resource
        // analysis, before the reference interpreter executes the program.
        let package = compile(&source).unwrap();
        let error = luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            300_000,
        )
        .unwrap_err();
        assert!(
            error.contains("Copy") || error.contains("transfer"),
            "{error}"
        );
    }
}

#[test]
fn nominal_negation_does_not_export_inner_bindings() {
    let error = compile(
        "struct Item {flag:bool} fn inspect(value:Item)->bool {
        value match {not {flag:hidden} => hidden,_=>false}
    }",
    )
    .unwrap_err();
    assert!(
        error.contains("Unresolved identifier") || error.contains("unresolved"),
        "{error}"
    );
}

#[test]
fn nominal_negation_uses_generic_field_evidence() {
    let source = "struct Item where T {flag:T}
        trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for Item<bool> {}
        fn inspect(value:Item<T>)->bool where T, requires T == bool {
            value match {not {flag:true} => true,_=>false}
        }
        fn entry()->bool {
            inspect(Item<bool> {flag:false}) and not inspect(Item<bool> {flag:true})
        }";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    assert_eq!(
        run(&source.replace("_=>false", "{flag:true}=>false")).unwrap(),
        Value::Bool(true)
    );
    assert!(compile(&source.replace(", requires T == bool", "")).is_err());
}

#[test]
fn nominal_negation_checks_payload_guards_only_after_tag_success() {
    let source = SOURCE.replace(
        "not .some({flag:true})",
        "not .some({flag:(hidden if probe(hidden))})",
    ) + " fn probe(flag:bool)->bool {flag}";
    assert_eq!(run(&source).unwrap(), Value::Bool(true));
    let package = compile(&source).unwrap();
    let graph = luna::instantiation::instantiate(&package, "entry", 200_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(
        &graph,
        comptime_interpretor::Budget::default(),
    )
    .unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let probe = support::id(&package, "probe");
    assert_eq!(vm.events().iter().filter(|event| matches!(event, comptime_interpretor::Event::EnterFunction(id) if *id==probe)).count(),2);
}

#[test]
fn complementary_nominal_patterns_prove_coverage() {
    let source = SOURCE.replace("_=>false,", ".some({flag:true})=>false,");
    // Keep the separate nested-struct match's complementary arm typed for Item.
    let source = source.replace(
        "not (not {flag:true}) => true,\n        .some({flag:true})=>false,",
        "not (not {flag:true}) => true,\n        {flag:false}=>false,",
    );
    assert_eq!(run(&source).unwrap(), Value::Bool(true));
}

#[test]
fn complements_of_alternatives_preserve_variant_and_field_correlations() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Pair {a:bool,b:bool}
        enum Choice {none,some(Pair)}
        impl Duplicate for Pair {} impl Duplicate for Choice {}
        fn inspect(value:Choice)->bool {value match {
            not (.none or .some({a:true,b:false})) => true,
            .none or .some({a:true,b:false}) => false,
        }}
        fn entry()->bool {
            not inspect(.none)
            and not inspect(.some(Pair {a:true,b:false}))
            and inspect(.some(Pair {a:false,b:true}))
            and inspect(.some(Pair {a:true,b:true}))
            and inspect(.some(Pair {a:false,b:false}))
        }";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    let incomplete = source.replace(
        ".none or .some({a:true,b:false}) => false",
        ".none or .some({a:false,b:true}) => false",
    );
    let error = compile(&incomplete).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
}

#[test]
fn unknown_guard_cannot_be_complemented_into_coverage_evidence() {
    for arms in [
        "not .some({flag:(a if probe(a))})=>true,.some({flag:true})=>false",
        "not (.some({flag:true}) or (.none if probe(true)))=>true,.some({flag:true})=>false",
    ] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            struct Item {{flag:bool}} enum Choice {{none,some(Item)}}
            fn probe(value:bool)->bool {{value}}
            fn inspect(value:Choice)->bool {{ value match {{ {arms} }} }}"
        );
        let error = compile(&source).unwrap_err();
        assert!(error.contains("non-exhaustive"), "{error}");
    }
}

#[test]
fn alternatives_can_contain_constructor_complements() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Item {flag:bool}
        enum Choice {none,some(Item)}
        impl Duplicate for Item {} impl Duplicate for Choice {}
        fn inspect(value:Choice)->bool {value match {
            (not .some({flag:true})) or .some({flag:true}) => true,
        }}
        fn entry()->bool {inspect(.none)
            and inspect(.some(Item {flag:false}))
            and inspect(.some(Item {flag:true}))}";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    let reversed = source.replace(
        "(not .some({flag:true})) or .some({flag:true})",
        ".some({flag:true}) or (not .some({flag:true}))",
    );
    assert_eq!(run(&reversed).unwrap(), Value::Bool(true));
    let unknown_sibling = source.replace(
        "(not .some({flag:true})) or .some({flag:true})",
        "(.none if probe()) or (not .some({flag:true})) or .some({flag:true})",
    ) + " fn probe()->bool {false}";
    assert_eq!(run(&unknown_sibling).unwrap(), Value::Bool(true));
    let incomplete = source.replace(
        "or .some({flag:true}) => true",
        "or .some({flag:false}) => true",
    );
    let error = compile(&incomplete).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
    let unknown = source.replace(
        "or .some({flag:true}) => true",
        "or (.some({flag:true}) if probe()) => true",
    ) + " fn probe()->bool {false}";
    let error = compile(&unknown).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
    for invalid in [
        source.replace("impl Duplicate for Choice {}", ""),
        source.replace("impl Duplicate for Item {}", ""),
        format!("{source} impl Retire for Item {{fn drop(*self) {{}}}}"),
    ] {
        let error = run(&invalid).unwrap_err();
        assert!(
            error.contains("Copy") || error.contains("transfer"),
            "{error}"
        );
    }
}

#[test]
fn negated_alternatives_preserve_guard_order_and_short_circuiting() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(bool)} impl Duplicate for Choice {}
        fn first(value:bool)->bool {value}
        fn second()->bool {true}
        fn inspect(value:Choice,accept:bool)->bool {value match {
            (not .some(true if first(accept))) or .some(true if second()) => true,
            _ => false,
        }}
        fn entry()->bool {
            inspect(.none,true) and inspect(.some(false),true)
            and inspect(.some(true),false) and inspect(.some(true),true)
        }";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    let package = compile(source).unwrap();
    let graph = luna::instantiation::instantiate(&package, "entry", 200_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(
        &graph,
        comptime_interpretor::Budget::default(),
    )
    .unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let first = support::id(&package, "first");
    let second = support::id(&package, "second");
    let calls = vm
        .events()
        .iter()
        .filter_map(|event| match event {
            comptime_interpretor::Event::EnterFunction(id) if *id == first || *id == second => {
                Some(*id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(calls, vec![first, first, second]);
}

#[test]
fn boolean_payload_guards_have_exact_complements() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(bool)} impl Duplicate for Choice {}
        fn inspect(value:Choice)->bool {value match {
            not .some(flag if flag) => true,
            .some(flag if flag) => false,
        }}
        fn direct(value:bool)->bool {value match {
            flag if flag => true,
            flag if not flag => false,
        }}
        fn entry()->bool {inspect(.none) and inspect(.some(false))
            and not inspect(.some(true)) and direct(true) and not direct(false)}";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    for condition in [
        "flag == true",
        "not (flag == false)",
        "flag and (flag or not flag)",
        "do {var selected=not flag; selected=not selected; selected}",
    ] {
        assert_eq!(
            run(&source.replace("flag if flag", &format!("flag if {condition}"))).unwrap(),
            Value::Bool(true)
        );
    }
    let incomplete = source.replace("flag if not flag", "flag if flag");
    assert!(compile(&incomplete).unwrap_err().contains("non-exhaustive"));
    let external = "trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(bool)}
        fn inspect(value:Choice,other:bool)->bool {value match {
            not .some(flag if other) => true,
            .some(true) => false,
        }}";
    let error = compile(external).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
}

#[test]
fn integer_complements_track_guard_local_assignment_without_changing_subject() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Byte=.u8'builtin;
        enum Choice {none,some(Byte)} impl Duplicate for Choice {}
        fn inspect(value:Choice)->bool {value match {
            not .some(x if do {var accepted=x < 10;accepted=not accepted;accepted}) => true,
            .some(x if x >= 10) => false,
        }}
        fn entry()->bool {inspect(.none) and inspect(.some(0)) and inspect(.some(9))
            and not inspect(.some(10)) and not inspect(.some(255))}";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    let snapshot = source.replace(
        "var accepted=x < 10;accepted=not accepted;accepted",
        "var bound=x;let original=bound;bound=10;original >= bound",
    );
    assert_eq!(run(&snapshot).unwrap(), Value::Bool(true));
    let incomplete = source.replace("x >= 10", "x > 10");
    let error = compile(&incomplete).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
    let unknown = source.replace("accepted=not accepted", "accepted=probe(accepted)")
        + " fn probe(value:bool)->bool {not value}";
    let error = compile(&unknown).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
}

#[test]
fn computed_boolean_patterns_use_their_result_and_first_successful_bindings() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        fn direct(value:bool)->bool {value match {
            flag and not flag is true => true,
            flag and not flag is false => false,
        }}
        fn ordered(value:bool)->bool {value match {
            ((x and not x is y) or (x and x is y)) if y => true,
            true => false,
        }}
        fn entry()->bool {direct(false) and not direct(true)
            and ordered(false) and not ordered(true)}";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    let chained = source.replace(
        "flag and not flag is",
        "flag and not flag is inverted and inverted is",
    );
    assert_eq!(run(&chained).unwrap(), Value::Bool(true));
    let incomplete = source.replace("true => false", "false => false");
    let error = compile(&incomplete).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
    let unknown = source.replace("flag and not flag is", "flag and flip(flag) is")
        + " fn flip(value:bool)->bool {not value}";
    let error = compile(&unknown).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
}

#[test]
fn quoted_constructor_complements_keep_private_observation_bindings() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(bool)} impl Duplicate for Choice {}
        comptime fn denied()-><Choice -|> {'<not .some(flag if flag)>}
        comptime fn accepted()-><Choice -|> {'<.some(flag if flag)>}
        comptime fn combined()-><Choice -|> {'<(~<denied()> or .some(true))>}
        fn inspect(value:Choice)->bool {value match {
            ~<denied()> => true, ~<accepted()> => false,
        }}
        fn all(value:Choice)->bool {value match {~<combined()> => true}}
        fn entry()->bool {inspect(.none) and inspect(.some(false))
            and not inspect(.some(true)) and all(.none)
            and all(.some(false)) and all(.some(true))}";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
}

#[test]
fn quoted_complements_connect_contextual_guards_before_coverage() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(bool)} impl Duplicate for Choice {}
        comptime fn denied(code:<x:bool|-bool>)-><Choice -|> {
            '<not .some(flag if ~[x=flag]<code>)>
        }
        fn inspect(value:Choice)->bool {
            const code:<x:bool|-bool> = '(x);
            const p=denied(code);
            value match {~<p> => true, .some(true) => false}
        }
        fn entry()->bool {inspect(.none) and inspect(.some(false)) and not inspect(.some(true))}";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    let negative = source
        .replace(
            "fn inspect(value:Choice)",
            "fn inspect_negative(value:Choice)",
        )
        .replace(
            "const code:<x:bool|-bool> = '(x)",
            "const code:<x:bool|-bool> = '(not x)",
        )
        .replace(".some(true) => false", ".some(false) => false");
    // Both instances coexist; the second must not reuse the first instance's
    // contextual substitution or synthetic observation local.
    let start = negative.find("        fn inspect_negative").unwrap();
    let end = negative.find("        fn entry").unwrap();
    let combined = source.replace(
        "and not inspect(.some(true))",
        "and not inspect(.some(true)) and inspect_negative(.none)
         and inspect_negative(.some(true)) and not inspect_negative(.some(false))",
    ) + &negative[start..end];
    assert_eq!(run(&combined).unwrap(), Value::Bool(true));
    let incomplete = source.replace(".some(true) => false", ".some(false) => false");
    let error = compile(&incomplete).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
    let escaped = source.replace("~<p> => true", "~<p> => flag");
    let error = compile(&escaped).unwrap_err();
    assert!(
        error.contains("Unresolved identifier") || error.contains("unresolved"),
        "{error}"
    );
}

#[test]
fn integer_payload_complements_preserve_interval_boundaries() {
    let source = "typealias Byte=.u8'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(Byte)} impl Duplicate for Choice {}
        fn inspect(value:Choice)->bool {value match {
            not (.none or .some(10..20) or .some(30..=40)) => true,
            .none or .some(10..20) or .some(30..=40) => false,
        }}
        fn entry()->bool {
            not inspect(.none) and inspect(.some(0)) and inspect(.some(9))
            and not inspect(.some(10)) and not inspect(.some(19))
            and inspect(.some(20)) and inspect(.some(29))
            and not inspect(.some(30)) and not inspect(.some(40))
            and inspect(.some(41)) and inspect(.some(255))
        }";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    let incomplete = source.replace(
        ".none or .some(10..20) or .some(30..=40) => false",
        ".none or .some(11..20) or .some(30..=40) => false",
    );
    assert!(compile(&incomplete).unwrap_err().contains("non-exhaustive"));
}

#[test]
fn numeric_complements_keep_signed_character_and_boolean_domains_distinct() {
    let source = "typealias Byte=.i8'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {number(Byte),letter(.char'builtin),pair(Byte,bool)}
        impl Duplicate for Choice {}
        fn inspect(value:Choice)->bool {value match {
            not (.number(-5..=5) or .letter('a'..='z') or .pair(-5..5,true)) => true,
            .number(-5..=5) or .letter('a'..='z') or .pair(-5..5,true) => false,
        }}
        fn entry()->bool {
            inspect(.number(-128)) and inspect(.number(-6))
            and not inspect(.number(-5)) and not inspect(.number(5))
            and inspect(.number(6)) and inspect(.number(127))
            and inspect(.letter('A')) and not inspect(.letter('a'))
            and not inspect(.letter('z')) and inspect(.letter('{'))
            and not inspect(.pair(0,true)) and inspect(.pair(0,false))
            and inspect(.pair(5,true)) and inspect(.pair(-6,true))
        }";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
}

#[test]
fn numeric_complement_does_not_capture_external_guard_inputs() {
    for pattern in ["not .some(x if probe(x))", "not .some(x if x < limit)"] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            enum Choice {{none,some(Byte)}}
            fn probe(x:Byte)->bool {{x < 10}}
            fn inspect(value:Choice,limit:Byte)->bool {{value match {{
                {pattern} => true, .none or .some(0..10) => false,
            }}}}"
        );
        let error = compile(&source).unwrap_err();
        assert!(error.contains("non-exhaustive"), "{error}");
    }
}

#[test]
fn floating_payload_complements_include_nan_and_preserve_signed_zero() {
    for primitive in ["f32", "f64"] {
        let source = format!(
            "typealias Float=.{primitive}'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            enum Choice {{none,some(Float)}} impl Duplicate for Choice {{}}
            fn inspect(value:Choice)->bool {{value match {{
                not (.none or .some(0.0..1.0) or .some(2.0..=3.0)) => true,
                .none or .some(0.0..1.0) or .some(2.0..=3.0) => false,
            }}}}
            fn entry()->bool {{
                not inspect(.none) and not inspect(.some(0.0))
                and not inspect(.some(-0.0)) and not inspect(.some(0.5))
                and inspect(.some(1.0)) and not inspect(.some(2.0))
                and not inspect(.some(3.0)) and inspect(.some(3.5))
                and inspect(.some(Float.NaN))
                and inspect(.some(Float.Infinity)) and inspect(.some(-Float.Infinity))
            }}"
        );
        assert_eq!(run(&source).unwrap(), Value::Bool(true), "{primitive}");
        let incomplete = source.replace(
            ".none or .some(0.0..1.0) or .some(2.0..=3.0) => false",
            ".none or .some(0.0..1.0) or .some(2.0..3.0) => false",
        );
        assert!(compile(&incomplete).unwrap_err().contains("non-exhaustive"));
    }
}

#[test]
fn floating_guard_assignments_preserve_saved_values_and_nan_paths() {
    for primitive in ["Float", "f32", "f64"] {
        let phase = if primitive == "Float" {
            "comptime "
        } else {
            ""
        };
        let exceptional = if primitive == "Float" {
            ""
        } else {
            "and not inspect(.some(Number.NaN)) and inspect(.some(Number.Infinity))
             and not inspect(.some(-Number.Infinity))"
        };
        let source = format!(
            "typealias Number=.{primitive}'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            enum Choice {{none,some(Number)}} impl Duplicate for Choice {{}}
            {phase}fn inspect(value:Choice)->bool {{value match {{
                not .some(x if do {{var boundary=x;let saved=boundary;boundary=0.0;
                    var selected=saved >= boundary;selected=not selected;selected}}) => true,
                .some(x if not (x >= 0.0)) => false,
            }}}}
            {phase}fn entry()->bool {{inspect(.none) and inspect(.some(0.0))
                and inspect(.some(-0.0)) and inspect(.some(0.5))
                and not inspect(.some(-0.5)) {exceptional}}}"
        );
        let result = if primitive == "Float" {
            luna::invocation::invoke(&compile(&source).unwrap(), "entry", 300_000)
        } else {
            run(&source)
        };
        assert_eq!(result.unwrap(), Value::Bool(true), "{primitive}");
        let unknown = source.replace(
            "selected=not selected",
            "selected=probe(selected);selected=false",
        ) + &format!(" {phase}fn probe(value:bool)->bool {{not value}}");
        let error = compile(&unknown).unwrap_err();
        assert!(error.contains("non-exhaustive"), "{primitive}: {error}");
        let incomplete = source.replace(
            ".some(x if not (x >= 0.0)) => false",
            ".some(x if x < 0.0) => false",
        );
        if primitive != "Float" {
            let error = compile(&incomplete).unwrap_err();
            assert!(error.contains("non-exhaustive"), "{primitive}: {error}");
        }
    }
}

#[test]
fn exact_comptime_float_payloads_use_exact_interval_complements() {
    let source = "typealias Float=.Float'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(Float)} impl Duplicate for Choice {}
        comptime fn inspect(value:Choice)->bool {value match {
            not (.none or .some(0.1..0.2) or .some(0.3..=0.4)) => true,
            .none or .some(0.1..0.2) or .some(0.3..=0.4) => false,
        }}
        comptime fn entry()->bool {
            not inspect(.none) and inspect(.some(0.0))
            and not inspect(.some(0.1)) and inspect(.some(0.2))
            and not inspect(.some(0.3)) and not inspect(.some(0.4))
            and inspect(.some(0.4000000000000000000000000000001))
        }";
    let package = compile(source).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "entry", 300_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn floating_complements_do_not_hide_nan_or_unknown_guard_gaps() {
    for primitive in ["f32", "f64"] {
        for pattern in [
            "not .some(x if x == x)",
            "not .some(x if x != x)",
            "not .some(x if probe(x))",
        ] {
            let source = format!(
                "typealias Float=.{primitive}'builtin;
                trait Duplicate {{}} trait Retire {{fn drop(*self);}}
                enum Choice {{none,some(Float)}}
                fn probe(value:Float)->bool {{value == value}}
                fn inspect(value:Choice)->bool {{value match {{
                    {pattern} => true, .none => false,
                }}}}"
            );
            let error = compile(&source).unwrap_err();
            assert!(
                error.contains("non-exhaustive"),
                "{primitive}, {pattern}: {error}"
            );
        }
    }
}

#[test]
fn bit_payload_complements_preserve_mask_regions() {
    let checks = (0..16)
        .map(|value| {
            format!(
                "{}inspect(.some({value}))",
                if value == 3 || (8..12).contains(&value) {
                    "not "
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" and ");
    let source = format!(
        "typealias Bits=.BitVec'builtin<4>;
        typealias Tail=.BitVec'builtin<2>; typealias Bit=.BitVec'builtin<1>;
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        enum Choice {{none,some(Bits)}} impl Duplicate for Choice {{}}
        fn inspect(value:Choice)->bool {{value match {{
            not (.none or .some(0b10 (_:Tail)) or .some(0b0011)) => true,
            .none or .some(0b10 (_:Tail)) or .some(0b0011) => false,
        }}}}
        fn entry()->bool {{not inspect(.none) and {checks}}}"
    );
    assert_eq!(run(&source).unwrap(), Value::Bool(true));
    let incomplete = source.replace(
        ".none or .some(0b10 (_:Tail)) or .some(0b0011) => false",
        ".none or .some(0b100 (_:Bit)) or .some(0b0011) => false",
    );
    assert!(compile(&incomplete).unwrap_err().contains("non-exhaustive"));
}

#[test]
fn bit_alternatives_export_only_declared_segment_bindings() {
    let source = "typealias Bits=.BitVec'builtin<4>;typealias Tail=.BitVec'builtin<2>;
        trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(Bits)} impl Duplicate for Choice {}
        fn extract(value:Choice)->Tail {value match {
            .some(0b (head:Tail) 11) or .some(0b00 (head:Tail)) => head,
            _=>0,
        }}
        fn entry()->bool {
            extract(.some(7)) == 1 and extract(.some(2)) == 2
            and extract(.some(3)) == 0 and extract(.some(15)) == 3
            and extract(.none) == 0
        }";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
}

#[test]
fn bit_guard_locals_preserve_projection_and_predicate_snapshots() {
    let checks = (0..16)
        .map(|value| {
            format!(
                "{}inspect(.some({value}))",
                if value == 9 { "not " } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(" and ");
    let source = format!(
        "typealias Bits=.BitVec'builtin<4>;
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        enum Choice {{none,some(Bits)}} impl Duplicate for Choice {{}}
        fn inspect(value:Choice)->bool {{value match {{
            not .some(x if do {{
                var part=x[0..2];let saved=part;part=x[2..4];
                var selected=(saved matches 0b01) and (part matches 0b10);
                let before=selected;selected=not selected;before and not selected
            }}) => true,
            .some(0b1001) => false,
        }}}}
        fn entry()->bool {{inspect(.none) and {checks}}}"
    );
    assert_eq!(run(&source).unwrap(), Value::Bool(true));
    let equality = source.replace("before and not selected", "before and (before != selected)");
    assert_eq!(run(&equality).unwrap(), Value::Bool(true));
    let incomplete = source.replace(".some(0b1001) => false", ".some(0b1000) => false");
    let error = compile(&incomplete).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
    let unknown = source.replace(
        "selected=not selected",
        "selected=probe(selected);selected=false",
    ) + " fn probe(value:bool)->bool {not value}";
    let error = compile(&unknown).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
}

#[test]
fn bit_predicate_results_support_matches_and_ordered_sequential_patterns() {
    let source = "typealias Bits=.BitVec'builtin<2>;
        trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(Bits)} impl Duplicate for Choice {}
        fn inspect(value:Choice)->bool {value match {
            not .some(x if ((x matches 0b01) match {flag if flag => true, false => false})) => true,
            .some(0b01) => false,
        }}
        fn entry()->bool {inspect(.none) and inspect(.some(0))
            and not inspect(.some(1)) and inspect(.some(2)) and inspect(.some(3))}";
    for pattern in [
        "x if ((x matches 0b01) match {flag if flag => true, false => false})",
        "x and (x matches 0b01) is (selected if selected)",
        "((x and (x matches 0b01) is selected) or (x and true is selected)) if selected",
    ] {
        let source = source.replace(
            "x if ((x matches 0b01) match {flag if flag => true, false => false})",
            pattern,
        );
        assert_eq!(
            run(&source).unwrap_or_else(|error| panic!("{pattern}: {error}")),
            Value::Bool(true)
        );
        let incomplete = source.replace(".some(0b01) => false", ".some(0b00) => false");
        let error = compile(&incomplete).unwrap_err();
        assert!(error.contains("non-exhaustive"), "{pattern}: {error}");
    }
    let unknown = source.replace("flag if flag => true", "flag if flag => probe(flag)")
        + " fn probe(value:bool)->bool {value}";
    let error = compile(&unknown).unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
}

#[test]
fn bit_equality_guards_preserve_constants_overlapping_views_and_saved_values() {
    for (condition, selected) in [
        ("x == 0b1001", vec![9]),
        ("0b1001 == x", vec![9]),
        ("x[0..2] == x[2..4]", vec![0, 5, 10, 15]),
        ("x[0..3] == x[1..4]", vec![0, 15]),
        ("x == x", (0..16).collect()),
        (
            "(x[0..2] and x[2..4]) == 0b01",
            (0..16)
                .filter(|value| ((value & 3) & (value >> 2)) == 1)
                .collect(),
        ),
        (
            "(x[0..2] or x[2..4]) matches 0b10",
            (0..16)
                .filter(|value| ((value & 3) | (value >> 2)) == 2)
                .collect(),
        ),
        (
            "do {var combined=x[0..2] and x[2..4];let saved=combined;combined=not combined;(saved[0..1] == 0b1) and (combined[1..2] == 0b1)}",
            (0..16)
                .filter(|value| ((value & 3) & (value >> 2)) == 1)
                .collect(),
        ),
        (
            "((x[0..2] and x[2..4]) or (not x[0..2] and not x[2..4])) == 0b11",
            vec![0, 5, 10, 15],
        ),
        ("(x and 0b1100) == 0b1000", vec![8, 9, 10, 11]),
        ("(x or 0b0011) == 0b1011", vec![8, 9, 10, 11]),
        ("(not x) == 0b0110", vec![9]),
        ("x == not x", vec![]),
        ("(x and not x) == 0b0000", (0..16).collect()),
        ("(x or not x) == 0b1111", (0..16).collect()),
        ("(x and 0b1100) matches 0b1000", vec![8, 9, 10, 11]),
        (
            "do {var masked=x and 0b1100;let saved=masked;masked=not masked;(saved[2..4] == 0b10) and (masked[2..4] == 0b01)}",
            vec![8, 9, 10, 11],
        ),
        (
            "do {let wide:.BitVec'builtin<65> = 19327352832;x == wide[31..35]}",
            vec![9],
        ),
        (
            "do {let wide:.BitVec'builtin<65> = 19327352832;x == wide[30..36][1..5]}",
            vec![9],
        ),
        (
            "do {let wide:.BitVec'builtin<65> = 19327352832;x[0..0] == wide[33..33]}",
            (0..16).collect(),
        ),
        (
            "do {let wanted:Bits=0b1001;(wanted matches 0b1001) and x == wanted}",
            vec![9],
        ),
        (
            "do {var wanted:Bits=0b0011;let saved=wanted;wanted=0b1001;(x == wanted) and (saved != wanted)}",
            vec![9],
        ),
    ] {
        let checks = (0..16)
            .map(|value| {
                format!(
                    "{}inspect(.some({value}))",
                    if selected.contains(&value) {
                        "not "
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(" and ");
        let source = format!(
            "typealias Bits=.BitVec'builtin<4>;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            enum Choice {{none,some(Bits)}} impl Duplicate for Choice {{}}
            fn inspect(value:Choice)->bool {{value match {{
                not .some(x if {condition}) => true,
                .some(x if {condition}) => false,
            }}}}
            fn entry()->bool {{inspect(.none) and {checks}}}"
        );
        assert_eq!(
            run(&source).unwrap_or_else(|error| panic!("{condition}: {error}")),
            Value::Bool(true)
        );
    }
    for condition in [
        "x == other",
        "x == probe(x)",
        "(probe(x) and 0b0000) == 0b0000",
        "(0b1111 or probe(x)) == 0b1111",
    ] {
        let source = format!(
            "typealias Bits=.BitVec'builtin<4>;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            enum Choice {{none,some(Bits)}}
            fn probe(value:Bits)->Bits {{value}}
            fn inspect(value:Choice,other:Bits)->bool {{value match {{
                not .some(x if {condition}) => true,.some(0b1001)=>false,
            }}}}"
        );
        let error = compile(&source).unwrap_err();
        assert!(error.contains("non-exhaustive"), "{condition}: {error}");
    }
}

#[test]
fn bit_equality_guards_cover_zero_width_and_multiword_constants() {
    for width in [0, 1, 33, 129] {
        let source = format!(
            "typealias Bits=.BitVec'builtin<{width}>;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            enum Choice {{none,some(Bits)}} impl Duplicate for Choice {{}}
            fn masked_zero(value:Bits)->bool {{value match {{
                x if (x and not x) == 0 => true,
            }}}}
            fn equal_inverse(value:Bits)->bool {{value match {{
                x if x == not x => true,
                x if x != not x => false,
            }}}}
            fn inspect(value:Choice)->bool {{value match {{
                .some(x if x == 0) => false,
                .none => true,
                .some(x if x != 0) => true,
            }}}}
            fn entry()->bool {{masked_zero(0) and (equal_inverse(0) == {empty})
                and inspect(.none) and not inspect(.some(0))
                and (inspect(.some({})) == {})}}",
            usize::from(width != 0),
            width != 0,
            empty = width == 0
        );
        assert_eq!(
            run(&source).unwrap_or_else(|error| panic!("width={width}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn bit_guard_local_predicates_cover_empty_and_multiword_domains() {
    for width in [0, 1, 33, 129] {
        let source = format!(
            "typealias Bits=.BitVec'builtin<{width}>;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            enum Choice {{none,some(Bits)}} impl Duplicate for Choice {{}}
            comptime fn zero()->Bits {{0}}
            fn inspect(value:Choice)->bool {{value match {{
                not .some(x if do {{var selected=x matches 0b < zero() >;
                    let saved=selected;selected=not selected;saved and not selected}}) => true,
                .some(0b < zero() >) => false,
            }}}}
            fn entry()->bool {{inspect(.none) and not inspect(.some(0))
                and (inspect(.some({})) == {})}}",
            usize::from(width != 0),
            width != 0
        );
        assert_eq!(
            run(&source).unwrap_or_else(|error| panic!("width={width}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn bit_payload_complements_handle_empty_and_multiword_values() {
    for width in [0, 1, 33, 129] {
        let outside = usize::from(width != 0);
        let source = format!(
            "typealias Bits=.BitVec'builtin<{width}>;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            enum Choice {{none,some(Bits)}} impl Duplicate for Choice {{}}
            comptime fn zero()->Bits {{0}}
            fn inspect(value:Choice)->bool {{value match {{
                not .some(0b < zero() >) => true,
                .some(0b < zero() >) => false,
            }}}}
            fn entry()->bool {{inspect(.none) and not inspect(.some(0))
                and (inspect(.some({outside})) == {})}}
        ",
            width != 0
        );
        assert_eq!(
            run(&source).unwrap_or_else(|error| panic!("width={width}: {error}")),
            Value::Bool(true),
            "width={width}"
        );
    }
}

#[test]
fn bit_complement_requires_exact_guard_information() {
    let source = "typealias Bits=.BitVec'builtin<2>;
        trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(Bits)}
        fn probe(value:Bits)->bool {true}
        fn inspect(value:Choice)->bool {value match {
            not .some(x if probe(x)) => true,
            .none or .some(0b00) => false,
        }}";
    assert!(compile(source).unwrap_err().contains("non-exhaustive"));
}

#[test]
fn symbol_payload_complements_include_unnamed_symbols() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(Symbol)} impl Duplicate for Choice {}
        fn inspect(value:Choice)->bool {value match {
            not (.none or .some(.red) or .some(.blue)) => true,
            .none or .some(.red) or .some(.blue) => false,
        }}
        fn entry()->bool {
            not inspect(.none) and not inspect(.some(.red))
            and not inspect(.some(.blue)) and inspect(.some(.green))
            and inspect(.some(.never_seen_in_pattern))
        }";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    let error = compile(&source.replace(
        ".none or .some(.red) or .some(.blue) => false",
        ".none or .some(.red) => false",
    ))
    .unwrap_err();
    assert!(error.contains("non-exhaustive"), "{error}");
}

#[test]
fn symbol_payload_guards_keep_open_domain_and_external_inputs_distinct() {
    let source = "typealias Symbol=.Symbol'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        enum Choice {none,some(Symbol)} impl Duplicate for Choice {}
        fn inspect(value:Choice)->bool {value match {
            not .some(name if name == .red or name == .blue) => true,
            .some(.red) or .some(.blue) => false,
        }}
        fn entry()->bool {
            inspect(.none) and not inspect(.some(.red))
            and not inspect(.some(.blue)) and inspect(.some(.green))
        }";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    for guard in ["name == other", "probe(name)"] {
        let source = format!(
            "typealias Symbol=.Symbol'builtin;
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            enum Choice {{none,some(Symbol)}}
            fn probe(value:Symbol)->bool {{value == .red}}
            fn inspect(value:Choice,other:Symbol)->bool {{value match {{
                not .some(name if {guard}) => true,
                .none or .some(.red) => false,
            }}}}"
        );
        assert!(compile(&source).unwrap_err().contains("non-exhaustive"));
    }
}

#[test]
fn error_result_negation_distinguishes_success_and_error_payloads() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        enum Failure {bad(bool)}
        typealias Result=![Failure] bool;
        impl Duplicate for Failure {} impl Duplicate for Result {}
        fn success(value:bool)->Result {value}
        fn failure(value:bool)->Result {error Failure.bad(value)}
        fn not_true(value:Result)->bool {value match {
            not true! => true, true! => false,
        }}
        fn not_bad(value:Result)->bool {value match {
            not (error .bad(true)) => true, error .bad(true) => false,
        }}
        fn entry()->bool {
            not not_true(success(true)) and not_true(success(false))
            and not_true(failure(true)) and not_true(failure(false))
            and not_bad(success(true)) and not_bad(success(false))
            and not not_bad(failure(true)) and not_bad(failure(false))
        }";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
}

#[test]
fn error_complements_keep_complete_generic_error_identities() {
    let source = "typealias Byte=.u8'builtin;
        trait Duplicate {} trait Retire {fn drop(*self);}
        enum Failure where T {bad(T)}
        typealias Outcome=![Failure<bool>,Failure<Byte>] bool;
        impl Duplicate for Failure<bool> {} impl Duplicate for Failure<Byte> {}
        impl Duplicate for Outcome {}
        fn flag(value:bool)->Outcome {error Failure<bool>.bad(value)}
        fn byte(value:Byte)->Outcome {error Failure<Byte>.bad(value)}
        fn success(value:bool)->Outcome {value}
        fn inspect(value:Outcome)->bool {value match {
            not (true! or (error Failure<bool>.bad(true)) or (error Failure<Byte>.bad(10..20))) => true,
            true! or (error Failure<bool>.bad(true)) or (error Failure<Byte>.bad(10..20)) => false,
        }}
        fn entry()->bool {
            not inspect(success(true)) and inspect(success(false))
            and not inspect(flag(true)) and inspect(flag(false))
            and inspect(byte(9)) and not inspect(byte(10))
            and not inspect(byte(19)) and inspect(byte(20))
        }";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    for source in [
        source.replace("impl Duplicate for Failure<Byte> {}", ""),
        source.replace("impl Duplicate for Outcome {}", ""),
        format!("{source} impl Retire for Failure<Byte> {{fn drop(*self) {{}}}}"),
    ] {
        let package = compile(&source).unwrap();
        let error = luna::execution::interpret_with_protocols(
            &package,
            "entry",
            "Duplicate",
            "Retire",
            300_000,
        )
        .unwrap_err();
        assert!(
            error.contains("Copy") || error.contains("transfer"),
            "{error}"
        );
    }
}

#[test]
fn error_result_negation_supports_void_success_payloads() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        enum Failure {bad}
        typealias Void=.void'builtin;
        typealias Outcome=![Failure] Void;
        impl Duplicate for Failure {} impl Duplicate for Outcome {}
        fn success()->Outcome {()}
        fn failure()->Outcome {error Failure.bad}
        fn inspect(value:Outcome)->bool {value match {
            not ()! => true, ()! => false,
        }}
        fn entry()->bool {not inspect(success()) and inspect(failure())}";
    assert_eq!(run(source).unwrap(), Value::Bool(true));
    let package = compile(&source.replace("impl Duplicate for Failure {}", "")).unwrap();
    let error = luna::execution::interpret_with_protocols(
        &package,
        "entry",
        "Duplicate",
        "Retire",
        300_000,
    )
    .unwrap_err();
    assert!(error.contains("Copy"), "{error}");
}
