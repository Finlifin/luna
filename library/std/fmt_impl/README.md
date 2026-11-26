# 格式分析、文本与值输出

从 `std.prelude` 显式导入或通过 `std.fmt.println` 调用：

```flurry
fn show(value: u32) -> !std.io.IoErr void {
    std.fmt.println("{format: .hex, align_to: .right, len: 12, placeholder: '0'}", value)
}
```

`std.fmt.FormatPlan.parse(format)` 是已接通的 comptime 扫描入口，返回
`!FormatErr FormatPlan`。计划中的 `pieces` 按输出顺序记录原文的半开 UTF-8
字节区间 `FormatPiece.text(start, end)` 与从零开始的参数槽位：

- `{}` 记录为 `FormatPiece.argument(index, start, end)`（空属性子串 `""`，要求 `Display`）；
- `{key: value, ...}` 校验 Flurry `property` 列表语法（键为标识符、值为 `.symbol` /
  `bool` / 整数 / `'char'` / `"str"`、支持逗号与尾逗号、引号内屏蔽 `}` 并校验转义、
  禁止重复键）：
  - 当属性列表包含 `format: .debug` 时，生成 `FormatPiece.debug_argument(index, start, end)`（要求 `Debug`，并支持 `align_to` / `len` / `placeholder`）；
  - 其他属性列表生成 `FormatPiece.argument(index, start, end)`（要求 `Display` 或 comptime `ComptimeDisplay`），并将原始属性切片经 `FormatPlan.argument_specs` 原样传给目标类型的 `Display.fmt` 或 `ComptimeDisplay.fmt`；
- `{{`／`}}` 保留其中一个花括号的原文字节；NUL 不终止扫描。

未闭合的左括号、孤立的右括号和非法字段语法（包括旧版 `{:x}` / `{:?}` / `{:#x}`、
编号/裸名 `{0}` / `{name}`、重复键、空白 `{ }` 或非法转义）分别返回
`UnclosedField`、`UnmatchedClose` 与 `UnsupportedField`，附带原文字节偏移。
计划不检查实参、不赋予格式化能力，也不创建 runtime 字符串。

`std.fmt.Parse(format)` 与 `std.fmt.ComptimeParse(format)` 调用扫描器，按字段顺序构造普通的 `meta.List<Type>`，
分别携带 `refines { Itself :- Display } Any` / `refines { Itself :- Debug } Any` 与
`refines { Itself :- meta.ComptimeDisplay } Any`，再把列表具名交给 `Tuple`。外部不再拆出
`LowerHex`/`UpperHex`/`Binary`/`Octal` 或 `FixedPrecision` 等 trait，格式选项的解释权完全归各类型的 `Display` / `ComptimeDisplay` 实现所有。
字段分发在 `write_fmt_tuple`（与 comptime `comptime_format`）中通过
`'tuple_indexed` 按槽位展开：runtime 借用实参并调用
`value.ref.as(Display).fmt(output, .format = field_fmt)`（不会消费仿射 `Once` 值），
comptime 调用 `value.as(meta.ComptimeDisplay).fmt(.format = field_fmt)`。

`println`/`eprintln`/`print`/`eprint` 与泛型 `write_fmt<W :- ByteWriter>`、`format`、
`format_into` 与 `comptime_format` 按格式写出文本与字段；其中 `println`/`eprintln`
末尾附加换行。写入失败立即返回，并保留已经输出的前缀。
`format` 与 `format_into` 通过运行时依赖的 `allocator` 构建或追加拥有型 `String`，
`print_value`/`println_value` 借用值调用 `Display.fmt(output.ref)`。
`std.fmt` 再导出同一 `Display`、`Debug`、`FormatOptions` 与 `options`。

当前支持：`bool`、全部 12 种机器整数（runtime 与 comptime 两相，支持
`format: .dec`/`.hex`/`.HEX`/`.oct`/`.bin`、`alt`、`sign`、`align_to`、`len`、
`placeholder`）、任意精度 `Integer`（comptime 相）、IEEE 浮点（runtime 相
`f16`/`bf16`/`f32`/`f64`，支持最短往返十进制、固定 `precision`、`sign`、
`align_to`、`len`、`placeholder` 与 `NaN`/`inf`/`-inf`/`-0`）、`char`、`str`
（含 UTF-8 标量 `precision` 截断）、`Symbol`、`String`（借用 `*self` + `precision`
与对齐填充）、`Duration`/`ExitCode`/容器及错误类型的 `{format: .debug, ...}` 格式化。
