# Elaboration、VM 与发布的状态协议

状态：目标协议设计，通用任务协议尚未落地；局部签名前提形成已有下述接线。
初始源码核对基线为 `873af90f`；第 4 节已按 `6a564ae5` 收敛当前接线，
其它节仍按各自注明的基线和范围阅读，不将历史报告当作本轮验证。
本文件细化 [全链路架构](architecture.md) 中的所有权，不新增源语言操作或等待语法。
依赖如何分类、稳定查询如何复用见 [依赖观察](dependency-observations.md)。

## 1. 先保留已有机制，而不是重新发明全部状态

| 当前入口 | 已经表达的正确边界 | 还不能承担的职责 |
| --- | --- | --- |
| `type_resolution/src/obligations.rs::Engine` | trial 独立、parent identity/revision 校验、meta watchers、settled 后提交 | 全部 elaboration 状态、宿主请求、代码 hole、候选诊断及不纯执行 |
| `comptime_interpretor/src/bytecode.rs::Frame` | PC、slots、prepared types、调用/默认值/plan 等 Waiting | 通用宿主协议、独立于当前 Interpreter 的可恢复 session |
| `comptime_interpretor/src/invocation.rs` | owned activation、前置条件／主体／后置条件进度、保存/恢复调用方、显式执行栈 | 跨稳定查询的 session 身份、发布后的语义可见范围 |
| `comptime_interpretor/src/code_session.rs::CodeSession` | 精确不可变世界绑定，代码与结果缓存分开，fresh VM | 跨世界通用迁移、活动帧持久化；不应放宽 ptr/world 检查假装支持 |
| `hir/src/definitions.rs::PreparedPublication` | prepare 后草稿冻结、drop 不发布、commit 不再校验/分配/回调 | 一般定义对所有消费者的统一准入和可见范围 |
| `generated_structs/admission_session.rs::DefinitionAdmissionSession` | struct/enum/newtype 共用 revision，foreign/stale receipt 拒绝、预付安装工作 | 活跃调用中的任意 definition 发布与整个实例的 coherence |
| `definitions/deferred_struct_publisher.rs::DeferredStructPublisher` | 首次真正请求才准备世界，未执行分支不做发布准备 | 一般宿主服务与完整定义种类，不应永久按 kind 扩张回调表 |

这些代码仍是生产实现。目标协议要接管实际调用者，不先删除失败保护，也不声称已有
Waiting 字段就完成了正式文档中的局部依赖调度。

### 1.1 签名前提形成的局部进展条件

`ready_signature_capabilities_with_clauses` 仍是局部迭代器，不是下文的通用 task
调度器。它形成显式声明的正前提；某个 atom 的 Type 投影可能依赖同一签名中另一个
尚未形成的前提。例如先写 `A.Item.Item :- Allowed`，再写 `A.Item :- Source` 和
`A :- Source`，不能因此要求声明者按求解顺序排列约束。

关联投影的给定证据查询现在保留 Unresolved 的原因，并标记当前失败仅允许在
capability 前提发生进展后重试。旧 `needs_capability` 已改名为
`retry_after_capability_progress`，避免把任意 Unknown 解释成已找到输入生产者。
Budget 与 Invalid 不获得此标记。成功形成 atom 只有增加新的前提才算本轮进展；
重复前提和只完成解析的其他谓词不能凭空触发重试。

失败尝试仍恢复该 owner 的 capability 表、PTC context 及 pending_type_uses 长度。
当本轮有新前提时再尝试保留的 atom；没有进展且签名须完成时，返回原失败，不将其
转成成功、前提不存在或默认实现。尚在准备参数前缀时保留 atom，后续真实参数供应
可再次进入。缺失、自举和循环前提不能因为队列曾有其他成功项而被接纳。

这是现有源码路径的修正，不是新的 waiter：没有给 Unknown 伪造 producer ticket，
没有改变候选世界或方法选择；通用事务、精确依赖登记与跨重试累计全部查询成本仍待
按后文实施。该循环恢复的三类状态也不能代表整个 Checker 已实现事务回退。

验证工作目录 `/home/fin/luna`，统一通过 `python3 -B development/check.py
--timeout 900 -- test ...`，result.json 保存完整命令与结果：

- `/tmp/luna-tools-1osgorhz/result.json` 退出 101，诊断跟踪确认原顺序用例在
  associated_projection → symbolic_interface_evidence → given 查询处返回
  UnvalidatedModel，直接越过原有重试逻辑。临时跟踪已删除。
- `/tmp/luna-tools-efr2exmj/result.json` 退出 0，原顺序用例通过；
  `/tmp/luna-tools-ckil_t4v/result.json` 退出 0，7 项相关 execution 回归与新增
  非法 atom 拒绝例通过。
- `/tmp/luna-tools-an8c0yq1/result.json` 退出 101：associated_signatures 9 项、
  trait_obligations 4 项和新增签名前提回归 2 项通过；首版非法 atom fixture 将整数
  传给 bool 域，在数字字面量目标检查处被拒绝，未到预期的普通类型不匹配诊断。
  已改为 Type 值传入 bool 域，拒绝预期不变，后一条记录确认通过。
- `/tmp/luna-tools-no7ne8e8/result.json` 退出 101：cmtt_type_inputs 24 项、
  deferred_associated_values 9 项、normal_input_plans 3 项与 open_normal_input_plans
  2 项通过；后两组的另外 16 项在 VM 中报 StepsExhausted。
- 将本次六个生产文件临时换为 HEAD `6bf6d3b4`，保留用户工作区修改，对照
  `/tmp/luna-tools-e0tq_55u/result.json` 退出 101，后两组仍为 5 项通过、16 项失败。
  逐项核对失败名称、位置与错误载荷相同，随后按字节及摘要恢复本次源码。

本阶段去重 61 项通过，16 项基线预算失败；没有扩大默认 VM 额度、跳过准入或修改
旧用例预期。新回归检查三层前提的六种排列、两种 Type 参数来源、缺失与自举前提、
无正前提的析取以及非法 sibling 优先报错。七个修改／新增 Rust 文件通过 scoped
rustfmt check，`/tmp/luna-tools-k40wbfcs/result.json` 退出 0；git diff --check 通过。
后续已定位并移除 runtime 闭包存储检查反复构建整包域表的成本，上述 16 项中
15 项恢复，剩余一项推进到实际执行阶段，见
[域表复用与成本测量](comptime-value-admission.md#runtime-闭包存储检查复用声明域)。
不得将该后续修复计入本节签名前提形成的验证结果。

## 2. WorldRevision、可见范围与 coherence 分开

[normalization 的开放定义规则](../docs/comptime/normalization-and-instantiation.md)
规定：loose 环境的 implementation、derivation 按线性可见性使用；整体实例化时，
生成定义与相关静态作用域的已检查定义一起接受 coherence 检查。
[包规则](../docs/code-organization/packages-and-umbrella.md)另限定合法依赖及实现世界，
不能靠减少 use 隐藏已在该世界内的冲突。

因此拟定的查询视图为：

```text
SemanticView {
  world_revision,
  lexical_scope,
  instance_identity,
  visibility_frontier,
  candidate_universe_stamp,
  phase,
  target_context
}
```

WorldRevision 描述物理数据版本；visibility_frontier 描述当前语义位置允许看见哪些
线性生成项。两者不是一个计数。物理目录中已经存在某定义，不表示较早位置或其他实例
可以使用它。静态声明的可见性也不能由“哪个检查任务先结束”决定。

候选 universe 由合法包/静态作用域、phase 和该 frontier 确定；记录完整的检索范围，
包括未命中的桶。前缀 A 内选定的证据带 A 的前提，新发布形成前缀 B，不能把 A 的缓存
换上 B 的标签后复用。对于 A 处已经合法完成的源执行，不因 B 出现而重新执行。

整体 coherence 的集合可能大于某次线性使用的集合。一次发布 receipt 证明该批在当前
输入下可安装，不证明所有未来生成项都相容。实例 owner 保留最终 coherence 义务；
未完成或失败就不能交付实例/产物，但不假装能够回滚已经发生的源程序外部行为。

静态源码编辑产生新构建输入；旧构建中的执行进度和线性 frontier 不能直接迁入新构建。
这一模型保留既有语言规则，不引入“每次最新全局世界就是正确世界”的调度依赖。

## 3. 活动状态的所有权划分

当前 `elaboration/src/program.rs::Checker` 的字段需要按如下责任拆分，不按字段名称机械移动：

| 代表字段 | 目标 owner | 发布边界 |
| --- | --- | --- |
| package、package_identities、target、protocols | WorldSnapshot / SemanticView | 稳定只读输入，不能随候选覆写 |
| signatures、requirements、defaults、checked_traits、checked_implementations | 定义生产者及其完成结果 | 已完成部分进入稳定世界，活动前缀不与其共表伪装完成 |
| active_signatures、active_constants、active_required | 请求 owner 的求值/观察栈 | 记录具体正在生产的 facet，退出后移除；不能只以 owner 相同判循环 |
| normal_input_prefixes、signature_prefixes、signature_domains | SignatureTask | 前缀可供合法观察，不能当完整签名导出 |
| locals、captures、loops、result、return_observations、effect_environment | BodyRegion / ControlFrame | 嵌套进入显式帧；退出交付结果和证据，不把临时上下文留在共享 Checker |
| context、pending_type_uses、pending_refinements、候选替换 | CandidateTrial / Region obligations | 前提、赋值、term result 与诊断决定一起提交 |
| code_values、pattern_codes、generator_cells、code_method_captures | 当前代码构造/执行 owner | 声明模板可共享，实例捕获、可变 cell 和实际 CodeValue 不能共享误用 |
| completed_aliases、open_instances、implementation_index | 带完整语义依赖的缓存/索引 | key 包含相应视图与输入；完成结果和 in-flight task 分开 |
| next_definition_identity、generated_functions、expansion_sources | DefinitionTransaction / 来源 owner | 成功发布才交付新定义；来源在失败诊断需要时仍由 owner 保活 |
| required_steps、alias_expression_budget 等 | WorkBudget 的具名子额度 | 消耗跨重试累积；失败不退款已做工作，回收另有保留容量 |

某字段同时放了稳定和活动数据时应拆其语义，不是简单包进一个更大的 Context。
struct 中没列出的字段仍需逐一归属；本表是拆分依据，不是穷尽迁移清单。

### 已接入：required invocation 的整批定义接收

VM 内完成 builder 发布，不等于调用者 Checker 已接收其描述。此前
`invoke_required_publishing` 逐个支付复制成本并写入 struct／enum／newtype 表及
PTC 构造器权限；后一个描述超预算时，早先条目已可见，implementation 布局索引
却尚未同步。由此不能把单次 VM 发布成功当作跨层事务已经完成。

现在生产入口交给 `required/publication.rs::Receiver`：先用同一调用方余额完成
全部描述的传输预检，再安装定义表、构造器权限及布局索引。失败不修改这些接收方
状态，已经消耗的工作不退款；builder 已保留的身份编号也不回退。生成描述仍先由
原 builder admission 校验，接收器不重复求值、不重新分配身份，也不创建第二套
形成规则。这里保证可返回错误的接收失败原子性，不回滚此前用户执行的外部行为。

局部回归跨多个 batch 和三种定义逐额度截断，检查已有条目、构造器权限、索引版本
及完整布局保持原样，再以足额接收同一组不可变描述；空发布零额度可完成。抽出
原逻辑但尚未调整顺序时，同一回归确实发现第一条 struct 泄漏：
`/tmp/luna-tools-n5aymb7q/result.json` 退出 101。真实 sysroot 用例通过公开 builder
组合生成三种相互引用的定义，required 调用后继续反射并经 trait 方法执行。

最终验证目录 `/home/fin/luna`：
`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -p luna --test required_definition_publication --test builder_invocations --test required_input_scopes --no-fail-fast`，
`/tmp/luna-tools-f1eem7h5/result.json` 退出 0，400 项通过。验证含已有资源分析工作区
修改，未执行全 workspace。原布局索引同步仍可能复制整个布局集合；完整成本模型、
一般 DefinitionTransaction 及待决 Type／源主体恢复没有因此完成。

### 已接入：晚生成描述交付给后续 IR

接收成功还不代表最终 NIR 包完整。初始描述快照取得之后，主体、默认值与模板中的
required invocation 仍可生成定义。原收尾只补收 struct 和内部 scalar packet enum，
遗漏普通 enum 与 newtype；局部 `const Flag = Wrapper()` 后执行
`true.as(Flag).as(bool)`，会在实例化阶段报 `IIR nominal type has no declaration`。
修复前真实 sysroot 回归 `/tmp/luna-tools-f212gyj3/result.json` 退出 101。

现在三类描述共用按声明身份去重的交付规则，保留包内先前条目，并在契约、纯度和
quote 分析前补齐 checker 已接收的描述。内部 scalar packet 只另记传输角色，
不再获得另一条 enum 描述收集路径。此处不重新调用 builder、不修改生成身份，
也不以布局代替类型身份。

成员查询另有一条一致性边界：enum 命名空间、普通／代码方法及关联项共用
`is_type_receiver` 观察接收者域；Type 可以来自局部值，不只来自声明语法。
`is_type_head` 仍负责其原有语法分类，不能借此把普通值引用折叠成 TypeValue。
域是 Type 也不代表身份已经完成：enum 仍先观察已知命名空间，真正选中成员后才走
原 required 类型形成；分组不是值，载荷仍按普通构造契约求值和检查。

旧路径的 `const Event = EventType(); Event.data(true)` 误入 struct 字段投影，
证据为 `/tmp/luna-tools-y3aii0eo/result.json`。新增命名空间基线还显示，局部 enum
别名的 `struct_fields(...)` 变体误入同名 Type 反射方法，报实参数量错误：
`/tmp/luna-tools-vv1xy062/result.json` 退出 101。共享域判断修复这两种路由差异；
带类型上下文的构造、显式成员构造与匹配均保留独立正例。quote 内待决 Type，
以及一般候选探测的生成定义所有权仍须后续处理，不能据此推定完整回滚或活动源任务
已经接通，也不能把“尚未观察到身份”当作已证明不是 enum。

验证目录 `/home/fin/luna`：
`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -p luna --test required_definition_publication --test builder_invocations --test cmtt_storage_world --test cmtt_phase_boundaries --test test_templates --test quote_functions --test inline_pattern_observation --no-fail-fast`，
`/tmp/luna-tools-wcicoi_g/result.json` 退出 0，639 项通过；新增用例经真实 sysroot
检查 newtype 转换、enum 构造与匹配，以及已有／晚生成混合描述的身份、反射和执行。
工作区保留已有资源分析修改，未执行全 workspace 或 LLVM 原生验收。

接收者域共用后的检查，目录仍为 `/home/fin/luna`，命令前缀均为
`python3 -B development/check.py --timeout 900 -- test`：

- `-p elaboration --lib -p luna --test cmtt_methods --test normal_form_methods --test enum_groups --test required_definition_publication --no-fail-fast`：
  487 项通过，`/tmp/luna-tools-3h9qge4c/result.json` 退出 0。
- `-p luna --test enum_groups --test associated_signatures --test deferred_associated_values --no-fail-fast`：
  28 项通过，`/tmp/luna-tools-11h79gqp/result.json` 退出 0；包括进一步区分载荷
  求值顺序的正例。两组有重叠，不相加为独立用例数。

## 4. Region 与 task 状态

本节在 `6a564ae5` 上核对生产接线。表中的“已有”表示真实消费者已经使用；
“目标”表示迁移必须满足的协议，不是可调用的 Rust API。历史迁移过程由 Git 保存，
本节只保留当前职责、未完成接线及足以定位的证据。

Region 的目标状态为 `Open → Closing → Complete | Failed | Cancelled`。
Closing 不接收无关请求，但允许完成已有任务必需的子请求；必检结果与语言允许的
residual 均可交付时才 Complete。空队列本身不证明完成。失败或取消回收 waiter、
候选及执行所有权，不发布成功实例，也不承诺撤销已经发生的外部效果。

任务身份为 `(RegionId, TaskSlot, Generation)`；单线程同样需要 generation，防止
槽位重用后接收旧通知。完整 Region 协议尚未实现，不能把现有 Task 枚举视为其替代。

| 状态 | 持有内容 | 下一步 |
| --- | --- | --- |
| Ready | 检查后继、输入描述及所属作用域 | 取得唯一写权后 Running |
| Running | 当前规则推进的写权 | Ready、Waiting、Resolved、Unresolved 或 Failed |
| Waiting | 后继、观察票据、真实生产者及代次 | 有效响应到达后继续，或失败／取消 |
| Resolved | 本代结果与依赖证据 | 本代终态；证据失效后另开一代 |
| Unresolved | budget、unsupported、missing-premise 等原因 | 条件实际变化后重试，或按语言规则提取 residual |
| Failed / Cancelled | 诊断来源与尚待回收责任 | 回收后终态 |

Waiting 与 Unresolved 不混用。没有就绪任务时，先区分外部请求、已失败生产者、
额度不足和未迁移规则；只有实际生产依赖闭环才能报告循环。AST 父子关系不是循环
证据。补充调度预算不补充 VM 执行预算，也不使失败的同步检查自动具备恢复能力。

### 当前生产接线

路径均相对 `compiler/`；这些入口构成接下来修改的依赖闭包。

| 入口 | 已有责任 | 未完成部分 |
| --- | --- | --- |
| `elaboration/src/program/block_regions/driver.rs` 与 `driver/region.rs` | 普通、contextual、部分结构输入共用 Region；帧栈、子请求及返回值由逐步状态持有，同步入口只负责驱动 | 仍使用原活动 Checker，没有可移交的外部等待权限；模式／guard 子检查仍同步 |
| `elaboration/src/program/branch_probes.rs` | 探测次序、默认类型策略和 Attempt 回退；if／when／match 帧持有搜索位置与当前事务 | 不提供执行前缀恢复或外部 Type 等待 |
| `elaboration/src/program/dependent_inputs/tasks.rs` | 依赖帧持有准备结果、活动路径、默认后继与待检查槽，poll/accept 交接源码子检查 | 入口仍同步驱动；required 求值、一般调用和外部 Type 响应尚未并入共同 Region |
| `elaboration/src/program/declaration_inputs/progress.rs` | producer、身份、检查次序与源码 schedule 分开 | Typed 路径仍要求普通完整 Expr，Contextual 仍保留 NormalFormArgument |
| `elaboration/src/program/dynamic_instances.rs` | callable packet 保留实际 producer、静态身份与 pending 集合 | 签名仍提及 pending 参数时直接拒绝发布 |
| `elaboration/src/program/function_bodies.rs` | 最终函数与 required 依赖共用主体检查 | 必须检查出完整 body 才能构造 NIR Function |
| `elaboration/src/program/required.rs`、`required/prepared.rs` | 完整根、输入准入、冻结声明世界和代码缓存所有权 | build 必须返回完整 Expr；没有源区域的增量根 |
| `comptime_interpretor/src/invocation/{host,session}.rs` | 同次 activation、契约进度、方法／关联／定义宿主请求 | 没有取得下一段受检源码的生产入口；Fragment 根仅供测试 |
| `comptime_interpretor/src/bytecode/bindings.rs` | 帧持有导入／导出绑定，完整验证后移动交付 | 不提供源作用域、外部控制出口或任意切点许可 |

#### Type 需求预检的独立测试模型

`627ab804` 之后的调用点核对确认：旧 Type Session 已没有前端生产消费者。
它现位于 `compiler/elaboration/test_support/type_computations/`，仅由测试显式
加载；生产 elaboration 不再导出这个模块。下述预检和签名 API 是独立协议模型，
不是普通 `defer` 已有的挂起路径；实际所有者仍见上表的 required 与源码区域。

测试模型的 `demand.rs` 对单输出、依赖恢复及同一注解的多个根共用有界预检。
计划保留依赖边、根归属、外部等待和已失败生产者；完整预检前不进入生产者。
共同依赖只执行一次，失败沿原边传播，无关兄弟可独立使用。后续根已有错误或循环
时，不能先运行较早根，再发现整体不可交付。

`resume_bounded` 只恢复指定输出，`resume_dependencies` 才驱动其生产者。
SchedulingBudget 保留等待和已交付输入，不将队列改为 Failed；循环终结相应需求。
便利入口的 100,000 步是整次调度的共同额度，非每条边的新额度；VM 额度另计。
计划只在独占推断 context 的操作内有效，不进入声明快照或 CodeSession。

签名恢复 API 的直接测试不能代替源码接线。尤其
`resume_instantiated_signature_annotation_checked` 与
`resume_runtime_signature_checked` 不会自动接管 `dynamic_instances` 的拒绝分支。

#### 控制与输入事实

`program/control.rs::Context` 拥有返回目标、部分返回上下文、待定返回点、返回观察和
循环目标。独立签名、默认值、required、quote、generator 与闭包边界整体移出／恢复
控制状态；闭包不继承外层返回目标。computation 仅替换相应返回目标，handler 保留
安装处的词法控制目标；type observation 保留所需循环可见性并隔离返回及捕获证据。

`input_facts::State` 集中保存输入类型、值、部分观察、特化、派生来源和 supplied scope。
三种退出必须分开：

- 失败或不提交的内部试探恢复完整事实 owner，不能漏掉新增字段。
- 块／迭代的词法退出删除内部绑定，保留外层值在写入／捕获之后发生的失效；恢复旧
  快照会错误地复活这些证据。展开迭代共享边界，失效跨迭代累积。
- 开放函数继承契约允许的 Type／value 绑定与 supplied scope，隔离调用者的活动
  特化／partial／派生来源；完成后归还调用者状态。

控制上下文、输入事实及部分观察的 dump 保留不同种类、源位置和转义后的名称；
不能把未完成输入只打印成一个不含归属的 pending 标志。

这些是检查事实，不是 VM 值环境。声明需求、预算和真实 invocation 效果不随事实
快照回滚。PTC 只管理类型命题；程序状态、内存及资源规格仍属于程序验证层。

#### 检查帧与探测

`block_regions::Progress` 保存受检语句前缀、语句／尾值后继、diverges 和分期模式
证据。子结果只能交给等待它的父帧；后继事实只在相应语句完成后建立。控制退出后的
源码仍按语言规则检查，但不据此增加可达路径的事实。OrdinaryTail／ContextualTail
共用绑定收尾、引用计量、混合分期物化及别名清除，分别产生 Expr／NormalFormArgument。
结果种类不能借 Any、假 Type 或未经准入的 rigid 混同。

`Destination` 按共享预算复制 Type、部分 callable 和结构输入上下文，嵌套帧共享
不可变目标；未知 Type、未知值与依赖域未定的值保持区别。语句推进、空块和只有尾值
的帧同样计费。复制深度与步数耗尽是资源错误，不变成上下文不足。

`LexicalScope` 保存父局部域、expr／pattern code、PTC context 及输入事实边界。
正常完成与失败退出恢复父绑定和前提，保留外层失效。复制局部域与边界前计费，共享
代码仅保留引用；这仍有随可见局部数量增长的复制成本，不代表完整快照已经有界化。

`delivery.rs` 的可选包装和错误结果转换是显式后继，只消费受检子结果。NoReturn
直接传递；已有 optional 保留存在性层次，错误结果保留 Embed／Success 区别。
普通 if 复用整体交付准备；它与 contextual、部分结构输入共用控制交接，却保留各自
结果检查。inline 选择、callee 位置、分支前提和 effect 交付按边界恢复。没有 else
的语句分支仍走原 Void／Never 规则；不是所有 if 都可按同一个目标推导规则合并。

contextual when 的条件与主体也进入同一帧栈。`when::Frame` 保存已检查分支、当前
条件／主体后继、整段词法边界和分支的正前提；主体退出后先恢复分支局部，再撤销
正前提，随后给后续条件安装本条件为假的事实。完成与失败都退出所属作用域，不能
把前支的绑定或正前提带入后支。生成的每个 NormalFormArgument 分支仍保留原有
Block 作用域，源码未选中的分支仍检查；这不是提前执行条件或用运行结果替代检查。
无 fallback 的 when 继续采用普通语句规则，不伪造一个 contextual 值。

普通 `when` 的条件与 arm 主体现也进入同一个 `when::Frame`，原 program.rs 的
同步分支循环已退役。帧显式保存完整目标、当前分支结果域、fallback、有序 inline
选择、父 callee 位置及 effect 交付责任。普通与 contextual 路径共用条件检查、
正／负类型前提、每个 arm 的词法边界和失败退出；没有用运行结果免除未选 arm 检查。

有 fallback 时，完成分支沿已有目标检查，缺少目标时从首个非 Never 分支取得后续域；
没有 fallback 时保留普通语句的结果丢弃和 void 尾部。连接分支前恢复原事实上下文，
不把某个 arm 的正前提带到连接点之外。inline 选择保留原有条件可用性检查与未选
分支抑制；最终选中主体的复制纳入已有预算。嵌套普通 when 经同一交付前置检查进入
父 Region，callees 仍只把交付位置传给 arm 结果，不能把条件或普通实参变成 callee。

深嵌套验收还暴露了检查后的另一条递归链：comptime 局部量的初始化信息观察会在
`observe_normal_input_in` 中为每层分支和块保留完整值构造器的宿主帧。32 层 when
已完成 Region 检查后，仍在这里栈溢出。现在 `normal/control_observations.rs` 迭代
跟随已知条件、匹配 arm 和块结果，独立维护观察用的局部绑定；值构造器只在非控制
节点进入。深度、步数、绑定复制和模式／guard 的预算保持原义，未知调用、赋值和
控制转移仍不给出值观察。这是静态证据路径的修复，不执行初始化器，也不补造结果。
模式、条件、局部初始化及值构造的子观察尚未全部帧化，不能据此声称全前端已消除
递归。参考树解释器另有 64 层宿主递归保护，深层源码验收与双引擎对照分别标明范围。

部分输入上下文的候选域搜索与最终检查现在共用 when 帧。搜索仍采用
`branch_probes::Search` 的原有次序：先寻找有类型证据的分支，再按允许的数字
默认策略重试。每个候选持有独立 Probe，在交出第一个条件前保存检查事务；其前序
条件提供负前提，本候选条件提供正前提，条件和主体都通过 Region 交还结果。
候选完成后回滚探测私有状态，只将取得的类型证据交给后续正式检查。缺少上下文
允许继续搜索；域错误、预算错误与恢复失败向外传播。

普通与 contextual match 的 selector 和主体现在都交给 `matches::Frame`；原
`pattern_checking/matches.rs` 的普通同步循环已删除，仅保留共同模式／guard 与
覆盖检查入口。帧保存受检 selector、当前模式、已完成 arms、交付目标，以及各 arm
的词法／匹配事实退出责任。模式与 guard 仍调用共同的同步规则，最终 guard 嵌在
Pattern 中，不能额外执行一次，也不能在覆盖性检查前将其丢弃。

普通交付状态位于 `block_regions/matches/ordinary.rs`，保留 inline 主语、选中
位置、待安装的物化绑定、分支结果域及父 callee/effect 交付责任。已有 inline 主语
只复制已检查的数据，不重新检查或执行 selector；相关复制和 generator_data 快照
纳入预算。每个 arm 在模式可能引入绑定前建立词法出口，在主体前保存 staged/member
绑定和 callee 位置。主体返回后撤销匹配前提再汇合结果域，离开 arm 时恢复局部与
staged 描述；对外层写入造成的输入事实失效仍按共同词法边界保留。

完整目标、先前分支的已知结果域及部分输入规则继续决定下一个 arm 的交付。
普通 match 的错误成功 payload 恢复、合成 arm 的独立清理来源、覆盖检查和行为
模式抽象保持原顺序；inline 选择仍检查所有 arms，只有选中 arm 被交付执行。
各路径在完成或失败时恢复父 inline 选择／抑制状态，不用局部成功免除覆盖或资源
义务。S-expression dump 区分 selector、当前 pattern、已完成 arms、inline 绑定
和待恢复的 scope。

部分 match 在取得一次受检 selector 后进入帧内搜索，保留 selector、搜索位置、
当前 Probe 与匹配前提。候选主体返回或失败时，先撤销匹配前提，再消费事务。
`optional_branch_context` 同步闭包循环已删除，嵌套部分输入 when／match 直接进入
同一 Region。即使候选证据补齐了 expected，帧仍保留原结构输入的错误结果交付
边界，不能在搜索完成后丢失或重复注入错误行。
模式、guard、inline 模式选择与绑定物化仍没有可移交的外部等待后继。
把 selector/主体移入 Region 不等于打通普通源码与 VM 的交织挂起，两个待定 Type
正例仍待其共同前置能力。匹配事实的进入／退出仍复用统一服务，没有另造帧专用的
类型事实规则。

普通比较的左右操作数也进入同一帧栈。`operators::peers::Progress` 保存两侧源码、
已检查代码、上下文搜索位置及默认类型策略；比较帧持有当前 Attempt，子检查结束
后消费一次，再决定是否借用另一侧类型继续检查。Never 不提供操作数域，比较结果
的 bool 上下文也不作为操作数域。词法退出先于探测回退，域错误与预算错误仍向外
传播。完成后复用原 Eq／Ord 协议选择、标量降码和 effect 交付规则。

`and/or` 的操作数现由 `logical::Frame` 保留并进入同一 Region，原同步递归与左链
单独展开循环已删除。帧先尝试左侧的既有上下文；缺少上下文才交给共同的
`operators::peers::Progress`。Never 不提供域，右侧先在无结果上下文下探测；若
右侧仍需上下文，再使用正常的 peer/default 规则，不能把整个表达式的结果域强加
给已经具有类型的右操作数。

bool 右侧在已检查左侧的相应真／假类型前提内检查，帧独占该前提的退出责任；
子检查正常完成或失败时先退出前提，完成整个结果交付或外层探测回退。BitVec
不安装 bool 前提，保留双侧求值。域选择只决定检查，不执行操作数、不改源码顺序，
块内 return 仍指向原函数，短路跳过的右侧也须通过源码检查。

普通算术现也进入同一 Region。帧持有左侧探测、右侧无上下文探测、原生域试探、
trait RHS 上下文及最终 peer 搜索的后继，拥有已检查操作数、当前 Attempt 和原生域
不相容诊断。只有原生 RHS 试探允许在事务恢复后用独立的 trait 域再检查；预算、
语义错误与恢复失败不变成候选证据。已成功检查的操作数直接交付原有协议选择／
标量构造；native 左侧不意味着 RHS 同型，关联 Output 也不等于输入域。原独立
同步算术探测流程及 peer 驱动适配器已经删除。

嵌套算术在普通 primitive 目标下直接压入共同帧栈；非 primitive 目标仍先走普通
交付适配，再进入相同算术规则。每个帧提供稳定 S-expression，保留源码位置、
阶段、等待项、已检查代码和失败诊断。检查先后仍不改变操作数执行顺序，左侧
提前退出仍使用原函数出口；右侧源码依旧检查。此迁移没有放宽身份观察的未知
效果边界，也没有修复未知前缀 Type 默认值的提前冻结问题。

一元 `-`／`not` 也由同一 Region 持有操作数探测与完成后继，原同步
`unary_expression` 检查流程已删除。帧保存协议选择、Attempt、已检查操作数、
目标域和 effect 交付责任；只对缺少上下文的探测回退，恢复 numeric defaults 后
才决定是否取得了重试依据。成功探测的操作数直接复用，关联 Output 由原协议规则
检查，不假定它与输入同型。primitive 或无目标域的嵌套一元运算直接压入共同栈；
其他目标继续先经普通交付适配，再进入同一规则。

立即负数字面量仍连同符号形成，避免先要求正幅值装入有符号目标；嵌套负号仍分别
检查，不能将 `- -1` 合并成一次字面量构造。操作数的 return 沿原函数退出，协议
探测不执行用户代码。S-expression 显示阶段、等待项、协议、事务和保留的操作数。
这一迁移不改变静态身份观察，也未补齐未知默认值所需的原 invocation continuation。

一元迁移的本轮证据（工作目录 `/home/fin/luna`，入口
`python3 -B development/check.py --timeout 900 --`）：

- `test -p elaboration --lib -p luna --test contextual_input_regions
  -- unary_ --test-threads=1`：退出 0，
  `/tmp/luna-tools-v359vkwd/result.json`；帧的 dump／重复响应拒绝及三个源码用例通过。
  源码用例在两种执行器中检查操作数次数、提前返回、探测恢复、字面量范围及嵌套分支。
- `test -p elaboration --lib -p luna --test contextual_input_regions
  --test arithmetic_contexts --test library_std_format --no-fail-fast
  -- --test-threads=1`：退出 0，`/tmp/luna-tools-zs43cah4/result.json`；
  elaboration／luna 单元、区域、算术上下文和真实 sysroot 格式扫描用例通过。
- `test -p luna --test execution --test inline_control --test library_protocols
  --no-fail-fast -- unary nested_negation --test-threads=1`：退出 0，
  `/tmp/luna-tools-3u3mrxbm/result.json`；七项既有用例覆盖关联输出、effect 边界、
  inline 求值次数、溢出／phase 检查和嵌套操作数的执行／物化限制。
- 六个修改／新增 Rust 文件 scoped rustfmt `--check` 退出 0，
  `/tmp/luna-tools-pxtjrxx8/result.json`。这些检查不证明源码—VM continuation 或
  `defer std.fmt.Parse(format)` 已实现。

这些规则仍在原活动 Checker 中同步驱动，尚未提供等待外部 Type 响应的入口，
不解除待定 callable／列表域正例的 ignore。源码状态迁移和原 VM activation
交接必须继续共同推进，不能把内部上下文试探当作已执行前缀的恢复。

算术迁移的验证目录为 `/home/fin/luna`，执行入口均为
`python3 -B development/check.py --timeout 900 --`：

- `test -p elaboration --lib -p luna --test arithmetic_contexts
  --test contextual_input_regions --no-fail-fast -- --test-threads=1`：退出 0，
  `/tmp/luna-tools-oxnachfj/result.json`。elaboration／luna 单元与算术上下文、
  共同 Region 用例通过；新增源码用例在两种执行器中检查生产者顺序、嵌套 peer
  上下文及原函数提前返回。没有解除既有 Type 等待用例的 ignore。
- `test -p luna --test execution --test library_std_format --no-fail-fast
  -- --test-threads=1 arithmetic numeric_peer_context format_plan`：退出 0，
  `/tmp/luna-tools-qku264vc/result.json`。22 项执行回归及 4 项格式扫描通过，
  包含原生／异构 RHS、关联输出、上下文 lambda、effect 交付、数值精度及溢出拒绝。
- 八个修改／新增 Rust 文件的 scoped rustfmt 检查退出 0，
  `/tmp/luna-tools-x1cm_fiz/result.json`。没有运行全 workspace 或性能基准。

检查驱动的帧栈与返回值已从局部循环移入 `driver::region::Region`。一次正常 poll
完成一个帧转换，返回 Continue、子表达式请求或完整结果；尚未迁移的原子表达式
仍由同步适配器调用同一 `Destination::check_expression`，然后把结果交回原 Region。
父帧、目标域、已检查分支和退出责任在两次 poll 之间保持原所有权，不复制活动树。

子请求发出后为 Waiting，保留原源码位置；只有该状态接受一个结果。重复结果、
无请求交付与结束后交付均被拒绝，原已接收结果或错误保留。失败作为数据交回原父帧；
下一步先退出内部词法帧，再由最近的既有 Attempt 决定能否恢复搜索。交接本身不重发
已完成子请求；Probe 是否尝试另一上下文仍由原搜索规则决定。域错误或预算不足不能
转成缺少上下文。最终完成或不可恢复失败都会终止 Region。
Region 与推进结果提供 S-expression dump，保留嵌套帧、返回值和错误位置。

这个私有协议当前仅由同一同步适配器消费，不是跨世界消息端口。HIR 和 Checker 的
活动状态仍由原调用保活，尚无完整源区域 owner、取消接口或外部 Type 响应票据。
因此不能把 Region 放入 CodeSession，也不能离开当前词法环境后给它灌入任意响应。
后续须把子规则、活动 Checker 所有权、原 VM activation 和响应准入一并接通。

本次 Region 拆分的检查（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p luna --test contextual_input_regions --test control_boundaries --test comparison_resources --test contextual_spreads --test declaration_input_plans --test prepared_input_dependencies --no-fail-fast -- --test-threads=1`：
  `/tmp/luna-tools-ak238bls/result.json` 退出 0，77 项通过。包含原有源码顺序与次数、
  嵌套返回、match guard、分支事实、Probe 恢复、错误域拒绝与默认供应检查。
  这组对应 Region 拆分后、为 Waiting 增补源码位置及新增 dump/交付单测之前的版本。
- 增补后运行 `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -- block_regions --test-threads=1`：
  `/tmp/luna-tools-_tnhf93p/result.json` 退出 0，10 项通过。覆盖嵌套帧与转义 dump、
  重复交付保留原结果／错误，以及无请求或结束后交付拒绝。
- 最终版本经同一入口运行 `test -p luna --test contextual_input_regions -- partial_probe_recovery_does_not_hide_errors_or_publish_branch_locals --exact --test-threads=1`：
  `/tmp/luna-tools-d1hcienv/result.json` 退出 0，确认探测失败不隐藏错误或泄漏分支绑定。
  已修改 Rust 的定向 rustfmt 检查退出 0（`/tmp/luna-tools-nxl043_c/result.json`）。
  未执行全 workspace 检查。

部分 optional／error 输入的 `Search` 先遍历实际类型证据，再允许数字默认类型；
Never 和只有错误出口的结果不提供成功域证据。`Probe` 保留 Attempt 与父探测模式；
条件帧保存搜索位置及当前 Probe，不在移动阶段状态时丢弃恢复责任。探测后仍正式
检查所有应检查的分支，不能因为本次执行不会选中某支就免检。

`checking_attempts::Attempt` 不可 Clone、不借用 Checker，必须由原 owner 完成一次。
它恢复控制／捕获、局部与 code／pattern 绑定、alias、输入事实、字段默认值和待发布
Type use；生成器存储与 quote 构造状态另有检查点。`OwnedGraphCheckpoint` 保留原
cell 身份和 owned graph，使用既有预算，不调用源语言 Copy／Clone，不扩展阶段准入。

子检查失败先退出内部词法帧，再交给最近的 Probe。只有 needs-context 允许继续搜索；
域错误、预算耗尽和恢复失败继续向外传播，外层 Probe 也必须清理。已经消耗的预算不
退款。Commit、Probe、RetainCode 三种完成策略继续保留，不能让成功的试探提交构造
副作用，也不能把真正保留的 quote 构造当作失败试探丢掉。

**当前帧不是外部等待接口。** 保存父环境不等于拥有当前 Checker；同步驱动中的 `?`
仍可终止整次检查。需要恢复源执行时，不能把 needs-context 捕获后从根重试。

#### 输入路由与域依赖

`input_routing::{Entry, Input, Plan, Cursor}` 只处理公开入口及供应到哪个槽。
它不持有默认表达式、参数域或执行许可。普通实参、声明／Type 子句及相关 CMTT
静态提示共用该路由；`arguments.rs`、`ClauseInputRouting` 只适配源码和诊断。
已删除 `type_input_slot`，不再为 NF collector 的名字保留另一套槽选择。

具名完整供应先预留槽位，实际到达另记；不能据此提前执行该值。固定包不借收集器
吞掉多余字段。未知剩余 arity 不能当作零或猜测包宽度；失败不消费当前位置。空收集
与未供应的默认输入保持区别。静态提示不适用可返回 None，但不能吞掉域、预算、形成
义务或 invocation 错误。跨函数值传送公开入口仍未完成，见
[callable 接口](callable-input-interfaces.md)，不能从可见实现猜测参数名。

声明 `Progress` 保存 producer 与可选静态身份，检查完成次序与源码 schedule 分开：
显式输入按源码求值，省略输入随后按声明顺序。后序输入先完成检查不等于先执行；
完整计划才可交付，重复／越界／预算失败不能留下半份身份前缀。prepared_bindings
在记录成功后更新，不在轮到源码位置时再克隆和重观察已保存的 producer。

`declaration_inputs::preparation::Frame` 现在拥有一次完整准备的路由、检查与调用者
环境退出责任。`routing::Frame` 保留原始源码位置、展开包子检查与 preparation 桶；
布局提示只决定位置，不执行 producer，也不替代包表达式的检查。包完成后才记录
字段绑定、静态观察和执行位置；空包与提前返回的包同样保留在源码 schedule 中。
现存字面量列表展开改为同一 scope 内的布局重算，不递归启动另一轮输入准备。
这没有推广现存 List collector 限制，也没有增设列表专用语言语义。

路由完成后，准备帧按所有权转入 `declaration_inputs::slots::Frame`，由后者推进
收集准备与声明槽位检查。它保留当前域及 pending 状态、原始子表达式、准备计划
和默认值 guard；普通与 contextual
子检查由共用的 `declaration_inputs::request::Request` 交回结果；域依赖与收集
子检查也使用该请求；普通期望类型、部分输入上下文与 contextual 检查是互斥的
请求模式，展开包使用相同的子请求返回入口。已检查的展开字段与收集项直接移入
Progress，`RoutedDeclarationInput` 不再可 Clone，不为交付重新复制整段 producer 代码。
这条路径同时服务普通 callable、NF、名义输入与透明别名，原槽位同步循环已退役。

依赖域检查由 `declaration_inputs::domain::Frame` 保留 consumer 槽位、传递依赖列表
及游标，只交付需要准备的槽位和接收域。显式输入、默认供应和收集结果都回到
`slots::Frame::check_slot`；原来只处理 Source／Prepared 的第二套依赖表达式检查已
删除。依赖默认值时同时准备其前序声明绑定，包含默认值链自身的域依赖；完成后
继续原 consumer，不重新从 schedule 选择目标，不重新检查已记录的 producer。
收集器的域准备也进入这个输入驱动；独立的同步 domain adapter 与
`declaration_input_domain` 已删除。依赖遍历复用有预算的
`input_observation::term_inputs`，保留重复身份 sort 一致性检查，不为枚举 rigid
复制类型树。这里得到的仍是静态观察，不能为取得 Type 提前执行实参。

检查顺序可先准备提供域证据的默认代码，例如 `Tagged<.T: Type = u8, value: T>`
中的 T；这样未调用定义里的 bool 实参和越界字面量也必须被拒绝。默认代码仍留在
显式输入之后的原执行位置，显式覆盖和提前退出不运行它。需要安装默认供应环境
时，由该槽位持有并恢复自己的 `scope::Scope`，然后才检查后面的显式源码；不能
等整个输入包完成才恢复 caller，否则选定父声明的绑定会污染后续实参。

`collections::Frame` 保留接收域、元素位置、连续尾部边界、已收集结果和各源码
位置的 preparation。每个元素按原规则请求普通检查或 contextual 检查；非尾部
待定域先尝试普通检查时，由帧持有 `checking_attempts::Attempt`。子检查返回后
消费同一退出责任，只有缺少上下文才进入 contextual 回退；类型与预算错误不能
被吞掉。普通 producer 只在原源码位置建立一次绑定，空展开的效果仍保留。
收集结果与 preparations 完成后一次移交槽位检查，原收集同步循环已退役。

`constants::PreparedObservations` 独占本次输入事实的临时安装／退出，完整安装前
的预算失败也须撤销已安装部分。退出只恢复本包涉及的 partial 与 immutable 项，
不恢复整张表、复活子检查已失效的外围存储事实，且不退还已消费的预算。
普通子检查结束或失败后，先退出观察与本帧默认值 guard，再观察并记录结果身份。

调用者环境由准备帧持有的 `scope::Scope` 保存。布局创建失败、包检查失败、
槽位检查失败与正常完成均恢复同一调用者环境；槽位退出先处理临时观察和默认 guard。
恢复范围保持原有词法边界，不恢复 input_facts 或存储，不退还已用预算。
准备帧不再借用调用函数栈上的参数望远镜或默认值环境。望远镜从消费者移入当前
准备，与完成消费者共享；路由完成后原数据转交槽位检查，不复制整组域。
默认提供者通过 `environment::Environment` 持有独立绑定表，绑定的不可变 Term
与 PTC context 继续共享。显式输入仍在 caller 环境检查，默认值才安装选定父项。
这些数据只属于当前活动准备，不是稳定缓存或跨世界发布；HIR 仍借用原源码 owner。

环境保留、局部类型和 caller 输入域的复制现在消耗同一检查预算。先准备完整副本，
再安装默认环境或改变 caller scope；失败不留下部分安装，也不退还已经消耗的工作。
深度耗尽与步数耗尽保留区别，不能从仍有 fuel 推断复制成功。域迭代逐项收费后
增加容量，不根据未获预算的完整 size hint 预分配。caller 域先按原优先级借用，
保留 open annotation 中未知槽位的位置，再由接收者决定复制策略；不是把未知域
截断为已知前缀。这里仍然复制词法表，尚未改为增量环境，亦未预算化外围所有消费者。

准备帧还拥有完成消费者 `Completion`，由消费者确定 contextual／typed 检查模式和
交付类型。typed 计划转换不再借用外层临时望远镜；callable 消费者保留完整声明域、
词法前缀与参数身份，在槽位准备完成后依次进行形成检查、最终实参域校验及 packet
组装。先退出本包观察、默认 guard 和 caller scope，才运行完成消费者；完成失败
不能交付半个 packet，也不重复恢复旧环境覆盖完成阶段产生的合法失效。

callable 默认环境直接经有预算的 `Environment::retain_prefix` 保留一次，前缀只
覆盖选定声明的默认提供者，显式源码仍在调用者环境检查。前缀覆盖移除同身份旧的
value/default 绑定，未覆盖项保留；失败不改原表。完成消费者保留的域与前缀、形成
证据及 packet 类型复制同样使用检查预算。该消费者及准备帧的 dump 包含尚未消费
的完成步骤。形成检查仍是同步操作，这项迁移没有把 pending 身份变成真实 VM 值，
也没有允许尚依赖 invocation 输入的 callable 签名发布。

声明 NF 应用现由 `block_regions::normal_applications::Frame` 持有同一个输入准备，
子请求交给已有 Region 栈；嵌套直接声明头和已绑定的常量 NF 头可继续在该栈中推进，
与块、分支、比较及逻辑表达式交接，不再经 `preparation::drive` 同步递归检查这些
参数。原 `planned_normal_expression` 已删除，形成义务、期望域、NF 头与执行计划
由准备帧持有的完成消费者统一交付。字面量包展开使用共同路由器，不再递归重启
NF 准备。`begin` 是同步与区域消费者共同的有预算准入入口。

每个子结果仍区分普通 Expr 与 contextual 输入。整段声明输入从原有 depth 0 开始，
嵌套块／分支保持既有深度规则；不是通过多一层适配无意缩短允许的输入结构。
失败先退出子区域，再送回输入准备中的收集器探测；没有待决子请求时保留原始
形成／域错误，不能用“无等待者”覆盖诊断。完成和失败都退出原 caller scope。
带捕获声明的供应、生成器数据处理及非 primitive 交付适配仍先经过原有前置规则，
不会因源头看起来是 NF 就跳过它们。

普通 comptime callable 的子句准备现也进入这个 Region 栈。原先
`comptime_function_instance` 同步调用整个输入驱动，再在宿主栈上保留签名收尾；
现在 `InstanceCompletion` 拥有原表达式来源、声明 packet 完成步骤、签名、期望类型
及函数值交付责任。参数准备完成并退出调用者环境边界后，才消费这一完成步骤。
旧同步入口只驱动同一个区域帧；不保留第二套函数实例组装逻辑。

opaque NF 与普通 callable 共享 `declaration_applications::Frame` 的子请求、
结果交回、探测失败处理和 scope 退出，完成规则仍分别检查各自的形成语义。
直接声明的嵌套 callable 应用可留在同一检查栈，包含 comptime 函数类型作为参数域
的情况；这些域仍经过普通 delivery 前置规则，不跳过 optional／error 适配。
声明的词法前缀仍由已有入口取得，显式输入继续在 caller 环境检查，默认值在其
提供者环境检查；方法与其他未迁移入口仍先走已有解析，再驱动共同准备。

源码回归新增 16 层 callable 输入、嵌套写入、提前 return、错误内层参数与外层
同名绑定保留，使用真实 sysroot，并由两种 VM 引擎检查最终值及函数进入次序。
这项迁移只保留检查后继：一般普通实参准备、源码 continuation 与原 VM 片段交接
仍未完成。尤其 pending Type 影响最终签名时仍拒绝交付，不能提前冻结或执行
producer 来使此边界看似完成。

验证以 `38f79806` 加本次改动为基线，工作目录 `/home/fin/luna`。
`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -p luna
--test ordinary_callable_types --test type_input_execution_order
--test ordinary_clause_collectors --test staged_callable_freezing --test default_calls
--test ordinary_input_identity --test library_std_format --no-fail-fast -- --test-threads=1`
退出 0，报告 `/tmp/luna-tools-_pyiz32b/result.json`。随后补齐捕获／delivery 前置
边界，执行 `python3 -B development/check.py --timeout 900 -- test -p luna
--test contextual_input_regions --test ordinary_callable_types -- callable_clause_regions
declaration_default early_default_domain sibling_callable_defaults --test-threads=1`，
两组共 10 项通过，报告 `/tmp/luna-tools-zpf_oc6x/result.json` 退出 0。
输出锚点均为各组 `test result: ok`；已知 ignored 正例未改成通过断言。
修改文件的 rustfmt 报告为 `/tmp/luna-tools-c3qruebj/result.json` 和最后调整后的
`/tmp/luna-tools-da_mecsa/result.json`，均退出 0；未修改正式 docs 或工作区原有文件。

路由、收集、依赖域与槽位已有共同子检查后继，帧仍借用同一 Checker 与 HIR。
布局提示扫描、其他表达式种类及非 NF 消费者仍有同步适配；新区域帧没有 VM 响应
收据、可迁移 Checker 或待定 Type 的执行入口。失败终止本次准备，不能把保留的
检查位置称为已支持外部补值／补预算恢复。

本次 NF 区域接线的验证工作目录为 `/home/fin/luna`，入口均为
`python3 -B development/check.py --timeout 900 -- test`。
`-p elaboration --lib -p luna --test normal_input_plans --test open_normal_form_inputs --test contextual_input_regions --test nesting --no-fail-fast -- nested_normal normal_input_plans open_normal_ declaration_inputs block_regions collection_ --test-threads=1`
退出 0，52 项通过（`/tmp/luna-tools-020eo78x/result.json`）；其中
`contextual_input_regions` 在该筛选下没有选中测试，随后单独包含完整组。
32 层混合 NF／块／if／when 用例使用独立子进程的默认线程栈检查 AST、HIR、NIR
及具体 IIR 执行；较浅嵌套核对两引擎的原 NF 头和每个生产者的调用次数。
最初新拒绝 fixture 把 requires 写在 where 之外，按语法修正后保留原错误域／形成
拒绝要求，未改生产准入来容纳错误语法。

扩大到 `-p luna --test normal_input_plans --test contextual_input_regions --test captured_input_observations --test cmtt_type_inputs --test normal_form_domains --test open_normal_form_inputs --no-fail-fast -- --test-threads=1`
得到 105 项通过、1 项失败（`/tmp/luna-tools-r05jnofr/result.json`，退出 101）。
失败是 `immutable_structural_type_evidence_survives_generator_update_and_guard_frames`
的手工 tuple-only 环境缺少跨阶段 Splice 协议。临时采用 `71981104` 的对应九个
生产文件、以 `-p luna --test cmtt_type_inputs -- immutable_structural_type_evidence_survives_generator_update_and_guard_frames --exact --test-threads=1`
复现相同诊断（`/tmp/luna-tools-2p2hdq1w/result.json`，退出 101），随后逐字节恢复
本次文件。这是既有测试环境缺口，不是区域接线引入的行为变化。该用例现改用真实
sysroot、`meta.List` 与实际 Splice 实现，四组原行为断言均保持；同命令复核退出 0
（`/tmp/luna-tools-l5750u8o/result.json`）。以上去重后 143 项通过；未运行全量测试。
修改的十三个 Rust 文件定向 rustfmt `--check` 退出 0
（`/tmp/luna-tools-d19oop7j/result.json`），`git diff --check` 通过。

pending 根据**原始 callee 域**中依赖本次尚未供应参数的交集判断，再在 caller 域中
检查实际值。含 rigid 不自动 pending；调用者的合法泛型域不必闭合。依赖在替换前
识别，避免递归／换参时 caller 与 callee 同号声明导致二次误判；观察结果不重复替换。
trait 参数与普通输入共用该原则。符号、元组／列表值先走普通表达式及上下文转换，
再做身份观察或 required invocation，不重新解析并执行另一份参数 AST。

`dependent_inputs/tasks.rs` 保留参数域、默认值引用和完成后继；共享依赖复用已检查
证据，转发默认值保留 Local 读取，不复制生产者。只有再次进入当前活动路径才是环，
不把无环 64 层以上依赖叫循环。推进和复制用共同额度；整个同步准备失败仍终止，
不能把“不消费下一个待办项”说成已经支持外部补预算恢复。

普通调用现在用同一个依赖帧持有整次 `PreparedInputs`，不再逐个需求进入一个拥有
局部结果的同步任务循环。`poll` 在 Ready 状态推进参数域／默认值依赖，遇到实际
源码输入时返回带槽位、源表达式和预期域的 Request；父帧保留当前路径及剩余默认
后继。同步入口用原 staged 检查规则处理子表达式，恢复调用者阶段后交给 `accept`。
已检查值和静态身份各自保留，默认转发仍引用原输入，检查次序不改变源码求值次序。

错槽响应不消费等待状态；接受匹配响应时先关闭再次接收的权限，只有身份观察和
路径完成都成功才回到 Ready。子检查／观察失败使帧终止，不能把同一源表达式作为
新任务重试。`finish` 只在所有任务完成后交付准备结果。Frame 和 Request 都提供
S-expression，分别显示保留的生产者／身份／默认值与本次子需求。帧仍借用当前
参数路由和 HIR，源码子检查仍在同一 Checker 中完成；没有跨 owner 的响应权限，
也没有借此实现 VM 暂停、补预算恢复或一般普通调用的全部规则帧化。

此前验证发现一个真实断点：三级普通 Type 默认值转发中，最后一项
为 `do { mark(); U }`，实际调用轨迹没有执行 `mark`。未知前缀使输入身份观察失效，
原 `freeze_dependency_input` 回退会执行 required 求值并替换生产者；这个回退
不等于在原调用中暂停并取得 Type。不能通过放宽未知效果的事实保留规则来修补。
合法用例保存在 `ordinary_input_identity.rs` 的
`dependent_default_unknown_prefix_runs_in_the_original_invocation`，保留两种执行器、
重复 invocation 的原次数预期，并明确 ignored 等待源码 continuation。
另一个纯转发依赖链用例将普通默认调用放在非 Type 依赖槽，独立验证已支持路径。

现在普通调用依赖帧在观察失败后检查被调用者的 callable phase：comptime callable
的输入属于将来那次源 invocation，不能在类型检查的依赖次序中另开 invocation。
这条路径明确报告缺少 source execution continuation，不执行或冻结输入；Type
字面量、已有合法身份与可以静态观察的结果仍保留原生产者。判断来自被调用者类型，
不来自 `in_required_comptime`：检查 required 根引用的函数体也没有执行该函数的
权限。runtime 正文的局部 `const` 所建立的 required 求值边界仍然有效；comptime
正文的数据 `const` 属于每次 invocation 的绑定，不能据关键字声称其已经求值。
局部读取也进入共同的身份观察器，读取已有不可变绑定事实并保留 Local；未知初始化、
可变绑定和失效事实不会因此成为已完成值。此前对所有 Local 跳过观察的路径已移除，
不再为取得现有局部身份而求值。

这是停止错误执行的阶段，不是待定 Type 调用已经可用：上述正例继续保留原预期和
ignore，临时诊断的回归另行检查。输入检查必须能等待原 invocation，接收实际值后
再继续域、能力与后续参数检查；未知不自动成为 Any、fresh rigid 或已完成值。
本阶段仅覆盖已经形成 callable 的普通调用依赖帧，不宣称开放函数的其他 required
准备路径已经统一，也不改变 runtime callable 的 staged 输入交付。

停止错误执行的定向验证在 `/home/fin/luna`，通过
`python3 -B development/check.py --timeout 900 -- test -p luna --test ordinary_input_identity --no-fail-fast -- --test-threads=1`：
`/tmp/luna-tools-uj9hqcbi/result.json` 退出 0，18 项通过，原未知默认值正例仍 ignored。
直接／存储的 callee、提前 return 后的未执行 Type 生产者均在缺少恢复处停止；
runtime const、不可变局部身份、失效事实拒绝及原次数／顺序断言通过。
初轮新 fixture 混淆了两阶段 const 并使用了未声明的具名入口，已按现行契约修正。

以同一入口运行 `source_input_dependencies`、`prepared_input_dependencies`、
`ordinary_dependency_tasks`，前两组共 6 项通过，后两项在 VM 构造时
`UnmaterializedDispatch`（`/tmp/luna-tools-40fhlius/result.json`，退出 101）。仅把
本次生产文件换回 `36773353` 后复现相同失败
（`/tmp/luna-tools-isdqihh3/result.json`，退出 101），随后逐字节恢复。
默认链用例现使用已有 `sysroot::with_comptime` 的源码查询服务及统一语义测试预算，
保持两种引擎、72 层依赖、调用次数、顺序与拒绝断言；该组复核退出 0
（`/tmp/luna-tools-mcc4581j/result.json`）。这不是预算性能验收，未提高生产默认额度。
三项 Rust 文件的定向格式检查退出 0，报告为
`/tmp/luna-tools-c3dig1cb/result.json` 与 `/tmp/luna-tools-6mqbqc5h/result.json`。
没有运行全 workspace，也不把临时诊断通过计为待定 Type 正例完成。

取证目录 `/home/fin/luna`：新未知前缀用例在帧化实现上失败于 `mark` 实际 0 次、
预期 1 次（`/tmp/luna-tools-48surjz6/result.json`，退出 101）。只将本次两个实现
文件临时换为 `70a458e1` 内容，保持用例不变，复现相同失败
（`/tmp/luna-tools-rld212zj/result.json`，退出 101），随后恢复本次实现；没有改动
他人的工作区文件。帧协议／dump 局部测试退出 0，
`/tmp/luna-tools-37hqg2jp/result.json`。最初编译因迁移漏改一个 Checker 实参而
失败（`/tmp/luna-tools-tktyptf4/result.json`），修正的是宿主代码类型错误。

最终检查在同一目录执行
`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -p luna --test ordinary_input_identity --test default_calls --test source_input_dependencies --test prepared_input_dependencies --test staged_variadic_inputs --test extended_calls --test library_protocols --test library_std_format --test open_functions --no-fail-fast -- --test-threads=1`，
退出 0（`/tmp/luna-tools-k6n38naf/result.json`）。新纯转发链用例通过；上述未知
前缀用例明确 ignored，不计为通过。依赖帧、默认供应、源码次序、扩展调用、真实
sysroot 格式扫描等已有路径没有发现回归；这些结果不证明目标变参 println 已接通。
三个修改的 Rust 文件格式检查退出 0（`/tmp/luna-tools-7kns5c70/result.json`）。

#### required 世界与绑定交接

`required/dependencies.rs` 使用 `nir::visit` 发现函数、trait 方法与关联值依赖，覆盖
根、闭包、模式、默认值、契约及保留的 contextual 节点，完整扫描后才发布集合。
允许保留不含表达式子项的 CodeEvidence 叶用于发现依赖，不授予它执行许可。
自由 LocalId 在 initializer substitution 后观察一次，仅在后续结构不变的本根中
复用；不是跨根缓存，也不允许先捕获所有外围值。

帧的 `Exports` 在创建请求与实际开始时检查名字冲突；解析到该 Program 的寄存器，
预算与全部检查通过后才移动入口环境。Frame 自己持有绑定交接，正常值出口要求已
终结、无 pending 请求、内部模式作用域闭合、所有输出有值且无环境冲突。先整体验证，
再移动任何新输出。错误或非局部控制不发布新绑定，但归还实际已变更的原输入。

这些是生产输入准备计划已有能力，仍缺源 scope、域准入和执行 receipt。Program
寄存器编号是片段私有的，不能直接把 slots／PatternScope 搬到另一 Program。
`restore_environment` 会退出帧内部作用域，不能用作任意源码暂停点。

### 源检查与执行的交接目标

以下协议尚未实现。它接管上述真实入口；不增加第二套可执行 HIR、不扩大
NormalFormArgument，也不把只有手工构造者的 fragment API 当作源码交付。

#### 父区域与待形成 callee 分开

`identity<Byte(), true>(9)` 等待的是 callee 的签名，但执行 Byte 的 owner 是
**调用者当前源区域**，不是尚未形成的 identity invocation。若先建立 identity 的
activation 来取得其自己的参数域，就已经绕过了调用准入。`count` 的输入域等待也
发生在其调用准备阶段；count 主体、入口快照和程序前置条件不能因此提前开始。

required 根没有已进入的源函数时，由本次必需计算建立根源区域，持有合法的入口
绑定与结果交付目标。函数体内部则由已经进入的调用持有该区域，返回仍指向原函数。
二者都不能为每段生成一个便利函数，再把多次 invocation 拼成“同一次执行”。

定义检查、必需计算和已经进入的 invocation 是三种许可来源，不由一个 comptime
标记推导。normalization 可以执行当时要求完成且输入具备的计算；未调用定义和未选
路径仍做可做的检查，不能擅自执行普通主体副作用。ordinary／loose 与执行阶段是
两条轴，遵循[正式规则](../docs/comptime/normalization-and-instantiation.md)。
合法 defer 的 Any／refinement 规则继续成立；这里禁止的是用占位 Any 谎报已完成域。

#### 所有权与规则推进

| 对象 | 所有者与可交付内容 | 禁止隐含携带 |
| --- | --- | --- |
| 稳定源码模板 | 源码服务；声明身份、owning snapshot、接口及可保留义务 | 当前 Checker、活动 HIR 借用、VM cell |
| 活动检查区域 | elaboration 会话；HIR 引用、词法／phase、规则后继、子结果和候选 | 克隆的另一套 VM 值环境、全局 AST 地址等待表 |
| 执行区域 | 原 VM 驱动；activation 身份、存储、执行位置及控制目标 | Checker 或可任意修改的宿主寄存器表 |
| 片段许可 | 指定区域／代次／执行位置、完成的域及前提 | 仅凭 HirId 或 pure／Type 结果签名取得的执行权 |
| 受检片段 | 普通 NIR／字节码、输入输出绑定、控制接口和声明世界证据 | 未检查节点、活动候选、waiter 或假类型 |
| 观察响应 | 原区域结果、接收世界／代次、Type 准入与发布许可 | 无来源 Term 赋值、可变值的永久静态身份 |

规则后继保存算法进度，引用原 HIR 与已检查子结果，不复制出另一份求值树：

```text
CheckTask(region, node, scope, target-observation, rule-state)
  -> NeedChild(task, result-slot, continuation)
   | NeedObservation(key, generation, continuation)
   | Checked(result-slot, code, domain-evidence)
   | Failed(diagnostic)
```

区域、规则后继、票据与片段交接均须有稳定的 S-expression Display，展示 source、
region／activation、代次、当前执行位置、待定观察、受检子结果和未退出 scope。
字符串与符号正确转义，不以宿主地址区分两次执行；需要世界才能展开时提供携带
上下文的 Display 适配器。dump 不是缓存键，也不赋予活动状态持久化能力。

Checked 不表示已执行。同步兼容入口驱动同一规则到完成；未迁移的叶规则允许暂时
同步调用，但遇到真实外部等待时必须报告接线缺口，不能重试整个根或改走 HIR 解释。
词法状态在借用服务时安装、步骤退出时归还；等待保留当前 scope，真正离开时才裁剪。

首次状态拆分须覆盖 locals、immutable、input facts、alias／default bindings、
control、captures、phase、numeric defaults、callee 交付及 pending Type uses。
声明服务和已提交目录按世界观察，active_required／active_signatures 属于活动任务。
PTC／candidate 记录所属前提；generator cell 与 VM slot 的实际内容仍归执行 owner，
不能靠克隆与恢复 Checker 合并真实值更新。

#### 检查依赖与执行次序

NeedDomain、NeedValue、NeedCheckedCode 表示信息依赖；AfterExecution、SelectedBy
表示次序和控制许可。调度器可推进已满足检查依赖的任务，VM 只执行许可及前序均
满足的片段。后实参的检查证据可帮助前 lambda，不代表后实参可先于 callee 求值。

```flurry
comptime fn Byte() -> Type { u8 }

comptime fn count() -> Integer
where T, ...values: meta.List<T>
{
    values.length()
}

comptime fn check() -> bool {
    var trace: Integer = 0;
    let size = count<
        .values = do { trace = trace * 10 + 1; [1, 255] },
        do { trace = trace * 10 + 2; Byte() }
    >();
    size == 2 and trace == 12
}
```

这是验收目标，尚未通过。共同检查规则先交付第一处写入，列表组装保留域依赖；
原区域完成写入后，才允许第二处写入与 Byte。实际 Type 经准入恢复元素范围检查和
组装，全部输入及形成条件完成后才进入 count。不是 List／Tuple 语法白名单允许跳过
源序；列表元素若含需上下文选择的调用，须保留相应执行依赖，不能套用这一时序。

已经接通的较窄路径是：前项自身可形成完整受检表达式，例如在同一 `do` 内声明
`let packet: meta.List<u8> = [1, 255];` 并返回 packet，后项才执行 Byte。
`CallableCompletion` 在路由完成时区分已知域和仍依赖本次生产者的域：已知域继续
严格检查；待定域只提前拒绝已证明的类型冲突，不把尚未执行的 Type rigid 当作
完整参数。未证明冲突不构成相等或准入证据；预算耗尽和无效命题仍报错。

comptime callable 由原 VM 在所有实际参数齐备后替换声明望远镜并检查准入，成功后
才形成函数值、进入主体。runtime callable 的 required 输入消费者在原 packet
执行后，用完整实参替换声明域，与 packet 保留的受检类型比较，然后才交付实例形成。
后一路径原来只检查形成条件；放开待定域的提前相等检查后，必须在此补齐最终域
检查，否则会引入漏检。本次一并接通，不重跑生产者。
这不是一般源码等待／恢复机制：需要 Type 上下文才能检查的列表字面量，以及依赖
该 Type 才能形成的函数签名，仍未接通。

源码回归集中在
[`type_input_execution_order`](../compiler/luna/tests/type_input_execution_order.rs)：
直接观察 Type 的对照、已受检 packet 的前置写入与旧值快照、错误输入在主体前拒绝、
同一 VM 内失败后恢复，以及 required runtime callable 的正反例。comptime 路径
在两种执行引擎中重复调用并检查函数进入次序。三个尚未实现的源码等待正例保留原
成功断言并显式 ignored；不能把改写成显式类型 packet 后的成功当作它们已完成。

验证基线为 `f0abe655` 加本次限定改动，工作目录 `/home/fin/luna`。
`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -p luna
--test type_input_execution_order --test ordinary_callable_types
--test ordinary_clause_collectors --test source_input_dependencies
--test prepared_input_dependencies --test staged_callable_freezing --test default_calls
--test library_std_format --no-fail-fast -- --test-threads=1` 退出 0；报告
`/tmp/luna-tools-qggsb50x/result.json`，输出锚点为各测试组的 `test result: ok`。
本次新增组 8 项通过、3 项明确未实现并 ignored；其他组也保留原有 ignored，未将其
计作成功。修改的三个 Rust 文件经 rustfmt，报告
`/tmp/luna-tools-86h0ck1m/result.json` 退出 0。工作区已有修改未纳入本次提交。

较早输入中的读取、调用、失败检查、所有权交付和域相关转换均占有执行位置。
读取可变字段后等待域，必须保留读取时依法取得的值；不能在后项写入后重读，也不能
凭 pure 标记取消顺序边。没有合法读取／保留规则就继续等待，真实闭环与未实现规则
分别诊断。

对 identity，Byte 在父区域的 callee 位置执行一次；响应交给本次 T，producer 的
执行 receipt 标记已消费。形成 `comptime fn(u8) -> u8` 后才以 u8 检查 9，不能再由
`ComptimeFunctionInstance.subject` 重放该 packet。签名、输入或形成检查失败时不
发布 callable，也不把错误推给一个已开始执行的 identity 来补救。

#### 片段边界、控制与取消

切点先限定为片段内部模式作用域已闭合的位置；源块外层 scope 由原执行区域保留。
guard 或子调用中等待时保留原 Frame，不调用 restore_environment。交接表包含
LocalId、域、访问／移动方式、输出绑定及其源 scope；先验证重复输出、错域、错 scope
和过期位置，再移动。未初始化、已移走、活跃值引用保持不同状态。

现有 compile_root 仅能将本段内部循环降为 Jump；外部 break／continue 不能凭原
loop id 穿过 Program。目标协议显式声明外部控制目标与需退出的 scope，由原区域
接收；未经声明的目标在片段准入处拒绝。return 保留 Flow::Return 并交给原函数，
不能由合成包装吞成 Value。effect resume 也不因具有一个循环目标而获准。

请求沿 Authority／Ticket 校验区域、代次与执行位置。同一 HirId 的递归调用和循环
迭代具有不同执行身份。重复、跨区域、旧代响应拒绝时保留当前 waiter；预算不足不
重置执行位置，不自动无限重试。取消不执行后继、默认值或退出契约，已发生的外部
效果不回滚。内部 Probe 不是源执行事务。

### 声明接口与主体发布目标

NIR Function 继续代表已检查产物；不能给 body 塞占位表达式或活动源码句柄。
源码服务分别持有接口与模板，活动任务归当前区域，完成片段才可进入执行器。

当前受检 NIR 已将声明字段收拢为 `program::FunctionDeclaration`：它持有身份、
签名 telescope、阶段／责任、参数绑定、形成条件、程序契约、入口快照及默认值。
`Function` 只组合这份 `interface` 和已检查 `body`，没有第二份字段副本。
旧字段投影通过 Deref 借用同一份声明；需要拆开可变借用或转移字段所有权的消费者
显式访问 interface，不通过额外 clone 规避所有权边界。

`check_source_function` 在检查主体之前组装受检声明，主体成功后才交付完整 Function；
失败仍恢复 Type 输入事实与 quote 检查状态，不把声明塞入执行函数表。VM 的
`invocation_declaration` 以及程序／类型契约的替换、能力检查只接收 FunctionDeclaration，
主体仍沿独立的
Entry → Body 路径取得。required 收集生成的 runtime 函数身份时只复制所需声明字段，
不再先复制整个主体；需要执行的依赖仍保留完整代码。声明和完整函数分别具有
S-expression Display，完整函数的既有 dump 结构保持不变。

这是已有受检产物的所有权拆分，尚非完整公开供应接口。公开参数名／默认提供者的
跨值传送、源码模板的独立发布、未完成主体请求及原区域 Type 响应仍待接通；
当前 Package 与 VM 注册表仍只收录完整函数。不能因为声明类型不含 body 就宣称
未调用定义可跳过检查，或已经能在等待 Type 时恢复同一次 invocation。

普通输入帧的结构准入现在使用
`CallableProgram::validate_input_frame`，直接接收声明视图：检查 callable telescope、
绑定数量及唯一性、默认供应槽位，以及默认表达式只能读取已经声明的前序参数。
默认内部的局部绑定仍由共同 free-bindings 遍历处理。返回值借用原声明的参数域，
不复制签名、不读取或执行主体，也不为接口制造 Unit 代码。

phase 声明目录与 VM 冷构造共用此入口，覆盖普通函数、quote template 和没有主体
的 trait 方法接口。VM 原独立的方法绑定计数／去重检查已删除；原先未经 phase
凭据的直接 VM 构造没有统一检查普通函数帧，现在在发布 VM 前拒绝错误帧。
`Inspection::callables_for` 将已完成的身份目录绑定到同一个不可变 Package。
phase 检查、VM 和 CodeSession 通过它区分可执行函数、quote 模板和仅有身份的
runtime 函数。声明观察读取接口和来源，执行另行取得主体。冻结世界的目录可以
复用；原始 VM 构造从自己的 Package 建立目录，重复身份在发布前产生诊断。
预算失败与无效绑定分别报告；默认表达式分析错误保留默认值自身的位置。

这只证明输入帧的结构和默认绑定可见性。默认值的类型、result／entry snapshot
约束、程序契约、phase、资源及代码安全仍由各自检查负责；没有新增执行授权。
现有 Package adapter 仍从已检查 body 取得诊断位置，源码声明独立发布和未完成
主体取得尚未实现。VM 构造成本不随主体语句数增加的测试，只证明该准入不扫描
主体，不能推导完整编译成本或一般源码 continuation 已完成。

验证基线 `a10c82c5`，工作目录 `/home/fin/luna`，统一经
`python3 -B development/check.py --timeout 900 --`：

- `test -p nir --lib -p comptime_interpretor --test callable_frame_admission
  --no-fail-fast -- --test-threads=1`：首轮与显式证据查询接线后的最终复核均退出 0，
  `/tmp/luna-tools-eecyeo_7/result.json`、`/tmp/luna-tools-7tao7ocd/result.json`。
  包含无主体声明、错误帧在注册时拒绝、逐预算耗尽／恢复、quote 不获执行权，
  以及已有 source owner、缓存权限和 VM 驱动回归。
- `test -p nir -p comptime_interpretor -p luna --test phases
  --test runtime_function_identities --test instance_budget --test default_calls
  --test declaration_contracts --test staged_callable_freezing --test library_std_format
  --no-fail-fast -- --test-threads=1`：退出 0，
  `/tmp/luna-tools-pt2f9npd/result.json`，覆盖 phase、运行时身份、实例预算、源码
  默认值和契约、冻结 callable 及真实 sysroot 格式扫描；随后仅将复用条件改为上述
  显式同源证据查询，并完成前一项的最终复核。没有修改失败预期或解除 ignored。
- 九个修改／新增 Rust 文件定向 rustfmt `--check` 退出 0，
  `/tmp/luna-tools-fesekzri/result.json`；未运行全 workspace 检查。

#### 程序契约使用声明，不制造可执行主体

`MethodContract::program_summary_declaration_inputs` 交付独立 FunctionDeclaration，
`CallableProgram::declaration` 只观察声明根。接口摘要不再制造 Unit 主体；调用分析
分别持有实现主体与公开接口声明，不能通过扫描一个假主体推导参数稳定性。
公开方法契约的输入属于独立调用帧，实现局部变量的重绑定不会重写这个帧；指针
所指存储是否稳定仍须通过完整 memory summary 另行证明。普通直接调用仍检查
真实主体中的参数写入和取址。声明摘要本身不授予内存访问或执行权限。

trait 契约继承的私有 overlay 只复制和替换 FunctionDeclaration，保持原实现
主体的所有权与存储；复制预检计入签名、默认值、形成条件和程序契约，不遍历
不需要复制的实现主体。整批有效契约准备好后才证明显式前置条件的弱化，失败或
unwind 恢复所有声明，绑定分配游标也只在整批成功后提交。

程序验证的 `flow::prove_implication` 直接在同一参数状态下证明前提蕴含结论，
不构造 Unit、return 或虚假结果值。它复用普通验证的输入域、谓词 definedness、
算术与内存事实检查；拒绝结果绑定、入口快照和默认供应这些需要调用执行语境的
载荷。trait 继承和 contextual code 方法细化共同使用该入口。预算耗尽继续返回
Unknown，不能被包装成“实现加强了前置条件”。所有程序命题仍由程序验证层处理。

memory 方法组合确实需要执行实现调用，因此只在真实 Call 构造完成后组合
Function；不先建立 Unit 占位再替换。以上是声明消费者的迁移，并不表示源码
检查已能挂起、恢复原 VM activation，或 `defer std.fmt.Parse(format)` 已接通。

本阶段验证（工作目录均为 `/home/fin/luna`）：

- 经 `python3 -B development/check.py --timeout 900 --` 运行
  `test -p nir -p program_verification --lib --test calls --test code_interfaces --test copy_budget --no-fail-fast -- --test-threads=1`，
  `/tmp/luna-tools-8hwye2qw/result.json` 退出 0，321 项通过。覆盖声明观察与复制预算、
  10,000 条语句主体下不增加契约继承成本、失败回滚与重试、蕴含证明逐预算耗尽。
  首次构建 `/tmp/luna-tools-841m65mx/result.json` 因新测试对无 Debug 的 Report 使用
  unwrap_err 而失败；测试改为显式提取 Err 后得到上述结果，未修改生产类型迁就测试。
- 同入口运行
  `test -p luna --test method_memory_contracts --test cmtt_methods --test declaration_contracts --test prepared_invocations --no-fail-fast -- --test-threads=1`，
  `/tmp/luna-tools-1vzwo67y/result.json` 退出 101：188 项通过、1 项失败。
  失败为 `cmtt_methods::transported_runtime_methods_preserve_clause_domains_and_nested_storage`，
  408..419 的方法选择报告 no-applicable-candidate。
- 为区分回归，将本阶段七个生产文件临时还原为 `9bc7b5a2` 的内容，仅运行上述
  失败测试（`test -p luna --test cmtt_methods -- transported_runtime_methods_preserve_clause_domains_and_nested_storage --exact --test-threads=1`）。
  `/tmp/luna-tools-lviozzbq/result.json` 退出 101，复现相同位置与诊断；随后按保存内容
  逐字节恢复本阶段生产文件。其他工作区修改未触碰。该既有失败仍未修复，不能将
  集成组整体记为通过，也不改变测试预期。
  后续[callable 字段核对](runtime-value-delivery.md#callable-字段的观察与调用)将
  这一失败定位为缺少库 Splice，保留原正例为待实现验收；它与新发现的嵌套源接收者
  过早交付、comptime callee 角色丢失分别处理。

- 指针观察扩展检查：同入口运行 `test -p luna --test execution -- pointer_observations --test-threads=1`，
  `/tmp/luna-tools-ac4abr98/result.json` 退出 101，10 项通过；
  `external_pointer_observations_track_nested_field_paths` 在要求拒绝右字段写入时失败。
  使用相同隔离与恢复方式，在 `9bc7b5a2` 上以该测试名加 `--exact` 复测，
  `/tmp/luna-tools-ll9y52z_/result.json` 同样退出 101。该用例与已实现的同根不相交
  字段观察保留存在预期冲突（见[标量存储更新](solvers-resources-and-lowering.md#64-标量存储更新与读取事实的有效期)），
  本阶段未调整其断言；区分既有不一致与本次回归。
  后续已按正式字段分离契约纠正该旧拒绝预期，并保留同字段覆盖及可能别名的拒绝；
  11 项观察测试移入独立目标，见[核对与反例](solvers-resources-and-lowering.md#指针观察旧拒绝预期的纠正)。
- `test -p luna --test execution -- bodyless_trait_contracts_preserve_pointer_reads_during_receiver_substitution --exact --test-threads=1`
  在本阶段代码上退出 0，`/tmp/luna-tools-wvqtc1te/result.json`；确认接口摘要没有主体
  时仍保留指针读取，并拒绝不成立的结果或范围契约。已修改 Rust 文件的定向 rustfmt
  检查退出 0，`/tmp/luna-tools-x4i57saa/result.json`。未执行全 workspace 测试。

#### 命名函数形成不再准备泛型主体

源码 VM 中，命名泛型函数值现在使用 `Value::FunctionInstance`，只携带声明身份与
按声明域检查后的普通 comptime 实参。形成该值仍检查签名、形成条件、能力要求与
接收域，但不再为此复制、替换整个主体，也不把命名函数包装为带代码的 Closure。
真正捕获环境的闭包继续保留自己的环境与代码；具体 IIR 的执行载体尚未迁移。

调用先在接收 VM 的声明世界核验身份与阶段；仅有 runtime 身份摘要或 IIR 的传送
证据不能授权源码调用。普通输入、类型要求、能力和入口快照由声明完成准备，程序
入口条件仍在同一次 activation 中逐项执行。只有 Entry 成功转入 Body，才按声明
实参替换主体并取得执行代码。默认供应和契约替换通过共同的声明实例观察取得实参，
不再要求 callee 恰好采用 Closure 表示。

实例代码缓存仍只保存稳定代码，不保存普通输入、结果或当前调用的证明。冷缓存的
入口失败不会准备主体；热缓存的调用仍重新检查当前契约。内部缓存暂沿用空捕获的
Closure 代码载体，与 IIR 共用已有存储，不把这个实现细节重新暴露为命名源码值。

这一步解除“取得函数值就准备主体”的耦合，不等于源码可以不检查未调用定义。
VM 注册表仍依赖完整受检 Function，初始化仍检查包；大主体的形成成本测试是在
初始化完成后测量，不能据此宣称包准入成本与主体大小无关。源码检查 continuation、
独立模板发布和原 activation 的 Type 响应仍未接通，`Parse(format)` 仍未完成。

本阶段验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p luna --test prepared_invocations --test normal_form_domains --test clause_packet_preparation --test comptime_bytecode_defaults --test scoped_comptime_callables --test library_comptime_comparison --test staged_invocation_reuse --no-fail-fast -- --test-threads=1`：
  `/tmp/luna-tools-8q7lz18j/result.json` 退出 0，65 项通过。覆盖默认值与 producer
  顺序、返回的 comptime 函数身份、普通 runtime 转交、跨次代码复用及当前契约拒绝。
- clause packet 的首次单项复现 `/tmp/luna-tools-qm02jgz2/result.json` 退出 101，
  在旧测试 helper 的 VM 初始化处报告 `UnmaterializedDispatch`，尚未进入调用。
  该 helper 对真实 sysroot 使用 closed 构造入口，已改用现有 `with_comptime`，
  同时提供当前包的类型、方法与关联项查询。采用 helper 既有的 sysroot 准备预算，
  保留两种引擎及全部结果、顺序和次数断言，不删除依赖或假造选择结果。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p elaboration --lib --test function_types --test instance_budget --test runtime_function_identities --test callable_reflection --test normal_values --test function_input_diagnostics --test type_computation_demands --no-fail-fast -- --test-threads=1`：
  `/tmp/luna-tools-h2ac0sg3/result.json` 退出 0，939 项通过。包含 606 项 VM 单元、
  241 项 elaboration 单元和 92 项接口用例。0/4096 语句主体在包初始化后的命名值
  形成成本相同；入口拒绝不准备代码，恢复后正常执行，热缓存不免除契约。
  首次 `/tmp/luna-tools-jdxmzj9d/result.json` 因外部测试误用私有 `invoke_value`
  编译失败；改用既有公开 `invoke_instantiated`，没有扩大生产 API。

未运行全 workspace 检查；这些证据不覆盖待实现的源码挂起和 Type 描述准入。

#### 实际调用链与替换边界

`3fd323ca` 上的生产链如下。箭头表示当前同步调用／数据依赖，不表示已经存在
挂起请求；这里的 `check_source_function` 同时服务包收集与 required 依赖收集。

```text
package collection / required dependency discovery
  -> check_source_function
     -> function_body -> expr -> comptime_function_instance
        -> prepare_callable_clause_packet -> planned_typed_declaration_inputs
        -> require complete callable signature or return Error
  -> completed nir::Function

prepare_required_root_with_live_inputs
  -> build closure -> checked root Expr
  -> complete referenced functions -> PreparedRequired
  -> invoke_publishing -> Interpreter
     -> begin_invocation -> prepare_invocation
        -> bind inputs, formation/capability checks, entry snapshots
     -> Session: execute entry conditions with prepare/accept_invocation_step
        -> prepare_invocation_body -> execute body -> execute exit conditions
        -> completed value / original control flow
```

第一个断点发生在完整根产生以前，而 VM 宿主请求只在第二段已经取得执行代码后
才会发生。因此给 `InvocationProgress` 增加一个枚举项、公开测试用 Fragment，
或者在 `dynamic_instances` 的错误分支调用 required 求值，都没有连接这两段。
特别是最后一种做法会在检查未调用定义时执行普通主体，或在原调用之外重建存储。

目前 `PreparedRequired.entry`、result_type 和 RequiredWorld 都以完整根为前提。
保留这个已经完成的快路径；新的活动源区域不能伪装成 PreparedRequired，也不能
每交付一个片段便建立一份新根、重新捕获输入。`required_live_input` 接收 live 值
不等于拥有原 activation，重复传入相同 Value 也不证明执行位置或 return 目标一致。

以下是需要共同落地的职责边界，名称描述目标职责，不是声称已有这些 Rust API：

| 交付物 | 首次建立它的入口 | 必须携带／验证 | 消费者 |
| --- | --- | --- | --- |
| 声明接口 | 当前签名、默认值和契约检查 | 身份、telescope、输入阶段与供应接口、形成条件、程序契约、结果观察及世界依赖；可以描述依赖，但不能谎报具体签名 | 定义检查与调用准备 |
| 已检查源码模板 | 包收集与 `check_open_definition` 的共同定义规则 | 源码快照、声明／body 身份、定义端证据、允许延迟的义务及自由声明绑定；不含本次输入值 | 创建活动检查任务 |
| 已准入调用 | 父区域的输入准备 | 当前签名、按源码序实际取得的参数、形成／阶段／域证据；默认值只按本次供应选择 | 原 VM 的调用入口 |
| 活动主体检查 | 已准入调用的主体请求，或获准执行的 required 根 | 原区域与执行位置、输入域观察、规则后继、词法退出责任、等待的 Type 消费者 | 交付下一段受检代码或失败 |
| 可执行片段 | 活动检查区域 | 完成的类型与 phase、输入输出绑定、原控制出口、世界与执行许可；没有待检查 HIR | 同一 VM 区域 |

接口存在不授予执行权。源码模板存在也不代表其中每项调用都已准入。普通完整函数
继续直接产生 NIR Function；合法延迟的主体保留在源码服务，不能为了让现有函数表
接受它而放入空 Block、Never 或假的 Type。已声明函数值的身份和接口观察可用时，
仍不等于已经取得一个可由 closed/runtime VM 执行的函数体。

`open_functions::Definition` 已有定义端检查，但其 Known/Pending/InputType/Suspended
只描述注解，不能直接当作一般主体模板。迁移必须共用已有定义规则及其前提管理，
替换按语法形状追踪的重复路径；不得给所有 comptime 函数设置 `checked = true`
绕过检查，也不能只等待具体调用后才检查未调用定义。已知错误、非法阶段和无依据
的结构／trait 操作仍在定义端拒绝；只有正式规则允许延迟的义务进入模板。

#### 检查服务与 VM 的借用边界

当前 `InvocationSession` 独占借用 Interpreter，`Checker` 又直接借用 HIR 包。
目标协调器分别拥有检查任务和执行驱动，在一次 poll 返回后传递消息；检查服务
不持有 VM 的 `&mut Environment`，执行器不持有 Checker。不能靠把双方包进
RefCell／Mutex 后从回调重入，或增加 unsafe Send／延长 HIR 生命周期来连接它们。

源码 owner 沿现有 `HirPackageBox::package()` 的借用期限保活。稳定模板保存
所属快照与声明/body 标识，活动任务在这个快照的有效借用内解析它们；快照退出前
销毁活动任务。不把内部 `Package<'static>` 的擦除寿命暴露给 VM，也不把活动 HIR
借用写进 NIR、CodeSession 或磁盘缓存。外部请求可拥有稳定标识与受检载荷，源码
检查回调只在有效 owner 下临时借用 HIR。

一次交接依次执行：

1. 检查任务先保存所有父规则后继和本次待观察位置，交付带域、scope 和执行许可
   的受检前缀。只检查完 producer 的类型不能标记它已经执行。
2. VM 在原区域执行前缀，保留参数 cell、作用域、当前控制阶段和已消费位置。
   若遇到关联项、方法或 builder 请求，继续使用已有线性宿主协议；请求未完成时
   不额外启动同一片段。
3. 正常结果经接收世界与域准入后形成观察响应。非局部 return、失败和取消走原
   控制路径，不被包装成一个 Type 响应；入口／退出契约也不因切片重复运行。
4. 原检查任务只恢复对应待观察规则。`CallableClausePacket.pending` 中的实际
   Type 完成签名后，callable 才可交付；producer 的 receipt 保留在本次输入计划，
   不能同时留下一个会再次执行的 `ComptimeFunctionInstance.subject`。
5. 后续受检片段引用已取得的绑定，按通用域和资源规则使用它们。返回可变值引用
   的读取与只读 Type 观察不同：描述响应不允许克隆可变环境，也不延长已退出 scope。

借用解除只是宿主实现条件，不是源作用域退出。等待 Type 时不运行
`restore_environment`、LexicalScope 的退出或 generator checkpoint 回退；正常离开
和失败退出才消费各自责任。内部检查 Probe 可以撤销候选构造状态，已经执行的
源程序前缀不能因此被回滚或重放。定义发布仍先完成整批接收预检，再切换后续代码
可观察的声明世界；不得原地替换旧 CodeSession 的包并继续使用其准入证据。

两个 ignored 正例分别覆盖签名与列表域消费者，并不覆盖这个交接协议的全部边界。
当前同步错误应保留为明确缺口，直到上述生产链一起接通；已有帧化不构成可以在
任何 `needs_context` 上自动执行源码的许可。Type 描述还须区分已完成的 Any 数据
与尚未取得的观察，具体见[描述生产与准入](type-descriptions-and-admission.md)。

| 完成点 | 可承诺 | 尚不能承诺 |
| --- | --- | --- |
| 定义接口／稳定模板 | 原身份、望远镜、阶段、已检查契约与合法 residual | 所有具体调用的签名已形成，或已有可执行 body |
| 调用准备 | 当前 callee 签名、输入与形成义务已经满足 | 入口程序契约已经运行或主体已完成 |
| 已进入 activation | 原参数存储、结果域、入口快照与契约进度 | 未取得的受检主体已经可以执行 |
| 可执行主体／片段 | 域、阶段、安全检查及所需声明世界满足交付契约 | 重放前缀、运行未选分支或改变 NF 身份的许可 |

现有 `check_source_function` 已共用参数绑定、普通 Type 的 bound 身份、前后置条件、
入口快照和模板／效应感知主体检查。最终声明携带已检查默认值；required 依赖由实际
call plan 提供用到的默认值，不能强制准备未使用项。

VM 已把普通输入的 body 重绑定放在 Entry → Body 转换：入口程序契约通过后才准备
这份执行代码，等待／取消不重新 begin_invocation。源码命名实例的声明参数主体替换
也已移入这一阶段；域、签名与形成条件检查仍在调用准备时完成。该接线不是定义端
源码延迟检查。新主体服务必须沿同一 activation 取得代码，不能
为了允许缺失 body 而删掉定义检查或阶段准入。

VM 的 `contracts::Progress::Body` 现直接持有已准备的 Body 与其代码缓存键，
不再由 InvocationFrame 在 Entry／Body／Exit 之外保留一个始终可回退执行的 body。
Entry → Body 先取得并按普通输入准备代码，成功后一次交付这个阶段状态；Exit
保留结果和原 source，用于结果交付、后置条件及清理。树执行与字节码执行共用
InvocationFrame 的当前代码选择，不在入口或出口缺少 condition 时回退到主体。
dump 区分 package-body／owned-body 及相应缓存键。

此处只拆开了活动调用的执行状态。被调用的 Function／Closure 仍可保留完整的
已检查源码；签名准入现通过下面的声明视图取得来源，完整包适配器仍从 body 提取
该位置。声明参数特化、函数收集和
RequiredWorld 尚未因此接受缺少主体的源码模板，也没有新增源码主体的外部请求。

接线要共同改变函数收集、required 根和 VM 主体取得。当前 RequiredWorld 只有完成
的 NIR 声明；只给 InvocationProgress 增加请求变体无法让未完成源码抵达它。
源服务不能提供某种模板时明确报告缺口，不转而解释 HIR，不将所有函数改为首调用时
才检查。只有正式规则允许延迟的义务可以进入稳定模板。

片段的声明世界与依赖证据必须匹配；新增定义需要显式可见世界交付，不能换掉
CodeSession 的包指针后复用旧准入。活动任务、receipt 和 VM cell 不进入 RequiredWorld、
代码缓存或 Type 结果缓存。相同源码的两次 invocation 只共享稳定模板、接口与合法
代码，不共享执行状态。NF 头始终不展开，实际 Type 响应也不改变它的身份规则。

#### VM 声明消费者与主体查询分开

当前源码函数目录提供 `FunctionDeclarationView`：借用的 `FunctionDeclaration`
和独立交付的诊断位置，不带函数主体、Closure 或执行环境；默认值和程序契约代码
仍是声明接口的一部分。命名函数准入、普通／泛型
调用入口、参数绑定、签名特化、trait 方法选择、默认代码的接口选择、值域检查及
冻结函数值恢复都通过声明查询取得元数据；程序契约沿 `invocation_declaration`
取得同一接口。消费者不再为访问接口字段而接收完整 Function。

命名函数的主体取得有独立 `body` 查询，仍发生在已存在的 Entry → Body 转换中；
泛型主体特化也先读取声明，再在主体阶段单独取得代码。恢复 ClosureSnapshot 时
仍需按原规则实际调用 factory，声明观察本身不会代替这次执行。闭包继续拥有其
捕获环境和代码，本次没有把匿名闭包强改为命名声明。

该视图保留原包借用期限，查询不复制接口或建立新的声明目录，也不授予阶段、
形成条件或契约许可。Display 输出 `(source-declaration (source start end) ...)`
及声明元数据，转义名称且不输出主体。名称／域／默认值仍来自原声明，未新增
normal-form 展开规则或依赖缓存。

**物理目录仍以完整 Package 为来源。** 当前适配器从完整 Function 借用 interface，
从其 body 取得原诊断范围；尚不能注册缺少代码的源码模板。因此这是消费接口的
拆分，不是懒检查服务，也不能证明两个待定 Type 正例已可执行。后续源码服务须
直接提供声明位置及 owning snapshot，再以受控响应满足主体需求，不能把当前
adapter 的成功当作部分包拥有完整 phase 凭证。完整包和代码缓存规则继续保留。

本阶段验证目录 `/home/fin/luna`，统一使用 `development/check.py --timeout 900 --`：

- `check -p comptime_interpretor`：退出 0，`/tmp/luna-tools-eofeqvab/result.json`。
- `test -p comptime_interpretor --lib --test runtime_function_identities
  --test instance_budget -p luna --test ordinary_callable_types --test default_calls
  --test staged_callable_freezing --test library_protocols --test library_std_format
  --no-fail-fast -- --test-threads=1`：退出 0，
  `/tmp/luna-tools-7m994kvp/result.json`。其中原有的待定签名正例仍标记 ignored；
  新声明视图测试检查借用来源、未知身份与稳定转义 dump，不声称验证缺少主体的目录。
- 单独执行两个 ignored Type 正例，命令使用 `test -p luna
  --test ordinary_callable_types --test ordinary_clause_collectors --no-fail-fast
  -- --ignored --test-threads=1 pending_clause_domains_can_wait_for_a_type_producer
  computed_type_inputs_can_complete_a_dependent_callable_signature`：退出 101，
  `/tmp/luna-tools-9tua589f/result.json`。仍分别失败于 `159..181` 的待定 callable
  签名和 `193..201` 的缺少列表元素域，与此前取证相同；没有更改预期或隐藏失败。
- 修改的 Rust 文件格式检查退出 0，`/tmp/luna-tools-7exef8xd/result.json`。
  没有执行全 workspace、native 或性能基准。

#### 完整包检查凭证也是迁移边界

在 `97918570` 上复核午报 D02／D03，阻塞链还包括下面这些实际消费者。
本表补充前面的 owner 设计，说明实施时必须同时替换什么；不是新增已实现 API。

| 位置 | 当前所需产物 | 迁移要求 |
| --- | --- | --- |
| `prepare_required_root_with_live_inputs` 与 `required_source_closure` | 完整根；按声明、方法、关联值和定义身份建立本次调用的源码闭包 | 区分声明可见性与本次代码需求；合法延迟模板保留定义端检查，不把源码引用一律当作立即取得完整主体的要求 |
| `required.rs::required_function` | known callable signature，然后同步 `check_source_function` | 签名待决由父检查区域保存；尚未准入的 callee 不能建立 activation 来计算自己的参数域 |
| `required.rs` 的 `check_package_with_evidence` 与 `validate_machine_operations_checked` | 整包 phase 完成证据及独立的算术安全检查 | 完整根保留现有快路径；源码片段交付须带与实际片段、binder 和世界匹配的检查证据 |
| `RequiredWorld::with_phase`／`CodeSession::with_phase_checked_source` | 对同一个不可变 Package 的整包凭证 | 不把部分目录升级为 PhaseCheckedPackage；源码服务和活动任务使用独立 owner，不伪造完整包 |
| `nir::callable_source::Callables` | 与源包绑定的不可变身份目录 | 声明观察与代码取得分别查询；待完成源码的独立发布仍需接线 |
| `prepare_invocation` | 通过 FunctionDeclarationView 读取接口及来源；底层适配器仍依赖完整 Function | 源码服务须直接发布声明来源；消费端已不接收主体，但尚未解除目录的完整代码要求 |
| `prepare_invocation_step` 的 Entry → Body | 同步取得、特化和重绑定完整主体 | 入口契约成功后在原 activation 等待代码，响应不得再次 bind inputs、运行入口契约或建立新调用 |

`nir::phases::Inspection` 与 `PhaseCheckedPackage` 也必须区别：前者拥有相同
世界的声明目录并可检查表达式，后者只在全部函数、quote 模板、默认值与契约的
phase 遍历成功后形成。`Inspection::check` 不是类型／资源安全证明，既有
`invoke_comptime_expression` 又会建立新的闭包调用；它们均不能直接作为活动
源码片段恢复 API。为避免扩大权限，新片段交付应满足以下条件：

- 稳定目录只证明声明和域的合法性，不声称未完成主体已经检查。定义模板仍须
  检查已知错误、阶段和定义端义务；合法 residual 必须显式保存。
- 片段证据绑定准确代码、输入输出 binder、世界依赖及控制出口；类型、phase、
  safe 义务分别完成。需要既有程序事实时交付其上下文，不能改用空前提重检后
  放宽失败，也不能仅持 phase 凭证绕过算术与资源验证。
- 接收端核对原区域／activation、请求序号和需要的完成点，再消费一次响应。
  预算失败、取消、错世界及错误 binder 不安装代码、不前移 PC；旧响应可以退回
  原 owner，取消后的响应不能复活调用。等待中的存储和词法退出责任仍在原帧。
- 稳定代码缓存只存已检查产物及依赖，当前调用的入口契约、Type 观察、cell 与
  pending request 不进入缓存。完整包检查不能因命中某个片段而提前标记成功。

现有 `DefinitionHostRequest` 的载荷只覆盖 Struct／Enum／Newtype builder，
回答的是“准备哪些新定义及其 receipt”，不是“取得哪个源码主体”。不要把主体
请求塞入 builder 协议：前者恢复已知声明的执行，后者安装新声明并更新可见范围。
两者可共用 ticket／预算的实现原则，但完成条件、取消责任和世界变动不同。

**实施验收必须越过这整条链。** 两个待定 Type 正例先证明父区域能交回观察；
随后还需未调用定义的已知错误拒绝、入口契约失败不准备主体、等待期间不重放
producer、同一模板的两次调用独立、片段算术／phase 违规拒绝，以及 builder
在片段之间发布后的正确世界交付。仅证明函数表能存接口，不能算恢复功能完成。

本次取证在 `/home/fin/luna`、`97918570` 加原有工作区改动上执行：

```sh
python3 -B development/check.py --timeout 900 -- test -p luna \
  --test ordinary_callable_types --test ordinary_clause_collectors \
  --no-fail-fast -- --ignored --test-threads=1 \
  pending_clause_domains_can_wait_for_a_type_producer \
  computed_type_inputs_can_complete_a_dependent_callable_signature
```

`/tmp/luna-tools-b79xm5lh/result.json` 退出 101；两个目标测试确实运行，未再
跳过。`stdout.log` 的失败位置及原文如下：

- `159..181`：`comptime callable signature still depends on an invocation-time clause input`。
- `193..201`：`list literal requires an element type context`。

这是当前缺口证据，测试源码及预期没有修改；本节未实现源码主体服务、未授予新
执行权限，也未重跑其他测试组。

### 纵向交付与验证证据

下一项交付同时贯通普通规则 continuation、原执行区域的片段交接、待定输入域和
callable 签名对实际 Type 的消费。只增加 owner、请求枚举或可手工构造的 fragment
入口不满足交付；也不能只解除两个 ignore 而遗漏下列边界。

| 验收 | 必须证明 |
| --- | --- |
| `count<.values = [1, 255], Byte()>()` | 域实际完成后检查和组装，Byte 一次，完整输入后才调用 count |
| `identity<Byte(), true>(9)` | 父区域执行 callee producer，签名完成后检查 9，无 packet 重放 |
| 前后输入读写同一存储 | 源序、读取时的值、原 cell 身份，含方法接收者 |
| 前项 return／失败 | 后项 Type、默认值和主体不运行，原控制出口保留 |
| 分支、guard、循环、递归 | 所选路径执行；应检查的未选代码仍检查；作用域及执行代次不串值 |
| 具名／默认／空展开 | 检查顺序可不同，实际求值顺序和次数不变 |
| 错域／预算／取消／旧响应 | 失败不发布半成品；不同失败原因保真；重试不重放已执行前缀 |
| 未调用定义 | 保留定义端检查及合法 residual，不执行普通主体效果 |
| NF／名义输入与真实 sysroot | 消费同一协议，普通值参数无库名特权，跨世界准入仍生效 |

普通路径与这些边界通过后，NF／名义消费者迁入同一协议，才能退役 InputChecking
分流和 NormalFormArgument 的执行树。未迁移消费者必须保留其真实检查，不能为了
删枚举而丢失能力。活动 continuation 不进入稳定 NIR；实例化从稳定模板创建新任务。

下表记录各次迁移的对应验证，未重跑的历史报告不累计为当前通过率；
详细命令与原失败过程可从相应提交及 result.json 查找。临时日志可能被环境清理，
持久行为用例以仓库测试为准。

| 已有边界 | 持久测试入口／历史报告 |
| --- | --- |
| 联合 Type 需求预检 | `elaboration/tests/type_computation_demands.rs`；`/tmp/luna-tools-_n_n357y/result.json`，退出 0 |
| 控制／输入事实边界 | `luna/tests/{control_boundaries,scoped_input_observation,required_input_scopes}.rs`；`/tmp/luna-tools-6u3qftz2/result.json`，退出 0 |
| 声明输入与已知泛型域 | `luna/tests/{clause_source_dependencies,clause_packet_preparation,normal_input_plans,trait_domain_substitution}.rs`；`/tmp/luna-tools-zy_crm7s/result.json`、`/tmp/luna-tools-timkx462/result.json`，退出 0 |
| 输入依赖后继 | `luna/tests/ordinary_dependency_tasks.rs`，含 72 层无环默认链及根调用次数；`/tmp/luna-tools-sdgv1vw8/result.json`，退出 0 |
| 声明槽位后继与临时观察退出 | 在 `/home/fin/luna` 经 `python3 -B development/check.py --timeout 900 -- test -p luna --test declaration_input_plans --test ordinary_clause_collectors --test ordinary_callable_types --test normal_input_plans --test type_spreads --test default_calls --test library_std_format --no-fail-fast -- --test-threads=1`，`/tmp/luna-tools-k8_euqcn/result.json` 退出 101：217 项通过，7 项在 VM 初始化失败，2 项原待定 Type 正例保持 ignored。7 项在 `1800a2bf` 编译器基线同样报 UnmaterializedDispatch（`/tmp/luna-tools-9kbmtex2/result.json`）；`b4872073` 为真实 sysroot 测试接回已有源码查询服务，原断言、两引擎选择及预算不变，基线复验 7 项通过（`/tmp/luna-tools-q2ecn5e_/result.json`）。恢复本次实现后，以同入口运行 elaboration/lib、ordinary_callable_types、ordinary_clause_collectors、contextual_input_regions、library_std 的相关过滤项，`/tmp/luna-tools-3tfwc5vz/result.json` 退出 0，15 项通过，覆盖上述 7 项、默认值退出、宿主输出、观察安装失败与 dump；完整参数见报告。已改 Rust 的 scoped rustfmt 检查退出 0，`/tmp/luna-tools-6yied94m/result.json`。这些结果不代表外部 Type 响应接线完成 |
| 依赖域后继与共享收集器预检 | 在 `/home/fin/luna` 经 `python3 -B development/check.py --timeout 900 -- test -p luna --test declaration_input_plans --test ordinary_callable_types --test ordinary_clause_collectors --test normal_input_plans --test library_std_format --no-fail-fast -- --test-threads=1`，`/tmp/luna-tools-081js_1d/result.json` 退出 0，91 项通过，两个原待定 Type 正例仍 ignored；包括嵌套预检执行顺序、不可达输入拒绝、传递依赖与失败后恢复。`test -p elaboration --lib declaration_inputs -- --test-threads=1` 经相同 wrapper，`/tmp/luna-tools-8diaucr4/result.json` 退出 0，6 项通过，覆盖预算、sort 冲突、计划发布与 S-expression dump。已改 Rust 的 scoped rustfmt 检查退出 0，`/tmp/luna-tools-gz5676f6/result.json`。这些结果不证明 VM Type 响应已接入源码检查 |
| 共用输入驱动中的收集检查 | 在 `/home/fin/luna` 经 `python3 -B development/check.py --timeout 900 -- test -p luna --test declaration_input_plans --test ordinary_callable_types --test ordinary_clause_collectors --test normal_input_plans --test library_std_format --no-fail-fast -- --test-threads=1`，`/tmp/luna-tools-x_6qo3mo/result.json` 退出 0，93 项通过，两个原待定 Type 正例仍 ignored；新增交错探测／字面量回退、空展开、源码早退和普通类型错误传播。初轮拒绝 fixture 使用 `1u8` 时先报 LiteralInto 未绑定（`/tmp/luna-tools-9pa2g49v/result.json`）；改用返回 u8 的普通函数供应错误实参后，仍断言类型不匹配且不落入 contextual 回退诊断，不宣称修复字面量后缀。`test -p elaboration --lib declaration_inputs -- --test-threads=1` 经相同 wrapper，`/tmp/luna-tools-anrhcita/result.json` 退出 0，7 项通过，含未完成收集不得发布及 dump。已改 Rust 的 scoped rustfmt 检查退出 0，`/tmp/luna-tools-vk36a4ey/result.json`。源码／VM 交织恢复仍未完成 |
| 完整输入准备与调用者环境退出 | 在 `/home/fin/luna` 经 `python3 -B development/check.py --timeout 900 -- test -p luna --test clause_packet_preparation --test declaration_input_plans --test ordinary_callable_types --test ordinary_clause_collectors --test normal_input_plans --test library_std_format --no-fail-fast -- --test-threads=1`，`/tmp/luna-tools-_ic5srl0/result.json` 退出 0，112 项通过、两个原待定 Type 正例仍 ignored；新增嵌套包默认值作用域、空包和提前返回的执行次数／顺序，以及提前返回后仍拒绝错误输入。初轮拒绝 fixture 的裸整数先触发 runtime proxy 目标诊断（`/tmp/luna-tools-u9oicu0r/result.json`），改用返回 u8 的普通函数隔离 bool 参数的域错误，保持 mismatch 断言。`test -p elaboration --lib declaration_inputs -- --test-threads=1` 经相同 wrapper，`/tmp/luna-tools-o4tltuu2/result.json` 退出 0，9 项通过，含路由和 caller scope 的 dump。已改 Rust 的 scoped rustfmt 检查退出 0，`/tmp/luna-tools-etsbuahp/result.json`。这些结果验证同步路径的迁移，不证明源码／VM Type 响应恢复或目标 println 完成 |
| 输入环境所有权与保留预算 | 在 `/home/fin/luna` 经 `python3 -B development/check.py --timeout 1200 -- test -p elaboration --test invocation_budget -p luna --test declaration_input_plans --test normal_input_plans --test ordinary_callable_types --test ordinary_clause_collectors --test required_input_scopes --test library_std_format --no-fail-fast -- --test-threads=1`，`/tmp/luna-tools-iq4dupri/result.json` 退出 101：103 项通过，两个原 Type 等待正例保持 ignored，另两项 required scope 测试在旧助手创建 VM 时报 UnmaterializedDispatch。仅将本轮所属编译器文件替换为 `cf859929` 的对照复现同样两项失败（`/tmp/luna-tools-3jyglitp/result.json`），其余已有工作区修改保留；全部本轮源码随后恢复。`a9f8edc1` 将助手接回已有源码查询服务，原预算、双引擎、契约与失败后重试断言不变：对照编译器通过（`/tmp/luna-tools-_qp14jug/result.json`），恢复本轮实现后过滤 `required_and_published` 复验两项通过（`/tmp/luna-tools-81wzyksq/result.json`）。同入口运行 `test -p elaboration --lib declaration_inputs -- --test-threads=1`，`/tmp/luna-tools-k68oz8to/result.json` 退出 0，14 项通过，覆盖快照隔离、逐项预算、深度耗尽、未知槽位与巨大 size hint；`open_functions` 四项 open clause／default／interleaved domain 过滤检查退出 0（`/tmp/luna-tools-a06zyus1/result.json`，完整过滤名见报告）。已改 Rust 的 scoped rustfmt 检查退出 0（`/tmp/luna-tools-ssw__ww1/result.json`）。本阶段移除了准备帧对临时望远镜／默认环境的借用，不代表 VM Type 响应恢复完成 |
| 输入完成消费者与 callable 形成 | 在 `/home/fin/luna` 经 `python3 -B development/check.py --timeout 1200 -- test -p elaboration --test invocation_budget -p luna --test declaration_input_plans --test normal_input_plans --test ordinary_callable_types --test ordinary_clause_collectors --test required_input_scopes --test library_std_format --no-fail-fast -- --test-threads=1`，`/tmp/luna-tools-015xk8ez/result.json` 退出 0：106 项通过，两个原待定 Type 正例保持 ignored。在 `/home/fin/luna` 经 `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -- --test-threads=1 declaration_inputs dynamic_instances`，`/tmp/luna-tools-tlsy0sji/result.json` 退出 0，17 项通过，含前缀覆盖隔离、预算耗尽／重试及 completion dump。`check -p elaboration` 退出 0（`/tmp/luna-tools-pctqrnjy/result.json`）；本轮六个 Rust 文件的 scoped rustfmt 检查退出 0（`/tmp/luna-tools-lq_znlsz/result.json`）。新增集成用例最初使用 `requires copied == seed`，在取得函数值时触发已有的程序前置条件擦除拒绝（`/tmp/luna-tools-0_cbhuyo/result.json`）；用例随后将形成检查改为独立的 `requires T == u8`，保留两个父实例、调用者显式输入和恰好一次执行断言，另检查非法 T 的具体诊断。程序前置条件随函数值传递仍未实现，不将该拒绝视为修复。 |
| 路由与源码适配 | `program/input_routing` 单元测试及 `luna/tests/{default_calls,type_spreads,cmtt_methods}.rs`；`/tmp/luna-tools-2x37n5o8/result.json`，退出 0，仍含两项 ignored |
| required 依赖发现 | `program/required/dependencies.rs` 及 `luna/tests/required_input_scopes.rs`；`/tmp/luna-tools-zrknan4h/result.json`，退出 0 |
| 帧绑定交接 | `bytecode::input_preparation_tests` 及 `luna/tests/clause_packet_preparation.rs`；`/tmp/luna-tools-r52uh6us/result.json`，退出 0 |
| 块／if 与探测 | `luna/tests/{contextual_input_regions,cmtt_storage_world,native_optional,error_surface,contextual_delivery,effect_spreads}.rs`；`/tmp/luna-tools-4qp6q76r/result.json`，退出 0 |
| contextual when 后继 | 在 `/home/fin/luna` 经 `development/check.py --timeout 900 -- test -p luna --test contextual_input_regions -- --test-threads=1` 运行，`/tmp/luna-tools-5a7k5fq6/result.json` 退出 0；随后新增 return 用例以同命令加 `contextual_when_returns` 过滤，`/tmp/luna-tools-_l3ix78q/result.json` 退出 0，共覆盖当前 19 项。包括真实 sysroot、两引擎顺序对照及深嵌套字节码路径；不是外部 Type 恢复验收 |
| 检查帧 dump | 同工作目录与入口，`test -p elaboration --lib program::block_regions -- --test-threads=1`，`/tmp/luna-tools-j2oi2f4c/result.json` 退出 0，8 项通过；检查待检查条件／selector、已完成分支和转义后的 S-expression |
| contextual match 后继 | 同工作目录与入口，`test -p luna --test contextual_input_regions --test normal_patterns --no-fail-fast -- --test-threads=1`，`/tmp/luna-tools-97cot4g5/result.json` 退出 101：contextual 22 项通过，normal_patterns 41 项通过、1 项失败；新增用例验证 selector／失败 guard 顺序、模式事实隔离、覆盖性与提前返回 |
| 扩展分支验收 | `/tmp/luna-tools-j385q075/result.json`：490 通过、1 个新 fixture 阶段声明错误；修正后该用例 `/tmp/luna-tools-qe9dsshx/result.json` 退出 0；32 层 CLI 用例 `/tmp/luna-tools-sjdhryg0/result.json` 退出 0 |
| 比较后继与共同操作数搜索 | 在 `/home/fin/luna` 经 `development/check.py --timeout 900 -- test -p luna --test contextual_input_regions --test ordering_protocol --test control_boundaries --no-fail-fast -- --test-threads=1` 运行，`/tmp/luna-tools-v_kmth4v/result.json` 退出 0，35 项通过；涵盖左右上下文补足、块与分支作用域、原 return 出口及一次性生产顺序 |
| 比较协议、资源及 dump 回归 | 同目录与入口，`test -p luna --test arithmetic_contexts --test equality_protocol --test comparison_resources --test library_comptime_comparison --no-fail-fast -- --test-threads=1`，`/tmp/luna-tools-a43rzqwr/result.json` 退出 0，36 项通过；`test -p elaboration --lib -p luna --test contextual_input_regions --no-fail-fast -- comparison_regions peer_progress_dump --test-threads=1`，`/tmp/luna-tools-3g6y5qi2/result.json` 退出 0，dump 与两个新增源码用例通过，后两项亦包含在上一行中 |
| 逻辑运算后继、短路前提与宿主回归 | 在 `/home/fin/luna` 经 `python3 -B development/check.py --timeout 900 -- test -p luna --test contextual_input_regions --test pure_bitvec --test nesting --test control_boundaries --test library_std --test library_std_format --no-fail-fast -- --test-threads=1` 运行，`/tmp/luna-tools-egjfuqg3/result.json` 退出 0，95 项通过；nesting 的子进程入口单独标为 ignored，由父测试实际调用。覆盖短路与 BitVec 次数／顺序、类型前提退出、上下文失败回退、原 return 目标、48 层混合嵌套及 LLVM O0/O3。相同入口运行 `test -p elaboration --lib -- logical_frame_dump --test-threads=1`，`/tmp/luna-tools-5s9x59gq/result.json` 退出 0；已修改 Rust 文件 scoped rustfmt 检查退出 0，`/tmp/luna-tools-snlvmkvx/result.json`。完整格式化接口与外部 Type 恢复不在本组完成范围内 |
| VM 主体阶段的代码所有权 | 在 `/home/fin/luna` 经 `development/check.py --timeout 900 -- test -p comptime_interpretor --lib -p luna --test invocation_host_requests --test method_host_requests --test prepared_invocations --test declaration_contracts --no-fail-fast -- --test-threads=1` 运行，`/tmp/luna-tools-oki5n2vh/result.json` 退出 0，718 项通过；涵盖两条执行路径、契约失败／取消／迟到响应、原参数存储及代码缓存与调用结果隔离。源码待定 Type 的两个 ignored 未在此组重跑，不据此声称源区域恢复已完成 |
| 受检声明／主体拆分：编译与核心实现 | 基线 `115d2d9e` 加本次工作区，在 `/home/fin/luna` 使用 `python3 -B development/check.py --timeout 900 --`：`check --workspace --all-targets` 退出 0，`/tmp/luna-tools-65gozkq3/result.json`；`test -p nir -p iir -p materialization -p comptime_interpretor -p elaboration -p program_verification -p type_resolution --lib --no-fail-fast -- --test-threads=1` 退出 0，1,183 项通过，`/tmp/luna-tools-h1ufip5h/result.json` |
| 受检声明／主体拆分：所有权、替换与预算 | 同目录、同入口运行 `test -p nir -p iir -p materialization -p elaboration -p type_resolution --test function_declarations --test copy_budget --test phases --test types --test runtime_instances --test dependent_staging --test expansion_boundary --test scoped_dispatch_inputs --test type_computation_demands --test callable_world --no-fail-fast -- --test-threads=1`，退出 0，189 项通过，`/tmp/luna-tools-_otv3vp2/result.json`；独立声明测试覆盖无主体的契约／默认值替换、失败后重试、输入身份与 Display 转义 |
| 受检声明／主体拆分：源码与公开库 | 同目录、同入口运行 `test -p luna --lib --test declaration_contracts --test prepared_invocations --test scoped_comptime_callables --test sysroot_preludes --test quote_functions --test library_std_format --test library_std --no-fail-fast -- --test-threads=1`，退出 0，264 项通过，`/tmp/luna-tools-2vvq0vyr/result.json`；含真实 sysroot、LLVM 宿主输出、文件资源、格式扫描、quote 展开及两阶段方法。两项待定 Type 的 ignored 未在此组重跑；格式扫描不等于完整 println |

本次修改文件的 scoped rustfmt `--check` 退出 0，`/tmp/luna-tools-zjfbc4e9/result.json`。
上述是本次选定检查的证据，原有工作区改动保留，不称为干净 HEAD 或全语言验收。

match 迁移发现的失败为 `normal_record_defaults_follow_evaluated_value_prefixes_once`。
临时撤回本次四个生产文件的补丁后，在 `a68e3087` 源码上按同入口执行
`test -p luna --test normal_patterns normal_record_defaults_follow_evaluated_value_prefixes_once -- --exact --test-threads=1`，
`/tmp/luna-tools-hwu9z154/result.json` 同样退出 101，仍报 `215..222` 的 comptime
局部输入 `(local 4294967295 1)` 不可用。已原样恢复补丁并检查一致性。这个对照
证明缺口先于 match 迁移；下述声明默认代码交接修复其根因，原合法用例保持不变。

#### 普通 when 的共同帧与静态观察

在 `/home/fin/luna` 经 `python3 -B development/check.py --timeout 900 --` 运行：

- `test -p elaboration --lib -p luna --test contextual_input_regions --test execution --test inline_control --test comptime_clause_inputs --no-fail-fast -- when --test-threads=1`：
  `/tmp/luna-tools-t5qn7lqp/result.json` 退出 0，30 项通过。新增嵌套 callee 求值次数、
  有序前提隔离和提前返回用例；32 层源码与字节码成功，8 层作双引擎对照。
- `test -p elaboration --lib -- normal:: block_regions --test-threads=1`：
  `/tmp/luna-tools-kdm9zsgd/result.json` 退出 0，33 项通过，覆盖帧 dump、值观察的
  未知／预算边界、普通列表域及 opaque head 身份。
- `test -p luna --test execution --test contextual_input_regions --no-fail-fast -- partial_ --test-threads=1`：
  `/tmp/luna-tools-c8chfow2/result.json` 退出 0，40 项通过，覆盖部分输入候选搜索、
  嵌套失败恢复、错误结果与分支交付。

- `test -p luna --test clause_packet_preparation --test scoped_input_observation --test captured_input_observations --no-fail-fast -- --test-threads=1`：
  `/tmp/luna-tools-97qdg8up/result.json` 退出 101；前两组 19 项通过，捕获输入用例在
  创建 VM 时因没有提供源码查询服务而报 `UnmaterializedDispatch`，尚未执行默认值。
  将该 fixture 迁至公共 `sysroot::with_comptime`，仍检查三次调用的默认值次数、
  显式供应跳过默认值、两引擎结果及错误域拒绝。
- 最终 `test -p luna --test captured_input_observations --test nesting --no-fail-fast -- member_defaults_observe_parent_value_indices_in_their_annotations nested_when_preserves_return_through_the_pipeline --test-threads=1`：
  `/tmp/luna-tools-y45fxzwn/result.json` 退出 0，两项通过；runtime 32 层 when 分别
  经过 AST、HIR、NIR 与执行。没有重复全 workspace 测试。

本阶段修改的 Rust 文件定向 rustfmt `--check` 退出 0，报告为
`/tmp/luna-tools-il7y6hvf/result.json` 与 `/tmp/luna-tools-7_s2ojio/result.json`；
`git diff --check` 通过。上述检查不覆盖外部 Type 恢复或完整格式化 println。

初次 32 层 comptime 源码用例在检查后的值观察中发生宿主栈溢出，
`/tmp/luna-tools-1rds7ruo/result.json` 退出 101；诊断版日志
`/tmp/luna-tools-qhmg4bh6/stderr.log` 确认全部 when 帧已退出，随后普通值观察在
深度 62 溢出。静态控制路径迭代化后该崩溃已消除。参考树解释器仍受独立的 64 层
宿主递归保护，不把扩大 invocation 预算当作修复：生产深度限制未改，32 层字节码
验收保留，双引擎对照明确限制为 8 层。临时诊断输出已移除。

#### 普通 match 的共同帧、阶段绑定与清理来源

工作目录 `/home/fin/luna`，以下均使用 `python3 -B development/check.py --timeout 900 --`：

- `test -p elaboration --lib -p luna --test contextual_input_regions --test inline_control --test error_surface --test nesting --no-fail-fast -- match --test-threads=1`：
  `/tmp/luna-tools-poo1e_8m/result.json` 退出 0，43 项通过。新增真实 sysroot 的嵌套
  callee／失败 guard 顺序、模式前提隔离和提前返回；32 层源码、字节码及 runtime
  AST/HIR/NIR 流水线通过，8 层作双引擎对照。
- `test -p luna --test execution --test normal_patterns --test effect_spreads --no-fail-fast -- match pattern partial_ --test-threads=1`：
  `/tmp/luna-tools-9f7s6c9m/result.json` 退出 101，176 项通过，1 项失败；失败为
  `computed_value_pattern_failure_stops_before_later_candidates` 的预算前提，见下文。
- `test -p luna --test drop --test inline_control --test contextual_input_regions --no-fail-fast -- scheduled_match_presence whole_enum_group_matches candidate_order_patterns_cross_variants ordinary_copy_pattern_guard_and_or inactive_inline_pattern inline_patterns_execute_nested_guards pure_inline_pattern_guards partial_probe_recovery partial_branch_search --test-threads=1`：
  `/tmp/luna-tools-wse350iz/result.json` 退出 0，14 项通过，覆盖选中 arm 清理、guard
  失败后的所有权保留、重复匹配、inline 阶段边界及探测失败恢复。

失败用例给予 VM 200 步，却要求已经进入首个候选的无限循环。实际轨迹仅到
`choose`，预算在准备 `first` 时耗尽。保存本阶段六个已修改生产文件后，临时采用
`3d3ff138` 的对应内容，以同入口执行 `test -p luna --test execution -- computed_value_pattern_failure_stops_before_later_candidates --exact --test-threads=1`，
`/tmp/luna-tools-k7p4kfp0/result.json` 退出 101，同样只有 `choose`；随后逐字节恢复
本阶段文件，未触及其他工作区改动。这不是 match 迁移引入的行为变化。

保留该用例的 `StepsExhausted` 和精确 `[choose, first]` 轨迹断言，测试预算改为
10,000，确保检验的是首个候选主体失败后不再尝试后续候选，而不是调用准备的固定
工作量。最终 `test -p elaboration --lib -p luna --test execution --no-fail-fast -- computed_value_pattern_failure_stops_before_later_candidates block_regions --test-threads=1`，
`/tmp/luna-tools-aw2pquwy/result.json` 退出 0，13 项通过。

扩充部分输入测试中的两个 match 场景后，`test -p luna --test contextual_input_regions -- partial_branch_search --test-threads=1`，
`/tmp/luna-tools-b0m502i0/result.json` 退出 0；包含 null arm 等待后续 u8 arm 提供
上下文、嵌套模式局部、两引擎及生产者一次性顺序。生产检查与执行预算规则未放宽。

公开库回归 `test -p luna --test library_std --test library_std_format --no-fail-fast -- --test-threads=1`：
`/tmp/luna-tools-s67pbew_/result.json` 退出 0，18 项通过。覆盖真实 sysroot 的文件／
错误结果、writer 借用与失败传播、描述符清理、文本输出、跨阶段构造和格式串扫描；
不把这些结果记作完整 `defer std.fmt.Parse(format)` 的交付。
本阶段修改 Rust 文件的定向 rustfmt `--check` 退出 0，
`/tmp/luna-tools-s88klgjl/result.json`；`git diff --check` 通过。没有修改 docs 或
其他既有工作区改动，也没有放宽资源、覆盖或模式准入规则。

#### 部分输入分支的探测后继与错误行上下文

`when` 的 ProbeCondition／ProbeBody 和 `match` 的 Inferring 状态现保留子检查
之后的位置；失败沿 Region 退出内部帧后，由最近的 Probe 消费并恢复。正式 arm
与探测 arm 共用源规则，没有候选专用的表达式解释器。原有搜索预算和 numeric
默认策略保留；词法与存储回滚不退还已消耗的检查工作。

新增错误结果用例发现一项入口遗漏：`argument_attempt_with_input` 及普通调用的
候选重试筛选没有包含 `InputContext::ErrorResult`。于是 `!bool T` 的实参分支在
T 未定时进入无目标检查，首个 `error true` 的 NoReturn 成功域成了后续分支目标，
合法的 u8 成功分支被拒绝。两处入口现与已有结构输入规则对齐，将已知错误行和
待定成功域交给共同检查；不更改错误结果 ABI、错误行允许范围或类型相等规则。

工作目录 `/home/fin/luna`。在临时采用 `a61a15f7` 的四个对应生产文件时执行
`python3 -B development/check.py --timeout 900 -- test -p luna --test contextual_input_regions -- partial_arm_probes_preserve_error_delivery_after_domain_selection --exact --test-threads=1`，
`/tmp/luna-tools-kh08lb7z/result.json` 退出 101，复现同一 incompatible-success-type：
输入 u8，目标 `!bool NoReturn`。随后逐字节恢复本阶段四个文件。这证明该错误先于
探测帧迁移存在；修复的是实际调用路径，不把拒绝固化为测试预期。

首批新增测试 `/tmp/luna-tools-qu8s99cf/result.json` 退出 101：错误行入口失败如上；
另一项是测试把错误成功模式 `flag!` 写到了可选值上，已按语言规则改为 `flag?`。
同轮候选切换、拒绝用例及 32 层混合分支 AST/HIR/NIR 和 runtime 检查通过。
修正入口后的两个新增行为测试以
`python3 -B development/check.py --timeout 900 -- test -p luna --test contextual_input_regions -- partial_arm_probes --test-threads=1`
通过，`/tmp/luna-tools-32hroree/result.json` 退出 0，覆盖三组类型前提和五组错误
结果分支的成功／失败路径，两引擎精确调用次序保持一致。

扩大回归使用 `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -p luna --test contextual_input_regions --test inline_control --test error_surface --test nesting --no-fail-fast -- partial_ block_regions when match --test-threads=1`：
`/tmp/luna-tools-jlu3vfj1/result.json` 退出 101，86 项通过，1 项失败。该失败在新增
拒绝测试的诊断文字断言：实际已拒绝 u8 错误载荷，报告 expected bool／found u8，
却不含断言要求的单词 error。将测试改为核对具体类型冲突，不修改生产诊断或准入。
定向重跑 `partial_arm_probes_restore_return_targets_and_reject_foreign_error_rows`
（同一 check.py 入口，`--exact --test-threads=1 --nocapture`），
`/tmp/luna-tools-9tig0mpn/result.json` 退出 0；提前返回仍跳过 byte 和 keep，三种
分支中的外来错误行均被拒绝。

补入分支外使用探测前提的拒绝例后，定向重跑
`partial_arm_probes_keep_type_facts_private_and_preserve_execution_order`，
`/tmp/luna-tools-8klnnejb/result.json` 退出 0，证实 when 的负前提与 match 的
类型模式前提不会扩大外层泛型值的权限。修改 Rust 文件的最终定向 rustfmt
`--check`（通过 `development/command.py --timeout 120`）退出 0，
`/tmp/luna-tools-m6pirfbu/result.json`；`git diff --check` 通过。
最后以 `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test library_std --test library_std_format --no-fail-fast -- --test-threads=1 --nocapture`
复核错误结果入口和公开库路径，`/tmp/luna-tools-8pzt0wr3/result.json` 退出 0：
121 项错误结果、14 项 std 宿主、4 项格式扫描测试通过。覆盖错误传播、writer
借用、描述符清理、跨阶段构造与原生输出，不代表完整格式化变参接口已经实现。
此处不把未完成的外部 Type 等待计作交付；docs 与其他既有工作区改动保持原样。

#### 声明默认代码与调用前缀的交接

上述默认值失败揭示了两种环境不能互换。`Config<enabled>` 在默认值所属声明中
引用的是合法的抽象形参；在调用输入准备阶段，`enabled()` 的临时绑定只是已经
检查的生产者，还不是已供应的值。把前者直接改为后者再调用 `type_expr`，会令
required 捕获一个尚不存在的值。扩大捕获权限、提前执行生产者或给临时绑定塞入
一个 rigid 都不能解决这个所有权错误。

NF 输入计划的迁移采用声明侧检查结果作为代码：保留形参身份与泛型域，将代码的
自由形参局部连接到本次计划的输入绑定，内部局部、闭包捕获、模式及控制身份按
现有通用 freshening 规则改名。连接只允许使用已有的前序输入位置，不使其变为
completed；已经观察到的身份可以代入，尚待执行的前缀继续由 VM 输入计划取得。
VM 在执行该默认值前，使用已经准入的前缀实例化代码中的域，并按实际接收域检查
结果。显式输入仍保持源码顺序，缺省输入仍排在显式输入之后按声明次序执行。

声明验证与调用处代码准备必须共用同一个默认表达式检查入口，保留自身／后续
形参引用禁令。声明侧事实与调用方的控制、输入观察不能混用；默认代码的内部局部
也不能在两次展开间共享身份。这里不新建默认结果缓存，也不把已执行调用的值写回
声明模板。当前准备仍重新检查声明侧源码；稳定模板查询的复用与失效属于后续工作。

回归入口还须保留真实源码查询服务。`normal_input_plans` 的三个默认值用例原先
使用不带 resolver 的 `Interpreter::for_comptime`；在 `c799dc9b` 原生产文件上复测
`/tmp/luna-tools-2w26g_0a/result.json`，三项同样在 VM 构造时失败，错误为
`UnmaterializedDispatch`。候选生产文件随后按字节恢复。现在这组及
`open_normal_input_plans` 统一调用现有 `sysroot::with_comptime`，使用同一受检
包的 Type、方法和关联项查询；两种引擎和全部行为断言保留。

这些语义用例使用该支持入口统一的 `COMPTIME_BUDGET`，包含声明保留、前提建立、
VM 准入和执行。旧的 100,000 步在接通查询后仍于 VM 准入耗尽，证据为
`/tmp/luna-tools-_xo0_yen/result.json`；没有改生产 `Budget::default`，也没有为
原 192 次 collection 调用成本用例提高其显式 200,000 步额度。预算内通过只证明
对应用例可执行，不据此宣称整体 comptime 成本已达标。

扩展验证 `/tmp/luna-tools-il07i1sw/result.json` 中，`normal_patterns` 42 项通过，
`normal_input_plans` 23 项通过、上述成本用例失败；`open_normal_input_plans` 4 项
通过，缺失必填参数的诊断回归一项失败。后者定位为把全部缺失槽都路由到默认代码，
现已将该路径限定为 Optional 槽，保留必填参数的原有缺失诊断。

成本失败已在 `c799dc9b` 的两个原生产文件、保留修正后的源码查询测试入口上单独
对照：`/tmp/luna-tools-cebk7w_v/result.json` 同样退出 101，仍在调用阶段报
`StepsExhausted`，source 为 `1095..3683`。已恢复候选文件并逐字节核对。
这不是默认代码迁移引入的差异。随后定位为源码投影复制与 VM 准入重复遍历；借用
声明视图后，原 200,000 步用例与 normal_input_plans 整组 24 项在
`/tmp/luna-tools-ysz7n68a/result.json` 中通过。该次命令还有一个 builder harness
失败，不能称整次命令通过。成本明细与后续修复验证见
[源码声明视图](world-storage-and-lifetimes.md#110-源码声明视图与物理执行表分开)，
没有通过提高该用例额度掩盖成本。

最终针对性检查在 `/home/fin/luna` 执行：

```text
python3 -B development/check.py --timeout 900 -- test -p luna \
  --test contextual_input_regions --test normal_patterns \
  --test open_normal_input_plans --no-fail-fast -- --test-threads=1 \
  declaration_default normal_record_defaults suspended_collection_routes
```

`/tmp/luna-tools-82o4tbxw/result.json` 退出 0：新增四项真实 sysroot 用例、原失败
用例以及整项路由拒绝检查均通过。新增用例在两引擎检查不同前缀与闭包捕获、显式
输入次序、覆盖跳过默认副作用、后续默认依赖、错误域的 `TypeContractRejected`、
失败后下一次调用恢复，以及自身／后续参数和无依据泛型等式的拒绝。五个改动 Rust
文件的 scoped rustfmt check 通过，报告 `/tmp/luna-tools-07fw1j0i/result.json`；
`git diff --check` 通过。首轮新增 fixture 的块／尖括号词法边界和诊断断言已修正，
原有回归的行为预期与成本额度未改。

这只能处理已能在声明环境形成受检代码的默认值。依赖外部 Type 生产者才能继续
检查的源码、普通调用的待定参数域、完整 `std.fmt.Parse` 描述准入仍需要源区域的
挂起和恢复，不能从这条默认值路径推导出它们已完成。

另有 32 项 luna elaboration 集成失败在 Attempt 迁移时与 `53696cc6` 基线逐例对照
一致，报告 `/tmp/luna-tools-o47vegep/result.json`；本轮未复测，仍待处理。不能把其中
诊断断言失败都认作测试过时；九项名义动态输入缺口另见[晚报回复](audit-reply.md)。
这些历史验证包含当时工作区原有资源分析修改，不宣称为纯提交验收。

此前在 `/home/fin/luna`、`6a564ae5` 加保留的已有修改上执行：

```text
python3 -B development/check.py --timeout 900 -- test -p luna \
  --test ordinary_callable_types --test ordinary_clause_collectors \
  --no-fail-fast -- --ignored
```

`/tmp/luna-tools-8_n2q3og/result.json` 退出 101，两项正例均失败，分别为
`159..181: comptime callable signature still depends on an invocation-time clause input`
及 `191..201: list literal requires an element type context`。未改预期或 ignore，未运行
全 workspace。它们证明源接线仍缺失，不说明 VM 的既有宿主请求已经失效。

## 5. Observe、登记与发布

ObservationKey 包含语义主体、所请求 facet、作用域/视图和当前前提；它与 task key 不同。
例如 `ParameterDomain(f,2)`、`TypeHead(t)`、`Layout(t,target)` 请求不同证据。

```text
observe(key, premise_scope) ->
    Known(value, evidence, dependency_stamp)
  | Pending(producer_id, observed_generation)
  | Unresolved(reason, source)
  | Failed(diagnostic_id)

wait(task, observed):
    register task-generation against producer-generation
    re-read producer status and generation
    if changed or terminal: enqueue task-generation once
```

发布先构造完整 update 并校验输入代次，再原子安装结果/赋值/依赖，最后收集通知。
通知只入队，不同步重入尚在更新的 checker/solver。union-find 合并要迁移或转发 watchers，
rollback/producer failure 必须使旧观察失效，不能仅在成功赋值时通知。

一个任务有多个依赖时，唤醒只意味着重新观察；按 task-generation 去重，不能每条边
重复恢复 VM。被取消 task 的票据失效；共享生产者只在没有剩余需求且允许取消时释放。

## 6. CandidateTrial 的提交协议

`Created → Checking → Applicable | Excluded | Undecided`。
只有 Applicable 可在选择条件成立后进入 Committed；任何未提交试探都可以 Discarded。
Undecided 仅在新增真实信息或额度后重新进入 Checking；Excluded 不可提交。
Applicable 只是试探成功；更精确候选未决、同精度冲突未排除时仍不能 commit。

trial 保存 base RegionRevision、SemanticView、局部赋值、原始义务、term holes、诊断缓冲
和依赖摘要。对外只可观察试探状态，不可导出尚未成立的值作为共享事实。

commit 分为 prepare 与 install：

1. 核对 parent/视图与候选 universe；变化时重证，不自动合并两个试探的赋值。
2. 检查全部本次必检义务、scope/sort/occurs、原始类型关系及 selection 规则。
3. 准备合并后的推断、term result、证据及诊断处置，保证 install 不需要失败性检查。
4. 一次安装，递增 region revision，然后通知受影响观察。

preflight 失败不修改 parent。安装阶段不运行用户 comptime 代码或新宿主请求。
未完成的其他 region 义务可以继续存在，但不能把本候选未完成的必检义务当成别人的任务。
允许 residual 的位置必须有独立语言依据及稳定描述，不以 CandidateState=Undecided 导出。

被放弃候选的内部诊断仅用于最终无解解释；明确源执行产生的编译诊断不能随候选丢弃。
后者因此不能在未选定候选里投机运行。PTC 学习可保留，但须保留 world/activation guards。

## 7. VM 请求：进度身份不等于结果缓存键

```text
RequestId = (VmSessionId, ActivationId, ExecutionOrdinal)
RequestRecord {
  request_id, operation, semantic_view,
  source_execution_owner, input_snapshot,
  continuation, destination, budget_lease, status
}
```

ExecutionOrdinal 标识一次动态执行。循环重复经过同一个 PC 必须产生不同 request；
同一次暂停重试则使用原 request。用 `(function,PC)` 去重会吞掉合法循环行为。
相同实参的两个不纯调用也不是同一个 request。请求输入不能被存储别名在等待中改写。

宿主 operation 类别区分：只读语义观察、纯 invocation、定义准备/发布、显式外部行为。
每种操作声明接收域、输出域、依赖、取消方式与是否可在候选中执行；不是一个万能 callback。
宿主需要另一次 invocation 时建立子 session/请求边，不递归借用同一 VM 的可变状态。

| 请求状态 | 含义 | 转移约束 |
| --- | --- | --- |
| Prepared | 输入已取得、源控制已允许，尚未发送 | 可安全取消，不能提前改变世界 |
| InFlight | producer 正在处理 | 再次 poll 只读取状态，不重发 |
| WaitingOnInput | 宿主有真实依赖生产者 | 子依赖变化后继续原请求，不重跑 VM 前缀 |
| Completed | 结果及依赖已保存，尚未注入 frame | 验证目标 session/activation/destination 后交付 |
| Delivered | frame 已取得结果，PC 已前进 | 重复响应不再写槽、不再执行 |
| Failed | 执行或接收契约失败 | 传播失败及已消耗预算，不尝试另一执行载体 |
| Cancelled / Indeterminate | 已取消，或外部行为完成情况未知 | 不能当成“未执行”自动重试 |

语义失败响应不能恢复成普通 Value；预算原因保留原来源。响应校验失败不回退重跑代码。
Completed→Delivered 对槽写入、进度前移和请求终态是一个局部原子动作。
generation 不匹配的旧响应不触碰新 activation；其宿主资源仍按 request owner 回收。

输入传输采用冻结值或受检的草稿快照。可变 self 的 VM lens 留在本 session；宿主仅拿
所需不可变描述，返回值由 session 在响应边界写入约定结果位置。不能把 lens 序列化成
comptime 指针，也不能从另一 session 回写。这里的暂停位置保护不改变普通嵌套调用中
已有的值引用更新和 RHS 求值规则。

### 普通函数契约的恢复边界

普通函数的前后置条件原先在 `prepare_invocation` 和 `finish_invocation` 内同步
调用树求值器。即使主体支持宿主请求，前置条件中的关联值选择也会在返回 session
之前尝试同步解析，后置条件则在主体完成的交付栈上同步执行。接口调用的
`Execution::Contract` 已能使用字节码，不能把这两条路径混为一谈。

普通声明现由同一 activation 持有 `Entry(index) → Body → Exit(index,result,returned)`。
每次只准备一个条件的普通 CheckPredicate 表达式，交给已有执行驱动；完成后保存
环境并推进原 activation。主体仅在所有前置条件成功后进入一次，后置条件保留已检查
的实际返回值，显式 return 不绕过后置条件。类型／capability 准入仍在各自原边界完成。

tree 参考执行和字节码驱动共用这套进度及完成规则。字节码路径在条件内遇到关联
请求时保留 PC、环境、调用者及当前条件索引，恢复不重跑前序条件或主体；Display
显示当前阶段、下一个条件和保留结果。body 的稳定 code 身份不能用作条件代码的键。
函数进入事件和 bytecode invocation 计数只计主体，不把每个条件计成新的函数调用。

失败及取消沿同一 activation 恢复链退出，不执行剩余条件；未交回的宿主预算不会
凭取消而退还。runtime 的 scheduled result／清理检查仍由原资源计划负责，这项
迁移没有去掉字节码的 cleanup/effect 门控，也没有让 PTC 执行程序谓词。

源码测试 `invocation_host_requests` 使用真实六包、SourceInvocationQueries 和外置
宿主响应，覆盖实例化 Type 参数的契约、闭合及泛型关联初始化器、代码会话复用。
泛型初始化器在前后置条件中各执行一次；缓存选择与代码不缓存读取的值或契约结论。
VM 局部测试另覆盖前后置条件拒绝、提前 return、等待、过期响应、预算耗尽和取消。

本项只接通普通程序契约的执行进度，尚非通用前端检查／发布宿主协议。参数准入、
类型契约及某些准备服务仍同步；本阶段原有的编译拒绝回退已由下节清理。
签名依赖实际 invocation 产物的两个普通调用正例仍未解锁。

验证在 `/home/fin/luna` 通过 `development/check.py --timeout 900 -- test`：

- 保留新回归，将两个生产文件临时还原到 `4ae19727`，精确运行
  `-p comptime_interpretor --lib invocation::host::tests::contracts::entry_body_and_exit_share_a_suspendable_activation -- --exact`：
  `/tmp/luna-tools-2u3witg2/result.json` 退出 101，start_invocation 提前报
  UnmaterializedDispatch，尚未产生可回答的宿主请求。随后按摘要恢复改动。
- `-p comptime_interpretor --lib --no-fail-fast`：
  `/tmp/luna-tools-kbbm25fp/result.json` 退出 0，579 项通过。
- `-p luna --test invocation_host_requests --no-fail-fast`：
  `/tmp/luna-tools-uba4431g/result.json` 退出 0，五项源码回归通过。
- 最终相关组使用
  `-p luna --test invocation_host_requests --test integer_contracts --test snapshot_contracts --test predicate_calls --test effect_contracts --test prepared_invocations --test staged_invocation_reuse --test default_calls --test comptime_bytecode_defaults --test conditional_capture_cleanup --test library_protocols --test builder_invocations --no-fail-fast`：
  `/tmp/luna-tools-m_ofiefj/result.json` 退出 0，216 项通过，一项原有 predicate
  phase-aware stability 用例仍 ignored。先前新源码 fixture 缺少调用前提证明的拒绝
  已通过改为直接调用带契约的公开 invocation 入口纠正，没有放宽源码程序验证。

七个修改 Rust 文件的格式检查 `/tmp/luna-tools-hfak84ny/result.json` 退出 0，
`git diff --check` 通过。未运行全 workspace、LLVM/native 或性能基准；工作区仍包含
作者原有资源分析修改，未将其改写或纳入提交。正式 docs 与 formatter 开发保持独立。

### 编译拒绝与执行模式分开

字节码编译失败原先返回 `Ok(None)`；声明、借用体、owned body 和 contextual input
代码缓存因此都能保存“没有程序”的成功条目。调用者会据此进入树执行，后续调用则
复用这次回退决定。它把编译错误、缓存未命中和显式参考执行混为一个协议。

现在编译返回可执行代码或带来源的错误。Unsupported、预算、深度、控制流和未解
类型证据分别保留错误类别；失败消耗的编译工作照常计费，不能发布代码缓存条目。
代码缓存的条目类型只容纳可执行程序，缓存未命中仍允许重新编译。执行入口的
`Option` 只表达现有 mode/schedule/effect 门控，不能由编译拒绝产生。

主 ExprKind 分派改为穷尽匹配，标量编译也进入该分派。尚未实现的节点逐项列出，
新 IR 变体必须做显式 lowering 决定。报告列出的十三种节点不能直接算作十三项
合法 comptime 能力缺失：

- 指针操作被 comptime 执行阶段拒绝；runtime 指针 Type 描述和 quote 不因此禁用。
- effect 当前在 phase 检查与 comptime 树执行中均明确未实现。这里没有把这个
  实现限制上升为最终语言永远禁止 comptime effect 的规则。
- StagedValue 尚未完成，不能作为可执行常量；TemplateEqual 服务 runtime 的
  staged callable 身份分派。换引擎不能完成其阶段证据。
- 原 wildcard 还隐藏了 guarded 分支的剩余情况：非法域 and/or、携带参数绑定的
  NF Call。后者在树执行也报 InvalidCallPlan，必须补齐实际准备协议，不能靠回退。

回归直接送入有赋值前缀的未完成 IR，验证编译拒绝保留原来源，未启动输入执行，
声明／借用／owned 缓存不产生负条目；同一 VM 后续合法代码仍可执行。这个 IR 探针
不是合法源码用例。缓存容量、LRU、过期弱身份、时钟耗尽测试改用真实可执行程序，
不再以失败条目当作缓存载荷。显式 tree 参考执行仍单独验证。

本项没有声称 tree 求值完全退出生产路径。nested staged input 的 depth 门控、
cleanup/effect 模式选择及同步准备服务尚在；通用前端 Region 恢复仍未完成。

本阶段在 `/home/fin/luna` 的实际验证（`development/check.py --timeout 900 -- test`）：

- `-p comptime_interpretor -p comptime_compiler --lib --no-fail-fast`：
  `/tmp/luna-tools-ul7bbp65/result.json` 退出 0，分别 580、7 项通过。
- `-p luna --test invocation_host_requests --test prepared_invocations --test staged_invocation_reuse --test default_calls --test comptime_bytecode_defaults --test conditional_capture_cleanup --test library_protocols --test builder_invocations --test cmtt --test cmtt_methods --test cmtt_type_inputs --test comptime_pointer_phase --test comptime_staged_values --test normal_form_domains --test comptime_bytecode --no-fail-fast`：
  `/tmp/luna-tools-lxvqpcqr/result.json` 退出 101，982 项通过，一项失败，无 ignored。
- 唯一失败是 `cmtt::generator_clause_domains_depend_on_prior_value_indices`，
  在 `118..122` 捕获 `(local 4294967295 1)` 时拒绝 required 输入。临时将三个
  生产文件恢复为 `59405db5` 后以 `-p luna --test cmtt generator_clause_domains_depend_on_prior_value_indices -- --exact`
  单独复测，`/tmp/luna-tools-g42rqpk1/result.json` 退出 101，出现相同错误。
  已恢复当前三个文件并按 `/tmp/luna-bytecode-rejection-baseline/restore.json`
  核对内容摘要。此为已有前端依赖输入捕获失败，本阶段未修改用例、放宽预期或修复它。
- 四个改动 Rust 文件的 scoped rustfmt 检查
  `/tmp/luna-tools-r_tyye2_/result.json` 退出 0；`git diff --check` 通过。

上述为工作区验证，仍含作者原有资源分析改动；没有执行全 workspace 或性能测量。

### required 输入证据的区域所有权

上述 CMTT 失败的来源现已进一步定位。外层 required preparation 已为 `flag` 供应
具体 bool，并建立普通局部绑定；默认值 `Flag<flag>{}` 的名义类型检查递归进入
required preparation 时，只继承了 binding/domain，没有继承值。于是内层把一次
已有身份观察变成另一次没有输入的 invocation。修复点在准备区域的输入交付，
不在 generator、Flag、List 或某个源码形状上。

`required::inputs::Scope` 现在按活动准备区域持有只读输入证据。每个 frame 保留
`LocalId → (domain, supplied value / unavailable)`，父 frame 用 Arc 保活；没有
pending producer、VM 可变存储或候选等待者，也不进入 RequiredWorld 或代码缓存。
子区域在形成后续参数域之前逐项登记已完成前缀，优先观察自己的输入；live 输入显式
遮蔽同号外层快照。退出准备区域（成功或
错误）恢复原 scope 和本区域引入的 immutable 标记。Display 使用 S-expression，
同时显示局部身份、域及可用性，不把 unavailable 当作已完成值。

普通 `preserved_input_identity` 的已有结构观察可读取仍在词法作用域的 immutable
输入证据；若需要真实执行 helper，nested required 捕获只复制实际引用的外层输入，
并继续走既有 domain/phase/capture 准入。读取、复制和遍历都消耗 required 预算；
失败不发布部分 snapshot。赋值、取址及捕获的既有失效入口同步遮蔽该证据，失败分支
恢复规则也保存 scope；多次失效替换当前 frame，不为每次写入增加一层 lexical 链。

普通函数的定义期默认值检查也建立独立输入区域：前序子句参数供应自己的 rigid 身份
和声明域，不把未知参数当作具体值。普通调用计划则继续区分「观察出的身份」与
「已经执行得到的输入」：前序 prepared binding 的事实临时进入已有 partial-staged
观察表，供嵌套表达式检查使用；它们不进入 required 的可执行输入 Scope，也不删除
原 producer。完成后只恢复本 packet 的事实条目，保留其他存储写入产生的失效。

这项交付只涉及已供应且可只读观察的输入。它没有把有待执行的 Type 生产者变成
已知值，没有重放副作用来取得证据，也没有接通通用前端 Region suspension。普通
待定域两个 ignored 正例仍需要原计划中的检查／执行恢复协议。

本项最终验证在 `/home/fin/luna`，使用 `development/check.py --timeout 900 -- test`：

- `-p elaboration --lib --no-fail-fast`：`/tmp/luna-tools-3_d6iitq/result.json`
  退出 0，193 项通过，包含输入遮蔽、live 阻断、预算失败、前缀扩展、失效和 dump。
- `-p luna --test required_input_scopes --test cmtt --test cmtt_methods --test prepared_input_dependencies --test comptime_clause_inputs --test clause_staged_constants --test default_calls --test normal_form_domains --test invocation_host_requests --test builder_invocations --test library_protocols --test comptime_bytecode_defaults --test trait_domain_substitution --test trait_obligations --test ordinary_clause_collectors --test ordinary_callable_types --no-fail-fast`：
  `/tmp/luna-tools-rgq9meba/result.json` 退出 0，1038 项通过，两个原有待定域正例仍 ignored。
  原 CMTT 失败已通过；七项新增真实 sysroot 用例覆盖重复实例、helper 捕获、泛型转发、
  普通函数、meta.List、错误域/后续参数/runtime 输入拒绝，以及带副作用实参恰好执行一次。
- 七个改动 Rust 文件的 scoped rustfmt 检查：
  `/tmp/luna-tools-9l1y0ao4/result.json` 退出 0；`git diff --check` 通过。

这是含作者既有资源分析修改的工作区验收，没有执行全 workspace、LLVM/native 或
性能基准；正式 docs、library 现有修改与另一开发者的 formatter 均未改写或纳入提交。

### Type 生产者的 invocation 与发布边界

普通源码的生产路径是：

```text
Checker::invoke_required_publishing
  -> PreparedRequired::invoke_publishing
     -> invoke_in_world
        -> ordinary VM invocation + source services + CodeSession lease
        -> restore_required_packet
     -> (Value, domain) + PublishedDefinitions
  -> publication::Receiver.register(definitions)
  -> ordinary (Value, domain)
```

这里的 publishing 是 builder 定义的检查与注册，不是向 Type metavariable
赋值。输入齐备的 Type 返回值与其他 comptime 值共用上述通道；`627ab804` 已删除
临时 InferenceContext、MetaId、排队、赋值和取回这一整条旁路。授权 TypeParameter
在共同恢复边界变为同身份 rigid Type 并按调用方 telescope 替换，转换计费，
不建立新的类型相等事实，不授予未授权 binder，也不展开 NF 头。

VM invocation session 保留活动帧、输入、契约及宿主请求；RequiredWorld 保留
不可变声明和可复用代码。Associated 请求与 Method 请求都携带来源世界的 resolver；
同步驱动在 VM 外准备响应，显式驱动可以保存请求后再交付。准备不等于接收，
接收也不等于选择已准入或 builder 已安装；原 ticket、世界和预算必须分别核对。
普通 required 驱动仍同步完成这些交接，不是一个可挂起的源码 Checker。

完整 Type 描述必须沿普通 packet 和 scope 恢复传送。身份消费者另持其赋值权限；
未形成的源输入需要原区域继续检查，不能另起一个已完成根的 invocation 代替。
Any/refinement 的共同数据载体、普通参数检查的 Type 响应及源码主体请求仍未接通。
`library_std_format` 只覆盖 FormatPlan 扫描，不能证明目标 println 完成。

#### 已退役队列与仍保留的赋值权限

`elaboration::type_computations::Session` 在 `627ab804` 后已无生产调用者，
`889c500c` 将其移至 `compiler/elaboration/test_support/type_computations/`。
它是依赖预检、取消、宿主交接和单输出发布的测试模型，不是新前端调度器。
模型可以使用真实 sysroot 及 VM，但不经过普通源码 Region 的待决域交接，
因此其成功不能解锁普通 callable／子句收集器两个待定 Type 正例。

模型保留一个 invocation owner，终态不重放，取消沿依赖传播；票据错投可退回，
已消费预算不退款，Live 输入准入先于 Normal 输入工厂。`type_resolution` 中
AssignmentPermit 仍管理单个 meta 的受检发布：保留输出阻止普通赋值和旧候选抢占，
不阻止无关变量推进；完成仍检查上下文、sort、occurrence 和预算。它不是语言
capability 或稳定 receipt。`nir::type_computations` 的 scope／残余／预算工具仍有
生产消费者，不随旧队列删除。

退役的源码边界、模型消费者和验证证据集中于
[模块退役第 8 节](module-boundaries-and-retirement.md#8-type-依赖队列退出生产模块)。
本节不再保留旧队列反复迁移的过程日志；历史实现和验证见 Git 中 `627ab804`、
`889c500c` 及其前序提交。下一步不得继续扩展这个测试模型来冒充源执行恢复。

输入齐备路径的既有证据：`/tmp/luna-tools-zsi3jlvj/result.json` 的 required
局部检查、`/tmp/luna-tools-a3t96ejn/result.json` 的相关九组检查均退出 0，
覆盖 opaque Type、调用方 scope、代码复用后重新核验权限、builder 及普通返回。
这是相应历史工作区的结果，不是本节整理时重跑的全量证明。

### 定义准备也能离开源 invocation 驱动

字节码的 `BuildDefinition` 原本虽已发出 preparation，却在下一驱动步同步执行
builder 回调。现已将 struct、enum、newtype 的处理拆成共同的三段：解码并检查
源描述；宿主准备未提交的 receipt；原 VM 核对描述、完成目录准入后安装。
源调用返回 `DefinitionHostRequest`，请求持有原描述、源世界、线性 ticket 和独占
预算额度；宿主可以保存请求，待其依赖就绪后再交回 prepared receipt。

暂停期间原字节码 frame 保留指令位置和目标槽，输入包已经消费，不再执行生产者。
测试模型的外层 Type 输出仍未赋值；接收响应也不立即安装，下一次 poll 才进入描述核对与
发布边界。错误类型、篡改字段／表示、无效定义及宿主虚增预算均不得发布。
误投响应原样退回；取消销毁活动 receipt，迟到响应不能命中新 invocation。
这里的取消不撤销宿主 prepare 已经发生的外部行为，也不会退款未知工作量。

测试用 Type 队列与 luna 源 invocation 入口均处理这个请求，默认同步适配器在 VM 外
调用已配置 publisher，再把 receipt 交回原执行。调用者也可自行调度请求后显式
respond。参考树执行及 IIR 内部驱动仍同步使用原 publisher，但与源调用共用
解码和接收方准入，不增加另一套描述检查。代码缓存不存请求、receipt 或完成 Type。

这条边界服务已有的具体定义 builder，不等于任意 HIR 表达式检查已可恢复，也不等于
publisher 内部的 Preparing 已拆成可恢复步骤。定义发布的统一版本根、一般源码
Region 和两个普通 callable 待定域正例仍未完成；不能把公开请求接口当成这些接线。

验证工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 900 -- check -p elaboration -p luna`：
  `/tmp/luna-tools-t76z7lzt/result.json`，退出 0。
- 最终 `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib -p luna --test invocation_host_requests --test type_invocation_sessions --test builder_invocations --no-fail-fast`：
  `/tmp/luna-tools-adhqa3r1/result.json`，退出 0，775 项通过。覆盖暂停期间不消耗
  VM 额度、接收后仍未发布、跨 owner 响应退回、取消、错误描述、虚增额度和宿主
  拒绝不执行后继。真实 sysroot 验证三类 builder 依次使用刚生成的类型、每次调用
  产生新身份，Type 队列在 receipt 安装前保持输出未赋值。
- 既有 86 项 builder 集成检查包含声明期生成、正常调用、描述拒绝、回调失败及
  具体 IIR／参考执行；没有用只拼 Rust 描述的测试代替公开库接口验收。
- 本次 16 个 Rust 文件的 `rustfmt --check`：
  `/tmp/luna-tools-549c9lek/result.json`，退出 0；`git diff --check` 通过。

未运行全 workspace 或性能基准；原有资源检查修改仍保留在工作区，以上并非纯 HEAD
验收。正式 docs 与另一位开发者的 formatter 未修改或纳入提交。

### 方法选择保留调用准备进度

trait 方法选择的宿主调用也已从字节码准备步中分离。请求产生前已经检查的接口
形成条件、公开方法身份与签名、原默认值绑定和接口程序契约保留在同一次准备中。
等待时不运行普通实参；响应后直接进入剩余选择准入，不重新调用整个准备函数。
直接方法调用与取得方法值共用选择协议；调用 continuation 另外保留原指令位置、
默认值及接口参数映射，完成后才建立原 `PendingCall`。

宿主给出的 `MethodInstance` 仍是待核验的选择结果。接收 VM 检查 member、方法
实参、实现参数、阶段和公开签名，再形成 callable 并保存已接纳的选择元数据。
错误响应不进入缓存、不执行实现体；修复响应后，新 invocation 可重新请求。
票据、源世界与预算随原 owner 交付，误投保留原响应供正确送达，取消不退款未知工作。
S-expression 可显示原方法、实际访问、已保留的契约判断及调用／值 continuation。

测试用 Type 队列与 luna 源调用均转发 `MethodHostRequest`；默认适配器在驱动外调用现有
源码 resolver。参考执行和具体 IIR 仍沿各自合法入口运行，与源路径共用选择准入。
接口条件中的类型观察、callable 具体化及调用准入仍可能同步执行；这不是任意方法
准备步骤均可恢复，也尚未补齐普通 HIR 表达式的上下文检查请求。

本阶段检查目录均为 `/home/fin/luna`，测试统一通过
`python3 -B development/check.py --timeout 900 -- test` 执行：

- `-p comptime_interpretor --lib -p luna --test method_host_requests --test invocation_host_requests --test type_invocation_sessions --test comptime_bytecode_traits --test dynamic_trait_clauses --test library_protocols --no-fail-fast`：
  `/tmp/luna-tools-zjje1rz2/result.json`，退出 101。luna 的 154 项全部通过；
  VM 584 项通过、1 项取消测试失败。旧断言把 resolver 返回等同于选择已接纳，
  新协议在两者之间允许取消，故未接纳的响应必须丢弃并在重试时重新选择。
- 取消测试增加响应待接纳状态的覆盖，验证该时刻缓存为空；在每个驱动边界取消后，
  再次调用只复用已接纳的选择。局部 `-p comptime_interpretor --lib method_preparation --no-fail-fast`
  退出 0、6 项通过，见 `/tmp/luna-tools-2u2f_17b/result.json`。
- 最终 `-p comptime_interpretor --lib --no-fail-fast` 退出 0、585 项通过，见
  `/tmp/luna-tools-9z_2ecmi/result.json`。真实源码的 4 项新增用例覆盖直接调用、方法值、
  默认值与参数顺序、错误响应后重试、取消、预算膨胀及跨 invocation 误投后正确送达。
- 修改的 14 个 Rust 文件定向 `rustfmt --check` 退出 0，见
  `/tmp/luna-tools-h11n2oev/result.json`；未格式化另一位开发者的 formatter 文件。

未运行全 workspace 或性能基准；检查仍包含工作区原有资源分析修改，不能称为纯 HEAD
验收。正式 docs、formatter 和其他原有改动均未纳入本阶段提交。

## 8. 宿主请求与候选、取消的组合

候选内允许的计算必须可隔离且可安全重放。pure 承诺用于相应纯计算，不自动使定义发布
或外部工具操作成为可回滚动作。必须先运行不纯候选操作才能判定适用性时，按正式规则
要求显式消歧或契约，不擅自试运行。

取消前有外部操作已经发生时，只能取消后续编译工作并报告已知进度；编译器内存 rollback
不撤销外部世界。实际完成状态未知则 Indeterminate，不静默再发一次。
共享的只读 query 可以有多个消费者；某个消费者取消不影响其他人。源执行产生的诊断和
副作用责任属于源执行 owner，不由“最后哪个 task 还在等待”决定。

预算 lease 预留给子工作，完成/失败返还未用额度，已消费部分不回退。未执行的分支
不创建发布世界。回收内存、注销 watcher、归还句柄等必须在预算耗尽后仍能完成，
通过预付容量/有界 owner teardown 实现，不能在 Drop 中启动新的用户代码检查。

## 9. DefinitionTransaction：准备与安装之间不能再运行用户代码

```text
Draft → Preparing → Prepared → Installed
            ↓           ↓
        Rejected      Stale
            └── repair / reprepare ──→ Draft
```

Draft 拥有生成身份、binder、代码、属性、来源和本次可见范围。Preparing 在私有 overlay
中使用普通 elaboration/coherence 准备；若需要用户计算或其他查询，都发生在此阶段，
明确归属父源执行和子请求。不可回放的已经完成步骤保存进度，不以 stale 为由从头重跑。

Prepared receipt 绑定：transaction ID、草稿版本、base world revision、可见范围、
接收者集合及版本、检查产物、所有待安装结构和预算/容量预留。
receipt 内容不可再修改；草稿改动使旧 receipt 失效。

### 安装的线性化点

目标采用一个 PublicationBundle 持有目录、已完成定义、VM 可用描述、协议索引增量和
必要来源；消费者通过同一个版本根取得相容集合。全部内存分配/检查在 prepare 完成，
install 只做版本核对及根切换，不逐个调用可失败的用户回调。

单线程 owner 已能保证唯一安装点，不要求先引入无锁算法。未来并发使用受控发布锁或
等效原子根，语义仍是一个版本；不能让查询看见新 impl 而 VM 仍缺它的声明描述。
源码批次中的 forward reference 和互递归在 overlay 内处理，不提前泄漏到其他实例。

物理实现使用不可变块和按路径复制索引；旧 root 的销毁不包含在根切换的常数工作内。
prepare 预留 retired-owner 记录，切换后单独回收，避免安装期间运行无界析构。
快照 pin、草稿回收及跨世界句柄见[世界存储与生命周期](world-storage-and-lifetimes.md)。

版本过期返回 Stale，并保留可重新准备的草稿。只有实际依赖未变且检查范围相同的
证据可复用；“仅新增声明”不证明 trait 查询、负查找或 coherence 不受影响。
编号可以跳号，已经逃出预留 owner 的身份不可复用。为减少复制的 overlay 优化不改变此契约。

### 安装不是最终实例成功

Installed 让本次作用域后续可见位置取得定义；仍需保持第 2 节的 frontier。
整体生成结束时完成剩余 coherence 及实例义务，成功才进入稳定交付边界。
失败不能链接其产物；已运行的外部行为仍按原进度报告，不能伪称全部撤销。

恢复 VM 时，旧 CodeObject 仍属于其固定语义视图；后续需要新生成定义的操作携带新视图
和显式依赖桥接。不能把所有旧 code cache 直接改标为新 world，也不能因声明编号相同
复用原调用选择。如何分割需新观察的 continuation 由 staging plan 指定，而非执行器猜测。

## 10. 三条时序推演

### 后实参帮助前 lambda

```text
call task: allocate T, U; create argument tasks a0 and a1
a0: observe T → Pending(T-producer); register and recheck
a1: establish i32 input → publish T=i32 → enqueue a0
a0: reobserve T → check body → publish U=i32
call task: settle original obligations → commit ArgumentPlan
```

源 runtime 参数还没有执行。将 a1 先出队也须得到相同调用及求值顺序；差别仅是无需等待。
若 a1 失败，则传播失败给 a0 的观察，不能留下一个没有生产者的永久 waiter。

### 生成定义请求在挂起后完成

```text
VM activation 7 reaches generation request ordinal 12
request 12: Prepared → InFlight; frame pins PC/destination
publication needs interface Q: WaitingOnInput(Q)
Q resolves; same request resumes preparation; no new ordinal
bundle installs at frontier B; response retains resulting Type and evidence
request 12: Completed → Delivered; frame receives value and advances once
next iteration at same PC uses ordinal 13, not 12
```

若响应重复到达，12 已 Delivered，不再注入。若 Q 失败，无半批定义、无 placeholder Type；
草稿可以留作修复，但这次必需 invocation 失败。

### 线性使用与整体 coherence

```text
frontier A: visible implementation set IA; use chooses evidence EA
publish B: subsequent positions see IA + IB
frontier B: a later use performs selection against the new eligible set
close enclosing instantiation: check generated definitions with static scope
```

这不是允许冲突：整体 coherence 失败，实例不能交付。也不是重跑 A：线性可见性保留
A 处已有的合法执行与证据来源；任何缓存复用必须携带对应 frontier。

## 11. 需要在实现中逐项证明的恢复性质

每个动作在“更新前、更新后通知前、响应保存后交付前”三个切点都应有明确状态。
单元层检验 owner/generation/receipt；源码层检验默认值、模板、builder、多实例及外部
请求轨迹；不能只用伪 session 构造器证明真实接线。

重点拒绝：旧 ticket 命中新 task、budget Unknown 自动重排队、candidate 的负查找跨
universe 复用、stale receipt 触发用户回调、整实例失败后仍交付机器码。
这些为后续验收要求，本轮没有运行这些测试，也未声称当前代码已经满足新协议。


### 所需类型计算的执行入口

`PreparedRequired` 的调用分两步。准备阶段的 `required_source_closure`
从已检查的根表达式、引用到的冻结数据和活动输入收集依赖。输入齐备后，
`start_prepared_invocation` 在这个世界中启动，与 CLI 共用同一个 VM 驱动。关联项、方法选择与定义
构造的响应交回原 activation。取得完成值之后，调用方恢复源作用域。

**方法声明和默认实现可能共享一个声明 ID。** 源码中保存方法值时，应保留方法契约；
实现表确实引用默认体时，才加入该定义。工作队列按“用途与 ID”去重。关联项的
初始化器可以继续引入其他关联项，即便中间没有普通函数引用也要走到闭包完成。
已访问的依赖不必在每次迭代后重新扫描。队列使用当前 required invocation 的
准备预算，库中可达声明的个数没有额外的 256 项门槛。

例如，一个已实例化的 comptime 函数可以在子句参数中携带另一个函数。无论入口
取得的是 NIR 函数实例、冻结的函数值还是 VM 中的活动值，准备端都应保留里面的
可调用声明。作为参数传递的 normal-form Type 只携带身份，发现依赖不会展开它的头。

Checker 在检查实现之前建立按身份查询的 trait 契约目录。同一个目录供方法值恢复、
实现检查和 required 源码闭包使用。方法位置在该世界内固定，关联项的默认值可在
一致性检查后的阶段完成。开放签名的验证片段仍有自己的 phase 与算术检查。

当前仍须在 VM 启动前完成根表达式和所需的 comptime 主体。涉及方法分派时，
准备端按接口保留候选实现的定义，VM 根据实际输入选择。未完成源码的按需交付、
`refines { Itself :- Display } Any` 的值传送及逐实参准入仍待接通。

真实 sysroot 的回归用例见 `compiler/luna/tests/required_input_scopes.rs`。
其中一个 Type 计算依次调用 260 个小函数。另一个把 `Forward<Flag>` 和
`Forward<Byte>` 交给同一个 `value: defer choose()`，两个调用分别得到
`bool` 和 `u8`。格式化演示尚未通过，`host-println.fl` 的双字段调用在
现有单参数库签名处报 `too many positional inputs`。

### 开放签名片段的可调用声明

检查带 `defer` 域的定义时，片段需要解析其中引用的函数。以
`measure(comptime flag: bool, value: defer Pick(flag), comptime .length: fn(Symbol) -> u64 = size)`
为例，若 `size` 绑定了一个编译器操作，检查器已为这个可调用值生成普通函数体。
片段按实际引用纳入这份定义及它的依赖。若调用的是只有签名可用的 runtime
源码函数，则纳入它的签名。保留的 trait 方法前置、后置条件也参与依赖收集。
先前这两条收集路径独立遍历，会为 `size` 的同一身份同时发布函数体和签名，
导致合法的调用在 phase 检查时报重复身份。

开放函数实例在检查主体前预留身份，递归调用由同一个键取得该身份。
主体和参数目录分别按发布与预留顺序保存，按身份查找主体。嵌套实例化失败时，
检查器只撤销本次尝试创建的预留、参数目录和主体。外层失败也会撤销已成功的
内层工作。已发出的数字身份不再分配给下次尝试。

源码回归覆盖默认生成回调、显式替换回调和两种实际参数域，见
`compiler/luna/tests/compiler_operation_bindings.rs`。片段仍需事先完成根表达式，
并复制名义类型及 trait 元数据。格式串扫描后的 Type 描述和各字段的具体准入
仍是 `std.fmt.Parse` 的独立依赖。
