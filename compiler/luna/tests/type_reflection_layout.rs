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
                and Type.size_of(!u8 u64) == 24 and Type.align_of(!u8 u64) == 8
                and Type.size_of(Padded) == 12 and Type.align_of(Padded) == 4
                and Type.size_of(Link) == 24 and Type.align_of(Link) == 8
                and Type.size_of(Pair<u8, u64>) == 16 and Type.align_of(Pair<u8, u64>) == 8
                and Type.size_of(Wrapper) == 8 and Type.align_of(Wrapper) == 8
                and Type.size_of(Color) == 4 and Type.align_of(Color) == 4
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
