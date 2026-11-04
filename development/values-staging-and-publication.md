# 值、执行、代码组装与定义发布

状态：目标设计。接续 [架构总览](architecture.md)，不是新语言语法或现有 API 声明。
这组边界共同解决 NF 专用值、VM 重入、builder 世界切换和模板替换的长期耦合。

## 1. 通用值不等于一个可随意互转的大枚举

```text
LiveValue       本次 VM 存储中的值；可以含受控值引用
FrozenValue     可跨局部求值边界传递的稳定值及描述
CodeValue       带类别、上下文、来源、控制接口和义务的代码值
DefinitionDraft 尚未发布的结构化定义草稿
TypeValue       完成当前所需形成检查的类型描述
```

这些是语义能力不同的载体，不要求五套完全重复的 payload。
可共享整数、文本、持久序列、记录等 immutable 节点，但不能以共享节点为由跳过
接收域、阶段、存储及 world 检查。NF、普通实参、反射、builder、缓存使用同一值准入服务。

comptime value-reference 用 VM 的 slot/lens 表示可更新位置。读取和更新仍作用于原值，
不能以删掉指针语法后按值复制替代；同时不得导出为 Flurry runtime pointer。
嵌套字段/列表位置需维护根、路径、世代和权限。赋值使旧内容证据失效，不一律使值引用
失效；同一存活槽的引用可重新定位并观察新内容。owner 结束、槽世代不符或路径不合法
时拒绝。具体准入、快照及恢复事务见[值传送协议](comptime-value-admission.md)。

## 2. Freeze / restore 协议

`freeze(value, receiving_contract, scope)` 逐项检查实际描述、依赖、可逃逸性及预算，
返回冻结值与依赖摘要。遇到活动引用或未完成义务不得只保存裸 handle。
是否能冻结代码/类型不能由外层 Any 标注决定，要检查实际值见证。

`restore(frozen, target_world, target_session)` 重新对应声明及版本、验证参数域和契约，
再产生本次 session 的值。目标 package 中存在同号声明不证明它与源声明相同。
不把一个世界的实现证据当作另一个世界的缓存许可。

内置紧凑表示可以继续使用，例如持久树或连续标量数组，但必须满足同一值接口、
同一形成规则、同一序列语义。Type 元素的列表不获得独立语言权限。
字符串和列表哈希可以缓存不可变结构结果；源语言 Eq/Hash 协议选择仍有自己的阶段和世界。

成员资格、传送资格、物理编码和缓存资格分别交付证据。开放定义的合法 residual 不等于
当前需要具体值的 CompletedValue；局部闭包快照也不等于 metadata wire 获准执行工厂。
这些边界及现有 list 普通编码、闭包重建路径的迁移见[准入细则](comptime-value-admission.md)。

## 3. 通用上下文表达式取代 NF 专用表达式

当前 NormalFormArgument 的 If/Block/Match/Integer/List 等分支，是把一般表达式的
“等待接收域”问题复制成另一套解释器。目标为共享的上下文表达式描述：

```text
ContextualExpr {
  expression,
  lexical_bindings,
  expected_domain_observation,
  permitted_residual_reason,
  source_evaluation_slot
}
```

域未就绪时可以保留字面量和分支结构，但不能执行不确定分支，不能把任何检查失败都
降成 deferred。域就绪后普通检查生成同一 Expr/Value；NF 只负责以已检查值形成应用。
收集器、命名赋值、默认值和展开都归 ArgumentPlan，不再各自实现顺序与转换。

迁移应从普通调用和用户定义 NF 的同一依赖参数例开始，同时覆盖 Tuple/List。
仅把 NormalFormArgument 改名而保留第二套遍历器，不算消除。

当前普通子句已先分离[检查顺序与源码求值位置](comptime-value-admission.md#13-域检查顺序与源码求值顺序分离)：
静态 Type 尚未取得时保留 HIR 输入，在其域就绪后交回普通 expr 检查。它不增加新的
表达式解释器，也不提前执行 Type 生产者。NF 表达式现已采用
[统一输入准备与普通集合组装](comptime-value-admission.md#17-nf-输入统一准备检查顺序与求值位置分离)，
删除独立 positional 收集入口；NF 类型标注也消费同一计划，不再拥有纯 Type 与混合
输入的独立绑定循环。静态域使用同一普通列表表达式，连续尾部与交错产生器分别保留
各自的源码位置。函数类型具名参数接入普通表达式检查；Type 值构造函数类型时的
binder 提升由静态检查、观察与两种 VM 共用。动态 contextual 参数树、交错输入的
可恢复检查及其他 NF 消费者仍须后续迁移，不能以共享组装器声称 ContextualExpr 已完成。
开放函数的 NF 描述与具体准备已共用槽位路由，并按实际元素域处理收集器；开放
描述仍需在具体化时重新检查原始源码，不是已经冻结的可执行 Expr。

签名标注的外层域环境全部明确，或共享的词法依赖查询已证明不使用待定输入时，
现已直接生成一个普通 typed NIR region，保留控制流和绑定供定义侧 phase／算术
检查，不再先压成域描述后重复生成。查询沿完整源码边及局部声明依赖闭包观察；
预算失败不能充当无依赖证据，未映射的符号输入仍保守保留 pending。这个临时区域
不代表标注已经执行或实际类型已经取得；具体化仍消费原 suspended source。实际
依赖未知域的结构保留尚未迁移，完整 ContextualExpr 仍须携带待定域、词法依赖和
证明义务，不能把这项有限接线称为开放定义的整体行为检查已完成。证据与边界见
[已知输入域的标注区域](comptime-value-admission.md#已知输入域的签名标注保留完整行为)。


### 待定闭包需要保留可调用证据与主体，不能伪造完整函数类型

当前实现的具体断点见
[待定 Type 捕获复现](repros/pending-type-closure-annotation.fl)。源码中 `|| T`
明确给出了闭包及其零参数调用接口，但 T 的接收域尚待早前输入确定。现有
`describe_open_expression` 没有闭包描述分支，会退到普通 typed region；普通
`closure_body` 又要求完整参数、结果和捕获类型。只修复依赖发现，不能解除这个
表示层循环。`FrozenDescription::Any` 不提供完整函数类型，不能把 Any 或 Type
临时塞进 NIR 的函数类型字段，也不能调用 Domain 生产者来猜测定义侧类型。

目标是在共同 ContextualExpr 中分别保留以下信息，具体 Rust 表示尚未落地：

| 信息 | 可以支持的检查 | 不能据此作出的承诺 |
| --- | --- | --- |
| 显式闭包的阶段、参数协议、源码身份 | 调用阶段、参数供应形状、来源诊断 | 结果域已经取得、主体已经通过 safe 检查 |
| 有序 binder 和各自域观察 | 后续参数依赖、已知类型的局部检查 | 未知域自动等于 Type，或任意 Any 都可调用 |
| 词法捕获及其来源、存储关系 | 自由依赖与环境取得计划 | 当前活动环境可以冻结或进入缓存 |
| 完整主体、控制边界与待决义务 | 保留分支前提，域就绪后继续 ordinary 检查 | 每个操作孤立通过即可证明整段行为 |

可调用证据必须来自该值的源码构造、已检查签名或合法 refinement，不能只凭
变量名恢复。普通别名传递保留相应证据；赋值和分支汇合按值身份及存储版本更新，
不能永久把某个 local id 映射到首次出现的 lambda。不同分支的 callable 或
non-callable 值必须共同决定可用接口，不能任选一个分支的签名。显式 Any 的
现有精度规则保持不变，不借新增结构描述自动授予调用或成员访问权限。

形成闭包、取得捕获和执行主体是不同的行为位置。保留普通捕获语义、共享关系、
comptime 值引用与 runtime 资源约束；描述器不执行主体。每次实际调用仍按普通
规则只准备一次实参，再进入闭包的独立 return 边界。待定区域保存检查进度和
求值位置，已有生产者结果不因域恢复而重放；活动 capture frame、等待者和候选
事务仍由 region 所有，不能塞进 FrozenDescription 或全局描述缓存。

迁移顺序应先让共同区域表达式支持带待定域的 binder、捕获和主体义务，再让
闭包构造及动态 callee 消费同一接口；域就绪后由普通检查产生完整 NIR。不能只
给零参数闭包加一个局部别名表，也不能新增闭包专用执行器。当前描述器的
`describable_open_type` 在描述器中仍存在。签名分类及推断调用的 readiness 已移除
该白名单，输入发现和供应改用共同源码依赖观察；其他消费者只有在共同机制覆盖其
输入及失败路径后才可删除。已知域的局部闭包标注，以及不引用自身参数的具名动态函数类型已接通；待定域
闭包和真正依赖自身 binder 的动态 callable telescope 仍未完成。具体边界见[签名依赖接线](comptime-value-admission.md#签名分类与调用供应共享输入依赖观察)。

交付验收须包括：待定 Type 的直接／嵌套捕获、已知域闭包、别名和重新赋值、分支
汇合、参数及结果标注、未调用定义中的错误主体、阶段错误、提前返回，以及暂停后
失败／恢复不重放生产者。真实 sysroot 和两种 VM 应观察一致的次数、顺序与结果；
捕获域和义务未闭合时不能交付可执行实例。现有复现继续保留为未完成能力，不能
改成拒绝例来让测试变绿。


## 4. 两种执行载体和一个语义服务层

混合 interpretation 负责 elaboration 中必要的局部求值/观察；invocation engine
负责独立 comptime 函数的字节码调用；runtime reference interpreter 是第三个用途，
用于参考执行而非定义 comptime 能力。

应共享的服务是算术/容器语义、域检查、绑定、存储、冻结、契约和宿主请求的含义，
而非强制三者具有相同控制流内部表示。字节码编译发生在执行之前；不能先执行前缀、
发现 Unsupported 后改用树解释器从头重跑。

每个 CodeObject 包含稳定函数身份、签名、指令、常量表、来源映射、宿主请求接口、
世界依赖和验证结果。代码缓存可复用它，不共享某次 invocation 的帧或可变值。
指令必须有完整 Display；原语操作通过明确绑定身份 dispatch，不按公共源码拼写识别。

## 5. 可恢复 VM 与宿主查询

拟定协议形状：

```text
poll(session) ->
    Returned(checked_value)
  | HostRequest(request_id, operation, frozen_inputs, continuation_token)
  | Failed(diagnostic, consumed_budget)
  | Exhausted(resource, progress)

resume(session, request_id, checked_response)
```

token 仅属于本次 session，不进入冻结定义或持久 query。宿主 response 必须匹配
operation、输入和 world 依赖；重复 response 不重复安装值，不恢复已取消帧。
frame 保存 PC、局部槽、结果目的地、控制和清理进度；异常退出也要回收一次。

请求身份包含 session、activation 和动态执行序号，不能只用 PC：循环经过同一指令是
新执行，恢复同一次暂停才沿用原请求。响应写槽、推进 PC 和标记 Delivered 一起提交。
完整状态、取消和预算规则见[状态协议](elaboration-state-and-publication.md)。

若请求有真实未完成输入生产者，region 让该 VM session 等待。若宿主只是预算耗尽，
不能伪造 waiter。必需结果循环报告依赖链；已存在签名支撑的普通递归仍按函数执行。

外部不纯操作只有在源控制流明确要求、候选选择已确定后执行。请求身份与进度保证
同一会话恢复不重放，不承诺崩溃后自动 exactly-once；崩溃重启或取消后重跑的外部任务
须由 build 任务契约定义。内存事务不能回滚外部文件或进程。

## 6. 缓存分别回答不同问题

| 缓存 | 键及依赖 | 可以省略什么 | 绝不能省略什么 |
| --- | --- | --- | --- |
| 声明索引/世界准备 | immutable world revision | 重建索引及固定校验 | 使用点契约、局部实例域 |
| 字节码 | definition/body 与编译语义版本 | 重编译已验证 code | 本次执行及实参行为 |
| Type invocation 结果 | pure 承诺、实际输入、相关世界/目标、选定 Eq/Hash | 获准复用的纯计算 | 实参准备、默认供应、接收世界检查 |
| 稳定 query | key、实际观察依赖、结果及诊断 | 未变更稳定计算 | 不纯执行许可、产物存在校验 |
| 资源/布局摘要 | 实例、目标、契约及环境前提 | 重算不变摘要 | 调用者的存活/版本/借用兑现 |

Type 结果过滤是当前策略，不是 Flurry 只能缓存 Type 的新语义。
Eq/Hash 形成缓存键时递归调用结果缓存会需要自己的键，必须有明确非缓存执行路径。
键协议失败、预算不足和碰撞都不得误命中；失败不能污染下一次正常调用。

性能评估至少分 cold prepare、warm prepare、code compile、execution、freeze/restore
和 whole-source compilation。默认测量关闭结果缓存，输入交替以防测到常量返回。
记录分配次数/字节、峰值与释放成本的差别，不从一个微基准推导全编译性能。

## 7. 统一 binder-aware 变换

模板、quote substitution、实例化、inline 展开、builder 复制和局部定义移动都需要
保留绑定关系。目标提供共享的受检重绑定服务，显式声明该变换是否创建新身份。

```text
RebindingPlan {
  declaration_mapping,
  binder_mapping,
  runtime_dependency_mapping,
  control_target_mapping,
  source_origin_mapping
}
```

遍历一个构造时必须覆盖类型、默认值、子句、属性、主体、模式导出、闭包捕获和控制目标。
服务不自动决定模板产生新名义身份还是引用原身份；语义调用者先给出策略。
在所有目标预留身份后再搬迁正文，支持嵌套和前向引用；失败不发布半份映射。

### 共同结构描述不等于一个万能遍历器

现有 [NIR visit](../compiler/nir/src/visit.rs)已经区分完整表达式遍历、单个 runtime scope、
闭包环境和 handler scope；这些差别不能在重构时压成“递归访问所有子节点”。
[局部定义复制](../compiler/elaboration/src/program/function_templates/local_definitions.rs)
也已经先预留 owner/声明身份，再复制主体。目标是把这些有效约束统一成受检接口，
不是假定已有实现完全没有绑定管理。

| 遍历用途 | 要覆盖的边 | 必须保留的界限 |
| --- | --- | --- |
| 存储保活 | 节点、代码、类型、证据、来源中的全部存储引用 | 不因当前控制流未执行而漏保活；也不把物理引用自动登记为语义依赖 |
| 绑定改写 | 引用及其 binder、域、默认值、契约、模式导出、控制目标 | 每条边按所属作用域处理，不能仅按相同整数编号映射 |
| 精确自由依赖 | 真正影响相应检查/执行的引用及依赖闭包 | 嵌套声明区分定义与调用；不把无关外层输入一并捕获 |
| 运行行为分析 | 当前执行 scope 内的操作和显式进入的子行为 | 捕获表达式与闭包主体、handler 安装与 arm 执行不是同一时刻 |

节点种类的字段描述可以共享，每种用途必须明确覆盖策略与进入/退出 scope 的事件。
源码新增变体时，穷尽匹配或等价的覆盖机制要求维护者处理新边；不以默认忽略掩盖缺项。
子树共享只省结构读取，不能省略不同环境下的观察或绑定检查。

### 身份映射必须区分保留、替换和新建

```text
MappingEntry = Preserve(identity, availability)
             | Substitute(formed_value, scope_evidence)
             | Fresh(target_identity, formation_owner)
```

该描述不规定新的源码语法。保持同一个已存在 runtime 对象的依赖身份，与为一次新执行
建立局部位置，是不同选择；不能把所有 RuntimeId 一律 fresh，也不能全部保留。
新鲜 binder 的目标互不碰撞，替换可以把多个输入映到同一个已合法的值，不要求替换单射。
定义端刚性参数不因模板复制而获得被求解器赋值的权限。

按 telescope 顺序改写参数域，进入 binder 后才使它对后续项可见；删除已消费 comptime
参数时同步改写剩余域、结果、契约及环境取得路径。pattern 的导出绑定也是有顺序的
scope 增量，后续 pattern 可以依赖先前绑定，不能把整组名字无差别提前放入环境。
CodeValue 的 ordered Ψ、pattern 导出与 expr 消费仍按各自方向处理。

词法 return、循环 break/continue 与 handler/continuation 目标使用独立映射。
不能用局部变量映射顺便改控制编号；进入闭包主体会改变 return 边界，而计算闭包捕获
表达式仍处于外层控制环境。现有 [inline_return](../compiler/nir/src/inline_return.rs)
对这些边界的处理是迁移资产，不应因“统一 visitor”而删除。

诊断来源保存原定义及展开链，不用新存储编号覆盖原来源。反之，同一个源码位置出现
两次动态执行，也不共享 RequestId；动态执行身份由 activation/执行序号产生，不能
从复制的来源节点取得。保留源码来源不代表复用执行结果。

### 证据迁移不是单纯重命名

变换交付新 IR 时，对每份使用到的证据作明确处置：

| 处置 | 准入条件 | 接收方得到什么 |
| --- | --- | --- |
| 保留 | 结论、前提、世界观察、目标和相关 IR 内容没有变化 | 仍与同一事实绑定的原证据 |
| 受检迁移 | 证明系统支持此次合法重命名/替换，形成条件和前提同步迁移并重放通过 | 对新结论成立的新证据 |
| 重新检查 | 变换改变依赖、控制、资源路径或适用域，原证明不再直接适用 | 新检查产出的证据；未完成时不交付可执行产物 |
| 消除 | 受检变换证明对应义务已不再需要 | 消除依据，不是任意丢弃旧义务 |

通用重绑定器只生成候选映射和受影响范围；各证据检查器判断能否迁移。
PTC 证据不能为环境存活背书，资源证据也不能为类型等式背书。
若局部重放失败，应重新检查或拒绝该变换，不把旧证据强行标成新 IR 的证据。
候选可以私下带未决项，但未闭合必需证据的结果不具有 `ValidatedInstance` 交付资格。

例如把一个实参值临时绑定后多次引用，与把它的求值表达式复制多次，类型可能相同，
执行次数却不同。前者仍需相应访问/资源许可，后者不能靠 alpha-renaming 获得许可。
参数或字段物理消去也不能顺便删掉源求值和 Drop；证明纯性不自动证明有限完成。
这些是变换本身的语义义务，不能只交给机器 ABI 或最终数值测试发现。

新生成代码仍经 normalization/safe 检查；已检查 IR 的合法变换维护相应证据，
verified 模式另按既定阶段检查。不能用“后面还会验证”跳过定义端应完成的 safe 义务。
变换失败保留原 IR 与证据，不能交付新主体配旧环境/cleanup 的混合结果。

### 精确词法依赖

以声明为单位收集签名、参数域、默认值、契约、主体和属性的自由引用；对局部调用形成
依赖边，求递归组的必要依赖闭包。保留 required-domain 的前置依赖，不能只扫描函数体
出现的变量名；也不能把所有外围子句直接复制成隐藏参数。

局部 `flag() { true }` 若不依赖外围 `.inverted`，在该默认值中调用不应等待 inverted
自身。若 helper 的契约或调用链确实依赖它，必须保留真实循环/不可用诊断。
结果用于模板复制和闭包环境规划，仍分别处理 comptime 输入与 runtime capture。

## 8. CMTT 的值与操作

CodeValue 的静态描述和实际证据分开，实际值始终保存 expr/pattern/arm 类别、ordered Ψ、
主体、固定外部依赖、控制接口和来源。Any 容器读出的值只能使用该次读取对应的证据。
pattern 接入产生新绑定；expr 接入消费已有上下文，不能共用错误方向的 substitution。

单洞是每段 quote 的构造限制，不是上下文只能有一个变量，也不是整个生成器只能组合
一次代码。新的组合仍需逐层满足既定限制；实现不能为方便 regex 生成静默放宽语法。
quote fn 的普通实参先建立新鲜绑定，主体重复引用也不重算实参。
生成自动机的验收应观察真正组装后的分支/状态代码，而非用解释 regex 字符串绕开生成。

## 9. DefinitionTransaction

目标让 struct/enum/newtype/fn/trait/impl 等走共同发布协议：

1. 在批次 owner 中预留声明与 binder 身份，草稿保留未完成结构。
2. 基于固定 WorldSnapshot 创建私有 overlay，解析生成代码的显式引用和来源。
3. 用普通接口检查与 elaboration 验证完整批次；名义互递归用已准入签名，不用占位成功。
4. 验证发布所需的全部接收方（声明目录、VM、实例/协议索引）并准备 receipt。
5. 核对基线 revision，原子发布后继世界；竞争收据失效，旧世界仍可只读使用。
6. 恢复原 invocation，返回可在后继世界使用的句柄/值，不返回活动 checker。

上述安装通过同一 PublicationBundle 版本根交付目录、定义、VM 描述和协议索引；
用户计算与失败性检查都在 prepare 中进行，install 不再逐个调用可失败回调。
旧代码仍绑定原语义视图，后续操作通过 staging plan 显式取得新视图，不能整体改标旧缓存。
逐批安装只授予对应后续位置可见性，最终实例交付仍需整体 coherence。
详见[发布的线性化点](elaboration-state-and-publication.md)。

结构分配失败、语义失败、接收世界拒绝、revision 过期都不泄漏部分声明。
事务保留可修复草稿；预算消费不回滚。预留编号可以留下空隙，不能将已观察身份复用给
不同定义。源码创建和 builder 创建共享检查规则，不为“简单 struct”另造长期语义通道。

## 10. 删除旧路径的条件

| 旧路径 | 删除前必须看到的生产行为 |
| --- | --- |
| NF 专用表达式/值求值 | 普通 callable、用户 NF、Tuple 与 List 经过同一计划及冻结路径 |
| invocation 的树 fallback | 支持范围内每种操作均走字节码，拒绝在执行前确定，次数/失败一致 |
| per-kind builder 桥接 | 多种 definition 在同批发布及活跃调用恢复时用同一 receipt 协议 |
| 手工零散替换 | 嵌套 binder、契约、属性、控制目标和来源均经统一变换验证 |
| 全量外围捕获 | 递归局部声明的精确依赖和真实循环均有源码证据 |

混合 interpretation、runtime reference、原子发布及接收世界检查不在删除名单中。


## 当前 NIR 迁移：残余值不等于冻结数据

[依赖观察 §5.5](dependency-observations.md#55-invocation-残余值与冻结值分离)已接通
`StagedValue(Term)`：共同替换按 invocation 槽是否仍存在区分残余表达式和冻结值，
实参供应后再恢复普通值表示。它不新增源语言的值种类或新的缓存资格，也不是完整
OpenDefinition/WorldView 已经落地。冻结入口、实际执行入口及 IIR 仍要求相应完成证据，
不能借参数域已知就取得还没有供应的输入值。


## 当前显式 invocation 迁移：统一源世界 VM

普通 CLI／`luna::invocation` 与 `PreparedInvocation` 已删除“同时遇到普通 Type
参数和 trait／关联值就先实例化整份 IIR”的分流。准备对象只持有源 package、入口身份
与源代码 session；执行统一通过源世界 VM 绑定本次实参、使用 demand resolver 选择
方法和关联值，并运行原有类型与程序契约。`needs_materialized_dispatch`、可选 IIR
字段、两类构造器和两套调用出口已从普通入口删除。

这条分流的历史前提是源 VM 尚不能按实际参数解析方法。现在该服务已存在，继续在
入口根据可达语法扫描切换世界，会使同一种 comptime 调用受到两套准备责任约束。
普通入口仍调用 `dispatch_instances_for_invocation` 准备可用 dispatch，并保留已接纳的
实现关系见证；这不是执行 IIR materialization。代码 session 复用代码，调用值、局部
存储、契约状态与 builder 服务仍在每次 invocation 中建立。

带程序前提的独立 callable 转换仍遵循[函数值契约边界](../docs/functions/comptime_fn.md)：
当前普通函数类型未携带该前提时拒绝转换。IIR 不提供擦除前提的许可，不能把这个拒绝
误认成必须保留整条 comptime IIR 分流的理由。直接方法调用继续消费接口契约，合法的
无前提方法值可以保存和经高阶参数调用。完整携带程序契约的 callable 仍是后续能力，
本次没有通过删除检查来扩充其支持范围。

移除分流后，VM 构造的诊断责任变得明确：`nir::phases::validate` 的具体失败现在通过
`ErrorKind::InvalidPhase` 保留原因与源码位置，`Display` 输出原始原因。此前全部被压成
PhaseMismatch，导致“方法值会擦除接口契约”等形成错误失去解释。实际执行错阶段仍
使用原 PhaseMismatch；构造期验证与调用期权限错误分别报告。同一不可变 source 的
验证成功证据仍由 code session 持有，失败不会安装索引，也不复用同号但不同世界的证据。

这一步首先迁移普通显式 invocation；当时保留的 `memoization::staging` Type 输入
IIR 专门化现已在下述后续迁移中删除。运行时 IIR、后端交付以及引用执行器继续使用
各自协议。完整通用参数计划、动态 Type producer、调用中的世界发布和树 fallback
退役均未因此完成。

### 本次验证与未修复基线

工作目录均为 `/home/fin/luna`；下表命令参数接在
`python3 -B development/check.py --timeout 900 -- test` 后。
证据目录中的 `result.json` 保存完整命令与退出结果，`stdout.log` 保存断言输出。

| 参数／范围 | 结果与证据目录 |
| --- | --- |
| `-p comptime_interpretor --lib --no-fail-fast -- --skip normal_input_data_tests::clause_list_inputs_share_domain_scope_and_budget_validation` | 退出 0，446 通过、1 项已知基线跳过；`/tmp/luna-tools-6ce5etz8` |
| `-p luna --test execution -- ordinary_type stored_staged_methods_reject_erased_interface_contracts conditional_method_signatures_are_materialized_only_when_called`，另跳过下述 3 项 Type 前端基线 | 退出 0，19 通过；`/tmp/luna-tools-jqapa_na` |
| `-p luna --test prepared_invocations --test invocation_value_templates --test comptime_pointer_phase --test comptime_storage_domains --test comptime_iir_bytecode --test comptime_bytecode_defaults --no-fail-fast` | 退出 101，36 通过，1 项仍断言旧 PhaseMismatch；`/tmp/luna-tools-ghtesw9n` |
| `-p luna --test comptime_bytecode_defaults` | 补充默认参数顺序／绑定帧错误的具体原因和来源断言后，退出 0，9 通过；`/tmp/luna-tools-ywr1egel` |

新增真实 sysroot 用例同时覆盖 Type 驱动的 trait 方法／关联值、保存并高阶调用方法值、
重复 invocation 的新鲜局部状态，以及预算失败后恢复。既有接口前提擦除拒绝例在迁移
初期失去具体原因，补齐 InvalidPhase 后原断言通过；没有放宽 callable 转换契约。
默认参数拒绝测试分别检查“默认参数顺序／绑定帧错误”和“显式参数出现在默认参数后”，
保留引用执行器自身的 InvalidCallPlan 防御检查。

扩大检查发现以下 5 项源码前端失败；临时恢复本阶段改动前的生产文件后，原失败重现，
随后恢复当前实现。它们都在 invocation 前失败，本次未修改其预期或实现：

- `enum_builder_edits_payloads_groups_and_runtime_patterns`：fixture 中 Symbol 未解析。
- `known_type_list_index_failures_remain_invocation_failures`：静态观察整数不能以目标定宽表示。
- `ordinary_type_inputs_do_not_hide_selective_memory_writes`、
  `ordinary_type_memory_summaries_forward_across_different_input_positions`：程序后置条件未证明。
- `ordinary_type_list_annotation_inputs_report_the_phase_boundary`：required comptime 输入不可用。

前两项基线复现为 `/tmp/luna-tools-lbxrkexa`，后三项为 `/tmp/luna-tools-j0r5g11s`，
均退出 101。VM 跳过项是此前已确认的名义列表域失败分类差异，仍待处理；不能将上述
局部成功称为全量通过。修改的 Rust 文件格式检查与提交范围的 `git diff --check` 通过；
正式 docs、已有资源分析修改及 library README 保持原样。

## 当前缓存 invocation 迁移：去掉 Type 输入专门化世界

`memoization::Invocation` 现在也只持有一个源 package 与对应 CodeSession。准备阶段
用 `dispatch_instances_for_invocation` 保留合法输入模板；执行时，声明实参与本次普通
实参直接交给源 VM，方法和关联值通过现有 resolver 按实际输入选择。结果的成员资格
与冻结准入由执行它的同一 VM 检查，不再为 IIR 返回值另建一个源 VM。

已删除 `memoization/staging.rs`、`prepare_bound_entry`、`admit_source_result` 和最后
一个 `needs_input_staging` 消费者及其可达性扫描。Type 实参变化不再触发 CLI 层复制
package、挑选伪造私有声明身份、移除 Type 实参并实例化整份 IIR。旧的
`staged_preparations` 计数 API 一并删除；实际代码编译／执行统计仍由
`last_execution_stats` 提供，结果缓存数仍由 `cached_results` 提供。

移除旧路径时观察到依赖注解主体每次重编译的问题，因此同步补齐 VM 的主体准备缓存。
缓存按原声明或精确 owned body 身份定位，再比较完整 FunctionInputBinding，包括
替换项、专门化后的签名和槽位域细化。签名相同不等于绑定相同；Type、普通值索引及
动态成为 Type 域的输入不使用互相独立的特判键。新添的只读 `terms()` 观察用于预算
预检，完整绑定相等仍是命中条件，不能只比较该观察返回的项而遗漏槽位细化。

每个主体保留至多一个依赖绑定对应的已准备主体，返回相同 Arc 供现有 owned bytecode
缓存复用。比较与复制前按当前预算检查双方键，存储权重同时计入主体和绑定，总量沿用
既有上限。成功替换时扣除原项权重；预算失败或重绑定失败不替换旧项。抽象解释中的
依赖主体不发布到该共享缓存；owned source 使用弱引用核对身份，不因缓存延长源主体
寿命。这不是新的类型等价关系，也不保存本次局部值、可变存储、契约执行结果。

不依赖输入的主体仍能跨 Type 实参复用。依赖主体的重绑定缓存与字节码缓存只消除准备
工作，输入准入、调用契约、实际执行和结果检查仍按本次调用进行。Type 结果缓存保持
原策略：只接受纯函数的已绑定 Type 结果，用实际输入与源 Eq/Hash 协议判定复用；
非 Type 结果不查询结果缓存。没有扩大缓存资格，也没有把 Type 实参变成 NF 特殊槽。

此迁移不等于完整通用参数计划已经实现。memoization 的输入域绑定仍是现有适配器，
一般值依赖域、待定 Type producer、定义发布与可恢复 invocation 的后续迁移继续遵循
本章目标。主体缓存是精确源世界内的有限复用，不承诺跨编辑持久化或无限保留实例。

### 验证范围与仍未接通的路径

工作目录 `/home/fin/luna`，命令前缀同上。

| `check.py ... test` 参数／范围 | 结果与证据目录 |
| --- | --- |
| `-p comptime_interpretor --lib --no-fail-fast -- --skip normal_input_data_tests::clause_list_inputs_share_domain_scope_and_budget_validation` | 退出 0，449 通过，跳过此前已确认的 1 项列表域失败分类基线；`/tmp/luna-tools-2wvj6e01` |
| `-p luna --test invocation_value_templates --test staged_invocation_reuse --test prepared_invocations --test comptime_memory --no-fail-fast` | 退出 101；存储 14、prepared 3、reuse 4 项全部通过；新 fixture 的 Type 键协议准备失败，见下文；`/tmp/luna-tools-um389xj0` |
| `-p luna --test invocation_value_templates` | 调整新用例的输入选择后退出 0，4 通过；`/tmp/luna-tools-7llybjxc` |
| `-p luna --test execution -- memoized` | 退出 0，13 通过；`/tmp/luna-tools-y5vs07g_` |
| `-p luna --test comptime_bytecode_defaults --test comptime_iir_bytecode --test comptime_storage_domains --no-fail-fast` | 退出 0，9 + 10 + 7 通过；`/tmp/luna-tools-5uljt7zd` |

移除 IIR 后最初出现的重复字节码编译由原测试检出：
`memoized_invocation_checks_dependent_type_contracts_before_caching` 在
`/tmp/luna-tools-o3j4u847` 失败。补齐共享主体缓存后，其原有“再次调用不编译字节码”
断言通过，未删除或放宽该断言。删除的只是已经不存在的 IIR preparation 计数；
不依赖输入的源码主体现在也要求在 Type 实参变化时继续复用代码。

新增真实 sysroot 用例以 bool 作为结果缓存键，运行分支中的 `SelectFor(bool/u8)`，
由 Type 实参选择 trait 方法值并与关联值核对；重复键命中、错误输入拒绝、关闭结果
保留后的执行／准入计费、预算耗尽及恢复均有断言。内部缓存用例另覆盖相同签名但不同
绑定、普通 bool 值索引、精确源主体身份、权重替换、失败不覆盖以及抽象调用不发布。

编写该用例同时确认了未完成能力，不能据此声称任意 Type 输入缓存已经可用：

- 当时 meta 的 ComptimeEq／ComptimeHash 都没有 Type 实现。后续已接通
  [Type 的 ComptimeEq](libraries-and-delivery.md#12-type-协议比较接入既有类型等式)，
  ComptimeHash 仍未接通，直接为 Type 输入准备完整键协议仍会失败；不能提供恒定
  Hash 占位实现掩盖缺口。
- `SelectFor(if flag { bool } else { u8 })` 的 required Type 输入当前不能依赖尚未供应
  的外层 flag，报 `required comptime input ... is not available`，证据为
  `/tmp/luna-tools-ng7jwxcd`。分支内的具体 Type 调用已通过，不代表一般待定 Type
  参数计划已接通。初始 fixture 的 assumes 中直接混用 Type 命题也未被当前前端接受。

上述新 fixture 的探索未改动前端规则、库协议或既有测试的语义预期。Rust 格式检查
`/tmp/luna-tools-yqcdaez2` 退出 0，提交范围 `git diff --check` 通过。已核对旧 staging
模块及扫描／准备函数不再有 Rust 消费者；正式 docs 和先前已有修改保持原样。

## 当前键协议迁移：共享代码与逐次执行分开

`comptime_keys::Protocol` 不再为每次快照准入、Eq、Hash 从零准备源世界与代码。
固定 Binding 持有协议选择、类型域、证明上下文、实现索引和 Arc<Package>；独立的
Execution 持有绑定到该精确 package 的 CodeSession 与最近 VM 操作统计。普通
invocation、缓存 invocation、键方法使用同一个 demand method resolver，删除键路径
重复维护的选择／计费适配逻辑。

每次操作仍构造新鲜 VM，安装原类型证明、方法／关联值和 Copy 证据服务，再检查本次
输入与执行协议正文。CodeSession 仅复用源索引、已准备主体和字节码，不复用用户
Eq／Hash 返回值、程序契约结论或接收者存储。构造失败保持原预算预留规则；执行失败
由同一代码 session 退出协议收回代码，按 VM 实际消费同步调用者预算。输入复制的
预算预留若在进入 VM 前失败，不伪造一次已执行的 VM 操作。

为显式表达这份可变执行缓存，Rust `Protocol::admit` 改为 `&mut self`，memoization
中相应的协议列表也通过可变借用准入新值，不引入隐藏的内部可变会话或共享活动 VM。
Snapshot 的 owner 仍属于唯一 Protocol，两个 Protocol 即使来自同一源 package，
也不能混用快照。`last_execution_stats` 返回最近一次 VM 准入或方法执行的字节码
编译／调用次数；在进入 VM 前被拒绝的操作不会产生新的执行统计。

本次没有改变结果缓存策略、源 Eq／Hash 的语义权限或 Type 哈希缺口。CodeSession
只在其精确不可变源世界内有效；同号声明在新世界中的代码不能继承旧缓存，也不能
反过来改变仍存活的旧 Protocol。

### 验证

工作目录 `/home/fin/luna`，以下参数接在
`python3 -B development/check.py --timeout 900 -- test -p luna` 后。

| 参数 | 结果与证据目录 |
| --- | --- |
| `--test key_code_sessions --test library_list_hash --no-fail-fast` | 退出 0，3 + 13 通过；`/tmp/luna-tools-qqoqdh14` |
| `--test key_code_sessions --test library_list_hash --test invocation_value_templates --no-fail-fast`（迁移初次检查） | invocation_value_templates 的 4 项通过；另有新预算断言与旧成本测试的测量前提需调整，整条命令退出 101；`/tmp/luna-tools-t5g4c3m7` |
| `--test execution -- memoized comptime_key comptime_snapshot pure_query` | 30 通过，1 项复制成本断言失败，退出 101；`/tmp/luna-tools-yyiiga0m` |
| `--test library_hash --test library_comptime_comparison --test comptime_memory --no-fail-fast` | 退出 0，18 + 3 + 14 通过；`/tmp/luna-tools-8yqcdrlt` |

新真实 sysroot 用例检查：重复 Hash／Eq 不重编译但仍执行方法，交替输入不复用答案，
方法修改 self 不污染保存的输入快照，程序条件逐次检查且失败后恢复。同号声明修改
主体后新旧 Protocol 使用各自代码，交叉快照拒绝。预算检查覆盖冷／热调用、输入预留
拒绝、执行中耗尽与恢复；热调用仍消耗预算。

原共享载荷成本测试比较小／大 payload 时，首次调用包含代码编译、后续调用已预热。
现在在两种大小测量前均预热代码，再保留原成本相等断言，并断言测量时未重编译；
没有改为不等式来掩盖逐字节复制。新预算用例按 Context 的既有预留契约区分尚未进入
VM 的失败与已执行工作的计费，不改变 Context::spend 的实现。

唯一尚未处理的扩大检查失败为
`comptime_snapshot_admission_checks_enum_tags_payloads_and_nested_references`：
包含未知 Function 身份的 tuple，其 `immutable_copy_work()` 当前并不返回 None。
恢复本阶段之前的 comptime_keys.rs 后，`/tmp/luna-tools-2b2_r_h3` 仍在同一断言失败
（退出 101），之后已恢复当前实现。本次未修改该预期；复制成本与世界准入的责任边界
需单独核对，不能据此声称完整回归通过。

Rust 格式检查 `/tmp/luna-tools-k8e_6o4o` 退出 0，提交范围 diff 检查通过。
正式 docs、已有资源分析修改及 library README 均保持原样。
