# 04 · Comptime、CMTT、反射与官方属性全量差距审计报告

> 审计基线：仓库 `dev` 分支 `HEAD = b9a52238`（含 `d20933c0` `__flurry_packed_mode` 与 `d0f09c9a` `BitVec` 位级 `attrs.packed.bit` 等最新落地提交）。
> 审计方式：基于实时 Rust/Flurry 源码与测试用例人工核查 + 非测试源码静态定量普查；未运行 `cargo build` / `cargo test`。所有状态结论与行号均来自当前 `HEAD` 源码直接观测，并核销了 `ai_dev_logs/comptime_coverage_report_2026-11-21.md` 中的过时结论。

---

## 1. 审计范围、检查基线与设计标尺

### 1.1 源码审查范围
- **Comptime 双引擎、执行器与运行时基础库**：
  - `compiler/comptime_interpretor/src/`（树求值器、`bytecode.rs` 字节码执行引擎、`value_reference.rs` 值引用、`observations.rs` / `reflection/`、`definition_build.rs` / `struct_build.rs` / `enum_build.rs` / `newtype_build.rs`）
  - `compiler/comptime_compiler/src/`（字节码编译器 `lib.rs`、`patterns.rs`、`leaf_patterns.rs`、`methods.rs`、`constants.rs`、`normal_types.rs`）
  - `compiler/comptime_executor/src/lib.rs`（`QueryId` / `Scope` / `KeyProtocol` 记忆化缓存内核）
  - `compiler/comptime_runtime/src/`（`Integer`、`BitVec`、`MachineInteger`、`Object`、`DecimalValue`、`Float16Format`）
  - `compiler/luna/src/memoization.rs`（驱动侧纯 comptime 查询缓存适配）
- **Staging、Freezing 与常量物化**：
  - `compiler/iir/src/staging.rs` 及 `compiler/iir/src/staging/{frozen.rs, general.rs, preparation.rs, error_values.rs}`
  - `compiler/elaboration/src/program/required.rs`、`required/{inputs.rs, prepared.rs, restore.rs}`、`constants/materialization.rs`、`constants/materialization/captures.rs`、`splicing.rs`、`derivation.rs`
  - `compiler/type_resolution/src/{lib.rs, obligations.rs}`（推理变量冻结边界）
- **CMTT（上下文模态类型论）**：
  - `compiler/elaboration/src/program/cmtt.rs`、`cmtt/{functions.rs, function_templates.rs, patterns.rs}`
  - `compiler/elaboration/src/program/cmtt/generators.rs`、`cmtt/generators/{matches.rs, methods.rs, methods/traits.rs, places.rs, inference.rs, statements.rs, ...}`、`cmtt/patterns/generators.rs`
  - `compiler/elaboration/src/program/block_regions/matches.rs`、`expression_values/special.rs`
- **反射、声明构建与官方属性**：
  - `compiler/elaboration/src/program/{projections.rs, type_data.rs, expression_values/views.rs, builtin_bindings/signatures.rs, builtin_bindings/values.rs}`、`compiler/elaboration/src/definitions/`
  - `compiler/nir/src/{types.rs, layout.rs, program.rs, program/definition_attributes.rs, phases.rs}`
  - `compiler/elaboration/src/program/attributes.rs` 与 `attributes/{derivation.rs, effects.rs, layout.rs, layouts.rs, merge.rs, metadata.rs, phases.rs, volatile.rs}`、`compiler/luna/src/{frontend.rs, macro_host.rs}`
- **Sysroot 库与端到端测试对照**：
  - `library/builtin/protocols/staging.fl`、`library/meta/{lib.fl, bindings.fl, reflection/structure.fl, definitions/*.fl, code/README.md, README.md}`、`library/attrs/{lib.fl, README.md, **/*.fl}`
  - `compiler/luna/tests/{cmtt.rs, cmtt_methods.rs, quote_functions.rs, comptime_pointer_phase.rs, comptime_receivers.rs, comptime_memory.rs, staged_callable_freezing.rs, attrs_and_meta_reflection.rs, type_reflection_layout.rs, source_reflection.rs, builder_invocations.rs, library_attributes.rs}`

### 1.2 权威标尺与扩展标尺区分
依据 `AGENTS.md` 权威层级规定，本报告严格采信权威设计基线，不采信早期历史方案文档、2026-05 前的归档草稿及非权威教学材料，并明确区分两类标尺：
1. **核心规范标尺（Core）**：
   - `AGENTS.md` §5（禁止 comptime 指针并采用值引用、S-expression `Display`、禁止使用处直写 `.symbol'builtin`、禁止为 `Tuple`/`meta.List` 特判、NF 头不展开、`quote fn` 不属于 `comptime fn` 等）
   - `docs/comptime/{index.md, attribute.md, official-attribute-keys.md, normalization-and-instantiation.md, purity-and-normal-form.md, the-comptime-phases-of-traits.md, inlining-comptime-code.md, quoting-and-splicing.md}`
   - `docs/cmtt/{index.md, any-and-code-evidence.md, contexts-and-substitution.md, program-construction.md, quotation-and-stages.md, quote-functions-and-templates.md, patterns-and-arms.md}`
   - `docs/type/{reflection-and-layout.md, type_id.md}`、`docs/compilation/staging-and-freezing.md`
   - 现存阻塞追踪：`issues/{0002, 0003}-*.md`
2. **Wishlist / Silver 扩展标尺（Wishlist）**：
   - `tmp/wishlist/05-meta-cmtt-macros-reflection.md`（`WL-05-01`..`WL-05-32`）
   - `tmp/wishlist/06-attrs-build-pipeline-projection.md`（`WL-06-01`..`WL-06-21`）
   - `tmp/silver-design/04-compiler-wishlist.md`（含 **Silver P0-2**：匿名记录 `Ps` / `Term::Record` 反射缺口）

### 1.3 6 级成熟度评估标尺
- `🟢 Full`：设计与实现完全闭合，语义、检查、IR/VM、sysroot 库接口及测试一致覆盖。
- `🟡 Partial`：主链路已落地并可用，但存在明确的分支/类型/阶段覆盖缺口、降级缺失或接线未贯通。
- `🟠 Frontend-Only`：前端/HIR/域校验或内部原型已实现，但 VM/求值器/库公开接口尚未接通。
- `🔴 Missing`：设计目标已确立，但编译器及库均未实现（或直接以 `"not yet implemented"` 拒绝）。
- `⚠️ Divergent`：当前实现或权威文档本身违反了 `AGENTS.md` §5 或已确认的 `docs/` 语义契约（技术债/规则冲突）。
- `⚪ Deferred`：按作者明确决定暂缓的高级特性。

---

## 2. 成熟度总览与统计表

### 2.1 全量特性差距（`GAP-04-01` ~ `GAP-04-46`）成熟度分布

| 子领域 | 总计 | 🟢 Full | 🟡 Partial | 🟠 Frontend-Only | 🔴 Missing | ⚠️ Divergent | ⚪ Deferred |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| A. Comptime 双引擎、Memoization、值引用与 Display (`01`–`08`) | 8 | 2 | 5 | 0 | 0 | 1 | 0 |
| B. Staging、Freezing 与 `Splice` (`09`–`15`) | 7 | 2 | 2 | 0 | 3 | 0 | 0 |
| C. CMTT (`cmtt.rs`, `contextual_code.rs`, `meta.code`) (`16`–`25`) | 10 | 1 | 2 | 0 | 6 | 1 | 0 |
| D. 反射、构建器、`Type.vars` 与 `Type.type_id` (`26`–`38`) | 13 | 6 | 2 | 1 | 4 | 0 | 0 |
| E. 官方属性系统与 `library/attrs` (`39`–`46`) | 8 | 1 | 2 | 1 | 3 | 1 | 0 |
| **合计（`GAP-04-01`–`GAP-04-46`）** | **46** | **12** | **13** | **2** | **16** | **3** | **0** |

> 标尺归属切分：**Core 核心规范项 39 项**（`🟢 Full 12`、`🟡 Partial 12`、`🟠 Frontend-Only 1`、`🔴 Missing 11`、`⚠️ Divergent 3`）与 **Wishlist / Silver 扩展项 7 项**（`GAP-04-08`, `29`, `30`, `34`, `38`, `44`, `45`，含 `🟡 Partial 1`、`🟠 Frontend-Only 1`、`🔴 Missing 5`）。

### 2.2 健全性 / 完备性 / 鲁棒性（`SND` / `CMP` / `ROB`）专项统计

| 专项维度 | 发现项数 | Critical | High | Medium | Low | 核心风险焦点 |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 健全性 (`SND-04-NN`) | 5 | 0 | 2 | 2 | 1 | `attributes.rs:140` `domain.is_some()` 绕过白名单且无消费者；CMTT 残留 comptime 指针路径；`quote_holes` 未按互斥分支隔离 |
| 完备性 (`CMP-04-NN`) | 5 | 0 | 4 | 1 | 0 | `iir/src/staging.rs:3254` 阻断闭包常量；字节码禁用 fallback 且拒绝对外 `TemplateEqual`/effect/非 bool `And`/`Or`；CMTT `Any`/bodyless `quote fn`/quoted arm 缺失；`Term::Record` 反射与 `Type.type_id` 缺失 |
| 鲁棒性 (`ROB-04-NN`) | 4 | 0 | 0 | 3 | 1 | 240 个非测试文件（84,385 行）含 `484` 处 panic 边界（189 `.unwrap()`, 83 `.expect()`, 212 `unreachable!()`, 0 `panic!()`）、`28` 处零 span 降级、`118` 处无序 `HashMap/HashSet` |
| **合计** | **14** | **0** | **6** | **6** | **2** | — |

---

## 3. 全量特性差距矩阵（GAP-04-NN）

### A. Comptime 双引擎执行、Memoization、值引用与 Display

| ID | 特性名称 | 标尺来源 | 成熟度 | 源码位置与真实状态证据 | 关联测试 |
| --- | --- | --- | --- | --- | --- |
| `GAP-04-01` | 字节码编译覆盖与无回退主路由 | Core (`docs/comptime/index.md`) | 🟡 Partial | `bytecode.rs:843-848` 在 `comptime_only && bytecode_enabled && cleanup_schedule.is_none() && effect_state.environment.is_empty()` 时默认走字节码，编译报错直传不回退树求值器（`invocation/session.rs:365-400`, `bytecode_compilation.rs:384-390`）。`HEAD` 已支持 NF 调用 `TypeInputPlan`、`Bool`/`BitVec` 短路 `And`/`Or`、`Closure` 与 `Build*`（`comptime_compiler/src/lib.rs:1752,1886-1940,2225-2259,3179-3242`；修正 `ai_dev_logs/comptime_coverage_report_2026-11-21.md:54` 误判）；仍不支持非 bool/BitVec `And`/`Or`（`L3304`）、`K::TemplateEqual`（`L3321`）、`StagedValue`/pointer/effect（`L3307-3320`）、非叶模式（`leaf_patterns.rs:19-24`）及 `depth != 0` 的 `start_staged_input`（`bytecode.rs:916`） | `bytecode_compilation.rs`, `comptime_memory.rs` |
| `GAP-04-02` | 纯 Comptime 查询 Memoization (`comptime_executor`) | Core (`docs/comptime/purity-and-normal-form.md`) | 🟡 Partial | `comptime_executor/src/lib.rs:1-274` 实现了按 `QueryId` 与 `KeyProtocol` 记忆化，但 `luna/src/memoization.rs:228,350` 硬编码仅缓存 `result_type == Term::Primitive(Primitive::Type)`，所有非 `Type` 纯调用绕过结果缓存；且 `elaboration/src/program/required*` 并未接入 `comptime_executor` | `comptime_memory.rs:317-345` (`memoization_driver_reuses_code_without_caching_non_type_results`) |
| `GAP-04-03` | Comptime `self` 值引用语义与无指针契约 | Core (`AGENTS.md` §5, `docs/comptime/index.md`) | ⚠️ Divergent | 普通 `comptime fn` 禁用 `*self`（`program.rs:3146-3165`）与指针操作（`nir/phases.rs:739-755`, `comptime_interpretor/src/lib.rs:2450-2453`），`value_reference.rs:1-80` 实现局部值引用。**但 CMTT 生成器仍残留大量 comptime 指针路径**：`cmtt/generators.rs:315-316,2305,2361-2367`、`cmtt/patterns/generators.rs:393-401`、`methods.rs:433-448`、`methods/traits.rs:213-229`、`places.rs:311`、`inference.rs:124-136` 仍生成/接收 `Term::Pointer` 或 `H::Ref(base)`/`H::Deref`；`list_patterns.rs:54-62` 保留 `K::AddressOf` 分支；且 4 份文档仍留有旧指针示例（见第 5 节） | `comptime_pointer_phase.rs`, `comptime_receivers.rs`, `cmtt.rs:856-871` |
| `GAP-04-04` | Comptime 阶段代数效应与析构调度执行 | Core (`docs/comptime/index.md`) | 🟡 Partial | 树求值器支持 `effects.rs` 与 `cleanup.rs`；一旦存在 `cleanup_schedule` 或非空 `effect_state.environment`，`bytecode_invocation_available()`（`bytecode.rs:843-848`）立即关闭字节码；`comptime_compiler/src/lib.rs:3313-3317` 对全部 `K::Effect*` 直接返回 `Error::Unsupported` | `comptime_memory.rs`, `effects` 测试 |
| `GAP-04-05` | Comptime IR S-expression `Display` | Core (`AGENTS.md` §5) | 🟢 Full | `comptime_compiler`（`Program` `lib.rs:608`, `Method` `methods.rs:77`, `DeclarationInputPlan` 等）、`comptime_runtime`（`Integer`, `BitVec`, `MachineInteger`, `Object`, `Attributes`, `DecimalValue`, `Float16Format`）、`comptime_executor`（`QueryId`, `Scope`）及 `comptime_interpretor`（`Value` `lib.rs:4848`, `Frame`, 反射/构建视图）均完整输出可解析 S-expression | 各模块内联 `display` / S-expr 单测 |
| `GAP-04-06` | 局部 `const` 与 `inline if/when/for`/循环转移 | Core (`docs/comptime/inlining-comptime-code.md`) | 🟢 Full | `comptime_interpretor` 与 `comptime_compiler/src/lib.rs:3150-3287` 完整支持 `If`/`Match`/`Loop`/`Break`/`Continue` 及词法循环跨边界转移，配合 `local` `const` 求值物化 | `comptime_inline_loops`, `inline_control` |
| `GAP-04-07` | 结构化元组展开 `'tuple_enumerate` / `'tuple_indexed` | Core (`docs/comptime/inlining-comptime-code.md`) | 🟡 Partial | `expression_values/views.rs:190-203`：`'tuple_enumerate` 无条件拒绝 `"tuple_enumerate structural iteration is not yet implemented"`；`'tuple_indexed` 仅在 `inline for` 语境下可用（否则 `L200-202` 报错） | — |
| `GAP-04-08` | Comptime VM 跟踪诊断与统一 Span 精度 | Wishlist (`WL-05-30/P1`) | 🟡 Partial | 字节码与求值器保存 `SourceRange` 与 `Event`（`comptime_interpretor/src/lib.rs:710`），但非测试代码存在 28 处 `SourceRange { start: 0, end: 0 }` / `DUMMY_SP` / `Span::default()` 退化（如 `admission.rs:235,248`, `attributes.rs:182`） | `compiler/comptime_interpretor/src/lib.rs` |

### B. Staging、Freezing 与 `Splice`

| ID | 特性名称 | 标尺来源 | 成熟度 | 源码位置与真实状态证据 | 关联测试 |
| --- | --- | --- | --- | --- | --- |
| `GAP-04-09` | Staged 常量路径对 `K::Closure` 的支持 | Core (`docs/compilation/staging-and-freezing.md`, `issues/0003`) | 🔴 Missing | `elaboration/src/program/constants/materialization.rs:829-945` 支持 `const` 中物化 `Value::Closure`，但 `compiler/iir/src/staging.rs:3254`（注：`HEAD` 位于 `L3254`，非 `issues/0003:11` 所记 `3173`）在 `constant()` 中命中 `_ => return Err("staged input requires a supported canonical constant".into())`；`staging/preparation.rs:277` 与 `staging/frozen.rs:126` 同步阻断函数/闭包 staged 输入与折叠 | `staged_callable_freezing.rs`（仅覆盖 `FunctionInstance` 命名回调，不覆盖闭合捕获闭包） |
| `GAP-04-10` | `Splice` 协议跨阶段值类型覆盖 | Core (`docs/comptime/quoting-and-splicing.md`, `cmtt_methods.rs:231`) | 🟡 Partial | `library/builtin/protocols/staging.fl:1-167` 与 `formatting/options.fl:46` 仅为标量原语（`bool`, `i*/u*`, `f*`, `char`, `Symbol`, `void`）和 `FormatOptions` 实现 `Splice`；`str`、元组、记录、函数指针与闭包均无官方实现 | `cmtt_methods.rs:231` (`#[ignore = "builtin has no runtime callable Splice implementation yet"]`) |
| `GAP-04-11` | 有限 Nominal/元组结构 `Splice` 自动派生 | Core (`docs/comptime/quoting-and-splicing.md`) | 🔴 Missing | `elaboration/src/program/derivation.rs:1-120` 仅处理显式 `Deriver` 策略声明，无按字段递归合成 `Splice` 实现的编译器派生支持 | — |
| `GAP-04-12` | `quote fn` 实参单次求值与逻辑返回边界 | Core (`docs/cmtt/quote-functions-and-templates.md`) | 🟢 Full | `cmtt/functions.rs:256-404, 547-700` 将位置实参求值一次到新鲜局部绑定，`contract_returns` 与 `checked_result` 为 `K::Return` 注入后置条件校验并展开 `K::EffectComputation` 边界 | `quote_functions.rs` |
| `GAP-04-13` | 冻结安全边界（禁止冻结活动元变量/等待者） | Core (`docs/compilation/staging-and-freezing.md`) | 🟢 Full | `type_resolution/src/lib.rs:310-314` 与 `obligations.rs:768-771` 在冻结阶段严格拒绝未绑定的 `InferenceTerm::Meta` 及挂起等待者 | `required/prepared.rs:1976-2001`, `staged_callable_freezing.rs` |
| `GAP-04-14` | `ordinary` vs `loose` 双模式规范化 | Core (`docs/comptime/normalization-and-instantiation.md`) | 🔴 Missing | `normal.rs`、`required.rs`、`iir/src/staging/` 仅实现严格的 `ordinary` 不透明 NF 保持与延迟求值，代码库中完全不存在 `loose` 规范化模式及授权近似展开路径 | — |
| `GAP-04-15` | `quote` 内部对 Comptime 数据/绑定的隐式提升 (`lift`) | Core (`docs/cmtt/index.md`, `quoting-and-splicing.md`) | 🟡 Partial | `splicing.rs` 与 `Splice` 协议负责参数化传递，但 `cmtt.rs:485-493` 直接拒绝在 `'(...)` 中直接捕获外层 comptime 绑定：`"quote contexts contain only runtime bindings; comptime data lifting is not yet implemented"` | `cmtt.rs` |

### C. CMTT (`cmtt.rs`, `contextual_code.rs`, `meta.code`)

| ID | 特性名称 | 标尺来源 | 成熟度 | 源码位置与真实状态证据 | 关联测试 |
| --- | --- | --- | --- | --- | --- |
| `GAP-04-16` | Rank-1 `< Ψ \|- T >` 与 `< T -\| Ψ >` 上下文代码与卫生连接 | Core (`docs/cmtt/`) | 🟢 Full | `cmtt.rs`, `cmtt/patterns.rs`, `contextual_code.rs`, `cmtt/generators.rs` 实现 rank-1 端口上下文检查、`freshen` 同步 alpha 重命名、`connect_code` 捕获转发与 `PatternCode` 卫生输出 | `cmtt.rs`, `cmtt_methods.rs`, `cmtt_phase_boundaries.rs` |
| `GAP-04-17` | 单洞规则（互斥分支独立计数 vs 全局平铺） | Core (`docs/cmtt/quotation-and-stages.md`) | ⚠️ Divergent | `cmtt/functions.rs:81-92` 的 `record_quote_hole` 将位置跨度 `(span.lo().0, span.hi().0)` 存入扁平 `quote_holes: BTreeSet<(u32, u32)>`，仅在 `quoted_runtime_boundary`（`cmtt.rs:542`）与 `pattern_code_binding_mode`（`cmtt/patterns.rs:169`）清零保存，**未按 `if`/`else` 或 `match` 互斥控制流分支分叉隔离**，错误拒绝不同分支各含一个 splice 的合法程序 | `cmtt.rs:1499,3347`, `quote_functions.rs:585,724` |
| `GAP-04-18` | `Any` / `any` 代码证据类型恢复与约束检验 | Core (`docs/cmtt/any-and-code-evidence.md`) | 🔴 Missing | `cmtt.rs:198-202` 在 `code_interface` 中直接报错拒绝：`!matches!(annotation.kind, H::ContextualExprType(..))` 即返回 `"code annotation requires an expression-code interface; Any evidence is not yet implemented"` | — |
| `GAP-04-19` | 无体 `quote fn` 声明（仅签名，供后续组装） | Core (`docs/cmtt/quote-functions-and-templates.md`) | 🔴 Missing | `cmtt/functions.rs:32-37` 直接拒绝非模板的无体 `ItemKind::FnDeclaration`：`Err(error(span, "quote fn declaration requires arm or template assembly, which is not yet implemented"))` | — |
| `GAP-04-20` | Contextual `quote fn` 输入与依赖参数协议 | Core (`docs/cmtt/quote-functions-and-templates.md`) | 🔴 Missing | `cmtt/functions.rs:50-58` 拒绝任何除 `TPARAM_IMPLICIT \| TPARAM_LAMBDA \| TPARAM_COMPTIME` 之外的参数标志或非平凡参数形态：`"contextual quote fn inputs or dependent parameter protocols are not yet implemented"` | — |
| `GAP-04-21` | Quoted Match Arms（`match` 内 `use expr` + 模板属性） | Core (`docs/cmtt/quote-functions-and-templates.md`) | 🔴 Missing | 全局检索 `ArmImport`、`for_match_arms`、`allow_non_exhaustive_instance` 在前端（`parse`、`hir`、`block_regions/matches.rs`）均为 0 实现；`library/attrs/quote_fn_template/README.md:3` 明确标注“待实现” | — |
| `GAP-04-22` | `code_match` 与 `code_sequence` 高阶组合原语 | Core (`docs/cmtt/patterns-and-arms.md`) | 🟡 Partial | `cmtt/generators/matches.rs:46` 实现生成期 `generator_code_match`；但 `code_sequence` 在全仓库 0 命中，且无独立公开库原语 | `cmtt.rs:2209-2533` |
| `GAP-04-23` | `meta.code` 公开标准库代码组装 API | Core (`docs/cmtt/index.md`, `library/STRUCTURE.md`) | 🔴 Missing | `library/meta/code/` 下仅存 `README.md`（1–3 行“待实现公开库接口”），未导出任何用户可用的代码构造/反射类型或函数 | — |
| `GAP-04-24` | 代码集合容器与容器化代码证据 | Core (`docs/cmtt/program-construction.md`) | 🔴 Missing | `cmtt/generators.rs:305-308` 显式拒绝代码类型的集合输入：`"code collections require code-container evidence not yet implemented"` | — |
| `GAP-04-25` | `issues/0002` CMTT/模板与控制流局部常量前序探针 | Core (`issues/0002`) | 🟡 Partial | `issues/0002:67-129` 中的 `Outer<flag>.Inner.FLAG`（`f305feed`）、条件 const 失败分支发散（`4089d99f`）、嵌套 `do`/`if`/`when`/`match` 退化（`61c1370c`, `57858f6c`, `64e71e54`, `37aeaf3c`）已合入修复；但**闭合 runtime 函数失败路径读取缺席 `const`**（`0002:96,131`）与 **runtime `match` 整体结果类型合并不可达发散臂**（`0002:132`）仍开放 | `test_templates` (`issues/0002:39`) |

### D. 反射、声明构建器、`Type.vars` 与 `Type.type_id`

| ID | 特性名称 | 标尺来源 | 成熟度 | 源码位置与真实状态证据 | 关联测试 |
| --- | --- | --- | --- | --- | --- |
| `GAP-04-26` | Struct 结构反射 (`struct_field_{names,types}`, `struct_fields`) | Core (`docs/type/reflection-and-layout.md`) | 🟢 Full | `reflection.rs:44-147`、`observations.rs:84-85,373`、`builtin_bindings/signatures.rs:217-218` 与 `library/meta/reflection/structure.fl:74-100` 贯通 | `source_reflection.rs:27-900` |
| `GAP-04-27` | Enum 结构与编码反射（路径/形状/Symbol/FixedBits） | Core (`docs/type/reflection-and-layout.md`) | 🟢 Full | `reflection/enums.rs`、`reflection/enums/bits.rs`、`observations.rs:114-137,165-196`、`structure.fl:46-120` 贯通（不含推算未声明判别值） | `source_reflection.rs:943-989`, `builder_invocations.rs:2009-2793` |
| `GAP-04-28` | Newtype 直接表示反射 (`NewtypeRepresentation`) | Core (`docs/type/reflection-and-layout.md`) | 🟢 Full | `reflection/newtypes.rs:19-77`、`observations.rs:139-149,361-370`、`structure.fl:41-43` 完整保留 opaque 头不展开 | `builder_invocations.rs:3102-3159` |
| `GAP-04-29` | **Anonymous Record `Ps` (`Term::Record`) 反射与 `is_record`** | Wishlist (**Silver P0-2**, `tmp/silver-design/04`, `tmp/wishlist/05`) | 🔴 Missing | `nir/src/types.rs:261-263` 在 `TypeKind::evaluate` 对 `Term::Record(_)` 直接返回 `Some(false)`；`projections.rs:156-171` 无 `is_record`；`reflection.rs:101-103` 只接受 `Term::Nominal`，无法读取匿名记录 `Ps` 的字段名列表与字段类型，阻塞 Silver 泛型 `Ps` 协议/容器 | — |
| `GAP-04-30` | `Type.predicates` 扩展轴 (`is_union`, `is_char`, `is_optional`, `is_error_result`) | Wishlist (`tmp/wishlist/05`) | 🔴 Missing | `nir/src/types.rs:164-175` 与 `projections.rs:156-171` 仅枚举 12 个闭合轴（`is_boolean`..`is_function`），`is_char`、`is_union`、`is_record`、`is_optional`、`is_error_result` 均缺失 | `type_reflection_layout.rs:108-255` |
| `GAP-04-31` | 可调用/Trait/Impl/Module 反射 (`reflect_callable` 导出与成员反射) | Core (`reflection-and-layout.md`; 扩展见 `WL-05`) | 🟡 Partial | `reflection/callables.rs:40-57` 在 Rust 层实现了 `Interpreter::reflect_callable`，但**完全未绑定到 `builtin_bindings/signatures.rs` 或 `library/meta/`**；Trait、Impl、Module、函数名与字段属性反射均未实现 | `comptime_interpretor/tests/callable_reflection.rs` |
| `GAP-04-32` | `Type.size_of` / `Type.align_of` 目标感知布局查询 | Core (`docs/type/reflection-and-layout.md`) | 🟢 Full | `nir/src/layout.rs:206-620`、`observations.rs:521-562`、`projections.rs:234-262` 按编译目标计算布局并检查循环、目标字宽与 comptime 域限制 | `type_reflection_layout.rs:257-436` |
| `GAP-04-33` | 事务性具体 `StructBuilder` / `EnumBuilder` / `NewtypeBuilder` | Core (`docs/type/reflection-and-layout.md`) | 🟢 Full | `definition_build.rs`、`struct_build.rs`、`enum_build.rs`、`newtype_build.rs`、`checked_definition_publisher.rs` 与 `meta.definitions` 支持 `Object` 描述、Symbol/FixedBits 与事务性回滚 | `builder_invocations.rs`, `source_reflection.rs` |
| `GAP-04-34` | 泛型/子句化定义构建器与 `Union`/`Trait`/`Impl`/`Fn` 构建器 | Wishlist (`tmp/wishlist/05`, `definitions/README.md:96-107`) | 🟠 Frontend-Only | `compiler/elaboration/src/definitions.rs` 具备内部泛型 HIR 批发布能力（`assemble_struct_definition`）；但 VM 的 `BuildStruct`/`Enum`/`Newtype` 拒绝泛型/子句/捕获（`newtype_build.rs:112-116`），且 `UnionBuilder`/`TraitBuilder`/`ImplBuilder`/`FunctionBuilder` 缺失 | `compiler/elaboration/tests/definitions.rs` |
| `GAP-04-35` | `Type.vars.{Closure, Reachability, ...}` 存在量化家族 | Core (`library/README.md:429`, `parse/diagnostics.rs:852`) | 🟡 Partial | `type_data.rs:155-163` 仅硬编码识别 `Type.vars.Closure` 映射为 `Term::ClosureContextType`（供 `library/builtin/protocols/iterator.fl`、`library/meta/ast_impl/token_buffer.fl:124` 等使用）；`Reachability`（`&s T`）及开放/自定义参数家族均未接入 | `closure_environment_parameters.rs:49-61,450-465,517-575` |
| `GAP-04-36` | `Type.type_id` (128-bit 稳定 Merkle ABI 指纹) | Core (`docs/type/type_id.md`, `wishlist/05`) | 🔴 Missing | 全仓库 `nir`、`elaboration`、`comptime_interpretor` 与 `library/{builtin,meta}` 中 `type_id` 相关实现数为 `0` | — |
| `GAP-04-37` | `Target'attribute` 合并属性反射与延迟依赖图 | Core (`docs/comptime/attribute.md`) | 🟢 Full | `views.rs:210-221`、`observations.rs:251-349`、`attributes/demands.rs` 支持任意声明/NF 实例读取合并后冻结 `Object` 并检测属性环 | `attrs_and_meta_reflection.rs:729-950`, `source_reflection.rs:854` |
| `GAP-04-38` | `'reflect` 统一词法反射后缀视图 | Wishlist (`tmp/wishlist/05`) | 🔴 Missing | `hir/src/expr.rs:240-249` 与 `parse/src/` 无 `TakeReflect` 节点，仅保留 `'attribute`/`'tag_name`/`'bitvec`/`'old` 等专项后缀 | — |

### E. 官方属性系统 (`__flurry_*`、`attributes.rs`、消费者与 `library/attrs`)

| ID | 特性名称 | 标尺来源 | 成熟度 | 源码位置与真实状态证据 | 关联测试 |
| --- | --- | --- | --- | --- | --- |
| `GAP-04-39` | 核心已落地的 6 组官方 `__flurry_*` 属性族 | Core (`docs/comptime/official-attribute-keys.md`) | 🟢 Full | `__flurry_comptime_phase`（`phases.rs:73`）、`__flurry_packed_mode`（`layout.rs:39`）、`__flurry_volatile`（`volatile.rs:65` + LLVM volatile）、`__flurry_direct_passing`（`effects.rs:37` + `program.rs:3489`）、`__flurry_metadata`（`metadata.rs:36`）、`__flurry_macro_{kind,variadic,grammar}`（`attributes.rs:127-129` + `macro_host.rs:1449`）全链打通 | `attrs_and_meta_reflection.rs`, `packed_structs.rs`, `volatile_attribute.rs`, `library_attributes.rs` |
| `GAP-04-40` | `__flurry_abi` 调用约定与覆盖范围 | Core (`docs/comptime/official-attribute-keys.md`) | 🟡 Partial | `attributes.rs:108-125` 与 `nir/program/definition_attributes.rs:37` 仅接受 runtime `Fn`/`FnDeclaration` 且仅支持 `.c`（`Symbol("c")`）；`.system`/`.stdcall`/`.win64`/`.sysv64`、函数指针类型与 `static` 均不支持 | `library_attributes.rs:31-119`, `attrs_and_meta_reflection.rs:604` |
| `GAP-04-41` | `__flurry_derivation_default` 消费与 `attrs.derivation.default` | Core (`docs/comptime/official-attribute-keys.md`) | 🟡 Partial | `attributes/derivation.rs:39-69`、`program.rs:2188`、`luna/src/frontend.rs:81` 已消费 `Deriver` 上的 `__flurry_derivation_default`；**但 `library/attrs/derivation/` 仍未导出 `.default`**（阻塞于 `default` 保留关键字语法，`derivation/README.md:3-6`） | `attrs_and_meta_reflection.rs:109` |
| `GAP-04-42` | `__flurry_stable_since` ABI 稳定版本锚点 | Core (`docs/comptime/official-attribute-keys.md`, `type_id.md`) | 🟠 Frontend-Only | `attributes.rs:129` 校验 `P::Str` 域，`attrs.stable_since(version)` 写入 `{ __flurry_stable_since: version }` 并可供 `'attribute` 读取；但**既无 SemVer 格式校验，也无目标种类校验，且全编译器零 `type_id` 语义消费者** | `attrs_and_meta_reflection.rs:948-961` |
| `GAP-04-43` | 遗留 `packed` 布尔属性键与过渡包装共存 | Core (`docs/comptime/official-attribute-keys.md`) | ⚠️ Divergent | `d20933c0`/`d0f09c9a` 已打通 `__flurry_packed_mode`，但 `attributes.rs:145`、`attributes/layout.rs:40,94-127`、`library/attrs/layout_impl/constructors.fl:10-15`、`library/attrs/lib.fl:20-22`、`library/meta/reflection/structure.fl:132-139` 仍保留非 `__flurry_*` 命名空间的裸 `packed` 键及旧 `attrs.layout.{packed, packed_layout}` | `attrs_and_meta_reflection.rs:351-509`, `packed_structs.rs` |
| `GAP-04-44` | 布局与优化指令属性 (`__flurry_align`, `__flurry_inline`, `__flurry_branch_hint`, `__flurry_target_feature`, `__flurry_caller_location`) | Wishlist (`tmp/wishlist/06`) | 🔴 Missing | 均不在 `attributes.rs:108-151` 域校验或消费者白名单内，全部在 `L153-158` 报错 `"elaboration is not implemented for definition attribute or modifier..."`；`library/attrs/` 无对应构造器 | — |
| `GAP-04-45` | 编译管线、效应策略与 FFI 链接属性 (`__flurry_pipeline`, `__flurry_pipeline_projection`, `__flurry_effect_strategy`, `__flurry_intrinsic_enum`, `__flurry_ffi_link*`) | Wishlist (`tmp/wishlist/06`) | 🔴 Missing | `attributes.rs` 与 `nir`/`elaboration` 零消费者；`library/attrs/compilation/README.md:3` 明确标记为“待实现” | — |
| `GAP-04-46` | `__flurry_quote_fn_template_{for_match_arms, allow_non_exhaustive_instance}` | Core (`docs/cmtt/quote-functions-and-templates.md`; 扩展见 `WL-06`) | 🔴 Missing | `attributes.rs` 无对应键值检查，quoted arm 组装管线未建立；`library/attrs/quote_fn_template/README.md:3` 明确标注“待实现” | — |

---

## 4. 关键缺口深度剖析（含源码证据）

### 4.1 双引擎执行路由、`ai_dev_logs` 过时判断核销与 Memoization 断层（`GAP-04-01`, `02`, `04`）
1. **主路由与“零回退”契约**：
   在 `compiler/comptime_interpretor/src/bytecode.rs:843-848`，只要调用处于 `self.comptime_only && self.bytecode_enabled && self.cleanup_schedule.is_none() && self.effect_state.environment.is_empty()`，VM 即选择编译并执行字节码；且根据 `invocation/session.rs:365-400` 与 `bytecode_compilation.rs:384-390`，一旦 `comptime_compiler::compile_body` 返回 `Error::Unsupported`，错误直接向上传播，**绝不静默回退到树求值器**。
2. **核销 `ai_dev_logs/comptime_coverage_report_2026-11-21.md:54` 的过时结论**：
   该旧日志声称 `comptime_compiler` 不支持短路 `And`/`Or` 和 normal-form 调用。核查 `HEAD = b9a52238` 实时代码发现两者均已在 `compiler/comptime_compiler/src/lib.rs` 中实装：
   - **NF 调用与类型输入**：`L2225-2259` 通过 `Op::TypeInputPlan` 统一编译 `NormalFormType` / `ApplyNormalForm` / `PlannedFunctionInstance` / `NominalType` / `CheckTypeAlias` / `WithTypeInputs`。
   - **短路与位向量逻辑**：`L3179-3217` 为 `Primitive::BitVec` 生成急切 `Op::Bits`，`L3218-3242` 为 `Primitive::Bool | Primitive::Never`（及结果为 `Bool|Never` 的 `AssociatedType`）生成 `Op::Branch` + `Op::CheckBool` 短路字节码。
   - **真正仍被字节码拒绝的指令子集**：`lib.rs:3304`（非 Bool/BitVec 的 `And`/`Or`）、`lib.rs:3321`（`K::TemplateEqual`）、`lib.rs:3307-3320`（全部指针、代数效应、`K::StagedValue`、`ConstAddress`/`StaticAddress`）、`leaf_patterns.rs:19-24`（仅限叶模式）以及 `bytecode.rs:916`（`depth != 0` 的 `start_staged_input` 退回 `Ok(None)`）。
3. **Memoization 尚未接通 Elaboration 类型构造主路径**：
   `comptime_executor/src/lib.rs:1-274` 提供了带步骤配额 `Context`、防重入保护 `forming_key` 与 `KeyProtocol`（委托 Flurry 层级 `Eq`/`Hash`）的有界缓存内核；然而 `compiler/luna/src/memoization.rs:228` 与 `L350` 人为加锁：
   `let cacheable = closed && **result == Term::Primitive(nir::Primitive::Type);`
   任何返回非 `Type`（如结构体/列表/整数/`Object`）的纯 `comptime fn` 一律绕过缓存。更关键的是，全仓库非测试代码检索显示 `elaboration/src/program/required*` 并未调用 `luna::memoization` 或 `comptime_executor`，依赖大量重复纯计算的类型形成与代码生成尚无法跨调用复用计算结果。

### 4.2 Staging 闭包阻断与 `Splice` 协议泛化停滞（`GAP-04-09` ~ `11`, `14`）
1. **Staged 闭包在 `iir/src/staging.rs:3254` 的架构断层**：
   在 `elaboration` 阶段，`elaboration/src/program/constants/materialization.rs:829-945` 与 `captures.rs:1-80` 已经能够把不捕获裸存储、通过闭合性校验的 `Value::Closure` 物化为带绑定共享的 NIR `K::Closure`。然而当该常数穿过 IIR 分阶段输入边界（如泛化依赖类型的 `fn`、高阶组合子或 `staged_value`）进入 `compiler/iir/src/staging.rs:2907-3256` 的 `constant()`（及 `staging/preparation.rs:277` `delivery_constant()`、`staging/frozen.rs:126` `decode()`）时，模式匹配未处理 `K::Closure`，直接落入 `compiler/iir/src/staging.rs:3254`：
   `_ => return Err("staged input requires a supported canonical constant".into())`
   这与 `issues/0003-builtin-packages-compiler-blockers.md` 报告一致（注意 `issues/0003` 记录的行号 `3173` 在 `HEAD` 中已后移至 `L3254`，`L3173` 当前处于整数字面量取反分支）。
2. **`Splice` 与自动派生覆盖窄小**：
   按 `docs/comptime/quoting-and-splicing.md`，`Splice` 应当支持任意标量、`str`、函数以及由有限字段构成的结构体/枚举/元组的自动派生，使高阶库能用 Flurry 原生风格自然传递跨越阶段的常量：
   ```flurry
   -- 目标设计中应通过 Splice 派生与函数 Splice 自然贯通的跨阶段调用
   use builtin.{bool, u64, Splice}

   fn invoke_staged(comptime pred: fn(u64) -> bool, x: u64) -> bool {
       -- 当前依赖 closure/fn-ptr 的 Splice 与 iir/staging 闭包物化尚未打通
       pred(x)
   }
   ```
   现行 `library/builtin/protocols/staging.fl:1-167` 仅实装了原始标量（及 `FormatOptions`）；`compiler/luna/tests/cmtt_methods.rs:231` 明确 `#[ignore = "builtin has no runtime callable Splice implementation yet"]`；`derivation.rs` 尚不支持自动 `Splice` 派生。

### 4.3 CMTT 缺口：`Any` 证据、Bodyless `quote fn` 与 Quoted Match Arms（`GAP-04-17` ~ `25`）
1. **`Any`/`any` 代码证据被一刀切拒绝**：
   `docs/cmtt/any-and-code-evidence.md` 设计了通过存在量化或 `Any` 证据携带未知上下文/结果代码值并在消费前恢复接口证据的机制；但在 `compiler/elaboration/src/program/cmtt.rs:197-203`，`code_interface` 若未匹配 `H::ContextualExprType(bindings, result)` 即直接返回：
   `"code annotation requires an expression-code interface; Any evidence is not yet implemented"`
   同时 `cmtt/generators.rs:305-308` 拒绝代码集合容器（`"code collections require code-container evidence not yet implemented"`）。
2. **`quote fn` 缺口集中在新组装形态**：
   尽管普通带体 `quote fn` 与成员方法已做到实参一次求值、`contract_returns` 返回契约包裹和单洞计数（`cmtt/functions.rs:256-404`, `quote_functions.rs`），但 `cmtt/functions.rs:32-37`（无体 `quote fn` 声明）与 `L50-58`（contextual/依赖参数）以及 `library/attrs/quote_fn_template/README.md:3` + `parse/hir/elaboration` 对 `match` 内 `use expr`（quoted arms）的支持全部处于未实现状态。
3. **公共库 `meta.code` 尚未对外供给**：
   `library/meta/code/` 下仅有 `README.md`，CMTT 在编译器内已运行良好（`cmtt.rs` 1.2 万行端到端测试），但尚未通过 `meta.code` 暴露用户可扩展的组合原语与 `code_sequence`。

### 4.4 反射与构建器：Silver P0-2 `Term::Record` 盲区与 `Type.type_id` 空白（`GAP-04-29`, `31`, `34`, `36`）
1. **匿名记录 `Ps` (`Term::Record`) 反射与谓词全盲（Silver P0-2）**：
   在 Flurry 的依赖/可变属性与 Silver 泛型协议设计中，匿名记录类型 `Ps`（如 `{ x: u32, y: bool }`，在 NIR 中对应 `Term::Record(Vec<(String, Term)>)`）需要被反射检查字段名与域类型，或由谓词识别。然而核查实时代码可见三重空白：
   - `compiler/nir/src/types.rs:261-263` 在 `TypeKind::evaluate` 中直接将 `Term::Record(_)` 归入返回 `Some(false)` 的不可判定分支；
   - `compiler/elaboration/src/program/projections.rs:156-172` 中 `parse_type_predicate_kind` 只有 12 个轴，没有 `is_record`（也无 `is_union`、`is_char`）；
   - `compiler/comptime_interpretor/src/reflection.rs:101-103` 的 `reflect_struct` 要求 `let Term::Nominal { declaration, .. } = ty else { return Err(ErrorKind::Type) }`，对 `Term::Record` 直接拒绝，同时全编译器无 `record_field_names` / `record_field_types` 观察通道。
2. **`CallableReflection` 只停留在 Rust 内核，未接入语言层**：
   `compiler/comptime_interpretor/src/reflection/callables.rs:40-57` 已完成带预算检查的 `Interpreter::reflect_callable(&mut self, ty: &Term) -> Result<CallableReflection, Error>`（见第 2 轮已读源码及 `callable_reflection.rs`），但它既未列入 `builtin_bindings/signatures.rs`，也未在 `library/meta/reflection/structure.fl` 暴露任何 Flurry 方法。
3. **泛型/高级定义构建器分界**：
   如 `library/meta/definitions/README.md:96-107` 所述，`compiler/elaboration/src/definitions.rs` + `hir::definitions::assemble_struct_definition` 已支持泛型子句发布；然而公开 `StructBuilder`/`EnumBuilder`/`NewtypeBuilder`（`newtype_build.rs:112-116`）在 VM 准入层显式强制要求无泛型参数/无子句/无捕获，且 `UnionBuilder`、`TraitBuilder`、`ImplBuilder`、`FunctionBuilder` 尚无设计接入。
4. **`Type.type_id` 与 `__flurry_stable_since` 脱节**：
   `library/attrs/stability_impl/constructors.fl:8-11` 与 `elaboration/src/program/attributes.rs:129` 已让 `^attrs.stable_since("1.0.0")` 在声明与 `'attribute` 上可读，但 `docs/type/type_id.md` 规定的 128-bit 稳定性 Merkle 哈希（3 个哈希输入规范、NF 实例沿头声明传播与 `!E T` 的 `0 = Ok` 哨兵契约）在编译期与库中均未动工。

### 4.5 官方属性键审计全表（对照 `docs/comptime/official-attribute-keys.md` + `tmp/wishlist/06`）

| 属性键 | 规范域 / 目标 | `attributes.rs` 与校验器 | 编译器消费者 | `library/attrs` 导出 | 结论 |
| --- | --- | --- | --- | --- | --- |
| `__flurry_comptime_phase` | `Bool` / Trait 成员与关联项 | `attributes.rs:144`, `phases.rs:73-141` | `program.rs` 成员阶段覆盖 | `attrs.comptime_phase` (`prelude`) | 🟢 Full |
| `__flurry_packed_mode` | `Symbol` (`.byte` \| `.bit`) / `Struct`（禁配置语法） | `attributes.rs:148`, `layout.rs:39-92` | `nominal.rs:798`, `packed_bit.rs:241`, `nir/layout.rs`, LLVM | `attrs.packed.{byte, bit}` + `Type.packed_mode` | 🟢 Full |
| `"packed"` *(legacy)* | `Bool` / `Struct` | `attributes.rs:145`, `layout.rs:40,94-127` | `layout.rs` 转 `PackedMode::Byte/None` | `attrs.layout.{packed, packed_layout}` + `structure.fl:133` | ⚠️ Divergent（待清理） |
| `__flurry_volatile` | `Bool` / `Struct`, `Newtype`, Field（禁 `packed` 混用、禁配置） | `attributes.rs:149`, `volatile.rs:65-128` | `nir/program/volatile_storage.rs`, `iir/src/lib.rs:235`, LLVM volatile + P0 纯度 | `attrs.mem.volatile` / `attrs.volatile` | 🟢 Full |
| `__flurry_direct_passing` | `Bool` / 同步无参 `Effect` | `attributes.rs:146`, `effects.rs:37-67` | `program.rs:1778,3489`, `effects.rs:550` | `attrs.effects.direct_passing` | 🟢 Full |
| `__flurry_metadata` | `Object` / 任意定义 | `attributes.rs:147`, `metadata.rs:36-57` | `DefinitionAttributes::Frozen` 供 `'attribute` 观察 | `attrs.metadata` (`prelude`) | 🟢 Full |
| `__flurry_macro_{kind,variadic,grammar}` | `Symbol`/`Bool`/`Str` | `attributes.rs:127-129`（*跳过 target 校验*） | `luna/src/macro_host.rs:1449-1500` | `attrs.macros.{ast,peg,macro3,macro4}` | 🟡 Partial（缺目标合法性检查） |
| `__flurry_abi` | `Symbol` / 函数等 | `attributes.rs:108-125`（仅允许 runtime 函数 `.c`） | `nir/program/definition_attributes.rs:37`, LLVM C 入口 | `attrs.abi` (`prelude`) | 🟡 Partial（缺非 C ABI / fn 类型 / static） |
| `__flurry_derivation_default` | `Bool` / `Deriver` | `program.rs:2188`, `derivation.rs:39`（注：`attributes.rs:105` 调用校验但 `L143-151` 漏列，依赖预展开前剔除） | `luna/src/frontend.rs:81`, `program.rs:2163` | **缺失 `.default`**（`derivation/README.md:3-6`） | 🟡 Partial |
| `__flurry_stable_since` | `Str` (SemVer) / 名义等 | `attributes.rs:129`（仅检查 `P::Str`） | **0 语义消费者**（`type_id` 不存在，仅 `'attribute` 可读） | `attrs.stable_since` (`prelude`) | 🟠 Frontend-Only |
| `__flurry_quote_fn_template_*` (`for_match_arms`, `allow_non_exhaustive_instance`) | `Bool` / `quote fn` 模板 | ❌ 未实现（`L153` 拒） | 0 | ❌ `quote_fn_template/README.md` | 🔴 Missing (Core/WL) |
| `__flurry_align`, `__flurry_pipeline*`, `__flurry_inline`, `__flurry_branch_hint`, `__flurry_target_feature`, `__flurry_caller_location`, `__flurry_effect_strategy`, `__flurry_intrinsic_enum`, `__flurry_ffi_link*` | `WL-06` 扩展键 | ❌ 未实现（`attributes.rs:153-158` 统一拒） | 0 | ❌ `compilation/README.md` 标“待实现” | 🔴 Missing (Wishlist) |

---

## 5. 架构偏离与技术债清单（⚠️ Divergent 专项）

### `DIV-04-01`（⚠️ Divergent · P0）：CMTT 生成器与 4 份文档残留 Comptime 指针机制（直接违背 `AGENTS.md` §5）
- **规则冲突**：`AGENTS.md` §5 明确规定“**comptime 没有指针；comptime 方法的 `self` 使用值引用语义，不得用 `*self`、`*List<T>`、`.ref` 或 `.*` 模拟接收者及其更新。现存 comptime 指针实现、测试与文档描述属于待纠正的实现偏差，不构成语义依据**”。
- **实时源码偏离点**：
  1. `compiler/elaboration/src/program/cmtt/generators.rs:315-316` 与 `cmtt/patterns/generators.rs:393-401`：无显式注解的 `*self`（`TyParamKind::Itself { is_ref: true }`）在生成器中仍被构造成 `Term::Pointer(Box::new(ty))`。
  2. `compiler/elaboration/src/program/cmtt/generators.rs:2305` 与 `2361-2367`：`generator_data_domain` 的域递归分支仍放行 `Term::Pointer(inner)`（随后虽依赖 `generator_comptime_storage` 等间接拦截，但域白名单自身违背边界），`validate_generator_data` 仍允许 `AddressOf`/`WritePointer` 分类标记。
  3. `compiler/elaboration/src/program/cmtt/generators/methods.rs:433-448` 与 `methods/traits.rs:213-229`：当方法首参为 `Itself { is_ref: true }` 时仍用 `Expr { kind: H::Ref(base), .. }` 包装接收者；`places.rs:311` 仍受理 `H::Deref(_)`；`inference.rs:124-136` 仍推断 `H::Ref`/`H::Deref`。
  4. `compiler/elaboration/src/program/list_patterns.rs:54-62`：在 `!contract.by_value` 分支保留了 `K::AddressOf` 列表模式构建（目前靠 `elaboration/src/list_patterns.rs:200-201` 强行传递 `by_value = comptime` 掩盖）。
- **过时文档偏离点（须同步修正）**：
  - `docs/comptime/index.md:201-242`（与文首 `3-36` 值引用声明自相矛盾，保留了完整的 `*self` / `.ref` / `.*` comptime 指针旧章）；
  - `docs/cmtt/index.md:1831-1904`（旧 CMTT 指针传递示例）；
  - `docs/cmtt/patterns-and-arms.md:217-229`（旧模式指针接收者示例）；
  - `library/meta/README.md:5,216`（提及历史 comptime 指针迁移/旧示例）。

### `DIV-04-02`（⚠️ Divergent · P1）：单洞规则采用“全 quote 扁平跨度集合”而非“每执行路径/互斥分支独立计数”
- **规则冲突**：`docs/cmtt/quotation-and-stages.md` 规定当 splice / `quote fn` 调用位于 `if`/`else` 或 `match` 等**互斥 runtime 分支**中时，只要单条动态执行路径最多经过一个洞即合法；而 `compiler/elaboration/src/program/cmtt/functions.rs:81-92` 的 `record_quote_hole` 仅以整个 quote 为粒度维护扁平 `quote_holes: BTreeSet<(u32, u32)>`，在 `expression_values/special.rs:67`（`H::CodeSplice`）和 `quote fn` 调用点无条件插入并在 `len() > 1` 时报错 `"a quote permits at most one splice or quote fn application"`，从未在 `if`/`match`/`when` 分支处保存/隔离并取分支最大值。

### `DIV-04-03`（⚠️ Divergent · P1）：官方属性机制留存旧 `packed` 键及 `domain.is_some()` 隐式白名单绕过
- **规则冲突**：`docs/comptime/official-attribute-keys.md` 规定官方编译器键一律使用 `__flurry_*` 前缀，非官方键不应获得编译器特殊语义；而 `attributes.rs:145` 与 `attributes/layout.rs:40,94-127` 继续认可裸 `"packed"` 键，`library/attrs/layout_impl/constructors.fl:10-15` 与 `library/attrs/lib.fl:20-22` 仍导出旧 `attrs.layout.{packed, packed_layout}`，且 `library/meta/reflection/structure.fl:132-139` 的 `Type.packed_mode` 在缺少 `__flurry_packed_mode` 时向 `.packed: true` 回退伪造 `.byte`。
- **同源校验漏洞**：`attributes.rs:126-141` 对 `__flurry_macro_*` 与 `__flurry_stable_since` 在检查值域（`P::Symbol`/`P::Bool`/`P::Str`）后因 `domain.is_some()` 直接跳过了 `L142-151` 的已消费属性白名单检查，使 `__flurry_stable_since` 与 `__flurry_macro_*` 可以无报错地附着到非宏/不适用目标。

### `DIV-04-04`（记录核销 · 非代码债务）：`ai_dev_logs/comptime_coverage_report_2026-11-21.md` 与 `issues/0003` 行号漂移
- `ai_dev_logs/comptime_coverage_report_2026-11-21.md:54` 所称“字节码编译器未处理短路 `And`/`Or` 及 NF 调用”已在 `HEAD`（`comptime_compiler/src/lib.rs:2225-2259, 3179-3242`）失效；`issues/0003:11` 所记 `staging.rs:3173` 在 `HEAD` 已移至 `compiler/iir/src/staging.rs:3254`。后续任务计划不得再直接转引上述过时文档切片。

---

## 6. 健全性 · 完备性 · 鲁棒性专项审计（SND / CMP / ROB）

### 6.1 健全性审计（`SND-04-NN`）

| ID | 严重度 | 发现描述与源码证据 | 潜在后果 |
| --- | --- | --- | --- |
| `SND-04-01` | **High** | **`attributes.rs:126-141` `domain.is_some()` 绕过消费者校验**：`prepare_phase_attributes` 先匹配 `__flurry_macro_kind/variadic/grammar` 与 `__flurry_stable_since` 的简单值域（`L126-139`），随即在 `L140` 用 `if domain.is_none()` 跳过 `L142-159` 的白名单与消费者检查。除 `macro_host.rs` 对真正宏处理器的检验外，普通函数/struct/字段挂载这些键既无目标种类校验也无消费者诊断；`__flurry_stable_since` 甚至无任何 SemVer 格式合法性检查 | 用户可在任意不生效的目标上静默附着 `__flurry_stable_since` 或 `__flurry_macro_*` 而以为版本锚定/宏生效，破坏“静默忽略即 bug”原则 |
| `SND-04-02` | **High** | **CMTT 生成器与模式残留指针构建路径**：`cmtt/generators.rs:315-316,2305`、`patterns/generators.rs:393-401`、`methods.rs:433-448`、`traits.rs:213-229`、`places.rs:311` 与 `list_patterns.rs:54-62` 仍在 comptime 上下文中生成或匹配 `Term::Pointer`、`H::Ref`、`H::Deref` 与 `K::AddressOf` | 若任一上下文绕过 `nir/phases.rs:739-755` 或由未来重构触及 `by_value=false`，将导致 comptime 指针语义泄漏或阶段契约冲突 |
| `SND-04-03` | **Medium** | **`quote_holes` 全局 `(lo, hi)` Span 集合缺陷**：`cmtt/functions.rs:81-92` 以源代码字节跨度去重并全局计数 `quote_holes.len() > 1`，既未隔离互斥分支（违背 `docs/cmtt/quotation-and-stages.md`），又隐含基于 Span 身份去重风险（宏/合成节点共享零 Span 时会错误合并为 1 个洞） | 合法互斥分支单洞被错误拒绝；若未来由代码合成多个共用默认 Span 的 splice，可能逃逸单洞检查造成重复求值 |
| `SND-04-04` | **Medium** | **`memoization.rs` 未接入与静态 `Scope { 0, 0, 0 }`**：`luna/src/memoization.rs:286-293` 仅硬编码固定空世界 `Scope { definitions: 0, protocols: 0, target: 0 }` 且限制仅 `Type` 返回可缓存（`L228,350`）；`elaboration/src/program/required*` 完全旁路该缓存 | 若后续扩展缓存到非封闭依赖世界或 builder 衍生世界而沿用零版本号，将导致跨世界脏缓存复用 |
| `SND-04-05` | **Low** | **过渡期 `packed` 与 `__flurry_packed_mode` 并行反射语义偏移**：虽然 `attributes/layout.rs:41-46` 拒绝在同一目标混用两者，但 `structure.fl:132-139` 对遗留 `{ packed: true }` 反射返回 `.byte`，且未对 `{ packed: false }` 覆盖 `__flurry_packed_mode` 或用户直写 `.packed` 属性键设立统一弃用告警 | 同一结构体可能同时受旧库包 `{ packed: bool }` 与新库包 `{ __flurry_packed_mode: Symbol }` 差异影响 |

### 6.2 完备性审计（`CMP-04-NN`）

| ID | 严重度 | 发现描述与源码证据 | 遗漏行为范围 |
| --- | --- | --- | --- |
| `CMP-04-01` | **High** | **Staged 常量与冻结器遗漏闭包及通用折叠**：`compiler/iir/src/staging.rs:3254`、`staging/preparation.rs:277` 与 `staging/frozen.rs:126` 分别以 `_ => Err("staged input requires a supported canonical constant")` 和 `_ => Err("staged frozen value requires supported immutable data")` 截断未列举的 NIR 节点，彻底阻塞 `K::Closure` 及含函数值/闭包的聚合 staged 常量 | 所有在 `runtime` 依赖类型、泛化调用阶段输入或冻结常亮中传递闭包的内置库/用户模式 |
| `CMP-04-02` | **High** | **字节码编译器与叶模式覆盖缺口**：`comptime_compiler/src/lib.rs:3304-3321` 拒绝 `TemplateEqual`、`StagedValue`、非 bool/BitVec `And`/`Or`、全部 `Effect*` 与指针节点；`leaf_patterns.rs:19-24` 仅处理标量/bool/symbol/tuple 等叶模式；`bytecode.rs:916` 拒绝 `depth != 0` 的 `start_staged_input`。由于无回退机制（`bytecode_compilation.rs:384-390`），纯 comptime 路径命中任一缺口即编译中止 | 含上述节点或深层嵌套 staged 输入的纯 comptime 调用 |
| `CMP-04-03` | **High** | **CMTT `Any`、Bodyless/Contextual `quote fn`、代码集合与 Quoted Arms 缺失**：`cmtt.rs:198-202,410-413`（`code_interface`/`code_value_inner`）、`cmtt/functions.rs:35,57`、`cmtt/generators.rs:305-308`、`block_regions/matches.rs`（缺 `ArmImport`）直接对设计中的高阶 CMTT 组合分支报错 | `Any` 反射式代码拼接、多臂协同 `quote fn` 模板、`meta.code` 用户生态与宏生成 `match` 臂 |
| `CMP-04-04` | **High** | **匿名记录 `Term::Record` / Union / Char / ABI / Wishlist 属性全线未匹配**：`nir/src/types.rs:164-175,261-271`、`projections.rs:156-171`、`reflection.rs:101-103` 与 `attributes.rs:108-159` 导致匿名记录 `Ps`、Union、Char 谓词、非 C ABI 及 12 个 `WL-06` 属性不可用；`CallableReflection`（`reflection/callables.rs:40`）虽在 Rust 就绪却未在 `builtin_bindings/signatures.rs` 接线 | Silver 泛型记录算法（P0-2）、`meta` 函数/Trait 反射、底层对齐/内联/pipeline 属性控制 |
| `CMP-04-05` | **Medium** | **`Splice` 非标量实现与 `Type.vars` 单点硬编码**：`library/builtin/protocols/staging.fl` 与 `derivation.rs` 缺 `str`/Tuple/Record/Callable `Splice` 实现及自动派生；`type_data.rs:155-163` 仅硬编码 `Type.vars.Closure` | 无法将闭合函数或复合值直接 `Splice` 注入 staged 代码；`Type.vars.Reachability` 等参数族缺失 |

### 6.3 鲁棒性审计（`ROB-04-NN`）

#### 非测试源码 Panic 边界与反模式定量普查（基线：`HEAD = b9a52238`）
> **精确普查口径**：脚本排除 `/tests/`、`tests.rs`、`*_tests.rs`、`*_test.rs`、外部 `#[cfg(test)] #[path = "..."] mod ...;`（自动发现 52 个模块，累计跳过 64 个纯测试文件）以及源码内所有 `#[cfg(test)]` 块和独立 `#[test]` 函数，仅统计进入编译期/生产执行链路的非测试代码。

| 模块范围 | 非测试文件 | 非测试代码行 | `.unwrap()` | `.expect()` | `panic!()` | `unreachable!()` | `todo!/unimpl!` | 零/默认 Span | `HashMap/Set` | `_ =>` 兜底臂 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `compiler/comptime_interpretor/src` | 131 | 41,037 | 57 | 60 | 0 | 121 | 0 | 15 | 94 | 285 |
| `compiler/comptime_compiler/src` | 22 | 5,653 | 10 | 5 | 0 | 14 | 0 | 0 | 3 | 27 |
| `compiler/comptime_executor/src` | 1 | 274 | 0 | 0 | 0 | 0 | 0 | 0 | 3 | 0 |
| `compiler/comptime_runtime/src` | 12 | 2,178 | 4 | 1 | 0 | 0 | 0 | 0 | 0 | 10 |
| `compiler/iir/src/staging*` | 7 | 4,484 | 13 | 0 | 0 | 27 | 0 | 0 | 0 | 56 |
| `compiler/elaboration/src/program/cmtt*` | 25 | 17,054 | 65 | 3 | 0 | 30 | 0 | 2 | 1 | 129 |
| `compiler/elaboration/src/program/attributes*` | 10 | 1,504 | 0 | 5 | 0 | 1 | 0 | 7 | 0 | 8 |
| `compiler/elaboration/src/program/{projections,type_data,splicing,derivation,required*,constants*}` | 32 | 12,201 | 40 | 9 | 0 | 19 | 0 | 4 | 17 | 74 |
| **合计** | **240** | **84,385** | **189** | **83** | **0** | **212** | **0** | **28** | **118** | **589** |

> 阳性基准确认：全量 240 个非测试文件中 `panic!()`、`todo!()` 与 `unimplemented!()` 均为 **`0`**；`comptime_executor/src/lib.rs` 实现为 `0 unwrap / 0 expect / 0 unreachable / 0 _ =>`，所有内存/大整数/图遍历核心边界（`staging.rs` `depth >= 128`、`cmtt.rs:264` `code_construction_depth >= 32`、`Budget::steps`）均配有显式配额与深度限制。

| ID | 严重度 | 鲁棒性缺陷描述与高危热点 `file:line` | 后果与触发场景 |
| --- | --- | --- | --- |
| `ROB-04-01` | **Medium** | **484 处内部不变式 Panic 表面（`.unwrap()` 189 + `.expect()` 83 + `unreachable!()` 212）**：高频集中在：(1) `cmtt/*`：`cmtt/generators.rs:533,556`、`cmtt/generators/clauses.rs:159,225`、`cmtt/generators/spreads.rs:406`、`cmtt/generators/places.rs:664,731` 对查表/`pop()` 直接 `.unwrap()`；(2) `comptime_interpretor`：`bytecode.rs:731-755,1649`（缓存淘汰与字节码导出）、`bit_operations.rs:149,212,231-232`（固定位置实参消费）、`cleanup.rs:400,409,1391`；(3) `iir/src/staging*`：`staging.rs:1148,1230-1236,2842-2859` 与 `staging/preparation.rs:353`；(4) `comptime_compiler`：`normal_types.rs:162,203-204,226,256,493` 与 `lib.rs:1384,2494,3010` | 畸形 NIR、未闭合包输入或阶段交错边界失配时直接进程 crash（Rust panic），无法转化为可恢复的编译期 `ErrorKind` 诊断 |
| `ROB-04-02` | **Medium** | **28 处零位置 / `DUMMY_SP` / `Span::default()` 诊断退化**：主要出现在 `attributes.rs:182`（`error(Span::default(), ...)`）配对各检验子模块的 `.unwrap_or(SourceRange { start: 0, end: 0 })`（`derivation.rs:34`, `effects.rs:32`, `layout.rs:34`, `metadata.rs:31`, `phases.rs:69`, `volatile.rs:60`），以及 `cmtt/functions.rs:1218,1230`（`DUMMY_SP`）、`derivation.rs:213,244,532`（`DUMMY_SP`）与 `comptime_interpretor`（`admission.rs:235,248`, `value_reference.rs:314,329`, `memory.rs:694,745,761`） | 当异常发生在无明确字段入口的属性合并失败、派生产物预算耗尽或引用读取越界时，用户将丢失精确源码行/列定位 |
| `ROB-04-03` | **Medium** | **118 处 `HashMap` / `HashSet` 非确定性遍历风险**：`comptime_interpretor` 占 94 处（尤其 `cleanup.rs:69,111,1610-1612`、`comptime_graph.rs:66-85`、`constructor_premises.rs:30-39`、`declaration_index.rs:184-196`、`function_instances.rs:8-37`），`elaboration/src/program` 占 17 处（含 `constants/materialization.rs:16-17` `closures: HashMap` / `active: HashSet` 与 `captures.rs:15-16`）。虽然关键有序路径（`attributes/merge.rs`, `cmtt.rs`, `comptime_executor` 除外）广泛使用 `BTreeMap`/`BTreeSet`，上述 `HashMap` 若被 `.iter()` 导出到错误消息或产物绑定顺序中，会引发构建与诊断顺序漂移 | 跨次编译出现非确定性诊断顺序或析构顺序波动（尤其与后续增量构建/确定性 `type_id` 冲突） |
| `ROB-04-04` | **Low** | **跨模块递归深度上限不统一且部分递归缺少集中配置**：`cmtt.rs:264` 限制 `code_construction_depth >= 32`、`cmtt.rs:280`/`functions.rs:360` 为 `64`、`staging.rs`/`memory.rs` 为 `128`，虽然全部有界防爆栈，但在深层嵌套模板或高阶展开中可能提前在 32 层触发不可微调截断 | 深层合法宏/代码模板组合比其他前端阶段（128 层）更早失败 |

---

## 7. 修复路径、依赖与验收建议

### 阶段 0：立即消债与语义纠偏（消除 `⚠️ Divergent` 与 `SND` 高危项）
1. **彻底清退 CMTT 残留 Comptime 指针（解决 `DIV-04-01` / `SND-04-02`）**：
   - 将 `cmtt/generators.rs:315-316` 与 `cmtt/patterns/generators.rs:393-401` 的 `Itself { is_ref: true }` 统一替换为拒绝 `*self` 的阶段诊断，清理 `methods.rs:433-448` / `traits.rs:213-229` 的 `H::Ref(base)`、`places.rs:311` 的 `H::Deref`、`inference.rs:124-136` 及 `generators.rs:2305,2361-2367` 的 `Term::Pointer` 放行，同步下线 `list_patterns.rs:54-62` 遗留 `K::AddressOf` 分支。
   - 同步清理 `docs/comptime/index.md:201-242`、`docs/cmtt/index.md:1831-1904`、`docs/cmtt/patterns-and-arms.md:217-229` 与 `library/meta/README.md:5,216` 的旧指针文本。
   - **验收**：扩展 `compiler/luna/tests/comptime_pointer_phase.rs` 与 `cmtt.rs`，验证生成器/方法 `*self` 与 `.ref`/`.*` 在检查阶段即被精准拒绝，同时既有 `cmtt.rs`/`builder_invocations.rs` 依赖值引用与可变接收者的用例全量通过。
2. **修补属性白名单绕过与退场旧 `packed` 键（解决 `DIV-04-03` / `SND-04-01`）**：
   - 收敛 `attributes.rs:126-141`，使 `__flurry_stable_since` 与 `__flurry_macro_*` 进入目标种类校验（SemVer 格式与可锚定定义，或宏函数定义），不再凭 `domain.is_some()` 绕过检查；
   - 在 `attributes.rs:145`、`attributes/layout.rs:40,94-127`、`library/attrs/layout_impl/constructors.fl:10-15` 与 `library/meta/reflection/structure.fl:132-139` 移除/弃用裸 `"packed"` 键，统一迁至 `__flurry_packed_mode`。
3. **修复 `quote_holes` 互斥分支隔离与 Span 退化（解决 `DIV-04-02` / `SND-04-03` / `ROB-04-02`）**：
   - 在 `cmtt/functions.rs:81-92` 将单洞计数改为在 `if`/`else` 与 `match` 互斥控制流臂间快照与取最大路径计数，且基于 HIR 节点身份 `HirId` 而非 `(lo, hi)` Span 字节跨度去重。
   - 清理 `attributes.rs:182` 的 `Span::default()` 与 `cmtt/functions.rs:1218,1230` / `derivation.rs` 的 `DUMMY_SP`。

### 阶段 1：Core 编译器高优先级阻塞修复（打通标准库与 `issues/0003` / Silver P0-2）
1. **打通 Staged 闭包与 `Splice` 函数/复合值传输（`GAP-04-09`, `10`, `11`）**：
   - 在 `compiler/iir/src/staging.rs:3254`、`staging/preparation.rs:277` 与 `staging/frozen.rs:126` 接入 `K::Closure`（复用/对齐 `constants/materialization.rs:829` 的闭合捕获与阶段校验规则），支持纯闭合闭包与函数引用作为 staged 输入与冻结常量；
   - 在 `library/builtin/protocols/staging.fl` 与编译器 `derivation.rs` 补全 `str`、元组、记录、函数类型的 `Splice` 实现与有限结构自动 `Splice` 派生，**解锁并取消忽略 `cmtt_methods.rs:231`**。
2. **补齐 Silver P0-2 匿名记录 `Term::Record` 反射与 `CallableReflection` 导出（`GAP-04-29`, `30`, `31`）**：
   - 在 `nir/src/types.rs:164-175,261-263` 与 `projections.rs:156-172` 增加 `TypeKind::Record`（`Type.predicates.is_record`）、`is_union`、`is_char`；
   - 在 `reflection.rs` / `observations.rs` / `builtin_bindings` 扩展 `record_field_names` / `record_field_types`（并复用 `meta.reflection.structure` 的 `Object` 视图 `record_fields()`）同时将已实现的 `reflect_callable` 暴露给 `meta`，不硬编码匿名记录特判；
   - **验收**：新增真实 sysroot 集成测试，在泛型 `where Ps` 记录与 `Type.predicates.is_record(Ps)` 守卫上完成反射与构造。
3. **字节码覆盖与 Memoization 完整接线（`GAP-04-01`, `02`, `04`）**：
   - 补齐 `comptime_compiler` 对 `TemplateEqual`、通用叶/聚合模式与 `depth > 0` staged 输入的编译支持，缩小与树求值器的行为差集；
   - 放开 `luna/src/memoization.rs:228,350` 的非 `Type` 结果缓存限制并引入完整世界版本 `Scope`，在 `elaboration/src/program/required*` 中按纯函数查询接线有界记忆化。

### 阶段 2：CMTT 高阶原语、`Type.type_id` 与剩余属性体系（`GAP-04-14, 18..24, 34, 36, 40..46`）
1. **CMTT 完整闭环**：依次交付 (a) `Any`/`any` 代码证据恢复（`GAP-04-18`）与 `code_sequence`/`meta.code` 公开库（`GAP-04-22`, `23`）；(b) bodyless `quote fn`、contextual `quote fn` 输入（`GAP-04-19`, `20`）与 `use expr` quoted match arms + `__flurry_quote_fn_template_*` 属性消费（`GAP-04-21`, `46`）；(c) `issues/0002` 遗留闭合函数 `const` 失败流与 `match` 发散臂类型合并（`GAP-04-25`）。
2. **`Type.type_id` 与 `__flurry_stable_since` 合一（`GAP-04-36`, `42`）**：
   - 按 `docs/type/type_id.md` 实现 128-bit 稳定性 Merkle Hash，令 `attrs.stable_since(version)` 的 SemVer 版本锚点直接作为声明规范哈希的权威版本输入并沿 NF 实例头传播，彻底替代散落的版本字符串空读取。
3. **泛型/扩展定义构建器与 `WL-06` 属性（`GAP-04-34`, `40`, `41`, `44`, `45`）**：
   - 结合 `default` 关键字语法决定导出 `attrs.derivation.default`（`GAP-04-41`）；扩展 `__flurry_abi` 至全平台调用约定与函数指针类型（`GAP-04-40`）；按统一属性消费协议与 `BTreeMap` 确定性遍历原则逐步接入 `__flurry_align`、`__flurry_inline`、`__flurry_pipeline` 与 `UnionBuilder`/`TraitBuilder`/`ImplBuilder`/`FunctionBuilder`。
