# 重构顺序、退役条件与验收

作者已要求开始清理；在已收敛设计上分批推进源码退役与生产迁移。
目标不是换一个更整齐的兼容壳，而是逐步消除已确认的语义偏差及信息丢失。

## 1. 总体顺序

先确立身份、稳定产物和事务所有权；以普通参数/值/类型为主干接通执行和生成；
再把运行时依赖、资源、ABI 和真实库联成完整交付。稳定 query 的边界从一开始参与，
持久化和并发放在生命周期及证据协议成熟之后。

每步为可验收的纵向迁移，不要求一次重写全编译器；也不允许只新增未被使用的平行类型。
阶段编号是依赖顺序说明，不恢复 graph engineering 或调度账本。

## 2. 阶段与删除边界

| 阶段 | 实现范围 | 必须真正替换的旧路径 | 完成证据 |
| --- | --- | --- | --- |
| M0 语义与架构整理 | 本目录、证据核对、文档退役 | 时间流水账充当现行设计 | 当前状态、目标、未完成和证据分开；不改变语言设计或正式 docs |
| M1 身份与产物所有权 | stable world、region、candidate、来源与受限产物构造 | 跨边界共享临时状态、隐式当前世界 | 失败/取消不泄漏；不同世界同号句柄拒绝；生产入口使用新所有者 |
| M2 通用参数和值 | ArgumentPlan、ContextualExpr、freeze/restore、binder 变换 | NF 专用表达式/TypeList 权限、重复参数准备 | 用户 NF 与真实 Tuple/List 同路径，域/顺序/默认/失败一致 |
| M3 交织执行与发布 | invocation VM 帧、宿主请求、DefinitionTransaction、精确词法依赖 | invocation fallback、per-kind 发布旁路、全量外围捕获 | 活跃调用中生成并调用定义，重复唤醒无重放，子句 helper 正例通过 |
| M4 生产 TypeRef 与 IIR | 类型引用、lift 生产、EnvironmentPlan、NIR safe 接线及高级实例 IR | 普通注解仅 Term、仅身份集合的依赖交付 | 同 core 不同对象跨调用/捕获/返回，safe 证据与环境共同进入 IIR/后端 |
| M5 资源和库贯通 | 来源/效果摘要、text region、cleanup、inline effect、allocator 边界 | pointer-only 消费者、str 直降绕路、重复来源猜测 | 真实 sysroot 文本及迭代、所有出口清理、死亡来源拒绝 |
| M6 目标与工程交付 | ABI、pipeline、artifact 依赖和稳定 query 覆盖 | CLI 层隐含语义、只缓存整前端而缺细粒度依赖 | 改主体/接口/impl/配置各有正确失效；native O0/O3 和产物恢复 |
| M7 退役与规模验证 | 删除已替换实现、停用遗留目录审计、持久化及并发接入 | 两套生产真相、临时 adapter、未被消费者使用的新结构 | 唯一生产接线、跨编辑/失败恢复、规模成本与完整回归 |

PTC 的完整理论协议和 Type 模型搜索沿 M1–M4 持续推进：M1 定 world/guard/候选事务，
M2 接通普通值域，M3 处理可恢复宿主观察，M4 消费 TypeRef 及身份域。
不能等所有模块重写完才做 solver，也不能用不完整 PTC 给资源层提供伪证明。

M5 是资源机制与库场景的进一步贯通，不是推迟 M1–M4 所涉代码的 safe 检查。
每个阶段交付均要满足相应 normalized IR 的 safe 义务；IIR verified 与后端证据维护
不替代它。具体 TypeRef、环境和消费者迁移见[类型引用与环境](type-references-and-environments.md)。

## 3. 当前迁移使用的架构边界

M2 先接通[统一值准入](comptime-value-admission.md)及普通参数的真实消费者，保留既有
带域 list 编码和闭包共享快照；再让 M3 的宿主响应、代码值与发布使用相应传送契约。
不能先删除 normal_terms/normal_values 中的检查，或把活动值引用统一复制成冻结数据。

Type 描述迁移跨 M2/M3，完整接线按
[共同冻结载体与 binder](type-descriptions-and-admission.md#9-共同冻结载体与-binder-的实施选择)
推进：VM 返回、聚合／NF 数据边、普通输入绑定、身份消费者和源码注解一起闭合。
`627ab804` 已让输入齐备的 required Type 返回走普通调用，不再临时建立推断输出；
旧 Type 队列仅保留为独立测试模型，已退出生产模块。不能继续给每种描述加一个队列状态就宣布完成，
也不能只放宽 List 的 sort 检查；冻结值及调用绑定必须先能保留描述的实际含义。
这条迁移与下面的待定域源码恢复共同支撑 Parse，二者都不由 scanner 代替。

源输入已经形成也不等于 runtime 值已经构造。[阶段交付设计](runtime-value-delivery.md)
把参数/常量/结果的消费角色放在 M2，把受检 Splice 调用、捕获和契约残余接入 M4，
再与 M5 的目标存储来源和真实文本 writer 贯通。现有 `iir::staging::wrap` 不能先删：
必须先接通基础构造终点，避免标量身份实现自递归，再逐项接管主体、契约、默认值和
捕获。对照已可执行，直接返回 comptime 参数仍是待修正例，不改变既有待定域主线。

布局观察在 normalization 的反射和 safe 检查中已经需要，不推迟到 M6 才设计。
M4 接通 TypeRef/环境与布局计划，M6 完成目标调用分类与交付；具体生产者、消费者和
repr_at 退役条件见[布局与调用 ABI](layout-and-call-abi.md)。仅新增一个布局容器不算迁移。

构建入口和输入身份从 M1 起参与，不能到 M6 才补 build 来源。M6 集成
[任务与产物协议](build-and-artifact-delivery.md)，在保留现行 wire/resident v1 及
单文件输出保护的前提下接通字段观察、IIR lease、完整 manifest 和恢复消费者。
既有工具超时/输出检查继续使用；尚未形成稳定 schema 前不提前承诺跨进程恢复。

不从把 program.rs 分成更多文件开始。先在其实际生产入口确定：

1. 哪些声明数据可形成不可变 snapshot，哪些字段确实属于 Region/Candidate。
2. `ensure_signature` 的输入依赖及输出契约；父签名、nominal capture、staged input
   不再用“先继承全部”混合表达。
3. 同一 ArgumentPlan 是否覆盖普通函数和 NF，且能独立保存域观察与执行序列。
4. VM 对前端的请求需要哪些稳定输入，哪些结果可冻结，哪里必须暂停调用。
5. TypeRef 的生产端要取得哪些依赖域与环境路径，哪些消费者需要一起迁移。

现有 `TypeInterner`、候选 Engine、PTC trail、CodeSession、DefinitionBatch 均先评估
是否能作为这些接口的实现；不因它们旧而全部重写，不因已有而保留错误责任。
重绑定迁移保留现有 visitor 的执行 scope 区分，接通各证据检查器的保留/迁移/重检/消除
结果；不能仅统一结构遍历，就声称类型、资源、控制和来源的跨层迁移已经完成。

[状态与发布协议](elaboration-state-and-publication.md)已给出 owner 划分、状态转换、
安装边界及恢复要求；[依赖观察](dependency-observations.md)细化接口前缀、精确捕获和
查询失效。[世界存储](world-storage-and-lifetimes.md)选择分块、持久索引及回收边界，
并列出旧 arena/侧表退役条件。这些属于 M0 设计成果，不能计为 M1/M3 实现完成；
具体 Rust 接口和生产消费者迁移仍需完成。
读取/草稿/安装接口按该文的所有权约束接线：跨暂停保存 ID 与 owning snapshot，
stale 返回冻结准备物；不能以单块 pin 或静态 lifetime 擦除替代跨块依赖保活。

求解线按[Type 模型与理论证据](type-models-and-theory-evidence.md)保留已有投影搜索与原义务
复核，接通带世界证据的联合模型；不得以推断出具体值代替候选选择、资源或调用交付。
新增 theory 时分别落实原子准入、正负证据和模型覆盖；用有限域冲突、开放行、未解释
predicate 等混合问题验证组合入口，不能只用各 theory 的独立成功案例作为接线证据。

## 4. 一条主干切片的验收方法

以“用户定义工厂接收普通值，产生带依赖的类型，被 builder 和普通调用使用”为切片：

- 从真实源码和 sysroot 输入，不在测试中手工注入已检查结构绕过生产者。
- 展示 HIR 来源、参数计划、冻结值、TypeRef、调用/环境证据和 IIR dump。
- 对域错误、NF 不展开、scope 逃逸、预算耗尽、失败后重试分别给拒绝/恢复结果。
- 检查实际调用次数和执行顺序，不只检查最终数值。
- 旧专用路径在该切片中不再被调用；迁移完成后删除对应生产分支及过期测试预期。
- 同时验证 public meta/builtin 的对应接口，不能只交内部 Rust 类型。

六包的现有入口、协议身份迁移和真实 sysroot 验收边界见
[库迁移接线](libraries-and-delivery.md#9-当前生产入口与迁移接线)。保留已有 builder 的
公开用法并替换底层发布责任；不通过复制 trait 声明、重复原始绑定或去掉资源检查迁移。

不要求这条切片等于整个 Flurry 完成；但也不能把大量未迁移消费者藏在“已经有 TypeRef”后面。

### 已交付部分不重复立项

局部声明输入需求、调用供应及 required 残余已沿同一词法依赖机制接线；普通 callable、
NF、名义类型和透明别名共享声明输入准备，原 Type-only 和名义成员的独立循环已退役。
contextual 签名、普通值读取与捕获共用父声明输入；跨 VM callable 在接收世界核验。
源码 VM 与普通显式 invocation 已借用原包的逻辑声明视图，避免为剔除旧 dispatch
复制整包；封闭执行和源码查询仍保持独立准入、索引及权限，详见
[所有权边界](world-storage-and-lifetimes.md#110-源码声明视图与物理执行表分开)。
这些是 M1–M3 的部分纵向交付，不是通用 Region、ContextualExpr 或发布协议已经完成。
当前入口及证据集中在 [README](README.md#当前生产接线与下一步)，不再安排重做旧 flag
helper 调查或已完成的停用 crate 删除。

### 下一条纵向交付：待定域的普通调用

以现存且本次仍失败的两个正例推进，而不是把它们改为预期拒绝：

```flurry
comptime fn Byte() -> Type { u8 }
comptime fn count() -> Integer
where T, ...values: meta.List<T>
{
    values.length()
}
comptime fn identity(value: T) -> T
where T, flag: bool
{
    value
}
```

`count<.values = [1, 255], Byte()>()` 需要先保留有待检查的输入，在 Byte 产生 Type 后
完成列表域检查；`identity<Byte(), true>(9)` 还要求该实际 Type 完成 callable 签名，
再给普通实参提供 u8 上下文。Byte 是普通 comptime 执行，不能按 NF 头处理或展开
一个 opaque 头来推导其结果。列表只是接收域的一种消费者，不因此获得特殊等待规则。

| 现有生产入口 | 当前已经保留 | 本次需要贯通的交接 |
| --- | --- | --- |
| `declaration_inputs.rs` | 路由、源码求值顺序、默认值、前置域与形成义务；Progress 独占 producer、身份，检查完成次序与源码 schedule 已分离；槽位帧保留子检查目标、输入事实退出与默认 guard，依赖域帧保留 consumer 与传递依赖游标，显式／默认依赖均交给同一槽位检查，收集器共用；收集帧保留普通探测、contextual 回退及各源码位置的 preparation，路由帧保留展开包检查与源码位置，准备帧拥有阶段交接及 caller scope，参数望远镜与默认环境不再借用外层临时变量，四类子检查共用请求与驱动；字面量列表展开不再递归重建整次准备；已检查 producer 按所有权交付；NF 默认代码连接本次前缀绑定，由 VM 取得实际前缀后实例化；声明 NF 应用现持有准备帧并通过共同 Region 栈交接参数子检查、失败恢复和形成完成，直接嵌套声明／常量头不再同步递归驱动准备；动态 NF 位置调用也已接入同一槽位和收集计划，源码默认提供者与入口投影分离，域替换使用本次持有的 telescope，不再依赖声明域回查 | 布局提示扫描、其余非 NF 消费者及未迁移子表达式仍同步；caller scope 与默认环境已经按预算保留，但仍复制词法表；共用输入驱动尚不接收 VM 产物。须让 `InputChecking::Typed` 不再迫使待定域立即产出完整 Expr，并接入原源码与执行区域 |
| `dynamic_instances.rs` | packet 持有源码顺序的 producer 和本次调用的替换。依赖从被调声明的原始域及签名读取；父实例前缀与已观察到的成员实参同时替换。递归调用中，来自 caller 的同一声明 ID 不会再次被 callee 替换，也不会被当成 callee 的待定槽位；comptime callable 子句应用与 opaque NF 共用 Region 子检查适配，直接声明的嵌套 callable 可在同一帧栈推进，最终签名及函数值交付由 InstanceCompletion 持有；完成消费者由共用准备帧持有，caller scope 恢复后检查形成义务，已知域严格检查，依赖未执行输入的域保留准入义务并提前拒绝已证明冲突；完整受检 packet 在原 VM 执行后检查实际声明域，required runtime 消费者也在完整实参齐备后重新检查；词法前缀及默认环境按预算保留 | 尚不能保留自身待检查的源码表达式；pending 参数影响签名时仍须保留尚未发布的调用准备，不能把 rigid 当作已完成 callable 类型。已受检 packet 的成功不等于前述两个正例已完成 |
| `invocation/host.rs::InvocationSession` | 独占执行现场、关联／方法选择与定义准备请求、线性响应及取消 | 一般前端表达式检查仍需接入；不把同步回调或另起一次求值当成恢复 |
| `invocation/session.rs` | 生产 invocation／plan 在 session 内拥有执行帧；输入准备通过帧绑定契约整体移动导出值；借用根帧的 fragment 入口仅用于测试 | 源 activation 接收后继受检片段，保留环境、原控制出口及一次性执行位置；不能把测试入口公开后当作源区域接线 |
| `required.rs` / `required/prepared.rs` | 完整根 Expr 的准备、冻结声明目录、单次输入及代码缓存所有者；Type 与普通值共用 VM 返回及 scope 恢复，不分配临时推断输出 | 在完整根形成前启动源区域；Type 产物回到原参数位置与消费者；世界可共享，活动前端区域、等待者及本次值不得冻结到代码缓存 |
| `function_bodies.rs::check_source_function` / VM `prepare_invocation` | 普通收集与 required 依赖共用主体检查和发布规则，当前仍只交付完整 Expr 的 Function | 分开声明接口、稳定源码模板、活动主体任务和可执行产物；保持原 activation 等待代码，不提前执行定义或发布假主体 |

先使用已有 HIR 身份、telescope、声明输入计划和 VM activation；不建立第二份
可修改的值表或以 AST 地址为键的全局等待表。具体所有权和响应提交沿
[状态协议](elaboration-state-and-publication.md)，表达式检查与执行依赖的区别沿
[调用期域边界](values-staging-and-publication.md#invocation-才能取得的域必须同时接通检查与执行)。

旧 `type_computations::Session` 已移入共享测试支持：它的唯一输出身份曾被用于
required Type 旁路，`627ab804` 后不再有生产调用者。保留其协议测试不等于选择它
作为新源码调度器；不能先重构这套孤立 API，再以其测试通过声称实际接线推进。

下一批不继续把“新增一个 owner／请求变体”单独当成恢复主干的交付。按
[源 activation 与片段交接](elaboration-state-and-publication.md#源检查与执行的交接目标)
完成三个共同依赖的改变：普通检查规则返回保留 continuation 的待决任务；原 VM
activation 在同一存储中执行已获准片段并交回控制结果；callable 输入域及签名消费
实际 Type 响应。完整根仍走已有快路径。不能为每个片段合成一个新函数／新 invocation，
也不能将活动 HIR 句柄塞进稳定 NIR 或代码缓存来获得一个看似可运行的入口。
特别地，callee 签名待定时的 producer 归调用者源区域执行；目标 callable 尚未通过
准入，不能先进入它的 activation 来求参数域。required 根由本次必需计算持有源区域，
函数内区域归原调用持有，不为片段另造函数。接口／模板、调用准备、activation 和
可执行主体的完成条件分别见上述状态协议；这一所有权顺序是接线约束，不是已有 API。

完整包 phase 与 safe 检查也是这次纵向迁移的一部分，具体生产消费者见
[凭证迁移边界](elaboration-state-and-publication.md#完整包检查凭证也是迁移边界)。
保留完整根快路径，不以部分声明目录伪造整包检查凭证；片段只有在原上下文下完成
相应检查才可交付同一 activation。builder 的 DefinitionHostRequest 不承载源码
主体请求，测试用 Type 队列也不承担源执行所有权。

实施分解见[普通检查 continuation](elaboration-state-and-publication.md#所有权与规则推进)
及其后的词法、绑定和控制交接协议。同步检查入口改为驱动同一规则的适配器，不能
另留一套只支持少数输入形状的检查器。Progress 已允许依赖项先完成检查并保留源码
schedule，但仍缺恢复任务；帧导出已经从特殊输入完成逻辑迁到显式绑定交接，仍缺
源作用域与执行许可。块尾值规则已由调用方闭包迁入 Progress，类型化的受检尾值
共用块绑定收尾；普通块现由显式帧栈交接语句／尾值结果及词法退出，嵌套普通块不再
递归保留整个分派栈。普通参数域／默认引用的依赖准备也已改为显式任务，保留默认值
后继并区分真实环与预算耗尽；块目标已按预算取得区域私有副本，并在相邻帧间共享。
普通与 contextual if 已与块共用帧栈，保留受检条件、then 结果、else 目标及分支
事实退出；普通入口复用原交付适配并保留 inline／callee／effect 状态。其余源码
子检查和 required 求值仍同步；活动词法状态、完整根准备和源 activation 仍须共同迁移。
部分结构输入的 if 现也进入条件帧，保留探测位置、numeric defaults 次序与当前
Attempt。子检查失败先退出内部词法帧，再由最近的 Attempt 消费失败并恢复；只有
缺少上下文才继续搜索。最终检查仍保留原来的部分输入合并与错误结果交付规则。
普通 when 的条件和主体已与 contextual when 共用同一帧，保留完整目标、有序
inline 选择、callee/effect 交付及正负前提退出；原独立同步分支循环已删除。部分
when／match 的候选域探测现也由所属帧持有 Search 与 Attempt，条件、主体请求
进入同一 Region；恢复时先退出内部词法／前提，再消费探测事务。普通调用的部分
错误结果域也已接入该检查入口，避免 error-only 分支过早固定成功域。初始化信息
观察中已知分支、匹配 arm 和块尾部改为迭代跟随，避免检查帧已退出后又在完整值构造器中堆积宿主栈；
保留静态观察预算和未知边界。普通与 contextual match 的 selector、主体后继、
已检查分支和词法／类型事实退出现也由同一帧持有；普通同步循环已退役，inline
选择与绑定物化、成功 payload 恢复和独立清理来源保留。模式、guard 与 inline
选择内部仍复用同步服务，不声称这些子工作已有外部等待后继。
普通比较与 and/or 现在由同一帧栈交接左右操作数，保留上下文搜索与检查事务；
逻辑帧另外持有 bool 短路前提与退出责任，BitVec 仍双侧求值。算术帧也已接入同一
Region，保留原生／trait RHS 试探及共同 peer 搜索，原同步探测与 peer 适配器退役。
一元 `-`／`not` 的协议探测、目标域重试与完成后继也已迁入共同帧栈，复用已检查
操作数并保持负数字面量形成规则；原同步一元检查流程退役。
它消除了这些已迁移规则持有子检查 Rust 栈的需要，但尚不支持
暂停并接收外部 Type 响应，两个待定域正例仍未解除 ignore。
帧栈及子检查返回值已迁入 `block_regions::driver::region::Region`，由同一同步入口
逐步 poll/accept；Waiting 和完成状态拒绝重复交付，失败仍沿原帧退出与 Probe 恢复。
这拆开了活动规则和驱动循环，尚未提供跨任务的 Checker 所有权或外部响应权限。
下一步须把这些活动检查与同一次 invocation 的前缀、存储和控制出口接起来，不能
把内部探测回退当作 VM activation 恢复。
能编译或导出一个 fragment 本身不证明原控制出口和一次性 producer 已被保留。
主体尚未形成时的发布责任见
[声明接口与可执行主体](elaboration-state-and-publication.md#声明接口与主体发布目标)。
这需要同时迁移函数收集与 VM 主体取得，不能只在完整 NIR 上增加一个宿主请求。

这一纵向交付必须同时做到：

1. 待定域、已观察的身份、已经执行的值分别存储并可 dump；未知不能伪装成 Any、
   Type 或正常的具体实例。显示采用现有 S-expression 规则。
2. 所需 Type 真实产生后，只恢复尚未完成的检查；保留原源码位置、输入域、词法绑定、
   phase 及预算。过期或外世界响应拒绝，取消不发布半成品。
3. 已执行前缀、可变存储和控制出口归原 invocation 所有；恢复不重放副作用。较早
   输入的组装可待域完成，不代表可以越过一个尚未获准执行的操作运行后面的生产者。
4. callable 签名、形成条件和输入检查全部完成后才交付实例。仍不支持的活动节点
   明确报缺口；只有实际生产者/执行边形成闭环时才能报循环。
5. 普通调用、NF／名义输入和真实 sysroot 的对应接口均消费该接线；覆盖域错误、
   默认/具名/展开顺序、分支/提前退出、失败重试、生产者次数及作用域逃逸。

两个 ignore 只在正确正例与上述边界实际通过后解除。不能通过在签名查询时偷偷执行
Byte、统一提前执行所有 Type 生产者，或给 List/Tuple 增加语法特判来交付。这一步也
不自动解除 P3／值引用证据的另两项 ignore，不代替运行时 EnvironmentPlan 或资源检查。

### 设计决定与后续实施细节分开

| 已确定、实现必须遵守 | 可在对应迁移阶段细化 |
| --- | --- |
| 声明/存储/执行身份分开，稳定 owner 与局部事务分开 | 私有 Rust 类型拆分、索引扇出、分块尺寸 |
| 普通参数和值、NF 不展开、无 Tuple/List 语义特权 | 具体值编码及按工作量选择的快路径 |
| 检查依赖与执行次数分开，暂停不重放 | instruction 布局、队列实现及局部调度优化 |
| TypeRef、环境取得、资源责任和 proof 分别交付 | 各节点字段、证据压缩和局部重放优化 |
| 布局/ABI/产物契约不改变类型及控制语义 | 各平台 ABI 分类、动态布局编码与持久 schema |

后列不是免责项：开始依赖某项能力的迁移时，必须先补齐该项具体设计并验证。
但它们也不是所有前端工作共同等待的全局门禁。默认 allocator 的不可恢复失败策略
及高级 verification 库不阻止上述声明输入迁移；涉及它们的库交付仍须解决真实语义
问题，不能临时替作者选择。格式化签名已由作者明确为
`fn println(comptime format: str, ...varargs: defer std.fmt.Parse(format))`。带 Display 的字段、转义与换行已经通过真实 sysroot 的宿主演示，
对应的类型数据与准入边界见[描述计算与参数准入](type-descriptions-and-admission.md)。

## 5. 测试如何支持架构，而不是牵着语义走

| 层 | 验证目标 | 反证需求 |
| --- | --- | --- |
| 语义内核 | identity、substitution、域、冻结 | 非单射 NF、capture、世代错误、错误 world |
| 事务/调度 | 提交、回退、唤醒、取消 | 失败后合法重试、旧 ticket、先完成宽候选 |
| solver | 传播理由、模型解释、投影 | 独立重放、有限域、trait world、Unknown 不转 false |
| VM | 指令与语义服务接线 | 双载体相同语义、不可重放副作用、外部请求恢复 |
| 源码/库 | 完整生产者到消费者 | 真实 prelude、不同包/target、错误域及资源 |
| native | 布局、ABI、控制与清理 | O0/O3 轨迹一致、错误与效应分流、环境有效期 |
| query/artifact | 正确复用和失效 | 新增 impl、失败修复、取消、产物删除、配置变化 |

现有测试保留为输入资产，按作者契约核对预期。未实现正例明确标记缺口，不靠改变期望变绿。
日常只运行受影响组，里程碑再扩大；这一设计轮不启动 Cargo 全量测试。

## 6. 性能判据

首先记录当前相同 binary/profile 的 cold/warm 行为，再比较新实现；结果缓存独立开关。
评估项包括：真实 sysroot 最小编译、不同包大小的域查询、深依赖/默认值、批量定义发布、
大列表组装、正则代码生成、PTC 分支/回跳、运行时来源及 cleanup 摘要、native 编译。

复杂度必须写清自变量。一次字段查询不应扫描所有无关定义；一次小批发布不应无条件
复制整个世界；持久列表复制不应总按展开载荷遍历。递归依赖闭包、模型搜索、必要检查
仍有真实成本，不承诺一切 O(1)。
分配与释放、hash、证据重放和结果接纳计入成本；不能把成本移出计时区后称算法更快。
超预算仍保持语义诚实，优化不能通过少验证来获取数字。

## 7. 源码清理的具体规则

迁移前列出旧入口的生产调用者、语义、错误和预算行为；新入口接管后检查这些调用者
确实改变，再删除旧入口。临时 adapter 只允许一个方向，不保留两套长期权威数据。
无调用引用不是充分删除依据：工具、序列化、测试 fixture 和公开 CLI 也要检查。

停用 compiler/typeck、mir、mir_build、codegen 的仓库内消费者和资产已核对，11 个文件
已作为独立退役单元删除。middle 的旧类型侧表及包装也已作为另一单元删除，
活动 HIR 查询及其生命周期约束保留。这两项无需等待 M7 全部条件成熟，也不以删文件数替代主干迁移。
生产 LLVM 在 luna/host，不因目录叫 codegen 就迁入它，也不先升级 LLVM。
unsafe 清理以生命周期证明为单位，不能用新增 unchecked Send/Sync 换取编译通过。

现有活动依赖与逐项退役条件见[模块边界](module-boundaries-and-retirement.md)。
middle 的 HIR 查询必须保留；intrinsic 的旧 TyCtxt 参数已经随该退役单元删除，
不能再据清理前注释推断存在待迁移的类型注册行为。单包与多包入口均须纳入适配器迁移验收。

## 8. 提交与状态

设计文件保持现行状态，修订正文，不追加每轮日志。完成一个可验收的功能/迁移提交一次，
附上直接相关库与测试；提交格式和权限沿用 AGENTS。
任何阶段未闭合时，报告实际缺的生产者、消费者或证据，不能以局部测试数宣称整个主题完成。
M0 设计基线已交付，停用源码退役已完成；M1–M3 已有上述生产接线，完整阶段及
M4–M7 的交付验收仍未完成。

M0 现在已形成可划定实施边界的设计基线，入口的[完成范围](README.md#设计基线与实际完成范围)
按原任务区分设计、现状证据和未实施部分。后续文档修改应解决新发现的契约冲突、
实际接线缺口或实施所需的具体表示，不继续以重复审计与增加状态段落代替源码工作。
下一批依其真实输入生产者、接收域和恢复边界落实 Rust 接口与检查；后续平台细节
不构成共同门禁，也不把所有后续实现扩大成 M0 必须预先完成的设计任务。
