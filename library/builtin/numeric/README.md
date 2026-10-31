# 运行时数学与比较

comparison.fl 定义 Eq、Ord、Order；math_intrinsics.fl 实现 runtime 标量包装。包根的 math facade 导出这些接口。编译期标量计算在 meta/numeric，二者阶段和 intrinsic 注册分开。

数值模块统一复用包根从 primitives.fl 导出的具名类型绑定，不在使用处访问编译器符号，也不重复声明 primitive 别名。独立编译库片段的测试夹具同样加载 primitives.fl；真实 sysroot 验证使用包根的正常导出。
