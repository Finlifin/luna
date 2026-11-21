# 新阶段：语义整理与全链路重构设计

本目录维护 Flurry 的实现设计。语言语义以作者确认、正式文档与已确认历史为依据。
[初始审计](architecture-audit.md)保留 11 月 3 日的源码基线；[当前复核](audit-reply.md)对应 11 月 18 日晚报及其后已提交的修正。

## 阅读顺序

| 文档 | 回答的问题 |
| --- | --- |
| [语言契约](language-contracts.md) | 哪些设计必须保持，哪些“未知”与阶段不能混淆？ |
| [自举编译器备忘](bootstrap-lessons.md) | 当前 comptime 与 elaboration 为何采用这些折衷？自举时用什么反例重新检验？ |
| [架构审计](architecture-audit.md) | 当前源码究竟在哪里偏离，哪些旧报告已经过时？ |
| [晚报回复](audit-reply.md) | 哪些生产路径已接通，待决参数域需要谁保存源码、值与控制流？ |
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

当前主线是**同一次调用中的源码检查与 comptime 执行**。实参的域可以依赖后面的 Type 生产者，实参的副作用仍按书写顺序发生。检查器可以先知道某个槽的类型依赖，VM 才知道原调用已经执行到哪里。[源码检查与原调用的交接](audit-reply.md#源码检查与原调用的交接)用同一段 Flurry 代码说明二者各持有什么、哪些步骤可以延后，以及遇到外层 return 时谁取消余下的工作。

| 主干 | 已有生产入口 | 下一处交接 |
| --- | --- | --- |
| 声明输入与函数值 | 普通 callable、NF、别名及名义类型共享输入规划；动态 NF 的位置实参使用同一准备器 | 将目标接口的公开槽名和默认提供者随函数值传送，支持具名动态调用。[接口设计](callable-input-interfaces.md) |
| 定义检查与原调用 | 检查区域保存子表达式的进度；VM 可以运行受检片段并转交控制；源码调用已有选择及定义的宿主请求 | 将待决源码、域观察和原调用的已执行前缀接在一起。完整受检主体仍是注册前提。[迁移顺序](refactoring-plan.md#下一条纵向交付待定域的普通调用) |
| 共享与发布 | 声明快照、理论和代码可以复用；接收世界负责 callable 准入 | 只发布稳定结果。活动绑定、存储授权、候选和待决调用继续由当前事务持有。[世界与生命周期](world-storage-and-lifetimes.md) |
| 类型与跨阶段值 | 闭合 TypeRef 有 IIR 消费者；comptime 调用结果可通过已选择的 Splice 交付 runtime 值 | 有洞类型需要带域的环境计划；普通参数的异构交付仍有缺口。[类型引用](type-references-and-environments.md) · [跨阶段值交付](runtime-value-delivery.md) |
| 六包与可执行产物 | Linux 上已有文件、字节 I/O、Layout、逐字段检查 Display 的异构变参 println；luna 提供 build、fmt 与 lsp 入口 | runtime 文本的内存来源、分配与更多 Display 实现仍待接通。[标准库演示](std-host-foundation.md) |

普通 comptime 函数的子句实参可以等待后序调用交付的 Type。列表之前的写入在原位执行，提前 return 会跳过余下实参和函数体。公开签名本身也依赖该 Type 时，源码检查器仍缺少恢复路径，见 `computed_type_wait_preserves_earlier_value_before_later_write`。[用例与复现命令](audit-reply.md#一次调用内的恢复)

## 开发入口

[工具与证据说明](working-guide.md)记录构建、复现、验证和提交入口。[包元数据传输协议](build-input-protocol.md)保留现行 v1 外部格式。[文档重置记录](documentation-reset.md)保留旧资料的迁移映射和备份位置。
