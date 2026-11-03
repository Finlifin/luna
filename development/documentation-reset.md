# 开发文档重置记录

## 范围与依据

作者要求新阶段文档继续放在 development、旧的删除、docs 不动；随后明确先不要改代码。
本次只处理旧开发文档正文：根目录 51 份 Markdown（含旧 README），以及 runs 下四份
过期任务报告/契约。旧阶段正文全部移除，不另建仓库内 archive。
新文档替代其现行职责，历史成功日志可从基线及其祖先读取。

`metadata-wire.md`、`build-context.md`、`source-tests.md` 重建为简短跳转，
仅维持 library/build/README.md 与 tests/language/README.md 的既有链接。
外部元数据 wire 协议表迁入 build-input-protocol.md，不因架构整理丢弃已发布格式。
这些跳转不保留旧日志或平行设计。工具、示例、复现、基准和 prototype 源码没有删除。
examples/README、library-prototypes/README 和两个 repro 说明保留在输入附近；
前两个 README 更新了指向已删除文档的链接及阶段说明。
旧 runs 中的脚本、fixture 和脚本读取的 JSON 输入保留，属于停用工具资产，
不作为本阶段任务图、检查门禁或当前进度依据。

## 未提交内容的保护

删除前对全部 59 份 development Markdown 保存了逐字节备份和 SHA-256 清单：

```text
/tmp/luna-architecture-baseline-0kqx0u10
```

该目录含原路径副本、documentation.json、删除前 development.patch 和 status.txt。
因此修改过的 README/frontend-checkpoint 以及两份未跟踪 builtin 草稿也可恢复，
没有以 Git 中的旧版本覆盖它们。库草稿中的作者补充已迁入新语言/库设计；候选接口不当成承诺。
此路径是本机安全备份，不是供其他开发者依赖的持久构建输入。

已提交历史可用 `git show 70214cdd:development/<旧文件>` 读取。
未经提交的全文不能从 Git 恢复，应使用上述备份；后续清理 /tmp 前先确认这些内容已不再需要。

## 旧主题去向

表中的对应表示职责和经验提炼，不表示把旧日志全部复制到新文件。

| 原文档 | 当前职责去向 |
| --- | --- |
| README.md | 本目录新入口，历史日志全部移除 |
| build-context.md | [libraries-and-delivery.md](libraries-and-delivery.md) |
| builder-publication-performance.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| builtin-implementation-admission.md | [libraries-and-delivery.md](libraries-and-delivery.md) |
| builtin-packages-domains.md | [libraries-and-delivery.md](libraries-and-delivery.md) |
| builtin-packages-proposal.md | [libraries-and-delivery.md](libraries-and-delivery.md) |
| cli.md | [working-guide.md](working-guide.md) |
| closure-environment-parameters.md | [architecture.md](architecture.md) |
| cmtt-phase-checks.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| coherence-budget.md | [architecture-audit.md](architecture-audit.md) |
| comptime-code-sessions.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| comptime-lists.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| comptime-object-queries.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| comptime-source-session-performance.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| comptime-type-realignment.md | [architecture-audit.md](architecture-audit.md) |
| comptime-vm.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| default-strategies.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| definition-builders.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| derivation.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| elaboration-split.md | [architecture.md](architecture.md) |
| fallible-iteration.md | [solvers-resources-and-lowering.md](solvers-resources-and-lowering.md) |
| frontend-checkpoint.md | [architecture-audit.md](architecture-audit.md) |
| function-template-inputs.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| git-permissions.md | [working-guide.md](working-guide.md) |
| graph-workflow.md | [refactoring-plan.md](refactoring-plan.md) |
| handler-return-provenance.md | [solvers-resources-and-lowering.md](solvers-resources-and-lowering.md) |
| host-pipeline.md | [working-guide.md](working-guide.md) |
| implementation-plan.md | [refactoring-plan.md](refactoring-plan.md) |
| iterator-protocol.md | [libraries-and-delivery.md](libraries-and-delivery.md) |
| library-layout-validation.md | [libraries-and-delivery.md](libraries-and-delivery.md) |
| memory-binder-cache.md | [solvers-resources-and-lowering.md](solvers-resources-and-lowering.md) |
| metadata-wire.md | [build-input-protocol.md](build-input-protocol.md) |
| named-object-lookup.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| native-values.md | [libraries-and-delivery.md](libraries-and-delivery.md) |
| native-wrapper-resources.md | [solvers-resources-and-lowering.md](solvers-resources-and-lowering.md) |
| normal-form-value-inputs.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| object-storage.md | [values-staging-and-publication.md](values-staging-and-publication.md) |
| package-cli.md | [working-guide.md](working-guide.md) |
| predicate-calls.md | [solvers-resources-and-lowering.md](solvers-resources-and-lowering.md) |
| property-layout.md | [architecture-audit.md](architecture-audit.md) |
| ptc.md | [solvers-resources-and-lowering.md](solvers-resources-and-lowering.md) |
| query-safety.md | [architecture.md](architecture.md) |
| rebase.md | [working-guide.md](working-guide.md) |
| runtime-type-reference-integration.md | [architecture.md](architecture.md) |
| source-tests.md | [working-guide.md](working-guide.md) |
| sysroot-integration.md | [libraries-and-delivery.md](libraries-and-delivery.md) |
| text-queries.md | [solvers-resources-and-lowering.md](solvers-resources-and-lowering.md) |
| text-region-integration.md | [solvers-resources-and-lowering.md](solvers-resources-and-lowering.md) |
| text-view-provenance.md | [solvers-resources-and-lowering.md](solvers-resources-and-lowering.md) |
| trait-domain-substitution-performance.md | [architecture-audit.md](architecture-audit.md) |
| trait-phases.md | [libraries-and-delivery.md](libraries-and-delivery.md) |
| runs/001-frontend-entry/contract.md | [refactoring-plan.md](refactoring-plan.md) |
| runs/001-frontend-entry/report.md | [refactoring-plan.md](refactoring-plan.md) |
| runs/002-literal-foundations/contract.md | [refactoring-plan.md](refactoring-plan.md) |
| runs/002-literal-foundations/report.md | [refactoring-plan.md](refactoring-plan.md) |

## 验证边界

本轮检查新文档本地链接、旧文件处理清单和 protected.json 中 docs/compiler/library
逐文件 SHA-256：1,457 个受保护文件均与开始时相同，59 份备份的 hash 均匹配。
development Markdown 的本地文件链接检查通过；本次 manifest 只包含 development Markdown，
工具和非 Markdown 输入未纳入修改；`git diff --check` 通过。
检查工作目录为 `/home/fin/luna`，结构化结果保存在上述备份目录的 verification.json。
这些是本轮文档检查，没有运行新架构或源码回归测试。
旧 /tmp 测试结果仅作为历史材料，没有作为本轮成功证据。

完整代码清理和架构迁移尚未开始，本阶段文档不宣称其已完成。
