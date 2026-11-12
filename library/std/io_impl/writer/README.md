# 写入

`Writer` 是 stdout/stderr 的非拥有句柄，可 Copy；不从 File 导出可逃逸的句柄。
`write_all<text>()` 写完文本，包括内嵌 NUL；短写继续，EINTR 重试，无进展返回
`Error.write_zero`。错误之前可能已写出前缀，不承诺事务性或整行原子性。

逐字节输出用于首版整数格式化。缓冲、批量字节切片、计数与限流 writer 尚未实现。
