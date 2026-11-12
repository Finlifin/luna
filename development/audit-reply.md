# 对 2026-11-11 晚报的回复

核对对象：`ai_dev_logs/comptime_type_vm_divergence_report_2026-11-11_evening.md`。
报告主体基线是 `a00e221f`，附录推进至 `bfe8221c`；本回复在 2026-11-12
整体核对至 `4937d332`，后续局部迁移与旧问题复查推进至 `b360822f`。
保留原报告，不把不同基线的事实混成一次测量。
formatter 由另一位开发者负责，本次未修改 `compiler/fmt` 及其依赖。

在 `6a564ae5` 上再次执行两个原有 ignored 正例，仍在待定签名与元素域处失败，
未抵达通用源执行恢复。现将状态协议第 4 节收敛为当前生产职责、交接目标与验收，
删除被后续迁移取代的过程记录；历史验证与本轮失败证据分别保留。新明确的准入
边界是：callee 形成期间的 Type 生产者由调用者源区域执行，不能先进入尚未形成
签名的目标 invocation。接口、调用准备、已进入 activation 与可执行主体是不同
完成点；详见[父区域与 callee](elaboration-state-and-publication.md#父区域与待形成-callee-分开)
和[本轮复核](elaboration-state-and-publication.md#纵向交付与验证证据)。本次未修改
编译器，也没有据文档整理宣称待定输入已经可用。

当前交付应按下面三条边界阅读，不能把前一条的完成推及后两条：

| 边界 | 已有证据与剩余工作 |
| --- | --- |
| 输入供应路由 | `fc166abb`、`691f9a02`、`d6950e39` 已让普通实参、声明子句、Type 子句和相关 CMTT 提示消费者使用同一入口描述与游标，并删除独立的 `type_input_slot`。此层只决定供应到哪个槽，不证明值已取得或域已满足。 |
| callable 公开接口传送 | `b360822f` 交付接口设计和真实 sysroot 复现。别名、选择和高阶返回之后，位置展开对照可执行，具名完整供应仍失败。公开入口、提供者与接收世界证据尚未随 callable 接口完整传送。 |
| 源码检查与执行恢复 | 普通块和输入依赖已保存显式检查后继，但尚不能保留同一次源调用的活动执行前缀并恢复待定签名／域检查。两个既有合法正例仍未接通；不以任务栈或供应游标代替这项验收。 |

前两条的具体源码链、命令及结果见
[动态 NF 具名入口](#动态-nf-的具名入口已复现先补参数协议载体)与
[接口传送设计](callable-input-interfaces.md)；第三条见
[源码检查与执行交接](elaboration-state-and-publication.md)。这里汇总的是既有提交
及其执行证据，本次没有重新运行这些检查，也不将 formatter 的独立进展计入语义修复。

此前复查六项旧问题，2 项通过、4 项失败：动态名义子句值与嵌套 Type 转发已通过；
一项失败是 TypeContractUnresolved 与旧 InvalidCallPlan 断言的差异，另三项仍在
调用期输入或程序后置条件处失败。未修改测试预期，也未将这批局部通过计作架构
完成。命令、位置及证据见[当前准入边界](comptime-value-admission.md#当前尚未完成的边界)。
该文的 VM／声明域接线说明已按当前职责压缩，移除重复中间日志，并纠正 fresh
phase 检查另开默认额度的过时描述；历史版本仍可从 Git 查阅。

截至 `bcbcd0bc`，普通块检查已经使用显式帧，普通调用参数的域依赖和默认值引用
也已使用显式任务栈。后续 contextual 块亦接入同一块驱动，删除其独立语句循环；
普通语句与待定尾值保存不同结果种类，共用帧栈的词法退出和受检子结果交接。
块目标也已按共享预算取得区域私有副本，在嵌套帧间共享；不再借用调用者的目标
上下文。普通与 contextual if 现也在同一栈保留条件、then 结果和分支事实退出，
删除原递归检查路径；普通 if 复用原交付准备，并保留 inline／callee／effect 边界
及区域拥有的 else 目标。部分结构输入的块／if 现已接入同一驱动；optional／error
分支的探测位置、默认类型模式与当前 Attempt 由帧保留。错误先经过内部词法退出，
再由最近的 Probe 恢复存储与检查状态；只有缺少上下文允许继续搜索，其余错误
继续传播。when／match 共用探测协议，子分支仍经同步适配器检查。
其它子规则与活动推断世界尚未迁移；这项内部回退不等于恢复已执行的源 activation。
这些任务仍同步调用其余源码子检查与 required invocation；不具备
一般源 activation 的挂起恢复。完整 callable 签名和待定 collector 域的两个正例
仍未实现，不能把任务栈本身当作解除这两项限制的证据。具体交接与验证见
[普通块帧](elaboration-state-and-publication.md#检查帧与探测)
及[输入依赖任务](elaboration-state-and-publication.md#输入路由与域依赖)。

R12 发布边界的后续核对还发现 required invocation 向 Checker 回传多条生成定义时
可能部分写入：后项超预算，早先描述与构造器权限已可见，布局索引尚未同步。
现已把全部可失败的传输预检放在接收方写入之前；原逻辑复现与 400 项针对性回归
见[整批定义接收](elaboration-state-and-publication.md#已接入required-invocation-的整批定义接收)。
这修复跨层接收原子性，尚不代表完整发布事务或调用期 Type 恢复已经完成。

沿同一发布原则，VM 的 contextual 域准备现也先完成私有草稿，再交付执行表：
后项错误或预算中断不再留下前项准备结果，NF 输入切换不再提前清空旧表。
实际失败复现、重试与双引擎／真实 sysroot 验证见
[域准备的完整结果交付](comptime-value-admission.md#域准备的完整结果交付)。
这是现有域检查的局部原子性交付，不是源 activation 恢复或 NormalFormArgument 退役。

沿同一交付链继续检查，发现 Checker 已接收的晚生成普通 enum／newtype 仍可能被
最终 NIR 收集遗漏。现统一三类描述的补收与身份去重；真实 sysroot 的函数体局部
newtype 构造可复现旧路径在 IIR 实例化时报缺少声明。详见
[晚生成描述交付](elaboration-state-and-publication.md#已接入晚生成描述交付给后续-ir)。
局部 Type 常量的 enum 成员曾因只接受语法类型头而误入字段或 Type 方法查询；
现与普通／代码方法、关联项共享接收者域判断，保持命名空间优先级与普通值引用。
这修复已完成 Type 的成员路由，尚不使待决身份提前完成。
探测回滚的生成定义所有权尚未得到完整动态验证，不能据收集遗漏反推其正确或错误。

完整包的声明世界仍有一条平行构造路径留在纯度分析：自建 Context 和 impl 索引
绕开共同计量，布局复制未受调用者额度约束，逐项插入还重复使用初始额度。现已
转接同一有界构造入口，保留本次分析的不可变缓存。完整计量暴露的真实 sysroot
回归，进一步定位为资源子分析反复读取整包参数 sort 目录；现共享父分析的声明
目录，各子分析仍独立建立行为证据，未扩大额度。对照与限制见
[纯度声明世界](solvers-resources-and-lowering.md#71-纯度分析复用完整包的有界声明构造)。
此修复不放宽 Copy 或 P3，也不代表泛型默认值缺口已经解决。

同一审查进一步发现，flow 的三处声明域读取还把预算失败压成 `InvalidSignature`。
现已共用有界域目录，保留结构错误与 `Unknown(BudgetExhausted)` 的区别；
普通调用、trait 契约和捕获值沿同一规则消费声明证据。原实现的重复扫描与误诊
反例见[程序契约声明域](solvers-resources-and-lowering.md#72-程序契约的声明域与失败分类)。

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

后续已删除 `normal_value_argument_at` 的聚合递归：类型参数值的元组、列表
不再逐项建立 required 求值，而由普通表达式检查及一个完整 required 根构造；
泛型输入继续走已有词法捕获，结果使用普通值编码。记录和其他聚合不需要另加
源语法分支。此退役不代表 Role、NormalFormArgument 或动态 NF 收集器均已移除。

工作目录 `/home/fin/luna`，本阶段执行以下针对性检查：

- 修改前新的 sysroot comptime 对照通过，`/tmp/luna-tools-tmjzd7s_/result.json`
  退出 0；覆盖合法聚合、不同值索引的拒绝及 Type 值的 opaque 头身份。
- 修改后 `python3 -B development/check.py --timeout 900 -- test -p luna --test normal_value_arguments
  --test normal_form_domains --test nominal_value_parameters --test captured_input_observations --no-fail-fast`：
  `/tmp/luna-tools-c38s7ksb/result.json` 退出 101。305 项通过、9 项失败；九项名称
  与诊断逐项等于上一阶段的 `/tmp/luna-tools-g4rr4ng7/stdout.log`。
- 扩展至 elaboration 单元检查以及普通输入、required scope、聚合与真实 sysroot：
  `/tmp/luna-tools-eruh0s3z/result.json` 退出 101，384 项通过、2 项失败。其中新增
  嵌套错误索引已被正确拒绝，测试初稿只接受 type mismatch 文本，漏掉 unresolved
  type equality；现同时识别这两类域错误，未修改合法／非法输入预期。
- 另一项 `descriptor_contracts_do_not_infer_factory_injectivity` 要求错误包含 unknown，
  当前诊断给出 not-entailed 模型。仅将本阶段唯一生产文件暂还原到 `3298e6b5`，
  单独执行该用例，`/tmp/luna-tools-w11d6bl8/result.json` 退出 101，完整诊断相同。
  随后逐字节恢复修改，没有改变旧用例的断言。
- 修正新增诊断断言后，`python3 -B development/check.py --timeout 900 -- test -p luna
  --test normal_value_arguments -- --nocapture`：`/tmp/luna-tools-ig9ca8fk/result.json`
  退出 0。两组矩阵覆盖 tuple/list/record、相互嵌套及 Type 值叶子的正反例。
  直接消费者 `--test trait_domain_substitution --test trait_obligations --no-fail-fast`
  另有 8 项通过，`/tmp/luna-tools-84qdjn2j/result.json` 退出 0。

最初 runtime 对照在修改前已经失败于 IIR 的 `layout requires a nominal type`
（`/tmp/luna-tools-47m1uo2t/result.json`）。源码保存在
[聚合索引布局复现](repros/aggregate-index-layout.fl)；comptime 路径通过不能证明
runtime 布局消费者完成。迁移后以 `python3 -B development/check.py --timeout 900 -- run
-p luna -- dump development/repros/aggregate-index-layout.fl --emit iir --entry run
--sysroot library --build-target x86_64 linux 64` 复跑，
`/tmp/luna-tools-opv69_qw/result.json` 退出 1，仍为相同布局错误。
该布局失败现已沿消费者修复。`value_admission` 原本就允许布局回调提供已登记的
积字段域，而 materialization、函数声明参数与 trait 参数传入的包查询只接受
Term::Nominal。新增 `query_value_fields` 接通接收世界已有的表示登记和递归参数
准入；名义类型的内部参数也使用同一查询，避免只修顶层后在嵌套处再次失败。
查找与字段复制消耗共享预算。仅名义布局的入口保持原限制，没有按 Tuple 名称
识别值、展开 NF 头或跳过字段载荷检查。现有 Role 登记机制本身仍待后续迁移。

新的低层对照使用任意声明 ID，覆盖直接 callable 参数与嵌套名义索引、字段数量、
字段域、u8 越界、错误 variant、缺失或不匹配的表示登记、零预算及后续重试。
源码矩阵同时执行 comptime 和具体 IIR，实际构造并读取嵌套 payload 字段，包含
元组、列表、记录、相互嵌套和保留 opaque NF 头的 Type 值索引。

本轮同一目录执行
`python3 -B development/check.py --timeout 900 -- test -p nir -p materialization -p luna --lib --test normal_inputs --test nominal_parameters --test dependent_admission --test trait_parameters --test value_admission --test scoped_dispatch_inputs --test expansion_boundary --test normal_value_arguments --test sysroot_preludes --no-fail-fast`，
`/tmp/luna-tools-8_pgihp9/result.json` 退出 0，314 项通过。随后原 CLI dump 命令复跑，
`/tmp/luna-tools-fze4bn2w/result.json` 退出 0，产物包含 Holder 与 Packet 的具体实例
及原 normal-form 头登记。初稿单元用例使用了不存在的 Integer::From 构造，改用
已有 from_u128 后完成上述验证；没有放宽值域拒绝预期。
本阶段不运行全 workspace 或 LLVM 原生验收；动态父输入及源码挂起仍未完成。

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
[输入事实的回滚与词法边界](elaboration-state-and-publication.md#控制与输入事实)。
这不是已经完成源码检查 Region，也没有把所有 Checker 字段重新打包成快照。

继续核对普通调用的检查边界时，复现了成功探测仍提交生成器存储写入的问题。
`checking_attempts` 现明确区分提交、只探测与保留 quote 产物：失败／探测恢复原有
cell 身份、内容和 ownership，成功 quote 仍保留构造效果。内部回退不重新执行
整包域准入；VM 的受验证观察入口仍保留。初版完整声明目录快照导致一个既有方法
传输用例耗尽预算，已按保存的表示规模处理，而非提高额度或跳过既有源值检查。
具体职责与剩余边界见
[探测与产物保留](elaboration-state-and-publication.md#检查帧与探测)。
最终相关检查 `/tmp/luna-tools-gwnms93_/result.json` 退出 0，1,679 项通过、
2 项既有 ignored；普通调用的待定 Type 域仍未接通。

声明输入准备循环随后迁入独立 Progress，集中保存 producer、身份、绑定与调度位置。
越序／未完成计划不能发布，身份复制失败不消费当前位置；观察到身份仍保留原 producer。
这是生产准备路径的状态清理，尚未接通 HIR 挂起或实际 Type 结果回填。范围见
[声明输入准备进度](elaboration-state-and-publication.md#输入路由与域依赖)。

在 `b613147b` 再次显式执行两个 ignored 正例，仍分别失败于待定 callable 签名和
列表缺少元素上下文，证据 `/tmp/luna-tools-qx183hcc/result.json`（退出 101）。
进一步核对发现，required 准备必须先取得完整根 Expr，而 fragment 驱动只借用
已建字节码帧；现有外部请求本身不能跨越这道前端边界。已补定
[源 activation 的交接设计](elaboration-state-and-publication.md#源检查与执行的交接目标)：
HIR 与检查 continuation 归前端，实际存储和执行位置归原 invocation；后继受检片段
通过移动交接绑定，并保留原控制出口。设计明确了完整根入口的迁移、身份／许可、
尚未取得的值、域响应及冻结边界；这部分尚未实现，没有把失败测试改成拒绝预期。

在 `363d477a` 继续核对生产源码，明确了这条接线的四个实施障碍：声明输入的串行
Progress 尚未分开检查就绪与执行次序；块作用域仍由同步闭包持有；通用源片段尚未
交付新绑定与作用域接口；字节码编译只接受本段登记的循环
目标，不能直接承接外部 break／continue。相应的规则 continuation、词法状态借用、
LocalId 交接、外部控制出口及实际 Type receipt 的消费已写入
[实施细化](elaboration-state-and-publication.md#所有权与规则推进)。
明确要求同步入口驱动同一检查规则，避免增加第三套局部求值器。本轮为静态源码核对
和设计修改，没有新增 Rust API、运行测试或宣称两个 ignored 正例已恢复。

这几步是 M1–M3 的前置清理，不是整条恢复架构完成。后续主干仍是
[待定域的普通调用](refactoring-plan.md#下一条纵向交付待定域的普通调用)，并用真实
sysroot 的 meta.List／builder／comptime 方法作消费者。不能继续只在已知域的静态
分支增加观察技巧，避开真实 Type 生产者、执行顺序和前端区域所有权。

后续已把 Progress 的检查完成次序与源码执行 schedule 分开，并接入现有域依赖
检查和收集器路径。提前检查的显式 producer 直接移入同一个 owner，身份观察与
局部域在记录成功后交付；不再到源位置后克隆该 producer、重复观察并把检查顺序
当成执行顺序。默认值和 packet preparations 仍使用原调度位置，dump 分别列出
checked-order 与 schedule。这是现有生产准备逻辑的迁移，没有新增另一套表达式
求值器或将同步回调标成可恢复任务。

3 项局部检查与 222 项真实消费者集成检查通过；新增用例检查后序 Type 提前检查时
副作用顺序、保存 callable 后不重放及早退取消后继求值。2 项普通调用 Type 等待
正例仍 ignored，本轮没有重跑或解除。完整命令与证据见
[声明输入准备](elaboration-state-and-publication.md#输入路由与域依赖)。
仍须接通前端 continuation、原 invocation 的实际 Type 响应及一次性执行 receipt；
当前 checked-prefix 只表示准备前缀，绝非已经执行的前缀。

继续核对时发现先前设计文字遗漏了现有导出路径：`InputFinish::Preparation` 本来
就在成功后将 exports 追加到 imported 再归还环境，不能说 VM 完全不能导出局部值。
已纠正这一表述，并把已有生产路径迁入帧自身的绑定交接：执行前解析 LocalId、检查
冲突及预算，正常完成且内部作用域闭合后整体交付。错域检查仍由原消费者负责；
这里验证绑定存在和交接状态，不把寄存器存在当成类型证明。

新接口消除了将导出伪装成导入的约定，也避免把寄存器表与实际 Frame 分开持有。
错误和非局部控制只归还原输入及其真实写入，不发布一半输出；源作用域、外部控制
出口和 Type 响应仍待前端接线，未将这次内部迁移写成完整 source activation。
生产入口、边界及证据见
[帧绑定导出](elaboration-state-and-publication.md#required-世界与绑定交接)。
局部 7 项通过；随后 VM 与调用方检查 755 项通过、2 项既有 ignored，退出 0，
证据 `/tmp/luna-tools-r52uh6us/result.json`，其中包含首轮局部用例。没有执行全
workspace 或解除普通调用 Type 等待的 ignore，也没有改动另一代理的 formatter。

required 世界准备中的另一套结构遍历已退役：其函数、方法和关联值依赖发现改用
NIR 公共遍历，required 根／关联初始化器／生成函数／普通延迟区域／live 闭包的
调用方都传入实际预算并处理失败。成员依赖在整次扫描成功后合并；原先不执行代码的
发现职责保持不变，retained pattern evidence 不被误当成已获准执行。原入口对同一
根的多次自由绑定扫描也合并为一次，不扩大外围捕获。具体阶段边界见
[required 依赖发现](elaboration-state-and-publication.md#required-世界与绑定交接)。
局部 35 项通过，随后包含 NIR／elaboration 和真实源码消费者的 1,177 项通过，
2 项既有 ignored 保留；证据 `/tmp/luna-tools-zrknan4h/result.json`，退出 0。
本次没有增加 source activation 或 Type 响应恢复能力，也未执行性能对比。

资源与库线仍保留独立缺口。运行时文本接口须以真实 sysroot、标准资源检查及正常
调用贯通，不能用手拼片段、直降 builtin 或跳过来源检查证明库可用。

报告 6.3 指出的键协议旧名称回退已移除：选择器按受信任的 trait 身份选择 `eq`／
`hash`，不再优先查找带 `comptime_` 前缀的成员。真实 sysroot 检查证明旧名称不能
抢占实现、补齐缺失成员或绕过阶段检查；原比较 fixture 改为独立 runtime／comptime
trait。相关缓存调用、预算失败恢复与协议检查共 15 项通过，证据
`/tmp/luna-tools-e2i_h2fz/result.json`。这不补齐 Type 的 Hash 实现或改变缓存准入范围。

报告 6.2 的重复绑定已在 build 落实一处清理：复用 `meta.Object` 的身份和公开
`get` 方法，删除 Object／object_get 的包内重复绑定，保持三个 build 输入的原始
声明与包来源。原源归属 fixture 仅提供原语，现补入真实 meta／attrs 依赖以及
builtin 绑定和比较定义；不为测试臆造 target，也不模拟 Object 查询。
`build_context` 和 `build_metadata` 共 47 项通过，包含真实 sysroot 的 O0／O3
执行、错域／缺失、别名来源和驻留配置失效恢复。证据
`/tmp/luna-tools-2m6haxur/result.json`（退出 0），完整命令及范围见
[库接线现状](libraries-and-delivery.md#9-当前生产入口与迁移接线)。随后 attrs 也改为
导入公开 `meta.Object`，基础域原始绑定与服务操作分开，库中 Object 与 object_get
各只保留一个原始绑定。显式加载 attrs 的 builder／List 调用方已补依赖；build 缓存
夹具此前遗漏的 meta 依赖一并补齐，与包来源检查共用真实库装配。

去重范围按完整契约判断，不能把报告的同名符号计数直接等同于冗余声明：runtime／
comptime 阶段、usize／isize 输入及整数转换方向均属于函数签名。已在
[绑定去重边界](libraries-and-delivery.md#绑定去重必须保留阶段和函数契约)逐项核对，
并修正 `library/STRUCTURE.md` 中“meta 文本接口复用 runtime 绑定”的过时描述。
runtime str 无类型操作及资源来源、数学包装复用仍未完成。

本轮验证均在 `/home/fin/luna`，前缀为
`python3 -B development/check.py --timeout 900 -- test -p luna`：

| 后续检查参数 | 实际结果 | 本机证据 |
| --- | --- | --- |
| `--test library_attributes --test builder_invocations --test build_context --no-fail-fast` | builder 86、build 35 通过；属性 2 项命令入口错误，后续已改为 dump 并重验 | `/tmp/luna-tools-_b093ekm/result.json`，退出 101 |
| `--test library_attributes --test trait_phases --test derivation --test compilation_cache --test library_meta_objects --no-fail-fast` | 属性 7、阶段 73、Object 34 通过；缓存 3 项依赖缺失后续已修复；derive 76 通过、1 项旧 comptime 指针接受例失败 | `/tmp/luna-tools-j4ghb7qr/result.json`，退出 101 |
| `--test compilation_cache --test build_context --no-fail-fast` | 46 项通过，保持缓存失效与缺失配置恢复检查 | `/tmp/luna-tools-zkil21o6/result.json`，退出 0 |
| `--test comptime_list_operations --test frontend_cli --test elaboration --no-fail-fast -- --exact ordinary_package_uses_public_list_operations_without_builtin_access sysroot_host_intrinsic_defaults_build_real_math_and_allow_explicit_override unsupported_abi_attributes_cannot_publish_default_abi_functions` | 3 项通过 | `/tmp/luna-tools-diko8kqp/result.json`，退出 0 |

derive 的旧例只使用独立源码，不依赖迁移的库：它执行 `storage.ref`／`pointer.*`，
实际报错为 comptime 禁止指针操作，符合作者明确规则及 `docs/comptime/index.md`
开头的语义边界。后续单独将该例迁为值引用更新：方法修改 `self.flag`，调用方读取
原存储确认写回，再将完成的 bool 捕获进生成的 runtime 成员；另保留明确的源指针
拒绝例。没有删除原有临时值／生成代码的阶段隔离检查目的，也没有恢复 comptime
指针。`--test derivation --no-fail-fast` 78 项通过，证据
`/tmp/luna-tools-bhk7ztwk/result.json`（退出 0，工作目录与命令前缀同上）。

### 地址键结论的纠正与检查消除器清理

11 月 11 日晚报 2.3 将程序验证层的地址键概括为“使有序集合的遍历顺序直接取决于
宿主堆内存分配器地址”，这个结论不能由转型计数推出。核对当前源码及报告基线
`a00e221f` 的 `memory.rs`：expression_results、callable_results、aggregate 表和
closure_nodes 以地址作 HashMap 查找键；局部代码候选使用首次访问时的
`local_code.len()` 序号，closure_nodes 保存这些序号。stable_writes 的地址集合用于
成员检查。这不是本轮修复后的性质，报告基线就已有确定性候选序号及相关注释。

不能据此反向宣称全部地址键都安全：树的移动、clone、COW、释放及跨查询复用仍需要
逐项检查生命周期；持久身份不能直接使用地址。修复应针对实际消费者和失效边界，
不以替换全部 `as usize` 的数量作为架构完成度。

核对中实际清理的是 `optimization.rs` 的重复递归遍历：现在扫描、闭包分离和改写
都调用 NIR 公共有界遍历，删除独立的表达式／模式遍历。同时修正包级入口在计量前
clone 整个函数的问题；它与 purity 共用复制前检查，没有待处理谓词时也不再 clone。
保留条件求值、每次访问均需证明、共享闭包隔离和整函数失败回退。proof site 仅用于
已分离私有树上的本次事务，没有冒充跨树稳定身份。

针对性检查为首轮 6 项、纯度／调用事实 73 项，以及所有直接调用检查消除器的
execution 用例 26 项，均退出 0；前后两轮 execution 用例有重叠，不累加为独立覆盖。
新增回归检查函数元数据的复制计量与低预算后的新尝试恢复。命令、日志和范围见
[检查消除的私有事务](solvers-resources-and-lowering.md#41-可选检查消除的私有事务)。
这不解决普通调用的待定 Type 域、运行时文本来源或完整统一 VM；未修改 formatter、
报告原文及正式 docs，既有 moves/effects.rs 工作区改动仍单独保留。

### 有界 visitor 仍需限制准备成本

沿用共享 visitor 并不自动保证遍历准备有界。实际核对发现，宽节点此前会先将全部
子节点加入 Vec，之后才按出栈节点扣费；空捕获、被过滤的默认实参和空 handler
还可能产生不受节点数量约束的扫描。已给表达式读／改写的工作栈加入剩余额度检查，
并对上述结构项计量。完全排除的作用域直接跳过，不再扫描整张列表。

这修复的是共享遍历的具体成本漏洞，不将其描述成完整 VM 或全局分配预算。节点
剪枝、结构访问顺序和改写失败后丢弃私有结果的契约保持；复制拥有载荷与闭包 COW
仍由调用者的复制前检查负责。边界与验证见
[共享遍历的准备成本](solvers-resources-and-lowering.md#42-共享遍历的准备成本)。

### 源声明查找与活动检查状态分开

继续核对 required 依赖准备和成员消费者，确认多处为一个完整 DeclarationId
重复扫描全部 HIR owner。现已迁到与不可变源码包绑定的 SourceDeclarations，
构建时使用 resolver 身份及语义包映射，排除 import 语法 owner，拒绝重复完整身份。
调用、成员、CMTT 和 NF 等消费者共用查找；索引不保存候选选择、活动域或 VM 值。
具体复杂度、不同视图的所有权及验证见
[源声明身份查询](dependency-observations.md#源声明身份查询不重复扫描包)。

这项迁移减少真实消费者的全包查询，不是细粒度增量 Query、前端 continuation 或
动态 NF 具名参数支持已经完成；这些缺口仍按前述边界保留。

### 普通检查的目标交付开始使用显式后继

`expr_context_adapted` 已由 `delivery.rs` 的共同规则和同步驱动器替换。可选包装与
错误结果适配现在保存子检查完成后要做的工作，并消费一次受检结果；这是真实普通
检查入口的迁移，不是另加一个仅供手工构造的 request 接口。新增真实 sysroot 用例
核对嵌套层次、提前 return、错误域拒绝和接收者求值次数，两条解释路径均通过。

这一阶段仍是同步驱动：一般子表达式、候选探测与块作用域还没有统一挂起／恢复。
没有解决普通 callable 的两个待定 Type 域正例，也没有把活动检查状态放入稳定世界。
具体已交付范围、复制预算和执行证据见
[目标域适配后继](elaboration-state-and-publication.md#检查帧与探测)。

扩大回归时发现错误结果值作为 trait 参数的一个既有用例失败。在关闭本次 delivery
接线、恢复 `a1aab74a` 的 program.rs 后仍得到相同 Type interpretation 错误；详细
命令和对照证据在上述链接。该次检查没有将它计入通过项，也没有改变测试预期；
后续修复见下节。

### 错误结果值参数的恒等传送修复

具体失败并非 trait 值参数语义不成立，而是 VM 的 Embed 即使面对相同源域与目标域，
也强行拆解具体错误标签。诊断跟踪确认输入为已有声明域授权的 AbstractValue，
类型为 `!bool bool`。现已让恒等交付保持原值并检查当前 VM 的授权；非恒等转换遇到
抽象标签报告未决，仍不制造成功分支。共享入口同时服务树解释和字节码。
实现边界见 [抽象错误结果交付](values-staging-and-publication.md#错误结果的恒等交付不观察抽象标签)。

局部命令（工作目录 `/home/fin/luna`）
`python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib error_results:: --no-fail-fast`
三项通过，`/tmp/luna-tools-jml75ops/result.json` 退出 0。修复前诊断复现记录为
`/tmp/luna-tools-l8xmhm90/result.json`，退出 101；临时跟踪代码已删除。

完整受影响检查使用
`python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --test abstract_values --test normal_values --test immutable_normal_inputs -p luna --test error_surface --test error_result_flow --test contextual_delivery --no-fail-fast`，
870 项通过，`/tmp/luna-tools-z8avn1z9/result.json` 退出 0；包含上述三项局部检查，
不重复累加覆盖。原失败案例在精简 fixture 与真实 sysroot 下都通过，未解除或新增
ignore。这里仍使用保留作者已有修改的工作区，不是纯 HEAD 或全 workspace 验收。

### 块检查进度与词法快照的复制边界

普通块和待定输入块已将受检语句前缀、下一个子检查、diverges 和分期模式证据移入
共同的 Progress。外层词法环境由 LexicalScope 持有，退出继续保留外层值的失效，
不把词法退出当成候选回滚。快照表项、局部 Term 复制和语句推进现在都先扣除预算，
避免在计量前复制全部可见局部域。源码用例检查嵌套写回、私有绑定拒绝及提前退出的
实际调用序列，普通块和待定输入块使用同一规则。

这仍不是可挂起的完整前端区域：子检查和尾值检查同步，活动词法状态尚在 Checker，
没有跨 VM Type 响应、取消协议或新的稳定世界载荷。边界及证据见
[块的语句进度与词法退出责任](elaboration-state-and-publication.md#检查帧与探测)。

### required 声明目录使用调用者预算

沿源 activation 的准备路径发现：`RequiredWorld::new` 每次以独立的 PTC 默认预算
调用 DeclarationSnapshot.prepare，实际目录建立成本没有从 required_steps 扣除。
执行世界与资源证据世界都会经过这个入口，因此仅计量后续 VM 调用不构成完整预算。
现已由调用者传入额度，成功和失败均结算实际准备成本，目录完整建立后才发布世界。
已准备目录的读取和同一世界的代码缓存复用保持；没有把资源证据与执行声明混为一份。

新检查覆盖零预算、精确预算及少一步、同一额度连续准备两个世界、非法重复声明和
失败后的新尝试。包组装与其上游所有复制的成本不是本次修复的覆盖范围；本次只关闭
目录准备自行领取默认额度的漏洞，也没有接通源主体挂起／恢复。

局部验证（工作目录 `/home/fin/luna`）：
`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib program::required::prepared::tests --test invocation_budget --no-fail-fast`，
24 项 prepared 检查通过，`/tmp/luna-tools-hsjprfb_/result.json` 退出 0；同一命令的
invocation_budget 被名字过滤器全部筛去，不计为已验证，其后另行运行无该过滤器的检查。

无名字过滤器的相关回归：
`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib --test invocation_budget -p luna --test required_input_scopes --test staged_invocation_reuse --test inline_pattern_observation --test library_comptime_comparison --test builder_invocations --test ordinary_callable_types --test ordinary_clause_collectors --no-fail-fast`，
465 项通过、2 项仍 ignored，`/tmp/luna-tools-y4fb1689/result.json` 退出 0。
这包含真实 sysroot 的内置包比较、builder 与普通调用回归，不表示两个待决域正例已完成。
两个修改的 Rust 文件经 scoped rustfmt 检查通过，
`/tmp/luna-tools-_oviwmxx/result.json` 退出 0。结果来自保留作者及 formatter agent
已有修改的工作区；本次未修改或纳入这些文件，没有运行全 workspace 检查。

### 待决域主线：补清声明与主体的发布边界

沿两项主线正例追踪发现，当前函数收集必须先取得完整 NIR Expr，VM 准入也直接
取得完整 Function.body。增加 VM 请求本身不能让尚未完成的源主体进入调用。
已在[声明接口与可执行主体](elaboration-state-and-publication.md#声明接口与主体发布目标)
区分稳定接口／源码模板、活动源任务、完成的可执行产物，并将函数收集和主体取得
列为同一迁移范围。定义检查、normalization 中的必需计算与 invocation 执行许可
分别保持；不能用延迟所有函数检查、假主体或直接解释 HIR 代替接线。
这部分是实施设计，尚无模板发布或主体请求 API。

本轮仍显式运行了两个 ignored 正例，工作目录 `/home/fin/luna`：
`python3 -B development/check.py --timeout 900 -- test -p luna --test ordinary_callable_types --test ordinary_clause_collectors --no-fail-fast -- --ignored`。
`/tmp/luna-tools-q2vw7una/result.json` 退出 101；依赖签名仍报
`comptime callable signature still depends on an invocation-time clause input`，
待决列表域仍报 `list literal requires an element type context`。
未改动用例、ignore 或语义预期；预算修复及回归通过不能替代这一缺口的实现。

### 主体检查入口合并，尚未接通待决主体

普通包收集原先经 function_body 检查模板／效应上下文，而 required 依赖另用 expr
检查主体，并重复组装参数及 Function 元数据。现由两者共同调用
`function_bodies.rs::check_source_function`：普通 Type 参数的 bound 身份、前后置条件、
入口快照、主体与声明要求使用同一规则。required 仍只接收真正的 comptime 函数，
不能执行 runtime 函数或 NF 头。默认值的区别保留：最终包携带定义默认值，required
的实际 call plan 携带使用的默认值，不因收集依赖而强制展开未使用默认值。

两项新增真实 sysroot 对照分别覆盖 Type 参数／列表、默认值与显式输入的效果顺序，
以及参数被写入后的入口快照、前后置契约、失败后再次调用。每份源码既进入 required
求值，也将完整包中的同一函数交给两种现有 VM 载体调用。

验证工作目录 `/home/fin/luna`，命令前缀均为
`python3 -B development/check.py --timeout 900 -- test`：

- `-p elaboration --lib -p luna --test ordinary_callable_types --test ordinary_clause_collectors --test required_input_scopes --test builder_invocations --test cmtt_methods --test effects --no-fail-fast`：
  616 项通过、2 项 ignored；`/tmp/luna-tools-7vqvsu2v/result.json` 退出 0。
- `-p luna --test required_input_scopes --test function_template_inputs --test function_template_local_definitions --test quote_functions --test cmtt_phase_boundaries --test comptime_bytecode_defaults --test effect_contracts --test inline_pattern_observation --no-fail-fast`：
  184 项通过、1 项 ignored；`/tmp/luna-tools-atlc64z4/result.json` 退出 0，包含新增对照。

两组存在重叠，不将其相加作为独立测试数。初轮编译的元组调用点／借用错误及新增
测试的 VM 值变体拼写错误已修复；上述结果来自修正后的检查。保留工作区外部修改，
未运行全 workspace。外围声明作用域仍由现有调用者建立，Type 证据仍使用保存／恢复，
共同规则仍同步交付完整主体。这减少一处必须同时迁移的重复实现，**不代表**稳定
源码模板、活动源任务、VM 主体请求或两个 ignored 待决域正例已经接通。

### VM 主体准备移到入口契约之后

沿主体交接检查发现，prepare_invocation 在入口程序契约执行前就按普通调用输入
重绑定整个主体，并可写入主体准备缓存。现由同一 activation 的 Entry → Body
转换完成这一工作；参数、签名、类型契约、结果域及入口快照仍先按原规则准入。
入口拒绝或取消不做这次主体准备，准备失败沿原调用恢复路径退出。包内主体和 owned
closure 共用此顺序，缓存命中不能替代入口契约，不会重新 begin_invocation。

两个新增用例覆盖宽主体、低预算、两种 VM 载体、包内／owned 主体、入口取消、准备
失败后重新调用及不同 Type 绑定。将两个生产文件暂还原到 f616ccc8、保留新增用例后，
`/tmp/luna-tools-am4kbplh/result.json` 退出 101：入口拒绝已产生主体缓存，宽主体的
调用在可取消的入口状态建立前耗尽预算。随后恢复修复；没有改动其他工作区文件。

最终相关回归（工作目录 `/home/fin/luna`）：
`python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib -p luna --test required_input_scopes --test staged_invocation_reuse --test invocation_value_templates --test comptime_bytecode_defaults --test effect_contracts --test invocation_host_requests --test type_invocation_sessions --test conditional_capture_cleanup --test temporary_method_cleanup --no-fail-fast`，
734 项通过，`/tmp/luna-tools-u67cj76a/result.json` 退出 0。涵盖真实 sysroot required
调用、宿主请求、代码复用及清理；未运行全 workspace 或性能基准。新增测试最初的
Function 路径拼写、连续独立调用未重新提供测试预算的问题已修正，未放宽拒绝预期。

本次只移动普通调用输入的主体重绑定。声明参数实例化和其他准入工作仍可能在入口
契约前发生；保留的仍是完整受检 NIR，不是未完成 HIR。源码模板、源任务与 VM 主体
请求仍待接通，两项待决 Type 正例没有因此获得实现。

### 真实 sysroot 下的标量探测与开放描述

原 nesting fixture 没加载 sysroot，HIR 阶段连 bool 都无法解析，实际没有验证其
声称的深层前端路径。修正公开绑定、阶段参数及具体 IIR 执行入口后，保留原来的
13 种源码、32 层深度和结果断言，暴露了三处独立问题：

- 一元协议探测关闭 numeric defaults 后缺少类型上下文，却再次以相同条件检查
  操作数；嵌套会成倍重复失败工作。现在没有新目标域或新 defaults 时直接传回
  needs-context；成功的受检操作数继续复用，不重复执行。
- 开放标量描述递归占用整个调用／控制流分派帧。标量规则现迁入独立分派函数，
  保留原预算、深度检查、类型前提及控制转移处理，没有增加线程栈或降低测试深度。
- 开放算术描述在协议存在时丢掉数字字面量的 peer context，过早默认成 i32，
  而普通检查会使用已知的 u8 结果域。两者现在共用 arithmetic_operand_hint；
  只在原普通规则可提供标量上下文的位置使用，不把任意协议 Output 当作输入域。

只将三个块检查文件暂还原至当时 HEAD、保留相同 fixture 与一元修复，
`/tmp/luna-tools-z9j59br7/result.json` 仍在 open-arithmetic/NIR 栈溢出，退出 101；
随后逐字节恢复这三个文件。拆分帧后 `/tmp/luna-tools-xbr5b14g/result.json` 暴露
u8/i32 不一致；共用上下文后，同一子案例 `/tmp/luna-tools-xqi67oat/result.json`
退出 0。此前长时停留于负号案例的运行由执行会话取消，退出 130；其
`/tmp/luna-tools-97uphhr1/result.json` 未完成落盘，不能按文件中的 running 字段
把它当作仍活跃进程或通过证据。

这些修复没有接通通用源任务挂起；开放描述与普通表达式仍有待继续合并的规则。

验证目录 `/home/fin/luna`，命令前缀为
`python3 -B development/check.py --timeout 900 -- test`：

- `-p elaboration --lib -p luna --test arithmetic_contexts --test open_functions --test nesting --no-fail-fast`：
  708 项通过，一项失败，一项 subprocess 驱动专用 ignore；
  `/tmp/luna-tools-knctk6hj/result.json` 退出 101。完整 nesting 矩阵通过，包括全部
  AST／HIR／NIR 阶段与具体执行；新 sysroot 对照还检查默认数字及未选分支的域错误。
- 唯一失败为 `later_type_inputs_keep_associated_result_identity`，报
  `342..364: invalid binder reference 0:2`。将本轮五个相关生产文件暂还原到
  `d43fef03` 后，以 `-p luna --test open_functions later_type_inputs_keep_associated_result_identity -- --exact`
  单独重跑，`/tmp/luna-tools-ei0m3_0l/result.json` 同样退出 101、诊断相同。
  文件随后逐字节恢复，未改变预期；后续根因与修复见下节。
- `-p luna --test execution source_unary_protocol -- --nocapture`：两项通过，
  `/tmp/luna-tools-ejpbjo3f/result.json` 退出 0；覆盖选定协议的 Output、操作数求值
  次数与效应边界。未运行全 workspace 或 LLVM 原生验收。

### 关联类型观察保留前提作用域，具体输入继续选择实现

上述旧失败不是参数编号在替换时错位。`given_metered` 以空 frame 验证前提，
而调用实例仍保留原定义中对后置 Type 参数的约束。即使查询主语已经是 bool，
用于证明其 trait 资格的前提仍可引用 `Bound(0, 2)`。关联观察和接口形成现在使用
世界原有 context 查询前提；观察到嵌套函数时，其局部 frame 不成为外层前提的 owner。

仅修复作用域后，原用例进一步报关联结果与 bool 的等式未决：旧观察器只要找到 given
就永远保留抽象投影。按正式 `docs/type/comptime-gradual-dependent-type.md` 的约定，
具体实例应使用选定实现的关联类型。现复用已有的带预算输入闭合性证明：输入已完成时
继续原有实现选择与关联值验证；仍抽象或闭合性证明未决时保留已经合法形成的 given
投影。后者不建立否定事实、不采用 trait 默认值，也不把预算耗尽或非法作用域当成
可以忽略的失败。实际成员来源、NF 头和不同实例的身份继续保留。

新增对照覆盖世界前提与嵌套函数 frame 的区别、具体实现值与抽象 given 的区别、
接口形成约束、失效 scope、闭合性证明 Unknown，以及缓存失败后的恢复。现有
`later_type_inputs_keep_associated_result_identity` 的 bool／u8 实例与拒绝断言保持原样。
初轮 sysroot 回归发现把闭合性 Unknown 当作整个观察失败的回归，已按上述边界修正；
没有放宽测试预期。新测试初稿重复 trait binder 身份及未区分继承前提的构造错误也已修正。

工作目录 `/home/fin/luna`，执行
`python3 -B development/check.py --timeout 900 -- test -p type_resolution --no-fail-fast`，
398 项通过，`/tmp/luna-tools-h55_2v2v/result.json` 退出 0。这仍是现有 Term／Context
架构中的查询修复，不代表新的 TypeRef 环境、待决源码 activation 或通用交织挂起完成。
formatter 由另一 agent 负责，本次未修改或纳入提交；正式 docs 与其他已有修改保持原样。

同一目录执行前端回归：
`python3 -B development/check.py --timeout 900 -- test -p luna --test open_functions --test associated_signatures --test trait_obligations --test trait_domain_substitution --test trait_query_failures --test sysroot_preludes --test ordinary_callable_types --no-fail-fast`，
443 项通过、2 项失败、1 项旧 ignore，`/tmp/luna-tools-xv6vk95h/result.json` 退出 101。
包括真实 sysroot 的列表、builder、两阶段方法、math、迭代器及相关 trait 查询；
没有运行全 workspace。两个失败为 `shipped_meta_object_preserves_type_valued_indices_in_normal_inputs`
和 `shipped_meta_object_values_dispatch_type_indexed_traits`。当时依据诊断将它们归为
调用期 Type 签名恢复缺失；后续检查实际输入计划证明此归因不准确，见下节。
仅将本次两个生产文件暂还原到 cbde5271，运行
`python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_meta_object -- --nocapture`，
`/tmp/luna-tools-zveh15kp/result.json` 同样退出 101，两个源码位置和诊断完全相同。
随后按保存的哈希逐字节恢复修复，不改这些用例的预期或 ignore。

### 名义类型输入身份观察与依赖域替换

两个 Object 旧失败的输入计划实际包含已形成的 `Key<Type, bool>`，其 NIR 节点为
NominalType。正常结果身份观察此前只接收 NormalFormType，遗漏名义类型，因而把
可静态观察的身份误报为 invocation-time 待定输入。临时记录真实计划的运行
`/tmp/luna-tools-6ijhnafi/result.json` 退出 101，定位后移除了诊断打印。

名义类型与 NF 现共用声明输入观察：依照原计划的顺序与绑定身份观察普通输入，
contextual 输入从声明参数域取得上下文。观察保留声明身份、域依赖和预算，不运行
输入生产者，不展开 NF 头，不跳过形成条件；含独立 preparation 的计划仍保守返回
不可用。执行计划原样交给实际 invocation。通用用户函数用例同时检查具名乱序、
依赖默认值、前缀副作用只发生一次，以及不同类型索引不能互换。

更强的 opaque 索引与形成条件用例另暴露 materialization 的旧固定 sort 替换：
`where T, value: T` 在 T 实例化为 Type 后，value 是普通 Type 值；形成条件分支仍用
声明时的 ComptimeValue sort 拒绝它。名义定义的字段和 capability 已使用依赖参数域
替换，现将形成条件归并到同一 DeclarationSubstitution；普通 callable 的 capability
也使用相同机制，NF 从头的参数域构造替换。输入准入、形成条件证明与 NF 身份均保留，
并未准许任意值越过 sort 或域检查。

扩展回归还发现本次初稿引入两项 staged trait 回归。prepare_method 已经在成员签名
frame 中替换契约，形成检查不能再次用空 frame 对该契约做无输入替换。现将“依赖
参数替换”和“检查已经替换的形成条件”分开：保留既有 frame 所属关系，不删除形成
检查，也不以更宽松的 sort 代替域证据。

这两个修复仍是静态身份与已有实例化路径的接线。它们不能从未知调用中产生 Type，
也不提供一般源码检查挂起、VM activation 恢复或待定 collector 域的恢复。

本阶段工作目录 `/home/fin/luna`；命令统一使用
`python3 -B development/check.py --timeout 900 -- test`：

- `-p materialization --no-fail-fast`：54 项通过，
  `/tmp/luna-tools-bawallba/result.json` 退出 0。包括三种名义定义的参数域准入、
  形成条件拒绝、预算失败及后续重新物化；后续 scoped 契约修正后的集成复核见下项。
- `-p elaboration --lib -p luna --test ordinary_callable_types --test ordinary_clause_collectors
  --test sysroot_preludes --test normal_form_domains --test normal_input_plans
  --test nominal_value_parameters --test ordinary_input_identity --no-fail-fast`：
  `/tmp/luna-tools-z4l8x82x/result.json` 退出 101，705 项通过、11 项失败、2 项原 ignore。
  其中共享观察器的预算／绑定检查、全部 sysroot prelude 和新的 callable 正反例通过。
- 仅将本阶段两个生产入口暂还原到 `803e53cc`，执行
  `-p luna --test nominal_value_parameters --no-fail-fast`：
  `/tmp/luna-tools-ol48zymo/result.json` 退出 101，299 项通过、9 项失败。
  这九项诊断与修改后逐项相同，涉及动态父域、别名上下文和默认值输入；其余两项是
  本次引入的 staged trait 空 frame 回归。恢复本阶段两个文件时逐字节核对备份，
  修复该回归后 `-p luna --test nominal_value_parameters staged_trait -- --nocapture`
  通过，`/tmp/luna-tools-1slofcpy/result.json` 退出 0。
- 最终执行 `-p materialization -p luna --test expansion_boundary --test scoped_dispatch_inputs
  --test nominal_value_parameters --test ordinary_callable_types --test ordinary_clause_collectors
  --test sysroot_preludes --no-fail-fast -- --include-ignored`：
  `/tmp/luna-tools-9fq0zt8q/result.json` 退出 101，380 项通过、11 项已知失败。
  九项名义参数失败仍与基线一致；另外两项是主动启用的旧 ignored 正例，动态签名仍报
  `159..181: comptime callable signature still depends on an invocation-time clause input`，
  待定 collector 域仍报 `191..201: list literal requires an element type context`。
  未修改这两项的预期或 ignore。两个 Object 用例、staged trait 回归及物化对照通过。
- 五个 Rust 文件的 scoped rustfmt check：`/tmp/luna-tools-n7mo9da4/result.json`
  退出 0；`git diff --check` 通过。未运行全 workspace、LLVM 或性能基准；验证包含
  工作区原有资源分析改动，不宣称为纯 HEAD 验收。formatter 与其他受保护文件不纳入提交。

### 父声明值捕获：静态证据已交付，动态输入仍待恢复

在 `bcbcd0bc` 上复查 `member_record_defaults_use_selected_value_capture`：
动态父输入 `Holder<enabled()>` 的成员默认值构造 `Config<enabled>` 时，
`normal_value_argument_at` 先检查 clause 参数并观察身份；观察不可用后进入
required invocation，最终因缺少捕获报错。临时诊断显示读取的是 bool 域的
`(local 4294967295 4)`，它存在于 `normal_default_bindings`，而 required 根的
available 输入仅含成员自己的 config 参数，没有父 enabled 的值。

此证据不能归因于 `dependent_inputs/tasks.rs` 中 Type 捕获分支的固定 sort：
该分支读取的 `alias_value_bindings` 保存 Type 域绑定，普通值域另存于
`normal_default_bindings`。`required.rs` 当前也明确不把已有的 pending 默认输入
自动升级为可执行的抽象值。这条限制不能通过给所有未供应的局部值制造 rigid
符号来解除；声明身份、可被观察的输入证据和本次 invocation 已取得的值必须区分。
是否可以合法保留符号还须核对使用处的参数域、声明作用域和后续替换消费者。

该失败定位到 `normal.rs` 的 clause 值观察与 required 输入捕获之间；这尚未
证明只改捕获映射即可完成整个默认值，也没有将它归为 PTC 问题。后续需要沿原父
输入计划交付绑定和待决检查，保留 enabled 与 compute_flag 的调用次数，再以
原用例中的动态／静态父输入及 true／false 身份对照验收。原预期保持不变。

定位时工作目录 `/home/fin/luna`，执行
`python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters member_record_defaults_use_selected_value_capture -- --exact --nocapture`，
`/tmp/luna-tools-se7qt8qv/result.json` 退出 101，诊断仍为
`256..263: required comptime input (local 4294967295 4) is not available in this invocation`。
该运行包含只为定位输入集合添加的临时打印；打印已移除，`required.rs` 与 HEAD
无差异。该次仅更新回复，没有提交编译器修复。

后续静态对照证明，`Holder<true>` 与 `Holder<false>` 也会失败：投影路径已经
观察到父输入身份并填入声明替换表，但成员默认值读取的是对应局部绑定，尚未收到
同一份观察证据。现在父输入前缀与普通输入共用有界前缀观察：未知操作清除旧事实，
预算或结构错误保留其失败分类；仍有效的身份通过 `with_prepared_input_observations`
进入成员检查。原生产者完整保留，不供应给 required invocation、不改变输入顺序，
也不为缺少证据的动态父值制造抽象替代。

新增真实 sysroot 用例在同一 VM 中依次调用 Enabled、Disabled、Enabled，分别与
显式构造比较；两种执行引擎均检查默认 compute_flag 每次只调用一次、显式参数不
调用默认值，以及不同父索引不能满足成员域。修改前同一用例失败于
`238..245: required comptime input (local 4294967295 654) is not available in this invocation`，
证据 `/tmp/luna-tools-c9p7le_v/result.json` 退出 101。此前初稿的角括号分行触发解析
错误，未作为这条语义失败的证据。

最终在同一工作目录执行
`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -p luna --test captured_input_observations --test nominal_value_parameters --test ordinary_input_identity --test required_input_scopes --test normal_input_plans --test sysroot_preludes --no-fail-fast`：
`/tmp/luna-tools-g4rr4ng7/result.json` 退出 101，670 项通过、9 项失败。九项失败的
名称与诊断逐项等于前述基线 `/tmp/luna-tools-ol48zymo/stdout.log`；动态父输入的
旧用例仍失败，没有修改其预期。静态交付不是通用 source activation 或默认值恢复
完成，也没有运行全 workspace、LLVM 或性能基准。

### 动态 NF 的具名入口：已复现，先补参数协议载体

在 `71efd6f8` 上核对报告 D04／S06：动态 NF 的完整收集值供应仍失败，
但位置收集与单包展开已有生产接线。保留三个使用真实 sysroot 的源码对照：

- [正向对照](repros/dynamic-nf-input-control.fl)：用户头 Bundle 的直接具名供应、
  高阶位置收集、高阶单包展开得到相同身份；公开 Tuple 绑定的两种高阶路径也与
  直接具名供应一致。参考执行 `run` 返回 `(bool true)`。
- [用户头的具名供应](repros/dynamic-nf-named-input.fl)：回调类型显式声明
  `...Items: meta.List<Type>`，`factory<.Items = items>` 仍被拒绝。
- [公开内置头的具名供应](repros/dynamic-nf-builtin-input.fl)：同样通过显式回调
  类型供应 `factory<.Es = elements>`，也被同一分支拒绝。不能只为 Es 增加入口。

这次没有改编译器。源码链条表明，缺口不止是路由器漏掉 Arg::Named：

1. `function_types.rs::form_normal_form_callable_type` 将变参声明改作命名位置
   参数，形成普通依赖签名，再只保留 collector 的索引。`form_callable_type`
   把依赖绑定抽象为参数位置，发布的 Parameter 只有 sort 和域，没有公开入口。
2. `normal.rs::dynamic_normal_form_application` 依 collector 索引单独切割实参，
   将尾部位置输入拼成 H::List；单个 Expand 另行改作完整位置输入。
   具名供应和混合展开在这里被拒绝，尚未进入共享输入路由。
3. `program.rs::finish_prepared_call` 对 NF 不恢复源码函数声明参数；
   `arguments.rs::route_arguments` 的具名入口又只能来自 FnParam。
   因此仅删除第一层拒绝，仍不能建立具名参数映射，也没有解决普通函数值的协议载体。

正式依据是 `docs/functions/parameters.md` 的完整参数值供应规则，以及
`docs/functions/higher-order-functions.md` 的 NF callable、依赖签名说明。
前者要求整个 collector 值不再收集，仍接受域、阶段和契约检查；后者把签名局部
依赖名字抽象为位置，一致重命名不改变该依赖关系。**局部绑定名字与公开供应入口
不是同一种信息。** 不能为了具名调用把每个局部绑定名都塞进类型身份，也不能因
依赖签名可以去名字，就抹掉调用接口声明的入口。这些文档也明确把动态具名入口
标为尚未实现，并未规定语言禁止此用法。

后续 M2 迁移需要同时交付以下边界，不能先增加一个 Tuple 专用 named 分支：

- 参数协议独立描述可供应入口与参数槽的映射、位置／完整值／收集方式，以及所需
  的默认、隐式和收集协议。依赖 telescope 继续负责参数域和 binder 替换。
  公开入口信息随检查后的 callable 接口传递；不能从偶然可见的函数体或运行时
  选择结果猜测名字。跨接口适配须保存入口与槽的对应证据。
- 直接声明、保存的函数值和 NF 使用同一输入规划契约。完整值供应只检查一次，
  不先展开再收集；逐项供应才走所声明的收集协议。具体 List 表示不决定路由。
  计划分别保存源码求值顺序和参数槽顺序，callee 与 producer 各求值一次。
- 参数域若仍依赖未取得的输入，保留原计划及对应检查任务，接入已有待定域的
  source activation 迁移；不能执行 callee 猜头、重放已执行前缀或以 Type 替代未知域。
  NF 最终检查所选头的形成条件并保留头身份，不执行其函数体。
- 验收覆盖用户头和公开内置绑定、同一接口下多个 callee、直接／高阶／返回值传递、
  完整值和逐项收集、未知名、重复供应、整包与追加冲突、错误域、源顺序和失败后重用。
  入口载体、通用路由与消费者一起接通后，删除现有动态列表合成分支。

工作目录 `/home/fin/luna`，统一通过
`python3 -B development/check.py --timeout 900 -- run -p luna --` 执行，
每条命令均追加 `--sysroot library --build-target x86_64 linux 64`：

| 剩余命令参数 | 结果 | 本机证据 |
| --- | --- | --- |
| `development/repros/dynamic-nf-input-control.fl --interpret run` | 退出 0，`(bool true)` | `/tmp/luna-tools-t7xkmqoq/result.json` |
| `check development/repros/dynamic-nf-named-input.fl` | 退出 1，`dynamic normal-form collection currently requires positional elements` | `/tmp/luna-tools-hi_y6v8d/result.json` |
| `check development/repros/dynamic-nf-builtin-input.fl` | 退出 1，同上 | `/tmp/luna-tools-oydfp37d/result.json` |

初稿把 Type 常量比较放在 runtime，先触发 Splice 边界；移入局部 const 后，额外的
不同头不等式又触发 TypeContractUnresolved。最终对照只核验各路径与各自直接构造
相等，不据这两次初稿失败判断具名供应问题或头不等式的语义。上述两个拒绝例保留为
已知失败复现，没有改成预期拒绝测试，也没有声称具名入口、通用收集协议或恢复已完成。

随后已退役 `arguments.rs` 中直接基于 FnParam 进行选择的路由算法，改由
`input_routing.rs` 消费独立入口描述并返回源码实参索引。普通调用、开放函数、
方法和 CMTT 的现有消费者已接入；域和默认值检查继续使用原输入准备机制。
这使路由不再要求借用源码签名，但尚未让函数值携带公开入口，动态 NF 的两项
失败仍未解决。职责边界和验证见
[调用供应入口](elaboration-state-and-publication.md#输入路由与域依赖)。
声明输入准备和开放 NF 描述现也使用同一增量供应游标，删除其独立的位置／供应
状态；已知整次输入数与逐步取得包宽度分别处理，失败不推进游标。后续清理还把
Type 子句值、开放输入描述及两个 CMTT 提示入口接入，删除 type_input_slot。
提示入口的冲突／信息不足仍返回 None，未知具名入口仍报错；未把提示失败升级为
必需检查，也未借此吞掉域、预算或 invocation 错误。具体范围与限制见
[展开包供应游标](elaboration-state-and-publication.md#输入路由与域依赖)。
新增[函数值接口传送设计](callable-input-interfaces.md)沿类型形成、NIR、VM 准入和
metadata 编码核对剩余缺口。其别名／返回值／高阶转发复现中，位置展开对照返回
true，具名完整供应仍被拒绝；不能只在调用表达式上挂名字表后声称完成。首条迁移
使用相同公开入口，不以此补定不同入口名自动适配的语言规则。

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
