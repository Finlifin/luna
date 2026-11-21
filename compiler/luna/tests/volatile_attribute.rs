//! End-to-end integration tests for `__flurry_volatile` (`attrs.mem.volatile` / `attrs.volatile`),
//! verifying `load volatile` / `store volatile` LLVM lowering, `-O3` optimization
//! preservation, conservative P0 purity/behavior attributes, and per-field isolation.
use luna::host::{OptimizationLevel, Toolchain};
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-volatile-attr-{}-{}",
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

    fn emit_llvm(&self) -> Result<String, String> {
        let output = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "dump",
                "main.fl",
                "--entry",
                "run",
                "--emit",
                "llvm",
                "--opt-level",
                "0",
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(Self::sysroot())
            .output()
            .unwrap();
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
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
            .arg(Self::sysroot())
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

fn attribute_group_for<'a>(ir: &'a str, define_line: &str) -> Option<&'a str> {
    let group = define_line
        .split('#')
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())?;
    let prefix = format!("attributes #{group} = {{ ");
    ir.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| rest.strip_suffix(" }"))
}

fn contains_any_token(text: &str, forbidden: &[&str]) -> bool {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    forbidden.iter().any(|needle| tokens.contains(needle))
}

#[test]
fn volatile_struct_and_newtype_emit_volatile_ir_preserve_o3_and_execute() {
    let source = r#"
        ^attrs.mem.volatile
        struct MmioReg {
            value: u32,
        }

        ^attrs.volatile
        newtype VolU32 = u32;

        fn pulse_mmio(reg: *MmioReg) -> u32 {
            reg.*.value = 1;
            reg.*.value = 40;
            let first = reg.*.value;
            let second = reg.*.value;
            builtin.math.saturating_add(first, second)
        }

        fn touch_newtype(a: u32, b: u32) -> u32 {
            var word: VolU32 = a.as(VolU32);
            word = b.as(VolU32);
            word.as(u32)
        }

        fn run() -> u32 {
            var reg: MmioReg = MmioReg { value: 0 };
            builtin.math.saturating_add(pulse_mmio(reg.ref), touch_newtype(2, 3))
        }
    "#;

    let case = NativeCase::new(source);
    let ir = case.emit_llvm().unwrap();

    // `pulse_mmio` must emit `load volatile` / `store volatile` and lose pure/nosync/readonly/noalias attrs.
    let pulse_block = ir
        .split("\ndefine ")
        .skip(1)
        .find(|blk| blk.contains("store volatile i32 40,") && blk.contains("load volatile i32,"))
        .unwrap_or_else(|| panic!("expected `pulse_mmio` with volatile load/store in IR:\n{ir}"));
    let pulse_header = pulse_block.lines().next().unwrap_or("");
    assert!(
        !pulse_header.contains("readonly") && !pulse_header.contains("noalias"),
        "volatile pointer param must omit readonly/noalias:\n{pulse_header}"
    );
    if let Some(group) = attribute_group_for(&ir, &format!("define {pulse_header}")) {
        assert!(
            !contains_any_token(
                group,
                &[
                    "memory(none)",
                    "memory(argmem: read)",
                    "memory(argmem: readwrite)",
                    "speculatable",
                    "nosync",
                ]
            ),
            "volatile accessor retained unsound LLVM attributes `{group}`:\n{ir}"
        );
    }

    // Verify LLVM -O3 keeps both volatile stores (`1` and `40`) and does not fold away volatile loads.
    let optimized = Toolchain {
        optimization: OptimizationLevel::O3,
        ..Toolchain::default()
    }
    .optimized_ir(&ir)
    .unwrap();
    assert!(
        optimized.contains("store volatile i32 1,") && optimized.contains("store volatile i32 40,"),
        "LLVM -O3 must preserve repeated volatile stores:\n{optimized}"
    );
    assert!(
        optimized.matches("load volatile i32,").count() >= 2,
        "LLVM -O3 must preserve repeated volatile loads:\n{optimized}"
    );

    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let out = case.run();
        assert_success(&out);
        assert_eq!(out.stdout, b"(unsigned-integer 83)\n");
    }
}

#[test]
fn volatile_field_is_isolated_from_nonvolatile_sibling_field() {
    let source = r#"
        struct Uart {
            ^attrs.mem.volatile
            status: u32,
            normal: u32,
        }

        fn read_status(u: *Uart) -> u32 {
            u.*.status
        }

        fn write_status(u: *Uart, val: u32) {
            u.*.status = val;
        }

        fn read_normal(u: *Uart) -> u32 {
            u.*.normal
        }

        fn run() -> u32 {
            var u: Uart = Uart { status: 10, normal: 5 };
            write_status(u.ref, 37);
            builtin.math.saturating_add(read_status(u.ref), read_normal(u.ref))
        }
    "#;

    let case = NativeCase::new(source);
    let ir = case.emit_llvm().unwrap();

    // Expect at least one `load volatile i32` + `store volatile i32` for `status`,
    // and `read_normal` to retain `readonly` + `memory(argmem: read)` without `volatile`.
    assert!(
        ir.contains("load volatile i32,") && ir.contains("store volatile i32 "),
        "expected volatile load and store on `status` field:\n{ir}"
    );
    let read_normal_block = ir
        .split("\ndefine ")
        .skip(1)
        .find(|blk| {
            let header = blk.lines().next().unwrap_or("");
            header.starts_with("internal i32 @flurry_")
                && header.contains("ptr ")
                && blk.contains("getelementptr ")
                && blk.contains(", i32 0, i32 1")
                && !blk.contains(" volatile ")
        })
        .unwrap_or_else(|| {
            panic!("expected non-volatile `read_normal` accessing field index 1:\n{ir}")
        });
    let normal_header = read_normal_block.lines().next().unwrap_or("");
    assert!(
        normal_header.contains("readonly"),
        "non-volatile `read_normal` param should keep `readonly`:\n{normal_header}"
    );

    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let out = case.run();
        assert_success(&out);
        assert_eq!(out.stdout, b"(unsigned-integer 42)\n");
    }
}

#[test]
fn pure_fn_rejects_volatile_struct_and_volatile_field_accesses() {
    for source in [
        "
            ^attrs.mem.volatile
            struct MmioReg { value: u32 }

            pure fn bad_struct(reg: *MmioReg) -> u32 {
                reg.*.value
            }

            fn run() -> u32 { 0 }
        ",
        "
            struct Uart {
                ^attrs.mem.volatile
                status: u32,
                normal: u32,
            }

            pure fn bad_field(u: *Uart) -> u32 {
                u.*.status
            }

            fn run() -> u32 { 0 }
        ",
    ] {
        let case = NativeCase::new(source);
        let err = case
            .emit_llvm()
            .expect_err("expected pure fn accessing volatile storage to be rejected");
        assert!(
            err.contains("pure") || err.contains("purity") || err.contains("elaboration"),
            "unexpected error for pure volatile read:\n{err}"
        );
    }
}
