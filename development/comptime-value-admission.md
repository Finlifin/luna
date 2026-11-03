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
本轮只静态核对这些生产入口，未执行所附测试。

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

本轮没有执行这些用例。具体稳定节点 Rust 接口、持久 codec 及一般循环快照存储仍须
按本协议细化；不在本次文档提交中声称这些生产路径已经统一。
