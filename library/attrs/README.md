# attrs

源码目录与待实现领域见 [六包结构](../STRUCTURE.md)。

`abi(name: Symbol) -> Object` 是普通的 `pure comptime fn`，返回含 `abi`
属性的 Object，并保留 Symbol 值及其类型。`comptime_phase` 是包含
`comptime_phase: true` 的 Object 常量，条目保留 Bool 类型。
`metadata(data: Object) -> Object` 将用户自定义元数据包装为 `{ metadata: data }`。
`stable_since(version: str) -> Object`（不稳定）将版本锚点包装为
`{ __flurry_stable_since: version }`，用于后续 ABI `type_id` 的规范版本固定。
`attrs.prelude` 导出 `abi`、`comptime_phase`、`metadata` 和 `stable_since`。

命名子模块另提供：
- `attrs.packed.byte` 与 `attrs.packed.bit`：分别产生 `{ __flurry_packed_mode: .byte }` 与 `{ __flurry_packed_mode: .bit }`，对应字节级紧凑布局（`align = 1`）与直接翻译为 `BitVec<W>` 并复用 `bitvec_opt` 的位级紧凑布局（`attrs.packed.bit` 总位宽超过 256 位时发出非致命编译警告）。
- `attrs.mem.volatile`（及 `attrs.volatile`）：产生 `{ __flurry_volatile: true }`，标示易失内存／MMIO 结构体、newtype 或结构体字段；LLVM 后端对对应访问发射 `load volatile` / `store volatile`，程序纯度分析归入 P0（禁止 `memory(none)`/`speculatable`/`nosync`/`readonly`）。不允许以裸 key 或 definition configuration 形式附着，且不能与 `packed` 混用。
- `attrs.layout.{packed, packed_layout}`：过渡期旧入口，产生 `{ packed: bool }`，供当前结构体字节级紧凑布局检查与降码消费（随 `__flurry_packed_mode` 消费者接通后统一收口）。
- `attrs.effects.direct_passing`：产生 `{ direct_passing: true }`，供直接传递效应声明检查消费。
- `attrs.derivation`：编译器已消费 `{ derivation_default: bool }`；`attrs.derivation.default` 导出仍待 `default` 成员声明语法决定（见 `derivation/README.md`）。

Object 通过 `meta.Object` 导入，attrs 不重复建立原始 builtin 绑定。
meta 的基础域绑定与集合操作分开；属性消费者仍通过普通包依赖取得同一域。

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
python3 -B development/cargo.py run -p luna -- \
  dump /tmp/attrs-example.fl --sysroot library \
  --build-target x86_64 linux 64 --emit merged-attributes
```

`--emit attributes` 显示求值后的 Object；`--emit merged-attributes` 还显示
按属性规则选中的原始条目位置，不重新构造或丢失条目的类型信息。
不合法的合并输入会报错；使用 `--output` 时，检查失败不会覆盖已有输出。

此入口只交付属性检查结果。ABI 消费者尚未接线，普通 NIR 编译仍拒绝尚未
应用的 ABI 属性；不能将检查成功解释为已经生成 C ABI 函数。这里加载完整
sysroot；格式化、分配和 I/O 接口仍有独立阻塞。

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
结构一致。`derivation_default`、`packed`、`direct_passing` 与 `metadata` 已接通对应编译器消费者；`abi` 降码、`__flurry_packed_mode`（`attrs.packed.{byte, bit}`）与 `stable_since`（`__flurry_stable_since`）消费者仍未接通，普通 NIR 编译会明确拒绝 `abi`、`__flurry_packed_mode`、`__flurry_stable_since` 及其他未消费属性。
