# 编译时计算与类型

Flurry 的类型可以作为编译时计算的输入和结果。定义带有子句参数时，编译器先依据参数及操作契约检查定义，再在具体输入具备后继续计算。

类型系统与编译时计算的交界由以下章节分别定义：

| 主题 | 文档 |
|---|---|
| comptime/runtime 与两阶段检查 | [编译时计算](../comptime/index.md) |
| ordinary/loose、操作契约、defer、PTC assumes | [Normalization 与 instantiation](../comptime/normalization-and-instantiation.md) |
| 返回类型的编译时函数、范式头、子句默认值 | [Comptime 函数、纯性与范式](../comptime/purity-and-normal-form.md) |
| 值的阶段边界、proxy、Splice | [跨阶段值传递](../comptime/quoting-and-splicing.md) |
| Trait 两相与 coherence | [Trait 的 comptime/runtime 两相](../comptime/the-comptime-phases-of-traits.md) |
| 形状值参与类型和代码生成 | [静态形状的张量加法](../comptime/static-shaped-tensors.md) |

PTC 使用操作要求和结果命题支持定义端检查。PTC 命题具有可执行语义；`assumes` 在相应实例化中执行。运行时程序验证与 PTC 共享部分语法，但分别承担各自的检查职责。

一个类型尚未具体化，不等于它没有可用性质。当前上下文提供的契约可以支持后续操作；需要具体结构的计算则继续依赖其编译时输入。
