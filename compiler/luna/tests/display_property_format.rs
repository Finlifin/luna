//! End-to-end tests for property-based formatting (`Display` runtime &
//! `^attrs.comptime_phase` `comptime_fmt`, `std.fmt.comptime_format`,
//! `std.fmt.format` / `write_fmt`, integer/float/text/`Debug` property options,
//! custom user `Display` property parsing, affine `Once` non-consuming borrows,
//! and compile-time rejection of malformed/mismatched options).
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
fn native_property_format_options_ints_floats_text_and_debug() {
    let source = r#"
        use std.prelude.*;
        use builtin.fmt.ByteWriter;
        use std.mem.AllocErr;
        use std.time.Duration;

        struct OnceRecord {
            label: str,
            drops: *u32,
        }

        impl builtin.Drop for OnceRecord {
            fn drop(*self) {
                let ptr = self.drops.*;
                let cur = ptr.*;
                if cur < 1000 {
                    ptr.* = cur + 1;
                }
            }
        }

        impl Display for OnceRecord {
            fn fmt(*self, output: *W, comptime .format: str = "") -> !W.as(ByteWriter).Err void
            where
                W :- ByteWriter
            {
                let opts = builtin.fmt.options.parse_text_options(format);
                builtin.fmt.options.write_padded_str_truncated(self.label.*, opts, output)
            }
        }

        fn check_integers() -> !AllocErr bool {
            let v: u32 = 0x2a;
            let ex = format("{format: .hex, align_to: .right, len: 12, placeholder: '0'}", v)!;
            if not ex.ref.equals_str("00000000002a") {
                return false;
            }

            let neg: i32 = -42;
            let pos: i32 = 42;
            let s1 = format(
                "{format: .dec, sign: .plus}|{format: .dec, sign: .space}|{format: .dec, sign: .minus}",
                pos,
                pos,
                neg,
            )!;
            if not s1.ref.equals_str("+42| 42|-42") {
                return false;
            }

            -- Sign/prefix-aware zero padding (default align) vs explicit align_to: .right
            let zpad = format(
                "{len: 6, placeholder: '0'}|{len: 6, sign: .plus, placeholder: '0'}|{format: .hex, alt: true, len: 6, placeholder: '0'}|{align_to: .right, len: 6, placeholder: '0'}",
                neg,
                pos,
                v,
                neg,
            )!;
            if not zpad.ref.equals_str("-00042|+00042|0x002a|000-42") {
                return false;
            }

            let n7: u32 = 7;
            let n255: u32 = 255;
            let n5: u8 = 5;
            let n64: u16 = 64;
            let align_mix = format(
                "{align_to: .left, len: 5, placeholder: '*'}|{align_to: .center, len: 6, placeholder: '·'}|{align_to: .right, len: 4, placeholder: '}'}|{format: .HEX, alt: true, len: 6, align_to: .left, placeholder: ','}|{format: .bin, alt: true}|{format: .oct, alt: true}",
                pos,
                pos,
                n7,
                n255,
                n5,
                n64,
            )!;
            align_mix.ref.equals_str("42***|··42··|}}}7|0XFF,,|0b101|0o100")
        }

        fn check_floats() -> !AllocErr bool {
            let pi: f64 = 3.1415926;
            let carry: f64 = 9.996;
            let f1 = format(
                "{precision: 2}|{precision: 0}|{precision: 2}|{format: .dec, sign: .plus, precision: 2}|{sign: .space, precision: 1}",
                pi,
                pi,
                carry,
                pi,
                pi,
            )!;
            if not f1.ref.equals_str("3.14|3|10.00|+3.14| 3.1") {
                return false;
            }

            let neg_zero_bits: u64 = 0x8000000000000000;
            let pos_inf_bits: u64 = 0x7ff0000000000000;
            let neg_inf_bits: u64 = 0xfff0000000000000;
            let nan_bits: u64 = 0x7ff8000000000000;
            let neg_zero = neg_zero_bits'bitvec.as(f64);
            let pos_inf = pos_inf_bits'bitvec.as(f64);
            let neg_inf = neg_inf_bits'bitvec.as(f64);
            let nan_val = nan_bits'bitvec.as(f64);
            let neg_three_half: f32 = -3.5;
            let f2 = format(
                "{precision: 2}|{sign: .plus, len: 6, placeholder: '*'}|{len: 6}|{sign: .plus, len: 6}|{len: 7, placeholder: '0'}",
                neg_zero,
                pos_inf,
                neg_inf,
                nan_val,
                neg_three_half,
            )!;
            if not f2.ref.equals_str("-0.00|**+inf|  -inf|   NaN|-0003.5") {
                return false;
            }

            let half_bits: u16 = 0x3e00;
            let bf_bits: u16 = 0x3fc0;
            let half = half_bits'bitvec.as(f16);
            let bf = bf_bits'bitvec.as(bf16);
            let f3 = format("{precision: 3, sign: .plus}|{precision: 2, align_to: .right, len: 6, placeholder: '_'}", half, bf)!;
            f3.ref.equals_str("+1.500|__1.50")
        }

        fn check_text_debug_and_once() -> !AllocErr bool {
            let s = String.from_str("你好世界")!;
            var drop_count: u32 = 0;
            let rec = OnceRecord { label: "alpha", drops: drop_count.ref };

            let rendered = format(
                "{precision: 2}|{precision: 2, align_to: .right, len: 4, placeholder: '·'}|{align_to: .center, len: 5, placeholder: '\\\''}|{len: 4, align_to: .right, placeholder: '0'}|{align_to: .center, len: 6, placeholder: '*'}|{len: 4, align_to: .right, placeholder: '.'}|{precision: 3, len: 5, align_to: .right, placeholder: '_'}|{format: .debug, align_to: .right, len: 8, placeholder: '.'}",
                "你好世界",
                s,
                true,
                'A',
                .ok,
                "x",
                rec,
                "hi",
            )!;
            -- rec and s must not have been dropped during format() (or rec was moved into varargs and dropped once on return of format)
            if not rendered.ref.equals_str("你好|··你好|true'|000A|**ok**|...x|__alp|....\"hi\"") {
                return false;
            }

            -- Verify explicit *self Display.fmt borrow does NOT consume `OnceRecord` or `String`
            var sink = Vec<u8>.new();
            let rec2 = OnceRecord { label: "keep", drops: drop_count.ref };
            let owned_s = String.from_str("hello")!;
            rec2.ref.as(Display).fmt(sink.ref, .format = "precision: 2, len: 4, align_to: .right, placeholder: '-'")!;
            owned_s.ref.as(Display).fmt(sink.ref, .format = "precision: 3")!;
            let rec2_ref = rec2.ref;
            if rec2_ref.drops.*.* != 1 {
                return false;
            }
            let out_s = String.from_utf8_lossy(sink)!;
            let ok_dur = format("{format: .debug, align_to: .left, len: 40, placeholder: '.'}", Duration.from_millis(5))!;
            out_s.ref.equals_str("--kehel") and ok_dur.ref.equals_str("Duration { secs: 0, nanos: 5000000 }....")
        }

        fn exercise() -> !AllocErr u32 {
            let ints_ok = check_integers()!;
            if not ints_ok {
                return 1;
            }
            let floats_ok = check_floats()!;
            if not floats_ok {
                return 2;
            }
            let text_ok = check_text_debug_and_once()!;
            if not text_ok {
                return 3;
            }
            0
        }

        fn run() -> u32 {
            exercise() match {
                code! => code,
                error _ => 99,
            }
        }
    "#;

    let case = NativeCase::new("display-props", source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(unsigned-integer 0)\n");
}

#[test]
fn custom_display_type_parses_custom_properties_in_both_phases() {
    let source = r#"
        use std.prelude.*;
        use builtin.fmt.ByteWriter;
        use meta.ComptimeTextBytes;

        struct Tag {
            code: u8,
        }

        pure comptime fn parse_style(format: str) -> u8 {
            if format == "" {
                0
            } else if format == "style: .upper" {
                1
            } else if format == "style: .bracket, sep: \":\"" {
                2
            } else {
                assumes false;
                0
            }
        }

        impl Display for Tag {
            fn fmt(*self, output: *W, comptime .format: str = "") -> !W.as(ByteWriter).Err void
            where
                W :- ByteWriter
            {
                let mode: u8 = parse_style(format);
                if mode == 1 {
                    output.write_str("TAG=")!;
                    self.code.as(Display).fmt(output, .format = "format: .HEX")
                } else if mode == 2 {
                    output.write_str("[tag:")!;
                    self.code.as(Display).fmt(output)!;
                    output.write_byte(93)
                } else {
                    output.write_str("tag=")!;
                    self.code.as(Display).fmt(output)
                }
            }
        }

        comptime impl Display for Tag {
            comptime fn comptime_fmt(self, .format: str = "") -> str {
                let mode: u8 = parse_style(format);
                if mode == 1 {
                    "TAG=".concat(self.code.as(Display).comptime_fmt(.format = "format: .HEX"))
                } else if mode == 2 {
                    "[tag:".concat(self.code.as(Display).comptime_fmt()).concat("]")
                } else {
                    "tag=".concat(self.code.as(Display).comptime_fmt())
                }
            }
        }

        const CT_OUT: str = comptime_format(
            "{}|{}|{}",
            Tag { code: 26 },
            Tag { code: 26 },
            Tag { code: 26 },
        );

        const CT_CUSTOM: str = comptime_format(
            "{}|{style: .upper}|{style: .bracket, sep: \":\"}",
            Tag { code: 26 },
            Tag { code: 26 },
            Tag { code: 26 },
        );

        const CT_OUT_OK: bool = CT_OUT == "tag=26|tag=26|tag=26";
        const CT_CUSTOM_OK: bool = CT_CUSTOM == "tag=26|TAG=1A|[tag:26]";

        fn run_native() -> !std.mem.AllocErr bool {
            let s = format(
                "{}|{style: .upper}|{style: .bracket, sep: \":\"}",
                Tag { code: 26 },
                Tag { code: 26 },
                Tag { code: 26 },
            )!;
            s.ref.equals_str("tag=26|TAG=1A|[tag:26]")
                and CT_CUSTOM_OK
                and CT_OUT_OK
        }

        fn run() -> bool {
            run_native() match {
                ok! => ok,
                error _ => false,
            }
        }
    "#;

    let case = NativeCase::new("display-custom-props", source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn comptime_format_covers_integer_machine_ints_text_char_symbol_and_options() {
    let source = r#"
        use std.fmt.comptime_format;

        comptime fn run() -> bool {
            let big: Integer = 340282366920938463463374607431768211456 + 42;
            let neg_big: Integer = -255;
            let u_max: u128 = 340282366920938463463374607431768211455;
            let min_i8: i8 = -128;
            let u_val: u32 = 0x2a;
            let i_val: i32 = -42;
            let sz: usize = 16;
            let isz: isize = -7;

            let s1 = comptime_format(
                "plain={} hex={format: .hex, align_to: .right, len: 12, placeholder: '0'} neg={} big={} nbig={format: .hex, alt: true}",
                true,
                u_val,
                i_val,
                big,
                neg_big,
            );
            let ok1 = s1 == "plain=true hex=00000000002a neg=-42 big=340282366920938463463374607431768211498 nbig=-0xff";

            let s2 = comptime_format(
                "{precision: 2, align_to: .center, len: 6, placeholder: '·'}|{align_to: .right, len: 3, placeholder: '*'}|{}|{}|{}|{format: .oct, alt: true}|{len: 4, placeholder: '0'}|{{}}",
                "你好世界",
                '界',
                .ready,
                min_i8,
                u_max,
                sz,
                isz,
            );
            let ok2 = s2 == "··你好··|**界|ready|-128|340282366920938463463374607431768211455|0o20|-007|{}";

            let zero = comptime_format("literal {{text}}");
            ok1 and ok2 and zero == "literal {text}"
        }
    "#;
    assert_eq!(support::invoke(source, "run").unwrap(), Value::Bool(true));
}

#[test]
fn property_format_compile_time_rejections() {
    for invalid_plan in [
        r#"format("{:x}", x)"#,
        r#"format("{:?}", x)"#,
        r#"format("{:#x}", x)"#,
        r#"format("{0}", x)"#,
        r#"format("{name}", x)"#,
        r#"format("{ }", x)"#,
        r#"format("{len: 4, len: 8}", x)"#,
        r#"format("{placeholder: ''}", x)"#,
        r#"format("{placeholder: 'ab'}", x)"#,
        r#"format("{placeholder: '\\q'}", x)"#,
        r#"format("{placeholder: '\\x{110000}'}", x)"#,
        r#"format("{len: 4", x)"#,
    ] {
        let src = format!(
            "use std.prelude.*; fn run() -> !std.mem.AllocErr String {{ let x: u32 = 1; {invalid_plan} }}"
        );
        let err = support::compile(&src).unwrap_err();
        assert!(
            !err.contains("LiteralInto") && !err.contains("FileParsingFailed"),
            "unexpected syntax/literal failure for {invalid_plan}: {err}"
        );
    }

    for invalid_option in [
        r#"format("{format: .hex}", true)"#,
        r#"format("{format: .hex}", "hi")"#,
        r#"format("{format: .hex}", f)"#,
        r#"format("{precision: 2}", n)"#,
        r#"format("{precision: 2}", true)"#,
        r#"format("{format: .debug, precision: 2}", "hi")"#,
        r#"format("{alt: true}", n)"#,
        r#"format("{format: .dec, alt: true}", n)"#,
        r#"format("{format: .hex, sign: .plus}", n)"#,
        r#"format("{bogus: true}", n)"#,
        r#"format("{bogus: true}", "hi")"#,
    ] {
        let src = format!(
            "use std.prelude.*; fn run() -> !std.mem.AllocErr String {{ let n: u32 = 42; let f: f64 = 1.5; {invalid_option} }}"
        );
        let res = support::compile(&src)
            .and_then(|pkg| luna::instantiation::instantiate(&pkg, "run", 1_000_000).map(|_| ()));
        assert!(
            res.is_err(),
            "expected option rejection for {invalid_option}"
        );
    }

    // Debug is disallowed in std.fmt.comptime_format
    let ct_src = r#"
        use std.fmt.comptime_format;
        comptime fn run() -> str {
            let n: u32 = 42;
            comptime_format("{format: .debug}", n)
        }
    "#;
    let ct_debug = support::invoke(ct_src, "run");
    assert!(
        ct_debug.is_err(),
        "expected comptime_format to reject format: .debug"
    );
}
