//! End-to-end Milestone D tests for `u32 -> ?char` conversions, `std.collections.Vec<T>`,
//! `std.string.{String, Utf8Err}`, and generic `std.fmt.{write_fmt, format, format_into}`.
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
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-coll-{}-{}",
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
fn char_from_u32_and_try_from_work_at_runtime_and_comptime() {
    let source = r#"
        use std.prelude.*;
        use builtin.{TryFrom, TryInto};
        use meta.{ComptimeTryFrom, ComptimeTryInto};

        comptime fn check_comptime() -> bool {
            let a_ok = char.as(ComptimeTryFrom<u32>).try_from(65) match {
                ch? => ch == 'A',
                null => false,
            };
            let han_code: u32 = 20013;
            let han_ok = han_code.as(ComptimeTryInto<char>).try_into() match {
                ch? => ch == '中',
                null => false,
            };
            let max_code: u32 = 1114111;
            let max_ok = max_code.as(ComptimeTryInto<char>).try_into() match {
                ch? => ch == '\x{10ffff}',
                null => false,
            };
            let sur_low: u32 = 55296;
            let sur_high: u32 = 57343;
            let over: u32 = 1114112;
            let sur_low_none = char.as(ComptimeTryFrom<u32>).try_from(sur_low).is_none();
            let sur_high_none = sur_high.as(ComptimeTryInto<char>).try_into().is_none();
            let over_none = over.as(ComptimeTryInto<char>).try_into().is_none();
            a_ok and han_ok and max_ok and sur_low_none and sur_high_none and over_none
        }

        const COMPTIME_CHAR_OK: bool = check_comptime();

        fn check_runtime() -> bool {
            let a_ok = char.from_u32(65) match {
                ch? => ch == 'A',
                null => false,
            };
            let han_code: u32 = 20013;
            let han_ok = char.as(TryFrom<u32>).try_from(han_code) match {
                ch? => ch == '中',
                null => false,
            };
            let max_code: u32 = 1114111;
            let max_ok = max_code.as(TryInto<char>).try_into() match {
                ch? => ch == '\x{10ffff}',
                null => false,
            };
            let sur_low: u32 = 55296;
            let sur_high: u32 = 57343;
            let over: u32 = 1114112;
            let sur_low_none = char.from_u32(sur_low).is_none();
            let sur_high_none = char.as(TryFrom<u32>).try_from(sur_high).is_none();
            let over_none = over.as(TryInto<char>).try_into().is_none();
            COMPTIME_CHAR_OK and a_ok and han_ok and max_ok
                and sur_low_none and sur_high_none and over_none
        }

        fn run() -> bool {
            check_runtime()
        }
    "#;

    assert_eq!(support::run(source), Value::Bool(true));

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn vec_operations_growth_mutation_and_clone_work_natively() {
    let source = r#"
        use std.prelude.*;
        use std.mem.AllocErr;

        fn exercise_vec() -> !AllocErr bool {
            var empty = Vec<u32>.new();
            if not empty.ref.is_empty() or empty.ref.len() != 0 or empty.ref.capacity() != 0 {
                return false;
            }
            if not empty.ref.pop().is_none() or not empty.ref.first().is_none()
                or not empty.ref.last().is_none() or not empty.ref.get(0).is_none()
            {
                return false;
            }

            -- Exercise with_capacity and geometric growth across multiple reallocations.
            var v = Vec<u32>.with_capacity(2)!;
            if v.ref.capacity() < 2 or not v.ref.is_empty() {
                return false;
            }
            var n: u32 = 0;
            while n < 20 {
                v.ref.push(n * 10)!;
                n = n + 1;
            }
            if v.ref.len() != 20 or v.ref.capacity() < 20 {
                return false;
            }
            let first_ok = v.ref.first() match { x? => x == 0, null => false };
            let mid_ok = v.ref.get(7) match { x? => x == 70, null => false };
            let last_ok = v.ref.last() match { x? => x == 190, null => false };
            let oob_none = v.ref.get(20).is_none();
            if not (first_ok and mid_ok and last_ok and oob_none) {
                return false;
            }

            -- Pop back to [0, 10, 20, 30].
            while v.ref.len() > 4 {
                let _popped = v.ref.pop();
            }
            let popped = v.ref.pop() match { x? => x == 30, null => false };
            if not popped or v.ref.len() != 3 {
                return false;
            }

            -- Insert front, middle, end, and reject out-of-bounds.
            let ins_front = v.ref.insert(0, 99)!;
            let ins_mid = v.ref.insert(2, 55)!;
            let ins_end = v.ref.insert(5, 77)!;
            let ins_oob = v.ref.insert(7, 88)!;
            -- Now [99, 0, 55, 10, 20, 77].
            if not (ins_front and ins_mid and ins_end) or ins_oob or v.ref.len() != 6 {
                return false;
            }

            -- remove and swap_remove
            let rem_mid = v.ref.remove(2) match { x? => x == 55, null => false };
            let rem_oob = v.ref.remove(10).is_none();
            -- Now [99, 0, 10, 20, 77].
            let swrem = v.ref.swap_remove(1) match { x? => x == 0, null => false };
            let swrem_oob = v.ref.swap_remove(4).is_none();
            -- Now [99, 77, 10, 20].
            if not (rem_mid and rem_oob and swrem and swrem_oob) or v.ref.len() != 4 {
                return false;
            }

            -- swap, replace, set
            let sw_ok = v.ref.swap(0, 3) and not v.ref.swap(0, 9);
            -- Now [20, 77, 10, 99].
            let rep_ok = v.ref.replace(1, 700) match { x? => x == 77, null => false };
            let rep_oob = v.ref.replace(4, 1).is_none();
            let set_ok = v.ref.set(2, 100) and not v.ref.set(4, 1);
            -- Now [20, 700, 100, 99].
            if not (sw_ok and rep_ok and rep_oob and set_ok) {
                return false;
            }

            -- contains, with_ref, for_each_ref, try_for_each_ref
            if not v.ref.contains(700) or v.ref.contains(77) {
                return false;
            }
            let ref_read = v.ref.with_ref(3, |item| item.*) match { x? => x == 99, null => false };
            let ref_oob = v.ref.with_ref(9, |item| item.*).is_none();
            if not (ref_read and ref_oob) {
                return false;
            }
            v.ref.for_each_ref(|item| {
                if item.* < 1000000 {
                    item.* = item.* + 1;
                }
            });
            -- Now [21, 701, 101, 100].
            let try_ok = v.ref.try_for_each_ref(|item| -> !AllocErr void {
                if item.* == 0 {
                    return error AllocErr.InvalidLayout;
                }
            }) match {
                _! => true,
                error _ => false,
            };
            let try_err = v.ref.try_for_each_ref(|item| -> !AllocErr void {
                if item.* == 101 {
                    return error AllocErr.InvalidLayout;
                }
            }) match {
                _! => false,
                error _ => true,
            };
            if not (try_ok and try_err) {
                return false;
            }

            -- reverse, append, shrink_to_fit, try_clone, Eq, IntoIterator
            v.ref.reverse();
            -- Now [100, 101, 701, 21].
            var extra = Vec<u32>.new();
            extra.ref.push(5)!;
            extra.ref.push(6)!;
            v.ref.append(extra.ref)!;
            if not extra.ref.is_empty() or v.ref.len() != 6 {
                return false;
            }
            v.ref.shrink_to_fit()!;
            if v.ref.capacity() != 6 {
                return false;
            }

            var cloned = v.ref.try_clone()!;
            cloned.ref.push(999)!;
            if cloned.ref.len() != 7 or v.ref.len() != 6 {
                return false;
            }
            var clone_eq = v.ref.try_clone()!;
            v.ref.truncate(4);
            clone_eq.ref.truncate(4);
            let eq_same = v.ref.as(Eq).eq(clone_eq.ref);
            var diff = v.ref.try_clone()!;
            diff.ref.set(0, 888);
            let ne_diff = not v.ref.as(Eq).eq(diff.ref);

            var sum: u32 = 0;
            let sum_ptr = sum.ref;
            v.as(IntoIterator).into_iter().as(Iterator).foreach(|item| {
                if item < 100000 and sum_ptr.* < 1000000 {
                    sum_ptr.* = sum_ptr.* + item;
                }
            });
            -- [100, 101, 701, 21] => 923
            let iter_ok = sum == 923;

            cloned.ref.clear();
            cloned.ref.shrink_to_fit()!;
            let clear_shrink_ok = cloned.ref.is_empty() and cloned.ref.capacity() == 0;

            eq_same and ne_diff and iter_ok and clear_shrink_ok
        }

        fn run() -> bool {
            exercise_vec() match {
                ok! => ok,
                error _ => false,
            }
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn vec_affine_zst_nested_and_iterator_drops_fire_exactly_once() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{AllocErr, Allocator, Layout, SystemAllocator};

        struct TrackDrop {
            id: u64,
            counter_addr: usize,
        }

        impl Drop for TrackDrop {
            fn drop(*self) {
                unsafe {
                    let count = builtin.mem.raw_read<u64>(self.counter_addr.*);
                    assumes count < 18446744073709551615;
                    builtin.mem.raw_write<u64>(self.counter_addr.*, count + 1);
                }
            }
        }

        struct ZeroDrop {}

        impl Drop for ZeroDrop {
            fn drop(*self) {
                let _ = std.fmt.print("Z") match {
                    _! => true,
                    error _ => false,
                };
            }
        }

        fn read_count(addr: usize) -> u64 {
            unsafe { builtin.mem.raw_read<u64>(addr) }
        }

        fn reset_count(addr: usize) {
            unsafe {
                builtin.mem.raw_write<u64>(addr, 0);
            }
        }

        fn exercise_drops() -> bool {
            let alloc = SystemAllocator {};
            let counter_layout = Layout.of<u64>() match {
                v? => v,
                null => return false,
            };
            let counter_block = alloc.as(Allocator).allocate_zeroed(counter_layout) match {
                b! => b,
                error _ => return false,
            };
            let addr = counter_block.address();

            -- 1. Scope exit drops every remaining TrackDrop once across growth.
            reset_count(addr);
            {
                var v = Vec<TrackDrop>.new();
                var k: u64 = 0;
                while k < 10 {
                    let ok = v.ref.push(TrackDrop { id: k, counter_addr: addr }) match {
                        _! => true,
                        error _ => false,
                    };
                    if not ok {
                        unsafe { alloc.as(Allocator).deallocate(counter_block); }
                        return false;
                    }
                    k = k + 1;
                }
                if read_count(addr) != 0 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
            }
            if read_count(addr) != 10 {
                unsafe { alloc.as(Allocator).deallocate(counter_block); }
                return false;
            }

            -- 2. Mutators: replace returns old (caller drops it), set drops old immediately,
            --    remove/swap_remove return removed (caller drops it), truncate & clear drop tails,
            --    and out-of-bounds insert/replace/set retire rejected values once.
            reset_count(addr);
            {
                var v = Vec<TrackDrop>.new();
                var k: u64 = 0;
                while k < 6 {
                    let _ = v.ref.push(TrackDrop { id: k, counter_addr: addr }) match {
                        _! => true,
                        error _ => false,
                    };
                    k = k + 1;
                }
                {
                    let old = v.ref.replace(1, TrackDrop { id: 100, counter_addr: addr });
                    -- replaced item held in `old`, not dropped yet
                    if read_count(addr) != 0 {
                        unsafe { alloc.as(Allocator).deallocate(counter_block); }
                        return false;
                    }
                }
                -- `old` dropped (+1 => 1)
                if read_count(addr) != 1 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
                let set_ok = v.ref.set(2, TrackDrop { id: 200, counter_addr: addr });
                -- `set` dropped old index 2 immediately (+1 => 2)
                if not set_ok or read_count(addr) != 2 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
                {
                    let _rem1 = v.ref.remove(0);
                    let _rem2 = v.ref.swap_remove(0);
                }
                -- removed items dropped at inner block exit (+2 => 4); 4 elements left
                if read_count(addr) != 4 or v.ref.len() != 4 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
                v.ref.truncate(2);
                -- 2 tail elements dropped (+2 => 6)
                if read_count(addr) != 6 or v.ref.len() != 2 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
                -- Out-of-bounds insert/replace/set retire their unused value (+3 => 9)
                let bad_ins = v.ref.insert(9, TrackDrop { id: 901, counter_addr: addr }) match {
                    res! => res,
                    error _ => true,
                };
                let bad_rep = v.ref.replace(9, TrackDrop { id: 902, counter_addr: addr }).is_none();
                let bad_set = v.ref.set(9, TrackDrop { id: 903, counter_addr: addr });
                if bad_ins or not bad_rep or bad_set or read_count(addr) != 9 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
                v.ref.clear();
                -- Remaining 2 elements dropped (+2 => 11)
                if read_count(addr) != 11 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
            }
            if read_count(addr) != 11 {
                unsafe { alloc.as(Allocator).deallocate(counter_block); }
                return false;
            }

            -- 3. Full VecIter consumption drops every yielded element once.
            reset_count(addr);
            {
                var v = Vec<TrackDrop>.new();
                var k: u64 = 0;
                while k < 4 {
                    let _ = v.ref.push(TrackDrop { id: k, counter_addr: addr }) match {
                        _! => true,
                        error _ => false,
                    };
                    k = k + 1;
                }
                var seen_sum: u64 = 0;
                let seen_ptr = seen_sum.ref;
                v.as(IntoIterator).into_iter().as(Iterator).foreach(|item| {
                    if seen_ptr.* < 100000 and item.id < 1000 {
                        seen_ptr.* = seen_ptr.* + item.id;
                    }
                });
                if seen_sum != 6 or read_count(addr) != 4 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
            }

            -- 4. Early break / short-circuit via Iterator.find drops yielded items
            --    AND remaining unyielded elements in VecIter.Drop without double-drops.
            reset_count(addr);
            {
                var v = Vec<TrackDrop>.new();
                var k: u64 = 0;
                while k < 5 {
                    let _ = v.ref.push(TrackDrop { id: k + 10, counter_addr: addr }) match {
                        _! => true,
                        error _ => false,
                    };
                    k = k + 1;
                }
                let found_ok = v.as(IntoIterator).into_iter().as(Iterator).find(|item| item.id.* == 12) match {
                    elem? => elem.id == 12,
                    null => false,
                };
                if not found_ok or read_count(addr) != 5 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
            }

            -- 5. Nested Vec<Vec<TrackDrop>> drops all inner elements and outer/inner buffers once.
            reset_count(addr);
            {
                var outer = Vec<Vec<TrackDrop>>.new();
                var a = Vec<TrackDrop>.new();
                let _ = a.ref.push(TrackDrop { id: 1, counter_addr: addr });
                let _ = a.ref.push(TrackDrop { id: 2, counter_addr: addr });
                var b = Vec<TrackDrop>.new();
                let _ = b.ref.push(TrackDrop { id: 3, counter_addr: addr });
                let _ = outer.ref.push(a);
                let _ = outer.ref.push(b);
                if read_count(addr) != 0 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
            }
            if read_count(addr) != 3 {
                unsafe { alloc.as(Allocator).deallocate(counter_block); }
                return false;
            }

            -- 6. Zero-sized affine elements push, pop, truncate, and drop on scope exit (4 "Z"s total).
            var zv = Vec<ZeroDrop>.new();
            let _ = zv.ref.push(ZeroDrop {});
            let _ = zv.ref.push(ZeroDrop {});
            let _ = zv.ref.push(ZeroDrop {});
            let _ = zv.ref.push(ZeroDrop {});
            if zv.ref.len() != 4 {
                unsafe { alloc.as(Allocator).deallocate(counter_block); }
                return false;
            }
            {
                let _popped_z = zv.ref.pop();
            }
            -- 1 Z dropped from pop; 3 remain.
            zv.ref.truncate(1);
            -- 2 Z dropped from truncate; 1 remains and drops on return.

            unsafe {
                alloc.as(Allocator).deallocate(counter_block);
            }
            true
        }

        fn run() -> bool {
            exercise_drops()
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"ZZZZ(bool true)\n");
}

#[test]
fn string_utf8_validation_mutation_search_and_protocols_work_natively() {
    let source = r#"
        use std.prelude.*;
        use std.mem.AllocErr;

        fn push_all(target: *Vec<u8>, byte_count: u8, b0: u8, b1: u8, b2: u8, b3: u8) -> !AllocErr void {
            if byte_count >= 1 {
                target.push(b0)!;
            }
            if byte_count >= 2 {
                target.push(b1)!;
            }
            if byte_count >= 3 {
                target.push(b2)!;
            }
            if byte_count >= 4 {
                target.push(b3)!;
            }
        }

        fn check_err(bytes: Vec<u8>, expected_offset: usize) -> bool {
            String.from_utf8(bytes) match {
                _! => false,
                error err => err.valid_up_to() == expected_offset,
            }
        }

        fn exercise_string() -> !AllocErr bool {
            let empty = String.new();
            let cap_init = String.with_capacity(16)!;
            if empty.ref.len() != 0 or not empty.ref.is_empty() or cap_init.ref.capacity() < 16 {
                return false;
            }

            -- Valid 1/2/3/4-byte UTF-8: "Aé中😀" = [65, 195,169, 228,184,173, 240,159,152,128] (10 bytes).
            var raw = Vec<u8>.new();
            raw.ref.push(65)!;
            push_all(raw.ref, 2, 195, 169, 0, 0)!;
            push_all(raw.ref, 3, 228, 184, 173, 0)!;
            push_all(raw.ref, 4, 240, 159, 152, 128)!;
            var s = String.from_utf8(raw) match {
                ok! => ok,
                error _ => return false,
            };
            if s.ref.len() != 10 or not s.ref.is_valid_utf8() or not s.ref.equals_str("Aé中😀") {
                return false;
            }

            -- Character boundary checks: 0 ('A'), 1 ('é'), 3 ('中'), 6 ('😀'), 10 (end).
            let bounds_ok = s.ref.is_char_boundary(0)
                and s.ref.is_char_boundary(1)
                and not s.ref.is_char_boundary(2)
                and s.ref.is_char_boundary(3)
                and not s.ref.is_char_boundary(4)
                and not s.ref.is_char_boundary(5)
                and s.ref.is_char_boundary(6)
                and not s.ref.is_char_boundary(7)
                and not s.ref.is_char_boundary(8)
                and not s.ref.is_char_boundary(9)
                and s.ref.is_char_boundary(10)
                and not s.ref.is_char_boundary(11);
            if not bounds_ok {
                return false;
            }

            -- Invalid UTF-8 sequences must return exact valid_up_to() byte offset.
            var trunc2 = Vec<u8>.new();
            trunc2.ref.push(65)!;
            trunc2.ref.push(195)!;
            var trunc3 = Vec<u8>.new();
            trunc3.ref.push(65)!;
            trunc3.ref.push(228)!;
            trunc3.ref.push(184)!;
            var overlong2 = Vec<u8>.new();
            overlong2.ref.push(65)!;
            overlong2.ref.push(192)!;
            overlong2.ref.push(175)!;
            var overlong3 = Vec<u8>.new();
            overlong3.ref.push(224)!;
            overlong3.ref.push(128)!;
            overlong3.ref.push(175)!;
            var surrogate = Vec<u8>.new();
            surrogate.ref.push(65)!;
            surrogate.ref.push(237)!;
            surrogate.ref.push(160)!;
            surrogate.ref.push(128)!;
            var above_max = Vec<u8>.new();
            above_max.ref.push(244)!;
            above_max.ref.push(144)!;
            above_max.ref.push(128)!;
            above_max.ref.push(128)!;

            if not (check_err(trunc2, 1) and check_err(trunc3, 1) and check_err(overlong2, 1)
                and check_err(overlong3, 0) and check_err(surrogate, 1) and check_err(above_max, 0))
            {
                return false;
            }

            -- from_utf8_lossy: valid fast path and invalid replacement with U+FFFD (239, 191, 189).
            var lossy_valid_in = Vec<u8>.new();
            lossy_valid_in.ref.push(72)!;
            lossy_valid_in.ref.push(105)!;
            let lossy_valid = String.from_utf8_lossy(lossy_valid_in)!;
            if not lossy_valid.ref.equals_str("Hi") {
                return false;
            }
            var lossy_bad_in = Vec<u8>.new();
            lossy_bad_in.ref.push(65)!;
            lossy_bad_in.ref.push(255)!;
            lossy_bad_in.ref.push(66)!;
            let lossy_fixed = String.from_utf8_lossy(lossy_bad_in)!;
            let lossy_bytes = lossy_fixed.into_bytes();
            let b0 = lossy_bytes.ref.get(0) match { x? => x == 65, null => false };
            let b1 = lossy_bytes.ref.get(1) match { x? => x == 239, null => false };
            let b2 = lossy_bytes.ref.get(2) match { x? => x == 191, null => false };
            let b3 = lossy_bytes.ref.get(3) match { x? => x == 189, null => false };
            let b4 = lossy_bytes.ref.get(4) match { x? => x == 66, null => false };
            if lossy_bytes.ref.len() != 5 or not (b0 and b1 and b2 and b3 and b4) {
                return false;
            }

            -- push_char, pop_char, truncate (mid-char vs boundary), push_str, push_string, clear, reserve, shrink_to_fit.
            var muts = String.new();
            muts.ref.push_char('A')!;
            muts.ref.push_char('é')!;
            muts.ref.push_char('中')!;
            muts.ref.push_char('😀')!;
            if not muts.ref.equals(s.ref) {
                return false;
            }
            if muts.ref.truncate(7) or muts.ref.len() != 10 {
                return false;
            }
            if not muts.ref.truncate(6) or muts.ref.len() != 6 {
                return false;
            }
            muts.ref.push_char('😀')!;
            let p4 = muts.ref.pop_char() match { c? => c == '😀', null => false };
            let p3 = muts.ref.pop_char() match { c? => c == '中', null => false };
            let p2 = muts.ref.pop_char() match { c? => c == 'é', null => false };
            let p1 = muts.ref.pop_char() match { c? => c == 'A', null => false };
            let p0 = muts.ref.pop_char().is_none();
            if not (p4 and p3 and p2 and p1 and p0) {
                return false;
            }

            var extra = String.from_str("中😀")!;
            muts.ref.push_str("你好")!;
            muts.ref.push_string(extra.ref)!;
            if not muts.ref.equals_str("你好中😀") {
                return false;
            }
            muts.ref.reserve(32)!;
            let cur = muts.ref.len();
            let reserved_ok = cur <= 1024 and muts.ref.capacity() >= cur + 32;
            muts.ref.clear();
            muts.ref.shrink_to_fit()!;
            if not reserved_ok or not muts.ref.is_empty() or muts.ref.capacity() != 0 {
                return false;
            }

            -- starts_with, ends_with, find, contains
            let search_ok = s.ref.starts_with("")
                and s.ref.starts_with("Aé")
                and not s.ref.starts_with("é")
                and s.ref.ends_with("")
                and s.ref.ends_with("中😀")
                and not s.ref.ends_with("Aé中")
                and (s.ref.find("") match { i? => i == 0, null => false })
                and (s.ref.find("A") match { i? => i == 0, null => false })
                and (s.ref.find("中") match { i? => i == 3, null => false })
                and (s.ref.find("😀") match { i? => i == 6, null => false })
                and (s.ref.find("Aé中😀") match { i? => i == 0, null => false })
                and s.ref.find("nope").is_none()
                and s.ref.find("Aé中😀!").is_none()
                and s.ref.contains("é中")
                and not s.ref.contains("A中");
            if not search_ok {
                return false;
            }

            -- Eq, Ord, Hash (consistent with str.Hash)
            let s_clone = s.ref.try_clone()!;
            let s_less = String.from_str("Aé中")!;
            let s_greater = String.from_str("B")!;
            let eq_ok = s.ref.as(Eq).eq(s_clone.ref);
            let ord_lt = s.ref.as(Ord).cmp(s_greater.ref) match { .less => true, _ => false };
            let ord_gt = s.ref.as(Ord).cmp(s_less.ref) match { .greater => true, _ => false };
            let ord_eq = s.ref.as(Ord).cmp(s.ref) match { .equal => true, _ => false };
            let abc = String.from_str("abc")!;
            let hash_ok = s.as(Hash).hash() == "Aé中😀".as(Hash).hash()
                and abc.as(Hash).hash() == "abc".as(Hash).hash();

            eq_ok and ord_lt and ord_gt and ord_eq and hash_ok
        }

        fn run() -> bool {
            exercise_string() match {
                ok! => ok,
                error _ => false,
            }
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn std_fmt_format_and_generic_write_fmt_work_across_vec_string_writer_and_file() {
    let source = r#"
        use std.prelude.*;
        use builtin.fmt.ByteWriter;
        use std.fs.{File, remove_file};
        use std.io.Writer;
        use std.mem.AllocErr;

        fn probe_bool_display(buf: *Vec<u8>, comptime T: Type, v: T) -> !AllocErr bool {
            inline if T :- Display {
                v.ref.as(Display).fmt(buf)!;
                true
            } else {
                false
            }
        }

        fn write_generic(
            output: *W,
            label: str,
            ch: char,
        ) -> !W.as(ByteWriter).Err void
        where W :- ByteWriter
        {
            const prefix = "gen";
            write_fmt(output, "[{}:{}={}]", prefix, label, ch)!;
            var scratch = Vec<u8>.new();
            inline if bool :- Display {
                let flag = true;
                let _w = flag.ref.as(Display).fmt(scratch.ref) match {
                    _! => true,
                    error _ => false,
                };
            }
            let _ok = probe_bool_display(scratch.ref, bool, true) match {
                b! => b,
                error _ => false,
            };
        }

        fn exercise_fmt() -> bool {
            let owned = String.from_str("世界") match {
                s! => s,
                error _ => return false,
            };
            let neg: i32 = -42;

            -- 1. std.fmt.format with UTF-8 str, char, String, bool, signed integer, escaped braces.
            let formatted = format(
                "你好 {} {} {} {} {} {{}}",
                '中',
                '😀',
                owned.ref.try_clone() match { s! => s, error _ => return false },
                true,
                neg,
            ) match {
                s! => s,
                error _ => return false,
            };
            if not formatted.ref.equals_str("你好 中 😀 世界 true -42 {}") {
                return false;
            }

            -- 2. std.fmt.format_into appending to existing String.
            var appended = String.from_str("prefix:") match {
                s! => s,
                error _ => return false,
            };
            let fi_ok = format_into(appended.ref, "{}={}", 'Ω', neg) match {
                _! => true,
                error _ => false,
            };
            if not fi_ok or not appended.ref.equals_str("prefix:Ω=-42") {
                return false;
            }

            -- 3. std.fmt.write_fmt targeting *Vec<u8> across multiple calls + caller local const + generic helper.
            var buf = Vec<u8>.new();
            const first_tag = "one";
            let w1 = write_fmt(buf.ref, "{}-", first_tag) match {
                _! => true,
                error _ => false,
            };
            let w2 = write_generic(buf.ref, "k", 'λ') match {
                _! => true,
                error _ => false,
            };
            let via_vec = String.from_utf8(buf) match {
                s! => s,
                error _ => return false,
            };
            if not (w1 and w2) or not via_vec.ref.equals_str("one-[gen:k=λ]") {
                return false;
            }

            -- 4. Writer.write_bytes, Writer.write_string, and write_fmt targeting *Writer (stdout) & File round-trip.
            let file_ok = do {
                var f = File.create<"fmt_roundtrip.txt">() match {
                    opened! => opened,
                    error _ => return false,
                };
                let ok1 = write_fmt(f.ref, "A={}", neg) match {
                    _! => true,
                    error _ => false,
                };
                var tail = String.from_str("!中") match {
                    s! => s,
                    error _ => return false,
                };
                let ok2 = f.ref.write_string(tail.ref) match {
                    _! => true,
                    error _ => false,
                };
                var raw_bytes = Vec<u8>.new();
                let _ = raw_bytes.ref.push(35);
                let ok3 = f.ref.write_bytes(raw_bytes.ref) match {
                    _! => true,
                    error _ => false,
                };
                let ok_close = f.ref.close() match {
                    _! => true,
                    error _ => false,
                };
                if not (ok1 and ok2 and ok3 and ok_close) {
                    let _ = remove_file("fmt_roundtrip.txt");
                    return false;
                }
                var rf = File.open<"fmt_roundtrip.txt">() match {
                    opened! => opened,
                    error _ => {
                        let _ = remove_file("fmt_roundtrip.txt");
                        return false;
                    },
                };
                var read_bytes = Vec<u8>.new();
                while true {
                    let next = rf.ref.read_byte() match {
                        opt! => opt,
                        error _ => {
                            let _ = remove_file("fmt_roundtrip.txt");
                            return false;
                        },
                    };
                    next match {
                        b? => {
                            let _ = read_bytes.ref.push(b);
                        },
                        null => break,
                    };
                }
                let _ = rf.ref.close();
                let _ = remove_file("fmt_roundtrip.txt");
                let read_str = String.from_utf8(read_bytes) match {
                    s! => s,
                    error _ => return false,
                };
                read_str.ref.equals_str("A=-42!中#")
            };
            if not file_ok {
                return false;
            }

            -- 5. Stdout via Writer.write_string, Writer.write_bytes, write_fmt, and println with String.
            var out = std.io.stdout();
            var banner = String.from_str("W:") match {
                s! => s,
                error _ => return false,
            };
            let s1 = out.write_string(banner.ref) match {
                _! => true,
                error _ => false,
            };
            var bytes_msg = Vec<u8>.new();
            let _ = write_fmt(bytes_msg.ref, "hello {} {}", "世界", owned);
            let s2 = out.write_bytes(bytes_msg.ref) match {
                _! => true,
                error _ => false,
            };
            let s3 = out.ref.write_byte(10) match {
                _! => true,
                error _ => false,
            };
            let s4 = println("fmt={}", via_vec) match {
                _! => true,
                error _ => false,
            };
            s1 and s2 and s3 and s4
        }

        fn run() -> bool {
            exercise_fmt()
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(
        output.stdout,
        "W:hello 世界 世界\nfmt=one-[gen:k=λ]\n(bool true)\n".as_bytes()
    );
}

#[test]
fn forged_str_via_raw_read_intrinsic_and_dangling_pointer_fail_closed() {
    let forged_source = r#"
        fn forge() -> str {
            let zero: usize = 0;
            unsafe { builtin.mem.raw_read<str>(zero) }
        }

        fn run() -> bool {
            let s = forge();
            s == "a"
        }
    "#;
    let forged_case = NativeCase::new(forged_source);
    let forged_output = forged_case.build();
    assert!(
        !forged_output.status.success(),
        "raw_read<str> unexpectedly built:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&forged_output.stdout),
        String::from_utf8_lossy(&forged_output.stderr)
    );
    let stderr = String::from_utf8_lossy(&forged_output.stderr);
    assert!(
        stderr.contains("backing-region provenance"),
        "unexpected stderr: {stderr}"
    );

    let escaping_source = r#"
        fn leak() -> *str {
            let local = "ephemeral";
            local.ref
        }

        fn run() -> bool {
            let _ = leak();
            true
        }
    "#;
    let escaping_case = NativeCase::new(escaping_source);
    let escaping_output = escaping_case.build();
    assert!(
        !escaping_output.status.success(),
        "dangling *str return unexpectedly built:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&escaping_output.stdout),
        String::from_utf8_lossy(&escaping_output.stderr)
    );
}
