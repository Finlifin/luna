# 024 — comptime — 03

- 日期：2026-08-23
- global_index：024
- theme：comptime
- subindex：03
- 本轮目标：消化作者对 `023` 的答复，收敛 normalization 强度、defer、纯性信任边界、comptime interpretation/execution 与 splice 基础

## 作者本轮输入摘要

作者本轮明确表示：正式文档仍然过早，继续进行 proposal 级讨论。主要答复如下：

1. normalization 强度必须在 HIR 阶段可判断；术语统一为 `ordinary` 与 `loose`。
2. `comptime fn` 定义和任意 `comptime {}` 块属于 `loose`；含 `defer` 类型的变量在 `ordinary` 中视为 `Any`，通常要用 inline 控制流约束；refinement gradual type 在 ordinary 中通过命题提取工作，约束在 instantiation 检查；含 defer 类型参数的调用在 ordinary 中需要就地执行并替换；CMTT 暂后；其余代码为 ordinary。
3. loose 中的开放定义到整体 instantiation 时检查全局冲突；开放分散定义若带子句参数（例如 `impl Tuple<T, T> where T`）不能在 loose 中定义。
4. loose 环境中生成的 runtime fn 仍是 ordinary；对外部、包裹它的 comptime fn 的引用标成 defer；runtime fn 内仍可有 comptime block。
5. static scope 中已有 implementation 先完成 coherence；loose 生成的 impl 在 loose 环境整体 instantiation 时与其一起检查。
6. `pure comptime fn` 的 `pure` 是开发者纪律下的可信公理，编译器不验证实现是否真的纯。runtime 另有多种 memory-effect 强度；memory effect 不等同 algebraic effect。
7. PTC 与程序验证不允许用户公理；需要逃逸检查使用带约束的 `assumes`。二者共享语法基础设施但相对独立。
8. 调用点环境信息不用可选子句参数承载，改由独立 implicit 参数自动传入。可选子句参数的默认值在 normalization 中直接执行；只有被 `defer` 包裹时才延迟。
9. query 缓存需要 comptime `Eq` 与 `Hash`，二者有特殊处理。
10. 两种 quote 共用关键字，由上下文/结果类型区分；2LTT quote 的内存副作用留到内存管理主题；CMTT quote/splice 留到 CMTT 主题。
11. 引入 `Splice` trait；不做一般 auto trait，只对 struct/enum/union/closure 等有限类型在条件满足时自动派生，其他情况引导用户手写精确语义。自动派生若遇共享引用、循环结构、未初始化字段或 Drop 字段则静态拒绝。
12. interpretation 与 execution 语义等价；所有代码都有 normalization 与 instantiation 两个大过程；comptime 资源预算可不同（interpretation 与 execution 分开）。两者不要求纯、终止或全，comptime 可用于编写诊断。
13. inline 条件若不能提取命题，则按未知 bool 处理，仍检查两分支且不增加路径事实；propositional type constraints 细节留到类型主题。
14. 依赖结果类型的几种形式都允许，留到类型主题；实例化失败归调用点。
15. trait 的 comptime/runtime 两相可分别实现；comptime expression 只调用 comptime 相，runtime expression 只调用 runtime 相；comptime 结果可以按普通值 splice 进入 runtime；两相共享候选集和 coherence。
16. 静态作用域是 `mod` / `struct` / `enum` 等无子句参数定义下的作用域；模块主题再展开。
17. runtime 上下文中的表达式若没有 runtime 义务，优先按 comptime 表达式解释；例如 `1 + 23` 在 `println("{}", 1 + 23)` 中被 interpretation 为 `24`。

## 可行性评估

### 已经明显收敛的部分

这轮把此前“强度由上下文模糊推断”的方向改成了 HIR 可判定的分层契约，这是对编译器可实现性和人类诊断都重要的收敛。尤其是以下边界现在可以作为后续讨论的基线：

- `ordinary` 是默认强度；`comptime fn` 与 `comptime {}` 建立 `loose` 区域。
- loose 区域可以包含 ordinary 的 runtime 定义；嵌套不等于整个父定义降级。
- `defer` 不只是“稍后求值”，还承担类型占位、调用点类型替换和可选子句默认值延迟三个方向。
- 开放分散定义的 coherence 检查不是被放弃，而是从静态作用域阶段推迟到 loose 整体实例化阶段。
- `pure` 是可信承诺，不是用户可写的逻辑公理；PTC/验证若需逃逸使用 `assumes`。
- interpretation 与 execution 的差异属于执行载体和资源预算，而不是用户语义的两套语言。
- comptime/runtime 数据流严格单向；quote 对运行时内容的“操作”发生在 AST/IR 或 reification 层，而不是读取运行时内存。

### 必须继续澄清的风险

1. `loose` 和 `ordinary` 目前既像 normalization 强度，又像局部模态环境。若不定义它们的关系，用户无法判断一个表达式的约束是“当前允许但实例化才检查”，还是“当前根本没有类型信息”。
2. `defer` 的三个用途可能是一个统一的阶段延迟算子，也可能是三个不同的表面构造。若只共享名称而不共享 obligation 模型，LSP、缓存和错误归属会不一致。
3. `pure comptime fn` 被信任为纯函数，同时 comptime interpretation/execution 又允许不要求纯、终止、全。若不限定哪些代码可以进入 query 缓存，缓存会把不纯行为错误地当作可复用结果。
4. “runtime 义务”是当前阶段推断的核心判据，但尚未定义：runtime 参数、memory effect、runtime method、动态分支、不可解释函数调用是否都属于同一类义务。
5. 两种 quote 共用关键字会简化表面语法，但仅靠结果类型区分可能造成递归类型推导和诊断循环；至少需要一个可观察的 disambiguation 规则。
6. `Splice` 的递归派生规则与布局、资源所有权、循环图、共享引用相互耦合；“自动派生很有限”还不是可执行的判定规则。
7. loose 生成 impl 与静态作用域 impl 的 coherence 合并，需要明确模块闭包、增量编译和实例化顺序，否则“整体实例化时检查”仍可能依赖编译顺序。

## 已确认 / 仍待定

### 本轮已确认

- 统一使用 `comptime` / `runtime` 术语；本轮不再使用 U₀/U₁ 作为面向读者的主术语。
- `ordinary` / `loose` 是至少两种 normalization 强度，且强度可在 HIR 阶段判断。
- `comptime fn` 定义和任意 `comptime` block 是 loose；其他代码默认 ordinary；嵌套 runtime fn 保持 ordinary。
- 含 defer 类型变量在 ordinary 中以 `Any` 参与，须通过 inline/命题提取取得可用约束；约束在 instantiation 检查。
- 含 defer 类型参数的调用在 ordinary 中要就地执行并替换。
- loose 的开放定义可延迟到整体实例化时做全局 coherence；带子句参数的开放分散定义在 loose 中禁止。
- static scope 中已有 impl 先完成 coherence；loose 生成 impl 在整体实例化时与其合并检查。
- `pure comptime fn` 的纯性是开发者承诺，编译器不验证；用户逻辑公理不开放，验证逃逸使用 `assumes`。
- 调用点环境信息使用独立 implicit 参数，不通过可选子句参数默认值承载。
- 可选子句参数默认值默认在 normalization 直接执行，被 `defer` 包裹时延迟；这是 defer 的第三个用途。
- query 缓存需要 comptime `Eq` 和 `Hash`，其实现有特殊处理。
- 两种 quote 共用关键字；2LTT quote 与 CMTT quote/splice 的细节分别留给后续主题。
- `Splice` 是显式 trait；自动派生只覆盖有限类型/条件，复杂类型引导用户手写实现；本轮选择对共享引用、循环结构、未初始化字段、Drop 字段等自动派生风险静态拒绝。
- interpretation 与 execution 语义等价，但可使用不同资源预算；comptime 不以纯、终止、全为普遍前置条件。
- inline 条件无法提取命题时按未知 bool，仍检查两分支且不增加路径事实。
- 依赖结果类型的多种表达力保留到类型主题；实例化失败归调用点。
- trait comptime/runtime 两相可分别实现，按当前表达式阶段选择对应方法，两相共享 coherence。
- runtime 上下文中无 runtime 义务的子表达式优先进行 comptime interpretation。

### 仍待定

- ordinary/loose 是否形成偏序、强弱关系，及其跨边界的 obligation 传递规则。
- HIR 阶段的强度判定输入、稳定性和用户可观察契约。
- loose 中禁止带子句参数的开放分散定义的完整语法/语义判定。
- `defer` 三种用途的统一语义与语法边界。
- `Any`、refinement gradual type 与命题提取的精确操作集和错误时机。
- pure 承诺违反后的缓存、诊断和编译器行为；不纯 comptime 的可用副作用集合。
- 可选子句默认值 normalization 的确切时点及其与 implicit 参数、调用点信息的关系。
- comptime `Eq`/`Hash` 的可信基础和 query 缓存适用范围。
- runtime 义务的判定及部分求值顺序。
- 2LTT quote 与 CMTT quote 的结果类型区分规则和共享关键字的消歧。
- `Splice` trait 的 `RuntimeTarget`、布局/资源规则和自动派生边界。
- interpretation/execution 的资源预算、失败分类和可观测副作用。
- PTC 与验证系统共享语法但独立的边界；`assumes` 的语义留待相应主题。

## 需要你拍板的问题

### 一、ordinary / loose 的判定与组合

1. ordinary 与 loose 的关系是什么？
   - A. loose 是“允许更多延迟义务”的较弱检查强度，ordinary 是较强强度；存在从 ordinary 进入 loose 的显式边界，但 loose 不能把未决事实直接提升回 ordinary。
   - B. 二者不是强弱关系，而是两种正交模式：ordinary 面向开放定义，loose 面向可执行的 comptime 生成；跨模式只通过 defer/结果契约。
   - C. loose 是 ordinary 的局部子环境，普通表达式仍按 ordinary 检查，只有 loose 产生的结果/定义携带延迟义务。
   - Other: …

2. HIR 阶段“可判断强度”具体依据什么？
   - A. 仅依据语法构造和显式标记（`comptime fn`、`comptime {}`、`defer`、refinement type）。确定、可增量缓存，但无法利用后续类型推导。
   - B. 依据已完成的签名 normalization 与局部类型信息；HIR 必须先形成稳定的阶段注解。
   - C. 依据完整约束求解结果；表达力高，但与“HIR 阶段可判断”冲突且容易产生循环。
   - Other: …

3. `comptime block` 嵌在 ordinary runtime fn 中时，哪一部分变为 loose？
   - A. 只有 block 内的 comptime 表达式和其直接依赖；runtime 外层继续 ordinary。
   - B. block 所在的整个函数体局部作用域变为 loose，便于共享构造结果，但可能扩大不可检查性。
   - C. block 仍是 ordinary，只是允许一个特殊 evaluator 调用；`loose` 只适用于独立 comptime fn。
   - Other: …

4. loose 中生成的 ordinary runtime fn 对外部引用标为 `defer`，该 defer 携带什么？
   - A. 只携带被引用定义的身份和参数依赖，调用点重新做 ordinary 检查。
   - B. 携带整个 loose 环境的延迟约束与 coherence 义务，直到父 comptime fn 实例化一起解决。
   - C. 只要 runtime fn 本身 normalization 成功，就不携带父环境信息；外部引用均视为独立定义。
   - Other: …

5. loose 中“开放分散定义不得带子句参数”的判定范围？
   - A. 所有 `impl`/`extend`/`derive`/template 生成定义的声明和目标类型都不得含未实例化子句参数。
   - B. 只禁止 implementation 的 `where` 子句参数；目标类型可以依赖已知 comptime 局部值。
   - C. 只禁止会改变候选集的子句参数；不参与 coherence 的附加参数可以保留。
   - Other: …

6. loose 生成的 impl 与静态 scope impl 的合并检查是否必须与编译顺序无关？
   - A. 必须；检查输入是模块/包静态闭包 + loose 实例化结果的规范集合，增量和全量结果一致。
   - B. 只要求同一实例化图内稳定，跨模块的开放集合由模块主题另行规定。
   - C. 允许编译顺序影响候选集，工具链以诊断提示用户固定顺序。
   - Other: …

### 二、defer、Any 与渐进依赖类型

7. 三种 defer 用途是否是同一个语义构造？
   - A. 是：defer 统一产生一个带源范围、依赖和目标阶段的 deferred obligation；变量类型、调用参数、可选默认值只是不同产生位置。
   - B. 只统一实现，不统一用户语义；每种位置有独立静态规则和错误分类。
   - C. 拆成不同关键字，避免类型延迟和默认值延迟被用户混淆。
   - Other: …

8. `defer T` 在 ordinary 中被视为 `Any` 时，允许哪些操作？
   - A. 只能进行不依赖具体结构的操作；投影、view、方法调用必须先经 inline 命题约束。
   - B. 所有操作都可暂时接受，形成延迟义务，实例化失败再报错。
   - C. 只有类型反射和等价判断可用，普通值操作一律静态拒绝。
   - Other: …

9. refinement gradual type 的约束提取失败时，是否与 inline 条件完全同构？
   - A. 是：能提取就增加路径命题，不能提取就保持未知并检查两侧。
   - B. 类型 refinement 比 inline 条件更强，提取失败即 ordinary 错误。
   - C. refinement 只在 instantiation 求值，ordinary 不做任何提取。
   - Other: …

10. `defer` 调用在 ordinary 中要求“就地执行并替换”，如果调用依赖当前 runtime 参数怎么办？
   - A. 只允许执行不依赖 runtime 值的 comptime 部分；依赖 runtime 值的部分保留为 deferred type/constraint。
   - B. 允许 quote/reify runtime 参数后执行，但结果只能是 CMTT code value，不能伪装成普通 Type。
   - C. 这种调用在 ordinary 中非法，必须移入 loose 或显式 CMTT。
   - Other: …

11. deferred obligation 在实例化时失败的诊断位置？
   - A. 优先指向触发 defer 的调用点，并附带原始定义范围。
   - B. 归属于定义点，因为定义者选择了延迟类型。
   - C. 按失败原因区分：类型构造错误指向定义，具体参数不满足指向调用。
   - Other: …

12. 可选子句参数默认值“normalization 时直接执行”的时点？
   - A. 定义 normalization：默认表达式必须不依赖调用点未知值；每个实例化复用该结果。
   - B. 调用 normalization：在调用发生时直接执行，但仍不是 instantiation；允许依赖当前调用可见的 comptime 参数。
   - C. 只有包/模块 normalization 执行一次，调用端若需不同值必须显式传入。
   - Other: …

13. 默认值被 `defer` 包裹后，是否允许它读取 implicit 参数？
   - A. 允许；implicit 参数在调用点由编译器注入，defer 在该点形成待解义务。
   - B. 不允许；implicit 只用于 ordinary/runtime 的元数据，不能决定 comptime 类型或默认值。
   - C. 只有标记为 compiler capability 的 implicit 参数可以参与。
   - Other: …

### 三、纯性、公理、资源和缓存

14. `pure comptime fn` 的“可信公理”违反时，语言承诺是什么？
   - A. 属于开发者违约，编译器可产生错误缓存/错误特化；语言不保证结果。
   - B. 仍必须保证编译器不崩溃和类型安全；不纯只会使缓存失效或强制重新执行。
   - C. 违反纯性是编译错误，但检查可由 lint/外部验证器完成，不属于 normalization 必需步骤。
   - Other: …

15. 不要求纯、终止、全的 comptime 代码允许哪些副作用？
   - A. 允许编译诊断、日志和受限编译器状态写入；文件/网络/时间等外部副作用需显式 capability。
   - B. 任意宿主副作用都允许，构建系统负责隔离和重现。
   - C. 只允许产生错误/警告，不允许成功路径副作用。
   - Other: …

16. interpretation 与 execution 的资源耗尽如何保持可观察语义一致？
   - A. 两者都产生同一类确定编译错误，预算不同只影响何时失败。
   - B. interpretation/execution 可有不同失败码和诊断，但不得生成不同的 runtime 语义。
   - C. 资源耗尽属于实现定义行为，语言不承诺诊断一致。
   - Other: …

17. query 缓存适用于哪些 comptime 结果？
   - A. 只缓存显式 `pure comptime fn` 且输入包含稳定 Eq/Hash 的调用。
   - B. 普通 interpretation 也可缓存，只要依赖图没有发现不纯操作。
   - C. loose/不终止 comptime 不缓存；其余按 evaluator 能力决定。
   - Other: …

18. comptime `Eq`/`Hash` 的特殊处理是什么？
   - A. 编译器为可缓存值提供内建结构化 Eq/Hash，用户实现只补充无法结构化的类型。
   - B. Eq/Hash 仍是普通 trait 的 comptime 相，但 evaluator 对递归/查询循环提供内建处理。
   - C. 缓存键不调用用户 Eq/Hash，而由 IR canonical identity 决定；trait 只用于用户层比较。
   - Other: …

### 四、comptime interpretation 的部分求值

19. “runtime 上下文优先视为 comptime 表达式”的判定顺序？
   - A. 先做局部依赖分析：无 runtime 参数、memory effect 或 runtime 方法调用的子树做 interpretation；其余保持 runtime。
   - B. 先按函数/运算的声明阶段选择，再尝试常量折叠；不会改变原始阶段。
   - C. 先构造完整混合 IR，再由后续 pass 选择可执行子树。
   - Other: …

20. 混合表达式如何处理？例如 comptime 子表达式结果作为 runtime 函数参数或 runtime 结构字段。
   - A. 允许局部 partial evaluation；每个跨界结果必须满足 splice/表示规则。
   - B. 只有完整表达式无 runtime 义务时才 interpretation，混合表达式不局部求值。
   - C. 混合表达式统一转为显式 `comptime {}`/splice，禁止隐式局部求值。
   - Other: …

21. 方法/函数阶段选择与“优先 comptime”谁先发生？
   - A. 先按当前表达式阶段筛选 comptime/runtime 候选，再做普通重载与 trait resolution。
   - B. 先完成候选集解析，再根据选中的定义阶段决定 interpretation 或 runtime 调用。
   - C. 两相同名方法必须在签名上互斥，否则直接报错，不依赖上下文消歧。
   - Other: …

22. `println("{}", 1 + 23)` 中 `1 + 23` 被算为 `24` 后，是否保证与手写 `24` 完全相同？
   - A. 是；包括类型、溢出、诊断和 splice 表示都相同。
   - B. 只保证值相同，类型/溢出由后续 runtime 参数约束决定。
   - C. 只是一种优化，不是用户可依赖的语义承诺。
   - Other: …

23. comptime interpretation 是否可以触发编译诊断副作用，即使外层最终仍生成 runtime 代码？
   - A. 可以；诊断是 comptime 求值的可观察结果，需定义执行次数和缓存行为。
   - B. 只有显式 `comptime` block 能触发诊断，隐式 interpretation 必须纯且无诊断。
   - C. 诊断属于 evaluator 内部行为，不作为语言语义承诺。
   - Other: …

### 五、Splice trait 与跨阶段数据流

24. `Splice` trait 的 `RuntimeTarget` 与实现方法的调用关系？
   - A. `Splice for T` 只允许 comptime 输入 `T`，方法返回一个可直接嵌入 runtime 的 `RuntimeTarget` 值；类型和表示由 trait 契约决定。
   - B. 方法返回 CMTT code value，再由另一个阶段操作拼接；普通值 splice 只是内建特例。
   - C. `RuntimeTarget` 只描述类型，不约束实际布局/初始化，具体库契约负责剩余语义。
   - Other: …

25. 有限自动派生的精确边界先采用哪种形式？
   - A. 仅当 struct/enum/union/closure 的所有字段/分支/捕获满足递归 `Splice` 且布局、初始化、资源条件可判定时自动派生。
   - B. 只对无引用、无 Drop、无递归的纯值类型自动派生；其余一律显式实现。
   - C. 自动派生只生成候选实现，最终仍需用户属性确认。
   - Other: …

26. `Splice` 自定义实现与自动派生冲突时如何选？
   - A. 用户实现优先，自动派生不生成候选。
   - B. 自动派生与用户实现同时进入 coherence，重叠即错误。
   - C. 用户必须显式禁用自动派生后才能手写实现。
   - Other: …

27. “comptime/runtime 数据流严格单向”如何与 2LTT quote reify runtime 符号并存？
   - A. 单向规则只约束运行时值流；quote 读取的是语法/类型/效果表示，不是 runtime value。
   - B. quote 是受控的反向数据流，必须显式携带 read/effect 证明。
   - C. 2LTT quote 暂不属于当前 comptime 语言，先只允许 CMTT code quote。
   - Other: …

### 六、开放定义与后续主题边界

28. `impl` 在 loose comptime fn 中生成后，何时可以被同一函数后续语句调用？
   - A. 生成即进入当前 loose context 的线性可见集合；当前函数后续代码可用，但外部引用标 defer。
   - B. 必须等整个 comptime fn instantiation 完成 coherence 后才可用。
   - C. 只能通过显式返回的 Type/implementation value 使用，不能隐式进入作用域。
   - Other: …

29. 静态 scope 无子句参数这一限制是否是当前 comptime 主题的前置公理？
   - A. 是；本主题只依赖它保证模块骨架简单，预静态/预线性作用域留模块主题。
   - B. 否；需要在 comptime 主题先给出最小形式化，否则 loose coherence 无法定义。
   - C. 仅把“静态 scope 不捕获外部子句参数”作为当前局部假设，不承诺完整模块语义。
   - Other: …

30. CMTT 作为第三个元编程层的范围是否暂时只记为接口依赖？
   - A. 是；当前只保留“CMTT quote/splice 会改变 runtime code 的阶段”，不定义其语法/卫生/模态类型。
   - B. 否；comptime 基础章必须先给出 CMTT 的最小 quote/splice 类型，否则当前 splice 叙述不完整。
   - C. 先开 CMTT 独立主题，再回来补 comptime 的交叉引用。
   - Other: …

## 示例语法/风格自检

- 作者 prompt 中的示例按原文存档，标记为 proposal，未升格为正式示例。
- `println("{}", 1 + 23);` 的中缀空白和语句分号符合现行 grammar/style。
- `--|`、`-->` 行在当前注释规则下只能作为说明性注释/伪 IR，不能作为 Flurry 语句；本轮不将其当作语法。
- `refines` 结果类型、CMTT quote/splice、`comptime implicit` 组合、`Splice` 的完整声明规则目前未由现行 syntax 完整确认，均保留为 proposal。
- `trait Splice` 示例中的关联类型与方法体/分隔符需待 trait 主题统一核对；本轮不改写作者原文。

## 文档写入

- 否：作者明确表示现在写正式文档为时尚早。
- 已将作者 `prompt` 全文追加到 `docs_chat_history/023-comptime-02.md`。
- 本轮评估、确认清单和追问写入本文件。

