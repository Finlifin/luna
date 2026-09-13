# 023 — comptime — 02

- 日期：2026-08-22
- global_index：023
- theme：comptime
- subindex：02
- 本轮目标：消化作者对 `022` 的答复，收敛 comptime 的多种 normalization 强度、两种 staged compilation、quote/splice 双重语义与 execution 模型

## 作者本轮输入摘要

作者已在 `prompt` 中回答 `022` 的 17 个问题，并补充了新的总体设计：

1. 理论上沿用 2LTT，但面向后续讨论统一使用 `comptime` / `runtime`，不再使用 `U₀` / `U₁` 作为主要术语。
2. normalization 不是单一强度，至少包括 `normal`、显式 `defer`、`loose`（渐进依赖类型）以及基于 contextual modal type 的 staged compilation；最后一种将另开主题。
3. 设计目标是防止 definition-site 不可检查性扩散，并让 `impl` / `derive` / `extend` / `template` 等开放分散定义保持最终 coherence。
4. 参与 query 缓存的编译时值必须具有 comptime 侧 `Eq`。
5. `lift` 主要用于将 quote 后的 runtime 代码、值或对象类型化。
6. comptime/runtime staging 主要把 comptime 值拼接为 runtime 常量；CMTT staging 把 contextual modal code 拼接为 runtime 代码。quoted code 在底层也属于 comptime 值，但应与普通 comptime 值区分。
7. 可 splice 到 runtime 的用户值要求其成员递归可 splice；作者提出未来可能设计 auto trait。
8. `quote` 有两种语义：2LTT 中将 runtime 符号 reify 为 comptime 值，以表达运行时依赖和内存副作用；CMTT 中产生卫生的 AST/语法数据。
9. `comptime` 参数显式标注；inline 控制流在 normalization 时检查所有分支；`defer` 保留。
10. 在最高 normalization 强度下，命题求解失败即编译错误；`requires` / `ensures` 需要良定义命题，而 inline 条件可以是任意 comptime bool，编译器尝试从中提取良定义命题。
11. comptime 递归允许存在，但有资源上限；trait 的 comptime/runtime 两面通过显式的 `comptime` 方法区分。
12. 新增两种执行模型：runtime fn 定义内部的零星 comptime 求值称为 `comptime interpretation`；独立 `comptime fn` 编译为 comptime engine 字节码，在调用时执行称为 `comptime execution`。

## 可行性评估

### 已形成的强方向

多种 normalization 强度是合理且必要的。普通 runtime 函数内的局部 comptime 求值、显式延迟的依赖类型检查、面向元编程构造的渐进依赖类型，以及 CMTT 的代码阶段，本来就不应被一个“所有未知项都必须立即证明”的规则吞并。

把“防止不可检查性扩散”和“开放分散定义最终 coherence”并列为约束也很关键：前者约束单个定义的检查边界，后者约束跨模块追加 implementation 的全局结果。两者需要独立的失败语义和检查时机。

把 interpretation 与 execution 分开也可行，但必须保证二者的用户可观察语义有明确关系。否则同一个 comptime 表达式放在 runtime fn 内和独立 comptime fn 内，可能出现不同的类型规则、资源限制、错误分类或副作用行为。

### 当前最明显的张力

1. 作者对第 4 题选择 `C`，但“编译器视其为公理而不是待证引理”没有明确指向：是纯性声明、`requires` 命题、`ensures` 命题，还是某类 evaluator 结果。若允许任意用户陈述成为公理，类型系统的健全性边界会改变；若只是把某些内建事实视为公理，则应列出可信基础。
2. 第 6 题选择“默认值必须纯”，而 `prompt.fl` 中仍保留读取 build feature、环境和 source span 的可选子句默认值例子。这是语义冲突，不应自行选边。
3. “quoted code 也是 comptime 值”和“CMTT quote 是 contextual modal type”说明 quote/splice 至少有两个轴：值的阶段与代码的模态上下文。若不建立二维分类，`splice` 很容易被写成一个过宽的总操作。
4. “成员都可 splice，则结构体可 splice”需要处理递归类型、共享引用、未初始化状态、析构/资源所有权、布局属性和用户自定义转换；单纯递归结构规则未必足够。
5. `defer` 的价值不仅是延迟 evaluator，也是在 LSP/增量检查中保留一个可继续处理的约束边界。因此需要定义它延迟的是表达式求值、类型构造、约束检查，还是整个子树的语义分析。

## 已确认 / 仍待定

### 本轮可视为已确认

- 面向语言讨论统一使用 `comptime` / `runtime`；2LTT 仍是理论背景。
- normalization 存在多种强度，不承诺所有代码使用同一检查级别。
- 至少存在 `normal`、显式 `defer`、`loose` 三类讨论对象；CMTT contextual modal staging 暂作为独立主题。
- 设计目标包含防止 def-site 不可检查性扩散与开放分散定义的最终 coherence。
- query 缓存使用的 comptime 值必须有 comptime `Eq`；缓存键至少包含定义身份与显式参数这一方向得到选择。
- lift 的主要对象是 quote 后的 runtime 代码/值/对象的类型化。
- comptime/runtime staging 与 CMTT staging 的 splice 目标不同：前者是 runtime 常量，后者是 runtime 代码。
- 普通 comptime 值与 quoted code 在底层分类上不同；用户值的可 splice 性倾向于递归封闭。
- `comptime` 参数显式标记；inline 分支在 normalization 时都要检查；`defer` 保留。
- 最高 normalization 强度下，`requires` / `ensures` 要求良定义命题；inline 条件允许任意 comptime bool 并尝试提取命题。
- comptime 递归允许，但需要资源上限。
- comptime 方法通过显式 `comptime` 关键字与 runtime 方法区分；二者可以共享基础定义，但语义不同。
- 存在 comptime interpretation 与 comptime execution 两种执行模型。

### 本轮仍不能当作已确认

- “公理”具体指什么，以及哪些事实属于可信基础。
- 第 14 题（依赖结果类型）的正式选择尚未回答。
- 第 6 题与 `prompt.fl` 中环境/source span 默认值的冲突。
- normalization 强度是否由定义、构造、属性或上下文决定，能否嵌套，以及每种强度的失败边界。
- quote 两种语义是否共用关键字/表面形式；其类型表示和卫生规则尚未定。
- splice 的二维分类、可 splice auto trait、结构递归规则的例外。
- interpretation 与 execution 的语义等价、缓存、资源、调试和副作用差异。

## 需要你拍板的问题

### 一、normalization 强度与 coherence

1. normalization 强度由什么决定？
   - A. 由定义或块上的显式模式决定；默认 `normal`，需要更弱检查时必须显式标记。可预测，但增加表面标注。
   - B. 由构造自动决定：普通函数、`defer`、渐进依赖值和 CMTT 各自携带固定强度。使用自然，但同一函数混合多个机制时规则较隐蔽。
   - C. 由上下文推断，并允许编译器在需要时自动降级。表达力高，但容易出现同一源码因调用位置不同而拥有不同检查语义。
   - Other: …

2. 不同 normalization 强度能否在同一函数中嵌套？
   - A. 可以；每个子表达式携带自己的检查契约，外层只能看到其承诺和延迟义务。
   - B. 只能从强到弱进入，不能从弱区域泄漏未检查事实回强区域。规则较稳健，但限制组合能力。
   - C. 不允许混合；函数选择一种强度，跨强度必须通过显式适配器。
   - Other: …

3. “def-site 不可检查性不会扩散”具体要求哪一条？
   - A. 一个 loose/defer 定义可以被 normal 定义调用，但调用点必须承担全部延迟义务。
   - B. loose/defer 定义不得被 normal 定义直接调用，只能先通过证明/封装恢复强契约。
   - C. 允许调用，但编译器必须把未决义务精确附着在调用表达式，不得污染父定义的签名。
   - Other: …

4. 开放分散定义的最终 coherence 在何时检查？
   - A. 完成模块/包的静态闭包后检查；增量编译需使结果与全量检查一致。
   - B. 每次新增 `impl` / `extend` / `derive` 时局部检查，并用全局索引保证最终一致。
   - C. 只有实例化具体类型时检查候选集；定义端只记录待决候选。
   - Other: …

5. coherence 失败属于哪一种错误？
   - A. 开放集合本身产生不可消解的重叠，因此在 coherence 检查阶段报错。
   - B. 只有具体调用无法选出唯一实现时才报 instantiation 错误。
   - C. 两者并存：可证明的重叠在定义端报错，依赖具体参数的重叠延迟到实例化。
   - Other: …

### 二、纯性、公理与缓存

6. 你对第 4 题的“编译器视其为公理”具体指哪一对象？
   - A. `pure comptime fn` 的纯性声明是可信假设；编译器不证明函数无副作用，只按声明决定缓存和调用规则。
   - B. `requires` / `ensures` 中的良定义命题是用户提供的公理；它们可以作为路径条件，但不自动产生内存安全结论。
   - C. 只有语言内建事实和 evaluator 语义规则是公理；用户命题仍需求值、证明或显式 unsafe/axiom 标记。
   - Other: …

7. 如果用户可以声明公理而编译器不验证，其后果是什么？
   - A. 允许不健全但可控的逻辑扩展；公理只影响静态选择，不改变 runtime 代码安全保证。
   - B. 公理可影响类型成立性，因此必须进入 `unsafe`/`verified` 等安全边界。
   - C. 不允许用户公理；“公理”只描述 evaluator 和内建库事实。
   - Other: …

8. 第 6 题选择“默认值必须纯”后，`build`、source span、caller 等调用点信息如何进入 comptime？
   - A. 由调用者显式计算并传参；默认值不能读取这些信息。
   - B. 通过独立的 compiler capability 参数进入，但 capability 本身不是纯值，不能参与普通 query 缓存键。
   - C. 这些例子应删除，调用点信息暂不属于 comptime 语言机制。
   - Other: …

9. comptime `Eq` 对缓存键的要求是什么？
   - A. 只需决定语义相等性；Hash 由缓存实现另行提供或结构化派生。
   - B. 必须同时有 comptime `Eq` 与 comptime `Hash`，且满足相等值哈希相同。
   - C. 不允许用户自定义 Eq 参与缓存；缓存值只能使用内建可判等类型。
   - Other: …

10. 如果 comptime `Eq` 本身依赖待实例化的 trait/类型函数，如何避免缓存递归？
   - A. 缓存键只允许由已 normalization 完成的基础类型构成。
   - B. 允许递归查询，但 evaluator/query 图必须检测 cycle 并报告确定的编译错误。
   - C. Eq 的 comptime 实现不参与普通 query，只在缓存失败时退化为不缓存。
   - Other: …

### 三、quote、lift 与 splice

11. 两种 `quote` 是否保留同一个表面关键字？
   - A. 保留同名，由上下文和结果类型区分；语法简洁，但错误信息必须明确指出是哪一种 quote。
   - B. 使用不同的表面构造或属性；可读性和诊断更好，但需要设计两套语法。
   - C. 统一为一个底层 code/value reification 操作，文档只按类型区分，不承诺两个独立关键字。
   - Other: …

12. 2LTT quote reify runtime 符号时，内存副作用如何表示？
   - A. quote 值携带显式 effect/读写集合，例如 `read<location>`；splice 前必须满足 effect 约束。
   - B. quote 只携带语法和类型，内存副作用由后续类型检查重新推导。
   - C. quote 不允许 reify 具有运行时内存依赖的符号；这类需求留给 CMTT code quote。
   - Other: …

13. 普通 comptime 值与 CMTT quoted code 的 splice 关系是什么？
   - A. 普通值只能 splice 为 runtime 常量，quoted code 只能 splice 为 runtime 代码；两者不可互换。
   - B. quoted code 也可在满足表示约束时作为普通值 splice，代码 splice 是额外的 modal 操作。
   - C. 所有 splice 都统一由目标上下文决定，值/代码分类只影响类型检查。
   - Other: …

14. “结构体成员都可 splice，则结构体可 splice”是否是自动规则？
   - A. 是一种递归 auto trait；布局、初始化和资源条件满足时自动实现。
   - B. 递归规则只是默认候选，用户必须显式声明/派生可 splice 能力。
   - C. 只有编译器内建类型支持自动 splice，用户结构体需手写转换。
   - Other: …

15. 递归可 splice 规则遇到以下对象时如何处理：共享引用、循环结构、未初始化字段、拥有 Drop/析构语义的字段？
   - A. 任何一项出现即静态拒绝，先保证规则可判定。
   - B. 共享引用和循环结构禁止，但 Drop/初始化可由显式 trait 契约处理。
   - C. 允许一般情况，splice trait 负责提供序列化/重建语义；失败延迟到实例化。
   - Other: …

### 四、interpretation 与 execution

16. `comptime interpretation` 与 `comptime execution` 是否必须语义等价？
   - A. 是。同一 comptime 代码无论嵌在 runtime fn 还是独立 comptime fn，都遵守同一求值、类型和副作用语义；区别只在实现载体。
   - B. 只要求纯表达式等价；解释器允许更宽的局部 IR 操作，独立 comptime fn 受字节码 ABI 限制。
   - C. 二者是不同语言子集，用户必须知道所在模型的能力差异。
   - Other: …

17. runtime fn 中的 interpretation 何时执行？
   - A. normalization 结束时对不依赖未知输入的片段就地执行；依赖参数的片段留到 instantiation。
   - B. 只在具体调用发生时执行，即使表达式在定义端已闭合。
   - C. 由编译器自由选择，只要生成代码和诊断可观察行为一致。
   - Other: …

18. 独立 `comptime fn` 的字节码编译与 normalization 的关系？
   - A. 先完成函数定义的 normalization，再编译为 comptime bytecode；字节码执行不重新放宽定义端检查。
   - B. 字节码编译本身就是 loose normalization，可把未决约束留到每次调用。
   - C. 只要函数标有 `comptime` 就直接编译字节码，类型/约束检查由调用时负责。
   - Other: …

19. 两种模型的资源上限是否统一？
   - A. 统一 fuel/时间/内存预算，便于预测和缓存。
   - B. interpretation 使用编译器 pass 预算，execution 使用 comptime engine 预算；失败分类分别定义。
   - C. 资源限制是实现细节，语言只承诺“可能因资源耗尽失败”。
   - Other: …

20. interpretation 与 execution 的副作用边界是否一致？
   - A. 一致；两者都只能执行 pure comptime 操作，非纯能力必须显式传入。
   - B. interpretation 可访问编译器内部局部状态，execution 更严格；这会牺牲可替换性换取实现便利。
   - C. 两者均可访问不纯能力，但所有副作用都必须进入依赖图和缓存键。
   - Other: …

### 五、命题提取与 defer

21. inline 条件是任意 comptime bool，而 `requires` / `ensures` 只接受良定义命题。若编译器无法从 inline bool 提取命题，normalization 应如何处理？
   - A. 把条件视为未知布尔，仍检查两条分支的类型，并不产生额外路径事实。
   - B. 在最高强度下报错，要求作者将条件改写为良定义命题。
   - C. 在 normal 中保守检查，在 loose/defer 中允许延迟到实例化。
   - Other: …

22. “良定义命题”最低包含哪些内容？
   - A. 可归约为 evaluator 可判定的纯 bool 表达式即可。
   - B. 还必须能被类型环境、路径条件和 coherence 查询安全引用。
   - C. 需要显式 theorem/axiom 标记；普通 comptime bool 永远不是命题。
   - Other: …

23. `defer` 延迟的最小单位是什么？
   - A. 只延迟表达式的 comptime 求值；表达式的静态形状和依赖关系仍在 normalization 建立。
   - B. 延迟整个结果类型/约束子树的构造与检查；调用点负责补齐。
   - C. 允许用户指定延迟阶段，既可延迟求值也可延迟类型检查。
   - Other: …

24. `defer` 的未决义务在 LSP/增量编译中如何表示？
   - A. 产生带源范围和依赖列表的稳定 deferred obligation 节点。
   - B. 只保留原 AST，下一次完整实例化时重新分析。
   - C. 由实现自行决定，只保证最终编译结果正确。
   - Other: …

### 六、重新表述依赖结果类型问题

25. 对“依赖结果类型”的精确定义，以下哪项最接近你的目标？
   - A. 一个 comptime 函数可以接收 comptime 值并返回具体 `Type`；类型身份和结构可以依赖该值，调用完成后必须得到闭合类型。
   - B. 在调用完成前允许返回带 refinement 的未知类型；类型操作可以在 `Any` 约束下继续，实例化时再精化或失败。
   - C. 只有 CMTT code value 可以依赖运行时值；普通 `Type` 结果不得依赖尚未确定的值。
   - D. A 与 B 并存：强 normalization 使用 A，loose 使用 B；需要明确从 B 恢复到 A 的边界。
   - Other: …

26. 依赖结果类型若实例化失败，失败归属是什么？
   - A. 属于调用点，因为只有调用点提供了决定类型的 comptime 参数。
   - B. 若失败来自定义中的未满足良定义命题，归属于定义点；若失败来自具体值，归属于调用点。
   - C. 一律作为 deferred obligation，直到整个实例化图完成后统一报告。
   - Other: …

### 七、trait 的 comptime/runtime 两相

27. 同一个 trait 同时包含 comptime 方法和 runtime 方法时，implementation 是否必须同时提供两相？
   - A. 可以只实现其中一相；调用另一相时按缺失方法报错。
   - B. trait 声明的两相是一个整体，implementation 必须同时满足。
   - C. 两相分别形成可独立满足的子 trait，组合 trait 只是语法糖。
   - Other: …

28. comptime 方法能否调用同一类型的 runtime 方法？
   - A. 不能直接调用；必须 quote/reify runtime 操作，或由显式 CMTT staging 生成代码。
   - B. 可以调用，但只对可 lift 的无副作用 runtime 方法开放。
   - C. 可以由编译器自动解释执行 runtime 方法；失败时再退回代码生成。
   - Other: …

29. runtime 方法能否使用 comptime 方法的结果？
   - A. 可以，只要结果满足普通值 splice 规则。
   - B. 只能通过显式 `comptime` 表达式/inline 构造使用，禁止隐式跨相调用。
   - C. 只有类型级结果可用，普通 comptime 值必须由调用者先物化。
   - Other: …

30. trait 的 comptime/runtime 两相是否影响 coherence？
   - A. 两相共享同一个 implementation 候选集和 coherence 规则。
   - B. 两相分别做候选集与 coherence，只有最终调用才检查是否存在跨相冲突。
   - C. comptime 相属于静态作用域，runtime 相属于线性作用域，二者不共享开放定义规则。
   - Other: …

## 文档写入

- 否：作者尚未授权把上述 proposal 写入正式 `docs/`。
- 已将作者 `prompt` 全文追加到 `docs_chat_history/022-comptime-01.md`。
- 本轮评估与问题写入 `docs_chat_history/023-comptime-02.md`。
- 作者提供的 Flurry 代码仅作为 `prompt` 原文存档，未被本轮升格为规范示例或语法承诺。

## 作者答复（prompt，2026-08-23）

1 为了编译器和人类友好，normalization强度一定要在hir阶段就可判断。
    + 对于comptime fn定义、任意comptime块，是loose；
    + 对于类型defer的变量，视其为Any类型，在ordinary强度下表现为完全无约束的泛型一样，
        因此一般来说，如果你想使用它，需要用内联控制流来约束它
    + 对于类型为refines {constraint_expr} type_expr这样的refinement gradual type（也许flurry是第一个提出它的），
        在ordinary强度下，编译器采用内联控制流一样的命题提取逻辑，从expr中提取，而所有约束也像内联控制流的谓词一样会在instantiation执行
    + 对于含有defer类型参数的函数调用，ordinary下，要求编译器就地执行，并进行替换
    + 含有CMTT quote/splice的代码后面再讨论
    + 其他所有代码是ordinary，我们这里统一一下用词
    + loose强度下的开放定义，会在instantiation时再检查全局冲突，这是可以预测的
    + loose强度下不可定义开放的开放分散定义，即impl Tuple<T, T> where T 这样的，一个简单的检查就是不能有子句参数

2 见上。比如comptime fn中直接用builder构造一个类型，并在该函数中impl, 
    那么impl里的runtime fn依然会是ordinary的，而所有对外部（包裹它的即comptime fn）引用都会被标记为defer；
    而一个runtime函数里，也可用comptime block。

3 见上。

4 见上，如果loose中impl在loose环境整体被instantiate时无法通过coherence检查（在静态作用域中的其它impl已经完成coherence检查），则报错

5 A，我的意思是，对于comptime fn，pure标记将其视为公理，因此不会检查实现是否真的pure, 是否真的pure依赖于开发者纪律。
    注意，runtime则不是这样，runtime分了数种内存副作用强度
    （这里的memory effect不要与algebraic effect混淆， 对于flurry这样一个底层语言来说， 后者主要用于控制流抽象）

7 无论对于PTC还是程序验证来说，都不允许用户公理，需要逃逸时请使用assumes语句来有约束地逃逸检查。
    注意，PTC和程序验证是两套相对独立的系统，只是它们共享了一套语法基础设施

8 那就不要通过可选子句参数做这个，请定义独立implicit参数，这里参数会由编译器自行传入，而不需要用户传入。
    比如： pure comptime fn Foo<implicit __src__: meta.compilation.source.Span> -> Type，不过一般来说这个定义没啥用，没人真的这样写，
    更常见的是fn log(..., comptime implicit __src__: meta.compilation.source.Span)这样的使用，后面函数主题会讲implicit parameter。

9 B

10 Eq和Hash特殊处理

11 同一个关键字即可。
12 内存管理系统主题再讲。编译时计算是flurry的基石，许多其他系统都建立在此之上，不可能在本主题中就一下讲完
13 都说了到CMTT主题再讲。
14 也许该定义它了，我想了一下，它还不能是auto trait，为了控制编译器的复杂度，只有struct/enum/union/闭包等类型会在条件符合时自动派生
    ```flurry
    trait Splice {
        assoc RuntimeTarget: Type,
        
        fn splice(comptime data: Self) -> RuntimeTarget
    }
    ```
15 A, 自动Splice派生非常有限，有关设计会刻意引导用户实现Splice来自行定义精准语义
16 A
17 normalization阶段，而且可能会在多趟中完成，这个要延迟到编译器实现来决策。
18 A, 所有代码都有normalization和instantiation两个大过程，而gradual dependent type带来的动态性不代表完全放弃normalization能做的检查
19 B
20 两者一致，但是不要求纯、终止、全，实际上鼓励开发者利用comptime编写优秀的编译报错
21 A
22 这个将留到类型主题中的propositional type constraints章节介绍。
23 A
24 C
25 实际上这些都允许，type theme再讲
26 A
27 A，comptime expr只会调用comptime相，runtime expr只会调用runtime相方法
28 A，comptime和runtime之间的数据流是严格单项流动的，quote等操作看起来是在操作运行时内容，实际上只是在"AST/IR"之类的层级操作
29 A，显然可以
30 A。你对flurry中静态作用域的理解有些偏差，但这是模块主题的内容， 现在你只需要知道，静态作用域就是类似mod/struct/enum这样的定义下的作用域，
    要求它们不能有子句参数，因此简单到足够编译器能基于其建立基本模块结构。


实际上，即使是runtime 上下文，一个表达式也会优先被视作comptime表达式，例如，当我们写下`println("{}", 1 + 23);`时，
`+`的两个子节点都是还是comptime（即没有在子表达式中有运行时义务），因此这里就是comptime interpretation发生的地方，1 + 23会被计算为24。
下面是另一个前面已经提到过的例子:
```flurry
fn foo(name: T) where T, requires T:- Display or T:- AsStr {
    println("Hello, {}!", name);
    --> println: fn(comptime format: str, ...args: defer std.fmt.Parse(format))
    --| e0: str = str_literal("Hello, {}!")
    --| runtime e1: T = param(name)
    --| e2: Type = defer std.fmt.Parse(e0) ⇓ Tuple<refines Any { Itself:- Display }>
    --| runtime e3: e2 = tuple(e1)
    --| runtime e4: void = apply(println, e0, e3)
    --| solve(clause ∧ path_condition ∧ model)
}
```

另外我纠正一下，子句参数（以后统一叫子句参数而不是"泛型参数"， flurry中基本不直接用泛型这个概念）里的可选参数，
其默认值在normalization时要求直接执行，而不是要求纯，除非被defer包裹，因此这是defer的第3个具体用途。
