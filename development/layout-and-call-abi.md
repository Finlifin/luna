# 表示观察、开放布局与调用 ABI

状态：内部重构设计，未实现。本文承接[类型引用与环境](type-references-and-environments.md)，
补齐从已检查类型到存储、调用及 LLVM 输入的协议；不修改源码或正式 docs。
依据为[反射与布局](../docs/type/reflection-and-layout.md)、
[运行时依赖类型](../docs/type/runtime-dependent-type.md)、
[指针](../docs/resource-management/pointers.md)、
[IIR 交付](../docs/compilation/iir-and-codegen.md)及
[效应 ABI](../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)。

## 1. 当前代码的职责和缺口

| 源码与入口 | 现有机制 | 重构边界 |
| --- | --- | --- |
| [nir/target.rs](../compiler/nir/src/target.rs) 的 TargetData | 显式 pointer_bits，usize/isize 与固定整数表示分开 | 尚不是完整大小、对齐、字节序及调用约定描述 |
| [iir/lib.rs](../compiler/iir/src/lib.rs) 的 Package、FunctionInstance | 完成类型检查、已选 dispatch、名义结构、原始高级 Function | 注解仍主要为 Term，不能代替生产 TypeRef/EnvironmentPlan |
| [iir/storage.rs](../compiler/iir/src/storage.rs) | runtime 存储阶段检查，区分 staged 实参与 runtime 实参 | 不等于目标布局、传参或对象存活已验证 |
| [host/llvm.rs](../compiler/luna/src/host/llvm.rs) 的 Ty、repr_at | 私有机器表示；指针截断按值递归；共享展开预算；名义表示与语义类型分开 | 直接递归构造 LLVM 形状尚不是统一的布局观察服务 |
| [normal_forms.rs](../compiler/nir/src/normal_forms.rs) 的 Head/Role | 以声明绑定记录可信 builtin 结构视图，NF 身份不展开 | 下游 tuple_elements 专门入口须迁到通用受检表示观察；不能扩大名称特判 |
| [llvm/enums.rs](../compiler/luna/src/host/llvm/enums.rs)、[optional.rs](../compiler/luna/src/host/llvm/optional.rs)、[errors.rs](../compiler/luna/src/host/llvm/errors.rs) | 固定位编码、分层可选、闭合错误行载荷及转换 | 当前 tag/载荷排列是私有 host 选择，不是全目标公共 ABI |
| [host/mod.rs](../compiler/luna/src/host/mod.rs) 的 Prepared、llvm_ir_prepared | graph/schedule 成对交付，inline 后重建资源依据，发射前复核 | 应保留入口限制，再替换为显式表示/调用计划消费者 |
| [program/attributes.rs](../compiler/elaboration/src/program/attributes.rs) 的 prepare_phase_attributes | 普通属性求值；未接入消费者的字段明确拒绝 | attrs.abi 已有构造器不代表 C ABI 已接通 |
| [llvm/entry.rs](../compiler/luna/src/host/llvm/entry.rs) | 开发用结果打印适配 | 不能作为语言公开入口 ABI，也不能充当 std 格式化接口 |

以上是静态证据，没有运行 native 测试。现有 `repr_at` 已对展开节点数和深度收费，
不能在重构中退化成无界递归。当前 optional 的 `{i1, payload}`、错误结果的标签和
逐分支载荷、闭包的代码/环境/状态位均为实现选择，不从测试字串倒推语言承诺。

## 2. 四个不同的问题

```text
TypeRef + formation evidence       -> 这是什么类型，依赖谁
RepresentationObservation          -> 有什么合法结构/编码，哪些输入影响表示
LayoutPlan + environment           -> 对象在哪里、多大、怎样访问
CallAbiPlan + control contract     -> 已取得的值怎样交付给这次调用
```

类型相等、表示相同和可按某个 ABI 调用分别检查。相同 LLVM 类型不能授予转换、Copy、
Drop、指针访问或函数调用资格。newtype 保持直接底层的大小/对齐/合法编码，同时保持
独立身份和资源操作；不能把这项明确契约推广到任意同宽类型。

布局服务必须能被 normalization 中的反射和 safe 检查调用，不可要求先生成整份 IIR。
它消费稳定结构观察、表示契约及目标输入；IIR 和后端复用同一结果及其依赖。
若反射只问字段，不要顺便求布局；若只问 `*T` 的本体，不要递归展开 T 的本体。

观察 NF 时保留头、普通输入值及形成证据。可信的结构视图不是 `NF = representation`
等式，更不能通过执行头的函数体制造类型相等。普通调用仍按自身执行规则处理。
统一观察入口覆盖源码结构、生成定义、newtype 和 compiler builtin 绑定；没有足够
表示依据的用户 NF 返回缺少依据，不能仅因叫 Tuple 或输入是 List<Type> 就准入。
Tuple 的普通签名、参数绑定和 NF 身份不由目标服务重新解释。

## 3. 布局结果必须分清状态

内部观察返回以下不同结果，不以 `Option<size>` 或 `is_sized` 布尔值包办：

| 结果 | 含义 | 消费者怎样处理 |
| --- | --- | --- |
| Fixed(plan) | 在当前目标及已完成输入下，存储大小/对齐等已固定 | 使用计划及证据；操作环境仍可能存在 |
| ValueDependent(plan) | 大小/偏移等布局量依赖合法 runtime 元数据，取得算法已知 | 在受检使用点取得元数据，兑现相应存储协议 |
| Blocked(observation) | 所需 comptime 输入/结构/目标有真实生产者，尚未就绪 | 在合法前端阶段等待；不进入成功的目标输入 |
| Unavailable(reason) | 缺少必要表示依据或环境，没有可等待生产者 | 诊断具体缺项，不能伪造动态布局 |
| Invalid(reason) | 已证明表示不合法，例如不能形成有限本体的按值环 | 拒绝并给出依赖路径 |
| Unsupported(target, reason) | 合法表示超出本目标或实现支持 | 报告目标限制，不修改类型身份 |
| Exhausted(budget) | 尚未在预算内决定 | 保留未知原因；不缓存为永久 Invalid |

`is_not_sized` 描述值相关大小，不描述检查进度。泛型参数未知时不能因此返回 true；
也不能因尚无动态布局实现就把所有合法类型假定为 Fixed。
这里的计划是内部结果，不给 `Type.size_of` 偷加一个 runtime 求值行为。请求 comptime
数值的观察必须满足该观察的输入条件，不能把 ValueDependent 的占位数返回给用户。

Fixed 还要区分无载荷、零存储大小与无可构造值。例如 void 的正常完成与 NoReturn 的
无正常完成不同；机器返回通道都可不携带数据，但控制语义不能合并。
`?NoReturn` 的缺失分支不能因为其主体无值而一起删掉；嵌套 optional 保留每层状态。

## 4. LayoutPlan 的内容与算法

拟议内部结构：

```text
LayoutPlan {
  logical_type, representation_source, target_revision,
  size_and_alignment, allocation_extent,
  projections: logical_field_or_branch -> physical_access,
  validity: branch_tags + legal_encodings + required_guards,
  representation_dependencies, environment_requirements,
  evidence, query_dependencies
}
```

逻辑字段、初始化与清理顺序不等于物理偏移顺序。表示属性与目标可以决定物理排列，
调用/资源计划仍保持源语言求值和析构次序。位字段的 projection 可以是位片段，不能
无条件生成可取地址的字段指针。普通 struct 也不能从 C 经验推定按声明顺序布局。

target 数据区分值位宽、存储大小、分配跨度、对齐、字节序及相关调用规则。LLVM 的
`i1` 或 `iN` 不是可直接当作字节 sizeof 的答案；bf16 的传送比特与计算表示也要分别保存。
不通过运行编译器的 Rust `size_of` 决定目标布局。已有 TargetLayout 的 triple/data_layout
是所选 LLVM 工具链事实，尚需受检解释，不能把仅有 pointer_bits 的 TargetData 叫作完整目标。

计划构造分为：取得合法结构/表示契约 → 在 substitution 下观察按值依赖 → 校验递归与
编码 → 按 target 组合大小/偏移 → 生成投影和访问条件 → 发布不可变计划。
按值依赖图以实际类型应用及表示来源为节点；指针只依赖自身目标指针表示，不展开 pointee。
递归遍历遇正在计算的按值节点时给出环；遇共享的已完成节点时复用，不能一概拒绝递归名义类型。
更一般的值相关递归若没有有限表示依据，不能仅以 ValueDependent 标签绕过检查。

常量大小计算使用受预算的数学整数或等价的显式溢出检查，最后验证目标可寻址范围，
不先窄化成编译宿主 usize。重复引用 DAG 节点可以省结构存储；实际对象的展开大小、
投影数量、编码生成和输出成本仍计费，不能把指数大小对象按几个节点收费后无界分配。

布局缓存保存表示依赖而非每个类型依赖的运行时内容。相同 core 不足以命中，必须核验
会影响表示的替换、属性、结构版本和 target。只供操作使用的校准对象不必改变本体布局；
缓存返回共享布局也不合并两个依赖该不同对象的 TypeRef。

## 5. 值相关布局：薄指针与显式元数据环境

普通 `*T` 与 usize 同宽。选择将动态布局信息交给 EnvironmentPlan/受检携带者，
不改变普通指针表示，也不默认给每个依赖值增加一个 metadata 指针。
与类型身份类似，元数据取得方式可以是已有实参、外层聚合或合法封闭环境。
仅知道某个对象的类型，不能凭空取得它的长度、当前内容或访问许可。

内部动态计划可以包含 checked_add、checked_mul、align_up、受 guard 的分支选择和
从合法环境读取元数据的动作。它是已知程序的布局计算，不是遗留 comptime 表达式在
runtime 重跑，也不在 runtime 搜索 trait 或求解 Type。具体公开动态容器 API 不由此引入。

访问协议按以下步骤交付：

1. 取得布局所需的元数据，核验来源、域、权限与存储世代。
2. 在资源契约允许的位置计算大小、偏移与对齐，检查范围和运算前提。
3. 取得或验证足够存储，建立已初始化部分及清理责任。
4. 投影、搬移、调用、返回与销毁消费同一布局依据，并维护来源与责任。
5. 责任结束后才释放存储及相应环境；部分构造失败只清理实际取得的部分。

元数据随对象变化时，不能继续复用旧内容版本的偏移。稳定布局描述可伴随一次分配保存；
若操作允许改变表示，必须显式重建存储/环境并处理旧责任，不能原地篡改已发布类型描述。
涉及捕获/挂起时还要保持所需地址稳定和 owner；动态 alloca 不是自动可逃逸的存储方案。

safe 的上述前提在 NIR 的符号计划中检查。自动算术仍遵守三次以内多元多项式边界；
align_up、除法和机器溢出使用明确的操作契约，不能当作任意多项式恒等式。
超出静态能力时只在已有合法运行时检查协议下使用 assumes，并保留检查次数与失败清理。
PTC 不接收“此地址当前可用”或动态分配范围作为类型命题。

## 6. CallAbiPlan：先取得值，再排列机器参数

语言签名、函数 ABI 属性、数据表示和效应控制协议分别保存，最后组合受检调用计划：

```text
CallAbiPlan {
  callable_contract, calling_convention, target_revision,
  source_slots_to_machine_arguments,
  environment_delivery, argument_transfers, result_delivery,
  error_representation, effect_entries, control_exits,
  cleanup_and_failure_edges, compatibility_evidence
}
```

ArgumentPlan 已决定源实参、默认值和 receiver 的执行依赖。CallAbiPlan 只对已取得值
安排寄存器/栈槽、拆分聚合、间接传参及结果位置；不能因机器参数排序重复求值或提前
执行默认值。省去零大小机器参数不省去它的求值、转移或 Drop。
采用 by-reference 传送拥有值也不把 move 变成借用或授予 Copy。

调用者、被调用定义、取函数值、间接调用和导出入口共享同一 ABI 计划来源。内部 thunk
若调整调用约定，必须有显式适配证据、两端契约及缓存依赖，不能仅按指针位宽转换。
所有机器函数地址可具有 LLVM ptr 形状，但仍须保留其完整可调用签名和控制协议。

闭包的代码及环境、依赖类型的外层环境、效应 evidence 可以共享传送基础设施，
其 owner 和生命周期保持各自契约。无环境 callable 身份、捕获闭包和 template 的
身份标签不是同一种机器地址。当前 template 表内 i64 标签不成为跨产物稳定符号。
代码共享须证明表示、行为和 ABI 参数化正确；不能按相同 TypeCore 或相同机器类型合并。

## 7. 错误、效应与外部调用

| 通道 | ABI 需要保存的信息 | 禁止合并的内容 |
| --- | --- | --- |
| 普通结果/可选 | 返回值与每层存在状态，合法载荷编码 | 未初始化载荷不能凭标签布局被读取 |
| 错误结果 | 完整错误类型标签、成功/错误载荷、行嵌入转换 | 不携带恢复权，不把内部中止包装成用户 error |
| direct effect | operation occurrence、入口、分支环境及外层上下文 | 不按同名操作合并不同安装层 |
| direct passing | 安装时取得的值及其资源关系 | 不在每次请求时重新执行提供分支 |
| 捕获/恢复 | 单次恢复权、内部处理关系、当前恢复位置、栈/环境来源 | 不把已失效的安装者返回地址或外层 evidence 永久缓存 |
| async | 子工作/Future 的协议、调度及资源交付 | 不由后端擅自转换所有普通 effect 为 async |

错误行只提供分支关系，表示转换还需明确重编码方案。不同错误行即使大小相同也不
直接 bitcast。optional 利用空闲编码时必须证明所有嵌套状态可区分；这不引入 Option/Result。
NoReturn 只排除正常结果，不消除失败/中止路径需要的清理；不能仅凭它就给包含隐藏
控制返回的机器函数标记 noreturn。

内部失败/非局部控制状态属于调用激活。当前 host 的 flurry_failed/flurry_effect_control
已是 thread_local，入口也有外层控制指针保存/恢复；不能误报为所有线程共用普通全局槽。
这些机制仍不直接证明任意重入或 continuation 跨激活恢复的正确性：TLS 归属线程，
不自动等于恢复权/任务归属。目标设计通过显式控制环境传递，不需要该通道的调用可消去。
迁移需同步调用入口、返回、handler、cleanup 和开发包装器，保留已有隔离和恢复行为。

首版捕获后端沿正式设计采用地址稳定的有栈模型；pool 来源随栈段保留，恢复点重新
供应外层 evidence，runtime 可封装 pool。这里只确定调用契约，不先换用状态机后端。
inline effect 在 callee 与 handler 联合展开并维护清理后可以消去部分交接，不能只删入口参数。

函数外部 ABI 由 `attrs.abi` 等普通属性产生，未知/尚未接通的 ABI 仍明确拒绝。
C ABI 入口效应行必须为空，且仍需满足目标数据表示和其他边界约束；空行不是全部条件。
若需环境，由合法包装在 Flurry 边界内取得，不能给 C 接口静默追加隐藏 Flurry 参数。
C 帧允许保留后恢复，不能被放弃越过；无法静态排除时保留正式设计要求的运行时屏障。
具体平台分类器须按选定目标约定实现，不把当前内部 LLVM 聚合传参自动称作 C ABI。

## 8. 从计划到 LLVM：验证与退役边界

LLVM 输入由稳定 IIR、已维护的 safe/verified 证据、LayoutPlan、CallAbiPlan、清理图
和 target 合规结果共同构成。发射器不重新搜索实现、猜环境或执行 comptime 类型工厂。
结构化循环、match、闭包和 handler 可先在高级层变换，再由目标转换产生 SSA；
不因使用 LLVM 就把 IIR 预设为 SSA。

每个改变控制、投影、捕获或表示的 pass 说明哪些证据保持、哪些需重新构造。
布局变化必须维护字段到实际访问的映射和资源区间；inline 后不能复用旧 cleanup。
LLVM noalias、nonnull、dereferenceable、对齐、完成性及内存属性只能由适用的证据产生，
不能从普通指针、P4 或一次布局查询直接推导。

迁移顺序为：先统一受检结构观察和静态布局 → 同步反射/资源/host 消费者 → 统一调用
计划与 ABI 属性接线 → 交付动态环境/布局 → 扩展控制后端。阶段实现不改变最终契约；
未支持路径保留具体拒绝，不放宽现有验证入口让未经检查的代码先运行。

`Ty` 可暂作新计划到 LLVM 的物理载体；当反射、指针投影、聚合、调用、返回和清理都
消费新计划后，才删除 repr_at 中重复的语义选择。保留已有预算、错误行转换和成对
graph/schedule 准入。新的布局模块如果生产入口还在走旧 match，不算完成迁移。

所有新计划同步提供 S-expression Display，显示语义类型/表示来源、目标、动态输入、
逻辑到物理投影、调用参数映射和控制出口。完整类型签名不能在 dump 中被一个 ptr 掩盖。
持久产物记录这些契约的版本与依赖，接入[产物发布](build-and-artifact-delivery.md)。

## 9. 验收与未完成边界

后续实现至少覆盖：

- 指针递归合法、按值无限环拒绝；相同 DAG 的共享与实际展开预算分别计量。
- 固定本体带不同 runtime 对象依赖时可共享布局，但类型、环境和 Drop 不混同。
- 同一 TypeRef 的字段观察不强求布局，布局等待不被误报为值相关大小。
- 薄指针访问动态布局需要真实元数据环境；存储结束或版本失效后拒绝旧投影。
- 零大小拥有值仍求值/清理；NoReturn 与 void、各层 optional 状态保持区别。
- 错误行转换保存完整标签和载荷；内部中止不能被用户错误 handler 截获。
- 具名参数、默认值和环境求值顺序不因 ABI 重排改变；直接/间接调用采用相同计划。
- ABI 属性经过真实 attrs 公开接口进入检查、取函数值及降码；不支持的目标明确失败。
- inline、捕获、返回、恢复/放弃均交付本次结构对应的资源证据；不同 target 不复用旧计划。

这不是已通过的测试列表。本轮未运行编译器测试；各平台 C ABI 分类、动态布局具体
编码与控制后端机器约定仍须在这些边界内细化，不以目前 LLVM 字串或未实现的公开 API
补定语言规则。源码迁移仍未开始。
