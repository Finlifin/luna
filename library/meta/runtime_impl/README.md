# 宿主桥接

已提供基础契约与静态栈界查询 (`library/meta/runtime_impl/mod.fl`)：`HostInvokeErr`、`HostCapability`、`HostOutput` 与 `bounded_stack_bytes(entry_fn: F) -> ?usize where F: Type`。其中 `bounded_stack_bytes` 基于 NIR 直接调用图与显式/声明对齐局部存储给出静态上界，遇到递归或无法静态定界调用返回 `null`；post-isel LLVM 机器寄存器溢出与栈保护帧不计入该静态界。完整外部进程/文件系统宿主桥接尚待后续能力授权层实现。
