//! End-to-end tests for Stage 2E-1: IEEE-754 float `Display`/`Debug`
//! (`f16`, `bf16`, `f32`, `f64`) and integer radix formatting traits +
//! `{:x}` / `{:#x}` / `{:X}` / `{:#X}` / `{:b}` / `{:#b}` / `{:o}` / `{:#o}`.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(prefix: &str, source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self) -> Output {
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
                "0",
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

#[test]
fn comptime_radix_format_plan_and_trait_refinement_rejection() {
    let source = r#"
        use std.fmt.{FormatErr, FormatKind, FormatPiece, FormatPlan};

        comptime fn check_radix_plan() -> bool {
            let plan = FormatPlan.parse("{:x} {:#x} {:X} {:#X} {:b} {:#b} {:o} {:#o}") match {
                p! => p,
                error _ => return false,
            };
            if plan.argument_count != 8 or plan.pieces.length() != 15 {
                return false;
            }
            let pieces_ok = (plan.pieces[0] matches FormatPiece.lower_hex_argument(0, false))
                and (plan.pieces[2] matches FormatPiece.lower_hex_argument(1, true))
                and (plan.pieces[4] matches FormatPiece.upper_hex_argument(2, false))
                and (plan.pieces[6] matches FormatPiece.upper_hex_argument(3, true))
                and (plan.pieces[8] matches FormatPiece.binary_argument(4, false))
                and (plan.pieces[10] matches FormatPiece.binary_argument(5, true))
                and (plan.pieces[12] matches FormatPiece.octal_argument(6, false))
                and (plan.pieces[14] matches FormatPiece.octal_argument(7, true));
            if not pieces_ok {
                return false;
            }

            let kinds = plan.argument_kinds();
            if kinds.length() != 8 {
                return false;
            }
            let kinds_ok = (kinds[0] matches FormatKind.lower_hex)
                and (kinds[1] matches FormatKind.lower_hex_alt)
                and (kinds[2] matches FormatKind.upper_hex)
                and (kinds[3] matches FormatKind.upper_hex_alt)
                and (kinds[4] matches FormatKind.binary)
                and (kinds[5] matches FormatKind.binary_alt)
                and (kinds[6] matches FormatKind.octal)
                and (kinds[7] matches FormatKind.octal_alt);
            if not kinds_ok {
                return false;
            }

            let mixed = FormatPlan.parse("{} {:?} {:#x} {:b} {:o}") match {
                p! => p,
                error _ => return false,
            };
            let mixed_kinds = mixed.argument_kinds();
            let mixed_domains = mixed.argument_domains();
            let mixed_ok = mixed_kinds.length() == 5
                and mixed_domains.length() == 5
                and (mixed_kinds[0] matches FormatKind.display)
                and (mixed_kinds[1] matches FormatKind.debug)
                and (mixed_kinds[2] matches FormatKind.lower_hex_alt)
                and (mixed_kinds[3] matches FormatKind.binary)
                and (mixed_kinds[4] matches FormatKind.octal);
            if not mixed_ok {
                return false;
            }

            (FormatPlan.parse("{:q}") matches error FormatErr.UnsupportedField(0))
                and (FormatPlan.parse("{:#}") matches error FormatErr.UnsupportedField(0))
                and (FormatPlan.parse("{:#?}") matches error FormatErr.UnsupportedField(0))
                and (FormatPlan.parse("{:xx}") matches error FormatErr.UnsupportedField(0))
                and (FormatPlan.parse("{:#") matches error FormatErr.UnclosedField(0))
                and (FormatPlan.parse("{:#x") matches error FormatErr.UnclosedField(0))
                and (FormatPlan.parse("{:x") matches error FormatErr.UnclosedField(0))
                and (FormatPlan.parse("a{:?") matches error FormatErr.UnclosedField(1))
        }

        fn run() -> bool {
            check_radix_plan()
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));

    let bool_hex_error = support::compile(
        r#"
        fn run() -> !std.mem.AllocErr std.string.String {
            std.fmt.format("{:x}", true)
        }
        "#,
    )
    .unwrap_err();
    assert!(
        bool_hex_error.contains("refinement requirement is not established"),
        "{bool_hex_error}"
    );

    let custom_hex_ok = support::compile(
        r#"
        use std.fmt.{LowerHex, format};
        use builtin.fmt.ByteWriter;
        struct HexOnly { tag: u8 }
        impl LowerHex for HexOnly {
            fn write_lower_hex(*self, output: *W) -> !W.as(ByteWriter).Err void where W :- ByteWriter {
                output.write_str("hex")
            }
        }
        fn run() -> !std.mem.AllocErr std.string.String {
            format("{:#x}", HexOnly { tag: 1 })
        }
        "#,
    );
    assert!(custom_hex_ok.is_ok(), "{custom_hex_ok:?}");

    let custom_hex_no_display = support::compile(
        r#"
        use std.fmt.{LowerHex, format};
        use builtin.fmt.ByteWriter;
        struct HexOnly { tag: u8 }
        impl LowerHex for HexOnly {
            fn write_lower_hex(*self, output: *W) -> !W.as(ByteWriter).Err void where W :- ByteWriter {
                output.write_str("hex")
            }
        }
        fn run() -> !std.mem.AllocErr std.string.String {
            format("{}", HexOnly { tag: 1 })
        }
        "#,
    )
    .unwrap_err();
    assert!(
        custom_hex_no_display.contains("refinement requirement is not established"),
        "{custom_hex_no_display}"
    );
}

#[test]
fn native_radix_formatting_across_machine_ints_and_custom_allocators() {
    let source = r#"
        use std.prelude.*;
        use std.fmt.LowerHex;
        use builtin.fmt.ByteWriter;
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, SystemAllocator};

        struct HexTag {}

        impl LowerHex for HexTag {
            fn write_lower_hex(*self, output: *W) -> !W.as(ByteWriter).Err void where W :- ByteWriter {
                output.write_str("custom")
            }
        }

        fn check_radix_small() -> !AllocErr bool {
            let v_u8: u8 = 0;
            let v_i8_neg: i8 = -1;
            let v_i128_zero: i128 = 0;

            let zeros = format(
                "{:x}|{:#x}|{:X}|{:#X}|{:b}|{:#b}|{:o}|{:#o}",
                v_u8,
                v_u8,
                v_u8,
                v_u8,
                v_u8,
                v_u8,
                v_u8,
                v_i128_zero,
            )!;
            if not zeros.ref.equals_str("0|0x0|0|0X0|0|0b0|0|0o0") {
                return false;
            }

            let signed_neg = format(
                "{:x}|{:#x}|{:X}|{:#X}|{:b}|{:#b}|{:o}|{:#o}",
                v_i8_neg,
                v_i8_neg,
                v_i8_neg,
                v_i8_neg,
                v_i8_neg,
                v_i8_neg,
                v_i8_neg,
                v_i8_neg,
            )!;
            signed_neg.ref.equals_str("ff|0xff|FF|0XFF|11111111|0b11111111|377|0o377")
        }

        fn check_radix_wide() -> !AllocErr bool {
            let v_u16: u16 = 0x1a2b;
            let v_u32: u32 = 0xdeadbeef;
            let v_u64: u64 = 255;
            let v_u128_max: u128 = 340282366920938463463374607431768211455;
            let v_usize: usize = 0x2f;

            let v_i16_neg: i16 = -1;
            let v_i32_neg: i32 = -1;
            let v_i64_neg: i64 = -1;
            let v_isize_neg: isize = -1;

            let wide_signed = format(
                "{:x}|{:x}|{:x}|{:x}",
                v_i16_neg,
                v_i32_neg,
                v_i64_neg,
                v_isize_neg,
            )!;
            if not wide_signed.ref.equals_str(
                "ffff|ffffffff|ffffffffffffffff|ffffffffffffffff",
            ) {
                return false;
            }

            let wide_unsigned = format(
                "{:x}|{:X}|{:#x}|{:b}|{:o}|{:x}",
                v_u16,
                v_u32,
                v_u64,
                v_usize,
                v_usize,
                v_u128_max,
            )!;
            wide_unsigned.ref.equals_str(
                "1a2b|DEADBEEF|0xff|101111|57|ffffffffffffffffffffffffffffffff",
            )
        }

        fn check_radix_custom(fba: *FixedBufferAllocator) -> !AllocErr bool {
            let v_u8: u8 = 0;
            let v_u16: u16 = 0x1a2b;
            let v_u64: u64 = 255;
            let custom = format<.A = FixedBufferAllocator, .allocator = fba>(
                "{} {:?} {:#x} {:X} {:b} {:o}",
                v_u64,
                "hi",
                HexTag {},
                v_u16,
                v_u16 % 256,
                v_u8 + 9,
            )!;
            custom.ref.equals_str("255 \"hi\" 0xcustom 1A2B 101011 11")
        }

        fn exercise(fba: *FixedBufferAllocator) -> !AllocErr u32 {
            let ok1 = check_radix_small()!;
            if not ok1 {
                return 1;
            }
            let ok2 = check_radix_wide()!;
            if not ok2 {
                return 2;
            }
            let ok3 = check_radix_custom(fba)!;
            if not ok3 {
                return 3;
            }
            0
        }

        fn run() -> u32 {
            let layout = Layout.new(65536, 16) match {
                found? => found,
                null => return 97,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return 98,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let code = exercise(fba.ref) match {
                passed! => passed,
                error _ => 99,
            };
            let clean = fba.ref.live_allocations() == 0;
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            if clean { code } else { 96 }
        }
    "#;

    let case = NativeCase::new("stage2e-radix-fmt", source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(unsigned-integer 0)\n");
}

#[test]
fn native_float_display_and_debug_for_f64_f32_f16_and_bf16() {
    let source = r#"
        use std.prelude.*;
        use std.mem.AllocErr;

        fn bits_f64(raw: u64) -> f64 {
            raw'bitvec.as(f64)
        }

        fn bits_f32(raw: u32) -> f32 {
            raw'bitvec.as(f32)
        }

        fn bits_f16(raw: u16) -> f16 {
            raw'bitvec.as(f16)
        }

        fn bits_bf16(raw: u16) -> bf16 {
            raw'bitvec.as(bf16)
        }

        fn check_pair(disp: str, dbg: str, a: String, b: String) -> bool {
            a.ref.equals_str(disp) and b.ref.equals_str(dbg)
        }

        fn check_f64_basic() -> !AllocErr u32 {
            if not check_pair("0", "0.0", format("{}", 0.0)!, format("{:?}", 0.0)!) {
                return 1;
            }
            let neg_zero_64 = bits_f64(0x8000000000000000);
            if not check_pair(
                "-0",
                "-0.0",
                format("{}", neg_zero_64)!,
                format("{:?}", neg_zero_64)!,
            ) {
                return 2;
            }
            if not check_pair("1", "1.0", format("{}", 1.0)!, format("{:?}", 1.0)!) {
                return 3;
            }
            if not check_pair("-42", "-42.0", format("{}", -42.0)!, format("{:?}", -42.0)!) {
                return 4;
            }
            if not check_pair("0.1", "0.1", format("{}", 0.1)!, format("{:?}", 0.1)!) {
                return 5;
            }
            if not check_pair("0.3", "0.3", format("{}", 0.3)!, format("{:?}", 0.3)!) {
                return 6;
            }
            if not check_pair("1.5", "1.5", format("{}", 1.5)!, format("{:?}", 1.5)!) {
                return 7;
            }
            if not check_pair(
                "3.141592653589793",
                "3.141592653589793",
                format("{}", 3.141592653589793)!,
                format("{:?}", 3.141592653589793)!,
            ) {
                return 8;
            }
            if not check_pair("0.0001", "0.0001", format("{}", 0.0001)!, format("{:?}", 0.0001)!) {
                return 9;
            }
            0
        }

        fn check_f64_edges() -> !AllocErr u32 {
            let one_e_neg5 = bits_f64(0x3ee4f8b588e368f1);
            if not check_pair(
                "1e-5",
                "1.0e-5",
                format("{}", one_e_neg5)!,
                format("{:?}", one_e_neg5)!,
            ) {
                return 10;
            }
            let one_e20 = bits_f64(0x4415af1d78b58c40);
            if not check_pair(
                "1e20",
                "1.0e20",
                format("{}", one_e20)!,
                format("{:?}", one_e20)!,
            ) {
                return 11;
            }
            let min_sub_64 = bits_f64(0x0000000000000001);
            if not check_pair(
                "5e-324",
                "5.0e-324",
                format("{}", min_sub_64)!,
                format("{:?}", min_sub_64)!,
            ) {
                return 12;
            }
            let min_norm_64 = bits_f64(0x0010000000000000);
            if not check_pair(
                "2.2250738585072014e-308",
                "2.2250738585072014e-308",
                format("{}", min_norm_64)!,
                format("{:?}", min_norm_64)!,
            ) {
                return 13;
            }
            let max_64 = bits_f64(0x7fefffffffffffff);
            if not check_pair(
                "1.7976931348623157e308",
                "1.7976931348623157e308",
                format("{}", max_64)!,
                format("{:?}", max_64)!,
            ) {
                return 14;
            }
            let exact_edge_64 = bits_f64(0xc3595dc20bd372a9);
            if not check_pair(
                "-2.8559849684847268e16",
                "-2.8559849684847268e16",
                format("{}", exact_edge_64)!,
                format("{:?}", exact_edge_64)!,
            ) {
                return 15;
            }
            let pos_inf_64 = bits_f64(0x7ff0000000000000);
            let neg_inf_64 = bits_f64(0xfff0000000000000);
            let nan_64 = bits_f64(0x7ff8000000000000);
            if not check_pair("inf", "inf", format("{}", pos_inf_64)!, format("{:?}", pos_inf_64)!)
                or not check_pair(
                    "-inf",
                    "-inf",
                    format("{}", neg_inf_64)!,
                    format("{:?}", neg_inf_64)!,
                )
                or not check_pair("NaN", "NaN", format("{}", nan_64)!, format("{:?}", nan_64)!)
            {
                return 16;
            }
            0
        }

        fn check_f32_and_f16() -> !AllocErr u32 {
            let point_one_32: f32 = 0.1;
            let forty_two_32: f32 = 42.0;
            let one_five_32: f32 = 1.5;
            let neg_zero_32 = bits_f32(0x80000000);
            let pos_inf_32 = bits_f32(0x7f800000);
            let neg_inf_32 = bits_f32(0xff800000);
            let nan_32 = bits_f32(0x7fc00000);
            if not check_pair(
                "0.1",
                "0.1",
                format("{}", point_one_32)!,
                format("{:?}", point_one_32)!,
            )
                or not check_pair(
                    "-0",
                    "-0.0",
                    format("{}", neg_zero_32)!,
                    format("{:?}", neg_zero_32)!,
                )
                or not check_pair(
                    "42",
                    "42.0",
                    format("{}", forty_two_32)!,
                    format("{:?}", forty_two_32)!,
                )
                or not check_pair(
                    "1.5",
                    "1.5",
                    format("{}", one_five_32)!,
                    format("{:?}", one_five_32)!,
                )
                or not check_pair(
                    "inf",
                    "inf",
                    format("{}", pos_inf_32)!,
                    format("{:?}", pos_inf_32)!,
                )
                or not check_pair(
                    "-inf",
                    "-inf",
                    format("{}", neg_inf_32)!,
                    format("{:?}", neg_inf_32)!,
                )
                or not check_pair("NaN", "NaN", format("{}", nan_32)!, format("{:?}", nan_32)!)
            {
                return 17;
            }

            let one_five_16 = bits_f16(0x3e00);
            let neg_zero_16 = bits_f16(0x8000);
            let pos_inf_16 = bits_f16(0x7c00);
            let neg_inf_16 = bits_f16(0xfc00);
            let nan_16 = bits_f16(0x7e00);
            let min_sub_16 = bits_f16(0x0001);
            if not check_pair(
                "1.5",
                "1.5",
                format("{}", one_five_16)!,
                format("{:?}", one_five_16)!,
            )
                or not check_pair(
                    "-0",
                    "-0.0",
                    format("{}", neg_zero_16)!,
                    format("{:?}", neg_zero_16)!,
                )
                or not check_pair(
                    "inf",
                    "inf",
                    format("{}", pos_inf_16)!,
                    format("{:?}", pos_inf_16)!,
                )
                or not check_pair(
                    "-inf",
                    "-inf",
                    format("{}", neg_inf_16)!,
                    format("{:?}", neg_inf_16)!,
                )
                or not check_pair("NaN", "NaN", format("{}", nan_16)!, format("{:?}", nan_16)!)
                or not check_pair(
                    "6e-8",
                    "6.0e-8",
                    format("{}", min_sub_16)!,
                    format("{:?}", min_sub_16)!,
                )
            {
                return 18;
            }
            0
        }

        fn check_bf16() -> !AllocErr u32 {
            let one_five_bf16 = bits_bf16(0x3fc0);
            let neg_zero_bf16 = bits_bf16(0x8000);
            let pos_inf_bf16 = bits_bf16(0x7f80);
            let neg_inf_bf16 = bits_bf16(0xff80);
            let nan_bf16 = bits_bf16(0x7fc0);
            if not check_pair(
                "1.5",
                "1.5",
                format("{}", one_five_bf16)!,
                format("{:?}", one_five_bf16)!,
            )
                or not check_pair(
                    "-0",
                    "-0.0",
                    format("{}", neg_zero_bf16)!,
                    format("{:?}", neg_zero_bf16)!,
                )
                or not check_pair(
                    "inf",
                    "inf",
                    format("{}", pos_inf_bf16)!,
                    format("{:?}", pos_inf_bf16)!,
                )
                or not check_pair(
                    "-inf",
                    "-inf",
                    format("{}", neg_inf_bf16)!,
                    format("{:?}", neg_inf_bf16)!,
                )
                or not check_pair("NaN", "NaN", format("{}", nan_bf16)!, format("{:?}", nan_bf16)!)
            {
                return 19;
            }
            0
        }

        fn exercise() -> !AllocErr u32 {
            let r1 = check_f64_basic()!;
            if r1 != 0 {
                return r1;
            }
            let r2 = check_f64_edges()!;
            if r2 != 0 {
                return r2;
            }
            let r3 = check_f32_and_f16()!;
            if r3 != 0 {
                return r3;
            }
            check_bf16()
        }

        fn run() -> u32 {
            exercise() match {
                code! => code,
                error _ => 99,
            }
        }
    "#;

    let case = NativeCase::new("stage2e-float-fmt", source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(unsigned-integer 0)\n");
}
