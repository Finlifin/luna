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

### 4.1 可选检查消除的私有事务

已证明的程序谓词检查可以消除拒绝动作，但保留条件求值及其顺序、次数和副作用。
这条优化属于程序验证层，不向 PTC 提交程序状态。无法证明、预算不足或改写后检查
失败时，使用已经通过前端检查的原函数；不能把优化 Unknown 当作接受非法程序的理由。

`program_verification::optimization` 现在复用 `nir::visit` 的有界结构遍历，删除
自己维护的递归表达式／模式遍历。结构访问次序不代表源程序执行次序；事实仍由 flow
分析按实际控制流建立。包级入口与 purity 消费同一个准备入口：

1. 有界扫描函数体；没有 CheckPredicate 时不复制函数。
2. 用共享 copy_budget 预先计量整个 Function 的拥有载荷，包括签名、名称、契约及
   后续闭包 copy-on-write。通过后才复制并分离共享闭包体。
3. 在这棵私有树上建立 flow 事实并证明、重放检查证据；同一节点多次访问必须全部
   支持消除。只将对应 CheckPredicate 改为保留条件的 Discard。
4. 对完整候选重新分析，通过后才发布；中途失败丢弃私有候选，保留原函数。

flow 的 proof site 仍是当前树的临时地址键。先分离闭包，再记录 site；改写不得移动
尚未消费的节点或重新 clone 其树。它不进入持久缓存、跨版本引用或可序列化身份。
这次清理没有建立全局 NodeId，也没有证明所有程序分析器的地址使用和宿主栈有界性。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p luna --test execution
  --test frontend_cli --no-fail-fast elision`：6 项通过，退出 0；
  `/tmp/luna-tools-err235ok/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test runtime_purity
  --test optional_purity --test call_local_facts --no-fail-fast`：73 项通过，退出 0；
  `/tmp/luna-tools-y69d69ch/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test execution
  -- --exact <直接调用 elide_proven_checks 的 26 个测试名>`：26 项通过，退出 0；
  `/tmp/luna-tools-cap6jk33/result.json` 保存完整命令，stdout.log 保存逐项结果。
  包含共享闭包、可变捕获、写回、别名失效、循环和条件求值；与首轮 5 项有重叠。

新增用例验证长函数元数据也占复制预算：低预算保留原函数，更充足的新尝试正常消除。
这些是带工作区既有 moves/effects.rs 修改的针对性结果，不是纯 HEAD 或全量验收。

### 4.2 共享遍历的准备成本

`nir::visit` 的表达式读遍历与私有改写遍历现在共用受预算约束的工作栈。
只在节点出栈时扣费并不足够：此前一个很宽的表达式、模式或 contextual 参数就能
在下一次扣费前，把全部子节点引用加入 Vec。现在入栈前检查剩余节点额度，并计入
已有的待访问兄弟节点；已知宽度超额时不枚举，未知宽度的流只读取到首个超额项。
不使用迭代器的巨大上界预留 Vec 容量。节点实际访问仍扣一次，准备检查不重复扣费。

捕获、调用实参、匹配臂及 handler 项还需要独立结构计量：其中某些项没有表达式
子节点，不能依靠过滤后的输出数量限制扫描。位选择和匿名模式标签保留原有计量。
完全排除的函数默认值或 handler 作用域直接选择空切片，不通过 filter 扫描再丢弃。
回调主动剪枝时不展开或计量其后代；成功访问的结构顺序保持不变，不代表执行顺序。

这是遍历准备成本的边界，不是所有内存分配的统一预算。Vec 的容量仍可按常数倍增长；
表达式拥有载荷、回调内的分配、共享闭包体的 copy-on-write 和其他独立 visitor
另有责任。改写入口仍要求调用者管理复制前检查，失败后的私有部分结果必须丢弃；
不能因为工作栈有界就直接发布半次改写。

验证工作目录 `/home/fin/luna`，统一使用
`python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --lib visit:: --no-fail-fast`：新增 7 项通过，退出 0，
  `/tmp/luna-tools-fvm7zv18/result.json`。包含实际迭代次数检查、宽表达式／模式／
  contextual 输入、空捕获／默认实参／handler、剪枝、作用域排除及重试访问顺序。
- `-p nir -p elaboration -p program_verification -p luna --lib
  --test required_input_scopes --test ordinary_callable_types --test ordinary_clause_collectors
  --test runtime_purity --test optional_purity --test call_local_facts --no-fail-fast`：
  598 项通过、2 项既有 ignored，退出 0，`/tmp/luna-tools-g_0b8zho/result.json`；
  包含前述新增用例。两个待定 Type 域用例没有改动，不能据此声称该能力已接通。
- `-p luna --test execution -- --exact <直接调用 elide_proven_checks 的 26 个测试名>`：
  26 项通过，退出 0，`/tmp/luna-tools-u6w9ixim/result.json` 保存完整命令；覆盖
  预算失败回退、共享闭包、别名失效和条件求值，不复跑整个 execution 集。

scoped rustfmt 退出 0，`/tmp/luna-tools-v25d0m2a/result.json`。未运行全 workspace
测试或性能测量；验证仍包含工作区原有 moves/effects.rs 修改，未将该文件纳入提交。

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

### 7.1 纯度分析复用完整包的有界声明构造

纯度的资源子查询原先在 `purity.rs::resource_world_parts` 自行建立 Context、布局和
impl 索引：Context 使用自己的默认额度，布局载荷先 clone，每次 impl 插入又取得
同一初始预算的副本。它与 dispatch 已使用的完整包构造规则重复，也不能把消耗
准确交回本次纯度分析。已有世界缓存只减少重复构造，不能修复第一次构造的计量。

现改为 `Context::from_package_metered` 与 `Index::from_package_metered`。两者沿
同一剩余额度顺序工作；字段与类型载荷在复制前计量，impl 插入共同消费余额，参数域
按同一完整包规则登记。失败保留实际余额，不发布局部索引；成功后才将 Context 与
Index 一起放入本次分析的不可变缓存。观察和清理查询保留各自前提，不将某次选择
证明或运行存储放入该缓存。此处只建立类型查询所需的声明世界，程序纯度、资源、
终止与存活证明仍由各自分析完成，PTC 不因此接管程序状态。

这是已完成包的共同入口，不用于替换 elaboration 中仍在增量建立声明的索引。
没有新建全局缓存；不同包的同号身份仍各自选择实现。既有泛型
默认值的抽象 Copy／表示／清理证据缺口保持未完成，不以具备 trait 前提就放行复制。
其它纯度分析的工作量和临时分配也不因这一构造迁移自动取得完整复杂度上界。

完整计量同时暴露了另一处重复：纯度已读取参数 sort 目录，随后每个资源子分析的
拒绝行为检查又重读整个包。真实 sysroot 的 `rotate_left`／`rotate_right` 每个标量
实例的资源检查消耗约 4,350 步，原先漏计世界构造掩盖了总额度耗尽。仅在单个
资源子分析内部共享目录不足以修复这条路径，必须保留父分析已经取得的声明证据。

`declarations::Metadata` 现在绑定一个不可变 Package，并按需、有界地构造完整
sort 目录。纯度、其资源子分析、资源中的拒绝检查和嵌套结果分析共享目录；失败
不发布，命中和 binder 输出仍计量。目录不包含调用现场、资源状态、选择证明或
行为结果；每个子分析重新建立自己的拒绝与清理证据。公开独立入口仍建立自己的
目录，不按裸 DeclarationId 跨包复用。这次不统一 memory 固定点内部的其它目录，
也未增加纯度预算或改变有限性、Copy、Drop 的证明要求。

对照与验证使用 `/home/fin/luna`，保留原有资源分析工作区修改：

- 仅将资源世界构造函数临时还原为 `a4f72090`，保留新测试，执行
  `python3 -B development/check.py --timeout 900 -- test -p program_verification
  --lib purity::resource_world_tests::resource_world_charges_layout_payload_before_publishing
  -- --exact --nocapture`。`/tmp/luna-tools-_j9vsovh/result.json` 退出 101：含 256
  字段的布局在低额度下仍成功发布，断言明确捕获未计费复制。对照后按 SHA-256
  核验备份并恢复本次生产修改。
- 新测试另外检查同一额度覆盖整批 impl、失败后重新构造、缓存命中仍计费、不同
  世界相同声明编号的正反选择。初轮将“无候选”误写成 Result 错误，修正为核对
  `Selection::NoApplicableCandidate`；不修改既有语义预期。
- 迁移构造而未共享父目录时，`/tmp/luna-tools-c9fhl2pu/result.json` 退出 101，
  模板局部定义组 21 项在真实 sysroot 的 `mix_byte`／`mix_digest` 纯度检查失败。
  临时恢复原构造后的单例对照 `/tmp/luna-tools-qyckx3xc/result.json` 退出 0；
  确认是本次回归。修复目录生命周期后 `/tmp/luna-tools-keq9r9f1/result.json`
  退出 0：模板组 22 项通过、1 项既有忽略，程序验证 121 项与 luna 单元 87 项通过。
- 最终执行 `python3 -B development/check.py --timeout 900 -- test -p
  program_verification --lib -p luna --test runtime_purity --test optional_purity
  --test default_calls --test function_template_local_definitions --test library_protocols
  --test resources --test method_memory_contracts --test comparison_resources --no-fail-fast`。
  `/tmp/luna-tools-m3tvdn1w/result.json` 退出 0，760 项通过、1 项既有忽略。新增
  覆盖包括低预算目录重试、相同编号在不同包中采用不同 sort、畸形目录不得发布，
  以及纯度的实际资源子分析复用已准备目录后仍消费自身预算。

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

### 8.1 资源分析的递归路径与宿主栈

完全内联后的资源检查曾在固定 2 MiB 线程栈上溢出。分阶段插桩确认源码编译、
实例化与 graph 检查均完成，崩溃发生在分析 `entry` 的拥有位置与值来源时，尚未
进入 VM。它在分析深度约 58 时已经耗尽宿主栈，未达到既有深度上限；仅有递归
计数并不能保证检查器正常返回预算诊断。复杂调用、闭包和写回分支的局部状态与
简单绑定共享大型 Rust 函数栈帧，是这条递归路径的额外成本。

`moves/operations.rs` 现在将调用、闭包、指针读写、位置写回与返回分别交给独立
函数；递归分派不再为未选择的复杂分支保留栈空间。`moves/value_origins.rs` 同样
分开 handler 安装、表示转换、构造、块与条件的来源处理。公共 `expr` 和
`evaluate_value_origins` 包装仍负责交付 callable、variant 与 pointer guard 证据；
求值顺序、临时作用域、退出清理、原预算扣费和递归深度计数均保留。表示透明边界
仍先于指针处理，不能因模块拆分改变借用结果的解释。

这是现有递归分析器的实现清理，不是全链路迭代工作机，也没有将深度上限变成
Flurry 的语言限制。更深的有限程序仍可能触发现有分析额度；任意输入和所有辅助
遍历的宿主栈有界性尚未证明。现有内联测试未扩大线程栈，未删减 contracts、默认
实参、提前返回或清理顺序检查。所有临时诊断插桩已撤销。

验证证据（工作目录 `/home/fin/luna`）：

- 修改前单独复现的 `/tmp/luna-tools-0klc0occ/result.json` 与栈帧定位的
  `/tmp/luna-tools-z4050a8h/result.json` 均退出 101，故障位于资源分析。
  可选 GDB 诊断受 ptrace 限制，未取得 backtrace，不作为成功证据。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop
  optional_inline_call_chains_preserve_defaults_contracts_and_owned_returns
  -- --exact --nocapture`：`/tmp/luna-tools-e3vf988s/result.json` 退出 0。
- `python3 -B development/check.py --timeout 900 -- test -p program_verification`：
  `/tmp/luna-tools-36s_rmna/result.json` 退出 0，128 项通过。新增固定 2 MiB 栈用例
  检查嵌套绑定的 Copy 交付、无额外 move、预算耗尽、深度拒绝及新尝试结果一致。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop
  --test closure_local_handlers -- --skip
  scheduled_handler_borrowed_copy_record_rejects_unproved_owner_layouts --skip
  outward_request_cannot_retire_the_storage_a_callback_will_read`：
  `/tmp/luna-tools-8u6n660u/result.json` 退出 0，1,002 项通过，2 项已知基线问题
  显式过滤。前者是准备成功与拒绝预期不符，后者是拒绝诊断断言不符，均未修复；
  [原对照证据](comptime-value-admission.md)保留。未执行全 workspace 回归。

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
