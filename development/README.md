# 新阶段：语义整理与全链路重构设计

2026-11-03 开始，审计基线 `70214cdd`，工作分支 dev。
先完成语言设计认知、代码劣化根因、跨层一致性和目标架构设计，再进入源码迁移。
设计与现状审计已完成首轮收敛，作者现已要求开始清理。源码按已核定边界分批退役，
正式 docs 继续保持不动；下列目标结构仍须区分设计与实际接线。

## 阅读顺序

| 文档 | 回答的问题 |
| --- | --- |
| [语言契约](language-contracts.md) | 哪些设计必须保持，哪些“未知”与阶段不能混淆？ |
| [架构审计](architecture-audit.md) | 当前源码究竟在哪里偏离，哪些旧报告已经过时？ |
| [晚报回复](audit-reply.md) | 11-11／11-12 报告哪些结论仍成立，11-15 复核确认了哪些实际断点？ |
| [一致性边界](consistency-boundaries.md) | 哪些信息在变换中容易丢失，用什么反例检查？ |
| [全链路架构](architecture.md) | 谁拥有世界、推断、执行、类型环境和产物？ |
| [模块边界与退役](module-boundaries-and-retirement.md) | 实际 crate 依赖承载什么，哪些职责须保留，旧路径何时可删除？ |
| [世界存储与生命周期](world-storage-and-lifetimes.md) | 快照怎样共享节点、原子发布并释放旧数据，句柄如何保证所有者有效？ |
| [状态与发布协议](elaboration-state-and-publication.md) | 如何暂停、恢复、提交、取消，并保持可见性与 coherence？ |
| [依赖观察](dependency-observations.md) | 如何观察局部信息、精确捕获外层输入并维护查询依赖？ |
| [值、执行与发布](values-staging-and-publication.md) | 如何统一参数和值，交织 VM、模板和 builder？ |
| [可计算类型描述](type-descriptions-and-admission.md) | Any 的完成数据、定义观察和等待怎样区分，Type 生产与参数准入怎样支撑 Parse(format)？ |
| [跨阶段 runtime 值交付](runtime-value-delivery.md) | 源输入、Splice 实例和运行时构造怎样分开，参数重建何时可以退役？ |
| [函数值的参数接口](callable-input-interfaces.md) | 公开入口、默认提供者与代码身份怎样共同传送，动态 NF 如何接入共享计划？ |
| [Comptime 值准入](comptime-value-admission.md) | 成员资格、值引用、快照、Any/代码见证与缓存资格如何分别检查？ |
| [求解、资源与降码](solvers-resources-and-lowering.md) | CDCL、程序规格和后端如何分工并交换证据？ |
| [Type 模型与理论证据](type-models-and-theory-evidence.md) | 模型怎样成为合法替换，跨 theory 的解释与理由如何验证？ |
| [类型引用与环境](type-references-and-environments.md) | 依赖身份怎样形成类型，并跨调用、捕获、清理和 IIR 交付？ |
| [布局与调用 ABI](layout-and-call-abi.md) | 固定/值相关布局怎样观察，薄指针、参数交付和控制出口怎样保持契约？ |
| [六包与交付](libraries-and-delivery.md) | 基础架构怎样支撑真正可用的库？ |
| [构建与产物交付](build-and-artifact-delivery.md) | 构建上下文、pipeline 任务与缓存如何交付完整代码和入口？ |
| [迁移计划](refactoring-plan.md) | 按什么顺序替换，何时才允许删除旧路径？ |

这些文档维护当前结论，不再追加逐轮成功日志。语言权威仍为作者确认、正式
[docs](../docs/README.md)与已确认历史；调查报告及教程不替代语言设计。
新阶段设计不能把已有实现限制提升为语言规则。

## 设计基线与实际完成范围

设计已收敛为一套可用于划定实施范围的基线；完整目标仍未完成。
以下按原任务核对，设计文本、现状测试和源码交付分别判断，不以文档篇幅代替完成度。

| 原任务 | 已有交付与依据 | 尚未完成 |
| --- | --- | --- |
| 旧开发文档清理 | 旧正文退役，作者补充与现行 wire 保留；[重置记录](documentation-reset.md)给出映射与备份 | 源码和工具资产没有随文档删除；它们按独立退役条件处理 |
| 语言设计认知整理 | [语言契约](language-contracts.md)按正式来源区分语言保证、实现策略和现状限制 | 实现中的语义偏差没有因整理文档而修复 |
| 一致性易崩点整理 | [C01–C38](consistency-boundaries.md)及重绑定、值域、环境、世界和执行次数协议 | 这些反例尚未全部成为生产回归；跨层证据迁移未接通 |
| 劣化代码清理 | [R01–R20](architecture-audit.md)与[退役审计](module-boundaries-and-retirement.md)定位现有路径；四个停用 crate、middle 旧类型侧表及包装已删除；共同输入、词法捕获和接收世界准入已有生产迁移 | 通用机制替代后的专用分支仍待清理，不能将局部接线计为完整新架构 |
| 新一代全链路重构设计 | [总架构](architecture.md)及专题明确数据所有者、接口、状态转换、查询/执行边界、理论证据、六包与产物交付 | 具体 Rust 接线、理论片段实现、平台 ABI、持久编码及规模验证按依赖阶段完成 |

## 当前生产接线与下一步

以下以 `4937d332` 的整体核对为基础，逐项纳入后续迁移。11-15 在 `3fd323ca`
上重新核对源码检查／执行链、声明视图与格式化入口；两个待定 Type 正例仍失败，
具体见[本轮复核](audit-reply.md#本轮复核与下一处交接)。这不是重新执行所有专题的
验收；旧证据保留原基线。验证保护工作区原有修改，不声称为纯 HEAD 或全语言验收。

| 主干 | 当前已接入的生产入口 | 尚未闭合 |
| --- | --- | --- |
| 声明输入与调用供应 | `lexical_inputs` 按声明根及调用闭包收集依赖；`declaration_inputs` 的同一计划服务普通 callable、NF、别名及名义类型；完整供应与局部提示共用入口游标，旧 type_input_slot 已删除 | typed 消费者仍要求立即取得可检查域；完整 ContextualExpr 尚未替换 NormalFormArgument |
| 函数值的公开入口 | 普通调用和子句已从 HIR 提取独立入口描述，分别保留预留、到达与源码位置 | 描述尚未跨 callable Type／值传送；动态 NF 具名供应仍失败；[接口设计](callable-input-interfaces.md)明确准入、提供者和编码的共同迁移边界 |
| 参数域与源码控制 | 声明／trait 输入在替换前识别 callee 依赖；块、if、when/match、比较与 and/or 保留检查帧；逻辑帧保留短路前提，控制状态和输入事实分别管理整体回滚及词法退出 | 整个 Checker 尚非可挂起区域；内部 Probe 回退不是源执行恢复，已知符号身份不代表 invocation 生产者已执行 |
| 词法参数与代码签名 | 普通表达式、contextual 签名和捕获共用可见声明输入；关联常量将主体与捕获一起按实例替换 | 这不能使尚未执行的 Type 生产者提前变成已完成输入 |
| 值传递与接收世界 | 通用准入保留域、phase 和身份；跨 VM callable 在接收世界完成方法选择及关联签名观察 | 活动值、权限及选择证据不随 CodeSession 共享；完整统一值传送协议仍有待迁移 |
| 跨阶段 runtime 交付 | 常量与 comptime 调用结果可选择普通 runtime Splice；实际构造次数已有 native 对照 | comptime 参数直接作为 runtime 结果仍按源域报错，残留读取还会重建为 runtime Let，quote 数据重建也会跳过转换；[交付设计](runtime-value-delivery.md)规定共同消费边界及退役条件，尚未实现 |
| 稳定世界与执行 | DeclarationSnapshot 保活声明与索引；source invocation 借用原包并隔离已选实例，保留 CodeSession；关联选择、方法选择及具体定义准备可交接宿主请求；Type 输出由单次发布许可保护 | 普通前端检查请求、通用 Region/Candidate 所有权及任意定义发布尚未全线接通；共享声明世界不等于共享活动 VM |
| 源码主体与调用入口 | 声明与已检查主体分开存储；源码命名实例只携带身份和声明实参，入口契约成功后才替换泛型主体并按普通输入重绑定 | 注册表仍要求完整受检 NIR 主体；源码模板独立发布、待决主体请求及原 activation 的 Type 响应尚未接通 |
| TypeRef 与 IIR | core/substitution 已有实现，闭合 callable/template 有消费者 | 普通函数仍以 NIR Function 为实例载体；runtime_bindings 不能替代带域的 EnvironmentPlan |
| 六包与资源 | 已有真实 sysroot 的 builder、列表、反射及两阶段方法；Linux 文件/字节 I/O、Layout、文本与单值输出已接通；Display/ByteWriter 位于 builtin；FormatPlan 已能扫描格式串 | `println(comptime format: str, ...varargs: defer std.fmt.Parse(format))` 尚未实现；runtime 文本来源、阶段输入重建、allocator、抽象 Copy 仍须贯通 |

具体接线及语义边界见[共同输入计划](values-staging-and-publication.md#已接通nf名义类型与透明别名共用输入规划)、
[父声明输入](values-staging-and-publication.md#已接通contextual-签名与普通值检查共用父声明输入)、
[接收世界的 callable 准入](comptime-value-admission.md#跨-vm-callable-输入的主动准入)、
[世界所有权](world-storage-and-lifetimes.md)和[库迁移](libraries-and-delivery.md)。
源码退役已经完成的部分见[模块边界](module-boundaries-and-retirement.md)，不再作为
“首批尚待开始”的任务反复安排。后续仍按[迁移计划](refactoring-plan.md)删除被通用机制替换的分支。

上述 callable 准入阶段运行覆盖前端与 VM 共 1,284 项，均通过；它只证明该阶段准入
改动及相关路径，不能证明 M1–M7 完成。证据及检查命令见
[callable 准入验证](comptime-value-admission.md#跨-vm-callable-输入的主动准入)。
此前 trait 域及阶段边界的针对性验证见[历史取证](audit-reply.md#4-本次证据与限制)，
不把不同源码基线的测试数量相加作为架构完成度。
`75c70197` 的[六项旧问题复查](comptime-value-admission.md#当前尚未完成的边界)
为 2 项通过、4 项失败，区分诊断断言差异与实际功能失败；这批没有复跑下列四项
ignored 用例，不以旧检查冒充当前执行结果。

此前重新运行四项语义缺口的 ignored 用例，四项均失败，未改其预期或 ignore。
下面保留该次报告的诊断；11-15 只重跑前两项，结果仍失败，最新位置与证据见
[本轮复核](audit-reply.md#本轮复核与下一处交接)，不把其余两项算作本轮执行：

| 未完成用例 | 该次失败边界 | 下一步责任 |
| --- | --- | --- |
| `ordinary_clause_collectors::pending_clause_domains_can_wait_for_a_type_producer` | `191..201: list literal requires an element type context` | 保留待定接收域与表达式检查进度，接收 Type 结果后继续检查 |
| `ordinary_callable_types::computed_type_inputs_can_complete_a_dependent_callable_signature` | `159..181: comptime callable signature still depends on an invocation-time clause input` | 参数供应完成后形成真实签名，再发布 callable |
| `function_template_local_definitions::generic_default_stability_needs_its_definition_proof_environment` | `25..44: default expression requires P3 stability/termination evidence` | 定义端的抽象 Copy、表示及清理证据进入 P3 |
| `predicate_calls::comptime_decisions_check_their_results_before_returning_types` | `120..141: assumes requires a supported effect-free program predicate` | phase 与值引用稳定性证明；不以 PTC 替代程序检查 |

工作目录 `/home/fin/luna`，执行
`python3 -B development/check.py --timeout 900 -- test -p luna --test ordinary_callable_types --test ordinary_clause_collectors --test function_template_local_definitions --test predicate_calls --no-fail-fast -- --ignored`，
`/tmp/luna-tools-ed1vdnf1/result.json` 退出 101。`nesting` 的 subprocess 专用 ignore
不属于这四项语义缺口，也不能据 ignored 名称推断整个前端只有四个缺口。

下一条主干是[调用期域与签名恢复](refactoring-plan.md#下一条纵向交付待定域的普通调用)。
保留前端区域、实际输入生产者和 VM 已执行前缀，既不伪造 Type／Any，也不重放
已完成副作用。P3／资源线仍独立保留，不能因为另一个机制有进展就将其标为完成。

初始资料清理的映射与备份说明见[文档重置记录](documentation-reset.md)；其中的文件数、
行数和链接核验属于当时基线，不是当前状态。可执行工具、examples、repros、bench
及 library-prototypes 按各自消费者保留，不随文档更新删除。
保持单代理直接工作，不恢复 graph engineering 或 prompt 文档问答循环。

## 开发入口

[工具与证据说明](working-guide.md)记录构建、复现、验证和提交入口。
[包元数据传输协议](build-input-protocol.md)保留现行 v1 外部格式，供未来迁移核对。
日常按变更选择针对性检查；上述基线没有运行 Cargo 全量回归或新架构 native 验收。
