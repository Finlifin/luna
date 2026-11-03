# 世界存储、句柄与生命周期

状态：设计，未实现；源码核对基线 `7178e238`。
本文细化[全链路架构](architecture.md)中的 WorldSnapshot 和 PublicationBundle，
与[发布状态协议](elaboration-state-and-publication.md)、[依赖观察](dependency-observations.md)
共同决定存储边界，不改变源语言可见性或声明身份。
增量语义依据为[正式查询契约](../docs/compilation/incremental-queries.md)。

## 1. 已有所有权保护不能因重构而丢失

下列路径均以 `compiler/` 为根。本轮只做静态审计，未运行 Miri、并发或内存压力验证。

| 入口 | 已存在的约束 | 需要进一步处理的问题 |
| --- | --- | --- |
| `hir/src/arena.rs::HirArena` | 分类型 arena、借用期返回节点、内部擦除 lifetime | arena 内引用来源、析构期间访问和跨快照复用需分别证明 |
| `middle/src/hir_package.rs::HirPackageBox` | 高阶生命周期构造回调、owner 借用期 accessor、包先于 arena 析构 | 不能直接将可变 arena 作为并发稳定世界；现有 compile-fail 约束应保留 |
| `hir/src/lib.rs::Package` | 节点借用与 owner/body/来源/解析表分离 | 克隆 Package 仍复制多张表，不等于全部按需增量存储 |
| `hir/src/definitions.rs::fork_package/PreparedPublication` | 复制预算、私有候选、无后续用户回调的安装 | 小批发布的全表复制、草稿 arena 残留与旧版本保活成本 |
| `middle/src/ty/interner.rs`、`ty/context.rs` | arena intern 和定义/节点侧表 | lifetime 擦除后再次查询的输入所有者约束，旧 ty 与新语义内核职责分离 |
| `symbol/src/lib.rs::Symbol` | 全局 intern 及同进程指针身份 | AST 将指针拆为两个 u32；不得进入持久协议，也不能假定其布局普适 |
| `comptime_interpretor/src/code_session.rs` | 强持有精确 source/IIR 世界，fresh VM，匹配后交换代码缓存 | 更细粒度复用需新的依赖证据，不能先移除精确快照检查 |
| `middle/src/queries.rs` | HIR 输入更换时失效查询；活动 provider 中拒绝输入替换 | 仍是包级产物，活动世界与稳定结果尚未全面分离 |
| `luna/src/compilation_cache.rs` | 有界整前端缓存、实际源码/发现重验、失败不回退旧结果 | 容量按条目限制不直接等于内存上界；不是跨进程格式 |

不能因为代码含 transmute 就宣称已经复现未定义行为，也不能把“分配地址稳定”当成完整
安全证明。旧表长期持有的引用，必须连同所有输入来源、后续查找与析构一起审计。
例如 `register_def_ty/register_node_ty` 接收 Ty 后擦除 lifetime，需验证所存 Ty 的 owner
确实由接收上下文保活；借用能够短暂重叠，不自动证明来源 owner 相同。

## 2. 目标物理结构

首选方案为**拥有数据的不可变分块、按路径复制的索引、单一发布根**。
先采用单线程写入 owner；不先引入无锁发布、共享可变 arena 或 unsafe Send/Sync。

```text
WorldRoot {
  session, revision,
  declarations: PersistentMap<DeclarationId, DefinitionRevision>,
  interfaces, bodies, implementation_index,
  provenance, symbol_chunks,
  chunks: PersistentMap<ChunkId, OwnedChunk>,
  admitted_contracts
}

OwnedChunk = shared owner of immutable owned node/slice arrays
NodeId<T> = session + chunk + slot + node kind
DefinitionRevision = declaration identity + checked facet references
```

节点内部存有类型的 ID/切片范围，不存指向其他 chunk 的裸 Rust 引用；其内容自身拥有
字符串、数组等载荷，或通过受检 ID 引用其他已保活分块。分块 owner 可以用 Arc，共享
发生在块/索引层，不要求每个小节点分配一个 Arc。

逻辑节点形状可以继续复用现有 HIR/NIR 设计，但边从借用改为受检引用。未完成接口前缀
仍由局部任务持有，不在 WorldRoot 中放 `Pending` 检查器或半完成推断变量。
稳定语法目录可以登记尚待检查的声明；其存在不签发已检查接口证据。

PersistentMap 采用按路径复制的有序树作为初始实现选择，公开 API 不暴露具体树类型。
叶块大小、扇出和批量插入策略按工作量测量；不必为此先引入大型依赖框架。
禁止将 `Arc<HashMap>` 包装后，每次写入触发整表 clone，却称为按需持久化。

每个新 root 持有其所需块及索引路径，不强引用上一整个 WorldRoot。
否则即便所有旧消费者已退出，最新 root 仍会保活整个历史链。
来源追踪通过稳定记录保持必要关系，不把活动旧 checker 一起保留。

## 3. 句柄的合法性与语义可见性

```text
lookup_node(reader, id):
    verify session and node kind
    locate an owned chunk in this root
    verify slot/slice bounds
    borrow the immutable payload for reader's lifetime

observe_declaration(view, declaration, facet):
    apply scope/instance/visibility rules
    locate the corresponding definition revision
    demand the checked facet and its evidence
```

两个入口不可混为一个。物理上能读取节点不表示该声明在当前语义位置可见；同一物理
root 内可以有不同 SemanticView。world revision、loose frontier 和最终 coherence
继续遵循[既定协议](elaboration-state-and-publication.md)。

NodeId 不包含“只能由创建时那个 root 使用”的固定 revision，否则不能共享未改节点。
它的 session/chunk/slot 身份由当前 root 的成员关系验证；语义查询另外验证所观察版本。
类型声明身份、节点存储身份和缓存键不互相代用。

不可变块在一个 session 中不重用 ChunkId；计数耗尽是受控失败，不 wrap 到旧 ID。
确需复用的局部 arena/task 槽用 owner/generation，类型上与稳定 NodeId 分开。
错误节点不能靠一个合法 slot 伪装为受检类型；形成证据仍有自己的构造入口。

可选择以下 Rust 接口形状，名称只是草案：

```rust
impl WorldRead<'_> {
    fn node<T>(&self, id: NodeId<T>) -> Result<NodeView<'_, T>, HandleError>;
}

impl DraftOwner {
    fn prepare(self, base: &WorldRead<'_>) -> Result<PreparedWorld, PrepareError>;
}
```

NodeView 的借用不能长于 reader；跨调用长期保存用 ID 加 owning snapshot 或 owning
chunk pin，不能把 view 擦为 static。需要发布活动进度时先退回稳定 ID，再保存 continuation。
实际 generic 约束及数据枚举在实现时确定，不先以此示意建立未接线的公共 crate。

## 4. 草稿、封块与批次发布

DraftOwner 独占可变节点缓冲、预留身份、解析映射与构造预算。DraftNodeId 与稳定 NodeId
属于不同类型；预留 forward reference 只在同一批次内解析，不能提前交给共享查询。
生成定义先完成结构及语义准备，再创建可安装根；批次不要求由某个特定 definition kind 组成。

prepare 执行：

1. 验证所有槽、子范围、kind、owner、跨块引用及来源，确认每个引用对象都会保活。
2. 完成对应普通检查和接收者准入，记录基线视图、facet 依赖与整体 coherence 的责任。
3. 将草稿缓冲封为不可变块，准备声明/接口/VM 描述/索引的路径增量。
4. 构造完整 PublicationBundle 和 root，预留通知及旧 root 回收记录。
5. 返回一次性 receipt，绑定草稿、基线及所有接收结构。

安装仅核对基线并切换根，通知随后入队。不能让一个消费者读新实现索引、另一个消费者
仍读旧 VM 描述；持有旧 root 的消费者可以继续按原语义视图工作。

**根切换不包含无界析构。** 替换下来的 root 转交预留好的 retired-owner 记录，之后按
明确回收策略处理；不能在安装点递归释放庞大历史，再把延迟和预算记成 O(1) 发布。
这并不承诺在进程崩溃或 allocator 无法满足底层分配时继续运行；所有可报告的容量和
预算失败必须在发布前处理，不能出现半份成功世界。

stale receipt 不安装任何增量。可保留冻结草稿继续准备，但实际未变化的证据才可复用，
不可重放计算继续使用原请求进度。修复失败草稿不会复用已经逃出的声明身份。
对 aborted batch 的节点，如果没有诊断或其他合法 owner 保留，应随草稿回收；不得每次
失败都永久追加到全局 arena。保留诊断可以只保存来源和必要描述，不强制保活全部草稿。

## 5. 声明修订与索引增量

一次定义可能有语法、接口、主体、属性、实现摘要等多个稳定 facet。它们各有版本和依赖，
不因同属于一个文件而被迫共用失效边界。DefinitionRevision 表达同一声明本次可观察的
facet 集合，更新时形成新记录，旧 reader 保留旧记录。

更新实现索引时，除命中桶还要更新受影响检索空间的代次。负查找读到了哪些范围也记录；
未知 Type 所需的兼容后代不能因索引优化漏掉。索引只是完整检索的加速，不能决定重叠语义。
多个表的路径修改都属于同一准备根；字段原地修改会破坏 snapshot 的保证。

声明、来源和块目录使用共享索引，避免每批复制所有已发布 owner。设已存在条目数 N，
本批更改条目数 k，初始有序树实现的路径更新目标为 O(k log N)，另计新载荷与必要检查。
完整 coherence、依赖重验和索引更新可能访问更多条目，不能据存储更新界限承诺发布总成本。

完整遍历、dump 或跨版本压缩本来就可能扫描全部相关数据，应独立计费。
不使用无限 overlay 链：若采用临时 delta，查询深度有界，合并成本显式记录，不能藏到
下一个看似便宜的 lookup 中。

## 6. 回收与长驻内存

| 所有者 | 持有内容 | 释放条件 |
| --- | --- | --- |
| 活动 region/query producer | 固定快照与本地任务状态 | 完成结果冻结，或失败/取消后所有请求退出 |
| VmSession | 代码、快照 pin、活动槽和请求 | 源执行结束及宿主请求责任完成 |
| CodeCache | 已准入 code 与所需稳定数据 | LRU/预算淘汰且无活动帧引用 |
| 结果缓存 | 稳定结果、证据、依赖与必要拥有者 | 失效或淘汰；不包含活动 VM 存储 |
| 诊断 | 来源快照/重映射依据与有限描述 | 消费者释放或转换为独立呈现结果 |
| 发布 owner | 当前根和待回收根 | 无 reader pin 后回收不再需要的块 |

缓存以条目数和估算/实测保留字节双重限制；共享块按明确所有权口径计费，不能多次相加
宣称实际峰值，也不能因共享就把它们全算成零。指标区分 live、cache-retained、retired
和未回收草稿，才能定位长期占用。

declaration 互递归用 ID 边，不用节点之间的强 Arc 环。root 负责保活闭包；查询缓存归
BuildSession，WorldRoot 不反向强持有该缓存。这样避免 result→world→cache→result 环。
跨块可达集合不能靠“最后一个外层 Arc 释放”猜测；必要时在安全点按 root 集合做块级
可达整理，旧 reader 未退出前不移动其节点或复用 ID。
当前根的块目录也可能保留已无定义引用的旧块；“没有父 root 指针”并不足以避免这类保留。
目录整理需要依据当前声明、代码、证据和来源的真实引用闭包生成后继根，工作量独立计费，
达到保留预算时安排回收或报告资源限制，不能无限推迟。

取消消费者不等于取消共享 producer。已经运行的不纯请求按执行 owner 收尾，不能为
释放快照而丢弃未知完成状态；但已取消代次的结果不能重新发布给复用槽位。
回收工作只释放编译器拥有的数据，不在析构中执行新的用户 comptime 程序。

## 7. Symbol、跨编辑定位与持久格式

目标 AST 存 SymbolId，文本由拥有的字符串分块提供；不再把 Intern<String> 的内部指针
拆进普通 child 数组。源符号相等仍遵循字符串/符号的既定语义，存储编号不是声明身份。
现有全局 interning 可在迁移期间保留，原始位编码的所有写入和读取成对替换。
不基于推测的第三方结构布局增加更多 transmute。

跨进程数据只写显式 schema、字符串/值内容和版本化引用。不能写进程地址、Symbol raw
parts、NodeId 的会话编号、活动 TypeRef owner 或 runtime 对象的上次执行实例。
反序列化先校验范围、kind、域、依赖闭包和预算，再建立本次 session 的映射。

跨编辑缓存定位使用包构建输入、模块/词法路径和局部定义辨认信息寻找候选结果；它不是
语言名义身份的 hash 定义。同名重新声明、宏/模板重复生成和不同包实例不能碰撞成一个身份。
映射有歧义就失效；缓存命中须重绑定整个依赖闭包并重验实际观察，不能仅重写顶层 DefId。

同一构建中更新声明主体的版本与新构建中的身份重建分别处理。文件移动或改名能否复用
某个工作，由有效身份映射和依赖证明决定，不承诺文本相似就复用。
损坏或过期缓存按受控 miss/诊断处理，不回退不匹配的成功产物。

## 8. Unsafe 审计与退役顺序

| 边界 | 必须检查的证明义务 | 目标处理 |
| --- | --- | --- |
| arena lifetime 擦除 | 每条内部引用的 owner、返回借用、drop 是否访问载荷 | 新持久结构用 owned chunks + ID；旧结构由明确 owner 隔离 |
| TyCtxt 侧表 | 输入 Ty 的来源保活、跨上下文登记、以后查询期 | 新内核不依赖旧 Ty；在消费者迁移后退役侧表 |
| HIR owning wrapper | 高阶回调不接纳外部短借用、析构顺序、可变别名 | 保留现有约束；转成 owned graph 后才移除擦除 |
| Symbol 位编码 | 写读配对、有效来源、布局假设、持久输入边界 | typed SymbolId 与显式字符串表 |
| 未来并发 | payload、索引、interner 与缓存各自线程安全 | sealed chunk 的自然 auto traits；不强制旧 arena Send/Sync |

源码里能找到外部 owner 混用的可能路径，意味着需要针对性安全核验，不等于本文已经
证明 exploit 或当前 CLI 必定触发。实现阶段要覆盖跨 owner 的拒绝、drop 后访问不可达、
panic/cancel 期间回收及需要时的 Miri 检查；本轮不冒充完成这些验证。

旧 middle::ty 仍由 Db 持有，interface/intrinsic 入口还有类型关联；不能因为旧 typeck/
mir/codegen 未加入 workspace 就直接删除 middle 整层。先迁移查询 owner、接口和 intrinsic
消费者，再移除不再承担职责的旧类型组件。compiler/hir、ast_lowering 与 middle 的实际
依赖也要同步调整，不能留下新结构仍经旧上下文回流的循环。

## 9. 实施切片和验收

首个切片应让一个已有源码定义与一批生成定义进入同一稳定目录：旧快照仍能 dump 和
编译旧 code，新快照能查询新定义；丢弃草稿不污染两者，释放新快照不破坏旧 reader。
同一切片迁移真正的解析/接口/VM 消费者，不能只验证手工构造的 OwnedChunk。

必须覆盖：foreign session、错误 kind、越界 slot/slice、旧 generation、未封草稿句柄、
forward reference、互递归、发布预算失败、stale receipt、通知前取消、最后 reader 释放、
诊断保活、代码缓存淘汰、来源文件变化和持久缓存损坏。成功/失败两侧都记录保留 owner。

规模验收同时改变 N、k、存活快照数、失败草稿数、节点边数和符号字节量。
分别测 prepare、根切换、通知、回收和真实下一次查询成本，防止把全表复制或析构搬到
计时之外。旧预算机制继续覆盖实际分配及遍历，不以“不可变”作为免费复制的理由。

dump 使用携带 WorldRead 的 Display 适配器，以稳定局部编号展示 root、chunk、声明、
facet、依赖和来源；字符串正确转义。调试地址可另作内部诊断，不成为 S-expression 的
身份格式或跨运行比较依据。

本文件选择存储和所有权方向，尚未完成具体节点枚举、Rust API、持久 schema 或性能验收。
没有修改源码、删除旧 arena 或扩大并发；迁移仍按[计划](refactoring-plan.md)推进。
