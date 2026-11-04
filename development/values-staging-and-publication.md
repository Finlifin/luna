# 值、执行、代码组装与定义发布

状态：目标设计。接续 [架构总览](architecture.md)，不是新语言语法或现有 API 声明。
这组边界共同解决 NF 专用值、VM 重入、builder 世界切换和模板替换的长期耦合。

## 1. 通用值不等于一个可随意互转的大枚举

```text
LiveValue       本次 VM 存储中的值；可以含受控值引用
FrozenValue     可跨局部求值边界传递的稳定值及描述
CodeValue       带类别、上下文、来源、控制接口和义务的代码值
DefinitionDraft 尚未发布的结构化定义草稿
TypeValue       完成当前所需形成检查的类型描述
```

这些是语义能力不同的载体，不要求五套完全重复的 payload。
可共享整数、文本、持久序列、记录等 immutable 节点，但不能以共享节点为由跳过
接收域、阶段、存储及 world 检查。NF、普通实参、反射、builder、缓存使用同一值准入服务。

comptime value-reference 用 VM 的 slot/lens 表示可更新位置。读取和更新仍作用于原值，
不能以删掉指针语法后按值复制替代；同时不得导出为 Flurry runtime pointer。
嵌套字段/列表位置需维护根、路径、世代和权限。赋值使旧内容证据失效，不一律使值引用
失效；同一存活槽的引用可重新定位并观察新内容。owner 结束、槽世代不符或路径不合法
时拒绝。具体准入、快照及恢复事务见[值传送协议](comptime-value-admission.md)。

## 2. Freeze / restore 协议

`freeze(value, receiving_contract, scope)` 逐项检查实际描述、依赖、可逃逸性及预算，
返回冻结值与依赖摘要。遇到活动引用或未完成义务不得只保存裸 handle。
是否能冻结代码/类型不能由外层 Any 标注决定，要检查实际值见证。

`restore(frozen, target_world, target_session)` 重新对应声明及版本、验证参数域和契约，
再产生本次 session 的值。目标 package 中存在同号声明不证明它与源声明相同。
不把一个世界的实现证据当作另一个世界的缓存许可。

内置紧凑表示可以继续使用，例如持久树或连续标量数组，但必须满足同一值接口、
同一形成规则、同一序列语义。Type 元素的列表不获得独立语言权限。
字符串和列表哈希可以缓存不可变结构结果；源语言 Eq/Hash 协议选择仍有自己的阶段和世界。

成员资格、传送资格、物理编码和缓存资格分别交付证据。开放定义的合法 residual 不等于
当前需要具体值的 CompletedValue；局部闭包快照也不等于 metadata wire 获准执行工厂。
这些边界及现有 list 普通编码、闭包重建路径的迁移见[准入细则](comptime-value-admission.md)。

## 3. 通用上下文表达式取代 NF 专用表达式

当前 NormalFormArgument 的 If/Block/Match/Integer/List 等分支，是把一般表达式的
“等待接收域”问题复制成另一套解释器。目标为共享的上下文表达式描述：

```text
ContextualExpr {
  expression,
  lexical_bindings,
  expected_domain_observation,
  permitted_residual_reason,
  source_evaluation_slot
}
```

域未就绪时可以保留字面量和分支结构，但不能执行不确定分支，不能把任何检查失败都
降成 deferred。域就绪后普通检查生成同一 Expr/Value；NF 只负责以已检查值形成应用。
收集器、命名赋值、默认值和展开都归 ArgumentPlan，不再各自实现顺序与转换。

迁移应从普通调用和用户定义 NF 的同一依赖参数例开始，同时覆盖 Tuple/List。
仅把 NormalFormArgument 改名而保留第二套遍历器，不算消除。

当前普通子句已先分离[检查顺序与源码求值位置](comptime-value-admission.md#13-域检查顺序与源码求值顺序分离)：
静态 Type 尚未取得时保留 HIR 输入，在其域就绪后交回普通 expr 检查。它不增加新的
表达式解释器，也不提前执行 Type 生产者。NF 表达式现已采用
[统一输入准备与普通集合组装](comptime-value-admission.md#17-nf-输入统一准备检查顺序与求值位置分离)，
删除独立 positional 收集入口；NF 类型标注也消费同一计划，不再拥有纯 Type 与混合
输入的独立绑定循环。静态域使用同一普通列表表达式，连续尾部与交错产生器分别保留
各自的源码位置。函数类型具名参数接入普通表达式检查；Type 值构造函数类型时的
binder 提升由静态检查、观察与两种 VM 共用。动态 contextual 参数树、交错输入的
可恢复检查及其他 NF 消费者仍须后续迁移，不能以共享组装器声称 ContextualExpr 已完成。
开放函数的 NF 描述与具体准备已共用槽位路由，并按实际元素域处理收集器；开放
描述仍需在具体化时重新检查原始源码，不是已经冻结的可执行 Expr。

签名标注的外层域环境全部明确，或共享的词法依赖查询已证明不使用待定输入时，
现已直接生成一个普通 typed NIR region，保留控制流和绑定供定义侧 phase／算术
检查，不再先压成域描述后重复生成。查询沿完整源码边及局部声明依赖闭包观察；
预算失败不能充当无依赖证据，未映射的符号输入仍保守保留 pending。这个临时区域
不代表标注已经执行或实际类型已经取得；具体化仍消费原 suspended source。实际
依赖未知域的结构保留尚未迁移，完整 ContextualExpr 仍须携带待定域、词法依赖和
证明义务，不能把这项有限接线称为开放定义的整体行为检查已完成。证据与边界见
[已知输入域的标注区域](comptime-value-admission.md#已知输入域的签名标注保留完整行为)。

### 已接通：纯 Type 与混合子句共用准备路径

`prepared_type_inputs` 的独立循环已删除。普通 callable、名义类型和成员实例的
输入准备都经过 `prepared_clause_inputs_in_domains`，不再因接收域全为 Type 就
绕过普通准备。传递给下游的仍是按源码位置保留的生产者和独立的身份观察，不能
从静态 Type 观察推导出生产者可以省略。

固定宽度展开包的布局与元素域分开取得；已观察的前置输入用于专门化各元素域，
域全部可用时才形成普通 Tuple 上下文并检查包表达式。这保留了 Type 包的返回类型
上下文，也用于 `Tuple<u8, bool>` 等普通混合包。上下文不能覆盖生产者的 requires，
不能把待定 rigid 当成具体元素域；带 collector 的尾部仍按列表收集协议处理，不用
固定 Tuple 布局猜测其长度。运行一次的空包、提前退出和默认值顺序继续由共同路径
保留。required Type 的立即求值入口仍保留，共享包布局查询；没有把实际 required
求值与保存 invocation 代码混成一个阶段。

真实 sysroot 回归见 `compiler/luna/tests/clause_packet_preparation.rs`：两种 VM
核对完整用户函数调用序列，覆盖普通混合包、纯 Type 名义包、空包／默认值及提前
return，并拒绝越界字面量、错误字段、宽度和不成立的生产者要求。这项准备合流
不解除下面的 invocation-time 域缺口。

工作目录 `/home/fin/luna` 的验证记录：

- `python3 -B development/check.py --timeout 900 -- test -p luna
  --test clause_packet_preparation --test type_spreads --test ordinary_clause_collectors
  --no-fail-fast`：`/tmp/luna-tools-px361ii4/result.json`，退出 0，125 项通过、
  1 项 ignored。随后补充提前退出并加强调用序列断言。
- 最终 `python3 -B development/check.py --timeout 900 -- test -p luna
  --test clause_packet_preparation --test nominal_value_parameters --test nominal_staging
  --test default_calls --test normal_input_plans --test open_normal_input_plans
  --test comptime_clause_inputs --test dependent_list_collectors --test library_protocols
  --no-fail-fast`：`/tmp/luna-tools-h5halzp7/result.json`，退出 0，461 项通过。
- 五个修改 Rust 文件的 rustfmt 检查：`/tmp/luna-tools-9ph1yr7i/result.json`，退出 0。
  未运行全 workspace、native 或性能基准；两组测试有重叠，不相加作为独立用例数。

### Invocation 才能取得的域：必须同时接通检查与执行

普通调用的现有正例 `pending_clause_domains_can_wait_for_a_type_producer`
仍未接通：`count<.values = [1, 255], Byte()>()` 中，Byte 是返回 Type 的普通
comptime 函数，列表的域由其实际结果决定。2026-11-05 在当前工作区显式运行该
ignored 用例，仍报 `191..201: list literal requires an element type context`；
证据 `/tmp/luna-tools-u3d0ape7/result.json`，退出 101。这不是要求新增语法的反例，
也不能改成预期拒绝来消除失败。

源码定位显示至少有以下独立边界，不能只修掉最先出现的诊断：

| 当前入口 | 已有能力 | 尚缺接线 |
| --- | --- | --- |
| `clause_inputs::prepared_clause_inputs_in_domains` | 保存 HIR 输入；静态域就绪后重排检查，最终仍按源码位置准备输入 | 无可用域时仍进入普通 `expr(None)`，没有可恢复的上下文检查请求 |
| `dynamic_instances::prepare_callable_clause_packet` | 保存普通值 local 与静态身份，分别用于执行和实例描述 | Type 输入仍要求 `TypeValue`；直接放行 Type local 不能保证 callable 签名已完成 |
| `input_collection::Frame` | NF／名义输入的源码顺序、已执行值、域唤醒、终止与清理 | 消费的仍是 `NormalFormArgument`，不持有普通前端区域或其检查义务 |
| `invocation::session::Session` | 独占 VM 的完整驱动现场，可在内部推进边界保存、移动与取消 | 生产入口仍同步驱动；尚无对外宿主请求与普通前端检查响应 |
| `PreparedRequired::invoke_in_world` | 独立准备世界、限定的类型前提、同步 builder 服务 | builder 的 `prepare` 回调不是通用的前端暂停／恢复协议 |

其中 Type local 的限制来自源码审计，尚未绕过首个错误单独复现。字节码已有的
`Suspended` 只表示内部驱动继续推进，不等于第 5 节的对外 `HostRequest`。

目标接口需要分别记录三个事实：表达式的检查进度、源码求值位置、接收域的观察。
检查完成不代表已经执行，值已经产生也不代表已获准进入接收域。区域拥有源码身份、
词法绑定、待决义务和执行许可；VM activation 拥有已经执行的前缀、存储及控制进度；
域观察拥有值的来源、world 和输入依赖。稳定代码缓存只保存已检查代码，不拥有这些
活动区域、捕获帧或本次域检查结论。所有权与响应提交沿用
[状态协议](elaboration-state-and-publication.md)，不能额外建立以源码地址为键的全局等待表。

列表字面量可以保留尚未转换的整数及结构，待域成为 `meta.List<u8>` 后才检查范围
并完成准入。这里允许推迟的是接收域相关的检查／组装，不能据此推迟任意表达式的
副作用。如果较早表达式的操作选择本身需要域，必须保留该执行依赖；不能越过它执行
后面的生产者，再声称仍保持源码顺序。域无关且已获执行许可的前缀只运行一次，之后
恢复检查必须复用其实际结果。分支条件、return、清理、捕获和局部写入仍属于原控制
位置，不能被参数调度器拆成互不关联的回调。

等待关系应明确区分“域由哪个实际值产生”和“执行为什么不能越过前一个位置”。
只有真实生产者与这些依赖形成闭环，才报告循环；缺少宿主接口、预算耗尽或某节点
尚无实现都不是循环证据。具体哪些表达式可以只延后组装，必须由共同表达式规则给出，
不能靠增补 Integer／List／Block 白名单决定。类型命题仍归 PTC；这里的执行顺序、
值存储、资源及程序状态义务不转交 PTC。

落地顺序是：共同前端区域保留待定上下文与检查义务；VM 栈取得可恢复的 session
所有权；宿主请求携带域观察和受检响应；普通 callable 消费同一输入计划并在签名
真正完成后发布实例。已有静态域路径和稳定代码缓存可以继续使用，不必为等待新接口
复制第二份 AST 或伪造 Any／Type。退役旧参数树前，须同时验证普通 callable 与 NF／
名义输入、依赖域闭包、失败恢复、提前退出、生产者次数与顺序；前面的列表用例只是
其中最小的接线入口。


### 待定闭包需要保留可调用证据与主体，不能伪造完整函数类型

当前实现的具体断点见
[待定 Type 捕获复现](repros/pending-type-closure-annotation.fl)。源码中 `|| T`
明确给出了闭包及其零参数调用接口，但 T 的接收域尚待早前输入确定。现有
`describe_open_expression` 没有闭包描述分支，会退到普通 typed region；普通
`closure_body` 又要求完整参数、结果和捕获类型。只修复依赖发现，不能解除这个
表示层循环。`FrozenDescription::Any` 不提供完整函数类型，不能把 Any 或 Type
临时塞进 NIR 的函数类型字段，也不能调用 Domain 生产者来猜测定义侧类型。

目标是在共同 ContextualExpr 中分别保留以下信息，具体 Rust 表示尚未落地：

| 信息 | 可以支持的检查 | 不能据此作出的承诺 |
| --- | --- | --- |
| 显式闭包的阶段、参数协议、源码身份 | 调用阶段、参数供应形状、来源诊断 | 结果域已经取得、主体已经通过 safe 检查 |
| 有序 binder 和各自域观察 | 后续参数依赖、已知类型的局部检查 | 未知域自动等于 Type，或任意 Any 都可调用 |
| 词法捕获及其来源、存储关系 | 自由依赖与环境取得计划 | 当前活动环境可以冻结或进入缓存 |
| 完整主体、控制边界与待决义务 | 保留分支前提，域就绪后继续 ordinary 检查 | 每个操作孤立通过即可证明整段行为 |

可调用证据必须来自该值的源码构造、已检查签名或合法 refinement，不能只凭
变量名恢复。普通别名传递保留相应证据；赋值和分支汇合按值身份及存储版本更新，
不能永久把某个 local id 映射到首次出现的 lambda。不同分支的 callable 或
non-callable 值必须共同决定可用接口，不能任选一个分支的签名。显式 Any 的
现有精度规则保持不变，不借新增结构描述自动授予调用或成员访问权限。

形成闭包、取得捕获和执行主体是不同的行为位置。保留普通捕获语义、共享关系、
comptime 值引用与 runtime 资源约束；描述器不执行主体。每次实际调用仍按普通
规则只准备一次实参，再进入闭包的独立 return 边界。待定区域保存检查进度和
求值位置，已有生产者结果不因域恢复而重放；活动 capture frame、等待者和候选
事务仍由 region 所有，不能塞进 FrozenDescription 或全局描述缓存。

迁移顺序应先让共同区域表达式支持带待定域的 binder、捕获和主体义务，再让
闭包构造及动态 callee 消费同一接口；域就绪后由普通检查产生完整 NIR。不能只
给零参数闭包加一个局部别名表，也不能新增闭包专用执行器。当前描述器的
`describable_open_type` 在描述器中仍存在。签名分类及推断调用的 readiness 已移除
该白名单，输入发现和供应改用共同源码依赖观察；其他消费者只有在共同机制覆盖其
输入及失败路径后才可删除。已知域的局部闭包标注，以及不引用自身参数的具名动态函数类型已接通；待定域
闭包和真正依赖自身 binder 的动态 callable telescope 仍未完成。具体边界见[签名依赖接线](comptime-value-admission.md#签名分类与调用供应共享输入依赖观察)。

交付验收须包括：待定 Type 的直接／嵌套捕获、已知域闭包、别名和重新赋值、分支
汇合、参数及结果标注、未调用定义中的错误主体、阶段错误、提前返回，以及暂停后
失败／恢复不重放生产者。真实 sysroot 和两种 VM 应观察一致的次数、顺序与结果；
捕获域和义务未闭合时不能交付可执行实例。现有复现继续保留为未完成能力，不能
改成拒绝例来让测试变绿。


## 4. 两种执行载体和一个语义服务层

混合 interpretation 负责 elaboration 中必要的局部求值/观察；invocation engine
负责独立 comptime 函数的字节码调用；runtime reference interpreter 是第三个用途，
用于参考执行而非定义 comptime 能力。

应共享的服务是算术/容器语义、域检查、绑定、存储、冻结、契约和宿主请求的含义，
而非强制三者具有相同控制流内部表示。字节码编译发生在执行之前；不能先执行前缀、
发现 Unsupported 后改用树解释器从头重跑。

每个 CodeObject 包含稳定函数身份、签名、指令、常量表、来源映射、宿主请求接口、
世界依赖和验证结果。代码缓存可复用它，不共享某次 invocation 的帧或可变值。
指令必须有完整 Display；原语操作通过明确绑定身份 dispatch，不按公共源码拼写识别。

## 5. 可恢复 VM 与宿主查询

拟定协议形状：

```text
poll(session) ->
    Returned(checked_value)
  | HostRequest(request_id, operation, frozen_inputs, continuation_token)
  | Failed(diagnostic, consumed_budget)
  | Exhausted(resource, progress)

resume(session, request_id, checked_response)
```

token 仅属于本次 session，不进入冻结定义或持久 query。宿主 response 必须匹配
operation、输入和 world 依赖；重复 response 不重复安装值，不恢复已取消帧。
frame 保存 PC、局部槽、结果目的地、控制和清理进度；异常退出也要回收一次。

请求身份包含 session、activation 和动态执行序号，不能只用 PC：循环经过同一指令是
新执行，恢复同一次暂停才沿用原请求。响应写槽、推进 PC 和标记 Delivered 一起提交。
完整状态、取消和预算规则见[状态协议](elaboration-state-and-publication.md)。

若请求有真实未完成输入生产者，region 让该 VM session 等待。若宿主只是预算耗尽，
不能伪造 waiter。必需结果循环报告依赖链；已存在签名支撑的普通递归仍按函数执行。

外部不纯操作只有在源控制流明确要求、候选选择已确定后执行。请求身份与进度保证
同一会话恢复不重放，不承诺崩溃后自动 exactly-once；崩溃重启或取消后重跑的外部任务
须由 build 任务契约定义。内存事务不能回滚外部文件或进程。

### 已接通：宿主调用的执行额度交接

关联选择、方法选择、struct／enum／newtype builder 和带预算的 Type solver
通过同一个 `host_budget` 边界交接额度：进入宿主前取走 VM 的剩余额度，宿主只操作
独立的 allowance；正常返回后再结算，包括返回错误的情况。宿主 unwind 时 VM 的
剩余额度保持为零，不能继续使用未经结算的工作额度，也不能借 panic 绕过补额检查。

各服务保留原有响应规则：选择 resolver 试图补额时清零并报告 `StepsExhausted`，
响应不进入选择目录；builder 和带预算 solver 将剩余额度限制在原授予量以内。
无预算 solver 的接口未改变。该机制只处理额度所有权，不赋予宿主新的证明权限，
不承诺任意 panic 后整个 VM 都可恢复，也不回滚宿主已经发生的外部副作用。

验证在 `/home/fin/luna` 执行，统一前缀为
`python3 -B development/check.py --timeout 900 -- test`：

- `-p comptime_interpretor --lib host_ --no-fail-fast`：退出 0，
  `/tmp/luna-tools-_qzdbdmc/result.json`。覆盖正常／错误结算、两种补额策略、
  unwind 及真实关联、方法和 solver 入口的显式新额度重试。
- `-p comptime_interpretor -p luna --lib --test builder_invocations --test comptime_bytecode_traits --test associated_purity --test associated_signatures --test deferred_associated_values --test library_meta_objects --test library_type_equality --no-fail-fast`：
  退出 0，`/tmp/luna-tools-0maue7b9/result.json`。覆盖 VM 与 luna 库测试、
  builder／trait／关联值和真实 sysroot 的公开 meta 对象及 Type 相等接口。

### 已接通：驱动现场归执行会话所有

native 递归深度与显式 invocation 深度由不同所有者管理。临时树求值、输入解释、
模式、效应映射和编译后方法观察，以及复用存储的列表／对象快速路径，统一通过
`native_depth` 作用域恢复进入前的计数；正常返回、错误与 unwind 共用同一恢复点。
`tick` 仍在原位置执行，深度／步数失败不会新进入结果交付或清理钩子。临时深度恢复
不退还执行预算，不清零其他调用者的深度，也不替代 activation 自己的退出协议。

此前直接方法准备在调用 resolver 前增加 native depth，宿主 panic 后跳过递减；
即使 Session 恢复了调用栈，外层深度仍从 3 留在 4。新增嵌套调用回归在修改前
于 `/tmp/luna-tools-ate_ceh9/result.json` 退出 101，明确复现此残留。修复还覆盖树解释、
已检查输入与待定输入的同类异常路径；宿主中断后额度保持为零，显式给予新额度后
才能重试。这里只保证深度作用域的恢复，不宣称任意 panic 后所有词法、效应或外部
状态都可恢复。

最终在 `/home/fin/luna` 执行
`python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p luna --lib --test effect_contracts --test effect_memory_transfers --test normal_input_plans --test open_normal_input_plans --test comptime_bytecode_traits --test comptime_bytecode_patterns --test comptime_bytecode_lists --test comptime_object_storage --test library_meta_objects --test library_list_hash --test builder_invocations --no-fail-fast`：
退出 0，886 项通过，`/tmp/luna-tools-89nhqypy/result.json`。覆盖深度／预算边界、
异常重试、调用与参数顺序、效应内存转移以及真实 sysroot 的 builder、对象和列表接口。
本批九个 Rust 文件的 rustfmt 检查退出 0，`/tmp/luna-tools-qd8n8tkr/result.json`。

原 `run_executions` 的局部调用栈、待启动计算和待交付结果已经迁入
`invocation::session::Session`。生产中的字节码 invocation、借入 fragment、借入输入
计划都由该会话驱动，默认值、关联值、契约与嵌套类型计划仍使用同一调用栈。
类型观察等非字节码入口启动的关联初始化器也作为独立 associated root 进入该驱动。
固定树模式的轻量入口保留，避免每层树解释都挂上字节码驱动栈帧。

会话在存活期间独占借用 Interpreter；暂停现场不能被拿去交给另一个 VM，也不能
同时在原 VM 上启动不相干的外部调用。根入口用枚举区分 invocation、associated、fragment
和输入计划，不能同时供应两个 root。每次 poll 推进一次驱动迭代，至多执行一个
字节码片段；已有同步准备和树解释仍受各自预算控制。这不是硬实时调度、instruction
级抢占或 `HostRequest` 已完成的承诺。

正常完成和语义失败都终止会话，重复 poll 拒绝继续。放弃尚未完成的会话时，将一次
内部失败沿既有交付路径向外传播：恢复各调用层保存的 effect 身份、调用深度和检查
上下文，归还已借出的环境，移除输入计划自有绑定，并使借入 root 不能恢复执行。
取消不执行剩余 producer、默认值或契约，不撤销已经发生的写入，也不回退 effect
身份分配器。成功完成后的借入环境／计划清理仍遵循原调用者契约。这里验证的是正常
Rust 控制流下的主动放弃与 Drop，不宣称进程崩溃或任意 panic 中可以恢复外部副作用。

会话 dump 使用 S-expression，显示 root、排队计算、关联选择请求、待交付类别和嵌套 code／plan。
活动会话仍不是可冻结值，不进入 CodeSession、结果缓存或持久查询。尚缺的是前端
区域、宿主 request 身份与受检 response 的实际生产接线；不能把内部 Poll::Pending
解释为等待宿主，也不能利用此会话容器提前执行域生产者。

验证使用 `/home/fin/luna` 工作区：

- 新的会话用例逐一放弃嵌套调用与依赖输入计划的每个严格执行前缀，覆盖会话移动、
  未启动的根调用、等待子调用、已完成但未交付的结果、staged 深度恢复、计划自有
  绑定清理及同一 VM 的重新调用。取消前后的事件、输入执行预算及 caller 环境分别
  核对；重复 poll 和继续已取消的 root 拒绝。无效 fixture 最初被既有 packet
  preparation 检查拒绝，已修正为合法 tuple pattern，没有放宽该检查。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
  -p luna --lib --test clause_packet_preparation --test comptime_bytecode_defaults
  --test comptime_bytecode_traits --test comptime_bytecode_types --test normal_input_plans
  --test open_normal_input_plans --test nominal_staging --test builder_invocations
  --test library_protocols --no-fail-fast`：`/tmp/luna-tools-xr0efrj7/result.json`，
  退出 101，751 项通过、2 项失败。VM 的 487 项与 luna 的 84 项单元检查通过；
  完整命令不能记为全通过。
- enum builder 的失败仍在 HIR lowering：共享 fixture 缺少 `Symbol` 绑定；与
  [先前源码对照](comptime-value-admission.md#12-required-与-invocation-共享-callable-参数包)
  记录一致。另一失败是 `deferred_normal_inputs_run_bytecode_before_their_domains_arrive`
  要求上下文输入总计数为 2、实际为 3；已用本次修改前的四份 VM 生产文件单独对照，
  同样在第 551 行失败，`/tmp/luna-tools-0z_8e64c/result.json`，退出 101。对照后本次
  修改逐字节恢复。两项 fixture／计数预期本次均未修改。
- 恢复后 `python3 -B development/check.py --timeout 900 -- test
  -p comptime_interpretor --lib invocation:: -- --nocapture`：
  `/tmp/luna-tools-kqxlw7nu/result.json`，退出 0，7 项通过。
- 五个修改／新增 Rust 文件的 rustfmt 检查：`/tmp/luna-tools-i92usp8x/result.json`，
  退出 0。未运行全 workspace、native 或性能基准。

### 已接通：调用与方法准备归会话所有

字节码的 `BeginCall`、`TraitMethod` 与动态实例的方法值分支不再在指令内部完成
调用准备／方法选择。指令推进 PC 后交出 owning `Preparation`，帧进入等待状态，
由同一执行会话在下一次驱动转换中消费；等待期间再次执行该帧被拒绝。普通调用与
直接 trait 调用共用此边界，保留动态方法身份，不能用实现函数替换公共接口后丢掉
默认参数与程序契约。调用准备持有原 CodeObject 和指令位置，不复制签名或重跑指令。

原有步骤顺序保持：动态接口默认输入、接口程序契约、方法选择、callee 身份、参数
绑定检查、调用暂存区安装，之后才执行调用参数。方法值读取也在准备结束后才写回
目的槽。Session 的请求槽以单一枚举持有关联请求或字节码准备，取消先丢弃未执行的
请求，再恢复已有调用栈；已执行的 caller 写入与消耗的预算不回滚。

这里的 `Preparation` 是内部驱动工作，不是已验证的外部 HostRequest。其中的域观察、
resolver 和函数实例准入仍同步执行；此处未承诺这些操作内部的暂停／恢复，也未完成
前端 region、跨 world 响应或独立请求身份协议。树解释继续使用原来的同步选择服务。

验证工作目录 `/home/fin/luna`：

- VM 首轮全库检查 `/tmp/luna-tools-p1c2no03/result.json` 退出 101：504 项通过，
  原 `pending_call_requires_delivery_and_rejects_bad_slots_before_effects` 尚未处理新增的
  准备暂停点。该测试已分别断言准备与调用两个等待边界均拒绝继续执行，保留原有
  重复交付拒绝及错误参数先于副作用的检查。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p luna --lib --test comptime_bytecode_traits --test dynamic_trait_clauses --test builder_invocations --test comptime_bytecode_types --test deferred_associated_values --test library_meta_objects --test library_list_hash --no-fail-fast`
  退出 0，`/tmp/luna-tools-u7eugzo2/result.json`。内部测试覆盖每个严格执行前缀的
  取消、选择失败及新调用恢复；源码组覆盖直接／存储／动态 trait 调用、接口契约、
  默认参数及调用顺序，并运行真实 sysroot 的 builder、meta 对象与 List hash 接口。

### 已接通：关联选择请求与初始化器准入分离

`associated_execution` 将关联访问分成 `Preparation::Request` 和已准入的
`Activation`。请求保留选定的 subject、interface 参数、原始访问和本次已观察的
结果域。字节码驱动将待处理请求保存在 Session 中，下一次推进才调用现有 resolver；
取消先丢弃请求，再沿调用栈交付失败。因此请求尚未处理时，不会因取消而调用宿主，
也没有需要释放的初始化器活动 key。已进入初始化器的 key 由 Activation 完成时
释放，返回值仍经过共同值域检查。树解释同步消费同样的准备与准入接口。

宿主响应的表达式在进入目录前支付复制工作、检查结果域并取得初始化权限；随后
直接使用该 Activation，不再插入目录后重新取出并重复检查。此前首次选择会对主体
类型调用两次 `selected_type_view`，可能实际执行两次关联 Type 程序。当前每次访问
只观察一次选定主体的结果域，保存访问端已观察的结果域用于完成检查。后续独立访问
仍重新观察，不缓存不纯程序结果，也不把 invocation initializer 改成常量。

Request 和 Activation 都有 S-expression Display，Session dump 包含待处理请求。
两者只属于当前 VM 的执行现场，不进入 CodeSession。已有 resolver 的禁止补充预算、
阶段限制、响应域拒绝、初始化循环与错误恢复检查继续生效。

主动 Type 观察的权限由 comptime 阶段及源世界边界决定，不再把“安装了 resolver”
作为前提。具体关联访问统一经 Preparation 判断：快照已经保留初始化器时直接准入，
只有缺少选定主体才需要 resolver。此前即使完整实现、dispatch 和初始化器已通过
源世界检查，未安装回调的 VM 仍将它误报为 `UnmaterializedDispatch`。runtime VM
仍不执行这些初始化器，IIR VM 仍不能借挂载源 resolver 回到源码选择；缺少主体且
没有 resolver 也继续报告未取得选择。主体可用性、执行权限与宿主服务能力分别检查。

回归使用完整的 trait、impl 和 dispatch 初始化器，经初次及复用 CodeSession 的
源 VM 构造进入，两种解释模式均覆盖独立重复观察、嵌套程序读取、耗尽后恢复和真正
缺失的选择。另验 runtime 与 runtime/comptime IIR 即使挂载 resolver 也不调用它。
工作目录 `/home/fin/luna` 的
`python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
--lib selected_types::observation_tests -- --nocapture` 在生产修复前
`/tmp/luna-tools-6x1cel49/result.json` 退出 101，仅无 resolver 的完整初始化器用例失败；
修复后 `/tmp/luna-tools-sp4v3g3c/result.json` 退出 0，5 项通过。

扩大检查运行 `python3 -B development/check.py --timeout 900 -- test
-p comptime_interpretor -p luna --lib --test deferred_associated_values
--test associated_signatures --test associated_purity --test comptime_bytecode_traits
--test library_protocols --test comptime_bytecode_observations --test comptime_bytecode_types
--no-fail-fast`：`/tmp/luna-tools-pzfa2prn/result.json` 退出 101，654 项通过、1 项失败。
失败仍是本节先前已对照确认的 `deferred_normal_inputs_run_bytecode_before_their_domains_arrive`
在第 551 行要求上下文输入计数 2、实际 3；返回值断言已通过，未修改该预期。
VM 全部 497 项、真实 sysroot 协议与其余关联／类型路径通过。两份 Rust 文件 scoped
rustfmt 检查 `/tmp/luna-tools-kq3dzqic/result.json` 退出 0；未运行全 workspace。

这仍是内部驱动边界：resolver 本身同步执行，subject／参数／结果域的观察仍可同步
执行关联 Type 程序。尚未提供公共 HostRequest/response API、跨请求身份验证或前端
region 的可恢复检查，不能把此处 Pending 宣称为完整宿主等待协议。它也不回滚已发生
的宿主副作用。

验证使用 `/home/fin/luna` 工作区：

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
  --lib associated -- --nocapture`：`/tmp/luna-tools-iw2cjddy/result.json`，退出 0，
  25 项通过。新用例逐个取消四层关联选择链的所有严格执行前缀，核对 resolver
  次数、预算、活动 key、caller 写入保留和同一 VM 重试；首次选择与后续读取均核对
  实际 Type 函数进入事件。既有完成值域用例改经真实准入取得 Activation，未放宽
  Unknown／Rejected 的预期。
- 将四份相关生产／测试文件暂换为 `b7a2aeb1`，保留新域观察回归单独运行：
  `/tmp/luna-tools-z8p1svtv/result.json`，退出 101，实际进入 `Domain` 两次、预期
  一次。对照后四份文件逐字节恢复。新 fixture 初稿把未观察的关联域直接放在 bool
  literal 上，被既有 literal 检查拒绝；最终用合法的带关联结果域 block 承载该值，
  未修改 literal 检查。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
  -p luna --lib --test deferred_associated_values --test associated_signatures
  --test associated_purity --test comptime_bytecode_traits --test library_protocols
  --no-fail-fast`：`/tmp/luna-tools-kpcapuye/result.json`，退出 0，632 项通过。
  包括 VM 全部 491 项单元检查、关联域／默认值／纯度、LLVM O0/O3 的关联值路径，
  以及真实 sysroot 协议。没有运行全 workspace 测试或性能基准。
- 六个修改／新增 Rust 文件 scoped rustfmt 检查：
  `/tmp/luna-tools-9konbg79/result.json`，退出 0。

类型观察入口此前调用 `eval_associated_value` 后直接树解释主体；即使 VM 启用了
字节码，关联初始化链仍递归占用原生栈。当前此入口先保留 Preparation，再通过
`Session::associated` 持有待选择请求或已准入 Activation，代码选择、嵌套读取、
完成值域检查及取消均走已有驱动。固定树模式、活动 effect 环境或 runtime 清理等
不适用字节码的上下文保留原路径；分支在进入独立驱动函数前选择，避免固定树递归
为每层保留大型 Session 栈帧。初始化器的词法环境继续独立，不读取调用方 locals。

96 层初始化器链在原生深度额度 8 下，旧字节码模式实际报 `DepthExhausted`；
修复后同额度成功，每次查询仍进入末端 Type 函数一次。固定树模式仍按该额度拒绝，
不会绕过资源限制。执行步骤耗尽后活动 key 和深度恢复，同一 VM 补充额度后可重新
执行。新的 associated root 逐个取消全部严格执行前缀，分别覆盖尚未调用 resolver
的请求及已取得初始化 key 的 Ready 状态；取消不花费执行预算或调用 resolver，
随后新执行只选择尚未取得的主体。

工作目录 `/home/fin/luna`：新增深链回归、生产代码未修改时，
`python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
--lib type_observation_runs_deep_initializers_on_the_driver_stack -- --nocapture`
在 `/tmp/luna-tools-7o_sr5p5/result.json` 退出 101，字节码阶段仍耗尽原生深度。
接通后同命令在 `/tmp/luna-tools-mt1dyv9r/result.json` 退出 0；含根入口取消用例的
VM 全部 495 项在 `/tmp/luna-tools-594poq_s/result.json` 退出 0。

最终源码（含固定树入口与驱动栈帧分离）运行
`python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
-p luna --lib --test deferred_associated_values --test associated_signatures
--test associated_purity --test comptime_bytecode_traits --test library_protocols
--test comptime_bytecode_observations --test comptime_bytecode_defaults --no-fail-fast`：
`/tmp/luna-tools-5d7qn9ck/result.json` 退出 0，647 项通过。四份 Rust 文件 scoped
rustfmt 检查 `/tmp/luna-tools-o8ci7k_t/result.json` 退出 0。未运行全 workspace
或墙钟性能基准。

这次统一的是初始化器主体的执行入口。关联 subject／参数／结果域准备仍同步，
外层类型观察循环也未向前端暴露可恢复请求；不能据此宣称全链路异步宿主协议完成。

## 6. 缓存分别回答不同问题

### 关联 Type 观察不进入执行目录

`selected_type_view` 的已观察 Type 值由本次查询的局部表持有，`Tracked` 查找仅在
该次 normalize 中叠加这份表。重复投影可以复用本查询已取得的视图；查询触发的
初始化器及其内部读取仍访问普通 VM 目录，不继承外层查询的临时值。错误、耗尽和
正常返回都直接释放局部表，不再修改／恢复 `associated_values`，旧的仅供这种恢复
使用的 `remove_owned` 已删除。正常 resolver 发布的已检查主体及常量目录保持。

旧实现虽然在返回前删除临时值，但该值曾对嵌套程序可见：观察 A 后执行 B，B 的主体
再次读取 A 时直接取了临时值。回归实际记录为 `Domain9, Domain10`，漏掉第二次
`Domain9`。当前观察 `[A, B, A]` 的轨迹为 `Domain9, Domain10, Domain9`：最后一个
外层 A 复用本查询结果，B 内的独立读取正常执行。连续三次独立查询均重新执行，树与
字节码路径一致；缺失选择及错误结果域后，后续查询不继承失败查询的值。

工作目录 `/home/fin/luna`：只加入回归、尚未改生产逻辑时，
`python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
--lib selected_types::observation_tests -- --nocapture` 在
`/tmp/luna-tools-jt43d__1/result.json` 退出 101，嵌套执行次数回归失败、失败后重试用例
通过。修复后同命令在 `/tmp/luna-tools-sp0vzbub/result.json` 退出 0，两项通过。
初稿 fixture 未提供函数体关联读取所需的 trait 契约，已补齐合法元数据，未放宽阶段检查。

扩大检查运行 `python3 -B development/check.py --timeout 900 -- test
-p comptime_interpretor -p luna --lib --test deferred_associated_values
--test associated_signatures --test associated_purity --test comptime_bytecode_traits
--test library_protocols --test comptime_bytecode_observations --no-fail-fast`：
`/tmp/luna-tools-bur0dqmj/result.json` 退出 0，636 项通过，包括 VM 全部 493 项、
真实 sysroot 协议和关联值 LLVM O0/O3 路径。三份 Rust 文件 scoped rustfmt 检查
`/tmp/luna-tools-ycozzwm6/result.json` 退出 0。未运行全 workspace 或性能基准。

该改动没有提供跨查询结果缓存，也没有使类型观察本身可暂停；未来区域／会话接线须
继续让观察表归查询所有，不能重新放回共享世界或 VM 目录。

### 缓存职责

| 缓存 | 键及依赖 | 可以省略什么 | 绝不能省略什么 |
| --- | --- | --- | --- |
| 声明索引/世界准备 | immutable world revision | 重建索引及固定校验 | 使用点契约、局部实例域 |
| 字节码 | definition/body 与编译语义版本 | 重编译已验证 code | 本次执行及实参行为 |
| Type invocation 结果 | pure 承诺、实际输入、相关世界/目标、选定 Eq/Hash | 获准复用的纯计算 | 实参准备、默认供应、接收世界检查 |
| 稳定 query | key、实际观察依赖、结果及诊断 | 未变更稳定计算 | 不纯执行许可、产物存在校验 |
| 资源/布局摘要 | 实例、目标、契约及环境前提 | 重算不变摘要 | 调用者的存活/版本/借用兑现 |

Type 结果过滤是当前策略，不是 Flurry 只能缓存 Type 的新语义。
Eq/Hash 形成缓存键时递归调用结果缓存会需要自己的键，必须有明确非缓存执行路径。
键协议失败、预算不足和碰撞都不得误命中；失败不能污染下一次正常调用。

性能评估至少分 cold prepare、warm prepare、code compile、execution、freeze/restore
和 whole-source compilation。默认测量关闭结果缓存，输入交替以防测到常量返回。
记录分配次数/字节、峰值与释放成本的差别，不从一个微基准推导全编译性能。

## 7. 统一 binder-aware 变换

模板、quote substitution、实例化、inline 展开、builder 复制和局部定义移动都需要
保留绑定关系。目标提供共享的受检重绑定服务，显式声明该变换是否创建新身份。

```text
RebindingPlan {
  declaration_mapping,
  binder_mapping,
  runtime_dependency_mapping,
  control_target_mapping,
  source_origin_mapping
}
```

遍历一个构造时必须覆盖类型、默认值、子句、属性、主体、模式导出、闭包捕获和控制目标。
服务不自动决定模板产生新名义身份还是引用原身份；语义调用者先给出策略。
在所有目标预留身份后再搬迁正文，支持嵌套和前向引用；失败不发布半份映射。

### 共同结构描述不等于一个万能遍历器

现有 [NIR visit](../compiler/nir/src/visit.rs)已经区分完整表达式遍历、单个 runtime scope、
闭包环境和 handler scope；这些差别不能在重构时压成“递归访问所有子节点”。
[局部定义复制](../compiler/elaboration/src/program/function_templates/local_definitions.rs)
也已经先预留 owner/声明身份，再复制主体。目标是把这些有效约束统一成受检接口，
不是假定已有实现完全没有绑定管理。

| 遍历用途 | 要覆盖的边 | 必须保留的界限 |
| --- | --- | --- |
| 存储保活 | 节点、代码、类型、证据、来源中的全部存储引用 | 不因当前控制流未执行而漏保活；也不把物理引用自动登记为语义依赖 |
| 绑定改写 | 引用及其 binder、域、默认值、契约、模式导出、控制目标 | 每条边按所属作用域处理，不能仅按相同整数编号映射 |
| 精确自由依赖 | 真正影响相应检查/执行的引用及依赖闭包 | 嵌套声明区分定义与调用；不把无关外层输入一并捕获 |
| 运行行为分析 | 当前执行 scope 内的操作和显式进入的子行为 | 捕获表达式与闭包主体、handler 安装与 arm 执行不是同一时刻 |

节点种类的字段描述可以共享，每种用途必须明确覆盖策略与进入/退出 scope 的事件。
源码新增变体时，穷尽匹配或等价的覆盖机制要求维护者处理新边；不以默认忽略掩盖缺项。
子树共享只省结构读取，不能省略不同环境下的观察或绑定检查。

### 身份映射必须区分保留、替换和新建

```text
MappingEntry = Preserve(identity, availability)
             | Substitute(formed_value, scope_evidence)
             | Fresh(target_identity, formation_owner)
```

该描述不规定新的源码语法。保持同一个已存在 runtime 对象的依赖身份，与为一次新执行
建立局部位置，是不同选择；不能把所有 RuntimeId 一律 fresh，也不能全部保留。
新鲜 binder 的目标互不碰撞，替换可以把多个输入映到同一个已合法的值，不要求替换单射。
定义端刚性参数不因模板复制而获得被求解器赋值的权限。

按 telescope 顺序改写参数域，进入 binder 后才使它对后续项可见；删除已消费 comptime
参数时同步改写剩余域、结果、契约及环境取得路径。pattern 的导出绑定也是有顺序的
scope 增量，后续 pattern 可以依赖先前绑定，不能把整组名字无差别提前放入环境。
CodeValue 的 ordered Ψ、pattern 导出与 expr 消费仍按各自方向处理。

词法 return、循环 break/continue 与 handler/continuation 目标使用独立映射。
不能用局部变量映射顺便改控制编号；进入闭包主体会改变 return 边界，而计算闭包捕获
表达式仍处于外层控制环境。现有 [inline_return](../compiler/nir/src/inline_return.rs)
对这些边界的处理是迁移资产，不应因“统一 visitor”而删除。

诊断来源保存原定义及展开链，不用新存储编号覆盖原来源。反之，同一个源码位置出现
两次动态执行，也不共享 RequestId；动态执行身份由 activation/执行序号产生，不能
从复制的来源节点取得。保留源码来源不代表复用执行结果。

### 证据迁移不是单纯重命名

变换交付新 IR 时，对每份使用到的证据作明确处置：

| 处置 | 准入条件 | 接收方得到什么 |
| --- | --- | --- |
| 保留 | 结论、前提、世界观察、目标和相关 IR 内容没有变化 | 仍与同一事实绑定的原证据 |
| 受检迁移 | 证明系统支持此次合法重命名/替换，形成条件和前提同步迁移并重放通过 | 对新结论成立的新证据 |
| 重新检查 | 变换改变依赖、控制、资源路径或适用域，原证明不再直接适用 | 新检查产出的证据；未完成时不交付可执行产物 |
| 消除 | 受检变换证明对应义务已不再需要 | 消除依据，不是任意丢弃旧义务 |

通用重绑定器只生成候选映射和受影响范围；各证据检查器判断能否迁移。
PTC 证据不能为环境存活背书，资源证据也不能为类型等式背书。
若局部重放失败，应重新检查或拒绝该变换，不把旧证据强行标成新 IR 的证据。
候选可以私下带未决项，但未闭合必需证据的结果不具有 `ValidatedInstance` 交付资格。

例如把一个实参值临时绑定后多次引用，与把它的求值表达式复制多次，类型可能相同，
执行次数却不同。前者仍需相应访问/资源许可，后者不能靠 alpha-renaming 获得许可。
参数或字段物理消去也不能顺便删掉源求值和 Drop；证明纯性不自动证明有限完成。
这些是变换本身的语义义务，不能只交给机器 ABI 或最终数值测试发现。

新生成代码仍经 normalization/safe 检查；已检查 IR 的合法变换维护相应证据，
verified 模式另按既定阶段检查。不能用“后面还会验证”跳过定义端应完成的 safe 义务。
变换失败保留原 IR 与证据，不能交付新主体配旧环境/cleanup 的混合结果。

### 精确词法依赖

以声明为单位收集签名、参数域、默认值、契约、主体和属性的自由引用；对局部调用形成
依赖边，求递归组的必要依赖闭包。保留 required-domain 的前置依赖，不能只扫描函数体
出现的变量名；也不能把所有外围子句直接复制成隐藏参数。

局部 `flag() { true }` 若不依赖外围 `.inverted`，在该默认值中调用不应等待 inverted
自身。若 helper 的契约或调用链确实依赖它，必须保留真实循环/不可用诊断。
结果用于模板复制和闭包环境规划，仍分别处理 comptime 输入与 runtime capture。

## 8. CMTT 的值与操作

CodeValue 的静态描述和实际证据分开，实际值始终保存 expr/pattern/arm 类别、ordered Ψ、
主体、固定外部依赖、控制接口和来源。Any 容器读出的值只能使用该次读取对应的证据。
pattern 接入产生新绑定；expr 接入消费已有上下文，不能共用错误方向的 substitution。

单洞是每段 quote 的构造限制，不是上下文只能有一个变量，也不是整个生成器只能组合
一次代码。新的组合仍需逐层满足既定限制；实现不能为方便 regex 生成静默放宽语法。
quote fn 的普通实参先建立新鲜绑定，主体重复引用也不重算实参。
生成自动机的验收应观察真正组装后的分支/状态代码，而非用解释 regex 字符串绕开生成。

生产侧的 contextual-code 分类现集中在 cmtt/generators/classification：expr 与 pattern
共用携带类别的 HIR 遍历，保留各自输入／输出接口及局部绑定判别；数据消费者沿同一
可失败的方法观察路径取得入口。分类只决定后续检查路径，不授权执行选中方法；实际
调用仍重放 implementation 证据、检查访问权、建立接收者及实参帧。

code_producer、pattern_code_producer、data_consumer_call 及其方法／owner 观察返回
Result。遍历深度／工作量失败、静态类型观察失败与固有方法查询失败向上传递，不能
通过 `.ok()`、布尔 false 或 Option::None 转入普通数据求值。已开始的固有方法查询
遇到 Deferred 等未决选择时报告未完成；只有确定无候选、非 contextual 方法或尚不
适用该观察的语法才返回没有分类。computed-container 观察仍要求所有可能容器的类别
一致，不执行分支条件、生成器、默认值或实参来猜测类别。

这是既有 checker 的诊断边界迁移，还不是可暂停的分类查询：未决分类尚未注册活动
Region 的 waiter；静态类型 hint 内部用于表达“此观察不支持”的可选边界，以及固有
方法查询的独立 PTC 额度仍待统一。不能据此宣称通用 ArgumentPlan、可恢复 invocation
或整个 CMTT 生命周期迁移已完成。

分类迁移验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- check -p elaboration -p luna
  --tests`：退出 0；`/tmp/luna-tools-3as58ssk/result.json`。
- 最终执行同一入口 `test -p luna --test cmtt --test cmtt_methods
  --test cmtt_phase_boundaries --test cmtt_type_inputs --test inline_pattern_observation
  --test quote_functions --test library_protocols --test library_meta_math
  --test sysroot_preludes --no-fail-fast -- --skip
  code_method_parent_value_clauses_form_indexed_code_signatures --skip
  generator_bit_compound_writeback_ --skip
  generator_bit_update_frame_preserves_selector_and_rhs_objects --skip
  inferred_producer_value_defaults_preserve_prior_inference_variables`：938 项通过，
  明确筛去下述 7 个已复现的基线失败，退出 0；`/tmp/luna-tools-e9un_k2j/result.json`。
- 17 个修改／新增 Rust 文件通过 scoped rustfmt check，退出 0；
  `/tmp/luna-tools-8dydxfpy/result.json`。git diff --check 通过。

未筛选的修改后运行 `/tmp/luna-tools-incssk35/result.json` 退出 101：cmtt 为
588 通过／6 失败，cmtt_methods 为 151 通过／1 失败，其余两组 27 项通过。
cmtt_methods 的 indexed-code-signature 用例在修改前即于 139..143 报未实现表达式，
对照 `/tmp/luna-tools-beoh2yks/result.json` 同样退出 101。为核对其余六项，将本次
13 个已修改生产文件临时替换为 `7441aa97` 内容（新增分类模块此时未被引用），
保留其他工作区，执行 `test -p luna --test cmtt --no-fail-fast`：
`/tmp/luna-tools-kdkooysw/result.json` 退出 101，同样 588 通过／6 失败。
其中四个 generator_bit_compound_writeback_*、一个 generator_bit_update_frame 用例
及 inferred_producer_value_defaults_preserve_prior_inference_variables 均在相同源码
位置报 interpretation Type。对照结束后逐字节恢复本次源码并核对哈希，未修改预期。
后续 invocation 签名观察清理已修复上述五个 BitVec 用例：失败来自被动类型比较
把尚未取得的关联 Type 视图当成不相等。随后定位的默认值用例也已通过
[required residual 作用域修复](dependency-observations.md#57-required-residual-保留声明值的词法作用域)
恢复：失败实际发生于声明签名形成时丢掉抽象值权限，并非调用推断结果错误。
indexed-code-signature 仍未修复。具体机制、证据与新发现的
runtime 方法传递问题见[代码实例的签名观察](comptime-value-admission.md#签名观察与发布补充验证)；
该传递问题已通过[恢复公共方法签名](comptime-value-admission.md#公共方法身份的重建)修复。

新增源码回归逐预算寻找 expr／pattern／data 分类的资源边界，随后在同一 HIR 上以
新预算完成 elaboration 和执行；深嵌套 quote 检查必须报告分类深度失败。真实 sysroot
还验证递归未决的固有代码方法不会进入普通数据路径，补充独立实现后成功。既有方法
组覆盖接收者、默认值、实参求值次数及写回，未为取得通过提高生产额度。

### 方法分类与公共来源一致

分类器不能跳过普通方法解析已有的 capability 来源规则。此前对抽象 T 的方法分类
直接搜索固有实现；即使声明约束已明确给出 Read 的公共方法，另一个同名固有方法
仍会产生 Deferred 并阻断调用。这里未决的是无关候选，不能用它否定已经确定的
公共接口，也不能通过吞掉所有 Deferred 来修复。

contextual_method_owner 现先复用 context_member_interface，遵守既有约束、父 trait
来源、歧义检查和 comptime 来源优先级。公共声明决定 expr／pattern／data consumer
类别；普通方法或关联值继续走普通调用。只有没有该来源时才进入固有方法查询，
该查询的真正未决与失败仍向上传递。分类不执行实现、默认值或实参。

隐式来源的 contextual 调用与显式 trait view 共用 with_trait_code_method_instance，
不复制另一套主体选择流程：检查 implementation world 就绪、所请求及继承接口的
证据、公共方法类别、访问权和接收者，再临时安装公共／实现捕获和目标映射。
调用结束恢复这些映射。泛型主体检查仍只能使用已成立的公共契约；选择实际主体
的要求没有因分类成功而放宽。

显式 trait view 的分类只读取声明及父接口图，不通过 interface_expr_for 求值实参。
本地方法和本地关联项都会截断该分支的继承搜索；关联函数值不能因为父接口恰有
同名生成器就被当成 contextual 方法。菱形路径按声明身份合并类别观察，多个不同
声明来源则报告 inherited-member 歧义，不能用没有分类来转入普通调用。
这里只观察类别：同一声明的不同接口实参并未因此成为同一来源，实际调用仍通过
完整 InterfaceInstance 和 inherited_member_source 检查来源、替换及证据。

显式、隐式及固有路径取得声明后，共用 contextual_declaration_owner 判断其签名。
已知普通成员返回没有 contextual 类别；所需 trait 契约或成员声明缺失则报错。
这避免把观察所需数据不可用误当成“普通成员”的确定答案。

真实 sysroot 回归覆盖三种类别的直接／继承来源和同层歧义，另检查 pure comptime
trait 的优先级。既有泛型名义值 trait 实参转发用例保持原有成功及拒绝断言。
显式视角另覆盖三种父方法被关联函数值遮蔽、关联项经菱形继承出现，以及普通方法
遮蔽 contextual 父方法；不同来源的 pattern 回归要求准确的继承歧义诊断。
同一 Parent 方法经两条 Parent<true> 路径可正常调用；把一条改为 Parent<false>
则必须在完整接口来源检查时报歧义，不能因分类时声明相同而合并接口实例。
独立查询预算与未决分类 waiter 仍未统一，限制同本节前述迁移边界。

验证工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 900 -- test -p luna
  --test cmtt_methods --test nominal_value_parameters --test trait_phases
  --test library_protocols --test cmtt_phase_boundaries --no-fail-fast -- --skip
  code_method_parent_value_clauses_form_indexed_code_signatures`：567 项通过，
  筛去上述 1 项既有未实现用例，退出 0；`/tmp/luna-tools-ib0a7k7v/result.json`。
- 四份修改的 Rust 文件 scoped rustfmt check 退出 0；
  `/tmp/luna-tools-__km3ko3/result.json`。git diff --check 通过。

首次扩展运行 `/tmp/luna-tools-4b93ojd9/result.json` 退出 101：新增来源优先级用例
误将普通 trait 的 runtime-phase 成员实现放在 comptime impl 中，被阶段检查拒绝。
最终按该 trait 的成员阶段使用普通 impl，保留 comptime 方法自身的执行阶段；
没有修改 phase 检查。未运行全 workspace 或性能基准。

显式视角清理的验证仍在 `/home/fin/luna` 使用同一 check.py 入口：

- 新的关联项遮蔽用例在修改前报 unknown trait code method，退出 101；
  `/tmp/luna-tools-3oszr9qx/result.json`。修复后 `test -p luna --test cmtt_methods
  --test trait_phases --no-fail-fast -- --skip
  code_method_parent_value_clauses_form_indexed_code_signatures` 为 230 项通过，
  退出 0；`/tmp/luna-tools-sb49dfg6/result.json`。
- 扩展菱形／普通方法遮蔽回归后，`test -p luna --test cmtt --test cmtt_methods
  --no-fail-fast -- --skip code_method_parent_value_clauses_form_indexed_code_signatures`
  为 753 项通过，退出 0；`/tmp/luna-tools-arhjyb2w/result.json`。
- 最后补充的 contextual_category_deduplication_does_not_merge_interface_instances
  精确测试通过，退出 0；`/tmp/luna-tools-s6jefsxx/result.json`。该次只增加测试，
  生产源码与上述两组检查相同。两份修改 Rust 文件 scoped rustfmt check 退出 0，
  `/tmp/luna-tools-ruml61pd/result.json`；git diff --check 通过。

## 9. DefinitionTransaction

目标让 struct/enum/newtype/fn/trait/impl 等走共同发布协议：

1. 在批次 owner 中预留声明与 binder 身份，草稿保留未完成结构。
2. 基于固定 WorldSnapshot 创建私有 overlay，解析生成代码的显式引用和来源。
3. 用普通接口检查与 elaboration 验证完整批次；名义互递归用已准入签名，不用占位成功。
4. 验证发布所需的全部接收方（声明目录、VM、实例/协议索引）并准备 receipt。
5. 核对基线 revision，原子发布后继世界；竞争收据失效，旧世界仍可只读使用。
6. 恢复原 invocation，返回可在后继世界使用的句柄/值，不返回活动 checker。

上述安装通过同一 PublicationBundle 版本根交付目录、定义、VM 描述和协议索引；
用户计算与失败性检查都在 prepare 中进行，install 不再逐个调用可失败回调。
旧代码仍绑定原语义视图，后续操作通过 staging plan 显式取得新视图，不能整体改标旧缓存。
逐批安装只授予对应后续位置可见性，最终实例交付仍需整体 coherence。
详见[发布的线性化点](elaboration-state-and-publication.md)。

结构分配失败、语义失败、接收世界拒绝、revision 过期都不泄漏部分声明。
事务保留可修复草稿；预算消费不回滚。预留编号可以留下空隙，不能将已观察身份复用给
不同定义。源码创建和 builder 创建共享检查规则，不为“简单 struct”另造长期语义通道。

### 当前 VM 的具体名义批次共用安装所有权

struct、enum、newtype 已删除各自的 Installation／Drop／commit 实现，改为共同的
`definition_installation::Installation`。借入批次、共享 owned 批次，以及
DefinitionAdmissionSession 已检查 receipt 的实际提交都通过这个边界。类型专用
适配器只选择目录及不可变描述句柄，不另维护安装进度或提交状态。

进入安装前仍由各类定义完成身份冲突、阶段、scope 和结构检查，并预付插入／回滚
工作；enum 编码和 newtype 表示等条件没有合并或删除。共同 guard 先安装整个批次，
使本批次前向引用可见，再执行已有 formation 检查。只有 publisher 的 commit 成功，
guard 才转为已提交；准入失败、commit 错误或 unwind 都只删除本 guard 插入的项。
回滚不申请新的执行预算，不撤销宿主外部副作用，也不退还已消费工作。

guard 继续持有原批次及插入计数，不增加逐定义复制或另一份可变目录。S-expression
dump 显示定义种类、提交状态和已插入身份。公开的各类 publication error 仍区分
Admission 与 Commit；receipt 的 world 身份、revision 检查及竞争收据失效规则保持。

这只统一 VM 安装事务的所有权。混合 definition 同批发布、函数／trait／impl builder、
跨目录与协议索引的 PublicationBundle、宿主请求与活动 invocation 的接线仍未完成。
现有同步 publisher 回调仍可失败，不能把共享 guard 当成目标协议的完整原子发布。

工作目录 `/home/fin/luna` 的验证：

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
  --lib generated_ -- --nocapture`：`/tmp/luna-tools-f9nkw89e/result.json`，退出 0，
  26 项通过。沿用实际发布测试，覆盖借入／owned 描述、前向引用、形成失败、commit
  错误与 unwind、预算耗尽，以及 receipt 的 revision／world 拒绝与失败后恢复。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
  -p luna --lib --test builder_invocations --test generated_definitions
  --test reflected_definitions --test library_meta_objects --no-fail-fast`：
  `/tmp/luna-tools-d0co2xbd/result.json`，退出 101，693 项通过、1 项失败。唯一失败
  仍是上文已复核的 enum builder fixture 缺少 `Symbol` 绑定，尚未进入 VM 发布；
  本次没有修改 fixture 或预期。其余 builder、真实包／prelude、反射和 meta object
  路径通过，不能将完整命令称为全通过。
- 六个修改／新增 Rust 文件 scoped rustfmt 检查：
  `/tmp/luna-tools-6vw9vkw8/result.json`，退出 0。未运行全 workspace、native 或性能基准。

### 源世界的保留身份索引

发布新定义时，活动 VM 目录继续首先决定身份是否已占用。对于不在执行目录中的
trait、Self、方法、关联成员、impl、无主体函数参数表和 quote template 身份，
`ReservedDeclarations` 按不可变源世界惰性建立完整集合，取代每个新身份都扫描
全部元数据的路径。IIR 继续使用已有 graph declaration index。

首次构建按当前 VM 预算收费，只有全部成功才发布集合；预算失败不缓存半成品，
也不把失败解释为身份不存在。后续查找仍支付入口工作，并查询有序集合。对于 R 个
保留身份，构建时间 O(R log R)、空间 O(R)，后续查询 O(log R)；这是结构复杂度，
尚未进行墙钟性能测量。没有实际发布需求的 VM 不触发这项额外构建。

该被动索引通过 CodeSession 的 SourceIndexes 复用，绑定同一 Arc 源快照，不能
仅凭相同声明编号跨世界复用。生成定义仍留在各 VM 的活动目录，成功发布、失败
回滚及查找缺失都不进入这个共享集合。身份保留不授予执行、阶段或证明权限。

工作目录 `/home/fin/luna` 的验证：

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
  --lib reserved_tests -- --nocapture`：`/tmp/luna-tools-bvrw2ss2/result.json`，
  退出 0，2 项通过。覆盖预算失败后重试、各类元数据冲突、预热后有限预算查询、
  精确快照隔离、生成定义提交／回滚的 VM 隔离，以及身份不授予执行权限。
- 与上一节相同的 VM／luna lib、builder、generated/reflected definition 和
  library meta object 集成命令：`/tmp/luna-tools-5024jsbd/result.json`，退出 101，
  695 项通过、1 项失败。失败仍是上述 `Symbol` fixture 问题；未修改其预期。
- 七个修改／新增 Rust 文件 scoped rustfmt 检查：
  `/tmp/luna-tools-44hr2uh7/result.json`，退出 0。没有进行性能基准测量。

## 10. 删除旧路径的条件

| 旧路径 | 删除前必须看到的生产行为 |
| --- | --- |
| NF 专用表达式/值求值 | 普通 callable、用户 NF、Tuple 与 List 经过同一计划及冻结路径 |
| invocation 的树 fallback | 支持范围内每种操作均走字节码，拒绝在执行前确定，次数/失败一致 |
| per-kind builder 桥接 | 多种 definition 在同批发布及活跃调用恢复时用同一 receipt 协议 |
| 手工零散替换 | 嵌套 binder、契约、属性、控制目标和来源均经统一变换验证 |
| 全量外围捕获 | 递归局部声明的精确依赖和真实循环均有源码证据 |

混合 interpretation、runtime reference、原子发布及接收世界检查不在删除名单中。


### 源码轨迹与显式绑定

前文历史检查中的两项验证问题现已修复：enum builder 独立 fixture 缺少 `Symbol`
绑定，以及 deferred normal input 用例要求固定的 contextual input 总数。保留历史
失败报告用于说明当时的证据，不能继续将这两项列作当前未解决基线。

类型用例保留原始值、NF 头和参数顺序断言，增加有名字的默认值及域观察函数，用完整
`EnterFunction` 轨迹核对执行一次及顺序。Ready 的域观察先于值生产，Deferred 的值
前缀先于域观察；提前返回不进入后续域生产者。相同源码在树解释与字节码中都执行，
并覆盖切换元素类型后重复调用。字节码计数只检查实际源码调用进入了字节码、默认值
执行一次及确实使用了 contextual／staged 路径，不再限定某次 IR 划分产生几个输入。
这没有把旧预期 2 改为 3，也没有修改编译器或语言契约。

enum builder 用例从该 fixture 已配置的 builtin 依赖显式导入 Type、u8、Symbol，
替代局部原始绑定并补齐共享示例的符号类型。继续运行原有反射结果与 runtime pattern
断言；真实 sysroot 路径仍由既有独立用例覆盖，不将隔离 fixture 冒充完整 prelude。

工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 900 -- test -p luna
  --test comptime_bytecode_types --test builder_invocations --no-fail-fast`：
  `/tmp/luna-tools-ttr13p5t/result.json` 退出 0，101 项通过。
- 临时把新类型 fixture 的 `enabled()` 改成 `enabled() and enabled()`，保留相同
  返回值并重复源码调用；单独运行该用例在 `/tmp/luna-tools-9w0ht1mz/result.json`
  退出 101，结果断言通过，源码轨迹准确检出重复的 default_flag／enabled 调用。
  随后逐字节恢复最终测试，同命令在 `/tmp/luna-tools-n_lk4nl3/result.json` 退出 0。
- 两份 Rust 测试文件 scoped rustfmt 检查：
  `/tmp/luna-tools-npc50epa/result.json` 退出 0。未修改生产代码或运行全 workspace。

以上仅修复验收载体。通用域等待、前端可恢复区域和完整宿主协议的未完成范围保持。

## 当前 NIR 迁移：残余值不等于冻结数据

[依赖观察 §5.5](dependency-observations.md#55-invocation-残余值与冻结值分离)已接通
`StagedValue(Term)`：共同替换按 invocation 槽是否仍存在区分残余表达式和冻结值，
实参供应后再恢复普通值表示。它不新增源语言的值种类或新的缓存资格，也不是完整
OpenDefinition/WorldView 已经落地。冻结入口、实际执行入口及 IIR 仍要求相应完成证据，
不能借参数域已知就取得还没有供应的输入值。


## 当前显式 invocation 迁移：统一源世界 VM

普通 CLI／`luna::invocation` 与 `PreparedInvocation` 已删除“同时遇到普通 Type
参数和 trait／关联值就先实例化整份 IIR”的分流。准备对象只持有源 package、入口身份
与源代码 session；执行统一通过源世界 VM 绑定本次实参、使用 demand resolver 选择
方法和关联值，并运行原有类型与程序契约。`needs_materialized_dispatch`、可选 IIR
字段、两类构造器和两套调用出口已从普通入口删除。

这条分流的历史前提是源 VM 尚不能按实际参数解析方法。现在该服务已存在，继续在
入口根据可达语法扫描切换世界，会使同一种 comptime 调用受到两套准备责任约束。
普通入口仍调用 `dispatch_instances_for_invocation` 准备可用 dispatch，并保留已接纳的
实现关系见证；这不是执行 IIR materialization。代码 session 复用代码，调用值、局部
存储、契约状态与 builder 服务仍在每次 invocation 中建立。

带程序前提的独立 callable 转换仍遵循[函数值契约边界](../docs/functions/comptime_fn.md)：
当前普通函数类型未携带该前提时拒绝转换。IIR 不提供擦除前提的许可，不能把这个拒绝
误认成必须保留整条 comptime IIR 分流的理由。直接方法调用继续消费接口契约，合法的
无前提方法值可以保存和经高阶参数调用。完整携带程序契约的 callable 仍是后续能力，
本次没有通过删除检查来扩充其支持范围。

移除分流后，VM 构造的诊断责任变得明确：`nir::phases::validate` 的具体失败现在通过
`ErrorKind::InvalidPhase` 保留原因与源码位置，`Display` 输出原始原因。此前全部被压成
PhaseMismatch，导致“方法值会擦除接口契约”等形成错误失去解释。实际执行错阶段仍
使用原 PhaseMismatch；构造期验证与调用期权限错误分别报告。同一不可变 source 的
验证成功证据仍由 code session 持有，失败不会安装索引，也不复用同号但不同世界的证据。

重复声明的 VM 集成检查也按此协议验收：必须拒绝构造，保留
`duplicate function declaration` 的具体原因，并在外层错误与内部 phase 原因之间
保留一致的源码位置；Display 不能再次丢失原因。同一个非法 snapshot 重试、用合法
snapshot 的既有 session 尝试非法 snapshot，都不能获得成功索引，合法原世界随后
仍可使用。构造入口目前采用保守预留：失败时 caller budget.steps 保持 0、depth
配置不变；它不同于 invocation 内部已迁移的实际消耗回传，不能据此声称构造失败
已经精确计费。

`instance_budget` 和 `runtime_function_identities` 中残留的两项 PhaseMismatch
断言已更新到上述更具体的拒绝契约，重复声明仍然拒绝，调用期的 PhaseMismatch
断言仍按其原有权限边界检查。两份完整集成组共 53 项通过，目录 `/home/fin/luna`，
命令 `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor
--test instance_budget --test runtime_function_identities --no-fail-fast`，退出 0；
`/tmp/luna-tools-wlh0bw22/result.json`。本次修正的是过期断言及覆盖范围，生产实现
没有放宽。此前失败证据保留在[字段与代码准入](comptime-value-admission.md#34-iir-callable-查询与代码复制视图)。

这一步首先迁移普通显式 invocation；当时保留的 `memoization::staging` Type 输入
IIR 专门化现已在下述后续迁移中删除。运行时 IIR、后端交付以及引用执行器继续使用
各自协议。完整通用参数计划、动态 Type producer、调用中的世界发布和树 fallback
退役均未因此完成。

### 本次验证与未修复基线

工作目录均为 `/home/fin/luna`；下表命令参数接在
`python3 -B development/check.py --timeout 900 -- test` 后。
证据目录中的 `result.json` 保存完整命令与退出结果，`stdout.log` 保存断言输出。

| 参数／范围 | 结果与证据目录 |
| --- | --- |
| `-p comptime_interpretor --lib --no-fail-fast -- --skip normal_input_data_tests::clause_list_inputs_share_domain_scope_and_budget_validation` | 退出 0，446 通过、1 项已知基线跳过；`/tmp/luna-tools-6ce5etz8` |
| `-p luna --test execution -- ordinary_type stored_staged_methods_reject_erased_interface_contracts conditional_method_signatures_are_materialized_only_when_called`，另跳过下述 3 项 Type 前端基线 | 退出 0，19 通过；`/tmp/luna-tools-jqapa_na` |
| `-p luna --test prepared_invocations --test invocation_value_templates --test comptime_pointer_phase --test comptime_storage_domains --test comptime_iir_bytecode --test comptime_bytecode_defaults --no-fail-fast` | 退出 101，36 通过，1 项仍断言旧 PhaseMismatch；`/tmp/luna-tools-ghtesw9n` |
| `-p luna --test comptime_bytecode_defaults` | 补充默认参数顺序／绑定帧错误的具体原因和来源断言后，退出 0，9 通过；`/tmp/luna-tools-ywr1egel` |

新增真实 sysroot 用例同时覆盖 Type 驱动的 trait 方法／关联值、保存并高阶调用方法值、
重复 invocation 的新鲜局部状态，以及预算失败后恢复。既有接口前提擦除拒绝例在迁移
初期失去具体原因，补齐 InvalidPhase 后原断言通过；没有放宽 callable 转换契约。
默认参数拒绝测试分别检查“默认参数顺序／绑定帧错误”和“显式参数出现在默认参数后”，
保留引用执行器自身的 InvalidCallPlan 防御检查。

扩大检查发现以下 5 项源码前端失败；临时恢复本阶段改动前的生产文件后，原失败重现，
随后恢复当前实现。它们都在 invocation 前失败，本次未修改其预期或实现：

- `enum_builder_edits_payloads_groups_and_runtime_patterns`：fixture 中 Symbol 未解析。
- `known_type_list_index_failures_remain_invocation_failures`：静态观察整数不能以目标定宽表示。
- `ordinary_type_inputs_do_not_hide_selective_memory_writes`、
  `ordinary_type_memory_summaries_forward_across_different_input_positions`：程序后置条件未证明。
- `ordinary_type_list_annotation_inputs_report_the_phase_boundary`：required comptime 输入不可用。

前两项基线复现为 `/tmp/luna-tools-lbxrkexa`，后三项为 `/tmp/luna-tools-j0r5g11s`，
均退出 101。VM 跳过项是此前已确认的名义列表域失败分类差异，仍待处理；不能将上述
局部成功称为全量通过。修改的 Rust 文件格式检查与提交范围的 `git diff --check` 通过；
正式 docs、已有资源分析修改及 library README 保持原样。

## 当前缓存 invocation 迁移：去掉 Type 输入专门化世界

`memoization::Invocation` 现在也只持有一个源 package 与对应 CodeSession。准备阶段
用 `dispatch_instances_for_invocation` 保留合法输入模板；执行时，声明实参与本次普通
实参直接交给源 VM，方法和关联值通过现有 resolver 按实际输入选择。结果的成员资格
与冻结准入由执行它的同一 VM 检查，不再为 IIR 返回值另建一个源 VM。

已删除 `memoization/staging.rs`、`prepare_bound_entry`、`admit_source_result` 和最后
一个 `needs_input_staging` 消费者及其可达性扫描。Type 实参变化不再触发 CLI 层复制
package、挑选伪造私有声明身份、移除 Type 实参并实例化整份 IIR。旧的
`staged_preparations` 计数 API 一并删除；实际代码编译／执行统计仍由
`last_execution_stats` 提供，结果缓存数仍由 `cached_results` 提供。

移除旧路径时观察到依赖注解主体每次重编译的问题，因此同步补齐 VM 的主体准备缓存。
缓存按原声明或精确 owned body 身份定位，再比较完整 FunctionInputBinding，包括
替换项、专门化后的签名和槽位域细化。签名相同不等于绑定相同；Type、普通值索引及
动态成为 Type 域的输入不使用互相独立的特判键。新添的只读 `terms()` 观察用于预算
预检，完整绑定相等仍是命中条件，不能只比较该观察返回的项而遗漏槽位细化。

每个主体保留至多一个依赖绑定对应的已准备主体，返回相同 Arc 供现有 owned bytecode
缓存复用。比较与复制前按当前预算检查双方键，存储权重同时计入主体和绑定，总量沿用
既有上限。成功替换时扣除原项权重；预算失败或重绑定失败不替换旧项。抽象解释中的
依赖主体不发布到该共享缓存；owned source 使用弱引用核对身份，不因缓存延长源主体
寿命。这不是新的类型等价关系，也不保存本次局部值、可变存储、契约执行结果。

不依赖输入的主体仍能跨 Type 实参复用。依赖主体的重绑定缓存与字节码缓存只消除准备
工作，输入准入、调用契约、实际执行和结果检查仍按本次调用进行。Type 结果缓存保持
原策略：只接受纯函数的已绑定 Type 结果，用实际输入与源 Eq/Hash 协议判定复用；
非 Type 结果不查询结果缓存。没有扩大缓存资格，也没有把 Type 实参变成 NF 特殊槽。

此迁移不等于完整通用参数计划已经实现。memoization 的输入域绑定仍是现有适配器，
一般值依赖域、待定 Type producer、定义发布与可恢复 invocation 的后续迁移继续遵循
本章目标。主体缓存是精确源世界内的有限复用，不承诺跨编辑持久化或无限保留实例。

### 验证范围与仍未接通的路径

工作目录 `/home/fin/luna`，命令前缀同上。

| `check.py ... test` 参数／范围 | 结果与证据目录 |
| --- | --- |
| `-p comptime_interpretor --lib --no-fail-fast -- --skip normal_input_data_tests::clause_list_inputs_share_domain_scope_and_budget_validation` | 退出 0，449 通过，跳过此前已确认的 1 项列表域失败分类基线；`/tmp/luna-tools-2wvj6e01` |
| `-p luna --test invocation_value_templates --test staged_invocation_reuse --test prepared_invocations --test comptime_memory --no-fail-fast` | 退出 101；存储 14、prepared 3、reuse 4 项全部通过；新 fixture 的 Type 键协议准备失败，见下文；`/tmp/luna-tools-um389xj0` |
| `-p luna --test invocation_value_templates` | 调整新用例的输入选择后退出 0，4 通过；`/tmp/luna-tools-7llybjxc` |
| `-p luna --test execution -- memoized` | 退出 0，13 通过；`/tmp/luna-tools-y5vs07g_` |
| `-p luna --test comptime_bytecode_defaults --test comptime_iir_bytecode --test comptime_storage_domains --no-fail-fast` | 退出 0，9 + 10 + 7 通过；`/tmp/luna-tools-5uljt7zd` |

移除 IIR 后最初出现的重复字节码编译由原测试检出：
`memoized_invocation_checks_dependent_type_contracts_before_caching` 在
`/tmp/luna-tools-o3j4u847` 失败。补齐共享主体缓存后，其原有“再次调用不编译字节码”
断言通过，未删除或放宽该断言。删除的只是已经不存在的 IIR preparation 计数；
不依赖输入的源码主体现在也要求在 Type 实参变化时继续复用代码。

新增真实 sysroot 用例以 bool 作为结果缓存键，运行分支中的 `SelectFor(bool/u8)`，
由 Type 实参选择 trait 方法值并与关联值核对；重复键命中、错误输入拒绝、关闭结果
保留后的执行／准入计费、预算耗尽及恢复均有断言。内部缓存用例另覆盖相同签名但不同
绑定、普通 bool 值索引、精确源主体身份、权重替换、失败不覆盖以及抽象调用不发布。

编写该用例同时确认了未完成能力，不能据此声称任意 Type 输入缓存已经可用：

- 当时 meta 的 ComptimeEq／ComptimeHash 都没有 Type 实现。后续已接通
  [Type 的 ComptimeEq](libraries-and-delivery.md#12-type-协议比较接入既有类型等式)，
  ComptimeHash 仍未接通，直接为 Type 输入准备完整键协议仍会失败；不能提供恒定
  Hash 占位实现掩盖缺口。
- `SelectFor(if flag { bool } else { u8 })` 的 required Type 输入当前不能依赖尚未供应
  的外层 flag，报 `required comptime input ... is not available`，证据为
  `/tmp/luna-tools-ng7jwxcd`。分支内的具体 Type 调用已通过，不代表一般待定 Type
  参数计划已接通。初始 fixture 的 assumes 中直接混用 Type 命题也未被当前前端接受。

上述新 fixture 的探索未改动前端规则、库协议或既有测试的语义预期。Rust 格式检查
`/tmp/luna-tools-yqcdaez2` 退出 0，提交范围 `git diff --check` 通过。已核对旧 staging
模块及扫描／准备函数不再有 Rust 消费者；正式 docs 和先前已有修改保持原样。

## 当前键协议迁移：共享代码与逐次执行分开

`comptime_keys::Protocol` 不再为每次快照准入、Eq、Hash 从零准备源世界与代码。
固定 Binding 持有协议选择、类型域、证明上下文、实现索引和 Arc<Package>；独立的
Execution 持有绑定到该精确 package 的 CodeSession 与最近 VM 操作统计。普通
invocation、缓存 invocation、键方法使用同一个 demand method resolver，删除键路径
重复维护的选择／计费适配逻辑。

每次操作仍构造新鲜 VM，安装原类型证明、方法／关联值和 Copy 证据服务，再检查本次
输入与执行协议正文。CodeSession 仅复用源索引、已准备主体和字节码，不复用用户
Eq／Hash 返回值、程序契约结论或接收者存储。构造失败保持原预算预留规则；执行失败
由同一代码 session 退出协议收回代码，按 VM 实际消费同步调用者预算。输入复制的
预算预留若在进入 VM 前失败，不伪造一次已执行的 VM 操作。

为显式表达这份可变执行缓存，Rust `Protocol::admit` 改为 `&mut self`，memoization
中相应的协议列表也通过可变借用准入新值，不引入隐藏的内部可变会话或共享活动 VM。
Snapshot 的 owner 仍属于唯一 Protocol，两个 Protocol 即使来自同一源 package，
也不能混用快照。`last_execution_stats` 返回最近一次 VM 准入或方法执行的字节码
编译／调用次数；在进入 VM 前被拒绝的操作不会产生新的执行统计。

本次没有改变结果缓存策略、源 Eq／Hash 的语义权限或 Type 哈希缺口。CodeSession
只在其精确不可变源世界内有效；同号声明在新世界中的代码不能继承旧缓存，也不能
反过来改变仍存活的旧 Protocol。

### 验证

工作目录 `/home/fin/luna`，以下参数接在
`python3 -B development/check.py --timeout 900 -- test -p luna` 后。

| 参数 | 结果与证据目录 |
| --- | --- |
| `--test key_code_sessions --test library_list_hash --no-fail-fast` | 退出 0，3 + 13 通过；`/tmp/luna-tools-qqoqdh14` |
| `--test key_code_sessions --test library_list_hash --test invocation_value_templates --no-fail-fast`（迁移初次检查） | invocation_value_templates 的 4 项通过；另有新预算断言与旧成本测试的测量前提需调整，整条命令退出 101；`/tmp/luna-tools-t5g4c3m7` |
| `--test execution -- memoized comptime_key comptime_snapshot pure_query` | 30 通过，1 项复制成本断言失败，退出 101；`/tmp/luna-tools-yyiiga0m` |
| `--test library_hash --test library_comptime_comparison --test comptime_memory --no-fail-fast` | 退出 0，18 + 3 + 14 通过；`/tmp/luna-tools-8yqcdrlt` |

新真实 sysroot 用例检查：重复 Hash／Eq 不重编译但仍执行方法，交替输入不复用答案，
方法修改 self 不污染保存的输入快照，程序条件逐次检查且失败后恢复。同号声明修改
主体后新旧 Protocol 使用各自代码，交叉快照拒绝。预算检查覆盖冷／热调用、输入预留
拒绝、执行中耗尽与恢复；热调用仍消耗预算。

原共享载荷成本测试比较小／大 payload 时，首次调用包含代码编译、后续调用已预热。
现在在两种大小测量前均预热代码，再保留原成本相等断言，并断言测量时未重编译；
没有改为不等式来掩盖逐字节复制。新预算用例按 Context 的既有预留契约区分尚未进入
VM 的失败与已执行工作的计费，不改变 Context::spend 的实现。

本阶段扩大检查时唯一未处理的失败为
`comptime_snapshot_admission_checks_enum_tags_payloads_and_nested_references`：
包含未知 Function 身份的 tuple，其 `immutable_copy_work()` 当前并不返回 None。
恢复本阶段之前的 comptime_keys.rs 后，`/tmp/luna-tools-2b2_r_h3` 仍在同一断言失败
（退出 101），之后已恢复当前实现。本次未修改该预期；复制成本与世界准入的责任边界
需单独核对，不能据此声称完整回归通过。

后续已完成这项责任核对：复制成本只检查表示，未知身份应由世界准入拒绝。
过期成本断言及新增正反例见
[函数身份的成本、准入与执行权限](comptime-value-admission.md#20-函数身份的成本准入与执行权限)。

Rust 格式检查 `/tmp/luna-tools-k8e_6o4o` 退出 0，提交范围 diff 检查通过。
正式 docs、已有资源分析修改及 library README 均保持原样。
