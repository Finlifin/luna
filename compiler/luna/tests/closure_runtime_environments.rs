//! End-to-end tests for closure operational runtime-environment delivery
//! separated from user-visible semantic capture fields (`GAP-02-22`,
//! `DIV-02-03`, `SND-02-02`).

use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-closure-env-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
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

fn assert_native_true_o0_and_o3(source: &str) {
    for opt_level in ["0", "3"] {
        let case = NativeCase::new(source);
        let build = case.build(opt_level);
        assert_success(&build);
        let run = case.run();
        assert_success(&run);
        assert_eq!(
            run.stdout, b"(bool true)\n",
            "unexpected output at -O{opt_level}"
        );
    }
}

#[test]
fn generic_and_explicit_closure_factories_share_abi_with_zero_capture_pollution() {
    let source = r#"
        struct Cell {
            value: i32,
        }

        fn snapshot(c: *Cell) -> i32 {
            c.value.*
        }

        struct Payload
        where
            quote src: lift *Cell,
            quote .saved: lift i32 = snapshot(src),
        {
            tag: i32,
        }

        trait Inspect {
            fn inspect(self) -> bool;
        }

        impl Inspect for Payload<src, .saved = n>
        where
            quote src: lift *Cell,
            quote n: lift i32,
        {
            fn inspect(self) -> bool {
                src.value.* == 42 and n == 7 and self.tag == 9
            }
        }

        fn generic_factory(v: T) -> %{ v: T } fn() -> bool
        where
            T :- Inspect,
        {
            || v.inspect()
        }

        fn explicit_factory(
            v: Payload<src, .saved = n>,
        ) -> %{ v: Payload<src, .saved = n> } fn() -> bool
        where
            quote src: lift *Cell,
            quote n: lift i32,
        {
            || v.inspect()
        }

        fn nested_factory(v: T) -> %{ v: T } fn() -> bool
        where
            T :- Inspect,
        {
            || {
                let inner = || v.inspect();
                inner()
            }
        }

        fn choose_factory(
            use_generic: bool,
            v: Payload<src, .saved = n>,
        ) -> %{ v: Payload<src, .saved = n> } fn() -> bool
        where
            quote src: lift *Cell,
            quote n: lift i32,
        {
            if use_generic {
                generic_factory(v)
            } else {
                explicit_factory(v)
            }
        }

        fn invoke(f: %c fn() -> bool) -> bool
        where
            c: Type.vars.Closure,
        {
            f()
        }

        fn run() -> bool {
            var cell = Cell { value: 7 };

            let p1 = Payload<cell.ref> { tag: 9 };
            let p2 = Payload<cell.ref> { tag: 9 };
            let p3 = Payload<cell.ref> { tag: 9 };
            let p4 = Payload<cell.ref> { tag: 9 };
            let p5 = Payload<cell.ref> { tag: 9 };

            let f_generic = generic_factory(p1);
            let f_explicit = explicit_factory(p2);
            let f_nested = nested_factory(p3);
            let f_choose_a = choose_factory(true, p4);
            let f_choose_b = choose_factory(false, p5);

            -- Semantic capture layout must hold only `v: Payload<...>` (same size as
            -- `%{ v: i32 } fn() -> bool`), never polluting the closure struct with
            -- operational `src` or `saved`.
            let size_ok = Type.size_of(%{ v: Payload<cell.ref> } fn() -> bool)
                == Type.size_of(%{ v: i32 } fn() -> bool)
                and Type.size_of(%{ v: Payload<cell.ref> } fn() -> bool)
                    < Type.size_of(%{ v: i32, src: *Cell, saved: i32 } fn() -> bool);

            cell.value = 42;

            size_ok
                and f_generic()
                and invoke(f_explicit)
                and f_nested()
                and f_choose_a()
                and invoke(f_choose_b)
        }
    "#;
    assert_native_true_o0_and_o3(source);
}

#[test]
fn capture_free_and_runtime_field_closures_deliver_operational_env() {
    let source = r#"
        struct Pair {
            first: i32,
            second: i32,
        }

        struct FieldView
        where
            quote src: lift *Pair,
            quote .field: lift *i32 = src.first,
        {
            delta: i32,
        }

        trait ReadDelta {
            fn read(self) -> i32;
        }

        impl ReadDelta for FieldView<src, .field = f>
        where
            quote src: lift *Pair,
            quote f: lift *i32,
        {
            fn read(self) -> i32 {
                if f.* >= -1000 and f.* <= 1000 and self.delta >= -1000 and self.delta <= 1000 {
                    f.* + self.delta
                } else {
                    0
                }
            }
        }

        fn make_capture_free_reader() -> fn(T) -> i32
        where
            T :- ReadDelta,
        {
            |v| v.read()
        }

        fn make_explicit_reader() -> fn(FieldView<src, .field = f>) -> i32
        where
            quote src: lift *Pair,
            quote f: lift *i32,
        {
            |v| v.read()
        }

        fn make_capturing_reader(v: T) -> %{ v: T } fn() -> i32
        where
            T :- ReadDelta,
        {
            || v.read()
        }

        fn apply_reader(reader: fn(T) -> i32, value: T) -> i32
        where
            T,
        {
            reader(value)
        }

        fn run() -> bool {
            var pair = Pair { first: 10, second: 99 };
            let v1 = FieldView<pair.ref> { delta: 5 };
            let v2 = FieldView<pair.ref> { delta: 6 };
            let v3 = FieldView<pair.ref> { delta: 7 };

            let r_generic = make_capture_free_reader<FieldView<pair.ref>>();
            let r_explicit = make_explicit_reader<pair.ref, pair.ref.first>();
            let r_capture = make_capturing_reader(v3);

            let size_ok = Type.size_of(%{ v: FieldView<pair.ref> } fn() -> i32)
                == Type.size_of(%{ v: i32 } fn() -> i32)
                and Type.size_of(%{ v: FieldView<pair.ref> } fn() -> i32)
                    < Type.size_of(%{ v: i32, src: *Pair, field: *i32 } fn() -> i32);

            pair.first = 30;

            size_ok
                and apply_reader(r_generic, v1) == 35
                and r_explicit(v2) == 36
                and r_capture() == 37
        }
    "#;
    assert_native_true_o0_and_o3(source);
}

#[test]
fn sysroot_box_and_vec_string_callbacks_and_debug_work_at_o0_and_o3() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{AllocErr, Allocator, DefaultAllocator, FixedBufferAllocator, Layout, SystemAllocator};

        fn check_box_debug(
            b: *Box<T, .A = A, .allocator = a>,
            expected: str,
        ) -> !AllocErr bool
        where
            T :- Debug,
            A :- Allocator,
            quote a: lift *A,
        {
            let formatted = format_debug<.A = A, .allocator = a>(b)!;
            formatted.ref.equals_str(expected)
        }

        fn check_vec_debug(
            v: *Vec<T, .A = A, .allocator = a>,
            expected: str,
        ) -> !AllocErr bool
        where
            T :- Debug,
            A :- Allocator,
            quote a: lift *A,
        {
            let formatted = format_debug<.A = A, .allocator = a>(v)!;
            formatted.ref.equals_str(expected)
        }

        fn ok_void() -> !AllocErr void {}

        fn run_default_alloc() -> !AllocErr bool {
            var boxed = Box<String>.new(String.from_str("hello\n")!)!;
            let boxed_dbg = format_debug(boxed.ref)!;
            let box_ok = boxed_dbg.ref.equals_str("\"hello\\n\"")
                and boxed.ref.with_ref(|inner| inner.equals_str("hello\n"));
            if not box_ok {
                return false;
            }

            var words = Vec<String>.new();
            words.ref.push(String.from_str("alpha")!)!;
            words.ref.push(String.from_str("beta\t")!)!;
            let vec_dbg = format_debug(words.ref)!;
            let vec_ok = vec_dbg.ref.equals_str("[\"alpha\", \"beta\\t\"]");
            var total_len: usize = 0;
            let total = total_len.ref;
            let count_ok = words.ref.try_for_each_ref(|item| {
                let n = item.len();
                if total.* <= 1000 and n <= 1000 {
                    total.* = total.* + n;
                }
                ok_void()
            }) match {
                ()! => total_len == 10,
                error _ => false,
            };
            vec_ok and count_ok
        }

        fn run_custom_fba(fba: *FixedBufferAllocator) -> !AllocErr bool {
            var boxed = Box<
                String<.A = FixedBufferAllocator, .allocator = fba>,
                .A = FixedBufferAllocator,
                .allocator = fba,
            >.new(String<.A = FixedBufferAllocator, .allocator = fba>.from_str("custom")!)!;
            let box_ok = check_box_debug(boxed.ref, "\"custom\"")!
                and boxed.ref.with_ref(|inner| inner.equals_str("custom"));
            if not box_ok {
                return false;
            }

            var words = Vec<
                String<.A = FixedBufferAllocator, .allocator = fba>,
                .A = FixedBufferAllocator,
                .allocator = fba,
            >.new();
            words.ref.push(String<.A = FixedBufferAllocator, .allocator = fba>.from_str("x")!)!;
            words.ref.push(String<.A = FixedBufferAllocator, .allocator = fba>.from_str("yz\"")!)!;
            let vec_ok = check_vec_debug(words.ref, "[\"x\", \"yz\\\"\"]")!;
            var total_len: usize = 0;
            let total = total_len.ref;
            let iter_ok = words.ref.try_for_each_ref(|item| {
                let n = item.len();
                if total.* <= 1000 and n <= 1000 {
                    total.* = total.* + n;
                }
                ok_void()
            }) match {
                ()! => total_len == 4,
                error _ => false,
            };
            vec_ok and iter_ok
        }

        fn run() -> bool {
            let default_ok = run_default_alloc() match {
                ok! => ok,
                error _ => false,
            };
            if not default_ok {
                return false;
            }

            let layout = Layout.new(32768, 16) match {
                found? => found,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let fba_ok = run_custom_fba(fba.ref) match {
                ok! => ok,
                error _ => false,
            };
            let clean = fba.ref.live_allocations() == 0;
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            fba_ok and clean
        }
    "#;
    assert_native_true_o0_and_o3(source);
}

fn assert_build_rejected(source: &str, label: &str, expected_substrings: &[&str]) {
    let case = NativeCase::new(source);
    let output = case.build("0");
    assert!(
        !output.status.success(),
        "{label}: expected build rejection, but build succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        expected_substrings
            .iter()
            .any(|needle| stderr.contains(needle)),
        "{label}: expected diagnostic matching {:?}, got:\n{stderr}",
        expected_substrings
    );
}

#[test]
fn soundness_rejections_for_closure_operational_runtime_environments() {
    let base_defs = r#"
        struct Cell {
            value: i32,
            pub const GLOBAL: Cell = Cell { value: 1 };
        }

        impl Splice for Cell {
            assoc RuntimeTarget: Type = Cell;
            fn splice(comptime data: Cell) -> Cell {
                Cell { value: data.value }
            }
        }

        enum Sample where quote src: lift *Cell { empty, }

        trait Inspect {
            fn inspect(self) -> i32;
        }

        impl Inspect for Sample<src> where quote src: lift *Cell {
            fn inspect(self) -> i32 {
                src.value.*
            }
        }

        fn factory(v: T) -> %{ v: T } fn() -> i32
        where
            T :- Inspect,
        {
            || v.inspect()
        }

        fn consume(c: Cell) {}
    "#;

    // 1. Escaping local storage via generic closure factory.
    let escaping_local = format!(
        r#"{base_defs}
        fn run() -> i32 {{
            let f = do {{
                var cell = Cell {{ value: 7 }};
                let v = Sample<cell.ref>.empty;
                factory(v)
            }};
            f()
        }}
    "#
    );
    assert_build_rejected(
        &escaping_local,
        "escaping local storage",
        &["no longer in scope"],
    );

    // 2. Moving/consuming backing storage before calling closure from generic factory.
    let moved_backing = format!(
        r#"{base_defs}
        fn run() -> i32 {{
            var cell = Cell {{ value: 7 }};
            let v = Sample<cell.ref>.empty;
            let f = factory(v);
            consume(cell);
            f()
        }}
    "#
    );
    assert_build_rejected(
        &moved_backing,
        "moved backing storage",
        &["memory call target unavailable: NotPresent", "moved"],
    );

    // 3. Reassigning runtime-dependency pointer binding after capture.
    let reassigned_binding = format!(
        r#"{base_defs}
        fn run() -> i32 {{
            var a = Cell {{ value: 1 }};
            var b = Cell {{ value: 2 }};
            var p = a.ref;
            let v = Sample<p>.empty;
            let f = factory(v);
            p = b.ref;
            f()
        }}
    "#
    );
    assert_build_rejected(
        &reassigned_binding,
        "reassigned runtime dependency binding",
        &["modified after capture"],
    );

    // 4. Writing through global constant pointer inside closure returned from generic factory.
    let write_global = format!(
        r#"{base_defs}
        trait Mutate {{
            fn bump(self);
        }}

        impl Mutate for Sample<src> where quote src: lift *Cell {{
            fn bump(self) {{
                src.*.value = 99;
            }}
        }}

        fn mut_factory(v: T) -> %{{ v: T }} fn()
        where
            T :- Mutate,
        {{
            || v.bump()
        }}

        fn run() -> i32 {{
            let v = Sample<Cell.GLOBAL.ref>.empty;
            let f = mut_factory(v);
            f();
            0
        }}
    "#
    );
    assert_build_rejected(
        &write_global,
        "write through global constant",
        &["constant"],
    );

    // 5. Returning borrowed local pointer out of a function via generic closure factory.
    let escaping_borrow = format!(
        r#"{base_defs}
        trait Borrow {{
            fn borrow(self) -> *i32;
        }}

        impl Borrow for Sample<src> where quote src: lift *Cell {{
            fn borrow(self) -> *i32 {{
                src.value
            }}
        }}

        fn borrow_factory(v: T) -> %{{ v: T }} fn() -> *i32
        where
            T :- Borrow,
        {{
            || v.borrow()
        }}

        fn leak() -> *i32 {{
            var cell = Cell {{ value: 7 }};
            let v = Sample<cell.ref>.empty;
            let f = borrow_factory(v);
            f()
        }}

        fn run() -> i32 {{
            leak().*
        }}
    "#
    );
    assert_build_rejected(
        &escaping_borrow,
        "escaping borrowed local pointer",
        &["escape", "local memory", "escaping provenance"],
    );

    // 6. Explicit pointer capture still rejected as escaping provenance.
    let explicit_pointer_capture = r#"
        fn run() -> i32 {
            let f = do {
                var cell: i32 = 7;
                let p = cell.ref;
                || p.*
            };
            f()
        }
    "#;
    assert_build_rejected(
        explicit_pointer_capture,
        "explicit pointer capture escaping provenance",
        &["escaping provenance"],
    );

    // 7. Closure referencing `quote src` NOT present in its type cannot be capture-free.
    let missing_env_in_type = format!(
        r#"{base_defs}
        fn bad() -> fn() -> i32
        where
            quote src: lift *Cell,
        {{
            || src.value.*
        }}

        fn run() -> i32 {{
            var cell = Cell {{ value: 7 }};
            let f = bad<cell.ref>();
            f()
        }}
    "#
    );
    assert_build_rejected(
        &missing_env_in_type,
        "quote src absent from closure type",
        &["type", "mismatch", "not entailed", "capture", "cannot"],
    );
}
