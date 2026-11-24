//! End-to-end tests for the four-tier safety model (`safe` / `spec` / `verified` / `unsafe`)
//! and static verification gate (`requires`, `ensures`, `asserts`, `invariant`, `decreases`).
#[path = "support/sysroot_execution.rs"]
mod sysroot;

use comptime_interpretor::Value;

#[test]
fn verified_recursive_and_loop_contracts_compile_and_run() {
    let source = r#"
        verified fn sum_down(n: i64) -> result: i64
        where requires n >= 0, requires n <= 1000, decreases n, ensures result >= 0, ensures result <= n'old
        {
            asserts n >= 0;
            if n == 0 {
                0
            } else {
                let rest = sum_down(n - 1);
                1 + rest
            }
        }

        verified fn count_up(limit: i64) -> result: i64
        where requires limit >= 0, requires limit <= 100, ensures result >= 0, ensures result <= limit'old
        {
            var i: i64 = 0;
            while i < limit {
                invariant 0 <= i;
                invariant i <= limit;
                decreases limit - i;
                i = i + 1;
            }
            asserts i >= 0;
            i
        }

        fn run() -> i64 {
            sum_down(5) + count_up(4)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::SignedInteger(9));
}

#[test]
fn verified_fn_rejects_unproven_ensures_requires_asserts_and_decreases() {
    let bad_ensures = r#"
        verified fn bad(x: i64) -> result: i64
        where ensures result > 0
        {
            x
        }
    "#;
    let err = sysroot::compile(bad_ensures).unwrap_err();
    assert!(err.contains("postcondition"), "{err}");

    let bad_callee_req = r#"
        verified fn callee(x: i64) -> i64 where requires x > 0 { x }
        verified fn caller(x: i64) -> i64 { callee(x) }
    "#;
    let err = sysroot::compile(bad_callee_req).unwrap_err();
    assert!(err.contains("precondition"), "{err}");

    let bad_assert = r#"
        verified fn bad_assert(x: i64) -> i64 {
            asserts x > 0;
            x
        }
    "#;
    let err = sysroot::compile(bad_assert).unwrap_err();
    assert!(err.contains("assertion"), "{err}");

    let bad_decreases = r#"
        verified fn diverge(n: i64) -> i64
        where requires n >= 0, decreases n
        {
            if n == 0 { 0 } else { diverge(n) }
        }
    "#;
    let err = sysroot::compile(bad_decreases).unwrap_err();
    assert!(
        err.contains("decreases") || err.contains("termination"),
        "{err}"
    );

    let negative_decreases = r#"
        verified fn neg(n: i64) -> i64
        where decreases n
        {
            0
        }
    "#;
    let err = sysroot::compile(negative_decreases).unwrap_err();
    assert!(
        err.contains("decreases") || err.contains("termination"),
        "{err}"
    );

    let bad_invariant = r#"
        verified fn bad_loop(n: i64) -> i64
        where requires n >= 0, requires n <= 10
        {
            var i: i64 = 0;
            while i < n {
                invariant i >= 1;
                i = i + 1;
            }
            i
        }
    "#;
    let err = sysroot::compile(bad_invariant).unwrap_err();
    assert!(err.contains("invariant"), "{err}");
}

#[test]
fn spec_fn_usable_in_contracts_and_rejected_in_runtime_and_side_effects() {
    let valid = r#"
        spec fn in_unit_range(x: i64) -> bool {
            x >= 0 and x <= 10
        }

        verified fn step(x: i64) -> result: i64
        where requires in_unit_range(x), ensures result >= 1
        {
            asserts in_unit_range(x);
            x + 1
        }

        fn run() -> i64 {
            step(3)
        }
    "#;
    assert_eq!(sysroot::run(valid), Value::SignedInteger(4));

    let runtime_call = r#"
        spec fn in_unit_range(x: i64) -> bool {
            x >= 0 and x <= 10
        }
        fn run() -> bool {
            in_unit_range(3)
        }
    "#;
    let err = sysroot::compile(runtime_call).unwrap_err();
    assert!(err.contains("spec function"), "{err}");

    let unsafe_in_spec = r#"
        unsafe fn dangerous(x: i64) -> i64 { x }
        spec fn bad_spec(x: i64) -> bool {
            unsafe { dangerous(x) > 0 }
        }
    "#;
    let err = sysroot::compile(unsafe_in_spec).unwrap_err();
    assert!(
        err.contains("unsafe") || err.contains("spec function"),
        "{err}"
    );

    let runtime_call_in_spec = r#"
        fn helper(x: i64) -> bool { x > 0 }
        spec fn bad_spec(x: i64) -> bool {
            helper(x)
        }
    "#;
    let err = sysroot::compile(runtime_call_in_spec).unwrap_err();
    assert!(err.contains("spec function"), "{err}");
}

#[test]
fn verified_fn_calling_unsafe_requires_unsafe_block_and_entry_proof() {
    let valid = r#"
        unsafe fn low_level(x: i64) -> result: i64
        where requires x > 0, requires x <= 100, ensures result > 0
        {
            x
        }

        verified fn wrapper(x: i64) -> result: i64
        where requires x > 0, requires x <= 100, ensures result > 0
        {
            unsafe { low_level(x) }
        }

        fn run() -> i64 {
            wrapper(7)
        }
    "#;
    assert_eq!(sysroot::run(valid), Value::SignedInteger(7));

    let missing_unsafe = r#"
        unsafe fn low_level(x: i64) -> i64
        where requires x > 0
        {
            x
        }

        verified fn wrapper(x: i64) -> i64
        where requires x > 0
        {
            low_level(x)
        }
    "#;
    let err = sysroot::compile(missing_unsafe).unwrap_err();
    assert!(err.contains("unsafe"), "{err}");

    let unproven_unsafe_precondition = r#"
        unsafe fn low_level(x: i64) -> i64
        where requires x > 0
        {
            x
        }

        verified fn wrapper(x: i64) -> i64 {
            unsafe { low_level(x) }
        }
    "#;
    let err = sysroot::compile(unproven_unsafe_precondition).unwrap_err();
    assert!(err.contains("precondition"), "{err}");
}

#[test]
fn invalid_safety_modifier_combinations_are_rejected() {
    for source in [
        "spec unsafe fn bad(x: i64) -> bool { x > 0 }",
        "spec verified fn bad(x: i64) -> bool { x > 0 }",
        "verified unsafe fn bad(x: i64) -> i64 { x }",
    ] {
        let err = sysroot::compile(source).unwrap_err();
        assert!(
            err.contains("spec") || err.contains("verified") || err.contains("unsafe"),
            "{source}: {err}"
        );
    }
}

#[test]
fn verified_fn_rejects_non_header_loop_decreases() {
    let source = r#"
        verified fn non_header(n: i64) -> i64
        where requires n >= 0
        {
            var i: i64 = 0;
            while i < n {
                let _guard: i64 = i;
                decreases n - i;
                i = i + 1;
            }
            i
        }
    "#;
    let err = sysroot::compile(source).unwrap_err();
    assert!(
        err.contains("decreases") || err.contains("termination") || err.contains("verified"),
        "{err}"
    );
}

#[test]
fn spec_fn_supports_reordered_named_arguments_in_contracts() {
    let valid = r#"
        spec fn in_range(.lo: i64 = 0, .hi: i64 = 100, .x: i64 = 0) -> bool {
            x >= lo and x <= hi
        }

        verified fn clamp_check(x: i64) -> result: i64
        where
            requires in_range(.hi = 20, .lo = 5, .x = x),
            ensures result == x
        {
            asserts in_range(.x = x, .hi = 20, .lo = 5);
            x
        }

        fn run() -> i64 {
            clamp_check(12)
        }
    "#;
    assert_eq!(sysroot::run(valid), Value::SignedInteger(12));

    let bad_call = r#"
        spec fn in_range(.lo: i64 = 0, .hi: i64 = 100, .x: i64 = 0) -> bool {
            x >= lo and x <= hi
        }

        verified fn guarded(x: i64) -> i64
        where requires in_range(.hi = 20, .lo = 5, .x = x)
        {
            x
        }

        verified fn bad_caller() -> i64 {
            guarded(21)
        }
    "#;
    let err = sysroot::compile(bad_call).unwrap_err();
    assert!(err.contains("precondition"), "{err}");
}

#[test]
fn spec_fn_rejects_loops_and_static_address() {
    let loop_in_spec = r#"
        spec fn bad_loop(n: i64) -> bool {
            var i: i64 = 0;
            while i < n {
                i = i + 1;
            }
            i >= 0
        }
    "#;
    let err = sysroot::compile(loop_in_spec).unwrap_err();
    assert!(
        err.contains("loop") || err.contains("spec function"),
        "{err}"
    );

    let static_in_spec = r#"
        static COUNTER: i64 = 0;
        spec fn bad_static() -> bool {
            COUNTER >= 0
        }
    "#;
    let err = sysroot::compile(static_in_spec).unwrap_err();
    assert!(
        err.contains("static") || err.contains("pointer") || err.contains("spec function"),
        "{err}"
    );
}
