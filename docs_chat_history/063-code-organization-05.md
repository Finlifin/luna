# 063 — code-organization — 05

- 日期：2026-09-27
- global_index：063
- theme：code-organization
- subindex：05
- 本轮目标：纠正函数签名中残留的 handles 段，同步 capability 的 trait 协议与 elaboration 模型。

## 作者输入

> 都说了capability系统由trait协议配合elaboration实现，函数签名里没有handles的部分了

## 评估与确认

这是作者对已确认设计的再次更正，不是新的待选方案。此前更新语法时仍沿用了四处错误的函数产生式，应直接修正。Capability 由 trait 协议描述处理能力，普通 implementation resolution 选择入口，elaboration 将入口与环境接入相应处理行为。错误／效应行、恢复与资源契约仍约束处理后的计算。

本轮承接已有文档编写授权与作者的明确纠正，没有需要再次拍板的问题，也没有新增待确认设计。

## 文档修改

从以下四处 function 产生式移除 `(handles(expr))?`：

- `docs/grammar/syntax`
- `docs/functions/index.md`
- `docs/functions/calls-and-returns.md`
- `docs/comptime/the-comptime-phases-of-traits.md`

在 `docs/traits/protocols.md` 的「处理能力」中直接说明 capability 系统由 trait 协议配合 elaboration 实现；补充现有 HandleError 示例对应的实现选择说明，以及处理分支接入的编译器引导。函数调用章节与 `docs/type/handlers.md` 增加指向该定义的说明与链接。

现有示例中 `let result = checked_count(12).use(handler)` 缺少语句分号，本轮一并补齐。已按 `docs/grammar/syntax`、`docs/grammar/index.md` 和作者确认的 handler 示例检查语法与风格。

## 纠正范围

函数签名的 handles 段，与 handler 声明模式中的 handles 用法是不同位置。作者给出的 `2026-9-18-handling-capability-by-trait-system.fl` 使用 handler 声明模式，并将处理操作展开为协议入口调用。因此本轮保留现有 handler 绑定示例和词法表中的 handles，不把本次纠正扩大成删除所有同名 token。

没有引入新的 capability trait 名称、函数修饰词或隐式参数规则；没有修改错误／效应结果类型。历史记录、旧版归档和设计草稿保留原文。本轮未修改编译器。

## 检查

- 搜索现行正式文档：函数签名中的 handles 段已清除；剩余两处分别是 handler 绑定示例和词法表。
- 本轮文件摘要对比确认只修改上述六份正式文档；此前已有编译器修改未被本轮触及。
- `git diff --check` 通过；另外检查了本轮修改的未跟踪 Markdown 文件，没有行尾空白。
- 新增交叉链接及目标「处理能力」标题存在。
- 本轮是文档和语法速查纠正，未运行编译器测试。
- prompt 保持为空；本轮作者直接在对话中作出更正，没有 prompt 答复需要归档。

## 对作者的完整回复

已清除 [syntax](/home/fin/luna/docs/grammar/syntax) 及相关章节中四处函数签名的 `handles` 段，并明确 capability 由 **trait 协议配合 elaboration** 实现。更正已归档。
