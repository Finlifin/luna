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

## 7. 实施闭包与验收

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
