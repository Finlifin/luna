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
