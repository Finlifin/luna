# builtin

提供语言基础类型、跨阶段协议、runtime 数学与基础文本接口。
源码与未来领域按 [六包结构](../STRUCTURE.md) 组织。
公开导出和默认 prelude 见 lib.fl，具体领域说明见对应子目录。

`primitives.fl` 集中绑定并导出 `f16`、`bf16`、`f32`、`f64`。
`f16` 与 `bf16` 在 NIR 中具有不同的 primitive 身份，均不别名为 `f32`；
它们可出现在类型查询与函数签名中，不能隐式互换。
两者已接通精确十进制字面量、comptime invocation、VM 值传递、常量物化与泛型字面量特化。
舍入直接面向目标格式，避免经 f32/f64 中转造成二次舍入；负零保留其位模式。
LLVM 分别使用 `half` 与 `bfloat` 表示，支持上述值的参数传递和返回。
两者已提供 runtime Clone/Copy，标量复制与纯度分类保持一致；克隆保留原始位模式。
已支持 NaN/Infinity 常量、相等与有序比较，以及字面量模式匹配。
NaN 的有序比较为 false，正负零比较相等；VM、常量折叠和 LLVM 使用相同规则。
加减乘除、数值转换与位视图仍待接通；
`builtin.math` 与 `meta.math` 当前的浮点运算域仍为 f32/f64。
