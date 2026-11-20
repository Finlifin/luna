# 读取接口与适配

首版通过 `std.io.read_byte(fd)` 或 `File.read_byte()` 读取一个字节，返回
`!IoErr ?u8`：EOF 为 null，系统错误走错误限定类型；EINTR 自动重试。
实际适配位于 `platform/unix.fl`。缓冲、字节切片读取和文本解码尚未实现。
