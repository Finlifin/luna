# 从 comptime 输入到 runtime 值的交付

本专题补齐 M2/M4/M5 之间的阶段交接，源于真实 std writer 暴露的问题。
这里描述目标职责与迁移条件，不表示这些载体已接入编译器。正式语义以
[跨阶段值传递](../docs/comptime/quoting-and-splicing.md)、
[函数参数阶段](../docs/functions/comptime_fn.md)和作者关于 comptime 值引用的决定为准。
初始复核基于 `689d45c5`，后续准备成本修复与当前证据见文末；不改正式 docs，
也不把当前残余重建方式补定为语言规则。工作区原有资源分析排版改动仍在，
不能称为隔离的纯 HEAD 验收。

## 1. 已确认的现状分裂

同一个源值从不同入口到达 runtime，目前经过不同检查：

| 入口 | 当前消费者 | 实际行为 |
| --- | --- | --- |
| 命名常量 | expression_values/bindings → splice_constant → splice_value | 选择普通 runtime Splice 方法，以 comptime 输入实例化 |
| 源字段接收者 | field_operand → projection_operand → deliver_operand | 已有源常量、已供应局部数据和可识别的 comptime 调用先保留源程序；嵌套字段投影后再按外层使用选择 Splice |
| comptime 函数结果 | call/finish_prepared_call 等 → deliver_comptime_result | runtime 数据使用走转换；作为下一个 comptime callee/Type 构造头时保留源值 |
| 普通函数的 comptime 参数 | expression_values/bindings 的 ResolvedFnParam | 通常直接生成 K::Local，先按源域核对 runtime 预期域，没有同等的交付步骤 |
| 传给后续 staged 输入 | iir::staging::substitute_forwarded_inputs | 在被标记的 staged 位置替换已完成值，检查写入/地址逃逸与捕获作用域 |
| 剩余参数读取 | iir::staging::wrap | 以 runtime Let 恢复值，随后按 runtime 移动、捕获、存储规则分析 |
| quote 主体直接读取生成器数据 | bindings 中的 generator_data / materialize_generator_data 提前返回 | 未经过字段观察的直接数据读取仍可能跳过同域 Splice；字段入口已迁移，见下文 CMTT 复核 |
| 生成器定义中的源实参与 callee | check_code_generator → quote_expression → 普通 required 输入准备 | 抽象 Data(None) 被要求立即交付值，未调用的生成器也可能报输入不可用 |

源码锚点：
[参数读取](../compiler/elaboration/src/program/expression_values/bindings.rs)、
[转换选择](../compiler/elaboration/src/program/splicing.rs)、
[阶段输入重建](../compiler/iir/src/staging.rs)。这些路径不能仅因都能输出 Expr
就视为同一阶段语义。

[类型改变复现](repros/staged-parameter-splice.fl)使用普通用户类型 Seed，其 Splice
的 RuntimeTarget 为 u8。直接返回 comptime 参数时，当前前端报 Seed/u8 类型不匹配。
[对照](repros/staged-parameter-splice-control.fl)中的命名常量和显式 Splice 调用均能
完成相同转换。显式调用是定位对照，不是要求用户改写所有参数读取的最终接口。

[文本观察复现](repros/staged-text-observations.fl)的域相同，因此逃过上述类型差异；
直到 residual 把参数恢复为 runtime str 局部，第二次读取才报 unavailable value。
这两个错误共同暴露阶段交接缺口，但拒绝来自不同检查层，不能假定一个补丁就能
同时解决，也不能只在文本优化或 Copy 查询里处理。
文本路径还缺 str 的 Splice/目标存储来源接线，不能仅消掉移动错误就宣称完成。

## 2. 先明确输入、使用与目标

参数 sort 决定输入在哪里形成，不自动决定每次读取如何进入另一阶段。
要分别保存：

- 源绑定身份、所属 activation/scope、实际域和输入阶段。
- 当前使用的角色：输入供应、类型/定义观察、代码拼接、普通 runtime 值或合法位置。
- 接收端的预期域、调用接口与需要执行的构造；不能先把源域塞进 runtime 位置再补查。

这些事实属于共同输入与检查上下文，不能以 str、Tuple、meta.List 等名字重建。
NF 输入仍是普通 comptime 值，normal-form 头仍不展开。已知同一结构视图不代表源域
与 RuntimeTarget 相同，也不授予 Copy、Drop 或内存访问能力。

下节以 Operand 与 demand 明确这些职责。同阶段引用不复制 producer；runtime
构造保留选中的 Splice 声明/实例、源输入、RuntimeTarget 与普通调用契约；代码
展开沿既有 CMTT 规则。尚未完成的域/实现选择继续属于活动检查状态，不能冻结为
“直接恢复一个 Local 即可”的成功计划。

### 源操作数不能提前降成已经交付的 Expr

`7f6d3715` 的对照进一步确认：问题在源操作数形成与消费之间，不能只归入
generator 数据物化。`CONFIG.byte` 中的 CONFIG 先经 `splice_constant` 转为 u8，
随后 `projection_expression` 在 u8 上找 byte 而失败。相反，生成器局部字段保持
Seed 源域，却在最终交付位置没有转为 u8。两条路径分别转换过早与转换缺失。

新的检查接口必须让父操作先取得带阶段的操作数，再选择怎样消费它。下列为完整
目标接口，不等同于当前 Rust API，也不改变正式语法；已迁移的字段入口见下节。

```text
check_operand(site, input_context, region) -> Progress<Operand>
consume(operand, demand, region)           -> Progress<Delivered>

Operand = (domain, phase, category, formation, owner, source_range)
phase   = comptime | runtime
category = data | callable | contextual-code | type
formation = source-program | available-source-value | residual-program
```

`phase` 与 `formation` 必须分开：一个参数确定属于 comptime，不代表其值已经供应；
一个 runtime 函数的身份可以是已形成的 comptime 数据，不代表可以在 comptime
执行其主体。`category` 从受检类型与代码接口取得，不能从源码名称或 ExprKind
形状猜测。普通 n::Expr 可以继续承载受检程序，但不能独自承担上述事实。

| demand | 操作数与结果的处理 |
| --- | --- |
| source-input | 按目标参数的源域检查并保留生产者顺序；输入未形成则保存检查后继，不能要求 RuntimeTarget |
| field-observation | 对已成立的 comptime 源对象在源域中投影；所得字段成为新的源操作数，之后由外层 demand 决定是否转换 |
| callee | 取得 callable 或 Type 构造头及接口；调用输入准备与调用结果另行交付，不把 callee 本身自动变成 runtime 数据 |
| runtime-value | 同阶段值直接交付；源值选择 Splice 和 RuntimeTarget，准备源输入，保留普通 runtime 调用 |
| code-expansion | 由 CMTT 核对上下文、孔和接入位置，不调用普通数据 Splice |
| place | 沿源阶段与资源协议检查可写位置；不能把临时 runtime 转换结果当成原 comptime 值引用的位置 |

消费描述必须沿表达式的实际结果边传递。外层需要 runtime 值，不代表其字段接收者、
callee、显式 comptime 参数也各自需要 runtime 值。callee 角色先确定调用阶段；
从源数据取得 runtime 函数值仍需其合法目标表示和转换，不能把 comptime callable
的免转换路径扩展到全部函数值而吞掉用户 Splice 的行为。反过来，观察接收者不能把其中
真正的 runtime 调用标成 comptime。普通 runtime 条件保持 runtime 控制流，分支中
需要的转换留在相应分支；仅凭分支结果的源身份已知，不能提前执行转换或条件。
跨阶段可用性由作用域和已执行输入证明，不以“表达式没有自由变量”代替。

### 已接通：源字段操作数与结果交付分离

Rust 的 `splicing::Operand` 当前区分 Source 与 Value：前者保留受检 comptime
程序，后者是已经按当前消费阶段检查的表达式。Source 不意味着已经取得执行结果，
Value 也不意味着一律属于 runtime。两者均持有原域和程序；Display 显示
`(source-operand ...)` 或 `(value-operand ...)`，不将观察记录放入按源码位置索引的
全局侧表，也不在 Checker 上增设临时禁止转换的环境开关。

`field_operand` 通过已有声明与输入信息识别源常量、已供应的局部 generator 数据
及 comptime 调用；调用域观察复用 `declared_comptime_call_result`，不执行 producer。
源调用仍由普通调用检查接收完整实参并保留程序。`projection_operand` 在源域中
检查字段，嵌套投影继续传送 Operand；`deliver_operand` 最后按使用位置调用既有
`deliver_comptime_result`，由普通 Splice 选择、输入准备与契约检查构造目标结果。
匿名记录和名义字段共用此交接；runtime 字段仍保持原阶段。

这消除了普通 `CONFIG.byte` 的过早转换，并允许不具备 Splice 的外层容器仅被源
字段观察。若字段本身为 Seed 且其 RuntimeTarget 为 u8，最终交付检查的是 u8；
若继续观察 Seed.byte，则先投影 byte，不执行 Seed 的转换。直接使用整个容器仍
须满足自己的 Splice，不能因字段可读取而取得任意 runtime 表示。

完整目标尚未接通：块／条件结果、类型成员常量的旧交付入口、尚未供应的 generator
参数与一般普通 comptime 参数，尚未统一返回这类操作数。源表达式的形成与区域
所有权仍依赖既有调用/检查后继；这不是新的全局阶段推断器。特别是不能用 Source
标记掩盖输入未完成，也不能根据最终期望类型去猜测该读源域还是目标域。

### callable 字段的观察与调用

嵌套 callee `data.callback.apply(false)` 暴露了字段入口之外的旧路径：
`resolve_receiver` 先通过普通 expr 交付 `data.callback`，然后才发现 `apply`
是数据字段。因此即使 callback 只供源字段观察，也会错误地要求它具有 Splice。
方法探查现在先取得相同的 Operand；未指定显式接口、无上下文接口且源结构确有该
字段时，退回字段调用路径。真正的方法接收者仍按交付及接口选择规则检查；不存在
字段、转换失败与接口选择失败不能互相吞掉。名义和匿名记录复用字段判断。

`invocation_callee` 对已排除关联值／静态方法的投影保留 callee 消费标记。取得
comptime callable 后，在源阶段调用，只交付结果；runtime callable 仍选择其
Splice。该标记只覆盖本次表达式检查，成功与失败均恢复外层状态。

旧 `transported_runtime_methods_preserve_clause_domains_and_nested_storage` 的
408..419 恰为 `value.apply`，并非 Factory.make。真实 sysroot 缺少该函数类型的
Splice；这不是关联签名传送失败。该原正例保留原有断言，标为缺库实现而 ignored，
不把缺失能力改成语言禁止，也不添加任意函数免转换规则。
`runtime_method_field_transport_obeys_explicit_conversion` 使用本地 Transfer 协议
隔离传送，保留方法／普通函数、true／false 子句与关联结果域，并检查删除实现时
拒绝、转换返回另一个函数时生成程序确实调用另一个函数。它不是六包公开接口验收。
Splice 选择失败现在附注实际源域，保留原选择错误、位置和重试分类，避免将隐式
转换错误误认作同位置的用户方法查找错误。

真实 sysroot 的 `projected_comptime_callees_are_invoked_before_result_delivery`
检查已完成顶层常量中的嵌套 comptime callable，包括 quote 内调用，并拒绝向该
comptime callable 传入 runtime 参数。生成器形参或局部数据未供应时的相同表达式
仍受定义检查提前索值阻塞，复现见
[待供应生成器 callee](repros/projected-callee-generator-input.fl)；本阶段不跳过该检查，
也不声称已完成源码／VM 后继。

验证工作目录 `/home/fin/luna`，使用 `development/check.py --timeout 900 -- test`：

- 改动前重跑旧 CMTT 传送及嵌套指针观察用例，
  `/tmp/luna-tools-owx5nn8n/result.json` 退出 101，两项均复现；指针观察用例未在
  本阶段修改；其旧拒绝断言随后已按
  [结构字段与 pointee 分离](solvers-resources-and-lowering.md#指针观察旧拒绝预期的纠正)分别核对。
- 新投影 callee 对照在改动方法探查前失败于 `data.callback` 的 Nominal 转换，
  `/tmp/luna-tools-smbaezwr/result.json` 退出 101；显式函数转换对照通过。
- `-p luna --test cmtt_methods --test library_std --no-fail-fast -- --test-threads=1`：
  `/tmp/luna-tools-mzo4a0lb/result.json` 退出 101。方法组 162 项通过、一项原库
  缺口 ignored；新增测试中的生成器局部 const 暴露上述提前索值缺口。标准库组
  14 项全部通过，包括 LLVM O0/O3 字段观察与转换次数、文件资源和实际输出。
- 将正例明确为已完成的顶层源常量、保留生成器缺口复现，并增加 runtime 参数
  拒绝后，以 `-p luna --test cmtt_methods -- --test-threads=1
  projected_comptime_callees_are_invoked_before_result_delivery
  runtime_method_field_transport_obeys_explicit_conversion` 复验，
  `/tmp/luna-tools-uemewopj/result.json` 退出 0，两项通过。生产实现与上项相同；
  不把分次检查记为完整方法组一次全绿，也不把本地 Transfer 当作 builtin 接线。

### 生成器定义检查与实际调用的交接

`check_code_generator` 以 `Argument::Data(None)` 检查尚无实际输入的定义，再通过
`run_code_generator` 检查 quote。此时知道参数源域，却没有调用值。现有
`quote_expression` 随即进入普通 expr 检查；`read(comptime data)` 或 `callback()`
会在这一层要求实际 required 输入，因此连未被调用的合法生成器也可能被拒绝。
这与调用后 `generator_data` 已有快照却跳过 Splice，是两个不同完成点。

定义检查可以建立字段域、Splice 义务与结果域、调用接口和代码上下文约束；无法
立即完成的输入及依赖域必须交给拥有源区域的检查后继。实际调用供应数据之后，
才形成相应快照、完成输入相关检查并发布代码。不能通过跳过生成器定义检查解决：
错误分支、非法 runtime 存储、越界上下文和不成立的契约仍须得到诊断。

引用可变生成器数据的 quote 在构造时观察源值；生成的 runtime Splice 保存该次
观察的输入，不能稍后重新读取已经变更的 generator cell。尚未形成的输入保留其
producer 和所属 activation，不能伪造冻结值。取消、预算失败和候选回滚遵守同一
Region 所有权；已形成快照与已执行前缀不能因检查恢复而重放。这里与
[源码检查和执行交接](elaboration-state-and-publication.md)共用恢复协议，不另造
一个只认识 quote 数据形状的求值器。

### 必须共同迁移的入口与退役条件

1. binding、常量、调用结果、字段观察和 captured staged input 都先产生 Operand；
   合并其源域检查，接收端预期域只在正确的消费边界参与。不能仅给 generator_data
   的两个提前返回加转换。
2. 将 `comptime_callee_result` 的特殊结果位置传播纳入消费描述。块尾、分支结果、
   接收者和参数供应各自传递自己的 demand；不再依赖一个环境布尔量代表所有子项。
3. Splice 选择完成后保留明确的源输入和目标调用。源参数擦除与 runtime 构造次数
   分开；现有 `iir::staging::wrap` 只能在所有残余读取有明确交付计划后退役。
4. quote 定义检查、调用、快照和接入均消费同一协议。先证明当前专题的类型改变、
   源字段、staged 实参、callee、快照及分支用例，再移除旧物化直返路径。

此顺序不要求先完成所有程序验证，但每个已交付结果仍要接受现有阶段、类型、
资源和契约检查。程序状态命题不进入 PTC；明确 Unknown／待决与非法输入的区别。

## 3. 转换发生在值消费边界

不能在遇到任意 ResolvedFnParam 时机械插入 Splice：

```flurry
struct Seed { byte: u8 }

fn field(comptime data: Seed) -> u8 { data.byte }
fn explicit(comptime data: Seed) -> u8 { Seed.as(Splice).splice(data) }
```

第一处需要先在源值上解释字段观察，再交付观察结果；不能先把 Seed 转成 u8，
再尝试访问 u8.byte。第二处 data 是 Splice 方法的 comptime 实参，不应再次要求
runtime 转换。把入口合流做成“所有名字读取一律 splice”会立即破坏两者。

检查器应在消费角色已经明确的地方要求交付，保留同阶段投影、参数准备和类型观察。
源值已知也不授权在 comptime 调用一个 runtime 函数：普通 runtime 操作继续留在
生成程序中，除非后续优化有保持行为的依据。不能把这一设计理解为任意封闭表达式
都可以提前执行。

参数、命名常量、已完成调用结果和合法捕获应使用同一交付服务；入口差异只负责
取得各自的源值/来源，不各自重新决定运行时构造规则。预期类型引导交付检查，不能
为满足预期而改写源值域或忽略选定 Splice 的关联 RuntimeTarget。

规格表达式仍遵守原有阶段与纯度规则，safe predicate 不能因此被改成 comptime
执行。不纯的构造不能被植入规格后免检；既有 staged aggregate 契约也不能因参数
变成普通 runtime Local 而丢失其源值事实。程序规格与类型命题仍由不同层处理。

## 4. 保存形成次数与执行位置

comptime 输入形成、Splice 实例准备和 runtime 构造是三件事。
已完成的输入可以供多个使用点引用，代码实例可以共享，但 runtime 构造仍按各
使用点及控制流执行。不能缓存构造结果来代替有副作用的 Splice 调用。

| 源码情景 | 必须保留的行为 |
| --- | --- |
| 同一源常量在两个执行到的值位置使用 | 各自的 runtime 构造执行；不因输入相等合并可观察副作用 |
| runtime if 的两支都使用同一常量 | 只执行所选支的构造 |
| 显式 runtime let 保存转换结果 | 后续使用该 runtime 值，按其资源规则复制/移动，不重新运行源 producer |
| runtime 闭包主体读取 comptime 输入 | 输入作为代码准备依赖保留；主体的 runtime 构造属于闭包调用 |
| 在闭包形成前先保存 runtime 转换结果再捕获 | 构造在形成前发生，捕获取得实际 runtime 值及其资源责任 |
| 默认输入或参数展开 | 沿既定供应顺序和提供者规则执行；交付不能重放已完成的默认值/实参 producer |

副作用、异常/错误、拒绝、清理和生命周期均属于构造行为。只保留结果类型，或把
Splice 当 comptime fn 执行，都不满足该交付契约。已提交 native 对照覆盖命名常量、
显式调用与所选 runtime 分支，尚未分别覆盖隐式参数读取、转换后局部复用、闭包、
默认值和任意聚合。不能把三个输出次数推及这些未执行的场景。

## 5. 构造的底层终点必须明确

[builtin 的标量 Splice](../library/builtin/protocols/staging.fl)当前采用返回 data
的身份实现。它依赖现有参数重建通路。如果直接把所有参数读取改为再次调用 Splice，
这些方法会形成自递归；对泛型返回、字段选择和同域用户转换同样不能草率处理。

迁移应明确由类型/阶段检查授权的字面量或代码构造底层能力，再由普通 Splice 实现
使用它。底层构造负责目标常量/代码表示，不负责替普通源值选择 Splice。固定字面量
和普通 CMTT 组合已经能表达部分构造，因此不预先要求新增 builtin intrinsic。
若性能需要额外构造操作，应沿六包集中绑定与受检 IR 交付完整契约，不新增源码
关键字，也不将重建任意 VM payload 的内部函数直接暴露为公共逃生口。
不能通过 `函数名 == splice`、内置包名称或“输入输出类型相同”跳过正常转换。
即使源域与目标域相同，用户实现也可能有可观察行为。

这也是普通 runtime literal 与 comptime 参数读取不能混为一类的原因：前者已有
字面量构造依据，后者先要解释跨阶段交付。允许直接常量表示并不等于允许删除
本应执行的自定义转换。优化必须以已经选定且检查过的构造为对象。

源为 str 时还必须提供目标数据的有效来源。静态文本可以由合法常量存储支持；
动态字符串、切片、拥有型对象和带引用的图各有不同依赖。全局追加 Copy、将源 VM
地址带入目标程序或将所有 frozen payload 直接当 native 数据均不成立。

### 已接通的标量表示构造边界

`nir::scalar_construction::expression` 现在集中处理已完成的普通标量值及其明确源域，
供 IIR 的 frozen staged input 解码和 contextual-code 方法的程序契约替换使用。
原先两处各自选择字面量表示的逻辑退役；契约中的机器整数按实际域构造 signed／
unsigned literal，精确 Integer 则保留无界拼写，不再一律套 Integer literal。
程序验证的冻结 bool／char 观察也经该服务验证实际域，再取得与字面量一致的事实。

接口接收 value、domain、显式 target、source range 和当前 fuel。它保留域身份，
检查整数范围与位宽、IEEE 位串、BitVec 宽度和文本载荷，预算覆盖验证、复制及精确
数值拼写；错误区分额度耗尽、表示规模限制与域错误。usize/isize 没有目标时拒绝，
不会取宿主宽度。未完成输入、Type、聚合及 opaque head 不在标量构造的接收范围，
不能靠其内容像标量或 NF 主体返回标量而取得表示。

这仍是编译器内部的**源表达式表示服务**。Integer/Float 表达式不会因此拥有 runtime
存储，str 不会因此取得 backing region，函数也不会因此选择或执行 Splice。
程序规格仍交给程序验证层。原先声明接收 char 的契约替换现在也接通字符比较事实，
程序验证器按 Unicode scalar 顺序观察字面量、冻结值、局部和匹配，修改局部后沿用
现有版本失效规则；范围使用包含 surrogate 空隙的保守区间，不据此授权字符构造
或算术。新的公开代码构造接口
若消费该服务，还必须完成 runtime 结果域、空上下文、代码类别和接入义务检查。

这一步没有开放一个可绕过 Splice 的源语言函数，也没有补上普通 comptime 参数
自动交付。后续须先把合法的代码构造终点接入 primitive Splice，再共同迁移参数、
捕获和 generator 数据使用；不能把内部表达式构造函数直接当作普通读取的替代品。

验证工作目录 `/home/fin/luna`，统一使用 `development/check.py --timeout 900 --`：

- `test -p nir -p iir -p program_verification --lib --no-fail-fast -- --test-threads=1`
  退出 0，277 项通过，`/tmp/luna-tools-mk82duag/result.json`。覆盖整数范围、显式
  target、精确数值和 IEEE 位串、复制预算与重试、字符事实及写入后的失效。
- `test -p luna --test scalar_construction --test cmtt_methods code_method_program_refinement -- --test-threads=1`
  退出 0，10 项通过，`/tmp/luna-tools-fqwef94o/result.json`。真实 sysroot 验证
  u8/i8/u128/i128/usize/char 的代码方法父约束、字符模式及强化前提的拒绝。
- 修改的 9 个 Rust 文件格式检查退出 0，`/tmp/luna-tools-e6yu99k9/result.json`。
  这是上述构造与程序观察边界的验收，不是普通 comptime 参数自动 Splice 的验收。

### CMTT 数据读取与代码构造必须分开

在 `dec912f4` 工作区补查：
[quoted-data-splice](repros/quoted-data-splice.fl)定义同域 `Seed -> Seed` 转换，
每次 Splice 输出 `convert;`。一个源常量直接交付、另一个值经生成器 `'(data)`
交付，程序应输出两次；实际只输出一次，然后返回 true。这是成功编译后的行为
偏差，不是缺实现的正常拒绝，不能把现有输出登记成语言预期。

源码上有两处相连的缺口：

- `bindings_value_expression` 的 generator_data 和活跃 receiver 观察分支在普通
  交付前返回重建 Expr；payload 已检查/快照化不等于已经执行了跨阶段转换。
- 阶段判断原来只看外层 signature_owner 的 comptime 修饰，忽略
  `quoted_runtime_boundary` 已建立的 runtime 边界。现已修正此处：类型观察、
  required／staged 输入保持 comptime；否则先采用当前闭包的阶段，再采用 quote
  的 runtime 阶段，最后才回退到外围声明。上述 generator_data 提前返回仍待迁移。

命名常量走现有 Splice 选择，因此阶段判断修复已经让 quote 内的命名常量保留
runtime 转换。新原生回归使用 `Seed -> u8`，生成器本身为 pure comptime，转换
方法输出 `convert;`：同一代码接入两次会执行两次；runtime 分支仅执行所选位置；
quote 中形成的 runtime 闭包按每次调用执行转换。`read(comptime data: Seed)` 的
显式 staged 实参仍按源类型接收 CONFIG，不插入 runtime 转换。O0/O3 均需输出
六次 `convert;` 并返回 true，而不能靠保持源域相同掩盖错误。

这项修复不改变生成器局部数据的物化分支，也未使普通 comptime 参数读取统一进入
Splice。后续表达式交付必须保留消费角色：例如先观察源 `data.byte`、把整个 data
交给 runtime，以及将 data 作为另一个 comptime 实参，是不同的接线。不能为绕过
前者缺少角色表示而在所有局部标识符处递归调用 Splice。

阶段边界修复的证据（工作目录 `/home/fin/luna`）：

- 从 quoted-data-splice 复现中将生成器改为
  `pure comptime fn quoted() -> <|- Seed> { '(CONFIG) }`，调用改为 `quoted()`，
  保存于 `/tmp/luna-quoted-constant-splice.fl`。修改前 build 退出 0，报告
  `/tmp/luna-tools-srhwu7h4/result.json`；运行只输出 `convert;(bool true)`，
  `/tmp/luna-tools-w3c6gspu/result.json`。修复后同一输入 build 退出 0，
  `/tmp/luna-tools-411zgg63/result.json`；原生产物输出
  `convert;convert;(bool true)`，`/tmp/luna-tools-id1k9sya/result.json`。
  构建使用 `development/check.py --timeout 900 -- run -p luna -- build SOURCE
  --entry run --sysroot library --fuel 10000000 --output OUTPUT`；运行使用
  `development/command.py --timeout 30 -- OUTPUT`，两个脚本均以 `python3 -B` 调用。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt_phase_boundaries --test comptime_call_staging --test trait_phases --test library_std --no-fail-fast -- --test-threads=1`：
  报告 `/tmp/luna-tools-_4idz985/result.json`，退出 101。其中 CMTT 阶段 3、trait
  阶段 73、std 12 项均通过；comptime_call_staging 为 1 通过、6 失败。新 std
  用例同时检查不同 RuntimeTarget 和转换副作用，并在 O0/O3 下运行。
- 仅将本次修改的 splicing.rs 临时恢复至 `eb904d48`，执行
  `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_call_staging -- --test-threads=1`，
  仍为同样的 1 通过、6 失败，退出 101；`/tmp/luna-tools-321mws0k/result.json`。
  六项均在 `for_comptime` 构造阶段报 UnmaterializedDispatch，尚未执行被测调用。
  当时这些测试把真实 sysroot 源码交给要求闭合分派表的旧入口。后续已迁移到
  [声明快照与独立参考路由](world-storage-and-lifetimes.md#源码世界准入与参考执行互相独立)，
  保留原有行为断言；这不改变该次失败作为阶段修复基线的证据。
- 局部常量与闭包的定向检查：`/tmp/luna-tools-9f3aenq4/result.json`，退出 101；
  cmtt 23 项与 elaboration 5 项通过，另一项拒绝例因期望 `type equality`、实际
  诊断 `type mismatch` 而失败。临时恢复 `eb904d48` 后单独运行
  `python3 -B development/check.py --timeout 900 -- test -p luna --test elaboration -- --test-threads=1 symbolic_type_references_in_runtime_closures_do_not_enable_runtime_type_storage`，
  同样退出 101，诊断一致，见 `/tmp/luna-tools-11gdytf2/result.json`。
  原程序仍被正确拒绝，本轮没有改动此测试预期；对照结束后恢复阶段判断修复。

目标行为是先在 quote 构造位置观察当时的源数据，保存其输入身份/快照及转换计划；
Splice 主体放入生成的 runtime 程序。之后再修改生成器局部值，不能改变先前引用；
同一份代码接入两个实际执行的位置，转换也分别执行。纯生成器生成带输出行为的
代码不矛盾，构造代码与执行代码的纯度检查须保持分开。

作为对照，[quoted-literal-control](repros/quoted-literal-control.fl)只用 comptime
布尔值选择 `'(true)` 或 `'(false)`。quote 主体没有读取普通生成器数据，因而没有
被省略的数据转换。这条真实 sysroot 路径可编译运行，返回 true。它是合法字面量
构造的参照，不能据此把 `'(data)` 宣布为免 Splice 的通用操作。

在此基础上，[scalar-code-construction](repros/scalar-code-construction.fl)用八次
迭代从最高位到最低位选择 0/1 字面量，通过单洞 quote 与固定宽度 BitVec 拼接产生
一个字节。源码层每个 quote 只接入前一段代码，没有把 data、mask 或 remaining
嵌入 runtime 主体，也不依赖 u8 的 Splice 身份实现。删除无查询调用的提前建目录后，
255 案例已通过真实 sysroot 编译，原生产物返回 255；回归另检查全零与交错位。
这证明这条字面量组装路径可用，不证明普通 comptime 参数交付或完整数值构造库
已经完成，也不代表更大规模生成已解决重复 VM 准备的问题。

此外，[staged-code-input](repros/staged-code-input.fl)只把上述布尔生成器放入普通
函数，让它接收该函数的 comptime 参数；当前仍报输入不可用。这与 quote 中遗漏
runtime Splice 是两种交接失败：前者要延后到源输入可取得的准备点，后者要在目标
主体中保留构造。不能用直接字面量调用的成功代替参数转发验收。

因此下一项生产迁移须同时明确：哪些读取属于普通数据交付，哪些是代码值接入，
哪些只是生成器自己的源阶段计算。源语法的单洞检查仍按原 CMTT 规则执行；数据
Splice 的 runtime 调用不能被假装成拥有任意上下文权限的代码值。

## 6. 替换 wrap 的条件

当前 wrap 同时承担“给残余补一个局部位置”和“把完成的 staged 输入恢复出来”。
这使源阶段信息消失，后续资源错误才暴露矛盾。退役时要分开：

1. 仅供 comptime/类型/代码准备使用的输入不获得 runtime 存储；普通值与代码
   的实际准入仍保留，不能因没有 runtime 存储而跳过域检查。
2. runtime 值使用携带明确构造计划，完成后的调用/构造结果才遵循 runtime
   存储、Copy/Once、借用与 Drop 规则。源 comptime 槽不以同名 Let 伪装成该结果。
3. 合法的 runtime place/临时位置由对应表达式规则创建，不能通过抹除源阶段
   制造一个可在 runtime 回写的 comptime 参数。显式 runtime 副本拥有独立位置。
4. 闭包、默认值、入口快照与契约按各自作用域改写；不能全局替换相同 LocalId，
   也不能把完整 producer 复制到每个使用点。
5. 构造选择和输入准备失败时不发布部分实例。NIR safe 检查、选定 residual 的
   资源检查、IIR 与后端应消费同一交付决定，不能各自猜测常量或指针的来源。

同一个 wrap 调用是否可删除，取决于对应位置已经由上述能力接管，而不是当前
fixture 是否能展开为 Literal。先移除它再依赖各后端补猜，只会制造另一套真相。

## 7. 实际消费者与交付状态

新服务应替换下列职责，不是在 `wrap` 前后附加一张只有生产者的表：

| 当前位置 | 应交付的信息/工作 | 不得继续承担的职责 |
| --- | --- | --- |
| elaboration 的参数、常量、调用结果读取 | 源绑定/结果身份、阶段、域、形成位置与消费角色 | 以源域直接核对 runtime 目标，再把失败归为普通类型不匹配 |
| `program/splicing.rs` | 在明确的 runtime 消费点选择普通协议，检查关联目标和调用契约 | 猜测一个 Local 属于哪个阶段，或以 callee 名字跳过转换 |
| `materialization` 的 staged input 准备 | 按现有输入顺序取得值；把稳定结果和选定实例一起交付 | 重放已经完成的 producer，或用资源检查失败触发另一套准备路径 |
| `iir::staging::bind_inputs_with_layouts` | 在正确 scope 中代入源输入，保留运行时构造位置和契约 | 仅因参数 ABI 已擦除，就为每个存活读取制造 runtime Let |
| NIR safe 与实例资源检查 | 分别验证源阶段使用及构造后的目标资源/来源 | 把 comptime 值引用当 runtime Copy 义务，或因源是常量而免除目标借用检查 |
| IIR 与后端 | 消费已选实例、合法常量表示和明确存储来源 | 自行再选择 Splice、重建源 VM 地址、按名字恢复阶段 |

当前 materialization 的 `bind_call_with_layouts_and_type_views` 调用还需要保留
trait 来源、接口契约、类型视图和程序义务。迁移不只改主体：`bind_inputs_with_layouts`
中的前/后置条件及默认输入也调用 `wrap`，闭包初始化由自己的捕获 frame 维护。
这些消费者须一起纳入范围，不能只让返回值用例通过。

准备按以下完成点区分，名称仅描述实现职责：

```text
(source-use (binding ...) (phase comptime) (domain ...) (scope ...))
    -> (delivery-demand (role runtime-value) (expected ...) (control ...))
    -> (selected-construction (impl ...) (target ...) (world ...))
    -> (prepared-input (value ...) (formation ...) (dependencies ...))
    -> (checked-residual (call ...) (contracts ...) (storage ...))
```

这是未来 Display 的结构约定示意，并非当前可导出的 IR。稳定 dump 应能看出源输入
与目标调用是两个实体，显示绑定/控制/实例关系，并正确转义名称；实现新 IR 时同步
提供携带上下文的 Display，不以地址或 Debug 输出充当身份。

阶段选择可能依赖待定域，输入准备也可能需要源 invocation。等待应归现有 Region/
准备后继，持有必要 world 与 source scope；同一次恢复不重跑已完成输入。不发布
携带活动 Meta 或候选选择的“完成”对象，不把阶段错误降成直接传送源值的 fallback。
代码实例的缓存可复用检查产物，但执行时仍保留每次 Splice 调用；不扩大当前仅 Type
结果 invocation 的缓存范围。

错误需要保留失败的真实边界：缺协议/实现、目标域不符、源输入尚未完成、资源耗尽、
目标表示无效分别报告。Seed 正例当前在选择前报源域错误，是待修行为；不能只换一条
错误文案后把它登记成已接通。没有合法 runtime 用途的 Type/代码值也不能靠新交付
机制获得权限，仍走原有类型观察或 CMTT 的上下文准入。

## 8. 纵向迁移与验收

这条工作线归入现有 M2/M4/M5，不是新的并行框架，也不建立额外调度账本。

| 迁移闭包 | 必须接通 | 退役/拒绝条件 |
| --- | --- | --- |
| 源值与使用上下文 | 参数、常量、函数结果、投影、所需阶段和预期域 | 不能只修返回位置；同阶段输入及类型/callee 观察不能误插转换 |
| 构造选择和底层表示 | 实际 Splice 身份、普通 runtime 调用、基础字面量构造 | 同域转换不跳过副作用，builtin 身份实现不自递归；缺实现/错误目标给阶段交付诊断 |
| 参数/捕获残余 | 稳定源值依赖、作用域、默认供应、真实 runtime 存储 | producer 不重放；闭包捕获与调用时构造不混淆；源值不变 runtime 可写槽 |
| 文本与库 | str 的转换及来源、TextBytes、通用 writer 文本方法 | 保留生命周期/读写检查；源码文本复现真正贯通，不只是换一个错误 |
| 两套执行与后端 | comptime invocation 的输入规则、runtime reference、LLVM O0/O3 | 执行次数、错误、清理及未选分支一致；活动域/预算失败不污染下次准备 |

已确认的未知仍保持未知。实现这些路径不需要修改公开 callable Type 的名字相等性
决定，也不应借此扩大 Any、quote 或 PTC 的权限。新载体必须替换真实消费者后才可
计为实现进度；仅增加枚举、side table 或成功打印的专用分支不算接通。

### 格式化变参边界：不能把静态文本输出当作 println 交付

作者再次明确目标签名为：

```flurry
fn println(comptime format: str, ...varargs: defer std.fmt.Parse(format))
```

此前只接通 `println(comptime text: str)` 与 `println_value`，是错误地缩小了
交付范围。这些函数证明 stdout、Display 与错误传播底座可用，不证明格式化
变参已完成；演示验收必须包含 `println("answer = {}", answer)`。

这不是向编译器添加 println intrinsic。最终层次和顺序是：

1. `FormatPlan.parse` 在 comptime 扫描格式串，形成原文字节区间和有序字段。
   已有实现是 `library/std/fmt_impl/parse.fl`，通过真实 sysroot 使用公开路径。
   初步语法为 `{}` 与转义花括号；其它字段形式明确报告错误。
2. `Parse` 根据计划计算参数类型描述。每个 `{}` 对应一个独立的未知类型，
   附带 Display 约束。两项未知描述不意味着两项类型相等，也不等于同一个待解
   metavariable。格式串本身决定字段数量，不能反过来按实参数量补全格式串。
3. 这种结果须作为普通 comptime 数据流经返回、局部量、列表与类型构造输入。
   `meta.List<Type>` 仍是普通列表，Tuple 仍保留普通头；不添加格式专用参数槽，
   不将 Any 冒充具有布局的具体 Type，也不把描述相容关系送入 PTC 当作等式。
4. `defer` 完成该计算后交付的是可供参数检查的描述。参数收集按槽位匹配实参，
   每个实际类型在调用方证据下分别证明 Display，保留对应具体类型及存储要求。
   对函数定义体提供的假设不能拿来自证调用方的实参准入；待定值不等于 Any。
5. 实例的 tuple 布局形成后，普通 inline 结构迭代生成输出。原文按计划输出，
   参数通过各自的 Display 写入同一个 writer。实参在普通调用位置按源码顺序
   各求值一次；重复检查、格式展开、O3 优化不能重放 producer 或省略资源清理。
   writer 的短写、失败前已输出前缀与清理规则继续适用。

当前通用缺口并不只在 std：`FrozenDescription` 主要是普通定义检查中的
Any/Known 观察；`type_computations::State::Complete` 保存具体 Term，
`execution::publish` 直接把 Type 结果写入类型求解变量；`type_expr` 的 refinement
先要求具体 base，再证明要求并返回 base。它们不能直接承担可计算、可嵌套、带
约束的未知描述。应扩展描述数据及其 VM 交付与参数准入边界，分别保留“描述
已计算完成”与“runtime 类型已具体化”两个事实，不能只在 publish 放宽匹配。
具体函数类型和 runtime IR 的发布仍要求所需布局与义务完整。

完整的状态和数据分工见[可计算类型描述与调用准入](type-descriptions-and-admission.md)。
尤其不能将 FrozenDescription::Any 当作已经求值的 Type 数据；Parse 只声明返回
Type 也不自动授予消费者 tuple 结构或 Display 证据，定义检查仍需可靠结果观察。

`development/repros/refined-any-type-producer.fl` 隔离最小合法输入：普通 Type
生产函数返回带 Display 约束的 Any，经 defer 用于两个 tuple 元素。当前在名字
解析时报 `Unresolved identifier Any`，说明显式源描述入口也未接通。复现命令：

```sh
python3 -B development/check.py --timeout 900 -- run -p luna -- check development/repros/refined-any-type-producer.fl --sysroot library --build-target x86_64 linux 64
```

报告 `/tmp/luna-tools-yw2cqbvc/result.json`，退出 1。这是待实现的合法程序，
不是应当永久拒绝的测试；不能把这个失败改成 language rule。

最终验收须同时覆盖：零／多个字段、不同机器类型与用户 Display、花括号转义、
UTF-8 与 NUL、参数数量不匹配、无 Display 类型、错误格式与 runtime 格式输入；
还包括按名称完整供应 tuple、展开输入、实参副作用顺序、writer 中途失败和 O0/O3。
当前 scanner 的通过不能替代其中任何一个调用与输出契约。

#### 已接通：收集准备的阶段完成点

具体 Type 的对照用例也曾失败：`comptime count: Integer` 后接
`...values: defer BooleanFields(count)`，虽然调用域已算出，收集准备仍把 count
提升成 runtime let，IIR 因而报告 comptime-only 存储。普通已知域与 comptime
变参包也有同一缺口；仅改 open 函数不能覆盖它们。

收集、属性与展开现在共用 `prepare_call_inputs`／`finish_input_preparation`
确定准备完成点。内层保留所有源码位置供外层合并；最外层完成后复用既有
`freeze_staged_preparation` 执行所需 comptime producer 并替换其使用。
不再提前把 staged 项从位置序列删除，也不在属性重排完成前删除其准备语句。
comptime callee 保留整个 invocation 的准备边界，错误退出恢复外层准备状态。
该修改不赋予 Any 布局，也不解决描述计算的剩余缺口。

定向验证（工作目录 `/home/fin/luna`）：

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test staged_variadic_inputs --test library_std_format --test extended_calls --test contextual_spreads --test open_functions --no-fail-fast -- --test-threads=1 computed_tuple_domains staged_inputs_do_not_escape format_plan collection collected variadic packet spread
```

`/tmp/luna-tools-c4ev5pyt/result.json` 退出 0：分别 2、4、9、15、107 项通过。
包括参数数量／字段域拒绝、不同调用的类型替换隔离、完整 Type 包、交错具名
实参的次数与顺序、comptime 收集失败与默认输入次序；不是全 workspace 验证。
scanner 的测试仅证明扫描契约与编译期阶段，不能据此称 Parse 已实现。

过程中的真实失败也保留区分：`dufnais4` 揭示原有 runtime Integer 存储问题；
`2vg1kiiy` 的简单跳过方案破坏展开位置，`q9hcgh1_` 的过早冻结破坏属性合并。
这些方案均已由上述共享完成点替代。报告均位于对应 `/tmp/luna-tools-*/result.json`。

最终补查 `extended_calls`、`contextual_spreads` 与 `library_std` 整组：
`python3 -B development/check.py --timeout 900 -- test -p luna --test extended_calls --test contextual_spreads --test library_std --no-fail-fast -- --test-threads=1`
退出 0，分别 38、16、14 项通过，报告 `/tmp/luna-tools-k05xc4q2/result.json`。
其中 std 为真实 sysroot 的 LLVM 原生检查，含 O0/O3 输出、File、短写和 EINTR；
不是 `{}` 参数替换验收。准备阶段修复已单独提交为 `488303ff`。

## 9. 证据与当前限制

以下先保留 `689d45c5` 阶段分析的证据，再记录各次实际迁移。早期对照只证明
当时的状态，不代表当前所有入口；源字段操作数的接线与限制见下节。工作目录均为
`/home/fin/luna`。

- `library_std::splice_construction_runs_per_use_after_staged_inputs_are_ready`
  使用真实 sysroot 与资源检查，在 O0/O3 输出三次 `convert;`：命名常量、显式
  staged 参数调用、runtime 分支各一次。构造没有提前进入 comptime 或被当作
  相同输入的缓存结果。测试通过，命令为
  `python3 -B development/check.py --timeout 900 -- test -p luna --test library_std -- --test-threads=1 splice_construction`，
  `/tmp/luna-tools-v4xslvoa/result.json`，退出 0。
- Rust 测试文件格式化退出 0，`/tmp/luna-tools-ifwb1n06/result.json`。
- `689d45c5` 上完整 `library_std` 定向组已通过 11 项，报告为
  `/tmp/luna-tools-xi8zy3i8/result.json`。这是前一轮证据，本轮没有重复该测试组。

本轮在仓库内复现，构建共用以下选项（SOURCE 和 OUTPUT 是下面各行的实际路径）：

```sh
python3 -B development/check.py --timeout 900 -- run -p luna -- build SOURCE --entry run --sysroot library --fuel 10000000 --output OUTPUT
```

| SOURCE | OUTPUT | 构建结果与报告 |
| --- | --- | --- |
| `development/repros/staged-parameter-splice-control.fl` | `/tmp/luna-staged-parameter-splice-control` | 退出 0，`/tmp/luna-tools-67x1wz8z/result.json` |
| `development/repros/staged-parameter-splice.fl` | `/tmp/luna-staged-parameter-splice` | 退出 1，预期 u8 而得到 Seed；`/tmp/luna-tools-0p0s7ok6/result.json` |
| `development/repros/staged-text-observations.fl` | `/tmp/luna-staged-text-observations` | 退出 1，资源检查报 `use of unavailable value`；`/tmp/luna-tools-ec4qzde3/result.json` |

执行 `python3 -B development/command.py --timeout 30 -- /tmp/luna-staged-parameter-splice-control`
退出 0，输出 `(bool true)`；`/tmp/luna-tools-2j0phy_q/result.json`。
失败行是仍待修复的合法源码，不是正式拒绝测试；保留成功对照用于区分协议不可用
与参数入口缺交付。上述准备状态和底层构造接口仍为设计，不能计为新的生产接线。

### 源字段操作数迁移的验证边界

`source_field_observation_precedes_runtime_construction` 使用真实 sysroot、资源检查和
LLVM O0/O3，外层 Packet 没有 Splice，字段 Seed 的 RuntimeTarget 为 u8。
它覆盖命名常量、局部 const、普通／泛型 comptime 生产者、存储的函数身份与显式
comptime 回调。直接交付 Seed 字段执行一次转换；继续观察 byte 则不执行 Seed
转换；runtime 分支只在被选择的位置转换。输出严格核对七次 `convert;` 和 true，
不以“最终值相等”替代构造次数。直接使用完整 Packet 与把 runtime Packet 当成
u8 的用法另有拒绝检查。基线 `ebf4369d` 上同一正例在 Packet 的 Splice 选择处
失败，报告 `/tmp/luna-tools-0729jqlw/result.json`；迁移后完整 `library_std`
14 项通过，`/tmp/luna-tools-hp1zw4_2/result.json`，退出 0。

旧 CMTT 字段 fixture 没有绑定 Splice，曾依赖直接重建 runtime 数据。标量字段
用例改用真实 sysroot；独立测试 Add 选择的 BitVec fixture 明确绑定自己的 Transfer
协议及 bool/u8 实现。错误限定值、函数载荷的 fixture 同样显式提供本地协议和
具体实现，保留原有标签重排、快照、阶段拒绝断言。这不添加 blanket 实现，也不
宣称复杂类型自动派生已完成。新的缺协议反例即使源域和目标域相同也要求拒绝。
不能为测试方便在用户包中为外部 Splice 和外部类型新增实现以绕过孤儿检查。

完整 CMTT 组在当前 compiler 上执行 596 项，590 项通过，6 项旧 fixture 缺少
转换协议，报告 `/tmp/luna-tools-125ia4tx/result.json`，退出 101。随后只修改这
6 项 fixture 的协议／sysroot 供应，未改原有预期；相关 9 项复验中 8 项通过，
`/tmp/luna-tools-_x_1xmg1/result.json`。余下一项在完整 sysroot 下触及准备预算，
按原有隔离范围显式绑定 Transfer 后通过，`/tmp/luna-tools-b3dwyjaa/result.json`，
退出 0。没有把这些分次结果写成“最终状态完整组一次全绿”。相关命令：

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt -- --test-threads=1
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt -- --test-threads=1 contextual_data_properties local_constant_callbacks_survive pattern_generators_lift_data pattern_generators_run_stateful
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt -- --test-threads=1 pattern_generators_run_stateful_loops_and_early_returns
```

当前 driver 构建报告 `/tmp/luna-tools-k88p5vj_/result.json`。原有
`source-field-before-splice.fl` 的 build 与运行分别退出 0，报告
`/tmp/luna-tools-8tifgt9o/result.json` 与 `/tmp/luna-tools-52dgjpde/result.json`，
输出只有 `(bool true)`，没有 Seed 的 `convert;`。

完整库的准备成本缺口保存为
[pattern-generator-sysroot-preparation.fl](repros/pattern-generator-sysroot-preparation.fl)。
对旧 driver `/tmp/luna-value-delivery-7f6d3715` 和当前 driver
`/tmp/luna-source-operands-current` 执行相同命令：

```sh
python3 -B development/reproduce.py --timeout 90 --binary DRIVER -- check development/repros/pattern-generator-sysroot-preparation.fl --sysroot library --build-target x86_64 linux 64
```

两者均在 required comptime preparation 报 StepsExhausted，退出 1；报告分别为
`/tmp/luna-tools-evvi9r5p/result.json` 和 `/tmp/luna-tools-7e83xgls/result.json`。
这是 `c15a0039` 阶段保留的成本问题，不是语言拒绝例。后续通过
[Phase 与 VM 共享已检查参数域](world-storage-and-lifetimes.md#18-phase-与-vm-冷构造共享已检查的声明参数域)
去掉同一个冷构造中的重复元数据检查；该源码现在进入
`cmtt_storage_world::repeated_pattern_invocations_fit_the_real_sysroot_preparation_budget`
并通过完整 sysroot 编译和 IIR 执行，报告 `/tmp/luna-tools-drfbyl5c/result.json`，
退出 0。`check` 不接收执行动作的 `--fuel`；前端调用准备预算仍为 100 万步，
没有提高预算或修改公共测试入口来取得通过。跨 RequiredWorld 的重复快照与更大
程序的准备成本仍需继续处理。

反射组的 `sysroot_reflection_uses_receiver_domains_without_unfolding_type_identity`
和 `computed_normal_form_receiver_keeps_live_clause_inputs` 仍在旧 VM 构造器处报
`UnmaterializedDispatch`。它们在上面的基线复跑中也失败，不能算成本次新回归或
隐藏为通过；需要按 `support/sysroot_execution::with_comptime` 的源查询世界准入
路径迁移。`cmtt_storage_world` 6 项、`cmtt_phase_boundaries` 3 项通过，
`/tmp/luna-tools-uzh5l8n0/result.json`；该报告因这两个反射失败整体退出 101。

### 源操作数与生成器就绪状态的联合复查

`7f6d3715` 基线，工作目录 `/home/fin/luna`。通过
`python3 -B development/command.py --timeout 900 -- python3 -B development/build_host_driver.py /tmp/luna-value-delivery-7f6d3715`
保存本基线 driver，退出 0，`/tmp/luna-tools-megh2bhq/result.json`。以下直接运行该
driver，所有实际构建输入均在 `development/repros/`，没有复用旧 native 产物：

```sh
python3 -B development/reproduce.py --timeout 90 --binary /tmp/luna-value-delivery-7f6d3715 -- build SOURCE --entry run --sysroot library --fuel 10000000 --output OUTPUT
```

首批七项由同一 `process_tools.run` 顺序执行并保存报告；callback 最初的函数类型
多写了 pure，触发另一项未实现诊断，修正为普通 `comptime fn` 类型后按上面命令
单独重跑。下表使用最终有效源码的结果，不将函数类型修饰符错误算作交付证据。

| 复现文件 | `7f6d3715` 基线结果 | 目标行为 | 报告目录（均在 `/tmp/`，内含 result.json） |
| --- | --- | --- | --- |
| [source-field-before-splice.fl](repros/source-field-before-splice.fl) | 构建退出 1：field projection requires a supported struct | 先观察 CONFIG.byte，返回 true，无 Seed 转换 | luna-tools-jhhjt_7d |
| [quoted-local-changing-target.fl](repros/quoted-local-changing-target.fl) | 构建退出 1：预期 u8，实际 Seed | 一个 runtime 转换，返回 true | luna-tools-_velwvdt |
| [quoted-source-field-delivery.fl](repros/quoted-source-field-delivery.fl) | 构建退出 1：字段仍为 Seed | 先观察 Packet.item，再转换 Seed；Packet 不需要 Splice | luna-tools-e053ubx8 |
| [quoted-source-staged-argument.fl](repros/quoted-source-staged-argument.fl) | 构建退出 1：required 输入不可用 | read 的 comptime 参数接收 Seed，不转换为 u8 | luna-tools-gnytk_ul |
| [quoted-source-callback.fl](repros/quoted-source-callback.fl) | 构建退出 1：required 输入不可用 | 调用源 callback，仅交付其结果 | luna-tools-05frv_pg |
| [quoted-source-snapshot.fl](repros/quoted-source-snapshot.fl) | 构建和运行退出 0，只输出 `(bool true)` | 保留 byte 1 的旧快照，另输出一次 `convert;` | 构建 luna-tools-grdcbze_；运行 luna-tools-9cki7lyx |
| [quoted-source-runtime-branch.fl](repros/quoted-source-runtime-branch.fl) | 构建和运行退出 0，只输出 `(bool true)` | runtime 选择的分支执行一次 `convert;` | 构建 luna-tools-grq1pg4_；运行 luna-tools-pg1hxbso |

这七项在该基线都是目标行为尚未完成的复现，不是应该拒绝的正式测试，也不以 native 退出
0 作为行为正确证据。各 report 保存完整 OUTPUT 路径及命令；七项初次执行的摘要
在 `/tmp/luna-delivery-audit-7f6d3715.json`，callback 以其后重跑报告为准。

把 staged-argument 和 callback 两例的 run 改成 `{ true }`、保留原生成器声明，
另存 `/tmp/quoted-source-staged-argument-unused.fl` 与
`/tmp/quoted-source-callback-unused.fl`，使用上述 driver 的 `check SOURCE --sysroot library`
仍分别得到同样的输入不可用错误，退出 1；报告为
`/tmp/luna-tools-cfw3fdp_/result.json`、`/tmp/luna-tools-m8wonjbm/result.json`。
这确认失败发生在定义检查阶段，不能只修调用后的物化分支。

另将 callback 例的 `'(callback())` 改成 `let result = callback(); '(result)`，保存
`/tmp/quoted-source-callback-before-quote.fl`。按同样选项构建至
`/tmp/luna-callback-before-quote` 退出 0，`/tmp/luna-tools-n1faqvpu/result.json`；经
`python3 -B development/command.py --timeout 15 -- /tmp/luna-callback-before-quote`
运行退出 0，输出 `(bool true)`，`/tmp/luna-tools-5h_b5aq8/result.json`。这个对照
证明现有 VM 可以调用该 callback；缺口在 quote 内源调用的检查／执行交接。
没有要求用户永久把调用搬到 quote 外，也没有修改 compiler 或正式 docs。

### CMTT 补查证据

本轮先在 `/tmp` 探测，再将下列源码原样保留到 `development/repros`，只添加说明
注释；下面记录实际执行的临时路径，不把未重跑的仓库路径冒充日志输入。构建命令
与前表相同，fuel 均为 10000000，OUTPUT 为 SOURCE 去掉 `.fl` 后的路径。

| 保存的复现 | 实际 SOURCE | 结果与报告 |
| --- | --- | --- |
| quoted-data-splice.fl | `/tmp/luna-quoted-data-delivery.fl` | 构建退出 0，`/tmp/luna-tools-9cpmhs5z/result.json`；运行只输出 `convert;(bool true)`，`/tmp/luna-tools-223qirr0/result.json`，行为仍错误 |
| quoted-literal-control.fl | `/tmp/luna-quoted-literal-control.fl` | 构建退出 0，`/tmp/luna-tools-q9q1aspo/result.json`；运行返回 `(bool true)`，`/tmp/luna-tools-gktyve11/result.json` |
| staged-code-input.fl | `/tmp/luna-staged-code-input.fl` | 构建退出 1，`required comptime input ... is not available in this invocation`；`/tmp/luna-tools-zlz9y20_/result.json` |
| scalar-code-construction.fl | `/tmp/luna-scalar-code-construction.fl` | 单个 255 的八步构造在 generator checkpoint preparation 报 StepsExhausted，退出 1；`/tmp/luna-tools-2hghr316/result.json` |

两个原生产物均通过 `python3 -B development/command.py --timeout 30 -- OUTPUT`
运行，退出 0。退出成功不能替代第一行的可观察行为比较。

规模诊断时还执行过四个字节值的同类构造；fuel 为 10000000 和 100000000 时均在
前端准备失败，报告分别为 `/tmp/luna-tools-0cap8plq/result.json` 与
`/tmp/luna-tools-_qr3cl3x/result.json`。这**不是提高前端额度后的性能测量**：
`packages::compile_observed_with_preludes` 当前调用 `elaborate_packages_for_target`，
后者仍内部提供独立的 1000000 invocation/preparation 步额度，build 的 `--fuel`
没有传入此处。不能据相同失败推断生成器无限循环，也不能把它归为位运算语义错误。
初次探测未测定准备成本，后续计量见下文；通用预算传送和减少重复准备仍需推进，不能简单
调大内部常量或取消准入检查后宣称代码构造已经满足库的性能要求。

### 回滚保留不再准备值观察世界

沿八步构造的失败位置检查，`checkpoint_generator_cells` 原来与真实读值共用
`generator_storage_world` 和 `with_storage_vm`：每个 generator frame、词法 scope
和更新 frame 都会复制表示元数据并准备 VM，即使只为失败时恢复原有存储。
这与实际取得/准入源值的职责不同。现在回滚复用已有的 `capture_retained`；图遍历、
复制及恢复计量保留，实际读值与 old 入口复制的世界检查没有删除。

新增 `cmtt::code_generator_rollback_cost_does_not_scale_with_unrelated_layouts`
用 256 个记录和八层代码生成验证该边界。对照只切换本次修改的 places.rs：

- 旧实现仍使用 `1a6296fb` 的代码，同一新测试退出 101，错误为 generator storage
  world capture budget exhausted；`/tmp/luna-tools-cfryc7bk/result.json`。
- 修改后的单例退出 0；`/tmp/luna-tools-52owl6hm/result.json`。两次命令均为
  `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt -- --test-threads=1 code_generator_rollback_cost`。

最初用十六层生成器触发了独立的 code construction depth 限制，随后将费用用例
限定为八层，避免把深度限制混入回滚成本测试；没有放宽生产深度或额度。

真实 sysroot 再运行
`python3 -B development/check.py --timeout 900 -- run -p luna -- build development/repros/scalar-code-construction.fl --entry run --sysroot library --fuel 10000000 --output /tmp/luna-scalar-code-checkpoint`，
仍退出 1，现报 `required comptime declaration directory: value admission resource
budget exhausted`；`/tmp/luna-tools-8bygfufu/result.json`。不能以错误位置推进代替
字节构造成功，也不能据这项局部清理宣称准入世界已共享或 CMTT 性能问题已解决。

最终定向检查（工作目录 `/home/fin/luna`，均使用 development/check.py）：

- `test -p luna --test cmtt --test cmtt_methods --test library_std -- --test-threads=1 code_generator_rollback_cost generator_field_updates generator_value_writes generator_storage_checks unused_code_generators_reject generator_readonly_nominal_captures generator_entry_aggregates trait_default_forwarding_preserves_value generator_refutable_tuple_filters generator_var_else_failure generator_callback_snapshots println_and_display`
  退出 0，分别 11、4、1 项通过；`/tmp/luna-tools-2wwhom9y/result.json`。包含原存储
  身份、快照独立性、失败后的状态、runtime 指针域拒绝，以及真实 std 的 O0/O3 输出。
- `test -p luna --test cmtt_storage_world -- --test-threads=1` 为 4 通过、1 失败，
  退出 101；`/tmp/luna-tools-imurfhgx/result.json`。失败的 nested_contextual_retries
  用例在 required comptime preparation 耗尽额度。切回旧 places.rs 后，以同一命令
  追加 `nested_contextual_retries` 复核仍失败，此时在 lexical scope snapshot 耗尽；
  `/tmp/luna-tools-6c7l4gvq/result.json`。这是当前基线的预算限制，没有改测试预期。
- 恢复本次实现后，`test -p elaboration -p comptime_interpretor --lib -- --test-threads=1 storage_observation::tests memory::checkpoint::tests`
  退出 0，3 项观察预算与 2 项存储图测试通过；`/tmp/luna-tools-r3borewb/result.json`。
  图保留不授予陌生名义描述成员资格，失败捕获不修改原图，恢复保留共享身份和责任。

以上命令均以 `python3 -B development/check.py --timeout 900 --` 为前缀。
两个修改的 Rust 文件已经定向格式化；git diff --check 通过。未执行全 workspace。

### Required comptime 准备不再建立 runtime 指针权限世界

`PreparedRequired` 原来在准备源码世界之外，还复制所有已检查的 trait 与 impl，
建立独立的 Copy/Drop 声明目录。该目录只服务于解释器的 runtime 指针读写权限
回调；comptime VM 的 `eval_pointer` 在调用该回调前就拒绝所有指针操作。
因此本阶段删除这份重复世界、其查询回调与 Type Session 的失效安装入口。
comptime 值引用仍通过自己的存储与准入规则处理，不借用 runtime Copy 权限。

runtime 解释器的指针权限检查不变；Type 值描述 runtime 指针、函数签名以及
quote 内的 runtime 指针代码仍合法。实际源码世界的目录、成员查询和存储准入
继续保留。CLI invocation、key 与 memoization 的同类清理已在后续接通，见
[执行角色与资源权限](comptime-value-admission.md#执行角色与资源权限已分开)。
这没有完成通用声明世界共享，也没有完成 staged 参数的 runtime 交付。

删除 runtime 权限世界后、目录尚未惰性化时，八步字节构造仍失败：在 `/home/fin/luna` 执行
`python3 -B development/check.py --timeout 900 -- run -p luna -- build development/repros/scalar-code-construction.fl --entry run --sysroot library --fuel 10000000 --output /tmp/luna-scalar-code-no-pointer-authority`，
退出 1，报 `required comptime preparation failed: interpretation StepsExhausted`；
`/tmp/luna-tools-8oog5bfa/result.json`。失败位置改变不代表代码构造已可用。

当前工作区的端到端检查（工作目录 `/home/fin/luna`）：

- `python3 -B development/demo_println.py` 构建与原生运行退出 0，报告
  `/tmp/luna-tools-alxkh2_h/result.json`。中文、运行时整数与 pid 输出正确，
  产物保留在 `/tmp/flurry-println`，未出现额外的 `(unit)` 输出。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_std --test comptime_pointer_phase --test comptime_storage_domains --test cmtt_storage_world -- --test-threads=1`
  退出 0，分别 11、5、7、5 项通过；
  `/tmp/luna-tools-z0uz251g/result.json`。包含真实 sysroot 的 comptime 值引用、
  old 快照、名义对象，以及 runtime 指针类型与 quoted runtime 指针控制用例。
  此前失败的 nested_contextual_retries 用例此次通过，预算常量与测试预期未改。
  std 原生测试覆盖 O0/O3 输出、文件关闭、短写、EINTR 与错误传播。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib -- --test-threads=1 program::required::prepared::tests type_computations::`
  退出 0，24 项通过；`/tmp/luna-tools-o69fgbqr/result.json`。覆盖准备失败的预算
  结算、值引用写回、目录身份隔离、重入和 Type/非 Type 两条调用路径。

该阶段未执行全 workspace 测试；当时的预算失败如上保留，不以定向通过替代
完整 CMTT 性能交付。下面记录此后对同一复现的修复。

### 查询目录按需求建立

在 `7a63879e` 基线上临时计量相同字节构造，得到 47 次 RequiredWorld 创建和
47 次冷 VM 准备，无同一代码会话的热命中。查询目录累计消费 368173 步，已完成
VM 准备累计消费 430509 步，最后仍在 VM 准备耗尽。末段每次只有一个可执行函数，
却含 41 个 trait 与 232 个 impl；目录成本为 8121 步，VM 准备约 9600 步。
这是预算工作量，不是 CPU 时间或分配量。计量只输出已有计数，没有修改额度；
日志为 `/tmp/luna-tools-xgsnvom3/result.json` 和 `stderr.log`，退出 1。
临时 trace 已移除，生产代码不增加日志开关或计量分支。

RequiredWorld 现在只接管声明快照。首次类型／成员查询通过既有
SourceInvocationQueries 建立目录，并消费该查询的真实预算；之后同源查询共享
已完成目录。发布器证明也走这条路径。没有查询的 Type 和普通值调用不建该目录，
但 VM 的声明、阶段、存储与输入准入仍执行。重复声明仍拒绝；索引耗尽不发布部分
目录，后续有额度可重试。具体所有权见
[Required 世界](world-storage-and-lifetimes.md#11-required-计算的共享声明快照)。

在 `/home/fin/luna` 执行
`python3 -B development/check.py --timeout 900 -- run -p luna -- build development/repros/scalar-code-construction.fl --entry run --sysroot library --fuel 10000000 --output /tmp/luna-scalar-code-lazy-directory`，
退出 0；`/tmp/luna-tools-wvcmzfla/result.json`。随后执行
`python3 -B development/command.py --timeout 30 -- /tmp/luna-scalar-code-lazy-directory`，
退出 0，输出 `(unsigned-integer 255)`；`/tmp/luna-tools-v38g1lim/result.json`。
两次构建的 frontend 额度相同，仍未将 CLI fuel 接入 frontend。

此项没有跨不同 RequiredWorld 共享 VM 元数据，不能用声明号或 package 内容近似
相等来取得旧世界的准入许可。更大代码组装仍需减少重复声明捕获和冷 VM 准备，
保留独立的输入、前提、可变存储及失败恢复责任。

定向验证均在 `/home/fin/luna` 执行，以
`python3 -B development/check.py --timeout 900 --` 为命令前缀：

- `test -p elaboration --lib program::required::prepared::tests -- --test-threads=1`：
  24 项通过，退出 0；`/tmp/luna-tools-me3772hh/result.json`。覆盖无查询调用、
  首次查询计费／失败／重试、错误声明拒绝、前提撤回、世界身份、重入与存储写回。
  初版新测试曾误断言重复声明在 InvalidCallPlan 层报错，实际更早由 phase 检查
  拒绝；`/tmp/luna-tools-nr54s1h4/result.json` 为 23 通过、1 断言失败。已按该
  真实拒绝边界修正断言，未修改生产检查以迎合测试。
- `test -p luna --test cmtt_storage_world --test builder_invocations --test library_std --no-fail-fast -- --test-threads=1 single_hole_scalar optional_branch_probes contextual_retry entry_snapshot_retains nominal_object_entries nested_contextual_retries required_builder required_enum required_newtype required_value package_driver builder_driver println_and_display`：
  分别 6、6、1 项通过，退出 0；`/tmp/luna-tools-nojj3jwr/result.json`。新增
  标量构造检查 0、165、255，其余覆盖真实 meta builder、存储快照、失败重试及
  std 的 O0/O3 字节输出与外部效应。

修改的 Rust 文件已定向格式化，git diff --check 通过。未运行全 workspace，
没有把以上通过扩大为完整数值生成库或一般参数阶段交付已完成。
