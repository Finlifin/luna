# Elaboration、VM 与发布的状态协议

状态：目标协议设计，通用任务协议尚未落地；局部签名前提形成已有下述接线。
初始源码核对基线为 `873af90f`，后续源码迁移按各节明确其实际范围。
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

本次另发现局部 Type 常量的显式成员构造仍有缺口：
`const Event = EventType(); let value = Event.data(true);` 在前端报
`field projection requires a supported struct`，证据为
`/tmp/luna-tools-y3aii0eo/result.json`。带目标域的
`let value: Event = .data(true);` 用于独立检查 enum 描述交付；这不是对前一种
语言用法的拒绝规则。该成员解析缺口、quote 内待决 Type，以及一般候选探测的
生成定义所有权仍须后续处理，不能从这次修复推定完整回滚或活动源任务已经接通。

验证目录 `/home/fin/luna`：
`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -p luna --test required_definition_publication --test builder_invocations --test cmtt_storage_world --test cmtt_phase_boundaries --test test_templates --test quote_functions --test inline_pattern_observation --no-fail-fast`，
`/tmp/luna-tools-wcicoi_g/result.json` 退出 0，639 项通过；新增用例经真实 sysroot
检查 newtype 转换、enum 构造与匹配，以及已有／晚生成混合描述的身份、反射和执行。
工作区保留已有资源分析修改，未执行全 workspace 或 LLVM 原生验收。

## 4. Region 与 task 状态

Region 状态：`Open → Closing → Complete | Failed | Cancelled`。
Closing 不接受无关新请求，但允许既有任务所必需的子请求；全部必检结果或合法 residual
可导出后才 Complete。Failed/Cancelled 回收本地 waiter、候选和 VM 所有权，不发布成功实例。

任务身份为 `(RegionId, TaskSlot, Generation)`。单线程下也要 generation，避免回退后
重用 slot 时接收旧通知。一个 task 的最小状态：

| 状态 | 拥有的进度 | 允许的后续 |
| --- | --- | --- |
| Ready | 重入安全的检查点、输入描述 | Running |
| Running | 当前步骤的唯一写权 | Ready、Waiting、Resolved、Unresolved、Failed |
| Waiting | 检查点、有效观察票据、真实生产者 | Ready、Failed、Cancelled |
| Resolved | 结果及依赖证据 | 本代终态；证据失效需新代检查 |
| Unresolved | budget/unsupported/missing-premise 等原因 | 改变真实条件后显式重试，或按语言规则提取 residual |
| Failed / Cancelled | 诊断或取消来源、待回收资源 | 回收后终态 |

“空队列”不是 Complete。Closing 必须区分：等待活跃生产者、生产者失败、真实依赖环、
合法 residual、不能交付的 Unknown。循环诊断沿输入生产边，不沿 AST parent 边猜测。
一个 parent 可以保留汇合进度，同时让 child 等待并继续安排 siblings。

任务预算耗尽不立即销毁可恢复进度，也不能自行用相同预算无限入队。是否提供新额度
由调用方工具策略决定，类型规则不因为重试次数改变。

### 已接入：Type 计算恢复共用有界依赖预检

`type_computations::Session` 的单输出恢复、带 CodeSession 的恢复和依赖驱动恢复，
现在共用 `type_computations/demand.rs` 的预检。计划保留按依赖排序的计算、观察到的
输入边、外部等待叶及已有失败；它只在本次独占 context 的同步操作中使用，可按
S-expression 显示，不保存到声明快照或代码缓存。预检完成前不进入任何计算体。

旧路径在循环／外部赋值分支重新进入另一套遍历，执行前及失败传播时又各自用新的
100,000 步扫描依赖。现路径在调用方给定的同一调度额度内观察并保留依赖边，稳定
Type 发布只移除已经完成的等待项；失败沿原边传播，无关兄弟保持可独立使用。
循环证据直接终结当前 demand，调度预算耗尽则返回 `SchedulingBudget`，不把队列
改为 Failed，不消费已交付值，也不重放生产者。VM 执行额度没有因调度重试而补充。

`resume_bounded` 只观察并恢复指定输出；`resume_dependencies` 才驱动其生产者。
原 `resume` 便利入口使用整次操作共用的 100,000 步调度额度，调用方可通过显式入口
提供其他额度。预算独立于 VM invocation，不能把此局部边界称为全编译器 WorkBudget
已统一。enqueue 的检查、完整 Region 身份及前端检查响应仍按各自迁移条件推进；
这项交付不解除普通 callable 的待定域与签名缺口。

新增验证覆盖两万节点依赖链、不同预检截断位置后恢复、共享生产者只执行一次、
真实循环与预算耗尽分离，以及单输出恢复不隐式驱动生产者。工作目录
`/home/fin/luna`，检查均通过 `python3 -B development/check.py --timeout 900 -- test`
执行，命令、退出值及 `stdout.log` 的 `test result:` 锚点见以下报告：

- `-p elaboration --test type_computation_demands -p luna --test ordinary_callable_types --test ordinary_clause_collectors --test source_reflection --test builder_invocations --test library_protocols --no-fail-fast`：
  `/tmp/luna-tools-kv8164xb/result.json`，退出 0；四项新检查及真实 sysroot 消费者通过，
  两项原有 ordinary callable 语义缺口仍 ignored。
- `-p luna --test execution type_computation`：`/tmp/luna-tools-8ed0env6/result.json`，退出 0，
  九项既有 Type 计算回归通过；该报告还包含被名称过滤掉的新检查，不能据此证明新检查。
- `-p luna --test execution signature`：`/tmp/luna-tools-t55cc9tj/result.json`，退出 0，
  42 项签名相关回归通过。
- `-p luna --test execution failed_type_demands_do_not_execute_unneeded_siblings -- --exact`：
  `/tmp/luna-tools-qjpumzwj/result.json`，退出 0，失败传播及兄弟计算保留通过。

初始实现的 elaboration 库检查亦通过，证据 `/tmp/luna-tools-jpvrgxqw/result.json`；
该轮早于新增显式 `resume_bounded` 入口，不冒充最终全库复验。三个 Rust 文件格式
检查退出 0，证据 `/tmp/luna-tools-9z5q_8pg/result.json`；`git diff --check` 通过。
以上均保留作者原有的资源分析修改，未运行全 workspace 或性能基准，正式 docs 未修改。

### 同一注解的联合需求

已知注解仍可能通过输入绑定依赖多个排队的 Type 生产者。逐个调用单根恢复不构成
整个注解的预检：较早独立生产者可以先执行，随后才发现另一根的循环或已有失败。
在 `a591e54a` 的两个实现文件上运行新增回归，确实观察到这两种错误执行；证据
`/tmp/luna-tools-7er7lruz/result.json`，退出 101，两项失败于生产者已经进入函数体，
正常共享及外部等待对照通过。核验后已恢复本轮修改，未回滚其他工作区内容。

现在同一注解的全部未决输入进入一个共同需求计划，共用 active/done 集合、调度额度
和失败预检，再由与单根恢复相同的执行器运行。遍历同时保留每条边所属的需求根；
循环或发布冲突使该生产者进入失败终态，注解报告对应的 DependencyFailed，保留
原始原因和失败归属。失败沿保留边传播；预检耗尽保留等待和输入，不执行任何生产者。正常外部等待允许保存已完成
生产者，恢复不会重放；共享生产者只运行一次。后续注解仍由原有逐项准入规则控制，
不因联合预检而被提前执行。

这修复了已有签名任务 API 的准备边界，并扩展了源程序 required Type invocation
共用的底层计划器。调用点核对显示，`resume_instantiated_signature_annotation_checked`
和 `resume_runtime_signature_checked` 目前仍主要由直接 API 测试调用；普通源码的
待定域与待定签名还没有接入这些接口，不能以这里的直接测试冒充该语言用例完成。
联合需求的最终结果只报告 Ready／外部等待／失败生产者，不制造占位 Type。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib --test type_computation_demands --no-fail-fast`
在 `/tmp/luna-tools-_n_n357y/result.json` 退出 0，186 项库检查及八项需求检查通过。
新增行为包括后续循环、后续已失败子计算、共享生产者与外部等待，以及后续大依赖图
耗尽预算时阻止较早根执行、补充调度额度后继续等待而不重放生产者。

最终 `luna --test execution` 的 `signature`、`type_computation` 和
`failed_type_demands_do_not_execute_unneeded_siblings -- --exact` 三组分别为
42／9／1 项通过，报告为 `/tmp/luna-tools-dsd3s1bh/result.json`、
`/tmp/luna-tools-iqer6zlk/result.json`、`/tmp/luna-tools-om0q2nga/result.json`，均退出 0。
首次签名检查曾在 `/tmp/luna-tools-z1h2vp0q/result.json` 暴露错误归属回归；上述最终
结果已经覆盖保留失败生产者身份后的修正，未放宽原测试预期。
`-p luna --test library_protocols --test ordinary_callable_types --test source_reflection --no-fail-fast`
在 `/tmp/luna-tools-stufey3b/result.json` 退出 0，真实 sysroot 路径通过，原有待定签名
用例保持 ignored。命令统一使用前述 check.py 入口，结果锚点为各报告的 `stdout.log`
中 `test result:`。三个 Rust 文件格式检查 `/tmp/luna-tools-05y1dxrc/result.json` 退出 0，
`git diff --check` 通过。保留作者原有资源分析修改，未做全 workspace 检查或性能测量。

### 已接入：源码控制状态按边界归属

`Checker` 原先分散保存返回目标、部分返回上下文、待完成的可选／错误返回点、
推断返回位置、返回观察以及循环目标。签名、默认值、required invocation、quote
和 generator 各自枚举这些字段；部分入口未包含错误传播及推断返回位置。
现在这些活动数据归入 `program/control.rs::Context`，独立边界整体移出并恢复，
推测检查复制及恢复完整控制上下文。闭包从独立控制上下文开始检查，不继承外层返回目标。

这覆盖真实源码的签名、参数／字段／关联项默认值、required root/function、开放函数、
闭包、quoted runtime 和 generator frame；不是另建一个仅供直接测试使用的入口。
同一 computation body 仍只临时替换返回目标与部分上下文，保留当前返回证据；handler
继续使用安装处的返回与循环目标。type observation 保持原有词法循环可见性，返回
证据和捕获仍隔离。控制上下文、返回观察及部分调用上下文均有 S-expression Display，
保留待定项的种类、源位置、嵌套结构和名称转义，不把 Unknown 导出为类型。

这一步只收拢活动源码控制状态。局部绑定、捕获存储、候选事务、VM 预算和副作用的
所有权没有被吸收到这里；复制控制上下文不意味着可以回退已经执行的 invocation。
完整 Region 身份、可恢复检查位置及源码待定 callable 签名的调度接线仍未完成。

验证工作目录 `/home/fin/luna`，以下命令均经
`python3 -B development/check.py --timeout 900 -- test` 执行；完整命令与
退出值见 result.json，输出锚点为同目录 stdout.log 的 `test result:`：

- `-p elaboration --lib`：`/tmp/luna-tools-wrd0qehk/result.json`，退出 0，187 项通过。
- `-p luna --test control_boundaries --test error_surface --test default_calls --test cmtt --test cmtt_methods --test optional_purity --test handler_lexical_returns --no-fail-fast`：
  `/tmp/luna-tools-w7rr8fe3/result.json`，退出 101；926 项通过，一项已有用例失败。
  新增源码检查覆盖六种嵌套入口、外层早退／错误传播、required 边界拒绝及 handler
  的词法控制目标；保留原测试预期。
- 将本次修改的生产文件临时换回 `dff0d369`，运行
  `-p luna --test error_surface generic_comptime_error_inputs_resume_with_completed_success_domains -- --exact`：
  `/tmp/luna-tools-uagymjto/result.json` 退出 101，在相同 313..325 调用处、内部
  174..179 位置报相同 `TypeContractRejected`。这是仍待修复的泛型 comptime
  错误限定类型准入问题；该控制状态阶段未修复，后续处理见下节。对照后按摘要恢复
  本次源码，保留作者原有修改。
- `-p luna --test library_protocols --test ordinary_callable_types --test closure_environment_parameters --test spread_control --test inline_control --no-fail-fast`：
  `/tmp/luna-tools-3ag0fquk/result.json`，退出 0，319 项通过；一项原有源码待定
  签名用例仍 ignored。真实 sysroot 接口、闭包环境及循环／展开路径均参与检查。
- `-p luna --test execution callback`：`/tmp/luna-tools-f3opzmrs/result.json`，退出 0，
  30 项通过，包含部分上下文传播至显式／提前返回及候选重试。

修改的 Rust 文件格式检查 `/tmp/luna-tools-g__5tg4l/result.json` 退出 0；
`git diff --check` 通过。未运行全 workspace 或性能基准，未改动正式 docs。

### 已接入：输入事实有共同的回滚与词法边界

`Checker` 的六组输入证据现归 `input_facts::State`：普通 Type 绑定、带域值绑定、
特化输入、部分观察、派生来源和 required supplied scope。它们仍有各自的含义，
集中所有权不允许用“观察到身份”替代“值已经供应”，也不把 provenance 当成执行权。
共享的失效操作先屏蔽 supplied 快照，再按原 phase 规则删除部分观察及完成值；
派生来源保留，用于后续物化核对实际存储，而不能继续读取已经失效的值。

原 `checking_attempt` 只枚举五组事实，遗漏后来加入的 required scope；失败分支又
保存了 scope 却漏掉派生来源。现在试探失败、不提交的试探及隔离的失败分支恢复
整个 owner。预算、声明需求和已经发生的 invocation 行为不随分析快照回滚。
成功试探的 pattern-code 查询标记仍按原调用协议交付，不把它误当作局部输入事实。

块和迭代的退出采用另一套明确操作：进入时保留各类证据的词法绑定集合，退出时
删除内部绑定，保留外层快照在区域内遭到的失效。不能直接恢复完整快照，否则会
复活写入／捕获之后不再可靠的外层值。原循环边界只裁剪 partial；普通循环、展开
循环及 comptime 范围循环现在共用完整词法边界，派生来源也不再积累已离开区域的
绑定。展开迭代重复使用同一边界，外层失效跨迭代保留，不重复复制整个事实状态。

开放函数检查沿已有契约继承 Type/value 绑定及 supplied scope，清空调用方的活动
特化／部分观察／派生来源；退出后整体恢复调用方 owner。模板绑定临时覆盖、
generator 签名及 required 参数供应仍有各自的局部操作，没有据此宣称所有 Region
入口已统一。新的 S-expression dump 分开显示完整观察、partial、来源和供应 scope。

这项交付替换的是生产检查器中的分散输入状态及三种边界处理，不是可恢复源码
表达式本身。VM 活动值、推断输出许可、候选义务和源码检查进度仍有独立所有者；
两个待定 callable 正例继续保留，不能由这次回滚清理推导其已经实现。

本阶段验证（工作目录 `/home/fin/luna`，均通过
`python3 -B development/check.py --timeout 900 -- test`）：

- `-p elaboration --lib -p luna --test scoped_input_observation --test required_input_scopes --no-fail-fast`：
  `/tmp/luna-tools-6u3qftz2/result.json`，退出 0，295 项通过。新增单元检查覆盖完整
  supplied 回滚、词法退出不复活外层证据、开放函数隔离、失效预算及嵌套 dump；
  源码检查覆盖可选分支试探后的默认值与错误依赖类型拒绝。
- `-p luna --test cmtt --test cmtt_methods --test cmtt_phase_boundaries --test comptime_staged_values --test staged_aggregate_contracts --test comptime_inline_loops --test iterator_protocol --test library_protocols --test default_calls --test ordinary_callable_types --test ordinary_clause_collectors --test scoped_comptime_callables --test scoped_comptime_aggregates --test type_invocation_sessions --no-fail-fast`：
  `/tmp/luna-tools-soelh8gs/result.json`，退出 101，962 项通过、1 项失败、2 项既有
  ignored。失败为 `descriptor_contracts_do_not_infer_factory_injectivity` 对
  `unknown` 字样的断言；源程序仍被拒绝，诊断为 `not-entailed`。
- 将本次 23 个生产文件临时换回 `2c6c3beb`，单独运行
  `-p luna --test scoped_comptime_aggregates descriptor_contracts_do_not_infer_factory_injectivity -- --exact`：
  `/tmp/luna-tools-arfdceqd/result.json`，退出 101，失败位置及完整诊断相同。
  对照后按 `/tmp/luna-input-facts-baseline/restore.json` 的 SHA256 确认本次源码
  已完整恢复。该既有断言失败保留，没有修改测试预期来消除它。

26 个本次 Rust 文件的格式检查 `/tmp/luna-tools-hjspvr60/result.json` 退出 0。
未运行全 workspace 或性能基准；作者原有资源检查改动仍在工作区，以上不是纯 HEAD
验收。正式 docs、formatter 和其他已有修改未纳入本次交付。

### 已接入：探测、提交与 quote 产物保留分开

`checking_attempts` 用 `Commit / Probe / RetainCode` 代替含糊的 `commit: bool`。
普通参数和回调检查在成功后提交；分支、投影、namespace 和生成器提示只取观察结果；
quote 主体成功后保留生成的代码及构造效果，但不向外泄漏其词法绑定与控制证据。
失败尝试恢复输入事实、局部检查环境、pattern pending、quote hole 和生成器存储；
成功 Probe 也恢复这些内容。预算不因恢复而退还。

此前 `checking_attempt(false)` 只恢复局部表，却保留了生成器接收者的实际存储写入。
真实 sysroot 用例中，`wrap(if true { ~<state.build()> } else { null })` 为发现可选
类型而探测分支，随后再正式检查；`build` 向 List 追加的结果因此被重复保留。
`cmtt_storage_world` 的新用例在 `855ba17a` 生产基线上返回 `Bool(false)`，而语义
要求单次 splice 的构造效果只提交一次。另一个用例覆盖 splice 后尚未确定的 null
尾表达式，以及其他实参提供上下文后的重试。

恢复对象同时保留 `generator_data` 的检查快照和 `generator_cells` 的原始身份。
`OwnedGraphCheckpoint::capture_retained` 是内部回退操作：保存现有表示和 ownership
树，不执行用户代码、不调用 Copy/Clone、不授予新的类型／phase／transport 准入。
它按实际 Clone 复制的结构预付计费，保留 Arc 代码、闭包、列表及存储身份；图遍历
另外找到这些载荷内的 owned cells。runtime 指针的借用目标不属于回退对象。
恢复时先取得所有 cell 锁，再原子安装保存的内容与部分移动责任状态。

原 VM `checkpoint_owned_graph` 继续使用受验证的表示观察入口；两种入口共用图发现、
ownership 保存和恢复机制。特别地，无声明目录的内部回退可以保存一个既有 nominal
载荷，但这不使它通过 VM 的类型或存储准入。不能把该检查点当作冻结值、公开反射或
跨世界传输证书。

初版在每次探测前重建完整声明目录，引入了真实预算回归；已改为上述与保存表示规模
相关的内部回退。完整检查器快照、外部不纯效果事务以及可恢复 HIR 区域仍未实现；
不能将内部存储恢复描述成任意用户副作用都可回滚。普通调用的两个待定 Type 域正例
仍保持原 ignore，后续须保存源检查进度并接收实际生产者结果。

验证工作目录 `/home/fin/luna`，全部通过 `development/check.py` 执行：

- `-p luna --test cmtt_storage_world optional_branch_probes_do_not_repeat_generator_receiver_effects -- --exact`：
  `/tmp/luna-tools-kcao2180/result.json` 退出 101，在未改生产代码时得到 `Bool(false)`；
  修复后 `/tmp/luna-tools-ew2bbrft/result.json` 退出 0。
- 初版相关回归 `/tmp/luna-tools-nexypetb/result.json` 退出 101：
  `transported_runtime_methods_preserve_clause_domains_and_nested_storage` 报
  `generator storage world capture exhausted: Steps`。保留预算和预期，修复表示保存
  的职责后该用例恢复；未将此回归归咎于基线。
- `-p comptime_interpretor --lib memory:: --no-fail-fast`：
  `/tmp/luna-tools-k33n_o5p/result.json` 退出 0，27 项通过。新增检查覆盖别名、
  部分移动责任、借用目标、所有预算截断、深度限制及不产生类型准入证据。
- `-p comptime_interpretor -p elaboration -p luna --lib --test cmtt_storage_world --test cmtt --test cmtt_methods --test ordinary_clause_collectors --test ordinary_callable_types --no-fail-fast`：
  `/tmp/luna-tools-gwnms93_/result.json` 退出 0，1,679 项通过、2 项既有 ignored。
  原 VM 验证、CMTT 方法与真实 sysroot 源码路径一起检查，没有运行全 workspace。

### 已接入：声明输入准备有独立的进度所有者

`declaration_inputs::progress::Progress` 接管原准备循环中的 producer、身份、局部绑定、
准备结果及已观察的域替换输入。路由先固定执行 schedule：显式输入保留源码顺序，
省略输入随后按声明顺序。检查可因域依赖先完成后序输入，不能因此改变 schedule；
checked-prefix 仅表示源码顺序中已连续准备的前缀，不是 VM 的执行位置。所有槽位
准备完成后才发布计划，重复／越界记录拒绝，预算不足不会留下半个身份前缀。
身份复制在写入前计费，失败消耗的预算不退还。

这里保存的是准备进度，并非已经执行的实参。producer 与可选的静态身份分别保留；
未知身份继续作为 deferred 形成输入，不能凭 rigid 占位宣称域已经完成。最终计划
按声明顺序返回参数，按原 schedule 保留求值位置，不能用检查完成顺序重排。
typed 消费者仍须取得普通 NIR 表达式，身份观察不能抹掉原 producer 或授予执行资格。
S-expression dump 分别显示 schedule、checked-order、checked-prefix 和各项身份。
这里的 checked 表示准备结果已交付；旧 contextual producer 仍可能有待完成的域。

域依赖检查直接把已经检查的显式 producer 移入 Progress，不再暂存一份、在轮到源
位置后再克隆并重新观察。收集器的域依赖使用同一个入口；prepared_bindings 与局部
域只在记录成功后更新。静态身份一旦取得可以供后续检查使用，但未知身份仍保留
实际 producer，不在这个入口调用 VM。packet 展开等更早的路由产物仍需自己的临时
位置；这次改变没有声称全部源码检查都已迁成任务队列。

生产准备循环已使用该 owner，不过它仍在同步调用内存活：路由后的 HIR、检查环境、
展开输入准备以及实际 invocation 没有因此成为可挂起区域。现有 NormalFormArgument
也尚未退役；本次没有增加其节点种类。两个待定 Type 域的普通调用正例继续 ignored，
下一步须将源检查进度与原 invocation 的 Type 结果交接，不能重新执行整个输入前缀。

验证工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -p luna --test ordinary_clause_collectors --test ordinary_callable_types --test clause_packet_preparation --test clause_source_dependencies --test normal_input_plans --test open_normal_input_plans --test default_calls --test builder_invocations --no-fail-fast`：
  `/tmp/luna-tools-402xn3ie/result.json`，退出 0，511 项通过、2 项既有 ignored。
  当时的局部测试覆盖串行越序拒绝、预算失败后继续、未完成计划拒绝、身份与 producer 分离及
  dump 转义；现有真实 sysroot 用例检查默认值、收集器、输入顺序和 builder 消费者。
- 两个修改的 Rust 文件通过 scoped rustfmt 检查，
  `/tmp/luna-tools-89agc13r/result.json` 退出 0。未运行全 workspace 或性能基准。

分离检查次序后的验证仍在 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib
  program::declaration_inputs --no-fail-fast`：3 项通过，退出 0，
  `/tmp/luna-tools-c22cddy2/result.json`。局部测试现在允许后序输入先完成检查，
  检查重复／越界拒绝和预算恢复，并确认 typed 产物仍按原 schedule 执行。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test
  ordinary_clause_collectors --test ordinary_callable_types --test clause_packet_preparation
  --test clause_source_dependencies --test normal_input_plans --test open_normal_input_plans
  --test default_calls --test builder_invocations --no-fail-fast`：222 项通过、2 项既有
  ignored，退出 0，`/tmp/luna-tools-zy_crm7s/result.json`。新增真实 sysroot 用例
  在两个现有执行入口检查后序 Type 的检查不会重排副作用、保存的 callable 不重放
  实参，以及较早输入 return 会跳过后序 Type 生产者；后者包含未运行的除零操作。
  尚未运行这两个 ignored 正例，也没有以本轮结果声称恢复 Type 请求已接通。
- scoped rustfmt 退出 0，`/tmp/luna-tools-d_xboq2d/result.json`；未运行全量回归。

以上是保留作者既有资源检查修改的工作区验证；该修改、formatter、正式 docs 和
library 的既有修改均未纳入本次提交。

### 已接入：已知泛型域与未供应输入分开

`03908c64` 后追踪上述错误限定类型失败，实际拒绝条件是把 `!bool bool` 与
`![bool,char] bool` 要求为相等。根因在源码输入规划：仅因为参数域含 rigid，便绕过
普通带上下文的表达式检查。调用者已经有效的泛型参数因此被错认成 callee 尚未供应
的输入，定义时应形成的转换缺失，直到 VM 检查域相等时才失败。

`declaration_inputs` 现在对原始 callee 参数域收集声明依赖，与本次调用尚未取得
稳定身份的参数求交；只有交集非空才算 pending。参数域已就绪不要求它闭合：来自
调用者的泛型 Type／值可以继续保持抽象，并使用调用者已有的检查上下文。已取得的
身份仍不取代源表达式；实际执行位置、次数、默认值环境和输入副作用保持原有计划。

域观察返回 caller-scoped 类型及 pending 状态，并合并已观察的输入身份；调用方
不再对观察结果重复套用同一替换。依赖判定发生在替换前，避免递归调用中 callee
和 caller 复用声明 ID 时，把已供应的 caller 参数重新当成另一个待供应项。
观察状态有 S-expression Display，依赖扫描与新增值复制计入现有检查预算。

合法的错误结果、可选值及收集元素走普通表达式转换，VM 没有新增补救转换，也没有
按具体 NF／名义类型／List 定义分支。相反，抽象 T 不授权把任意 bool 当作 T 使用：
新的拒绝例证明旧实现会接受这样的未调用泛型定义，现在在定义检查阶段拒绝。
这仍不是任意生产者执行后的前端恢复协议，两个既有普通 callable 待定用例仍 ignored。

同一依赖观察现由 `input_observation::pending_domain_inputs` 提供给声明与 trait 输入。
trait 的标量、具名参数及展开包都在原始 callee 域中识别待供应项，再使用替换后的
caller 域检查值。否则 `Swapped<B, .tag = value, A>` 中 caller 的 B 会被误认成
callee 尚未供应的 B；调整具名参数位置不应改变合法性或实际执行顺序。

`normal_value_argument_at` 的符号子句参数不再自行查多张侧表并要求域直接相等，
而是复用普通 `expr` 检查及上下文转换。静态身份观察保留符号值；需要 invocation
时交付已经检查的表达式，不重新检查或重放源 AST。真实 sysroot 用例覆盖 trait
自身引用、换参、具名乱序、可选与错误结果转换，以及不相关泛型域的拒绝。
此处删除了该符号分支的平行检查规则，尚未删除整个 NormalFormArgument。

上述 trait 扩展在 `/home/fin/luna` 经同一检查入口验证：

- `-p elaboration --lib`：`/tmp/luna-tools-rplpwgyc/result.json` 退出 0，189 项通过，
  包括依赖交集及预算耗尽不得视作域就绪的回归。
- `-p luna --test trait_domain_substitution --test trait_obligations --test trait_phases --test normal_form_domains --test cmtt_type_inputs --test cmtt_methods --no-fail-fast`：
  `/tmp/luna-tools-9ofq6e01/result.json` 退出 0，270 项通过。
- 同一生产代码的输入规划、trait 动态子句及普通收集器检查见
  `/tmp/luna-tools-4uo0xbs0/result.json`，退出 0；原有待定域用例仍 ignored。

验证目录 `/home/fin/luna`，命令均经 `development/check.py --timeout 900 -- test`：

- 临时还原唯一生产文件为 `03908c64`，保留新测试，运行
  `-p luna --test normal_input_plans generic_domains`：
  `/tmp/luna-tools-5t_3l0c4/result.json` 退出 101。两个合法输入在 VM 报
  TypeContractUnresolved，非法泛型载荷却编译成功。随后按摘要恢复本轮源码。
- `-p luna --test normal_input_plans --test ordinary_clause_collectors --test error_surface --no-fail-fast`：
  `/tmp/luna-tools-timkx462/result.json` 退出 0，176 项通过，一项原有用例 ignored。
  包含原失败、名义与 NF 输入、变参、具名乱序、递归换参、错误拒绝及实际副作用顺序。
- `-p luna --test builder_invocations --test cmtt_type_inputs --test source_reflection --no-fail-fast`：
  `/tmp/luna-tools-ckhx5zl5/result.json` 退出 0，130 项通过。
- 同一生产代码下，library_protocols 27 项及 open_normal_input_plans 五项在
  `/tmp/luna-tools-p0eblqsp/result.json` 通过，ordinary_callable_types 13 项及一个
  原有 ignored 在 `/tmp/luna-tools-8nep0n44/result.json` 通过。这两次组合命令整体
  退出 101，原因是当时新 fixture 的声明／具名参数语法及诊断断言错误；修正后的
  新用例由前述最终 176 项覆盖，没有改变既有用例预期。

三个 Rust 文件格式检查 `/tmp/luna-tools-ezcbplta/result.json` 退出 0，
`git diff --check` 通过；未运行全 workspace 或性能基准。工作期间外部新增 fmt
workspace 成员；同步的 Cargo.lock 仅增该成员条目，和外部 Cargo.toml 修改一起留在
工作区，不属于本次语义修正提交。正式 docs 与作者原有源码修改保持不变。

### 设计决定：待定源区域与已检查片段共用一个 execution owner

以下是尚未实现的接口决定，核对基线为 `b613147b`。单有输入 Progress 不足以恢复
普通调用；目前 `prepare_required_root_with_live_inputs` 的 `build` 必须先返回完整
`n::Expr`，之后才能构造 required 世界并 invocation。仅供测试的 `Session::fragment`
虽能暂停子调用，却借用已建立的 `bytecode::Frame`；它没有源区域身份、待定接收域或后继代码
交付入口。`InvocationSession` 对外则从一次已获准的函数调用开始。这三个入口都不能
直接承接“正在检查一个尚无完整结果类型的调用”。

该借用式根及同步输入测试驱动现已由 cfg(test) 隔离，生产消费者仍是拥有执行帧的
invocation／plan session；具体边界见[执行测试驱动](module-boundaries-and-retirement.md#6-执行测试驱动与生产调度分离)。

因此下一条接线引入 **源 activation**：前端持有 HIR 与检查任务，VM 持有本次源区域
的实际存储、已执行前缀及控制出口。它不成为新的源语言函数，不通过合成函数包装
每个片段，也不增加 `NormalFormArgument` 的表达式分支。完整、已检查的函数仍走
已有字节码调用；源 activation 在取得一个已检查片段后也进入同一字节码执行核心。

#### 交接数据与所有权

| 对象 | 唯一所有者与内容 | 不能携带的内容 |
| --- | --- | --- |
| 源检查区域 | elaboration 会话；保留 HIR snapshot 的借用、`HirId`、词法／phase 环境、任务 continuation、待决域、候选及义务 | 克隆出的另一份 VM 值环境、全局源码地址等待表 |
| 源 activation | 原 invocation 驱动；区域的本次 activation 身份、环境存储、执行序号、分支／循环出口和未交付结果 | 整个 Checker、HIR 语法树、可由宿主任意改写的寄存器 |
| 片段许可 | 一个具体 activation、一个尚未消费的执行位置、已完成的域及前提观察 | 仅凭 `HirId` 取得的执行权、旧代许可、其它调用的完成值 |
| 检查完成片段 | 普通受检 NIR／字节码、显式 import/export 绑定和控制结果契约 | Any 占位、待域完成的表达式、活动 candidate 或 waiter |
| 域观察响应 | 来自原 activation 的结果身份、接收世界与代次、观察预算；Type 走已有结果准入与发布许可 | 一次无来源的 `Term` 赋值、可变值的永久静态身份、隐含 Copy/Clone 权限 |

表中名字是逻辑职责，尚非新增 Rust API。HIR snapshot 在编译会话中保持有效，任务
引用原节点；continuation 只保存“已完成哪些子项及下一条规则”，不重建第二份可执行
AST。完成片段可以包含已有普通控制流；只有遇到实际待决域的地方才交回源检查任务。
VM 侧请求仅含不透明区域句柄和受限观察，不依赖 `hir` 或 `elaboration` crate。
新增状态落地时同步提供 S-expression Display，分别显示 source／region／activation
身份、当前检查任务、执行位置、待定域、实际结果是否已交付及未退出的作用域；不得
只输出一个含糊的 pending 标志，也不以宿主指针地址区分两次源码执行。

现有 `start_frame` 已将环境移入帧，再把所需绑定移入寄存器；`restore_environment`
负责归还 imported 绑定。片段交接沿这套移动机制实现，不用 `Value::clone` 快照整个
环境。新增绑定的 export、未取得的值及已经移走的责任必须分别记录；片段结束不等于
词法作用域结束。外层可变绑定和捕获保留原存储身份，默认值、方法接收者和嵌套调用
都共享这条边界。真实作用域退出才按现有规则处理剩余责任。

这里不能原样调用当前 `restore_environment` 来实现任意切点：它先遍历并退出全部
`Frame::scopes`，适合现有完整片段的完成，不适合在模式／块作用域中挂起。新的交接
必须由源 activation 保留仍活跃的作用域责任，或仅在内部作用域已经闭合的切点归还
完整帧；等待本身不触发 `leave_scope`。import/export 交付、控制退出和取消各有一次性
归还记录，避免正常完成与 Drop 再次恢复同一环境。这一项属于片段协议的验收条件，
不是现有 Frame 已经提供的能力。

#### 检查前沿和执行前沿分别推进

源检查任务的步骤返回“已完成片段”“等待具体输入”“尚无实现／预算不足”或语义失败。
等待不能编码成普通 `Error::needs_context` 后从头重试；每个待决项必须保留输入来源
及 continuation。`checking_attempt` 仍用于候选探测，但不能成为已执行前缀的恢复工具。
一个步骤退出前结束对活动 Checker 的借用，所有后继所需数据由区域持有；不能把引用
Checker 的 Rust closure 塞进 VM 或冻结产物。

调度器只交付执行前沿已经获准的片段。纯粹的域检查／表示组装可以留下任务，但其中
的调用、写入、分支条件、guard、默认值和控制转移仍占有原执行位置。哪些子步骤依赖
接收域，由普通表达式检查规则产生；不能用 Integer／List／Tuple 名称或 HIR 节点
白名单赋予“可以跨过”的特权，也不能因为某函数 pure 就越过源码求值顺序执行它。

例如下面的目标用例要求 `trace == 12`，且各处写入只发生一次：

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

检查先保存列表接收域的依赖，并按普通规则准备独立于该域的前缀。原 activation
执行第一处写入；尚待 `u8` 的字面量组装不伪装成一个已经取得的列表值。到达第二个
输入时，在同一存储中执行写入及 Byte；结果通过 Type 准入后唤醒列表组装和范围检查。
两个输入都完成后才调用 count。列表中出现需上下文才能选择的调用时，必须保留那条
操作依赖；不能无条件沿用这个时序。真实依赖闭环才报循环，未实现的节点单独报缺口。
该示例是接口验收目标，当前尚未动态通过，不是新增的语言放宽规则。

`identity<Byte(), true>(9)` 的另一处暂停发生在 callee：输入 packet 已有完整 producer，
但 callable 签名还不能发布。区域保存待形成的 callee 任务；Byte 在原 activation 的
callee 位置执行，接收结果形成 `fn(u8) -> u8` 后才为 9 提供上下文。它不能先作为
`fn(rigid T) -> rigid T` 进入已完成 NIR，也不能退成任意 `comptime fn` 值。后续实参
若提供推断证据，可推进检查；这不授予它们先于 callee 执行的许可。

#### 恢复、控制出口与冻结边界

每次交接沿现有请求 Authority/Ticket 增加源 activation 的代次和执行位置校验。
同一 HIR 节点递归调用或循环再次进入时具有不同执行身份；拒绝重复、跨区域及旧代
响应时保留当前 waiter。预算不足保留已完成前缀，不重置执行额度，也不自动无限重试。
响应准入失败终止本次区域，并通过已有调用／输入帧恢复路径处理责任。

片段完成必须传回原 `Flow` 语义，不能把所有出口都折叠为 Value。`return` 离开原函数，
带标签的 break/continue 指向原循环；它们终结或跳转相应前沿，未到达的后继生产者
不运行。取消同样不执行后继源码、默认值或契约；已发生的不纯效果不承诺回滚。内部
存储检查点只用于尚未提交的检查探测，不抵消真实 invocation 的写入或外部效果。

声明世界仍由不可变快照及发布协议提供。活动区域不能冻结进 RequiredWorld、代码
缓存或 Type 结果缓存；新增片段必须带可见世界与依赖证据。后继片段若需要新发布
定义，显式申请对应可见世界，不能替换当前 CodeSession 的包指针并沿用旧准入结果。
首次交付可以限制为现有世界中的代码片段，但必须明确诊断跨世界缺口，不悄悄忽略。

#### 第一条接线必须闭合的范围

先在同一纵向提交中接通源检查 continuation、源 activation 的片段交接，以及普通
callable 的两个等待点。只增加 public request enum 或可手工构造的 fragment session
不能算这条路径完成。完整表达式继续沿已有入口；待定表达式由共同规则逐步产出片段，
最终不再要求 VM 执行一份扩张后的专用参数 AST。NF／名义类型随后改为同一消费者，
在它们迁移前不能删除仍在用的旧参数计划。

验收同时检查：上述源码顺序、Type 生产者次数、分支只执行所选路径、提前 return
阻断后继、跨片段可变接收者身份、默认值与空展开位置、域错误拒绝、失败／取消后旧
响应拒绝，以及同一源码的递归／循环 activation 不串值。Type 结果保留不透明 NF 头；
错误元素不能借机改变为“延迟接受”。普通定义和未选分支仍执行语言要求的检查；已知
错误不能因为本次没有执行而消失，合法 residual 与尚无实现的检查分开报告。

当前动态证据：在 `/home/fin/luna` 运行
`python3 -B development/check.py --timeout 900 -- test -p luna --test ordinary_callable_types --test ordinary_clause_collectors --no-fail-fast -- --ignored`，
`/tmp/luna-tools-qx183hcc/result.json` 退出 101。两个原有正例分别报
`comptime callable signature still depends on an invocation-time clause input` 和
`list literal requires an element type context`。没有改变其预期或解除 ignore。
本次是接口与迁移设计收敛，不声称实现了源 activation。

#### 实施细化：普通检查规则必须交付 continuation

在 `363d477a` 核对上述入口，尚有四处不能靠增加一个 Pending 变体解决的依赖：

| 当前路径 | 同步假设 | 迁移时必须交出的状态 |
| --- | --- | --- |
| `program.rs::expr_context`／`delivery.rs` | 目标域适配已保存显式后继；一般子表达式检查仍是同步调用 | 将现有后继接到区域的子结果槽，补齐 expected／partial context 的待定观察；恢复不能重新选择一次转换 |
| `block_regions.rs::check_block_region` | 语句进度、受检前缀与词法退出责任已有独立状态；子检查和尾值仍同步 | 将活动词法状态与尾值检查一并交给可恢复区域；等待不等于退出词法域 |
| `declaration_inputs.rs::Progress` | 原串行 cursor；后续已分离 schedule／checked-order 并接入前置域检查 | 仍须补齐可恢复检查任务及每项域依赖；已有结果位置和源码顺序不能被执行调度覆盖 |
| `required.rs::prepare_required_root_with_live_inputs` | `build` 返回完整根后才收集自由绑定和建立世界 | 独立于完整根的源输入描述及受限可见世界；后续每段所需绑定增量检查，不能先捕获所有外围值 |

采用显式规则 continuation，不再让 Rust 调用栈兼任待定区域的唯一所有者。它保存
检查算法的局部进度，而非再保存一套有自己求值语义的表达式树：源语法仍由 HIR
snapshot 持有，已完成子结果由区域的受检结果表持有，continuation 引用二者。
字面量、调用、块、分支等规则必须共享现有检查责任，不把 NF 参数旧分派复制一份。

一条规则的推进协议如下，名称仅描述实现职责：

```text
CheckTask(region, hir-node, lexical-scope, expected-observation, rule-state)
  -> NeedChild(child-task, result-slot, continuation)
   | NeedObservation(key, generation, continuation)
   | Checked(result-slot, ordinary-code, domain-evidence)
   | Failed(diagnostic)

CheckRegion
  tasks + ready-queue + child-results + observations + lexical-scopes
  invocation-owner + source-execution-dependencies
```

`Checked` 不等于“表达式已执行”。它既可能是可编译的完整子树，也可能是一个父规则
所需的类型／形成检查结果。域尚未成立的部分仍是 CheckTask；不能在 Expr.ty 中放
一个未获准 rigid 或 Any，再让通用 NIR 消费者以为整个表达式已完成。

同步兼容入口应驱动**同一规则实现**直至完成：只在不需要外部输入的子树返回现有
`Result<Expr, Error>`。不能同时保留完整递归检查器，又新增一个“支持少数输入形状”
的区域检查器。迁移中尚未改造的规则可以作为不可挂起的叶调用；它若遇到真实待决项，
必须返回明确的迁移缺口，不能退回树解释器、反复 probe 或解释错误字符串。

领域检查和执行依赖使用不同的边：`NeedDomain`、`NeedValue`、`NeedCheckedCode`
描述所需信息；`AfterExecution` 与 `SelectedBy` 描述次序及控制许可。调度器可以推进
所有检查依赖已满足的任务，VM 只能执行控制许可和前序效果已满足的代码。没有就绪
任务时先检查未收回的宿主请求、预算和未迁移规则；只有完整依赖中的闭环才报循环。
当前 `active_required` 的 32 层保护不能直接复用为区域依赖循环证明。

#### 已接入：目标域适配保存子检查后继

`delivery.rs` 替换原 `expr_context_adapted`，由同一组规则区分已检查结果、子域检查
和暂未迁移的一般表达式检查。可选包装与错误结果交付保存为显式 Delivery；同步
驱动器检查子域后，逆序消费这些后继。后继消费受检结果，不执行源码，不重新发起
一次调用来取得值。提前离开控制区域的 NoReturn 结果直接传递，不附加包装。

既有上下文规则继续生效：分支、null、上下文枚举构造等仍接收完整目标域；已有
optional 保留自身存在性层次，已有错误结果走 Embed，普通值走 Success。整域探测
和无上下文探测沿用原有策略，本次不将探测改造成执行许可或新的失败分类协议。

每次规则推进消耗 required 步数；需要保留错误结果的子域时，先通过共享复制预算
检查，再保存拥有的 Term。外层目标尽量借用或移动，后继不复制 Checker。借用仅在
当前同步调用内有效，不能直接冻结进 RequiredWorld 或持久缓存。Delivery 和推进
结果均提供 S-expression Display，保留目标域、源码位置及字符串转义。

这是普通检查器中真实目标适配规则的迁移，仍没有 CheckRegion、词法区域挂起或
外部 Type 响应。一般检查和候选探测仍有同步递归；两个普通调用的待定域正例仍未
接通，不能用这次局部迁移声称源 activation 已完成。下一步必须处理块的作用域
所有权及子结果交接，使后继真正可以跨域等待存活。

新增 `contextual_delivery` 使用真实 sysroot，分别在字节码与树解释路径检查三层
optional、错误结果、null、提前 return、分支及可变接收者仅求值一次；拒绝越界
整数、错误 payload 域和成功 payload 域不匹配。工作目录 `/home/fin/luna`，命令
`python3 -B development/check.py --timeout 900 -- test -p luna --test contextual_delivery --no-fail-fast`，
两项通过，`/tmp/luna-tools-j0lgm2ce/result.json` 退出 0。

受影响回归命令为
`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib --test invocation_budget -p luna --test native_optional --test error_surface --test error_result_flow --test contextual_spreads --test ordinary_callable_types --test ordinary_clause_collectors --test cmtt_methods --no-fail-fast`。
`/tmp/luna-tools-7osoe6b4/result.json` 退出 101：671 项通过、2 项原有 ignored，
`error_value_trait_indices_reconstruct_both_branches_in_selected_bodies` 一项失败，
诊断为 `177..180: required comptime evaluation failed: interpretation Type at
(generated-source 0)`。其中 elaboration 的 211 项包含新增后继 dump 检查。

为区分本次回归与基线问题，暂时将唯一接线文件 program.rs 恢复为 `a1aab74a` 的
内容，使新增 delivery 模块不参与构建；其他工作区改动保持原样。运行
`python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface -- --exact error_value_trait_indices_reconstruct_both_branches_in_selected_bodies`，
`/tmp/luna-tools-z6jzssh3/result.json` 同样退出 101，诊断完全相同。随后恢复本次
接线。该基线失败后续已定位并修复：VM 的恒等 Embed 不应要求具体标签，见
[抽象错误结果交付](values-staging-and-publication.md#错误结果的恒等交付不观察抽象标签)。
没有修改其预期或加入 ignore。这里的验证包含既有工作区修改，
不宣称为纯 HEAD 验收。

#### 已接入：块的语句进度与词法退出责任

普通块和待定输入块共用的 `block_regions::Progress` 现在拥有源码块引用、受检语句
前缀、当前子检查状态、diverges 和分期模式证据。同步驱动器请求下一条语句，再把
检查结果交还；一个子检查尚未返回时不能再次请求，尾值状态不能重新开始语句遍历。
完成语句后才发布相应后继类型事实。发生控制退出后仍检查后续源码，但不为它加入
可达路径的后继事实；执行次数仍由原 NIR 控制流决定。

`LexicalScope` 独占外层局部域、expr/pattern code 引用、PTC context 和输入事实边界。
成功或普通诊断退出都恢复局部声明及类型前提，保留块内写入造成的外层值失效。
它不是候选事务回滚，不复制整个 input_facts 来复活已失效值，也不拥有 VM 存储。
复制局部域前使用共享 Term 复制预算，表项和边界集合先计费；共享 code/context 只
保留引用。每条语句推进也扣除 required 步数。复制失败只消耗预算，不安装半份作用域。
这限制了既有快照路径的成本，尚未消除随可见局部数量增长的复制开销。

词法父环境、语句进度、输入边界和分期模式证据提供 S-expression dump。这里的 HIR
引用仍借用同一次检查的源码包；不能发布到稳定世界。一般子检查、尾值检查和作用域
装卸仍由同步入口驱动，尚不支持交错调度、取消外部请求或跨 Type 等待恢复。后续必须
连同当前活动词法状态、tail continuation 及源 activation 迁移，不能将保存外层环境
误当成保存了完整 Checker，也不能在挂起时执行 leave。

验证在 `/home/fin/luna`，命令前缀为
`python3 -B development/check.py --timeout 900 -- test`：

- `-p elaboration --lib --test invocation_budget -p luna --test contextual_input_regions
  --test contextual_delivery --test required_input_scopes --no-fail-fast`：315 项通过，
  `/tmp/luna-tools-r7d6b9xs/result.json` 退出 0，包含真实 sysroot 的新源码用例。
- 补充 dump 单元测试时两处测试类型名写错，首次编译退出 101，
  `/tmp/luna-tools-p7ry40yw/result.json`；修正后执行 `-p elaboration --lib -p luna
  --test inline_pattern_observation --test ordinary_callable_types --test ordinary_clause_collectors
  --test cmtt_methods --test contextual_spreads --no-fail-fast`：537 项通过、2 项既有
  ignored、2 项旧诊断匹配失败，`/tmp/luna-tools-c9j2d0px/result.json` 退出 101。
- 旧诊断已在 `a00e221f` 删除 clause_inputs 独立实现时退役。扩大扫描范围仍失败
  （`/tmp/luna-tools-o_kut3ni/result.json`，退出 101），因此撤回范围修改。两项测试
  改为核对现行 `input_observation` 的 observation-limit 和 declaration Progress
  的身份捕获预算错误，源码、原有额度范围、失败后新尝试的执行断言都保留。
  `-p luna --test inline_pattern_observation --no-fail-fast`：16 项通过，
  `/tmp/luna-tools-7r44av9l/result.json` 退出 0。新增作用域复制及语句推进预算用例
  也在此组，确认资源失败没有变成合法输入或上下文缺失。
- 最后将新增拒绝例的 bool 结果函数名按约定改为小写；单独重跑
  `-p luna --test contextual_input_regions -- --exact block_exit_does_not_skip_checks_or_export_a_private_binding`，
  `/tmp/luna-tools-5tlaf2be/result.json` 退出 0。修改的 Rust 文件经 scoped rustfmt
  检查，`/tmp/luna-tools-2tebstto/result.json` 退出 0；后续测试文件修改也已格式化。

上述分组有重叠，不累加为独立覆盖；两个普通 callable 待定域正例仍未完成。检查
使用保留作者已有修改的工作区，未执行全 workspace 或性能测量。

#### 已接入：required 世界的依赖发现共用结构遍历

`required/dependencies.rs` 现在统一发现代码中的函数、trait 方法和关联值依赖，
替换 required.rs 的独立表达式／模式工作栈。required 根、生成函数、关联初始化器、
普通延迟区域以及 live 闭包均已迁到同一入口。结构递归交给 `nir::visit`，并消耗
调用者的 required 步数；代码中的模式计算、闭包体、默认值、前后条件与 contextual
参数占位节点都参与计量。一次完整扫描成功后才合并成员／关联值集合，失败消耗的
预算不退还，也不返回半份可供世界发布的依赖集合。

依赖发现与可执行性检查仍分开。retained_expressions_checked 允许尚未实例化的
CodeEvidence 模式作为不透明叶保留：该节点本身没有待遍历的表达式子节点。它不赋予
执行许可；普通 NIR 遍历仍拒绝未解决的 evidence，最终程序和域检查仍照常执行。
同样，扫描不会运行 Type 生产者，不把 opaque NF 头作为函数体展开，不选择任意
concrete impl 来补齐未知的动态成员。冻结值和活动对象图仍走各自的身份／依赖发现
协议，活动对象图不因为代码遍历统一就能进入持久缓存。

同一 required 根的自由 LocalId 现在在 initializer substitution 后观察一次，供
生成器输入、已供应输入、Type 槽运输和世界依赖选择共同读取。随后的 Type-slot
rebind 只将 Bound 替换为当前 invocation 的 rigid 身份，不引入或消除词法 LocalId；
因此这份结果仅在这次结构稳定的根内复用。不是跨根缓存，也没有把所有外围值都捕获。
不同根、实际改写结构或新增后继片段仍须各自发现依赖并通过域／阶段准入。

这一步清理了现有完整根入口的依赖准备，但尚未让 build 闭包提前交出未完成根，
也没有建立源区域的增量世界和冻结发布协议。共享 visitor 按结构节点扣费，不能据此
宣称任意输入的所有临时分配、宿主栈和整个依赖闭包成本已经得到统一上界。
后续已为表达式读／改写 visitor 的工作栈增加入栈前限额，并计量过滤扫描的结构项；
具体责任及未覆盖的复制成本见
[共享遍历的准备成本](solvers-resources-and-lowering.md#42-共享遍历的准备成本)。

验证工作目录 `/home/fin/luna`：

- 初次局部编译因新增测试误写 `nir::InterfaceInstance` 路径退出 101，
  `/tmp/luna-tools-axflm07s/result.json`；改为实际的 `nir::program` 类型后继续检查。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib
  program::required --no-fail-fast`：35 项通过，退出 0，
  `/tmp/luna-tools-nhep8l7d/result.json`。新增检查覆盖所有函数根、闭包和模式计算，
  预算截断不发布部分集合、重试，以及 contextual 占位节点的计量；普通遍历仍拒绝
  unresolved evidence，没有将发现依赖的宽松模式用于执行检查。
- `python3 -B development/check.py --timeout 900 -- test -p nir -p elaboration
  -p luna --lib --test required_input_scopes --test open_normal_input_plans --test cmtt
  --test cmtt_storage_world --test cmtt_methods --test staged_invocation_reuse
  --test deferred_associated_values --test ordinary_callable_types --test ordinary_clause_collectors
  --no-fail-fast`：1,177 项通过、2 项既有 ignored，退出 0，
  `/tmp/luna-tools-zrknan4h/result.json`，包含前述局部检查。
- scoped rustfmt 退出 0，最终修改的文件见 `/tmp/luna-tools-m_sar90_/result.json`；
  NIR visitor 与普通延迟区域此前已通过 `/tmp/luna-tools-qdjtsaca/result.json`。

未运行全 workspace、性能对比或 ignored 正例；仍是带既有资源分析修改的工作区
验证。正式 docs、formatter 及其他已有改动保持原样。

#### 实施细化：词法状态与值存储不共用快照

现有 `Checker` 同时持有声明服务、推断状态和词法字段。区域暂停时，仅保存
`HirId` 与 expected 不够；继续运行别的任务会改变它稍后读到的环境。迁移按以下
职责拆分，不先把整个 Checker clone 成一个 Region：

- 声明身份、HIR snapshot、稳定签名与已提交实现目录由会话服务持有，按世界版本观察。
  `active_signatures`／`active_required` 等活动检查登记属于任务，不随快照发布。
- 局部类型、immutable、input facts、alias substitution、callee-delivery context、
  control 和 capture 边界归词法 scope／当前规则。一个任务每次借用检查服务时安装
  自己的 scope，退出推进步骤时归还；等待保留 scope，真正离开块才裁剪局部事实。
- PTC Context 和候选事务归相应推断区域；记录 observation 的前提和版本。程序状态、
  内存与资源规格仍由程序验证层负责，不因为调度依赖统一就塞进 PTC。
- 可变 generator cell 与 VM slot 的实际内容归 execution owner。检查结果只保存
  受限的域／身份观察；不能把 `generator_data`、`generator_cells` 与另一份 VM
  environment 一起克隆，然后靠恢复快照“合并”真实执行。

首批拆分要覆盖 required 入口当前手工保存的 control、captures、phase、numeric
defaults、pending type uses、locals、default bindings、supplied inputs 和 immutable。
每组状态都有显式安装／归还责任；取消与错误沿同一出口归还。代码缓存、声明目录
或其它区域取得的结果不能把这个借用时的环境隐式当成自己的环境。

`checking_attempts::Probe` 仍只能回滚未提交的内部探测。一次源 activation 已执行
的写入不受该快照保护，后续类型检查失败也不许重放或假称外部效果被撤销。候选未
确定时，不执行候选专属的用户计算来“试试看”；把所需计算登记为依赖，并按普通
候选选择契约判定何时获得执行许可。

#### 实施细化：片段边界必须同时带绑定和控制接口

`bytecode::Frame::start_frame` 将入口环境中存在的 LocalId 移入对应寄存器，
`restore_environment` 归还这些 imported 槽。此前 `InputFinish::Preparation` 已有
特殊导出路径：成功时把指定 exports 追加到 imported，再恢复环境。因此不能笼统
称现有 VM 不会导出新绑定；真正缺少的是源区域对任意后继片段的绑定／作用域契约。
这条已有生产路径现已迁入帧自身的交接接口，详见下节。Program 的寄存器编号仍是
每段局部编号，不能直接把旧 slots 或 PatternScope 搬到另一个 Program。

采用显式绑定交接表：每段声明需要的 LocalId、输入域、访问／移动方式、输出绑定、
所属源作用域和退出后责任；寄存器编号仅由该段自己的编译结果解释。新建绑定只有
在它实际初始化完成后才交付，尚无值、被移走、仍活跃的值引用分别保留状态。对重复
输出、错域、错 scope、过期执行位置的交接，先验证再移动，失败不能只归还一半。

切点先限定为**该段内部模式作用域已闭合**的边界；源块的外层 scope 由 activation
持有，片段完成不退出它。内部模式若在 guard／子调用中暂停，原 Frame 继续存活，
不调用 `restore_environment`。只有达到已声明的交接点，才一次性取回 imported 和
exported 绑定；真正的源 scope 退出再处理局部责任。这样不必把私有寄存器作用域
解释成跨 Program 的公共词法作用域，也没有免除未来更细切点的正确性要求。

控制出口同样要在编译片段前声明。当前 `comptime_compiler::compile_root` 的
break／continue 只接受本次编译遍历中登记的 loop，外部目标报 InvalidControl；
return 则经 `Op::Finish { explicit: true }` 保留为 `Flow::Return`。因此不能只把
原函数的一段抽出来交给现有 compile，就声称它仍能跳回外层循环。

目标接口区分本段内部跳转与指向源 activation 的外部出口。内部循环仍降为 Jump；
合法的外部 break／continue 携带经过检查的源控制目标和需退出的源 scope，由原
activation 接收。未经声明的目标仍拒绝。return 也由原函数 owner 接收，不能由
片段的便利包装吞成普通 Value。最初未实现的出口必须限制在片段准入处，而非执行
到一半才静默回退。代数效应 resume 的 continuation 也不能仅凭一个 loop id 接收。

#### 已接入：帧持有显式绑定导出

`bytecode/bindings.rs` 接管已有输入准备片段的导出逻辑；没有建立另一个执行器。
请求保存 LocalId 与来源，创建时拒绝重复或与调用者已有绑定冲突；开始执行前再次
检查实际环境，并将名字解析到这次 Program 的寄存器。全部检查及帧分配预算通过后
才移动环境，失败时调用者仍持有原环境。请求不授予类型或所有权准入，代码的域检查
仍由现有前端和 VM 负责。

解析结果由实际执行的 Frame 持有，不再放在可脱离代码的 InputFinish 中。完成时：

- 正常值出口要求 Frame 已终结、没有待响应请求且内部模式作用域已闭合。
- 所有导出槽都必须有值，环境中也不能出现同名新绑定；先整体验证，再移动任何输出。
- 输入准备完成后的新绑定直接移入环境，原输入继续按 imported 归还；导出不伪装
  成导入，也不复制／冻结存储或执行源语言的 Copy、Clone。
- 错误、return 等非正常值出口不发布新绑定，但归还实际已变更的原输入；没有回滚
  真实副作用。取消路径仍保留原有 Frame 和 session 的清理责任。

requested-exports、bound-exports 和帧 dump 均为 S-expression，后者展示本段
LocalId 到寄存器的映射。请求是 invocation 内部数据，不是缓存 key、持久 IR 或
跨 activation 的许可；后续源区域仍须交付 scope、域及执行 receipt。这里也没有
允许跨未闭合模式作用域切段，或把 return／外部循环控制转成普通 Value。

验证工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
  --lib bytecode::input_preparation_tests --no-fail-fast`：7 项通过，退出 0，
  `/tmp/luna-tools-m6w3d2e0/result.json`。包括所有预算截断、提前返回、生产者错误、
  名字冲突、缺失输出、未闭合作用域、未终结帧、环境冲突及存储身份保持。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
  --lib -p luna --test clause_packet_preparation --test ordinary_clause_collectors
  --test ordinary_callable_types --test comptime_receivers --test comptime_storage_domains
  --no-fail-fast`：755 项通过、2 项既有 ignored，退出 0，
  `/tmp/luna-tools-r52uh6us/result.json`。覆盖 VM 的暂停／恢复／取消、调用输入计划
  及真实 sysroot 的参数展开、收集器和两阶段存储消费者；包含首轮 7 项局部检查。
- scoped rustfmt 退出 0，`/tmp/luna-tools-ljbba8vg/result.json`。

这些是保留既有 moves/effects.rs 修改的工作区结果；未运行全 workspace、性能或
源区域 Type 等待的 ignored 用例，没有修改它们的语义预期。

#### 实施补充：声明接口与可执行主体不能共用完成状态

当前调用路径还有一个早于 Type 响应的发布边界。普通函数收集与 required 依赖准备
共用 `function_bodies.rs::check_source_function`，经 function_body 取得完整 Expr
后才构造 `nir::program::Function`；Function 的 body 字段没有待决形式。
VM 的 `invocation.rs::prepare_invocation` 则从 functions
表直接取得该 body，随后准备环境、契约和字节码。`RequiredWorld` 又只持有已经
形成的 NIR 声明快照。因此“等 Type 返回后继续参数检查”不能仅增加在现有 VM
session 的请求枚举上：尚未完成的源主体根本到不了这个 session。

共同入口已经合并参数绑定、普通 Type 输入的 bound 身份、前后置条件与入口快照、
模板／效应感知的主体检查，以及完成 Function 的元数据。required 不再直接调用 expr
另建一份主体。两种产物仍有必要区别：最终声明携带已检查的默认值描述，required
依赖由实际 call plan 携带使用到的默认值，不在依赖收集时强制准备未使用默认值。
外围声明作用域仍由调用者建立；这个同步规则尚未交付可挂起任务或新的执行许可。

VM 侧按普通调用输入进行的主体重绑定已移到同一 activation 的 Entry → Body 转换。
进入调用时完成签名／参数准入、类型契约、结果域及入口快照，保留原受检 NIR 主体；
入口程序契约通过后才调用 prepare_input_body 或 prepare_owned_input_body，随后
发布该 activation 的可执行主体及代码缓存身份。拒绝入口或在入口等待时取消，不做
这次主体准备；准备失败仍沿原 activation 退出，不能再次 begin_invocation。
这不是延迟定义端检查，也不让未受检源码进入 VM。声明参数实例化及其他准入工作
仍可能早于入口程序契约；本次移动的是普通调用输入的主体准备，不声称所有主体成本
均已移后。函数体的源码服务取得和分段交付仍未实现。

这里必须区分三个完成点，而不是将未完成主体装成占位 Function：

| 产物／活动状态 | 可对消费者承诺 | 尚不能承诺 |
| --- | --- | --- |
| 声明接口与稳定源码模板 | 身份、参数望远镜、阶段、已形成的接口及明确保留的检查义务；模板绑定原源码快照 | 已有可执行 body，或所有调用点的依赖签名已完成 |
| 源主体检查任务 | 当前 scope、检查后继、取得的域观察、当前 invocation 的执行许可及结果位置 | 可进入稳定 NIR、CodeSession 或供另一 invocation 直接复用 |
| 完成的 Function／代码片段 | 已满足相应阶段与安全检查的 NIR、绑定接口、控制接口及所需声明世界 | 获得运行未选分支、重放前缀或改变源定义身份的许可 |

稳定模板属于源码服务；使用声明身份与 owning source snapshot 查找，不能把当前
Checker、临时 HIR 引用或 VM cell 作为 Function.body 的新分支冻结。模板中的源码
描述与接口证据可以长期保留，活动规则与已执行值不能。进入具体调用时创建新的任务
和 activation；若需要依赖域，只向该 activation 的源任务交付观察，完成后再产生
真正的受检片段。NIR Function 继续表示受检产物，不以 Any／rigid 假类型替代缺失主体。

检查模式与执行许可也须分开。现行正式语义规定 comptime 函数仍有定义检查与具体
调用，comptime 环境采用 loose；normalization 可以完成当时要求完成且输入具备的
计算。由此不能推导“看到 comptime 标记就立刻执行”，也不能禁止 normalization 中
所有必需计算。源任务必须带明示的检查／执行原因：只描述定义、当前必需计算、或
已经进入的调用。许可来自相应规则、形成义务及控制前序，不能由缓存命中或一个
函数的 Type 结果签名推导。参见正式
[normalization 与 instantiation](../docs/comptime/normalization-and-instantiation.md)。

实施时同时更改两端：前端的函数收集与 required 根准备允许返回稳定模板或活动任务；
VM 的声明准入读取接口，主体取得另走源码服务。在执行获准后保留同一个 activation，
等待受检代码到达，不能重新 begin_invocation，也不能为每段代码合成函数。方法、
默认值与契约仍走原身份和域检查，已完成的 NIR 函数保持现有快路径。源码服务无法
提供某种模板时明确返回未迁移边界；不能转而直接解释 HIR，或将所有函数延迟到第一次
调用而免除定义端检查。只有语义允许延迟的义务可以进入稳定模板。

新增验收必须包括“尚未调用的 comptime 定义”与“已经进入 required 根”的区别：前者
不因检查整个包就执行普通函数体副作用；可即时判定的非法操作仍被拒绝。后者在前序
效果与形成义务允许后执行生产者一次，并恢复原 callee／实参规则。相同源码进入两个
invocation 时建立独立状态；只共享稳定模板、接口与合法代码，不能共享执行 receipt。
这补充了前述源 activation 设计，并未实现新的模板发布或主体请求 API。

#### 实施细化：两个正例的逐步数据流（执行交接）

`count<.values = [1, 255], Byte()>()` 的检查任务先形成两个参数位置及来源，登记
values 的域依赖 T，同时检查 Byte 调用及其 Type 结果契约。这个阶段不调用 Byte。
列表字面量的未定组装仍属于源检查任务，不先生成一个带假元素域的 Value。
到达执行许可位置后，Byte 在原 activation 中执行；其真实 Type 经准入交回 T 的
观察位置，再恢复字面量范围检查、集合组装、参数形成与 count 的调用准备。

源码顺序示例中的 `trace` 写入必须先成为独立受检片段，完成写入后才允许调度下一
执行位置。若较早输入还有读取、用户调用、失败检查、所有权交付或域相关转换，
它们不能被归类为“只是尚未组装”而跳过。例如先读取一个可变字段再等待 T，必须
保存读取时依法取得的值，不能在后序参数写入后重新读取；没有合法读取／保留规则
时继续等待。不能把源码表达式的 pure 标记当作取消这条顺序边的依据。

`identity<Byte(), true>(9)` 在 callee 检查位置等待实例签名。Byte 的完成值只提交
给这次调用的 T，原 pending packet 的 producer 随执行 receipt 标为已完成，不再
由 `ComptimeFunctionInstance.subject` 执行第二遍。完成参数域和形成检查后发布
本次 callable，再用 u8 检查 9。若 Type 结果错误或返回的域不接受 9，调用终止；
不先创建可调用实例再期待 VM 后补签名。声明主体的 NF 身份规则保持不变。

#### 实施细化：退役条件和验收定位

| 验收情形 | 必须保留的证据 | 失败时先查的边界 |
| --- | --- | --- |
| 两个现有 ignored 正例 | 实际 Type 生产者一次执行；输入范围／签名在其后完成 | 检查 continuation → Type 准入 → 原输入位置 |
| 前后输入都修改或读取同一值 | 源顺序、读取时的值、相同可变存储 owner | 执行依赖与绑定交接，不能靠重新观察补结果 |
| 较早输入提前 return／失败 | 后继 Type 生产者、默认值和主体均未执行 | Flow 外部出口与待执行任务取消 |
| 前缀建立局部，后缀读取或通过 self 更新 | 新绑定 export、原 cell 身份与词法存活范围 | Frame → activation → 下一 Frame 的移动交接 |
| 同一源码递归／循环进入两次 | 不同 activation 与执行序号；旧响应不消费新等待者 | 动态身份，不能仅用 HirId／PC |
| 模式 guard 中挂起，随后失败 | 原模式作用域仍在，实际失败路径清理一次 | Frame 保留与作用域出口，不是普通片段完成 |
| 未选分支含可静态判定的错误 | 该分支仍按语言要求检查，副作用未执行 | 检查与执行分离，不能以本次未运行免检 |

第一批替换 `expr_context`、块规则、声明输入准备、callable packet、required 根和
VM 片段入口的共同路径。旧同步入口作为同一规则驱动的适配器保留；普通 callable
正例与这些边界验收通过才解除 ignore。随后 NF／名义输入迁入同一规则和交接协议，
退役 `InputChecking` 分流及 NormalFormArgument 中的执行树。活动 continuation 不
进入稳定 NIR；需要延迟到实例化的源码规则必须按冻结协议保存稳定描述和契约，并在
实例化创建新任务。未迁移路径保留明确边界，不能为了删除旧枚举而丢掉其真实消费者。

本小节是基于当前生产入口的实施设计，不是新增的可用 API；未新增占位状态机或
改写失败正例。源码与库实现状态未因这些设计细化改变。

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

此前 Type 计算队列保存了输入等待者，但就绪后的 `resume_prepared` 一次性调用 VM，
不能将主体或普通函数契约中的关联选择交回队列外的驱动者。现在 `Session::start`
沿用有界依赖预检，再返回独占的 `Invocation`；等待输入或已有终态直接返回 `Settled`。
活动句柄拥有该输出的 Pending、失败集合和单输出发布许可，不再借用整个推断
上下文，也不把执行现场序列化进 Type、残余调用或 code cache。其 Display 显示
输出、许可的上下文身份及内部执行阶段。

`poll` 区分 Running、Waiting、Associated 和 Settled。Associated 请求使用已有
VM session ticket、源世界和预算所有权；等待期间再次 poll 不重发请求、不执行前缀。
宿主响应经原 VM 接收检查恢复同一个 activation。仅当 VM 完成、返回值确为 Type
或合法 Type binder，并完成有界结果传递后，才向输出 meta 赋值并保存 Complete。
后续观察终态不再次执行生产者。公开的 `new_resolving_in_scope` 使用 retained source
及选择服务，避免为使用该协议先急切物化整个 dispatch 世界。

取消、Drop、宿主错误、执行或发布预算耗尽都会终结本次计算；依赖它的生产者沿原
需求边失败，不产生占位 Type。取消不退还尚在宿主响应中的额度；旧答案交给已终结
句柄或另一会话时原样拒绝，不消费接收方当前等待者。新的独立会话有独立预算，
不能借重试给原会话补充额度。失败前未耗尽额度的会话仍能处理独立计算。

源码 required Type 路径已通过 `resume_with_code_session` 使用这一驱动：代码会话
覆盖执行的完整生命周期，关联选择在 VM 外处理，完成后才发布结果。参数冻结、
prepared value 准入和结果发布与旧同步接口共用；Live 值保留图与域信息，所有 Live
输入先通过准入，随后才执行 Normal 输入工厂。没有因接入会话而改成全量 Term 化。

边界仍然明确：旧 `resume`／依赖批量驱动保留同步入口；输入工厂、类型准入、方法
解析和 builder 服务还没有全部变成可恢复请求。暂停中可以进行独立推断，多个独立
Type 会话也可交错发布到同一上下文；一个会话的 VM 和队列仍由活动句柄独占，
required 的便利驱动仍同步处理宿主请求至完成。
一般源码检查 Region、实际输入生产者与待定 callable 签名之间的双向交接尚未实现，
两个既有普通调用正例仍 ignored。此次没有提前求值 Type 生产者、重跑调用 AST，
也没有放宽原正例或更改正式 docs。

真实 sysroot 的 `type_invocation_sessions` 验证 Type 源码前缀只执行一次、关联
初始化器在恢复后执行一次、等待保持现场、终态不重放、取消传播、迟到响应拒绝、
宿主失败／耗尽，以及 Live 输入等待和域拒绝后的独立工作。直接源码案例另覆盖
required Type 调用到运行时值的路径；这些不是整个普通 callable 恢复能力的验收。

验证工作目录为 `/home/fin/luna`，Cargo 均使用
`python3 -B development/check.py --timeout 900 -- test`，输出锚点为各日志的
`test result:`；以下结果均退出 0：

- `-p comptime_interpretor -p elaboration --lib --no-fail-fast`：
  `/tmp/luna-tools-pyrcgb9v/result.json`，VM 580 项、elaboration 193 项通过。
  随后仅清理 elaboration 已无消费者的同步 code-session 分支，VM 文件未再改变。
- 最终 `-p elaboration --lib --test type_computation_demands --test delivered_types --test invocation_budget --no-fail-fast`：
  `/tmp/luna-tools-ewat3vcl/result.json`，212 项通过。
- 最终 `-p luna --test type_invocation_sessions --test invocation_host_requests --test builder_invocations --test prepared_invocations --test ordinary_callable_types --test ordinary_clause_collectors --test required_input_scopes --test deferred_associated_values --test staged_invocation_reuse --test default_calls --test cmtt --test cmtt_type_inputs --test library_protocols --no-fail-fast`：
  `/tmp/luna-tools-x41aigbo/result.json`，13 组共 860 项通过、两项既有正例 ignored。
- 七个修改 Rust 文件的限定范围 `rustfmt --check`：
  `/tmp/luna-tools-a1o1zqsd/result.json`；`git diff --check` 通过。

本轮保留了作者的资源分析修改，是工作区验证；没有执行全 workspace、性能基准或
纯 HEAD 验收。`compiler/fmt`、正式 docs 和既有库文档改动不属于本次提交范围。

### 单输出发布许可允许暂停中的独立推断

初始 invocation 接线为保护结果赋值而借用整个 `InferenceContext`。这使等待宿主时
无法处理其他输入，也使两个独立生产者不能在同一上下文中交错推进。解除借用本身
不安全：普通赋值或候选提交可能抢先改写生产者的输出，直到执行完成才发现冲突。

现由 `type_resolution::AssignmentPermit` 表示某个已提交上下文内、单个未赋值 meta
的发布权。执行前保留输出并推进 revision，旧候选不能覆盖这次保留。许可不可复制；
普通及有界 assign 对该输出返回 ReservedVariable，候选不能取得或使用生产者许可。
候选仍可观察其未解值、修改无关变量并正常提交；共享变量快照保留输出的归属标记。
完成时共用 occurrence／sort 检查，凭原许可赋值一次。许可和正常变量都保留原来的
错误／预算边界，不用全局 revision 相等否决无关推断进展。

`Invocation::poll(context)` 在任何 VM 进展前核对提交态及上下文身份。错投或传入
trial 返回错误，保持原执行现场、宿主请求和额度；可以随后用正确上下文继续。
正常完成才执行有界 Type 结果传递及授权发布。同步 Type 驱动同样在执行前保留
输出，不能成为绕过归属的第二条路径。Drop／取消只放弃发布，输出继续不可由普通
推断写入；计算队列负责保留 Failed 并沿依赖传播，不能将取消当作可重新执行。

这里没有改变自由推断变量的写入规则；只有已开始生产的输出使用许可。尚在输入
等待中的输出仍由原依赖预检检查外部赋值冲突。许可也不是语言级 capability、PTC
命题或持久化 receipt；它只管理活动编译任务的单次结果交付。完整源码 Region、
候选义务所有权及前端检查请求仍按前述边界继续迁移。

该许可接线的验证在 `/home/fin/luna` 执行，Cargo 入口仍为
`python3 -B development/check.py --timeout 900 -- test`；各报告退出 0，计数锚点
为 `stdout.log` 中的 `test result:`：

- `-p type_resolution --tests --no-fail-fast`：
  `/tmp/luna-tools-3255i47o/result.json`，396 项通过。包含普通／有界写入拒绝、
  旧候选拒绝、无关候选提交、错误域与 foreign/trial 发布，以及放弃许可的四项回归。
- `-p elaboration --lib --test type_computation_demands --test delivered_types --test invocation_budget -p luna --test type_invocation_sessions --test ordinary_callable_types --test ordinary_clause_collectors --test builder_invocations --test required_input_scopes --test prepared_invocations --test invocation_host_requests --no-fail-fast`：
  `/tmp/luna-tools-_o1_sir4/result.json`，461 项通过、两项既有正例 ignored。
  十项源码会话测试包括暂停中推断、候选干扰、错误上下文 poll，以及两个独立
  VM 向同一个推断上下文交错发布；跨会话误投响应仍可送回原生产者完成一次。
- `-p luna --test execution type_computation`：
  `/tmp/luna-tools-eha1mgx5/result.json`，九项既有同步／依赖驱动回归通过，覆盖
  外部赋值冲突、循环、live capture 和不重放；其余 1,359 项按名称过滤，未执行。
- 七个修改 Rust 文件的限定范围 `rustfmt --check`：
  `/tmp/luna-tools-3osuj9yg/result.json`；`git diff --check` 通过。

以上仍为保留既有工作区修改的针对性验证；没有运行全 workspace 或性能基准。

### 定义准备也能离开源 invocation 驱动

字节码的 `BuildDefinition` 原本虽已发出 preparation，却在下一驱动步同步执行
builder 回调。现已将 struct、enum、newtype 的处理拆成共同的三段：解码并检查
源描述；宿主准备未提交的 receipt；原 VM 核对描述、完成目录准入后安装。
源调用返回 `DefinitionHostRequest`，请求持有原描述、源世界、线性 ticket 和独占
预算额度；宿主可以保存请求，待其依赖就绪后再交回 prepared receipt。

暂停期间原字节码 frame 保留指令位置和目标槽，输入包已经消费，不再执行生产者。
外层 Type 输出仍未赋值，接收响应也不立即安装：下一次 poll 才进入原有描述核对与
发布边界。错误类型、篡改字段／表示、无效定义及宿主虚增预算均不得发布。
误投响应原样退回；取消销毁活动 receipt，迟到响应不能命中新 invocation。
这里的取消不撤销宿主 prepare 已经发生的外部行为，也不会退款未知工作量。

Type 队列驱动与 luna 源 invocation 入口均处理这个请求，默认同步适配器在 VM 外
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

Type 队列与 luna 源调用均转发 `MethodHostRequest`；默认适配器在驱动外调用现有
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
