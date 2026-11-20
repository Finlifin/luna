# 读取接口与适配

`std.io.Reader` 提供同步字节输入协议（`assoc Err`、`read_raw`、`read_byte` 及默认方法
`read_into`/`read_exact`/`read_to_end`/`read_to_string`）。已实现：
- `Stdin` 与 `std.io.stdin()`、`File` 实现了 `Reader`（`Err = IoErr`；EOF 返回 `0` 或 `null`，`EINTR` 自动重试）；
- `Cursor<.A, .allocator>` 基于 `Vec<u8, .A, .allocator>` 提供内存读取与定位；
- `BufReader<R, .A, .allocator>`（位于 `buffered.fl`）支持 `refill`、`consume`、`peek_byte`、`read_byte`、`read_until` 与 `read_line`。

首版标量辅助 `std.io.read_byte(fd)` 与 `File.read_byte()` 继续可用。
