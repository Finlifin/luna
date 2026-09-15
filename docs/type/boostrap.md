# 类型与编译时计算

从[类型系统总览](index.md)进入完整阅读路线。类型值、身份、参数化、精化、数据声明、行与环境分别在对应章节说明。

编译时阶段规则见[comptime 总览](../comptime/index.md)，操作检查见[normalization 与 instantiation](../comptime/normalization-and-instantiation.md)，类型工厂见[纯性与范式](../comptime/purity-and-normal-form.md)。

类型尚未具体化不意味着没有可用事实；当前契约可以支持操作，需要具体输入的计算则保留其依赖。
