# attrs

源码目录与待实现领域见 [六包结构](../STRUCTURE.md)。

`abi(name: Symbol) -> Object` 是普通的 `pure comptime fn`，返回含 `__flurry_abi`
属性的 Object，并保留 Symbol 值及其类型。`comptime_phase` 是包含
`__flurry_comptime_phase: true` 的 Object 常量，条目保留 Bool 类型。
`metadata(data: Object) -> Object` 将用户自定义元数据包装为 `{ __flurry_metadata: data }`。
`stable_since(version: str) -> Object`（不稳定）将版本锚点包装为
`{ __flurry_stable_since: version }`，用于后续 ABI `type_id` 的规范版本固定。
`attrs.prelude` 导出 `abi`、`comptime_phase`、`metadata` 和 `stable_since`。

命名子模块另提供：
- `attrs.packed.byte` 与 `attrs.packed.bit`：分别产生 `{ __flurry_packed_mode: .byte }` 与 `{ __flurry_packed_mode: .bit }`，对应字节级紧凑布局（`align = 1`）与直接翻译为 `BitVec<W>` 并复用 `bitvec_opt` 的位级紧凑布局（`attrs.packed.bit` 总位宽超过 256 位时发出非致命编译警告）。
- `attrs.mem.volatile`（及 `attrs.volatile`）：产生 `{ __flurry_volatile: true }`，标示易失内存／MMIO 结构体、newtype 或结构体字段；LLVM 后端对对应访问发射 `load volatile` / `store volatile`，程序纯度分析归入 P0（禁止 `memory(none)`/`speculatable`/`nosync`/`readonly`）。不允许以裸 key 或 definition configuration 形式附着，且不能与 `packed` 混用。
- `attrs.macros.ast(variadic)`、`attrs.macros.peg(grammar)`、`attrs.macros.macro3` 与 `attrs.macros.macro4` 声明宏处理器的输入模式，展开器读取顶层的 `__flurry_macro_kind` 及对应参数。
- `attrs.effects.direct_passing`：产生 `{ __flurry_direct_passing: true }`，供直接传递效应声明检查消费。
- `attrs.purity.{p0, p1, p2, p3, p4, p5}`：产生 `{ __flurry_purity: .p0 }`..`{ __flurry_purity: .p5 }`，用于为运行时函数声明最低纯度层级（P0 一般行为、P1 有界状态变更、P2 观测只读、P3 稳定观测、P4 值纯、P5 总值纯／有限完成且无 trap），不允许以 definition configuration 形式附着，并由编译器在编译期核验。
- `attrs.quote_fn_template.{for_match_arms, allow_non_exhaustive_instance}`：分别产生 `{ __flurry_quote_fn_template_for_match_arms: true }` 与 `{ __flurry_quote_fn_template_allow_non_exhaustive_instance: true }`，标记用于 `match` `use` 规则接入的无体 `template quote fn` 声明及其单臂非穷尽许可。
- `attrs.derivation.default`：产生 `{ __flurry_derivation_default: true }`，标记可在无 `.use(...)` 时参与可见默认策略选择的 `derive fn`。

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

检查属性对象可使用上述两个入口。普通 NIR 编译会将 `attrs.abi(.c | .system | .stdcall | .win64 | .sysv64)` 应用于 runtime 函数，host 在支持的签名上生成对应调用约定入口。

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
结构一致。默认派生标记、紧凑布局、直接传递效应和用户元数据分别由相应的编译阶段消费。
`attrs.stable_since(version)` 把版本字符串附到声明。对范式实例求 `'attribute` 会读到其头声明上的版本。
