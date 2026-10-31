# 运行 Flurry host 示例

从仓库根目录执行。运行器构建当前工作区的 luna，经 LLVM 生成 host 可执行文件，再实际运行；无需手工调用 Cargo。

```sh
python3 -B development/run_host.py development/examples/host-arithmetic.fl --release-driver --build-timeout 900
```

预期 stdout 为 `(unsigned-integer 42)`。默认入口是源码中的 `run`。`--entry NAME` 可选择其他入口；`--opt-level 3` 开启生成程序的 LLVM O3 优化，`--release-driver` 则只控制编译器自身的构建配置。

带库依赖或协议绑定的例子通过目录运行器启动，它会传入所需包和声明绑定：

```sh
python3 -B development/host_examples.py arithmetic number-theory library-tuple list-clauses --release-driver --compare-optimization --timeout 900
```

| 示例 | 内容 | 预期 stdout |
| --- | --- | --- |
| arithmetic | 函数调用和整数算术 | `(unsigned-integer 42)` |
| number-theory | 循环与整数算法 | `(unsigned-integer 1121)` |
| library-tuple | 实际库中的 Tuple 与资源协议 | `(bool true)` |
| list-clauses | 类型列表、延后签名和捕获声明值的函数工厂 | `(bool true)` |
| staged-bitvec | 声明子句及普通 comptime 输入决定 BitVec 宽度 | `(bool true)` |

`--compare-optimization` 分别运行 O0/O3 并检查结果。终端显示 `/tmp/luna-host-examples-…` 或 `/tmp/luna-run-host-…` 产物目录，包含汇总、构建／执行日志和生成程序。失败时先看 `summary.json` 中的失败阶段；不要只凭旧可执行文件判断当前源码成功。

工具链默认使用 `llvm-config-21` 和 `cc`，可用 `LUNA_LLVM_CONFIG`、`LUNA_CC` 指定已有安装。LLVM 的验证、优化、目标文件生成及链接都属于原生链路。

这里的返回值打印由开发驱动生成，不表示标准库 `println`、完整 sysroot 或全部语言特性已经可用。当前实现边界和验证证据见 [host pipeline](../host-pipeline.md)，前端待补项见 [语义前端检查点](../frontend-checkpoint.md)。
