# 拥有型容器 (`std.collections`)

已实现 `std.collections.{Vec, VecIter}`（定义于 [`collections_impl/vec.fl`](collections_impl/vec.fl)）：

- 基于 `std.mem.{Allocator, SystemAllocator, RawBlock, Layout}` 与 `builtin.mem` 的几何扩容拥有型连续数组，`Once` 仿射语义，析构与局部退出按顺序 drop 所有已初始化元素后再释放缓冲。
- 零大小元素（ZST）不分配 OS 堆内存，仍准确维护长度并执行元素的 `Drop` 与所有权转移。
- 已提供：`new`、`with_capacity`、`len`、`capacity`、`is_empty`、`as_addr`、`reserve`、`shrink_to_fit`、`push`、`pop`、`insert`、`remove`、`swap_remove`、`swap`、`replace`、`set`、`truncate`、`clear`、`reverse`、`append`、`with_ref`、`for_each_ref`、`try_for_each_ref`、面向 `Copy` 元素的 `get`/`first`/`last`/`contains`、面向 `Clone` 元素的 `try_clone`、`IntoIterator`/`Iterator`（`VecIter<T>`）、`Eq for Vec<T>` 与 `ByteWriter for Vec<u8>`。

`map`/`set`、索引切片视图等尚未实现。
