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
嵌套字段/列表位置需维护根、路径、世代和失效规则；存储重新赋值后旧位置证据不再有效。

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

若请求有真实未完成输入生产者，region 让该 VM session 等待。若宿主只是预算耗尽，
不能伪造 waiter。必需结果循环报告依赖链；已存在签名支撑的普通递归仍按函数执行。

外部不纯操作只有在源控制流明确要求、候选选择已确定后执行。请求身份与进度保证
同一会话恢复不重放，不承诺崩溃后自动 exactly-once；崩溃重启或取消后重跑的外部任务
须由 build 任务契约定义。内存事务不能回滚外部文件或进程。

## 6. 缓存分别回答不同问题

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

## 9. DefinitionTransaction

目标让 struct/enum/newtype/fn/trait/impl 等走共同发布协议：

1. 在批次 owner 中预留声明与 binder 身份，草稿保留未完成结构。
2. 基于固定 WorldSnapshot 创建私有 overlay，解析生成代码的显式引用和来源。
3. 用普通接口检查与 elaboration 验证完整批次；名义互递归用已准入签名，不用占位成功。
4. 验证发布所需的全部接收方（声明目录、VM、实例/协议索引）并准备 receipt。
5. 核对基线 revision，原子发布后继世界；竞争收据失效，旧世界仍可只读使用。
6. 恢复原 invocation，返回可在后继世界使用的句柄/值，不返回活动 checker。

结构分配失败、语义失败、接收世界拒绝、revision 过期都不泄漏部分声明。
事务保留可修复草稿；预算消费不回滚。预留编号可以留下空隙，不能将已观察身份复用给
不同定义。源码创建和 builder 创建共享检查规则，不为“简单 struct”另造长期语义通道。

## 10. 删除旧路径的条件

| 旧路径 | 删除前必须看到的生产行为 |
| --- | --- |
| NF 专用表达式/值求值 | 普通 callable、用户 NF、Tuple 与 List 经过同一计划及冻结路径 |
| invocation 的树 fallback | 支持范围内每种操作均走字节码，拒绝在执行前确定，次数/失败一致 |
| per-kind builder 桥接 | 多种 definition 在同批发布及活跃调用恢复时用同一 receipt 协议 |
| 手工零散替换 | 嵌套 binder、契约、属性、控制目标和来源均经统一变换验证 |
| 全量外围捕获 | 递归局部声明的精确依赖和真实循环均有源码证据 |

混合 interpretation、runtime reference、原子发布及接收世界检查不在删除名单中。
