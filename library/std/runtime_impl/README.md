# 程序效应 runtime (`std.runtime`)

已实现公开模块树（实现位于 `runtime_impl/`，经 `std.lib` 导出为 `std.runtime`）：

- `TypedContinuation<Y, B>`：基于 `builtin.Continuation` 的仿射 `Once` 类型化包装（禁止 `Copy`/`Clone`/`Send`/`Sync`）。
- 栈池协议（`OQ-08-03` 地址稳定池化栈）：`StackPoolErr`、`StackSlot`（`Once`）、`StackPool` trait、`BoundedStackPool<slot_bytes, .A, .allocator>`（slab 分级固定槽池）与 `GuardedStackPool<.A, .allocator>`。
- 结构化 `async effect` 内核：`TaskId`、`CancelErr`、`TaskErr`、`RuntimeErr`、`FutureState`、`TaskContext`（匹配 `task_context.ll` `%flurry.task = type { i8, ptr }`）、`CancelToken`、`Future`、`JoinHandle`、`AsyncCore`、`TaskScope`、`Select2`，以及 `scoped`/`yield_now`/`sleep`/`join_pair`/`select_pair`/`race_pair`/`timeout`。
- 严守 `OQ-08-06` Tier 1 边界：不提供 `State`/`Reader`/`Writer`/`Yield`/`NonDet` 等通用效应套件，不内置具体 `epoll`/`io_uring` 事件循环。

## 挂接边界（待 `@llvm-effects` / `08-library` / async lowering 合入后接通）

- `TypedContinuation.{resume_with, abort, crosses_c_frame, origin_thread_id}` 待 `@llvm-effects` Commit 3 合入 `host-intrinsic.continuation_{resume_raw,abort_raw,crosses_c_frame,origin_thread_id}`（`TypedContinuation` 与 `FutureBox` 已接入 `builtin.mem.{TypeMarker, MaybeUninit}`）。
- `BoundedStackPool` 已通过 `bounded_stack_bytes_for` / `verify_bounded_stack_bytes` / `require_bounded_stack_bytes` 接通 `meta.runtime.bounded_stack_bytes` 与 `meta.compilation.err` 编译期栈上界校验（`OQ-08-03`）。
- `GuardedStackPool` 待 OS 虚拟内存保留与 guard page 平台 intrinsic（当前经由 `Allocator` 托管栈缓冲，支持完整池化分配、归还与 `Drop` 回收）。
- `AsyncCore` 组合子的 `.await` 与 `async` handler arm 降码待前端 HIR/elaboration 支持 `.await` 与 `Abi::Async` handler 后接通。
