//! End-to-end tests for Type.predicates.*, Type.size_of/align_of, std.mem.Layout,
//! and builtin protocol additions (Display for char, Ord for ?T / !E T).
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use luna::cli::{self, Command};
use luna::frontend::{self, Artifact};
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-reflect-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self, level: &str) -> Output {
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
                level,
                "--fuel",
                "10000000",
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
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_with_target(source: &str, arch: &str, os: &str, bits: &str) -> Result<Value, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "luna-reflect-target-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, source).unwrap();
    let library = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let Command::Check(options) = cli::parse([
        "check".into(),
        path.clone().into_os_string(),
        "--sysroot".into(),
        library.into_os_string(),
        "--build-target".into(),
        arch.into(),
        os.into(),
        bits.into(),
    ])
    .unwrap() else {
        panic!("expected check command");
    };
    let compiled = frontend::compile(&options);
    let _ = fs::remove_file(&path);
    let Artifact::Nir(package) = compiled? else {
        panic!("expected NIR artifact");
    };
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000)?;
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default())
        .map_err(|e| format!("{e:?}"))?;
    vm.invoke_instance(&graph.roots[0], vec![])
        .map_err(|e| format!("{e:?}"))
}

#[test]
fn type_predicates_cover_all_nine_closed_axes_and_nominal_boundaries() {
    let source = r#"
        newtype RegisterId = u32;
        struct Plain { value: u32 }
        enum Choice { left, right }
        comptime fn schema(...properties: T) -> T where T { properties }
        const SAMPLE_RECORD = schema { a: true, b: false };
        typealias SampleRecord = SAMPLE_RECORD'type;

        comptime fn check_comptime_predicates() -> bool {
            Type.predicates.is_boolean(bool)
                and not Type.predicates.is_boolean(u8)
                and Type.predicates.is_symbol(Symbol)
                and not Type.predicates.is_symbol(str)
                and Type.predicates.is_integer(i32)
                and Type.predicates.is_integer(u64)
                and Type.predicates.is_integer(Integer)
                and Type.predicates.is_signed_integer(i16)
                and Type.predicates.is_signed_integer(isize)
                and not Type.predicates.is_signed_integer(u16)
                and Type.predicates.is_unsigned_integer(u8)
                and Type.predicates.is_unsigned_integer(usize)
                and not Type.predicates.is_unsigned_integer(i8)
                -- Newtypes, structs, enums, optionals, error results, and records never inherit closed classifications.
                and not Type.predicates.is_integer(RegisterId)
                and not Type.predicates.is_unsigned_integer(RegisterId)
                and not Type.predicates.is_tuple(Plain)
                and not Type.predicates.is_tuple(Choice)
                and not Type.predicates.is_integer(?u32)
                and not Type.predicates.is_integer(!u8 u32)
                and not Type.predicates.is_tuple(SampleRecord)
                and Type.predicates.is_floating_point(f32)
                and Type.predicates.is_floating_point(Float)
                and Type.predicates.is_tuple(Tuple<u8, bool>)
                and Type.predicates.is_pointer(*u32)
                and Type.predicates.is_function(fn(u32) -> bool)
        }

        const COMPTIME_PREDICATES_OK = check_comptime_predicates();

        fn classify(value: T) -> u8 where T {
            inline if Type.predicates.is_boolean(T) {
                1
            } else if Type.predicates.is_signed_integer(T) {
                2
            } else if Type.predicates.is_unsigned_integer(T) {
                3
            } else if Type.predicates.is_floating_point(T) {
                4
            } else if Type.predicates.is_tuple(T) {
                5
            } else {
                0
            }
        }

        fn classify_when(value: T) -> u8 where T {
            inline when {
                Type.predicates.is_pointer(T) => 6,
                Type.predicates.is_function(T) => 7,
                Type.predicates.is_symbol(T) => 8,
                else => 9,
            }
        }

        fn check_int_clause(x: T) -> bool where T, requires Type.predicates.is_integer(T) {
            true
        }

        fn helper(x: u32) -> bool { true }

        fn run() -> bool {
            let p: u32 = 10;
            let signed_val: i32 = -5;
            let unsigned_val: u64 = 5;
            let byte_val: u8 = 1;
            let s: f64 = 1.5;
            let small_u8: u8 = 42;
            let small_i64: i64 = -3;
            let reg_raw: u32 = 1;
            COMPTIME_PREDICATES_OK
                and classify(true) == 1
                and classify(signed_val) == 2
                and classify(unsigned_val) == 3
                and classify(s) == 4
                and classify((true, byte_val)) == 5
                and classify(reg_raw.as(RegisterId)) == 0
                and classify_when(p.ref) == 6
                and classify_when(helper) == 7
                and classify_when(Plain { value: 1 }) == 9
                and check_int_clause(small_u8)
                and check_int_clause(small_i64)
                and Type.predicates.is_tuple(Tuple<u8, bool>)
                and not Type.predicates.is_floating_point(bool)
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));

    let bad = "const bad = Type.predicates.is_bogus(u32);";
    assert!(support::compile(bad).is_err());

    let bad_clause = r#"
        newtype RegisterId = u32;
        fn need_int(x: T) -> bool where T, requires Type.predicates.is_integer(T) { true }
        fn run() -> bool {
            let v: u32 = 1;
            need_int(v.as(RegisterId))
        }
    "#;
    let err = support::compile(bad_clause).unwrap_err();
    assert!(!err.contains("LiteralInto"), "{err}");
}

#[test]
fn nominal_kind_predicates_follow_declarations_across_stages() {
    let source = r#"
        struct Cell where T { value: T }
        enum Choice { on, off }
        newtype Wrapped = Cell<bool>;
        typealias Alias = Cell<bool>;

        comptime fn accepts_record(subject: Type) -> bool {
            Type.predicates.is_struct(subject)
        }

        fn describe(value: T) -> bool where T, requires Type.predicates.is_struct(T) {
            inline if Type.predicates.is_newtype(T) { false } else { true }
        }

        fn run() -> bool {
            const reflected = accepts_record(Alias);
            reflected and describe(Alias { value: true })
                and Type.predicates.is_enum(Choice)
                and Type.predicates.is_newtype(Wrapped)
                and not Type.predicates.is_struct(Wrapped)
                and not Type.predicates.is_enum(?Choice)
                and not Type.predicates.is_struct(Tuple<bool>)
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    let wrong = source.replace("describe(Alias { value: true })", "describe(Choice.on)");
    let error = support::compile(&wrong).unwrap_err();
    assert!(
        error.contains("requirement") || error.contains("obligation"),
        "{error}"
    );
}

#[test]
fn layout_reflection_matches_llvm_and_docs_across_native_and_target_widths() {
    let source = r#"
        use std.prelude.*;

        struct SamplePair {
            first: u32,
            second: u32,
        }

        struct Padded {
            a: u8,
            b: u32,
            c: u8,
        }

        struct Link {
            value: u32,
            next: ?*Link,
        }

        struct Pair where T, U {
            first: T,
            second: U,
        }

        newtype Wrapper = u64;

        enum Color {
            red,
            green,
            blue,
        }

        enum Tag {
            .base_type Symbol,
            .discrete_tag true,
            alpha = .alpha,
            beta = .beta,
        }

        enum Packet {
            .base_type u16,
            .width 16,
            .discrete_tag true,
            empty: 0b0000000000000000,
            byte: 0b00000001 (value: u8),
        }

        enum Payload {
            none,
            pair(u8, u64),
        }

        const PAIR_SIZE = Type.size_of(SamplePair);
        const POINTER_SIZE = Type.size_of(*SamplePair);
        const WORD_SIZE = Type.size_of(usize);
        const LINK_POINTER_SIZE = Type.size_of(*Link);

        comptime fn check_comptime_layout_assumptions() -> bool {
            assumes POINTER_SIZE == WORD_SIZE;
            assumes LINK_POINTER_SIZE == Type.size_of(usize);
            assumes PAIR_SIZE == 8;
            true
        }

        const COMPTIME_LAYOUT_OK = check_comptime_layout_assumptions();

        comptime fn dynamic_size(t: Type) -> usize {
            Type.size_of(t)
        }

        const DYN_SIZE_OK = dynamic_size(u64) == 8 and dynamic_size(Tuple<u8, u64>) == 16;

        quote fn storage_bytes(value: T) -> usize where T {
            Type.size_of(value'type)
        }

        fn check_storage_bytes() -> bool {
            let value: u32 = 27;
            storage_bytes(value) == 4 and Type.size_of(value'type) == 4
        }

        fn run() -> bool {
            COMPTIME_LAYOUT_OK
                and DYN_SIZE_OK
                and PAIR_SIZE == 8
                and Type.align_of(SamplePair) == 4
                and POINTER_SIZE == 8
                and WORD_SIZE == 8
                and Type.size_of(void) == 0 and Type.align_of(void) == 1
                and Type.size_of(bool) == 1 and Type.align_of(bool) == 1
                and Type.size_of(u8) == 1 and Type.align_of(u8) == 1
                and Type.size_of(u16) == 2 and Type.align_of(u16) == 2
                and Type.size_of(u32) == 4 and Type.align_of(u32) == 4
                and Type.size_of(u64) == 8 and Type.align_of(u64) == 8
                and Type.size_of(u128) == 16 and Type.align_of(u128) == 16
                and Type.size_of(char) == 4 and Type.align_of(char) == 4
                and Type.size_of(f32) == 4 and Type.align_of(f32) == 4
                and Type.size_of(f64) == 8 and Type.align_of(f64) == 8
                and Type.size_of(str) == 16 and Type.align_of(str) == 8
                and Type.size_of(Symbol) == 16 and Type.align_of(Symbol) == 8
                and Type.size_of(*u8) == 8 and Type.align_of(*u8) == 8
                and Type.size_of(fn(u32) -> u32) == 8 and Type.align_of(fn(u32) -> u32) == 8
                and Type.size_of(Tuple<>) == 0 and Type.align_of(Tuple<>) == 1
                and Type.size_of(Tuple<u8, u64>) == 16 and Type.align_of(Tuple<u8, u64>) == 8
                and Type.size_of(?u64) == 16 and Type.align_of(?u64) == 8
                and Type.size_of(?void) == 1 and Type.align_of(?void) == 1
                and Type.size_of(!u8 u64) == 16 and Type.align_of(!u8 u64) == 8
                and Type.size_of(Padded) == 12 and Type.align_of(Padded) == 4
                and Type.size_of(Link) == 16 and Type.align_of(Link) == 8
                and Type.size_of(Pair<u8, u64>) == 16 and Type.align_of(Pair<u8, u64>) == 8
                and Type.size_of(Wrapper) == 8 and Type.align_of(Wrapper) == 8
                and Type.size_of(Color) == 1 and Type.align_of(Color) == 1
                and Type.size_of(Tag) == 16 and Type.align_of(Tag) == 8
                and Type.size_of(Packet) == 2 and Type.align_of(Packet) == 2
                and Type.size_of(Payload) == 24 and Type.align_of(Payload) == 8
                and check_storage_bytes()
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build("0"));
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");

    // Verify 32-bit vs 64-bit target pointer sensitivity.
    let target_probe = r#"
        fn run() -> bool {
            Type.size_of(usize) == 4
                and Type.align_of(usize) == 4
                and Type.size_of(*u32) == 4
                and Type.align_of(*u32) == 4
        }
    "#;
    assert_eq!(
        run_with_target(target_probe, "x86", "linux", "32").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        run_with_target(target_probe, "x86_64", "linux", "64").unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn layout_reflection_rejects_cycles_and_inapplicable_domains() {
    let direct_cycle = r#"
        struct InvalidLink {
            next: InvalidLink,
        }
        const INVALID_SIZE = Type.size_of(InvalidLink);
        fn run() -> usize { INVALID_SIZE }
    "#;
    assert!(support::compile(direct_cycle).is_err());

    let mutual_cycle = r#"
        struct A { b: B }
        struct B { a: A }
        const INVALID_SIZE = Type.size_of(A);
        fn run() -> usize { INVALID_SIZE }
    "#;
    assert!(support::compile(mutual_cycle).is_err());

    for ty in [
        "Type",
        "Integer",
        "Float",
        "meta.Object",
        "meta.List<u32>",
        "NoReturn",
    ] {
        let source = format!("const BAD = Type.size_of({ty}); fn run() -> usize {{ BAD }}");
        assert!(
            support::compile(&source).is_err(),
            "expected layout query on {ty} to fail"
        );
    }
}

#[test]
fn std_mem_layout_and_builtin_display_ord_work_natively() {
    let source = r#"
        use std.prelude.*;
        use std.mem.Layout;

        struct ThreeFields {
            a: u8,
            b: u64,
            c: u16,
        }

        fn check_layouts() -> bool {
            let of_three = Layout.of<ThreeFields>() match { value? => value, null => return false };
            let arr = Layout.array<u32>(5) match { value? => value, null => return false };
            let padded = (Layout.new(5, 4) match {
                value? => value.pad_to_align(),
                null => null,
            }) match {
                value? => value,
                null => return false,
            };
            let first = Layout.new(1, 1) match { value? => value, null => return false };
            let second = Layout.new(8, 8) match { value? => value, null => return false };
            let (combined, offset) = first.append(second) match {
                value? => value,
                null => return false,
            };
            let zero = Layout.of<void>() match { value? => value, null => return false };
            let overflow = Layout.array<u64>(4611686018427387904).is_none();
            of_three.byte_size() == 24 and of_three.byte_alignment() == 8
                and arr.byte_size() == 20 and arr.byte_alignment() == 4
                and padded.byte_size() == 8 and padded.byte_alignment() == 4
                and combined.byte_size() == 16 and combined.byte_alignment() == 8
                and offset == 8
                and zero.byte_size() == 0 and zero.byte_alignment() == 1
                and overflow
        }

        fn check_ordering() -> bool {
            let none_val: ?u32 = null;
            let one: ?u32 = 1;
            let two: ?u32 = 2;
            let nested_none: ??u32 = null;
            let nested_inner_none: ??u32 = none_val;
            let nested_some: ??u32 = 1;

            let ok_small: !u8 u32 = 3;
            let ok_big: !u8 u32 = 9;
            let err_small: !u8 u32 = error 1;
            let err_big: !u8 u32 = error 5;

            none_val.as(Ord).lt(one)
                and one.as(Ord).lt(two)
                and (two.as(Ord).cmp(two) matches .equal)
                and nested_none.as(Ord).lt(nested_inner_none)
                and nested_inner_none.as(Ord).lt(nested_some)
                and ok_small.as(Ord).lt(ok_big)
                and ok_big.as(Ord).lt(err_small)
                and err_small.as(Ord).lt(err_big)
                and (err_small.as(Ord).cmp(err_small) matches .equal)
        }

        fn check_fmt() -> !std.io.IoErr bool {
            println("chars:{}:{}:{}:{}", 'A', 'é', '中', '🦀')!;
            true
        }

        fn run() -> bool {
            check_layouts() and check_ordering() and (check_fmt() match { v! => v, error _ => false })
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build("0"));
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, "chars:A:é:中:🦀\n(bool true)\n".as_bytes());
}

#[test]
fn error_qualified_sum_type_abi_layout_indexing_and_drop_work_natively() {
    let source = r#"
        use std.prelude.*;
        use std.collections.Vec;
        use std.mem.AllocErr;

        enum ErrA {
            Pair(u64, u64),
        }

        enum ErrB {
            Pair(u64, u64),
        }

        enum ErrC {
            Pair(u64, u64),
        }

        struct BigOk {
            a: u64,
            b: u64,
            tag: u8,
        }

        ^attrs.packed.byte
        struct Packed9Err {
            head: u64,
            tail: u8,
        }

        struct ZeroErr {}

        struct ZeroOk {}

        struct OwnedOk {
            id: u64,
        }

        impl Drop for OwnedOk {
            fn drop(*self) {
                let _ = std.fmt.println("drop-ok:{}", self.id.*);
            }
        }

        struct OwnedErr {
            code: u8,
            extra: u64,
        }

        impl Drop for OwnedErr {
            fn drop(*self) {
                let _ = std.fmt.println("drop-err:{}", self.code.*);
            }
        }

        fn make_narrow(flag: bool) -> !ErrA u64 {
            if flag {
                42
            } else {
                error ErrA.Pair(7, 99)
            }
        }

        fn widen_row(flag: bool) -> ![ErrA, ErrB, ErrC] u64 {
            let v = make_narrow(flag)!;
            if v < 1000 {
                v + 1
            } else {
                v
            }
        }

        fn make_owned_result(ok: bool) -> !OwnedErr OwnedOk {
            if ok {
                OwnedOk { id: 11 }
            } else {
                error OwnedErr { code: 33, extra: 77 }
            }
        }

        fn check_vec_sum_elements() -> !AllocErr bool {
            var items = Vec<!ErrB BigOk>.new();
            items.push(BigOk { a: 1, b: 2, tag: 3 })!;
            items.push(error ErrB.Pair(100, 200))!;
            items.push(BigOk { a: 4, b: 5, tag: 6 })!;

            let third = items.pop() match {
                found? => found match {
                    v! => v.a == 4 and v.b == 5 and v.tag == 6,
                    error _ => false,
                },
                null => false,
            };
            let second = items.pop() match {
                found? => found match {
                    _! => false,
                    error ErrB.Pair(hi, lo) => hi == 100 and lo == 200,
                },
                null => false,
            };
            let first = items.pop() match {
                found? => found match {
                    v! => v.a == 1 and v.b == 2 and v.tag == 3,
                    error _ => false,
                },
                null => false,
            };
            third and second and first
        }

        fn check_sizes() -> bool {
            Type.size_of(!u8 u8) == 2
                and Type.align_of(!u8 u8) == 1
                and Type.size_of(!u8 u64) == 16
                and Type.align_of(!u8 u64) == 8
                and Type.size_of(![ErrA, ErrB, ErrC] BigOk) == 32
                and Type.align_of(![ErrA, ErrB, ErrC] BigOk) == 8
                and Type.size_of(!ErrB BigOk) == 32
                and Type.align_of(!ErrB BigOk) == 8
                and Type.size_of(!u128 u8) == 32
                and Type.align_of(!u128 u8) == 16
                and Type.size_of(!Packed9Err u64) == 24
                and Type.align_of(!Packed9Err u64) == 8
                and Type.size_of(!ZeroErr ZeroOk) == 1
                and Type.align_of(!ZeroErr ZeroOk) == 1
        }

        fn check_embedding() -> bool {
            let a = widen_row(true) match {
                v! => v == 43,
                error _ => false,
            };
            let b = widen_row(false) match {
                _! => false,
                error ErrA.Pair(code, detail) => code == 7 and detail == 99,
                error ErrB.Pair(_, _) => false,
                error ErrC.Pair(_, _) => false,
            };
            a and b
        }

        fn exercise_drop() {
            let _ok_val = make_owned_result(true);
            let _err_val = make_owned_result(false);
        }

        fn run() -> bool {
            let sizes_ok = check_sizes();
            let vec_ok = check_vec_sum_elements() match {
                v! => v,
                error _ => false,
            };
            let embed_ok = check_embedding();
            exercise_drop();
            sizes_ok and vec_ok and embed_ok
        }
    "#;
    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        let build = case.build(opt);
        assert_success(&build);
        let output = case.run();
        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("drop-err:33\n")
                && stdout.contains("drop-ok:11\n")
                && stdout.ends_with("(bool true)\n"),
            "opt={opt}: {stdout}"
        );
    }
}

#[test]
fn aggregate_enum_sum_type_abi_layout_const_vec_and_drop_work_natively() {
    let source = r#"
        use std.prelude.*;
        use std.collections.Vec;
        use std.mem.AllocErr;

        enum Small {
            none,
            byte(u8),
            pair(u8, u8),
        }

        enum Rich {
            none,
            pair(u64, u64),
            text(str, u64),
            triple(u8, u64, u8),
        }

        impl Clone for Rich {
            fn clone(*self) -> Self {
                self.* match {
                    Rich.none => Rich.none,
                    Rich.pair(a, b) => Rich.pair(a, b),
                    Rich.text(s, n) => Rich.text(s, n),
                    Rich.triple(a, b, c) => Rich.triple(a, b, c),
                }
            }
        }

        impl Copy for Rich {}

        enum Wide {
            small(u8),
            huge(u128),
        }

        ^attrs.packed.byte
        struct Packed9 {
            head: u64,
            tail: u8,
        }

        enum PackedPayload {
            packed(Packed9),
            word(u64),
        }

        struct ZeroStruct {}

        enum ZeroPayload {
            a(void),
            b(ZeroStruct),
        }

        enum ConstRich {
            none,
            pair(u64, u64),
            sym(Symbol, u64),
            triple(u8, u64, u8),
        }

        struct ConstRichSeed {
            sym: Symbol,
            n: u64,
        }

        impl Splice for ConstRichSeed {
            assoc RuntimeTarget: Type = ConstRich;

            fn splice(comptime data: ConstRichSeed) -> ConstRich {
                ConstRich.sym(data.sym, data.n)
            }
        }

        impl Clone for ConstRich {
            fn clone(*self) -> Self {
                self.* match {
                    ConstRich.none => ConstRich.none,
                    ConstRich.pair(a, b) => ConstRich.pair(a, b),
                    ConstRich.sym(s, n) => ConstRich.sym(s, n),
                    ConstRich.triple(a, b, c) => ConstRich.triple(a, b, c),
                }
            }
        }

        impl Copy for ConstRich {}

        struct ConstWrap {
            lead: u8,
            item: ConstRich,
            tail: u8,
        }

        struct ConstWrapSeed {
            lead: u8,
            a: u8,
            b: u64,
            c: u8,
            tail: u8,
        }

        impl Splice for ConstWrapSeed {
            assoc RuntimeTarget: Type = ConstWrap;

            fn splice(comptime data: ConstWrapSeed) -> ConstWrap {
                ConstWrap {
                    lead: data.lead,
                    item: ConstRich.triple(data.a, data.b, data.c),
                    tail: data.tail,
                }
            }
        }

        impl Clone for ConstWrap {
            fn clone(*self) -> Self {
                ConstWrap {
                    lead: self.lead.*,
                    item: self.item.*.clone(),
                    tail: self.tail.*,
                }
            }
        }

        impl Copy for ConstWrap {}

        const CONST_RICH = ConstRichSeed {
            sym: .union_const,
            n: 777,
        };
        const CONST_WRAP = ConstWrapSeed {
            lead: 11,
            a: 3,
            b: 999,
            c: 7,
            tail: 22,
        };

        struct OwnedA {
            id: u64,
        }

        impl Drop for OwnedA {
            fn drop(*self) {
                let _ = std.fmt.println("drop-a:{}", self.id.*);
            }
        }

        struct OwnedB {
            id: u64,
        }

        impl Drop for OwnedB {
            fn drop(*self) {
                let _ = std.fmt.println("drop-b:{}", self.id.*);
            }
        }

        enum OwnedChoice {
            empty,
            one(OwnedA),
            two(u8, OwnedA, OwnedB),
        }

        fn check_sizes() -> bool {
            Type.size_of(Small) == 3
                and Type.align_of(Small) == 1
                and Type.size_of(Rich) == 32
                and Type.align_of(Rich) == 8
                and Type.size_of(ConstRich) == 32
                and Type.align_of(ConstRich) == 8
                and Type.size_of(Wide) == 32
                and Type.align_of(Wide) == 16
                and Type.size_of(PackedPayload) == 24
                and Type.align_of(PackedPayload) == 8
                and Type.size_of(ZeroPayload) == 1
                and Type.align_of(ZeroPayload) == 1
        }

        fn check_vec_enum_elements() -> !AllocErr bool {
            var items = Vec<ConstRich>.new();
            items.push(ConstRich.none)!;
            items.push(ConstRich.pair(101, 202))!;
            items.push(ConstRich.sym(.hello_sum, 55))!;
            items.push(ConstRich.triple(9, 888, 4))!;

            let fourth = items.pop() match {
                found? => found match {
                    ConstRich.triple(a, b, c) => a == 9 and b == 888 and c == 4,
                    _ => false,
                },
                null => false,
            };
            let third = items.pop() match {
                found? => found match {
                    ConstRich.sym(s, n) => s == .hello_sum and n == 55,
                    _ => false,
                },
                null => false,
            };
            let second = items.pop() match {
                found? => found match {
                    ConstRich.pair(a, b) => a == 101 and b == 202,
                    _ => false,
                },
                null => false,
            };
            let first = items.pop() match {
                found? => found match {
                    ConstRich.none => true,
                    _ => false,
                },
                null => false,
            };
            let runtime_text = make_rich_text("hello-sum", 55) match {
                Rich.text(s, n) => s == "hello-sum" and n == 55,
                _ => false,
            };
            fourth and third and second and first and runtime_text
        }

        fn make_rich_text(s: str, n: u64) -> Rich {
            Rich.text(s, n)
        }

        fn read_const_rich(p: *ConstRich) -> bool {
            p.* match {
                ConstRich.sym(s, n) => s == .union_const and n == 777,
                _ => false,
            }
        }

        fn read_const_wrap(p: *ConstWrap) -> bool {
            let w = p.*;
            let item_ok = w.item match {
                ConstRich.triple(a, b, c) => a == 3 and b == 999 and c == 7,
                _ => false,
            };
            w.lead == 11 and w.tail == 22 and item_ok
        }

        fn check_consts() -> bool {
            read_const_rich(CONST_RICH.ref) and read_const_wrap(CONST_WRAP.ref)
        }

        fn partial_move_choice() -> u64 {
            let choice = OwnedChoice.two(9, OwnedA { id: 40 }, OwnedB { id: 50 });
            choice match {
                OwnedChoice.two(tag_byte, first, _) => {
                    if tag_byte == 9 {
                        let consumed = first;
                        consumed.id
                    } else {
                        0
                    }
                },
                _ => 0,
            }
        }

        fn exercise_drops() -> bool {
            do {
                let _two = OwnedChoice.two(5, OwnedA { id: 10 }, OwnedB { id: 20 });
                let _one = OwnedChoice.one(OwnedA { id: 30 });
                let _empty = OwnedChoice.empty;
            };
            partial_move_choice() == 40
        }

        fn run() -> bool {
            let sizes_ok = check_sizes();
            let vec_ok = check_vec_enum_elements() match {
                v! => v,
                error _ => false,
            };
            let const_ok = check_consts();
            let drop_ok = exercise_drops();
            sizes_ok and vec_ok and const_ok and drop_ok
        }
    "#;
    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        let build = case.build(opt);
        assert_success(&build);
        let output = case.run();
        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let drop_lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("drop-")).collect();
        assert!(
            stdout.contains("drop-b:20\n")
                && stdout.contains("drop-a:10\n")
                && stdout.contains("drop-a:30\n")
                && stdout.contains("drop-a:40\n")
                && stdout.contains("drop-b:50\n")
                && drop_lines.len() == 5
                && stdout.ends_with("(bool true)\n"),
            "opt={opt}: {stdout}"
        );
    }
}

#[test]
fn unit_enum_widths_and_compositional_niches_execute_and_drop_cleanly() {
    let mut big_variants = String::new();
    for i in 0..257 {
        big_variants.push_str(&format!("            v{i},\n"));
    }
    let source = format!(
        r#"
        use std.prelude.*;

        enum ZeroUnit {{}}
        enum SoloUnit {{ only }}
        enum TriColor {{ red, green, blue }}
        enum Big257 {{
{big_variants}        }}

        enum Packet {{
            .base_type u8,
            .width 8,
            .discrete_tag true,
            empty: 0b00000000,
            low: 0b00000001,
        }}

        struct Flagged {{
            lead: u8,
            flag: bool,
            tail: u64,
        }}

        struct PtrBox {{
            ptr: *u32,
            tag: u64,
        }}

        enum Tagged {{
            empty,
            alt,
            num(u64),
        }}

        enum SingleWrap {{
            wrap(bool),
        }}

        enum SinglePair {{
            pair(u8, u64),
        }}

        enum MaybePtr {{
            empty,
            some(*u32),
        }}

        enum MaybeBool {{
            empty,
            alt,
            some(bool),
        }}

        enum PairOpt {{
            empty,
            pair(bool, u64),
        }}

        struct OwnedWithBool {{
            flag: bool,
            id: u64,
        }}

        impl Drop for OwnedWithBool {{
            fn drop(*self) {{
                let mark = if self.flag.* {{ "T" }} else {{ "F" }};
                let _ = std.fmt.println("drop-bool:{{}}-{{}}", self.id.*, mark);
            }}
        }}

        struct OwnedNoNiche {{
            id: u64,
        }}

        impl Drop for OwnedNoNiche {{
            fn drop(*self) {{
                let _ = std.fmt.println("drop-no:{{}}", self.id.*);
            }}
        }}

        enum OwnedChoice {{
            empty,
            alt,
            active(OwnedWithBool),
        }}

        fn check_sizes() -> bool {{
            Type.size_of(ZeroUnit) == 0 and Type.align_of(ZeroUnit) == 1
                and Type.size_of(SoloUnit) == 0 and Type.align_of(SoloUnit) == 1
                and Type.size_of(TriColor) == 1 and Type.align_of(TriColor) == 1
                and Type.size_of(Big257) == 2 and Type.align_of(Big257) == 2
                and Type.size_of(?bool) == 1 and Type.align_of(?bool) == 1
                and Type.size_of(??bool) == 1 and Type.align_of(??bool) == 1
                and Type.size_of(?*u32) == 8 and Type.align_of(?*u32) == 8
                and Type.size_of(??*u32) == 16 and Type.align_of(??*u32) == 8
                and Type.size_of(?char) == 4 and Type.align_of(?char) == 4
                and Type.size_of(?TriColor) == 1 and Type.align_of(?TriColor) == 1
                and Type.size_of(?Packet) == 1 and Type.align_of(?Packet) == 1
                and Type.size_of(??u64) == 16 and Type.align_of(??u64) == 8
                and Type.size_of(?Flagged) == 16 and Type.align_of(?Flagged) == 8
                and Type.size_of(?PtrBox) == 16 and Type.align_of(?PtrBox) == 8
                and Type.size_of(?Tagged) == 16 and Type.align_of(?Tagged) == 8
                and Type.size_of(??Tagged) == 16 and Type.align_of(??Tagged) == 8
                and Type.size_of(SingleWrap) == 1 and Type.align_of(SingleWrap) == 1
                and Type.size_of(SinglePair) == 16 and Type.align_of(SinglePair) == 8
                and Type.size_of(MaybePtr) == 8 and Type.align_of(MaybePtr) == 8
                and Type.size_of(MaybeBool) == 1 and Type.align_of(MaybeBool) == 1
                and Type.size_of(PairOpt) == 16 and Type.align_of(PairOpt) == 8
                and Type.size_of(?OwnedWithBool) == 16 and Type.align_of(?OwnedWithBool) == 8
                and Type.size_of(??OwnedNoNiche) == 16 and Type.align_of(??OwnedNoNiche) == 8
                and Type.size_of(OwnedChoice) == 16 and Type.align_of(OwnedChoice) == 8
        }}

        fn check_unit_enums(flag: bool) -> bool {{
            let s = SoloUnit.only;
            let s_ok = s'tag_name == "only" and (s match {{ SoloUnit.only => true }});
            let c = if flag {{ TriColor.green }} else {{ TriColor.blue }};
            let c_ok = c'tag_name == "green" and (c match {{ TriColor.green => true, _ => false }});
            let b = Big257.v256;
            let b_ok = b'tag_name == "v256" and (b match {{ Big257.v256 => true, _ => false }});
            s_ok and c_ok and b_ok
        }}

        fn check_nested_optionals() -> bool {{
            let b0: ??bool = null;
            let b1_inner: ?bool = null;
            let b1: ??bool = b1_inner;
            let b2_inner: ?bool = false;
            let b2: ??bool = b2_inner;
            let b3_inner: ?bool = true;
            let b3: ??bool = b3_inner;

            let s0: u8 = b0 match {{ null => 0, _? => 9 }};
            let s1: u8 = b1 match {{ null => 9, inner? => inner match {{ null => 1, _? => 8 }} }};
            let s2: u8 = b2 match {{ null => 9, inner? => inner match {{ null => 8, v? => if v {{ 9 }} else {{ 2 }} }} }};
            let s3: u8 = b3 match {{ null => 9, inner? => inner match {{ null => 8, v? => if v {{ 3 }} else {{ 9 }} }} }};

            var target: u32 = 42;
            let p0: ?*u32 = null;
            let p1: ?*u32 = target.ref;
            let pp0: ??*u32 = null;
            let pp1_in: ?*u32 = null;
            let pp1: ??*u32 = pp1_in;
            let pp2: ??*u32 = p1;

            let pp0_c: u8 = pp0 match {{ null => 0, _? => 9 }};
            let pp1_c: u8 = pp1 match {{ null => 9, inner? => inner match {{ null => 1, _? => 8 }} }};
            let pp2_c: u8 = pp2 match {{ null => 9, inner? => inner match {{ null => 8, r? => if r.* == 42 {{ 2 }} else {{ 9 }} }} }};
            let p_ok = (p0 match {{ null => true, _? => false }})
                and pp0_c == 0
                and pp1_c == 1
                and pp2_c == 2;

            let c_null: ?TriColor = null;
            let c_some: ?TriColor = TriColor.red;
            let c_ok = (c_null match {{ null => true, _? => false }})
                and (c_some match {{ null => false, v? => v'tag_name == "red" }});

            let pk_null: ?Packet = null;
            let pk_low: ?Packet = Packet.low;
            let pk_ok = (pk_null match {{ null => true, _? => false }})
                and (pk_low match {{ null => false, v? => v'tag_name == "low" }});

            let ch_null: ?char = null;
            let ch_ok_val: ?char = 'A';
            let ch_ok = (ch_null match {{ null => true, _? => false }})
                and (ch_ok_val match {{ null => false, c? => c == 'A' }});

            let f_null: ?Flagged = null;
            let f_val: ?Flagged = Flagged {{ lead: 7, flag: false, tail: 99 }};
            let f_ok = (f_null match {{ null => true, _? => false }})
                and (f_val match {{ null => false, f? => f.lead == 7 and not f.flag and f.tail == 99 }});

            let pb_null: ?PtrBox = null;
            let pb_val: ?PtrBox = PtrBox {{ ptr: target.ref, tag: 55 }};
            let pb_ok = (pb_null match {{ null => true, _? => false }})
                and (pb_val match {{ null => false, b? => b.ptr.* == 42 and b.tag == 55 }});

            let t0: ??Tagged = null;
            let t1_in: ?Tagged = null;
            let t1: ??Tagged = t1_in;
            let t2_in: ?Tagged = Tagged.num(123);
            let t2: ??Tagged = t2_in;
            let t_ok = (t0 match {{ null => true, _? => false }})
                and (t1 match {{ null => false, i? => i match {{ null => true, _? => false }} }})
                and (t2 match {{ null => false, i? => i match {{ null => false, v? => v match {{ Tagged.num(n) => n == 123, _ => false }} }} }});

            let u0: ??u64 = null;
            let u1_in: ?u64 = null;
            let u1: ??u64 = u1_in;
            let u2: ??u64 = 777;
            let u0_c: u8 = u0 match {{ null => 0, _? => 9 }};
            let u1_c: u8 = u1 match {{ null => 9, i? => i match {{ null => 1, _? => 8 }} }};
            let u2_c: u8 = u2 match {{ null => 9, i? => i match {{ null => 8, v? => if v == 777 {{ 2 }} else {{ 9 }} }} }};
            let u_ok = u0_c == 0 and u1_c == 1 and u2_c == 2;

            s0 == 0 and s1 == 1 and s2 == 2 and s3 == 3
                and p_ok and c_ok and pk_ok and ch_ok and f_ok and pb_ok and t_ok and u_ok
        }}

        fn check_niche_enums() -> bool {{
            var target: u32 = 91;
            let sw = SingleWrap.wrap(true);
            let sp = SinglePair.pair(9, 88);
            let sw_ok = sw match {{ SingleWrap.wrap(b) => b }};
            let sp_ok = sp match {{ SinglePair.pair(a, b) => a == 9 and b == 88 }};

            let mp_e = MaybePtr.empty;
            let mp_s = MaybePtr.some(target.ref);
            let mp_ok = mp_e'tag_name == "empty"
                and mp_s'tag_name == "some"
                and (mp_e match {{ MaybePtr.empty => true, _ => false }})
                and (mp_s match {{ MaybePtr.some(p) => p.* == 91, _ => false }});

            let mb_e = MaybeBool.empty;
            let mb_a = MaybeBool.alt;
            let mb_f = MaybeBool.some(false);
            let mb_t = MaybeBool.some(true);
            let mb_ok = mb_e'tag_name == "empty"
                and mb_a'tag_name == "alt"
                and mb_f'tag_name == "some"
                and mb_t'tag_name == "some"
                and (mb_e match {{ MaybeBool.empty => true, _ => false }})
                and (mb_a match {{ MaybeBool.alt => true, _ => false }})
                and (mb_f match {{ MaybeBool.some(v) => not v, _ => false }})
                and (mb_t match {{ MaybeBool.some(v) => v, _ => false }});

            let po_e = PairOpt.empty;
            let po_p = PairOpt.pair(true, 314);
            let po_ok = po_e'tag_name == "empty"
                and po_p'tag_name == "pair"
                and (po_e match {{ PairOpt.empty => true, _ => false }})
                and (po_p match {{ PairOpt.pair(f, v) => f and v == 314, _ => false }});

            sw_ok and sp_ok and mp_ok and mb_ok and po_ok
        }}

        fn exercise_niche_drops() -> bool {{
            do {{
                let _opt_none: ?OwnedWithBool = null;
                let _opt_false: ?OwnedWithBool = OwnedWithBool {{ flag: false, id: 1 }};
                let _opt_true: ?OwnedWithBool = OwnedWithBool {{ flag: true, id: 2 }};
                let _nest_outer: ??OwnedNoNiche = null;
                let _inner_none: ?OwnedNoNiche = null;
                let _nest_inner: ??OwnedNoNiche = _inner_none;
                let _nest_some: ??OwnedNoNiche = OwnedNoNiche {{ id: 3 }};
                let _enum_empty = OwnedChoice.empty;
                let _enum_alt = OwnedChoice.alt;
                let _enum_active = OwnedChoice.active(OwnedWithBool {{ flag: true, id: 4 }});
            }};
            true
        }}

        fn run() -> bool {{
            check_sizes()
                and check_unit_enums(true)
                and check_nested_optionals()
                and check_niche_enums()
                and exercise_niche_drops()
        }}
        "#
    );
    for opt in ["0", "3"] {
        let case = NativeCase::new(&source);
        let build = case.build(opt);
        assert_success(&build);
        let output = case.run();
        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let drop_lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("drop-")).collect();
        assert!(
            drop_lines
                == vec![
                    "drop-bool:4-T",
                    "drop-no:3",
                    "drop-bool:2-T",
                    "drop-bool:1-F"
                ],
            "opt={opt}: {stdout}"
        );
        assert!(stdout.ends_with("(bool true)\n"), "opt={opt}: {stdout}");
    }
}

#[test]
fn base_type_unsigned_encodings_groups_and_reflection_work_natively() {
    let source = r#"
        use std.prelude.*;

        const CODE8: u8 = 0x05;
        enum E8 {
            .base_type u8,
            .discrete_tag true,
            zero: 0,
            mid: < CODE8 > ,
            grp.{ c0 = 16, c1 = 17, c2: 18, c3: 19 },
            pfx.{ p0: 0b0010 (x: BitVec<4>), p1: 0b0011 (y: BitVec<4>) },
            midg.{ m0: 0b10 (a: BitVec<4>) 01, m1: 0b10 (b: BitVec<4>) 10 },
        }

        const K16: u16 = 0x1234;
        enum E16 {
            .base_type u16,
            .discrete_tag true,
            empty = 0,
            fixed = K16,
            hi: 0xAA (lo: u8),
        }

        const K32: u32 = 0x1234_5678;
        enum E32 {
            .base_type u32,
            .discrete_tag true,
            empty: 0,
            fixed: < K32 > ,
            op: 0x5 (val: u16) ABC,
        }

        const K64: u64 = 0x0102_0304_0506_0708;
        enum E64 {
            .base_type u64,
            .discrete_tag true,
            empty = 0,
            fixed = K64,
            pkt: 0xFE (body: u32) CAFE_01,
        }

        const K128: u128 = 1267650600228229401496703205376;
        enum E128 {
            .base_type u128,
            .discrete_tag true,
            empty = 0,
            fixed = K128,
            big: 0x5A (mid: u64) 0000_0000_0000_01,
        }

        const KUSIZE: usize = 99;
        enum EPos {
            .base_type usize,
            .discrete_tag true,
            zero = 0,
            alt: < KUSIZE > ,
        }

        enum UGrp {
            a.{ x, y, z },
            tail,
        }

        enum TGrp {
            a.{ x(u64), y(u32) },
            none,
        }

        enum NGrp {
            grp.{ k0, k1 },
            other,
            pay(bool),
        }

        enum NGrpData {
            none,
            grp.{ keep, pay(bool) },
        }

        comptime fn base_is(subject: Type, expected: Type) -> bool {
            subject.enum_bit_base_type() match {
                found? => found == expected,
                null => false,
            }
        }

        const REFLECT_OK: bool = do {
            let bases_ok = base_is(E8, u8)
                and base_is(E16, u16)
                and base_is(E32, u32)
                and base_is(E64, u64)
                and base_is(E128, u128)
                and base_is(EPos, usize);
            let widths_ok = E8.enum_bit_width() == 8
                and E16.enum_bit_width() == 16
                and E32.enum_bit_width() == 32
                and E64.enum_bit_width() == 64
                and E128.enum_bit_width() == 128
                and (if Type.size_of(usize) == 8 { EPos.enum_bit_width() == 64 } else { EPos.enum_bit_width() == 32 });
            let masks = E8.enum_bit_masks();
            let values = E8.enum_bit_values();
            let offsets = E8.enum_bit_offsets();
            let groups = E8.enum_group_paths();
            let meta8_ok = masks.length() == 10
                and values.length() == 10
                and offsets.length() == 10
                and masks[0] == 255
                and values[1] == 5
                and values[2] == 16
                and masks[6] == 240
                and values[6] == 32
                and offsets[0].length() == 0
                and offsets[6].length() == 1
                and offsets[6][0] == 0
                and offsets[8].length() == 1
                and offsets[8][0] == 2
                and groups.length() == 3;
            bases_ok and widths_ok and meta8_ok
        };

        fn in_e8_grp(v: E8) -> bool {
            v match { .grp => true, _ => false }
        }

        fn in_e8_pfx(v: E8) -> bool {
            v match { .pfx => true, _ => false }
        }

        fn in_e8_midg(v: E8) -> bool {
            v match { .midg => true, _ => false }
        }

        fn check_e8() -> bool {
            let vals_ok = (E8.zero'bitvec).as(u8) == 0
                and (E8.mid'bitvec).as(u8) == 5
                and (E8.grp.c0'bitvec).as(u8) == 16
                and (E8.grp.c3'bitvec).as(u8) == 19
                and (E8.pfx.p0 { x: 0x0 }'bitvec).as(u8) == 0x20
                and (E8.pfx.p1 { y: 0xf }'bitvec).as(u8) == 0x3f
                and (E8.midg.m0 { a: 0xf }'bitvec).as(u8) == 0b1011_1101
                and (E8.midg.m1 { b: 0x5 }'bitvec).as(u8) == 0b1001_0110;

            let tags_ok = E8.zero'tag_name == "zero"
                and E8.mid'tag_name == "mid"
                and g2_tag(E8.grp.c3) == "grp.c3"
                and E8.pfx.p1 { y: 0xf }'tag_name == "pfx.p1"
                and E8.midg.m0 { a: 0xf }'tag_name == "midg.m0";

            let grp_ok = in_e8_grp(E8.grp.c0) and in_e8_grp(E8.grp.c3)
                and not in_e8_grp(E8.mid)
                and not in_e8_grp(E8.pfx.p0 { x: 0x0 })
                and not in_e8_grp(E8.midg.m0 { a: 0xf });
            let pfx_ok = in_e8_pfx(E8.pfx.p0 { x: 0x0 })
                and in_e8_pfx(E8.pfx.p1 { y: 0xf })
                and not in_e8_pfx(E8.grp.c3)
                and not in_e8_pfx(E8.midg.m0 { a: 0xf })
                and not in_e8_pfx(E8.zero);
            let mid_ok = in_e8_midg(E8.midg.m0 { a: 0xf })
                and in_e8_midg(E8.midg.m1 { b: 0x5 })
                and not in_e8_midg(E8.pfx.p1 { y: 0xf })
                and not in_e8_midg(E8.grp.c0);
            let p1 = E8.pfx.p1 { y: 0xf };
            let extract_ok = p1 match {
                E8.pfx.p1 { y: yv } => yv.as(u8) == 15,
                _ => false,
            };

            vals_ok and tags_ok and grp_ok and pfx_ok and mid_ok and extract_ok
        }

        fn g2_tag(v: E8) -> str {
            v'tag_name
        }

        fn check_wide_bases() -> bool {
            let ok16 = Type.size_of(E16) == 2 and Type.align_of(E16) == 2
                and (E16.empty'bitvec).as(u16) == 0
                and (E16.fixed'bitvec).as(u16) == K16
                and (E16.hi { lo: 0x3C }'bitvec).as(u16) == 0xAA3C
                and (E16.fixed match { .fixed => true, _ => false })
                and (E16.hi { lo: 0x3C } match { .hi { lo: l } => l == 0x3C, _ => false });

            let ok32 = Type.size_of(E32) == 4 and Type.align_of(E32) == 4
                and (E32.empty'bitvec).as(u32) == 0
                and (E32.fixed'bitvec).as(u32) == K32
                and (E32.op { val: 0x4242 }'bitvec).as(u32) == 0x5424_2ABC
                and (E32.empty match { .empty => true, _ => false })
                and (E32.op { val: 0x4242 } match { .op { val: v } => v == 0x4242, _ => false });

            let ok64 = Type.size_of(E64) == 8 and Type.align_of(E64) == 8
                and (E64.empty'bitvec).as(u64) == 0
                and (E64.fixed'bitvec).as(u64) == K64
                and (E64.pkt { body: 0xDEAD_BEEF }'bitvec).as(u64) == 0xFE_DEAD_BEEF_CAFE_01
                and (E64.fixed match { .fixed => true, _ => false })
                and (E64.pkt { body: 0xDEAD_BEEF } match { .pkt { body: b } => b == 0xDEAD_BEEF, _ => false });

            let ok128 = Type.size_of(E128) == 16 and Type.align_of(E128) == 16
                and (E128.empty'bitvec).as(u128) == 0
                and (E128.fixed'bitvec).as(u128) == K128
                and (E128.big { mid: 0x0123_4567_89AB_CDEF }'bitvec).as(u128) == 0x5A_0123_4567_89AB_CDEF_0000_0000_0000_01
                and (E128.fixed match { .fixed => true, _ => false })
                and (E128.big { mid: 0x0123_4567_89AB_CDEF } match { .big { mid: m } => m == 0x0123_4567_89AB_CDEF, _ => false });

            let ok_pos = Type.size_of(EPos) == Type.size_of(usize)
                and Type.align_of(EPos) == Type.align_of(usize)
                and (EPos.zero'bitvec).as(usize) == 0
                and (EPos.alt'bitvec).as(usize) == KUSIZE
                and EPos.alt'tag_name == "alt"
                and (EPos.alt match { .alt => true, _ => false });

            ok16 and ok32 and ok64 and ok128 and ok_pos
        }

        fn check_other_groups() -> bool {
            let u_ok = (UGrp.a.x match { .a => true, _ => false })
                and (UGrp.a.z match { .a => true, _ => false })
                and not (UGrp.tail match { .a => true, _ => false });

            let t_ok = (TGrp.a.x(10) match { .a => true, _ => false })
                and (TGrp.a.y(20) match { .a => true, _ => false })
                and not (TGrp.none match { .a => true, _ => false });

            let n_ok = Type.size_of(NGrp) == 1
                and (NGrp.grp.k0 match { .grp => true, _ => false })
                and (NGrp.grp.k1 match { .grp => true, _ => false })
                and not (NGrp.other match { .grp => true, _ => false })
                and not (NGrp.pay(false) match { .grp => true, _ => false });

            let nd_ok = Type.size_of(NGrpData) == 1
                and (NGrpData.grp.keep match { .grp => true, _ => false })
                and (NGrpData.grp.pay(true) match { .grp => true, _ => false })
                and not (NGrpData.none match { .grp => true, _ => false });

            u_ok and t_ok and n_ok and nd_ok
        }

        fn run() -> bool {
            REFLECT_OK and check_e8() and check_wide_bases() and check_other_groups()
        }
    "#;
    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        let build = case.build(opt);
        assert_success(&build);
        let output = case.run();
        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.ends_with("(bool true)\n"), "opt={opt}: {stdout}");
    }

    let usize_probe = r#"
        const TARGET_CODE: usize = 77;
        enum TargetTag {
            .base_type usize,
            .discrete_tag true,
            empty: 0,
            marked: < TARGET_CODE > ,
        }

        comptime fn is_usize_base(subject: Type) -> bool {
            subject.enum_bit_base_type() match {
                found? => found == usize,
                null => false,
            }
        }

        const WIDTH_OK: bool = TargetTag.enum_bit_width() == 32;
        const BASE_OK: bool = is_usize_base(TargetTag);

        fn run() -> bool {
            WIDTH_OK
                and Type.size_of(TargetTag) == 4
                and Type.align_of(TargetTag) == 4
                and BASE_OK
        }
    "#;
    let probe64 = r#"
        const TARGET_CODE: usize = 77;
        enum TargetTag {
            .base_type usize,
            .discrete_tag true,
            empty: 0,
            marked: < TARGET_CODE > ,
        }
        const WIDTH64_OK: bool = TargetTag.enum_bit_width() == 64;
        fn run() -> bool {
            WIDTH64_OK
                and Type.size_of(TargetTag) == 8
                and Type.align_of(TargetTag) == 8
        }
    "#;
    let prefixed32 = r#"
        enum TargetTag {
            .base_type usize,
            .discrete_tag true,
            prefixed: 0x0F (low: u16) FF,
        }
        const WIDTH32_OK: bool = TargetTag.enum_bit_width() == 32;
        fn run() -> bool {
            WIDTH32_OK and Type.size_of(TargetTag) == 4
        }
    "#;
    assert_eq!(
        run_with_target(usize_probe, "x86", "linux", "32").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        run_with_target(usize_probe, "x86_64", "linux", "64").unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        run_with_target(probe64, "x86_64", "linux", "64").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        run_with_target(prefixed32, "x86", "linux", "32").unwrap(),
        Value::Bool(true)
    );
    assert!(run_with_target(prefixed32, "x86_64", "linux", "64").is_err());
}

#[test]
fn type_stride_bit_width_field_offset_and_vars_work_end_to_end() {
    let source = r#"
        struct Padded {
            a: u8,
            b: u64,
            c: u16,
        }

        ^attrs.packed.byte
        struct BytePacked {
            a: u8,
            b: u64,
        }

        ^attrs.packed.bit
        struct BitPacked {
            low: BitVec<5>,
            mid: BitVec<3>,
            high: u8,
        }

        newtype FiveBits = BitVec<5>;

        enum TriColor { red, green, blue }
        enum FixedBits3 {
            .width 3,
            .discrete_tag true,
            a: 0,
            b: 1,
        }

        const PADDED_STRIDE: usize = Type.stride_of(Padded);
        const PADDED_BITS: usize = Type.bit_width_of(Padded);
        const PADDED_B_OFF: usize = Type.field_offset_of(Padded, .b);
        const PADDED_C_OFF: usize = Type.field_offset_of(Padded, .c);
        const BP_STRIDE: usize = Type.stride_of(BytePacked);
        const BP_B_OFF: usize = Type.field_offset_of(BytePacked, .b);
        const BITP_BITS: usize = Type.bit_width_of(BitPacked);
        const BITP_HIGH_OFF: usize = Type.field_offset_of(BitPacked, .high);

        fn generic_stride() -> usize where T {
            Type.stride_of(T)
        }

        fn accept_vars(x: T) -> T
        where T,
              r: Type.vars.EffectRow,
              e: Type.vars.ErrorRow,
              p: Type.vars.Reachability,
              a: Type.vars.Any
        {
            x
        }

        fn forward_vars(x: T) -> T
        where T,
              r2: Type.vars.EffectRow,
              e2: Type.vars.ErrorRow,
              p2: Type.vars.Reachability,
              a2: Type.vars.Any
        {
            accept_vars<T, r2, e2, p2, a2>(x)
        }

        fn run() -> bool {
            let vars_ok = true;
            vars_ok
                and PADDED_STRIDE == 24
                and PADDED_BITS == 192
                and PADDED_B_OFF == 8
                and PADDED_C_OFF == 16
                and BP_STRIDE == 9
                and BP_B_OFF == 1
                and BITP_BITS == 16
                and BITP_HIGH_OFF == 1
                and Type.stride_of(u32) == 4
                and generic_stride<Padded>() == 24
                and Type.bit_width_of(void) == 0
                and Type.bit_width_of(bool) == 1
                and Type.bit_width_of(BitVec<5>) == 5
                and Type.bit_width_of(FiveBits) == 5
                and Type.bit_width_of(TriColor) == 8
                and Type.bit_width_of(FixedBits3) == 3
                and Type.field_offset_of(Padded, .a) == 0
                and Type.field_offset_of(Padded, .b) == 8
                and Type.field_offset_of(BytePacked, .b) == 1
                and Type.field_offset_of(BitPacked, .low) == 0
                and Type.field_offset_of(BitPacked, .high) == 1
                and not Type.predicates.is_not_sized(Padded)
        }
    "#;
    assert_eq!(
        run_with_target(source, "x86_64", "linux", "64").unwrap(),
        Value::Bool(true)
    );
    for level in ["0", "3"] {
        let case = NativeCase::new(source);
        let built = case.build(level);
        assert_success(&built);
        let ran = case.run();
        assert_success(&ran);
        assert_eq!(ran.stdout, b"(bool true)\n");
    }

    // Target sensitivity of bit_width_of(usize).
    let width_probe = r#"
        fn run() -> usize { Type.bit_width_of(usize) }
    "#;
    assert_eq!(
        run_with_target(width_probe, "x86", "linux", "32").unwrap(),
        Value::UnsignedInteger(32)
    );
    assert_eq!(
        run_with_target(width_probe, "x86_64", "linux", "64").unwrap(),
        Value::UnsignedInteger(64)
    );

    // Negative cases: missing field, unaligned bit-packed field, comptime-only types & Type.vars.*.
    let bad_field_const = r#"
        struct Pair { a: u8, b: u64 }
        const BAD: usize = Type.field_offset_of(Pair, .missing);
        fn run() -> usize { BAD }
    "#;
    assert!(support::compile(bad_field_const).is_err());

    let bad_field_rt = r#"
        struct Pair { a: u8, b: u64 }
        fn run() -> usize { Type.field_offset_of(Pair, .missing) }
    "#;
    assert!(run_with_target(bad_field_rt, "x86_64", "linux", "64").is_err());

    let unaligned_bit_field = r#"
        ^attrs.packed.bit
        struct Bits { a: BitVec<5>, b: BitVec<3> }
        const BAD: usize = Type.field_offset_of(Bits, .b);
        fn run() -> usize { BAD }
    "#;
    assert!(support::compile(unaligned_bit_field).is_err());

    let wrong_var_domain = r#"
        fn accept_effect_var(x: u32) -> u32
        where r: Type.vars.EffectRow
        {
            x
        }
        fn run() -> u32 { accept_effect_var<void>(1) }
    "#;
    assert!(support::compile(wrong_var_domain).is_err());

    for var_ty in [
        "Type.vars.Closure",
        "Type.vars.EffectRow",
        "Type.vars.ErrorRow",
        "Type.vars.Reachability",
        "Type.vars.Any",
    ] {
        for q in ["size_of", "stride_of", "bit_width_of"] {
            let bad = format!("const BAD: usize = Type.{q}({var_ty}); fn run() -> usize {{ BAD }}");
            assert!(
                support::compile(&bad).is_err(),
                "Type.{q}({var_ty}) must fail"
            );
        }
    }
}
