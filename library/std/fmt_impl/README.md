# 格式分析、文本与值输出

目标入口是作者指定的
`fn println(comptime format: str, ...varargs: defer std.fmt.Parse(format))`。
格式串确定变参数量与每项约束，实参保留各自的具体类型，并按普通调用规则
求值一次。下面的文本输出与 `println_value` 是已有底层能力，不能当作该
格式化接口的完成版本。

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

`Parse` 的类型描述结果、描述的 VM 传递与调用实例化尚未接通；不得用固定
整数 tuple、统一装箱或识别 println 名称代替。具体接线见
`development/runtime-value-delivery.md` 的格式化变参边界。

`print`/`println` 输出 comptime 文本；`eprint`/`eprintln` 输出到 stderr。
`print_value`/`println_value` 调用builtin 的 runtime `Display.write(self, output)`，output 为泛型 writer 的借用。
标准输出入口返回 `!std.io.Error`，使用后缀 `!` 传播；Display 本身保留实际 writer 的关联错误类型。
协议与标量实现在 builtin/protocols/formatting，std.fmt 只再导出同一 Display。

当前支持 bool、机器有符号/无符号整数的十进制输出，包括 i128 最小值、u128
最大值。负数先在自己的有符号域取得安全幅值，再转到无符号域，避免窄整数
零扩展造成负数输出错误。没有借用开发用 S-expression 结果打印器。

尚不支持格式串参数替换、宽度/精度、浮点、运行时 str 或 String。它们需要继续
接通 comptime 展开和文本来源契约，不由编译器识别 println 名称来实现。
