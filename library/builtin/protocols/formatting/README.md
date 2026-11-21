# 格式协议

`builtin.fmt.Display`、`builtin.fmt.Debug` 与 `builtin.fmt.ByteWriter`
是不依赖 std、String 或 allocator 的格式化协议。`Display` 与 `Debug`
进入 `builtin.prelude`；`ByteWriter`、`FormatOptions` 与 `options` 辅助模块通过
`builtin.fmt` 导出（`std.fmt` 再导出同一声明）。

```flurry
use builtin.fmt.ByteWriter;

pub trait Display {
    fn fmt(*self, output: *W, comptime .format: str = "") -> !W.as(ByteWriter).Err void
    where
        W :- ByteWriter;

    ^attrs.comptime_phase
    comptime fn comptime_fmt(self, .format: str = "") -> str;
}

fn write_value(value: *T, output: *W) -> !W.as(ByteWriter).Err void
where
    T :- Display,
    W :- ByteWriter,
{
    value.as(Display).fmt(output)
}
```

`Display` 将 runtime 与 comptime 两相统一在同一协议上，并由可选默认参数
`comptime .format: str = ""` 将字段选项下放给具体类型解析：

- runtime `fmt(*self, output: *W, comptime .format: str = "")` 借用接收者，
  因此格式化仿射 `Once` 值时不会移动或销毁它；
- comptime `comptime_fmt(self, .format: str = "") -> str` 在编译期产生格式化文本，
  已为 `bool`、全部 12 种机器整数、任意精度 `Integer`、`char`、`str` 与 `Symbol`
  提供 `comptime impl Display`；
- 进制与定点精度不再拆成外部 `LowerHex`/`UpperHex`/`Binary`/`Octal` 或
  `FixedPrecision` trait，而是由类型自身的选项解析（例如 `builtin.fmt.options`
  的 `FormatOptions` + `Splice` 传递及内部 helper）在编译期完成验证并在
  runtime 零分配执行。

ByteWriter 借用接收者，不要求 Clone/Copy。`write_byte` 成功恰好接受一个字节；
失败时不接受本次字节，前面已经接受的字节保持有效，错误域由具体 writer 的
关联 `Err` 决定。`Display` 不把错误转换成 `std.io.IoErr`，也不消费 writer。

ByteWriter 同时提供 `write_str` 与 `unsafe fn write_raw_bytes(src, len)` 批量
接口（默认回退到按字节 `write_byte` 顺序写入）；std 的 `Writer`、`File`、
`Vec<u8>` 与 `BufWriter` 对 `write_raw_bytes` 提供批量覆盖实现，`String.write_to`
与 `String.fmt` 直接走 `write_raw_bytes`。多字节写入与多步格式化在首次失败时即停止，
不回滚已写前缀，也不返回已写长度。

内建实现覆盖：`bool`、全部 12 种机器整数（支持 `format: .dec`/`.hex`/`.HEX`/
`.oct`/`.bin`、`alt: true/false`、`sign: .minus`/`.plus`/`.space`、`align_to`、
`len`、`placeholder`）、任意精度 `Integer`（comptime 相，支持上述整数选项）、
IEEE 浮点（runtime 相 `f16`/`bf16`/`f32`/`f64`，支持最短往返十进制、固定
`precision`、`sign`、`align_to`、`len`、`placeholder` 与 `NaN`/`inf`/`-inf`/`-0`）、
Unicode 标量 `char`、`str`（支持 UTF-8 标量截断 `precision` 与对齐填充）及 `Symbol`。
`Debug` 借用接收者并使用相同的 UTF-8 字符计数与填充辅助支持 `{format: .debug, ...}`。
