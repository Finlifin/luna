//! End-to-end tests for bodyless `template quote fn` match-arm assembly,
//! `use expr` arm imports, and contextual `implicit quote` resource linearity.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::Value;

#[test]
fn quote_fn_arms_assemble_into_match_and_verify_exhaustiveness() {
    let source = "
        enum Operation {
            literal(u32),
            add(u32, u32),
            multiply(u32, u32),
        }

        ^attrs.quote_fn_template.for_match_arms
        ^attrs.quote_fn_template.allow_non_exhaustive_instance
        template quote fn evaluate_rule(operation: Operation) -> u32;

        quote fn add_rule(case .add(left, right)) use evaluate_rule {
            if left <= 1000 and right <= 1000 {
                left + right
            } else {
                0
            }
        }

        quote fn multiply_rule(case .multiply(left, right)) use evaluate_rule {
            if left <= 1000 and right <= 1000 {
                left * right
            } else {
                0
            }
        }

        fn evaluate(operation: Operation) -> u32 {
            operation match {
                use evaluate_rule,
                .literal(value) => value,
            }
        }

        fn run() -> u32 {
            let a = evaluate(Operation.multiply(6, 7));
            let b = evaluate(Operation.add(3, 4));
            let c = evaluate(Operation.literal(5));
            if a <= 1000 and b <= 1000 and c <= 1000 {
                a + b + c
            } else {
                0
            }
        }
    ";
    assert_eq!(sysroot::run(source), Value::UnsignedInteger(42 + 7 + 5));
}

#[test]
fn quote_fn_arms_isolate_bindings_support_logical_return_and_contextual_inputs() {
    let source = "
        enum Expr {
            lit(u32),
            plus(u32, u32),
            scaled(u32),
        }

        ^attrs.quote_fn_template.for_match_arms
        ^attrs.quote_fn_template.allow_non_exhaustive_instance
        template quote fn eval_with_ctx(expr: Expr, implicit quote factor: u32) -> u32;

        quote fn eval_plus(case .plus(x, y)) use eval_with_ctx {
            if x == 0 and y <= 1000 and factor <= 1000 {
                return y + factor;
            }
            if x <= 1000 and y <= 1000 {
                x + y
            } else {
                0
            }
        }

        quote fn eval_scaled(case .scaled(x), implicit quote factor: u32) use eval_with_ctx {
            if x <= 1000 and factor <= 1000 {
                x * factor
            } else {
                0
            }
        }

        fn compute(expr: Expr, factor: u32) -> u32 {
            let outside = expr match {
                use eval_with_ctx,
                .lit(x) => x,
            };
            if outside <= 1000 { outside + 1 } else { outside }
        }

        fn run() -> u32 {
            let a = compute(Expr.plus(0, 5), 10);
            let b = compute(Expr.scaled(3), 4);
            let c = compute(Expr.lit(2), 9);
            if a <= 1000 and b <= 1000 and c <= 1000 {
                a + b + c
            } else {
                0
            }
        }
    ";
    assert_eq!(sysroot::run(source), Value::UnsignedInteger(16 + 13 + 3));
}

#[test]
fn generic_quote_fn_arms_and_exhaustive_single_arm_templates_work() {
    let source = "
        enum Boxed where T {
            empty,
            one(T),
        }

        ^attrs.quote_fn_template.for_match_arms
        ^attrs.quote_fn_template.allow_non_exhaustive_instance
        template quote fn unwrap_rule(box: Boxed<T>, implicit quote fallback: T) -> T where T;

        quote fn unwrap_empty(case .empty) use unwrap_rule {
            fallback
        }

        ^attrs.quote_fn_template.for_match_arms
        template quote fn full_rule(flag: bool) -> u32;

        quote fn handle_all(case x) use full_rule {
            if x { 10 } else { 20 }
        }

        fn pick(b: Boxed<u32>, fallback: u32, flag: bool) -> u32 {
            let a = b match {
                use unwrap_rule,
                .one(v) => v,
            };
            let c = flag match {
                use full_rule,
            };
            if a <= 1000 and c <= 1000 { a + c } else { 0 }
        }

        fn run() -> u32 {
            let x = pick(Boxed<u32>.empty, 5, true);
            let y = pick(Boxed<u32>.one(7), 5, false);
            if x <= 1000 and y <= 1000 { x + y } else { 0 }
        }
    ";
    assert_eq!(sysroot::run(source), Value::UnsignedInteger(15 + 27));
}

#[test]
fn quote_fn_arm_import_works_inside_structural_call_inference() {
    let source = "
        enum Choice {
            a(u32),
            b(u32),
        }

        ^attrs.quote_fn_template.for_match_arms
        ^attrs.quote_fn_template.allow_non_exhaustive_instance
        template quote fn choice_rule(c: Choice) -> u32;

        quote fn choice_a(case .a(v)) use choice_rule { v + 1 }

        fn id(x: T) -> T where T { x }

        fn run() -> u32 {
            id(Choice.a(9) match {
                use choice_rule,
                .b(v) => v,
            })
        }
    ";
    assert_eq!(sysroot::run(source), Value::UnsignedInteger(10));
}

#[test]
fn contextual_quote_fn_preserves_once_linearity_and_identity() {
    let valid_once = "
        struct Handle { tag: u32 }

        fn consume(h: Handle) -> u32 { h.tag }

        quote fn ignore_ctx(x: u32, implicit quote h: Handle) -> u32 { x }
        quote fn take_ctx(implicit quote h: Handle) -> u32 { consume(h) }

        fn run() -> u32 {
            let h = Handle { tag: 11 };
            let a = ignore_ctx(5);
            let b = consume(h);
            let h = Handle { tag: 22 };
            let c = take_ctx();
            if a <= 1000 and b <= 1000 and c <= 1000 {
                a + b + c
            } else {
                0
            }
        }
    ";
    assert_eq!(
        sysroot::run(valid_once),
        Value::UnsignedInteger(5 + 11 + 22)
    );

    let double_use_after_quote_move = "
        struct Handle { tag: u32 }
        fn consume(h: Handle) -> u32 { h.tag }
        quote fn take_ctx(implicit quote h: Handle) -> u32 { consume(h) }

        fn bad() -> u32 {
            let h = Handle { tag: 22 };
            let a = take_ctx();
            a + consume(h)
        }
    ";
    assert!(
        sysroot::compile(double_use_after_quote_move).is_err(),
        "moving Once contextual input inside quote fn must prevent later move in caller"
    );
}

#[test]
fn quote_fn_arms_reject_invalid_templates_conflicts_and_non_exhaustive_matches() {
    let base_enum = "
        enum Operation {
            literal(u32),
            add(u32, u32),
            multiply(u32, u32),
        }
    ";

    for (label, source) in [
        (
            "non-arm template in use",
            format!(
                "{base_enum}
                 fn helper(x: Operation) -> u32 {{ 0 }}
                 fn bad(op: Operation) -> u32 {{ op match {{ use helper, _ => 0 }} }}"
            ),
        ),
        (
            "bodyless template quote fn without for_match_arms in use",
            format!(
                "{base_enum}
                 template quote fn plain(op: Operation) -> u32;
                 fn bad(op: Operation) -> u32 {{ op match {{ use plain, _ => 0 }} }}"
            ),
        ),
        (
            "conflicting overlapping arms on same template",
            format!(
                "{base_enum}
                 ^attrs.quote_fn_template.for_match_arms
                 ^attrs.quote_fn_template.allow_non_exhaustive_instance
                 template quote fn eval(op: Operation) -> u32;
                 quote fn a1(case .add(x, y)) use eval {{ x + y }}
                 quote fn a2(case .add(l, r)) use eval {{ l + r }}"
            ),
        ),
        (
            "non-exhaustive single arm without allow_non_exhaustive_instance",
            format!(
                "{base_enum}
                 ^attrs.quote_fn_template.for_match_arms
                 template quote fn eval(op: Operation) -> u32;
                 quote fn a1(case .add(x, y)) use eval {{ x + y }}"
            ),
        ),
        (
            "remaining unhandled cases after use",
            format!(
                "{base_enum}
                 ^attrs.quote_fn_template.for_match_arms
                 ^attrs.quote_fn_template.allow_non_exhaustive_instance
                 template quote fn eval(op: Operation) -> u32;
                 quote fn a1(case .add(x, y)) use eval {{ x + y }}
                 fn bad(op: Operation) -> u32 {{ op match {{ use eval }} }}"
            ),
        ),
        (
            "missing case on for_match_arms instance",
            format!(
                "{base_enum}
                 ^attrs.quote_fn_template.for_match_arms
                 ^attrs.quote_fn_template.allow_non_exhaustive_instance
                 template quote fn eval(op: Operation) -> u32;
                 quote fn a1(op: Operation) use eval {{ 0 }}"
            ),
        ),
        (
            "case on non-for_match_arms template",
            format!(
                "{base_enum}
                 template quote fn eval(op: Operation) -> u32;
                 quote fn a1(case .add(x, y)) use eval {{ x + y }}"
            ),
        ),
        (
            "direct call to for_match_arms template",
            format!(
                "{base_enum}
                 ^attrs.quote_fn_template.for_match_arms
                 ^attrs.quote_fn_template.allow_non_exhaustive_instance
                 template quote fn eval(op: Operation) -> u32;
                 fn bad(op: Operation) -> u32 {{ eval(op) }}"
            ),
        ),
        (
            "direct call to quote fn arm instance",
            format!(
                "{base_enum}
                 ^attrs.quote_fn_template.for_match_arms
                 ^attrs.quote_fn_template.allow_non_exhaustive_instance
                 template quote fn eval(op: Operation) -> u32;
                 quote fn a1(case .add(x, y)) use eval {{ x + y }}
                 fn bad(op: Operation) -> u32 {{ a1(op) }}"
            ),
        ),
    ] {
        assert!(
            sysroot::compile(&source).is_err(),
            "expected rejection for `{label}`:\n{source}"
        );
    }
}

#[test]
fn quote_fn_arms_support_pattern_guards_and_structural_early_returns() {
    let source = "
        enum Op {
            lit(u32),
            zero,
        }

        ^attrs.quote_fn_template.for_match_arms
        ^attrs.quote_fn_template.allow_non_exhaustive_instance
        template quote fn eval_guard(op: Op) -> u32;

        quote fn guard_pos(case .lit(n) if n > 0) use eval_guard {
            if n == 7 {
                return 700;
            }
            n
        }

        quote fn guard_zero(case .zero) use eval_guard {
            0
        }

        fn id(value: T) -> T where T {
            value
        }

        fn eval(op: Op) -> u32 {
            id(op match {
                use eval_guard,
                .lit(_) => 999,
            })
        }

        fn run() -> bool {
            eval(Op.lit(7)) == 700 and eval(Op.lit(3)) == 3 and eval(Op.lit(0)) == 999 and eval(Op.zero) == 0
        }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn quote_fn_arms_reject_comptime_only_contextual_locals() {
    let source = "
        typealias Type = .Type'builtin;

        enum Tag {
            one,
        }

        ^attrs.quote_fn_template.for_match_arms
        template quote fn read_ctx(t: Tag, implicit quote ctx: T) -> u32 where T;

        quote fn read_one(case .one) use read_ctx {
            0
        }

        fn bad(t: Tag) -> u32 {
            let ctx = Type;
            t match {
                use read_ctx,
            }
        }
    ";
    assert!(
        sysroot::compile(source).is_err(),
        "expected comptime-only contextual binding in match scope to be rejected"
    );
}
