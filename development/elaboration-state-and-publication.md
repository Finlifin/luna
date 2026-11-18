# Elaboration、VM 与发布的状态协议

同一次源调用可能先要从程序取得一个 Type，再用它检查后续源码。检查器持有域、
前提和规则进度，VM 持有执行位置、存储和调用预算，发布接收者持有可见定义。
下文区分当前生产入口与需要接通的所有权交接。
[全链路架构](architecture.md) · [依赖观察](dependency-observations.md)

## 1. 先保留已有机制，而不是重新发明全部状态

| 当前入口 | 已经表达的正确边界 | 还不能承担的职责 |
| --- | --- | --- |
| `type_resolution/src/obligations.rs::Engine` | trial 独立、parent identity/revision 校验、meta watchers、settled 后提交 | 全部 elaboration 状态、宿主请求、代码 hole、候选诊断及不纯执行 |
| `comptime_interpretor/src/bytecode.rs::Frame` | PC、slots、prepared types、调用/默认值/plan 等 Waiting | 通用宿主协议、独立于当前 Interpreter 的可恢复 session |
| `comptime_interpretor/src/invocation.rs` | owned activation、前置条件／主体／后置条件进度、保存/恢复调用方、显式执行栈 | 跨稳定查询的 session 身份、发布后的语义可见范围 |
| `comptime_interpretor/src/code_session.rs::CodeSession` | 精确不可变世界绑定，代码与结果缓存分开，fresh VM | 跨世界通用迁移、活动帧持久化；不应放宽 ptr/world 检查假装支持 |
| `hir/src/definitions.rs::PreparedPublication` | prepare 后草稿冻结、drop 不发布、commit 不再校验/分配/回调 | 一般定义对所有消费者的统一准入和可见范围 |
| `generated_structs/admission_session.rs::DefinitionAdmissionSession` | struct/enum/newtype 共用 revision，foreign/stale receipt 拒绝、预付安装工作 | 活跃调用中的任意 definition 发布与整个实例的 coherence |
| `definitions/deferred_struct_publisher.rs::DeferredStructPublisher` | 首次真正请求才准备世界，未执行分支不做发布准备 | 一般宿主服务与完整定义种类，不应永久按 kind 扩张回调表 |

这些代码仍是生产实现。目标协议要接管实际调用者，不先删除失败保护，也不声称已有
Waiting 字段就完成了正式文档中的局部依赖调度。

### 1.1 签名前提形成的局部进展条件

`ready_signature_capabilities_with_clauses` 仍是局部迭代器，不是下文的通用 task
调度器。它形成显式声明的正前提；某个 atom 的 Type 投影可能依赖同一签名中另一个
尚未形成的前提。例如先写 `A.Item.Item :- Allowed`，再写 `A.Item :- Source` 和
`A :- Source`，不能因此要求声明者按求解顺序排列约束。

关联投影的给定证据查询现在保留 Unresolved 的原因，并标记当前失败仅允许在
capability 前提发生进展后重试。旧 `needs_capability` 已改名为
`retry_after_capability_progress`，避免把任意 Unknown 解释成已找到输入生产者。
Budget 与 Invalid 不获得此标记。成功形成 atom 只有增加新的前提才算本轮进展；
重复前提和只完成解析的其他谓词不能凭空触发重试。

失败尝试仍恢复该 owner 的 capability 表、PTC context 及 pending_type_uses 长度。
当本轮有新前提时再尝试保留的 atom；没有进展且签名须完成时，返回原失败，不将其
转成成功、前提不存在或默认实现。尚在准备参数前缀时保留 atom，后续真实参数供应
可再次进入。缺失、自举和循环前提不能因为队列曾有其他成功项而被接纳。

这是现有源码路径的修正，不是新的 waiter：没有给 Unknown 伪造 producer ticket，
没有改变候选世界或方法选择；通用事务、精确依赖登记与跨重试累计全部查询成本仍待
按后文实施。该循环恢复的三类状态也不能代表整个 Checker 已实现事务回退。

签名前提的回归覆盖三层前提的排列、缺失前提、自举、无正前提的析取及同层非法
atom。查询预算和阶段目录复用另见[值准入与预算](comptime-value-admission.md#预算与复用)。

## 2. WorldRevision、可见范围与 coherence 分开

[normalization 的开放定义规则](../docs/comptime/normalization-and-instantiation.md)
规定：loose 环境的 implementation、derivation 按线性可见性使用；整体实例化时，
生成定义与相关静态作用域的已检查定义一起接受 coherence 检查。
[包规则](../docs/code-organization/packages-and-umbrella.md)另限定合法依赖及实现世界，
不能靠减少 use 隐藏已在该世界内的冲突。

因此拟定的查询视图为：

```text
SemanticView {
  world_revision,
  lexical_scope,
  instance_identity,
  visibility_frontier,
  candidate_universe_stamp,
  phase,
  target_context
}
```

WorldRevision 描述物理数据版本；visibility_frontier 描述当前语义位置允许看见哪些
线性生成项。两者不是一个计数。物理目录中已经存在某定义，不表示较早位置或其他实例
可以使用它。静态声明的可见性也不能由“哪个检查任务先结束”决定。

候选 universe 由合法包/静态作用域、phase 和该 frontier 确定；记录完整的检索范围，
包括未命中的桶。前缀 A 内选定的证据带 A 的前提，新发布形成前缀 B，不能把 A 的缓存
换上 B 的标签后复用。对于 A 处已经合法完成的源执行，不因 B 出现而重新执行。

整体 coherence 的集合可能大于某次线性使用的集合。一次发布 receipt 证明该批在当前
输入下可安装，不证明所有未来生成项都相容。实例 owner 保留最终 coherence 义务；
未完成或失败就不能交付实例/产物，但不假装能够回滚已经发生的源程序外部行为。

静态源码编辑产生新构建输入；旧构建中的执行进度和线性 frontier 不能直接迁入新构建。
这一模型保留既有语言规则，不引入“每次最新全局世界就是正确世界”的调度依赖。

## 3. 活动状态的所有权划分

当前 `elaboration/src/program.rs::Checker` 的字段需要按如下责任拆分，不按字段名称机械移动：

| 代表字段 | 目标 owner | 发布边界 |
| --- | --- | --- |
| package、package_identities、target、protocols | WorldSnapshot / SemanticView | 稳定只读输入，不能随候选覆写 |
| signatures、requirements、defaults、checked_traits、checked_implementations | 定义生产者及其完成结果 | 已完成部分进入稳定世界，活动前缀不与其共表伪装完成 |
| active_signatures、active_constants、active_required | 请求 owner 的求值/观察栈 | 记录具体正在生产的 facet，退出后移除；不能只以 owner 相同判循环 |
| normal_input_prefixes、signature_prefixes、signature_domains | SignatureTask | 前缀可供合法观察，不能当完整签名导出 |
| locals、captures、loops、result、return_observations、effect_environment | BodyRegion / ControlFrame | 嵌套进入显式帧；退出交付结果和证据，不把临时上下文留在共享 Checker |
| context、pending_type_uses、pending_refinements、候选替换 | CandidateTrial / Region obligations | 前提、赋值、term result 与诊断决定一起提交 |
| code_values、pattern_codes、generator_cells、code_method_captures | 当前代码构造/执行 owner | 声明模板可共享，实例捕获、可变 cell 和实际 CodeValue 不能共享误用 |
| completed_aliases、open_instances、implementation_index | 带完整语义依赖的缓存/索引 | key 包含相应视图与输入；完成结果和 in-flight task 分开 |
| next_definition_identity、generated_functions、expansion_sources | DefinitionTransaction / 来源 owner | 成功发布才交付新定义；来源在失败诊断需要时仍由 owner 保活 |
| required_steps、alias_expression_budget 等 | WorkBudget 的具名子额度 | 消耗跨重试累积；失败不退款已做工作，回收另有保留容量 |

某字段同时放了稳定和活动数据时应拆其语义，不是简单包进一个更大的 Context。
struct 中没列出的字段仍需逐一归属；本表是拆分依据，不是穷尽迁移清单。

### 已接入：required invocation 的整批定义接收

VM 内完成 builder 发布，不等于调用者 Checker 已接收其描述。此前
`invoke_required_publishing` 逐个支付复制成本并写入 struct／enum／newtype 表及
PTC 构造器权限；后一个描述超预算时，早先条目已可见，implementation 布局索引
却尚未同步。由此不能把单次 VM 发布成功当作跨层事务已经完成。

现在生产入口交给 `required/publication.rs::Receiver`：先用同一调用方余额完成
全部描述的传输预检，再安装定义表、构造器权限及布局索引。失败不修改这些接收方
状态，已经消耗的工作不退款；builder 已保留的身份编号也不回退。生成描述仍先由
原 builder admission 校验，接收器不重复求值、不重新分配身份，也不创建第二套
形成规则。这里保证可返回错误的接收失败原子性，不回滚此前用户执行的外部行为。

具体描述发布后，包内其他调用者可以使用其字段、变体载荷及 newtype 表示。
这些类型的形成条件必须能从包级声明证明。显式 `luna comptime` 的执行 VM
与发布器共用该次调用的源码查询服务，按需选择实现并从当前请求扣除证明成本。
源码内的 required invocation 在计算 builder 实参时可以使用调用者的词法前提。
发布器读取同一源码快照的声明，另以全局构造器理论和空局部前提检查具体描述。
因此某条分支独有的证据可以用于求值，却不能随发布的具体类型逃逸到包级作用域。

整批接收失败时保留原条目、构造器权限和索引版本。实际调用在同一包里消费刚生成的
字段、变体与表示，可以检验后续检查获得的是完整目录。[发布验收](../compiler/luna/tests/required_definition_publication.rs)

### 已接入：晚生成描述交付给后续 IR

接收成功还不代表最终 NIR 包完整。初始描述快照取得之后，主体、默认值与模板中的
required invocation 仍可生成定义。原收尾只补收 struct 和内部 scalar packet enum，
遗漏普通 enum 与 newtype；局部 `const Flag = Wrapper()` 后执行
`true.as(Flag).as(bool)`，会在实例化阶段报 `IIR nominal type has no declaration`。

现在三类描述共用按声明身份去重的交付规则，保留包内先前条目，并在契约、纯度和
quote 分析前补齐 checker 已接收的描述。内部 scalar packet 只另记传输角色，
不再获得另一条 enum 描述收集路径。此处不重新调用 builder、不修改生成身份，
也不以布局代替类型身份。

成员查询另有一条一致性边界：enum 命名空间、普通／代码方法及关联项共用
`is_type_receiver` 观察接收者域；Type 可以来自局部值，不只来自声明语法。
`is_type_head` 仍负责其原有语法分类，不能借此把普通值引用折叠成 TypeValue。
域是 Type 也不代表身份已经完成：enum 仍先观察已知命名空间，真正选中成员后才走
原 required 类型形成；分组不是值，载荷仍按普通构造契约求值和检查。

例如 `const Event = EventType(); Event.data(true)`，晚生成 enum 的命名空间须保留声明身份
和变体载荷。局部变体、显式成员构造与匹配都使用相同的接收者域判断。quote 内待决
Type 与一般候选试探中的生成定义仍需各自的所有权交接。

## 4. Region 与 task 状态

Region 的目标状态为 `Open → Closing → Complete | Failed | Cancelled`。
Closing 不接收无关请求，但允许完成已有任务必需的子请求；必检结果与语言允许的
residual 均可交付时才 Complete。空队列本身不证明完成。失败或取消回收 waiter、
候选及执行所有权，不发布成功实例，也不承诺撤销已经发生的外部效果。

任务身份为 `(RegionId, TaskSlot, Generation)`；单线程同样需要 generation，防止
槽位重用后接收旧通知。完整 Region 协议尚未实现，不能把现有 Task 枚举视为其替代。

| 状态 | 持有内容 | 下一步 |
| --- | --- | --- |
| Ready | 检查后继、输入描述及所属作用域 | 取得唯一写权后 Running |
| Running | 当前规则推进的写权 | Ready、Waiting、Resolved、Unresolved 或 Failed |
| Waiting | 后继、观察票据、真实生产者及代次 | 有效响应到达后继续，或失败／取消 |
| Resolved | 本代结果与依赖证据 | 本代终态；证据失效后另开一代 |
| Unresolved | budget、unsupported、missing-premise 等原因 | 条件实际变化后重试，或按语言规则提取 residual |
| Failed / Cancelled | 诊断来源与尚待回收责任 | 回收后终态 |

Waiting 与 Unresolved 不混用。没有就绪任务时，先区分外部请求、已失败生产者、
额度不足和未迁移规则；只有实际生产依赖闭环才能报告循环。AST 父子关系不是循环
证据。补充调度预算不补充 VM 执行预算，也不使失败的同步检查自动具备恢复能力。

### 当前生产接线

路径均相对 `compiler/`；这些入口构成接下来修改的依赖闭包。

| 入口 | 已有责任 | 未完成部分 |
| --- | --- | --- |
| `elaboration/src/program/block_regions/driver.rs` 与 `driver/region.rs` | 普通、contextual、部分结构输入共用 Region；帧栈保留子请求；根消费者交付表达式、声明输入或普通调用的依赖证据 | 仍使用原活动 Checker，没有可移交的外部等待权限；模式／guard 子检查仍同步 |
| `elaboration/src/program/branch_probes.rs` | 探测次序、默认类型策略和 Attempt 回退；if／when／match 帧持有搜索位置与当前事务 | 不提供执行前缀恢复或外部 Type 等待 |
| `elaboration/src/program/dependent_inputs/tasks.rs` | 依赖帧持有准备结果、活动路径、默认后继与待检查槽，poll/accept 交接源码子检查 | 所需参数的子检查已进入共同 Region。完整普通调用及原 VM 交付 Type 仍待接通 |
| `elaboration/src/program/declaration_inputs/progress.rs` | producer、身份、检查次序与源码 schedule 分开 | Typed 路径仍要求普通完整 Expr，Contextual 仍保留 NormalFormArgument |
| `elaboration/src/program/dynamic_instances.rs` | callable packet 保留实际 producer、静态身份与 pending 集合 | 签名仍提及 pending 参数时直接拒绝发布 |
| `elaboration/src/program/function_bodies.rs` | 最终函数与 required 依赖共用主体检查 | 必须检查出完整 body 才能构造 NIR Function |
| `elaboration/src/program/required.rs`、`required/prepared.rs` | 完整根、输入准入、冻结声明世界和代码缓存所有权 | build 必须返回完整 Expr；没有源区域的增量根 |
| `comptime_interpretor/src/invocation/{host,session}.rs` | 同次 activation、契约进度、方法／关联／定义宿主请求 | 没有取得下一段受检源码的生产入口；Fragment 根仅供测试 |
| `comptime_interpretor/src/bytecode/bindings.rs` | 帧持有导入／导出绑定，完整验证后移动交付 | 不提供源作用域、外部控制出口或任意切点许可 |

#### Type 需求预检的独立测试模型

`627ab804` 之后的调用点核对确认：旧 Type Session 已没有前端生产消费者。
它现位于 `compiler/elaboration/test_support/type_computations/`，仅由测试显式
加载；生产 elaboration 不再导出这个模块。下述预检和签名 API 是独立协议模型，
不是普通 `defer` 已有的挂起路径；实际所有者仍见上表的 required 与源码区域。

测试模型的 `demand.rs` 对单输出、依赖恢复及同一注解的多个根共用有界预检。
计划保留依赖边、根归属、外部等待和已失败生产者；完整预检前不进入生产者。
共同依赖只执行一次，失败沿原边传播，无关兄弟可独立使用。后续根已有错误或循环
时，不能先运行较早根，再发现整体不可交付。

`resume_bounded` 只恢复指定输出，`resume_dependencies` 才驱动其生产者。
SchedulingBudget 保留等待和已交付输入，不将队列改为 Failed；循环终结相应需求。
便利入口的 100,000 步是整次调度的共同额度，非每条边的新额度；VM 额度另计。
计划只在独占推断 context 的操作内有效，不进入声明快照或 CodeSession。

签名恢复 API 的直接测试不能代替源码接线。尤其
`resume_instantiated_signature_annotation_checked` 与
`resume_runtime_signature_checked` 不会自动接管 `dynamic_instances` 的拒绝分支。

#### 控制与输入事实

`program/control.rs::Context` 拥有返回目标、部分返回上下文、待定返回点、返回观察和
循环目标。独立签名、默认值、required、quote、generator 与闭包边界整体移出／恢复
控制状态；闭包不继承外层返回目标。computation 仅替换相应返回目标，handler 保留
安装处的词法控制目标；type observation 保留所需循环可见性并隔离返回及捕获证据。

`input_facts::State` 集中保存输入类型、值、部分观察、特化、派生来源和 supplied scope。
三种退出必须分开：

- 失败或不提交的内部试探恢复完整事实 owner，不能漏掉新增字段。
- 块／迭代的词法退出删除内部绑定，保留外层值在写入／捕获之后发生的失效；恢复旧
  快照会错误地复活这些证据。展开迭代共享边界，失效跨迭代累积。
- 开放函数继承契约允许的 Type／value 绑定与 supplied scope，隔离调用者的活动
  特化／partial／派生来源；完成后归还调用者状态。

控制上下文、输入事实及部分观察的 dump 保留不同种类、源位置和转义后的名称；
不能把未完成输入只打印成一个不含归属的 pending 标志。

这些是检查事实，不是 VM 值环境。声明需求、预算和真实 invocation 效果不随事实
快照回滚。PTC 只管理类型命题；程序状态、内存及资源规格仍属于程序验证层。

#### 检查帧与探测

源码区域持有帧栈、受检前缀和当前子请求。父帧记录请求的源码位置、目标域和完成后要做的事；
只有等待中的位置能接收一次结果。失败先经过内层的作用域出口，再交给拥有探测事务的帧。
每次推进按实际规则计费。dump 以 S-expression 展示阶段、等待项、已检查的操作数和返回位置。

**词法作用域。** 进入块时保留父局部类型、表达式代码、模式代码的三个绑定根，
以及 PTC 前提和输入事实的边界。每棵绑定树按 LocalId 的 owner、index 排序；
保存根只增加共享引用，写入只更新键路径上的分支。类型与引用的代码值保持原有身份。
块退出时恢复父绑定与前提，同时保留块内写入对外层输入事实造成的失效。
当前输入事实边界仍需遍历成员并计费。声明实参准备只保留自己会恢复的局部类型根，
另行保留调用者的默认环境、参数域和签名 owner。

**候选与存储。** Attempt 独占一次提交或恢复权。子规则缺少上下文时，Probe 可在
还原控制、捕获、默认值、输入事实及候选私有绑定后继续搜索。域错误、预算耗尽和恢复失败
向外传播，已花的预算不退还。生成器存储检查点单独保留 cell 身份及所拥有的值图；
词法绑定版本不授予访问另一 invocation 的权限。生成器的普通成功出口保留已有代码变量
的新值，丢弃子作用域新增的名字。失败出口恢复进入时的词法版本与原存储。
Commit、Probe、RetainCode 分别遵守各自对构造副作用的交付规则。

| 检查位置 | 帧必须保留的语义 |
| --- | --- |
| 块与结果交付 | 语句完成后才建立后继事实。不可达源码仍要检查。OrdinaryTail 与 ContextualTail 共用引用计量、绑定物化及别名收尾，各自产出 Expr 或 NormalFormArgument。NoReturn 直接传递，可选与错误限定类型保留包装层次。 |
| if、when | 保存真假前提、当前分支域、inline 选择、callee 位置与 effect 交付责任。后续条件继承先前条件为假的事实，汇合时退出分支前提。缺省分支及语句位置遵守各自的 Void／Never 规则。 |
| match | selector 只检查一次。每个 arm 保留模式、guard、受检主体与独立词法出口，再按原顺序检查覆盖及清理。inline 只交付选中的主体，所有 arm 仍须通过检查。 |
| 部分输入域探测 | 先找实际类型证据，再按规则允许数字默认值。Never 和只有错误出口的分支不能确定成功域。候选回退后仍要按所得上下文正式检查。 |
| 比较、算术与一元操作 | 单独保存各操作数、peer 搜索、原生域与 trait 域的试探结果。关联 Output 可以不同于输入域。比较结果的 bool 不给操作数定型，负数字面量连同符号形成。 |
| and／or | bool 右侧在短路前提下检查，成功与失败都退出此前提。BitVec 保留双侧求值。每个操作数仍使用原函数的控制出口。 |

静态身份观察可以迭代跟随已知分支与块结果，未知调用或写入不能凭观察取得值。
模式、guard、inline 选择及其余同步规则仍依赖活动 Checker。保存词法根解决了保留检查位置时的绑定复制；
将 VM 产生的 Type 交回尚未检查的 HIR，并允许原 invocation 在原处继续，仍需要源区域和执行现场共同持有交接点。
端到端的次序、退出及失败条件见[待定域的普通调用](refactoring-plan.md#下一条纵向交付待定域的普通调用)。

#### 输入路由与域依赖

一次调用同时有声明槽位、源码求值顺序和域依赖。调用者先写 `.values = [byte]`、后写会给 `byte` 赋值的 Type 生产者时，依赖检查可以先找 Type 槽；执行必须读到赋值之前的 `byte`。默认实参在未显式供应时才运行。静态观察取得一个身份以后，已经检查的生产者仍留在原求值位置。[输入次序与快照用例](../compiler/luna/tests/type_input_execution_order.rs)

| 所有者 | 准备的内容 | 交付时机 |
| --- | --- | --- |
| 源码输入路由 | 将具名、位置、展开和完整参数包分配到目标接口的槽位，保留源码位置。空展开也可能产生副作用。 | 包的表达式通过检查后，保存各字段及其绑定。路由失败保留对应诊断。 |
| 声明实参准备帧 | 持有路由、域依赖、收集进度、调用者环境和最终结果的消费者。声明 NF、名义类型、别名、函数实例和动态 NF 的位置参数共用这条路径。 | 在同一检查区域内把普通期望域、部分结构上下文或待定上下文交给子帧。退出当前观察、默认供应环境和调用者作用域之后，消费者才取得结果。 |
| 普通调用的依赖帧 | 持有所需参数及其默认值、活动依赖路径和已观察的输入。按目标槽位取得原始 HIR，收集和 tactic 参数保留各自在本次调用中的语法视图。 | 在同一个区域协议中检查所需实参。只有该子检查进入 staged 上下文；交付或失败都先恢复父上下文。全部依赖完成后一次交还原调用准备结果。 |
| VM 的原调用 | 持有已执行的前缀、值、局部存储、控制出口及本次执行的预算。 | 目前已检查的声明实参可以在域就绪时继续。普通函数的源码实参等待执行产生的 Type 时，还缺原 VM 与源码检查区域之间的交接。 |

声明准备持有临时望远镜、准备语句与最终的执行计划。显式实参引用调用者绑定，默认供应引用选定声明和父项的绑定。处理依赖时先在原始 callee 域中找待定槽，再用已有实参一次代入，保留调用者合法的开放参数。递归调用中，实参带有和被调声明同号的参数仍可成立。域检查与复制共用预算，越界或预算耗尽会终止准备。[槽位检查](../compiler/elaboration/src/program/declaration_inputs/slots.rs) · [身份观察](../compiler/elaboration/src/program/declaration_inputs/observations.rs)

收集器保留各元素的源码位置、已检查字段和非连续元素的准备语句。缺上下文的探测由发起探测的帧恢复；已知类型冲突、形成失败和预算耗尽须继续报告。失败从最内层词法帧退出，恢复默认值或 staged 输入的环境，不回滚已消费的预算，也不撤销其他已提交的存储事实。[收集规则](../compiler/elaboration/src/program/declaration_inputs/collections.rs) · [子帧分派](../compiler/elaboration/src/program/block_regions/driver/children.rs) · [恢复与交付](../compiler/elaboration/src/program/block_regions/driver/region.rs)

普通参数的默认值可引用较早完成的输入。检查器保留原 Local 读取，只有再次进入本次活动路径才报告依赖环。对于 `do { mark(); U }`，即使最后的 U 能提供类型信息，也必须在调用时执行前面的 `mark()`。如果依赖检查不能静态观察其身份，普通 comptime callable 的准备仍缺执行许可；另开 required invocation 会改变副作用次序。runtime 函数局部常量的必需计算有其独立入口。[普通调用域依赖](../compiler/elaboration/src/program/dependent_inputs/tasks.rs) · [默认值用例](../compiler/luna/tests/ordinary_input_identity.rs)

公开参数名与默认供应跟随目标调用接口。固定参数包的宽度由已检查布局确定，位置路由保留完整具名供应与逐项收集的区别。高阶调用的公开名字及默认供应的传送边界见[可调用输入接口](callable-input-interfaces.md)。

#### required 世界与绑定交接

`required/dependencies.rs` 使用 `nir::visit` 发现函数、trait 方法与关联值依赖，覆盖
根、闭包、模式、默认值、契约及保留的 contextual 节点，完整扫描后才发布集合。
允许保留不含表达式子项的 CodeEvidence 叶用于发现依赖，不授予它执行许可。
自由 LocalId 在 initializer substitution 后观察一次，仅在后续结构不变的本根中
复用；不是跨根缓存，也不允许先捕获所有外围值。

帧的 `Exports` 在创建请求与实际开始时检查名字冲突；解析到该 Program 的寄存器，
预算与全部检查通过后才移动入口环境。Frame 自己持有绑定交接，正常值出口要求已
终结、无 pending 请求、内部模式作用域闭合、所有输出有值且无环境冲突。先整体验证，
再移动任何新输出。错误或非局部控制不发布新绑定，但归还实际已变更的原输入。

这些是生产输入准备计划已有能力，仍缺源 scope、域准入和执行 receipt。Program
寄存器编号是片段私有的，不能直接把 slots／PatternScope 搬到另一 Program。
`restore_environment` 会退出帧内部作用域，不能用作任意源码暂停点。

受检实参也能从自己的代码片段退出外围循环。例如在 `while` 里构造类型，
第一个实参写入计数器后执行 `continue`，这一轮的剩余实参都应跳过，下一轮
读取已经更新的计数器。elaboration 已将跳转解析到词法循环的 LocalId。字节码
在片段内直接跳转到本地循环；目标属于外围代码时，把该身份随控制结果一并交还。
可复用的实参代码只引用词法身份，实际跳转地址由挂起它的 Program 提供。

invocation session 区分私有执行环境、声明的默认参数或契约环境、从调用者借入的
词法环境。输入计划退出时，先归还借入的绑定，再按循环身份和挂起指令的位置查找
目标。拥有循环的帧退出多余的作用域，`break` 恢复到出口，`continue` 恢复到
循环头。中间的输入片段继续向外交付。函数 invocation 只接纳正常完成和函数返回，
阻止跳转跨入另一调用。循环目标表只保留包围输入挂起点的循环，空间随这些循环的
数量增长；同一循环内的多个实参共享一条记录。

这条通路处理已经生成 NIR 的实参及准备语句。等待 Type 才能完成源码检查的实参
仍需要由父区域保留检查位置，并向原执行区域交付一次执行许可。

#### 所需类型计算的执行入口

`PreparedRequired` 的调用分两步。准备阶段的 `required_source_closure`
从捕获后的入口和实际选中的参数收集依赖。入口先保留原签名的槽位顺序，再从局部读取和
类型中的绑定槽出发，沿参数域递归选择所需槽位。例如，结果类型可以引用一个
`witness` 值，而 `witness` 的域又引用一个回调。即使主体没有直接读取这两个局部量，
本次调用也需要回调的声明。显式提供的活动值优先于外层为同一绑定保存的快照。

冻结参数提供可执行声明的引用。对选中的活动参数，一次遍历其值图，分别记录可执行声明、
携带的 Type 绑定和使用到的值绑定。函数实例的实参和名义值携带的类型实参均参与作用域
观察。一个不透明类型出现在元数据中，只需保留其身份。传递抽象值时，从调用方已经持有的
值绑定中选出本次所需项，VM 准入仍会核对值的域。未选中的活动参数无需读取其存储。

延迟类型注解能否继续检查，取决于本次调用还需要哪些外层子句参数。举例说，入口取到
`keep<T>`，入口处的 `T` 来自调用方，而 `keep` 自己声明的形参只在该声明内绑定。
准备阶段分别检查保留的函数体、默认值、契约以及无主体函数身份的签名与形成条件。
子句参数的域按绑定身份从阶段检查已接纳的目录取得，因此实现从容器继承的形参也有
同样的域证据。每个声明先扣除自己绑定的形参，再合并自由输入。这能保留只在被调用
函数中读取的外层值，也避免把另一个声明的局部形参当作注解的等待条件。

这份自由输入集合属于不可变的源码快照。第一次查询成功后，注解检查和抽象值作用域
筛选共享它；预算耗尽的查询可重试。每次调用仍检查自己的捕获值和结果域。同一准备
派生出的调用共享只读的词法假设、参数替换和作用域帧，各自绑定本次供应的捕获值。

输入齐备后，`start_prepared_invocation` 与 CLI 共用同一个 VM 驱动。关联项、方法选择与定义
构造的响应交回原 activation。取得完成值之后，调用方恢复源作用域。

**方法声明和默认实现可能共享一个声明 ID。** 源码中保存方法值时，应保留方法契约；
实现表确实引用默认体时，才加入该定义。工作队列按“用途与 ID”去重。关联项的
初始化器可以继续引入其他关联项，即便中间没有普通函数引用也要走到闭包完成。
已访问的依赖不必在每次迭代后重新扫描。队列使用当前 required invocation 的
准备预算，库中可达声明的个数没有额外的 256 项门槛。

例如，一个已实例化的 comptime 函数可以在子句参数中携带另一个函数。无论入口
取得的是 NIR 函数实例、冻结的函数值还是 VM 中的活动值，准备端都应保留里面的
可调用声明。作为参数传递的 normal-form Type 只携带身份，发现依赖不会展开它的头。

Checker 在检查实现之前建立按身份查询的 trait 契约目录。同一个目录供方法值恢复、
实现检查和 required 源码闭包使用。方法位置在该世界内固定，关联项的默认值可在
一致性检查后的阶段完成。开放签名的验证片段仍有自己的 phase 与算术检查。

当前仍须在 VM 启动前完成根表达式和所需的 comptime 主体。涉及方法分派时，
准备端按接口保留候选实现的定义，VM 根据实际输入选择。未完成源码的按需交付仍待接通。
格式参数使用的 `refines { Itself :- Display } Any` 已可随 Type 描述交付，
并在调用处逐实参检查。

真实 sysroot 的回归用例见 `compiler/luna/tests/required_input_scopes.rs`。
其中一个 Type 计算依次调用 260 个小函数。另一个把 `Forward<Flag>` 和
`Forward<Byte>` 交给同一个 `value: defer choose()`，两个调用分别得到
`bool` 和 `u8`。[格式化演示](examples/host-println.fl) 使用同一条公开的
`println` 签名，传入编译期产生的格式串，由它决定运行时字段的个数及约束。

#### 开放签名片段的可调用声明

检查带 `defer` 域的定义时，片段需要解析其中引用的函数。以
`measure(comptime flag: bool, value: defer Pick(flag), comptime .length: fn(Symbol) -> u64 = size)`
为例，若 `size` 绑定了一个编译器操作，检查器已为这个可调用值生成普通函数体。
片段按实际引用纳入这份定义及它的依赖。若调用的是只有签名可用的 runtime
源码函数，则纳入它的签名。保留的 trait 方法前置、后置条件也参与依赖收集。
先前这两条收集路径独立遍历，会为 `size` 的同一身份同时发布函数体和签名，
导致合法的调用在 phase 检查时报重复身份。

开放函数实例在检查主体前预留身份，递归调用由同一个键取得该身份。
主体和参数目录分别按发布与预留顺序保存，按身份查找主体。嵌套实例化失败时，
检查器只撤销本次尝试创建的预留、参数目录和主体。外层失败也会撤销已成功的
内层工作。已发出的数字身份不再分配给下次尝试。

源码回归覆盖默认生成回调、显式替换回调和两种实际参数域，见
`compiler/luna/tests/compiler_operation_bindings.rs`。片段仍需事先完成根表达式，
并复制名义类型及 trait 元数据。

### 源检查与执行的交接目标

以下协议尚未实现。它接管上述真实入口；不增加第二套可执行 HIR、不扩大
NormalFormArgument，也不把只有手工构造者的 fragment API 当作源码交付。

#### 父区域与待形成 callee 分开

`identity<Byte(), true>(9)` 等待的是 callee 的签名，但执行 Byte 的 owner 是
**调用者当前源区域**，不是尚未形成的 identity invocation。若先建立 identity 的
activation 来取得其自己的参数域，就已经绕过了调用准入。`count` 的输入域等待也
发生在其调用准备阶段；count 主体、入口快照和程序前置条件不能因此提前开始。

required 根没有已进入的源函数时，由本次必需计算建立根源区域，持有合法的入口
绑定与结果交付目标。函数体内部则由已经进入的调用持有该区域，返回仍指向原函数。
二者都不能为每段生成一个便利函数，再把多次 invocation 拼成“同一次执行”。

定义检查、必需计算和已经进入的 invocation 是三种许可来源，不由一个 comptime
标记推导。normalization 可以执行当时要求完成且输入具备的计算；未调用定义和未选
路径仍做可做的检查，不能擅自执行普通主体副作用。ordinary／loose 与执行阶段是
两条轴，遵循[正式规则](../docs/comptime/normalization-and-instantiation.md)。
合法 defer 的 Any／refinement 规则继续成立；这里禁止的是用占位 Any 谎报已完成域。

#### 所有权与规则推进

| 对象 | 所有者与可交付内容 | 禁止隐含携带 |
| --- | --- | --- |
| 稳定源码模板 | 源码服务；声明身份、owning snapshot、接口及可保留义务 | 当前 Checker、活动 HIR 借用、VM cell |
| 活动检查区域 | elaboration 会话；HIR 引用、词法／phase、规则后继、子结果和候选 | 克隆的另一套 VM 值环境、全局 AST 地址等待表 |
| 执行区域 | 原 VM 驱动；activation 身份、存储、执行位置及控制目标 | Checker 或可任意修改的宿主寄存器表 |
| 片段许可 | 指定区域／代次／执行位置、完成的域及前提 | 仅凭 HirId 或 pure／Type 结果签名取得的执行权 |
| 受检片段 | 普通 NIR／字节码、输入输出绑定、控制接口和声明世界证据 | 未检查节点、活动候选、waiter 或假类型 |
| 观察响应 | 原区域结果、接收世界／代次、Type 准入与发布许可 | 无来源 Term 赋值、可变值的永久静态身份 |

规则后继保存算法进度，引用原 HIR 与已检查子结果，不复制出另一份求值树：

```text
CheckTask(region, node, scope, target-observation, rule-state)
  -> NeedChild(task, result-slot, continuation)
   | NeedObservation(key, generation, continuation)
   | Checked(result-slot, code, domain-evidence)
   | Failed(diagnostic)
```

区域、规则后继、票据与片段交接均须有稳定的 S-expression Display，展示 source、
region／activation、代次、当前执行位置、待定观察、受检子结果和未退出 scope。
字符串与符号正确转义，不以宿主地址区分两次执行；需要世界才能展开时提供携带
上下文的 Display 适配器。dump 不是缓存键，也不赋予活动状态持久化能力。

Checked 不表示已执行。同步兼容入口驱动同一规则到完成；未迁移的叶规则允许暂时
同步调用，但遇到真实外部等待时必须报告接线缺口，不能重试整个根或改走 HIR 解释。
词法状态在借用服务时安装、步骤退出时归还；等待保留当前 scope，真正离开时才裁剪。

首次状态拆分须覆盖 locals、immutable、input facts、alias／default bindings、
control、captures、phase、numeric defaults、callee 交付及 pending Type uses。
声明服务和已提交目录按世界观察，active_required／active_signatures 属于活动任务。
PTC／candidate 记录所属前提；generator cell 与 VM slot 的实际内容仍归执行 owner，
不能靠克隆与恢复 Checker 合并真实值更新。

#### 检查依赖与执行次序

域检查可以利用后一个实参提供的类型信息。运行实参则受源码位置和控制流约束。声明中的参数引用确定依赖哪一个槽，调用者提供的值决定该槽完成后如何代入。递归调用中，同一个声明的外层绑定可以出现在内层实参里。等待关系、求值快照和完成槽位的例子见[源码次序与完成槽位](comptime-value-admission.md#源码次序与完成槽位)。

普通 callable 的形成有三个检查点。

1. **定义检查。** 在本次证明内为待定输入分配符号。一个输入在不同位置共享同一个符号，多个符号与已有实参同时代入声明域。这样，即使主体从未执行，若已知 `A != B`，仍可拒绝向 `Pair<T, T>` 供应 `Pair<A, B>`。证明使用调用者的前提，查询结束后释放私有符号。预算耗尽或命题无效时，保留对应失败。[域检查](../compiler/elaboration/src/program/dynamic_instances/domain_checks.rs)
2. **输入执行。** 原调用负责取得每个值。先读可变局部再执行会写该局部的 Type 生产者，须保留读到的旧值。已定型的子句参数包能做到这一点。普通 callable 中，需要等待计算域才能定型的源码子表达式仍缺少交接；[顺序、快照和早退用例](../compiler/luna/tests/type_input_execution_order.rs)保留了这条路径的成功预期。
3. **实例形成。** 输入齐备后检查实际域与形成条件。comptime callable 在 VM 中形成函数值；runtime callable 的 required 路径取得已执行参数包的值和受检类型，再完成代入与比较。若 callee 签名本身依赖尚在执行的 Type，仍需要将结果交还原检查帧。例如 `identity<Byte()>(9)` 应在 `Byte()` 完成后以实际类型检查 9，并让同一次执行取得的实参形成 callable。[实例形成](../compiler/elaboration/src/program/dynamic_instances.rs)

类型契约证明从本次恢复的 PTC 上下文取得前提。目标或前提依赖 trait 时，查询在剩余额度内取得实现证据。结果区分成立、拒绝和未知；证据属于查询上下文，跨调用复用声明元数据时仍需检查实际前提。

#### 片段边界、控制与取消

切点先限定为片段内部模式作用域已闭合的位置；源块外层 scope 由原执行区域保留。
guard 或子调用中等待时保留原 Frame，不调用 restore_environment。交接表包含
LocalId、域、访问／移动方式、输出绑定及其源 scope；先验证重复输出、错域、错 scope
和过期位置，再移动。未初始化、已移走、活跃值引用保持不同状态。

输入片段中，外层循环的 `break` 和 `continue` 已能按词法循环身份返回挂起输入的字节码位置。一个函数的目标只能由该函数内的片段恢复。`return` 走原函数的控制出口。这些规则也约束将来的源码交接；效应恢复另需检查 handler 和 continuation 的权限。[循环目标](../compiler/comptime_compiler/src/lib.rs) · [输入片段中的控制流](../compiler/luna/tests/type_input_execution_order.rs)

请求沿 Authority／Ticket 校验区域、代次与执行位置。同一 HirId 的递归调用和循环
迭代具有不同执行身份。重复、跨区域、旧代响应拒绝时保留当前 waiter；预算不足不
重置执行位置，不自动无限重试。取消不执行后继、默认值或退出契约，已发生的外部
效果不回滚。内部 Probe 不是源执行事务。

### 声明接口与主体发布目标

NIR Function 继续代表已检查产物；不能给 body 塞占位表达式或活动源码句柄。
源码服务分别持有接口与模板，活动任务归当前区域，完成片段才可进入执行器。

当前受检 NIR 已将声明字段收拢为 `program::FunctionDeclaration`：它持有身份、
签名 telescope、阶段／责任、参数绑定、形成条件、程序契约、入口快照及默认值。
`Function` 只组合这份 `interface` 和已检查 `body`，没有第二份字段副本。
旧字段投影通过 Deref 借用同一份声明；需要拆开可变借用或转移字段所有权的消费者
显式访问 interface，不通过额外 clone 规避所有权边界。

`check_source_function` 在检查主体之前组装受检声明，主体成功后才交付完整 Function；
失败仍恢复 Type 输入事实与 quote 检查状态，不把声明塞入执行函数表。VM 的
`invocation_declaration` 以及程序／类型契约的替换、能力检查只接收 FunctionDeclaration，
主体仍沿独立的
Entry → Body 路径取得。required 收集生成的 runtime 函数身份时只复制所需声明字段，
需要执行的依赖仍保留完整代码。独立声明和完整函数的 S-expression dump 都包含声明
来源；完整函数还显示主体和表达式各自的来源。

这是已有受检产物的所有权拆分，尚非完整公开供应接口。公开参数名／默认提供者的
跨值传送、源码模板的独立发布、未完成主体请求及原区域 Type 响应仍待接通；
当前 Package 与 VM 注册表仍只收录完整函数。不能因为声明类型不含 body 就宣称
未调用定义可跳过检查，或已经能在等待 Type 时恢复同一次 invocation。

普通输入帧的结构准入现在使用
`CallableProgram::validate_input_frame`，直接接收声明视图：检查 callable telescope、
绑定数量及唯一性、默认供应槽位，以及默认表达式只能读取已经声明的前序参数。
默认内部的局部绑定仍由共同 free-bindings 遍历处理。返回值借用原声明的参数域，
不复制签名、不读取或执行主体，也不为接口制造 Unit 代码。

phase 声明目录与 VM 冷构造共用此入口，覆盖普通函数、quote template 和没有主体
的 trait 方法接口。VM 原独立的方法绑定计数／去重检查已删除；原先未经 phase
凭据的直接 VM 构造没有统一检查普通函数帧，现在在发布 VM 前拒绝错误帧。
`Inspection::callables_for` 将已完成的身份目录绑定到同一个不可变 Package。
phase 检查、VM 和 CodeSession 通过它区分可执行函数、quote 模板和仅有身份的
runtime 函数。声明观察读取接口和来源，执行另行取得主体。冻结世界的目录可以
复用；原始 VM 构造从自己的 Package 建立目录，重复身份在发布前产生诊断。
预算失败与无效绑定分别报告；默认表达式分析错误保留默认值自身的位置。

这只证明输入帧的结构和默认绑定可见性。默认值的类型、result／entry snapshot
约束、程序契约、phase、资源及代码安全仍由各自检查负责；没有新增执行授权。
普通函数、仅保留身份的 runtime 函数和 trait 方法各自携带声明来源。调用目录按声明
报告输入帧错误，表达式和默认代码的错误保留各自的位置。源包仍要先完成主体检查。
VM 构造成本不随主体语句数增加的测试，只证明该准入不扫描
主体，不能推导完整编译成本或一般源码 continuation 已完成。

#### 程序契约使用声明，不制造可执行主体

`MethodContract::program_summary_declaration_inputs` 交付独立 FunctionDeclaration，
`CallableProgram::declaration` 只观察声明根。接口摘要不再制造 Unit 主体；调用分析
分别持有实现主体与公开接口声明，不能通过扫描一个假主体推导参数稳定性。
公开方法契约的输入属于独立调用帧，实现局部变量的重绑定不会重写这个帧；指针
所指存储是否稳定仍须通过完整 memory summary 另行证明。普通直接调用仍检查
真实主体中的参数写入和取址。声明摘要本身不授予内存访问或执行权限。

trait 契约继承的私有 overlay 只复制和替换 FunctionDeclaration，保持原实现
主体的所有权与存储；复制预检计入签名、默认值、形成条件和程序契约，不遍历
不需要复制的实现主体。整批有效契约准备好后才证明显式前置条件的弱化，失败或
unwind 恢复所有声明，绑定分配游标也只在整批成功后提交。

程序验证的 `flow::prove_implication` 直接在同一参数状态下证明前提蕴含结论，
不构造 Unit、return 或虚假结果值。它复用普通验证的输入域、谓词 definedness、
算术与内存事实检查；拒绝结果绑定、入口快照和默认供应这些需要调用执行语境的
载荷。trait 继承和 contextual code 方法细化共同使用该入口。预算耗尽继续返回
Unknown，不能被包装成“实现加强了前置条件”。所有程序命题仍由程序验证层处理。

memory 方法组合确实需要执行实现调用，因此只在真实 Call 构造完成后组合
Function；不先建立 Unit 占位再替换。源码检查与原 VM activation 的交接由调用区域负责。

#### 命名函数形成不再准备泛型主体

源码 VM 中，命名泛型函数值现在使用 `Value::FunctionInstance`，只携带声明身份与
按声明域检查后的普通 comptime 实参。形成该值仍检查签名、形成条件、能力要求与
接收域，但不再为此复制、替换整个主体，也不把命名函数包装为带代码的 Closure。
真正捕获环境的闭包继续保留自己的环境与代码；具体 IIR 的执行载体尚未迁移。

调用先在接收 VM 的声明世界核验身份与阶段；仅有 runtime 身份摘要或 IIR 的传送
证据不能授权源码调用。普通输入、类型要求、能力和入口快照由声明完成准备，程序
入口条件仍在同一次 activation 中逐项执行。只有 Entry 成功转入 Body，才按声明
实参替换主体并取得执行代码。默认供应和契约替换通过共同的声明实例观察取得实参，
不再要求 callee 恰好采用 Closure 表示。

实例代码缓存仍只保存稳定代码，不保存普通输入、结果或当前调用的证明。冷缓存的
入口失败不会准备主体；热缓存的调用仍重新检查当前契约。内部缓存暂沿用空捕获的
Closure 代码载体，与 IIR 共用已有存储，不把这个实现细节重新暴露为命名源码值。

源码中未调用的定义仍应接受定义端检查。VM 的可执行函数注册仍依赖完整受检主体。
已知格式的 `std.fmt.Parse(format)` 通过普通的 Type 结果交付变参域；需要执行当前
调用才能完成源码检查的域仍待交接。[公开格式接口](../library/std/fmt_impl/arguments.fl)

#### 实际调用链与替换边界

`3fd323ca` 上的生产链如下。箭头表示当前同步调用／数据依赖，不表示已经存在
挂起请求；这里的 `check_source_function` 同时服务包收集与 required 依赖收集。

```text
package collection / required dependency discovery
  -> check_source_function
     -> function_body -> expr -> comptime_function_instance
        -> prepare_callable_clause_packet -> planned_typed_declaration_inputs
        -> require complete callable signature or return Error
  -> completed nir::Function

prepare_required_root_with_live_inputs
  -> build closure -> checked root Expr
  -> complete referenced functions -> PreparedRequired
  -> invoke_publishing -> Interpreter
     -> begin_invocation -> prepare_invocation
        -> bind inputs, formation/capability checks, entry snapshots
     -> Session: execute entry conditions with prepare/accept_invocation_step
        -> prepare_invocation_body -> execute body -> execute exit conditions
        -> completed value / original control flow
```

第一个断点发生在完整根产生以前，而 VM 宿主请求只在第二段已经取得执行代码后
才会发生。因此给 `InvocationProgress` 增加一个枚举项、公开测试用 Fragment，
或者在 `dynamic_instances` 的错误分支调用 required 求值，都没有连接这两段。
特别是最后一种做法会在检查未调用定义时执行普通主体，或在原调用之外重建存储。

目前 `PreparedRequired.entry`、result_type 和 RequiredWorld 都以完整根为前提。
保留这个已经完成的快路径；新的活动源区域不能伪装成 PreparedRequired，也不能
每交付一个片段便建立一份新根、重新捕获输入。`required_live_input` 接收 live 值
不等于拥有原 activation，重复传入相同 Value 也不证明执行位置或 return 目标一致。

以下是需要共同落地的职责边界，名称描述目标职责，不是声称已有这些 Rust API：

| 交付物 | 首次建立它的入口 | 必须携带／验证 | 消费者 |
| --- | --- | --- | --- |
| 声明接口 | 当前签名、默认值和契约检查 | 身份、telescope、输入阶段与供应接口、形成条件、程序契约、结果观察及世界依赖；可以描述依赖，但不能谎报具体签名 | 定义检查与调用准备 |
| 已检查源码模板 | 包收集与 `check_open_definition` 的共同定义规则 | 源码快照、声明／body 身份、定义端证据、允许延迟的义务及自由声明绑定；不含本次输入值 | 创建活动检查任务 |
| 已准入调用 | 父区域的输入准备 | 当前签名、按源码序实际取得的参数、形成／阶段／域证据；默认值只按本次供应选择 | 原 VM 的调用入口 |
| 活动主体检查 | 已准入调用的主体请求，或获准执行的 required 根 | 原区域与执行位置、输入域观察、规则后继、词法退出责任、等待的 Type 消费者 | 交付下一段受检代码或失败 |
| 可执行片段 | 活动检查区域 | 完成的类型与 phase、输入输出绑定、原控制出口、世界与执行许可；没有待检查 HIR | 同一 VM 区域 |

接口存在不授予执行权。源码模板存在也不代表其中每项调用都已准入。普通完整函数
继续直接产生 NIR Function；合法延迟的主体保留在源码服务，不能为了让现有函数表
接受它而放入空 Block、Never 或假的 Type。已声明函数值的身份和接口观察可用时，
仍不等于已经取得一个可由 closed/runtime VM 执行的函数体。

`open_functions::Definition` 已有定义端检查，但其 Known/Pending/InputType/Suspended
只描述注解，不能直接当作一般主体模板。迁移必须共用已有定义规则及其前提管理，
替换按语法形状追踪的重复路径；不得给所有 comptime 函数设置 `checked = true`
绕过检查，也不能只等待具体调用后才检查未调用定义。已知错误、非法阶段和无依据
的结构／trait 操作仍在定义端拒绝；只有正式规则允许延迟的义务进入模板。

#### 检查服务与 VM 的借用边界

当前 `InvocationSession` 独占借用 Interpreter，`Checker` 又直接借用 HIR 包。
目标协调器分别拥有检查任务和执行驱动，在一次 poll 返回后传递消息；检查服务
不持有 VM 的 `&mut Environment`，执行器不持有 Checker。不能靠把双方包进
RefCell／Mutex 后从回调重入，或增加 unsafe Send／延长 HIR 生命周期来连接它们。

源码 owner 沿现有 `HirPackageBox::package()` 的借用期限保活。稳定模板保存
所属快照与声明/body 标识，活动任务在这个快照的有效借用内解析它们；快照退出前
销毁活动任务。不把内部 `Package<'static>` 的擦除寿命暴露给 VM，也不把活动 HIR
借用写进 NIR、CodeSession 或磁盘缓存。外部请求可拥有稳定标识与受检载荷，源码
检查回调只在有效 owner 下临时借用 HIR。

一次交接依次执行：

1. 检查任务先保存所有父规则后继和本次待观察位置，交付带域、scope 和执行许可
   的受检前缀。只检查完 producer 的类型不能标记它已经执行。
2. VM 在原区域执行前缀，保留参数 cell、作用域、当前控制阶段和已消费位置。
   若遇到关联项、方法或 builder 请求，继续使用已有线性宿主协议；请求未完成时
   不额外启动同一片段。
3. 正常结果经接收世界与域准入后形成观察响应。非局部 return、失败和取消走原
   控制路径，不被包装成一个 Type 响应；入口／退出契约也不因切片重复运行。
4. 原检查任务只恢复对应待观察规则。实际 Type 完成签名后，才能交付 callable。
   本次输入计划持有已执行的准备位置和实际值，后续槽位直接读取已有绑定。
5. 后续受检片段引用已取得的绑定，按通用域和资源规则使用它们。返回可变值引用
   的读取与只读 Type 观察不同：描述响应不允许克隆可变环境，也不延长已退出 scope。

借用解除只是宿主实现条件，不是源作用域退出。等待 Type 时不运行
`restore_environment`、LexicalScope 的退出或 generator checkpoint 回退；正常离开
和失败退出才消费各自责任。内部检查 Probe 可以撤销候选构造状态，已经执行的
源程序前缀不能因此被回滚或重放。定义发布仍先完成整批接收预检，再切换后续代码
可观察的声明世界；不得原地替换旧 CodeSession 的包并继续使用其准入证据。

两个 ignored 正例分别覆盖签名与列表域消费者，并不覆盖这个交接协议的全部边界。
当前同步错误应保留为明确缺口，直到上述生产链一起接通；已有帧化不构成可以在
任何 `needs_context` 上自动执行源码的许可。Type 描述还须区分已完成的 Any 数据
与尚未取得的观察，具体见[描述生产与准入](type-descriptions-and-admission.md)。

| 完成点 | 可承诺 | 尚不能承诺 |
| --- | --- | --- |
| 定义接口／稳定模板 | 原身份、望远镜、阶段、已检查契约与合法 residual | 所有具体调用的签名已形成，或已有可执行 body |
| 调用准备 | 当前 callee 签名、输入与形成义务已经满足 | 入口程序契约已经运行或主体已完成 |
| 已进入 activation | 原参数存储、结果域、入口快照与契约进度 | 未取得的受检主体已经可以执行 |
| 可执行主体／片段 | 域、阶段、安全检查及所需声明世界满足交付契约 | 重放前缀、运行未选分支或改变 NF 身份的许可 |

现有 `check_source_function` 已共用参数绑定、普通 Type 的 bound 身份、前后置条件、
入口快照和模板／效应感知主体检查。最终声明携带已检查默认值；required 依赖由实际
call plan 提供用到的默认值，不能强制准备未使用项。

VM 已把普通输入的 body 重绑定放在 Entry → Body 转换：入口程序契约通过后才准备
这份执行代码，等待／取消不重新 begin_invocation。源码命名实例的声明参数主体替换
也已移入这一阶段；域、签名与形成条件检查仍在调用准备时完成。该接线不是定义端
源码延迟检查。新主体服务必须沿同一 activation 取得代码，不能
为了允许缺失 body 而删掉定义检查或阶段准入。

VM 的 `contracts::Progress::Body` 现直接持有已准备的 Body 与其代码缓存键，
不再由 InvocationFrame 在 Entry／Body／Exit 之外保留一个始终可回退执行的 body。
Entry → Body 先取得并按普通输入准备代码，成功后一次交付这个阶段状态；Exit
保留结果和原 source，用于结果交付、后置条件及清理。树执行与字节码执行共用
InvocationFrame 的当前代码选择，不在入口或出口缺少 condition 时回退到主体。
dump 区分 package-body／owned-body 及相应缓存键。

此处只拆开了活动调用的执行状态。被调用的 Function／Closure 仍可保留完整的
已检查源码；签名准入现通过下面的声明视图取得来源，完整包适配器仍从 body 提取
该位置。声明参数特化、函数收集和
RequiredWorld 尚未因此接受缺少主体的源码模板，也没有新增源码主体的外部请求。

接线要共同改变函数收集、required 根和 VM 主体取得。当前 RequiredWorld 只有完成
的 NIR 声明；只给 InvocationProgress 增加请求变体无法让未完成源码抵达它。
源服务不能提供某种模板时明确报告缺口，不转而解释 HIR，不将所有函数改为首调用时
才检查。只有正式规则允许延迟的义务可以进入稳定模板。

片段的声明世界与依赖证据必须匹配；新增定义需要显式可见世界交付，不能换掉
CodeSession 的包指针后复用旧准入。活动任务、receipt 和 VM cell 不进入 RequiredWorld、
代码缓存或 Type 结果缓存。相同源码的两次 invocation 只共享稳定模板、接口与合法
代码，不共享执行状态。NF 头始终不展开，实际 Type 响应也不改变它的身份规则。

#### 函数值的形成、调用与主体取得

检查具名函数值需要其声明身份、子句参数、签名、阶段和类型形成条件。完整函数声明与
只保留身份的 runtime 函数通过同一个借用视图提供这些字段。elaboration 在为
required computation 或开放函数构造局部源码环境时，须一并投影声明的类型命题和
trait 要求。否则接收方会把一个有条件的函数值当作无条件的函数值使用。

普通函数及 trait 成员的签名特化按声明参数建立输入依赖关系。某个值实参的域若依赖
位置靠后的类型参数，先取得该参数的类型证据，仍按公开槽位绑定实参。检查后的同一组
输入负责替换签名里的参数。这个过程适用于普通声明，NF 头保留自己的应用与身份规则。

VM 在形成具名 runtime 函数值和接收冻结值时，用当前声明环境检查实参、特化签名和
已闭合的形成条件。IIR 在函数值作为类型实参出现时也需要这些证据。比如某声明要求
`T :- Allowed`，保存声明身份和 `T = bool` 可以重建查询，接收方仍须建立
`bool :- Allowed`。无法判定与已被拒绝是不同结果。

调用还有独立的需求。默认值、前后置条件和诊断位置取自调用声明，进入函数体或特化
泛型代码才需读取主体。runtime 函数身份可以穿过 comptime 并作为 IIR 类型中的数据，
仅凭身份无法将函数列为可执行根。当前可执行源码仍由完整包或已有的已检查片段供应；
在同一轮 elaboration 中按需向尚未完成的源码模板请求主体，仍属后续工作。

#### 完整包检查凭证也是迁移边界

完整包与活动源码区域之间的交接需要以下消费者同时识别声明可见性、代码需求和当前检查证据。

| 位置 | 当前所需产物 | 迁移要求 |
| --- | --- | --- |
| `prepare_required_root_with_live_inputs` 与 `required_source_closure` | 完整根；按声明、方法、关联值和定义身份建立本次调用的源码闭包 | 区分声明可见性与本次代码需求；合法延迟模板保留定义端检查，不把源码引用一律当作立即取得完整主体的要求 |
| `required.rs::required_function` | known callable signature，然后同步 `check_source_function` | 签名待决由父检查区域保存；尚未准入的 callee 不能建立 activation 来计算自己的参数域 |
| `required.rs` 的 `check_package_with_evidence` 与 `validate_machine_operations_checked` | 整包 phase 完成证据及独立的算术安全检查 | 完整根保留现有快路径；源码片段交付须带与实际片段、binder 和世界匹配的检查证据 |
| `RequiredWorld::with_phase`／`CodeSession::with_phase_checked_source` | 对同一个不可变 Package 的整包凭证 | 不把部分目录升级为 PhaseCheckedPackage；源码服务和活动任务使用独立 owner，不伪造完整包 |
| `nir::callable_source::Callables` | 与源包绑定的不可变身份目录 | 声明观察与代码取得分别查询；待完成源码的独立发布仍需接线 |
| `prepare_invocation` | 直接借用 FunctionDeclaration 取得接口和声明来源 | 独立发布源码接口，并在 Entry 成功后按需交付主体；源包注册仍依赖完整 Function |
| `prepare_invocation_step` 的 Entry → Body | 同步取得、特化和重绑定完整主体 | 入口契约成功后在原 activation 等待代码，响应不得再次 bind inputs、运行入口契约或建立新调用 |

`nir::phases::Inspection` 与 `PhaseCheckedPackage` 也必须区别：前者拥有相同
世界的声明目录并可检查表达式，后者只在全部函数、quote 模板、默认值与契约的
phase 遍历成功后形成。`Inspection::check` 不是类型／资源安全证明，既有
`invoke_comptime_expression` 又会建立新的闭包调用；它们均不能直接作为活动
源码片段恢复 API。为避免扩大权限，新片段交付应满足以下条件：

- 稳定目录只证明声明和域的合法性，不声称未完成主体已经检查。定义模板仍须
  检查已知错误、阶段和定义端义务；合法 residual 必须显式保存。
- 片段证据绑定准确代码、输入输出 binder、世界依赖及控制出口；类型、phase、
  safe 义务分别完成。需要既有程序事实时交付其上下文，不能改用空前提重检后
  放宽失败，也不能仅持 phase 凭证绕过算术与资源验证。
- 接收端核对原区域／activation、请求序号和需要的完成点，再消费一次响应。
  预算失败、取消、错世界及错误 binder 不安装代码、不前移 PC；旧响应可以退回
  原 owner，取消后的响应不能复活调用。等待中的存储和词法退出责任仍在原帧。
- 稳定代码缓存只存已检查产物及依赖，当前调用的入口契约、Type 观察、cell 与
  pending request 不进入缓存。完整包检查不能因命中某个片段而提前标记成功。

现有 `DefinitionHostRequest` 的载荷覆盖 Struct／Enum／Newtype builder，
回答的是“准备哪些新定义及其 receipt”，不是“取得哪个源码主体”。不要把主体
请求塞入 builder 协议：前者恢复已知声明的执行，后者安装新声明并更新可见范围。
两者可共用 ticket／预算的实现原则，但完成条件、取消责任和世界变动不同。

**实施验收必须越过这整条链。** 两个待定 Type 正例先证明父区域能交回观察；
随后还需未调用定义的已知错误拒绝、入口契约失败不准备主体、等待期间不重放
producer、同一模板的两次调用独立、片段算术／phase 违规拒绝，以及 builder
在片段之间发布后的正确世界交付。仅证明函数表能存接口，不能算恢复功能完成。

### 纵向交付与验证证据

下一项交付同时贯通普通规则 continuation、原执行区域的片段交接、待定输入域和
callable 签名对实际 Type 的消费。只增加 owner、请求枚举或可手工构造的 fragment
入口不满足交付；也不能只解除两个 ignore 而遗漏下列边界。

| 验收 | 必须证明 |
| --- | --- |
| `count<.values = [1, 255], Byte()>()` | 域实际完成后检查和组装，Byte 一次，完整输入后才调用 count |
| `identity<Byte(), true>(9)` | 父区域执行 callee producer，签名完成后检查 9，无 packet 重放 |
| 前后输入读写同一存储 | 源序、读取时的值、原 cell 身份，含方法接收者 |
| 前项 return／失败 | 后项 Type、默认值和主体不运行，原控制出口保留 |
| 分支、guard、循环、递归 | 所选路径执行；应检查的未选代码仍检查；作用域及执行代次不串值 |
| 具名／默认／空展开 | 检查顺序可不同，实际求值顺序和次数不变 |
| 错域／预算／取消／旧响应 | 失败不发布半成品；不同失败原因保真；重试不重放已执行前缀 |
| 未调用定义 | 保留定义端检查及合法 residual，不执行普通主体效果 |
| NF／名义输入与真实 sysroot | 消费同一协议，普通值参数无库名特权，跨世界准入仍生效 |

普通路径与这些边界通过后，NF／名义消费者迁入同一协议，才能退役 InputChecking
分流和 NormalFormArgument 的执行树。未迁移消费者必须保留其真实检查，不能为了
删枚举而丢失能力。活动 continuation 不进入稳定 NIR；实例化从稳定模板创建新任务。

现有行为可以从下列入口分别复核。两个需要在源码检查期间等待 Type 的正例应显式运行
`--ignored`，常规测试组会跳过它们。

| 行为 | 持久检查入口 |
| --- | --- |
| 已供应与待定 Type、参数域对前项的依赖 | `luna/tests/ordinary_callable_types.rs`、`ordinary_clause_collectors.rs` |
| 捕获域间接引用的回调、活动值的绑定范围 | `elaboration/src/program/required/{capture,scope}.rs`、`luna/tests/required_input_scopes.rs` |
| 默认值、具名供应、前缀副作用及提前返回 | `luna/tests/clause_packet_preparation.rs`、`captured_input_observations.rs`、`default_calls.rs` |
| 受检输入片段归还词法绑定 | `comptime_interpretor` 的 `bytecode::input_preparation_tests` |
| `if`、`when`、`match` 探测后的恢复和方法调用顺序 | `luna/tests/contextual_input_regions.rs`、`inline_control.rs`、`nesting.rs` |
| 函数契约、源码选择及 VM 入口 | `luna/tests/declaration_contracts.rs`、`prepared_invocations.rs`、`required_definition_publication.rs` |
| 真实 sysroot 中的延迟变参域 | `luna/tests/library_std_format.rs`、`development/examples/host-println.fl` |

探测与正式检查共用分支规则。失败沿检查区域退回最近的探测点，恢复词法作用域和存储，
保留已经消耗的预算。对 `!bool T` 实参，分支可以先获得已知的错误行，成功值的域由
后续分支补足。最早出现的 `error true` 不能决定所有成功分支的类型。

嵌套 `when` 曾在所有检查帧都已退出后，于静态值观察阶段耗尽宿主调用栈。现在静态
控制路径迭代推进。参考树解释器仍受自身递归深度保护。复核深层样例时应区分检查区域、
静态观察和 VM 执行轨迹，预算耗尽的位置决定还能对哪些行为作出断言。

#### 声明默认代码与调用前缀的交接

`Config<enabled>` 在默认值所属声明中
引用的是合法的抽象形参；在调用输入准备阶段，`enabled()` 的临时绑定只是已经
检查的生产者，还不是已供应的值。把前者直接改为后者再调用 `type_expr`，会令
required 捕获一个尚不存在的值。扩大捕获权限、提前执行生产者或给临时绑定塞入
一个 rigid 都不能解决这个所有权错误。

NF 输入计划的迁移采用声明侧检查结果作为代码：保留形参身份与泛型域，将代码的
自由形参局部连接到本次计划的输入绑定，内部局部、闭包捕获、模式及控制身份按
现有通用 freshening 规则改名。连接只允许使用已有的前序输入位置，不使其变为
completed；已经观察到的身份可以代入，尚待执行的前缀继续由 VM 输入计划取得。
VM 在执行该默认值前，使用已经准入的前缀实例化代码中的域，并按实际接收域检查
结果。显式输入仍保持源码顺序，缺省输入仍排在显式输入之后按声明次序执行。

声明验证与调用处代码准备必须共用同一个默认表达式检查入口，保留自身／后续
形参引用禁令。声明侧事实与调用方的控制、输入观察不能混用；默认代码的内部局部
也不能在两次展开间共享身份。这里不新建默认结果缓存，也不把已执行调用的值写回
声明模板。当前准备仍重新检查声明侧源码；稳定模板查询的复用与失效属于后续工作。

当默认表达式引用关联项或 trait 方法时，执行世界必须提供对应声明及查询服务。
显式实参已占用的槽位不会启动默认代码，必填槽位缺失在路由时报告。
[默认值执行](../compiler/luna/tests/normal_input_plans.rs) ·
[开放域默认值](../compiler/luna/tests/open_normal_input_plans.rs)

这只能处理已能在声明环境形成受检代码的默认值。依赖外部 Type 生产者才能继续
检查的源码、普通调用中等待执行结果的参数域，仍需要源区域的挂起和恢复。
公开 println 在 format 已知时的描述准入见[宿主演示](std-host-foundation.md)。

## 5. Observe、登记与发布

ObservationKey 包含语义主体、所请求 facet、作用域/视图和当前前提；它与 task key 不同。
例如 `ParameterDomain(f,2)`、`TypeHead(t)`、`Layout(t,target)` 请求不同证据。

```text
observe(key, premise_scope) ->
    Known(value, evidence, dependency_stamp)
  | Pending(producer_id, observed_generation)
  | Unresolved(reason, source)
  | Failed(diagnostic_id)

wait(task, observed):
    register task-generation against producer-generation
    re-read producer status and generation
    if changed or terminal: enqueue task-generation once
```

发布先构造完整 update 并校验输入代次，再原子安装结果/赋值/依赖，最后收集通知。
通知只入队，不同步重入尚在更新的 checker/solver。union-find 合并要迁移或转发 watchers，
rollback/producer failure 必须使旧观察失效，不能仅在成功赋值时通知。

一个任务有多个依赖时，唤醒只意味着重新观察；按 task-generation 去重，不能每条边
重复恢复 VM。被取消 task 的票据失效；共享生产者只在没有剩余需求且允许取消时释放。

## 6. CandidateTrial 的提交协议

`Created → Checking → Applicable | Excluded | Undecided`。
只有 Applicable 可在选择条件成立后进入 Committed；任何未提交试探都可以 Discarded。
Undecided 仅在新增真实信息或额度后重新进入 Checking；Excluded 不可提交。
Applicable 只是试探成功；更精确候选未决、同精度冲突未排除时仍不能 commit。

trial 保存 base RegionRevision、SemanticView、局部赋值、原始义务、term holes、诊断缓冲
和依赖摘要。对外只可观察试探状态，不可导出尚未成立的值作为共享事实。

commit 分为 prepare 与 install：

1. 核对 parent/视图与候选 universe；变化时重证，不自动合并两个试探的赋值。
2. 检查全部本次必检义务、scope/sort/occurs、原始类型关系及 selection 规则。
3. 准备合并后的推断、term result、证据及诊断处置，保证 install 不需要失败性检查。
4. 一次安装，递增 region revision，然后通知受影响观察。

preflight 失败不修改 parent。安装阶段不运行用户 comptime 代码或新宿主请求。
未完成的其他 region 义务可以继续存在，但不能把本候选未完成的必检义务当成别人的任务。
允许 residual 的位置必须有独立语言依据及稳定描述，不以 CandidateState=Undecided 导出。

被放弃候选的内部诊断仅用于最终无解解释；明确源执行产生的编译诊断不能随候选丢弃。
后者因此不能在未选定候选里投机运行。PTC 学习可保留，但须保留 world/activation guards。

## 7. 原 invocation 的宿主请求

当关联项缺少选择、trait 方法需要实例或 builder 需要准备定义时，执行已经到达一个具体位置。
同一循环的下一次迭代即使经过相同指令、取得相同输入，也有权再次执行。源调用的会话持有
活动栈、参数存储和恢复位置。缓存可以复用该世界中的受检代码，请求的身份则只对签发它的
会话和代次有效。[执行驱动](../compiler/comptime_interpretor/src/invocation/session.rs) ·
[请求和交付](../compiler/comptime_interpretor/src/invocation/session/services.rs)

| 边界 | 宿主拥有 | VM 拥有 | 允许的下一步 |
| --- | --- | --- | --- |
| 请求就绪 | 无 | 已检查的请求、调用前缀、剩余额度 | 本地驱动调用配置的服务；外部驱动在本次 poll 交出请求 |
| 请求已发出 | 源世界、请求描述、线性票据和预算 | 同代票据、执行栈和对应的恢复位置 | 再次 poll 返回 Waiting；宿主可独立准备响应 |
| 响应送达 | 误投时仍持有完整响应 | 正确送达后持有待接纳结果和未花额度 | 下一次 poll 在原位置核验实际结果 |
| 结果接纳 | 无 | 新的初始化 activation、已准备的方法调用或已安装的定义及目标槽 | 继续执行一次，或沿原调用链传播失败 |

发送时一次性转出全部剩余额度。请求到响应移动同一份凭证，宿主只能调整剩余额度。
送达先核对源世界、会话代次和恢复位置的种类。误投时两边均保留原状态；超过初始额度的
响应会在原调用中产生带请求来源的 StepsExhausted，返还额度为零。取消负责释放 VM
拥有的调用前缀、丢弃尚未接纳的响应并恢复调用者。已经发生的外部动作及其花费仍归原执行。

| 操作 | 停在 VM 内的状态 | 收到响应后的语义检查 |
| --- | --- | --- |
| 关联值 | 原访问的调用位置 | 核对选择及声明，进入初始化器，以原结果域接收返回值 |
| 方法值或方法调用 | 值的目标槽，或调用指令、默认供应和接口契约 | 验证公开 member、类型实参、实现和签名，再形成 callable；普通实参按原次序执行 |
| 定义 builder | 已消费输入包的原指令和目标槽 | 核对请求描述、未提交 receipt 和目录准入，安装成功才写入目标槽 |

内部同步驱动还服务于具体 IIR。它可直接调用配置的 resolver，并将结果交给相同的
下一步接纳逻辑。外部驱动允许把请求移出 VM，原调用的可变存储和方法准备状态始终归 VM。
宿主若需等待另一项语义工作，目标调度器须保存本次请求及其子依赖。后续唤醒继续已有
prepare；子依赖的预算耗尽、失败、未知外部行为分别报告。纯计算的缓存资格和候选试探
的重放权限由各自的语义规则决定。

### 源码检查的下一条交接边界

当前三种请求都始于已经检查的表达式。下一个需要恢复的位置在更早的阶段，例如前面的列表
字面量仍缺元素域，而后面提供类型的表达式可能写变量或根本不会执行。检查任务须保留
原 HIR 位置、已检查子结果、域的依赖和词法前提；执行任务须保留原 invocation 的存储、
控制目标和已经产生的值。取得实际 Type 后，由该次检查的拥有者接纳它，生成可执行的
后续片段。遇到提前 return，原执行直接返回，后续类型生产者没有执行机会。
这条路径须通过普通源码入口、跨冷暖调用验证次数、次序和读取快照；单独构造一个宿主
响应无法证明它已接入。[源检查与执行的交接目标](#源检查与执行的交接目标) ·
[源码执行次序验收](../compiler/luna/tests/type_input_execution_order.rs)

### 普通函数契约的恢复边界

普通函数的前后置条件原先在 `prepare_invocation` 和 `finish_invocation` 内同步
调用树求值器。即使主体支持宿主请求，前置条件中的关联值选择也会在返回 session
之前尝试同步解析，后置条件则在主体完成的交付栈上同步执行。接口调用的
`Execution::Contract` 已能使用字节码，不能把这两条路径混为一谈。

普通声明现由同一 activation 持有 `Entry(index) → Body → Exit(index,result,returned)`。
每次只准备一个条件的普通 CheckPredicate 表达式，交给已有执行驱动；完成后保存
环境并推进原 activation。主体仅在所有前置条件成功后进入一次，后置条件保留已检查
的实际返回值，显式 return 不绕过后置条件。类型／capability 准入仍在各自原边界完成。

tree 参考执行和字节码驱动共用这套进度及完成规则。字节码路径在条件内遇到关联
请求时保留 PC、环境、调用者及当前条件索引，恢复不重跑前序条件或主体；Display
显示当前阶段、下一个条件和保留结果。body 的稳定 code 身份不能用作条件代码的键。
函数进入事件和 bytecode invocation 计数只计主体，不把每个条件计成新的函数调用。

失败及取消沿同一 activation 恢复链退出，不执行剩余条件；未交回的宿主预算不会
凭取消而退还。runtime 的 scheduled result／清理检查仍由原资源计划负责，这项
迁移没有去掉字节码的 cleanup/effect 门控，也没有让 PTC 执行程序谓词。

源码测试 `invocation_host_requests` 使用真实六包、SourceInvocationQueries 和外置
宿主响应，覆盖实例化 Type 参数的契约、闭合及泛型关联初始化器、代码会话复用。
泛型初始化器在前后置条件中各执行一次；缓存选择与代码不缓存读取的值或契约结论。
VM 局部测试另覆盖前后置条件拒绝、提前 return、等待、过期响应、预算耗尽和取消。

本项只接通普通程序契约的执行进度，尚非通用前端检查／发布宿主协议。参数准入、
类型契约及某些准备服务仍同步；本阶段原有的编译拒绝回退已由下节清理。
签名依赖实际 invocation 产物的两个普通调用正例仍未解锁。

### 编译拒绝与执行模式分开

字节码编译失败原先返回 `Ok(None)`；声明、借用体、owned body 和 contextual input
代码缓存因此都能保存“没有程序”的成功条目。调用者会据此进入树执行，后续调用则
复用这次回退决定。它把编译错误、缓存未命中和显式参考执行混为一个协议。

现在编译返回可执行代码或带来源的错误。Unsupported、预算、深度、控制流和未解
类型证据分别保留错误类别；失败消耗的编译工作照常计费，不能发布代码缓存条目。
代码缓存的条目类型只容纳可执行程序，缓存未命中仍允许重新编译。执行入口的
`Option` 只表达现有 mode/schedule/effect 门控，不能由编译拒绝产生。

主 ExprKind 分派改为穷尽匹配，标量编译也进入该分派。尚未实现的节点逐项列出，
新 IR 变体必须做显式 lowering 决定。报告列出的十三种节点不能直接算作十三项
合法 comptime 能力缺失：

- 指针操作被 comptime 执行阶段拒绝；runtime 指针 Type 描述和 quote 不因此禁用。
- effect 当前在 phase 检查与 comptime 树执行中均明确未实现。这里没有把这个
  实现限制上升为最终语言永远禁止 comptime effect 的规则。
- StagedValue 尚未完成，不能作为可执行常量；TemplateEqual 服务 runtime 的
  staged callable 身份分派。换引擎不能完成其阶段证据。
- 原 wildcard 还隐藏了 guarded 分支的剩余情况：非法域 and/or、携带参数绑定的
  NF Call。后者在树执行也报 InvalidCallPlan，必须补齐实际准备协议，不能靠回退。

回归直接送入有赋值前缀的未完成 IR，验证编译拒绝保留原来源，未启动输入执行，
声明／借用／owned 缓存不产生负条目；同一 VM 后续合法代码仍可执行。这个 IR 探针
不是合法源码用例。缓存容量、LRU、过期弱身份、时钟耗尽测试改用真实可执行程序，
不再以失败条目当作缓存载荷。显式 tree 参考执行仍单独验证。

本项没有声称 tree 求值完全退出生产路径。nested staged input 的 depth 门控、
cleanup/effect 模式选择及同步准备服务尚在；通用前端 Region 恢复仍未完成。

### required 输入证据的区域所有权

例如外层 required preparation 已为 `flag` 供应
具体 bool，并建立普通局部绑定；默认值 `Flag<flag>{}` 的名义类型检查递归进入
required preparation 时，只继承了 binding/domain，没有继承值。于是内层把一次
已有身份观察变成另一次没有输入的 invocation。修复点在准备区域的输入交付，
不在 generator、Flag、List 或某个源码形状上。

`required::inputs::Scope` 现在按活动准备区域持有只读输入证据。每个 frame 保留
`LocalId → (domain, supplied value / unavailable)`，父 frame 用 Arc 保活；没有
pending producer、VM 可变存储或候选等待者，也不进入 RequiredWorld 或代码缓存。
子区域在形成后续参数域之前逐项登记已完成前缀，优先观察自己的输入；live 输入显式
遮蔽同号外层快照。退出准备区域（成功或
错误）恢复原 scope 和本区域引入的 immutable 标记。Display 使用 S-expression，
同时显示局部身份、域及可用性，不把 unavailable 当作已完成值。

普通 `preserved_input_identity` 的已有结构观察可读取仍在词法作用域的 immutable
输入证据；若需要真实执行 helper，nested required 捕获只复制实际引用的外层输入，
并继续走既有 domain/phase/capture 准入。读取、复制和遍历都消耗 required 预算；
失败不发布部分 snapshot。赋值、取址及捕获的既有失效入口同步遮蔽该证据，失败分支
恢复规则也保存 scope；多次失效替换当前 frame，不为每次写入增加一层 lexical 链。

普通函数的定义期默认值检查也建立独立输入区域：前序子句参数供应自己的 rigid 身份
和声明域，不把未知参数当作具体值。普通调用计划则继续区分「观察出的身份」与
「已经执行得到的输入」：前序 prepared binding 的事实临时进入已有 partial-staged
观察表，供嵌套表达式检查使用；它们不进入 required 的可执行输入 Scope，也不删除
原 producer。完成后只恢复本 packet 的事实条目，保留其他存储写入产生的失效。

这项交付只涉及已供应且可只读观察的输入。它没有把有待执行的 Type 生产者变成
已知值，没有重放副作用来取得证据，也没有接通通用前端 Region suspension。普通
待定域两个 ignored 正例仍需要原计划中的检查／执行恢复协议。

### Type 生产者的 invocation 与发布边界

普通源码的生产路径是：

```text
Checker::invoke_required_publishing
  -> PreparedRequired::invoke_publishing
     -> invoke_in_world
        -> ordinary VM invocation + source services + CodeSession lease
        -> restore_required_packet
     -> (Value, domain) + PublishedDefinitions
  -> publication::Receiver.register(definitions)
  -> ordinary (Value, domain)
```

这里的 publishing 是 builder 定义的检查与注册，不是向 Type metavariable
赋值。输入齐备的 Type 返回值与其他 comptime 值共用上述通道；`627ab804` 已删除
临时 InferenceContext、MetaId、排队、赋值和取回这一整条旁路。授权 TypeParameter
在共同恢复边界变为同身份 rigid Type 并按调用方 telescope 替换，转换计费，
不建立新的类型相等事实，不授予未授权 binder，也不展开 NF 头。

VM invocation session 保留活动帧、输入、契约及宿主请求；RequiredWorld 保留
不可变声明和可复用代码。Associated 请求与 Method 请求都携带来源世界的 resolver；
同步驱动在 VM 外准备响应，显式驱动可以保存请求后再交付。准备不等于接收，
接收也不等于选择已准入或 builder 已安装；原 ticket、世界和预算必须分别核对。
普通 required 驱动仍同步完成这些交接，不是一个可挂起的源码 Checker。

闭合的 Any、refinement 和应用描述已经可以作为普通 Type 结果交付。
`std.fmt.Parse(format)` 利用这条路径为 `println` 的每个字段建立 Display 约束；
真实 sysroot 的调用检查和 Linux 宿主输出分别有[格式测试](../compiler/luna/tests/library_std_format.rs)
与[原生演示](examples/host-println.fl)覆盖。

required 子句调用把选中声明的域模板留在调用者一侧，执行世界接收已检查的
操作数。返回的字段类型和值在恢复后的作用域中一起检查，形成义务仍由实例
消费者负责。普通调用若还需要活动 VM 的返回值来检查一段 HIR，则原源码区域
和原 invocation 之间仍缺恢复通道。两种情形的交付顺序见[迁移计划](refactoring-plan.md#下一条纵向交付待定域的普通调用)。

#### 已退役队列与仍保留的赋值权限

`elaboration::type_computations::Session` 在 `627ab804` 后已无生产调用者，
`889c500c` 将其移至 `compiler/elaboration/test_support/type_computations/`。
它是依赖预检、取消、宿主交接和单输出发布的测试模型，不是新前端调度器。
模型可以使用真实 sysroot 及 VM，但不经过普通源码 Region 的待决域交接，
因此其成功不能解锁普通 callable／子句收集器两个待定 Type 正例。

模型保留一个 invocation owner，终态不重放，取消沿依赖传播；票据错投可退回，
已消费预算不退款，Live 输入准入先于 Normal 输入工厂。`type_resolution` 中
AssignmentPermit 仍管理单个 meta 的受检发布：保留输出阻止普通赋值和旧候选抢占，
不阻止无关变量推进；完成仍检查上下文、sort、occurrence 和预算。它不是语言
capability 或稳定 receipt。`nir::type_computations` 的 scope／残余／预算工具仍有
生产消费者，不随旧队列删除。

退役的源码边界、模型消费者和验证证据集中于
[模块退役第 8 节](module-boundaries-and-retirement.md#8-type-依赖队列退出生产模块)。
本节不再保留旧队列反复迁移的过程日志；历史实现和验证见 Git 中 `627ab804`、
`889c500c` 及其前序提交。下一步不得继续扩展这个测试模型来冒充源执行恢复。

真实入口的宿主验收覆盖关联选择、方法值与直接调用、跨会话误投和取消后的重试。
定义的验收从公开 builder 构造 struct、enum 和 newtype，再在同一执行中消费刚生成的
类型。只有接纳成功的定义才对后续位置可见。
[关联值与定义请求](../compiler/luna/tests/invocation_host_requests.rs) ·
[方法请求](../compiler/luna/tests/method_host_requests.rs) ·
[定义构造与消费](../compiler/luna/tests/builder_invocations.rs)

## 8. 宿主请求与候选、取消的组合

候选内允许的计算必须可隔离且可安全重放。pure 承诺用于相应纯计算，不自动使定义发布
或外部工具操作成为可回滚动作。必须先运行不纯候选操作才能判定适用性时，按正式规则
要求显式消歧或契约，不擅自试运行。

取消前有外部操作已经发生时，只能取消后续编译工作并报告已知进度；编译器内存 rollback
不撤销外部世界。实际完成状态未知则 Indeterminate，不静默再发一次。
共享的只读 query 可以有多个消费者；某个消费者取消不影响其他人。源执行产生的诊断和
副作用责任属于源执行 owner，不由“最后哪个 task 还在等待”决定。

预算 lease 预留给子工作，完成/失败返还未用额度，已消费部分不回退。未执行的分支
不创建发布世界。回收内存、注销 watcher、归还句柄等必须在预算耗尽后仍能完成，
通过预付容量/有界 owner teardown 实现，不能在 Drop 中启动新的用户代码检查。

## 9. DefinitionTransaction：准备与安装之间不能再运行用户代码

```text
Draft → Preparing → Prepared → Installed
            ↓           ↓
        Rejected      Stale
            └── repair / reprepare ──→ Draft
```

Draft 拥有生成身份、binder、代码、属性、来源和本次可见范围。Preparing 在私有 overlay
中使用普通 elaboration/coherence 准备；若需要用户计算或其他查询，都发生在此阶段，
明确归属父源执行和子请求。不可回放的已经完成步骤保存进度，不以 stale 为由从头重跑。

Prepared receipt 绑定：transaction ID、草稿版本、base world revision、可见范围、
接收者集合及版本、检查产物、所有待安装结构和预算/容量预留。
receipt 内容不可再修改；草稿改动使旧 receipt 失效。

### 安装的线性化点

目标采用一个 PublicationBundle 持有目录、已完成定义、VM 可用描述、协议索引增量和
必要来源；消费者通过同一个版本根取得相容集合。全部内存分配/检查在 prepare 完成，
install 只做版本核对及根切换，不逐个调用可失败的用户回调。

单线程 owner 已能保证唯一安装点，不要求先引入无锁算法。未来并发使用受控发布锁或
等效原子根，语义仍是一个版本；不能让查询看见新 impl 而 VM 仍缺它的声明描述。
源码批次中的 forward reference 和互递归在 overlay 内处理，不提前泄漏到其他实例。

物理实现使用不可变块和按路径复制索引；旧 root 的销毁不包含在根切换的常数工作内。
prepare 预留 retired-owner 记录，切换后单独回收，避免安装期间运行无界析构。
快照 pin、草稿回收及跨世界句柄见[世界存储与生命周期](world-storage-and-lifetimes.md)。

版本过期返回 Stale，并保留可重新准备的草稿。只有实际依赖未变且检查范围相同的
证据可复用；“仅新增声明”不证明 trait 查询、负查找或 coherence 不受影响。
编号可以跳号，已经逃出预留 owner 的身份不可复用。为减少复制的 overlay 优化不改变此契约。

### 安装不是最终实例成功

Installed 让本次作用域后续可见位置取得定义；仍需保持第 2 节的 frontier。
整体生成结束时完成剩余 coherence 及实例义务，成功才进入稳定交付边界。
失败不能链接其产物；已运行的外部行为仍按原进度报告，不能伪称全部撤销。

恢复 VM 时，旧 CodeObject 仍属于其固定语义视图；后续需要新生成定义的操作携带新视图
和显式依赖桥接。不能把所有旧 code cache 直接改标为新 world，也不能因声明编号相同
复用原调用选择。如何分割需新观察的 continuation 由 staging plan 指定，而非执行器猜测。

## 10. 三条时序推演

### 后实参帮助前 lambda

```text
call task: allocate T, U; create argument tasks a0 and a1
a0: observe T → Pending(T-producer); register and recheck
a1: establish i32 input → publish T=i32 → enqueue a0
a0: reobserve T → check body → publish U=i32
call task: settle original obligations → commit ArgumentPlan
```

源 runtime 参数还没有执行。将 a1 先出队也须得到相同调用及求值顺序；差别仅是无需等待。
若 a1 失败，则传播失败给 a0 的观察，不能留下一个没有生产者的永久 waiter。

### 生成定义请求在挂起后完成

```text
VM activation 7 reaches generation request ordinal 12
request 12: Prepared → InFlight; frame pins PC/destination
publication needs interface Q: WaitingOnInput(Q)
Q resolves; same request resumes preparation; no new ordinal
host returns an uncommitted receipt; origin accepts delivery for request 12
original VM validates and installs at frontier B, then fills the destination once
next iteration at same PC uses ordinal 13, not 12
```

若响应重复到达，12 已 Delivered，不再注入。若 Q 失败，无半批定义、无 placeholder Type；
草稿可以留作修复，但这次必需 invocation 失败。

### 线性使用与整体 coherence

```text
frontier A: visible implementation set IA; use chooses evidence EA
publish B: subsequent positions see IA + IB
frontier B: a later use performs selection against the new eligible set
close enclosing instantiation: check generated definitions with static scope
```

这不是允许冲突：整体 coherence 失败，实例不能交付。也不是重跑 A：线性可见性保留
A 处已有的合法执行与证据来源；任何缓存复用必须携带对应 frontier。

## 11. 需要在实现中逐项证明的恢复性质

每个动作在“更新前、更新后通知前、响应保存后交付前”三个切点都应有明确状态。
单元层检验 owner/generation/receipt；源码层检验默认值、模板、builder、多实例及外部
请求轨迹；不能只用伪 session 构造器证明真实接线。

重点拒绝：旧 ticket 命中新 task、budget Unknown 自动重排队、candidate 的负查找跨
universe 复用、stale receipt 触发用户回调、整实例失败后仍交付机器码。
这些为后续验收要求，本轮没有运行这些测试，也未声称当前代码已经满足新协议。
