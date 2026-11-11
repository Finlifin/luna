# Type 模型、理论证据与候选提交

状态：总体协议仍为重构设计；已落地部分在相应小节列出接线和验证。最初源码核对基线 `517dc043`。
本文细化[求解器总览](solvers-resources-and-lowering.md)，不扩大隐式推导范围，不将程序验证
并入 PTC。依据为正式[原子目录](../docs/type/propositional-constraints/atoms.md)、
[理论分工](../docs/type/propositional-constraints/theory-solvers.md)、
[组合协议](../docs/type/propositional-constraints/solver-protocol.md)及作者对 Type 模型搜索的要求。

## 1. 已有机制与迁移起点

| 现有入口 | 应保留的能力 | 后续需要接通的边界 |
| --- | --- | --- |
| `ptc/src/search.rs`、`search/trail.rs` | first-UIP、回跳、带理由赋值及四类消费者通知 | 新 theory 的同步、异步观察和证据生命周期 |
| `ptc/src/theory_state.rs` | 可回退等价类和整数势函数、分层事实、预算中断前预留回退工作 | 其他 theory 检查点与共享项协议 |
| `ptc/src/lib.rs::verify_proof/verify_witness` | 在精确 scope/前提/准入契约下独立重放；不信任搜索赋值 | 世界证据、通用值和新增 theory 的重放 |
| `ptc/src/type_inputs.rs`、`type_inputs/symbolic.rs` | 选定投影证明、保留外围符号输入、辅助解不自动导出 | 通用 TypeRef/值准入及完整模型解释 |
| `ptc/src/type_inputs/ambiguity.rs` | 同外围环境比较两份解；固定一份环境不能证明全域唯一 | 新 theory 的环境一致性与见证覆盖 |
| `type_resolution/src/obligations/type_inputs.rs` | 独立搜索、保留原义务、只从原匹配关系提出 Type 解 | 与新 CandidateTrial 原子提交和局部任务接线 |
| `type_resolution/src/capabilities/propositions.rs` | Given/选定实现/排除证据，精确借用 World 的结果封装 | trait 证据随联合模型重放；跨查询稳定依赖替代裸借用身份 |
| `ptc/src/model_search.rs`、`model_applications.rs` | 有预算的具体 Type 候选及有限应用解释；候选不成为等式事实 | trait 原子当前阻止裸模型构造；Tuple 特例和专用值遍历仍需退役 |
| `ptc/src/integer.rs`、`integer/state.rs` | 数学整数差分约束、可检查边界证据、查询内可回退差分图 | 传播理由的增量维护、一般线性整数、同余与其他域的明确桥接 |

当前并非“没有 CDCL”或“完全没有 trait 事实”。真正缺口是统一后的组合与交付协议。
也不能把现有 four-atom 搜索器、世界适配器和投影搜索简单拼起来，便宣布 A01–A19 全覆盖。
初次表格核对为静态检查；后续实现和执行证据见以下小节。

### 已接通：整数差分一致性的查询内增量状态

2026-11-12 的实现继续使用现有四类 atom，没有改变 PTC 的语言职责。差分图的
变量目录由本次查询的 atom 集建立，边只来自已赋值整数文字；保存的势函数满足
全部已提交边。新增事实从新边源点出发传播变化，达到顶点数轮仍有改善则冲突。
无新增整数边的同步直接复用一致性结果，不重新构图或从零松弛。

EUF 与整数状态共用事实层生命周期。每层保存并查集、边数和势函数撤销边界；
撤销前统一预留预算，恢复后重新准入被该层连带撤销但仍成立的事实。未提交的
松弛遇到冲突、预算耗尽或算术容量不足时撤销到尝试前检查点；这些失败不能留下
可被后续分支消费的部分模型。没有按模型代表元创造额外等式事实。

只有需要交付模型时才将势函数相对零顶点平移，得到独立整数赋值；该步骤仍检查
算术容量并计费。搜索状态只用于搜索加速，独立证明和见证验证不接受其检查点
作为证据。`integer::propagate_explained` 的理由图仍重新建立；行、Trait、Kind
的增量化、跨查询缓存和完整理论组合仍待实现。

本次 PTC 的 245 项检查以及 `type_resolution` 消费者检查通过；命令、日志及
失败恢复覆盖见 [审查回复](audit-reply.md) 的 PTC 小节。这里报告的是一致性状态
及其消费者接线，不能据此宣称整个整数 theory 或完整 PTC 已经增量化。

### 已接通：名义类型中的冻结值索引不等证据

`ptc::value_indices` 扩展了已准入名义构造子的索引比较：除直接标量外，普通 tuple
载荷以及同一保留域的 nominal 载荷可以通过不同字段数、不同 variant 或内部固定
标量建立不等。它比较的是已形成 comptime 值的保留身份，不是用户 Eq、程序状态或
内存规格；不识别 List、Tuple 等库名称，也不为普通 NF 头赋予逆向单射规律。

结构遍历先从本次 query 预算扣除。域标签不能确定相同时、浮点／存储句柄／Type
语义尚需另行证明时，这个入口不提供额外不等证据。证明搜索和独立 proof replay
消费同一受限规则；缺少名义构造子准入不能重放原证明。模型搜索不是这里的否定
证据来源，不把 equality 的反例直接当作 inequality 的证明。

验证：工作目录 `/home/fin/luna`，
`python3 -B development/check.py --timeout 900 -- test -p ptc --no-fail-fast`，
`/tmp/luna-tools-svjat95a/result.json` 退出 0，242 项通过。`nominal_models`
覆盖嵌套值、域和 variant、证明重放、缺失准入、预算耗尽后重试、opaque NF 无单射性、
浮点与未完成存储不提供证据，以及独立值等式继续被 PTC 拒绝。初版新增 variant
用例把枚举 tag 放在非名义域上，被正确诊断为 InvalidNominalValueType；已修正
测试载体为名义域，没有放宽值准入。

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

### 当前接线：materialization 查询共用剩余额度

`Builder` 的接口选择、类型形成前提与 given 查询通过 `query_budget` 交出当前
`fuel`，仅继承初始配置中的深度等限制，不再反复把初始 `steps` 赠送给求解器。
正常或错误返回均结算实际余量，查询 unwind 时保留已预留的额度。staged invocation
改用 VM 的 budgeted Type solver，证明工作从该次 invocation 的额度中支付。

`DeclarationWorld::select_interface` 接收可变预算，直接传递 capability 查询的
Budget／Unresolved／Invalid 类别并返回实际余量。选中的实参从已有证据移出，
不再额外复制。该服务仍只返回选定实现身份，不授予程序规格或内存访问权限。

修改前的真实 `Builder::prove_requirement` 回归显示成功证明后 `fuel` 完全未减少：
`/tmp/luna-tools-vamgxrq0/result.json` 退出 101。此问题不同于预算耗尽的错误分类：
materialization 的完整图构建顶层仍有 String 错误适配；source method resolver
已按下述独立查询贯通类别，但这尚不等于所有宿主入口都采用完整的结构化响应协议。

真实 sysroot 暴露了旧准备路径的额外耦合：每次 required invocation 都预选全部具体
impl，staged input 又可重复构造同一类 dispatch 闭包。查询开始正确收费后，这些
无关选择会耗尽调用者额度。这里不能用提高默认额度或免收查询成本解决。

required／staged source invocation 现在通过 `SourceResolvers` 在同一冻结声明世界中
按需取得类型命题、method identity 和 associated initializer。构造仍检查声明形状、
参数域与 phase；不把“存在具体 impl 声明”直接装成可调用方法，也不预先遍历所有
impl 的父 trait／capability 闭包。真正的成员需求仍由 compiler selection 证明这些
关系，VM 再检查实际参数、公开签名、阶段及调用契约。该入口拒绝混入预选 dispatch 表，
不把表中旧证据与当前宿主服务混用；旧的 closed-table 入口和 IIR 路径仍单独保留。

definition publisher 的非执行准入也有明确来源：source publisher 提供同一世界的
budgeted Type solver，描述符 session 只使用声明／layout 元数据与形成证明，不能调用
method 或 initializer。带 trait 形成条件的字段仍须取得证明；Unknown／拒绝／超预算
不发布候选，后续重试不继承失败状态。无 solver 的既有 publisher 保持 closed-table
准入；不把“尚未取得证明”替换为默认通过。具体类型仍按通用机制处理。

开发中专门增加了 `Checked<T :- Marker>` 作为 builder 字段的反例：移除全局执行表
但遗漏 descriptor proof 接线时，`/tmp/luna-tools-yiibm6n7/result.json` 退出 1，报
TypeContractUnresolved。据此把 source proof 服务贯通到 publisher，而不是删除字段
形成检查。调用模板测试亦区分稳定源码与每次解析新建的 associated initializer：
只要求同样热身状态下没有额外重复编译，不能把所有 initializer 都假定成持久代码身份。

验证工作目录 `/home/fin/luna`，通过 `development/check.py --timeout 900 -- test ...`
执行；各 `result.json` 保存完整命令与退出状态，`stdout.log` 保存逐组结果：

- `/tmp/luna-tools-x0fr796g/result.json`：退出 0；builder_invocations、
  invocation_value_templates、trait_query_failures 共 97 项通过，包含真实 sysroot
  builder、带约束字段与 source selection 的拒绝／恢复。
- `/tmp/luna-tools-w2uz6gjm/result.json`：退出 101；VM、elaboration、materialization、
  type_resolution、luna 的相关 lib 和集成组共 1038 项通过、2 项失败。
  source_reflection 的 `ordered_field_objects_keep_receiver_admission_and_recover_after_failure`
  与 `explicit_symbol_codes_are_reflected_without_inventing_discriminants` 失败于 contextual
  method classification；它们未被跳过或修改预期。
- 对基线 `6d267e8f` 还原本批拥有的源码后，以相同 source_reflection 命令复测：
  `/tmp/luna-tools-wnx6leq7/result.json` 退出 101，同样 15 项通过、同样 2 项失败，
  位置与诊断一致。随后逐字节恢复本批源码；作者已有修改未纳入此基线切换。
  这些失败在该批次仍是前端分类缺口，不计作本批通过证据；后续已通过
  [统一计算型接收者的类别观察](values-staging-and-publication.md#计算型-type-接收者的类别观察)
  修复，未改变 NF 身份或候选资格规则。

### Source method identity 与可执行契约分离

方法身份查询已从 `Builder` 的 `Roots::MethodIdentity` 路径移出，后者删除。
`DeclarationWorld::method_identity` 只关闭声明侧的接口形成条件、父 trait、
接口与实现的 capability 前提，并按选定实现取得成员身份及参数替换。
非根前提可以由当前世界的 given 证据支持；根方法仍必须选择实际实现，不能仅凭
“满足 trait”就伪造可执行代码。查询访问集合与尚待处理的前提仅存活于本次请求。

接口参数准入在 declaration world 与完整 materializer 之间共用同一实现，
按声明域检查普通 comptime 值，不按具体 trait、Tuple 或列表名称分流。
方法绑定共用 binder 构造，身份查询只检查替换的 telescope 与 scope，不复制或
遍历成员签名中的程序表达式、pre/postcondition 或方法体。成员自身的类型条件、
实际参数、公开签名、phase 与程序契约仍由 VM 的调用入口检查。
因此选定一个方法并不等于准许调用，更不提供程序验证或内存访问证据。

`resolve_method_identity_in_context` 在成功和失败时均结算实际剩余额度。
required、staged 与 luna invocation 共用一个 VM 适配器，分别保留
Budget → StepsExhausted、Unresolved → TypeContractUnresolved、
Invalid → UnmaterializedDispatch，并保留 callee source；不靠错误文本或剩余
额度猜测失败类型。传统完整 materialization、quote 与 closed-table 入口仍有
各自的构建职责和 String 边界，本次没有把它们也声称为已退役。

针对性证据覆盖大量可执行契约不增加身份查询工作、接口形成与父 trait 证据、
预算不足／错误成员／缺失实现后的恢复，以及真实 sysroot 的调用路径。
同一工作目录与检查入口下：

- `/tmp/luna-tools-u_3_fg2g/result.json`：退出 0，方法／关联项查询、普通参数域、
  VM trait 调用与动态 trait 子句等 66 项通过。
- `/tmp/luna-tools-elvlr2qz/result.json`：退出 0，相关 lib、builder、prepared/staged
  invocation、两阶段 trait、列表／排序／公开库协议与展开边界等 353 项通过。
- `/tmp/luna-tools-_b6zecdn/result.json`：退出 0，更新后的 trait_query_failures
  共 8 项通过，包含选定方法后仍执行类型条件及 pre/postcondition、拒绝前后
  的调用次数与恢复；与前两组去重共 420 项。本批未重跑上述已确认的
  source_reflection 基线失败，也未宣称它们已经修复。

### 已统一的源声明索引构造

`Index::from_package_metered` 现在共同承接 declaration world、required comptime
快照与 driver 的源索引构造。原来分散的布局复制、参数域登记与 impl 插入循环已删除；
driver 的调用／缓存／键协议／资源协议／参考执行通过同一个薄错误适配器消费该入口。
整份 package 的元数据预检、复制、候选插入与参数域登记使用同一可变额度，不再给
每个 impl 或参数域重新分配默认额度。预算或非法声明导致失败时，局部部分索引丢弃；
只有全部构造完成才返回索引，实际未用额度保留在调用者的 Budget 中。

该索引只保留类型候选、普通 comptime 参数域和声明布局；不遍历程序契约、方法体或
关联项 initializer，也不将候选收录当作 trait 成立的证明。固有 impl 不进入 trait
候选集，但其参数域仍保留。DeclarationWorld 的 callable 索引与规范化查询状态仍由
自身管理，并从共同索引读取参数域，避免再扫描一遍声明 telescope。

目前 driver／required 的外层准备仍有各自的阶段预算，尚未全部计入活动 invocation。
整份源索引的实际构造与内部计费已统一；下面的调用内查询状态进一步消除逐个方法
需求的重复构造，但这些迁移仍不等于所有世界存储、生命周期和查询预算已统一。

工作目录 `/home/fin/luna`，检查入口为 `development/check.py --timeout 900 -- test`；
各 `result.json` 保存完整命令、退出结果和日志路径：

- `/tmp/luna-tools-mqle2z5m/result.json`：退出 0，源索引构造、关联项、实现选择、
  类型查询失败、prepared invocation 与键 session 共 135 项通过。
- `/tmp/luna-tools-dpnrsx23/result.json`：扩大检查退出 101；相关 lib、生成 trait、
  builder、普通 NF 输入、公开库协议、Once、列表／比较与 comptime 内存共 443 项通过；
  resources 单组 67 项通过、298 项在 HIR lowering 失败，全部为未绑定的 bool。
  这些 fixture 未启用 sysroot，写入入口也未调用已有 primitive 绑定辅助函数，
  因此尚未触达资源分析。本批没有据此修改语言解析或资源预期。
- 独立提交 `5ce879f3` 将 resources fixture 接入共同 primitive writer 后，
  `/tmp/luna-tools-m0q22zos/result.json` 退出 0，原有 365 项资源断言全部通过。
  当前相关覆盖去重共 943 项通过；未执行全量回归，也未重测此前记录的前端基线缺口。
- 修改的八个 Rust 文件通过 scoped rustfmt check，
  `/tmp/luna-tools-vjcfur9x/result.json` 退出 0；fixture 的模块排序随后由 rustfmt
  完成，`/tmp/luna-tools-wsd9h_nt/result.json` 退出 0；`git diff --check` 通过。

### Trait 与固有成员共用候选查询

类型索引的 resolve_in_domain_with_search_budget 共同承接 trait 实例与固有成员的
候选搜索、已证明排除、剩余额度分配、精度选择及索引身份记录。trait 入口固定完整
接口实例；固有入口按成员名收窄候选，不伪造 trait 身份。两个入口保留各自的完整
头匹配及能力兑现，删除固有入口重复的搜索／选择循环和独立的半额预算分配。

固定实现头若已被当前类型前提证明与查询主体不相等，可以先排除；不必先求出
该主体是否允许用户 impl。此前只有 trait 查询走这一步，固有方法会先在主体资格
处返回 Pending，从而挡住已有反证的排除。每次排除仍保留可重放的 PTC 证明，
未知／超预算不构成排除，量化头仍通过原有新鲜变量匹配，不反演 opaque NF。
搜索实际未用额度与反证后剩余额度由共同路径分配，未提高默认预算。

真实 sysroot 的例子是一个含 `requires T != Type` 的 comptime helper：
其 `T.struct_fields()` 调用可以排除 represented T 上的 `impl Type` 候选，再通过
普通 Type 值的接收者方法反射。传 Row 成功；传 bool 仍在反射准入时报 Type 错误。
移除不等前提则恢复未决，不能沿用前次查询证据。该约束是验证候选排除的用例，
不是给反射 API 新增的语言要求，也不把 PTC 用作字段布局或程序状态验证器。

局部回归同时运行 trait／固有查询，覆盖 rigid 与 opaque NF 主体的无前提、负前提、
正前提及撤回，检查排除证明和选择证据的重放；预算扫描要求未完成时不能伪造排除。
成员域、precision、世界修订、未知候选与既有调用资格检查继续由原有契约约束。

该批没有修复枚举构造探测过早取得 live NF 输入的独立缺口。直接把 receiver_call
挪到 variant_target 前会改变同名成员的选择并重复准备实参；后续应让调用类别观察
与实际参数准备共享结果，而不是吞掉失败。后续的
[枚举名字观察](values-staging-and-publication.md#枚举名字观察与类型形成分开)
已退役为探测而强求身份的路径；完整调用输入准备的统一仍未完成。

验证目录 `/home/fin/luna`，入口 `development/check.py --timeout 900 -- test`；
完整命令、退出结果及逐组输出在各目录的 result.json / stdout.log / stderr.log：

- `/tmp/luna-tools-pprqndqq`：implementation、inherent、capabilities 共 156 项通过，
  退出 0，包含负前提撤回、正证据重放与低预算恢复。
- `/tmp/luna-tools-s2pgjxab`：真实 sysroot 泛型反射正例及非法主体拒绝通过，退出 0。
  两份生产文件临时还原到 `f98c782e` 后，`/tmp/luna-tools-hq05a6ye` 在相同源码
  130..145 报主体资格未决，退出 101；随后逐字节恢复本批源码。
- 最终 `/tmp/luna-tools-iprrmp6f`：相关 lib、关联项、两阶段实现、receiver 来源、
  NF 方法、公开库协议、反射与失败恢复共 393 通过，退出 101。
  cmtt_methods 的两项既有失败与[上一阶段基线](values-staging-and-publication.md#计算型-type-接收者的类别观察)
  位置及诊断一致；另明确保留并跳过 1 项 live-NF 枚举探测回归，没有修改其预期。
  反射组为 19 通过、1 ignored，包含新增的撤回负前提拒绝检查。
- 四份修改的 Rust 文件 scoped rustfmt check 退出 0：`/tmp/luna-tools-udjr58s_`；
  git diff --check 通过。未运行全 workspace、性能或并发压力测试。

### 调用内的统一声明查询状态

`SourceInvocationQueries` 借用本次 invocation 的不可变 package、Context 与前提，
类型证明、方法或关联项的首次需求到达时才构造 DeclarationWorld。三种服务共享
同一份声明索引；后续方法需求还可复用完成的规范化 Type 视图。required、staged、
公开 invocation、缓存 invocation 与键协议五处生产调用均使用这一作用域，不再让
类型／关联项沿外层 world 查询、方法另建一份索引。公开 invocation 的额外实现索引
已删除。无查询的调用不支付这份索引成本。

构造成功才安装 world，构造失败保留可重试的空状态。拒绝一个成员或命题可以保留
此前完成的元数据，但不保存失败选择或调用许可；每个需求仍检查自身的形成条件和
实现关系。关联项服务返回待交付的初始化程序，不执行它；用户调用契约和程序执行
仍属于 VM，不把用户方法的结果纳入这个查询缓存。

required 的抽象输入恢复可以为单次类型证明提供显式的 binder scope／Context，
而不替换默认上下文、源声明或后续方法／关联项的前提。该证明不写入方法规范化缓存。
一次调用结束即释放规范化与选择状态；下一次调用、新前提或新快照建立自己的状态。
已持有不可变源码的入口可共享[声明目录](world-storage-and-lifetimes.md#12-保活源码的共享声明目录)，
其中不包含上述状态。staged 求值先结束 VM 与查询借用，再进入可变的结果冻结步骤，
不能在改写 snapshot 时保留旧世界。

三种宿主服务统一预留当前额度，包含惰性构造成本，正常／错误返回结算实际剩余量；
单次证明的上限不会吞掉外层未用的额度。unwind 不回填预留额度。类型证明发生构造
失败、超预算或尚未形成时返回 Unknown，不能伪装成已证明的否定；成员选择仍保留 Budget、
Unresolved、Invalid 的分类与源码位置。内部可变状态只承载同步、非执行的声明查询，
不存活动推断变量、可暂停 VM 或资源证明。

资源权限仍由程序验证层的对应服务检查。公开预备调用、缓存调用和 Eq／Hash 协议
现在从持有源码的 DeclarationSnapshot 共享不可变目录，缓存键接纳与实际调用也从
同一 owner 取得索引。required 的发布器证明与实际调用也复用所属 snapshot 的目录，
其资源权限使用另一份源码及其目录，不能与执行声明合并。luna driver 的资源
ProtocolContext 也持有所属 snapshot，分析、执行与 host 查询不再通过它拼接外部
package。runtime 实例化也从 DeclarationSource 复用目录；Context、规范化视图、展开
与图验证仍按构建独立进行。staged 求值已移除可变临时声明，单独检查／执行本次
表达式，从保活声明的 Snapshot 共享目录；当前 Context 与前提仍逐次传入。staged
VM 进一步复用同源 CodeSession 的声明索引；前提展开、抽象参数权限和宿主服务仍
由每次构造重新准入，命名源码调用不能凭缓存代码跳过闭合 capability 检查。
phase 声明目录也绑定同一源码 owner 复用，但表达式检查、存储查询与 unsafe 授权
仍逐次建立；目录不携带本次类型命题证明，也不是完整 package 的验证收据。
跨编辑的持久索引、分块源码和整体准备预算的统一仍未完成，不能把这些生产 owner 扩大为完整 WorldSnapshot 迁移。

以下保留调用内作用域迁移时的证据；后续共享目录的当前检查见
[世界存储第 1.2 节](world-storage-and-lifetimes.md#12-保活源码的共享声明目录)。
工作目录 `/home/fin/luna`，采用 `development/check.py --timeout 900 --`；
各 result 文件包含完整命令及 stdout/stderr 路径：

- `check -p luna`：退出 0，`/tmp/luna-tools-_j31s_2f/result.json`。
- `/tmp/luna-tools-v3lxvq7f/result.json`：elaboration／luna lib 264 项及
  prepared invocation、键协议、staged 复用、调用值模板 15 项通过。该轮退出 101，
  唯一失败是新增测试误把预检即拒绝也要求为非零计费；修正为同时检查预检拒绝与
  部分构造耗尽，不改生产计费规则。
- `/tmp/luna-tools-74ryykst/result.json`：materialization lib 25 项通过，包含
  八个统一查询边界用例。该轮退出 101：新增 sysroot 用例的 `inspect` 与库成员重名，
  公开 CLI 的无包选择器拒绝歧义；用例改用唯一入口名，保留原有歧义检查。
- `/tmp/luna-tools-rj7_46ic/result.json`：退出 0，真实 sysroot builder、公开协议、
  列表 Hash／Eq、排序、两阶段 trait、生成 trait、staged 常量及类型查询共 247 项通过。
  新用例贯通 required／公开／code-session 调用中的三类需求、耗尽后重试及新源隔离。
- 最终去重共 551 项通过；上述两次测试设置修正未改变生产实现，未运行全量回归。
  十三个修改／新增 Rust 文件的 scoped rustfmt check 退出 0，
  `/tmp/luna-tools-urjowomp/result.json`；`git diff --check` 通过。

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

## 11. 原子准入与覆盖声明

不能用 `supports(atom) -> bool` 同时表达形成合法、可以传播、可以证明否定以及可以构造模型。
为每种已注册关系保存以下独立信息；这是内部契约，不是用户注册公理的接口：

```text
RelationContract {
  identity, version,
  input_domains, formation_requirements,
  positive_rules, negative_rules,
  observation_dependencies, proof_checker,
  model_interpretation, supported_fragment,
  incomplete_reasons
}
```

coverage 绑定具体公式和适用前提，不是整个 theory 上一个永久的 complete 标记。
例如 INT 能完整处理当前合法差分约束，不代表随后加入非线性乘法仍有同样覆盖；
有界 BV 电路已编码完，也不代表随附的任意 predicate guard 已得到解释。
已验证的一条推论在其他片段未决时仍可使用，模型成功则要闭合整个待满足问题。

下表是重构时的注册要求，全部为目标契约；不表示这些入口已实现。
具体算法遵循正式[理论分工](../docs/type/propositional-constraints/theory-solvers.md)。

| 原子 | 正事实/传播的依据 | 否定或冲突的依据 | 模型交付仍需闭合的内容 |
| --- | --- | --- | --- |
| A01 Eq | 对应域的身份、受检同余或理论等式 | 不等前提、合法构造差异或等价类冲突解释 | 共享项在各理论中解释一致；用户 Eq 不自动进入 EUF |
| A02 Kind | 描述及明确的类别包含 | 仅使用登记的互斥和封闭描述 | 分类轴与 Type 解释一致，不能任意指派分类位 |
| A03 InUniverse | 已形成类型的宇宙契约 | 对应归属的独立反证 | 保留双宇宙归属，不能令 runtime 与 comptime 互为补集 |
| A04 Qualified | 指定限定的规范描述 | 已完成描述中确实没有该限定 | 限定载荷合法，遵循各自归一规则 |
| A05 HasSlot | 指定种类选择器的存在证据 | 完整适用描述中的缺失证据 | 槽存在、槽类型与结构一致；未暴露不等于不存在 |
| A06 HasHeadShape | 原对象的保留头及来源 | 完整来源形状的排除 | 来源不由 EUF 任意代表替换，不把工厂当逆函数 |
| A07 Matches | 域、模式及正向匹配证据 | 同一合法域内的排除证明 | 值解释满足模式；绑定单独交付，否定没有绑定 |
| A08 Compatible | 相应部分信息关系的已定义规则 | 该关系的明确冲突规则 | Any/any 的部分信息解释不能被当成类型相等 |
| A09 Subtype | 已定义方向关系的证明 | 该关系的反证 | 不擅加转换、协变或新的子类型规则 |
| A10 Implements | Given 或适用世界中的完整选取 | 完整候选空间及排除证据 | 解释与选取视图一致；假值也必须通过世界验证 |
| A11 NotSized | 合法表示的值相关大小证据 | 合法固定表示证据 | 表示前提、目标、必要环境一致；未知大小不是肯定 |
| A12 IntLe | 差分路径或扩展线性证据 | 负环、精确算术反证 | 数学整数赋值满足全部约束，不接受分数替代 |
| A13 IntCongruent | 正常数模数下的整除/整数见证 | 非同余的覆盖证明 | 存在见证的量词和整数性；否定不能选一个失败见证代替 |
| A14 BVUnsignedLe | 同宽无符号语义及电路/直接计算 | 同一合法域内的反证 | 已知正宽度、所有位及合法编码域 |
| A15 BVSignedLe | 同宽有符号语义及电路/直接计算 | 同一合法域内的反证 | 位串与有符号解释一致，不混用无符号次序 |
| A16 RowContains | 具体成员或受检开放尾部事实 | 封闭缺失或有效 lacks 事实 | 行种类、尾部解释及标签身份；成员事实不提供路由 |
| A17 RowSubset | 唯一错误行的全体成员关系 | 有效反例标签及其两侧成员证据 | 开放尾部也满足包含；不用于 effect embedding |
| A18 RowDisjoint | 唯一错误行全部相交可能被排除 | 一个已证共同标签 | 未知标签不能按不同哈希判不等 |
| A19 PredicateCall | 已给事实、准入展开或合法执行 | 同一准入命题的负事实/展开/执行 | 定义、实参、依赖与结果一致；未解释 guard 不能任意赋值 |

A13、A18 可以降为已有公式，不要求专用搜索器；降解必须同时给出形成前提、
等价依据与新鲜见证的作用域。只展开正向定义，未必已经正确处理其否定。
字段类型、关联类型、长度、布局等观察另用观察契约登记，不能因它们不是原子就免除准入。

当前源码的 `Atom` 只有 TypeEqual、IntLe、Kind、Trait 四类；其中 IntLe 的项是
单个变量加常量，算术容量为 i128，溢出返回 ArithmeticRange；这不是一般 INT 已完成。
`classification.rs` 的现有类别表也不能直接扩展成全类型性质的两两互斥表。
`model_search.rs::witness` 遇到 Trait 会拒绝裸模型构造，说明 trait 事实接入和完整模型
接入确实是不同里程碑。行与限定参与现有 Type 等式，不足以证明 A16–A18 已接通。

## 12. 组合覆盖的闭合条件

联合模型交付维护一份随分支回退的义务集合，其元素包括原子解释、共享项一致性、
部分观察前提、域约束、量词覆盖及世界证据。每项必须有具体验证结果或未完成原因。
这份集合不能只由各 theory 自报已处理项组成：协调层从原公式和准入观察出发登记需求，
防止整个关系被遗漏后仍得到一个“全部已完成”的空集合。
若受检化简已证明某项不影响当前问题，可以用化简依据消除对应需求；不能因搜索器未访问
某一分支就自行省略，也不能因此绕过原公式按源语义必须完成的形成检查。

同一逻辑项在不同 theory 中解释不同，需要桥接或重新搜索，不能各自选择方便的模型。
例如以下为一位 BV 域的内部公式，不是新的 Flurry 语法：

```text
x != y ∧ y != z ∧ x != z
```

将三个符号作为不受限 EUF 元素可满足；在只有两个值的一位 BV 域内不可满足。
因此“EUF 没冲突，BV 只看到各自合法的宽度”不是联合模型。已定宽电路必须编码完整
共享等式/不等式，或由具备等价覆盖的组合规则处理；不能只把各片段的成功标记合取。
类似地，行的有限性、trait 实现世界和部分观察都不由普通等式共享自动保证。

布尔层可以对已形成原子分支，但分支值不是新的语义公理。
理论覆盖不足时，仍可用真实理由学习子句；不能把“这种解释我不会构造”学习成其否定。
存在完整反驳即可交付 UNSAT；交付 SAT/反例则必须验证所有相关解释义务。
模型中的有界候选列表耗尽只结束本次搜索尝试，不提供 Type 域穷尽证明。

有限域展开需要覆盖证据，而不仅是容器恰好含有几个值。量词实例化产生的新见证要保留
外围 binder 依赖；`forall A. exists X. X=A` 可有依赖 A 的见证，不能把某次 A 的模型
选出的常量导出为对全部 A 有效的 X。这与第 3–4 节的投影及相同外围环境检查共同实施。

准入检查不能通过把不合法观察写成逻辑蕴含来绕过；条件观察只在源语义允许、对应 guard
已在该分支成立时形成。该分支产生的观察与结论带同一 guard，回跳时一起失效。
最终模型重放必须检查实际采用的分支前提，不能用任意结果反过来证明观察合法。

注册和组合验收先覆盖以下反例：同一类型属于两个宇宙；开放尾部没有已知成员；
一位 BV 的三个两两不等值；同余否定错误复用存在见证；不适用布局的布尔否定；
trait 负分支无完整排除证据；一个已支持原子与一个未解释 guard 混合。
这些反例用于确定重构所需能力，当前没有新增或执行 solver 测试。
