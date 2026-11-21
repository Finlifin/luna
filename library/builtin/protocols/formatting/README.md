# 格式协议

`builtin.fmt.Display`、`builtin.fmt.Debug`、`builtin.fmt.{LowerHex, UpperHex, Binary, Octal}`
与 `builtin.fmt.ByteWriter` 是不依赖 std、String 或 allocator 的 runtime 协议。
Display 和 Debug 同时进入 builtin.prelude；ByteWriter 与进制格式化 trait
通过具名导入使用。std.fmt.Display、std.fmt.Debug、std.fmt.{LowerHex, UpperHex, Binary, Octal}
与 std.io.ByteWriter 只是同一声明的再导出。

```flurry
use builtin.fmt.ByteWriter;

fn write_value(value: T, output: *W) -> !W.as(ByteWriter).Err void
where T :- Display, W :- ByteWriter
{
    value.write(output)
}
```

ByteWriter 借用接收者，不要求 Clone/Copy。write_byte 成功恰好接受一个字节；
失败时不接受本次字节，前面已经接受的字节保持有效，错误域由具体 writer 的
关联 Err 决定。Display 不把错误转换成 std.io.IoErr，也不消费 writer。

当前协议只承诺单字节操作；多字节格式化按顺序调用，首次失败即停止，不回滚
已写前缀，也不返回已写长度。调用者不能在失败后盲目重试整个格式化操作；恢复
方式由具体 writer 的状态与错误契约决定。完整文本/批量写入需要独立的进度契约。

整数、bool、unit (`void`)、IEEE 浮点（`f16`、`bf16`、`f32`、`f64`）、Unicode 标量
`char`、`str` 与 `Symbol` 的 `Display`/`Debug` 实现，以及全部 12 种机器整数的
`LowerHex`/`UpperHex`/`Binary`/`Octal` 实现在此目录。std 的文件、`Vec<u8>` 与标准输出
提供 ByteWriter 实现；用户 writer 可以维护自己的状态、错误和重试策略，不需要文件描述符。
浮点格式化采用纯静态 128 位缩放算法，保证有限非零值的最短十进制可往返还原输出，并支持
`NaN`/`inf`/`-inf`/`-0`。
格式选项（宽度/对齐/精度）和批量进度接口仍未交付。
