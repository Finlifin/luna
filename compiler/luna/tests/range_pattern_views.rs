//! End-to-end tests for `RangeEnd<T>`, single-trait two-phase `RangePatternView` /
//! `StableRangePatternView`, heterogeneous `Bound`, all 5 range shapes, `Ord` fallback,
//! NaN preservation, `MatchPlan` integration, and endpoint type-mismatch rejection.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use nir::DeclarationId;
use nir::match_plan::{MatchPlan, MatchPlans};
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-range-view-{}-{}",
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

fn verify_interpret_and_native(source: &str) {
    assert_eq!(support::run(source), Value::Bool(true));
    let native = NativeCase::new(source);
    for opt in ["0", "3"] {
        let build = native.build(opt);
        assert!(
            build.status.success(),
            "build opt={opt} failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        let run = native.run();
        assert!(
            run.status.success(),
            "run opt={opt} failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            "(bool true)",
            "opt={opt}"
        );
    }
}

fn plans_for_fn(plans: &MatchPlans, fn_id: DeclarationId) -> Vec<&MatchPlan> {
    plans
        .plans
        .iter()
        .filter(|(key, _)| key.function == Some(fn_id))
        .map(|(_, plan)| plan)
        .collect()
}

#[test]
fn heterogeneous_bound_all_five_shapes_and_two_phase_execution() {
    let source = r#"
        struct Endpoint {
            port: u16,
            tls: bool,
        }

        impl Clone for Endpoint {
            fn clone(*self) -> Self { self.* }
        }

        impl Copy for Endpoint {}

        impl RangePatternView for Endpoint {
            assoc Bound: Type = u16;

            fn range_pattern_view_contains(
                *self,
                start: RangeEnd<u16>,
                end: RangeEnd<u16>,
            ) -> bool {
                let p = self.port.*;
                let lo_ok = start match {
                    .unbounded => true,
                    .inclusive(lo) => p >= lo,
                    .exclusive(lo) => p > lo,
                };
                let hi_ok = end match {
                    .unbounded => true,
                    .inclusive(hi) => p <= hi,
                    .exclusive(hi) => p < hi,
                };
                lo_ok and hi_ok
            }
        }

        impl StableRangePatternView for Endpoint {}

        comptime impl RangePatternView for Endpoint {
            assoc BoundComptime: Type = u16;

            comptime fn range_pattern_view_contains_comptime(
                self,
                start: RangeEnd<u16>,
                end: RangeEnd<u16>,
            ) -> bool {
                let p = self.port;
                let lo_ok = start match {
                    .unbounded => true,
                    .inclusive(lo) => p >= lo,
                    .exclusive(lo) => p > lo,
                };
                let hi_ok = end match {
                    .unbounded => true,
                    .inclusive(hi) => p <= hi,
                    .exclusive(hi) => p < hi,
                };
                lo_ok and hi_ok
            }
        }

        comptime impl StableRangePatternView for Endpoint {}

        fn shape_to_hi(ep: Endpoint) -> bool {
            ep match {
                ..80 => true,
                _ => false,
            }
        }

        fn shape_to_inc_hi(ep: Endpoint) -> bool {
            ep match {
                ..=80 => true,
                _ => false,
            }
        }

        fn shape_lo_from(ep: Endpoint) -> bool {
            ep match {
                49152.. => true,
                _ => false,
            }
        }

        fn shape_lo_hi(ep: Endpoint) -> bool {
            ep match {
                1024..49152 => true,
                _ => false,
            }
        }

        fn shape_lo_inc_hi(ep: Endpoint) -> bool {
            ep match {
                80..=443 => true,
                _ => false,
            }
        }

        fn shape_full_wildcard(ep: Endpoint) -> bool {
            ep match {
                .. => true,
            }
        }

        fn classify(ep: Endpoint) -> u8 {
            ep match {
                ..80 => 1,
                80..=443 => 2,
                1024..49152 => 3,
                49152.. => 4,
                _ => 0,
            }
        }

        comptime fn classify_ct(ep: Endpoint) -> u8 {
            ep match {
                ..80 => 1,
                80..=443 => 2,
                1024..49152 => 3,
                49152.. => 4,
                _ => 0,
            }
        }

        comptime fn ct_check() -> bool {
            classify_ct(Endpoint { port: 22, tls: false }) == 1
                and classify_ct(Endpoint { port: 80, tls: false }) == 2
                and classify_ct(Endpoint { port: 443, tls: true }) == 2
                and classify_ct(Endpoint { port: 800, tls: false }) == 0
                and classify_ct(Endpoint { port: 1024, tls: false }) == 3
                and classify_ct(Endpoint { port: 49151, tls: false }) == 3
                and classify_ct(Endpoint { port: 49152, tls: true }) == 4
        }

        const CT_OK: bool = ct_check();

        fn run() -> bool {
            let p79 = Endpoint { port: 79, tls: false };
            let p80 = Endpoint { port: 80, tls: false };
            let p443 = Endpoint { port: 443, tls: true };
            let p800 = Endpoint { port: 800, tls: false };
            let p1024 = Endpoint { port: 1024, tls: false };
            let p49151 = Endpoint { port: 49151, tls: false };
            let p49152 = Endpoint { port: 49152, tls: true };

            CT_OK
                and shape_to_hi(p79) and not shape_to_hi(p80)
                and shape_to_inc_hi(p80) and not shape_to_inc_hi(p443)
                and not shape_lo_from(p49151) and shape_lo_from(p49152)
                and not shape_lo_hi(p800) and shape_lo_hi(p1024)
                and shape_lo_hi(p49151) and not shape_lo_hi(p49152)
                and shape_lo_inc_hi(p80) and shape_lo_inc_hi(p443)
                and not shape_lo_inc_hi(p79) and not shape_lo_inc_hi(p800)
                and shape_full_wildcard(p79)
                and classify(p79) == 1
                and classify(p80) == 2
                and classify(p443) == 2
                and classify(p800) == 0
                and classify(p1024) == 3
                and classify(p49151) == 3
                and classify(p49152) == 4
        }
    "#;

    verify_interpret_and_native(source);
    assert_eq!(
        support::invoke(source, "ct_check").unwrap(),
        Value::Bool(true)
    );

    let pkg = support::compile(source).unwrap();
    let classify_id = support::source_function(&pkg, "classify");
    let plans = plans_for_fn(&pkg.match_plans, classify_id);
    assert_eq!(plans.len(), 1);
    assert!(plans[0].structural_exact);
    assert!(plans[0].is_exhaustive);
    assert_eq!(plans[0].case_views.len(), 4);
}

#[test]
fn range_pattern_view_takes_priority_over_ord_and_ord_fallback_works() {
    let source = r#"
        use builtin.math.{Eq, Ord, Order};

        -- Implements BOTH Ord and RangePatternView: RangePatternView must win.
        struct TaggedScore {
            score: i32,
            override_band: bool,
        }

        impl Eq for TaggedScore {
            fn eq(self, other: Self) -> bool {
                self.score == other.score and self.override_band == other.override_band
            }
        }

        impl Ord for TaggedScore {
            fn cmp(self, other: Self) -> Order {
                if self.score < other.score {
                    .less
                } else if self.score > other.score {
                    .greater
                } else {
                    .equal
                }
            }
        }

        impl RangePatternView for TaggedScore {
            assoc Bound: Type = i32;

            fn range_pattern_view_contains(
                *self,
                start: RangeEnd<i32>,
                end: RangeEnd<i32>,
            ) -> bool {
                -- Distinct from Ord: override_band forces containment regardless of score!
                if self.override_band.* {
                    return true;
                }
                let s = self.score.*;
                let lo_ok = start match {
                    .unbounded => true,
                    .inclusive(lo) => s >= lo,
                    .exclusive(lo) => s > lo,
                };
                let hi_ok = end match {
                    .unbounded => true,
                    .inclusive(hi) => s <= hi,
                    .exclusive(hi) => s < hi,
                };
                lo_ok and hi_ok
            }
        }

        -- Implements ONLY Ord (no RangePatternView): must fall back to Ord.
        struct OrderedPair {
            hi: i32,
            lo: i32,
        }

        impl Clone for OrderedPair {
            fn clone(*self) -> Self { self.* }
        }

        impl Copy for OrderedPair {}

        impl Eq for OrderedPair {
            fn eq(self, other: Self) -> bool {
                self.hi == other.hi and self.lo == other.lo
            }
        }

        impl Ord for OrderedPair {
            fn cmp(self, other: Self) -> Order {
                if self.hi < other.hi {
                    .less
                } else if self.hi > other.hi {
                    .greater
                } else if self.lo < other.lo {
                    .less
                } else if self.lo > other.lo {
                    .greater
                } else {
                    .equal
                }
            }
        }

        fn check_priority(v: TaggedScore) -> bool {
            v match {
                10..=20 => true,
                _ => false,
            }
        }

        fn check_ord(v: OrderedPair, bound_lo: OrderedPair, bound_hi: OrderedPair) -> i32 {
            v match {
                ..bound_lo => 1,
                bound_lo..bound_hi => 2,
                bound_hi ..= < OrderedPair { hi: 9, lo: 9 } > => 3,
                _ => 4,
            }
        }

        fn run() -> bool {
            let forced = TaggedScore { score: 999, override_band: true };
            let normal_out = TaggedScore { score: 999, override_band: false };
            let normal_in = TaggedScore { score: 15, override_band: false };

            let p0 = OrderedPair { hi: 1, lo: 0 };
            let p1 = OrderedPair { hi: 2, lo: 5 };
            let p2 = OrderedPair { hi: 5, lo: 0 };
            let p3 = OrderedPair { hi: 9, lo: 9 };
            let p4 = OrderedPair { hi: 10, lo: 0 };
            let lo = OrderedPair { hi: 2, lo: 0 };
            let hi = OrderedPair { hi: 5, lo: 0 };

            check_priority(forced)
                and not check_priority(normal_out)
                and check_priority(normal_in)
                and check_ord(p0, lo, hi) == 1
                and check_ord(p1, lo, hi) == 2
                and check_ord(p2, lo, hi) == 3
                and check_ord(p3, lo, hi) == 3
                and check_ord(p4, lo, hi) == 4
        }
    "#;

    verify_interpret_and_native(source);
}

#[test]
fn nan_safety_and_default_self_bound_preserved() {
    let source = r#"
        struct Band {
            value: i32,
        }

        -- Uses default `assoc Bound: Type = Self;`
        impl RangePatternView for Band {
            fn range_pattern_view_contains(
                *self,
                start: RangeEnd<Band>,
                end: RangeEnd<Band>,
            ) -> bool {
                let v = self.value.*;
                let lo_ok = start match {
                    .unbounded => true,
                    .inclusive(lo) => v >= lo.value,
                    .exclusive(lo) => v > lo.value,
                };
                let hi_ok = end match {
                    .unbounded => true,
                    .inclusive(hi) => v <= hi.value,
                    .exclusive(hi) => v < hi.value,
                };
                lo_ok and hi_ok
            }
        }

        fn match_float(x: f64) -> u8 {
            x match {
                ..0.0 => 1,
                0.0..10.0 => 2,
                10.0..=20.0 => 3,
                20.0.. => 4,
                _ => 99,
            }
        }

        fn match_float_full(x: f64) -> bool {
            x match {
                .. => true,
            }
        }

        fn match_band(b: Band) -> bool {
            b match {
                < Band { value: 1 } > ..= < Band { value: 5 } > => true,
                _ => false,
            }
        }

        fn run() -> bool {
            let zero: f64 = 0.0;
            let nan = zero / zero;
            match_float(nan) == 99
                and match_float_full(nan)
                and match_float(-1.5) == 1
                and match_float(0.0) == 2
                and match_float(10.0) == 3
                and match_float(20.0) == 3
                and match_float(25.0) == 4
                and match_band(Band { value: 3 })
                and not match_band(Band { value: 6 })
        }
    "#;

    verify_interpret_and_native(source);
}

#[test]
fn stable_range_pattern_view_dag_dedup_and_stability_rules() {
    let source = r#"
        struct Probe {
            val: u16,
            flag: bool,
        }

        impl Clone for Probe {
            fn clone(*self) -> Self { self.* }
        }

        impl Copy for Probe {}

        impl RangePatternView for Probe {
            assoc Bound: Type = u16;

            fn range_pattern_view_contains(
                *self,
                start: RangeEnd<u16>,
                end: RangeEnd<u16>,
            ) -> bool {
                let v = self.val.*;
                let lo_ok = start match {
                    .unbounded => true,
                    .inclusive(lo) => v >= lo,
                    .exclusive(lo) => v > lo,
                };
                let hi_ok = end match {
                    .unbounded => true,
                    .inclusive(hi) => v <= hi,
                    .exclusive(hi) => v < hi,
                };
                lo_ok and hi_ok
            }
        }

        impl StableRangePatternView for Probe {}

        fn dedup_pair(p: Probe) -> u8 {
            (p, p.flag) match {
                (10..=20, true) => 1,
                (10..=20, false) => 2,
                _ => 0,
            }
        }

        fn duplicate_arm(p: Probe) -> u8 {
            p match {
                10..=20 => 1,
                10..=20 => 2,
                _ => 0,
            }
        }

        fn dynamic_range(p: Probe, lo: u16, hi: u16) -> u8 {
            p match {
                lo..=hi => 1,
                _ => 0,
            }
        }

        fn run() -> bool {
            dedup_pair(Probe { val: 15, flag: false }) == 2
                and dedup_pair(Probe { val: 15, flag: true }) == 1
                and dedup_pair(Probe { val: 25, flag: true }) == 0
                and duplicate_arm(Probe { val: 15, flag: true }) == 1
                and dynamic_range(Probe { val: 15, flag: true }, 10, 20) == 1
                and dynamic_range(Probe { val: 5, flag: true }, 10, 20) == 0
        }
    "#;

    verify_interpret_and_native(source);

    let pkg = support::compile(source).unwrap();
    let dedup_id = support::source_function(&pkg, "dedup_pair");
    let dedup_plans = plans_for_fn(&pkg.match_plans, dedup_id);
    assert_eq!(dedup_plans.len(), 1);
    assert!(dedup_plans[0].structural_exact);
    assert_eq!(
        dedup_plans[0].case_views.len(),
        1,
        "identical static stable range checks must share one CaseViewOps"
    );

    let dup_id = support::source_function(&pkg, "duplicate_arm");
    let dup_plans = plans_for_fn(&pkg.match_plans, dup_id);
    assert_eq!(dup_plans.len(), 1);
    assert!(dup_plans[0].structural_exact);
    assert_eq!(dup_plans[0].unreachable_arms, vec![1]);

    let dyn_id = support::source_function(&pkg, "dynamic_range");
    let dyn_plans = plans_for_fn(&pkg.match_plans, dyn_id);
    assert_eq!(dyn_plans.len(), 1);
    assert!(
        !dyn_plans[0].structural_exact,
        "runtime-dynamic range endpoints must not mark MatchPlan structural_exact"
    );
    assert!(dyn_plans[0].case_views.is_empty());

    // Verify LLVM IR lowers `dedup_pair` with a single call to `range_pattern_view_contains`!
    let native = NativeCase::new(source);
    let command = luna::cli::parse([
        native.0.join("main.fl").into_os_string(),
        "--build".into(),
        "run".into(),
        "--output".into(),
        "program".into(),
        "--fuel".into(),
        "10000000".into(),
        "--sysroot".into(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../library")
            .into_os_string(),
    ])
    .unwrap();
    let luna::cli::Command::Host { options, .. } = command else {
        panic!("expected Host command");
    };
    let (artifact, bindings) =
        luna::frontend::compile_bound_for_native_host(&options, None, None, None, None, None, &[])
            .unwrap();
    let luna::frontend::Artifact::Nir(host_pkg) = artifact else {
        panic!("expected NIR artifact");
    };
    let prepared = luna::host::prepare_with_protocols(
        &host_pkg,
        "run",
        10_000_000,
        bindings.copy.unwrap(),
        bindings.drop.unwrap(),
    )
    .unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 10_000_000).unwrap();
    let dedup_fn_idx = prepared
        .graph()
        .functions
        .iter()
        .position(|f| f.key.declaration.package == 0 && f.function.name == "dedup_pair")
        .expect("dedup_pair in prepared");
    let contains_indices: Vec<usize> = prepared
        .graph()
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| f.function.name == "range_pattern_view_contains")
        .map(|(idx, _)| idx)
        .collect();
    assert_eq!(contains_indices.len(), 1);
    let contains_idx = contains_indices[0];
    let needle = format!("@flurry_{dedup_fn_idx}(");
    let def_start = ir
        .match_indices("define ")
        .find_map(|(pos, _)| {
            let line_end = ir[pos..].find('\n').map_or(ir.len(), |n| pos + n);
            ir[pos..line_end].contains(&needle).then_some(pos)
        })
        .expect("dedup_pair LLVM function");
    let rest = &ir[def_start..];
    let body_end = rest.find("\n}\n").map_or(rest.len(), |n| def_start + n + 3);
    let body = &ir[def_start..body_end];
    let call_needle = format!("@flurry_{contains_idx}(");
    let scalar_needle = format!("@flurry_{contains_idx}_scalar(");
    let total_calls = body.matches(&call_needle).count() + body.matches(&scalar_needle).count();
    assert_eq!(
        total_calls, 1,
        "expected deduplicated single range_pattern_view_contains call in dedup_pair LLVM IR:\n{body}"
    );
}

#[test]
fn unstable_range_view_requires_fallback_and_evaluates_endpoints_in_source_order() {
    let unstable_source = r#"
        struct UnstablePort {
            port: u16,
        }

        impl RangePatternView for UnstablePort {
            assoc Bound: Type = u16;

            fn range_pattern_view_contains(
                *self,
                start: RangeEnd<u16>,
                end: RangeEnd<u16>,
            ) -> bool {
                let p = self.port.*;
                let lo_ok = start match {
                    .unbounded => true,
                    .inclusive(lo) => p >= lo,
                    .exclusive(lo) => p > lo,
                };
                let hi_ok = end match {
                    .unbounded => true,
                    .inclusive(hi) => p <= hi,
                    .exclusive(hi) => p < hi,
                };
                lo_ok and hi_ok
            }
        }

        fn next_lo() -> u16 {
            10
        }

        fn next_hi() -> u16 {
            20
        }

        fn match_order(u: UnstablePort) -> bool {
            u match {
                < next_lo() > ..= < next_hi() > => true,
                _ => false,
            }
        }

        fn match_twice(u: UnstablePort) -> u8 {
            u match {
                10..=20 => 1,
                10..=20 => 2,
                _ => 0,
            }
        }

        -- Non-Copy Once type must borrow via `*self` without consuming the scrutinee.
        struct OnceToken {
            id: u16,
        }

        impl RangePatternView for OnceToken {
            assoc Bound: Type = u16;

            fn range_pattern_view_contains(
                *self,
                start: RangeEnd<u16>,
                end: RangeEnd<u16>,
            ) -> bool {
                let v = self.id.*;
                let lo_ok = start match {
                    .unbounded => true,
                    .inclusive(lo) => v >= lo,
                    .exclusive(lo) => v > lo,
                };
                let hi_ok = end match {
                    .unbounded => true,
                    .inclusive(hi) => v <= hi,
                    .exclusive(hi) => v < hi,
                };
                lo_ok and hi_ok
            }
        }

        fn consume_token(t: OnceToken) -> u16 {
            t.id
        }

        fn match_once_then_consume() -> u16 {
            let t = OnceToken { id: 15 };
            t match {
                30..=40 as x => { let _ = consume_token(x); 0 },
                10..=20 as x => consume_token(x),
                x => { let _ = consume_token(x); 0 },
            }
        }

        fn run() -> bool {
            match_order(UnstablePort { port: 15 })
                and not match_order(UnstablePort { port: 50 })
                and match_twice(UnstablePort { port: 15 }) == 1
                and match_twice(UnstablePort { port: 50 }) == 0
                and match_once_then_consume() == 15
        }
    "#;

    verify_interpret_and_native(unstable_source);

    let pkg = support::compile(unstable_source).unwrap();
    let order_id = support::source_function(&pkg, "match_order");
    let plans = plans_for_fn(&pkg.match_plans, order_id);
    assert_eq!(plans.len(), 1);
    assert!(!plans[0].structural_exact);
    assert!(plans[0].case_views.is_empty());

    // Verify source-order single evaluation: next_lo -> next_hi -> range_pattern_view_contains.
    let graph = luna::instantiation::instantiate(&pkg, "run", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let call_names: Vec<String> = vm
        .events()
        .iter()
        .filter_map(|event| {
            let comptime_interpretor::Event::EnterFunction(id) = event else {
                return None;
            };
            graph
                .functions
                .iter()
                .find(|f| f.key.declaration == *id)
                .map(|f| f.function.name.clone())
        })
        .collect();
    let order_calls: Vec<&str> = call_names
        .iter()
        .map(String::as_str)
        .filter(|name| matches!(*name, "next_lo" | "next_hi" | "range_pattern_view_contains"))
        .collect();
    // run() calls: match_order(15) -> next_lo, next_hi, contains;
    // match_order(50) -> next_lo, next_hi, contains;
    // match_twice(15) -> contains (1st arm hits);
    // match_twice(50) -> contains, contains (unstable: no dedup, both arms tested);
    // match_once_then_consume() -> contains (30..=40 miss), contains (10..=20 hit).
    assert_eq!(
        order_calls,
        vec![
            "next_lo",
            "next_hi",
            "range_pattern_view_contains",
            "next_lo",
            "next_hi",
            "range_pattern_view_contains",
            "range_pattern_view_contains",
            "range_pattern_view_contains",
            "range_pattern_view_contains",
            "range_pattern_view_contains",
            "range_pattern_view_contains",
        ]
    );

    // Omitting `_` on unstable RangePatternView (even with static bounds) must be rejected.
    let missing_fallback_unstable = support::compile(&format!(
        "{unstable_source}
        fn missing_fallback(u: UnstablePort) -> u8 {{
            u match {{
                0..=100 => 1,
                101.. => 2,
            }}
        }}"
    ))
    .unwrap_err();
    assert!(
        missing_fallback_unstable.contains("non-exhaustive match"),
        "{missing_fallback_unstable}"
    );
}

#[test]
fn rejects_bound_type_mismatch_missing_fallback_and_phase_mismatch() {
    let base = r#"
        struct Endpoint {
            port: u16,
            tls: bool,
        }

        impl Clone for Endpoint {
            fn clone(*self) -> Self { self.* }
        }

        impl Copy for Endpoint {}

        impl RangePatternView for Endpoint {
            assoc Bound: Type = u16;

            fn range_pattern_view_contains(
                *self,
                start: RangeEnd<u16>,
                end: RangeEnd<u16>,
            ) -> bool {
                let p = self.port.*;
                let lo_ok = start match {
                    .unbounded => true,
                    .inclusive(lo) => p >= lo,
                    .exclusive(lo) => p > lo,
                };
                let hi_ok = end match {
                    .unbounded => true,
                    .inclusive(hi) => p <= hi,
                    .exclusive(hi) => p < hi,
                };
                lo_ok and hi_ok
            }
        }

        impl StableRangePatternView for Endpoint {}
    "#;

    // 1. Reject str endpoints when Bound = u16
    let bad_bound = support::compile(&format!(
        r#"{base}
        fn bad_range(ep: Endpoint) -> u8 {{
            ep match {{
                "80".."443" => 1,
                _ => 0,
            }}
        }}"#
    ))
    .unwrap_err();
    assert!(
        bad_bound.contains("expected") || bad_bound.contains("type") || bad_bound.contains("u16"),
        "unexpected error for Bound mismatch: {bad_bound}"
    );

    // 2. Reject missing fallback `_` even with StableRangePatternView & constant bounds
    let missing_fallback_stable = support::compile(&format!(
        r#"{base}
        fn missing_fallback(ep: Endpoint) -> u8 {{
            ep match {{
                0..=1023 => 1,
                1024..49152 => 2,
                49152.. => 3,
            }}
        }}"#
    ))
    .unwrap_err();
    assert!(
        missing_fallback_stable.contains("non-exhaustive match"),
        "{missing_fallback_stable}"
    );

    // 3. Reject missing fallback `_` with StableRangePatternView & dynamic bounds
    let missing_fallback_dyn = support::compile(&format!(
        r#"{base}
        fn missing_dyn(ep: Endpoint, lo: u16, hi: u16) -> u8 {{
            ep match {{
                lo..=hi => 1,
            }}
        }}"#
    ))
    .unwrap_err();
    assert!(
        missing_fallback_dyn.contains("non-exhaustive match"),
        "{missing_fallback_dyn}"
    );

    // 4. Reject using runtime-only RangePatternView inside a comptime fn
    let wrong_phase = support::compile(&format!(
        r#"{base}
        comptime fn ct_only(ep: Endpoint) -> u8 {{
            ep match {{
                0..=80 => 1,
                _ => 0,
            }}
        }}"#
    ))
    .unwrap_err();
    assert!(
        wrong_phase.contains("comptime RangePatternView phase"),
        "{wrong_phase}"
    );
}
