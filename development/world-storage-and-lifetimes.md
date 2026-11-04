# 世界存储、句柄与生命周期

状态：整体目标仍是设计；required 计算的声明快照所有权已按第 1.1 节接入生产。
初始源码审计基线为 `7178e238`。
本文细化[全链路架构](architecture.md)中的 WorldSnapshot 和 PublicationBundle，
与[发布状态协议](elaboration-state-and-publication.md)、[依赖观察](dependency-observations.md)
共同决定存储边界，不改变源语言可见性或声明身份。
增量语义依据为[正式查询契约](../docs/compilation/incremental-queries.md)。

## 1. 已有所有权保护不能因重构而丢失

下列路径均以 `compiler/` 为根，记录初始静态审计。旧 middle::ty 已在源码清理中删除，
第 1.1 节以外的世界存储迁移仍待实施；未运行 Miri、并发或内存压力验证。

| 入口 | 已存在的约束 | 需要进一步处理的问题 |
| --- | --- | --- |
| `hir/src/arena.rs::HirArena` | 分类型 arena、借用期返回节点、内部擦除 lifetime | arena 内引用来源、析构期间访问和跨快照复用需分别证明 |
| `middle/src/hir_package.rs::HirPackageBox` | 高阶生命周期构造回调、owner 借用期 accessor、包先于 arena 析构 | 不能直接将可变 arena 作为并发稳定世界；现有 compile-fail 约束应保留 |
| `hir/src/lib.rs::Package` | 节点借用与 owner/body/来源/解析表分离 | 克隆 Package 仍复制多张表，不等于全部按需增量存储 |
| `hir/src/definitions.rs::fork_package/PreparedPublication` | 复制预算、私有候选、无后续用户回调的安装 | 小批发布的全表复制、草稿 arena 残留与旧版本保活成本 |
| 已退役的 `middle/src/ty`（可从 `20af1c41` 恢复） | 原 arena intern 和定义/节点侧表 | 无生产消费者的旧体系已整体移除，未迁入新语义内核 |
| `symbol/src/lib.rs::Symbol` | 全局 intern 及同进程指针身份 | AST 将指针拆为两个 u32；不得进入持久协议，也不能假定其布局普适 |
| `comptime_interpretor/src/code_session.rs` | 强持有精确 source/IIR 世界，fresh VM，匹配后交换代码缓存 | 更细粒度复用需新的依赖证据，不能先移除精确快照检查 |
| `middle/src/queries.rs` | HIR 输入更换时失效查询；活动 provider 中拒绝输入替换 | 仍是包级产物，活动世界与稳定结果尚未全面分离 |
| `luna/src/compilation_cache.rs` | 有界整前端缓存、实际源码/发现重验、失败不回退旧结果 | 容量按条目限制不直接等于内存上界；不是跨进程格式 |

不能因为代码含 transmute 就宣称已经复现未定义行为，也不能把“分配地址稳定”当成完整
安全证明。旧表长期持有的引用，必须连同所有输入来源、后续查找与析构一起审计。
例如 `register_def_ty/register_node_ty` 接收 Ty 后擦除 lifetime，需验证所存 Ty 的 owner
确实由接收上下文保活；借用能够短暂重叠，不自动证明来源 owner 相同。

### 1.1 Required 计算的共享声明快照

`compiler/elaboration/src/program/required/prepared.rs` 的 RequiredWorld 把已检查的
NIR Package 与从它构造的 implementation Index 放在同一 owner 中。构造函数消费
package，索引建立成功才返回 Arc；字段保持模块私有，生产入口不能分别替换声明
或索引。它是准备计算的声明世界，不是第 10 节尚待实现的待发布 PreparedWorld。

PreparedRequired 持有这个 owner，其输入赋值、scope substitution、调用前提及
context 仍属于具体准备结果。fork 共享声明世界，复制本次调用的供应状态；不再
深拷贝 package 或重新建立索引。派生过程中没有声明检查或执行，不需要诊断 span，
也不再用 Result 暗示这里会重新准备代码。索引建立失败仍在最初的准备入口报告。

普通 invocation 从自身 owner 取得 package 与索引，不能由调用者另传一个世界。
builder 发布也以这同一快照建立发布服务，服务和 invocation 的借用期间保活 owner。
已有 definition identity cursor 检查通过只读 accessor 取得原快照游标；调用者产生
了更新的定义时，仍按既有路径重新准备，不向旧快照补入当前声明或活动推断状态。

资源权限保存另一份 RequiredWorld，包含资源声明及由它们建立的索引。原先 Copy
权限查询把资源 package 的 implementations／traits 与可执行 package 的 index
拼在一起；当可执行声明闭包没有保留某个 runtime Copy implementation 时，权限
查询会漏掉资源世界已经具有的实现。现在初次准备同时完成资源世界构造，fork 共享
它，Copy 查询从这个 owner 取得 package 和 index。类型前提与关联值查询也统一
通过 RequiredWorld::capabilities 取得借用视图，调用点不再手工配对这些字段。

调用前提与类型 context 仍按本次 invocation 传入借用视图；它们不是资源快照中的
全局事实。资源 dump 展开自身的 index 和 package，便于核对权限来源。新增回归
检查同一 Copy 实现仅存在于资源世界时，两份世界各自给出正确分类、其他 subject
不取得该权限、资源不足后可重试，以及派生调用共享同一资源 owner。

共享声明不缓存计算结果。每次 invocation 仍按自己的输入、预算与 live graph 执行；
输入赋值失败不会污染其他派生调用。原准备结果释放后，未完成的派生调用继续保活
快照，最后一个 owner 释放后 package 与索引一起回收。S-expression dump 继续展开
实际 package、index 和调用供应，输出不依赖 Arc 地址。

回归覆盖相同 owner／索引共享、失败输入路由的原子性、派生调用不同输入的独立结果、
不同快照中同号声明的不同结果，以及 Weak 观察的最后引用释放。原有 live closure
连续调用和 builder 公开接口检查继续覆盖实际执行。测试夹具只在唯一 owner 时允许
修改函数片段；生产没有可变访问器或 Arc::make_mut 旁路。

当前仍是单份完整 package 的共享所有权。初次准备分别收集可执行与资源声明并建立各自索引，输入供应
与上下文仍会复制；没有完成分块存储、跨编辑增量索引、稳定句柄或通用发布事务。
不以此宣称 M1 的完整 WorldSnapshot 已实现，也没有测量整体编译性能收益。

本切片的检查证据（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib
  prepared --no-fail-fast`：16 通过、157 filtered，退出 0；
  `/tmp/luna-tools-vbh4hq58/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna
  -p program_verification -p type_resolution --test prepared_invocations
  --test invocation_value_templates --test staged_invocation_reuse
  --test sysroot_preludes --test comptime_pointer_phase --test comptime_storage_domains
  --test associated_resources --test capabilities --no-fail-fast`：
  91 通过，退出 0；`/tmp/luna-tools-risp403w/result.json`。

- `python3 -B development/check.py --timeout 900 -- test -p luna
  --test builder_invocations --no-fail-fast`：85 通过、1 失败，退出 101；
  `/tmp/luna-tools-n6n4xgus/result.json`。失败仍为
  enum_builder_edits_payloads_groups_and_runtime_patterns 在 HIR lowering
  找不到 Symbol；此前以 `b2d402ae` 源码进行的精确对照也有同一失败，证据为
  `/tmp/luna-tools-tnid5emr/result.json`。没有修改该 fixture 或其预期。
- 修改源码的 rustfmt check 退出 0；`/tmp/luna-tools-qgovoepb/result.json`。

未执行全 workspace 或性能基准；没有将既有 builder 失败计为通过。

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

借用接口采用以下形状；名称是设计名，尚未加入源码：

```rust
impl<'world> WorldRead<'world> {
    fn node<'read, K: NodeKind>(
        &'read self,
        id: NodeId<K>,
    ) -> Result<NodeView<'read, K>, HandleError>;
}

impl<K: NodeKind> PinnedNode<K> {
    fn read(&self) -> WorldRead<'_>;
    fn id(&self) -> NodeId<K>;
}
```

WorldRead 借用其 root；NodeView 借用不长于 reader，不复制底层载荷，也不将临时查表结果
的借用延长。NodeKind 是 sealed 的内部分类，关联载荷必须拥有其数据或保存受检 ID，
不包含擦除生命周期的外部借用。节点表以受检枚举匹配或分类型表取出载荷，不靠根据
泛型参数进行未检查指针转换。NodeId 的构造字段私有；kind、session、边界仍在查找时验证。

跨调用长期保存默认使用 PinnedNode：它持有 NodeId 和 owning snapshot，读取时重新
借用该快照。跨暂停保存 ID 与所有者，恢复时重新 lookup，不把 NodeView 擦为 static。
**单独持有一个 chunk 的 Arc 不足以支持跨块遍历。** A 中的节点引用 B，pin A 不会自动
保活 B；只有经完整闭包检查的 GraphPin 才能作为快照 pin 的精简替代。
GraphPin 暂不作为第一版接口，避免在生命周期正确前引入另一套回收协议。

PinnedNode 仅保证存储有效。迁入新的 SemanticView 仍须检查声明可见性、修订与依赖，
不能因保留了旧快照而把旧 trait 选择当成新世界事实。原始存储读取接口限制在编译器内部；
源级反射、lookup 和执行入口使用带语义视图的观察服务。

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

本文件选择存储和所有权方向；下文继续明确接口约束，具体节点枚举、完整 Rust 实现、
持久 schema 与性能验收仍未完成。
没有修改源码、删除旧 arena 或扩大并发；迁移仍按[计划](refactoring-plan.md)推进。

## 10. 草稿与发布接口的所有权约束

prepare 需要暂停，因此不能把整个准备过程藏在一个必须同步返回的函数里。
目标 owner 划分为可变 DraftOwner、PreparingPublication continuation、不可变且不可克隆的
PreparedWorld，以及安装后的 PublicationReceipt。它们不是同一对象上的公开状态字段。
下面仅表达接口约束，具体服务类型随生产接线确定：

```text
DraftOwner.begin_prepare(base_snapshot, semantic_view, dependencies)
    -> PreparingPublication

PreparingPublication.advance(ready_response, allowance)
    -> Waiting(request)
     | Ready(PreparedWorld)
     | Rejected(owned_diagnostic, recoverable_state)

PublicationOwner.install(PreparedWorld)
    -> Installed(PublicationReceipt, RetiredOwner)
     | Stale(PreparedWorld, current_base)
```

advance 的实现不得重做已消费的响应；Waiting 保存稳定身份、拥有者与进度，不跨暂停
持有 NodeView、可变表条目借用或旧 VM 槽指针。预算不足也不把 continuation 变成成功根。
生产入口是否采用枚举或分类型回调可由实现决定，但 Ready 必须转移所有权，不能同时
保留能够修改其块或证据的草稿别名。

PreparedWorld 拥有候选根、基线票据、冻结草稿、接收者证据和预留安装资源。
install 仅由单一发布 owner 调用，并消费 PreparedWorld；不提供一个公共可克隆 token
允许多次安装。同一个已经完成的宿主请求收到重复响应时，由 RequestId 的结果记录
返回先前收据，不再次运行 install，也不重新分配声明身份。

Stale 返回原 PreparedWorld 的所有权，而不是丢弃它并隐式重跑 builder。
重新准备是显式转换：保留已完成且仍合法的源执行结果，重新核对变化的前提和视图；
依赖没变不能只凭 world revision 猜测，依赖变了也不能让不纯计算自动重放。
可修复拒绝必须携带足以继续的受控状态；不可修复拒绝消费草稿并留下独立诊断。
所有错误及诊断不得借用即将销毁的草稿缓冲。

安装成功时，旧根移动到事先预留的 RetiredOwner 中。调用方负责在安装临界区之外
安排释放；若未来改为内部队列，队列节点同样在 prepare 中预留，不能在根切换后才
尝试有失败可能的分配。通知的发布状态先写入可恢复记录，再由外部调度实际唤醒；
消费者取消不会撤销已经成功的世界发布，也不会使重复通知产生第二次结果交付。

Drop 只清理编译器拥有的数据和登记，不能执行源程序、继续求解或决定发布成功。
回收超过普通执行预算时仍须完成必要 owner 清理；数据量大的释放按回收策略计费，
不承诺一次 Drop 总是常数时间。内部节点不允许带任意插件提供的析构回调。

## 11. 依赖闭包与接口验收

封块时分别核对批内 DraftNodeId 和指向已发布数据的 NodeId。
所有批内预留槽必须有匹配种类的定义或明确不交付；未填的 forward reference 不能
在封块后靠共享可变单元补写。跨批依赖必须属于基线快照或已纳入候选根的合法依赖。
草稿 owner 不同但 slot 相同的 ID 不可混用；失败后也不回收稳定身份计数供下一批冒用。

引用闭包包含节点边、切片成员、值载荷、代码常量、证据和来源中的全部存储引用。
采用按节点种类穷尽匹配的内部访问器收集边；新增节点变体时必须补相应规则，不能用
默认“无子节点”吞掉新边。收集器负责物理可达性，语义依赖观察仍有自己的规则，
不能因为某个块被保活就把其中全部声明当成当前查询的语义依赖。

闭包检查允许合法互递归的 ID 环，使用 visited 集合控制遍历；环本身不等于非法。
按值布局环、类型 occurs 及源阶段限制由各自检查器决定，不用物理图无环性替代它们。
候选 root 无需永久保留未交付的临时节点；但清理不能遗漏实际代码或证据引用的节点。

迁移时需同时验证编译期 API 约束与运行时语义：

| 情形 | 所需结果 |
| --- | --- |
| 返回超过 reader 生命周期的 NodeView | Rust 借用检查拒绝，不依赖运行时恰好没回收 |
| A 引用 B，丢弃原快照后只留长期节点句柄 | 默认 PinnedNode 仍保活 A/B；单块 Arc 不作为遍历凭据 |
| 相同 slot 来自不同草稿或 session | 形成/查找拒绝，不读取碰巧存在的节点 |
| prepare 暂停后原调用栈退出 | continuation 拥有必要快照与状态，恢复不访问旧栈借用 |
| install 因基线过期被拒绝 | 当前根不变，冻结草稿仍有 owner，不重放源副作用 |
| install 成功后消费者取消或重复通知 | 世界保留一次发布，VM 结果至多交付一次 |
| 跨块互递归与按值布局递归 | 存储可达检查与布局检查分别作出结果 |
| 旧快照 pin 存活，但新语义视图已变化 | 旧节点可安全读取，新查询仍重新验证语义证据 |

这些是实现验收要求，尚未生成对应 Rust 类型或运行 compile-fail、Miri、回收压力测试。
现有 HirPackageBox 的高阶构造与借用期 accessor、CodeSession 的精确 owner 匹配继续保留，
直到这些保证在实际新入口中被替代。
