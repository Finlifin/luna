# 求解、程序规格与目标交付

状态：设计。三者可以交换明确的证据，不能共用一个无边界的“证明上下文”。
当前实现可复用的组件及缺口见 [架构审计](architecture-audit.md)。
PTC 的搜索、投影、世界适配与重放协议详见[Type 模型与理论证据](type-models-and-theory-evidence.md)。

## 1. PTC 先把命题、观察和操作分开

正式目录 A01–A19 是语义模式，不要求对应 19 个 Rust enum 分支。
也不能因为当前 Atom 只有四种就宣称其余模式已经覆盖。

| 模式 | 目标解释与主责 | 接入时需要的证据 |
| --- | --- | --- |
| A01 Eq | EUF 加各语义域的相等 | 合法同余、binder/world/sort 一致；NF 不透明 |
| A02 Kind、A03 InUniverse、A04 Qualified | 类型分类与限定观察 | 合法分类轴，禁止把所有类别作互斥枚举 |
| A05 HasSlot | 结构/序列观察 | 选择器种类、结构前提、开放信息范围 |
| A06 HasHeadShape、A07 Matches | 来源形状及语法制导匹配 | 成功绑定与来源另给；不能由等式假定逆函数 |
| A08 Compatible、A09 Subtype | 各自关系 | Any 相容不入等式闭包，不增加通用协变 |
| A10 Implements | trait 实现世界 | 正证据为合法选择；负证据覆盖完整相关候选 |
| A11 NotSized | 目标表示性质 | 域适用且存在表示；输入未知不等于值相关大小 |
| A12 IntLe、A13 IntCongruent | 数学整数片段 | 整数性、模数前提；不拿有理数解充整数解 |
| A14/A15 BV comparison | 已定宽位向量 | 宽度、符号解释、有限域模型 |
| A16 RowContains | 行成员 | 错误行/效应行 kind 及开放尾部 |
| A17/A18 error RowSubset/Disjoint | 唯一错误行关系 | 不自动生成值转换，不用于 scoped effect 路由 |
| A19 PredicateCall | 稳定的准入命题 | 定义、参数、实际依赖、展开或执行依据 |

字段类型、长度、布局大小和关联项是带前提的观察项，不必各加一个命题原子。
实现选择、接收者适配、row split 和效应 embedding 则返回操作证据，不只返回 true。
每个模式的正负证据、模型闭合义务及当前四类 Atom 的局限，见
[原子准入与覆盖声明](type-models-and-theory-evidence.md#11-原子准入与覆盖声明)。
覆盖针对具体公式和片段登记，不用一个 supports 标记混淆传播、证明与模型能力。

## 2. 完整 CDCL(T) 的交付边界

已有 trail、first-UIP、学习、根子句传播和 theory 状态保留，按接口逐步统一：

```text
Theory.register(term, formation_evidence)
Theory.assume(literal, level, reason)
Theory.propagate() -> explained literals | conflict | no progress
Theory.rollback(checkpoint)
Theory.model_fragment() -> interpretation | unresolved reason
Theory.dependencies() -> world / guards / premises
```

同一决策层的传播理由必须足以重放；回退同时恢复 theory 状态、队列和理由引用。
根 CNF 辅助变量属于布尔编码，不伪装成 Type 变量。学习子句可以比一次候选更长寿，
但必须保留适用 world、activation guards 和前提；候选失败不发布事实。

Theory fragment 不能拼凑成未经检查的源模型。共享等式、有限域、部分函数前提、
实现世界和量词范围都要联合验证；无法解释的 trait 或其他项产生 Unknown。
不要在 trait 查询失败后随意给原子 false 来获得 SAT 模型。

独立 proof/model 检查器不共享搜索中的可变状态。可共享纯语义定义，不能复用可能
出错的搜索缓存来证明缓存本身正确。模型候选枚举穷尽预算不等于公式不可满足。

证明 Γ 蕴含 G、寻找 Γ ∧ ¬G 的反例、提出类型替换是不同接口。
NotEntailed 的反例不等于 Γ 蕴含 ¬G；可据反例拒绝必须对全域成立的要求，不能直接
将其作为负命题传播。Theory 的 Quiescent 同样只表示没有新传播，不表示模型已合法。

## 3. Type 模型搜索作为 unification 托底

先尝试局部结构分解和合法观察，再把获准求解的 Type 输入、原始匹配关系、环境前提
交给模型搜索。显式输入和定义端刚性参数不加入可任意赋值集合。

求解结果区分：无解且有证明、唯一/可提交的选定投影、歧义、未知。
一个模型仅证明某次赋值可行；若提交要求该输出被确定，需要在原问题下证明该投影，
或由语言明确的选择规则授权。辅助变量可以不唯一；符号依赖也可以保留为合法 Type 项。

候选导出同时检查 scope、sort、occurs、参数域及原义务。不把候选 requires 加成区域
假设后再宣布候选成立。没有选择依据时请用户给类型，不扩张隐式推断以隐藏歧义。

投影证明在临时匹配问题中成立，不免除回到真实前提后的原义务检查。例如 X=u8 与
外围刚性 A=u8 同时加入搜索可以得到 X=u8，却不能证明任意 A 的调用都合法。
相反，X=A 可导出符号 A；不同外围 A 产生的不同具体模型不构成同环境内的歧义。

NF 输入匹配可以提出一个实参选择，再证明原始等式；这不授予工厂单射。
一般 symbol/type/value 输入的域解释通过通用冻结值协议，不能为 Tuple/List 单开出口。

## 4. 程序规格与 PTC 的桥

safe 的全部检查位于 normalized IR；verified 检查位于 instantiated IR。
语义清理、调用环境与效果必须参与 NIR safe 检查，不能以日后特化可能成功代替当前必检义务。
实例化和降码维护已交付证据；新生成代码也经过相应 normalization/safe 检查。

类型层可以提供已成立的类型关系、选定实现、静态布局和接口契约；程序验证层将这些
作为有来源的输入，在实际控制流中检查前后条件、移动、访问、初始化及 cleanup。
程序层的“该地址当前有效”不能反过来作为任意 Type 命题。

safe 自动算术采用受预算的三次以内多元多项式处理，保留机器运算范围、除法/索引前提，
不把数学等式直接改写成可能溢出的机器计算。Unknown 保留可检查的剩余义务。
assumes 的条件求值次数、稳定性及有限完成另行检查，成功路径才取得事实。
没有合法 runtime 检查的义务不能凭 Unknown 自动插入一个不明操作。

verified 的高级证明扩展保持接口边界，本阶段不设计大型验证库。

## 5. 统一资源描述，区分具体含义

资源证据采用逻辑路径、物理位置、分支 guard、时间/存储版本和责任所有者的组合。
至少区分：

- 拥有位置及初始化/移动责任。
- 指针目标的可访问区域。
- str 等视图的 backing region。
- 闭包/handler 保存的代码及环境依赖。
- 类型依赖所需的 runtime 环境。
- 析构及非局部控制转移的清理责任。

这些证据可以共享路径和集合算法，但不能相互替代。`*str` 指向描述符的位置，
不等于 str 指向的字符区域。类型参数提供对象身份，不自动提供借用许可。
依赖域、取得路径和责任怎样交付，见[类型引用与环境](type-references-and-environments.md)。

目标摘要大致为：输入相对 read/write/kill/初始化与返回依赖、受 guard 限定的结果路径、
回调及捕获要求、所有出口的清理足迹、控制转移与完成性。每个摘要有明确前提和实例版本。
摘要推导可递归迭代，但不能用空摘要打断递归并当成已证明无效果。

## 6. str 是贯通资源设计的首个案例

1. callee 摘要保存参数相对文本区域，不能保存 callee 局部地址作为 caller 根。
2. caller 按实参准备顺序将摘要代入实际来源，证明每次读的区域存活。
3. 返回视图绑定到实际结果路径，分支合并保留所有可达来源及 guards。
4. 描述符更新更新其依赖，复制描述符不延长字符存储存活，也不消灭原拥有者。
5. 捕获、错误载荷、可选载荷、handler 与 cleanup 使用同一传递规则。
6. 最后再迁移 builtin TextBytes 的旧直降绑定为普通带类型调用。

已有 TextRegionSummary 可作事实生产者；不能删除 pointer_input_layout 的 Text 拒绝
就当完成。需改所有相关消费者后运行真实 sysroot 的原生调用。

### 6.1 已有来源分析不等于可执行资源证据

2026-11-04 对[六包基线](libraries-and-delivery.md#111-真实六包的两阶段方法基线)中
被资源入口拒绝的同一 public.fl，执行了独立来源分析：

```sh
python3 -B development/reproduce.py --timeout 60 --binary /tmp/luna-design-baseline-driver -- analyze text-regions /tmp/luna-library-phase-gcjlk26j/public.fl --entry run --sysroot /home/fin/luna/library --build-target x86_64 linux 64 --fuel 4000000
```

工作目录 `/home/fin/luna`，退出 0，输出含 `(reads (static-text-region))`，returned
为空（本例返回 bool，不携带文本）。报告在 `/tmp/luna-tools-21fvo5l9/result.json`，
stdout.log 保存完整摘要，stderr 为空。未重新执行已失败的 interpret，也未改动输入。

源码接线与这一结果一致：`luna/src/text_regions.rs` 通过独立 analyze 命令调用
`infer_text_regions`；`luna/src/cleanup.rs::field_borrows`、moves 的调用/捕获/返回
路径和 purity 的资源摘要仍消费 `pointer_input_layout`。后者遇 Text 明确拒绝，
`owned_fields` 也另有 str 拒绝。因此不能只放开一个入口，更不能将 Text 过滤后返回
空指针集合，伪装成没有借用依赖。分析命令成功不证明整条调用有可执行的清理计划。

现有 TextRegionSummary 的 reads/returned 是来源信息，明确不含完整 cleanup 足迹；
Static 表示静态来源类别，Parameter 带参数编号与逻辑字段路径。它不是内存地址表，
也没有交付每次实际访问的权限、范围与存活证明。未来需要区分静态分配身份时，在
物化/存储服务中补交付，不能把所有 Static 来源合并为同一个可写或可别名对象。

### 6.2 调用边界交付的最小契约

资源层统一使用带来源的借用描述，Pointer 与 Text 是不同的观察种类。第一批接线
可以覆盖文本，但数据契约不能把字符存储硬编码成任意指针的 pointee。下列项分别拥有
证明责任；它们不是一个含混的 `valid` 标记，也不交给 PTC 求解：

| 交付项 | 生产与消费规则 |
| --- | --- |
| 叶位置 | 带输入/结果的逻辑路径、物理描述符位置与外到内的 payload guards；布局只说明如何找叶 |
| 区域来源 | 静态存储、调用者视图或已建立的拥有存储，附所需范围；不得以 callee 局部地址作为 caller 根 |
| 访问足迹 | 对描述符与 backing storage 的读写分别记录，关联控制条件、调用/回调及 cleanup 出口 |
| 有效性义务 | 对每个可达访问证明存活、已初始化、权限及范围；Copy/纯度不替代这些证明 |
| 变更与结果 | 描述符更新、区域失效、返回依赖及其 guards；按实际顺序作用于调用者状态 |

callee 可以在“调用者提供有效视图”的显式前提下形成条件摘要；这不是替调用者断言
区域真实存在。调用者先按源规则求值实参一次，固定传入的描述符值，再把参数相对
来源代入实际区域，检查足迹及全部出口，最后交付结果依赖。未知回调需要其受检契约
或真实摘要；没有来源的活动 Text 叶保持未决/拒绝，不得自动视为静态存储。

描述符的值版本与 backing storage 的存活是两个状态：

- 复制描述符只复制来源关系，不延长 backing storage 的寿命，也不转移其释放责任。
- 经 `*str` 更新描述符时更新该位置及别名观察；此前复制出的旧描述符仍指向旧区域。
  若无法确定唯一写入目标，合并可能结果并撤销不再成立的观察，不能做无依据的强更新。
- backing storage 被释放、移动到不保持地址的位置或失效时，拒绝后续依赖它的访问；
  描述符本身仍在作用域内不构成反证。保持存储身份的合法移动按实际契约处理。
- 分支汇合可保守合并所有可能来源，但不能漏掉某条分支；访问受 payload guard 支配，
  不得先读取 inactive optional/error 载荷再检查 tag。丢失相关性时宁可保留额外义务。
- 未知、活动叶没有来源、分支没有该叶、无正常返回是不同状态。空集合不能兼任它们。
  递归分析可用 bottom 作为私有迭代种子，所有依赖完成并收敛前不得发布为已验证摘要。

最后一条保留现有 `text_regions.rs::Query::solve/function_with_callbacks` 的有效设计：
发现实例后排队，依赖增长重排调用者，失败体延后或报错；不是禁止使用固定点算法。
新摘要须把 cleanup/回调及失效边一并纳入求解，不将当前有限 reads/returned 包装为
全量证书。摘要缓存属于其定义世界与实例，跨世界复用另按观察依赖检查。

### 6.3 迁移闭包与反例

同一次接线要覆盖入口前提、普通/间接调用、捕获/handler、返回载荷、描述符写回、
所有出口 cleanup，以及 purity 对内存足迹的读取；共用来源替换与 guard 规则。
已有 Text 拒绝只能在对应消费者真正接管后移除，未迁移路径继续明确拒绝。
不得改变 builtin.TextBytes 的公开接口或按其名字放行；用户的同类 wrapper 走相同规则。

验收至少包含：静态字面量经普通调用读取；调用者两个文本参数择一返回；?str/错误
载荷的活动分支；复制后改写原描述符；backing 存储失效后的旧视图读取；闭包/析构读取；
递归摘要预算失败后的重试。正例先检查 NIR safe 证据，再检查 reference/native 的实际
调用与清理，拒绝例必须在错误访问发生前诊断。不能用现有 Static 正例替代拥有存储
和生命周期反例。这里规定的是待实现契约，本轮只执行了独立来源分析。

## 7. 纯度的优化使用方式

分析输出应包含层级、适用区间、依赖版本、读写/身份足迹、完成性和控制风险。
P4 不保证终止；P3 只有在范围内存储不变时可复用；未知回调可能破坏范围。
Drop 的读写、发散或效应属于表达式行为，不能只分析显式函数体。

后端的 memory/alias/termination 属性逐项由已成立保证导出，不直接把 P4 映射
memory(none)、P2 映射 readonly。在没有额外证据时，不移动可能 trap 或发散的操作。
优化必须保留参数求值、错误、handler 路由和资源 retirement 次数。

## 8. IIR 到 ABI 的明确顺序

IIR 完成类型的 comptime 部分和实例选择，保留高级操作、真实 runtime 依赖及 NIR 已建立的
safe 证据；verified 检查按模式在实例层执行。结构变换必须声明保留/失效哪些证据，并
验证其保持条件或重建受影响证明；不能借后端重证绕过 NIR safe 检查。
最终目标入口只接受本次结构对应的验证产物。

错误结果根据结果 ABI 编码；代数效应根据已确定的 effect strategy、occurrence 路由、
continuation 和 runtime 接口降码。两者不能用一个“可能失败调用”包装统一。
inline effect 在 callee + handler 联合展开后核对次数、提前退出和 cleanup；
ordinary/direct passing 等策略各有可验证的边界，不能由后端喜好改选语义。

闭包、依赖环境和 handler frame 分别表达其拥有/借用关系；runtime 可以封装 stack pool。
已经固定的实现和环境不能在 runtime 重新搜索来补前端漏解。

表示观察、固定/值相关布局及完整调用计划见[布局与调用 ABI](layout-and-call-abi.md)。
它们分别消费类型、target 和环境证据，不用同一个 LLVM Ty 判定所有相等、访问和调用资格。

## 9. BitVec 与目标能力

定义端 BitVec 宽度可为受约束的 comptime 整数项；具体实例必须确定。
不能在语义类型构造时强迫所有宽度立即成为 host usize，亦不意味着必须承诺
任意大宽度都能在当前资源预算和目标上物化。

源位操作形成关系表达，静态宽度、索引、已定义比特和位置写回分别检查。
先构造合法基准实现，再进行合成/优化；Unknown 使用基准实现，不能删除输入计算。
位转换保留 enum/newtype 的合法编码、所有权与表示前提。

## 10. Pipeline 与产物协议

声明接口和目标代码交付分开。Pipeline 观察的 IIR handle 必须绑定稳定版本、实例、
target 与权限；不暴露活动检查器。设备入口不自动当 host 函数指针。
intrinsic 的开放输入仍经过普通实参检查、unsafe 边界、显式结果及目标合法化。

Artifact 记录接口指纹、target、ABI、符号、数据、工具版本与实际依赖。
先生成临时完整产物，再验证并发布；失败/取消不留下可链接的半成品成功记录。
LLVM 沿用现有依赖，不为重构先引入新后端；跨目标差异在目标合法化层显式出现。

任务状态、IIR handle 的关闭与保活、工具输入、manifest 发布点以及失效/恢复规则
见[构建与产物交付](build-and-artifact-delivery.md)。接口查询与代码生成分别记录依赖；
源程序执行身份、共享构建任务和内容缓存不共用一套“相同调用只运行一次”的规则。
