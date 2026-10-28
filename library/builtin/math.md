# builtin.math 的原生验收

`math` 由 `builtin.prelude` 默认导入，使用者直接写 `math.sqrt<f32>(81.0)`。
它仍是 builtin 的模块；不需要创建独立 math 包，也不需要手工指定 intrinsic
或 Copy/Drop 绑定。声明和 LLVM 操作的对应关系由 sysroot 提供。

真实消费者示例见
[host-math-tests.fl](../../development/examples/host-math-tests.fl)。
示例只使用公开类型，没有直接访问编译器内置符号。十一项原生测试覆盖：

- f32/f64 的算术、舍入、负零、NaN 与最小次正规数。
- 位计数的零输入、位反转、字节交换及有符号位组合。
- 旋转量超过位宽或等于位宽，以及 funnel 两个输入不同的情况。
- 有符号和无符号饱和边界，最小负数的 wrapping_abs。
- 多个参数的源序求值，以及旋转包装复用输入时不重复求值。

在仓库根目录执行：

```sh
python3 -B development/library_check.py --release --timeout 900 --stage native-math
```

单独验证优化后的原生产物：

```sh
python3 -B development/check.py --timeout 900 -- run --release -p luna -- development/examples/host-math-tests.fl --sysroot library --test-native --fuel 2000000 --opt-level 3
```

将最后的 `3` 改为 `0` 可验证 O0。入口显式提供 200 万步预算：完整 sysroot
下 wrapping_abs 的契约检查会耗尽默认 10 万步预算。这里没有关闭契约或资源
检查，也没有改变整数溢出规则。该验收只覆盖 host pipeline 的 math 接口，
不代表全部内置包完成，也不承诺这些 LLVM 操作已能在 comptime VM 执行。

最终 residual 契约验证已省去供分析报告使用的逐检查点事实、聚合值和字段
路径快照；证明及成功路径上的事实更新仍然执行。优化器使用的分析入口继续
保留完整报告。该优化降低了验证开销，但尚不足以让上述全部用例在 10 万步
内完成，因此验收入口仍保留显式预算。

后续验证又省去了同一事实状态上的立即证明重放，并为无写入的标量短路
表达式保留入口状态，避免分支结束时重建相同存储的关系。右侧算术仍在
对应分支内检查溢出；含调用、指针读取或赋值的表达式不使用这条路径。
独立 i8 wrapping_abs 实例的 residual 验证从 135,634 步降到 128,643 步
（约 5.2%）。这是确定性工作预算的测量，不是端到端耗时，也不代表默认
10 万步已足够。回归同时检查错误保护条件的拒绝和右侧赋值的执行顺序。

`isnan`、`isinf`、`isfinite`、`isnormal`、`issubnormal` 和 `signbit`
也接受 `f16`、`bf16`。分类按原始格式进行，不先扩宽到 f32；因此次正规数
分类不会改变，符号位检查能识别负零与带符号 NaN。对应的 `meta.math`
函数遵循相同规则。半精度分类的 sysroot 集成测试覆盖 comptime 与 LLVM O0/O3；
这不代表其余半精度数学包装已实现。

半精度还支持 `abs`、`copysign` 和四种极值操作。前两者只修改符号位，
保留 NaN 载荷与 signaling 位。`minimum/maximum` 传播 NaN；
`minimumnum/maximumnum` 在仅一侧为 NaN 时选择数值。最小值选择负零，
最大值选择正零。`meta.math` 提供相同的 comptime 行为。
