//! End-to-end native execution tests for custom runtime-dependent allocators
//! (`FixedBufferAllocator`, `ArenaAllocator`) and `std.collections` containers
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
