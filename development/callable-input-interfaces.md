# 函数值的参数入口、接口与传送

声明调用、函数值的位置调用和动态 NF 调用共享槽位路由。当前 NF 函数类型保存
参数 telescope 和收集器位置，公开参数名及默认提供者的高阶传送仍待实现。

## 1. 语义依据与不能混同的身份

[参数规则](../docs/functions/parameters.md)与作者在
[056 的补充](../docs_chat_history/056-functions-and-closures-01.md)确认：optional、
implicit、变参及拓展变参都可按 `.name = value` 供应完整值。直接供应不再运行
该位置的缺省供应或收集协议；仍须检查实际域、阶段、适配和契约。

[依赖函数类型](../docs/functions/higher-order-functions.md)要求局部依赖名字按位置
抽象，一致重命名不改变依赖绑定。`T: Type, value: T` 中的局部 T 与
`...Items: meta.List<Type>` 中对调用者公开的 Items，承担不同职责；后者同时也是
一个签名内绑定，不能把这两种用途混成单一字符串身份。

[调用规则](../docs/functions/calls-and-returns.md)要求 callee 先取得，显式实参随后
按源码顺序各求值一次；不能因编译器看见函数来源，就重新获得没有传送的参数协议。
[comptime 方法](../docs/functions/comptime_fn.md)还明确：接口的缺省输入在接口处
取得，再把完整输入交给实现，不能换成实现主体声明的另一份默认值。

因此实现必须区分：

| 信息 | 回答的问题 | 不能据此推导 |
| --- | --- | --- |
| 参数 telescope | 每槽的 sort、域、绑定及结果依赖是什么？ | 公开名字、缺省供应和收集方式 |
| 公开调用接口 | 调用者能按什么名字和方式供应各槽？ | 实际代码身份或主体已经可执行 |
| callable 值身份 | 选中了哪份代码、声明实参与环境？ | 可以替换公开接口的默认值或要求 |
| 调用实例的供应计划 | 原输入怎样交付给本次接口？ | 已预留的槽已经求值或完成准入 |

本设计不从“局部绑定可以改名”推出“公开入口名也不可观察”。也不反过来把所有
绑定拼写加入 Type identity。作者本轮确认：**允许按槽位适配**。当参数域和槽位
相同时，实现的 `...Items` 可以交付到声明 `...Es` 的目标接口；调用者使用目标
接口的 `.Es = value`，默认供应也由目标接口决定，不能借用实现的另一份默认值。
适配与 alpha-equivalence 分开判定；这条决定不自动规定两个接口 Type 的相等关系，
也不能仅修改 Term 的 PartialEq／Hash 来代替接口传送与准入。

### 接口关系不能由现有 Term 相等代替

在 `9e3509d7` 上，局部依赖名的改写和公开收集入口的改写都得到相同 Type：

```flurry
typealias DependentLeft = fn(comptime T: Type, value: T) -> T;
typealias DependentRight = fn(comptime U: Type, item: U) -> U;

typealias CollectedLeft = pure comptime fn<...Left: meta.List<Type>> -> Type;
typealias CollectedRight = pure comptime fn<...Right: meta.List<Type>> -> Type;
```

前一组体现正式文档已确认的依赖绑定重命名；后一组只能说明当前表示没有入口名。
后者不能证明 `.Left` 和 `.Right` 已经可以互换，也不能证明语言要求两组接口
成为不同 Type。完整[观察程序](repros/callable-interface-name-observation.fl)分别
返回两个比较结果，没有把第二个结果写成必须继续通过的语义测试。

迁移时必须分别回答三个问题：两个参数 telescope 是否等价，目标公开接口允许
哪些调用，以及实际代码能否接收该接口交付的完整输入。代码身份相同不保证接口
相同，代码身份不同也不妨碍实现同一个接口。按作者已确认的适配规则，检查后的
槽映射应说明目标名字如何交付，不能在每次动态调用中读取所选主体的名字。
保留下面的同名用例作为基础对照，再覆盖实现名和目标名不同的转发；不从允许
适配推出两个接口 Type 必须相等。

## 2. 当前信息在哪里丢失

| 生产者／消费者 | 当前事实 | 必须迁移的边界 |
| --- | --- | --- |
| `elaboration/program/function_types.rs` | NF 变参改作依赖位置参数，只额外保存 collector 索引 | 形成 callable Type 时保存公开供应描述，依赖绑定仍按位置抽象 |
| `nir/types.rs::Parameter` | 仅有 sort 与 ty | 不能把这份 telescope 当成完整调用接口 |
| `nir/program/function.rs` | FunctionDeclaration 独立持有受检声明字段，Function 组合声明与 body；`callable_type` 仍只合成 telescope 及阶段／责任修饰 | 所有权已拆开；公开供应协议及源码模板发布仍须迁移，不能把受检声明结构当成协议传送完成 |
| `elaboration/program/input_routing` | 入口描述、完整值预留、源码实参位置已共用 | 当前 Entry 是路由投影，不包含默认提供者、协议证据及世界依赖，不能直接当完整发布格式 |
| `elaboration/program/normal/dynamic_inputs.rs` | 动态 NF 的位置输入已进入共同准备器；callee 先保存，固定包、收集和依赖域沿同一计划检查 | 接入具名槽位及默认供应的公开接口 |
| `comptime_interpretor/callable_domains.rs` | 从接收世界取得声明／方法的签名，且不执行源码 | 返回的准入依据须包含相应公开接口；缺失与预算失败继续分开 |
| `nir/value_admission.rs`、VM `value_types.rs` | callable 准入比较签名／关联类型视图 | 不能只证明 telescope 相容，就接受缺少供应协议的值 |
| `luna/build_context/metadata/wire.rs` | tag 35 只传 NF 签名与可选 collector 索引 | 不能让新接口通过旧格式静默丢掉入口、默认值或协议依赖 |

这不是新增一个 `name: String` 就能闭合的修改。编译器在局部变量上记录名字，
但函数返回、类型别名、列表／记录载荷、生成定义或 metadata 传送不携带它，
就会再次出现“同一个值在不同入口具有不同调用能力”的偏差。

显式写出接口的常量调用已接通按槽适配。这里 `.Public` 是调用入口，形成的
不透明类型仍为 `Head` 的应用。表达式和类型标注都接受 `.Public`，都拒绝
未在该接口声明的 `.Actual`。

```flurry
pure comptime fn Head<...Actual: meta.List<Type>> -> Type { bool }
const Factory: pure comptime fn<...Public: meta.List<Type>> -> Type = Head;
typealias Selected = Factory<.Public = [u8]>;
```

`static_normal_application` 对一次静态调用保留两个声明身份。输入准备器从公开声明
读取供应协议，从实际头取得有序域与形成约束；类型标注直接消费同一份受检计划。
将带默认参数的头交给必填位置接口后，调用者须显式供应该槽。实际头的 `requires`
仍用这些实参检查。该路径涵盖显式写出 NF 函数签名的模块常量。经类型别名、函数
返回或参数转发的接口还缺稳定表示；仅凭完成的函数值查到实际头，会重新暴露实现名。
[静态调用选择](../compiler/elaboration/src/program/normal.rs) · [两种位置及双执行器用例](../compiler/luna/tests/declaration_input_plans.rs)

### 必须随载体迁移的消费者

继续沿 `9e3509d7` 的实际消费链核对，载体变更还必须覆盖以下边界：

| 消费者 | 当前行为 | 迁移要求 |
| --- | --- | --- |
| `nir/normal_forms.rs::Head::callable_type` | 从参数域重建 telescope，只附 collector 槽位；Head 本身也没有公开入口 | 源码声明和 builtin 集中绑定都交付稳定接口，不能只修改函数类型表达式的生产者 |
| `ptc::constructor`、`model_domain::closed_type` | NF 构造子仅区分 collector 槽位；模型域另行检查签名形状 | 若接口限定参与类型命题，构造子分解、模型准入及重建都须保留它；不能只改 Term 的 Eq／Hash |
| `type_resolution::implementation::Key`、`matching`、`associated` | 索引头、匹配条件及关联项重建都仅传 collector | 接口不能在索引、规范化或替换后消失，也不能制造与 PTC 不一致的相等判断 |
| VM `callable_domains`、`value_types` 与 NIR `value_admission` | 从接收世界取得实际签名，再检查 callable 域；冻结描述与活动值有各自入口 | 两条准入链都要检查所需接口依据，不能只补源码调用或只补冻结值；失败和 Budget 继续区分 |
| metadata `wire` 的 tag 35 | v1 编码仅含签名和可选槽位；现有往返测试只证明这些字段被保存 | 新接口使用明确的格式判别及对应版本入口，旧包不能被默认为拥有未传送的公开入口 |

这里不据“当前缺少接口字段”推断 CodeSession 已发生跨世界污染。
`code_session.rs::matches` 使用实际不可变 world 的指针身份，独立 world 不共用
会话中的代码缓存。新增接口必须成为相应 world 的稳定内容，并由元数据编码和
调用准入共同保存；不能另加一个会话外可修改的名字表来规避 world 所有权。

首条迁移的完成条件因此是生产、替换、求解、准入、调用及传输共同保持同一个
接口。只让 `.Items = items` 通过路由、只新增一个未被消费的接口结构，或仅让
新的 Term 能往返编码，均不能计为这条纵向交付完成。

### 共同准备器的路由边界

在 `cfab85f6` 后的迁移中，`declaration_inputs::routing::Frame` 的构造直接接收
参数入口描述与域 telescope，不再接收源码 owner 或读取 ClauseParam。源码入口
在准备器外层将已确认声明投影成 Entry；公开名字仅来自这份投影，不能为了布局
推算而临时创造一个名字。默认供应的源码环境仍由后续槽位检查持有，这一步没有
把整个准备器或默认提供者转换为可序列化接口。

固定位置上的 `...(T, value)` 将两个字段分别交给对应声明槽位。检查后一个字段时，
可以使用第一个槽位已经取得的类型。若该类型要由原 invocation 产生，NF 输入沿用
延后检查的字面量表示。计算得到的包则只执行一次，通过外层 tuple pattern 导出字段。
路由游标消费包的实际字段后才决定下一项展开进入固定前缀还是收集器。因此，即使
前一个包的宽度要检查局部绑定后才知道，后一个列表展开仍能取得收集器的元素域。
包宽提示只用于给计算包提供上下文。

被具名供应隔开的收集元素在其源码位置用一个普通局部绑定保存，连续尾部的元素直接在收集器中执行。
计算包的准备动作记录原实参位置，零字段的计算包也占一个执行位置。收集器按源码次序合并这些动作和需要保存的元素。VM 只导出最外层绑定，生产者内部的临时变量留在原词法作用域。[共享路由](../compiler/elaboration/src/program/declaration_inputs/routing.rs) · [输入计划用例](../compiler/luna/tests/declaration_input_plans.rs)

`c127b11a` 后进一步将
默认提供者的源码 owner、槽位和表达式从入口投影中分离：普通位置输入不需要
源码 owner，只有真正进入默认供应时才安装提供者环境及递归 guard。活动投影
保存源码引用，因此仍属于检查区域，不能当成完整 callable 接口冻结或发布。

动态 NF 位置调用已接入该准备器，原先临时合成 HIR 列表与仅接收单个展开的分支
删除。参数 telescope 的位置绑定打开为本次准备私有的依赖键；Progress 共享原始
参数域，使用带预算的 DeclarationSubstitution 兑现已观察输入，不再按这些私有
键回查源码声明域并默认成 Type。普通源码、名义类型和动态调用共同使用这一域
替换边界，默认代码连接也沿同一规则进行。

动态位置调用保留为 NIR 的 `apply-normal-form`。callee 在原调用环境中先求值
一次。其公开位置签名给出本次输入的域和依赖，实际选择的声明给出形成契约
及最终 NF 头。两种执行器都按照源顺序推进输入计划。字节码先将 callee 存入
寄存器，再将计划所需的自由绑定交给输入帧。普通 `Call` 只处理函数调用；
阶段检查和执行入口拒绝将 NF 头交给它。普通字节码调用帧因此只保存值实参，
无需为构造类型保留另一套参数冻结、槽位排序和执行分支。

例如 `factory<...prefix(), 1, 255>` 的固定包可以到执行时才产生 `u8`。
帧在这个域下检查两个文字并构造收集值。操作数中的提前退出仍回到原调用，
选中声明的函数体不参与类型构造。

具名动态供应和完整公开接口传送仍待实现。按槽位适配后，调用点的名字与
默认供应由目标接口提供。普通函数列表实参等待后续 Type 的恢复进度另见
[审查回复](audit-reply.md)。

当前 NF 元数据和 VM 形成服务仍只交付 Type 结果；动态准备入口对非 Type 结果
保留明确的未实现诊断。这是当前实现范围，不是语言要求所有范式头只能返回 Type；
后续普通值范式须同时接通结果域代入、值表示与准入，不能执行头主体代替。

### 已接通：共同的声明 telescope 构造

公开入口传送尚未实现；其下层的依赖绑定形成已从两个独立循环迁到
`nir::callable_types::DeclarationTelescope`。源码普通／comptime 函数类型和 NF
元数据共用这一构造器，源码／集中绑定的 NF 值直接借用声明域，不再为取得签名
先复制 Head 的域、要求和能力列表。NF 仍保留原头，构造过程不执行主体。

每个参数先对已完成前缀抽象其域，成功后才发布自己的绑定；失败不改变前缀。
前缀索引在构造期间复用，不再为每个参数重新建立。`Term::abstract_parameters`
也把外层 Bound 位移和 Rigid 转换合为一次重写：已有内层绑定保持不变，外层引用
跨过新增帧，刚生成的位置绑定不再被位移。普通 Type、值和 runtime 输入沿同一
规则处理，参数名、收集协议及默认提供者仍属于上层接口。

构造和最终 NF scope 检查使用调用者额度；VM 接收世界、类型解析的声明查询、
NIR 阶段检查及 materialization 已接入。阶段检查不再用 `.ok()` 吞掉构造失败，
预算失败保持 Budget，实际声明不匹配才报结构错误。Head 的无额度便捷入口仅给
独立调用提供有限默认值，生产消费者使用有额度入口；没有新增签名结果缓存。

这是共享绑定机制和失败传播的迁移，不是公开接口传送完成。构造器的 Display
采用 S-expression；回归覆盖嵌套作用域、混合 sort、失败后重试、错误域、过深输入
及 2,000 个参数的有限构造。该宽度用例检查共享额度内可完成构造，不是计时基准，
也不把整个类型检查过程宣称为线性复杂度。

形成依赖函数签名时，已经读过的外层声明身份不能再分配给本层参数。例如第二个槽的域
是 `fn() -> Tuple<T, Outer>`，第一个槽绑定 T，Outer 来自外围。打开这份签名只应重命名
T。给任一槽分配 Outer 的身份都会把两种引用合并。源码签名一次预留本层身份，嵌套签名
使用另外的范围。动态 NF 调用和 builtin 集中绑定使用同一个打开操作，在域的结构遍历中
检查新身份与捕获是否互斥。形成、打开、再形成的往返用例检查嵌套引用及完整签名。
[声明侧形成](../compiler/nir/src/callable_types/telescope.rs) · [嵌套作用域往返](../compiler/nir/tests/normal_form_callables.rs)

方法跨阶段贡献时，两侧已有完整的声明参数组。比较器分别为它们建立一次“声明身份到槽位”
的索引，再按各自的索引比较域。外部身份保持原义。elaboration 比较 capability 实参和 VM
接收改名的方法前缀时共用这套抽象。512 个槽的检查以 10,000 步额度验证不会按每槽重复
构建整张索引。这个比较消费声明参数的域，公开调用名和默认提供者的传送仍待接入。
[完整参数组](../compiler/nir/src/function_parameters.rs) · [阶段贡献](../compiler/elaboration/src/program/interfaces/phase_contributions.rs)

接收方作用域、额度与失败恢复可分别查阅 [NIR 输入用例](../compiler/nir/tests/normal_inputs.rs) 和 [接收世界查询](../compiler/type_resolution/tests/callable_world.rs)。

## 3. 稳定接口与活动计划

建议由一个稳定的 callable 接口描述持有以下信息；字段组织由实现选择，不要求
另建一套与现有 Term 完全重复的类型树：

```text
CallableInterface
  telescope                 参数与结果域，含 binder scope
  supply_entries            公开名字 → 参数槽及允许的供应方式
  providers                 默认／隐式／收集协议的受检引用与输入映射
  phase_and_responsibility  调用阶段与 unsafe 等责任
  contract_dependencies     类型、程序、资源契约及所属语义世界
```

公开名字必须进入接口自身的稳定描述和缓存依赖。不能只用裸 telescope 的 intern
句柄建立全局侧表：两个同形签名可能来自不同公开接口或缺省提供者，后写入者不能
覆盖前者。保存 callable Type 的产物也必须保留可供调用所需的接口描述，不能只在
值表达式上临时挂一个名字表，再在 Type 返回或别名展开时丢弃。

这里的接口身份、Type 的语义相等和 callable 值的代码身份是不同关系。实现应为
这些关系分别提供受检查询，不能把“代码签名相等”同时用于证明三者。若最终采用
Term 的接口限定或独立的受检类型描述，两种布局都须兑现上面的传送契约；不能先
以忽略协议字段的 Eq／Hash 发布，再期望之后从源码补回来。

默认提供者不是 `FnParam` 引用或一个活动 Checker 闭包。它应引用稳定源码模板或
已检查代码、定义作用域及前置参数槽；具体调用再建立独立环境。提供者仍待形成时，
保留其检查义务，不能把它作为已经可执行的默认值发布。运行时默认表达式的 P3 与
有限完成要求、调用失败和清理规则不因保存函数值而放宽。

活动 ArgumentPlan 从该接口导出 Entry，再使用已有 Cursor 路由。它另行持有
callee 取得位置、源码输入位置、所需域观察、执行后继和已交付值；这些状态不冻结
进接口，也不跨 invocation 共用。未知参数域交回
[源码检查与执行交接](elaboration-state-and-publication.md)，不能填入假 Type／Any。

普通源码函数、内置集中绑定、trait 接口和 builder 生成的函数都必须生产这份契约。
不为 Tuple、List 或生成器开第二种接口格式。closure／quote／effect 的主体资格和
阶段限制继续依据原规则；承载参数协议不自动扩展 lambda 可声明的参数种类。

## 4. 值传递与调用规则

值交付先检查目标接口，再保存实际代码及环境与该接口的对应依据。校验至少覆盖
槽的数目、sort、binder scope、依赖域、阶段、调用责任，以及所需供应协议是否存在。
实现应保留原接口依赖，不能凭另一个世界中相同的声明编号复用证据。

显式接口决定对调用者公开的入口。接口不提供某个名字时，即使 callee 恰好指向
一个有该名字的源码声明，也不能从实现偷取入口。需要改变视图时，应经过明确的
接口相容性与槽映射检查。名字不同本身不阻止适配；实际域、阶段、责任和契约仍须
满足目标接口。适配后只暴露目标入口，实现的原始名字不自动成为附加入口。

默认值属于本次公开接口。若接口明确给出提供者，应使用它，而非切换到所选实现的
另一份默认值。若提供者必须随实际 callable 才能确定，产物须显式携带对应依据和
合法的派发路径；不能在编译期猜测运行时选择，或先执行 callee 来反查声明。
这项语义需求不预设所有函数值都是 fat pointer，也不改变已经确认的 trait object ABI。

执行仍遵循 callee 先取得、显式输入按源码次序取得的规则；未显式供应的提供者
按既有默认准备及阶段规则执行，形成与调用义务成立后才进入主体。检查就绪次序
不等于源码求值次序。完整收集值直接交付对应槽，不展开再收集。
NF 在完成这些输入和形成检查后构造不透明头应用，不能执行头的函数体来补齐接口。
提前退出必须取消未执行的后继，原输入与默认值不能在重试时重放。

## 5. 首条纵向迁移及删除条件

从固定的、已形成域的 NF callable 接口开始，用现有公开 API 打通整个传送路径：

```flurry
typealias Factory = pure comptime fn<...Items: meta.List<Type>> -> Type;

pure comptime fn First<...Items: meta.List<Type>> -> Type { bool }
pure comptime fn Second<...Items: meta.List<Type>> -> Type { bool }

comptime fn select(flag: bool) -> Factory {
    if flag { First } else { Second }
}

comptime fn forward(factory: Factory) -> Factory { factory }

comptime fn Apply(factory: Factory, items: meta.List<Type>) -> Type {
    factory<.Items = items>
}
```

两个头使用相同公开入口，先作为保存完整接口的基础对照。作者已确认的槽位适配
另用同域的 `...Items` 实现交付到 `...Es` 目标接口验证：`.Es = items` 成功，
通过该目标接口调用 `.Items = items` 拒绝；返回、选择和转发不得重新暴露实现名。
应同时接通公开 Tuple 集中绑定对应的同名接口，不能以用户源码头必须有 body
为前提恢复参数描述。第一阶段覆盖完整值供应，并保留已经合法的位置收集与展开；
未迁移的协议必须保留原有明确限制，不能以此切片宣称所有高阶默认值已经支持。

| 必须一起完成 | 验收证据 |
| --- | --- |
| callable Type 形成、别名与接口引用 | 同一接口经过 Type 别名与函数返回仍有入口；局部 binder 改名不捕获外层变量 |
| 表达式、参数、返回值和聚合传送 | 直接与高阶调用一致；两个所选头的类型身份分别保持；不依赖静态源码来源 |
| 接收世界准入与实际执行 | 错误域、阶段、scope、缺失协议及过期世界拒绝；预算失败不作为“不匹配”静默继续 |
| 统一输入计划 | `.Items = items` 一次交付；callee 和每个 producer 各一次；重复／混合供应拒绝 |
| 代码及接口发布 | 未完成域与提供者不能进入可执行快照；错误后新调用不继承半成品 |
| 序列化与缓存 | 新契约版本可往返；旧输入不能伪装成具有新协议；接口变化使相关键失效 |
| 可读性与退役 | 接口、槽映射、提供者引用和活动供应状态分别有稳定 S-expression dump；动态 NF 的独立路由／列表合成在消费者迁移后删除 |

随后才扩展 optional／implicit、拓展收集和运行时动态 callable；默认提供者、
程序／资源义务与阶段派发必须随对应消费者一起实现。不能先把旧函数值拒绝检查
删除，让没有协议的普通签名冒充完整接口。也不能通过全局名表或库头特判缩小这项迁移。

## 6. 当前源码证据

[完整复现](repros/dynamic-nf-interface-forwarding.fl)覆盖类型别名、两个 callee 的
选择结果、高阶返回与再次传参。只把 Apply 内的 `factory<.Items = items>` 改为
`factory<...items>`，得到其位置展开对照；两者的域、头、分支与参数生产均相同。
具名版本仍是预期合法但尚未实现的程序，不把其失败改成规范拒绝。

在仓库根目录可以分别运行源码检查和真实 sysroot 的输入计划验证。

```sh
python3 -B development/check.py --timeout 900 -- run -p luna -- check development/repros/dynamic-nf-interface-forwarding.fl --sysroot library --build-target x86_64 linux 64
python3 -B development/check.py --timeout 900 -- test -p luna --test declaration_input_plans --test type_spreads
```

当前真实 sysroot 在第一条命令的 `factory<.Items = items>` 处报 `unknown named normal-form argument`。仅将该处改为 `factory<...items>`，执行 `run` 得到 `(bool true)`。这个对照表明位置展开已经接通，高阶调用仍丢失公开入口名。第二条命令检查已有的位置路由、生产者顺序、依赖域及收集边界。
