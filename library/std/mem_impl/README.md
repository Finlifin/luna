# 内存管理与堆分配

`std.mem` 在 `Layout` 之上提供底层未类型化块元数据、分配器协议及单所有权堆盒：

- `Layout`：`Layout.new(size, alignment)`、`Layout.of<T>()`、`Layout.array<T>(count)`、`repeat`、`pad_to_align`、`append`，以及面向 `usize` 地址空间的 `byte_size_usize` / `byte_alignment_usize`。
- `AllocErr` / `ReallocErr`：描述布局无效与内存耗尽错误。
- `RawBlock`：携带已检查对齐的非空或悬空地址与 `Layout`，不包含隐式释放责任。
- `Allocator` 与 `SystemAllocator`：`allocate`、`allocate_zeroed`、`deallocate`、`reallocate`；零大小布局返回对齐的悬空块且不触发系统分配；`SystemAllocator.GLOBAL` 提供静态只读默认实例，`pub typealias DefaultAllocator = SystemAllocator;`。
- `FixedBufferAllocator`：基于调用方传入 `RawBlock` 的非拥有式 bump 分配器，追踪活跃非零分配数量与尾块原地扩展／回收，并在容量耗尽时返回 `AllocErr.OutOfMemory`。
- `ArenaAllocator<.Backing: Type = DefaultAllocator, quote .backing: lift *Backing = DefaultAllocator.GLOBAL.ref>`：基于侵入式链表块的组合式区域分配器，从运行时依赖的 `.backing` 分配器申请块，并在 `reset()` 或 `Drop` 时将全部块归还给 `.backing`。
- `Box<T, .A: Type = DefaultAllocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref>`：采用运行时依赖类型的单所有权（`Once`）堆分配包装，在 `Drop` 时通过环境参数交付的 `allocator` 释放块而不在容器本体存储分配器指针（零存储开销），并提供 `new`、`into_inner`、`replace`、`set`、`get`、`address`、`layout`。
