# 基础内存原语与规格

`builtin.mem`（由 `builtin.mem_impl.raw` 导出）提供受 `unsafe` 边界约束的底层内存宿主原语与目标无关的 `usize` 溢出检查算术：

- `target.Memory` 与 `raw_read<T>`、`raw_write<T>`、`copy_bytes`、`copy_bytes_nonoverlapping`、`set_bytes`、`compare_bytes`（零长度与零大小类型按无操作或 `0` 处理）。
- `usize_max`、`checked_add_usize`、`checked_sub_usize`、`checked_mul_usize`、`align_up_usize`（按显式目标 `usize` 位宽完成范围检查）。

高级分配策略（`Allocator`、`SystemAllocator`、`RawBlock`、`Box`）位于 `std.mem`；程序状态证明接口仍由 `verification` 负责。
