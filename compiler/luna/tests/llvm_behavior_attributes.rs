//! Integration tests verifying certified purity and memory behavior attributes
//! emitted to LLVM 23.1.4 IR (`memory(...)`, `willreturn`, `nounwind`,
//! `speculatable`, `nosync`, `nofree`, pointer parameter attributes, and
//! indirect callsite attributes) under `-O0` and `-O3` native execution.
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
            "luna-behavior-attrs-{}-{}",
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

fn contains_all_tokens(text: &str, required: &[&str]) -> bool {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    required.iter().all(|needle| tokens.contains(needle))
}

fn contains_any_token(text: &str, forbidden: &[&str]) -> bool {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    forbidden.iter().any(|needle| tokens.contains(needle))
}

#[test]
fn p5_total_pure_and_p4_loop_enforce_termination_boundary_through_llvm_o3() {
    let source = r#"
        fn total_mix(a: u32, b: u32) -> u32 {
            builtin.math.saturating_add(a, b)
        }

        fn loop_pure(n: u32) -> u32 {
            var i: u32 = 0;
            var acc: u32 = 0;
            while i < n {
                acc = builtin.math.saturating_add(acc, i);
                i = builtin.math.saturating_add(i, 1);
            }
            acc
        }

        fn run() -> u32 {
            var sum: u32 = 0;
            var k: u32 = 0;
            while k < 4 {
                sum = builtin.math.saturating_add(sum, total_mix(10, 20));
                k = builtin.math.saturating_add(k, 1);
            }
            builtin.math.saturating_add(sum, loop_pure(4))
        }
    "#;

    let case = NativeCase::new(source);
    let ir = case.emit_llvm().unwrap();

    // Check both P5 (speculatable + willreturn + memory(none)) and P4 (no willreturn/speculatable) attribute groups exist.
    let p5_group_present = ir.lines().any(|line| {
        line.starts_with("attributes #")
            && contains_all_tokens(
                line,
                &[
                    "nounwind",
                    "willreturn",
                    "nosync",
                    "nofree",
                    "memory(none)",
                    "speculatable",
                ],
            )
    });
    assert!(
        p5_group_present,
        "expected P5 speculatable attribute group in IR:\n{ir}"
    );
    let p4_group_present = ir.lines().any(|line| {
        line.starts_with("attributes #")
            && contains_all_tokens(line, &["nounwind", "nosync", "nofree", "memory(none)"])
            && !contains_any_token(line, &["willreturn", "speculatable"])
    });
    assert!(
        p4_group_present,
        "expected P4 non-terminating group without willreturn/speculatable:\n{ir}"
    );

    let optimized = Toolchain {
        optimization: OptimizationLevel::O3,
        ..Toolchain::default()
    }
    .optimized_ir(&ir)
    .unwrap();
    assert!(!optimized.is_empty());

    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let out = case.run();
        assert_success(&out);
        assert_eq!(out.stdout, b"(unsigned-integer 126)\n");
    }
}

#[test]
fn guarded_traps_and_preconditions_forbid_memory_none_and_speculative_hoisting() {
    let source = r#"
        fn checked_div(a: u32, b: u32) -> u32
            where requires b != 0
        {
            a / b
        }

        fn guarded_div(flag: bool, d: u32) -> u32 {
            if flag and d != 0 {
                checked_div(100, d)
            } else {
                77
            }
        }

        fn run() -> u32 {
            guarded_div(false, 0)
        }
    "#;

    let case = NativeCase::new(source);
    let ir = case.emit_llvm().unwrap();
    for line in ir.lines().filter(|line| line.starts_with("define ")) {
        if let Some(group) = attribute_group_for(&ir, line) {
            assert!(
                !contains_any_token(group, &["memory(none)", "speculatable"]),
                "trapping precondition call chain must not get memory(none)/speculatable on `{line}`:\n{ir}"
            );
        }
    }

    let optimized = Toolchain {
        optimization: OptimizationLevel::O3,
        ..Toolchain::default()
    }
    .optimized_ir(&ir)
    .unwrap();
    assert!(!optimized.is_empty());

    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let out = case.run();
        assert_success(&out);
        assert_eq!(out.stdout, b"(unsigned-integer 77)\n");
    }
}

#[test]
fn pointer_params_indirect_callsites_and_dealloc_attributes_pass_llvm23_opt_o3() {
    let pointer_source = r#"
        fn read_word(p: *u32) -> u32 {
            p.*
        }

        fn write_word(p: *u32, value: u32) {
            p.* = value;
        }

        fn is_even_low_bit(x: u32) -> bool {
            builtin.math.bit_and(x, 1) == 0
        }

        fn run() -> u32 {
            var slot: u32 = 10;
            write_word(slot.ref, 42);
            let cb: fn(u32) -> bool = is_even_low_bit;
            let loaded = read_word(slot.ref);
            if cb(loaded) {
                loaded
            } else {
                0
            }
        }
    "#;

    let case = NativeCase::new(pointer_source);
    let ir = case.emit_llvm().unwrap();
    let has_readonly_ptr_param = ir.lines().any(|line| {
        line.starts_with("define internal i32 @flurry_")
            && line.contains("ptr ")
            && line.contains("readonly")
            && line.contains("noalias")
            && line.contains("captures(none)")
            && line.contains("nonnull")
    });
    assert!(
        has_readonly_ptr_param,
        "expected readonly noalias captures(none) nonnull ptr param for read_word:\n{ir}"
    );
    let has_write_ptr_param = ir.lines().any(|line| {
        line.starts_with("define internal void @flurry_")
            && line.contains("ptr ")
            && !line.contains("readonly")
            && line.contains("noalias")
            && line.contains("captures(none)")
            && line.contains("nonnull")
    });
    assert!(
        has_write_ptr_param,
        "expected non-readonly noalias captures(none) nonnull ptr param for write_word:\n{ir}"
    );
    assert!(
        ir.contains("memory(argmem: read)"),
        "expected memory(argmem: read) for read_word:\n{ir}"
    );
    assert!(
        ir.contains("memory(argmem: readwrite)"),
        "expected memory(argmem: readwrite) for write_word:\n{ir}"
    );
    // Verify indirect callsite in `run` attaches `#<group>`.
    let has_indirect_attr_call = ir
        .lines()
        .any(|line| line.contains("call i1 %") && line.contains(" #"));
    assert!(
        has_indirect_attr_call,
        "expected indirect callsite attribute group on `cb(loaded)`:\n{ir}"
    );

    let optimized = Toolchain {
        optimization: OptimizationLevel::O3,
        ..Toolchain::default()
    }
    .optimized_ir(&ir)
    .unwrap();
    assert!(!optimized.is_empty());

    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let out = case.run();
        assert_success(&out);
        assert_eq!(out.stdout, b"(unsigned-integer 42)\n");
    }

    // Verify `SystemAllocator.deallocate` (`LinuxDealloc`) is never annotated `nofree` or `memory(none)`.
    let dealloc_source = r#"
        use std.mem.{Allocator, Layout, RawBlock, SystemAllocator};

        fn free_block(block: RawBlock) {
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(block);
            }
        }

        fn run() -> u32 {
            let layout = Layout.new(64, 8) match {
                found? => found,
                null => return 1,
            };
            let block = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                b! => b,
                error _ => return 2,
            };
            free_block(block);
            0
        }
    "#;
    let dealloc_case = NativeCase::new(dealloc_source);
    let dealloc_ir = dealloc_case.emit_llvm().unwrap();
    // Neither `@flurry_entry` nor any deallocating helper may carry `nofree` or `memory(none)`.
    let entry_line = dealloc_ir
        .lines()
        .find(|line| line.starts_with("define ") && line.contains("@flurry_entry("))
        .unwrap_or_else(|| panic!("missing @flurry_entry in IR:\n{dealloc_ir}"));
    if let Some(group) = attribute_group_for(&dealloc_ir, entry_line) {
        assert!(
            !contains_any_token(group, &["nofree", "memory(none)"]),
            "entry calling deallocate must not be marked nofree/memory(none):\n{dealloc_ir}"
        );
    }
    assert!(
        !dealloc_ir.split("\ndefine ").skip(1).any(|fn_block| {
            let header = fn_block.lines().next().unwrap_or("");
            let does_dealloc =
                fn_block.contains("munmap") || fn_block.contains("asm sideeffect \"syscall\"");
            does_dealloc
                && attribute_group_for(&dealloc_ir, header)
                    .is_some_and(|g| contains_any_token(g, &["nofree", "memory(none)"]))
        }),
        "function performing dealloc must not be marked nofree/memory(none):\n{dealloc_ir}"
    );
    assert_success(&dealloc_case.build("3"));
    let out = dealloc_case.run();
    assert_success(&out);
    assert_eq!(out.stdout, b"(unsigned-integer 0)\n");
}
