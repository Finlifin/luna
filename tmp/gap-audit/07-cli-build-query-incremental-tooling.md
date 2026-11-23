# Wave 2 深度差距审计：CLI、工程构建、查询引擎与增量工具链（`07-cli-build-query-incremental-tooling`）

## 1. 审计范围、检查基线与设计标尺

### 1.1 检查基线与静态规模

本次审计基于仓库 `/home/fin/luna` `HEAD` 提交 `b9a52238`，对 CLI 单二进制入口、工程/多包调度、查询引擎、虚拟文件系统、格式化、诊断渲染与 LSP 工具链所涉 **7 个核心 Crate** 执行源码级静态审计（不含任何 `cargo build` / `cargo test` 运行，不转引历史开发日志），并对 `#[cfg(test)]` 外部子模块与内联测试块进行静态剔除后完成非测试生产代码盘点：

| Crate | 非测源文件数 | 非测 LOC | 核心覆盖模块（实测） | 配套测试资产（`tests/`） |
|---|---:|---:|---|---|
| [compiler/luna](file:///home/fin/luna/compiler/luna/src/main.rs) | 64 | 41,481 | `main.rs`, `cli/`, `cli.rs`, `frontend.rs`, `packages.rs`, `package_cache.rs`, `package_cli.rs`, `package_stream.rs`, `compilation_cache.rs`, `build_context/`, `output.rs`, `testing.rs`, `protocols/`, `host/tools.rs`, `host/llvm/` | `frontend_cli.rs` (252), `builder_invocations.rs` (87), `packages.rs` (50), `build_context.rs` (35), `build_metadata.rs` (12), `subcommands.rs` (11), `package_cache.rs` (11), `compilation_cache.rs` (11), `package_incremental.rs` (9), `metadata_wire.rs` (9), `cli_fmt_options.rs` (7), `cli_lsp.rs` (3), `cli_fmt.rs` (2) |
| [compiler/anar](file:///home/fin/luna/compiler/anar/src/lib.rs) | 8 | 7,754 | `cli.rs`, `server.rs`, `analysis.rs`, `index.rs`, `json.rs`, `rpc.rs`, `position.rs` | `lsp_integration.rs` (4) |
| [compiler/query](file:///home/fin/luna/compiler/query/src/lib.rs) | 7 | 822 | `lib.rs`, `cache.rs`, `dependencies.rs`, `stack.rs`, `cycle.rs`, `dep.rs`, `howto.rs` | `query_engine.rs` (9), `invalidation.rs` (6) |
| [compiler/vfs](file:///home/fin/luna/compiler/vfs/src/lib.rs) | 1 | 234 | `lib.rs` (`FileId`, `AstNodeId`, `Vfs`) | — |
| [compiler/fmt](file:///home/fin/luna/compiler/fmt/src/lib.rs) | 13 | 12,633 | `cli.rs`, `config.rs`, `formatter.rs`, `items.rs`, `exprs.rs`, `patterns.rs`, `comments.rs`, `directives.rs`, `bridge.rs`, `verify.rs`, `diff.rs`, `doc.rs` | `stage1_foundation.rs` (8), `stage2_formatting.rs` (17) |
| [compiler/diagnostic](file:///home/fin/luna/compiler/diagnostic/src/lib.rs) | 1 | 444 | `lib.rs` (`DiagnosticBuilder`, `DiagnosticContext`, `emit_to_ariadne`, `failure_summary`) | 内联 `#[cfg(test)]` |
| [compiler/interface](file:///home/fin/luna/compiler/interface/src/lib.rs) | 2 | 237 | `lib.rs` (`CompilerInstance`), `session.rs` (`Session`) | — |
| **合计（7 Crate）** | **96** | **63,605** | 剔除 3 个外部测试模块：`host/method_contract_tests.rs`、`resources/tests.rs`、`host/llvm/handler_frame_tests.rs` | 538 个外部集成测试用例（含 15 个 `query`、25 个 `fmt`、4 个 `anar`、494 个 `luna` CLI/包/增量用例） |

> 辅助对照基线：[sysroot.rs](file:///home/fin/luna/compiler/intrinsic/src/sysroot.rs)、[library/language-items.conf](file:///home/fin/luna/library/language-items.conf) 与 [library/build/](file:///home/fin/luna/library/build/lib.fl)。

### 1.2 设计标尺与规范来源

- **Core Norm Target（核心规范目标）**：
  - [AGENTS.md](file:///home/fin/luna/AGENTS.md) §1（取消独立 `anar` 计划，统一于 `luna` 单二进制 CLI；六个内置包地位；当前后端沿用 LLVM 约定）与 §5（5 个默认 prelude 与去重/自屏蔽边界、路径点分隔、IR S-expression `Display` 规范）。
  - `docs/code-organization/`：[index.md](file:///home/fin/luna/docs/code-organization/index.md)、[packages-and-umbrella.md](file:///home/fin/luna/docs/code-organization/packages-and-umbrella.md)、[modules-and-sources.md](file:///home/fin/luna/docs/code-organization/modules-and-sources.md)、[builtin-packages.md](file:///home/fin/luna/docs/code-organization/builtin-packages.md)、[configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md)、[imports-and-reexports.md](file:///home/fin/luna/docs/code-organization/imports-and-reexports.md)、[scope-and-paths.md](file:///home/fin/luna/docs/code-organization/scope-and-paths.md)、[visibility.md](file:///home/fin/luna/docs/code-organization/visibility.md)。
  - `docs/pipeline-projection/`：[target-validation-and-assembly.md](file:///home/fin/luna/docs/pipeline-projection/target-validation-and-assembly.md)、[backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md)、[declarations-and-interfaces.md](file:///home/fin/luna/docs/pipeline-projection/declarations-and-interfaces.md)。
- **Wishlist Extension Target（扩展路线目标 `[Wishlist]`）**：
  - [06-attrs-build-pipeline-projection.md](file:///home/fin/luna/tmp/wishlist/06-attrs-build-pipeline-projection.md)（`WL-06-10..15`、`WL-06-21`）与 [00-index-and-roadmap.md](file:///home/fin/luna/tmp/wishlist/00-index-and-roadmap.md)（`OQ-36` 等）。注：磁盘实际仅存在 `06-attrs-build-pipeline-projection.md` 与 `00-index-and-roadmap.md`，不存在独立的 `06-build-targets-and-tooling.md`。

---

## 2. 成熟度总览与统计表

### 2.1 特性成熟度汇总（对应 §3 全量矩阵 24 项 `GAP-07-01`..`24`）

| 成熟度等级 | Core 核心项数 | Wishlist 扩展项数 | 合计项数 | 占比 | 核心代表项（摘要） |
|---|---:|---:|---:|---:|---|
| 🟢 Full（完备落地） | 3 | 0 | 3 | 12.5% | `luna` 子命令表面、版本化 `--packages-v1`/`--packages-stream-v1`/`FMD\x01`/`language-items.conf`、`fmt` CLI 模式与配置自动发现 |
| 🟡 Partial（部分实现） | 9 | 0 | 9 | 37.5% | `build.info` + SCC 分波并行、`luna test` + 原子输出、`QueryEngine` 进程内缓存、`.flpkg` 快照与 `CompilationCache`、`Vfs`、`fmt` 文档引擎、`diagnostic`、`anar` LSP 协议、`query` 错误处理 |
| 🟠 Frontend-Only（仅前端停滞） | 0 | 0 | 0 | 0.0% | — |
| 🔴 Missing（完全缺失） | 3 | 3 | 6 | 25.0% | `package.fl` 与 Umbrella、`build.{package,targets,tasks}` DAG + `scripts/` 调度、持久化查询指纹/Early-Cutoff；`[Wishlist]` `build.{generated,util}` 沙盒、`watch`/`doc`/`bench`/`init`/`clean`、`InvalidationDep` 字段级失效 |
| ⚠️ Divergent（规范偏离） | 6 | 0 | 6 | 25.0% | `anar` 独立身份/配置命名空间残留、`Sysroot` 两包割裂与单 prelude 路径、硬编码宿主 x86_64/Linux 代码生成与缺失外部管线交付、`fmt` 错设 `\|>` 优先级 80、`fmt` 校验漏掉宏/属性与有损非 UTF-8 路径、`anar` LSP 仅达 HIR 前段 |
| ⚪ Deferred（显式延后） | 0 | 0 | 0 | 0.0% | — |
| **合计** | **21** | **3** | **24** | **100.0%** | Core 占 87.5%，Wishlist 占 12.5%；Core `🟢 Full` 仅 3/21（14.3%） |

### 2.2 健全性 · 完备性 · 鲁棒性风险汇总（对应 §6 专项审计 19 项）

| 审计维度 | Critical | High | Medium | Low | 合计 | 编号范围 |
|---|---:|---:|---:|---:|---:|---|
| 健全性（Soundness） | 0 | 5 | 2 | 0 | 7 | `SND-07-01` .. `SND-07-07` |
| 完备性（Completeness） | 1 | 2 | 2 | 1 | 6 | `CMP-07-01` .. `CMP-07-06` |
| 鲁棒性（Robustness） | 0 | 3 | 2 | 1 | 6 | `ROB-07-01` .. `ROB-07-06` |
| **合计** | **1** | **10** | **6** | **2** | **19** | 全面覆盖 7 个 Crate 非测生产路径 |

---

## 3. 全量特性差距矩阵（`GAP-07-NN`）

| 编号 | 所属维度 | 特性 / 子系统 | 归属 | 设计标尺 | 当前实现现状（精确 `file:line` 与符号） | 成熟度 | 关键差距摘要 |
|---|---|---|---|---|---|---|---|
| `GAP-07-01` | CLI 与入口 | `luna` 单二进制子命令路由与契约选项 | Core | [packages-and-umbrella.md](file:///home/fin/luna/docs/code-organization/packages-and-umbrella.md):47, [AGENTS.md](file:///home/fin/luna/AGENTS.md) §1 | [main.rs](file:///home/fin/luna/compiler/luna/src/main.rs):18-35、[cli/subcommands.rs](file:///home/fin/luna/compiler/luna/src/cli/subcommands.rs):1-401、[cli/help.rs](file:///home/fin/luna/compiler/luna/src/cli/help.rs):1-157、[cli.rs](file:///home/fin/luna/compiler/luna/src/cli.rs):140-965 | 🟢 Full | 覆盖 `check`/`build`/`test`/`fmt`/`lsp`/`dump`/`comptime`/`interpret`/`analyze`/`packages`/`serve packages-v1`/`help`，支持 `--` 与原生 `OsString` 路径 |
| `GAP-07-02` | CLI 与单二进制 | 取消独立 `anar` 计划并收敛命名空间 | Core | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §1、[packages-and-umbrella.md](file:///home/fin/luna/docs/code-organization/packages-and-umbrella.md):47 | [Cargo.toml](file:///home/fin/luna/Cargo.toml):29、[anar/Cargo.toml](file:///home/fin/luna/compiler/anar/Cargo.toml):2-5、[luna/main.rs](file:///home/fin/luna/compiler/luna/src/main.rs):24、[anar/server.rs](file:///home/fin/luna/compiler/anar/src/server.rs):88,580、[anar/analysis.rs](file:///home/fin/luna/compiler/anar/src/analysis.rs):79、[anar/cli.rs](file:///home/fin/luna/compiler/anar/src/cli.rs):12 | ⚠️ Divergent | `luna lsp` 已代理至 `anar::cli::run`，但仍保留独立 `anar` workspace crate、`settings.flurry.anar.sysroot`、LSP server 名 `"anar"` 与诊断源 `"anar"` |
| `GAP-07-03` | 工程配置 | `package.fl` 清单、Umbrella 工作区与成员边界 | Core | [configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md):18,22-50、[packages-and-umbrella.md](file:///home/fin/luna/docs/code-organization/packages-and-umbrella.md):16-48,133-141 | 全量 `compiler/luna/src`、`compiler/anar/src`、`compiler/interface/src` 对 `package.fl` / Umbrella 匹配为 **0 处**；[anar/analysis.rs](file:///home/fin/luna/compiler/anar/src/analysis.rs):312-315 仅查找 `lib.fl`/`main.fl` | 🔴 Missing | 无 `package.fl` 解析/comptime 求值器、无依赖版本/别名表与 Umbrella 成员隔离，只能靠 CLI 显式 `--package` 注入或单目录发现 |
| `GAP-07-04` | 构建调度 | `build.{package,targets,tasks}` DAG 与 `scripts/` 宏前置执行 | Core | [configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md):28-44,155-184；扩展参见 [WL-06-11..13](file:///home/fin/luna/tmp/wishlist/06-attrs-build-pipeline-projection.md):57-59 | [library/build/lib.fl](file:///home/fin/luna/library/build/lib.fl):1-6 仅导出 `info.*`；[library/build/README.md](file:///home/fin/luna/library/build/README.md):39,96,123-125；`package/`、`targets/`、`tasks/` 目录仅为空占位 `README.md` | 🔴 Missing | 缺少 `schedule(selected_targets)`、`TaskSpec`/`BuildDag` 与 `scripts/` 先于消费者解析的编译执行管道（仅检测到 `scripts/` 时简单绕开缓存） |
| `GAP-07-05` | 构建沙盒 | `build.{generated,util}` 生成源码注册与受追纵构建辅助 | Wishlist | `[Wishlist: WL-06-14/15]` [06-attrs-build-pipeline-projection.md](file:///home/fin/luna/tmp/wishlist/06-attrs-build-pipeline-projection.md):60-61 | [library/build/generated/README.md](file:///home/fin/luna/library/build/generated/README.md)、[library/build/util/README.md](file:///home/fin/luna/library/build/util/README.md) 均为空占位文档 | 🔴 Missing | 未提供 `register_generated_source`/`read_tracked_file`/`tracked_env_var`/`run_tracked_tool` 及防 `../` 与包外写入沙盒校验 |
| `GAP-07-06` | CLI 工具链 | 标准工程辅助命令（`watch`/`doc`/`bench`/`init`/`new`/`clean`） | Wishlist | `[Wishlist: CLI/Tooling]` | [cli/subcommands.rs](file:///home/fin/luna/compiler/luna/src/cli/subcommands.rs):1-401 | 🔴 Missing | 尚无文件监听重编 `watch`、文档生成 `doc`、基准测试 `bench`、脚手架 `init`/`new` 与缓存清理 `clean` 子命令 |
| `GAP-07-07` | Sysroot 与 Prelude | 六内置包 Sysroot 清单与 5 个默认 Prelude 注入契约 | Core | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5、[builtin-packages.md](file:///home/fin/luna/docs/code-organization/builtin-packages.md) | [sysroot.rs](file:///home/fin/luna/compiler/intrinsic/src/sysroot.rs):179 仅枚举 `builtin`+`std`；[frontend.rs](file:///home/fin/luna/compiler/luna/src/frontend.rs):677-680,701-730 加载 6 包；[packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs):459,604 | ⚠️ Divergent | `Sysroot::packages()` 漏掉 `meta`/`build`/`attrs`/`verification`；`packages.rs` 多个内外部入口默认只注入 `&[builtin]` 单 prelude；`Emit::Hir` 跳过 `language-items.conf` 协议绑定 |
| `GAP-07-08` | 外部驱动接口 | `--packages-v1`、`--packages-stream-v1`、`BuildContext` 与 `FMD\x01` 协议 | Core | [configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md):24-26 | [package_cli.rs](file:///home/fin/luna/compiler/luna/src/package_cli.rs):10-96,535-780、[package_stream.rs](file:///home/fin/luna/compiler/luna/src/package_stream.rs):1-130、[build_context.rs](file:///home/fin/luna/compiler/luna/src/build_context.rs):295-520、[build_context/metadata/wire.rs](file:///home/fin/luna/compiler/luna/src/build_context/metadata/wire.rs):1-120、[type_constructors.rs](file:///home/fin/luna/compiler/luna/src/protocols/type_constructors.rs):515-620、[language-items.conf](file:///home/fin/luna/library/language-items.conf):1-136 | 🟢 Full | 具备严格 hex/字节线编码、包所有权隔离的 `ModeOptions`/`TargetOptions`/`metadata` 与 22 键 `language-items.conf` + `--*-protocol` 覆写 |
| `GAP-07-09` | 多包与目标信息 | `build.info` 目标/模式快照与多包 SCC 分波调度 | Core | [packages-and-umbrella.md](file:///home/fin/luna/docs/code-organization/packages-and-umbrella.md):133-141；扩展参见 [WL-06-10](file:///home/fin/luna/tmp/wishlist/06-attrs-build-pipeline-projection.md):56 | [library/build/info/target.fl](file:///home/fin/luna/library/build/info/target.fl):9-65、[package_cache.rs](file:///home/fin/luna/compiler/luna/src/package_cache.rs):571-775 (`schedule_package_waves`, `execute_build_plan_parallel`)、[build_context.rs](file:///home/fin/luna/compiler/luna/src/build_context.rs):295-520 | 🟡 Partial | 具备 `mode`/`target`/`package_metadata` 按包隔离及 Tarjan SCC `-j` 并行分波执行，但缺 `Target` `endianness`/`abi`/`triple` 与多目标联合清单 |
| `GAP-07-10` | 测试与产物发布 | `luna test`（VM / 原生 LLVM）与原子文件发布 | Core | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §1 | [testing.rs](file:///home/fin/luna/compiler/luna/src/testing.rs):1-330、[output.rs](file:///home/fin/luna/compiler/luna/src/output.rs):1-120、[host/tools.rs](file:///home/fin/luna/compiler/luna/src/host/tools.rs):388-456 | 🟡 Partial | 支持原子临时文件写入、reference VM 与 native 测试（含进程组隔离、超时控制与 `--filter`/`--list-tests`），但缺少并行测例运行与跨多包工作区测试发现 |
| `GAP-07-11` | 跨目标与管线投影 | 跨目标代码生成及外部后端投影交付（`pipeline` / `IIR`） | Core | [backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md):124-156、[target-validation-and-assembly.md](file:///home/fin/luna/docs/pipeline-projection/target-validation-and-assembly.md):121-178；扩展 [WL-06-21](file:///home/fin/luna/tmp/wishlist/06-attrs-build-pipeline-projection.md):67 | [host/llvm/platform.rs](file:///home/fin/luna/compiler/luna/src/host/llvm/platform.rs):11-18、[host/tools.rs](file:///home/fin/luna/compiler/luna/src/host/tools.rs):252,274-290,310,505-511,581-592,673-691,721-730、[cli/help.rs](file:///home/fin/luna/compiler/luna/src/cli/help.rs):22 | ⚠️ Divergent | `platform.rs` 硬编码 `x86_64/linux`；`tools.rs` 强校 `--host-target`、`llc` 无 `-mtriple`、`cc` 仅链 `-lm` 且强绑 Unix `CommandExt`/`UnixStream`；外部 `^attrs.compilation.pipeline` 句柄/产物恢复与校验全未接入 |
| `GAP-07-12` | 查询引擎 | 进程内 `QueryEngine` 记忆化、无环栈与依赖追踪 | Core | [configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md):174、[AGENTS.md](file:///home/fin/luna/AGENTS.md) §1 | [query/src/lib.rs](file:///home/fin/luna/compiler/query/src/lib.rs):1-390、[query/src/cache.rs](file:///home/fin/luna/compiler/query/src/cache.rs):1-155、[query/src/dependencies.rs](file:///home/fin/luna/compiler/query/src/dependencies.rs):1-85、[query/src/stack.rs](file:///home/fin/luna/compiler/query/src/stack.rs):1-62、[interface/src/lib.rs](file:///home/fin/luna/compiler/interface/src/lib.rs):154-156 | 🟡 Partial | 具备类型化同步单线程去重、并发等待、`BTreeMap`/`BTreeSet` 精确读写失效图；但 `interface` 仅接通 1 条 `hir_package` 查询，且 `QueryCache::clone` 清空数据 (`cache.rs:151-155`) |
| `GAP-07-13` | 持久化查询 | 跨会话查询指纹、Early-Cutoff 复用与全前端查询管线 | Core | [backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md):144-150、[configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md):178-180；扩展 [OQ-36](file:///home/fin/luna/tmp/wishlist/00-index-and-roadmap.md):430 | [query/src/lib.rs](file:///home/fin/luna/compiler/query/src/lib.rs):26-30（文档明确标注未实现持久化指纹与结果未变复用） | 🔴 Missing | 跨进程无法保存查询图与摘要，主包每次编译需从词法/语法起全量执行前端与 elaboration |
| `GAP-07-14` | 磁盘与流式缓存 | `.flpkg` 包快照缓存与常驻 `CompilationCache` | Core | [packages-and-umbrella.md](file:///home/fin/luna/docs/code-organization/packages-and-umbrella.md):129,139 | [package_cache.rs](file:///home/fin/luna/compiler/luna/src/package_cache.rs):1502,1676-2236、[packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs):1286,1292,1328,1810-1816,1858,1895-1900、[compilation_cache.rs](file:///home/fin/luna/compiler/luna/src/compilation_cache.rs):1-147 | 🟡 Partial | `ParsedPackage` 与 `VerifiedUpstream` (`FLPK` v1 + SHA-256) 已落盘；但 `ResolvedBase` 为死枚举、`ElaboratedBase` 仅存进程内 `OnceLock` (上限 32)、主包永不入盘、上游含 `@`/`scripts/` 即禁用全部磁盘缓存 |
| `GAP-07-15` | 精准失效 | 基于观测钩子的字段级 `InvalidationDep` 增量失效 | Wishlist | `[Wishlist: WL-06-10/13, OQ-36]` [06-attrs-build-pipeline-projection.md](file:///home/fin/luna/tmp/wishlist/06-attrs-build-pipeline-projection.md):56,59,1078 | [library/build/README.md](file:///home/fin/luna/library/build/README.md):39,96,123-125；[compilation_cache.rs](file:///home/fin/luna/compiler/luna/src/compilation_cache.rs):1-147 | 🔴 Missing | 驱动仍把整个构建上下文输入与发现目录哈希打包为保守全请求缓存键，改未读元数据字段仍触发重编 |
| `GAP-07-16` | VFS 与实例化 | `Vfs` 源码存储与 `CompilerInstance` 按需加载 | Core | [packages-and-umbrella.md](file:///home/fin/luna/docs/code-organization/packages-and-umbrella.md):29、[configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md):30-32 | [vfs/src/lib.rs](file:///home/fin/luna/compiler/vfs/src/lib.rs):1-234、[interface/src/lib.rs](file:///home/fin/luna/compiler/interface/src/lib.rs):63,77-86、[luna/frontend.rs](file:///home/fin/luna/compiler/luna/src/frontend.rs):349-364 | 🟡 Partial | `Vfs` 提供 `FileId`/`AstNodeId` 管理，但无内存覆盖/生成文件钩子与错误路径上下文；`CompilerInstance::new` 无条件扫描 Sysroot 即便后续丢弃 (`frontend.rs:349-364`) |
| `GAP-07-17` | 代码格式化 | `fmt` 文档布局引擎、`flurryfmt.fl` 与行内指令 | Core | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §1/§6 | [fmt/src/formatter.rs](file:///home/fin/luna/compiler/fmt/src/formatter.rs):101-200、[fmt/src/config.rs](file:///home/fin/luna/compiler/fmt/src/config.rs):1-100,430-530,620-720、[fmt/src/directives.rs](file:///home/fin/luna/compiler/fmt/src/directives.rs):308-340 | 🟡 Partial | Wadler 风格 `Doc` 布局、`flurryfmt.fl` 配置与行内 `--%` 指令较完善；但 `exprs.rs` 缺失 8 个 `NodeKind` 直接分支，且 `glob_match_inner` 存在多重通配回溯风险 |
| `GAP-07-18` | 代码格式化 | 二元运算符优先级表同步（`\|>` 错设为 `80`） | Core | [00-index-and-roadmap.md](file:///home/fin/luna/tmp/wishlist/00-index-and-roadmap.md):374 (`OQ-05`)、`docs/grammar/` | [fmt/src/exprs.rs](file:///home/fin/luna/compiler/fmt/src/exprs.rs):2617-2618 (`Pipe \| PipePrepend => 80`) | ⚠️ Divergent | `fmt` 与前端 parser 共享把 `\|>` 错置于 `+`/`*` 之上的优先级 `80`，直接扭曲管道表达式括号保护与换行决策 |
| `GAP-07-19` | 代码格式化 | `fmt` 桥接验证完整性与非 UTF-8 CLI 路径保真 | Core | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §1/§6 | [fmt/src/cli.rs](file:///home/fin/luna/compiler/fmt/src/cli.rs):134-137 (`to_string_lossy`)、[fmt/src/verify.rs](file:///home/fin/luna/compiler/fmt/src/verify.rs):104,163,169-173,219-230,261-269、[fmt/src/bridge.rs](file:///home/fin/luna/compiler/fmt/src/bridge.rs):116-128,725 | ⚠️ Divergent | `run_cli_with_io` 将 `OsString` 参数强转为 `String` 损坏非 UTF-8 路径；`verify.rs` 归一化属性/导入序并跳过全部宏桥接区 AST 检验 |
| `GAP-07-20` | 代码格式化 | `luna fmt` CLI 工作流与差异模式 | Core | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §1 | [fmt/src/cli.rs](file:///home/fin/luna/compiler/fmt/src/cli.rs):231-330,460-560、[fmt/src/diff.rs](file:///home/fin/luna/compiler/fmt/src/diff.rs):120-210 | 🟢 Full | 完整支持 `--check`、`--diff`（Myers O(ND)）、`--stdin`、`--stdin-filepath`、`--files-with-diff`、`--emit` 与退出码契约（仅超高行差 `max_d > 8192` 回退整文件替换） |
| `GAP-07-21` | 诊断渲染 | `diagnostic` 多跨度错误上下文与 `ariadne` 渲染 | Core | [target-validation-and-assembly.md](file:///home/fin/luna/docs/pipeline-projection/target-validation-and-assembly.md):178、[AGENTS.md](file:///home/fin/luna/AGENTS.md) §1 | [diagnostic/src/lib.rs](file:///home/fin/luna/compiler/diagnostic/src/lib.rs):190-444 | 🟡 Partial | `DiagnosticBuilder`/`failure_summary` 干净无测试副作用；但 `emit_to_ariadne` 丢弃跨文件辅助标签 (`L374-398`) 且 `DUMMY_SP` 下 `L349-350` 存在 `u32` 减法下溢风险 |
| `GAP-07-22` | IDE / LSP | `anar` LSP JSON-RPC/UTF-16 协议与 IDE 导航/重构集合 | Core | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §1 | [anar/src/server.rs](file:///home/fin/luna/compiler/anar/src/server.rs):77-484、[anar/src/json.rs](file:///home/fin/luna/compiler/anar/src/json.rs):1-551、[anar/src/rpc.rs](file:///home/fin/luna/compiler/anar/src/rpc.rs):1-238、[anar/src/position.rs](file:///home/fin/luna/compiler/anar/src/position.rs):1-271 | 🟡 Partial | 已落地 zero-dep JSON-RPC、UTF-16 偏移、`definition`/`references`/`rename`/`hover`/`completion`/`signatureHelp`/`documentSymbol`/`workspace/symbol`/`semanticTokens`/`foldingRange`/`formatting`；缺 `codeAction`/`inlayHint`/`typeDefinition`/`implementation`/`workspace/diagnostic` |
| `GAP-07-23` | IDE / LSP 语义 | `anar` LSP 全栈分析深度（宏展开与 Elaboration） | Core | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §1、[packages-and-umbrella.md](file:///home/fin/luna/docs/code-organization/packages-and-umbrella.md):133-138 | [anar/src/analysis.rs](file:///home/fin/luna/compiler/anar/src/analysis.rs):378-580,841,1260-1347、[anar/Cargo.toml](file:///home/fin/luna/compiler/anar/Cargo.toml):11-23 | ⚠️ Divergent | 仅依赖 `resolve`/`ast_lowering` 停留在 HIR 前段（零 `macro_expander`/`elaboration` 依赖）；注入空 AST 掩盖解析失败、吞掉跨包错误并通过 `detail` 字符串分割猜测点补全类型 |
| `GAP-07-24` | 基础设施健壮性 | `query` 环/中毒错误恢复与前端错误向驱动传播 | Core | [configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md):174、[backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md):150 | [query/src/lib.rs](file:///home/fin/luna/compiler/query/src/lib.rs):384-385、[query/src/cycle.rs](file:///home/fin/luna/compiler/query/src/cycle.rs):1-31、[query/src/dependencies.rs](file:///home/fin/luna/compiler/query/src/dependencies.rs):13,43,48 | 🟡 Partial | `CycleError` 能格式化完整调用栈，但 `QueryEngine` 遇到环或中毒直接 `panic!`，未能作为结构化 `Diagnostic` 返回 `luna` CLI 或常驻进程 |

---

## 4. 关键缺口深度剖析（含源码证据）

### 4.1 工程描述与多阶段构建任务图从缺：`package.fl`、`scripts/` 与 `build.{package,targets,tasks}`

[configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md):18-44 与 `155-184` 及 [packages-and-umbrella.md](file:///home/fin/luna/docs/code-organization/packages-and-umbrella.md):16-48 明确定义了 Flurry 的工程组织：以 `package.fl` 为 comptime Flurry 工程描述文件，通过 `build.{package,targets,tasks}`（对应扩展规划 [WL-06-11..13](file:///home/fin/luna/tmp/wishlist/06-attrs-build-pipeline-projection.md):57-59）由单一 `luna` 二进制编排多任务 DAG `schedule(selected_targets)`，先编制 `scripts/`（提供 `@use` 构造宏接口与生成源码），再启动主包 `src/{main,lib}.fl` 的解析与检查，同时严格保持 Umbrella 成员之间的语言作用域与实现边界隔离。

对照现行实现，这一整层处于 **完全缺位** 状态：
1. **清单发现停留在硬编码后缀**：`compiler/{luna,anar,interface}/src/` 下无任何 `package.fl` 或 Umbrella 拓扑解析逻辑；[anar/src/analysis.rs](file:///home/fin/luna/compiler/anar/src/analysis.rs):312-315 的 `find_package_root` 只沿父目录查找 `lib.fl` 或 `main.fl`。
2. **`library/build/` 仅剩只读 `info` 快照**：[library/build/lib.fl](file:///home/fin/luna/library/build/lib.fl):2-4 仅重导出 `info.mode.mode`、`info.package.*`、`info.target.*`（与 [build_context.rs](file:///home/fin/luna/compiler/luna/src/build_context.rs):295-520 的 `ModeOptions`、`TargetOptions` 及 [build_context/metadata/wire.rs](file:///home/fin/luna/compiler/luna/src/build_context/metadata/wire.rs) `FMD\x01` 协议对接）；而 `package/`、`targets/`、`tasks/`、`generated/`、`util/` 5 个子目录全部只剩占位 `README.md`，且 [library/build/README.md](file:///home/fin/luna/library/build/README.md):39,96,123-125 坦承目前既无工作区清单/构建脚本调度，也无字段级观测失效。
3. **`scripts/` 前置编排缺失**：[packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs):1292,1328 在发现包内存在 `scripts/` 时只是保守关闭 `ParsedPackage`/`VerifiedUpstream` 缓存，而非按任务 DAG 分波编排宏构建与产物登记。

### 4.2 增量缓存体系的架构断层：从内存 `QueryEngine` 到磁盘 `.flpkg`

`luna` 与 `query` 分别演化出两套互不打通的缓存设施，各存在明显半成品断点：
- **`compiler/query` 仅限于单次进程内存去重**：[query/src/lib.rs](file:///home/fin/luna/compiler/query/src/lib.rs):26-30 在源码注释中明确说明未实现持久化指纹与结果未变复用（Early-Cutoff）；[interface/src/lib.rs](file:///home/fin/luna/compiler/interface/src/lib.rs):154-156 通过 `QueryEngine` 仅注册了 1 条 `hir_package` 查询，且 `CompilerInstance::new` (`interface/src/lib.rs:63,77-86`) 只要遇到 `sysroot = Some(...)` 即通过 `Vfs::scan` 预先全量扫描 `builtin`+`std`，即便是 [frontend.rs](file:///home/fin/luna/compiler/luna/src/frontend.rs):349-364 的 `compile_sysroot` 等随即丢弃该状态的调用也无一幸免。
- **`luna/src/package_cache.rs` 磁盘快照仅覆盖无宏上游且缺阶段产物**：
  - [package_cache.rs](file:///home/fin/luna/compiler/luna/src/package_cache.rs):1502 定义的 `ArtifactKind::ResolvedBase = 1` 在全库无消费者（已成死枚举）。
  - `ArtifactKind::ElaboratedBase = 2` 仅存在于进程内 `OnceLock` 有界 LRU 缓存 `global_elaborated_cache`（上限 32 项，[packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs):1858,1895-1900）；跨 CLI 进程写入磁盘的 `.flpkg` `VerifiedUpstream` ([package_cache.rs](file:///home/fin/luna/compiler/luna/src/package_cache.rs):2163-2165) 仅存储验证通过的 `Vec<u64>` 包身份认证凭证，冷启动新进程仍必须完整重跑所有上游包的名称解析、AST lowering 与 elaboration（[packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs):1286 更彻底跳过主包自身的 `ParsedPackage` 落盘）。
  - 只要任何上游包含 `@` 宏调用或 `scripts/` 目录（`any_upstream_used_macros || has_expansion || emit != Emit::Nir`，[packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs):1292,1328,1810-1816），整条 `ParsedPackage` 与 `VerifiedUpstream` 缓存直接全局失效。

### 4.3 跨目标工程能力与宿主工具链深度耦合

尽管 CLI（[cli.rs](file:///home/fin/luna/compiler/luna/src/cli.rs):236-266）与 `build.info.target`（[target.fl](file:///home/fin/luna/library/build/info/target.fl):9-65）暴露了 `--build-target`、`--package-build-target` 与 `--build-feature`，但实际后端工具链把“编译期逻辑目标视图”与“实际代码生成目标”彻底绑死在本机宿主之上：
- [cli/help.rs](file:///home/fin/luna/compiler/luna/src/cli/help.rs):22 明确声明：`Targets describe build context, not cross-compiling backend selection.`
- [host/llvm/platform.rs](file:///home/fin/luna/compiler/luna/src/host/llvm/platform.rs):11-18 硬编码限定 `cfg!(all(target_os = "linux", target_arch = "x86_64"))` 与 64 位指针宽度。
- [host/tools.rs](file:///home/fin/luna/compiler/luna/src/host/tools.rs):505-511 与 `581-592` 永远调用 `llvm-config --host-target` 探测三元组与数据布局，并对输入的 LLVM IR 执行强制宿主一致性校验（mismatch 直接拒绝）；`compile_native_in`（`L673-691`）调用 `llc` 未传入 `-mtriple`/`-mcpu`/`-mattr`；`compile`（`L721-730`）硬编码 `self.linker` 仅附带 `-lm` 链接；且进程管理与管道（`L252,274-290,310`）强依赖 Unix `CommandExt`、`extern "C" kill(-pid, 9)` 与 `UnixStream::pair()`，无法原生迁移至非 Unix 宿主。
- 同时，[backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md):124-156 与 [WL-06-21](file:///home/fin/luna/tmp/wishlist/06-attrs-build-pipeline-projection.md):67 规划的 `^attrs.compilation.pipeline` 外部管线、`meta.compilation.IIR.access` 临时句柄隔离及 `ArtifactManifest` 跨进程恢复协议在 `luna` 与 `build` 中均尚无实现。

### 4.4 `anar` LSP 与 `fmt` 工程化深度局限

- **`anar` 仅是语法/解析期 IDE 索引器**：[anar/Cargo.toml](file:///home/fin/luna/compiler/anar/Cargo.toml):11-23 仅依赖至 `resolve` 与 `ast_lowering`，完全不接入 `macro_expander` 与 `elaboration`。因此 [anar/src/analysis.rs](file:///home/fin/luna/compiler/anar/src/analysis.rs):378-580,1260-1347 既无法展开 `@macro` 生成的定义，也无法获得真实类型推断/能力约束/效应行信息，甚至在 `L1308-1324` 靠切割悬浮文档字符串 `detail` 来猜测点号补全接收者类型。
- **`fmt` 路径语义与校验覆盖留有后门**：`fmt` 的桥接层 ([bridge.rs](file:///home/fin/luna/compiler/fmt/src/bridge.rs):116-128) 把传统扩展构造、属性关键字与宏体当作半透明字节切片，而校验器 ([verify.rs](file:///home/fin/luna/compiler/fmt/src/verify.rs):104,163,169-173,219-230,261-269) 在做 AST 比对时既归一化跳过顺序/属性节点，也直接豁免了宏桥接区间内部节点对比。

---

## 5. 架构偏离与技术债清单（`⚠️ Divergent` 专项）

下表与 §3 中全部 **6 项** `⚠️ Divergent` 条目建立 **1 对 1 严格对应**，逐一核对规范冲突及重构方向：

| 对应矩阵 ID | 偏离位置（`file:line`） | 现行实现与规范冲突点 | 重构与矫正要求 |
|---|---|---|---|
| `GAP-07-02` | [Cargo.toml](file:///home/fin/luna/Cargo.toml):29, [anar/Cargo.toml](file:///home/fin/luna/compiler/anar/Cargo.toml):2-5, [anar/server.rs](file:///home/fin/luna/compiler/anar/src/server.rs):88,580, [anar/analysis.rs](file:///home/fin/luna/compiler/anar/src/analysis.rs):79, [anar/cli.rs](file:///home/fin/luna/compiler/anar/src/cli.rs):12 | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §1 与 [packages-and-umbrella.md](file:///home/fin/luna/docs/code-organization/packages-and-umbrella.md):47 明确取消独立 `anar` 计划，统一于 `luna` 单二进制 CLI；但现行代码仍把 `anar` 作为独立命名 crate、`settings.flurry.anar.sysroot` 配置根、`"anar"` LSP server 标识及诊断来源暴露给用户端 | 将 LSP 配置域迁移为 `flurry.lsp.*` / `flurry.sysroot`，server name 与 diagnostic source 统一改为 `"luna"`；逐步将 `compiler/anar` 重构并入 `luna-lsp` 内部模块并复用 `luna` 前端查询会话 |
| `GAP-07-07` | [sysroot.rs](file:///home/fin/luna/compiler/intrinsic/src/sysroot.rs):179, [packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs):459,604, [frontend.rs](file:///home/fin/luna/compiler/luna/src/frontend.rs):677-680,701-730 | [AGENTS.md](file:///home/fin/luna/AGENTS.md) §5 明确规定 6 个内置包与 5 个默认 prelude（`builtin`/`meta`/`build`/`attrs`/`verification`，排除 `std` 与自身），而 `Sysroot::packages()` 仅枚举 `[builtin, std]` 两包；`packages.rs` `compile_with_builtin`/`compile_observed`/`compile_with_definitions` 默认仅注入 `&[builtin]` 单 prelude；`Emit::Hir` 绕过 `language-items.conf` 协议绑定 | 统一由唯一权威 6 包 Sysroot 清单驱动 `interface` 与 `luna`；消除默认单 prelude 捷径入口，让内部与外部调用共享一致的 5-prelude 注入与 `language-items.conf` 解析逻辑 |
| `GAP-07-11` | [host/llvm/platform.rs](file:///home/fin/luna/compiler/luna/src/host/llvm/platform.rs):11-18, [host/tools.rs](file:///home/fin/luna/compiler/luna/src/host/tools.rs):252,274-290,310,505-511,581-592,673-691,721-730 | [configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md):48 与 [packages-and-umbrella.md](file:///home/fin/luna/docs/code-organization/packages-and-umbrella.md):47 强调“宿主配置不等于生成程序目标”，[backends-and-artifacts.md](file:///home/fin/luna/docs/pipeline-projection/backends-and-artifacts.md) 要求管线解耦外部后端产物；但代码硬编码 `x86_64/linux` 与 `--host-target` 一致性，使 `--build-target` 与后端解耦契约互相矛盾 | 分离 `HostToolchain` 与 `TargetSpec`：支持向 `opt`/`llc`/`lld` 显式传 `-mtriple`/`-mcpu` 与跨平台链接库参数；补齐独立进程控制抽象；为外部 `^attrs.compilation.pipeline` 接入可验证的 `IIR.access` 与 `ArtifactManifest` |
| `GAP-07-18` | [fmt/src/exprs.rs](file:///home/fin/luna/compiler/fmt/src/exprs.rs):2617-2618 | 与 parser 共同将 `Pipe` / `PipePrepend` (`\|>`) 优先级硬编码为 `80`（高于 `Mul`/`Div` 与 `Add`/`Sub` 的 `60`/`70`），违背管道低优先级从左到右组合语义（`a + 1 \|> f` 误当作 `a + (1 \|> f)`，见 [OQ-05](file:///home/fin/luna/tmp/wishlist/00-index-and-roadmap.md):374） | 与前端 `compiler/parse` 协同将 `\|>` 优先级统一下调至 `35`（或 `15`）并补全 `==>` 右结合格式化规则与括号消歧回归 |
| `GAP-07-19` | [fmt/src/cli.rs](file:///home/fin/luna/compiler/fmt/src/cli.rs):134-137, [fmt/src/verify.rs](file:///home/fin/luna/compiler/fmt/src/verify.rs):104,163,169-173,219-230,261-269 | `luna` 主 CLI 在 [cli.rs](file:///home/fin/luna/compiler/luna/src/cli.rs):780、[cli/help.rs](file:///home/fin/luna/compiler/luna/src/cli/help.rs):23 明确支持非 UTF-8 `OsString` 路径，但 `fmt` 驱动先将其 `.to_string_lossy()` 转 `Vec<String>` 造成路径破坏；同时 `verify.rs` 跳过宏桥接区内部与属性对比，背离“零语义变更格式化”承诺 | 将 `compiler/fmt/src/cli.rs` 参数接口改为 `OsString`/`PathBuf` 全链路保真；升级 `verify.rs` 对重新解析后的宏片段与属性/顺序执行结构严格比对（非归一化静默放行） |
| `GAP-07-23` | [anar/src/analysis.rs](file:///home/fin/luna/compiler/anar/src/analysis.rs):378-580,841,1260-1347, [anar/Cargo.toml](file:///home/fin/luna/compiler/anar/Cargo.toml):11-23 | IDE 服务必须准确反馈真实类型、宏展开与能力/错误结果，而 `anar` 只运行到 `ast_lowering`，且对解析出错文件注入 `build_empty_ast`、吞掉跨包构建错误并以 `detail` 文本切片伪造点补全类型，造成 IDE 语义与 `luna check` 长期分叉 | 让 `luna lsp` 通过 `interface`/`query` 共享同一前端与递增 elaboration 管线（对展开定义建虚位置映射，并从真实 HIR/NIR 类型绑定计算点补全与悬浮签名） |

---

## 6. 健全性 · 完备性 · 鲁棒性专项审计（`SND / CMP / ROB`）

### 6.1 健全性审计（`SND-07-01` .. `SND-07-07`）

| 编号 | 严重度 | 健全性隐患 | 源码实证（`file:line`） | 失败后果与触发路径 |
|---|---|---|---|---|
| `SND-07-01` | High | `extra_config_files` 缓存键文件名碰撞、读取失败混淆及 `rel_path` 有损序列化 | [package_cache.rs](file:///home/fin/luna/compiler/luna/src/package_cache.rs):1254-1271 (`compute_unit_merkle_key`), 1773-1791 (`compute_parsed_package_key`) | `extra_config_files` 仅哈希 `path.file_name()` 而丢弃目录层级，且 `fs::read(path)` I/O 错误与文件不存在均喂入 `update_bool(false)`；`compute_parsed_package_key` 以 `to_string_lossy()` 编码 `rel_path` 且忽略空 `vfs.source_directories` 变更，可导致不同配置碰撞误命中旧 `.flpkg` |
| `SND-07-02` | High | `compiler_fingerprint` 退化静默忽略工具链变化，及内部调用默认单 prelude / `Emit::Hir` 绕过协议配置 | [package_cache.rs](file:///home/fin/luna/compiler/luna/src/package_cache.rs):957-1006; [packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs):459,604; [frontend.rs](file:///home/fin/luna/compiler/luna/src/frontend.rs):677-680 | 在非 ELF 或被 strip 的二进制上，`compiler_fingerprint` 回退至文件 stat 或静态串 `"flurry-compiler-fallback-v1"`，全程不哈希 `llvm-config`/`cc` 工具链版本号；内部辅助接口默认只注单 prelude 且 `Emit::Hir` 不读 `language-items.conf` 默认键，易使下游检验行为与主 CLI 出现偏差 |
| `SND-07-03` | High | 常驻包流缓存未追踪外部配置文件失效，且产物验证只查文件非空 | [compilation_cache.rs](file:///home/fin/luna/compiler/luna/src/compilation_cache.rs):46-103; [host/tools.rs](file:///home/fin/luna/compiler/luna/src/host/tools.rs):739-755 (`validate_artifact`) | `changed_discovery`/`changed_sources` 仅校验源码目录及 `.fl` 文件字节，若进程外修改 `language-items.conf` 且显式 `--packages-v1` 未变更对应键则有漏失效风险；`validate_artifact` 仅检查 `metadata.len() > 0`，无法捕获截断 ELF 或错误 ABI 对象 |
| `SND-07-04` | High | `fmt` 验证器系统性跳过宏区 AST 比对与属性/导入顺序校验 | [fmt/src/verify.rs](file:///home/fin/luna/compiler/fmt/src/verify.rs):104,163,169-173,219-230,261-269 | `verify.rs` 主动归一化 `ExprStatement`、丢弃属性树与顺序比较，并跳过所有宏桥接切片（`--% .skip`、`@path{...}`、`define`、`{{...}}`）内部等价校验，格式化器若误改宏实参分号/括号将无法自检拦阻 |
| `SND-07-05` | Medium | `fmt` 运算符优先级把 `\|>` 设为 `80` 导致错误结合保护 | [fmt/src/exprs.rs](file:///home/fin/luna/compiler/fmt/src/exprs.rs):2617-2618 | 当格式化需要判定内嵌表达式是否加括号时，`Pipe`/`PipePrepend` (`80`) 高于算术运算符 (`60`/`70`)，造成源码形式重构后与语言目标文法语义冲突 |
| `SND-07-06` | High | `anar` LSP 解析错误注入空 AST 并静默吞掉跨包错误与省略宏/Elaboration 检查 | [anar/src/analysis.rs](file:///home/fin/luna/compiler/anar/src/analysis.rs):495-499,841,1308-1324 | 破坏性语法错误文件会以 `build_empty_ast` 参与构建并吞掉 `build_package_set_with_preludes` 错误返回，可能对损坏包输出不真实的类型/符号诊断，造成“编辑器无错而 `luna check` 失败”的假阴性/假阳性 |
| `SND-07-07` | Medium | `QueryCache::clone` 静默返回空缓存且跨进程无指纹校验 | [query/src/cache.rs](file:///home/fin/luna/compiler/query/src/cache.rs):151-155; [query/src/lib.rs](file:///home/fin/luna/compiler/query/src/lib.rs):26-30 | `QueryCache::clone` 不保留原记忆化状态与读依赖图而直接构造空 `Mutex<HashMap::new()>`，同时无跨进程指纹与 unchanged-result 复用，复制引擎实例会隐式退回冷启动且无法复用中间结果 |

### 6.2 完备性审计（`CMP-07-01` .. `CMP-07-06`）

| 编号 | 严重度 | 完备性缺口 | 源码实证与分布统计 | 影响范围 |
|---|---|---|---|---|
| `CMP-07-01` | Critical | 工程清单、Umbrella 工作区与 `build.{package,targets,tasks,generated,util}` 构建流全缺 | `package.fl` 全库 0 实现；[library/build/README.md](file:///home/fin/luna/library/build/README.md):39,96,123-125；[docs/code-organization/configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md):155-184 | 无法脱离外部包装脚本以标准 `luna build`/`luna test` 组织多包工程、构建生成文件或前置 `scripts/` 宏库 |
| `CMP-07-02` | High | 跨目标编译、外部管线投影与非 Unix 宿主支持缺失 | [host/llvm/platform.rs](file:///home/fin/luna/compiler/luna/src/host/llvm/platform.rs):11-18; [host/tools.rs](file:///home/fin/luna/compiler/luna/src/host/tools.rs):252,274-290,310,505-511,581-592,673-691,721-730 | 无法输出 ARM64/RISC-V/GPU 等异构目标产物，无法在 Windows 编译运行，且无法交付 `^attrs.compilation.pipeline` 外部后端插件 |
| `CMP-07-03` | High | 跨进程 `ElaboratedBase`/`ResolvedBase`/主包/含宏包缓存空白及单查询接线 | [packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs):1286,1292,1328,1810-1816; [package_cache.rs](file:///home/fin/luna/compiler/luna/src/package_cache.rs):1502; [interface/src/lib.rs](file:///home/fin/luna/compiler/interface/src/lib.rs):154-156 | 下游反复迭代时无法落盘跳过主包自身或含 `@` 宏上游包的 elaboration；无字段级 `InvalidationDep` 钩子；缺 `watch`/`doc`/`bench`/`init`/`clean` 命令 |
| `CMP-07-04` | Medium | `Sysroot` 两包清单、LSP 高阶能力及 `fmt` 未直连节点 | [intrinsic/src/sysroot.rs](file:///home/fin/luna/compiler/intrinsic/src/sysroot.rs):179; [anar/src/server.rs](file:///home/fin/luna/compiler/anar/src/server.rs):77-484; [fmt/src/exprs.rs](file:///home/fin/luna/compiler/fmt/src/exprs.rs) | 8 个 AST `NodeKind`（`ClosureEnvironmentBinding`、`CodeContextBinding`、`CodeSubstitution`、`DeclarationBound`、`FieldMethodBound`、`TokenBufferDollar/Interp/Token`）无直接分支；LSP 缺少 `codeAction`/`inlayHint`/`typeDefinition`/`implementation`/`rangeFormatting`/`workspace/diagnostic` |
| `CMP-07-05` | Medium | **307 处** `_ =>` 通配手臂隐藏语义/格式化退化分支 | 7 Crate 精确计数：`luna: 190`、`fmt: 65`、`anar: 50`、`query: 1`、`diagnostic: 1`、`vfs: 0`、`interface: 0`；例：[fmt/src/exprs.rs](file:///home/fin/luna/compiler/fmt/src/exprs.rs):536,735,803,1100,1438,2090,2266、[fmt/src/items.rs](file:///home/fin/luna/compiler/fmt/src/items.rs):898（默认双分支通配回退）、[anar/src/index.rs](file:///home/fin/luna/compiler/anar/src/index.rs):3377（未知 Expr 静默降为 `"..."`） | 一旦 AST/HIR/CLI 扩展新变体，二分支 `_ =>` 与占位降级臂不会触发 Rust 穷尽性编译错误，而是静默错打印关键字或操作符 |
| `CMP-07-06` | Low | `fmt` diff 上界退化与 `luna test` 单线程串行执行 | [fmt/src/diff.rs](file:///home/fin/luna/compiler/fmt/src/diff.rs):168; [testing.rs](file:///home/fin/luna/compiler/luna/src/testing.rs):183 | 超过 `max_d > 8192` 的巨型差异文件整体退化为全删+全插；测试执行循环未利用工作线程池并发执行独立用例 |

### 6.3 鲁棒性审计（`ROB-07-01` .. `ROB-07-06`）

#### 7-Crate 非测试生产代码（`src/**/*.rs`）Panic 风险面与确定性全量普查

> 普查方法：静态扫描 7 个 Crate 下全部 `.rs` 源文件，自动解析并剔除 `#[cfg(test)] #[path = "..."] mod ...;` 外部测试文件（已实证剔除 [method_contract_tests.rs](file:///home/fin/luna/compiler/luna/src/host/method_contract_tests.rs)、[resources/tests.rs](file:///home/fin/luna/compiler/luna/src/resources/tests.rs) 与 [handler_frame_tests.rs](file:///home/fin/luna/compiler/luna/src/host/llvm/handler_frame_tests.rs)）以及全量内联 `#[cfg(test)]` 块，并过滤单行/块注释与字面量。

| Crate | 非测文件 | 非测 LOC | `.unwrap()` | `.expect()` | `panic!()` | `unreachable!()` | `todo!()`/`unimpl!()` | `.unwrap_or*()` | 裸切片 `[idx]` (~) | `_ =>` 臂 | `DUMMY_SP` 总引用 (非检查发射/回退) | `HashMap` / `HashSet` |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---:|
| `luna` | 64 | 41,481 | 39 | 27 | 0 | 32 | 0 / 0 | 83 | 313 | 190 | 3 (`1` import + `1` check + `1` decode) | 57 / 29 |
| `anar` | 8 | 7,754 | 1 | 0 | 0 | 1 | 0 / 0 | 51 | 29 | 50 | 5 (`3` checks + `2` fallbacks) | 33 / 23 |
| `query` | 7 | 822 | 0 | 3 | 2 | 0 | 0 / 0 | 21 | 0 | 1 | 0 | 5 / 0 |
| `vfs` | 1 | 234 | 0 | 0 | 0 | 0 | 0 / 0 | 2 | 0 | 0 | 0 | 0 / 0 |
| `fmt` | 13 | 12,633 | 6 | 1 | 0 | 0 | 0 / 0 | 198 | 622 | 65 | 1 (`1` check) | 10 / 6 |
| `diagnostic` | 1 | 444 | 0 | 0 | 0 | 0 | 0 / 0 | 6 | 0 | 1 | 2 (`1` check + `1` fallback) | 0 / 0 |
| `interface` | 2 | 237 | 0 | 0 | 0 | 0 | 0 / 0 | 0 | 0 | 0 | 0 | 0 / 0 |
| **合计** | **96** | **63,605** | **46** | **31** | **2** | **33** | **0 / 0** | **361** | **964** | **307** | **11（其中 4 处非检查发射/回退）** | **105 / 58** |

> 说明：硬 panic 原语总计 **`112` 处**（`46 unwrap + 31 expect + 2 panic! + 33 unreachable!`），零 `todo!()` / `unimplemented!()`。在 `DUMMY_SP` 的 **11 处**源码字符匹配中，**6 处**为守卫检查，**1 处**为 `use` 导入，其余 **4 处**实际落盘或回退发射点位于 [package_cache.rs](file:///home/fin/luna/compiler/luna/src/package_cache.rs):2028（解包哨兵占位）、[anar/src/analysis.rs](file:///home/fin/luna/compiler/anar/src/analysis.rs):625,778 与 [diagnostic/src/lib.rs](file:///home/fin/luna/compiler/diagnostic/src/lib.rs):324。

| 编号 | 严重度 | 鲁棒性风险 | 精确位置与实证 | 异常场景与崩溃机理 |
|---|---|---|---|---|
| `ROB-07-01` | High | `diagnostic` 触发 `DUMMY_SP` 时 `u32` Span 减法下溢 panic 及跨文件标签静默丢弃 | [diagnostic/src/lib.rs](file:///home/fin/luna/compiler/diagnostic/src/lib.rs):324,349-350,374-398 | 当无 `primary_span` 且 `labels` 为空时，`L324` 回退到 `DUMMY_SP` (`lo=0, hi=0`)；若 `source_map` 中文件以 `BytePos(1)` 起算（如缓存 AST 解码约定），`primary_span.lo().0 - source_file.start_pos.0` 直接无符号整数下溢（Debug 模式崩溃，Release 回绕为 4GiB 触发无效切片）；`L377` 仅保留同文件 label，其余跨文件宏/调用标签静默消失 |
| `ROB-07-02` | High | `QueryEngine` 检测到循环依赖或中毒时直接 `panic!` 击溃宿主 | [query/src/lib.rs](file:///home/fin/luna/compiler/query/src/lib.rs):384-385; [query/src/dependencies.rs](file:///home/fin/luna/compiler/query/src/dependencies.rs):13,43,48 | `QueryResult::Cycle(err)` 与 `QueryResult::Poisoned` 在顶层直接 `panic!`，常驻模式 `serve packages-v1` 或接入该引擎的 LSP 进程一旦遭遇查询环会整进程闪退，且 `dependencies.rs` 中帧栈状态破坏也会触发 `.expect` 崩溃 |
| `ROB-07-03` | High | `fmt` 忽略规则通配符 `glob_match_inner` 指数回溯 ReDoS 与路径编码损失 | [fmt/src/config.rs](file:///home/fin/luna/compiler/fmt/src/config.rs):645-693; [fmt/src/cli.rs](file:///home/fin/luna/compiler/fmt/src/cli.rs):134-137 | 含多个 `**`/`*` 的 `ignore` 模式（如 `**/**/**/x` 匹配长路径失败时）无记忆化 DP 表，复杂度呈指数爆炸；`run_cli_with_io` 将 OS 参数有损转换 `.to_string_lossy()`，含非 UTF-8 字节的合法 POSIX 路径直接损坏找不到文件 |
| `ROB-07-04` | Medium | `package_cache` 工作线程/锁崩溃放大与 CLI/元数据前置 `.unwrap()` 依赖 | [package_cache.rs](file:///home/fin/luna/compiler/luna/src/package_cache.rs):438,444,448,456-457,620,760,773,1707-1764; [packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs):1597-1598,1660-1661; [metadata.rs](file:///home/fin/luna/compiler/luna/src/build_context/metadata.rs):220 | 任一并行包解析工作线程恐慌都将沿 `Mutex` 中毒与 `.join().expect()` 放大传播；前端入口与 LLVM/格式切片仍残留 `46` 个 `.unwrap()` 及约 `964` 处直接 `[idx]` 下标索引（集中于 `fmt` 622 与 `luna` 313），依赖隐式长度不变量 |
| `ROB-07-05` | Medium | `HashMap`/`HashSet` 迭代顺序非确定性暴露 | [anar/src/server.rs](file:///home/fin/luna/compiler/anar/src/server.rs):452; [anar/src/index.rs](file:///home/fin/luna/compiler/anar/src/index.rs):240-252 | 全量 `HashMap` (105 处) / `HashSet` (58 处) 中，`package_cache` 与 `build_context` 多数已用 `BTreeMap` 或显式 `sort()`；但 `anar` 的 `republish_open_diagnostics` (`L452`) 按 `open_docs` (`HashMap`) 随机哈希序遍历发通知，且核心 `SymbolIndex` 依赖 8 张 `HashMap` 维护索引，要求调用方不得漏排序 |
| `ROB-07-06` | Low | 外部 LLVM 子进程硬固定 120s 超时与 Unix 强耦合资源边界 | [host/tools.rs](file:///home/fin/luna/compiler/luna/src/host/tools.rs):252,274-290,310,388-456; [package_stream.rs](file:///home/fin/luna/compiler/luna/src/package_stream.rs):1-130 | 外部 LLVM 工具写死 120s 时限不可按工程规模配置；`package_stream.rs` 单帧 1 MiB 上限可截断超大规模源码/诊断负载（但有错误返回，无静默吞错） |

---

## 7. 修复路径、依赖与验收建议

### P0：止血修正规范偏离、基础健全性与崩溃点（立即实施）
1. **统一 `diagnostic` 跨文件渲染与零下溢保护**（解决 `ROB-07-01`、`GAP-07-21`）：在 [diagnostic/src/lib.rs](file:///home/fin/luna/compiler/diagnostic/src/lib.rs):349-350,377 用 `saturating_sub` 替代无符号 `-`，对 `DUMMY_SP` 降级为纯文本摘要输出不强查 `SourceMap` 偏移，并对跨文件 `label` 逐一加载源码渲染而非丢弃。
2. **统一 6 包 `Sysroot` 清单与 5-Prelude 默认规则**（解决 `GAP-07-07`、`SND-07-02`）：将 [sysroot.rs](file:///home/fin/luna/compiler/intrinsic/src/sysroot.rs):179 扩展为全量 6 包（`builtin, meta, build, attrs, verification, std`）；收口 [packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs):459,604 单 prelude 捷径与 `Emit::Hir` 跳过 `language-items.conf` 默认键行为；让 `CompilerInstance::new` 按需延迟扫描 Sysroot。
3. **修复 `fmt` 路径编码、`|>` 优先级与 `glob_match` ReDoS**（解决 `GAP-07-18`、`GAP-07-19`、`ROB-07-03`、`SND-07-05`）：将 `fmt/src/cli.rs` 参数接口改为 `OsString`/`PathBuf`；同步 `fmt` `Pipe`/`PipePrepend` 优先级与前端修正一致；使用线性的 DP/NFA 状态匹配重写 `glob_match_inner` 消除指数回溯。
4. **修复 `.flpkg` 配置快照键与文件缺失区分**（解决 `SND-07-01`）：`compute_unit_merkle_key` 改用规范绝对/相对路径加显式 `Missing` vs `IoError` vs `Present(hash)` 状态标记；`compute_parsed_package_key` 纳入空目录列表与原始 `OsStr` 字节。

### P1：工程清单、`luna` 单二进制收敛、查询管线统一与 `anar` 语义升级
1. **落地 `package.fl` 清单与 `build.{package,targets,tasks}` 核心 DAG**（解决 `GAP-07-03`、`GAP-07-04`、`CMP-07-01`）：在 `luna` 中新增 comptime `package.fl` 求值入口与 `scripts/` -> `src/` 拓扑调度（先编译执行宏库/生成任务，校验输出再入主包），取代裸 `--package` 手工传参依赖，并为 Umbrella 成员严格隔离名称作用域与 `impl` 归属。
2. **彻底收敛 `anar` 至 `luna` 内部并打通 Elaboration LSP**（解决 `GAP-07-02`、`GAP-07-23`、`SND-07-06`、`ROB-07-05`）：重命名 `flurry.anar.sysroot` 为 `flurry.lsp.sysroot` 及 server/source 为 `"luna"`；让 `luna lsp` 通过 `QueryEngine` 复用前端宏展开与增量 elaboration（不再注入空 AST 与分割 `detail` 字符串猜类型），并将 `QueryResult::Cycle`/`Poisoned` 改为 `Result<T, QueryFailure>` 可控诊断（`ROB-07-02`、`GAP-07-24`），同时按排序输出多文件通知与索引。
3. **打通 `fmt` 宏与属性结构等价验证、清除高频二分支 `_ =>` 盲区**（解决 `SND-07-04`、`CMP-07-04`、`CMP-07-05`）：补齐剩余 8 个 `NodeKind` 直接格式化，消除双分支 `_ =>` 以恢复编译器穷尽性检查，升级 `verify.rs` 比对属性列表与宏展开片段。

### P2：跨平台后端、全阶段持久化缓存与可观测工具链
1. **解耦宿主与目标代码生成、落地外部管线投影**（解决 `GAP-07-11`、`CMP-07-02`）：扩展 `host/tools.rs` 接受显式 `-mtriple`/`-mcpu`/`-mattr` 与库链接参数、抽象跨平台进程/管道捕获（去除非 Unix 阻断）；接入 `^attrs.compilation.pipeline` + `meta.compilation.IIR.access` 冻结句柄传递与 `ArtifactManifest` 跨进程验证恢复。
2. **统一 `QueryEngine` + `.flpkg` 持久化与主包/含宏上游增量缓存**（解决 `GAP-07-12`、`GAP-07-13`、`GAP-07-14`、`CMP-07-03`）：引入跨进程查询摘要存储、unchanged-result early-cutoff、把 `ElaboratedBase` 序列化至 `.flpkg`（移除死枚举 `ResolvedBase` 或将其并入阶段产物），并围绕显式依赖边支持含 `@` 宏上游与主包分模块重用。
3. **补齐增量构建工具与开发者 CLI 生态**（解决 `GAP-07-06`、`GAP-07-10`）：补充 `luna watch`、`luna doc`、`luna clean`、`luna init` 与多线程并行测试运行器。

### P3：字段级精准失效、构建沙盒与工业级增量上限
1. **引入观测字段级 `InvalidationDep`**（解决 `GAP-07-15`、`SND-07-03`、[OQ-36](file:///home/fin/luna/tmp/wishlist/00-index-and-roadmap.md):430）：记录每次构建真实读取的 `TargetFeatures`/`MetadataFeatures`/文件区间/环境变量，仅对被观测字段变动触发上游失效。
2. **完成 `build.{generated,util}` 生成产物与脚手架沙盒**（解决 `GAP-07-05`）：限制外部工具/写文件逃逸包目标目录，与 DAG 失效边自动绑定。

### 跨阶段测试验收纲要（Verifiable Acceptance Criteria）
- **基线健全性与边界回归**：新增 `diagnostic` 零 `primary_span` + 空 `labels` + `BytePos(1)` 源文件渲染不 panic 且保留跨文件标签用例；`fmt` 非 UTF-8 路径原样写入用例、`glob_match` 连续 10 段 `**/**` 秒级返回防 ReDoS 用例、同名跨目录 `extra_config_files` 不碰撞与无权限配置报错不命中缓存用例。
- **工程与缓存闭环**：使用真实 `library/` 六包 `sysroot`，构造“主包 + 内部 `scripts/` 宏包 + 多目标依赖包 + `package.fl`”端到端构建：验证仅修改未使用的 `metadata` 字段时不触发全量重编（P3）、修改上游源码/含 `@` 宏上游实现仅失效受影响查询或波次单元（P2）、遭遇查询环或 `elaboration` 错误时 `luna lsp` 与 `serve packages-v1` 返回清晰诊断而不崩溃退出（P1）。
- **目标解耦与格式化等价**：对跨目标三元组输出进行产物头/对象格式断言而非只检查 `len() > 0`；对含复杂管道 `a + 1 |> f`、多属性与 `@macro{...}` 的源程序在 `luna fmt --check` 与 `verify.rs` 重解析 AST 对比中保证语义零飘移。
