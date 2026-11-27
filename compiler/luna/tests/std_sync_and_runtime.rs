//! Real-sysroot integration tests for `std.process` panic types, `std.sync`
//! (`Send`/`Sync`, `atomic`, `Mutex`, `RwLock`, `Condvar`, `OnceLock`,
//! `LazyLock`, `Barrier`, `Semaphore`, allocator-parameterized `mpsc`/`mpmc`/
//! `oneshot`/`broadcast`, `thread`), and `std.runtime` (`TypedContinuation`,
//! `BoundedStackPool`, `GuardedStackPool`, `TaskContext`, `CancelToken`,
//! `Future`, `TaskScope`, `AsyncCore`).
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-std-sync-rt-{}-{}",
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
                "--build-target",
                "x86_64",
                "linux",
                "64",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .output()
            .unwrap()
    }

    fn check(&self) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "check",
                "main.fl",
                "--build-target",
                "x86_64",
                "linux",
                "64",
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
fn std_process_panic_types_and_atomics_execute_natively() {
    let source = r#"
        use std.prelude.*;
        use std.process.{CallerLocation, PanicInfo, PanicStrategy};
        use std.sync.atomics.{
            AtomicBool, AtomicI32, AtomicOrdering, AtomicPtr, AtomicU64, AtomicUsize,
            compiler_fence, fence,
        };

        fn run() -> bool {
            let loc = CallerLocation.new("main.fl", 10, 4);
            let info = PanicInfo.new("boom", loc, true);
            if loc.line() != 10 or not info.is_unwind() {
                return false;
            }
            let strat = PanicStrategy.unwind;
            let unwind_strat = PanicStrategy.unwind;
            if not strat.ref.as(Eq).eq(unwind_strat.ref) {
                return false;
            }

            var flag = AtomicBool.new(false);
            flag.ref.store(true, AtomicOrdering.release);
            if not flag.ref.load(AtomicOrdering.acquire) {
                return false;
            }
            let swapped = flag.ref.swap(false, AtomicOrdering.acq_rel);
            if not swapped or flag.ref.load(AtomicOrdering.relaxed) {
                return false;
            }

            var counter = AtomicU64.new(10);
            let prev = counter.ref.fetch_add(5, AtomicOrdering.seq_cst);
            if prev != 10 or counter.ref.load(AtomicOrdering.seq_cst) != 15 {
                return false;
            }
            let cas_ok = counter.ref.compare_exchange(
                15,
                42,
                AtomicOrdering.acq_rel,
                AtomicOrdering.acquire,
            ) match {
                v! => v == 15,
                error _ => false,
            };
            let cas_err = counter.ref.compare_exchange(
                99,
                100,
                AtomicOrdering.seq_cst,
                AtomicOrdering.relaxed,
            ) match {
                _! => false,
                error e => e.current() == 42,
            };
            if not cas_ok or not cas_err {
                return false;
            }

            var signed = AtomicI32.new(-5);
            let _ = signed.ref.fetch_max(3, AtomicOrdering.seq_cst);
            if signed.ref.load(AtomicOrdering.seq_cst) != 3 {
                return false;
            }

            var x: u64 = 7;
            var y: u64 = 9;
            var aptr = AtomicPtr<u64>.new(x.ref);
            let old_p = aptr.ref.swap(y.ref, AtomicOrdering.acq_rel);
            if old_p.* != 7 or aptr.ref.load(AtomicOrdering.acquire).* != 9 {
                return false;
            }

            var u = AtomicUsize.new(1);
            let upd_ok = u.ref.fetch_update(
                AtomicOrdering.acq_rel,
                AtomicOrdering.acquire,
                |cur| if cur < 1000 { cur + 2 } else { cur },
            ) match {
                v! => v == 1,
                error _ => false,
            };
            fence(AtomicOrdering.seq_cst);
            compiler_fence(AtomicOrdering.acquire);
            upd_ok and u.ref.load(AtomicOrdering.relaxed) == 3
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let out = case.run();
    assert_success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn std_sync_locks_once_barrier_and_semaphore_execute_natively() {
    let source = r#"
        use std.prelude.*;
        use std.sync.{
            Barrier, Condvar, LazyLock, Mutex, OnceLock, RwLock, Semaphore, TryLockErr,
        };

        fn make_lazy(f: %c fn() -> T) -> LazyLock<T, c>
        where
            T: Type,
            c: Type.vars.Closure,
        {
            LazyLock<T, c>.new(f)
        }

        fn run() -> bool {
            -- Mutex + TryLockErr + PoisonErr
            var m = Mutex<u64>.new(10);
            do {
                let g = m.ref.lock() match {
                    guard! => guard,
                    error _ => return false,
                };
                let contended = m.ref.try_lock() match {
                    _! => false,
                    error err => err match {
                        .WouldBlock => true,
                        .Poisoned => false,
                    },
                };
                if not contended {
                    return false;
                }
                g.ref.as_ptr().* = 55;
            };
            m.ref.mark_poisoned();
            let got_poison = m.ref.lock() match {
                _! => false,
                error perr => {
                    let g = perr.into_inner();
                    g.ref.as_ptr().* == 55
                },
            };
            if not got_poison or not m.ref.is_poisoned() {
                return false;
            }
            m.ref.clear_poison();
            if m.into_inner() match { v! => v != 55, error _ => true } {
                return false;
            }

            -- RwLock
            var rw = RwLock<u64>.new(12);
            do {
                let r1 = rw.ref.try_read() match { g! => g, error _ => return false };
                let r2 = rw.ref.try_read() match { g! => g, error _ => return false };
                let w_blocked = rw.ref.try_write() match {
                    _! => false,
                    error err => err match {
                        .WouldBlock => true,
                        .Poisoned => false,
                    },
                };
                if not w_blocked or r1.ref.as_ptr().* != 12 or r2.ref.as_ptr().* != 12 {
                    return false;
                }
            };
            do {
                let w = rw.ref.try_write() match { g! => g, error _ => return false };
                w.ref.as_ptr().* = 99;
            };
            if rw.ref.read() match { g! => g.ref.as_ptr().* != 99, error _ => true } {
                return false;
            }

            -- Condvar notify
            let cv = Condvar.new();
            cv.ref.notify_one();
            cv.ref.notify_all();

            -- OnceLock + LazyLock (`MaybeUninit<T>` get/force/get_or_init/get_or_try_init + Drop)
            var once = OnceLock<u64>.new();
            if once.ref.get() matches _? {
                return false;
            }
            let set_ok = once.ref.set(77) match { _! => true, error _ => false };
            let dup_err = once.ref.set(88) match {
                _! => false,
                error e => e.into_inner() == 88,
            };
            let get_ok = once.ref.get() match {
                p? => p.* == 77,
                null => false,
            };
            let read_ok = once.ref.with_ref(|p| p.* == 77) match {
                v? => v,
                null => false,
            };
            if not set_ok or not dup_err or not get_ok or not read_ok {
                return false;
            }
            let taken = once.ref.take() match { v? => v == 77, null => false };
            if not taken or once.ref.is_initialized() or (once.ref.get() matches _?) {
                return false;
            }
            if once.ref.get_or_init(|| 55).* != 55 {
                return false;
            }
            var once_try = OnceLock<u64>.new();
            let try_failed = once_try.ref.get_or_try_init(|| -> !u64 u64 { error 99 }) match {
                _! => false,
                error e => e == 99,
            };
            if not try_failed or once_try.ref.is_initialized() {
                return false;
            }
            let try_ok = once_try.ref.get_or_try_init(|| -> !u64 u64 { 88 }) match {
                p! => p.* == 88,
                error _ => false,
            };
            if not try_ok {
                return false;
            }

            var lazy = make_lazy(|| 123);
            if lazy.ref.get() matches _? {
                return false;
            }
            let lazy_ptr_ok = lazy.ref.force().* == 123;
            let lazy_get_ok = lazy.ref.get() match {
                p? => p.* == 123,
                null => false,
            };
            let lazy_ok = lazy_ptr_ok and lazy_get_ok and lazy.ref.with_ref(|p| p.* == 123);
            if not lazy_ok {
                return false;
            }

            -- Barrier (n = 1 leader) + Semaphore
            var bar = Barrier.new(1);
            if not bar.ref.wait().is_leader() {
                return false;
            }

            var sem = Semaphore.new(2);
            do {
                let p1 = sem.ref.try_acquire() match { p? => p, null => return false };
                let p2 = sem.ref.try_acquire() match { p? => p, null => return false };
                if sem.ref.try_acquire() matches _? {
                    return false;
                }
                let _ = p1;
                let _ = p2;
            };
            if sem.ref.available_permits() != 2 {
                return false;
            }
            let p_forget = sem.ref.acquire();
            p_forget.forget();
            sem.ref.available_permits() == 1 and lazy_ok
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let out = case.run();
    assert_success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn std_sync_channels_with_custom_allocator_are_leak_free() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{
            AllocErr, Allocator, FixedBufferAllocator, Layout, RawBlock, ReallocErr,
            SystemAllocator,
        };
        use std.sync.{Mutex, RecvErr, Send, Sync, TryRecvErr, TrySendErr};
        use std.sync.mpsc;
        use std.sync.mpmc;
        use std.sync.oneshot;
        use std.sync.broadcast;

        struct SyncFba {
            inner: Mutex<FixedBufferAllocator>,
        }

        impl SyncFba {
            fn from_raw_block(block: RawBlock) -> Self {
                SyncFba {
                    inner: Mutex<FixedBufferAllocator>.new(
                        FixedBufferAllocator.from_raw_block(block),
                    ),
                }
            }

            fn live_allocations(*self) -> usize {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                g.ref.as_ptr().live_allocations()
            }

            fn reset(*self) {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                g.ref.as_ptr().reset();
            }
        }

        impl Allocator for SyncFba {
            fn allocate(*self, layout: Layout) -> !AllocErr RawBlock {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                g.ref.as_ptr().as(Allocator).allocate(layout)
            }

            fn allocate_zeroed(*self, layout: Layout) -> !AllocErr RawBlock {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                g.ref.as_ptr().as(Allocator).allocate_zeroed(layout)
            }

            unsafe fn deallocate(*self, block: RawBlock) {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                unsafe {
                    g.ref.as_ptr().as(Allocator).deallocate(block);
                }
            }

            unsafe fn reallocate(*self, block: RawBlock, new_layout: Layout) -> !ReallocErr RawBlock {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                unsafe {
                    g.ref.as_ptr().as(Allocator).reallocate(block, new_layout)
                }
            }
        }

        impl Send for SyncFba {}
        impl Sync for SyncFba {}

        fn exercise(fba: *SyncFba) -> !AllocErr bool {
            -- 1. mpsc bounded + unbounded with Box<u64> affine payloads
            do {
                let (tx, rx) = mpsc.sync_channel<Box<u64, .A = SyncFba, .allocator = fba>, .A = SyncFba, .allocator = fba>(2)!;
                let b1 = Box<u64, .A = SyncFba, .allocator = fba>.new(10)!;
                let b2 = Box<u64, .A = SyncFba, .allocator = fba>.new(20)!;
                let b3 = Box<u64, .A = SyncFba, .allocator = fba>.new(30)!;
                tx.ref.try_send(b1) match { _! => {}, error _ => return false };
                tx.ref.try_send(b2) match { _! => {}, error _ => return false };
                let full_ok = tx.ref.try_send(b3) match {
                    _! => false,
                    error TrySendErr<Box<u64, .A = SyncFba, .allocator = fba>>.Full(returned) => returned.into_inner() == 30,
                    error _ => false,
                };
                if not full_ok {
                    return false;
                }
                let v1 = rx.ref.try_recv() match { v! => v.into_inner(), error _ => return false };
                if v1 != 10 {
                    return false;
                }
                -- Leave b2 in queue to test Drop cleanup of unconsumed payloads.
            }
            if fba.live_allocations() != 0 {
                return false;
            }
            fba.reset();

            -- 2. mpmc + clone_sender + clone_receiver
            do {
                let (tx, rx) = mpmc.channel<u64, .A = SyncFba, .allocator = fba>(2)!;
                let tx2 = tx.ref.clone_sender();
                let rx2 = rx.ref.clone_receiver();
                let _ = tx.ref.try_send(101);
                let _ = tx2.ref.try_send(202);
                let a = rx.ref.try_recv() match { v! => v, error _ => return false };
                let b = rx2.ref.try_recv() match { v! => v, error _ => return false };
                if a != 101 or b != 202 {
                    return false;
                }
            }
            if fba.live_allocations() != 0 {
                return false;
            }
            fba.reset();

            -- 3. oneshot
            do {
                let (tx, rx) = oneshot.channel<u64, .A = SyncFba, .allocator = fba>()!;
                let send_ok = tx.send(555) match { _! => true, error _ => false };
                let got = rx.recv() match { v! => v == 555, error _ => false };
                if not send_ok or not got {
                    return false;
                }
            }
            if fba.live_allocations() != 0 {
                return false;
            }
            fba.reset();

            -- 4. broadcast
            do {
                let (tx, rx1) = broadcast.channel<u64, .A = SyncFba, .allocator = fba>(4)!;
                let rx2 = tx.ref.subscribe();
                let sent_count = tx.ref.send(42) match { n! => n, error _ => return false };
                let v1 = rx1.ref.try_recv() match { v! => v, error _ => return false };
                let v2 = rx2.ref.try_recv() match { v! => v, error _ => return false };
                if sent_count != 2 or v1 != 42 or v2 != 42 {
                    return false;
                }
            }
            fba.live_allocations() == 0
        }

        fn run() -> bool {
            let layout = Layout.new(8192, 16) match {
                l? => l,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                b! => b,
                error _ => return false,
            };
            var fba = SyncFba.from_raw_block(backing);
            let ok = exercise(fba.ref) match {
                passed! => passed,
                error _ => false,
            };
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            ok
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let out = case.run();
    assert_success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn std_runtime_stack_pools_tasks_and_thread_execute_natively() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{
            AllocErr, Allocator, FixedBufferAllocator, Layout, RawBlock, ReallocErr,
            SystemAllocator,
        };
        use std.runtime.{
            BoundedStackPool, CancelErr, CancelToken, Future, FutureState, GuardedStackPool,
            StackPool, StackPoolErr, TaskContext, TaskErr, TaskId, TaskScope,
        };
        use std.sync.{Mutex, Send, Sync};
        use std.sync.thread;

        struct SyncFba {
            inner: Mutex<FixedBufferAllocator>,
        }

        impl SyncFba {
            fn from_raw_block(block: RawBlock) -> Self {
                SyncFba {
                    inner: Mutex<FixedBufferAllocator>.new(
                        FixedBufferAllocator.from_raw_block(block),
                    ),
                }
            }

            fn live_allocations(*self) -> usize {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                g.ref.as_ptr().live_allocations()
            }

            fn reset(*self) {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                g.ref.as_ptr().reset();
            }
        }

        impl Allocator for SyncFba {
            fn allocate(*self, layout: Layout) -> !AllocErr RawBlock {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                g.ref.as_ptr().as(Allocator).allocate(layout)
            }

            fn allocate_zeroed(*self, layout: Layout) -> !AllocErr RawBlock {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                g.ref.as_ptr().as(Allocator).allocate_zeroed(layout)
            }

            unsafe fn deallocate(*self, block: RawBlock) {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                unsafe {
                    g.ref.as_ptr().as(Allocator).deallocate(block);
                }
            }

            unsafe fn reallocate(*self, block: RawBlock, new_layout: Layout) -> !ReallocErr RawBlock {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                unsafe {
                    g.ref.as_ptr().as(Allocator).reallocate(block, new_layout)
                }
            }
        }

        impl Send for SyncFba {}
        impl Sync for SyncFba {}

        fn exercise_runtime(fba: *SyncFba) -> bool {
            -- 1. BoundedStackPool with SyncFba
            do {
                var pool = BoundedStackPool<256, .A = SyncFba, .allocator = fba>.new(2) match {
                    p! => p,
                    error _ => return false,
                };
                if pool.ref.total_slots() != 2 or pool.ref.active_slots() != 0 {
                    return false;
                }
                let overflow_ok = pool.ref.as(StackPool).acquire_stack(512) match {
                    _! => false,
                    error err => err match {
                        .StackSizeOverflow(req, cap) => req == 512 and cap == 256,
                        _ => false,
                    },
                };
                if not overflow_ok {
                    return false;
                }
                let slot1 = pool.ref.as(StackPool).acquire_stack(128) match {
                    s! => s,
                    error _ => return false,
                };
                let slot2 = pool.ref.as(StackPool).acquire_stack(128) match {
                    s! => s,
                    error _ => return false,
                };
                -- Triggers growing a second slab.
                let slot3 = pool.ref.as(StackPool).acquire_stack(128) match {
                    s! => s,
                    error _ => return false,
                };
                if pool.ref.active_slots() != 3 or pool.ref.total_slots() != 4 {
                    return false;
                }
                unsafe {
                    pool.ref.as(StackPool).release_stack(slot1);
                    pool.ref.as(StackPool).release_stack(slot2);
                    pool.ref.as(StackPool).release_stack(slot3);
                }
                if pool.ref.active_slots() != 0 {
                    return false;
                }
            }
            if fba.live_allocations() != 0 {
                return false;
            }
            fba.reset();

            -- 2. GuardedStackPool with SyncFba
            do {
                var gpool = GuardedStackPool<.A = SyncFba, .allocator = fba>.new(512) match {
                    p! => p,
                    error _ => return false,
                };
                let s = gpool.ref.as(StackPool).acquire_stack(256) match {
                    sl! => sl,
                    error _ => return false,
                };
                if s.ref.size_bytes() != 512 or gpool.ref.active_slots() != 1 {
                    return false;
                }
                unsafe {
                    gpool.ref.as(StackPool).release_stack(s);
                }
                if gpool.ref.active_slots() != 0 {
                    return false;
                }
            }
            if fba.live_allocations() != 0 {
                return false;
            }
            fba.reset();

            -- 3. TaskContext, CancelToken, Future, TaskScope
            do {
                var ctx = TaskContext.new(0);
                if ctx.ref.is_cancelled() {
                    return false;
                }
                ctx.ref.request_cancel();
                if not ctx.ref.is_cancelled() {
                    return false;
                }

                let tok = CancelToken<.A = SyncFba, .allocator = fba>.new() match {
                    t! => t,
                    error _ => return false,
                };
                let tok2 = tok.ref.clone_token();
                tok.ref.cancel();
                if not tok2.ref.is_cancelled() {
                    return false;
                }

                let fut = Future<u64, .A = SyncFba, .allocator = fba>.new_pending() match {
                    f! => f,
                    error _ => return false,
                };
                let cur_state = fut.ref.state();
                let pending_state = FutureState.pending;
                if not cur_state.ref.as(Eq).eq(pending_state.ref) {
                    return false;
                }
                fut.ref.complete_with(42);
                if not fut.ref.is_ready() {
                    return false;
                }
                let val_ok = fut.try_take() match {
                    res? => res match { v! => v == 42, error _ => false },
                    null => false,
                };
                if not val_ok {
                    return false;
                }

                let scope = TaskScope<.A = SyncFba, .allocator = fba>.new() match {
                    s! => s,
                    error _ => return false,
                };
                scope.ref.cancel_all();
                let scope_tok = scope.ref.cancel_token();
                if not scope_tok.ref.is_cancelled() {
                    return false;
                }
            }
            if fba.live_allocations() != 0 {
                return false;
            }
            fba.reset();

            -- 4. std.sync.thread with custom allocator
            let handle = thread.spawn<.A = SyncFba, .allocator = fba>(|| 84) match {
                h! => h,
                error _ => return false,
            };
            let joined = handle.join() match {
                v! => v == 84,
                error _ => false,
            };
            joined and fba.live_allocations() == 0
        }

        fn run() -> bool {
            let layout = Layout.new(16384, 16) match {
                l? => l,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                b! => b,
                error _ => return false,
            };
            var fba = SyncFba.from_raw_block(backing);
            let ok = exercise_runtime(fba.ref);
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            ok
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build());
    let out = case.run();
    assert_success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn std_runtime_async_effect_signatures_and_non_copy_non_send_rejections() {
    let valid_async = r#"
        use std.prelude.*;
        use std.runtime.{AsyncCore, CancelErr, TaskErr, yield_now, sleep};
        use std.time.Duration;

        fn step() -> #AsyncCore ![CancelErr, TaskErr] void {
            yield_now()!;
            sleep(Duration.from_millis(10))!;
        }

        fn run() -> bool { true }
    "#;
    let rejections = [
        (
            "TypedContinuation must not be Copy",
            r#"
                use std.runtime.TypedContinuation;
                fn req(v: T) where T :- Copy {}
                fn bad(k: TypedContinuation<u32, u32>) { req(k); }
            "#,
        ),
        (
            "TypedContinuation must not be Send",
            r#"
                use std.runtime.TypedContinuation;
                use std.sync.Send;
                fn req(v: T) where T :- Send {}
                fn bad(k: TypedContinuation<u32, u32>) { req(k); }
            "#,
        ),
        (
            "MutexGuard must not be Send",
            r#"
                use std.sync.{MutexGuard, Send};
                fn req(v: T) where T :- Send {}
                fn bad(g: MutexGuard<u32>) { req(g); }
            "#,
        ),
        (
            "StackSlot must not be Copy",
            r#"
                use std.runtime.StackSlot;
                fn req(v: T) where T :- Copy {}
                fn bad(s: StackSlot) { req(s); }
            "#,
        ),
        (
            "OQ-08-05: std.sync.stm must not exist",
            r#"
                use std.sync.stm.TVar;
                fn run() -> bool { true }
            "#,
        ),
        (
            "OQ-08-06: generic State effect must not exist in std",
            r#"
                use std.runtime.State;
                fn run() -> bool { true }
            "#,
        ),
    ];

    let checks: Vec<(&str, &str, bool)> = std::iter::once(("valid_async", valid_async, true))
        .chain(rejections.into_iter().map(|(l, s)| (l, s, false)))
        .collect();
    for batch in checks.chunks(4) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|&(label, source, expect_ok)| {
                    scope.spawn(move || (label, expect_ok, NativeCase::new(source).check()))
                })
                .collect();
            for handle in handles {
                let (label, expect_ok, out) = handle.join().unwrap();
                if expect_ok {
                    assert_success(&out);
                } else {
                    assert!(
                        !out.status.success(),
                        "expected failure for `{label}`, but check succeeded"
                    );
                }
            }
        });
    }
}

#[test]
fn std_process_sync_and_runtime_check_cleanly() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{
            AllocErr, Allocator, FixedBufferAllocator, Layout, RawBlock, ReallocErr,
            SystemAllocator,
        };
        use std.process.{CallerLocation, PanicInfo, PanicStrategy, abort, panic, set_panic_hook};
        use std.sync.{
            Barrier, Condvar, LazyLock, Mutex, OnceLock, RwLock, Semaphore, broadcast, mpmc, mpsc,
            oneshot, thread,
        };
        use std.sync.atomics.{
            AtomicBool, AtomicOrdering, AtomicPtr, AtomicU64, AtomicUsize, compiler_fence, fence,
        };
        use std.runtime.{
            BoundedStackPool, CancelToken, Future, FutureState, GuardedStackPool, TaskContext,
            TaskScope,
        };

        struct SyncFba {
            inner: Mutex<FixedBufferAllocator>,
        }

        impl SyncFba {
            fn from_raw_block(block: RawBlock) -> Self {
                SyncFba { inner: Mutex<FixedBufferAllocator>.new(FixedBufferAllocator.from_raw_block(block)) }
            }
        }

        impl Allocator for SyncFba {
            fn allocate(*self, layout: Layout) -> !AllocErr RawBlock {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                g.ref.as_ptr().as(Allocator).allocate(layout)
            }
            fn allocate_zeroed(*self, layout: Layout) -> !AllocErr RawBlock {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                g.ref.as_ptr().as(Allocator).allocate_zeroed(layout)
            }
            unsafe fn deallocate(*self, block: RawBlock) {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                unsafe {
                    g.ref.as_ptr().as(Allocator).deallocate(block);
                }
            }
            unsafe fn reallocate(*self, block: RawBlock, new_layout: Layout) -> !ReallocErr RawBlock {
                let g = self.inner.lock() match {
                    guard! => guard,
                    error p => p.into_inner(),
                };
                unsafe {
                    g.ref.as_ptr().as(Allocator).reallocate(block, new_layout)
                }
            }
        }

        impl std.sync.Send for SyncFba {}
        impl std.sync.Sync for SyncFba {}

        fn hook_fn(info: *PanicInfo) {
            let _ = info.message();
        }

        fn make_lazy(f: %c fn() -> u64) -> LazyLock<u64, c>
        where
            c: Type.vars.Closure,
        {
            LazyLock<u64, c>.new(f)
        }

        fn exercise(fba: *SyncFba) -> bool {
            let loc = CallerLocation.new("main.fl", 1, 1);
            let info = PanicInfo.new("boom", loc, true);
            let _ = info.is_unwind();
            let unwind = PanicStrategy.unwind;
            let abort_strat = PanicStrategy.abort;
            let _ = unwind.ref.as(Eq).eq(abort_strat.ref);
            set_panic_hook(hook_fn);
            if false {
                panic("auto-location");
            } else if false {
                panic("unreachable", .location = loc);
            } else if false {
                panic("suppressed-location", .location = null);
            } else if false {
                abort();
            }

            let ab = AtomicBool.new(false);
            ab.ref.store(true, AtomicOrdering.release);
            let au = AtomicU64.new(10);
            let _ = au.ref.fetch_add(1, AtomicOrdering.acq_rel);
            let aus = AtomicUsize.new(0);
            let _ = aus.ref.load(AtomicOrdering.acquire);
            var dummy: u64 = 1;
            let aptr = AtomicPtr<u64>.new(dummy.ref);
            let _ = aptr.ref.load(AtomicOrdering.relaxed);
            fence(AtomicOrdering.seq_cst);
            compiler_fence(AtomicOrdering.acq_rel);

            let m = Mutex<u64>.new(7);
            do {
                let g = m.ref.lock() match {
                    v! => v,
                    error p => p.into_inner(),
                };
                let _ = g.ref.as_ptr().*;
            };
            let rw = RwLock<u64>.new(9);
            do {
                let rg = rw.ref.read() match {
                    v! => v,
                    error p => p.into_inner(),
                };
                let _ = rg.ref.as_ptr().*;
            };
            let cv = Condvar.new();
            cv.ref.notify_one();
            let once = OnceLock<u64>.new();
            let _ = once.ref.set(42);
            let _ = once.ref.get();
            let _ = once.ref.get_or_init(|| 7);
            let lazy = make_lazy(|| 99);
            let _ = lazy.ref.get();
            let _ = lazy.ref.force();
            let barrier = Barrier.new(1);
            let _ = barrier.ref.wait().is_leader();
            let sem = Semaphore.new(2);
            let permit = sem.ref.acquire();
            let _ = permit.ref.count();

            let _ = mpsc.channel<u64, .A = SyncFba, .allocator = fba>();
            let _ = mpsc.sync_channel<u64, .A = SyncFba, .allocator = fba>(4);
            let _ = mpmc.channel<u64, .A = SyncFba, .allocator = fba>(4);
            let _ = oneshot.channel<u64, .A = SyncFba, .allocator = fba>();
            let _ = broadcast.channel<u64, .A = SyncFba, .allocator = fba>(4);

            let _ = BoundedStackPool<4096, .A = SyncFba, .allocator = fba>.new(1);
            let _ = GuardedStackPool<.A = SyncFba, .allocator = fba>.new(4096);
            let ctx = TaskContext.new(0);
            let _ = ctx.ref.is_cancelled();
            let tok = CancelToken<.A = SyncFba, .allocator = fba>.new();
            let fut = Future<u64, .A = SyncFba, .allocator = fba>.new_pending();
            let _ = fut;
            let _ = tok;
            let st = FutureState.pending;
            let _ = st;
            let scope = TaskScope<.A = SyncFba, .allocator = fba>.new();
            let _ = scope;
            let _ = thread.spawn<.A = SyncFba, .allocator = fba>(|| 1);
            true
        }

        fn run() -> bool {
            let layout = Layout.new(8192, 16) match {
                l? => l,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                b! => b,
                error _ => return false,
            };
            var fba = SyncFba.from_raw_block(backing);
            let ok = exercise(fba.ref);
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            ok
        }
    "#;
    assert_success(&NativeCase::new(source).check());
}

#[test]
fn meta_bounded_stack_pool_and_attrs_effects_stack_strategy() {
    let positive = r#"
        use std.prelude.*;
        use std.runtime.{
            bounded_stack_bytes_for, require_bounded_stack_bytes, verify_bounded_stack_bytes,
        };

        ^attrs.layout.align(64)
        struct Frame64 {
            x: u64,
        }

        fn leaf_worker(x: u64) -> u64 {
            assumes x < 1000000;
            x + 1
        }

        fn aligned_worker(x: u64) -> u64 {
            let f = Frame64 { x: x };
            leaf_worker(f.x)
        }

        fn recursive_worker(n: u64) -> u64 {
            if n == 0 { 0 } else { recursive_worker(n - 1) }
        }

        comptime fn verify_phase2a_contracts() -> bool {
            let leaf_ok = verify_bounded_stack_bytes(leaf_worker, 256);
            let aligned_too_small = not verify_bounded_stack_bytes(aligned_worker, 32);
            let rec_unbounded = not verify_bounded_stack_bytes(recursive_worker, 4096)
                and (bounded_stack_bytes_for(recursive_worker) matches null);
            let req_bytes = require_bounded_stack_bytes(leaf_worker, 256);

            let bp: meta.Object = attrs.effects.stack_strategy(.bounded_pool);
            let gp: meta.Object = attrs.effects.stack_strategy(.guarded_pool);
            let inl: meta.Object = attrs.effects.stack_strategy(.inline_frame);

            let bp_ok = bp.get<Symbol>(.__flurry_effect_strategy) match {
                s? => s == .bounded_pool,
                null => false,
            };
            let gp_ok = gp.get<Symbol>(.__flurry_effect_strategy) match {
                s? => s == .guarded_pool,
                null => false,
            };
            let inl_ok = inl.get<Symbol>(.__flurry_effect_strategy) match {
                s? => s == .inline_frame,
                null => false,
            };

            leaf_ok
                and aligned_too_small
                and rec_unbounded
                and req_bytes <= 256
                and bp_ok
                and gp_ok
                and inl_ok
        }

        fn run() -> bool {
            const ok = verify_phase2a_contracts();
            ok
        }
    "#;
    let recursive_reject = r#"
        use std.runtime.require_bounded_stack_bytes;
        fn rec(n: u64) -> u64 { if n == 0 { 0 } else { rec(n - 1) } }
        comptime fn bad() -> usize { require_bounded_stack_bytes(rec, 4096) }
        fn run() -> bool { const _ = bad(); true }
    "#;

    let overflow_reject = r#"
        use std.runtime.require_bounded_stack_bytes;
        ^attrs.layout.align(64)
        struct BigFrame { x: u64 }
        fn heavy(x: u64) -> u64 { let b = BigFrame { x: x }; b.x }
        comptime fn bad() -> usize { require_bounded_stack_bytes(heavy, 32) }
        fn run() -> bool { const _ = bad(); true }
    "#;

    let (pos_case, rec_out, ov_out) = std::thread::scope(|scope| {
        let h_pos = scope.spawn(|| {
            let pos_case = NativeCase::new(positive);
            assert_success(&pos_case.build());
            let pos_out = pos_case.run();
            assert_success(&pos_out);
            assert_eq!(pos_out.stdout, b"(bool true)\n");
            pos_case
        });
        let h_rec = scope.spawn(|| NativeCase::new(recursive_reject).check());
        let h_ov = scope.spawn(|| NativeCase::new(overflow_reject).check());
        (
            h_pos.join().unwrap(),
            h_rec.join().unwrap(),
            h_ov.join().unwrap(),
        )
    });
    drop(pos_case);

    assert!(!rec_out.status.success());
    let rec_err = String::from_utf8_lossy(&rec_out.stderr);
    assert!(rec_err.contains("unbounded"), "{rec_err}");

    assert!(!ov_out.status.success());
    let ov_err = String::from_utf8_lossy(&ov_out.stderr);
    assert!(ov_err.contains("slot_bytes"), "{ov_err}");
}
