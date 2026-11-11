# 对 2026-11-11 晚报的回复

核对对象：`ai_dev_logs/comptime_type_vm_divergence_report_2026-11-11_evening.md`。
报告主体基线是 `a00e221f`，附录推进至 `bfe8221c`；本回复在 2026-11-12
核对至 `4937d332`。保留原报告，不把不同基线的事实混成一次测量。
formatter 由另一位开发者负责，本次未修改 `compiler/fmt` 及其依赖。

报告第 7.4 节指出的三类根因成立：重复实现同一表达式语义、跨层值传送分散、
生产证据未完整交付给消费者。下一阶段应按这些职责迁移，而不是追着行数、
枚举变体数或测试通过数量继续增加局部分支。以下同时列出必须修正的审查判断。

## 1. 已过时或证据不足的结论

### 前端没有接入 CodeSession

报告第 7.10 节已纠正主体的这项判断。当前
`elaboration/src/program/required/prepared.rs::RequiredWorld` 持有代码会话，
required 值与 Type 计算通过租借调用；`type_computations::Session` 有
`resume_with_code_session` 生产接线。不能再据主体第 2.2 节安排一次相同接线。

仍然缺少的是跨稳定查询的活动执行所有权和完整依赖复用，不是所有调用都必须共用
一台可变 Interpreter。代码缓存、结果缓存和活动帧分别有不同生命周期。作者已限定
当前 invocation 结果缓存只考虑输出 Type 的调用；输出 Type 也不自动证明纯度、
依赖稳定或缓存收益。不能为了消除“结果缓存零调用者”而把所有调用接入记忆化。

### Value::Pointer 的存在证明 comptime 仍支持指针

这个推论不成立。解释器还承担 runtime 参考执行；共享表示含 Pointer 不能独自证明
comptime 源程序能执行指针操作。当前有两层明确边界：

- `nir/src/phases.rs` 在 comptime 执行区域拒绝取址、读写和指针投影；
  `phases/comptime_storage.rs` 拒绝实际存储 runtime 指针的 comptime 载荷。
- runtime 指针的 Type 描述、runtime callable 的签名及 quoted runtime 代码不是
  comptime 指针载荷，必须仍然允许构造。`memory.rs::observe_pointer_scalar`
  也明确拒绝 comptime-only VM。

本次重跑 `comptime_pointer_phase`、`comptime_storage_domains`、`comptime_receivers`，
22 项通过，包括未调用定义、不可达代码、值引用接收者、runtime 参考执行和 quote
边界。证据见第 4 节。这个结果不等于所有宿主注入路径已穷尽验证；报告所指出的
pointer-copy 回调耦合仍值得在执行角色拆分时审计，但应给出可达调用链，不能据其
注册位置认定已经发生语言违规，也不能删除 runtime 指针能力来“修复”计数。

### 七套引擎应统一成一个求值器

NF 专用表达式、普通表达式及 invocation fallback 的重复职责确实需要收敛。
但 PTC 对类型命题的解释和程序验证层的符号状态推演不是两台重复的 Flurry VM。
前者不能接管内存／资源规格，后者也不能为了复用而执行用户程序取得符号结论。
被动观察已有稳定身份与主动执行用户函数同样必须分开。

目标是每个语言动作有唯一检查和执行契约，各消费者共享该契约；不是把理论求解、
混合 interpretation、runtime 参考执行和 comptime invocation 强塞进同一个循环。
保留参考引擎不代表允许生产 invocation 对未实现操作静默退回另一套语义。

### PTC 只完成了 4/19

四种 `ptc::Atom` 变体是当前表示事实，不能据此计算语义完成率。正式
`docs/type/propositional-constraints/atoms.md` 明确允许 A13、A18 派生，也要求
公开谓词先尝试降到已有公式。是否落实需逐项核对形成域、正负解释、残余、冲突理由
和实际消费者；不能通过添加十五个枚举成员宣称完成。

`ptc/src/model_search.rs::witness` 遇 Trait 原子放弃裸模型构造的限制仍在。
但 `type_resolution/src/obligations/type_inputs.rs` 已有模型搜索托底及选定 Type
投影，候选解仍须满足原义务。elaboration 没有直接调用 PTC API，不等于其间接
生产路径不存在。下一步是带合法实现世界的联合模型与证据交付，不是删掉 Trait
拦截后把未经解释的模型当解，也不是默认扩大隐式推导范围。

报告第 4.2 节关于整数差分理论每次重建的判断，在 `4be43a06` 基线仍成立。
本次已将 `integer::State` 接入搜索的理论层：保留差分边、邻接表和可行势函数，
只从新增边的源点启动有轮数界限的 Bellman–Ford 传播；没有新整数事实时不重复
松弛。每层同时记录 EUF 和整数检查点，回跳预留撤销预算后恢复边与势函数；
失败或预算中断撤销本次未提交更新，后续重试不沿用半成品。整数赋值只在导出
模型时物化。新增状态及检查点均有 S-expression Display。

整数传播也已接通同一份保留图；每条边携带源 atom 索引及极性，并随边一起撤销。
生成理由时借用图计算源点路径，只复制路径上的源文字，不重建整个变量目录和图。
提取路径时核对当前源文字的极性，导出的证书独立拥有文字，不含活动图的边句柄。
回跳并复用边槽后，旧证书仍能在原前提下重放，但不能借旧前提证明新分支。
最短路径仍按次计算，尚未跨传播调用维护。冲突最小化、proof replay 和 witness
检查继续使用独立求解／验证；行、Trait、Kind 的重算、
Tseitin 辅变量的独立状态、跨查询学习和合法 trait 模型交付没有因此完成。
也没有增加原子、扩展数学整数理论范围，或将程序状态规格交给 PTC。

工作目录 `/home/fin/luna`，本次执行：

- `python3 -B development/check.py --timeout 900 -- test -p ptc --no-fail-fast`：
  `/tmp/luna-tools-9rt1yakk/result.json`，退出 0，245 项通过。
  新增用例比较 243 种部分赋值的正反遍历与独立求解、有限整数模型枚举；
  覆盖预算切断重试、已存在边的再传播、负环撤销和算术容量失败恢复。
  原有 CDCL 正负整数理由与独立证明重放用例同时通过。
- `python3 -B development/check.py --timeout 900 -- test -p type_resolution --no-fail-fast`：
  `/tmp/luna-tools-zmwd2p51/result.json`，退出 0，396 项消费者回归通过。

初次编译的内部状态重导出可见性设置过窄，已修正；没有修改语义预期。

传播接线完成后执行
`python3 -B development/check.py --timeout 900 -- test -p ptc -p type_resolution --no-fail-fast`：
`/tmp/luna-tools-n4vy2_fx/result.json`，退出 0，642 项通过。新增覆盖混合 atom
索引、正负前提与结论、回跳复用边槽、过期极性拒绝及证书脱离活动图后的独立验证；
预算切断不会改变保留状态，同路径生成无需再次支付构图开销。

### List、Tuple 和跨阶段值转换的数量直接代表特权数量

`Role::TypeList`／`Role::Tuple` 仍参与值域准入，NormalFormArgument 仍存在，
这些确实是待迁移路径。`normal.rs` 对动态 NF 收集器的具名实参仍有明确拒绝，
也不能因为普通声明输入已经共用就声称所有 callable 完成统一。

但源码列表语法需要有 lowering 分支；类型身份、内部积视图、元组值也不是天然应当
合并成同一个对象。审计应查结构视图是否越权改变身份、准入是否依赖具体库头、
转换是否丢失域／phase／world／图共享，而不是仅计算 `H::List` 或 Tuple 的出现数。

同理，`Identity<bool>` 的合法具体值需要完整构造与域证据；不能从其函数体返回 bool
就直接把 bool 值接纳进该域。报告的通用值路径缺口应修复，但修复不能破坏 opaque NF
头不展开的契约。wire 编码、VM 恢复和 runtime 物化有不同输出职责，统一其传送
契约与准入依据，不要求把不同操作改名成一个转换函数。

### math 镜像及 comptime trait 所在包属于设计偏离

作者明确要求 builtin.math 服务 runtime，并提供 meta.math 的 comptime 对应能力。
两份签名本身是预期接口；可以共享描述或生成维护代码，不能取消一个阶段来减少行数。
基础 comptime trait 位于 meta 也不能仅凭所在包认定违规，需有正式归属约定或具体
解析／导出失败。重复原始绑定仍应按真实符号、阶段签名和包依赖分别清理；集中绑定
不等于必须把所有包的绑定物理合并到一个文件。

## 2. 仍成立的架构缺口

| 生产证据 | 必须完成的迁移 | 不能冒充完成的局部工作 |
| --- | --- | --- |
| `declaration_inputs` 的 Typed 输入、`dynamic_instances` 的 pending 签名限制 | 普通检查区域与 VM activation 共同保留待定域、真实生产者及已执行前缀，恢复后形成签名 | 扩展 NormalFormArgument 的语法白名单；提前执行后实参；重跑整个调用 |
| `bytecode_invocation_available` 的 cleanup/effect 门控及契约回调 | 合法 comptime 操作在同一 invocation 进度协议内执行、失败、恢复 | 只去掉门控或更换默认引擎；不处理尚未支持的操作 |
| 值导出、准入及恢复散落多个消费者 | 统一保留类型域、阶段、所有者、捕获与图共享的值传送契约 | 把所有值转成 Term 后丢掉不可表示信息；按库头补白名单 |
| `LiftType` 缺少 HIR lowering，TypeRef 仅有闭合消费者 | 类型依赖生产、带域环境取得、资源有效性与 IIR 消费共同接通 | 新增 EnvironmentPlan 空壳；传空 substitution；只把 panic 改成成功 |
| `memory.rs` 两处 string-view provenance 拒绝 | 文本 backing region 从调用接口传到清理、捕获和返回检查 | 删除拒绝分支；将所有 str 当作永久存活的指针 |
| Trait 裸模型及理论证据未闭合 | 按 world／guard 维护模型解释和原义务，保留 Budget、Unresolved、Invalid | 把未知判成否；把任意见证赋给显式输入 |

lift 崩溃是需要纠正的诊断缺陷，消除崩溃与实现依赖类型必须分开验收。
本回复对该点为静态核对，没有声称已经执行并复现或修复。
完整的 Region／Candidate 所有权、任意 definition 发布、独立高级 IIR、细粒度持久
query 和 std 的实际接口仍未完成；报告中的局部能力不能替代这些交付。

继续沿契约路径核对时发现，第 2.1 节的定位也需细分：`Progress::Contract` 经
`Execution::Contract` 已能进入字节码执行，它服务接口调用，不能直接称为必然逃回
树解释器。实际仍同步执行普通声明前后置条件的是 `prepare_invocation`／原
`finish_invocation`。这条路径现已迁入 owned activation 的 entry/body/exit 状态，
对应交付与限制见[普通函数契约的恢复边界](elaboration-state-and-publication.md#普通函数契约的恢复边界)。
编译 Unsupported 后静默选择 tree 的 fallback 现已移除，代码缓存也不再存储失败
决定；显式参考执行、嵌套 staged input 与 mode 门控仍在。十三种不支持节点的阶段
定位、额外 guarded 分支及剩余迁移见
[编译拒绝与执行模式分开](elaboration-state-and-publication.md#编译拒绝与执行模式分开)。
尚未接通一般前端检查请求的判断仍然成立。此次扩大 CMTT 验证还复现了
`generator_clause_domains_depend_on_prior_value_indices` 的已有 required 输入捕获失败，
修改前 `59405db5` 的三个 VM／编译文件也产生相同诊断。后续已沿
[required 输入证据的区域所有权](elaboration-state-and-publication.md#required-输入证据的区域所有权)
修正它，并覆盖普通函数默认值和真实 meta.List 消费者：已供应输入进入准备区域，
普通调用的观察事实留在原观察表，不能据此删掉或提前执行 producer。
原先失败的证据保留于前一小节；当前验证单独记录，没有跳过或放宽该正例。

Type 生产队列还有一个实际断点：保留输入等待者之后，就绪计算仍通过同步 VM 调用
一次执行到底。现已增加队列拥有的 invocation 进度，并将 required Type 的生产路径
迁入该驱动；关联选择可交给 VM 外处理，完成后才发布 Type。取消与失败终结原计算，
迟到响应不消费新等待者，代码缓存也不持有活动推断状态。详见
[Type 生产者的 invocation 与发布边界](elaboration-state-and-publication.md#type-生产者的-invocation-与发布边界)。
活动句柄随后已改为单输出发布许可：暂停期间可在同一上下文继续独立推断或提交
无关候选，其他写入不能抢占原输出；错上下文 poll 和迟到响应不改变原执行。
这仍不是通用前端 Region：参数准备与部分服务仍同步，两个普通 callable 待定
正例仍未解锁。报告关于全链路恢复未接通的判断继续保留。

定义 builder 的同步边界也已进一步拆开：struct／enum／newtype 的源字节码请求
现在能离开 invocation 驱动，宿主完成准备后返回未提交 receipt，由原 VM 核对并安装。
Type 队列与 luna 调用入口均使用该接线；参考执行共用描述检查和安装规则。
这消除了“已有 preparation 但外层仍不能保存等待状态”的一处断点；普通源码检查和
publisher 内部步骤尚未因此获得恢复能力。范围及验证见
[定义准备的宿主交接](elaboration-state-and-publication.md#定义准备也能离开源-invocation-驱动)。

方法选择也已在原调用准备中保存 continuation：选择请求可以离开 VM，返回后继续
核验 member、参数、阶段和公开签名，不重跑前置约束及默认值准备。普通实参仍留在
原调用位置；直接调用与方法值共享同一选择准入。详见
[方法准备的恢复边界](elaboration-state-and-publication.md#方法选择保留调用准备进度)。
剩余同步类型观察及普通 HIR 检查继续保留为缺口，不因增加宿主消息类别宣称完成。

CMTT 存储服务另修复了入口快照使用空声明世界的问题：真实 sysroot 中 Object
携带用户名义载荷时不再因此误报 phase 错误。读取、检查点和入口复制共同结算准备／
执行额度，失败与 unwind 不退款未知工作；快照按入口批次完成后交付。详见
[存储观察的世界与预算](values-staging-and-publication.md#存储观察与入口快照的世界和预算)。
存储世界的元数据捕获也已改为复制前预检，与完整声明捕获共用字段、约束、变体和
编码的计量规则；低额度不会先无界复制世界，再在 VM 准备时才失败。稳定世界共享
仍未接通，不将计费完整化称作缓存或跨调用复用。

## 3. 已有清理在目标架构中的位置

`03908c64` 将源码控制状态集中到 `control::Context`，降低进入／退出检查边界漏恢复
字段的风险；它没有把整个 Checker 变成可挂起 Region。`b66a9ab3` 和 `4937d332`
则修正了参数域的一个共同边界：callee 尚未供应的输入不同于 caller 已有效的泛型值。
依赖在替换前观察，值在 caller 环境通过普通表达式规则检查；后者还删除了 NF 符号
子句参数自行查表、直接要求类型相等的平行检查分支。

输入事实也已从 Checker 的六个平行字段迁入 `input_facts::State`。试探回滚和失败
分支恢复整体事实；词法退出只移除内部绑定，不能恢复已经失效的外层值。原试探
漏存 required scope、循环退出只裁剪 partial 的边界已经收拢，CMTT／普通输入仍
分别保留观察与供应的含义。具体范围见
[输入事实的回滚与词法边界](elaboration-state-and-publication.md#已接入输入事实有共同的回滚与词法边界)。
这不是已经完成源码检查 Region，也没有把所有 Checker 字段重新打包成快照。

继续核对普通调用的检查边界时，复现了成功探测仍提交生成器存储写入的问题。
`checking_attempts` 现明确区分提交、只探测与保留 quote 产物：失败／探测恢复原有
cell 身份、内容和 ownership，成功 quote 仍保留构造效果。内部回退不重新执行
整包域准入；VM 的受验证观察入口仍保留。初版完整声明目录快照导致一个既有方法
传输用例耗尽预算，已按保存的表示规模处理，而非提高额度或跳过既有源值检查。
具体职责与剩余边界见
[探测与产物保留](elaboration-state-and-publication.md#已接入探测提交与-quote-产物保留分开)。
最终相关检查 `/tmp/luna-tools-gwnms93_/result.json` 退出 0，1,679 项通过、
2 项既有 ignored；普通调用的待定 Type 域仍未接通。

这几步是 M1–M3 的前置清理，不是整条恢复架构完成。后续主干仍是
[待定域的普通调用](refactoring-plan.md#下一条纵向交付待定域的普通调用)，并用真实
sysroot 的 meta.List／builder／comptime 方法作消费者。不能继续只在已知域的静态
分支增加观察技巧，避开真实 Type 生产者、执行顺序和前端区域所有权。

资源与库线仍保留独立缺口。运行时文本接口须以真实 sysroot、标准资源检查及正常
调用贯通，不能用手拼片段、直降 builtin 或跳过来源检查证明库可用。

## 4. 本次证据与限制

工作目录 `/home/fin/luna`；所有检查通过 `development/check.py --timeout 900 -- test`。
工作区包含作者原有的 `program_verification/src/moves/effects.rs` 修改，未改写或提交；
因此这些是工作区验证，不宣称为纯 HEAD 验收。

| 检查参数 | 实际结果 | 本机证据 |
| --- | --- | --- |
| `-p elaboration --lib` | 退出 0，189 项通过 | `/tmp/luna-tools-rplpwgyc/result.json` |
| `-p luna --test trait_domain_substitution --test trait_obligations --test trait_phases --test normal_form_domains --test cmtt_type_inputs --test cmtt_methods --no-fail-fast` | 退出 0，270 项通过 | `/tmp/luna-tools-9ofq6e01/result.json` |
| `-p luna --test comptime_pointer_phase --test comptime_storage_domains --test comptime_receivers --no-fail-fast` | 退出 0，22 项通过 | `/tmp/luna-tools-psupmd3g/result.json` |

这些结果支持所述域检查和阶段边界，不是完整语言、LLVM、性能或跨编辑缓存验收。
原有两个普通调用待定正例仍 ignored；PTC 联合模型、runtime TypeRef 与文本来源在本次
没有新增动态验收。报告原文、作者其他改动及正式 docs 保持不动。
