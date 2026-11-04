# Comptime 值的准入、快照与恢复

状态：内部设计与分阶段实现记录。第 10 节起说明实际接线与验证边界。承接[值与执行总览](values-staging-and-publication.md)，
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
`/tmp/luna-admission-baseline-wr76d3vj`；对照后已逐字节恢复本次源码。该阶段未修复
此失败；第 19 节说明后续 invocation 准备入口的实际修复。共有值域检查本身不能证明
invocation staging 或结果缓存准备已经完成。

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


## 15. Pattern 观察保留失败类别，必需选择消费实际原因

`PatternInspector` 与标量比较不再以 `Option<bool>` 合并未知证据和失败。正常的
`Ok(false)` 才允许尝试下一候选；`Unavailable`、Steps、Depth、Size 及 Invalid
立即中止本次选择，不能据此令 fallback 获选。`Failure` 提供 S-expression Display，
资源分类直接来自运算／复制服务的返回值，不从剩余 fuel 推断。

pattern 绑定使用公共受限 Term 复制，因而复制载荷超预算时即使 fuel 仍非零也保留
Steps；过深结构保留 Depth。Or 的候选环境仍独立复制，失败不提交局部绑定；guard
与 Then 的计算仍通过原有求值回调按顺序执行，未知或失败不重放后续候选。
畸形 NF 参数数、无效字面量／BitVec 表示保留非法证据结果，不能冒充正常不匹配。

字面量读取移至共享的 `known_scalars::literal`。静态表达式观察、字面量 pattern 及
inline 的字面量计算复用它；字面量 pattern 不再复制 literal 制造临时 Expr，也不再
经一个能够 invocation 的回调解析。节点预算由外层负责，载荷预算由读取服务负责。
IEEE unordered 比较仍为 false；unsupported 的半精度观察与缺接收域的 pending
literal 仍不提供证据，没有借此变更语言的数值规则。

必需的 `select_inline_pattern` 消费带类型的结果，分别报告证据不可得、步数耗尽、
深度超限、值大小超限和非法证据；真实 invocation 错误继续保留原诊断和位置。
可选静态表达式观察的上层适配器仍可放弃失败证据。本节首先完成 pattern 自身、
标量服务与必需 inline 选择的接线；表达式内部递归及 guard/Then 的后续迁移见第 16 节。
NF contextual 参数树及整个通用观察协议仍待迁移，不能仅靠更名宣称完成。

验证在 `/home/fin/luna` 使用 `development/check.py --timeout 900 -- test`：
`-p elaboration --lib program::normal::` 17 项通过，证据
`/tmp/luna-tools-vbca9b12/result.json`；`-p luna --test inline_control pattern` 23 项
通过，证据 `/tmp/luna-tools-16a150q6/result.json`；普通收集器、NF 域和 comptime
子句三组 75 项通过、1 项既有 ignored，证据 `/tmp/luna-tools-ix4buvcb/result.json`。
新增 `-p luna --test inline_pattern_observation` 从源码解析所得同一 HIR 寻找必需
pattern 的预算失败边界，确认生产诊断，并用新预算重试及执行正确分支；1 项通过，
证据 `/tmp/luna-tools-4tx2rw98/result.json`。上述命令均退出 0；没有扩大为全
workspace、native 或性能验收，ignored 的动态 Type 生产者未由本次解决。
修改 Rust 文件的 rustfmt check 退出 0，证据 `/tmp/luna-tools-mjo4anyo/result.json`；
`git diff --check` 通过，原有工作区修改未改动，正式 docs 未修改。


## 16. 表达式递归观察传递失败，域需求决定诊断责任

`observe_normal_input_in` 是原静态观察器的迁移，不是新增解释器。它与 NF 实参
观察、收集值、pattern 回调、算术、复制和类型相等观察统一返回 Result；内部递归
不再经过 `known_normal_input_in` 的 Option 适配器。表达式仍使用同一 NIR，不运行
函数、不展开 NF 头，也不把观察到的静态身份视为删除原生产者的许可。

If、布尔短路、Match、块中的不可变绑定仍只观察实际选中的路径；未知条件、需要
invocation 的表达式、未支持的结构及无法证明的类型关系返回 Unavailable。错误不
变成 false，失败不授权下一分支胜出。列表越界、运算非法、集合编码／替换无效与
资源不足保留各自结果；某些结构形状仍仅能判断“无法提供证据”，本次没有把所有
不支持的形状都升级为语言错误。

复制与运算可以分别报告 Steps、Depth、Size。共享结构／替换 API 本身只提供汇总
Budget 时保留 Structure；PTC 只处理 Type 命题，其 BudgetExhausted 保留 Proof。
不凭剩余 fuel 猜测是哪种限制，也不把 PTC 未证实相等解释为已证实不等。对相等与
不等分别查询证明，预算／非法命题失败不被第二次查询掩盖。

普通子句准备保存 Type 观察失败，但不立即拒绝调用。待定输入若无需该证据即可
完成检查，仍按原路径继续；若其检查明确返回 needs_context，且所缺域涉及那个
失败的 Type 观察，则报告原上下文错误与观察原因，且不再标记为可继续推断的错误。
无关的语义错误保持原诊断。仅有 Unavailable 的 Type 生产者仍等待后续 invocation
机制，本次不会为了获得上下文而提前执行它。

NF 的必需检查现已消费带类型结果：类型标注、收集值准备、先前参数对后续域的
替换、computed packet 的字段观察、默认／显式参数、required NF 求值计划，以及
名义类型的准备绑定和默认值依赖检查，均只把 Unavailable 转成可延后的 None。
Steps、Depth、Size、Structure、Proof 与 Invalid 直接形成带源码位置的
`static value observation failed` 诊断，不再降格成未知后尝试 invocation 或生成
更弱的义务。每次观察无论成功、未知还是失败，都记回同一 required 预算；收集器
组装也保留逐项读取、冻结和结构检查的真实失败。

这一边界不把静态观察升级为一般求值。库 callable 等需要执行的表达式仍可以
Unavailable，并保留实际 invocation 的错误、短路和执行次数。观察到静态身份也
不删除原实参生产者。`known_normal_input`／`known_normal_input_in` 仍供其他可选
证据消费者使用，CMTT 的 `known_collected_list_in` 仍是明确保留的 Option 适配器，
其底层组装已经复用带类型结果；这些消费者还需按自身需求迁移。NF contextual
参数树和动态 Type 生产者问题仍未整体退役。

本节在 `/home/fin/luna` 用 `development/check.py --timeout 900 -- check -p elaboration`
检查通过，证据 `/tmp/luna-tools-d_vlbx7n/result.json`。最终 `test -p elaboration
--lib program::normal::` 19 项通过，证据 `/tmp/luna-tools-vd4y8fe6/result.json`；
类型相等用例区分复制不足、证明预算不足和正常证明，集合用例验证各类失败停止读取
后续输入并允许新尝试恢复。

`test -p luna --test inline_pattern_observation` 2 项通过，证据
`/tmp/luna-tools-75uqxs87/result.json`。新增用例使用用户定义 opaque 头和类型别名，
令显式 optional 参数依赖后置 Type，验证复制失败不会仅落成缺少上下文；同一份
解析后的 HIR 在正常预算下重试，并执行正确结果。低预算探测不固定前序准备成本；
直接 NF 构造可能先耗尽在其他准备步骤，所以该用例使用已形成的别名 Type 来覆盖
复制观察的实际边界。临时诊断输出已删除，生产源码恢复并完成后续检查。

普通子句、comptime 子句、名义参数、模板局部定义、动态 trait、依赖列表收集、NF
域和 NF 方法八组共 420 项通过、2 项既有 ignored、1 项既有失败显式跳过，命令退出
0，证据 `/tmp/luna-tools-xfhdt1g9/result.json`。跳过项仍是
`memoized_value_indexed_keys_keep_declaration_and_value_arguments` 的 staging 缺口；
两个 ignored 仍为抽象 Copy 默认值与 invocation Type 生产者，不声称本次修复。
`test -p luna --test inline_control pattern` 23 项通过，证据
`/tmp/luna-tools-f75ga0fu/result.json`。修改文件 rustfmt check 退出 0，证据
`/tmp/luna-tools-f028_ogy/result.json`；`git diff --check` 通过。未运行全 workspace、
native 或性能基准，正式 docs 与原有工作区改动保持不变。

NF 必需观察接线的验证在 `/home/fin/luna` 完成：`development/check.py --timeout
900 -- test -p elaboration --lib program::normal::` 21 项通过，退出 0，证据
`/tmp/luna-tools-podnv8f6/result.json`。新增低预算源级用例保留同一份已解析 HIR，
检查直接／分支 NF 输入的观察失败，再以充足预算重新 elaboration 并执行。
真实 sysroot 用例以 `meta.div_trunc` 验证库调用仍等待 invocation、除零保留实际
错误类别、死分支不执行，且失败后的同一 VM 可以继续执行；覆盖两种 VM。

13 组受影响的 luna 集成检查（完整命令见
`/tmp/luna-tools-s8tgc9g2/result.json`）最初 558 项通过、1 项新增测试断言失败，
退出 101；该断言误用小写错误文本，现改为检查实际 `ErrorKind::DivisionByZero`。
随后 `-p luna --test normal_input_plans --test open_functions --no-fail-fast` 的
NF 输入计划 15 项全通过，开放函数 389 项通过、1 项既有失败，合并命令退出 101，
证据 `/tmp/luna-tools-7sc2llhb/result.json`。其余 12 组无后续语义改动，连同复验
共验证 559 项；2 项既有 ignored 和 memoized key 的 1 项显式 skip 仍保留。
展开组 92 项通过，CMTT 类型输入 24 项通过，不据此声称可选观察消费者全部迁移。
开放函数唯一失败是第 17 节说明的定义侧机器算术证明缺口；忽略线程位置和通用
回溯提示后，失败内容与 `/tmp/luna-tools-zv3lbekk/result.json` 的基线逐字节一致。

修改文件的 rustfmt check 退出 0，证据 `/tmp/luna-tools-6cnu96ee/result.json`；
原有压缩排版的 type_spreads.rs 只局部整理本次函数，未全文件重排。
`git diff --check` 通过；原有受保护文件的哈希未变。提交清单、源码哈希和基线
比较保存于 `/tmp/luna-observation-checks-gyfdzef6`，未执行全 workspace、native
或性能基准。

## 17. NF 输入统一准备，检查顺序与求值位置分离

`normal_type_expression` 的固定参数、逐项收集、具名整包参数和展开输入均进入
`planned_normal_expression`。原 `collected_normal_expression` 已删除，不再根据
有无默认值或是否纯 positional 切换整套输入检查。NF 类型标注也通过
`normal_type_annotation` 消费这份计划：先取得静态证据，无法取得时才进入所需的
comptime 求值。原标注中的纯 Type／可选子句分流、独立 positional 循环及专用
`normal_collection_value_arguments` 已删除；默认值、整包参数与展开不再因出现在
类型位置而改用另一套绑定规则。重复具名输入统一报告 duplicate normal-form argument。
开放函数的签名／表达式描述现在与具体准备共用槽位路由；这不等于所有 NF
消费者或完整 ArgumentPlan 已统一。

计划先完成路由。收集元素和列表展开保留解析后的 HIR 引用、源码序号、前置固定
输入数量及 spread 标记，随后在同一个检查环境中处理。它们不是另一棵可执行表达式
树。域依赖可以查看声明上较早、源码中较晚的已路由实参；`normal_input_domain`
将已检查产生器留在原槽，仅观察可用静态证据，不提前执行 Type 产生器。相同收集域
在路由完成后只观察一次，不对每个元素重复检查 telescope；观察共用 required 预算。

源码求值位置分成两个明确情况：

- 连续的收集后缀留在最终集合表达式中，按从左到右的叶序求值。后缀的定义依据源码
  输入序号；任何固定、具名或 packet 输入都会结束前一段，不能只比较产生的字段数量。
- 后缀以前的已检查产生器在原位置准备并绑定结果，包括空展开。preparation 按源码
  输入分组，最终按源码顺序合并；两个产生器即使拥有相同 `before` 位置也不会倒序。
  读取可变值保留当时的快照，后来的赋值不反向改变早先实参。

因此，后置的具名静态 Type 既能约束整包列表，也能约束其前面的分支、列表展开和
逐项字面量。域已就绪的收集值通过普通子句也使用的 `clause_collection_expression`
形成普通 NIR，不再生成 NF 专用 List 树。共享组装器保持域检查、元素与展开的叶序，
使用平衡 ListConcat 结构，避免多次展开形成线性深度和反复复制整个前缀。组装器也
传递 Never：元素或展开提前返回时，整体不伪装成正常返回的列表。已知输入形成 NF
应用仍保留 opaque 头，meta.List 与 Tuple 没有获得额外的参数类别或权限。

静态观察器同时消费普通 ListConcat，按叶序读取、核对声明域并共用受预算的列表
读写服务。Unknown、资源失败和非法值保持各自结果；左侧失败不读取右侧，不执行
函数产生器。这个消费者不能遗漏，否则共享组装器虽能在 VM 中执行，却可能把原本
定义时可判定的错误 formation 延迟到 invocation。实际拒绝例使用未调用的函数和
含多个静态展开的 Type 命题，验证错误仍在定义检查时报告。


动态域下的连续收集后缀仍可保留 contextual 分支、Match 和块。这些输入位于同一
后缀，交给原计划状态机和 NF 参数解释器执行，不会跨过后续具名实参移动产生器。
此前 defaulted 路径拒绝的此类后缀现在与普通 positional 收集走同一入口。动态域下
夹在其他实参之前的结构化 contextual 输入仍需要可恢复 preparation；当前没有这个
通用机制时仍报告缺少上下文，不能把整个输入移到末尾。无产生器效果的 contextual
字面量保留既有延迟接纳行为，不以此声称接纳失败的时机已全面统一。

签名检查期间，普通 comptime 值的已检查前缀通过 `normal_input_prefixes` 提供实际
域和声明身份；表达式使用抽象 ComptimeValue，不伪造 invocation Local，也不把前缀
发布为完整 callable 元数据。缺少 invocation 值的签名读取说明阶段原因，不放行
runtime 输入。函数类型的具名 telescope 同样通过 `callable_type_input_expression` 为
普通表达式和类型入口提供抽象 Type／ComptimeValue，保留声明身份与实际域；普通
runtime 参数不能因此成为 comptime 值。该入口的复制共用预算，离开 telescope 后仍
恢复原绑定环境，不把临时 rigid 输入作为 invocation Local 发布。

这次统一暴露了原表达式路径的 binder 缺口：构造 `Identity<fn(T) -> T>` 时，T 属于
外层函数的输入，内层函数类型新增一层 binder，不能直接复用深度 0 的引用。
`nir::callable_types` 现在共享 Type 值构造函数类型的规则：先限制结构工作，再将各个
外层操作数提升一层；内层已绑定引用保持原深度，普通值索引与 Type 索引一并处理。
静态表达式、普通静态观察、树解释器与字节码调用都使用该服务。替换返回类型保留
closure environment、阶段及 unsafe 包装；已经位于 telescope 内的类型仍使用明确的
`with_callable_result`，不对所有调用者无条件二次提升。具名 telescope 继续负责自己的
参数抽象。NF 头仍 opaque，不用 PTC 处理程序状态。

名义类型的输入准备同样保留静态证据。`prepared_type_input_terms` 只观察 immutable
Let 前缀并读取已建立的值绑定；遇到未知产生器就结束前缀，不越过未知写入追认后续
读取。原准备语句仍保留。于是 `Tuple<Flag<flag>, bool>` 中合法的抽象值索引可以
形成静态名义身份，不再错误地进入 required invocation。形成义务使用观察到的实际
输入；已知错误的 trait 约束在定义检查时拒绝，真正依赖 Type 生产函数的义务仍在
invocation 时检查。不能只补名义身份观察却跳过其形成义务，也不能根据是否出现
某个库类型名称选择路径。

开放函数描述不再依据“固定前缀是否全为 Type”选择另一套收集器检查。
`ClauseInputRouting` 由开放描述与具体 NF 准备共同消费，底层槽位选择复用普通
子句的 `type_input_slot`；它区分固定参数、可选具名参数、整包供应及逐项收集，
检查重复／混合供应和 tuple packet 的固定前缀边界。旧的
`normal_type_list_route`、`normal_type_list_supply` 已删除，路由过程不重排输入。

收集元素按声明列表的元素域描述，整包和 tail spread 按完整列表域描述。混合值
前缀、可选参数及后置 Type 实参不再退回旧检查器。已知的 Type 输入可以替换后续
域；仍依赖 invocation 输入的域保留开放描述，不制造刚性 Type 值，不提前调用
生产者。具体化时原始源码继续完成值域、形成义务与效果检查。共享路由结构提供
S-expression Display，但它本身既不是可执行 IR，也不保存候选事务。

剩余退役边界：普通子句与 NF 尚未共用完整 ArgumentPlan；动态域的 NF If／Block／
Match／Integer 等参数树及其解释器仍存在。交错 contextual 输入需要保留产生器进度
和待定域的共同表达式协议，而不是改名或新增第二套解释器。上层观察 Option 适配器
及失败原因按需求传播也尚未完成。以上是当前实现限制，不是新的 Flurry 语言限制。

验证在 `/home/fin/luna`，使用 `development/check.py --timeout 900 -- test`。
普通子句、comptime 子句、名义参数、模板局部定义、动态 trait、依赖列表、NF 域、
NF 方法、NF 输入计划、inline 观察及开放 NF 输入计划共 441 项通过，退出 0，证据
`/tmp/luna-tools-90cb_nw9/result.json`（包含完整参数列表）。2 项既有 ignored 和
memoized value-indexed key 的 1 项显式 skip 没有解除，不计为本阶段完成。

新增真实 sysroot 的 open_normal_input_plans 共 5 项，覆盖 13 种成功的开放应用：
实际 u8 元素域、混合值前缀、可选参数、整包供应、逐项收集、列表／固定 packet
展开、空收集和后置可选 Type 输入。各例具体化 runtime 回调并比较两种 comptime
引擎返回的完整函数类型。拒绝例检查重复／混合供应、错误元素及列表域、未知标签、
缺失固定输入，以及未知域在 Type 实参到达后仍被检查。原 normal_input_plans
14 项也继续通过，保持求值轨迹、提前返回与 opaque 头身份。

`-p luna --test open_functions --test type_spreads --no-fail-fast` 479 项通过、3 项
失败，退出 101，证据 `/tmp/luna-tools-zv3lbekk/result.json`。三项分别为
suspended_index_computations_keep_short_circuit_and_arithmetic_checks、
computed_prefix_packets_share_the_named_type_list_input_plan 和
returning_type_list_inputs_keep_preparation_nodes_diverging。它们在改动前已复现于
`/tmp/luna-tools-47tozw_6/result.json` 与 `/tmp/luna-tools-hfba0xl_/result.json`；
除线程位置外的完整失败内容逐项一致。后续核对确认后两项是旧断言：未绑定的
opaque Tuple 仍被拒绝，但诊断已在 packet 入口给出；收集准备使用普通列表节点，
不再要求旧 Tuple 节点。现分别检查该拒绝诊断和 TypeList／ListConcat 的 Never
传播，并保留已绑定路径的完整 NF 实参与调用顺序检查；提前返回还在两种 VM 上
验证只调用 Run，不执行 Later 或 Default。

第一项暴露了真实缺口：开放签名描述没有保留足够的完整表达式／前提，机器算术的
定义侧证明因此可能遗漏，不能以具体 n=9 时计算成功替代。直接对每个描述片段
打开算术检查会丢失分支守卫及其前提；先描述再重新 elaboration 整个标注也可能
重复准备／执行生产者。该用例现由下面的完整 typed region 路径修复，原拒绝预期
保持不变；含未知域的环境仍未完成这一迁移。

局部 rustfmt check 退出 0，证据 `/tmp/luna-tools-s59wtbhs/result.json`；
`git diff --check` 通过。旧路由两函数已无源码引用。提交范围、源码哈希、基线比较与
原有改动哈希保存在 `/tmp/luna-open-routing-ivho07zy`。正式 docs 及原有工作区修改
均未动；未运行全 workspace、native 或性能基准，不以这些检查声称完整
ArgumentPlan／ContextualExpr 已经完成。


### 已知输入域的签名标注保留完整行为

`check_suspended_annotation` 先观察输入域依赖，而不是检查失败后再试另一条
路径。外层输入域全部明确，或完整依赖观察证明标注不使用那些待定输入时，未知的
仍然可以是普通值；标注经共享的 `elaborate_known_region` 一次生成普通 NIR，再把
整个区域交给现有 phase／机器算术检查。原输入绑定、局部 let、条件与短路结构一起进入证明，检查完成后才丢弃这一
定义侧区域。具体调用仍用原 suspended source 形成实际签名；NF 头不展开，静态
域已知不等于执行过生产者，也不把临时区域发布为冻结 invocation 产物。

`describe_known_region` 复用同一生成入口，但只在其原有 domain-only 消费位置丢弃
行为。已知域的签名标注不再先逐片描述、用临时操作数代替原绑定，再仅返回 Type
域。失败直接传播，既不退回较弱的描述器，也不等实际调用恰好给出小整数后才检查。
声明阶段没有调用的 `n + 1`（n:u8）也必须拒绝；`if n < 255 { n + 1 } else { n }`
则按整段分支前提检查。参数标注和返回标注使用相同入口。

这里不使用当前执行区域的 body requirement walker 证明未知输入无关。
旧 `body_local_references` 已由[自由源码输入分析](dependency-observations.md#46-已接通自由源码输入与执行区域扫描分离)替换；新的 `annotation_may_use_pending_inputs` 复用 lexical_inputs 的
完整源码边遍历，从标注表达式根出发，观察类型组件、模式、闭包／post-lambda body、
被引用的局部声明及其签名、继承前提和局部协议贡献。它按实际绑定身份与待定输入
比较，不凭名称，也不把别的函数的同名参数当成当前输入；未被引用的局部声明不因
出现 Item 节点而把整份外围环境强加给标注。公共遍历的初始化、协议索引和工作队列
已在声明捕获与这项表达式查询间共用，不新增第二套表达式边表。

该查询的 false 才是“不依赖待定输入”的证据。引用到候选绑定、存在未映射的
clause／Rigid 身份，或 delivered term 带未映射作用域参数时，都保守保留 pending；
不能把“没有 Local 节点”当成不依赖的证明。缺失／未完成的源信息与预算失败直接
传播，不能转成 false，也不凭失败改走较弱路径。输入环境扫描和查询的成功、失败
均记入同一 preparation 预算；查询只观察源码及已存在元数据，不调用生产者，也不
发布含当前活动上下文的缓存产物。

因此，位于标注前面的无关 Any 输入不再屏蔽 `Index<n + 1>` 的定义侧范围检查；
参数标注和结果标注一致。实际依赖未知域、或尚不能证明不相关的标注仍走原
contextual 描述器，其完整控制流／证明义务保留尚未迁移。后续仍需待定域结构区域
和更精确的符号身份映射，不能用本次依赖查询声称所有开放签名行为已完成。

普通 typed region 与旧描述器的拒绝诊断位置不同。测试据此核对同一错误行为：缺少
输入仍拒绝，过宽 packet 报超出位置槽，bool／void 不能代替 tuple，runtime 生产者
不能在 comptime 标注中调用。没有放宽类型或阶段检查。现有 dormant 无限循环
生产者用例也保留：仅定义标注不调用生产者，实际调用的契约失败仍正常传播。


本次在 `/home/fin/luna` 执行 `development/check.py --timeout 900 -- test -p luna`，
open_annotation_behavior、open_functions、open_normal_input_plans、normal_input_plans、
type_spreads、inline_pattern_observation 六组共 508 项通过，退出 0，完整命令与
证据 `/tmp/luna-tools-wttogcs2/result.json`。原先唯一真实失败的机器算术定义检查
用例保留原断言并通过；open_functions 全部 390 项通过。新增真实 sysroot 三项
测试覆盖直接／局部别名／死分支的范围保护、255 边界调用，以及未被调用的参数／
返回标注仍须拒绝无范围证明的运算；正例在两种 VM 中执行。

随后五组相邻检查（模板局部定义、comptime 子句、普通收集器、名义参数、CMTT
类型输入）共 424 项通过，退出 0，证据 `/tmp/luna-tools-znsww3fa/result.json`。
原来的抽象 Copy 默认值和 invocation Type 生产者两项 ignored，以及 memoized
value-indexed key 的一项显式 skip 仍保留，未算作本次交付。没有通过修改原
机器算术拒绝预期获得成功，也没有把未知域上下文计为已检查。

新测试及 open_normal_input_plans 的 rustfmt check 退出 0，证据
`/tmp/luna-tools-gipq3qnm/result.json`；两个既有大文件保持局部排版，避免无关全量
重排。`git diff --check` 通过，受保护文件哈希未变。提交清单、源码哈希和试验
证据在 `/tmp/luna-open-annotation-behavior-3js_nxhn`。未运行全 workspace、native
或性能基准，正式 docs 未动。


依赖查询扩展还定位到独立的既有缺口：直接在标注中写 `let choose = || T`，其中
T 的域仍由早前输入决定，再以 `choose()` 构造函数类型，会报告
`required comptime input is not yet available in this definition`。将
open_functions.rs 恢复为 d93fcd33 的内容、保留公共遍历重构但停用新查询消费者后，
相同来源仍给出逐字节相同诊断，证据 `/tmp/luna-tools-xc5fg3pz/result.json` 与
`/tmp/luna-tools-iqt0f2hh/result.json`，均退出 101；随后恢复本次源码。
[复现](repros/pending-type-closure-annotation.fl) 保留语言期望的正例，不修改它去迎合
当前拒绝，也不把它纳入已通过项。新回归覆盖的是已有合法路径：显式 comptime
类型输入、类型组件和捕获已知域条件的局部闭包；直接捕获待定 Type 仍须后续接通。


本次依赖观察接线在 `/home/fin/luna` 使用 `development/check.py --timeout 900 --
test -p luna` 验证。开放函数、两类 NF 输入计划、展开、标注行为和观察预算六组
512 项通过，退出 0，完整参数和证据 `/tmp/luna-tools-n7wppk30/result.json`。
新增拒绝例不提供具体调用，确认参数／结果标注不能因无关待定输入而漏掉范围证明；
正例使用真实 sysroot，覆盖守卫及边界、确有依赖的类型组件和局部闭包。观察预算
用例在同一份 HIR 上找到失败边界，再用足够预算重新检查并执行，不以失败授权
更强的 typed path。

公共遍历的旧消费者也复验：模板局部定义、comptime 子句、普通收集器、名义参数、
CMTT 类型输入和函数类型依赖六组 463 项通过，退出 0，证据
`/tmp/luna-tools-lg_hamot/result.json`。2 项既有 ignored 与 memoized value-indexed
key 的 1 项显式 skip 未解除。四个已格式化 Rust 文件的 rustfmt check 退出 0，
证据 `/tmp/luna-tools-iy7l0mjk/result.json`；open_functions.rs 保持局部排版。
`git diff --check` 通过；原有受保护文件的哈希未变。清单、源码哈希、基线比较及
本机检查证据在 `/tmp/luna-annotation-dependencies-fja2zljt`。未执行全 workspace、
native 或性能基准，未把闭包待定 Type 的既有失败计入通过项。


### 签名分类与调用供应共享输入依赖观察

`signature_annotation` 使用 `annotation_may_use_pending_inputs` 分类。普通 comptime
参数即使已有确定域，其值仍须由调用提供；分类不再用 `describable_open_type`
白名单决定是否保留这项依赖。源码显式闭包、嵌套闭包及控制流与其他表达式遵循
同一输入规则。域已经明确的标注仍由完整 typed region 检查，未知域的结构描述
限制没有因此消失。

依赖发现还必须接到实际供应。此前 `infer_open_clause_arguments` 和
`open_function_call` 使用 `signature_inputs_ready` 的回调副作用收集索引，丢弃
返回值；它在闭包等节点上停止，导致调用者没有提供标注恢复所需的 T。现在这两个
调用入口及被选中默认值的依赖闭包使用 `expression_parameter_dependencies`，沿
共同 lexical_inputs 遍历发现源码引用，再按实际 owner/binder 身份映射到本次
参数槽。默认值被显式覆盖时不观察它的供应依赖，也不执行它。既有默认值顺序、
共享 live graph 和已准备实参仍由原 ArgumentPlan 迁移中的供应机制保持。

推断入口只对 pending 标注建立本次局部依赖表，供应选择和 readiness 复用同一份
观察；不再为了判定“是否齐备”重复扫描源码，也不以另一个语法白名单拒绝闭包。
普通子句的刚性身份仍走原子句/词法输入机制，不能当成本次普通参数槽。被搬运的
staged 参数身份通过源码 telescope 的映射恢复；索引只为目标函数的参数构建，
不遍历全包的 staged 参数。delivered term 若仍有自由 scoped 参数而没有映射，
查询明确报告 `source dependency requires a delivered scope mapping`，不把它
当作空依赖。这个限制需要后续显式 binder 环境接线，不是新增的源语言限制。

观察及槽位映射消耗共享 preparation 预算，失败保留为失败；不调用生产者，不
把活动 locals、候选或等待者存进冻结签名缓存。源码查询共用表达式/模式、局部声明、协议及继承前提的边表，但目的不同：
定义检查中的 may-use 对未映射符号保守保留上下文；调用供应必须交付实际源码
参数槽；普通和 CMTT 子句默认值按自由声明身份等待推断输入。后两者不能拿一个
bool 代替依赖集合，也不能用 readiness 回调副作用收集不完整的依赖。
协议结构索引已改为[按不可变源码视图共享](dependency-observations.md#45-已接通源码协议索引与活动依赖观察分开)，
不再为每次查询重扫整包；语义依赖仍逐次观察，未宣称完成整体规模优化。
源码参数收集现按各观察范围扣除内部绑定，并沿局部声明递归传播自由输入；默认值、
staged 转发与别名分类已汇入同一入口，当前执行区域的 staging 扫描仍独立保留。

真实 sysroot 正例覆盖 Type 输入被直接及嵌套闭包捕获、直接调用与带类型子句的
推断调用、默认 Type 计算中的闭包捕获，以及带范围守卫的闭包在 9/255 输入下
执行；两种 VM 均验证结果。未调用定义中的闭包算术错误仍须拒绝。预算用例在同一
HIR 上触发调用依赖映射耗尽，再给足预算完成编译及调用准备，不以旧失败授权空
依赖或另一条求值路径。

仍须保持以下未完成项：

- [待定 Type 捕获](repros/pending-type-closure-annotation.fl)需要共同上下文表达式
  保存待定域、捕获和主体义务，不能伪造完整 Type。
- 真正依赖自身参数 binder、同时又要计算外部动态域的 callable telescope，仍需
  统一带作用域的 Type 值构造；不能伪造参数域，也不能删除真实的参数引用。

之前列为缺口的两个复现已分别纳入回归：[具名但无自身参数依赖的动态函数类型](repros/computed-named-callable-annotation.fl)
现在通过；[NF 参数中的局部闭包算术](repros/closure-nf-annotation-arithmetic.fl)
已在上一阶段完整依赖接线后给出预期的范围证明错误，补充验证据此纠正旧状态。
后者还有守卫成立和 255 边界的正例，且仍拒绝以 bool 替代 Index<10>，没有展开 NF 头。

`signature_inputs_ready` 的其他 readiness 消费者，以及描述器中其他
`describable_open_type` 用途尚未退役。完整 ContextualExpr/ArgumentPlan、动态
callable telescope 和 scoped delivered term 的映射仍需后续接通。具体表示边界见
[待定闭包设计](values-staging-and-publication.md#待定闭包需要保留可调用证据与主体不能伪造完整函数类型)。


### 函数类型构造按 binder 依赖选择路径

旧 `has_named_inputs` 仅看到参数名称就切换到 `type_expr` 的静态 telescope，导致
`fn(value: Input(flag)) -> Output()` 不能作为普通 comptime Type 值计算，局部闭包
的输入也会被过早索取。名称不是 callable identity 的组成部分；是否需要为后续
类型建立参数 binder 才是这两个构造入口的实际差别。

现在 `callable_type_needs_binders` 收集本 telescope 的具名参数身份，利用共同
`source_may_use_bindings` 观察完整输入及结果表达式。确认没有自身参数依赖后，
具名输入与位置输入一起生成已有 FunctionType/FunctionResultType 节点，复用静态
观察及两种 VM 的 callable_types 构造服务。Type 生产者留在原表达式位置，参数
类型与结果类型按源码顺序计算；命名没有引入第二套求值器，也没有删除原 HIR 名称。

实际引用自身参数，或存在尚未映射的符号依赖时，仍保守使用 binder-aware 路径，
保留 telescope 顺序、嵌套作用域、comptime 参数 sort、执行阶段及 unsafe 责任。
不把“观察失败”当作无依赖证据；候选扫描和源码观察均消耗 preparation 预算。
这尚未把真正依赖自身参数的动态 telescope 迁到共同 ContextualExpr，原静态入口
仍有其明确消费者，不能宣称所有函数类型构造已经统一。

真实 sysroot 测试既覆盖具名闭包标注的直接/推断调用，也检查执行轨迹：Build 的
输入 Type 生产者失败时，结果 Type 生产者不执行；同一 VM 重试后严格按 Build、
Input、Output 各一次完成，并与普通函数类型相等。树引擎和字节码引擎均验证。
原函数类型依赖组继续检查嵌套 binder、依赖值索引、阶段及 unsafe 边界；新预算
用例在同一 HIR 上耗尽后重新检查，不复用失败的无依赖结论。


当前在 `/home/fin/luna` 通过 `development/check.py --timeout 900 -- test -p luna`
检查：开放函数、标注行为、观察预算及局部模板四组 433 项通过，证据
`/tmp/luna-tools-xtldocsm/result.json`；函数类型依赖、comptime 子句、名义参数、
CMTT 类型输入及两类 NF 计划六组 435 项通过，证据
`/tmp/luna-tools-f2kn4pfq/result.json`。均退出 0；本次覆盖中的 1 项既有 ignored
及 memoized value-indexed key 的 1 项显式 skip 未解除。五个修改的 Rust 文件
rustfmt check 退出 0，证据 `/tmp/luna-tools-9dyomqdw/result.json`；
`git diff --check` 通过。没有执行全 workspace、native 或性能基准。
正式 docs 与原有受保护修改未动，范围与本机证据在
`/tmp/luna-nf-closure-region-5x1w2vx7`。


## 18. invocation 已完成输入共用槽位存储

NF 与名义类型／typealias 的输入帧现在共用 `input_bindings::Bindings`。它只存放
实际完成的 declaration、声明域与 comptime Term，槽位保存完成序号；未完成槽位
明确为空。名义类型不再预填与声明同名的 rigid，并另靠 ready 集合判断它是否是真实
输入。完成先后与最终声明参数顺序分别保留，最终收集缺少任何槽位都会失败。
S-expression dump 显示 pending 槽位及真实完成值，不把待定输入伪装成实参。

域的受预算复制和值的 residual transport 都成功后，才一次发布绑定；重复或越界槽位
拒绝发布。名义帧仍先取得局部值快照，再完成绑定，最后更新调用环境。资源耗尽或
未获授权的抽象值不能留下可被后续替换读取的半完成槽位。

共享的是输入所有权和完成状态，不是取消各消费者的接纳边界。NF 继续使用自己的
域替换规则；名义类型在动态域导致实际 sort 与声明 sort 不同时，仍按声明顺序调用
带 layout 与抽象值证据的 admission，再形成替换。已完成的 Type 参数可使后续数据
参数实际接收 Type；不能因此放行其他参数中不符合域的值。未完成参数不进入替换
实参表，仍由原计划等待后续供应并恢复。

这只是通用 invocation 参数计划的基础。普通 comptime callable 的参数包目前要求
Type 子句在准备时已知；`count<.values = [1, 255], Byte()>()` 一类调用同时受列表域
准备和 callable 签名准备阻塞。不能提前执行 Byte() 越过源码位置来修补。原
`pending_clause_domains_can_wait_for_a_type_producer` 仍保留 ignored；本轮显式执行
在列表元素域缺失处失败，退出 101，证据 `/tmp/luna-tools-47jlkw01/result.json`。
完整可恢复 callable 参数包、ContextualExpr 以及旧 NF 参数树退役都尚未完成。


验证在 `/home/fin/luna` 使用 `development/check.py --timeout 900 -- test`：

- `-p comptime_interpretor --lib --no-fail-fast -- --skip
  normal_input_data_tests::clause_list_inputs_share_domain_scope_and_budget_validation`
  443 项通过、1 项显式跳过，退出 0，证据 `/tmp/luna-tools-v44ctzs5/result.json`。
  新增检查覆盖完成预算的逐边界失败／重试、未授权传输不破坏已有槽位，以及乱序
  完成后的动态 sort admission 和抽象值证据。原有树解释／字节码检查继续通过。
- 首次未跳过的 VM 检查为 440 通过、1 失败，退出 101，证据
  `/tmp/luna-tools-t65ep9ap/result.json`。该项期望 Type 错误，实际为带参数与 Value
  阶段原因的 FunctionInputRejected；用改动前 HEAD 源码单独执行仍为相同失败，
  退出 101，证据 `/tmp/luna-tools-wwaxjwl3/result.json`。未改动其预期。
  后续在本文件第 21 节核对并修正错误类别断言；该用例已恢复到不跳过的 VM 检查中。
- `-p luna --test normal_input_plans --test open_normal_input_plans --test nominal_staging
  --test nominal_value_parameters --test normal_form_domains --test comptime_clause_inputs
  --test ordinary_clause_collectors --no-fail-fast` 共 412 项通过、1 项失败、1 项 ignored，
  整条命令退出 101，证据 `/tmp/luna-tools-gxt9_tqw/result.json`。失败仍为既有
  `memoized_value_indexed_keys_keep_declaration_and_value_arguments`：dispatch 函数
  仍需 ordinary invocation input staging。ignored 为上文的动态 Type 供应用例。

未运行全 workspace、native 或性能基准。正式 docs、既有资源检查修改与 library
README 的原有工作区内容保持不变。

修改 Rust 文件的格式检查退出 0，证据 `/tmp/luna-tools-jz03jkgm/result.json`；
本次范围的 `git diff --check` 通过。


## 19. 可复用 invocation 保留普通 comptime 值参数

`memoization::Invocation` 同时承担可复用代码准备与受限的 Type 结果缓存。旧准备
分支对无需普通 Type 参数专门化的 callable 使用 strict dispatch closure；该 closure
要求所有普通输入均已完成 staging。因此合法的 `comptime enabled: bool` 在调用值
尚未供应时就被拒绝。问题位于执行准备责任，与名义类型值参数的接纳、Eq／Hash 或
是否缓存 bool 结果无关。

准备现在使用与普通 invocation 相同的 `dispatch_instances_for_invocation`。带普通
comptime 输入的声明保留模板与契约，VM 在每次调用时按原参数协议绑定实际值。
仅取得函数身份不执行主体或输入，也不授权将未绑定模板转成可执行 IIR；strict
materialization 的检查仍保留。现有 Type 输入专门化与其代码准备缓存继续承担原职责。

被保留模板内的 trait 方法与关联值直到实际调用才会被需求。可复用 invocation 现已
安装与普通入口相同的源世界解析服务，方法选择共用 `resolve_method_for_invocation`，
使用所属不可变 package/context 和当前调用预算。关联值同样经过原有的被选初始化器
协议。VM 执行所选主体、检查接口和参数契约；代码复用不保存本次值、成功断言或
消费权限。传入不同名义 value index 的值仍拒绝，随后合法调用可以继续使用同一准备对象。

Type 结果过滤保持在结果缓存准备之前。非 Type 返回值不准备键协议，也不查存结果；
返回 Type 的调用仍要求源 Eq／Hash、兼容的结构判别与结果形成检查。新源码用例直接
使用真实六包的 `ComptimeEq`、`ComptimeHash`，通过 bool 参数选择 bool／u8 返回值，
分别验证首次执行和后续命中。没有新增按某个库定义或源码形状识别的分支。

本轮开始在 HEAD `1682462e` 显式复现
`memoized_value_indexed_keys_keep_declaration_and_value_arguments`，仍以 dispatch
input staging 失败，退出 101：`/tmp/luna-tools-2ngvdo4c/result.json`。修复后该项
随 nominal_value_parameters 全组通过，不再需要显式跳过，原用例与期望未改动。

完整 ArgumentPlan、动态 Type producer 的列表上下文、生成定义的 invocation 发布
与统一世界所有权仍未完成。本节闭合可复用入口的模板保留和需求解析，不将其描述为
所有 comptime 调用准备已经统一。


验证目录 `/home/fin/luna`，统一入口 `development/check.py --timeout 900 -- test`：

| 范围 | 本轮结果 |
| --- | --- |
| 原失败与 Type 输入准备缓存 | `-p luna --test nominal_value_parameters --test staged_invocation_reuse` 在混合命令中分别 307、4 项通过；证据 `/tmp/luna-tools-m_sqthdz/result.json`。该混合命令整体退出 101，因为新增用例最初选中了同名方法入口，以及契约失败断言使用了错误的错误类别，不能把整条命令写作成功。 |
| 最终真实 sysroot 用例 | `-p luna --test invocation_value_templates --no-fail-fast`，3 项通过，退出 0；`/tmp/luna-tools-bijraiqu/result.json`。使用唯一命名入口，明确检查 ProgramContractRejected；覆盖每次值供应、名义域拒绝、方法／关联值选择、预算／契约失败恢复、非 Type 结果不缓存、真实 meta 协议的 Type 结果命中。 |
| 原 invocation、值引用与关联调用 | `-p luna --test prepared_invocations --test dynamic_trait_clauses --test deferred_associated_values --test comptime_memory --no-fail-fast`，35 项通过，退出 0；`/tmp/luna-tools-9ud_aaij/result.json`。 |
| 结果缓存与泛型调用回归 | `-p luna --test execution memoized -- --nocapture`，13 项通过，退出 0；`/tmp/luna-tools-ipoojvso/result.json`。覆盖协议判别、失败不发布、独立世界、Type 普通输入及结果形成检查。 |

最终生产源码在以上检查之间没有语义改动。三个 Rust 实现／测试文件的格式检查
退出 0，证据 `/tmp/luna-tools-s95822j3/result.json`；materialization 仅修改接口注释，
未全文件重排；`git diff --check` 通过。清单、原始源码和原有修改的哈希核对保存在
`/tmp/luna-invocation-template-dispatch-xuws5gmu`。未运行全 workspace、native 或性能
基准；正式 docs 与原有工作区修改保持不变。

## 20. 函数身份的成本、准入与执行权限

`Value::immutable_copy_work`／`immutable_clone_work` 估算值表示的工作量，
不接收源世界，不能判断声明是否存在，也不证明函数实例满足子句。Function 身份
即使指向未知声明，仍有有界表示成本。此前 execution 测试要求包含这种身份的
tuple 返回 None，与本文件第 2 节区分表示和接纳的边界冲突；现已纠正该断言，
保留原有的准入拒绝，并增加独立正反例，而未改变生产实现以迁就测试。

源世界中的 `validate_immutable_input` 负责域、声明身份和实例形成契约；
`admit_callable_identity` 查询源声明及其输入约束，不执行函数。runtime 函数身份
可以作为编译期所携带的数据，通过对应函数类型的准入；这不授予 comptime 调用
该 runtime 函数的权限。comptime callable 的阶段类型仍须匹配。wire 解码是否
允许重建 callable 是另一项传输政策，本次没有放宽。

新增检查覆盖同成本的已知／未知函数及嵌套身份、同成本但满足／违反 Type 子句
的实例、runtime／comptime 签名错配、准入不执行、runtime 调用的 PhaseMismatch，
以及随后合法 comptime 调用恢复。它们防止再次用表示成本替代世界或阶段检查。

验证目录 `/home/fin/luna`，入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p luna --test execution -- comptime_snapshot`：9 项通过，退出 0；
  `/tmp/luna-tools-6bgon2u4/result.json`。
- `-p luna --test execution -- memoized comptime_key comptime_snapshot pure_query`：
  32 项通过，退出 0；`/tmp/luna-tools-c6gg0gyl/result.json`。包含上一阶段原失败项。

初次新测试误用了私有 `invoke_value`，编译失败（`/tmp/luna-tools-tuqp5sq4`）；
改用既有公开 `invoke` 后执行以上检查，没有扩大 VM API。未运行全量测试，
没有修改正式 docs 或工作区原有修改；本次交付是契约检查的纠偏，不是新增执行能力。

## 21. NF 参数检查消费统一的接收世界查询

`Head::check_arguments` 现在直接接收 `value_admission::DomainQueries`，NF 参数
循环只负责按声明顺序代入域并准入值。布局、集合元素域、抽象值证据、callable
签名和目标事实来自接收世界，不再在生产检查入口中通过 `is_tuple`／`is_list`
布尔回调重新推断表示。编译器生产调用方及 NIR／luna 测试均已迁入这一入口，
原来的 13 个 `check_closed_arguments*`／分级 evidence 适配接口已删除，没有保留
另一套默认预算、隐式空作用域或 shape-only 集合授权入口。

测试直接声明所需 DomainQueries；无证据 fixture 只提供拒绝／缺失结果，不封装
参数检查。集合与元组用例提供注册 Head 的视图，callable 用例直接交付带 Budget／
Invalid 区分的签名查询，保留原来的拒绝、作用域、目标范围、预算和失败恢复断言。

VM 的普通调用与 NF 形成检查共用 `with_admission_queries`；源声明世界的 callable
专门化和 materialization 共用另一组由 `DeclarationWorld` 提供的查询。IIR 从保留
的实例、签名和头索引提供同一接口；描述工作量检查保留其原有 binder-domain
证据。查询不执行 NF 头或函数正文，形成 predicate 仍在原调用方检查，表示成员
资格不能替代形成契约。普通函数的参数准备与 NF 的依赖代入循环尚未合成完整
ArgumentPlan，这次迁移不改变输入求值顺序或授予新的结果缓存权限。

集合查询使用注册 Head 的完整元素域视图；元组查询通过
`registered_tuple_fields` 同时核对元组头、其精确列表参数域及列表头注册证据。
匹配的载荷或孤立的 Tuple 角色不足以授权布局。已有 TypeList／Tuple 表示元数据
仍是后续迁移对象，本次没有把它们升级为语言中的特殊参数种类。

查询接口保留显式 `Error::Budget`，即使宿主查询返回时调用者仍有剩余 fuel，也按
预算失败处理，不转成类型不匹配。VM 已有的 typed callable 查询直接进入 NF 检查，
不再先压成 Option 再靠零 fuel 猜测；声明世界的 callable 查询也已按下节迁移。
名义布局核心及源世界／VM 准入已改为显式失败结果，见第 23 节；IIR 的 NF 参数
准入也已迁移，见第 24 节。存储阶段查询已按第 25 节迁移；
elaboration 的其他观察入口及部分旧描述服务仍有 Option 适配，不能称为全部查询协议已统一。

原 `clause_list_inputs_share_domain_scope_and_budget_validation` 在改动前再次以
`FunctionInputRejected { stage: Value, .. }` 与旧 `Type` 断言不符而失败，证据
`/tmp/luna-tools-huvgcww2/result.json`。现在检查参数索引、声明身份、Value 阶段和
非空原因；合法列表、作用域拒绝、错误元素与预算失败后的恢复断言均保留。没有
为使该测试通过而把带来源的诊断退化成 Type。

验证目录 `/home/fin/luna`，以下参数接在
`python3 -B development/check.py --timeout 900 -- test` 后；最终均退出 0：

| 参数 | 结果与证据目录 |
| --- | --- |
| `-p nir --test value_admission --test normal_inputs --test normal_callable_inputs --no-fail-fast` | 36 通过；`/tmp/luna-tools-8s_ciljc`。新增检查覆盖同一接收证据下的依赖域、缺失／错误 callable、预算与恢复，以及精确 tuple/list 注册证据。 |
| `-p comptime_interpretor --lib` | 450 通过，无跳过；`/tmp/luna-tools-p02immv8`。 |
| `-p type_resolution --test callable_world --no-fail-fast` | 3 通过；`/tmp/luna-tools-b7ayydti`。 |
| `-p iir --test types --test comptime_lists --test callable_type_cores --test integer_formation --test float_formation --no-fail-fast` | 39 通过；`/tmp/luna-tools-0k28ng2b`。 |
| `-p luna --test normal_form_domains --test nominal_staging --test invocation_value_templates --test library_list_hash --test library_type_equality --test comptime_iir_bytecode --no-fail-fast` | 43 通过；`/tmp/luna-tools-d4xj4r2k`。覆盖真实 sysroot 列表协议、Type 比较、NF 域及 source/IIR 调用。 |

初次迁移 IIR 闭包时缺少参数类型标注，Rust 编译失败（`/tmp/luna-tools-i0e487ds`）；
补齐借用类型后完成上述检查。较早的 VM／源码检查不替代最终证据。Rust 格式检查
`/tmp/luna-tools-psp0ba5c` 退出 0，提交范围 diff 检查通过。未跑全 workspace、native
或性能基准；正式 docs 及原有资源分析、library README 的修改保持原样。

后续适配接口退役仅删除旧转发层，统一检查核心保持不变。工作目录仍为
`/home/fin/luna`，使用同一 `development/check.py --timeout 900 -- test` 入口：
`-p nir --test normal_inputs --test normal_callable_inputs --test value_admission
--no-fail-fast` 的 36 项通过（`/tmp/luna-tools-ubn0xdfj`），
`-p luna --test nominal_staging --no-fail-fast` 的 11 项通过
（`/tmp/luna-tools-ytdq5xtr`），两者退出 0。初次转换测试闭包时两处缺少参数类型
标注，分别在 `/tmp/luna-tools-p1t_1ufe`、`/tmp/luna-tools-igc2pig2` 编译失败；
补齐标注后完成上述检查。已全仓核对旧接口无剩余代码引用，没有再跑全量 VM
或 workspace，也没有改变任何语言预期。

## 22. 声明世界的 callable 查询保留失败类别

旧 `DeclarationWorld::callable_signature`／`callable_value_signature` 将域查询、
子句实参检查及签名代入的错误压成 Option。接到 DomainQueries 时只根据 fuel
是否为零恢复 Budget，但预算预留失败不会消费尚未预留的额度：剩余 2，查身份后
剩余 1，再尝试预留 3 个输入域就会失败，旧入口却返回 `Ok(None)`。改动前以该
真实路径复现，`/tmp/luna-tools-vd8w5wh6` 退出 101，不能将其解释为缺少声明证据。

这些入口现在返回 `Result<Option<Term>, value_admission::Error>`：只有未知声明、
接口或成员返回 `Ok(None)`，已知 callable 的错误输入／签名返回 Invalid，资源
失败返回 Budget。`input_domains` 同样保留 typed 错误，参数预留、NF 参数检查及
substitution 的错误不再经过 `.ok()` 丢失原因。查询仍不读函数主体或执行调用，
也不代替外层形成 predicate。预算计算和失败预留的剩余额度保持原规则。

DomainQueries 直接传递该结果；materialization 的 callable identity 路径先保留
失败原因，再单独诊断缺失声明证据。其他现有字符串诊断消费者通过错误的 Display
取得原因，没有为查询添加结果缓存、失败缓存或执行降级。

验证目录 `/home/fin/luna`，入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p type_resolution --test callable_world --no-fail-fast`：5 通过，退出 0；
  `/tmp/luna-tools-ropet5x0`。覆盖索引成本、独立世界、缺失／无效身份，以及函数与
  方法查询从零预算到成功的每个边界和失败后的恢复。初次新增测试使用了要求 Copy
  的数组重复语法，编译失败于 `/tmp/luna-tools-xv7i7z0j`；改用普通 Vec 后通过。
- `-p luna --test normal_form_domains --test nominal_staging --test invocation_value_templates
  --test comptime_iir_bytecode --no-fail-fast`：28 通过，退出 0；`/tmp/luna-tools-s9zup4rc`。
- `-p luna --test scoped_comptime_callables -- retained_callable_signatures normal_form_method_inputs
  method_normal_form_inputs runtime_instances_retain_function_valued_type_arguments
  materialized_callable_signatures`：5 通过，退出 0；`/tmp/luna-tools-6p2vc5nw`。

未运行全 workspace、native 或性能基准。仍未统一所有布局／描述查询的失败协议，
也未完成普通参数与 NF 参数的可恢复计划；正式 docs 与原有工作区修改保持不变。


## 23. 名义布局的失败贯穿准入与代入

`query_nominal_value_fields` 与索引世界入口 `query_nominal_value_fields_with`
统一返回 `Result<Vec<Term>, NominalAdmissionError>`。声明查找、名义实参准入、
递归布局和 newtype 表示代入直接保留 Budget／Invalid；不再将错误压成 None 后
由调用方猜测。缺失声明、错误变体和不合域的实参属于 Invalid，预算或递归限制
属于 Budget。失败的预算预留保留原有余额，正余额不代表查询尚有能力完成。

`declaration_fields::instantiate` 是字段代入核心，按字段及参数帧预留成本，
显式接收 caller frames，仍验证空载荷的完整参数帧。`instantiate_value_fields_checked`
作为闭合的选中字段入口向它单向适配（见第 26 节），旧 Option 接口也只向这一核心适配，
在兼容边界把 Budget 转为零 fuel；没有保留另一套检查或布局算法。

源世界的普通函数参数、trait／impl 参数、DeclarationWorld 查询和 materialization
准入／staged 名义值冻结已直接消费显式结果。VM 的普通／名义参数、effect 参数
及启动时的 dispatch 实例接纳也直接传播结果，
最终错误分类按错误枚举匹配，不能再次根据 fuel 是否为零分类。VM 仍从自身的
源声明、生成声明或保留的具体 IIR 实例取得字段，再调用同一字段代入核心；
不能把它替换为仅能看到原 package 的扫描器，丢失所属世界的声明证据。

IIR TypeLinks 的 NF 参数准入现已按第 24 节迁移；elaboration 的借用元数据入口、
VM 的旧描述／解码观察接口仍有 Option 边界；IIR 存储查询已按第 25 节迁移。本阶段没有实现统一 ArgumentPlan，
没有改变形成 predicate、类型身份、调用顺序或缓存准入政策。

验证目录 `/home/fin/luna`，入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --test nominal_parameters --test trait_parameters --no-fail-fast`：
  47 通过，退出 0；`/tmp/luna-tools-zw3u7ah9/result.json`。新增检查覆盖有余额的
  预留失败、未知声明、错误实参、查找错误原样传递及后续成功。
- `-p comptime_interpretor --lib`：450 通过，退出 0；
  `/tmp/luna-tools-e5fa_wmo/result.json`。随后仅新增 VM 边界用例，使用同一入口
  加 `normal_input_data_tests::nominal_layout_admission_keeps_failed_reservations_typed
  -- --exact`，1 通过，退出 0；`/tmp/luna-tools-0v1q0obe/result.json`。
- `-p luna --test nominal_value_parameters --test nominal_staging --test newtypes
  --test library_list_hash --test library_type_equality --test invocation_value_templates
  --no-fail-fast`：548 通过、1 失败，整体退出 101；
  `/tmp/luna-tools-wx4i84o_/result.json`。唯一失败为
  `mixed_trait_arguments_expand_literal_and_computed_packets` 的旧诊断文字断言。
  使用本阶段保存且逐个核对 HEAD `9b65039a` 的 Rust 基线执行该项，仍以相同位置和
  原因失败，退出 101；`/tmp/luna-tools-jbdefcnq/result.json`。基线检查后逐字节恢复
  本阶段源码并核对，不把混合命令记作全通过，也没有修改该测试预期。

初次 NIR 检查漏写 newtype ConversionError 的显式转换，编译失败于
`/tmp/luna-tools-mz57ov6y`；补齐 Budget／Invalid 映射后执行上述检查。
未运行全 workspace、native 或性能基准。正式 docs 和原有工作区修改均未纳入。

后续单独纠正该变参拒绝例的诊断断言：统一的子句签名检查会先拒绝未注册的
收集域，诊断为 `variadic clause input requires a bound meta.List<T>`。拒绝要求、
注册后的展开成功及所有非法载荷检查不变，没有改生产代码。使用同一入口执行
`-p luna --test newtypes mixed_trait -- --nocapture`，两项通过、退出 0，证据
`/tmp/luna-tools-vglziu_q/result.json`；该项不再列作未解决基线失败。


后续将 impl 实参检查、VM dispatch 实例接纳和 staged 名义值冻结也迁入显式查询，
删除两处零 fuel 重建以及一处“无效或超预算”的合并诊断。预算仍由原调用者持有，
没有补发预算或缓存拒绝结果。type_resolution、comptime_interpretor、materialization
中已无对 `nominal_value_fields_budgeted` 的引用；其余兼容观察者仍按上文保留。


验证目录仍为 `/home/fin/luna`，同一 `development/check.py --timeout 900 -- test` 入口：

- `-p type_resolution --lib captured_head_domains`：3 通过，退出 0；
  `/tmp/luna-tools-rs7xw2i2/result.json`。
- `-p comptime_interpretor --lib`：451 通过，退出 0；
  `/tmp/luna-tools-3dhyk3bu/result.json`。
- `-p luna --test nominal_value_parameters --test nominal_staging --test newtypes
  --test invocation_value_templates --no-fail-fast`：534 通过，退出 0；
  `/tmp/luna-tools-i0976fhf/result.json`。包括完整 newtypes 组、名义值参数及真实
  sysroot 调用模板；未重复全 workspace 检查。


## 24. IIR 的闭合参数检查保留接收世界错误

IIR 使用保留的具体名义实例、callable 身份与接口签名核验闭合 NF 参数。
该索引不访问源声明、不执行函数，也不以源泛型布局替代交付的具体布局。
`TypeLinks::nominal` 和内部计费现在保留 `value_admission::Error`；布局查询将
缺失证据、错误变体报告为 Invalid，费用不足报告为 Budget。函数／方法签名查询
只有找不到完整实例键时返回 `Ok(None)`，查询失败直接传播。

`validate_application` 中布局和 callable 的 Option／零 fuel 重建分支已删除。
集合查询原已保留显式错误；这三个查询现在直接供应统一 DomainQueries。
检查顺序、字段复制成本及索引键没有改动；开放 binder 或 runtime 依赖仍需要
生产端的环境证据，不能由这项闭合值检查代替。接口签名证据也不代替外层对
method dispatch 的独立检查。

两个回归在生产代码修改前复现：callable allowance=418、nominal allowance=1225，
均在 remaining=1 时误报不兼容值，见 `/tmp/luna-tools-4w07my_g`（退出 101）。
修复后检查每一档预算直到首次成功，之前只允许预算失败；另检验缺失世界、错误
变体／字段、方法的完整实例键及随后成功。没有通过增加预算或更换合法载荷避开问题。

IIR 的 runtime storage 观察已按第 25 节迁入带错误类别的查询；elaboration 和
VM 的其他描述观察尚未全部迁移。所有布局接口或可恢复参数计划仍未完成。

验证目录 `/home/fin/luna`，入口 `python3 -B development/check.py --timeout 900 -- test`，
最终检查均退出 0：

- `-p iir --lib --test types --test comptime_lists --test callable_type_cores
  --test integer_formation --test float_formation --no-fail-fast`：100 通过，
  `/tmp/luna-tools-kbzrciz5/result.json`。含 61 项 lib 检查与 39 项相关集成检查。
- `-p luna --test normal_form_domains --test nominal_staging --test invocation_value_templates
  --test library_list_hash --test library_type_equality --test comptime_iir_bytecode
  --no-fail-fast`：43 通过，`/tmp/luna-tools-w4xecpvg/result.json`。覆盖真实 sysroot
  列表协议／Type 相等及 source/IIR 调用。

未运行全 workspace、native 或性能基准；正式 docs 和原有修改保持不变。


## 25. 存储阶段查询与资源失败分开交付

存储阶段检查回答 runtime 能否保存该类型的值、comptime 能否保存该载荷，不是
PTC 类型命题求解。`phases::StorageError` 明确区分 Budget 和 Invalid，提供稳定的
Display；递归深度、描述计费、布局解析与字段代入的失败传到调用者，不再统一压成
`() / None`。Invalid 保留存储阶段的静态诊断，来自底层类型／布局服务的细节仍
归纳为对应的描述或名义布局错误，不宣称它已是完整来源链诊断。

runtime／comptime 的三个可供应布局入口统一接收
`Result<Vec<Term>, StorageError>`。IIR 的具体存储检查、VM 的源世界及 IIR 世界
检查、生成器的数据布局供应全部接入。返回空字段仍表示成功取得无载荷视图，
缺失具体实例必须明确报错；没有把 Option 缺失默认接受为空布局。

VM 按错误类别选择 StepsExhausted 或 PhaseMismatch，不能根据 fuel==0 分类。
失败预留保留剩余预算；最终失败路径只扣除实际已消耗工作。NIR 的 runtime 递归
观察中，closure context 的声明域查询和 newtype 表示代入也复用本次预算，删除
内部重新供应 100000 的做法。没有增加结果或失败缓存。

存储检查仍遍历所有可能的载荷，包括未激活的 enum/error 分支；它与值准入只观察
选中变体的布局请求不同。phantom 参数、Type 值所描述的指针、runtime callable
签名不等于保存指针载荷，opaque NF 头仍不展开。既有这些准入边界保持不变。

旧 source convenience 入口仍将最终错误渲染成静态文本，生成器候选预检仍返回
bool；它们是外层诊断／选择边界，没有第二套存储算法。源声明 struct/enum 的受限字段代入现已按第 26 节接通；
更一般的观察依赖、目标布局计划和可恢复请求协议仍未完成。

验证目录 `/home/fin/luna`，入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --test phases --test closure_contexts --test unsafe_callable_types --no-fail-fast`：
  53 通过，退出 0；`/tmp/luna-tools-iaw9bsza/result.json`。新增检查覆盖三个查询
  入口的正余额 Budget、零余额 Invalid、非法阶段及失败后恢复。
- `-p comptime_interpretor -p iir --lib --no-fail-fast`：VM 451、IIR 61 项通过，
  退出 0；`/tmp/luna-tools-71u1_o3q/result.json`。VM 的具体 enum 查询还验证
  空变体扫描需计费，预留失败保留余额并报告 StepsExhausted，随后合法检查恢复。
- `-p luna --test comptime_storage_domains --test comptime_pointer_phase
  --test cmtt_phase_boundaries --test cmtt_type_inputs --test closure_environment_parameters
  --test comptime_iir_bytecode --test invocation_value_templates --no-fail-fast`：
  76 通过，退出 0；`/tmp/luna-tools-_a0xoi2d/result.json`。包括未激活载荷拒绝、
  phantom／代码描述正例、闭包环境、source/IIR 执行和真实 sysroot 调用模板。

初次编译仍调用返回 String 的声明域入口，失败于 `/tmp/luna-tools-04r8uwk6`；
改用已有的 `declaration_parameter_domains_checked` 后完成上述检查，没有按字符串
内容猜测预算错误。未运行全 workspace、native 或性能基准；正式 docs 与原有改动
保持不变。


## 26. 源声明存储字段使用同一受限代入核心

字段代入归属于声明 telescope 的通用 substitution，不属于 Tuple/List 特例，也
不是 NF 头执行。`declaration_fields::instantiate` 从原选中字段核心提取，显式接收
调用者的 binder frames；复制前预留字段和参数帧成本，再用已有 bounded substitution
检查与代入。闭合值准入继续通过空 frames 使用它，开放存储描述保留嵌套 binder 深度。

Struct／Enum 的 `instantiate_stored_fields` 仅交付字段类型，不复制随后被丢弃的
字段名称、变体名称或 shape。Enum 先预留全部变体遍历及输出字段槽位的成本，再
一次准备参数帧并代入完整字段集合；无需临时收集字段引用数组。空变体仍消耗遍历
预算，零字段的声明仍验证完整参数帧。
值准入的选中变体查询继续只代入选中字段，两种选择共用核心而不互相替代。

NIR runtime／comptime 存储检查、VM 源声明存储检查，以及生成器的数据检查均已
替换原来的无界命名字段／变体实例化路径。程序流分析的 struct 字段及选中 enum
载荷查询也已接入该核心，直接使用 flow 的共享工作预算。VM 的具体 IIR 路径继续观察保留的实例，
不重新应用源声明参数。形成 predicate、目标布局、资源权利与缓存准入仍由各自
消费者负责，取得字段类型不能代替这些检查。

基线复现保留最终测试，只将 VM 的生产实现恢复为本轮前 HEAD `e8b6d7bc`：
源声明的 128 个空变体在预算 16 时错误成功，`/tmp/luna-tools-vu7znsjh/result.json`
退出 101。测试同时覆盖源世界和具体 IIR 世界的计费、正余额失败及恢复；修复后
两条路径均通过。基线检查后逐字节恢复最终 VM 文件，没有改动其他工作区内容。

`instantiate_fields_in_scope`／`instantiate_variants_in_scope` 仍有需要名称和变体
元数据的其他消费者，本次没有将其全部删除或宣称完整布局服务已统一。后续应按
调用方所需视图推进，不能用完整命名结构的复制替代只需字段类型的受限观察。

验证目录 `/home/fin/luna`，统一入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --test nominal_parameters --test phases --test closure_contexts --no-fail-fast`：
  76 通过，退出 0；`/tmp/luna-tools-lfjzfg0m/result.json`。覆盖逐档预算到成功、
  开放帧与嵌套函数深度、空载荷错误实参、NF 头保留、名称无关成本及全部变体。
- `-p comptime_interpretor --lib`：451 通过，退出 0；
  `/tmp/luna-tools-e8076_m4/result.json`。随后基线对照没有改变最终源码。
- `-p luna --test comptime_storage_domains --test comptime_pointer_phase
  --test cmtt_phase_boundaries --test cmtt_type_inputs --test closure_environment_parameters
  --test comptime_iir_bytecode --test invocation_value_templates --test nominal_staging
  --test nominal_value_parameters --test newtypes --no-fail-fast`：606 通过，退出 0；
  `/tmp/luna-tools-yhr9c8na/result.json`。覆盖实际消费者和真实 sysroot 调用模板。

未运行全 workspace、native 或性能基准；正式 docs 和原有工作区修改保持不变。


程序流分析原先用 `instantiate_types_in_scope` 代入 enum，隐含所有声明参数均为
Type；这与 struct 的普通值参数支持不一致。合法的 `Choice<bool, true>` 在显式
`flow::analyze_in_package` 中被拒绝为 InvalidSignature，改动前复现证据为
`/tmp/luna-tools-gt2m6elt/result.json`（退出 101）。现在按声明 parameter_types
进行通用代入，选中空变体也核验完整参数帧；struct 路径则直接取得字段类型，
不复制随后丢弃的名称。此查询不提供程序事实或调用 PTC 来证明内存规格。

flow 接收端将代入 Budget 映射为 Unknown(Arithmetic(BudgetExhausted))，真正的
域／scope 错误映射为 InvalidSignature。新增源码用例覆盖实际分析入口、递增
预算直到默认上限内成功、伪造声明参数域拒绝、失败后恢复和 IIR 执行结果。
测试没有假设完整流分析必须在固定 5000 单位预算内结束；初次该假设不成立，
`/tmp/luna-tools-if3w06a_` 退出 101。最初测试调用 unwrap_err 要求成功值 Report
实现 Debug，编译失败于 `/tmp/luna-tools-pvn7y8wb`；改用显式取得错误，未扩大生产 API。

验证目录 `/home/fin/luna`，入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p luna --test nominal_value_parameters program_flow_accepts_enum_value_indices_and_preserves_query_failures
  -- --exact`：1 通过，退出 0；`/tmp/luna-tools-ajgr8op1/result.json`。
- `-p program_verification --test verification --test memory_type_budget --test associated_resources
  --no-fail-fast`：5 通过，退出 0；`/tmp/luna-tools-37vysx26/result.json`。
- `-p luna --test nominal_value_parameters --test newtypes --test nominal_staging
  --test invocation_value_templates --no-fail-fast`：535 通过，退出 0；
  `/tmp/luna-tools-01zlbjae/result.json`。包含真实 sysroot 调用模板与相关流分析回归。

未运行全 workspace、native 或性能基准。materialization 和覆盖检查等仍有旧元数据
消费者；cleanup 的按分支字段视图已按下述迁移，不能据此宣称全链路布局服务完成。


cleanup 的名义布局检查也已退役无界 named-field／variant 实例化。旧实现只计费
未代入的字段形状；一个含 32 个字段的类型实参被复制到 64 个泛型字段时，即使
给定预算 2000 也返回成功。改动前回归 `/tmp/luna-tools-yxkoryri/result.json`
退出 101，表明源形状检查不能界定实参替换后的输出成本。

Enum 新增 `instantiate_variant_fields`：与平铺的 `instantiate_stored_fields`
共用一次有预算的参数帧准备，并保留每个原始变体位置及字段顺序，包括空分支。
它不复制名称或 shape，也不重新定义源码 tag／错误结果 ABI。cleanup 消费这个
分支视图；struct 消费平铺字段视图。每次替换通过同一 bounded substitution
预先计费；失败不交付部分布局，也不留可被后续成功复用的残缺查询结果。

回归同时检查 struct／enum 的展开成本、非法参数域拒绝、足额预算恢复，以及空
分支位置和嵌套调用者 scope。清理责任取得、析构函数选择、已移动字段过滤和
出口执行顺序仍由现有 cleanup 协议决定，本次没有用字段视图代替资源检查。


本次验证目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p program_verification --lib
  cleanup_layout_bounds_expanded_fields_and_preserves_variant_positions -- --nocapture`：
  1 通过，退出 0；`/tmp/luna-tools-yg1yn7ov/result.json`。
- `-p nir --test nominal_parameters --test phases --no-fail-fast`：65 通过，
  退出 0；`/tmp/luna-tools-bxsy8sbj/result.json`。
- `-p comptime_interpretor -p program_verification --lib --no-fail-fast`：VM 451、
  程序验证 105 通过，退出 0；`/tmp/luna-tools-0cndat2n/result.json`。
- `-p luna --test drop --test match_cleanup --test temporary_method_cleanup
  --test conditional_capture_cleanup --test invocation_value_templates --no-fail-fast`：
  后四组共 19 通过；Drop 在固定 2 MiB 栈的
  `optional_inline_call_chains_preserve_defaults_contracts_and_owned_returns`
  中栈溢出并 SIGABRT，整条命令退出 101；`/tmp/luna-tools-9hv6a0ce/result.json`。
- 保留最终测试、仅恢复本轮前 `64aab766` 的三个 Rust 文件后，以
  `-p luna --test drop optional_inline_call_chains_preserve_defaults_contracts_and_owned_returns
  -- --exact` 复现相同栈溢出，退出 101；`/tmp/luna-tools-rlwsjqx6/result.json`。
- 恢复最终源码，以 `-p luna --test drop --
  --skip optional_inline_call_chains_preserve_defaults_contracts_and_owned_returns`
  继续其余用例：995 通过、1 失败、1 过滤，退出 101；
  `/tmp/luna-tools-ydxugoj0/result.json`。失败项为
  `scheduled_handler_borrowed_copy_record_rejects_unproved_owner_layouts`，
  预期拒绝却成功。再将同样三个文件恢复至 `64aab766`，以该完整名称和
  `-- --exact` 单独复现相同失败，退出 101；`/tmp/luna-tools-m9hg5dnb/result.json`。

两次基线对照均在 finally 中逐字节恢复最终源码，并核对已有修改的摘要。
未修改 Drop 测试、扩大栈或放宽拒绝预期；两个基线缺陷仍未修复，不能将整组 Drop
报告为通过。未运行全 workspace、native 或性能基准。正式 docs 和原有工作区
修改未纳入本次交付，materialization／覆盖检查等元数据消费者仍待迁移。


## 27. 内存字段观察不再复制声明元数据

内存字段观察属于程序验证层的存储描述；它不能授予 Copy／Drop，不能证明指针
来源或替代资源取得，也不调用 PTC 证明内存命题。最终布局服务的环境与表示依赖
尚未统一，本次先消除现有消费者的不受限字段复制，而不新增另一份布局真相。

`memory::owned_fields`、数据／可读表示检查、pointer-free 检查及 effect 载荷检查
现在直接使用 NIR 的受限字段代入。它们共享调用者工作预算，代入前计算展开成本，
不再先检查未替换的字段和实参、随后无界展开。空 enum 分支也计入遍历成本；
即使字段集合为空，仍检查完整参数帧。newtype 的受限表示边和 NF 身份规则保持原职责。

`memory::owned_variants` 已删除，唯一替代入口 `owned_variant_fields` 返回按原分支
编号排列的字段类型。它不复制 enum 名称／shape，也不为可选类型和错误结果构造
假的 Variant 元数据。所有消费者同步迁移，包括指针叶、分支投影、模式载荷、
callable 路径和 moves 的 tag-domain 检查。可选缺失分支仍为空；错误结果成功分支
及错误行的逻辑编号不变；这不是物理 tag ABI 的重新定义。

基线 `147b2393` 加入相同四项行为测试后，三项失败：字段查询在预算 2000 下仍交付
64 份含 32 个字段的实参；两个表示检查在预算 16 下仍遍历 128 个空变体并成功。
`/tmp/luna-tools-q0supyf2/result.json` 退出 101；分支／指针 guard 的对照测试通过。
迁移后四项均通过，`/tmp/luna-tools-lu8nexwc/result.json` 退出 0。后续补充空载荷
非法参数拒绝和 effect 数据／code／指针边界检查，失败后均重新执行合法查询。

验证目录 `/home/fin/luna`，入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p program_verification --lib --tests --no-fail-fast`：111 项单元测试及 8 项集成测试
  通过，退出 0；`/tmp/luna-tools-e3kg7mko/result.json`。
- `-p luna --test effect_memory_transfers --test captured_memory --test method_memory_contracts
  --test handler_capture_lifetimes --test nominal_value_parameters --test invocation_value_templates
  --test drop --no-fail-fast --
  --skip optional_inline_call_chains_preserve_defaults_contracts_and_owned_returns
  --skip scheduled_handler_borrowed_copy_record_rejects_unproved_owner_layouts`：
  1525 通过、2 过滤，退出 0；`/tmp/luna-tools-ewc1hqhj/result.json`。两个过滤项是
  §26 已在改动前基线上复现的缺陷，本次未宣称修复，也未修改它们的测试源码。

未运行全 workspace、native 或性能基准。materialization 和覆盖检查仍有旧的元数据
代入消费者；moves 的后续迁移见下文。此迁移不代表完整 LayoutPlan 或全链路共享预算已完成。
正式 docs 与原有工作区改动未纳入本次交付。


moves 的名义类型观察也已使用同一受限代入：观察模式、record 分解、ownership shape、
callable 输入／返回路径、捕获恢复、临时投影及 pointer-origin 查询不再调用完整命名
字段／变体实例化。原 `callable_variant_layout` 退役为仅附加 SourceRange 诊断的
`callable_variant_fields`，不再自己遍历源字段并重复计费参数。enum 观察模式从受限
结果中移动所选字段集合，不额外复制该集合。程序验证 crate 已无
`instantiate_fields`／`instantiate_variants`（包括 in_scope 版本）的调用。

这次验证使用一个字段内部重复引用参数 64 次、参数自身含 32 个指针字段的输入。
它区别于多个字段各引用参数一次：原“每个字段检查一次实参”的估算仍会漏算此展开。
基线 `639424ee` 加上相同四项测试，变体观察、临时投影与 pointer-origin 三项均错误
接受预算 2000，`/tmp/luna-tools-mhaj3xso/result.json` 退出 101；callable 路径及指针
权限对照通过。额外回归遍历低预算失败到成功，检查 qualified 路径始终恢复，且
只由既有 tag 证据排除指针分支；域错误不转成“没有指针”，合法查询可随后成功。

迁移中一次编译检查发现将匿名 record 的 `(name, type)` 错当字段类型，
`/tmp/luna-tools-7t_adqss/result.json` 退出 101，已恢复该原始结构分支。随后
`/tmp/luna-tools-mi4d62qh/result.json` 的 1 项失败暴露旧错误适配器用 Debug 输出
`Budget`；三处迁移入口已改用错误的 Display，保留预算原因及源位置，不靠调整
测试字符串隐藏差异。上述两轮不是最终通过证据。

字段视图不授予 Copy，不创建 code identity，不改变实际分支 guard、析构责任或
pointer contract。ownership shape 的预算仍由现有分析入口单独供应；此阶段不声称
整个 moves 的预算与内存／资源证明都已统一。正式 docs 与原有修改保持不变。

验证目录 `/home/fin/luna`，统一入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p program_verification --lib --tests --no-fail-fast`：117 项单元测试与 8 项集成测试
  通过，退出 0；`/tmp/luna-tools-r8s6ydfc/result.json`。
- `-p luna --test drop --test match_cleanup --test temporary_method_cleanup
  --test conditional_capture_cleanup --test handler_affine_captures --test nominal_value_parameters
  --test newtypes --test invocation_value_templates --no-fail-fast --
  --skip optional_inline_call_chains_preserve_defaults_contracts_and_owned_returns
  --skip scheduled_handler_borrowed_copy_record_rejects_unproved_owner_layouts`：
  1537 通过、2 过滤，退出 0；`/tmp/luna-tools-13ps23iz/result.json`。
  两个过滤项仍是 §26 的既有缺陷，未改测试源码或宣称已解决。

未运行全 workspace、native 或性能基准。格式检查通过；已有工作区修改的摘要
保持不变，提交范围限于 moves 生产迁移、行为回归和本开发记录。


## 28. 具体名义实例按所需元数据构造

materialization 的 struct／enum 先前使用完整字段实例化，随后 clone 整个源声明、
覆盖字段和契约并清空参数。dispatch 探索也复制名称和 shape，尽管只使用字段类型。
现在两条路径明确分开：dispatch 使用已有 payload-only 视图；具体 IIR 使用
`instantiate_definition_bounded` 直接构造专门化声明，原始泛型字段与契约不再
先复制后丢弃。这一通用接口仍不执行 NF 头，也不建立类型形成、Copy 或目标 ABI 证明。

完整实例保留声明身份、名称、字段顺序、enum groups／shape／intrinsic 标记及既有
位／Symbol 表示。参数 telescope 在代入后清空；字段、requirements、capabilities
通过同一个受限 DeclarationSubstitution 代入。字符串、集合槽位、位表示的两个
word buffer 及 Symbol 值在复制前计费。字段视图不顺带观察这些元数据，因此仍能
用较小预算查询一个编码很大的 enum 的空载荷。

materialization 随后执行原有 normalization 和形成检查；准备依赖队列的字段副本
另外预先计费。形成要求的代入也在复制前检查展开成本，struct／enum 的 capability
依赖通过 bounded requirement 代入供应。发布仍属于现有 Builder 事务：错误返回
不交付部分 IIR，后续调用重新构造私有 candidate。

基线 `ff4a77dc` 在预算 5000 下仍复制十万字符的 struct 名称并成功发布：
`/tmp/luna-tools-2n68dnl9/result.json` 退出 101。修复后的同一集成测试同时覆盖
struct 名称及 enum 变体名称，低预算拒绝后以足额预算成功、元数据保持一致；
`/tmp/luna-tools-w6dk52e1/result.json` 退出 0。

NIR 回归覆盖契约代入、参数清空、NF 头保留、空分支与 named shape、非法参数后的
恢复、位编码及长 Symbol 的复制预算。首次构造宽 BitVec 测试时错误传入空 words，
`/tmp/luna-tools-lk7wuisn/result.json` 退出 101；修正测试为宽度要求的完整 256 个
word，没有放宽 BitVec 的 exact representation 检查。

验证目录 `/home/fin/luna`，入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --test nominal_parameters --test enum_encoding --no-fail-fast`：38 通过，
  退出 0；`/tmp/luna-tools-ell4lhue/result.json`。
- `-p materialization --lib --tests --no-fail-fast`：22 通过，退出 0；
  `/tmp/luna-tools-dv80d0yy/result.json`。
- `-p luna --test nominal_value_parameters --test nominal_staging --test newtypes
  --test enum_groups --test enum_configurations --test encoded_enums --test integer_formation
  --test invocation_value_templates --no-fail-fast`：567 通过，退出 0；
  `/tmp/luna-tools-oeoz2xmq/result.json`。包含真实 sysroot 调用、形成拒绝与编码保持。

未运行全 workspace、native 或性能基准。elaboration 中的旧命名元数据消费者仍待
清理；newtype 的完整定义构造按下述继续迁移，原有表示检查未被绕过。本次不是完整 LayoutPlan、
全阶段统一预算或所有发布协议已完成的声明。正式 docs 与既有修改未纳入提交。


newtype 已接入同一完整实例构造协议。`representation_substitution` 是表示查询和
完整实例共享的参数准备入口，检查 capture_count 与表示的 Type sort；原来的
`representation_budgeted` 继续只返回表示，不复制名字或契约。完整实例则在同一
受限 substitution 下代入 representation、requirements、capabilities，并单独计费
名称，完成后清空已供应 telescope 与 capture_count。

捕获的身份参数顺序没有改变：自己的值参数仍可在前，提供其域的词法 Type 捕获仍
位于后缀；completed admission 保留先检查捕获域的既有规则。实例保存原 newtype
声明身份，显式表示里的 NF 头仍不展开，也不把表示观察当成类型相等或转换权限。
调用者 frames 继续显式供应，缺少 frame 的开放 Type 仍拒绝。

materialization 已删除 newtype 的整个声明 clone／覆盖路径。具体 IIR 的表示依赖
从 normalized representation 作受预算约束的复制；dispatch 只查询表示。newtype
capability 依赖也改为 bounded requirement 代入，不再用未计费的普通代入补队列。
struct／enum／newtype 的具体声明均使用受限构造，仍须经过原有形成与发布检查。

将上一轮元数据发布回归扩展到 newtype 后，基线 `fef2c07f` 在预算 5000 下仍接受
十万字符名称，`/tmp/luna-tools-nmgb7ajx/result.json` 退出 101。迁移后同一测试
三种声明均通过，`/tmp/luna-tools-zs_nz0c2/result.json` 退出 0。
NIR 新增检查覆盖捕获后缀、依赖域、契约与表示一致、NF 头保留、非法 capture_count／
表示 sort、开放 frame、名字预算，以及失败后恢复；源声明始终保持不变。

验证目录 `/home/fin/luna`，入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --test nominal_parameters --test newtypes --no-fail-fast`：52 通过，退出 0；
  `/tmp/luna-tools-f589j2ql/result.json`。
- `-p materialization --lib --tests --no-fail-fast`：22 通过，退出 0；
  `/tmp/luna-tools-reszj1w7/result.json`。
- `-p luna --test newtypes --test nominal_value_parameters --test nominal_staging
  --test invocation_value_templates --no-fail-fast`：535 通过，退出 0；
  `/tmp/luna-tools-ub3zffep/result.json`。包含捕获／值索引源码与真实 sysroot 调用。

未运行全 workspace、native 或性能基准；正式 docs 和原有工作区修改保持不变。
elaboration 的旧元数据观察仍未迁移，完整 LayoutPlan 与跨阶段共享预算仍未完成。


## 29. 覆盖矩阵的字段观察与 Unknown 边界

覆盖检查是已检查 runtime pattern 的矩阵分析，不是 PTC 命题。它可以使用已有
Context 的类型域与 binder frames，但不能把 runtime guard 或内存状态提交给 PTC。
名义 struct／enum 现在直接通过受限字段代入观察载荷，共享矩阵的 steps；不再
复制随后丢弃的字段名称、变体名称和 shape。既有分支编号、匿名 record 标签匹配
以及矩阵深度限制保持不变。

否定模式的 `complement_fields` 改用带错误结果的选中载荷查询，不再把失败转为
Option::None。该私有接口交付 `Result<Vec<Term>, ()>`：合法空载荷是空 Vec，
查询失败使本次矩阵返回 Unknown；不根据剩余 fuel 是否为零猜测成功。collection
域证据和 nominal 值参数准入仍由同一查询检查。未知的可执行 guard 继续走原来的
保守覆盖逻辑，不因此获得 complement 证据。

elaboration 的 readable-storage 形成检查也迁移到受限字段视图；它仍不授予 Copy
或指针来源权限。既有局部观察预算和后续资源检查尚未变成全阶段统一预算协议。
构造器、具名字段模式和 enum 成员解析仍有需要元数据的旧消费者，未在本轮退役。

基线 `37612c1c` 中，64 次参数引用乘以 32 字段实参的矩阵在预算 2000 下仍展开并
返回 Exhaustive；否定模式布局失败仍返回成功的 None。两项回归失败，
`/tmp/luna-tools-57plwugk/result.json` 退出 101。迁移后预算失败为 Unknown，足额
预算恢复 Exhaustive；正模式与其 complement 的组合在逐档预算检查中从不产生
Missing。额外检查了正余额下预留失败、合法空载荷、开放 Type frame 及缺失 frame。

新增测试最初要求大展开无模式矩阵在预算内得到 Missing，实际还触及原有矩阵
深度限制，`/tmp/luna-tools-0gqjidu6/result.json` 退出 101。Missing 对照改用同一
声明的较小实参；未放宽深度限制或将 Unknown 改判为 Missing。

验证目录 `/home/fin/luna`，入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p elaboration --lib coverage:: -- --nocapture`：72 通过，退出 0；
  `/tmp/luna-tools-172hctqw/result.json`。
- 加入开放 frame 与逐档 complement 检查后，`-p elaboration --lib --no-fail-fast`：
  168 通过，退出 0；`/tmp/luna-tools-754kd6in/result.json`。
- `-p luna --test negated_nominal_patterns --test normal_patterns --test inline_pattern_observation
  --test enum_groups --test newtypes --test nominal_value_parameters --test invocation_value_templates
  --no-fail-fast`：621 通过，退出 0；`/tmp/luna-tools-0px1bo4d/result.json`。

格式检查通过。未运行全 workspace、native 或性能基准；正式 docs、已有工作区
修改与未迁移的构造器元数据入口保持原状。


## 30. 具名字段查询退役与构造器预算

构造器与具名模式需要字段名和变体 shape；存储观察只需要载荷，具体实例发布才
需要完整声明。三种视图现在共享同一个 declaration substitution 内核，并分别
承担实际复制的成本。`Struct::instantiate_fields_bounded` 与
`Enum::instantiate_variants_bounded` 显式接收 caller frames 和 fuel；字段名、
变体名、具名 shape 与代入后展开的字段都受预算约束，声明名称、groups、编码与
契约不属于具名载荷视图。NF 头身份和已有 scope/domain 检查不变。

删除 `instantiate_fields`、`instantiate_fields_in_scope`、`instantiate_variants`
和 `instantiate_variants_in_scope` 四个无界入口；生产源码不再调用它们。既有
名义参数、开放 scope 和反射测试迁移到强制预算接口，不保留默认无限预算包装。

elaboration 的构造器、字段模式与 enum 成员观察扣减既有 `required_steps`。
失败前已执行的工作也计入消耗，Budget 不等于余额必须为零。只验证 enum 参数和
载荷时使用无名称的存储视图；真正路由变体时使用具名视图。选中变体直接从返回
容器移出，不再索引后复制一份。struct 构造器不再复制整份声明，只在有界检查后
保留默认字段代入所需的参数身份和域快照。

这仍不是统一的全阶段预算所有权协议。默认字段表达式本身仍经过既有
`instantiate_declaration_parameters_in_scope` 复制与重写，尚未迁移为有界表达式
代入；成员选择等前置元数据操作也未全部纳入本轮。这里的载荷观察不替代完整值域
准入、形成检查、资源证明或布局计划。

新增检查覆盖长字段名／变体名／具名 shape 的正余额预留失败、无关声明元数据不
进入载荷预算、64 次参数引用乘以 32 字段实参的展开限制，以及失败后足额预算
恢复。构造器入口单独检查预算扣减与参数头保留。新增测试首次将匿名 record 的
Rust 表示误写为纯 Term 列表，编译失败于 `/tmp/luna-tools-9dja_079/result.json`
（退出 101）；改为具名字段对后重跑，未改生产规则来迎合测试。

验证目录 `/home/fin/luna`，统一入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --test nominal_parameters --test scoped_contracts --no-fail-fast`：
  38 通过，退出 0；`/tmp/luna-tools-1moa1h6c/result.json`。
- `-p elaboration --lib --no-fail-fast`：170 通过，退出 0；
  `/tmp/luna-tools-85er6__x/result.json`。
- `-p comptime_interpretor --lib reflection::tests --no-fail-fast`：12 通过，退出 0；
  `/tmp/luna-tools-3s5jpy1o/result.json`。
- `-p luna --test newtypes --test nominal_value_parameters --test enum_groups
  --test enum_configurations --test normal_patterns --test negated_nominal_patterns
  --test invocation_value_templates --no-fail-fast`：613 通过，退出 0；
  `/tmp/luna-tools-avu15l0k/result.json`。

受影响 Rust 文件格式检查通过；未运行全 workspace 或性能基准。正式 docs 与
原有工作区修改未纳入本次变更。


## 31. 默认表达式与程序规格的声明代入

声明拥有的 checked expression 现在通过
`DeclarationSubstitution::instantiate_expression_bounded` 复制和代入：先检查
完整展开树的成本，再复制私有树、重写其中的类型。预检包括共享闭包体、模式
条件及隐藏的类型操作数；不是只计顶层类型，也不是只计展开前的实参体积。
失败不发布局部重写结果，原始模板保持不变。已有 binder frame、dependent domain
成为 Type 时的 sort 检查和 ComptimeValue 到 TypeValue 的重写继续沿用统一机制。

删除表达式上的 `instantiate_declaration_parameters_in_scope` 无界入口，改为
强制 caller frames 和 fuel 的 `instantiate_declaration_parameters_bounded`。
struct／enum 默认字段、上下文推导后的 enum 默认值与 associated initializer 的
五处生产消费者均接通既有 elaboration 预算；失败前消耗照常扣减。字段默认值缓存
取出与首次写入也在复制前预检。associated initializer 原来只预检原模板的做法由
展开预检替代，不再漏掉参数重复出现造成的复制增长。

`ProgramContract` 复用同一个表达式代入内核，删除重复 binder 构造和复制逻辑。
这只是程序规格的结构变换，不把程序状态命题送入 PTC，也不把形成检查、验证或
associated implementation 选择当作结构代入的一部分。

新增用例验证 64 次引用乘以 32 字段实参的预算失败与恢复、共享闭包体在复制前
计费且不改变原 Arc、开放 caller frame 经嵌套函数类型的深度调整、缺失 frame
拒绝，以及已有 dependent value 成为 Type 的行为。首次脚本替换错误跨越了构造器
代码边界，格式检查 `/tmp/luna-tools-nk3tirq1/result.json` 退出 1；使用本阶段修改前
快照重新应用限定范围的修改后恢复，未覆盖其他人的改动。

这次退役仅覆盖 declaration-owned expression 接口。旧的按固定 sort 参数／Type
进行表达式或完整函数代入仍有消费者，不能据此声称所有模板复制都已受限。
输入域获取、默认表达式形成与元数据取得的前置复制，以及全阶段预算所有权，
也仍需继续整理。

验证目录 `/home/fin/luna`，统一入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --test calls --test copy_budget --test type_rewrite --no-fail-fast`：
  45 通过，退出 0；`/tmp/luna-tools-zohk9ngp/result.json`。
- `-p elaboration --lib --no-fail-fast`：170 通过，退出 0；
  `/tmp/luna-tools-402vjsfh/result.json`。
- `-p luna --test nominal_value_parameters --test default_calls --test associated_signatures
  --test deferred_associated_values --test associated_purity --test newtypes --test enum_groups
  --no-fail-fast`：591 通过，退出 0；`/tmp/luna-tools-83c57c4o/result.json`。

格式检查通过。未运行全 workspace、native 或性能基准；正式 docs 和原有工作区
修改保持不变。


## 32. 完整函数模板的预检所有权

`Function::instantiate_declaration_parameters` 原来要求调用者另行计费，并对纯 Type
输入走无界的严格代入分支。这样，即使接口显式接收 fuel，直接调用仍可能在低预算
下复制任意长名称、函数体、前后条件和默认值；载荷未使用的实参也不能证明整个
函数复制是受限的。新增两项直接接口回归在 `d324b704` 基线上失败：
`/tmp/luna-tools-u1v_fpgr/result.json`，退出 101。

现在该函数接口拥有整个展开后的复制预检，包括声明元数据、签名、类型命题、
capability、程序契约、默认值与共享闭包体。纯 Type 与 dependent-domain 输入均
通过 DeclarationSubstitution；普通调用输入 frame 只用于主体与契约，签名仍按
关闭的外层 scope 代入。调用者继续负责实际值域准入、形成义务、value-local 替换
与 staged 参数消除，结构代入不授予任何资源权限。

materialization 的普通函数选择统一使用此入口；已完成的 staged function 仍只做
复制预检，不重复做声明参数代入。quote fn 展开与文本区域的函数摘要分析删除了
原有只扫描 expr.ty 的不完整复制预检，使用内核的完整预检。前者仍是 quote fn，
不因共享代入设施变成 comptime fn；后者仍属于程序分析，不向 PTC 注入内存事实。

新增检查同时覆盖无参数和带 bool 参数模板的各个 payload、Type 实参重复展开、
正余额下预留失败、失败后足额恢复及原模板不变。另验证普通 Type 输入 frame 在
主体、默认值和前置条件中保留，非法 frame 索引拒绝后能够恢复。固定 sort 的旧
`Function::instantiate_parameters` 与其他模板消费路径仍未完全退役；这次移除了
主 dispatch 中对该入口的调用，不代表整个模板系统或预算所有权改造已经完成。


验证目录 `/home/fin/luna`，统一入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --test copy_budget --test calls --test scoped_contracts --no-fail-fast`：
  39 通过，退出 0；`/tmp/luna-tools-wotc551p/result.json`。
- `-p materialization -p elaboration -p program_verification --lib --no-fail-fast`：
  303 通过，退出 0；`/tmp/luna-tools-0qtko0ie/result.json`。
- `-p luna --test quote_functions --test cmtt_methods --test cmtt_type_inputs
  --test text_regions --test nominal_value_parameters --test newtypes --test default_calls
  --test invocation_value_templates --no-fail-fast`：899 通过、1 失败，退出 101；
  `/tmp/luna-tools-jd4ycivk/result.json`。

源码组的唯一失败是 `code_method_parent_value_clauses_form_indexed_code_signatures`，
诊断 `139..143: expression elaboration requires a protocol or construct not yet implemented`。
临时恢复本阶段修改前的五份实现文件，在 `d324b704` 基线上精确运行该用例，同样
失败于上述诊断：`/tmp/luna-tools-axb96zfg/result.json`，退出 101。随后逐字节恢复
当前实现。未修改用例或将它标为 ignored；本轮不声称整个 CMTT 组已通过。

`-p materialization --test expansion_boundary --no-fail-fast`：6 通过，退出 0；
`/tmp/luna-tools-gmm6xmz1/result.json`。补充正余额断言后精确重跑两项预算回归，
`/tmp/luna-tools-y97xkmhn/result.json`：2 通过，退出 0。

受影响 Rust 文件格式检查通过；未运行全 workspace、native 或性能基准。正式 docs
与原有工作区修改保持不变。


## 33. Invocation 代码实例与表达式代入内核

源码 VM 的 closure 实例实际保留签名、参数身份和主体，程序契约与 capability 仍在
调用路径检查。该生产者不再预检随后不复制的整份 Function；签名／参数身份单独
受限，主体、实例默认表达式和 invocation condition 共享声明表达式代入内核。
完整函数发布继续使用第 32 节的完整视图，两者不能互相冒充。

内核返回 `ExpressionInstantiationError`，区分复制步数、复制深度与 scope 失败。
VM 将它们分别映射为 StepsExhausted、DepthExhausted、Type；剩余预算为正不代表
查询成功。原有 NIR 表达式／ProgramContract 接口通过显式转换保留其 Budget／Scope
契约。程序规格依然属于程序验证层，公共内核只执行结构变换。

去掉整份函数预检后，逐个 capability 的前置 clone 也必须删除，避免重新暴露
未计费复制。函数身份准入和代码实例准备现在直接借用 requirement，以同一个
scoped substitution 完成受限代入；函数 frame 在分配前计费。类型命题和 capability
在实例缓存命中时仍检查，缓存不承担当前调用义务。没有类型代入的 condition
使用普通复制预检，共享代码体按真实 clone 行为保留共享。

新增 VM 检查覆盖深度失败、正余额下步数失败、scope 失败及随后恢复；失败没有
向实例缓存发布条目。另以很长但 closure 不保留的声明名称验证所选代码视图，
确认重复准备共享同一个 Arc、只产生一次实例化事件，缓存存在时错误实参仍被
拒绝且合法实参仍能使用原实例。这里只验证准备好的代码缓存，不能推导为任意
invocation 结果都可以缓存。

源码默认值的固定 sort 代入、IIR 中的实例复制和其他签名观察入口仍有各自的
消费者，尚未全部收敛；本轮没有完成跨阶段稳定 world、全局预算或实例发布协议。

验证目录 `/home/fin/luna`，统一入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p comptime_interpretor --lib --no-fail-fast`：453 通过，退出 0；
  `/tmp/luna-tools-_klz6gte/result.json`。
- `-p nir --test calls --test copy_budget --test scoped_contracts --no-fail-fast`：
  39 通过，退出 0；`/tmp/luna-tools-rz6h6lel/result.json`。
- `-p luna --test prepared_invocations --test staged_invocation_reuse
  --test invocation_value_templates --test comptime_bytecode_defaults --test default_calls
  --test nominal_value_parameters --test newtypes --no-fail-fast`：583 通过，退出 0；
  `/tmp/luna-tools-wuh_2_c7/result.json`。

受影响 Rust 文件格式检查通过；未运行全 workspace 或性能基准。正式 docs 和
原有工作区修改保持不变。


## 34. IIR callable 查询与代码复制视图

IIR 的 `iir_callable` 在取得函数定义、计算实例键 hash 或查询本地实例缓存之前，
先检查键的结构成本与深度；无参数键也至少消耗一次查询预算。原来热缓存直接
返回，零预算亦可成功；未知但很宽／很深的键则先经过 graph lookup，过晚才有
机会检查。现在预算不足与结构深度失败先于查询，足额预算下不存在的键仍报告
UnknownFunction；已缓存 closure 的 Arc 身份与合法重试保持不变。

`copy_budget::charge_callable_code` 只预检 closure 真正拥有的签名、参数身份和
主体。它不复制声明名称、默认模板或契约，嵌套 closure body 按 Arc clone 行为
保留共享。IIR 的完整实例验证仍在 graph 接收边界，程序契约继续在 invocation
路径检查；这个复制视图不是验证证据，也不改变 callable phase 或运行权限。

两项新回归在 `ff7194b2` 基线上失败：热缓存零预算返回成功，长而未保留的声明
名称耗尽代码复制预算；`/tmp/luna-tools-veiha0gl/result.json`，退出 101。修复后
覆盖热缓存计费、宽／深未知键在 hash 前拒绝、足额未知键分类、失败后恢复、长
名称下的正常调用，以及 signature／参数列表／主体的复制成本和共享 body 保留。

其他 graph 查询仍有各自的准入与计费前置条件，源码实例路径也尚未形成统一的
键查询证据对象。这次只闭合 IIR callable 生产入口，不能据此宣称所有 graph
lookup、CodeSession 生命周期或整个预算系统已经统一。

验证目录 `/home/fin/luna`，统一入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --test copy_budget --no-fail-fast`：23 通过，退出 0；
  `/tmp/luna-tools-wxaix7oy/result.json`。
- `-p comptime_interpretor --lib --test instance_budget --test runtime_function_identities
  --no-fail-fast`：库测试 455 通过，集成测试 51 通过、2 失败；退出 101，
  `/tmp/luna-tools-4xbbuc7r/result.json`。
- 补上深键检查后，`-p comptime_interpretor --lib iir_execution::callable_budget_tests
  -- --nocapture`：2 通过，退出 0；`/tmp/luna-tools-6tbodrc4/result.json`。
- `-p luna --test prepared_invocations --test staged_invocation_reuse --test default_calls
  --test nominal_staging --test quote_functions --test newtypes --no-fail-fast`：
  379 通过，退出 0；`/tmp/luna-tools-324detyv/result.json`。

两个集成失败分别为 `declaration_tables_are_bounded_before_world_indexing_and_phase_validation`
与 `source_session_constructor_rejects_duplicate_ids_in_another_snapshot`。它们期待
PhaseMismatch，实际重复声明被拒绝并返回携带 `duplicate function declaration`
诊断的 InvalidPhase。恢复本阶段修改前的两份实现文件，在 `ff7194b2` 基线上重跑
这两个完整测试目标，仍为 51 通过、2 个相同失败；
`/tmp/luna-tools-cn10fng6/result.json`，退出 101。随后恢复当前实现，当时未修改这两项
测试或屏蔽诊断。后续审查确认它们残留旧的 PhaseMismatch 断言，现已依照
[构造期诊断与 session 验收协议](values-staging-and-publication.md)修正为具体原因、
源码传播、重复拒绝与恢复检查，两份完整集成组 53 项通过；生产拒绝规则未放宽。

格式检查通过；未运行全 workspace、native 或性能基准。正式 docs 与原有工作区
修改保持不变。


## 35. Phase 存储缓存与构造预算的后续边界

phase validator 原来先 clone 类型和 frames 组成缓存键，再调用带内部固定额度的
bool 存储观察；错误都折叠成 inspection limit。现已删除这条 bool 包装入口，生产
遍历使用绑定到一个不可变 Package 借用的 `RuntimeStorageQueries`，复用
`validate_runtime_storage_inner` 的 Type／scope 和载荷规则。

每次查询先计入 frame 宽度、类型结构与键成本，然后借用键查询。仅成功或确定的
Invalid 入缓存，Budget 不入缓存；不同 frames 不共享结果，不同世界使用各自
查询对象。同一次查询的键检查、存储观察与字段展开消费同一 fuel。内核保留
StorageError，phase 适配器保留原位置与具体诊断；它的公共 Error 仍没有单独的
预算 variant，不能说 VM 构造错误分类已全部迁移。

生产回归发现旧 phase validation 会接受没有输入 frame 的 Bound 表达式类型。
在 `baa71adb` 基线上，新拒绝例得到 Ok，
`/tmp/luna-tools-hn9_h12u/result.json` 退出 101；恢复当前实现后拒绝并报告原表达式
位置，修复为合法类型后可通过。缓存用例另外检查展开超预算后不发布条目、足额
重试、热命中仍计费、正余额 frame 预留失败，以及同号名义声明在不同世界的不同
存储结论。NF 头仍不因这个查询被展开，内存规格不进入 PTC。

本次没有把各个固定额度简单包进一个名义上的总预算。当前 phase validator 还有
parameter-domain 目录、默认值 free-binding 分析、trait membership、调用准备、
comptime 值准入和 call-safety 的独立额度；当前存储适配器也仍按单次查询供给
100,000。这些是尚未完成的构造期预算所有权迁移，不是语言承诺。

后续迁移应让一次 phase-validation context 共同拥有调用者额度与本世界缓存：
参数目录先以 typed error 准入；默认绑定、成员、表达式、存储和值准入等子查询
消费同一额度；拒绝保留原诊断，资源不足单独返回，禁止依靠错误文本推断耗尽。
只有整个验证成功才可随 SourceIndexes 发布验证证据。构造预算的保守失败预留
应在所有失败路径能回传实际消耗后再撤销，不能只在某个成功分支返还额度。
现有 runtime/IIR 资源检查依旧由各自阶段负责，不由此 context 替代。

验证目录 `/home/fin/luna`，统一入口 `python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --lib --test phases --no-fail-fast`：53 通过，退出 0；
  `/tmp/luna-tools-0cmywcj4/result.json`。
- `-p comptime_interpretor --lib --no-fail-fast`：455 通过，退出 0；
  `/tmp/luna-tools-75btbzza/result.json`。
- `-p luna --test comptime_storage_domains --test comptime_pointer_phase --test trait_phases
  --test quote_functions --test nominal_value_parameters --test newtypes
  --test cmtt_phase_boundaries --no-fail-fast`：712 通过，退出 0；
  `/tmp/luna-tools-lc782jus/result.json`。

格式检查通过；未运行全 workspace 或性能基准。正式 docs 与原有工作区修改未动。


### 绑定分析的错误边界

`free_bindings` 的表达式、pattern、arm 及符号依赖入口现在共同返回 typed error：
Budget 表示步数预留失败，Depth 表示分析递归上限，Invalid 表示已有的作用域或
输入计划拒绝。错误携带原诊断，但分类不取决于文本。文本诊断消费者可以在自身
边界转换为 String；原有分析实现已直接迁移，没有保留另一套字符串分析内核。

不可变 expression walker 共用一个支持 typed callback 的遍历内核，遍历本身的
Budget／Invalid 与 callback 的错误通过显式转换衔接。现有字符串 visitor 接口
仍作为适配器使用这个内核。此修改没有改变遍历顺序、词法绑定规则或资源限额；
可变 visitor 及其他字符串检查服务尚未整体迁移。

三条生产路径已接通：phase 默认值分析不再搜索 `exhausted`，并区分调用默认值的
作用域拒绝和资源耗尽；Type invocation 的 expression/captured entry 构造将分析
和注解遍历的资源失败保留为 Budget；字节码模板依赖分析将深度上限保留为 Depth，
不再误报 Unsupported。entry 的既有 Error 没有细分步数／深度，因此在该边界二者
仍合并为 Budget。phase 公共 Error 仍是位置和诊断，单次独立额度也仍存在；这只是
共享 phase-validation context 的必要前置迁移，不能宣称总预算所有权已经闭合。

三条生产回归在 `9038fb04` 的实现上均失败：调用默认值只给出含糊的合并诊断，
entry 的零预算成为 Structure，模板依赖深度无法得到 Depth。
`/tmp/luna-tools-2lnanfvt/result.json` 退出 101；随后恢复当前实现。新用例还检查
从零到足额的预算梯度、失败后的成功重试、正余额预留失败、循环依赖的深度上限、
原位置保留，以及诊断文本不能改变 callback 的明确错误分类。


本次验证目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --test free_bindings --test type_computations --test phases
  --test closure_environment_visit -p comptime_compiler --test control --no-fail-fast`：
  106 通过，退出 0；`/tmp/luna-tools-9v9k8s22/result.json`。
- `-p comptime_interpretor -p elaboration -p program_verification -p materialization
  --lib --test instance_budget --test runtime_function_identities --no-fail-fast`：
  811 通过，退出 0；`/tmp/luna-tools-fm2urhy3/result.json`。
- `-p luna --test default_calls --test quote_functions --test cmtt_type_inputs
  --test comptime_bytecode_closures --test sysroot_preludes --test nominal_staging
  --no-fail-fast`：201 通过，退出 0；`/tmp/luna-tools-t8pqj5mr/result.json`。

首次局部构建发现两项旧测试对错误调用 String::contains，已改成精确的 Invalid
断言；`/tmp/luna-tools-gvxlrhvm/result.json` 退出 101。一次检查命令重复指定
`--lib` 被 Cargo 拒绝，退出 1；`/tmp/luna-tools-dt2_ztxf/result.json`。以上均不作为
通过证据。格式检查通过；未运行全 workspace、native 或性能基准。正式 docs、
用户原有源码和 library 修改保持不变。


### 参数校验预算的归并

phase validation 的声明参数目录、alias formation contracts、源码函数和 quote
模板的 frame／默认值分析现在共享 pass 提供的一份参数额度；子服务没有自行
补充 fuel。当前兼容入口为这部分分配 1,000,000，并不表示整个 phase validator
或 VM 构造已使用调用者总预算。独立的 trait membership、NF 元数据校验、生成的
trait summary、主体准备、存储和 call-safety 仍待迁移；对外 Error 仍为原位置和
诊断，typed 参数 Failure 只在内部保留。

参数目录改用 checked 入口，Budget 不再被翻译成“声明域无效”。目录遍历对没有
输入的声明也计费，防止空 telescope 集合绕过额度。完成目录后，phase 不再为
struct、enum 和 implementation 重复构造同一 binder 集合；enum encoding 等
独立检查保留。参数 frame 先预留参数／默认值列表及 signature 观察成本，再建立
索引；单一位置索引同时负责重复身份与默认值的前向引用检查。frame 仅借用 NIR，
不复制默认表达式，也不改变 quote 与 runtime 的执行规则。

alias contracts 的 `validate_in` 返回 Budget／Invalid 并使用调用者额度；旧
`validate` 仅保留固定额度的诊断适配。形成义务的 term、predicate 和 capability
复用一个有界工作栈，展开子数组前预留成本，移除了两个重复的 term 校验路径。
按 binder 身份建立的排序索引同时检查 sort 和声明先后，替代逐个引用线性扫描
整个 telescope；这不是新增类型推导，也不把程序验证命题引入 PTC。

生产参数 pass 的回归验证一份精确额度可完成目录、alias、quote 和多个函数，少
一步在后一个函数的默认值处失败并保留源位置，足额重试成功。另两项公共入口
回归在 `5814cfc2` 基线上失败：空声明目录在不足额度时仍返回 Ok，宽声明域的
资源失败仍被报告为无效元数据；`/tmp/luna-tools-ust9ixm1/result.json` 退出 101。
随后恢复当前实现，未修改既有语义断言或扩大通过条件。


计入空声明成本后，首次真实 sysroot 验收出现 11 项 P4 拒绝；
`/tmp/luna-tools-e47rox3q/result.json` 退出 101。临时诊断定位到
`rejection::Cache::query -> declaration_parameter_sorts`：同一不可变 Package 的
每个冷实例都重建整包目录，资源检查在重复工作中耗尽额度，最后表现为
`mix_digest` 缺少 P4 证据。单例诊断 `/tmp/luna-tools-o043vd2o/result.json`
退出 101 并保留调用栈；诊断插桩随后全部撤销，purity.rs 恢复原内容。

只在单个 rejection cache 内保存目录仍不足以修复 sysroot：每个独立资源分析
都会新建 cache；`/tmp/luna-tools-u_evobs_/result.json` 再次退出 101，同样为 11 项
P4 拒绝。这证明仅缓存冷查询结果没有消除不必要的观察。

最终路径按实际依赖取得声明 sort：空 telescope 不查询整包目录；遍历到有声明
输入的被调函数时才构建目录。其成功结果保存在本次 Package 借用内，与已有的
函数索引共享生命周期；每个新实例的扫描和 cleanup 证明照旧独立计费。目录
构造失败不发布条目，不保存部分结果；不同 Package 使用独立 cache。回归覆盖
低额度失败后的足额重试、不同 root 复用目录时仍计费、新世界同号函数的不同
拒绝行为，以及无效目录重复拒绝。另有闭合调用进入泛型调用边的用例，保证延迟目录取得
不会跳过后来需要的声明证据。这是消除不必要和重复的观察，未扩大 P4 预算或
豁免任何资源义务；全局 world/parameter query 服务仍待后续归并。


本批验证目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --lib --test phases --test trait_parameters --test type_aliases
  --test nominal_parameters --no-fail-fast`：118 通过，退出 0；
  `/tmp/luna-tools-pkmzs4bg/result.json`。
- `-p comptime_interpretor -p elaboration -p materialization --lib
  --test instance_budget --test runtime_function_identities --no-fail-fast`：
  694 通过，退出 0；`/tmp/luna-tools-r6pk_gjz/result.json`。
- `-p program_verification --lib --no-fail-fast`：119 通过，退出 0；
  `/tmp/luna-tools-0wj500ra/result.json`。
- `-p luna --test default_calls --test quote_functions --test nominal_value_parameters
  --test trait_phases --test sysroot_preludes --no-fail-fast`：543 通过，退出 0；
  `/tmp/luna-tools-382ohk1h/result.json`。其中真实 sysroot 全部 15 项通过，覆盖此前
  11 项 P4 回归；未通过扩大纯度预算或更改库实现绕过拒绝。

格式检查 `/tmp/luna-tools-nj8ep5ng/result.json` 退出 0。未执行全 workspace 或
性能基准。正式 docs、用户已有的资源源码修改及 library 修改保持不变。


### Phase 成员类型观察的有界替换

associated value 与 trait method 的 phase 路径现在使用同一次 preparation fuel
支付实参复制、声明域复制和结果展开；不再为 associated value 的 binders 单独
重置 100,000，也不再调用无预算的成员类型替换。实参即使没有出现在结果类型中，
其结构也在复制前计费。associated value 复用 trait 的普通域检查 substitution，
method 复用 `DeclarationSubstitution::bounded`，不展开 NF 头、不执行 Type 函数，
也不改变 trait/implementation 选择与程序契约责任。

`MethodSignature::instantiate_declaration_inputs_bounded` 为 Callable、Generator、
PatternGenerator 和 DataConsumer 提供同一入口。先检查签名携带的上下文、port
名称、默认连接和普通输入 binder，再按通用规则预留每个替换后类型的成本；失败
丢弃私有结果。不能只计算 type term，而让 port 名称或 input binder 元数据绕过
复制预算。contextual signature 的类别与固定依赖保持不变。

生产回归以合法 Symbol 域的大实参验证未使用输入也受限。在 `6e264e14` 的 phase
实现上，该调用返回 Ok，`/tmp/luna-tools-hxz7lcbu/result.json` 退出 101；恢复当前
实现后返回带原位置的资源诊断，缩小输入后可重试成功。签名测试另外覆盖重复
参数导致的展开膨胀、开放 scope 拒绝／接受，以及 contextual port、默认连接和
opaque NF 身份的保留。

这次没有把 trait/member 的线性查找、trait membership 的独立额度或生成 summary
的复制路径宣称为已统一；它们仍是后续 phase context 的接线工作。成员替换现在
复用 preparation allowance，该额度本身仍由旧 phase 入口提供，未接到 VM 总预算。


本批验证目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --test phases --test code_interfaces --test trait_parameters --no-fail-fast`：
  90 通过，退出 0；`/tmp/luna-tools-v_7_3kuj/result.json`。
- `-p nir -p comptime_interpretor --lib --no-fail-fast`：471 通过，退出 0；
  `/tmp/luna-tools-evrbjxrj/result.json`。
- `-p luna --test trait_phases --test quote_functions --test associated_signatures
  --test associated_purity --test deferred_associated_values --test cmtt_type_inputs
  --test sysroot_preludes --no-fail-fast`：237 通过，退出 0；
  `/tmp/luna-tools-ppddqbzr/result.json`。

格式检查 `/tmp/luna-tools-rk_jxo2m/result.json` 退出 0。未运行全 workspace 或性能
基准。正式 docs 与已有工作区修改保持不变。


### Trait phase 索引与 membership 额度

phase pass 现在构造借用当前不可变 Package 的 trait/member 索引，关联值、trait
method 和保留的 comptime method 身份查询均使用该索引，删除三条反复扫描整个
trait/member 列表的生产路径。接口内的成员身份和 membership 名称映射分别保留，
不把按名的实现阶段检查误作成员身份解析或 implementation selection。保留方法
引用仍遵循此前的 package 顺序选择；本次不据此新增跨接口声明身份规则。

membership 检查不再自行分配 100,000。trait 数量、成员数量、方法名比较及
implementation 数量／方法名消耗共享 declaration allowance；参数目录、alias
contracts 和 callable frames 也继续使用该额度。内部入口据此更名为 declaration
budget，仍不是 VM 的总预算。生成 trait summary 的复制、其他 metadata validation
及完整 body traversal 仍待接线。

索引只保存借用，不复制成员的签名、默认值或契约。查询前单独计费，已有索引也
不能在零预算下继续查询；缺失接口和错误成员归属保留各自诊断。索引构造完成后
才返回，预算或重复身份失败不发布部分目录。重复 trait identity 及同一 trait
内重复的 method／associated identity 被明确拒绝，避免元数据顺序选择不同结果。
这些检查不改变成员 phase、pure comptime trait 或 quote 的规则。

公共 phase 回归在 `0694a36b` 实现上得到 Ok，
`/tmp/luna-tools-xgxav7k0/result.json` 退出 101；当前实现拒绝重复 associated
identity，移除重复项后恢复。索引测试还覆盖宽名称预留失败时保留正余额、空接口
集合的总成本、足额重试、一次查询的局部成本、跨接口拒绝与不同 Package 同号
成员的不同类型观察。


本批验证目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --lib --test phases --no-fail-fast`：61 通过，退出 0；
  `/tmp/luna-tools-p8d3syym/result.json`。
- `-p comptime_interpretor --lib --test instance_budget --test runtime_function_identities
  --no-fail-fast`：508 通过，退出 0；`/tmp/luna-tools-2cy_9ice/result.json`。
- `-p luna --test trait_phases --test quote_functions --test associated_signatures
  --test associated_purity --test deferred_associated_values --test cmtt_type_inputs
  --test sysroot_preludes --no-fail-fast`：237 通过，退出 0；
  `/tmp/luna-tools-qw3ikfe1/result.json`。

首次局部构建因新测试同时借用 vector 的元素和 push 接收者失败，已用局部 clone
修正 fixture；`/tmp/luna-tools-341lsc60/result.json` 退出 101，不作为通过证据。
格式检查 `/tmp/luna-tools-a43zfj65/result.json` 退出 0。未运行全 workspace 或性能
基准。正式 docs 与用户原有工作区修改保持不变。


### Trait 契约只读视图与 summary 构造边界

phase validation 与 IIR validation 不再为了遍历方法的程序契约而调用空实参
summary 实例化。两者使用借用原始签名、参数绑定、requires、ensures 和 entry
snapshots 的 `CallableProgram`；method 视图没有 body，普通函数视图保留原 body
与 defaults。phase 不再收集所有 owned summaries，也不复制名称、capabilities、
requirements 和整棵契约表达式。这里是只读检查适配器，不是新的可执行 IR，不能
作为 invocation 入口或实现选择证据；其 Display 采用 S-expression。

方法 phase 仍从原 callable signature 取得，普通 telescope 视图与 unsafe 责任
分别保留。Generator、PatternGenerator 和 DataConsumer 不经此入口转成普通
callable。参数／结果绑定冲突、snapshot 约束、契约形状、表达式阶段和 lexical
unsafe 检查继续运行；程序契约仍不进入 PTC。没有把移除复制理解为移除检查。

共享不可变 visitor 与 call-safety visitor 按原来的后进先出顺序惰性取得根节点，
不先分配与 requires／ensures／defaults 数量相等的根引用数组；子树仍复用原有
遍历规则和调用者 fuel。回调取得原表达式的引用，遇到资源错误即停止，后续调用
从原不可变输入重新检查。此变更不证明子树所有临时分配都已纳入统一预算。

无预算的 `MethodContract::program_summary` 和 `program_summary_parameters`
已移除。程序验证中实际需要实例化的 summary 继续走已有的
`program_summary_declaration_inputs`，显式传入声明域、caller frames 和 fuel；
该路径仍构造 owned Function summary。本次没有重写完整 summary 表示，也没有
把整个 phase pass 接到 VM 总预算；storage、predicate shape 和完整 body traversal
的额度／成本归属仍需继续整理。

新增检查覆盖原始 signature／表达式／snapshot 引用身份、无虚构 body、根遍历
顺序和预算耗尽后重试、前后置条件都拒绝非法形状、snapshot 绑定、comptime phase
与 unsafe scope 不越过兄弟根。IIR 用例把同一契约移到 ensures 后，仍拒绝内部
未完成的类型值；contextual code 的拒绝与原有有界 summary 的声明输入检查保留。

本批验证目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p nir -p iir -p comptime_interpretor -p program_verification --lib --test calls
  --test code_interfaces --test phases --test types --test instance_budget
  --test runtime_function_identities --no-fail-fast`：869 通过，退出 0；
  `/tmp/luna-tools-8fcyo6r7/result.json`。
- `-p luna --test trait_phases --test quote_functions --test associated_signatures
  --test associated_purity --test deferred_associated_values --test cmtt_type_inputs
  --test sysroot_preludes --no-fail-fast`：237 通过，退出 0；
  `/tmp/luna-tools-17d514ij/result.json`。
- `-p luna --test method_memory_contracts --test snapshot_contracts --no-fail-fast`：
  13 通过，退出 0；`/tmp/luna-tools-geop83tl/result.json`。

首次局部编译因新测试的 source position 使用 usize 而非 u32 失败，修正后通过；
`/tmp/luna-tools-ws63n769/result.json` 退出 101，不作为通过证据。共享 visitor 的
惰性根改动前，局部与消费者检查分别为 `/tmp/luna-tools-9rz0aw03/result.json`、
`/tmp/luna-tools-_5cbchzz/result.json` 退出 0；上述 869 项为最终版本的重验。
格式检查 `/tmp/luna-tools-9pn5p2hy/result.json` 退出 0。未运行全 workspace 或
性能基准；正式 docs 与用户原有工作区修改保持不变。


### Phase inspection allowance 的实际归属

phase pass 内的 runtime storage、comptime storage、冻结／staged value admission
以及 call safety 现已接到同一次 inspection allowance，和既有成员替换、默认值
绑定观察、pattern／bit selection 准备共用余额。删除原 preparation 私有额度，
以及这些消费者按查询、值或函数重置 100,000 的路径。外层默认 inspection 额度
仍为 1,000,000，没有提高原 preparation 限额；声明目录／membership／callable
frame 的 declaration allowance 保持独立。

`StorageQueries` 同时接收两种存储阶段，分别缓存结果。两者共享同一不可变
Package 所有者，均在 hash／复制之前支付类型与 frame key 成本，命中也需要预算。
comptime 查询失败不能借用 runtime 查询的成功结论；Budget 不入缓存，Invalid
与成功只在同一 world、相同 phase/type/frames 下复用。这没有改变存储语义：
runtime callable 的参数／结果描述与 comptime 捕获的实际存储仍按各自规则检查。

call safety 返回类型化的 Budget 或 Invalid；共享 visitor 的资源失败不再被
phase 层统一报告为非法 unsafe 调用。非法操作保留具体源位置与责任诊断；IIR
及 quote expansion 的旧字符串诊断边界通过 Display 转换。此处没有更改 VM
构造错误的总分类：phase pass 的公开 Error 仍是来源与消息，constructor 的失败
预算保留策略也未改为精确计费。

生产 phase 入口的回归以 64 个各自可在旧局部额度内检查的 Symbol 值检验总成本，
现在在捕获值的源位置报告资源失败，缩小输入可恢复。内部入口另验证恰好足额、
少一步在 call-safety 处耗尽、多个函数共用额度与再次调用恢复；缓存用例涵盖
两阶段同类型的相反结论、不同 Package 同号 nominal 的不同字段、展开失败不入
缓存、正余额预留失败及热查询成本。

这还不是完整 phase 总预算。normal-form/newtype 等元数据验证、主 body/pattern 工作栈以及部分临时集合仍有独立额度或成本未接线；内部入口
据此只命名为 declaration／inspection 两个 allowance，不暴露为 VM 的全量有界
验证承诺。后续需要一起迁移这些消费者，再统一构造失败的资源分类与实际扣款。

本批验证目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --lib --test phases --no-fail-fast`：70 通过，退出 0；
  `/tmp/luna-tools-co0ty9ki/result.json`。
- `-p iir -p comptime_interpretor -p elaboration --lib --test types
  --test instance_budget --test runtime_function_identities --no-fail-fast`：
  752 通过，退出 0；`/tmp/luna-tools-p2br0_35/result.json`。
- `-p luna --test sysroot_preludes --test host_intrinsic_protocol --test trait_phases
  --test quote_functions --test cmtt_type_inputs --test method_memory_contracts
  --test snapshot_contracts --no-fail-fast`：267 通过，退出 0；
  `/tmp/luna-tools-6lk4ls5s/result.json`。

格式检查 `/tmp/luna-tools-co_z870o/result.json` 退出 0。未运行全 workspace 或性能
基准；没有提高额度、放宽存储／责任检查或改动真实 sysroot 以消除回归。正式 docs
和用户已有工作区修改保持不变。


### Predicate shape：支持结论与资源失败分离

程序谓词 shape 检查移入 NIR 的单一实现，两个入口均显式接收调用者 fuel，返回
`Result<bool, PredicateShapeError>`。Ok(false) 只表示当前 read-only 片段不接纳
该形状；Budget 单独返回，不再用 10,000 的私有上限把大谓词改判为非法。shape
不是谓词真值、指针有效性、稳定性或终止证明，候选调用入口仍不能用作最终准入。

所有生产调用者已迁移：phase 使用共享 inspection allowance；elaboration 使用
required work；字节码编译使用编译额度，资源失败不生成 `PreparePredicate false`；
树解释器及 interface／exit checks 使用 invocation steps，失败保留原条件位置。
资源验证的 moves、memory、text region 和 flow guards 使用各自现有分析额度；
可选 check elision 的资源错误进入原有事务回退，不发布部分删改。无预算的布尔
生产入口已删除，少数既有小 fixture 用测试局部适配器显式供应额度并 unwrap。

批量表达式、match arms 和 pattern children 在扩展工作栈前预留宽度成本；
类型比较、pointer／intrinsic helper 的输入类型预先计费。嵌入 frozen value 的
Budget 不再折成 false；错误 value domain 仍是 unsupported shape。pattern guard
与 observation 和外层表达式共用 fuel，不得凭 pattern 层次重新取得额度。预算
预留失败可以留下正余额；调用者依据类型化结果判断失败，不根据剩余额度猜测。

回归覆盖超过旧 10,000 节点上限的合法谓词、宽度预留失败与重试、pattern guard
中的非法操作、捕获值资源失败与域错误的区分。phase 正例和字节码的 true shape
guard 验证生产接线；树解释器验证耗尽发生在求值及 CheckPredicate event 之前，
足额重试后才产生成功事件。原有指针、intrinsic、候选调用和纯度责任未放宽。

本次没有把 shape 当作完整类型／存储／程序验证，也没有改变底层 pointer 和
intrinsic helper 的其他准入契约。normal-form/newtype metadata、主 phase 遍历
及临时集合的总预算接线仍未完成；VM constructor 的全局错误分类与失败精确
计费也仍待后续统一。

本批验证目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p nir -p comptime_compiler -p comptime_interpretor -p program_verification
  --lib --test predicate_reads --test phases --no-fail-fast`：665 通过，退出 0；
  `/tmp/luna-tools-9d96iuwy/result.json`。
- `-p elaboration -p comptime_compiler --lib --test control --no-fail-fast`：
  206 通过，退出 0；`/tmp/luna-tools-5a03493a/result.json`。
- `-p luna --test predicate_calls --test integer_contracts --test snapshot_contracts
  --test method_memory_contracts --test staged_aggregate_contracts
  --test host_intrinsic_protocol --test sysroot_preludes --test quote_functions
  --no-fail-fast`：225 通过、1 个既有 ignore，退出 0；
  `/tmp/luna-tools-23i823ra/result.json`。ignore 是
  `comptime_decisions_check_their_results_before_returning_types`，原因为 comptime
  call stability 仍缺 phase-aware value-reference evidence，本次未修改或宣称解决。

格式检查 `/tmp/luna-tools-ut_lz41d/result.json` 退出 0。未运行全 workspace 或性能
基准。正式 docs 与原有工作区修改保持不变；通过检查不能替代完整 phase／VM
预算及程序谓词调用协议的后续交付。


### Phase 主工作栈的入栈预算

phase 的表达式／pattern 主遍历改用私有 `Worklist`。body、contracts、defaults、
closure captures、effect arms 和 contextual inputs 的全部入栈路径消费同一次
inspection allowance；原先裸 Vec 的 78 处 push／extend 已迁移。入栈前支付一个
待处理节点，pop 不重复收费。direct-callee 与 comptime-context 标记随节点保留，
不借这次迁移改变遍历顺序、staged producer 传播或 runtime／comptime 准入规则。

已知精确长度的批次先预留整个宽度，再迭代并扩展 Vec；失败保留原有 pending
内容且不执行映射闭包。未知宽度的过滤／展开迭代器逐个支付节点成本，不能把
未付费子项留在待处理栈里。后者失败时允许内部队列保留已经付费的前缀，但整个
phase 查询立即失败，队列随查询丢弃，没有部分 phase 结果对外发布。宽度预留
失败仍可能留下正余额，不能由余额推断检查成功。

生产回归覆盖宽 tuple、宽 pattern alternatives 和连续单子节点表达式链。
在 `6357b484` 的原 phase 实现上，长链先完成未计费的主遍历，才在后续 call safety
耗尽；`/tmp/luna-tools-qh7uu37o/result.json` 退出 101，实际诊断为 call-safety
budget。恢复当前实现后，同一测试在主遍历入栈时报告 traversal budget，并保留
对应表达式源位置。批次失败定位于函数 owner，pattern 仍沿用 owner 来源。

这次限定为工作节点和 pending 槽位的成本归属，不将它等同于全量检查成本。
过滤器在产生节点前扫描的 capture／input 元数据、函数 frames／snapshot 集合、
成员计划与部分类型／声明观察仍需单独计费或接入共享索引。normal-form/newtype
metadata 与 VM constructor 资源分类／精确扣款也仍未完成。后续统一入口前必须
继续核对这些操作，不能因主栈有预算就把 pass 标记为完整有界。

本批验证目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --lib --test phases --no-fail-fast`：75 通过，退出 0；
  `/tmp/luna-tools-n675xbzg/result.json`。
- `-p nir -p comptime_interpretor --lib --test phases --test instance_budget
  --test runtime_function_identities --no-fail-fast`：584 通过，退出 0；
  `/tmp/luna-tools-r6njd5ou/result.json`，已恢复新实现后的检查。
- `-p luna --test default_calls --test quote_functions --test cmtt_type_inputs
  --test trait_phases --test sysroot_preludes --test comptime_bytecode_patterns
  --test closure_local_handlers --no-fail-fast`：272 通过、1 失败，退出 101；
  `/tmp/luna-tools-v4gzhzpq/result.json`。

唯一集成失败为
`closure_local_handlers::outward_request_cannot_retire_the_storage_a_callback_will_read`：
程序拒绝于 source 525..573，诊断为 `handler lexical capture address exposure
requires a memory contract`，未匹配该用例预期的拒绝原因。保持其余工作区及用户
改动不变，仅恢复本批之前的 `6357b484` phase 实现，运行
`-p luna --test closure_local_handlers
 outward_request_cannot_retire_the_storage_a_callback_will_read -- --exact`，
`/tmp/luna-tools-28v7rs67/result.json` 同样退出 101，源位置与诊断完全相同。
该基线问题未修复，也未修改测试预期；不能把此组记为全通过。

对照结束后恢复已测试的新 phase 文件并核对内容一致。格式检查
`/tmp/luna-tools-2ramcsip/result.json` 退出 0。未运行全 workspace 或性能基准；
正式 docs 与原有工作区修改保持不变。


### Callable 检查上下文的单次准备

phase 为每个 callable 构造一个私有 `InspectionFrame`：借用原有 telescope，保留
参数 sort 的原顺序，建立 snapshot target 集合。只沿 callable 包装层迭代观察，
不复制类型载荷或展开 normal-form 定义。包装层、sort frame 槽位以及输入／snapshot
索引项均消费同一 inspection allowance，预留成功后才分配相应集合。

原先每个 snapshot 两次线性扫描参数列表，现改为一次参数索引构造及集合查询；
参数 membership 仅在 result binding 或 snapshot 实际需要时建立。result 与输入
别名、snapshot 来源不在输入中、target 与输入／result 冲突及 target 重复，仍是
原有的 Invalid 诊断；资源不足是独立的 Budget，不伪装成非法绑定。批次预留失败
可以保留正余额，失败后的重新检查不继承半成品集合。

每个 callable 在同一轮完成 body、comptime storage 和 call-safety 检查，复用同一
sort frame；删除第二次全包 callable 遍历及重复 frame 构造。所有跨 callable 的
声明索引仍先建立，合并循环不以已检查的函数前缀充当可见声明全集。若多个 callable
同时非法，首个诊断可能因逐 callable 完成检查而改变；不承诺保留旧的跨函数错误
优先次序。没有放宽合法程序的准入要求。

验证覆盖预算耗尽后的重试、sort 顺序与原 telescope 借用、snapshot／result 的
五类非法关系，以及生产 phase 中无法支付准备成本的拒绝。长链遍历回归保留
child 与 owner 的不同源位置，去掉对旧准备成本下恰好落在第几个节点的依赖。

本批目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --lib --test phases --no-fail-fast`：79 通过，退出 0；
  `/tmp/luna-tools-5q715wvh/result.json`。
- `-p nir -p iir -p comptime_interpretor --lib --test phases --test instance_budget
  --test runtime_function_identities --no-fail-fast`：649 通过，退出 0；
  `/tmp/luna-tools-jbhaedz6/result.json`。
- `-p luna --test snapshot_contracts --test method_memory_contracts
  --test default_calls --test quote_functions --test cmtt_type_inputs
  --test trait_phases --test sysroot_preludes --no-fail-fast`：272 通过，退出 0；
  `/tmp/luna-tools-9nsr0_64/result.json`。

格式检查 `/tmp/luna-tools-_y_xybz8/result.json` 退出 0。未执行全 workspace 或性能
基准；集合复杂度改善不作为已测量的吞吐提升报告。上一批已复现的
`closure_local_handlers` 拒绝原因基线问题本批未修改，也未重新宣称该组通过。

此上下文不替代声明准入或 trait method view 的前置检查。方法 view 构造中的包装
观察、normal-form/newtype 元数据、其他临时索引及 VM constructor 的完整成本和
资源错误分类仍需继续迁移；不能由本批推出整个 pass 或 VM 构造已经完全有界。
正式 docs 与原有工作区修改保持不变。


### NF／newtype 声明元数据的共享准入预算

phase 的 normal-form 与 newtype 元数据检查现消费同一 declaration allowance。
`normal_forms::validate_in` 调用 `newtypes::validate_in`，不为每个 head、参数域或
newtype 重新创建额度。集合和输入槽位在分配前预留，类型／契约观察及表示替换继续
消费该额度。保留兼容 `validate` 入口给尚未迁移的独立调用方，使用一次 100,000
额度；phase 不经过该入口，也未提高既有 phase 默认额度。

两个检查器分别保留 typed Budget 与 Invalid；NF 检查还保留内层 newtype 错误。
newtype 表示查询的资源失败不再变成“非法或过大的表示”，NF 参数域的预算耗尽也
不再成为“不支持或前向依赖的类型”。类型遍历的实现深度限制仍归资源 Budget，
本批未另建精确的 depth 错误协议。旧静态字符串边界按 message 显示资源原因。

NF 元数据检查独立移入 `normal_forms/validation.rs`。每次检查建立借用的 head／
function 身份索引与 nominal 集合，collection 角色查询不再对每个类型节点扫描
整个声明列表。查询依据注册身份与既有表示契约，不增加按源码名称猜测的规则；
NF 主体保持不执行、头保持不展开。预先建立索引可以改变同时存在多种非法元数据
时的首个诊断顺序，不改变这些元数据均须拒绝的规则。

newtype 捕获观察改为预算内借用遍历，删除为了收集 rigid 参数而通过 rewrite
克隆整棵类型树的路径。重复参数的 sort 冲突、未声明捕获及 capture-first 域检查
保留。开放 collection 域已经通过 scope 检查后，只需一次 rigid 观察判定是否开放，
不再为每个 prior 参数重走同一类型。返回域或布局没有借此获得额外的相等证据。

生产回归检查无函数体的保留 head contracts 也消耗 phase 声明预算；独立测试覆盖
newtype 各个耗尽点、失败后重试、跨 head 共享额度、捕获 sort 错误和宽类型域。
新域检查接口 `supported_input_type_with_collections_budgeted` 区分普通 false 与
资源错误，能消费调用方实际给出的额度，不再内置每域 100,000 的隐藏上限。

本批工作目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p nir --lib --test phases --test declaration_metadata --test newtypes
  --test normal_inputs --no-fail-fast`：129 通过，退出 0；
  `/tmp/luna-tools-tdpqkt6c/result.json`。
- `-p nir -p ptc -p comptime_interpretor --lib --test declaration_metadata
  --test newtypes --test normal_inputs --test type_list_inputs --test instance_budget
  --no-fail-fast`：654 通过、1 失败，退出 101；
  `/tmp/luna-tools-dhaeu8dr/result.json`。
- `-p luna --test newtypes --test normal_form_domains --test normal_input_plans
  --test open_normal_input_plans --test dependent_list_collectors --test tuple_examples
  --test type_lists --test sysroot_preludes --test library_list_equality --no-fail-fast`：
  286 通过、1 失败，退出 101；`/tmp/luna-tools-2p3ztkkw/result.json`。

两项失败已分别用本批之前 `51bbc523` 的 newtypes／normal_forms／phase 源码重跑，
其余工作区保持不变：

- VM 的 `type_list_constant_nodes_validate_elements_domain_and_substitution` 在
  `type_list_inputs.rs:335` 仍断言构造期错误应为 `PhaseMismatch`。基线命令
  `-p comptime_interpretor --test type_list_inputs
  type_list_constant_nodes_validate_elements_domain_and_substitution -- --exact`
  同样失败于该断言，`/tmp/luna-tools-xzxolwbo/result.json` 退出 101。
- `tuple_examples::migrated_tuple_examples_compile_with_their_source_protocols`
  在第一个 `anonymous-record-patterns.fl` 报 `bool` 未绑定，未进入本批元数据检查。
  基线命令 `-p luna --test tuple_examples
  migrated_tuple_examples_compile_with_their_source_protocols -- --exact` 同样报错，
  `/tmp/luna-tools-vpmgjz2w/result.json` 退出 101。

本批没有修改这两个用例或降低它们的预期，也不把这些检查组报告为全通过。对照后
逐字节恢复已经测试的新实现，格式检查 `/tmp/luna-tools-j27x2e7w/result.json` 退出 0。
没有运行全 workspace 或性能基准；正式 docs 和原有工作区修改保持不变。

本批结束时，elaboration／CMTT 的五处域观察仍用兼容 bool 入口；下节已完成这些
调用方的迁移与旧域检查 API 退役。PTC／VM 独立 metadata 构造仍用兼容入口，完整
phase／constructor 的精确扣款、错误分类与稳定世界证据复用仍未闭合。其他声明
观察和临时集合亦须继续审计。


### Elaboration／CMTT 域观察接线与布尔 API 退役

普通／可选 NF 子句、表达式代码 generator 子句、pattern generator 子句和 generator
数据域已统一调用带显式预算的形状检查。`input_domains.rs` 把前缀准备、声明域观察
与类型遍历接到现有 `required_steps`，成功和失败均扣除实际消耗。资源失败携带原始
参数 span 并报告 `comptime input domain inspection budget exhausted`；普通 false
继续由对应语境报告不支持的域。这里不执行 NF 主体，也不代替类型形成证明。

generator 数据域不再有独立 100,000 遍历额度；其后的 comptime storage 检查也不再
截取另一份最多 100,000 的额度。两个步骤及 sort frame 准备消费同一次调用余额。
storage 的 Budget 向外传播为资源失败，Invalid 仍是数据域不被接纳，保持 runtime
指针存储的拒绝。runtime callable 的签名只是描述，不因 frame 准备变成存储载荷。

binder 准备现在借用输入域来判断 sort，不再克隆完整 Term。共享的观察路径保留
lexical input、nominal prefix、trait telescope、normal prefix／完成参数表的原有
查询优先级；新 bounded 消费者支付 map 查询、owner 和参数扫描成本。旧 owned
观察消费者复用同一查找逻辑并在返回时克隆，它们尚未整体迁移到有预算的接口。
该路径没有创建跨语义修订复用的缓存，也没有把未知域固化为新的语言规则。

storage frame 按原 `call_inference_scope` 的优先级取得已知参数前缀，逐项支付后
收集 sort。signature 包装层迭代观察，open function 在第一个未完成 annotation
处停止；不为取得 sort 克隆参数类型或 closure 环境。此处的停止语义与原路径一致。

全部生产调用方迁移后，删除 NIR 的六个无调用方预算、返回 bool 的兼容域检查 API。
生产仅保留 `supported_input_type_with_collections_budgeted`。小型旧测试使用
`tests/support/domain_shapes.rs` 的显式 fixture 额度，资源失败立即令测试失败，
不再 `.unwrap_or(false)`。这暴露了超长 record label 测试先前把资源耗尽当成语义
拒绝的问题：现断言小额度返回 Budget，并验证同一域在充足额度下可接受；重复字段名
仍明确拒绝。未增大生产默认预算，也未将超长名称排除为永久语言限制。

发布回归通过真实 DefinitionBatch 检查普通 NF／expr code／pattern code 的必要和
可选子句。CMTT 在域检查前会准备 generator checkpoint，因此测试逐步找到域检查
自身的耗尽点，而不硬编码其前置成本。每次失败都核对 batch 未变化、没有发布定义；
之后在充足额度下重试成功。这些测试不以更早的 checkpoint 失败代替域检查接线。

工作目录 `/home/fin/luna`，统一入口
`python3 -B development/check.py --timeout 900 -- test`：

- `-p elaboration --lib --test invocation_budget --test generated_normal_forms
  --test default_strategies --no-fail-fast`：184 通过，退出 0；
  `/tmp/luna-tools-io4pnvfm/result.json`，旧 NIR API 删除之前。
- `-p luna --test cmtt_type_inputs --test cmtt_phase_boundaries --test test_templates
  --test normal_form_domains --test normal_input_plans --test open_normal_input_plans
  --test sysroot_preludes --test library_list_equality --no-fail-fast`：182 通过，退出 0；
  `/tmp/luna-tools-gkggyfxu/result.json`，生产迁移完成、旧 API 删除之前。
- 删除旧 API 后，`-p nir -p elaboration -p comptime_interpretor --lib
  --test normal_inputs --test normal_callable_inputs --test float_formation
  --test declaration_metadata --test invocation_budget --test generated_normal_forms
  --no-fail-fast`：705 通过，退出 0；`/tmp/luna-tools-mfkfqw7k/result.json`。
- 最终源码的 `-p luna --test cmtt_type_inputs --test cmtt_phase_boundaries
  --test sysroot_preludes --no-fail-fast`：42 通过，退出 0；
  `/tmp/luna-tools-sann13b5/result.json`。

过程中 `/tmp/luna-tools-eh41lklm/result.json` 的新 helper 与既有实例化 helper 重名
导致编译失败，已改为 `normal_input_domain_supported`；
`/tmp/luna-tools-_2cy2yzo/result.json` 暴露 CMTT 零额度先停在 checkpoint 的测试假设，
已按实际发布顺序验证域检查边界；`/tmp/luna-tools-40osdowc/result.json` 暴露上述
record label 测试的资源／语义混淆。最终检查已覆盖相应修正，不把这些中间失败计为
通过。格式检查 `/tmp/luna-tools-v3d65b9l/result.json` 退出 0。

未跑全 workspace 或性能基准；已有 VM 错误分类、tuple example 和 closure handler
基线问题本批未处理，也未声明解决。required_steps 仍不是整个 elaborator 的完整
成本模型；signature／generator 元数据准备、其他 owned 域观察、PTC／VM 构造中的
预算与稳定世界证据仍须继续迁移。正式 docs 和原有工作区修改保持不变。


### VM 构造与声明域准入：当前接线

构造器的资源失败必须保持为 StepsExhausted，非法元数据、作用域、sort 或值域仍
拒绝为 InvalidCallPlan。失败的批次预留可能留下正余额；不能根据余额是否为零
猜测错误类别。构造期间没有用户执行事件，不发布部分索引或前提闭包。

**元数据与快照。** 新世界通过 `normal_forms::validate_in` 检查 NF／newtype
结构并支付构造额度，外层和 newtype 内层的 Budget 均保留分类。
SourceIndexes 持有原 Arc 快照，只能在精确 Package 身份匹配时复用结构证明；
同内容副本、修改后的世界及同号声明不能借用。caller premises 不属于结构证明，
仍须单独检查。公开 for_comptime_in_session 目前只传空 premises；非空 premises
与索引组合由内部构造器回归覆盖，尚未新增公开的带前提 session API。

**前提闭包。** `constructor_premises::expand` 借用初始 premises，先预留队列批次，
再在键复制／哈希和替换输入复制前检查嵌套 Term。父 trait 与 capability 通过
Trait::input_substitution 及 DeclarationSubstitution 的 bounded 接口展开；重复
替换载荷按展开结果计费。队列、声明参数域与后续 binder 检查共用同一份余额，
成功后将剩余额度交给执行器。去重与原有 64 层展开上限保留，前提不提供方法体。
即使叶节点没有后续边，也检查 Self／实参的作用域与 sort；普通 comptime 值仍按
其声明域使用。这项结构检查不替代 scalar range、requirements 或方法可用性。

**trait 值域准入。** Trait 的 validate_arguments 系列现在直接返回共享的
NominalAdmissionError（value_admission::Error），保留 Budget、Unresolved 与 Invalid。
Unresolved 表示接收世界尚未完成某个前提检查，既不是域反例，也不授权接纳。
参数 binder、Term 预检、声明替换和布局查询不再先转换成 String。
声明域查找仍按声明身份取得独立证据，不以待验证的参数槽反推实际域；冲突、
缺失与不匹配证据继续拒绝。VM 的具体 implementation 检查消费这个分类，资源
耗尽不再变成 InvalidCallPlan。

**嵌套查询。** DomainQueries::declared 与 trait Type 域关系回调均返回
Result<bool, AdmissionError>。scalar、record、optional、nominal 及 record 内
nominal 的声明查询和关系检查都传播错误；Ok(false) 仅表示没有匹配的证据，
不是预算失败。Type 关系不建立值索引相等，也不展开 NF 头。已经无调用者的
validate_symbolic_value 与 validate_record_index 私有包装已删除。

**Type 域证明。** equivalent_domains 仅对 Type 操作数查询 PTC：Entailed 返回
Ok(true)，NotEntailed 返回 Ok(false)，表示当前前提不足以保证必检域要求，不能
据此添加否定等式。BudgetExhausted 保持 Budget，其他 Unknown 保留原因为
Unresolved，IllFormed 为 Invalid。调用使用同一余额并在每种结果后同步；失败的
预检仍可能留下正余额。未注册 NF congruence 的 UnvalidatedModel 不被当作域反例，
也不以展开 NF 主体求解。

implementation 头的直接值输入及嵌套 symbolic 字段均使用这一可失败查询。
其参数作用域和 sort 已由 DeclarationSubstitution 检查，值本身直接进入共享
value_admission 内核，退役该路径重复构造单参数声明并经过布尔 evidence 包装的
做法。Budget／Unresolved 在候选形成处保持 TypeAssessment::Pending；不能从
一次失败查询永久排除候选。实际回归覆盖缺少 congruence 时 Pending，补齐同一
世界中的证据后重新匹配成功。

NF 的 ApplicationError::Unresolved 保留失败的参数位置和原因；dispatch 转回
共享准入错误时不丢失该分类。执行侧 typed admission 消费者将其映射为
TypeContractUnresolved，区别于 StepsExhausted 和类型错误。NF 未完成检查不会
人为清空尚余预算，后续有证据的独立检查可以成功。

回归检验不同 inherited premises 的替换、重复循环终止、叶节点、非法接口与
参数、宽输入和重复替换载荷；fresh 构造及具体 implementation 准入在每个不足
额度上返回资源错误，精确额度成功，失败后重试保持原世界且无执行事件。
嵌套值准入还逐级扫描不足额度，并在有正余额时注入查询资源错误和语义错误，
验证原分类传播；关系不成立仍拒绝，独立关系证据成立才接受。
PTC 会按当前预算为传播与学习分配额度，较小预算可能走更便宜的证明路径；因此
大预算运行的消耗不是最小成功阈值。域证明回归逐预算检查只能成功或返回 Budget，
不能误报 Invalid／Unresolved，也不能增长余额，而非要求所有较小预算必须失败。

当前尚未完成的边界：

- fresh phase pass 仍使用自己的默认额度，VM 随后再做结构检查；构造失败的
  for_comptime_in_budget 仍保守保留全部 reservation，尚非精确失败扣款。
- 后续关联项／方法签名与 world capability 的部分实例化尚未有界。
- PTC 的域关系查询已保留 Unknown，但 capability 外层仍有 String、is_ok 与
  可选结果边界；proof replay 也仍有合并 admission 错误的路径。完整求解链的
  Pending／资源／反例传播尚未完成。
- 旧 layout／callable 可选观察器把 Unresolved 表示为 unavailable view，仍不能
  保留具体原因；typed admission 直接消费完整错误。后续须迁移这些观察器及其
  消费者，不能把本次新增分类说成所有描述观察都已无损。
- nominal 的既有公开 evidence 包装仍接收总布尔关系，再适配到共享内核；
  需要可失败域查询的消费者必须直接走 fallible DomainQueries。各公开包装的
  后续迁移以及旧 index shape 布尔观察仍待处理。

验证证据见下；历史中间版本的逐轮日志由 git 保留，不作为当前实现状态。

最终源码的验证范围与输出锚点如下（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test -p type_resolution -p nir
  -p comptime_interpretor -p iir --lib --test implementation --test capabilities
  --test trait_parameters --test value_admission --test normal_inputs
  --test normal_callable_inputs --test callable_world --no-fail-fast`：769 通过，
  退出 0；`/tmp/luna-tools-1rr5l0my/result.json`。
- 修改文件的 rustfmt check：退出 0；`/tmp/luna-tools-x0qkiza3/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test trait_phases
  --test trait_domain_substitution --test dynamic_trait_clauses --test comptime_bytecode_traits
  --test sysroot_preludes --test library_list_equality --test normal_input_plans
  --test cmtt_type_inputs --test nominal_staging --test newtypes --no-fail-fast`：371 通过，
  退出 0；`/tmp/luna-tools-j7zkw6a9/result.json`。

未跑全 workspace 或性能基准，其他已记录基线问题不在这批宣称解决。
正式 docs、program verification 与 library 的既有工作区修改保持原字节内容。
