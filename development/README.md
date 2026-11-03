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
| [状态与发布协议](elaboration-state-and-publication.md) | 如何暂停、恢复、提交、取消，并保持可见性与 coherence？ |
| [依赖观察](dependency-observations.md) | 如何观察局部信息、精确捕获外层输入并维护查询依赖？ |
| [值、执行与发布](values-staging-and-publication.md) | 如何统一参数和值，交织 VM、模板和 builder？ |
| [求解、资源与降码](solvers-resources-and-lowering.md) | CDCL、程序规格和后端如何分工并交换证据？ |
| [Type 模型与理论证据](type-models-and-theory-evidence.md) | 模型怎样成为合法替换，跨 theory 的解释与理由如何验证？ |
| [六包与交付](libraries-and-delivery.md) | 基础架构怎样支撑真正可用的库？ |
| [迁移计划](refactoring-plan.md) | 按什么顺序替换，何时才允许删除旧路径？ |

这些文档维护当前结论，不再追加逐轮成功日志。语言权威仍为作者确认、正式
[docs](../docs/README.md)与已确认历史；调查报告及教程不替代语言设计。
新阶段设计不能把已有实现限制提升为语言规则。

## 当前边界

已完成首轮静态证据核对，并细化状态协议和依赖观察设计；列出的源码重构及跨层验收均未实施。
设计区分世界存储版本、线性可见范围和整体 coherence，明确 VM 动态请求身份与发布边界。
Type 投影、原义务复核及 theory 证据协议已展开；具体 Rust 接口、各 theory 的完整准入与
见证覆盖、世界存储布局及开放布局表示仍待进一步设计。
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
