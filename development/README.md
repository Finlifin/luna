# 开发入口

语言基础文档从 [docs/README.md](../docs/README.md)进入。当前开发规则是根 [AGENTS.md](../AGENTS.md)，并发协作细则见[任务图协议](graph-workflow.md)。`main` 用于集成产物，开发在 `dev`。

## 工程地图

以下是 2026-10-07 的本地核对，不是永久不变的架构；每项任务仍须读取当前 manifest 和相关文件。

| 范围 | 当前证据与使用方式 |
|---|---|
| `compiler/` | Rust workspace；包含前端、query、middle、interface、luna、comptime 与 IIR 等 crate；具体成员以根 Cargo.toml 为准 |
| 后端旧目录 | typeck、mir、mir_build、codegen 当前未列为启用成员；workspace 检查不覆盖它们，不由目录存在推定后端完整 |
| `library/` | 当前有 builtin 与 std 源码；文档定义的其他内置包不代表已经实现 |
| anar | 未在当前 checkout 或 /home/fin/anar 发现；第一次任务先定位真实仓库、协议、工具链与测试入口，再分配写入范围 |
| `compiler/query/tests/query_engine.rs` | 已有行为断言，覆盖缓存、环、panic 恢复及键隔离，可作为现成局部测试入口 |
| `test_project/`、根 `test.fl` | 示例与手工输入；未发现完整自动化语言测试运行器，不能把文件数量当成通过用例数 |

AST 的部分测试和 comptime/IIR 新 crate 的模板测试没有覆盖相应语言特性。未来应由测试角色围绕具体契约补充行为验收，不只保留工程脚手架测试。

现有 luna driver 固定读取工作目录的 test.fl，并写入 ast.lisp、hir.lisp。运行它会改变文件；若做演示验收，使用独立临时工作目录、准备指定输入并提供所需 sysroot，不在共享工作区覆盖作者样例。

库源码中的历史注释也可能落后于规范，例如 std prelude 的导入规则以[内置包文档](../docs/code-organization/builtin-packages.md)为准，不能由现存注释反推隐式导入行为。

## 可用检查入口

下列命令来自实际 manifest 与测试文件；列出命令不表示本次已执行或已通过。执行者需记录工具链、工作目录、退出码和输出，并按任务选择检查范围。

```sh
cargo metadata --no-deps --format-version 1 --locked --offline
cargo test -p query --test query_engine --locked --offline --target-dir /tmp/luna-query-check
cargo check -p luna --locked --offline --target-dir /tmp/luna-driver-check
cargo check --workspace --all-targets --locked --offline --target-dir /tmp/luna-workspace-check
cargo fmt --all -- --check
```

这些命令从仓库根执行。并行任务把示例 target 目录换成自己的唯一目录，避免锁竞争及产物混用；对比基线和新版本时保持构建条件一致。大型缓存需要复用时，由协调者明确管理，不能多个任务清理同一目录。

离线依赖不足、工具链不兼容和代码失败分别报告。需要下载依赖时按实际权限处理，不能把网络失败写成代码失败，也不能为了获得绿色结果擅自更新 lockfile。

library 与 anar 没有本轮已核实的独立测试命令。它们的首项开发工作应建立可复现入口或接入已有真实运行器，再承诺可执行覆盖。

## 首轮开发的拆分方法

先确认可复现基线、现有失败及保护文件，再选一条有限的端到端路径，例如从一个最小包到确定的 HIR 输出。不要把“实现整个类型系统”作为一个节点。

可以围绕同一份接口描述并行展开：

| 节点 | 有界产物 | 开始所需的真实依赖 |
|---|---|---|
| luna | 选定构造的解析／HIR 行为及诊断 | 语法、节点契约、基线 |
| anar | 已定位工程中选定输入描述或任务调度接口 | 包／构建契约、luna 接入描述 |
| library | 该路径需要的最小显式导出与契约 | 编译器支持清单、内置包规则 |
| 测试 | 正例、拒绝例、运行器入口与输出核对 | 语义契约和预期结果；设计不等待实现完成 |

共同完成后由协调者执行集成验收。此表是拆分方法，不表示这些节点已经启动或已有实现承诺。

## 文档交接

[BitVec 代数](../docs/program-synthesis/bitvec-algebra.md)完成基础章节收尾。原文档协作协议按字节备份到 [AGENTS.docs.md](../AGENTS.docs.md)；它不再要求开发回合写 history 或填 prompt。

文档仍是实现契约。开发中发现文字错误可以随获准任务修复；语义变更须标明依据与影响，不能通过改测试、文档或示例掩盖实现不符合既定行为。
