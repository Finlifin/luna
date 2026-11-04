# Comptime 值的准入、快照与恢复

状态：内部设计，未修改实现。承接[值与执行总览](values-staging-and-publication.md)，
具体拆开值的成员资格、可传送性、物理编码和可缓存性。依据为
[comptime 值引用](../docs/comptime/index.md)、
[两阶段冻结](../docs/compilation/staging-and-freezing.md)、
[Any 与代码见证](../docs/cmtt/any-and-code-evidence.md)及
[纯调用与结果复用](../docs/comptime/purity-and-normal-form.md)。
不将旧实现中的 comptime pointer、TypeList 特权或受限冻结格式提升为语言规则。

## 1. 静态审计结果

| 源码 | 已有能力 | 仍需统一的边界 |
| --- | --- | --- |
| [nir/comptime_values.rs](../compiler/nir/src/comptime_values.rs) | frozen list 保留准确声明域，Type 元素也使用普通名义值载体；标量范围、sort/scope 检查 | validate 明确不承担全部 aggregate/abstract 准入；不能把结构通过当作形成证明 |
| [normal_terms.rs](../compiler/comptime_interpretor/src/normal_terms.rs)、[normal_values](../compiler/comptime_interpretor/src/normal_values/mod.rs) | 受预算的数据编码/恢复、作用域及恢复后 has_type 检查 | 各容器、Type、callable 分支仍与外部准入逻辑共同承担职责，尚无统一受限结果 |
| [constants.rs](../compiler/elaboration/src/program/constants.rs) | 已求值依赖不重算；局部常量保留闭包图内共享，生成快照重建 factory | 不能把这种有来源的局部快照当作任意 wire 可调用工厂；生成函数尚承担内部传送职责 |
| [value_reference.rs](../compiler/comptime_interpretor/src/value_reference.rs) | owner 共享、路径重验、RHS 后写回、根重绑定权限；不依赖 runtime Copy | 位置身份、内容版本和可导出快照需在统一边界表达 |
| [comptime_graph.rs](../compiler/comptime_interpretor/src/comptime_graph.rs) | 每次观察重新检查可变捕获图，访问去重不复制 owner | visited 集合不是冻结许可，也不证明所有循环图可逃逸 |
| [code_values.rs](../compiler/comptime_interpretor/src/code_values.rs) | expr/pattern 类别与描述保留；内部表示相等不声称源 Eq | 当前代码传送限定 invocation；arm/Any/跨稳定边界的完整见证交付仍需接线 |
| [memoization.rs](../compiler/luna/src/memoization.rs) | Type 结果过滤、已准备实参、来源世界重新准入、Eq/Hash 及额外输入判别 | pointer transfer 等旧接线不能成为 comptime 的长期权限；结果缓存与共享代码仍须分开 |
| [meta List](../library/meta/data/list.fl)、[Eq](../library/meta/data/list_equality.fl)、[Hash](../library/meta/data/list_hashing.fl) | 值引用 append、固定快照遍历、逐元素协议和短路顺序 | 通用值改造必须保留真实公开 API 行为，不能用仅在 Rust 构造的列表替代验证 |

既有 list 编码已保留 NF 域，不能再把“所有 List<Type> 都被导出为无域类型向量”当作
现状。仍有待迁移的具体分支不证明应删除全部紧凑存储或现有准入检查。
初始设计只静态核对这些生产入口；后续已接通的共同值域检查与运行证据见第 10 节。

## 2. 一个值通过四道不同的边界

```text
观察实际值与域 -> 接纳成员资格 -> 准备本次传送 -> 编码/恢复
                                  |
                                  +-> 独立判断是否可作缓存输入/结果
```

成员资格回答该值是否满足实际域及其形成契约；传送资格回答接收者是否有合法的依赖、
阶段与 owner；编码回答如何保存已有许可；缓存再回答是否可以省略某次纯计算。
四者不能由 `immutable_copy_work != None`、`has_type` 或一个万能 `is_valid` 代替。
不可变存储可以共享，不意味无需域检查；可复制的内部 Arc 也不授予 runtime Copy。

拟议内部接口用不同结果类型表达已经检查的事项，不允许调用者自行标记完成：

```text
ObservedValue {
  description, actual_witness, origin,
  owner_and_content_version, stable_dependencies
}
AdmittedValue = value + checked_domain + formation_evidence + scope
TransferPlan = receiver_contract + dependency_mapping + ownership_actions
FrozenValue = typed_nodes + root + stable_scope + dependencies + admission_summary
```

证据不代替实际 payload。接纳过程中需要类型命题、布局或已有契约时，通过相应服务
取得有前提的结果；值表示模块不反向拥有整个 elaborator。实际输入未就绪沿观察协议
等待，缺少生产者、违反域和预算耗尽分别返回，不能失败后换成 Any 继续编码。

## 3. 接收契约按用途区分

| 边界 | 可以保留什么 | 不自动获得什么 |
| --- | --- | --- |
| 同一 VM 的普通调用 | 已取得普通值，按参数计划转交受控值引用 | 当前调用结束后的引用逃逸权 |
| 稳定开放定义 | 显式参数、合法 residual、稳定 runtime identity 描述 | 必检义务豁免、活动推断变量或等待者 |
| 已完成 NF 输入 | 按普通 telescope 接纳的类型/值/合法依赖项 | 专用 TypeList 参数槽、NF 主体展开 |
| 局部常量/拥有的闭包快照 | 已准入数据与允许的闭包环境图 | 外部可变存储别名、任意循环图或原工厂重放 |
| CMTT 代码传送 | 类别、ordered Ψ、主体/合法残余、固定依赖及控制接口 | runtime 代码对象、未知上下文视为空、代码主体立即执行 |
| 结果缓存 | 纯性及输入协议允许复用的稳定结果 | 活动 VM 帧、值引用、任意不纯行为跳过 |
| 包 metadata wire | 现行版本明确允许的带类型稳定数据 | callable 工厂执行、代码 handle 或活动存储导入 |

统一服务不等于所有边界接受同一个全集。契约应是有明确准入规则的类别及参数，
不用几十个 allow_* 布尔标志拼出未经定义的组合。新增值种类先说明上述接收方各自的
生产者、消费者和拒绝理由；暂不支持某边界时明确报实现缺口，不把它写成永久语言限制。

## 4. 域驱动的普通值表示

冻结图每个需要独立解释的节点保存实际域或指向受检域的引用；统一的序列/记录编码
可以共享载荷算法，但保留名义身份、元素域、顺序、分支标签及嵌套层次。
Type 是普通 comptime 值的一类实际见证；其内部 sort 仍按 Type 规则检查，不需要把
它伪装成 bool 一样的标量，也不因此为 List<Type> 增设 NF 参数种类。

以一个合法已绑定的 `List<T>` 为例：

1. 按普通调用规则取得该次完整应用及精确域，不从载荷数组反猜它属于 meta.List。
2. 通过受检结构/序列观察取得元素域和表示依据，不能仅识别一个单实参 NF 形状。
3. 依次准入元素；空列表也保留域，不能从没有元素推得任意类型列表相等。
4. 冻结为同一普通序列载体，保留原 NF 头；Type 元素与 bool、nominal、代码等元素
   各按实际见证和接收契约处理。并非每条 wire 都允许所有这些元素。
5. 恢复时重新解释域及其依赖，再取得该目标执行器支持的内部表示。

已有 `Tuple<.Es = Es>` 经普通 ArgumentPlan 供应整个列表，不展开再重收集。
结构视图服务不执行 NF 头来推出等式；用户生成结构、普通列表和 builtin 的集中绑定
共享准入边界，不能让 `tuple_elements` 成为所有值传送的前置条件。

Object 保留有序位置项和具名项、每项实际域及重复名称规则，不在冻结时随意转换成
去重 map。错误载荷保留完整错误身份，optional 保留每层缺失，newtype 保留自己的域。
算术值遵守原有精确值/机器位模式契约；编码不调用用户 Eq 来重新定义 NaN、零或
名义值的内部身份。具体整数范围仍由域与目标事实接纳，不由宿主容器位宽推定。

## 5. 值引用：位置身份不等于内容版本

目标内部位置描述区分：

```text
Place = owner/session + slot generation + projection path + permissions
ReadEvidence = place + observed content version + actual value witness
```

正常赋值改变内容版本，不必改变仍存活的根槽身份。值引用的再次读取要重新定位，
可以看到新内容；不能笼统规定“写过一次，所有 alias 永久失效”。根槽被回收重用、
owner 结束、路径不合法或权限不足时拒绝。读到的一份独立不可变快照仍保持旧内容。

投影路径保存结构依据，不能仅以字段文本或旧 Vec 元素地址定位。RHS 可能调用回调、
替换聚合或改动列表；完成 RHS 后重新核对目标路径、权限及实际域，再准备写回。
旧内容证据因写入失效，意味着不能复用旧准入/代码接口检查，不意味着应忽略合法的
同槽新内容。`let` 接收者的根重绑定限制在转发时保留，合法字段更新按其自己的权限处理。

准备写回的验证与分配在修改 owner 前完成，commit 替换相应值并更新版本。
预算耗尽或新值域错误不安装半个容器。RHS 已经发生的外部行为不因此回滚；不能在
写回失败后自动重求 RHS。对两个实参传入同一值引用时，保持其实际别名关系，不能
为了参数隔离各复制一份再写回。

跨宿主请求的 lens 留在 VmSession；宿主接收稳定输入，响应由同一 session 复核后交付。
位置目标和普通返回槽分别记录，不能在等待期间因 owner 已变就向同号新槽写入。
这不改变普通嵌套方法可以按源规则更新原接收者的行为。

## 6. 图快照与恢复事务

冻结使用私有节点表，按实际 owner/代码对象身份记录已访问节点；不可变数据可结构共享，
具有身份的闭包不能因为环境内容相同就合并。同一闭包被多个字段引用时保留共享关系。
现有 ClosureIdentity 持有 Arc 防止地址复用；该地址只在本次观察中识别对象，不进入 wire。

活动值引用不能借“读取当前内容”被静默转换成闭包捕获快照。是否取独立数据快照由
源操作/参数契约决定；能够检查一张活动捕获图不等于这张图可以逃逸。
循环图必须有合法的拥有及恢复协议，不能因 visited 集合避免了无限遍历就放行。
尚未支持的合法循环图需专门补齐存储/回收设计，不能靠任意工厂执行伪造恢复。

当前 ClosureSnapshot 的 factory 是局部常量生产者从已求值捕获生成的重建函数，
不是把原用户工厂简单再调一遍。迁移要保留这一来源限制；包 metadata 解码不能因此
获得执行 factory 的权限。目标用显式快照节点和恢复动作表达重建，逐步去除传送专用
生成函数，不能在未替换共享身份、依赖和消费者前先删现有路径。

恢复分为：验证格式/预算 → 重绑定稳定声明与 scope → 分配本次私有节点身份 →
接纳依赖域及载荷 → 安装内部引用 → 发布可用根。任何一步失败都不把半图放入接收会话。
允许的内部前向边先预留身份再接线；不采用假成功占位值跳过形成契约。
可变 owner 在不同恢复会话中独立，不能把上一会话的存储作为缓存 payload 直接共享。
不可变节点只在所属世界/作用域及接收证据相容时共享。

FrozenDefinition 可以含合法 residual 和稳定参数；CompletedValue 在某次需要具体值的
接收边界必须真的完成。两者共用节点基础不意味着恢复一个未解义务就是取得一个值。
销毁失败的私有图同样受控，避免拒绝输入后以递归析构耗尽栈；已消耗预算不退还，
必要的内存释放不能因语义步数归零而停止。

## 7. Any 与代码：实际见证绑定到本次值

Any/any 只放宽描述，不改变实际值类别、阶段或依赖。接收者需要代码时，按此次
ObservedValue 恢复 expr/pattern/arm 类别、接口、主体和固定依赖，不按目标类型合成代码。
代码隐藏在聚合、列表或 Object 内，检查也须递归遵守同一规则。

例如先从可变容器位置读到 expr，并完成接口检查；随后该位置被写为 pattern。
继续使用已经取得的不可变 expr 快照可以保持原证据，重新读取同一下标则必须重新检查。
“同路径”或“同 Any 标注”不是“同一份代码”。固定外部对象不因 any 而变成自由端口。

稳定代码保留 ordered runtime Ψ、结果/输入类型、成功输出绑定、控制接口、来源与
合法残余义务。pattern/arm 输出绑定在成功接入范围产生，不能按 expr 的输入替换方向处理。
rank-1、两阶段和每段 quote 至多一个 splice 的限制继续成立；冻结不提升 rank 或把
代码参数塞入 runtime-only 上下文。quote fn 普通实参单次求值与接入后的资源检查另行保留。

代码表示共享不证明两个程序等价，也不授予生成程序重复消费仿射资源的许可。
当前 CodeValue 的 Arc 相等只作内部表示比较；不能用它实现源 Eq 或 PTC 的代码等价原子。

## 8. 三种相等与预算分账

| 比较 | 用途 | 必须保留的限制 |
| --- | --- | --- |
| 节点/表示相等 | intern、去重不可变载荷、完整性核验 | 不取代类型相等或用户协议，不以地址跨会话恢复 |
| 类型及 NF 身份关系 | Type 接纳、替换及类型命题 | 保留头、普通输入及作用域，不从 factory 结果反推单射 |
| 源 comptime Eq/Hash | 获准的 query 输入键 | 选定阶段/impl/world；碰撞继续 Eq；保证复用计算不能区分该等价类 |

现有 ExactInputs 先调用源协议，再用额外表示判别保证查询依赖精确，不能误称它已经
以 Rust PartialEq 替代用户 Eq。重构可以改进判别粒度，但必须证明结果复用相容性。
没有该证明时保留更严格判别或不缓存，不能为提升命中率复用会区分两份输入的计算。

当前仅输出 Type 的 invocation 进入结果缓存；纯性、成本及可接纳性仍另查。
非 Type 路径不应为了确定“不缓存”先运行昂贵 Eq/Hash。键协议自身执行不递归查询
同一个待形成键；错误/预算不足不形成成功条目。不可缓存值仍可按合法普通调用执行，
不要把优化所需协议变成所有 comptime 参数的新增要求。

预算区分：访问节点/边、域检查、实际复制字节、逻辑展开规模、编码大小及协议执行。
共享树的浅克隆按实际工作计费，但展平 wire 或深比较仍按其真实规模计费。
缓存的容器摘要只在不可变 payload 和对应版本有效时减少扫描；不能免除新接收域、
剩余深度和世界准入。整数、字符串、序列长度的算术本身也要防溢出。

## 9. 生产迁移与验收

先把普通调用、NF、staged 参数和返回交付的域检查接到一个 AdmittedValue 边界；
编码器暂作适配器，仍接受各自受限契约。再迁移局部快照/代码值、恢复和缓存消费者。
新服务真正接管准备和失败路径后，才移除 normal_terms/normal_values/constants 中
重复的域决策；不要先删验证、再靠成功编码推测合法性。

首组验收走真实 sysroot：普通用户 NF 接收完整 List 值、Tuple 具名供应、
meta.List.append_all 自追加、foreach 中回调修改原容器、List 的元素 Eq 短路与 Hash 次数。
同时覆盖闭包共享快照和 Any 中代码的真实类别，不能只比较两个 Term round trip。

拒绝/恢复案例包括：同 payload 不同列表域、空列表域丢失、错世界同号声明、依赖域替换
失败、活动引用隐藏在聚合、容器更新后复用旧代码证据、快照后修改原 owner、预算耗尽
后同会话合法调用、闭包重建不重放原工厂、错误/optional 层次丢失及未完成代码义务。
所有案例都要比较操作次数、值/类型与相关责任，而非只检查输出字符串。

上表是完整迁移的验收范围，不等于目前全部通过。具体稳定节点 Rust 接口、持久 codec
及一般循环快照存储仍须按本协议细化；第 10 节区分当前共同准入路径与其余未完成部分。


## 10. 已接通的共同值域检查

`nir::value_admission` 已接管不可变值描述的递归成员检查。普通 callable 子句、NF 输入、
名义参数及 trait 中的符号聚合值使用同一个递归检查器；各自仍负责参数路由、依赖域
替换和完整形成义务。旧 NF 聚合检查循环与名义参数递归检查器已移除。

`DomainQueries` 只向接收世界查询已注册布局、集合元素域、符号值域依据和 callable
签名；服务不拥有 elaborator，不执行源码，不凭 NF 头的拼写或载荷形状授予成员资格。
当前布局注册仍使用既有的 Tuple/List 元数据，尚未完成 Role 的整体退役。

普通 callable 输入不再按 Object/list/callable 分类伪造一个单参数 `Head`。它在原有
完整 telescope 替换完成后直接交付实参与确定的域，包括嵌套聚合中的 callable。
输入失败仍保留外层 slot、声明身份和 value 阶段，且发生在实例发布与函数主体执行前。
例如 optional 内的函数值也通过接收世界取得签名，不能因外层 optional 不属于旧分支
白名单而拒绝，也不能只凭内部 declaration 数字接受。

VM 的签名查询移到 `callable_domains.rs`，typed 结果区分实际缺失、域错误和预算耗尽；
普通值准入直接消费它。其他既有 `Option` 观察入口暂作适配：真实预算错误会置空剩余
预算，不能把“剩余预算非零但不足以复制整个描述”误判成不存在的声明。NIR 的
`ApplicationError` 同样新增独立 Budget 结果。NF 的旧外部 Option/bool 回调接口仍在，
不能据此声称全部查询服务都已迁移为完整证据类型。

共同服务保留 Type/sort/显式 frame 检查、准确 optional/error 分支、整数目标范围、
字段域以及接收端 callable 身份与签名。记录成员按字段名取得各自的域；VM 恢复时按
声明顺序重建内部字段序列，拒绝重名或缺字段。该恢复动作不改 NF 头、声明身份或类型
等式，也不是调用源语言的相等协议。结构预算与域错误分开，不缓存不完整结论。

当前返回值仍是成员检查成功/失败，不是第 2 节完整的 AdmittedValue/TransferPlan。
尚未统一 LiveValue、闭包图快照、代码值/Any 传送及持久 codec；NormalFormArgument 的
上下文表达式与参数准备路径也仍待 ArgumentPlan 迁移。本阶段没有将这些缺口包装成
一个万能的“已验证值”。


当前实际验收从 `/home/fin/luna` 通过 `development/check.py --timeout 900 -- test`
执行，保留工作区原有资源分析修改，未跑全 workspace/native：

| 范围 | 结果与证据 |
| --- | --- |
| NIR nominal_parameters、normal_inputs、trait_parameters、value_admission | 72 项通过，含跨入口记录域、嵌套 callable/集合、未知世界、frame 与预算恢复；`/tmp/luna-tools-lp15nyc2/result.json` |
| VM abstract_values、function_input_diagnostics、nominal_types、runtime_function_identities、type_list_inputs | 120 项通过；`/tmp/luna-tools-xtlb4gc8/result.json` |
| VM signature 单元组 | 检查资源限制、保留签名 core 借用与恢复；`/tmp/luna-tools-7f91pcbk/result.json` |
| luna comptime_clause_inputs、function_template_local_definitions | 46、22 项通过；后者 2 项 ignored；真实六包的固定具名 `meta.List<bool>` 值同时用于普通子句与用户 NF；`/tmp/luna-tools-lt6asiz5/result.json` |
| luna dependent_list_collectors、normal_form_domains、normal_form_methods | 1、3、6 项通过；`/tmp/luna-tools-grmad_md/result.json` |
| luna nominal_value_parameters | 306 项通过，1 项既有失败，见下述对照；该组及混合命令整体退出 101，不写作全通过 |

`memoized_value_indexed_keys_keep_declaration_and_value_arguments` 在
`memoization::Invocation::prepare` 请求 dispatch 时仍报
`requested dispatch function still requires ordinary invocation input staging`。
将本次涉及的 8 个生产源文件暂时还原为 HEAD `962a03b4`、保留同一工作区资源修改后，
单独运行此例仍在同一断言、同一消息处失败，退出 101：
`/tmp/luna-tools-biw368sn/result.json`。对照备份、限定文件列表及恢复结果保存在
`/tmp/luna-admission-baseline-wr76d3vj`；对照后已逐字节恢复本次源码。这项失败尚未修复，
不能以共有值域检查通过推断 invocation staging 或结果缓存准备已经完成。

真实六包尝试还暴露普通函数子句收集器的既有限制。进一步核对 `bcd5127e`
中的 `ensure_signature`：旧条件是 `!is_normal_form_head || list_element_type(...).is_none()`，
它拒绝所有普通函数的 variadic 子句，而非只接受 `List<Type>`；前一版记录对根因的
描述不准确。`where ...values: meta.List<bool>` 在值准入之前就被拒绝，旧现场为
`/tmp/luna-tools-i2jbbvo7/result.json`。第 11 节说明后续迁移的实际接通范围。
泛型 Copy 默认值的 P3/资源证据缺口仍独立存在。

源码验收只要求同头同参的 NF 相等；不同实参不能自动证明 opaque NF 结果不等，
不通过放宽 PTC 的非单射边界来验证值域传送。当前源码仍保留各实参的实际描述与域。


## 11. 普通子句输入与列表收集

收集参数仍是一个普通 comptime 值输入。声明时将其 ID、完整域和声明位置登记进
普通函数的输入 telescope；不会把列表拆成多个函数泛型槽，也不创建伪造 NF 头。
现阶段使用已经绑定的集合协议 `meta.List<T>`，元素域不限定为 Type；其他用户收集
协议仍待统一 ArgumentPlan 支持，不能把现有列表接线提升为语言唯一允许的收集域。

`program/clause_inputs.rs` 接管原先位于 `type_defaults.rs` 的普通输入准备，NF 与普通
子句共用收集器识别、固定前缀计数和字面前缀展开。值准备仍与 NF 的上下文参数树不同；
这一步没有宣称两条参数计划已完全合一。

普通 comptime callable 的已知签名路径现在保留以下关系：

- 每个显式元素、列表展开或完整供应在源码位置求值并绑定一次。默认计算仍在显式
  输入之后进行，读取本次已绑定输入；继承输入仍由词法依赖计划供应。
- `.values = expression` 直接供应完整列表。它与逐项收集或展开互斥，重复名称及
  既收集又整体供应均拒绝；展开空列表也属于一次显式收集，并保留其生产者行为。
- 固定前缀使用 tuple 展开，尾部使用列表展开，不把空 tuple 或 Type tuple 当作列表。
  省略尾部产生该声明域下的空列表。列表元素逐项接受域检查，最终列表接受完整域检查。
- 保存函数值会保存已取得的子句列表快照，之后修改来源变量不能改变那个实例。
  失败 invocation 后，其他实例仍可形成与执行；提前退出阻止后续生产者执行。
- 多段列表只在所有生产者已绑定后组装，使用已有列表值及 `ListConcat` 节点；组装树
  保持平衡，避免连续展开产生线性深度的 NIR 和反复复制全部前缀。它不重排生产者。

`c851ac9a` 的阶段边界是：普通 runtime callable 的 required 子句供应仍走旧
`trait_clause_arguments_with_prefix` 路径，尚未共享这份准备计划；签名本身依赖值输入、
或元素域尚需后面的 Type 供应才能取得时，也还需要统一的 ContextualExpr/ArgumentPlan。
尤其不能通过提前调用后面的 Type 生产函数，为先出现的列表字面量制造上下文。
这些缺口保留期望成功的独立回归，不修改语言规则来追认实现限制。runtime callable
收集输入的后续接线见第 12 节；此处记录前一阶段的证据边界。

本阶段在 `/home/fin/luna` 使用 `development/check.py --timeout 900 -- test -p luna`
检查 `ordinary_clause_collectors`、`function_template_local_definitions`、
`comptime_clause_inputs`、`dependent_list_collectors`、`nominal_value_parameters`、
`normal_form_domains`、`normal_form_methods`，并显式 `--skip
memoized_value_indexed_keys_keep_declaration_and_value_arguments`：394 项通过、3 项 ignored、
1 项既有失败被筛除，命令退出 0，证据 `/tmp/luna-tools-_gt2wxx9/result.json`。
名义输入的既有失败仍使用第 10 节已做过的基线对照，不重复伪装成新结果。

新收集器组通过真实 sysroot 验证 10 项，包括 128 段列表展开；其两项 ignored 单独
用 `--test ordinary_clause_collectors -- --ignored --nocapture` 执行，退出 101：runtime
实例路径仍把首个 bool 当作完整列表输入，先供应的列表字面量仍缺元素上下文。
现场 `/tmp/luna-tools-mc58vqey/result.json` 保留两种不同失败，不把 ignored 当作验收。
其余一项 ignored 是既有抽象 Copy 默认值问题。已修改 Rust 文件的 rustfmt check
退出 0，证据 `/tmp/luna-tools-vldv5jur/result.json`；未运行全 workspace 或 native 检查。


## 12. required 与 invocation 共享 callable 参数包

`prepare_callable_clause_packet` 从原 `comptime_function_instance` 中抽出。它处理前缀、
显式输入、默认值、声明域与形成要求，返回内部 tuple 参数包及签名替换证据；它不决定
执行时间，也不授予 callee 的执行资格。

已知 comptime callable 仍把同一参数包留给 invocation。普通函数的 required 收集输入
则在 required 检查上下文中构建并执行参数包，取得不可变子句值后走已有
`function_instance` 形成与准入。直接函数、静态方法以及包含 staged const 的 open
函数均接到这个入口；方法仍保留原 trait 来源。NF 头的身份规则没有改动。

这里没有把 runtime 函数体塞进 comptime VM，也没有放宽 `ComptimeFunctionInstance`
的阶段契约。新增的构建回调进入 required 原有的隔离上下文之后才检查生产者，避免把
调用者的 runtime 局部值、return 及 loop 上下文当作编译期输入的权限。已有检查入口和
回调入口共用捕获准备、依赖快照、预算、失败恢复与结果发布。

required 的活动查询键从单独源码 LocalId 改为 `(LocalId, RootKind)`，区分源表达式
与 callable 输入包计算。嵌套 `const saved = function<...>` 在同一源码节点下形成输入包
是真实的不同查询，不能据此报告循环；相同查询的递归检测与深度限额继续保留。
在增加种类之前，真实 sysroot 正例于 `/tmp/luna-tools-9x6lkqnq/result.json` 退出 101，
报告 cyclic required dependency；修正后同一用例于 `/tmp/luna-tools-o7cg5qwq/result.json`
通过。它只是现有 required 生命周期的局部修正，尚不是完整世界版本、依赖指纹和
事务状态查询系统。

测试保留 runtime 特化实际读取列表、空尾部、完整供应、展开、依赖元素域、默认值、
方法及常量函数值形成；拒绝 runtime 局部读、调用者控制出口以及在 comptime 调用
runtime 函数。动态 trait 的一次性求值测试原先使用 comptime 指针，违背已确认契约；
现改成 struct 的值引用 self 方法更新字段，仍断言子句生产者执行一次、普通实参随后
读取更新值，没有更改预期结果或赋予 comptime 指针资格。

该阶段尚未完成的部分包括：先供应字面量而元素域依赖后来的 Type 输入、通用自定义收集
协议、值相关 callable 签名，以及 trait/NF/普通输入的完整 ContextualExpr/ArgumentPlan
合流。非收集器 required 参数目前仍有旧 trait 路由，不能把本次 packet 复用称为整个
输入系统已经统一。任意 live comptime 环境中的 runtime callable 生成与传送也没有
因静态 required 输入路径通过而自动获得完整证明。

验收在 `/home/fin/luna` 使用 `development/check.py --timeout 900 -- test -p luna`：
`dynamic_trait_clauses` 10 项、`function_template_local_definitions` 22 项、
`nominal_value_parameters` 306 项、`open_functions` 390 项、
`ordinary_clause_collectors` 18 项通过，命令退出 0，证据
`/tmp/luna-tools-vptmv1b_/result.json`。其中两个 ignored 分别为抽象 Copy 默认值及
后置 Type 供应下的列表字面量；既有 memoization staging 失败继续按名称显式跳过，
未宣称修复。此前同版生产源码的 `clause_staged_constants` 2 项与
`comptime_clause_inputs` 46 项已通过；其混合命令随后因旧指针 fixture 失败而整体
退出 101，证据 `/tmp/luna-tools-_bwxuxhw/result.json`，不能将整条命令写作成功。
迁移该 fixture 后重新执行上述动态 trait 组通过。修改文件的 rustfmt check 与
`git diff --check` 均通过，格式证据 `/tmp/luna-tools-iz3x2t1_/result.json`；未运行全
workspace、性能基准或 native 验收。


## 13. 域检查顺序与源码求值顺序分离

普通子句准备现在为每个源码输入保留独立 preparation 位置。接收域仍引用未提供的
Type 参数时，`PendingClauseInput` 暂存已解析的 HIR 引用、目标槽、供应种类和源码
序号；后面的显式 Type 输入提供静态身份后，只检查一次待定输入。生成的绑定与计算
仍按原始源码位置组合，最后才收集列表并处理声明默认值。

这不是新增一套字面量/分支解释器。列表、嵌套列表、数字、if、match、块及控制出口
仍由普通 `expr` 检查并生成已有 NIR。pending 状态只在本次调用者检查环境中有效，
保留已解析绑定，不是可以跨调用缓存或发布的闭合代码对象；`PendingClauseInput`
和 `PreparedClauseInput` 均提供 S-expression Display。完整 ContextualExpr 所需的
持久词法环境、等待理由与可恢复查询还没有由这些局部状态代替。

Type 证据复用已有的静态值观察，因而后置的 `meta.List<bool>`、用户 NF 等也可以
提供接收域。观察不调用 Type 生产函数、不展开 NF 头；即使取得静态身份，原生产者
仍留在自己的 preparation 位置，不能因替换证据可用而删除执行或形成检查。
只有声明域确为 Type 的参数才登记 Type 绑定。收集器里的 Type 元素仍属于那个
普通列表值，不能把收集参数自身改成 Type sort。

这一步同时用于普通 comptime callable 的 invocation 参数包和 runtime callable 的
required 参数包；固定值子句也复用同一准备服务。覆盖范围包括静态后置 Type 的
字面量上下文、空列表、嵌套域、数值范围、分支、源码求值顺序、可变 callable 读取、
提前返回，以及 NF 头身份。旧的
`clause_collectors_contextualize_literal_before_its_element_type_supply` 已取消 ignored。

仍需区别两种“还不知道”：静态显式 Type 尚未检查，可以延后本次输入检查；Type
只有执行生产函数才能得到，则不能为了先检查列表而提前执行它。后者保留
`pending_clause_domains_can_wait_for_a_type_producer` 为期望成功的 ignored 用例，
等待 comptime invocation 的域观察与通用参数计划。NF 的旧 contextual 参数树尚未
退役；同一参数计划覆盖 NF、trait 与普通函数仍是迁移目标，而非本节已完成事项。

本节在 `/home/fin/luna` 使用 `development/check.py --timeout 900 -- test -p luna`
执行收集器、comptime 子句、名义输入、模板局部定义、动态 trait、NF 域/方法及依赖列表
收集八组：420 项通过、2 项 ignored，既有 memoization staging 失败仍显式跳过，
命令退出 0；证据 `/tmp/luna-tools-v8cnvlwl/result.json`。随后加强 opaque 头用例，
令函数体比较收到的 T 与 `Identity<u8>`，单独执行通过：
`/tmp/luna-tools-kr20npyt/result.json`，不是仅以空列表可构造来推断头身份正确。
两个 ignored 为既有抽象 Copy 默认值与新的 invocation Type 生产者用例；后者显式执行
仍以缺少列表元素上下文失败，退出 101，证据 `/tmp/luna-tools-byp6h53s/result.json`。
格式检查退出 0：`/tmp/luna-tools-kj3a8iy5/result.json`；未跑全 workspace/native 或性能
基准。这里没有把静态检查重排描述为任意动态域均已支持。


## 14. 静态观察复制复用完整结构预算

静态观察器原有的 `clone_known_term` 自行遍历 Term，并以通配分支忽略未列出的结构。
它遗漏 ErrorResult、ClosureContextValue、ContextualClosure、FunctionValue、
ClosureSnapshot 和 MethodValue 的子项，导致这些结构内的载荷与深度无法计入复制预算。
现删除该遍历，改由 `nir::copy_budget::clone_term` 在公共完整结构预检成功后才复制。
该入口复用 `visit_children` 及已有载荷计费，不另建一份 Term 边列表。

这项服务只保证复制资源受限。它不执行 NF 头，不替换开放绑定，不证明 scope、形成
或值域成员资格；接收者仍承担原有准入责任。即使剩余 fuel 非零，下一项载荷、宽度
或深度也可能已超过预算，因而调用者必须检查 Result，不能从计数器猜测结果。

静态观察的可选适配器仍可以放弃提供证据；必需的类型别名展开则直接消费带类型的
复制结果，将 Steps 与 Depth 分别报告为预算耗尽与展开深度超限，并记回已消耗工作。
这次没有将整个 `known_normal_input` 改成带类型的观察结果。未知值、未支持观察、
算术无效以及不同资源限制在该观察器其余分支中的分类仍须统一；不能把本次复制边界
修正描述为通用可恢复观察协议已完成，更不能用 fuel 归零作为这项迁移的替代。

回归覆盖所有上述遗漏结构，其中 MethodValue 分别覆盖 subject、interface arguments
和 method arguments；检查隐藏载荷、过深嵌套、非零剩余预算下的失败及新预算下恢复。
另保留精确十进制载荷、结构复制成本及深度边界检查，验证开放 opaque 身份原样保存，
没有因复制成功而获得作用域证明。

本节在 `/home/fin/luna` 使用 `development/check.py --timeout 900 -- test` 验证：
`-p nir --test copy_budget` 19 项通过，证据 `/tmp/luna-tools-j0zcj4mv/result.json`；
`-p elaboration --lib program::normal::` 13 项通过，证据
`/tmp/luna-tools-wut3z1t9/result.json`；`-p luna --test ordinary_clause_collectors
--test normal_form_domains --test comptime_clause_inputs` 共 75 项通过、1 项既有 ignored，
证据 `/tmp/luna-tools-lcd010m3/result.json`，上述命令均退出 0。ignored 仍为需要执行
Type 生产者才能完成接收域的用例，本节没有解决它。

另运行 `-p luna --test elaboration alias`：12 项通过、2 项失败，退出 101，证据
`/tmp/luna-tools-232hgxp2/result.json`。两个失败分别是
`generic_aliases_check_arity_scope_cycles_and_target_types` 与
`typealias_defaults_reject_invalid_unused_defaults_and_missing_parameters`；都期望
`unresolved type equality`，实际拒绝诊断为 `type mismatch`。以 `828c1fa7` 的三个
生产文件替换本阶段变更、保留其余工作区后执行相同检查，结果和失败诊断一致，基线
证据 `/tmp/luna-tools-j6an4vhn/result.json` 退出 101；随后三个文件逐字节恢复。
没有修改测试预期，也不把这两项既有失败记为本次通过。原工作区受保护修改保持不变。
修改文件 rustfmt check 退出 0，证据 `/tmp/luna-tools-wp09l2u0/result.json`；
`git diff --check` 通过。未执行全 workspace、native 或性能检查。
