# 世界存储、句柄与生命周期

状态：整体目标仍未完成；required、调用、driver 资源与实例化已按第 1.1–1.4 节接入源码目录。
初始源码审计基线为 `7178e238`。
本文细化[全链路架构](architecture.md)中的 WorldSnapshot 和 PublicationBundle，
与[发布状态协议](elaboration-state-and-publication.md)、[依赖观察](dependency-observations.md)
共同决定存储边界，不改变源语言可见性或声明身份。
增量语义依据为[正式查询契约](../docs/compilation/incremental-queries.md)。

## 1. 已有所有权保护不能因重构而丢失

下列路径均以 `compiler/` 为根，记录初始静态审计。旧 middle::ty 已在源码清理中删除，
第 1.1–1.4 节以外的世界存储迁移仍待实施；未运行 Miri、并发或内存压力验证。

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
NIR Package 封装到 DeclarationSnapshot 中。构造函数消费 package，通过 prepare
完成第 1.2 节的共享目录后才返回 Arc；失败不会发布 RequiredWorld。字段保持模块私有，
生产入口不能分别替换声明或索引。它是准备计算的声明世界，不是第 10 节尚待实现的
待发布 PreparedWorld。

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
它，Copy 查询从这个 owner 取得 package 和 index。required 发布器的证明服务也从
RequiredWorld::capabilities 取得借用视图；实际 invocation 的三类查询进一步统一到
[调用内作用域](type-models-and-theory-evidence.md#调用内的统一声明查询状态)，不再在调用点
手工配对这些字段。调用内查询通过 DeclarationSource::Snapshot 保留同一个目录，
不再为每次 required invocation 重建；发布器证明也借用这个目录。执行与资源声明
分别拥有自己的 snapshot，不能因为两者都使用同一个 owner 类型而合并权限。

调用前提与类型 context 仍按本次 invocation 传入借用视图；它们不是资源快照中的
全局事实。资源 dump 展开自身的 source snapshot，包含 index 和 package，便于核对
权限来源。回归检查同一 Copy 实现仅存在于资源世界时，两份世界各自给出正确分类、其他 subject
不取得该权限、资源不足后可重试，以及派生调用共享同一资源 owner。

共享声明不缓存计算结果。每次 invocation 仍按自己的输入、预算与 live graph 执行；
输入赋值失败不会污染其他派生调用。原准备结果释放后，未完成的派生调用继续保活
快照，最后一个 owner 释放后 package 与索引一起回收。S-expression dump 继续展开
实际 package、index 和调用供应，输出不依赖 Arc 地址。

回归覆盖相同 owner／索引共享、失败输入路由的原子性、派生调用不同输入的独立结果、
不同快照中同号声明的不同结果，以及 Weak 观察的最后引用释放。原有 live closure
连续调用和 builder 公开接口检查继续覆盖实际执行。测试夹具编辑 package 副本后
重新建立 RequiredWorld，不能保留修改前的索引；生产没有可变访问器或 Arc::make_mut
旁路。目录共享回归构造两千多个声明，先测量无额外证明需求的调用成本，再只增加
不足以冷建目录的证明额度；Type 和普通值 invocation 均须在该总额度内重试并完成。
这单独保留了 VM 执行索引的准备成本，不宣称它也已跨调用复用。单次证明加入的前提
不会进入另一查询。

当前仍是单份完整 package 的共享所有权。初次准备分别收集可执行与资源声明并建立各自索引，输入供应
与上下文仍会复制；没有完成分块存储、跨编辑增量索引、稳定句柄或通用发布事务。
不以此宣称 M1 的完整 WorldSnapshot 已实现，也没有测量整体编译性能收益。

本切片的共享、释放、输入隔离和实际执行回归继续保留；本轮检查证据集中列在
第 1.2 节。本轮 builder 用例已通过，不再将
历史失败当作当前状态。未执行全 workspace 或内存压力验证。

### 1.2 保活源码的共享声明目录

`type_resolution::dispatch::DeclarationSnapshot` 同时保活不可变 `Arc<Package>` 与
惰性建立的声明目录。目录包含 implementation Index、参数域对应的 sort，以及
function／runtime callable／normal-form head 的 ID 到数组位置映射；不持有指向
owner 内部的自引用。查询时从该 owner 的 package 取得借用，不能将目录另配一份
同号但内容不同的源码，也不能通过公开 API 修改已登记的 package。

克隆 snapshot 共享 owner；从它导出的 Arc 再经 `Arc::make_mut` 修改会形成另一份
源码，旧目录保持不变。新源码需要新 snapshot。构造目录按实际工作计费，只有完整
构造及保留成功才安装；失败后再次构造不能享受半成品的折扣。显式 prepare 可以在
发布准备结果前完成同一目录，不需要伪造 Context 或证明请求；重复 prepare 和完整
目录的只读借用不复制表，也不收取新的构造成本。建立新的 DeclarationWorld 计入
目录句柄保留成本，并创建独立的 Context、前提和规范化状态。共享目录不签发调用许可，也不缓存用户函数、Eq／Hash 或命题结果。

公开 PreparedInvocation、纯函数缓存 Invocation 和 Eq／Hash 的 Binding 已用这个
owner 替代分别保管 package／index 的字段。CodeSession 从相同 owner 取得精确源码
Arc，继续只复用代码；每次执行仍创建新 VM。缓存准入、资源协议查询视图和调用期的
三个声明服务从各自的同一 source owner 取得索引，避免人工拼接不同来源的字段。
同一个缓存调用为不同输入类型建立 Eq／Hash binding 时，也共享这个 owner，不再
为每种协议复制 package 或另建目录。binding 的值接纳 token 仍分别建立：目录共享
不允许协议 A 接受协议 B 的值快照，资源与用户契约也仍需各自检查。共享协议入口
要求源码已移除预选 dispatch 表，并在准备阶段拒绝混入闭合执行表的快照；按需 VM
仍保留同一拒绝，不把既有执行表当作声明证据。

最后一个 snapshot／CodeSession／源码 Arc 消费者释放后，目录和源码可分别回收；
目录不含反向 owner 引用。S-expression dump 展开源码及 pending／完成的目录状态，
不输出地址。回归检查首个请求构造、跨调用共享索引、不同前提重检、同号新源隔离、
预算失败后的完整重建、Weak 所见的最后引用释放，以及 dump 的嵌套和字符串转义。

required 准备、发布器证明和其资源权限 owner 也已采用该目录，见第 1.1 节。
这仍是一份完整 Package 的共享目录，未实现分块或跨编辑的结构共享。luna driver
资源 ProtocolContext 的后续接线见第 1.3 节；staged 入口独立于声明快照的迁移见第 1.5 节。
PTC Context 的建立、required 和外围资源／协议准备预算仍有独立成本；接受 `&Package` 的
兼容准备入口仍复制整包，其捕获成本尚未纳入统一预算。未测整体编译性能收益。

required 目录迁移阶段的检查工作目录 `/home/fin/luna`，统一使用
`python3 -B development/check.py --timeout 900 --`；result 文件保留完整命令及输出：

- `test -p elaboration -p materialization --lib --no-fail-fast`：退出 0，181 + 29 项通过，
  `/tmp/luna-tools-n2nyv2bd/result.json`。
- `test -p luna --test builder_invocations --test library_protocols
  --test clause_staged_constants --test comptime_staged_values --test trait_domain_substitution
  --test trait_query_failures --test staged_invocation_reuse --no-fail-fast`：退出 0，
  真实 sysroot builder、公开协议、staged 与类型查询共 131 项通过，
  `/tmp/luna-tools-yro__7c9/result.json`。
- 三个修改的 Rust 文件通过 scoped rustfmt check，
  `/tmp/luna-tools-9rwofz81/result.json` 退出 0；`git diff --check` 通过。

本轮去重 341 项通过。最初的新增预算用例遗漏了独立 VM 执行索引的初始化成本，
`/tmp/luna-tools-wxvr5j9c/result.json` 为 209 通过、该 1 项失败；已改为先测无额外
证明需求的调用成本，再检查新增证明额度不足以重建目录，未提高生产预算或绕过检查。
未执行全量回归、Miri 或整体性能基准。

先前可复用调用迁移的证据仍在 `/tmp/luna-tools-8saue3f2/result.json`、
`/tmp/luna-tools-rez2nnq6/result.json` 和 `/tmp/luna-tools-55y018wp/result.json`，
不计作本轮执行结果。已知 `memoized_invocation_specializes_nested_type_forwarding`
前端失败本轮未重跑：此前 `/tmp/luna-tools-p1o2diml/result.json` 退出 101，在
`1465..1484` 报 `Type domain equality: UnvalidatedModel`，与独立基线
`/tmp/luna-tools-uq7f8i42/result.json` 一致；本轮不声称修复它。

### 1.3 Driver 资源协议的源码所有权

`luna::resources::ProtocolContext` 不再独立保存 implementation Index，然后由
`world(package)` 接受另一份 package。它现在持有 DeclarationSnapshot、从同一源码
建立的 Context 与显式绑定的 Copy／Drop 声明身份；只有完整目录和协议绑定都成功
才交付上下文。`world()` 从该 owner 借用 implementations、traits、index 和 Context，
调用者不能通过这个接口替换权限来源。`package()` 返回同一 owner 中的只读源码。

资源分析、Copy transfer、效应值传送、checked runtime session 和 native 清理计划
已使用这个接口。CheckedSession 将优化后的源码移入 owner，不再另外保存一份 Cow；
后续测试入口只共享已准备源码，各自建立实例图、清理计划和解释器。失败、预算、
程序存储与 Drop 执行仍属于各次调用，目录共享不签发资源安全证明。

`selected_in_source` 接受已经存在的 snapshot。缓存调用及 Eq／Hash 协议的资源
绑定因此复用自己的目录，不再为验证 Copy／Drop 再建 implementation Index。
显式 comptime invocation 若启用资源检查，其类型、方法与关联项服务也使用这个
目录。资源世界只接纳显式选择的源码 trait 身份；热目录不会使一个同名普通 trait
自动获得 Copy／Drop 资格，也不能跳过缺失或歧义检查。

源码快照可能包含闭合 dispatch 表，资源绑定不把该表作为证明；source invocation
仍遵守自身入口对预选执行表的拒绝。这里没有让程序状态、内存或清理义务进入 PTC，
只是让程序验证消费者借用来源一致的类型／trait 声明事实。

回归分别检查：资源查询与其他声明查询借用同一个索引；原源码 Arc 经 copy-on-write
移除实现后，新旧同号世界给出不同 Copy 分类；冷构造失败可以重试，热目录中的
零预算资源查询仍不能取得 Copy；最后一个 owner 释放后源码回收。S-expression
诊断展开绑定、Context 及 source snapshot，不依赖指针地址。

兼容的 `selected(&Package, ...)` 入口为取得所有权会复制一次完整源码；初次将借用
源码放进 checked session 也有这项成本，已经持有 snapshot 的路径不复制。捕获复制、
Context 建立和各阶段预算尚未统一，不能将本迁移称为整体性能优化。ReferenceSession
及进一步的实例化复用见第 1.4 节，staged 入口与 VM session 迁移见第 1.5–1.6 节。
通用验证函数的其他 package／world 调用边界不在此声称全部关闭。

driver 资源 owner 迁移阶段在 `/home/fin/luna` 使用
`python3 -B development/check.py --timeout 900 --` 检查，结果均退出 0：

- `test -p luna --lib --test resources --test key_code_sessions --test library_protocols
  --test trait_query_failures --no-fail-fast`：491 项通过，包含真实 sysroot 公开协议、
  Copy／Drop 分析、host 方法清理和新的快照隔离回归；
  `/tmp/luna-tools-6637mekl/result.json`。
- `test -p luna --test execution --test host --no-fail-fast` 加 direct-passing、checked
  pointer、资源入口、bound tuple、quote callable 和 splice cleanup 过滤，共 58 + 35
  项通过；完整过滤列表在 `/tmp/luna-tools-_hj8kunv/result.json`。覆盖解释执行与
  LLVM/native 消费者，不以仅构造 Context 证明端到端接线。
- `test -p luna --test source_tests --no-fail-fast`：13 项通过，验证多测试入口共享准备
  后的失败、预算、清理隔离和 native 恢复；`/tmp/luna-tools-e86jpo1_/result.json`。

本轮去重 597 项通过；八个修改／新增 Rust 文件通过 scoped rustfmt check，
`/tmp/luna-tools-j6s8usej/result.json` 退出 0，`git diff --check` 通过。
未执行全量回归、Miri 或性能基准；第 1.2 节所述前端既有失败本轮没有重跑。

### 1.4 实例化与清理闭包复用源码目录

`materialization::instantiate_in_source` 及带外部类型根的对应入口接受
DeclarationSource；原 `&Package` 入口仍委托同一个构建实现，不保留另一套算法。
Builder 从 source 取得精确 package 与 DeclarationWorld。Snapshot 复用已经完成的
不可变目录，Borrowed 为本次构建建立目录；不允许将来源和目录分别传入。

每次构建建立独立 Context、前提、规范化视图和工作队列，执行
当前 expansion 服务，再校验完整图。生成函数、展开来源、临时布局、runtime binding、
实例去重集合和待发布结果不进入共享目录。热目录不能跳过用户代码展开、缺失主体
检查或最终图验证；失败只丢弃本次候选图，不把半成品保存在源码 owner。

runtime ReferenceSession 现在拥有 DeclarationSnapshot；其执行期类型证明及每次
实例化从同一个 owner 取得目录，原独立 package／index 字段和 driver index 构造
helper 已删除。效应 reference execution 同样从 ProtocolContext.source 进入实例化。

checked 清理的内部入口改为接收 ProtocolContext，集中取得 source、world 和协议
绑定。清理闭包增长的每一轮构图复用此目录，仍重新处理当前根、临时生成布局和
收敛所需的资源事实；前一轮的 schedule 不作为新一轮证据发布。执行与 native
消费者都接入该入口。公开低层 planner 继续接受调用者的借用世界，以支持独立
清理规划和畸形 NIR 的拒绝检查；它不是已检查源码的执行许可。

`InstantiationSession` 将只读 DeclarationSource 与已完成的 NIR phase 检查绑定。
第一次构图在当前预算内完成全包阶段验证；后续根集合变化复用该结构性事实，仍收费
访问并重建当前图。phase 失败不标记成功；phase 成功而后续展开／发布失败时可以保留
这一源码事实，但不保留候选图、资源事实或展开结果。新源码需要新 session，即便
声明号相同；借用期阻止原源码被修改。Display 显示所属源码与 pending／checked 状态。

带 `&mut Budget` 的构图入口把输入 phase、目录准备、展开、发布复制和最终图验证
接到同一额度；成功返回余额，失败或 unwind 保留预留额度，不返回部分图。旧按值
接口委托这一入口。cleanup 的重建循环持有同一 InstantiationSession 和同一构图
额度，发现更多析构函数根后不能重新得到初始额度。构图和资源规划仍是 driver 明确
分开的阶段：构图消费其 Budget，规划消费自己的 fuel；runtime 执行也保留原来的
独立阶段额度。本次没有把 per-stage 接口改成整个编译流程共用一个总计数器。

最终发布在复制根、runtime binding、展开来源和 normal-form 元数据前预留相应工作。
NF 参数域和完整类型命题载荷都计入复制成本，不展开头、不求值其中的命题。可执行
目标与 template storage 目标合并为一个按完整实例身份查询的临时索引，替换逐方法
扫描所有 template 表；保留／剔除仍比较声明及全部实参。过滤前完成预算检查，最终
IIR 验证仍是交付条件；这个索引不提供类型相等、方法选择或资源证明。

回归使用数百个声明，比较冷构造和热目录进入实际展开回调时剩余的预算，同时
要求输出图一致、每次仍调用本次展开服务。换入口、展开错误、零预算、同号源码
改成缺失 callee 以及恢复原源码均分别检查，不能由已有图或过去的成功代替。

这完成的是声明目录／source phase 复用及构图预算接线，不是 IIR 图缓存或完整增量
编译。Context、根闭包、实例展开与资源检查仍按次执行；staged 快照及 VM 索引的
后续迁移见第 1.5–1.7 节。将借用 package 转为长期 owner 的整包复制、其他阶段的
工作计费和跨编辑失效仍未完成；尚无整体性能收益结论。

本轮构图预算与发布边界的检查工作目录为 `/home/fin/luna`，命令通过
`python3 -B development/check.py --timeout 900 --` 运行：

- `test -p luna -p materialization --lib --test expansion_boundary
  --test scoped_dispatch_inputs --test materialization_budget --test library_protocols
  --test conditional_capture_cleanup --test temporary_method_cleanup --test quote_functions
  --test staged_aggregate_contracts --test native_optional --no-fail-fast`：退出 0，
  313 项通过，`/tmp/luna-tools-cjctikis/result.json`。覆盖全部构图预算截断点、
  phase 失败／展开失败后的恢复、热 session 换根与同号新源码隔离，以及真实 sysroot
  协议、quote、聚合资源与 native 可选值路径。
- `test -p luna --test drop
  late_quote_cleanup_rebuilds_layouts_when_destructors_add_generic_roots -- --exact`：
  退出 0，1 项通过，`/tmp/luna-tools-n94iy6so/result.json`。验证析构函数新增泛型根
  后仍重建正确布局；新增 materialization_budget 用例另行验证重建不能重领预算。
- 六个修改／新增 Rust 文件通过 scoped rustfmt check，
  `/tmp/luna-tools-p8g2vw1k/result.json` 退出 0；`git diff --check` 通过。

最终去重 314 项通过。实现期间曾把构图开销扣入资源规划 fuel，导致公开库路径
预算耗尽；已按 driver 原有 per-stage 契约恢复两个阶段各自计费，同时保留构图各轮
共享余额。没有提高 runtime 默认预算或修改现有测试额度来掩盖问题。

额外检查发现 `closure_local_handlers` 中
`outward_request_cannot_retire_the_storage_a_callback_will_read` 的诊断断言失败：
实际先报告 `handler lexical capture address exposure requires a memory contract`。
临时用 HEAD 的三个生产实现文件运行同一精确测试，仍以同一诊断退出 101，证据为
`/tmp/luna-tools-l2lncmwl/result.json`；其他用户工作区文件保持原样，之后已按保存的
SHA-256 恢复本轮实现。该基线问题未修复，也未改断言或弱化资源检查。
未运行全量回归、Miri 或整体性能基准；其他小节的历史结果不计作本轮执行。

### 1.5 Staged 入口与声明世界分离

materialization 不再给私有 package 追加 `<staged-input>`，不再寻找空闲声明 ID，
也不在每次求值前后改写、清空该函数的签名和主体。Snapshot 持有不可变
DeclarationSnapshot；类型命题、方法与关联项查询从它取得目录，并仍使用当前
Builder 的 Context 和前提。未完成的目录不能作为成功证据发布。

`Interpreter::invoke_comptime_expression` 接收本次已经完成类型／资源检查的闭合
表达式。它单独检查 phase 与 lexical unsafe 边界，再建立无声明身份、无捕获的
comptime closure，通过普通 invocation admission、调用帧、返回值检查与状态恢复
执行。它不是公开的免检查 eval，也不接受 runtime VM 或 IIR 图执行权限。
临时 closure 的代码由本次 owner 保活，不借用某个真实声明的身份作为缓存键。

NIR 的 CallableProgram 现在允许没有声明身份的主体；对应 dump 明确输出
`(callable-program (anonymous) ...)`。共享 PhaseContext 检查逻辑同时服务声明主体
和 detached 主体；匿名主体不获得 quote 身份，也不会把 machine owner 记录到
伪造声明。完整 package 检查仍保留原有声明与主体的验证范围。

Phase Error 保留 Budget／Invalid 分类；声明参数、成员目录、存储、默认参数、
preparation 依赖、captured value 与 call safety 的资源失败不能退化为非法代码。
preparation 遍历也保留类型化失败，不能靠字符串或“剩余 fuel 等于零”判断：宽度
预留失败可能仍有余额。VM 的源码和生成定义入口统一将该 Budget 映射为
StepsExhausted。新的 detached 检查、复制和执行消费同一次 invocation 的预算。

每次 staged 求值仍创建独立 VM，冻结本次结果，保留已有 closure 共享／循环检查。
失败后保留的是不可变声明快照，不是半成品入口、局部存储或此前的返回值。
quote template 不再追加到普通 `functions` 表。VM 的声明准入另建只读模板位置表，
用完整 Function 记录检查 trait 实现的阶段和参数；这张表不会进入执行函数目录。
quote 成员及默认主体必须仍属于 quote 声明，普通 callable adapter 保持独立身份。
闭合源码准备不会为 quote 成员建立可执行方法映射，显式 dispatch 表中的 quote
目标也被拒绝。保留签名、默认值、前后置条件和类型要求，不以无主体 identity
替代模板，更不依靠“comptime 恰好不能调用 runtime”来保护 quote 边界。

Builder 保留入口的 DeclarationSource。staged 捕获遇到不含预选 dispatch 的现有
DeclarationSnapshot 时直接保留该 owner，CodeSession 与三个查询服务使用原来的
精确源码及声明目录；保留句柄先消费一步预算。该路径不遍历函数来估算不存在的
复制，也不复制 Package。保留 owner 本身不签发 phase、类型或资源证明；VM 首次
准备和每次调用继续执行对应检查。借用入口仍需要取得源码所有权，含预选 dispatch
的入口仍建立剥离该表的私有声明快照；两者保持 quote 表与完整契约，不改写原源码。

需要私有所有权的入口改用 `nir::copy_budget::clone_declaration_source`。先在同一
额度内完成所有保留载荷的预检，再构造副本；直接省略预选 dispatch，既不遍历也不
复制待丢弃的执行表。Package 用穷尽字段解构和显式构造，新增字段必须决定其保留
与计费责任。函数与 quote 的默认值／契约、参数域、NF 元数据、trait 与 impl、
关联初始化器、上下文代码签名、enum 编码、运行实例及清理路径均计入预算。
源码、目标、来源、身份游标和完整契约保持原值；谓词只作为数据复制，不求值、不
展开 opaque 头，也不把程序规格送入 PTC。

表达式与类型沿用通用 copy preflight，分别保持 Steps／Depth 失败；预算是复制和
保守结构预检的工作额度，不是精确分配字节数。普通 Expr::clone 保留共享 closure
body 的 Arc，只检查实际复制的捕获初始化器、参数和类型部分；共享本身不签发该
body 的执行许可。所有复制都在完整预检之后发生，失败不返回部分 Package。原有
source owner 快路径仍只保留句柄，不因兼容入口需要复制而重收其载荷成本。

源码分块与借用的 dispatch 投影视图尚未实现：两个兼容入口仍复制所保留的声明
数据，只是这次捕获已经有完整预检且不再复制被丢弃的 dispatch。其他子系统的
Package 捕获不因新增此入口而自动完成预算迁移。共享 owner 路径已消除这次复制，
不能据此声称所有源码捕获均已修复。携带作用域前提的 CodeSession 与 phase 目录
接线见第 1.6–1.7 节。类型／资源正确性仍由上游保证；没有整体性能收益结论。

本轮完整捕获预检的检查工作目录为 `/home/fin/luna`，通过
`python3 -B development/check.py --timeout 900 -- test -p nir -p materialization -p luna
--lib --test copy_budget --test expansion_boundary --test quote_functions
--test staged_aggregate_contracts --test library_protocols --test materialization_budget
--no-fail-fast` 执行，退出 0，366 项通过；
`/tmp/luna-tools-k02oo0pz/result.json` 保存完整命令及输出。覆盖宽载荷在额度不足时
拒绝、准确额度恢复、深度限制、保留共享 closure、丢弃 dispatch 与原源码隔离，
以及实际 staged 捕获对未使用声明的计费。原有真实 sysroot 协议、quote 展开、
聚合资源和清理重建保持通过，没有提高生产预算或修改已有语义断言。
五个修改／新增 Rust 文件通过 scoped rustfmt check，
`/tmp/luna-tools-txw2m7bd/result.json` 退出 0；`git diff --check` 通过。
最初测试编译的 fixture 使用了不存在的 Budget 构造器和错误的 Term 载荷类型，
已按实际表示修正；没有修改既有用例。未执行全量回归、Miri 或性能基准。

### 1.6 Staged VM 索引与当前调用权限分离

按需源码入口拆成独立的声明准备与当前调用准入。前者验证精确 source package，
建立没有调用者前提或抽象参数权限的基础 VM；该模式禁止预选 dispatch 表，因此
声明准入不需要借用调用者前提来补齐表。后者每次重新展开当前 trait 前提（含父约束
和 capability），授予当前 Type／普通 comptime 值参数的显式权限，并安装本次宿主
证明、方法及关联项服务。这里的前提只授予已有证明，不生成方法实现。

`for_comptime_resolving_in_scope_in_session` 只复用同一 Arc 所拥有的精确 package。
无作用域的旧 session 入口委托该入口，非 session 构造也使用同一准备／准入边界。
另一份内容相同、声明号相同的 package 必须重新验证；closed-table 与按需解析的
索引仍使用不同槽位，不能因为共享源码就交换执行许可。

SourceIndexes 新增 trait 声明位置目录，扩展前提时不重新扫描、复制整个 trait 表。
目录只持有原声明位置，不保存前提集合；每个 VM 单独建立 assumed_interfaces 与
abstract 参数权限。初次调用若因错误前提、冲突权限或预算不足失败，不发布部分
索引；已经存在的完整目录可在失败后保留，但失败 VM 的权限不能进入下一次调用。
构造失败继续遵守整份额度预留的旧契约，不伪造精确剩余工作量。

materialization::staged::Snapshot 同时持有 DeclarationSnapshot 与同源 CodeSession。
多次 staged 求值复用 VM 声明索引与有限代码缓存；每次仍创建新 VM、安装当前查询
服务、检查当前表达式并冻结本次返回值。退出与错误恢复只交还不可变代码，不交还
局部存储、返回值、活动推断状态或证明答案。匿名入口主体仍按本次代码建立 owner；
此处没有增加按源码范围缓存任意表达式结果的捷径。

本轮测试暴露并修复了既有的直接调用缺口：comptime 源码入口跳过函数值形成步骤，
却假定闭合 capability 已在形成时验证。现在命名源码调用在输入绑定后核对这些义务，
不能从缓存代码获得过去调用的权限。直接 invoke、空声明参数的 invoke_instantiated、
以及 detached 表达式中的调用均走该检查；Missing／Unknown 与 Rejected 分开拒绝。
普通参数依赖的义务仍在绑定后处理；已验证 IIR 的形成证据与 runtime 源码原有的
形成检查保留，不把源码宿主查询强加给 concrete artifact。

回归在同一 session 交替有／无前提，检查父约束扩展、每次宿主服务、真实代码复用、
抽象参数权限撤销、冷／热失败后恢复、同号不同世界和逐预算切点。materialization
回归含大量声明元数据，比较首次与后续实际 staged 求值的预算消费，并检查输出与
局部绑定隔离。较少的准备计费是该边界的证据，不是整体 wall-clock 性能结论。

detached phase 声明目录的后续复用见第 1.7 节，每次表达式检查仍建立独立上下文。
尚未处理：整包源码捕获、各阶段统一预算及跨编辑增量失效。
required／其他消费者的准备路径仍各自审计；不能据 staged 接线完成宣称
全部 VM 世界所有权或 M1–M7 目标完成。

本阶段在 `/home/fin/luna` 经 `development/check.py --timeout 900 --` 执行：

- `test -p comptime_interpretor -p materialization --lib --no-fail-fast`：退出 0，
  514 + 30 项通过，`/tmp/luna-tools-ifauj3q4/result.json`。新回归覆盖 tree／bytecode
  直接入口、热代码的前提撤销、抽象 Type 传送、错误准入不发布、预算不足与恢复，
  以及实际 staged Builder 的准备费用和状态隔离。
- `test -p luna -p materialization --test prepared_invocations --test scoped_dispatch_inputs
  --test clause_staged_constants --test comptime_staged_values --test quote_functions
  --test staged_aggregate_contracts --test invocation_value_templates --test builder_invocations
  --test generated_definitions --test library_protocols --test trait_query_failures --no-fail-fast`：
  退出 0，282 项通过，`/tmp/luna-tools-lz41k7sj/result.json`。含真实 sysroot、公开库
  native 用例、按需方法服务和构造失败恢复；没有改写既有用例的语义预期。
- `test -p luna --lib --test key_code_sessions --test staged_invocation_reuse --no-fail-fast`：
  退出 0，86 + 4 + 4 项通过，`/tmp/luna-tools-t3dxt24o/result.json`。检查共享构造入口的
  其他消费者，确保 Eq／Hash、缓存协议及普通调用仍区分代码复用与结果复用。

本轮去重 920 项通过。最初新增测试捕获的 closed capability 缺口已由调用入口检查
修复；抽象 Type 的预期改为既有 TypeParameter 值表示，未改变传值或准入语义。
12 个 Rust 文件 scoped rustfmt check 退出 0，`/tmp/luna-tools-d83870x7/result.json`；
`git diff --check` 通过。未运行全量回归或整体性能测试，正式 docs 与已有工作区改动
保持原样。

### 1.7 Phase 声明目录与表达式检查分离

`nir::phases::Inspection` 将声明目录绑定到精确不可变源码。目录持有参数域、
函数阶段、参数框架位置、quote 身份和 trait 成员位置；成员合同及参数框架仍从所属
package 取得，不复制成另一份合同，也不保留指向外部临时数组的引用。
普通借用 VM 在自身生命周期内复用目录；CodeSession 的 SourceIndexes 保活原 Arc，
让后续新 VM 复用同一目录。`retain_in` 只接受原 package 的精确 owner，不能用内容
相同或声明号相同的另一份源码延长位置的有效期。IIR VM 不获得源码表达式执行权限。

目录惰性建立。首次准备在调用者 fuel 下完成声明检查，先预留成员位置表的容量成本，
再按原有参数／合同遍历计费；失败不发布半成品。后续访问仍收费，但不重复遍历已经
检查的声明元数据。完整目录可以在某个表达式失败后保留，这不等于保留失败表达式
的执行许可。Display 使用 S-expression，显示 pending／ready 与所属源码，不输出地址。

源码 VM 构造现在通过同一 Inspection 完成 package 阶段验证，并将已完成声明目录
交给 VM；首次 detached 表达式不再重新检查整份声明元数据。普通闭合构造、带显式
作用域的构造、resolver-backed 构造及 DefinitionAdmissionSession 均消费当前构造
预算，随后建表和准入只能使用余额。原先复制一份预算做顶层长度预检、在独立固定
额度内遍历全部主体、再恢复原预算继续构造的路径已删除。函数主体、默认值、合同及
trait 方法使用同一份检查额度，不向每个 callable 发放新的额度。

Inspection 自身只保存目录；`check_package` 每次遍历全部主体，不把一次 detached
检查视为全包验证。CodeSession 仍只在 phase、建表与当前 scope 准入全部成功后
发布 SourceIndexes；冷构造在 phase 中失败时不能留下可复用索引。已成功的精确
源码索引可以免除后续构造的重复验证，当前表达式及调用权限仍按次检查。
构造失败继续保留外层预算预留，成功才返还余额；没有借本次改动伪造失败的精确成本。

显式计费暴露了缓存调用内部的重复冷构造：真实 sysroot 的 547 个函数在单次阶段检查
消费 524,580 步，原先参数协议与函数执行各检查一遍，首次 Type 缓存调用共消费
1,062,326 步（诊断测量 `/tmp/luna-tools-5lj3oj6v/stderr.log`，没有计作性能基准）。
`CodeSession::fork_for_source` 现让同一精确 owner 的消费者共享只发布一次的完整
源码准入槽；closed/resolving 槽保持分离。fork 时尚未准备的槽也能在成功发布后共享，
但代码缓存、值准入 token、前提和局部 VM 状态各自独立。另一份同内容源码重新准备。
memoized invocation 的参数 Eq／Hash 协议与主体执行使用这一共享族，包括后来建立
的输入 schema；不能把缓存键的值准入结论传给函数或另一个协议。

CLI comptime 的默认总 `--fuel` 改为 2,100,000，将此前两份各百万步的独立 phase
上限和十万步 action 默认额度显式纳入一个总额度。显式 `--fuel` 不增加任何隐含
补贴，其他 CLI action 和低层 VM 的默认值不因此改变。这个默认策略不承诺所有
源码都能执行，也不解除预算失败；它避免默认公开入口在开始计入检查成本后无法
处理已有完整 sysroot。help 与选项测试同步说明新边界。

每次 `check` 都重新构造 PhaseContext、存储查询状态和 worklist，检查当前主体、签名、
runtime/comptime 边界及 lexical unsafe。目录不缓存表达式结果、前提、unsafe 授权或
程序验证结论。调用者仍负责类型和资源正确性；VM 的普通调用准入、执行帧和返回检查
不变。测试检查热目录拒绝非法调用、低预算失败后恢复、精确 owner 生命周期、同号
新世界隔离，以及 closed-table／resolving 两类 session 的真实生产入口。

这消除了 detached 入口逐次重建声明目录的工作，未消除按表达式检查。独立 NIR
`validate`／machine-arithmetic 查询仍保留默认声明／主体额度；materialization 的
后续构图预算接线见第 1.4 节，其他非 VM 消费者尚未全部迁移。
VM 建表中部分 metadata 检查也仍重复，整包捕获与跨编辑
增量失效尚未完成；不能把这一预算接线称为全编译流程统一计费。测试比较冷／热路径
实际预算消费，不据此推断整体运行时间收益。

本阶段工作目录 `/home/fin/luna`，最终经
`python3 -B development/check.py --timeout 900 --` 执行并退出 0：

- `test -p nir -p comptime_interpretor -p materialization -p luna --lib
  --test invocation_value_templates --test builder_invocations --test key_code_sessions
  --test staged_invocation_reuse --test prepared_invocations --test subcommands --no-fail-fast`：
  789 项通过，`/tmp/luna-tools-mft7pkib/result.json`。含逐预算切点、失败后恢复、
  fork 的权限／代码隔离、同源目录共享及真实 sysroot 的公开 Type 缓存协议。
  原有两个 Type 缓存用例保留百万步额度；新增检查要求冷调用付出整包检查成本，
  且参数协议和主体合计不重复付出两份 phase 成本。
- `test -p nir -p luna -p materialization -p comptime_interpretor --test phases
  --test clause_staged_constants --test comptime_staged_values --test quote_functions
  --test staged_aggregate_contracts --test generated_definitions --test scoped_dispatch_inputs
  --test library_protocols --test instance_budget --test struct_admission_session --no-fail-fast`：
  243 项通过，`/tmp/luna-tools-g4aszkal/result.json`。含 NIR 现有阶段规则、definition
  准入、真实 sysroot 公开库协议及 native 用例、quote 和连续 staged 求值。

最终去重 1,032 项通过；18 个 Rust 文件 scoped rustfmt check 退出 0，
`/tmp/luna-tools-4wjvqzwx/result.json`；`git diff --check` 通过。
初轮集成 `/tmp/luna-tools-36pby9ez/result.json` 退出 101，298 项通过、3 项因预算耗尽
失败；通过同源准入共享和明确 CLI 默认总额度修复，没有减免 phase 工作或提高原有
Type 缓存用例的额度。诊断用临时 probe 已移除。未运行全量回归或性能基准，正式
`docs/` 和已有用户改动保持原样。

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

旧 middle::ty 及 interface／intrinsic 中的 TyCtxt 关联已退役；当前 middle 保留
hir_package、hir_query 和 queries，仍服务于活动 HIR owner 与查询。不能因旧类型
组件已删除而移除整个 middle。后续迁移继续同步 compiler/hir、ast_lowering 与
middle 的真实消费者，防止新结构经旧上下文回流。

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
