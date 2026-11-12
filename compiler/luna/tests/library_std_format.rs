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
