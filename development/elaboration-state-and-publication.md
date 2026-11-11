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
