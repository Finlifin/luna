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

现在，属性求值期间的 required invocation 可以向创建同一索引的 checker 请求 B。外层 VM 保留调用栈和局部值，B 在自己的声明作用域内使用剩余预算。每个供应者在这次属性求值期间保留已物化的结果，多个读取方共享按既有覆盖次序合并的冻结对象。整个包仍须通过属性消费者检查才能发布。本地取舍是同步进入源检查器，保留外层 VM，并在宿主线程栈不足时为下一层另接一段栈。仅设八层上限曾在默认测试线程上先触发栈溢出，因此现在单独控制两种资源：按剩余栈空间决定何时扩栈，同时活动的供应者仍暂限八层。当前按需读取和最终发布还会分别执行合并与冻结。自举时若采用显式 continuation，可将深度交由资源预算控制，并让最终发布复用已经验证的冻结结果。验收包括真正的环、超深的无环链，以及读取前先用 builder 生成声明，读取后检查包中同名见证只出现一次。[供应者调度](../compiler/elaboration/src/program/attributes/demands.rs) · [端到端用例](../compiler/luna/tests/attrs_and_meta_reflection.rs)

在 `a01a21fd` 还复现了一个交付缺口。B 用 `meta.StructBuilder.new().Build()` 生成类型，把该 Type 作为 `metadata` 的条目；仅编译 B 可以成功。若正在计算属性的 A 此时调用 `meta.attrs_of(B)`，A 的 VM 捕获的声明世界早于 B 生成的类型，解码这个 Object 会报告 `Type`。把 B 的条目改为普通 bool 后，同一次嵌套读取可以完成。[属性按需求值](../compiler/elaboration/src/program/attributes/demands.rs)

等待方现在从冻结值的类型和值载荷出发，沿字段类型、枚举载荷和 newtype 表示查找尚缺的生成声明。先交付依赖，再交付使用者。它们沿用供应者分配的身份。等待中的 VM 据此解码 Object，调用方的 builder 也把这些描述纳入自己的形成与有限存储检查，可以继续用收到的类型构造定义。最终包只从原生成方接收每个定义。跨种类验收用例让 B 返回枚举，其载荷为结构体，字段为 newtype；A 连读两次、反射到底层表示，并用枚举生成新的外层结构体。[声明依赖查找](../compiler/elaboration/src/program/attributes/layouts.rs) · [源码验收](../compiler/luna/tests/attrs_and_meta_reflection.rs)

当前取舍是为已有生成声明的属性求值维护临时索引，每次读取按接收方实有的布局查找缺项。接收 VM 在解码前验证描述。只有实际调用 builder 才会为它建立检查目录，按依赖顺序补验导入的声明，并检查有限存储。纯反射避免了这部分开销；同时读取和构造的程序仍需支付两个检查器的验证成本。自举时若让两者共有不可变的布局证据，需要把证据绑定到精确的声明世界，并保留失败恢复：同一 VM 读到错误载荷、冲突身份或耗尽预算后，这次导入的描述都应撤回，原有描述可继续使用。[接收与失败恢复](../compiler/comptime_interpretor/src/source_values.rs)
