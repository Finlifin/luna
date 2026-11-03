# 开发工具与证据入口

这里保留现有工具的必要操作说明，不保留旧阶段逐轮成功日志。
本轮未修改这些工具。当前规范以 [AGENTS.md](../AGENTS.md) 为准。

## 构建与检查

在仓库根目录执行：

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference
python3 -B development/check.py --timeout 900 -- test -p ptc
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

Cargo 经 `development/cargo.py` 固定复用 `/tmp/luna-dev-002/target`，串行、locked、offline，
关闭 incremental 和 dev/test debuginfo。入口管理共享维护锁、预算空间及可重建缓存清理，
不能同时再启动直接 Cargo。编辑编译中的源码会使结果对应旧输入，不应当作当前验证。
开发通常用 dev/test；release 用于性能或确需优化的验收，同一阶段不要来回重建 profile。

## 单二进制 CLI

```sh
python3 -B development/check.py --timeout 900 -- run -p luna -- check app.fl --sysroot library
python3 -B development/check.py --timeout 900 -- run -p luna -- test app.fl --sysroot library
python3 -B development/check.py --timeout 900 -- run -p luna -- comptime app.fl --entry Schema
python3 -B development/check.py --timeout 900 -- run -p luna -- dump app.fl --emit iir --entry main
```

示例中的 app.fl/Schema 是占位输入。实际帮助用 `luna help <command>` 或
`luna <command> --help`。子命令包括 check/build/test/dump/comptime/interpret/analyze、
packages 和 serve packages-v1。check 不是完整程序验证；reference 执行也不是 LLVM 验收。
语义选择器使用点路径，文件路径仍用操作系统路径。

## 现有验收与测量工具

| 入口 | 用途 |
| --- | --- |
| `reproduce.py` | 对已有二进制作受控超时复现 |
| `report.py` | 读取本轮过程和结果证据 |
| `backtrace.py` | 可选回溯；不可用时不阻塞独立工作 |
| `library_check.py` | 分阶段真实库检查，支持选择阶段 |
| `build_host_driver.py` | 构建并保留本次私有 driver，复用受管缓存 |
| `run_host.py` | 构建并执行单个 host 示例 |
| `host_examples.py` | 按登记用例运行原生程序，可对照 O0/O3 |
| `comptime_bench.py` | invocation/容器/会话测量 |
| `builder_source_bench.py` | 源码 builder 发布成本测量 |

具体参数从对应脚本 `--help` 读取，不以旧文档里的用例总数作为当前事实。
examples/repros 是持续保留的输入；library-prototypes 仍是未完成历史接口草稿。

## 进程和日志

wrapper 关闭 stdin、保留退出码和 stdout/stderr，并在返回的目录保存 result.json。
检查证据同时看实际 command、cwd、状态、退出码和输出，不引用旧日志冒充本轮运行。
观察超时不代表进程结束，继续轮询同一 handle；不能看到旧 running 文件就另启动构建。

证据保存失败返回 125；若终端报告 `report_saved: false` 或 `summary_saved: false`，
磁盘旧报告不是本次终态。读取终端附带的 evidence，不根据存储失败自动重跑已执行程序。
流量排空、进程回收和产物失败规则以 process_tools 与各脚本当前实现为准。

## 提交

```sh
python3 -B development/commit.py --manifest /tmp/change.manifest --message-file /tmp/change.message --dry-run
python3 -B development/commit.py --manifest /tmp/change.manifest --message-file /tmp/change.message
git log -1 --format=%B
```

manifest 每行一个本次路径，包括删除项；message 使用 `type(scope): summary` 与 Changes
正文，有实质限制时加 Limitations，不写 Validation。commit 已获授权，push 需用户另行要求。
脚本不绕过沙箱权限，沿用已有批准前缀。先看 status/diff，保护不属于本次的修改。

历史整理工具 rebase.py/rebase_history.py 保留，但不属于日常提交的自动前置步骤，
不能擅自重写他人或远端历史。需要时先读帮助及实际计划，再按当前授权处理。
