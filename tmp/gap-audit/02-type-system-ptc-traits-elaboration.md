# 领域 02 差距审计：类型系统、PTC、Traits、Derivation 与 Elaboration

> **审计对象**：`compiler/ptc`、`compiler/type_resolution`、`compiler/middle`、`compiler/interface`、`compiler/elaboration`（核心类型检查、依赖输入、特质派发、衍生展开与局部精化管线），以及关联的 `compiler/nir` 契约层、`compiler/materialization`、`compiler/luna` 前端驱动与 `library/` 六内置包接口
> **审计基线**：本地仓库 `HEAD` 提交 `b9a52238`
> **审计性质**：静态源码与契约差距审计（严格依据现场源码、测试文件与规范文档，未执行 `cargo build` 或 `cargo test`）

---

## 1. 审计范围、检查基线与设计标尺

### 1.1 检查范围与模块规模

本报告聚焦 Flurry 编译器的中端语义核心（领域 02），实盘核查以下非测试源码（`#[cfg(test)]` 与独立 `tests.rs` / `*_tests.rs` 另行统计）及对应验收测试：

| Crate / 模块 | 非测试 `.rs` 文件数 | 非测试源码行数 | 关联测试模块 / 集成测试代表 |
|---|---:|---:|---|
| [compiler/ptc/src](file:///home/fin/luna/compiler/ptc/src/lib.rs) | 38 | 8,347 | `elaboration/tests/{ptc,ptc_integer,ptc_kinds}.rs` |
| [compiler/type_resolution/src](file:///home/fin/luna/compiler/type_resolution/src/lib.rs) | 39 | 17,683 | `type_resolution/tests/{inference,implementation,implementation_phases,capabilities,calls,matching,builtin_ownership}.rs` |
| [compiler/middle/src](file:///home/fin/luna/compiler/middle/src/lib.rs) | 4 | 363 | 内联 compile-fail doctests (`hir_package.rs`, `queries.rs`) |
| [compiler/interface/src](file:///home/fin/luna/compiler/interface/src/lib.rs) | 2 | 239 | `luna` 前端多包集成测试 |
| [compiler/elaboration/src](file:///home/fin/luna/compiler/elaboration/src/lib.rs) | 293 | 134,536 | `elaboration/tests/*.rs`（34 个）及 `luna/tests/{runtime_dependent_types,derivation,trait_phases,type_spreads,type_reflection_layout,predicate_calls,...}.rs` |
| **合计** | **376** | **161,168** | — |

核查的 `elaboration/src` 重点子模块包括：
- 入口与发布层：[lib.rs](file:///home/fin/luna/compiler/elaboration/src/lib.rs)、[definitions/](file:///home/fin/luna/compiler/elaboration/src/definitions/struct_publisher.rs)、[key_protocols.rs](file:///home/fin/luna/compiler/elaboration/src/key_protocols.rs)（注：`gradual` 与 `type_computations` 处于 `#[cfg(test)]` 归档状态）。
- `program/` 核心子系统：`trait_inputs/`、`derivation/`、`interfaces/`、`dynamic_instances/`、`refinements/`、`normal/`、`source_declarations/`、`builtin_bindings/`、`constants/`、`declaration_inputs/`、`clause_inputs/`、`dependent_inputs/`、`block_regions/`、`projections.rs`、`type_data.rs`、`type_defaults.rs`、`call_selection.rs`、`open_functions.rs`、`runtime_origins.rs`、`runtime_defaults.rs`、`runtime_environment.rs`、`closure_bodies.rs`。
- 跨边界支撑文件：[compiler/nir/src/{types,layout,program,normal_forms}.rs](file:///home/fin/luna/compiler/nir/src/types.rs)、[compiler/materialization/src/lib.rs](file:///home/fin/luna/compiler/materialization/src/lib.rs)、[compiler/luna/src/{packages,frontend,package_cli}.rs](file:///home/fin/luna/compiler/luna/src/frontend.rs)、[library/language-items.conf](file:///home/fin/luna/library/language-items.conf) 与 `library/{builtin,meta,std}`。

### 1.2 设计标尺与资料权威边界

本审计严格区分 **Core 核心规范目标** 与 **Wishlist 扩展目标**：

1. **Core 核心规范目标**（具有强制一致性要求，违背即定为 `⚠️ Divergent`）：
   - 作者确认与 [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5 语义边界（无 `Option`/`Result` 枚举、点路径、集中 `.symbol'builtin` 绑定、六内置包孤儿规则豁免、禁止针对 `Tuple` / `meta.List<Type>` 等具体定义设特判、NF 头不展开 `Identity<T> != T`、优先 `Tuple<.Es = Es>`、PTC 零程序状态泄露、双相优先纯 `comptime` trait 等）；
   - `docs/type/`：[atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md)、[solver-protocol.md](file:///home/fin/luna/docs/type/propositional-constraints/solver-protocol.md)、[occurrence-typing.md](file:///home/fin/luna/docs/type/propositional-constraints/occurrence-typing.md)、[theory-solvers.md](file:///home/fin/luna/docs/type/propositional-constraints/theory-solvers.md)、[runtime-dependent-type.md](file:///home/fin/luna/docs/type/runtime-dependent-type.md)；
   - `docs/traits/`：[coherence.md](file:///home/fin/luna/docs/traits/coherence.md)、[derivation.md](file:///home/fin/luna/docs/traits/derivation.md)、[dynamic-interfaces.md](file:///home/fin/luna/docs/traits/dynamic-interfaces.md)、[implementation-resolution.md](file:///home/fin/luna/docs/traits/implementation-resolution.md)；
   - `docs/compilation/`：[elaboration.md](file:///home/fin/luna/docs/compilation/elaboration.md)、[candidate-solving.md](file:///home/fin/luna/docs/compilation/candidate-solving.md)、[local-dependencies.md](file:///home/fin/luna/docs/compilation/local-dependencies.md)、[runtime-dependent-types.md](file:///home/fin/luna/docs/compilation/runtime-dependent-types.md)；
   - [issues/0001-runtime-dependent-origin-and-default-param.md](file:///home/fin/luna/issues/0001-runtime-dependent-origin-and-default-param.md) 记录的已确认闭包 ABI 边界。
2. **Extended Wishlist 扩展目标**（代表面向 Silver/Tier-2 库生态的后继演进需求，不与 Core 混淆）：
   - `tmp/wishlist/01`、`tmp/wishlist/03`、[tmp/wishlist/05-meta-cmtt-macros-reflection.md](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md)。
3. **非权威历史材料及勘误**：
   - 本审计不引用 `proposal_trace/`、`OLD-2024/` 或 `flurry_tutorial/` 作为语义依据。
   - **重要勘误**：历史日志 [ai_dev_logs/ptc_atoms_status_report_2026-11-21.md](file:///home/fin/luna/ai_dev_logs/ptc_atoms_status_report_2026-11-21.md#L19) 声称 `nir::TypeKind` 仅有 9 个变体且缺失 `struct`/`enum`/`newtype`；经现场核对 `HEAD` `b9a52238`，[compiler/nir/src/types.rs](file:///home/fin/luna/compiler/nir/src/types.rs#L146-L175) **已经**实现 `NominalKind::{Struct, Enum, Newtype}` 与 `TypeKind::Nominal(NominalKind)`（共 10 个变体），且 [projections.rs](file:///home/fin/luna/compiler/elaboration/src/program/projections.rs#L156-L172) 已支持 `is_struct` / `is_enum` / `is_newtype` 共 12 个谓词；但仍缺 `Union`、`Record`、`Module`、`Char` 与其余 15 个 PTC 原子。

---

## 2. 成熟度总览与统计表

### 2.1 六级成熟度矩阵统计（共 32 项 `GAP-02-01` .. `GAP-02-32`）

| 规范归属 | `🟢 Full` | `🟡 Partial` | `🟠 Frontend-Only` | `🔴 Missing` | `⚠️ Divergent` | `⚪ Deferred` | 小计 |
|---|---:|---:|---:|---:|---:|---:|---:|
| **Core 核心规范**（`docs/` + `AGENTS.md` + `issues/0001`） | 4 | 13 | 1 | 6 | 3 | 0 | **27** |
| **Extended Wishlist**（`tmp/wishlist/{01,03,05}`） | 0 | 1 | 0 | 3 | 0 | 1 | **5** |
| **合计** | **4** | **14** | **1** | **9** | **3** | **1** | **32** |

按技术子域细分 Core 与 Wishlist 成熟度分布：

| 子域 | `🟢 Full` | `🟡 Partial` | `🟠 Frontend-Only` | `🔴 Missing` | `⚠️ Divergent` | `⚪ Deferred` | 关键结论 |
|---|---:|---:|---:|---:|---:|---:|---|
| **A. PTC 原子、求解器与 Type 反射**（`01..07`） | 0 | 6 | 0 | 1 | 0 | 0 | 理论求解器内核健全，但原子覆盖仅 4/19 且缺 `IntLe -> NIR`、缺 `is_not_sized` 与布局/存在性/行谓词 |
| **B. 特判清理与 NF 通用语义**（`08..13`） | 2 | 1 | 0 | 2 | 1 | 0 | `Identity<T> != T` 与 `library/` 合规；但 `Tuple`/`meta.List` 硬编码特判严重违反 `AGENTS.md` §5 |
| **C. Traits、Coherence、Derivation 与 `dyn Trait`**（`14..20`） | 2 | 1 | 1 | 2 | 1 | 0 | Coherence/Orphan/6 包豁免扎实；固有 `comptime impl` 与 `^attrs.comptime_phase` 优先级断裂，`dyn Trait` 止步于 AST |
| **D. 运行时依赖类型、闭包 ABI 与 `where` 局部推断**（`21..24`） | 0 | 2 | 0 | 1 | 1 | 0 | `DefaultParam` 落地可用，但泛型/方法体内闭包的隐式 `RuntimeId` 交付与语义捕获边界混同（`issues/0001`） |
| **E. Occurrence Typing、局部依赖与 `middle`/`interface`**（`25..27`） | 0 | 3 | 0 | 0 | 0 | 0 | `if`/`when`/`match`/短路精化扎实，`K::Loop` 缺不动点；区域续体稳定但缺统一 `Goal`/`WaitTicket`；`middle` 仅 1 query |
| **F. Extended Wishlist 扩展（`28..32`）** | 0 | 1 | 0 | 3 | 0 | 1 | `RecordInfo`/`type_id`/短命名统一/`stride_of`/`VariadicCollector` 待补；`verification` 按决策暂缓 |
| **合计** | **4** | **13+1=14** | **1** | **6+3=9** | **3** | **1** | — |

### 2.2 `SND` / `CMP` / `ROB` 严重度统计总表

对应第 6 节逐项审计记录（`SND-02-01..05`、`CMP-02-01..06`、`ROB-02-01..05`）：

| 审计维度 | `Critical` | `High` | `Medium` | `Low` | 合计 |
|---|---:|---:|---:|---:|---:|
| **健全性（`SND-02-NN`）** | 0 | 2 | 2 | 1 | **5** |
| **完备性（`CMP-02-NN`）** | 0 | 3 | 2 | 1 | **6** |
| **鲁棒性（`ROB-02-NN`）** | 0 | 2 | 2 | 1 | **5** |
| **合计** | **0** | **7** | **6** | **3** | **16** |

> 注：本领域尚无 `Critical` 级已编译后端静默错误（例如 `issues/0001` 闭包环境未交付问题当前在 `materialization` 被拦截报错而未产生静默 UB；`ptc` 在未知理论前保持保守返回 `Unknown`），但存在 **7 项 High** 级语义隐患、正交功能缺口与高密度 panic 表面。

---

## 3. 全量特性差距矩阵（GAP-02-NN）

| ID | 归属 | 特性 / 契约 | 设计标尺 | 现网状态 | 成熟度 | 源码证据与测试现状 |
|---|---|---|---|---|---|---|
| `GAP-02-01` | Core | PTC 原子集合 A01–A19 覆盖与 NIR 接线 | [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md#L136-L154) | `ptc::Atom` 仅含 `TypeEqual`(A01), `Kind`(A02), `Trait`(A10), `IntLe`(A12)；`nir::TypePredicate` 仅 3 原子缺 `IntLe`，其余 15 个原子缺失 | `🟡 Partial` | [ptc/src/lib.rs](file:///home/fin/luna/compiler/ptc/src/lib.rs#L60-L93), [nir/src/program.rs](file:///home/fin/luna/compiler/nir/src/program.rs#L5035-L5043) |
| `GAP-02-02` | Core | PTC CDCL(T) / Nelson-Oppen 求解与零状态泄露 | [solver-protocol.md](file:///home/fin/luna/docs/type/propositional-constraints/solver-protocol.md), [theory-solvers.md](file:///home/fin/luna/docs/type/propositional-constraints/theory-solvers.md) | EUF 同余闭包、环检测、效应行、错误标签集、差分整数、CDCL+proof replay 就绪且零程序状态/堆指针泄露；但 11 个求解模块仅服务已接入的 4 类原子 | `🟡 Partial` | [ptc/src/lib.rs](file:///home/fin/luna/compiler/ptc/src/lib.rs#L1357-L1902), [effects.rs](file:///home/fin/luna/compiler/ptc/src/effects.rs#L1-L85), [error_results.rs](file:///home/fin/luna/compiler/ptc/src/error_results.rs#L1-L85), [integer.rs](file:///home/fin/luna/compiler/ptc/src/integer.rs#L1-L100), `ptc.rs`/`ptc_integer.rs` |
| `GAP-02-03` | Core | `Type.predicates.*` 谓词集合与类型分类覆盖 | [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md#L18-L32) | 仅接 12 个 `TypeKind` 谓词（含 `is_struct/enum/newtype`），缺 `is_union`/`is_anonymous_record`/`is_module`/`is_char` 等（`is_not_sized` 见 `GAP-02-06`；长短命名与 `wishlist/05` 差异见 `GAP-02-28`/`TD-02-02`） | `🟡 Partial` | [projections.rs](file:///home/fin/luna/compiler/elaboration/src/program/projections.rs#L156-L172), [nir/src/types.rs](file:///home/fin/luna/compiler/nir/src/types.rs#L146-L175), `elaboration/tests/ptc_kinds.rs` |
| `GAP-02-04` | Core | `Type.*` 布局/表示反射入口 | [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md), `type_reflection_layout.rs` | `type_layout_query_call` 仅处理 `size_of` 与 `align_of`；缺 `stride_of`、`bit_width_of`、`field_offset_of` 等 | `🟡 Partial` | [projections.rs](file:///home/fin/luna/compiler/elaboration/src/program/projections.rs#L244-L248), [nir/src/layout.rs](file:///home/fin/luna/compiler/nir/src/layout.rs#L18-L45), `luna/tests/type_reflection_layout.rs` |
| `GAP-02-05` | Core | `Type.vars.*` 编译期占位/模式标记 | `type_data.rs`, `wishlist/05` | 仅处理 `Type.vars.Closure`（对应 `Term::ClosureContextType`），缺 `EffectRow`、`ErrorRow`、`Reachability`、`Any` | `🟡 Partial` | [type_data.rs](file:///home/fin/luna/compiler/elaboration/src/program/type_data.rs#L155-L163) |
| `GAP-02-06` | Core | A11 `NotSized(T, target)` 与前置表示错误判定 | [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md#L28-L116) | 尚未定义 `Type.predicates.is_not_sized`，亦未将 comptime-only 类型的 `IllFormed` 前置检查接通谓词域 | `🔴 Missing` | [projections.rs](file:///home/fin/luna/compiler/elaboration/src/program/projections.rs#L156-L172), [ptc/src/lib.rs](file:///home/fin/luna/compiler/ptc/src/lib.rs#L60-L78) |
| `GAP-02-07` | Core | PTC 反模型构造（`NotEntailed` 证伪完备性） | [solver-protocol.md](file:///home/fin/luna/docs/type/propositional-constraints/solver-protocol.md) | 含 `Atom::Trait(_)` 时直接返回 `None` 退回 `Unknown`；仅列举已知种子及 3 类结构见证（Tuple/Pointer/Function），不生成新鲜 Nominal | `🟡 Partial` | [ptc/src/model_search.rs](file:///home/fin/luna/compiler/ptc/src/model_search.rs#L20-L80), [classification.rs](file:///home/fin/luna/compiler/ptc/src/classification.rs#L17-L71) |
| `GAP-02-08` | Core | 消除针对 `Tuple` 与 `meta.List<Type>` 的专用语言规则 | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5 | `BuiltinCallable::{List,Tuple}`、`Role::{Tuple,TypeList}`、专属变参收集分支与专用构造器判异跨 5 个 crate 硬编码 | `⚠️ Divergent` | [nir/src/normal_forms.rs](file:///home/fin/luna/compiler/nir/src/normal_forms.rs#L386-L420), [builtin_bindings.rs](file:///home/fin/luna/compiler/elaboration/src/program/builtin_bindings.rs#L78-L117), [normal.rs](file:///home/fin/luna/compiler/elaboration/src/program/normal.rs#L55-L110), [collections.rs](file:///home/fin/luna/compiler/elaboration/src/program/declaration_inputs/collections.rs#L110-L115), [program.rs](file:///home/fin/luna/compiler/elaboration/src/program.rs#L4219-L4256), [luna/src/protocols/type_constructors.rs](file:///home/fin/luna/compiler/luna/src/protocols/type_constructors.rs#L329-L460), [language-items.conf](file:///home/fin/luna/library/language-items.conf#L6-L7) |
| `GAP-02-09` | Core | Normal Form 头不展开与身份保留（`Identity<T> != T`） | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5 | `matching.rs` 显式禁止对未知 NF/未注册 nominal 反演，`ptc` 仅认可显式等式/构造器，`type_spreads.rs` 验证保留 `NormalForm` 头 | `🟢 Full` | [matching.rs](file:///home/fin/luna/compiler/type_resolution/src/matching.rs#L143-L144), [matching.rs](file:///home/fin/luna/compiler/type_resolution/src/matching.rs#L739-L741), `luna/tests/type_spreads.rs:98-118` |
| `GAP-02-10` | Core | 具名全列表赋值 `Tuple<.Es = Es>` 与 `...Es` 通用转发 | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5 | `Tuple<.Es = Es>` 与用户变参头 `.Es = Pass(es)` 均可直接转发全量表；但收集协议仍硬编码绑定 `meta.List<T>` 角色 | `🟡 Partial` | [clause_inputs.rs](file:///home/fin/luna/compiler/elaboration/src/program/clause_inputs.rs#L222-L255), `luna/tests/execution.rs:34475-34595`, `library/std/fmt_impl/parse.fl:306` |
| `GAP-02-11` | Core | 非 `Type` 返回域的 Normal Form 函数及动态应用 | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5 | 遇到结果非 `P::Type` 的动态 NF 调用直接报错 `"non-Type normal-form results are not implemented"` | `🔴 Missing` | [normal/dynamic_inputs.rs](file:///home/fin/luna/compiler/elaboration/src/program/normal/dynamic_inputs.rs#L81-L86) |
| `GAP-02-12` | Core | 依赖普通 `comptime fn` 输入的 NF / 词法标注与调用期求值 | `docs/compilation/elaboration.md` | 若 NF 标注、词法标注或 `required` 契约引用普通 `comptime fn` 未就绪形参，直接拒绝报错要求“invocation-time type elaboration” | `🔴 Missing` | [normal.rs](file:///home/fin/luna/compiler/elaboration/src/program/normal.rs#L331-L337), [lexical_inputs.rs](file:///home/fin/luna/compiler/elaboration/src/program/lexical_inputs.rs#L428-L447), [required/prepared.rs](file:///home/fin/luna/compiler/elaboration/src/program/required/prepared.rs#L573) |
| `GAP-02-13` | Core | 集中 `.symbol'builtin` 绑定与无 `Option`/`Result` 枚举规范 | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5 | `library/{builtin,meta}` 已采用集中绑定、`option.fl`/`result.fl` 仅提供 ops trait（无枚举）；遗留 `#[cfg(test)] mod gradual` 尚未物理清理 | `🟢 Full` | [tuple.fl](file:///home/fin/luna/library/builtin/types/tuple.fl#L6), [option.fl](file:///home/fin/luna/library/builtin/types/option.fl), [result.fl](file:///home/fin/luna/library/builtin/types/result.fl), [primitives.fl](file:///home/fin/luna/library/builtin/primitives.fl#L13) |
| `GAP-02-14` | Core | 特质分桶精度（`cut`/`index_depth`）、重复变量同秩与连贯性证明 | [coherence.md](file:///home/fin/luna/docs/traits/coherence.md) | `Key::precision()` 精确计算最浅开放 `Wildcard` 深度；同秩一对一用 PTC `check_type_coherence` 证明并支持 `verify_type_coherence` 重放 | `🟢 Full` | [implementation.rs](file:///home/fin/luna/compiler/type_resolution/src/implementation.rs#L310-L338), [coherence.rs](file:///home/fin/luna/compiler/type_resolution/src/implementation/coherence.rs#L479-L697) |
| `GAP-02-15` | Core | 孤儿规则有序局部身份与 6 内置包 `impl` 豁免 | [coherence.md](file:///home/fin/luna/docs/traits/coherence.md), [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5 | `ownership.rs` 按接口->Subject->参数有序查找局部身份；`frontend.rs`/`packages.rs` 将全部 6 个内置包纳入 `builtin_implementation_packages` 并向编译器 builtin 主体授予豁免 | `🟢 Full` | [ownership.rs](file:///home/fin/luna/compiler/type_resolution/src/implementation/ownership.rs#L30-L236), [implementation.rs](file:///home/fin/luna/compiler/type_resolution/src/implementation.rs#L615-L647), [frontend.rs](file:///home/fin/luna/compiler/luna/src/frontend.rs#L627-L662), [packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs#L1783-L1786) |
| `GAP-02-16` | Core | 双相特质优先级与 `^attrs.comptime_phase` / 固有 `comptime impl` | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5, `docs/traits/` | `pure comptime trait` 在 comptime 上下文享有优先级，但普通 trait 上 `^attrs.comptime_phase` 不授优先级（导致歧义）；固有 `comptime impl` 被直接拒绝 | `⚠️ Divergent` | [discovery.rs](file:///home/fin/luna/compiler/elaboration/src/program/interfaces/discovery.rs#L117-L248), [parents.rs](file:///home/fin/luna/compiler/elaboration/src/program/interfaces/parents.rs#L465-L478), [interfaces.rs](file:///home/fin/luna/compiler/elaboration/src/program/interfaces.rs#L447-L451), `trait_phases.rs:80-90,1021-1031` |
| `GAP-02-17` | Core | `derive fn` 基础设施与 `^attrs.derivation.default` 注册选择 | [derivation.md](file:///home/fin/luna/docs/traits/derivation.md) | 支持无 `where` 子句与常规参数的 `derive fn` 生成与 `attrs.derivation.default` 冲突/可见性注册选择 | `🟡 Partial` | [program.rs](file:///home/fin/luna/compiler/elaboration/src/program.rs#L2214-L2368), [attributes/derivation.rs](file:///home/fin/luna/compiler/elaboration/src/program/attributes/derivation.rs#L1-L71), `luna/tests/derivation.rs`, `derivation_attributes.rs` |
| `GAP-02-18` | Core | `derive fn` 7 项阻断（子句/`quote`/块内/计算选择/未使用检查等） | [derivation.md](file:///home/fin/luna/docs/traits/derivation.md) | 存在明确 `unsupported(...)` 拦截：策略 `clause_params`、块内 `derive`、动态策略名、泛型请求中的计算体、非常规参数标记、未使用策略独立检查、捕获属性环境 | `🔴 Missing` | [program/derivation.rs](file:///home/fin/luna/compiler/elaboration/src/program/derivation.rs#L39-L215), [program/derivation.rs](file:///home/fin/luna/compiler/elaboration/src/program/derivation.rs#L499-L505) |
| `GAP-02-19` | Core | `dyn Trait` 动态接口（vtable、`%c dyn Trait`、`Self` 约束） | [dynamic-interfaces.md](file:///home/fin/luna/docs/traits/dynamic-interfaces.md) | 仅 `ast`/`parse`/`fmt` 有 `NodeKind::TraitObjectType`；`ast_lowering`、`hir`、`nir`、`type_resolution`、`elaboration` 完全未接线 | `🟠 Frontend-Only` | [ast/src/lib.rs](file:///home/fin/luna/compiler/ast/src/lib.rs#L261), [parse/src/expressions.rs](file:///home/fin/luna/compiler/parse/src/expressions.rs#L346), `ast_lowering`/`hir`/`nir` 零命中 |
| `GAP-02-20` | Core | Trait 参数化/抽象关联值与循环父接口契约 | [implementation-resolution.md](file:///home/fin/luna/docs/traits/implementation-resolution.md) | 参数化与抽象关联值、循环父接口契约均明确报错 `"not yet implemented"` | `🔴 Missing` | [interfaces/associated.rs](file:///home/fin/luna/compiler/elaboration/src/program/interfaces/associated.rs#L68), [interfaces/associated.rs](file:///home/fin/luna/compiler/elaboration/src/program/interfaces/associated.rs#L749-L757), [interfaces/parents.rs](file:///home/fin/luna/compiler/elaboration/src/program/interfaces/parents.rs#L133) |
| `GAP-02-21` | Core | 运行时依赖类型（2 层核心+替换、`LocalRuntimeOrigin`、`DefaultParam`） | [runtime-dependent-types.md](file:///home/fin/luna/docs/compilation/runtime-dependent-types.md) | `Sensor<cal>`/`NodeIndex<scene.ref>`/零开销 `GlobalRuntimeOrigin::DefaultParam` 端到端贯通运行 | `🟡 Partial` | [runtime_origins.rs](file:///home/fin/luna/compiler/elaboration/src/program/runtime_origins.rs#L30-L92), [runtime_defaults.rs](file:///home/fin/luna/compiler/elaboration/src/program/runtime_defaults.rs#L356-L428), [runtime_environment.rs](file:///home/fin/luna/compiler/elaboration/src/program/runtime_environment.rs#L1-L85), `luna/tests/runtime_dependent_types.rs` |
| `GAP-02-22` | Core | 闭包 ABI 边界：隐式 `RuntimeEnvParameter` 传递与语义捕获分离 | [issues/0001](file:///home/fin/luna/issues/0001-runtime-dependent-origin-and-default-param.md#L318-L366) | 作用域内清理槽已修（`e14afe49`），但泛型/间接闭包如 `factory`、`Box.with_ref`/`Vec.try_for_each_ref` 被 `closure_allowed` 拦截，且显式签名把操作环境混入语义字段 | `⚠️ Divergent` | [materialization/src/lib.rs](file:///home/fin/luna/compiler/materialization/src/lib.rs#L2848-L3220), [issues/0001](file:///home/fin/luna/issues/0001-runtime-dependent-origin-and-default-param.md#L318-L366) |
| `GAP-02-23` | Core | 开放闭包环境类型、捕获存储共享传输与开放签名形参协议 | [runtime-dependent-type.md](file:///home/fin/luna/docs/type/runtime-dependent-type.md), [elaboration.md](file:///home/fin/luna/docs/compilation/elaboration.md) | `H::TyClosure` 拒绝开放环境变量/源表达式/`non-callable` 体；`catch` 形参、非位置 effect 形参及开放签名延迟协议未实现 | `🔴 Missing` | [program.rs](file:///home/fin/luna/compiler/elaboration/src/program.rs#L3503-L3539), [program.rs](file:///home/fin/luna/compiler/elaboration/src/program.rs#L4921-L4960), [open_functions.rs](file:///home/fin/luna/compiler/elaboration/src/program/open_functions.rs#L1938-L1995), [captures.rs](file:///home/fin/luna/compiler/elaboration/src/program/constants/materialization/captures.rs#L191-L345) |
| `GAP-02-24` | Core | `where` 局部具名/位置类型与值参数推断（如 `format_debug<.A=...>`） | [elaboration.md](file:///home/fin/luna/docs/compilation/elaboration.md), [candidate-solving.md](file:///home/fin/luna/docs/compilation/candidate-solving.md) | `with_partial_named_call_plan` + `observed_call_clause_types` 支持普通函数部分具名推断；但排除 `comptime fn`、静态/特质方法与变参收集器函数 | `🟡 Partial` | [trait_inputs.rs](file:///home/fin/luna/compiler/elaboration/src/program/trait_inputs.rs#L445-L637), [call_evidence.rs](file:///home/fin/luna/compiler/elaboration/src/program/trait_inputs/call_evidence.rs#L6-L170), [call_selection.rs](file:///home/fin/luna/compiler/elaboration/src/program/call_selection.rs#L242-L267), [interfaces.rs](file:///home/fin/luna/compiler/elaboration/src/program/interfaces.rs#L3124-L3206) |
| `GAP-02-25` | Core | Occurrence Typing（`if`/`when`/短路/早退/开放区域与循环回边） | [occurrence-typing.md](file:///home/fin/luna/docs/type/propositional-constraints/occurrence-typing.md) | `if`/`when`/`and`/`or`/`match`/`Never` 早退与 `TypeConditionScope` 已打通；`K::Loop` 直接返 `None`，缺循环不变量与回边重校验机制 | `🟡 Partial` | [refinements.rs](file:///home/fin/luna/compiler/elaboration/src/program/refinements.rs#L423-L515), [refinements.rs](file:///home/fin/luna/compiler/elaboration/src/program/refinements.rs#L860-L950), [block_regions/logical.rs](file:///home/fin/luna/compiler/elaboration/src/program/block_regions/logical.rs#L1-L100) |
| `GAP-02-26` | Core | 局部依赖调度、目标挂起/并查集转发/取消协议 | [local-dependencies.md](file:///home/fin/luna/docs/compilation/local-dependencies.md) | `block_regions::driver` 与 `dependent_inputs::tasks` 实现帧式续体状态机；但未实现规范定义的通用 `Goal`/`WaitTicket`/并查集 watcher 转发与拓扑取消协议 | `🟡 Partial` | [driver/region.rs](file:///home/fin/luna/compiler/elaboration/src/program/block_regions/driver/region.rs#L154-L238), [dependent_inputs/tasks.rs](file:///home/fin/luna/compiler/elaboration/src/program/dependent_inputs/tasks.rs#L6-L100), [declaration_inputs/progress.rs](file:///home/fin/luna/compiler/elaboration/src/program/declaration_inputs/progress.rs#L15-L100) |
| `GAP-02-27` | Core | `middle` 查询系统与 `interface` 会话接口完整性 | `middle/queries.rs`, `interface/lib.rs` | `middle::Db` 仅定义 `hir_package` 1 个查询，未接入 elaboration/类型/特质/NIR 增量查询；`interface` 仅暴露 VFS 与 `hir_package`，且注释仍留旧两包描述 | `🟡 Partial` | [middle/src/queries.rs](file:///home/fin/luna/compiler/middle/src/queries.rs#L66-L126), [interface/src/lib.rs](file:///home/fin/luna/compiler/interface/src/lib.rs#L43-L157) |
| `GAP-02-28` | Wishlist | `wishlist/05` 14 组全量 `Type.predicates.*`（含短命名统一）与布局扩展 | [wishlist/05](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md#L15-L60) | 缺失 `is_bool`/`is_void`/`is_never`/`is_zst`/`has_niche`/`needs_drop`/`has_padding`/`has_head`/`HeadOf`/`stride_of`/`field_offset_of` 及短命名（`is_bool`/`is_int`/`is_fn`）对齐 | `🔴 Missing` | [projections.rs](file:///home/fin/luna/compiler/elaboration/src/program/projections.rs#L156-L248), [nir/src/layout.rs](file:///home/fin/luna/compiler/nir/src/layout.rs#L18-L45), [wishlist/05:2850-2852](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md#L2850-L2852) |
| `GAP-02-29` | Wishlist | `wishlist/05` `RecordInfo` 与结构反射 / 128-bit `type_id` | [wishlist/05](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md#L15-L60) | 缺匿名记录/结构体字段名与类型列表反射、`type_name`、128 位 Merkle `type_id` 及 `__flurry_stable_since` 接入 | `🔴 Missing` | [projections.rs](file:///home/fin/luna/compiler/elaboration/src/program/projections.rs#L156-L262) |
| `GAP-02-30` | Wishlist | `wishlist/01` & `03` `VariadicCollector` / `ImplicitPassing` 泛化协议 | `wishlist/01`, `wishlist/03` | 编译器内零实现；变参收集与 `Tuple` / `meta.List` 仍绑定专用逻辑，`implicit` 自动传参未打通 | `🔴 Missing` | [program/arguments.rs](file:///home/fin/luna/compiler/elaboration/src/program/arguments.rs#L261-L443), [input_routing.rs](file:///home/fin/luna/compiler/elaboration/src/program/input_routing.rs#L187-L189) |
| `GAP-02-31` | Wishlist | `wishlist/01` 自定义字面量类型精化与一等变参值扩散 | `wishlist/01` | 普通展开与基础字面量协议已运行，但一般化字面量协议及非 tuple 扩展传播受限于特判 | `🟡 Partial` | [literals.rs](file:///home/fin/luna/compiler/elaboration/src/program/expression_values/literals.rs#L66), [type_spreads.rs](file:///home/fin/luna/compiler/luna/tests/type_spreads.rs#L42-L185) |
| `GAP-02-32` | Wishlist | `verification` 内置包高级程序验证与高阶证明契约 | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §3 | 按作者确认决策暂缓高级内容，仅维持接口与包边界及 PTC 纯类型命题隔离 | `⚪ Deferred` | [frontend.rs](file:///home/fin/luna/compiler/luna/src/frontend.rs#L627), [ptc/src/lib.rs](file:///home/fin/luna/compiler/ptc/src/lib.rs#L1-L6) |

---

## 4. 关键缺口深度剖析（含源码证据）

### 4.1 `ptc` 19 类原子覆盖边界、理论模块与零状态泄露核验

依据 [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md#L136-L154) 定义的 A01–A19 规范全集，对 [compiler/ptc/src/lib.rs](file:///home/fin/luna/compiler/ptc/src/lib.rs#L60-L93) 与 [compiler/nir/src/program.rs](file:///home/fin/luna/compiler/nir/src/program.rs#L5035-L5043) 逐项核对：

| 原子编号与模式 | 所属理论 | `ptc::Atom` 现状 | `nir::TypePredicate` 接线 | 关键源码约束与缺失详情 |
|---|---|---|---|---|
| **A01** `Eq(sort,a,b)` | EUF / TYPE | `Atom::TypeEqual(Equality)` | `TypePredicate::Equal` | `same_congruent_head` ([lib.rs:1357-1446](file:///home/fin/luna/compiler/ptc/src/lib.rs#L1357-L1446))、环检测 `has_structural_type_cycle` ([lib.rs:1813-1902](file:///home/fin/luna/compiler/ptc/src/lib.rs#L1813-L1902))、`effects` & `error_results` 行理论已就绪 |
| **A02** `Kind(T,k)` | TYPE | `Atom::Kind(KindQuery)` | `TypePredicate::Kind` | `nir::TypeKind` 含 10 个变体（含 `Nominal(NominalKind::{Struct,Enum,Newtype})`），缺 `Union`/`Record`/`Module`/`Char`；[classification.rs:13-71](file:///home/fin/luna/compiler/ptc/src/classification.rs#L13-L71) |
| **A10** `Implements(x,t,world)` | TRAIT | `Atom::Trait(TraitQuery)` | `TypePredicate::Trait` | `traits.rs` 验证输入 sort 与作用域；世界由 `type_resolution` 适配。但 [model_search.rs:20-23](file:///home/fin/luna/compiler/ptc/src/model_search.rs#L20-L23) 遇 `Atom::Trait` 直接放弃反模型搜索 |
| **A12** `IntLe(a,b)` | INT | `Atom::IntLe(IntLe)` | **未接线 ❌** | [integer.rs](file:///home/fin/luna/compiler/ptc/src/integer.rs#L14-L82) + [integer/state.rs](file:///home/fin/luna/compiler/ptc/src/integer/state.rs#L21-L100) + `integer/propagation.rs` 实现 `i128` 数学差分图与 Bellman-Ford 路径证人；但 [nir/src/program.rs:5035-5043](file:///home/fin/luna/compiler/nir/src/program.rs#L5035-L5043) 与 [Formula::from_predicate](file:///home/fin/luna/compiler/ptc/src/lib.rs#L81-L93) **无 `IntLe` 分支**，源级 `requires` / `assumes` 无法直接携带之 |
| **A11** `NotSized(T,target)` | LAYOUT | **缺失 ❌** | **缺失 ❌** | 无原子表示；`projections.rs` 不识别 `is_not_sized` |
| **A03–A09, A13–A19**（`InUniverse`, `Qualified`, `HasSlot`, `HasHeadShape`, `Matches`, `Compatible`, `Subtype`, `IntCongruent`, `BVUnsignedLe`, `BVSignedLe`, `RowContains`, `RowSubset`, `RowDisjoint`, `PredicateCall`） | TYPE / MATCH / REL / INT / BV / ROW / PRED | **全部缺失 ❌** | **全部缺失 ❌** | 无对应 `Atom` 变体或理论求解器支持；部分（如 `ErrorResult` 子集/不交、效应行同余）目前仅隐藏在 A01 `TypeEqual` 内部或 `type_resolution/matching/` 局部逻辑中，未能以独立原子进入布尔精化与反驳证明 |

**零程序状态泄露核验（正向合规）**：
- `ptc::lib.rs`、`traits.rs`、`effects.rs`、`error_results.rs`、`declaration_theory.rs` 与 `model_domain.rs` 中所有输入域严格限制为 `Sort::{Type, ComptimeValue}` 及纯类型项/静态标量，`Atom` 无任何指针别名、堆状态、读写版本号或内存地址命题；所有非正/溢出的整数运算与未关闭的未知量统一退回 `Unknown`（如 [integer.rs:80](file:///home/fin/luna/compiler/ptc/src/integer.rs#L80) 返回 `UnknownReason::ArithmeticRange`、[error_results.rs:46-48](file:///home/fin/luna/compiler/ptc/src/error_results.rs#L46-L48) 在目标未闭合时跳过缺失判断），符合 [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5 “PTC 只处理类型命题”边界。

### 4.2 `Type.predicates.*` 与 `Type.*` 反射查询缺口

1. **谓词覆盖与命名对齐**：[atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md#L18-L32) 仅直接点名 `Type.predicates.is_tuple` 与 `Type.predicates.is_not_sized` 并申明“表格中文描述是语义对象，不额外规定 API 拼写”。当前 [projections.rs](file:///home/fin/luna/compiler/elaboration/src/program/projections.rs#L156-L172) 中的 `parse_type_predicate_kind` 仅硬编码识别 12 个长命名属性：`is_boolean`、`is_symbol`、`is_integer`、`is_signed_integer`、`is_unsigned_integer`、`is_floating_point`、`is_struct`、`is_enum`、`is_newtype`、`is_tuple`、`is_pointer`、`is_function`；缺少 Core 规范要求的 `is_not_sized`（A11），同时与 [wishlist/05](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md#L2850-L2852) 的短命名规范（`is_bool`、`is_int`、`is_signed_int`、`is_unsigned_int`、`is_float`、`is_fn`）以及 `is_void`、`is_never`、`is_char`、`is_union`、`is_anonymous_record`、`is_module`、`is_optional`、`is_error_qualified`、`is_effect_qualified`、`has_head` 尚待统一补齐。
2. **布局查询与标记类型局限**：`type_layout_query_call`（[projections.rs:234-262](file:///home/fin/luna/compiler/elaboration/src/program/projections.rs#L244-L248)）仅接通 `size_of` 与 `align_of`；[type_data.rs:155-163](file:///home/fin/luna/compiler/elaboration/src/program/type_data.rs#L155-L163) 在 `Type.vars` 下仅识别 `"Closure"` 映射为 `Term::ClosureContextType`，未提供 `EffectRow`、`ErrorRow`、`Reachability`、`Any`。

### 4.3 特质双相解析、`derive fn` 阻断与 `dyn Trait` 现状

1. **双相解析优先级非对称**：
   - `pure comptime trait`（`contract.comptime_only == true`）在 [discovery.rs:117-248](file:///home/fin/luna/compiler/elaboration/src/program/interfaces/discovery.rs#L117-L248) 与 [parents.rs:465-478](file:///home/fin/luna/compiler/elaboration/src/program/interfaces/parents.rs#L465-L478) 中会先筛 `preferred = true` 再筛 `false`，确保在 comptime 上下文中优先于普通 `trait`；
   - 但是 **方法级** `^attrs.comptime_phase` 不参与该分层排序（见 [trait_phases.rs:1021-1031](file:///home/fin/luna/compiler/luna/tests/trait_phases.rs#L1021-L1031) `member_phase_attribute_does_not_grant_interface_source_priority` 测试确认：`where T :- Ordinary + StagedMember` 中即使后者标注 `^attrs.comptime_phase` 仍报 `ambiguous capability member`）；
   - 同时**固有 `comptime impl Subject`** 在 [interfaces.rs:447-451](file:///home/fin/luna/compiler/elaboration/src/program/interfaces.rs#L447-L451) 直接报错 `"comptime implementation phase selection is not yet implemented"`，而 VM 侧 [comptime_interpretor/src/lib.rs:1166-1170](file:///home/fin/luna/compiler/comptime_interpretor/src/lib.rs#L1166-L1170) 对缺乏接口的 `comptime` 实现直接拒绝返回 `Err(invalid())`。
2. **`derive fn` 七大显式阻断**（[program/derivation.rs](file:///home/fin/luna/compiler/elaboration/src/program/derivation.rs#L39-L215) & [L499-L505](file:///home/fin/luna/compiler/elaboration/src/program/derivation.rs#L499-L505)）：
   - 第 39–44 行：禁止策略自身带 `where` 子句（`!def.clause_params.is_empty()`）；
   - 第 115–122 行：禁止块内局部 `derive` 请求；
   - 第 130–132 行：禁止经 comptime 表达式动态计算的策略名（`"computed strategy selection"`）；
   - 第 149–158 行：在带 `clause_params` 的泛型 `derive` 请求中，禁止策略带输入参数、约束或计算语句；
   - 第 159–177 行：策略形参标志严格限定在 `TPARAM_COMPTIME | TPARAM_IMPLICIT | TPARAM_LAMBDA`（`quote` 形参触发 `"specialized strategy parameter matching"` 错误）及常规位置/可选/变参形状；
   - 第 200–206 行：所有未被引用的 `derive fn` 声明触发编译失败（`"independent checking of unused strategies"`）；
   - 第 499–505 行：禁止捕获省略外围环境的属性。
   - 正向进度：`^attrs.derivation.default` 已通过 [attributes/derivation.rs](file:///home/fin/luna/compiler/elaboration/src/program/attributes/derivation.rs#L23-L71) 与 [program.rs:2214-2368](file:///home/fin/luna/compiler/elaboration/src/program.rs#L2214-L2368) 接入注册与单参数/结构匹配选型（`match_default_strategy_pairs`）。
3. **`dyn Trait` 止步于语法前端**：仅存在于 `compiler/ast/src/lib.rs:261`（`TraitObjectType`）、`compiler/parse/src/expressions.rs:346` 与 `compiler/fmt/src/exprs.rs:490`；`ast_lowering`、`hir`、`nir`、`type_resolution` 与 `elaboration` 中完全不存在动态接口对象类型、虚表（vtable）布局、`%c dyn Trait` 捕获扩展、`Self` 重装箱或参数安全检查。

### 4.4 运行时依赖类型、闭包 ABI、`where` 局部推断与 `elaboration` 未实现普查

1. **运行时依赖类型与 `GlobalRuntimeOrigin::DefaultParam`**：
   - 本地存储地址捕获（`runtime_origins.rs:30-295`）、环境传参计划（`runtime_environment.rs:7-85`）以及全局只读/无自由变量叶子默认值去重缓存 `GlobalRuntimeOrigin::DefaultParam`（[runtime_defaults.rs:356-428](file:///home/fin/luna/compiler/elaboration/src/program/runtime_defaults.rs#L356-L428)，如默认 `std.mem.DefaultAllocator.GLOBAL.ref`）均已运行并在 `luna/tests/runtime_dependent_types.rs` 验证。
2. **尚未打通的闭包 ABI 边界（[issues/0001:318-366](file:///home/fin/luna/issues/0001-runtime-dependent-origin-and-default-param.md#L318-L366)）**：
   - 当泛型函数体内构造闭包或容器回调方法内部调用依赖特化后才显现的隐式 `quote` / `RuntimeEnvParameter` 方法时（例如下例在 `Box<String>` / `Vec<String>` 中通过 `with_ref` / `try_for_each_ref` 调 `String.write_debug`），[materialization/src/lib.rs:2848-3220](file:///home/fin/luna/compiler/materialization/src/lib.rs#L2848-L3220) 的 `closure_allowed` 检查发现所需 `RuntimeId` 不在闭包签名中而报错 `"materialized call cannot deliver runtime environment parameter"`：
   ```flurry
   struct Cell {
       value: i32,
   }

   fn read(p: *Cell) -> i32 {
       p.value.*
   }

   trait Inspect {
       fn inspect(self) -> bool;
   }

   struct Sample where quote src: lift *Cell, quote .saved: lift i32 = read(src) {
       tag: u8,
   }

   impl Inspect for Sample<src, .saved = n>
   where quote src: lift *Cell, quote n: lift i32 {
       fn inspect(self) -> bool { src.value.* == 42 and n == 7 }
   }

   -- 当前阻塞：闭包本身只捕获 v: T，特化后需要隐式传递的 src/n 无操作环境转发路径
   fn factory(v: T) -> %{ v: T } fn() -> bool where T :- Inspect {
       || v.inspect()
   }
   ```
   - 并且若将签名显式写为 `Sample<src, .saved = n>`，当前 elaboration 还会把 `src` 与 `n` 错误抬升为语义捕获字段（实得 3 字段而非 1 字段）。
3. **`where` 局部参数推断与 `format_debug<.A=...>`**：
   - `trait_inputs.rs:547-637` 的 `with_partial_named_call_plan` 结合 `call_evidence.rs:6-170` 的 `observed_call_clause_types` 已成功支持普通函数如 `format_debug<.A = FixedBufferAllocator, .allocator = fba>(value)` 省略首位待推导参数 `T`（`std/fmt_impl/arguments.fl:181-189`、`luna/tests/builtin_debug_and_iterator.rs:84`）。
   - **当前盲区**：① [call_selection.rs:242-244](file:///home/fin/luna/compiler/elaboration/src/program/call_selection.rs#L242-L244) 硬性附加 `&& !comptime`，导致 `comptime fn` 无法享受局部具名推断；② [interfaces.rs:3124-3206](file:///home/fin/luna/compiler/elaboration/src/program/interfaces.rs#L3124-L3206) 的 `explicit_method_instance` 未调用 `with_partial_named_call_plan`，方法调用 `recv.m<.A = ...>(x)` 省略前置泛型形参直接失配；③ [trait_inputs.rs:559](file:///home/fin/luna/compiler/elaboration/src/program/trait_inputs.rs#L559) 排除含变参收集器 `clause_collector(owner).is_some()` 的函数；④ [trait_inputs.rs:459](file:///home/fin/luna/compiler/elaboration/src/program/trait_inputs.rs#L459) 中 `partial_positional_clause_prefix` 在 `has_optional_type_clauses(owner)` 时直接放弃。
4. **`compiler/elaboration` 未实现诊断与 `middle` / `interface` 现状**：
   - 全 crate 非测试源码中虽 `todo!()` / `unimplemented!()` 为 0，但含 **80+ 处** `"is not implemented"` / `"is not yet implemented"` 显式错误门控，集中在 `program.rs`（参数协议/catch/开放闭包/精确 Float 除/comptime 求值标注）、`open_functions.rs:1938-1995`（开放签名的接口/effect/模板/方法协议）、`effects/handlers.rs`（comptime effect/非直接 handler 降码）、`cmtt/{functions,generators,patterns}.rs`（模式代码生成/上下文 `quote fn`）、`interfaces/associated.rs`、`spreads/inputs.rs:348` 与 `input_routing.rs:187-189`。
   - [compiler/middle/src/queries.rs:67-126](file:///home/fin/luna/compiler/middle/src/queries.rs#L67-L126) 的 `Providers` / `QueryCaches` 当前仅注册单一 `hir_package` 查询，尚未接入类型检查、特质求解或 elaboration 增量查询；[compiler/interface/src/lib.rs:43](file:///home/fin/luna/compiler/interface/src/lib.rs#L43) 注释仍停留于旧 `(0=builtin, 1=std)` 假设。

### 4.5 Occurrence Typing 与局部依赖调度机制核查

1. **Occurrence Typing 精化管线**（[refinements.rs](file:///home/fin/luna/compiler/elaboration/src/program/refinements.rs#L415-L515)、`block_regions/{conditionals,when,logical,matches}.rs`）：
   - `condition_source_type_fact` 与 `continuation_type_fact` 在 `if`/`else`、顺序 `when`、短路 `and`/`or`、`match` 分支守卫以及 `P::Never` 早退后均能构造并通过 PTC 验证 `TypeConditionScope` 假设；
   - **循环回边收敛缺口**：[refinements.rs:511-513](file:///home/fin/luna/compiler/elaboration/src/program/refinements.rs#L511-L513) 明确对所有循环 `K::Loop` 返回 `None`（既不向循环后导出入口否定条件，也不做跨迭代不变量交集或显式回边精化），体内仅靠 `local_write_versions` epoch 与可变局部写入检测防误用。
2. **局部依赖调度（对比 [local-dependencies.md](file:///home/fin/luna/docs/compilation/local-dependencies.md)）**：
   - 现网由 [block_regions/driver/region.rs](file:///home/fin/luna/compiler/elaboration/src/program/block_regions/driver/region.rs#L154-L238) 的栈式 `RegionFrame` 与 [dependent_inputs/tasks.rs](file:///home/fin/luna/compiler/elaboration/src/program/dependent_inputs/tasks.rs#L6-L100)、[declaration_inputs/progress.rs](file:///home/fin/luna/compiler/elaboration/src/program/declaration_inputs/progress.rs#L15-L100) 的槽位任务队列承担源序延迟准备与投机分支回滚（`checking_attempts::Attempt`）；
   - 虽然对标准函数调用、构造器、条件/when 分支工作稳定，但尚未抽象为文档设计中的通用 `Goal` 依赖图、`WaitTicket`、合并时并查集 watcher 转发以及环检测全局取消协议，`constants.rs:1519` 等局部表仍以专用 `waiters` 临时编码。

---

## 5. 架构偏离与技术债清单（⚠️ Divergent 专项）

### 5.1 规范偏离清单（严格 1:1 对应第 3 节 3 项 `⚠️ Divergent`）

| 编号 | 对应 `GAP` | 偏离项 | 违反规范依据 | 现网偏离代码点 | 危害与修复契约 |
|---|---|---|---|---|---|
| `DIV-02-01` | `GAP-02-08` | **`Tuple` 与 `meta.List<Type>` 专用编译器特判债** | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5（严禁为 `Tuple`/`meta.List<Type>` 设专用规则） | `nir/normal_forms.rs:386-420` (`BuiltinCallable::{List,Tuple}`, `Role::{Tuple,TypeList}`), `builtin_bindings.rs:78-117`, `normal.rs:55-110`, `collections.rs:110-115`, `clause_inputs.rs:228-231`, `arguments.rs:261-265,439-443`, `program.rs:4219-4256`, `type_resolution/src/implementation.rs:627`, `applicability.rs:947-978`, `luna/src/protocols/type_constructors.rs:329-460`, `library/language-items.conf:6-7` | 用户定义变参 NF 头无法作为一等变参收集目标或结构分类；须重构为一般 comptime 列表值收集/构造与一般 NF 头协议，仅保留布局优化而不改变语言身份语义 |
| `DIV-02-02` | `GAP-02-16` | **`^attrs.comptime_phase` 对固有 `impl` 与同级方法优先级的非对称** | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §3 / §5 | `interfaces.rs:447-451` 拒绝固有 `comptime impl`；`comptime_interpretor/src/lib.rs:1166-1170` 拒绝对无接口 `comptime` 实现求值（`Err(invalid())`）；`discovery.rs:118-248` 与 `trait_phases.rs:1021-1031` 使普通 trait 上的 `comptime_phase` 方法不获优先级 | 阻碍在单一 `trait` 中共存 `runtime`/`comptime` 双相方法（如内置类型与容器的方法双相化需求），迫使库作者另拆 `pure comptime trait` |
| `DIV-02-03` | `GAP-02-22` | **显式闭包的语义捕获与操作环境字段混同** | [issues/0001:343-358](file:///home/fin/luna/issues/0001-runtime-dependent-origin-and-default-param.md#L343-L358) | `program.rs:4921-4942`, 显式 `%{ v: Sample<src, .saved = n> }` 闭包把 `src`/`n` 塞入语义捕获列表导致期待 1 字段实得 3 字段 | 破坏用户可见闭包类型的捕获字段数量与布局契约，必须与 `materialization` 协同拆出“操作环境交付”与“语义捕获字段” |

### 5.2 非规范违例的技术债与命名对齐项（对应 `🟡 Partial` / `🟢 Full` / Wishlist 项）

| 编号 | 对应 `GAP` | 技术债项 | 参考依据 | 现网位置 | 危害与修复契约 |
|---|---|---|---|---|---|
| `TD-02-01` | `GAP-02-01` | **`ptc::Atom::IntLe` 与 `nir::TypePredicate` 断层** | [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md#L147) | `ptc/src/lib.rs:60-93` 已定义 `Atom::IntLe` 与 `Formula::IntLe`，但 `nir/src/program.rs:5035-5043` 与 `Formula::from_predicate` 缺失 `IntLe` | 导致已编写的 392+478 行整数差分理论求解器无法被用户 `requires` / `where` / `assumes` 声明契约直接消费 |
| `TD-02-02` | `GAP-02-03`, `GAP-02-28` | **`Type.predicates.*` 长命名实现 vs `wishlist/05` 短命名规格** | [wishlist/05:2850-2852](file:///home/fin/luna/tmp/wishlist/05-meta-cmtt-macros-reflection.md#L2850-L2852) | `projections.rs:156-172` 接收 `is_boolean`/`is_integer`/`is_signed_integer`/`is_unsigned_integer`/`is_floating_point`/`is_function`；Wishlist 建议 `is_bool`/`is_int`/`is_signed_int`/`is_unsigned_int`/`is_float`/`is_fn` | 长短命名分裂将使 Wishlist 反射生态与现有代码不一致；需按 Wishlist 默认建议统一为短命名并对旧长名发废弃警告 |
| `TD-02-03` | `GAP-02-13`, `GAP-02-27` | **遗留死模块 `#[cfg(test)] mod gradual` / `type_computations` 与过时注释** | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §1 | `elaboration/src/lib.rs:35-47` 声明二者无生产消费者却保留维护包袱；`interface/src/lib.rs:43` 注释仍写 `0 = builtin, 1 = std` | 易误导后续实现误以为旧兼容模型或旧双包 sysroot 仍在生产链生效；应在迁移对应单元测试后移除死模块并更新注释 |

> **正向合规确认（已实盘验证未发生偏离）**：
> 1. `library/builtin` 与 `library/meta` 已按 `AGENTS.md` §5 完成集中绑定（`builtin/types/tuple.fl:6`、`meta/lib.fl:4`、`primitives.fl:13`），且 `builtin/types/{option,result}.fl` 仅定义 `OptionalOps` / `ErrorResultOps`，不存在 `Option`/`Result` 标准库枚举；
> 2. `Identity<T> != T` 非展开原则已在 `matching.rs:739-741`、`ptc` 与 `type_spreads.rs:98-118` 严格保持；
> 3. 6 个内置包（`builtin`、`std`、`meta`、`build`、`attrs`、`verification`）均已进入 `builtin_implementation_packages` 并享受编译器 builtin 主体的孤儿豁免（`frontend.rs:627-662`, `packages.rs:1783-1786`, `ownership.rs:217-236`）。

---

## 6. 健全性 · 完备性 · 鲁棒性专项审计（SND / CMP / ROB）

### 6.1 健全性审计（`SND-02-NN`）

首先确认已守住的核心健全性红线：
- **Occurs Check**：`InferenceContext::check_occurs` ([type_resolution/src/lib.rs:262-306](file:///home/fin/luna/compiler/type_resolution/src/lib.rs#L262-L306)) 与 `ptc::Search::has_structural_type_cycle` ([ptc/src/lib.rs:1813-1902](file:///home/fin/luna/compiler/ptc/src/lib.rs#L1813-L1902)) 均在统一及等式推理前拦截无穷自指类型；
- **未知安全（`Unknown` 保守性）**：`ptc` 在遭遇 `Atom::Trait`、`ArithmeticRange` 溢出、未闭合 `ErrorResult` 尾部或未知效果标签时均保持 `Unknown` 而不伪证真假（[integer.rs:80](file:///home/fin/luna/compiler/ptc/src/integer.rs#L80), [error_results.rs:46-48](file:///home/fin/luna/compiler/ptc/src/error_results.rs#L46-L48), [effects.rs:68-70](file:///home/fin/luna/compiler/ptc/src/effects.rs#L68-L70), [model_search.rs:20-23](file:///home/fin/luna/compiler/ptc/src/model_search.rs#L20-L23)）；
- **NF 头非展开**：未展开任何自定义 `NormalForm` 定义体用于类型等价反演（`matching.rs:739-741`）。

本领域仍检出 **5 项健全性缺陷**：

| 编号 | 严重度 | 健全性隐患描述 | 实证 `file:line` 与符号 | 后果与修复要求 |
|---|---|---|---|---|
| `SND-02-01` | **High** | **默认衍生策略匹配的集合语义错配与泛化重叠未证明**：`match_default_strategy_term` 对 `ErrorResult` 错误标签采用有序切片 `match_default_strategy_pairs(a_err, b_err, ...)`，且对 `Computation.effects` 做原样 `a_eff == b_eff` 对比（不替换 `params` 绑定）；同时 `program.rs:2232-2333` 在注册多个参数化默认 `derive fn` 策略时未接入类似 `CoherenceReport` 的 PTC 区间不相交证明，仅靠运行时选型冲突检测 | [program.rs:2232-2368](file:///home/fin/luna/compiler/elaboration/src/program.rs#L2232-L2368), [program.rs:2548-2563](file:///home/fin/luna/compiler/elaboration/src/program.rs#L2548-L2563) | 将本应等价的无序错误标签集拆散导致误拒绝或遗漏重叠策略；下游用户使用带参默认衍生时缺少编译期前端连贯性保证，应改用有序无重复集语义与 PTC 分桶冲突检查 |
| `SND-02-02` | **High** | **闭包跨特化边界的运行时身份（`RuntimeId`）逃逸风险与语义捕获污染**：目前 `materialization` 的 `closure_allowed` 拦截保护了后端不发生悬垂读取，但前端若直接在 `%{ v: Sample<src, .saved = n> }` 中把 `src`/`n` 晋升进 `Environment` 字段，会扭曲语义字段数与 ABI 契约；反之若放宽检查则会引发悬垂局部地址读 | [issues/0001:318-366](file:///home/fin/luna/issues/0001-runtime-dependent-origin-and-default-param.md#L318-L366), [materialization/src/lib.rs:2848-3220](file:///home/fin/luna/compiler/materialization/src/lib.rs#L2848-L3220) | 必须维持语义捕获字段不变，并在前端-物化-后端之间建立独立的操作环境交付槽及存储生命周期证明，绝不可简单放宽 `closure_allowed` |
| `SND-02-03` | **Medium** | **循环内局部写入与精化失效隔离的保守边界脆弱**：`refinements.rs:511-513` 仅保障循环退出**后**不继承分支精化（返回 `None`），但循环体内多次迭代对引用可变局部绑定/依赖类型指针指向对象的交叉写回完全依赖 `local_write_versions` epoch 单趟扫描，尚无显式循环回边（back-edge）状态汇聚校验 | [refinements.rs:511-513](file:///home/fin/luna/compiler/elaboration/src/program/refinements.rs#L511-L513), [runtime_origins.rs:61-65](file:///home/fin/luna/compiler/elaboration/src/program/runtime_origins.rs#L61-L65) | 一旦未来接入跨回边 occurrence typing 或迭代内部局部身份缓存，缺乏回边失效屏障将诱发循环第二次迭代读取到第一次迭代的过期依赖/类型事实 |
| `SND-02-04` | **Medium** | **`Tuple`/`meta.List` 离散构造器判异特权引发类型身份不对称风险**：`type_theory_with_classes` 与 `applicability.rs` 直接把 `is_tuple_type` / `is_list_type` 视为与所有 `Nominal` 互斥的外构造器；一旦新增非特权可收集 NF 头，除非同步纳入通用构型规则，否则 `distinct_outer_type_heads` 与 `Key::precision()` 将产生矛盾结论 | [ptc/src/lib.rs:1661-1681](file:///home/fin/luna/compiler/ptc/src/lib.rs#L1661-L1681), [applicability.rs:947-978](file:///home/fin/luna/compiler/type_resolution/src/implementation/applicability.rs#L947-L978) | 造成“同一套 NF 调用语义在两处产生相反不相交判断”的理论不一致性风险 |
| `SND-02-05` | **Low** | **Coherence 校验隐含不变量强耦合 panic**：`verify_type_coherence` 先通过 L637 `if a.head.interface.is_none() { return false; }` 截断开放接口，再于 L650 执行 `a.head.interface.unwrap()`；若任一调用方重排或单独抽取内循环逻辑，空 `interface` 将使验证直接崩溃而非返 `false` | [coherence.rs:636-656](file:///home/fin/luna/compiler/type_resolution/src/implementation/coherence.rs#L636-L656), [coherence.rs:682-688](file:///home/fin/luna/compiler/type_resolution/src/implementation/coherence.rs#L682-L688) | 违背“证明校验永不 panic”原则；应将 L650/L687 改为 `let Some(iface) = a.head.interface else { return false; }` |

### 6.2 完备性审计（`CMP-02-NN`）

| 编号 | 严重度 | 完备性盲区描述 | 实证 `file:line` | 影响面 |
|---|---|---|---|---|
| `CMP-02-01` | **High** | **PTC 原子与反模型完备性不足**：① `Atom` 仅 4/19（缺失匹配/位宽/同余/子集/可调用谓词等 15 类）；② 已实现的 `IntLe` 未接 `nir::TypePredicate`；③ `model_search.rs:20-23` 遇 `Atom::Trait` 立即退回 `Unknown` 且不构造新鲜 Nominal 见证，导致本可证伪（`NotEntailed`）的分支被错卡在不确定态 | [ptc/src/lib.rs:60-93](file:///home/fin/luna/compiler/ptc/src/lib.rs#L60-L93), [nir/src/program.rs:5035-5043](file:///home/fin/luna/compiler/nir/src/program.rs#L5035-L5043), [model_search.rs:20-80](file:///home/fin/luna/compiler/ptc/src/model_search.rs#L20-L80) | 阻碍 `inline when`、依赖整数尺寸校验、`BitVec` 及内置泛型容器的编译期化简与分支裁剪 |
| `CMP-02-02` | **High** | **跨特性正交盲区（`where` 局部具名推断 × 方法 / comptime / 变参 / 非 Type NF）**：① 显式方法 `recv.m<.A=...>(x)` 未经 `with_partial_named_call_plan`；② `comptime fn` 调用被 `!comptime` 排除；③ 变参函数（`clause_collector().is_some()`）无法结合部分具名推断；④ 非 `Type` 返回值 NF 动态调用直接报错 | [call_selection.rs:242-244](file:///home/fin/luna/compiler/elaboration/src/program/call_selection.rs#L242-L244), [interfaces.rs:3124-3206](file:///home/fin/luna/compiler/elaboration/src/program/interfaces.rs#L3124-L3206), [trait_inputs.rs:559](file:///home/fin/luna/compiler/elaboration/src/program/trait_inputs.rs#L559), [normal/dynamic_inputs.rs:84](file:///home/fin/luna/compiler/elaboration/src/program/normal/dynamic_inputs.rs#L84) | 一旦标准库方法（如容器或 IO 方法）采用带默认 `.A: Type = DefaultAllocator` 参数，调用者 `vec.fmt_with<.A = FBA>(...)` 即无法省略类型形参 |
| `CMP-02-03` | **High** | **`derive fn` 与泛型参数 / `where` / `quote` / 块局部作用域正交缺失**：策略不能写 `where` 子句、不能接受 `quote` 参数、不能在块内局部 `derive`、不能独立检查未被使用的策略 | [program/derivation.rs:39-215](file:///home/fin/luna/compiler/elaboration/src/program/derivation.rs#L39-L215) | 限制 `std` 容器（携带 `quote .allocator` 与复杂 `where` 约束）使用 `derive fn` 生成 `Debug`/`Eq`/`Hash`/`Clone` 等协议 |
| `CMP-02-04` | **Medium** | **双相特质与固有实现覆盖盲区**：固有 `comptime impl Subject` 完全无法使用；同级普通 trait 中使用 `^attrs.comptime_phase` 的成员无法在 comptime 上下文胜出无标记同签名成员 | [interfaces.rs:447-451](file:///home/fin/luna/compiler/elaboration/src/program/interfaces.rs#L447-L451), [discovery.rs:117-248](file:///home/fin/luna/compiler/elaboration/src/program/interfaces/discovery.rs#L117-L248), `trait_phases.rs:1021-1031` | 阻碍库类型为同一类型以同一套接口名自然区分 comptime/runtime 两相方法实现（违背 `AGENTS.md` §3） |
| `CMP-02-05` | **Medium** | **开放函数与签名协议大量保留桩拒绝**：`catch` 参数、非位置 effect 参数、开放闭包源表达式、开放模板中的待定域 `pure` 函数与抽象/参数化关联值均止步于前端门控错误 | [program.rs:3503-3539](file:///home/fin/luna/compiler/elaboration/src/program.rs#L3503-L3539), [program.rs:4921-4960](file:///home/fin/luna/compiler/elaboration/src/program.rs#L4921-L4960), [open_functions.rs:1938-1995](file:///home/fin/luna/compiler/elaboration/src/program/open_functions.rs#L1938-L1995), [interfaces/associated.rs:68,749-757](file:///home/fin/luna/compiler/elaboration/src/program/interfaces/associated.rs#L68) | 高阶效应签名、带环境源闭包的通用定义及关联常量泛化受阻 |
| `CMP-02-06` | **Low** | **`middle` / `interface` 增量查询管道尚未贯通中端**：`middle::Db` 仅缓存 1 个 `hir_package` query，所有 elaboration、trait 索引、PTC 缓存与 NIR 结果均脱离 `query` 引擎之外按全量包重跑 | [middle/src/queries.rs:67-126](file:///home/fin/luna/compiler/middle/src/queries.rs#L67-L126), [interface/src/lib.rs:146-157](file:///home/fin/luna/compiler/interface/src/lib.rs#L146-L157) | LSP 与大型多包工作区修改单文件后难以利用细粒度类型/特质查询失效复用 |

### 6.3 鲁棒性审计（`ROB-02-NN`）

#### 定量非测试代码 Panic 表面与基础设施普查表

经基于词法感知剥离（准确跳过 `#[cfg(test)]` 块、`*_tests.rs`/`tests.rs`、注释/字面量内嵌套花括号以及 `use`/`pub use` 导入行）对 `HEAD` `b9a52238` 五大审计 crate 统计，实盘定量数据如下：

| Crate (`src/`) | 非测试文件数 | 非测试源码行数 | 排除测试文件数 / 测试行数 | `.unwrap()` | `.expect()` | `panic!()` | `unreachable!()` | `todo!()` / `unimplemented!()` | **Panic 表面合计** | 非 `use` `HashMap`/`HashSet` | 非 `use` `DUMMY_SP`/`Span::default()` |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `compiler/ptc/src` | 38 | 8,347 | 2 / 5,969 | 17 | 4 | 0 | 6 | 0 | **27** | 17 | 0 |
| `compiler/type_resolution/src` | 39 | 17,683 | 0 / 2,022 | 11 | 9 | 0 | 6 | 0 | **26** | 49 | 0 |
| `compiler/middle/src` | 4 | 363 | 0 / 0 | 0 | 1 | 1 | 0 | 0 | **2** | 0 | 0 |
| `compiler/interface/src` | 2 | 239 | 0 / 0 | 0 | 0 | 0 | 0 | 0 | **0** | 0 | 0 |
| `compiler/elaboration/src` | 293 | 134,536 | 6 / 15,913 | 357 | 100 | 0 | 272 | 0 | **729** | 95 | 43 |
| **合计** | **376** | **161,168** | **8 / 23,904** | **385** | **114** | **1** | **284** | **0** | **784** | **161** | **43** |

**重点 Hotspot 文件（非测试生产行）**：
- `elaboration/src`: [open_functions.rs](file:///home/fin/luna/compiler/elaboration/src/program/open_functions.rs) (**70** 处：`L66,88,288,1007,...`)、[interfaces.rs](file:///home/fin/luna/compiler/elaboration/src/program/interfaces.rs) (**41** 处：`L147,235,476,...`)、[program.rs](file:///home/fin/luna/compiler/elaboration/src/program.rs) (**33** 处：`L1260,1299,4029,...`)、[interfaces/associated.rs](file:///home/fin/luna/compiler/elaboration/src/program/interfaces/associated.rs) (**19** 处)、[cmtt/generators.rs](file:///home/fin/luna/compiler/elaboration/src/program/cmtt/generators.rs) (**19** 处)、[cmtt/patterns/generators.rs](file:///home/fin/luna/compiler/elaboration/src/program/cmtt/patterns/generators.rs) (**18** 处)、[declaration_inputs/slots.rs](file:///home/fin/luna/compiler/elaboration/src/program/declaration_inputs/slots.rs) (**17** 处)、[projections.rs](file:///home/fin/luna/compiler/elaboration/src/program/projections.rs) (**15** 处)。
- `ptc/src`: [search.rs](file:///home/fin/luna/compiler/ptc/src/search.rs) (**8** 处：`L26,127,157,212,282,308,349,445`)、[theory_state.rs](file:///home/fin/luna/compiler/ptc/src/theory_state.rs) (**5** 处：`L60,184,206,214,355`)、[integer/state.rs](file:///home/fin/luna/compiler/ptc/src/integer/state.rs) (**3** 处：`L66,70,147`)、[ptc/src/lib.rs:1492](file:///home/fin/luna/compiler/ptc/src/lib.rs#L1492)。
- `type_resolution/src`: [lib.rs](file:///home/fin/luna/compiler/type_resolution/src/lib.rs) (**6** 处)、[inference_budget.rs](file:///home/fin/luna/compiler/type_resolution/src/inference_budget.rs) (**4** 处)、[obligations.rs](file:///home/fin/luna/compiler/type_resolution/src/obligations.rs) (**3** 处)、[implementation/phases.rs:232,238](file:///home/fin/luna/compiler/type_resolution/src/implementation/phases.rs#L232-L238)、[implementation/coherence.rs:650,687](file:///home/fin/luna/compiler/type_resolution/src/implementation/coherence.rs#L650-L687)。
- `middle/src`: [queries.rs:75,192](file:///home/fin/luna/compiler/middle/src/queries.rs#L75-L192)。

#### 鲁棒性具体问题项（`ROB-02-NN`）

| 编号 | 严重度 | 鲁棒性风险描述 | 实证 `file:line` | 修复建议 |
|---|---|---|---|---|
| `ROB-02-01` | **High** | **`elaboration/src` 高密度 729 处非测试 panic 表面**：虽然未见 `todo!()`/`unimplemented!()` 残留，但 `357` 个 `.unwrap()`、`100` 个 `.expect()` 与 `272` 个 `unreachable!()` 严重依赖前驱校验不出错；面对前端恢复态残缺 HIR 或复杂泛型错误传递极易转化为 compiler panic (ICE) | `open_functions.rs:66,88,288,289`, `interfaces.rs:235,238,476`, `program.rs:1299,4029` | 高频边界（尤其错误恢复后可到达的状态读取、表查找 `.get(...)`）改为显式结构化诊断返回 |
| `ROB-02-02` | **High** | **`ptc` 证明验证器与求解核心的 panic 路径**：[ptc/src/lib.rs:1492](file:///home/fin/luna/compiler/ptc/src/lib.rs#L1492) 在验证外部传入 `ProofTree::BooleanConflict` 时使用 `self.atoms.iter().position(...).unwrap()`，异常证明树将直接崩进程而非返 `ProofError::Invalid`；`phases.rs:232,238` 与 `search.rs:127,157` 同理依赖紧耦合不变量 | [ptc/src/lib.rs:1491-1493](file:///home/fin/luna/compiler/ptc/src/lib.rs#L1491-L1493), [ptc/src/search.rs:127,157](file:///home/fin/luna/compiler/ptc/src/search.rs#L127-L157), [phases.rs:232,238](file:///home/fin/luna/compiler/type_resolution/src/implementation/phases.rs#L232-L238) | 证明校验器 (`verify_proof`) 彻底零 `.unwrap()`，未知原子/越界全部安全返回 `Err(ProofError::Invalid)` |
| `ROB-02-03` | **Medium** | **局部递归缺少深度上限防爆栈**：全 crate 绝大多数算法（`matching.rs`、`ptc`、`coherence`、`applicability`、`refinements`）已正确配齐 `steps` 燃料与 `max_depth.min(128)` 双重上限，但 `match_default_strategy_term` 等部分新辅助递归仅扣总步数 `select_fuel = 100_000` 而未传 `depth` 上限 | [program.rs:2389-2650](file:///home/fin/luna/compiler/elaboration/src/program.rs#L2389-L2650) | 改为显式显栈迭代或传递 `depth >= 128` 深度护栏，杜绝深层嵌套类型触发原生调用栈溢出 |
| `ROB-02-04` | **Medium** | **`DUMMY_SP` / `Span::default()` 诊断定位丢失（43 处：`28` 个 `DUMMY_SP` + `15` 个 `Span::default()`）**：属性并合错误、默认衍生策略错误、生成结构/捕获构建及若干 `program.rs` / `dynamic_instances.rs` / `arguments.rs` 报错使用默认空 Span，用户收到 `0..0: <message>` 而无法定位源位置 | [program.rs:1419-1459,1681,2108,2208,2264](file:///home/fin/luna/compiler/elaboration/src/program.rs#L2208-L2266), [arguments.rs:835](file:///home/fin/luna/compiler/elaboration/src/program/arguments.rs#L835), [dynamic_instances.rs:680,751](file:///home/fin/luna/compiler/elaboration/src/program/dynamic_instances.rs#L680-L751), [derivation/captures.rs:11](file:///home/fin/luna/compiler/elaboration/src/program/derivation/captures.rs#L11), [struct_publisher.rs:154-164](file:///home/fin/luna/compiler/elaboration/src/definitions/struct_publisher.rs#L154-L164) | 从对应 `Attr`/`Derive`/`Item` 节点回传精确 `Span`，并保留 `note` 指向真实来源位点 |
| `ROB-02-05` | **Low** | **局部非 `use` `HashMap`/`HashSet` 审计（总计 161 处）**：核查显示 `Index` / `InherentIndex` / `CoherenceReport` / `DeclarationTheory` 等持久输出结构均已采用 `BTreeMap` / `BTreeSet`；现有 `HashMap`/`HashSet`（`elaboration=95`, `type_resolution=49`, `ptc=17`）主要用于内部记忆化与去重，但如 `model_search.rs:27` `representatives`、`calls.rs:87` `views` 与 `symbolic.rs:171` 须持续审计防止未来经迭代泄漏顺序敏感行为 | [model_search.rs:27](file:///home/fin/luna/compiler/ptc/src/model_search.rs#L27), [calls.rs:87](file:///home/fin/luna/compiler/type_resolution/src/calls.rs#L87), [symbolic.rs:171](file:///home/fin/luna/compiler/ptc/src/type_inputs/symbolic.rs#L171) | 约定凡参与反模型构造、错误汇报或产物序列化的映射一律切换为 `BTreeMap` / `IndexMap` |

---

## 7. 修复路径、依赖与验收建议

按依赖拓扑与阻塞紧迫度划分为 **P0–P3 四个交付阶段**：

### P0（止血、健全性与架构纠偏前置）
1. **消除 `SND-02-02` / `DIV-02-03`（`issues/0001` 闭包操作环境与语义捕获分离）**：
   - 依赖链路：`elaboration/closure_bodies.rs` & `program.rs:4921` + `materialization/src/lib.rs:2848-3220` + LLVM 闭包降码协同；
   - 将隐式传递的 `RuntimeId` 与用户显式声明的 `%{ v: T }` 语义字段严格拆槽，解除 `Box`/`Vec` `with_ref` 与泛型闭包工厂阻断；
2. **统一 `TD-02-02` `Type.predicates.*` 命名并接通 `IntLe` (`TD-02-01`)**：
   - 将 `nir::TypePredicate` 与 `Formula::from_predicate` 补齐 `IntLe`，同步裁决并落地短命名谓词表（保留长名短期废弃诊断）与 `Type.predicates.is_not_sized`（`GAP-02-06`）；
3. **修复 `SND-02-01` & `ROB-02-02`（默认衍生策略集匹配/连贯性与 PTC `verify_proof` 防 panic）**：
   - 修正 `match_default_strategy_term` 无序错误集比较、效应参数替换与 `depth <= 128` 深度护栏；消除 `ptc/src/lib.rs:1492` 等校验路径 `.unwrap()`。

### P1（通用机制替代特判，打通标准库与核心特性）
1. **清理 `DIV-02-01` `Tuple` / `meta.List<Type>` 编译器特判**：
   - 把 `BuiltinCallable`、`Role::{Tuple,TypeList}` 与专属分支收敛为通用可变长 comptime 列表协议与一般 NF 头处理规则，确保同样规则适用于用户定义（对齐 `AGENTS.md` §5）；
2. **补齐双相解析对称性 (`GAP-02-16` / `DIV-02-02`) 与 `where` 局部推断盲区 (`GAP-02-24`)**：
   - 实现固有 `comptime impl` 与普通 trait `^attrs.comptime_phase` 方法的 comptime 优先级；
   - 让 `with_partial_named_call_plan` 同时覆盖 `comptime fn`、静态/特质方法调用（`explicit_method_instance`）及含变参/可选混合参数场景；
3. **解除 `derive fn` 核心阻断 (`GAP-02-18`)**：
   - 优先打通策略 `where` 子句（如 `where T :- Eq` / `quote .allocator`）与独立未使用策略检查，支持 `std` 容器自动衍生协议。

### P2（核心语言能力补完与高级扩展）
1. **打通 `dyn Trait` 全链路 (`GAP-02-19`)**：
   - 依据 [dynamic-interfaces.md](file:///home/fin/luna/docs/traits/dynamic-interfaces.md) 从 `ast_lowering -> hir -> nir -> type_resolution -> elaboration` 串联双指针胖指针/vtable 布局、`%c dyn Trait`、`Self` 重装箱与参数同型保护；
2. **补齐 PTC 核心原子 (`A03-A09, A13-A19`) 与反模型完备性 (`CMP-02-01`)**：
   - 按需求批次接入 `HasHeadShape`、`IntCongruent`、`BVUnsignedLe`/`BVSignedLe`、`RowContains`/`RowSubset`、`PredicateCall`，以及 `Type.vars.{EffectRow,ErrorRow,Reachability,Any}`；
3. **推进 `local-dependencies.md` 统一调度抽象 & 循环回边精化 (`GAP-02-25`, `GAP-02-26`)**：
   - 统一 `Goal`/`WaitTicket`/并查集 watcher 转发并加强循环体内部局部依赖与精化不变量的回边收敛。

### P3（工程化、Wishlist 反射与增量架构演进）
1. **接入 `compiler/middle` 增量查询与诊断 Span/Panic 治理 (`GAP-02-27`, `ROB-02-01`, `ROB-02-04`)**：
   - 在 `middle::Providers` 中逐步纳入类型签名、特质索引与 elaboration 产物缓存；消除高频 `open_functions.rs`/`interfaces.rs` 中的 `unwrap()` 与 `DUMMY_SP`/`Span::default()`，并清除归档测试模块 `gradual`/`type_computations`（`TD-02-03`）；
2. **扩展 `tmp/wishlist/01,03,05` 反射生态 (`GAP-02-28`..`30`)**：
   - 开放 `RecordInfo`、`fields_of`/`methods_of`、`stride_of`/`field_offset_of`/`type_id`（128-bit Merkle）、`is_zst`/`has_niche`/`needs_drop` 及通用 `VariadicCollector` / `ImplicitPassing`。

### 验收测试建议
- 针对 P0–P1：在 `compiler/luna/tests/` 增加 ① 带隐式 `quote .allocator` 的 `Box<String>`/`Vec<String>` 经 `with_ref`/`try_for_each_ref` 回调触发 `format_debug` 的真实 sysroot 端到端原生测试；② `recv.method<.A = MyAlloc>(val)` 方法局部具名推断测试；③ 同一普通 `trait` 内 `runtime`/`comptime` 同名方法经 `^attrs.comptime_phase` 在 comptime 上下文无歧义自动选相测试；④ 用户自定义变参 NF 头（非 `Tuple`/`meta.List`）使用一般化列表收集协议并保持 `Head<T> != T` 的正反测试；⑤ 无序 `ErrorResult` 变体在默认衍生策略匹配中的集合等价正例与重叠拒绝例。
- 针对 P2–P3：补充 `dyn Trait`、`derive fn ... where ...` 泛型容器衍生、`IntLe`/`PredicateCall` 精化裁剪与畸形 `ProofTree` 拒斥测试（确认返回 `ProofError::Invalid` 而非 panic）。
