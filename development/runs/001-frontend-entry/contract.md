# 第一轮前端接口

本轮交付一个独立源码文件从 lex、parse、resolve 到 HIR 的可重复入口，并补全当前 HIR 的展示。这里定义开发工具接口，不扩大已实现的语言能力。

## 命令行

```text
luna <input.fl> [--emit hir|ast] [--output <path>] [--sysroot <library-path>]
luna --help
```

- 必须指定一个输入，默认输出 HIR；`--emit ast` 在解析成功后输出 AST。
- 默认将 dump 写到 stdout；`--output` 将 dump 写到指定路径，stdout 不再混入进度消息。
- `--help` 成功退出；缺失或重复参数、未知选项和非法 emit 值属于用法错误，退出码为 2。
- 文件 I/O、词法、语法、名称解析和 lowering 错误退出码为 1。诊断写到 stderr；发现前一阶段错误后不执行后续阶段，不输出成功 dump。
- 只在所选阶段成功后打开输出文件。拒绝输出到输入本身，包括能够确认的路径别名；失败时保持已有输出文件。底层写入失败必须报告，不能 panic。
- 不隐式读写工作目录下的 `test.fl`、`ast.lisp`、`hir.lisp`。输入文件名可以包含空白和 Unicode。
- `--sysroot` 是已有显式配置的命令行入口；不存在或非法显式目录应报错。未提供时沿用当前自动发现，不将缺少可选 sysroot 当作空文件无法解析的理由。
- 不增加 CLI 依赖或修改根 manifest/lockfile；必要的小型参数解析在 luna 内完成。

## HIR 展示

- `hir::Package` 实现标准 `Display`；现有 `dump_to_lisp()` 保留并委托同一实现。
- 输出一个完整 `(hir-package ...)` S-expression。当前 ItemKind、ExprKind、PatternKind 的每个分支均有明确节点标签，完整保留载荷、签名、参数、类型表达式、子节点及函数体。
- owner/body 引用用包含 owner 与 local 分量的结构化 ID；owner 与 body 表分开输出，避免循环展开。表迭代顺序稳定，不能直接依赖 hash map 遍历顺序。
- 字符串、符号与字符正确引用／转义；符号可统一用字符串承载。保留必要的节点身份；没有实现载荷的现存 TODO 变体输出自身标签。
- 格式允许编辑器按 Lisp 括号结构缩进，不使用 Rust Debug 结构或 `...` 替代实际载荷。此格式暂不承诺可反序列化为 HIR。
- 不改 HIR 类型定义或 lowering 语义；遇到基线缺陷，记录可复现证据交集成负责人。

## 所有权和执行

| 节点 | 唯一可写范围 | 本地检查 |
|---|---|---|
| hir | 隔离副本的 `compiler/hir/src/lib.rs`、新增 `display.rs`、`compiler/hir/tests/` | `cargo test -p hir --locked --offline`，必要时局部 check |
| driver | 隔离副本的 `compiler/luna/src/`、`compiler/diagnostic/src/lib.rs` | `cargo check -p luna --locked --offline`，临时目录中手工调用 |
| tests | 隔离副本的 `compiler/luna/tests/` | 新增独立 CLI 集成测试；基线失败是预期，集成后运行 |
| root | 任务记录、开发入口；移除 `ast_lowering/src/item.rs` 的无条件 stdout 调试输出，将 `parse/src/parser.rs` 的警告改到 stderr；按 patch 接收上述范围 | 集成测试、差异检查、保护基线检查 |
| grader | 只读最终工作区，测试缓存仅写 `/tmp` | 独立重跑及 C1–C14 缺陷列表 |

三个写者的副本来自相同 dirty baseline：`/tmp/luna-dev-001/baseline`。每个写者使用独立 target 目录，不执行提交、不创建下级代理。集成负责人逐文件验证基线后接收改动。

第一轮独立验收发现基线 lowering 对未支持的 take view 和顶层 test block 会 panic。定向修复由 root 负责 `ast_lowering/src/expr.rs`、`item.rs` 的分派诊断，以及已识别 take view 的 AST 子节点读取；测试作者在原隔离副本中补充拒绝与结构回归。此修复兑现原有阶段失败契约，不修改语言定义或验收标准。

## 验收与限制

固定结果标准见 `graph.json` 的 C11–C14。测试至少覆盖成功输入、默认/AST/文件输出、参数错误、文件错误、词法/语法/解析拒绝、原文件与原 dump 保护，以及 HIR 嵌套结构、转义、body 顺序稳定性。测试预期来自此接口和现行 grammar，不能为了通过把真实失败改成成功。

独立审阅与定向修复循环最多两轮。最多 5 个参与者、并发 4、4 波、单节点重试 1、深度 1、12 个委派工作回合、90 分钟。此次不安排提交、推送或合入 main；开发产物留在 dev 供后续集成。
