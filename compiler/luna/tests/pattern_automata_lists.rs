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

#[test]
fn list_pattern_view_default_rest_rejects_rest_binding_and_direct_call() {
    let wild_ok = r#"
        struct RepeatTwo { first: i64, second: i64 }
        impl builtin.ListPatternView for RepeatTwo {
            assoc Element: Type = i64;
            fn list_pattern_view_get_len(*self) -> usize { 2 }
            fn list_pattern_view_get_element(*self, comptime index: usize) -> i64 {
                if index == 0 { self.first.* } else { self.second.* }
            }
        }
        fn run() -> bool {
            let r = RepeatTwo { first: 11, second: 22 };
            r match {
                [a, ..._] => a == 11,
                _ => false,
            }
        }
    "#;
    assert_eq!(support::run(wild_ok), Value::Bool(true));

    let expected_err = "type implements ListPatternView without overriding list_pattern_view_rest required for '...rest' binding";

    let named_rest_err = support::compile(&format!(
        "{wild_ok}
        fn bad_rest(r: RepeatTwo) -> bool {{
            r match {{
                [_, ...rest] => rest.first == 11,
                _ => false,
            }}
        }}"
    ))
    .unwrap_err();
    assert!(named_rest_err.contains(expected_err), "{named_rest_err}");

    let direct_as_call_err = support::compile(&format!(
        "{wild_ok}
        fn bad_direct_as(r: RepeatTwo) -> bool {{
            let rest = r.as(builtin.ListPatternView).list_pattern_view_rest(1);
            rest.first == 11
        }}"
    ))
    .unwrap_err();
    assert!(
        direct_as_call_err.contains(expected_err),
        "{direct_as_call_err}"
    );

    let direct_method_err = support::compile(&format!(
        "{wild_ok}
        fn bad_direct_method(r: RepeatTwo) -> bool {{
            let rest = r.list_pattern_view_rest(1);
            rest.first == 11
        }}"
    ))
    .unwrap_err();
    assert!(
        direct_method_err.contains(expected_err),
        "{direct_method_err}"
    );
}

#[test]
fn two_phase_list_pattern_view_works_in_runtime_and_comptime() {
    let source = r#"
        struct PairSeq { a: i64, b: i64, count: usize }

        impl builtin.ListPatternView for PairSeq {
            assoc Element: Type = i64;
            assoc Rest: Type = PairSeq;

            fn list_pattern_view_get_len(*self) -> usize { self.count.* }

            fn list_pattern_view_get_element(*self, comptime index: usize) -> i64 {
                if index == 0 { self.a.* } else { self.b.* }
            }

            fn list_pattern_view_rest(*self, comptime start: usize) -> PairSeq {
                let c = self.count.*;
                if start >= c {
                    PairSeq { a: 0, b: 0, count: 0 }
                } else if start == 0 {
                    PairSeq { a: self.a.*, b: self.b.*, count: c }
                } else {
                    PairSeq { a: self.b.*, b: 0, count: c - start }
                }
            }
        }

        impl builtin.StableListPatternView for PairSeq {}

        comptime impl builtin.ListPatternView for PairSeq {
            assoc ElementComptime: Type = i64;
            assoc RestComptime: Type = PairSeq;

            pure comptime fn list_pattern_view_get_len_comptime(self) -> usize { self.count }

            pure comptime fn list_pattern_view_get_element_comptime(self, index: usize) -> i64 {
                if index == 0 { self.a } else { self.b }
            }

            pure comptime fn list_pattern_view_rest_comptime(self, start: usize) -> PairSeq {
                if start >= self.count {
                    PairSeq { a: 0, b: 0, count: 0 }
                } else if start == 0 {
                    self
                } else {
                    PairSeq { a: self.b, b: 0, count: self.count - start }
                }
            }
        }

        comptime impl builtin.StableListPatternView for PairSeq {}

        fn rt(p: PairSeq) -> i64 {
            p match {
                [] => 0,
                [x] => x,
                [x, ...rest] => {
                    if x >= 0 and x < 1000 and rest.a >= 0 and rest.a < 1000 {
                        x * 10 + rest.a
                    } else {
                        0
                    }
                },
            }
        }

        comptime fn ct() -> bool {
            let p = PairSeq { a: 3, b: 7, count: 2 };
            let ok_first = p match {
                [] => false,
                [x, ...rest] => x == 3 and rest.count == 1 and rest.a == 7,
            };
            ok_first
        }

        const CT_OK: bool = ct();

        fn run() -> bool {
            CT_OK and rt(PairSeq { a: 4, b: 9, count: 2 }) == 49
                and rt(PairSeq { a: 5, b: 0, count: 1 }) == 5
                and rt(PairSeq { a: 0, b: 0, count: 0 }) == 0
        }
    "#;

    assert_eq!(support::run(source), Value::Bool(true));
    assert_eq!(support::invoke(source, "ct").unwrap(), Value::Bool(true));
}

#[test]
fn endless_list_pattern_view_and_stable_exhaustiveness_and_errors() {
    let fib_source = r#"
        struct FibStream { a: i64, b: i64 }

        impl builtin.ListPatternViewEndless for FibStream {
            assoc Element: Type = i64;
            assoc Rest: Type = FibStream;

            fn list_pattern_view_get_element(*self, comptime index: usize) -> i64 {
                var cur_a = self.a.*;
                var cur_b = self.b.*;
                var i: usize = 0;
                while i < index {
                    let next = if cur_a >= 0 and cur_b >= 0 and cur_a < 10000 and cur_b < 10000 {
                        cur_a + cur_b
                    } else {
                        0
                    };
                    cur_a = cur_b;
                    cur_b = next;
                    i = i + 1;
                }
                cur_a
            }

            fn list_pattern_view_rest(*self, comptime start: usize) -> FibStream {
                var cur_a = self.a.*;
                var cur_b = self.b.*;
                var i: usize = 0;
                while i < start {
                    let next = if cur_a >= 0 and cur_b >= 0 and cur_a < 10000 and cur_b < 10000 {
                        cur_a + cur_b
                    } else {
                        0
                    };
                    cur_a = cur_b;
                    cur_b = next;
                    i = i + 1;
                }
                FibStream { a: cur_a, b: cur_b }
            }
        }

        impl builtin.StableListPatternViewEndless for FibStream {}

        comptime impl builtin.ListPatternViewEndless for FibStream {
            assoc ElementComptime: Type = i64;
            assoc RestComptime: Type = FibStream;

            pure comptime fn list_pattern_view_get_element_comptime(self, index: usize) -> i64 {
                var cur_a = self.a;
                var cur_b = self.b;
                var i: usize = 0;
                while i < index {
                    let next = if cur_a >= 0 and cur_b >= 0 and cur_a < 10000 and cur_b < 10000 {
                        cur_a + cur_b
                    } else {
                        0
                    };
                    cur_a = cur_b;
                    cur_b = next;
                    i = i + 1;
                }
                cur_a
            }

            pure comptime fn list_pattern_view_rest_comptime(self, start: usize) -> FibStream {
                var cur_a = self.a;
                var cur_b = self.b;
                var i: usize = 0;
                while i < start {
                    let next = if cur_a >= 0 and cur_b >= 0 and cur_a < 10000 and cur_b < 10000 {
                        cur_a + cur_b
                    } else {
                        0
                    };
                    cur_a = cur_b;
                    cur_b = next;
                    i = i + 1;
                }
                FibStream { a: cur_a, b: cur_b }
            }
        }

        comptime impl builtin.StableListPatternViewEndless for FibStream {}

        struct BoolStream { head: bool }

        impl builtin.ListPatternViewEndless for BoolStream {
            assoc Element: Type = bool;

            fn list_pattern_view_get_element(*self, comptime index: usize) -> bool {
                if index == 0 { self.head.* } else { not self.head.* }
            }
        }

        impl builtin.StableListPatternViewEndless for BoolStream {}

        fn inspect_fib(s: FibStream) -> i64 {
            s match {
                [0, 1, third, ...tail] => {
                    if third >= 0 and third < 1000 and tail.a >= 0 and tail.a < 1000 and tail.b >= 0 and tail.b < 1000 {
                        third * 100 + tail.a * 10 + tail.b
                    } else {
                        0
                    }
                },
                [first, ..._] => first,
            }
        }

        fn inspect_bools(s: BoolStream) -> bool {
            s match {
                [true, ..._] => true,
                [false, ..._] => false,
            }
        }

        comptime fn ct_fib() -> i64 {
            let s = FibStream { a: 0, b: 1 };
            s match {
                [0, 1, third, ...tail] => {
                    if third >= 0 and third < 1000 and tail.a >= 0 and tail.a < 1000 and tail.b >= 0 and tail.b < 1000 {
                        third * 100 + tail.a * 10 + tail.b
                    } else {
                        0
                    }
                },
                [first, ..._] => first,
            }
        }

        const CT_VAL: i64 = ct_fib();

        comptime fn ct_fib_ok() -> bool {
            ct_fib() == 123
        }

        fn run() -> bool {
            CT_VAL == 123
                and inspect_fib(FibStream { a: 0, b: 1 }) == 123
                and inspect_fib(FibStream { a: 7, b: 8 }) == 7
                and inspect_bools(BoolStream { head: true })
                and not inspect_bools(BoolStream { head: false })
        }
    "#;

    assert_eq!(support::run(fib_source), Value::Bool(true));
    assert_eq!(
        support::invoke(fib_source, "ct_fib_ok").unwrap(),
        Value::Bool(true)
    );

    let pkg = support::compile(fib_source).unwrap();
    for fn_name in ["inspect_fib", "inspect_bools"] {
        let fn_id = support::source_function(&pkg, fn_name);
        let plans = plans_for_fn(&pkg.match_plans, fn_id);
        assert_eq!(plans.len(), 1);
        assert!(plans[0].is_exhaustive);
        let ops = plans[0]
            .list_views
            .values()
            .next()
            .expect("endless list ops");
        assert!(ops.endless);
        assert_eq!(ops.mode, ListObservationMode::StableProtocol);
    }

    let missing_bool = support::compile(&format!(
        "{fib_source}
        fn non_exhaustive(s: BoolStream) -> bool {{
            s match {{
                [true, ..._] => true,
            }}
        }}"
    ))
    .unwrap_err();
    assert!(
        missing_bool.contains("non-exhaustive match") && missing_bool.contains("[false, ..._]"),
        "{missing_bool}"
    );

    let fixed_len_err = support::compile(&format!(
        "{fib_source}
        fn bad_fixed(s: FibStream) -> i64 {{
            s match {{
                [a, b] => a + b,
                [a, ..._] => a,
            }}
        }}"
    ))
    .unwrap_err();
    assert!(
        fixed_len_err.contains("endless list pattern requires a trailing '..._' or '...rest'"),
        "{fixed_len_err}"
    );

    let both_views_err = support::compile(&format!(
        "{fib_source}
        impl builtin.ListPatternView for FibStream {{
            assoc Element: Type = i64;
            fn list_pattern_view_get_len(*self) -> usize {{ 1 }}
            fn list_pattern_view_get_element(*self, comptime _index: usize) -> i64 {{ self.a.* }}
        }}
        fn bad_both(s: FibStream) -> i64 {{
            s match {{
                [a, ..._] => a,
            }}
        }}"
    ))
    .unwrap_err();
    assert!(
        both_views_err.contains(
            "ambiguous list pattern view: type implements both ListPatternView and ListPatternViewEndless"
        ),
        "{both_views_err}"
    );

    let default_rest_err = support::compile(&format!(
        "{fib_source}
        fn bad_default_rest(s: BoolStream) -> bool {{
            s match {{
                [a, ...rest] => a and rest.head,
            }}
        }}"
    ))
    .unwrap_err();
    assert!(
        default_rest_err.contains(
            "type implements ListPatternViewEndless without overriding list_pattern_view_rest required for '...rest' binding"
        ),
        "{default_rest_err}"
    );
}

#[test]
fn vec_deque_and_vec_deque_slice_list_patterns_wraparound_and_exhaustiveness() {
    let source = r#"
        use std.prelude.*;
        use std.collections.{VecDeque, VecDequeSlice};

        fn sum_slice(s: VecDequeSlice<i64>) -> i64 {
            var cur = s;
            var total: i64 = 0;
            while cur is [head, ...tail] do {
                if total >= 0 and total < 10000 and head >= 0 and head < 10000 {
                    total = total + head;
                }
                cur = tail;
            }
            total
        }

        fn classify_bools(d: VecDeque<bool>) -> i64 {
            d match {
                [] => 0,
                [true, ..._] => 1,
                [false, ..._] => 2,
            }
        }

        fn run() -> bool {
            var dq = VecDeque<i64>.new();
            dq.push_back(10) match {
                ok! => {},
                error _ => return false,
            }
            dq.push_back(20) match {
                ok! => {},
                error _ => return false,
            }
            dq.push_back(30) match {
                ok! => {},
                error _ => return false,
            }
            let popped = dq.pop_front() match {
                v? => v,
                null => return false,
            };
            if popped != 10 {
                return false;
            }
            -- Push more elements to force ring-buffer wraparound (second_part non-empty).
            dq.push_back(40) match {
                ok! => {},
                error _ => return false,
            }
            dq.push_back(50) match {
                ok! => {},
                error _ => return false,
            }
            let sum_all = dq match {
                [first, second, ...rest] => {
                    let tail_sum = sum_slice(rest);
                    if first >= 0 and first < 10000 and second >= 0 and second < 10000 and tail_sum >= 0 and tail_sum < 100000 {
                        first + second + tail_sum
                    } else {
                        -1
                    }
                },
                _ => -1,
            };
            if sum_all != 140 {
                return false;
            }
            let empty_bd = VecDeque<bool>.new();
            if classify_bools(empty_bd) != 0 {
                return false;
            }
            var bd = VecDeque<bool>.new();
            bd.push_back(false) match {
                ok! => {},
                error _ => return false,
            }
            classify_bools(bd) == 2
        }
    "#;

    let pkg = support::compile(source).unwrap();
    let iir_pkg = luna::instantiation::instantiate(&pkg, "run", 1_000_000).unwrap();
    let fn_id = support::source_function(&pkg, "classify_bools");
    let plans = plans_for_fn(&iir_pkg.match_plans, fn_id);
    assert_eq!(plans.len(), 1);
    assert!(plans[0].is_exhaustive);
    let ops = plans[0]
        .list_views
        .values()
        .next()
        .expect("VecDeque list view ops");
    assert_eq!(ops.mode, ListObservationMode::StableProtocol);

    for opt_level in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt_level));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n", "opt={opt_level}");
    }
}
