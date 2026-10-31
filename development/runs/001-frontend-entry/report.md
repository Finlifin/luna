# 第一轮前端开发交付

本轮建立显式单文件前端入口，并补全当前 HIR 的可读展示。固定验收条件在 [contract.md](contract.md)与 [graph.json](graph.json)中。

## 已实现

- `luna <input.fl> [--emit hir|ast] [--output <path>] [--sysroot <path>]`，以及 `--help` 和输入选项分隔符 `--`。
- 默认 stdout 只包含 HIR；诊断写到 stderr。用法错误退出 2，阶段或 I/O 错误退出 1，成功退出 0。AST 模式在解析后结束。
- 阶段失败不发布 dump。文件输出先完成临时文件写入，再发布；保护原输入、旧输出及工作目录中的历史 dump。Unix 下检查硬链接别名。
- 当前 HIR 的 Package、item、表达式、pattern、签名、函数体及相关载荷都有结构化 S-expression 展示。body 排序稳定，文本转义；原 `dump_to_lisp()` 委托 `Display`。
- 清理 lowering 的无条件 stdout 调试输出，将 parser 警告转到 stderr。
- 修复已有 take view 的 AST 子节点读取错误；未支持的 view、顶层构造和 builtin 操作数通过已有诊断路径拒绝，避免已复现的 panic 与递归栈溢出。

## 修改范围

实现位于 `compiler/luna/src/`、`compiler/hir/src/display.rs` 与 HIR 导出接线。`compiler/diagnostic/src/lib.rs` 调整诊断输出；`compiler/ast_lowering/src/item.rs`、`expr.rs` 修复分派诊断和 take view 子节点读取；`compiler/parse/src/parser.rs` 仅调整警告输出。测试位于 `compiler/hir/tests/display.rs` 和 `compiler/luna/tests/frontend_cli.rs`。开发入口已同步用法与局部测试命令。

原有源码修改作为共同基线保留；此次补丁相对该基线保存在 `/tmp/luna-dev-001/final.patch`。工作区保护检查见 [workspace-check.json](workspace-check.json)，可重跑 [verify-workspace.py](verify-workspace.py)。分支为 dev，未提交或推送。

## 实际验证

| 检查 | 结果 | 输出锚点 |
|---|---|---|
| 开工基线 `cargo check -p luna --locked --offline` | 通过，原有 warnings 保留 | [baseline-check.log](baseline-check.log) |
| 独立 CLI 测试在旧 driver 上执行 | 0/26 通过，预期失败，确认旧入口没有兑现接口 | `/tmp/luna-dev-001/tests-baseline.log` |
| 集成 `cargo test -p hir -p luna --locked --offline --target-dir /tmp/luna-dev-001/integration-target` | 41 passed，0 failed；3 个原有 doctest ignored | `/tmp/luna-dev-001/integration-tests.log` |
| 独立审阅新增的 6 项 lowering 回归，在修复前执行 | 1 passed，5 failed；实际捕获 panic 与栈溢出 | `/tmp/luna-dev-001/tests-repair-red.log` |
| 定向修复后重跑相同 HIR/luna 测试命令 | 47 passed，0 failed；3 个原有 doctest ignored | `/tmp/luna-dev-001/repair-tests.log` |
| HIR、driver、测试及 diagnostic 改动文件的 rustfmt 检查 | 通过；未格式化全库 | `/tmp/luna-dev-001/format-check.log` |
| 实际 HIR 交给 Emacs reader 与 formatter | 通过，格式化前后结构相等 | `/tmp/luna-dev-001/editor-check.json` |
| 工作区范围核对 | 1766 个不属本轮范围的既有路径保持原状 | [workspace-check.json](workspace-check.json) |

最终 47 项由 8 个 HIR 测试、7 个 driver 单元测试、32 个 CLI 集成测试组成；HIR 表格用例覆盖当前各类变体。CLI 子进程设有 15 秒超时，stdout/stderr 同时读取并限制保留量，以免错误恢复阻塞测试。take view 正例只验 AST 到 HIR 的结构，不宣称通过类型检查。

driver 工作者另运行了输出部分写入失败、stderr/stdout 写入失败与文件发布检查，原始证据在 `/tmp/luna-dev-001/luna-driver-write-failure-dlsfu185` 和 `/tmp/luna-dev-001/luna-driver-manual-bnsszwx4`。集成验收以本表的实际命令为准。

## 协作和验收

使用 graph-engineering-workflow。根代理先调查代码，再按共同契约并行分派 HIR、driver 和测试；三个写者使用包含既有未提交改动的独立源码副本。根代理按文件所有权和基线摘要接收改动，执行集成检查。未使用 Graphify：没有现成索引，限定范围的源码读取已能确定依赖。

独立测试设计不依赖 driver 实现，HIR 与 driver 也只通过既有导出接口接线；集成依赖三者实际产物。初次新建测试代理遇到宿主线程上限后，复用一个已完成旧任务的代理担任测试作者，其后不担任本轮验收者。

两处输出流修复由根代理负责；临时输出成功发布后的清理问题交 driver 作者处理，并有单元测试。没有重启无关任务。

第一轮独立验收在额外输入中发现两处基线 lowering panic，C10、C11 未通过，得分 11/13。根代理使用已有诊断与 invalid 节点路径修复分派，同时修复相邻已识别 take view 的错误子节点读取；测试作者独立补充 6 项回归，实际记录修复前的失败。此后才重跑受影响测试并重新验收全部固定标准；没有降低验收条件或用捕获任意 panic 的办法掩盖缺陷。

独立审阅与定向修复循环已完成：第二轮 **13/13** 适用标准通过，`gate: passed`，C4 因未进行外部研究而不适用。完整逐项结果见 [acceptance.json](acceptance.json)，过程及计数见 [graph.json](graph.json)。C1、C2、C7、C8 是运行记录类证据；其余适用项依据产物或独立重跑。没有剩余验收缺陷。

独立审阅者重跑了全部 47 项定向测试，并额外完成 35 项检查；先前 panic 输入现在产生正常诊断，AST 阶段仍能输出，源文件、旧输出与缺失输出的状态均符合契约。证据为 `/tmp/luna-dev-001/grader2-tests.log`、`grader2-extra.json`、`grader2-workspace.json` 与 `grader2-emacs.log`。

实际参与者 5/5、最高并发 4/4、波次 4/4、重试 0/1、深度 1/1、委派工作回合 6/12、验收轮 2/2；耗时在 90 分钟上限内，精确秒数记录于 graph.json。预算单位是可观测的委派工作回合。三个实施节点均交付有效产物；初次创建测试代理的宿主上限问题通过复用已完成任务的代理解决。没有提交、推送、发布、部署、外部发送或删除项目文件。

## 边界

本轮没有进行全 workspace 测试、LLVM codegen 或运行时程序执行。现有 AST visitor 仍是 AST 导出入口；当前 HIR 的历史字段和占位变体照实展示，不能从 dump 中出现某个节点推定对应语言特性已经实现。后续 elaboration、NIR/IIR、library 与 anar 接入须各自建立行为验收。

非 Unix 平台没有实现或测试硬链接身份检测；路径规范化仍用于检查可识别的别名。dump 是调试接口，本轮不承诺 HIR 的反序列化或跨版本格式稳定性。
