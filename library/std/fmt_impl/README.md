# 格式分析、文本与值输出

从 `std.prelude` 显式导入或通过 `std.fmt.println` 调用：

```flurry
fn show(value: u32) -> !std.io.Error void {
    std.fmt.println("value = {}, ready = {}", value, true)
}
```

`std.fmt.FormatPlan.parse(format)` 是已接通的 comptime 扫描入口，返回
`!FormatError FormatPlan`。计划中的 `pieces` 按输出顺序记录原文的半开 UTF-8
字节区间 `FormatPiece.text(start, end)` 或从零开始的
`FormatPiece.argument(index)`；`argument_count` 为所需参数数目。
空文本产生空计划，普通文本保留为整段，`{}` 消耗一个参数，`{{`／`}}`
保留其中一个花括号的原文字节。NUL 不终止扫描。

未闭合的左括号、孤立的右括号和尚未支持的非空字段分别返回
`unclosed_field`、`unmatched_close`、`unsupported_field`，附带原文字节偏移。
编号、命名、宽度和精度规则尚未实现，不将这些内容静默当作普通 `{}`。
计划不检查实参、不赋予 Display 能力，也不创建 runtime 字符串。

`std.fmt.Parse(format)` 调用扫描器，按字段数构造普通的 `meta.List<Type>`，
每项携带 `refines { Itself :- Display } Any`，再把列表具名交给 Tuple。
经 `defer` 使用时，调用方逐项检查实际类型；完整具名参数也遵循相同约束。
无效格式在 comptime 拒绝，扫描器本身可以提供错误种类和字节位置。
开放定义从待算结果的共同返回头取得元组遍历依据。字段上的 `inline if`
在 Display 成立的分支中检查方法调用，实例按实际字段类型展开。

`println`/`eprintln`/`print`/`eprint` 与泛型 `write_fmt<W :- ByteWriter>`、`format`、`format_into` 按格式写出文本与字段；其中 `println`/`eprintln` 末尾附加换行。
当前实例按字节或字面片段顺序输出；写入失败立即返回，并保留已经输出的前缀。
`format` 分配并构建拥有型 `std.string.String`，`format_into` 将格式化结果追加到既有 `*String`，两者均返回 `!std.mem.AllocError`。
`print_value`/`println_value` 调用 builtin 的 runtime `Display.write(self, output)`，output 为泛型 writer 的借用。
标准输出入口返回 `!std.io.Error`，使用后缀 `!` 传播；`write_fmt` 与 `Display` 保留实际 writer 的关联错误类型。
协议与标量/`char`/`str` 实现在 builtin/protocols/formatting，`String` 实现在 `std.string_impl`，`std.fmt` 再导出同一 `Display`。

当前支持 bool、机器有符号/无符号整数（含 i128 最小值与 u128 最大值）、Unicode 标量 `char`、`str` 与拥有型 `std.string.String` 的格式化输出。没有借用开发用 S-expression 结果打印器。

尚未实现宽度、精度与浮点格式化。
