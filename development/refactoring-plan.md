# 重构顺序、退役条件与验收

作者已要求开始清理；在已收敛设计上分批推进源码退役与生产迁移。
目标不是换一个更整齐的兼容壳，而是逐步消除已确认的语义偏差及信息丢失。

## 1. 总体顺序

先确立身份、稳定产物和事务所有权；以普通参数/值/类型为主干接通执行和生成；
再把运行时依赖、资源、ABI 和真实库联成完整交付。稳定 query 的边界从一开始参与，
持久化和并发放在生命周期及证据协议成熟之后。

每步为可验收的纵向迁移，不要求一次重写全编译器；也不允许只新增未被使用的平行类型。
阶段编号是依赖顺序说明，不恢复 graph engineering 或调度账本。

## 2. 阶段与删除边界

| 阶段 | 实现范围 | 必须真正替换的旧路径 | 完成证据 |
| --- | --- | --- | --- |
| M0 语义与架构整理 | 本目录、证据核对、文档退役 | 时间流水账充当现行设计 | 当前状态、目标、未完成和证据分开；不改变语言设计或正式 docs |
| M1 身份与产物所有权 | stable world、region、candidate、来源与受限产物构造 | 跨边界共享临时状态、隐式当前世界 | 失败/取消不泄漏；不同世界同号句柄拒绝；生产入口使用新所有者 |
| M2 通用参数和值 | ArgumentPlan、ContextualExpr、freeze/restore、binder 变换 | NF 专用表达式/TypeList 权限、重复参数准备 | 用户 NF 与真实 Tuple/List 同路径，域/顺序/默认/失败一致 |
| M3 交织执行与发布 | invocation VM 帧、宿主请求、DefinitionTransaction、精确词法依赖 | invocation fallback、per-kind 发布旁路、全量外围捕获 | 活跃调用中生成并调用定义，重复唤醒无重放，子句 helper 正例通过 |
| M4 生产 TypeRef 与 IIR | 类型引用、lift 生产、EnvironmentPlan、NIR safe 接线及高级实例 IR | 普通注解仅 Term、仅身份集合的依赖交付 | 同 core 不同对象跨调用/捕获/返回，safe 证据与环境共同进入 IIR/后端 |
| M5 资源和库贯通 | 来源/效果摘要、text region、cleanup、inline effect、allocator 边界 | pointer-only 消费者、str 直降绕路、重复来源猜测 | 真实 sysroot 文本及迭代、所有出口清理、死亡来源拒绝 |
| M6 目标与工程交付 | ABI、pipeline、artifact 依赖和稳定 query 覆盖 | CLI 层隐含语义、只缓存整前端而缺细粒度依赖 | 改主体/接口/impl/配置各有正确失效；native O0/O3 和产物恢复 |
| M7 退役与规模验证 | 删除已替换实现、停用遗留目录审计、持久化及并发接入 | 两套生产真相、临时 adapter、未被消费者使用的新结构 | 唯一生产接线、跨编辑/失败恢复、规模成本与完整回归 |

PTC 的完整理论协议和 Type 模型搜索沿 M1–M4 持续推进：M1 定 world/guard/候选事务，
M2 接通普通值域，M3 处理可恢复宿主观察，M4 消费 TypeRef 及身份域。
不能等所有模块重写完才做 solver，也不能用不完整 PTC 给资源层提供伪证明。

M5 是资源机制与库场景的进一步贯通，不是推迟 M1–M4 所涉代码的 safe 检查。
每个阶段交付均要满足相应 normalized IR 的 safe 义务；IIR verified 与后端证据维护
不替代它。具体 TypeRef、环境和消费者迁移见[类型引用与环境](type-references-and-environments.md)。

## 3. 当前迁移使用的架构边界

M2 先接通[统一值准入](comptime-value-admission.md)及普通参数的真实消费者，保留既有
带域 list 编码和闭包共享快照；再让 M3 的宿主响应、代码值与发布使用相应传送契约。
不能先删除 normal_terms/normal_values 中的检查，或把活动值引用统一复制成冻结数据。

Type 描述迁移跨 M2/M3，完整接线按
[共同冻结载体与 binder](type-descriptions-and-admission.md#9-共同冻结载体与-binder-的实施选择)
推进：VM 返回、聚合／NF 数据边、普通输入绑定、身份消费者和源码注解一起闭合。
`627ab804` 已让输入齐备的 required Type 返回走普通调用，不再临时建立推断输出；
旧 Type 队列仅保留为独立测试模型，已退出生产模块。不能继续给每种描述加一个队列状态就宣布完成，
也不能只放宽 List 的 sort 检查；冻结值及调用绑定必须先能保留描述的实际含义。
`std.fmt.Parse(format)` 已能在格式串确定后返回含字段约束的类型描述，
`println` 按调用方的实际参数逐字段准入。该路径的格式串可以由 comptime 函数计算。
D03 仍有另一种输入顺序：一个尚待检查的源码实参需要正在执行的 Type 生产者给出接收域。
此时需保留原调用者的检查位置和 VM 激活帧。下文的 `count` 和 `identity` 分别覆盖
收集器与被调签名的恢复边界。

在 M3 的声明世界中，trait 的普通方法、表达式生成器、模式生成器和消费代码的数据函数
可以同时存在。源码检查可以借用任何一类方法的程序规格；VM 函数值和 IIR 调用要求普通
签名。源码区域暂停时必须保留原方法类别、实参绑定与代码上下文。把整个声明世界预先
投影成可执行函数表，会让未调用的生成器影响普通表达式检查，也会丢失恢复时所需的
绑定身份。具体检查见[方法规格视图](comptime-value-admission.md#函数身份与源码契约)。

源输入已经形成也不等于 runtime 值已经构造。[阶段交付设计](runtime-value-delivery.md)
把参数/常量/结果的消费角色放在 M2，把受检 Splice 调用、捕获和契约残余接入 M4，
再与 M5 的目标存储来源和真实文本 writer 贯通。现有 `iir::staging::wrap` 不能先删：
必须先接通基础构造终点，避免标量身份实现自递归，再逐项接管主体、契约、默认值和
捕获。对照已可执行，直接返回 comptime 参数仍是待修正例，不改变既有待定域主线。

布局观察在 normalization 的反射和 safe 检查中已经需要，不推迟到 M6 才设计。
M4 接通 TypeRef/环境与布局计划，M6 完成目标调用分类与交付；具体生产者、消费者和
repr_at 退役条件见[布局与调用 ABI](layout-and-call-abi.md)。仅新增一个布局容器不算迁移。

构建入口和输入身份从 M1 起参与，不能到 M6 才补 build 来源。M6 集成
[任务与产物协议](build-and-artifact-delivery.md)，在保留现行 wire/resident v1 及
单文件输出保护的前提下接通字段观察、IIR lease、完整 manifest 和恢复消费者。
既有工具超时/输出检查继续使用；尚未形成稳定 schema 前不提前承诺跨进程恢复。

不从把 program.rs 分成更多文件开始。先在其实际生产入口确定：

1. 哪些声明数据可形成不可变 snapshot，哪些字段确实属于 Region/Candidate。
2. `ensure_signature` 的输入依赖及输出契约；父签名、nominal capture、staged input
   不再用“先继承全部”混合表达。
3. 同一 ArgumentPlan 是否覆盖普通函数和 NF，且能独立保存域观察与执行序列。
4. VM 对前端的请求需要哪些稳定输入，哪些结果可冻结，哪里必须暂停调用。
5. TypeRef 的生产端要取得哪些依赖域与环境路径，哪些消费者需要一起迁移。

现有 `TypeInterner`、候选 Engine、PTC trail、CodeSession、DefinitionBatch 均先评估
是否能作为这些接口的实现；不因它们旧而全部重写，不因已有而保留错误责任。
重绑定迁移保留现有 visitor 的执行 scope 区分，接通各证据检查器的保留/迁移/重检/消除
结果；不能仅统一结构遍历，就声称类型、资源、控制和来源的跨层迁移已经完成。

[状态与发布协议](elaboration-state-and-publication.md)已给出 owner 划分、状态转换、
安装边界及恢复要求；[依赖观察](dependency-observations.md)细化接口前缀、精确捕获和
查询失效。[世界存储](world-storage-and-lifetimes.md)选择分块、持久索引及回收边界，
并列出旧 arena/侧表退役条件。这些属于 M0 设计成果，不能计为 M1/M3 实现完成；
具体 Rust 接口和生产消费者迁移仍需完成。
读取/草稿/安装接口按该文的所有权约束接线：跨暂停保存 ID 与 owning snapshot，
stale 返回冻结准备物；不能以单块 pin 或静态 lifetime 擦除替代跨块依赖保活。

求解线按[Type 模型与理论证据](type-models-and-theory-evidence.md)保留已有投影搜索与原义务
复核，接通带世界证据的联合模型；不得以推断出具体值代替候选选择、资源或调用交付。
新增 theory 时分别落实原子准入、正负证据和模型覆盖；用有限域冲突、开放行、未解释
predicate 等混合问题验证组合入口，不能只用各 theory 的独立成功案例作为接线证据。

## 4. 一条主干切片的验收方法

以“用户定义工厂接收普通值，产生带依赖的类型，被 builder 和普通调用使用”为切片：

- 从真实源码和 sysroot 输入，不在测试中手工注入已检查结构绕过生产者。
- 展示 HIR 来源、参数计划、冻结值、TypeRef、调用/环境证据和 IIR dump。
- 对域错误、NF 不展开、scope 逃逸、预算耗尽、失败后重试分别给拒绝/恢复结果。
- 检查实际调用次数和执行顺序，不只检查最终数值。
- 旧专用路径在该切片中不再被调用；迁移完成后删除对应生产分支及过期测试预期。
- 同时验证 public meta/builtin 的对应接口，不能只交内部 Rust 类型。

六包的现有入口、协议身份迁移和真实 sysroot 验收边界见
[库迁移接线](libraries-and-delivery.md#9-当前生产入口与迁移接线)。保留已有 builder 的
公开用法并替换底层发布责任；不通过复制 trait 声明、重复原始绑定或去掉资源检查迁移。

不要求这条切片等于整个 Flurry 完成；但也不能把大量未迁移消费者藏在“已经有 TypeRef”后面。

### 已交付部分不重复立项

局部声明输入需求、调用供应及 required 残余已沿同一词法依赖机制接线；普通 callable、
NF、名义类型和透明别名共享声明输入准备，原 Type-only 和名义成员的独立循环已退役。
contextual 签名、普通值读取与捕获共用父声明输入；跨 VM callable 在接收世界核验。
源码 VM 与普通显式 invocation 已借用原包的逻辑声明视图，避免为剔除旧 dispatch
复制整包；封闭执行和源码查询仍保持独立准入、索引及权限，详见
[所有权边界](world-storage-and-lifetimes.md#110-源码声明视图与物理执行表分开)。
这些是 M1–M3 的部分纵向交付，不是通用 Region、ContextualExpr 或发布协议已经完成。
当前入口及证据集中在 [README](README.md#当前生产接线与下一步)，不再安排重做旧 flag
helper 调查或已完成的停用 crate 删除。

### 下一条纵向交付：待定域的普通调用

下面两个调用分别需要恢复参数域和 callable 签名。

```flurry
comptime fn Byte() -> Type { u8 }
comptime fn count() -> Integer
where T, ...values: meta.List<T>
{
    values.length()
}
comptime fn identity(value: T) -> T
where T, flag: bool
{
    value
}
```

`count<.values = [1, 255], Byte()>()` 需要先保留有待检查的输入，在 Byte 产生 Type 后
完成列表域检查；`identity<Byte(), true>(9)` 还要求该实际 Type 完成 callable 签名，
再给普通实参提供 u8 上下文。Byte 是普通 comptime 执行，不能按 NF 头处理或展开
一个 opaque 头来推导其结果。列表只是接收域的一种消费者，不因此获得特殊等待规则。

已观察到的身份按域中实际出现的声明引用做替换。显式实参在调用者环境中检查，
默认源码从提供者取得前缀。真实 sysroot 的宽度回归覆盖 720 个独立布尔子句的
直接调用，以及转发 1000 个显式实参后由默认子句读取首槽的调用。后者在
`04b91db5` 耗尽前端预算，拆开这两个词法环境后，两次传入不同前缀都得到预期值。

对普通形参，每次调用要把本次的 `root: Type` 传给后续的 Type 默认值。真实 sysroot
用例连续传入 `u8` 和 `bool`，各检查 512 对默认值与依赖参数。NIR 的 storage
检查在进入 callable 时核验并索引整个 sort 环境，域检查在同一作用域内按 Type 和
阶段查询。不同的槽位顺序即使出现在同一包里也会得到不同的缓存键。
源码 dispatch 查询为每个声明建立一次受检替换，投影签名。在仍需普通 comptime
输入时，查询借用模板，逐一检查函数体、默认值、契约以及闭包内嵌的类型。
调用选定实参后才可为这样的入口生成可执行 IIR。对可直接遍历的函数，
查询分别投影各项契约及表达式以发现传递依赖。实际生成 IIR 仍须特化完整函数。
512 对参数的两次调用在默认 dispatch 预算下返回 `true`。扩大到 768 对时，
默认 100000 步在值准入阶段耗尽，150000 步返回 `true`。
[源码 dispatch](../compiler/materialization/src/dispatch_function.rs) ·
[模板作用域回归](../compiler/materialization/tests/expansion_boundary.rs)

把 1000 个固定实参直接写在无子句入口中，前端也能完成实参检查。当前整包
dispatch 从这样的入口遍历调用体及 sysroot 的声明，超过其默认 100000 步；
单独给 `--dispatch-fuel 120000` 即可返回 `true`。这两种入口的根集合不同。
上例 `Byte()` 返回后如何接回调用中的源码槽位仍待实现。
[宽度用例](../compiler/luna/tests/contextual_input_costs.rs) · [域检查边界](elaboration-state-and-publication.md#检查帧与探测)

当前所有权边界如下。

| 持有者 | 已接通的路径 | 仍缺的交接 |
| --- | --- | --- |
| 源码检查区域 | 普通调用、声明子句、默认供应和收集器在所属帧中准备受检输入。规则帧保存当前源码节点，子检查接管唯一的父区域和结果目标。runtime 调用的必需计算也能交回驱动，执行期间父区域持有等待槽位。VM 回复由原区域接收并登记本次生成的定义。候选试探失败时，先退出内部作用域，再恢复该候选的检查事务。 | 显式 comptime 子句与随后的普通实参现在共用调用者的检查区域。普通实参保留 HIR 引用，收集后的字段由请求共享持有，驱动仍使用原 Checker。实例完成仍要求已知签名，尚不能从原 VM 收取用来检查实参的 Type。 |
| 词法绑定 | 局部类型、expression code、pattern code 各有不可变版本。块、声明实参、试探和生成器推演只持有其所需的根。更新按 LocalId 共享未修改分支，dump 仍按 owner、index 排序。 | 块退出另行处理输入事实失效，候选失败另行回退存储，生成器成功只带回已有代码名字的赋值。单独保留绑定根不足以恢复整个 Checker。 |
| 声明与 callable 实例 | 源码顺序的 producer、公开槽位与本次调用的替换分开持有。已观察的 Type 和普通值都可完成签名和默认参数。输出槽位保留实际读取，接收方在本次调用中检查子句域和形成义务。NF 默认代码在实际前缀齐备时实例化。 | 签名依赖尚待执行的 Type 或值索引时，必须保留未发布的实例及其输入消费者。默认供应共享声明替换、值绑定和默认参数类型的词法根。首次从借用的外层签名取得域时仍需按预算复制，嵌套使用已保存的域时直接共享。 |
| 源 invocation | VM 在同一操作数栈上执行已检查的普通和 contextual 输入，保留前缀、存储及控制出口。required Type 与普通值共享执行及恢复路径。 | 入口仍须先取得已检查的代码。尚无从活动 VM 把 Type 交回未检查 HIR 槽位、再接回该 invocation 的生产路径。 |

对已经能独立检查的子句实参，调用准备现在把选中声明的参数和域模板移交给结果
消费者。父项捕获占声明前缀；具名供应、默认值和依赖检查可以有各自的顺序。
交给 VM 的代码保持源码求值顺序，返回的参数按声明槽位排列。在调用者作用域中，
required invocation 的结果和原域模板建立一次同时替换，用它检查包内每个字段。
这条返回路径不再按参数身份回查全局声明表。

已检查的开放函数注解还保留从捕获位置到源码形参槽位的映射。比如
`echo(comptime unused, comptime flag, value: defer Choose(flag))` 的注解
只读取 `flag`。即使调用者在 `unused` 处传入函数，检查器也能复用定义端准备好的
计算。目标域本身引用的形参同样属于捕获。每次调用从自己的槽位取值，按实际展开的
项给替换计费。选中的实参携带活动闭包或函数声明时，要在调用者可见的声明环境中
重新准备；新生成的定义改变了身份分配游标时仍沿用重新准备路径。
[捕获映射](../compiler/elaboration/src/program/required/capture.rs) ·
[注解调用](../compiler/elaboration/src/program/open_functions/annotations.rs)

例如声明含有 `where T, field: T` 时，表达式 `bool` 自身的类型已经确定；
它作为 `field` 的身份还需经过声明域检查。若产生 `T` 的调用返回 `Type`，
`field` 可以接收这个类型值；若返回 `bool`，这里应接收布尔值。准备期间将
源码操作数保留到执行点。已观察的值可以帮助检查后面的槽位；若将它代入会
改变绑定的 sort，则必须等到 `T` 返回，以实际域决定 `field` 的 sort。这个区别
也适用于捕获和收集器中的依赖域。
[声明域检查](../compiler/elaboration/src/program/dynamic_instances/domain_checks.rs) ·
[真实 sysroot 用例](../compiler/luna/tests/type_input_execution_order.rs)

原 VM 的交付边界已经细化到单个等待实参。以两个整数都在等待同一个 Type
为例，较晚的生产者给出 `u8` 后，第一次交付可以完成 `7`，下一次交付才检查
`256` 并在该文字的位置报错。两次之间可以取消调用。输入帧拥有先前执行取得的
快照和声明绑定，取消清理本次计划的局部名字；实参已经对调用者作出的写入仍然
可见。树解释在原栈上继续推进，字节码会话在每次交付后归还控制权。多个槽位
同时就绪时沿用声明顺序，已经越过的槽位留到下一轮。[输入帧](../compiler/comptime_interpretor/src/input_collection.rs) · [取消与恢复用例](../compiler/comptime_interpretor/src/input_collection/tests.rs)

执行侧的词法作用域随 invocation 的环境移动。树解释进入块、子句实参或候选
pattern 时建一个空作用域，随后只登记本层新增和移除的绑定。失败、控制转移或
正常离开时，本层引入的名字消失，外层绑定的写入和移动继续生效。成功的
let-pattern 把新绑定交给父作用域。字节码用自己的寄存器作用域实现相同的
可见性。在 32 层块中形成同一个函数实例，调用帧从 1 个已有绑定扩大到 4096
个时，两种执行方式各自的步数均保持不变。[VM 环境](../compiler/comptime_interpretor/src/environment.rs) · [实例形成用例](../compiler/comptime_interpretor/src/dynamic_instances.rs)

普通 callable 的声明输入检查借用已观察槽位的原始域，形成替换时只接管用于后续
签名计算的值和调用者作用域。对显式给出 sort 的声明，函数主体、默认表达式和契约
共用一次检查后的实参索引。片段保留外围绑定，由持有完整签名的一方检查。

VM 从已完成的输入槽位取出声明身份、声明 sort 和借用的值，交给 NIR 计算依赖域。
同一域多次引用一个槽位时，各处分别支付展开预算。替换值来自调用者，进入被调函数
的嵌套绑定时只移动自由引用的深度，其中的声明引用保留原身份。显式交付的每个值
都要通过作用域和 sort 检查。对于实参改变 binder sort 的调用，仍由实际声明域决定
有效 sort。名义类型的准入继续要求接收世界的证据。
[结构替换](../compiler/nir/src/type_computations.rs) · [VM 输入绑定](../compiler/comptime_interpretor/src/input_bindings.rs)

对于源码端已能检查的 callable 子句，VM 在执行第一个生产者前先确认目标和元数。
源码准备语句按供应顺序运行，具名参数可以在这一步保存到局部绑定。读取每个受检
操作数时，字节码把值从临时寄存器移入当前输入作用域，随后可以复用这个寄存器。
这样，较晚操作数改写同一个绑定时，先前读出的值仍然可用。输入作用域正常结束后，
VM 按声明槽位顺序检查各自的域，把值冻结为实例实参。此时 VM 可在两个
槽位之间交还控制权。若较晚的生产者直接跳出外层循环，作用域同时释放目标和先前
保存的值。若槽位检查失败，错误指向出错的实参，调用者已执行的改写仍然可见。
字节码 dump 中的 `retain-instance-argument` 和 `freeze-instance-argument` 分别
标出这两个时刻。用常量布尔操作数编译宽度为 1、16、1024 的输入包时，三者都使用
两个寄存器，保存的实参数量仍随包宽增长。[宽度回归](../compiler/comptime_compiler/tests/control.rs) · [编译入口](../compiler/comptime_compiler/src/lib.rs) ·
[作用域与实例形成回归](../compiler/comptime_interpretor/src/dynamic_instances.rs)

从这里接普通调用，需要在源码的两个消费位置保留不同的交付责任：

- 对 `count`，声明的返回域已经是 `Integer`。待定位置是列表的元素检查。前缀
  可执行时，将其 checked code 交给原 invocation，保留列表文字及其接收位置；
  从可变绑定读取的字段则在该位置取得值。稍后得到的域用于检查与组装这些字段。
- 对 `identity`，给出 `T` 的那次执行还决定被调签名。函数实例的形成检查和
  普通实参 `9` 的检查都要接收同一次执行的结果。提前发布带自由形参的签名会
  让普通调用检查器把声明端的绑定当成调用者已有的证据。

当前输入帧能处理的等待位置已有受检 NIR。具体断点在声明输入准备的完成方式。
`CallableCompletion` 选择 `InputChecking::Typed`，即使元素域待定，
列表文字也必须立即产生一个带类型的表达式。`PreparedDeclarationInputs::into_typed`
据此生成有序的 let 和声明槽位上的读取。NF 与名义类型的输入计划已经能携带
待域的文字、聚合和块。普通 callable 若沿用此计划，还需要由它的消费者接管
实例形成和后续实参的检查。单独放宽列表文字检查会丢失这个消费者。
[callable 完成点](../compiler/elaboration/src/program/dynamic_instances.rs) ·
[声明输入计划](../compiler/comptime_compiler/src/declaration_inputs.rs)

真实 sysroot 的入口在获得 Package
后也可单独创建解释器；一个只存在于先前 Checker 中的 HIR 借用无法在这个入口
恢复。需要原 HIR 的检查应留在与声明世界匹配的源码服务中，由会话关联请求、
世界和返回位置；已独立检查、仅需兑现域的操作则可随代码交付。两者共用原
invocation 的预算和控制出口。[源码消费者](../compiler/elaboration/src/program/dynamic_instances.rs) · [独立包执行入口](../compiler/luna/tests/support/sysroot_execution.rs)

这一纵向交付必须同时做到：

1. 待定域、已观察的身份、已经执行的值分别存储并可 dump；未知不能伪装成 Any、
   Type 或正常的具体实例。显示采用现有 S-expression 规则。
2. 所需 Type 真实产生后，只恢复尚未完成的检查；保留原源码位置、输入域、词法绑定、
   phase 及预算。过期或外世界响应拒绝，取消不发布半成品。
3. 已执行前缀、可变存储和控制出口归原 invocation 所有；恢复不重放副作用。较早
   输入的组装可待域完成，不代表可以越过一个尚未获准执行的操作运行后面的生产者。
4. callable 签名、形成条件和输入检查全部完成后才交付实例。仍不支持的活动节点
   明确报缺口；只有实际生产者/执行边形成闭环时才能报循环。
5. 普通调用、NF／名义输入和真实 sysroot 的对应接口均消费该接线；覆盖域错误、
   默认/具名/展开顺序、分支/提前退出、失败重试、生产者次数及作用域逃逸。

对应的 ignore 只在这些正例与上述边界实际通过后解除。不能通过在签名查询时偷偷执行
Byte、统一提前执行所有 Type 生产者，或给 List/Tuple 增加语法特判来交付。这一步也
不自动解除 P3／值引用证据的另两项 ignore，不代替运行时 EnvironmentPlan 或资源检查。

### 设计决定与后续实施细节分开

| 已确定、实现必须遵守 | 可在对应迁移阶段细化 |
| --- | --- |
| 声明/存储/执行身份分开，稳定 owner 与局部事务分开 | 私有 Rust 类型拆分、索引扇出、分块尺寸 |
| 普通参数和值、NF 不展开、无 Tuple/List 语义特权 | 具体值编码及按工作量选择的快路径 |
| 检查依赖与执行次数分开，暂停不重放 | instruction 布局、队列实现及局部调度优化 |
| TypeRef、环境取得、资源责任和 proof 分别交付 | 各节点字段、证据压缩和局部重放优化 |
| 布局/ABI/产物契约不改变类型及控制语义 | 各平台 ABI 分类、动态布局编码与持久 schema |

后列不是免责项：开始依赖某项能力的迁移时，必须先补齐该项具体设计并验证。
但它们也不是所有前端工作共同等待的全局门禁。默认 allocator 的不可恢复失败策略
及高级 verification 库不阻止上述声明输入迁移；涉及它们的库交付仍须解决真实语义
问题，不能临时替作者选择。格式化签名已由作者明确为
`fn println(comptime format: str, ...varargs: defer std.fmt.Parse(format))`。带 Display 的字段、转义与换行已经通过真实 sysroot 的宿主演示，
对应的类型数据与准入边界见[描述计算与参数准入](type-descriptions-and-admission.md)。

## 5. 测试如何支持架构，而不是牵着语义走

| 层 | 验证目标 | 反证需求 |
| --- | --- | --- |
| 语义内核 | identity、substitution、域、冻结 | 非单射 NF、capture、世代错误、错误 world |
| 事务/调度 | 提交、回退、唤醒、取消 | 失败后合法重试、旧 ticket、先完成宽候选 |
| solver | 传播理由、模型解释、投影 | 独立重放、有限域、trait world、Unknown 不转 false |
| VM | 指令与语义服务接线 | 双载体相同语义、不可重放副作用、外部请求恢复 |
| 源码/库 | 完整生产者到消费者 | 真实 prelude、不同包/target、错误域及资源 |
| native | 布局、ABI、控制与清理 | O0/O3 轨迹一致、错误与效应分流、环境有效期 |
| query/artifact | 正确复用和失效 | 新增 impl、失败修复、取消、产物删除、配置变化 |

现有测试保留为输入资产，按作者契约核对预期。未实现正例明确标记缺口，不靠改变期望变绿。
日常只运行受影响组，里程碑再扩大；这一设计轮不启动 Cargo 全量测试。

## 6. 性能判据

首先记录当前相同 binary/profile 的 cold/warm 行为，再比较新实现；结果缓存独立开关。
评估项包括：真实 sysroot 最小编译、不同包大小的域查询、深依赖/默认值、批量定义发布、
大列表组装、正则代码生成、PTC 分支/回跳、运行时来源及 cleanup 摘要、native 编译。

复杂度必须写清自变量。一次字段查询不应扫描所有无关定义；一次小批发布不应无条件
复制整个世界；持久列表复制不应总按展开载荷遍历。递归依赖闭包、模型搜索、必要检查
仍有真实成本，不承诺一切 O(1)。
分配与释放、hash、证据重放和结果接纳计入成本；不能把成本移出计时区后称算法更快。
超预算仍保持语义诚实，优化不能通过少验证来获取数字。

## 7. 源码清理的具体规则

迁移前列出旧入口的生产调用者、语义、错误和预算行为；新入口接管后检查这些调用者
确实改变，再删除旧入口。临时 adapter 只允许一个方向，不保留两套长期权威数据。
无调用引用不是充分删除依据：工具、序列化、测试 fixture 和公开 CLI 也要检查。

停用 compiler/typeck、mir、mir_build、codegen 的仓库内消费者和资产已核对，11 个文件
已作为独立退役单元删除。middle 的旧类型侧表及包装也已作为另一单元删除，
活动 HIR 查询及其生命周期约束保留。这两项无需等待 M7 全部条件成熟，也不以删文件数替代主干迁移。
生产 LLVM 在 luna/host，不因目录叫 codegen 就迁入它，也不先升级 LLVM。
unsafe 清理以生命周期证明为单位，不能用新增 unchecked Send/Sync 换取编译通过。

现有活动依赖与逐项退役条件见[模块边界](module-boundaries-and-retirement.md)。
middle 的 HIR 查询必须保留；intrinsic 的旧 TyCtxt 参数已经随该退役单元删除，
不能再据清理前注释推断存在待迁移的类型注册行为。单包与多包入口均须纳入适配器迁移验收。

## 8. 提交与状态

设计文件保持现行状态，修订正文，不追加每轮日志。完成一个可验收的功能/迁移提交一次，
附上直接相关库与测试；提交格式和权限沿用 AGENTS。
任何阶段未闭合时，报告实际缺的生产者、消费者或证据，不能以局部测试数宣称整个主题完成。
M0 设计基线已交付，停用源码退役已完成；M1–M3 已有上述生产接线，完整阶段及
M4–M7 的交付验收仍未完成。

M0 现在已形成可划定实施边界的设计基线，入口的[完成范围](README.md#设计基线与实际完成范围)
按原任务区分设计、现状证据和未实施部分。后续文档修改应解决新发现的契约冲突、
实际接线缺口或实施所需的具体表示，不继续以重复审计与增加状态段落代替源码工作。
下一批依其真实输入生产者、接收域和恢复边界落实 Rust 接口与检查；后续平台细节
不构成共同门禁，也不把所有后续实现扩大成 M0 必须预先完成的设计任务。
