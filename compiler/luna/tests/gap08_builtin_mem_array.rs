//! End-to-end real-sysroot tests for Wave 2B (`--sysroot library`):
//! - `Array<T, N>` literal, indexing, iteration, Clone/Copy/Eq/Ord/Hash/Default, affine/Drop, and `VariadicCollector`
//! - `Slice<T>` `VariadicCollector` zero-heap stack buffer (`VariadicArrayBuffer`) and element `Drop`
//! - `builtin.mem` core (`Layout`, `Align`, `NonNull<T>`, `MaybeUninit<T>`, `ManuallyDrop<T>`, `TypeMarker<T>`, typed `*T` ops) + `std.mem` re-export
//! - `^attrs.layout.align(bytes)` (`__flurry_align`) layout & native LLVM alignment checks + invalid target/value rejections
//! - `meta.diagnostics`, `meta.compilation`, and `meta.runtime.bounded_stack_bytes`
//! - `native-text-method-provenance.fl` and `str` `TextBytes.slice`

#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-gap08-wave2b-{}-{}",
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
            .arg(Self::library_dir())
            .output()
            .unwrap()
    }

    fn run_binary(&self) -> Output {
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
fn array_literals_indexing_mutation_iterators_and_traits_native() {
    let source = r#"
        use builtin.mem.TypeMarker;
        use std.hash.DefaultHasher;

        fn sum_slice(s: Slice<i32>) -> i32 {
            var acc: i32 = 0;
            var i: usize = 0;
            let s_len = s.len();
            while i < s_len {
                s.get(i) match {
                    v? => {
                        assumes acc >= -10000 and acc <= 10000 and v >= -10000 and v <= 10000;
                        acc = acc + v;
                    },
                    null => {},
                }
                i = i + 1;
            }
            acc
        }

        fn make_val(idx: usize) -> i32 {
            if idx == 0 { 5 } else if idx == 1 { 15 } else { 25 }
        }

        fn run() -> bool {
            var xs: Array<i32, 3> = [10, 20, 30];
            xs[1] = 22;
            let x0 = xs[0].*;
            let x1 = xs[1].*;
            let x2 = xs[2].*;
            assumes x0 >= -10000 and x0 <= 10000
                and x1 >= -10000 and x1 <= 10000
                and x2 >= -10000 and x2 <= 10000;
            let s0 = x0 + x1 + x2;

            let ys = xs.clone();
            let eq_ok = xs == ys;

            var zs: Array<i32, 3> = [10, 23, 30];
            let ord_ok = xs < zs;

            let rep: Array<i32, 4> = Array<i32, 4>.from_repeat(7);
            let f_arr: Array<i32, 3> = Array<i32, 3>.from_fn(|i: usize| -> i32 { make_val(i) });

            var iter_sum: i32 = 0;
            for item in f_arr {
                assumes iter_sum >= -10000 and iter_sum <= 10000
                    and item >= -10000 and item <= 10000;
                iter_sum = iter_sum + item;
            }

            let slice_sum = sum_slice(xs.as_slice());
            let get_ok = (xs.get(1) matches 22?) and (xs.get(3) matches null);
            let first_last_ok = xs.first() match {
                fp? => xs.last() match {
                    lp? => unsafe { fp.* == 10 and lp.* == 30 },
                    null => false,
                },
                null => false,
            };

            let def_arr: Array<TypeMarker<i32>, 2> = Array<TypeMarker<i32>, 2>.as(Default).default();
            let def_ok = def_arr.len() == 2 and Type.size_of(TypeMarker<i32>) == 0;

            var h1 = DefaultHasher.new();
            var h2 = DefaultHasher.new();
            xs.hash(h1.ref);
            ys.hash(h2.ref);
            let hash_ok = h1.finish() == h2.finish();

            let empty_arr: Array<i32, 0> = [];
            let empty_ok = empty_arr.len() == 0 and empty_arr.is_empty() and (empty_arr.first() matches null);

            s0 == 62
                and eq_ok
                and ord_ok
                and rep.len() == 4
                and rep[0].* == 7 and rep[3].* == 7
                and iter_sum == 45
                and slice_sum == 62
                and get_ok
                and first_last_ok
                and def_ok
                and hash_ok
                and empty_ok
        }
    "#;

    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt));
        let out = case.run_binary();
        assert_success(&out);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "(bool true)");
    }
}

#[test]
fn array_affine_drop_and_early_iterator_break() {
    let source = r#"
        struct Tracked {
            id: u64,
            drops: *u64,
        }

        impl Drop for Tracked {
            fn drop(*self) {
                unsafe {
                    assumes self.drops.*.* < 10000;
                    self.drops.*.* = self.drops.*.* + 1;
                }
            }
        }

        fn build_and_drop(drops: *u64) {
            let arr: Array<Tracked, 3> = [
                Tracked { id: 1, drops: drops },
                Tracked { id: 2, drops: drops },
                Tracked { id: 3, drops: drops },
            ];
            let _ = arr.len();
        }

        fn break_early(drops: *u64) -> u64 {
            let arr: Array<Tracked, 4> = [
                Tracked { id: 10, drops: drops },
                Tracked { id: 20, drops: drops },
                Tracked { id: 30, drops: drops },
                Tracked { id: 40, drops: drops },
            ];
            var picked: u64 = 0;
            for item in arr {
                picked = item.id;
                if picked == 20 {
                    break;
                }
            }
            picked
        }

        fn run() -> bool {
            var count: u64 = 0;
            build_and_drop(count.ref);
            let after_scope = count == 3;
            let picked = break_early(count.ref);
            after_scope and picked == 20 and count == 7
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build("0"));
    let out = case.run_binary();
    assert_success(&out);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "(bool true)");
}

#[test]
fn array_and_slice_variadic_collectors_native_and_rejections() {
    let source = r#"
        struct DropItem {
            value: i32,
            drops: *u64,
        }

        impl Drop for DropItem {
            fn drop(*self) {
                unsafe {
                    assumes self.drops.*.* < 10000;
                    self.drops.*.* = self.drops.*.* + 1;
                }
            }
        }

        fn sum_fixed(...xs: Array<i32, 3>) -> i32 {
            let a = xs[0].*;
            let b = xs[1].*;
            let c = xs[2].*;
            assumes a >= -10000 and a <= 10000
                and b >= -10000 and b <= 10000
                and c >= -10000 and c <= 10000;
            a + b + c
        }

        fn empty_fixed(...xs: Array<i32, 0>) -> usize {
            xs.len()
        }

        fn sum_slice(...xs: Slice<i32>) -> i32 {
            var acc: i32 = 0;
            var i: usize = 0;
            let len = xs.len();
            while i < len {
                xs.get(i) match {
                    v? => {
                        assumes acc >= -10000 and acc <= 10000 and v >= -10000 and v <= 10000;
                        acc = acc + v;
                    },
                    null => {},
                }
                i = i + 1;
            }
            acc
        }

        fn inspect_dropped_slice(...xs: Slice<DropItem>) -> i32 {
            var acc: i32 = 0;
            var i: usize = 0;
            let len = xs.len();
            while i < len {
                xs.get_ref(i) match {
                    p? => unsafe {
                        let v = p.*.value;
                        assumes acc >= -10000 and acc <= 10000 and v >= -10000 and v <= 10000;
                        acc = acc + v;
                    },
                    null => {},
                }
                i = i + 1;
            }
            acc
        }

        fn run() -> bool {
            let arr_ok = sum_fixed(10, 20, 30) == 60 and empty_fixed() == 0;
            let slice_ok = sum_slice(4, 5, 6) == 15 and sum_slice() == 0;

            var drops: u64 = 0;
            let item_sum = inspect_dropped_slice(
                DropItem { value: 7, drops: drops.ref },
                DropItem { value: 8, drops: drops.ref },
                DropItem { value: 9, drops: drops.ref },
            );
            arr_ok and slice_ok and item_sum == 24 and drops == 3
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build("0"));
    let out = case.run_binary();
    assert_success(&out);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "(bool true)");

    let length_mismatch = r#"
        fn sum_fixed(...xs: Array<i32, 3>) -> i32 { xs[0].* }
        fn run() -> i32 { sum_fixed(10, 20) }
    "#;
    assert!(support::compile(length_mismatch).is_err());

    let type_mismatch = r#"
        fn sum_fixed(...xs: Array<i32, 2>) -> i32 { xs[0].* }
        fn run() -> i32 { sum_fixed(10, "wrong") }
    "#;
    assert!(support::compile(type_mismatch).is_err());
}

#[test]
fn builtin_mem_primitives_and_std_reexports_native() {
    let source = r#"
        use builtin.mem.{
            Align,
            Layout,
            ManuallyDrop,
            MaybeUninit,
            NonNull,
            TypeMarker,
            copy_nonoverlapping,
            drop_in_place,
            forget,
            make_type_marker,
            ptr_addr,
            replace,
            swap,
            zeroed,
        }
        use std.mem as std_mem;

        struct Token {
            id: u64,
            drops: *u64,
        }

        impl Drop for Token {
            fn drop(*self) {
                unsafe {
                    assumes self.drops.*.* < 10000;
                    self.drops.*.* = self.drops.*.* + 1;
                }
            }
        }

        fn run() -> bool {
            let l1 = Layout.of<u64>() match {
                v? => v,
                null => return false,
            };
            let l2 = std_mem.Layout.of<u64>() match {
                v? => v,
                null => return false,
            };
            let layout_ok = l1.byte_size() == l2.byte_size() and l1.byte_alignment() == l2.byte_alignment();

            let align_bad = Align.new(3) matches null;
            let align_ok = Align.new(16) match {
                a? => a.as_usize() == 16 and Align.of_type<u64>().as_usize() >= 8,
                null => false,
            };

            let marker: TypeMarker<u64> = make_type_marker<u64>();
            let marker_ok = Type.size_of(TypeMarker<u64>) == 0;
            let _ = marker;

            var value: u64 = 11;
            var other: u64 = 22;
            let nn_null = std_mem.NonNull<u64>.from_addr(0) matches null;
            let nn_ok = unsafe { NonNull<u64>.new(value.ref) } match {
                nn? => unsafe {
                    let prev = nn.read();
                    nn.write(33);
                    prev == 11 and value == 33 and nn.as_usize() == ptr_addr(value.ref)
                },
                null => false,
            };

            unsafe {
                swap(value.ref, other.ref);
            }
            let swap_ok = value == 22 and other == 33;
            let prev_other = unsafe { replace(other.ref, 44) };
            let replace_ok = prev_other == 33 and other == 44;

            var dst: u64 = 0;
            unsafe {
                copy_nonoverlapping(other.ref, dst.ref, 1);
            }
            let z: u64 = unsafe { zeroed<u64>() };

            var drops: u64 = 0;
            var uninit = unsafe { MaybeUninit<Token>.uninit() };
            unsafe {
                uninit.write(Token { id: 100, drops: drops.ref });
            }
            let drops_before_take = drops;
            let tok = unsafe { uninit.assume_init() };
            let tok_id = tok.id;
            forget(tok);
            let forget_ok = drops_before_take == 0 and drops == 0 and tok_id == 100;

            var md = ManuallyDrop<Token>.new(Token { id: 200, drops: drops.ref });
            let drops_while_md = drops;
            unsafe {
                ManuallyDrop<Token>.drop(md.ref);
            }
            let md_ok = drops_while_md == 0 and drops == 1;

            var direct_uninit = unsafe { MaybeUninit<Token>.uninit() };
            unsafe {
                direct_uninit.write(Token { id: 300, drops: drops.ref });
                drop_in_place(direct_uninit.as_mut_ptr());
            }
            let drop_in_place_ok = drops == 2;

            layout_ok
                and align_bad
                and align_ok
                and marker_ok
                and nn_null
                and nn_ok
                and swap_ok
                and replace_ok
                and dst == 44
                and z == 0
                and forget_ok
                and md_ok
                and drop_in_place_ok
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build("0"));
    let out = case.run_binary();
    assert_success(&out);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "(bool true)");
}

#[test]
fn attrs_layout_align_checks_and_native_alignment() {
    let source = r#"
        use builtin.mem.ptr_addr;

        ^attrs.layout.align(32)
        struct Aligned32 {
            a: u8,
            b: u32,
        }

        struct FieldAligned {
            tag: u8,
            ^attrs.layout.align(16)
            data: u32,
        }

        ^attrs.layout.align(16)
        newtype AlignedU64 = u64;

        fn is_aligned(addr: usize, align: usize) -> bool {
            assumes align > 0;
            (addr % align) == 0
        }

        fn run() -> bool {
            let s = Aligned32 { a: 1, b: 2 };
            let f = FieldAligned { tag: 7, data: 99 };
            let raw_n: u64 = 42;
            let n = raw_n.as(AlignedU64);

            Type.align_of(Aligned32) == 32
                and Type.size_of(Aligned32) == 32
                and Type.align_of(FieldAligned) >= 16
                and Type.align_of(AlignedU64) == 16
                and Type.size_of(AlignedU64) == 16
                and unsafe {
                    is_aligned(ptr_addr(s.ref), 32)
                        and is_aligned(ptr_addr(f.ref), 16)
                        and is_aligned(ptr_addr(f.data.ref), 16)
                        and is_aligned(ptr_addr(n.ref), 16)
                }
                and s.b == 2
                and f.data == 99
        }
    "#;

    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt));
        let out = case.run_binary();
        assert_success(&out);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "(bool true)");
    }

    for invalid in [
        "^attrs.layout.align(3) struct BadAlign { x: u32 }",
        "^attrs.layout.align(0) struct ZeroAlign { x: u32 }",
        "^attrs.layout.align.32 struct DotAlign { x: u32 }",
        "^attrs.packed.byte ^attrs.layout.align(16) struct Both { a: u8, b: u32 }",
    ] {
        let case = NativeCase::new(&format!("{invalid} fn run() -> bool {{ true }}"));
        assert!(
            !case.build("0").status.success(),
            "expected failure for `{invalid}`"
        );
    }
}

#[test]
fn meta_diagnostics_compilation_and_bounded_stack_bytes() {
    let source = r#"
        use meta.compilation;
        use meta.diagnostics.{Diagnostic, DiagnosticLevel};
        use meta.runtime;

        ^attrs.layout.align(64)
        struct Pad64 {
            x: u64,
        }

        fn leaf(v: u64) -> u64 {
            assumes v < 1000000;
            v + 1
        }

        fn aligned_local_fn(v: u64) -> u64 {
            let p = Pad64 { x: v };
            leaf(p.x)
        }

        fn recursive(n: u64) -> u64 {
            if n == 0 { 0 } else { recursive(n - 1) }
        }

        comptime fn inspect() -> bool {
            let target_cfg = compilation.cfg();
            let pointer_bytes = target_cfg.pointer_bytes();
            assumes pointer_bytes <= 16;
            let cfg_ok = target_cfg.pointer_bits() == pointer_bytes * 8
                and (target_cfg.is_64_bit() or target_cfg.is_32_bit());

            compilation.warn("wave2b-direct-warning");
            compilation.note("wave2b-direct-note");
            Diagnostic.builder(DiagnosticLevel.warning, "wave2b-builder-warning")
                .with_code("W2B")
                .with_note("extra-note")
                .emit();

            let leaf_bound = runtime.bounded_stack_bytes(leaf);
            let outer_bound = runtime.bounded_stack_bytes(aligned_local_fn);
            let rec_bound = runtime.bounded_stack_bytes(recursive);

            let bounds_ok = leaf_bound match {
                lb? => outer_bound match {
                    ob? => ob >= 64 and ob >= lb,
                    null => false,
                },
                null => false,
            };
            let rec_ok = rec_bound matches null;
            cfg_ok and bounds_ok and rec_ok
        }

        fn run() -> bool {
            const ok = inspect();
            ok
        }
    "#;

    let case = NativeCase::new(source);
    let build = case.build("0");
    assert_success(&build);
    let stderr = String::from_utf8_lossy(&build.stderr);
    assert!(stderr.contains("wave2b-direct-warning"), "{stderr}");
    assert!(stderr.contains("wave2b-direct-note"), "{stderr}");
    assert!(stderr.contains("[W2B] wave2b-builder-warning"), "{stderr}");

    let out = case.run_binary();
    assert_success(&out);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "(bool true)");

    let compile_err_src = r#"
        use meta.compilation;
        comptime fn trigger() -> bool {
            compilation.err("wave2b-explicit-compile-error");
            true
        }
        fn run() -> bool {
            const _ = trigger();
            true
        }
    "#;
    let err_case = NativeCase::new(compile_err_src);
    let err_build = err_case.build("0");
    assert!(!err_build.status.success());
    let err_stderr = String::from_utf8_lossy(&err_build.stderr);
    assert!(
        err_stderr.contains("wave2b-explicit-compile-error"),
        "{err_stderr}"
    );
}

#[test]
fn native_text_method_provenance_and_slice_with_sysroot() {
    let repro = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../development/repros/native-text-method-provenance.fl");
    let test_out = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
        .args(["test"])
        .arg(&repro)
        .arg("--sysroot")
        .arg(NativeCase::library_dir())
        .output()
        .unwrap();
    assert_success(&test_out);

    let slice_source = r#"
        fn run() -> bool {
            let text = "a🦀b";
            let len_ok = text.byte_length() == 6;
            let crab_ok = text.slice(1, 5) match {
                s? => s == "🦀",
                null => false,
            };
            let prefix_ok = text.slice(0, 1) match {
                s? => s == "a",
                null => false,
            };
            let empty_ok = text.slice(5, 5) match {
                s? => s == "",
                null => false,
            };
            let mid_char_empty = text.slice(2, 2) matches null;
            let bad_boundary = text.slice(1, 3) matches null;
            len_ok and crab_ok and prefix_ok and empty_ok and mid_char_empty and bad_boundary
        }
    "#;
    let case = NativeCase::new(slice_source);
    assert_success(&case.build("0"));
    let out = case.run_binary();
    assert_success(&out);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "(bool true)");
}
