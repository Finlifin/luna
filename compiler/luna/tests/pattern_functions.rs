#[path = "support/sysroot_execution.rs"]
mod sysroot_execution;

use comptime_interpretor::Value;
use luna::{
    cli,
    frontend::{self, Artifact},
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TempCaseDir(PathBuf);

impl TempCaseDir {
    fn new(files: &[(&str, &str)]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "luna-pattern-fn-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for (rel, content) in files {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }
        Self(dir)
    }

    fn sysroot() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
    }
}

impl Drop for TempCaseDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn compile_dir(files: &[(&str, &str)], entry_rel: &str) -> Result<nir::program::Package, String> {
    let dir = TempCaseDir::new(files);
    let cli::Command::Check(mut options) = cli::parse([
        "check".into(),
        dir.0.join(entry_rel).into_os_string(),
        "--sysroot".into(),
        TempCaseDir::sysroot().into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("expected Check command");
    };
    options.emit = cli::Emit::Nir;
    match frontend::compile(&options) {
        Ok(Artifact::Nir(package)) => Ok(package),
        Ok(_) => panic!("expected NIR artifact"),
        Err(error) => Err(error.to_string()),
    }
}

fn prepare_host(source: &str) -> (nir::program::Package, luna::host::Prepared, String) {
    let dir = TempCaseDir::new(&[("main.fl", source)]);
    let command = cli::parse([
        dir.0.join("main.fl").into_os_string(),
        "--build".into(),
        "run".into(),
        "--output".into(),
        "program".into(),
        "--fuel".into(),
        "10000000".into(),
        "--sysroot".into(),
        TempCaseDir::sysroot().into_os_string(),
    ])
    .unwrap();
    let cli::Command::Host { options, .. } = command else {
        panic!("expected Host command");
    };
    let (artifact, bindings) =
        frontend::compile_bound_for_native_host(&options, None, None, None, None, None, &[])
            .unwrap();
    let Artifact::Nir(package) = artifact else {
        panic!("expected NIR artifact");
    };
    let prepared = luna::host::prepare_with_protocols(
        &package,
        "run",
        10_000_000,
        bindings.copy.unwrap(),
        bindings.drop.unwrap(),
    )
    .unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 10_000_000).unwrap();
    (package, prepared, ir)
}

fn llvm_function_body<'a>(ir: &'a str, fn_index: usize) -> &'a str {
    let needle = format!("@flurry_{fn_index}(");
    let def_start = ir
        .match_indices("define ")
        .find_map(|(pos, _)| {
            let line_end = ir[pos..].find('\n').map_or(ir.len(), |n| pos + n);
            ir[pos..line_end].contains(&needle).then_some(pos)
        })
        .unwrap_or_else(|| panic!("missing LLVM function @flurry_{fn_index} in:\n{ir}"));
    let rest = &ir[def_start..];
    let body_end = rest.find("\n}\n").map_or(rest.len(), |n| def_start + n + 3);
    &ir[def_start..body_end]
}

fn iir_fn_indices(prepared: &luna::host::Prepared, package: u64, name: &str) -> Vec<usize> {
    prepared
        .graph()
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| f.key.declaration.package == package && f.function.name == name)
        .map(|(idx, _)| idx)
        .collect()
}

fn count_flurry_calls(body: &str, fn_index: usize) -> usize {
    let direct = format!("@flurry_{fn_index}(");
    let scalar = format!("@flurry_{fn_index}_scalar(");
    body.matches(&direct).count() + body.matches(&scalar).count()
}

#[test]
fn predicate_bool_and_unit_optional_case_views() {
    let src = r#"
pub struct Num { value: i64 }

pub pure fn void_val() -> void {}

impl Num {
    ^attrs.pattern.stable
    pub case is_even(*self) -> bool {
        self.value.* == 2 or self.value.* == 4 or self.value.* == -4
    }

    ^attrs.pattern.stable
    pub case is_zero(*self) -> ?void {
        if self.value.* == 0 { void_val() } else { null }
    }

    ^attrs.pattern.stable
    pub case is_one(*self) -> ?Tuple<> {
        let empty: Tuple<> = ();
        if self.value.* == 1 { empty } else { null }
    }
}

pub fn classify(n: Num) -> i64 {
    n match {
        .is_zero() => 100,
        .is_one() => 101,
        .is_even => 2,
        .is_even() => 999,
        _ => -1,
    }
}

pub fn run() -> i64 {
    let a = classify(Num { value: 0 });
    let b = classify(Num { value: 1 });
    let c = classify(Num { value: 4 });
    let d = classify(Num { value: 5 });
    if a >= -1000 and a <= 1000 and b >= -1000 and b <= 1000 and c >= -1000 and c <= 1000 and d >= -1000 and d <= 1000 {
        a * 1000 + b * 100 + c * 10 + d
    } else {
        0
    }
}
"#;
    assert_eq!(
        sysroot_execution::run(src),
        Value::SignedInteger(100 * 1000 + 101 * 100 + 2 * 10 - 1)
    );

    let bad_binding = r#"
pub struct Num { value: i64 }

impl Num {
    pub pure case is_even(*self) -> bool {
        self.value.* == 2 or self.value.* == 4
    }
}

pub fn run(n: Num) -> i64 {
    n match {
        .is_even(x) => 1,
        _ => 0,
    }
}
"#;
    let err = sysroot_execution::compile(bad_binding).unwrap_err();
    assert!(
        err.contains("predicate case pattern takes no payload bindings"),
        "unexpected error: {err}"
    );
}

#[test]
fn case_views_with_extra_arguments_and_free_case_views() {
    let src = r#"
pub struct Num { value: i64 }

impl Num {
    ^attrs.pattern.stable
    pub case in_range(*self, lo: i64, hi: i64) -> ?Tuple<i64> {
        if self.value.* >= lo and self.value.* <= hi { (self.value.*,) } else { null }
    }

    ^attrs.pattern.stable
    pub case between(*self, lo: i64, hi: i64) -> bool {
        self.value.* >= lo and self.value.* <= hi
    }
}

pub pure case free_even(n: i64) -> bool {
    n == 4 or n == 8
}

pub pure case split_shifted(p: Tuple<i64, i64>, delta: i64) -> ?Tuple<i64, i64> {
    let (x, y) = p;
    if x > 0 and x <= 1000 and y >= -1000 and y <= 1000 and delta >= -1000 and delta <= 1000 {
        (x + delta, y + delta)
    } else {
        null
    }
}

pub mod views {
    pub pure case neg_val(x: i64) -> ?Tuple<i64> {
        if x < 0 and x >= -1000 { (0 - x,) } else { null }
    }
}

pub fn run() -> i64 {
    let n = Num { value: 7 };
    let r1 = n match {
        .in_range(0, 5, v) => v,
        .in_range(6, 10, v) => if v >= -1000 and v <= 1000 { v * 10 } else { 0 },
        _ => 0,
    };
    let r2: i64 = Num { value: 15 } match {
        .between(10, 20) => 3,
        _ => 0,
    };
    let n8: i64 = 8;
    let r3: i64 = n8 match {
        free_even() => 4,
        _ => 0,
    };
    let pair: Tuple<i64, i64> = (2, 5);
    let r4 = pair match {
        split_shifted(10, a, b) => if a >= -1000 and a <= 1000 and b >= -1000 and b <= 1000 { a + b } else { 0 },
        _ => 0,
    };
    let neg9: i64 = -9;
    let r5 = neg9 match {
        views.neg_val(v) => v,
        _ => 0,
    };
    if r1 >= -10000 and r1 <= 10000 and r2 >= -10000 and r2 <= 10000 and r3 >= -10000 and r3 <= 10000 and r4 >= -10000 and r4 <= 10000 and r5 >= -10000 and r5 <= 10000 {
        r1 + r2 + r3 + r4 + r5
    } else {
        0
    }
}
"#;
    assert_eq!(
        sysroot_execution::run(src),
        Value::SignedInteger(70 + 3 + 4 + (12 + 15) + 9)
    );
}

#[test]
fn pure_and_stable_case_views_dedup_into_llvm_dag_while_impure_runs_sequentially() {
    let dag_src = r#"
pub pure case split_pos(n: i64) -> ?Tuple<i64> {
    if n > 0 { (n,) } else { null }
}

^attrs.pattern.stable
pub case stable_even(n: i64) -> bool {
    n < 0 and n >= -1000 and (n + 4) == 0
}

pub fn inspect(x: i64) -> i64 {
    x match {
        split_pos(1) => 10,
        split_pos(2) => 20,
        split_pos(k) => if k >= -10000 and k <= 10000 { k + 100 } else { 0 },
        stable_even() => 50,
        _ => -1,
    }
}

pub fn run() -> i64 {
    let s1 = inspect(1);
    let s2 = inspect(2);
    let s3 = inspect(5);
    let s4 = inspect(-4);
    let s5 = inspect(-3);
    if s1 >= -1000 and s1 <= 1000 and s2 >= -1000 and s2 <= 1000 and s3 >= -1000 and s3 <= 1000 and s4 >= -1000 and s4 <= 1000 and s5 >= -1000 and s5 <= 1000 {
        s1 + s2 + s3 + s4 + s5
    } else {
        0
    }
}
"#;
    let (package, prepared, ir) = prepare_host(dag_src);
    let inspect_decl = sysroot_execution::source_function(&package, "inspect");
    let plan = package
        .match_plans
        .plans
        .iter()
        .find(|(k, _)| k.function == Some(inspect_decl))
        .map(|(_, p)| p)
        .expect("inspect match plan");
    assert_eq!(
        plan.case_views.len(),
        2,
        "expected split_pos deduplicated + stable_even: {:?}",
        plan.case_views
    );
    let inspect_indices = iir_fn_indices(&prepared, 0, "inspect");
    let split_pos_indices = iir_fn_indices(&prepared, 0, "split_pos");
    let stable_even_indices = iir_fn_indices(&prepared, 0, "stable_even");
    assert_eq!(inspect_indices.len(), 1);
    assert_eq!(split_pos_indices.len(), 1);
    assert_eq!(stable_even_indices.len(), 1);
    let dag_dump = plan.dag.to_string();
    let case_switch_nodes = dag_dump.matches("(case (id ").count();
    assert_eq!(
        case_switch_nodes, 2,
        "expected all 3 split_pos arms to share 1 SwitchKind::Case plus 1 for stable_even: {dag_dump}"
    );
    let body = llvm_function_body(&ir, inspect_indices[0]);
    assert_eq!(
        body.matches("alloca { i64 }").count(),
        1,
        "pure split_pos must allocate a single CaseCacheSlot payload in DAG LLVM IR:\n{body}"
    );
    assert_eq!(
        count_flurry_calls(body, stable_even_indices[0]),
        1,
        "attrs.pattern.stable case view must be emitted once in DAG LLVM IR:\n{body}"
    );
    assert_eq!(
        sysroot_execution::run(dag_src),
        Value::SignedInteger(10 + 20 + 105 + 50 - 1)
    );

    // Non-pure case view without `^attrs.pattern.stable` must NOT be entered into DAG `case_views`.
    let seq_src = r#"
pub case check_tag(x: i64) -> ?Tuple<i64> {
    if x > 0 { (x,) } else { null }
}

pub fn seq_match(x: i64) -> i64 {
    x match {
        check_tag(1) => 11,
        check_tag(2) => 22,
        _ => 33,
    }
}

pub fn run() -> i64 {
    let a = seq_match(1);
    let b = seq_match(2);
    let c = seq_match(0);
    if a >= 0 and a <= 100 and b >= 0 and b <= 100 and c >= 0 and c <= 100 {
        a + b + c
    } else {
        0
    }
}
"#;
    let seq_pkg = sysroot_execution::compile(seq_src).unwrap();
    let seq_decl = sysroot_execution::source_function(&seq_pkg, "seq_match");
    let seq_plan = seq_pkg
        .match_plans
        .plans
        .iter()
        .find(|(k, _)| k.function == Some(seq_decl))
        .map(|(_, p)| p);
    assert!(
        seq_plan.is_none_or(|p| p.case_views.is_empty()),
        "impure case view must stay sequential, not enter case_views"
    );
    assert_eq!(
        sysroot_execution::run(seq_src),
        Value::SignedInteger(11 + 22 + 33)
    );
}

#[test]
fn package_wide_open_pattern_functions_order_independent_multi_module_and_recursive() {
    let dir_files = &[
        (
            "lib.fl",
            r#"
pub enum Expr {
    lit(i64),
    neg(i64),
    add(i64, i64),
}

-- Arms can appear before and after the base declaration across modules.
pub fn eval(case .lit(n)) -> i64 { n }

pub fn eval(e: Expr) -> i64;

pub fn eval_sum() -> i64 {
    let a = eval(.lit(5));
    let b = eval(.neg(3));
    let c = eval(.add(10, 20));
    if a >= -1000 and a <= 1000 and b >= -1000 and b <= 1000 and c >= -1000 and c <= 1000 {
        a + b + c
    } else {
        0
    }
}
"#,
        ),
        (
            "extra.fl",
            r#"
use .Expr;
use .eval;

pub fn eval(case .neg(n)) -> i64 {
    if n >= -1000 and n <= 1000 { 0 - n } else { 0 }
}
pub fn eval(case .add(a, b)) -> i64 {
    let lhs = eval(.lit(a));
    let rhs = eval(.lit(b));
    if lhs >= -1000 and lhs <= 1000 and rhs >= -1000 and rhs <= 1000 {
        lhs + rhs
    } else {
        0
    }
}
"#,
        ),
    ];
    let pkg = compile_dir(dir_files, "lib.fl").unwrap();
    let graph = luna::instantiation::instantiate(&pkg, "eval_sum", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::SignedInteger(5 - 3 + 30)
    );

    // Recursive integer pattern function + LLVM native codegen
    let fib_src = r#"
pub fn fib(n: i64) -> i64;
pub fn fib(case 0) -> i64 { 0 }
pub fn fib(case 1) -> i64 { 1 }
pub fn fib(case ..0) -> i64 { 0 }
pub fn fib(case 2.. as n) -> i64 {
    if n >= 2 and n <= 30 {
        let a = fib(n - 1);
        let b = fib(n - 2);
        if a >= 0 and a <= 100000 and b >= 0 and b <= 100000 { a + b } else { 0 }
    } else {
        0
    }
}

pub fn run() -> i64 {
    fib(8)
}
"#;
    assert_eq!(sysroot_execution::run(fib_src), Value::SignedInteger(21));
    let (_, prepared, ir) = prepare_host(fib_src);
    let fib_idx = iir_fn_indices(&prepared, 0, "fib")[0];
    let fib_ir = llvm_function_body(&ir, fib_idx);
    assert!(fib_ir.contains("define "), "{fib_ir}");
}

#[test]
fn pattern_function_multi_param_generic_and_comptime() {
    let src = r#"
pub enum Token { num(i64), plus, minus }

pub fn combine(case .num(a), case .num(b), scale: i64) -> i64 {
    if a >= -1000 and a <= 1000 and b >= -1000 and b <= 1000 and scale >= -1000 and scale <= 1000 {
        (a + b) * scale
    } else {
        0
    }
}
pub fn combine(case .plus, case _, scale: i64) -> i64 { scale }
pub fn combine(case .minus, case _, scale: i64) -> i64 {
    if scale >= -1000 and scale <= 1000 { 0 - scale } else { 0 }
}
pub fn combine(case .num(_), case .plus, scale: i64) -> i64 {
    if scale >= -1000 and scale <= 1000 { scale * 2 } else { 0 }
}
pub fn combine(case .num(_), case .minus, scale: i64) -> i64 {
    if scale >= -1000 and scale <= 1000 {
        let neg = 0 - scale;
        if neg >= -1000 and neg <= 1000 { neg * 2 } else { 0 }
    } else {
        0
    }
}
pub fn combine(a: Token, b: Token, scale: i64) -> i64;

pub fn unwrap_or(opt: ?T, fallback: T) -> T where T;
pub fn unwrap_or(case null, fallback: T) -> T where T { fallback }
pub fn unwrap_or(case val?, fallback: T) -> T where T { val }

pub comptime fn ct_step(n: i64) -> i64;
pub comptime fn ct_step(case 0) -> i64 { 1 }
pub comptime fn ct_step(case ..0 as n) -> i64 {
    if n >= -1000 and n < 0 { 0 - n } else { 0 }
}
pub comptime fn ct_step(case 1.. as n) -> i64 {
    if n >= 1 and n <= 1000 { n * 10 } else { 0 }
}

pub comptime fn eval_ct() -> i64 {
    let c0 = ct_step(0);
    let c1 = ct_step(-4);
    let c2 = ct_step(3);
    if c0 >= -10000 and c0 <= 10000 and c1 >= -10000 and c1 <= 10000 and c2 >= -10000 and c2 <= 10000 {
        c0 + c1 + c2
    } else {
        0
    }
}

const CT_VAL: i64 = eval_ct();

pub fn run() -> i64 {
    let x = combine(.num(3), .num(4), 10);
    let u1: i64 = unwrap_or(null, 9);
    let u2: i64 = unwrap_or(42, 9);
    let ct: i64 = CT_VAL;
    if x >= -10000 and x <= 10000 and u1 >= -10000 and u1 <= 10000 and u2 >= -10000 and u2 <= 10000 and ct >= -10000 and ct <= 10000 {
        let s1 = x + u1;
        let s2 = u2 + ct;
        if s1 >= -20000 and s1 <= 20000 and s2 >= -20000 and s2 <= 20000 {
            s1 + s2
        } else {
            0
        }
    } else {
        0
    }
}
"#;
    assert_eq!(
        sysroot_execution::run(src),
        Value::SignedInteger(70 + (9 + 42) + (1 + 4 + 30))
    );
}

#[test]
fn pattern_function_rejects_overlapping_arms_and_non_exhaustive_sets() {
    let overlap_ranges = r#"
pub fn f(n: i64) -> i64;
pub fn f(case 0..=10) -> i64 { 1 }
pub fn f(case 10..=20) -> i64 { 2 }
pub fn f(case _) -> i64 { 3 }
"#;
    let err1 = sysroot_execution::compile(overlap_ranges).unwrap_err();
    assert!(
        err1.contains("overlapping case function arms across order-independent package collection"),
        "expected strict overlap error, got: {err1}"
    );

    let overlap_multi_arg = r#"
pub enum Pair { a(i64), b }
pub fn g(x: Pair, y: Pair) -> i64;
pub fn g(case .a(_), case _) -> i64 { 1 }
pub fn g(case _, case .b) -> i64 { 2 }
"#;
    let err2 = sysroot_execution::compile(overlap_multi_arg).unwrap_err();
    assert!(
        err2.contains("overlapping case function arms across order-independent package collection"),
        "expected multi-arg overlap error, got: {err2}"
    );

    let overlap_distinct_cases = r#"
pub pure case p1(x: i64) -> bool { x > 0 }
pub pure case p2(x: i64) -> bool { x < 10 }
pub fn h(x: i64) -> i64;
pub fn h(case p1()) -> i64 { 1 }
pub fn h(case p2()) -> i64 { 2 }
pub fn h(case _) -> i64 { 3 }
"#;
    let err3 = sysroot_execution::compile(overlap_distinct_cases).unwrap_err();
    assert!(
        err3.contains("overlapping case function arms across order-independent package collection"),
        "expected distinct case overlap error, got: {err3}"
    );

    let non_exhaustive = r#"
pub enum E { a, b, c }
pub fn k(e: E) -> i64;
pub fn k(case .a) -> i64 { 1 }
pub fn k(case .b) -> i64 { 2 }
"#;
    let err4 = sysroot_execution::compile(non_exhaustive).unwrap_err();
    assert!(
        err4.contains("non-exhaustive"),
        "expected exhaustiveness error, got: {err4}"
    );
}
