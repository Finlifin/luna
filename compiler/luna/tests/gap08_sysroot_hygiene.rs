//! Regression suite for Milestone C (`DIV-08-01`, `DIV-08-06`, `DIV-08-08..10`,
//! `SND-08-03..04`, `ROB-08-01`, `GAP-08-37`):
//! - Runtime `builtin.fmt.Display` vs comptime `meta.ComptimeDisplay` split
//! - `ByteWriter.write_str` / `write_raw_bytes` error propagation
//! - Private `ProbeWriter` / `ProbeErr` in `builtin.fmt.options`
//! - `^attrs.compilation.intrinsic_enum` elaboration handling (and no bare `^intrinsic_enum` bypass)
//! - Removal of `meta.ast.Optional`, re-export of `meta.macros.{ast, peg}`, and `build.prelude`
//! - Omitting optional `list_pattern_view_rest` on `ComptimeListPatternView` / `ListPatternView`
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

#[test]
fn comptime_display_and_runtime_display_are_distinct_same_named_protocols() {
    let comptime_only = r#"
        use meta.ComptimeDisplay;
        use std.fmt.comptime_format;

        struct ComptimeBadge {
            id: u16,
        }

        comptime impl ComptimeDisplay for ComptimeBadge {
            comptime fn fmt(self, .format: str = "") -> str {
                if format == "style: .hex" {
                    "badge#".as(meta.ComptimeTextBytes).concat(
                        self.id.as(ComptimeDisplay).fmt(.format = "format: .HEX"),
                    )
                } else {
                    "badge:".as(meta.ComptimeTextBytes).concat(self.id.as(ComptimeDisplay).fmt())
                }
            }
        }

        comptime fn run() -> bool {
            let b = ComptimeBadge { id: 42 };
            let big: Integer = 100000000000000000000 + 7;
            let s = comptime_format("{} | {style: .hex} | {format: .hex, alt: true}", b, b, big);
            s == "badge:42 | badge#2A | 0x56bc75e2d63100007"
        }
    "#;
    assert_eq!(
        support::invoke(comptime_only, "run").unwrap(),
        Value::Bool(true)
    );

    // `Display` no longer declares a `comptime_fmt` method.
    let legacy_display_comptime = r#"
        struct Legacy { x: u8 }
        comptime impl builtin.fmt.Display for Legacy {
            comptime fn comptime_fmt(self, .format: str = "") -> str { "legacy" }
        }
        fn run() -> bool { true }
    "#;
    assert!(support::compile(legacy_display_comptime).is_err());

    // Passing a comptime-only ComptimeDisplay type to runtime `std.fmt.write_fmt` must fail.
    let comptime_only_in_runtime_fmt = r#"
        use meta.ComptimeDisplay;
        use builtin.fmt.ByteWriter;

        struct OnlyCt { x: u8 }
        comptime impl ComptimeDisplay for OnlyCt {
            comptime fn fmt(self, .format: str = "") -> str { "ct" }
        }

        enum NoErr { Boom }
        struct Sink {}
        impl ByteWriter for Sink {
            assoc Err: Type = NoErr;
            fn write_byte(*self, byte: u8) -> !NoErr void {}
        }

        fn run() -> !NoErr void {
            var s = Sink {};
            std.fmt.write_fmt(s.ref, "{}", OnlyCt { x: 1 })
        }
    "#;
    assert!(support::compile(comptime_only_in_runtime_fmt).is_err());
}

#[test]
fn byte_writer_propagates_errors_in_write_str_and_write_raw_bytes() {
    let source = r#"
        use builtin.fmt.ByteWriter;

        enum SinkErr {
            Full,
        }

        struct LimitedSink {
            bytes: u64,
            len: u8,
            limit: u8,
        }

        impl LimitedSink {
            fn new(limit: u8) -> Self {
                LimitedSink {
                    bytes: 0,
                    len: 0,
                    limit: limit,
                }
            }
        }

        impl ByteWriter for LimitedSink {
            assoc Err: Type = SinkErr;

            fn write_byte(*self, byte: u8) -> !SinkErr void {
                let cur = self.len.*;
                if cur >= self.limit.* {
                    return error SinkErr.Full;
                }
                assumes cur < 8;
                assumes self.bytes.* <= 72057594037927935;
                self.bytes.* = self.bytes.* * 256 + byte'bitvec.as(u64);
                self.len.* = cur + 1;
            }
        }

        fn run() -> bool {
            var s1 = LimitedSink.new(2);
            let str_res = s1.ref.write_str("abcd");
            let str_failed = str_res matches error SinkErr.Full;
            let str_prefix_ok = s1.len == 2 and s1.bytes == 97 * 256 + 98;

            var s2 = LimitedSink.new(4);
            let n: u8 = 7;
            let ok_res = std.fmt.write_fmt(s2.ref, "{align_to: .right, len: 4, placeholder: '0'}", n);
            let fmt_ok = ok_res matches _!;
            let fmt_bytes_ok = s2.len == 4 and s2.bytes == ((48 * 256 + 48) * 256 + 48) * 256 + 55;

            str_failed and str_prefix_ok and fmt_ok and fmt_bytes_ok
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));

    // Also test native execution of `ByteWriter.write_raw_bytes` error propagation.
    let raw_case_dir =
        std::env::temp_dir().join(format!("luna-gap08-raw-bytes-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&raw_case_dir);
    let raw_source = r#"
        use builtin.fmt.ByteWriter;

        enum SinkErr {
            Full,
        }

        struct LimitedSink {
            bytes: u64,
            len: u8,
            limit: u8,
        }

        impl LimitedSink {
            fn new(limit: u8) -> Self {
                LimitedSink {
                    bytes: 0,
                    len: 0,
                    limit: limit,
                }
            }
        }

        impl ByteWriter for LimitedSink {
            assoc Err: Type = SinkErr;

            fn write_byte(*self, byte: u8) -> !SinkErr void {
                let cur = self.len.*;
                if cur >= self.limit.* {
                    return error SinkErr.Full;
                }
                assumes cur < 8;
                assumes self.bytes.* <= 72057594037927935;
                self.bytes.* = self.bytes.* * 256 + byte'bitvec.as(u64);
                self.len.* = cur + 1;
            }
        }

        fn run() -> bool {
            var s3 = LimitedSink.new(1);
            let raw_src: u32 = 0x434241;
            let raw_res = unsafe {
                s3.ref.write_raw_bytes(builtin.mem.ptr_addr(raw_src.ref), 3)
            };
            let raw_failed = raw_res matches error SinkErr.Full;
            let raw_prefix_ok = s3.len == 1 and s3.bytes == 65;
            raw_failed and raw_prefix_ok
        }
    "#;
    std::fs::write(raw_case_dir.join("main.fl"), raw_source).unwrap();
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let build = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&raw_case_dir)
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
    let exec = std::process::Command::new(raw_case_dir.join("program"))
        .current_dir(&raw_case_dir)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&raw_case_dir);
    assert!(
        exec.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&exec.stdout),
        String::from_utf8_lossy(&exec.stderr)
    );
    assert_eq!(exec.stdout, b"(bool true)\n");
}

#[test]
fn probe_writer_and_probe_err_are_private_in_builtin_fmt_options() {
    let use_writer = r#"
        use builtin.fmt.options.ProbeWriter;
        fn run() -> bool { true }
    "#;
    assert!(support::compile(use_writer).is_err());

    let use_err = r#"
        use builtin.fmt.options.ProbeErr;
        fn run() -> bool { true }
    "#;
    assert!(support::compile(use_err).is_err());
}

#[test]
fn intrinsic_enum_requires_attrs_compilation_attribute_not_lowering_bypass() {
    let valid = r#"
        ^attrs.compilation.intrinsic_enum
        enum TargetOps {
            fma,
        }
        fn run() -> bool { true }
    "#;
    let package = support::compile(valid).unwrap();
    let target_ops = package
        .enums
        .iter()
        .find(|e| e.declaration.package == 0 && e.name == "TargetOps")
        .expect("TargetOps enum");
    assert!(target_ops.is_intrinsic);

    // Bare `^intrinsic_enum` must no longer be intercepted by AST lowering.
    let bare = r#"
        ^intrinsic_enum
        enum LegacyOps { fma }
        fn run() -> bool { true }
    "#;
    assert!(support::compile(bare).is_err());

    // Applying `^attrs.compilation.intrinsic_enum` to a struct must fail cleanly.
    let non_enum = r#"
        ^attrs.compilation.intrinsic_enum
        struct NotAnEnum { x: u8 }
        fn run() -> bool { true }
    "#;
    let err = support::compile(non_enum).unwrap_err();
    assert!(
        err.contains("__flurry_intrinsic_enum requires an enum declaration"),
        "{err}"
    );
}

#[test]
fn meta_optional_removed_macros_ast_peg_and_build_prelude_exported() {
    for legacy_import in [
        "use meta.ast.Optional; fn run() -> bool { true }",
        "use meta.macros.Optional; fn run() -> bool { true }",
        "use meta.Optional; fn run() -> bool { true }",
    ] {
        assert!(
            support::compile(legacy_import).is_err(),
            "expected removal error for `{legacy_import}`"
        );
    }

    let ok_source = r#"
        use meta.macros.{ast, peg};
        use build.prelude.*;

        ^ast(false)
        comptime fn identity_macro(expr: meta.ast.FlurryExpr) -> meta.ast.TokenBuffer {
            expr.to_tokens()
        }

        comptime fn run() -> bool {
            let cur_target: Target = target;
            let _features: TargetFeatures = cur_target.features;
            let _ptr_bits: Integer = cur_target.pointer_bits;
            _ptr_bits == 64
        }
    "#;
    assert_eq!(
        support::invoke(ok_source, "run").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_list_pattern_view_omits_rest_when_unused_and_rejects_named_rest() {
    let wild_ok = r#"
        use meta.ComptimeListPatternView;

        struct Pair {
            a: Integer,
            b: Integer,
        }

        comptime impl ComptimeListPatternView for Pair {
            assoc Element: Type = Integer;

            comptime fn list_pattern_view_get_len(self) -> usize { 2 }
            comptime fn list_pattern_view_get_element(self, index: usize) -> Integer {
                if index == 0 { self.a } else { self.b }
            }
        }

        comptime fn run() -> bool {
            let p = Pair { a: 10, b: 20 };
            p match {
                [first, ..._] => first == 10,
                _ => false,
            }
        }
    "#;
    assert_eq!(support::invoke(wild_ok, "run").unwrap(), Value::Bool(true));

    let named_rest_err = format!(
        "{wild_ok}
        comptime fn bad(p: Pair) -> bool {{
            p match {{
                [_, ...rest] => rest.a == 10,
                _ => false,
            }}
        }}"
    );
    let err = support::compile(&named_rest_err).unwrap_err();
    assert!(
        err.contains("without overriding list_pattern_view_rest required for '...rest' binding"),
        "{err}"
    );
}
