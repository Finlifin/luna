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

### 4.3 契约继承的整批准备与发布

依据 [trait 契约替换](../docs/traits/declarations.md#编译器实现引导)，入口弱化检查
使用 `trait 前提 ⇒ impl 前提`。这属于程序验证，不是 PTC 类型命题；继承阶段还把
接口的 pure、结果条件与入口快照交给完整方法检查，不能据此声称方法体已经满足
所有结果、资源、纯度和效应要求。

原 `contracts::inherit` 逐个方法复制、证明并立即写回包，同时直接推进调用者的
临时绑定编号。后项拒绝会留下前项契约与 pure 标志；此外，前项的弱化证明可能
读取后项尚未继承的入口条件。已复现：某方法的实现前提调用另一方法，后者的
接口入口域为 false，却在继承前表现为无前提调用；旧顺序因此错误接受弱化。

当前分成三个边界：

1. 只读原包，以共享声明域目录准备全部方法草稿和弱化义务；新结果／入口快照
   编号先保存在私有游标。没有复制整个 implementation 列表、trait 或整个包。
2. 在独占 `&mut Package` 借用内安装方法草稿，保存被替换的原函数；弱化分析
   读取完整有效契约。任何拒绝或展开式 panic 都由局部 RAII guard 恢复原函数。
   这个同步私有 overlay 不向外发布 snapshot、候选权限或可执行代码。
3. 所有弱化义务通过后才提交方法和新游标。函数体的完整检查仍走既有后续阶段。

`inherit_with_limits` 将声明目录、owner 实参替换、完整函数／表达式复制预检、
局部绑定 remap 和弱化证明放在同一个预算内；`inherit` 是现有生产入口的默认
额度适配器。owner 重绑定改用带完整域的 DeclarationSubstitution，接口程序契约
沿用完整域替换。定义端尚未完成的输入仍保留抽象 sort，不把这个初始分类误当成
完成实参的准入证据。NF 头不会因这次替换而展开。

Metadata 只借用准备阶段的原包，安装前结束其生命周期；证明阶段重新在有效包
上建立分析视图，避免跨包修改复用旧目录所有权。这里不引入可变 Package 的全局
缓存，也尚未把该额度并入整个 elaboration／invocation 的统一事务预算。NIR 的
原 remap API 保留默认额度，新增 metered 入口供继承共享计量；失败可改变私有
表达式草稿，但不能改变调用方可见的原契约。预算 Unknown 与语义弱化失败分别报告。

工作目录 `/home/fin/luna`，使用
`python3 -B development/check.py --timeout 900 -- test`：

- 原实现运行 `-p program_verification --lib contracts::tests::late_rejection
  -- --nocapture`，`/tmp/luna-tools-xuicmjod/result.json` 退出 101：后项被拒绝时
  调用方游标已由 100 变成 101。修复后同一用例的
  `/tmp/luna-tools-i4kq18em/result.json` 退出 0，原包和游标均保持不变。
- 临时还原旧继承器，保留新顺序反例，以
  `-p program_verification --lib
  contracts::tests::weakening_observes_all_effective_callee_contracts_in_either_method_order
  -- --exact --nocapture` 对照，`/tmp/luna-tools-vgdac3r1/result.json` 退出 101：
  旧实现返回 Ok，错误接受依赖尚未继承前提的证明。随后恢复当前实现；两种方法
  顺序均拒绝，且整包不变。对照中仅保留编译新预算测试所需的未执行兼容入口，
  被测路径直接使用旧 `inherit`。
- 最终执行 `-p nir -p program_verification -p luna --lib
  --test declaration_contracts --test trait_domain_substitution --test trait_phases
  --test trait_purity --test method_memory_contracts --test snapshot_contracts
  --test integer_contracts --test effect_contracts --test staged_aggregate_contracts
  --test library_protocols --no-fail-fast`，
  `/tmp/luna-tools-vb84wljr/result.json` 退出 0，494 项通过，无过滤或忽略。
  包含零／中途／最后阶段预算失败后的完整重试，以及真实 sysroot 下 trait owner
  与方法自身的 `D = Type, item = u8`、`D = bool, item = true` 和运行时前提拒绝。
  源码域用例是交叉路径守护，不把它们称为已复现的旧 Type 拒绝错误。

四个修改的 Rust 文件通过 rustfmt 检查，`/tmp/luna-tools-r5g5n8by/result.json`
退出 0。本阶段保留工作区其他修改；未执行全 workspace 测试，没有改动正式 docs。

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

### 5.1 Handler 借用检查不要求 owner 可复制

正式语义依据是 [Once／Clone／Copy](../docs/resource-management/once-clone-copy.md)
中的交付规则和[指针的复制与访问](../docs/resource-management/pointers.md#复制与访问)：
复制指针不复制目标，也不取得目标的终结责任。借用非 Copy 聚合的标量字段，不要求
整个 owner 具有 Copy；未被访问的指针字段也不能使另一独立字段的借用自动非法。
真正需要核对的是目标存活、初始化、实际读取的交付能力、写入依赖和唯一清理责任。

此前 Drop 回归中的
`scheduled_handler_borrowed_copy_record_rejects_unproved_owner_layouts` 仍期待
`live local Copy scalar`／`pointer-free Copy layout` 两项旧阶段诊断，而且 handler
主体只是 `true`，根本没有使用被取址的局部。历史提交 `179a2f6d` 已允许借用仿射
owner 的标量字段，`7ecfd4ab` 后续支持稳定的指针字段遍历；现有
`moves/effects.rs::stable_handler_pointees` 导入初始化的借用存储，字段读取独立
检查交付能力。不能为恢复这个旧断言而退回对 owner 的一概拒绝。

现在以行为用例替换该断言：四种 owner／字段指针组合各运行 handler 使用和不使用
指针的两条路径；使用路径连续请求两次，验证真实写回、调用次数、无关指针字段
保留以及仿射 owner 恰好一次析构。拒绝例分别检查移动 owner 后访问、通过借用
消耗仿射整体、将 handler 局部地址写入外部指针字段。第三项目前在字段访问摘要
不完整处保守拒绝，**不是已经建立完整逃逸分析的证据**；前两项分别要求初始化和
Copy 交付诊断。newtype 的对应拒绝例也去掉已不存在的旧诊断备选。

本阶段只修正测试契约及其证据，没有改动生产检查器，也没有修改正式 docs。
先保留旧用例、单独执行新增行为用例，确认它们在原实现上通过后才删除旧断言。
第 7.4、7.5 节中的失败和 skip 是当时的真实历史证据；本节负责解释其后续处理。

工作目录 `/home/fin/luna`，检查统一通过
`python3 -B development/check.py --timeout 900 -- test`：

- 替换前单独运行 `-p luna --test drop scheduled_handler_borrowed_storage_
  -- --nocapture`，`/tmp/luna-tools-9yak6qfs/result.json` 退出 0，两项测试覆盖
  八次正例执行和三种拒绝；当时旧断言仍保留，生产代码未改。
- 替换并收紧拒绝诊断后运行 `-p luna --test drop --test effect_memory_transfers
  --test library_iteration --no-fail-fast`，`/tmp/luna-tools-bf_bgdi8/result.json`
  退出 101。Drop 1002 项、效应内存传递 108 项全部通过，没有 skip；迭代器组在
  `find_map_preserves_present_null_and_stops_mapping` 栈溢出并 SIGABRT，不能
  宣称该组通过。该阶段未定位根因；后续处理见[契约分析栈帧](#82-程序契约分析的操作栈帧)。
- `-p luna --test library_iteration -- --exact
  handler_borrows_pointer_bearing_affine_capture_without_copy
  borrowed_pointer_capture_retains_owner_and_storage_restrictions
  borrowed_affine_capture_cleanup_follows_lexical_return_and_loop_exit`：
  `/tmp/luna-tools-ko7x2t_q/result.json` 退出 0，三项借用／清理用例通过。
  这些采用库源码片段，不单独证明公开 sysroot 可用。
- `-p luna --test newtypes installed_effects_do_not_revive_wrapped_pointer_targets
  -- --exact`：`/tmp/luna-tools-waaonc49/result.json` 退出 0，检查死亡目标拒绝
  和对应存活目标仍可执行。
- `-p luna --test sysroot_preludes
  complete_shipped_sysroot_runs_tuple_optional_math_and_iteration -- --exact`：
  `/tmp/luna-tools-n_tcxca2/result.json` 退出 0，通过实际 sysroot、公开协议和
  LLVM 原生执行验证 Tuple、可选方法、math 及普通迭代调用。

两个修改的 Rust 文件通过 rustfmt 检查，`/tmp/luna-tools-79eyzm53/result.json`
退出 0。检查保留工作区原有修改，没有将其纳入本次提交，也未执行全 workspace 测试。

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

### 名义类型的有限存储

检查一个名义类型能否按值存储时，结构字段、枚举载荷与 newtype 表示都使用本次实例的声明实参替换。遍历栈记录待检查字段和等待子项完成的名义类型。若同一个完整类型仍在当前路径上，再次遇到它就是按值递归；指针边界在此结束。一个类型的所有字段通过后才发布其有限存储证明。[表示检查器](../compiler/elaboration/src/representations.rs)

`S<T>` 的字段若是 `S<?T>`，沿途每个实例都不同。检查器消耗调用者额度来展开字段，达到结构或工作量限制时返回可重试的 Budget。另一种情况是 1,024 个不同声明各嵌入下一个声明，末端只含一个标量；额度充足时整条链通过，中间类型的证明也可复用。两种用例均在默认测试线程栈上执行。[回归用例](../compiler/elaboration/tests/representations.rs)

### 借用布局的显式遍历

Pointer 与 Text 的共同叶布局现由
[`memory/borrowed_layout.rs`](../compiler/program_verification/src/memory/borrowed_layout.rs)
提供，原 `memory` API 再导出同一入口。遍历栈保存字段／变体游标及路径恢复位置，
按原声明顺序逐项推进；逻辑路径、物理路径和外到内 guards 只在交付叶时复制。
空变体仍消费 guard 工作，指针 newtype 的物理解包层数也保留。借用的 closure
环境直接观察，已经实例化的字段按所有权移动，不为每个待办子项复制整条路径。

这替换了递归布局 walk，没有改变类型形成服务、路径深度限制或预算策略。
旧实现虽然限制逻辑路径长度，仍会为每层保留宿主栈帧。新增用例在 96 KiB 线程栈
上枚举 127 层名义结构，并验证第 128 层按原限制返回诊断；将同一用例临时切回
`79e3d971` 的旧 walk 后发生 stack overflow／SIGABRT，候选实现则通过。
这份证据只证明布局遍历的栈帧问题，不声称所有类型复制、分析和源码路径都已无递归。

混合文本／指针用例另以独立 storage projection 核对叶域和 guards，逐个不足预算
验证不交付结果，再用足够额度完成同一查询。已有条件载荷、空变体及错误名义实参
仍由共同布局和形成检查处理；没有通过过滤 Text 叶把它伪装成无借用值。
此处是布局基础的实现，仍不提供 backing region 的存活、访问权限或 cleanup 证据，
runtime str 的生产接线继续遵守前述迁移闭包。

验证工作目录 `/home/fin/luna`，均通过
`python3 -B development/check.py --timeout 900 -- test` 执行：

- `-p program_verification --lib memory::borrowed_layout -- --test-threads=1`：
  `/tmp/luna-tools-j8b42oro/result.json` 退出 0，四项通过。
- 临时切回旧 walk、其余工作区保持原样，单独执行上述深层用例：
  `/tmp/luna-tools-h8wzr13y/result.json` 退出 101，stderr 记录 stack overflow
  和 SIGABRT。随后恢复候选文件；未放宽深度、预算或用例的线程栈大小。
- 最终 `-p program_verification -p luna --lib --test text_regions --test text_bytes
  --no-fail-fast -- --test-threads=1`：`/tmp/luna-tools-0_c0_onz/result.json`
  退出 0，四组共 296 项通过，输出锚点为 `test result:`。
- `-p luna --test resources -- --exact
  borrowed_string_views_do_not_gain_pointer_free_layout_from_copy --test-threads=1`：
  `/tmp/luna-tools-fex3zkpk/result.json` 退出 0，原拒绝用例通过。

两个 Rust 文件的 scoped rustfmt 检查退出 0，
`/tmp/luna-tools-a170389s/result.json`；diff 检查通过。未运行全 workspace、native
或性能测量，也未改动作者已有的 moves/effects 修改；结果属于当前工作区验证。

### 6.4 标量存储更新与读取事实的有效期

[writer 范围复现](repros/writer-sibling-range.fl)暴露了一条通用精度缺口：
检查 `sink.count < 8` 后写入 `sink.bytes`，原先 `WritePointer` 会清空整个外部
读取表并 havoc，下一次 `count + 1` 因失去范围而失败。问题不属于类型命题，
不能交给 PTC，也不能要求每个库方法在每次兄弟字段写入后重复 assumes。

本阶段将标量 store 的保留判定集中到 `flow/stores.rs`，共享 State 的写入失效
机制。它同时检查：

1. 目标是 bool/机器整数存储，地址由普通指针上的结构字段投影构成；中途读取
   另一指针、调用或未证明的地址计算不取得这一证据。
2. 地址在右侧求值前取得，其根版本在右侧完成后仍匹配。根是未暴露地址的局部
   指针值；复制指针名称本身与读取该指针指向的内容分开。
3. 读取和写入共享当前地址根，投影在同一 record 的不同字段分开。不同根、不同
   结构种类、相同路径及祖先/后代路径均不能据此证明不相交。
4. 只保留获准的标量观察及其根身份；保留同根的局部指针别名时，同样要求原值
   与别名版本匹配，且别名的地址未暴露。不同指针字段可能指向同一存储，不因
   字段索引不同获得 pointee 分离结论。

其余观察仍失效，旧写入结果的范围传播路径不变。保留算术事实不建立指针的存活、
初始化或读写许可，这些仍由资源/内存检查独立验证。扫描、路径复制和比较仍使用
调用方的程序分析预算；没有新建全局事实缓存或将程序观察写入类型身份。

这不是完整内存足迹替换：普通调用、位写回、未知别名及聚合写入仍使用现有保守
规则。后续可将已核定的 memory summary 映射到同一失效入口，但必须同时传送
地址版本、可能目标集合、字段覆盖和 effect；不能只拿字段路径集合推断不别名。
union/重叠 view 也不能复用普通 record 的字段分离判定。

`scalar_store_facts` 使用真实 sysroot 验证嵌套 record、分支交汇、完整指针及子字段
指针别名；拒绝同字段覆盖、不同根可能别名、整对象与祖先覆盖、右侧改写内容或
地址、指针字段可能别名、已知写调用和 opaque call 后的不安全算术。实际库 writer
测试恢复先写 bytes、再增加 count 的原顺序，不以提前计算结果掩盖这一精度缺口。

本次检查工作目录 `/home/fin/luna`；除原生复现执行外，以下检查均经
`python3 -B development/check.py --timeout 900 --`，没有全量 workspace 测试：

| 参数 | 结果与证据 |
| --- | --- |
| `run -p luna -- build development/repros/writer-sibling-range.fl --entry run --sysroot library --fuel 10000000 --output /tmp/luna-writer-sibling-range` | 退出 0，`/tmp/luna-tools-tfm19dg6/result.json`；随后经 command.py 执行产物，退出 0、`(bool true)`，`/tmp/luna-tools-b_tf_utw/result.json` |
| `test -p program_verification --lib flow:: -- --test-threads=1` | 最终状态转移实现通过 24 项单测，含预算失败/恢复；`/tmp/luna-tools-tn2yj38j/result.json` |
| `test -p luna --test scalar_store_facts --test library_std -- --test-threads=1` | 退出 0，2 + 10 项通过；`/tmp/luna-tools-h2km8zyc/result.json`。writer 原始写入顺序通过 O0/O3 原生检查 |
| `test -p luna --test scalar_store_facts --test integer_contracts --test snapshot_contracts -- --test-threads=1` | 退出 0，2 + 18 + 2 项通过；`/tmp/luna-tools-tbm50a8d/result.json` |
| `test -p luna --test scalar_store_facts -- --test-threads=1 sibling_spelling` | 扩充后的拒绝组退出 0；`/tmp/luna-tools-ga6w9w41/result.json`。另外检查加载指针的潜在别名，以及保留兄弟事实时仍撤销被覆盖字段的旧事实 |

扩充后的正例组还验证两个字段间的不等式关系经过兄弟写入仍能证明安全减法，
不只保存单个字段的区间。命令 `test -p luna --test scalar_store_facts --
--test-threads=1 sibling_stores` 退出 0，`/tmp/luna-tools-l_dq_q53/result.json`。

Rust 改动使用 rustfmt edition 2024、skip_children=true 格式化，最后一轮测试文件
格式化证据 `/tmp/luna-tools-w9_mghwg/result.json`；工作区原有改动没有纳入提交。

#### 指针观察旧拒绝预期的纠正

`external_pointer_observations_track_nested_field_paths` 曾要求写入
`p.right.value` 后不能证明 `p.left.value` 保持不变。它与正式
[视图分离契约](../docs/resource-management/memory-views.md#分离性)冲突：例中两者
是同一个普通结构的独立字段，没有加载另一指针或经过重叠表示，布局足以证明
这次标量写入不覆盖左字段。错误断言不应推动检查器重新对所有写入执行全量失效。

该用例现分别写入外层兄弟 `p.right.value` 与内层兄弟 `p.left.other`，要求原
左字段后置条件成立，并在执行后核对被写字段确实变为 0。负例改为覆盖实际观察的
`p.left.value`；改绑到另一个根、读取错误字段和调用错误子字段的负例仍保留，
并区分 precondition 与 postcondition 诊断，不以任意编译错误充当契约拒绝。

真实 sysroot 的对照在 Links 的两个指针槽中保存同一个字节地址。无“不变”承诺时
执行能观察到左指针读出的值被右指针写入改变；加入该后置条件则拒绝。按值 Links
与 `*Links` 间接加载都如此。这个可执行反例独立说明字段名分离不等于 pointee
分离，不能把结构字段的正例推广到它们。

11 项原指针观察测试现位于 `compiler/luna/tests/pointer_observations.rs`，其中
10 项逐字保留正文；唯一语义预期修正如上。这样可以单独检查读事实、调用／默认值
副作用、析构、别名与字段路径，而不必反复编译大型 execution 测试集。本项未修改
生产验证器，PTC、资源存活与内存权限规则也未改变。

验证目录 `/home/fin/luna`，统一经 `development/check.py --timeout 900 --`：

- 修改前 `test -p luna --test execution -- pointer_observations --test-threads=1`：
  10 项通过、一项旧断言失败，退出 101，`/tmp/luna-tools-wtvmabuk/result.json`。
- 修改后 `test -p luna --test pointer_observations --test scalar_store_facts
  --no-fail-fast -- --test-threads=1`：11 项观察测试及 3 项存储事实测试全部通过，
  退出 0，`/tmp/luna-tools-3l2v7ecw/result.json`；stdout 的 `test result:` 为输出锚点。
- `test -p luna --test execution --no-run`：剩余的大型测试目标编译通过，退出 0，
  `/tmp/luna-tools-6lmtix8j/result.json`。没有运行全量 execution 或 workspace。

三份 Rust 文件格式检查通过，`/tmp/luna-tools-xz8qp8n6/result.json`；抽取前后
核对原 11 个测试名字与 10 份未改正文一致，diff 检查通过。上述结果包含保留的
原有工作区改动；不声称完成一般内存足迹或指针来源分析。

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

### 7.2 程序契约的声明域与失败分类

`flow::Walker` 的普通泛型调用、trait 方法契约和捕获的 comptime 值原先分别读取
整包参数域，并将目录构造的所有错误映射为 `InvalidSignature`。这既把重复调用的
成本乘上无关包大小，也把预算耗尽误判为程序非法。声明域用于确定替换与程序值
的类型；它不证明程序谓词，修复仍属于程序验证层，不能转交 PTC 求内存或程序事实。

现在同一次 flow 分析持有绑定不可变 Package 的 `declarations::Metadata`。它从
`declaration_parameter_domains_checked` 取得完整域目录，sort 从该目录派生，两个
结果分别只在完整构造后发布。各查询仍支付目录访问、binder 输出及所选域载荷的
复制成本；没有预付未知的总额度，也没有将工作转移到调用者预算之外。不同 flow
分析不共享程序状态、成功路径事实或契约证明；不同包的同号声明不共用目录。

结构化失败保持到 flow 的接收边界：`Budget` 成为 `Unknown(BudgetExhausted)`，
畸形域或不一致的签名仍拒绝为 `InvalidSignature`，不成立的程序前提仍是
`MissingPreconditionProof`。耗尽不发表否定结论，重新分析有足够额度时可正常完成。
这次只覆盖声明目录及其消费；其它契约替换、内存固定点和复制路径的复杂度仍须
按各自边界审查，不能据目录共享宣称整个程序验证已经线性化。

对照在 `/home/fin/luna` 执行
`python3 -B development/check.py --timeout 900 -- test -p program_verification
--lib flow::declaration_tests -- --nocapture`。仅增加用例、未改生产路径时，
`/tmp/luna-tools-o__7i_d_/result.json` 退出 101：两条耗尽路径实际返回
`InvalidSignature`；八次相同泛型契约调用加入 2,048 条无关别名后，检查成本从
380 增至 18,812 步。迁移后的针对性检查使用同一输入和额度，不放宽证明要求。

最终检查执行 `python3 -B development/check.py --timeout 900 -- test -p
program_verification --lib -p luna --test integer_contracts --test snapshot_contracts
--test staged_aggregate_contracts --test effect_contracts --test method_memory_contracts
--test predicate_calls --test runtime_purity --test function_template_local_definitions
--no-fail-fast`。`/tmp/luna-tools-kydv1ihv/result.json` 退出 0，375 项通过、2 项
既有忽略。三条消费路径分别覆盖低预算与恢复；另验同号不同域、错误前提及热目录
下的大载荷复制计量。重复调用测试限制无关目录增加的额外成本，覆盖真实分析入口，
不以只命中 Metadata 的单元测试代替接线验证。

### 7.3 契约替换消费完成后的声明域

共享目录不等于正确替换。flow 的直接调用还曾把目录中的定义端 sort 取出，分别
对 precondition、postcondition 使用固定 sort 替换；有开放参数时先试闭合 scope，
失败后再换调用者 scope。非泛型契约直接 clone，两条路径都没有在复制前计量。
这与 trait 方法已经采用的 `DeclarationSubstitution` 契约不同。

以下程序通过真实 sysroot 暴露了语义后果：`D = Type` 完成后，`item` 的实际域就是
Type，不能仍要求它具有定义时的 ComptimeValue sort。

```flurry
fn admits(ok: bool) -> result: bool
where D, item: D, requires ok, ensures result
{
    ok
}

fn run() -> bool {
    admits<Type, u8>(true) and admits<bool, true>(true)
}
```

直接调用现同样用完整声明域建立 `DeclarationSubstitution::bounded`，在调用者的
明确输入 frame 中一次完成 scope 与 sort 检查，再通过
`instantiate_expression_bounded` 准备两组契约。值参数成为 Type 的许可来自它的
实际域，不能从实参长得像 Type 反推；`D = bool, item = u8` 仍非法。完整成员资格、
形成义务与 capability 准入继续由调用边界负责，结构替换不代替这些检查。
NF 头不展开，也不为 Type-valued 参数增加专用替换分支。

契约树、隐藏类型操作数及替换扩张在复制前计量。未执行分支无需执行，但它仍被
复制进私有契约树，不能因此免计复制成本。准备失败不输出半组契约或发布程序事实；
超预算保留 Unknown，开放 binder 只可引用实际调用者 frame，不以 scope 检查失败
作为切换环境的信号。这是程序验证消费者迁移，不是一般前端 continuation 已完成。

工作目录 `/home/fin/luna`：新增正例后、生产路径未改时，执行
`python3 -B development/check.py --timeout 900 -- test -p program_verification
--lib -p luna --test declaration_contracts --no-fail-fast`，
`/tmp/luna-tools-3siluwjo/result.json` 退出 101。真实 sysroot 与直接 NIR 用例均被
`invalid-signature` 拒绝。恢复旧生产路径对照复制用例，
`/tmp/luna-tools-6so8_40h/result.json` 退出 101：256 步仍成功准备包含两份 512
字段类型载荷的契约，明确捕获了未计费复制；随后恢复本次实现。

迁移后，以 `python3 -B development/check.py --timeout 900 -- test -p
program_verification --lib -p luna --test declaration_contracts --test integer_contracts
--test staged_aggregate_contracts --test snapshot_contracts --test effect_contracts
--test method_memory_contracts --test runtime_purity --test function_template_local_definitions
--no-fail-fast` 验证，`/tmp/luna-tools-1hnu86xe/result.json` 退出 0，373 项通过、
1 项既有忽略。补充调用者 binder 与未执行分支复制反例后，仅重跑程序验证单元及
`declaration_contracts`（同一入口的 `--lib -p luna --test declaration_contracts
--no-fail-fast`），`/tmp/luna-tools-m4zet26v/result.json` 退出 0，220 项通过。
后者包含真实 sysroot 的显式 Type 值与跨函数转发，并保留错误运行时前提的拒绝。

同一规则现延伸到 P3／P4 的函数特化与拒绝行为扫描。纯度采用
`Function::instantiate_declaration_parameters`，同时预检整个函数的元数据、契约、
默认值和主体扩张；原来分别计量部分表达式、再按固定 sort 复制函数的路径已删除。
因此契约接受的 `D = Type, item = u8` 不会仅因进入纯度消费者而再次被拒绝。

拒绝扫描的工作项需要比一次借用活得更久。`DeclarationSubstitution::into_owned`
在复制前计量借用的实参和 frame，移动已拥有的数据，保留原 scope 和完成域的
结构检查。工作项共享该只读替换上下文；逐项展开仍由共同的 bounded 接口计量。
它只拥有替换语法和 frame 形状，不是 VM 存储、世界租约、值成员资格或行为证明，
不能据此跨包复用选择结果。旧扫描器的固定 sort 映射与最大实参成本乘树大小的
独立替换估算已退役。失败仍不写入成功行为缓存。

此前新增真实 sysroot 的 `pure fn answer() -> bool where D, item: D { true }`
及 `pure fn run() -> bool { answer<Type, u8>() }`，执行
`python3 -B development/check.py --timeout 900 -- test -p luna --test declaration_contracts
-- --nocapture`；`/tmp/luna-tools-f8f6w8k8/result.json` 退出 101，实际在 `run` 的
P4 检查失败。同轮高阶输入／结果与 Drop 正例通过，故不把它们称为本次修复的旧
失败，也不据此认定全部消费者已迁移。源码调用已有 residual body 时可能跳过
声明侧替换，因此还需独立覆盖未特化函数的消费者。

最终执行 `python3 -B development/check.py --timeout 900 -- test -p nir
-p program_verification -p luna --lib --test declaration_contracts --test runtime_purity
--test optional_purity --test default_calls --test function_template_local_definitions
--test resources --test library_protocols --no-fail-fast`，
`/tmp/luna-tools-ss__0lqr/result.json` 退出 0，811 项通过、1 项既有忽略。新增单元
验证 owned 替换在原输入释放后仍保留 frame 与不透明头，低预算不复制大载荷，
错误域不污染拒绝行为缓存，失败后的新合法查询仍能成功。


资源分析中的四处声明消费者现沿用这一协议：callable 输入和内存摘要准备以
完成域检查并有界复制签名；返回 callable 与清理调用图仅在存在声明输入时以共同
Function API 预检完整函数后特化，没有输入的不可变函数直接借用。消费者仍计量
其分析遍历，不再先 clone 整个函数或逐份套用固定 sort 替换。递归结果
键的实参改为先计量再复制，清理候选参数类型的复制也单独计量。完整函数替换
包含 requirements、capabilities、默认值和普通参数 frame，但它不证明这些义务；
形成、选择与发布仍由原边界负责，NF 头保持不透明。

所有生产调用者迁走后，删除 Metadata 的固定 sort binder 构造器。定义端抽象
检查仍可查询 sort 目录；依赖域的实例化必须使用完整域。内存摘要独立入口目前
仍自行读取包域目录，未宣称已经共享整个分析事务。Drop 消费者的后续迁移见
下一节；契约继承后续迁移见第 4.3 节，泛型默认值的 P3／Copy 证据缺口仍未解决。

工作目录 `/home/fin/luna`，执行
`python3 -B development/check.py --timeout 900 -- test -p program_verification
--lib moves::declaration_tests -- --nocapture`，原生产实现的
`/tmp/luna-tools-s8mvdybj/result.json` 退出 101：三个合法的 `D = Type, item = u8`
用例分别在 callable 输入、内存签名和返回 callable 分析失败。迁移后同组
`/tmp/luna-tools-vme7hn4r/result.json` 退出 0。这些是直接 NIR 消费者证据，不能
替代真实源码／sysroot 验收。

额外预算对照仅临时恢复旧 callable 输入检查：同一入口加
`moves::declaration_tests::callable_input_charges_expanded_signature_before_copying`，
`/tmp/luna-tools-hlpnl_iq/result.json` 退出 101；旧实现以 1000 步接受了将 32 项
类型载荷复制到 64 个签名参数的输出，违反复制预算断言。随后恢复迁移实现，
低预算拒绝、高预算重试成功；错误域与失败后的有效查询也独立覆盖。


首次扩大回归 `/tmp/luna-tools-jdhtvr64/result.json` 退出 101，只有
`runtime_purity::recursive_nested_closure_factories_preserve_each_capture_layer`
新失败。定位到 caller 剩余 36782 步时，递归闭包工厂的内存摘要耗尽预算；完整
复制计量暴露了无需替换的函数仍被反复复制。准备入口现返回借用或拥有的函数：
空声明输入借用原已检查函数，非空输入仍完整计量并替换，错误实参数量不走借用。
没有扩大 P4 预算、跳过资源遍历或把错误域作为借用条件。恢复后执行
`python3 -B development/check.py --timeout 900 -- test -p program_verification
-p luna --lib --test runtime_purity --no-fail-fast`，
`/tmp/luna-tools-q63ncb0_/result.json` 退出 0，288 项通过。临时诊断打印已删除。


最终在恢复实现与移除诊断打印后执行
`python3 -B development/check.py --timeout 900 -- test -p program_verification
-p luna --lib --test declaration_contracts --test resources --test runtime_purity
--test optional_purity --test default_calls --test function_template_local_definitions
--test library_protocols --test method_memory_contracts --test comparison_resources
--no-fail-fast`，`/tmp/luna-tools-yv94_tz9/result.json` 退出 0，778 项通过、1 项
既有忽略。包含真实 sysroot 的声明契约、callable 输入／返回和 Drop 正例；它们
与直接消费者反例共同支撑本次迁移，不表示整个资源或程序验证系统已经完成。
修改的三个 Rust 文件通过 rustfmt 检查，证据
`/tmp/luna-tools-99ffs6o7/result.json`；工作区原有修改未纳入本次提交。


### 7.4 Drop 选择与捕获契约准备

析构选择和清理时的访问核对都消费同一份声明域，不能在前者接受
`D = Type, item = bool`，再由后者的固定 sort 替换拒绝。显式实现的 method 输入
来自所选 implementation 的实参；trait 默认方法接收整个 Self。两种计划仍各自
保留，不以重选实现、展开 NF 头或改变参数身份来消除差异。

Drop 选择现以有界声明替换准备签名、类型 requirements 和 capabilities；程序
前后条件保留普通 receiver frame，仅交给程序验证层，不进入 PTC。接口 Self
替换也计量完整 MethodContract 的相关元数据，并替换其能力需求；接口与所选
主体的准备共享本次 preparation fuel。此前直接 clone 整份 MethodContract、
先 clone 函数前后条件再进行第二次替换的路径均退役。

`CallableProgram::instantiate_contract` 从已借用的函数／方法取程序契约，与
拥有的 ProgramContract 共用同一复制实现。只复制前后条件和入口快照，不复制
函数体或默认值，更不据此证明这些程序的纯度、访问权限或安全性。低预算不返回
半份契约，失败后能从原不可变输入重试；大主体和大默认值不影响只请求契约的
复制成本，但请求的大契约本身必须完整计量。

清理瞬间仍核对所捕获的方法和程序契约，不重新选择析构方法。implementation
subject 的重建、函数契约重建和名义字段投影都使用完整域的 bounded 替换。
旧 `memory_binders` 已失去全部生产调用者并删除。此迁移没有统一整个 selection
查询的预算：类型查询仍接收独立 Budget，完整事务的预算与证据所有权还有后续
工作。契约继承的旧 owner 替换与发布路径已另按第 4.3 节迁移。

工作目录 `/home/fin/luna`，以下命令均使用
`python3 -B development/check.py --timeout 900 -- test`：

- 新的源码声明经真实 frontend 编译后，以公开 Drop 入口选择和检查；
  `-p luna --test drop dependent_domains_survive_drop_selection_and_captured_access
  -- --nocapture` 在原实现 `/tmp/luna-tools-bt9ebib8/result.json` 退出 101，
  报 `expected comptime-value, got type`。迁移后
  `/tmp/luna-tools-a02uot7c/result.json` 退出 0。覆盖显式／默认析构方法、Type
  与 bool 值输入、低预算后重试以及捕获的程序契约被改写后的拒绝。
- 仅临时恢复旧 Drop selector，以
  `-p luna --test drop selected_drop_bounds_program_contract_copy_before_publication
  -- --nocapture` 做预算对照，`/tmp/luna-tools-7k2cbii0/result.json` 退出 101：
  1000 步仍发布包含 2000 个后置条件的选择。随后恢复本次代码，低预算拒绝且
  充足预算重新选择后完整保留全部条件。
- `-p nir -p program_verification -p luna --lib --test drop
  --test declaration_contracts --no-fail-fast` 的
  `/tmp/luna-tools-baehal1i/result.json` 退出 101；新增用例通过，完整 Drop 组
  有一项既有 handler 布局拒绝断言失败。将本次五个生产文件全部恢复为
  `2ebe880d`，单独执行
  `-p luna --test drop scheduled_handler_borrowed_copy_record_rejects_unproved_owner_layouts
  -- --nocapture`，`/tmp/luna-tools-rs5eabcx/result.json` 同样退出 101，确认是
  本阶段之前的基线行为。未改该测试预期；尚待按实际捕获与访问规则判断是
  旧阶段限制已经过时还是遗漏检查，不能仅凭断言失败补定语言规则。
- 恢复本次实现后，以
  `-p nir -p program_verification -p luna --lib --test drop
  --test declaration_contracts --test runtime_purity --test resources
  --test method_memory_contracts --test library_protocols --no-fail-fast
  -- --skip scheduled_handler_borrowed_copy_record_rejects_unproved_owner_layouts`
  验证，`/tmp/luna-tools-2hvmqs5o/result.json` 退出 0，1758 项通过，单独过滤
  上述已复现的基线断言。不能将此结果称为完整 Drop 组无失败；真实 sysroot 的
  声明契约与公开内置包检查已包含在本轮中。


随后将契约视图单元用例的默认值改为一个合法槽上的大表达式，单独重跑
`-p nir --lib contracts::program_view::tests::contract_specialization_does_not_copy_unrequested_body_or_defaults
-- --nocapture`，`/tmp/luna-tools-liaupxh6/result.json` 退出 0。另执行
`-p nir --test scoped_contracts --test contract_alignment --test substitution_input_observation
--no-fail-fast`，`/tmp/luna-tools-3kbuawk6/result.json` 退出 0，13 项通过。
六个修改的 Rust 文件通过 rustfmt 检查，证据
`/tmp/luna-tools-7w13wiwl/result.json`。这些都是保留原有工作区修改的验证，
不宣称为隔离的纯 HEAD 检查。

### 7.5 捕获的 Drop 查询证据与前提作用域

此前 SelectedDrop 仅保留公开可改写的 TypeResolution，丢失完整 capability
Resolution 中的前提作用域与条件支持。访问检查只比较 index 身份／版本，再
重建方法描述；同一索引中另一有效实例的 subject、method、contract 等字段
可以与原选择结果混合，检查仍然接受。索引未变也不意味着调用者的前提未变。

当前以私有 `SelectionEvidence` 共享持有完整 query：protocol、subject、类型
选择、qualification、前提 scope 及 condition support。只在选择入口创建，克隆
共享同一个不可变对象；读者可借用 TypeResolution 和 qualification，不能单独
替换捕获的证明或清空能力需求。正向选择与 NoCustomDrop 都持有此证据，避免
只保护“有析构方法”而丢掉“没有自定义析构方法”的前提。

直接访问入口将公开描述中的 protocol、subject、implementation 与实参与原
查询及 Applicable 证据对应，比较前计量类型载荷；继续独立核对源码方法、契约、
当前构造状态和内存访问。清理计划在访问已获取责任时也检查查询作用域。条件
支持使用既有 replay 路径，新增 metered 入口将成功和失败消耗交回同一 fuel。
没有重新运行析构候选选择，没有把程序前后条件送进 PTC，也没有借此取得运行时
handler／allocator 等环境。新证据 Display 为 S-expression，包含查询和作用域。

这是一条 Drop 消费者的证据所有权迁移，不是所有 TypeResolution 都已封装，
也不是整个 Package／函数体已拥有可跨编辑恢复的 snapshot。现有方法、契约、
布局检查仍承担自己的职责；整体查询预算、完整代码来源与环境获取继续按前文
协议推进，不能从 retained query 推导它们已经完成。

工作目录 `/home/fin/luna`，统一通过
`python3 -B development/check.py --timeout 900 -- test` 执行：

- `-p luna --test drop captured_drop_selection_stays_bound_to_its_instance_and_premise_scope
  -- --nocapture` 在旧实现 `/tmp/luna-tools-65eyene0/result.json` 退出 101：
  接受结果为 `[true, true, true]`，分别对应换实参、换实现和换前提作用域。
  修复后 `/tmp/luna-tools-txn4c3uf/result.json` 退出 0，三项均拒绝，原查询在
  原作用域中仍可重试成功。
- 增加 NoCustomDrop 作用域检查和嵌套条件支持的共享预算／耗尽恢复覆盖后，
  执行 `-p type_resolution -p program_verification -p luna --lib --test capabilities
  --test drop --test declaration_contracts --test runtime_purity --test resources
  --test library_protocols --no-fail-fast
  -- --skip scheduled_handler_borrowed_copy_record_rejects_unproved_owner_layouts`，
  `/tmp/luna-tools-ah93fw1f/result.json` 退出 0，1769 项通过。只过滤前一节已在
  本次修改前复现的基线断言，其预期仍未修改；不能宣称完整 Drop 组无失败。
- 补充证据 dump 的嵌套结构断言后，仅重跑首项实例／作用域用例，
  `/tmp/luna-tools-cjris69m/result.json` 退出 0。七个修改的 Rust 文件通过
  rustfmt 检查，证据 `/tmp/luna-tools-bwxdwu89/result.json`。验证保留了原有
  工作区修改，未声称隔离的纯 HEAD 验收。

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
  原对照证据可从提交 `3c6e61c3` 的 `development/comptime-value-admission.md` 取得。未执行全 workspace 回归。

### 8.2 程序契约分析的操作栈帧

`find_map_preserves_present_null_and_stops_mapping` 的另一条溢出发生在 cleanup
准备的最终 residual contract 检查中。源码检查、inline 展开、实例化和 moves 分析
已经完成，尚未进入参考 VM。进程内回溯指向 flow 的 `inner → general_expression
→ child`，不是迭代器运行时自动机无限递归。保留的无优化、无 debuginfo 二进制中，
`general_expression` 的 prologue 分配 `0x25898`（153752）字节栈空间；每层简单
操作也携带未选中的指针写回、聚合与类型形成分支的临时状态。

`flow/operations.rs` 现按操作职责隔离普通值操作、指针读取、写回、透明边界、
类型形成和聚合。总分派仅选择路径；所有子表达式仍回到同一 `child`，保留预算、
失败 source、状态失效及证据完成钩子。操作主体和次序逐项核对为原实现；没有绕过
程序契约、增加分析额度或扩大执行线程栈。此次构建中总分派栈帧为 56 字节，六类
操作分别约 6–36 KiB，实测 prologue 保存在 `/tmp/luna-flow-stack-after.txt`。
这些是该构建的诊断测量，不是跨平台 ABI 或所有递归路径的空间上界。

新增固定 2 MiB 栈的独立回归：40 层透明边界中的指针写入和随后读取仍支持真实
后置条件证明；相反后置条件按原 source 拒绝；20 步预算返回 BudgetExhausted，
重新分析可以成功。原有 find_map 用例继续验证 present-null 与 absent 的区别、
mapper 次数、展开／未展开参考执行及 LLVM O0/O3。此阶段属于现有检查器实现
清理，尚未把全部程序验证迁入显式 continuation；其他递归路径的安全性仍需审计。

工作目录 `/home/fin/luna`，Cargo 通过
`python3 -B development/check.py --timeout 900 -- test`：

- 原实现单独运行 `-p luna --test library_iteration
  find_map_preserves_present_null_and_stops_mapping -- --exact --nocapture`，
  `/tmp/luna-tools-x6t870p3/result.json` 退出 101，SIGABRT。可选 GDB 因 ptrace
  不可用，`/tmp/luna-tools-yciol9i9/result.json` 标记 unavailable；后续进程内
  定位回溯在 `/tmp/luna-tools-ad70wt4n/stderr.log`，临时插桩已全部撤除。
- `-p program_verification -p luna --lib --test library_iteration
  --test integer_contracts --test effect_contracts --test method_memory_contracts
  --test snapshot_contracts --test staged_aggregate_contracts --test error_result_flow
  --test pure_bitvec --no-fail-fast`，`/tmp/luna-tools-oyqzrgvn/result.json`
  退出 101。除迭代器组外 340 项通过，包括独立栈回归；迭代器组仍有下述两项
  问题，因进程中止不能计为该组完整验收。
- 最终 `-p luna --test library_iteration --test sysroot_preludes -- --exact
  find_map_preserves_present_null_and_stops_mapping
  complete_shipped_sysroot_runs_tuple_optional_math_and_iteration`，
  `/tmp/luna-tools-gpyvrfg3/result.json` 退出 0，两项通过；后者使用真实六包
  sysroot 和原生执行，未改写公开库源码。rustfmt 检查退出 0，
  `/tmp/luna-tools-x3r_hddl/result.json`。

完整迭代器组剩余已观察问题：`counted_adapters_retire_skipped_delivered_and_remaining_items_once`
仍栈溢出；`ordinary_for_prefers_direct_iteration_and_converts_generic_inputs`
在源码检查处报 `Type domain equality: UnvalidatedModel`。用本次生产修改之前
保留的二进制逐项复现，`/tmp/luna-tools-fqinaqxb/result.json` 为 SIGABRT，
`/tmp/luna-tools-17ztwbja/result.json` 为相同 UnvalidatedModel。二者尚未修复；
没有删除用例或改变语义预期，也不将本节交付称为所有宿主栈问题已经解决。

### 8.3 词法捕获、默认实参与阶段检查

同一段已检查的代码可以作为闭包体、quote 的模式或声明实参进入后续阶段。
自由绑定取决于读取位置。例如闭包的捕获初始化器读取外层绑定，进入主体后只能读取
参数和显式捕获。普通调用的显式实参读取调用者环境，默认实参只读取已供应的参数。
对声明实参，准备语句和供应步骤按同一张已检查的顺序表执行，供应完成才公开该槽绑定。
这些边界由 [NIR 捕获分析](../compiler/nir/src/free_bindings/walk.rs) 统一处理。

分析把表达式、模式、上下文实参和退出作用域的动作放在同一个显式栈上。块结束时撤销
该块引入的绑定，分支汇合时只留下每条成功路径都可见的绑定。切入闭包体或默认实参
时连同自由变量集合一起切换环境，因此内层缺失的捕获保留其错误位置所属的作用域。
普通结构操作复用 [NIR 子节点枚举](../compiler/nir/src/visit/children.rs)，直接向当前栈追加子项。
扩展前按剩余预算限制枚举宽度，词法遍历为已排入栈的兄弟节点记账。符号捕获的依赖图
在每个读取位置展开，遇到当前路径上的活动节点报告循环。

原回调在表达式、模式和声明实参之间递归重入。早先分离结构分派的栈帧后，仍会因
128 层阈值拒绝合法代码。现在这条路径的限制是调用者提供的工作预算。阶段检查分别保留
预算不足和非法作用域两种失败，补足预算后可重新检查同一个不可变输入。NIR 复制、
类型准入和 VM 执行各自的深度或预算契约仍须独立验证。

[词法回归](../compiler/nir/tests/free_bindings.rs) 在 256 KiB 线程栈上，检查含 384 层
模式、块与上下文实参的输入、跨分支的可见性、漏报捕获及失败后重试。
[阶段回归](../compiler/nir/tests/phases.rs) 通过真实包检查默认实参，分别检验合法深层输入、
预算不足和不合法的闭包环境。

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
