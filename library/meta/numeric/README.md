# 编译期数学

scalar_math.fl 是与 builtin.math 对应的 comptime 标量函数，通过 VM intrinsic 执行。公开入口 meta.math；不在 meta.prelude 中导出 math，以免遮蔽 runtime math。当前机器数值域与任意精度运算的边界见 ../math.md。
