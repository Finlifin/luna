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
            use std.fmt.{FormatPlan, FormatErr};
            comptime fn inspect() -> bool {
                (FormatPlan.parse("界{") matches error FormatErr.UnclosedField(3)) and
                (FormatPlan.parse("}") matches error FormatErr.UnmatchedClose(0)) and
                (FormatPlan.parse("{{}") matches error FormatErr.UnmatchedClose(2)) and
                (FormatPlan.parse("{0}") matches error FormatErr.UnsupportedField(0)) and
                (FormatPlan.parse("{:q}") matches error FormatErr.UnsupportedField(0)) and
                (FormatPlan.parse("{name}") matches error FormatErr.UnsupportedField(0)) and
                (FormatPlan.parse("{}}") matches error FormatErr.UnmatchedClose(2))
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
            "struct Unknown {{}} fn run() -> !std.io.IoErr void {{ let byte:u8 = 42; {call} }}"
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
        fn show(format: str, value: u32) -> !std.io.IoErr void {
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
        pure comptime fn Shape(flag: bool) -> R: Type
        where ensures Type.predicates.is_tuple(R)
        {
            if flag { return OTHER; };
            Tuple<bool, bool>
        }
        fn consume(comptime flag: bool, ...values: defer Shape(flag)) {
            inline for value in values'tuple_enumerate { };
        }
    "#;
    // The early return can deliver a scalar while the tail constructs a tuple.
    assert!(support::compile(&template.replace("OTHER", "bool")).is_err());
    support::compile(&template.replace("OTHER", "Tuple<u8>")).unwrap();
    // An unannotated producer must NOT authorize tuple_enumerate via body-peeking.
    let unannotated = r#"
        pure comptime fn Shape(flag: bool) -> Type {
            if flag { return Tuple<u8>; };
            Tuple<bool, bool>
        }
        fn consume(comptime flag: bool, ...values: defer Shape(flag)) {
            inline for value in values'tuple_enumerate { };
        }
    "#;
    assert!(support::compile(unannotated).is_err());
}

#[test]
fn a_tuple_head_guarantee_does_not_supply_a_field_trait() {
    let source = r#"
        trait Mark { fn mark(self) -> bool; }
        impl Mark for bool { fn mark(self) -> bool { self } }
        pure comptime fn Shape(flag: bool) -> R: Type
        where ensures Type.predicates.is_tuple(R)
        { Tuple<Any> }
        fn consume(comptime flag: bool, ...values: defer Shape(flag)) {
            inline for value in values'tuple_enumerate { value.mark(); };
        }
    "#;
    assert!(support::compile(source).is_err());
}

#[test]
fn type_predicates_is_tuple_works_across_propositions_and_comptime() {
    let source = r#"
        comptime fn executable_checks() -> bool {
            Type.predicates.is_tuple(Tuple<u8, bool>)
                and Type.predicates.is_tuple(Tuple<>)
                and not Type.predicates.is_tuple(bool)
        }
        fn requires_tuple(value: T) -> bool
        where T, requires Type.predicates.is_tuple(T)
        {
            true
        }
        fn iterate_assumed(...args: T) -> bool where T {
            assumes Type.predicates.is_tuple(args'type);
            var seen: u8 = 0;
            inline for _ in args'tuple_enumerate {
                assumes seen < 255;
                seen = seen + 1;
            }
            seen == 2
        }
        typealias TupleOnly = refines { Type.predicates.is_tuple(Itself) } Any;
        fn refined_tuple(value: TupleOnly) -> bool { true }
        fn run() -> bool {
            let one: u8 = 1;
            let pair: Tuple<u8, bool> = (1, true);
            executable_checks()
                and requires_tuple(pair)
                and iterate_assumed(one, true)
                and refined_tuple(pair)
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));

    assert!(
        support::compile(
            r#"
            fn requires_tuple(value: T) -> bool where T, requires Type.predicates.is_tuple(T) { true }
            fn run() -> bool { requires_tuple(true) }
            "#,
        )
        .is_err()
    );
    assert!(
        support::compile(
            r#"
            typealias TupleOnly = refines { Type.predicates.is_tuple(Itself) } Any;
            fn refined_tuple(value: TupleOnly) -> bool { true }
            fn run() -> bool { let x: u8 = 1; refined_tuple(x) }
            "#,
        )
        .is_err()
    );
}

#[test]
fn type_producer_ensures_is_tuple_contract_enforced_and_authorizes_iteration() {
    let source = r#"
        pure comptime fn PairOf(T: Type) -> R: Type
        where ensures Type.predicates.is_tuple(R)
        {
            Tuple<T, T>
        }
        fn consume(comptime T: Type, ...args: defer PairOf(T)) -> bool {
            var count: u8 = 0;
            inline for _ in args'tuple_enumerate {
                assumes count < 255;
                count = count + 1;
            }
            count == 2
        }
        fn run() -> bool {
            let one: u8 = 1;
            let two: u8 = 2;
            consume(bool, true, false) and consume(u8, one, two)
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));

    // A Type-producing function declaring ensures is_tuple(R) must fail if it returns bool.
    assert!(
        support::compile(
            r#"
            pure comptime fn BadProducer(T: Type) -> R: Type
            where ensures Type.predicates.is_tuple(R)
            {
                bool
            }
            fn run() {}
            "#,
        )
        .is_err()
    );

    // Without ensures Type.predicates.is_tuple(R), 'tuple_enumerate must remain rejected.
    assert!(
        support::compile(
            r#"
            pure comptime fn UnannotatedPair(T: Type) -> Type { Tuple<T, T> }
            fn bad(comptime T: Type, ...args: defer UnannotatedPair(T)) {
                inline for _ in args'tuple_enumerate {}
            }
            "#,
        )
        .is_err()
    );
}

#[test]
fn std_fmt_write_fmt_println_and_eprintln_cover_zero_single_escaped_and_heterogeneous_args() {
    let ok = support::compile(
        r#"
        use std.prelude.*;
        use std.fmt.write_fmt;
        fn exercise() -> !std.io.IoErr void {
            var out = std.io.stdout();
            write_fmt(out.ref, "empty")!;
            write_fmt(out.ref, "{{escaped}}")!;
            write_fmt(out.ref, "{}", true)!;
            let byte: u8 = 7;
            let num: i32 = -12;
            write_fmt(out.ref, "{} {} {}", false, byte, num)!;
            println("")!;
            println("{{}}")!;
            println("{}", byte)!;
            println("{} {} {}", true, byte, num)!;
            eprintln("")!;
            eprintln("{{err}}")!;
            eprintln("{}", byte)!;
            eprintln("{} {} {}", false, byte, num)
        }
        fn run() -> bool { true }
        "#,
    );
    assert!(ok.is_ok(), "{ok:?}");

    for call in [
        r#"std.fmt.eprintln("{}:{}", true)"#,
        r#"std.fmt.eprintln("{}", Unknown {})"#,
        r#"std.fmt.write_fmt(w.ref, "{}:{}", true)"#,
        r#"std.fmt.write_fmt(w.ref, "{}", Unknown {})"#,
    ] {
        let invalid = format!(
            "use std.prelude.*; struct Unknown {{}} fn run() -> !std.io.IoErr void {{ var w = std.io.stdout(); {call} }}"
        );
        assert!(support::compile(&invalid).is_err(), "{call}");
    }
}
