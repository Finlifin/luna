# attrs

`abi(name: Symbol) -> Object` 是普通的 `pure comptime fn`，返回含 `abi`
属性的 Object，并保留 Symbol 值及其类型。`attrs.prelude` 导出 `abi`。

```flurry
use attrs.abi;

^abi(.c)
fn sample() {}
```

目前可以检查属性对象及其合并结果。将示例保存为 `/tmp/attrs-example.fl`，
在仓库根目录运行：

```sh
python3 -B development/cargo.py run --release -p luna -- \
  --packages-v1 \
  --package 1 101 app /tmp/attrs-example.fl \
  --package 2 202 attrs library/attrs/lib.fl \
  --dependency 1 attrs 2 --primary 1 --emit merged-attributes
```

`--emit attributes` 显示求值后的 Object；`--emit merged-attributes` 还显示
按属性规则选中的原始条目位置，不重新构造或丢失条目的类型信息。
不合法的合并输入会报错；使用 `--output` 时，检查失败不会覆盖已有输出。

此入口只交付属性检查结果。ABI 消费者尚未接线，普通 NIR 编译仍拒绝尚未
应用的 ABI 属性；不能将检查成功解释为已经生成 C ABI 函数。这里显式加载
attrs 包，完整仓库 sysroot 的格式化、分配和 I/O 接口仍有独立阻塞。
