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
