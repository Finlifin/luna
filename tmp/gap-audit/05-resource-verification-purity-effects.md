# 05 · 资源管理、程序验证、纯度分级与效应/错误系统差距审计

## 1. 审计范围、检查基线与设计标尺

### 1.1 检查基线与源码盘点

- **仓库基线**：`HEAD` = `b9a52238`（只读静态源码审计，未执行 `cargo build` / `cargo test`）。
- **核心编译器范围**（共计 **105** 个非测试生产 `.rs` 源文件 / **94,755** 行生产代码，另含 **10** 个内联 `_tests.rs`/`tests.rs` 拆分文件、**300** 个内联 `#[test]` 与 **110** 个外部集成测试 `compiler/{program_verification,verification_core}/tests/*.rs`）：
  - [program_verification/src](file:///home/fin/luna/compiler/program_verification/src)：`54` prod 文件 / `75,259` LOC，`7` 个内联 `_tests.rs` 文件，`199` 个内联 `#[test]`，外加 [program_verification/tests](file:///home/fin/luna/compiler/program_verification/tests) 下 `5` 文件 `10` 个 `#[test]`。涵盖 [moves.rs](file:///home/fin/luna/compiler/program_verification/src/moves.rs)、[moves/](file:///home/fin/luna/compiler/program_verification/src/moves)、[drop.rs](file:///home/fin/luna/compiler/program_verification/src/drop.rs)、[drop/](file:///home/fin/luna/compiler/program_verification/src/drop)、[copy.rs](file:///home/fin/luna/compiler/program_verification/src/copy.rs)、[memory.rs](file:///home/fin/luna/compiler/program_verification/src/memory.rs)、[memory/](file:///home/fin/luna/compiler/program_verification/src/memory)、[flow.rs](file:///home/fin/luna/compiler/program_verification/src/flow.rs)、[flow/](file:///home/fin/luna/compiler/program_verification/src/flow)、[purity.rs](file:///home/fin/luna/compiler/program_verification/src/purity.rs)、[purity/](file:///home/fin/luna/compiler/program_verification/src/purity)、[behavior.rs](file:///home/fin/luna/compiler/program_verification/src/behavior.rs)、[contracts.rs](file:///home/fin/luna/compiler/program_verification/src/contracts.rs)、[effect_usage.rs](file:///home/fin/luna/compiler/program_verification/src/effect_usage.rs)、[effect_transfer.rs](file:///home/fin/luna/compiler/program_verification/src/effect_transfer.rs)、[rejection.rs](file:///home/fin/luna/compiler/program_verification/src/rejection.rs)、[optimization.rs](file:///home/fin/luna/compiler/program_verification/src/optimization.rs)、[observation_region.rs](file:///home/fin/luna/compiler/program_verification/src/observation_region.rs)、[pattern_facts.rs](file:///home/fin/luna/compiler/program_verification/src/pattern_facts.rs)、[declarations.rs](file:///home/fin/luna/compiler/program_verification/src/declarations.rs) 与 [control.rs](file:///home/fin/luna/compiler/program_verification/src/control.rs)。
  - [verification_core/src](file:///home/fin/luna/compiler/verification_core/src)：`13` prod 文件 / `6,758` LOC，`42` 个内联 `#[test]`，外加 `2` 个外部测试文件 `100` 个 `#[test]`。涵盖 [lib.rs](file:///home/fin/luna/compiler/verification_core/src/lib.rs)、[ownership.rs](file:///home/fin/luna/compiler/verification_core/src/ownership.rs)、[relations.rs](file:///home/fin/luna/compiler/verification_core/src/relations.rs)、[retained_conditions.rs](file:///home/fin/luna/compiler/verification_core/src/retained_conditions.rs)、[differences.rs](file:///home/fin/luna/compiler/verification_core/src/differences.rs)、[predicates.rs](file:///home/fin/luna/compiler/verification_core/src/predicates.rs)、[cubic.rs](file:///home/fin/luna/compiler/verification_core/src/cubic.rs)、[quadratic.rs](file:///home/fin/luna/compiler/verification_core/src/quadratic.rs)、[factors.rs](file:///home/fin/luna/compiler/verification_core/src/factors.rs)、[signs.rs](file:///home/fin/luna/compiler/verification_core/src/signs.rs)、[congruence.rs](file:///home/fin/luna/compiler/verification_core/src/congruence.rs)、[division.rs](file:///home/fin/luna/compiler/verification_core/src/division.rs)。
  - `elaboration` 效应/错误子系统：`13` prod 文件 / `3,977` LOC，包括 [effects.rs](file:///home/fin/luna/compiler/elaboration/src/effects.rs)、[effects/](file:///home/fin/luna/compiler/elaboration/src/effects)、[program/effects.rs](file:///home/fin/luna/compiler/elaboration/src/program/effects.rs)、[program/effects/](file:///home/fin/luna/compiler/elaboration/src/program/effects)、[program/error_handlers.rs](file:///home/fin/luna/compiler/elaboration/src/program/error_handlers.rs)、[program/error_handlers/](file:///home/fin/luna/compiler/elaboration/src/program/error_handlers)、[program/pattern_checking/errors.rs](file:///home/fin/luna/compiler/elaboration/src/program/pattern_checking/errors.rs)。
  - `nir` 相关模块：`25` prod 文件 / `8,761` LOC，`3` 个内联 `_tests.rs` 文件，`59` 个内联 `#[test]`，包括 [contracts.rs](file:///home/fin/luna/compiler/nir/src/contracts.rs)、[contracts/](file:///home/fin/luna/compiler/nir/src/contracts)、[error_results.rs](file:///home/fin/luna/compiler/nir/src/error_results.rs)、[error_results/](file:///home/fin/luna/compiler/nir/src/error_results)、[copy_budget.rs](file:///home/fin/luna/compiler/nir/src/copy_budget.rs)、[copy_budget/](file:///home/fin/luna/compiler/nir/src/copy_budget)、[phases.rs](file:///home/fin/luna/compiler/nir/src/phases.rs)、[phases/](file:///home/fin/luna/compiler/nir/src/phases)、[effects.rs](file:///home/fin/luna/compiler/nir/src/effects.rs)、[call_safety.rs](file:///home/fin/luna/compiler/nir/src/call_safety.rs)、[display.rs](file:///home/fin/luna/compiler/nir/src/display.rs)。
- **交叉接线核对**：同时核查了 [luna/src/{cleanup.rs,protocols.rs,text_regions.rs,packages.rs,package_cli.rs,host/llvm/attributes.rs}](file:///home/fin/luna/compiler/luna/src/cleanup.rs)、[parse](file:///home/fin/luna/compiler/parse/src)、[ast_lowering](file:///home/fin/luna/compiler/ast_lowering/src)、[hir](file:///home/fin/luna/compiler/hir/src)、[iir/src/behavior.rs](file:///home/fin/luna/compiler/iir/src/behavior.rs) 以及 [library/{builtin,attrs,verification}](file:///home/fin/luna/library/STRUCTURE.md) 相关实现。

### 1.2 权威设计标尺与层次划分

1. **核心规范（Core Norm Target）**：`AGENTS.md` §5（`Once`/`Copy`/`Clone`、无 `Option`/`Result` 枚举、`PascalCaseErr` 错误规范、运行时依赖类型 allocator 范式、错误 ABI 与效应 ABI 隔离、PTC 与程序验证隔离、S-expression `Display`）、[docs/resource-management/*](file:///home/fin/luna/docs/resource-management/index.md)、[docs/program-verification/safety.md](file:///home/fin/luna/docs/program-verification/safety.md)、[docs/algebraic-effect-and-delimited-continuation/*](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/index.md)、[docs/type/{rows,handlers,self-reference,qualified-types/{effect-type,error-type}}.md](file:///home/fin/luna/docs/type/index.md) 与已追踪阻塞记录 [issues/0003-builtin-packages-compiler-blockers.md](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md)。
2. **空白占位规范文件警示**：`HEAD` 下 [docs/type/affine-type.md](file:///home/fin/luna/docs/type/affine-type.md)、[docs/type/reachability-type.md](file:///home/fin/luna/docs/type/reachability-type.md)、[docs/type/qualified-types/reachability-type-also.md](file:///home/fin/luna/docs/type/qualified-types/reachability-type-also.md) 与 [docs/concurrency/software-transactional-memory.md](file:///home/fin/luna/docs/concurrency/software-transactional-memory.md) 均为 **0 字节空文件**，且 [docs/program-verification/specification-and-verification/](file:///home/fin/luna/docs/program-verification) 目录为空。
3. **愿望单扩展（Wishlist Extension Target）**：[tmp/silver-design/04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md)、[tmp/wishlist/08-effects-async-concurrency-net-runtime.md](file:///home/fin/luna/tmp/wishlist/08-effects-async-concurrency-net-runtime.md) 与 [tmp/wishlist/09-verification-synthesis-and-tensors.md](file:///home/fin/luna/tmp/wishlist/09-verification-synthesis-and-tensors.md)。
4. **陈旧记录勘误（Live Code vs `ai_dev_logs/` / `issues/`）**：
   - `issues/0003` 表中条目 12（“runtime env `Drop` 完全未实现”）与 [drop/plan.rs:430-431](file:///home/fin/luna/compiler/program_verification/src/drop/plan.rs#L430-L431) 注释**已过时**：`HEAD` 现已通过 `drop_environment` 与 `DropEnvContext` 支持**可提升指针（`Term::Pointer`）**的 `Drop` 运行时环境传入（`drop.rs:484-550`、`drop/access.rs:275-349`），但仍存在架构偏离与间接调用限制（见 `GAP-05-04` 与 `SND-05-01`）。
   - [ai_dev_logs/purity_system_status_report_2026-11-21.md](file:///home/fin/luna/ai_dev_logs/purity_system_status_report_2026-11-21.md) 中“未向 IIR/LLVM 输出 P0–P5 行为摘要”与“待 `static` 实现”两项**已过时**：`HEAD` 已具备 [behavior.rs](file:///home/fin/luna/compiler/program_verification/src/behavior.rs) Tarjan SCC 认证至 LLVM 23 属性的全链路，且 `K::StaticAddress` 已接入 NIR/验证层但被 `purity.rs:2058` 全盘拒绝（见 `GAP-05-16` 与 `GAP-05-20`）。

---

## 2. 成熟度总览与统计表

### 2.1 六级特性成熟度统计（共 35 项，与第 3 节严格对齐）

| 支柱域 | `🟢 Full` | `🟡 Partial` | `🟠 Frontend-Only` | `🔴 Missing` | `⚠️ Divergent` | `⚪ Deferred` | 小计 |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1. 资源管理（Once/Copy/Drop/Moves） | 2 | 2 | 0 | 1 | 1 | 0 | **6** |
| 2. 内存效应、指针、来源与可达性 | 1 | 0 | 1 | 1 | 1 | 0 | **4** |
| 3. 安全层级、`verification_core` 与契约 | 2 | 2 | 1 | 1 | 0 | 1 | **7** |
| 4. 运行时纯度分级（P0–P5） | 2 | 1 | 0 | 1 | 2 | 0 | **6** |
| 5. 错误行、代数效应、STM 与工程卫生 | 2 | 5 | 1 | 4 | 0 | 0 | **12** |
| **合计（核心规范 / 愿望单扩展）** | **9** | **10** | **3** | **8** | **4** | **1** | **35** |

> 说明：35 项中 `GAP-05-01`–`34` 主属 Core 规范（其中 `GAP-05-31`/`32`/`33` 兼具 Wishlist 扩展诉求），`GAP-05-17` 为 `⚪ Deferred` 规范项，`GAP-05-35` 为纯 Wishlist `09` 扩展项；`9+10+3+8+4+1 = 35`。

### 2.2 `SND / CMP / ROB` 专项严重度矩阵（共 19 项，与第 6 节严格对齐）

| 审计维度 | Critical | High | Medium | Low | 合计 |
|---|---:|---:|---:|---:|---:|
| 健全性 `SND`（Soundness） | 1 | 2 | 2 | 0 | **5** |
| 完备性 `CMP`（Completeness） | 1 | 4 | 3 | 1 | **9** |
| 鲁棒性 `ROB`（Robustness） | 0 | 1 | 3 | 1 | **5** |
| **合计** | **2** | **7** | **8** | **2** | **19** |

---

## 3. 全量特性差距矩阵（GAP-05-NN）

### 支柱 1：资源管理（`Once`/`Copy`/`Clone`、Moves、部分移动/初始化与 `Drop`）

| 编号 | 特性 / 机制 | 标尺属性 | 成熟度 | 现状摘要与核心源码位置 |
|---|---|---|---|---|
| `GAP-05-01` | `Once` 泛化实现与 `Clone`/`Copy` 分离 | Core (`AGENTS.md` §5, `once-clone-copy.md`) | `🟢 Full` | `library/builtin/protocols/resources.fl:6-184` 与 `native_resources.fl:1-48` 定义单一 blanket `impl Once for T where T {}` 及标量/`*T`/`str`/`?T`/`!E T` 实现；`copy.rs:39-273` 与 `luna/src/protocols.rs:28-64` 独立验证所有变体与析构平凡性。无 `Option`/`Result` 枚举残留。 |
| `GAP-05-02` | 聚合体部分移动、字段重初始化与 LIFO 多出口清理 | Core (`moves-and-initialization.md`) | `🟢 Full` | `verification_core/src/ownership.rs` `CleanupTree` 与 `program_verification/src/moves.rs` 在分支合并、循环 `break`/`continue`、`return`、赋值覆盖与模式失败路径生成 LIFO `CleanupAction`（`ScopeExit`/`Return`/`LoopBreak`/`LoopContinue`/`AssignmentOverwrite`/`GuardFailure`）。 |
| `GAP-05-03` | `Pattern::Then` 计算结果失败路径清理 | Core (`temporaries.md`, `moves.rs`) | `🟡 Partial` | `moves.rs:10167-10324` 已为无指针、满足 `copy_readable_representation` 且析构无拒绝的持有型 `Pattern::Then` 值生成 `CleanupReason::PatternFailure`（`iir/src/cleanup.rs:171`、`host/llvm/cleanup.rs:636`）；但 `moves.rs:10192-10205` 仍拒绝内部返回包、含指针值与可拒绝 `Drop`（`"computed pattern value needs independent resource and failure-path transfer"`）。 |
| `GAP-05-04` | 运行时依赖环境 `Drop`（自定义 allocator） | Core (`drop-elaboration.md`, `issues/0003`) | `⚠️ Divergent` | `drop.rs:484-550` 与 `drop/access.rs:275-349` 支持从调用方 `env_parameters`/`runtime_deliveries`/`global_runtime_deliveries` 解析 lifted `Term::Pointer` 环境参入 `Drop`；**偏离**：在**清理点**而非对象构造/转移点解析环境（`drop-elaboration.md:46`），拒绝非指针 env 值（`drop/access.rs:341-345`），拒绝 `Resolution::Assumed`（`drop/plan.rs:456-484`）及带前置/后置契约的 `Drop`（`drop/plan.rs:779-786`），且 `drop/plan.rs:430-431` 残留误导性过期注释。 |
| `GAP-05-05` | Pin-free 自引用类型（`self-reference` 协议与重定位） | Core (`docs/type/self-reference.md`) | `🔴 Missing` | 编译器各阶段及 `library/` 均无自引用字段/指针重定位追踪；`parse/src/items.rs:166` 仅指 `&self` 接收者修饰，非自引用结构类型。 |
| `GAP-05-06` | 临时值、默认参数、闭包与间接调用资源传递 | Core (`bindings.md`, `temporaries.md`) | `🟡 Partial` | 支持具名直接调用与无 env 默认值清理，但 `moves/operations.rs:696-766` 与 `moves.rs:9201-9206` 显式拒绝在临时调用、局部可选候选、间接具名调用、闭包及 guard 调用上携带 `env_arguments`（阻塞高阶函数+allocator 组合）。 |

### 支柱 2：内存效应、指针、来源（Provenance）与可达性

| 编号 | 特性 / 机制 | 标尺属性 | 成熟度 | 现状摘要与核心源码位置 |
|---|---|---|---|---|
| `GAP-05-07` | 字段指针、顺序别名分析与初始化保持写回 | Core (`pointers.md`, `memory-effects.md`) | `🟢 Full` | `memory.rs`、`moves/cleanup_memory.rs` 与 `flow/memory_contracts.rs` 对 `*T`、`PointerField` 投影、顺序别名、借用读写足迹与写回初始化状态提供完整静态证明。 |
| `GAP-05-08` | `str` 视图 `TextProvenance` 与方法/intrinsic 集成 | Core (`pointers.md`, `issues/0003:10`) | `⚠️ Divergent` | `luna/src/cleanup.rs:693-706` 仅在 `graph_has_text`（且不被 `:543` 清零重算破坏）时启用 `TextProvenance::VerifiedByRegionAnalysis`，`purity.rs:865-871` 仅在 `!self.finite`（P4）启用；但 `luna/src/protocols.rs:300-323, 500-566`（`analyze_method_cleanup` -> `moves.rs:15052`）、`memory/borrowed_code.rs:99`、`flow.rs:2180` 仍硬编码 `TextProvenance::Unchecked` 导致 `native-text-method-provenance.fl` 触发 `"string-view memory provenance is not implemented"`；`memory/text_regions/transfers.rs:1184-1190` 禁止 intrinsic 返回 `str`。 |
| `GAP-05-09` | `&r T` 可达性修饰类型（`ReachabilityType`） | Core (`reachability.md`, `docs/type/reachability-type.md`) | `🟠 Frontend-Only` | `parse/src/expressions.rs:1171`、`fmt/src/exprs.rs:532`、`ast_lowering/src/expr.rs:137` 与 `hir/src/expr.rs:209` 解析并保留节点，但在 `elaboration/src/program/lexical_inputs/dependencies.rs:1107` 被跳过，NIR/IIR/验证器全空；`docs/type/reachability-type.md` 自身为空。 |
| `GAP-05-10` | `MemoryView` 分离借用与视图切片协议 | Core (`memory-views.md`) | `🔴 Missing` | `compiler/` 与 `library/` 均无 `MemoryView` 或视图分裂（split/reborrow）类型与验证通路。 |

### 支柱 3：安全层级、`verification_core` 与程序契约

| 编号 | 特性 / 机制 | 标尺属性 | 成熟度 | 现状摘要与核心源码位置 |
|---|---|---|---|---|
| `GAP-05-11` | 四级安全层级（`safe` / `spec` / `verified` / `unsafe`） | Core (`safety.md`) | `🟡 Partial` | 仅 `unsafe`（`modifiers.is_unsafe`、`UnsafeScope` 与 `nir/src/call_safety.rs:64-115`）打通全链路；无 `safe` 关键字（默认安全语义仅靠未标 `unsafe` 禁止 `Intrinsic`/`is_unsafe_callable`）；`spec`/`verified` 解析于 `parse`（`TokenKind::Spec`/`Verified`），但在 `ast_lowering/src/item.rs:236-245` **函数声明处被静默丢弃**，在 `fn_type` 处降码为 `is_spec`/`is_verified`（`ast_lowering/src/expr.rs:3207-3208`）后被 `elaboration/src/program/function_types.rs:188-193` 显式报错拒绝。 |
| `GAP-05-12` | `requires` / `ensures` / `old` / `assumes` 契约 | Core (`contracts-and-verification.md`) | `🟡 Partial` | `contracts::inherit` 检查 trait 实现前置条件弱化（`contracts.rs:189`, `flow::prove_implication`）；`program_verification/src/flow.rs` 配合 `verification_core` 与 `optimization::elide_proven_checks`（仅在 `luna/src/{packages.rs:729, package_cli.rs:915}` 可选消去 `CheckPredicate`）；常规构建仅在 `elaboration/src/program.rs:2128` 强制运行 `validate_machine_operations`，其余 `requires`/`ensures` 退化为运行时 `ProgramContractRejected`（`host/llvm/callables.rs:292,344` / `comptime_interpretor`）。 |
| `GAP-05-13` | `invariant` / `asserts` / `decreases` / `axiom` | Core (`safety.md`) | `🟠 Frontend-Only` | `parse/src/statements.rs:128-167` 与 `items.rs:556` 解析全部 4 种节点，`ast_lowering/src/clause.rs:47-67` 将 `Decreases` 降入 `ClauseConstraintKind::Decreases`，但 `ast_lowering/src/expr.rs` 仅降 `Assumes`，`elaboration/src/program.rs:3482` 及 `required.rs` 静默忽略 `Decreases`，无任何验证器消费。 |
| `GAP-05-14` | `outcomes` / `predicate` 声明 / `lemma` | Core (`safety.md`) | `🔴 Missing` | 无词法/语法/HIR/NIR/验证支持；仅 `fmt/src/bridge.rs:1109` 在格式化桥接期临时将 `outcomes` 文本映射为 `requires`（`outcomes_kw_offsets`）以便排版回环。 |
| `GAP-05-15` | `verification_core` 多理论有界求解器 | Core (`program_verification` / `verification_core`) | `🟢 Full` | `verification_core` 完整实现区间、`Relations`（多变量多项式 `<=0` 传播）、`DifferenceBounds`（稀疏闭合）、`congruence`（模 `{2,3,4,5,8,9}` 排零）、`quadratic`（16 维以下半正定矩阵证书）、`cubic`（单变量三次有限差分极值）、`factors`/`signs`/`division` 与可重放 `PredicateProof`。 |
| `GAP-05-16` | IIR `BehaviorAttributes`、Tarjan SCC 摘要与 LLVM 23 属性发射 | Core (`behavior.rs`, `iir/behavior.rs`) | `🟢 Full` | `behavior.rs:165-410` 基于 Tarjan SCC（最大不动点收缩、红线 1 禁止循环 SCC 设 `terminates`、红线 2 有 trap 强制降 `MemoryEffect::Unknown`）认证 `BehaviorAttributes`/`CallsiteBehavior`，经 `luna/src/cleanup.rs:1988` 与 `host/mod.rs:246` 接入 `host/llvm/attributes.rs` 发射 `nounwind`/`willreturn`/`nosync`/`nofree`/`memory(...)`/`speculatable` 及指针形参属性。 |
| `GAP-05-17` | `library/verification` 高级验证 API 与扩展文档 | Core + Wishlist (`safety.md`, `09`) | `⚪ Deferred` | `library/verification/lib.fl` 仅保留空 `pub mod prelude {}`，`docs/program-verification/specification-and-verification/` 为空目录，符合当前阶段暂缓策略。 |

### 支柱 4：运行时纯度分级（`P0`–`P5`）

| 编号 | 特性 / 机制 | 标尺属性 | 成熟度 | 现状摘要与核心源码位置 |
|---|---|---|---|---|
| `GAP-05-18` | `P4`（`pure fn`）与 `P3`（默认实参/谓词+有限终止）验证 | Core (`runtime-purity.md`, `purity.rs`) | `🟢 Full` | `purity.rs` `validate_runtime_promises` 严格检验 P4 值纯（局部 `var`/私有指针投影隐藏、`CodeKey` 多态回调实例、双世界 `with_definition_bodies` 校验）；`purity/observations.rs` `validate_defaults`/`validate_predicates` 强制 P3 + 严格有限终止。 |
| `GAP-05-19` | `P0`/`P1`/`P2`/`P5` 显式源码契约与细粒度分级 | Core (`runtime-purity.md`) | `🟡 Partial` | `purity.rs` 源码验证层仅二分为 P4（`!finite`）与 P3+有限终止（`finite`）；`P1`（有界修改）与 `P2`（全局只读）无源码/NIR 级别区分；`P5`（total pure）虽可由 `behavior.rs` + `attributes.rs:93-101` 对无循环/无 trap 叶子函数底向上发射 `speculatable`，但缺少源码级 `decreases` 驱动的总纯证明通路。 |
| `GAP-05-20` | 全局 `static` 读写的纯度区分 | Core (`runtime-purity.md`, `purity.rs:2058`) | `⚠️ Divergent` | `K::StaticAddress` 已接入 NIR/资源检查，但 `purity.rs:2058` 无条件返回 `false`，将**不可变 `static const` 读取**与可变 `static var` 一并拒绝于 P3/P4 之外；且早期日志 `purity_system_status_report_2026-11-21.md:41` 仍误称 `static` 尚未接入。 |
| `GAP-05-21` | `^attrs.mem.volatile` / `volatile` 降级至 `P0` | Core (`runtime-purity.md`, `library/attrs`) | `🟢 Full` | `library/attrs/mem_impl/volatile.fl` + `attributes/volatile.rs` 验证 `__flurry_volatile` 并禁止与 `packed` 混用；`nir/src/program/volatile_storage.rs` 传播字段/类型易失标记；`purity.rs:1081-1132` 拒绝任何易失读写进入 P3/P4；`behavior.rs:683-740` 触发 `degrade_for_volatile_access` 清空 `memory`/`readonly`/`noalias`/`nosync`；`host/llvm/attributes.rs:57` 做终检降级。 |
| `GAP-05-22` | `^attrs.purity.*` 显式纯度属性与 `pure fn(...)` 函数类型 | Core (`runtime-purity.md`, `library/attrs`) | `🔴 Missing` | `library/attrs/lib.fl` 与 `elaboration/src/program/attributes/` 均无 `purity` 子模块或 `__flurry_purity` 属性消费者；`function_types.rs:188-193` 明确拒绝 `pure fn(...)` 函数类型。 |
| `GAP-05-23` | `P3` 默认表达式误拒无 sysroot 下的 `comptime t: Type` | Core (`purity.rs:939, 1054-1059`) | `⚠️ Divergent` | `purity.rs:1057` 仅在 `!self.finite`（P4）豁免 `Primitive::Type`/`Integer`/`Float`，导致 P3（`self.finite == true`）下 `default_inputs_are_copy`（`:939`）与 `K::Local`（`:2006`）在缺少 `Copy` impl（无 sysroot 裸测试）时对阶段参数 `comptime t: Type` 报错失败。 |

### 支柱 5：错误行、代数效应、STM 与工程卫生

| 编号 | 特性 / 机制 | 标尺属性 | 成熟度 | 现状摘要与核心源码位置 |
|---|---|---|---|---|
| `GAP-05-24` | `!E T` / `![E1, E2] T` 错误行、`?` 传播与 `catch`/`error-catch-handle` | Core (`error-handling.md`, `AGENTS.md` §5) | `🟢 Full` | `nir/src/error_results.rs` + `error_handlers.rs` + `error_handlers/catches.rs` + `catches`（`before`/`arms`/`after`）完整实现错误行传输、残余行推导、全值保留清理与独立于效应继续体的低开销错误 ABI。 |
| `GAP-05-25` | 多错误行 `![E1, E2] T` 标签模式匹配与开放错误行变量 | Core (`cc178adb`, `errors.rs`) | `🟡 Partial` | `cc178adb` 在 `pattern_checking/errors.rs:49-78` 打通具名记录 `HP::Struct` 与枚举变体标签选择；但 `:60-76` 在多错误行中仍拒绝元组、原生标量（如 `!u32`/`!str`）、泛型形参等非变体/非记录标签，且不支持开放多态错误行（`![E, ...Es] T`）。 |
| `GAP-05-26` | `HandleError` / `HandleEffect` capability trait 协议 | Core (`handling-capability.md`, `AGENTS.md` §5) | `🔴 Missing` | `parse/src/items.rs:1169` 按规约拒绝 `handles` 关键字，但 `library/builtin/protocols/capabilities/` 仅含占位 `README.md`，编译器无 `HandleError`/`HandleEffect` trait 解析与 elaboration 接线。 |
| `GAP-05-27` | `effect` / `effect enum` 行、作用域标签索引、双向推导与直接处理器 | Core (`algebraic-effect.md`, `rows.md`, `effects/`) | `🟡 Partial` | `elaboration/src/effects/` 实现 `RowEmbedding`/`RowLayer`/`select_layer`、`effects/inference.rs` 双向推导与 `effects/spreads.rs` 展开，但 `effect_row_inner` 仅支持闭合列表/标签，且 `effects/handlers.rs:12-14, 119-125` 拒绝 `comptime` 效应处理与运行时依赖签名的源处理器。 |
| `GAP-05-28` | `DirectPassing`（`^attrs.effects.direct_passing`）与 `effect_usage` 重数验证 | Core (`direct-passing.md`, `effect_usage.rs`) | `🟡 Partial` | `library/attrs/effects_impl/strategy.fl` + `effect_usage.rs` + `iir/effect_values.rs` 完成零参、同步直接传递效应的多重调用/仿射槽位验证；但 `elaboration/src/program.rs:3493` 强制限定 `"direct passing effects must be synchronous and have no parameters"`，LLVM 亦受限于 `iir::effects::data_passing_signature`（`host/llvm/effects.rs:430`）。 |
| `GAP-05-29` | `inline effect` 与 `emit_item` 零成本展开 | Core (`algebraic-effect.md`, `iterator.fl`) | `🟢 Full` | `library/builtin/protocols/iterator.fl:14` `pub inline effect emit_item` 与 `elaboration/src/program/iteration.rs:454`（`metadata.inline && Abi::Direct`）打通编译器展开与库迭代组合子。 |
| `GAP-05-30` | `Abi::Capturing` 多发/非尾部定界继续体与多提示符控制 | Core (`multi-prompt-delimited-continuation.md`, `prompt.fl`) | `🔴 Missing` | `nir/src/effects.rs:10` 虽声明 `Abi::Capturing`，但 `elaboration/src/program/effects.rs:549-560` 从不构造它，`effects/handlers.rs:94-99` 显式拒绝非 Direct/DirectPassing 处理器；`special.rs:88-94` 仅允许封闭直接尾分支内使用 `resume`。 |
| `GAP-05-31` | 函数签名 `catch continuation: builtin.Continuation`（Silver P1-1） | Core + Wishlist (`04-compiler-wishlist.md`) | `🔴 Missing` | `elaboration/src/program.rs:3503-3507` 显式报错阻断：`"catch parameters require continuation binding, which is not yet implemented for function signatures"`，`builtin.Continuation` 亦未建立。 |
| `GAP-05-32` | `async effect` 与结构化取消/并发运行时 | Core + Wishlist (`async-algebraic-effect.md`, `08`) | `🟡 Partial` | `async effect` 声明已生成 `Abi::Async` 元数据（`program.rs:1785`、`execution.rs:27280`）且 LLVM 含基础 `task_context.ll` 原子同步，但 `async effect` 处理器源码降码被拒（`handlers.rs:94-99`），且无结构化取消作用域、异步调度器或效应网络运行时（`08`）。 |
| `GAP-05-33` | `atomic(...) { ... }` 软件事务内存（STM） | Core + Wishlist (`software-transactional-memory.md`, `08`) | `🟠 Frontend-Only` | `parse/src/expressions.rs:1234` 与 `fmt` 支持 `NodeKind::AtomicBlock`，`ast_lowering/src/expr.rs:134` 仅有死错误枚举 `AtomicBlockRequiredResource`（实际 `lower_expr` 未落地 STM 节点），无 HIR/NIR/冲突重试或运行时事务；设计文档 `software-transactional-memory.md` 为 0 字节。 |
| `GAP-05-34` | 审计范围内 IR/报告类型 S-expression `Display` 完备性 | Core (`AGENTS.md` §5) | `🟡 Partial` | 全审计域 `149` 个 `pub struct`/`pub enum` 中 `109` 个已实现 S-expr `Display`，但仍有 **40 个**公开类型缺失 `Display`（如 `flow::{Reason, PointKind}`、`verification_core::{PlaceId, Limits, ownership::{Shape, Availability}}`、`nir::phases::ErrorKind`、`moves::{Delivery, Cleanup, ...}`），导致日志与诊断退回 `{:?}`。 |
| `GAP-05-35` | 不变量/终止证明综合与张量形状/广播 SMT 验证 | Wishlist (`09-verification-synthesis-and-tensors.md`) | `🔴 Missing` | `09` 设计的循环不变式综合、自动 `decreases` 度量发现、张量维度代数与证书导出在编译器及 `verification_core` 中均无实现。 |

---

## 4. 关键缺口深度剖析（含源码证据）

### 4.1 运行时环境 `Drop` 演化与生命周期语义错位（`GAP-05-04`, `SND-05-01`, `CMP-05-01`）

`issues/0003` 与 `drop/plan.rs:430-431` 的旧描述称 runtime env `Drop` 完全不可用，但 `HEAD` 源码已大幅推进，同时暴露出三层真实缺口：
1. **已实现能力（打破过期认知）**：`drop_environment`（[drop.rs:484-550](file:///home/fin/luna/compiler/program_verification/src/drop.rs#L484-L550)）收集 `SelectedDrop` 的显式 `env_parameters` 与通过 `implicit_runtime_env_parameters` 推导的隐式运行环境依赖；随后 `drop/access.rs:275-349` 从 `DropEnvContext`（调用点当前函数的 `env_parameters`、`runtime_deliveries` 及包级 `global_runtime_deliveries`）经 `nir::runtime_fields::address` 投影并检查指针可读目标（`resolve_env_pointer_targets`），使标准 `Box`/`Vec` 等默认/显式抬升指针型 allocator 在直接作用域内的销毁得以编译执行。
2. **语义偏离（清理点查找 vs 构造/移动期捕获）**：[docs/resource-management/drop-elaboration.md](file:///home/fin/luna/docs/resource-management/drop-elaboration.md) 要求依赖环境对象在构造/传递时锁定或携带对应的运行环境身份；而 `drop/access.rs:278-308` 是在**每个作用域清理点**反查外层 `env_ctx`：
   ```rust
   // compiler/program_verification/src/drop/access.rs:308 & 341-345
   .ok_or("Drop runtime environment argument is not deliverable in this scope")?;
   ...
   match &req.ty {
       Term::Pointer(pointee) => { /* resolve_env_pointer_targets */ }
       _ => return Err("Drop runtime environment value must currently be a lifted pointer".into()),
   }
   ```
   当对象通过返回值、闭包捕获或条件分支跨越不同 allocator 作用域时，清理点作用域未必拥有构造点绑定；非指针型运行环境句柄直接报错。
3. **高阶与边界封锁**：`moves/operations.rs:696-766` 与 `moves.rs:9201-9206` 在 5 类调用路径直接拦截带 `env_arguments` 的调用（临时调用、局部候选集、间接具名调用、闭包调用、guard 调用）；`drop/plan.rs:456-484` 拒绝 `Resolution::Assumed` 或带未具象 `capabilities` 的 `Drop`；`drop/plan.rs:779-786` 拒绝任何带有 `preconditions`/`postconditions` 的 `Drop` 实现。

### 4.2 `str` 文本视图来源分裂与 `native-text-method-provenance.fl` 根因（`GAP-05-08`, `SND-05-03`, `CMP-05-02`）

`development/repros/native-text-method-provenance.fl`（`"hello".len_bytes()`）揭示的是**`TextProvenance` 双轨不一致**而非文本区域分析完全缺失：
- **正向通路**：[luna/src/cleanup.rs:693-706](file:///home/fin/luna/compiler/luna/src/cleanup.rs#L693-L706) 在图含 `str`（`graph_has_text`）时切换为 `TextProvenance::VerifiedByRegionAnalysis`，配合 `memory/text_regions.rs` 追踪 `TextRegionOrigin` 并允许 `str` 通过方法签名与布局检查；`purity.rs:865-871` 在 `!self.finite`（P4）下同样启用 `VerifiedByRegionAnalysis`。
- **阻断路径**：
  1. `luna/src/protocols.rs:300-323`（`analyze_cleanup_with_borrowed_code_context`）与 `500-566`（`analyze_method_cleanup` -> `analyze_method_cleanup_with_context`）调用了 `moves::analyze_in_package_with_borrowed_code_context`（[moves.rs:15052](file:///home/fin/luna/compiler/program_verification/src/moves.rs#L15052)），后者硬编码 `TextProvenance::Unchecked`，触发 [memory/borrowed_layout.rs:96-98](file:///home/fin/luna/compiler/program_verification/src/memory/borrowed_layout.rs#L96-L98) 与 [memory.rs:674-676](file:///home/fin/luna/compiler/program_verification/src/memory.rs#L674-L676) 报错：`"string-view memory provenance is not implemented; str requires its backing character storage to remain valid"`。
  2. 此外，`memory/borrowed_code.rs:99`、`flow.rs:2180,2201`、`memory/effects.rs:156,176` 等多处公开/内部入口同样固定传入 `TextProvenance::Unchecked`；`cleanup.rs:543` 在新生成布局扩展后重设 `graph_has_text = false`；且 `memory/text_regions/transfers.rs:1184-1190` 禁止任意 intrinsic 产生 `str` 视图，阻碍基于 FFI/内存原语构建零拷贝字符串视图。

### 4.3 安全层级、`spec`/`verified` 与静态验证管线的断裂（`GAP-05-11`–`14`, `SND-05-02`）

Flurry 设计了业界领先的 `verification_core`（支持多项式关系、16 维半正定、单变量三次有限差分极值、模 `{2,3,4,5,8,9}` 同余排零与稀疏差分图闭包）以及四级安全体系（`safe`/`spec`/`verified`/`unsafe`），但**前后端之间存在明显断层**：
- **修饰符与子句漏斗**：
  - `parse` 层识别 `TokenKind::{Spec, Verified, Axiom, Decreases, Invariant, Asserts, Assumes}`（[statements.rs:128-167](file:///home/fin/luna/compiler/parse/src/statements.rs#L128-L167), [items.rs:469-581](file:///home/fin/luna/compiler/parse/src/items.rs#L469-L581)）。
  - `ast_lowering/src/item.rs:236-245` 仅将 `__flurry_kw_comptime`/`_quote`/`_pure`/`_unsafe`/`_async`/`_inline` 填入修饰符，`__flurry_kw_spec`/`__flurry_kw_verified` 在 `_ => break` 中未被消费。
  - `ast_lowering/src/expr.rs:3207-3208` 虽将 `spec`/`verified` 降至函数类型修饰符，但 `elaboration/src/program/function_types.rs:188-193` 立即拒绝对外暴露的任何修改后函数类型。
  - `ast_lowering/src/clause.rs:47-67` 将 `Decreases` 降至 `hir::clause::ClauseConstraintKind::Decreases`，而 `elaboration/src/program.rs:3482` 仅遍历 `Requires`，完全跳过 `Decreases`。
- **管线执行现状**：在 `elaboration/src/program.rs:2128-2133` 中，强制运行的是面向整数溢出/除法/位窗口的 `flow::validate_machine_operations_*` 与检查 trait LSP 弱化的 `contracts::inherit`；完整的 `flow::analyze` 仅作为可选优化 pass `optimization::elide_proven_checks` 出现在 `luna/src/{packages.rs:729, package_cli.rs:915}`（遇到 `Unknown`/`Unsupported` 时静默保留运行时 `CheckPredicate`），并未作为 `verified fn` 的静态准入门槛在 `luna build` 中强制拒绝未证契约。

### 4.4 运行时纯度 `P0`–`P5` 实际能力版图（`GAP-05-18`–`23`）

- **`P4` 与 `P3` 实质完成**：`purity.rs` 已覆盖私有局部变量读写隐藏、基于 `CodeKey` 的参数化闭环递归与高阶回调实例化、以及开放 witness 与闭合残余双世界联合审查（`elaboration/src/program/purity.rs:28-64`）。
- **`P0`/`P5` 靠 IIR 补齐，`P1`/`P2` 与属性缺失**：`behavior.rs` + `host/llvm/attributes.rs` 通过 IIR Tarjan SCC 认证自底向上为函数生成 LLVM 23 属性（含满足红线 1/2 时的 P5 级 `speculatable` + `memory(none)` + `willreturn` + `nounwind` + `nofree` + `nosync`），并利用 `volatile_storage.rs` 将易失访问降级为 P0。但在语言表面，`^attrs.purity.*` 完全缺席，`static const` 读取因 `K::StaticAddress(_) => false`（`purity.rs:2058`）在 P3/P4 中一律失效，且 `copy_type`（`purity.rs:1057`）在 `self.finite == true` 时遗漏对 `Term::Primitive(P::Type | P::Integer | P::Float)` 的豁免。

### 4.5 错误行（`cc178adb` 进展与边界）与代数效应非直传鸿沟（`GAP-05-24`–`33`）

- **错误系统现状**：单/多标签 `!E T` 与 `![E1, E2] T` 已支持独立无延续栈 ABI、结构化传输与 `catch` 相位（`before`/`arms`/`after`）保留全值所有权。提交 `cc178adb` 新增 [pattern_checking/errors.rs](file:///home/fin/luna/compiler/elaboration/src/program/pattern_checking/errors.rs)，解决了具名结构体记录（`HP::Struct`）与枚举变体在多标签错误行中的判别；但 `:60-76` 仍无法匹配原生类型（如 `![IoErr, u32]`) 或元组作为多错误行成员，且尚无开放错误行变量。
- **效应系统分水岭**：仅 `Abi::Direct`（尾调单次直传与 `inline effect` 如 `emit_item`）与零参同步 `Abi::DirectPassing`（`^attrs.effects.direct_passing` + `effect_usage.rs` 重数校验）具备完整编译执行路径。一旦涉及以下任意一项即直接阻断：
  1. `Abi::Capturing` / `Abi::Async` 处理器（[effects/handlers.rs:94-99](file:///home/fin/luna/compiler/elaboration/src/program/effects/handlers.rs#L94-L99)）；
  2. 运行时依赖签名的效应处理器（[effects/handlers.rs:119-125](file:///home/fin/luna/compiler/elaboration/src/program/effects/handlers.rs#L119-L125)）；
  3. 含 `comptime` 求值域的效应处理器（[effects/handlers.rs:12-14](file:///home/fin/luna/compiler/elaboration/src/program/effects/handlers.rs#L12-L14)）；
  4. 函数签名 `catch` 绑定续体 `builtin.Continuation`（[elaboration/src/program.rs:3503-3507](file:///home/fin/luna/compiler/elaboration/src/program.rs#L3503-L3507)）；
  5. 带参数 `direct_passing` 效应（[elaboration/src/program.rs:3493](file:///home/fin/luna/compiler/elaboration/src/program.rs#L3493)）；
  6. `HandleError`/`HandleEffect` 能力 trait 与 `atomic(...)` STM（仅止步于 `parse`/`ast_lowering` 枚举定义）。

---

## 5. 架构偏离与技术债清单（⚠️ Divergent 专项）

| 编号 | 偏离描述与违背原则 | 实证源码位置 | 修复要求 |
|---|---|---|---|
| `DIV-05-01` (`GAP-05-04`) | **`Drop` 运行时环境采用“清理点反查”而非“构造/移动期捕获”**，且不支持非指针 env 值，同时 [drop/plan.rs:430-431](file:///home/fin/luna/compiler/program_verification/src/drop/plan.rs#L430-L431) 保留声称“runtime env 未实现”的误导性文档注释。违背 [drop-elaboration.md:46](file:///home/fin/luna/docs/resource-management/drop-elaboration.md#L46) 与“容器不得依赖局部作用域隐式存在”的依赖资源设计。 | [drop/access.rs:275-349](file:///home/fin/luna/compiler/program_verification/src/drop/access.rs#L275-L349), [drop/plan.rs:430-431](file:///home/fin/luna/compiler/program_verification/src/drop/plan.rs#L430-L431) | 修正过时注释；将非全局 env 身份随 `CleanupTree`/所有权槽位捕获并扩展 `DropEnvContext` 至非指针环境值及间接/临时调用路径。 |
| `DIV-05-02` (`GAP-05-08`) | **`TextProvenance` 状态在各入口之间割裂**（`luna/cleanup.rs` 与 `purity.rs(P4)` 使用 `VerifiedByRegionAnalysis`，而 `protocols.rs:310/551` -> `moves.rs:15052`、`memory/borrowed_code.rs:99` 与 `memory/effects.rs:156` 默认注入 `TextProvenance::Unchecked`），直接引发 `native-text-method-provenance.fl` 类特例失败。 | [moves.rs:15052](file:///home/fin/luna/compiler/program_verification/src/moves.rs#L15052), [luna/src/protocols.rs:300-323, 500-566](file:///home/fin/luna/compiler/luna/src/protocols.rs#L300-L323), [luna/src/cleanup.rs:543, 693-706](file:///home/fin/luna/compiler/luna/src/cleanup.rs#L543) | 消除分裂枚举或使方法/借用/效应分析统一经由 `VerifiedByRegionAnalysis` 会话管线，不再对 `str` 方法静默降回 `Unchecked`。 |
| `DIV-05-03` (`GAP-05-20`) | **`K::StaticAddress(_)` 在 `purity.rs` 中一刀切返回 `false`**，未遵循 [runtime-purity.md](file:///home/fin/luna/docs/resource-management/runtime-purity.md) 对不可变全局常量读取（应属 P2/P3/P4 可证分支）与可变 `static var`（属外部状态读写 P0/P1）的区分。 | [purity.rs:2058](file:///home/fin/luna/compiler/program_verification/src/purity.rs#L2058) | 查询 `package.statics` 的可变性/初始化完成属性，放行已初始化不可变 `static` 只读访问进入对应纯度级别。 |
| `DIV-05-04` (`GAP-05-23`) | **`purity.rs` 对阶段形参 `Type`/`Integer`/`Float` 的免除规则在 P3 模式下缺漏**，`copy_type` 仅在 `!self.finite`（P4）下豁免 `Term::Primitive(P::Type \| P::Integer \| P::Float)`，致使默认表达式引用 `comptime t: Type` 时在无 sysroot 环境误报缺少运行时 `Copy` 实现。 | [purity.rs:939, 1018-1059, 2006](file:///home/fin/luna/compiler/program_verification/src/purity.rs#L1054-L1059) | 将 `staged_parameters`（及不可运行期物化的阶段类型输入）与 `self.finite` 条件解耦，在 P3 验证中也正确豁免纯编译期值。 |

---

## 6. 健全性 · 完备性 · 鲁棒性专项审计（SND / CMP / ROB）

### 6.1 健全性审计（SND-05-NN）

1. **`SND-05-01`（Critical）· `DropEnvContext` 清理点动态解析而非创建点捕获**
   - **依据**：[drop/access.rs:278-308](file:///home/fin/luna/compiler/program_verification/src/drop/access.rs#L278-L308) 在清理发生所在函数帧的 `env_parameters`/`runtime_deliveries`/`global_runtime_deliveries` 按 `req.identity` 查找 env 表达式。
   - **风险**：当携带自定义 `.allocator` 的容器从子函数返回、存入外层结构或跨越不同作用域的 `runtime_deliveries` 后，原构造点的 env 绑定不再位于清理点词法作用域，极易因作用域错配无法交付甚至在同名/重用符号下混淆环境实例。
2. **`SND-05-02`（High）· `spec` / `verified` 修饰符静默失效且常规编译缺少全契约静态准入门禁**
   - **依据**：[ast_lowering/src/item.rs:236-245](file:///home/fin/luna/compiler/ast_lowering/src/item.rs#L236-L245) 未匹配 `__flurry_kw_spec`/`__flurry_kw_verified`；[elaboration/src/program.rs:3482](file:///home/fin/luna/compiler/elaboration/src/program.rs#L3482) 忽略 `Decreases`；[elaboration/src/program.rs:2128-2133](file:///home/fin/luna/compiler/elaboration/src/program.rs#L2128-L2133) 仅强制运行机器运算验证（`validate_machine_operations`），[luna/src/packages.rs:729](file:///home/fin/luna/compiler/luna/src/packages.rs#L729) 与 `package_cli.rs:915` 仅将 `elide_proven_checks` 作为最优努力（best-effort）优化调用。
   - **风险**：用户书写 `verified fn` 或声明 `decreases`/`invariant` 时编译器不给出未证明拦截，契约静默降级为运行时检查或完全被丢弃。
3. **`SND-05-03`（High）· `TextProvenance` 双轨判据及 `cleanup.rs:543` 状态重置风险**
   - **依据**：[memory/borrowed_layout.rs:81-118](file:///home/fin/luna/compiler/program_verification/src/memory/borrowed_layout.rs#L81-L118)、[luna/src/cleanup.rs:543, 693-706](file:///home/fin/luna/compiler/luna/src/cleanup.rs#L543)、[purity.rs:865-871](file:///home/fin/luna/compiler/program_verification/src/purity.rs#L865-L871) 与 [moves.rs:15052](file:///home/fin/luna/compiler/program_verification/src/moves.rs#L15052)。
   - **风险**：不同分析模块对 `str` 是否持有独立借用叶子做出相反结论；`cleanup.rs:543` 在每次动态生成枚举布局后重置 `graph_has_text = false` 依赖后续 `validated_instances.insert` 重建（但仅对新入实例循环！若此前已入实例含 `str` 而被标记为 validated，请留意 `validated_instances.clear()` 位于 `:539` 才勉强兜底，一旦二者顺序调整即跳过 `TextRegions` 审查！）。
4. **`SND-05-04`（Medium）· `flow::CheckInput::site` 依赖原始堆内存地址对齐检查点**
   - **依据**：[flow.rs:58-64](file:///home/fin/luna/compiler/program_verification/src/flow.rs#L58-L64) 与 [optimization.rs:126](file:///home/fin/luna/compiler/program_verification/src/optimization.rs#L126) 使用 `expr as *const Expr as usize` 作为 `site` 键并在 `expression_mut` 回调中核销 `CheckPredicate`。
   - **风险**：依赖裸指针地址在同一份未重分配 `Expr` 树上保持稳定；虽然 `optimization.rs:86-93` 通过先克隆再 `analyze_in_package` 规避了崩溃，但公开的 `flow::Report` 极易因外部消费方克隆/移动 `Function` 而产生悬空或错位匹配。
5. **`SND-05-05`（Medium）· LLVM 机器属性后置消毒依赖汇编文本子串扫描**
   - **依据**：[host/llvm/attributes.rs:48-60](file:///home/fin/luna/compiler/luna/src/host/llvm/attributes.rs#L48-L60) 通过 `text.contains("@llvm.trap")` 与 `prologue.contains(" volatile ") \|\| body.contains(" volatile ")` 字符串子串检查来决定是否对 `BehaviorAttributes` 执行最后降级。
   - **风险**：非结构化文本匹配受 LLVM 汇编格式空格或注释/字面量干扰（如缺少前导空格的 `volatile` 内联写法或不同靶标陷阱命名），不如直接由代码生成器维护结构化 side-effect 标志稳健。

### 6.2 完备性审计（CMP-05-NN）

1. **`CMP-05-01`（Critical）· 运行时 `env_arguments` 与多形态 `Drop` 契约阻断高阶与自定义 allocator 生态**
   - `moves/operations.rs:696-766` 与 `moves.rs:9201-9206` 拒绝临时/闭包/间接/候选/guard 调用携带 `env_arguments`；`drop/access.rs:341-345` 仅支持指针型 env；`drop/plan.rs:456-484, 779-786` 拒绝 assumed/contracted `Drop`；`moves.rs:10192-10205` 拒绝含指针或可拒绝 `Drop` 的 `Pattern::Then` 临时值。
2. **`CMP-05-02`（High）· `str` 方法调用（`native-text-method-provenance.fl`）与 intrinsic 文本合成缺口**
   - `luna/src/protocols.rs:300-323, 500-566` 走 `Unchecked` 导致对原生 `str` 的方法校验失败；`memory/text_regions/transfers.rs:1184-1190` 无法为 FFI/intrinsic 输出构造文本区来源证明。
3. **`CMP-05-03`（High）· 非直传效应与能力句柄缺口**
   - `Abi::Capturing` 与 `Abi::Async` 处理器无法源级降码（`effects/handlers.rs:94-99`）；`comptime`/运行时依赖效应处理器无法源级降码（`:12-14, :119-125`）；`catch continuation: builtin.Continuation`（Silver P1-1）被 `program.rs:3503-3507` 拒绝；有参/异步 `DirectPassing` 被 `program.rs:3493` 拒绝；`HandleError`/`HandleEffect` trait 未实装；开放效应/错误行变量（`![E, ...Es]`、`#row`）未实现。
4. **`CMP-05-04`（High）· 安全层级/`decreases`/`invariant`/`axiom`/`outcomes`/`lemma`/`predicate` 与求解器缩放限制**
   - 语言级证明语法全部停留在解析或空缺阶段，无法驱动 `verification_core`；同时求解器本身存在明确规模天花板：`quadratic.rs:62` 半正定矩阵上限 `MAX_COORDINATES = 16`、`congruence.rs:26,45` 变量 `<= 8` 且余数积 `<= 256`、`differences.rs:65-67` 矩阵格点数上限 `max_terms = 1024`（仅容 `<= 31` 个活跃差分变量）、`cubic.rs` 仅限单变量三次多项式。
5. **`CMP-05-05`（High）· `&r T` 可达性类型、`MemoryView`、Pin-free 自引用与 `atomic` STM 缺位**
   - `&r T` 止于 HIR 依赖收集跳过（`dependencies.rs:1107`）；`MemoryView` 与 `self-reference` 完全缺失（`🔴 Missing`）；`atomic(...)` 止步于 AST 与未接入的 `AtomicBlockRequiredResource` 枚举。
6. **`CMP-05-06`（Medium）· 多标签错误行 `![E1, E2] T` 尚不支持非变体/非具名记录模式**
   - `pattern_checking/errors.rs:49-78` 虽在 `cc178adb` 后支持结构体与枚举变体，但遇到原生标量、元组或类型变量作为多错误标签成员时报错（`"multiple error types require a type-identifying payload pattern"`）。
7. **`CMP-05-07`（Medium）· `P0`/`P1`/`P2`/`P5` 源码分级、`^attrs.purity.*` 与 `static const` 读取**
   - `library/attrs/lib.fl` 无 `purity` 属性；`function_types.rs:188-193` 禁止 `pure fn(...)` 函数类型签名；`purity.rs:2058` 阻止不可变 `static` 在 `pure fn`/默认值中被读取。
8. **`CMP-05-08`（Medium）· 无 sysroot 下 `P3` 默认表达式误杀 `comptime t: Type`**
   - 见 `purity.rs:939, 1018-1059, 2006`。
9. **`CMP-05-09`（Low）· 异步结构化取消、Wishlist 08/09 与 4 份零字节规范文档**
   - `tmp/wishlist/{08,09}` 中的效应网络调度、不变量/张量验证、`library/verification` 及 4 份 0 字节文档（`affine-type.md`、`reachability-type.md`、`reachability-type-also.md`、`software-transactional-memory.md`）尚待补全。

### 6.3 鲁棒性审计（ROB-05-NN）

#### 量化生产代码普查表（已剔除 `tests/`、`_tests.rs`/`tests.rs` 及 `#[cfg(test)]` 块）

| 子系统 / 模块集合 | Prod 文件数 | Prod 行数 | `.unwrap()` | `.expect()` | `panic!()` | `unreachable!()` | `todo!/unimpl!` | `HashMap/Set` | `0..0` Span |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| [program_verification/src](file:///home/fin/luna/compiler/program_verification/src) | 54 | 75,259 | 78 | 26 | 0 | 99 | 0 | 144 | 6 |
| [verification_core/src](file:///home/fin/luna/compiler/verification_core/src) | 13 | 6,758 | 15* | 2 | 0 | 2 | 0 | 0 | 0 |
| `elaboration` 效应/错误处理 | 13 | 3,977 | 8 | 0 | 0 | 5 | 0 | 0 | 0 |
| `nir`（`contracts`/`error_results`/`copy_budget`/`phases`/`effects`/`call_safety`） | 25 | 8,761 | 0 | 3 | 0 | 7 | 0 | 13 | 13 |
| **审计范围总计** | **105** | **94,755** | **101** | **31** | **0** | **113** | **0** | **157** | **19** |

> \*注：`verification_core` 的 `15` 个 `.unwrap()` 中包含 [relations.rs:96, 113](file:///home/fin/luna/compiler/verification_core/src/relations.rs#L96) 2 处带函数级 `#[cfg(test)]` 标注的测试辅助实现；纯无条件生产 `.unwrap()` 为 `13` 处（[differences.rs:81,82,86,120,268,271,485,546](file:///home/fin/luna/compiler/verification_core/src/differences.rs#L81) 共 8 处、[relations.rs:632,838,863](file:///home/fin/luna/compiler/verification_core/src/relations.rs#L632) 共 3 处、[cubic.rs:31,123](file:///home/fin/luna/compiler/verification_core/src/cubic.rs#L31) 共 2 处）。此外 105 个文件共 **245** 处 panic 原语调用（`101+31+0+113+0`）。

1. **`ROB-05-01`（High）· `tarjan_sccs::strongconnect` 缺乏递归深度/栈保护**
   - **依据**：[behavior.rs:372-386](file:///home/fin/luna/compiler/program_verification/src/behavior.rs#L372-L386) 的递归 `strongconnect(w, st)` 在整个调用图上以 Rust 原生函数调用栈展开，无 `depth >= ...` 限深保护也未改写为显式迭代栈（相比之下 `program_verification` 其余 AST/NIR 遍历普遍设置 `depth >= 64/128` 护栏）。
   - **影响**：大型或深度耦合的程序调用图（特别是单态化后成百上千例程）在 `luna build` 时可能直接触发宿主线程栈溢出（SIGSEGV）。
2. **`ROB-05-02`（Medium）· 核心验证与效应降低路径存在 245 处生产级 panic 面**
   - **热点证据**：
     - [memory/text_regions/transfers.rs](file:///home/fin/luna/compiler/program_verification/src/memory/text_regions/transfers.rs)：`16` `.unwrap()`（含 `:18, 67, 131, 273, 680, 692, 871..901, 1003, 1065, 1180, 1222, 1256, 1434`）与 `31` `unreachable!()`；
     - [moves.rs](file:///home/fin/luna/compiler/program_verification/src/moves.rs)：`13` `.unwrap()`、`11` `.expect()`、`3` `unreachable!()`；
     - [flow.rs](file:///home/fin/luna/compiler/program_verification/src/flow.rs)：`11` `.unwrap()`（含 `:6466` `TargetData::new(width).unwrap()`）、`6` `.expect()`（含 `:6408-6422` 多线程 `Mutex` 中毒 panic 与 `.expect("failed to spawn verification worker thread")`）、`9` `unreachable!()`；
     - [verification_core](file:///home/fin/luna/compiler/verification_core/src)：`differences.rs` `binary_search(...).unwrap()` 及 `node.unwrap()` 共 `8` 处、`relations.rs` `3` 处生产 `.unwrap()`、`cubic.rs:31,123` `2` 处、`ownership.rs:431,548` `2` 处 `.expect()`；
     - [elaboration/src/program/effects.rs](file:///home/fin/luna/compiler/elaboration/src/program/effects.rs)（`6` `.unwrap()` 如 `:276, 464, 497, 525, 533, 542`）与 [purity.rs:744, 1560, 1582](file:///home/fin/luna/compiler/program_verification/src/purity.rs#L744)（`parameter_sorts.as_ref().unwrap()`）。
   - 若上游传入畸形 NIR 或燃料/深度边界错位，易演变为 ICE panic 而非有序诊断。
3. **`ROB-05-03`（Medium）· 157 处非确定性 `HashMap`/`HashSet` 使用与清理依赖遍历顺序隐患**
   - **依据**：`program_verification/src/memory.rs` 含 `59` 处、`memory/text_regions.rs` `26` 处、`borrowed_layout.rs` `14` 处、`purity.rs` `11` 处、`behavior.rs` `7` 处、`drop/plan.rs` `7` 处、`nir/phases/storage_*` `13` 处；且 [luna/src/cleanup.rs:285-315](file:///home/fin/luna/compiler/luna/src/cleanup.rs#L285-L315) 直接遍历 `changed: HashSet` 与 `instance_deps: HashMap` 构建 `callers_of` 与 `affected`。
   - 当前因输出有序化或多为查找索引暂未直接导致代码生成乱序，但迭代 `HashMap`/`HashSet` 构造依赖列表（如 `cleanup.rs:299-307` 将 `caller` 压入 `Vec`）对随机哈希种子敏感，后续若依据工作队列首次失败顺序报错即会产生非确定性诊断。
4. **`ROB-05-04`（Medium）· 19 处 `SourceRange { start: 0, end: 0 }` 位置丢失引发“零位置幽灵报错”**
   - **依据**：[nir/src/phases.rs:355,376,383,387,393,401](file:///home/fin/luna/compiler/nir/src/phases.rs#L355)、[nir/src/phases/declarations.rs:20,28,43,48,57](file:///home/fin/luna/compiler/nir/src/phases/declarations.rs#L20)、[nir/src/phases/parameters.rs:26](file:///home/fin/luna/compiler/nir/src/phases/parameters.rs#L26)、[nir/src/phases/inspection.rs:156](file:///home/fin/luna/compiler/nir/src/phases/inspection.rs#L156)、[program_verification/src/contracts.rs:214](file:///home/fin/luna/compiler/program_verification/src/contracts.rs#L214)、[purity.rs:711](file:///home/fin/luna/compiler/program_verification/src/purity.rs#L711)、[memory.rs](file:///home/fin/luna/compiler/program_verification/src/memory.rs)。直接对应 `issues/0003:29` 报告的 `位置 0..0` 报错体验问题。
5. **`ROB-05-05`（Low）· 40 个公开类型缺少 `AGENTS.md` §5 要求的 S-expression `Display`**
   - 详见 `GAP-05-34`（如 `flow::Reason`、`flow::PointKind`、`verification_core::{PlaceId, Limits, ownership::{Shape, Availability}}`、`nir::phases::ErrorKind`、`moves::TemporaryBoundary`），在错误报告与转储边界被迫使用 Rust `{:?}` 调试格式。

---

## 7. 修复路径、依赖与验收建议

### 优先级 P0（立刻解决阻塞与健全性红线）

1. **修正 `Drop` 运行时环境生命周期捕获（`GAP-05-04`, `SND-05-01`, `CMP-05-01`）**：
   - 将非全局 `runtime_deliveries` / `env_parameters` 绑定时机前移至值构造与 `CleanupTree` 转移点（`verification_core/src/ownership.rs` + `program_verification/src/moves.rs`），不再仅依赖清理点词法 `DropEnvContext`；扩展 `drop/access.rs:328-346` 支持非指针标量/句柄 env 值，消除 `drop/plan.rs:430-431` 过期注释，并打通 `moves/operations.rs:696-766` 对闭包/间接/临时/guard 调用的 `env_arguments` 支持。
2. **统一 `TextProvenance` 通路并解除 `native-text-method-provenance.fl` 阻塞（`GAP-05-08`, `SND-05-03`, `CMP-05-02`）**：
   - 改造 [luna/src/protocols.rs:300-323, 500-566](file:///home/fin/luna/compiler/luna/src/protocols.rs#L500-L566) 及 [moves.rs:15052](file:///home/fin/luna/compiler/program_verification/src/moves.rs#L15052)，使 `analyze_method_cleanup` 与借用代码分析运行在启用 `VerifiedByRegionAnalysis` 的会话内；修复 `cleanup.rs:543` 状态重设逻辑，并规划 `text_regions/transfers.rs:1184-1190` 对明确注解借用来源的文本输出 intrinsic 的支持。
3. **修复 `purity.rs` P3 `Type` 豁免与 `static const` 读取（`GAP-05-20`, `GAP-05-23`）**：
   - 在 `purity.rs:1054-1059` 放行无论 P3/P4 下的已标记 `staged_parameters` / 纯编译期元类型；在 `purity.rs:2058` 按 `package.statics` 区分不可变 `static` 读取与可变 `static var`。

### 优先级 P1（核心语言语义闭环与安全证明接线）

4. **打通安全层级与合约静态验证闭环（`GAP-05-11`–`13`, `SND-05-02`, `ROB-05-01`）**：
   - 在 `ast_lowering/src/item.rs:236-245` 保留 `spec`/`verified` 修饰符；在 `elaboration` 为 `verified fn` 与 `decreases`/`invariant`/`asserts` 建立经 NIR 到 `program_verification::flow` 的强制验证门禁（复用现有 `verification_core`）；将 `behavior.rs:372-386` 的 `strongconnect` 改写为显式迭代 Tarjan 算法以防栈溢出。
5. **补齐关键效应与错误处理缺口（`GAP-05-25`, `GAP-05-26`, `GAP-05-28`, `GAP-05-30`, `GAP-05-31`）**：
   - 扩展 `pattern_checking/errors.rs:49-78` 支持原生类型/元组标签及开放错误行；实现 `HandleError`/`HandleEffect` trait 协议；按 Silver P1-1 打通 `elaboration/src/program.rs:3503` 的 `catch continuation: builtin.Continuation`；逐步放开带参数 `DirectPassing`（`program.rs:3493`）与 `Abi::Capturing` 帧捕获降码。

### 优先级 P2（类型级内存安全、纯度属性与鲁棒性加固）

6. **实现 `&r T` 可达性、`MemoryView`、`Pin-free` 自引用与 `^attrs.purity.*`（`GAP-05-05`, `09`, `10`, `22`）**：
   - 先补全 `docs/type/{affine-type, reachability-type, qualified-types/reachability-type-also}.md` 设计规格，再打通 `ReachabilityType`（`&r T`）的 elaboration/NIR/region 校验、`MemoryView` 协议、`self-reference` 协议及 `library/attrs/purity` 子模块。
7. **消除 `ROB` 隐患（`ROB-05-02`–`05`）**：
   - 系统将 `verification_core::differences`/`relations`/`cubic` 及 `transfers.rs` 的 101 处 `.unwrap()` 替换为带语义上下文的结构化 `Result` 错误；将 `flow::CheckInput::site` 从裸指针转换为稳定 AST/NIR 节点索引；补齐 40 个缺失 S-expression `Display`；传递源位置替换 19 处 `0..0` span；用 `BTreeMap`/`BTreeSet` 收口 `luna/src/cleanup.rs:285-315` 工作队列。

### 优先级 P3（并发事务、异步运行时与 Wishlist 扩展）

8. **推进 `async effect` 调度、`atomic` STM 与 Wishlist 09 综合验证（`GAP-05-32`, `33`, `35`, `17`）**：
   - 在 `software-transactional-memory.md` 定稿后实现 `NodeKind::AtomicBlock` HIR/NIR 事务边界；完善 `Abi::Async` 处理器降码与结构化取消；按需推进 `library/verification` 与张量/不变式自动综合。

### 最小可验证验收建议

1. **资源与文本回归**：对带显式 `std.mem.DefaultAllocator` / 自定义局部 allocator 的 `Box`/`Vec` 在跨函数返回与局部闭包路径验证 `Drop` 正确捕获原 env；验证 `luna check` / `luna build` 对 `"hello".len_bytes()` 及返回 `str` 的方法链路不在 `pointer_input_layout` 报 provenance 错误。
2. **纯度与安全验收**：在无 `--sysroot` 最小编译下验证含 `comptime t: Type` 的 P3 默认实参通过；构造读不可变 `static const` 的 `pure fn` 通过且写 `static var` 的 `pure fn` 被拒；构造未满足 `requires`/`decreases` 的 `verified fn` 在编译期被 `flow` 显式拦截。
3. **错误与效应验收**：构造含 `!u32` 与 `!IoErr` 混合标签及记录模式的 `![IoErr, u32] T` 精确匹配；验证带参数 `DirectPassing`、直接 `catch continuation: builtin.Continuation` 绑定与 S-expression dump 在各错误/验证类型上括号配对且零 `{:?}` 回退。
