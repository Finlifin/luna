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
        let bin = std::env::var_os("CARGO_BIN_EXE_luna")
            .unwrap_or_else(|| std::ffi::OsString::from(env!("CARGO_BIN_EXE_luna")));
        ProcessCommand::new(bin)
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

    fn dump_llvm_o0(&self) -> Result<String, String> {
        let output = self.run_cli(&[
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
        ]);
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(format!(
                "stdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ))
        }
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

#[test]
fn packed_bit_construction_order_defaults_and_all_pod_field_types_execute_at_o0_and_o3() {
    let source = r#"
        enum Exhaustive2 {
            .width 2,
            .discrete_tag true,
            z0: 0b00,
            z1: 0b01,
            z2: 0b10,
            z3: 0b11,
        }

        enum Sparse5 {
            .width 5,
            .discrete_tag true,
            off: 0b00001,
            on: 0b10101,
        }

        newtype Tag5 = BitVec<5>;

        ^attrs.packed.bit
        struct EmptyBit {}

        ^attrs.packed.bit
        struct Inner5 {
            flag: bool,
            nibble: BitVec<4>,
        }

        ^attrs.packed.bit
        struct OrderedFields {
            first: u8,
            second: u16 = bump_default(),
            third: u8,
        }

        fn step(log: *u64, tag: u64, val: u8) -> u8 {
            let cur = log.*;
            if cur <= 1_000_000 and tag <= 9 {
                log.* = cur * 10 + tag;
            }
            val
        }

        fn step16(log: *u64, tag: u64, val: u16) -> u16 {
            let cur = log.*;
            if cur <= 1_000_000 and tag <= 9 {
                log.* = cur * 10 + tag;
            }
            val
        }

        fn default_mid(log: *u64) -> u16 {
            let cur = log.*;
            if cur <= 1_000_000 {
                log.* = cur * 10 + 9;
            }
            0x2222
        }

        ^attrs.packed.bit
        struct DefaultAndOrder {
            a: u8,
            b: u16 = 0x1234,
            c: u8,
        }

        ^attrs.packed.bit
        struct OrderProbe {
            first: u8,
            mid: u16,
            last: u8,
        }

        fn build_with_default(log: *u64, use_default: bool) -> OrderProbe {
            -- Avoid pure literal folding and test runtime named-order evaluation.
            OrderProbe {
                last: step(log, 3, 33),
                mid: if use_default { default_mid(log) } else { step16(log, 2, 22) },
                first: step(log, 1, 11),
            }
        }

        ^attrs.packed.bit
        struct AllPod {
            b_true: bool,
            b_false: bool,
            vu8: u8,
            vi8: i8,
            vu16: u16,
            vi16: i16,
            vu32: u32,
            vi32: i32,
            vu64: u64,
            vi64: i64,
            vu128: u128,
            vi128: i128,
            vusize: usize,
            visize: isize,
            bv0: BitVec<0>,
            bv4: BitVec<4>,
            bv13: BitVec<13>,
            vf16: f16,
            vbf16: bf16,
            vf32: f32,
            vf64: f64,
            vch: char,
            ex: Exhaustive2,
            sp: Sparse5,
            nt: Tag5,
            nested: Inner5,
            vvoid: void,
            empty: EmptyBit,
        }

        fn bump_default() -> u16 { 77 }

        fn make_default_and_order() -> DefaultAndOrder {
            DefaultAndOrder { c: 0xAB, a: 0x55 }
        }

        fn check_order_and_defaults() -> bool {
            var log1: u64 = 0;
            let p1 = build_with_default(log1.ref, true);
            var log2: u64 = 0;
            let p2 = build_with_default(log2.ref, false);
            let d = make_default_and_order();
            let o = OrderedFields { third: 3, first: 1 };
            -- Expected declaration order: first (1) -> mid (9 or 2) -> last (3).
            log1 == 193
                and log2 == 123
                and p1.first == 11
                and p1.mid == 0x2222
                and p1.last == 33
                and p2.first == 11
                and p2.mid == 22
                and p2.last == 33
                and d.a == 0x55
                and d.b == 0x1234
                and d.c == 0xAB
                and o.first == 1
                and o.second == 77
                and o.third == 3
        }

        fn check_all_pod() -> bool {
            let f16_bits: u16 = 0x3C00;
            let bf16_bits: u16 = 0x4049;
            let f32_bits: u32 = 0xC0490FDB;
            let f64_bits: u64 = 0x400921FB54442D18;
            let empty_in = EmptyBit {};
            let nt_bits: BitVec<5> = 0b11010;
            let val = AllPod {
                empty: empty_in,
                vvoid: (),
                nested: Inner5 { flag: true, nibble: 0b1011 },
                nt: nt_bits.as(Tag5),
                sp: Sparse5.on,
                ex: Exhaustive2.z2,
                vch: 'λ',
                vf64: f64_bits'bitvec.as(f64),
                vf32: f32_bits'bitvec.as(f32),
                vbf16: bf16_bits'bitvec.as(bf16),
                vf16: f16_bits'bitvec.as(f16),
                bv13: 0x1ABC,
                bv4: 0b0110,
                bv0: 0,
                visize: -1234567,
                vusize: 987654321,
                vi128: -17014118346046923173168730371588410572,
                vu128: 0xFEDCBA98765432100123456789ABCDEF,
                vi64: -9223372036854775000,
                vu64: 0x0123456789ABCDEF,
                vi32: -123456789,
                vu32: 0xDEADBEEF,
                vi16: -32000,
                vu16: 0xBEEF,
                vi8: -77,
                vu8: 0xA5,
                b_false: false,
                b_true: true,
            };

            let nested_ok = val.nested.flag == true and val.nested.nibble == 0b1011;
            let sp_ok = val.sp match { Sparse5.on => true, _ => false };
            let ex_ok = val.ex match { Exhaustive2.z2 => true, _ => false };
            val.b_true == true
                and val.b_false == false
                and val.vu8 == 0xA5
                and val.vi8 == -77
                and val.vu16 == 0xBEEF
                and val.vi16 == -32000
                and val.vu32 == 0xDEADBEEF
                and val.vi32 == -123456789
                and val.vu64 == 0x0123456789ABCDEF
                and val.vi64 == -9223372036854775000
                and val.vu128 == 0xFEDCBA98765432100123456789ABCDEF
                and val.vi128 == -17014118346046923173168730371588410572
                and val.vusize == 987654321
                and val.visize == -1234567
                and val.bv0 == 0
                and val.bv4 == 0b0110
                and val.bv13 == 0x1ABC
                and val.vf16'bitvec.as(u16) == f16_bits
                and val.vbf16'bitvec.as(u16) == bf16_bits
                and val.vf32'bitvec.as(u32) == f32_bits
                and val.vf64'bitvec.as(u64) == f64_bits
                and val.vch == 'λ'
                and ex_ok
                and sp_ok
                and val.nt.as(BitVec<5>) == 0b11010
                and nested_ok
        }

        fn run() -> bool {
            check_order_and_defaults() and check_all_pod()
        }
    "#;

    let case = CliCase::new(source);
    let _llvm = case.dump_llvm_o0().unwrap();
    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let out = case.run_binary();
        assert_success(&out);
        assert_eq!(out.stdout, b"(bool true)\n", "opt={opt}");
    }
}

#[test]
fn packed_bit_bitvec_view_and_safe_vs_unsafe_cast_semantics() {
    let source = r#"
        enum Exhaustive2 {
            .width 2,
            .discrete_tag true,
            a: 0b00,
            b: 0b01,
            c: 0b10,
            d: 0b11,
        }

        enum Sparse4 {
            .width 4,
            .discrete_tag true,
            low: 0b0010,
            high: 0b1101,
        }

        ^attrs.packed.bit
        struct Inner3 {
            flag: bool,
            ex: Exhaustive2,
        }

        ^attrs.packed.bit
        struct FlagsByte {
            ready: bool,
            priority: BitVec<3>,
            channel: BitVec<4>,
        }

        impl Clone for FlagsByte {
            fn clone(*self) -> Self {
                FlagsByte {
                    ready: self.ready,
                    priority: self.priority,
                    channel: self.channel,
                }
            }
        }
        impl Copy for FlagsByte {}

        ^attrs.packed.bit
        struct UnrestrictedMix {
            inner: Inner3,
            signed_nib: i8,
            half: f16,
            empty: void,
        }

        ^attrs.packed.bit
        struct RestrictedChar {
            tag: BitVec<4>,
            ch: char,
        }

        ^attrs.packed.bit
        struct RestrictedNested {
            lead: bool,
            inner: RestrictedChar,
        }

        ^attrs.packed.bit
        struct RestrictedEnum {
            kind: Sparse4,
            pad: BitVec<4>,
        }

        newtype WrapChar = char;

        ^attrs.packed.bit
        struct RestrictedNewtype {
            c: WrapChar,
        }

        unsafe fn cast_via_unsafe_fn(bits: BitVec<36>) -> RestrictedChar {
            bits.as(RestrictedChar)
        }

        fn check_safe_and_lsb() -> bool {
            let f = FlagsByte {
                ready: true,
                priority: 0b101,
                channel: 0b1100,
            };
            -- LSB-first: bit 0 = 1, bits 1..4 = 0b101, bits 4..8 = 0b1100 => 0b1100_101_1 = 0xCB.
            let raw: BitVec<8> = f'bitvec;
            let f2 = raw.as(FlagsByte);

            let h_raw: u16 = 0x3E00;
            let neg_s: i8 = -19;
            let um = UnrestrictedMix {
                inner: Inner3 { flag: true, ex: Exhaustive2.c },
                signed_nib: neg_s,
                half: h_raw'bitvec.as(f16),
                empty: (),
            };
            let um_bits: BitVec<27> = um'bitvec;
            let um2 = um_bits.as(UnrestrictedMix);
            let ex_ok = um2.inner.ex match { Exhaustive2.c => true, _ => false };

            raw == 0b1100_101_1
                and f2.ready == true
                and f2.priority == 0b101
                and f2.channel == 0b1100
                and um2.inner.flag == true
                and ex_ok
                and um2.signed_nib == -19
                and um2.half'bitvec.as(u16) == h_raw
        }

        fn check_unsafe_casts() -> bool {
            let rc = RestrictedChar { tag: 0b1010, ch: 'A' };
            let rc_bits: BitVec<36> = rc'bitvec;
            let rc_block = unsafe { rc_bits.as(RestrictedChar) };
            let rc_fn = unsafe { cast_via_unsafe_fn(rc_bits) };

            let rn = RestrictedNested {
                lead: true,
                inner: RestrictedChar { tag: 0b1010, ch: 'A' },
            };
            let rn_bits: BitVec<37> = rn'bitvec;
            let rn2 = unsafe { rn_bits.as(RestrictedNested) };

            let re = RestrictedEnum { kind: Sparse4.high, pad: 0b0110 };
            let re_bits: BitVec<8> = re'bitvec;
            let re2 = unsafe { re_bits.as(RestrictedEnum) };
            let re_ok = re2.kind match { Sparse4.high => true, _ => false };

            let rnt = RestrictedNewtype { c: 'Z'.as(WrapChar) };
            let rnt_bits: BitVec<32> = rnt'bitvec;
            let rnt2 = unsafe { rnt_bits.as(RestrictedNewtype) };

            rc_block.tag == 0b1010
                and rc_block.ch == 'A'
                and rc_fn.tag == 0b1010
                and rc_fn.ch == 'A'
                and rn2.lead == true
                and rn2.inner.ch == 'A'
                and re_ok
                and re2.pad == 0b0110
                and rnt2.c.as(char) == 'Z'
        }

        fn run() -> bool {
            check_safe_and_lsb() and check_unsafe_casts()
        }
    "#;

    let case = CliCase::new(source);
    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let out = case.run_binary();
        assert_success(&out);
        assert_eq!(out.stdout, b"(bool true)\n", "opt={opt}");
    }

    for bad in [
        "^attrs.packed.bit struct S { ch: char } fn bad(b: BitVec<32>) -> S { b.as(S) }",
        "enum Sp { .width 4, .discrete_tag true, a: 0b0001 } ^attrs.packed.bit struct S { s: Sp } fn bad(b: BitVec<4>) -> S { b.as(S) }",
        "^attrs.packed.bit struct Inner { ch: char } ^attrs.packed.bit struct Outer { i: Inner } fn bad(b: BitVec<32>) -> Outer { b.as(Outer) }",
        "newtype W = char; ^attrs.packed.bit struct S { w: W } fn bad(b: BitVec<32>) -> S { b.as(S) }",
        "^attrs.packed.bit struct S { ch: char } fn bad(b: BitVec<32>) -> S { let f = unsafe { || { b.as(S) } }; f() }",
    ] {
        let err = support::compile(bad).unwrap_err();
        assert!(
            err.contains("unsafe"),
            "expected unsafe rejection for `{bad}`:\n{err}"
        );
    }

    let wrong_width = "^attrs.packed.bit struct S { a: bool, b: BitVec<7> } fn bad(b: BitVec<7>) -> S { b.as(S) }";
    let width_err = support::compile(wrong_width).unwrap_err();
    assert!(
        width_err.contains("width") || width_err.contains("BitVec<8>"),
        "expected width mismatch error:\n{width_err}"
    );

    let generic_concrete_restricted = r#"
        ^attrs.packed.bit
        struct Wrap where T {
            item: T,
        }

        fn bad_wrap(b: BitVec<32>) -> Wrap<char> {
            b.as(Wrap<char>)
        }
    "#;
    let err = support::compile(generic_concrete_restricted).unwrap_err();
    assert!(
        err.contains("unsafe"),
        "expected unsafe error for Wrap<char> cast:\n{err}"
    );

    let generic_abstract = r#"
        ^attrs.packed.bit
        struct Wrap where T {
            item: T,
        }

        fn cast_gen(b: BitVec<32>) -> Wrap<T> where T {
            b.as(Wrap<T>)
        }
    "#;
    let err = support::compile(generic_abstract).unwrap_err();
    assert!(
        err.contains("concrete type arguments"),
        "expected concrete type arguments error for generic Wrap<T> cast:\n{err}"
    );
}

#[test]
fn packed_bit_rmw_mutations_pointers_compound_assign_bit_writeback_and_patterns() {
    let source = r#"
        ^attrs.packed.bit
        struct Inner5 {
            low: u8,
            nib: BitVec<4>,
        }

        impl Clone for Inner5 {
            fn clone(*self) -> Self {
                Inner5 {
                    low: self.low,
                    nib: self.nib,
                }
            }
        }
        impl Copy for Inner5 {}

        ^attrs.packed.bit
        struct Packet {
            lo: u8,
            inner: Inner5,
            hi: u8,
        }

        impl Clone for Packet {
            fn clone(*self) -> Self {
                Packet {
                    lo: self.lo,
                    inner: self.inner.*,
                    hi: self.hi,
                }
            }
        }
        impl Copy for Packet {}

        ^attrs.packed.bit
        struct FloatCounter {
            tag: u8,
            x: f32,
            y: f32,
        }

        fn mutate_lo_and_return(p: *Packet) -> u8 {
            p.*.lo = 0x55;
            0xAA
        }

        fn run() -> bool {
            var pkt = Packet {
                lo: 0x11,
                inner: Inner5 { low: 0x22, nib: 0b0101 },
                hi: 0x33,
            };

            -- Local field mutations and compound assignment
            pkt.lo = 0x12;
            pkt.inner.nib = 0b1010;
            pkt.lo += 1;
            pkt.inner.low += 2;
            let local_ok = pkt.lo == 0x13 and pkt.inner.low == 0x24 and pkt.inner.nib == 0b1010;

            -- RHS evaluation must observe/mutate storage before RMW loads preserved bits
            pkt.hi = mutate_lo_and_return(pkt.ref);
            let rhs_order_ok = pkt.lo == 0x55 and pkt.hi == 0xAA and pkt.inner.low == 0x24;

            -- Bit writeback on local bit-packed struct & its fields
            pkt'bitvec[0..4] = 0xE;
            pkt.inner.low'bitvec[4..8] = 0xB;
            pkt.inner.nib[1..3] = 0b00;
            pkt'bitvec[4..8] = pkt'bitvec[4..8] or 0b0010;
            let local_wb_ok = pkt.lo == 0x7E
                and pkt.inner.low == 0xB4
                and pkt.inner.nib == 0b1000;

            -- Pointer deref RMW and pointer field compound assign / bit writeback
            let p: *Packet = pkt.ref;
            p.*.lo = 0x41;
            p.hi.* = 0x99;
            p.inner.low.* = 0x30;
            p.inner.nib.* = 0b0111;
            let direct_hi: u8 = p.hi;
            let direct_inner_low: u8 = p.inner.low;
            p.*'bitvec[0..4] = 0x9;
            p.*.inner.low'bitvec[0..4] = 0xC;
            p.inner.nib.*[0..2] = 0b10;

            var fc = FloatCounter { tag: 7, x: 1.0, y: 2.0 };
            let fp: *FloatCounter = fc.ref;
            fp.*.x += 1.5;
            fp.y.* += 2.5;
            let fc_ok = fc.tag == 7 and fc.x == 2.5 and fc.y == 4.5;

            let ptr_ok = direct_hi == 0x99
                and direct_inner_low == 0x30
                and pkt.lo == 0x49
                and pkt.hi == 0x99
                and pkt.inner.low == 0x3C
                and pkt.inner.nib == 0b0110
                and fc_ok;

            -- Pattern matching destructuring including nested attrs.packed.bit
            let pat_ok = pkt match {
                Packet {
                    lo,
                    inner: Inner5 { low, nib },
                    hi,
                } => lo == 0x49 and low == 0x3C and nib == 0b0110 and hi == 0x99,
            };

            local_ok and rhs_order_ok and local_wb_ok and ptr_ok and pat_ok
        }
    "#;

    let case = CliCase::new(source);
    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let out = case.run_binary();
        assert_success(&out);
        assert_eq!(out.stdout, b"(bool true)\n", "opt={opt}");
    }

    let restricted_wb = r#"
        ^attrs.packed.bit
        struct Restricted {
            ch: char,
            pad: u8,
        }

        fn bad() {
            var r = Restricted { ch: 'a', pad: 1 };
            r'bitvec[32..40] = 2;
        }
    "#;
    let err = support::compile(restricted_wb).unwrap_err();
    assert!(
        err.contains("unrestricted"),
        "expected unrestricted writeback error:\n{err}"
    );

    let restricted_ptr_wb = r#"
        ^attrs.packed.bit
        struct Restricted {
            ch: char,
            pad: u8,
        }

        fn bad() {
            var r = Restricted { ch: 'a', pad: 1 };
            let rp: *Restricted = r.ref;
            rp.*'bitvec[32..40] = 2;
        }
    "#;
    let err = support::compile(restricted_ptr_wb).unwrap_err();
    assert!(
        err.contains("unrestricted"),
        "expected unrestricted pointer writeback error:\n{err}"
    );

    let affine_bitvec_use_after_move = r#"
        ^attrs.packed.bit
        struct AffineToken {
            tag: u8,
            payload: u8,
        }

        fn run() -> u8 {
            var tok = AffineToken { tag: 1, payload: 2 };
            tok'bitvec[0..8] = tok'bitvec[0..8];
            tok.tag
        }
    "#;
    let affine_case = CliCase::new(affine_bitvec_use_after_move);
    let affine_out = affine_case.build("0");
    assert!(
        !affine_out.status.success(),
        "expected move-error for non-Copy attrs.packed.bit overlapping bitvec writeback"
    );
    let affine_stderr = String::from_utf8_lossy(&affine_out.stderr);
    assert!(
        affine_stderr.contains("move-error") && affine_stderr.contains("NotPresent"),
        "expected move-error NotPresent:\n{affine_stderr}"
    );
}

#[test]
fn packed_bit_containers_const_ref_and_over_256_bit_execution() {
    let source = r#"
        use std.prelude.*;
        use std.collections.Vec;

        ^attrs.packed.bit
        struct FlagsByte {
            ready: bool,
            priority: BitVec<3>,
            channel: BitVec<4>,
        }

        impl Clone for FlagsByte {
            fn clone(*self) -> Self {
                FlagsByte {
                    ready: self.ready,
                    priority: self.priority,
                    channel: self.channel,
                }
            }
        }
        impl Copy for FlagsByte {}

        ^attrs.packed.byte
        struct ByteWireOuter {
            tag: u8,
            flags: FlagsByte,
            tail: u16,
        }

        struct PlainOuter {
            id: u32,
            flags: FlagsByte,
        }

        ^attrs.packed.bit
        struct Wide300 {
            w0: u64,
            w1: u128,
            cross: u64,
            tail: BitVec<44>,
        }

        struct FlagsSeed {
            ready: bool,
            priority: u8,
            channel: u8,
        }

        impl Splice for FlagsSeed {
            assoc RuntimeTarget: Type = FlagsByte;

            fn splice(comptime data: FlagsSeed) -> FlagsByte {
                FlagsByte {
                    ready: data.ready,
                    priority: data.priority'bitvec[0..3],
                    channel: data.channel'bitvec[0..4],
                }
            }
        }

        ^attrs.packed.bit
        struct DirectConstBit {
            flag: bool,
            tag: u8,
            wide: u64,
        }

        impl Splice for DirectConstBit {
            assoc RuntimeTarget: Type = DirectConstBit;

            fn splice(comptime data: DirectConstBit) -> DirectConstBit {
                DirectConstBit {
                    flag: data.flag,
                    tag: data.tag,
                    wide: data.wide,
                }
            }
        }

        struct Wide300Seed {
            w0: u64,
            w1: u128,
            cross: u64,
            tail: u64,
        }

        impl Splice for Wide300Seed {
            assoc RuntimeTarget: Type = Wide300;

            fn splice(comptime data: Wide300Seed) -> Wide300 {
                Wide300 {
                    w0: data.w0,
                    w1: data.w1,
                    cross: data.cross,
                    tail: data.tail'bitvec[0..44],
                }
            }
        }

        const CONST_FLAGS = FlagsSeed {
            ready: true,
            priority: 0b011,
            channel: 0b1010,
        };

        const CONST_DIRECT_BP = DirectConstBit {
            flag: true,
            tag: 0x3C,
            wide: 0x0102030405060708,
        };

        const CONST_WIDE = Wide300Seed {
            w0: 0x0102030405060708,
            w1: 0x1112131415161718191A1B1C1D1E1F20,
            cross: 0xAABBCCDDEEFF0011,
            tail: 0x123456789AB,
        };

        fn check_containers() -> bool {
            let expected_flags = FlagsByte {
                ready: true,
                priority: 0b011,
                channel: 0b1010,
            };

            -- 1. Nested inside attrs.packed.byte with pointer RMW
            var bw = ByteWireOuter {
                tag: 9,
                flags: expected_flags,
                tail: 0x4567,
            };
            let pb: *ByteWireOuter = bw.ref;
            pb.flags.priority.* = 0b111;
            let bw_ok = bw.tag == 9
                and bw.flags.ready == true
                and bw.flags.priority == 0b111
                and bw.flags.channel == 0b1010
                and bw.tail == 0x4567;

            -- 2. Nested inside regular struct & optional
            var po = PlainOuter { id: 42, flags: FlagsByte { ready: false, priority: 2, channel: 5 } };
            po.flags.channel = 12;
            let opt_some: ?FlagsByte = po.flags;
            let opt_none: ?FlagsByte = null;
            let opt_ok = (opt_some match {
                v? => v.ready == false and v.priority == 2 and v.channel == 12,
                null => false,
            }) and (opt_none match {
                _? => false,
                null => true,
            });

            -- 3. Stored inside std.collections.Vec<FlagsByte>
            var items = Vec<FlagsByte>.new();
            let p1 = items.ref.push(expected_flags) match { _! => true, error _ => false };
            let p2 = items.ref.push(FlagsByte { ready: false, priority: 0b101, channel: 0b0111 }) match {
                _! => true,
                error _ => false,
            };
            let v_ok = p1
                and p2
                and items.ref.len() == 2
                and (items.ref.get(0) match {
                    v? => v'bitvec == expected_flags'bitvec,
                    null => false,
                })
                and (items.ref.pop() match {
                    v? => v.ready == false and v.priority == 0b101 and v.channel == 0b0111,
                    null => false,
                });

            -- 4. const global address read (including direct attrs.packed.bit and >256-bit globals)
            let c_ptr: *FlagsByte = CONST_FLAGS.ref;
            let d_ptr: *DirectConstBit = CONST_DIRECT_BP.ref;
            let w_ptr: *Wide300 = CONST_WIDE.ref;
            let const_ok = c_ptr.*.priority == 0b011
                and c_ptr.*'bitvec == 0b1010_011_1
                and d_ptr.*.flag == true
                and d_ptr.*.tag == 0x3C
                and d_ptr.*.wide == 0x0102030405060708
                and w_ptr.*.cross == 0xAABBCCDDEEFF0011
                and w_ptr.*.tail == 0x123456789AB;

            bw_ok and opt_ok and v_ok and const_ok
        }

        fn check_wide_300() -> bool {
            var big = Wide300 {
                w0: 0x0102030405060708,
                w1: 0x1112131415161718191A1B1C1D1E1F20,
                cross: 0xAABBCCDDEEFF0011,
                tail: 0x123456789AB,
            };
            big.cross = 0x5566778899AABBCC;
            big.tail = 0xFEDCBA98765;
            let fields_ok = big.w0 == 0x0102030405060708
                and big.cross == 0x5566778899AABBCC
                and big.tail == 0xFEDCBA98765;
            let bits: BitVec<300> = big'bitvec;
            -- w0 [0..64], w1 [64..192], cross [192..256], tail [256..300]
            let slice_ok = bits[0..64] == 0x0102030405060708
                and bits[64..192] == 0x1112131415161718191A1B1C1D1E1F20
                and bits[192..256] == 0x5566778899AABBCC
                and bits[256..300] == 0xFEDCBA98765;
            fields_ok and slice_ok
        }

        fn run() -> bool {
            check_containers() and check_wide_300()
        }
    "#;

    let case = CliCase::new(source);
    let ir = case.dump_llvm_o0().unwrap();
    assert!(ir.contains("<{ i8, i8, i16 }>"), "{ir}");
    assert!(ir.contains("align 1"), "{ir}");

    for opt in ["0", "3"] {
        let build = case.build(opt);
        assert_success(&build);
        let stderr = String::from_utf8_lossy(&build.stderr);
        assert!(
            stderr.contains("W_PACKED_BIT_WIDTH")
                && stderr.contains("Wide300")
                && stderr.contains("300"),
            "expected non-fatal >256-bit warning on opt={opt}:\n{stderr}"
        );
        let out = case.run_binary();
        assert_success(&out);
        assert_eq!(out.stdout, b"(bool true)\n", "opt={opt}");
    }
}

#[test]
fn packed_bit_shares_bitvec_opt_bswap_and_identity_simplification() {
    let source = r#"
        ^attrs.packed.bit
        struct Bytes4 {
            b0: u8,
            b1: u8,
            b2: u8,
            b3: u8,
        }

        impl Clone for Bytes4 {
            fn clone(*self) -> Self {
                Bytes4 {
                    b0: self.b0,
                    b1: self.b1,
                    b2: self.b2,
                    b3: self.b3,
                }
            }
        }
        impl Copy for Bytes4 {}

        fn swap_bytes(x: Bytes4) -> Bytes4 {
            Bytes4 {
                b0: x.b3,
                b1: x.b2,
                b2: x.b1,
                b3: x.b0,
            }
        }

        fn rebuild_identity(x: Bytes4) -> Bytes4 {
            Bytes4 {
                b0: x.b0,
                b1: x.b1,
                b2: x.b2,
                b3: x.b3,
            }
        }

        fn read_word() -> Bytes4 {
            let w: u32 = 0x11223344;
            w'bitvec.as(Bytes4)
        }

        fn run() -> bool {
            let input = read_word();
            let swapped = swap_bytes(input);
            let same = rebuild_identity(input);
            swapped'bitvec == 0x44332211 and same'bitvec == 0x11223344
        }
    "#;

    let case = CliCase::new(source);
    let ir = case.dump_llvm_o0().unwrap();
    if cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
        assert!(
            ir.contains("@llvm.bswap.i32"),
            "expected Bytes4 byte reversal to synthesize @llvm.bswap.i32:\n{ir}"
        );
    }

    // Verify `rebuild_identity` lowers cleanly without redundant shl/lshr reassembly:
    // find the small i32 -> i32 function without bswap and ensure it contains no shl/lshr.
    let mut found_identity_without_shifts = false;
    for chunk in ir.split("\ndefine ") {
        if let Some(header_end) = chunk.find('{') {
            let header = &chunk[..header_end];
            if header.contains("i32 @flurry_")
                && header.contains("(i32 %")
                && !chunk.contains("bswap")
                && !chunk.contains("287454020")
            {
                let body = &chunk[header_end..];
                if !body.contains("shl ") && !body.contains("lshr ") && !body.contains("or i32") {
                    found_identity_without_shifts = true;
                }
            }
        }
    }
    assert!(
        found_identity_without_shifts,
        "expected rebuild_identity(Bytes4) to simplify to pure value without shifts:\n{ir}"
    );

    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let out = case.run_binary();
        assert_success(&out);
        assert_eq!(out.stdout, b"(bool true)\n", "opt={opt}");
    }
}

#[test]
fn packed_bit_comptime_tree_and_bytecode_vm_parity_and_validation() {
    let source = r#"
        enum Exhaustive2 {
            .width 2,
            .discrete_tag true,
            e0: 0b00,
            e1: 0b01,
            e2: 0b10,
            e3: 0b11,
        }

        enum Sparse4 {
            .width 4,
            .discrete_tag true,
            alpha: 0b0011,
            beta: 0b1100,
        }

        ^attrs.packed.bit
        struct Inner5 {
            flag: bool,
            nib: BitVec<4>,
        }

        ^attrs.packed.bit
        struct Packet {
            signed: i8,
            inner: Inner5,
            ex: Exhaustive2,
            tail: u8,
        }

        ^attrs.packed.bit
        struct WithChar {
            ch: char,
            pad: u8,
        }

        ^attrs.packed.bit
        struct WithSparse {
            tag: Sparse4,
            v: BitVec<4>,
        }

        comptime fn ct_roundtrip_and_mutate() -> bool {
            var p = Packet {
                tail: 0xA5,
                ex: Exhaustive2.e2,
                inner: Inner5 { flag: true, nib: 0b1001 },
                signed: -17,
            };
            let bits = p'bitvec;
            let p2 = bits.as(Packet);
            p.inner.nib = 0b0110;
            p.signed = -3;
            p'bitvec[0..8] = 0xF0;
            let pat_ok = p2 match {
                Packet {
                    signed,
                    inner: Inner5 { flag, nib },
                    ex,
                    tail,
                } => {
                    signed == -17
                        and flag == true
                        and nib == 0b1001
                        and (ex match { Exhaustive2.e2 => true, _ => false })
                        and tail == 0xA5
                }
            };
            let expected_signed: i8 = -16;
            pat_ok
                and p.signed == expected_signed
                and p.inner.nib == 0b0110
                and p.tail == 0xA5
        }

        comptime fn ct_float_char_sparse() -> bool {
            let wc = WithChar { ch: 'β', pad: 0x3C };
            let ws = WithSparse { tag: Sparse4.beta, v: 0b0101 };
            let wc2 = unsafe { wc'bitvec.as(WithChar) };
            let ws2 = unsafe { ws'bitvec.as(WithSparse) };
            let ws_ok = ws2.tag match { Sparse4.beta => true, _ => false };
            wc2.ch == 'β' and wc2.pad == 0x3C and ws_ok and ws2.v == 0b0101
        }

        comptime fn ct_invalid_char() -> WithChar {
            let bad: BitVec<40> = 0x000000D800;
            unsafe { bad.as(WithChar) }
        }

        comptime fn ct_invalid_sparse() -> WithSparse {
            let bad: BitVec<8> = 0b0101_1111;
            unsafe { bad.as(WithSparse) }
        }
    "#;

    let package = support::compile(source).unwrap();
    for fn_name in ["ct_roundtrip_and_mutate", "ct_float_char_sparse"] {
        let id = support::source_function(&package, fn_name);
        let vm_val = support::with_comptime(&package, support::COMPTIME_BUDGET, false, |vm| {
            let before = vm.bytecode_invocations();
            let val = vm.invoke(id, vec![]).unwrap();
            assert!(
                vm.bytecode_invocations() > before,
                "expected bytecode VM execution for {fn_name}"
            );
            val
        });
        let tree_val = support::with_comptime(&package, support::COMPTIME_BUDGET, true, |tree| {
            tree.invoke(id, vec![]).unwrap()
        });
        assert_eq!(vm_val, Value::Bool(true), "{fn_name} vm");
        assert_eq!(tree_val, Value::Bool(true), "{fn_name} tree");
    }

    for invalid_fn in ["ct_invalid_char", "ct_invalid_sparse"] {
        let id = support::source_function(&package, invalid_fn);
        let vm_err = support::with_comptime(&package, support::COMPTIME_BUDGET, false, |vm| {
            vm.invoke(id, vec![]).unwrap_err()
        });
        let tree_err = support::with_comptime(&package, support::COMPTIME_BUDGET, true, |tree| {
            tree.invoke(id, vec![]).unwrap_err()
        });
        assert_eq!(vm_err.kind, tree_err.kind, "{invalid_fn}");
    }
}
