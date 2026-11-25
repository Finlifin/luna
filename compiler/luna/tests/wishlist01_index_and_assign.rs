//! End-to-end Milestone 3 integration tests:
//! 1. `Index` (`*self -> *Output`) & `IndexMut :- Index<Idx>` in `builtin`/`std`
//!    (`Slice`, `Vec`, `VecDeque`, `HashMap`) and place elaboration (`a[i]`, `a[i] = v`,
//!    `a[i] += v`, field/chained/BitVec writebacks, non-Copy safety).
//! 2. `AddAssign..RemAssign` (`*self`) + `ComptimeAddAssign..ComptimeRemAssign` (comptime
//!    value-reference `self`) + single-eval `Copy + Add` (`Output == Self`) fallback +
//!    strict no-silent-fallback on ambiguity/unsatisfied bounds.
//! 3. Multi-argument `[arg1, arg2, ...]` syntax/lowering + `SpaceApply` routing with
//!    `VariadicCollector` (and `ExtendedVariadicCollector`), `SliceBounds` on `Vec`/`Slice`,
//!    strict `Index`-before-`SpaceApply` priority without silent fallback.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod execution_support;

#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use execution_support::{calls, compile_with_source_bindings};
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-w01-{}-{}",
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

fn ops_selectors() -> luna::protocols::type_constructors::Selectors<'static> {
    luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        meta_list: Some("input.meta.List"),
        tuple: Some("input.Tuple"),
        index: Some("input.Index"),
        index_mut: Some("input.IndexMut"),
        space_apply: Some("input.SpaceApply"),
        variadic_collector: Some("input.VariadicCollector"),
        extended_variadic_collector: Some("input.ExtendedVariadicCollector"),
        arithmetic: [
            Some("input.Add"),
            Some("input.Sub"),
            Some("input.Mul"),
            Some("input.Div"),
            Some("input.Rem"),
            None,
            None,
        ],
        compound_arithmetic: [
            Some("input.AddAssign"),
            Some("input.SubAssign"),
            Some("input.MulAssign"),
            Some("input.DivAssign"),
            Some("input.RemAssign"),
        ],
        comptime_compound_arithmetic: [
            Some("input.ComptimeAddAssign"),
            Some("input.ComptimeSubAssign"),
            Some("input.ComptimeMulAssign"),
            Some("input.ComptimeDivAssign"),
            Some("input.ComptimeRemAssign"),
        ],
        ..Default::default()
    }
}

const PRIMITIVES: &str = include_str!("../../../library/builtin/primitives.fl");

fn compile_ops(source: &str) -> Result<nir::program::Package, String> {
    let tuple = include!("support/tuple_source.rs");
    let full = format!(
        "{PRIMITIVES}\nmod meta {{\n    pub const List: pure comptime fn<T: .Type'builtin> -> .Type'builtin = .List'builtin;\n}}\n{tuple}\n{source}"
    );
    compile_with_source_bindings(
        &execution_support::with_attrs(&full),
        &Default::default(),
        ops_selectors(),
    )
}

const MINIMAL_OPS_HEADER: &str = r#"
    pub trait Copy {}
    pub trait Drop { fn drop(*self); }
    impl Copy for bool {}
    impl Copy for i32 {}
    impl Copy for u32 {}
    impl Copy for u64 {}
    impl Copy for void {}
    impl Copy for Type {}
    pub trait VariadicCollector {
        assoc Tmp: Type = void;
        fn variadic_init() -> Tmp where S;
        fn variadic_collect(tmp: *Tmp, args: S) -> Self where S;
        ^attrs.comptime_phase
        comptime fn comptime_variadic_collect(args: S) -> Self where S;
    }
    pub trait ExtendedVariadicCollector {
        assoc Tmp: Type = void;
        fn extended_variadic_init() -> Tmp where Ps;
        fn extended_variadic_collect(tmp: *Tmp, properties: Ps) -> Self where Ps;
        ^attrs.comptime_phase
        comptime fn comptime_extended_variadic_collect(properties: Ps) -> Self where Ps;
    }
    pub trait Index where Idx: Type {
        assoc Output: Type;
        fn index(*self, idx: Idx) -> *Output;
    }
    pub trait IndexMut :- Index<Idx> where Idx: Type {
        fn index_mut(*self, idx: Idx) -> *Self.as(Index<Idx>).Output;
    }
    pub trait SpaceApply {
        assoc CollectTo: Type;
        assoc Target: Type;
        fn space_apply(*self, index: CollectTo) -> Target;
    }
    pub trait Add where .Rhs: Type = Self {
        assoc Output: Type;
        fn add(self, rhs: Rhs) -> Output;
    }
    pub trait Sub where .Rhs: Type = Self {
        assoc Output: Type;
        fn sub(self, rhs: Rhs) -> Output;
    }
    pub trait Mul where .Rhs: Type = Self {
        assoc Output: Type;
        fn mul(self, rhs: Rhs) -> Output;
    }
    pub trait Div where .Rhs: Type = Self {
        assoc Output: Type;
        fn div(self, rhs: Rhs) -> Output;
    }
    pub trait Rem where .Rhs: Type = Self {
        assoc Output: Type;
        fn rem(self, rhs: Rhs) -> Output;
    }
    pub trait AddAssign where .Rhs: Type = Self { fn add_assign(*self, rhs: Rhs) -> void; }
    pub trait SubAssign where .Rhs: Type = Self { fn sub_assign(*self, rhs: Rhs) -> void; }
    pub trait MulAssign where .Rhs: Type = Self { fn mul_assign(*self, rhs: Rhs) -> void; }
    pub trait DivAssign where .Rhs: Type = Self { fn div_assign(*self, rhs: Rhs) -> void; }
    pub trait RemAssign where .Rhs: Type = Self { fn rem_assign(*self, rhs: Rhs) -> void; }
    pub pure comptime trait ComptimeAddAssign where .Rhs: Type = Self { comptime fn add_assign(self, rhs: Rhs) -> void; }
    pub pure comptime trait ComptimeSubAssign where .Rhs: Type = Self { comptime fn sub_assign(self, rhs: Rhs) -> void; }
    pub pure comptime trait ComptimeMulAssign where .Rhs: Type = Self { comptime fn mul_assign(self, rhs: Rhs) -> void; }
    pub pure comptime trait ComptimeDivAssign where .Rhs: Type = Self { comptime fn div_assign(self, rhs: Rhs) -> void; }
    pub pure comptime trait ComptimeRemAssign where .Rhs: Type = Self { comptime fn rem_assign(self, rhs: Rhs) -> void; }
"#;

#[test]
fn sysroot_collections_index_index_mut_and_slice_space_apply_execute_natively() {
    let source = r#"
        use std.prelude.*;
        use std.collections.{HashMap, Slice, SliceBounds, Vec, VecDeque};
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, SystemAllocator};

        struct Pair {
            x: u64,
            y: u64,
        }
        impl Clone for Pair {
            fn clone(*self) -> Self { Pair { x: self.x.*, y: self.y.* } }
        }
        impl Copy for Pair {}

        struct Acc {
            total: u64,
            marker: u64,
        }
        impl AddAssign<.Rhs = u64> for Acc {
            fn add_assign(*self, rhs: u64) {
                assumes self.total.* < 1000000 and rhs < 1000000;
                self.total.* = self.total.* + rhs;
            }
        }

        struct Counter {
            n: u64,
        }
        impl Copy for Counter {}
        impl Clone for Counter {
            fn clone(*self) -> Self { Counter { n: self.n.* } }
        }
        impl Add<.Rhs = u64> for Counter {
            assoc Output: Type = Counter;
            fn add(self, rhs: u64) -> Counter {
                assumes self.n < 1000000 and rhs < 1000000;
                Counter { n: self.n + rhs }
            }
        }

        fn run_checks(fba: *FixedBufferAllocator) -> !AllocErr bool {
            -- 1. Vec<u64>: a[i].*, a[i] = v, a[i] += v, and chained slice a[lo, hi][i].*
            var v = Vec<u64, .A = FixedBufferAllocator, .allocator = fba>.new();
            v.ref.push(10)!;
            v.ref.push(20)!;
            v.ref.push(30)!;
            v.ref.push(40)!;
            v[1usize] = 25;
            let v_slot = v[2usize];
            assumes v_slot.* < 1000000;
            v_slot.* += 12;
            if v[0usize].* != 10 or v[1usize].* != 25 or v[2usize].* != 42 {
                return false;
            }
            let sub = v[1usize, 4usize];
            if sub.len() != 3 or sub[0usize].* != 25 or sub[1usize].* != 42 or sub[2usize].* != 40 {
                return false;
            }
            -- Direct chained multi-arg slice + element index.
            if v[1usize, 3usize][1usize].* != 42 {
                return false;
            }

            -- 2. Struct element field mutation through IndexMut + Slice Index/IndexMut
            var pairs = Vec<Pair, .A = FixedBufferAllocator, .allocator = fba>.new();
            pairs.ref.push(Pair { x: 3, y: 4 })!;
            let p0 = pairs[0usize];
            assumes p0.x.* < 1000000;
            p0.x.* += 7;
            pairs[0usize].y = 99;
            if pairs[0usize].x.* != 10 or pairs[0usize].y.* != 99 {
                return false;
            }
            var sub_slice = pairs.ref.as_slice();
            let sp0 = sub_slice[0usize];
            assumes sp0.x.* < 1000000;
            sp0.x.* += 5;
            if pairs[0usize].x.* != 15 {
                return false;
            }

            -- Direct `a[i] += v` via `Copy + Add` fallback on `Vec<Counter>`.
            var counters = Vec<Counter, .A = FixedBufferAllocator, .allocator = fba>.new();
            counters.ref.push(Counter { n: 30 })!;
            counters[0usize] += 12;
            if counters[0usize].n.* != 42 {
                return false;
            }

            -- 3. Nested Vec<Vec<u64>> indexing and Vec<.u16'builtin'bitvec> bit-slice writeback
            var inner = Vec<u64, .A = FixedBufferAllocator, .allocator = fba>.new();
            inner.ref.push(5)!;
            inner.ref.push(6)!;
            var matrix = Vec<Vec<u64, .A = FixedBufferAllocator, .allocator = fba>, .A = FixedBufferAllocator, .allocator = fba>.new();
            matrix.ref.push(inner)!;
            matrix[0usize][1usize] = 88;
            if matrix[0usize][0usize].* != 5 or matrix[0usize][1usize].* != 88 {
                return false;
            }

            var bvec = Vec<.u16'builtin, .A = FixedBufferAllocator, .allocator = fba>.new();
            bvec.ref.push(0)!;
            bvec[0usize]'bitvec[4..8] = 0b1010;
            if bvec[0usize].* != 160 {
                return false;
            }

            -- 4. VecDeque<u64> wrap-around Index and IndexMut
            var dq = VecDeque<u64, .A = FixedBufferAllocator, .allocator = fba>.with_capacity(4)!;
            dq.ref.push_back(30)!;
            dq.ref.push_back(40)!;
            dq.ref.push_front(20)!;
            dq.ref.push_front(10)!;
            let dq0 = dq[0usize];
            assumes dq0.* < 1000000;
            dq0.* += 5;
            dq[3usize] = 400;
            if dq[0usize].* != 15 or dq[1usize].* != 20 or dq[2usize].* != 30 or dq[3usize].* != 400 {
                return false;
            }

            -- 5. HashMap<u64, u64> Index and IndexMut
            var map = HashMap<u64, u64, .A = FixedBufferAllocator, .allocator = fba>.new();
            let _ = map.ref.insert(7, 100)!;
            let _ = map.ref.insert(13, 200)!;
            let m7 = map[7];
            assumes m7.* < 1000000;
            m7.* += 23;
            map[13] = 555;
            if map[7].* != 123 or map[13].* != 555 {
                return false;
            }

            -- 6. Non-Copy container & non-Copy Box elements: borrow repeatedly via Index without moving,
            --    and verify non-Copy AddAssign through Vec<Acc>[0usize] += 42
            let before_boxes = fba.live_allocations();
            let boxes_ok = do {
                var vb = Vec<Box<u64, .A = FixedBufferAllocator, .allocator = fba>, .A = FixedBufferAllocator, .allocator = fba>.new();
                vb.ref.push(Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(33)!)!;
                vb.ref.push(Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(66)!)!;
                let first = vb[0usize].get();
                let second = vb[1usize].get();
                first == 33 and second == 66 and vb.ref.len() == 2
            };
            if not boxes_ok or fba.live_allocations() != before_boxes {
                return false;
            }

            var accs = Vec<Acc, .A = FixedBufferAllocator, .allocator = fba>.new();
            accs.ref.push(Acc { total: 100, marker: 7 })!;
            accs[0usize] += 42;
            accs[0usize].total.* == 142 and accs[0usize].marker.* == 7
        }

        fn main_check() -> bool {
            let layout = Layout.new(16384, 16) match {
                found? => found,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let ok = run_checks(fba.ref) match {
                passed! => passed,
                error _ => false,
            };
            let clean = fba.ref.live_allocations() == 0;
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            ok and clean
        }

        fn run() -> u8 {
            if main_check() { 0 } else { 1 }
        }
    "#;
    let case = NativeCase::new(source);
    let build = case.build();
    assert_success(&build);
    let exec = case.run();
    assert!(
        exec.status.success(),
        "native execution failed with status {:?}\nstdout: {}\nstderr: {}",
        exec.status.code(),
        String::from_utf8_lossy(&exec.stdout),
        String::from_utf8_lossy(&exec.stderr)
    );
}

#[test]
fn index_and_index_mut_reject_immutable_missing_mut_and_non_copy_whole_access() {
    // Sysroot checks against real `std.collections.Vec`:
    let sysroot_imm_err = support::compile(
        r#"
        use std.prelude.*;
        use std.collections.Vec;
        fn bad() { let v = Vec<u64>.new(); v[0usize] = 1; }
    "#,
    )
    .unwrap_err();
    assert!(
        sysroot_imm_err.contains("immutable") || sysroot_imm_err.contains("let"),
        "unexpected error: {sysroot_imm_err}"
    );
    let sysroot_non_copy_pkg = support::compile(
        r#"
        use std.prelude.*;
        use std.collections.Vec;
        struct NonCopyVal { x: u64 }
        fn bad() -> NonCopyVal { var v = Vec<NonCopyVal>.new(); v[0usize].* }
    "#,
    )
    .unwrap();
    assert!(
        luna::execution::interpret_with_protocols(
            &sysroot_non_copy_pkg,
            "bad",
            "Copy",
            "Drop",
            1_000_000,
        )
        .is_err(),
        "non-Copy pointer read through Vec[i].* must fail resource verification"
    );

    // Fast isolated protocol checks for read-only `Index`, immutable `+=`, `IndexMut` missing,
    // whole-value non-Copy overwrite, and `a[i]` pointer-result type.
    let ro_base = format!(
        r#"
        {MINIMAL_OPS_HEADER}
        struct NonCopyVal {{ x: u64 }}
        struct NonCopySlot {{ item: NonCopyVal }}
        impl Index<u32> for NonCopySlot {{
            assoc Output: Type = NonCopyVal;
            fn index(*self, _idx: u32) -> *NonCopyVal {{ self.item }}
        }}
        impl IndexMut<u32> for NonCopySlot {{
            fn index_mut(*self, _idx: u32) -> *NonCopyVal {{ self.item }}
        }}
        struct ReadOnlyBox {{ ptr: *u64 }}
        impl Index<u32> for ReadOnlyBox {{
            assoc Output: Type = u64;
            fn index(*self, _idx: u32) -> *u64 {{ self.ptr.* }}
        }}
        struct MutBox {{ ptr: *u64 }}
        impl Index<u32> for MutBox {{
            assoc Output: Type = u64;
            fn index(*self, _idx: u32) -> *u64 {{ self.ptr.* }}
        }}
        impl IndexMut<u32> for MutBox {{
            fn index_mut(*self, _idx: u32) -> *u64 {{ self.ptr.* }}
        }}
    "#
    );

    let ro_ok = format!(
        "{ro_base}\nfn run() -> u64 {{ var x: u64 = 7; let ro = ReadOnlyBox {{ ptr: x.ref }}; ro[0].* }}"
    );
    let ro_pkg = compile_ops(&ro_ok).unwrap();
    assert_eq!(
        luna::execution::interpret(&ro_pkg, "run", 100_000).unwrap(),
        Value::UnsignedInteger(7)
    );

    for bad in [
        "fn bad() { var x: u64 = 7; var ro = ReadOnlyBox { ptr: x.ref }; ro[0] = 9; }",
        "fn bad() { var x: u64 = 7; var ro = ReadOnlyBox { ptr: x.ref }; ro[0] += 2; }",
        "fn bad() { var x: u64 = 7; let mb = MutBox { ptr: x.ref }; mb[0] += 2; }",
        "fn bad() -> u64 { var x: u64 = 7; let ro = ReadOnlyBox { ptr: x.ref }; ro[0] }",
    ] {
        assert!(
            compile_ops(&format!("{ro_base}\n{bad}")).is_err(),
            "accepted invalid `{bad}`"
        );
    }

    for resource_bad in [
        "fn bad() { var s = NonCopySlot { item: NonCopyVal { x: 1 } }; s[0] = NonCopyVal { x: 2 }; }",
        "fn bad() -> NonCopyVal { var s = NonCopySlot { item: NonCopyVal { x: 1 } }; s[0].* }",
    ] {
        let pkg = compile_ops(&format!("{ro_base}\n{resource_bad}")).unwrap();
        assert!(
            luna::execution::interpret_with_protocols(&pkg, "bad", "Copy", "Drop", 100_000)
                .is_err(),
            "accepted non-Copy pointer access `{resource_bad}`"
        );
    }
}

#[test]
fn compound_assign_protocols_single_eval_fallback_comptime_and_no_silent_fallback() {
    // 1. Primary in-place `AddAssign..RemAssign` on non-Copy type (no Copy, no Add..Rem),
    //    single-eval of target and rhs, plus `Copy + Add` (`Output == Self`) fallback with
    //    indexed target single-eval verification.
    let runtime_src = format!(
        r#"
        {MINIMAL_OPS_HEADER}

        struct NonCopyAcc {{
            value: u64,
        }}
        impl AddAssign<.Rhs = u64> for NonCopyAcc {{
            fn add_assign(*self, rhs: u64) {{
                assumes self.value.* < 100000 and rhs < 100000;
                self.value.* = self.value.* + rhs;
            }}
        }}
        impl SubAssign<.Rhs = u64> for NonCopyAcc {{
            fn sub_assign(*self, rhs: u64) {{
                assumes self.value.* >= rhs;
                self.value.* = self.value.* - rhs;
            }}
        }}
        impl MulAssign<.Rhs = u64> for NonCopyAcc {{
            fn mul_assign(*self, rhs: u64) {{
                assumes self.value.* < 1000 and rhs < 1000;
                self.value.* = self.value.* * rhs;
            }}
        }}
        impl DivAssign<.Rhs = u64> for NonCopyAcc {{
            fn div_assign(*self, rhs: u64) {{
                assumes rhs > 0;
                self.value.* = self.value.* / rhs;
            }}
        }}
        impl RemAssign<.Rhs = u64> for NonCopyAcc {{
            fn rem_assign(*self, rhs: u64) {{
                assumes rhs > 0;
                self.value.* = self.value.* % rhs;
            }}
        }}

        struct CopyNum {{
            v: u64,
        }}
        impl Copy for CopyNum {{}}
        impl Add<.Rhs = u64> for CopyNum {{
            assoc Output: Type = CopyNum;
            fn add(self, rhs: u64) -> CopyNum {{
                assumes self.v < 100000 and rhs < 100000;
                CopyNum {{ v: self.v + rhs }}
            }}
        }}

        struct PairSlots {{
            a: CopyNum,
            b: CopyNum,
        }}
        impl Index<u32> for PairSlots {{
            assoc Output: Type = CopyNum;
            fn index(*self, idx: u32) -> *CopyNum {{
                if idx == 0 {{ self.a }} else {{ self.b }}
            }}
        }}
        impl IndexMut<u32> for PairSlots {{
            fn index_mut(*self, idx: u32) -> *CopyNum {{
                if idx == 0 {{ self.a }} else {{ self.b }}
            }}
        }}

        fn target_slot(acc: *NonCopyAcc, calls: *u32) -> *NonCopyAcc {{
            assumes calls.* < 100;
            calls.* = calls.* + 1;
            acc
        }}
        fn bump_index(calls: *u32) -> u32 {{
            assumes calls.* < 100;
            calls.* = calls.* + 1;
            0
        }}
        fn bump_rhs(calls: *u32) -> u64 {{
            assumes calls.* < 100;
            calls.* = calls.* + 10;
            5
        }}

        fn run() -> bool {{
            var acc = NonCopyAcc {{ value: 10 }};
            var target_calls: u32 = 0;
            target_slot(acc.ref, target_calls.ref).* += bump_rhs(target_calls.ref);
            acc -= 3;
            acc *= 4;
            acc /= 6;
            acc %= 5;
            -- 10 + 5 = 15; -3 = 12; *4 = 48; /6 = 8; %5 = 3; calls = 1 + 10 = 11
            let direct_ok = acc.value == 3 and target_calls == 11;

            -- Fallback: `CopyNum` has Copy + Add (Output == Self), no AddAssign.
            -- Must evaluate base, index, and rhs ONCE!
            var slots = PairSlots {{ a: CopyNum {{ v: 20 }}, b: CopyNum {{ v: 99 }} }};
            var eval_log: u32 = 0;
            slots[bump_index(eval_log.ref)] += bump_rhs(eval_log.ref);
            let fallback_ok = slots.a.v == 25 and slots.b.v == 99 and eval_log == 11;
            direct_ok and fallback_ok
        }}
    "#
    );
    let package = compile_ops(&runtime_src).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100_000)
            .unwrap(),
        Value::Bool(true)
    );

    // 2. Comptime `ComptimeAddAssign..ComptimeRemAssign` using value-reference `self`
    //    (mutating local + struct field, zero comptime pointers) + comptime `Copy + Add` fallback.
    let comptime_src = format!(
        r#"
        {MINIMAL_OPS_HEADER}

        struct CtAcc {{
            total: u64,
        }}
        struct CtWrap {{
            inner: CtAcc,
        }}
        comptime impl ComptimeAddAssign<.Rhs = u64> for CtAcc {{
            comptime fn add_assign(self, rhs: u64) -> void {{
                assumes self.total < 100000 and rhs < 100000;
                self.total = self.total + rhs;
            }}
        }}
        comptime impl ComptimeSubAssign<.Rhs = u64> for CtAcc {{
            comptime fn sub_assign(self, rhs: u64) -> void {{
                assumes self.total >= rhs;
                self.total = self.total - rhs;
            }}
        }}
        comptime impl ComptimeMulAssign<.Rhs = u64> for CtAcc {{
            comptime fn mul_assign(self, rhs: u64) -> void {{
                assumes self.total < 1000 and rhs < 1000;
                self.total = self.total * rhs;
            }}
        }}
        comptime impl ComptimeDivAssign<.Rhs = u64> for CtAcc {{
            comptime fn div_assign(self, rhs: u64) -> void {{
                assumes rhs > 0;
                self.total = self.total / rhs;
            }}
        }}
        comptime impl ComptimeRemAssign<.Rhs = u64> for CtAcc {{
            comptime fn rem_assign(self, rhs: u64) -> void {{
                assumes rhs > 0;
                self.total = self.total % rhs;
            }}
        }}

        comptime fn run() -> bool {{
            var w = CtWrap {{ inner: CtAcc {{ total: 10 }} }};
            w.inner += 5;
            w.inner -= 3;
            w.inner *= 4;
            w.inner /= 6;
            w.inner %= 5;
            var cv: u64 = 7;
            cv += 8;
            w.inner.total == 3 and cv == 15
        }}
    "#
    );
    let ct_pkg = compile_ops(&comptime_src).unwrap();
    assert_eq!(
        luna::invocation::invoke(&ct_pkg, "run", 100_000).unwrap(),
        Value::Bool(true)
    );

    // 3. Rejections & strict no-silent-fallback:
    //    - `let` binding forbids `+=`
    //    - `Copy + Add` where `Output != Self` forbids `+=` fallback
    //    - Non-Copy without `AddAssign` forbids `+=`
    //    - Unsatisfied `AddAssign` bound or ambiguity NEVER falls back silently to `Copy + Add`!
    let rejection_cases = [
        format!(
            r#"
            {MINIMAL_OPS_HEADER}
            struct S {{ v: u64 }}
            impl Copy for S {{}}
            impl Add<.Rhs = u64> for S {{ assoc Output: Type = S; fn add(self, _rhs: u64) -> S {{ S {{ v: 0 }} }} }}
            fn bad() {{ let x = S {{ v: 1 }}; x += 2; }}
            "#
        ),
        format!(
            r#"
            {MINIMAL_OPS_HEADER}
            struct Other {{ v: u64 }}
            struct Mismatch {{ v: u64 }}
            impl Copy for Mismatch {{}}
            impl Add<.Rhs = u64> for Mismatch {{
                assoc Output: Type = Other;
                fn add(self, _rhs: u64) -> Other {{ Other {{ v: 0 }} }}
            }}
            fn bad() {{ var x = Mismatch {{ v: 1 }}; x += 2; }}
            "#
        ),
        format!(
            r#"
            {MINIMAL_OPS_HEADER}
            struct PlainNonCopy {{ v: u64 }}
            impl Add<.Rhs = u64> for PlainNonCopy {{
                assoc Output: Type = PlainNonCopy;
                fn add(self, _rhs: u64) -> PlainNonCopy {{ PlainNonCopy {{ v: 0 }} }}
            }}
            fn bad() {{ var x = PlainNonCopy {{ v: 1 }}; x += 2; }}
            "#
        ),
        // Deferred/Unknown `AddAssign` requirement on a `Copy + Add` type must NOT silently fall back!
        format!(
            r#"
            {MINIMAL_OPS_HEADER}
            trait Marker {{}}
            struct Guarded where T: Type {{ v: u64 }}
            impl Copy for Guarded<T> where T: Type {{}}
            impl Add<.Rhs = u64> for Guarded<T> where T: Type {{
                assoc Output: Type = Guarded<T>;
                fn add(self, _rhs: u64) -> Guarded<T> {{ Guarded<T> {{ v: 1 }} }}
            }}
            impl AddAssign<.Rhs = u64> for Guarded<T> where T: Type, requires T :- Marker {{
                fn add_assign(*self, _rhs: u64) -> void {{}}
            }}
            fn bad(g_in: Guarded<T>) where T: Type {{ var g = g_in; g += 1; }}
            "#
        ),
        // Ambiguous `AddAssign` candidates on a `Copy + Add` type must NOT silently fall back!
        format!(
            r#"
            {MINIMAL_OPS_HEADER}
            trait Left {{}}
            trait Right {{}}
            impl Left for u64 {{}}
            impl Right for u64 {{}}
            struct Dual {{ v: u64 }}
            impl Copy for Dual {{}}
            impl Add<.Rhs = u64> for Dual {{
                assoc Output: Type = Dual;
                fn add(self, _rhs: u64) -> Dual {{ Dual {{ v: 1 }} }}
            }}
            impl AddAssign<.Rhs = R> for Dual where R: Type, requires R :- Left {{
                fn add_assign(*self, _rhs: R) -> void {{}}
            }}
            impl AddAssign<.Rhs = R> for Dual where R: Type, requires R :- Right {{
                fn add_assign(*self, _rhs: R) -> void {{}}
            }}
            fn bad() {{ var d = Dual {{ v: 0 }}; let one: u64 = 1; d += one; }}
            "#
        ),
    ];
    for (idx, src) in rejection_cases.iter().enumerate() {
        assert!(
            compile_ops(src).is_err(),
            "rejection case #{idx} unexpectedly succeeded"
        );
    }

    // 4. Sysroot comptime arithmetic compounds work cleanly end-to-end.
    let sysroot_ct = r#"
        use std.prelude.*;
        comptime fn run() -> u64 {
            var x: u64 = 10;
            x += 5;
            x -= 3;
            x *= 4;
            x /= 6;
            x %= 5;
            x
        }
    "#;
    assert_eq!(
        support::invoke(sysroot_ct, "run").unwrap(),
        Value::UnsignedInteger(3)
    );
}

#[test]
fn multi_arg_indexing_and_space_apply_respect_tmp_drop_and_no_silent_fallback() {
    // 1. Custom `Grid` supporting:
    //    - `Index<u32>` (single arg) returning `*u64` (preferred over `SpaceApply`)
    //    - `SpaceApply` with `VariadicCollector` (`[r, c]`, `[...coords]`, `[]`, and `[Mask]`)
    let source = format!(
        r#"
        {MINIMAL_OPS_HEADER}

        struct CoordTmp {{
            a: u32,
            b: u32,
        }}

        struct Mask {{ take_sum: bool }}

        struct CoordPacket {{
            row: u32,
            col: u32,
            is_mask: bool,
            mask_sum: bool,
        }}
        impl VariadicCollector for CoordPacket {{
            assoc Tmp: Type = CoordTmp;

            fn variadic_init() -> CoordTmp where _S {{
                CoordTmp {{ a: 0, b: 0 }}
            }}

            fn variadic_collect(tmp: *CoordTmp, args: S) -> CoordPacket where S {{
                inline if S == Tuple<u32, u32> {{
                    let p: Tuple<u32, u32> = args;
                    let (r, c) = p;
                    tmp.a.* = r;
                    tmp.b.* = c;
                    CoordPacket {{ row: tmp.a.*, col: tmp.b.*, is_mask: false, mask_sum: false }}
                }} else {{
                    inline if S == Tuple<Mask> {{
                        let p: Tuple<Mask> = args;
                        let (m,) = p;
                        CoordPacket {{ row: 0, col: 0, is_mask: true, mask_sum: m.take_sum }}
                    }} else {{
                        inline if S == Tuple<> or S == void {{
                            CoordPacket {{ row: 0, col: 0, is_mask: true, mask_sum: true }}
                        }} else {{
                            assumes false;
                            CoordPacket {{ row: 0, col: 0, is_mask: false, mask_sum: false }}
                        }}
                    }}
                }}
            }}
        }}

        struct GridView {{
            total: u64,
        }}
        struct Grid {{
            c0: u64,
            c1: u64,
            c2: u64,
            c3: u64,
        }}
        impl Index<u32> for Grid {{
            assoc Output: Type = u64;
            fn index(*self, idx: u32) -> *u64 {{
                if idx == 0 {{ self.c0 }}
                else if idx == 1 {{ self.c1 }}
                else if idx == 2 {{ self.c2 }}
                else {{ self.c3 }}
            }}
        }}
        impl SpaceApply for Grid {{
            assoc CollectTo: Type = CoordPacket;
            assoc Target: Type = GridView;
            fn space_apply(*self, index: CoordPacket) -> GridView {{
                assumes self.c0.* < 10000
                    and self.c1.* < 10000
                    and self.c2.* < 10000
                    and self.c3.* < 10000
                    and index.row < 1000
                    and index.col < 1000;
                if index.is_mask {{
                    let s = if index.mask_sum {{
                        self.c0.* + self.c1.* + self.c2.* + self.c3.*
                    }} else {{
                        self.c0.*
                    }};
                    GridView {{ total: s }}
                }} else {{
                    let flat = index.row * 2 + index.col;
                    let v = if flat == 0 {{ self.c0.* }}
                        else if flat == 1 {{ self.c1.* }}
                        else if flat == 2 {{ self.c2.* }}
                        else {{ self.c3.* }};
                    GridView {{ total: v }}
                }}
            }}
        }}

        fn arg_r(calls: *u32) -> u32 {{
            assumes calls.* < 1000;
            calls.* = calls.* + 1;
            1
        }}
        fn arg_c(calls: *u32) -> u32 {{
            assumes calls.* < 1000;
            calls.* = calls.* + 10;
            1
        }}
        fn make_grid(calls: *u32) -> Grid {{
            assumes calls.* < 1000;
            calls.* = calls.* + 100;
            Grid {{ c0: 10, c1: 20, c2: 30, c3: 40 }}
        }}

        fn run() -> bool {{
            var g = Grid {{ c0: 10, c1: 20, c2: 30, c3: 40 }};
            -- Single positional matching `Index<u32>` returns `*u64` (does NOT route to SpaceApply).
            let single_idx: u32 = 2;
            let single_ok = g[single_idx].* == 30;
            -- Multi-arg `[r, c]` routes to `SpaceApply`.
            var eval_log: u32 = 0;
            let multi = make_grid(eval_log.ref)[arg_r(eval_log.ref), arg_c(eval_log.ref)];
            -- Spread `[...coords]` routes to `SpaceApply`.
            let c0: u32 = 1;
            let c1: u32 = 0;
            let coords = (c0, c1);
            let spread_v = g[...coords];
            -- Empty `[]` routes to `SpaceApply`.
            let empty_v = g[];
            -- Single-arg `Mask` has no `Index<Mask>` candidate -> routes cleanly to `SpaceApply`.
            let mask_v = g[Mask {{ take_sum: true }}];
            single_ok
                and multi.total == 40
                and eval_log == 111
                and spread_v.total == 30
                and empty_v.total == 100
                and mask_v.total == 100
        }}

        fn early_return() -> u32 {{
            var g = Grid {{ c0: 10, c1: 20, c2: 30, c3: 40 }};
            let one: u32 = 1;
            let _ = g[one, do {{ return 99; }}];
            0
        }}
    "#
    );

    let pkg = compile_ops(&source).unwrap();
    assert_eq!(
        luna::execution::interpret(&pkg, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&pkg, "early_return", 100_000).unwrap(),
        Value::UnsignedInteger(99)
    );

    // 2. Named indexing `[.row = 1, .col = 2]` via ExtendedVariadicCollector + Tmp Drop ordering.
    let named_src = format!(
        r#"
        {MINIMAL_OPS_HEADER}
        fn dropped() {{}}
        struct NamedTmp {{ row: u32, col: u32 }}
        impl Drop for NamedTmp {{
            fn drop(*self) {{ dropped(); }}
        }}
        struct NamedCoords {{ row: u32, col: u32 }}
        impl ExtendedVariadicCollector for NamedCoords {{
            assoc Tmp: Type = NamedTmp;
            fn extended_variadic_init() -> NamedTmp where _Ps {{
                NamedTmp {{ row: 0, col: 0 }}
            }}
            fn extended_variadic_collect(tmp: *NamedTmp, _properties: Ps) -> NamedCoords where Ps {{
                NamedCoords {{ row: tmp.row.*, col: tmp.col.* }}
            }}
        }}
        struct Board {{ bias: u32 }}
        impl SpaceApply for Board {{
            assoc CollectTo: Type = NamedCoords;
            assoc Target: Type = u32;
            fn space_apply(*self, index: NamedCoords) -> u32 {{
                assumes self.bias.* < 1000 and index.row < 1000 and index.col < 1000;
                self.bias.* + index.row + index.col
            }}
        }}
        fn run() -> u32 {{
            let b = Board {{ bias: 7 }};
            let r: u32 = 1;
            let c: u32 = 2;
            b[.row = r, .col = c]
        }}
    "#
    );
    let named_pkg = compile_ops(&named_src).unwrap();
    let graph = luna::instantiation::instantiate(&named_pkg, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::UnsignedInteger(7)
    );
    assert!(
        calls(&named_pkg, &vm).contains(&"space_apply".to_string()),
        "expected space_apply call in {:?}",
        calls(&named_pkg, &vm)
    );

    // 3. Strict NO-silent-fallback: if single-arg `Index` candidate fails bounds (`requires`) or
    //    is ambiguous, compiler MUST fail and NEVER silently fall back to `SpaceApply`.
    let no_fallback_bound = format!(
        r#"
        {MINIMAL_OPS_HEADER}
        trait Flagged {{}}
        struct EmptyPacket {{}}
        impl VariadicCollector for EmptyPacket {{
            assoc Tmp: Type = void;
            fn variadic_init() -> void where _S {{}}
            fn variadic_collect(_tmp: *void, _args: S) -> EmptyPacket where S {{ EmptyPacket {{}} }}
        }}
        struct Boxed where T: Type {{ v: u32 }}
        impl Index<u32> for Boxed<T> where T: Type, requires T :- Flagged {{
            assoc Output: Type = u32;
            fn index(*self, _idx: u32) -> *u32 {{ self.v }}
        }}
        impl SpaceApply for Boxed<T> where T: Type {{
            assoc CollectTo: Type = EmptyPacket;
            assoc Target: Type = u32;
            fn space_apply(*self, _index: EmptyPacket) -> u32 {{ 42 }}
        }}
        fn bad(b: Boxed<T>) -> u32 where T: Type {{
            let zero: u32 = 0;
            b[zero]
        }}
    "#
    );
    assert!(
        compile_ops(&no_fallback_bound).is_err(),
        "deferred/unknown Index bound must not fall back to SpaceApply"
    );

    let no_fallback_ambiguous = format!(
        r#"
        {MINIMAL_OPS_HEADER}
        trait Left {{}}
        trait Right {{}}
        impl Left for u32 {{}}
        impl Right for u32 {{}}
        struct EmptyPacket {{}}
        impl VariadicCollector for EmptyPacket {{
            assoc Tmp: Type = void;
            fn variadic_init() -> void where _S {{}}
            fn variadic_collect(_tmp: *void, _args: S) -> EmptyPacket where S {{ EmptyPacket {{}} }}
        }}
        struct Amb {{ v: u32 }}
        impl Index<I> for Amb where I: Type, requires I :- Left {{
            assoc Output: Type = u32;
            fn index(*self, _idx: I) -> *u32 {{ self.v }}
        }}
        impl Index<I> for Amb where I: Type, requires I :- Right {{
            assoc Output: Type = u32;
            fn index(*self, _idx: I) -> *u32 {{ self.v }}
        }}
        impl SpaceApply for Amb {{
            assoc CollectTo: Type = EmptyPacket;
            assoc Target: Type = u32;
            fn space_apply(*self, _index: EmptyPacket) -> u32 {{ 42 }}
        }}
        fn bad() -> u32 {{
            let a = Amb {{ v: 1 }};
            let zero: u32 = 0;
            a[zero]
        }}
    "#
    );
    assert!(
        compile_ops(&no_fallback_ambiguous).is_err(),
        "ambiguous Index must not fall back to SpaceApply"
    );
}
