# 增量查询与诊断

## Intro

修改一个函数体，不应无条件重做所有调用者的类型检查；修改它的接口，也不能让调用者继续使用失效的结论。

增量查询记录一次结果真正依赖了什么。复用依据是这些输入和前提仍然成立，而不是某个文件名或缓存槽位碰巧没有变化。

## 语法

```text
function -> pure? comptime? inline? (unsafe | spec | verified | atomic)? fn id(parameter*) (-> result)? clauses? (block | (= expr))?
const_def -> const id (: expr)? = expr
```

源码没有与每个内部 query 对应的声明。编译器划分查询粒度，但必须保持函数、类型、阶段和构建实例的语义。

## 正式定义

### 可复用的结果

缓存复用必须恢复该结果所需的依赖、证据和相关诊断。冻结的结果不能引用旧会话的推断变量、waiter、continuation 或临时 IR handle。

类型、属性、实现环境或实际读取的构建输入变化后，依赖它们的结果须重新验证。build 默认指 host pipeline；目标库配置按显式读取关系参与依赖。

一个 query 包含不纯执行时，不能仅凭它有可比较的返回值就跳过源程序要求的调用。纯计算的复用也须符合输入身份和依赖契约。

### 身份与表示

声明身份、语言中的 runtime identity、内部节点编号和跨编辑的缓存定位分别承担责任。一个进程中的地址不能用作下次会话的稳定语义身份。

缓存定位允许在编辑后重新找到相应工作，但不会自行证明两个不同声明是同一个类型。重命名或文件移动后的复用需要身份与依赖重新核验，不能仅凭文本相似合并声明。

### 失败与循环

未求得的必需结果不能由旧缓存、空值或占位成功替代。失败结果可以参与诊断复用，但不得成为代码生成成功凭据。

合法的普通递归可依赖已经存在的签名。若一项具体计算必须先取得自身尚未产生的结果，则报告实际依赖环；不能把“正在处理”解释成已经证明。

## 常见示例

### 函数体变化与接口变化

```flurry
use std.prelude.*;

fn adjust(value: i32) -> i32 {
    value + 1
}

fn show_adjusted(value: i32) {
    println("{}", adjust(value));
}
```

将 adjust 的 1 改为 2 后，新的函数体必须进入代码生成。若调用者检查只依赖仍相同的接口，其类型检查结果可以复用；若后端曾内联该主体，相应生成结果仍须按主体依赖重新处理。

因此，“调用者类型检查不变”不等于“最终机器码不变”。若修改了参数、结果、推导契约或被实际读取的其他接口信息，则重新检查相应消费者。

### 配置是实际输入

```flurry
const TRACE_ENABLED: bool = build.mode matches .debug;

fn trace_enabled() -> bool {
    TRACE_ENABLED
}
```

改变所属构建实例的 mode 会影响 TRACE_ENABLED。缓存必须记录这项读取，不能从另一个包或另一个构建实例恢复同名常量。

### 值计算的环不能以占位打断

以下是独立拒绝例：

```flurry
const FIRST: u32 = SECOND + 1;
const SECOND: u32 = FIRST + 1;
```

取得 FIRST 需要 SECOND，取得 SECOND 又需要 FIRST。编译器应说明这条计算依赖，而不是在第二次访问 FIRST 时返回零并宣称两次初始化成功。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 编译器以 query 组织按需计算，并记录依赖以支持增量处理。本章采用的依赖要求还涵盖 Flurry 可冻结的残余计算与显式 runtime 依赖替换。[Rust queries](https://rustc-dev-guide.rust-lang.org/query.html)

C/C++ 工程常以翻译单元和构建依赖安排重编译；Zig 同样需要根据编译输入生成实例。源语言中的函数或文件边界都不等于每个编译器必须采用的最小缓存单位。Flurry 不把某一种物理缓存粒度写成程序合法性的条件。

## 运行时实现设计引导

最终程序使用本次构建的产物，不能因为调用者检查被复用就链接到不匹配的旧实现。类型接口、代码产物与运行时库版本的关联必须由构建交付保证。

缓存中的类型或证据描述不能包含上次程序执行的具体对象。共享模板在本次环境重新绑定，实际对象仍由生成程序按参数和捕获关系取得。

## 编译器实现引导

### 三类结构

| 结构 | 责任 |
|---|---|
| AST/HIR 包含关系 | 描述源构造及语义结构 |
| 局部 goal 依赖 | 推动本次推断区域内受阻的工作 |
| 持久 query 依赖 | 验证稳定结果能否复用 |

局部字段读取不必成为全局 query，每个表达式也不必单独持久化。可从定义接口、主体检查、具体实例等有稳定输入输出的边界起步，再根据实际测量细分。

若一个大结果的某个字段没有变化，要单独复用依赖该字段的消费者，就需要可追踪的投影或独立结果边界。只给整个结果计算一个指纹，不会自动获得逐字段增量。

### 查询协议

```text
query(key):
    if an entry is available:
        validate every relevant dependency and premise
        if still valid:
            restore result and required diagnostics/artifacts
            return result
    if this request forms a real unresolved result cycle:
        report the dependency path
    result = compute in a fresh local region
    require a valid frozen boundary before publication
    record observed dependencies and result fingerprint
    publish success or failure with its evidence
```

依赖应涵盖实际读取的签名、实现候选世界、phase、类型替换、属性、构建配置与目标输入。索引返回“当前没有候选”时也依赖相应实现集合的版本，不能只记录命中项。

相同 key 的并发请求可以共享生产者，但要分别管理消费者取消与整个生产任务的归属。一个消费者退出不能使其他合法消费者得到半份结果。

### 诊断及错误恢复

诊断保留原定义、当前调用、生成代码来源及必要依赖路径。去重依据逻辑错误及其有效上下文，不按输出时序决定是否保留。

错误占位可以帮助编辑器分析不依赖该错误的其他位置；它必须传播失败标志，不能作为正确类型、实现证据或完整 IIR 输入。恢复缓存诊断时重新对应当前来源，避免把旧文本位置当成本次错误位置。

### 验收

覆盖无编辑复用、仅改主体、接口变化、实现新增、构建配置变化、失败后修复、产物缺失和取消后重试。记录重新计算的查询及原因，确认缓存命中确实恢复了所需内容。

对固定依赖图，失效传播可以按访问的节点和边计费；重新计算的求解与执行成本另计。源码变更、候选数及实例数会改变图规模，不以固定图复杂度承诺整个增量构建耗时。

### 包级增量构建与并行编译（Package Snapshot / Sysroot Cache & Parallel Waves）

在不把 `elaboration::Checker` 一次性改写为函数级持久化查询前，编译器按**闭上游包快照 + DAG/SCC 并行波次**推进包级增量与并行构建：

1. **闭上游分区与 Tarjan SCC 波次调度**：
   - **闭上游分区**（`partition_closed_upstream`）：从入口包 `primary` 出发沿反向依赖边标记下游集合 `D`（包含 `primary` 及直接/间接依赖它的包）；其余包构成向上闭合集合 `B`（如 6 个 sysroot 内置包 `{builtin, std, meta, build, attrs, verification}` 或未改动的第三方/工作区依赖），`B` 中没有任何包依赖 `D`。
   - **SCC 缩点与波次并行**（`schedule_package_waves`）：使用 Tarjan 强连通分量把存在相互可见环的包集合并（例如 6 个 sysroot 包组成一个闭环单元 `PackageUnit`），再按无环 DAG 拓扑分层为 `Wave 0, Wave 1, ...`。同一波次内彼此独立的 `PackageUnit` 可并发执行；前一轮波次任一单元失败即中止，不启动后续波次，保证并发调度下错误报告与输出顺序完全确定。
2. **内容寻址快照键（Merkle `PackageSnapshotKey`）**：
   - 使用确定性 SHA-256 对格式化版本、编译器二进制指纹、上游 SCC/包拓扑、各上游包的规范目录树与 `.fl` 原文（支持按包并行哈希并按规范顺序折叠，线程数变化不改变键字节）、外部绑定配置文件（如 `language-items.conf`）、目标平台 `TargetData`、仅过滤到上游包语义身份的 `build.mode` / `build.target` / `build.package_metadata`、协议选择器以及直接依赖单元的 Merkle 键共同摘要。
   - 修改下游用户源码或仅更改用户包的 `build.mode` 不会使闭上游快照键失效；修改任一上游源码字节、新增/删除上游文件、更改目标平台、上游 `build.mode` 或编译器二进制本身会立即使对应上游快照键失效。
3. **两级缓存与并行验证**：
   - 进程内线程安全有界 LRU 缓存 `MemoryPackageCache` 与原子落盘 `.flpkg` 文件缓存 `DiskPackageCache`（含魔数 `FLPK`、版本、`ArtifactKind`、完整 256 位键校验、负载长度与 SHA-256 校验和，写入采用同目录临时文件 + 原子 rename；损坏/截断/版本不匹配时安全退回缓存未命中并重建）。
   - `program_verification` 在 `flow` 与 `contracts` 阶段分别提供 `VerifiedUpstream` 增量跳过门禁（跳过已冻结上游函数体、impl 关联初始化程序与契约继承证明，同时保持完整上游签名、前提/后置条件对下游可见），并在 `flow::validate_machine_owners` 中支持多核确定性并行验证（`LUNA_JOBS` 控制并行度，始终按声明顺序返回首个诊断）。

#### 当前实现分层状态

- **已落地（Layer 1 & Layer 2）**：`program_verification` 增量上游跳过门禁与多核确定性并行 `flow` 验证；`luna::package_cache` 的 SHA-256 指纹引擎、并行目录/文件哈希、闭上游分区与 Tarjan SCC 并行波次调度器、两级 `.flpkg` / 内存快照缓存引擎。
- **已落地（Layer 3A：源码加载/解析并行化 + `ParsedPackage` AST / `VerifiedUpstream` 跨进程增量缓存）**：
  - `ast` 支持 `NodeKind::from_u32` 与 `Ast::rebase`，`package_cache` 提供带长度/结构/checksum 校验的 `ParsedPackage`（`.flpkg`，带字符串表重驻留与基于 `BytePos(1)` 的相对 span）及 `VerifiedUpstream` 跨进程证书编码。
  - `luna` 驱动接入 `--cache-dir <path>`、`--no-cache`（及 `LUNA_NO_CACHE`）、`-j/--jobs <count>`（及 `LUNA_JOBS`），实现全包源码发现/读取并行化、无宏上游包两级 AST 快照复用、非宏文件多线程解析，以及 `contracts::inherit_incremental_skip_proofs` / `purity::validate_runtime_promises_incremental` / `flow::validate_machine_operations_with_jobs` 增量验证接线（已核验上游包跳过重证但完整保留继承契约、下游及动态生成定义仍全量检查）。
- **已落地（Layer 3B-1：规范上游优先序 + 进程内 `ElaboratedBase` 函数体缓存）**：
  - `luna::packages` 对非单文件 AST 请求按闭上游升序排列包并在 `HirQueryInput::prefix_files` / `ast_lowering` 中先于下游 lowering，保证闭上游声明身份跨下游增删保持不变；`elaboration` 在上游跳过闭上游-闭上游 formation 重复证明，并对无副作用上游函数体提供 `CachedUpstreamBodies` 复用（经 `global_elaborated_cache()` 按 `UpstreamKey` 存储，下游编辑后输出 NIR 与 `--no-cache` 逐字节一致）。
- **待推进（Layer 3B-2：`.flpkg` `ResolvedBase` / `NirBase` 磁盘持久化）**：在 `luna::package_cache` 实现跨进程 `.flpkg` 符号表与 NIR 编解码，进一步在 CLI 冷启动场景跳过未改动上游 sysroot 的 `resolve` / `ast_lowering` / `elaboration`。
