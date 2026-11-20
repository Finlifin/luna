//! End-to-end native execution tests for `builtin.fmt.Debug` (`std.fmt.format_debug`
//! and std container/error `Debug` impls) and `builtin.Iterator` combinators
//! (`enumerate`, `step_by`, `count`, `position`, `min_by`, `max_by`, `min`, `max`).
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-builtin-di-{}-{}",
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

#[test]
fn builtin_debug_formats_primitives_strings_vecs_boxes_and_errors_without_consuming() {
    let source = r#"
        use std.prelude.*;
        use std.io.IoErr;
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, ReallocErr, SystemAllocator};
        use std.string.Utf8Err;

        fn check_debug(
            fba: *FixedBufferAllocator,
            expected: str,
            value: *T,
        ) -> !AllocErr bool
        where
            T :- Debug,
        {
            let formatted = format_debug<.A = FixedBufferAllocator, .allocator = fba>(value)!;
            formatted.ref.equals_str(expected)
        }

        fn check_string_debug(
            expected: str,
            value: *T,
        ) -> !AllocErr bool
        where
            T :- Debug,
        {
            let formatted = format_debug(value)!;
            formatted.ref.equals_str(expected)
        }

        fn exercise_debug(fba: *FixedBufferAllocator) -> !AllocErr bool {
            let b_true = true;
            let unit_val = ();
            let neg_i32: i32 = -42;
            let big_u16: u16 = 65535;
            let prim_ok = check_debug(fba, "true", b_true.ref)!
                and check_debug(fba, "()", unit_val.ref)!
                and check_debug(fba, "-42", neg_i32.ref)!
                and check_debug(fba, "65535", big_u16.ref)!;
            if not prim_ok {
                return false;
            }

            let ch_quote = '\'';
            let ch_nl = '\n';
            let ch_ctrl = char.from_u32(7) match {
                c? => c,
                null => return false,
            };
            let ch_han = '中';
            let char_ok = check_debug(fba, "'\\''", ch_quote.ref)!
                and check_debug(fba, "'\\n'", ch_nl.ref)!
                and check_debug(fba, "'\\x07'", ch_ctrl.ref)!
                and check_debug(fba, "'中'", ch_han.ref)!;
            if not char_ok {
                return false;
            }

            let raw_text = "a\t\"\\中\0";
            let str_ok = check_debug(fba, "\"a\\t\\\"\\\\中\\0\"", raw_text.ref)!;
            if not str_ok {
                return false;
            }

            -- Non-Copy String, Vec<String>, and Box<String> must remain intact after format_debug.
            var owned_str = String<.A = FixedBufferAllocator, .allocator = fba>.from_str("hi\n\"!")!;
            let owned_ok = check_debug(fba, "\"hi\\n\\\"!\"", owned_str.ref)!;
            if not owned_ok or not owned_str.ref.equals_str("hi\n\"!") {
                return false;
            }

            var words = Vec<
                String<.A = FixedBufferAllocator, .allocator = fba>,
                .A = FixedBufferAllocator,
                .allocator = fba,
            >.new();
            words.ref.push(String<.A = FixedBufferAllocator, .allocator = fba>.from_str("a")!)!;
            words.ref.push(String<.A = FixedBufferAllocator, .allocator = fba>.from_str("b\t")!)!;
            let vec_ok = check_debug(fba, "[\"a\", \"b\\t\"]", words.ref)!;
            if not vec_ok or words.ref.len() != 2 {
                return false;
            }

            var boxed = Box<
                String<.A = FixedBufferAllocator, .allocator = fba>,
                .A = FixedBufferAllocator,
                .allocator = fba,
            >.new(String<.A = FixedBufferAllocator, .allocator = fba>.from_str("bx\r")!)!;
            let box_dbg_ok = check_debug(fba, "\"bx\\r\"", boxed.ref)!;
            if not box_dbg_ok {
                return false;
            }
            let inner_ok = boxed.ref.with_ref(|inner| inner.equals_str("bx\r"));
            if not inner_ok {
                return false;
            }

            let io_os = IoErr.Os(2);
            let io_wz = IoErr.WriteZero;
            let io_eof = IoErr.UnexpectedEof;
            let alloc_oom = AllocErr.OutOfMemory;
            let realloc_bad = ReallocErr.InvalidLayout;
            let lay = Layout.new(64, 16) match {
                v? => v,
                null => return false,
            };
            let err_ok = check_string_debug("IoErr.Os(2)", io_os.ref)!
                and check_string_debug("IoErr.WriteZero", io_wz.ref)!
                and check_string_debug("IoErr.UnexpectedEof", io_eof.ref)!
                and check_string_debug("AllocErr.OutOfMemory", alloc_oom.ref)!
                and check_string_debug("ReallocErr.InvalidLayout", realloc_bad.ref)!
                and check_string_debug("Layout { size: 64, align: 16 }", lay.ref)!;
            if not err_ok {
                return false;
            }

            var invalid_bytes = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            invalid_bytes.ref.push(255)!;
            let utf8_ok = String<.A = FixedBufferAllocator, .allocator = fba>.from_utf8(invalid_bytes) match {
                _! => false,
                error err => check_debug(fba, "Utf8Err { valid_up_to: 0 }", err.ref)!,
            };
            utf8_ok
        }

        fn run() -> bool {
            let layout = Layout.new(32768, 16) match {
                found? => found,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let ok = exercise_debug(fba.ref) match {
                passed! => passed,
                error _ => false,
            };
            let clean = fba.ref.live_allocations() == 0;
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            ok and clean
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn iterator_enumerate_step_by_count_position_and_min_max_support_once_items() {
    let source = r#"
        use std.prelude.*;
        use builtin.math.Order;
        use std.mem.{
            AllocErr, Allocator, DefaultAllocator, FixedBufferAllocator, Layout, SystemAllocator,
        };

        struct TaggedBox
        where
            .A: Type = DefaultAllocator,
            requires A :- Allocator,
            quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
        {
            tag: u64,
            payload: Box<u64, .A = A, .allocator = allocator>,
        }

        fn make_tagged(
            tag: u64,
            value: u64,
        ) -> !AllocErr TaggedBox<.A = FixedBufferAllocator, .allocator = fba>
        where
            quote fba: lift *FixedBufferAllocator,
        {
            let boxed = Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(value)!;
            TaggedBox<.A = FixedBufferAllocator, .allocator = fba> {
                tag: tag,
                payload: boxed,
            }
        }

        fn compare_tagged(
            left: *TaggedBox<.A = FixedBufferAllocator, .allocator = fba>,
            right: *TaggedBox<.A = FixedBufferAllocator, .allocator = fba>,
        ) -> Order
        where
            quote fba: lift *FixedBufferAllocator,
        {
            let lv = left.payload.get();
            let rv = right.payload.get();
            if lv < rv {
                Order.less
            } else if lv == rv {
                Order.equal
            } else {
                Order.greater
            }
        }

        fn build_nums() -> !AllocErr Vec<u64, .A = FixedBufferAllocator, .allocator = fba>
        where
            quote fba: lift *FixedBufferAllocator,
        {
            var v = Vec<u64, .A = FixedBufferAllocator, .allocator = fba>.with_capacity(6)!;
            v.ref.push(10)!;
            v.ref.push(20)!;
            v.ref.push(30)!;
            v.ref.push(40)!;
            v.ref.push(50)!;
            v
        }

        fn build_tagged() -> !AllocErr Vec<TaggedBox<.A = FixedBufferAllocator, .allocator = fba>, .A = FixedBufferAllocator, .allocator = fba>
        where
            quote fba: lift *FixedBufferAllocator,
        {
            var v = Vec<TaggedBox<.A = FixedBufferAllocator, .allocator = fba>, .A = FixedBufferAllocator, .allocator = fba>.with_capacity(5)!;
            v.ref.push(make_tagged<fba>(1, 20)!)!;
            v.ref.push(make_tagged<fba>(2, 5)!)!;
            v.ref.push(make_tagged<fba>(3, 50)!)!;
            v.ref.push(make_tagged<fba>(4, 5)!)!;
            v.ref.push(make_tagged<fba>(5, 50)!)!;
            v
        }

        fn exercise_iterators(fba: *FixedBufferAllocator) -> !AllocErr bool {
            -- 1. step_by (0, 1, 2) and count on Copy items.
            let c0 = build_nums<fba>()!.as(IntoIterator).into_iter().as(Iterator).step_by(0).as(Iterator).count();
            let c1 = build_nums<fba>()!.as(IntoIterator).into_iter().as(Iterator).step_by(1).as(Iterator).count();
            if c0 != 0 or c1 != 5 {
                return false;
            }

            var stepped_sum: u64 = 0;
            let stepped_sum_ptr = stepped_sum.ref;
            var stepped_count: usize = 0;
            let stepped_count_ptr = stepped_count.ref;
            build_nums<fba>()!
                .as(IntoIterator)
                .into_iter()
                .as(Iterator)
                .step_by(2)
                .as(Iterator)
                .enumerate()
                .as(Iterator)
                .foreach(|pair| {
                    let (idx, val) = pair;
                    let idx64 = builtin.compiler.try_usize_to_u64(idx) match {
                        v? => v,
                        null => return,
                    };
                    if idx64 < 10 and val < 1000 and stepped_sum_ptr.* < 100000 {
                        stepped_sum_ptr.* = stepped_sum_ptr.* + val + idx64;
                    }
                    if stepped_count_ptr.* < 100 {
                        stepped_count_ptr.* = stepped_count_ptr.* + 1;
                    }
                });
            -- step_by(2) emits (0, 10), (1, 30), (2, 50) => sum = 10 + 0 + 30 + 1 + 50 + 2 = 93.
            if stepped_count != 3 or stepped_sum != 93 {
                return false;
            }

            -- 2. min / max on Copy + Ord items, plus empty iterator.
            let min_val = build_nums<fba>()!.as(IntoIterator).into_iter().as(Iterator).min() match {
                v? => v,
                null => return false,
            };
            let max_val = build_nums<fba>()!.as(IntoIterator).into_iter().as(Iterator).max() match {
                v? => v,
                null => return false,
            };
            let empty = Vec<u64, .A = FixedBufferAllocator, .allocator = fba>.new();
            let empty_min = empty.as(IntoIterator).into_iter().as(Iterator).min();
            if min_val != 10 or max_val != 50 or empty_min.is_some() {
                return false;
            }

            -- 3. position with early exit and affine Once items; losers must all Drop cleanly.
            let pos_found = build_tagged<fba>()!
                .as(IntoIterator)
                .into_iter()
                .as(Iterator)
                .position(|item| item.payload.get() == 50) match {
                    idx? => idx,
                    null => return false,
                };
            let pos_missing = build_tagged<fba>()!
                .as(IntoIterator)
                .into_iter()
                .as(Iterator)
                .position(|item| item.payload.get() == 999);
            if pos_found != 2 or pos_missing.is_some() {
                return false;
            }

            -- 4. enumerate + step_by over affine Once TaggedBox stream.
            var affine_check: u64 = 0;
            let affine_ptr = affine_check.ref;
            build_tagged<fba>()!
                .as(IntoIterator)
                .into_iter()
                .as(Iterator)
                .step_by(2)
                .as(Iterator)
                .enumerate()
                .as(Iterator)
                .foreach(|pair| {
                    let (idx, entry) = pair;
                    let idx64 = builtin.compiler.try_usize_to_u64(idx) match {
                        v? => v,
                        null => return,
                    };
                    let val = entry.payload.get();
                    if idx64 < 10 and val < 1000 and entry.tag < 100 and affine_ptr.* < 100000 {
                        affine_ptr.* = affine_ptr.* + entry.tag * 100 + val + idx64;
                    }
                });
            -- Selected tags 1(20)@0, 3(50)@1, 5(50)@2 => (120 + 0) + (350 + 1) + (550 + 2) = 1023.
            if affine_check != 1023 {
                return false;
            }

            -- 5. min_by keeps first tie (tag 2, value 5); max_by keeps last tie (tag 5, value 50).
            let min_entry = build_tagged<fba>()!
                .as(IntoIterator)
                .into_iter()
                .as(Iterator)
                .min_by(|a, b| compare_tagged(a, b)) match {
                    v? => v,
                    null => return false,
                };
            let max_entry = build_tagged<fba>()!
                .as(IntoIterator)
                .into_iter()
                .as(Iterator)
                .max_by(|a, b| compare_tagged(a, b)) match {
                    v? => v,
                    null => return false,
                };
            min_entry.tag == 2
                and min_entry.payload.get() == 5
                and max_entry.tag == 5
                and max_entry.payload.get() == 50
        }

        fn run() -> bool {
            let layout = Layout.new(65536, 16) match {
                found? => found,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let ok = exercise_iterators(fba.ref) match {
                passed! => passed,
                error _ => false,
            };
            let clean = fba.ref.live_allocations() == 0;
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            ok and clean
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}
