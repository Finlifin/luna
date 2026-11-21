//! End-to-end tests for Stage 2D: `{:?}` Debug formatting in `std.fmt`,
//! `Symbol` `Display`/`Debug`, `char` ASCII/UTF-8 helpers, and runtime/comptime
//! `TextBytes` / `ComptimeTextBytes` methods on `str`.
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
fn comptime_format_plan_debug_spec_and_comptime_text_bytes() {
    let source = r#"
        use meta.ComptimeTextBytes;
        use std.fmt.{FormatErr, FormatPiece, FormatPlan};

        comptime fn check_plan() -> bool {
            let plan = FormatPlan.parse("a={} b={format: .debug} {{}}") match {
                p! => p,
                error _ => return false,
            };
            if plan.argument_count != 2 or plan.pieces.length() != 7 {
                return false;
            }
            let pieces_ok = (plan.pieces[0] matches FormatPiece.text(0, 2))
                and (plan.pieces[1] matches FormatPiece.argument(0, 3, 3))
                and (plan.pieces[2] matches FormatPiece.text(4, 7))
                and (plan.pieces[3] matches FormatPiece.debug_argument(1, 8, 22))
                and (plan.pieces[4] matches FormatPiece.text(23, 24))
                and (plan.pieces[5] matches FormatPiece.text(24, 25))
                and (plan.pieces[6] matches FormatPiece.text(26, 27));
            if not pieces_ok {
                return false;
            }

            let dbg = plan.argument_is_debug();
            if dbg.length() != 2 or (dbg[0] != false) or (dbg[1] != true) {
                return false;
            }

            let segs = plan.literal_segments("a={} b={format: .debug} {{}}");
            if segs.length() != 3 or segs[0] != "a=" or segs[1] != " b=" or segs[2] != " {}" {
                return false;
            }

            (FormatPlan.parse("{:") matches error FormatErr.UnclosedField(0))
                and (FormatPlan.parse("{format: .debug") matches error FormatErr.UnclosedField(0))
                and (FormatPlan.parse("x{:") matches error FormatErr.UnclosedField(1))
                and (FormatPlan.parse("x{format: .debug") matches error FormatErr.UnclosedField(1))
                and (FormatPlan.parse("{:?}") matches error FormatErr.UnsupportedField(0))
                and (FormatPlan.parse("{:q}") matches error FormatErr.UnsupportedField(0))
                and (FormatPlan.parse("{:}") matches error FormatErr.UnsupportedField(0))
        }

        comptime fn check_text_bytes() -> bool {
            if not "".is_empty() or "a".is_empty() {
                return false;
            }
            if not "".is_ascii()
                or not "abc123!~".is_ascii()
                or "a中".is_ascii()
                or "😀".is_ascii()
            {
                return false;
            }
            let sample = "a中😀";
            sample.is_char_boundary(0)
                and sample.is_char_boundary(1)
                and (not sample.is_char_boundary(2))
                and (not sample.is_char_boundary(3))
                and sample.is_char_boundary(4)
                and (not sample.is_char_boundary(5))
                and (not sample.is_char_boundary(6))
                and (not sample.is_char_boundary(7))
                and sample.is_char_boundary(8)
                and (not sample.is_char_boundary(9))
        }

        fn run() -> bool {
            check_plan() and check_text_bytes()
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));

    let no_debug_error = support::compile(
        r#"
        struct NoDebug {}
        fn run() -> !std.mem.AllocErr std.string.String {
            std.fmt.format("{format: .debug}", NoDebug {})
        }
        "#,
    )
    .unwrap_err();
    assert!(
        no_debug_error.contains("refinement requirement is not established"),
        "{no_debug_error}"
    );

    let no_display_error = support::compile(
        r#"
        fn run() -> !std.mem.AllocErr std.string.String {
            std.fmt.format("{}", std.time.ZERO)
        }
        "#,
    )
    .unwrap_err();
    assert!(
        no_display_error.contains("refinement requirement is not established"),
        "{no_display_error}"
    );
}

#[test]
fn std_fmt_debug_specifier_supports_mixed_heterogeneous_once_and_custom_allocators() {
    let source = r#"
        use std.prelude.*;
        use builtin.fmt.ByteWriter;
        use std.collections.VecDeque;
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, SystemAllocator};
        use std.process.ExitCode;
        use std.time.Duration;

        struct Plain {
            v: u32,
        }

        impl Display for Plain {
            fn fmt(*self, output: *W, comptime .format: str = "") -> !W.as(ByteWriter).Err void
            where
                W :- ByteWriter
            {
                output.write_str("Plain(")!;
                self.v.as(Display).fmt(output, .format = format)!;
                output.write_byte(41)
            }
        }

        fn make_fba_str(text: str) -> !AllocErr String<.A = FixedBufferAllocator, .allocator = fba>
        where
            quote fba: lift *FixedBufferAllocator,
        {
            String<.A = FixedBufferAllocator, .allocator = fba>.from_str(text)
        }

        fn exercise(fba: *FixedBufferAllocator) -> !AllocErr u32 {
            -- 1. DefaultAllocator `format` mixing Display-only, Debug-only Duration, char, str, Symbol.
            let msg = format(
                "p={} d={format: .debug} c={} cd={format: .debug} s={} sd={format: .debug} sym={} symd={format: .debug}",
                Plain { v: 9 },
                Duration.from_millis(1250),
                'x',
                '\n',
                "hi",
                "a\tb",
                .ok,
                .ok,
            )!;
            if not msg.ref.equals_str(
                "p=Plain(9) d=Duration { secs: 1, nanos: 250000000 } c=x cd='\\n' s=hi sd=\"a\\tb\" sym=ok symd=.ok",
            ) {
                return 1;
            }

            -- 2. Custom FixedBufferAllocator `format` with Once String, Vec, VecDeque, ExitCode, bool, ints.
            let owned = make_fba_str<fba>("own")!;
            var vec_nums = Vec<i32, .A = FixedBufferAllocator, .allocator = fba>.new();
            vec_nums.ref.push(-5)!;
            vec_nums.ref.push(42)!;
            var dq_nums = VecDeque<i32, .A = FixedBufferAllocator, .allocator = fba>.new();
            dq_nums.ref.push_back(1)!;
            dq_nums.ref.push_back(2)!;
            let fba_msg = format<.A = FixedBufferAllocator, .allocator = fba>(
                "b={} n={} s={format: .debug} sd={} v={format: .debug} dq={format: .debug} ec={format: .debug}",
                true,
                -5,
                owned,
                make_fba_str<fba>("st")!,
                vec_nums,
                dq_nums,
                ExitCode.from_u8(7),
            )!;
            if not fba_msg.ref.equals_str(
                "b=true n=-5 s=\"own\" sd=st v=[-5, 42] dq=[1, 2] ec=ExitCode(7)",
            ) {
                return 2;
            }

            -- 3. Custom FixedBufferAllocator `format_into` with Box<String> + Display-only Plain + Once String.
            var target = make_fba_str<fba>("lead:")!;
            let boxed = Box<
                String<.A = FixedBufferAllocator, .allocator = fba>,
                .A = FixedBufferAllocator,
                .allocator = fba,
            >.new(make_fba_str<fba>("bx")!)!;
            format_into(
                target.ref,
                " {format: .debug} + {} = {format: .debug}",
                boxed,
                Plain { v: 3 },
                make_fba_str<fba>("rhs\n")!,
            )!;
            if not target.ref.equals_str("lead: \"bx\" + Plain(3) = \"rhs\\n\"") {
                return 3;
            }

            -- 4. `write_fmt` into Vec<u8> ByteWriter.
            var raw = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            write_fmt(raw.ref, "{format: .debug}:{}", "k", 99)!;
            let raw_str = String<.A = FixedBufferAllocator, .allocator = fba>.from_utf8_lossy(raw)!;
            if not raw_str.ref.equals_str("\"k\":99") {
                return 4;
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

    let case = NativeCase::new("stage2d-fmt-debug", source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(unsigned-integer 0)\n");
}

#[test]
fn builtin_symbol_formatting_char_methods_and_runtime_str_text_bytes() {
    let source = r#"
        use std.prelude.*;
        use std.mem.AllocErr;

        fn check_runtime_str(empty_s: str, ascii_s: str, hay: str, utf8_s: str) -> bool {
            if not empty_s.is_empty() or ascii_s.is_empty() or utf8_s.is_empty() {
                return false;
            }
            if not empty_s.is_ascii() or not ascii_s.is_ascii() or utf8_s.is_ascii() {
                return false;
            }

            -- "a中😀" has UTF-8 length 8 with boundaries at 0, 1, 4, 8.
            let boundaries_ok = utf8_s.is_char_boundary(0)
                and utf8_s.is_char_boundary(1)
                and (not utf8_s.is_char_boundary(2))
                and (not utf8_s.is_char_boundary(3))
                and utf8_s.is_char_boundary(4)
                and (not utf8_s.is_char_boundary(5))
                and (not utf8_s.is_char_boundary(6))
                and (not utf8_s.is_char_boundary(7))
                and utf8_s.is_char_boundary(8)
                and (not utf8_s.is_char_boundary(9));
            if not boundaries_ok {
                return false;
            }

            let affixes_ok = hay.starts_with("")
                and hay.starts_with("aba")
                and hay.starts_with("ababa")
                and (not hay.starts_with("baa"))
                and (not hay.starts_with("abababa"))
                and hay.ends_with("")
                and hay.ends_with("ba")
                and hay.ends_with("ababa")
                and (not hay.ends_with("abb"))
                and (not hay.ends_with("xababa"))
                and empty_s.starts_with("")
                and empty_s.ends_with("")
                and (not empty_s.starts_with("a"));
            if not affixes_ok {
                return false;
            }

            let empty_zero = empty_s.find("") matches 0?;
            let empty_miss = empty_s.find("a") matches null;
            let hay_empty = hay.find("") matches 0?;
            let hay_ba = hay.find("ba") matches 1?;
            let hay_aba = hay.find("aba") matches 0?;
            let hay_miss = hay.find("xyz") matches null;
            let utf8_emoji = utf8_s.find("😀") matches 4?;
            let contains_ok = hay.contains("bab")
                and hay.contains("")
                and (not hay.contains("bbb"))
                and utf8_s.contains("a中")
                and (not utf8_s.contains("中!"));
            empty_zero and empty_miss and hay_empty and hay_ba and hay_aba and hay_miss and utf8_emoji and contains_ok
        }

        fn exercise() -> !AllocErr u32 {
            -- 1. Symbol Display & Debug.
            let sym = .linux;
            let sym_disp = format("{}", sym)!;
            let sym_dbg = format_debug(sym.ref)!;
            if not sym_disp.ref.equals_str("linux") or not sym_dbg.ref.equals_str(".linux") {
                return 1;
            }

            -- 2. char methods.
            if 'A'.as_u32() != 65 or '😀'.as_u32() != 128512 {
                return 2;
            }
            if 'a'.len_utf8() != 1
                or 'é'.len_utf8() != 2
                or '中'.len_utf8() != 3
                or '😀'.len_utf8() != 4
            {
                return 3;
            }
            if not 'a'.is_ascii() or 'é'.is_ascii() or '中'.is_ascii() {
                return 4;
            }
            if not 'A'.is_ascii_alphabetic()
                or not 'z'.is_ascii_alphabetic()
                or '0'.is_ascii_alphabetic()
                or 'é'.is_ascii_alphabetic()
                or not 'A'.is_ascii_uppercase()
                or 'a'.is_ascii_uppercase()
                or not 'z'.is_ascii_lowercase()
                or 'Z'.is_ascii_lowercase()
            {
                return 5;
            }
            if not '0'.is_ascii_digit()
                or not '9'.is_ascii_digit()
                or 'a'.is_ascii_digit()
                or not '0'.is_ascii_hexdigit()
                or not 'f'.is_ascii_hexdigit()
                or not 'F'.is_ascii_hexdigit()
                or 'g'.is_ascii_hexdigit()
                or not 'k'.is_ascii_alphanumeric()
                or not '7'.is_ascii_alphanumeric()
                or '!'.is_ascii_alphanumeric()
            {
                return 6;
            }
            let ch_ff = char.from_u32(12) match {
                c? => c,
                null => return 7,
            };
            let ch_del = char.from_u32(127) match {
                c? => c,
                null => return 8,
            };
            if not ' '.is_ascii_whitespace()
                or not '\t'.is_ascii_whitespace()
                or not '\n'.is_ascii_whitespace()
                or not '\r'.is_ascii_whitespace()
                or not ch_ff.is_ascii_whitespace()
                or 'a'.is_ascii_whitespace()
            {
                return 9;
            }
            if not '\0'.is_ascii_control()
                or not '\n'.is_ascii_control()
                or not ch_del.is_ascii_control()
                or ' '.is_ascii_control()
                or not '!'.is_ascii_graphic()
                or not '~'.is_ascii_graphic()
                or ' '.is_ascii_graphic()
                or 'é'.is_ascii_graphic()
            {
                return 10;
            }
            if not '!'.is_ascii_punctuation()
                or not '/'.is_ascii_punctuation()
                or not ':'.is_ascii_punctuation()
                or not '@'.is_ascii_punctuation()
                or not '['.is_ascii_punctuation()
                or not '`'.is_ascii_punctuation()
                or not '{'.is_ascii_punctuation()
                or not '~'.is_ascii_punctuation()
                or 'a'.is_ascii_punctuation()
                or '0'.is_ascii_punctuation()
                or ' '.is_ascii_punctuation()
            {
                return 11;
            }
            if 'A'.to_ascii_lowercase() != 'a'
                or 'é'.to_ascii_lowercase() != 'é'
                or 'z'.to_ascii_uppercase() != 'Z'
                or '9'.to_ascii_uppercase() != '9'
                or not 'a'.eq_ignore_ascii_case('A')
                or 'a'.eq_ignore_ascii_case('b')
                or not 'é'.eq_ignore_ascii_case('é')
            {
                return 12;
            }

            -- 3. Runtime builtin.TextBytes methods on str.
            if not check_runtime_str("", "hello!", "ababa", "a中😀") {
                return 13;
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

    let case = NativeCase::new("stage2d-text-char", source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(unsigned-integer 0)\n");
}
