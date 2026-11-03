# 构建输入、pipeline 任务与产物交付

状态：内部架构设计，尚未实现。本文细化 luna 单二进制的工程交付责任，不制定新的
工程文件语法、公开 build API 或传输格式。源码仍保持审计基线，`docs/` 不作修改。
语义依据为[包与 umbrella](../docs/code-organization/packages-and-umbrella.md)、
[build 上下文](../docs/code-organization/builtin-packages.md)、
[投影接口](../docs/pipeline-projection/declarations-and-interfaces.md)和
[后端交付](../docs/pipeline-projection/backends-and-artifacts.md)。

## 1. 当前实现已经做到哪里

| 入口与源码 | 已有机制 | 不能据此声称完成的部分 |
| --- | --- | --- |
| [cli/subcommands.rs](../compiler/luna/src/cli/subcommands.rs) | 子命令解析、显式包入口和驻留协议入口 | 工程解析、依赖版本选择及完整任务调度 |
| [packages.rs](../compiler/luna/src/packages.rs) 的 SourcePackage、ObservedLoader、CompilationInputs | 解析 ID 与语义 identity 分开；首次读取内容、目录发现及根路径变化可观察 | 持久源码数据库、跨编辑名义身份映射和通用生成源码交付 |
| [build_context.rs](../compiler/luna/src/build_context.rs) 的 Snapshot、Observation | 按来源包读取 mode/target/执行宿主，区分 feature 缺失和 false | 字段级 Observation 尚未接入编译缓存 |
| [compilation_cache.rs](../compiler/luna/src/compilation_cache.rs) | 有界进程内完整请求缓存，重验源码/发现/入口；失效失败不回退旧成功 | 不是持久 artifact store；不缓存 invocation 或执行动作 |
| [frontend.rs](../compiler/luna/src/frontend.rs) 的 Artifact | AST/HIR/NIR/属性检查结果 | 此枚举不是代码文件、入口关联及发布收据 |
| [host/tools.rs](../compiler/luna/src/host/tools.rs) | 工具选择、目标布局、超时、输出限额、临时目录及非空普通文件检查 | 非空文件不证明对象格式、入口 ABI 或完整依赖正确 |
| [output.rs](../compiler/luna/src/output.rs) | 同目录独占临时文件、完整写入/sync、rename，失败清理；保护输入文件 | 多文件集合与入口索引的一次交付、父目录持久性、竞争写入保护 |
| [package_stream.rs](../compiler/luna/src/package_stream.rs) | 顺序 v1 framing、坏帧恢复、前端缓存及回复 | 不接受执行动作或 --output，不是可取消的并发构建服务 |
| [library/build](../library/build/lib.fl) | mode、Target/TargetFeatures、PackageMetadata/MetadataFeatures | tasks/generated/targets 等目录规划不是已交付工具 API |

这些是本轮静态源码证据，未重新运行现有测试。重构保留上述有效行为，尤其不能为了
统一名称而删去首次读取快照、源文件保护、工具限额及失败后的恢复能力。

## 2. 一个输入入口，多个有明确完成条件的动作

CLI、显式 package 请求和 Rust 嵌入入口先转换成同一受检 BuildRequest。
解析层负责参数格式与动作组合；工程层负责包集合、依赖、输入及输出目的地；
语义前端负责语言检查；后端负责目标交付。CLI 不自行选择 impl、模拟类型形成或补隐式类型。

拟议内部结构，不是新的命令行参数或 wire schema：

```text
BuildRequest {
  action, selected_packages, dependency_bindings, builtin_protocol_bindings,
  source_inputs, build_contexts, metadata_inputs,
  target_requirements, tool_selection, output_intent, resource_limits
}

BuildSession {
  admitted_request, source_snapshot, package_instances,
  query_service, task_registry, tool_runner, artifact_store
}
```

输出意图与语义输入分开：换一个导出路径通常只需重新导出已验证产物；若源码或工具
确实读取该路径，则这次读取仍是依赖。不能简单删除当前完整请求键中的某字段而不
检查其消费者。目标、检查模式、协议绑定和 sysroot 选择不得由动作分支静默补齐。

语法 dump、已检查接口、已检查定义、实例、目标产物及实际执行结果具有不同收据。
一次 dump 成功不证明资源检查完成；一次 check 成功不表示已有可执行文件。
声明为完整 safe 检查的入口必须兑现 NIR 检查责任，不能把当前入口的能力缺口写成最终语义。
运行和测试动作消费相应已交付代码，并另行报告执行/断言结果；编译成功不能覆盖运行失败。

## 3. 包身份、配置来源与三个目标概念

| 标识或上下文 | 表达什么 | 不表达什么 |
| --- | --- | --- |
| ResolutionPackageId | 本次解析图中的节点 | 类型的跨会话身份 |
| DeclarationIdentity | 原始包及声明归属 | 导入别名、磁盘位置或配置 hash |
| BuildInstanceId | 源码在本次依赖和配置下的检查环境 | 每次改配置都自动创造一种新名义类型 |
| Query/ArtifactKey | 已验证输入下可复用的结果 | 源程序对象身份或一次外部动作的执行身份 |
| ExecutionHost | 编译器及宿主后端程序在哪里执行 | 所生成 host 程序或设备代码的目标 |
| HostPipelineTarget | 默认 build.target 描述的目标 | 当前机器属性或任意设备配置 |
| ProjectionTarget | 所选投影协议实际读取的设备/其他目标配置 | 覆盖所有 build 查询的动态全局 target |

跨构建实例的结果必须带所属上下文，不能仅按原声明 ID 复用；同时不能把上下文指纹
偷偷并入语言名义身份。跨编辑声明对应关系使用独立恢复映射，见[世界存储](world-storage-and-lifetimes.md)。
Umbrella 成员关系不扩张包私有访问或 open fn 范围，也不把未参与构建的旁支 impl 加入世界。

每个 build 读取保留表达式所属定义的来源构建实例。helper 调用、内联、别名和转交
查询函数不切换来源；template 默认表达式与实例主体分别按自身来源检查。
生成代码保留生成协议确定的来源，不能以“现在正在编译哪个包”恢复丢失的归属。

例如编译器运行在目标 H，host 程序面向 T，而 CUDA 产物面向 D：后端程序在 H 执行，
普通 build.target 仍描述 T，设备库通过显式 package_metadata 等目标接口取得 D。
已经完成的 comptime 布局查询不能由后端重新解释成 D 的布局。
当前 host native 入口已先取得工具链布局；build.target 选项本身尚不切换 LLVM triple/CPU。
最终实现必须验证实际工具选择与请求相容，不能仅因 pointer_bits 一致就宣布完整目标相容。
只请求不依赖目标的前端结果时，不应无条件启动 LLVM；真正需要布局时不能猜宿主值。

## 4. 输入快照与实际观察

输入准入先验证包身份唯一性、依赖别名绑定、根文件及源码归属、配置拥有者和目标约束。
包成员收集只为所选构建准备合法可达输入；名字准备与值计算依赖分开，不能因
builtin/meta 互相引用基础声明就要求按包一次性求值，亦不能允许具体值自我依赖。

SourceSnapshot 保存实际读取字节与解析所用文本之间的关系、根解析结果、发现规则、
目录成员及排除边界。语义查询可依赖解析产物；读取原始源码的操作仍依赖原始字节。
负查询也记录依赖，例如未发现模块、未配置 feature、别名不存在。
单靠已有文件的 mtime/hash 无法发现新增模块或 symlink 根改指向。

同一请求内的输入以快照为准。编译过程中发现快照构造不一致时报告输入变化，不能把
新旧文件混合产物发布为最新构建。只有确认重试不重放源程序外部动作时才可自动重试；
已经执行不纯操作的失败不能由驱动偷偷重跑整包。

构建观察至少保留 `(owner, query, result, provenance)`；值相同但来源不同不自动通用。
`Feature(None)` 与 `Feature(false)` 不同；metadata 未提供与显式空输入不同。
查询字段不存在、类型不匹配或重复键处理也属于可观察结果。
若操作按字段索引读取，可观察该字段及查找规则；若枚举整个 Object，必须观察集合、
顺序及实际读取内容，不能冒称只有单字段依赖。

从完整请求缓存迁至精确观察时，使用“查询种类及显式语义输入定位候选 + 核验依赖清单”。
不能只把旧观察结果的 hash 当作可独立找到未来结果的万能键；控制流因输入变化而读取
新字段时，应重新执行受影响查询并产生新清单。未接通的消费者继续保守失效。

包元数据沿用[wire v1](build-input-protocol.md)的准入和限额；解码成功不等于值在接收世界
合法。不得执行闭包工厂、展开 NF 头或把进程句柄塞进 wire 来省去恢复检查。

## 5. 投影接口、实例与执行任务分开

```text
checked signature + attributes + required inputs
                    -> ProjectionInterface
checked definition + instance inputs
                    -> ValidatedInstance -> target checks
ProjectionInterface + target-ready instance + backend inputs
                    -> PreparedTask -> pipeline callback -> DeliveryBundle
```

接口查询只读取其必要输入，不强制等待整个目标代码生成。类型接口不是未经检查的
签名字串；属性、普通类型依赖及形成条件仍须成立。生成路径的 target checks 不能
取代定义端 NIR safe 检查及实例层证据维护。
设备入口即使由 fn 声明，也不能据此当作普通 host 函数指针调用。

pipeline 回调接收任务范围内的 IIR handle，通过任务通道登记代码、入口和诊断，返回 void。
pure comptime 回调的普通返回值与任务交付不能混为一份 invocation 结果缓存：作者当前
只允许输出 Type 的 invocation 进入结果缓存，不能借后端管理把所有 void 调用缓存起来。
输出 Type 也只是必要条件；不纯执行不能被省略，缓存成本和实际依赖仍须单独判断。
任务结果是否可复用，由其任务契约、执行身份和完整依赖另行决定。

拟议任务状态：Prepared → Running → Validating → Published，活动状态也可进入
Failed 或 Cancelled。CachedCandidate 仅是待核验记录，核验通过才能交付成功收据。
Running 中后端返回不代表完成；必须收齐输出、相关子任务和诊断，再关闭产物登记并验证。
验证后的内容不可继续被后端写入。存在错误诊断、未交付必需入口或目标不符时均失败。

任务先登记生产者，再运行回调。同一任务的多个消费者共享结果；它们并不是同一源代码
位置两次动态 invocation。源执行 RequestId 沿用[状态协议](elaboration-state-and-publication.md)，
不按 callee/PC/实参 hash 折叠不纯调用。外部操作已发起但完成未知时保留该状态和诊断，
不能自动从头执行回调；只有工具协议明确可重试，才允许相应恢复。

一个消费者取消只撤销其等待；没有消费者且任务协议允许停止时再取消生产者。
取消检查与发布使用同一状态转换边界：发布前胜出的取消不产生成功收据；发布已完成后
可撤销消费者交付，但不能宣称已撤回外部行为或删除其他消费者依赖的产物。

## 6. IIR handle 与宿主工具边界

任务持有不可变实例 owner，handle 包含任务世代、实例/世界及访问权限；reader 借用
由 owner 保活。长期异步访问须有显式受管理的 lease，不能保存裸引用到 callback 结束后。
任务关闭后拒绝新的访问；已开始的受保护读取结束、相关 worker 退出后才释放 owner。
取消不能一边释放 IR，一边让宿主回调继续访问。

`meta.runtime.invoke` 的 bridge 由协议提供受控访问；不能序列化内存地址跨进程。
若后端必须在另一进程运行，需要单独版本化的访问协议或稳定 IR 导出，并重新做准入；
本文不假定现有 metadata wire 或 resident v1 已经提供这种能力。

ToolRunner 将程序选择、实际可执行文件、参数、cwd、影响输出的环境、输入文件、
链接库/系统输入和后端版本记录成依赖。只记录 `cc` 路径或 `--version` 字串不证明
工具及隐式输入没变。不能跟踪的外部读取必须由工具契约补齐或取消相应持久复用资格。
驱动读取 LUNA_LLVM_CONFIG/LUNA_CC 等选择项，在会话建立时固定，而非运行中无记录改取环境。

保留现有 stdout 完整性限额、超时和临时 workspace。编译输出超过限额必须失败，
诊断可标明截断；不能把截断的 IR 交给后续步骤。退出码 0 仍须检查真实输出。
取消/超时须收割进程与输出，说明平台上能否终止子进程树；不能把父进程退出解释为
所有写入者均已停止。工具不合作时不发布正在被写入的文件。

## 7. 多产物发布与恢复

拟议 DeliveryBundle：

```text
DeliveryBundle {
  task_identity, interface_reference, target_and_abi,
  blobs[(content_digest, size, kind)],
  entries[(declaration, exported_symbol, representation_contract, blob)],
  dependencies, compiler_and_backend_compatibility, diagnostics
}
PublicationReceipt = immutable manifest identity + available content references
```

内容 digest 检查字节完整性，不能替代类型、ABI 或来源检查。发布前核验每个必需入口
兑现已交付接口；不能让后端先产生某种 ABI，再反向改宿主检查结果。
入口映射、数据及重定位依赖与代码一起交付，不能只恢复一个机器码文件。

选择不可变内容存储 + 单一 manifest 发布点：先在私有暂存区收集并验证内容，封存所有
blob，再发布引用它们的完整 manifest。消费者只从已发布 manifest 取得整组产物，
不枚举工作目录推测完成度。多次文件 rename 不是整体原子操作。
发布前失败可留下未引用的内容块，后续按可达性回收；不能留下成功索引。
读取期间的 manifest/content pin 防止回收删除仍在使用的产物。

manifest 的提交采用同文件系统的原子替换或等效事务。需承诺断电恢复时，还须按平台
同步内容与目录，明确不支持的持久性保证；现有 sync 文件 + rename 不证明父目录已持久化。
进程内任务注册表同步发布收据与成功状态，重复完成通知不能再次执行工具。
磁盘发布成功但回复丢失时，按任务/manifest 身份核验恢复，不能仅因没收到回复就重跑。

导出到用户指定路径是独立 materialization 动作。单文件沿用 output.rs 的机制并扩展
到完整输入保护集合；多文件导出采用版本目录与单一选择入口，或明确报告分步导出，
不能假称任意多个用户路径能同时原子替换。拒绝覆盖源码、配置及工具输入；命中内容
缓存也要执行保护。路径 canonicalize 不消除检查后的替换竞争，受管输出需限定写入者，
必要时用目录句柄等平台机制约束路径解析，不承诺对任意外部竞争写入都可回滚。

命中缓存先核验 schema、声明恢复、语义/目标/工具依赖、所有内容和入口映射。
文件缺失或损坏时从可信内容源重新物化，不能恢复则重建获准可重建的任务；不具备
安全重放条件的外部任务报告需恢复的具体输入/产物，而非假成功。
旧请求的成功 manifest 可以继续存在，不能作为当前不匹配请求的回退答案。
设备加载及执行失败由 runtime 库另行报告，不与编译发布状态合并。

## 8. 保留外部协议，逐步接通真实库

现行 `serve packages-v1` 保留顺序、有限帧、坏帧排空恢复及 HIR/NIR/属性回复范围。
外部输入错误、编译失败与传输失败分别处理；某次请求失败不能污染下一次合法请求。
并发、异步任务、取消或产物句柄若进入外部协议，须另行定义版本和恢复语义，不在 v1
偷偷增加执行权限。S-expression dump 展示结构，不自动成为稳定 wire 或缓存编码。

内部 Display 同样采用 S-expression，显示请求、来源包、host/target、任务世代、
接口与 manifest 关系及失败原因；稳定排序并转义路径。不要输出进程地址充当身份。
构建依赖环诊断显示接口、配置、实例或工具产物的真实边，不以空产物或宿主默认值打断环。

先以真实 build.mode、build.target.features.get、build.package_metadata 迁移来源与观察，
经 sysroot 公开接口验证。随后接通 meta 的 IIR 访问、attrs 的投影协议、build 的任务交付
和 host 后端，最后扩展设备库消费者；这些是纵向实现顺序，不新造六包之外的构建工具。
接口查询与代码生成分别核验依赖，不能把 build/tasks 目录存在算作该链已可用。

## 9. 后续实现的验收反例

| 输入或事件 | 必须观察到的结果 |
| --- | --- |
| 依赖 helper 被别名转交、内联或由另一配置包调用 | build 仍读取定义来源；只使真正相关结果失效 |
| host target 与 execution host 不同，另有设备投影 | 三种配置保持来源，已完成布局计算不重解释 |
| feature 缺失改 false；metadata 未提供改为空 | 读取结果与失败/成功边界分别保持 |
| 原文件不变但目录新增模块、根 symlink 改指向 | 发现依赖失效，不复用旧名称或实现世界 |
| 接口不变而主体变；未读字段变而接口所需字段不变 | 代码/内联失效，满足条件的接口查询可复用 |
| callback 返回 void、工具退出 0，但缺入口/输出或有错误诊断 | 任务失败，消费者拿不到成功 receipt |
| 代码 blob 完整但入口映射来自旧接口 | 发布/恢复失败，不以文件存在冒充 ABI 匹配 |
| 多文件发布中途失败，或发布成功后回复断开 | 前者没有完整 manifest；后者能核验恢复且不重放源动作 |
| 两个消费者同任务、取消其中一个；两次不纯源调用同 PC | 前者不取消仍需工作，后者不合并动态执行身份 |
| 任务取消时后端仍读 IR 或写输出 | owner 保活至访问结束；不发布仍可变化的内容 |
| cache 命中但文件损坏、输出路径指向依赖源码 | 恢复/重建或明确失败；源保护不因 cache 命中绕过 |
| 驻留坏帧后合法请求；配置环；工具完成未知 | 分别恢复协议、报告真实依赖环、禁止盲目重试 |

以上尚未执行。本轮只核对设计与源码/正式文档的对应、链接和修改边界。
schema 细节、跨编辑恢复编码、各平台进程取消及持久性 API 留给相应实现设计；
它们不能通过复用旧指针、弱化准入或把所有外部操作视为可重放来省略。
