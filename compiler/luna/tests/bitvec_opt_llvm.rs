//! End-to-end integration tests wiring `bitvec_opt` into `host/llvm`:
//! - Verifies algebraic simplification & target-aware synthesis (`@llvm.bswap.i{16,32,64}`,
//!   `xor`, `@llvm.fshr.i16` / `@llvm.fshl.i16`, `shl` / `lshr`) in emitted LLVM IR.
//! - Verifies `BitVec<0>`, non-power-of-2 (`BitVec<1>`, `BitVec<5>`, `BitVec<12>`, `BitVec<24>`),
//!   64-bit, 128-bit, and wide `BitVec<1024>` execution at `-O0` and `-O3`.
//! - Verifies side-effect execution order (`side_effect() and 0`, `left() and right()`,
//!   `left() or right()`), local mutation isolation (`memory_version`), and `Drop` resource
//!   cleanup preservation under checked native execution.

use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-bvopt-{}-{}",
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

#[test]
fn synthesizes_bswap_xor_rotate_and_shift_idioms_in_llvm_ir_and_runs_at_o0_o3() {
    let source = r#"
        use std.prelude.*;

        fn swap_bytes_16(word: u16) -> u16 {
            let bits = word'bitvec;
            (bits[0..8] ++ bits[8..16]).as(u16)
        }

        fn swap_bytes_32(word: u32) -> u32 {
            let bits = word'bitvec;
            (bits[0..8] ++ bits[8..16] ++ bits[16..24] ++ bits[24..32]).as(u32)
        }

        fn swap_bytes_64(word: u64) -> u64 {
            let bits = word'bitvec;
            (bits[0..8]
                ++ bits[8..16]
                ++ bits[16..24]
                ++ bits[24..32]
                ++ bits[32..40]
                ++ bits[40..48]
                ++ bits[48..56]
                ++ bits[56..64])
                .as(u64)
        }

        fn xor_via_algebra(a: BitVec<8>, b: BitVec<8>) -> BitVec<8> {
            (a and (not b)) or ((not a) and b)
        }

        fn rotate_16(word: u16) -> u16 {
            let bits = word'bitvec;
            (bits[0..5] ++ bits[5..16]).as(u16)
        }

        fn shift_left_concat(bits: BitVec<32>) -> BitVec<32> {
            const ZERO8: BitVec<8> = 0;
            bits[0..24] ++ ZERO8
        }

        fn shift_right_zext(bits: BitVec<32>) -> BitVec<32> {
            bits[8..32].as(BitVec<32>)
        }

        fn run() -> bool {
            let b16_zero = swap_bytes_16(0) == 0;
            let b16_val = swap_bytes_16(0x1234) == 0x3412 and swap_bytes_16(0xAF3C) == 0x3CAF and swap_bytes_16(0xFFFF) == 0xFFFF;
            let b32_ok = swap_bytes_32(0) == 0
                and swap_bytes_32(0x12345678) == 0x78563412
                and swap_bytes_32(0xDEADBEEF) == 0xEFBEADDE
                and swap_bytes_32(0xFFFFFFFF) == 0xFFFFFFFF;
            let b64_ok = swap_bytes_64(0) == 0
                and swap_bytes_64(0x0123456789ABCDEF) == 0xEFCDAB8967452301
                and swap_bytes_64(0xFFFFFFFFFFFFFFFF) == 0xFFFFFFFFFFFFFFFF;
            let xor_ok = xor_via_algebra(0xAF, 0x3C) == 0x93
                and xor_via_algebra(0xFF, 0xFF) == 0
                and xor_via_algebra(0, 0x5A) == 0x5A;
            let rot_ok = rotate_16(0x1234) == 0xA091 and rotate_16(0) == 0 and rotate_16(0xFFFF) == 0xFFFF;
            let shl_ok = shift_left_concat(0x12345678) == 0x34567800;
            let shr_ok = shift_right_zext(0x12345678) == 0x00123456;

            b16_zero and b16_val and b32_ok and b64_ok and xor_ok and rot_ok and shl_ok and shr_ok
        }
    "#;

    let case = NativeCase::new(source);
    let ir = case.emit_llvm().expect("LLVM IR emission should succeed");

    if cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
        assert!(
            ir.contains("@llvm.bswap.i16"),
            "missing bswap16 in IR:\n{ir}"
        );
        assert!(
            ir.contains("@llvm.bswap.i32"),
            "missing bswap32 in IR:\n{ir}"
        );
        assert!(
            ir.contains("@llvm.bswap.i64"),
            "missing bswap64 in IR:\n{ir}"
        );
        assert!(ir.contains("xor i8 "), "missing xor i8 in IR:\n{ir}");
        assert!(
            ir.contains("@llvm.fshr.i16") || ir.contains("@llvm.fshl.i16"),
            "missing funnel shift rotate in IR:\n{ir}"
        );
        assert!(ir.contains("shl i32 "), "missing shl i32 in IR:\n{ir}");
        assert!(ir.contains("lshr i32 "), "missing lshr i32 in IR:\n{ir}");
    }

    for opt in ["0", "3"] {
        let build = case.build(opt);
        assert_success(&build);
        let run = case.run();
        assert_success(&run);
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert_eq!(stdout.trim(), "(bool true)", "opt={opt}: {stdout}");
    }
}

#[test]
fn handles_zero_odd_and_wide_bitvec_widths_natively_at_o0_and_o3() {
    let source = r#"
        use std.prelude.*;

        fn run() -> bool {
            let empty: BitVec<0> = 0;
            let b5: BitVec<5> = 0b10110;
            let b1: BitVec<1> = 1;
            let cat0 = empty ++ b5 ++ empty;
            let ok0 = (not empty) == empty and empty == 0 and cat0 == b5;

            let b12: BitVec<12> = b5 ++ (not b5) ++ b1 ++ b1;
            let rot12 = b12[0..5] ++ b12[5..12];
            let ok12 = rot12[7..12] == b12[0..5] and rot12[0..7] == b12[5..12];

            let b24: BitVec<24> = (b12 ++ (not b12));
            let b24_xor = (b24 and (not b24)) or 0;
            let ok24 = b24_xor == 0;

            let w64: BitVec<64> = 0x0123456789ABCDEF;
            let w128: BitVec<128> = w64 ++ (not w64);
            let ok128 = w128[64..128] == w64 and w128[0..64] == (not w64);

            let ones1024: BitVec<1024> = not 0;
            let zeros1024: BitVec<1024> = 0;
            let mixed1024: BitVec<1024> = w128.as(BitVec<1024>);
            let ok1024 = (ones1024 and zeros1024) == 0
                and (ones1024 or zeros1024) == ones1024
                and (not not ones1024) == ones1024
                and mixed1024[0..128] == w128
                and mixed1024[128..1024] == 0;

            ok0 and ok12 and ok24 and ok128 and ok1024
        }
    "#;

    let case = NativeCase::new(source);
    let _ir = case
        .emit_llvm()
        .expect("wide and odd BitVec IR should emit cleanly");

    for opt in ["0", "3"] {
        let build = case.build(opt);
        assert_success(&build);
        let run = case.run();
        assert_success(&run);
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert_eq!(stdout.trim(), "(bool true)", "opt={opt}: {stdout}");
    }
}

#[test]
fn preserves_side_effect_order_mutation_isolation_and_drop_contracts() {
    let source = r#"
        use std.prelude.*;

        struct DropGuard {
            tag: u64,
            bits: BitVec<8>,
        }

        impl Drop for DropGuard {
            fn drop(*self) {
                let _ = std.fmt.println("drop:{}", self.tag.*);
            }
        }

        fn log_bits(label: str, val: BitVec<8>) -> BitVec<8> {
            let _ = std.fmt.println("fx:{}", label);
            val
        }

        fn drop_leaf(tag: u64, val: BitVec<8>) -> BitVec<8> {
            let g = DropGuard { tag: tag, bits: val };
            g.bits
        }

        fn check_side_effect_fold() -> bool {
            let res = log_bits("zero-fold", 0xAA) and 0;
            res == 0
        }

        fn check_eval_order() -> bool {
            let a = log_bits("and-L", 0x00) and log_bits("and-R", 0xFF);
            let o = log_bits("or-L", 0xFF) or log_bits("or-R", 0x00);
            a == 0 and o == 0xFF
        }

        fn check_mutation_isolation() -> bool {
            var a: BitVec<8> = 0xAA;
            -- Must not simplify `a and (do { a = 0x0F; a })` to `a`.
            let masked = a and (do {
                a = 0x0F;
                a
            });

            var bits: BitVec<16> = 0x1234;
            -- Must not simplify to bswap(bits) because right operand mutates bits.
            let mutated_cat = bits[0..8] ++ (do {
                bits = 0x9988;
                bits
            })[8..16];

            -- Verify mutation visible in subsequent read too.
            var b: BitVec<8> = 0xF0;
            let full: BitVec<8> = 0xFF;
            let seq_and = b and (do { b = 0x3C; full }) and b;

            masked == 0x0A
                and mutated_cat == 0x3499
                and seq_and == 0x30
        }

        fn check_drop_leaf_fold() -> bool {
            let r1 = (drop_leaf(101, 0xFF) and 0)
                and (drop_leaf(202, 0x55) or 0xFF);
            let r2 = (do {
                let g = DropGuard { tag: 303, bits: 0xAA };
                g.bits
            }) and 0;
            r1 == 0 and r2 == 0
        }

        fn run() -> bool {
            check_side_effect_fold()
                and check_eval_order()
                and check_mutation_isolation()
                and check_drop_leaf_fold()
        }
    "#;

    let case = NativeCase::new(source);
    let _ir = case
        .emit_llvm()
        .expect("effectful BitVec IR should emit cleanly");

    let expected_lines = [
        "fx:zero-fold",
        "fx:and-L",
        "fx:and-R",
        "fx:or-L",
        "fx:or-R",
        "drop:101",
        "drop:202",
        "drop:303",
        "(bool true)",
    ];

    for opt in ["0", "3"] {
        let build = case.build(opt);
        assert_success(&build);
        let run = case.run();
        assert_success(&run);
        let stdout = String::from_utf8_lossy(&run.stdout);
        let actual_lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(actual_lines, expected_lines, "opt={opt}:\n{stdout}");
    }
}
