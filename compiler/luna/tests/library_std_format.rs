//! Format analysis through the public std interface and the real sysroot.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

#[test]
fn format_plan_preserves_utf8_ranges_escapes_and_argument_order() {
    assert_eq!(
        support::run(
            r#"
            use std.fmt.{FormatPlan, FormatPiece};
            comptime fn inspect() -> bool {
                let plan = FormatPlan.parse("界{}{{}}\0{}") match {
                    value! => value,
                    error _ => return false,
                };
                if plan.argument_count != 2 or plan.pieces.length() != 6 { return false; }
                (plan.pieces[0] matches FormatPiece.text(0, 3)) and
                (plan.pieces[1] matches FormatPiece.argument(0)) and
                (plan.pieces[2] matches FormatPiece.text(5, 6)) and
                (plan.pieces[3] matches FormatPiece.text(7, 8)) and
                (plan.pieces[4] matches FormatPiece.text(9, 10)) and
                (plan.pieces[5] matches FormatPiece.argument(1))
            }
            fn run() -> bool { inspect() }
            "#,
        ),
        Value::Bool(true)
    );
}

#[test]
fn format_plan_handles_empty_plain_and_adjacent_fields() {
    assert_eq!(
        support::run(
            r#"
            use std.fmt.{FormatPlan, FormatPiece};
            comptime fn inspect() -> bool {
                let empty = FormatPlan.parse("") match {
                    value! => value, error _ => return false,
                };
                let text = FormatPlan.parse("hello 世界") match {
                    value! => value, error _ => return false,
                };
                let fields = FormatPlan.parse("{}{}{}") match {
                    value! => value, error _ => return false,
                };
                let escaped = FormatPlan.parse("{{{}}}") match {
                    value! => value, error _ => return false,
                };
                empty.argument_count == 0 and empty.pieces.length() == 0 and
                text.argument_count == 0 and text.pieces.length() == 1 and
                (text.pieces[0] matches FormatPiece.text(0, 12)) and
                fields.argument_count == 3 and fields.pieces.length() == 3 and
                (fields.pieces[2] matches FormatPiece.argument(2)) and
                escaped.argument_count == 1 and escaped.pieces.length() == 3
            }
            fn run() -> bool { inspect() }
            "#,
        ),
        Value::Bool(true)
    );
}

#[test]
fn format_plan_rejects_malformed_fields_at_utf8_byte_offsets() {
    assert_eq!(
        support::run(
            r#"
            use std.fmt.{FormatPlan, FormatError};
            comptime fn inspect() -> bool {
                (FormatPlan.parse("界{") matches error FormatError.unclosed_field(3)) and
                (FormatPlan.parse("}") matches error FormatError.unmatched_close(0)) and
                (FormatPlan.parse("{{}") matches error FormatError.unmatched_close(2)) and
                (FormatPlan.parse("{0}") matches error FormatError.unsupported_field(0)) and
                (FormatPlan.parse("{:x}") matches error FormatError.unsupported_field(0)) and
                (FormatPlan.parse("{name}") matches error FormatError.unsupported_field(0)) and
                (FormatPlan.parse("{}}") matches error FormatError.unmatched_close(2))
            }
            fn run() -> bool { inspect() }
            "#,
        ),
        Value::Bool(true)
    );
}

#[test]
fn format_plan_remains_a_comptime_operation() {
    let error = support::compile(
        "fn invalid(format: str) { const valid = do { let plan = std.fmt.FormatPlan.parse(format); true }; }",
    )
    .err()
    .expect("a runtime string cannot supply the comptime format scanner");
    assert!(error.contains("comptime"), "{error}");
}

#[test]
fn format_domains_preserve_the_field_count_and_each_display_requirement() {
    let source = r#"
        comptime fn Fields() -> meta.List<Type> {
            std.fmt.FormatPlan.parse("{{}}{}:{}") match {
                plan! => plan.argument_domains(),
                error _ => [],
            }
        }
        const fields = Fields();
        fn pairs(first: defer fields[0], second: defer fields[1]) -> bool { true }
        fn empty(...values: defer std.fmt.Parse("{{}}")) -> bool { true }
        fn run() -> bool {
            let byte: u8 = 42;
            empty() and pairs(true, byte) and pairs(byte, false)
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    let invalid = source.replace("pairs(byte, false)", "pairs(byte, Missing {})");
    let invalid = format!("struct Missing {{}} {invalid}");
    let error = support::compile(&invalid).unwrap_err();
    assert!(
        error.contains("refinement requirement is not established"),
        "{error}"
    );
    let error = support::compile(&source.replace("empty()", "empty(true)")).unwrap_err();
    assert!(
        error.contains("expected (normal-form")
            || error.contains("count")
            || error.contains("arity"),
        "{error}"
    );
}

#[test]
fn invalid_format_cannot_supply_a_variadic_domain() {
    let error =
        support::compile("typealias Invalid = std.fmt.Parse(\"{} }\"); fn run() {}").unwrap_err();
    assert!(error.contains("ProgramContractRejected"), "{error}");
}

#[test]
fn deferred_format_parameters_accept_complete_packs_and_check_each_field() {
    let source = r#"
        fn accept(comptime format: str, ...varargs: defer std.fmt.Parse(format)) -> bool { true }
        fn run() -> bool {
            let byte: u8 = 42;
            accept("{}", byte) and accept("{}:{}", true, byte)
                and accept("{}:{}", .varargs = (byte, true))
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    for (call, diagnostic) in [
        (r#"std.fmt.println("{}:{}", true)"#, "shape mismatch"),
        (r#"std.fmt.println("{}", byte, false)"#, "shape mismatch"),
        (
            r#"std.fmt.println("{}", Unknown {})"#,
            "refinement requirement is not established",
        ),
        (
            r#"std.fmt.println("unclosed {")"#,
            "ProgramContractRejected",
        ),
    ] {
        let invalid = format!(
            "struct Unknown {{}} fn run() -> !std.io.Error void {{ let byte:u8 = 42; {call} }}"
        );
        let error = support::compile(&invalid).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn println_requires_the_format_before_it_can_admit_runtime_fields() {
    let error = support::compile(
        r#"
        use std.prelude.*;
        fn show(format: str, value: u32) -> !std.io.Error void {
            println(format, value)
        }
        "#,
    )
    .unwrap_err();
    assert!(error.contains("comptime"), "{error}");
}

#[test]
fn tuple_iteration_requires_evidence_from_all_producer_returns() {
    let template = r#"
        pure comptime fn Shape(flag: bool) -> Type {
            if flag { return OTHER; };
            Tuple<bool, bool>
        }
        fn consume(comptime flag: bool, ...values: defer Shape(flag)) {
            inline for (_, value) in values'tuple_enumerate { };
        }
    "#;
    // The early return can deliver a scalar while the tail constructs a tuple.
    assert!(support::compile(&template.replace("OTHER", "bool")).is_err());
    support::compile(&template.replace("OTHER", "Tuple<u8>")).unwrap();
}

#[test]
fn a_tuple_head_guarantee_does_not_supply_a_field_trait() {
    let source = r#"
        trait Mark { fn mark(self) -> bool; }
        impl Mark for bool { fn mark(self) -> bool { self } }
        pure comptime fn Shape(flag: bool) -> Type { Tuple<Any> }
        fn consume(comptime flag: bool, ...values: defer Shape(flag)) {
            inline for (_, value) in values'tuple_enumerate { value.mark(); };
        }
    "#;
    assert!(support::compile(source).is_err());
}
