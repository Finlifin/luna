//! End-to-end verification for `^attrs.packed.byte` and `^attrs.packed.bit`.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct CliCase(PathBuf);

impl CliCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-packed-case-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn library_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
    }

    fn run_cli(&self, extra: &[&str]) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args(extra)
            .arg("--sysroot")
            .arg(Self::library_dir())
            .output()
            .unwrap()
    }

    fn dump_nir_with_target(&self) -> Output {
        self.run_cli(&[
            "dump",
            "main.fl",
            "--build-target",
            "x86_64",
            "linux",
            "64",
            "--emit",
            "nir",
        ])
    }

    fn build(&self, opt_level: &str) -> Output {
        self.run_cli(&[
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
        ])
    }

    fn emit_llvm(&self) -> Result<String, String> {
        let out_file = self.0.join("out.ll");
        let output = self.run_cli(&[
            "main.fl",
            "--emit-llvm",
            "run",
            "--output",
            "out.ll",
            "--fuel",
            "10000000",
        ]);
        if !output.status.success() {
            return Err(format!(
                "stdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        fs::read_to_string(out_file).map_err(|e| e.to_string())
    }

    fn run_binary(&self) -> Output {
        ProcessCommand::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for CliCase {
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
fn packed_byte_supports_layout_reflection_no_width_warning_and_native_execution() {
    let source = r#"
        ^attrs.packed.byte
        struct ByteWire {
            tag: u8,
            big_a: u128,
            big_b: u128,
            tail: u16,
        }

        impl Clone for ByteWire {
            fn clone(*self) -> Self {
                ByteWire {
                    tag: self.tag,
                    big_a: self.big_a,
                    big_b: self.big_b,
                    tail: self.tail,
                }
            }
        }

        impl Copy for ByteWire {}

        comptime fn inspect_byte() -> bool {
            ByteWire.is_packed()
                and (ByteWire.packed_mode() matches .byte?)
                and Type.size_of(ByteWire) == 35
                and Type.align_of(ByteWire) == 1
        }

        fn run() -> bool {
            var wire = ByteWire {
                tag: 7,
                big_a: 11,
                big_b: 29,
                tail: 0x1234,
            };
            let p: *ByteWire = wire.ref;
            p.tail.* = p.tail.* + 1;
            let direct_tail: u16 = p.tail;
            Type.size_of(ByteWire) == 35
                and Type.align_of(ByteWire) == 1
                and wire.tag == 7
                and wire.big_a == 11
                and wire.big_b == 29
                and p.tail.* == 0x1235
                and direct_tail == 0x1235
        }
    "#;

    let case = CliCase::new(source);
    // Byte-packed struct > 256 bits must never trigger a bit-width warning.
    let nir_out = case.dump_nir_with_target();
    assert_success(&nir_out);
    let stderr = String::from_utf8_lossy(&nir_out.stderr);
    assert!(
        !stderr.contains("W_PACKED_BIT_WIDTH"),
        "unexpected bit-width warning for attrs.packed.byte:\n{stderr}"
    );
    let stdout = String::from_utf8_lossy(&nir_out.stdout);
    assert!(stdout.contains("(packed byte)"), "{stdout}");

    assert_eq!(
        support::invoke(source, "inspect_byte").unwrap(),
        Value::Bool(true)
    );

    let llvm = case.emit_llvm().unwrap();
    assert!(llvm.contains("<{ i8, i128, i128, i16 }>"), "{llvm}");
    assert!(llvm.contains("align 1"), "{llvm}");

    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let out = case.run_binary();
        assert_success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }

    for bad in [
        "^attrs.packed.byte struct S { a: u8, b: u64 } fn bad() { var s = S { a: 1, b: 2 }; let _ = s.b.ref; }",
        "^attrs.packed.byte struct S { a: u8, b: u64 } fn bad(p: *S) -> *u64 { p.b }",
        "^attrs.packed.byte struct S { a: u8, b: u64 } fn bad() { var s = S { a: 1, b: 2 }; let p = s.ref; let _ = p.b.*.ref; }",
    ] {
        let err = support::compile(bad).unwrap_err();
        assert!(
            err.contains("cannot take an aligned pointer to a field of a packed struct"),
            "{bad}: {err}"
        );
    }
}

#[test]
fn packed_bit_layout_matches_bitvec_and_embeds_exact_bits_without_padding() {
    let source = r#"
        typealias Bits3 = BitVec<3>;
        typealias Bits4 = BitVec<4>;
        typealias Bits5 = BitVec<5>;
        typealias Bits0 = BitVec<0>;

        ^attrs.packed.bit
        struct EmptyBit {}

        ^attrs.packed.bit
        struct OneBool {
            flag: bool,
        }

        ^attrs.packed.bit
        struct NineBits {
            flag: bool,
            byte: u8,
        }

        ^attrs.packed.bit
        struct FlagsByte {
            ready: bool,
            priority: Bits3,
            channel: Bits4,
        }

        ^attrs.packed.bit
        struct Inner3 {
            flag: bool,
            code: BitVec<2>,
        }

        -- Nested packed.bit must pack exact 3 + 3 = 6 bits (1 byte), not 1 + 1 = 2 bytes.
        ^attrs.packed.bit
        struct NestedSix {
            left: Inner3,
            empty: EmptyBit,
            right: Inner3,
            unit: void,
        }

        -- Packed.byte holding packed.bit uses standalone BitVec<3> byte layout (1 byte).
        ^attrs.packed.byte
        struct ByteHoldingBit {
            tag: u8,
            inner: Inner3,
        }

        newtype WrapBits5 = Bits5;

        enum Mode5 {
            .width 5,
            .discrete_tag true,
            off: 0b00000,
            on: 0b00001,
        }

        ^attrs.packed.bit
        struct WrappedAndEnum {
            wrapped: WrapBits5,
            mode: Mode5,
        }

        ^attrs.packed.bit
        struct FloatsCharAndPtrInt {
            h: f16,
            b: bf16,
            f: f32,
            d: f64,
            ch: char,
            empty_bv: Bits0,
            u: usize,
            i: isize,
        }

        ^attrs.packed.bit
        struct Pair where T {
            lo: T,
            hi: T,
        }

        comptime fn check_all() -> bool {
            let reflect_ok = EmptyBit.is_packed()
                and (EmptyBit.packed_mode() matches .bit?)
                and (FlagsByte.packed_mode() matches .bit?)
                and (Pair<Bits3>.packed_mode() matches .bit?);

            let empty_ok = Type.size_of(EmptyBit) == 0
                and Type.align_of(EmptyBit) == 1;

            let one_bool_ok = Type.size_of(OneBool) == Type.size_of(BitVec<1>)
                and Type.align_of(OneBool) == Type.align_of(BitVec<1>)
                and Type.size_of(OneBool) == 1
                and Type.align_of(OneBool) == 1;

            let nine_ok = Type.size_of(NineBits) == Type.size_of(BitVec<9>)
                and Type.align_of(NineBits) == Type.align_of(BitVec<9>)
                and Type.size_of(NineBits) == 2
                and Type.align_of(NineBits) == 2;

            let byte_ok = Type.size_of(FlagsByte) == 1
                and Type.align_of(FlagsByte) == 1;

            let nested_ok = Type.size_of(NestedSix) == Type.size_of(BitVec<6>)
                and Type.align_of(NestedSix) == Type.align_of(BitVec<6>)
                and Type.size_of(NestedSix) == 1
                and Type.align_of(NestedSix) == 1;

            let byte_wrap_ok = Type.size_of(ByteHoldingBit) == 2
                and Type.align_of(ByteHoldingBit) == 1;

            let nt_enum_ok = Type.size_of(WrappedAndEnum) == Type.size_of(BitVec<10>)
                and Type.align_of(WrappedAndEnum) == Type.align_of(BitVec<10>)
                and Type.size_of(WrappedAndEnum) == 2
                and Type.align_of(WrappedAndEnum) == 2;

            -- 16 + 16 + 32 + 64 + 32 + 0 + 64 + 64 = 288 bits -> BitVec<288> (48 bytes, align 16)
            let wide_ok = Type.size_of(FloatsCharAndPtrInt) == Type.size_of(BitVec<288>)
                and Type.align_of(FloatsCharAndPtrInt) == Type.align_of(BitVec<288>);

            let generic_ok = Type.size_of(Pair<Bits3>) == Type.size_of(BitVec<6>)
                and Type.size_of(Pair<Bits3>) == 1
                and Type.size_of(Pair<Bits5>) == Type.size_of(BitVec<10>)
                and Type.size_of(Pair<Bits5>) == 2;

            reflect_ok
                and empty_ok
                and one_bool_ok
                and nine_ok
                and byte_ok
                and nested_ok
                and byte_wrap_ok
                and nt_enum_ok
                and wide_ok
                and generic_ok
        }
    "#;
    assert_eq!(
        support::invoke(source, "check_all").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn packed_bit_rejects_non_pod_fields_drop_cycles_and_field_pointers() {
    for (bad, expected) in [
        (
            "^attrs.packed.bit struct S { p: *u8 }",
            &["not allowed"][..],
        ),
        (
            "^attrs.packed.bit struct S { f: fn() -> void }",
            &["not allowed"],
        ),
        ("^attrs.packed.bit struct S { s: str }", &["not allowed"]),
        ("^attrs.packed.bit struct S { k: Symbol }", &["not allowed"]),
        (
            "struct Plain { x: u8 } ^attrs.packed.bit struct S { p: Plain }",
            &["not allowed"],
        ),
        (
            "^attrs.packed.byte struct Byte { x: u8 } ^attrs.packed.bit struct S { p: Byte }",
            &["not allowed"],
        ),
        (
            "^attrs.packed.bit struct S { t: Tuple<u8, bool> }",
            &["not allowed"],
        ),
        ("^attrs.packed.bit struct S { o: ?u8 }", &["not allowed"]),
        (
            "enum E {} ^attrs.packed.bit struct S { r: !E u8 }",
            &["not allowed"],
        ),
        (
            "enum UnitE { a, b } ^attrs.packed.bit struct S { e: UnitE }",
            &["not allowed"],
        ),
        (
            "enum TaggedE { a, b(u8) } ^attrs.packed.bit struct S { e: TaggedE }",
            &["not allowed"],
        ),
        (
            "^attrs.packed.bit struct S { x: u8 } impl Drop for S { fn drop(*self) {} }",
            &["Drop"],
        ),
        (
            "newtype OwnedByte = u8; impl Drop for OwnedByte { fn drop(*self) {} } ^attrs.packed.bit struct S { x: OwnedByte }",
            &["Drop"],
        ),
        (
            "^attrs.packed.bit struct S { a: u8 } fn f(s: *S) -> *u8 { s.a }",
            &["cannot take an aligned pointer to a field of a packed struct"],
        ),
        (
            "^attrs.packed.bit struct S { a: u8 } fn f() { var s = S { a: 1 }; let _ = s.a.ref; }",
            &["cannot take an aligned pointer to a field of a packed struct"],
        ),
        (
            "^attrs.packed.bit struct SelfCycle { next: SelfCycle }",
            &["cycle", "recursive"],
        ),
        (
            "^attrs.packed.bit struct CycleA { b: CycleB } ^attrs.packed.bit struct CycleB { a: CycleA }",
            &["cycle", "recursive"],
        ),
        (
            "^attrs.packed.bit struct BadGen where T { ptr: *T }",
            &["not allowed"],
        ),
        (
            "^attrs.packed.bit struct Gen where T { x: T } fn f(g: Gen<*u8>) -> bool { true }",
            &["not allowed"],
        ),
    ] {
        let err = support::compile(bad).unwrap_err();
        assert!(
            !err.contains("package resolution failed") && !err.contains("expected a block"),
            "negative case failed at parse/resolution instead of semantic check for `{bad}`:\n{err}"
        );
        assert!(
            expected.iter().any(|needle| err.contains(needle)),
            "unexpected error for `{bad}` (expected one of {expected:?}):\n{err}"
        );
    }

    // Hidden instantiation of an invalid generic `attrs.packed.bit` struct inside
    // a generic function body must also fail during concrete IIR materialization.
    let hidden_invalid = r#"
        ^attrs.packed.bit
        struct BoxedBits where T {
            item: T,
        }

        fn helper(x: ?T) -> usize where T {
            Type.size_of(BoxedBits<T>)
        }

        fn run() -> usize {
            let p: ?*u8 = null;
            helper(p)
        }
    "#;
    match support::compile(hidden_invalid) {
        Err(err) => {
            assert!(
                !err.contains("package resolution failed") && !err.contains("expected a block"),
                "hidden_invalid failed to parse:\n{err}"
            );
            assert!(
                err.contains("attrs.packed.bit") && err.contains("not allowed"),
                "unexpected compile error for hidden_invalid:\n{err}"
            );
        }
        Ok(nir) => {
            let err = luna::instantiation::instantiate(&nir, "run", 1_000_000)
                .expect_err("expected materialization rejection for BoxedBits<*u8>")
                .to_string();
            assert!(
                err.contains("attrs.packed.bit") && err.contains("not allowed"),
                "unexpected materialization error for BoxedBits<*u8>:\n{err}"
            );
        }
    }
}

#[test]
fn packed_bit_emits_deduplicated_warning_above_256_bits_without_failing_build() {
    // Exactly 256 bits -> no warning.
    let exact_256 = r#"
        ^attrs.packed.bit
        struct Exact256 {
            lo: u128,
            hi: u128,
        }
    "#;
    let case_256 = CliCase::new(exact_256);
    let out_256 = case_256.dump_nir_with_target();
    assert_success(&out_256);
    let stderr_256 = String::from_utf8_lossy(&out_256.stderr);
    assert!(
        !stderr_256.contains("W_PACKED_BIT_WIDTH"),
        "exact 256-bit struct must not emit width warning:\n{stderr_256}"
    );

    // 257 bits -> non-fatal warning naming struct, 257, 256, and attrs.packed.byte; exit 0.
    let over_256 = r#"
        ^attrs.packed.bit
        struct Exact257 {
            lo: u128,
            hi: u128,
            extra: bool,
        }
    "#;
    let case_257 = CliCase::new(over_256);
    let out_257 = case_257.dump_nir_with_target();
    assert_success(&out_257);
    let stderr_257 = String::from_utf8_lossy(&out_257.stderr);
    assert!(stderr_257.contains("W_PACKED_BIT_WIDTH"), "{stderr_257}");
    assert!(stderr_257.contains("Exact257"), "{stderr_257}");
    assert!(stderr_257.contains("257"), "{stderr_257}");
    assert!(stderr_257.contains("256"), "{stderr_257}");
    assert!(stderr_257.contains("attrs.packed.byte"), "{stderr_257}");

    // Generic Big<u64> (129 bits) -> no warning.
    let generic_under = r#"
        ^attrs.packed.bit
        struct Big where T {
            a: T,
            b: T,
            extra: bool,
        }

        fn measure() -> usize {
            Type.size_of(Big<u64>)
        }
    "#;
    let case_gen_ok = CliCase::new(generic_under);
    let out_gen_ok = case_gen_ok.dump_nir_with_target();
    assert_success(&out_gen_ok);
    let stderr_gen_ok = String::from_utf8_lossy(&out_gen_ok.stderr);
    assert!(
        !stderr_gen_ok.contains("W_PACKED_BIT_WIDTH"),
        "129-bit generic instance must not warn:\n{stderr_gen_ok}"
    );

    // Generic Big<u128> (257 bits) referenced repeatedly -> deduplicated to 1 warning.
    let generic_over = r#"
        ^attrs.packed.bit
        struct Big where T {
            a: T,
            b: T,
            extra: bool,
        }

        fn use1(x: Big<u128>) -> bool { true }
        fn use2(x: Big<u128>) -> usize { Type.size_of(Big<u128>) }
        fn use3() -> bool {
            let v = Big<u128> { a: 1, b: 2, extra: true };
            let _ = v;
            true
        }
    "#;
    let case_gen_warn = CliCase::new(generic_over);
    let out_gen_warn = case_gen_warn.dump_nir_with_target();
    assert_success(&out_gen_warn);
    let stderr_gen_warn = String::from_utf8_lossy(&out_gen_warn.stderr);
    assert_eq!(
        stderr_gen_warn.matches("W_PACKED_BIT_WIDTH").count(),
        1,
        "expected exactly one deduplicated warning for Big<u128>:\n{stderr_gen_warn}"
    );
    assert!(stderr_gen_warn.contains("257"), "{stderr_gen_warn}");
    assert!(stderr_gen_warn.contains("Big<"), "{stderr_gen_warn}");
}
