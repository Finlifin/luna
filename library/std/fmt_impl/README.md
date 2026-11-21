# 格式分析、文本与值输出

从 `std.prelude` 显式导入或通过 `std.fmt.println` 调用：

```flurry
fn show(value: u32) -> !std.io.IoErr void {
    std.fmt.println("value = {}, ready = {}", value, true)
}
```

`std.fmt.FormatPlan.parse(format)` 是已接通的 comptime 扫描入口，返回
`!FormatErr FormatPlan`。计划中的 `pieces` 按输出顺序记录原文的半开 UTF-8
字节区间 `FormatPiece.text(start, end)` 与从零开始的参数槽位：`{}` 对应
`FormatPiece.argument(index)`（`Display`），`{:?}` 对应 `FormatPiece.debug_argument(index)`
（`Debug`），`{:x}`/`{:#x}`、`{:X}`/`{:#X}`、`{:b}`/`{:#b}`、`{:o}`/`{:#o}` 分别对应
`lower_hex_argument`（`LowerHex`）、`upper_hex_argument`（`UpperHex`）、`binary_argument`
（`Binary`）与 `octal_argument`（`Octal`，布尔载荷标记是否输出 `0x`/`0X`/`0b`/`0o`
前缀）；`argument_count` 为所需参数数目。
空文本产生空计划，普通文本保留为整段，`{{`／`}}` 保留其中一个花括号的原文字节。NUL 不终止扫描。

未闭合的左括号（包括未闭合的 `{:`、`{:?`、`{:#`、`{:#x` 等）、孤立的右括号和尚未支持的
其他字段语法（如 `{0}`、`{:#}`、`{:#?}`、`{:q}`、`{:xx}`）分别返回 `UnclosedField`、
`UnmatchedClose`、`UnsupportedField`，附带原文字节偏移。
编号、命名、宽度、对齐与精度规则尚未实现，不将这些内容静默当作普通 `{}`。
计划不检查实参、不赋予格式化能力，也不创建 runtime 字符串。

`std.fmt.Parse(format)` 调用扫描器，按字段顺序构造普通的 `meta.List<Type>`，
分别携带 `refines { Itself :- Display / Debug / LowerHex / UpperHex / Binary / Octal } Any`，
再把列表具名交给 Tuple。
经 `defer` 使用时，调用方逐项检查实际类型；完整具名参数也遵循相同约束。
无效格式在 comptime 拒绝，扫描器本身可以提供错误种类和字节位置。
开放定义从待算结果的共同返回头取得元组遍历依据。字段分发经 `inline if`
分别在对应 trait（`Display` 按值，`Debug` 及各进制 trait 按借用 `*T`）成立的分支中
检查方法调用，实例按实际字段类型展开。

`println`/`eprintln`/`print`/`eprint` 与泛型 `write_fmt<W :- ByteWriter>`、`format`、`format_into` 按格式写出文本与字段；其中 `println`/`eprintln` 末尾附加换行。
当前实例按字节或字面片段顺序输出；写入失败立即返回，并保留已经输出的前缀。
`format` 分配并构建拥有型 `std.string.String<.A = A, .allocator = a>`（默认 `.A = DefaultAllocator, .allocator = DefaultAllocator.GLOBAL.ref`），`format_into` 将格式化结果追加到既有 `*String<.A = A, .allocator = a>`，两者均通过运行时环境参数交付分配器且返回 `!std.mem.AllocErr`。
`print_value`/`println_value` 调用 builtin 的 runtime `Display.write(self, output)`，output 为泛型 writer 的借用。
标准输出入口返回 `!std.io.IoErr`，使用后缀 `!` 传播；`write_fmt` 与 `Display`/`Debug` 保留实际 writer 的关联错误类型。
协议与标量/`f16`/`bf16`/`f32`/`f64`/`char`/`str`/`Symbol` 实现在 builtin/protocols/formatting，`String<.A = A, .allocator = a>` 与容器/时间/进程/错误 `Debug` 实现位于 `std`，`std.fmt` 再导出同一 `Display`、`Debug`、`LowerHex`、`UpperHex`、`Binary`、`Octal`。

当前支持 bool、全部 12 种机器整数（`Display`/`Debug`/十六进制/二进制/八进制）、IEEE 浮点（`f16`/`bf16`/`f32`/`f64` 最短可往返 `Display`/`Debug`）、Unicode 标量 `char`、`str`、`Symbol`、`Duration`/`ExitCode`、分配型容器及标准错误类型的 `{}`/`{:?}` 格式化输出。没有借用开发用 S-expression 结果打印器。

尚未实现宽度、对齐、精度与位置/命名参数。
