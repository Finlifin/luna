# 基础文本

`bytes.fl` 定义 runtime `TextBytes` 协议及 str 的实现，经 builtin 和
builtin.prelude 导出。`builtin.text.TextBytes` 保留作为显式协议路径。

```flurry
fn bytes() -> bool {
    "é".byte_length() == 2 and ("é".byte(1) matches 169?)
}
```

`byte_length()` 返回 u64 字节数；`byte(index: u64)` 返回 ?u8；`slice(start: u64, end: u64)` 返回 ?str。索引按 UTF-8
字节计数，越界返回 null，包括空字符串的任何索引。它们不解码 Unicode 字符。

comptime 对应协议为 meta.ComptimeTextBytes，使用相同方法名，从 meta.prelude
默认导入。它是 pure comptime trait，在 comptime 上下文优先选择；方法的 pure
仍表示函数纯度，trait 上的 pure 表示只包含 comptime 内容。显式指定 runtime
协议不会绕过阶段检查。旧的 text.byte、text.byte_length、comptime_byte 和
comptime_byte_length 自由函数已移除，调用方迁为接收者方法或 str 上的关联调用。

str 属于 builtin，拥有型 String 属于 std/string。编码策略和 I/O 适配不放在这里。
Symbol 的底层字节观察仍由编译器操作提供，尚未在此协议中提供库实现。

带完整资源检查的跨函数 str 参数传递与方法调用（包括 `development/repros/native-text-method-provenance.fl`）
已通过背后的静态文本来源与 `TextBytes` 方法接通；`slice` 与字节观察共同通过 `builtin.text.TextBytes` / `builtin.prelude` 导出。
