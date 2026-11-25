#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use comptime_interpretor::Value;

fn compile_native_and_run(source: &str) -> Output {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "luna-wishlist01-ranges-concat-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let source_path = root.join("main.fl");
    let output_path = root.join("program");
    fs::write(&source_path, source).unwrap();

    let library = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let build = Command::new(env!("CARGO_BIN_EXE_luna"))
        .args([
            "build".into(),
            source_path.into_os_string(),
            "--entry".into(),
            "run".into(),
            "--output".into(),
            output_path.clone().into_os_string(),
            "--opt-level".into(),
            "0".into(),
            "--fuel".into(),
            "10000000".into(),
            "--sysroot".into(),
            library.into_os_string(),
        ])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "native build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&output_path).output().unwrap();
    let _ = fs::remove_dir_all(&root);
    run
}

#[test]
fn six_range_types_bound_range_bounds_and_once_elements_work() {
    let source = r#"
        use meta.ComptimeRangeBounds;

        struct Token {
            id: u32,
        }

        pure comptime fn ct_check() -> bool {
            let r = 2i32..6i32;
            let ri = 2i32..=6i32;
            let rf = 2i32..;
            let rt = ..6i32;
            let rti = ..=6i32;
            let rall = ..;

            let bounds_ok = r.as(ComptimeRangeBounds<i32>).contains(2i32)
                and (not r.as(ComptimeRangeBounds<i32>).contains(6i32))
                and ri.as(ComptimeRangeBounds<i32>).contains(6i32)
                and rf.as(ComptimeRangeBounds<i32>).contains(100i32)
                and (not rf.as(ComptimeRangeBounds<i32>).contains(1i32))
                and rt.as(ComptimeRangeBounds<i32>).contains(5i32)
                and (not rt.as(ComptimeRangeBounds<i32>).contains(6i32))
                and rti.as(ComptimeRangeBounds<i32>).contains(6i32)
                and (not rti.as(ComptimeRangeBounds<i32>).contains(7i32))
                and rall.as(ComptimeRangeBounds<i32>).contains(-999i32);

            let sb_ok = r.as(ComptimeRangeBounds<i32>).start_bound() match {
                .included(2i32) => true,
                _ => false,
            };
            let eb_ok = r.as(ComptimeRangeBounds<i32>).end_bound() match {
                .excluded(6i32) => true,
                _ => false,
            };
            let full_sb = rall.as(ComptimeRangeBounds<i32>).start_bound() match {
                .unbounded => true,
                _ => false,
            };
            let (cs, ce, cex) = ri.deconstruct();
            let deconstruct_ok = cs == 2i32 and ce == 6i32 and (not cex);

            bounds_ok and sb_ok and eb_ok and full_sb and deconstruct_ok
        }

        const CT_CHECK: bool = ct_check();

        fn check_once_range() -> u32 {
            let r = Range<Token> { start: Token { id: 10 }, end: Token { id: 25 } };
            r.start.id + r.end.id
        }

        fn run() -> bool {
            let r = 2i32..6i32;
            let ri = 2i32..=6i32;
            let rf = 2i32..;
            let rt = ..6i32;
            let rti = ..=6i32;
            let rall = ..;

            let two: i32 = 2;
            let five: i32 = 5;
            let six: i32 = 6;
            let seven: i32 = 7;
            let one: i32 = 1;
            let hundred: i32 = 100;
            let neg: i32 = -42;

            let empty_r1 = 6i32..2i32;
            let empty_r2 = 4i32..4i32;
            let empty_ri = 6i32..=2i32;

            let contains_ok = r.contains(two.ref)
                and (not r.contains(six.ref))
                and (not r.is_empty())
                and empty_r1.is_empty()
                and empty_r2.is_empty()
                and ri.contains(six.ref)
                and (not ri.is_empty())
                and empty_ri.is_empty();

            let trait_ok = r.as(RangeBounds<i32>).contains(five.ref)
                and ri.as(RangeBounds<i32>).contains(six.ref)
                and rf.as(RangeBounds<i32>).contains(hundred.ref)
                and (not rf.as(RangeBounds<i32>).contains(one.ref))
                and rt.as(RangeBounds<i32>).contains(five.ref)
                and (not rt.as(RangeBounds<i32>).contains(six.ref))
                and rti.as(RangeBounds<i32>).contains(six.ref)
                and (not rti.as(RangeBounds<i32>).contains(seven.ref))
                and rall.as(RangeBounds<i32>).contains(neg.ref)
                and rall.as(RangeBounds<i32>).contains(hundred.ref);

            let bound_ok = (ri.as(RangeBounds<i32>).start_bound() match {
                .included(p) => p.* == 2i32,
                _ => false,
            }) and (ri.as(RangeBounds<i32>).end_bound() match {
                .included(p) => p.* == 6i32,
                _ => false,
            }) and (r.as(RangeBounds<i32>).end_bound() match {
                .excluded(p) => p.* == 6i32,
                _ => false,
            }) and (rall.as(RangeBounds<i32>).start_bound() match {
                .unbounded => true,
                _ => false,
            });

            let start_end_ok = ri.start().* == 2i32 and ri.end().* == 6i32;
            let (s, e) = ri.into_inner();
            let into_inner_ok = start_end_ok and s == 2i32 and e == 6i32;

            let b_inc: Bound<i32> = Bound<i32>.included(3i32);
            let b_exc: Bound<i32> = Bound<i32>.excluded(5i32);
            let b_unb: Bound<i32> = Bound<i32>.unbounded;
            let b_match = (b_inc match { .included(v) => v == 3i32, _ => false })
                and (b_exc match { .excluded(v) => v == 5i32, _ => false })
                and (b_unb match { .unbounded => true, _ => false });

            CT_CHECK
                and contains_ok
                and trait_ok
                and bound_ok
                and into_inner_ok
                and b_match
                and check_once_range() == 35
        }
    "#;

    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn range_iteration_and_overflow_safe_u8_inclusive_work_in_vm_and_native() {
    let source = r#"
        use builtin.convert.TryFrom;

        fn widen_u8(x: u8) -> u32 {
            u32.as(TryFrom<u8>).try_from(x) match {
                v! => v,
                error _ => 0,
            }
        }

        fn widen_i8(x: i8) -> i32 {
            i32.as(TryFrom<i8>).try_from(x) match {
                v! => v,
                error _ => 0,
            }
        }

        fn add_i32_bounded(acc: i32, delta: i32) -> i32 {
            if acc >= -10000 and acc <= 10000 and delta >= -10000 and delta <= 10000 {
                acc + delta
            } else {
                acc
            }
        }

        fn add_u32_bounded(acc: u32, delta: u32) -> u32 {
            if acc <= 1000000 and delta <= 1000000 {
                acc + delta
            } else {
                acc
            }
        }

        fn sum_0_to_10() -> i32 {
            var acc: i32 = 0;
            for x in 0i32..10i32 {
                acc = add_i32_bounded(acc, x);
            }
            acc
        }

        fn sum_for_0_to_5() -> i32 {
            var acc: i32 = 0;
            for x in 0i32..5i32 {
                acc = add_i32_bounded(acc, x);
            }
            acc
        }

        fn sum_u8_full_inclusive() -> u32 {
            var acc: u32 = 0;
            for x in 0u8..=255u8 {
                acc = add_u32_bounded(acc, widen_u8(x));
            }
            acc
        }

        fn sum_from_take_5() -> u32 {
            var acc: u32 = 0;
            for v in (0u8..).as(Iterator).take(5) {
                acc = add_u32_bounded(acc, widen_u8(v));
            }
            acc
        }

        fn sum_signed_inclusive() -> i32 {
            var acc: i32 = 0;
            for x in (-3i8)..=2i8 {
                acc = add_i32_bounded(acc, widen_i8(x));
            }
            acc
        }

        fn sum_empty_inclusive() -> i32 {
            var acc: i32 = 99;
            for x in 10i8..=-10i8 {
                acc = add_i32_bounded(acc, widen_i8(x));
            }
            acc
        }

        fn run() -> u32 {
            let ok = sum_0_to_10() == 45i32
                and sum_for_0_to_5() == 10i32
                and sum_from_take_5() == 10
                and sum_signed_inclusive() == -3i32
                and sum_empty_inclusive() == 99i32;
            if ok {
                sum_u8_full_inclusive()
            } else {
                0
            }
        }
    "#;

    assert_eq!(support::run(source), Value::UnsignedInteger(32640));

    let native_source = r#"
        use std.prelude.*;
        use builtin.convert.TryFrom;
        use std.mem.DefaultAllocator;
        use std.string.String;

        fn widen_u8(x: u8) -> u32 {
            u32.as(TryFrom<u8>).try_from(x) match {
                v! => v,
                error _ => 0,
            }
        }

        fn add_u32_bounded(acc: u32, delta: u32) -> u32 {
            if acc <= 1000000 and delta <= 1000000 {
                acc + delta
            } else {
                acc
            }
        }

        fn sum_u8_full_inclusive() -> u32 {
            var acc: u32 = 0;
            for x in 0u8..=255u8 {
                acc = add_u32_bounded(acc, widen_u8(x));
            }
            acc
        }

        fn run() -> u8 {
            let s0 = String.from_str("hello") match {
                v! => v,
                error _ => return 2u8,
            };
            let s = s0 ++ " world";
            let sum_ok = sum_u8_full_inclusive() == 32640;
            let str_ok = s.ref.equals_str("hello world") and s.ref.len() == 11;
            if sum_ok and str_ok { 0u8 } else { 1u8 }
        }
    "#;

    let out = compile_native_and_run(native_source);
    assert!(
        out.status.success(),
        "native execution failed with status {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn concat_default_and_custom_allocator_with_decide_alloc_err_and_comptime_concat_work() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{
            AllocErr,
            Allocator,
            DecideAllocErr,
            DefaultAllocator,
            FixedBufferAllocator,
            Id,
            Layout,
            SystemAllocator,
        };
        use std.string.String;

        const CT_STR: str = "flurry" ++ "-" ++ "lang";
        const CT_LIST: meta.List<i32> = [1i32, 2i32] ++ [3i32, 4i32];
        const CT_LIST_OK: bool =
            CT_LIST.length() == 4 and CT_LIST[0] == 1i32 and CT_LIST[3] == 4i32;

        struct CustomAlloc {}
        const CUSTOM_GLOBAL: CustomAlloc = CustomAlloc {};

        const CT_ID_SAME: bool = Id<DefaultAllocator.GLOBAL.ref> == Id<DefaultAllocator.GLOBAL.ref>;
        const CT_ERR_DEFAULT_LIST: meta.List<Type> = DecideAllocErr(DefaultAllocator, DefaultAllocator.GLOBAL.ref);
        const CT_ERR_CUSTOM_LIST: meta.List<Type> = DecideAllocErr(CustomAlloc, CUSTOM_GLOBAL.ref);
        const CT_ERR_DEFAULT_EMPTY: bool = CT_ERR_DEFAULT_LIST.length() == 0;
        const CT_ERR_CUSTOM_ALLOC_ERR: bool =
            CT_ERR_CUSTOM_LIST.length() == 1 and CT_ERR_CUSTOM_LIST[0] == AllocErr;

        fn append_bang(
            s: String<.A = A, .allocator = a>,
            suffix: str,
        ) -> !DecideAllocErr(A, a) String<.A = A, .allocator = a>
        where
            A: Type,
            requires A :- Allocator,
            quote a: lift *A,
        {
            s ++ suffix
        }

        fn test_custom_allocator(fba: *FixedBufferAllocator) -> !AllocErr bool {
            let s = String<.A = FixedBufferAllocator, .allocator = fba>.from_str("alpha")!;
            let combined = (s ++ "-beta")!;
            let via_generic = append_bang(combined, "!")!;
            via_generic.ref.equals_str("alpha-beta!")
        }

        fn test_custom_allocator_oom(fba: *FixedBufferAllocator) -> bool {
            let s = String<.A = FixedBufferAllocator, .allocator = fba>.from_str("abcd") match {
                v! => v,
                error _ => return false,
            };
            (s ++ "0123456789012345678901234567890123456789") match {
                _! => false,
                error err => err match {
                    .OutOfMemory => true,
                    .InvalidLayout => false,
                },
            }
        }

        fn test_non_global_default_allocator() -> bool {
            var local_default = DefaultAllocator {};
            let local_ok = String<.A = DefaultAllocator, .allocator = local_default.ref>.from_str("hi") match {
                s! => (s ++ "!") match {
                    res! => res.ref.equals_str("hi!"),
                    error _ => false,
                },
                error _ => false,
            };
            local_ok
        }

        fn run() -> u8 {
            let s1 = String.from_str("hello") match {
                v! => v,
                error _ => return 2u8,
            };
            let s2 = s1 ++ ", " ++ "flurry!";
            let default_ok = s2.ref.equals_str("hello, flurry!");

            let s3 = String.from_str("generic") match {
                v! => v,
                error _ => return 3u8,
            };
            let s4 = append_bang(s3, "-infallible");
            let generic_default_ok = s4.ref.equals_str("generic-infallible");

            let bv3: BitVec<3> = 0b101;
            let bv2: BitVec<2> = 0b11;
            let bv5: BitVec<5> = bv3 ++ bv2;
            let bits_ok = bv5 == 0b10111;

            let ct_ok = CT_STR == "flurry-lang"
                and CT_LIST_OK
                and CT_ID_SAME
                and CT_ERR_DEFAULT_EMPTY
                and CT_ERR_CUSTOM_ALLOC_ERR;

            let layout_big = Layout.new(512, 16) match {
                found? => found,
                null => return 4u8,
            };
            let backing_big = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout_big) match {
                block! => block,
                error _ => return 5u8,
            };
            var fba_big = FixedBufferAllocator.from_raw_block(backing_big);
            let custom_ok = test_custom_allocator(fba_big.ref) match {
                ok! => ok,
                error _ => false,
            };
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing_big);
            }

            let layout_tiny = Layout.new(16, 8) match {
                found? => found,
                null => return 6u8,
            };
            let backing_tiny = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout_tiny) match {
                block! => block,
                error _ => return 7u8,
            };
            var fba_tiny = FixedBufferAllocator.from_raw_block(backing_tiny);
            let oom_ok = test_custom_allocator_oom(fba_tiny.ref);
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing_tiny);
            }

            if default_ok
                and generic_default_ok
                and bits_ok
                and ct_ok
                and custom_ok
                and oom_ok
                and test_non_global_default_allocator()
            {
                0u8
            } else {
                1u8
            }
        }
    "#;

    let vm_concat_source = r#"
        const CT_STR: str = "flurry" ++ "-" ++ "lang";
        const CT_LIST: meta.List<i32> = [1i32, 2i32] ++ [3i32, 4i32];

        pure comptime fn ct_check() -> bool {
            CT_STR == "flurry-lang"
                and CT_LIST.length() == 4
                and CT_LIST[0] == 1i32
                and CT_LIST[3] == 4i32
        }

        const CT_OK: bool = ct_check();

        fn run() -> bool {
            CT_OK
        }
    "#;
    assert_eq!(support::run(vm_concat_source), Value::Bool(true));

    let out = compile_native_and_run(source);
    assert!(
        out.status.success(),
        "native execution failed with status {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // Omitting error handling for String<.A = FixedBufferAllocator, ...> ++ str must fail.
    let unhandled_custom_concat = r#"
        use std.prelude.*;
        use std.mem.FixedBufferAllocator;
        use std.string.String;

        fn bad(fba: *FixedBufferAllocator, s: String<.A = FixedBufferAllocator, .allocator = fba>) -> String<.A = FixedBufferAllocator, .allocator = fba> {
            s ++ "x"
        }
    "#;
    assert!(
        support::compile(unhandled_custom_concat).is_err(),
        "custom allocator String ++ str must require handling AllocErr"
    );

    // Runtime str ++ str is intentionally unsupported (ComptimeConcat only).
    let runtime_str_concat = r#"
        fn bad(a: str, b: str) -> str {
            a ++ b
        }
    "#;
    assert!(
        support::compile(runtime_str_concat).is_err(),
        "runtime str ++ str should be rejected"
    );
}

#[test]
fn signed_integer_and_float_div_rem_work_and_verify_preconditions() {
    let source = r#"
        pure comptime fn ct_div_rem_check() -> bool {
            let p7_8: i8 = 7;
            let n7_8: i8 = -7;
            let p2_8: i8 = 2;
            let n2_8: i8 = -2;
            let q3_8: i8 = 3;
            let n3_8: i8 = -3;
            let p1_8: i8 = 1;
            let n1_8: i8 = -1;
            let i8_ok = (n7_8 / p2_8) == n3_8
                and (n7_8 % p2_8) == n1_8
                and (p7_8 / n2_8) == n3_8
                and (p7_8 % n2_8) == p1_8
                and (n7_8 / n2_8) == q3_8
                and (n7_8 % n2_8) == n1_8;

            let n100_16: i16 = -100;
            let p7_16: i16 = 7;
            let n14_16: i16 = -14;
            let n2_16: i16 = -2;
            let i16_ok = (n100_16 / p7_16) == n14_16 and (n100_16 % p7_16) == n2_16;

            let n7_32: i32 = -7;
            let p2_32: i32 = 2;
            let n3_32: i32 = -3;
            let n1_32: i32 = -1;
            let i32_ok = (n7_32 / p2_32) == n3_32 and (n7_32 % p2_32) == n1_32;

            let n_64: i64 = -1000000000007;
            let d_64: i64 = 1000;
            let q_64: i64 = -1000000000;
            let r_64: i64 = -7;
            let i64_ok = (n_64 / d_64) == q_64 and (n_64 % d_64) == r_64;

            let n7_128: i128 = -7;
            let p2_128: i128 = 2;
            let n3_128: i128 = -3;
            let n1_128: i128 = -1;
            let i128_ok = (n7_128 / p2_128) == n3_128 and (n7_128 % p2_128) == n1_128;

            let n29_s: isize = -29;
            let p5_s: isize = 5;
            let n5_s: isize = -5;
            let n4_s: isize = -4;
            let isize_ok = (n29_s / p5_s) == n5_s and (n29_s % p5_s) == n4_s;

            let fa16: f16 = 7.5;
            let fb16: f16 = 2.5;
            let fc16: f16 = 2.0;
            let f16_q: f16 = 3.0;
            let f16_r: f16 = 1.5;
            let f16_ok = (fa16 / fb16) == f16_q and (fa16 % fc16) == f16_r;

            let fabf16: bf16 = 7.5;
            let fbbf16: bf16 = 2.5;
            let fcbf16: bf16 = 2.0;
            let bf16_q: bf16 = 3.0;
            let bf16_r: bf16 = 1.5;
            let bf16_ok = (fabf16 / fbbf16) == bf16_q and (fabf16 % fcbf16) == bf16_r;

            let fa32: f32 = -7.5;
            let fb32: f32 = 2.5;
            let fc32: f32 = 2.0;
            let f32_q: f32 = -3.0;
            let f32_r: f32 = -1.5;
            let f32_ok = (fa32 / fb32) == f32_q and (fa32 % fc32) == f32_r;

            let fa64: f64 = -7.5;
            let fb64: f64 = 2.5;
            let fc64: f64 = 2.0;
            let f64_q: f64 = -3.0;
            let f64_r: f64 = -1.5;
            let f64_ok = (fa64 / fb64) == f64_q and (fa64 % fc64) == f64_r;

            i8_ok and i16_ok and i32_ok and i64_ok and i128_ok and isize_ok
                and f16_ok and bf16_ok and f32_ok and f64_ok
        }

        const CT_DIV_REM_OK: bool = ct_div_rem_check();

        fn rt_i8_pos(a: i8, b: i8) -> Tuple<i8, i8>
        where
            requires b > 0,
        {
            (a / b, a % b)
        }

        fn rt_i8_neg(a: i8, b: i8) -> Tuple<i8, i8>
        where
            requires b < -1,
        {
            (a / b, a % b)
        }

        fn rt_i16(a: i16, b: i16) -> Tuple<i16, i16>
        where
            requires b > 0,
        {
            (a / b, a % b)
        }

        fn rt_i32(a: i32, b: i32) -> Tuple<i32, i32>
        where
            requires b > 0,
        {
            (a / b, a % b)
        }

        fn rt_i64(a: i64, b: i64) -> Tuple<i64, i64>
        where
            requires b > 0,
        {
            (a / b, a % b)
        }

        fn rt_i128(a: i128, b: i128) -> Tuple<i128, i128>
        where
            requires b > 0,
        {
            (a / b, a % b)
        }

        fn rt_isize(a: isize, b: isize) -> Tuple<isize, isize>
        where
            requires b > 0,
        {
            (a / b, a % b)
        }

        fn run() -> bool {
            let p7_8: i8 = 7;
            let n7_8: i8 = -7;
            let p2_8: i8 = 2;
            let n2_8: i8 = -2;
            let (q8, r8) = rt_i8_pos(n7_8, p2_8);
            let (q8b, r8b) = rt_i8_neg(p7_8, n2_8);
            let (q8c, r8c) = rt_i8_neg(n7_8, n2_8);

            let n100_16: i16 = -100;
            let p7_16: i16 = 7;
            let (q16, r16) = rt_i16(n100_16, p7_16);

            let n7_32: i32 = -7;
            let p2_32: i32 = 2;
            let (q32, r32) = rt_i32(n7_32, p2_32);

            let n_64: i64 = -1000000000007;
            let d_64: i64 = 1000;
            let (q64, r64) = rt_i64(n_64, d_64);

            let n7_128: i128 = -7;
            let p2_128: i128 = 2;
            let (q128, r128) = rt_i128(n7_128, p2_128);

            let n29_s: isize = -29;
            let p5_s: isize = 5;
            let (qs, rs) = rt_isize(n29_s, p5_s);

            let fa16: f16 = 7.5;
            let fd16: f16 = 2.5;
            let fb16: f16 = 2.0;
            let fq16: f16 = 3.0;
            let fr16: f16 = 1.5;

            let fabf16: bf16 = 7.5;
            let fdbf16: bf16 = 2.5;
            let fbbf16: bf16 = 2.0;
            let fqbf16: bf16 = 3.0;
            let frbf16: bf16 = 1.5;

            let fa32: f32 = -7.5;
            let fd32: f32 = 2.5;
            let fb32: f32 = 2.0;
            let fq32: f32 = -3.0;
            let fr32: f32 = -1.5;

            let fa64: f64 = -7.5;
            let fd64: f64 = 2.5;
            let fb64: f64 = 2.0;
            let fq64: f64 = -3.0;
            let fr64: f64 = -1.5;

            CT_DIV_REM_OK
                and q8 == -3i8 and r8 == -1i8
                and q8b == -3i8 and r8b == 1i8
                and q8c == 3i8 and r8c == -1i8
                and q16 == -14i16 and r16 == -2i16
                and q32 == -3i32 and r32 == -1i32
                and q64 == -1000000000i64 and r64 == -7i64
                and q128 == -3i128 and r128 == -1i128
                and qs == -5isize and rs == -4isize
                and (fa16 / fd16) == fq16 and (fa16 % fb16) == fr16
                and (fabf16 / fdbf16) == fqbf16 and (fabf16 % fbbf16) == frbf16
                and (fa32 / fd32) == fq32 and (fa32 % fb32) == fr32
                and (fa64 / fd64) == fq64 and (fa64 % fb64) == fr64
        }
    "#;

    assert_eq!(support::run(source), Value::Bool(true));

    let bad_div_zero = "fn bad(a: i32, b: i32) -> i32 { a / b }";
    let bad_rem_zero = "fn bad(a: i32, b: i32) -> i32 { a % b }";
    let bad_min_div_neg_one = "fn bad(a: i8, b: i8) -> i8 where requires b != 0i8 { a / b }";
    let bad_min_rem_neg_one = "fn bad(a: i8, b: i8) -> i8 where requires b != 0i8 { a % b }";
    assert!(support::compile(bad_div_zero).is_err());
    assert!(support::compile(bad_rem_zero).is_err());
    assert!(support::compile(bad_min_div_neg_one).is_err());
    assert!(support::compile(bad_min_rem_neg_one).is_err());
}
