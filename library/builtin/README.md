# builtin

提供语言基础类型、跨阶段协议、runtime 数学与基础文本接口。
源码与未来领域按 [六包结构](../STRUCTURE.md) 组织。
公开导出和默认 prelude 见 lib.fl，具体领域说明见对应子目录。

`primitives.fl` 集中绑定并导出 `f16`、`bf16`、`f32`、`f64`。
`f16` 与 `bf16` 在 NIR 中具有不同的 primitive 身份，均不别名为 `f32`；
它们可出现在类型查询与函数签名中，不能隐式互换。
两者已接通精确十进制字面量、comptime invocation、VM 值传递、常量物化与泛型字面量特化。
舍入直接面向目标格式，避免经 f32/f64 中转造成二次舍入；负零保留其位模式。
LLVM 对 f16 使用 `half`；bf16 的存储与内部调用使用保留原始位模式的 `i16`，
只在浮点算术处转换为 LLVM `bfloat`，避免目标降码在传值时改写 signaling NaN。
这不合并 bf16 与 u16 的语言类型身份，也不承诺外部平台的 bf16 FFI ABI。
两者已提供 runtime Clone/Copy，标量复制与纯度分类保持一致；克隆保留原始位模式。
已支持 NaN/Infinity 常量、相等与有序比较，以及字面量模式匹配。
NaN 的有序比较为 false，正负零比较相等；VM、常量折叠和 LLVM 使用相同规则。
已支持加减乘除与一元取负，包括泛型运算、comptime invocation 和 IIR 常量折叠。
算术直接按目标格式最近偶数舍入，溢出产生带符号无穷；算术 NaN 规范化，
一元取负仅翻转符号位，保留 NaN 载荷。VM 与 LLVM O0/O3 路径有对应回归用例。
已支持 `value'bitvec` 读取 16 位表示、同宽 `.as(f16)`／`.as(bf16)`
重解释和可写位置的位段更新；这些操作保留负零与 NaN 载荷，不做数值舍入。
数值转换仍待接通；
`builtin.math` 与 `meta.math` 的六个分类函数已支持 f16/bf16，
包括 `isnan`、`isinf`、`isfinite`、`isnormal`、`issubnormal` 和 `signbit`；
`abs`、`copysign`、`minimum`、`maximum`、`minimumnum`、`maximumnum`
也已接通半精度；符号操作保留其余位模式，极值操作区分 NaN 策略与正负零。
其余数学包装当前的浮点运算域仍为 f32/f64。
