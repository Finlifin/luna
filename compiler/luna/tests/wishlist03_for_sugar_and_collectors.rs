//! Milestone A integration tests (`OQ-03-02`, `OQ-03-07`, `WL-03-01..05`, `WL-03-07`, `WL-03-24`):
//! - `for<parameter*> type_expr` syntactic sugar for `pure comptime fn<parameter*> -> type_expr`
//!   across `parse`, `fmt`, `ast_lowering`, `elaboration`, and `assoc Tmp: for<S: Type> Type`.
//! - Removal of legacy `assoc id<parameter*>: expr` syntax.
//! - Dependent `VariadicCollector.Tmp<S>` stack buffers and reverse `Drop` ordering.
//! - `TryVariadicCollector` (`assoc Tmp: for<S: Type> Type`, `assoc Err: Type`, `!Err` propagation,
//!   reverse `Tmp` cleanup on error, strict no-silent-fallback on missing error row / ambiguity).
//! - Tier 1 shipped collectors via real sysroot: `std.collections.Vec` (default + custom allocator,
//!   `!AllocErr` propagation) and `meta.List<T>` (comptime value-ref collection).
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn drop_markers(
    package: &nir::program::Package,
    entry: &str,
    allowed: &[&str],
) -> (Value, Vec<String>) {
    let (result, events) =
        luna::execution::interpret_with_protocols_events(package, entry, "Copy", "Drop", 2_000_000)
            .unwrap();
    let markers = events
        .into_iter()
        .filter(|name| allowed.iter().any(|candidate| candidate == name))
        .collect();
    (result, markers)
}

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-w03-m1-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "build",
                "main.fl",
                "--entry",
                "run",
                "--output",
                "program",
                "--opt-level",
                "0",
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .output()
            .unwrap()
    }

    fn run(&self) -> Output {
        Command::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }

    fn emit_fmt(&self, file: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args(["fmt", "--emit", "stdout", file])
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

#[test]
fn for_type_sugar_parses_formats_lowers_and_matches_pure_comptime_nf() {
    let fmt_src = r#"typealias UnaryType = for<S: Type> Type;
typealias DepType = for<T: Type, value: T> T;
typealias VariadicType = for<...Es: meta.List<Type>> Type;

trait Example {
    assoc Tmp: for<S: Type> Type = |_s: Type| -> Type { void };
}
"#;
    let case = NativeCase::new(fmt_src);
    let formatted = case.emit_fmt("main.fl");
    assert_success(&formatted);
    let stdout = String::from_utf8_lossy(&formatted.stdout);
    assert!(stdout.contains("for<S: Type> Type"), "{stdout}");
    assert!(stdout.contains("for<T: Type, value: T> T"), "{stdout}");
    assert!(
        stdout.contains("for<...Es: meta.List<Type>> Type"),
        "{stdout}"
    );
    assert!(stdout.contains("assoc Tmp: for<S: Type> Type"), "{stdout}");

    let source = r#"
        pure comptime fn Identity<T: Type> -> Type { T }
        pure comptime fn Tagged<T: Type, value: T> -> Type { T }

        typealias SugarUnary = for<T: Type> Type;
        typealias LongUnary = pure comptime fn<T: Type> -> Type;
        typealias SugarDep = for<T: Type, value: T> Type;
        typealias LongDep = pure comptime fn<T: Type, value: T> -> Type;
        typealias SugarVariadic = for<...Es: meta.List<Type>> Type;
        typealias LongVariadic = pure comptime fn<...Es: meta.List<Type>> -> Type;
        typealias SugarReturnDep = for<T: Type, value: T> T;
        typealias LongReturnDep = pure comptime fn<T: Type, value: T> -> T;

        comptime fn apply_unary(factory: for<Type> Type, x: Type) -> Type {
            factory<x>
        }

        comptime fn apply_tuple(factory: for<...Es: meta.List<Type>> Type) -> Type {
            factory<u32, bool>
        }

        const EXPR_SUGAR: Type = for<S: Type> Type;
        const EXPR_DEP: Type = for<T: Type, x: T> T;

        comptime fn check_types() -> bool {
            const u_eq = SugarUnary == LongUnary and EXPR_SUGAR == LongUnary;
            const d_eq = SugarDep == LongDep;
            const v_eq = SugarVariadic == LongVariadic;
            const r_eq = SugarReturnDep == LongReturnDep and EXPR_DEP == LongReturnDep;
            const app_u = apply_unary(Identity, u32) == Identity<u32>;
            const app_t = apply_tuple(Tuple) == Tuple<u32, bool>;
            const tag_t: Type = Tagged<bool, true>;
            const tag_eq = tag_t == Tagged<bool, true>;
            let local_ty: Type = for<T: Type> ?T;
            let expected_opt: Type = pure comptime fn<T: Type> -> ?T;
            u_eq and d_eq and v_eq and r_eq and app_u and app_t and tag_eq and (local_ty == expected_opt)
        }
    "#;
    assert_eq!(
        support::invoke(source, "check_types").unwrap(),
        Value::Bool(true)
    );

    // Removed legacy `assoc id<param>: expr` syntax must be rejected by the parser.
    let err = support::compile("trait Legacy { assoc Tmp<S: Type>: Type = void; }").unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn dependent_variadic_collector_tmp_and_reverse_drop_order() {
    let source = r#"
        static FALLBACK_VAL: u32 = 0;

        fn mark_tmp_one() {}
        fn mark_tmp_two() {}
        fn mark_outer() {}

        struct TmpOne {
            a: u32,
            tag: u32,
        }
        impl Drop for TmpOne {
            fn drop(*self) {
                let _ = self.tag.*;
                mark_tmp_one();
            }
        }

        struct TmpTwo {
            a: u32,
            b: u32,
            tag: u32,
        }
        impl Drop for TmpTwo {
            fn drop(*self) {
                let _ = self.tag.*;
                mark_tmp_two();
            }
        }

        struct Bag {
            sum: u32,
            seen_Ptr: *u32,
        }

        impl VariadicCollector for Bag {
            assoc Tmp: for<S: Type> Type = |s: Type| -> Type {
                if s == Tuple<u32, u32> {
                    TmpTwo
                } else {
                    TmpOne
                }
            };

            fn variadic_init() -> Tmp<S> where S {
                inline if S == Tuple<u32, u32> {
                    TmpTwo { a: 0, b: 0, tag: 2 }
                } else {
                    TmpOne { a: 0, tag: 1 }
                }
            }

            fn variadic_collect(tmp: *Tmp<S>, args: S) -> Bag where S {
                inline if S == Tuple<u32, u32> {
                    let typed: Tuple<u32, u32> = args;
                    let (x, y) = typed;
                    assumes x < 1000 and y < 1000;
                    tmp.a.* = x;
                    tmp.b.* = y;
                    let av = tmp.a.*;
                    let bv = tmp.b.*;
                    assumes av < 1000 and bv < 1000;
                    Bag { sum: av + bv, seen_Ptr: tmp.*.a.ref }
                } else {
                    inline if S == Tuple<u32> {
                        let typed: Tuple<u32> = args;
                        let (x,) = typed;
                        tmp.a.* = x;
                        Bag { sum: tmp.a.*, seen_Ptr: tmp.*.a.ref }
                    } else {
                        assumes false;
                        Bag { sum: 0, seen_Ptr: FALLBACK_VAL.ref }
                    }
                }
            }
        }

        struct OuterTag {
            tag: u32,
        }
        impl Drop for OuterTag {
            fn drop(*self) {
                let _ = self.tag.*;
                mark_outer();
            }
        }

        fn eval_bag(outer: OuterTag, ...xs: Bag) -> u32 {
            let seen = xs.seen_Ptr.*;
            assumes seen < 1000 and xs.sum < 1000;
            seen + xs.sum
        }

        fn run_one() -> bool {
            let c: u32 = 5;
            eval_bag(OuterTag { tag: 7 }, c) == (5 + 5)
        }

        fn run_two() -> bool {
            let a: u32 = 10;
            let b: u32 = 20;
            eval_bag(OuterTag { tag: 8 }, a, b) == (10 + 30)
        }

        fn run() -> bool {
            run_one() and run_two()
        }
    "#;
    let package = support::compile(source).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 2_000_000)
            .unwrap(),
        Value::Bool(true)
    );
    let allowed = ["mark_tmp_one", "mark_tmp_two", "mark_outer"];
    let (v1, trace1) = drop_markers(&package, "run_one", &allowed);
    assert_eq!(v1, Value::Bool(true));
    assert_eq!(trace1, vec!["mark_outer", "mark_tmp_one"]);
    let (v2, trace2) = drop_markers(&package, "run_two", &allowed);
    assert_eq!(v2, Value::Bool(true));
    assert_eq!(trace2, vec!["mark_outer", "mark_tmp_two"]);
}

#[test]
fn try_variadic_collector_propagates_err_drops_tmp_reverse_and_rejects_missing_row_or_ambiguity() {
    let ok_and_fail_source = r#"
        enum CollectErr {
            LimitExceeded,
        }

        fn mark_tmp_init() {}
        fn mark_tmp_updated() {}
        fn mark_outer() {}
        fn mark_sum() {}

        struct GuardTmp {
            tag: u32,
        }
        impl Drop for GuardTmp {
            fn drop(*self) {
                if self.tag.* == 2 {
                    mark_tmp_updated();
                } else {
                    mark_tmp_init();
                }
            }
        }

        struct BoundedSum {
            sum: u32,
        }
        impl Drop for BoundedSum {
            fn drop(*self) {
                let _ = self.sum.*;
                mark_sum();
            }
        }

        impl TryVariadicCollector for BoundedSum {
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type { GuardTmp };
            assoc Err: Type = CollectErr;

            fn try_variadic_init() -> Tmp<S> where S {
                GuardTmp { tag: 1 }
            }

            fn try_variadic_collect(tmp: *Tmp<S>, args: S) -> !CollectErr BoundedSum where S {
                tmp.tag.* = 2;
                var total: u32 = 0;
                inline if Type.predicates.is_tuple(S) {
                    inline for (_idx, elem) in args'tuple_indexed {
                        inline if elem'type == u32 {
                            let x: u32 = elem;
                            if x > 50 {
                                return error CollectErr.LimitExceeded;
                            }
                            assumes total < 1000 and x <= 50;
                            total = total + x;
                        } else {
                            assumes false;
                        }
                    }
                }
                BoundedSum { sum: total }
            }
        }

        struct OuterGuard {
            tag: u32,
        }
        impl Drop for OuterGuard {
            fn drop(*self) {
                let _ = self.tag.*;
                mark_outer();
            }
        }

        fn sink(guard: OuterGuard, ...xs: BoundedSum) -> u32 {
            assumes xs.sum < 1000;
            xs.sum
        }

        fn try_call(x: u32, y: u32) -> !CollectErr u32 {
            sink(OuterGuard { tag: 7 }, x, y)
        }

        fn run_ok() -> bool {
            try_call(10, 20) match {
                v! => v == 30,
                error _ => false,
            }
        }

        fn run_fail() -> bool {
            try_call(10, 99) match {
                _! => false,
                error CollectErr.LimitExceeded => true,
            }
        }

        fn run() -> bool {
            run_ok() and run_fail()
        }
    "#;
    let pkg = support::compile(ok_and_fail_source).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&pkg, "run", "Copy", "Drop", 2_000_000).unwrap(),
        Value::Bool(true)
    );
    let allowed = [
        "mark_tmp_init",
        "mark_tmp_updated",
        "mark_outer",
        "mark_sum",
    ];
    let (v_ok, trace_ok) = drop_markers(&pkg, "run_ok", &allowed);
    assert_eq!(v_ok, Value::Bool(true));
    assert_eq!(trace_ok, vec!["mark_sum", "mark_outer", "mark_tmp_updated"]);
    let (v_fail, trace_fail) = drop_markers(&pkg, "run_fail", &allowed);
    assert_eq!(v_fail, Value::Bool(true));
    assert_eq!(trace_fail, vec!["mark_tmp_updated", "mark_outer"]);

    // Missing `!CollectErr` on caller return type must be rejected loudly (no silent fallback).
    let missing_err_row = r#"
        enum CollectErr { Fail }
        struct FallibleBag {}
        impl TryVariadicCollector for FallibleBag {
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type { void };
            assoc Err: Type = CollectErr;
            fn try_variadic_init() -> Tmp<S> where S { () }
            fn try_variadic_collect(_tmp: *Tmp<S>, _args: S) -> !CollectErr FallibleBag where S {
                error CollectErr.Fail
            }
        }
        fn take(...xs: FallibleBag) -> u32 { 1 }
        fn caller() -> u32 {
            take(1, 2)
        }
    "#;
    let err = support::compile(missing_err_row).unwrap_err();
    assert!(!err.is_empty());

    // Wrong error row type (not accepting CollectErr) must be rejected.
    let wrong_err_row = r#"
        enum CollectErr { Fail }
        enum OtherErr { Other }
        struct FallibleBag {}
        impl TryVariadicCollector for FallibleBag {
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type { void };
            assoc Err: Type = CollectErr;
            fn try_variadic_init() -> Tmp<S> where S { () }
            fn try_variadic_collect(_tmp: *Tmp<S>, _args: S) -> !CollectErr FallibleBag where S {
                error CollectErr.Fail
            }
        }
        fn take(...xs: FallibleBag) -> u32 { 1 }
        fn caller() -> !OtherErr u32 {
            take(1, 2)
        }
    "#;
    let err = support::compile(wrong_err_row).unwrap_err();
    assert!(!err.is_empty());

    // Implementing BOTH VariadicCollector and TryVariadicCollector on one type must fail ambiguity.
    let both_protocols = r#"
        enum CollectErr { Fail }
        struct Ambig {}
        impl VariadicCollector for Ambig {
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type { void };
            fn variadic_init() -> Tmp<S> where S { () }
            fn variadic_collect(_tmp: *Tmp<S>, _args: S) -> Ambig where S { Ambig {} }
        }
        impl TryVariadicCollector for Ambig {
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type { void };
            assoc Err: Type = CollectErr;
            fn try_variadic_init() -> Tmp<S> where S { () }
            fn try_variadic_collect(_tmp: *Tmp<S>, _args: S) -> !CollectErr Ambig where S {
                Ambig {}
            }
        }
        fn take(...xs: Ambig) -> u32 { 1 }
        fn caller() -> !CollectErr u32 {
            take(1, 2)
        }
    "#;
    let err = support::compile(both_protocols).unwrap_err();
    assert!(err.contains("ambiguous"), "{err}");
}

#[test]
fn std_vec_try_variadic_collector_and_meta_list_collector_work_with_real_sysroot() {
    // 1. meta.List<T> comptime VariadicCollector via real sysroot.
    let meta_src = r#"
        comptime fn sum_list(...items: meta.List<Integer>) -> Integer {
            var acc: Integer = 0;
            inline for i in 0..items.length() {
                acc = acc + items[i];
            }
            acc
        }

        comptime fn sum_collected_list(...items: meta.List<Integer>) -> Integer {
            var acc: Integer = 0;
            inline for i in 0..items.length() {
                acc = acc + items[i];
            }
            acc
        }

        const TOTAL: Integer = sum_list(10, 20, 12);
        const COLLECTED_TOTAL: Integer = sum_collected_list(10, 20, 12);

        comptime fn check_meta() -> bool {
            TOTAL == 42 and COLLECTED_TOTAL == 42 and sum_list() == 0 and sum_collected_list() == 0
        }
    "#;
    assert_eq!(
        support::invoke(meta_src, "check_meta").unwrap(),
        Value::Bool(true)
    );

    // 2. std.collections.Vec<T> with DefaultAllocator AND custom Allocator + !AllocErr propagation.
    let vec_src = r#"
        use std.collections.Vec;
        use std.mem.{AllocErr, Allocator, Layout, RawBlock, ReallocErr};

        struct FailingAlloc {
            fail_after: usize,
            allocs: usize,
            frees: usize,
        }

        impl Allocator for FailingAlloc {
            fn allocate(*self, layout: Layout) -> !AllocErr RawBlock {
                let count = self.allocs.*;
                if count >= self.fail_after.* or count >= 1000000 {
                    return error AllocErr.OutOfMemory;
                }
                self.allocs.* = count + 1;
                std.mem.DefaultAllocator.GLOBAL.ref.as(Allocator).allocate(layout)
            }

            fn allocate_zeroed(*self, layout: Layout) -> !AllocErr RawBlock {
                let count = self.allocs.*;
                if count >= self.fail_after.* or count >= 1000000 {
                    return error AllocErr.OutOfMemory;
                }
                self.allocs.* = count + 1;
                std.mem.DefaultAllocator.GLOBAL.ref.as(Allocator).allocate_zeroed(layout)
            }

            unsafe fn reallocate(*self, block: RawBlock, new_layout: Layout) -> !ReallocErr RawBlock {
                let count = self.allocs.*;
                if count >= self.fail_after.* or count >= 1000000 {
                    return error ReallocErr.OutOfMemory;
                }
                self.allocs.* = count + 1;
                let next = unsafe {
                    std.mem.DefaultAllocator.GLOBAL.ref.as(Allocator).reallocate(block, new_layout)
                }!;
                let frees = self.frees.*;
                if frees < 1000000 {
                    self.frees.* = frees + 1;
                }
                next
            }

            unsafe fn deallocate(*self, block: RawBlock) {
                let frees = self.frees.*;
                if frees < 1000000 {
                    self.frees.* = frees + 1;
                }
                unsafe {
                    std.mem.DefaultAllocator.GLOBAL.ref.as(Allocator).deallocate(block);
                }
            }
        }

        fn sum_default(...xs: Vec<u32>) -> u32 {
            var acc: u32 = 0;
            var i: usize = 0;
            let len = xs.len();
            while i < len {
                let v = xs.get(i) match {
                    elem? => elem,
                    null => 0,
                };
                assumes acc < 10000 and v < 1000;
                acc = acc + v;
                i = i + 1;
            }
            acc
        }

        fn collect_default(a: u32, b: u32, c: u32) -> !AllocErr u32 {
            sum_default(a, b, c)
        }

        fn collect_empty() -> !AllocErr u32 {
            sum_default()
        }

        fn sum_custom(
            ...xs: Vec<u32, .A = FailingAlloc, .allocator = custom>,
        ) -> usize
        where
            quote custom: lift *FailingAlloc,
        {
            xs.len()
        }

        fn collect_custom(alloc: *FailingAlloc) -> !AllocErr usize {
            sum_custom<alloc>(1u32, 2u32, 3u32, 4u32, 5u32)
        }

        fn run() -> bool {
            let def_ok = collect_default(10, 20, 12) match {
                v! => v == 42,
                error _ => false,
            };
            let empty_ok = collect_empty() match {
                v! => v == 0,
                error _ => false,
            };

            var ok_alloc = FailingAlloc { fail_after: 10, allocs: 0, frees: 0 };
            let custom_ok = collect_custom(ok_alloc.ref) match {
                n! => n == 5,
                error _ => false,
            };
            let custom_freed = ok_alloc.allocs > 0 and ok_alloc.allocs == ok_alloc.frees;

            var oom_alloc = FailingAlloc { fail_after: 1, allocs: 0, frees: 0 };
            let custom_oom = collect_custom(oom_alloc.ref) match {
                _! => false,
                error AllocErr.OutOfMemory => true,
                error _ => false,
            };
            -- On second push reallocation OOM, partially constructed Vec was dropped via Drop!
            let oom_freed = oom_alloc.allocs == 1 and oom_alloc.frees == 1;

            def_ok and empty_ok and custom_ok and custom_freed and custom_oom and oom_freed
        }

        fn main() -> i32 {
            if run() { 0 } else { 1 }
        }
    "#;
    let case = NativeCase::new(vec_src);
    let build = case.build();
    assert_success(&build);
    let out = case.run();
    assert_success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}
