# 新阶段：语义整理与全链路重构设计

2026-11-03 开始，审计基线 `70214cdd`，工作分支 dev。
先完成语言设计认知、代码劣化根因、跨层一致性和目标架构设计，再进入源码迁移。
作者已要求不要着急改代码：本轮仅整理 development 文档，未修改编译器、库或测试代码，
也不修改 docs。下面的目标结构不是已实现功能。

## 阅读顺序

| 文档 | 回答的问题 |
| --- | --- |
| [语言契约](language-contracts.md) | 哪些设计必须保持，哪些“未知”与阶段不能混淆？ |
| [架构审计](architecture-audit.md) | 当前源码究竟在哪里偏离，哪些旧报告已经过时？ |
| [一致性边界](consistency-boundaries.md) | 哪些信息在变换中容易丢失，用什么反例检查？ |
| [全链路架构](architecture.md) | 谁拥有世界、推断、执行、类型环境和产物？ |
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

## 当前边界

已完成首轮静态证据核对，并细化状态协议和依赖观察设计；列出的源码重构及跨层验收均未实施。
设计区分世界存储版本、线性可见范围和整体 coherence，明确 VM 动态请求身份与发布边界。
值准入设计区分实际域、传送、编码和缓存；修正普通赋值一律使值引用失效的表述，
保留同一存活槽重新定位与独立快照的区别。通用准入及恢复事务尚未迁入实现。
Type 投影、原义务复核及 theory 证据协议已展开。世界存储选定不可变分块、按路径复制索引
及单一发布根；具体 Rust 接口、各 theory 的完整准入与见证覆盖和持久 schema
仍待进一步设计。
类型引用与环境设计明确了 NIR safe 检查、IIR verified 检查及后续证据维护的阶段边界。
布局/调用设计区分计算进度、值相关大小和目标支持，选择薄指针配合显式元数据环境；
统一反射/布局观察与调用计划的责任。各平台 ABI 分类和动态布局具体编码尚未实现。
构建交付设计区分执行宿主、host target 与投影目标，细化任务执行身份、IIR 访问有效期
及不可变内容/manifest 发布；现有驻留 v1 和元数据 wire 保持各自边界，尚未迁移实现。
完整 goal 仍在进行，不能以本目录写完代替代码清理或架构迁移完成。
保持单代理直接工作，不恢复 graph engineering，也不恢复 prompt 文档问答循环。

旧阶段文档正文已清理，历史可从 Git 查询；原有未提交开发文档另有删除前备份。
清理范围、旧主题去向和保护检查见[文档重置记录](documentation-reset.md)。
可执行脚本、examples、repros、bench 和 library-prototypes 保留。
三个旧链接入口只作跳转，现行外部协议独立保留，不再承载历史日志。

## 开发入口

[工具与证据说明](working-guide.md)记录构建、复现、验证和提交入口。
[包元数据传输协议](build-input-protocol.md)保留现行 v1 外部格式，供未来迁移核对。
本轮不运行 Cargo 全量回归；文档验证不声称证明编译器行为。
