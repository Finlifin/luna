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
