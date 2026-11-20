# 内存管理与堆分配

`std.mem` 在 `Layout` 之上提供底层未类型化块元数据、分配器协议及单所有权堆盒：

- `Layout`：`Layout.new(size, alignment)`、`Layout.of<T>()`、`Layout.array<T>(count)`、`repeat`、`pad_to_align`、`append`，以及面向 `usize` 地址空间的 `byte_size_usize` / `byte_alignment_usize`。
- `AllocErr` / `ReallocErr`：描述布局无效与内存耗尽错误。
- `RawBlock`：携带已检查对齐的非空或悬空地址与 `Layout`，不包含隐式释放责任。
- `Allocator` 与 `SystemAllocator`：`allocate`、`allocate_zeroed`、`deallocate`、`reallocate`；零大小布局返回对齐的悬空块且不触发系统分配。
- `Box<T>`：单所有权（`Once`）堆分配包装，在 `Drop` 时先消费并析构内部 `T` 再释放块，并提供 `new`、`into_inner`、`replace`、`set`、`get`、`address`、`layout`。
