# 文件系统

`File.open<path>()` 只读打开；`File.create<path>()` 创建或截断，以 0666 交给
进程 umask 过滤。两者设置 close-on-exec。方法包括
`write_all<text>()`、`read_byte()`、`close()`；字节读取通过 `?u8` 表示 EOF。
`remove_file(comptime path)` 删除目录项。另提供一次性读写辅助函数
`read(comptime path)`、`read_to_string(comptime path)`、`write_str(comptime path, text)`
与 `write_bytes(comptime path, bytes)`（其中读取辅助支持可选 `.A` 与
`quote .allocator` 运行时依赖 allocator 子句并组合 `![IoErr, AllocErr, ...]` 错误行）。

File 不实现 Copy/Clone。Drop 关闭剩余描述符；显式 close 先退休句柄，再报告
关闭结果，失败也不会让析构再次关闭一个可能已被复用的描述符。需要知道关闭
失败时使用显式 close。关闭不等同于持久化，尚未提供 fsync。

首版路径和写入文本是 comptime 值。动态路径、目录遍历、元数据、随机访问与
缓冲 I/O 尚未实现。
