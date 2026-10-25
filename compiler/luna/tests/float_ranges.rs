//! Machine-float range patterns must use ordered IEEE comparisons.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod execution_support;
use comptime_interpretor::Value;
use execution_support::compile;

#[test]
fn machine_float_range_patterns_preserve_ordered_bounds() {
    for width in ["f32", "f64"] {
        for comptime in [false, true] {
            for (value, lower, upper) in [
                ("0.0", "-0.0", "0.0"),
                ("-0.0", "0.0", "1.0"),
                ("1.0", "0.0", "1.0"),
                ("-1.0", "0.0", "1.0"),
                ("0.5", "1.0", "0.0"),
                ("Float.NaN", "0.0", "1.0"),
                ("0.0", "Float.NaN", "1.0"),
                ("0.0", "-1.0", "Float.NaN"),
                ("Float.Infinity", "0.0", "Float.Infinity"),
                ("-Float.Infinity", "-Float.Infinity", "0.0"),
            ] {
                let phase = if comptime { "comptime " } else { "" };
                let source = format!(
                    "typealias Float=.{width}'builtin;
                    trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                    {phase}fn run()->Tuple<bool,bool,bool,bool,bool> {{
                        let value:Float={value};let lower:Float={lower};let upper:Float={upper};
                        (value match {{'lower..'upper=>true,_=>false}},
                         value match {{'lower..='upper=>true,_=>false}},
                         value match {{'lower..=>true,_=>false}},
                         value match {{..'upper=>true,_=>false}},
                         value match {{..='upper=>true,_=>false}})
                    }}"
                );
                let number = |text: &str| match text {
                    "Float.NaN" => f64::NAN,
                    "Float.Infinity" => f64::INFINITY,
                    "-Float.Infinity" => f64::NEG_INFINITY,
                    _ => text.parse::<f64>().unwrap(),
                };
                let (v, lo, hi) = (number(value), number(lower), number(upper));
                let expected = [
                    lo <= v && v < hi,
                    lo <= v && v <= hi,
                    lo <= v,
                    v < hi,
                    v <= hi,
                ];
                let package = execution_support::compile_with_tuple(&source);
                let actual = if comptime {
                    luna::invocation::invoke(&package, "run", 100000)
                } else {
                    luna::execution::interpret_with_protocols(
                        &package,
                        "run",
                        "Duplicate",
                        "Retire",
                        100000,
                    )
                };
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{source}: {e}")),
                    Value::Tuple(expected.into_iter().map(Value::Bool).collect()),
                    "{source}"
                );
            }
        }
    }
}

#[test]
fn machine_float_range_endpoints_execute_once_in_source_order() {
    use comptime_interpretor::{Budget, Interpreter};
    use execution_support::{calls, id};
    for width in ["f32", "f64"] {
        let package = compile(&format!(
            "typealias Float=.{width}'builtin;
            comptime fn lower()->Float {{ -1.0 }}
            comptime fn upper()->Float {{1.0}}
            comptime fn probe(value:Float)->bool {{
                value match {{ < lower() > .. < upper() > =>true,_=>false }}
            }}
            comptime fn early(value:Float)->bool {{
                value match {{ < do {{return true;}} > .. < upper() > =>false,_=>false }}
            }}"
        ));
        for (number, expected) in [(0.0, true), (1.0, false), (-2.0, false), (f64::NAN, false)] {
            let value = if width == "f32" {
                Value::Float32((number as f32).to_bits())
            } else {
                Value::Float64(number.to_bits())
            };
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(id(&package, "probe"), vec![value.clone()])
                    .unwrap(),
                Value::Bool(expected)
            );
            assert_eq!(calls(&package, &vm), ["probe", "lower", "upper"]);
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(id(&package, "early"), vec![value]).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(calls(&package, &vm), ["early"]);
        }
    }
}

#[test]
fn negated_float_ranges_include_unordered_values() {
    for width in ["f32", "f64"] {
        for comptime in [false, true] {
            let phase = if comptime { "comptime " } else { "" };
            for (value, expected) in [
                ("Float.NaN", true),
                ("0.5", false),
                ("1.0", true),
                ("-0.0", false),
            ] {
                for depth in 1..=3 {
                    let mut pattern = "0.0..1.0".to_string();
                    for _ in 0..depth {
                        pattern = format!("not ({pattern})");
                    }
                    let expected = if depth % 2 == 0 { !expected } else { expected };
                    let package = compile(&format!(
                        "typealias Float=.{width}'builtin;
                    trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                    {phase}fn outside(value:Float)->bool {{
                        value match {{{pattern}=>true,_=>false}}
                    }}
                    {phase}fn run()->bool {{outside({value})}}"
                    ));
                    let actual = if comptime {
                        luna::invocation::invoke(&package, "run", 100000)
                    } else {
                        luna::execution::interpret_with_protocols(
                            &package,
                            "run",
                            "Duplicate",
                            "Retire",
                            100000,
                        )
                    };
                    assert_eq!(
                        actual.unwrap(),
                        Value::Bool(expected),
                        "{width}, {comptime}, {value}, depth={depth}"
                    );
                }
            }
        }
    }
}

#[test]
fn ordered_ranges_do_not_prove_coverage_of_nan() {
    use execution_support::compile_with_protocols;
    for width in ["f32", "f64"] {
        for arms in [
            "..0.0=>false,0.0..=>true",
            "..=0.0=>false,0.0..=>true",
            "'low..='high=>true",
            "..0.0=>false,0.0..=>true, 'nan=>false",
        ] {
            let source = format!(
                "typealias Float=.{width}'builtin;fn classify(value:Float)->bool {{let low:Float= -Float.Infinity;let high:Float=Float.Infinity;let nan:Float=Float.NaN;value match {{{arms}}}}}"
            );
            let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
            assert!(
                error.contains("non-exhaustive") || error.contains("exhaustiveness"),
                "{source}: {error}"
            );
        }
    }
}

#[test]
fn machine_float_ranges_prove_complementary_coverage() {
    for width in ["f32", "f64"] {
        let package = execution_support::compile_with_tuple(&format!(
            "typealias Float=.{width}'builtin;
            trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            fn classify(value:Float)->bool {{
                value match {{0.0..1.0=>true,not (0.0..1.0)=>false}}
            }}
            fn run()->Tuple<bool,bool,bool,bool> {{
                (classify(0.5),classify(-0.0),classify(1.0),classify(Float.NaN))
            }}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100000
            )
            .unwrap(),
            Value::Tuple(
                [true, true, false, false]
                    .into_iter()
                    .map(Value::Bool)
                    .collect()
            )
        );
    }
}

#[test]
fn machine_float_coverage_retains_nan_and_tuple_correlations() {
    use execution_support::compile_with_protocols;
    for width in ["f32", "f64"] {
        let package = execution_support::compile_with_tuple(&format!(
            "typealias Float=.{width}'builtin;
            fn ordered(value:Float)->bool {{value match {{
                _ as x if x == x =>true,_ as x if x != x =>false,
            }}}}
            fn pair(value:Tuple<Float,bool>)->bool {{value match {{
                (0.0..1.0,true)=>true,(0.0..1.0,false)=>false,(not (0.0..1.0),_)=>false,
            }}}}
            fn run()->Tuple<bool,bool,bool,bool,bool> {{
                (ordered(Float.NaN),ordered(-0.0),pair((0.5,true)),pair((0.5,false)),pair((Float.NaN,true)))
            }}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100000).unwrap(),
            Value::Tuple(
                [false, true, true, false, false]
                    .into_iter()
                    .map(Value::Bool)
                    .collect()
            )
        );
        for body in [
            "value match {_ as x if x == x=>true}",
            "(value,true) match {(0.0..1.0,false)=>true,(not (0.0..1.0),_)=>false}",
        ] {
            let source =
                format!("typealias Float=.{width}'builtin;fn bad(value:Float)->bool {{{body}}}");
            assert!(
                execution_support::compile_with_tuple_protocols(&source, &Default::default())
                    .is_err(),
                "{source}"
            );
        }
        let source = format!(
            "typealias Float=.{width}'builtin;fn decide(value:Float)->bool {{true}} fn bad(value:Float)->bool {{value match {{_ as x if decide(x)=>true,not (_ as x if decide(x))=>false}}}}"
        );
        let error = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_err();
        assert!(
            error.contains("match exhaustiveness could not be proved"),
            "executable guards must not become symbolic evidence: {error}"
        );
    }
}

#[test]
fn conditional_float_guards_preserve_branch_domains() {
    use execution_support::compile_with_protocols;
    for width in ["f32", "f64"] {
        for comptime in [false, true] {
            let phase = if comptime { "comptime " } else { "" };
            let package=execution_support::compile_with_tuple(&format!(
                "typealias Float=.{width}'builtin;
                trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                {phase}fn ordered(value:Float)->bool {{value match {{
                    _ as x if if x < 0.0 {{true}} else {{x >= 0.0}} =>true,
                    _ as x if x != x =>false,
                }}}}
                {phase}fn all(value:Float)->bool {{value match {{
                    _ as x if if x == x {{if x < 0.0 {{true}} else {{x >= 0.0}}}} else {{true}} =>true,
                }}}}
                {phase}fn run()->Tuple<bool,bool,bool,bool,bool,bool> {{
                    (ordered(Float.NaN),ordered(-0.0),ordered(-Float.Infinity),ordered(Float.Infinity),all(Float.NaN),all(0.5))
                }}"
            ));
            let actual = if comptime {
                luna::invocation::invoke(&package, "run", 100000)
            } else {
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100000,
                )
            };
            assert_eq!(
                actual.unwrap(),
                Value::Tuple(
                    [false, true, true, true, true, true]
                        .into_iter()
                        .map(Value::Bool)
                        .collect()
                )
            );
        }
        for guard in [
            "if x < 0.0 {true} else {x >= 0.0}",
            "if x == x {false} else {true}",
            "if x < 0.0 {x > 0.0} else {x != x}",
        ] {
            let source = format!(
                "typealias Float=.{width}'builtin;fn bad(value:Float)->bool {{value match {{_ as x if {guard}=>true}}}}"
            );
            assert!(
                execution_support::compile_with_tuple_protocols(&source, &Default::default())
                    .is_err(),
                "{source}"
            );
        }
    }
}

#[test]
fn local_boolean_guard_evidence_survives_aliases_and_branch_narrowing() {
    use execution_support::compile_with_protocols;
    for width in ["f32", "f64"] {
        let package=execution_support::compile_with_tuple(&format!(
            "typealias Float=.{width}'builtin;
            trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            fn ordered(value:Float)->bool {{value match {{
                _ as x if do {{let ordered=x == x;let alias=ordered;alias}} =>true,
                _ as x if do {{let ordered=x == x;not ordered}} =>false,
            }}}}
            fn all(value:Float)->bool {{value match {{
                _ as x if do {{let negative=x < 0.0;if negative {{let known=negative;known}} else {{not negative}}}} =>true,
            }}}}
            fn run()->Tuple<bool,bool,bool,bool,bool> {{
                (ordered(Float.NaN),ordered(-0.0),all(-1.0),all(1.0),all(Float.NaN))
            }}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100000
            )
            .unwrap(),
            Value::Tuple(
                [false, true, true, true, true]
                    .into_iter()
                    .map(Value::Bool)
                    .collect()
            )
        );
        for guard in [
            "do {let ordered=x == x;ordered}",
            "if x < 0.0 {let negative=x < 0.0;not negative} else {false}",
            "do {let flag=true;let alias=flag;not alias}",
        ] {
            let source = format!(
                "typealias Float=.{width}'builtin;fn bad(value:Float)->bool {{value match {{_ as x if {guard}=>true}}}}"
            );
            assert!(
                execution_support::compile_with_tuple_protocols(&source, &Default::default())
                    .is_err(),
                "{source}"
            );
        }
    }
}

#[test]
fn shared_interval_boolean_locals_preserve_original_subject() {
    use execution_support::compile_with_protocols;
    let package = execution_support::compile_with_tuple(
        "typealias Signed=.i32'builtin;trait Duplicate {}trait Retire {fn drop(*self);}
        fn negative(value:Signed)->bool {value match {
            _ as x if do {let negative=x < 0;let x:Signed=1;negative == true}=>true,
            _ as x if do {let negative=x < 0;let alias=negative;alias == false}=>false,
        }}
        fn run()->Tuple<bool,bool,bool> {(negative(-1),negative(0),negative(1))}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100000)
            .unwrap(),
        Value::Tuple([true, false, false].into_iter().map(Value::Bool).collect())
    );
    for guard in [
        "do {let negative=x < 0;negative}",
        "do {let negative=decide(x);negative or not negative}",
    ] {
        let source = format!(
            "typealias Signed=.i32'builtin;fn decide(x:Signed)->bool {{true}} fn bad(value:Signed)->bool {{value match {{_ as x if {guard}=>true}}}}"
        );
        assert!(
            execution_support::compile_with_tuple_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn boolean_guard_comparisons_preserve_predicate_regions() {
    use execution_support::compile_with_protocols;
    for width in ["f32", "f64"] {
        let package = execution_support::compile_with_tuple(&format!(
            "typealias Float=.{width}'builtin;
            trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            fn ordered(value:Float)->bool {{value match {{
                _ as x if do {{let ordered=x == x;ordered == true}}=>true,
                _ as x if do {{let ordered=x == x;false == ordered}}=>false,
            }}}}
            fn same(value:Float)->bool {{value match {{
                _ as x if (x < 0.0) == (x >= 0.0)=>true,
                _ as x if (x < 0.0) != (x >= 0.0)=>false,
            }}}}
            fn run()->Tuple<bool,bool,bool,bool,bool> {{
                (ordered(Float.NaN),ordered(-0.0),same(Float.NaN),same(-1.0),same(1.0))
            }}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100000
            )
            .unwrap(),
            Value::Tuple(
                [false, true, true, false, false]
                    .into_iter()
                    .map(Value::Bool)
                    .collect()
            )
        );
        for guard in [
            "(x == x) == true",
            "(x < 0.0) == (x >= 0.0)",
            "(x == x) != false",
        ] {
            let source = format!(
                "typealias Float=.{width}'builtin;fn bad(value:Float)->bool {{value match {{_ as x if {guard}=>true}}}}"
            );
            assert!(
                execution_support::compile_with_tuple_protocols(&source, &Default::default())
                    .is_err(),
                "{source}"
            );
        }
    }
}

#[test]
fn boolean_matches_in_guards_preserve_selection_and_bound_values() {
    use execution_support::compile_with_protocols;
    for width in ["f32", "f64"] {
        let package=execution_support::compile_with_tuple(&format!(
            "typealias Float=.{width}'builtin;
            trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            fn ordered(value:Float)->bool {{value match {{
                _ as x if (x == x) match {{true=>true,false=>false}}=>true,
                _ as x if (x == x) match {{true=>false,false=>true}}=>false,
            }}}}
            fn all(value:Float)->bool {{value match {{
                _ as x if (x < 0.0) match {{true as flag if not flag=>false,flag=>flag == (x < 0.0)}}=>true,
            }}}}
            fn run()->Tuple<bool,bool,bool,bool> {{(ordered(Float.NaN),ordered(-0.0),all(-1.0),all(Float.NaN))}}
            "
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100000
            )
            .unwrap(),
            Value::Tuple(
                [false, true, true, true]
                    .into_iter()
                    .map(Value::Bool)
                    .collect()
            )
        );
        let source = format!(
            "typealias Float=.{width}'builtin;fn bad(value:Float)->bool {{value match {{_ as x if (x == x) match {{true=>true,false=>false}}=>true}}}}"
        );
        assert!(
            execution_support::compile_with_tuple_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn constant_matches_in_guards_keep_constant_bindings() {
    use execution_support::compile_with_protocols;
    for kind in ["f32", "f64", "Float"] {
        let package = execution_support::compile_with_tuple(&format!(
            "typealias Float=.{kind}'builtin;
            comptime fn zero(value:Float)->bool {{value match {{
                _ as x if do {{let c:Float=0.0;c match {{1.0=>false,0.0 as bound if bound == x=>true,_=>false}}}}=>true,
                _ as x if do {{let c:Float=0.0;c match {{1.0=>true,0.0 as bound if bound != x=>true,_=>false}}}}=>false,
            }}}}
            comptime fn run()->Tuple<bool,bool,bool,bool> {{(zero(0.0),zero(-0.0),zero(1.0),zero(0.0000000000000000000000000000001))}}"
        ));
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100000).unwrap(),
            Value::Tuple(
                [true, true, false, false]
                    .into_iter()
                    .map(Value::Bool)
                    .collect()
            )
        );
        for guard in [
            "do {let c:Float=0.0;c match {0.0 as bound=>bound == x,_=>false}}",
            "do {let c:Float=0.0;c match {1.0=>true,_=>false}}",
        ] {
            let source = format!(
                "typealias Float=.{kind}'builtin;comptime fn bad(value:Float)->bool {{value match {{_ as x if {guard}=>true}}}}"
            );
            assert!(
                execution_support::compile_with_tuple_protocols(&source, &Default::default())
                    .is_err(),
                "{source}"
            );
        }
    }
}

#[test]
fn constant_match_guard_runtime_preserves_nan_and_signed_zero() {
    for kind in ["f32", "f64"] {
        let package=execution_support::compile_with_tuple(&format!(
            "typealias Float=.{kind}'builtin;
            trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            fn zero(value:Float)->bool {{value match {{
                _ as x if do {{let c:Float=0.0;c match {{0.0 as bound if bound == x=>true,_=>false}}}}=>true,
                _ as x if do {{let c:Float=0.0;c match {{0.0 as bound if bound != x=>true,_=>false}}}}=>false,
            }}}}
            fn run()->Tuple<bool,bool,bool,bool,bool> {{(zero(0.0),zero(-0.0),zero(1.0),zero(Float.NaN),zero(Float.Infinity))}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100000
            )
            .unwrap(),
            Value::Tuple(
                [true, true, false, false, false]
                    .into_iter()
                    .map(Value::Bool)
                    .collect()
            )
        );
    }
}

#[test]
fn machine_float_guards_skip_unreachable_calls_but_keep_nan_paths() {
    use execution_support::compile_with_protocols;
    for kind in ["f32", "f64"] {
        for guard in [
            "x == x or (x != x or decide(x))",
            "if x == x {true} else {x != x or decide(x)}",
            "x match {_ if x == x=>true,_ if x != x=>true,_=>decide(x)}",
            "not (x == x and (x != x and decide(x)))",
        ] {
            let package=execution_support::compile_with_tuple(&format!(
                "typealias Float=.{kind}'builtin;
                trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                fn decide(x:Float)->bool {{false}}
                fn all(value:Float)->bool {{value match {{_ as x if {guard}=>true}}}}
                fn run()->Tuple<bool,bool,bool,bool> {{(all(-1.0),all(-0.0),all(Float.NaN),all(Float.Infinity))}}"
            ));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100000
                )
                .unwrap(),
                Value::Tuple(vec![Value::Bool(true); 4])
            );
        }
        for guard in [
            "x < 0.0 or (x >= 0.0 or decide(x))",
            "if x == x {true} else {decide(x)}",
            "x match {_ if x == x=>true,_=>decide(x)}",
            "not (x != x and decide(x))",
            "decide(x) or (x == x or x != x)",
        ] {
            let source = format!(
                "typealias Float=.{kind}'builtin;fn decide(x:Float)->bool {{true}} fn bad(value:Float)->bool {{value match {{_ as x if {guard}=>true}}}}"
            );
            assert!(
                execution_support::compile_with_tuple_protocols(&source, &Default::default())
                    .is_err(),
                "{source}"
            );
        }
    }
}

#[test]
fn computed_float_patterns_preserve_nan_and_numeric_result_domains() {
    use execution_support::compile_with_protocols;
    for width in ["f32", "f64", "Float"] {
        let phase = if width == "Float" { "comptime " } else { "" };
        for value in ["-1.0", "-0.0", "0.0", "1.0"] {
            let source = format!(
                "typealias Number=.{width}'builtin;
                {phase}fn classify(input:Number)->bool {{
                    input match {{
                        value and (value < 0.0) is true=>false,
                        value and (value >= 0.0) is true=>true,
                        value and (not (value == value)) is true=>false,
                    }}
                }}
                {phase}fn run()->bool {{classify({value})}}"
            );
            let package = compile(&source);
            let actual = if width == "Float" {
                luna::invocation::invoke(&package, "run", 100_000)
            } else {
                luna::execution::interpret(&package, "run", 100_000)
            };
            assert_eq!(
                actual.unwrap(),
                Value::Bool(value != "-1.0"),
                "{width}: {value}"
            );
            if width != "Float" {
                let incomplete =
                    source.replace("value and (not (value == value)) is true=>false,", "");
                let error = compile_with_protocols(&incomplete, &Default::default()).unwrap_err();
                assert!(error.contains("exhaustive"), "{error}");
                for special in ["Number.NaN", "Number.Infinity", "-Number.Infinity"] {
                    let special_source = source.replace(
                        &format!("classify({value})"),
                        &format!("classify({special})"),
                    );
                    let package = compile(&special_source);
                    assert_eq!(
                        luna::execution::interpret(&package, "run", 100_000).unwrap(),
                        Value::Bool(special == "Number.Infinity")
                    );
                }
            }
        }
        let numeric = format!(
            "typealias Number=.{width}'builtin;
            {phase}fn identity(input:Number)->bool {{
                input match {{
                    value and value is projected if projected < 0.0=>false,
                    value and value is projected if not (projected < 0.0)=>true,
                }}
            }}
            {phase}fn run()->bool {{identity(1.0)}}"
        );
        let package = compile(&numeric);
        let actual = if width == "Float" {
            luna::invocation::invoke(&package, "run", 100_000)
        } else {
            luna::execution::interpret(&package, "run", 100_000)
        };
        assert_eq!(actual.unwrap(), Value::Bool(true));
    }
}

#[test]
fn machine_float_coverage_skips_opaque_calls_with_aggregate_inputs() {
    use execution_support::compile_with_protocols;
    for width in ["f32", "f64"] {
        for (declarations, call) in [
            (
                "fn decide(packet:Tuple<Float,bool>)->bool {false}",
                "decide((x,true))",
            ),
            (
                "struct Packet {value:Float,flag:bool} fn decide(packet:Packet)->bool {false}",
                "decide(Packet {value:x,flag:true})",
            ),
            (
                "enum Packet {empty,item(Float)} fn decide(packet:Packet)->bool {false}",
                "decide(Packet.item(x))",
            ),
            (
                "fn decide(value:Float,tag:.char'builtin)->bool {false}",
                "decide(x,'a')",
            ),
            (
                "fn decide(value:Float,count:.u8'builtin)->bool {false}",
                "decide(x,9)",
            ),
            (
                "fn identity(value:Float)->bool {false} fn decide(action:fn(Float)->bool,value:Float)->bool {false}",
                "decide(identity,x)",
            ),
            (
                "fn decide(packet:T)->bool where T {false}",
                "decide((x,true))",
            ),
        ] {
            let source = format!(
                "typealias Float=.{width}'builtin;
                trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                typealias Action=fn(Float)->bool;impl Duplicate for Action {{}}
                {declarations}
                fn all(value:Float)->bool {{value match {{
                    _ as x if x == x or (x != x or {call})=>true,
                }}}}
                fn run()->Tuple<bool,bool,bool,bool,bool> {{
                    (all(-1.0),all(-0.0),all(0.0),all(Float.NaN),all(Float.Infinity))
                }}"
            );
            let package = execution_support::compile_with_tuple(&source);
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000
                )
                .unwrap(),
                Value::Tuple(vec![Value::Bool(true); 5]),
                "{width}: {call}"
            );
            // Also inspect the execution trace: coverage must not invoke the
            // skipped predicate while establishing or executing the branch.
            let mut vm =
                comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
            assert_eq!(
                vm.invoke(execution_support::id(&package, "run"), vec![])
                    .unwrap(),
                Value::Tuple(vec![Value::Bool(true); 5]),
                "{width}: {call}"
            );
            assert!(
                !execution_support::calls(&package, &vm)
                    .iter()
                    .any(|name| name == "decide")
            );
            // NaN must reach the call; its body cannot serve as proof evidence.
            let incomplete = source.replace("x == x or (x != x or", "x < 0.0 or (x >= 0.0 or");
            let failure =
                execution_support::compile_with_tuple_protocols(&incomplete, &Default::default())
                    .unwrap_err();
            assert!(failure.contains("exhaustive"), "{width}: {call}: {failure}");
        }
    }
}
