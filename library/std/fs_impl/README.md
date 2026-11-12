# 文件系统

`File.open<path>()` 只读打开；`File.create<path>()` 创建或截断，以 0666 交给
进程 umask 过滤。两者设置 close-on-exec。方法包括
`write_all<text>()`、`read_byte()`、`close()`；字节读取通过 `?u8` 表示 EOF。
`remove_file(path)` 删除目录项。

File 不实现 Copy/Clone。Drop 关闭剩余描述符；显式 close 先退休句柄，再报告
关闭结果，失败也不会让析构再次关闭一个可能已被复用的描述符。需要知道关闭
失败时使用显式 close。关闭不等同于持久化，尚未提供 fsync。

首版路径和写入文本是 comptime 值。动态路径、目录遍历、元数据、随机访问与
缓冲 I/O 尚未实现。
