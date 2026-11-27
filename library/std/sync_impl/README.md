# 同步 (`std.sync`)

已实现公开模块树（实现位于 `sync_impl/`，经 `std.lib` 导出为 `std.sync`）：

- `std.sync.{Send, Sync}`：线程安全标记 trait，已为原语类型与 `std` 核心/集合/并发类型提供显式实现（`builtin.Continuation`、`TypedContinuation`、`MutexGuard`、`RwLock*Guard` 等显式不实现 `Send`/`Sync`）。
- `std.sync.atomics`（同时直接重导出至 `std.sync.*`；注：`atomic` 为词法关键字）：`AtomicOrdering`（含内存序合法性校验与 `0..4` LLVM 标签映射）、`CompareExchangeErr<T>`、`AtomicScalar`、`Atomic<T>`、`AtomicPtr<T>`、`fence`、`compiler_fence`，以及 `AtomicBool`/`AtomicI8`..`AtomicUsize` 别名。
- `std.sync` 锁与原语：`TryLockErr`、`PoisonErr<G>`（依 `OQ-08-04` 在 `Mutex` 与 `RwLock` 启用投毒）、`Mutex`/`MutexGuard`、`RwLock`/`RwLockReadGuard`/`RwLockWriteGuard`、`Condvar`/`WaitTimeoutResult`、`OnceLock`/`OnceInitErr`、`LazyLock`、`Barrier`/`BarrierWaitResult`、`Semaphore`/`SemaphorePermit`。
- `std.sync.{mpsc, mpmc, oneshot, broadcast}`：全套采用运行时依赖 allocator 子句（`.A: Type = DefaultAllocator, requires A :- Allocator + Sync, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref`），并在 `Drop` 中清零残留仿射 `T` 载荷后归还共享控制块与环形缓冲。
- `std.sync.thread`：`ThreadId`、`Thread`、`ThreadBuilder`、`ThreadJoinHandle`、`ScopedJoinHandle`、`ThreadScope`、`spawn`、`scope`、`current`、`sleep`、`yield_now`、`park`、`park_timeout`、`available_parallelism`。

## 挂接边界（待 `@llvm-effects` / `08-library` 合入后接通）

- `Atomic<T>` 与 `fence`/`compiler_fence` 的叶节点待接入 `@llvm-effects` Commit 2 的 `host-intrinsic.atomic_{load,store,swap,cmpxchg,cmpxchg_weak,fetch_add,fetch_sub,fetch_and,fetch_or,fetch_xor,fetch_max,fetch_min,fence,compiler_fence}`（`u8` tag `0..4`）。
- `Mutex`/`RwLock`/`Condvar`/`Barrier`/`Semaphore` 的竞争等待与锁守卫 `Drop` 中毒检测待接入 OS futex 与 `host-intrinsic.runtime_panic`（`{ T, ptr = 1 }`）展开状态。
- `OnceLock.get(*self) -> ?*T`、`get_or_init`/`get_or_try_init`、`LazyLock.{force, get}` 与 `ThreadPacket` 结果槽已接入 `builtin.mem.MaybeUninit<T>`，`AtomicPtr<T>` 已接入 `builtin.mem.TypeMarker<T>`；`OnceLock`/`LazyLock` 并在 `Drop` 中按状态安全析构。
- `std.sync.thread` 的 `spawn`/`sleep`/`yield_now`/`park`/`available_parallelism` 待接入 OS 线程与时间平台 intrinsic。
- `std.sync.stm` 依 `OQ-08-05` 明确延后，不提供。
