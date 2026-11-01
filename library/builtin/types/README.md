# 语言类型接口

Tuple 的 normal-form 构造入口，以及原生 ?T、!E T 的接口。option.fl 和 result.fl 不定义 Option/Result 枚举。这里只提供语言类型的库接口，不替代编译器的类型身份和 ABI。

`equality.fl` 为 `?T` 和单错误类型的 `!E T` 实现 runtime `Eq`。每个
载荷类型都必须实现 Eq。可选类型按存在性比较，错误结果先区分成功／错误；
只有对应分支调用载荷比较。实现消费输入，遵循普通仿射移动和清理，不要求
载荷 Copy，也不借用 comptime 的值引用语义。未参与比较的载荷同样正常清理。

comptime 上下文由独立的 `meta.ComptimeEq` 实现提供同名方法。真实 sysroot
验收 `development/examples/sysroot-qualified-equality.fl` 覆盖非 Copy 载荷的
Drop、所有分支组合以及两阶段选择。runtime Hash 和多错误标签行尚未提供。
