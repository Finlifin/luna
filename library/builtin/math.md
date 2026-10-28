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
