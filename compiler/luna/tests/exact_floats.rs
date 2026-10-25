//! Exact comptime Float source semantics, independently runnable.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod execution_support;
use execution_support::{compile, compile_with_protocols, compile_with_source_bindings, id};

#[test]
fn exact_float_literals_flow_through_comptime_calls_and_suffix_protocols() {
    let package = compile_with_source_bindings(
        r#"
        typealias Float = .Float'builtin;
        typealias Symbol = .Symbol'builtin;
        comptime fn identity(value:Float)->Float {value}
        comptime fn run()->Float {identity(1.0000000000000000000000000000001)}
        trait Convert where suffix:Symbol,R {comptime fn literal_into(comptime value:Self)->R;}
        impl Convert<.exact,Float> for Float {
            comptime fn literal_into(comptime value:Float)->Float {identity(value)}
        }
        comptime fn suffix()->Float {1.0000000000000000000000000000002exact}
        comptime fn negative()->Float { -0.0 }
        const SAVED = identity(1.0000000000000000000000000000003);
        const NEGATIVE_ZERO = identity(-0.0);
        const HUGE = identity(1.0e9999999999999999999999999999999999999999);
        comptime fn saved()->Float {SAVED}
        comptime fn saved_zero()->Float {NEGATIVE_ZERO}
        comptime fn huge()->Float {HUGE}
    "#,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            literal_into: Some("input::Convert"),
            ..Default::default()
        },
    )
    .unwrap();
    for (name, text) in [
        ("run", "1.0000000000000000000000000000001"),
        ("suffix", "1.0000000000000000000000000000002"),
        ("negative", "-0.0"),
        ("saved", "1.0000000000000000000000000000003"),
        ("saved_zero", "-0.0"),
        ("huge", "1.0e9999999999999999999999999999999999999999"),
    ] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let actual = vm.invoke(id(&package, name), vec![]).unwrap();
        let expected = comptime_runtime::decimal::DecimalValue::parse_literal(
            text,
            &mut comptime_runtime::Budget::default(),
        )
        .unwrap();
        assert_eq!(actual, Value::Float(expected), "{name}");
    }
}

#[test]
fn exact_float_source_values_cannot_leak_into_runtime_storage() {
    for source in [
        "typealias Float=.Float'builtin;fn leak()->Float {1.0}",
        "typealias Float=.Float'builtin;fn leak(value:Float)->bool {true}",
        "typealias Float=.Float'builtin;fn leak(comptime value:Float)->bool {value < 1.0}",
        "typealias Float=.Float'builtin;const SAVED:Float=1.0;fn leak()->Float {SAVED}",
    ] {
        assert!(
            compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn exact_float_source_comparisons_use_decimal_values() {
    let package = compile(
        r#"
        typealias Float=.Float'builtin;
        comptime fn eq(a:Float,b:Float)->bool {a == b}
        comptime fn ne(a:Float,b:Float)->bool {a != b}
        comptime fn lt(a:Float,b:Float)->bool {a < b}
        comptime fn le(a:Float,b:Float)->bool {a <= b}
        comptime fn gt(a:Float,b:Float)->bool {a > b}
        comptime fn ge(a:Float,b:Float)->bool {a >= b}
        const LEFT:Float=1.0000000000000000000000000000001;
        const RIGHT:Float=1.0000000000000000000000000000002;
        comptime fn staged()->bool {lt(LEFT,RIGHT) and ne(LEFT,RIGHT)}
    "#,
    );
    let cases = [
        ("-1.0e9999999999999999999999999999999999999999", -3),
        ("-0.000000000000000000000001", -2),
        ("-0.0", 0),
        ("0.0", 0),
        ("1.0000000000000000000000000000001", 1),
        ("10.000000000000000000000000000001e-1", 1),
        ("1.0000000000000000000000000000002", 2),
        ("1.0e9999999999999999999999999999999999999999", 3),
    ];
    for (a, rank_a) in cases {
        for (b, rank_b) in cases {
            for (name, expected) in [
                ("eq", rank_a == rank_b),
                ("ne", rank_a != rank_b),
                ("lt", rank_a < rank_b),
                ("le", rank_a <= rank_b),
                ("gt", rank_a > rank_b),
                ("ge", rank_a >= rank_b),
            ] {
                let value = |text| {
                    Value::Float(
                        comptime_runtime::decimal::DecimalValue::parse_literal(
                            text,
                            &mut comptime_runtime::Budget::default(),
                        )
                        .unwrap(),
                    )
                };
                let actual = Interpreter::for_comptime(&package, Budget::default())
                    .unwrap()
                    .invoke(id(&package, name), vec![value(a), value(b)])
                    .unwrap();
                assert_eq!(actual, Value::Bool(expected), "{a} {name} {b}");
            }
        }
    }
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "staged"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn exact_float_suffix_can_generate_runtime_results_from_precise_comparisons() {
    let package = compile_with_source_bindings(
        r#"
        typealias Type=.Type'builtin;
        trait Splice {assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;}
        impl Splice for bool {assoc RuntimeTarget:Type=bool;fn splice(comptime data:bool)->bool {data}}
        typealias Float=.Float'builtin;
        typealias Symbol=.Symbol'builtin;
        trait Convert where suffix:Symbol,R {comptime fn literal_into(comptime value:Self)->R;}
        impl Convert<.below,bool> for Float {
            comptime fn literal_into(comptime value:Float)->bool {
                value < 1.0000000000000000000000000000002
            }
        }
        fn below()->bool {1.0000000000000000000000000000001below}
        fn equal()->bool {1.0000000000000000000000000000002below}
        fn above()->bool {1.0000000000000000000000000000003below}
    "#,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            literal_into: Some("input::Convert"),
            splice: Some("Splice"),
            ..Default::default()
        },
    )
    .unwrap();
    for (name, expected) in [("below", true), ("equal", false), ("above", false)] {
        let value = luna::execution::interpret(&package, name, 100_000).unwrap();
        assert_eq!(value, Value::Bool(expected), "{name}");
    }
}

#[test]
fn exact_float_negation_preserves_values_through_calls_constants_and_zero_signs() {
    let package = compile(
        r#"
        typealias Float=.Float'builtin;
        comptime fn negate(value:Float)->Float { -value }
        comptime fn twice(value:Float)->Float { -negate(value) }
        comptime fn identity(value:Float)->Float {value}
        const NEGATIVE=negate(1.0000000000000000000000000000001);
        const ZERO=negate(0.0);
        comptime fn saved()->Float {NEGATIVE}
        comptime fn zero()->Float {ZERO}
        comptime fn call()->Float { -identity(1.0e9999999999999999999999999999999999999999) }
    "#,
    );
    let value = |text| {
        Value::Float(
            comptime_runtime::decimal::DecimalValue::parse_literal(
                text,
                &mut comptime_runtime::Budget::default(),
            )
            .unwrap(),
        )
    };
    for (positive, negative) in [
        ("0.0", "-0.0"),
        ("0.000000000000001", "-0.000000000000001"),
        (
            "1.0000000000000000000000000000001",
            "-1.0000000000000000000000000000001",
        ),
        (
            "1.0e9999999999999999999999999999999999999999",
            "-1.0e9999999999999999999999999999999999999999",
        ),
    ] {
        for (input, expected) in [(positive, negative), (negative, positive)] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(id(&package, "negate"), vec![value(input)])
                    .unwrap(),
                value(expected)
            );
            assert_eq!(
                vm.invoke(id(&package, "twice"), vec![value(input)])
                    .unwrap(),
                value(input)
            );
        }
    }
    for (name, expected) in [
        ("saved", "-1.0000000000000000000000000000001"),
        ("zero", "-0.0"),
        ("call", "-1.0e9999999999999999999999999999999999999999"),
    ] {
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(id(&package, name), vec![])
                .unwrap(),
            value(expected)
        );
    }
    assert!(
        compile_with_protocols(
            "typealias Float=.Float'builtin;fn bad(comptime value:Float)->Float { -value }",
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn exact_float_arithmetic_preserves_small_differences_and_constant_results() {
    let package = compile(
        r#"
        typealias Float=.Float'builtin;
        comptime fn add(a:Float,b:Float)->Float {a + b}
        comptime fn subtract(a:Float,b:Float)->Float {a - b}
        comptime fn multiply(a:Float,b:Float)->Float {a * b}
        const SUM=add(0.1,0.2);
        const DIFFERENCE=subtract(1.0000000000000000000000000000001,1.0);
        const PRODUCT=multiply(1.0000000000000000000000000000001,10.0);
        comptime fn sum()->Float {SUM}
        comptime fn literal_sum()->Float {0.1 + 0.2}
        comptime fn difference()->Float {DIFFERENCE}
        comptime fn product()->Float {PRODUCT}
        comptime fn combined(a:Float,b:Float)->Float {(a + b) * (a - b)}
    "#,
    );
    let value = |text| {
        Value::Float(
            comptime_runtime::decimal::DecimalValue::parse_literal(
                text,
                &mut comptime_runtime::Budget::default(),
            )
            .unwrap(),
        )
    };
    for (name, expected) in [
        ("sum", "0.3"),
        ("literal_sum", "0.3"),
        ("difference", "0.0000000000000000000000000000001"),
        ("product", "10.000000000000000000000000000001"),
    ] {
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(id(&package, name), vec![])
                .unwrap(),
            value(expected)
        );
    }
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "combined"), vec![value("2.5"), value("1.5")])
            .unwrap(),
        value("4.0")
    );
    for operation in ["/", "%"] {
        let source = format!(
            "typealias Float=.Float'builtin;comptime fn bad(a:Float,b:Float)->Float {{a {operation} b}}"
        );
        assert!(compile_with_protocols(&source, &Default::default()).is_err());
    }
    assert!(
        compile_with_protocols(
            "typealias Float=.Float'builtin;fn bad(comptime a:Float)->Float {a + 1.0}",
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn exact_float_patterns_use_numeric_equality_and_support_negation() {
    let package = compile(
        r#"
        typealias Float=.Float'builtin;
        comptime fn precise(value:Float)->bool {value match {
            1.0000000000000000000000000000001 => true,
            _ => false,
        }}
        comptime fn nonzero(value:Float)->bool {value match {not 0.0=>true,0.0=>false}}
        comptime fn alternative(value:Float)->bool {value match {
            0.0 or 1.0000000000000000000000000000001 => true,
            _ => false,
        }}
        const RESULT=precise(10.000000000000000000000000000001e-1);
        comptime fn saved()->bool {RESULT}
    "#,
    );
    let value = |text: &str| {
        Value::Float(
            comptime_runtime::decimal::DecimalValue::parse_literal(
                text,
                &mut comptime_runtime::Budget::default(),
            )
            .unwrap(),
        )
    };
    for (input, precise, nonzero, alternative) in [
        ("0.0", false, false, true),
        ("-0.0", false, false, true),
        ("1.0000000000000000000000000000001", true, true, true),
        ("10.000000000000000000000000000001e-1", true, true, true),
        ("1.0000000000000000000000000000002", false, true, false),
    ] {
        for (name, expected) in [
            ("precise", precise),
            ("nonzero", nonzero),
            ("alternative", alternative),
        ] {
            assert_eq!(
                Interpreter::for_comptime(&package, Budget::default())
                    .unwrap()
                    .invoke(id(&package, name), vec![value(input)])
                    .unwrap(),
                Value::Bool(expected),
                "{name} {input}"
            );
        }
    }
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "saved"), vec![])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn exact_float_coverage_preserves_tuple_correlations_and_rejects_executable_guards() {
    let package = execution_support::compile_with_tuple(
        r#"
        typealias Float=.Float'builtin;
        comptime fn classify(value:Tuple<Float,bool>)->bool {value match {
            (0.0,true)=>true,(not 0.0,_)=>true,(0.0,false)=>true,
        }}
        comptime fn nested(value:Float)->bool {value match {
            not (0.0 or 1.0)=>true,0.0=>true,1.0=>true,
        }}
    "#,
    );
    for text in ["0.0", "-0.0", "1.0", "1.0000000000000000000000000000001"] {
        let value = Value::Float(
            comptime_runtime::decimal::DecimalValue::parse_literal(
                text,
                &mut comptime_runtime::Budget::default(),
            )
            .unwrap(),
        );
        for flag in [false, true] {
            assert_eq!(
                Interpreter::for_comptime(&package, Budget::default())
                    .unwrap()
                    .invoke(
                        id(&package, "classify"),
                        vec![Value::Tuple(vec![value.clone(), Value::Bool(flag)])]
                    )
                    .unwrap(),
                Value::Bool(true)
            );
        }
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(id(&package, "nested"), vec![value])
                .unwrap(),
            Value::Bool(true)
        );
    }
    for body in ["value match {(0.0,true)=>true,(not 0.0,_)=>false}"] {
        let source = format!(
            "typealias Float=.Float'builtin;comptime fn bad(value:Tuple<Float,bool>)->bool {{{body}}}"
        );
        let error = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_err();
        assert!(error.contains("non-exhaustive match"), "{error}");
    }
    for pattern in [
        "not (_ if accepts(value))",
        "not (0.0 or (_ if accepts(value)))",
    ] {
        let source = format!(
            "typealias Float=.Float'builtin;comptime fn accepts(value:Float)->bool {{true}} comptime fn bad(value:Float)->bool {{value match {{{pattern}=>true,0.0=>false}}}}"
        );
        let error = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_err();
        assert!(error.contains("non-exhaustive match"), "{error}");
    }
}

#[test]
fn exact_float_pure_equality_guards_cover_complements_and_keep_subject_identity() {
    let package = execution_support::compile_with_tuple(
        r#"
        typealias Float=.Float'builtin;
        comptime fn reflexive(value:Float)->bool {value match {_ as x if x == x => true}}
        comptime fn guard(value:Float)->bool {value match {
            _ as x if x == -0.0 => true,
            _ as y if 0.0 != y => false,
        }}
        comptime fn close(value:Float)->bool {value match {
            _ as x if x == 1.0000000000000000000000000000001 => true,
            _ as y if y != 10.000000000000000000000000000001e-1 => false,
        }}
        comptime fn negative(value:Float)->bool {value match {
            < -1.25 > => true, not < -1.25 > => false,
        }}
        comptime fn negative_tuple(value:Tuple<Float,bool>)->bool {value match {
            ( < -0.0 > ,true)=>true,(not < -0.0 > ,_)=>false,(0.0,false)=>true,
        }}
    "#,
    );
    let value = |text: &str| {
        Value::Float(
            comptime_runtime::decimal::DecimalValue::parse_literal(
                text,
                &mut comptime_runtime::Budget::default(),
            )
            .unwrap(),
        )
    };
    for (name, input, expected) in [
        ("reflexive", "-0.0", true),
        ("reflexive", "1.0000000000000000000000000000002", true),
        ("guard", "0.0", true),
        ("guard", "-0.0", true),
        ("guard", "1.0", false),
        ("close", "1.0000000000000000000000000000001", true),
        ("close", "1.0000000000000000000000000000002", false),
        ("negative", "-1.25", true),
        ("negative", "1.25", false),
    ] {
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(id(&package, name), vec![value(input)])
                .unwrap(),
            Value::Bool(expected)
        );
    }
    for flag in [false, true] {
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(
                    id(&package, "negative_tuple"),
                    vec![Value::Tuple(vec![value("-0.0"), Value::Bool(flag)])]
                )
                .unwrap(),
            Value::Bool(true)
        );
    }
    for guards in [
        "_ as x if x == 0.0 => true, _ as y if y != 1.0 => false",
        "_ as x if x == 0.0 => true, _ as y if other != 0.0 => false",
        "_ as x if x == 0.0 => true, _ as y if accepts(y) => false",
    ] {
        let source = format!(
            "typealias Float=.Float'builtin;comptime fn accepts(value:Float)->bool {{true}} comptime fn bad(value:Float,other:Float)->bool {{value match {{{guards}}}}}"
        );
        let error = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_err();
        assert!(error.contains("non-exhaustive match"), "{error}");
    }
}

#[test]
fn exact_float_ordered_guards_cover_intervals_without_rounding_endpoints() {
    let package = compile(
        r#"
        typealias Float=.Float'builtin;
        comptime fn sign(value:Float)->.u8'builtin {value match {
            _ as x if x < 0.0 => 0,
            _ as x if x == -0.0 => 1,
            _ as x if 0.0 < x => 2,
        }}
        comptime fn window(value:Float)->bool {value match {
            _ as x if x < 1.0000000000000000000000000000001 => false,
            _ as x if x >= 1.0000000000000000000000000000001 and x <= 1.0000000000000000000000000000002 => true,
            _ as x if 1.0000000000000000000000000000002 < x => false,
        }}
        comptime fn extreme(value:Float)->bool {value match {
            _ as x if x <= 1.0e9999999999999999999999999999999999999999 => true,
            _ as x if x > 1.0e9999999999999999999999999999999999999999 => false,
        }}
    "#,
    );
    let value = |text: &str| {
        Value::Float(
            comptime_runtime::decimal::DecimalValue::parse_literal(
                text,
                &mut comptime_runtime::Budget::default(),
            )
            .unwrap(),
        )
    };
    for (input, expected) in [("-1.0", 0), ("-0.0", 1), ("0.0", 1), ("1.0", 2)] {
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(id(&package, "sign"), vec![value(input)])
                .unwrap(),
            Value::UnsignedInteger(expected)
        );
    }
    for (input, expected) in [
        ("1.0", false),
        ("1.0000000000000000000000000000001", true),
        ("1.00000000000000000000000000000015", true),
        ("1.0000000000000000000000000000002", true),
        ("1.0000000000000000000000000000003", false),
    ] {
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(id(&package, "window"), vec![value(input)])
                .unwrap(),
            Value::Bool(expected)
        );
    }
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "extreme"), vec![value("2.0")])
            .unwrap(),
        Value::Bool(true)
    );
    for arms in [
        "_ as x if x < 0.0=>true,_ as x if x > 0.0=>false",
        "_ as x if x <= 0.0=>true,_ as x if x >= 1.0=>false",
        "_ as x if x < x=>true",
        "_ as x if x <= 0.0=>true,_ as x if other > 0.0=>false",
    ] {
        let source = format!(
            "typealias Float=.Float'builtin;comptime fn bad(value:Float,other:Float)->bool {{value match {{{arms}}}}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("non-exhaustive match"), "{error}");
    }
}

#[test]
fn exact_float_range_patterns_preserve_endpoints_negation_and_evaluation_order() {
    let package = compile(
        r#"
        typealias Float=.Float'builtin;
        comptime fn closed(value:Float)->bool {value match {
            1.0000000000000000000000000000001..=1.0000000000000000000000000000002=>true,
            not (1.0000000000000000000000000000001..=1.0000000000000000000000000000002)=>false,
        }}
        comptime fn open(value:Float)->bool {value match {0.0..1.0=>true,not (0.0..1.0)=>false}}
        comptime fn one_sided(value:Float)->.u8'builtin {value match {..0.0=>0,0.0..=>1}}
        comptime fn empty(value:Float)->bool {value match {1.0..1.0=>true,not (1.0..1.0)=>false}}
        comptime fn dynamic(value:Float,lower:Float,upper:Float)->bool {value match {lower..=upper=>true,_=>false}}
        comptime fn ordered(sample:Float)->bool {
            var count:.u8'builtin=0;
            let result:bool=sample match {
                < do {count += 1;0.0} > .. < do {assumes count == 1;count += 2;1.0} > => true,
                _=>false,
            };
            (result == (sample >= 0.0 and sample < 1.0)) and count == 3
        }
    "#,
    );
    let value = |text: &str| {
        Value::Float(
            comptime_runtime::decimal::DecimalValue::parse_literal(
                text,
                &mut comptime_runtime::Budget::default(),
            )
            .unwrap(),
        )
    };
    for (name, input, expected) in [
        ("closed", "1.0", false),
        ("closed", "1.0000000000000000000000000000001", true),
        ("closed", "1.00000000000000000000000000000015", true),
        ("closed", "1.0000000000000000000000000000002", true),
        ("closed", "1.0000000000000000000000000000003", false),
        ("open", "-0.0", true),
        ("open", "1.0", false),
        ("empty", "1.0", false),
    ] {
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(id(&package, name), vec![value(input)])
                .unwrap(),
            Value::Bool(expected),
            "{name} {input}"
        );
    }
    for (input, expected) in [("-1.0", 0), ("-0.0", 1), ("1.0", 1)] {
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(id(&package, "one_sided"), vec![value(input)])
                .unwrap(),
            Value::UnsignedInteger(expected)
        );
    }
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(
                id(&package, "dynamic"),
                vec![value("0.5"), value("0.0"), value("1.0")]
            )
            .unwrap(),
        Value::Bool(true)
    );
    for input in ["-1.0", "0.5", "2.0"] {
        assert_eq!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke(id(&package, "ordered"), vec![value(input)])
                .unwrap(),
            Value::Bool(true)
        );
    }
    for arms in [
        "..0.0=>true,0.0000000000000000000000000000001..=>false",
        "lower..=upper=>true",
    ] {
        let source = format!(
            "typealias Float=.Float'builtin;comptime fn bad(value:Float,lower:Float,upper:Float)->bool {{value match {{{arms}}}}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("non-exhaustive match"), "{error}");
    }
}

#[test]
fn exact_float_alternatives_export_common_bindings_to_coverage_guards() {
    let package = compile(
        r#"
        typealias Float=.Float'builtin;
        comptime fn selected(value:Float)->bool {value match {
            ((0.0 as x) or (1.0 as x)) if x >= 0.0 => true,
            not (0.0 or 1.0) => false,
        }}
        comptime fn overlapping(value:Float)->bool {value match {
            ((_ as x) or (1.0 as x)) if x < 0.0 => true,
            _ as x if x >= 0.0 => false,
        }}
        comptime fn nested(value:Float)->bool {value match {
            (((0.0 as a) as b) or (((1.0 as b) or (2.0 as b)) as a)) if a == b => true,
            not (0.0 or 1.0 or 2.0) => false,
        }}
    "#,
    );
    let value = |text: &str| {
        Value::Float(
            comptime_runtime::decimal::DecimalValue::parse_literal(
                text,
                &mut comptime_runtime::Budget::default(),
            )
            .unwrap(),
        )
    };
    for (input, selected, overlapping, nested) in [
        ("-1.0", false, true, false),
        ("-0.0", true, false, true),
        ("1.0", true, false, true),
        ("2.0", false, false, true),
        ("1.0000000000000000000000000000001", false, false, false),
    ] {
        for (name, expected) in [
            ("selected", selected),
            ("overlapping", overlapping),
            ("nested", nested),
        ] {
            assert_eq!(
                Interpreter::for_comptime(&package, Budget::default())
                    .unwrap()
                    .invoke(id(&package, name), vec![value(input)])
                    .unwrap(),
                Value::Bool(expected),
                "{name} {input}"
            );
        }
    }
    for arms in [
        "((0.0 as x) or (1.0 as x)) if x > 0.0=>true,not (0.0 or 1.0)=>false",
        "((0.0 as x) or (1.0 as x)) if accepts(x)=>true,not (0.0 or 1.0)=>false",
    ] {
        let source = format!(
            "typealias Float=.Float'builtin;comptime fn accepts(value:Float)->bool {{true}} comptime fn bad(value:Float)->bool {{value match {{{arms}}}}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("non-exhaustive match"), "{error}");
    }
}

#[test]
fn exact_float_guard_boolean_locals_and_conditionals_keep_decimal_precision() {
    let package=execution_support::compile_with_tuple(
        "typealias Float=.Float'builtin;
        comptime fn split(value:Float)->bool {value match {
            _ as x if do {let low=x < 1.0000000000000000000000000000001;let alias=low;alias == true}=>true,
            _ as x if do {let low=x < 1.0000000000000000000000000000001;low == false}=>false,
        }}
        comptime fn all(value:Float)->bool {value match {
            _ as x if do {let low=x < 0.0;if low {low == true} else {not low}}=>true,
        }}
        comptime fn run()->Tuple<bool,bool,bool,bool> {
            (split(1.0000000000000000000000000000000),split(1.0000000000000000000000000000001),all(-0.0),all(-1.0))
        }"
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100000).unwrap(),
        Value::Tuple(
            [true, false, true, true]
                .into_iter()
                .map(Value::Bool)
                .collect()
        )
    );
    for guard in [
        "do {let low=x < 0.0;low}",
        "if x < 0.0 {let low=x < 0.0;not low} else {false}",
        "do {let result=decide(x);result == result}",
    ] {
        let source = format!(
            "typealias Float=.Float'builtin;comptime fn decide(x:Float)->bool {{true}} comptime fn bad(value:Float)->bool {{value match {{_ as x if {guard}=>true}}}}"
        );
        assert!(
            execution_support::compile_with_tuple_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn exact_float_nested_match_guards_preserve_first_matching_arm() {
    let package=execution_support::compile_with_tuple(
        "typealias Float=.Float'builtin;
        comptime fn split(value:Float)->bool {value match {
            _ as x if (x < 1.0000000000000000000000000000001) match {true=>true,false=>false}=>true,
            _ as x if (x < 1.0000000000000000000000000000001) match {true=>false,false=>true}=>false,
        }}
        comptime fn all(value:Float)->bool {value match {
            _ as x if (x < 0.0) match {true as flag if not flag=>false,flag=>flag == (x < 0.0)}=>true,
        }}
        comptime fn numeric(value:Float)->bool {value match {
            _ as x if x match {..0.0=>true,0.0=>true,_=>true}=>true,
        }}
        comptime fn run()->Tuple<bool,bool,bool,bool,bool> {
            (split(1.0000000000000000000000000000000),split(1.0000000000000000000000000000001),all(-1.0),all(1.0),numeric(-0.0))
        }"
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100000).unwrap(),
        Value::Tuple(
            [true, false, true, true, true]
                .into_iter()
                .map(Value::Bool)
                .collect()
        )
    );
    for guard in [
        "x match {_=>false,_=>true}",
        "(x < 0.0) match {true=>true,false=>false}",
    ] {
        let source = format!(
            "typealias Float=.Float'builtin;comptime fn bad(value:Float)->bool {{value match {{_ as x if {guard}=>true}}}}"
        );
        assert!(
            execution_support::compile_with_tuple_protocols(&source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn exact_float_guards_skip_unreachable_opaque_conditions() {
    for guard in [
        "(true or decide(x)) match {_=>true}",
        "(false and decide(x)) match {_=>true}",
        "do {let unused=true or decide(x);true}",
        "do {let unused=false and decide(x);not unused}",
        "do {let unused=if x < 0.0 {true} else {x >= 0.0 or decide(x)};true}",
        "x < 0.0 or (x >= 0.0 or decide(x))",
        "if x < 0.0 {true} else {x >= 0.0 or decide(x)}",
        "x match {..0.0=>true,0.0..=>true,_=>decide(x)}",
        "not (x < 0.0 and (x >= 0.0 and decide(x)))",
    ] {
        let package = execution_support::compile_with_tuple(&format!(
            "typealias Float=.Float'builtin;
            comptime fn decide(x:Float)->bool {{false}}
            comptime fn all(value:Float)->bool {{value match {{_ as x if {guard}=>true}}}}
            comptime fn run()->Tuple<bool,bool,bool> {{(all(-1.0),all(0.0),all(1.0))}}"
        ));
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100000).unwrap(),
            Value::Tuple(vec![Value::Bool(true); 3])
        );
    }
    for guard in [
        "(false or decide(x)) match {_=>true}",
        "not ((false or decide(x)) match {_=>false})",
        "(x < 0.0 or decide(x)) match {_=>true}",
        "do {let unused=(false or decide(x)) match {_=>true};true}",
        "do {let unused=false or decide(x);true}",
        "not do {let unused=false or decide(x);false}",
        "do {let unused=if x < 0.0 {true} else {decide(x)};true}",
        "do {let first=x >= 0.0 or decide(x);let second=true;second}",
        "x < 0.0 or decide(x)",
        "if x < 0.0 {true} else {decide(x)}",
        "x match {..0.0=>true,_=>decide(x)}",
        "not (x < 0.0 and decide(x))",
        "decide(x) or (x < 0.0 or x >= 0.0)",
    ] {
        let source = format!(
            "typealias Float=.Float'builtin;comptime fn decide(x:Float)->bool {{true}} comptime fn bad(value:Float)->bool {{value match {{_ as x if {guard}=>true}}}}"
        );
        let error = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_err();
        assert!(error.contains("exhaustive"), "{source}: {error}");
    }
}

#[test]
fn exact_float_or_patterns_export_selected_computed_booleans() {
    let source = "typealias Float=.Float'builtin;
        comptime fn classify(input:Float)->bool {
            input match {
                (((value if value < 0.0) and false is selected) or
                 ((value if value >= 0.0) and true is selected))
                    if selected == (value >= 0.0) => selected,
            }
        }
        comptime fn run()->Tuple<bool,bool,bool> {(classify(-1.0),classify(0.0),classify(1.0))}";
    let package = execution_support::compile_with_tuple(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Tuple(vec![
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(true)
        ])
    );
    let nested = source.replace(
        "((value if value >= 0.0) and true is selected)",
        "(((value if value == 0.0) and true is selected) or ((value if value > 0.0) and false is selected))",
    ).replace("selected == (value >= 0.0)", "selected == (value == 0.0)");
    let package = execution_support::compile_with_tuple(&nested);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Tuple(vec![
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(false)
        ])
    );
    // The first candidate now matches every input. Its false result must not
    // be replaced by the later candidate's true result after guard failure.
    let shadowed = source.replace("value if value < 0.0", "value");
    let error = execution_support::compile_with_tuple_protocols(&shadowed, &Default::default())
        .unwrap_err();
    assert!(error.contains("exhaustive"), "{error}");
    let wrong = source.replace("and true is selected", "and false is selected");
    let error =
        execution_support::compile_with_tuple_protocols(&wrong, &Default::default()).unwrap_err();
    assert!(error.contains("exhaustive"), "{error}");
}

#[test]
fn generic_float_literals_form_at_the_selected_width() {
    let source="fn midpoint()->T where T, requires T == .f32'builtin or T == .f64'builtin {1.0000000596046447753906250000000001}
        fn negative_zero()->T where T, requires T == .f32'builtin or T == .f64'builtin { -0.0 }
        fn run()->bool {
            let a=midpoint<.f32'builtin>(); let b=midpoint<.f64'builtin>();
            let z=negative_zero<.f32'builtin>(); let w=negative_zero<.f64'builtin>();
            a'bitvec == 1065353217 and b'bitvec == 4607182419068452864
            and z'bitvec == 2147483648 and w'bitvec == 9223372036854775808
        }";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100000).unwrap(),
        Value::Bool(true)
    );
    let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn generic_float_literals_require_a_proven_float_domain() {
    for source in [
        "fn zero()->T where T {0.0}",
        "fn zero()->T where T, requires T == .f32'builtin or T == bool {0.0}",
        "pure comptime fn Identity<T>->.Type'builtin {T} fn zero()->Identity<T> where T {0.0}",
    ] {
        assert!(
            compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn generic_float_default_forms_after_specialization() {
    let source="fn choose(value:T,.fallback:T=0.0)->T where T, requires T == .f32'builtin or T == .f64'builtin {fallback}
        fn run()->bool {choose<.f32'builtin>(1.0)==0.0 and choose<.f64'builtin>(1.0)==0.0}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100000).unwrap(),
        Value::Bool(true)
    );
    let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
    luna::host::Toolchain::default()
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
}

#[test]
fn iir_rejects_pending_float_even_with_concrete_annotation() {
    let package = compile("fn run()->.f32'builtin {0.0}");
    let mut graph = luna::instantiation::instantiate(&package, "run", 100000).unwrap();
    graph.functions[0].function.body.kind =
        nir::program::ExprKind::Literal(nir::program::Literal::PendingFloat {
            spelling: "0.0".into(),
            negative: false,
        });
    assert!(
        graph
            .validate(&mut 100000)
            .unwrap_err()
            .contains("unformed float")
    );
}

#[test]
fn generic_float_overflow_is_checked_for_each_instance_independently() {
    let package = compile(
        "fn huge()->T where T, requires T == .f32'builtin or T == .f64'builtin {1.0e40}
        fn small()->.f32'builtin {huge<.f32'builtin>()}
        fn large()->.f64'builtin {huge<.f64'builtin>()}",
    );
    let error = luna::instantiation::instantiate(&package, "small", 100000).unwrap_err();
    assert!(
        error.contains("float-formation-error")
            && error.contains("finite float literal exceeds")
            && error.contains("f32"),
        "{error}"
    );
    assert_eq!(
        luna::execution::interpret(&package, "large", 100000).unwrap(),
        Value::Float64(1e40f64.to_bits())
    );
    assert!(luna::instantiation::instantiate(&package, "large", 1).is_err());
    assert!(luna::instantiation::instantiate(&package, "large", 100000).is_ok());
}

#[test]
fn generic_float_formation_is_available_during_comptime_invocation() {
    let package=compile("comptime fn midpoint()->T where T, requires T == .f32'builtin or T == .f64'builtin {1.0000000596046447753906250000000001}
        fn run()->bool {const value=midpoint<.f32'builtin>();value'bitvec == 1065353217}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_float_union_preserves_exact_comptime_values() {
    let package=compile("typealias Float=.Float'builtin;
        comptime fn decimal()->T where T, requires T == Float or T == .f32'builtin or T == .f64'builtin {1.0000000596046447753906250000000001}
        comptime fn zero()->T where T, requires T == Float or T == .f32'builtin or T == .f64'builtin { -0.0 }
        comptime fn exact()->bool {decimal<Float>() == 1.0000000596046447753906250000000001}
        comptime fn negative()->Float {zero<Float>()}
        fn run()->bool {const valid=exact();const rounded=decimal<.f32'builtin>(); valid and rounded'bitvec == 1065353217}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100000).unwrap(),
        Value::Bool(true)
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let value = vm.invoke(id(&package, "negative"), vec![]).unwrap();
    let Value::Float(value) = value else {
        panic!("not exact Float")
    };
    assert!(value.is_zero() && value.is_negative());
    let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
    luna::host::Toolchain::default()
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
}

#[test]
fn exact_generic_float_does_not_become_runtime_storage() {
    let source = "fn value()->T where T, requires T == .Float'builtin or T == .f32'builtin {0.0}
        fn run()->.Float'builtin {value<.Float'builtin>()}";
    let error = match compile_with_protocols(source, &Default::default()) {
        Ok(package) => {
            assert!(luna::host::llvm_ir(&package, "run", 100000).is_err());
            luna::instantiation::instantiate(&package, "run", 100000).unwrap_err()
        }
        Err(error) => error,
    };
    assert!(
        error.contains("comptime-only value cannot inhabit runtime storage"),
        "{error}"
    );
}

#[test]
fn generic_float_formation_survives_quote_closure_and_default_callback_boundaries() {
    for body in [
        "quote fn produce()->T where T, requires T == .f32'builtin or T == .f64'builtin { -0.0 }
         fn calculate()->T where T, requires T == .f32'builtin or T == .f64'builtin {produce<T>()}",
        "fn make()->fn()->T where T, requires T == .f32'builtin or T == .f64'builtin {|| {let value:T= -0.0;value}}
         fn calculate()->T where T, requires T == .f32'builtin or T == .f64'builtin {let action=make<T>();action()}",
        "fn calculate(.action:fn()->T=|| { -0.0 })->T where T, requires T == .f32'builtin or T == .f64'builtin {action()}",
        "fn make()->%{value:T} fn()->T where T, requires T == .f32'builtin or T == .f64'builtin {let value:T= -0.0;||value}
         fn calculate()->T where T, requires T == .f32'builtin or T == .f64'builtin {let action=make<T>();action()}",
    ] {
        let source=format!("{body}
            fn run()->bool {{let a=calculate<.f32'builtin>();let b=calculate<.f64'builtin>();a'bitvec == 2147483648 and b'bitvec == 9223372036854775808}}");
        let package=compile_with_protocols(&source,&Default::default()).unwrap_or_else(|e|panic!("{source}: {e}"));
        assert_eq!(luna::execution::interpret(&package,"run",100000).unwrap_or_else(|e|panic!("{source}: {e}")),Value::Bool(true));
        let ir=luna::host::llvm_ir(&package,"run",100000).unwrap();
        for optimization in [luna::host::OptimizationLevel::O0,luna::host::OptimizationLevel::O3] {
            luna::host::Toolchain {optimization,..Default::default()}.execute_test(&ir,std::time::Duration::from_secs(5)).unwrap();
        }
    }
}
