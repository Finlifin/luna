//! End-to-end tests for `pattern_automata` list/slice exhaustiveness, missing witnesses,
//! runtime `Slice<T>` / `Vec<T>` rest slicing, `StableListPatternView`, and A1 purity boundaries.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use nir::DeclarationId;
use nir::match_plan::{ListObservationMode, MatchPlan};
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-pa-lists-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self, opt_level: &str) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "build",
                "main.fl",
                "--entry",
                "run",
                "--output",
                "program",
                "--opt-level",
                opt_level,
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .output()
            .unwrap()
    }

    fn run(&self) -> Output {
        ProcessCommand::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for NativeCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_balanced_sexpr(text: &str) {
    assert!(text.starts_with("(match-plans"), "{text}");
    let mut depth = 0isize;
    let mut in_str = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                assert!(depth >= 0, "unbalanced closing paren in: {text}");
            }
            _ => {}
        }
    }
    assert!(!in_str, "unterminated string in: {text}");
    assert_eq!(depth, 0, "unbalanced parens in: {text}");
}

fn plans_for_fn(plans: &nir::match_plan::MatchPlans, fn_id: DeclarationId) -> Vec<&MatchPlan> {
    plans
        .plans
        .iter()
        .filter(|(key, _)| key.function == Some(fn_id))
        .map(|(_, plan)| plan)
        .collect()
}

#[test]
fn comptime_meta_list_exhaustive_without_fallback_and_rest_slicing() {
    let source = r#"
        comptime fn sum_list(xs: meta.List<Integer>) -> Integer {
            xs match {
                [] => 0,
                [x] => x,
                [x, y, ...rest] => x + y + sum_list(rest),
            }
        }

        comptime fn run_comptime() -> bool {
            sum_list([]) == 0
                and sum_list([7]) == 7
                and sum_list([3, 4]) == 7
                and sum_list([10, 20, 30]) == 60
                and sum_list([1, 2, 3, 4, 5]) == 15
        }

        const COMPTIME_OK: bool = run_comptime();

        fn run() -> bool {
            COMPTIME_OK
        }
    "#;

    assert_eq!(
        support::invoke(source, "run_comptime").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::run(source), Value::Bool(true));

    let pkg = support::compile(source).unwrap();
    let fn_id = support::source_function(&pkg, "sum_list");
    let plans = plans_for_fn(&pkg.match_plans, fn_id);
    assert_eq!(plans.len(), 1, "expected 1 match plan for sum_list");
    let plan = plans[0];
    assert!(plan.is_exhaustive);
    assert!(plan.unreachable_arms.is_empty());
    let root_ops = plan.list_views.values().next().expect("list view ops");
    assert_eq!(root_ops.mode, ListObservationMode::Comptime);
    assert!(
        plan.dag.to_string().contains("(list-len "),
        "expected ListLen switch in DAG: {}",
        plan.dag
    );
    assert_balanced_sexpr(&pkg.match_plans.to_string());
}

#[test]
fn comptime_meta_list_cross_element_and_missing_witnesses() {
    let missing_single = support::compile(
        r#"
        comptime fn bad(xs: meta.List<i64>) -> i64 {
            xs match {
                [] => 0,
                [x, y, ...rest] => x + y,
            }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        missing_single.contains("non-exhaustive match")
            && missing_single.contains("missing pattern(s):")
            && missing_single.contains("[0]")
            && missing_single.contains("add an unguarded fallback pattern"),
        "{missing_single}"
    );

    let missing_empty = support::compile(
        r#"
        comptime fn bad(xs: meta.List<i64>) -> i64 {
            xs match {
                [x, ..._] => x,
            }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        missing_empty.contains("non-exhaustive match")
            && missing_empty.contains("missing pattern(s):")
            && missing_empty.contains("[]"),
        "{missing_empty}"
    );

    let pair_ok = r#"
        comptime fn check_pair(pair: Tuple<bool, meta.List<bool>>) -> i64 {
            pair match {
                (true, []) => 1,
                (true, [_, ..._]) => 2,
                (false, []) => 3,
                (false, [true, ..._]) => 4,
                (false, [false, ..._]) => 5,
            }
        }

        const OK: bool =
            check_pair((true, [])) == 1
                and check_pair((true, [false, true])) == 2
                and check_pair((false, [])) == 3
                and check_pair((false, [true, false])) == 4
                and check_pair((false, [false])) == 5;

        fn run() -> bool { OK }
    "#;
    assert_eq!(support::run(pair_ok), Value::Bool(true));

    let pair_missing = support::compile(
        r#"
        comptime fn bad_pair(pair: Tuple<bool, meta.List<bool>>) -> i64 {
            pair match {
                (true, []) => 1,
                (true, [_, ..._]) => 2,
                (false, [true, ..._]) => 4,
                (false, [false, ..._]) => 5,
            }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        pair_missing.contains("non-exhaustive match") && pair_missing.contains("(false, [])"),
        "{pair_missing}"
    );

    let opt_ok = r#"
        comptime fn check_opt(xs: ?meta.List<i64>) -> i64 {
            xs match {
                null => 0,
                []? => 1,
                [x, ..._]? => x,
            }
        }

        const OK: bool =
            check_opt(null) == 0
                and check_opt([]) == 1
                and check_opt([9, 10]) == 9;

        fn run() -> bool { OK }
    "#;
    assert_eq!(support::run(opt_ok), Value::Bool(true));

    let opt_missing = support::compile(
        r#"
        comptime fn bad_opt(xs: ?meta.List<i64>) -> i64 {
            xs match {
                null => 0,
                [x, ..._]? => x,
            }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        opt_missing.contains("non-exhaustive match") && opt_missing.contains("[]?"),
        "{opt_missing}"
    );
}

#[test]
fn comptime_list_or_pattern_rest_binder_checks_and_unreachable_arm() {
    let or_rest_ok = r#"
        comptime fn strip_zero_or_one(xs: meta.List<i64>) -> meta.List<i64> {
            xs match {
                [0, ...rest] or [1, ...rest] => rest,
                other => other,
            }
        }

        comptime fn run_comptime() -> bool {
            let a = strip_zero_or_one([0, 10, 20]);
            let b = strip_zero_or_one([1, 30]);
            let c = strip_zero_or_one([2, 40]);
            a.length() == 2 and a[0] == 10 and a[1] == 20
                and b.length() == 1 and b[0] == 30
                and c.length() == 2 and c[0] == 2
        }

        const OK: bool = run_comptime();
        fn run() -> bool { OK }
    "#;
    assert_eq!(
        support::invoke(or_rest_ok, "run_comptime").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::run(or_rest_ok), Value::Bool(true));

    let dup_binder = support::compile(
        r#"
        comptime fn bad(xs: meta.List<i64>) -> i64 {
            xs match {
                [a, ...a] => 1,
                _ => 0,
            }
        }
        "#,
    );
    assert!(
        dup_binder.is_err(),
        "duplicate binder [a, ...a] must be rejected"
    );

    let mismatched_or = support::compile(
        r#"
        comptime fn bad(xs: meta.List<i64>) -> i64 {
            xs match {
                [a, ...rest] or [a, ..._] => a,
                _ => 0,
            }
        }
        "#,
    );
    assert!(
        mismatched_or.is_err(),
        "mismatched or-pattern binders must be rejected"
    );

    let redundant_source = r#"
        comptime fn classify_redundant(xs: meta.List<i64>) -> i64 {
            xs match {
                [] => 0,
                [_, ..._] => 1,
                [1, 2] => 2,
            }
        }
        const C: i64 = classify_redundant([1, 2]);
        fn run() -> bool { C == 1 }
    "#;
    let pkg = support::compile(redundant_source).unwrap();
    let fn_id = support::source_function(&pkg, "classify_redundant");
    let plans = plans_for_fn(&pkg.match_plans, fn_id);
    assert_eq!(plans.len(), 1);
    assert!(plans[0].is_exhaustive);
    assert_eq!(plans[0].unreachable_arms, vec![2]);
    assert_eq!(support::run(redundant_source), Value::Bool(true));
}

#[test]
fn runtime_slice_and_vec_exhaustive_matching_and_rest_slicing_natively() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{AllocErr, ArenaAllocator};

        fn bounded_add(a: i64, b: i64) -> i64 {
            if a > -100000 and a < 100000 and b > -100000 and b < 100000 {
                a + b
            } else {
                0
            }
        }

        fn sum_slice(s: Slice<i64>) -> i64 {
            s match {
                [] => 0,
                [x] => x,
                [x, y, ...rest] => bounded_add(bounded_add(x, y), sum_slice(rest)),
            }
        }

        fn bool_slice_tag(s: Slice<bool>) -> i64 {
            s match {
                [] => 0,
                [true, ..._] => 1,
                [false, ..._] => 2,
            }
        }

        fn match_default_vec(v: Vec<i64>) -> i64 {
            v match {
                [] => 0,
                [only] => only,
                [first, ...rest] => {
                    if rest.ref.len() == 3 and first > -1000 and first < 1000 {
                        bounded_add(first * 100, sum_slice(rest))
                    } else {
                        -1000
                    }
                },
            }
        }

        fn exercise() -> !AllocErr bool {
            -- 1. Slice<i64> recursive rest peeling across lengths 0, 1, 2, 3, 5.
            var v = Vec<i64>.new();
            let s0 = v.ref.as_slice();
            if sum_slice(s0) != 0 or not s0.ref.is_empty() {
                return false;
            }
            v.ref.push(10)!;
            v.ref.push(20)!;
            v.ref.push(30)!;
            v.ref.push(40)!;
            v.ref.push(50)!;
            let s_all = v.ref.as_slice();
            if sum_slice(s_all) != 150 {
                return false;
            }
            let tail1 = s_all.ref.subslice_from(4) match { s? => s, null => return false };
            if sum_slice(tail1) != 50 {
                return false;
            }

            -- 2. Slice<bool> starting with empty [] arm.
            var bv = Vec<bool>.new();
            let b0 = bv.ref.as_slice();
            if bool_slice_tag(b0) != 0 {
                return false;
            }
            bv.ref.push(true)!;
            bv.ref.push(false)!;
            let b1 = bv.ref.as_slice();
            let b2 = b1.ref.subslice_from(1) match { s? => s, null => return false };
            if bool_slice_tag(b1) != 1 or bool_slice_tag(b2) != 2 {
                return false;
            }

            -- 3. Default-allocator Vec<i64> direct exhaustive match & ...rest => Slice<i64>.
            var empty_v = Vec<i64>.new();
            var single_v = Vec<i64>.new();
            single_v.ref.push(42)!;
            var multi_v = Vec<i64>.new();
            multi_v.ref.push(7)!;
            multi_v.ref.push(11)!;
            multi_v.ref.push(22)!;
            multi_v.ref.push(33)!;
            if match_default_vec(empty_v) != 0
                or match_default_vec(single_v) != 42
                or match_default_vec(multi_v) != 766
            {
                return false;
            }

            -- 4. Custom ArenaAllocator Vec<i64> direct exhaustive match & ...rest => Slice<i64>.
            var arena = ArenaAllocator.new();
            var av = Vec<i64, .A = ArenaAllocator, .allocator = arena.ref>.with_capacity(4)!;
            av.ref.push(5)!;
            av.ref.push(10)!;
            av.ref.push(15)!;
            let arena_res: i64 = av match {
                [] => -1,
                [x] => x,
                [x, ...rest] => {
                    if rest.ref.len() == 2 and x > -1000 and x < 1000 {
                        bounded_add(x * 10, sum_slice(rest))
                    } else {
                        -2
                    }
                },
            };
            arena_res == 75
        }

        fn run() -> bool {
            exercise() match {
                ok! => ok,
                error _ => false,
            }
        }
    "#;

    let pkg = support::compile(source).unwrap();
    let iir_pkg = luna::instantiation::instantiate(&pkg, "run", 1_000_000).unwrap();
    assert!(
        !iir_pkg.match_plans.is_empty(),
        "expected materialized iir match_plans"
    );
    assert_balanced_sexpr(&iir_pkg.match_plans.to_string());

    for fn_name in ["sum_slice", "bool_slice_tag", "match_default_vec"] {
        let id = support::source_function(&pkg, fn_name);
        let plans = plans_for_fn(&iir_pkg.match_plans, id);
        assert_eq!(plans.len(), 1, "expected 1 iir plan for {fn_name}");
        let plan = plans[0];
        assert!(plan.is_exhaustive, "{fn_name} plan should be exhaustive");
        assert!(plan.unreachable_arms.is_empty());
        let ops = plan
            .list_views
            .values()
            .next()
            .unwrap_or_else(|| panic!("expected list view ops for {fn_name}"));
        assert_eq!(ops.mode, ListObservationMode::StableProtocol);
        assert!(plan.dag.to_string().contains("(list-len "));
    }

    for opt_level in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt_level));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n", "opt={opt_level}");
    }
}

#[test]
fn runtime_slice_non_exhaustive_reports_missing_witnesses() {
    let missing_singleton = support::compile(
        r#"
        use std.prelude.*;
        fn bad(s: Slice<i64>) -> i64 {
            s match {
                [] => 0,
                [_, _, ..._] => 2,
            }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        missing_singleton.contains("non-exhaustive match")
            && missing_singleton.contains("missing pattern(s):")
            && missing_singleton.contains("[0]"),
        "{missing_singleton}"
    );

    let missing_false = support::compile(
        r#"
        use std.prelude.*;
        fn bad_bool(s: Slice<bool>) -> i64 {
            s match {
                [] => 0,
                [true, ..._] => 1,
            }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        missing_false.contains("non-exhaustive match") && missing_false.contains("[false]"),
        "{missing_false}"
    );
}

#[test]
fn generic_stable_list_pattern_view_and_quoted_list_rest_pattern() {
    let source = r#"
        use std.prelude.*;
        use std.mem.AllocErr;

        fn bounded_add(a: i64, b: i64) -> i64 {
            if a > -100000 and a < 100000 and b > -100000 and b < 100000 {
                a + b
            } else {
                0
            }
        }

        fn classify(xs: T) -> i64
        where
            T :- StableListPatternView,
            requires T.as(ListPatternView).Element == i64,
        {
            xs match {
                [] => 0,
                [x] => bounded_add(x, 10),
                [x, y, ..._] => bounded_add(bounded_add(x, y), 100),
            }
        }

        comptime fn head_tail_code() -> <Slice<i64> -| first: i64, rest: Slice<i64>> {
            '<[first, ...rest]>
        }

        fn sum_tail(s: Slice<i64>) -> i64 {
            s match {
                [] => 0,
                [x, ...rest] => bounded_add(x, sum_tail(rest)),
            }
        }

        fn inspect_quoted_rest(s: Slice<i64>) -> i64 {
            const pat = head_tail_code();
            s match {
                ~[first=hd, rest=tl]<pat> => {
                    if hd > -1000 and hd < 1000 {
                        bounded_add(hd * 10, sum_tail(tl))
                    } else {
                        0
                    }
                },
                _ => -1,
            }
        }

        fn exercise() -> !AllocErr bool {
            var v0 = Vec<i64>.new();
            var v1 = Vec<i64>.new();
            v1.ref.push(5)!;
            var v2 = Vec<i64>.new();
            v2.ref.push(2)!;
            v2.ref.push(3)!;
            v2.ref.push(9)!;

            let s0 = v0.ref.as_slice();
            let s1 = v1.ref.as_slice();
            let s2 = v2.ref.as_slice();

            let slice_ok = classify(s0) == 0 and classify(s1) == 15 and classify(s2) == 105;
            let quoted_ok = inspect_quoted_rest(s0) == -1
                and inspect_quoted_rest(s1) == 50
                and inspect_quoted_rest(s2) == 32;
            let vec_ok = classify(v0) == 0 and classify(v1) == 15 and classify(v2) == 105;
            slice_ok and quoted_ok and vec_ok
        }

        fn run() -> bool {
            exercise() match {
                ok! => ok,
                error _ => false,
            }
        }
    "#;

    for opt_level in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt_level));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n", "opt={opt_level}");
    }
}

#[test]
fn pure_methods_list_pattern_view_exhaustiveness_and_impure_rest_safety() {
    let pure_ok_source = r#"
        struct ConstPair {}

        impl builtin.ListPatternView for ConstPair {
            assoc Element: Type = i64;

            pure fn list_pattern_view_get_len(*self) -> usize { 2 }

            pure fn list_pattern_view_get_element(*self, comptime index: usize) -> i64 {
                if index == 0 { 10 } else { 20 }
            }
        }

        fn inspect_pure(p: ConstPair) -> i64 {
            p match {
                [] => 0,
                [x, ..._] => x,
            }
        }

        fn run() -> bool {
            inspect_pure(ConstPair {}) == 10
        }
    "#;

    assert_eq!(support::run(pure_ok_source), Value::Bool(true));

    let pkg = support::compile(pure_ok_source).unwrap();
    let fn_id = support::source_function(&pkg, "inspect_pure");
    let plans = plans_for_fn(&pkg.match_plans, fn_id);
    assert_eq!(plans.len(), 1);
    assert!(plans[0].is_exhaustive);
    let ops = plans[0]
        .list_views
        .values()
        .next()
        .expect("PureMethods list view ops");
    assert_eq!(ops.mode, ListObservationMode::PureMethods);

    // Non-pure `list_pattern_view_rest` mixed into a match without `_` must NOT be accepted
    // as single-observation exhaustive.
    let impure_rest_err = support::compile(
        r#"
        struct ConstPairWithImpureRest {}

        impl builtin.ListPatternView for ConstPairWithImpureRest {
            assoc Element: Type = i64;
            assoc Rest: Type = Self;

            pure fn list_pattern_view_get_len(*self) -> usize { 2 }

            pure fn list_pattern_view_get_element(*self, comptime index: usize) -> i64 {
                if index == 0 { 10 } else { 20 }
            }

            fn list_pattern_view_rest(*self, comptime _start: usize) -> Self {
                ConstPairWithImpureRest {}
            }
        }

        fn bad_impure_rest(p: ConstPairWithImpureRest) -> i64 {
            p match {
                [] => 0,
                [x, ...rest] => x,
                [x, ..._] => x,
            }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        impure_rest_err.contains("non-exhaustive match"),
        "{impure_rest_err}"
    );
}

#[test]
fn impure_list_pattern_view_requires_fallback_and_preserves_reobservation() {
    let view_def = r#"
        struct ImpureCounterView {
            reads: *usize,
            len: usize,
        }

        impl builtin.ListPatternView for ImpureCounterView {
            assoc Element: Type = usize;

            fn list_pattern_view_get_len(*self) -> usize {
                if self.reads.*.* < 100000 {
                    self.reads.*.* = self.reads.*.* + 1;
                }
                self.len.*
            }

            fn list_pattern_view_get_element(*self, comptime index: usize) -> usize {
                if self.reads.*.* < 100000 {
                    self.reads.*.* = self.reads.*.* + 10;
                }
                index
            }
        }
    "#;

    let missing_fallback = support::compile(&format!(
        "{view_def}
        fn bad(v: ImpureCounterView) -> usize {{
            v match {{
                [] => 0,
                [_, ..._] => 1,
            }}
        }}"
    ))
    .unwrap_err();
    assert!(
        missing_fallback.contains("non-exhaustive match"),
        "{missing_fallback}"
    );

    let with_fallback = format!(
        "{view_def}
        fn observe(v: ImpureCounterView) -> usize {{
            v match {{
                [1, _] => 100,
                [a, b] => if a < 100000 and b < 100000 {{ a + b }} else {{ 0 }},
                _ => 999,
            }}
        }}

        fn run() -> bool {{
            var reads: usize = 0;
            let v = ImpureCounterView {{ reads: reads.ref, len: 2 }};
            let res = observe(v);
            -- Arm 0 observes len (+1) and index 0 (+10 => 11) and fails (0 != 1).
            -- Arm 1 must freshly re-observe len (+1), index 0 (+10), index 1 (+10) => 32 total.
            res == 1 and reads == 32
        }}"
    );
    assert_eq!(support::run(&with_fallback), Value::Bool(true));

    let pkg = support::compile(&with_fallback).unwrap();
    let fn_id = support::source_function(&pkg, "observe");
    for plan in plans_for_fn(&pkg.match_plans, fn_id) {
        assert!(
            !plan.exact,
            "impure ListPatternView match plan must be marked inexact"
        );
        assert!(
            plan.list_views.is_empty(),
            "impure ListPatternView must not enter single-observation list_views"
        );
    }
}
