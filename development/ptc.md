# PTC 与类型求解的实现边界

作者确认：完整 CDCL 的职责包含为 unification 提供 model searching 托底，
优先让 `Type` sort 的变量能够通过约束求解；语言整体偏向显式，不鼓励扩大隐式推导。
正式协议见 [PTC 组合、证明与诊断](../docs/type/propositional-constraints/solver-protocol.md)。

## 当前接线

- `type_resolution::obligations::Engine` 拥有候选局部 metavariable、等待义务和提交边界。
  直接匹配提出赋值，完整原始关系仍作为义务检查。
- PTC 已接入索引布尔骨架、文字传播、theory 冲突学习和按冲突前提回跳。
  搜索使用显式决策帧保存布尔传播、学习子句传播和真假分支的后续动作；所有出口恢复
  进入查询时的赋值。逻辑深度预算仍保留，搜索帧尚未作为可挂起任务暴露给 elaboration。
- 已解析的布尔分支证明可形成查询局部学习子句。引理显式保留前提及共享证明；
  检查器在这些前提下重放证明，不信任学习缓存。缓存清理不影响已交付证明。
- `Witness` 目前用于验证不蕴含的源语义反例，类型部分通过闭合类型重建取得。
  它不是可直接写入 inference store 的 substitution。

## 后续解提取接口必须保持的区别

1. 满足约束的模型、可表达的类型 substitution、可提交的候选解是不同产物。
   搜索找到模型不自动授权任意选择类型。
2. 类型解可以保留符号关系与未解参数，不能把闭合反例生成器的限制变成语言规则。
3. 只求解调用方明确交付的变量；显式输入、作用域、sort、occurs check、normal-form
   头部不展开以及候选事务隔离仍须保持。
4. 候选解必须回到原环境检查原始关系和义务。不能把待证明的 requires 当成无条件事实，
   再以此宣布候选成立。
5. Unknown、未决计算与多种无选择依据的解不能冒充已解，也不能通过增大推导范围消除。

学习子句已使用双文字监视，赋值变化和回退会重新安排相关子句检查；预算中断保留未完成
事件，清理子句表同时清理监视索引。目前仍通过赋值快照同步变化，尚未接入显式 trail 事件。

搜索现记录决策层。在冲突理由按逆赋值顺序解析时，当前层首次只剩一个必要赋值就保留
first-UIP 子句；传播得到的汇合点不会等到归约为原决策文字才被学习。确认该子句仍在
学习表中后，直接撤销高于 asserting level 的赋值和决策帧并重启传播。入口与低层事实
保留；预算不足或引理未被保留时继续使用已有证明回退路径。

根布尔公式已有线性 Tseitin 子句表示，辅助变量仅属于布尔层，不进入 theory atom
或源语言 `Type` 变量空间。搜索重建源语义模型后解释所有 theory atoms，再确定性扩展
辅助变量并检查子句；独立 witness 检查器仍检查原公式。编译、扩展及子句检查计入预算。
根子句现在按查询惰性编译并复用。索引布尔骨架无法直接传播时，根子句通过 occurrence
事件队列传播辅助变量，向现有搜索帧交付 theory unit 或冲突。理由携带根子句编号轨迹；
独立检查器从原公式重新生成子句，逐步验证 unit 推导直到出现 falsified clause，不信任
缓存的辅助赋值。重复文字按同一未知量处理。编码、事件与轨迹检查均受预算约束。

根子句冲突与 unit 的反证理由会沿传播依赖裁剪，同时删除无关 theory 前提与无关轨迹
步骤；得到的是该传播推导的依赖闭包，不保证全局最小不可满足核。裁剪预算不足时保留
原证明和保守支持，不能只裁剪 support 却保留仍依赖旧前提的轨迹。独立检查器仍重放结果。
根子句与学习子句尚未统一到持久 trail／双文字监视；当前每次根子句传播重建辅助赋值。

搜索的等价类闭包已使用可撤销 union-by-size 与查询局部前提检查点。单调增加前提时
保留已有闭包；撤销或翻转前提时退回兼容层，失败或预算中断撤销本次合并。模型快照
与活动状态分离。证明检查和冲突最小化始终从新等价类开始，不复用搜索缓存。
普通结构与获授权 normal-form 同余的闭包复用会追踪代表元变化，沿子项到父项依赖
标记受影响节点，仅枚举涉及这些节点的候选对。小型目录中索引构建成本高于全对扫描时
保留直接扫描。错误结果与效应行暂保留全对扫描，其标签关系还依赖显式不等式，不能
仅凭子项代表元不变跳过检查。trait、kind 与整数一致性仍重新检查，尚非完整增量 theory solver。

当前尚未实现 Type 解提取及其生产调用接线；也未完成统一子句 trail、
其余 theory 增量状态和跨查询 activation guard。现有查询局部学习不代表完整 CDCL 已交付。

结构匹配现可使用区域内已成立的命题恢复符号类型的结构视图：直接分解没有提出输入时，
从已知公式中收集候选形状，并由 PTC 证明实际类型与该形状相等后才交给结构匹配。
析取/否定中的出现不等于事实；歧义保持未决，原始完整关系仍作为候选提交前的义务。
该路径保留符号输入，不从 `Witness` 取闭合类型，也不把 requires 提升为假设。
结构子项也使用该恢复路径；外层已产生部分输入，不会阻止内部别名继续匹配。候选形状
的筛选关闭递归别名搜索，以免别名环重复展开。进入新的局部 binder 后暂不使用区域
别名事实，避免把同索引的局部变量误认作外层绑定。这仍不是完整的 existential Type
模型解提取；binder 下的上下文重定位和嵌套未决计算还需要后续求解接口。

实现候选的 eligibility、结构排除和输入合一现在共享原有 head 预算分区，
按实际消耗将余额传给后续阶段。此前先消耗单个查询额度，再乘以关系数分配
合一预算，可能把本已预留的额度全部变成零。结构视图搜索也显式扣除返回的
实际消耗，不能丢弃计量结果。候选总上限、域检查与 requires 的预留分区不变。
`f16 | bf16` 符号主体排除错误结果候选的回归使用 4600 steps；证明需在原上下文
独立回放，去掉前提则拒绝。低预算失败不污染后续候选查询。

本阶段验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p type_resolution`：全部通过，`/tmp/luna-tools-8olfmz64/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test builder_invocations contextual_data_consumer_publishes_structs_through_the_complete_sysroot`：通过，`/tmp/luna-tools-hwaapn8f/result.json`。完整库用例约 42 秒，尚未进行端到端性能优化。


### 根子句双文字监视

根 Tseitin 子句的传播现使用两个未定文字作为监视点，已满足子句只需监视使它成立的
文字。辅助变量赋值仅唤醒仍监视它的子句；迁移后残留的桶条目在唤醒前检查归属。
扫描游标沿子句循环移动，替换监视点时不反复从已否定的前缀开始。删除原来的完整
occurrence 表，避免保存不再使用的全出现索引。

传播产生的证书仍只包含根子句编号。独立 proof replay 与支持集裁剪继续使用完整子句
扫描，不信任监视点、扫描游标或传播时的辅助赋值。预算耗尽丢弃本次局部传播状态，
下次查询、翻转或撤回赋值均从当前输入重新开始。

这一实现只推进 CDCL 布尔层。监视状态仍在单次根传播内建立，并未与学习子句组成
统一持久 trail；跨回跳辅助赋值维护、Type 模型解提取及提交到 inference 的接口仍缺失。
不能据此宣称完整 CDCL(T) 或模型驱动 unification 已经完成。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`：exit 0；`/tmp/luna-tools-zas71iqi/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p ptc boolean::cnf::propagation`：新增监视迁移检查后 exit 0；`/tmp/luna-tools-1teig_wc/result.json`。
- 小公式检查扩展至三个原子的全部 27 种部分赋值，对全部 8 种完整赋值检查冲突／unit
  语义；每个产生的证明用独立扫描器重放。另覆盖重复文字、宽子句、过期监视记录、
  预算中断后恢复与重新赋值。格式化和 `git diff --check` 通过。


### 显式 Type 输入的模型搜索接口

`Context::search_type_inputs` 接收明确列出的 rigid Type 输入与待求解约束，共享调用方
预算。它在私有问题上下文中搜索满足模型，再逐一证明每个请求输入等于模型给出的值。
仅满足约束不能得到 Unique；若存在不同赋值，返回 Ambiguous 及两个 witness。不可满足
返回 False 的证明；模型重建不支持、预算不足和非法输入分别保留为 Unknown／IllFormed。
未请求的环境变量可以出现在 witness 中，但不会作为输入 substitution 导出。

`type_input_context` 重建证据的精确上下文，供独立 `verify_witness`／`verify_proof` 回放。
Unique 中每个 binding 的证明目标必须是该变量与该值的等式；存在性 witness 的目标是
False（它是该目标的反例）。反例生成器里的任意默认值不能通过这个确定性检查。
重复请求被拒绝；即使请求变量未出现在约束中，也登记反身等式以明确包含在模型中。
返回结果提供 S-expression dump。

这个接口是明确约束问题的求解接口，不授权调用方将函数 requires 或候选待证义务
提升为区域事实。尚未接入 elaboration 的候选事务；后续接线必须先做直接 unification，
只在明确交付的待解输入上使用搜索，随后在原环境重新检查候选和全部剩余义务。
当前 witness 仍只重建已有的闭合结构类型片段，不能替代符号 substitution；opaque
normal-form 的不支持仍返回 Unknown，也不能由头部相等反解参数。完整 Type 解提取
和统一持久 CDCL trail 仍未完成。

自由等价类的模型值现在避开输入中已出现的具体类型及其他自由类，修正了如
`T != *bool` 时默认仍选 `*bool` 而误退到 Unknown 的情况。重建及候选分配计入预算。
trait atom 没有裸模型解释时，在分配类型 witness 前返回未决，避免浪费后续
implementation-world 求解需要的额度；没有扩大原有预算。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`：exit 0；`/tmp/luna-tools-td3evoor/result.json`。
- 新增六项 Type-input 测试，覆盖析取排除、多变量、独立证据重放、歧义／无约束、
  矛盾、预算恢复、作用域、环境变量隔离、opaque head 非单射及具体类型碰撞。
- 首轮广泛回归暴露新增模型计量下 trait 查询的无效重建开销；提前检查模型解释边界后，
  原 `trait_proposition_reuses_budget_after_formation_and_given_checks` 预算保持不变并通过。


### 候选事务中的显式 Type 搜索

`obligations::Engine::search_type_inputs` 将明确给定的未赋值 Type metavariable 映射到
临时 rigid 输入，在独立 inference 快照中冻结调用方给出的约束，再调用 PTC 搜索。
临时声明身份会检查区域前提、问题、已有赋值和作用域中的 rigid 参数（包括 binder
内部），不会依赖某个固定 package 编号永远不被源码使用。

唯一结果只生成 Proposed 候选：最终赋值写入另一份正常 Engine trial，而不是写回
临时编码快照。候选唤醒已有 obligation，并将原始问题重新加入 `require`；`advance`
必须在真实区域中证明原问题和全部既有 requires，之后 `commit` 才能发布。临时搜索
假设、生成的 rigid 身份与模型中的环境变量均不进入最终赋值。失败、歧义、不一致、
未决以及预算中断不修改父状态；已有赋值、错误 sort 和非请求的未解变量不能被覆盖
或顺便推导。要求过期候选提交时仍由原 revision 检查拒绝。

这提供了类型解析候选引擎的显式搜索入口，尚未自动加入普通调用／impl 的匹配调度。
调用方应先尝试直接 unification，再明确交付需要搜索的输入和原始匹配约束；不能把
候选 requires 自动作为输入生成前提。搜索接口目前仍受闭合模型重建片段限制，符号
解、binder 下模型重建和完整持久 CDCL trail 仍是后续工作。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p type_resolution --test type_input_search`：exit 0，五项事务用例；`/tmp/luna-tools-72naccff/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p type_resolution`：exit 0；`/tmp/luna-tools-l97e75xt/result.json`。
- 覆盖候选隔离、原义务重证、requires 不参与推导且仍阻止错误提交、未授权输入、
  预算恢复、临时身份碰撞、sort 和 stale candidate。格式化与 `git diff --check` 通过。


### 普通匹配的模型兜底调度

`Engine::match_many_metered` 在直接结构提案达到固定点后，收集原始匹配关系中仍未解决的
输入，仅在全部属于 Type sort 时调用模型搜索。`match_types` 复用同一事务入口。
搜索使用剩余匹配额度的四分之一作为上限，按实际消耗扣除；找不到确定解或搜索预算
不足时保持原输入未决，不扩大搜索域、不添加 requires 作为假设。

搜索成功后，在另一份 inference trial 中原子导入全部值，再回到原匹配候选。临时
搜索候选的 parent/revision 和假设不被发布；原始匹配关系仍逐项注册为 obligation。
如果模型只选择了区域前提的一条可行分支，原区域无法证明最终关系，候选不能提交。

真实正例：区域前提 `*A == **bool`，将 `A` 类型的实参交给参数 `*T`。直接 matcher
只查看前提的整段形状，无法提出 T；模型搜索提出 `T = bool`，随后原区域的证明器
独立证明完整匹配。源码测试检查生成 NIR 的 FunctionInstance 实参是 bool，拒绝例
则使用 `A == ?bool or A == *bool`，不能把仅某个分支成立的匹配用于泛型函数。
可选参数会触发合法的自动包装，最初的可选源码反例因此被替换为 runtime 指针接口；
没有改变语言转换规则或拒绝合法调用。

尚未完成符号解提取、所有理论的模型重建或统一持久 CDCL trail；该调度不会把
Unknown 或多解提升为成功。需要 comptime/value 输入共同求解的关系保持原有路径。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p type_resolution`：exit 0，319 项；`/tmp/luna-tools-fxz991uy/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p type_resolution --test type_input_search`：增加 direct-matcher 对照后 exit 0，七项；`/tmp/luna-tools-0r351097/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`：检查 NIR 实参的源码正例及拒绝例均通过；`/tmp/luna-tools-ozxdil5j/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test builder_invocations contextual_data_consumer_publishes_structs_through_the_complete_sysroot`：exit 0；`/tmp/luna-tools-adht8rtm/result.json`。
- Workspace 格式化与 `git diff --check` 通过。


### 嵌套区域形状中的符号输入

PTC 新增 `mentioned_types_metered`：按稳定遍历顺序列出区域命题中出现的 Type 候选，
包括等式子结构、分类主体、trait 显式输入及 opaque head 的显式参数。所有候选仅是
待检查的形状；Not／Or 中出现不授予事实，使用前仍要证明实际类型与该形状相等。
遍历、去重和结构检查受同一预算约束，局部 binder 中的引用不会重解释为外层变量。

直接 matcher 的区域别名恢复改用这个候选目录。因此 `*A == **U` 下把 A 匹配到
`*T`，可以先证明 `A == *U`，再保留 `T = U`，不需要构造某个闭合模型。前一阶段
`*A == **bool` 的回归也由更强的直接匹配完成，测试相应更新了对 direct matcher 的
能力断言；类型结果与原区域义务没有放宽。模型搜索仍是直接路径停滞后的后备机制。

这补齐已有区域等式的符号结构恢复，不是完整的 symbolic model extraction；不从
函数体展开 normal form，也不由 opaque head 相等推导其参数相等。约束中尚未出现的
一般符号解、binder 内的上下文重定位与统一持久 CDCL trail 仍未完成。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`：exit 0，438 项；`/tmp/luna-tools-02tj7pf7/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`：exit 0，三项；`/tmp/luna-tools-p0bs9dyq/result.json`。新增源码用例检查 NIR 中保留原声明的 U 身份。
- 原有 opaque-head 非单射、binder 隔离、歧义、低预算及候选 requires 拒绝例保持通过。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test builder_invocations contextual_data_consumer_publishes_structs_through_the_complete_sysroot`：exit 0；`/tmp/luna-tools-_l3awd3c/result.json`。
- Workspace 格式化与 `git diff --check` 通过。


### 根子句传播的查询内延续

根 CNF 传播现在在同一 Boolean circuit 中保存辅助变量赋值、双监视表、待处理队列
与辅助推导证书。新的 theory atom 赋值若仅扩展旧前提，只唤醒受影响的监视子句；
不再为每个推出的 atom 重建整套传播状态。撤销或翻转旧前提时重新建立延续；
预算中断和冲突不保留半完成状态。重复查询尚未接受的 unit 提案仍返回同一义务，
不会自行把提案写入 theory assignment。

证书只保留辅助推导序列，并在交付时附上本次 unit 或 conflict 子句。此前交付的
theory unit 不能混入辅助序列，否则下一次重放会把已成立的前提误作新的推导步骤。
证书构造计费；独立重放仍从原始 CNF 和当前前提重建，完全不读取延续状态。
新增状态的 Display 使用 S-expression。

这减少同一单调分支内的重复传播，尚未实现跨回跳的辅助 trail 回滚，也没有统一
根子句、学习子句和 theory 的全部赋值／理由存储。Type 求解的歧义、作用域与原义务
重证规则不变，不把可满足模型当成确定输入。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`：439 项通过；`/tmp/luna-tools-n4goz9yh/result.json`。
- 补充中断恢复与 dump 检查后，`python3 -B development/check.py --timeout 900 -- test -p ptc boolean::cnf::propagation -- --nocapture`：6 项通过；`/tmp/luna-tools-3nh0cobk/result.json`。覆盖三原子所有部分赋值的源模型检查、连续提案、独立证书重放、前提撤销／翻转、重复冲突和 256 种续跑预算。
- 32 个连续 unit 的对照中，新旧路径分别消耗 4,460 与 27,984 个计费工作单位；不包含一次性 CNF 编译，不是端到端时间基准。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`：通过；`/tmp/luna-tools-4_gzwjm6/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test builder_invocations contextual_data_consumer_publishes_structs_through_the_complete_sysroot`：通过；`/tmp/luna-tools-vz0uaafo/result.json`。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：通过；`/tmp/luna-tools-jbh232p5/result.json`。`git diff --check` 通过。


### 相对于外层参数的 Type 解

Type 输入搜索不再把 `T = U` 中外层 U 的不同具体模型解释直接判为 T 歧义。
闭合模型值不能被证明为唯一时，搜索从原问题提及的 Type 形状中选择不依赖任何
本次待解变量的符号候选；每个交付值仍须由整个问题证明 `T == candidate`。
候选只作为提案，Not／Or 中的出现本身不是事实，不展开不透明 normal-form 头。
输出保留外层声明身份，并沿既有 inference trial 路径重证原义务后才允许提交。

本次排除了 `T = S`（T、S 都待解）这样的循环或未决输入依赖。`T = U`、
`T = *U` 以及分支合并后得到的符号形状可保留；多输入可共同绑定同一外层形状，
不会把搜索临时身份发布到结果。若未找到符号解，只有两份模型的外层 Type 与
整数输入解释相同时才交付 Ambiguous，否则保留 Unknown。比较、候选遍历和证明
都消耗请求预算，预算耗尽不会发布部分绑定。

边界：仍先要求一个有效的满足模型；候选来自问题已有形状，尚未合成任意新的
符号表达式。一般不透明类型模型与 binder 内符号解仍有未实现部分。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p ptc --test type_inputs -p type_resolution --test type_input_search`：通过；`/tmp/luna-tools-43uvz6es/result.json`。
- 最终计费与反例断言完成后，`python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`：443 项通过；`/tmp/luna-tools-zkeht2m2/result.json`。新增检查覆盖符号身份、分支等式、多个待解输入、循环拒绝、非事实候选、低预算恢复、证书重放及候选隔离。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`：通过；`/tmp/luna-tools-6uo02w8a/result.json`。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：通过；`/tmp/luna-tools-o9wci2d6/result.json`。`git diff --check` 通过。


## Dependent symbolic Type inputs reach a fixed point

Model search previously excluded every candidate mentioning another requested
input and stopped at the first unresolved input. Consequently `T = *S`,
`S = ?V`, `V = U` could not recover `T = *?U` when U was ambient, although each
binding is determined relative to U. The added regression failed with
`Unknown(UnvalidatedModel)` before the change
(`/tmp/luna-tools-0a7s9ru1/result.json`).

Search now keeps unresolved inputs while proved bindings grow. Each pass
substitutes already proved bindings into mentioned candidate shapes, excludes
remaining requested identities, and proves every resulting input equality
against the original private problem. It neither adds candidate equalities as
assumptions nor unfolds normal-form heads. Repetition stops on no progress or the
shared budget; ambiguity still needs two models in the same ambient world.
Substitution copying and expanded candidate depth are metered. Type resolution
continues to isolate proposals and reprove original obligations before commit.

Checks cover both request orders, chained pointer/optional shapes, proof replay,
opaque-head candidate preservation, negative/disjunctive occurrences, budget
exhaustion without partial publication, and transactional engine integration.
Budget-dependent search paths can finish with less work than a larger-budget
probe, so a low-budget success is accepted only with replayed proofs rather than
assuming a fixed execution-cost boundary.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 447 passed (`/tmp/luna-tools-srp9kmkh/result.json`).
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: passed (`/tmp/luna-tools-avi2l8ni/result.json`).

- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: 3 passed (`/tmp/luna-tools-60yihh1d/result.json`).
- `git diff --check`: passed.

Remaining limitation: this is dependency closure over mentioned shapes, not
arbitrary symbolic synthesis. An initial validated satisfying model is still
required. For example `T = F<S>, S = U` with opaque F can still return Unknown
because that initial model cannot be reified, even though candidate substitution
and congruence prove `T = F<U>`; the unit test exercises those latter operations
without pretending the full search succeeded.


## Finite interpretations for admitted opaque Type applications

The previous dependency-closure limitation for `T = F<S>, S = U` came from
model reification, not failure to prove the symbolic equality. Witnesses now
carry an ordered finite table of admitted opaque Type applications, with ground
Type arguments and closed structural results. Congruence-class reification uses
those results without interpreting function bodies or treating opaque heads as
injective constructors. Nested calls and equations such as `T = F<T>` can have
finite interpretations without creating recursive structural Types.

The independent witness verifier checks registration, closed inputs/results,
unique ground application keys, all observed atoms, and the complete assumptions
and negated goal. Missing, duplicate or forged table entries are rejected. Model
substitution and table admission charge the shared budget; failures cannot
publish partial input solutions. S-expression counterexample dumps include the
ordered application table. Relative ambiguity checks additionally retain the
same function interpretation, conservatively returning Unknown when the two
tables differ.

These tables are model evidence only. They never enter regional assumptions or
replace the proof needed for an exported Type input. The solver can now recover
`T = F<U>` with a replayable equality proof, while `F<A> = F<B>` still does not
prove `A = B`. Two solver tests that required Unknown after congruence admission
now require independently verified countermodels; unregistered heads remain
Unknown.

The supported fragment covers admitted opaque heads whose arguments and result
are Type-sort. Value arguments, implementation-world trait evidence and trusted
Tuple layout/classification interpretations remain outside this model table.
This is not a complete symbolic model implementation.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 451 passed (`/tmp/luna-tools-sb6gv6qz/result.json`).
- After adding cyclic-application coverage, `python3 -B development/check.py --timeout 900 -- test -p ptc --test opaque_models`: 5 passed (`/tmp/luna-tools-27ujwler/result.json`).
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: passed (`/tmp/luna-tools-24ezdqvm/result.json`).

- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-hopp3w1w/result.json`). Real sysroot imports now also exercise `A == List<U>` forwarding to `List.length()`; the Integer assertion stays comptime and only its bool result enters runtime. Earlier fixture attempts correctly rejected a runtime pointer to comptime-only List, an incorrectly spelled method, and runtime use of Integer; no phase rule was relaxed.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: 3 passed (`/tmp/luna-tools-zdnj4cdw/result.json`).
- `git diff --check`: passed.


## Completed intrinsic values in opaque model applications

Finite application tables now also accept completed Unit, bool, char, exact
Integer, str, Symbol and BitVec identities as ordinary inputs alongside Type
arguments. Reification and independent replay share the same admission rule;
results must still be closed Types. No declaration name, List<Type> shape or
special normal-form input slot is recognized. Symbolic Type-input extraction
retains these values in the proved opaque application rather than replacing the
application with its model's incidental structural result.

Different input values do not imply different output Types: models validate both
equal-output and unequal-output possibilities for `F<true>` and `F<false>`.
Floating values, open comptime parameters, value-store handles and aggregates
needing protocol evidence remain Unknown. This change does not import program
arithmetic or user Eq/Hash into PTC.

Input traversal now charges strings/symbols by byte length, exact integers by
limb count and BitVecs by word count before hashing or copying them. The initial
preflight and witness replay both enforce this budget. Tests cover every admitted
value category, independent proof/model replay, unsupported and forged domains,
large-payload exhaustion and successful retry with sufficient work.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 456 passed (`/tmp/luna-tools-ufg1rcg9/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: 4 passed (`/tmp/luna-tools-6w_w27uq/result.json`). The new source case checks the inferred `Tagged<U, true>` retains the original head, ambient U and completed bool input.
- `git diff --check`: passed.

## Reversible root propagation trail

Root CNF continuations now survive premise retraction and search backjumps.
The session records external theory-atom assignments and auxiliary implications
in chronological order, with the root clause responsible for each auxiliary.
On a removed or flipped premise, synchronization finds its earliest trail
position, undoes that suffix and retains the valid prefix. Still-present caller
assignments in the undone suffix are then admitted again. This is conservative
chronological rollback, not dependency-minimal invalidation.

Watches persist across these transitions. Unit clauses retain subscriptions;
undone implications reschedule their reason clauses, and pending proposals or
conflicts are reconsidered after rollback. Repeated conflicts remain available
without consuming the queue and incorrectly returning no propagation. Watch
buckets now use ordered sets with explicit subscription removal, so repeated
assignment and retraction cannot accumulate duplicate/stale subscriptions.

Auxiliary certificates are truncated with the trail. Independent replay and
explanation still reconstruct the original CNF without consulting retained state.
Budget-interrupted updates are discarded by the circuit owner; no partially
updated trail becomes evidence. The S-expression dump includes chronological
entries, auxiliary reasons and a retained conflict clause.

This extends the earlier monotone continuation stage. Root, learned and theory
reason storage are not yet unified: search still communicates atom-assignment
snapshots, and synchronization scans those atoms. Full event-driven CDCL trail
integration remains unfinished. Type input extraction still requires a proof of
the candidate equality under the original problem; a satisfying model alone
never authorizes committing an inferred Type.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 458 passed (`/tmp/luna-tools-_mbth90g/result.json`).
- Regressions preserve a 64-auxiliary prefix across 64 branch changes, compare with fresh propagation, independently replay certificates, retain/retract conflicts, and check bounded subscriptions after 1,000 transitions.
- Existing exhaustive three-atom partial assignments and interruption/recovery checks also use the retained path. No expected logical outcomes or independent verifier checks were weakened.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-el942yz6/result.json`), including rejection of a model satisfying only one regional alternative.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-x5v118in/result.json`), exercising real library List identity and method forwarding.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`: passed (`/tmp/luna-tools-0yuiomdl/result.json`); `git diff --check` passed.

## Search-owned assignment trail

Search decisions, circuit-forced atoms, root-clause units and learned-clause
units now write through one chronological assignment owner. Entries retain their
literal, decision level and origin. Normal unwind and nonchronological backjump
remove entries in reverse order; backjump no longer scans every atom to clear
higher-level assignments, and logical depth is recovered from the retained trail
length. Entry premises remain outside the removable search suffix.

Proof explanation uses a scoped temporary opposite-value view. The callback sees
only a read-only assignment slice; the original value is restored on return or
unwind, without changing the live trail or decision levels. Dropping the search
owner restores its caller's entry assignment even during host unwinding.
S-expression dumps identify each assignment's level and source.

This is a shared assignment ownership boundary, not yet the final shared reason
arena. Proof-resolution continuations still retain learned/root reasons; root
auxiliaries have their own reversible trail, and root/learned/theory consumers
still synchronize assignment snapshots rather than consuming mutation events.
The existing conservative backjump fuel reservation is unchanged. Independent
proof/model validation and explicit Type-input admission remain unchanged.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 459 passed (`/tmp/luna-tools-kjzsqluw/result.json`).
- After restricting probe callbacks to read-only slices and adding whole-search unwind coverage, `python3 -B development/check.py --timeout 900 -- test -p ptc --lib search`: six passed (`/tmp/luna-tools-t210xi2o/result.json`). Includes first-UIP learning, lower-level-prefix retention, proof support, all search exit kinds across budget cutoffs, and temporary-probe restoration.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-ieh8wiss/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed with the shipped library (`/tmp/luna-tools-7o16fj8w/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-l3m0bbla/result.json`).

## Trail notifications for learned clauses

The production learned-clause propagation path now consumes deduplicated changed
atom IDs from the search trail instead of comparing a complete previous
assignment vector. Decisions, propagated assignments and undo/backjump operations
all emit notifications. Scoped proof probes do not: their opposite value is not
a change to the live search world. The trail's current notification buffer has
one consumer, learned propagation; root CNF and theory still use snapshots.

A fresh search starts a new notification epoch and schedules existing clauses
for reinspection. New learned clauses remain pending independently of whether any
atom changed. Successful propagation acknowledges notifications; interrupted work
keeps them available for retry, and resending IDs only schedules clauses once.
Clause-body interruption leaves the unfinished clause pending. Ordinary search
cleanup restores caller values without allocating notifications for a consumer
that can no longer run; the next search's epoch handles that boundary.

The old snapshot synchronizer is now test-only. The production watch table no
longer stores an atom-sized previous-assignment vector. A 4,096-atom test checks
that a quiescent event pass succeeds with zero fuel and a change to one unwatched
atom consumes only one notification step. This is a local work bound, not an
end-to-end timing claim. Independent clause/proof validation and model admission
are unchanged. Shared reason storage and root/theory event consumers remain work
for later stages of the complete CDCL implementation.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 461 passed (`/tmp/luna-tools-3moca8j2/result.json`).
- New tests compare all 81 four-atom partial assignments in both directions against a full clause evaluator, retry interrupted notifications across 40 fuel cutoffs, reset search epochs, and add new clauses without assignment changes. Trail tests verify that proof probes emit no events and backjump reports exactly the undone atoms.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-4qvx1qjm/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-ubls547g/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-fpf3ybci/result.json`).

## Independent root-CNF notification consumption

Root CNF propagation now consumes trail changes as well. Root and learned
propagation have separate pending sets and acknowledgement states because they
can run at different points in search. Before a consumer's first call, a complete
snapshot is required and individual notifications need not be stored. Once
initialized, assignments and backjumps notify both active consumers; neither can
clear the other's pending work.

Root synchronization examines notified atoms to find the earliest invalid
auxiliary-trail premise. Undoing that suffix can also remove caller atoms whose
values did not change. Those atoms are explicitly added to the readmission set,
so the new local trail still reflects the complete caller state without scanning
every atom. Independent certificates continue to replay from the original CNF.
A failed root update discards its partial session; retry initializes from the
full current assignment even if the caller resends only its pending changes.
New search epochs discard previous proposals and auxiliary state.

The previous full-snapshot root synchronization entrypoint is retained only for
tests. A quiescent 4,096-atom root session needs no synchronization fuel; adding
one unwatched assignment uses three local work units (copy/change/readmission),
independent of the atom count. This does not eliminate the separate Boolean
circuit evaluation in search, and theory still consumes assignment snapshots.
Root auxiliaries and proof-resolution continuations also remain separate from
the final shared reason arena. Complete CDCL integration remains unfinished.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 464 passed (`/tmp/luna-tools-ror26qqr/result.json`).
- Exhaustive three-atom partial assignments now exercise the production notification entrypoint in both directions, with independent source-model checks and proof replay. Added tests cover 96 interrupted-update budgets, new epochs, delayed consumers, unchanged premises removed by local rollback, and bounded sparse-update work.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-3zlh09t3/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-2xcubrze/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-knh612vj/result.json`).

## Sparse theory notification consumption

Theory is now a third independent consumer of assignment-trail notifications.
Its reversible equality cache stores each admitted atom once, together with its
introduction layer. Layers retain only their new atoms and a union checkpoint,
instead of copying the entire assignment at each successful check. Changed atoms
identify the earliest invalid layer. Removing its suffix also queues unchanged
atoms from later layers for readmission. Failed checks retain this pending debt;
failed rollback reservation leaves the admitted facts and classes unchanged.
A new consumer epoch reconstructs the cache from the complete assignment.

Boolean propagation now reaches a fixed point before theory checking. Partial
assignments still undergo theory checking before decisions, but no longer copy
an owned equality model that search immediately discards. Only witness production
requests that snapshot. This removed redundant work exposed by the existing
trait-candidate budget regression: the initial cache refactor exhausted the
negative requirement query, leaving a candidate pending. The original budget and
selection expectation remain unchanged; diagnostic instrumentation was removed.

This advances the CDCL infrastructure supporting Type-sort model search. A
satisfying model remains only a candidate for inference: committing a Type
solution still requires the independent entailment check under original
constraints. Explicit language design and opaque NF identity are unchanged.
Theory checking itself still scans assignments and term relations; notifications
currently optimize cache synchronization, not every theory operation. Shared
reason storage and complete CDCL integration remain unfinished.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 465 passed (`/tmp/luna-tools-1bmieo63/result.json`).
- Warm/fresh comparison covers all 81 four-atom partial assignments in both directions through the notification entrypoint, plus interrupted calls across 256 budgets. Sparse-layer tests use 4,096 atoms, check atomic rollback across 12 budgets, and verify unchanged later facts are readmitted. A quiescent cache preparation uses zero synchronization fuel.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-ktfqu0vu/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-knbijlif/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-tb0gtz76/result.json`).

## Assignment-owned propagation reasons

The chronological assignment trail now owns each propagation's reason. Root
and learned clause explanations travel with the assigned literal; Boolean
propagation carries its deferred circuit-replay reason. Search continuations
hold only the start of a propagated suffix rather than duplicating its literal
list or separately owning one clause explanation. All propagation kinds now
share the same unwind/resolution path and first-UIP observation points.

Undo transfers an explanation to conflict resolution. Nonchronological jumps
and search teardown release the discarded suffix's explanations while retaining
the surviving prefix. Clause-store eviction cannot invalidate an assigned
literal's owned proof (including its shared immutable lemma references). Trail
dumps include the origin and concrete explanation in S-expression form.

This is an ownership step toward complete implication-graph analysis, not a
claim that the final shared reason arena is complete. Boolean explanations are
still reconstructed lazily against the restored prefix; root auxiliary reasons
remain in the CNF engine, and decisions still use explicit continuations.
Independent proof replay and candidate-model validation are unchanged.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 466 passed (`/tmp/luna-tools-2p3x5ny1/result.json`).
- Reason lifetime coverage checks retained prefix evidence, discarded suffix references, ownership transfer on undo, and teardown with live explanations. Root-unit coverage now extracts the proof from its assignment and independently checks it under sufficient and insufficient premises. Existing exit-restoration and first-UIP tests exercise the unified unwind path.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-uilrw90y/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-8rww2_ja/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-z7pvo424/result.json`).

## Boolean propagation captures its antecedents

Circuit propagation now returns a unit literal together with the antecedents
that justify it. Descending through a disjunction (or negated conjunction)
collects evidence for the other children's known values using the current
circuit evaluation. Descending through a conjunction shares an immutable
antecedent set among its units. No type terms are copied into these reasons.

The assignment trail retains that support. Resolution adds the negated unit to
form a Boolean-conflict proof, without rerunning the circuit against a probe.
The independent proof checker still evaluates the original formula; recorded
support alone cannot authorize a proof. If optional support materialization
cannot fit the remaining budget, the existing conservative all-premises proof
is retained. Propagation itself charges explanation traversal and cannot publish
partially collected antecedents on interruption.

This replaces the preceding stage's lazy Boolean re-explanation. It does not
make the whole Boolean circuit incremental, unify CNF auxiliary reasons, or
complete CDCL implication-graph analysis. These remain distinct implementation
gaps; Type inference still requires entailment beyond a candidate model.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 467 passed (`/tmp/luna-tools-y57eowy5/result.json`).
- The exhaustive partial-assignment circuit tests now independently evaluate each reason using only its antecedents and the opposite unit. Added coverage verifies shared support across conjunctive units, exclusion of irrelevant assigned atoms, failure when a required guard is removed, and retry after every insufficient propagation budget.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-swnci1ir/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-uiqtfwv4/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-y4lc6x0b/result.json`).

## Active first-UIP analysis over live assignments

Search now analyzes a returned conflict before unwinding continuations. It walks
the chronological assignment trail backwards, skips literals outside the
conflict support, and resolves current-level Boolean/root/learned reasons until
at most one current-level literal remains. The resulting proof enters the
existing clause-admission and asserting-backjump path. This identifies the UIP
while assignments, theory state, and propagation notification cursors remain
unchanged; chronological unwind remains the fallback when analysis or retention
cannot finish within its budget.

Analysis uses a bounded fraction of query fuel. Every intermediate resolution
is already a valid proof, so interruption returns that proof rather than a
partial dependency claim. Boolean reasons use captured antecedents. Root and
learned reasons copy bounded owned proof structure while sharing older lemma
DAGs; preflight limits copying depth before recursive clone. A unified proof
arena would remove this remaining copy. Full CDCL completion is still not
claimed: root auxiliary reasoning and decision continuations remain separate,
and Boolean/theory evaluation is not entirely event-driven.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 468 passed (`/tmp/luna-tools-wkqbuuop/result.json`).
- A mixed-reason implication graph checks all 512 analysis budgets, independently replays every intermediate proof from only its recorded support, verifies the completed UIP is the propagated dominator, and rejects the opposite UIP premise. Trail contents and all three consumers' pending notifications remain unchanged.
- The existing end-to-end first-UIP search test now asserts that active analysis actually resolved live trail reasons before the learned asserting clause is reused on the opposite decision branch.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-3obuzhk1/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-i6pp0b27/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-bf6d111b/result.json`).

## Shared immutable proof payloads

Proof splits now share both child nodes and their pivot atom. Theory literals,
lemma premises, and CNF replay traces use immutable shared slices. Cloning a
proof root therefore retains its payload instead of recursively cloning owned
split trees or copying type terms and clause traces. Existing lemma references
continue to form a DAG, and S-expression dumps keep their established content.

First-UIP reason reuse now charges only the copied query-local support set and
one root operation. It no longer walks the entire proof before copying or
rejects reuse because the owned tree exceeds the former 32-level copy limit.
This is not a relaxation of verification: proof admission and independent replay
retain their own depth, fuel, premise, and declaration-authority checks. Shared
nodes remain alive when a clause store or original reason is released.

This removes owned subtree copying from the preceding stage. It is reference-
counted immutable sharing, not a unified indexed arena for root auxiliaries and
search reasons. Verification still traverses the proof under its actual
premises; sharing alone supplies no cached validation result.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 469 passed (`/tmp/luna-tools-kh1eyunu/result.json`).
- A 64-level proof reuses its nodes with one unit of copy fuel, survives release of the original root, preserves its dump, and independently replays successfully. Copy-on-write corruption of a shared pivot is rejected without changing the retained proof. A short verifier depth still rejects the valid deep proof and restores assignments.
- Existing learned-clause eviction, missing/duplicate premises, forged CNF traces, scope checks, and all interrupted first-UIP analysis budgets remain covered.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-r8nkny3j/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-blu7g9h4/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-ql8mge5h/result.json`).

## Explained equality-theory propagation

After a successful theory synchronization, search now asks the retained equality
closure for an unassigned type-equality literal. Equal roots imply a positive
literal; a retained negative equality implies a negative literal for another
pair with the same unordered roots. This includes transitivity and structural
congruence already established by the ordinary closure, without treating model
representatives as equalities or introducing library-specific cases.

Each unit carries a theory-conflict proof under the admitted source premises
and the opposite target literal. It enters the assignment trail at the current
decision level with a theory origin and participates in the same unwind and
first-UIP analysis as other explained propagations. Premises are conservative
rather than minimized. Independent replay rebuilds theory from those source
premises; it never trusts the retained closure as a certificate.

This optional scan has a bounded fuel allowance, charges root traversal and
explanation construction, and leaves closure state unchanged on interruption.
Large scans may decline propagation and continue ordinary search. This stage
covers equality and transported disequality only; it does not yet provide
explained propagation for every kind, trait, or arithmetic atom, or replace
full theory scans with an incremental theory agenda.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 471 passed (`/tmp/luna-tools-6_jzdjzp/result.json`).
- All 81 partial assignments over four equalities are visited in both directions; each produced positive/negative unit is checked by fresh theory and independent proof replay. Another 512 budgets check interruption without retained-state mutation. Unconstrained variables do not propagate merely because a model can choose equal representatives.
- Production search tests require actual theory-unit insertion before either successful model production or a subsequent Boolean contradiction; the contradiction proof independently replays and both paths restore the entry assignment.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-qdkw061p/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-i7pz6xcx/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-ts09fqd0/result.json`).

## Explained kind and trait propagation

The theory-unit scan now also indexes trait truth by declaration identity and
the equality roots of every input (subject plus Type/comptime arguments). A
known positive or negative literal transfers only to the same proposition under
congruence. Missing implementation evidence still produces no literal; the
implementation-world adapter remains responsible for proving selection or
absence. Different declarations, arities, or noncongruent inputs remain separate.

Kind units use either an existing class member's audited classification or an
explicit kind premise on that class. The classification helper expresses the
existing inclusion, disjointness, and contrapositive rules and is shared with
consistency checking. A negative signed-integer fact does not establish unsigned
integer, and an integer fact does not choose a signedness. No opaque factory
body is unfolded to manufacture a classification.

Equality, kind, and trait units share the same conservative source-premise proof
builder, trail ownership, and first-UIP path. The scan remains bounded and
nonmutating; arithmetic propagation and a fully incremental theory agenda remain
unfinished. This expands the preceding equality-only stage without changing
model admission or making inference more implicit.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 473 passed (`/tmp/luna-tools-bxjy1a2k/result.json`).
- Exhaustive partial assignments in both directions cover primitive classification through equality, kind implications/disjointness, trait subjects and Type arguments, distinct trait declarations, and distinct comptime-value indices. Each propagated reason independently replays; unconstrained predicates stay unassigned.
- Production search tests require positive and negative kind/trait propagation, then independently replay the resulting complete contradiction proofs. Existing equality interruption and rollback coverage remains active.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-rrqas4q0/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-0rmpdwz8/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-w1j693_g/result.json`).

## Explained integer difference-bound propagation

The existing mathematical-integer theory now shares one difference-graph
construction between consistency/model checking and propagation. Positive and
negative literals use the same checked offset conversion; negation adjusts the
bound by one in the integer domain. The graph has a stable S-expression dump.

Propagation computes shortest paths from each queried source as needed, reusing
those paths within the scan. Unreachable vertices remain unknown. It does not
reuse the model's all-zero super-source distances as logical bounds. A path
proving the queried bound, or its integer negation, proposes a theory unit.
Before publication, the opposite literal is checked with the independent
consistency path to ensure the certificate is replayable within the current
arithmetic capacity. Overflow or budget exhaustion declines optional propagation
without publishing a partial reason or inventing a machine-overflow rule.

Integer units use the existing source-premise proof builder, trail, and conflict
analysis. This concerns integer predicates in PTC's type propositions, not
program-state or memory specifications. The implementation still covers the
existing difference fragment, not general linear/nonlinear arithmetic or modular
constraints. Graphs are rebuilt per scan; explanation support is conservative,
and the extra certificate-capacity check is not an incremental path proof.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 476 passed (`/tmp/luna-tools-6vvoles4/result.json`).
- Partial-assignment propagation is compared against direct integer valuations for all 81 assignments of a four-bound system, covering positive and negative consequences. Tests retry every insufficient propagation budget, check constant bounds and unconstrained variables, and verify graph dumps.
- `python3 -B development/check.py --timeout 900 -- test -p ptc integer::tests`: two passed after adding the certificate-capacity boundary (`/tmp/luna-tools-b38dhyhe/result.json`). A path may fit i128 while opposite-literal replay exceeds capacity; that case publishes no unit.
- Production search requires actual positive/negative integer propagation and independently replays the resulting contradiction proofs with entry assignments restored.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-jg4p5pnc/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-4n7tngy2/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-27fq34h2/result.json`).

## Source-path certificates for integer units

Shortest-path propagation now retains predecessor edges and reconstructs the
actual signed source constraints establishing a bound. The proof records that
ordered path, its conclusion, and polarity using shared immutable payloads.
Conflict support includes only the path literals and the opposite conclusion,
not every assigned predicate. This replaces the previous whole-graph
opposite-literal trial during propagation.

The independent verifier checks exact source-literal membership and polarity,
requires the opposite conclusion, checks connected endpoints, and sums path
weights with checked arithmetic. The resulting bound must imply the conclusion.
It never consults retained distances, predecessor arrays, model values, or a
cached validation decision. Empty paths prove only valid same-vertex bounds.
Path corruption, absent premises, or excessive conclusions are rejected.

A finite path can fit the arithmetic capacity even when Bellman-Ford on the
added opposite edge would overflow before recognizing a negative cycle. Such
propagation certificates now verify directly. This does not widen mathematical
integer representation or fix all consistency-query capacity limits: path
summation can still report ArithmeticRange, and the baseline negative-cycle
solver is unchanged. Difference graphs also still rebuild per scan.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 478 passed (`/tmp/luna-tools-wvl_jxrl/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p ptc integer::proof::tests`: two passed after strengthening irrelevant-premise coverage (`/tmp/luna-tools-f4tmsu3q/result.json`). Positive/negative certificates reject missing premises, wrong polarity, disconnected/reordered paths, stronger conclusions, and short budgets; unused source constraints do not enter support.
- Direct replay covers the former opposite-cycle overflow boundary, valid empty reflexive paths, and genuine checked-sum overflow. Existing valuation comparisons and production search proof replay remain active.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-l40lgp_v/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-oocth4t_/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-794dzesz/result.json`).

## Incremental Boolean circuit values

The Boolean circuit is now a fourth independently acknowledged trail consumer.
After reverse dependencies are installed, changed atoms schedule their source
occurrences; topological node order recomputes affected parents only when a
child's three-valued result changes. Assignments, removals, and backjumps use the
same path. A new query epoch rebuilds from the complete assignment. Interrupted
evaluation or explanation discards its session, so retry cannot retain partially
updated node values. Proof replay still evaluates source formulas independently.

Index construction is optional and amortized against observed direct-evaluation
work. The circuit first evaluates directly; it installs reverse dependencies
only after enough reuse and when the full reserved construction cost fits a
bounded fraction of remaining fuel. Tiny circuits keep direct evaluation.
Eager indexing initially regressed existing short-query budget tests; retaining
those budgets and making initialization demand-driven restored them. No test
expectations or language rules were relaxed.

This eliminates repeated whole-circuit value evaluation for warmed, affected
subgraphs. It does not yet cache child truth counts inside wide operators or
make unit-reason extraction incremental, so changing a wide parent's value may
still scan that parent's children. The earlier root-CNF and theory caches remain
separate consumers; complete CDCL integration is still unfinished.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution`: 481 passed (`/tmp/luna-tools-c1ahz0qu/result.json`).
- Every four-atom partial assignment is compared with full evaluation in both directions, including unit antecedents and conflict support. Another 128 budgets exercise interrupted updates, retry, and new epochs. Trail tests check independent circuit acknowledgement.
- Warmed 16- and 4,096-subexpression circuits use equal update fuel (at most 16 units) when changed atom occurrences leave their containing tautology true; a quiescent settled circuit needs no evaluation fuel. This is bounded local work evidence, not an end-to-end timing claim.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_model_inference`: four passed (`/tmp/luna-tools-j9qhz343/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_type_factories_keep_identity_through_comptime_parameters`: passed (`/tmp/luna-tools-byjfqnl3/result.json`).
- Formatting check and `git diff --check` passed (`/tmp/luna-tools-nza7nmmp/result.json`).
