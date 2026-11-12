# 文本与值输出

`print`/`println` 输出 comptime 文本；`eprint`/`eprintln` 输出到 stderr。
`print_value`/`println_value` 调用builtin 的 runtime `Display.write(self, output)`，output 为泛型 writer 的借用。
标准输出入口返回 `!std.io.Error`，使用后缀 `!` 传播；Display 本身保留实际 writer 的关联错误类型。
协议与标量实现在 builtin/protocols/formatting，std.fmt 只再导出同一 Display。

当前支持 bool、机器有符号/无符号整数的十进制输出，包括 i128 最小值、u128
最大值。负数先在自己的有符号域取得安全幅值，再转到无符号域，避免窄整数
零扩展造成负数输出错误。没有借用开发用 S-expression 结果打印器。

尚不支持格式串参数替换、宽度/精度、浮点、运行时 str 或 String。它们需要继续
接通 comptime 展开和文本来源契约，不由编译器识别 println 名称来实现。
