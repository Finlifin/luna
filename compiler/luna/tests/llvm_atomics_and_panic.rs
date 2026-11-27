//! End-to-end integration tests for atomic memory intrinsics, fences,
//! runtime panic unwind (`{ T, ptr = 1 }`, reverse `Drop`, double-panic trap,
//! C-ABI boundary trap), and immediate `runtime_abort` under `-O0` and `-O3`.

use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-atomics-panic-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn sysroot() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
    }

    fn emit_llvm(&self, entry: &str) -> Result<String, String> {
        let mut cmd = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"));
        cmd.current_dir(&self.0)
            .args([
                "dump",
                "main.fl",
                "--entry",
                entry,
                "--emit",
                "llvm",
                "--opt-level",
                "0",
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(Self::sysroot());
        let output = cmd.output().unwrap();
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    }

    fn build(&self, entry: &str, opt_level: &str) -> Output {
        let mut cmd = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"));
        cmd.current_dir(&self.0)
            .args([
                "build",
                "main.fl",
                "--entry",
                entry,
                "--output",
                "program",
                "--opt-level",
                opt_level,
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(Self::sysroot());
        cmd.output().unwrap()
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

const INTRINSIC_PREAMBLE: &str = r#"
use std.prelude.*;
use std.os.linux;

fn addr_of(p: *T) -> usize where T {
    unsafe { builtin.mem.ptr_addr(p) }
}

fn trigger_panic(msg: str) -> NoReturn {
    builtin.control.runtime_panic(msg)
}

fn trigger_abort() -> NoReturn {
    builtin.control.runtime_abort()
}
"#;

#[test]
fn atomic_operations_and_fences_execute_natively_at_o0_and_o3() {
    let source = format!(
        r#"{INTRINSIC_PREAMBLE}
        fn dynamic_order(n: u8) -> u8 {{ n }}

        fn touch_atomic(p: *u64, v: u64) {{
            let seq: u8 = 4;
            unsafe {{ builtin.atomics.atomic_store(p, v, seq); }}
        }}

        fn run() -> bool {{
            var cell: u64 = 0;
            var exp: u64 = 0;
            var signed_cell: i32 = 0;
            var unsigned_cell: u32 = 0;
            var flag: bool = false;
            var flag_exp: bool = false;
            var target_a: u64 = 111;
            var target_b: u64 = 222;
            var ptr_cell: *u64 = target_a.ref;
            var ptr_exp: *u64 = target_a.ref;

            let relaxed: u8 = 0;
            let acquire: u8 = 1;
            let release: u8 = 2;
            let seq_cst: u8 = 4;
            let dyn_seq: u8 = dynamic_order(4);

            let v10: u64 = 10;
            let v20: u64 = 20;
            let v5: u64 = 5;
            let v3: u64 = 3;
            let v100: u64 = 0x100;
            let v101: u64 = 0x101;
            let v0f: u64 = 0x0F;
            let v77: u64 = 77;
            let v55: u64 = 55;
            let s_neg10: i32 = -10;
            let s_3: i32 = 3;
            let s_neg20: i32 = -20;
            let u_big: u32 = 0xFFFFFFF0;
            let u_5: u32 = 5;
            let u_7: u32 = 7;

            unsafe {{
                touch_atomic(cell.ref, v10);
                let l1 = builtin.atomics.atomic_load(cell.ref, acquire);
                let sw = builtin.atomics.atomic_swap(cell.ref, v20, dyn_seq);
                let fa = builtin.atomics.atomic_fetch_add(cell.ref, v5, seq_cst);
                let fs = builtin.atomics.atomic_fetch_sub(cell.ref, v3, release);
                let fo = builtin.atomics.atomic_fetch_or(cell.ref, v100, seq_cst);
                let fx = builtin.atomics.atomic_fetch_xor(cell.ref, v101, seq_cst);
                let fn_and = builtin.atomics.atomic_fetch_and(cell.ref, v0f, seq_cst);
                let cur = builtin.atomics.atomic_load(cell.ref, dyn_seq);

                exp = 999;
                let cas_miss = builtin.atomics.atomic_cmpxchg(cell.ref, exp.ref, v77, seq_cst, acquire);
                let miss_exp = exp;
                let cas_hit = builtin.atomics.atomic_cmpxchg_weak(cell.ref, exp.ref, v55, dyn_seq, relaxed);
                let after_cas = builtin.atomics.atomic_load(cell.ref, acquire);

                builtin.atomics.atomic_store(signed_cell.ref, s_neg10, release);
                let smax_prev = builtin.atomics.atomic_fetch_max(signed_cell.ref, s_3, seq_cst);
                let smax_cur = builtin.atomics.atomic_load(signed_cell.ref, acquire);
                let smin_prev = builtin.atomics.atomic_fetch_min(signed_cell.ref, s_neg20, seq_cst);
                let smin_cur = builtin.atomics.atomic_load(signed_cell.ref, acquire);

                builtin.atomics.atomic_store(unsigned_cell.ref, u_big, release);
                let umax_prev = builtin.atomics.atomic_fetch_max(unsigned_cell.ref, u_5, seq_cst);
                let umax_cur = builtin.atomics.atomic_load(unsigned_cell.ref, acquire);
                let umin_prev = builtin.atomics.atomic_fetch_min(unsigned_cell.ref, u_7, seq_cst);
                let umin_cur = builtin.atomics.atomic_load(unsigned_cell.ref, acquire);

                builtin.atomics.atomic_store(flag.ref, true, release);
                flag_exp = true;
                let bcas = builtin.atomics.atomic_cmpxchg(flag.ref, flag_exp.ref, false, seq_cst, acquire);
                let bload = builtin.atomics.atomic_load(flag.ref, acquire);

                let pcas = builtin.atomics.atomic_cmpxchg(ptr_cell.ref, ptr_exp.ref, target_b.ref, seq_cst, acquire);
                let pload = builtin.atomics.atomic_load(ptr_cell.ref, acquire);

                builtin.atomics.atomic_fence(dyn_seq);
                builtin.atomics.atomic_compiler_fence(acquire);

                l1 == 10
                    and sw == 10
                    and fa == 20
                    and fs == 25
                    and fo == 22
                    and fx == 278
                    and fn_and == 23
                    and cur == 7
                    and (not cas_miss)
                    and miss_exp == 7
                    and cas_hit
                    and after_cas == 55
                    and smax_prev == -10
                    and smax_cur == 3
                    and smin_prev == 3
                    and smin_cur == -20
                    and umax_prev == 0xFFFFFFF0
                    and umax_cur == 0xFFFFFFF0
                    and umin_prev == 0xFFFFFFF0
                    and umin_cur == 7
                    and bcas
                    and (not bload)
                    and pcas
                    and addr_of(pload) == addr_of(target_b.ref)
            }}
        }}
        "#
    );

    let case = NativeCase::new(&source);
    let ir = case
        .emit_llvm("run")
        .expect("LLVM IR emission should succeed");
    assert!(
        ir.contains("store atomic i64"),
        "expected atomic store in IR:\n{ir}"
    );
    // Verify the function containing `store atomic i64` does not get `memory(none)`.
    let store_fn_header = ir
        .split("define internal ")
        .find(|block| block.contains("store atomic i64"))
        .and_then(|block| block.lines().next())
        .expect("store atomic function header missing");
    if let Some((_, attr_id)) = store_fn_header.rsplit_once('#') {
        let attr_tag = format!("attributes #{} = {{", attr_id.trim_end_matches(" {"));
        if let Some(attr_line) = ir.lines().find(|line| line.starts_with(&attr_tag)) {
            assert!(
                !attr_line.contains("memory(none)") && !attr_line.contains("nounwind"),
                "atomic store function must not be memory(none) or nounwind: {attr_line}"
            );
        }
    }
    assert!(
        ir.contains("atomicrmw max ptr"),
        "expected signed atomicrmw max in IR:\n{ir}"
    );
    assert!(
        ir.contains("atomicrmw umax ptr"),
        "expected unsigned atomicrmw umax in IR:\n{ir}"
    );
    assert!(
        ir.contains("fence syncscope(\"singlethread\") acquire"),
        "expected compiler_fence in IR:\n{ir}"
    );

    for opt in ["0", "3"] {
        let build = case.build("run", opt);
        assert_success(&build);
        let run = case.run();
        assert_success(&run);
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "(bool true)");
    }
}

#[test]
fn invalid_atomic_ordering_traps_at_runtime() {
    for (label, call) in [
        (
            "release_load",
            "let _ = builtin.atomics.atomic_load(cell.ref, bad_rel);",
        ),
        (
            "acquire_store",
            "builtin.atomics.atomic_store(cell.ref, one, bad_acq);",
        ),
        ("relaxed_fence", "builtin.atomics.atomic_fence(zero);"),
        (
            "release_failure_cmpxchg",
            "let _ = builtin.atomics.atomic_cmpxchg(cell.ref, exp.ref, one, seq, bad_rel);",
        ),
    ] {
        let source = format!(
            r#"{INTRINSIC_PREAMBLE}
            fn pick(v: u8) -> u8 {{ v }}
            fn run() -> bool {{
                var cell: u64 = 0;
                var exp: u64 = 0;
                let one: u64 = 1;
                let bad_acq = pick(1);
                let bad_rel = pick(2);
                let zero = pick(0);
                let seq = pick(4);
                unsafe {{ {call} }}
                true
            }}
            "#
        );
        let case = NativeCase::new(&source);
        let build = case.build("run", "0");
        assert_success(&build);
        let run = case.run();
        assert!(
            !run.status.success(),
            "{label} should trap on invalid ordering, got {:?}",
            run.status
        );
    }
}

#[test]
fn runtime_panic_unwinds_reverse_drop_order_and_abort_skips_drop() {
    let panic_source = format!(
        r#"{INTRINSIC_PREAMBLE}
        struct TraceDrop {{
            tag: u32,
        }}
        impl Drop for TraceDrop {{
            fn drop(*self) {{
                let t = self.tag.*;
                if t == 1 {{ let _ = linux.write_str(2, "drop:1\n", 0); }}
                else if t == 2 {{ let _ = linux.write_str(2, "drop:2\n", 0); }}
                else if t == 3 {{ let _ = linux.write_str(2, "drop:3\n", 0); }}
                else {{ let _ = linux.write_str(2, "drop:outer\n", 0); }}
            }}
        }}
        fn inner_panic() -> NoReturn {{
            let d1 = TraceDrop {{ tag: 1 }};
            let d2 = TraceDrop {{ tag: 2 }};
            let d3 = TraceDrop {{ tag: 3 }};
            trigger_panic("boom")
        }}
        fn run() -> bool {{
            let outer = TraceDrop {{ tag: 0 }};
            inner_panic()
        }}
        "#
    );

    let case = NativeCase::new(&panic_source);
    let build = case.build("run", "0");
    assert_success(&build);
    let run = case.run();
    assert!(!run.status.success(), "panic should terminate abnormally");
    let stderr = String::from_utf8_lossy(&run.stderr);
    let expected = "panic: boom\ndrop:3\ndrop:2\ndrop:1\ndrop:outer\n";
    assert!(
        stderr.contains(expected),
        "expected reverse drop unwind `{expected}` in stderr:\n{stderr}"
    );

    // Abort must trap immediately without running any Drop handlers.
    let abort_source = format!(
        r#"{INTRINSIC_PREAMBLE}
        struct TraceDrop {{
            tag: u32,
        }}
        impl Drop for TraceDrop {{
            fn drop(*self) {{
                let _ = linux.write_str(2, "drop:should-not-run\n", 0);
            }}
        }}
        fn run() -> bool {{
            let guard = TraceDrop {{ tag: 0 }};
            trigger_abort()
        }}
        "#
    );
    let abort_case = NativeCase::new(&abort_source);
    let abort_build = abort_case.build("run", "3");
    assert_success(&abort_build);
    let abort_run = abort_case.run();
    assert!(!abort_run.status.success(), "abort must trap");
    let abort_stderr = String::from_utf8_lossy(&abort_run.stderr);
    assert!(
        !abort_stderr.contains("drop:should-not-run"),
        "abort must not invoke Drop:\n{abort_stderr}"
    );
}

#[test]
fn double_panic_in_drop_traps_immediately_without_running_earlier_drops() {
    let source = format!(
        r#"{INTRINSIC_PREAMBLE}
        struct LogDrop {{
            tag: u32,
            panic_on_drop: bool,
        }}
        impl Drop for LogDrop {{
            fn drop(*self) {{
                if self.tag.* == 1 {{
                    let _ = linux.write_str(2, "drop:first\n", 0);
                }} else {{
                    let _ = linux.write_str(2, "drop:second\n", 0);
                }}
                if self.panic_on_drop.* {{
                    trigger_panic("double")
                }}
            }}
        }}
        fn run() -> bool {{
            let first = LogDrop {{ tag: 1, panic_on_drop: false }};
            let second = LogDrop {{ tag: 2, panic_on_drop: true }};
            trigger_panic("outer")
        }}
        "#
    );

    let case = NativeCase::new(&source);
    for opt in ["0", "3"] {
        let build = case.build("run", opt);
        assert_success(&build);
        let run = case.run();
        assert!(!run.status.success(), "opt={opt}: double panic must trap");
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(stderr.contains("panic: outer\n"), "opt={opt}: {stderr}");
        assert!(stderr.contains("drop:second\n"), "opt={opt}: {stderr}");
        assert!(stderr.contains("panic: double\n"), "opt={opt}: {stderr}");
        assert!(
            !stderr.contains("drop:first"),
            "double panic in drop:second must trap immediately before drop:first runs (opt={opt}):\n{stderr}"
        );
    }
}

#[test]
fn c_abi_boundary_traps_when_panic_unwinds_into_c_caller() {
    let source = format!(
        r#"{INTRINSIC_PREAMBLE}
        struct TraceDrop {{
            tag: u32,
        }}
        impl Drop for TraceDrop {{
            fn drop(*self) {{
                let _ = linux.write_str(2, "drop:inside-c-entry\n", 0);
            }}
        }}
        ^attrs.abi(.c)
        fn c_panic_entry(flag: u32) -> u32 {{
            if flag == 0 {{
                let guard = TraceDrop {{ tag: 0 }};
                trigger_panic("cross-c")
            }} else {{
                42
            }}
        }}
        fn run() -> bool {{
            c_panic_entry(1) == 42
        }}
        "#
    );

    let case = NativeCase::new(&source);
    let ir = case
        .emit_llvm("run")
        .expect("LLVM IR emission should succeed")
        .replace("define i32 @main()", "define i32 @unused_main()");
    let ll_path = case.0.join("module.ll");
    fs::write(&ll_path, &ir).unwrap();
    let llc = ProcessCommand::new("llc-21")
        .current_dir(&case.0)
        .args([
            "-relocation-model=pic",
            "-filetype=obj",
            "module.ll",
            "-o",
            "module.o",
        ])
        .output()
        .unwrap();
    assert_success(&llc);

    let c_main = case.0.join("c_main.c");
    fs::write(
        &c_main,
        "extern unsigned int c_panic_entry(unsigned int flag);\nint main(void) { return (int)c_panic_entry(0); }\n",
    )
    .unwrap();
    let cc_out = ProcessCommand::new("cc")
        .current_dir(&case.0)
        .args(["c_main.c", "module.o", "-o", "c_program"])
        .output()
        .unwrap();
    assert_success(&cc_out);

    let run = ProcessCommand::new(case.0.join("c_program"))
        .current_dir(&case.0)
        .output()
        .unwrap();
    assert!(
        !run.status.success(),
        "C-ABI boundary must trap on panic unwind"
    );
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("panic: cross-c\n"), "{stderr}");
    assert!(stderr.contains("drop:inside-c-entry\n"), "{stderr}");
}
