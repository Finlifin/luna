//! Error-result program facts must not depend on effect continuation analysis.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use support::{compile, compile_with_protocols};

#[test]
fn arithmetic_before_error_return_is_checked() {
    compile(
        "typealias Byte = .u8'builtin; typealias Void = .void'builtin;
        fn observe(item:bool, calls:*Byte)->!bool Void {
            assumes calls.* < 3;
            calls.* += 1;
            if not item {return error true;}
        }",
    );
    let error = compile_with_protocols(
        "typealias Byte = .u8'builtin; typealias Void = .void'builtin;
        fn observe(calls:*Byte)->!bool Void {
            calls.* += 1; error true
        }",
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("range"), "{error}");
}

#[test]
fn known_error_and_success_payloads_keep_scalar_ranges() {
    for constructor in ["17", "error 17"] {
        compile(&format!(
            "typealias Byte = .u8'builtin;
            fn run()->Byte {{
                let result:!Byte Byte = {constructor};
                result match {{value! => value + 1, error failure => failure + 1}}
            }}"
        ));
    }
    for constructor in ["255", "error 255"] {
        let error = compile_with_protocols(
            &format!(
                "typealias Byte = .u8'builtin;
            fn run()->Byte {{
                let result:!Byte Byte = {constructor};
                result match {{value! => value + 1, error failure => failure + 1}}
            }}"
            ),
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.contains("range"), "{error}");
    }
}

#[test]
fn branch_guards_check_each_unknown_payload_independently() {
    compile(
        "typealias Byte = .u8'builtin;
        fn run(result:!Byte Byte)->Byte {
            result match {
                value! => {assumes value < 255; value + 1},
                error failure => {assumes failure < 255; failure + 1},
            }
        }",
    );
    let error = compile_with_protocols(
        "typealias Byte = .u8'builtin;
        fn run(result:!Byte Byte)->Byte {
            result match {
                value! => {assumes value < 255; value + 1},
                error failure => failure + 1,
            }
        }",
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("range"), "{error}");
}

#[test]
fn joining_results_keeps_common_ranges_without_reusing_another_branch() {
    compile(
        "typealias Byte = .u8'builtin;
        fn run(flag:bool)->Byte {
            let result:!Byte Byte = if flag {17} else {23};
            result match {value! => value + 1, error failure => failure + 1}
        }",
    );
    let error = compile_with_protocols(
        "typealias Byte = .u8'builtin;
        fn run(flag:bool)->Byte {
            let result:!Byte Byte = if flag {17} else {error 255};
            result match {value! => value + 1, error failure => failure + 1}
        }",
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("range"), "{error}");
}

#[test]
fn widening_remaps_known_error_tags_without_retyping_payloads() {
    for (payload, accepted) in [(17, true), (255, false)] {
        let source = format!(
            "typealias Byte = .u8'builtin;
            enum Low {{value(Byte)}} enum High {{value(Byte)}}
            fn run()->Byte {{
                let narrow:!Low bool = error Low.value({payload});
                let wide:![High,Low] bool = narrow;
                wide match {{
                    _! => 0,
                    error High.value(value) => value + 1,
                    error Low.value(value) => value + 1,
                }}
            }}"
        );
        let result = compile_with_protocols(&source, &Default::default());
        if accepted {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("range"), "{error}");
        }
    }
}

#[test]
fn copying_result_facts_does_not_make_later_reassignment_safe() {
    compile(
        "typealias Byte = .u8'builtin;
        fn run()->Byte {
            var original:!Byte Byte = error 17;
            let saved = original;
            original = error 255;
            saved match {value! => value + 1, error failure => failure + 1}
        }",
    );
    let error = compile_with_protocols(
        "typealias Byte = .u8'builtin;
        fn run()->Byte {
            var original:!Byte Byte = error 17;
            let saved = original;
            original = error 255;
            original match {value! => value + 1, error failure => failure + 1}
        }",
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("range"), "{error}");
}

#[test]
fn unknown_row_transport_preserves_payload_bounds_without_reusing_tag_numbers() {
    for guard in ["value < 255", "value <= 255"] {
        let source = format!(
            "typealias Byte = .u8'builtin;
            enum Low {{value(Byte)}} enum High {{value(Byte)}}
            fn run(input:![Low,High] bool)->Byte {{
                assumes input match {{error Low.value(value) => {guard}, _ => false}};
                let output:![High,Low] bool = input;
                output match {{
                    _! => 0,
                    error Low.value(value) => value + 1,
                    error High.value(_) => 0,
                }}
            }}"
        );
        let result = compile_with_protocols(&source, &Default::default());
        if guard == "value < 255" {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("range"), "{error}");
        }
    }
}

#[test]
fn nested_product_transport_preserves_payload_and_sibling_ranges() {
    for payload in [17, 255] {
        let source = format!(
            "typealias Byte = .u8'builtin;
            enum Low {{value(Byte)}} enum High {{value(Byte)}}
            fn run()->Byte {{
                let result:![Low,High] bool = error Low.value({payload});
                let byte:Byte = 3;
                let original = ((result, byte), byte);
                let output:Tuple<Tuple<![High,Low] bool,Byte>,Byte> = original;
                let ((result,first),second) = output;
                let observed:Byte = result match {{
                    _! => 0,
                    error Low.value(value) => value + 1,
                    error High.value(_) => 0,
                }};
                first + second
            }}"
        );
        let result = support::compile_with_tuple_protocols(&source, &Default::default());
        if payload == 17 {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("range"), "{error}");
        }
    }
}

#[test]
fn product_transport_retires_stale_source_observations() {
    let source = "typealias Byte = .u8'builtin;
        enum Low {value(Byte)} enum High {value(Byte)}
        fn run()->Byte {
            let good:![Low,High] bool = error Low.value(17);
            let bad:![Low,High] bool = error Low.value(255);
            var original = (good,);
            let output:Tuple<![High,Low] bool> = original;
            original = (bad,);
            let (result,) = output;
            result match {_! => 0,error Low.value(value)=>value + 1,error High.value(_)=>0}
        }";
    support::compile_with_tuple(source);
    let source = source.replace("let (result,) = output;", "let (result,) = original;");
    let error = support::compile_with_tuple_protocols(&source, &Default::default()).unwrap_err();
    assert!(error.contains("range"), "{error}");
}

#[test]
fn error_predicate_admission_still_rejects_calls_in_payload_guards() {
    for predicate in [
        "input match {error Low.value(value) => inspect_byte(value), _ => false}",
        "input matches (error Low.value(value) if inspect_byte(value))",
    ] {
        let error = compile_with_protocols(
            &format!(
                "typealias Byte = .u8'builtin;
                enum Low {{value(Byte)}}
                fn inspect_byte(value:Byte)->bool {{true}}
                fn run(input:!Low bool) {{assumes {predicate};}}"
            ),
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.contains("effect-free program predicate"), "{error}");
    }
}

#[test]
fn anonymous_record_transport_preserves_error_payloads() {
    for (payload, relation) in [(17, "<"), (255, "<=")] {
        let source = format!(
            "typealias Byte = .u8'builtin;
            fn record(...fields:T)->T where T {{fields}}
            enum Low {{value(Byte)}} enum High {{value(Byte)}}
            fn expected()->![High,Low] bool {{true}}
            fn run()->Byte {{
                let result:![Low,High] bool = error Low.value({payload});
                let byte:Byte = 3;
                let input = record {{result:result,byte:byte}};
                assumes input.result match {{error Low.value(value) => value {relation} 255, _ => false}};
                assumes input.byte < 10;
                let output:(record {{result:expected(),byte:byte}})'type = input;
                let observed:Byte = output.result match {{
                    _! => 0,
                    error Low.value(value) => value + 1,
                    error High.value(_) => 0,
                }};
                output.byte + 1
            }}"
        );
        let result = compile_with_protocols(&source, &Default::default());
        if payload == 17 {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("range"), "{error}");
        }
    }
}

#[test]
fn nested_error_pattern_guards_export_only_checked_payload_conditions() {
    for relation in ["<", "<="] {
        let source = format!(
            "typealias Byte = .u8'builtin;
            enum Low {{value(Byte)}}
            fn run(input:!Low bool)->Byte {{
                assumes input matches (error Low.value(value) if value {relation} 255);
                input match {{_! => 0, error Low.value(value) => value + 1}}
            }}"
        );
        let result = compile_with_protocols(&source, &Default::default());
        if relation == "<" {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("range"), "{error}");
        }
    }
}

#[test]
fn nested_guard_conditions_do_not_leak_from_failed_or_stale_candidates() {
    for assumption in [
        "assumes not (input matches (error Low.value(value) if value < 255));",
        "assumes input matches (error Low.value(value) if value < 255); input = error Low.value(255);",
        "assumes input matches ((error Low.value(value) if value < 255) or error Low.value(value));",
    ] {
        let source = format!(
            "typealias Byte = .u8'builtin;
            enum Low {{value(Byte)}}
            fn run(original:!Low bool)->Byte {{
                var input = original;
                {assumption}
                input match {{_! => 0, error Low.value(value) => value + 1}}
            }}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(error.contains("range"), "{error}");
    }
}

#[test]
fn structural_nested_guards_preserve_optional_and_tuple_payload_bounds() {
    for (ty, pattern, output) in [
        (
            "?Byte",
            "(value if value < 255)?",
            "input match {value? => value + 1, null => 0}",
        ),
        (
            "Tuple<Byte>",
            "(value if value < 255,)",
            "{let (value,) = input; value + 1}",
        ),
    ] {
        let source = format!(
            "typealias Byte = .u8'builtin;
            fn run(input:{ty})->Byte {{
                assumes input matches {pattern};
                {output}
            }}"
        );
        support::compile_with_tuple_protocols(&source, &Default::default()).unwrap();
        let error = support::compile_with_tuple_protocols(
            &source.replace("< 255", "<= 255"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.contains("range"), "{error}");
    }
}

#[test]
fn nested_guard_with_prior_field_binding_keeps_ordinary_match_valid() {
    support::compile_with_tuple(
        "typealias Byte = .u8'builtin;
        fn run(input:Tuple<Byte,Byte>)->bool {
            let result = input matches (first, (second if first == second));
            result
        }",
    );
}
