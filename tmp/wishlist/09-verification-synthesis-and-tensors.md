# WL-09: 程序验证（`verification`）、纯度体系与四类程序合成代数完整 Wish List 规格

> **文档角色**：本文档为 Flurry 六内置包中 `verification` 程序验证层、P0–P5 运行时纯度与内存效果抽象，以及**四类程序合成与编译期代数（`BitVec` 位向量代数、静态形状阶多态 `Tensor` 代数、零开销组合子融合代数、匹配自动机与编译期正则合成）**的长期通用架构规格与开发路线图。
> **遵循原则**：严格以作者确认、现行正式 `docs/`、`AGENTS.md` 与近 6 个月（`2026-5` 至 `2026-10`）有效 `proposal_trace/` 为参考并向正式文档收口；严守**三层反膨胀边界**（`regex` 与具体 BLAS/GPU/ML 张量后端绝不进入 `builtin`/`std`/`verification` 等 6 个内置包），严守 **PTC 与 `verification` 正交分离原则**（程序状态/内存区域/资源数量绝不混入 PTC 类型命题）。

---

## 1. 范围、权威来源与归属包

### 1.1 范围与跨模块边界

本规格覆盖五大核心子域：

1. **`verification` 内置包与程序验证底座**：
   - 幽灵/逻辑规格类型（`SpecInt`、`SpecNat`、`SpecSeq`、`SpecSet`、`SpecMap`、`Ghost`、`Snapshot`、`Perm`、`PointsTo`、`MemRegion`、`MemoryEffect`）；
   - 规格视角与证明构造（`'old`、`'snapshot`、`predicate`、`spec fn`、`lemma`、`axiom`、`invariant`、`requires`、`ensures`、`decreases`、`outcomes`、`assumes`、`asserts`）；
   - 四级安全边界协同（`safe` / `spec` / `verified` / `unsafe`）、`P0`–`P5` 运行时纯度分级（结合 `compiler/program_verification/src/purity.rs` 当前实现缺口）、以及六种内存效果摘要（`read`、`write`、`kill`、`connect`、`move`、`system`）。
2. **`BitVec` 代数扩展与程序合成**：
   - `BitVec<n>` 拼接/切片/位写回语义、`^attrs.packed.bit`（`__flurry_packed_mode: .bit`）与 `bitvec_opt` 等价重写规则，以及 `Unknown` 回退路径对求值顺序、仿射移动和 `Drop` 契约的严格保全。
3. **静态形状阶多态 `Tensor` 代数（严格限制在合成层抽象）**：
   - 编译期 `Shape` / `broadcastable` / `broadcast_shape` 与普通依赖 `Tensor<T, ...dims>` 阶段分层、`einsum`/矩阵乘法/转置/降阶视图切片（`RangeOrIndex`）与核融合投影（经 `^attrs.compilation.pipeline` 对接 Tier 2 `tensor_protocol`）。
4. **组合子代数（零开销流与解析组合子融合）**：
   - 基于 `inline fn` + 推送式代数效应 `#emit_item<T>`（`builtin.iterator`）与 CMTT（`meta.code` + `meta.patterns`）的零分配组合子融合，在不发明特殊编译器硬编码的前提下消除中间状态并保持严格控制流与清理次序。
5. **匹配自动机与编译期 Regex DFA/NFA 合成（严格桥接 Tier 2/3）**：
   - `pattern_automata` 决策 DAG 合成（面向 `StableListPatternView`、`StableMapPatternView`、`StableConcatPatternView`、`BitVec` 与 `Stable` `case`），以及借助 CMTT `code_match`/`code_sequence` 将编译期 DFA/NFA 状态机合成降码至 Tier 2 `regex_protocol` 与 Tier 3 `regex_dfa` 生态包。

与其余 Wish List 文档的协同边界如下：

- **WL-02（模式匹配与 Take View）**：拥有 `StableListPatternView`、`StableMapPatternView`、`StableConcatPatternView`、`TuplePatternView`、`MatchPreparation`、`code_match`、`code_sequence` 以及运行时/契约 `'old`/`'bitvec` Take View；WL-09 从合成与验证角度定义其 `DecisionDag` 自动机生成规则、逻辑 `'snapshot` 视角与 `BitVec` 优化底座。
- **WL-04（`builtin` 基础类型与数学）**：拥有 `BitVec<n>`、`Array<T, n>`、`Tuple`、`Slice<T>`、`MaybeUninit`、`Once`/`Clone`/`Copy`/`Drop` 与 `builtin.iterator.emit_item` 基础原语；WL-09 在此之上构建 `bitvec_opt` 重写器、静态张量类型算子示范及 `emit_item` 流融合代数。
- **WL-05（`meta` CMTT 与反射）**：拥有编译器类型命题 PTC（`Type.predicates.*` A01–A19）、`meta.code`、以及 `meta.compilation.IIR` 管线编译桥梁；WL-09 严禁把程序状态/内存契约放入 `Type.predicates.*`。
- **WL-06（`attrs` 属性规范）**：拥有官方 `__flurry_*` 键注册；WL-09 提出 `^attrs.purity.*` 与 `^attrs.verification.*` 官方键需求（以及沿用 `^attrs.packed.bit` 和 `^attrs.compilation.pipeline`）。
- **WL-10（官方薄协议包与反膨胀边界）**：拥有 Tier 2 的 `tensor_protocol`（`TensorStorage`、`TensorOp`、`Differentiable`、`GpuProjection`、`TensorErr`）与 `regex_protocol`（`PatternCompile`、`RegexMatchOps`、`CaptureLayout`、`RegexErr`）；WL-09 的张量与正则合成能力通过这两个 Tier 2 薄协议无缝对接 Tier 3 生态实现，**绝不把具体 CUDA/cuBLAS 驱动或 `regex` 状态机放入 Tier 1 六内置包**。

### 1.2 权威语义来源

- 协作、三层与语义不可破坏边界：`AGENTS.md`
- 安全等级与形式化路线：`docs/program-verification/safety.md`
- 契约、内存效果与运行时纯度：`docs/resource-management/contracts-and-verification.md`、`docs/resource-management/memory-effects.md`、`docs/resource-management/runtime-purity.md`
- `BitVec` 代数与程序合成：`docs/program-synthesis/bitvec-algebra.md`
- 静态形状张量案例：`docs/comptime/static-shaped-tensors.md`
- 纯性、范式不展开与 PTC 隔离：`docs/comptime/purity-and-normal-form.md`、`docs/type/propositional-constraints/atoms.md`
- Pipeline 投影与后端契约：`docs/pipeline-projection/index.md`、`docs/pipeline-projection/backends-and-artifacts.md`、`docs/pipeline-projection/intrinsics.md`
- 双阶段 Trait 与官方属性规范：`docs/comptime/the-comptime-phases-of-traits.md`、`docs/comptime/official-attribute-keys.md`
- 仓库实现基准：`library/verification/README.md`、`library/verification/lib.fl`、`ai_dev_logs/purity_system_status_report_2026-11-21.md`
- 近 6 个月有效草案（`2026-5` 至 `2026-10`，仅作设计演进背景并已严格按正式 `docs/` 现代化）：
  `proposal_trace/2026-7-24-verification.fl`、`2026-8-28-purity-at-runtime.fl`、`2026-8-12-memory-effects.fl`、`2026-9-16-memory-effects.fl`、`2026-8-5-tensors.fl`、`2026-8-13-tensor-view.fl`、`2026-8-27-tensor-rank-polymorphism.fl`、`2026-9-7-tensor-algebra.fl`、`2026-6-10-better-combinators.fl`、`2026-8-28-better-combinators!.fl`、`2026-9-19-regex.fl`、`2026-9-19-regex-history.md`。

### 1.3 归属包分层布局

| 层级 / 包 | 模块路径 | 归属内容 | 反膨胀与架构边界 |
|---|---|---|---|
| **Tier 1：`verification`** | `verification.{spec, ghost, memory, contracts, prelude}`（`lib.fl` 默认空 prelude 逐步按阶开放；除显式导入外不自动污染普通用户作用域） | `SpecInt`、`SpecNat`、`SpecSeq`、`SpecSet`、`SpecMap`、`Ghost`、`Snapshot`、`Perm`、`PointsTo`、`MemRegion`、`MemoryEffect`、区域谓词、`verified`/`spec` 证明契约与 `decreases` 良基度量 | **仅承载形式化逻辑与内存安全规格**；编译后全部幽灵状态抹除；**严禁向 PTC 提交任何状态命题** |
| **Tier 1：`builtin` / `meta` / 编译器合成核心** | `builtin.protocols.iterator`、`meta.synthesis`（或编译器 `program_verification` / `bitvec_opt` / `pattern_automata` Pass） | `P0`–`P5` 纯度与 6 内存效果推理、`BitVec` 宽度代数与 `bitvec_opt`、CMTT `inline` 效应流融合、静态形状计算示例与 `DecisionDag` 自动机生成器 | 不内置领域库（无 `cuda`、无 BLAS、无 `regex`）；通用合成引擎以语言级机制开放给上层包 |
| **Tier 1：`attrs`** | `attrs.purity`、`attrs.verification`、`attrs.packed`、`attrs.compilation` | `__flurry_purity_at_least`、`__flurry_purity_skip_unsafe_propagation`、`__flurry_verification_trusted_axiom`、`__flurry_packed_mode`、`__flurry_pipeline` | 所有官方属性均包裹 `__flurry_*` 内部键 |
| **Tier 2 / Tier 3（六内置包之外）** | `tensor_protocol`（`10`）、`regex_protocol`（`10`）；`cuda_compile`、`cpu_blas`、`regex_dfa` | 动态多维张量存储与 GPU 投影实现、DFA/NFA 正则编译器及特定执行后端 | 消费 Tier 1 合成原语（`BitVec`、`meta.code`、`pipeline-projection`、`TuplePatternView`） |

---

## 2. 现状与缺口全景总表

| ID | 符号 / 特性 | 归属包 / 编译器位置 | 阶段 | 当前现状（基于仓库代码与状态报告核实） | 优先级 | 核心编译器依赖 |
|---|---|---|---|---|---|---|
| `WL-09-01` | `verification` 逻辑数学幽灵类型：`SpecInt`、`SpecNat`、`SpecSeq`、`SpecSet`、`SpecMap`、`Ghost`、`Snapshot` | `verification.spec` / `verification.ghost` | Spec（编译期擦除） | `library/verification/lib.fl` 仅为占位空 prelude（`README.md` 标明暂缓高级库） | P1 | `spec` 语境求值/擦除检查、禁止幽灵数据流向可观察 runtime 输出 |
| `WL-09-02` | 分离与权限类型：`Perm`、`PointsTo`、`MemRegion`、`MemoryEffect`（六大效果 + 6 源根） | `verification.memory` | Static Analysis / Spec | 编译器 `program_verification/src/{memory,moves,effect_transfer}.rs` 已实现局部指针来源与借用分析底座，但公开库类型与区域代数 API 未定义 | P1 | OSL/RL 符号区域与世代模型、非溢出资源责任检查 |
| `WL-09-03` | `predicate` / `spec fn` / `lemma` / `axiom` / `invariant` / `decreases` 与 `'snapshot` 逻辑视角 | `verification.contracts` + 编译器前端 | Spec / Verified | `program_verification/src/contracts.rs` 与 `purity.rs` 支持简单 `predicate`/`requires`/`ensures`（P3+终止）；`lemma`/`axiom`/`decreases`/`invariant`/`'snapshot` 未接通 | P1 | `decreases` 良基度量与无环验证、`'snapshot` 仅限验证层禁用在普通 runtime |
| `WL-09-04` | 条件 `outcomes` 摘要与分支/概率扩展（`outcomes case:name cond => ...`） | `verification.contracts` + `program_verification` | Static Analysis / Contract | 现行仅内部效果推理；语法层 `outcomes` 条件分支、与 `distribution` 概率提示未统一接入与校验 | P2 | `effects` 摘要上界校验器（入口快照条件求值 + 未知合流） |
| `WL-09-05` | `safe`（$\le 3$ 次算术与 `assumes` 运行时检查器）vs `verified`（OSL/RL/Matching Logic）与 `unsafe` 调用交接 | `compiler/program_verification` | RT + Verified | `flow.rs` 已覆盖常规路径与简单边界检查，但未建立严格三次算术多项式求值器预算与 `assumes` 未证残余条件转可控 checker 的完整闭环 | P1 | 有限三次算术求值器、checker 契约一致性证明、`verified` 后端证明调度器 |
| `WL-09-06` | 运行时纯度 `P0`–`P5` 完整分级导出与已知缺口修复（结合 2026-11-21 调研报告） | `compiler/program_verification/src/purity.rs` + `attrs.purity` | Static Analysis -> IIR/LLVM | **仅 `P4` 与 `P3（+终止）` 已落地**；缺通用 `P0`/`P1`/`P2`/`P5` 向 IIR/LLVM 属性导出、缺 `P3` 区间/别名范围分析、缺 `static` 全局变量接入及裸环境 `comptime t: Type` bug 修复 | P0 | `purity.rs` 输出 `P0`–`P5` + memory-effect 摘要至 IIR、`^attrs.purity.*` 键注册 |
| `WL-09-07` | 显式 `move` vs `kill` 仿射语义统一（修复早先非 `Copy` 默认 `kill` 误区） | `compiler/program_verification/src/{moves,drop}.rs` | Static Analysis | 按 `2026-9-16-memory-effects.fl:554` 修正为“移动转移责任而不默认 kill 旧分配”，须在闭环迁移与自引用搬迁检查中严格兑现 | P0 | `moves.rs` 与 `drop.rs` 分离描述符移出与实际堆释放动作 |
| `WL-09-08` | `BitVec<n>` 合成代数重写优化（`bitvec_opt`：切片融合、常量折叠、`++` 零宽消除） | `compiler/nir` / `bitvec_opt` | Compile-time Opt / IR | `BitVec<n>` 构造/切片/位写回已支持；通用 `bitvec_opt` 等价代换优化器未覆盖完整片段融合规则 | P1 | 位向量符号区间分析与跨边界 slice 归约 |
| `WL-09-09` | `^attrs.packed.bit`（`__flurry_packed_mode: .bit`）位级紧凑布局到 `BitVec<W>` 翻译与 `Unknown` 基准保全 | `compiler/elaboration` + `attrs.layout` | Layout -> IR | `attrs.packed.bit` 属性与 >256 位警告已说明；到 `BitVec<W>` 全字段存取生成及 `Unknown` 回退求值/资源守恒检查须全面闭环 | P1 | `layout` -> `BitVec` 读写投射、`Unknown` 回退求值顺序/Drop 测试 |
| `WL-09-10` | 静态形状阶多态 `Tensor` 核心编译期原型：`broadcastable`、`broadcast_shape` 与 `Tensor<T, ...dims>` | 教学原型 / 用户示例（经 `pipeline-projection` 对接 Tier 2） | CT Shape + RT Elem | 仅在 `docs/comptime/static-shaped-tensors.md` 有算法描述；`Array<T, n>` 已存在，通用变参列表值收集与多维降阶投影（无 `undefined` 占位）待补齐 | P2 | `...dims: meta.List<Integer>` 变参绑定、降阶切片视图、`MaybeUninit` 初始化安全 |
| `WL-09-11` | 编译期收缩与张量算子：`matmul`、`einsum`、`transpose`、`reshape`、`TensorView` 与 `VariadicCollector` | 教学原型 / Tier 2 `tensor_protocol` 对接示例 | CT + RT | 尚无库实现；`VariadicCollector`（WL-03）与多维索引 `SpaceApply` 尚未在编译器落地 | P2 | `VariadicCollector`（WL-03）、`SpaceApply`、`MaybeUninit` 安全写入 |
| `WL-09-12` | 张量核融合与 `^attrs.compilation.pipeline` GPU 投影桥接（严禁在 6 内置包放具体驱动） | Tier 1 `meta.compilation.IIR` <-> Tier 2 `tensor_protocol` | Synthesis / Pipeline | `docs/pipeline-projection/` 定义了 `IIR.Func` 与 `meta.runtime.invoke` 契约，但 IIR 遍历融合 pass 与 `tensor_protocol` 尚未接通 | P3 | `meta.compilation.IIR` 只读访问器（WL-05）、`meta.runtime.invoke` |
| `WL-09-13` | 推送式 `inline` 效应流组合子融合（`Iterator` + `#emit_item<T>` 消去中间分配与结构） | `builtin.protocols.iterator` + `compiler/elaboration` / LLVM | Inline -> Direct Passing | `VecIter` 等已使用 `emit_item#`；跨长链 `.map().filter().zip().collect()` 自动去森林化与 `Once`/早期退出清理融合仍待完善 | P1 | `inline fn` + direct-passing `#emit_item` handler 内联、循环控制流与资源清理合并 |
| `WL-09-14` | CMTT 分阶段结构化组合子（`< T -\| Ψ >` 模式 + `< Ψ \|- U >` 配对过滤映射与解析组合子融合） | `meta.patterns` / `meta.code` <-> Tier 2 `parser_protocol` | CT-driven RT | 单洞 expr/pattern quote 已部分通；双向配对 `< T -\| Ψ >` + `< Ψ \|- U >`（`patterns-and-arms.md`）与 `code_sequence` 仍待接通 | P1 | `meta.patterns` 配对上下文校验器（WL-02 `WL-02-13`/`21`）、`inline` 状态机展平 |
| `WL-09-15` | `pattern_automata` 通用决策 DAG 合成（拓展至 `StableMap`/`StableConcat`/`Stable` `case`/`BitVec` 前缀 Trie） | `compiler/pattern_automata` + `compiler/nir/src/match_plan.rs` | Elaboration -> NIR -> LLVM | 当前仅对有限标量/枚举/`StableListPatternView` 生成 `DecisionDag`（`LenInterval` + 单次缓存）；`Map`/`Concat`/`case` 尚走线性回退 | P1 | 跨视图统一观察因子抽象（`ObservationKey`）与带概率分布加权分支排序 |
| `WL-09-16` | 编译期 Regex DFA/NFA 合成范式（基于 CMTT `code_match`/`code_sequence` 桥接 Tier 2 `regex_protocol` 与 Tier 3 `regex_dfa`） | 合成示例 -> Tier 2 `regex_protocol`（**严禁放入 `builtin`/`std`**） | CT DFA Gen + RT | `02` 已验证 Tier 2/3 `TuplePatternView` 消费模式；基于 `code_match` 生成无分配跳转表/字节类表自动机尚待 `WL-02-13` 与 `WL-10-21` 闭合 | P2 | `code_match`/`code_sequence`（WL-02）、`PatternCompile`（WL-10）、`Splice` 常量切片支持 |

---

## 3. 详细规格设计

### 3.1 `verification` 包幽灵/规格类型、契约、安全性分级与 P0–P5 纯度体系

#### 3.1.1 严守架构红线：PTC 与 `verification` 的绝对正交性

依据 [atoms.md](file:///home/fin/luna/docs/type/propositional-constraints/atoms.md)、[memory-effects.md](file:///home/fin/luna/docs/resource-management/memory-effects.md#L53) 及 `05-meta-cmtt-macros-reflection.md`：

1. **PTC（编译期命题类型约束系统）只证明与类型、normal-form 头身份、字段存在性和静态可计算域相关的命题**（A01–A19）。
2. **`verification` 层只负责程序运行时/动态语义下的内存位置身份、分配世代、别名/分离性、仿射线性资源剩余计数、执行终止性（`decreases`）、运行时纯度（P0–P5）和内存效果上界（`outcomes`）**。
3. **禁止跨层混淆**：
   - 即使函数签名写出了 `&R T` 可达限定类型或 `requires` / `ensures` / `outcomes` 子句，这些区域有效性谓词与内存状态事实也**绝不进入 PTC 原子命题或 PTC 理论求解器**；
   - 反过来，程序验证层的 `predicate` / `lemma` 不得伪造正常类型相等证明（例如宣称 `lemma id_eq where ensures Identity<i32> == i32` 从而打破 normal-form 头不展开规则）。

#### 3.1.2 `verification` 包完整核心接口设计（零占位实现，编译期完全擦除）

> 说明：以下 `verification.*` 类型全部属于**仅在逻辑/规格语境存在的幽灵与数学对象**，不参与运行时内存表示或 C ABI 传递；在编译期通过规范化校验后于后端代码生成阶段彻底擦除（零运行时内存与指令成本）。声明严格遵循 Flurry 语法（类型形参置于 `where` 子句、`Tuple<...>` 表示元组类型、`:-` 表示 trait 约束、所有 `assoc` 显式标注 `: Type`）。

```flurry
-- 模块：verification.spec
use bindings;
use builtin.{Type, bool, u64, usize}

-- 数学任意精度整数（逻辑域 ℤ，仅存在于 spec / predicate / verified 证明与幽灵字段）
pub struct SpecInt {
    raw: bindings.SpecIntHandle,
}

impl SpecInt {
    pub spec fn zero() -> SpecInt {
        bindings.spec_int_from_literal(0)
    }

    pub spec fn from_usize(value: usize) -> SpecInt {
        bindings.spec_int_from_usize(value)
    }

    pub spec fn add(self, rhs: SpecInt) -> SpecInt {
        bindings.spec_int_add(self, rhs)
    }

    pub spec fn sub(self, rhs: SpecInt) -> SpecInt {
        bindings.spec_int_sub(self, rhs)
    }

    pub spec fn mul(self, rhs: SpecInt) -> SpecInt {
        bindings.spec_int_mul(self, rhs)
    }

    pub spec fn le(self, rhs: SpecInt) -> bool {
        bindings.spec_int_le(self, rhs)
    }

    pub spec fn lt(self, rhs: SpecInt) -> bool {
        bindings.spec_int_lt(self, rhs)
    }
}

-- 数学自然数（逻辑域 ℕ，维护 value >= 0 不变量，常用于 `decreases` 良基度量）
pub newtype SpecNat = SpecInt;

impl SpecNat {
    pub spec fn zero() -> SpecNat {
        bindings.spec_nat_zero()
    }

    pub spec fn from_usize(value: usize) -> SpecNat {
        bindings.spec_nat_from_usize(value)
    }

    pub spec fn pred_or_zero(self) -> SpecNat {
        bindings.spec_nat_sub_saturating(self, 1)
    }
}

-- 数学不可变有限序列 SpecSeq<T>
pub struct SpecSeq
where
    T: Type,
{
    raw: bindings.SpecSeqHandle,
}

impl SpecSeq<T> where T: Type {
    pub spec fn empty() -> SpecSeq<T> {
        bindings.spec_seq_empty(T)
    }

    pub spec fn singleton(value: T) -> SpecSeq<T> {
        bindings.spec_seq_singleton(T, value)
    }

    pub spec fn len(self) -> SpecNat {
        bindings.spec_seq_len(T, self)
    }

    pub spec fn index(self, i: SpecNat) -> T {
        bindings.spec_seq_index(T, self, i)
    }

    pub spec fn concat(self, other: SpecSeq<T>) -> SpecSeq<T> {
        bindings.spec_seq_concat(T, self, other)
    }

    pub spec fn push(self, value: T) -> SpecSeq<T> {
        self.concat(SpecSeq<T>.singleton(value))
    }

    pub spec fn slice(self, start: SpecNat, end: SpecNat) -> SpecSeq<T> {
        bindings.spec_seq_slice(T, self, start, end)
    }

    pub spec fn update(self, i: SpecNat, value: T) -> SpecSeq<T> {
        bindings.spec_seq_update(T, self, i, value)
    }

    pub spec fn contains(self, value: T) -> bool {
        bindings.spec_seq_contains(T, self, value)
    }
}

-- 数学集合 SpecSet<T> 与数学映射 SpecMap<K, V>
pub struct SpecSet
where
    T: Type,
{
    raw: bindings.SpecSetHandle,
}

impl SpecSet<T> where T: Type {
    pub spec fn empty() -> SpecSet<T> {
        bindings.spec_set_empty(T)
    }

    pub spec fn contains(self, elem: T) -> bool {
        bindings.spec_set_contains(T, self, elem)
    }

    pub spec fn insert(self, elem: T) -> SpecSet<T> {
        bindings.spec_set_insert(T, self, elem)
    }

    pub spec fn remove(self, elem: T) -> SpecSet<T> {
        bindings.spec_set_remove(T, self, elem)
    }

    pub spec fn union(self, other: SpecSet<T>) -> SpecSet<T> {
        bindings.spec_set_union(T, self, other)
    }

    pub spec fn intersect(self, other: SpecSet<T>) -> SpecSet<T> {
        bindings.spec_set_intersect(T, self, other)
    }

    pub spec fn diff(self, other: SpecSet<T>) -> SpecSet<T> {
        bindings.spec_set_diff(T, self, other)
    }

    pub spec fn is_subset_of(self, other: SpecSet<T>) -> bool {
        bindings.spec_set_subset(T, self, other)
    }

    pub spec fn len(self) -> SpecNat {
        bindings.spec_set_len(T, self)
    }
}

pub struct SpecMap
where
    K: Type,
    V: Type,
{
    raw: bindings.SpecMapHandle,
}

impl SpecMap<K, V> where K: Type, V: Type {
    pub spec fn empty() -> SpecMap<K, V> {
        bindings.spec_map_empty(K, V)
    }

    pub spec fn lookup(self, key: K) -> ?V {
        bindings.spec_map_lookup(K, V, self, key)
    }

    pub spec fn insert(self, key: K, value: V) -> SpecMap<K, V> {
        bindings.spec_map_insert(K, V, self, key, value)
    }

    pub spec fn remove(self, key: K) -> SpecMap<K, V> {
        bindings.spec_map_remove(K, V, self, key)
    }

    pub spec fn domain(self) -> SpecSet<K> {
        bindings.spec_map_domain(K, V, self)
    }
}
```

```flurry
-- 模块：verification.ghost
use builtin.Type;

{- 幽灵包裹器：零运行时字节大小与零指令开销；严禁向普通 runtime 分支控制流输出其内部状态。 -}
pub struct Ghost
where
    T: Type,
{
    raw: bindings.GhostToken,
}

impl Ghost<T> where T: Type {
    pub spec fn new(value: T) -> Ghost<T> {
        bindings.ghost_pack(T, value)
    }

    pub spec fn view(self) -> T {
        bindings.ghost_view(T, self)
    }

    pub spec fn open(self) -> T {
        bindings.ghost_open(T, self)
    }
}

{- 逻辑状态快照载体（对应程序验证中的状态点捕捉 `'snapshot` 语义实体）。 -}
pub struct Snapshot
where
    T: Type,
{
    raw: bindings.SnapshotToken,
}

impl Snapshot<T> where T: Type {
    pub spec fn capture(value: T) -> Snapshot<T> {
        bindings.snapshot_capture(T, value)
    }

    pub spec fn inner(self) -> T {
        bindings.snapshot_inner(T, self)
    }
}
```

```flurry
-- 模块：verification.memory
use builtin.{Any, Type, bool, u64, usize}
use verification.spec.SpecNat;

-- 区域根分类与区域句柄（对齐 docs/resource-management/memory-effects.md）
pub enum RegionRoot {
    constant_root,
    heap_root,
    local_root,
    global_root,
    tls_root,
    external_root,
}

pub struct MemRegion {
    raw: bindings.RegionHandle,
}

impl MemRegion {
    pub spec fn of_expr(quote target: lift Any) -> MemRegion {
        bindings.region_of_expr(target)
    }

    pub spec fn of_root(root: RegionRoot) -> MemRegion {
        bindings.region_of_root(root)
    }

    pub spec fn root_kind(self) -> RegionRoot {
        bindings.region_root_kind(self)
    }

    pub spec fn union(self, other: MemRegion) -> MemRegion {
        bindings.region_union(self, other)
    }

    pub spec fn is_disjoint(self, other: MemRegion) -> bool {
        bindings.region_disjoint(self, other)
    }

    pub spec fn is_subregion_of(self, outer: MemRegion) -> bool {
        bindings.region_subregion(self, outer)
    }

    pub spec fn is_alive(self) -> bool {
        bindings.region_is_alive(self)
    }
}

-- 内存效果规格构造（非可执行操作；仅在 predicate -> MemoryEffect 与 outcomes 使用）
pub struct MemoryEffect {
    raw: bindings.MemoryEffectHandle,
}

impl MemoryEffect {
    pub spec fn empty() -> MemoryEffect {
        bindings.effect_empty()
    }

    pub spec fn read(region: MemRegion) -> MemoryEffect {
        bindings.effect_read(region)
    }

    pub spec fn write(region: MemRegion) -> MemoryEffect {
        bindings.effect_write(region)
    }

    pub spec fn kill(region: MemRegion) -> MemoryEffect {
        bindings.effect_kill(region)
    }

    pub spec fn connect(source: MemRegion, target: MemRegion) -> MemoryEffect {
        bindings.effect_connect(source, target)
    }

    pub spec fn move_effect(region: MemRegion) -> MemoryEffect {
        bindings.effect_move(region)
    }

    pub spec fn system_io() -> MemoryEffect {
        bindings.effect_system()
    }

    pub spec fn union(self, other: MemoryEffect) -> MemoryEffect {
        bindings.effect_union(self, other)
    }
}

-- 分数/独占分离逻辑权限与 PointsTo 谓词载体（线性验证资产，不允许隐式 Copy）
pub struct Perm {
    raw: bindings.PermHandle,
}

impl Perm {
    pub spec fn split(self, numerator: SpecNat, denominator: SpecNat) -> Tuple<Perm, Perm> {
        bindings.perm_split(self, numerator, denominator)
    }

    pub spec fn merge(left: Perm, right: Perm) -> ?Perm {
        bindings.perm_merge(left, right)
    }

    pub spec fn is_exclusive(self) -> bool {
        bindings.perm_is_exclusive(self)
    }
}

pub struct PointsTo
where
    T: Type,
{
    raw: bindings.PointsToHandle,
}

impl PointsTo<T> where T: Type {
    pub spec fn ptr_address(self) -> u64 {
        bindings.points_to_addr(T, self)
    }

    pub spec fn region(self) -> MemRegion {
        bindings.points_to_region(T, self)
    }

    pub spec fn value(self) -> T {
        bindings.points_to_value(T, self)
    }

    pub spec fn perm(self) -> Perm {
        bindings.points_to_perm(T, self)
    }

    pub spec fn is_mut_valid(self) -> bool {
        self.perm().is_exclusive() and self.region().is_alive()
    }
}
```

#### 3.1.3 安全等级、`predicate`/`spec fn`/`lemma`/`decreases` 与 `'snapshot` 规范

1. **四类关键体与职责界定**：
   - **`spec fn`**：纯数学逻辑函数；只可在 `spec fn`、`predicate`、`lemma`、`invariant`、`requires`/`ensures`/`decreases`、`ghost { ... }` 语境调用；编译到最终可执行码前完全擦除；不得触及 `unsafe` 内存操作。
   - **`predicate`**：返回 `bool` 或 `MemoryEffect` 的程序规格构造。遵循 [runtime-purity.md](file:///home/fin/luna/docs/resource-management/runtime-purity.md#L127-L140)：
     - 若仅用于 `verified` 证明或纯静态分析（如含 `Perm`/`PointsTo`/量词/无界 `SpecSeq`），属于**纯逻辑规格**；
     - 若要在 `safe` 函数的 `assumes pred(...)` 动态检查位执行且未被静态消去，则必须同时满足 **P3 稳定观测 + 有限终止** 与可安全执行的判定体。
   - **`lemma` & `axiom`**：`lemma` 声明全称命题，必须自带 `verified` 证明体（支持归纳法与 `decreases` 度量）才能无条件引入；`axiom` 代表未在编译器内证明的外部公理假定，必须使用 `^attrs.verification.trusted_axiom` 显式审计标注，不得无声混入安全 `safe` 函数契约。
   - **`decreases` & `invariant`**：
     - 递归 `verified fn` / `spec fn` / `lemma` 或带证明义务的循环使用 `decreases expr`（类型为 `SpecNat` 或良基字典序元组）保证有限终止；
     - 循环中使用 `invariant expr` 维持状态不变量。
2. **视角边界：`'old` vs `'snapshot`**：
   - `'old`（已存在于 HIR `ExprKind::TakeOld`）：仅允许出现在函数签名 `ensures` 后置条件中，指代函数入口状态；
   - `'snapshot`：**仅作为 `spec` / `verified` 契约语境内的逻辑状态快照构造**（等价于绑定到 `Snapshot<T>`），**严禁开放为通用 runtime `expr'snapshot` 语法**（参见第 3.8 节 Take View 边界，与 WL-02 `WL-02-26` 一致），避免模糊普通 runtime `Clone` 与验证期逻辑状态冻结的界限。
3. **资源语义纠偏（`move` vs `kill`）**：
   - 按 [2026-9-16-memory-effects.fl:554-579](file:///home/fin/luna/proposal_trace/2026-9-16-memory-effects.fl#L554-L579) 对早期草案的重大纠偏：**非 `Copy` 按值传递 `self` 产生 `move` 转移，不默认产生 `kill`**！仅当值的实际堆/系统资源在析构点或 `Allocator.deallocate` 处释放、终结世代时，才记录 `kill(region)`。

#### 3.1.4 运行时纯度分级（P0–P5）与编译器当前缺口（对齐 2026-11-21 调研报告）

| 纯度层级 | 正式保证 | 对程序合成/优化的授权 | 当前仓库实现状况（`purity.rs`） |
|---|---|---|---|
| **P5（总值纯）** | P4 + 在满足入口 `requires` 下有限完成且无 trap / 挂起 | 循环外提（LICM）、死代码完全消除、任意投机性重排（仅在 `requires` 满足域内） | **未全面导出**：`validate_predicates` 只以简单无递归规则检查终止，缺完整 `decreases` 度量与无 trap 证明导出 |
| **P4（值纯）** | P3 + 结果完全由显式值输入与常量决定，私有 `var`/局部指针不逃逸 | 跨调用 CSE、可安全消除无外部可观察副作用的死纯计算（非终止 `spin()` 不得删除） | **已实质性落地**（`validate_runtime_promises`）：支持私有局部可变性隐藏、高阶已知回调递归闭合、复用 `flow` 静态证明 |
| **P3（稳定观测）** | P2 + 在无干涉写入/终结区间内多次观察等价 | `pattern_automata` 单次观察缓存、区间内重复读取折叠、默认实参稳定性要求 | **已部分落地**（`validate_defaults`/`validate_predicates`）：支持参数化默认值检查；**缺跨可变区间的别名稳定性分析与 `static` 接入** |
| **P2（观测只读）** | P1 + 不修改/终结任何入口可观察状态、不造新资源身份 | 多个只读观察可并行/任意重排 | **未单独导出**：缺一般 P2 IIR 标记导出至后端 |
| **P1（有界变更）** | 所有内存效果收敛在显式已知的 `read`/`write`/`kill`/`move`/`connect` 上界内 | 不交区域（`MemRegion.is_disjoint`）间的写操作可安全调度与重排 | **仅内部分析**：`effect_transfer.rs` 推导部分读写，未形成统一导出的 IIR 摘要与区域分离优化通道 |
| **P0（一般行为）** | 含 `system()` I/O、未知外部调用、易失访问 `^attrs.mem.volatile`、不确定同步 | 严禁删除、严禁投机外提、严格按程序序执行 | **基线保底**（`attrs.mem.volatile` 已要求标为 P0 并禁止 LLVM `memory(none)`/`readonly`） |

> **已知实现 Bug 同步追踪**：依据 `ai_dev_logs/purity_system_status_report_2026-11-21.md:42`，无 sysroot 裸环境下检查含 `comptime t: Type` 形参的默认实参时，`purity.rs` 误将仅在 comptime 存在的 `Type` 算作运行时需要 `Copy` 的值而报 `default expression requires P3`，需在 P0 纯度收口期同步修复。

#### 3.1.5 用法示例与反例（Verification & Purity）

```flurry
use builtin.{Any, Array, Type, u32, usize, void}
use verification.memory.{MemRegion, MemoryEffect, RegionRoot}
use verification.spec.SpecNat;

-- 正例 1：声明 MemoryEffect predicate 配合 `outcomes` 与 `verified` 合约
predicate modify_region(quote target: lift Any) -> MemoryEffect {
    let r = MemRegion.of_expr(target);
    MemoryEffect.read(r).union(MemoryEffect.write(r))
}

verified fn clamp_u32(slot: *u32, max_val: u32) -> old_val: u32
where
    outcomes modify_region(slot),
    requires MemRegion.of_expr(slot).is_alive(),
    ensures old_val == slot'old.* and slot.* <= max_val,
{
    let prev = slot.*;
    if prev > max_val {
        slot.* = max_val;
    }
    prev
}

-- 正例 2：`safe` 函数复用静态三次以内算术契约或 `assumes` 运行时检查器调用 `verified` 函数
verified fn read_checked_elem(arr: Array<u32, n>, idx: usize) -> u32
where
    n: usize,
    requires idx < n,
{
    arr[idx]
}

fn safe_caller(arr: Array<u32, n>, idx: usize) -> ?u32
where
    n: usize,
{
    if idx < n {
        assumes idx < n;
        read_checked_elem(arr, idx)
    } else {
        null
    }
}

-- 负例 1（编译期拒绝）：显式 `outcomes` 声称只读，实际写入目标存储
{-
fn bad_readonly_write(slot: *u32)
where
    outcomes MemoryEffect.read(MemRegion.of_expr(slot)),
{
    slot.* = 42u32; -- 拒绝：write(slot) 超出 outcomes 上界
}
-}

-- 负例 2（编译期拒绝）：违反 PTC 与 verification 分离，试图把内存区域存活状态放进编译期 Type 命题
{-
pure comptime fn IllegalMemoryDependentType<T: Type> -> Type
where
    requires MemRegion.of_root(RegionRoot.heap_root).is_alive(), -- 拒绝：运行时内存/区域状态不得进入编译期 Type / PTC 命题
{
    T
}
-}

-- 负例 3（编译期拒绝）：带 `decreases` 检验的递归 `verified` 函数未严格递减度
{-
verified fn bad_infinite_spec(k: SpecNat) -> SpecNat
where
    decreases k,
{
    bad_infinite_spec(k) -- 拒绝：递归实参 k 不小于入口 k，违反良基递减义务
}
-}
```

---

### 3.2 `BitVec` 代数扩展与程序合成

依据 [bitvec-algebra.md](file:///home/fin/luna/docs/program-synthesis/bitvec-algebra.md) 与 `02-pattern-matching-and-views.md`（§3.7 位段模式契约）：

#### 3.2.1 `BitVec<n>` 代数核心与 `^attrs.packed.bit` 降码

1. **统一类型与位序模型**：
   - `BitVec<n>`（`n: Integer >= 0`）：`bits[lo..hi]` 以 **LSB = bit 0** 索引，拼接 `hi ++ lo` 将 `hi` 置于高位、`lo` 置于低位（与模式语法 `0x seg0 seg1` 左高右低文本顺序严格互为正逆对齐）。
   - **位级紧凑布局属性 `^attrs.packed.bit`**：
     - 产生 `{ __flurry_packed_mode: .bit }`（参见 `library/attrs/README.md:14`）；
     - 总位宽 $W$ 的结构直接降码为底层 `BitVec<W>` 槽位，结构体字段的读写编译为精确的 `place'bitvec[lo..hi]` 切片读取与位置写回（总位宽超过 256 位时产生非致命编译期警告，仍保证语义正确）。
2. **`bitvec_opt` 等价代换重写法则库**：
   程序合成与 IR 优化通道共享如下经过声音性验证（soundness-preserving）的重写法则：

| 规则编号 | 重写前形态 | 重写后形态 | 前提 / 守恒条件 |
|---|---|---|---|
| **BV-R1**（零宽单位元） | `x ++ z` 或 `z ++ x` | `x` | `z: BitVec<0>` 且 `z` 无副作用（或先单独求值 `z` 丢弃） |
| **BV-R2**（切片直接路由） | `(hi ++ lo)[a..b]` | `hi[(a - W_lo)..(b - W_lo)]` 或 `lo[a..b]` | 若切片完全落入左部或右部且两操作数可纯净求值/已绑定局部副本 |
| **BV-R3**（位域连续读融合） | `((w'bitvec[8..16] ++ w'bitvec[0..8]))`（高位 `8..16` 在左、低位 `0..8` 在右的连续拼接） | `(w'bitvec[0..16])` | 两次读取指向同一未修改内存世代（P3 稳定区间）且不跨 `^attrs.mem.volatile`！ |
| **BV-R4**（冗余掩码消除） | `(x and m1) and m2` | `x and (m1 and m2)`；当 `m == all_ones` 时消为 `x` | 同宽度纯位运算 |
| **BV-R5**（位模式匹配树压缩） | 多个 `0b`/`0x` 枚举标签判别臂 | 单次 Masked Equal `(raw and mask) == tag_val` + 分支表 | 来自显式固定位宽编码 `enum` 且无观察副作用 |

#### 3.2.2 关键边界：`Unknown` 回退保全与 `volatile` 隔离

按 `AGENTS.md` §5 明确约束：
- **输入与资源契约保全**：当程序合成或 `bitvec_opt` 在分支宽度未知、别名不确定或分析预算耗尽时返回 `Unknown`，**严禁跳过求值或裁剪看似冗余的读取**！编译器必须保留标准按序求值与仿射/清理基准实现；
- **`^attrs.mem.volatile` 严格免疫**：标记 `^attrs.mem.volatile` 的 MMIO 寄存器或结构体字段（P0）严禁应用 **BV-R2 / BV-R3** 切片重排或读写合并，所有 `load volatile` / `store volatile` 严格按源码词法次序发生。

#### 3.2.3 正反例（`BitVec` 合成与回退）

```flurry
use attrs.packed;
use builtin.{BitVec, u16, u8}

-- 正例：^packed.bit 自动降码为 BitVec<16> 并允许 bitvec_opt 归约无副作用字段打包
^packed.bit
pub struct PacketHeader {
    version: BitVec<4>,
    flags: BitVec<4>,
    payload_len: u8,
}

fn inspect_header(hdr: PacketHeader) -> BitVec<4> {
    -- `^packed.bit` 按 LSB-first 打包：`version` 占 `[0..4]`、`flags` 占 `[4..8]`、`payload_len` 占 `[8..16]`；
    -- `0x` 模式字面从左到右按 MSB -> LSB 对齐位段
    hdr'bitvec match {
        0x (len: BitVec<8>) (flags: BitVec<4>) 4 => flags,
        _ => (0u8'bitvec)[0..4],
    }
}

-- 负例（禁止规则误用）：volatile MMIO 寄存器不可做 BV-R3 两次读取合并
use attrs.mem.volatile;

^volatile
pub struct MmioReg {
    raw: u16,
}

fn read_twice_volatile(reg: *MmioReg) -> BitVec<16> {
    -- 虽然两次读取同一个位置，但 volatile 属 P0：合成优化器严禁将 hi 与 lo 合并为单次 load
    let hi = reg.raw.*'bitvec[8..16];
    let lo = reg.raw.*'bitvec[0..8];
    hi ++ lo
}
```

---
### 3.3 静态形状阶多态 `Tensor` 代数（严格限在合成层抽象）

#### 3.3.1 三层反膨胀与 Normal-Form 定位

1. **严格隔离运行时后端**：依据 `AGENTS.md` 与 `10-official-protocol-packages-and-boundary.md`（§3.13），**任何具体 BLAS/cuBLAS/CUDA/GPU/ML 自动微分运行时驱动均不得进入 `builtin`、`std`、`meta`、`attrs`、`build`、`verification` 六内置包**。
2. **本节定位**：本节规定的是 Flurry 如何借助**编译期值依赖形状（`meta.List<Integer>`）+ normal-form 头 `Tensor<T, ...dims>` + 编译期可执行谓词 + `pipeline-projection`** 表达零阶到高阶静态形状多态代数（延续并现代化 [static-shaped-tensors.md](file:///home/fin/luna/docs/comptime/static-shaped-tensors.md) 与 `2026-8-5/13/27`、`2026-9-7` 草案）。具体多维动态缓冲区与硬件内核派发由 Tier 2 `tensor_protocol`（`TensorStorage`、`TensorOp`、`Differentiable`、`GpuProjection`）与 Tier 3 生态后端承接。
3. **Normal-Form 头不展开边界**：`2026-8-27-tensor-rank-polymorphism.fl:19-21` 曾尝试写 `axiom scalar_is_a_zero_rank_tensor where T { Tensor<T, .dims = []> == T }`。但依据 `AGENTS.md` 与 [purity-and-normal-form.md](file:///home/fin/luna/docs/comptime/purity-and-normal-form.md#L107-L118)，normal-form 头严禁通过函数体或类型别名自动展开为 `T`。因此**零阶 `Tensor<T, .dims = []>` 与标量 `T` 保持明确的名义转换方法**（`into_scalar` / `from_scalar`），仅在特化构造或代数计算层按阶段展开；该边界亦列入第 5 节 Open Question `OQ-09-04` 待作者最终拍板。

#### 3.3.2 完整编译期形状代数（广播、矩阵乘、转置、重塑与张量收缩）

> 说明：完全遵循 [static-shaped-tensors.md](file:///home/fin/luna/docs/comptime/static-shaped-tensors.md#L155-L169) 的索引扫描算法与 `library/meta/data/list.fl` 真实方法签名（`length() -> Integer`、`concat`、`self[i]`），不依赖尚未确认的前置残余 `[...rest, last]` 列表模式写法。

```flurry
-- 模块：tensor_synthesis_core（编译期形状代数规范接口；亦可由 Tier 2/3 张量库直接复用）
use builtin.{Integer, Type, bool}
use meta.List;
use meta.math as comptime_math;

pub enum TensorShapeErr {
    ShapeMismatch,
    NonPositiveDimension,
    InvalidRank,
    InvalidPermutation,
    ReshapeElementCountMismatch,
    ContractionAxisMismatch,
}

pub pure comptime fn all_dims_positive(dims: List<Integer>) -> bool {
    let n = dims.length();
    var i: Integer = 0;
    while i < n {
        if dims[i] <= 0 {
            return false;
        }
        i = i + 1;
    }
    true
}

pub pure comptime fn shape_element_count(dims: List<Integer>) -> Integer {
    let n = dims.length();
    var prod: Integer = 1;
    var i: Integer = 0;
    while i < n {
        prod = prod * dims[i];
        i = i + 1;
    }
    prod
}

-- 从右向左对齐检验广播相容性：相同维度、或任一侧为 1、或一侧缺少前导维度
pub pure comptime fn broadcastable(shape_a: List<Integer>, shape_b: List<Integer>) -> bool {
    if not all_dims_positive(shape_a) or not all_dims_positive(shape_b) {
        return false;
    }
    let la = shape_a.length();
    let lb = shape_b.length();
    let r = if la >= lb { la } else { lb }
    var offset: Integer = 0;
    while offset < r {
        let ia = la - 1 - offset;
        let ib = lb - 1 - offset;
        let da = if ia >= 0 { shape_a[ia] } else { 1 }
        let db = if ib >= 0 { shape_b[ib] } else { 1 }
        if da != db and da != 1 and db != 1 {
            return false;
        }
        offset = offset + 1;
    }
    true
}

pub pure comptime fn broadcast_shape(
    shape_a: List<Integer>,
    shape_b: List<Integer>,
) -> !TensorShapeErr List<Integer> {
    if not broadcastable(shape_a, shape_b) {
        return error TensorShapeErr.ShapeMismatch;
    }
    let la = shape_a.length();
    let lb = shape_b.length();
    let r = if la >= lb { la } else { lb }
    var reversed: List<Integer> = [];
    var offset: Integer = 0;
    while offset < r {
        let ia = la - 1 - offset;
        let ib = lb - 1 - offset;
        let da = if ia >= 0 { shape_a[ia] } else { 1 }
        let db = if ib >= 0 { shape_b[ib] } else { 1 }
        let out_d = if da >= db { da } else { db }
        reversed.append(out_d);
        offset = offset + 1;
    }
    var out: List<Integer> = [];
    var k = reversed.length() - 1;
    while k >= 0 {
        out.append(reversed[k]);
        k = k - 1;
    }
    out
}

pub pure comptime fn transpose_shape(
    dims: List<Integer>,
    perm: List<Integer>,
) -> !TensorShapeErr List<Integer> {
    let rank = dims.length();
    if perm.length() != rank or not all_dims_positive(dims) {
        return error TensorShapeErr.InvalidPermutation;
    }
    -- 校验 perm 是 0..(rank - 1) 的严格双射
    var out: List<Integer> = [];
    var i: Integer = 0;
    while i < rank {
        let axis = perm[i];
        if axis < 0 or axis >= rank {
            return error TensorShapeErr.InvalidPermutation;
        }
        var j: Integer = 0;
        while j < i {
            if perm[j] == axis {
                return error TensorShapeErr.InvalidPermutation;
            }
            j = j + 1;
        }
        out.append(dims[axis]);
        i = i + 1;
    }
    out
}

pub pure comptime fn reshape_compatible(old_dims: List<Integer>, new_dims: List<Integer>) -> bool {
    if not all_dims_positive(old_dims) or not all_dims_positive(new_dims) {
        return false;
    }
    shape_element_count(old_dims) == shape_element_count(new_dims)
}

pub pure comptime fn matmul_shape(
    shape_a: List<Integer>,
    shape_b: List<Integer>,
) -> !TensorShapeErr List<Integer> {
    let la = shape_a.length();
    let lb = shape_b.length();
    if la < 2 or lb < 2 or not all_dims_positive(shape_a) or not all_dims_positive(shape_b) {
        return error TensorShapeErr.InvalidRank;
    }
    let m = shape_a[la - 2];
    let k_a = shape_a[la - 1];
    let k_b = shape_b[lb - 2];
    let n = shape_b[lb - 1];
    if k_a != k_b {
        return error TensorShapeErr.ContractionAxisMismatch;
    }
    let batch_a = shape_a.slice(0, la - 2) match {
        s? => s,
        null => [],
    }
    let batch_b = shape_b.slice(0, lb - 2) match {
        s? => s,
        null => [],
    }
    let batch_out = broadcast_shape(batch_a, batch_b)!;
    batch_out.concat([m, n])
}

pub pure comptime fn contract_axes_shape(
    shape_a: List<Integer>,
    axis_a: Integer,
    shape_b: List<Integer>,
    axis_b: Integer,
) -> !TensorShapeErr List<Integer> {
    let la = shape_a.length();
    let lb = shape_b.length();
    if axis_a < 0 or axis_a >= la or axis_b < 0 or axis_b >= lb {
        return error TensorShapeErr.InvalidRank;
    }
    if shape_a[axis_a] != shape_b[axis_b] {
        return error TensorShapeErr.ContractionAxisMismatch;
    }
    var out: List<Integer> = [];
    var ia: Integer = 0;
    while ia < la {
        if ia != axis_a {
            out.append(shape_a[ia]);
        }
        ia = ia + 1;
    }
    var ib: Integer = 0;
    while ib < lb {
        if ib != axis_b {
            out.append(shape_b[ib]);
        }
        ib = ib + 1;
    }
    out
}
```

#### 3.3.3 静态形状阶多态 `Tensor`、降阶投影与 `TensorView` 切片协议

修正 `proposal_trace/2026-8-27-tensor-rank-polymorphism.fl` 中的两大问题（`add` 返回类型误写为 `shape_a` 与使用未类型安全的裸 `undefined`），改用 Flurry 合法的 `Array` / `MaybeUninit` 缓冲初始化、正常 `where` 子句、`Tuple` 与变参索引衔接：

```flurry
-- 模块：tensor_synthesis_core（延续上节，呈现静态形状加法、TensorView 切片与线性步长计算模型）
use builtin.{Add, Array, Copy, Integer, Type, usize}
use builtin.mem.MaybeUninit;
use meta.{List, to_usize}

-- 扁平连续定长数组作为基石，长度在编译期以 `to_usize(shape_element_count(dims))` 精确计算
pub struct StaticTensor
where
    T: Type,
    dims: List<Integer>,
{
    storage: Array<T, to_usize(shape_element_count(dims))>,
}

-- 运行时非拥有张量切片视图（固定秩 `rank` 直接保存在栈上 Array 中，不引用 comptime-only List）
pub struct StaticTensorView
where
    T: Type,
    dims: List<Integer>,
{
    base_ptr: *T,
    strides: Array<usize, to_usize(dims.length())>,
}

pub enum RangeOrIndex {
    index(usize),
    slice_range(usize, usize),
    full_dim,
}

-- 将编译期形状加法校验置于 instantiation 体内（broadcast_shape 失败触发精准编译期诊断）
pub pure comptime fn BroadcastedStaticTensor(
    elem: Type,
    shape_a: List<Integer>,
    shape_b: List<Integer>,
) -> Type {
    let target = broadcast_shape(shape_a, shape_b) match {
        s! => s,
        error _ => meta.compilation.err("incompatible static tensor broadcast shapes"),
    }
    StaticTensor<elem, target>
}

pub pure comptime fn row_major_strides(dims: List<Integer>) -> List<Integer> {
    let rank = dims.length();
    var rev: List<Integer> = [];
    var acc: Integer = 1;
    var i = rank - 1;
    while i >= 0 {
        rev.append(acc);
        acc = acc * dims[i];
        i = i - 1;
    }
    var out: List<Integer> = [];
    var k = rev.length() - 1;
    while k >= 0 {
        out.append(rev[k]);
        k = k - 1;
    }
    out
}

impl StaticTensor<T, dims>
where
    T: Type,
    dims: List<Integer>,
{
    pub pure fn elem_count(*self) -> usize {
        comptime { to_usize(shape_element_count(dims)) }
    }
}

-- 同形状/可广播加法实现：严格使用 .Rhs 命名参数、Output 关联类型与 MaybeUninit<Array<...>> 安全写入，彻底废除裸 undefined
impl Add<.Rhs = StaticTensor<T, shape_b>> for StaticTensor<T, shape_a>
where
    T :- Copy + Add,
    requires T.as(Add).Output == T,
    shape_a: List<Integer>,
    shape_b: List<Integer>,
{
    assoc Output: Type = BroadcastedStaticTensor(T, shape_a, shape_b);

    fn add(self, rhs: StaticTensor<T, shape_b>) -> Output {
        const out_dims: List<Integer> = broadcast_shape(shape_a, shape_b) match {
            s! => s,
            error _ => meta.compilation.err("incompatible static tensor broadcast shapes"),
        }
        const total = to_usize(shape_element_count(out_dims));
        const rank = to_usize(out_dims.length());
        var buf = MaybeUninit<Array<T, total>>.uninit();
        -- 依赖 WL-04-26 `*T.addr()`（或使用 `buf.as_mut_ptr()'bitvec.as(usize)`，不使用非法的 `*T.as(usize)`）
        let base_addr = buf.as_mut_ptr().addr();
        let elem_stride = builtin.mem.align_up_usize(Type.size_of(T), Type.align_of(T)) match {
            s? => s,
            null => Type.size_of(T),
        }
        var linear_idx: usize = 0;
        while linear_idx < total {
            -- 编译期由 `row_major_strides` 确定各轴坐标投影；每个输出槽恰好写入一次后整块交付
            var rem = linear_idx;
            var idx_a: usize = 0;
            var idx_b: usize = 0;
            inline for axis in 0..rank {
                const axis_int = meta.from_usize(axis);
                const stride_out = comptime { to_usize(row_major_strides(out_dims)[axis_int]) }
                const coord = rem / stride_out;
                rem = rem % stride_out;
                const offset_from_right = comptime { out_dims.length() - 1 - axis_int }
                const pos_a = comptime { shape_a.length() - 1 - offset_from_right }
                const pos_b = comptime { shape_b.length() - 1 - offset_from_right }
                inline if pos_a >= 0 {
                    const dim_a = comptime { shape_a[pos_a] }
                    const stride_a = comptime { to_usize(row_major_strides(shape_a)[pos_a]) }
                    let zero_a: usize = 0;
                    let mapped_a = inline if dim_a == 1 { zero_a } else { coord }
                    idx_a = idx_a + mapped_a * stride_a;
                }
                inline if pos_b >= 0 {
                    const dim_b = comptime { shape_b[pos_b] }
                    const stride_b = comptime { to_usize(row_major_strides(shape_b)[pos_b]) }
                    let zero_b: usize = 0;
                    let mapped_b = inline if dim_b == 1 { zero_b } else { coord }
                    idx_b = idx_b + mapped_b * stride_b;
                }
            }
            let sum_elem = self.storage[idx_a] + rhs.storage[idx_b];
            unsafe {
                builtin.mem.raw_write<T>(base_addr + linear_idx * elem_stride, sum_elem);
            }
            linear_idx = linear_idx + 1;
        }
        StaticTensor<T, out_dims> {
            storage: unsafe { buf.assume_init() },
        }
    }
}

-- 负例（编译期拒绝）：形状 [128, 64] 与 [32] 在编译期触发 ShapeMismatch / BroadcastedStaticTensor 错误
{-
comptime fn bad_tensor_shape_pair() {
    let _ = broadcast_shape([128, 64], [32])!; -- 触发 TensorShapeErr.ShapeMismatch（64 != 32 且均非 1）
}
-}
```

> **多维 `SpaceApply` 与 `VariadicCollector` 衔接（对应 WL-03）**：`xs[0..2, 0, ..]` 在语法层依赖 WL-03 提出的变参收集协议 `VariadicCollector`（先收集为 `SmallVec` / 局部 `Array<RangeOrIndex, R>` 再转成 `Slice<RangeOrIndex>`）；在静态确定维数的 `StaticTensor` 上也可通过 `inline for` 静态遍历元组实参，在编译期确定输出视图维度类型 `dims_out`。

#### 3.3.4 核融合（Kernel Fusion）与 `pipeline-projection` 对接 Tier 2 `tensor_protocol`

为了在不膨胀内置包的前提下支持 GPU/并行张量计算融合：
1. **编译期表达融合**：用户以常规算子编写组合计算（如 `alpha * a + b`），或以 CMTT 开放代码 `< a: StaticTensor<f32, S>, b: StaticTensor<f32, S> |- StaticTensor<f32, S> >` 构造融合体；编译器利用 `P4`/`P5` 纯度与线性不变量，将多层按元素操作合并为单层索引循环，消除中间全形状临时 `StaticTensor` 分配。
2. **Tier 2 跨管线投影**：
   - 将融合后内核函数标记 `^attrs.compilation.pipeline` 回调；
   - 回调接收 `meta.compilation.IIR.Func` 并通过 `meta.runtime.invoke` 调用 Tier 2 `tensor_protocol.GpuProjection<target>.compile_entry(ir)`，产出设备内核句柄 `KernelHandle`；
   - **严禁把任何设备分配器、CUDA launch config 或特定硬件内核代码写入 Tier 1 六内置包**。

---

### 3.4 组合子代数（零开销流融合与阶段化解析组合子）

#### 3.4.1 从 `2026-6-10` 放弃路径到双正交编译期融合机制

`proposal_trace/2026-6-10-better-combinators.fl` 试图直接在普通函数签名上添加非标准上下文子类型化 `< T | PsiP >` 与 `PsiP <: PsiE`，因耦合复杂度过高被作者标注 `-- gave up`。现行 Flurry 通过以下两套**正交且已进入正式文档**的语言机制彻底实现零成本组合子融合：

1. **机制 A：`inline fn` + 推送式效应 `#emit_item<T>`（`builtin.iterator`）**
   - 依据 `library/builtin/protocols/iterator.fl` 与 `2026-8-28-better-combinators!.fl`：
   - 组合链 `vec.iter().map(...).filter(...).collect_to_vec()` 的核心步骤全部为 `inline fn`，通过 `#emit_item<Item>(val)#`（**不使用不存在的 `perform` 关键字**）向下游推送元素；
   - **完全去森林化（Deforestation）**：编译期按 direct-passing 内联 `#emit_item` 处理器后，整个链条被压平为单重 `while` 循环与局部变量更新，**零中间迭代器状态枚举体、零堆分配闭包、零虚表分派**！
2. **机制 B：CMTT 配对模式与表达式组合子（`meta.patterns` + `meta.code`）**
   - 依据 [patterns-and-arms.md](file:///home/fin/luna/docs/cmtt/patterns-and-arms.md)：通过传递模式代码 `< T -| Ψ >` 与配对表达式代码 `< Ψ |- U >` 在编译期直接拼装单一内层 `match` 循环体（支撑过滤-映射组合子与 Tier 2 `parser_protocol.ComptimeParserCompiler`）。

#### 3.4.2 组合子融合等价重写律与线性/资源守恒前提

| 法则编号 | 融合变换 | 必须满足的形式化前提（由 `program_verification` 与效应检查守卫） |
|---|---|---|
| **CF-R1**（`map-map` 融合） | `iter.map(f).map(g)` -> `iter.map(\|x\| g(f(x)))` | 仅在 `inline` + 单次直接恢复（one-shot direct passing）下恒成立，严格保留每次迭代 `f` 后紧跟 `g` 的副作用顺序！ |
| **CF-R2**（`filter-filter` 融合） | `iter.filter(p).filter(q)` -> `iter.filter(\|x\| p(x) and q(x))` | 利用 Flurry `and` 短路语义：当 `p(x)` 返回 `false` 时绝不执行 `q(x)` |
| **CF-R3**（`map-filter` -> `filter_map`） | `iter.map(f).filter(p)` -> 单步转换+分支提前跳过 | 当 `x` 被 `filter` 过滤掉且 `f(x)` 产出的中间值含 `Once` 资源时，融合后分支内必须插入完整 `Drop` 析构！ |
| **CF-R4**（短回路 `break`/`?`/`!` 退出清理） | 下游在 `#emit_item` 处理臂中 `break` / `return` / `!` 传播错误 | 上游生产者循环在栈展开/直接跳出路径上，仍须对其持有的活跃外部迭代器与容器视图执行准确清理 |
| **CF-R5**（CMTT 解析/序列组合子） | `seq(p1, p2)` / `alt(p1, p2)`（经 `code_sequence` / `code_match`） | 每段 `quote` 保持单洞限制，输出端口上下文由构造时一次性检验通过，生成的 `match` 接受全量 `Once` 与 `Drop` 核验 |

#### 3.4.3 完整融合契约示例与反例

```flurry
-- 示例：基于 builtin.iterator.emit_item 的单次推送零分配过滤映射融合契约
use builtin.Type;
use builtin.iterator.emit_item;
use std.collections.Vec;

pub inline fn push_filter_map(
    producer: %c1 fn() -> #emit_item<In> void,
    transform: %c2 fn(In) -> ?Out,
) -> #emit_item<Out> void
where
    In: Type,
    Out: Type,
    c1: Type.vars.Closure,
    c2: Type.vars.Closure,
{
    -- 按 Flurry `expr# { ... }` 语法拦截上游 `#emit_item<In>` 并在同帧内直接分发，零中间 Vec 或状态机
    producer()# {
        emit_item<In>(item) => {
            transform(item) match {
                out? => emit_item<Out>(out)#;,
                null => {}, -- 若 In 含未消耗清理责任在此路径自动完成
            }
        },
    }
}

-- 负例（编译期拒绝）：试图通过重复使用或两次移动含 Once 资源的项违反单次/仿射契约
{-
use std.string.String;

fn bad_double_consume(
    producer: %c fn() -> #emit_item<String> void,
)
where
    c: Type.vars.Closure,
{
    producer()# {
        emit_item<String>(owned_str) => {
            emit_item<String>(owned_str)#;
            let _dup = owned_str; -- 编译错：owned_str 为 Once 资源且已移出，严禁二次消费
        },
    }
}
-}
```

---
### 3.5 匹配自动机与编译期 Regex DFA/NFA 合成（桥接 Tier 2/3）

#### 3.5.1 编译器通用匹配自动机合成（`pattern_automata::DecisionDag`）

依据 `compiler/pattern_automata/src/lib.rs`、`compiler/nir/src/match_plan.rs` 及 `02-pattern-matching-and-views.md`（§3.1–§3.6）：

1. **跨视图统一观察抽象（`ObservationFactor`）**：
   `DecisionDag` 合成引擎将各类模式条件统一投影为**稳定幂等观察键**：

| 模式类别 | 观察因子与域格（Lattice） | DAG 节点优化策略 | 穷尽性条件 |
|---|---|---|---|
| 标量 / 枚举 / `?T` / `!E T` | 判别标签、有限整数区间、布尔二元域 | `Switch` 多路跳转表 + 公共前缀合并 | 全分支覆盖或整数区间全覆盖 |
| `BitVec<N>` 段模式 | 定长高位->低位切片 `bits[lo..hi]` + 掩码比较 | **Masked Prefix Radix Trie**：按共用常量掩码比特位优先分支 | 位向量域覆盖完备（掩码并集为全空间） |
| `StableListPatternView` / `Endless` | `LenInterval`（`[lo, hi]` 或 `[min, +inf)`）与位置读取 `(list_id, index)` | 单次观察懒缓存 `list_views`（同一 `(list_id, index)` 全树最多调用一次） | 有限列表区段覆盖 $[0, +\infty)$；无限列表以 `[prefix, ..._]` 覆盖 $[prefix, +\infty)$ |
| `StableMapPatternView` / `StructPatternView` | 字段存在探测 `(map_id, .key)` 与值节点 | 同键只调用一次 `map_pattern_view_get(.key)`，共享成功解包结果 | `StructPatternView` 已知封闭域全覆盖；开放 `Map` 要求 `_` |
| `StableConcatPatternView` | 前缀/后缀常量字节/切片比对 Trie | **Prefix / Suffix Trie + Aho-Corasick 公共前缀节点共享** | 需含覆盖全集或 `_` |
| Tier 1 `Stable` `case` 视图 | `(case_fn_id, subject, frozen_args)`（满足 P3 稳定观测） | 同输入跨臂单次调用结果缓存；Tier 2/3 不纯 `case` 退回串行 `Observe`/`Then` | 除非标注已知互斥完备集合，否则要求 `_` |

2. **列选择启发式（Column Selection）**：
   - 首选必需（无通配）列、最小分支因子且与上层守卫解耦的稳定列；
   - 若用户提供分支频率提示（例如基于性能剖析或 `distribution` 概率提示，`2026-7-24`），仅允许在**同一互斥层**或**已证明 P3 纯稳定**的测试边之间重排测试次序；**绝对禁止改变存在副作用或短路失败语义的观察顺序**！

#### 3.5.2 编译期 Regex DFA/NFA 合成范式（严格三层分离，桥接 Tier 2 `regex_protocol`）

> **三层反膨胀强制声明**：`regex` 状态机、NFA/DFA 构建器与正则语法解析器**严禁放入 `builtin`、`std` 或 `verification` 等六内置包**。Tier 1 仅提供 CMTT 代码组装（`meta.code` / `meta.patterns`）、`TuplePatternView` 协议、`BitVec<256>` 字节字符集掩码与 `Splice` 常量切片支持；Tier 2 `regex_protocol`（`10-official-protocol-packages-and-boundary.md` §3.16）定义公开编译契约；Tier 3 生态包（如 `regex_dfa`）承载具体的 CT DFA 合成器与零分配运行期执行器。

**CMTT 自动机生成管线（四步合成流程）**：
1. **CT 解析与 Thompson NFA**：在 `comptime fn` 中调用 `compile_regex_ast(pattern)` 校验语法并捕获非法正则为 `RegexErr`（`EmptyPattern`、`UnbalancedParen`、`InvalidQuantifier`、`StateLimitExceeded`）；
2. **字节类划分（Byte Equivalence Classes）与子集 DFA 最小化**：以 `BitVec<256>`（每个等价类压缩为一个 256 位掩码常量，利用 `BitVec` 位代数求并/交）合并相同转移行为的 `u8` 字符集合，用 Hopcroft 划分生成最小 DFA 状态表 `List<DfaStateRow>`；
3. **CMTT 零分配运行时匹配器代码生成**：利用 `meta.patterns.code_sequence` 与 `meta.patterns.code_match` 将最小 DFA 状态转移编码成嵌套 `ExprCode`（或者通过 `meta.List<DfaTransition>.as(builtin.Splice).splice(table)` 静态落盘为只读常量切片 `Slice<DfaTransition>`，**严格遵守 CMTT 单洞规则，不在单个 `'(` 中写多个 `~<...>`**！）；
4. **接入 `match` 模式解构**：`regex_dfa.RegexMatcher` 同时实现 Tier 2 `regex_protocol.PatternCompile<.dfa_linear>` 与 Tier 1 `meta.patterns.TuplePatternView`（细节参见 `02` §3.5.3 示例 C 与 `10` §3.16.1）。

以下完整 Flurry 示范展示 Tier 3 生态合成包 `regex_dfa` 如何作为 `10` §3.16（`regex_protocol.PatternCompile`）的底层 DFA 编译辅助核，利用 `DfaStateRow :- Splice` 与 CMTT 单端口模板合成确定性自动机代码（`meta.List<T> :- Splice` 跨阶段切片落盘契约参见 `01` `WL-01-28` 与 `03` `WL-03-17`）：

```flurry
-- 模块：regex_dfa（Tier 3 生态合成示例包；严禁放入 builtin / std / verification 六内置包）
use builtin.{Splice, TextBytes, Type, bool, str, u64, u8, usize, void}
use meta.{ComptimeTextBytes, List}
use regex_protocol.{RegexErr, RegexSpan}
use std.collections.Slice;

pub struct DfaStateRow {
    accepting: bool,
    -- 为保持示例自包含，以单字节字符转移目标状态示范；完整实现使用 BitVec<256> 等价类掩码表
    expected_byte: u8,
    next_state: usize,
}

impl Clone for DfaStateRow {
    fn clone(*self) -> Self {
        DfaStateRow {
            accepting: self.accepting.*,
            expected_byte: self.expected_byte.*,
            next_state: self.next_state.*,
        }
    }
}
impl Copy for DfaStateRow {}

-- 按 docs/comptime/quoting-and-splicing.md:55，自定义类型进入常量区必须显式实现 Splice
impl Splice for DfaStateRow {
    assoc RuntimeTarget: Type = DfaStateRow;

    fn splice(comptime data: Self) -> Self {
        data
    }
}

pub pure comptime fn compile_dfa_table(pattern: str) -> !RegexErr List<DfaStateRow> {
    let view = pattern.as(ComptimeTextBytes);
    let len = view.byte_length();
    if len == 0 {
        return error RegexErr.EmptyPattern;
    }
    var rows: List<DfaStateRow> = [];
    var idx: u64 = 0;
    var next_idx: usize = 1;
    while idx < len {
        let b = view.byte(idx) match {
            ch? => ch,
            null => return error RegexErr.SyntaxError,
        }
        if b == 40 or b == 41 {
            return error RegexErr.UnbalancedParen;
        }
        rows.append(DfaStateRow {
            accepting: idx + 1 == len,
            expected_byte: b,
            next_state: next_idx,
        });
        idx = idx + 1;
        next_idx = next_idx + 1;
    }
    rows
}

-- 运行时只读静态表驱动 DFA 执行引擎（零堆分配、线性 O(n) 扫描）
pub fn run_compiled_dfa(input: str, table: Slice<DfaStateRow>) -> ?RegexSpan {
    let text_view = input.as(TextBytes);
    let total = text_view.byte_length();
    let table_len = table.len();
    var state_idx: usize = 0;
    var byte_idx: u64 = 0;
    while state_idx < table_len {
        let row = table.get(state_idx) match {
            r? => r,
            null => return null,
        }
        let actual = text_view.byte(byte_idx) match {
            b? => b,
            null => return null,
        }
        if actual != row.expected_byte {
            return null;
        }
        state_idx = row.next_state;
        byte_idx = byte_idx + 1;
    }
    if byte_idx != total {
        return null;
    }
    RegexSpan { start: 0, end: state_idx }
}

pub pure comptime fn FixedSpanLayout(pattern: str) -> Type {
    let view = pattern.as(ComptimeTextBytes);
    if view.byte_length() == 0 {
        void
    } else {
        RegexSpan
    }
}

-- 在单端口 CMTT 模板中生成匹配 `< input: str |- ?FixedSpanLayout(pattern) >`（零多洞违例）
pub comptime fn synthesize_dfa_matcher_code(
    pattern: str,
) -> !RegexErr < input: str |- ?FixedSpanLayout(pattern) > {
    let table = compile_dfa_table(pattern)!;
    -- table 为编译期数据，直接按常量捕获并在模板体内经 Splice 静态落盘
    if pattern.as(ComptimeTextBytes).byte_length() == 0 {
        return error RegexErr.EmptyPattern;
    }
    '(
        run_compiled_dfa(
            input,
            meta.List<DfaStateRow>.as(Splice).splice(table),
        )
    )
}

-- 负例（编译期拒绝）：试图传入运行期动态字符串或非正则字面量给编译期 DFA 合成器
{-
fn bad_runtime_pattern_synthesis(dyn_pat: str) {
    -- 编译错：`synthesize_dfa_matcher_code` 需要确定编译期 str，不可接收运行期变量 `dyn_pat`
    let _code = synthesize_dfa_matcher_code(dyn_pat);
}
-}
```

---

### 3.6 `attrs` 官方属性键增量与 `language-items.conf` 绑定汇总

#### 3.6.1 `attrs` 新增 `__flurry_*` 键建议（移交 WL-06 统一注册）

以下属性严格遵循 [official-attribute-keys.md](file:///home/fin/luna/docs/comptime/official-attribute-keys.md)，由 `attrs` 构造函数生成（严禁在源码使用裸 key）：

| `attrs` 公开入口 | 生成的 `__flurry_*` 条目 | 消费阶段 | 用途与安全边界 |
|---|---|---|---|
| `attrs.purity.at_least(level)` | `__flurry_purity_at_least: Symbol` | `program_verification/src/purity.rs` | 强制校验函数纯度不低于声明级别（如 `.p4_value_pure` / `.p3_stable`），不足直接编译报错 |
| `attrs.purity.skip_unsafe_propagation` | `__flurry_purity_skip_unsafe_propagation: true` | 调用点纯度检查 | **仅用于调用点**审计拦截已验证安全包装的 `unsafe` 传播（不可用于隐瞒实际 `system()` / 外部写，见 `OQ-09-02`） |
| `attrs.verification.trusted_axiom` | `__flurry_verification_trusted_axiom: true` | `verification` 证明器 | 显式审计标记无证明体的 `axiom` 声明（使依赖链可追溯、不悄然混入 `safe` 证明） |
| `attrs.packed.bit`（已有） | `__flurry_packed_mode: .bit` | Layout & `bitvec_opt` | 将紧凑结构体直接降码为 `BitVec<W>`（见 `library/attrs/README.md:14`） |
| `attrs.compilation.pipeline`（已有/WL-05） | `__flurry_pipeline: true` | `pipeline-projection` | 标注 `IIR.Func` 投影调度回调，桥接 Tier 2 `tensor_protocol` |

#### 3.6.2 `language-items.conf` 与编译器协议绑定增量

| 键名 | 绑定目标 | 说明 |
|---|---|---|
| `verification-spec-int-type` | `verification.spec.SpecInt` | 将数学整数规格类型身份绑定到验证求解器（按阶段 P1 启用） |
| `verification-spec-nat-type` | `verification.spec.SpecNat` | 将良基自然数规格绑定到 `decreases` 检验器 |
| `verification-ghost-type` | `verification.ghost.Ghost` | 识别幽灵包裹器并在后端代码生成彻底擦除 |
| `verification-memory-effect-type` | `verification.memory.MemoryEffect` | 识别 `outcomes` / `predicate -> MemoryEffect` 构造体 |
| `iterator-emit-effect` | `builtin.protocols.iterator.emit_item` | 识别推送式流融合的线性内联效应核心（已存在并扩展至 `CF-R1..CF-R5`） |

---

## 4. 排期优先级与分阶段交付路线

### 4.1 分阶段里程碑与依赖拓扑

1. **P0（纯度底座收口与移动/析构语义统一）**：
   - 包含条目：`WL-09-06`（导出 `P0`–`P5` + memory-effect IIR 摘要、补齐 `static` 变量检测、修复裸环境 `comptime t: Type` bug）、`WL-09-07`（统一显式 `move` vs 真正 `kill` 的资源与区域迁移语义）。
   - 前置依赖：直接基于现有 `compiler/program_verification/src/{purity,moves,drop,memory}.rs` 推进。
   - 交付目标：消除 `purity_system_status_report_2026-11-21.md` 指出的已知缺口，为后端优化、`BitVec` 切片融合和 `emit_item` 组合子融合提供可信纯度与不重叠区域证据。
2. **P1（`verification` 公共 API、`BitVec` 合成、`inline` 流组合子融合与 `DecisionDag` 通用化）**：
   - 包含条目：`WL-09-01`、`WL-09-02`、`WL-09-03`、`WL-09-05`、`WL-09-08`、`WL-09-09`、`WL-09-13`、`WL-09-14`、`WL-09-15`。
   - 前置依赖：P0 完成后可并行推进；`WL-09-14` 与 `WL-09-15` 强耦合 `WL-02` 交付的 `MapPatternView`、`ConcatPatternView` 与 `TuplePatternView`。
   - 交付目标：落地 `verification.{spec, ghost, memory}` 类型定义与 `decreases` 检验；完成 `bitvec_opt` 等价代换与 `Unknown` 保全测试；打通 `Iterator` `#emit_item` 多步融合与 `DecisionDag` 多协议自动机。
3. **P2（静态形状张量原型、条件 `outcomes` 与 CMTT 正则合成接入）**：
   - 包含条目：`WL-09-04`（条件/分支 `outcomes`）、`WL-09-10`、`WL-09-11`（静态 `Tensor` 广播/收缩/切片，依赖 `WL-03` `VariadicCollector`）、`WL-09-16`（Tier 3 `regex_dfa` 编译期自动机合成示例接通 `regex_protocol`）。
   - 前置依赖：P1 `MaybeUninit` / `bitvec_opt` / `code_match` 稳定 + WL-03 `VariadicCollector` 与 WL-10 Tier 2 接口稳定。
4. **P3（核融合优化与 GPU/并行 `pipeline-projection` 闭环）**：
   - 包含条目：`WL-09-12`（跨多层张量表达式融合与 `IIR.Func` 回调桥接至 Tier 2 `tensor_protocol`）。
   - 前置依赖：WL-05 `meta.compilation.IIR` 只读访问器与 `meta.runtime.invoke` 完备交付。

### 4.2 关键验收测试大纲（含正例、负例与三层反膨胀门禁）

| 里程碑 | 核心验收用例范围（须以真实 `--sysroot library` 验证） |
|---|---|
| **P0** | 1. **纯度 `P0`–`P5` 分级**：`P5` 函数允许安全外提循环不变量；`P4` 允许死计算消去但保留 `spin()`；`static` 读写自动踢出 `P4`；`^attrs.mem.volatile` 严格锁定 `P0`；<br>2. **裸环境 `comptime t: Type` 修复**：无 sysroot 裸测试不再误报 `default expression requires P3`；<br>3. **`move` vs `kill`**：`pass_string` 后保留指向堆字符区的合法视图切片直至原缓冲区真正析构；局部指针逃逸必须拒绝。 |
| **P1** | 1. **`verification` & PTC 隔离**：`SpecSeq` / `Ghost` / `PointsTo` / `decreases` 通过 `verified` 证明并在最终目标代码中零字节存在；试图将 `MemRegion.is_alive()` 写入编译期 `Type` 谓词必须编译拒收；<br>2. **`bitvec_opt` & `Unknown`**：`BV-R1..BV-R5` 等价压缩后，含副作用参数在 `Unknown` 路径仍保持求值一次且 `Once` 资源准确清理；`^volatile` 读取严格禁做 **BV-R3** 合并；<br>3. **`emit_item` 融合与 `DecisionDag`**：`vec.iter().map(...).filter(...).collect_to_vec()` 编译为无中间结构的单循环，early-break 时清理不泄漏；两次移动 `Once` 元素在 `# { ... }` 中被拒绝；`StableConcatPatternView` 多臂共享前缀比对一次。 |
| **P2** | 1. **静态 `Tensor` 形状代数**：`[128, 128] + [128]` 编译通过并在运行时沿第 0 维零拷贝广播；`[128, 64] + [32]` 在实例化时精准报 `ShapeMismatch`；<br>2. **Tier 3 `regex_dfa` 合成校验**：`synthesize_dfa_matcher_code` 为字面正则生成线性状态表；运行期非常量串 `dyn_pat` 必须编译拒收；静态包扫描门禁确认六内置包中无任何 `regex` 符号或具体 CUDA/BLAS 驱动。 |
| **P3** | 1. **核融合与 `pipeline-projection`**：融合算子经 `^attrs.compilation.pipeline` 提交至模拟 Tier 2 `tensor_protocol.GpuProjection`，校验 `IIR.Func` 生命周期在回调结束后不可悬空逃逸。 |

---

## 5. 待作者裁决的语义空缺与冲突（Open Questions）

### OQ-09-01：`verification` 中“可运行 `predicate`”与“纯逻辑/分离逻辑 `predicate`”的区分规则
- **冲突来源**：[runtime-purity.md](file:///home/fin/luna/docs/resource-management/runtime-purity.md#L127-L140) 区分了“要求 P3+终止的可执行 `predicate`”与“不要求可判定的纯逻辑规格”；[memory-effects.md](file:///home/fin/luna/docs/resource-management/memory-effects.md#L53) 又定义了返回 `MemoryEffect` 的 `predicate`。但三者当前共享同一关键字 `predicate`。
- **推荐方案**：**按返回域与使用上下文自动分层检查（不分裂新关键字）**：返回 `MemoryEffect` 的 `predicate` 由效果分析层消费；返回 `bool` 且仅含逻辑构造（`SpecInt`/`SpecSeq`/`Perm`/`PointsTo`/量词）的 `predicate` 允许出现在 `verified` / `lemma` 静态证明上下文，若被放进 `safe` 代码的动态 `assumes` 检查位且无法静态证成，才在调用点报 `"non-executable logic predicate cannot run as runtime checker"`；反之含纯可执行运算且满足 P3+终止的 `predicate` 通吃两边！
- **备选方案**：引入显式关键字修饰 `spec predicate` / `ghost predicate` 专指不可执行的纯逻辑谓词，将 `predicate` 严格限定为可执行检查器。

### OQ-09-02：`^attrs.purity.skip_unsafe_propagation` 的合法边界
- **冲突来源**：`proposal_trace/2026-8-28-purity-at-runtime.fl:39-42` 曾给出在函数体内调用 `println` 后标注 `^attrs.purity.skip_unsafe_propagation` 的示例；而 [runtime-purity.md](file:///home/fin/luna/docs/resource-management/runtime-purity.md#L141-L145) 明确声明 **“阻断某个审计标记的传播，不会消除真实输出、写入或终结；runtime pure 不是任意免检的信任入口”**。
- **推荐方案**：**严格收窄 `skip_unsafe_propagation` 仅用于抑制“安全语法包装层对 `unsafe` 能力要求的传染”**（例如安全封装内部已经以受控方式完成了边界校验），**绝不免除底层真实 `system()` / `write` 内存效果记录**；在调用 `println`（产生真实 I/O）上声称 `P4` `pure` 必须依然报错！
- **备选方案**：完全废除 `skip_unsafe_propagation`，统一要求用 `unsafe { ... }` 块配合可信契约 `^attrs.verification.*` 或 `verified` 交付包装证明。

### OQ-09-03：`safe` 自动验证器三次算术边界与 4D 运行时 Checker 契约化
- **冲突来源**：`proposal_trace/2026-9-16-memory-effects.fl:937-1020` 将 `safe` 内置算术自动推导严格限在 **$\le 3$ 次（degree <= 3）多项式范围**，把四维边界检验 `four_dimensions_fit` 放到受检 runtime `predicate`；[contracts-and-verification.md](file:///home/fin/luna/docs/resource-management/contracts-and-verification.md) 尚未将“三次界限 + 乘法系数豁免”定为硬性前端规范。
- **推荐方案**：正式确立 **`safe` 模式提供确定性的 $\le 3$ 次算术与线性区间求解器**（编译器内置已知常系数不计次数），超出次数或非线性组合统一输出清晰残余证明义务，由用户通过 `verified` 函数归纳分解或经受检 `predicate` + `assumes` 运行时检查器安全衔接。
- **备选方案**：不将次数写死为语言规范，作为编译器分析预算参数（`--verifier-budget`）暴露，仅承诺有界不完整自动推导。

### OQ-09-04：零阶张量 `Tensor<T, .dims = []>` 与标量 `T` 的类型相等性
- **冲突来源**：`proposal_trace/2026-8-27-tensor-rank-polymorphism.fl:19-21` 写入 `axiom scalar_is_a_zero_rank_tensor where T { Tensor<T, .dims = []> == T }`，且 [static-shaped-tensors.md](file:///home/fin/luna/docs/comptime/static-shaped-tensors.md#L27) 也有“零阶 Tensor 与 T 相等”的语义描述；然而 `AGENTS.md` §5 明确要求 **“Normal form 的头不展开；Identity<T> 不因函数体返回 T 就与 T 相等”**！
- **推荐方案**：**严格遵守 `AGENTS.md` 不展开 Normal-Form 头规则**，不让 `Tensor<T, .dims = []>` 与 `T` 自动结构相等，而是为零阶 `StaticTensor<T, []>` 提供 `.into_scalar()` / `.from_scalar()` 与原生标量运算符重载（或将 `ScalarBroadcast<T>` 作为通用 `Add` / `Mul` 参数接收标量 `T`），从而在不违背 normal-form 类型身份统一规则的前提下提供同等书写体验（`zeros + 1.0`）。
- **备选方案**：若将 `Tensor` 本身定义为返回嵌套数组 / 在 `rank == 0` 直接作为条件类型函数在 **普通计算位置** 返回 `T`（而非 opaque normal-form head），可获得相等性，但会失去不透明头标识；或者引入专用内置规范公理仅放行零阶（不推荐，违背 `AGENTS.md` 禁特判原则）。

### OQ-09-05：`StaticTensor` 多维循环写入的安全初始化模型（替换历史裸 `undefined`）
- **冲突来源**：`proposal_trace/2026-8-27-tensor-rank-polymorphism.fl:62` 与 `proposal_trace/2026-8-5-tensors.fl:14` 用 `var result: Output = undefined;` 先声明未初始化张量再逐片写入；虽然正式文档收录了底层 `undefined` 语法词素（`docs/grammar/syntax:51`、`docs/basics/lexical.md:30`、`docs/basics/unit-null-undefined.md:30-40`），但 `library/builtin/` 与 `std` 安全代码规范避免使用非类型安全的裸 `undefined`，未初始化聚合槽位统一交由 `builtin.mem.MaybeUninit<T>`（WL-04）明确管理，并在安全边界经 `verified` 或局部不变式验证完全初始化后调用 `assume_init`。
- **推荐方案**：全面废除 `undefined`，在 `StaticTensor` 的内部构造与高阶降维循环中统一使用 **`MaybeUninit<Array<T, ...>>` 配合安全逐元素写入包装（或先以 `T.default()` / 累加器构建切片后再组装返回）**，对 `verified` 模式直接由分离逻辑逐槽证明初始化完备后零成本脱壳！
- **备选方案**：在语言层面为定长聚合引入专用的逐元素安全推导构造器 `Array.init_with(|idx| ...)` / `StaticTensor.build(|coords| ...)` 供安全代码完全避免暴露 `MaybeUninit`。

### OQ-09-06：`distribution` 概率提示与条件 `outcomes` 归属核心语法还是属性系统
- **冲突来源**：`proposal_trace/2026-7-24-verification.fl:37-40` 使用了 `where distribution b ~ { 0 => .almost_never, _ => .almost_always }` 与 `^probability(.almost_always) case:ok_branch`；而 [memory-effects.md](file:///home/fin/luna/docs/resource-management/memory-effects.md#L12-L16) 中的正式语法只包含 `outcomes expr`、`requires`、`ensures`，未收录 `distribution` 关键字或 `case:label` 后缀。
- **推荐方案**：保持语言语法精简——**条件 `outcomes` 采用复用现行 `match`/条件分支形态的标准化写法，而执行概率分布提示不新加 `distribution` 关键字，改为通过官方属性 `^attrs.verification.branch_hint(...)`（或 `attrs.profile.*`）附着为元数据**，专供后端分支布局与匹配自动机 `DecisionDag` 列顺序优化消费。
- **备选方案**：将 `distribution` 正式纳入 `clauses` 语法供概率分析及验证器直接使用。
