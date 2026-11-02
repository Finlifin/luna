# 语言类型接口

Tuple 的 normal-form 构造入口，以及原生 ?T、!E T 的接口。option.fl 和 result.fl 不定义 Option/Result 枚举。这里只提供语言类型的库接口，不替代编译器的类型身份和 ABI。

`equality.fl` 为 `?T` 和单错误类型的 `!E T` 实现 runtime `Eq`。每个
载荷类型都必须实现 Eq。可选类型按存在性比较，错误结果先区分成功／错误；
只有对应分支调用载荷比较。实现消费输入，遵循普通仿射移动和清理，不要求
载荷 Copy，也不借用 comptime 的值引用语义。未参与比较的载荷同样正常清理。

comptime 上下文由独立的 `meta.ComptimeEq` 实现提供同名方法。真实 sysroot
验收 `development/examples/sysroot-qualified-equality.fl` 覆盖非 Copy 载荷的
Drop、所有分支组合以及两阶段选择。多错误标签行尚未提供。

`hashing.fl` 为相同的限定类型提供 runtime `Hash`，要求所有载荷类型实现
Hash。每个存在的载荷只调用一次协议方法，并按普通所有权规则清理；空可选值
不调用载荷方法。编码与 meta 对应实现一致：可选值先输入不存在／存在的
0／1 字节，错误结果先输入成功／错误的 0／1 字节，再混合载荷的完整小端
u64 摘要。混合函数共享于两类限定类型；乘法显式保留低 64 位，不依赖溢出。

native 验收使用非 Copy 载荷记录调用和清理，并与 comptime 求得的摘要比较。
Hash 不要求 Eq，但摘要碰撞不能替代值相等；调用方仍须使用适用的 Eq 协议。

摘要混合辅助函数声明为 `pure fn`。程序验证器利用 BitVec 到无符号整数
重建时的源宽度上界证明范围条件，P4 分析据此识别不会失败的检查；源码中的
`assumes` 和显式低位截断均保留，不以取消检查来取得纯度。
