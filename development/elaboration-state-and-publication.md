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
| `comptime_interpretor/src/invocation.rs` | owned activation、保存/恢复调用方、显式执行栈 | 跨稳定查询的 session 身份、发布后的语义可见范围 |
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
