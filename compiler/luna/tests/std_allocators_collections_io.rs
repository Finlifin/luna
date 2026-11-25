//! End-to-end native execution tests for custom runtime-dependent allocators
//! (`FixedBufferAllocator`, `ArenaAllocator`), `std.collections` containers, and
//! `std.io` (`Reader`, `Write`, `Cursor`, `Stdin`, `BufReader`, `BufWriter`)
//! backed through real sysroot compilation.
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-std-aci-{}-{}",
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
fn fixed_buffer_allocator_backs_box_vec_string_and_enforces_oom() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, SystemAllocator};

        fn exercise_fba(fba: *FixedBufferAllocator) -> !AllocErr bool {
            do {
                let boxed = Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(99)!;
                var vec = Vec<u64, .A = FixedBufferAllocator, .allocator = fba>.with_capacity(4)!;
                vec.ref.push(10)!;
                vec.ref.push(20)!;
                vec.ref.push(30)!;
                var text = String<.A = FixedBufferAllocator, .allocator = fba>.from_str("hello-fba")!;
                text.ref.push_str("!")!;
                if boxed.get() != 99 or vec.ref.len() != 3 or not text.ref.equals_str("hello-fba!") {
                    return false;
                }
                if fba.live_allocations() != 3 or fba.used() == 0 {
                    return false;
                }
            }
            if fba.live_allocations() != 0 {
                return false;
            }
            -- Verify tail reclamation actually released bytes when String was last.
            let used_after_drop = fba.used();
            fba.reset();
            if fba.used() != 0 or fba.remaining() != fba.capacity() {
                return false;
            }

            -- Exhaust the buffer and confirm AllocErr.OutOfMemory fires cleanly.
            let half_ok = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.with_capacity(128)!;
            let oom_ok = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.with_capacity(200) match {
                _! => false,
                error err => err match {
                    .OutOfMemory => true,
                    .InvalidLayout => false,
                },
            };
            let zst_box = Box<void, .A = FixedBufferAllocator, .allocator = fba>.new(())!;
            let _ = zst_box;
            oom_ok and used_after_drop < fba.capacity() and fba.live_allocations() == 1
        }

        fn run() -> bool {
            let layout = Layout.new(256, 16) match {
                found? => found,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let ok = exercise_fba(fba.ref) match {
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
fn arena_allocator_composes_backing_allocator_and_drops_chunks() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{AllocErr, Allocator, ArenaAllocator, FixedBufferAllocator, Layout, SystemAllocator};

        fn exercise_arenas(fba: *FixedBufferAllocator) -> !AllocErr bool {
            var default_arena = ArenaAllocator.new();
            let d_vec = do {
                var v = Vec<u64, .A = ArenaAllocator, .allocator = default_arena.ref>.with_capacity(4)!;
                v.ref.push(7)!;
                v.ref.push(11)!;
                v.ref.len() == 2
                    and default_arena.ref.chunk_count() == 1
                    and default_arena.ref.total_allocated() >= 32
            };
            if not d_vec
                or default_arena.ref.chunk_count() != 1
                or default_arena.ref.total_allocated() != 0
            {
                return false;
            }
            default_arena.ref.reset();
            if default_arena.ref.chunk_count() != 0 or default_arena.ref.total_allocated() != 0 {
                return false;
            }

            let scoped_ok = do {
                var arena = ArenaAllocator<.Backing = FixedBufferAllocator, .backing = fba>.with_chunk_size(128);
                var s1 = String<.A = ArenaAllocator<.Backing = FixedBufferAllocator, .backing = fba>, .allocator = arena.ref>.from_str("chunk-one-")!;
                s1.ref.push_str("payload-exceeding-initial-chunk-capacity-0123456789abcdef")!;
                var s2 = String<.A = ArenaAllocator<.Backing = FixedBufferAllocator, .backing = fba>, .allocator = arena.ref>.from_str("chunk-two-")!;
                s2.ref.push_str("more-data-triggering-multiple-slabs-0123456789abcdef0123456789")!;
                s1.ref.equals_str("chunk-one-payload-exceeding-initial-chunk-capacity-0123456789abcdef")
                    and s2.ref.equals_str("chunk-two-more-data-triggering-multiple-slabs-0123456789abcdef0123456789")
                    and arena.ref.chunk_count() == 2
                    and fba.live_allocations() == 2
            };
            scoped_ok and fba.live_allocations() == 0
        }

        fn run() -> bool {
            let layout = Layout.new(4096, 16) match {
                found? => found,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let ok = exercise_arenas(fba.ref) match {
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
fn vec_deque_ring_wrap_custom_allocator_zst_and_early_break_drop() {
    let source = r#"
        use std.prelude.*;
        use std.collections.VecDeque;
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, SystemAllocator};

        fn expect_opt_u64(opt: ?u64, expected: u64) -> bool {
            opt match {
                v? => v == expected,
                null => false,
            }
        }

        fn exercise_deque(fba: *FixedBufferAllocator) -> !AllocErr bool {
            -- 1. Wrap-around ring buffer operations & growth across wrapped tail/head boundary.
            var dq = VecDeque<u64, .A = FixedBufferAllocator, .allocator = fba>.with_capacity(4)!;
            dq.ref.push_back(30)!;
            dq.ref.push_back(40)!;
            dq.ref.push_front(20)!;
            dq.ref.push_front(10)!;
            -- Ring holds [10, 20, 30, 40] wrapped around cap 4; push_front(5) forces linearized growth.
            dq.ref.push_front(5)!;
            dq.ref.push_back(50)!;
            if dq.ref.len() != 6
                or not expect_opt_u64(dq.ref.front(), 5)
                or not expect_opt_u64(dq.ref.back(), 50)
                or not expect_opt_u64(dq.ref.get(2), 20)
            {
                return false;
            }
            if not dq.ref.swap(0, 5) or not dq.ref.set(1, 15) {
                return false;
            }
            -- Expected order: [50, 15, 20, 30, 40, 5]
            var cloned = dq.ref.try_clone()!;
            let cloned_eq = dq.ref.as(Eq).eq(cloned.ref);
            if not cloned_eq {
                return false;
            }
            if not expect_opt_u64(dq.ref.pop_front(), 50)
                or not expect_opt_u64(dq.ref.pop_back(), 5)
                or not expect_opt_u64(dq.ref.pop_front(), 15)
                or dq.ref.len() != 3
            {
                return false;
            }
            cloned.ref.clear();
            if not cloned.ref.is_empty() or cloned.ref.capacity() == 0 {
                return false;
            }

            -- 2. Zero-sized elements require no OS allocation yet track length.
            var zv = VecDeque<void, .A = FixedBufferAllocator, .allocator = fba>.with_capacity(8)!;
            let live_before_zst = fba.live_allocations();
            zv.ref.push_front(())!;
            zv.ref.push_back(())!;
            zv.ref.push_front(())!;
            if zv.ref.len() != 3 or fba.live_allocations() != live_before_zst {
                return false;
            }
            if zv.ref.pop_back().is_none() or zv.ref.len() != 2 {
                return false;
            }

            -- 3. Affine Box<u64, ...> elements + early-break Iterator.take(2) drop all boxes once.
            let boxes_ok = do {
                let start_live = fba.live_allocations();
                var bq = VecDeque<
                    Box<u64, .A = FixedBufferAllocator, .allocator = fba>,
                    .A = FixedBufferAllocator,
                    .allocator = fba,
                >.new();
                bq.ref.push_back(Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(100)!)!;
                bq.ref.push_front(Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(200)!)!;
                bq.ref.push_back(Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(300)!)!;
                bq.ref.push_front(Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(400)!)!;
                -- Order: [400, 200, 100, 300]
                var sum: u64 = 0;
                let sum_ptr = sum.ref;
                bq.as(IntoIterator).into_iter().as(Iterator).take(2).as(Iterator).foreach(|item| {
                    let v = item.get();
                    if v < 10000 and sum_ptr.* < 1000000 {
                        sum_ptr.* = sum_ptr.* + v;
                    }
                });
                sum == 600 and fba.live_allocations() == start_live
            };

            -- 4. FixedBufferAllocator exhaustion preserves existing deque state cleanly.
            var small_dq = VecDeque<u64, .A = FixedBufferAllocator, .allocator = fba>.new();
            small_dq.ref.push_back(77)!;
            let oom_ok = small_dq.ref.reserve(256) match {
                _! => false,
                error err => err match {
                    .OutOfMemory => true,
                    .InvalidLayout => false,
                },
            };
            oom_ok and expect_opt_u64(small_dq.ref.front(), 77) and boxes_ok
        }

        fn run() -> bool {
            let layout = Layout.new(2048, 16) match {
                found? => found,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let ok = exercise_deque(fba.ref) match {
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
fn hash_map_probing_tombstones_rehash_affine_values_and_custom_allocator() {
    let source = r#"
        use std.prelude.*;
        use std.collections.HashMap;
        use std.mem.{AllocErr, Allocator, ArenaAllocator, FixedBufferAllocator, Layout, SystemAllocator};

        fn expect_opt_u64(opt: ?u64, expected: u64) -> bool {
            opt match {
                v? => v == expected,
                null => false,
            }
        }

        fn exercise_maps(fba: *FixedBufferAllocator) -> !AllocErr bool {
            -- 1. Probing, overwrite, tombstone reuse, and multiple rehashes with ArenaAllocator.
            var arena = ArenaAllocator<.Backing = FixedBufferAllocator, .backing = fba>.with_chunk_size(512);
            var map = HashMap<
                u64,
                u64,
                .A = ArenaAllocator<.Backing = FixedBufferAllocator, .backing = fba>,
                .allocator = arena.ref,
            >.new();
            var i: u64 = 0;
            while i < 40 {
                if map.ref.insert(i, i * 10)!.is_some() {
                    return false;
                }
                i = i + 1;
            }
            if map.ref.len() != 40 or not expect_opt_u64(map.ref.get(26), 260) {
                return false;
            }
            if not expect_opt_u64(map.ref.insert(26, 999)!, 260) or map.ref.len() != 40 {
                return false;
            }
            -- Delete odd keys to build tombstones, verify remaining keys, then re-insert to clear/reuse.
            var k: u64 = 1;
            while k < 40 {
                if not expect_opt_u64(map.ref.remove(k), k * 10) {
                    return false;
                }
                k = k + 2;
            }
            if map.ref.len() != 20 or map.ref.contains_key(15) {
                return false;
            }
            var cloned = map.ref.try_clone()!;
            if not expect_opt_u64(cloned.ref.get(26), 999) or cloned.ref.len() != 20 {
                return false;
            }
            var r: u64 = 100;
            while r < 140 {
                let _ = map.ref.insert(r, r + 1)!;
                r = r + 1;
            }
            if map.ref.len() != 60
                or not expect_opt_u64(map.ref.get(26), 999)
                or not expect_opt_u64(map.ref.get(135), 136)
            {
                return false;
            }

            -- 2. Zero-sized value HashMap<u64, void> works cleanly without non-empty value storage.
            var zmap = HashMap<u64, void, .A = FixedBufferAllocator, .allocator = fba>.new();
            if zmap.ref.insert(1, ())!.is_some() or zmap.ref.insert(1, ())!.is_none() or not zmap.ref.contains_key(1) {
                return false;
            }
            if zmap.ref.remove(1).is_none() or zmap.ref.len() != 0 {
                return false;
            }

            -- 3. Strictly affine (Once, non-Copy) Box values drop on overwrite, remove, clear, and early break.
            let affine_ok = do {
                let base_live = fba.live_allocations();
                var amap = HashMap<
                    u64,
                    Box<u64, .A = FixedBufferAllocator, .allocator = fba>,
                    .A = FixedBufferAllocator,
                    .allocator = fba,
                >.with_capacity(6)!;
                let _ = amap.ref.insert(1, Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(11)!)!;
                let _ = amap.ref.insert(2, Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(22)!)!;
                let _ = amap.ref.insert(3, Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(33)!)!;
                let _ = amap.ref.insert(4, Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(44)!)!;
                -- Replacing key 2 returns old Box, which drops immediately on scope exit.
                let old_sum = do {
                    let old_box = amap.ref.insert(2, Box<u64, .A = FixedBufferAllocator, .allocator = fba>.new(220)!)! match {
                        b? => b,
                        null => return false,
                    };
                    old_box.get() == 22
                };
                if not old_sum {
                    return false;
                }
                let removed_ok = do {
                    let rem = amap.ref.remove(4) match {
                        b? => b,
                        null => return false,
                    };
                    rem.get() == 44
                };
                if not removed_ok or amap.ref.len() != 3 {
                    return false;
                }
                var sum: u64 = 0;
                let sum_ptr = sum.ref;
                amap.as(IntoIterator).into_iter().as(Iterator).take(2).as(Iterator).foreach(|pair| {
                    let (_, boxed) = pair;
                    let val = boxed.get();
                    if val < 10000 and sum_ptr.* < 1000000 {
                        sum_ptr.* = sum_ptr.* + val;
                    }
                });
                fba.live_allocations() == base_live and sum > 0
            };

            -- 4. FixedBufferAllocator exhaustion leaves existing HashMap entries untouched.
            var oom_map = HashMap<u64, u64, .A = FixedBufferAllocator, .allocator = fba>.with_capacity(4)!;
            let _ = oom_map.ref.insert(42, 777)!;
            let oom_ok = oom_map.ref.reserve(1024) match {
                _! => false,
                error err => err match {
                    .OutOfMemory => true,
                    .InvalidLayout => false,
                },
            };
            affine_ok and oom_ok and expect_opt_u64(oom_map.ref.get(42), 777)
        }

        fn run() -> bool {
            let layout = Layout.new(16384, 16) match {
                found? => found,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let ok = exercise_maps(fba.ref) match {
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
fn hash_set_operations_cross_allocator_subset_disjoint_and_iteration() {
    let source = r#"
        use std.prelude.*;
        use std.collections.HashSet;
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, SystemAllocator};

        fn exercise_sets(fba: *FixedBufferAllocator) -> !AllocErr bool {
            var sys_set = HashSet<u64>.new();
            let ins10 = sys_set.ref.insert(10)!;
            let ins20 = sys_set.ref.insert(20)!;
            let ins20_dup = sys_set.ref.insert(20)!;
            let ins30 = sys_set.ref.insert(30)!;
            if not ins10 or not ins20 or ins20_dup or not ins30 {
                return false;
            }
            if sys_set.ref.len() != 3
                or not sys_set.ref.contains(20)
                or sys_set.ref.contains(99)
            {
                return false;
            }

            var fba_set = HashSet<u64, .A = FixedBufferAllocator, .allocator = fba>.with_capacity(4)!;
            let _ = fba_set.ref.insert(10)!;
            let _ = fba_set.ref.insert(30)!;
            if not fba_set.ref.is_subset(sys_set.ref) or sys_set.ref.is_subset(fba_set.ref) {
                return false;
            }
            if fba_set.ref.is_disjoint(sys_set.ref) {
                return false;
            }

            var other_set = HashSet<u64, .A = FixedBufferAllocator, .allocator = fba>.new();
            let _ = other_set.ref.insert(100)!;
            let _ = other_set.ref.insert(200)!;
            if not fba_set.ref.is_disjoint(other_set.ref) {
                return false;
            }

            var cloned = fba_set.ref.try_clone()!;
            if not cloned.ref.remove(10) or cloned.ref.remove(10) or cloned.ref.len() != 1 {
                return false;
            }

            var early_count: u64 = 0;
            let early_ptr = early_count.ref;
            fba_set.ref.try_clone()!.as(IntoIterator).into_iter().as(Iterator).take(1).as(Iterator).foreach(|elem| {
                let _ = elem;
                if early_ptr.* < 100 {
                    early_ptr.* = early_ptr.* + 1;
                }
            });

            var sum: u64 = 0;
            let sum_ptr = sum.ref;
            sys_set.as(IntoIterator).into_iter().as(Iterator).foreach(|elem| {
                if elem < 10000 and sum_ptr.* < 1000000 {
                    sum_ptr.* = sum_ptr.* + elem;
                }
            });
            sum == 60 and early_count == 1 and cloned.ref.contains(30)
        }

        fn run() -> bool {
            let layout = Layout.new(2048, 16) match {
                found? => found,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let ok = exercise_sets(fba.ref) match {
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
fn buf_writer_and_buf_reader_roundtrip_file_and_custom_allocator() {
    let source = r#"
        use std.prelude.*;
        use std.fs.{File, remove_file};
        use std.io.{BufReader, BufWriter, IoErr, Reader};
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, SystemAllocator};
        use std.string.Utf8Err;

        fn exercise_buffered_file_io(
            fba: *FixedBufferAllocator,
        ) -> ![IoErr, AllocErr, Utf8Err] bool {
            -- 1. Write via small BufWriter<File> using write_fmt, write_byte, bypass write,
            --    explicit flush, and a trailing write left solely for Drop auto-flush.
            do {
                let file = File.create<"stage1c_roundtrip.txt">()!;
                var bw = BufWriter<
                    File,
                    .A = FixedBufferAllocator,
                    .allocator = fba,
                >.with_capacity(11, file)!;
                write_fmt(bw.ref, "head={}!", 42)!;
                bw.ref.write_byte(10)!;
                var mid = String<
                    .A = FixedBufferAllocator,
                    .allocator = fba,
                >.from_str("bypass-payload-line\n")!;
                bw.ref.write_string(mid.ref)!;
                bw.ref.flush()!;
                if bw.ref.buffered_len() != 0 {
                    return false;
                }
                var tail = String<
                    .A = FixedBufferAllocator,
                    .allocator = fba,
                >.from_str("tail-drop\n")!;
                bw.ref.write_string(tail.ref)!;
                if bw.ref.buffered_len() != 10 {
                    return false;
                }
            }

            -- 2. Read back with a small BufReader<File> (cap = 7) using peek_byte,
            --    read_byte, read_line, read_until, read_exact, UnexpectedEof, and into_inner.
            let file = File.open<"stage1c_roundtrip.txt">()!;
            var br = BufReader<
                File,
                .A = FixedBufferAllocator,
                .allocator = fba,
            >.with_capacity(7, file)!;
            let peeked = br.ref.peek_byte()! match {
                b? => b,
                null => return false,
            };
            let first = br.ref.read_byte()! match {
                b? => b,
                null => return false,
            };
            if peeked != 104 or first != 104 {
                return false;
            }

            var line1 = String<.A = FixedBufferAllocator, .allocator = fba>.new();
            let n1 = br.ref.read_line(line1.ref)!;
            if n1 != 8 or not line1.ref.equals_str("ead=42!\n") {
                return false;
            }

            var line2 = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            let n2 = br.ref.read_until(10, line2.ref)!;
            if n2 != 20 or line2.ref.len() != 20 {
                return false;
            }
            let line2_str = String<.A = FixedBufferAllocator, .allocator = fba>.from_utf8(line2)!;
            if not line2_str.ref.equals_str("bypass-payload-line\n") {
                return false;
            }

            var tail_bytes = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            br.ref.as(Reader).read_exact(tail_bytes.ref, 10)!;
            let tail_str = String<.A = FixedBufferAllocator, .allocator = fba>.from_utf8(tail_bytes)!;
            if not tail_str.ref.equals_str("tail-drop\n") {
                return false;
            }

            var eof_probe = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            let got_eof = br.ref.as(Reader).read_exact(eof_probe.ref, 4) match {
                _! => false,
                error IoErr.UnexpectedEof => true,
                error _ => false,
            };
            let inner_file = br.into_inner();
            let _ = inner_file.ref.close();
            got_eof
        }

        fn run() -> bool {
            let layout = Layout.new(16384, 16) match {
                found? => found,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let ok = exercise_buffered_file_io(fba.ref) match {
                passed! => passed,
                error _ => false,
            };
            let _ = remove_file("stage1c_roundtrip.txt");
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
fn cursor_reader_buffered_utf8_errors_and_vec_buf_writer() {
    let source = r#"
        use std.prelude.*;
        use std.io.{BufReader, BufWriter, Cursor, IoErr, Reader, Stdin, stdin};
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, SystemAllocator};
        use std.string.Utf8Err;

        fn push_str_bytes(
            dst: *Vec<u8, .A = FixedBufferAllocator, .allocator = fba>,
            text: str,
        ) -> !AllocErr void
        where
            quote fba: lift *FixedBufferAllocator,
        {
            let len = builtin.compiler.str_byte_length(text);
            var i: u64 = 0;
            while i < len {
                let b = builtin.compiler.str_byte(text, i) match {
                    found? => found,
                    null => break,
                };
                dst.push(b)!;
                i = i + 1;
            }
        }

        fn exercise_memory_io(
            fba: *FixedBufferAllocator,
        ) -> ![IoErr, AllocErr, Utf8Err] bool {
            let _stdin_handle: Stdin = stdin();

            -- 1. BufWriter<Vec<u8, ...>> with assoc Err = AllocErr and scoped with_inner check.
            var sink = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            var bw = BufWriter<
                Vec<u8, .A = FixedBufferAllocator, .allocator = fba>,
                .A = FixedBufferAllocator,
                .allocator = fba,
            >.with_capacity(8, sink)!;
            write_fmt(bw.ref, "v={}", 7)!;
            bw.ref.write_byte(44)!;
            if bw.ref.buffered_len() != 4 {
                return false;
            }
            var big = String<
                .A = FixedBufferAllocator,
                .allocator = fba,
            >.from_str("0123456789")!;
            bw.ref.write_string(big.ref)!;
            bw.ref.flush()!;
            let vec_ok = bw.ref.with_inner(|inner| -> bool {
                if inner.len() != 14 {
                    return false;
                }
                let b0 = inner.get(0) match {
                    v? => v,
                    null => return false,
                };
                let b3 = inner.get(3) match {
                    v? => v,
                    null => return false,
                };
                let b13 = inner.get(13) match {
                    v? => v,
                    null => return false,
                };
                b0 == 118 and b3 == 44 and b13 == 57
            });
            if not vec_ok {
                return false;
            }

            -- 2. Direct Cursor Reader operations: read_byte, read_into, read_exact, UnexpectedEof,
            --    set_position, read_to_end, and read_to_string.
            var raw = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            push_str_bytes(raw.ref, "alpha-beta")!;
            var cursor = Cursor<.A = FixedBufferAllocator, .allocator = fba>.new(raw);
            let b_first = cursor.ref.as(Reader).read_byte()! match {
                v? => v,
                null => return false,
            };
            if b_first != 97 or cursor.ref.position() != 1 or cursor.ref.remaining() != 9 {
                return false;
            }
            var part = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            let got_into = cursor.ref.as(Reader).read_into(part.ref, 4)!;
            if got_into != 4 or cursor.ref.remaining() != 5 {
                return false;
            }
            cursor.ref.as(Reader).read_exact(part.ref, 5)!;
            if not cursor.ref.is_empty() {
                return false;
            }
            let eof_hit = cursor.ref.as(Reader).read_exact(part.ref, 1) match {
                _! => false,
                error IoErr.UnexpectedEof => true,
                error _ => false,
            };
            if not eof_hit {
                return false;
            }
            let part_text = String<.A = FixedBufferAllocator, .allocator = fba>.from_utf8(part)!;
            if not part_text.ref.equals_str("lpha-beta") {
                return false;
            }

            cursor.ref.set_position(0);
            var end_buf = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            let total_end = cursor.ref.as(Reader).read_to_end(end_buf.ref)!;
            if total_end != 10 or end_buf.ref.len() != 10 {
                return false;
            }
            cursor.ref.set_position(0);
            var full_str = String<.A = FixedBufferAllocator, .allocator = fba>.new();
            let total_str = cursor.ref.as(Reader).read_to_string(full_str.ref)!;
            if total_str != 10 or not full_str.ref.equals_str("alpha-beta") {
                return false;
            }

            -- 3. BufReader<Cursor> read_line reports Utf8Err without corrupting destination String.
            var mixed = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            push_str_bytes(mixed.ref, "ok\n")!;
            mixed.ref.push(255)!;
            mixed.ref.push(254)!;
            mixed.ref.push(10)!;
            let bad_cursor = Cursor<.A = FixedBufferAllocator, .allocator = fba>.new(mixed);
            var bad_br = BufReader<
                Cursor<.A = FixedBufferAllocator, .allocator = fba>,
                .A = FixedBufferAllocator,
                .allocator = fba,
            >.with_capacity(5, bad_cursor)!;
            var lines = String<.A = FixedBufferAllocator, .allocator = fba>.new();
            let valid_len = bad_br.ref.read_line(lines.ref)!;
            if valid_len != 3 or not lines.ref.equals_str("ok\n") {
                return false;
            }
            let utf8_failed = bad_br.ref.read_line(lines.ref) match {
                _! => false,
                error IoErr.Os(_) => false,
                error IoErr.WriteZero => false,
                error IoErr.UnexpectedEof => false,
                error AllocErr.OutOfMemory => false,
                error AllocErr.InvalidLayout => false,
                error _ => true,
            };
            utf8_failed and lines.ref.equals_str("ok\n")
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
            let ok = exercise_memory_io(fba.ref) match {
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
