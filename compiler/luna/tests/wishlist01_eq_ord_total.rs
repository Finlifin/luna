//! Integration tests for Milestone 2A of Wishlist 01:
//! - `Eq` / `Ord` `*self` & `*Rhs` borrowing without consuming non-`Copy` operands
//! - Heterogeneous `Eq<.Rhs>` (custom types + `std.string.String == str`)
//! - `Vec` / `VecDeque` / `HashMap` / `Iterator.min` / `Iterator.max` borrowing non-`Copy` items
//! - `ComptimeEq<.Rhs>` / `ComptimeOrd` value semantics in comptime expressions & operators
//! - `TotalOrd` / `ComptimeTotalOrd` / `TotalFloat<F>` IEEE 754 `totalOrder` for `f16`/`bf16`/`f32`/`f64`
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
            "luna-wl01-eq-ord-{}-{}",
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
fn non_copy_eq_ord_and_containers_borrow_without_consuming() {
    let source = r#"
        use std.prelude.*;
        use std.collections.{HashMap, Vec, VecDeque};
        use std.mem.{AllocErr, Allocator, Layout, SystemAllocator};

        struct AffineScore {
            score: i32,
            drops: *u32,
        }

        impl Drop for AffineScore {
            fn drop(*self) {
                assumes self.drops.*.* < 1000;
                self.drops.*.* = self.drops.*.* + 1;
            }
        }

        impl Eq for AffineScore {
            fn eq(*self, other: *Self) -> bool {
                self.score.* == other.score.*
            }
        }

        impl Ord for AffineScore {
            fn cmp(*self, other: *Self) -> Order {
                if self.score.* < other.score.* {
                    .less
                } else if self.score.* > other.score.* {
                    .greater
                } else {
                    .equal
                }
            }
        }

        struct TrackedItem {
            score: i32,
            drops_addr: usize,
        }

        impl Drop for TrackedItem {
            fn drop(*self) {
                unsafe {
                    let count = builtin.mem.raw_read<u32>(self.drops_addr.*);
                    assumes count < 1000;
                    builtin.mem.raw_write<u32>(self.drops_addr.*, count + 1);
                }
            }
        }

        impl Eq for TrackedItem {
            fn eq(*self, other: *Self) -> bool {
                self.score.* == other.score.*
            }
        }

        impl Ord for TrackedItem {
            fn cmp(*self, other: *Self) -> Order {
                if self.score.* < other.score.* {
                    .less
                } else if self.score.* > other.score.* {
                    .greater
                } else {
                    .equal
                }
            }
        }

        fn read_u32(addr: usize) -> u32 {
            unsafe { builtin.mem.raw_read<u32>(addr) }
        }

        fn reset_u32(addr: usize) {
            unsafe {
                builtin.mem.raw_write<u32>(addr, 0);
            }
        }

        struct TrackedKey {
            id: u64,
            clones: *u32,
        }

        impl Clone for TrackedKey {
            fn clone(*self) -> Self {
                assumes self.clones.*.* < 1000;
                self.clones.*.* = self.clones.*.* + 1;
                TrackedKey {
                    id: self.id.*,
                    clones: self.clones.*,
                }
            }
        }

        impl Eq for TrackedKey {
            fn eq(*self, other: *Self) -> bool {
                self.id.* == other.id.*
            }
        }

        impl Hash for TrackedKey {
            fn hash(*self, state: *H) where H: Type, requires H :- Hasher {
                self.id.as(Hash).hash(state);
            }
        }

        fn exercise() -> !AllocErr bool {
            -- 1. Direct == / != / < / <= / > / >= on affine Drop structs borrow without consuming.
            var drops: u32 = 0;
            let direct_ok = do {
                let a = AffineScore { score: 10, drops: drops.ref };
                let b = AffineScore { score: 20, drops: drops.ref };
                let c = AffineScore { score: 10, drops: drops.ref };
                let ops_ok = (a == c) and not (a != c) and (a != b)
                    and (a < b) and (a <= c) and (b > a) and (b >= b);
                let still_alive = drops == 0;
                ops_ok and still_alive and (a.score == 10) and (b.score == 20) and (c.score == 10)
            };
            if not direct_ok or drops != 3 {
                return false;
            }

            let counter_layout = Layout.of<u32>() match {
                v? => v,
                null => return false,
            };
            let vec_block = SystemAllocator.GLOBAL.ref.as(Allocator).allocate_zeroed(counter_layout)!;
            let vec_drops_addr = vec_block.address();
            let dq_block = SystemAllocator.GLOBAL.ref.as(Allocator).allocate_zeroed(counter_layout)!;
            let dq_drops_addr = dq_block.address();

            -- 2. Vec<TrackedItem> == borrows elements and handles self-aliasing v == v.
            reset_u32(vec_drops_addr);
            let vec_ok = do {
                var v1 = Vec<TrackedItem>.new();
                v1.ref.push(TrackedItem { score: 5, drops_addr: vec_drops_addr })!;
                v1.ref.push(TrackedItem { score: 15, drops_addr: vec_drops_addr })!;

                var v2 = Vec<TrackedItem>.new();
                v2.ref.push(TrackedItem { score: 5, drops_addr: vec_drops_addr })!;
                v2.ref.push(TrackedItem { score: 15, drops_addr: vec_drops_addr })!;

                var v3 = Vec<TrackedItem>.new();
                v3.ref.push(TrackedItem { score: 5, drops_addr: vec_drops_addr })!;
                v3.ref.push(TrackedItem { score: 99, drops_addr: vec_drops_addr })!;

                let self_eq = v1 == v1;
                let pair_eq = v1 == v2;
                let pair_ne = v1 != v3;
                let len_after = v1.ref.len() == 2 and v2.ref.len() == 2 and v3.ref.len() == 2;
                let none_dropped_yet = read_u32(vec_drops_addr) == 0;
                self_eq and pair_eq and pair_ne and len_after and none_dropped_yet
            };
            let vec_drop_count = read_u32(vec_drops_addr);
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(vec_block);
            }
            if not vec_ok or vec_drop_count != 6 {
                unsafe {
                    SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(dq_block);
                }
                return false;
            }

            -- 3. VecDeque<TrackedItem> == borrows elements across wrapped ring and handles dq == dq.
            reset_u32(dq_drops_addr);
            let dq_ok = do {
                var d1 = VecDeque<TrackedItem>.with_capacity(4)!;
                d1.ref.push_back(TrackedItem { score: 2, drops_addr: dq_drops_addr })!;
                d1.ref.push_front(TrackedItem { score: 1, drops_addr: dq_drops_addr })!;
                d1.ref.push_back(TrackedItem { score: 3, drops_addr: dq_drops_addr })!;

                var d2 = VecDeque<TrackedItem>.new();
                d2.ref.push_back(TrackedItem { score: 1, drops_addr: dq_drops_addr })!;
                d2.ref.push_back(TrackedItem { score: 2, drops_addr: dq_drops_addr })!;
                d2.ref.push_back(TrackedItem { score: 3, drops_addr: dq_drops_addr })!;

                let self_eq = d1 == d1;
                let eq_ok = d1 == d2;
                let none_dropped = read_u32(dq_drops_addr) == 0;
                self_eq and eq_ok and none_dropped and d1.ref.len() == 3
            };
            let dq_drop_count = read_u32(dq_drops_addr);
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(dq_block);
            }
            if not dq_ok or dq_drop_count != 6 {
                return false;
            }

            -- 4. String == String and String < String do not consume either string.
            let s1 = String.from_str("alpha")!;
            let s2 = String.from_str("alpha")!;
            let s3 = String.from_str("beta")!;
            if not (s1 == s2) or not (s1 != s3) or not (s1 < s3) or not (s3 > s1) {
                return false;
            }
            if s1.ref.len() != 5 or s2.ref.len() != 5 or s3.ref.len() != 4 {
                return false;
            }

            -- 5. HashMap probing and hashing borrow keys without calling Clone.
            var map = HashMap<TrackedKey, u32>.new();
            var clone_count: u32 = 0;
            map.ref.insert(TrackedKey { id: 1, clones: clone_count.ref }, 100)!;
            map.ref.insert(TrackedKey { id: 2, clones: clone_count.ref }, 200)!;
            let clones_before_probe = clone_count;
            let hit = map.ref.get(TrackedKey { id: 1, clones: clone_count.ref }) match {
                v? => v == 100,
                null => false,
            };
            let contains_hit = map.ref.contains_key(TrackedKey { id: 2, clones: clone_count.ref });
            assumes clone_count >= clones_before_probe;
            let extra_clones = clone_count - clones_before_probe;
            if not hit or not contains_hit or extra_clones != 0 {
                return false;
            }

            -- 6. Iterator.min / Iterator.max work on non-Copy Ord items (`String` implements Drop + Ord, not Copy).
            let min_max_ok = do {
                var items = Vec<String>.new();
                items.ref.push(String.from_str("charlie")!)!;
                items.ref.push(String.from_str("alpha")!)!;
                items.ref.push(String.from_str("bravo")!)!;
                let smallest = items.into_iter().as(Iterator).min() match {
                    item? => item == "alpha",
                    null => false,
                };

                var items2 = Vec<String>.new();
                items2.ref.push(String.from_str("charlie")!)!;
                items2.ref.push(String.from_str("alpha")!)!;
                items2.ref.push(String.from_str("bravo")!)!;
                let largest = items2.into_iter().as(Iterator).max() match {
                    item? => item == "charlie",
                    null => false,
                };
                smallest and largest
            };
            min_max_ok
        }

        fn run() -> bool {
            exercise() match {
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
fn heterogeneous_eq_and_comptime_eq_ord_work() {
    let source = r#"
        use std.prelude.*;
        use std.mem.AllocErr;

        struct Meter {
            mm: i32,
        }

        struct Centimeter {
            cm: i32,
        }

        impl Eq for Meter {
            fn eq(*self, other: *Self) -> bool {
                self.mm.* == other.mm.*
            }
        }

        impl Eq<.Rhs = Centimeter> for Meter {
            fn eq(*self, other: *Centimeter) -> bool {
                assumes other.cm.* >= -100000 and other.cm.* <= 100000;
                self.mm.* == other.cm.* * 10
            }
        }

        comptime impl ComptimeEq for Meter {
            comptime fn eq(self, other: Self) -> bool {
                self.mm == other.mm
            }
        }

        comptime impl ComptimeEq<.Rhs = Centimeter> for Meter {
            comptime fn eq(self, other: Centimeter) -> bool {
                assumes other.cm >= -100000 and other.cm <= 100000;
                self.mm == other.cm * 10
            }
        }

        struct Rank {
            val: i32,
        }

        comptime impl ComptimeEq for Rank {
            comptime fn eq(self, other: Self) -> bool {
                self.val == other.val
            }
        }

        comptime impl ComptimeOrd for Rank {
            comptime fn cmp(self, other: Self) -> Order {
                if self.val < other.val {
                    .less
                } else if self.val > other.val {
                    .greater
                } else {
                    .equal
                }
            }
        }

        comptime fn ct_check() -> bool {
            let m1 = Meter { mm: 250 };
            let m2 = Meter { mm: 250 };
            let cm_match = Centimeter { cm: 25 };
            let cm_diff = Centimeter { cm: 26 };
            let het_ok = (m1 == m2) and (m1 == cm_match) and (m1 != cm_diff);

            let r1 = Rank { val: 1 };
            let r2 = Rank { val: 2 };
            let ord_ok = (r1 == r1) and (r1 != r2) and (r1 < r2) and (r1 <= r1)
                and (r2 > r1) and (r2 >= r2) and not (r2 < r1);
            het_ok and ord_ok
        }

        fn rt_check() -> !AllocErr bool {
            let m = Meter { mm: 1200 };
            let cm1 = Centimeter { cm: 120 };
            let cm2 = Centimeter { cm: 99 };
            if not (m == cm1) or not (m != cm2) {
                return false;
            }
            let s = String.from_str("flurry")!;
            (s == "flurry") and (s != "rust") and s.ref.len() == 6
        }

        fn run() -> bool {
            rt_check() match {
                ok! => ok,
                error _ => false,
            }
        }
    "#;
    assert_eq!(
        support::invoke(source, "ct_check").unwrap(),
        Value::Bool(true)
    );
    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn total_ord_and_total_float_order_nan_and_signed_zero_consistently() {
    let source = r#"
        use builtin.math.{TotalFloat, TotalOrd};

        fn check_f64_order() -> bool {
            let neg_nan = TotalFloat<f64>.new(-f64.NaN);
            let neg_inf = TotalFloat<f64>.new(-f64.Infinity);
            let neg_one = TotalFloat<f64>.new(-1.0);
            let neg_zero = TotalFloat<f64>.new(-0.0);
            let pos_zero = TotalFloat<f64>.new(0.0);
            let pos_one = TotalFloat<f64>.new(1.0);
            let pos_inf = TotalFloat<f64>.new(f64.Infinity);
            let pos_nan = TotalFloat<f64>.new(f64.NaN);

            let nz: f64 = -0.0;
            let pz: f64 = 0.0;
            let pinf: f64 = f64.Infinity;
            let pnan: f64 = f64.NaN;
            let nnan: f64 = -f64.NaN;

            (neg_nan < neg_inf)
                and (neg_inf < neg_one)
                and (neg_one < neg_zero)
                and (neg_zero < pos_zero)
                and (neg_zero != pos_zero)
                and (pos_zero < pos_one)
                and (pos_one < pos_inf)
                and (pos_inf < pos_nan)
                and (pos_nan == pos_nan)
                and (neg_nan == neg_nan)
                and (nz.as(TotalOrd).total_lt(pz.ref))
                and (pnan.as(TotalOrd).total_gt(pinf.ref))
                and (nnan.as(TotalOrd).total_cmp(nnan.ref) matches .equal)
        }

        fn check_f32_f16_bf16() -> bool {
            let f32_neg_zero: f32 = -0.0;
            let f32_pos_zero: f32 = 0.0;
            let f32_neg_nan: f32 = -f32.NaN;
            let f32_pos_nan: f32 = f32.NaN;

            let f16_neg_zero: f16 = -0.0;
            let f16_pos_zero: f16 = 0.0;
            let f16_neg_nan: f16 = -f16.NaN;
            let f16_pos_nan: f16 = f16.NaN;

            let bf16_neg_zero: bf16 = -0.0;
            let bf16_pos_zero: bf16 = 0.0;
            let bf16_neg_nan: bf16 = -bf16.NaN;
            let bf16_pos_nan: bf16 = bf16.NaN;

            (TotalFloat<f32>.new(f32_neg_nan) < TotalFloat<f32>.new(f32_neg_zero))
                and (TotalFloat<f32>.new(f32_neg_zero) < TotalFloat<f32>.new(f32_pos_zero))
                and (TotalFloat<f32>.new(f32_pos_zero) < TotalFloat<f32>.new(f32_pos_nan))
                and (TotalFloat<f16>.new(f16_neg_nan) < TotalFloat<f16>.new(f16_neg_zero))
                and (TotalFloat<f16>.new(f16_neg_zero) < TotalFloat<f16>.new(f16_pos_zero))
                and (TotalFloat<f16>.new(f16_pos_zero) < TotalFloat<f16>.new(f16_pos_nan))
                and (TotalFloat<bf16>.new(bf16_neg_nan) < TotalFloat<bf16>.new(bf16_neg_zero))
                and (TotalFloat<bf16>.new(bf16_neg_zero) < TotalFloat<bf16>.new(bf16_pos_zero))
                and (TotalFloat<bf16>.new(bf16_pos_zero) < TotalFloat<bf16>.new(bf16_pos_nan))
        }

        fn generic_eq_ord(a: T, b: T) -> bool where T :- Eq + Ord + Hash {
            var hasher = std.hash.DefaultHasher.new();
            a.ref.as(Hash).hash(hasher.ref);
            let _ = hasher.ref.finish();
            a < b and a != b
        }

        fn run() -> bool {
            let ieee_nan = f64.NaN;
            let ieee_ok = not (ieee_nan == ieee_nan) and (ieee_nan != ieee_nan) and (-0.0 == 0.0);
            let generic_ok = generic_eq_ord(TotalFloat<f64>.new(-0.0), TotalFloat<f64>.new(0.0));
            ieee_ok and generic_ok and check_f64_order() and check_f32_f16_bf16()
        }

        comptime fn ct_check() -> bool {
            let neg_nan = TotalFloat<f64>.comptime_new(-f64.NaN);
            let neg_inf = TotalFloat<f64>.comptime_new(-f64.Infinity);
            let neg_zero = TotalFloat<f64>.comptime_new(-0.0);
            let pos_zero = TotalFloat<f64>.comptime_new(0.0);
            let pos_inf = TotalFloat<f64>.comptime_new(f64.Infinity);
            let pos_nan = TotalFloat<f64>.comptime_new(f64.NaN);

            let nz: f64 = -0.0;
            let pz: f64 = 0.0;
            let pinf: f64 = f64.Infinity;
            let pnan: f64 = f64.NaN;

            let ct_f64 = (neg_nan < neg_inf)
                and (neg_inf < neg_zero)
                and (neg_zero < pos_zero)
                and (neg_zero != pos_zero)
                and (pos_zero < pos_inf)
                and (pos_inf < pos_nan)
                and (pos_nan == pos_nan)
                and nz.as(ComptimeTotalOrd).total_lt(pz)
                and pnan.as(ComptimeTotalOrd).total_gt(pinf);

            let ct_f32 = (TotalFloat<f32>.comptime_new(-f32.NaN) < TotalFloat<f32>.comptime_new(-0.0))
                and (TotalFloat<f32>.comptime_new(-0.0) < TotalFloat<f32>.comptime_new(0.0))
                and (TotalFloat<f32>.comptime_new(0.0) < TotalFloat<f32>.comptime_new(f32.NaN));

            let ct_f16 = (TotalFloat<f16>.comptime_new(-0.0) < TotalFloat<f16>.comptime_new(0.0))
                and (TotalFloat<bf16>.comptime_new(-0.0) < TotalFloat<bf16>.comptime_new(0.0));

            ct_f64 and ct_f32 and ct_f16
        }
    "#;
    assert_eq!(
        support::invoke(source, "ct_check").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::run(source), Value::Bool(true));

    // Raw floats must still NOT satisfy Ord.
    let bad = r#"
        fn require_ord(a: T) -> bool where T :- Ord { true }
        fn run() -> bool { require_ord(1.0) }
    "#;
    assert!(support::compile(bad).is_err());
}
