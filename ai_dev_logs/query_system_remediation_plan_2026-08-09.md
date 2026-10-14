# Luna Query System 整改方案

> 调查日期：2026-08-09
> 范围：`compiler/query`、`compiler/middle`、`compiler/interface`、`compiler/luna`，以及它们与 AST lowering 的连接点。
> 结论：当前系统不宜继续按现有模式扩展。应停止新增 provider/caches 字段，先收敛为**单一、单线程、显式输入版本**的本地 query 引擎；在前端和类型检查稳定前，不做 Salsa 全量迁移，也不承诺细粒度增量编译。

## 一、现状与证据

当前实现把三种模型叠在一起：

1. `salsa::Storage<Db>` 和 `#[salsa::db]` 仅提供 `Database` 宿主；没有声明 Salsa input 或 tracked query。
2. `query::QueryCache`、`try_get_with` 和 `QueryEngine` 实际承担 memo、调用栈和循环检测。
3. `middle::Providers` 用函数指针做运行时分发，driver 在运行前注册 provider，并把本次编译的资料写入 `Db`。

因此开发者看到的是 Salsa 风格的 `Db`，实际需要操作的是手工缓存和 provider 表。两套抽象都不完整，且边界不清。

| 问题 | 证据 | 直接后果 |
| --- | --- | --- |
| Salsa 没有提供实际 query 能力 | `compiler/query/src/lib.rs` 只 re-export Salsa；`compiler/middle/src/queries.rs` 中缓存由 `force_get_with` 管理 | 引入了 `Db: Sync` 约束和复杂度，却没有自动依赖追踪或失效 |
| 新增一个 query 需要跨五处修改 | `compiler/query/src/howto.rs` 明确要求 provider slot、cache、`LunaDatabase`、注册、可选 interface 转发 | 改动机械、遗漏风险高，query 数量一增长就阻碍正常开发 |
| 生产系统仅有一个 query | `Providers`、`QueryCaches` 和 `LunaDatabase` 都只定义了 `hir_package` | 管线仍是命令式，后续 typeck/MIR 无法自然按需组合 |
| 输入不属于 key，且无失效 | `Db::set_hir_input` 只覆盖 `hir_input`；`hir_package` 使用 `()` 作 key；其缓存不清除 | 同一 `Db` 先 lower A、再设入 B，会继续返回 A 的 HIR。这会使守护进程、IDE 和多文件编译得到陈旧结果 |
| “dep graph” 并不存在 | `DepNode` / `QueryHooks` 只有 start/end/read 回调，没有保存调用边或 revision | 无法做传递失效、验证或解释“为什么重算” |
| 语言错误与引擎错误都变成 panic | `force_get_with` 对 cycle/poison panic；provider 默认值也 panic | 用户可写出的 import/type cycle 没有可恢复诊断路径，失败后无法继续编译其他项 |
| `Poisoned` 不能恢复 | `QueryCache` 存储 `Poisoned`，而 `Db` 没有在输入变更时清除缓存 | 一次 provider panic 会污染当前 session 的同一 key |
| 为适配未使用的 Salsa 约束出现不安全代码 | `QueryEngine`、`QueryCache`、`Db`、`HirQueryInput`、`HirPackageBox` 都有 `unsafe impl Send/Sync` 或寿命擦除 | `RefCell` 包装的对象一旦经 `Arc` 跨线程使用可能造成未定义行为；缓存输入还持有 erased raw pointer |
| query 边界没有覆盖真正的编译阶段 | `compiler/luna/src/main.rs` 直接执行 lex、parse、resolve，只有 lowering 走 query | 缓存不能覆盖最常重复、最适合作为依赖节点的前端阶段 |
| 测试覆盖只验证引擎小样例 | `compiler/query/tests/query_engine.rs` 有 4 个基础测试 | 没有 `Db` 输入替换、传递失效、诊断恢复、真实 pipeline 或多文件用例 |

`cargo test -p query` 通过（4 个测试），`cargo check -p luna` 通过；后者同时报告 `HirQueryInput` 中的 raw-pointer dereference 警告。它们说明当前单次 happy path 可编译，并不能证明 query 可安全扩展或可增量复用。

## 二、目标与非目标

### 目标

- 新 query 的公共声明、缓存、key、计算函数和调用入口应位于一个清晰位置；不再要求 provider 表注册。
- 所有会影响结果的数据都必须是带 revision 的显式 input，或出现在 query key 中。
- 查询可以调用查询，自动记录依赖；用户可触发的循环返回诊断值，不直接中止进程。
- 当前阶段保持单线程，移除仅为满足 Salsa 而存在的 `unsafe Send/Sync`。
- 前端、解析、解析名称、lowering、类型检查、MIR 形成可观测的 query DAG。
- 缓存值与诊断不携带借用 session 的 raw pointer。

### 非目标

- 本轮不实现磁盘持久化 cache、跨进程增量或并行 query 执行。
- 本轮不重写整个 HIR/type interner；只规定 query value 必须是可拥有、可缓存的句柄或 `Arc`。
- 本轮不把“每次输入变更只重算最小节点”作为前置条件。先保证失效正确，再优化粒度。

## 三、架构决策

### 选择：短中期采用本地 query 引擎，移除闲置 Salsa 壳

推荐保留现有项目已经开始使用的 rustc 风格，但改为明确且一致的本地实现。理由如下：

- 当前代码没有使用 Salsa 的 input/tracked 机制。继续保留它不会减少任何新增 query 的样板代码。
- 现有 HIR 与 type context 广泛使用 arena、`RefCell` 和寿命擦除。直接全量迁移到要求 `Send + Sync` 的数据库模型，会先放大内存模型和诊断模型问题，阻塞 Phase 1。
- Luna 暂时按一次编译创建一个 `CompilerInstance`，正确的全量失效已足以解开当前前端、typeck、MIR 的依赖。
- 自建引擎的范围必须严格限制在：typed cache、调用栈、依赖记录、revision 验证、诊断化 cycle recovery。不要在这个阶段复制一个通用数据库框架。

### 被拒绝的方案

**维持手工 provider 表并继续添加 cache 字段。** 改动最小，但每个 query 都要增加新的全局可变输入和注册顺序，已经是当前阻塞本身。

**立即完整使用 Salsa。** 长期可重新评估，尤其在需要 IDE、并行或成熟增量时；但这要求先将 HIR/value/diagnostic 生命周期改造成能满足 Salsa 的线程与所有权边界。它不应与 typeck/MIR 首次接入同时进行。

若未来决定迁移 Salsa，应以本方案中的 query 契约、owned value 和显式 input 为前置条件。届时可替换存储实现，而不改变前端 query 的语义接口。

## 四、目标模型

### 4.1 输入、key 与结果

`Db` 只保存可拥有的编译输入和查询状态。每次 `set_*_input` 都增加 revision。输入不能再通过全局 `Option<HirQueryInput>` 或 raw pointer 注入。

```rust
pub struct FileInput {
    pub path: Arc<PathBuf>,
    pub text: Arc<str>,
    pub revision: Revision,
}

pub struct QueryOutput<T> {
    pub value: T,
    pub diagnostics: Arc<[Diagnostic]>,
}

pub trait Query: 'static {
    type Key: Clone + Eq + Hash + Debug;
    type Value: Clone + 'static;

    const NAME: &'static str;
    fn compute(cx: &QueryContext<'_>, key: Self::Key)
        -> QueryResult<Self::Value>;
    fn recover_cycle(cycle: &QueryCycle) -> QueryOutput<Self::Value>;
}
```

这里的 `QueryResult` 指的是可恢复的编译结果，不是当前 `QueryResult::Poisoned`。内部 panic 由顶层 driver 捕获并报 ICE；它不得永久写入正常 cache。所有用户错误均以 error node/error type 加 diagnostics 的方式缓存。

推荐的首批 query 合同如下：

| Query | Key | Value | 直接依赖 |
| --- | --- | --- | --- |
| `lex_file` | `FileId` | `Arc<LexedFile>` | `file_text` |
| `parse_file` | `FileId` | `Arc<ParsedFile>` | `lex_file` |
| `package_files` | `PackageId` | `Arc<[FileId]>` | package manifest/config |
| `module_tree` | `PackageId` | `Arc<ModuleTree>` | `parse_file`（包内每个文件） |
| `lower_file` | `FileId` | `Arc<LoweredFile>` | `parse_file`、`module_tree` |
| `hir_package` | `PackageId` | `Arc<HirPackage>` | `package_files`、`lower_file` |
| `item_signature` | `DefId` | `QueryOutput<TyId>` | `hir_package`、名称解析 |
| `typeck_body` | `BodyId` | `Arc<TypeckResults>` | `item_signature`、`resolve_body` |
| `mir_body` | `InstanceId` | `Arc<mir::Body>` | `typeck_body` |
| `codegen_unit` | `CodegenUnitId` | `Arc<ObjectFile>` | `mir_body`、target options |

约束：`()` 只允许表示真正无参数且全局唯一的输入；不能用作“当前文件”或“当前包”的隐式替代。`Ty<'tcx>` 等带内部借用的实体不能作为跨 query 的公开 value；改为 `TyId`、`DefId`、`BodyId` 或拥有它们的 `Arc` 容器。

### 4.2 计算上下文与服务边界

`SourceMap`、诊断 emitter 和命令行环境是本次运行的服务，不是 query input，也不能进入 cache value。

```text
CompilerInstance
  ├─ CompilerServices: SourceMap、诊断收集器、文件加载器
  └─ Db: revisioned inputs、query tables、dependency graph

Compiler::query(...)
  └─ QueryContext { db, services, active stack }
       └─ 读取 input / 调用子 query / 收集 Diagnostic
```

短期兼容阶段可以把 `&SourceMap` 作为仅在 cache miss 时存在的 `QueryContext` 服务参数；不得把它的裸指针、`DiagnosticContext` 的裸指针或借用写入 `Db`、`QueryInput`、cache value。中期应让 parse/lower 返回可重放的 `Diagnostic` 数据，由 driver 负责 emit 和去重。

### 4.3 依赖与失效

每个 cache entry 保存：

```text
Entry {
  state: Vacant | Computing | Ready { value, diagnostics },
  computed_at: Revision,
  verified_at: Revision,
  dependencies: Vec<DepNode>,
}
```

当一个 query 读取 input 或调用子 query 时，`QueryContext` 把依赖边记录到当前 frame。调用栈在 `Drop` 中无条件弹出；异常时条目退回 `Vacant`，而不是 `Poisoned`。

实施分两步：

1. **正确性优先（Phase 1）**：任一输入 revision 变化时使所有 query entry 失效。这样实现最小，但保证没有陈旧值。
2. **性能优化（Phase 2）**：用记录的反向依赖边做传递失效，或按 `verified_at` 验证直接依赖。只有已有重编译 trace 的瓶颈才值得做细分。

循环检测使用 `(query kind, key)` 的完整 identity。`QueryCycle` 应只包含形成环的栈片段，而不是从根开始的全部 frame，并应支持 query 指定的 recovery。比如 import/path/type alias cycle 产生语言诊断和 `Error` value；编译器不变量被破坏才报告 ICE。

### 4.4 声明和调用方式

移除 `Providers`、`set_providers`、`QueryCaches` 手工字段和 `LunaDatabase` 中每个 query 的转发。建立一个 query catalog 宏或一个集中声明模块，为每个 query 生成类型化 accessor 和其 table：

```rust
define_query! {
    pub query ParseFile(FileId) -> Arc<ParsedFile> {
        name: "parse_file",
        compute: frontend::parse_file,
        cycle: frontend::recover_parse_cycle,
    }
}

let parsed = cx.parse_file(file_id)?;
```

宏只生成重复样板；计算函数仍是普通 Rust 函数，便于跳转、测试和重构。query 的定义与调用 API 放在 `interface::queries`（或新建依赖 `middle` 和各 pass 的 `compiler/pipeline` crate）中，避免 `middle -> ast_lowering -> middle` 循环。`compiler/luna` 只做 CLI、创建 `CompilerInstance`、设置 inputs、请求最终目标。

## 五、迁移计划

### M0：冻结并建立回归测试（0.5-1 天）

- 暂停在当前 `Providers`/`QueryCaches` 模式新增 query。
- 为现有 `hir_package` 加回归测试：同一 `Db` 更换 input 后不得返回旧的 package；未注册 provider、cycle、panic 后重试均要有明确行为。
- 将 `cargo test -p query`、`cargo test -p middle`、`cargo check -p luna` 写入本地验收命令。`cargo test --workspace` 等稳定后再提升为门禁。
- 在 query 调用入口输出可开关 trace：query 名、key、cache hit/miss、revision、耗时。没有 trace 不进入细粒度失效阶段。

### M1：剥离错误的基础依赖（1-2 天）

- 从 `query`、`middle`、`interface` 的 query 路径移除 Salsa re-export、`Storage`、`#[salsa::db]` 和 `LunaDbBase`。
- 删除仅因其 `Sync` 约束而出现的 `unsafe impl Sync`；单线程性质通过 `Db: !Sync` 自然表达，而不是注释承诺。
- 保留但改写 `QueryCache` 和 active stack；它们不再公开可绕过的 `clear`，只由 revision/invalidation API 管理。
- `HirQueryInput` 不再存 raw pointer。先把 file/package identity、AST/module data 改成拥有的 input；服务引用仅在 `QueryContext` 调用期间传递。

这一步完成后的系统可以尚未支持增量，但必须不再有“输入替换返回旧 cache”的语义漏洞。

### M2：引入统一 query API（2-4 天）

- 在 `compiler/query` 增加 `Revision`、`QueryKey`、`QueryContext`、`QueryCycle`、typed query table 和 declaration macro。
- 在 `Db` 只保留一个 `QueryStore`，由 catalog 统一定义的 table 管理；不再为每个 query 改 `Db` struct、provider 表和 extension trait。
- 将 `hir_package` 改为 `hir_package(PackageId)`，并拆出 `parse_file(FileId)`、`module_tree(PackageId)` 和 `lower_file(FileId)`。
- 计算实现转移到 `interface::queries` 或 `compiler/pipeline`，删除启动时 `set_providers` 的顺序要求。
- 所有 query 返回 `QueryOutput<T>` 或等价的“value + diagnostics”对象；针对预期语言错误提供 recovery value。

### M3：接入 Phase 1 编译 DAG（3-6 天）

- 将 driver 中直接的 lex/parse/resolve 移入 query；driver 只设置 `file_text` / package / options input。
- 以 `DefId`、`BodyId`、`InstanceId` 为 key 接入 `item_signature`、`typeck_body`、`mir_body`；禁止新的“当前 package”全局输入。
- 将 source-to-HIR 的输出调整为能被安全缓存的 owned handle。`HirPackageBox` 的寿命擦除与 `unsafe Send/Sync` 作为单独清理项：先消除跨线程伪承诺，再逐步改为 index/arena-owned HIR。
- 每一个新增 query 都有 unit test、错误恢复测试和最少一个端到端 fixture。

### M4：验证后做细粒度失效（2-5 天，按 profile 决定）

- 启用依赖记录与反向边，先验证 `parse_file` 的文件级失效。
- 多文件修改时，未变文件应 cache hit；依赖被改变的 module/typeck/MIR 必须重算。
- 为 trace 增加 `why_recomputed`：输入 revision、失效依赖或 cycle recovery。
- 在这一阶段才评估是否需要将统一 query 契约移植到 Salsa。

## 六、验收标准

### 正确性

- 在同一 `CompilerInstance` 修改一个 `FileId` 的文本后，相关 query 不得返回旧版本结果。
- 修改文件 A 不影响无依赖的文件 B；A 的模块、签名和 body 类型检查按依赖重算。
- import、type alias、trait/约束等预期循环均产出稳定诊断和 error value，编译器可继续处理其他文件。
- provider/缓存初始化顺序不再是可观察 API；调用 `compiler.parse_file(file)` 即可工作。
- 正常语言错误不会导致 cache 永久 `Poisoned`；内部 panic 清理 stack 和 computing 标记，下一次请求可重试。

### 安全与可维护性

- query 路径不再含 `unsafe impl Send/Sync`、寿命擦除的 session pointer 或裸 `SourceMap`/`DiagnosticContext` pointer。
- 新增 query 的改动集中在一个声明和一个 compute 实现，不再跨 `Providers`、`QueryCaches`、`LunaDatabase`、注册点、interface 多点同步。
- `rg 'salsa::|unsafe impl (Send|Sync)' compiler/query compiler/middle compiler/interface` 的结果不再包含因 query 引擎产生的条目。
- 文档中的架构图、实际代码和日志 trace 一致：没有“使用 Salsa memoisation”之类与实现不符的描述。

### 测试矩阵

| 类别 | 必须覆盖 |
| --- | --- |
| 引擎单元测试 | hit/miss、不同 key、revision 失效、依赖记录、传递失效、cycle、panic cleanup 后 retry |
| 前端集成测试 | 两文件包的 parse/resolve/lower；只改一个文件时的 trace 与输出 |
| 诊断测试 | 重复查询不重复 emit；输入修复后旧诊断消失；cycle 不终止其他 query |
| 编译端到端 | 现有 `test.fl`，再加跨文件调用、泛型签名、type error 与 MIR fixture |
| 工具检查 | `cargo test -p query`、`cargo test -p middle`、`cargo check -p luna`，最后 `cargo test --workspace` |

## 七、风险控制

- 不把“依赖图完善”与“首次 typeck/MIR query 化”绑定。M2 的全量失效保证正确性，M4 才用 profile 驱动优化。
- 不在 query 引擎中隐藏全局可变状态。配置、sysroot、target、feature flag 都必须成为 input 并参与 revision。
- 不缓存引用诊断 emitter、session 或临时 AST borrow 的结果。缓存的 value 用 ID、索引、owned struct 或 `Arc` 表示。
- `HirPackageBox` 的 self-referential/寿命擦除设计不是由 query 引擎独自造成，但它当前被 cache 放大了影响面；重构时先把该边界与 query 的线程边界分开处理。
- 在 M1 前不要试图通过继续添加 `unsafe impl` 让更多 query “编过”。这会把 API 摩擦转换为难以定位的内存安全风险。

## 八、建议的立即执行项

1. 合入 M0 的回归测试，特别是“set input 后 cache 仍返回旧 `hir_package`”的失败用例，作为修复基线。
2. 确认采用本方案的本地引擎路线后，优先完成 M1，删除 Salsa 壳和 query 相关的伪 `Sync`。
3. 以 `parse_file(FileId)` 和 `module_tree(PackageId)` 作为 M2 的样板；它们比直接把 typeck 塞进现有 provider 表更能验证输入、依赖、诊断和失效模型。
4. 只有 M2 的 trace 和多文件集成测试稳定后，再让 typeck/MIR/codegen 成为 query。

