# builtin

提供语言基础类型、跨阶段协议、runtime 数学与基础文本接口。
源码与未来领域按 [六包结构](../STRUCTURE.md) 组织。
公开导出和默认 prelude 见 lib.fl，具体领域说明见对应子目录。

`primitives.fl` 集中绑定并导出 `f16`、`bf16`、`f32`、`f64`。
`f16` 与 `bf16` 在 NIR 中具有不同的 primitive 身份，均不别名为 `f32`；
它们可出现在类型查询与函数签名中，不能隐式互换。
目前前两者仅接通类型层；字面量、算术、转换、资源协议和 LLVM 数值执行仍待实现。
`builtin.math` 与 `meta.math` 当前的浮点运算域仍为 f32/f64。
