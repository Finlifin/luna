# 自举编译器备忘

这份记录留给用 Flurry 重写编译器的人。每个条目从一个真实程序出发，说明当前实现受什么约束、付出了什么代价，以及换一种实现后还得通过哪个反例。后续遇到新的取舍或移除旧限制时，在相应条目补上提交和证据。

## 2026-11-21 · 泛型别名里，知道类型身份仍然不足以给字面量定型

`typealias Alias<T> = Indexed<T, 7>` 在定义处已经知道两个位置谈的是同一个 `T`。这项证据足以证明 `T == T`。给 `7` 选择整数表示却要等到调用方提供 `T`。另一方面，若另一个参数要求 `Choice<T>`，枚举的声明已经确定，写错变体名可以在别名定义处报错。把这些情况合成一个“参数已知”标志，会让其中一种检查做出错误决定。[别名契约](../docs/type/user-defined-types/newtype-and-typealias.md)

在 `3f052ce8`，elaborator 沿用既有表达式检查器，在检查别名定义期间单独保存抽象 Type 参数的声明身份。进入独立声明时移出这组身份，返回时恢复。这样可以继续用原来的符号项证明形成条件。代价是检查器多了一项要随作用域保存的状态，别名展开时还要按实际参数再次检查相关表达式。[别名检查](../compiler/elaboration/src/program/aliases.rs) · [声明作用域](../compiler/elaboration/src/program/declaration_scope.rs)

自举时可以尝试让实参槽同时携带“可用于证明的身份”和“完成值表示还缺的输入”。检验时让一个违反形成条件的别名始终没有调用点，确认定义检查仍会拒绝它。再对一个合法别名交替传入不同整数宽度，检查一次溢出后，换一个有效类型仍能成功。[形成条件与调用隔离用例](../compiler/luna/tests/nominal_value_parameters.rs) · [具名整表与逐项变参用例](../compiler/luna/tests/normal_input_plans.rs)

## 2026-11-21 · 两条 VM 执行路径保留同一份字面量意图

考虑 `Key<ty, if choose() { .ready } else { .other }>`。当 `ty` 由 comptime 调用给出时，`.ready` 可能交付为 Symbol，也可能选中某个枚举的无载荷变体。另一分支的名字同样需要检查，实际调用只求值一次 `choose()`。把源码表达式一直留到 VM 里重新 elaboration，会把词法环境、控制流和重复求值的问题一并带过去。[实参执行次序用例](../compiler/luna/tests/normal_input_plans.rs)

当前实现让 NIR 的 `NormalFormArgument` 保存字面量拼写、源码位置和必要的控制流。字节码编译器再从中生成检查实参所需的结构描述。树解释器与字节码 VM 各自读取一种表示，在同一个入口按具体参数域完成检查。`2a5745b0` 增加裸符号时，两种表示、遍历器、复制预算和 dump 都要同步修改。共享的变体查找保留了普通表达式的命名规则和诊断。[NIR 实参](../compiler/nir/src/program.rs) · [字节码输入描述](../compiler/comptime_compiler/src/normal_types.rs) · [两条路径共用的域检查](../compiler/comptime_interpretor/src/input_domains.rs)

自举时值得尝试把待定字面量的种类、源码位置和大小计费集中在同一个不可变描述中。验收用两个变体顺序相反的枚举、一个 Symbol 域和一个带错误行及两层可选的枚举域。在同一个 VM 里重复提交同一拼写，必须得到各次调用自己的变体身份和包装层次。即使条件恒真，未选分支里的错误名字也要指向它自己的源码位置。[双执行路径与诊断用例](../compiler/luna/tests/nominal_value_parameters.rs)

加入浮点实参后，NIR 和字节码描述共用 `ContextualLiteral`，复制拼写的计费依据也集中到了这个类型上。树解释器与字节码 VM 仍各自保存源码跨度、分支关系和执行位置。当前取舍是让 elaborator 先按预算检查十进制拼写，等具体域出现后再解析一次并形成值。这样编译结果只需保存拼写和符号。自举时若改为保存解析好的十进制系数和指数，要一起衡量复制成本和反复解析成本。验收时把略高于 `f32` 舍入中点的 `1.000000059604644775390625000000001` 交给依赖域，结果位模式必须是 `0x3f800001`。先经 `f64` 舍入会丢掉决定结果的末位。另用同一 VM 检查未选分支中的溢出，再换到更宽的域重试。[浮点实参与失败恢复用例](../compiler/luna/tests/exact_floats.rs) · [逐项收集与具名整表用例](../compiler/luna/tests/normal_input_plans.rs)

## 2026-11-21 · 复用实参字节码也有成本

解释器可以复用同一段实参的编译结果。当前缓存保存受检实参描述、可选目标类型和代码，每次调用仍从自己的环境取值。为了让键比较和克隆也服从工作预算，查找前要遍历描述来计费，候选项还可能需要结构比较。当前采用有限表，满表或插入将超过总权重上限时整表清空。它使内存占用和失效逻辑容易审计，也可能同时丢掉仍然热门的代码。这些阈值是当前实现参数。[缓存与失败恢复用例](../compiler/comptime_interpretor/src/contextual_code.rs)

自举时若已有能表达源码归属与生命周期的稳定 IR 身份，可以实测用它引用实参代码的收益。比较时分别测冷编译、热查找及交错域调用。还要让同一段代码在不同调用环境中读出不同值，并验证预算耗尽后给足预算能够重试。命中编译结果和命中 invocation 的返回值是两件独立的事。[实参代码缓存](../compiler/comptime_interpretor/src/contextual_code.rs) · [调用开销用例](../compiler/luna/tests/contextual_input_costs.rs)

## 2026-11-21 · 属性快照里，已知没有属性和属性尚未就绪是两种信息

在 `Decorated` 的属性构造器内读取 `Plain` 的属性时，`Plain` 可以在文件后面声明。只要它没有附加属性，读到的就应当是空 Object。早期实现用全包的“正在求值属性”标志处理循环，这个例子也被拒绝。现在 elaborator 根据 HIR 的定义目标预先登记有供应者的声明身份。发布前，读取登记过的声明仍会报告未就绪，其他已确定身份的声明可以返回空对象。所有供应者求值、合并和检查成功后，一次发布整张冻结表。[源码回归](../compiler/luna/tests/attrs_and_meta_reflection.rs)

required invocation 的声明快照共享其创建时的不可变索引或冻结表，复制快照只保留所有者；读取才按声明查找并解码该属性 Object。发布不会改变早先捕获的快照。在 `8a72d984`，如果 A 读取带属性的 B，即使 B 不依赖 A，这一批的属性也会因 B 尚未发布而被拒绝。[快照所有权与预算](../compiler/nir/src/copy_budget/package/tests.rs)

现在，属性求值期间的 required invocation 可以向创建同一索引的 checker 请求 B。外层 VM 保留调用栈和局部值，B 在自己的声明作用域内使用剩余预算。每个供应者在这次属性求值期间保留已物化的结果，多个读取方共享按既有覆盖次序合并的冻结对象。整个包仍须通过属性消费者检查才能发布。本地取舍是同步进入源检查器，保留外层 VM，并在宿主线程栈不足时为下一层另接一段栈。仅设八层上限曾在默认测试线程上先触发栈溢出，因此现在单独控制两种资源：按剩余栈空间决定何时扩栈，同时活动的供应者仍暂限八层。最初按需读取和最终发布分别执行合并与冻结。自举时若采用显式 continuation，可将供应者深度交由资源预算控制。验收包括真正的环、超深的无环链，以及读取前先用 builder 生成声明，读取后检查包中同名见证只出现一次。[供应者调度](../compiler/elaboration/src/program/attributes/demands.rs) · [端到端用例](../compiler/luna/tests/attrs_and_meta_reflection.rs)

在 `a01a21fd` 还复现了一个交付缺口。B 用 `meta.StructBuilder.new().Build()` 生成类型，把该 Type 作为 `metadata` 的条目；仅编译 B 可以成功。若正在计算属性的 A 此时调用 `meta.attrs_of(B)`，A 的 VM 捕获的声明世界早于 B 生成的类型，解码这个 Object 会报告 `Type`。把 B 的条目改为普通 bool 后，同一次嵌套读取可以完成。[属性按需求值](../compiler/elaboration/src/program/attributes/demands.rs)

等待方现在从冻结值的类型和值载荷出发，沿字段类型、枚举载荷和 newtype 表示查找尚缺的生成声明。先交付依赖，再交付使用者。它们沿用供应者分配的身份。等待中的 VM 据此解码 Object，调用方的 builder 也把这些描述纳入自己的形成与有限存储检查，可以继续用收到的类型构造定义。最终包只从原生成方接收每个定义。跨种类验收用例让 B 返回枚举，其载荷为结构体，字段为 newtype；A 连读两次、反射到底层表示，并用枚举生成新的外层结构体。[声明依赖查找](../compiler/elaboration/src/program/attributes/layouts.rs) · [源码验收](../compiler/luna/tests/attrs_and_meta_reflection.rs)

当前取舍是为已有生成声明的属性求值维护临时索引，每次读取按接收方实有的布局查找缺项。接收 VM 在解码前验证描述。只有实际调用 builder 才会为它建立检查目录，按依赖顺序补验导入的声明，并检查有限存储。纯反射避免了这部分开销；同时读取和构造的程序仍需支付两个检查器的验证成本。自举时若让两者共有不可变的布局证据，需要把证据绑定到精确的声明世界，并保留失败恢复：同一 VM 读到错误载荷、冲突身份或耗尽预算后，这次导入的描述都应撤回，原有描述可继续使用。[接收与失败恢复](../compiler/comptime_interpretor/src/source_values.rs)

最终发布现在沿用同一轮源检查中按需读取所冻结的 Object。每个非空定义属性对象持有一个独立的共享句柄，提前读取与最终 NIR 可以持有同一份载荷；已知空的结果也可直接交付。目标合法性、键和值域仍由最终消费者逐个检查。例如提前读取函数上的布局属性可以得到 Object，但发布时仍须报错，因为只有结构体能采用该布局。全包合并仍会遍历原始属性，保留各项的词法来源和覆盖次序以供诊断。若要进一步复用这部分工作，自举实现需能同时保留选中的原始属性位置和冻结值。[源检查与发布](../compiler/elaboration/src/program/attributes.rs) · [真实包回归](../compiler/luna/tests/attrs_and_meta_reflection.rs)

用相同的源 HIR 测量 required invocation 预算：前面的结构体读取后面结构体的全部属性，后者的 metadata 只有一个字符串。字符串取 64、2048 和 4096 个 ASCII 字节时，修改前分别消耗 1983、21823、42303 步，修改后为 1899、19755、38187 步。原先第二次冻结的开销随载荷增长；交付已有句柄只需查询本轮定义表。

## 2026-11-21 · 重复调用同一函数时，签名遍历的成本可以复用

VM 在绑定实参前会遍历签名，按节点与载荷扣除预算，并检查结构深度。同一具名函数可以连续接收不同的实参，而源码签名在这个 VM 的生命周期内不可变。当前只保留最近一次成功遍历的签名引用与步数。下一次遇到同一棵签名树且预算充足时直接扣费，随后照常观察关联类型、检查实参和契约、执行函数体。预算不足时仍需遍历：访问记录字段名或为子节点预留步数可能提前失败，调用者此时还有剩余预算。

这个单槽在相邻调用间占用固定空间，交替调用不同具名函数会替换它。特化实例、闭包和 IIR 实例沿用逐次遍历。以现有 invocation 基准交替运行优化前后的 release 二进制，每组取七次中位数。64 个普通参数、只返回首参时，每次调用由 5.795 µs 降到 5.290 µs，堆分配从 11 次降到 6 次，均消耗 398 步。加入 256 次局部读取后，耗时由 11.670 µs 降到 11.142 µs，均消耗 973 步。自举时若将这一成本存入已准备的调用接口，仍要使低预算调用与首次遍历交付同样的失败位置和剩余步数。[调用入口与预算回归](../compiler/comptime_interpretor/src/invocation.rs) · [基准程序](../compiler/comptime_interpretor/benches/invocation.rs)

## 2026-11-21 · 求关联类型时保留已经遍历的前缀

设记录的第一个字段名很长，后面的字段类型依次投影关联项 A、B、A。A 和 B 各自有 comptime 初始化器，B 的函数体又显式读取 A。源 VM 原先在遇到尚未求值的投影时返回缺项，从类型根部重试。它已经能将外层三次投影合并成 A、B 两次求值，但仍把记录前缀遍历三遍。在基线 `3a333c3d`，仅把字段名从 1 字节延长到 4097 字节，就会多扣 12288 步。

现在结构遍历走到投影处，先结算已消耗的步数，再把剩余预算交给关联项初始化器。成功时从当前位置继续；失败时保留初始化器报告的源码位置及剩余预算。两个执行器中，同样的字段名变化都只多扣 4096 步。外层末次投影使用本次查询中 A 的结果，B 的函数体仍独立执行它对 A 的读取。运行时解释和具体 IIR 的类型观察仍只读所在快照已交付的选择。[投影与预算交接](../compiler/comptime_interpretor/src/selected_types/projection.rs) · [求值顺序与失败恢复](../compiler/comptime_interpretor/src/selected_types/observation_tests.rs)

当前取舍是让结构遍历的宿主栈帧在初始化器执行期间保持活动；剩余栈空间较小时在同一线程扩栈。类型结构沿用 64 层检查，字节码初始化器链由 VM 驱动。自举时若把结构遍历也迁到显式续延，续延需要携带已构造的字段、当前查询的关联项观察和交回的预算。若把观察放到 VM 全局，嵌套函数对 A 的读取就可能少执行一次。

## 2026-11-21 · 关联 Type 初始化器的定义检查和实例求值

设 `Selected(enabled)` 根据一个 bool 参数返回 bool 或 char。把它作为 `trait Source where enabled: bool` 中 `assoc Item: Type` 的默认值时，声明处已经知道结果属于 Type，分支选择仍依赖具体的 enabled。在基线 `deabc828`，仅检查这个声明就会尝试解释抽象的 enabled，报 `AbstractValueUnresolved`。现在泛型实现可保存检查并替换过的初始化程序；读取 `Holder<true>.as(Source<true>).Item` 作为局部注解或类型别名时，由该处的 required comptime 求值取得具体结果。若分支返回 `Wrapped<bool>`，交付的身份仍含 Wrapped 的 normal-form 头。[定义与选择](../compiler/elaboration/src/program/interfaces/associated.rs) · [经源码验证的实例](../compiler/luna/tests/deferred_associated_values.rs)

当前的取舍是继续向只读的签名查询提供已完成的闭合 Type。以同一个 Source 为例，若泛型函数的参数为 `T.as(Source<flag>).Item`，调用处提供 `true`、`Holder<true>` 和实参 `true`，签名特化仍会在选中初始化程序时报告 `selected associated value is not a completed Type template`。自举时，选中程序的查询需要把完整接口实例、接收者和成员身份交还给 elaboration；由请求方承担求值的预算、失败与生成定义的发布，再用结果继续检查该次调用。直接在 PTC 内运行初始化程序会使只读类型查询承担这些职责。[当前签名查询](../compiler/elaboration/src/program/interfaces/values.rs) · [只读投影](../compiler/type_resolution/src/associated/values.rs)

## 2026-11-21 · 泛型调用中由谁执行关联类型程序

`keep<false, Holder<false>>('n')` 的参数和返回值可以同为 `T.as(Source<flag>).Item`。在上一个阶段，类型别名能读出 char，同一投影一旦出现在函数签名却会在检查实参前失败。这次将单步选择的结果区分为已完成的类型和持有实现证据的初始化程序。遍历一份签名时，elaboration 遇到输入已齐的程序，借当前源码位置发起 required comptime invocation，再从该投影继续遍历。参数和返回值重复读取同一投影时共享这次遍历的结果。若接收者为 `Holder<flag>`，即使它已经匹配到泛型 impl，初始化器也仍然要等 flag 绑定。显式输入和普通 comptime 参数两种调用路径均有源码用例。[调用端](../compiler/elaboration/src/program/interfaces/call_types.rs) · [闭包捕获与依赖参数用例](../compiler/luna/tests/deferred_associated_values.rs)

源码 VM 还需要能执行这个程序。收集 dispatch 时将闭合投影及其所选程序保留为依赖，闭包构造检查具体捕获值时才向 VM 索取结果。例如同一个泛型闭包分别可以捕获 bool 和 char。闭包的保留值检查与具体 IIR 查询继续只读已交付的证据；缺失证据时不能把尚未执行的程序视为类型等式。[依赖收集](../compiler/materialization/src/lib.rs) · [VM 的捕获检查](../compiler/comptime_interpretor/src/selected_types/observation_tests.rs)

自举时有两笔开销值得处理。请求方交付具体的接口实例、接收者与成员，required VM 会在自己的声明快照里重新验证选择；当前节省了复制整个声明世界，代价是两次选择。逻辑查询使用类型查询预算，程序运行使用调用者的 required comptime 预算。运行时泛型函数的源码检查现在也可以完成，但具体 IIR 冻结仍缺少交付已观察类型的通道：把上例中的 `keep` 和入口改成 runtime 函数，实例化时仍报告 `selected associated value is not a completed Type template`。后续交付须保留完整选择依据、生成定义的身份以及求值失败的来源，避免在只读 IIR 查询中再执行用户代码。

## 2026-11-21 · 向具体 IIR 交付已求出的关联类型

若 `get` 的参数为 `Boxed<T.as(Source<flag>).Item>`，函数体可以只读字段。调用处对 flag=true 与 flag=false 分别得到 bool 与 char。此前在源码检查后丢弃了这次选择，IIR 再看同一函数签名时只看得到泛型初始化程序。现在每次成功的源码类型观察同时保留完整投影、实现声明、实现实参、初始化程序身份、结果类型及源码位置。IIR 在当前声明世界中重新选择，再按这几项取回类型。检查同一源码调用的不同部分时，仍只有一次初始化执行。用 `meta.StructBuilder` 作为类型程序的用例验证了 IIR 使用源码已经发布的那个结构体声明。[源码交付](../compiler/elaboration/src/program/interfaces/call_types.rs) · [生成类型的端到端用例](../compiler/luna/tests/required_definition_publication.rs)

冻结表随同一次编译的 NIR 与不可变声明快照共享，弱引用标记具体的已检查程序。独立构造的另一份初始化器即使正文相同也无法消费旧记录。源码表达式读取成员仍遵守原有的执行规则；如果两次观察为同一选择交付不同类型，仅在请求这个具体身份时报告冲突并附两个位置。自举时若要跨进程或跨修订重用记录，需要为初始化器和其调用的声明提供可校验的版本身份。进程内的程序身份只适用于本次冻结的声明世界。[记录和冲突检查](../compiler/nir/src/program/observed_types.rs) · [匹配原声明快照](../compiler/type_resolution/src/associated.rs)

本阶段还实测了没有源码观察可供交付的情形：泛型 `relay<flag>` 仅在函数体中取得 `leaf<flag>`，由入口调用 `relay<false>()`。如果 leaf 的签名引用上述 Item，投影要到 relay 实例化才首次闭合。已有交付可以支持这条路径；首次闭合时的缺项需要由实例化的编译阶段向源码 comptime 世界发起请求。

## 2026-11-21 · 在具体特化时第一次需要关联类型

`relay<flag>` 的签名可以完全不含关联类型，函数体里却有 `let callback = leaf<flag>`，而 `leaf` 返回 `?Holder<flag>.as(Source<flag>).Item`。源码检查 relay 时 flag 仍然开放；冻结入口 relay<false> 时才有足够信息求 Item。这个请求由具体 IIR 的特化器发起。它先在声明环境中选择实现、核对实参和初始化程序身份；若已有交付则沿用，否则把闭合的成员读取交给同一源码快照的 comptime VM。合约失败保留程序位置与拒绝原因。PTC 只负责选择所需的类型命题。[特化入口](../compiler/materialization/src/staged.rs) · [间接调用与失败恢复用例](../compiler/luna/tests/deferred_associated_values.rs)

特化器复用声明快照与已准备的代码，每次执行仍有独立的 VM 状态，并从请求方扣除实际消耗的步数。新观察暂存在本次类型遍历中，整个类型及其结果成功保留后再供同一声明环境的后续查询读取。如果元组的第二个投影失败，重试时必须重新获得第一个投影的结果。这个事务只覆盖类型观察和视图缓存。

目前，源码阶段已发布的生成类型可按原身份进入 IIR；若第一次在上述 leaf 特化时才运行调用 `meta.StructBuilder.Build` 的初始化程序，真实 sysroot 用例仍停在 VM 的 `interpretation Type`。冻结快照目前没有接入新定义发布。自举时需要将新声明及其布局连同类型身份交付给消费该类型的声明环境，并为失败的发布和旧快照分别维持一致性。仅交付一个新声明编号不足以恢复其字段与布局。
