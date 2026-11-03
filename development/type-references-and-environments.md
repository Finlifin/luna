# 类型引用、环境交付与 IIR 入口

状态：目标设计，未实现。源码核对基线 `1dc25be5`。
依据为[两层类型表示](../docs/compilation/runtime-dependent-types.md)、
[依赖替换](../docs/type/substitution-inference.md)、
[资源检查阶段](../docs/resource-management/contracts-and-verification.md)、
[闭包环境](../docs/functions/closure-environments.md)及[自动清理](../docs/resource-management/drop-elaboration.md)。

本文补齐的不是另一种 Type 容器，而是“生产依赖—形成引用—交付环境—维护证据”的完整边界。
类型里有 RuntimeId，并不表示任何消费者已经知道如何安全取得对应对象。

## 1. 当前生产能力与缺口

下面路径均以 `compiler/` 为根；位置说明当前实现，不作为语言规则。

| 入口 | 当前可用机制 | 不可据此宣称完成的内容 |
| --- | --- | --- |
| `nir/src/types.rs::TypeInterner` | 受预算的 core intern、按显式域抽象 runtime 依赖、失败不安装 core | 不负责从源码推导正确依赖域，也不证明环境可交付 |
| `nir/src/types.rs::TypeRef` | sort/shape/scope、顺序域回调、无捕获替换、依赖收集 | `bind` 不是所有语义契约检查；可用集合不是访问路径或存活证明 |
| `nir/src/program.rs::Expr/Capture` | 类型仍主要由 Term 保存；已有身份与控制结构可复用 | 普通表达式、捕获、调用尚未统一消费 TypeRef 与环境证据 |
| `iir/src/lib.rs::CallableIdentity`、`template_storage.rs` | 以 TypeRef 保存已完成且无环境的签名 | 不能放宽 closed_signature 或表存储前提来冒充一般依赖 callable |
| `iir/src/lib.rs::Package` | runtime_bindings 集合、completed_type、关联项和阶段检查 | 集合只证明身份已登记，不能覆盖逐使用点的域、取得、初始化、世代 |
| `elaboration/src/materialization.rs` | 向实例化传递 runtime identity 集合 | 尚无通用 EnvironmentPlan 生产/消费协议 |
| `parse/src/expressions.rs`、`ast/src/lib.rs` | lift 前缀与 LiftType AST | `ast_lowering/src/expr.rs` 尚无对应专门分支，不能把词法支持当全链路支持 |
| `elaboration/src/program/effects.rs` | 对未接通的 runtime-dependent effect request 明确拒绝 | 普通请求内部创建符号参数不证明一般源码 lift 路径已接通 |
| `program_verification/src`、`luna/src/cleanup.rs` | NIR 责任、来源、效果与实际清理处理 | 新环境不能绕过这些消费者，也不能只在 IIR 增加一张无使用者表 |

`TypeRef::check_with` 的语义回调与 `check_runtime_bindings` 的有限职责在当前注释中已明确。
重构应让调用者交付其缺失证据，不能删除检查或改注释将有限检查包装为完整准入。
`lift_lexical_type` 是外围参数重绑定 helper，不是源码 `lift` 特性的完整实现。

## 2. 先固定检查阶段

正式设计要求：**safe 的全部检查位于 normalized IR；verified 检查位于 instantiated IR。**
因此 normalization 产物必须带上依赖域、环境要求、语义清理、调用契约及 safe 证明。
ordinary 必检义务不能留到某个具体实例恰好满足才放行。

这不要求 normalization 预知 runtime 地址或读取未来对象内容。定义端以符号对象、
契约和程序状态关系证明安全；获准的 runtime assumes 在该层成为明确检查及条件事实。
调用方仍要按被调用接口兑现入口条件，不能把签名承诺当成自己的证明。

实例化解释合法残余及本次替换，并维护已完成的 safe 证据；不得自动重新选择更宽松的
前提。新生成代码仍须经过相应 normalization/safe 检查，不能因为出现在实例化中而豁免。
IIR 接收已完成类型与资源交付关系，verified 模式在此进行相应高级证明。
后续变换校验或重建被变换影响的证据，这是正确性维护，不是推迟源程序 safe 检查。

## 3. 身份、域、存储和取得路径分开

| 数据 | 表达什么 | 不表达什么 |
| --- | --- | --- |
| RuntimeDependencyTerm | 当前实例中的依赖 binder/外部对象身份及受检重绑定 | 对象当前内容、某个机器地址 |
| DependencyDomain | 该依赖的类型/阶段/契约及前置域依赖 | 当前位置可读、可写或可释放 |
| Storage/RegionGeneration | 具体资源的实例与生命周期世代 | 所有指向它的视图都已初始化 |
| ContentVersion | 指定范围内内容事实的版本 | 每次写入都新建对象身份 |
| AccessPath | 从调用参数、聚合、捕获、全局等合法来源取得对象 | 类型相等、对象永远存活 |
| Responsibility | 清理、转移及初始化责任 | 可任意复制环境 |

RuntimeId 的具体整数布局可调整，但必须区分 owner/实例，不能只按源码局部变量编号共享。
运行时循环与递归使用符号 binder 及每次调用的环境实例，不要求编译器预先枚举未来对象。
同一个代码模板可以重复运行；上次运行的环境不进入下次的查询缓存。

创建依赖时记录它来自哪个受检表达式/绑定。形成类型只取得获准 identity，不执行对象
内容读取。普通实参已有的求值槽位继续复用，不能为取得 identity 再执行一次实参。
同一个源表达式执行两次是否指向同一对象，要有来源关系；相同 AST 不是该关系的证明。

移动处理责任和位置变换，不能笼统规定“所有依赖身份都变”或“所有引用都保活”。
依赖若指向被结束的旧存储，旧路径失效；所有权转移能够合法保留的资源关系则随证据迁移。
同地址重建的新世代不会复活旧依赖。具体操作按资源契约判定，而非 TypeRef 自行猜测。

## 4. 从源码到受检 TypeRef

生产入口接受类型表达式、binder 环境和允许观察的阶段，返回类型引用与形成证据：

```text
form_type(expression, context):
    elaborate required comptime computation and its contracts
    retain opaque NF heads and checked ordinary values
    collect dependency terms with their domains and provenance
    close domain dependencies; order the hole telescope
    abstract occurrences with binder-aware mappings
    intern immutable core after structural validation
    validate substitution and semantic domains in telescope order
    return reference + formation evidence + environment requirements
```

收集范围包含主体、参数域、限定、代码/接口描述和契约；只扫描 materialize 后的类型主体
会漏掉仅在槽域中使用的依赖。域的闭包也必须有来源，不能替缺失依赖编造一个 Type。
遇到尚有真实生产者的域时沿[依赖观察](dependency-observations.md)挂起。

同一依赖的重复出现只抽象一次。不同外部对象不能为了共享率合并；局部 binder 可以一致
改名，但名字相同不构成绑定相同。先检查合法作用域，再安排内部 telescope 顺序，不能
通过拓扑排序使源语言不允许的前向引用变合法。

例如内部形状可以是：

```text
core C: holes [h0 : Type, h1 : Runtime(lift Pointer(h0))]
        body Apply(SensorDeclaration, [h0, h1])

reference A: C with [Calibration, dependency(first)]
reference B: C with [Calibration, dependency(second)]
```

检查 h1 的域时必须已经替换 h0。只检查 h1.sort=Runtime 不足以证明它具有需要的指针域。
两份引用共用 C，不说明 first=second，也不要求 Sensor 本体内新增一个指针字段。
示例为内部记号，不规定新的源码 Pointer API。

形成证据限定视图、域前提与替换版本；core 命中只省结构存储，不能跳过语义准入。
类型形成的成功可以缓存，使用点访问许可和责任状态仍须分别检查。

## 5. 替换组合、观察和相等

目标保留惰性 substitution：组合节点记录输入/输出 binder scope、映射及父关系。
只有观察某项或达到深度阈值才压缩，压缩结果仍是不可变引用；不得修改旧 core 或候选。
插入 binder、删除已消费 comptime 槽、内联重命名都使用同一受检映射接口。

没有必要每次字段观察先 materialize 整个类型。观察器可以在 core 和组合替换上解释
需要的部分，并返回来源与域证据。真正展开时按展开大小收费，不能按共享节点数量
收费后无界复制被反复引用的大值。

结构散列/相等用于 intern；完整类型语义相等仍解释 substitution 并遵循 PTC 的关系。
相同 core 不证明引用相等，不同 core 也不直接证明不等。NF 头不展开，用户 Eq/Hash
不自动成为类型身份规则。错误行/效应行等项保留各自理论，不被通用结构遍历改写语义。

发生检查针对不允许的替换环和逃逸，不能把合法名义递归全部拒绝。
按值布局循环在表示层另查；通过指针引用递归类型不需要先展开 pointee 布局。
跨候选返回的引用不能带其未提交 metavariable、局部 waiter 或过期 owner。

## 6. EnvironmentPlan 是操作交付计划

类型引用记录依赖是谁；环境要求说明当前操作需要什么；具体计划说明如何取得。
不是所有类型依赖都在每次操作中被读取，只有实际操作/清理契约要求的部分才生成访问。
然而未被本次读取的依赖仍不能从类型身份或未来责任中随意删除。

```text
EnvironmentRequirement {
  dependency, domain, required_operation,
  contract, use_boundary, origin
}

EnvironmentPlan {
  semantic_view, binder_mapping,
  entries: [dependency -> checked access path],
  evaluation_dependencies,
  resource_preconditions,
  transfer_and_cleanup_edges
}
```

允许的来源包括已交付参数、receiver 的外层环境、合法闭包捕获、显式封闭实现视角、
合法全局对象和接口已有的依赖证据。不能按同名局部或“唯一同类型对象”搜索替代品。
隐藏参数是已有依赖的实现传递方式，不授权编译器给公开函数偷偷增加新的语义要求。

计划构造按以下顺序进行：

1. 从已选操作与结果/清理契约列出依赖和所需权限。
2. 用类型匹配、显式绑定和已有证据核对 identity；来源冲突必须证明相等或拒绝。
3. 在当前输入与捕获中寻找**有证据的**取得路径，检查域及前置环境依赖。
4. 将取得动作接入 ArgumentPlan 的执行依赖，复用已经完成的实参/receiver 槽。
5. 在 normalized IR 检查访问、移动、初始化、效果与清理；交付安全计划及条件。
6. 实例化保持 binder 映射；目标降码选择寄存器、隐藏参数或共享环境存储。

多个合法路径不影响类型身份，但执行行为仍需遵守源控制顺序。不能为了省一次 load
提前跨过可能修改或终结依赖的回调；不能把需要 move 的取得改为隐式 Clone。
借用路径也不自动取得释放权限。路径取出失败/早退时，只处理实际已经取得的责任。

## 7. 五类边界怎样交付

| 边界 | 必须保持的关系 | 拒绝或额外契约的触发 |
| --- | --- | --- |
| 调用 | 实参依赖到 callee binder 的映射、输入求值顺序、环境入口条件 | 依赖固定为 A 却显式供应 B；无法取得或已移出 |
| 返回 | 结果 TypeRef、返回环境与调用方可保留来源、出口清理后的有效性 | 返回指向已结束局部对象的依赖 |
| 闭包/handler 捕获 | 字段语义次序、依赖、拥有/借用关系、调用和销毁契约 | 类型正确但环境逃逸，或重复消耗捕获 |
| 聚合/容器 | 字段与共同外层环境的关联、元素要求和实际责任 | 容器拥有缓冲区却未保活元素的外部目标 |
| 错误/可选/分支 | 实际载荷路径及 guard、各出口和合流环境 | 把某分支的环境用在另一分支，或错误路径丢失清理 |

分支 A 返回依赖 first 的值、分支 B 返回依赖 second 的值，不能把两个 substitution
随意并成“任选对象”。若已声明接口支持封闭携带者，保留分支对应的环境；否则需调整
接口或拒绝。内部 guard/合流节点不新增源语言的任意存在类型或动态依赖类型转换。

外部依赖绑定与拥有环境字段的责任不同。捕获字段按正式首次出现次序形成、逆序清理；
不能为满足依赖图而重排可观察析构。固定顺序无法满足依赖就拒绝。
Drop 的实现、替换和环境在责任建立时固定，随责任转移；销毁现场不重新挑 allocator。
悬挂/取消 continuation 的清理必须仍持有所需环境，开始 Drop 不等于完成全部责任。

## 8. 本体布局、代码共享与 ABI

区分四类输入依赖：类型身份、本体表示、操作环境、程序检查前提。
一个 dependency 可以参与多类，但不能因为它出现在类型中就全部加入 sizeof 图。
Sensor 的校准对象若只供方法读取，可由外层共享；不要求每个 Sensor 都加字段。

共享机器代码前必须核对实际表示、调用约定和已实例化行为；只比较 core ID 不足够。
代码模板可把环境当明确输入参数，调用点分别供应各自环境；这不合并两个 TypeRef。
捕获环境布局的优化不能改变语义捕获次序、责任或可观察清理。

值相关大小必须有明确的目标表示与运行时取得计划。不能拿“类型计算未完成”冒充动态大小，
也不能一律退化成未说明协议的 fat pointer。目标尚不支持时报告目标限制，不修改类型身份。
薄指针与元数据环境、表示观察状态、LayoutPlan 和 CallAbiPlan 的交付协议见
[布局与调用 ABI](layout-and-call-abi.md)。各 target 的具体编码与调用分类仍需细化，
不能将目前 LLVM 形状当作跨目标公开 ABI。

## 9. IIR 交付与变换约束

新 IIR 不继续将 NIR Function 加 key 作为长期唯一表示，但可复用稳定表达式数据结构。
类型注解、参数、返回、捕获、聚合载荷、调用适配及清理均要引用同一类型/环境协议。
独立 IIR 构造入口至少核验：

1. 每个 TypeRef 的 telescope 和替换完整，语义域及 scope 证据有效。
2. 必需 comptime 部分已完成；合法 NF 与 runtime binder 保留，不强行闭合化。
3. 所有依赖由实例/捕获/参数环境绑定，不能以 package 级集合代替局部作用域。
4. 操作与清理携带受检环境交付关系，safe 证据已经由相应 normalized IR 产生。
5. 实现选择、控制目标、错误与效应路由完整；verified 义务按模式在实例层处理。

当前 `completed_type` 对裸 Hole 的拒绝应保留在其裸 Term 入口；新引用入口检查的是
已解释的 TypeRef，不是删掉拒绝分支就接受任何 hole。CallableIdentity、TemplateStorage
继续履行原来的无环境契约；一般带环境 callable 使用适当的独立描述。

| 变换 | 必须同时映射或重证的内容 |
| --- | --- |
| 内联/展开 | 局部 binder、TypeRef substitution、环境路径、return/control 目标、责任及来源 |
| 聚合标量化 | 字段路径、环境共享关系、部分初始化和清理顺序 |
| 参数消去 | 已消费 comptime 槽到 runtime 槽的映射，结果域和契约引用 |
| handler/continuation 降码 | 捕获环境、occurrence 路由、挂起期间存活、取消清理 |
| 布局/ABI 合法化 | 路径到机器位置映射、目标表示前提、调用者/被调者一致性 |

每个变换返回新 IR 版本及保留/失效的证据集合。变换失败不交付混用新旧映射的实例。
不会在后端重新执行 PTC 搜索或从变量名补齐漏掉的环境。

## 10. 验收线与可观察性

首先用正式文档的 Calibration/Sensor 和 NodeStorage/NodeIndex 走真实源码路径：
观察 lift 的 HIR、域、共享 core、不同 substitution、调用环境、NIR safe 检查、IIR 和目标传参。
在此前置通路完成后，才用真实库 allocator/容器验证共享环境与 Drop；当前 std 尚未完整，
不能引用虚构 API 代替库交付。

成组反例包括：同 core 不同对象、重复同一依赖、仅在域中使用的依赖、嵌套 binder 重绑定、
同名遮蔽、循环重复执行、原位置移出、同地址不同世代、返回局部、分支不同环境、
Drop 依赖早于清理结束失效、未知 callback 破坏访问依据，以及错误/取消路径的部分取得。

内部 dump 同时显示 TypeRef 和使用点环境，避免只打印 materialize 后看似一样的类型：

```lisp
(typed-use
  (type-ref (core 9) (arguments (dependency (owner 4) (binder 2))))
  (operation (method 17))
  (environment (entry (dependency 4 2) (path (parameter 0) (environment-slot 1))))
  (safe-evidence (nir-version 6) (proof 12))
  (source (file 1) (start 70) (end 90)))
```

这只是结构示意；实际 Display 应稳定编号、正确转义并明确 owner/generation。
性能分别统计 core intern、域检查、替换组合/展开、环境计划、safe 分析和 ABI 物化。
core 命中率不能单独证明调用开销下降；更少的环境字段也不能单独证明清理正确。

本次没有运行这些验收，也没有改变任何源码；上述路径仍须按迁移阶段实际接通。
