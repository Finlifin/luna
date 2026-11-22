# 03. 参数 Elaboration、接收者适配与派生策略（Parameter Elaboration & Derivation Wishlist）

> 本文档是 Flurry 语言标准库与 `luna` 编译器对齐的 **Wave 2 规格愿望单 #03**，系统定义参数收集（`VariadicCollector`、`ExtendedVariadicCollector` 与匿名记录 `Ps` 反射）、隐式参数供应（`ImplicitPassing` 与内置隐式源）、`Extension` 提案与孤儿规则调和、Tier 1 内置 `derive` 目录与 Tier 2 生态 `derive` 机制支撑、`post do` / `tactic lambda` / 三层接收者适配与统一调用准备计划，以及对应的 `language-items.conf` 绑定扩展。

---

## 1. 范围、权威来源与归属包

### 1.1 文档范围与跨文档边界

本文档完整规范参数角色解析、调用准备流水线与派生代码生成体系：

1. **可变参数与扩展属性收集协议**：`VariadicCollector`、`ExtendedVariadicCollector` 及各自编译时同名对偶 `ComptimeVariadicCollector`、`ComptimeExtendedVariadicCollector`；两阶段构造（`assoc Tmp: Type` 栈保活与可达性锚定）；`Tuple`、`Slice<T>`、`Array<T, N>`、`std.collections.Vec<T, ...>`（含运行期依赖分配器透传）、`meta.List<T>`、`meta.Object` 标准实现；Silver Layer-2 DSL 用户库示例（`ComponentChildrenList` / `ComponentPropertySet`）；以及匿名记录 `Ps` 编译期字段反射、`inline if` occurrence narrowing 与带 `SourceSpan` 的属性拼写诊断（Silver `P0-1`、`P0-2`）。
2. **隐式参数供应协议与内置隐式源**：`ImplicitPassing where param_name: Symbol`、`Context`、`CallDescription`、六大内置隐式源（`__src__` 绑定 `meta.ast.SourceSpan`、`__caller__`、`__module__`、`__package__`、`__file__`、`__line__`）、`.name = expr` 显式旁路、`...spread` 包与前序可选默认参数交互，以及 `quote fn` 中的 `implicit quote`。
3. **`Extension` 与 `derive` 孤儿规则调和**：将近期草案中的 `Extension` 派生设想与正式一致性/派生规范对账，确立“策略不得绕过包归属检查，外部扩展统一采用本地名义 `newtype` + `.as(View)` 视角”的刚性契约。
4. **标准 `derive` 策略目录与引擎机制（严守三层包架构）**：
   - **Tier 1 Builtin 派生器**（位于 `builtin` 与 `meta`）：覆盖 `Eq`、`Ord`、`Hash`、`Default`、`Clone`、`Copy`、`Debug`、`Display`、`Splice`、`meta.ast.ToTokens`，以及 `comptime derive` 对偶（`ComptimeEq`、`ComptimeOrd`、`ComptimeHash`、`ComptimeClone`、`ComptimeDefault`、`ComptimeDebug`、`ComptimeDisplay`）；
   - **Tier 2/3 生态派生机制示范**：严格将 `Serialize`/`Deserialize` 保留在六个内置包之外，展示 `derive Serialize<.json>.use(json_lib.without_allocation) for Data` 如何在参数化 `derive fn`、`requires` 子句、策略约束和事务验证支撑下实现零隐式分配互操作；
   - **属性与引擎缺口**：`^attrs.derivation.default`（`__flurry_derivation_default`）导出与 `default` 关键字冲突解决、带参/带子句策略实例化阻塞解除、默认策略本地注册检查、去重与事务回滚缓存。
5. **`post do`、`tactic lambda`、接收者适配与调用准备计划**：`post do` 目标路由优先级、`tactic lambda`（`_0`、`_1`）作用域/嵌套/`Once` 校验、三层接收者适配（显式 `.as(View)` 逆序 -> 内建 `*` 解引用 + 唯一 `Deref` 链 -> 结构精度 + 末端 `.ref`/`Reference`）与目标引导内建指针 `Copy` 读取、以及全调用源序准备计划（含失败部分清理与 P3 稳定默认计算）。

**跨文档职责边界**：

- `Index` / `IndexMut` / `SpaceApply` / `Deref` / `DerefMut` / `Reference` / 算术与比较协议自身完整签名见 [01-syntax-protocols-and-operators.md](file:///home/fin/luna/tmp/wishlist/01-syntax-protocols-and-operators.md)；本文档规范 `VariadicCollector` 与调用方准备计划如何协同 `SpaceApply` 及接收者适配引擎运行。
- 模式视图协议及 Take View 注册表见 [02-pattern-matching-and-views.md](file:///home/fin/luna/tmp/wishlist/02-pattern-matching-and-views.md)。
- `Once` / `Clone` / `Copy` / `Drop` / `Debug` / `Display` trait 本体及 `Formatting` 协议见 [04-builtin-types-math-iterators-fmt.md](file:///home/fin/luna/tmp/wishlist/04-builtin-types-math-iterators-fmt.md)；本文档规范它们的 Tier 1 默认派生策略。
- `RecordInfo`、`FieldInfo`、`StructInfo`、`EnumInfo`、`SourceSpan`、`ToTokens`、匿名记录 `Ps` 底层 PTC/反射 API 见 [05-meta-cmtt-macros-reflection.md](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md)。
- `__flurry_derivation_default` 与全部 `__flurry_*` 官方属性键集中规范见 [06-attrs-build-pipeline-projection.md](file:///home/fin/luna/tmp/wishlist/06-attrs-build-pipeline-projection.md)（待 Wave 2 同批落地）。
- Tier 2 `serialization`（`Serialize`/`Deserialize`）完整接口与三层边界规则见 [10-official-protocol-packages-and-boundary.md](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md)。

### 1.2 权威依据等级

严格遵循 `AGENTS.md`：

1. **最高权威**：作者确认结论与 `AGENTS.md`（§5 语义边界、六个内置包规则、无 `Option`/`Result`、comptime 无指针值引用语义、运行期依赖 allocator 范式）。
2. **正式规范文档（`docs/`）**：
   - 函数、参数与调用：[docs/functions/parameters.md](../../docs/functions/parameters.md)、[docs/functions/parameter-elaboration.md](../../docs/functions/parameter-elaboration.md)、[docs/functions/comptime_fn.md](../../docs/functions/comptime_fn.md)、[docs/functions/calls-and-returns.md](../../docs/functions/calls-and-returns.md)
   - 派生、归属与接收者适配：[docs/traits/derivation.md](../../docs/traits/derivation.md)、[docs/traits/coherence.md](../../docs/traits/coherence.md)、[docs/traits/declarations.md](../../docs/traits/declarations.md)、[docs/traits/implementation-resolution.md](../../docs/traits/implementation-resolution.md)、[docs/type/receiver-adaptation.md](../../docs/type/receiver-adaptation.md)、[docs/type/member-projection.md](../../docs/type/member-projection.md)
   - 属性、阶段、包组织与依赖类型：[docs/comptime/official-attribute-keys.md](../../docs/comptime/official-attribute-keys.md)、[docs/comptime/the-comptime-phases-of-traits.md](../../docs/comptime/the-comptime-phases-of-traits.md)、[docs/code-organization/builtin-packages.md](../../docs/code-organization/builtin-packages.md)、[docs/type/reflection-and-layout.md](../../docs/type/reflection-and-layout.md)、[docs/type/propositional-constraints/occurrence-typing.md](../../docs/type/propositional-constraints/occurrence-typing.md)、[docs/type/runtime-dependent-type.md](../../docs/type/runtime-dependent-type.md)
3. **跨愿望单对齐参考**：[01-syntax-protocols-and-operators.md](file:///home/fin/luna/tmp/wishlist/01-syntax-protocols-and-operators.md)、[04-builtin-types-math-iterators-fmt.md](file:///home/fin/luna/tmp/wishlist/04-builtin-types-math-iterators-fmt.md)、[05-meta-cmtt-macros-reflection.md](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md)、[10-official-protocol-packages-and-boundary.md](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md)、[tmp/silver-design/04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md)。
4. **非权威现状/近期草案（仅用于盘点缺口与设计意图，绝不引用 `2024-*` / `2025-*` / `2026-1..4-*` 或 `flurry_tutorial/`）**：
   - 仅限 `2026-5` 至 `2026-10` 近期草案：`proposal_trace/2026-9-8-variadic-argument-collecting-rules.fl`、`proposal_trace/2026-9-12-implitcit-substitution.fl`、`proposal_trace/2026-8-13-derivation.fl`、`proposal_trace/2026-9-13-receiver-adaption.fl`、`proposal_trace/2026-8-14-dispersed-definition-collection.fl`；
   - 现状参照：`library/`、`compiler/elaboration/src/program/{derivation.rs, tactics.rs, open_functions.rs, input_routing.rs, spreads.rs, extended.rs}`。

### 1.3 内置包与三层归属矩阵

| 协议 / 策略 / 组件 | 层级归属 | Runtime / Elaboration 模块 | Comptime 对偶模块 | Prelude 与绑定规则 |
|---|---|---|---|---|
| `VariadicCollector`, `ExtendedVariadicCollector` | Tier 1 | `builtin.protocols.ops`（`builtin.prelude`） | `meta.ops.ComptimeVariadicCollector`, `ComptimeExtendedVariadicCollector` | `language-items.conf` 绑定；comptime 版仅用 `self` 值引用 |
| `ImplicitPassing`, `Context`, `CallDescription` | Tier 1 | `builtin.protocols.implicit`（`builtin.prelude`） | 供应函数本身在编译时执行（CMTT 返回 `< \|- ArgType >`） | `language-items.conf` 绑定；无 `.symbol'builtin` 越权访问 |
| 内置隐式供应实现（`__caller__`, `__module__`, `__package__`, `__file__`, `__line__`, `__src__`） | Tier 1 | `builtin.protocols.implicit`（原语源）与 `meta.ast`（`SourceSpan` 用于 `__src__`） | 编译期调用点直接生成 | 因 `meta` 依赖 `builtin`，`__src__` 实现位于 `meta.ast`（满足 `SourceSpan` 本地身份） |
| `^attrs.derivation.default` | Tier 1 | `attrs.derivation`（展开为 `{ __flurry_derivation_default: bool }`） | 编译期派生选择消费 | 当前因 `default` 为关键字暂阻塞导出 |
| `Eq`, `Ord`, `Hash`, `Default`, `Clone`, `Copy`, `Debug`, `Display`, `Splice`, `ToTokens` 派生策略 | Tier 1 | 各自 trait 所在 `builtin.*` 模块 | `meta.*` 下 `Comptime*` 同名对偶派生策略（`comptime derive`） | Tier 1 内建策略允许标量、原语与内置容器使用默认/具名策略 |
| 通用字段投影与派生辅助原语（`builtin.field_ptr`, `builtin.tuple_into_array`, `meta.reflection.{field_value, enum_fields_eq, enum_fields_hash, construct_struct_from_cloned_fields}`） | Tier 1 | `builtin.mem` / `builtin.bindings`（受 PTC `has_field` 守卫） | `meta.reflection`（无指针值引用读取） | 集中绑定并服务通用 `derive fn` 与 `Array` 变参构造，避免迫使每个派生使用 `unsafe` 偏移算术 |
| `ComponentChildrenList` / `ComponentPropertySet` | **Tier 2/3（用户框架示例）** | Silver Layer-2 UI 包（**非六个内置包成员**） | 可选配套编译时校验 | 仅演示 `VariadicCollector` + `ExtendedVariadicCollector` |
| `Serialize` / `Deserialize` 与 `json_lib.without_allocation` | **Tier 2 / Tier 3** | `serialization`（Tier 2）与 `json_lib`（Tier 3）（**严禁放入六内置包**） | `serialization.ComptimeSerialize` | 本文档仅展示其对 `derive` 机制的参数化策略与 allocator 透传 |

---

## 2. 现状与缺口全景总表

| ID | 符号 / 特性 | 归属包路径 | 阶段 | 现状 (Current Status) | 优先级 | 编译器依赖 |
|---|---|---|---|---|---|---|
| `WL-03-01` | `VariadicCollector` | `builtin.protocols.ops` | Runtime | 仅存在 `proposal_trace/2026-9-8` 旧草案；`spreads.rs` 仅支持内置 `Tuple` 收集（**Silver `P0-1`**） | `P0` | elaboration 路由检测 `VariadicCollector`、栈上保活 `assoc Tmp` 可达性依赖与反向清理 |
| `WL-03-02` | `ComptimeVariadicCollector` | `meta.ops` | Comptime (`pure comptime trait`) | 完全缺失；旧草案使用非法的 `^comptime_phase` + `_comptime` 后缀 | `P1` | `pure comptime trait` 查找层、同名 `variadic_collect` 与 `self` 值引用 |
| `WL-03-03` | `ExtendedVariadicCollector` | `builtin.protocols.ops` | Runtime | 完全缺失；`extended.rs` 仅收集默认匿名结构体 `Ps`（**Silver `P0-1`**） | `P0` | elaboration 将 `{ k: v, ... }` 匿名记录 `Ps` 路由至 `extended_variadic_collect` 并保活 `Tmp` |
| `WL-03-04` | `ComptimeExtendedVariadicCollector` | `meta.ops` | Comptime (`pure comptime trait`) | 完全缺失 | `P1` | comptime `ExtendedApplication` 调用准备与 `pure comptime trait` 分派 |
| `WL-03-05` | 标准集合 collector 实现（`Slice`, `Array`, `Vec`, `meta.List`, `meta.Object`） | `builtin`, `std.collections`, `meta` | Runtime & Comptime | 完全缺失；其中 `Vec` 必须透传 `quote alloc: lift *A`；`Slice` 存在 `Tmp` 与 `S` 长度关联问题 | `P0` | `VariadicCollector` 落地、`std` allocator 透传、`Array` 编译期定长校验 |
| `WL-03-06` | 匿名记录 `Ps` 字段反射、occurrence narrowing 与属性拼写报错 | `meta.reflection`, `PTC` | Comptime check -> Runtime | `Term::Record` 在 `nir/types.rs` 缺反射；依赖 `05` `WL-05-13`（**Silver `P0-2`**） | `P0` | `reflect_record(Ps)`、`has_field`、`FieldType`、PTC 局部收窄及带 `SourceSpan` 的 `meta.compilation.err` |
| `WL-03-07` | 默认位置包/属性包部分元素推断、同级上下文与 `Never` 包计划 | `luna` elaboration | Runtime & Comptime | `spreads.rs`/`extended.rs` 已实现主体；需与定制 collector 共享 `Unknown` 阻断规则与源码顺序计划 | `P0` | `arguments` / `input_routing` 统一协议分派，严禁 `Unknown` 时静默降级为默认元组 |
| `WL-03-08` | `ImplicitPassing`, `Context`, `CallDescription` | `builtin.protocols.implicit` | Comptime elaboration | 完全缺失；`open_functions.rs:1981` & `cursor.rs:214` 拒绝所有未显式提供的 `implicit` 形参 | `P1` | `implicit-passing-protocol` 绑定；elaboration 查 `ImplicitPassing<.param> for ArgType` 并拼接 `< \|- ArgType >` |
| `WL-03-09` | 内置隐式供应源族（`__src__`, `__caller__`, `__module__`, `__package__`, `__file__`, `__line__`） | `builtin.protocols.implicit`, `meta.ast` | Comptime elaboration -> RT/CT | 完全缺失；旧草案 `2026-9-12` 引用了过时路径 `meta.compilation.src.Span` | `P1` | `CallDescription` 暴露调用点 `SourceSpan`、所在函数/模块/包路径与行号内建构建器 |
| `WL-03-10` | `implicit` 与 `.name = expr`、`...spread`、可选默认值及 `implicit quote` 交互 | `luna` elaboration | Runtime & Comptime | 显式 `.name = expr` 已接入直调与 `...spread`；开放签名中的协议供应待与 `P1` 一起打通 | `P1` | `parameter-elaboration.md:71, 121` 开放调用 `implicit` 供应计划接线 |
| `WL-03-11` | `Extension` 提案与 `docs/traits/coherence.md` 孤儿规则调和 | 语言规范 & Coherence | Static check | `2026-9-12` 草案提出 `Extension<.id>`；`docs/traits/coherence.md` & `derivation.md` 与 `program.rs:2245` 明确坚持本地身份原则 | `P1` | 维持并复用既有 `coherence` / `resolve_derivation_defaults` 检查；外部扩展走 `newtype` + `.as(View)` |
| `WL-03-12` | `derive fn` 参数、子句、策略条件与依赖参数实例化 | `luna` derivation | Comptime | **严重缺口**：`derivation.rs:39-44, 152-158` 拒绝带 `clause_params`/策略输入/子句约束的 `derive` | `P0` | 补齐策略形参默认值/implicit/lambda/variadic/子句契约 instantiation 与依赖求值 |
| `WL-03-13` | `attrs.derivation.default` 与 `default` 关键字位置决议 | `attrs.derivation`, `parse` | Comptime attr | `library/attrs/derivation/README.md:3-6` 记录 `default` 关键字阻止声明导出；编译器已消费 `__flurry_derivation_default` | `P0` | parser 允许成员路径/上下文关键字位置 `default`，或在 `attrs` 集中导出并同步 `docs/basics/lexical.md` |
| `WL-03-14` | 默认策略本地注册、同精度冲突检测与事务验证缓存 | `luna` derivation | Static check | `program.rs:2163-2363` 已建注册骨架；需补完整策略缓存键与产物失败零残留回滚 | `P1` | 缓存键纳人主体、实参、观察结构、接口实参、phase 与实现世界；事务发布失败不留半成品 |
| `WL-03-15` | Tier 1 派生策略：`Eq`, `Ord`, `Hash`, `Default` 及其 `Comptime*` 对偶 | `builtin`, `meta` | Runtime & Comptime | 完全缺失（无一内置策略实现）；依赖 `01` 中 `*self` 签名修复与类型安全字段投影原语 | `P0` | `reflect_struct`/`reflect_enum`、安全字段投影 `field_ptr`/`field_value`、`derive fn` 参数与泛型实例化 |
| `WL-03-16` | Tier 1 派生策略：`Clone`, `Copy`, `Debug`, `Display` 及其 `Comptime*` 对偶 | `builtin`, `meta` | Runtime & Comptime | 完全缺失；需处理运行期依赖 `quote .allocator` 透传及 `Copy` 无自定义 `Drop` 互斥校验 | `P1` | `needs_drop`/`is_copy` 查询、`Formatter`/`DebugStruct`/`DebugTuple`（`04`）与 `derive` 依赖传递 |
| `WL-03-17` | Tier 1 派生策略：`Splice` 与 `meta.ast.ToTokens` | `builtin`, `meta` | Comptime -> RT / CT | 完全缺失 | `P1` | 两宇宙 `Splice` 准入校验、`TokenBuffer`（`05` `WL-05-26`）准引用/拼装生成 |
| `WL-03-18` | Tier 2 生态 `derive` 支撑（如 `Serialize<.json>.use(json_lib.without_allocation)`） | 语言派生引擎（跨引用 `10`） | Comptime -> Runtime | 依赖 `WL-03-12` 解除泛型宿主上的策略约束阻塞及 `.as(View)` 本地 `newtype` 适配器 | `P1` | 泛型容器（带 `A: Allocator` + `quote .allocator`）上成功运行外部 `derive fn` |
| `WL-03-19` | `post do` 尾随回调路由与多入口/包混合 | `luna` elaboration | Runtime & Comptime | `input_routing.rs` / `tactics.rs` 已接通直调及部分包；需对齐与定制 `VariadicCollector` 共存时的锚点优先规则 | `P0` | 锚定优先路由先于变参边界划分；唯一目标未决即挂起/拒绝 |
| `WL-03-20` | `tactic lambda`（`_0`, `_1`, ...）边界、仿射资源复用拒绝与依赖签名检查 | `luna` elaboration | Runtime & Comptime | `tactics.rs` 已实现基础检查；需覆盖开放 `defer` 返回类型、嵌套 `lambda` 与 `derive fn` 内回调 | `P0` | 保持非 `Copy` 同一占位符多次消费拒收、作用域遮蔽与最大下标越界检查 |
| `WL-03-21` | 三层接收者适配引擎（`.as(View)` 逆序 -> `*`/`Deref` 链 -> 精度 + `.ref`/`Reference`） | `luna` elaboration | Runtime | 现仅支持内建单步指针/类型视图；缺 `Deref`/`Reference` 协议链（依赖 `01` `WL-01-22`） | `P1` | 有限深度与类型头环路检测、`Unknown` 阻塞回退、只执行选中路径动作一次 |
| `WL-03-22` | 目标引导内建指针 `Copy` 读取边界 | `luna` elaboration | Runtime | 部分支持（初始化/返回/实参）；需严格禁止非 `Copy` 自动搬出及未初始化无效内存读取 | `P1` | `ReceiverAdaptation` 与资源/有效性检查协同验证 |
| `WL-03-23` | 统一调用准备计划（源序一次性求值、依赖默认 P3 校验、失败部分清理） | `luna` elaboration / MIR | Runtime & Comptime | `calls-and-returns.md` 规范完备；开放签名与复合协议接入路径尚存分散逻辑 | `P0` | 统一 `CallPreparationPlan`：源序评估 -> collector -> implicit -> P3 默认值 -> 异常回滚 |
| `WL-03-24` | `library/language-items.conf` & `SysrootBindings` 参数/隐式/字段原语键注册 | `library/`, `luna` | Compiler & Sysroot | 现无 `variadic-collector-protocol`、`extended-variadic-collector-protocol`、`implicit-passing-protocol` 等键 | `P0` | 扩展 `SysrootBindings`、`Protocols`、CLI `--*-protocol` 选项与 `language-items.conf` |

---

## 3. 详细规格设计

> 本章全部 Flurry 代码严格符合语言规范：`where` 子句承载全部类型参数（不在 `fn`/`struct`/`trait`/`impl`/`derive fn` 名称后写 `<T>`）、trait 约束使用 `:-`、`assoc` 声明（含 `impl` 体内）强制写出 `: Type`、无体方法声明以 `;` 结尾、元组类型写作 `Tuple<A, B>`、可选 trait 实参以具名形式传入、无 `Option`/`Result` 枚举、无 `::` 路径、无 `&&`/`||`/`!`、无 `//`/`/* */` 注释、多行 `where`/`requires` 左花括号 `{` 另起一行对齐、中缀操作符两侧留白、省略 `}` 后分号、comptime 方法全部采用无指针 `self` 值引用语义。

### 3.1 `VariadicCollector`、`ExtendedVariadicCollector` 与匿名记录 `Ps` 反射

#### 3.1.1 完整协议签名（Runtime 与 Comptime 同名对偶）

依据 `Silver P0-1` 与 [docs/comptime/the-comptime-phases-of-traits.md](../../docs/comptime/the-comptime-phases-of-traits.md)，将 `proposal_trace/2026-9-8-variadic-argument-collecting-rules.fl` 现代化为**独立 Runtime trait + Comptime `pure comptime trait` 双对偶**，删除过时的 `^comptime_phase`、`TmpComptime` 与 `_comptime` 方法名后缀，并保持与 [01-syntax-protocols-and-operators.md](file:///home/fin/luna/tmp/wishlist/01-syntax-protocols-and-operators.md) 中 `SpaceApply` 签名完全一致：

```flurry
-- 归属：library/builtin/protocols/ops.fl（builtin.protocols.ops，由 builtin.prelude 导出）

pub trait VariadicCollector {
    assoc Tmp: Type;
    fn variadic_collect(comptime S: Type, args: S) -> Tuple<Tmp, Self>;
}

pub trait ExtendedVariadicCollector {
    assoc Tmp: Type;
    fn extended_variadic_collect(comptime Ps: Type, properties: Ps) -> Tuple<Tmp, Self>;
}
```

```flurry
-- 归属：library/meta/ops.fl（meta.ops，由 meta.prelude 导出）
-- 注意：使用 pure comptime trait，方法名与 Runtime 版完全相同（variadic_collect / extended_variadic_collect），
-- 参数均为编译时值（无 *self / .ref / .* 指针）。

pub pure comptime trait ComptimeVariadicCollector {
    assoc Tmp: Type;
    pure comptime fn variadic_collect(S: Type, args: S) -> Tuple<Tmp, Self>;
}

pub pure comptime trait ComptimeExtendedVariadicCollector {
    assoc Tmp: Type;
    pure comptime fn extended_variadic_collect(Ps: Type, properties: Ps) -> Tuple<Tmp, Self>;
}
```

#### 3.1.2 收集语义、两阶段 `assoc Tmp` 保活与分层协议边界

1. **两阶段调用准备计划**：
   - 当调用形参声明 `...items: TargetTy`（位置变参）或扩展应用 `{ prop: val, ... }` 指向 `...props: TargetTy`（扩展变参）时：
     - 若调用处以 `.items = whole_val` 或 `.props = whole_val` 直接供应完整参数值，**跳过** `VariadicCollector` / `ExtendedVariadicCollector` 调用，直接将该值交付给形参（[docs/functions/parameters.md](../../docs/functions/parameters.md#L41-L45)）。
     - 否则，检查目标类型 `TargetTy` 是否实现 `VariadicCollector` / `ExtendedVariadicCollector`（在编译时调用上下文优先查找 `Comptime*` 对偶层）：
       - **未决阻塞铁律**：若 `TargetTy` 实现查询为 `Unknown` 或发生同精度候选冲突，立即挂起或报错，**严禁静默降级为默认元组或匿名记录**；
       - 若明确证明 `TargetTy` 未实现对应 collector 且目标类型与实参包形态匹配内置规则（即位置参数构造 `Tuple<...>` 或扩展属性构造匿名记录 `Ps`），则按默认收集路径处理。
   - **源码求值顺序与资源责任**：收集区实参（含 `...spread` 元组与交错的 `.named = expr`）严格按源码先后顺序各求值一次。若任一实参生产者或包体提前退出（`return`、`break`、`error` 传播 `!`），立即逆序清理已创建的临时值并中止准备计划。
   - **`assoc Tmp: Type` 栈上保活与可达依赖**：编译器生成等效绑定 `let collected = TargetTy.as(VariadicCollector).variadic_collect(S, raw_args);`，将 `collected.0`（类型 `Tmp`）存放于调用方调用准备帧内，其生存期**严格覆盖整个被调函数执行期与返回值接收**，然后将 `collected.1`（类型 `TargetTy`，可包含指向 `collected.0` 的 `Slice` / `&r` 可达视图）移交给被调函数。被调函数返回（或后续实参准备失败）时，先清理 `collected.1`（若拥有关联清理），再清理 `collected.0`（`Tmp`）。
2. **`Slice<T>` 的栈缓冲边界分析**：
   - 若 `Slice<T>` 直接实现非参数化的 `assoc Tmp: Type`，由于 `Tmp` 不能直接依赖方法参数 `S: Type`，栈缓冲容量只能取固定小数组/混合结构（例如 `SmallVec<T, 8>`，存在两处隐患：① `builtin` 中 `Slice<T>` 不能依赖 `std.collections.SmallVec` 且无法在无堆分配器下支持超过 8 项的 `Slice<T>` 变参；② 引入堆分配器则会违反无隐式分配原则）。
   - 若将 `VariadicCollector` 中的 `assoc Tmp<S: Type>: Type` 参数化（利用 Flurry 原生支持参数化关联项语法 `docs/traits/declarations.md:13`），`Slice<T>` 可精确计算 `Array<T, meta.reflection.tuple_len_usize(S)>` 作为全栈精确大小 `Tmp`，**零堆分配且不限参数数量**！然而 [01-syntax-protocols-and-operators.md](file:///home/fin/luna/tmp/wishlist/01-syntax-protocols-and-operators.md) 中 `SpaceApply` 依赖非参数化 `assoc Tmp: Type`；因此本文档保持兼容签名并把“是否将 `VariadicCollector.Tmp` 升级为 `assoc Tmp<S: Type>: Type`”专列为 `OQ-03-02` 交由作者统一裁定。

#### 3.1.3 Tier 1 标准实现（`Array`, `Vec`, `meta.List`, `meta.Object` 与 `Slice` 示例）

```flurry
-- 1. Array<T, N>：直接栈上收集，无需 Tmp 临时区，长度不符在编译期报错
impl VariadicCollector for Array<T, N>
where
    T: Type,
    N: usize,
{
    assoc Tmp: Type = void;

    fn variadic_collect(comptime S: Type, args: S) -> Tuple<void, Array<T, N>> {
        inline if not Type.predicates.is_tuple(S) {
            meta.compilation.err("Array VariadicCollector expects a Tuple packet");
        }
        const ARITY: usize = Type.predicates.tuple_len(S) match {
            n? => n,
            null => meta.compilation.err("Array VariadicCollector requires known tuple arity"),
        }
        inline if ARITY != N {
            meta.compilation.err("Array length mismatch in VariadicCollector");
        }
        -- 编译期按固定数量 ARITY 依次解构 S 并构造定长数组（移动所有权，不要求 T :- Copy）
        let raw_array = builtin.tuple_into_array<T, N>(args);
        ((), raw_array)
    }
}

-- 2. std.collections.Vec<T, .A = A, .allocator = alloc>：
--    严格遵守 Flurry 运行期依赖 allocator 范式，绝不硬编码 SystemAllocator！
--    注：Vec.with_capacity / push 返回 !AllocErr；因 VariadicCollector.variadic_collect 为不可失败签名，
--    此处显式处理分配失败（见 §5 OQ-03-07 关于是否增设可失败 TryVariadicCollector）。
impl VariadicCollector for Vec<T, .A = A, .allocator = alloc>
where
    T: Type,
    A: Type,
    requires A :- std.mem.Allocator,
    quote alloc: lift *A,
{
    assoc Tmp: Type = void;

    fn variadic_collect(comptime S: Type, args: S) -> Tuple<void, Vec<T, .A = A, .allocator = alloc>> {
        inline if not Type.predicates.is_tuple(S) {
            meta.compilation.err("Vec VariadicCollector expects a Tuple packet");
        }
        const ARITY: usize = Type.predicates.tuple_len(S) match {
            n? => n,
            null => meta.compilation.err("Vec VariadicCollector requires known tuple arity"),
        }
        var out = Vec<T, .A = A, .allocator = alloc>.with_capacity(ARITY) match {
            v! => v,
            error _ => std.process.panic("out of memory in Vec.variadic_collect"),
        }
        inline for elem in args'tuple_enumerate {
            inline if Type.predicates.eq(elem'type, T) {
                out.push(elem) match {
                    _! => (),
                    error _ => std.process.panic("out of memory in Vec.variadic_collect"),
                }
            } else {
                meta.compilation.err("Vec element type mismatch in variadic argument");
            }
        }
        ((), out)
    }
}

-- 3. meta.List<T>（Comptime 对偶，使用 self 值引用与无指针值语义）
impl ComptimeVariadicCollector for meta.List<T>
where
    T: Type,
{
    assoc Tmp: Type = void;

    pure comptime fn variadic_collect(S: Type, args: S) -> Tuple<void, meta.List<T>> {
        inline if not Type.predicates.is_tuple(S) {
            meta.compilation.err("meta.List ComptimeVariadicCollector expects a Tuple packet");
        }
        var out: meta.List<T> = [];
        inline for elem in args'tuple_enumerate {
            inline if Type.predicates.eq(elem'type, T) {
                out = out.concat([elem]);
            } else {
                meta.compilation.err("meta.List element type mismatch in comptime variadic argument");
            }
        }
        ((), out)
    }
}

-- 4. meta.Object（Comptime 扩展属性收集对偶：将匿名记录 Ps 转成持久化 meta.Object）
impl ComptimeExtendedVariadicCollector for meta.Object {
    assoc Tmp: Type = void;

    pure comptime fn extended_variadic_collect(Ps: Type, properties: Ps) -> Tuple<void, meta.Object> {
        let info = meta.reflection.reflect_record(Ps) match {
            ok! => ok,
            error _ => meta.compilation.err("meta.Object collector requires an anonymous record Ps"),
        }
        var obj: meta.Object = {}
        inline for field in info.fields {
            inline if field.name is key? do {
                let val = meta.reflection.field_value(properties, key);
                obj = obj.append(val, .key = key);
            }
        }
        ((), obj)
    }
}
```

> 说明：上例 `args'tuple_enumerate` 遵从 [02-pattern-matching-and-views.md](file:///home/fin/luna/tmp/wishlist/02-pattern-matching-and-views.md#L1878) 规定，按元素迭代（索引与元素成对迭代使用 `args'tuple_indexed`）。关于 `Vec.variadic_collect` 是否应允许通过可失败协议向上传播 `!AllocErr`，见 §5 `OQ-03-07`。

#### 3.1.4 匿名记录 `Ps` 反射与 Silver Layer-2 DSL 完整示范（非六个内置包成员）

结合 [05-meta-cmtt-macros-reflection.md](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md) 的 `WL-05-13`（`reflect_record(Ps)`、`RecordInfo`、`FieldInfo`、`has_field`、`FieldType`）与 `inline if` occurrence typing，下面演示**Layer-2 用户库（Silver UI DSL 示例，严禁放入 `builtin`/`std`）**如何在零宏下静态验证全部属性名与类型并在编译期对拼写错误报出带 `SourceSpan` 的错误：

```flurry
-- 声明：以下 ComponentPropertySet 与 ComponentChildrenList 属于 Layer-2 用户 DSL 包示例，不属于六个内置包。
use builtin.{ExtendedVariadicCollector, Symbol, Tuple, Type, VariadicCollector, bool, u32, void}

pub struct ComponentPropertySet {
    pub width: ?u32,
    pub padding: u32,
    pub gap: u32,
}

pub struct UiNode {
    pub tag: u32,
}

pub struct ComponentChildrenList {
    pub count: u32,
}

impl VariadicCollector for ComponentChildrenList {
    assoc Tmp: Type = void;

    fn variadic_collect(comptime S: Type, args: S) -> Tuple<void, ComponentChildrenList> {
        inline if not Type.predicates.is_tuple(S) {
            meta.compilation.err("ComponentChildrenList expects a Tuple of child UI nodes");
        }
        var total: u32 = 0u32;
        inline for child in args'tuple_enumerate {
            inline if Type.predicates.eq(child'type, UiNode) {
                total = total + 1u32;
            } else {
                meta.compilation.err("child component must have type UiNode");
            }
        }
        ((), ComponentChildrenList { count: total })
    }
}

pure comptime fn is_allowed_ui_prop(name: Symbol) -> bool {
    name == .width or name == .padding or name == .gap
}

impl ExtendedVariadicCollector for ComponentPropertySet {
    assoc Tmp: Type = void;

    fn extended_variadic_collect(comptime Ps: Type, properties: Ps) -> Tuple<void, ComponentPropertySet> {
        const info = meta.reflection.reflect_record(Ps) match {
            ok! => ok,
            error _ => meta.compilation.err("ComponentPropertySet requires an anonymous property record"),
        }
        -- 编译期遍历所有传入字段：对未知/拼写错误字段（如 pading）给出精确 Span 报错
        inline for field in info.fields {
            inline if field.name is key? do {
                inline if not is_allowed_ui_prop(key) {
                    meta.compilation.err("unknown component property: " ++ key'tag_name, .span = field.span);
                }
            }
        }
        -- 利用 PTC has_field + FieldType + inline if occurrence typing 静态读取存在字段（零运行期反射分支）
        let resolved_width: ?u32 = inline if Type.predicates.has_field(Ps, .width) {
            inline if Type.predicates.FieldType(Ps, .width) is wt? do {
                inline if Type.predicates.eq(wt, u32) {
                    properties.width
                } else {
                    meta.compilation.err("property 'width' expects u32")
                }
            } else {
                null
            }
        } else {
            null
        }
        let resolved_padding: u32 = inline if Type.predicates.has_field(Ps, .padding) {
            inline if Type.predicates.FieldType(Ps, .padding) is pt? do {
                inline if Type.predicates.eq(pt, u32) {
                    properties.padding
                } else {
                    meta.compilation.err("property 'padding' expects u32")
                }
            } else {
                0u32
            }
        } else {
            0u32
        }
        let resolved_gap: u32 = inline if Type.predicates.has_field(Ps, .gap) {
            inline if Type.predicates.FieldType(Ps, .gap) is gt? do {
                inline if Type.predicates.eq(gt, u32) {
                    properties.gap
                } else {
                    meta.compilation.err("property 'gap' expects u32")
                }
            } else {
                0u32
            }
        } else {
            0u32
        }
        ((), ComponentPropertySet {
            width: resolved_width,
            padding: resolved_padding,
            gap: resolved_gap,
        })
    }
}
```

**反例（必须拒绝）**：
- 用户在 DSL 写 `vbox { pading: 16u32 }` -> 编译期在 `is_allowed_ui_prop(.pading)` 分支命中 `meta.compilation.err`，指出 `field.span` 拼写错误，绝不在运行期静默忽略。
- `volume(2, 3, .shape = (4, 5))` -> 拒绝：同一变参既收到位置实参，又收到 `.shape = ...` 整包供应（[docs/functions/parameters.md](../../docs/functions/parameters.md#L441-L444)）。
- 对需要 `assoc Tmp` 保活的容器试图在 `variadic_collect` 返回指向自身局部变量再被移动的悬垂指针 -> 由内存/可达性检查拒绝。

---

### 3.2 `ImplicitPassing` 与内置隐式供应源族

#### 3.2.1 `ImplicitPassing`、`Context` 与 `CallDescription`（`builtin.protocols.implicit`）

将 `proposal_trace/2026-9-12-implitcit-substitution.fl` 严格校准至正式规范（修复过时 `meta.compilation.src.Span` 为 `meta.ast.SourceSpan`，并用 `impl` / `derive` 双相边界清晰界定）：

```flurry
-- 归属：library/builtin/protocols/implicit.fl（builtin.protocols.implicit，由 builtin.prelude 导出）

-- 仅在编译时参数 elaboration 期可读的上下文元描述（不依赖高层 meta，避免包依赖回环）
pub struct Context {
    pub pure comptime fn package_name(self) -> str;
    pub pure comptime fn module_path(self) -> str;
    pub pure comptime fn callable_name(self) -> str;
}

pub struct CallDescription {
    pub pure comptime fn callee_name(self) -> str;
    pub pure comptime fn param_name(self) -> Symbol;
    pub pure comptime fn call_file(self) -> str;
    pub pure comptime fn call_line(self) -> u32;
    pub pure comptime fn call_column(self) -> u32;
    pub pure comptime fn call_byte_start(self) -> usize;
    pub pure comptime fn call_byte_end(self) -> usize;
    pub pure comptime fn expansion_id(self) -> Integer;
}

pub trait ImplicitPassing
where
    param_name: Symbol,
{
    assoc ArgType: Type;

    pure comptime fn pass(
        caller: Context,
        callee: Context,
        calling: CallDescription,
    ) -> < |- ArgType >;
}
```

> **包依赖层级说明**：`builtin` 位于依赖最底层，严禁反向导入 `meta`。因此 `CallDescription` 暴露纯原语位置字段（`call_file`、`call_line`、`call_byte_start`、`call_byte_end`、`expansion_id`）；上层 `meta` 包自身拥有名义类型 `meta.ast.SourceSpan`，直接在 `meta` 中将其封装为 `SourceSpan` 并实现 `ImplicitPassing<.__src__> for meta.ast.SourceSpan`，完全符合 [docs/traits/coherence.md](../../docs/traits/coherence.md) 包归属规则！

#### 3.2.2 六大内置隐式供应源完整定义

| 形参声明惯例 | 查询路径与主语类型 | 产出值阶段与生成内容 | 归属实现位置 |
|---|---|---|---|
| `implicit __src__: meta.ast.SourceSpan` | `ImplicitPassing<.__src__> for SourceSpan` | 运行期/编译期常量 `SourceSpan` 值（文件名、起止字节偏移、宏展开链） | `meta`（`SourceSpan` 归 `meta`） |
| `implicit __caller__: str` | `ImplicitPassing<.__caller__> for str` | 调用方所在函数/块限定的可读限定路径字符串（`caller.callable_name()`） | `builtin`（`builtin` 拥有 `ImplicitPassing`） |
| `implicit __module__: str` | `ImplicitPassing<.__module__> for str` | 调用点所在模块全路径（`caller.module_path()`） | `builtin` |
| `implicit __package__: str` | `ImplicitPassing<.__package__> for str` | 调用点所属包名/身份标识（`caller.package_name()`） | `builtin` |
| `implicit __file__: str` | `ImplicitPassing<.__file__> for str` | 调用点源文件路径（`calling.call_file()`） | `builtin` |
| `implicit __line__: u32` | `ImplicitPassing<.__line__> for u32` | 调用点 1-based 源码行号（`calling.call_line()`） | `builtin` |

`__src__` 与 `__line__` 标准实现示例（跨阶段常数值通过 `Splice` 直接进入 `'( ... )`，注意 `~<...>` 仅用于引用代码 `< Ψ |- T >`，非代码值不得包裹 `~<...>`）：

```flurry
-- 位于 meta 包（SourceSpan 为本地名义类型，满足对 builtin.ImplicitPassing<.__src__> 的本地身份规则）
impl builtin.protocols.implicit.ImplicitPassing<.__src__> for meta.ast.SourceSpan {
    assoc ArgType: Type = meta.ast.SourceSpan;

    pure comptime fn pass(
        caller: builtin.protocols.implicit.Context,
        callee: builtin.protocols.implicit.Context,
        calling: builtin.protocols.implicit.CallDescription,
    ) -> < |- meta.ast.SourceSpan > {
        let span_val = meta.ast.SourceSpan.new(
            calling.call_file(),
            meta.from_usize(calling.call_byte_start()),
            meta.from_usize(calling.call_byte_end()),
            calling.expansion_id(),
        );
        -- 假设 SourceSpan 按 03 WL-03-17 实现 Splice（RuntimeTarget = SourceSpan）
        '( span_val )
    }
}

-- 位于 builtin 包（拥有 ImplicitPassing trait，故可为原语 u32 / str 实现内置源）
impl ImplicitPassing<.__line__> for u32 {
    assoc ArgType: Type = u32;

    pure comptime fn pass(
        caller: Context,
        callee: Context,
        calling: CallDescription,
    ) -> < |- u32 > {
        let line_no = calling.call_line();
        '( line_no )
    }
}
```

#### 3.2.3 路由优先级、`.name = expr`、`...spread`、可选默认值与 `implicit quote` 交互规则

依据 [docs/functions/parameter-elaboration.md](../../docs/functions/parameter-elaboration.md#L86-L125)：

1. **显式 `.name = expr` 优先级绝对高于 `ImplicitPassing`**：一旦调用处写明 `.limit = 3` 或 `.bias = flag`，立即标记该 `implicit` 槽位为直接供应，**完全跳过 `ImplicitPassing` 查找**。
2. **禁止作用域内同名变量隐式捕获**：即使调用方局部存在 `let bias: u32 = 10;`，省略 `.bias` 时也**绝不按局部变量名字或类型在作用域搜寻替代**，必须查 `ImplicitPassing<.param_name> for ParamTy`；找不到有效实现即报错（`MissingImplicit`）。
3. **与 `...spread` 元组及扩展属性交互**：位置展开包（`...args`）和冒号属性包（`bias: 3`）**不占用 `implicit` 参数槽位**；显式 `.name = expr` 可以出现在 `...spread` 前后任意位置，按源码书写顺序求值。
4. **与前序/后序依赖可选默认值交互**：前序 `implicit` 输入先被供应绑定后，后序 `.opt = expr` 默认表达式可以读取它（如 `fn check(implicit __line__: u32, .tag: u32 = __line__)`）；显式传入 `.opt` 时照常跳过默认计算。
5. **与 `comptime` 及 `quote fn` 的 `implicit quote` 交互**：
   - `implicit comptime T: Type` 要求 `pass` 返回的编译时值直接作为编译时参数实例化；不允许把 runtime 表达式赋给 `implicit comptime` 槽位。
   - `quote fn` 中的 `implicit quote ctx: ...` 按 contextual 绑定语义从外围上下文提取运行时目标绑定，不退化为运行期对象内容读取。

**反例（必须拒绝）**：
- `let __line__: u32 = 99; fn log_val(x: i32, implicit custom_tag: u32) -> void { ... } log_val(1);` -> 拒绝：未给 `.custom_tag` 提供 `ImplicitPassing<.custom_tag>` 实现，绝不捕获调用处变量。
- `fn bad(implicit comptime flag: bool) -> void { ... } bad(.flag = runtime_bool_fn());` -> 拒绝：即使显式按名供应，也不得用 runtime 值满足 `comptime` 形参。
- 用户声明两份不同模块导入或重叠范围的 `impl ImplicitPassing<.ctx> for MyCtx` -> 拒绝：coherence 重叠冲突，必须唯一确定。

---

### 3.3 `Extension` 与 `derive` 一致性（Coherence）调和

#### 3.3.1 规范冲突识别

`proposal_trace/2026-9-12-implitcit-substitution.fl:41-62` 曾提出两项放宽设想：
1. 引入特殊 `Extension<.method_name>` trait 配合 `derive fn ... for Extension<.span_to_vec4>` + `derive Extension<.span_to_vec4>.by_my_deriver for u32;` 模拟全局固有扩展（inherent extension）；
2. 允许第三方写 `derive SomeForeignTrait.by_my_deriver for ForeignType;`。

对照正式权威文档，这两项设想均**直接与现行语言核心不变量冲突**：
- [docs/traits/coherence.md](../../docs/traits/coherence.md#L24-L43) **有序本地身份规则**明确规定：`Symbol`、数值、`str` 不授予任何包归属权；实现外部 trait 时，从外到内、从左到右检查规范化主体和 trait 实参，**在抵达首个归属当前包的本地名义身份之前，不得存在自由变量或只有外部结构**。
- [docs/traits/derivation.md](../../docs/traits/derivation.md#L59-L64) 明确规定：**派生策略身份不赋予产物额外的包归属权**；默认策略注册自身就必须具备本地 trait 或本地前缀名义身份，具名 `.use(strategy)` 生成出的 `impl` 仍必须通过全套 `coherence` 检查。`compiler/elaboration/src/program.rs:2245-2261` 已将该规则严格编码入编译器。

#### 3.3.2 权威调和结论

1. **禁止 `Extension<.tag>` 作为绕过孤儿规则的漏洞**：在标准包生态中，如果任意包都能为 `u32` 注册 `Extension<.foo>`，两个独立第三方库使用同一语义名 `.to_json` 或 `.clamp` 会在下游合流时爆发不可追踪的方法污染与歧义；而带长后缀 `Extension<.pkg_v1_foo>` 则等于退化成繁琐函数前缀。
2. **统一使用官方“本地名义 `newtype` + `.as(LocalView)` 接收者视角”范式**（[docs/traits/coherence.md](../../docs/traits/coherence.md#L57-L81)、[docs/traits/derivation.md](../../docs/traits/derivation.md#L61-L64)）：
   - 第三方库在自己包内声明 `pub newtype LocalSlice = Slice<T> where T: Type;` 或 `pub newtype Vec4Ops = u32;`，为其实现本地或外部 trait（`impl SomeTrait for LocalSlice<T>`）；
   - 调用方在参数类型或局部绑定中标注 `x: Slice<u32>.as(LocalSlice<u32>)` 或 `x.as(LocalSlice<u32>).method()`，接收者适配第一层显式视角自动提供便利调用，且零全局污染、100% 可模块化并行编译！
3. **保留讨论入口**：如作者希望在不修改外部类型签名的前提下提供局部作用域的“命名方法扩展语法糖”，已作为 `OQ-03-03` 列入 §5 供作者明确拍板，未裁决前**严禁削弱 `coherence` 或 `derivation` 归属检查**。

---

### 3.4 Tier 1 内置 `derive` 目录与 Tier 2 生态派生支撑

#### 3.4.1 `derive fn` 语法、参数化能力与机制缺口

依据 [docs/traits/derivation.md](../../docs/traits/derivation.md#L11-L18)：

```text
deriver_def -> derive fn id (parameter*)? for expr clauses? { (assoc_decl | definition | statement)* }
derive_def  -> comptime? derive expr for expr clauses?
```

**编译器当前真实缺口**（实证源自 `compiler/elaboration/src/program/derivation.rs` & `library/attrs/derivation/README.md`）：
1. **`^attrs.derivation.default` 关键词导出阻塞**（`WL-03-13`）：编译器 `attributes/derivation.rs` 已经消费内部键 `{ __flurry_derivation_default: bool }`，但因 `default` 在 `docs/basics/lexical.md` 与 `compiler/parse/tests/derivation.rs` 中仍为全保留关键字，导致 `library/attrs/derivation/` 无法导出规范路径 `attrs.derivation.default`。
2. **策略形参、子句与约束实例化阻塞**（`WL-03-12`）：`derivation.rs:39-44` 拒绝任何带有 `clause_params` 的 `derive fn`（`"strategy input and contract instantiation"`）；`derivation.rs:152-158` 拒绝泛型类型上携带形参/前置约束的 `derive` 请求（`"strategy inputs in generic request environments"`）。
3. **策略体可组合编译时计算与安全字段投影**（`WL-03-15`）：`derive fn` 体内执行发生在编译期，需要完整的 `meta.reflection.{reflect_struct, reflect_record, reflect_enum}` + 类型安全的 `builtin.field_ptr` / `meta.reflection.field_value` 原语，才不必在所有 `Eq`/`Hash`/`Clone` 派生中强行依赖 `unsafe` 字节偏移转型（尤其 `comptime derive` 禁止指针，无法使用 `byte_offset`！）。

#### 3.4.2 Tier 1 内置派生策略完整目录（严格限于 `builtin` 与 `meta`）

> 严格执行 Board #10 边界：`Serialize` / `Deserialize` **不属于**本表；本表仅收录语言核心资源、比较、哈希、默认值、格式化、阶段拼接与 AST 词法 token 转换 trait。

| Trait（与归属包） | 默认策略（`^attrs.derivation.default`） | 具名备选策略 | 适用主体范围与自动添加的 `where` 约束 | 关键语义、资源/依赖透传与拒绝边界 |
|---|---|---|---|---|
| `Eq`（`builtin.numeric.comparison`） | `by_fields` | `by_fields` | `struct`、`enum`（全变体）；自动追加所有观测字段类型 `F :- Eq`（若 `Self` 带 `.A` 分配器子句则保留原样透传） | 遵照 `01` 修正版 `fn eq(*self, other: *Self) -> bool`：先比 `enum` 判别标签，再逐字段比较 `self.field` 与 `other.field`（绝不消费 `self`） |
| `Ord`（`builtin.numeric.comparison`） | `by_fields` | `by_fields` | `struct`、`enum`；要求 `Self :- Eq`（须先实现或同 derive `Eq`）且各字段 `F :- Ord` | 按字段声明序（非物理重排偏移序）与枚举变体声明序做字典序比较，先决不相等即提前返回 `.less` / `.greater` |
| `Hash`（`builtin.hash`） | `by_fields` | `by_fields` | `struct`、`enum`；要求各字段 `F :- Hash` | 遵照 `01` 修正版 `fn hash(*self, state: *H) -> void where H: Type, requires H :- Hasher`；枚举先写入判别索引防不同变体哈希撞车，再逐字段喂入 `state` |
| `Default`（`builtin.protocols.conversion`） | `by_fields` | `by_fields`, `with_struct_defaults` | 仅 `struct`（或配合 `^attrs.derivation.default_variant` 标记单元变体的 `enum`？不猜测，无标记则拒绝）；要求未声明字段默认表达式的字段 `F :- Default` | 按字段声明顺序依次求值：若存在字段级声明默认式且选择 `with_struct_defaults` 优先采用，否则调用 `F.as(Default).default_value()`；注意保留 `allocator` 依赖传递 |
| `Clone`（`builtin.protocols.resources`） | `by_fields` | `by_fields` | `struct`、`enum`；要求各字段 `F :- Clone` | `fn clone(*self) -> Self`；若后续字段 `clone()` 失败/分配出错或抛效应退出，已克隆的前序拥有字段必须按逆序安全 `Drop`！支持携带 `quote .allocator` 的容器字段 |
| `Copy`（`builtin.protocols.resources`） | `by_fields` | `by_fields` | `struct`、`enum`；要求 `Self :- Clone`，全部字段 `F :- Copy`，且 **`Self` 未声明自定义 `Drop`** | 生成空标记实现 `impl Copy for Self`（按位复制语义合法化） |
| `Debug`（`builtin.protocols.formatting.debug`） | `by_fields` | `by_fields`, `compact` | `struct`、`enum`；要求字段 `F :- Debug` | 遵照 `04` `fn fmt(*self, f: *Formatter) -> !FormatErr void`，经 `DebugStruct` / `DebugTuple` 格式化输出结构体名与字段名/负载 |
| `Display`（`builtin.protocols.formatting.display`） | **无通用默认**（结构无单一展示格式） | `by_variant_name`, `for_newtype`, `debug_fallback` | `by_variant_name` 限无载荷纯枚举；`for_newtype` 限单字段新类型包装且内部字段 `:- Display` | 避免任意 `struct` 自动派生不规范用户文案；强制由作者显式 `.use(...)` 选择策略 |
| `Splice`（`builtin.protocols.staging`） | `by_fields` | `by_fields` | `struct`、`enum`；要求全部字段 `F :- Splice` 且 `Self` 非 `ComptimeOnly`、不含禁止跨阶段的运行期句柄 | `assoc RuntimeTarget: Type = Self;`；生成将编译期常量各字段递归 `~` 拼接成等价运行期构造的 `fn splice(comptime data: Self) -> Self` |
| `ToTokens`（`meta.ast`） | `by_fields` | `by_fields` | 编译期 AST/词法结构 `struct`、`enum`；要求字段 `F :- ToTokens` | 生成 `pure comptime fn into_tokens(self) -> TokenBuffer` 与 `into_tokens_with_spacing`，仅可用在 `meta` / 宏编译期结构 |
| `ComptimeEq` / `ComptimeOrd` / `ComptimeHash` / `ComptimeClone` / `ComptimeDefault` / `ComptimeDebug` / `ComptimeDisplay`（`meta.*`） | `by_fields`（与对应 `Comptime*` trait 同名） | 同 Runtime 对应策略 | 用于 `comptime derive ComptimeEq for MyType;`（支持泛型参数 `where` 约束自动映射至 `Comptime*`） | 与 Runtime 版方法名完全相同，全部使用无指针 `self` 值引用语义（`pure comptime fn eq(self, other: Self) -> bool` 等） |

#### 3.4.3 Tier 1 派生器完整 Flurry 定义（无省略示范）

为避免占位式规格，以下给出 `Eq.by_fields`、`Hash.by_fields`、`Clone.by_fields` 与 `ComptimeEq.by_fields` 的完整 `derive fn` 实现体（利用 `05` `reflect_struct` / `reflect_enum` 与安全编译期索引字段投影 `builtin.field_ptr` / `meta.reflection.field_value`）：

```flurry
-- 1. Runtime Eq 默认策略（library/builtin/numeric/comparison.fl）
^attrs.derivation.default
pub derive fn by_fields for Eq {
    const STRUCT_INFO = meta.reflection.reflect_struct(Self);
    const ENUM_INFO = meta.reflection.reflect_enum(Self);

    fn eq(*self, other: *Self) -> bool {
        inline if STRUCT_INFO is info! do {
            inline for field in info.fields {
                inline if field.name is fname? do {
                    const fty = field.FieldType();
                    let lhs_f = builtin.field_ptr(self, fname);
                    let rhs_f = builtin.field_ptr(other, fname);
                    if not fty.as(Eq).eq(lhs_f, rhs_f) {
                        return false;
                    }
                } else {
                    meta.compilation.err("Eq.by_fields requires named fields");
                }
            }
            true
        } else {
            inline if ENUM_INFO is einfo! do {
                -- 对比两个 enum 判别标签及同变体各字段（通过变体匹配协议与反射逐一核验）
                meta.reflection.enum_fields_eq(Self, self, other)
            } else {
                meta.compilation.err("Eq.by_fields supports only nominal structs and enums");
            }
        }
    }
}

-- 2. Runtime Hash 默认策略（library/builtin/hash.fl）
^attrs.derivation.default
pub derive fn by_fields for Hash {
    const STRUCT_INFO = meta.reflection.reflect_struct(Self);
    const ENUM_INFO = meta.reflection.reflect_enum(Self);

    fn hash(*self, state: *H) -> void
    where
        H: Type,
        requires H :- Hasher,
    {
        inline if STRUCT_INFO is info! do {
            inline for field in info.fields {
                inline if field.name is fname? do {
                    const fty = field.FieldType();
                    let ptr = builtin.field_ptr(self, fname);
                    fty.as(Hash).hash(ptr, state);
                }
            }
        } else {
            inline if ENUM_INFO is einfo! do {
                meta.reflection.enum_fields_hash(Self, self, state);
            } else {
                meta.compilation.err("Hash.by_fields supports only nominal structs and enums");
            }
        }
    }
}

-- 3. Runtime Clone 默认策略（library/builtin/protocols/resources.fl）
^attrs.derivation.default
pub derive fn by_fields for Clone {
    const STRUCT_INFO = meta.reflection.reflect_struct(Self);

    fn clone(*self) -> Self {
        inline if STRUCT_INFO is info! do {
            -- 借助 meta/编译器受检结构体逐字段构造，确保若后续字段 clone 触发清理，前序字段自动 Drop
            meta.reflection.construct_struct_from_cloned_fields(Self, self)
        } else {
            meta.compilation.err("Clone.by_fields supports structs and requires enum handler");
        }
    }
}

-- 4. Comptime Eq 对偶策略（library/meta/numeric/equality.fl，零 *self / .ref / .*）
^attrs.derivation.default
pub derive fn by_fields for ComptimeEq {
    const STRUCT_INFO = meta.reflection.reflect_struct(Self);

    pure comptime fn eq(self, other: Self) -> bool {
        inline if STRUCT_INFO is info! do {
            inline for field in info.fields {
                inline if field.name is fname? do {
                    const fty = field.FieldType();
                    let lhs_val = meta.reflection.field_value(self, fname);
                    let rhs_val = meta.reflection.field_value(other, fname);
                    if not fty.as(ComptimeEq).eq(lhs_val, rhs_val) {
                        return false;
                    }
                }
            }
            true
        } else {
            meta.compilation.err("ComptimeEq.by_fields supports nominal structs");
        }
    }
}
```

#### 3.4.4 Tier 2 生态派生机制示范（交叉引用 `10-official-protocol-packages-and-boundary.md`）

严格遵循三层边界：`Serialize` 属于 Tier 2 官方薄协议包 `serialization`，`json_lib` 属于 Tier 3 生态实现包，两者**均不在六个内置包内**。下例演示 Flurry 通用派生引擎（`WL-03-12`、`WL-03-18`）如何支持泛型宿主结构体携带运行期依赖 `allocator` 完成无隐式分配派生：

```flurry
-- 外部用户包（仅依赖 Tier 2 serialization 与 Tier 3 json_lib；严禁写入 builtin/std！）
use serialization.Serialize
use std.collections.Vec
use std.mem.{Allocator, DefaultAllocator}
use std.string.String

pub struct Data
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    pub id: u64,
    pub label: String<.A = A, .allocator = allocator>,
    pub tags: Vec<u32, .A = A, .allocator = allocator>,
}

-- 1. Tier 1 基础能力可直接默认派生（逐个声明独立 derive 并透传容器子句，where 后无尾随分号）：
derive Eq for Data<.A = A, .allocator = allocator>
where
    A: Type,
    requires A :- Allocator,
    quote allocator: lift *A

derive Hash for Data<.A = A, .allocator = allocator>
where
    A: Type,
    requires A :- Allocator,
    quote allocator: lift *A

derive Debug for Data<.A = A, .allocator = allocator>
where
    A: Type,
    requires A :- Allocator,
    quote allocator: lift *A

-- 2. Tier 2 序列化协议显式选用 Tier 3 零堆分配策略：
--    注：json_lib 不拥有 Serialize<.json>（.json Symbol 不授归属权），故 json_lib 不可注册默认策略，
--    而由 Data 所在包显式写出 `.use(json_lib.without_allocation)`。
derive Serialize<.json>.use(json_lib.without_allocation) for Data<.A = A, .allocator = allocator>
where
    A: Type,
    requires A :- Allocator,
    quote allocator: lift *A
```

#### 3.4.5 派生引擎通用不变量与事务缓存键

1. **产物回归普通检查与零残留**：策略成功生成成员后，产物进入标准 implementation 检查流程（包归属、同精度重叠冲突、关联项完整性、方法签名/phase/效果/资源契约校验）。任一步骤失败必须原子回滚，**绝不在实现世界中留下半份可见实现**。
2. **禁止多策略伪造多份身份**：对同一主体 `Data`、同一完整实例 `FormatLabel<.json>` 与同一 phase，分别写出 `derive FormatLabel<.json>.use(s1) for Data;` 与 `derive FormatLabel<.json>.use(s2) for Data;` 构成同精度重叠冲突，直接拒绝（即使生成的方法体文本完全一致也不能因正文相同而去重）。
3. **默认策略本地注册检查与完备缓存键**：注册 `^attrs.derivation.default` 时，必须验证至少存在本地 trait 或规范实参首个本地名义身份（无前置自由量化变量）；派生缓存键必须包含：`策略 DefId` + `策略输入实参` + `主体完整类型头` + `trait 完整实参` + `phase` + `策略观察到的反射结构摘要` + `当前可见实现世界版本`。

**反例（必须拒绝）**：
- `struct Bad { f: NonCloneBox, } derive Copy for Bad;` -> 拒绝：`NonCloneBox` 未实现 `Copy`（且拥有非平凡 `Drop`），生成的 `impl Copy` 在产物校验期被拒。
- `derive FormatLabel<.json>.use(s1) for Data; derive FormatLabel<.json>.use(s2) for Data;` -> 拒绝：同主体、同完整 trait 实例、同 phase 重叠实现冲突。
- 在 `json_lib` 中写 `^attrs.derivation.default pub derive fn bad for Serialize<.json> { ... }`（其中 `Serialize` 在外部包 `serialization` 且 `.json` 为 `Symbol`）-> 拒绝：`Symbol` 不提供本地身份，默认策略注册被包归属规则拦截。
- 在声明于 `builtin` 的类型或策略体内引用未在 `where` 中透传的隐藏 `allocator`（或硬编码 `SystemAllocator`）-> 拒绝。

---

### 3.5 `post do`、`tactic lambda`、接收者适配与统一调用准备计划

#### 3.5.1 `post do` 路由与 `tactic lambda`（`_0`, `_1`, ...）规则

依据 [docs/functions/parameter-elaboration.md](../../docs/functions/parameter-elaboration.md#L25-L85)：

1. **`post do` 唯一目标路由**：
   - 先检索被调签名中的所有形参：若存在**唯一**带有 `lambda` 修饰的形参（无论其处于参数表末尾还是中间，如 `inspect_with(lambda action, value)`），`post do` 正文路由至该形参；
   - 若签名中无 `lambda` 形参，则路由至**最后一个锚定参数**；
   - **必须拒绝的异常情形**：签名中出现多个 `lambda` 标记参数且无法消歧；无 `lambda` 标记且最后一个位置非锚定参数（或签名无参数）；目标形参已被位置实参或 `.name = expr` 重复供应。
2. **`tactic lambda` 边界与资源契约**：
   - 只有声明了 `lambda` 标记的形参槽位（或作为 `post do` 接入该带标记的槽位）才开启 `_0`、`_1`、... 下标占位符作用域；未带 `lambda` 标记的函数形参可以使用显式闭包 `|x| ...` 或 `post do |x| ...`，但**禁止使用 `_0`**。
   - 目标参数表完整 arity 决定合法下标集合（最大出现下标不缩减也不扩充形参数目；例如二元回调中使用 `_0 == _0` 仍构建二元闭包；出现 `_2` 直接报越界错误）。
   - **非 `Copy` 占位符防重复消耗铁律**：重复写 `_0 + _0` 或 `use_twice(_0, _0)` 引用同一份已取得的闭包的形参参数；若形参类型为非 `Copy` 的 `Once` 所有者且闭包体两次消费它，资源分析按“二次移动同一参数”直接拒绝，**绝不重新执行调用处外围实参来复制出第二份值**。
   - **作用域隔离**：嵌套的 `lambda` 标记实参位置以及显式 `|...|` 闭包各自建立全新内部边界，内部 `_0` 遮蔽且不穿透访问外层 `_0`（需要跨层访问时应显式写具名 lambda）。

```flurry
-- 正例：lambda 标记位于非末尾位置，与可选命名参数、tactic lambda 组合
fn inspect_with(
    lambda check: fn(u32) -> bool,
    value: u32,
    .fallback: bool = false,
) -> bool {
    if value == 0 { fallback } else { check(value) }
}

test {
    let ok = inspect_with(7, .fallback = true) do { _0 < 10 }
}

-- 正例：在 comptime 开放 defer 签名中检查依赖边界后的 tactic lambda
pure comptime fn Select(flag: bool, T: Type) -> Type {
    if flag { T } else { bool }
}

fn apply(comptime flag: bool, lambda action: defer fn(T) -> T, value: T) -> defer Select(flag, T)
where
    T: Type,
{
    inline if flag {
        action(value)
    } else {
        let _ = action(value);
        false
    }
}

fn run_open() -> bool {
    apply(true, false) do { not _0 }
}
```

#### 3.5.2 三层接收者适配引擎与目标引导读取

整合 [docs/type/receiver-adaptation.md](../../docs/type/receiver-adaptation.md)、[docs/type/member-projection.md](../../docs/type/member-projection.md) 与 `01` 中 `WL-01-22`（`Deref`、`DerefMut`、`Reference`）：

```text
接收者适配查找唯一三层优先序（严禁按总步数跨视角越级抢占）：
  Layer 1（显式类型视角，从后往前逆序）：
    给定 receiver: A.as(B).as(C)，查找顺序严格为 C -> B -> A；
  Layer 2（当前视角内解引用深度）：
    从当前视角直接形态开始 -> 逐步执行内建指针 * 解引用与唯一确定的 Deref 链（调用 deref(*self) -> *Target，
    直接取得 *Target 供下步方法调用，绝不拷贝 Target 再取临时地址）；
  Layer 3（同形态内成员来源与末端取址）：
    按成员来源与 normalized IR 结构精度选择唯一实现；
    若选定方法要求 *self，触发一次内建 .ref 取址或唯一确定的 Reference 适配（不反过来扩展查找链）。
```

**关键运行、资源与错误边界**：
1. **无空跑副作用**：多候选探测只形成“适配路径规划 + 契约要求”，绝不运行未被选中分支的 `Deref` 或 `Reference` 体代码。
2. **`Unknown` 阻断**：更高优先视角或更浅解引用深度上出现 `Unknown`（如 `Deref.Target` 尚待推断）或同类条件未决时，挂起该路径等待证据，**绝不越级跳过并采用低优先候选**。
3. **防循环与有限展开**：记录已访问的归一化接收者类型头与深度上限；`Deref` 自环（如 `SelfRefPtr` -> `*SelfRefPtr`）或持续构造膨胀包装类型在到达重复头/深度限额时立即报错。
4. **目标引导内建指针读取**：仅当检查位置已有**明确非指针 `Copy` 目标类型**（初始化 `let x: u32 = ptr.field;`、参数传值、返回结果）且指针有效时，自动插入内建 `Copy` 读取；无明确目标的 `let f = s.ref.field;` 保持 `*FieldType`，**非 `Copy` 对象绝不自动解引用搬出**！
5. **Comptime 严格禁用指针适配**：comptime 方法接收 `self`（值引用），不经过 `Deref`/`Reference` 或内建指针解引用层。

#### 3.5.3 统一调用准备计划（Call Preparation Plan）

整合 [docs/functions/calls-and-returns.md](../../docs/functions/calls-and-returns.md) 与 [docs/functions/parameters.md](../../docs/functions/parameters.md)，任意复杂调用（同时含接收者适配、位置/扩展收集、`.named`、`implicit`、依赖可选默认值、`post do`）按如下严格顺序构建准备计划：

1. **阶段检查与实参路由**：区分并固定 `comptime`/`quote`/runtime 形参；路由所有显式实参到对应形参或收集包，禁止重复、未知命名和混合整包/追加。
2. **源码顺序一次性求值**：
   - 若为实例方法调用，先求值**接收者表达式**（及其选中 `Deref` 适配动作）一次；
   - 按调用点从左到右源码书写顺序，依次求值各显式位置实参、`.name = expr`、`...spread` 生产者、属性字段表达式以及 `post do` 闭包环境捕获；
   - 若途中任一表达式触发控制流跳转（`return`/`break`/错误传播 `!`）或为 `Never`，停止后续所有实参求值与收集构造，按**已初始化顺序逆序**释放此前在本准备区已创建的临时对象与捕获环境，**绝不调用目标函数**。
3. **收集构建与隐式/默认值依存顺序补齐**：
   - 在全部收集原始输入完成求值后，按 `VariadicCollector` / `ExtendedVariadicCollector` 执行栈上 `Tmp` 构造；
   - 运行编译期选定 `ImplicitPassing` 注入的代码/值（按声明依存顺序）；
   - 按形参声明先后顺序为**未显式供应**的 `.opt = default_expr` 计算默认值（每次只执行未被覆盖项的默认式）；每条运行期默认计算必须通过 **P3 稳定观测、有限完成、无非 `Copy` 责任复制、允许已知稳定工厂与私有临时 Drop** 检查（若默认表达式执行写入外部状态、`println` 输出或依赖后序未声明参数，编译期直接拒绝）。
4. **进入被调函数前置条件检查与清理交接**：所有参数（与 `assoc Tmp` 栈槽）齐备后检查函数 `requires` 前置条件；若前置条件失败，按与参数逆序清理（含 `Tmp`）后返回错误/中止；前置条件通过后移交实参所有权进入被调主体，主体退出时按正常作用域逆序销毁 `Tmp` 及未移走的临时值。

**负例（必须拒绝）**：
- `fn bad(.first: u32 = second, .second: u32 = 1) -> u32 { first }` -> 拒绝：前序默认值引用尚未声明的后序参数 `second`。
- `fn noisy() -> u32 { println("side effect"); 8 } fn bad_def(.limit: u32 = noisy()) -> u32 { limit }` -> 拒绝：运行期默认计算包含输出副作用，违反 P3 稳定观测契约。
- `apply(owned_non_copy) do { consume(_0); consume(_0) }` -> 拒绝：同一 `tactic lambda` 槽位两次消耗非 `Copy` 形参 `_0`。
- `apply_plain(5, _0 + 1)`（其中 `apply_plain` 形参未标注 `lambda`）-> 拒绝：未带 `lambda` 标记不得使用 tactic 占位符。

---

### 3.6 `language-items.conf` 协议绑定与注册表扩展

对齐 [01-syntax-protocols-and-operators.md](file:///home/fin/luna/tmp/wishlist/01-syntax-protocols-and-operators.md) 第 3.12.2 节与 `SysrootBindings`（`compiler/luna/src/protocols/type_constructors.rs`），明确区分既有实配与新增绑定：

| 配置键（`language-items.conf`） | 对应 CLI 参数 | 当前实配路径 | 目标路径 | 状态与职责 |
|---|---|---|---|---|
| `variadic-collector-protocol` | `--variadic-collector-protocol` | 未配置 | `builtin.protocols.ops.VariadicCollector` | 待新增；与 `01` `SpaceApply` 及位置 `...args` 收集共用 |
| `extended-variadic-collector-protocol` | `--extended-variadic-collector-protocol` | 未配置 | `builtin.protocols.ops.ExtendedVariadicCollector` | 待新增；驱动 `{ prop: val, ... }` 收集 |
| `comptime-variadic-collector-protocol` | `--comptime-variadic-collector-protocol` | 未配置 | `meta.ops.ComptimeVariadicCollector` | 待新增；编译时位置收集 |
| `comptime-extended-variadic-collector-protocol` | `--comptime-extended-variadic-collector-protocol` | 未配置 | `meta.ops.ComptimeExtendedVariadicCollector` | 待新增；编译时属性收集 |
| `implicit-passing-protocol` | `--implicit-passing-protocol` | 未配置 | `builtin.protocols.implicit.ImplicitPassing` | 待新增；驱动未显式传入的 `implicit` 形参解析 |
| `call-context-type` | `--call-context-type` | 未配置 | `builtin.protocols.implicit.Context` | 待新增；构造传入 `ImplicitPassing.pass` 的调用上下文 |
| `call-description-type` | `--call-description-type` | 未配置 | `builtin.protocols.implicit.CallDescription` | 待新增；提供源码跨度、行号与文件名元信息 |
| `source-span-type` | `--source-span-type` | 未配置 | `meta.ast.SourceSpan` | 待新增（与 `05` 协同）；支持 `__src__` 构造与 `meta.compilation.err` 诊断定位 |
| `deref-protocol`, `deref-mut-protocol`, `reference-protocol` | `--deref-protocol`, `--deref-mut-protocol`, `--reference-protocol` | 未配置 | `builtin.protocols.ops.{Deref,DerefMut,Reference}` | 与 `01` 对齐，服务三层接收者适配引擎 |
| `field-ptr-intrinsic`, `tuple-into-array-intrinsic`, `comptime-field-value-intrinsic`, `enum-derive-helpers` | 无（`builtin`/`meta` 集中绑定） | 未配置 | 集中于 `builtin/bindings.fl` 与 `meta/bindings.fl` | 支撑通用 `derive fn` 安全按编译期 `Symbol` 投影字段、枚举派生比较/哈希与 `Array` 变参构建 |

---

## 4. 排期优先级与分阶段交付路线

### Phase 0（`P0` — 解除 Silver DSL、泛型派生与调用准备核心阻塞）

- **前置依赖**：依赖 `01` 完成 `*self` 签名与 `language-items.conf` 基础设施；`WL-03-06` 依赖 `05` 的 `WL-05-13`（匿名记录 `Ps` 反射 `RecordInfo`、`has_field`、`FieldType`）。
- **核心交付项**：
  1. **落地 `VariadicCollector` & `ExtendedVariadicCollector` elaboration 协议**（`WL-03-01`, `WL-03-03`, `WL-03-05`, `WL-03-07`, `WL-03-24`）：补齐 `language-items.conf` 绑定；在 `spreads.rs`/`extended.rs` 优先查 collector trait，严格锚定 `assoc Tmp` 栈寿命与逆序清理，并交付 `Array`、`Vec`（透传依赖 allocator）标准实现及 Silver `ComponentPropertySet` / `ComponentChildrenList` 验收路径。
  2. **匿名记录 `Ps` 编译期字段反射 + occurrence narrowing**（`WL-03-06`）：联动 `05` 打通 `reflect_record(Ps)` 与 `inline if` 字段存在性/类型收窄，实现属性拼写报错与零运行期开销访问。
  3. **解决 `default` 关键字导出冲突与放开带参/带约束 `derive fn`**（`WL-03-12`, `WL-03-13`）：允许 `attrs.derivation.default` 正常导出；清除 `derivation.rs:39-44, 152-158` 的 `unsupported` 阻断，打通泛型结构体与策略参数实例化。
  4. **实现安全字段投影原语并落地基础 Tier 1 派生器**（`WL-03-15`）：交付 `Eq`、`Ord`、`Hash`、`Default` 的 `by_fields` 默认策略。
  5. **统一调用准备计划与 `post do`/`tactic lambda` 边界收口**（`WL-03-19`, `WL-03-20`, `WL-03-23`）：确保源序求值、局部异常清理、`tactic lambda` 非 `Copy` 二次移动拒收与 `post do` 协同定制 collector 全部工作。
- **P0 验收测试大纲**：
  - `variadic_collector_tmp_lifetime.rs`：验证含 `Drop` 日志的 `Tmp` 缓冲必须在被调函数返回后（且若后续实参触发早退则在清理后）按逆序析构，不悬垂、不提前释放；
  - `silver_extended_properties_ps.rs`（Layer-2 用户包测试）：正例验证 `vbox { width: 120u32, padding: 8u32 }` 零分配收集；负例（compile-fail）验证 `vbox { pading: 8u32 }` 在编译期报错并指向精确属性 Span；
  - `derive_generic_allocator_eq_hash.rs`：带 `.A: Allocator` + `quote .allocator` 的自定义结构分别写 `derive Eq for MyBoxedPair<.A = A, .allocator = a> ...` 与 `derive Hash for ...` 正常通过一致性与运行验证；
  - `tactic_lambda_affine_reject.rs`（compile-fail）：两次使用同一非 `Copy` `_0`、越界 `_2` 及未标 `lambda` 位置写 `_0` 全部被精确拒绝。

### Phase 1（`P1` — `ImplicitPassing`、Comptime 对偶、`Deref` 适配与全套 Tier 1 + Tier 2 派生）

- **前置依赖**：Phase 0 完成；`Deref`/`DerefMut`/`Reference` 与 `ToTokens`/`Splice` 对接 `01` `WL-01-22`、`04` 与 `05`。
- **核心交付项**：
  1. **完整接入 `ImplicitPassing` 与六大内置隐式源**（`WL-03-08`, `WL-03-09`, `WL-03-10`）：在 `open_functions.rs` / `input_routing` 启用 `ImplicitPassing<.param_name> for ArgType` 供应管线（包括开放调用与 `.name = expr`、`...spread`、依赖默认交互）。
  2. **补齐 Comptime collector 对偶与 `comptime derive` 族**（`WL-03-02`, `WL-03-04`, `WL-03-15`, `WL-03-16`）：交付 `ComptimeVariadicCollector`、`ComptimeExtendedVariadicCollector` 与 `meta.List`/`meta.Object` 实现；交付 `ComptimeEq`/`ComptimeOrd`/`ComptimeHash`/`ComptimeClone`/`ComptimeDefault`/`ComptimeDebug`/`ComptimeDisplay` 全套零指针 `pure comptime trait` 派生策略。
  3. **交付 Runtime `Clone`/`Copy`/`Debug`/`Display`/`Splice` 与 `ToTokens` 派生**（`WL-03-16`, `WL-03-17`）：验证 `Copy` 无自定义 `Drop` 检查、`Clone` 半克隆失败清理及 `ToTokens` 双向词法派生。
  4. **实现三层接收者适配引擎与目标引导指针 `Copy` 读取**（`WL-03-21`, `WL-03-22`）：集成 `01` 的 `Deref`/`DerefMut`/`Reference`，落地视角逆序、防自环、无空跑、`Unknown` 挂起与目标类型引导读取。
  5. **加固一致性/孤儿归属与 Tier 2 派生生态验证**（`WL-03-11`, `WL-03-14`, `WL-03-18`）：完善 `derive` 完备缓存键与事务回滚；联合 `10` 验证 `derive Serialize<.json>.use(json_lib.without_allocation) for Data` 端到端通过且外源无豁免漏洞。
- **P1 验收测试大纲**：
  - `implicit_passing_sources.rs`：`implicit __src__: meta.ast.SourceSpan`、`__line__`、`__file__`、`__caller__` 自动注入正确源码位置；显式 `.name = ...` 跳过生成；未配置实现且未显式传入时拒绝，绝不捕获同名局部变量；
  - `receiver_adaptation_three_layers.rs`：同时存在 `.as(View1).as(View2)` 与 `Deref` 包装时按准确三层顺次解析；`SelfRefPtr` 自环报循环诊断；`Deref` 副作用计数严格等于 `1`；
  - `comptime_derive_no_pointers.rs`：`comptime derive ComptimeEq for Config;`、`comptime derive ComptimeHash for Config;`、`comptime derive ComptimeClone for Config;` 在编译期常量容器中正常工作；
  - `tier2_serialize_derive_integration.rs`（Tier 2/3 集成用例）：验证带 `FixedBufferAllocator` 的 `Data` 执行 `derive Serialize<.json>.use(json_lib.without_allocation)` 成功，并校验在 `json_lib` 中对外部 `Slice` 注册默认策略被 coherence 拒绝。

### Phase 2 & Phase 3（`P2`–`P3` — 高级策略表达、异步清理与工具链打磨）

- **核心交付项**：
  1. 完善带 `async` 效应与复杂依赖字段类型的 `Clone` / `Default` 策略清理证明；
  2. 为 `ToTokens` / AST 结构增加自定义属性控制（如跳过/重命名字段的通用派生策略组合能力）；
  3. 配合IDE诊断与宏展开链呈现清晰的 `derive` 来源与调用准备计划错误溯源。

---

## 5. 待作者裁决的语义空缺与冲突（Open Questions）

### OQ-03-01：`attrs.derivation.default` 与 `default` 保留关键字的词法/语法冲突
- **来源引用**：[library/attrs/derivation/README.md](../../library/attrs/derivation/README.md#L3-L6)、[docs/traits/derivation.md](../../docs/traits/derivation.md#L18)、`docs/basics/lexical.md`、`compiler/parse/tests/derivation.rs`。
- **冲突描述**：正式文档规定默认策略标记为 `^attrs.derivation.default`，编译器已在 `attributes/derivation.rs` 消费 `__flurry_derivation_default`，但 `default` 在词法与解析测试中被当作保留关键字，阻止在 `library/attrs/derivation/` 中声明名为 `pub const default` 的公开导出项。
- **推荐方案（方案 A）**：保留正式规范写法 `^attrs.derivation.default`，在词法/解析器中将成员路径段及模块导出名位置（`mod.default`、`pub const default`、`expr.default`）放宽为上下文关键字识别，解除 `library/attrs/derivation/` 导出阻塞。
- **备选方案（方案 B）**：将官方属性路径更名为 `^attrs.derivation.default_strategy`（或 `is_default`）并同步更新 `docs/traits/derivation.md` 与 `docs/comptime/official-attribute-keys.md`。

### OQ-03-02：`VariadicCollector.Tmp` 是否升级为参数化关联项 `assoc Tmp<S: Type>: Type`
- **来源引用**：`proposal_trace/2026-9-8-variadic-argument-collecting-rules.fl:11-26`、[docs/traits/declarations.md](../../docs/traits/declarations.md#L13)、[tmp/silver-design/04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md#L30-L35) 与 [01-syntax-protocols-and-operators.md](file:///home/fin/luna/tmp/wishlist/01-syntax-protocols-and-operators.md#L1154-L1166)。
- **冲突描述**：`Slice<T>` 属于 `builtin`（不能依赖 `std.collections.SmallVec` 且不应隐式堆分配）。若 `assoc Tmp: Type` 不以 `S: Type` 为参数，`Slice<T>` 无法写出“随调用元数 `S` 长度而变的精确栈数组 `Array<T, tuple_len(S)>`”，只能硬编码上限或退化为外部 `Vec`；但若将 `VariadicCollector` 改为 `assoc Tmp<S: Type>: Type`，需同步更新 `01` 中的 `SpaceApply` 示例签名。
- **推荐方案（方案 A）**：采纳参数化关联项 `assoc Tmp<S: Type>: Type;`（[docs/traits/declarations.md](../../docs/traits/declarations.md#L13) 已原生支持 `assoc id<parameter*>: expr`），不需要根据 `S` 变化的类型（如 `Vec`/`Array`/`meta.List`/`AxisList`）可直接写 `assoc Tmp<S: Type>: Type = void;`（或借助编译器默认空白映射），同时使 `Slice<T>` 获得零堆分配、无限参数长度的 `Array<T, N>` 栈支持。
- **备选方案（方案 B）**：保持无参 `assoc Tmp: Type;`（与 `2026-9-8` 及 `01` 当前文本一致），将 `Slice<T>` 变参收集限制为仅当 `T` 或调用环境采用固定上限栈缓冲区（如 `SmallVec<T, 8>` 迁入 `builtin` 或仅对 `Array`/`Vec` 提供收集）。

### OQ-03-03：`proposal_trace/2026-9-12` 的 `Extension` 提案与 `coherence` / 孤儿规则边界
- **来源引用**：`proposal_trace/2026-9-12-implitcit-substitution.fl:41-62` vs [docs/traits/coherence.md](../../docs/traits/coherence.md#L24-L43)、[docs/traits/derivation.md](../../docs/traits/derivation.md#L59-L64) 与 `compiler/elaboration/src/program.rs:2245-2261`。
- **冲突描述**：`2026-9-12` 设想通过 `Extension<.id>` 与策略为任意外部类型加方法/外部 trait 实现；但 `coherence.md` 明确以有序本地身份禁止外部 trait + 外部类型组合，并指定用本地 `newtype` + `.as(LocalView)` 解决体验问题。
- **推荐方案（方案 A）**：彻底确认**不引入 `Extension` trait**，坚持 `coherence.md` 与 `derivation.md` 现行规范（所有外部类型扩展走本地 `newtype` + `.as(LocalView)`）；若需简化局部导入视角书写，可考虑参数/块级视角糖而不打破全局包归属。
- **备选方案（方案 B）**：允许“需调用方显式作用域导入才能激活”的局部作用域扩展（不进入全局 impl 世界），但会显著增加成员来源稳定性和缓存失效复杂度。

### OQ-03-04：用户自定义 `ImplicitPassing` 的作用域激活与匿名按类型查询问题
- **来源引用**：`proposal_trace/2026-9-12-implitcit-substitution.fl:26-39`、[docs/functions/parameter-elaboration.md](../../docs/functions/parameter-elaboration.md#L86-L125)。
- **冲突描述**：当前设计使用 `ImplicitPassing<param_name: Symbol> for ArgType`（依赖参数名字+参数类型全局唯一实现），但尚未明确当用户声明 `implicit ctx: MyExternalCtx` 时，如果希望不同调用作用域提供不同默认策略（而非全程序只能绑定一份全局构造体）应如何表达。
- **推荐方案（方案 A）**：在 `ImplicitPassing<.param_name>` 中保持编译期源码级供应定位（主要服务 `__src__`、`__line__`、`__caller__` 及明确具名编译期元数据句柄），运行期动态环境（如 allocator、logger、DB 连接）优先走 **Flurry 代数效应（`effect` / `direct_passing`）或运行期依赖参数 `quote .allocator`**，不把 Scala 式动态隐式参数搜索混入 `implicit`。
- **备选方案（方案 B）**：允许在函数/模块级通过属性或 `use` 声明显式绑定局部 `ImplicitPassing` 策略（类似 `.use(...)`），但需新增局部供应覆盖规则。

### OQ-03-05：通用 `derive fn` 遍历结构体字段的安全无 `unsafe` 投影原语形态
- **来源引用**：`proposal_trace/2026-8-13-derivation.fl:39`、[10-official-protocol-packages-and-boundary.md](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md#L484-L489)、[02-pattern-matching-and-views.md](file:///home/fin/luna/tmp/wishlist/02-pattern-matching-and-views.md#L1896) 与 [05-meta-cmtt-macros-reflection.md](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md#L825-L856)。
- **冲突描述**：`2026-8-13` 草案曾假想 `self'get_field(field.name)`，但 `02` 已排除字段级 Take View 语法污染；`10` 借助 `field.byte_offset` + `unsafe` 指针转换处理运行期 POD 字段，但这既迫使安全 `Eq`/`Hash`/`Clone` 派生进入 `unsafe`，也完全无法用于 `comptime derive`（comptime 无指针）及 `attrs.packed.bit` 字段。
- **推荐方案（方案 A）**：在语言核心集中绑定层导出两项编译期受 PTC `has_field` 守卫的类型安全原语：`builtin.field_ptr(ptr: *S, comptime name: Symbol) -> *FieldType(S, name)`（以及对应的只读/构造原语）与 `meta.reflection.field_value(value: S, comptime name: Symbol) -> FieldType(S, name)`（仅限合法可见字段，不打破私有字段封装，除非 `derive fn` 位于允许观察主体内部布局的受信任策略边界）。
- **备选方案（方案 B）**：全面采用 CMTT 宏/AST 节点拼装（在 `derive fn` 中构造带静态 `expr.field_ident` AST 的方法体再降码），完全避免在类型系统层面引入动态 `Symbol` 投影原语，但会提高 `derive fn` 编写复杂度（需等 `05` `WL-05-26` 结构化 AST 落地）。

### OQ-03-06：`Clone` / `Default` 派生在携带运行期依赖类型（`quote .allocator`）结构体上的约束传递
- **来源引用**：[docs/type/runtime-dependent-type.md](../../docs/type/runtime-dependent-type.md)、`04` `WL-04-26`、`10` `WL-10-06`。
- **冲突描述**：当结构体 `Data<.A = A, .allocator = alloc>` 含有 `Vec<u32, .A = A, .allocator = alloc>` 字段时，`derive Clone` / `derive Default` 需要将外层结构体自身携带的运行期环境 `alloc`（而非隐式硬编码 `DefaultAllocator`）传递给内部字段的 `Default.default_value` 或 `Clone.clone`；同时如果前序字段 `clone()` 成功而后续字段发生错误或分配失败，部分构造成品需可靠释放。
- **推荐方案（方案 A）**：`derive` 引擎按普通 `impl` 签名继承宿主声明中携带的 `quote` 依赖环境到方法作用域；对 `Default` 等静态关联方法无法读取实例字段但拥有宿主类型子句上的 `quote .allocator` 参数，可直接把 `alloc` 传给字段默认构造器；复合结构体逐字段构造失败时的部分 `Drop` 交由编译器统一的局部初始化状态跟踪（`moves-and-initialization.md`）处理，不在策略源码手写裸指针释放。
- **备选方案（方案 B）**：初期仅允许无 `quote` 依赖参数的普通类型使用内置 `Default`/`Clone` 自动派生，含运行期依赖的类型要求手写 `impl` 或显式传参构造器。

### OQ-03-07：分配型容器（`Vec` 等）实现 `VariadicCollector` 时 `!AllocErr` 的失败传播机制
- **来源引用**：`library/std/collections_impl/vec.fl:138, 288`、`proposal_trace/2026-9-8-variadic-argument-collecting-rules.fl:11-22`、`07` `WL-07-06`。
- **冲突描述**：`VariadicCollector.variadic_collect(comptime S: Type, args: S) -> Tuple<Tmp, Self>` 返回不可失败结果，而 `std.collections.Vec.with_capacity` 与 `Vec.push` 返回 `!AllocErr`（遵守“不隐式假设堆分配永不失败”的原则）。若用户函数签名仅写 `fn log_all(...items: Vec<str, .A = A, .allocator = a>)`（未声明错误结果），`variadic_collect` 内部 OOM 只能调用 `std.process.panic`；若要显式向调用方传播 `!AllocErr`，需要可失败收集协议或异常路径契约。
- **推荐方案（方案 A）**：保持核心 `VariadicCollector` 为无分配错误的基础收集协议（优先推荐非堆失败路径使用 `Tuple`、`Array`、`Slice` 或调用方提供足够预留）；针对确实可能发生收集阶段错误的容器，补充平行协议 `TryVariadicCollector`（`assoc Err: Type; fn try_variadic_collect(...) -> !Err Tuple<Tmp, Self>`）并在被调函数返回 `!Err ...` 时自动以 `!` 向下传播至调用方准备计划（失败时照常逆序清理已创建的 `Tmp`）。
- **备选方案（方案 B）**：所有容器的 `variadic_collect` 统一规定 OOM 时直接调用 `std.process.panic("out of memory")`，将可能失败的收集限制为显式构造而非语法收集。
