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
| [一致性边界](consistency-boundaries.md) | 哪些信息在变换中容易丢失，用什么反例检查？ |
| [全链路架构](architecture.md) | 谁拥有世界、推断、执行、类型环境和产物？ |
| [模块边界与退役](module-boundaries-and-retirement.md) | 实际 crate 依赖承载什么，哪些职责须保留，旧路径何时可删除？ |
| [世界存储与生命周期](world-storage-and-lifetimes.md) | 快照怎样共享节点、原子发布并释放旧数据，句柄如何保证所有者有效？ |
| [状态与发布协议](elaboration-state-and-publication.md) | 如何暂停、恢复、提交、取消，并保持可见性与 coherence？ |
| [依赖观察](dependency-observations.md) | 如何观察局部信息、精确捕获外层输入并维护查询依赖？ |
| [值、执行与发布](values-staging-and-publication.md) | 如何统一参数和值，交织 VM、模板和 builder？ |
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
| 一致性易崩点整理 | [C01–C34](consistency-boundaries.md)及重绑定、值域、环境、世界和执行次数协议 | 这些反例尚未全部成为生产回归；跨层证据迁移未接通 |
| 劣化代码清理 | [R01–R20](architecture-audit.md)与[退役审计](module-boundaries-and-retirement.md)定位现有路径；四个停用 crate 的 11 个文件已核对仓库内消费者 | 四个停用 crate、middle 旧类型侧表及包装已删除；通用机制替代后的专用分支仍待清理 |
| 新一代全链路重构设计 | [总架构](architecture.md)及专题明确数据所有者、接口、状态转换、查询/执行边界、理论证据、六包与产物交付 | 具体 Rust 接线、理论片段实现、平台 ABI、持久编码及规模验证按依赖阶段完成 |

本阶段取得的有限运行证据用于保留正确行为和定位失败，不作为新架构已完成的证明：

| 现状路径 | 已核对结果与位置 |
| --- | --- |
| 真实 sysroot 的反射/builder、默认局部 const | [运行基线](architecture-audit.md#6-可复现的现状基线)通过；driver 包含工作区原有资源分析改动，不是纯 HEAD 构建 |
| 局部 helper 子句默认 | [已按声明根/调用闭包选择输入](dependency-observations.md#54-声明根依赖选择与调用闭包已接通)，默认 helper 正例恢复；[staged 默认值已分离残余与冻结表示](dependency-observations.md#55-invocation-残余值与冻结值分离)，泛型默认值的 P3 证据仍有独立缺口 |
| HIR 查询保留能力 | [两个现有测试](module-boundaries-and-retirement.md#41-middle-的旧类型子系统须另作一个完整退役单元)覆盖输入失效与拥有结果的寿命；后续源码退役也已重跑这些检查及生命周期拒绝例 |
| 两阶段方法与文本资源 | [真实六包的来源选择及有限调用](libraries-and-delivery.md#111-真实六包的两阶段方法基线)已核对；[文本来源分析成功而资源执行失败](solvers-resources-and-lowering.md#61-已有来源分析不等于可执行资源证据)，交付尚未接通 |

后续有两个已界定的入口：已完成的停用源码/旧类型包装退役，以及主干的声明需求 →
依赖闭包 → 调用供应 → required 求值。它们的边界见[迁移计划](refactoring-plan.md)；
不能只改继承循环，也不必等所有平台 ABI 和持久格式细节结束才开始前端工作。
首批实施仍须补齐它实际依赖的 Rust 接口和失败现场证据，并保留当前有效机制。

当前核验：旧文档 59 份逐字节备份仍完整；development Markdown 的 282 个相对文件
链接可解析；从审计基线至 `9e86aa16` 的 75 个变更路径全部为 development Markdown。
这证明修改边界与资料完整性，不证明语言实现或架构迁移完成。完整证据见
`/tmp/luna-design-convergence-wl114wwt`；本机临时路径不是持久构建输入。

可执行脚本、examples、repros、bench 和 library-prototypes 保留；三个旧入口只作跳转。
保持单代理直接工作，不恢复 graph engineering 或 prompt 文档问答循环。

## 开发入口

[工具与证据说明](working-guide.md)记录构建、复现、验证和提交入口。
[包元数据传输协议](build-input-protocol.md)保留现行 v1 外部格式，供未来迁移核对。
日常按变更选择针对性检查；上述基线没有运行 Cargo 全量回归或新架构 native 验收。
