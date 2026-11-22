# WL-02: 模式匹配协议（Pattern Views）、模式函数与取像（Take View）完整 Wish List 规格

> **文档角色**：本文档为 Flurry 编译器与六内置包（`builtin`、`meta`、`attrs`、`build`、`std`、`verification`）在**模式匹配视图协议（Pattern View Protocols）、依赖 `TuplePatternView`、类型模式、模式函数 / quoted arms、`BitVec` 位模式、以及完整 Take View（`expr ' id`）注册表**方面的长期通用设计与开发需求基线。
> **遵循原则**：严格以作者确认、现行正式 `docs/`、`AGENTS.md` 与 `docs_chat_history/` 已确认结论为语义权威；不迁就当前编译器局部缺口削足适履，不引入 `Option`/`Result`、不使用 `::` 路径、不把 `comptime` 指针或旧式 `^comptime_phase` / `_comptime` 后缀带入新规格。

---

## 1. 范围、权威来源与归属包

### 1.1 范围与跨模块边界

本规格覆盖 Flurry 中**结构解构、模式扩展、模式匹配决策图及表达式取像（Take View）**的完整表面契约与库接口：

1. **序列与无限序列模式视图**：`ListPatternView`、`StableListPatternView`、`ComptimeListPatternView`、`ComptimeStableListPatternView`，以及 `ListPatternViewEndless`、`StableListPatternViewEndless`、`ComptimeListPatternViewEndless`、`ComptimeStableListPatternViewEndless`。
2. **映射/键值记录模式视图**：`MapPatternView`、`StableMapPatternView`、`ComptimeMapPatternView`、`ComptimeStableMapPatternView`。
3. **区间模式视图**：`RangeEnd<T>`、`RangePatternView`、`StableRangePatternView`、`ComptimeRangePatternView`、`ComptimeStableRangePatternView`（及与 `Ord` / 标量区间、区间穷尽性的衔接）。
4. **拼接模式视图**：`ConcatPatternView`、`StableConcatPatternView`、`ComptimeConcatPatternView`、`ComptimeStableConcatPatternView`（及与 `BitVec<N>` 定宽位拼接的边界划分）。
5. **编译时驱动依赖元组模式视图（CT `TuplePatternView`）与 CMTT 协议对象**：`CodeBinding<T>`、`ExprCode`、`PatternShape`、`PatternInput`、`TuplePatternPrefix<Self>`、`TuplePatternStart<P>`、`TuplePatternSlot<P>`、`MatchPreparation`、`code_sequence`、`code_match`、`TuplePatternView`（含 `ComptimeTuplePatternView` 分析），以及含 `RegexMatcher` 在内的三个完整端到端示例与负例。
6. **结构体剩余匹配、类型模式与模式函数**：`StructPatternView` / `{ x, .. }` 剩余模式、`TypePatternView` 与类型 `'id` / 依赖 normal-form 模式、`case` 模式视图与函数级 `case` 开放分发、`quote fn(case ...)` 与 `use template` quoted arms。
7. **`BitVec` 位段模式与跨元素表示**：`0b`/`0o`/`0x` 段、动态宽度 `<expr>` / `BitVec<0>`、枚举显式表示属性的 `__flurry_*` 演进迁移。
8. **完整 Take View（`expr ' id`）注册表**：清点编译器前端/HIR 已接通的 10 类取像（含内部 `'builtin`/`'intrinsic` 及尚未完成 elaboration 的 `'multi_deref`），并逐一裁定候选新取像（`'variant_index`、`'discriminant`、`'as_bytes`、`'snapshot`、`'field_names`、`'dst_deref`）应进入 `expr ' id` 语法还是留在 `Type.*` / `meta.*` / `builtin.mem`。

与其余 Wish List 文档的明确分工如下：

- **WL-01（语法协议与操作符）**：拥有 6 个 `Range*` 类型族、`RangeBounds`、`Concat` 表达式协议、`Eq` / `Ord` / `Index` / `Splice` / `LiteralInto` 核心操作符定义；WL-02 消费其中的 `Ord`、`Eq`、`Splice` 与 `Range*` 类型，定义模式侧的 `RangePatternView` 与 `ConcatPatternView`。
- **WL-04（`builtin` 基础类型与数学）**：拥有 `Tuple`、`BitVec<n>`、`str`、`Symbol`、`Type`、`Slice`（若留在 `std.collections` 则由 WL-07 拥有）本体；WL-02 定义它们上的模式视图协议与实现要求。
- **WL-05（`meta` CMTT 与反射）**：拥有核心 `< Ψ |- T >` / `< T -| Ψ >` 通用报价与类型反射 API（`Type.struct_fields`、`Type.enum_*` 等）；WL-02 定义服务于模式视图与模板 arms 的 `CodeBinding` / `ExprCode` / `PatternInput` / `MatchPreparation` / `ArmPreparation` 及 Take View 与 `Type.*` 反射的边界。
- **WL-06（`attrs` 属性规范）**：拥有官方 `__flurry_*` 键注册；WL-02 提出模式模板与布局属性键需求（`__flurry_quote_fn_for_match_arms` 等）。
- **WL-07（`std` 集合与文本）**：拥有 `Vec`、`VecDeque`、`HashMap`、`String` 等标准容器（严格维持六内置包精简边界，**不包含 `regex` 等 Tier 2/3 生态库**）；WL-02 定义 `Vec` / `VecDeque` / `HashMap` / `String` 须实现的 `ListPatternView`、`MapPatternView`、`ConcatPatternView` 契约，而 `regex`（`RegexMatcher`）仅作为 Tier 2/3 用户生态示例展现 `TuplePatternView` 消费方式。

### 1.2 权威语义来源

- 协作与不可破坏边界：`AGENTS.md`
- 模式匹配语法、控制流、`case`、DAG 与穷尽性：`docs/control-flow/pattern-matching/index.md`
- 位向量与位段模式：`docs/control-flow/pattern-matching/bitvec.md`、`docs/program-synthesis/bitvec-algebra.md`
- 函数级模式与模式模板：`docs/functions/pattern-functions.md`、`docs/functions/inline-and-quote.md`、`docs/cmtt/quote-functions-and-templates.md`
- 模式代码与 quoted arms：`docs/cmtt/patterns-and-arms.md`、`docs/cmtt/index.md`、`docs/cmtt/any-and-code-evidence.md`、`docs/cmtt/quotation-and-stages.md`、`docs/cmtt/contexts-and-substitution.md`
- 类型模式与命题约束：`docs/type/type-patterns.md`、`docs/type/propositional-constraints/atoms.md`、`docs/type/propositional-constraints/occurrence-typing.md`
- Take View 语义：`docs/others/view-taking.md`、`docs/compilation/syntax-and-hir.md`、`docs/comptime/attribute.md`、`docs/type/user-defined-types/enums.md`、`docs/resource-management/contracts-and-verification.md`、`docs/resource-management/runtime-purity.md`
- 双阶段 Trait 规范：`docs/comptime/the-comptime-phases-of-traits.md`、`docs/comptime/official-attribute-keys.md`
- 词法与文法：`docs/grammar/index.md`、`docs/grammar/syntax`、`docs/traits/protocols.md`
- 已确认历史讨论：`docs_chat_history/013-control-flow-07.md`、`046-trait-implementation-02.md`、`053-resource-management-04.md`、`055-resource-management-06.md`

> 注：依据仓库时效口径，仅有近 6 个月内（`2026-5` 至 `2026-10`）的 `proposal_trace/` 草案（`2026-9-7-pattern-traits.fl`、`2026-9-19-regex.fl`、`2026-9-19-regex-history.md`、`2026-6-11-first-class-pattern-reconsideration.fl`、`2026-6-6-function-level-pattern-matching.fl`）保留背景参考价值，且一切细节仍以正式 `docs/` 与 `AGENTS.md` 为唯一准则；更早期历史草案已过期且不再引用。本文已将上述草稿中的旧式 `^comptime_phase`、`_comptime` 方法后缀、`*self` 编译时指针、`extend` 语法及旧错误处理全部按现行正式规范现代化。

### 1.3 归属包分层布局

| 层级 / 包 | 模块路径 | 归属内容 |
|---|---|---|
| Runtime 协议层（`builtin`） | `builtin.protocols.patterns`（re-export 至 `builtin` 与 `builtin.prelude`） | `ListPatternView`、`StableListPatternView`、`ListPatternViewEndless`、`StableListPatternViewEndless`、`MapPatternView`、`StableMapPatternView`、`RangeEnd<T>`、`RangePatternView`、`StableRangePatternView`、`ConcatPatternView`、`StableConcatPatternView`、`StructPatternView`、`StableStructPatternView` |
| Comptime 协议与 CMTT 元编程协议（`meta`） | `meta.data.patterns` / `meta.patterns`（re-export 至 `meta` 与 `meta.prelude`） | `ComptimeListPatternView`、`ComptimeStableListPatternView`、`ComptimeListPatternViewEndless`、`ComptimeStableListPatternViewEndless`、`ComptimeMapPatternView`、`ComptimeStableMapPatternView`、`ComptimeRangePatternView`、`ComptimeStableRangePatternView`、`ComptimeConcatPatternView`、`ComptimeStableConcatPatternView`、`ComptimeStructPatternView`、`ComptimeStableStructPatternView`；CMTT 元编程对象 `CodeBinding<T>`、`ExprCode`、`PatternShape`、`PatternInput`、`TuplePatternPrefix<Self>`、`TuplePatternStart<P>`、`TuplePatternSlot<P>`、`MatchPreparation`、`ArmPreparation`、`code_sequence`、`code_match`、`TuplePatternView`、`TypePatternView` |
| 属性层（`attrs`） | `attrs.quote_fn_template`、`attrs.layout` | `for_match_arms`、`allow_non_exhaustive_instance`（以及后续配合固定宽度 enum 表示的 `__flurry_*` 布局键） |
| Tier 1：标准容器实现（`std`） | `std.collections`、`std.string` | `Slice<T>`、`Vec<T>`、`VecDeque<T>`、`HashMap<K, V>`、`String` 实现对应视图协议（严禁把 `regex` 等生态库塞入 `std`） |
| Tier 2/3：生态消费示例（非六内置包） | 外部生态包（如 `user_regex`） | `RegexMatcher`（消费 `meta.patterns.TuplePatternView` 与 `builtin.ConcatPatternView` 的典型生态用户示范） |

> **包依赖方向提醒**：当前六包依赖单向为 `builtin` -> `meta` -> `attrs` -> `build` -> `std` -> `verification`。因此将使用 `ExprCode`、`PatternInput`、`MatchPreparation` 与 CMTT `< Ψ |- T >` 报价的 `TuplePatternView` 与 `TypePatternView` 放置在 `meta.patterns`（并通过 `language-items.conf` 直接绑定 declaration identity），可完全避免 `builtin` 反向依赖 `meta`；详见第 3.5 节与第 5 节 Open Question 1。

---

## 2. 现状与缺口全景总表

| ID | 符号 / 特性 | 归属包路径 | 阶段 | 当前现状（基于仓库源码核实） | 优先级 | 核心编译器依赖 |
|---|---|---|---|---|---|---|
| `WL-02-01` | `ListPatternView`、`StableListPatternView` 与默认 `rest` 清理 | `builtin.protocols.patterns` | RT | 已存在且已接入 `Slice<T>`/`Vec<T>`/`pattern_automata`；但默认 `list_pattern_view_rest` 体为 `while true {}`，且定长数组 `Array<T, N>` 尚无统一 `ListPatternView` 实现 | P2 | `elaboration::list_patterns`、定长数组布局与元素投影 |
| `WL-02-02` | `ComptimeListPatternView` 对齐 `assoc Rest` 与 `ComptimeStableListPatternView` | `meta.data.patterns` | CT | `ComptimeListPatternView` 已接通 `meta.List<T>`，但缺少 `assoc Rest: Type = Self;` 与 `ComptimeStableListPatternView` 标记 | P1 | `elaboration::list_patterns::ComptimeListProtocol` 解析 `assoc Rest` 与标记校验 |
| `WL-02-03` | `ListPatternViewEndless`、`StableListPatternViewEndless` 及其 `Comptime*` 双相孪生 | `builtin.protocols.patterns` / `meta.data.patterns` | RT + CT | 尚未定义，编译器仅支持有限长度 `ListPatternView`；无限流匹配缺乏强制 `..._`/`...rest` 尾缀校验 | P2 | `PatternKind::List` 分发拓展、与 `ListPatternView` 互斥歧义检查、DAG 单次观察 |
| `WL-02-04` | `VecDeque<T, .A, .allocator>` 的 `ListPatternView` / `StableListPatternView` | `std.collections` | RT | `VecDeque` 为环形缓冲区，目前未实现 `ListPatternView` / `StableListPatternView`（无法零拷贝返回连续 `Slice<T>`） | P2 | 需要 `VecDeque` 子视图 `VecDequeSlice<T>` 或 `Rest` 关联类型解耦 |
| `WL-02-05` | `MapPatternView`、`StableMapPatternView`（含依赖 `assoc FieldType`） | `builtin.protocols.patterns` | RT | 尚未定义；`elaboration::pattern_checking::records` 明确报错 `"record pattern requires a struct type; custom pattern view elaboration is not implemented"` | P1 | 记录模式解析、依赖关联函数 `FieldType(.id)` 求值、`MapView` 单次观察缓存 |
| `WL-02-06` | `ComptimeMapPatternView`、`ComptimeStableMapPatternView` 及 `Object`/`Map` 实现 | `meta.data.patterns` / `meta.data.{object, map}` | CT | 尚未定义；当前 CT 下记录模式仅支持普通 `struct` 与匿名记录 | P1 | CT `MapPatternView` 相位选择与 `meta.Object` 类型域取证 |
| `WL-02-07` | `std.collections.HashMap`（`Symbol` / `str` / `String` 键）实现 `MapPatternView` | `std.collections` | RT | 尚未实现 | P2 | `MapPatternView` 上线与 `Symbol.tag_name` / `str` 比较 |
| `WL-02-08` | `RangeEnd<T>`、`RangePatternView`、`StableRangePatternView` | `builtin.protocols.patterns` | RT | 尚未定义；`pattern_checking/mod.rs` 仅硬编码标量边界与 `Ord.lt`/`Ord.le` 回退，无独立范围成员/区间穷尽协议 | P1 | 5 种范围形状统一降码到 `(RangeEnd<Bound>, RangeEnd<Bound>)` 与 `Ord` 回退优先级 |
| `WL-02-09` | `ComptimeRangePatternView`、`ComptimeStableRangePatternView` 与离散区间穷尽性 | `meta.data.patterns` | CT | 尚未定义；`pattern_automata` 仅内置整数/字符/`bool` 区间格 | P2 | CT `RangePatternView` 相位路由与自定义离散增量/前驱格检查 |
| `WL-02-10` | `ConcatPatternView`、`StableConcatPatternView`（`++` 前缀/后缀解构） | `builtin.protocols.patterns` | RT | 尚未定义；`parse::operators::get_pattern_op_info` 中无 `++`，`hir::PatternKind` 无 concat 节点 | P1 | 模式操作符 `++` 解析、前缀/后缀判定与歧义拒绝、`PatternKind::Concat` |
| `WL-02-11` | `ComptimeConcatPatternView`、`ComptimeStableConcatPatternView` 及 `meta.List`/`str` 实现 | `meta.data.patterns` | CT | 尚未定义 | P1 | CT concat 求值与字符边界合法性检查 |
| `WL-02-12` | `str`、`String`、`Slice<u8>` 运行时 `ConcatPatternView` / `StableConcatPatternView` | `builtin` / `std.string` / `std.collections` | RT | 尚未实现；另 `builtin.Splice` 尚未为 `str` 实现 | P2 | `str` UTF-8 码点切片原语与 `Splice for str` 接通 |
| `WL-02-13` | `meta` 依赖元组协议对象（`CodeBinding<T>`、`ExprCode`、`PatternInput`、`TuplePatternPrefix`、`TuplePatternStart`、`TuplePatternSlot`、`MatchPreparation`、`code_sequence`、`code_match`） | `meta.patterns` | CT-驱动 RT | 尚未实现（仅散见于 `proposal_trace` 草案）；缺跨步骤存在上下文 `CodeBinding` 与卫生隔离支撑 | P0 | CMTT 局部 arm 上下文绑定生成、`CodeBinding` 逃逸检查、存在类型/代码载体 |
| `WL-02-14` | `TuplePatternView`（及 `ComptimeTuplePatternView` 评估） | `meta.patterns` | CT-驱动 RT（+CT 评估） | 尚未定义；`pattern_checking/mod.rs` 明确报错 `"tuple pattern requires a tuple type; custom pattern view elaboration is not implemented"` | P0 | 元组模式多阶段 CT 协议驱动器、回溯分支点与 NIR 事务合并、资源/Drop 静态复用检查 |
| `WL-02-15` | Tier 2/3 生态正则模式示例（`RegexErr`、`RegexMatcher`、`compile_regex` 与 `TuplePatternView` 实现） | Tier 2/3 生态包（如 `user_regex`，**非 `builtin`/`std`**） | CT 编 + RT 跑 | 尚未实现（作为核心协议消费者设计示例） | P2 | `WL-02-13`/`WL-02-14`、常量数组/`Splice for str` 接通 |
| `WL-02-16` | 记录/结构体剩余模式 `{ field, .. }` 与 `StructPatternView` / `ComptimeStructPatternView` | `builtin.protocols.patterns` / `meta.data.patterns` | RT + CT | `records.rs` 明确拒绝 `"record rest binding is not implemented"`；具名 `struct` 不支持自定义结构视图 | P1 | `hir::PatternKind::Record` `rest` 标志支持；`StructPatternView` 关联 schema 与命名绑定 |
| `WL-02-17` | `PatternKind::Ref`、`Async`、`Comptime` 遗漏降码闭合 | `compiler/elaboration` | RT + CT | AST->HIR 已构造 `PatternKind::{Ref, Async, Comptime}`，但 `pattern_checking/mod.rs` 落入 `_ => Err("not yet implemented")` | P1 | 引用模式 `ref pat` 借用/解引用语义、`comptime`/`async` 模式约束补全 |
| `WL-02-18` | 依赖 normal-form 值模式 & `TypePatternView`（`Type` 上 `'id` 与自定义高阶头模式） | `meta.patterns` / `compiler/elaboration` | CT + Type | `Type` 仅内置 `'id`、tuple `(p, q)` 与简单 NF 头应用；`pattern_checking/mod.rs` 明确拒绝依赖值参数 NF 模式 | P0 | 上下文反向依赖求值器（Contextual NF Pattern Elaboration）、`TypePatternView` 协议 |
| `WL-02-19` | `case` 视图增强（额外参数、纯度/`Stable` 层次、DAG 集成、谓词 `case -> bool` 裁决） | 语言特性 + `pattern_automata` | RT + CT | `cases.rs` 仅支持零额外参数 `*self`/`self -> ?Tuple<...>`，且走 `Pattern::Observe`/`Then` 事务回溯，不进入 `DecisionDag` | P1 | `case` 额外形参解析/传递、`Stable` 标记或纯度推导、`DecisionDag` 观察节点共享 |
| `WL-02-20` | 函数级模式定义（`fn foo(x: T); fn foo(case .lit(n))` 包封闭开放函数集合） | 语言特性 | RT + CT | 尚未实现（`parse`/`hir` 无 `case` 顶层多臂聚合） | P2 | 同包开放 `case` 函数臂收集、顺序无关互斥覆盖检查、统一分发体合成 |
| `WL-02-21` | `quote fn(case ...)`、quoted arms（`ArmPreparation`）与 `use` 模板 | `meta.patterns` / `attrs.quote_fn_template` | CT-驱动 RT | `library/attrs/quote_fn_template/README.md` 标记 `"待实现"`；`official-attribute-keys.md` 未注册相关 `__flurry_*` 键 | P1 | `Arm(Γ; T; Ψ; U)` 双向校验、`match { use ... }` 模板展开与增量覆盖率检查 |
| `WL-02-22` | `BitVec` 模式增强（`BitVec` `++`、动态宽度/`BitVec<0>`、枚举 `__flurry_*` 布局属性迁移） | `builtin` / `attrs.layout` | RT + CT | 静态定宽段 `0b`/`0o`/`0x`/`<expr>`/`(p: T)` 已支持；`BitVec` `++` 分裂与动态宽度切片未接通；`enums.md` 仍用 `.base_type`/`.discrete_tag`/`.dst` | P1 | `BitVec` 宽度代数分裂与运行时切片界检查、`attrs.layout` 对 enum 位布局统一收口 |
| `WL-02-23` | 既有 Take View 文档对齐与属性完善（`'type`、`'tag_name`、`'old`、`'bitvec`、`'tuple_indexed`、`'attribute`） | 核心语法 | RT / CT | 全部已接通；但 `docs/others/view-taking.md` 遗漏 `'attribute`；`attrs` 键缺统一文档索引 | P1 | 文档更新；`'type` 运行时依赖存在类型查询统一 |
| `WL-02-24` | `'tuple_enumerate` 在 CT `inline for` 混合 runtime 值中的缺口 | 核心语法 | CT / Inline | `views.rs:194` 在某些 `inline for` + CT/RT 混合元组展开路径仍有边界限制 | P1 | `expression_values::views` 元组与 `Code`/运行时混合降码修复 |
| `WL-02-25` | `'multi_deref` 取像 elaboration 接通 | 核心语法 | RT | 解析器与 AST->HIR 已构造 `ExprKind::MultiDeref`，但 `elaboration::expression_values` 完全未处理直接报错 | P1 | `expression_values/{mod,views,places}.rs` 实现多级解引用语义界定与降码 |
| `WL-02-26` | 候选新 Take View 评估（`'variant_index`/`'discriminant`、`'as_bytes`、`'snapshot`、`'field_names`、`'dst_deref`） | 核心语法 vs `Type.*`/`builtin.mem` | RT / CT | 尚未实现（需界定语法扩展准入边界）；`Type.struct_field_names` 等已在 `meta.reflection` 提供 | P3 | 遵循最小语法污染原则，决定纳入 Take View 或路由至方法/反射 |

---

## 3. 详细规格设计

> 本节中**所有新建或修订协议**均遵循 [the-comptime-phases-of-traits.md](file:///home/fin/luna/docs/comptime/the-comptime-phases-of-traits.md) 的双阶段规范：运行时能力定义在普通 `trait`，编译时能力定义在独立 `pure comptime trait Comptime*`（两者同名方法和同名关联项，`comptime` 侧使用 `self` 值引用语义、零 `*self`/`.ref`/`.*`）。

### 3.1 序列与无限序列模式视图（List & Endless List Pattern Views）

#### 3.1.1 问题与对历史草案的修正

1. `library/builtin/protocols/patterns.fl` 当前默认方法体 `fn list_pattern_view_rest(*self, comptime start: usize) -> Rest { while true {} }` 依赖死循环占位，缺乏静态不可达语义或拆分为可选扩展 trait。
2. `library/meta/data/patterns.fl` 中的 `ComptimeListPatternView` 漏写了 `assoc Rest: Type = Self;`，导致泛型 CT 代码无法抽象 rest 切片类型；同时缺少 `ComptimeStableListPatternView` 与 `meta.List<T>` 显式 `Rest = List<T>` 绑定。
3. `proposal_trace/2026-9-7-pattern-traits.fl` 对 `ListPatternViewEndless` 使用旧式 `^comptime_phase` 与 `_comptime` 后缀，未定义其与 `ListPatternView` 冲突检查及无限流对 `..._` / `...rest` 的强制约束。
4. `std.collections.VecDeque<T, .A, .allocator>` 为双端环形缓冲区，尚无 `ListPatternView` / `StableListPatternView` 实现。

#### 3.1.2 完整 Flurry 接口定义

```flurry
-- 模块：builtin.protocols.patterns
use primitives.{Type, bool, usize}

pub trait ListPatternView {
    assoc Element: Type;
    assoc Rest: Type = Self;

    fn list_pattern_view_get_len(*self) -> usize;
    fn list_pattern_view_get_element(*self, comptime index: usize) -> Element;

    fn list_pattern_view_rest(*self, comptime start: usize) -> Rest {
        assumes false;
        while true {}
    }
}

pub trait StableListPatternView :- ListPatternView {}

{- 无限序列（流、生成器、惰性序列）模式视图。
   不声明长度查询，因而匹配臂末尾必须显式包含 `..._` 或 `...rest`。 -}
pub trait ListPatternViewEndless {
    assoc Element: Type;
    assoc Rest: Type = Self;

    fn list_pattern_view_endless_get_element(*self, comptime index: usize) -> Element;

    fn list_pattern_view_endless_rest(*self, comptime start: usize) -> Rest {
        assumes false;
        while true {}
    }
}

pub trait StableListPatternViewEndless :- ListPatternViewEndless {}
```

> **设计说明（默认 `rest` 方法契约）**：`assumes false; while true {}` 保留了当前可编译性，但**编译器应升级契约**：如果一个 `match` 中任意臂使用了 `...rest`（绑定非通配剩余视图），则目标类型必须覆盖实现 `list_pattern_view_rest` / `list_pattern_view_endless_rest`（或者通过 `^attrs` / 编译器标记默认占位方法未实现，调用点报错），而非在运行时静默进入死循环；若仅使用 `..._` 忽略尾部，则无需调用且不强制实现 `rest`。见第 5 节 Open Question 2。

对应的编译时双胞胎接口（同方法名与关联项名、`self` 值引用语义）：

```flurry
-- 模块：meta.data.patterns
use bindings.{from_usize, to_usize}
use builtin.{Type, usize}
use data.list.List;

pub pure comptime trait ComptimeListPatternView {
    assoc Element: Type;
    assoc Rest: Type = Self;

    comptime fn list_pattern_view_get_len(self) -> usize;
    comptime fn list_pattern_view_get_element(self, index: usize) -> Element;

    comptime fn list_pattern_view_rest(self, start: usize) -> Rest {
        assumes false;
        self
    }
}

pub pure comptime trait ComptimeStableListPatternView :- ComptimeListPatternView {}

pub pure comptime trait ComptimeListPatternViewEndless {
    assoc Element: Type;
    assoc Rest: Type = Self;

    comptime fn list_pattern_view_endless_get_element(self, index: usize) -> Element;

    comptime fn list_pattern_view_endless_rest(self, start: usize) -> Rest {
        assumes false;
        self
    }
}

pub pure comptime trait ComptimeStableListPatternViewEndless :- ComptimeListPatternViewEndless {}

comptime impl ComptimeListPatternView for List<T> where T {
    assoc Element: Type = T;
    assoc Rest: Type = List<T>;

    comptime fn list_pattern_view_get_len(self) -> usize {
        to_usize(self.length())
    }

    comptime fn list_pattern_view_get_element(self, index: usize) -> T {
        let position = from_usize(index);
        self[position]
    }

    comptime fn list_pattern_view_rest(self, start: usize) -> List<T> {
        self.slice(from_usize(start), self.length()) match {
            rest? => rest,
            null => [],
        }
    }
}

comptime impl ComptimeStableListPatternView for List<T> where T {}
```

`std.collections.VecDeque` 环形视图与 `ListPatternView` 实现设计（供 WL-07 落地）：

```flurry
-- 模块：std.collections_impl.vec_deque（概念接口与行为契约）
pub struct VecDequeSlice
where
    T,
{
    first_part: Slice<T>,
    second_part: Slice<T>,
}

impl VecDequeSlice<T> where T :- Copy {
    pub fn len(*self) -> usize {
        self.first_part.len() + self.second_part.len()
    }

    pub fn get(*self, index: usize) -> ?T {
        let n1 = self.first_part.len();
        if index < n1 {
            return self.first_part.get(index);
        }
        self.second_part.get(index - n1)
    }
}

impl ListPatternView for VecDequeSlice<T> where T :- Copy {
    assoc Element: Type = T;
    assoc Rest: Type = VecDequeSlice<T>;

    fn list_pattern_view_get_len(*self) -> usize {
        self.len()
    }

    fn list_pattern_view_get_element(*self, comptime index: usize) -> T {
        self.get(index) match {
            v? => v,
            null => while true {}, -- 仅在 index < len 前提下由模式检查器调用
        }
    }

    fn list_pattern_view_rest(*self, comptime start: usize) -> VecDequeSlice<T> {
        let n1 = self.first_part.len();
        if start < n1 {
            VecDequeSlice<T> {
                first_part: self.first_part.subslice_from(start) match {
                    s? => s,
                    null => Slice<T>.empty(),
                },
                second_part: self.second_part.*,
            }
        } else {
            VecDequeSlice<T> {
                first_part: Slice<T>.empty(),
                second_part: self.second_part.subslice_from(start - n1) match {
                    s? => s,
                    null => Slice<T>.empty(),
                },
            }
        }
    }
}

impl StableListPatternView for VecDequeSlice<T> where T :- Copy {}
```

同理，`VecDeque<T, .A = A, .allocator = alloc>`（当 `T :- Copy`）实现 `ListPatternView` 与 `StableListPatternView`，其 `assoc Rest: Type = VecDequeSlice<T>;`，完全零堆分配即可通过 `[head, second, ...tail]` 匹配环形队列（包括折返场景）。

#### 3.1.3 语义、阶段、资源与报错契约

- **固定 vs 无限互斥规则**：若同一具体类型 `S` 同时实现了 `ListPatternView` 与 `ListPatternViewEndless`（或其 `Comptime*` 孪生），必须在语义检查期报编译错误 `"ambiguous list pattern view: type implements both ListPatternView and ListPatternViewEndless"`。
- **无限列表模式形状规则**：当主语仅实现 `ListPatternViewEndless`（或 `ComptimeListPatternViewEndless`）时，列表模式必须包含 `..._` 或 `...rest`（位于末尾）；封闭形状 `[a, b]` 或 `[]` 必须拒绝，防止对无限序列做不存在的长度检查。
- **稳定性（`Stable*`）与单次观察 DAG**：实现 `StableListPatternView` / `StableListPatternViewEndless`（或自身为纯函数方法）的类型承诺：在同一不可变观察期内，对同一 `index` 重复调用 `get_element` 或 `get_len` 语义幂等且无可见副作用，因而进入 `pattern_automata` 的单次观察懒缓存 `DecisionDag`；`LenInterval` 对无限序列永远视为 `[prefix_min, +inf)`，可由 `[_, _, ..._]` 与 `[a, b, ...rest]` 构成覆盖穷尽。
- **非 `Copy` 元素安全边界**：`ListPatternView` 按 `*self` 借用并按值返回 `Element`，因此当元素通过拷贝产生（如 `Slice<T>`、`Vec<T>`、`VecDeque<T>`）时要求 `T :- Copy`，避免借用视图偷走所有权或制造双次 `Drop`。

#### 3.1.4 `language-items.conf` 绑定扩展

```ini
list-pattern-protocol = builtin.protocols.patterns.ListPatternView
comptime-list-pattern-protocol = meta.data.patterns.ComptimeListPatternView
stable-list-pattern-protocol = builtin.protocols.patterns.StableListPatternView
comptime-stable-list-pattern-protocol = meta.data.patterns.ComptimeStableListPatternView
list-pattern-endless-protocol = builtin.protocols.patterns.ListPatternViewEndless
comptime-list-pattern-endless-protocol = meta.data.patterns.ComptimeListPatternViewEndless
stable-list-pattern-endless-protocol = builtin.protocols.patterns.StableListPatternViewEndless
comptime-stable-list-pattern-endless-protocol = meta.data.patterns.ComptimeStableListPatternViewEndless
```

#### 3.1.5 用法示例与反例

```flurry
-- 正例：无限斐波那契生成器（StableListPatternViewEndless）
struct FibStream {
    prev: u64,
    curr: u64,
}

impl ListPatternViewEndless for FibStream {
    assoc Element: Type = u64;
    assoc Rest: Type = FibStream;

    fn list_pattern_view_endless_get_element(*self, comptime index: usize) -> u64 {
        var a = self.prev.*;
        var b = self.curr.*;
        var i: usize = 0;
        while i < index {
            let next = a + b;
            a = b;
            b = next;
            i = i + 1;
        }
        a
    }

    fn list_pattern_view_endless_rest(*self, comptime start: usize) -> FibStream {
        var a = self.prev.*;
        var b = self.curr.*;
        var i: usize = 0;
        while i < start {
            let next = a + b;
            a = b;
            b = next;
            i = i + 1;
        }
        FibStream { prev: a, curr: b }
    }
}

impl StableListPatternViewEndless for FibStream {}

fn inspect_fib(s: FibStream) -> u64 {
    s match {
        [0, 1, third, ...tail] => third + tail.list_pattern_view_endless_get_element(0),
        [first, ..._] => first,
    }
}

-- 负例（编译期必须拒绝）：对 ListPatternViewEndless 使用封闭列表模式
{-
fn bad_endless(s: FibStream) -> u64 {
    s match {
        [a, b] => a + b, -- 错误：ListPatternViewEndless 不存在有限长度检查，缺少 `..._` / `...rest`
        _ => 0,
    }
}
-}
```

---

### 3.2 映射模式视图（MapPatternView / StableMapPatternView / ComptimeMapPatternView）

#### 3.2.1 问题与对历史草案的修正

1. `proposal_trace/2026-9-7-pattern-traits.fl` 使用了 `^comptime_phase` 与 `assoc FieldTypeComptime` / `map_pattern_view_get_comptime`，必须拆分为对称的 `MapPatternView` 与 `pure comptime trait ComptimeMapPatternView`。
2. 当前 `compiler/elaboration/src/program/pattern_checking/records.rs:48-55` 仅支持具名 `struct` 与匿名记录，既未接通 `MapPatternView` 自定义解构，也未接通记录剩余模式 `{ field, .. }`。
3. 需要精确界定**键不存在（`null`）**与**字段值为可空类型 `?V` 中的 `null`** 二者的嵌套语义（根据 `meta/data/list.fl:17-21`，Flurry 的 `??V` 严格保留外层与内层存在性，不会静默压平）。

#### 3.2.2 完整 Flurry 接口定义

```flurry
-- 模块：builtin.protocols.patterns
use primitives.{Symbol, Type, bool}

pub trait MapPatternView {
    assoc FieldType: pure comptime fn(Symbol) -> Type;

    fn map_pattern_view_get(*self, comptime property: Symbol) -> ?FieldType(property);
}

pub trait StableMapPatternView :- MapPatternView {}
```

```flurry
-- 模块：meta.data.patterns
use builtin.{Integer, Symbol, Type, bool}
use data.map.Map;
use data.object.Object;
use bindings.{CompilerObjectEntryType, compiler_object_get}

pub pure comptime trait ComptimeMapPatternView {
    assoc FieldType: pure comptime fn(Symbol) -> Type;

    comptime fn map_pattern_view_get(self, property: Symbol) -> ?FieldType(property);
}

pub pure comptime trait ComptimeStableMapPatternView :- ComptimeMapPatternView {}

pub pure comptime fn ConstantFieldType(_property: Symbol) -> Type
where
    V,
{
    V
}

comptime impl ComptimeMapPatternView for Map<Symbol, V> where V {
    assoc FieldType: pure comptime fn(Symbol) -> Type = ConstantFieldType<V>;

    comptime fn map_pattern_view_get(self, property: Symbol) -> ?V {
        self.get(property)
    }
}

comptime impl ComptimeStableMapPatternView for Map<Symbol, V> where V {}
```

#### 3.2.3 语义规则、阶段策略与资源契约

1. **记录模式到 `MapPatternView` 的降码语义**：
   当主语 `S` 非具名 `struct` 且非编译器已知固定匿名记录类型时，模式 `{ k1: pat1, k2: pat2, .. }` 按如下规则展开：
   - 对每个字段 `k_i`（转为编译时 `Symbol` 字面量 `.k_i`），先求值类型 `T_i = S.as(MapPatternView).FieldType(.k_i)`（或 comptime 下 `ComptimeMapPatternView`）；
   - 调用 `subject.map_pattern_view_get(.k_i)` 取得 `?T_i`；
   - 若返回外层 `null`（即键 `.k_i` 在映射中不存在），**该候选臂失败**，控制流转向下一臂（不会报错或 panic）；
   - 若返回外层 `val?`（其中 `val: T_i`），再用子模式 `pat_i` 匹配解包后的 `val`；若 `T_i` 本身为 `?U`，则模式 `{ k1: null }` 匹配“键存在且其值为 `null`”，而模式 `{ k1: inner? }` 匹配“键存在且其值非空”；
   - 若模式简写为 `{ k1, .. }`，等价于 `{ k1: k1, .. }`（绑定名 `k1` 类型为 `T_1`）。
2. **`..`（rest）在 `MapPatternView` 中的规则**：
   - 由于一般映射的键集合为运行时动态或开放域，匹配部分键时允许写 `{ a: 1, .. }`；省略 `..` 是否被视为“开放子集匹配”还是“拒绝多余键”取决于类型是否为开放容器：
     - **开放键值映射（无封闭字段集合）**：`{ a: x }` 默认允许省略 `..`（或警告/按开放记录处理；见 Open Question 3 对“显式 `..` 要求 vs 开放默认”的权衡，本规格推荐：**一般 `MapPatternView` 上写 `{ a: x, .. }` 恒合法；若省略 `..`，仅当类型系统或 schema 已知完整键集合时校验无未列键，否则拒绝并发出提示建议追加 `..` 或引入精确 `MapPatternView` 键集合查询**，避免与 `struct` 闭世界语义混淆！）。
3. **稳定性与穷尽性（`StableMapPatternView` / `ComptimeStableMapPatternView`）**：
   - 未实现 `StableMapPatternView` 时，多次访问同一键 `.k` 不可跨臂合并观察，且由于运行时动态键值对一般非穷尽，`match` 必须以通配臂 `_` 结尾。
   - 实现 `StableMapPatternView` 后，若编译器在前续臂已对键 `.k` 成功取得 `FieldType(.k)` 且缓存结果，同一观测链上的后续候选复用同一结果不再重复哈希/查找（单次观察 DAG）；在 `FieldType` 返回封闭静态字段且结合 `Type`/`Object` 键集的场景，可进一步判定字段组合覆盖度。
4. **`std.collections.HashMap` 适配说明**：
   - `HashMap<Symbol, V, .A, .allocator>` 可直接实现 `MapPatternView`（要求 `V :- Copy` 时拷贝返回 `?V`；若 `V` 非 `Copy` 则需先拥有 `*V` 指针视图或移动策略，当前 `*self` 借用模型下建议限制 `V :- Copy`）。
   - 对于 JSON/配置常见的字符串键映射 `HashMap<String, V, ...>` 或 `HashMap<str, V, ...>`，因 `property: Symbol` 为编译时已知，可用 `property'tag_name` 在编译时获得精确 ASCII/UTF-8 `str` 键名（零运行时 `Symbol` 表查找），轻松实现 `MapPatternView`！
5. **CT `meta.Object` 的特殊性**：
   - `meta.Object` 同时承载异构键值对（不同条目可拥有不同 `Type`）。通过 `MapPatternView` 匹配特定 `Object` **实例**时，若 `FieldType` 需依赖接收者值而非仅类型，静态 trait 签名 `assoc FieldType: pure comptime fn(Symbol) -> Type` 只能按 `Object` 类型统一返回 `meta.Any`（或者依赖编译器对 `Object` 值采用内置已知匿名记录结构直接解构，如现行 `Object` 反射路径）。这也是将 `FieldType` 限定在类型级、把值依赖记录匹配留给编译时值解构或 `PatternFunction` 的清晰分界。

#### 3.2.4 `language-items.conf` 绑定

```ini
map-pattern-protocol = builtin.protocols.patterns.MapPatternView
stable-map-pattern-protocol = builtin.protocols.patterns.StableMapPatternView
comptime-map-pattern-protocol = meta.data.patterns.ComptimeMapPatternView
comptime-stable-map-pattern-protocol = meta.data.patterns.ComptimeStableMapPatternView
```

#### 3.2.5 用法示例与反例

```flurry
-- 正例：动态 JSON-like 键值类型安全解构
enum Json {
    null_val,
    num(i64),
    flag(bool),
}

struct MiniObj {
    tag_type: ?i64,
    enabled: bool,
}

pure comptime fn MiniObjFieldType(property: Symbol) -> Type {
    property match {
        .code => i64,
        .active => bool,
        _ => NoReturn, -- 静态阻止访问未知 schema 字段
    }
}

impl MapPatternView for MiniObj {
    assoc FieldType: pure comptime fn(Symbol) -> Type = MiniObjFieldType;

    fn map_pattern_view_get(*self, comptime property: Symbol) -> ?FieldType(property) {
        property match {
            .code => self.tag_type.*,
            .active => self.enabled.*,
        }
    }
}

impl StableMapPatternView for MiniObj {}

fn decode(obj: MiniObj) -> i64 {
    obj match {
        { code: c, active: true, .. } => c,
        { active: false, .. } => 0,
        _ => -1,
    }
}

-- 负例（编译期拒绝）：查询不存在字段触发 NoReturn / 类型不可满足，或非法修改 comptime 接收者
{-
fn bad_field(obj: MiniObj) -> i64 {
    obj match {
        { unknown: x, .. } => 1, -- 期望返回 ?NoReturn，无法匹配绑定 `x` 或构造合法非空路径
        _ => 0,
    }
}
-}
```

---

### 3.3 区间模式视图（RangePatternView & ComptimeRangePatternView）

#### 3.3.1 问题与对历史草案的修正

`proposal_trace/2026-9-7-pattern-traits.fl` 将区间视图写作 `fn in_the_range(*self, comptime range: Range) -> bool`，存在三项硬伤：
1. Flurry 没有单一非泛型 `Range` 类型（WL-01 定义 `Range<Idx>`、`RangeInclusive<Idx>`、`RangeFrom<Idx>`、`RangeTo<Idx>`、`RangeToInclusive<Idx>`、`RangeFull`）；
2. [pattern-matching/index.md](file:///home/fin/luna/docs/control-flow/pattern-matching/index.md#L83-L96) 明确允许在区间模式边界中使用运行时表达式（例如 `start()..=end()`），强加 `comptime` 会破坏既有语义；
3. 当前编译器 `pattern_checking/mod.rs` 对内置数值/字符走原生边界比较、对自定义类型仅回退到 `Ord.lt`/`Ord.le`，无法让只具备“集合包含/分段成员判定”（如版本号范围、子网 CIDR 区间、时间区间、非全序自定义坐标）或想定制高效范围测试的类型接入区间模式。

#### 3.3.2 完整 Flurry 接口定义

采用统一边界描述枚举 `RangeEnd<T>`，用一个 trait 方法覆盖全部 5 种有界/半无界范围形式（`..hi`、`..=hi`、`lo..`、`lo..hi`、`lo..=hi`），彻底避免为 5 种 `Range*` 类型各写一遍 trait：

```flurry
-- 模块：builtin.protocols.patterns
use primitives.{Type, bool}

pub enum RangeEnd where T {
    unbounded,
    inclusive(T),
    exclusive(T),
}

pub trait RangePatternView {
    assoc Bound: Type = Self;

    fn range_pattern_view_contains(
        *self,
        start: RangeEnd<Bound>,
        end: RangeEnd<Bound>,
    ) -> bool;
}

pub trait StableRangePatternView :- RangePatternView {}
```

```flurry
-- 模块：meta.data.patterns
use builtin.{RangeEnd, Type, bool}

pub pure comptime trait ComptimeRangePatternView {
    assoc Bound: Type = Self;

    comptime fn range_pattern_view_contains(
        self,
        start: RangeEnd<Bound>,
        end: RangeEnd<Bound>,
    ) -> bool;
}

pub pure comptime trait ComptimeStableRangePatternView :- ComptimeRangePatternView {}
```

#### 3.3.3 分派优先级、双相与穷尽性契约

1. **解析与匹配分派顺序（清晰确定性优先级）**：
   - `..`（全范围模式）在语法与语义层仍等价于通配符逻辑，不调用 `RangePatternView`（除非置于嵌套视图且需要显式构造范围值时另议，按现行模式文法 `..` 仅在区间模式允许，见 `docs/grammar/syntax`）；
   - 对于非全无界范围（`lo..hi`、`lo..=hi`、`lo..`、`..hi`、`..=hi`）：
     1. **内置原生标量（`Integer`、定宽整数、`char`、IEEE 浮点）**：保持零抽象开销的原生常量折叠与决策格降码（且保持 IEEE `NaN` 不落入任何 `lo..=hi` 区间的既定语义）；
     2. **显式实现 `RangePatternView`（CT 下为 `ComptimeRangePatternView`）**：优先于通用 `Ord` 回退（支持自定义 `Bound` 类型，例如 `SemVer` 主主主版本号匹配 `Version` 或 `u32` 端口段匹配）；
     3. **回退 `Ord`（CT 下为 `ComptimeOrd`）**：对仅实现 `Ord`/`ComptimeOrd` 的普通全序类型，由编译器自动按 `>= start` 与 `< end`/`<= end` 展开。
2. **空白与空区间规则**：
   - 若 `start` 与 `end` 均为已知常量且根据 `Ord` 或整数/字符语义明显为空（如 `10..5` 或 `10..10`），编译器延续既有的不可达分支诊断 / 空区间拒绝规则（具体空区间提示归编译器 `pattern_automata`）。
3. **稳定性（`StableRangePatternView`）**：
   - 声明 `range_pattern_view_contains` 无可见副作用且满足单调区间律（若 `start2..end2 ⊆ start1..end1` 且 `x` 命中 `start2..end2` 则必命中 `start1..end1`）；实现此标记且使用编译时常量边界时，`match` 可参与 `DecisionDag` 去重并在类型声明为离散有界集合时推导穷尽性（若仅动态运行时区间边界或开放边界，仍要求 `_` 臂）。
4. **阶段切换**：
   - 运行时匹配解析 `RangePatternView`；编译时上下文优先查找 `ComptimeRangePatternView`（若缺失再检查全序 `ComptimeOrd` 回退，严禁在 CT 下调用运行时的 `RangePatternView`）。

#### 3.3.4 `language-items.conf` 绑定

```ini
range-end-type = builtin.protocols.patterns.RangeEnd
range-pattern-protocol = builtin.protocols.patterns.RangePatternView
stable-range-pattern-protocol = builtin.protocols.patterns.StableRangePatternView
comptime-range-pattern-protocol = meta.data.patterns.ComptimeRangePatternView
comptime-stable-range-pattern-protocol = meta.data.patterns.ComptimeStableRangePatternView
```

#### 3.3.5 用法示例与反例

```flurry
-- 正例：异构 Bound 的区间模式视图（用 u16 端口号范围匹配 Endpoint）
struct Endpoint {
    port: u16,
    tls: bool,
}

impl RangePatternView for Endpoint {
    assoc Bound: Type = u16;

    fn range_pattern_view_contains(
        *self,
        start: RangeEnd<u16>,
        end: RangeEnd<u16>,
    ) -> bool {
        let p = self.port.*;
        let lo_ok = start match {
            .unbounded => true,
            .inclusive(lo) => p >= lo,
            .exclusive(lo) => p > lo,
        }
        let hi_ok = end match {
            .unbounded => true,
            .inclusive(hi) => p <= hi,
            .exclusive(hi) => p < hi,
        }
        lo_ok and hi_ok
    }
}

impl StableRangePatternView for Endpoint {}

fn classify_endpoint(ep: Endpoint) -> u8 {
    ep match {
        0..=1023 => 1,
        1024..49152 => 2,
        49152.. => 3,
        _ => 0, -- 动态/未知端值或非静态完备边界时提供回退
    }
}

-- 负例（编译期拒绝）：Bound 类型与模式端点表达式类型不匹配
{-
fn bad_range(ep: Endpoint) -> u8 {
    ep match {
        "80".."443" => 1, -- 错误：Bound 为 u16，不能用 str 作为端点
        _ => 0,
    }
}
-}
```

---

### 3.4 拼接模式视图（ConcatPatternView & ComptimeConcatPatternView）

#### 3.4.1 问题与对历史草案的修正

1. `proposal_trace/2026-9-7-pattern-traits.fl` 中 `ConcatPatternView` 仅支持单一前缀函数、缺少 `postfix`、错误强制返回 `?Self`（导致 `String` 这种 `Once` 所有权容器无法零拷贝切片返回 `?str`），且同样混用旧式 `^comptime_phase` 与 `_comptime`。
2. 编译器当前完全没有解析模式层面的 `++` 操作符（`parse::operators::get_pattern_op_info` 未注册 `++`，`hir::PatternKind` 无拼接模式节点）。
3. **关键架构区分：`ConcatPatternView` vs `BitVec<N>` 的 `++` 模式**：
   - 对于序列类型（`str`、`String`、`Slice<u8>`、`Vec<T>`、`meta.List<T>`），切分前缀或后缀后的**剩余部分类型 `Rest` 与前缀长度无关**（都是 `str` / `Slice<T>` / `List<T>`）。
   - 对于 `BitVec<N>`，按 `(hi : BitVec<M>) ++ (lo : BitVec<N - M>)` 切分时，剩余宽度直接取决于左半部类型宽度 `M`——其 `Rest` 不是一个不依赖拆分点长度的单一 `assoc Rest: Type`！因此本节让 `ConcatPatternView` 专注等型/切片不变域的序列前缀后缀拆分，而 **`BitVec<N>` 的 `++` 位拼接模式由编译器内建位代数（第 3.7 节）直接处理**，两者语法统一为 `lhs ++ rhs`，语义互不冲突。

#### 3.4.2 完整 Flurry 接口定义

```flurry
-- 模块：builtin.protocols.patterns
use primitives.{Type, bool}

pub trait ConcatPatternView {
    assoc Part: Type;
    assoc Rest: Type = Self;

    fn concat_pattern_view_strip_prefix(*self, part: Part) -> ?Rest;
    fn concat_pattern_view_strip_suffix(*self, part: Part) -> ?Rest;
}

pub trait StableConcatPatternView :- ConcatPatternView {}
```

```flurry
-- 模块：meta.data.patterns
use builtin.{Integer, Type, bool, str}
use data.list.List;

pub pure comptime trait ComptimeConcatPatternView {
    assoc Part: Type;
    assoc Rest: Type = Self;

    comptime fn concat_pattern_view_strip_prefix(self, part: Part) -> ?Rest;
    comptime fn concat_pattern_view_strip_suffix(self, part: Part) -> ?Rest;
}

pub pure comptime trait ComptimeStableConcatPatternView :- ComptimeConcatPatternView {}
```

> 注：`part: Part` **有意不强加 `comptime`**，使得既可写字面量模式 `"http://" ++ rest`，也可写局部运行时前缀匹配 `expected_prefix ++ rest`（就像区间模式支持运行时边界一样）；同时编译器可以对字面量常量 `part` 做 Trie / Aho-Corasick / 编译时常量折算优化。

#### 3.4.3 语法拆分规则、阶段策略与文本边界安全

1. **左/右拆分识别与多段链式关联**：
   - `a ++ b` 中，`++` 在模式语法优先级上应当低于范围 `..`、高于 `or` 与 `as`（具体优先表须与 WL-01 表达式 `++` 对齐，推荐左结合或统一扁平切分序列）。
   - **唯一可切分形式（非 `BitVec` 序列）**：
     - **前缀拆分**：`part_expr ++ rest_pat`（左侧为表达式/字面量/常量绑定转 `Part`，对主语调用 `concat_pattern_view_strip_prefix`，所得 `Rest` 继续交给 `rest_pat` 匹配，可嵌套为 `"https://" ++ rest ++ ".html"`——先按前缀剥离 `"https://"`，再按后缀剥离 `".html"`，最后用最内层 `rest` 绑定剩余！）；
     - **后缀拆分**：`rest_pat ++ part_expr`（右侧为 `Part` 表达式，调用 `concat_pattern_view_strip_suffix`）；
     - **歧义形式必须拒绝**：在非定宽序列上同时出现未定边界的变量模式拆分（如 `head ++ tail`，其中 `head` 与 `tail` 均不是 `Part` 表达式、也不是带有明确静态长度的 `BitVec<K>`）在编译期直接拒绝：`"ambiguous concat pattern: cannot split sequence between two variable-length patterns without a fixed-width BitVec annotation"`。
2. **UTF-8 安全边界（`str` 与 `std.string.String`）**：
   - `str` 实现 `ConcatPatternView`（`Part = str`，`Rest = str`）与 `StableConcatPatternView`；
   - `std.string.String`（`Once` 所有权堆文本）实现 `ConcatPatternView`（`Part = str`，`Rest = str`）与 `StableConcatPatternView`，**按 `*self` 借用并返回 `str` 视图切片而不消费或分配 `String`**！
   - 底层字节比对前，由于 `part: str` 本身是合法 UTF-8，只要字节前缀或字节后缀与 `part` 完全相等，其切片终点（或起点）必然精确落在 UTF-8 码点边界，绝不可能切碎多字节码点！
3. **`Slice<T>` 与 `meta.List<T>` 实现**：
   - `Slice<T>`（`T :- Copy + Eq`）：`Part = Slice<T>`，`Rest = Slice<T>`；
   - `meta.List<T>`（`T :- ComptimeEq`）：实现 `ComptimeConcatPatternView` 与 `ComptimeStableConcatPatternView`，`Part = List<T>`，`Rest = List<T>`。
4. **生命周期与逃逸**：
   - 对 `String`（或临时构造的非 `Static` 文本）调用 `concat_pattern_view_strip_prefix` 返回的 `Rest = str` 受借用生存期/区域分析约束（不能比被借用的 `String` 活得更久），沿用 Flurry 标准视图生命周期规则。

#### 3.4.4 `language-items.conf` 绑定

```ini
concat-pattern-protocol = builtin.protocols.patterns.ConcatPatternView
stable-concat-pattern-protocol = builtin.protocols.patterns.StableConcatPatternView
comptime-concat-pattern-protocol = meta.data.patterns.ComptimeConcatPatternView
comptime-stable-concat-pattern-protocol = meta.data.patterns.ComptimeStableConcatPatternView
```

#### 3.4.5 用法示例与反例

```flurry
-- 正例：链式前缀与后缀解构 HTTP 路径（StableConcatPatternView）
fn route(path: str) -> str {
    path match {
        "/api/v1/" ++ endpoint ++ ".json" => endpoint,
        "/static/" ++ asset => asset,
        _ => "",
    }
}

-- 正例：在 comptime 下对 List<u8> 使用 ComptimeConcatPatternView
comptime fn strip_magic(bytes: meta.List<u8>) -> ?meta.List<u8> {
    bytes match {
        [0x7f, 0x45, 0x4c, 0x46] ++ payload => payload,
        _ => null,
    }
}

-- 负例（编译期拒绝）：双方均为未知长度模式绑定
{-
fn bad_concat(s: str) -> bool {
    s match {
        left ++ right => true, -- 错误：str 不是定宽拆分，无法确定 left/right 分界点
    }
}
-}
```

---
### 3.5 编译时驱动的依赖元组模式视图（CT `TuplePatternView`）与 CMTT 协议对象

#### 3.5.1 架构定位与核心契约

与 `ListPatternView`、`MapPatternView` 等**在运行时直接执行固定签名方法**的视图不同，异构元组 `(p0, p1, ..., pk)` 的元素数量、每个槽位的类型 `T_i` 甚至后续槽位类型对前序槽位运行时值的依赖关系（例如 `(storage, NodeIndex<storage>)` 或正则捕获组 `("(a)(b)?", first, second)`）都无法由静态单态运行时方法签名表达。

因此，`TuplePatternView` 是一个**编译时元编程（CT-driven RT）协议**：
- 它的 3 个协议方法均是 `comptime fn`，但属于普通 `trait TuplePatternView`（位于 `meta.patterns`，**不使用**过时的 `^comptime_phase`）；
- 在模式 elaboration 期，编译器于编译时执行 `tuple_pattern_view_begin` / `tuple_pattern_view_next` / `tuple_pattern_view_finish`，通过 `MatchPreparation` 向当前匹配臂注入生成后的常规 runtime 提取代码与判断 guard；
- 生成的 runtime 代码**不引入单独的所有权提交钩子**（遵循 `docs_chat_history/053-resource-management-04.md` 与 `055-resource-management-06.md` 已确认结论）：生成的绑定、借用、移动与分支回退随后直接接受 Flurry 标准类型、仿射 `Once` 与 `Drop` 静态检查！

> **为什么使用 `meta.patterns`**：`TuplePatternView` 签名直接依赖 `CodeBinding<T>`、`ExprCode`、`PatternInput`、`MatchPreparation` 等 CMTT 元编程载体（`meta.code` / `meta.patterns`）。将其置于 `meta.patterns` 并通过 `language-items.conf` 绑定声明身份，使 `builtin -> meta -> attrs -> build -> std -> verification` 保持严格有向无环；第 5 节 Open Question 1 进一步评估是否在 `builtin` 重导出别名。

#### 3.5.2 完整协议对象与 `TuplePatternView` 定义

```flurry
-- 模块：meta.patterns
use bindings;
use builtin.{Any, Integer, NoReturn, Symbol, Type, bool, str, usize, void}
use data.integer.{from_usize, to_usize}
use data.list.List;

-- 协议错误类型（遵循 PascalCaseErr 与 PascalCase 变体规范）
pub enum PatternViewErr {
    InvalidArity,
    ExpectedConstantSlot,
    InvalidPatternInput,
    TypeMismatch,
    UnsupportedSlotShape,
    DisallowedContextEscape,
}

{- 模式槽位在编译期静态可见的语法形状摘要。 -}
pub enum PatternShape {
    wildcard,
    binding,
    constant,
    tuple,
    record,
    list,
    typed_binding,
    complex, -- 例如槽位内部包含 `or`、guard 或自定义子视图
}

{- 由编译器为每个生成臂分配的卫生的局部运行时绑定句柄。
   它仅代表存在于当前 MatchPreparation 上下文帧 Γ_match 中的局部绑定，
   禁止逃逸到其它 MatchPreparation 或外层非当前臂代码中。 -}
pub struct CodeBinding
where
    T,
{
    -- 编译器内部持有帧身份与局部绑定符号；不直接向用户暴露原始指针或裸 AST
    raw_handle: bindings.RawCodeBindingHandle,
}

impl CodeBinding<T> where T {
    pub pure comptime fn BindingType(self) -> Type {
        T
    }

    {- 将当前匹配帧 Γ_match 内的运行时绑定直接包装为帧作用域内的 ExprCode。
       注意：由于绑定身份属于动态展开帧 Γ_match（非空上下文 `|-`），这里不伪造 `< |- T >`
       闭合签名，而是保持仅在当前 MatchPreparation 帧内合法的开放上下文见证。 -}
    pub pure comptime fn as_erased(self) -> ExprCode {
        ExprCode {
            target_type: T,
            raw_handle: bindings.expr_code_from_binding(T, self.raw_handle),
        }
    }
}

{- 带有显式类型见证与帧作用域开放上下文的表达式代码包：(T: Type, < Γ_match |- T >)。
   支持从闭合代码 `< |- T >` 提升、从 `CodeBinding<T>` 引用，或通过单/双端口具名 CMTT
   上下文模板 `< a: A |- R >` / `< a: A, b: B |- R >` 与 `Γ_match` 内已有 ExprCode 组合，
   严格遵守 Flurry CMTT 单洞规则（无需在单个 `'(` 内写入多洞 `~<...>`）。 -}
pub struct ExprCode {
    target_type: Type,
    raw_handle: bindings.RawExprCodeHandle,
}

impl ExprCode {
    pub pure comptime fn ExprType(self) -> Type {
        self.target_type
    }

    pub pure comptime fn from_closed(code: < |- T >) -> ExprCode
    where
        T: Type,
    {
        ExprCode {
            target_type: T,
            raw_handle: bindings.expr_code_from_closed(T, code),
        }
    }

    pub pure comptime fn try_unwrap_closed(self) -> ?< |- T >
    where
        T: Type,
    {
        if self.target_type == T {
            bindings.expr_code_try_to_closed(T, self.raw_handle)
        } else {
            null
        }
    }

    {- 将单端口 CMTT 开放模板 `< p0: A |- R >` 的虚拟端口绑定到帧内参数 `arg0`（类型匹配 `A`）。 -}
    pub pure comptime fn apply_1(arg0: ExprCode, body: < p0: A |- R >) -> !PatternViewErr ExprCode
    where
        A: Type,
        R: Type,
    {
        if arg0.target_type != A {
            return error PatternViewErr.TypeMismatch;
        }
        ExprCode {
            target_type: R,
            raw_handle: bindings.expr_code_apply_1(A, R, arg0.raw_handle, body),
        }
    }

    {- 将双端口 CMTT 开放模板 `< p0: A, p1: B |- R >` 按序位置端口接入帧内 `arg0` 与 `arg1`。 -}
    pub pure comptime fn apply_2(
        arg0: ExprCode,
        arg1: ExprCode,
        body: < p0: A, p1: B |- R >,
    ) -> !PatternViewErr ExprCode
    where
        A: Type,
        B: Type,
        R: Type,
    {
        if arg0.target_type != A or arg1.target_type != B {
            return error PatternViewErr.TypeMismatch;
        }
        ExprCode {
            target_type: R,
            raw_handle: bindings.expr_code_apply_2(A, B, R, arg0.raw_handle, arg1.raw_handle, body),
        }
    }

    {- 当单端口 CMTT 模板返回类型依赖于 `p0` 运行时身份（如 `NodeIndex<p0>`）时，
       按 `docs/cmtt/any-and-code-evidence.md:27` 通过 `< p0: A |- Any >` 保留运行时依赖结构并绑定目标类型 `expected_ret`。 -}
    pub pure comptime fn apply_1_dependent(
        arg0: ExprCode,
        expected_ret: Type,
        body: < p0: A |- Any >,
    ) -> !PatternViewErr ExprCode
    where
        A: Type,
    {
        if arg0.target_type != A {
            return error PatternViewErr.TypeMismatch;
        }
        ExprCode {
            target_type: expected_ret,
            raw_handle: bindings.expr_code_apply_1_dependent(
                A,
                expected_ret,
                arg0.raw_handle,
                body,
            ),
        }
    }

    {- 双端口运行时依赖实例化：支持返回值类型依赖入参 `p0`/`p1` 的模板（如 `< st: Arena, idx: usize |- Any >`）。 -}
    pub pure comptime fn apply_2_dependent(
        arg0: ExprCode,
        arg1: ExprCode,
        expected_ret: Type,
        body: < p0: A, p1: B |- Any >,
    ) -> !PatternViewErr ExprCode
    where
        A: Type,
        B: Type,
    {
        if arg0.target_type != A or arg1.target_type != B {
            return error PatternViewErr.TypeMismatch;
        }
        ExprCode {
            target_type: expected_ret,
            raw_handle: bindings.expr_code_apply_2_dependent(
                A,
                B,
                expected_ret,
                arg0.raw_handle,
                arg1.raw_handle,
                body,
            ),
        }
    }
}

{- 基于运行期绑定的 ExprCode 构造依赖于该值的 normal-form/存在类型应用 `Head<arg>`（PascalCase 命名）。 -}
pub pure comptime fn ApplyDependentType(head: Type, arg: ExprCode) -> !PatternViewErr Type {
    bindings.apply_dependent_type_1(head, arg.raw_handle)
}

{- 编译器向 TuplePatternView 公开的单个子模式语法信息。 -}
pub struct PatternInput {
    slot_shape: PatternShape,
    annotated_type: ?Type,
    constant_carrier: ?ExprCode,
}

impl PatternInput {
    pub pure comptime fn shape(self) -> PatternShape {
        self.slot_shape
    }

    pub pure comptime fn expected_type(self) -> ?Type {
        self.annotated_type
    }

    pub pure comptime fn has_constant(self) -> bool {
        self.constant_carrier match {
            _? => true,
            null => false,
        }
    }

    {- 仅当该槽位为编译期可求值常量且类型精确匹配 T 时返回其 CT 值。 -}
    pub pure comptime fn constant_as(self) -> ?T where T: Type {
        self.constant_carrier match {
            code? => bindings.eval_constant_expr_code(T, code),
            null => null,
        }
    }
}

{- 传给 TuplePatternView 各步骤的只读主语与已就绪前缀快照。 -}
pub struct TuplePatternPrefix
where
    Subject,
{
    subject_binding: CodeBinding<Subject>,
    slot_inputs: List<PatternInput>,
    matched_prefix: List<ExprCode>,
}

impl TuplePatternPrefix<Subject> where Subject {
    pub pure comptime fn subject(self) -> CodeBinding<Subject> {
        self.subject_binding
    }

    pub pure comptime fn arity(self) -> usize {
        to_usize(self.slot_inputs.length())
    }

    pub pure comptime fn input(self, index: usize) -> ?PatternInput {
        self.slot_inputs.get(from_usize(index))
    }

    pub pure comptime fn prefix_values(self) -> List<ExprCode> {
        self.matched_prefix
    }

    pub pure comptime fn prefix_value(self, index: usize) -> ?ExprCode {
        self.matched_prefix.get(from_usize(index))
    }
}

{- 单臂代码准备构建器：按顺序向当前匹配臂的失败回退事务中追加求值、解包与守卫。 -}
pub struct MatchPreparation {
    raw_prep: bindings.RawMatchPrepHandle,
}

impl MatchPreparation {
    {- 生成无条件局部绑定：let tmp: T = expr; 供后续槽位依赖引用。 -}
    pub comptime fn bind(self, expr: ExprCode) -> !PatternViewErr CodeBinding<T>
    where
        T: Type,
    {
        if expr.ExprType() != T {
            return error PatternViewErr.TypeMismatch;
        }
        CodeBinding<T> {
            raw_handle: bindings.prep_bind_typed(self.raw_prep, T, expr.raw_handle),
        }
    }

    {- 生成类型擦除形式的无条件局部绑定，常用于泛化异构元组迭代与依赖类型构造。 -}
    pub comptime fn bind_erased(self, expr: ExprCode) -> ExprCode {
        bindings.prep_bind_erased(self.raw_prep, expr)
    }

    {- 生成可失败的可选解包：接收类型为 `?T` 的 ExprCode，若运行时为 null 则立即回退至下一臂。 -}
    pub comptime fn try_bind(self, expr: ExprCode) -> !PatternViewErr CodeBinding<T>
    where
        T: Type,
    {
        if expr.ExprType() != ?T {
            return error PatternViewErr.TypeMismatch;
        }
        CodeBinding<T> {
            raw_handle: bindings.prep_try_bind_typed(self.raw_prep, T, expr.raw_handle),
        }
    }

    {- 生成动态依赖类型的可选解包：传入明确内层 `inner_type` 与类型为 `?inner_type` 的 ExprCode。 -}
    pub comptime fn try_bind_erased(
        self,
        inner_type: Type,
        expr: ExprCode,
    ) -> !PatternViewErr ExprCode {
        bindings.prep_try_bind_erased(self.raw_prep, inner_type, expr.raw_handle)
    }

    {- 生成布尔守卫谓词测试（expr.ExprType() == bool）：若运行时为 false 则立即回退至下一臂。 -}
    pub comptime fn test(self, cond: ExprCode) -> !PatternViewErr void {
        if cond.ExprType() != bool {
            return error PatternViewErr.TypeMismatch;
        }
        bindings.prep_test(self.raw_prep, cond.raw_handle);
    }
}

pub struct TuplePatternStart
where
    P,
{
    state: P,
}

pub struct TuplePatternSlot
where
    P,
{
    element_type: Type,
    extractor: ExprCode,
    next_state: P,
}

pub trait TuplePatternView {
    assoc State: Type;

    comptime fn tuple_pattern_view_begin(
        prefix: TuplePatternPrefix<Self>,
        prep: MatchPreparation,
    ) -> !PatternViewErr TuplePatternStart<State>;

    comptime fn tuple_pattern_view_next(
        prefix: TuplePatternPrefix<Self>,
        prep: MatchPreparation,
        state: State,
        slot_index: usize,
        prev_slot: ?ExprCode,
    ) -> !PatternViewErr TuplePatternSlot<State>;

    comptime fn tuple_pattern_view_finish(
        prefix: TuplePatternPrefix<Self>,
        prep: MatchPreparation,
        state: State,
    ) -> !PatternViewErr void;
}

-- 辅助通用代码组合子（归 meta.patterns / meta.code）
pub pure comptime fn code_sequence(steps: List<ExprCode>) -> !PatternViewErr ExprCode {
    bindings.build_code_sequence(steps)
}

pub pure comptime fn code_match(
    scrutinee: ExprCode,
    result_type: Type,
    branches: List<ArmPreparation>,
) -> !PatternViewErr ExprCode {
    bindings.build_code_match(scrutinee, result_type, branches)
}
```

> **注**：`MatchPreparation` 的 `comptime fn` 使用 `self` 值引用语义操作当前展开上下文帧（编译器内部可变句柄），**绝无** `*self` 或 comptime 指针；`CodeBinding<T>` 携带帧身份检查，跨帧使用将返回 `PatternViewErr.DisallowedContextEscape`。

#### 3.5.3 完整工作示例与反例

##### 示例 A：标准异构定长包装元组 `TripleBox<A, B, C>`

展示无依赖异构容器如何在编译期逐槽导出静态类型 `A`、`B`、`C` 与提取代码（单端口 CMTT 模板 + `ExprCode.apply_1`，零多洞 splice）：

```flurry
use meta.patterns.{
    ExprCode,
    MatchPreparation,
    PatternViewErr,
    TuplePatternPrefix,
    TuplePatternSlot,
    TuplePatternStart,
    TuplePatternView,
}

pub struct TripleBox
where
    A,
    B,
    C,
{
    tag: u8,
    first: A,
    second: B,
    third: C,
}

impl TuplePatternView for TripleBox<A, B, C>
where
    A :- Copy,
    B :- Copy,
    C :- Copy,
{
    assoc State: Type = usize;

    comptime fn tuple_pattern_view_begin(
        prefix: TuplePatternPrefix<Self>,
        prep: MatchPreparation,
    ) -> !PatternViewErr TuplePatternStart<usize> {
        if prefix.arity() != 3 {
            return error PatternViewErr.InvalidArity;
        }
        -- 确保前置条件：tag 非 0 才可视作有效三元组
        let subj_code = prefix.subject().as_erased();
        let check_tag: < box_val: TripleBox<A, B, C> |- bool > = '( box_val.tag != 0 );
        let cond_code = ExprCode.apply_1<TripleBox<A, B, C>, bool>(subj_code, check_tag)!;
        prep.test(cond_code)!;
        TuplePatternStart<usize> { state: 0 }
    }

    comptime fn tuple_pattern_view_next(
        prefix: TuplePatternPrefix<Self>,
        _prep: MatchPreparation,
        _state: usize,
        slot_index: usize,
        _prev_slot: ?ExprCode,
    ) -> !PatternViewErr TuplePatternSlot<usize> {
        let subj_code = prefix.subject().as_erased();
        slot_index match {
            0 => {
                let get_first: < box_val: TripleBox<A, B, C> |- A > = '( box_val.first );
                TuplePatternSlot<usize> {
                    element_type: A,
                    extractor: ExprCode.apply_1<TripleBox<A, B, C>, A>(subj_code, get_first)!,
                    next_state: 1,
                }
            },
            1 => {
                let get_second: < box_val: TripleBox<A, B, C> |- B > = '( box_val.second );
                TuplePatternSlot<usize> {
                    element_type: B,
                    extractor: ExprCode.apply_1<TripleBox<A, B, C>, B>(subj_code, get_second)!,
                    next_state: 2,
                }
            },
            2 => {
                let get_third: < box_val: TripleBox<A, B, C> |- C > = '( box_val.third );
                TuplePatternSlot<usize> {
                    element_type: C,
                    extractor: ExprCode.apply_1<TripleBox<A, B, C>, C>(subj_code, get_third)!,
                    next_state: 3,
                }
            },
            _ => error PatternViewErr.InvalidArity,
        }
    }

    comptime fn tuple_pattern_view_finish(
        _prefix: TuplePatternPrefix<Self>,
        _prep: MatchPreparation,
        state: usize,
    ) -> !PatternViewErr void {
        if state != 3 {
            return error PatternViewErr.InvalidArity;
        }
    }
}
```

##### 示例 B：运行时依赖元组 `(storage, NodeIndex<storage>)` 与通配符 `(_, index)` 规则

设有一组在堆区或竞技场中通过**运行时实例身份**区分类型的句柄 `Arena` 与 `NodeIndex<arena>`，打包为运行时存在/依赖载体 `GraphCursor`：

```flurry
-- 依赖类型：NodeIndex 由一个运行时 Arena 的身份绑定参数化
pub struct Arena {
    id: u64,
    count: usize,
}

impl Clone for Arena {
    fn clone(*self) -> Self {
        Arena { id: self.id.*, count: self.count.* }
    }
}
impl Copy for Arena {}

pub struct NodeIndex
where
    quote storage: Arena,
{
    slot: usize,
}

impl Arena {
    pub fn node_at(*self, idx: usize) -> ?NodeIndex<self.*> {
        if idx < self.count.* {
            NodeIndex<self.*> { slot: idx }
        } else {
            null
        }
    }

    pub fn read_slot(self, idx: NodeIndex<self>) -> u64 {
        idx.slot.as(u64) + self.id
    }
}

-- 对外屏蔽动态身份的依赖对视图载体（概念化封装）
pub struct GraphAnchor {
    raw_arena: Arena,
    raw_slot: usize,
}

impl TuplePatternView for GraphAnchor {
    assoc State: Type = usize;

    comptime fn tuple_pattern_view_begin(
        prefix: TuplePatternPrefix<Self>,
        _prep: MatchPreparation,
    ) -> !PatternViewErr TuplePatternStart<usize> {
        if prefix.arity() != 2 {
            return error PatternViewErr.InvalidArity;
        }
        TuplePatternStart<usize> { state: 0 }
    }

    comptime fn tuple_pattern_view_next(
        prefix: TuplePatternPrefix<Self>,
        prep: MatchPreparation,
        _state: usize,
        slot_index: usize,
        prev_slot: ?ExprCode,
    ) -> !PatternViewErr TuplePatternSlot<usize> {
        let subj_code = prefix.subject().as_erased();
        slot_index match {
            0 => {
                -- 第 0 槽产生 Arena；即使上层用户写成 `_`，编译器仍将其值通过 prev_slot 传给第 1 槽
                let get_arena: < g: GraphAnchor |- Arena > = '( g.raw_arena );
                let arena_expr = ExprCode.apply_1<GraphAnchor, Arena>(subj_code, get_arena)!;
                let arena_binding = prep.bind<Arena>(arena_expr)!;
                TuplePatternSlot<usize> {
                    element_type: Arena,
                    extractor: arena_binding.as_erased(),
                    next_state: 1,
                }
            },
            1 => {
                -- 第 0 槽已通过 prep.bind<Arena>(...) 固定成帧局部绑定并传回 prev_slot，可直接复用
                let storage_code = prev_slot match {
                    code? => code,
                    null => return error PatternViewErr.InvalidPatternInput,
                }
                let DepNode = meta.patterns.ApplyDependentType(NodeIndex, storage_code)!;
                let get_slot: < g: GraphAnchor |- usize > = '( g.raw_slot );
                let slot_code = ExprCode.apply_1<GraphAnchor, usize>(subj_code, get_slot)!;
                -- 使用 `< st: Arena, idx: usize |- Any >` 经 apply_2_dependent 保留对 `st` 身份依赖并校验目标 `?DepNode`
                let call_node_at: < st: Arena, idx: usize |- Any > = '( st.node_at(idx) );
                let opt_node_code = ExprCode.apply_2_dependent<Arena, usize>(
                    storage_code,
                    slot_code,
                    ?DepNode,
                    call_node_at,
                )!;
                let checked_idx = prep.try_bind_erased(DepNode, opt_node_code)!;
                TuplePatternSlot<usize> {
                    element_type: DepNode,
                    extractor: checked_idx,
                    next_state: 2,
                }
            },
            _ => error PatternViewErr.InvalidArity,
        }
    }

    comptime fn tuple_pattern_view_finish(
        _prefix: TuplePatternPrefix<Self>,
        _prep: MatchPreparation,
        _state: usize,
    ) -> !PatternViewErr void {}
}

fn use_dependent_tuple(anchor: GraphAnchor) -> u64 {
    -- 1. 显式绑定两槽：index 的类型直接对齐到 storage
    let a = anchor match {
        (storage, index) => storage.read_slot(index),
        _ => 0,
    }
    -- 2. 用通配符丢弃第 0 槽：index 仍然获得匿名内部 storage_0 的合法类型 NodeIndex<__slot0>！
    --    `(_, index)` 可以提取 index，但不能在没有同一 storage 的情况下混用给别的 Arena
    let b = anchor match {
        (_, index) => index.slot.as(u64),
        _ => 0,
    }
    a + b
}
```

> **注意**：在 `(_, index)` 中，编译器保留了 `slot 0` 的 `CodeBinding<Arena>` 作为内部存在见证身份，保证 `index` 的依赖类型健全；同时由于 `_` 未向用户作用域暴露 `storage`，用户无法错误地将 `index` 传给另一个不相关的 `other_arena.read_slot(index)`，从而在编译期彻底杜绝跨竞技场索引错乱。

##### 示例 C：Tier 2/3 生态正则模式消费者 `RegexMatcher`（含 4 项显式负例）

> **包归属说明**：本示例演示**外部 Tier 2/3 生态包（如 `user_regex`）**如何在六内置包之外消费 `meta.patterns.TuplePatternView`、`CodeBinding` 与 `MatchPreparation`，**绝不将 `regex` 放入 `builtin` 或 `std` 六内置包内**。整体语法依据 `docs/` 规范按现行 Flurry 编写（使用 `RegexErr` 错误枚举、无 `Option`/`Result`、无 `^comptime_phase`、无 `*self` 编译时指针）。

```flurry
-- 模块：user_regex（Tier 2/3 生态用户包示例，不属于 builtin / std 六内置包）
use builtin.{TextBytes, u64, usize}
use meta.ComptimeTextBytes;
use meta.patterns.{
    CodeBinding,
    ExprCode,
    MatchPreparation,
    PatternViewErr,
    TuplePatternPrefix,
    TuplePatternSlot,
    TuplePatternStart,
    TuplePatternView,
}
use std.collections.Slice;

pub enum RegexErr {
    EmptyPattern,
    UnbalancedGroup,
    UnexpectedQuantifier,
    TrailingEscape,
}

pub struct RegexSpan {
    start: usize,
    end: usize,
}

impl Clone for RegexSpan {
    fn clone(*self) -> Self {
        RegexSpan { start: self.start.*, end: self.end.* }
    }
}
impl Copy for RegexSpan {}

pub struct RegexGroupSpec {
    optional: bool,
    literal_byte: u8,
}

impl Clone for RegexGroupSpec {
    fn clone(*self) -> Self {
        RegexGroupSpec { optional: self.optional.*, literal_byte: self.literal_byte.* }
    }
}
impl Copy for RegexGroupSpec {}

pub struct CompiledRegex {
    group_specs: meta.List<RegexGroupSpec>,
}

-- 完整自包含的编译期微型正则解析器：使用 ComptimeTextBytes 解析 `(x)` 与 `(x)?` 并严格报错
pub pure comptime fn compile_regex(pattern: str) -> !RegexErr CompiledRegex {
    let text_view = pattern.as(ComptimeTextBytes);
    let total = text_view.byte_length();
    if total == 0 {
        return error RegexErr.EmptyPattern;
    }
    var specs: meta.List<RegexGroupSpec> = [];
    var idx: u64 = 0;
    while idx < total {
        let ch = text_view.byte(idx) match {
            b? => b,
            null => return error RegexErr.UnbalancedGroup,
        }
        if ch == 92 {
            return error RegexErr.TrailingEscape;
        }
        if ch == 40 {
            let inner = text_view.byte(idx + 1) match {
                b? => b,
                null => return error RegexErr.UnbalancedGroup,
            }
            let close = text_view.byte(idx + 2) match {
                b? => b,
                null => return error RegexErr.UnbalancedGroup,
            }
            if close != 41 {
                return error RegexErr.UnbalancedGroup;
            }
            var opt = false;
            var step: u64 = 3;
            let maybe_q = text_view.byte(idx + 3);
            if maybe_q match {
                b? => b == 63,
                null => false,
            } {
                opt = true;
                step = 4;
            }
            specs.append(RegexGroupSpec { optional: opt, literal_byte: inner });
            idx = idx + step;
        } else if ch == 63 {
            return error RegexErr.UnexpectedQuantifier;
        } else {
            return error RegexErr.UnbalancedGroup;
        }
    }
    CompiledRegex { group_specs: specs }
}

pub pure comptime fn RegexCaptureType(spec: RegexGroupSpec) -> Type {
    if spec.optional { ?str } else { str }
}

pub struct RegexMatcher {
    input: str,
}

pub fn match_literal_groups(input: str, specs: Slice<RegexGroupSpec>) -> ?Slice<?RegexSpan> {
    let text_view = input.as(TextBytes);
    let total = text_view.byte_length();
    var cursor: u64 = 0;
    let group_len = specs.len();
    var i: usize = 0;
    -- 仅用于概念示范线性匹配；实际 Tier 2 生态包可使用显式 Allocator 缓冲区管理捕获跨度表
    while i < group_len {
        let spec = specs.get(i) match {
            s? => s,
            null => return null,
        }
        let next_byte = text_view.byte(cursor);
        if cursor < total and (next_byte match {
            b? => b == spec.literal_byte,
            null => false,
        }) {
            cursor = cursor + 1;
        } else if not spec.optional {
            return null;
        }
        i = i + 1;
    }
    if cursor != total {
        return null;
    }
    -- 为保持示例代码零隐式全局分配且不省略核心类型流，这里返回空捕获元切片契约；
    -- 真实 Tier 2 匹配引擎按 specs 写入捕获表或固定静态槽位数组。
    Slice<?RegexSpan>.empty()
}

-- 注：依赖 WL-02-12 在 builtin.TextBytes 补齐与 ComptimeTextBytes.slice 对称的 slice 方法
pub fn slice_capture(input: str, opt_span: ?RegexSpan) -> ?str {
    opt_span match {
        span? => input.as(TextBytes).slice(span.start.as(u64), span.end.as(u64)),
        null => null,
    }
}

pub struct RegexTupleState {
    specs: meta.List<RegexGroupSpec>,
    pattern_lit: str,
    captures_binding: CodeBinding<Slice<?RegexSpan>>,
}

impl TuplePatternView for RegexMatcher {
    assoc State: Type = RegexTupleState;

    comptime fn tuple_pattern_view_begin(
        prefix: TuplePatternPrefix<Self>,
        prep: MatchPreparation,
    ) -> !PatternViewErr TuplePatternStart<RegexTupleState> {
        if prefix.arity() == 0 {
            return error PatternViewErr.InvalidArity;
        }
        -- 第 0 槽必须是编译期常量字符串，不允许变量、通配符或 `or` 组合模式
        let first_input = prefix.input(0) match {
            v? => v,
            null => return error PatternViewErr.InvalidArity,
        }
        first_input.shape() match {
            .constant => {},
            _ => return error PatternViewErr.ExpectedConstantSlot,
        }
        let pattern_lit = first_input.constant_as<str>() match {
            s? => s,
            null => return error PatternViewErr.TypeMismatch,
        }
        let compiled = compile_regex(pattern_lit) match {
            c! => c,
            error _ => return error PatternViewErr.InvalidPatternInput,
        }
        let expected_arity = 1 + meta.to_usize(compiled.group_specs.length());
        if prefix.arity() != expected_arity {
            return error PatternViewErr.InvalidArity;
        }
        let subj_code = prefix.subject().as_erased();
        let specs = compiled.group_specs;
        -- `specs` 是普通编译期数据，直接按常量值捕获在单端口模板内经 Splice 落盘；零 `~<...>` 违例
        -- 依赖 meta.List<T> 实现 Splice<RuntimeTarget = Slice<T.RuntimeTarget>>
        let run_match: < m: RegexMatcher |- ?Slice<?RegexSpan> > = '(
            match_literal_groups(
                m.input,
                meta.List<RegexGroupSpec>.as(builtin.Splice).splice(specs),
            )
        );
        let match_code = ExprCode.apply_1<RegexMatcher, ?Slice<?RegexSpan>>(subj_code, run_match)!;
        -- try_bind 仅生成一次运行时执行：失败则返回 null 自动回退至 match 下一臂
        let captures_binding = prep.try_bind<Slice<?RegexSpan>>(match_code)!;
        TuplePatternStart<RegexTupleState> {
            state: RegexTupleState {
                specs: compiled.group_specs,
                pattern_lit: pattern_lit,
                captures_binding: captures_binding,
            },
        }
    }

    comptime fn tuple_pattern_view_next(
        prefix: TuplePatternPrefix<Self>,
        _prep: MatchPreparation,
        state: RegexTupleState,
        slot_index: usize,
        _prev_slot: ?ExprCode,
    ) -> !PatternViewErr TuplePatternSlot<RegexTupleState> {
        if slot_index == 0 {
            -- 第 0 槽从闭合常量代码返回正则源码字面量，供子模式 `"(a)(b)?"` 恒等校验通过
            let pat = state.pattern_lit;
            let pat_code: < |- str > = '( pat );
            return TuplePatternSlot<RegexTupleState> {
                element_type: str,
                extractor: ExprCode.from_closed<str>(pat_code),
                next_state: state,
            }
        }
        let idx_ct = slot_index - 1;
        let spec = state.specs[meta.from_usize(idx_ct)];
        let subj_code = prefix.subject().as_erased();
        let cap_code = state.captures_binding.as_erased();
        if spec.optional {
            -- idx_ct 作为普通 comptime usize 常量直接被模板捕获，双端口模板仅含 CMTT 虚拟端口 m 与 caps
            let extract_opt: < m: RegexMatcher, caps: Slice<?RegexSpan> |- ?str > = '(
                slice_capture(
                    m.input,
                    caps.get(idx_ct) match {
                        inner? => inner,
                        null => null,
                    },
                )
            );
            let opt_code = ExprCode.apply_2<RegexMatcher, Slice<?RegexSpan>, ?str>(
                subj_code,
                cap_code,
                extract_opt,
            )!;
            TuplePatternSlot<RegexTupleState> {
                element_type: ?str,
                extractor: opt_code,
                next_state: state,
            }
        } else {
            let extract_req: < m: RegexMatcher, caps: Slice<?RegexSpan> |- str > = '(
                slice_capture(
                    m.input,
                    caps.get(idx_ct) match {
                        inner? => inner,
                        null => null,
                    },
                ) match {
                    s? => s,
                    null => "",
                }
            );
            let req_code = ExprCode.apply_2<RegexMatcher, Slice<?RegexSpan>, str>(
                subj_code,
                cap_code,
                extract_req,
            )!;
            TuplePatternSlot<RegexTupleState> {
                element_type: str,
                extractor: req_code,
                next_state: state,
            }
        }
    }

    comptime fn tuple_pattern_view_finish(
        _prefix: TuplePatternPrefix<Self>,
        _prep: MatchPreparation,
        _state: RegexTupleState,
    ) -> !PatternViewErr void {}
}

-- 正例：matching "(a)(b)?" 绑定 first: str, second: ?str
fn parse_ab(text: str) -> str {
    let matcher = RegexMatcher { input: text }
    matcher match {
        ("(a)(b)?", first, second?) => second,
        ("(a)(b)?", first, null) => first,
        _ => "no match",
    }
}

-- 负例 1（编译期拒绝）：第 0 槽使用非常量/绑定变量
{-
fn bad_dynamic_regex(m: RegexMatcher, pat: str) -> str {
    m match {
        (pat, x) => x, -- ExpectedConstantSlot：shape 为 .binding 而非 .constant
        _ => "",
    }
}
-}

-- 负例 2（编译期拒绝）：第 0 槽内部出现 `or` 析取模式
{-
fn bad_or_slot_regex(m: RegexMatcher) -> str {
    m match {
        ("(a)" or "(b)", x) => x, -- ExpectedConstantSlot：shape 为 .complex
        _ => "",
    }
}
-}

-- 负例 3（编译期拒绝）：正则声明的捕获组数与模式元组元数不一致
{-
fn bad_arity_regex(m: RegexMatcher) -> str {
    m match {
        ("(a)(b)?", only_one) => only_one, -- InvalidArity：期望 3 个槽位，实际得到 2 个
        _ => "",
    }
}
-}

-- 负例 4（编译期拒绝）：必选分组 vs 可选分组类型冲突或缺失通配臂
{-
fn bad_type_and_exhaustiveness(m: RegexMatcher) -> str {
    m match {
        ("(a)(b)?", 123, _) => "", -- 类型错：第 1 槽产出 str 而非整数
        -- 且即使类型正确，由于 RegexMatcher 未被标记为全域覆盖，省略 `_` 必须报非穷尽错误
    }
}
-}
```

#### 3.5.4 `ComptimeTuplePatternView` 与纯编译时求值

`TuplePatternView` 在 `tuple_pattern_view_begin` / `next` / `finish` 中使用 `MatchPreparation` 向 runtime 发出代码绑定。当用户在 `comptime fn` 中**直接对一个编译时已知值的自定义 `TuplePatternView` 类型做模式匹配**时，存在两种可行方案：
1. **CT VM 直接执行 `MatchPreparation` 残余代码**（推荐最小正交路径）：前提是 `tuple_pattern_view_begin`/`next` 提取表达式内调用的函数（如 `compile_regex` / `match_regex_group_count`）具备 `comptime` 相位实现；
2. **引入对称的 `pure comptime trait ComptimeTuplePatternView`**：跳过 `CodeBinding` / `< |- T >` 引号层，直接操纵 `comptime` 值快照 `(Self, List<PatternInput>) -> !PatternViewErr List<ComptimeAny>`（需配套完善 `Any` 与值引用的反射解包）。本问题列于第 5 节 Open Question 4 待作者裁决。

---

### 3.6 `StructPatternView`、记录剩余 `..`、`PatternKind::{Ref, Async, Comptime}`、`TypePatternView` 与模式函数

#### 3.6.1 记录剩余 `..` 与 `StructPatternView` / `ComptimeStructPatternView`

1. **普通具名 `struct` 与匿名记录中的 `..`（`records.rs:53` 缺口闭合）**：
   - 允许写 `Point { x, .. }` 与 `{ a: first, .. }`。
   - **所有权与 `Drop` 交互语义**：
     - 当主语为非仿射/所有字段满足 `Copy` 或主语经由指针 `*Point` 匹配时，`{ x, .. }` 仅投影或拷贝 `x`；
     - 当主语按值传入且包含 `Once` 非 `Copy` 未绑定字段（例如 `Pair { first: owned, .. }`），只要 `Pair` **未自定义 `Drop`**，允许局部拆分移动 `first` 并自动对其余未提及且活跃的 `Once` 字段执行尾部清理；若 `Pair` 实现了 `Drop`，禁止部分移动（只能绑定 `*self`/全字段或拒绝 `Once` 移动），避免破坏 `Drop` 整体不变量！
2. **`StructPatternView`（定域结构视图）vs `MapPatternView`（开放动态键）**：
   - `MapPatternView` 的 `map_pattern_view_get` 返回 **`?FieldType(property)`**，表达“键可能缺席”；而很多强类型代理视图（AoS-to-SoA 行视图 `SoaElemRef<T>`、位包结构体零拷贝解包视图、跨 FFI 只读句柄）在编译期就拥有固定封闭的字段列表且**每个合法字段必然存在**，无需返回 `?` 包装！
   - 为此提供 `StructPatternView` 及其稳定性标记与 `Comptime*` 双胞胎：

```flurry
-- 模块：builtin.protocols.patterns
use primitives.{Symbol, Type, bool}

pub trait StructPatternView {
    assoc FieldType: pure comptime fn(Symbol) -> Type;

    fn struct_pattern_view_get(*self, comptime field: Symbol) -> FieldType(field);
}

pub trait StableStructPatternView :- StructPatternView {}
```

```flurry
-- 模块：meta.data.patterns
use builtin.{Symbol, Type}

pub pure comptime trait ComptimeStructPatternView {
    assoc FieldType: pure comptime fn(Symbol) -> Type;

    comptime fn struct_pattern_view_get(self, field: Symbol) -> FieldType(field);
}

pub pure comptime trait ComptimeStableStructPatternView :- ComptimeStructPatternView {}
```

- 与 `MapPatternView` 的分派优先序：对非原生命名 `struct`/匿名记录类型，若同时寻找自定义结构解构视图，**`StructPatternView` 适用于字段必存在且类型由 `FieldType(field)` 返回的定域结构模式（未声明字段通过 `FieldType` 返回 `NoReturn` 或直接静态诊断拒绝），`MapPatternView` 适用于允许键缺席的映射模式（返回 `?FieldType`）**；同一类型严禁同时实现两者以免二义。

#### 3.6.2 闭合 `PatternKind::{Ref, Async, Comptime}` HIR/Elaboration 断层

现状核实：`compiler/hir/src/pattern.rs` 已定义 `PatternKind::Ref`、`Async`、`Comptime`，但在 `compiler/elaboration/src/program/pattern_checking/mod.rs` 均落入未实现分支。必须闭合或清理：
- **`PatternKind::Ref`（`ref pat`，见 `docs/grammar/syntax:543`）**：对指针或可借用主语执行受控借用/解引用投影（仅在 `T :- Copy` 或内部继续绑定指针/引用时合法，不可通过 `ref pat` 偷走 `Once` 所有权）；与显式 `.ref` / `value.ref` 保持所有权边界一致（与 [patterns.md](file:///home/fin/luna/docs/control-flow/pattern-matching/index.md) 保持一致）。
- **`PatternKind::Comptime` / `PatternKind::Async`**：若语法不再保留 `async pat`（`docs/grammar/syntax:544`）或内联 `comptime` 模式前缀，应显式在 `ast_lowering` 拒绝并移除死变体；若保留，须在 `pattern_checking` 检查并落地相应阶段约束。

#### 3.6.3 `TypePatternView`、`Type` `'id` 绑定与依赖 Normal-Form 模式

依据 [type-patterns.md](file:///home/fin/luna/docs/type/type-patterns.md) 与 `pattern_checking/mod.rs:1109,1133` 的现存缺口：

1. **编译器必须补全通用上下文依赖 NF 值模式**：
   例如匹配依赖头应用 `Buffer<'Len, 'Payload>` 时，解开 `'Len` 后需立即把已绑定的 `Len: usize` 注入上下文再验证并绑定依赖类型项 `'Payload`（例如 `Array<u8, Len>`），消除当前 `"dependent normal-form value patterns require contextual pattern elaboration not yet implemented"` 阻断。
2. **自定义类型构造函数解构协议 `TypePatternView`**：
   除原生支持 `Tuple`、`struct`、`enum` 及 `pure comptime fn<...>(...) -> Type` 的 normal-form 头匹配外，允许“经变换/规范化后的自定义编译时类型构造器”暴露语义解构视图，而不必展开 normal-form 透明定义（严格遵守 `AGENTS.md`：Normal form 的头不展开！）：

```flurry
-- 模块：meta.patterns
use builtin.{Type, bool}
use data.list.List;

pub trait TypePatternView {
    assoc Args: Type; -- 通常为 Tuple<...> 或有序字段结构

    pure comptime fn type_pattern_view_destructure(target: Type) -> ?Args;
}
```

- 语义边界：仅当目标类型通过已知应用头或绑定实现 `TypePatternView` 且用户在类型模式位置解构时触发；返回值 `null` 表示类型匹配失败并转移到 `match` 下一臂（绝不绕过 `Identity<T> != T` 的名义/normal-form 身份隔离规则）。

#### 3.6.4 模式函数（`case` 视图、函数级 `case` 开放分发与 Quoted Arms）

1. **`case` 自定义模式视图的三级增强（`cases.rs` 扩展）**：
   - **额外实参支持**：支持 `case in_range(*self, lo: i64, hi: i64) -> ?Tuple<i64>` 及自由 `case parse_hex(s: str) -> ?Tuple<u64>`（与 `docs/control-flow/pattern-matching/index.md:729-760` 规格一致）；
   - **三级纯度与 DAG 分析等级**：
     - **Tier 1（Pure / `Stable`）**：`pure fn` / 标记 `^attrs.pattern.stable`（或随 `StableCase` 标记）的 `case`：无副作用、结果只依赖输入值，允许接入 `pattern_automata::DecisionDag` 跨臂单次调用缓存并合并公共观察节点；
     - **Tier 2（Read-only 无副作用但未证明幂等）**：仍以 `*self` 借用并走事务 `Observe`/`Then` 回退链，不可跨无关联参数复用，非穷尽（要求 `_`）；
     - **Tier 3（有副作用 / 消耗状态）**：禁止与多臂乐观合并，必须严格按从上到下源码次序每臂重新执行，且需 `_` 兜底。
   - **谓词 `case`（`-> bool`）**：[pattern-matching/index.md](file:///home/fin/luna/docs/control-flow/pattern-matching/index.md) 目前将“无载荷谓词 `case` 直接返回 `-> bool` 还是必须返回 `-> ?Tuple<>`”标为待定，列入第 5 节 Open Question 5。
2. **函数级模式定义（`fn eval(expr: Expr) -> i64; fn eval(case .lit(n)) -> i64 { ... }` 开放包封闭集合）**：
   - 依据 [pattern-functions.md](file:///home/fin/luna/docs/functions/pattern-functions.md) 与 `docs/grammar/syntax:294`（`case_param -> case pattern`，无 `: Type` 标注）：允许先声明共享签名的原函数，再于同包不同文件以多个 `fn name(case <pattern>, ...) -> Output` 定义各分支，自动聚合成包封闭的函数族；
   - **包封闭与顺序无关覆盖检查**：各 `case` 定义臂除 `case` 形参外其余形参和返回签名必须与主签名一致；编译器在包级分析完成前合并所有 `case` 臂，检查**互斥性与覆盖完备性**，生成统一 `match` 分发体（允许与其它调用交错且不依赖源码书写顺序；若两条 `case` 臂存在非包含重叠模式则报错提示歧义，见第 5 节 Open Question 6）。
3. **Quoted Patterns `< T -| Ψ >`、Quoted Arms 与 `template quote fn` / `use` 模板**：
   - 依据 [patterns-and-arms.md](file:///home/fin/luna/docs/cmtt/patterns-and-arms.md) 与 [quote-functions-and-templates.md](file:///home/fin/luna/docs/cmtt/quote-functions-and-templates.md)：

```flurry
-- 模块：meta.patterns
use bindings;
use builtin.{Type, bool}
use data.list.List;

{- 封装带闭合/开放上下文见证的模式模板臂，供 template quote fn 与 match `use` 协议展开。 -}
pub struct ArmPreparation {
    raw_arm: bindings.RawMatchArmHandle,
}

impl ArmPreparation {
    pub comptime fn has_guard(self) -> bool {
        bindings.arm_has_guard(self.raw_arm)
    }
}
```

   - **完整 `template quote fn` + `quote fn arm(case ...)` + `match { use template }` 契约示例**：

```flurry
use attrs.quote_fn_template;

enum EvalAction {
    literal(u32),
    add(u32, u32),
    neg(u32),
}

-- 1. 声明模板接口（须携带 for_match_arms；可选 allow_non_exhaustive_instance）
^quote_fn_template.for_match_arms
^quote_fn_template.allow_non_exhaustive_instance
template quote fn eval_action_rule(action: EvalAction) -> u32;

-- 2. 各规则臂通过 `case <pattern>` 与 `use <template>` 挂载到模板
quote fn add_action_arm(case .add(left, right)) use eval_action_rule {
    left + right
}

quote fn neg_action_arm(case .neg(val)) use eval_action_rule {
    0u32 - val
}

-- 3. `match` 通过 `use eval_action_rule` 展开全部挂载臂，并与局部显式臂联合检查穷尽性
fn run_action(action: EvalAction) -> u32 {
    action match {
        use eval_action_rule,
        .literal(v) => v,
    }
}

-- 负例（编译期拒绝）：未携带 for_match_arms 的普通 quote fn 不能被 `use` 接入 `match`
{-
quote fn plain_helper(x: EvalAction) -> u32 { 0 }

fn bad_use_arm(v: EvalAction) -> u32 {
    v match {
        use plain_helper, -- 错误：plain_helper 不是携带 __flurry_quote_fn_for_match_arms 的 template quote fn
        _ => 0,
    }
}
-}
```

   - 覆盖度交互：默认情况下，`match` 中经 `use template` 展开的所有 quoted arms 必须覆盖完整主语类型（除非显式与局部臂互补）；若模板自身刻意仅生成部分规则臂，须额外标注 `^attrs.quote_fn_template.allow_non_exhaustive_instance`（如上 `eval_action_rule`），由最终实例化处的 `match` 将挂载臂与局部 `.literal(v)` 臂合并后执行完整穷尽性校验（若仍缺变体如漏掉 `.literal`，外层 `match` 照常报非穷尽错误）。

---

### 3.7 `BitVec` 模式与跨元素编码

依据 [bitvec.md](file:///home/fin/luna/docs/control-flow/pattern-matching/bitvec.md) 与 [bitvec-algebra.md](file:///home/fin/luna/docs/program-synthesis/bitvec-algebra.md) 及 `docs/grammar/syntax:546-554`（`0b|0o|0x segment+` 空格并置段语法）：

1. **合法主语范围与隐式转换禁止**：
   - `0b segment+` / `0o segment+` / `0x segment+` 位模式段**仅允许**匹配：
     1. `BitVec<N>` 类型值；
     2. 显式经 `expr'bitvec` 取像转成 `BitVec<N>` 的整数/值表达式；
     3. 声明了显式固定位宽编码的 `enum`（豁免直接应用位模式，无需先写 `'bitvec`）。
   - 普通结构体、元组、整数 `u32`（未做 `'bitvec` 取像）直接匹配 `0x aa b` 必须拒绝（整数用普通整数字面量 `0xff`，不要混淆进 bitvec 空格并置段语法，禁止使用不存在的方括号逗号写法）。
2. **段位序（Text MSB-first vs BitVec Algebra LSB = bit 0）精确对齐契约**：
   - 源码文本字面量（如 `0x f (a: BitVec<4>) (low: BitVec<4>)`）从左到右按**最高有效位到最低有效位（MSB -> LSB）**排列；
   - 因此在 `BitVec<N>` 位索引语义下，若左边第 1 段占 `w0` 位、第 2 段占 `w1` 位……最后一段占 `wk` 位且 `w0 + w1 + ... + wk == N`，则第 0 段对应切片 `bits[(N - w0) .. N]`，最后一段对应 `bits[0 .. wk]`；等价于代数串联 `seg_0 ++ seg_1 ++ ... ++ seg_k`。编译器前端与 LLVM 后端必须严格统一该切片方向，防止小端序主机下字节高低位反转。
3. **`BitVec` `++` 拼接模式（宽度代数分裂）**：
   - 与 `ConcatPatternView` 不同，在 `BitVec<N>` 上写 `(hi : BitVec<8>) ++ (lo : BitVec<24>)` 或 `0xcafebabe ++ payload` 时：
     - 若左侧（或右侧）提供了静态固定位宽（来自字面位段宽度 `32` 或显式类型标定 `: BitVec<M>` 或固定位宽 enum），且 `0 <= M <= N`，则另一侧自动推导出精确类型 **`BitVec<N - M>`** 并直接切出高/低位子 `BitVec`；
     - 若两侧均未知静态位宽（如 `a ++ b` 无任何类型/长度标注），在编译期直接拒绝歧义。
4. **动态宽度段与运行时位切片 `<expr>`**：
   - 允许在 `0b` / `0o` / `0x` 空格并置段中使用转义表达式 `<width_or_val_expr>` 与嵌套检查 `(inner_pat : SubType)`（如 `0b1110(xs: BitVec<4>) 10(ys: BitVec<6>) 10(zs: BitVec<6>)`）；
   - 当存在动态长度或 `BitVec<0>`/运行期计算位宽时，模式绑定时先检查所有静态已知段宽度之和不超过目标位宽，并对动态段执行一次单求值宽度边界与值比对。
5. **枚举位表示配置的 `__flurry_*` 正式属性化演进**：
   - 当前 [enums.md](file:///home/fin/luna/docs/type/user-defined-types/enums.md) 使用过渡期的类内配置 `.base_type = u8` / `.discrete_tag = false` / `.dst = true`；
   - 按 `AGENTS.md` 官方属性规范，后续统一由 `attrs.layout` 产出 `__flurry_*` 属性（如 `__flurry_enum_base_type`、`__flurry_enum_discrete_tag`、`__flurry_enum_dst_tag`）附着到 `enum` 声明，并在过渡期内保持对既有 `.id expr` 配置语法的只读兼容（同时对用户提供规范迁移路径）。

---
### 3.8 完整 Take View（`expr ' id`）注册表与准入边界

#### 3.8.1 现有 HIR Take View 全景台账（全部 10 种实现现状核查）

依据 [view-taking.md](file:///home/fin/luna/docs/others/view-taking.md)、[syntax-and-hir.md](file:///home/fin/luna/docs/compilation/syntax-and-hir.md)、`compiler/ast_lowering/src/expr.rs` 与 `compiler/hir/src/expr.rs`：

| Take View 写法 | 对应 `hir::ExprKind` | 允许阶段 | 主语域与返回语义 | 编译器当前状态与缺口 |
|---|---|---|---|---|
| `expr'type` | `ExprKind::TypeOf` | RT + CT | 观察 `expr` 的静态类型或存在/依赖类型并返回 `Type`，**不移动也不消耗 `Once` 所有权**；主语仅求值一次（若为存在依赖载体可携带运行时身份） | **已接通**；与依赖类型运行期检查持续同步 |
| `expr'tag_name` | `ExprKind::TakeTagName` | RT + CT | 主语仅限 `Symbol` 或 `enum` 值；返回 `str`（对于嵌套分组 `enum` 返回 `"group.leaf"` 完整变体声明路径，不返回符号编码值）；普通结构体拒绝 | **已接通** |
| `expr'old` | `ExprKind::TakeOld` | Contract（`ensures` 专属） | 在函数后置条件中捕获形参或可变地点在入口时的前态值快照；禁止出现在普通函数体、`requires` 或循环不变式外位置 | **已接通** |
| `expr'bitvec` | `ExprKind::TakeBitVec` | RT + CT | 将固定位宽整数、`bool`、`char` 或固定位宽编码 `enum` 投射为 `BitVec<W>` 快照；作 `word'bitvec[lo..hi] = rhs` 左值切片时提供安全位置写回（place writeback） | **已接通** |
| `target'attribute` | `ExprKind::TakeAttribute` | CT | 读取附着在类型、实例或目标声明上的合并后 `meta.Object`（含 `__flurry_*` 官方条目与用户元数据）；无属性时返回空 `{}` | **已接通**，但 **`docs/others/view-taking.md` 当前遗漏此条（`WL-02-23`）** |
| `tup'tuple_indexed` | `ExprKind::TakeTupleIndexed` | CT / `inline for` | 仅可作为 `inline for (i, elem) in tup'tuple_indexed` 的被迭代体，静态展开为 `(comptime Integer index, elem)`；脱离 `inline for` 直接绑定变量必须拒绝 | **已接通** |
| `tup'tuple_enumerate` | `ExprKind::TakeTupleEnumerate` | CT / `inline for` | 仅可用于 `inline for elem in tup'tuple_enumerate` 静态按序遍历元组元素；支持混合类型与存在/依赖项展开 | **部分接通**（`WL-02-24`：CT `inline for` 混合 runtime 值元组路径在 `views.rs:194` 存在待修复边界） |
| `ptr'multi_deref` | `ExprKind::MultiDeref` | RT（计划） | 解析器与 AST->HIR 已构造 `ExprKind::MultiDeref`，意图沿嵌套指针链或宽指针执行多层解引用视图 | **断层未接通（`WL-02-25`）**：`elaboration/src/program/expression_values/{mod,views,places}.rs` 完全缺失该节点分支，使用即报未实现错误（见 OQ-7） |
| `.name'builtin` | `ExprKind::Builtin` | 内部绑定层 | 绑定编译器内置符号或构造器；**仅限六内置包集中绑定文件使用，用户源码严禁书写** | **已接通且强制隔离** |
| `.name'intrinsic` | `ExprKind::TakeIntrinsic` | 内部绑定层 | 绑定编译器/目标/宿主原语；**仅限六内置包底层原语绑定文件使用，用户源码严禁书写** | **已接通且强制隔离** |

#### 3.8.2 Take View（`expr ' id`）语法准入四原则

为防止 `expr'id` 退化成无约束的“伪方法/反射杂烩”，任何新 Take View 必须同时满足以下四条原则：

1. **不可由普通方法/关联函数替代原则**：只有当操作需要语言核心级静态介入——例如不移动 `Once` 所有权观察（`'type`）、跨函数前后状态快照（`'old`）、编译器 `inline for` 静态展开协议（`'tuple_indexed`/`'tuple_enumerate`）、底层可变切片位置写回（`'bitvec`）或编译时声明元数据读取（`'attribute`）——才准进入 `expr'id`；
2. **严格单次求值与所有权完备原则**：Take View 操作数至多求值一次，且若其仅观察元数据而不产生消费移动，则不得影响被观察值的 `Once` / 借用合法性；
3. **不与 `Type.*` / `meta.reflection` 重叠原则**：一切类型级布局、字段名列表、变体路径与尺寸查询必须留在 `Type.*` / `meta` 反射接口中；
4. **不与 `builtin.mem` / `std` 值方法重叠原则**：任何可在受控库或 `unsafe` 模块通过普通函数完成的内存/字节重解释，不得占用全局 `expr'id` 关键字空间。

#### 3.8.3 候选 Take View 扩展裁定表

| 候选写法 | 意图 | 裁决结论 | 详尽论证与推荐替代路径 |
|---|---|---|---|
| `'field_names` | 取得结构体字段名称列表 | **拒绝进入 Take View** | 已有完备的 `Type.struct_field_names(T) -> meta.List<Symbol>` 与 `Type.enum_variant_field_names(T)`（`library/meta/reflection/structure.fl`）。在值上额外开 `'field_names` 既污染语法，又模糊“类型级静态信息”与“运行时值观察”的边界。 |
| `'as_bytes` | 按裸字节切片读取任意值 | **拒绝进入 Take View** | 不经验证将含 padding、指针或未对齐字段的结构体强转为字节切片会引入未定义/未初始化内存泄露，破坏安全边界；应使用 `BitVec` 切片（`expr'bitvec`）或 `builtin.mem` `unsafe` 原语 / 显式 Tier 2 序列化协议，不提供隐式万能 `'as_bytes`。 |
| `'snapshot` | 捕获当前状态的不可变逻辑副本 | **拒绝进入 Take View** | 在函数后置条件中已有 `'old` 覆盖入口快照；在一般算法中状态复制应显式调用 `Clone` 或构造值拷贝；若指程序验证规格层幽灵状态快照，属于 `verification` 幽灵契约内部原语而非通用运行期 `expr'id`。 |
| `'variant_index` / `'discriminant` | 取得 `enum` 运行时判别序号或底层标签整数 | **暂不新设 `'id`，建议由 `attrs.layout` / 反射方法替代（附 OQ-7）** | 普通 `enum` 的判别码分布属编译器优化与 ABI 隐私细节（如 niche 折叠后可能无独立紧凑整型下标）；需要精确可控数字编码的枚举已经支持固定位宽位表示，可直接用 `e'bitvec`（或 `Type.enum_bit_values`）与派生方法观察。仅在 FFI/底层明确需要稳定标签读数时，由声明侧显式配置 `attrs` 并暴露普通转换方法，避免任意枚举依赖未契约化的整数编号。 |
| `'dst_deref` | 对带元数据胖指针/DST 执行受控解引用或层叠指针提取 | **与 `'multi_deref` 合并裁定（OQ-7）** | 目标应与现存 HIR 节点 `ExprKind::MultiDeref`（`'multi_deref`）统一审视：若需要多级连续指针穿透视图保留 `'multi_deref` 并补全 elaboration；对于宽指针 DST 本身，Flurry 指针体系偏向直接 `.deref` / `.*` + 显式切片元方法，不再另起 `'dst_deref` 分叉关键字。 |

#### 3.8.4 `views.rs` 与 `docs/others/view-taking.md` 对齐修复要点

1. 补充 [view-taking.md](file:///home/fin/luna/docs/others/view-taking.md)，纳入已实现但遗漏的 `'attribute` 条目（含 `--emit merged-attributes` 示例与“仅限 comptime，不消耗目标”契约），以及明确 `'multi_deref` 的最终去留决策。
2. 修复 `'tuple_enumerate` 在 `compiler/elaboration/src/program/expression_values/views.rs:194` 遇到 `comptime inline for` 混用已计算型别与运行时绑定元组时的报错（使依赖对 `((T, seed), flag)` 形式在全部 CT/RT 组合中稳定工作）。

---

### 3.9 统一 `language-items.conf`、CLI 选择器与 `attrs` 官方键清单

#### 3.9.1 `library/language-items.conf`（建议完整模式协议段）

编译器**严禁**按字符串硬编码上述任何 trait 或结构体名称，所有模式视图均须在 `library/language-items.conf` 建立声明身份绑定（并按既有规范提供对应 `--*-protocol` CLI 选择器，确保独立最小测试可用自定义替代模块验证通用性）：

```ini
# 既有绑定（保持兼容）
list-pattern-protocol = builtin.protocols.patterns.ListPatternView
comptime-list-pattern-protocol = meta.data.patterns.ComptimeListPatternView
stable-list-pattern-protocol = builtin.protocols.patterns.StableListPatternView

# 本规格新增模式协议绑定
comptime-stable-list-pattern-protocol = meta.data.patterns.ComptimeStableListPatternView
list-pattern-endless-protocol = builtin.protocols.patterns.ListPatternViewEndless
stable-list-pattern-endless-protocol = builtin.protocols.patterns.StableListPatternViewEndless
comptime-list-pattern-endless-protocol = meta.data.patterns.ComptimeListPatternViewEndless
comptime-stable-list-pattern-endless-protocol = meta.data.patterns.ComptimeStableListPatternViewEndless

map-pattern-protocol = builtin.protocols.patterns.MapPatternView
stable-map-pattern-protocol = builtin.protocols.patterns.StableMapPatternView
comptime-map-pattern-protocol = meta.data.patterns.ComptimeMapPatternView
comptime-stable-map-pattern-protocol = meta.data.patterns.ComptimeStableMapPatternView

struct-pattern-protocol = builtin.protocols.patterns.StructPatternView
stable-struct-pattern-protocol = builtin.protocols.patterns.StableStructPatternView
comptime-struct-pattern-protocol = meta.data.patterns.ComptimeStructPatternView
comptime-stable-struct-pattern-protocol = meta.data.patterns.ComptimeStableStructPatternView

range-end-type = builtin.protocols.patterns.RangeEnd
range-pattern-protocol = builtin.protocols.patterns.RangePatternView
stable-range-pattern-protocol = builtin.protocols.patterns.StableRangePatternView
comptime-range-pattern-protocol = meta.data.patterns.ComptimeRangePatternView
comptime-stable-range-pattern-protocol = meta.data.patterns.ComptimeStableRangePatternView

concat-pattern-protocol = builtin.protocols.patterns.ConcatPatternView
stable-concat-pattern-protocol = builtin.protocols.patterns.StableConcatPatternView
comptime-concat-pattern-protocol = meta.data.patterns.ComptimeConcatPatternView
comptime-stable-concat-pattern-protocol = meta.data.patterns.ComptimeStableConcatPatternView

tuple-pattern-protocol = meta.patterns.TuplePatternView
type-pattern-protocol = meta.patterns.TypePatternView
```

#### 3.9.2 `attrs` 官方 `__flurry_*` 键增量需求（移交 WL-06 统一注册）

以下键由 `attrs` 模块构造方法产生，严禁裸写裸 key（如 `^{ for_match_arms: true }`）：

| `attrs` 公开入口 | 生成的内部键与值域 | 消费阶段 | 用途 |
|---|---|---|---|
| `attrs.quote_fn_template.for_match_arms` | `__flurry_quote_fn_for_match_arms: true` | Elaboration（`match { use ... }`） | 授权 `quote fn` 在 `match` 主体内以 arm 模板形式展开 |
| `attrs.quote_fn_template.allow_non_exhaustive_instance` | `__flurry_quote_fn_allow_non_exhaustive: true` | Elaboration（`match` 覆盖率） | 声明模板自身允许产出非穷尽片段，依赖外层实例化 `match` 补充检查 |
| `attrs.pattern.stable`（可选） | `__flurry_pattern_stable: true` | `pattern_automata` | 显式标注用户自定义自由/成员 `case` 满足观察幂等性可进入 `DecisionDag` |
| `attrs.layout.enum_base_type(T)`（演进建议） | `__flurry_enum_base_type: T` | Layout & BitVec Pattern | 替代 `enums.md` 旧式 `.base_type = T` 类内配置 |
| `attrs.layout.enum_discrete_tag(flag)`（演进建议） | `__flurry_enum_discrete_tag: flag` | Layout & BitVec Pattern | 替代旧式 `.discrete_tag = bool` 类内配置 |
| `attrs.layout.enum_dst_tag(flag)`（演进建议） | `__flurry_enum_dst_tag: flag` | Layout & Type | 替代旧式 `.dst = bool` 类内配置 |

---

## 4. 排期优先级与分阶段交付路线

### 4.1 分阶段里程碑与依赖拓扑

1. **P0（元编程基建与依赖模式核心闭合）**：
   - 包含条目：`WL-02-13`（`meta.patterns` CMTT 协议载体）、`WL-02-14`（CT `TuplePatternView` 驱动器）、`WL-02-18`（依赖 NF 值模式 + `TypePatternView`）。
   - 前置依赖：CMTT `< Ψ |- T >` 局部帧绑定生成与逃逸防护、通用依赖上下文反向模式求值（`pattern_checking/mod.rs`）。
   - 交付目标：完成异构容器与 `(storage, NodeIndex<storage>)` / `(_, index)` 依赖对的端到端编译与运行时资源安全验证。
2. **P1（主流视图补全、函数模板与既有缺口收口）**：
   - 包含条目：`WL-02-02`、`WL-02-05`、`WL-02-06`、`WL-02-08`、`WL-02-10`、`WL-02-11`、`WL-02-16`、`WL-02-17`、`WL-02-19`、`WL-02-21`、`WL-02-22`、`WL-02-23`、`WL-02-24`、`WL-02-25`。
   - 前置依赖：P0 部分完成后可并行推进；`WL-02-08` 依赖 WL-01 确定区间相关核心边界类型；`WL-02-10`/`WL-02-11` 需先在 `parse::operators` 注册模式 `++` 并添加 `hir::PatternKind::Concat`；`WL-02-21` 需与 WL-05 / WL-06 对齐 `quote_fn_template` 属性键。
   - 交付目标：消灭源码中既有的 `"not implemented"` 断层（`records.rs` rest `..`、`PatternKind::Ref`、`inline for` `'tuple_enumerate`、`'multi_deref`），落地 `Map`/`Range`/`Concat`/`Struct` 视图、`case` 额外参数与 DAG 分级、以及 `match { use template }`。
3. **P2（标准容器接入、无限序列与函数级 `case`）**：
   - 包含条目：`WL-02-01`（定长数组 `ListPatternView` 与 default `rest` 静态约束）、`WL-02-03`（`Endless` 双相协议）、`WL-02-04`（`VecDeque` 环形视图）、`WL-02-07`（`HashMap`）、`WL-02-09`（CT 离散区间格）、`WL-02-12`（`builtin.TextBytes.slice` + `Splice for str` + `str`/`String`/`Slice<u8>` `ConcatPatternView`）、`WL-02-15`（外部 Tier 2/3 示例独立集成校验）、`WL-02-20`（同包开放 `fn foo(case ...)` 分发）。
   - 前置依赖：P1 上线稳定后展开。
4. **P3（Take View 长期收口与跨协议决策图优化）**：
   - 包含条目：`WL-02-26`（基于第 5 节裁决结果处理候选扩展），以及 `pattern_automata` 对 `ConcatPatternView` 前缀 Trie 共享、`MapPatternView` 键决策共享的多协议联合决策树优化。

### 4.2 关键验收测试大纲（含正例、负例与资源安全检查）

| 里程碑 | 核心验收用例范围（必须经真实 `--sysroot library` 验证） |
|---|---|
| **P0** | 1. **依赖对正例**：`(storage, index)` 与 `(_, index)` 对 `Once` 或带运行时身份 `Arena` 的解构；`(_, index)` 跨 `Arena` 使用必须类型拒绝；<br>2. **`TuplePatternView` 事务与资源正负例**：`MatchPreparation.try_bind` 在臂 1 失败回退至臂 2 时不重复释放已移动变量；`CodeBinding` 返回出 `tuple_pattern_view_finish` 后进入外层作用域报 `DisallowedContextEscape`；<br>3. **依赖 NF 值模式**：`Buffer<'N, 'buf>` 绑定 `N` 后正确检查 `buf: Array<u8, N>`，实参不匹配触发拒绝。 |
| **P1** | 1. **`MapPatternView` & `StructPatternView`**：`MiniObj` 正常匹配 `{ code: x, active: true, .. }`；查询未定义字段 `unknown` 因 `NoReturn` 静态拒绝；`Point { x, .. }` 在有 `Drop` 结构体上局部移走非 `Copy` 字段必须拒绝；<br>2. **`RangePatternView` & `ConcatPatternView`**：5 种边界形式精确路由；`str` & `String` 链式 `"/api/" ++ path ++ ".json"` 合法且借用返回 `str` 不逃逸出 `String` 作用域；`left ++ right` 无定界拼接在 `str` 上必须编译拒收；<br>3. **Quoted Arms & `case`**：缺失 `for_match_arms` 属性的普通 `quote fn` 在 `match { use ... }` 处拒绝；`PatternKind::Ref`（`ref pat`）禁偷取 `Once` 所有权；<br>4. **Take View & `BitVec`**：`'bitvec` `0x (hi: BitVec<8>) (lo: BitVec<8>)` 高低位切片顺序在大小端目标输出一致；`inline for` 混合元组与 `'multi_deref`（按决策）全绿。 |
| **P2** | 1. **`VecDeque` 折返环切片**：先 `push_back` + `pop_front` + `push_back` 形成内存折返后再用 `[a, b, ...rest]` 匹配 `VecDeque` 并校验 `rest`；<br>2. **`Endless` 协议**：`FibStream` 通过 `[0, 1, c, ..._]`；缺少 `..._`/`...rest` 的 `[a, b]` 编译期拒绝；同时 impl `ListPatternView` 与 `ListPatternViewEndless` 报歧义拒绝；<br>3. **`str`/`meta.List` `Concat` + Tier 2/3 示例**：利用完整 sysroot 外部包编译运行 `RegexMatcher`（`("(a)(b)?", first, second)` 正例 + 本规格 4 项负例）；<br>4. **函数级 `case`**：同包跨文件拆分两臂 `eval(case ...)` 覆盖联合检查，遗漏变体触发包级非穷尽报错。 |
| **P3** | 1. 多臂共享前缀 `"/v1/" ++ a` 与 `"/v1/" ++ b` + `StableConcatPatternView` 经 NIR/LLVM 核查仅做单次前缀比对；<br>2. 候选 Take View（如 `'variant_index`）按裁决确认不在核心语法污染或保留唯一正交入口。 |

---

## 5. 待作者裁决的语义空缺与冲突（Open Questions）

### OQ-1：`TuplePatternView` 与 `TypePatternView` 的归属包层级（`meta.patterns` vs `builtin` 重导出）
- **冲突来源**：`AGENTS.md` 与 `library/STRUCTURE.md` 确立了六包由底层到高层的依赖序 `builtin -> meta -> attrs -> build -> std -> verification`；`library/builtin/types/tuple.fl:6` 已存在对 `meta.List<Type>` 的跨包引用；而 `TuplePatternView`（`proposal_trace/2026-9-7-pattern-traits.fl:142`）在签名中直接依赖 `meta.patterns.{CodeBinding, ExprCode, PatternInput, MatchPreparation}`。
- **核心问题**：若将 `TuplePatternView` 定义在 `builtin.protocols.patterns`，会导致 `builtin` 深度反向依赖 `meta` 的 CMTT 基础设施；若定义在 `meta.patterns`，仅使用默认内置包的用户在书写自定义 `TuplePatternView` 时需显式 `use meta.patterns.TuplePatternView`（或依赖 `meta.prelude`）。
- **推荐方案**：将 `TuplePatternView`、`TypePatternView` 及其 CMTT 协议载体全部定义为 **`meta.patterns` 的一级成员**，由 `language-items.conf`（`tuple-pattern-protocol = meta.patterns.TuplePatternView`）显式绑定编译器协议身份（无需在 `builtin` 倒挂引用 `meta` 的复杂元编程数据结构）。
- **备选方案**：在编译器六内置包协同编译前提下保留 `meta.patterns` 为唯一真定义，仅在 `builtin.prelude` 中通过 `pub use meta.patterns.TuplePatternView` 提供别名（前提是确认不会造成编译构建 DAG 环形失效）。

### OQ-2：`ListPatternView` / `ListPatternViewEndless` 未重写 `rest` 方法时的默认错误检查机制
- **冲突来源**：`library/builtin/protocols/patterns.fl:12-14` 目前用 `fn list_pattern_view_rest(*self, comptime start: usize) -> Rest { while true {} }` 表达可选性；但在无限序列 `ListPatternViewEndless` 和有限序列 `...rest` 绑定下，若类型实现者遗漏覆盖 `rest` 方法而用户写出了 `[head, ...rest]`，当前做法只在运行时卡死进死循环，极不安全。
- **推荐方案**：将带剩余切片绑定能力的视图拆出独立细化标记/子 trait，或在编译期由模式 elaboration 直接检测目标类型实现的 `list_pattern_view_rest` 是否继承自协议默认占位体（通过声明身份或 `^attrs` 标记），一旦用户在臂中写了 `...rest`（绑定非 `_` 标识符）即触发**编译期错误** `"type implements ListPatternView without overriding list_pattern_view_rest required for '...rest' binding"`，同时 `..._` 仍无需覆盖 `rest`！
- **备选方案**：将 `list_pattern_view_rest` 设为必需方法（移除默认实现），或单独新增 `trait RestListPatternView :- ListPatternView` 专供需要 `...rest` 切片的类型实现。

### OQ-3：`MapPatternView` 匹配时省略 `..` 的语义（开放默认 vs 显式 `..` 强制）
- **冲突来源**：[pattern-matching/index.md](file:///home/fin/luna/docs/control-flow/pattern-matching/index.md) 对具名 `struct` 与闭记录要求列出全部字段或加 `..`；`proposal_trace/2026-9-7-pattern-traits.fl` 示例中则直接在 `MapPatternView` 中写出开放子集；而 `records.rs:53` 当前尚未实现记录 `..`。
- **推荐方案**：区分**定域 `StructPatternView`** 与**开放 `MapPatternView`**：
  - `StructPatternView` 与普通 `struct`/匿名记录遵循闭世界规则：省略某字段**必须显式写 `..`**（否则报缺失字段错）；
  - `MapPatternView` 若写了 `{ a: x, .. }` 恒合法；当省略 `..`（写成 `{ a: x }`）时，建议在未声明封闭键集合的开放映射上给出编译错误或要求写 `{ a: x, .. }`，以明确区分“映射必仅此一键”与“忽略其余未知键”！
- **备选方案**：约定所有 `MapPatternView` 类型天然视为开放键值集合，`{ a: x }` 默认等价于 `{ a: x, .. }`（但会丧失将来对 schema 封闭映射做未列键诊断的精确性）。

### OQ-4：在 `comptime` 上下文匹配自定义 `TuplePatternView` 主语时的双相策略
- **冲突来源**：[the-comptime-phases-of-traits.md](file:///home/fin/luna/docs/comptime/the-comptime-phases-of-traits.md) 规范指出 runtime 与 comptime 分别使用普通 `trait` 与同名方法 `pure comptime trait Comptime*`；而 `TuplePatternView` 的 3 个方法本身已是 `comptime fn` 并通过 `MatchPreparation` 生成引用 runtime 变量的 `< |- T >` 报价（`2026-9-7-pattern-traits.fl:142-146` 注明不自动推导纯 CT 主语匹配）。
- **推荐方案**：**复用意图方案（CT VM 解释残余代码）**——当主语是 `comptime` 值（例如在 `comptime fn` 中匹配一个 comptime `TripleBox` 或 `GraphAnchor`）时，编译器依然驱动同一 `TuplePatternView` 生成单臂残余 HIR/IIR 匹配块，然后直接由 CT VM 立即求值该残余块；只要里面调用的运行时辅助函数同时存在 `pure comptime` 版本（双相覆盖），无需重写第二套 `ComptimeTuplePatternView` 即可工作！
- **备选方案**：在 `meta.patterns` 另建 `pure comptime trait ComptimeTuplePatternView`（使用 `comptime` 值而非 `CodeBinding` / `< |- T >`），但代价是每个元组视图开发者必须维护两套截然不同的状态机代码。

### OQ-5：无载荷谓词 `case` 的返回签名（`-> bool` vs `-> ?Tuple<>`）
- **冲突来源**：[pattern-matching/index.md](file:///home/fin/luna/docs/control-flow/pattern-matching/index.md) 明确标明“无载荷谓词 `case` 返回 `-> bool` 还是统一使用 `-> ?Tuple<>` 尚属待定”；而现行编译器 `compiler/elaboration/src/program/pattern_checking/cases.rs` 强制要求返回值类型为 `?Tuple<...>`（可为空元组 `()`）。
- **推荐方案**：**同时支持 `-> bool` 谓词 `case`（等价于成功返回 `()` 失败返回 `null`）**：写 `case is_even(*self) -> bool` 远比 `-> ?Tuple<> { if ... { () } else { null } }` 自然得多，且编译器仅需在 `cases.rs` 统一将 `bool` 检查与 `?Tuple<...>` 检查归并为同一 `Observe` 判别分支，不会破坏有载荷 `-> ?Tuple<A, B>` 规则。
- **备选方案**：坚持单一返回形式 `-> ?Tuple<...>`（0 元时写 `-> ?()`），避免签名分支。

### OQ-6：函数级 `case`（`fn foo(case ...)`）同包多臂重叠时的静态消解规则
- **冲突来源**：[pattern-functions.md](file:///home/fin/luna/docs/functions/pattern-functions.md) 规定函数级 `case` 收集全包内分散在各文件的同名定义且**与源码书写顺序无关**，要求编译期覆盖检查与一致的分配调度。若跨文件两条 `case` 臂的模式存在交集（例如一条写 `.node(a, _)`，另一条写 `.node(0, b)`），由于没有源码前后顺序，不能简单套用 `match` 的“先写先中”。
- **推荐方案**：**默认严格要求同包所有函数级 `case` 模式两两严格互斥（disjoint）**；遇到任意非空交集直接报错 `"overlapping case function arms across order-independent package collection"`，并提示改用显式精化条件或单一 `match` 表达式（明确把“顺序优先匹配”留给 `match`，把“可独立推理的开放互斥分派”留给 `fn foo(case ...)`）。
- **备选方案**：引入基于模式“特化偏序（more-specific pattern first）”的自动消解规则（若仅能严格比对子结构偏序则允许特例覆写，不可比偏序仍报歧义），但这会显著增加交互覆盖可理解性复杂度。

### OQ-7：Take View `'multi_deref` / `'variant_index` 的最终去留与 `BitVec` `++` 语法统一
- **冲突来源**：
  1. `compiler/ast_lowering/src/expr.rs` 与 `compiler/hir/src/expr.rs` 已内置解析与降低 `ExprKind::MultiDeref`（`'multi_deref`），但 `compiler/elaboration/src/program/expression_values/{mod,views,places}.rs` 全部未接通；同时 `docs/others/view-taking.md` 未提及 `'multi_deref`（也遗漏了已实现的 `'attribute`）。
  2. `BitVec<N>` 上的 `(hi : BitVec<8>) ++ (lo : BitVec<24>)` 改变了剩余位宽（`N -> N - 8`），无法直接塞进单类型参数固定的 `ConcatPatternView`（`assoc Rest: Type`）。
- **推荐方案**：
  1. 对 `'multi_deref`：**保留并在 P1 补齐 elaboration 语义（按声明或安全层级穿透指针链，禁止越过 `Once` 所有权界线）OR 若作者确认“只用显式多次 `.*` 不再引入隐式多层解引用”则从 HIR 彻底删去 `'multi_deref` 死节点**——推荐明确确认前者或后者，避免半吊子 HIR 悬空；同时**拒绝**将 `'field_names`、`'as_bytes`、`'snapshot` 加入 Take View，`enum` 标签读数优先通过固定位宽 `'bitvec` 与 `Type.*` 反射承担，不单独污染 `'variant_index`；
  2. 对 `BitVec<N>` 的 `++` 模式：**保留由编译器内建位代数（第 3.7 节）处理**，因为切分点宽度直接进入静态依赖返回类型 `BitVec<N - M>`，无需强行扭曲 `ConcatPatternView` 去承载值依赖关联类型计算；两套机制共享模式 `++` 语法并在 elaboration 按主语是否为 `BitVec<N>` 正交分派。
- **备选方案**：若未来引入类型级高阶/依赖关联类型计算，也可在 `ConcatPatternView` 上扩展依赖签名 `assoc Rest: pure comptime fn(PartType) -> Type`，以统一 `BitVec<N>` 与切片序列。
