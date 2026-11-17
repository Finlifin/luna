# 可计算类型描述与调用准入

状态：重构设计；源码核对基线 `e1c57dc9`。这里不声明新语言规则，也不声称 Any
已进入 VM。目标是支撑作者指定的
`fn println(comptime format: str, ...varargs: defer std.fmt.Parse(format))`，
同时适用于用户类型工厂、别名、容器和普通函数。正式 docs 保持不动。

## 1. 契约与当前断点

依据为作者确认，以及[类型身份](../docs/type/type-identity.md)、
[Any 与上下文检查](../docs/type/comptime-gradual-dependent-type.md)、
[两阶段检查](../docs/comptime/normalization-and-instantiation.md)和
[参数供应](../docs/functions/parameters.md)。历史 prompt 中的格式化意向仅帮助
理解需求，其中旧的 postfix refinement 拼写不是当前语法依据。

Any 与 bool、u8 分别相容，不产生 bool == u8，也不产生二者间的转换。
`Identity<Any>` 不因 Identity 的函数体返回参数而变成 Any；具体化后仍保留头。
返回描述含 Any 不是立即拒绝的理由，但调用实例必须有合法类型与交付表示。

| 当前载体或入口 | 已有职责 | 尚缺的能力 |
| --- | --- | --- |
| `gradual::FrozenDescription::{Known, Any}` | 定义侧的稳定信息观察；Computed 注解可被观察为 Any | 不是已经求值的 Type 数据，不能直接放入 VM 作为返回值 |
| `open_functions::Annotation` | 已知域、待执行计算、前置 Type 输入、待恢复源码 | 没有已完成但仍含 Any/refinement 的参数描述状态 |
| `Value::Type(Term)` / `TypeParameter` | 完整或受作用域授权的抽象类型身份 | 不能保存未知描述节点、复合描述与待准入要求 |
| `type_computations::execution::publish` | 将 invocation 的 Term 结果写入保留的推断变量 | 把完成计算和解出 Type 身份绑定到同一步 |
| `type_expr` 的 `RefinedType` | 取得 base，证明要求，返回 base | 不能先保留针对未知描述的要求，再于实例准入时检查 |
| `comptime_values::encode_admitted_list` | 已准入列表的普通稳定编码 | 元素仍为 Term，描述数据不能靠 Type 列表专用旁路接入 |
| `normal_types`、值准入与 nominal formation | 按接收世界检查身份和普通实参 | 描述应用不能直接借旧身份/布局接口伪造完成 |
| `hir::ExprKind::TyAny` | 一个现存 HIR 枚举项 | 源绑定、alias、检查和执行没有因枚举存在而接通 |

[最小复现](repros/refined-any-type-producer.fl)目前在名字解析失败。不能据此判定
只缺一个 builtin 名称；即使补名称，返回、容器、构造应用和调用准入仍各有断点。
`488303ff` 的准备阶段修复、`e1c57dc9` 的 scanner 和现有 println_value 都不能
证明完整格式化接口完成。

## 2. 三种未知与两个完成点

必须区分：显式 Any 是已经得到的描述；尚未取得输入的计算仍在等待；刚性 Type
参数已有身份，只是具体内容不可观察。三者不能通过一个 Unknown 枚举项互转。

以下是内部职责，不是新语言类型或已经可用的 Rust API：

```text
TypeObservation
  KnownIdentity / KnownStructure / KnownRequirements / Unknown
TypeProduction
  Waiting(dependencies, continuation) / Running(invocation)
  Complete(TypeValue) / Failed(diagnostic)
TypeValue
  Identity(checked type reference) / Description(stable description graph)
InputAdmission
  Waiting(actual domain or evidence)
  Accepted(concrete domain, adaptations, evidence) / Rejected(diagnostic)
```

TypeObservation 回答检查器知道什么；TypeProduction 回答计算进行到哪里；TypeValue
是已经得到的普通 comptime 数据。现有 FrozenDescription 只承担第一种职责，
其 Any 可能来自一个仍未完成的计算，不能直接移到 VM 当作第三种职责。

Type producer 应完成到 TypeValue。需要具体身份的消费者再请求身份形成，不能让
所有 Type 数据返回都必须占有一个最终 PTC metavariable。AssignmentPermit 继续
保护实际推断赋值，但它属于需要赋值的消费者，不是每个 Type 值的天然所有者。

消费者需要具体身份时，Identity 走原有形成、scope 和赋值检查；Description 必须
再取得实际域或合法适配，不能将 Any 写进变量后称为已解。消费者需要参数描述时，
可接收已完成的 Description，再运行独立的准入工作。两个消费者等待同一 invocation
完成不授权重跑其 producer；生产失败、等待、准入拒绝仍是不同状态。

## 3. 描述数据与普通值通道

描述图至少承载显式 Any、已知身份引用、受检构造应用与 refinement，并保存声明
来源、binder、普通值参数和所需世界依赖。不保存活动推断变量、候选事务或未恢复
的执行栈；这些属于 TypeProduction/准入状态，不能冻结进值。

同一份 Any 描述放进列表两次，可分别接纳 bool 和 u8；同一刚性 T 出现两次则仍
要求满足 T。共享图节点表示共享数据，不表示共享一个实例变量。refinement 的
Itself 在各次准入中绑定到自己的 subject，不沿用上一次调用的变量或前提。

结构相等可用于图去重，但不是语言的类型相等证明。dump 须明确区分
`(type-value (description ...))`、`(type-observation ...)` 和等待状态，采用
已有 S-expression、确定顺序与符号转义规则。

返回、局部量、值引用、容器、闭包捕获和普通参数应保留同一描述。接收域为 Type
时检查实际 Type 值的合法性及 scope/world，不要求立即为每个 Any 选择具体类型。
`meta.List<Type>` 仍保存普通元素值，不新增 Any 列表或 Type 专用参数槽；用户
comptime struct、匿名记录和 Object 也必须通过同一准入和冻结服务。

稳定图及其依赖可以合法传送时，冻结其数据；这不代表 runtime 类型已形成。
当前 Term 混合承载身份与冻结数据，迁移要在共同冻结值层支持描述载荷，不能把
Any 塞进普通 Sort::Type 身份等式以省去改造。消费身份的地方必须明确取得 Identity。
预算不足、依赖未取得与内容非法分别报告，不能回退成 Any。

源名称由六包集中绑定提供，alias 不增加身份；不按 Any、Parse、println 的用户
拼写识别权限。Any 也不能注册成有大小的 primitive 以复用布局。源绑定、alias、
普通 Type 返回和容器传送须一起验证，不能只添加 lexer/HIR 分支便宣布可用。

## 4. 构造应用、refinement 与身份

`Parse(format)` 执行普通类型计算；`Parse<format>` 不因需要结构而执行主体。
构造输入含描述时，保留原始 head、按签名检查的普通输入和约束，形成对应描述
应用。位置选择、默认值和收集复用共同输入计划；Tuple 仍接收一个普通 List<Type>。

结构观察需要该声明的受检结构协议或既有语义规则。不能从名称、参数数量或函数体
猜测结构，也不假设所有头可逆。元组视图可提供有序元素要求，但不能抹掉头或建立
头应用与结构本体的等式。用户定义有相同受检能力时，应使用同一机制。

描述携带要求，不携带“所有将来实参已经通过”的证明。定义检查可在假设要求成立
的子上下文中检查操作；调用准入必须在原调用方上下文证明实际类型满足要求。
不能用定义端自己安装的假设自证。现有 RefinementScope 的上下文分工应保留，
但它存在不说明源码描述和 VM 传递已经接通。

Itself 上可提取的类型命题交给 PTC；关于 runtime 值、内存与状态的谓词仍属于
程序验证及规定的运行时检查。不可提取的谓词不提供静态事实。两类义务都保留
binder、来源和错误位置，不因同在 refinement 内而混用求解器。

## 5. 普通定义检查还需要结果证据

仅有 `Parse: comptime fn(str) -> Type` 并不证明结果是可遍历的 tuple，或每项都
满足 Display。一个返回 bool 类型的工厂不能因为同样返回 Type，就授权消费者
执行 tuple_enumerate。只检查格式化调用的具体实例，会遗漏这条定义端契约。

需要保留 Type producer 的可靠结果观察，来源为显式契约或普通检查可证明的返回
结构。推导只发布所有正常返回路径共同成立的信息，不执行未知输入，不任选第一条
return，也不把某次调用结果推广到所有输入。等待、分析失败和预算耗尽不提供保证。

例如所有正常返回都通过同一个有合法结构视图的构造形成，可保留共同头与参数域
信息；具体长度仍依赖 format。字段 refinement 必须来自实际描述构造或已证明
契约。经过容器及函数转发，观察仍绑定实际值来源，不能凭静态外壳扩大权限。

inline 主体也可以显式进行有依据的 Type/trait 检查，只在对应分支内调用 Display，
并对不满足条件的情况给编译期诊断。guard 证据具有自己的作用域。tuple_enumerate
本身仍需要结构依据，不能仅因参数有 `...` 就假设完整具名供应也是 tuple；具名
供应与自定义收集协议都使这种假设不成立。

结果观察必须与源检查后继共同设计；“将来会实例化”不是跳过定义检查的开关。
现有 open definition 观察逐步迁入共同服务，不新建格式器专用 AST 解释器。

## 6. 调用实例、缓存与 runtime 交付

对 `println("{} {}", number, flag)`：先按普通规则取得 format，执行所需 Parse；
取得两个字段的描述后，检查实际域与各自 Display 义务，再形成具体 tuple 类型
及资源契约，最后检查实例主体与 inline 展开。数字默认类型服从普通上下文检查，
不能由格式器固定成一种机器类型。完整具名供应不运行收集，但仍检查描述与义务。

实际 runtime 输入仍按源码序各求值一次；检查顺序不决定执行次序。不同调用的
实际类型、变量、前提和资源状态独立。错误参数不能污染后续合法调用。

描述引用依赖不可变世界 owner 及合法范围，不仅是数字 declaration id。可共享
稳定数据和元数据，不能跨世界复用某个调用的 impl 选择。Type-result 缓存仍遵循
纯性、键及可传送条件；缓存描述不缓存准入事务。不能因为格式扫描频繁而扩大非
Type invocation 的结果缓存。新的不纯调用仍是新的执行。

等待后原位恢复；取消不重放已发生的副作用；非法结果和预算失败不发布半张图。
IIR/runtime 只接收其阶段允许的类型与值。Any 没有自动 tagged union、统一指针
或动态分派 ABI。返回 Any 的函数可在具体调用中形成合法单态结果；如果 runtime
分支没有满足共同目标及表示的交付，就在该边界拒绝，不能任选一支的布局。

## 7. 生产完成与消费者完成分别发布

`3567aefe` 已使 Type invocation 的同步与挂起路径使用同一个执行 owner；它解决
取消与恢复的一致性，尚未拆开 Type 数据生产和具体身份赋值。当前
[`execution::Invocation`](../compiler/elaboration/src/type_computations/execution.rs)
在收到 VM `Complete(value)` 后立即调用 `publish`，取得 `Term` 并执行
`assign_reserved`，再将队列项置为 `State::Complete(Term)`。
[`AnnotationState::Ready`](../compiler/elaboration/src/type_computations.rs)也直接借用
这个 Term。因此不能仅将 `Value::Type` 的字段换成描述图，然后沿用现有完成路径。

随后已清理输入齐备的 required invocation：Type 返回值与其他 comptime 数据共用
普通 VM 调用和 packet 恢复，不再为其新建空推断上下文及临时输出变量。具体接线见
[状态与发布协议](elaboration-state-and-publication.md#type-生产者的-invocation-与发布边界)。
这解除了一处生产路径的人工耦合；上面的真实依赖队列仍以 MetaId 表示输出，完整
描述载体与多消费者准入尚未实现，不能据此宣布第 2 节的 TypeValue 协议已经落地。

迁移后的所有权分为三部分；以下为目标协议，不是现有 API：

| 所有者 | 完成产物 | 失败影响 |
| --- | --- | --- |
| 当前调用的 Type producer | 经域、scope/world 检查的不可变 TypeValue | invocation 错误使依赖该次生产的消费者失败 |
| 参数描述消费者 | 描述及其结构观察，实际输入到达后形成实例准入结果 | 参数数量、实际域或 refinement 拒绝仅使该消费者失败 |
| 身份消费者 | 合法具体身份及相应推断赋值 | 无法取得身份或赋值冲突使该消费者失败，不改写已经完成的 TypeValue |

producer 的身份必须独立于 `MetaId`。一次生产可以没有推断赋值消费者，例如将
结果保存到局部变量或普通容器；一个结果也可以被多个后续操作读取。具体实现可以
保留专用于身份需求的适配器，但不能继续让每次 Type 返回天然预留一个 PTC 变量。
同一个源码调用的依赖边可以引用同一生产结果；两个独立源码调用不因此合并，尤其
不能借此给不纯调用增加隐式 memoization。

执行完成先检查返回值确实属于 Type，并检验描述图内部的绑定、世界和已知身份，
再发布完整结果。未通过检查不发布，不能把任意宿主构造的图标成已完成。描述中的
refinement 在这里检查保存是否合法，不证明所有未来实参都满足要求。
消费者随后在自己的调用方环境中实例化要求；不能把实例变量、impl 选择和证明
写回共享描述，也不能用一次消费者的失败覆盖生产结果。

例如同一份“两项 Display 参数”的描述先用于 bool/u32，再用于另一对实际类型。
第二次若没有 Display 实现，它应在准入处拒绝；第一次的结果与描述本身仍有效。
反过来，描述生产过程中发生错误时，不存在可供任一消费者继续使用的半成品。
这一分工同样适用于普通用户函数，不识别 Parse 或 println 名称。

恢复时由原执行 owner 保留 VM 栈、宿主请求和剩余预算。身份消费者只在取得合法
身份后通过自己的 `AssignmentPermit` 发布；过期许可不得撤销已完成生产。
放弃某个消费者不应借其析构取消其他存活依赖仍需要的 producer；真正取消 producer
则终止该次执行，丢弃未发布结果，并使其依赖明确收到取消。即使执行已产生副作用，
也不能因消费者重试而重放。这里的读取共享与未来跨调用结果缓存是两件事。

dump 应显示生产状态及各消费边，例如下面的结构示意，而不将所有状态都打印成
`complete Type`：

```text
(type-production p0
  (complete (type-value (description d0)))
  (consumer c0 (admitted (identity t0)))
  (consumer c1 (rejected (slot 1) "missing Display evidence")))
```

`p0`、`d0`、`c0`、`t0` 是示意 owner/节点标记，不是可持久化的全局编号。
保存诊断与观察不授予执行权；稳定描述中不嵌入消费者或活动生产队列。

## 8. 同一次迁移必须覆盖的值通道

下面按 `3567aefe` 的源码核对列出入口。它补足第 3 节的载体要求，不把这些函数
当前接受的 Term 范围提升为语言规则。仅新增 VM enum 分支或注册 Any 名称，会在
后续容器、冻结或签名路径再次丢失信息。

| 当前入口 | 当前边界 | 目标职责与退役条件 |
| --- | --- | --- |
| [`Interpreter::type_description_term`](../compiler/comptime_interpretor/src/lib.rs) | 从 Type/授权 TypeParameter 取得 Term | 消费者明确请求 Type 数据或具体身份；数据转发不强制形成身份 |
| [`value_types::has_type_at`](../compiler/comptime_interpretor/src/value_types.rs) | Type 域的值走 `inspect_type_description` | 检查合法 TypeValue 成员资格；不将数据检查等同于实际参数准入 |
| [`admission::admit_value_types`](../compiler/comptime_interpretor/src/admission.rs) | Type 载荷走接收世界的 `admit_type` | 遍历描述中的身份、要求和绑定；身份引用仍须在接收世界有效，实际输入的 trait 选择另行进行 |
| [`normal_terms`](../compiler/comptime_interpretor/src/normal_terms.rs) | Type 值冻结为身份 Term | 共同冻结载体保留描述载荷及依赖；容器、Object 与捕获递归走同一规则 |
| [`normal_values`](../compiler/comptime_interpretor/src/normal_values/mod.rs) | Type 域将输入 Term 解码成 `Value::Type` | 解码后仍区分身份与描述，不根据接收域为 Type 就将任意载荷当作身份 |
| [`nir::comptime_values`](../compiler/nir/src/comptime_values.rs) | 列表字段是 Term；Type 元素用 `Sort::Type`，普通值验证入口拒绝 Type 域 | 迁移共同冻结值与域检查；不得添加仅用于 List&lt;Type&gt; 的描述字段或宽松 sort 绕行 |
| [`deferred_signatures`](../compiler/nir/src/deferred_signatures.rs) | 稳定注解为 Known(Term) 或 Computed 索引 | 保存稳定描述与计算计划；活动生产、准入事务与等待者仍留在 elaboration |
| [`type_computations::execution`](../compiler/elaboration/src/type_computations/execution.rs) | invocation 完成即赋值身份变量 | 按上一节拆分生产完成与各消费者完成；具体身份路径仍保留原赋值权限检查 |

共同冻结载体需要独立表达“这个值属于 Type”与“这个载荷是具体类型身份”。不能
靠将 Any 声明为 `Primitive`、沿用当前按 `Sort::Type` 接入身份等式的路径，或删除
列表字段的 sort 检查来迁移；这些做法会使 PTC、布局或正常 NF 参数比较误收描述为身份。
旧具体身份可以由单向适配器注入新 TypeValue 通道；反方向转换必须显式要求身份，
并允许等待或拒绝，不能提供不检查的 `Description -> Term` 转换。

冻结与解冻只保留数据及其合法依赖，不保留调用方的准入成功。持久编码、哈希、
scope 检查、substitution、预算计费和 Display 都必须按同一载体迁移；某个描述尚
不能合法冻结时明确拒绝该传输边界，不能丢弃 refinement 后缓存更宽松的结果。
这也不要求所有 invocation 都缓存：仍只有既有策略允许的 Type 结果可参与，且
其纯性、输入键、世界依赖及可传送条件必须成立。

先验证不涉及格式器的普通 Type 返回、转发与容器往返，再以两项不同实际类型的
延迟参数检查生产与准入分离；最后接通 std 的真实入口和 native 输出。各层应保留
拒绝、取消与恢复证据。公开格式化接口未接通前，`FormatPlan.parse` 仍只是扫描器，
现有 `println(comptime text: str)` 仍是待替换接口，不能通过改名报告目标已完成。

## 9. 实施闭包与验收

| 闭包 | 必须共同接通 | 完成前不得宣称 |
| --- | --- | --- |
| 描述数据 | 集中源绑定、alias、普通 Type 返回、VM、容器/值引用/冻结与世界检查 | Any 已可供 Parse 使用 |
| 描述应用 | 普通输入、列表、保留头、refinement binder、结构观察 | Tuple<Any> 已能实例化 |
| 生产与准入 | 完成结果、待准入状态、实际参数域/义务、推断赋值许可 | 添加 Value variant 即打通 defer |
| 定义证据 | 可靠的结果观察、inline 主体与待恢复后继 | 只验具体调用即可发布泛型函数 |
| 格式化库 | Parse、收集、writer 输出、错误/清理与实际 demo | scanner 或 println_value 等于 println |

旧 Value::Type(Term) 在所有消费者迁移前不能草率删除，也不能让新旧路径按库名
分流。FrozenDescription 可保留为有限的观察载体，但必须与已完成数据分开且保留
所需约束。具体 Type 快路径复用共同的形成、传送与预算契约，不再重复实现。

必须成组验收，以下是未完成要求，不是通过记录：

- Any 与 refined Any 经 const、alias、函数、List、用户 comptime struct、Object、
  值引用和冻结恢复保持含义；错误域和外世界引用分别拒绝。
- 两个 Any 字段可接纳不同类型；重复 T 仍保持同一约束；相容不产生传递等式。
  同一描述图复用到不同调用，不共享实例变量或定义端证明前提。
- Identity<Any> 保留头，不能用普通 bool 冒充；用户工厂与 builtin 共用机制。
- producer 等待/失败/取消及多消费者读取不重放前缀；缓存仅共享合法稳定数据。
- 定义侧拒绝无依据的结构和 trait 操作；非 tuple 工厂不能授权 tuple_enumerate；
  合法结果观察经转发仍有效，改写或依赖变化后重新验证。
- println 的零/多字段、转义、Unicode/NUL、不同数值类型、自定义 Display、具名
  完整 tuple、展开、参数数量/trait 错误、非法格式及 runtime 格式输入。
- 实参次数与顺序、writer 失败前缀和资源清理正确；真实 sysroot、资源检查及
  O0/O3 原生输出一致。

已有 scanner 与准备阶段修复证据见[跨阶段交付](runtime-value-delivery.md)。本设计
来自源码与正式契约核对，不把旧测试数量计作这里的能力已实现。
