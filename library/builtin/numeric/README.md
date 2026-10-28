# 运行时数学与比较

comparison.fl 定义 Eq、Ord、Order；math_intrinsics.fl 实现 runtime 标量包装。包根的 math facade 导出这些接口。编译期标量计算在 meta/numeric，二者阶段和 intrinsic 注册分开。
