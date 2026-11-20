# 写入

`std.io.Write` 定义同步字节输出协议（`assoc Err`、`write_raw`、`write_all_raw`、`flush` 与
默认 `write_bytes`/`write_string`），已由 `Writer`、`File`、`Vec<u8, .A, .allocator>` 与
`BufWriter<W, .A, .allocator>` 实现；`BufWriter` 在 `Drop` 时自动刷新缓冲字节并实现 `ByteWriter`。

`Writer` 是 stdout/stderr 的非拥有句柄，可 Copy；不从 File 导出可逃逸的句柄。
`write_all<text>()` 写完文本，包括内嵌 NUL；短写继续，EINTR 重试，无进展返回
`IoErr.WriteZero`。错误之前可能已写出前缀，不承诺事务性或整行原子性。
