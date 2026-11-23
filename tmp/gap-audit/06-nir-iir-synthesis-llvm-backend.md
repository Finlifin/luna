# Flurry Backend 架构与实现差距全面审计（06：NIR / IIR / Synthesis / LLVM Backend）

> **审计对象**：`compiler/nir`、`compiler/iir`、`compiler/materialization`、`compiler/bitvec_opt`、`compiler/pattern_automata`（消费面）、`compiler/intrinsic`、`compiler/luna/src/{host/, resources/, cleanup/, protocols/}`  
> **检查基线**：`dev` 分支 `HEAD = b9a52238edaf31207f3061e7a4afdc8c18692695`（只读代码静态审计，未运行 `cargo build` / `cargo test`）  
> **文档性质**：Gap 差距、架构偏离及健全性/完备性/鲁棒性（SND / CMP / ROB）专项审计

---

## 1. 审计范围、检查基线与设计标尺

### 1.1 实码检查范围与资产盘点

本轮审计按实码证据覆盖后端及相关支撑边界全部源码、内建运行时 IR 模板与集成测试套件：

| 模块 / Crate | 非测试 `.rs` 文件数 | 非测试源码行数 | 核心职责与关键实现入口 |
|---|---:|---:|---|
| `compiler/nir` | 86 | 37,732 | 规范中端 IR、类型内核 (`TypeCore`/`TypeRef`)、布局计算 (`layout.rs`)、枚举编码 (`enum_encoding.rs`)、NF 头 (`normal_forms.rs`)、内在函数表 (`intrinsics.rs`)、匹配计划 (`match_plan.rs`) |
| `compiler/iir` | 19 | 8,797 | 单态实例化闭图 IR (`lib.rs`)、行为认证载荷 (`behavior.rs`)、Direct 效应契约 (`effects.rs`)、清理计划 IR (`cleanup.rs`)、阶段冻结 (`staging.rs`)、闭合存储校验 (`storage.rs`) |
| `compiler/materialization` | 16 | 9,141 | 闭图需求追踪与燃料受限实例化 (`lib.rs`, `staged.rs`, `publication.rs`, `recursive_factories.rs`) |
| `compiler/bitvec_opt` | 8 | 9,508 | `BitVec<N>` 零宽消除 (`zero_width.rs`)、16 轮代数重写 (`rewrites.rs`)、内置 CDCL SAT 等价验证与 CEGIS (`smt.rs`, `synth.rs`)、`BaselineProgram` 降码 (`baseline.rs`) |
| `compiler/pattern_automata` | 7 | 7,341 | Maranget 决策 DAG 编译 (`compile.rs`, `decision.rs`)、位段拆分 (`bitvec.rs`)、模式描述与参考求值 (`pattern.rs`, `eval.rs`) |
| `compiler/intrinsic` | 3 | 584 | 语言项注册 (`lang_item.rs`) 与 Sysroot 包发现 (`sysroot.rs`) |
| `compiler/luna/src/host` | 32 | 22,269 | LLVM 23 文本 IR 降码主机 (`llvm.rs` 及 20 余子模块、`platform.ll` 233L、`task_context.ll` 31L、`tools.rs`) |
| `compiler/luna/src/cleanup` | 2 | 2,365 | 源码控制流清理计划与方法契约重定位 (`cleanup.rs`, `cleanup/method.rs`) |
| `compiler/luna/src/protocols` | 4 | 1,533 | 驱动语言项绑定 (`Copy`/`Drop`/`Splice`/`TypeConstructors`) |
| `compiler/luna/src/resources` | 1 | 335 | 显式所有权义务报告与 Copy/Drop 协议分析 (`resources.rs`) |
| **合计（非测试生产代码）** | **178** | **99,605** | 另有 `compiler/luna/tests/` 等大量端到端测试文件（见各章节证据引用） |

### 1.2 权威标尺与核心 / 愿望单区分

严格遵循 `AGENTS.md` 权威层级，**完全排除** `proposal_trace/`、`OLD-2024/`、`prompt.fl` 与 `flurry_tutorial/`：
1. **核心规范标尺（Core Norm Target）**：
   - [`AGENTS.md`](file:///home/fin/luna/AGENTS.md) §5（IR S-expression `Display` 闭合性、`Tuple`/`meta.List<Type>` 禁止特判、`Once` 统一仿射基线、无 `Option`/`Result`、无 comptime 指针、`BitVec` `Unknown` 降级必须保留原输入求值与资源契约、错误类型为 `PascalCaseErr` + `PascalCase` 变体、点号路径、6 内置包与单 `luna` CLI）。
   - 现行 `docs/` 规范：[`docs/compilation/iir-and-codegen.md`](file:///home/fin/luna/docs/compilation/iir-and-codegen.md)、[`docs/algebraic-effect-and-delimited-continuation/effect-abi.md`](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/effect-abi.md)、[`docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md`](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md)、[`docs/type/user-defined-types/{structs,enums,unions}.md`](file:///home/fin/luna/docs/type/user-defined-types/structs.md)、[`docs/program-synthesis/bitvec-algebra.md`](file:///home/fin/luna/docs/program-synthesis/bitvec-algebra.md)、[`docs/pipeline-projection/{index,backends-and-artifacts,declarations-and-interfaces,intrinsics,target-validation-and-assembly}.md`](file:///home/fin/luna/docs/pipeline-projection/index.md)、[`docs/comptime/{official-attribute-keys,static-shaped-tensors}.md`](file:///home/fin/luna/docs/comptime/official-attribute-keys.md)、以及标明当前仍为空文件（0 字节）的核心文档占位：[`docs/others/{ffi,arbitary-sized-integers}.md`](file:///home/fin/luna/docs/others/ffi.md)、[`docs/program-synthesis/{combinator-algebra,matching-automata,tensor-algebra}.md`](file:///home/fin/luna/docs/program-synthesis/tensor-algebra.md)。
2. **扩展愿望单标尺（Wishlist Extension Target）**：
   - [`tmp/silver-design/04-compiler-wishlist.md`](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md)（P0-1..P2-3）与 [`tmp/wishlist/{06,08,09}-*.md`](file:///home/fin/luna/tmp/wishlist/06-attrs-build-pipeline-projection.md)。这些条目作为**扩展交付标尺**，在矩阵中与核心 `docs/` 缺口显式区分。

> [!IMPORTANT]
> **对 `ai_dev_logs/` 过时快照的实码纠偏（8 项核心现状已在 `HEAD` 彻底改写）**  
> 核对 [`ai_dev_logs/runtime_abi_inventory_2026-11-21.md`](file:///home/fin/luna/ai_dev_logs/runtime_abi_inventory_2026-11-21.md) 与 [`ai_dev_logs/purity_to_llvm_optimization_plan_2026-11-21.md`](file:///home/fin/luna/ai_dev_logs/purity_to_llvm_optimization_plan_2026-11-21.md) 与当前 `HEAD` (`b9a52238`)，以下 8 项原日志结论**已过期失实**，严禁沿用：
> 1. **TLS 全局失败态已彻底删除**：`@flurry_failed` 与 `@flurry_effect_control` 在全仓库已清零；已被 `ControlWord` `{ T, ptr }`（[`control.rs:8-10`](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs#L8-L10)）、[`task_context.ll`](file:///home/fin/luna/compiler/luna/src/host/llvm/task_context.ll)（`%flurry.task = type { i8, ptr }`）及认证纯函数的 `_scalar` 快路径（[`control.rs:12-21`](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs#L12-L21)）替代。
> 2. **`^attrs.packed.bit` 已端到端实现**：`PackedMode::Bit`、[`nir/src/layout.rs:1200-1203`](file:///home/fin/luna/compiler/nir/src/layout.rs#L1200-L1203)、`Ty::BitPackedRecord`、RMW 写回与 `bitvec_opt_bridge` 已落地并通过 [`packed_structs.rs`](file:///home/fin/luna/compiler/luna/tests/packed_structs.rs)。
> 3. **`iir::Package::match_plans` 已被 LLVM 后端真实消费**：[`pattern_dag.rs:80-112`](file:///home/fin/luna/compiler/luna/src/host/llvm/pattern_dag.rs#L80-L112) 已接入决策 DAG（支持 `Bool`/`Optional`/`Error`/`Enum`/`Int`/`Char`/`ListLen`/`BitSlice`）并通过 [`pattern_dag_llvm.rs`](file:///home/fin/luna/compiler/luna/tests/pattern_dag_llvm.rs)。
> 4. **`bitvec_opt` 已建成并接线**：`compiler/bitvec_opt` 与 [`bitvec_opt_bridge.rs:719-806`](file:///home/fin/luna/compiler/luna/src/host/llvm/bitvec_opt_bridge.rs#L719-L806) 实现重写 + SAT 证明 + `Unknown` 回退 `BaselineProgram`，并通过 [`bitvec_opt_llvm.rs`](file:///home/fin/luna/compiler/luna/tests/bitvec_opt_llvm.rs)。
> 5. **枚举与错误和类型已使用重叠联合与标签优化**：`TaggedUnion` 与 `ErrorResult` 已共享联合字节阵布局（[`enums.rs:27-35`](file:///home/fin/luna/compiler/luna/src/host/llvm/enums.rs#L27-L35), [`errors.rs`](file:///home/fin/luna/compiler/luna/src/host/llvm/errors.rs)），`EnumRepr` 已支持 `Unit` 窄标签（`i0`/`i8`/`i16`/`i32`）、`SingleVariant` 拆封、`Niche` 编码，以及 `variant_set_condition` + `merge_contiguous_ranges` 区间分组条件发射（[`enums.rs:829-1041`](file:///home/fin/luna/compiler/luna/src/host/llvm/enums.rs#L829-L1041)）。
> 6. **LLVM 23 纯度属性发射已完整落地**：[`iir/src/behavior.rs`](file:///home/fin/luna/compiler/iir/src/behavior.rs)、[`program_verification/src/behavior.rs:165`](file:///home/fin/luna/compiler/program_verification/src/behavior.rs#L165)、[`host/llvm/attributes.rs:48-123`](file:///home/fin/luna/compiler/luna/src/host/llvm/attributes.rs#L48-L123) 与 [`host/llvm.rs:3542-3644`](file:///home/fin/luna/compiler/luna/src/host/llvm.rs#L3542-L3644) 已发射 `nounwind`/`willreturn`/`nosync`/`nofree`/`memory(...)`/`speculatable`/指针参数属性（含 `captures(none)`）及间接调用点属性，并通过 [`llvm_behavior_attributes.rs`](file:///home/fin/luna/compiler/luna/tests/llvm_behavior_attributes.rs)。
> 7. **`^attrs.abi(.c)` 标量/指针 C 导出跳板已落地**：[`control.rs:166-275`](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs#L166-L275) 已生成带 `zeroext`/`signext` 与 `%flurry.task` 适配器并测试于 [`host_direct_abi.rs:589`](file:///home/fin/luna/compiler/luna/tests/host_direct_abi.rs#L589)（仍缺聚合体分类与外部 FFI 导入）。
> 8. **`static` 全局定义已实现**：`nir::Static`、[`ExprKind::StaticAddress`](file:///home/fin/luna/compiler/nir/src/program.rs#L1528) 与 [`pointers.rs:121-159`](file:///home/fin/luna/compiler/luna/src/host/llvm/pointers.rs#L121-L159) 已接入并测试于 [`static_def.rs`](file:///home/fin/luna/compiler/luna/tests/static_def.rs)（但存在实码验证与 `Display` 漏洞）。

---

## 2. 成熟度总览与统计表

### 2.1 特性成熟度分布总表（对应 §3 `GAP-06-01`..`GAP-06-28`）

| 标尺类别 | `🟢 Full` | `🟡 Partial` | `🟠 Frontend-Only` | `🔴 Missing` | `⚠️ Divergent` | `⚪ Deferred` | 项数合计 |
|---|---:|---:|---:|---:|---:|---:|---:|
| **核心规范（Core `docs/` / `AGENTS.md`）** | 5 | 8 | 0 | 7 | 3 | 0 | **23** |
| **扩展愿望单（Extended `tmp/wishlist/` / `04`）** | 0 | 0 | 0 | 5 | 0 | 0 | **5** |
| **总计（All `GAP-06-NN`）** | **5 (17.9%)** | **8 (28.6%)** | **0 (0.0%)** | **12 (42.9%)** | **3 (10.7%)** | **0 (0.0%)** | **28** |

> [!NOTE]
> 上表严格互斥拆分 **23 项核心规范** 与 **5 项扩展愿望单**：5 项归入扩展愿望单的条目为 `GAP-06-08`（`^attrs.layout.align`，`Silver 04` P1-5 / `wishlist/06`）、`GAP-06-15`（`Tensor` 代数融合，`wishlist/09`，核心 `docs/program-synthesis/tensor-algebra.md` 目前为 0 字节占位，伴随 `static-shaped-tensors.md`）、`GAP-06-16`（`Combinator` 融合，`wishlist/09`，`combinator-algebra.md` 为 0 字节占位）、`GAP-06-23`（`std.sync.atomic`，`wishlist/08`）与 `GAP-06-24`（便携 SIMD，`Silver 04` P2-1）。核心规范 23 项中已有 `13` 项达到 `🟢`/`🟡`，其余 `7` 项核心 `🔴 Missing` 聚焦于捕获式续延/栈池/`async`/`panic`/`union`/`^attrs.ffi`、编译器侧通用管线投影胶水（赋能 Tier-2/3 后端）以及目标指令内在封装。

### 2.2 `SND / CMP / ROB` 专项严重度统计表（对应 §6）

| 审计维度 | Critical | High | Medium | Low | 小计 |
|---|---:|---:|---:|---:|---:|
| **健全性（`SND-06-NN`）** | 0 | 2 | 1 | 0 | **3** |
| **完备性（`CMP-06-NN`）** | 0 | 3 | 2 | 0 | **5** |
| **鲁棒性（`ROB-06-NN`）** | 0 | 1 | 2 | 1 | **4** |
| **总计** | **0** | **6** | **5** | **1** | **12** |

### 2.3 非测试源码 Panic 表面与非确定性容器定量普查（静态扫描全范围 `178` 文件 / `99,605` 行）

| Crate / 子模块 | 文件 | 源码行 | `.unwrap()` | `.expect()` | `panic!()` | `unreachable!()` | `todo!/unimpl` | `HashMap` | `HashSet` | `TODO/FIXME` |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `compiler/nir/src` | 86 | 37,732 | 14 | 3 | 0 | 19 | 0 | 10 | 12 | 0 |
| `compiler/iir/src` | 19 | 8,797 | 13 | 0 | 0 | 31 | 0 | 21 | 21 | 0 |
| `compiler/materialization/src` | 16 | 9,141 | 5 | 5 | 0 | 11 | 0 | 23 | 18 | 0 |
| `compiler/bitvec_opt/src` | 8 | 9,508 | 0 | 17 | 0 | 4 | 0 | 0 | 0 | 0 |
| `compiler/pattern_automata/src` | 7 | 7,341 | 0 | 30 | 10 | 19 | 0 | 0 | 0 | 0 |
| `compiler/intrinsic/src` | 3 | 584 | 0 | 1 | 0 | 0 | 0 | 0 | 0 | 0 |
| `compiler/luna/src/host` | 32 | 22,269 | 19 | 4 | 0 | 26 | 0 | 3 | 0 | 0 |
| `compiler/luna/src/cleanup` | 2 | 2,365 | 3 | 0 | 0 | 0 | 0 | 10 | 13 | 0 |
| `compiler/luna/src/protocols` | 4 | 1,533 | 2 | 0 | 0 | 0 | 0 | 9 | 2 | 0 |
| `compiler/luna/src/resources` | 1 | 335 | 0 | 1 | 0 | 0 | 0 | 0 | 0 | 0 |
| **总计** | **178** | **99,605** | **56** | **61** | **10** | **110** | **0** | **76** | **66** | **0** |

---

## 3. 全量特性差距矩阵（`GAP-06-01` .. `GAP-06-28`）

| 编号 | 领域与特性 | 规范分类与来源 | 成熟度 | 优先级 | 现存实码证据（文件:行号 & 符号 / 测试） | 差距与限制摘要 |
|---|---|---|---|---|---|---|
| `GAP-06-01` | NIR / IIR 结构保留与运行时依赖类型内核 (`TypeCore`/`TypeRef`) | Core: `docs/compilation/iir-and-codegen.md` | 🟡 Partial | P1 | `compiler/nir/src/types.rs:1836-2050`; `compiler/iir/src/lib.rs:213-240`; `runtime_dependent_types.rs` | IIR 非 SSA 结构化表示、hole 抽象与闭合存储校验已成型；仅缺 `static` 依赖闭包验证及部分控制转移边界 |
| `GAP-06-02` | IR Lisp S-expression `Display` 完备性 | Core: `AGENTS.md` §5 | ⚠️ Divergent | P1 | `nir/src/program.rs:2272-2277,3973-4090`; `iir/src/lib.rs:961-1036`; `bitvec_opt/src/sexpr.rs` | `nir::Static` 未实现 `Display`；`nir::Package::fmt`（已打印 `formed_nullary_nominals`/`builtin_implementation_packages`/`tests`）与 `iir::Package::fmt` 均彻底遗漏 `.statics` |
| `GAP-06-03` | `static` 全局定义全链实例化、校验与降码 | Core: `AGENTS.md` §5, `docs/compilation/iir-and-codegen.md` | 🟡 Partial | P1 | `nir/src/program.rs:1528,2272-2340`; `materialization/src/lib.rs:622,858`; `pointers.rs:121-159`; `static_def.rs` | 读写降码已通；但 `materialization` 仅浅克隆 `statics`（未计 fuel、不入队其依赖），且 `iir::Package::validate`（`384-698`）零校验 `statics` |
| `GAP-06-04` | `Tuple` / `meta.List<Type>` 禁止特殊规则 | Core: `AGENTS.md` §5 | ⚠️ Divergent | P1 | `nir/src/normal_forms.rs:77-132` (`BuiltinCallable`, `Role`); `intrinsic/src/lang_item.rs:47` (`LangItem::Tuple`) | NIR 与 LangItem 层仍专设 `Role::{Tuple, TypeList}` 与 `BuiltinCallable::{List, Tuple}`，尚未统一至纯普通 comptime 值与签名绑定机制 |
| `GAP-06-05` | 6 个内置包 Sysroot 固定身份与文档规范一致性 | Core: `AGENTS.md` §1, §5 | ⚠️ Divergent | P1 | `compiler/intrinsic/src/sysroot.rs:10-46` (`BUILTIN=0, STD=1, USER_START=2`) | Sysroot 发现仅认识 2 个内置包（缺 `meta`/`build`/`attrs`/`verification`），且源码文档 L13 仍残留 `defines Option, Result` |
| `GAP-06-06` | `Once` 仿射基线与无 `Option`/`Result` 协议收敛 | Core: `AGENTS.md` §5 | 🟡 Partial | P2 | `compiler/intrinsic/src/lang_item.rs:4,52-66`; `host/llvm/optional.rs`, `errors.rs` | 运行时 `?T`/`!E T` 与协议绑定已正交化，不再导出 `Option`/`Result`；`lang_item.rs:4` 注记滞后，且需全内置包由 `Once` 泛化替代旧假设 |
| `GAP-06-07` | 结构体布局：`PackedMode::{None, Byte, Bit}` | Core: `docs/type/user-defined-types/structs.md` | 🟢 Full | P2 | `nir/src/layout.rs:290-319,1200-1203`; `host/llvm.rs:129-138`; `packed_structs.rs` | `Byte` align=1 与 `Bit` LSB-first POD 位域、RMW 与 >256b 告警均已端到端接通并在 `O0`/`O3` 执行 |
| `GAP-06-08` | `^attrs.layout.align(bytes)` 显式类型对齐 | Wishlist: `Silver 04` P1-5, `tmp/wishlist/06-*` | 🔴 Missing | P1 | `nir/src/layout.rs`; `host/llvm.rs` (`Slot.align`) | `Slot.align` 仅区分 `None` 与打包 `Some(1)`；无自定义字节对齐属性键、布局验证或 LLVM `align N` 传播 |
| `GAP-06-09` | 枚举紧凑布局：单变体、Niche、重叠联合与区间分组切换 | Core: `docs/type/user-defined-types/enums.md` | 🟢 Full | P2 | `host/llvm/enums.rs:4-256,829-1041`; `nir/src/enum_encoding.rs:287`; `encoded_enums.rs`, `enum_groups.rs` | 支持 `Unit` 窄标签/`SingleVariant`/`Niche`/`FixedBits`/`TaggedUnion` 及 `merge_contiguous_ranges` 区间判断 |
| `GAP-06-10` | `union` 原生类型、C ABI 布局与免旧成员 `Drop` | Core: `docs/type/user-defined-types/unions.md` | 🔴 Missing | P1 | `nir/src/types.rs:146-150` (`NominalKind` 无 `Union`); `nir/src/layout.rs:290` | 仅内部 `ErrorResult`/`Enum` 借用 `union_layout`；无用户 `union` 声明、无 `Ty::Union` C 尺寸/对齐降码与字段覆写免析构规则 |
| `GAP-06-11` | `^attrs.abi(.c)` 导出跳板与目标 C ABI 聚合体分类 | Core: `docs/comptime/official-attribute-keys.md`, `docs/others/ffi.md` (stub) | 🟡 Partial | P1 | `nir/src/program/definition_attributes.rs:25-39`; `host/llvm/control.rs:166-275`; `host_direct_abi.rs:589` | 仅导出闭合纯标量/指针/`void`/`Never` `.c` 函数；拒绝聚合体（无 SysV/AAPCS64 分类）、`i128`/`f16`/`bf16`、闭环境及其他 calling conv |
| `GAP-06-12` | `^attrs.ffi.*` 无体外部 FFI 声明与 C 函数指针调用 | Core: `docs/others/ffi.md` (stub); Wishlist: `06` | 🔴 Missing | P1 | `host/llvm/control.rs`; `host/llvm/platform.ll` (仅内部硬编码 libc 声明) | 尚无外部链接库/符号属性、无体函数 `declare` 发射及跨 C-ABI 间接调用 |
| `GAP-06-13` | `bitvec_opt` 代数重写、SAT 等价证明与 `Unknown` 安全回退 | Core: `docs/program-synthesis/bitvec-algebra.md`, `AGENTS.md` §5 | 🟢 Full | P2 | `compiler/bitvec_opt/src/{ir,rewrites,smt,synth,baseline,zero_width}.rs`; `bitvec_opt_bridge.rs:719-806`; `bitvec_opt_llvm.rs` | 零宽消除、16 轮重写、`BehaviorBoundary` 守卫、`Unknown(Budget)` 回退 `BaselineProgram` 均已闭环验证 |
| `GAP-06-14` | `pattern_automata` 决策 DAG 消费 (`match_plans`) | Core: `docs/program-synthesis/matching-automata.md` (stub) | 🟡 Partial | P2 | `host/llvm/pattern_dag.rs:80-131,1964-2050`; `patterns.rs`; `pattern_dag_llvm.rs` | 主干离散与位切片/列表长度开关已接入 DAG；但含 arm `guard`、`Pattern::Then` cleanup、非 `list_views` 动态视图仍回退线性匹配 |
| `GAP-06-15` | 静态形状 `Tensor` 编译期代数融合与向量/循环降码 | Wishlist: `tmp/wishlist/09-*` (核心参考 `docs/comptime/static-shaped-tensors.md`; `tensor-algebra.md` 为空) | 🔴 Missing | P1 | `compiler/{nir,iir,luna/src/host}`（全仓未出现任何 `Tensor` IR 节点） | 规范 `tensor-algebra.md` 为空文件；中端与 LLVM 后端均无收缩/算子融合表达或向量循环发射 |
| `GAP-06-16` | `Combinator` 组合子代数融合与零分配降码 | Wishlist: `tmp/wishlist/09-*` (`docs/program-synthesis/combinator-algebra.md` 为空) | 🔴 Missing | P2 | `compiler/{nir,iir,luna/src/host}`（全仓无组合子融合管线） | `combinator-algebra.md` 为空文件；无高阶流/迭代组合子融合成闭环 IR Pass |
| `GAP-06-17` | `Direct` / `DirectPassing` 效应 ABI、`ControlWord` 与 `TaskContext` | Core: `docs/algebraic-effect-and-delimited-continuation/effect-abi.md` | 🟢 Full | P2 | `host/llvm/control.rs:8-275`; `task_context.ll`; `host/llvm/effects.rs`; `host_direct_abi.rs` | TLS 已清零；`ControlWord` `{ T, ptr }`、`_scalar` 快路径与 `Direct`/`DirectPassing`（限零参无指针数据签名）均已可执行 |
| `GAP-06-18` | 资源清理调度与 `Step::Invoke` 析构（含 allocator `env_arguments`） | Core: `docs/compilation/iir-and-codegen.md` | 🟡 Partial | P2 | `luna/src/cleanup.rs`; `host/llvm/{cleanup.rs:1411-1567,defaults.rs:36,frames.rs:78}`; `drop.rs` | 结构化清理、默认值清理、`Step::Invoke` 带 runtime `env_arguments` 与 `Drop` 二次跳转 trap 已通；但默认值内禁非局部控制返回 |
| `GAP-06-19` | LLVM 23 `BehaviorAttributes`、指针参数属性与调用点传播 | Core: `docs/compilation/iir-and-codegen.md` | 🟢 Full | P2 | `iir/src/behavior.rs`; `program_verification/src/behavior.rs`; `host/llvm/attributes.rs`; `llvm.rs:3542-3644`; `llvm_behavior_attributes.rs` | `memory(...)`/`willreturn`/`nounwind`/`speculatable`/`nosync`/`nofree` 与 `captures(none)` 等全套属性可发射并经 `O3` 校验（Layer-2 存 `SND-06-01` 盲区） |
| `GAP-06-20` | 捕获式 `builtin.Continuation` (`Abi::Capturing`)、栈池与分段栈 | Core: `docs/.../effect-abi.md`; `Silver 04` P0-3 | 🔴 Missing | P0 | `host/llvm/effects.rs:88,438` (`Abi::Capturing` 直接返回错误); `task_context.ll` | 仅 `task_context.ll` 预留 `%pool` 槽；后端完全拒绝 `Abi::Capturing`，无 `StackPool`/`BoundedStackPool` 运行时及上下文切换 |
| `GAP-06-21` | `async` 效应 (`Abi::Async`) 状态机/协程栈与调度器接线 | Core: `docs/.../async-algebraic-effect.md`; Wishlist `08` | 🔴 Missing | P0 | `host/llvm/effects.rs:88,438` (`Abi::Async` 被拒绝); `task_context.ll:1-24` | 仅有取消标志原子轮询原型；无协程帧切分、挂起恢复或任务调度循环 |
| `GAP-06-22` | `std.process.panic` 不可恢复展开/终止 ABI | Core: `docs/.../effect-abi.md`; Wishlist `08` | 🔴 Missing | P1 | `host/llvm/entry.rs`, `host/llvm.rs:1173-1185` (仅内置 `dprintf` + `@llvm.trap()`) | 无标准 `panic` 元数据传播、栈帧清理区分或用户级 panic hook/回溯通道 |
| `GAP-06-23` | `std.sync.atomic` 跨类型原子操作与内存序降码 | Wishlist: `tmp/wishlist/08-effects-async-concurrency-net-runtime.md` | 🔴 Missing | P1 | `task_context.ll:8,14,20` (仅硬编码 `i8` release/acquire); `nir/src/intrinsics.rs` (无原子操作) | 除内部 `task_context.ll` 外，NIR/LLVM 无通用 `load/store/rmw/cmpxchg/fence` 降码 |
| `GAP-06-24` | 便携 SIMD 向量类型与向量内在函数 | Wishlist: `Silver 04` P2-1 | 🔴 Missing | P1 | `nir/src/types.rs:56-87` (`Primitive` 无向量); `host/llvm.rs:113-138` (`Ty` 无 `Vector`) | 无 `<N x T>` LLVM 类型映射、向量算术、广播/混洗或掩码访存 |
| `GAP-06-25` | 任意精度及 `>128`-bit `ArbitrarySizedInteger` 标量/算术支持 | Core: `docs/others/arbitary-sized-integers.md` (stub) | 🟡 Partial | P1 | `nir/src/layout.rs:15` (`MAX_BIT_WIDTH = 65_536`); `host/llvm/decimal.rs:7-23`; `nir/src/types.rs:56-87` | `BitVec<N>` 与位操作/十进制输出已达 65,536b；但 NIR 整数原语与算术仍只限于 `8..=128` 位，缺运行时大整数 / >128b div/mul 运行时库 |
| `GAP-06-26` | `intrinsic` 内在函数家族扩展（溢出算术/数学/参数化 tag） | Core: `docs/pipeline-projection/intrinsics.md`; Wishlist `06` | 🟡 Partial | P1 | `compiler/nir/src/intrinsics.rs:40-118`; `host/llvm.rs:1978-2210`; `platform.rs:11`; `host_intrinsic_protocol.rs` | 已接通 61 个标量/位/Linux x86_64/MemRaw 内在函数（含 `f16`/`bf16` 精确 `fma`）；但禁用参数化 tag family，缺 overflow/widening 算术、`tan`/反三角、跨平台 `Linux*` 解耦 |
| `GAP-06-27` | 编译器侧通用管线投影胶水（`^attrs.compilation.pipeline` / `IIR.access` / `meta.runtime.invoke`）赋能 Tier-2/3 后端 | Core: `docs/pipeline-projection/{index,backends-and-artifacts,declarations-and-interfaces}.md` (Wishlist `06`/`Silver P2-3` 扩展 `^attrs.compilation.project` 与 GPU 生态调用) | 🔴 Missing | P1 | `compiler/luna/src/host/{mod.rs,tools.rs,llvm.rs}` (仅单机 Host LLVM CLI 链路) | 缺失编译器侧通用投影接入：`__flurry_pipeline_projection`/`^attrs.compilation.pipeline` 回调分发、`meta.compilation.IIR.Func` / `IIR.access` 只读 IR 句柄、`meta.runtime.invoke` 动态桥接与产物/诊断交付（注：`CUDA`/`SPIR-V`/`WGSL` 发射器属 Tier-2/3 包职责，不应并入 `compiler/luna`） |
| `GAP-06-28` | 目标特性验证与受检指令封装 (`expr'intrinsic` / `build.target.features`；Wishlist 内联 `asm`) | Core: `docs/pipeline-projection/target-validation-and-assembly.md` (Wishlist `06` 扩展 `^attrs.compilation.target_feature`) | 🔴 Missing | P2 | `nir/src/intrinsics.rs:40-44`; `host/llvm.rs` | 核心规范通过 `build.target.features.get(...)` 与 `expr'intrinsic(args).as(T)`（无独立 asm 字符串语法）封装目标指令，但 `Registry` 禁泛型家族且无目标向量/特化指令检查及 Wishlist 内联 `asm` 支持 |

---

## 4. 关键缺口深度剖析（含源码证据）

### 4.1 效应、续延、栈池与异步调度缺失（`GAP-06-20`, `GAP-06-21`, `GAP-06-22`）

当前 [`host/llvm/effects.rs`](file:///home/fin/luna/compiler/luna/src/host/llvm/effects.rs#L87-L101) 与 [`host/llvm/control.rs`](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs#L12-L77) 已成功构建一套紧凑、无 TLS 污染的直接效应机械模型：
- 函数统一返回值与控制指针构成的双字包 `{ T, ptr }`（[`control.rs:8-10`](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs#L8-L10)），所有携带控制通道的调用末尾传入 `ptr %task`（指向 `%flurry.task = type { i8, ptr }`，见 [`task_context.ll:2`](file:///home/fin/luna/compiler/luna/src/host/llvm/task_context.ll#L2)）。
- 对满足空效应行且经 `BehaviorAttributes` 证明 `no_trap_or_unwind == true` 的函数，后端在内部生成 `@flurry_{idx}_scalar` 裸返回 `T`（[`host/llvm.rs:3508-3524`](file:///home/fin/luna/compiler/luna/src/host/llvm.rs#L3508-L3524)），同时保留统一控制包包装器供间接函数指针调用。
- **核心阻断**：在 `effects.rs:87-92`（局部 handler 安装）与 `effects.rs:417-439`（跨函数效应入口契约绑定）中，后端明确写死仅接受 `Abi::Direct | Abi::DirectPassing`：
  - `Abi::Direct` 将 handler 降码为 `(callee_ptr, env_ptr)` 对，支持通过 `outlined_return` 与 `resource_unwinds` 执行多级返回和结构化清理；
  - `Abi::DirectPassing` 甚至进一步要求 [`iir::effects::data_passing_signature`](file:///home/fin/luna/compiler/iir/src/effects.rs#L30-L42) 返回真，即**强制零参数、结果无指针**的静态数据槽传递；
  - **`Abi::Capturing` 与 `Abi::Async` 直接触发错误**：`"host lexical handlers require direct or direct-passing ABI"` 与 `"host cross-function effect ABI requires direct entries or pointer-free direct passing"`。`task_context.ll` 中的 `%pool` 指针字段仅有读写 getter（`flurry_task_pool`），既没有任何分段栈/协程池分配器实现，也没有 `builtin.Continuation` 帧挂起、迁移、多发或单发恢复代码生成；`std.process.panic` 亦无独立 unwind/abort ABI 区分，只以 `dprintf(2, ...)` + `@llvm.trap()` 收尾（[`host/llvm.rs:1183,1225`](file:///home/fin/luna/compiler/luna/src/host/llvm.rs#L1183)）。

### 4.2 `static` 实例化与校验孤岛、C ABI 与 `union`/对齐缺失（`GAP-06-03`, `GAP-06-08`, `GAP-06-10`, `GAP-06-11`, `GAP-06-12`）

1. **`static` 定义的跨层校验缺口**：[`nir::program::Static`](file:///home/fin/luna/compiler/nir/src/program.rs#L2272-L2277) 在 LLVM 后端已有实打实的发射通路（[`pointers.rs:121-159`](file:///home/fin/luna/compiler/luna/src/host/llvm/pointers.rs#L121-L159) 通过 `static_address` 与 `const_eval` 产出 `@flurry_static_{pkg}_{local} = internal global ...`）。但中端与实例化层只是挂了个便车：
   - [`materialization/src/lib.rs:622`](file:///home/fin/luna/compiler/materialization/src/lib.rs#L622) 与 [`858`](file:///home/fin/luna/compiler/materialization/src/lib.rs#L858) 原样执行 `statics: package.statics.clone()`，既不扣减燃料，也不扫描 `static.ty` 与 `static.initializer` 所引用的名义类型或静态常量依赖入工作队列；
   - [`iir/src/lib.rs:384-698`](file:///home/fin/luna/compiler/iir/src/lib.rs#L384-L698) 的 `Package::validate` 全程未检查 `self.statics`！如果 `static` 引用了一个未被其他根函数提及的 `struct`，或者内部含有未完成的 staged 节点，它会在无校验的前提下直接滑入 LLVM `const_eval` 才报错。
2. **缺失用户 `union` 与自定义对齐 `^attrs.layout.align`**：
   - [`nir/src/types.rs:146-150`](file:///home/fin/luna/compiler/nir/src/types.rs#L146-L150) 中 `NominalKind` 只有 `{ Struct, Enum, Newtype }`；虽然 [`nir/src/layout.rs:290-319`](file:///home/fin/luna/compiler/nir/src/layout.rs#L290-L319) 实现了 `union_layout`，但仅供 `ErrorResult` 与 `EnumAggregate` 内部计算最大尺寸与对齐。用户态 `union`（C ABI 重叠字段、非构造函数上下文要求 `unsafe`、成员覆写时不执行旧值 `Drop`）在 NIR/IIR/Cleanup/LLVM 全线缺席。
   - [`host/llvm.rs`](file:///home/fin/luna/compiler/luna/src/host/llvm.rs) 的 `Slot.align` 仅能表现自然对齐（`None`）或字节打包（`Some(1)`），尚无 `__flurry_align` 或 `^attrs.layout.align(bytes)` 布局元数据消费通道。
3. **C FFI 边界半程状态**：
   - [`control.rs:166-275`](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs#L166-L275) 已经支持对带有 `__flurry_abi = .c`（[`definition_attributes.rs:25-39`](file:///home/fin/luna/compiler/nir/src/program/definition_attributes.rs#L25-L39)）的函数发射外导 C 包装器，还自动补充 `zeroext`/`signext`（i8/i16/bool）、检查符号重名、并用 Unicode 字节转义引号处理导出名。但其 `scalar` 白名单（`200-220`）把**所有结构体、枚举、Tuple、`Optional`、`ErrorResult`、`str`、`BitVec`、`i128`/`u128`/`f16`/`bf16`** 全部拦在门外，没有 SysV AMD64 eightbyte 分类或 AAPCS64 HFA 分类；而且另一半——**导入外部 C 函数声明与调用**（`^attrs.ffi.link`、`^attrs.ffi.link_name`）与 C 函数指针调用同样空白。

### 4.3 内在函数、交叉目标耦合、GPU 投影与内联汇编（`GAP-06-23`, `GAP-06-24`, `GAP-06-25`, `GAP-06-26`, `GAP-06-27`, `GAP-06-28`）

1. **宿主 `cfg!` 锁死目标语义（交叉编译断点）**：
   - [`host/llvm/platform.rs:11-17`](file:///home/fin/luna/compiler/luna/src/host/llvm/platform.rs#L11-L17) 直接以 `cfg!(all(target_os = "linux", target_arch = "x86_64"))`（编译器二进制构建宿主的条件编译宏，而非 `self.graph.target` 或 `Toolchain` 探测出的 target triple）决定是否允许所有 11 个 `Linux*` 系统内在函数；
   - [`host/llvm/bitvec_opt_bridge.rs:486-497`](file:///home/fin/luna/compiler/luna/src/host/llvm/bitvec_opt_bridge.rs#L486-L497) 的 `host_target_profile` 同样用 `cfg!(target_arch = "aarch64")` / `cfg!(target_arch = "x86_64")` 选择 `TargetProfile`，且 `emit_baseline_program`（[`bitvec_opt_bridge.rs:1005-1010`](file:///home/fin/luna/compiler/luna/src/host/llvm/bitvec_opt_bridge.rs#L1005-L1010)）强行要求 `program.word_bits == 64`。如果用同一 `luna` 交叉编译到 `aarch64-linux-gnu` 或 32 位嵌入式目标，将发生策略错配甚至构建失败。
2. **内在函数家族与大整数边界**：[`nir/src/intrinsics.rs:40-44`](file:///home/fin/luna/compiler/nir/src/intrinsics.rs#L40-L44) 的 `Registry` 明确禁止任何带类型参数/要求的 tag family（`"intrinsic tag family instances are not implemented"`）；[`Operation`（`56-118`）](file:///home/fin/luna/compiler/nir/src/intrinsics.rs#L56-L118) 已涵盖 61 项标量数学、`f16`/`bf16` 软件单舍入 `half_fma`（[`half_fma.rs`](file:///home/fin/luna/compiler/luna/src/host/llvm/half_fma.rs)）、位计数、`Linux*` 与 `Mem*`，但缺失：溢出/进位/宽乘（如 `checked_add`、`carrying_add`）、`tan`/`asin`/`acos`/`atan`/`atan2`/双曲函数、`std.sync.atomic` 原子操作、SIMD 向量运算、以及 `>128`-bit `ArbitrarySizedInteger` 标量与算术（当前整数算术只允许 `I8..I128`/`U8..U128`，虽然 `BitVec` 与 `decimal.rs` 支撑至 65,536b）。
3. **通用管线投影接入与目标指令封装空白（不破坏 Tier-1/Tier-2-3 分层）**：核心 [`docs/pipeline-projection/`](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md#L55-L68) 与 [`declarations-and-interfaces.md:155-186`](file:///home/fin/luna/docs/pipeline-projection/declarations-and-interfaces.md#L155-L186) 明确分工：**Tier 2/3 包**（如 `cuda_compile`、`tensor_protocol`、`vulkan_tensor`）实现具体的 `CUDA`/`SPIR-V`/`WGSL` 目标发射器，而不应把 GPU 后端硬塞进 `compiler/luna`；**编译器与宿主层 (`luna`)** 必须提供通用的投影胶水：`^attrs.compilation.pipeline` / `__flurry_pipeline_projection` 属性分发（Wishlist `06` 另拟 `^attrs.compilation.project`）、只读 `meta.compilation.IIR.Func` / `IIR.access` 反射视图、`meta.runtime.invoke` 宿主调用桥与诊断/依赖/产物传递。同时核心 [`target-validation-and-assembly.md:17,57-88`](file:///home/fin/luna/docs/pipeline-projection/target-validation-and-assembly.md#L17-L88) 不引入独立 `asm` 字符串产生式，而是通过 `build.target.features.get(...)` 与泛型 `expr'intrinsic(args).as(T)` 封装硬件指令（`^attrs.compilation.target_feature` 属 Wishlist `06` 扩展）。目前 `compiler/{nir,iir,materialization,luna/src/host}` 中以上投影分发、`IIR.access`、`meta.runtime.invoke` 与目标特化指令校验均未实现，[`host/tools.rs`](file:///home/fin/luna/compiler/luna/src/host/tools.rs#L616-L725) 的工具链仅连接单机 LLVM `llvm-as -> opt -> llc -> ld`。

### 4.4 综合、张量与组合子融合现状（`GAP-06-13`, `GAP-06-14`, `GAP-06-15`, `GAP-06-16`）

- **已建成部分质量较高**：`compiler/bitvec_opt`（8 个模块、9,508 行、非测试代码中 0 `unwrap`/`panic!`）配合 [`bitvec_opt_bridge.rs`](file:///home/fin/luna/compiler/luna/src/host/llvm/bitvec_opt_bridge.rs) 已完整落地零宽消除、16 轮规则重写、`memory_version` 副作用隔离、CDCL SAT 等价性验证及 `Unknown` -> `BaselineProgram` 回退；`compiler/pattern_automata` 已通过 `iir::Package::match_plans` 接入 [`pattern_dag.rs`](file:///home/fin/luna/compiler/luna/src/host/llvm/pattern_dag.rs)，覆盖了离散标签、`BitSlice` 与列表长度视图决策树，以及范围归并条件生成。
- **未建成与空白部分**：`docs/program-synthesis/{tensor-algebra,combinator-algebra}.md` 在正文层级本身即是 **0 字节占位**，静态 `Tensor` 代数与 `Combinator` 流融合既无规范化中端表示，也无循环变换/向量化后端降码；同时 [`pattern_dag.rs:100-105,114-131`](file:///home/fin/luna/compiler/luna/src/host/llvm/pattern_dag.rs#L100-L131) 面对含外层 `guard`、带资源清理顺序要求的 `Pattern::Then`（`cleanup_compatible_with_dag` 返回 `false`）以及 `NominalType`/`TypeTuple`/`NormalForm`/`CodeEvidence` 模式（`1437-1441`）均会退回线性 [`patterns.rs`](file:///home/fin/luna/compiler/luna/src/host/llvm/patterns.rs)。

---

## 5. 架构偏离与技术债清单（`⚠️ Divergent` 专项）

| 编号 | 偏离条目与涉及位置 | 违背的核心规范（`AGENTS.md` §5） | 影响分析与偿还建议 |
|---|---|---|---|
| **DIV-06-01** (`GAP-06-02`) | `nir::Static` 无 `Display`；`nir::Package::fmt` ([`program.rs:3973-4090`](file:///home/fin/luna/compiler/nir/src/program.rs#L3973-L4090)) 与 `iir::Package::fmt` ([`iir/src/lib.rs:961-1036`](file:///home/fin/luna/compiler/iir/src/lib.rs#L961-L1036)) 均彻底遗漏 `.statics` 字段（注：`nir::Package::fmt` 的 `3979-4004` 行已正常输出 `formed_nullary_nominals`、`builtin_implementation_packages` 与 `tests`） | “设计每层 IR 时同步设计并实现 `Display`，dump 统一采用 Lisp 风格 S-expression…子节点关系明确” | 含全局静态变量的程序在 dump NIR / IIR 时静默丢失声明，破坏可观测性与故障复现。补齐 `impl fmt::Display for Static` 并输出至两层 `Package::fmt` 的 `(statics ...)` 节 |
| **DIV-06-02** (`GAP-06-04`) | `nir::normal_forms::{BuiltinCallable::{List, Tuple}, Role::{Opaque, Tuple, TypeList}}` ([`normal_forms.rs:77-132`](file:///home/fin/luna/compiler/nir/src/normal_forms.rs#L77-L132))、`LangItem::Tuple` ([`lang_item.rs:47`](file:///home/fin/luna/compiler/intrinsic/src/lang_item.rs#L47)) 及相关分支 | “不得为 `Tuple`、`meta.List<Type>` 或其他具体库定义设计特异的语言规则…NF 参数按签名接收普通 comptime 值” | 中端类型展开、布局和 NF 应用仍在部分路径根据 `Role` 对 `Tuple`/`meta.List<Type>` 走特殊钩子；应退化为仅凭内部声明 ID 挂接底层内存布局视角而不改变通用参数收集/求值/身份规则 |
| **DIV-06-03** (`GAP-06-05`) | [`compiler/intrinsic/src/sysroot.rs:10-46`](file:///home/fin/luna/compiler/intrinsic/src/sysroot.rs#L10-L46) 固定 `PackageId::{BUILTIN(0), STD(1), USER_START(2)}` 且注释 `L13` 留存 `defines Option, Result` | “除 std 外，builtin、meta、build、attrs、verification 的 prelude 默认导入”；“Flurry 没有 Option 和 Result” | 六大内置包在底层 `intrinsic` Sysroot 边界缺乏一等固定包 ID 区分，长期迫使驱动将部分内置包当作普通包或临时合并；应升级为 6 包固定槽并清理 `Option`/`Result` 文档毒留 |

*(注：除上表 3 项外，[`compiler/luna/src/host/llvm/platform.rs:11`](file:///home/fin/luna/compiler/luna/src/host/llvm/platform.rs#L11) 与 [`bitvec_opt_bridge.rs:490`](file:///home/fin/luna/compiler/luna/src/host/llvm/bitvec_opt_bridge.rs#L490) 中以编译器宿主机 `cfg!(...)` 取代 `graph.target` 的实现，构成了从源码语义层到原生汇编层的潜在目标漂移债，已计入 `SND-06-03` 专项治理。)*

---

## 6. 健全性 · 完备性 · 鲁棒性专项审计（`SND / CMP / ROB`）

### 6.1 健全性审计（`SND-06-01` .. `SND-06-03`）

#### `SND-06-01` [High] Layer-2 `BehaviorAttributes` 机器 IR 消毒器对诊断 I/O、非 `@llvm.trap` 失败出口及外调存在检测盲区
- **源码证据**：[`host/llvm/attributes.rs:48-60`](file:///home/fin/luna/compiler/luna/src/host/llvm/attributes.rs#L48-L60)、[`host/llvm.rs:1157-1228,3470-3477,3542-3602`](file:///home/fin/luna/compiler/luna/src/host/llvm.rs#L1157-L1228)
- **失效机理**：`host/llvm.rs:3542` 注释宣称建立“Layer 2 machine-IR soundness pass”以清理带有 trap 或 volatile 访问的危险 LLVM 属性。但实际实现中：
  1. `attributes::sanitize_for_emitted_body` **仅检查文本子串 `"@llvm.trap"` 与 `" volatile "`**。
  2. 当 `guard_with_diagnostic` 带有 `cleanup = Some(...)` 且处于 `checked && !fast` 模式时（如前后置条件 [`llvm.rs:3472`](file:///home/fin/luna/compiler/luna/src/host/llvm.rs#L3472)、[`1273`](file:///home/fin/luna/compiler/luna/src/host/llvm.rs#L1273)、[`2486`](file:///home/fin/luna/compiler/luna/src/host/llvm.rs#L2486)），失败分支不发射 `@llvm.trap`，而是调用外部 libc **`@dprintf(i32 2, ptr @flurry_predicate_message...)`**（产生实际 fd 2 I/O 副作用）并返回非空控制字 `control_return(control::FAILURE)`。
  3. 同时 `extract_direct_callees`（`llvm.rs:3544-3557`）只抓取本模块内定义的 `@flurry_<ident>(` 函数，忽略 `@flurry_linux_*`（`platform.ll`）、`@flurry_task_*`（`task_context.ll`）、`@dprintf`、`@memcmp` 以及所有间接 `call ... %callee`。
- **实际边界与后果**：当前依靠上游 [`program_verification/src/behavior.rs:637-642,1183-1188,1589-1615`](file:///home/fin/luna/compiler/program_verification/src/behavior.rs#L637-L642) 先行将含前后置条件、`CheckPredicate`、`Linux*` 和未识别节点的实例标为保守状态才避免直接崩溃；但这意味着 **Layer-2 本身并未真正守住机器 IR 边界**。一旦后续引入新后端路径（如通过内部指令注入诊断打印、外调libc或扩展 `StaticAddress` 读写），如果上游遗漏降级，`speculatable` / `memory(none)` 将被 LLVM 23 `-O3` 合法提升或消去，导致静默越界或丢失副作用（UB / 误优化）。

#### `SND-06-02` [High] `materialization` 浅克隆 `statics` 未计费不入队，且 `iir::Package::validate` 漏检 `self.statics`
- **源码证据**：[`compiler/materialization/src/lib.rs:622,858`](file:///home/fin/luna/compiler/materialization/src/lib.rs#L622), [`compiler/iir/src/lib.rs:384-698`](file:///home/fin/luna/compiler/iir/src/lib.rs#L698), [`compiler/luna/src/host/llvm/pointers.rs:121-159`](file:///home/fin/luna/compiler/luna/src/host/llvm/pointers.rs#L121-L159)
- **失效机理**：IIR 的核心健全性契约（[`docs/compilation/iir-and-codegen.md:143-152`](file:///home/fin/luna/docs/compilation/iir-and-codegen.md#L143-L152)）规定：**所有被引用的名义类型、依赖实例与常量初始化式必须完成 comptime 计算并被 IIR `validate()` 验证闭合**。然而目前：
  1. `materialization` 的 `build` 和 `extend` 均直接复制 `statics: package.statics.clone()`，跳过 `static.ty` 与 `static.initializer` 中出现的 `Nominal`、`Function` 或嵌套 `Static` 依赖入队；
  2. `iir::Package::validate`（`384-698`）遍历校验了 `roots`、`nominals`、`interfaces`、`functions`、`effect_contracts`，却**从头到尾未遍历 `self.statics`**！
- **后果**：若 `static S: UnreferencedStruct = ...` 在入口仅被取址或用于间接初始化，或者包含非法阶段表达式，该错误或未实例化类型可直接绕过 IIR 守门员进入 LLVM 降码，导致 `static_address` / `const_eval` 生成失配或在跨包部分快照下出现静默语义错误。

#### `SND-06-03` [Medium] `host_target_profile` 与 `platform_intrinsic` 依赖编译器宿主 `cfg!()`，破坏交叉目标 ABI/指令集健全性
- **源码证据**：[`compiler/luna/src/host/llvm/bitvec_opt_bridge.rs:486-497`](file:///home/fin/luna/compiler/luna/src/host/llvm/bitvec_opt_bridge.rs#L486-L497), [`compiler/luna/src/host/llvm/platform.rs:11-17`](file:///home/fin/luna/compiler/luna/src/host/llvm/platform.rs#L11-L17)
- **失效机理**：`bitvec_opt_bridge` 根据编译期宿主条件 `cfg!(target_arch = "aarch64")` / `cfg!(target_arch = "x86_64")` 选择 `TargetProfile` 并据此作为 `SynthesisCache` key（`synth.rs:1315`）；`platform_intrinsic` 以 `cfg!(all(target_os = "linux", target_arch = "x86_64"))` 控制允许条件。两者均未校验 `Toolchain::target_layout().triple`，使得在 x86_64 宿主构建针对非 x86_64 目标的 BitVec 综合缓存或 Linux errno 常量（`platform.ll` 内 `-22`/`-36`/`-14` 等）产生目标体系错配风险。

### 6.2 完备性审计（`CMP-06-01` .. `CMP-06-05`）

| 编号 | 严重度 | 完备性缺口 | 证据与缺失边界 |
|---|---|---|---|
| `CMP-06-01` | High | `ExprKind`、类型表示与目标字宽覆盖缺口 | [`host/llvm.rs:2995`](file:///home/fin/luna/compiler/luna/src/host/llvm.rs#L2995) (`inner_unimplemented`)；`Ty` 无 `Union`/`Vector`；[`bitvec_opt_bridge.rs:1005`](file:///home/fin/luna/compiler/luna/src/host/llvm/bitvec_opt_bridge.rs#L1005) 强限 `BaselineProgram` 仅支持 `word_bits == 64`（32 位目标失败）；NIR 缺 `>128b` 整数标量运算与 `Abi::Capturing`/`Async` 降码 |
| `CMP-06-02` | High | `MatchPlan` 决策 DAG 降码静默回退路径较多 | [`pattern_dag.rs:80-131,1437-1441`](file:///home/fin/luna/compiler/luna/src/host/llvm/pattern_dag.rs#L80-L131) 遇任意带 `guard` 的 arm、存在 `Pattern::Then` cleanup 序、`NominalType`/`TypeTuple`/`NormalForm`/`CodeEvidence` 或非 `list_views` 动态切片时即返回 `Ok(None)` 退回线性匹配 [`patterns.rs`](file:///home/fin/luna/compiler/luna/src/host/llvm/patterns.rs) |
| `CMP-06-03` | High | C ABI 仅覆盖纯标量导出、无聚合体分类且无 FFI 导入 | [`control.rs:166-275`](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs#L166-L275) 拒绝对所有聚合体（struct/enum/tuple/Optional/ErrorResult/str）、`i128`/`f16`/`bf16` 及闭环境函数生成 `.c` 入口，缺少 SysV/AAPCS64 寄存器/SRET 规则；`^attrs.ffi.*` 导入完全缺失 |
| `CMP-06-04` | Medium | `DirectPassing` 受限零参无指针数据且默认值域禁非局部跳转 | [`iir/src/effects.rs:30-42`](file:///home/fin/luna/compiler/iir/src/effects.rs#L30-L42) 与 [`effects.rs:93-101,430-438`](file:///home/fin/luna/compiler/luna/src/host/llvm/effects.rs#L93-L101) 禁止带参数或含指针的 `DirectPassing` handler；[`defaults.rs:35-37`](file:///home/fin/luna/compiler/luna/src/host/llvm/defaults.rs#L35-L37) 与 [`frames.rs:70-79`](file:///home/fin/luna/compiler/luna/src/host/llvm/frames.rs#L70-L79) 拒绝默认实参求值跨越到外层 handler 返回 |
| `CMP-06-05` | Medium | `Operation` 内在家族未泛化且运算/目标矩阵偏窄 | [`nir/src/intrinsics.rs:40-44`](file:///home/fin/luna/compiler/nir/src/intrinsics.rs#L40-L44) 禁止泛型/参数化 tag family；缺少溢出/带进位/双倍宽算术、反三角/双曲/`tan`、`std.sync.atomic`、SIMD、目标特化指令校验及 Wishlist 内联 `asm`；`platform_intrinsic` 硬锁 x86_64 Linux |

### 6.3 鲁棒性审计（`ROB-06-01` .. `ROB-06-04`）

- **全范围定量表现肯定项**：在 178 个非测试文件（99,605 行）中，**`todo!()`、`unimplemented!()` 与 `TODO`/`FIXME` 注释计数全部为 0**；`compiler/bitvec_opt` 在 9,508 行中保持 **`unwrap=0`、`panic!=0`、`HashMap=0`、`HashSet=0`**；`compiler/luna/src/host/llvm/` 对预算 (`fuel`)、递归深度 (`depth >= 128`)、类型展开节点预算 (`layout_budget_tests`, `llvm.rs:4160-4290`) 与子进程超时/重入（`tools.rs`）均有显式防御。以下为 4 项具体鲁棒性风险：

| 编号 | 严重度 | 鲁棒性问题与定位 | 影响与缓解方向 |
|---|---|---|---|
| `ROB-06-01` | High | `compiler/pattern_automata` 存在 **10 处非测试 `panic!()`** 与 **30 处 `.expect()`** | [`pattern_automata/src/eval.rs:465-562`](file:///home/fin/luna/compiler/pattern_automata/src/eval.rs#L465-L562) 含 9 处直接 `panic!()`（`Invalid place projection` / `Switch` 类型不匹配）；[`pattern.rs:147`](file:///home/fin/luna/compiler/pattern_automata/src/pattern.rs#L147) 对超过 128 位 `FiniteInt` 直接 `panic!()`。参考求值器与宽整数转换应统一返回 `Result<_, EvalError>`，杜绝畸形 IR 触发宿主编译器进程崩溃 |
| `ROB-06-02` | Medium | 生产代码含 **56 处 `.unwrap()`**、**61 处 `.expect()`**、**110 处 `unreachable!()`**，集中在阶段求值与 LLVM 降码 | 关键热点：<br>• [`iir/src/staging.rs:1148,1230,1234-1236,2842-2859`](file:///home/fin/luna/compiler/iir/src/staging.rs#L1148) 占 12 处 `.unwrap()`，重度耦合前置类型/二元算符假设；<br>• [`host/llvm/`](file:///home/fin/luna/compiler/luna/src/host/llvm.rs) 占 19 处 `.unwrap()` / 4 处 `.expect()`，如 [`effects.rs:67`](file:///home/fin/luna/compiler/luna/src/host/llvm/effects.rs#L67) (`effect_resume.as_mut().unwrap()`)、[`pattern_dag.rs:1498,1602,1742,2578,2799`](file:///home/fin/luna/compiler/luna/src/host/llvm/pattern_dag.rs#L1498)、[`bits.rs:20,116`](file:///home/fin/luna/compiler/luna/src/host/llvm/bits.rs#L20)、[`control.rs:236`](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs#L236)；<br>• [`luna/src/resources.rs:160`](file:///home/fin/luna/compiler/luna/src/resources.rs#L160) (`.expect`) 与 [`cleanup.rs:513,1616,1986`](file:///home/fin/luna/compiler/luna/src/cleanup.rs#L513) (`.unwrap`)。<br>应逐步以带上下文 `.ok_or_else(...)` 错误传递取代，防止 IIR 校验略过的边缘构造引爆 panic |
| `ROB-06-03` | Medium | 中端与调度层共 **76 处 `HashMap` 与 66 处 `HashSet`**，带来确定性回归与增量缓存风险 | 相比 [`host/llvm/strings.rs:9,27`](file:///home/fin/luna/compiler/luna/src/host/llvm/strings.rs#L9) 显式防泄漏、`bitvec_opt`/`pattern_automata` 全用 `BTreeMap`/`BTreeSet`，[`materialization/src/lib.rs:118-138,476-492`](file:///home/fin/luna/compiler/materialization/src/lib.rs#L118-L138)（23 `HashMap` / 18 `HashSet`）、[`iir/src/{index,lib,type_links,storage}.rs`](file:///home/fin/luna/compiler/iir/src/lib.rs#L387)（21 `HashMap` / 21 `HashSet`）与 [`cleanup.rs:262-652`](file:///home/fin/luna/compiler/luna/src/cleanup.rs#L262)（10 `HashMap` / 13 `HashSet`）大量用无序哈希表驱动去重与依赖收集；任何未排好序的导出或迭代将直接破坏跨进程构建 IR 一致性 |
| `ROB-06-04` | Low | 子进程工具链依赖、IR 体积放大与预算兜底细节 | [`host/tools.rs:616-768`](file:///home/fin/luna/compiler/luna/src/host/tools.rs#L616-L768) 依赖临时盘文件与外部 `llvm-as`/`opt`/`llc`/linker 进程通信，缺少目标无关的内存/对象缓存层及跨目标 `llc -march` 明确参数传递；此外 `EffectHandle` 展开与 `decimal.rs` 对超宽整数虽设了 fuel 防护，但应在交叉目标集成前补全内存增长边界用例 |

---

## 7. 修复路径、依赖与验收建议

按长期架构优先级（P0 阻断 -> P1 核心完善 -> P2 生态/多管线扩展）排定演进里程碑，不以局部补丁掩盖通用能力缺口：

### P0 阶段：运行时效应续延、异步调度与 Soundness 防线加固
1. **加固 Layer-2 与 `static` 闭合验证（消除 `SND-06-01`, `SND-06-02`）**：
   - 在 [`host/llvm/attributes.rs:48`](file:///home/fin/luna/compiler/luna/src/host/llvm/attributes.rs#L48) 与 [`host/llvm.rs:3542`](file:///home/fin/luna/compiler/luna/src/host/llvm.rs#L3542) 中扩展外调扫描（识别 `@dprintf`、`@flurry_linux_*`、`@flurry_task_*`、非空 `ControlWord` 返回路径及所有间接调用 `call ptr %...`），对未白名单外调强制剥离 `memory(none)`/`speculatable`/`nounwind`。
   - 将 `statics` 纳入 [`materialization`](file:///home/fin/luna/compiler/materialization/src/lib.rs) 工作队列与燃料扣减，并在 [`iir::Package::validate`](file:///home/fin/luna/compiler/iir/src/lib.rs#L384) 严格验证 `self.statics` 类型、阶段封闭性与初始化式引用。
2. **补全 `Abi::Capturing` (`builtin.Continuation`)、栈池与 `Abi::Async` 运行时后端（解决 `GAP-06-20`, `GAP-06-21`, `GAP-06-22`）**：
   - 前置依赖前端/Elaboration 交付已检查的捕获框架与效应行展开节点（对应 `Silver 04` P0-3）。
   - 在 `host/llvm` 设计统一轻量栈帧切换与 `StackPool` / `BoundedStackPool`（`meta.runtime.bounded_stack_bytes`）运行时（兼容 `x86_64` 与 `aarch64`），同时接入 `Abi::Async` 挂起状态机/调度循环及 `std.process.panic` 终止/清理语义（区分常规错误返回与不可捕获终止）。
   - **验收标准**：能原生编译运行捕获式 `Continuation`（单发/多发、跨 C 帧 barrier trap）、带 `BoundedStackPool` 限界的递归/效应程序及 `async` 任务调度用例。

### P1 阶段：系统级完备性（`union`、C ABI、FFI、原子、SIMD、大整数、Sysroot 与 S-expr 合规）
1. **修补 S-expr、6-Sysroot、`Tuple`/`List` 特判与 `pattern_automata` 鲁棒性（解决 `GAP-06-02`, `GAP-06-04`, `GAP-06-05`, `ROB-06-01`, `ROB-06-03`）**：
   - 为 `nir::Static` 实现 Lisp S-expr `Display` 并接入 `nir::Package::fmt` 与 `iir::Package::fmt`；
   - 将 [`intrinsic/src/sysroot.rs`](file:///home/fin/luna/compiler/intrinsic/src/sysroot.rs) 扩展至 6 个固定 builtin 包 ID，并清除残余 `Option`/`Result` 文档及 `Role::{Tuple, TypeList}` 特设路径；
   - 替换 `pattern_automata/src/eval.rs` 中的 10 处 `panic!()` 为结构化 `Err`，并将关键输出相关 `HashMap`/`HashSet` 替换为确定性 `BTreeMap`/`BTreeSet`。
2. **原生 `union`、`^attrs.layout.align`、完整 C FFI 与跨平台目标解耦（解决 `GAP-06-08`, `GAP-06-10`, `GAP-06-11`, `GAP-06-12`, `SND-06-03`, `CMP-06-03`）**：
   - 引入 `NominalKind::Union` 与 `Ty::Union` C 布局、覆写免旧值 `Drop` 规则，以及任意 2 的幂对齐属性降码；
   - 实现 SysV AMD64 / AAPCS64 C 聚合体分类器，打通双向 `^attrs.abi(.c)`（含结构体参数/返回、`.sysv64`/`.aapcs`/`.win64`）与 `^attrs.ffi.{link, link_name}` 无体外部 `declare` 发射；
   - 移除 `platform.rs` 与 `bitvec_opt_bridge.rs` 中的编译宿主机 `cfg!(...)` 绑定，完全按 `graph.target` / `TargetLayout::triple` 驱动，同时开放 32 位 `BaselineProgram` 字长泛化。
3. **`std.sync.atomic`、SIMD、溢出/数学内在与 `>128`-bit `ArbitrarySizedInteger`（解决 `GAP-06-23`, `GAP-06-24`, `GAP-06-25`, `GAP-06-26`）**：
   - 为 `nir::intrinsics::Registry` 开放泛型参数化家族，补齐溢出/进位算术、`tan`/反三角、LLVM `<N x T>` SIMD 与 `cmpxchg`/`atomicrmw`/`fence` 原子操作、及 `ArbitrarySizedInteger` 大整数运行库降码。

### P2 阶段：程序综合扩展、多管线 GPU 投影与内联汇编
1. **完成 `DirectPassing` 带参/指针扩展与 `MatchPlan` 全模式闭环（解决 `CMP-06-02`, `CMP-06-04`, `GAP-06-18`）**：解除 `DirectPassing` 零参无指针约束、允许默认值期内非局部控制转移并接通含 guard/`Pattern::Then` cleanup 序的决策 DAG。
2. **实现编译器侧通用管线投影接入（赋能 Tier-2/3 GPU 后端）与目标指令封装（解决 `GAP-06-27`, `GAP-06-28`）**：在 `compiler/luna` 与 `iir`/`nir` 落地 `__flurry_pipeline_projection` / `^attrs.compilation.pipeline`（及 Wishlist `^attrs.compilation.project`）协议识别、目标依赖闭包校验、只读 `meta.compilation.IIR.Func` / `IIR.access` 视图、`meta.runtime.invoke` 宿主桥接与产物/诊断交付，从而支撑独立 Tier-2/3 包实现 `CUDA`/`SPIR-V`/`WGSL` 编译；同时开放 `expr'intrinsic` 目标特化向量指令校验（及 Wishlist `target_feature`/内联 `asm`）。
3. **补完 `docs/program-synthesis/{tensor-algebra,combinator-algebra}.md` 并落地静态 `Tensor` / `Combinator` 融合管线（解决 `GAP-06-15`, `GAP-06-16`）**：建立与 `bitvec_opt`、`pattern_automata` 同等级的形式等价/边界保全约束与自动化差分验证套件。
