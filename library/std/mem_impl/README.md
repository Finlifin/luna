# 分配布局

`Layout.new(size, alignment)` 检查非零二次幂对齐及向上对齐的溢出边界，返回
`?Layout`。`Layout.of<T>()` 与 `Layout.array<T>(count)` 基于 `Type.size_of(T)` 和
`Type.align_of(T)` 构造类型与数组布局；`byte_size()`、`byte_alignment()` 观察布局，
`repeat(count)` 按对齐后步长计算数组布局，`pad_to_align()` 按自身对齐补齐，
`append(next)` 计算顺序拼接后的组合布局及下一字段偏移，并检查乘法与加法溢出。
零大小、零元素数有效。

大小使用 u64。真正分配时仍须检查目标地址空间限制并建立 memory view、初始化、
归属与释放契约。当前没有 allocator、arena 或假分配实现。
