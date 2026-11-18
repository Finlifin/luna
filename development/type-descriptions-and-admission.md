# 可计算类型描述与调用准入

状态：重构设计；初始源码核对基线 `e1c57dc9`，第 9 节按 `627ab804` 复核值载体
和输入绑定。显式 Any 的源码和 VM 传递现状见第 1 节。目标是支撑作者指定的
`fn println(comptime format: str, ...varargs: defer std.fmt.Parse(format))`，
同时适用于用户类型工厂、别名、容器和普通函数。正式 docs 保持不动。

## 1. 契约与当前断点

依据为作者确认，以及[类型身份](../docs/type/type-identity.md)、
[Any 与上下文检查](../docs/type/comptime-gradual-dependent-type.md)、
[两阶段检查](../docs/comptime/normalization-and-instantiation.md)和
[参数供应](../docs/functions/parameters.md)。历史 prompt 中的格式化意向仅帮助
理解需求，其中旧的 postfix refinement 拼写不是当前语法依据。

Any 与 bool、u8 分别相容，不产生 bool == u8，也不产生二者间的转换。
`Identity<Any>` 不因 Identity 的函数体返回参数而变成 Any；具体化后仍保留头。
返回描述含 Any 不是立即拒绝的理由，但调用实例必须有合法类型与交付表示。

| 当前载体或入口 | 已有职责 | 尚缺的能力 |
| --- | --- | --- |
| `open_functions::observations::TypeObservation::{Known, Unknown}` | 定义侧的有限域观察；待定注解观察为 Unknown | 不持有生产状态，也不是已经求值的 Type 数据 |
| `open_functions::Annotation` | 分别保存具体身份、已完成的显式 Any、待执行计算、前置 Type 输入和待恢复源码。调用者可用自己的实参域具体化 Any | 已检查代码支持有限的外层头观察；更一般的返回值传送仍待接通 |
| `Value::Type`、`TypeParameter`、`Description` | 传递完整或受作用域授权的身份，以及显式 Any 与受闭合类型命题约束的描述 | 闭合 NF 输入可传送，外层捕获还需要重定位 |
| `PreparedRequired::invoke_in_world` / `restore_required_packet` | 普通 VM 调用可交付具体身份或显式 Any，回到调用方 scope 后继续使用 | 复合描述和未完成源码区域的恢复仍待接通 |
| `Checker::completed_type` / `type_expr` | 前者完成 Type 值；后者在具体类型使用点索取身份。闭合别名可以发布两种结果 | 有描述实参的别名目前只接通必选 Type 槽位；默认值和混合值域仍走 Term 路径 |
| `type_expr` 的 `RefinedType` | 具体 base 检查要求；描述 base 保留 subject binder，在调用方分别证明闭合类型命题 | 外层捕获与可执行谓词还缺传送机制 |
| `comptime_values::encode_admitted_list` | 已准入列表的普通稳定编码 | 元素仍为 Term，描述数据不能靠 Type 列表专用旁路接入 |
| `normal_types`、值准入与 nominal formation | 按接收世界检查身份和普通实参 | 描述应用不能直接借旧身份/布局接口伪造完成 |
| `builtin.Any` | 集中绑定的 Type 常量；可通过 prelude 或重命名导入 | 用于具体类型注解时仍需实例准入 |

### 源码中的显式 Any

```flurry
use builtin.Any as Field;
const Fields: meta.List<Type> = [Field, u8];

struct Slot { domain: Type }
comptime fn Forward(value: Type) -> Type { value }
comptime fn First() -> Type {
    let slot = Slot { domain: Forward(Fields[0]) };
    slot.domain
}
```

`First()` 返回 `(type-value (description any))`。源码使用普通名字解析。常量重建、
两套 VM 的函数入参与返回、列表取值以及结构体字段都保留 Type 数据。需要类型
相等判定、布局或结构观察的使用点另行要求身份。描述中的 Any 不会为 PTC 引入等式。
上述行为及失败后同一 VM 的后续调用见
[源码回归](../compiler/luna/tests/gradual_type_data.rs)。

同样的区别也适用于类型别名。下面的 `Forward` 经一次元组展开得到 Any；把
`Forward` 放进 `meta.List<Type>` 可以传递这份数据。若将它用作结构体字段的
存储类型，诊断出现在需要具体类型身份的字段上。

```flurry
typealias Field = Any;
typealias Second<A, B> = B;
comptime fn packet() -> Tuple<Type, Type> { (bool, Field) }
typealias Forward = Second<...packet()>;
```

具有稳定实参的别名按声明、槽位实参和声明实际捕获的输入保存结果。例如
`typealias Declared = Make();` 只产生一次类型，通过 `Through<bool>` 与
`Through<u8>` 引用它仍得到同一身份。另写一条调用生成式工厂的别名声明会得到
独立结果。成员别名引用所在类型的参数时，该参数也参与区分。静态类型构造沿用
原有的形成检查，包括位宽、实参域和约束。含描述的实参走别名的参数绑定和约束
检查路径，目前还不能作为稳定项写入 NF 或 metadata。

[带 Display 的最小复现](repros/refined-any-type-producer.fl)现在可以解析 Any。
`refines { Itself :- Display } Any` 可以作为 Type 返回，实参在自己的调用点证明 Display。含描述的闭合 NF 实参在独立的描述数据中保留头与值模板，模板槽位逐项绑定实际输入。
`std.fmt.Parse` 已接通变参准入，定义端从受检 Type 生产者的返回路径观察共同 NF 头，并在字段 guard 内取得方法证据。

### 已接通的调用点准入

```flurry
comptime fn Select(flag: bool) -> Type { if flag { Any } else { u8 } }
fn consume(comptime flag: bool, value: defer Select(flag)) -> bool { true }
fn pass(value: Any) -> Any { value }

fn run() -> bool {
    let byte: u8 = 7;
    let copy: u8 = pass(byte);
    let keep: fn(bool) -> bool = pass;
    consume(true, keep(true)) and consume(false, copy)
}
```

`consume(true, true)` 和 `consume(true, byte)` 各自采用实参已有的类型。
`consume(false, true)` 在同一条调用路径上收到具体的 `u8` 要求，因而拒绝。
检查实参域时保留已检查的实参表达式，调用仍按源码顺序求值。若具名实参使
comptime 调用提前完成后面槽位的注解，同一次调用会向后续步骤交付已完成的域。
用闭包交替返回 Any 和 bool 的回归覆盖了这个顺序。

`copy` 的注解给出了返回值要交付的类型，`keep` 则同时给出了参数和结果的类型。
两次均须检查 `pass` 的具体函数体。例如 `let bad: bool = pass(byte);` 会在检查
交付时失败。没有目标类型的 Any 返回值目前仍缺从实例主体取得结果域的路径。
定义端对未知成员的访问、实际存储类型和具体参数的资源检查继续生效。
源码、调用副作用顺序与拒绝例见[真实 sysroot 回归](../compiler/luna/tests/gradual_type_data.rs)。

这里交付的是单个显式 Any。要使 `Select` 返回 `refines { Itself :- Display } Any`，
需要在 Type 数据中保留要求的 subject binder 及其声明来源，在接收世界对实际域
检查要求。当前形成 refinement 时就对 base 调用 `type_expr`，仍会索取身份。
再往下，`std.fmt.Parse` 要把这种描述作为普通值装入保留头的构造应用中，交给
变参槽位按字段取得实际域。这两步都超出了单个 Any 的准入能力。

### 已接通的函数类型值传送

一个闭合的函数类型可以在签名内部绑定自己的参数。例如下面的 `Reader` 可作为
`meta.List<Type>` 的元素，`read` 可作为 `meta.List<Reader>` 的元素。取出同一个
函数值后，两个调用各自绑定自己的 `T`。

```flurry
struct Packet where T { payload: T }
typealias Reader = comptime fn(comptime T: Type, Packet<T>) -> T;
comptime fn read(comptime T: Type, packet: Packet<T>) -> T { packet.payload }

comptime fn evaluate() -> bool {
    let readers: meta.List<Reader> = [read];
    let domains: meta.List<Type> = [Reader];
    domains[0] == Reader and readers[0](bool, Packet<bool> { payload: true })
        and readers[0](u8, Packet<u8> { payload: 7 }) == 7
}
```

VM 在检查参数域后才把这个参数加入当前签名的可见前缀；返回类型可见整个前缀。
嵌套签名引用外层值索引时，先把它的声明域移入当前位置，再与要求的域比较。
例如外层 `tag: T` 进入内层 `U` 的作用域后，仍只为外层的 `T` 提供值域证据。
这个规则用于被传送签名中的普通名义声明、NF 应用和效应操作实参。作用域检查借用
外层参数，NIR 准入结果携带同一作用域供后续替换使用。

类型形成本身仍须检查声明的要求。依赖签名局部参数、无法在本地判定的命题，
目前返回 `TypeContractUnresolved`。现有证明回调只能接收当前 invocation 的命题，
传入一个属于内层签名的自由槽位会造成变量捕获。具体调用完成绑定后可以检查闭合要求。

真实 sysroot 用例在 `compiler/luna/tests/staged_callable_freezing.rs`。
这里传送的都是具有类型身份的值。`refines { Itself :- Display } Any` 的描述载体、
按实际参数逐项准入和 `std.fmt.Parse` 仍待实现。

## 2. 三种未知与两个完成点

必须区分：显式 Any 是已经得到的描述；尚未取得输入的计算仍在等待；刚性 Type
参数已有身份，只是具体内容不可观察。三者不能通过一个 Unknown 枚举项互转。

以下是内部职责，不是新语言类型或已经可用的 Rust API：

```text
TypeObservation
  KnownIdentity / KnownStructure / KnownRequirements / Unknown
TypeProduction
  Waiting(dependencies, continuation) / Running(invocation)
  Complete(TypeValue) / Failed(diagnostic)
TypeValue
  Identity(checked type reference) / Description(stable description graph)
InputAdmission
  Waiting(actual domain or evidence)
  Accepted(concrete domain, adaptations, evidence) / Rejected(diagnostic)
```

TypeObservation 回答检查器知道什么；TypeProduction 回答计算进行到哪里；TypeValue
是已经得到的普通 comptime 数据。当前源码检查使用私有 TypeObservation，只承担
第一种职责；Unknown 可能来自仍未完成的计算，不能移到 VM 当作第三种职责。
旧 FrozenDescription 与 compatibility/refinement 模型已整体移至 test_support，
不再从 elaboration 生产库导出；模型测试不能证明源 Any 或描述准入已经实现。

Type producer 应完成到 TypeValue。需要具体身份的消费者再请求身份形成，不能让
所有 Type 数据返回都必须占有一个最终 PTC metavariable。AssignmentPermit 继续
保护实际推断赋值，但它属于需要赋值的消费者，不是每个 Type 值的天然所有者。

消费者需要具体身份时，Identity 走原有形成、scope 和赋值检查；Description 必须
再取得实际域或合法适配，不能将 Any 写进变量后称为已解。消费者需要参数描述时，
可接收已完成的 Description，再运行独立的准入工作。两个消费者等待同一 invocation
完成不授权重跑其 producer；生产失败、等待、准入拒绝仍是不同状态。

## 3. 描述数据与普通值通道

描述图至少承载显式 Any、已知身份引用、受检构造应用与 refinement，并保存声明
来源、binder、普通值参数和所需世界依赖。不保存活动推断变量、候选事务或未恢复
的执行栈；这些属于 TypeProduction/准入状态，不能冻结进值。

同一份 Any 描述放进列表两次，可分别接纳 bool 和 u8；同一刚性 T 出现两次则仍
要求满足 T。共享图节点表示共享数据，不表示共享一个实例变量。refinement 的
Itself 在各次准入中绑定到自己的 subject，不沿用上一次调用的变量或前提。

结构相等可用于图去重，但不是语言的类型相等证明。dump 须明确区分
`(type-value (description ...))`、`(type-observation ...)` 和等待状态，采用
已有 S-expression、确定顺序与符号转义规则。

返回、局部量、值引用、容器、闭包捕获和普通参数应保留同一描述。接收域为 Type
时检查实际 Type 值的合法性及 scope/world，不要求立即为每个 Any 选择具体类型。
`meta.List<Type>` 仍保存普通元素值，不新增 Any 列表或 Type 专用参数槽；用户
comptime struct、匿名记录和 Object 也必须通过同一准入和冻结服务。

稳定图及其依赖可以合法传送时，冻结其数据；这不代表 runtime 类型已形成。
当前 Term 混合承载身份与冻结数据，迁移要在共同冻结值层支持描述载荷，不能把
Any 塞进普通 Sort::Type 身份等式以省去改造。消费身份的地方必须明确取得 Identity。
预算不足、依赖未取得与内容非法分别报告，不能回退成 Any。

源名称由六包集中绑定提供，alias 不增加身份；不按 Any、Parse、println 的用户
拼写识别权限。Any 也不能注册成有大小的 primitive 以复用布局。源绑定、alias、
普通 Type 返回和容器传送须一起验证，不能只添加 lexer/HIR 分支便宣布可用。

## 4. 构造应用、refinement 与身份

`Parse(format)` 执行普通类型计算；`Parse<format>` 不因需要结构而执行主体。
构造输入含描述时，保留原始 head、按签名检查的普通输入和约束，形成对应描述
应用。位置选择、默认值和收集复用共同输入计划；Tuple 仍接收一个普通 List<Type>。

结构观察需要该声明的受检结构协议或既有语义规则。不能从名称、参数数量或函数体
猜测结构，也不假设所有头可逆。元组视图可提供有序元素要求，但不能抹掉头或建立
头应用与结构本体的等式。用户定义有相同受检能力时，应使用同一机制。

描述携带要求，不携带“所有将来实参已经通过”的证明。定义检查可在假设要求成立
的子上下文中检查操作；调用准入必须在原调用方上下文证明实际类型满足要求。
不能用定义端自己安装的假设自证。test_support 中 RefinementScope 的上下文分工
保留为协议回归，但没有源码或 VM 生产消费者；未来接线须实际实现这一职责。

Itself 上可提取的类型命题交给 PTC；关于 runtime 值、内存与状态的谓词仍属于
程序验证及规定的运行时检查。不可提取的谓词不提供静态事实。两类义务都保留
binder、来源和错误位置，不因同在 refinement 内而混用求解器。

## 5. 普通定义检查还需要结果证据

仅有 `Parse: comptime fn(str) -> Type` 并不证明结果是可遍历的 tuple，或每项都
满足 Display。一个返回 bool 类型的工厂不能因为同样返回 Type，就授权消费者
执行 tuple_enumerate。只检查格式化调用的具体实例，会遗漏这条定义端契约。

需要保留 Type producer 的可靠结果观察，来源为显式契约或普通检查可证明的返回
结构。推导只发布所有正常返回路径共同成立的信息，不执行未知输入，不任选第一条
return，也不把某次调用结果推广到所有输入。等待、分析失败和预算耗尽不提供保证。

例如所有正常返回都通过同一个有合法结构视图的构造形成，可保留共同头与参数域
信息；具体长度仍依赖 format。字段 refinement 必须来自实际描述构造或已证明
契约。经过容器及函数转发，观察仍绑定实际值来源，不能凭静态外壳扩大权限。

inline 主体也可以显式进行有依据的 Type/trait 检查，只在对应分支内调用 Display，
并对不满足条件的情况给编译期诊断。guard 证据具有自己的作用域。tuple_enumerate
本身仍需要结构依据，不能仅因参数有 `...` 就假设完整具名供应也是 tuple；具名
供应与自定义收集协议都使这种假设不成立。

结果观察必须与源检查后继共同设计；“将来会实例化”不是跳过定义检查的开关。
现有 open definition 观察逐步迁入共同服务，不新建格式器专用 AST 解释器。

## 6. 调用实例、缓存与 runtime 交付

对 `println("{} {}", number, flag)`：先按普通规则取得 format，执行所需 Parse；
取得两个字段的描述后，检查实际域与各自 Display 义务，再形成具体 tuple 类型
及资源契约，最后检查实例主体与 inline 展开。数字默认类型服从普通上下文检查，
不能由格式器固定成一种机器类型。完整具名供应不运行收集，但仍检查描述与义务。

实际 runtime 输入仍按源码序各求值一次；检查顺序不决定执行次序。不同调用的
实际类型、变量、前提和资源状态独立。错误参数不能污染后续合法调用。

描述引用依赖不可变世界 owner 及合法范围，不仅是数字 declaration id。可共享
稳定数据和元数据，不能跨世界复用某个调用的 impl 选择。Type-result 缓存仍遵循
纯性、键及可传送条件；缓存描述不缓存准入事务。不能因为格式扫描频繁而扩大非
Type invocation 的结果缓存。新的不纯调用仍是新的执行。

等待后原位恢复；取消不重放已发生的副作用；非法结果和预算失败不发布半张图。
IIR/runtime 只接收其阶段允许的类型与值。Any 没有自动 tagged union、统一指针
或动态分派 ABI。返回 Any 的函数可在具体调用中形成合法单态结果；如果 runtime
分支没有满足共同目标及表示的交付，就在该边界拒绝，不能任选一支的布局。

## 7. 生产完成与消费者完成分别发布

`3567aefe` 曾统一 Type 队列的同步与挂起 owner；其中 VM 完成后立即取得 Term、
`assign_reserved` 再发布 `State::Complete(Term)`，注解直接借用该身份。
但 `627ab804` 清理输入齐备的 required invocation 后，这个队列已没有前端生产
调用者，不能再称为“当前真实依赖队列”。旧调度器现只保存在
[共享测试支持](../compiler/elaboration/test_support/type_computations/mod.rs)，用于
检验独立的依赖、预算、取消和不重放契约，不是待定源码 Type 响应的接线终点。

真实 `PreparedRequired` 将 Type 返回值与其他 comptime 数据共用普通 VM 调用和
packet 恢复，不再新建空推断上下文及临时输出变量。具体接线见
[状态与发布协议](elaboration-state-and-publication.md#type-生产者的-invocation-与发布边界)。
该通道仍要求预先完成根 Expr；完整描述载体、多消费者准入和原源码区域恢复尚未
实现。后续应在这一普通值通道保留生产结果，再让有身份需求的消费者持有赋值权限，
不能为了复用旧测试模型而重新为每次 Type 生产分配 MetaId。

迁移后的所有权分为三部分；以下为目标协议，不是现有 API：

| 所有者 | 完成产物 | 失败影响 |
| --- | --- | --- |
| 当前调用的 Type producer | 经域、scope/world 检查的不可变 TypeValue | invocation 错误使依赖该次生产的消费者失败 |
| 参数描述消费者 | 描述及其结构观察，实际输入到达后形成实例准入结果 | 参数数量、实际域或 refinement 拒绝仅使该消费者失败 |
| 身份消费者 | 合法具体身份及相应推断赋值 | 无法取得身份或赋值冲突使该消费者失败，不改写已经完成的 TypeValue |

producer 的身份必须独立于 `MetaId`。一次生产可以没有推断赋值消费者，例如将
结果保存到局部变量或普通容器；一个结果也可以被多个后续操作读取。具体实现可以
保留专用于身份需求的适配器，但不能继续让每次 Type 返回天然预留一个 PTC 变量。
同一个源码调用的依赖边可以引用同一生产结果；两个独立源码调用不因此合并，尤其
不能借此给不纯调用增加隐式 memoization。

执行完成先检查返回值确实属于 Type，并检验描述图内部的绑定、世界和已知身份，
再发布完整结果。未通过检查不发布，不能把任意宿主构造的图标成已完成。描述中的
refinement 在这里检查保存是否合法，不证明所有未来实参都满足要求。
消费者随后在自己的调用方环境中实例化要求；不能把实例变量、impl 选择和证明
写回共享描述，也不能用一次消费者的失败覆盖生产结果。

例如同一份“两项 Display 参数”的描述先用于 bool/u32，再用于另一对实际类型。
第二次若没有 Display 实现，它应在准入处拒绝；第一次的结果与描述本身仍有效。
反过来，描述生产过程中发生错误时，不存在可供任一消费者继续使用的半成品。
这一分工同样适用于普通用户函数，不识别 Parse 或 println 名称。

恢复时由原执行 owner 保留 VM 栈、宿主请求和剩余预算。身份消费者只在取得合法
身份后通过自己的 `AssignmentPermit` 发布；过期许可不得撤销已完成生产。
放弃某个消费者不应借其析构取消其他存活依赖仍需要的 producer；真正取消 producer
则终止该次执行，丢弃未发布结果，并使其依赖明确收到取消。即使执行已产生副作用，
也不能因消费者重试而重放。这里的读取共享与未来跨调用结果缓存是两件事。

dump 应显示生产状态及各消费边，例如下面的结构示意，而不将所有状态都打印成
`complete Type`：

```text
(type-production p0
  (complete (type-value (description d0)))
  (consumer c0 (admitted (identity t0)))
  (consumer c1 (rejected (slot 1) "missing Display evidence")))
```

`p0`、`d0`、`c0`、`t0` 是示意 owner/节点标记，不是可持久化的全局编号。
保存诊断与观察不授予执行权；稳定描述中不嵌入消费者或活动生产队列。

## 8. 同一次迁移必须覆盖的值通道

下面以 `3567aefe` 的载体核对为基础，纳入 required 普通返回及测试队列退役后的
生产入口。它补足第 3 节的载体要求，不把这些函数当前接受的 Term 范围提升为语言规则。
仅新增 VM enum 分支或注册 Any 名称，会在
后续容器、冻结或签名路径再次丢失信息。

| 当前入口 | 当前边界 | 目标职责与退役条件 |
| --- | --- | --- |
| [`Interpreter::type_data` / `type_identity_term`](../compiler/comptime_interpretor/src/lib.rs) | 分别读取 Type 数据和具体身份；数据转发可保留显式 Any | 复合描述须保持相同的消费边界 |
| [`value_types::has_type_at`](../compiler/comptime_interpretor/src/value_types.rs) | Type 域的值走 `inspect_type_description` | 检查合法 TypeValue 成员资格；不将数据检查等同于实际参数准入 |
| [`admission::admit_value_types`](../compiler/comptime_interpretor/src/admission.rs) | Type 载荷走接收世界的 `admit_type` | 遍历描述中的身份、要求和绑定；身份引用仍须在接收世界有效，实际输入的 trait 选择另行进行 |
| [`normal_terms`](../compiler/comptime_interpretor/src/normal_terms.rs) | Type 值冻结为身份 Term | 共同冻结载体保留描述载荷及依赖；容器、Object 与捕获递归走同一规则 |
| [`normal_values`](../compiler/comptime_interpretor/src/normal_values/mod.rs) | Type 域将输入 Term 解码成 `Value::Type` | 解码后仍区分身份与描述，不根据接收域为 Type 就将任意载荷当作身份 |
| [`nir::comptime_values`](../compiler/nir/src/comptime_values.rs) | 列表字段是 Term；Type 元素用 `Sort::Type`，普通值验证入口拒绝 Type 域 | 迁移共同冻结值与域检查；不得添加仅用于 List&lt;Type&gt; 的描述字段或宽松 sort 绕行 |
| [`deferred_signatures`](../compiler/nir/src/deferred_signatures.rs) | 稳定注解为 Known(Term) 或 Computed 索引 | 保存稳定描述与计算计划；活动生产、准入事务与等待者仍留在 elaboration |
| [`required::prepared`](../compiler/elaboration/src/program/required/prepared.rs) / [`restore`](../compiler/elaboration/src/program/required/restore.rs) | 完整根的普通结果返回与 scope 恢复；Type 仍限于现有载荷 | 保留完整 Type 数据，再由描述或身份消费者准入；与原源码区域及 VM activation 交接，不恢复旧队列旁路 |

共同冻结载体需要独立表达“这个值属于 Type”与“这个载荷是具体类型身份”。不能
靠将 Any 声明为 `Primitive`、沿用当前按 `Sort::Type` 接入身份等式的路径，或删除
列表字段的 sort 检查来迁移；这些做法会使 PTC、布局或正常 NF 参数比较误收描述为身份。
旧具体身份可以由单向适配器注入新 TypeValue 通道；反方向转换必须显式要求身份，
并允许等待或拒绝，不能提供不检查的 `Description -> Term` 转换。

冻结与解冻只保留数据及其合法依赖，不保留调用方的准入成功。持久编码、哈希、
scope 检查、substitution、预算计费和 Display 都必须按同一载体迁移；某个描述尚
不能合法冻结时明确拒绝该传输边界，不能丢弃 refinement 后缓存更宽松的结果。
这也不要求所有 invocation 都缓存：仍只有既有策略允许的 Type 结果可参与，且
其纯性、输入键、世界依赖及可传送条件必须成立。

先验证不涉及格式器的普通 Type 返回、转发与容器往返，再以两项不同实际类型的
延迟参数检查生产与准入分离；最后接通 std 的真实入口和 native 输出。各层应保留
拒绝、取消与恢复证据。公开格式化接口未接通前，`FormatPlan.parse` 仍只是扫描器，
现有 `println(comptime text: str)` 仍是待替换接口，不能通过改名报告目标已完成。

## 9. 共同冻结载体与 binder 的实施选择

这一节是实现方案，不是已经存在的 Rust API。按 `627ab804` 核对，真正需要替换
的不只是 `Value::Type(Term)`。当前三个边界共同把 Type 数据等同于身份项：

- `nir::comptime_values::encode_admitted_list` 依据元素域是否为 Type，要求元素分别
  使用 `Sort::Type` 或 `Sort::ComptimeValue`；其 `validate_scoped` 则明确拒绝 Type 域。
- `normal_values::normal_pattern_value_at` 在接收域为 Type 时，把输入 Term 检查后
  直接包成 `Value::Type`；`normal_terms` 反向导出同一身份载体。
- `types::input_binding::input_sort` 在已形成参数域为 Type 时，将 comptime 槽位
  解释为 `Sort::Type`；`replace_scope` 随后按这个 sort 替换。仅修改列表元素检查，
  仍会在普通函数输入绑定处丢失描述，或把描述错误送入身份替换。

此外，`Term::NormalForm.arguments`、名义实参、函数值实参和普通聚合字段都还是
Term；[现行 wire](../compiler/luna/src/build_context/metadata/wire.rs)直接编码这些
字段。因此不能给 List 单独加一个 Description 元素分支，也不能给 Type 返回额外
开一个不经过普通参数绑定的入口。

### 9.1 数据边与身份边分开建模

采用[通用值传送](comptime-value-admission.md)中的带域冻结图，Type 载荷是其中
一种普通数据。以下表示各模块必须共同保留的信息，不规定内存布局或新增语言类型：

```text
FrozenValueRef = graph owner + node
FrozenValueNode = checked value domain + payload + stable dependencies
Payload = scalar / aggregate / callable / type-data / other admitted data
TypeData = identity(TypeRef) / description(DescriptionRef)
Description = explicit-any
            / construction(checked head or constructor, ordered operands)
            / refinement(base, scoped requirement)
Operand = frozen value / authorized scoped input
```

图引用按拥有它的冻结单元解释；数字节点索引不是跨世界身份。TypeData 自己的值域
是 Type，载荷才是被描述的类型。装有 Any 的 Type 值不是一个“域为 Any 的未知
值”，更不是未完成的计算。普通聚合的每条元素边都指向值节点，保留该元素自己的
域；列表的声明域及元素协议仍由普通形成检查提供。

只把已完成且获准传送的数据放入 FrozenValue。`Operand` 中的 scoped input 是
明确绑定的符号引用，用于合法的开放描述，不是伪装成完成值的待执行 producer。
引用必须带可核对的 telescope 及声明来源；缺少域或归属的输入继续留在活动任务。
描述含有合法符号引用时也不能据此作为闭合缓存结果或 runtime 布局输入。保留
合法 runtime 依赖身份不授权 comptime 读取其值；对应环境仍按
[TypeRef 与环境取得](type-references-and-environments.md)交付。

身份消费者继续接收已检查的 TypeRef／身份项。它从 TypeData 取得身份要经过显式
转换；Description 不提供不检查的 `as_term`。具体描述完成该消费者要求的形成
与义务交付后，才可向旧身份路径交付；仍含 Any、缺少输入或无法形成合法交付时，
返回该消费边界的等待／拒绝原因。需要保留到后续程序检查的义务不能被丢弃，也
不能误要求所有 runtime 谓词在 comptime 证明。数据转发消费者无需取得身份。

内部继续可以对身份、普通值和 runtime 输入使用不同 sort；这不是要求把所有
`Sort::Type` 改为 `Sort::ComptimeValue`。应删除的是“Type 值出现在哪里，就天然
是求解器类型项”的隐含转换。PTC 的 Type 变量、相等及 congruence 入口只消费
身份视图，不直接消费 FrozenValue 或 Description 节点编号。

NF 输入依旧按函数签名接收普通值。具体身份中的稳定参数以受检值引用保留，而不再
要求每个普通值都借用身份 Term 编码。含描述输入的构造先形成描述应用，保留原头
与完整输入；不能为了构造 TypeRef 丢掉 Any/refinement，也不能执行 NF 头的主体。
已获得的具体身份可适配进新通道，旧 Term 编码只能作为**具体子集**的边界适配器。
没有表示能力时明确失败，不得选一个具体类型替换描述后继续输出旧格式。

### 9.2 普通调用先绑定数据，再按用途消费

同一个 Type 输入可能既被返回、放入容器，又被用作后续参数的类型。调用计划应
保存一次已检查的数据供应，再为每种用途创建消费者，不能在供应点就强制解析身份：

| 使用点 | 需要的检查 | 不应发生的动作 |
| --- | --- | --- |
| `comptime T: Type` 的实参供应 | 实际值属于 Type，描述来源及 scope 合法 | 因 T 的域是 Type 就直接赋值 PTC meta |
| 返回 T、保存局部量或列表元素 | 按接收边界传送同一 TypeData | 再执行生产者、选择 Any 的具体实例 |
| 后续参数 `value: T` | 用 T 的描述在该调用方检查实际输入及要求 | 使用定义侧假设证明调用方已满足 trait |
| 需要布局、ABI 或具体类型相等 | 取得该消费者获准使用的具体身份与相关环境 | 把相容关系变成身份相等或任意选布局 |
| 保存 NF 应用 | 绑定普通实参、保留头及数据依赖 | 按 Tuple/List 名称决定参数种类 |

这要求输入槽位分开保存阶段、已经形成的值域和供应状态。阶段为 comptime 不等于
域已经是 Type；取得 Type 域也不等于其实际载荷是具体身份。现有
`FunctionInputBuilder` 的前缀域检查、失败不更新前缀、原始位置和去除 staged 槽位
的职责仍须保留，但替换操作需区分“数据引用替换”和“身份使用处形成”。

例如前置域计算依次得到 `Type`、`bool` 时，后续供应按不同域检查；收到 bool 值
之后，若再把它用于类型位置，须在该位置拒绝。不能通过给 pending 槽位预填 Any
绕过这一检查，也不能让 Type 值转发因为尚无具体布局而失败。

### 9.3 替换、refinement 与共享图

已实现的子集是只引用自身 subject 和已解析声明的类型命题。定义端为每个参数
单独引入刚性 subject，调用端在自己的上下文中实例化并证明要求。VM 以共享
快照中登记的不可变引用检查来源，另一个编译世界即使槽号相同也不能接收。
含描述的闭合 NF 构造保留原头及每个普通实参的值模板。模板的私有槽位只在准入时匹配，
不会进入具体类型身份。外层自由输入的重定位与可执行 predicate 的捕获仍待实现。

描述的替换必须走共同的有作用域遍历，并同时处理节点域、构造实参及要求代码。
原 TypeTerm 的 binder 算法可以服务身份叶子，但不能只替换叶子而漏掉外围数据图。
每次遍历显式持有源 telescope、目标 telescope、替换表和所属世界；缓存键包含
这些环境，不能仅按节点编号复用另一调用的替换结果。

refinement 的 Itself 是要求自己的 subject binder。外围参数替换进入要求时要
跨过这一层，不能把 Itself 与同号外部参数混淆。要求保存受检表达式、自由依赖及
适用的阶段；构造时检查它能够合法保存，准入时才把 subject 绑定到当前实际输入。
PTC 只接收其中可提取的类型命题；程序谓词由程序验证／规定的执行检查处理。

把这层 binder 接入现有源码时，有三个相互依赖的改动。首先，
`completed_type` 对具体 base 可以继续检查现有要求；对描述 base 要在新的
subject 作用域内形成要求，然后连同 base 返回。既有 `extracted_refinement`
已经决定哪些要求可以表示为类型命题。可执行要求要保有它自己的代码及词法输入。
其次，VM 的 `has_type_at` 目前允许闭合的 Any 叶进入 comptime 的 Type 域。
带要求的描述必须在此检查捕获是否合法，例如一个 trait 引用在接收世界的声明
归属；这里还没有实参，无法选择该实参的 Display 实现。最后，开放调用在
取得实参域、默认供应域或返回目标时才有 subject，此时才能在接收方作用域
代入并检查所保存的要求。这个检查须发生在发布带具体签名的函数实例之前。

共享描述数据不共享准入状态。图中两个元素可指向同一份显式 Any 数据，接受它们
的两个槽位仍分别检查实际类型；刚性类型 binder 的重复引用则继续保留同一个
声明身份。节点去重不能把这两种关系都实现成“为该节点创建一个推断变量”。
已经准入的实例也不能把实际类型写回共享节点，影响另一个独立调用。

遇到局部声明引用时，传送计划必须保活合法来源或提供受检重定位；没有这种能力
就拒绝该传送边界。TypeData 中有 nominal 引用不使 nominal 定义自身成为描述图
的递归子节点。冻结的描述和值数据使用有界 DAG 遍历；活动可变存储及其循环继续
交给现有存储传送规则，不能借 Type 描述图获得任意冻结许可。

### 9.4 编码相同、类型相等与可缓存分别判断

描述图可以按规范化数据编码比较／去重，目的仅是共享不可变存储和记录依赖。
这不定义源语言 Type 的 Eq/Hash，不向 PTC 添加相等式，也不决定高阶函数公开
参数名是否参与 Type 相等。后者仍按[公开入口设计](callable-input-interfaces.md)
保留尚待决定的边界，不能借此次换载体悄悄作出选择。

冻结与解冻共享节点、深度、字符串／整数载荷及依赖数量的预算。遍历先收费再分配，
失败不安装半张图；释放未发布图不靠重新补充执行额度。重复子图可共享内部处理结果，
但用户可观察的 comptime 调用次数不因此减少。跨世界导入先核对声明依赖与域，再
让消费者使用；来源世界的一次 impl 选择不自动成为接收世界的证明。

metadata 当前仍是版本化的 Term 协议。新图应有独立、明确版本的编码入口及完整
接收检查，不能把旧 tag 下的 Type Term 偷换成 Description。具体子集的转换可
继续输出旧版本；不能表达的描述必须报告不支持该版本。解码不执行 factory、
refinement 或生产者，不根据相同数字 DeclarationId 授权引用。共享代码缓存保持
现有职责，结果缓存仍单独检查纯性、Type 结果策略、源 Eq/Hash 和传送资格。

### 9.5 一次迁移必须接通的路径

实施从真实源码的普通 Type 值传递开始，同时替换下面五条边。可以分提交落地，
但不得在它们闭合前宣称描述值已经可用：

1. 集中绑定和普通表达式形成 TypeData；alias、函数返回及值引用沿共同 VM 值
   通道保存它，错误值域与来源在入口拒绝。
2. 聚合、普通函数输入和 NF 输入使用带域数据边；同时迁移
   `comptime_values`、`normal_terms`、`normal_values`、`input_binding` 及 scope
   visitor。List、Object、用户 comptime struct 不能各有独立 Type 旁路。
3. 共同输出先发布 TypeData；需要推断赋值的消费者单独取得身份并保留许可检查。
   已退役的测试队列不是生产接线位置，也不要求恢复每次调用独占输出 MetaId。
   参数描述消费者不需要预留 MetaId。消费失败不撤销已完成数据、不重放生产者。
4. 源码注解与待定域检查接收描述，保存原规则后继及调用方环境，再检查具体输入。
   单独给 `AnnotationState` 增加 ReadyDescription 不算接通这条边。
5. 真实 `std.fmt.Parse` 构造带要求的参数描述，println 沿普通调用准备、收集协议
   和 writer 输出；实现前文的定义证据与一次求值约束，保持资源检查有效。

冻结方案还须检查 CMTT 的 Type 数据输入，但不把 Type 描述当作 expr/pattern/arm
的代码见证；代码类别、Ψ 和控制接口仍由其传送协议检查。TypeRef/IIR、程序验证
和后端只消费该阶段合法的身份及环境，不能让新描述图成为绕过形成检查的后门。

这一闭包的最小回归同时包含：描述经普通函数和至少两种普通容器往返；相同描述
用于两组不同实际类型；错误域拒绝后独立调用仍成功；嵌套 refinement 的 subject
及外层 binder 替换；外世界同号引用拒绝；预算失败不发布；旧具体身份子集往返；
以及真实 sysroot 的格式化正反例。已有具体身份和 scanner 的通过记录不覆盖这些
要求。本节未新增代码实现或改变现行 wire 版本。

### 9.6 Parse 的一次数据交付推演

以下是目标内部 dump 的结构示意，声明名只为阅读，实际引用仍须包含世界归属。
`Parse("{} {}")` 完成后，数据可以表达为：

```text
(type-production p0
  (complete
    (type-data
      (description
        (application (head Tuple)
          (argument Es
            (value (domain (List Type))
              (elements
                (type-data (refinement (any) (subject s0) (requires (Display s0))))
                (type-data (refinement (any) (subject s1) (requires (Display s1))))))))))))
```

Tuple 的 Es 是一个完整供应的普通列表值；列表元素各自属于 Type。这里没有把
元素数组当作 Tuple 的特殊参数列表，也没有通过 Tuple 的实现体展开身份。
该结果可先存入局部量，再传给普通函数，之后才出现需要具体输入的消费者。

检查两个 runtime 实参时，参数准备器分别取得它们的实际域，例如 u32 与 bool。
消费者 c0 以这两个域分别检查描述和 Display 要求，通过后形成保留 Tuple 头及
普通 Es 值的具体类型引用，同时保存该调用的交付与资源证据。p0 的数据不被改写。
另一消费者 c1 收到缺少 Display 的实际类型时，在其准入处拒绝；p0 仍已完成，
c0 仍有效。c1 不通过重跑 Parse 或沿用 c0 的 impl 选择恢复成功。

如果 p0 尚未完成，消费者等待其真实执行；如果 Parse 已返回完整数据而 c1 尚未
取得实参域，等待归 c1。二者在 dump 和错误来源中必须可区分。运行时值的源码
执行次序仍由普通调用计划保存，描述检查先后不授权提前执行或重复执行实参。

## 10. 实施闭包与验收

| 闭包 | 必须共同接通 | 完成前不得宣称 |
| --- | --- | --- |
| 描述数据 | 集中源绑定、alias、普通 Type 返回、VM、容器/值引用/冻结与世界检查 | Any 已可供 Parse 使用 |
| 描述应用 | 普通输入、列表、保留头、refinement binder、结构观察 | Tuple<Any> 已能实例化 |
| 生产与准入 | 完成结果、待准入状态、实际参数域/义务、推断赋值许可 | 添加 Value variant 即打通 defer |
| 定义证据 | 可靠的结果观察、inline 主体与待恢复后继 | 只验具体调用即可发布泛型函数 |
| 格式化库 | Parse、收集、writer 输出、错误/清理与实际 demo | scanner 或 println_value 等于 println |

旧 Value::Type(Term) 在所有消费者迁移前不能草率删除，也不能让新旧路径按库名
分流。TypeObservation 继续只描述检查器掌握的信息；增加结构和要求观察时仍须与
已完成数据及生产状态分开。具体 Type 快路径复用共同的形成、传送与预算契约，
不再重复实现。

必须成组验收，以下是未完成要求，不是通过记录：

- Any 与 refined Any 经 const、alias、函数、List、用户 comptime struct、Object、
  值引用和冻结恢复保持含义；错误域和外世界引用分别拒绝。
- 两个 Any 字段可接纳不同类型；重复 T 仍保持同一约束；相容不产生传递等式。
  同一描述图复用到不同调用，不共享实例变量或定义端证明前提。
- Identity<Any> 保留头，不能用普通 bool 冒充；用户工厂与 builtin 共用机制。
- producer 等待/失败/取消及多消费者读取不重放前缀；缓存仅共享合法稳定数据。
- 定义侧拒绝无依据的结构和 trait 操作；非 tuple 工厂不能授权 tuple_enumerate；
  合法结果观察经转发仍有效，改写或依赖变化后重新验证。
- println 的零/多字段、转义、Unicode/NUL、不同数值类型、自定义 Display、具名
  完整 tuple、展开、参数数量/trait 错误、非法格式及 runtime 格式输入。
- 实参次数与顺序、writer 失败前缀和资源清理正确；真实 sysroot、资源检查及
  O0/O3 原生输出一致。

已有 scanner 与准备阶段修复证据见[跨阶段交付](runtime-value-delivery.md)。本设计
来自源码与正式契约核对，不把旧测试数量计作这里的能力已实现。
