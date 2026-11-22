# 08. 效应 ABI、异步运行时、并发同步与基础网络 Wishlist（`builtin` / `std.runtime` / `std.sync` / `std.sync.stm` / `std.net`）

> 本文档为 Wave 2 核心并发、控制与运行时系统 Wishlist（`WL-08-01` ~ `WL-08-18`）。覆盖核心能力处理协议（`HandleError` / `HandleEffect` / `DirectPassing` / `Panic` 契约）、Tier 1 vs Tier 2 严格防膨胀边界、限定 continuation 与三级栈池（`builtin.Continuation` / `TypedContinuation` / `StackPool` / `BoundedStackPool` / `GuardedStackPool`）、结构化 `async effect` 运行时（`std.runtime`）、原子与同步原语/通道/线程（`std.sync`）、BTO 软件事务内存与有序锁（`std.sync.stm`），以及极简同步/异步 L3/L4 套接字（`std.net`）。

---

## 1. 范围、权威来源与归属包

### 1.1 权威依据与历史截断边界

严格依照 [AGENTS.md](../../AGENTS.md) 确立的权威顺位与最近性规则：

1. **第一顺位（最高权威）**：作者确认规则与 `AGENTS.md` 第 5 节不可破坏的语义边界。
2. **第二顺位（现行正式文档 `docs/`）**：
   - 代数效应、限定 continuation 与直接传值：[docs/algebraic-effect-and-delimited-continuation/index.md](../../docs/algebraic-effect-and-delimited-continuation/index.md)、[algebraic-effect.md](../../docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md)、[async-algebraic-effect.md](../../docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md)、[direct-passing.md](../../docs/algebraic-effect-and-delimited-continuation/direct-passing.md)、[effect-abi.md](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)、[error-handling.md](../../docs/algebraic-effect-and-delimited-continuation/error-handling.md)、[handling-capability.md](../../docs/algebraic-effect-and-delimited-continuation/handling-capability.md)、[multi-prompt-delimited-continuation.md](../../docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md)
   - 类型行与 handler 契约：[docs/type/rows.md](../../docs/type/rows.md)、[docs/type/handlers.md](../../docs/type/handlers.md)、[docs/type/qualified-types/effect-type.md](../../docs/type/qualified-types/effect-type.md)
   - 并发与 STM 规范：[docs/concurrency/prompt.fl](../../docs/concurrency/prompt.fl)（注：[docs/concurrency/software-transactional-memory.md](../../docs/concurrency/software-transactional-memory.md) 当前为 0 字节空文件，相关未决契约单列于第 5 节）
3. **架构设计与跨卷协同**：
   - Silver 栈池与帧调度契约：[tmp/silver-design/01-stack-instance-and-event-loop.md](../silver-design/01-stack-instance-and-event-loop.md)、[03-async-and-scheduling.md](../silver-design/03-async-and-scheduling.md)、[04-compiler-wishlist.md](../silver-design/04-compiler-wishlist.md)（`P0-3`、`P1-1`）
   - 同系列 Wishlist 交叉对齐：[05-meta-cmtt-macros-reflection.md](./05-meta-cmtt-macros-reflection.md)（`meta.runtime.bounded_stack_bytes`）、[06-attrs-build-pipeline-projection.md](./06-attrs-build-pipeline-projection.md)（`^attrs.effects.*` 与 `^attrs.compilation.caller_location`）、[07-std-mem-collections-io-os.md](./07-std-mem-collections-io-os.md)（`std.mem` 分配器/`std.io`/`std.time`/`std.process.panic`）、[10-official-protocol-packages-and-boundary.md](./10-official-protocol-packages-and-boundary.md)（Tier 2 `async_runtime`/`telemetry`/`random`/`datetime`/`http`/`wire` 边界）
4. **近期 `proposal_trace/` 草案截断边界（严格限于 `2026-5` 至 `2026-10`）**：
   - 仅以下 6 个最近半年草案作问题意图与原型对照，**均不构成语义权威**，凡与 `docs/`/`AGENTS.md` 冲突处（如 `2026-8-25-handler-capability.fl` 在 `fn` 签名上写 `handles (...)`、或旧 `{ ... }` 选项/省略号）一律按 `docs/` 重写：
     - [2026-9-18-handling-capability-by-trait-system.fl](../../proposal_trace/2026-9-18-handling-capability-by-trait-system.fl)
     - [2026-8-25-effect-enum.fl](../../proposal_trace/2026-8-25-effect-enum.fl)
     - [2026-8-25-handler-capability.fl](../../proposal_trace/2026-8-25-handler-capability.fl)
     - [2026-9-28-algebraic-effects.fl](../../proposal_trace/2026-9-28-algebraic-effects.fl)
     - [2026-9-29-continuation-pool.fl](../../proposal_trace/2026-9-29-continuation-pool.fl)
     - [2026-9-19-inline-effect.fl](../../proposal_trace/2026-9-19-inline-effect.fl)
   - **严禁引用任何 `2024-*`、`2025-*`、`2026-1..4-*` 历史草案或 `flurry_tutorial/`**。

### 1.2 归属包与模块树（严守六内置包边界）

| 模块路径 | 归属包 | 核心职责与导出 | 关键上下游依赖 |
|---|---|---|---|
| `builtin.Continuation` | `builtin` | 无公开类型参数的原始限定 continuation 句柄、控制后端与 C 帧屏障接口 | LLVM host `{ T, ptr }` 控制返回、[effect-abi.md](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md) |
| `builtin.effects` | `builtin` | `HandleError`、`HandleEffect` 等核心处理能力协议 trait 与内部中止控制通道 | `compiler/elaboration`、`program_verification` |
| `attrs.effects` | `attrs` | `^attrs.effects.direct_passing`、`^attrs.effects.stack_strategy(mode)` | `06` `WL-06-04`、`docs/comptime/official-attribute-keys.md` |
| `std.runtime` | `std`（`library/std/runtime/`） | `TypedContinuation`、`StackSlot`、`StackPool`、`BoundedStackPool`、`GuardedStackPool`、`TaskContext`、`TaskScope`、`JoinHandle`、`CancelToken`、`CancelErr`、核心 `async effect` 挂起/取消原语与 Tier 2 `async_runtime` 桥 | 依赖 `std.mem.Allocator`、`std.sync.atomic`、`meta.runtime.bounded_stack_bytes` |
| `std.sync` | `std`（`library/std/sync/`） | `Send`/`Sync`、`atomic`（`Atomic*`/`AtomicOrdering`/`fence`）、`Mutex`/`RwLock`/`Condvar`/`OnceLock`/`LazyLock`/`Barrier`/`Semaphore`、`mpsc`/`mpmc`/`oneshot`/`broadcast` 通道、`thread`（`Thread`/`ThreadScope`/`JoinHandle`） | 依赖 `std.mem.Allocator`、`std.os` 线程/futex/mutex 原语、`std.time` |
| `std.sync.stm` | `std`（`library/std/sync/`） | `Transactional`、`TVar`、`TArray`、`TQueue`、`OrderedLock`、`StmErr`、`retry`、`or_else`、`atomic(vars) { ... }` | 依赖 `compiler/hir`/`elaboration` 事务块降码与 `std.sync.atomic` |
| `std.net` | `std`（`library/std/net/`） | `IpAddr`/`Ipv4Addr`/`Ipv6Addr`/`SocketAddr*`/`AddrParseErr`、`TcpListener`/`TcpStream`/`UdpSocket`/`UnixListener`/`UnixStream`/`Shutdown`/`NetErr` 与异步适配接口 | 依赖 `std.io`、`std.os` socket 原语、`std.runtime` / Tier 2 `async_runtime` |

### 1.3 五大强制语义与语法红线

1. **错误 ABI（`!E T`）与效应 ABI（`#Eff T`）严格正交分离**：错误返回占位结果槽 `{ T_with_err_tag, null }`；效应控制使用处理器 evidence / 栈捕获 / 隐藏中止通道控制字。**严禁在函数签名上添加 `handles` 子句**（`handles` 仅作为 `let pat handles expr = ...;` 绑定模式与多重 `and` 约束使用）。
2. **运行时依赖类型 `allocator` 范式（禁止硬编码 `SystemAllocator`）**：`BoundedStackPool`、`GuardedStackPool`、`TaskScope`、异步通道（`mpsc`/`mpmc`/`broadcast`）、`TArray`/`TQueue`、非全局 DNS 解析（`ToSocketAddrs`）等一切堆分配实体均采用规范签名：
   ```flurry
   where
       T,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   ```
3. **`Once` 单次消费与 C 帧安全契约**：`TypedContinuation`、`Builtin` `Continuation`、`MutexGuard`、`RwLock*Guard`、`JoinHandle`、`CancelScope` 等遵循原生仿射 `Once`；严禁实现 `Copy`/`Clone`（除显式可克隆句柄 `CancelToken`/`Sender` 外）；跨 C 帧捕获的 continuation 可恢复、**禁止放弃（否则触发 trap）**。
4. **语法精度要求**：`struct`/`trait`/`enum`/`union`/`fn` 声明一律用 `where` 子句（仅 `normal_form_head_def`/`newtype`/`typealias` 与 `impl Target<...>` 允许名称后 `<...>`）；多行 `where`/`requires` 的 `{` 另起对齐一行；无主体的 `fn`/`effect`/`assoc` 声明以 `;` 结束；`assoc` 必须显式写 `: Type`；`while true {`（无 `loop`）；`expr! { catch pat => ... }`（无裸 `catch`）；`Tuple<A, B>`（非 `(A, B)`）；`PascalCaseErr` + `PascalCase` 错误变体 vs `snake_case` 普通变体；`and`/`or`/`not`；`--`/`{- -}`；中缀运算符两侧空格。
5. **官方属性与集中绑定**：官方属性全用 `^attrs.*` 包装 `__flurry_*` 键；禁止在使用处写 `.symbol'builtin`。

### 1.4 严格 Tier 1 vs Tier 2 效应与运行时防膨胀边界

依 `AGENTS.md` 六内置包边界与 [10-official-protocol-packages-and-boundary.md](./10-official-protocol-packages-and-boundary.md) §1.3–1.4，本文严格守卫 Tier 1（`builtin` + `std`）的极简核心定位：

| 类别 | 严格保留在 Tier 1（`builtin` / `std`）的内容 | 明确拒绝放入 Tier 1 的膨胀内容 | 正确归属层（Tier 2 / Tier 3） |
|---|---|---|---|
| **控制与续体 ABI** | `builtin.Continuation`、`HandleError`/`HandleEffect`、隐藏中止通道、`Panic`/`Abort` | 具体调度器实现、协程池策略引擎插件 | Tier 1 最小底座；Tier 3 调度器消费者 |
| **栈池与异步内核** | `std.runtime.StackPool`/`BoundedStackPool`/`GuardedStackPool`、`TaskScope`/`CancelToken`/`JoinHandle`、核心 `async effect` 挂起/取消协议 | `io_uring` 事件循环、`epoll`/`kqueue` reactor、work-stealing 线程池具体实现、`Silver` UI 主循环 | Tier 2：`async_runtime`（`Executor`/`Reactor`/`Timer`/`AsyncReader`/`AsyncWriter`）；Tier 3：`io_uring_rt`/`epoll_rt`/`silver` |
| **领域效应** | 无（仅核心控制/异步/取消机制） | `Log`/`Trace`/`Metric` 效应、`Random`/`random_bytes` 效应、`system_clock`/历法/时区效应 | Tier 2：`telemetry`、`random`、`datetime` |
| **通用代数效应套件** | 仅 `Cancel` 与语言内建 `emit_item`（`Iterator`）；`State`/`Reader`/`Writer`/`Yield`/`NonDet` 移出 Tier 1（或由 `Q6` 裁决边界） | 完整效应Monad变换栈、非确定性回溯搜索树、多路选择效应组合库 | Tier 2：`effects_ext`（可选薄协议包） |
| **同步与 STM** | `std.sync`（`Atomic`/`Mutex`/`RwLock`/`Condvar`/`mpsc`/`mpmc`/`oneshot`/`broadcast`/`Thread`/`ThreadScope`）与 `std.sync.stm` | 分布式共识、Actor 消息框架、无锁数据结构百科 | Tier 1 基础同步；Tier 3 并发架构框架 |
| **网络 (`std.net`)** | 原始 L3/L4 OS 套接字：`IpAddr`/`SocketAddr`/`TcpListener`/`TcpStream`/`UdpSocket`/`Unix*` 与非阻塞异步挂起适配 | HTTP/1.1/2/3、TLS/SSL、WebSocket、QUIC、gRPC、URL 解析、DNS 高级客户端 | Tier 2：`wire`、`http`；Tier 3：`h2_flurry`、`rustls_bridge` |

---

## 2. 现状与缺口全景总表

> **核实基线**：当前 [library/std/runtime/README.md](../../library/std/runtime/README.md) 与 [library/std/sync/README.md](../../library/std/sync/README.md) 均为“待实现”空占位（仅 3 行 Markdown）；仓库无 `std.net` 目录。编译器层面，`compiler/hir/src/item.rs`、`compiler/elaboration/src/program/effects.rs` 与 `attributes/effects.rs` 已接入同步直接效应、行解析、部分转发与 `direct_passing`（LLVM host 已支持 `{ T, ptr }` 控制字返回、原子取消标志与栈池上下文槽位传递，见 [effect-abi.md:193-199](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)）；`Continuation` 捕获/恢复/放弃、`BoundedStackPool`、`HandleError`/`HandleEffect` trait、`async effect` 运行时、`std.sync`、`std.sync.stm`、`std.net` 与 `panic` 运行时均未落地。

| ID | 模块与主题 | 现状（核查仓库事实） | 核心缺口（Wishlist 目标） | 关联编译器 / 跨文档依赖 | 优先级 |
|---|---|---|---|---|---|
| `WL-08-01` | `HandleError` 与 `HandleEffect` 处理能力协议 | 仅有 `effect-handle` 语法与内联匹配；`builtin.effects` 与 trait 协议未注册 | 定义 `builtin.effects.HandleError`、`DirectEffect`/`HandleEffect`（含 `effect enum` 分组与恢复路径）并打通 `let x handles ...` + `.use(h)` elaboration | `compiler/elaboration/effects`、`library/language-items.conf`、**OQ-08-01** | **P0** |
| `WL-08-02` | `^attrs.effects.direct_passing` 与 `stack_strategy` | `library/attrs/effects_impl/strategy.fl` 已导出 `{ __flurry_direct_passing: true }`；`attributes/effects.rs` 校验有参/异步拒绝 | 统一前端/中端 `__flurry_direct_passing` 消费、开放行实例化时隐式传值与覆盖刷新；新增 `^attrs.effects.stack_strategy`（`__flurry_effect_strategy`） | `06` `WL-06-04`、`docs/comptime/official-attribute-keys.md` | **P0** |
| `WL-08-03` | 错误/效应 ABI 隔离与 `Panic`/`Abort` 终止契约 | LLVM host 返回 `{ T, ptr }`（控制字 `1` 表示检查失败），但 `panic` 无统一标准库入口与展开契约（`library/README.md:395`） | 确立 `builtin.panic`/`std.process.panic` 运行时与 `comptime` 语义：基于控制字逆序 `Drop` 展开（C 帧 trap、`panic = .abort` 配置）、保持空效应行、`PanicInfo` + `caller_location` | `07` `WL-07-17`/Q1、`06` `__flurry_caller_location`、**OQ-08-02** | **P0** |
| `WL-08-04` | `effect enum` 组、部分转发与动态 evidence 路由 | 编译器支持直接同步 `effect enum` 与部分转发；未展开调用或可恢复多 occurrence 动态链路由未完整打通 | 完整实现多 occurrence 有序路由表、动态 `EvidenceVec` 传递与非尾恢复外层供应重绑定 | `docs/type/rows.md`、`compiler/mir`/`llvm` | **P0** |
| `WL-08-05` | `builtin.Continuation` 原始捕获原语与 C 帧屏障 | 无库定义；`effect-abi.md` 规定无公开类型参数及 C 帧放弃运行时 trap | 新增 `builtin.Continuation`（`Once`）、`unsafe` 捕获/恢复/放弃原语、LLVM stack/frame 切换后端与 C 帧屏障 trap | `compiler/continuation` / `host/llvm`、**OQ-08-03** | **P0** |
| `WL-08-06` | `std.runtime.TypedContinuation` 与 `TaskContext` | 仅 `effect-abi.md:199` 阐述 task context 中的取消标志与栈池指针 | 新增类型安全 `Once` 封装 `TypedContinuation`（`resume_with(self, Y) -> B`、尾恢复优化、自动 `Drop` 放弃）与 `TaskContext` 访问接口 | `WL-08-05`、`std.sync.atomic` | **P0** |
| `WL-08-07` | `StackPool`、`BoundedStackPool`、`GuardedStackPool`（Silver P0-3 / P1-1） | `library/std/runtime/` 为空占位；无池化后端实现 | 定义 `StackPool` trait、`StackSlot`、`StackPoolErr`、`BoundedStackPool`（`slot_bytes: usize` + allocator 子句）与 `GuardedStackPool`（虚拟页+guard page），对接 `meta.runtime.bounded_stack_bytes` | `05` `WL-05-31`、`tmp/silver-design/01`/`04` `P0-3`/`P1-1` | **P0**（`StackPool`/`BoundedStackPool`） / **P1**（`GuardedStackPool`） |
| `WL-08-08` | 结构化 `async effect` 运行时核心（`std.runtime`） | `docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md` 已定语义；仓库零实现 | 新增 `Future`、`JoinHandle`、`TaskScope`（正常等完、异常取消等清理）、`CancelToken`、`CancelErr`、`async effect` 请求登记-等待交接协议 | `WL-08-05`..`07`、`std.sync.atomic`、`async-algebraic-effect.md` | **P1** |
| `WL-08-09` | `spawn` / `join` / `select` / `race` / `sleep` / `timeout` 与 Tier 2 `async_runtime` 桥接 | 仓库空白；Tier 2 协议已定于 `10` §3.5（`Executor`/`Reactor`/`AsyncRuntimeOps`） | 提供 `std.runtime` 最小协作调度钩子与 `AsyncIoBridge`；保留 `io_uring`/`epoll`/work-stealing 引擎在 Tier 2/3；提供零丢失 `select`/`race`/`timeout` 并发组合子 | `10` `WL-10-10`、`std.time.Duration` | **P1** |
| `WL-08-10` | Tier 1 vs Tier 2 效应边界（`effects_ext` / `telemetry` / `random` / `datetime`） | `proposal_trace/` 中常用 `Log` 作为效应示例，易误入 `builtin`/`std` | 正式将 `Log`/`Rng`/`Clock` 锁定在 `10` Tier 2；将 `State`/`Reader`/`Writer`/`Yield`/`NonDet` 划入 `effects_ext` 或显式裁决（见 §5 Q6） | `10` §1.3–1.4、**OQ-08-06** | **P0** |
| `WL-08-11` | `std.sync.atomic` 与 `Send` / `Sync` 标记协议 | 仓库空白；`07` `Arc`/`ArcWeak` 被此直接阻塞（`WL-07-03`） | 定义 `Send`/`Sync`、`AtomicOrdering`、`Atomic`（含整型/布尔/指针特化）、`fence`/`compiler_fence`，接入 LLVM 原子指令与 `-Clink`/TSO/ARM 内存序 | `07` `WL-07-03`、LLVM `atomicrmw`/`cmpxchg`/`fence` | **P0** |
| `WL-08-12` | `std.sync` 锁、条件变量与单次初始化 | 仓库空白 | 新增 `Mutex`/`MutexGuard`、`RwLock`/`RwLockReadGuard`/`RwLockWriteGuard`、`Condvar`、`OnceLock`、`LazyLock`、`Barrier`、`Semaphore`、`PoisonErr`/`TryLockErr` | `WL-08-11`、`std.os` futex/pthread/SRWLock 原语、**OQ-08-04** | **P1** |
| `WL-08-13` | `std.sync` 异步/阻塞并发通道（带 Allocator） | 仓库空白 | 新增 `mpsc`（bounded/unbounded）、`mpmc`、`oneshot`、`broadcast`；全部环形/链表节点缓冲均支持 `.A` / `quote .allocator`，错误载荷保活未发送值 `T` | `WL-08-11`、`WL-08-12`、`std.mem.Allocator` | **P1** |
| `WL-08-14` | `std.sync.thread` 线程与作用域线程 | 仓库空白 | 新增 `Thread`、`ThreadId`、`ThreadBuilder`、`ThreadJoinHandle`、`ThreadScope`、`ScopedJoinHandle`、`spawn`/`scope`/`park`/`unpark`/`yield_now`/`available_parallelism` | `std.os` `pthread`/`Win32` 线程原语、`Send` 契约 | **P1** |
| `WL-08-15` | `std.sync.stm`（BTO STM 与 `OrderedLock`） | `docs/concurrency/prompt.fl` 已给 BTO + OrderedLock 草图；`software-transactional-memory.md` 为空 | 补齐 `Transactional`、`TVar`、`TArray`、`TQueue`、`OrderedLock`、`StmErr`、`retry`、`or_else`、`atomic(vars) { ... }` 编译降码（栈上固定 log + 版本时钟 + 禁用不可回滚 IO 效应） | `WL-08-11`、`compiler/hir`/`elaboration`、**OQ-08-05** | **P2** |
| `WL-08-16` | `std.net` 地址模型与解析 | 仓库空白 | 新增 `Ipv4Addr`、`Ipv6Addr`、`IpAddr`、`SocketAddrV4`、`SocketAddrV6`、`SocketAddr`、`AddrParseErr`、`ToSocketAddrs`（allocator 子句） | `std.fmt.Display`/`Debug`、`std.collections.Vec`、`std.ffi`/`std.os` `getaddrinfo` | **P1** |
| `WL-08-17` | `std.net` 同步 TCP / UDP / Unix 套接字 | 仓库空白 | 新增 `TcpListener`、`TcpStream`、`UdpSocket`、`UnixListener`、`UnixStream`、`Incoming`、`Shutdown`、`NetErr`；全面实现 `std.io.{Reader, Write, ByteWriter}` 与 `Drop` 自动关闭 | `07` `std.io`、`std.os` socket/`poll` 原语、`std.time.Duration` | **P1** |
| `WL-08-18` | `std.net` 非阻塞异步适配与防膨胀守卫 | 仓库空白 | 为各套接字提供非阻塞模式与 `async_io` 桥（对接 Tier 2 `async_runtime` `AsyncReader`/`Reactor`）；严禁掺入 HTTP/TLS/WebSocket/QUIC | `WL-08-08`/`09`、`10` `WL-10-10`/`11` | **P2** |

---

## 3. 详细规格设计

### 3.1 核心处理能力协议与 `Panic` / `Abort` 契约（`WL-08-01` ~ `WL-08-04`）

#### 3.1.1 `HandleError` 与 `HandleEffect` 协议族（归属 `builtin.effects`）

依据 [docs/algebraic-effect-and-delimited-continuation/handling-capability.md](../../docs/algebraic-effect-and-delimited-continuation/handling-capability.md) 与 [2026-9-18-handling-capability-by-trait-system.fl](../../proposal_trace/2026-9-18-handling-capability-by-trait-system.fl)，处理器绑定在词法作用域内声明能力，**安装仍由 `.use(handler)` 或 `expr# { ... }` 显式发生，且函数签名严禁出现 `handles` 子句**：

```flurry
-- 归属：builtin.effects
pub trait HandleError
where
    E: Type,
    R: Type,
{
    fn handle_error(*self, err: E) -> R;
}

-- 1. 同步直接效应处理能力：接收操作实参 X，交付结果 Y；不捕获 continuation
-- OpTag 为编译期具名操作实例标签（如单个 effect 或 effect enum 的具体项）
pub trait DirectEffectHandler
where
    OpTag: Type,
    Args: Type,
    Y: Type,
{
    fn handle_direct(*self, args: Args) -> Y;
}

-- 2. 可恢复限定效应处理能力：接收操作实参 Args 与唯一 TypedContinuation<Y, B>，产出 B
pub trait ResumptiveEffectHandler
where
    OpTag: Type,
    Args: Type,
    Y: Type,
    B: Type,
{
    fn handle_resumptive(*self, args: Args, k: std.runtime.TypedContinuation<Y, B>) -> B;
}

-- 3. 统一 HandleEffect 入口抽象（用于 .use(handler) 对单操作或 effect enum 组接入，具体编码见 OQ-08-01）
pub trait EffectRunner
where
    GroupOrOp: Type,
    BodyResult: Type,
{
    fn run_body(*self) -> BodyResult;
}

pub trait HandleEffect
where
    GroupOrOp: Type,
    BodyResult: Type,
    FinalResult: Type,
{
    fn handle_effect_body(*self, runner: *dyn EffectRunner<GroupOrOp, BodyResult>) -> FinalResult;
}
```

- **标准能力绑定语法与多重错误处理**：
  ```flurry
  -- 正例：handles 仅在模式绑定出现，多个错误类型以 and 连接，函数签名完全无 handles
  enum NetErr {
      TimedOut(u32),
      ConnectionRefused,
  }

  enum ParseErr {
      UnexpectedToken(usize),
  }

  struct FallbackNet {
      default_port: u32,
  }

  impl HandleError<NetErr, u32> for FallbackNet {
      fn handle_error(*self, err: NetErr) -> u32 {
          err match {
              .TimedOut(_) => self.default_port.*,
              .ConnectionRefused => 80u32,
          }
      }
  }

  fn connect_and_parse(raw: str) -> ![NetErr, ParseErr] u32 {
      if raw == "" {
          error ParseErr.UnexpectedToken(0usize)
      } else {
          443u32
      }
  }

  test {
      let net_handler handles NetErr = FallbackNet { default_port: 8080u32 }
      let port = connect_and_parse("ok").use(net_handler)! {
          .UnexpectedToken(pos) => {
              let _ = pos;
              0u32
          },
      }
      let _ = port;
  }
  ```

- **拒绝例（函数签名禁止 `handles` / 错误与效应 ABI 混同）**：
  ```flurry
  -- 拒绝例 1（语法/架构拒绝）：禁止把能力处理子句写到 fn 签名（修正 2026-8-25-handler-capability.fl 旧草稿写法）
  {-
  fn bad_logger_method(body: #Log void) -> void handles (Log) {
      body# { .info(msg) => println("{}", msg), }
  } -- 编译错误：函数签名不允许出现 handles 子句；效应消去由参数与返回类型上的 #Log -> void 体现
  -}

  -- 拒绝例 2（ABI 隔离拒绝）：禁止用错误 catch arm 截取可恢复效应，或用 resumptive 恢复消去 !Err
  {-
  effect ask_id(catch continuation: builtin.Continuation) -> u32;
  fn mix_abis() -> !NetErr u32 {
      ask_id()# match {
          error _ => 0u32,
      } -- 编译错误：ask_id 是 #ask_id 效应而非 !NetErr 错误结果，必须使用 `# { ... }`
  }
  -}
  ```

#### 3.1.2 `DirectPassing`、`effect enum` 与动态 Evidence 路由

- **`^attrs.effects.direct_passing`（`__flurry_direct_passing: true`）**：
  1. 仅合法修饰**零参数且非 `async`** 的同步操作（含 `effect enum` 单个 variant）。
  2. 安装处在其外层环境中**精确求值一次**提供分支并写入槽位；即便主体从未发出请求，安装求值亦不可被 DCE 消除；多次请求读取同一槽位（非 `Copy` 返回类型受单次消费资源约束，不得复制所有权）。
  3. 动态分发/可恢复恢复点处，主体不可跨挂起点缓存外层直接传值槽位（恢复后必须重读当前外层供应，见 [direct-passing.md](../../docs/algebraic-effect-and-delimited-continuation/direct-passing.md)）。
  - **拒绝例（非法 `direct_passing` 声明）**：
    ```flurry
    {-
    ^attrs.effects.direct_passing
    effect invalid_with_arg(key: str) -> u32; -- 编译错误（attributes/effects.rs）：direct_passing 禁止带参数

    ^attrs.effects.direct_passing
    async effect invalid_async() -> u32; -- 编译错误：direct_passing 禁止修饰 async effect
    -}
    ```
- **`effect enum` 组与部分处理转发**：
  - `effect enum Ops { op_a() -> u32, op_b(x: u32) -> void }` 不融合为单一大签名标签，而是在 elaboration 阶段按独立操作实例注册，行中可单独引用 `#Ops.op_a` 或整组 `#Ops`（依 [rows.md](../../docs/type/rows.md) 保持同名 occurrence 顺序，禁止随意置换未证明不等的符号标签）。
  - 编译期闭行已知安装点优化降码为静态直调 / evidence 槽位传参；不可内联或跨动态多态调用边界维护固定布局的 `EvidenceTable`（按 occurrence 槽位与操作实例 ID 匹配，维持分支在安装者外层环境执行的不变式）。

#### 3.1.3 `Panic` / `Abort` 完整运行时与清理契约（解决 `library/README.md:395` 与 `07` Q1 阻塞）

依据 [effect-abi.md:195](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md) 与 [library/README.md:393-395](../../library/README.md)，Flurry calls 在资源检查后返回 LLVM 结构 `{ T, ptr }`，其中控制字 `ptr` 为：
- `null`：正常返回（错误结果 `!E T` 的标签+载荷完全位于 `T` 内）；
- `ptr == 1`：契约/断言失败或 `Panic` 隐藏中止展开信号；
- 对齐地址：非局部退出目标或任务取消上下文地址。

据此制定完整的 `Panic` / `Abort` 三层契约：

```flurry
-- 归属：std.process / builtin（核心原语）
pub struct CallerLocation {
    file: str,
    line: u32,
    column: u32,
}

pub struct PanicInfo {
    message: str,
    location: ?CallerLocation,
    is_unwind: bool,
}

pub enum PanicStrategy {
    unwind,
    abort,
}

-- 底层永不返回入口（均保持空效应行，不污染任何函数类型签名！）
-- 注：CallerLocation 捕获依赖在 official-attribute-keys.md 登记 __flurry_caller_location
pub fn panic(message: str) -> NoReturn;
pub fn abort() -> NoReturn; -- 立即触发 SIGABRT / @llvm.trap()，绝不执行任何栈上 Drop
pub fn set_panic_hook(hook: fn(*PanicInfo)) -> void;
```

1. **可控逆序展开（默认 `unwind`）vs 极速中止（`abort`）**：
   - 在 `unwind` 策略下，`panic(message)` 打印结构化诊断与可选位置信息后激活控制字 `1`：沿调用链回溯每一帧并逆序执行剩余 `Once` 资源的 `Drop`，保证文件句柄、堆块、锁守卫与 `TypedContinuation` 放弃清理得到兑现。
   - **不可跨越的三大安全围栏**：
     1. **C ABI 边界屏障**：若展开遇到 `^attrs.abi(.c)` 等 C 帧边界，C 调用包装立即 trap（阻止未知外部帧泄漏）；
     2. **析构内二次失控屏障**（依 `effect-abi.md:195`）：若正在清理中的 `Drop` 自身再次返回非空控制字（如双 panic），运行时立即终止展开并 `abort()`；
     3. **`panic = .abort` 构建配置**：嵌入式或极限体积构建模式在 `build` 配置选择 `.abort` 时，`panic` 直接调用 `abort()` 跳过控制字清理检查。
2. **线程 / 异步任务隔离**：工作线程或 `async` 子任务在顶层 `JoinHandle` / `ThreadJoinHandle` 入口设置监督边界（拦截控制字 `1` 转换为 `TaskErr.Panicked`），**不会静默拖垮其他无关联任务**（未捕获到顶层 `main` 时再打印并非零退出进程）。
3. **编译期 `comptime` 求值语义**：在 `comptime` 中触发 `panic(msg)` 或契约失败，**必须由 VM 立刻转化为明确的编译期错误诊断（同 `meta.compilation.err` 语义）**，禁止延迟到运行时或吞掉。
4. **对 `pure fn` 与 LLVM 属性的影响**：保持 `panic` **空效应行**（否则 `unwrap`/越界检查会感染全程序效应行）；依 [compiler/luna/src/host/llvm/attributes.rs:53-56](../../compiler/luna/src/host/llvm/attributes.rs)，调用会 `trap`/传递控制字 `1` 的路径自动剥夺 LLVM `nounwind`/`speculatable`/`memory(none)` 属性（即只有编译期已证明条件成立、或返回 `?T`/`!E T` 且永不触发 `panic` 的 `pure fn` 才享受无副作用优化属性）。

---

### 3.2 Tier 1 vs Tier 2 效应边界规范与反例（`WL-08-10`）

为彻底贯彻反膨胀原则，区分**控制与并发底座效应（Tier 1）**与**领域及高层效应套件（Tier 2）**：

1. **严格外置至 [10-official-protocol-packages-and-boundary.md](./10-official-protocol-packages-and-boundary.md) Tier 2 的内容**：
   - 日志/追踪/指标：`telemetry` 包（`effect enum { Log, Trace, Metric }`）— **禁止以 `std.log` 形态存在于 `std`**。
   - 伪随机熵源：`random` 包（`RngCore`、`effect enum Random { random_bytes, random_u64 }`）— **`std` 仅在 `std.os` 提供系统调用级 `getrandom` 字节填充原语**。
   - 历法与系统钟表：`datetime` 包（`Calendar`、`TimeZone`、`effect system_clock`）— **`std.time` 仅含 `Duration`、`Instant`、`SystemTime` 与阻塞 `sleep`**。
   - 通用可插拔异步引擎：`async_runtime` 包（`Executor`、`TaskSpawner`、`Reactor`、`Timer`、`AsyncReader`/`AsyncWriter`、`PollState`）— **`std.runtime` 仅保留最小续体、栈池、任务作用域、取消契约与核心 `async effect` 挂起协议**。
   - 通用纯函数式效应套件（`State<S>`、`Reader<R>`、`Writer<W>`、`Yield<T>`、`NonDet`）：**移出 `builtin`/`std` 放入 Tier 2 官方包 `effects_ext`**（待 §5 Q6 最终确认是否需特批保留单向流生成器 `Yield` 给高级迭代器扩展）。

```flurry
-- 拒绝例（严禁写入 Tier 1 std/builtin 的领域效应膨胀）：
{-
-- 错位放置：禁止在 library/builtin 或 library/std 中声明以下领域效应！
pub effect enum Log { info(msg: str), error(msg: str) }
pub effect random_u64() -> u64;
pub effect current_timezone() -> TimeZone;
-}
```

---

### 3.3 限定 Continuation 与三级栈池 ABI（`WL-08-05` ~ `WL-08-07`，Silver P0-3 / P1-1）

#### 3.3.1 `builtin.Continuation` 底层无参数原生协议

依 [multi-prompt-delimited-continuation.md](../../docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md) 与 [effect-abi.md](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)，`builtin.Continuation` **保持无公开类型参数**，是纯正的 `Once`（非 `Copy`、非 `Clone`）控制令牌：

```flurry
-- 归属：builtin.continuation（通过 builtin 门面暴露 builtin.Continuation）
pub struct Continuation {
    raw_handle: usize,
}

-- Continuation 实现 Once（经由 builtin blanket impl Once for T），严格禁止 Copy 与 Clone！
impl Continuation {
    -- unsafe 底层消费并恢复一次，由控制后端切换到绑定栈帧并交付 Y_ptr -> 返回 B_ptr
    -- 契约：只能在原始挂起线程内消费；二次消费、跨线程或类型载荷不匹配属于 UB
    pub unsafe fn resume_raw(self, arg_addr: usize, out_addr: usize) -> usize;

    -- unsafe 主动沿隐藏中止通道触发放弃清理直至限定边界；若穿过 ^attrs.abi(.c) 帧触发 trap
    pub unsafe fn abort_raw(self);

    -- 查询该 continuation 当前是否关联外部 C ABI 帧（用于诊断与静态/调试检验）
    pub pure fn crosses_c_frame(*self) -> bool;

    -- 查询持有此续体的栈链来源池 ID / 所属线程校验（零分配元信息）
    pub pure fn origin_thread_id(*self) -> u64;
}

impl Drop for Continuation {
    fn drop(*self) {
        -- 若未经 resume_raw / abort_raw 消费就离开作用域，自动走隐藏中止通道清理并归还栈槽！
        if self.raw_handle.* != 0usize {
            unsafe {
                let raw = Continuation { raw_handle: self.raw_handle.* }
                self.raw_handle.* = 0usize;
                raw.abort_raw();
            }
        }
    }
}
```

> **安全边界**：`Drop for Continuation` 自动执行放弃清理，即使调用方忽略 `continuation` 参数或提前 `return`/`panic`，栈上挂起的 `Once` 资源也绝对不会泄漏，也不会在不经 `Drop` 清理的前提下把栈槽直接塞回空闲池（`tmp/silver-design/01` §1、`multi-prompt-delimited-continuation.md:114`）。

#### 3.3.2 安全类型化封装 `TypedContinuation`（`std.runtime`）

```flurry
-- 归属：std.runtime
-- Y: 恢复点传入的值类型（操作返回类型）
-- B: 被捕获边界最终产出的结果类型（支持非尾恢复观测 B；尾恢复用 resume 语义）
pub struct TypedContinuation
where
    Y: Type,
    B: Type,
{
    inner: builtin.Continuation,
    _marker: builtin.mem.TypeMarker<Tuple<Y, B>>,
}

impl TypedContinuation<Y, B>
where
    Y: Type,
    B: Type,
{
    -- 仅可由控制后端或编译器 elaboration 在 catch continuation 站点构造
    pub unsafe fn from_raw(inner: builtin.Continuation) -> Self {
        TypedContinuation<Y, B> {
            inner: inner,
            _marker: builtin.mem.TypeMarker {},
        }
    }

    pub fn into_raw(self) -> builtin.Continuation {
        self.inner
    }

    -- 非尾显式恢复：消费唯一所有权，传递 value: Y 到挂起点，并等待边界产出 B 返回分支
    -- 注意：`resume` 为词法保留关键字（仅用于 `resume;`/`resume expr;` 语句恢复），故库级恢复方法命名为 `resume_with`；
    -- 必须在原挂起线程内执行；挂起点内部 handler 自动重装，外层由当前恢复点供应
    pub fn resume_with(self, value: Y) -> B {
        var in_slot = value;
        var out_slot = builtin.mem.MaybeUninit<B>.uninit();
        let in_addr = in_slot.ref'bitvec.as(usize);
        let out_addr = out_slot.ref'bitvec.as(usize);
        let k = self.into_raw();
        let ctrl = unsafe { k.resume_raw(in_addr, out_addr) }
        if ctrl != 0usize {
            -- 恢复路径内部发生未处理控制字（如 panic 展开信号 1 或取消目标）
            std.process.panic("suspended continuation terminated abnormally");
        }
        unsafe { out_slot.assume_init() }
    }

    -- 显式放弃并沿隐藏中止通道执行挂起栈上的 Drop 清理
    pub fn abort(self) {
        let k = self.into_raw();
        unsafe { k.abort_raw(); }
    }
}
```

#### 3.3.3 `StackPool` 协议与三级栈存储架构（Silver P0-3 & P1-1）

依据 [effect-abi.md#显式栈池实例](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md) 与 [tmp/silver-design/01 §2.3](../silver-design/01-stack-instance-and-event-loop.md)，库层抽象**统一由池实例供应执行存储，每段栈保留自己的分配来源，完成所有清理后方能归还原池**：

```flurry
use builtin.{bool, Type, u64, usize, void}
use std.collections.Vec;
use std.mem.{Allocator, DefaultAllocator}

pub enum StackPoolErr {
    OutOfMemory,
    StackSizeOverflow(usize, usize), -- (required_bytes, pool_slot_bytes)
    InvalidSlotAlignment,
    GuardPageSetupFailed(std.io.IoErr),
    PoolExhausted,
}

-- 不透明槽位描述符（绑定于分配它的 StackPool；Once，绝不 Copy/Clone）
pub struct StackSlot {
    stack_base: usize,
    stack_top: usize,
    slot_bytes: usize,
    pool_id: u64,
    token: usize,
}

-- 核心栈池 trait：同步 .use(pool) 或 runtime 内部均通过本协议取得与归还稳定栈
pub trait StackPool {
    fn acquire_stack(*self, min_bytes: usize) -> !StackPoolErr StackSlot;
    unsafe fn release_stack(*self, slot: StackSlot);
    fn pool_id(*self) -> u64;
    fn slot_capacity_bytes(*self) -> ?usize;
}

-- Tier 1：定长分级有界栈池（Silver P0-3 核心目标！）
-- 严格遵守运行时依赖 allocator 范式：用 DefaultAllocator 时无额外分配器字段开销；
-- Debug 可选 redzone canary 检测；Release 依赖 meta.runtime.bounded_stack_bytes 证明零越界系统调用
pub struct BoundedStackPool
where
    slot_bytes: usize,
    .A: Type = DefaultAllocator,
    requires slot_bytes >= 128usize and (slot_bytes % 16usize) == 0usize and A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    pool_uid: u64,
    free_list_head: usize,
    slab_head: usize,
    slab_slots: usize,
    active_count: usize,
    total_count: usize,
}

impl BoundedStackPool<slot_bytes, .A = A, .allocator = alloc>
where
    slot_bytes: usize,
    A: Type,
    requires slot_bytes >= 128usize and (slot_bytes % 16usize) == 0usize and A :- Allocator,
    quote alloc: lift *A,
{
    pub fn new(slots_per_slab: usize) -> !StackPoolErr Self;
    pub fn preallocate_slabs(*self, slab_count: usize) -> !StackPoolErr void;
    pub fn active_slots(*self) -> usize;
    pub fn total_slots(*self) -> usize;
}

impl StackPool for BoundedStackPool<slot_bytes, .A = A, .allocator = alloc>
where
    slot_bytes: usize,
    A: Type,
    requires slot_bytes >= 128usize and (slot_bytes % 16usize) == 0usize and A :- Allocator,
    quote alloc: lift *A,
{
    fn acquire_stack(*self, min_bytes: usize) -> !StackPoolErr StackSlot;
    unsafe fn release_stack(*self, slot: StackSlot);
    fn pool_id(*self) -> u64;
    fn slot_capacity_bytes(*self) -> ?usize;
}

impl Drop for BoundedStackPool<slot_bytes, .A = A, .allocator = alloc>
where
    slot_bytes: usize,
    A: Type,
    requires slot_bytes >= 128usize and (slot_bytes % 16usize) == 0usize and A :- Allocator,
    quote alloc: lift *A,
{
    -- 前置：所有从本池借出的 StackSlot 及关联 Continuation 均已完成清理并归还（active_count == 0）
    fn drop(*self);
}

-- Tier 2：通用保护栈池（支持未知递归/动态闭包；由 OS 虚拟内存保留 + 按需提交 + guard page 保护）
pub struct GuardedStackPool
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    pool_uid: u64,
    default_stack_bytes: usize,
    guard_bytes: usize,
    free_slots: Vec<StackSlot, .A = A, .allocator = allocator>,
    active_count: usize,
}

impl GuardedStackPool<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    pub fn new(default_stack_bytes: usize) -> !StackPoolErr Self;
}

impl StackPool for GuardedStackPool<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    fn acquire_stack(*self, min_bytes: usize) -> !StackPoolErr StackSlot;
    unsafe fn release_stack(*self, slot: StackSlot);
    fn pool_id(*self) -> u64;
    fn slot_capacity_bytes(*self) -> ?usize;
}

impl Drop for GuardedStackPool<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    -- 前置：所有从本池借出的 StackSlot 及关联 Continuation 均已完成清理并归还（active_count == 0）
    -- 遍历 free_slots 解除 OS 虚拟内存保留与 guard page 保护，再由 Vec 经 alloc 释放槽位索引缓冲
    fn drop(*self);
}
```

- **与 `meta.runtime.bounded_stack_bytes` 及 `.use(pool)` 联动**：
  1. 编译器处理 `.use(pool)` 时，若静态已知 `pool` 类型为 `BoundedStackPool<slot_bytes, ...>`（或带 `^attrs.effects.stack_strategy(.bounded_pool)` 标注），在编译期调用 `meta.runtime.bounded_stack_bytes(body_fn)`（`05` `WL-05-31`）：
     - 若返回 `bound?` 且 `bound <= slot_bytes`：零 guard page 静态直分入该池；
     - 若返回 `null`（存在无界递归或未定界间接调用）或 `bound > slot_bytes`：**编译期诊断拒绝**，引导改用更大档位或 `GuardedStackPool`。
  2. **多池跨界复用不变式**：外层安装 `.use(pool_a)` 挂起后，即使被转交到由 `.use(pool_b)` 驱动的调度器恢复，该段捕获栈仍记录 `pool_id == pool_a.pool_id()`，在完成清理后精确归还 `pool_a`（`effect-abi.md:56`）。

- **正例与拒绝例（池化栈依赖与 C 帧屏障）**：
  ```flurry
  -- 正例：同步生成器通过 .use(pool) 接入独立 BoundedStackPool，无需异步运行时
  effect yield_u32(val: u32, catch k: builtin.Continuation);

  fn emit_three() -> #yield_u32 void {
      yield_u32(1u32)#;
      yield_u32(2u32)#;
      yield_u32(3u32)#;
  }

  test {
      var pool = BoundedStackPool<512usize>.new(16usize)! {
          catch _ => return,
      }
      var sum: u32 = 0u32;
      (emit_three()# {
          yield_u32(val, k) => {
              sum = sum + val;
              resume;
          },
      }).use(pool);
      let _ = sum; -- 6u32，pool 零遗留活性槽位
  }

  -- 拒绝例 1（栈池生命期短于逃逸 continuation）：pool 先于持有未消费 K 的结构体析构
  {-
  fn reject_dangling_pool() -> TypedContinuation<void, void> {
      var local_pool = BoundedStackPool<512usize>.new(4usize)! { catch _ => std.process.abort(), }
      var captured: ?TypedContinuation<void, void> = null;
      (emit_three()# {
          yield_u32(_, k) => {
              captured = TypedContinuation<void, void>.from_raw(k);
              ()
          },
      }).use(local_pool);
      -- 编译错误：captured 持有的栈槽依赖于局部绑定 local_pool，local_pool 在返回前退出作用域
      captured!
  }
  -}

  -- 拒绝例 2（跨 C 帧放弃 continuation）：C ABI 栈帧跨挂起点并在放弃时触发 trap
  {-
  ^attrs.abi(.c)
  pub unsafe fn c_reenter(cb: ^attrs.abi(.c) fn()) -> void;

  fn reject_c_frame_abort(pool: *GuardedStackPool) {
      -- 若 C 帧内部回调 Flurry 代码并跨越 yield_u32 挂起，随后 handler 丢弃 k（触发隐藏中止），
      -- 编译器无法静态排除 C 帧时保留运行时检查，实际放弃越过 C 帧屏障立刻 trap（effect-abi.md:66）
  }
  -}
  ```

---
### 3.4 结构化 `async effect` 运行时（`std.runtime`，`WL-08-08` ~ `WL-08-09`）

#### 3.4.1 `TaskContext`、`CancelToken`、`Future`、`JoinHandle` 与 `TaskScope`

依据 [docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md](../../docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md) 与 [effect-abi.md:199](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)：
- Flurry **没有 `async fn` 关键字**；异步挂起与子任务派生通过 **`async effect`** 声明与 `.use(runtime)` 调度范围表达。
- 请求点在调用时刻立即登记子工作并交付类型化 `Future<T>`；**丢弃 `Future` 不会自动取消子工作，所属 `TaskScope` 安装边界承担结构化完成/取消与释放栈存储的责任**。

```flurry
-- 归属：std.runtime
use builtin.{bool, Drop, Type, u64, usize, void}
use std.collections.Vec;
use std.mem.{AllocErr, Allocator, DefaultAllocator}
use std.sync.{Send, Sync}
use std.sync.atomic.AtomicBool;
use std.time.Duration;

pub newtype TaskId = u64;

pub enum CancelErr {
    Cancelled,
    TimedOut,
    ScopeClosed,
}

pub enum TaskErr {
    Cancelled,
    Panicked,
    RuntimeShutdown,
    Pool(StackPoolErr),
    Alloc(AllocErr),
}

pub enum RuntimeErr {
    ExecutorShutdown,
    QueueFull,
    Pool(StackPoolErr),
    Alloc(AllocErr),
    Io(std.io.IoErr),
}

pub enum FutureState {
    pending,
    completed,
    cancelled,
}

-- 对应 effect-abi.md:199 LLVM host 隐式传递的任务上下文布局：
-- 至少包含原子取消标志与当前任务构建时选定的栈池指针
pub struct TaskContext {
    task_id: TaskId,
    cancel_requested: AtomicBool,
    stack_pool_ptr: usize, -- *dyn StackPool 的类型擦除/原生入口地址
}

impl TaskContext {
    pub fn current() -> ?*TaskContext;
    pub fn is_cancelled(*self) -> bool;
    pub fn request_cancel(*self);
    pub unsafe fn stack_pool_addr(*self) -> usize;
}

-- 可克隆、跨任务传递的结构化取消令牌（基于原子状态与 allocator 分配的等待链表）
pub struct CancelToken
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    state_addr: usize,
}

impl CancelToken<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn new() -> !AllocErr Self;
    pub fn child_token(*self) -> !AllocErr Self;
    pub fn cancel(*self);
    pub fn is_cancelled(*self) -> bool;
    pub fn clone_handle(*self) -> Self;
    pub fn check(*self) -> !CancelErr void;
}

-- 语言核心异步控制与等待挂起效应（具体后端调度由 `.use(runtime)` 提供）
pub effect enum AsyncCore {
    park_slot(slot_addr: usize), -- 同步可恢复等待效应：未就绪时交出当前 continuation，完成事件触发后由调度器恢复
    async yield_now(),
    async sleep_for(duration: Duration) -> !CancelErr void,
    async check_cancelled(),
}

-- 核心 Future / Awaitable 协议：编译器将后缀语法 `.await` 降码至 `Awaitable.poll_await`（`await` 为词法保留字）
-- 依 async-algebraic-effect.md:33，结果未就绪时必须通过 runtime 安装的可恢复效应（#AsyncCore）挂起当前计算
pub trait Awaitable {
    assoc Output: Type;

    fn is_ready(*self) -> bool;
    fn poll_await(self) -> #AsyncCore Output;
}

-- async effect 请求返回的唯一所有权结果句柄（Once，不提供 Copy/Clone）
-- 与所属子工作通过共享原子状态计数协同销毁结果槽，严格经由 quote .allocator 回收
pub struct Future
where
    T: Type,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    slot_addr: usize,
    _marker: builtin.mem.TypeMarker<T>,
}

impl Awaitable for Future<T, .A = A, .allocator = alloc>
where
    T: Type,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    assoc Output: Type = !TaskErr T;

    fn is_ready(*self) -> bool;
    fn poll_await(self) -> #AsyncCore !TaskErr T;
}

impl Drop for Future<T, .A = A, .allocator = alloc>
where
    T: Type,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    -- 仅减少结果槽位读者计数；若任务已完成且仅剩本端，则经 alloc 释放槽位；不取消在途工作
    fn drop(*self);
}

-- 显式可取消的任务句柄（由 TaskScope.spawn 返回；Once）
pub struct JoinHandle
where
    T: Type,
    requires T :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    task_id: TaskId,
    future: Future<T, .A = A, .allocator = allocator>,
    cancel_flag_addr: usize,
}

impl JoinHandle<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Send,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn id(*self) -> TaskId;
    pub fn cancel(*self);
    pub fn is_finished(*self) -> bool;
    pub fn join(self) -> !TaskErr T;
}

-- 结构化并发作用域（Nursery）：
-- 1) 正常退出：按 LIFO/注册序等待全部尚未结束的衍生子任务完成
-- 2) 提前退出 / 错误 / Panic：先向所有子任务发取消并沿隐藏中止通道等待清理完成，再释放所依赖的栈池与局部环境
pub struct TaskScope
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    scope_id: u64,
    children: Vec<usize, .A = A, .allocator = allocator>,
    cancel_token: CancelToken<.A = A, .allocator = allocator>,
    aborted: bool,
}

impl TaskScope<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn spawn(*self, body: %c fn() -> #AsyncCore T) -> !TaskErr JoinHandle<T, .A = A, .allocator = alloc>
    where
        T: Type,
        requires T :- Send,
        c: Type.vars.Closure;

    pub fn cancel_all(*self);
    pub fn cancel_token(*self) -> CancelToken<.A = A, .allocator = alloc>;
}

impl Drop for TaskScope<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    -- 强不变量：无论正常或错误路径退出，离开 TaskScope 前必须等待全部衍生任务停止并回收栈槽
    fn drop(*self);
}
```

#### 3.4.2 最小并发原语与 Tier 2 `async_runtime` 调度器桥接

为兼顾开箱即用的结构化并发与“具体 `io_uring`/epoll 调度器外置到 Tier 2/3”的红线，`std.runtime` 仅定义语言级 `async effect` 核心控制操作，并与 [10-official-protocol-packages-and-boundary.md](./10-official-protocol-packages-and-boundary.md) §3.5（`async_runtime`）中的 `Executor` / `TaskSpawner` / `Reactor` / `Timer` / `AsyncReader` / `AsyncWriter` / `AsyncRuntimeOps` 无缝对接：

```flurry
-- 归属：std.runtime（基于 3.4.1 定义的 AsyncCore / Future / Awaitable / TaskScope 与限定 continuation）
use builtin.{Tuple, Type, void}
use std.mem.{Allocator, DefaultAllocator}
use std.sync.{Send, Sync}
use std.time.Duration;

-- 核心并发组合子（全部基于 AsyncCore / Future / TaskScope 与限定 continuation 构建）
pub fn scoped(entry: %c fn(*TaskScope<.A = A, .allocator = allocator>) -> #AsyncCore R) -> !TaskErr R
where
    R: Type,
    c: Type.vars.Closure,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

pub fn yield_now() -> #AsyncCore !TaskErr void {
    AsyncCore.yield_now()#.await
}

pub fn sleep(duration: Duration) -> #AsyncCore ![CancelErr, TaskErr] void {
    -- `AsyncCore.sleep_for(duration)#.await` 返回 `!TaskErr (!CancelErr void)`：
    -- 先通过后缀 `!` 向外传播外层 `TaskErr`，再将内层 `!CancelErr void` 传播合并进 `![CancelErr, TaskErr]`
    let inner: !CancelErr void = AsyncCore.sleep_for(duration)#.await!;
    inner!
}

pub fn join_pair(
    left: Future<L, .A = AL, .allocator = alloc_l>,
    right: Future<R, .A = AR, .allocator = alloc_r>,
) -> #AsyncCore Tuple<!TaskErr L, !TaskErr R>
where
    L: Type,
    R: Type,
    AL: Type,
    requires AL :- Allocator + Sync,
    quote alloc_l: lift *AL,
    AR: Type,
    requires AR :- Allocator + Sync,
    quote alloc_r: lift *AR;

pub enum Select2
where
    L: Type,
    R: Type,
    .AL: Type = DefaultAllocator,
    requires AL :- Allocator + Sync,
    quote .alloc_l: lift *AL = DefaultAllocator.GLOBAL.ref,
    .AR: Type = DefaultAllocator,
    requires AR :- Allocator + Sync,
    quote .alloc_r: lift *AR = DefaultAllocator.GLOBAL.ref,
{
    first(!TaskErr L, Future<R, .A = AR, .allocator = alloc_r>),
    second(Future<L, .A = AL, .allocator = alloc_l>, !TaskErr R),
}

-- 与取消/继续权争用保持唯一所有权：获胜分支交回结果，未完成 Future 交还调用方决定继续或取消
pub fn select_pair(
    left: Future<L, .A = AL, .allocator = alloc_l>,
    right: Future<R, .A = AR, .allocator = alloc_r>,
) -> #AsyncCore Select2<L, R, .AL = AL, .alloc_l = alloc_l, .AR = AR, .alloc_r = alloc_r>
where
    L: Type,
    R: Type,
    AL: Type,
    requires AL :- Allocator + Sync,
    quote alloc_l: lift *AL,
    AR: Type,
    requires AR :- Allocator + Sync,
    quote alloc_r: lift *AR;

-- 优先返回最先完成的结果；对另一条 Future 发取消并等待其清理完成后再返回（绝不泄露未归属孤儿任务）
pub fn race_pair(
    scope: *TaskScope<.A = A, .allocator = alloc>,
    left: Future<T, .A = A, .allocator = alloc>,
    right: Future<T, .A = A, .allocator = alloc>,
) -> #AsyncCore !TaskErr T
where
    T: Type,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A;

-- 为异步计算附加截止时间：在所属 TaskScope 内并发启动 body 与 sleep_for 计时子任务，
-- 超时后向未完成子任务发取消并等待其清理退出，返回 CancelErr.TimedOut（绝不泄露孤儿任务）
pub fn timeout(
    scope: *TaskScope<.A = A, .allocator = alloc>,
    duration: Duration,
    body: %c fn() -> #AsyncCore T,
) -> #AsyncCore ![CancelErr, TaskErr] T
where
    T: Type,
    requires T :- Send,
    c: Type.vars.Closure,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A;
```

- **与 Tier 2 `async_runtime` 的互操作契约**：
  - Tier 2 包 `async_runtime`（`10` §3.5）定义 `Executor`、`TaskSpawner`、`Reactor`、`Timer`、`AsyncReader`、`AsyncWriter`、`PollState` 与 `WakerHandle`；
  - 任意 Tier 3 后端（如 `io_uring_rt`、`epoll_rt` 或 `silver` UI `Executor.tick()` 主循环）只需在内部持有一个 `std.runtime.BoundedStackPool` 或 `GuardedStackPool`，并通过 `HandleEffect` 处理 `AsyncCore` 与 `async_runtime.AsyncRuntimeOps` 的 `async effect`，即可让 `.use(runtime)` 自动接通调度与栈存储（见 `async-algebraic-effect.md:43`、`effect-abi.md:52`）。

- **正例与拒绝例（结构化并发所有权边界）**：
  ```flurry
  -- 拒绝例 1（伪造 async fn 关键字）：Flurry 无 async fn 语法
  {-
  pub async fn invalid_async_fn() -> u32 { 1u32 } -- 语法错误：async 仅修饰 effect 与 case_arm 模式
  -}

  -- 拒绝例 2（子任务悬垂借用父作用域局部变量）：TaskScope 不允许脱离局部生命周期指针逃逸
  {-
  fn reject_dangling_borrow() -> !TaskErr JoinHandle<u32> {
      var local_x = 10u32;
      let ptr = local_x.ref;
      var handle_out: ?JoinHandle<u32> = null;
      scoped(|scope| {
          let h = scope.spawn(|| ptr.*)!;
          handle_out = h; -- 即使写入外层变量，scoped 退出时的 Drop 必先 join/取消，且 ptr 不可比 local_x 活更久
          ()
      })!;
      handle_out!
  }
  -}
  ```

---

### 3.5 `std.sync`：线程安全契约、原子原语、锁、通道与作用域线程（`WL-08-11` ~ `WL-08-14`）

#### 3.5.1 `Send` / `Sync` 与完整 `std.sync.atomic`（解除 `07` `Arc` 阻塞）

```flurry
-- 归属：std.sync（及 std.sync.atomic 子模块）
use builtin.{bool, i8, i16, i32, i64, isize, Type, u8, u16, u32, u64, usize}

-- Send: 跨线程安全转移所有权；Sync: 跨线程通过只读/共享 *T 安全访问（要求底层遵守同步协议）
pub trait Send {}
pub trait Sync {}

-- 规则约定（由编译器按结构化组合自动推导 + 显式否定退出）：
--   1) 标量（bool/u8..u128/i8..i128/f16..f64/char/void/NoReturn/str 等底层共享只读或值类型）均 Send + Sync
--   2) struct/enum/tuple 当且仅当全部字段均为 Send（或 Sync）时实现 Send（或 Sync）
--   3) 显式否定：std.mem.Rc / std.mem.RcWeak / builtin.Continuation / TypedContinuation / MutexGuard
--      一律明确禁止 Send 与 Sync（multi-prompt-delimited-continuation.md:61 禁止跨线程恢复）

pub enum AtomicOrdering {
    relaxed,
    consume,
    acquire,
    release,
    acq_rel,
    seq_cst,
}

pub pure comptime fn is_atomic_scalar(t: Type) -> bool {
    t == bool or t == i8 or t == u8 or t == i16 or t == u16 or t == i32 or t == u32 or t == i64 or t == u64 or t == isize or t == usize
}

pub pure comptime fn is_atomic_int(t: Type) -> bool {
    t == i8 or t == u8 or t == i16 or t == u16 or t == i32 or t == u32 or t == i64 or t == u64 or t == isize or t == usize
}

pub struct CompareExchangeErr where T: Type {
    current: T,
}

-- 支持锁自由硬件原子的底层标量约束（bool, i8..i64, u8..u64, isize, usize）
pub trait AtomicScalar
where
    requires is_atomic_scalar(Self),
{}

pub struct Atomic
where
    T: Type,
    requires T :- AtomicScalar,
{
    inner: T,
}

impl Atomic<T>
where
    T: Type,
    requires T :- AtomicScalar,
{
    pub pure fn new(value: T) -> Self;
    pub fn into_inner(self) -> T;
    pub fn get_mut(*self) -> *T; -- 独占 *Atomic 时无需原子屏障

    pub fn load(*self, order: AtomicOrdering) -> T;
    pub fn store(*self, val: T, order: AtomicOrdering);
    pub fn swap(*self, val: T, order: AtomicOrdering) -> T;
    pub fn compare_exchange(*self, current: T, new: T, success: AtomicOrdering, failure: AtomicOrdering) -> !CompareExchangeErr<T> T;
    pub fn compare_exchange_weak(*self, current: T, new: T, success: AtomicOrdering, failure: AtomicOrdering) -> !CompareExchangeErr<T> T;
    pub fn fetch_update(*self, set_order: AtomicOrdering, fetch_order: AtomicOrdering, f: %c fn(T) -> ?T) -> !CompareExchangeErr<T> T
    where
        c: Type.vars.Closure;
}

impl Atomic<T>
where
    T: Type,
    requires T :- AtomicScalar,
    requires is_atomic_int(T),
{
    pub fn fetch_add(*self, val: T, order: AtomicOrdering) -> T;
    pub fn fetch_sub(*self, val: T, order: AtomicOrdering) -> T;
    pub fn fetch_and(*self, val: T, order: AtomicOrdering) -> T;
    pub fn fetch_or(*self, val: T, order: AtomicOrdering) -> T;
    pub fn fetch_xor(*self, val: T, order: AtomicOrdering) -> T;
    pub fn fetch_max(*self, val: T, order: AtomicOrdering) -> T;
    pub fn fetch_min(*self, val: T, order: AtomicOrdering) -> T;
}

pub struct AtomicPtr where T: Type {
    ptr_addr: Atomic<usize>,
    _marker: builtin.mem.TypeMarker<T>,
}

impl AtomicPtr<T> where T: Type {
    pub pure fn new(ptr: *T) -> Self;
    pub fn load(*self, order: AtomicOrdering) -> *T;
    pub fn store(*self, ptr: *T, order: AtomicOrdering);
    pub fn swap(*self, ptr: *T, order: AtomicOrdering) -> *T;
    pub fn compare_exchange(*self, current: *T, new: *T, success: AtomicOrdering, failure: AtomicOrdering) -> !CompareExchangeErr<*T> *T;
    pub fn compare_exchange_weak(*self, current: *T, new: *T, success: AtomicOrdering, failure: AtomicOrdering) -> !CompareExchangeErr<*T> *T;
}

pub fn fence(order: AtomicOrdering);
pub fn compiler_fence(order: AtomicOrdering);

-- 常用别名（与 07 Arc<T> 及 std.runtime/OnceLock 直接对接）
pub typealias AtomicBool = Atomic<bool>;
pub typealias AtomicI8 = Atomic<i8>;
pub typealias AtomicU8 = Atomic<u8>;
pub typealias AtomicI16 = Atomic<i16>;
pub typealias AtomicU16 = Atomic<u16>;
pub typealias AtomicI32 = Atomic<i32>;
pub typealias AtomicU32 = Atomic<u32>;
pub typealias AtomicI64 = Atomic<i64>;
pub typealias AtomicU64 = Atomic<u64>;
pub typealias AtomicIsize = Atomic<isize>;
pub typealias AtomicUsize = Atomic<usize>;
```

- **内存序错误拒绝提示**：`AtomicOrdering` 禁止对 `load` 传 `release`/`acq_rel`、对 `store` 传 `acquire`/`acq_rel`、对 `compare_exchange` 的 `failure` 强于 `success` 或传 `release`/`acq_rel`（在静态分支或调试检查期直接拒绝）。

#### 3.5.2 `Mutex`、`RwLock`、`Condvar`、`OnceLock`、`LazyLock`、`Barrier`、`Semaphore`

```flurry
-- 归属：std.sync
use builtin.{bool, Drop, Tuple, Type, usize, void}
use builtin.mem.MaybeUninit;
use std.sync.{Send, Sync}
use std.sync.atomic.{AtomicBool, AtomicU8, AtomicU32, AtomicUsize}
use std.time.Duration;

pub enum TryLockErr {
    WouldBlock,
    Poisoned,
}

pub struct PoisonErr where G: Type {
    guard: G,
}

impl PoisonErr<G> where G: Type {
    pub fn into_inner(self) -> G {
        self.guard
    }
}

pub struct Mutex
where
    T: Type,
    requires T :- Send,
{
    state: AtomicU32,
    poisoned: AtomicBool,
    data: T,
}

-- MutexGuard 实现 Once，禁止 Send 与 Copy/Clone（OS 互斥锁通常要求解锁与加锁同线程）
pub struct MutexGuard where T: Type {
    lock: *Mutex<T>,
}

impl Mutex<T>
where
    T: Type,
    requires T :- Send,
{
    pub pure fn new(value: T) -> Self;
    pub fn lock(*self) -> !PoisonErr<MutexGuard<T>> MutexGuard<T>;
    pub fn try_lock(*self) -> !TryLockErr MutexGuard<T>;
    pub fn is_poisoned(*self) -> bool;
    pub fn clear_poison(*self);
    pub fn into_inner(self) -> !PoisonErr<T> T;
    pub fn get_mut(*self) -> !PoisonErr<*T> *T;
}

impl MutexGuard<T> where T: Type {
    pub fn as_ptr(*self) -> *T;
    pub fn with_ref(*self, f: %c fn(*T) -> R) -> R
    where
        R: Type,
        c: Type.vars.Closure;
}

impl Drop for MutexGuard<T> where T: Type {
    -- 若持有期间当前任务正在控制字 1（Panic unwind）中退出，则标记 poisoned = true 后解锁
    fn drop(*self);
}

pub struct RwLock
where
    T: Type,
    requires T :- Send + Sync,
{
    state: AtomicU32,
    writer_waiting: AtomicU32,
    poisoned: AtomicBool,
    data: T,
}

pub struct RwLockReadGuard where T: Type {
    lock: *RwLock<T>,
}

pub struct RwLockWriteGuard where T: Type {
    lock: *RwLock<T>,
}

impl RwLock<T>
where
    T: Type,
    requires T :- Send + Sync,
{
    pub pure fn new(value: T) -> Self;
    pub fn read(*self) -> !PoisonErr<RwLockReadGuard<T>> RwLockReadGuard<T>;
    pub fn try_read(*self) -> !TryLockErr RwLockReadGuard<T>;
    pub fn write(*self) -> !PoisonErr<RwLockWriteGuard<T>> RwLockWriteGuard<T>;
    pub fn try_write(*self) -> !TryLockErr RwLockWriteGuard<T>;
    pub fn into_inner(self) -> !PoisonErr<T> T;
}

impl RwLockReadGuard<T> where T: Type {
    pub fn as_ptr(*self) -> *T;
}

impl Drop for RwLockReadGuard<T> where T: Type {
    fn drop(*self);
}

impl RwLockWriteGuard<T> where T: Type {
    pub fn as_ptr(*self) -> *T;
}

impl Drop for RwLockWriteGuard<T> where T: Type {
    fn drop(*self);
}

pub struct WaitTimeoutResult {
    timed_out_flag: bool,
}

impl WaitTimeoutResult {
    pub pure fn timed_out(self) -> bool {
        self.timed_out_flag
    }
}

pub struct Condvar {
    seq: AtomicU32,
}

impl Condvar {
    pub pure fn new() -> Self;
    pub fn wait(*self, guard: MutexGuard<T>) -> !PoisonErr<MutexGuard<T>> MutexGuard<T> where T: Type;
    pub fn wait_while(*self, guard: MutexGuard<T>, cond: %c fn(*T) -> bool) -> !PoisonErr<MutexGuard<T>> MutexGuard<T>
    where
        T: Type,
        c: Type.vars.Closure;
    pub fn wait_timeout(*self, guard: MutexGuard<T>, dur: Duration) -> !PoisonErr<Tuple<MutexGuard<T>, WaitTimeoutResult>> Tuple<MutexGuard<T>, WaitTimeoutResult>
    where
        T: Type;
    pub fn notify_one(*self);
    pub fn notify_all(*self);
}

pub struct OnceInitErr where T: Type {
    value: T,
}

pub struct OnceLock where T: Type {
    state: AtomicU8, -- 0: uninit, 1: initializing, 2: initialized
    value: MaybeUninit<T>,
}

impl OnceLock<T> where T: Type {
    pub pure fn new() -> Self;
    pub fn get(*self) -> ?*T;
    pub fn set(*self, value: T) -> !OnceInitErr<T> void; -- 已初始化则通过 OnceInitErr 退还仿射 value
    pub fn get_or_init(*self, f: %c fn() -> T) -> *T where c: Type.vars.Closure;
    pub fn get_or_try_init(*self, f: %c fn() -> !E T) -> !E *T where E: Type, c: Type.vars.Closure;
    pub fn take(*self) -> ?T;
}

impl Drop for OnceLock<T> where T: Type {
    fn drop(*self);
}

pub struct LazyLock
where
    T: Type,
    c: Type.vars.Closure,
{
    once: OnceLock<T>,
    init: MaybeUninit<%c fn() -> T>,
}

impl LazyLock<T, c>
where
    T: Type,
    c: Type.vars.Closure,
{
    pub pure fn new(init_fn: %c fn() -> T) -> Self;
    pub fn force(*self) -> *T;
    pub fn get(*self) -> ?*T;
}

impl Drop for LazyLock<T, c>
where
    T: Type,
    c: Type.vars.Closure,
{
    -- 若已完成初始化，交由 inner OnceLock<T>.drop 析构已初始化的 T；
    -- 若仍未求值完成，显式析构仍驻留在 MaybeUninit 中的初始化闭包（防止捕获 Once 值泄漏）
    fn drop(*self);
}

pub struct BarrierWaitResult {
    is_leader_flag: bool,
}

impl BarrierWaitResult {
    pub pure fn is_leader(self) -> bool {
        self.is_leader_flag
    }
}

pub struct Barrier {
    num_threads: usize,
    count: AtomicUsize,
    generation: AtomicUsize,
}

impl Barrier {
    pub fn new(n: usize) -> Self;
    pub fn wait(*self) -> BarrierWaitResult;
}

pub struct Semaphore {
    permits: AtomicUsize,
    wait_seq: AtomicU32,
}

pub struct SemaphorePermit {
    sem: *Semaphore,
    count: usize,
}

impl Semaphore {
    pub pure fn new(permits: usize) -> Self;
    pub fn acquire(*self) -> SemaphorePermit;
    pub fn acquire_many(*self, n: usize) -> SemaphorePermit;
    pub fn try_acquire(*self) -> ?SemaphorePermit;
    pub fn add_permits(*self, n: usize);
    pub fn available_permits(*self) -> usize;
}

impl SemaphorePermit {
    pub fn forget(self);
}

impl Drop for SemaphorePermit {
    fn drop(*self);
}
```

#### 3.5.3 多模式并发通道（全部携带运行时依赖 Allocator 子句）

所有通道（`mpsc` 多生产者单消费者、`mpmc` 多生产者多消费者、`oneshot` 单发单收、`broadcast` 广播队列）内部缓冲区与共享控制块**绝不硬编码 `SystemAllocator`**，统一接收 `.A` 与 `quote .allocator`：

```flurry
-- 归属：std.sync.mpsc / std.sync.mpmc / std.sync.oneshot / std.sync.broadcast
use builtin.{bool, Clone, Drop, Tuple, Type, u64, usize, void}
use std.mem.{AllocErr, Allocator, DefaultAllocator}
use std.runtime.AsyncCore;
use std.sync.{Send, Sync}
use std.time.Duration;

pub struct SendErr where T: Type {
    value: T, -- 对端关闭时无损交回原仿射值
}

pub enum TrySendErr where T: Type {
    Full(T),
    Disconnected(T),
}

pub enum RecvErr {
    Disconnected,
}

pub enum TryRecvErr {
    Empty,
    Disconnected,
}

pub enum RecvTimeoutErr {
    Timeout,
    Disconnected,
}

-- 1. mpsc 有界与无界通道（支持阻塞与可选非阻塞尝试）
pub struct MpscSender
where
    T: Type,
    requires T :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    shared_addr: usize,
}

pub struct MpscReceiver
where
    T: Type,
    requires T :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    shared_addr: usize,
}

pub fn mpsc_channel() -> !AllocErr Tuple<MpscSender<T, .A = A, .allocator = allocator>, MpscReceiver<T, .A = A, .allocator = allocator>>
where
    T: Type,
    requires T :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

pub fn mpsc_sync_channel(bound: usize) -> !AllocErr Tuple<MpscSender<T, .A = A, .allocator = allocator>, MpscReceiver<T, .A = A, .allocator = allocator>>
where
    T: Type,
    requires T :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

impl MpscSender<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Send,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn send(*self, value: T) -> ![SendErr<T>, AllocErr] void;
    pub fn try_send(*self, value: T) -> ![TrySendErr<T>, AllocErr] void;
    pub fn clone_sender(*self) -> Self;
}

impl MpscReceiver<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Send,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn recv(*self) -> !RecvErr T;
    pub fn try_recv(*self) -> !TryRecvErr T;
    pub fn recv_timeout(*self, timeout: Duration) -> !RecvTimeoutErr T;
    pub fn recv_async(*self) -> #AsyncCore !RecvErr T; -- 协作式 async effect 等待桥接
}

impl Drop for MpscSender<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Send,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    fn drop(*self);
}

impl Drop for MpscReceiver<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Send,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    fn drop(*self);
}

-- 2. mpmc / oneshot / broadcast（共享同一 allocator 透传与引用计数清理不变式）
pub struct MpmcSender
where
    T: Type,
    requires T :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    shared_addr: usize,
}

pub struct MpmcReceiver
where
    T: Type,
    requires T :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    shared_addr: usize,
}

pub struct OneshotSender
where
    T: Type,
    requires T :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    shared_addr: usize,
}

pub struct OneshotReceiver
where
    T: Type,
    requires T :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    shared_addr: usize,
}

pub struct BroadcastSender
where
    T: Type,
    requires T :- Send + Clone,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    shared_addr: usize,
}

pub struct BroadcastReceiver
where
    T: Type,
    requires T :- Send + Clone,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    shared_addr: usize,
    cursor: u64,
}

pub fn mpmc_channel(capacity: usize) -> !AllocErr Tuple<MpmcSender<T, .A = A, .allocator = allocator>, MpmcReceiver<T, .A = A, .allocator = allocator>>
where
    T: Type,
    requires T :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

pub fn oneshot_channel() -> !AllocErr Tuple<OneshotSender<T, .A = A, .allocator = allocator>, OneshotReceiver<T, .A = A, .allocator = allocator>>
where
    T: Type,
    requires T :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

pub fn broadcast_channel(capacity: usize) -> !AllocErr Tuple<BroadcastSender<T, .A = A, .allocator = allocator>, BroadcastReceiver<T, .A = A, .allocator = allocator>>
where
    T: Type,
    requires T :- Send + Clone,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

impl OneshotSender<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Send,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn send(self, value: T) -> !SendErr<T> void;
    pub fn is_closed(*self) -> bool;
}

impl OneshotReceiver<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Send,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn recv(self) -> !RecvErr T;
    pub fn try_recv(*self) -> !TryRecvErr T;
    pub fn recv_async(self) -> #AsyncCore !RecvErr T;
}

impl MpmcSender<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Send,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn send(*self, value: T) -> !SendErr<T> void;
    pub fn try_send(*self, value: T) -> !TrySendErr<T> void;
    pub fn clone_sender(*self) -> Self;
}

impl MpmcReceiver<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Send,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn recv(*self) -> !RecvErr T;
    pub fn try_recv(*self) -> !TryRecvErr T;
    pub fn clone_receiver(*self) -> Self;
    pub fn recv_async(*self) -> #AsyncCore !RecvErr T;
}

impl BroadcastSender<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Send + Clone,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn send(*self, value: T) -> !SendErr<T> usize;
    pub fn subscribe(*self) -> BroadcastReceiver<T, .A = A, .allocator = alloc>;
}

impl BroadcastReceiver<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Send + Clone,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn recv(*self) -> !RecvErr T;
    pub fn try_recv(*self) -> !TryRecvErr T;
    pub fn recv_async(*self) -> #AsyncCore !RecvErr T;
}

impl Drop for MpmcSender<T, .A = A, .allocator = alloc> where T: Type, requires T :- Send, A: Type, requires A :- Allocator + Sync, quote alloc: lift *A { fn drop(*self); }
impl Drop for MpmcReceiver<T, .A = A, .allocator = alloc> where T: Type, requires T :- Send, A: Type, requires A :- Allocator + Sync, quote alloc: lift *A { fn drop(*self); }
impl Drop for OneshotSender<T, .A = A, .allocator = alloc> where T: Type, requires T :- Send, A: Type, requires A :- Allocator + Sync, quote alloc: lift *A { fn drop(*self); }
impl Drop for OneshotReceiver<T, .A = A, .allocator = alloc> where T: Type, requires T :- Send, A: Type, requires A :- Allocator + Sync, quote alloc: lift *A { fn drop(*self); }
impl Drop for BroadcastSender<T, .A = A, .allocator = alloc> where T: Type, requires T :- Send + Clone, A: Type, requires A :- Allocator + Sync, quote alloc: lift *A { fn drop(*self); }
impl Drop for BroadcastReceiver<T, .A = A, .allocator = alloc> where T: Type, requires T :- Send + Clone, A: Type, requires A :- Allocator + Sync, quote alloc: lift *A { fn drop(*self); }
```

> **通道 `Drop` 与共享 Allocator 寿命不变式**：当最后一个发送者或接收者离开作用域时，原子引用计数归零的最后一方通过 `quote .allocator` 先析构环形队列或链表节点内残留未取走的 `T` 载荷，再释放通道共享控制块；因此无论发送端或接收端以何种线程顺序退出，编译器检查保证传入的 `allocator` 覆盖全部发送与接收实例生命期。

#### 3.5.4 `std.sync.thread`：OS 线程与作用域线程（`ThreadScope`）

```flurry
-- 归属：std.sync.thread
use builtin.{bool, Drop, Type, u64, usize}
use std.collections.Vec;
use std.mem.{AllocErr, Allocator, DefaultAllocator}
use std.runtime.TaskErr;
use std.sync.{Send, Sync}
use std.sync.atomic.AtomicUsize;
use std.time.Duration;

pub newtype ThreadId = u64;

pub struct Thread {
    raw_id: ThreadId,
    raw_handle: usize,
}

pub struct ThreadBuilder
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    name: ?std.string.String<.A = A, .allocator = allocator>,
    stack_size: ?usize,
}

pub struct ThreadJoinHandle
where
    T: Type,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    raw_handle: usize,
    state_addr: usize,
    _marker: builtin.mem.TypeMarker<T>,
}

impl ThreadJoinHandle<T, .A = A, .allocator = alloc>
where
    T: Type,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn thread(*self) -> *Thread;
    pub fn is_finished(*self) -> bool;
    pub fn join(self) -> !TaskErr T; -- 子线程 Panic unwind 映射为 TaskErr.Panicked，并经 alloc 释放共享状态块
}

impl Drop for ThreadJoinHandle<T, .A = A, .allocator = alloc>
where
    T: Type,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    -- 未调用 join(self) 直接丢弃时：分离 OS 线程句柄（detach）并原子递减 state_addr 引用计数；
    -- 若本端为最后释放者，则析构未取走的 T（若有）并经 alloc 归还共享线程状态块
    fn drop(*self);
}

pub struct ScopedJoinHandle
where
    T: Type,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    raw_handle: usize,
    state_addr: usize,
    _marker: builtin.mem.TypeMarker<T>,
}

impl ScopedJoinHandle<T, .A = A, .allocator = alloc>
where
    T: Type,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn thread(*self) -> *Thread;
    pub fn is_finished(*self) -> bool;
    pub fn join(self) -> !TaskErr T;
}

impl Drop for ScopedJoinHandle<T, .A = A, .allocator = alloc>
where
    T: Type,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    -- 未调用 join(self) 时释放本端包引用计数，由所属 ThreadScope.drop 等待子线程终止后经 alloc 回收 state_addr
    fn drop(*self);
}

pub struct ThreadScope
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    active_threads: AtomicUsize,
    children: Vec<usize, .A = A, .allocator = allocator>,
}

impl ThreadBuilder<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn new() -> Self;
    pub fn name(self, name: std.string.String<.A = A, .allocator = alloc>) -> Self;
    pub fn stack_size(self, size: usize) -> Self;
    pub fn spawn(self, body: %c fn() -> T) -> ![std.io.IoErr, AllocErr] ThreadJoinHandle<T, .A = A, .allocator = alloc>
    where
        T: Type,
        requires T :- Send,
        c: Type.vars.Closure,
        requires c :- Send;
}

impl ThreadScope<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn spawn(*self, body: %c fn() -> T) -> ![std.io.IoErr, AllocErr] ScopedJoinHandle<T, .A = A, .allocator = alloc>
    where
        T: Type,
        requires T :- Send,
        c: Type.vars.Closure,
        requires c :- Send;
}

impl Drop for ThreadScope<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    fn drop(*self);
}

pub fn spawn(body: %c fn() -> T) -> ![std.io.IoErr, AllocErr] ThreadJoinHandle<T, .A = A, .allocator = allocator>
where
    T: Type,
    requires T :- Send,
    c: Type.vars.Closure,
    requires c :- Send,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

pub fn scope(f: %c fn(*ThreadScope<.A = A, .allocator = allocator>) -> R) -> R
where
    R: Type,
    c: Type.vars.Closure,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

pub fn current() -> Thread;
pub fn sleep(dur: Duration);
pub fn yield_now();
pub fn park();
pub fn park_timeout(dur: Duration);
pub fn available_parallelism() -> !std.io.IoErr usize;
```

- **`ThreadBuilder` 与 `ThreadScope` 约束**：`ThreadBuilder.spawn` 从 `impl ThreadBuilder<.A = A, .allocator = alloc>` 接入构造；`ThreadScope` 在退出 `scope` 闭包前强制等待所有通过 `scope.spawn(...)` 派生的作用域线程结束并回收句柄，允许作用域内子线程安全借用外层局部栈数据（无需 `'static`）；而脱离 `ThreadScope` 的 `std.sync.thread.spawn` 严禁捕获未存活于全局的局部指针。

---

### 3.6 `std.sync.stm`：软件事务内存（BTO STM）与有序锁（`WL-08-15`）

依据 [docs/concurrency/prompt.fl](../../docs/concurrency/prompt.fl)，Flurry 对高并发共享状态实行**双轨分层指导**：
- **轻量可克隆值（`T :- Clone + Send + Sync`）**：优先采用 **BTO（Basic Timestamp Ordering）乐观软件事务内存**（`TVar` / `TArray` / `TQueue` + `atomic(vars) { ... }`），在栈上以固定长度写日志运行，无锁冲突直接提交；
- **重克隆或非 `Clone` 仿射资源（如大型容器、`File`、`Once` 句柄）**：引导采用 **Ordered Lock（`OrderedLock<T>`）**：进入 `atomic(a, b) { ... }` 时在栈上按**变量内存地址升序**排序并依次获取互斥锁，退出作用域自动按逆序释放，**结构性消除死锁且无需堆分配**！

```flurry
-- 归属：std.sync.stm
use builtin.{bool, NoReturn, Type, u64, usize, void}
use std.collections.Vec;
use std.mem.{AllocErr, Allocator, DefaultAllocator}
use std.sync.{AtomicBool, AtomicU64, Mutex, Send, Sync}

-- STM 状态载荷必须可克隆（支持回滚丢弃本地写缓冲）、可跨线程发送与共享
pub trait Transactional
where
    requires Self :- Clone + Send + Sync,
{}

impl Transactional for T
where
    T: Type,
    requires T :- Clone + Send + Sync,
{}

pub enum StmErr {
    Conflict,
    Retry,
    NestedTransaction,
    SideEffectForbidden,
    LockOrderViolation,
}

-- BTO 事务变量（替代 docs/concurrency/prompt.fl 中的占位名称 Resource）
pub struct TVar
where
    T: Type,
    requires T :- Transactional,
{
    data: T,
    read_ts: AtomicU64,
    write_ts: AtomicU64,
    commit_lock: AtomicBool,
}

impl TVar<T>
where
    T: Type,
    requires T :- Transactional,
{
    pub pure fn new(value: T) -> Self;
    -- 仅可在 `atomic(...)` 事务上下文中读写，由编译器注入局部 TransactionLog 上下文
    pub fn read(*self) -> !StmErr T;
    pub fn write(*self, new_val: T) -> !StmErr void;
    pub fn modify(*self, f: %c fn(T) -> T) -> !StmErr void where c: Type.vars.Closure;
    pub fn read_snapshot(*self) -> T; -- 事务外单点一致性快照读取
}

-- 事务数组与事务队列（堆分配容器，必须携带运行时依赖 allocator 子句）
pub struct TArray
where
    T: Type,
    requires T :- Transactional,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    slots: Vec<TVar<T>, .A = A, .allocator = allocator>,
}

impl TArray<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Transactional,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn with_len(len: usize, init: %c fn(usize) -> T) -> !AllocErr Self where c: Type.vars.Closure;
    pub fn len(*self) -> usize;
    pub fn get(*self, idx: usize) -> !StmErr ?T;
    pub fn set(*self, idx: usize, val: T) -> !StmErr bool;
}

pub struct TQueue
where
    T: Type,
    requires T :- Transactional,
    .A: Type = DefaultAllocator,
    requires A :- Allocator + Sync,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    ring_vars: TArray< ?T, .A = A, .allocator = allocator>,
    head: TVar<usize>,
    tail: TVar<usize>,
    cap: usize,
}

impl TQueue<T, .A = A, .allocator = alloc>
where
    T: Type,
    requires T :- Transactional,
    A: Type,
    requires A :- Allocator + Sync,
    quote alloc: lift *A,
{
    pub fn with_capacity(cap: usize) -> !AllocErr Self;
    pub fn push_back(*self, item: T) -> !StmErr void; -- 队列满时在事务内调用 retry() 等待出队唤醒
    pub fn pop_front(*self) -> !StmErr T;            -- 队列空时调用 retry() 挂起直到任意入队提交
    pub fn try_pop_front(*self) -> !StmErr ?T;
    pub fn is_empty(*self) -> !StmErr bool;
}

-- 有序锁容器（无需 T :- Clone，专用于仿射 Once 资源、大结构与 I/O 句柄的无死锁多对象同步）
pub struct OrderedLock
where
    T: Type,
    requires T :- Send + Sync,
{
    data: T,
    owner_tid: AtomicU64,
    inner_mutex: Mutex<void>,
}

impl OrderedLock<T>
where
    T: Type,
    requires T :- Send + Sync,
{
    pub pure fn new(value: T) -> Self;
    pub fn into_inner(self) -> T;
    pub fn with_lock(*self, f: %c fn(*T) -> R) -> R where R: Type, c: Type.vars.Closure;
}

-- 组合控制原语与策略处理器（对应 docs/concurrency/prompt.fl `.use(std.sync.stm.retry/await)`）
pub fn retry() -> !StmErr NoReturn;
pub fn or_else(primary: %c1 fn() -> !StmErr R, fallback: %c2 fn() -> !StmErr R) -> !StmErr R
where
    R: Type,
    c1: Type.vars.Closure,
    c2: Type.vars.Closure;

pub struct RetryPolicy {
    max_spins: usize,
}

pub struct AwaitPolicy {
    backoff_spins: usize,
}

pub fn retry_policy() -> RetryPolicy;
pub fn await_policy() -> AwaitPolicy;
```

#### STM 与 `Once` / `Drop` / 错误 / 效应交互契约
1. **栈上定长事务日志（零堆分配）**：`atomic(v1, v2, ..., vn) { ... }` 在词法入口编译时确定涉事变量集合并分配固定栈日志（新写覆写旧写）；`OrderedLock` 则在栈上对 `v_i` 的地址排序并按地址升序逐个上锁，绝不动态分配锁节点列表。
2. **错误传播先回滚**：在 `atomic(a, b) { a.write(1)!; check()!; }` 中，若 `check()` 返回用户错误并通过 `!` 向上传播，事务运行时**必须丢弃本地未提交写日志且不推进全局时钟**，然后才将错误结果交回外层。
3. **仿射 `Once` 与不可回滚效应禁令**：禁止将未实现 `Transactional`（特别是不可 `Clone` 的 `Once` 资源如 `File`、`TcpStream`、`TypedContinuation`）放入 `TVar`，否则重试会制造多次消费幻象；同理，`atomic(...)` 纯 STM 块内禁止执行不可逆外部 I/O（必须使用 `OrderedLock` 或事务提交后再外泄事件）。

```flurry
-- 正例：BTO 事务转账（失败自动回滚写日志，冲突按策略重试）
fn transfer(src: *TVar<u64>, dst: *TVar<u64>, amount: u64) -> !StmErr void {
    let bal = src.read()!;
    if bal < amount {
        retry()!;
    }
    src.write(bal - amount)!;
    let d_bal = dst.read()!;
    dst.write(d_bal + amount)!;
}

-- 拒绝例（禁止将不可 Clone 的 Once 文件句柄放入 TVar，应改用 OrderedLock<File>）
{-
fn reject_non_transactional_tvar(f: std.fs.File) {
    let bad = TVar<std.fs.File>.new(f); -- 编译错误：std.fs.File 未实现 Clone / Transactional
}
-}
```

---

### 3.7 `std.net`：基础 L3/L4 套接字（同步 I/O 与异步桥接，零 HTTP/TLS 膨胀，`WL-08-16` ~ `WL-08-18`）

**严格边界声明**：`std.net` 仅封装操作系统原生网络地址解析与 TCP/UDP/Unix Domain 套接字原语；**严禁在 `std.net` 中引入 HTTP/1.1/2/3、TLS/SSL、WebSocket、QUIC 或 URI 解析**（全部由 `10` 中的 Tier 2 `wire`/`http` 协议包与 Tier 3 实现承担）。

```flurry
-- 归属：std.net
use builtin.{Array, bool, str, Type, u16, u32, u8, usize, void}
use std.collections.{MutSlice, Slice, Vec}
use std.io.{IoErr, Reader, Write}
use std.mem.{AllocErr, Allocator, DefaultAllocator}
use std.time.Duration;

pub enum AddrParseErr {
    InvalidIp,
    InvalidPort,
    InvalidSocketAddr,
    EmptyInput,
}

pub enum NetErr {
    Io(IoErr),
    AddrParse(AddrParseErr),
    DnsResolutionFailed(i32),
    AddrInUse,
    AddrNotAvailable,
    ConnectionRefused,
    ConnectionReset,
    ConnectionAborted,
    NotConnected,
    TimedOut,
    WouldBlock,
    InvalidInput,
}

pub enum Shutdown {
    read,
    write,
    both,
}

pub struct Ipv4Addr {
    octets: Array<u8, 4usize>,
}

pub struct Ipv6Addr {
    segments: Array<u16, 8usize>,
}

pub enum IpAddr {
    v4(Ipv4Addr),
    v6(Ipv6Addr),
}

pub struct SocketAddrV4 {
    ip: Ipv4Addr,
    port: u16,
}

pub struct SocketAddrV6 {
    ip: Ipv6Addr,
    port: u16,
    flowinfo: u32,
    scope_id: u32,
}

pub enum SocketAddr {
    v4(SocketAddrV4),
    v6(SocketAddrV6),
}

-- DNS/地址解析返回动态地址序列，严格遵循运行时依赖 allocator 范式
pub trait ToSocketAddrs {
    fn to_socket_addrs(
        *self,
    ) -> ![NetErr, AllocErr] Vec<SocketAddr, .A = A, .allocator = allocator>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}

impl Ipv4Addr {
    pub const LOCALHOST: Ipv4Addr = Ipv4Addr { octets: [127u8, 0u8, 0u8, 1u8] }
    pub const UNSPECIFIED: Ipv4Addr = Ipv4Addr { octets: [0u8, 0u8, 0u8, 0u8] }
    pub const BROADCAST: Ipv4Addr = Ipv4Addr { octets: [255u8, 255u8, 255u8, 255u8] }

    pub pure fn new(a: u8, b: u8, c: u8, d: u8) -> Self;
    pub pure fn from_bits(bits: u32) -> Self;
    pub pure fn to_bits(self) -> u32;
    pub pure fn octets(self) -> Array<u8, 4usize>;
    pub pure fn is_loopback(self) -> bool;
    pub pure fn is_private(self) -> bool;
    pub pure fn is_multicast(self) -> bool;
    pub pure fn parse(s: str) -> !AddrParseErr Self;
}

impl Ipv6Addr {
    pub const LOCALHOST: Ipv6Addr = Ipv6Addr { segments: [0u16, 0u16, 0u16, 0u16, 0u16, 0u16, 0u16, 1u16] }
    pub const UNSPECIFIED: Ipv6Addr = Ipv6Addr { segments: [0u16, 0u16, 0u16, 0u16, 0u16, 0u16, 0u16, 0u16] }

    pub pure fn new(a: u16, b: u16, c: u16, d: u16, e: u16, f: u16, g: u16, h: u16) -> Self;
    pub pure fn segments(self) -> Array<u16, 8usize>;
    pub pure fn is_loopback(self) -> bool;
    pub pure fn is_multicast(self) -> bool;
    pub pure fn parse(s: str) -> !AddrParseErr Self;
}

impl SocketAddr {
    pub pure fn new(ip: IpAddr, port: u16) -> Self;
    pub pure fn ip(self) -> IpAddr;
    pub pure fn port(self) -> u16;
    pub fn set_port(*self, new_port: u16);
    pub pure fn parse(s: str) -> !AddrParseErr Self;
}

-- 底层操作系统套接字描述符（均拥有 OS fd/handle，实现 Once + Drop 自动 close）
pub struct TcpStream {
    raw_handle: i64,
}

pub struct TcpListener {
    raw_handle: i64,
}

pub struct UdpSocket {
    raw_handle: i64,
}

pub struct UnixStream {
    raw_handle: i64,
}

pub struct UnixListener {
    raw_handle: i64,
}

pub struct Incoming {
    listener: *TcpListener,
}

impl TcpListener {
    pub fn bind(addr: SocketAddr) -> !NetErr Self;
    pub fn bind_addrs(addr: *T) -> ![NetErr, AllocErr] Self
    where
        T: Type,
        requires T :- ToSocketAddrs,
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
    pub fn accept(*self) -> !NetErr Tuple<TcpStream, SocketAddr>;
    pub fn incoming(*self) -> Incoming;
    pub fn local_addr(*self) -> !NetErr SocketAddr;
    pub fn try_clone(*self) -> !NetErr Self;
    pub fn set_ttl(*self, ttl: u32) -> !NetErr void;
    pub fn set_nonblocking(*self, nonblocking: bool) -> !NetErr void;
    pub fn take_error(*self) -> !NetErr ?NetErr;
}

impl TcpStream {
    pub fn connect(addr: SocketAddr) -> !NetErr Self;
    pub fn connect_addrs(addr: *T) -> ![NetErr, AllocErr] Self
    where
        T: Type,
        requires T :- ToSocketAddrs,
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
    pub fn connect_timeout(addr: SocketAddr, timeout: Duration) -> !NetErr Self;
    pub fn peer_addr(*self) -> !NetErr SocketAddr;
    pub fn local_addr(*self) -> !NetErr SocketAddr;
    pub fn shutdown(*self, how: Shutdown) -> !NetErr void;
    pub fn try_clone(*self) -> !NetErr Self;
    pub fn set_read_timeout(*self, dur: ?Duration) -> !NetErr void;
    pub fn set_write_timeout(*self, dur: ?Duration) -> !NetErr void;
    pub fn read_timeout(*self) -> !NetErr ?Duration;
    pub fn write_timeout(*self) -> !NetErr ?Duration;
    pub fn peek(*self, buf: MutSlice<u8>) -> !NetErr usize;
    pub fn set_nodelay(*self, nodelay: bool) -> !NetErr void;
    pub fn nodelay(*self) -> !NetErr bool;
    pub fn set_nonblocking(*self, nonblocking: bool) -> !NetErr void;
    pub fn take_error(*self) -> !NetErr ?NetErr;
}

impl Reader for TcpStream {
    assoc Err: Type = IoErr;

    unsafe fn read_raw(*self, dst: usize, len: usize) -> !IoErr usize;
    fn read_byte(*self) -> !IoErr ?u8;
    fn read(*self, buf: MutSlice<u8>) -> !IoErr usize;
}

impl Write for TcpStream {
    assoc Err: Type = IoErr;

    unsafe fn write_raw(*self, src: usize, len: usize) -> !IoErr usize;
    fn flush(*self) -> !IoErr void;
    fn write(*self, buf: Slice<u8>) -> !IoErr usize;
    fn write_all_slice(*self, buf: Slice<u8>) -> !IoErr void;
}

impl Drop for TcpStream { fn drop(*self); }
impl Drop for TcpListener { fn drop(*self); }
impl Drop for UdpSocket { fn drop(*self); }
impl Drop for UnixStream { fn drop(*self); }
impl Drop for UnixListener { fn drop(*self); }

impl UdpSocket {
    pub fn bind(addr: SocketAddr) -> !NetErr Self;
    pub fn bind_addrs(addr: *T) -> ![NetErr, AllocErr] Self
    where
        T: Type,
        requires T :- ToSocketAddrs,
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
    pub fn connect(*self, addr: SocketAddr) -> !NetErr void;
    pub fn send(*self, buf: Slice<u8>) -> !NetErr usize;
    pub fn recv(*self, buf: MutSlice<u8>) -> !NetErr usize;
    pub fn send_to(*self, buf: Slice<u8>, addr: SocketAddr) -> !NetErr usize;
    pub fn send_to_addrs(*self, buf: Slice<u8>, addr: *T) -> ![NetErr, AllocErr] usize
    where
        T: Type,
        requires T :- ToSocketAddrs,
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
    pub fn recv_from(*self, buf: MutSlice<u8>) -> !NetErr Tuple<usize, SocketAddr>;
    pub fn peek_from(*self, buf: MutSlice<u8>) -> !NetErr Tuple<usize, SocketAddr>;
    pub fn local_addr(*self) -> !NetErr SocketAddr;
    pub fn peer_addr(*self) -> !NetErr SocketAddr;
    pub fn set_broadcast(*self, broadcast: bool) -> !NetErr void;
    pub fn set_multicast_loop_v4(*self, on: bool) -> !NetErr void;
    pub fn join_multicast_v4(*self, multiaddr: Ipv4Addr, interface: Ipv4Addr) -> !NetErr void;
    pub fn leave_multicast_v4(*self, multiaddr: Ipv4Addr, interface: Ipv4Addr) -> !NetErr void;
    pub fn set_read_timeout(*self, dur: ?Duration) -> !NetErr void;
    pub fn set_write_timeout(*self, dur: ?Duration) -> !NetErr void;
    pub fn set_nonblocking(*self, nonblocking: bool) -> !NetErr void;
}

impl UnixListener {
    pub fn bind(path: std.path.Path) -> !NetErr Self;
    pub fn accept(*self) -> ![NetErr, AllocErr] Tuple<UnixStream, std.path.PathBuf<.A = A, .allocator = allocator>>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
    pub fn set_nonblocking(*self, nonblocking: bool) -> !NetErr void;
}

impl UnixStream {
    pub fn connect(path: std.path.Path) -> !NetErr Self;
    pub fn pair() -> !NetErr Tuple<UnixStream, UnixStream>;
    pub fn shutdown(*self, how: Shutdown) -> !NetErr void;
    pub fn set_read_timeout(*self, dur: ?Duration) -> !NetErr void;
    pub fn set_write_timeout(*self, dur: ?Duration) -> !NetErr void;
    pub fn read_timeout(*self) -> !NetErr ?Duration;
    pub fn write_timeout(*self) -> !NetErr ?Duration;
    pub fn set_nonblocking(*self, nonblocking: bool) -> !NetErr void;
    pub fn take_error(*self) -> !NetErr ?NetErr;
}

impl Reader for UnixStream {
    assoc Err: Type = IoErr;

    unsafe fn read_raw(*self, dst: usize, len: usize) -> !IoErr usize;
    fn read_byte(*self) -> !IoErr ?u8;
    fn read(*self, buf: MutSlice<u8>) -> !IoErr usize;
}

impl Write for UnixStream {
    assoc Err: Type = IoErr;

    unsafe fn write_raw(*self, src: usize, len: usize) -> !IoErr usize;
    fn flush(*self) -> !IoErr void;
    fn write(*self, buf: Slice<u8>) -> !IoErr usize;
    fn write_all_slice(*self, buf: Slice<u8>) -> !IoErr void;
}

-- 非阻塞异步 I/O 桥接效应（由外部 Tier 2 async_runtime Reactor 驱动，std.net 不内置事件循环）
pub effect enum NetIoWait {
    async wait_readable(fd: i64) -> !NetErr void,
    async wait_writable(fd: i64) -> !NetErr void,
}

impl TcpStream {
    pub fn read_async(*self, buf: MutSlice<u8>) -> #NetIoWait !NetErr usize;
    pub fn write_async(*self, buf: Slice<u8>) -> #NetIoWait !NetErr usize;
}

impl TcpListener {
    pub fn accept_async(*self) -> #NetIoWait !NetErr Tuple<TcpStream, SocketAddr>;
}

impl UdpSocket {
    pub fn recv_from_async(*self, buf: MutSlice<u8>) -> #NetIoWait !NetErr Tuple<usize, SocketAddr>;
    pub fn send_to_async(*self, buf: Slice<u8>, addr: SocketAddr) -> #NetIoWait !NetErr usize;
}

impl UnixStream {
    pub fn read_async(*self, buf: MutSlice<u8>) -> #NetIoWait !NetErr usize;
    pub fn write_async(*self, buf: Slice<u8>) -> #NetIoWait !NetErr usize;
}

impl UnixListener {
    pub fn accept_async(*self) -> #NetIoWait ![NetErr, AllocErr] Tuple<UnixStream, std.path.PathBuf<.A = A, .allocator = allocator>>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}
```

#### `std.net` 异步非阻塞效应集成（无执行器膨胀）
- **设计策略**：`TcpStream`/`TcpListener`/`UdpSocket`/`UnixStream`/`UnixListener` 通过 `set_nonblocking(true)` 与 `#NetIoWait`（或直接适配至 `10` §3.5 Tier 2 `async_runtime.AsyncRuntimeOps.io_wait` / `AsyncReader`/`AsyncWriter`）挂起并由外部 `.use(runtime)` 注入的 `Reactor` 唤醒；`std.net` 本身**绝不绑定特定 OS 事件循环**（如 `epoll` 或 `io_uring` 线程）。

```flurry
-- 拒绝例：严禁将 HTTP / TLS / WebSocket 塞入 Tier 1 std.net
{-
use std.net.HttpClient; -- 规范拒绝：HTTP 属于 Tier 2 `http` + Tier 3 生态实现，绝不进入 std.net
use std.net.TlsStream;  -- 规范拒绝：TLS 属于 Tier 2/3 生态协议包
-}
```

---

## 4. 排期优先级与分阶段交付路线

```text
Phase 1（P0 核心 ABI、处理能力、Continuation 与原子基线解锁）
  ├── builtin/attrs/llvm：HandleError/HandleEffect elaboration、let x handles ...、__flurry_direct_passing、{ T, ptr } 控制字 Panic 展开与 C 帧 trap
  ├── builtin.Continuation + std.runtime.{TypedContinuation, StackPool, BoundedStackPool}（接入 Silver P0-3 meta.runtime.bounded_stack_bytes）
  └── std.sync.atomic：Send/Sync + Atomic*/AtomicPtr/AtomicOrdering/fence（直接解锁 07 Arc/ArcWeak）
        │
        ▼
Phase 2（P1 结构化异步、基础 OS 同步锁/通道与同步 Socket）
  ├── std.runtime：TaskContext、CancelToken、Future/Awaitable、JoinHandle、TaskScope（正常等待 + 取消清理）、GuardedStackPool
  ├── std.sync：Mutex/RwLock/Condvar/OnceLock/LazyLock/Barrier/Semaphore、mpsc/oneshot 通道（带 allocator）、Thread/ThreadScope
  └── std.net：Ipv4Addr/Ipv6Addr/IpAddr/SocketAddr*、TcpListener/TcpStream/UdpSocket/Unix*（对接 std.io.Reader/Write）
        │
        ▼
Phase 3（P1/P2 Tier 2 async_runtime 互操作、高级通道与异步 Socket）
  ├── std.runtime <-> Tier 2 async_runtime：AsyncCore 与 Executor/Reactor/Timer/AsyncReader/AsyncWriter 完整联调（io_uring_rt / Silver UI tick）
  ├── std.runtime 组合子：join_pair/select_pair/race_pair/timeout 零丢弃/零泄漏压力测试
  └── std.sync & std.net：mpmc/broadcast 通道（allocator 透传）与非阻塞 socket io_wait 桥接
        │
        ▼
Phase 4（P2 STM 软件事务内存与有序锁完备化）
  ├── std.sync.stm：Transactional、TVar/TArray/TQueue、OrderedLock、retry/or_else
  └── compiler/hir & elaboration：atomic(vars) { ... } 栈上 BTO 日志、地址升序加锁与错误/IO 回滚静态拒绝
```

**阶段验收基线**：
1. 必须覆盖 `TypedContinuation` 多次挂起后中途直接丢弃（`Drop` 走隐藏中止通道按词法逆序执行栈上 `Once` 清理，且栈槽准确归还原 `BoundedStackPool`）；
2. 必须包含跨越 `^attrs.abi(.c)` 帧捕获后的合法恢复通过用例与非法放弃触发 trap 的运行期验收测试；
3. 所有并发通道、`TaskScope`、`CancelToken`、`TArray`/`TQueue` 与 `ToSocketAddrs` 必须在真实 `--sysroot library` 下同时通过 `DefaultAllocator`（零额外存储开销）与自定义局部 `TrackingAllocator`（并发零泄漏）验收。

---

## 5. 待作者裁决的语义空缺与冲突（Open Questions）

### OQ-08-01 `HandleEffect` trait 的多操作组（`effect enum`）与可恢复参数精确编码
- **来源依据**：[2026-9-18-handling-capability-by-trait-system.fl:73-78](../../proposal_trace/2026-9-18-handling-capability-by-trait-system.fl) 结尾留空（`trait HandleEffect where E, eff: E, ...`）；[2026-8-25-handler-capability.fl:21-22](../../proposal_trace/2026-8-25-handler-capability.fl) 将 `handles (...)` 错放于 `fn` 签名；[handling-capability.md](../../docs/algebraic-effect-and-delimited-continuation/handling-capability.md)。
- **核心矛盾**：单个操作可用 `DirectEffectHandler<OpTag, Args, Y>` 或 `ResumptiveEffectHandler<OpTag, Args, Y, B>` 参数化，但如何用一个统一且能消去多分支 `effect enum`（每个 variant 有不同 `(Args_i -> Y_i)`）的单一 trait 让 `let handler handles Log = DefaultLogger` 与 `expr.use(handler)` 无缝接通，尚未确定最终签名形状。
- **最小决策选项**：
  - *选项 A（推荐）*：编译器把 `effect enum G { op1(X1) -> Y1, op2(X2) -> Y2 }` 按项展开，要求处理器类型对 `G` 的每个独立操作实现 `DirectEffectHandler` / `ResumptiveEffectHandler`（或由 `HandleEffect` 接收统一 `EffectFrame`/操作分派投影），`.use(handler)` 通过 trait resolution 核对整组完备性，完全避免在任何 `fn` 签名保留 `handles`。
  - *选项 B*：为每个 `effect enum G` 自动合成配套配套 trait（类似生成 `trait HandleG`），`impl HandleG for HandlerType` 显式编写全部 variant 处理分支。

### OQ-08-02 `std.process.panic` 展开策略、双 Panic 处理与 `__flurry_caller_location`（解除 `library/README.md:395` 阻塞）
- **来源依据**：[library/README.md:393-395](../../library/README.md)、[effect-abi.md:195](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)、[07-std-mem-collections-io-os.md](./07-std-mem-collections-io-os.md) §5 Q1、[06-attrs-build-pipeline-projection.md](./06-attrs-build-pipeline-projection.md)（`__flurry_caller_location`）。
- **受影响行为**：`PanicStrategy.unwind` 沿 `{ T, ptr = 1 }` 执行 `Drop` 后如何暴露给 `ThreadJoinHandle.join` / `TaskScope`；`Drop` 内再次 panic 时（控制字校验 `effect-abi.md:195`）是否直接 trap；是否将 `__flurry_caller_location` 正式登记入 `official-attribute-keys.md`。
- **最小决策建议**：确立默认 `unwind` 清理（仅捕获于线程/`JoinHandle` 边界，禁止用户任意函数无约束捕获 `panic`）+ `panic = .abort` 编译选项 + C 帧边界立即 trap + 登记 `__flurry_caller_location` 提供文件名与行列号。

### OQ-08-03 `builtin.Continuation` 物理后端迁移兼容：`BoundedStackPool` 栈切换 vs 无栈状态机优化
- **来源依据**：[effect-abi.md:42](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)（“首版使用地址稳定的有栈后端”）、[tmp/silver-design/04-compiler-wishlist.md](../silver-design/04-compiler-wishlist.md) `P0-3`/`P1-1`、[2026-9-28-algebraic-effects.fl:10-16](../../proposal_trace/2026-9-28-algebraic-effects.fl)。
- **空缺描述**：若未来某些 `async effect` 或同步生成器同时引入无栈状态机（CPS/coro-split）降码，通过 `.use(pool)` 传入的 `StackPool` 契约是降为编译期空操作（只检查 `StackPool` 供应资格）还是明确规定可捕获多 prompt 协程统一运行在地址稳定池化栈上、仅由 `inline` 消除闭环？
- **最小决策建议**：维持 [effect-abi.md](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md) 契约——标准通用路线为地址稳定的池化有栈后端（`BoundedStackPool` 消除虚内存与 guard page 开销），状态机化仅作为局部封闭 `inline`/单 prompt 优化，且不得改变语义或绕过池与依赖寿命检查。

### OQ-08-04 `Mutex` / `RwLock` 死后投毒（Poisoning）契约取舍
- **来源依据**：[library/std/sync/README.md](../../library/std/sync/README.md)（空白）、`OQ-08-02`（Panic 展开契约）。
- **冲突现状**：若线程持有 `MutexGuard` 期间发生 `Panic` 并沿控制字 `1` 展开运行 `MutexGuard.Drop` 解锁，共享数据可能处于半更新不变式破坏状态。是否沿用 Rust 式 `PoisonErr<MutexGuard<T>>` 返回错误让后续加锁者显式判断，还是保持无投毒（在 Panic 展开中若 `panic = .abort` 直接终止，或由用户自行 `unwrap`）？
- **最小决策建议**：默认提供 `PoisonErr`（`lock() -> !PoisonErr<MutexGuard<T>> MutexGuard<T>`）以显式利用 Flurry 原生 `!E T` 零开销联合返回，另可选配 `NonPoisonMutex` 或等待作者裁定全语言 `Mutex` 是否采用无投毒模型。

### OQ-08-05 STM `atomic(vars) { ... }` 语言原语归属、`OrderedLock` 混合以及空文档 `software-transactional-memory.md`
- **来源依据**：[docs/concurrency/prompt.fl](../../docs/concurrency/prompt.fl)、[docs/concurrency/software-transactional-memory.md](../../docs/concurrency/software-transactional-memory.md)（目前 0 字节空文件）、[docs/grammar/syntax](../../docs/grammar/syntax)（当前语法未列入 `atomic(...)` 关键字）。
- **受影响行为**：`atomic(destination, source) { ... }` 目前仅存在于设计草图 `prompt.fl`，既未写入正式 `syntax` 规则，也未确认其通过语言内置原语还是结构化宏（`define` / 构造宏 + 编译器事务日志降码）引入；此外同一个 `atomic(...)` 块是否允许混用 `TVar`（乐观日志回滚）与 `OrderedLock`（悲观升序锁，无法无副作用回滚）。
- **最小决策建议**：严禁混用乐观可重试 `TVar` 与不可回滚 `OrderedLock`（`TVar` 块禁止不可逆外泄且允许 `retry`；`OrderedLock` 块采用单独 `lock_ordered(a, b) { ... }` 或静态禁止内含 `retry`/`TVar` 冲突回滚），并由作者冻结后同步补全 `docs/concurrency/software-transactional-memory.md`。

### OQ-08-06 通用代数效应实用套件（`State` / `Reader` / `Writer` / `Yield` / `NonDet`）归属 `std` 还是 Tier 2 `effects_ext`
- **来源依据**：`AGENTS.md` 六内置包精简原则、Board #10 指示、[10-official-protocol-packages-and-boundary.md](./10-official-protocol-packages-and-boundary.md)。
- **冲突现状**：`State<S>`/`Reader<R>`/`Writer<W>`/`Yield<T>`/`NonDet` 在效应编程教学与库中常见，但均非系统运行时或 ABI 硬底座（不像 `Continuation`、`StackPool`、`Cancel`、`AsyncCore`、`emit_item`）。若全部塞入 `builtin` 或 `std` 会造成 Tier 1 膨胀；但 `Yield<T>` 对用户快速构建自定义协程流极高频。
- **最小决策选项**：
  - *选项 A（推荐，极致精简 Tier 1）*：全部通用效应组合套件（`State`/`Reader`/`Writer`/`Yield`/`NonDet`）放入 Tier 2 官方薄协议包 `effects_ext`；Tier 1 `std.runtime` 仅保留 `TypedContinuation`、`StackPool`/`BoundedStackPool`/`GuardedStackPool`、`AsyncCore`、`TaskScope` 与 `CancelToken`。
  - *选项 B*：仅在 `std.runtime` 额外保留极简 `Yield<T>`，其余放入 `effects_ext`。

### OQ-08-07 裸指针 `*T` 的 `Send` / `Sync` 规则与 `OnceLock` / `LazyLock` 初始化异常行为
- **来源依据**：[docs/resource-management/pointers.md](../../docs/resource-management/pointers.md)、`07` `MutSlice` / `Box` / `Arc` 接口。
- **受影响行为**：`Box<T>`、`Vec<T>`、`MutexGuard<T>`、`Arena<T>` 内部常保存 `raw_addr: usize`（或 `*T`）；若 `*T` 默认实现 `Send`/`Sync`，会导致 `Rc` 若用 `*RcInner` 时误获自动 `Send`（必须显式禁止）；若 `*T` 像 Rust 一样默认 `!Send` 且 `!Sync`，则所有含内部指针的容器都需显式安全声明 `impl Send`/`impl Sync`。另外 `LazyLock` 在首次求值回调触发 `Panic` 后，后续访问应直接传播/再触发失败还是进入中毒态，需与 `OQ-08-04` 一并裁定。
