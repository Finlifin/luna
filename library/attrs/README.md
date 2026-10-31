# attrs

源码目录与待实现领域见 [六包结构](../STRUCTURE.md)。

`abi(name: Symbol) -> Object` 是普通的 `pure comptime fn`，返回含 `abi`
属性的 Object，并保留 Symbol 值及其类型。`comptime_phase` 是包含
`comptime_phase: true` 的 Object 常量，条目保留 Bool 类型。
`attrs.prelude` 导出两者。

需要逐成员指定阶段的混合 trait 可以复用 `^attrs.comptime_phase`，不重复写属性
Object。只包含 comptime 内容的协议使用 `pure comptime trait`；例如 meta 的
`ComptimeListPatternView`，其方法与 builtin 的 runtime 列表模式协议同名。


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

trait 成员可以使用普通属性值及别名：

```flurry
use attrs.comptime_phase as staged;

trait Code {
    ^staged
    comptime fn value() -> bool;
}
```

成员 phase 校验器检查最终选中条目的目标和 Bool 域，并在接口构造前应用。
属性的 trait phase 含义与函数的 `comptime` 关键字分别处理。例如：

```flurry
struct Subject {}

comptime impl Code for Subject {
    comptime fn value() -> bool { true }
}

comptime fn run() -> bool {
    Subject.as(Code).value()
}
```

单个 comptime 实现、泛型主体及该相可用的默认方法已有源码调用路径。
同一关系的普通与 comptime 源码实现支持方法和关联项的合并，包括泛型参数改名。
关联项按成员 phase 检查；共享值必须一致，泛型初始化程序目前要求替换参数后的
结构一致。依赖尚未完成接口的属性计算仍待接通。ABI 等其他属性的正常编译仍会明确拒绝。
