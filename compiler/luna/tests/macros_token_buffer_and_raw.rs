//! End-to-end tests for `{{ ... }}` (`TokenBuffer`), `ToTokens`, `@use`, and `@builtin.str.raw`.
use comptime_interpretor::Value;
use luna::cli::{self, Command};
use luna::frontend;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_path(prefix: &str, suffix: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{nanos}-{seq}{suffix}",
        std::process::id()
    ))
}

fn library_dir() -> PathBuf {
    if let Ok(path) = std::env::var("LUNA_TEST_SYSROOT") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

fn invoke_sysroot_comptime(source: &str, entry: &str) -> Result<Value, String> {
    let path = unique_temp_path("luna-macro-tb-ct", ".fl");
    std::fs::write(&path, source).unwrap();
    let library = library_dir();
    let Command::Check(options) = cli::parse([
        "check".into(),
        path.clone().into_os_string(),
        "--sysroot".into(),
        library.into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("expected check options")
    };
    let result = frontend::invoke_comptime(&options, entry, 20_000_000, None, None);
    let _ = std::fs::remove_file(&path);
    result
}

#[test]
fn token_buffer_literals_and_to_tokens_protocol_work_at_comptime() {
    let source = r#"
        use meta.ast.{Token, TokenBuffer};

        comptime fn inspect_buffer() -> bool {
            let empty = {{}};
            if not empty.is_empty() or empty.length() != 0 {
                return false;
            }

            let op_buf = {{ 1 + 2 $$ }};
            let op_ok = op_buf.length() == 4
                and (op_buf.get(0) match {
                    tok? => tok.kind == .int and tok.text == "1",
                    null => false,
                })
                and (op_buf.get(1) match {
                    tok? => tok.kind == .separated_plus and tok.text == "+",
                    null => false,
                })
                and (op_buf.get(3) match {
                    tok? => tok.kind == .dollar and tok.text == "$",
                    null => false,
                });
            if not op_ok {
                return false;
            }

            let name = Token.ident("answer");
            let count: Integer = -42;
            let flag: bool = true;
            let text: str = "hi \$x";
            let sym: Symbol = .ready;
            let items: List<Integer> = [7, 8];
            let maybe_some: ?str = "present";
            let maybe_none: ?str = null;

            let built = {{
                let $name = $(count);
                let ok = $flag;
                let msg = ${text};
                let tag = $sym;
                let xs = $items;
                let some_v = $maybe_some;
                let none_v = $maybe_none;
            }};

            -- Inspect negative integer tokens (minus joint + magnitude), symbol (. + id),
            -- string literal ($ escaped), and value-reference mutation via push/append.
            var has_minus = false;
            var has_42 = false;
            var has_dot = false;
            var has_ready = false;
            let list = built.to_list();
            var i: Integer = 0;
            while i < list.length() {
                let t = list[i];
                if t.kind == .minus and t.spacing == .joint {
                    has_minus = true;
                }
                if t.kind == .int and t.text == "42" {
                    has_42 = true;
                }
                if t.kind == .dot and t.spacing == .joint {
                    has_dot = true;
                }
                if t.kind == .id and t.text == "ready" {
                    has_ready = true;
                }
                i = i + 1;
            }
            if not (has_minus and has_42 and has_dot and has_ready) {
                return false;
            }

            var mut_buf = {{ let a }};
            let alias_snapshot = mut_buf;
            mut_buf.push(Token.punct(.eq, "="));
            mut_buf.append({{ 99; }});

            let joined = TokenBuffer.join(
                [{{ "a" }}, {{ "b" }}, {{ "c" }}],
                {{ or }},
            );

            let nested = {{ if cond { 1 } else { 2 }}};

            mut_buf.length() == 5
                and alias_snapshot.length() == 2
                and joined.length() == 5
                and nested.length() == 9
        }
    "#;
    assert_eq!(
        invoke_sysroot_comptime(source, "inspect_buffer").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn token_buffer_rejects_interpolated_value_without_to_tokens_impl() {
    let source = r#"
        struct NotConvertible {
            value: Integer,
        }

        comptime fn bad() -> meta.ast.TokenBuffer {
            let x = NotConvertible { value: 1 };
            {{ $x }}
        }
    "#;
    let err = invoke_sysroot_comptime(source, "bad").unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn builtin_str_raw_matches_exact_bytes_in_comptime_and_native_executable() {
    let dir = unique_temp_path("luna-macro-raw-native", "");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    let main_fl = dir.join("main.fl");

    let source = r#"
        use std.prelude.*;
        @use builtin.str.raw;
        @use builtin.str.raw as raw_text;
        @use builtin.str.*;

        const SAMPLE = @builtin.str.raw {
    name = sample
    description = punctuation: $ @ ^
};

        const UNESCAPED_AND_BALANCED = @raw_text { quote: " esc: \bad comment: {- not comment } brace: { ok } utf8: 你好 🦀 };
        const EMPTY = @raw {};

        comptime fn check_comptime() -> bool {
            use meta.ComptimeTextBytes;
            let sample_ok = SAMPLE.contains("punctuation: $ @ ^") and SAMPLE.contains("name = sample");
            let unesc_ok = UNESCAPED_AND_BALANCED.contains("quote: \" esc: \\bad")
                and UNESCAPED_AND_BALANCED.contains("{ ok }")
                and UNESCAPED_AND_BALANCED.contains("你好 🦀");
            sample_ok and unesc_ok and EMPTY == ""
        }
    "#;
    std::fs::write(&main_fl, source).unwrap();

    // First verify comptime string values directly.
    assert_eq!(
        invoke_sysroot_comptime(source, "check_comptime").unwrap(),
        Value::Bool(true)
    );

    // Next build and execute native binary with runtime printing of raw string + `@use`.
    let runtime_src = r#"
        use std.prelude.*;
        @use builtin.str.raw as raw_text;

        define emit_check(name, expected) {
            fn $name(actual: str) -> bool {
                actual == $expected
            }
        }

        $emit_check(matches_raw, "line1: \$ @ ^\nline2: \" {ok}")

        fn run() -> bool {
            let raw_msg = @raw_text {line1: $ @ ^
line2: " {ok}};
            let printed = println("{}", raw_msg) match {
                _! => true,
                error _ => false,
            };
            printed and matches_raw(raw_msg)
        }
    "#;
    std::fs::write(&main_fl, runtime_src).unwrap();

    let library = library_dir();
    let build = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&dir)
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
        .arg(&library)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let run = ProcessCommand::new(dir.join("program"))
        .current_dir(&dir)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(run.status.success(), "{run:?}");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(stdout, "line1: $ @ ^\nline2: \" {ok}\n(bool true)\n");
}

#[test]
fn macro_imports_and_use_at_pkg_coexist_and_reject_bad_delimiters() {
    let ok = invoke_sysroot_comptime(
        r#"
        @use builtin.str.{raw as r}
        use meta.List;

        mod local {
            pub comptime fn one() -> Integer {
                1
            }
        }
        use @local.one;

        comptime fn check() -> bool {
            let s = @r { ok $1 };
            let xs: List<str> = [s];
            xs.length() == one() and s == " ok $1 "
        }
        "#,
        "check",
    )
    .unwrap();
    assert_eq!(ok, Value::Bool(true));

    let bad_delim = invoke_sysroot_comptime(
        r#"
        comptime fn bad() -> str {
            @builtin.str.raw(123)
        }
        "#,
        "bad",
    )
    .unwrap_err();
    assert!(
        bad_delim.contains("brace delimiters") || bad_delim.contains("4011"),
        "{bad_delim}"
    );

    let bad_unclosed = invoke_sysroot_comptime(
        r#"
        comptime fn bad() -> str {
            @builtin.str.raw { { unclosed
        }
        "#,
        "bad",
    )
    .unwrap_err();
    assert!(
        bad_unclosed.contains("unterminated raw macro input") || bad_unclosed.contains("4018"),
        "{bad_unclosed}"
    );

    let bad_undef = invoke_sysroot_comptime(
        r#"
        comptime fn bad() -> str {
            @does.not.exist { hi }
        }
        "#,
        "bad",
    )
    .unwrap_err();
    assert!(
        bad_undef.contains("undefined macro") || bad_undef.contains("4008"),
        "{bad_undef}"
    );
}
