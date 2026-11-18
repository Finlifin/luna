# Comptime 值的准入、快照与恢复

一个 VM 可以复制函数身份的内部表示。另一个 VM 收到这份身份后，仍须在自己的声明世界里找到函数，检查子句、签名和阶段，才能把它交给调用方。结果缓存还需要证明两次调用可互换。值的表示、成员资格、传送和调用复用分别有自己的消费者。[值与发布](values-staging-and-publication.md) · [冻结的语言契约](../docs/compilation/staging-and-freezing.md)

## 接收世界中的值域

| 接收点 | 检查内容 | 交付结果 |
| --- | --- | --- |
| 当前调用的形参或返回位置 | 用已取得的输入实例化声明域，检查实际值及形成契约 | 在该调用中可使用的值；值引用按接收者契约传送 |
| NF、名义类型和别名的子句 | 检查声明 telescope 中每一项的域、作用域及依赖 | 完整的声明实参。NF 保存头身份，别名可继续处理透明主体 |
| 已发布的定义 | 检查开放 binder、稳定依赖和允许保留的残余 | 可在约定作用域中消费的定义 |
| 局部常量或快照 | 检查数据与捕获图、共享关系和接收者权限 | 可恢复的根值。包 metadata 只接收 wire 版本列出的值 |
| 结果缓存 | 检查调用、输入等价与可复用结果 | 在同一依赖条件下可以省略的纯调用 |

NIR 的共同入口负责不可变描述的递归成员检查。VM 提供当前世界的布局、声明域、集合、callable 和目标信息。结构上匹配的名义值还须检查形成义务；嵌套载荷遵循同样的检查。只读观察可使用已有见证，主动接收时要在接收作用域重验。查询耗尽预算、前提未决、输入非法分别交还给调用者。一次可选查询没有找到见证，只说明该查询的结果。[共同值域检查](../compiler/nir/src/value_admission.rs) · [VM 的域查询](../compiler/comptime_interpretor/src/admission.rs) · [实际值检查](../compiler/comptime_interpretor/src/value_types.rs)

收集域由声明及受检结构观察提供。即使载荷为空，`meta.List<u8>` 与 `meta.List<Type>` 仍携带各自的域。用 `Tuple<.Es = Es>` 交付已有列表时，整个列表作为一个普通 comptime 值进入具名槽。冻结和恢复保留 NF 头、序列顺序、名义身份、错误标签、嵌套 optional 与 Object 中各项的键及域。Type 描述可携带尚待消费的结构条件；需要类型身份的操作另行取得身份依据。[类型描述与身份](type-descriptions-and-admission.md) · [值表示](../compiler/nir/src/comptime_values.rs)

调用方授权的抽象值只能在对应 binder 和域中使用。私有 VM 在校验 dispatch 前安装这次调用的作用域，完成整份校验后才可执行。共享代码目录不会把该权限传给下一个闭合 VM。检查普通实参的 trait 义务时，调用者给定的前提和已存在的具体 impl 都可以提供证据；是否执行哪个方法体由方法选择另行决定。程序状态和内存规格归程序验证层。[作用域中的 dispatch 准入](../compiler/materialization/src/input_admission.rs) · [开放与闭合构造用例](../compiler/materialization/tests/scoped_dispatch_inputs.rs)

从源码交给 VM 的权限范围也决定证明的有效期。定义侧可以保留显式 binder 和合法 residual；当接收点要求一个已经完成的普通值时，每个必需槽位都要有实际载荷。通用传送接口的设计应将下列信息随工作传递。

| 工作 | 由谁提供 | 何时可以发布 |
| --- | --- | --- |
| 观察实际值、域、来源和依赖 | 原检查区域或持有值的 VM | 可变位置的观察只对相应内容有效 |
| 检查成员资格与名义形成义务 | 接收世界及本次词法作用域 | 整个值及嵌套载荷均通过 |
| 准备目标表示 | 了解接收契约的消费者 | 声明已重绑定，捕获、共享与所有权动作均可完成 |
| 安装结果 | 持有目标槽或恢复图的事务 | 依赖都已接通，失败能够丢弃私有工作，不会暴露半成品 |

这些是目标接口应交付的信息。当前 VM 在主动接收点重新核验，尚无一份贯穿所有消费者的准入凭证。存储表示能共享时也要保留接收检查。[接收世界](world-storage-and-lifetimes.md)

## 源码次序与完成槽位

以这个普通调用的输入为例。第一个子句需要后面的 `Byte()` 交付元素域，但它读取 `byte` 的时候，后面的赋值尚未发生。

```flurry
comptime fn Byte() -> Type { u8 }

comptime fn first() -> T
where T, ...values: meta.List<T>
{
    values[0]
}

comptime fn check() -> bool {
    var byte: u8 = 7;
    let result = first<.values = [byte], do { byte = 9; Byte() }>();
    result == 7 and byte == 9
}
```

这里有三种顺序。源码规定求值和控制退出的位置。接收域的依赖决定哪些成员检查可以完成。声明顺序决定 callee 读到的槽位。受检的普通值或已执行的前缀须留在原调用中；后面的 Type 返回后，只继续仍待完成的域检查。若前缀从外层函数返回，后面的生产者和 callee 都不会执行。具名完整供应、逐项收集、展开和默认值也要保留各自的求值位置。[输入检查与原调用](audit-reply.md#一次调用内的恢复)

现有实现中，NF 和名义输入的执行帧可以保留部分结果；域复制与值传送都成功后才发布槽位。普通 callable 与 NF 已经共享源码路由和准备过程，列表和元组的检查帧可以按位置保存字段。普通 callable 的消费端仍把准备结果收束为已定型的 NIR 操作数。上例在检查 `[byte]` 时就缺少元素域，尚未运行 `Byte()`。把列表显式标为 `meta.List<u8>` 可验证随后的 VM 交付，但不能验证这条待决输入路径。[源码检查帧](../compiler/elaboration/src/program/block_regions/aggregates.rs) · [普通 callable 消费端](../compiler/elaboration/src/program/dynamic_instances.rs) · [已完成输入槽](../compiler/comptime_interpretor/src/input_bindings.rs) · [顺序、快照与早退用例](../compiler/luna/tests/type_input_execution_order.rs)

执行计划里，`order[position]` 指向声明槽位，展开生产者的 `before` 指向源码求值位置。两个生产者可以位于同一位置；即使最后一次展开贡献零个值，它也可能产生副作用。普通 callable 的子句降码、NIR 捕获分析和程序验证按这个次序交替处理生产者和槽位。对外部 NIR 的检查还要确认每个槽恰好出现一次、局部绑定互异、展开位置单调且没有越界。在 VM 中，这批局部变量必须与调用者的活动变量分开。调用取得绑定清单后一直持有到完成或取消，执行别名主体时也保留它。字节码的环境捕获包括可能与计划绑定发生冲突的调用者位置。[输入顺序](../compiler/nir/src/program/input_schedule.rs) · [调用内的作用域](../compiler/comptime_interpretor/src/input_collection.rs) · [含空展开和默认值的调用用例](../compiler/luna/tests/clause_packet_preparation.rs)

递归调用还要区分同一声明的两次绑定。设内层调用传入 `A = 外层 B`、`B = bool`、`T = u8`。一个域为 `Tuple<A, T>` 的槽只等本次调用的 A 和 T。若它们已经交付，该槽可以立即供后续实参读取，即使内层 B 尚未求值。若 T 较晚才到，VM 保存已读字段和所选分支；A、T 都就绪后，从原始标注同时代入，得到 `Tuple<外层 B, u8>`。在先前代入过的标注上再做一遍替换，会错误地把首字段改成 bool。NF 与名义构造、别名检查共用这套等待和恢复规则，各自执行声明的最终准入检查。[VM 输入帧](../compiler/comptime_interpretor/src/input_collection.rs) · [同编号绑定回归用例](../compiler/comptime_interpretor/src/input_collection/tests.rs)

收到完整参数包后，NIR 用声明身份建立槽位索引。每个原始域只登记一次它引用的本次调用槽位。某个值通过接收检查后，只更新依赖这个槽位的消费者。就绪槽沿声明顺序检查，本轮已越过的位置留给下一轮。newtype 从尾部的第一个捕获槽开始，检查完成后仍按原声明顺序解释类型身份。若就绪槽耗尽而仍有未完成的依赖，才报告域依赖成环。[参数准入与调度](../compiler/nir/src/nominal_parameters/telescope.rs) · [前向依赖和捕获顺序](../compiler/nir/tests/dependent_admission.rs)

例如一个名义类型的输入同时含有列表、函数和 `field: T`。若 `T` 返回 `Type`，后续域可以使用作为类型传入的 `field`。VM 按声明顺序借用先前取得的值，由当前世界提供集合元素域、函数签名、名义布局和词法绑定证据。接收方仍检查整个输入包，包括后续域未引用的实参。通过后，字段和契约复用本次替换的索引。复制预检计算模板自身及每次实际展开的载荷，同一实参出现两次就按两次展开计费。此证据的有效范围是当前接收世界和调用者作用域。[VM 接收查询](../compiler/comptime_interpretor/src/admission.rs) · [源码到两种 VM 的用例](../compiler/luna/tests/declaration_input_plans.rs)

签名中的标注也需要区分两种工作。定义检查使用词法绑定身份判断标注依赖的输入，按已有参数域检查完整表达式及其分支前提。实例形成仍由原调用提供具体值。对 `u8` 参数计算 `n + 1` 时，定义检查必须看到保护加法的条件；调用时恰好得到小整数不能替定义证明算术安全。[词法依赖](../compiler/elaboration/src/program/lexical_inputs.rs) · [标注行为用例](../compiler/luna/tests/open_annotation_behavior.rs)

## 值引用与快照

comptime 方法的 `self` 使用值引用。VM 在当前会话内保存 owner、投影路径与根重绑定权限。转交两个指向同一位置的引用必须保留别名；写入后再次读取，得到同一位置的新内容。已经独立取得的数据快照则保留旧内容。右值可能替换容器或执行回调，所以写回要在右值执行后重新定位路径，核对权限和域。写回失败不能造成半更新，也不能重放右值已产生的行为。[值引用实现](../compiler/comptime_interpretor/src/value_reference.rs) · [容器更新](../compiler/comptime_interpretor/src/immutable_storage.rs)

Comptime 调用、缓存键协议和 Type 计算均无需 runtime 指针 Copy 或 Drop 授权。runtime 参考执行仍负责指针权限。一个 comptime Type 可描述 runtime 指针，quoted code 也可包含合法的 runtime 指针操作。[阶段与资源权限的公开用例](../compiler/luna/tests/comptime_resource_independence.rs)

稳定快照的设计需要表达图中的共享。相同闭包出现在两个字段里，恢复后仍是同一闭包；两个环境内容相同的独立闭包则保留各自身份。当前局部常量路径可从已求值的捕获构造恢复函数。一般跨世界恢复还需私下重绑定声明、分配节点、检查域与捕获、连接边，最后才发布根。遍历时能识别环只解决终止问题，环的归属、逃逸与回收仍须有依据。包解码也没有执行任意用户工厂的权限。[局部常量重建](../compiler/elaboration/src/program/constants.rs) · [捕获图检查](../compiler/comptime_interpretor/src/comptime_graph.rs) · [包传输格式](build-input-protocol.md)

公开库的可观察行为也约束快照实现。向同一列表追加自身时，追加的输入应来自该次操作取得的序列。遍历过程中回调更新原容器，后续遍历仍须遵守本次取得的遍历范围。元素 Eq 的短路和 Hash 的调用次数也不能因更换内部存储而改变。[List](../library/meta/data/list.fl) · [列表 Eq](../library/meta/data/list_equality.fl) · [列表 Hash](../library/meta/data/list_hashing.fl)

## 函数身份与源码契约

函数身份的表示成本可在没有源码世界时估算。实际接收要解析声明、受限代入参数并核验 callable 域。同一份已编译代码可由多个 VM 复用；前一个 VM 认可的本地 callable、可变捕获和 dispatch 授权属于原 VM。若源码调用产生稳定的命名函数身份，快照保存其声明与实参，恢复时仍须在接收世界校验。[Callable 域查询](../compiler/comptime_interpretor/src/callable_domains.rs) · [函数身份验收](../compiler/comptime_interpretor/tests/runtime_function_identities.rs)

例如 runtime 接口 `Factory.make` 声明返回关联类型 `Item`，在一个调用位置可能已知它等于 `bool`。重建 NIR 时，内部方法节点保持公开声明、接收者和方法实参，外层 callable 视图才向该使用位置交付已观察的类型。编译期传送这个身份不执行 runtime 方法；把方法体换成不终止循环也应能完成编译。源码阶段可读取 trait 成员各自的规格，只有需要普通函数签名的消费者才作相应投影。[公共签名代入](../compiler/nir/src/member_types.rs) · [方法传送用例](repros/cmtt-runtime-associated-method-transport.fl)

## Any 中的代码

Any 放宽接收描述。恢复 quoted code 时仍须取得实际值的 expr、pattern 或 arm 类别、按序的 runtime 上下文、主体、固定依赖与控制接口。pattern 和 arm 的输出绑定在匹配成功后进入对应范围。每段 quote 至多一个 splice；quote fn 的普通实参只求值一次。目标类型不能替一个未知值生成缺失的代码见证。小写值级 any 与一般跨稳定边界的代码传送仍待接通。[Any 与代码见证](../docs/cmtt/any-and-code-evidence.md) · [VM 代码值](../compiler/comptime_interpretor/src/code_values.rs)

若先从可变 Object 中取得 expr 快照，再把该项改成 pattern，已有快照可继续按原接口使用。重新读取同一个键则需要检查 pattern 的接口。路径相同只说明访问同一个容器位置；它不足以复用先前的接口证明。

## 预算与复用

表示相等用于共享不可变数据；类型相等保留 NF 头、实参和作用域；源 comptime Eq/Hash 按所选实现形成缓存键。当前结果缓存仅考虑纯且输出为 Type 的 invocation，输入准备和默认值求值先完成。散列碰撞要继续检查等价；若调用仍可区分按源协议相等的输入，缓存还须保存额外判别。可缓存与可调用有独立的接受条件。[结果缓存](../compiler/luna/src/memoization.rs) · [键协议](../compiler/luna/src/comptime_keys.rs)

域查询、布局代入、复制或展开载荷、编码和执行源协议都消耗实际工作预算。一次查询里的失败要保留类别与已消耗的额度。阶段检查可复用同一不可变包中已完成的声明域表；新包里即使声明编号相同，也须使用新包的域。借用方法规格或已有索引只省去重复准备。当前通用冻结图、持久恢复协议、普通 callable 在原激活帧中等待计算域，这三处仍有各自的实现缺口。[声明域表](../compiler/nir/src/phases/storage_domains.rs) · [世界与生命周期](world-storage-and-lifetimes.md) · [当前审查复核](audit-reply.md)
