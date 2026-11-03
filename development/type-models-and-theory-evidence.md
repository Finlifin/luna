# Type 模型、理论证据与候选提交

状态：重构设计，未实施。源码核对基线 `517dc043`。
本文细化[求解器总览](solvers-resources-and-lowering.md)，不扩大隐式推导范围，不将程序验证
并入 PTC。依据为正式[原子目录](../docs/type/propositional-constraints/atoms.md)、
[理论分工](../docs/type/propositional-constraints/theory-solvers.md)、
[组合协议](../docs/type/propositional-constraints/solver-protocol.md)及作者对 Type 模型搜索的要求。

## 1. 已有机制与迁移起点

| 现有入口 | 应保留的能力 | 后续需要接通的边界 |
| --- | --- | --- |
| `ptc/src/search.rs`、`search/trail.rs` | first-UIP、回跳、带理由赋值及四类消费者通知 | 新 theory 的同步、异步观察和证据生命周期 |
| `ptc/src/theory_state.rs` | 可回退等价类、分层事实、预算中断前预留回退工作 | 跨 theory 检查点与共享项协议 |
| `ptc/src/lib.rs::verify_proof/verify_witness` | 在精确 scope/前提/准入契约下独立重放；不信任搜索赋值 | 世界证据、通用值和新增 theory 的重放 |
| `ptc/src/type_inputs.rs`、`type_inputs/symbolic.rs` | 选定投影证明、保留外围符号输入、辅助解不自动导出 | 通用 TypeRef/值准入及完整模型解释 |
| `ptc/src/type_inputs/ambiguity.rs` | 同外围环境比较两份解；固定一份环境不能证明全域唯一 | 新 theory 的环境一致性与见证覆盖 |
| `type_resolution/src/obligations/type_inputs.rs` | 独立搜索、保留原义务、只从原匹配关系提出 Type 解 | 与新 CandidateTrial 原子提交和局部任务接线 |
| `type_resolution/src/capabilities/propositions.rs` | Given/选定实现/排除证据，精确借用 World 的结果封装 | trait 证据随联合模型重放；跨查询稳定依赖替代裸借用身份 |
| `ptc/src/model_search.rs`、`model_applications.rs` | 有预算的具体 Type 候选及有限应用解释；候选不成为等式事实 | trait 原子当前阻止裸模型构造；Tuple 特例和专用值遍历仍需退役 |
| `ptc/src/integer.rs` | 数学整数差分约束及可检查边界证据 | 一般线性整数、同余与其他域的明确桥接 |

当前并非“没有 CDCL”或“完全没有 trait 事实”。真正缺口是统一后的组合与交付协议。
也不能把现有 four-atom 搜索器、世界适配器和投影搜索简单拼起来，便宣布 A01–A19 全覆盖。
本次为静态核对，未运行上述模块测试。

## 2. 三种问题必须具有不同入口

以下为内部数学记号，不是新增 Flurry 语法。

| 问题 | 输入 | 成功意味着什么 |
| --- | --- | --- |
| 证明 | 已成立前提 Γ、目标 G | Γ 蕴含 G；反驳 Γ ∧ ¬G |
| 找模型 | 已形成公式 F、语义视图 V | 存在通过联合验证的解释满足 F |
| 提出类型替换 | Γ、原匹配关系 M、允许求解集合 U、导出集合 O | 得到可表达的替换候选及其投影依据；尚不等于可提交 |

F 不可满足与 G 不可证明不同。G 的一个合法反例证明“不被当前前提蕴含”，不是证明
Γ 蕴含 ¬G。调用要求对所有合法输入成立时，一个反例足以拒绝该要求；将其发布为负事实
则需要独立证明。Unknown 两者都不是。

显式输入、定义端刚性 binder 和 runtime dependency identity 不属于 U。
内部为 U 编码的新鲜逻辑符号必须有反向映射及 owner，不能与用户声明身份混用。
O 可以严格小于 U；例如只确定返回域需要的 Type，其他输入继续由合法信息供应。
未导出的搜索选择不是默认实参，也不能因存在于模型中就填入源调用。

候选 requires 不作为反向制造实参的搜索假设。先从获准匹配关系提出解，再在真实 Γ 下
检查 requires；已有正式接口给予的前提则属于 Γ，二者必须在来源上区分。

## 3. 投影证明与提交证明是两道不同检查

设 A 是外围刚性输入，X 是允许求解的 Type，H 是内部辅助变量。
搜索问题为 `Γ(A) ∧ M(A,X,H)`。从模型得到 X 的某个值，只能用于提出候选。
导出符号表达式 `X := e(A)` 时，至少需要以下检查：

1. **形成与可表达性：** e 由当前可用 binder 和受检 Type 项组成；sort、scope、参数域、
   occurs 及运行时身份均合法。不能导出某个搜索域元素的编号或活动 model handle。
2. **投影依据：** 在该搜索问题下证明 `X = e(A)`，或使用语言明确允许的选择规则。
   本设计不新增后一种规则；当前以被确定的投影为提交前提。
3. **原义务兑现：** 回到真实 Γ，在替换后证明原始 M 和候选必检要求。
   搜索时临时假设的 M 不随候选进入 Γ，不能用它自己证明自己。
4. **整体选择与提交：** 其他候选的精度/排除、父版本和 SemanticView 均仍有效，
   再将替换、term result 和证据一起交付。

若只导出部分 X，第三步可留下尚有真实生产者的剩余义务；但候选此时仍未完成，不能将
“部分 Type 可观察”误当成整次调用已提交。允许冻结残余的位置另按语言规则判断。

两个容易漏掉的例子：

```text
Γ = true
M = (X = u8) ∧ (A = u8)
```

搜索有模型，并且在 Γ ∧ M 下 X 确定为 u8；但任意外围 A 并不保证 A=u8。
因此 X=u8 可以是局部提案，整次泛型调用不能据此通过。原义务检查会保留或拒绝 A=u8。
不得把模型里 A=u8 的一次解释写回刚性 A。

```text
Γ = true
M = (X = A)
```

不同外围解释可以分别给出 X=u8 和 X=bool；这不构成同一环境内的歧义。
合法导出是 X=A，并在 Γ 下验证替换后的 A=A。固定 A=u8 后证明 X=u8，不能导出全域常量解。

搜索问题不可满足时，不利用逻辑上的空真为任意 e 制造“唯一解”。必须先确认相应可满足性
或合法符号构造依据；若现有模型能力不足，报告 Unknown，而不是虚构输入。
这可能使实现保守，不能为了消除保守性删除这道检查。

## 4. 歧义与未完成怎样区分

拟定结果保留 `Proposed / Ambiguous / Inconsistent / Unknown / IllFormed`，不以 Option 表达。

Ambiguous 的证据需要在**相同外围语义环境**中给出两组满足原问题、导出投影不同的解。
相同环境包括外围 Type/值、binder、已准入应用解释、候选世界、phase 和 target。
只比较 source 参数表不足以确保两份理论模型具有相同外围解释。

两份模型在辅助 H 上不同，但 O 相同，不是输出歧义。反之，O 具有两个已验证解释时，
不能按搜索顺序、类型名称或最短表示选第一个。不要求证明整个模型唯一。

无法找到第二份模型不证明唯一；唯一性来自投影证明。找不到可表达 e、不具备必要的
theory 解释或预算耗尽分别进入 Unknown 原因，不伪称已证明歧义或无解。
支持用户显式补全的诊断应指出未确定输入及其来源，而不是直接展示抽象模型占位值。

## 5. Theory 接口交付的内容

接口名仅描述目标职责，具体 Rust trait/enum 尚未决定。

```text
TheoryInput = formed terms + signed literals + premise references
Checkpoint = owner + generation + undo boundaries

TheoryEvent =
    Propagate(literal, explanation)
  | Conflict(explanation)
  | NeedObservation(key, required_premises)
  | NeedSplit(coverage_certificate, alternatives)
  | Quiescent
  | Incomplete(reason)

ModelFragment = interpretation + coverage + dependencies + unresolved_items
```

Quiescent 只表示没有新传播，绝不等于 SAT。NeedSplit 的 alternatives 必须覆盖其声称的域；
有限 BV 可以携带穷尽依据，当前出现过的有限 Type 列表不能被当作 Type 全集。
不完备的 Type 候选枚举是搜索启发式，不能产生“已试完所以冲突”的学习子句。

传播解释表达 `guards ∧ premises ⇒ literal`，冲突解释表达 `guards ∧ premises ⇒ false`。
解释至少携带稳定项引用、rule/schema 身份、适用域前提、世界依赖及子证据。
布尔 resolution 只能组合这些真实理由，不能把“本次没找到见证”编码为 theory conflict。

登记部分观察前先证明形成条件。例如 list 元素需要合法索引，关联类型需要对应来源，
布局需要合法表示。观察所得值不反过来证明观察本身合法。
形成条件缺失且有生产者时等待；已证非法时 IllFormed；超预算时 Incomplete。

## 6. 搜索回跳与宿主响应

搜索拥有布尔 trail 和理论检查点；region 拥有搜索任务及真实输入生产者。
theory 请求的宿主观察沿[既定状态协议](elaboration-state-and-publication.md)进行：

1. 保存请求所需语义视图、前提、决策代次和依赖，不把活跃 theory 的可变引用交给宿主。
2. 宿主只执行获准、可隔离的观察/计算；选择未确定前不执行外部不纯行为。
3. 搜索回跳撤销临时赋值并通知各消费者；已在途观察不因此变成无条件事实。
4. 响应回来先核验 owner/generation、视图、前提及形成条件。过期响应不直接传播。
5. 与当前分支无关的稳定计算结果可按其真实依赖缓存；分支结论仍携带 guards。

部分 theory 尚未完成不阻止其他有效传播。若布尔与已有理论已给出可重放反驳，不必为
无关模型片段继续搜索；但不能跳过原公式的必需形成检查来接受非法操作。
没有需要等待的生产者时，Unknown 返回调用者，不能重新将同一问题排队到“自然变好”。

回退容量应在变更前准备，理由节点需至少存活到所有引用它的 trail/学习子句退出。
子句淘汰不能释放活跃赋值所需的证明；可以共享不可变 proof DAG，但不共享候选的可变解释。
预算统计分别保留搜索、外部观察、模型构造与重放成本，不为子请求重新赠送整份额度。

## 7. 共享项和联合模型

同一共享项采用共同的语义身份、sort、binder scope 和形成证据，不能由每个模块独立按
文本重建。不同模块对其解释必须一致；必要的跨域关系由受检 bridge 描述。

| 域 | 模型必须包含的内容 | 不能混入的假设 |
| --- | --- | --- |
| EUF / TYPE | 等价类、合法 Type 表示、已准入构造规律及分类 | Any 相容等于等式；所有分类互斥 |
| NF / 普通冻结值 | 原头、实际参数域、值身份规则、合法应用解释 | 执行头以消去 NF；用户 Eq/Hash 等于求解器相等 |
| INT | 整数解释、边界/同余前提及整数性证据 | 分数模型满足整数式；实现容量溢出等于源不可满足 |
| BV | 固定宽度、signed/unsigned 解释、合法编码 | 数学整数运算默认按位向量截断，或反过来 |
| SEQ | 完整有限描述或明确符号残余、索引前提 | 当前观察到的元素就是整个列表 |
| ROW | error 唯一标签/lacks；effect occurrence 次序及尾部 | 同标签 effect 去重；未知标签按不同 hash 永久分离 |
| TRAIT | Given 或完整选取/排除证据、视图及关联来源 | SAT 指派 true 就制造 impl；未命中就设 false |
| LAYOUT / PRED | 目标和操作前提、已验证观察/展开/执行依据 | 非法布局解释成 NotSized；任意布尔函数充当公理 |

联合验证接收各片段，并检查共享值一致、部分操作前提、有限域覆盖、量词范围与实际世界。
模型中的抽象域元素和有限函数表，只有在对应 theory 明确允许的解释契约下才能成为证据。
它们绝不直接成为可执行源值、布局许可或 `NF(args)=模型代表` 的新定理。
缺少源语义解释时仍是候选模型；不能将抽象 SAT 升格为合法反例或类型替换。

NF 的同余只允许相等输入得到相等应用，不允许反推输入。来源头形状有自己的证据，
不能从 EUF 类的某个代表猜构造来源。Tuple 与用户工厂均通过普通签名、值和准入协议，
不能依赖 `tuple_heads` 作为模型出口白名单或黑名单。

## 8. Trait 世界适配器的具体职责

已有 WorldProposition 通过借用精确 world 阻止将其结果脱离上下文使用；这是当前保护，
迁移前不移除。目标以 SemanticView 和可重放依赖替代长期裸引用，不仅增加一个 world ID。

正证据为已成立 Given，或完整选择结果：trait 实例、impl 声明、替换、要求证明、来源及
必要环境描述。负证据包含适用候选空间和逐项排除依据；候选检索范围也属于依赖。
未知输入可能匹配更精确实现、要求未决或世界前缀未完成时，不能出具完整排除证据。

将一个模型中的 Type 候选代入 trait 查询，得到的是该模型分支下的证据；不据此推出
对全部外围 Type 成立的 trait 事实。要求自身递归回来不算证明，需要保留原目标和递归边，
没有独立依据时未决。关联 Type 观察始终带选定来源，不能按同名成员直接统一。

模型层需要正/负解释包与验证入口，不能仅调用当前 `WorldProposition::verify` 就声称
已有完整模型重放；该入口目前要求 Entailed 结果。真值查询、联合模型和执行分派三者分开。
选择证据携带 runtime identity 不意味着当前已取得对象；EnvironmentPlan 与资源检查
仍由交付层完成，PTC 不证明地址存活。

## 9. 证据格式与缓存有效性

```text
EvidenceEnvelope {
  statement, binder_scope, semantic_view,
  premise_support, admitted_contracts,
  stable_term_roots, dependency_observations,
  proof_or_model_payload, provenance
}
```

当前 Proof/Witness 精确检查 scope、assumptions 和准入集合，是正确的保守边界。
若要跨查询复用更小的 premise_support，必须显式重绑定并重放；不能直接删除精确比较。
只有实际使用的前提被可靠抽取后，才能缩小失效范围。

证据至少区分：纯理论 lemma、带世界前提的 lemma、候选局部结论、模型见证、投影提案、
最终选择证据。缓存 lemma 不等于缓存语义执行；模型解释不得经由缓存变成逻辑假设。
更高层相同编号不代表同一 binder，证据迁入新 owner 必须进行受检映射。

独立重放可共享纯数据结构、算术和规则定义，但不用搜索中的并查集、赋值缓存或未验证
trait 选择来证明自身正确。缓存的重放成功也需绑定证据内容及全部依赖。

内部 dump 保留这些区别，并使用 S-expression，例如：

```lisp
(type-input-proposal
  (view (world 8) (scope 3) (frontier 2))
  (requested (meta 4 7))
  (binding (meta 4 7) (ambient-type 3 1))
  (projection-proof (evidence 12))
  (original-obligations (pending 19))
  (state uncommitted))
```

编号需要 owner/generation 与稳定排序；示例不是 wire schema，也不是现有 dump 承诺。

## 10. 迁移时的必要反例与交付线

| 场景 | 需要证明的行为 |
| --- | --- |
| X=A，模型对 A 有不同解释 | 导出符号 A；不误报相对歧义、不导出单个模型常量 |
| X=u8 且 A=u8，Γ 未限制 A | 搜索提案不能使整个泛型调用通过 |
| X=u8，H 有多个合法解 | O={X} 可确定，不强求完整模型唯一 |
| X=u8 或 X=bool，同外围环境 | 保留两份有效见证，不按先后选择 |
| 搜索关系自身矛盾 | 不凭空真制造 Unique |
| NF 输出相等、输入未确定 | 不反推单射；模型代表不成为 NF 的替换身份 |
| trait 高精度候选未决 | 不选择宽候选，不以模型指定真值绕过 |
| 同一 trait 查询跨 frontier | 复核相应视图；负查找依赖包含候选空间 |
| 决策回跳后旧宿主响应到达 | 不污染新分支；所保留结果带真实前提 |
| 整数约束 2x=1 | 不接纳 x=1/2；非完整搜索耗尽不等于 UNSAT |
| 有限 Type 候选集全部失败 | Unknown，不能推出 Type 域已穷尽 |
| 伪造 proof 前提、scope 或模型共享值 | 独立重放拒绝；失败不改变 parent |

实施顺序应先让现有原子和投影搜索走共同证据接口，再接世界适配器与模型重放，随后按
正式目录补 theory；每一步保留真实 Engine/调用入口，不增设只用于单元测试的平行 solver。
库验收至少包括用户工厂与 Tuple/List 的普通值参数、阶段化 trait 选择及关联类型观察，
必须通过实际 sysroot 的公开接口。程序规格和资源仍在各自层验证。

本轮只定义上述协议与反例，未修改实现、解除忽略测试或运行 solver 回归。
完整组合能力、具体 Rust 接口及性能数据尚未交付，不能以本文存在替代这些成果。
