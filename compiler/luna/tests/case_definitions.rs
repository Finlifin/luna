//! Frontend lowering and diagnostics for custom pattern view (`case`) definitions.
use luna::cli::{Emit, Options};
use luna::frontend::{self, Artifact};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "luna-case-def-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let result = frontend::compile(&Options {
        dispatch_fuel: None,
        comptime_intrinsics: Vec::new(),
        input: path.clone(),
        emit,
        output: None,
        build_modes: Default::default(),
        build_targets: Default::default(),
        sysroot: None,
        splice_protocol: None,
        meta_list_type: None,
        tuple_type: None,
        index_protocol: None,
        ord_protocol: None,
        eq_protocol: None,
        list_pattern_protocol: None,
        comptime_list_pattern_protocol: None,
        stable_list_pattern_protocol: None,
        list_pattern_endless_protocol: None,
        stable_list_pattern_endless_protocol: None,
        range_end_type: None,
        range_pattern_protocol: None,
        stable_range_pattern_protocol: None,
        concat_pattern_protocol: None,
        stable_concat_pattern_protocol: None,
        map_pattern_protocol: None,
        stable_map_pattern_protocol: None,
        struct_pattern_protocol: None,
        stable_struct_pattern_protocol: None,
        type_pattern_protocol: None,
        iterator_protocol: None,
        into_iterator_protocol: None,
        variadic_collector_protocol: None,
        extended_variadic_collector_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
        incremental: Default::default(),
    });
    std::fs::remove_file(path).unwrap();
    result
}

#[test]
fn case_definitions_lower_to_hir_across_containers_modifiers_and_bodies() {
    let source = r#"
typealias Type = .Type'builtin;
typealias bool = .bool'builtin;
typealias i32 = .i32'builtin;
typealias u32 = .u32'builtin;
typealias str = .str'builtin;
typealias Object = .Object'builtin;
trait Viewable {
    case req(*self) -> ?bool { null }
}
mod views {
    pub case positive(value: i32) -> ?i32 = if value > 0 { value } else { null };
}
struct Student {
    name: str,
    age: u32,
    pub case adult(*self) -> out: ?u32
    where ensures true {
        if self.age >= 18 { self.age } else { null }
    }
    private pure case has_age(self, .min: u32 = 0) -> ?u32 {
        if self.age >= min { self.age } else { null }
    }
}
enum Packet {
    empty,
    value(u32),
    pub inline case non_empty(*self) -> ?u32 {
        self.* match {
            .value(v) => v,
            .empty => null,
        }
    }
}
impl Student {
    comptime case ct_view(comptime flag: bool) -> ?Type {
        if flag { bool } else { null }
    }
    unsafe case raw_view(*self) -> ?u32 { self.age }
}
impl Viewable for Student {
    case req(*self) -> ?bool { self.age >= 18 }
}
comptime fn tag(name: str) -> Object { let meta = { tag: name }; meta }
^tag("student")
case annotated(student: Student) -> ?u32 { student.age }
"#;
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("expected HIR artifact")
    };
    for name in [
        "req",
        "positive",
        "adult",
        "has_age",
        "non_empty",
        "ct_view",
        "raw_view",
        "annotated",
    ] {
        let needle = format!("(name (ident (name \"{name}\")");
        assert!(
            dump.lines()
                .any(|line| line.contains(&needle) && line.contains("(kind (case (signature")),
            "missing case HIR owner `{name}` in:\n{dump}"
        );
    }
    let req_count = dump
        .lines()
        .filter(|line| {
            line.contains("(name (ident (name \"req\")") && line.contains("(kind (case (signature")
        })
        .count();
    assert_eq!(req_count, 2, "expected Viewable + impl req cases:\n{dump}");
    for expected in [
        "(inline true)",
        "(pure true)",
        "(comptime true)",
        "(unsafe true)",
        "(deferred-attribute ",
    ] {
        assert!(dump.contains(expected), "missing `{expected}` in:\n{dump}");
    }
    assert!(
        !dump.contains("unsupported struct body entry"),
        "unexpected unsupported struct body entry pending marker:\n{dump}"
    );
    assert!(
        !dump.contains("pending \"top-level CaseDef\""),
        "CaseDef should lower cleanly to HIR:\n{dump}"
    );
}

#[test]
fn case_definitions_publish_checked_signatures_and_bodies() {
    let source = r#"
typealias bool = .bool'builtin;
typealias i32 = .i32'builtin;
struct Pair where T {
    left: T,
    right: T,
    pub case equal(*self) -> ?T
    where .strict: bool = true {
        null
    }
}
case top(x: i32) -> ?i32 = x;
"#;
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("expected HIR artifact")
    };
    assert!(
        dump.contains("(pending-semantics))"),
        "expected empty pending-semantics for CaseDef HIR:\n{dump}"
    );
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("expected checked NIR")
    };
    assert!(package.functions.iter().any(|f| f.name == "top"));
    let invalid = source.replace("= x;", "= true;");
    assert!(compile(&invalid, Emit::Nir).is_err());
}

#[test]
fn invalid_case_definitions_diagnose_cleanly() {
    for (source, expected) in [
        (
            "quote case bad(x: i32) -> ?i32 { null }",
            "quote modifier is not valid on case declarations",
        ),
        (
            "case dup(x: i32, x: i32) -> ?i32 { null }",
            "duplicate function parameter",
        ),
        (
            "case dup_ret(x: i32) -> x: ?i32 { null }",
            "result name duplicates a function parameter",
        ),
    ] {
        let err = compile(source, Emit::Hir)
            .err()
            .unwrap_or_else(|| panic!("expected HIR diagnostic for `{source}`"));
        assert!(err.contains(expected), "expected `{expected}` in `{err}`");
    }
}

#[path = "support/sysroot_execution.rs"]
mod sysroot;

#[test]
fn case_view_destructures_the_present_tuple_and_skips_absent_subjects() {
    let source = include_str!("../../../development/repros/case-view-option-payload.fl");
    let package = sysroot::compile(source).unwrap();
    let entry = sysroot::source_function(&package, "run");
    let mut reference =
        comptime_interpretor::Interpreter::new(&package, sysroot::COMPTIME_BUDGET).unwrap();
    assert_eq!(
        reference.invoke(entry, vec![]).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let wrong = source.replace(".adult(age, true)", ".adult(age)");
    let error = sysroot::compile(&wrong).unwrap_err();
    assert!(error.contains("case pattern payload arity"), "{error}");
}

#[test]
fn comptime_case_views_bind_nested_patterns_and_allow_a_presence_test() {
    let source = r#"
        struct Entry { number: u32 }
        impl Entry {
            comptime case pair(self) -> ?Tuple<u32, bool> {
                if self.number == 0 { null } else { (self.number, true) }
            }
        }
        comptime fn eligible(value: Entry) -> bool {
            value match {
                .pair(x, true) if x == 5 => true,
                .pair => false,
                _ => value.number == 0,
            }
        }
        const accepted = eligible(Entry { number: 5 }) and eligible(Entry { number: 0 }) and not eligible(Entry { number: 1 });
        fn run() -> bool { accepted }
    "#;
    assert_eq!(
        sysroot::run(source),
        comptime_interpretor::Value::Bool(true)
    );
}

struct NativeCaseDir(std::path::PathBuf);

impl NativeCaseDir {
    fn new(source: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-case-native-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn sysroot() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
    }

    fn build(&self, opt_level: &str) -> std::process::Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
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
            .arg(Self::sysroot())
            .output()
            .unwrap()
    }

    fn interpret(&self) -> std::process::Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "main.fl",
                "--interpret",
                "run",
                "--fuel",
                "10000000",
                "--build-target",
                "x86_64",
                "linux",
                "64",
                "--sysroot",
            ])
            .arg(Self::sysroot())
            .output()
            .unwrap()
    }

    fn run(&self) -> std::process::Output {
        let mut command = std::process::Command::new(self.0.join("program"));
        command.current_dir(&self.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            match command.output() {
                Err(error)
                    if error.raw_os_error() == Some(26) && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(
                        std::time::Duration::from_millis(2)
                            .min(deadline.saturating_duration_since(std::time::Instant::now())),
                    );
                }
                result => return result.unwrap(),
            }
        }
    }
}

impl Drop for NativeCaseDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assert_output_ok(output: &std::process::Output, context: &str) {
    assert!(
        output.status.success(),
        "{context}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn prepare_sysroot_host(source: &str) -> (nir::program::Package, luna::host::Prepared) {
    let dir = NativeCaseDir::new(source);
    let command = luna::cli::parse([
        dir.0.join("main.fl").into_os_string(),
        "--build".into(),
        "run".into(),
        "--output".into(),
        "program".into(),
        "--fuel".into(),
        "10000000".into(),
        "--sysroot".into(),
        NativeCaseDir::sysroot().into_os_string(),
    ])
    .unwrap();
    let luna::cli::Command::Host { options, .. } = command else {
        panic!("expected host command");
    };
    let (artifact, bindings) =
        luna::frontend::compile_bound_for_native_host(&options, None, None, None, None, None, &[])
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
    (package, prepared)
}

#[test]
fn repro_case_view_option_payload_executes_in_interpreter_and_native_llvm() {
    let source = include_str!("../../../development/repros/case-view-option-payload.fl")
        .replace("_ => person.age < 18,", "{ age: age } => age < 18,");
    let dir = NativeCaseDir::new(&source);
    let interp = dir.interpret();
    assert_output_ok(&interp, "interpret");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout).trim(),
        "(bool true)"
    );
    for opt in ["0", "3"] {
        let build = dir.build(opt);
        assert_output_ok(&build, opt);
        let run = dir.run();
        assert_output_ok(&run, opt);
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            "(bool true)",
            "opt={opt}"
        );
    }
}

#[test]
fn owned_case_payloads_transfer_residuals_and_cleanup_failures_end_to_end() {
    let source = r#"
        fn mark_arm() {}
        fn mark_after_partial() {}
        fn mark_fallback_inner() {}
        fn mark_after_inner() {}
        fn mark_fallback_guard() {}
        fn mark_after_guard() {}
        fn mark_fallback_multi_guard() {}
        fn mark_fallback_multi_null() {}
        fn mark_let_after() {}
        fn mark_let_else() {}
        fn mark_iter_0() {}
        fn mark_iter_1() {}
        fn mark_iter_2() {}

        fn drop_10() {}
        fn drop_11() {}
        fn drop_20() {}
        fn drop_21() {}
        fn drop_30() {}
        fn drop_31() {}
        fn drop_40() {}
        fn drop_41() {}
        fn drop_50() {}
        fn drop_51() {}
        fn drop_60() {}
        fn drop_61() {}
        fn drop_70() {}
        fn drop_71() {}
        fn drop_80() {}
        fn drop_81() {}

        struct Token {
            id: u32,
        }

        impl Drop for Token {
            fn drop(*self) {
                let id = self.id.*;
                if id == 10 {
                    drop_10();
                } else if id == 11 {
                    drop_11();
                } else if id == 20 {
                    drop_20();
                } else if id == 21 {
                    drop_21();
                } else if id == 30 {
                    drop_30();
                } else if id == 31 {
                    drop_31();
                } else if id == 40 {
                    drop_40();
                } else if id == 41 {
                    drop_41();
                } else if id == 50 {
                    drop_50();
                } else if id == 51 {
                    drop_51();
                } else if id == 60 {
                    drop_60();
                } else if id == 61 {
                    drop_61();
                } else if id == 70 {
                    drop_70();
                } else if id == 71 {
                    drop_71();
                } else if id == 80 {
                    drop_80();
                } else if id == 81 {
                    drop_81();
                }
            }
        }

        struct Source {
            a: u32,
            b: u32,
        }

        impl Source {
            pub case split(*self) -> ?Tuple<Token, Token> {
                if self.a.* == 0 {
                    null
                } else {
                    (Token { id: self.a.* }, Token { id: self.b.* })
                }
            }

            pub case single(*self) -> ?Tuple<Token, bool> {
                if self.a.* == 0 {
                    null
                } else {
                    (Token { id: self.a.* }, true)
                }
            }
        }

        fn test_partial_move_and_residual() -> bool {
            let s = Source { a: 10, b: 11 };
            let moved = s match {
                .split(t, _) => {
                    mark_arm();
                    t.id == 10
                },
                _ => false,
            };
            mark_after_partial();
            moved
        }

        fn test_inner_mismatch_failure() -> bool {
            let s = Source { a: 20, b: 21 };
            let res = s match {
                .split(_, { id: 999 }) => false,
                _ => {
                    mark_fallback_inner();
                    true
                }
            };
            mark_after_inner();
            res
        }

        fn test_guard_failure() -> bool {
            let s = Source { a: 30, b: 31 };
            let res = s match {
                .split(t1, t2) if t1.id > 100 and t2.id > 100 => false,
                _ => {
                    mark_fallback_guard();
                    true
                }
            };
            mark_after_guard();
            res
        }

        fn test_multi_case_lifo_failure() -> bool {
            let p1a = Source { a: 40, b: 0 };
            let p1b = Source { a: 40, b: 0 };
            let p2 = Source { a: 41, b: 0 };
            let p3 = Source { a: 0, b: 0 };
            let first = (p1a, p2) match {
                (.single(t1, true), .single(t2, true)) if t1.id > 500 and t2.id > 500 => false,
                _ => {
                    mark_fallback_multi_guard();
                    true
                },
            };
            let second = (p1b, p3) match {
                (.single(t1, true), .single(t2, true)) => t1.id == 0 and t2.id == 0,
                _ => {
                    mark_fallback_multi_null();
                    true
                },
            };
            first and second
        }

        fn test_let_else_success_and_failure(trigger_fail: bool) -> bool {
            let s = Source { a: 50, b: 51 };
            if trigger_fail {
                let .split(t, _) if t.id > 999 = Source { a: 60, b: 61 } else {
                    mark_let_else();
                    return true;
                };
                false
            } else {
                let .split(t, _) = s else {
                    return false;
                };
                mark_let_after();
                t.id == 50
            }
        }

        fn test_loop_iterations() -> bool {
            var i: u32 = 0;
            var ok = true;
            while i < 3 {
                let s = if i == 0 {
                    Source { a: 70, b: 71 }
                } else if i == 1 {
                    Source { a: 80, b: 81 }
                } else {
                    Source { a: 0, b: 82 }
                };
                let step = s match {
                    .split(t, _) if t.id == 70 => t.id == 70,
                    _ => true,
                };
                ok = ok and step;
                if i == 0 {
                    mark_iter_0();
                } else if i == 1 {
                    mark_iter_1();
                } else {
                    mark_iter_2();
                }
                i = i + 1;
            }
            ok
        }

        fn run() -> bool {
            test_partial_move_and_residual()
                and test_inner_mismatch_failure()
                and test_guard_failure()
                and test_multi_case_lifo_failure()
                and test_let_else_success_and_failure(false)
                and test_let_else_success_and_failure(true)
                and test_loop_iterations()
        }
    "#;

    let expected_events = [
        // test_partial_move_and_residual: t (10) drops at arm ScopeEnd; unmoved residual (11) drops at Match TemporaryEnd before mark_after_partial
        "mark_arm",
        "drop_10",
        "drop_11",
        "mark_after_partial",
        // test_inner_mismatch_failure: PatternFailure drops full ?Tuple(20, 21) before fallback runs
        "drop_21",
        "drop_20",
        "mark_fallback_inner",
        "mark_after_inner",
        // test_guard_failure: PatternFailure drops full ?Tuple(30, 31) before fallback runs
        "drop_31",
        "drop_30",
        "mark_fallback_guard",
        "mark_after_guard",
        // test_multi_case_lifo_failure: second Pattern::Then (41) drops before first (40) on guard fail
        "drop_41",
        "drop_40",
        "mark_fallback_multi_guard",
        // second tuple arm: p1 produces 40, p3 returns null -> 40 drops before fallback
        "drop_40",
        "mark_fallback_multi_null",
        // test_let_else_success: residual (51) drops at let TemporaryEnd before mark_let_after; moved binding (50) drops at function end
        "drop_51",
        "mark_let_after",
        "drop_50",
        // test_let_else_failure: PatternFailure drops (61, 60) before else body runs mark_let_else
        "drop_61",
        "drop_60",
        "mark_let_else",
        // test_loop_iterations: iter 0 moves 70 (drops in arm), drops 71 at TemporaryEnd; iter 1 guard fails dropping (81, 80); iter 2 returns null
        "drop_70",
        "drop_71",
        "mark_iter_0",
        "drop_81",
        "drop_80",
        "mark_iter_1",
        "mark_iter_2",
    ];

    let (package, prepared) = prepare_sysroot_host(source);
    let schedule_dump = prepared.to_string();
    assert!(
        schedule_dump.contains("pattern-failure"),
        "expected PatternFailure cleanup boundary in prepared host schedule:\n{schedule_dump}"
    );

    let mut vm = comptime_interpretor::Interpreter::from_iir(
        prepared.graph(),
        comptime_interpretor::Budget {
            steps: 10_000_000,
            depth: 128,
        },
    )
    .unwrap()
    .with_cleanup_schedule(prepared.schedule())
    .unwrap();
    assert_eq!(
        vm.invoke_instance(&prepared.graph().roots[0], vec![])
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let observed_events: Vec<&str> = vm
        .events()
        .iter()
        .filter_map(|event| match event {
            comptime_interpretor::Event::EnterFunction(id) => {
                let name = package
                    .functions
                    .iter()
                    .find(|f| f.declaration == *id)
                    .map(|f| f.name.as_str())?;
                if name.starts_with("drop_") || name.starts_with("mark_") {
                    Some(name)
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        observed_events, expected_events,
        "unexpected IIR cleanup event trace"
    );

    let ir = luna::host::llvm_ir_prepared(&prepared, 10_000_000).unwrap();
    let drop_indices: Vec<usize> = prepared
        .graph()
        .functions
        .iter()
        .enumerate()
        .filter(|(_, function)| {
            function.key.declaration.package == 0 && function.function.name == "drop"
        })
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        drop_indices.len(),
        1,
        "expected unique user Token::drop in IIR"
    );
    let drop_idx = drop_indices[0];
    let drop_calls = ir
        .matches(&format!("call {{ {{}}, ptr }} @flurry_{drop_idx}(ptr "))
        .count()
        + ir.matches(&format!("call void @flurry_{drop_idx}_scalar(ptr "))
            .count();
    assert!(
        drop_calls >= 6,
        "expected Token::drop (@flurry_{drop_idx}) calls across native failure/residual sites, found {drop_calls}:\n{ir}"
    );

    let dir = NativeCaseDir::new(source);
    let interp = dir.interpret();
    assert_output_ok(&interp, "interpret");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout).trim(),
        "(bool true)"
    );
    for opt in ["0", "3"] {
        let build = dir.build(opt);
        assert_output_ok(&build, opt);
        let run = dir.run();
        assert_output_ok(&run, opt);
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            "(bool true)",
            "opt={opt}"
        );
    }
}

#[test]
fn owned_case_payloads_interpret_and_preserve_pointer_and_mutation_rejections() {
    let pure_source = r#"
        struct Pair {
            a: u32,
            b: u32,
        }
        struct Held {
            value: u32,
        }
        impl Drop for Held {
            fn drop(*self) {}
        }
        impl Pair {
            pub case split(*self) -> ?Tuple<Held, Held> {
                if self.a.* == 0 {
                    null
                } else {
                    (Held { value: self.a.* }, Held { value: self.b.* })
                }
            }
        }
        fn run() -> bool {
            let p1 = Pair { a: 10, b: 20 };
            let r1 = p1 match {
                .split(x, _) if x.value == 10 => true,
                _ => false,
            };
            let r2 = Pair { a: 5, b: 6 } match {
                .split(x, _) if x.value > 100 => false,
                _ => true,
            };
            let let_ok = do {
                let .split(h, _) = Pair { a: 7, b: 8 } else { return false; };
                h.value == 7
            };
            r1 and r2 and let_ok
        }
    "#;
    let dir = NativeCaseDir::new(pure_source);
    let interp = dir.interpret();
    assert_output_ok(&interp, "interpret");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout).trim(),
        "(bool true)"
    );

    let pointer_return = r#"
        struct Holder {
            x: u32,
            pub case bad(*self) -> ?Tuple<*u32, bool> {
                (self.x, true)
            }
        }
        fn run() -> bool {
            let h = Holder { x: 1 };
            h match {
                .bad(p, true) => p.* == 1,
                _ => false,
            }
        }
    "#;
    let ptr_dir = NativeCaseDir::new(pointer_return);
    assert!(
        !ptr_dir.build("0").status.success(),
        "pointer-bearing computed case return must be rejected"
    );

    let mutating_case = r#"
        struct Holder {
            x: u32,
            pub case mut_bad(*mut self) -> ?Tuple<u32, bool> {
                self.x.* = 2;
                (self.x.*, true)
            }
        }
        fn run() -> bool {
            var h = Holder { x: 1 };
            h match {
                .mut_bad(v, true) => v == 2,
                _ => false,
            }
        }
    "#;
    let mut_dir = NativeCaseDir::new(mutating_case);
    assert!(
        !mut_dir.build("0").status.success(),
        "mutating case view must be rejected"
    );

    let rejecting_drop = pure_source.replace(
        "fn drop(*self) {}",
        "fn drop(*self) { assumes self.value.* != 0; }",
    );
    let reject_dir = NativeCaseDir::new(&rejecting_drop);
    let reject_build = reject_dir.build("0");
    assert!(
        !reject_build.status.success(),
        "may-reject Drop on owned case payload must be rejected"
    );
    let reject_stderr = String::from_utf8_lossy(&reject_build.stderr);
    assert!(
        reject_stderr.contains("observation cleanup needs failure-path resource transfer"),
        "expected observation cleanup rejection, got:\n{reject_stderr}"
    );
}
