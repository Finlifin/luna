# Flurry 演进手记 (`flurry_tutorial`) —— 200 篇全景目录与路线图

欢迎来到 **《Flurry 演进手记》**。

这里不是一份枯燥的语法说明书，也不是按字母表排列的特性词典。
本系列教程通过 **200 篇循序渐进的工程实战专题**，从具体系统编程问题出发，在分析边界条件与演进设计的过程中，一步步展开 Flurry 语言的核心机制与工程实践。

> **关于生态假设与权威规范**：由于 Flurry 目前正处于语言核心与编译器实现阶段（内置包与标准库仍在设计中），本系列教程**专注于语言本身的设计与心智模型**。所有示例均严格遵循 [`docs/`](../docs/README.md) 中的正式语言规范，不依赖尚未定型的外部库生态。

---

## 第一卷：从一个字节的硬件协议开始——基础语法、控制流与数据结构 (001–020)
*故事主线：从零开发一套 8 比特硬件传感器与 RISC-V 微控制器解码监控固件。*

- [x] **[001：一个字节里的世界——从繁琐掩码到定宽模式](./001-a-world-in-a-byte.md)** (`docs/control-flow/pattern-matching/bitvec.md`, `docs/type/user-defined-types/enums.md`)
- [x] **[002：当传感器遇到噪声——从可选类型 `?T` 到错误行 `!Err T`](./002-when-sensors-meet-noise.md)** (`docs/type/qualified-types/error-type.md`, `docs/algebraic-effect-and-delimited-continuation/error-handling.md`)
- [x] **[003：`let` 并没有冻结你的对象——绑定、字段指针与失败安全的资源更替](./003-let-does-not-freeze-your-object.md)** (`docs/resource-management/bindings.md`, `docs/resource-management/pointers.md`)
- [x] **[004：为什么 `a<b` 不是小于号——词法边界、空白规则与省略分号的哲学](./004-why-a-lt-b-is-not-less-than.md)** (`docs/basics/lexical.md`, `docs/grammar/index.md`)
- [x] **[005：给数字贴上单位标签——编译期字面量、`Symbol` 与 `LiteralInto` 后缀协议](./005-literals-symbols-and-custom-suffixes.md)** (`docs/basics/literals.md`, `docs/traits/protocols.md`)
- [x] **[006：空无一物的三种面孔——`()`、`null`、`_` 与危险的 `undefined`](./006-three-faces-of-nothingness.md)** (`docs/basics/unit-null-undefined.md`, `docs/basics/primitives.md`)
- [x] **[007：状态机分支大扫除——表达式化的 `if`/`when` 与带 `while` 守卫的跳转](./007-branching-with-if-when-and-while-guards.md)** (`docs/control-flow/basic.md`)
- [x] **[008：跳出深层嵌套迷宫——带标签的 `for:label`、`while:label` 与 `never` 类型推导](./008-escaping-nested-loops-with-labels-and-never.md)** (`docs/control-flow/basic.md`)
- [x] **[009：把流式解析写得像呼吸一样自然——`while ... is ... do` 与 `let ... else`](./009-streaming-parsers-with-while-is-and-let-else.md)** (`docs/control-flow/basic.md`, `docs/control-flow/pattern-matching/index.md`)
- [x] **[010：给数据包做外科手术——模式优先级、括号分组 `(p)` vs `(p,)` 与范围模式](./010-pattern-precedence-grouping-and-ranges.md)** (`docs/control-flow/pattern-matching/index.md`)
- [x] **[011：当 `NaN` 闯入浮点数匹配——区间覆盖证明与纯守卫的穷尽性分析](./011-when-nan-crashes-float-matching.md)** (`docs/control-flow/pattern-matching/index.md`)
- [x] **[012：字段顺序不等于内存顺序——`struct` 声明序求值、默认字段与物理布局解耦](./012-struct-evaluation-order-vs-memory-layout.md)** (`docs/type/user-defined-types/structs.md`)
- [x] **[013：从高层消息到底层枚举——`enum` 分组变体 `.network.{...}` 与 `'tag_name` 视图](./013-enum-grouped-variants-and-tag-names.md)** (`docs/type/user-defined-types/enums.md`)
- [x] **[014：手写 RISC-V 指令解码器——32 位定宽 `enum` 的互斥性与覆盖性证明](./014-riscv-decoder-with-fixed-width-enums.md)** (`docs/type/user-defined-types/fixed-width-enum-example.md`)
- [x] **[015：跨越字节边界的比特流——跨元素 `BitVec` 模式与 UTF-8 变长解码](./015-cross-element-bitvec-patterns-and-utf8.md)** (`docs/control-flow/pattern-matching/bitvec.md`)
- [x] **[016：不用移位和掩码怎么写位操作——`BitVec<n>` 拼接 `++`、切片与原位回写](./016-bitvec-concatenation-slicing-and-writeback.md)** (`docs/program-synthesis/bitvec-algebra.md`)
- [x] **[017：与硬件寄存器赤身肉搏——`union` 共享存储、C ABI 与显式初始化契约](./017-unions-shared-storage-and-c-abi.md)** (`docs/type/user-defined-types/unions.md`)
- [x] **[018：同样是 `u32`，凭什么不能混用——`newtype` 独立类型身份与 `typealias` 透明别名](./018-newtype-identity-vs-typealias.md)** (`docs/type/user-defined-types/newtype-and-typealias.md`)
- [x] **[019：在不暴露内部结构的情况下匹配——`case` 自定义匹配视图](./019-custom-pattern-views-with-case.md)** (`docs/control-flow/pattern-matching/index.md`)
- [x] **[020：第一卷里程碑：组装一台完整的 8 位传感器协议分析仪](./020-milestone-sensor-protocol-analyzer.md)** (综合实战 001–019)

---

## 第二卷：函数的十八般武艺——参数协议、模式链、闭包与模板 (021–040)
*故事主线：为传感器分析仪构建一套可配置、支持插件回调、零样板代码的查询与事件处理引擎。*

- [x] **[021：实参求值的核心规则——源码从左到右求值、方法接收者与失败回滚边界](./021-call-evaluation-order-and-failure-boundaries.md)** (`docs/functions/calls-and-returns.md`)
- [x] **[022：让配置接口告别重载膨胀——可选参数 `.id = expr` 与前序参数依赖](./022-optional-parameters-and-forward-dependencies.md)** (`docs/functions/parameters.md`)
- [x] **[023：为什么默认参数里不能偷偷打日志——运行时默认值与 `P3` 稳定观测契约](./023-why-default-params-require-p3-stability.md)** (`docs/functions/parameters.md`, `docs/resource-management/runtime-purity.md`)
- [x] **[024：把异构参数打包进元组——位置变参 `...items` 与 `.items = tuple` 完整供应](./024-variadic-tuples-and-direct-supply.md)** (`docs/functions/parameters.md`)
- [x] **[025：像写 JSON 一样调用函数——扩展调用 `fn { k: v }` 与匿名结构体属性收集](./025-extended-applications-and-property-collection.md)** (`docs/functions/parameters.md`, `docs/control-flow/pattern-matching/index.md`)
- [x] **[026：上下文自动注入与尾随块——`implicit` 参数、`post do` 与 `tactic lambda`](./026-implicit-parameters-and-post-do-blocks.md)** (`docs/functions/parameter-elaboration.md`)
- [x] **[027：直接按数据形状分派函数——`case` 参数与多输入模式函数](./027-pattern-functions-and-case-parameters.md)** (`docs/functions/pattern-functions.md`)
- [x] **[028：在模式匹配中途算个值——`and expr is pat` 链式顺序匹配与局部观察块](./028-chained-matching-with-and-is-and-observation-blocks.md)** (`docs/control-flow/pattern-matching/index.md`)
- [x] **[029：既要检查字段又要搬走整个包——仿射类型的 `as whole` 与 `or` 模式所有权汇合](./029-affine-pattern-matching-as-whole-and-or.md)** (`docs/control-flow/pattern-matching/index.md`)
- [x] **[030：当 Lambda 捕获外部世界——`|x| expr`、按值捕获与最小独立位置指针观察](./030-lambdas-value-capture-and-pointer-observation.md)** (`docs/functions/lambdas.md`)
- [x] **[031：闭包环境的生死簿——`%c fn` 闭包限定类型、捕获状态更新与逆序清理](./031-closure-environments-and-cleanup-order.md)** (`docs/functions/closure-environments.md`, `docs/type/qualified-types/closure-type.md`)
- [x] **[032：写一个零开销的过滤器——高阶函数接口、环境传递与回调纯度契约](./032-higher-order-functions-and-callback-contracts.md)** (`docs/functions/higher-order-functions.md`)
- [x] **[033：比宏更干净、比普通函数更透明——`inline fn` 与编译期确定性展开](./033-inline-functions-and-deterministic-expansion.md)** (`docs/functions/inline-and-quote.md`)
- [x] **[034：实参只求值一次的代码拼装——`quote fn` 与普通宏的本质区别](./034-quote-functions-evaluate-arguments-once.md)** (`docs/functions/inline-and-quote.md`)
- [x] **[035：一套签名、百种实现与批量验收——`template fn` 与 `template test`](./035-function-templates-and-test-templates.md)** (`docs/functions/templates.md`)
- [x] **[036：指针层级再深也能一键调用——方法接收者适配与三层查找优先级](./036-receiver-adaptation-and-three-tier-lookup.md)** (`docs/type/receiver-adaptation.md`, `docs/type/member-projection.md`)
- [x] **[037：同一数值的多种人格——`.as(View)` 类型视角链与显式接口消歧](./037-type-view-chains-and-disambiguation.md)** (`docs/type/receiver-adaptation.md`)
- [x] **[038：把项目拆成清晰的模块树——文件映射、`mod`、父级查找与顺序遮蔽](./038-modules-file-mapping-and-sequential-shadowing.md)** (`docs/code-organization/modules-and-sources.md`, `docs/code-organization/scope-and-paths.md`)
- [x] **[039：精确控制谁能看见什么——`use` 导入、重导出、`private`/`pub` 与六个内置包](./039-imports-visibility-and-six-builtin-packages.md)** (`docs/code-organization/imports-and-reexports.md`, `docs/code-organization/visibility.md`, `docs/code-organization/builtin-packages.md`)
- [x] **[040：第二卷里程碑：构建模块化、可插拔的流式数据处理管线](./040-milestone-modular-stream-pipeline.md)** (综合实战 021–039)

---

## 第三卷：告别借用检查器焦虑——所有权、可达性与运行时纯度 (041–065)
*故事主线：从零实现一套高性能零拷贝环形缓冲区（RingBuffer）与内存池分配器。*

- [x] **[041：三个层次的值宿命——`Once` 消耗、显式 `Clone` 与位级 `Copy`](./041-once-clone-and-copy-responsibilities.md)** (`docs/resource-management/once-clone-copy.md`, `docs/type/affine-type.md`)
- [x] **[042：把结构体拆开搬走一半——部分移出（Partial Move）、重新初始化与合并交付](./042-partial-moves-and-reinitialization.md)** (`docs/resource-management/moves-and-initialization.md`)
- [x] **[043：当对象内部指向自己——自引用结构（Self-Reference）的移动与安全锚定](./043-self-referential-structs-and-safe-anchoring.md)** (`docs/resource-management/moves-and-initialization.md`, `docs/type/self-reference.md`)
- [x] **[044：析构函数背后的精密钟表——局部与聚合 `Drop` 次序、失败路径与环境约束](./044-drop-elaboration-and-failure-paths.md)** (`docs/resource-management/drop-elaboration.md`)
- [x] **[045：那些一闪而过的匿名值——临时值生命周期、视图保活与 `_` 立即丢弃规则](./045-temporaries-view-lifetime-and-wildcard-drop.md)** (`docs/resource-management/temporaries.md`)
- [x] **[046：为什么 Flurry 不需要 `&mut` 独占锁——指针目标身份、世代（Generation）与顺序别名](./046-pointer-identities-generations-and-sequential-aliasing.md)** (`docs/resource-management/pointers.md`, `docs/resource-management/reachability.md`)
- [x] **[047：追踪指针从哪里来、到哪里去——可达性限定类型 `&reachability_set T` 入门](./047-reachability-types-tracking-origins.md)** (`docs/resource-management/reachability.md`, `docs/type/reachability-type.md`)
- [x] **[048：阻止悬垂指针逃逸出作用域——返回值、容器与闭包中的可达性检查](./048-preventing-dangling-escapes-with-reachability.md)** (`docs/resource-management/reachability.md`, `docs/type/qualified-types/reachability-type-also.md`)
- [x] **[049：把一块大缓冲区安全切成两半——内存视图（Memory Views）、字段视图与区间视图](./049-memory-views-fields-and-intervals.md)** (`docs/resource-management/memory-views.md`)
- [x] **[050：证明两个切片绝不重叠——分离证据（Disjointness Evidence）与并行原地修改](./050-disjointness-evidence-and-in-place-mutation.md)** (`docs/resource-management/memory-views.md`)
- [x] **[051：给每次内存触碰记账——六种内存效果（`read`/`write`/`alloc`/`free`...）与行为上界](./051-six-memory-effects-and-behavior-bounds.md)** (`docs/resource-management/memory-effects.md`)
- [x] **[052：函数退出时留下了什么——出口状态契约與跨调用事实保持](./052-exit-states-and-fact-preservation.md)** (`docs/resource-management/memory-effects.md`, `docs/functions/contracts-and-effects.md`)
- [x] **[053：“纯函数”不是非黑即白——运行时纯度的六层阶梯（`P0` 到 `P5`）全解](./053-six-tiers-of-runtime-purity-p0-to-p5.md)** (`docs/resource-management/runtime-purity.md`)
- [x] **[054：`pure fn` 的真正含义——稳定观测、局部可变性与编译器合法优化前提](./054-what-pure-fn-really-guarantees.md)** (`docs/resource-management/runtime-purity.md`)
- [x] **[055：当编译器能自动算清下标——三次以内多元多项式的自动算术安全证明](./055-cubic-polynomial-automatic-bounds-proving.md)** (`docs/resource-management/contracts-and-verification.md`)
- [x] **[056：把动态检查变成静态事实——`assumes` 断言、事实版本失效与分支精化](./056-assumes-runtime-checks-and-fact-invalidation.md)** (`docs/resource-management/contracts-and-verification.md`)
- [x] **[057：在 函数边界立下军令状——`where requires` 前置条件与 `ensures` 后置条件](./057-preconditions-requires-and-postconditions-ensures.md)** (`docs/functions/contracts-and-effects.md`, `docs/resource-management/contracts-and-verification.md`)
- [x] **[058：证明你的递归一定能停下来——`decreases` 终止性度量与良基关系](./058-proving-termination-with-decreases.md)** (`docs/functions/contracts-and-effects.md`)
- [x] **[059：安全的三重境界——`normal`（默认安全）、`unsafe` 与 `verified` 的边界交互](./059-three-levels-of-safety-normal-unsafe-verified.md)** (`docs/program-verification/safety.md`)
- [x] **[060：写出第一段经过数学证明的代码——`predicate` 与 `verified fn` 实战](./060-writing-your-first-verified-function.md)** (`docs/resource-management/contracts-and-verification.md`, `docs/program-verification/safety.md`)
- [x] **[061：用 Outcome Separation Logic 刻画状态转移——`where outcomes` 子句详解](./061-outcome-separation-logic-and-outcomes-clauses.md)** (`docs/resource-management/contracts-and-verification.md`, `docs/resource-management/pointers.md`)
- [x] **[062：高阶回调里的资源契约——当闭包遇到内存效果与可达性验证](./062-verifying-higher-order-callbacks-and-resources.md)** (`docs/functions/higher-order-functions.md`, `docs/resource-management/contracts-and-verification.md`)
- [x] **[063：复杂控制流中的移动与析构——分支汇合、循环不变量与提前返回清理](./063-compiler-view-of-moves-and-drop-elaboration.md)** (`docs/resource-management/moves-and-initialization.md`, `docs/resource-management/drop-elaboration.md`)
- [x] **[064：安全地跨过 C 语言边界——`^attrs.abi(.c)`、裸指针契约与 FFI 封装](./064-safe-ffi-boundaries-and-c-abi-attributes.md)** (`docs/others/ffi.md`, `docs/program-verification/safety.md`)
- [x] **[065：第三卷里程碑：实现一个零拷贝、带静态越界与分离证明的环形缓冲区 `RingBuffer`](./065-milestone-verified-zero-copy-ring-buffer.md)** (综合实战 041–064)

---

## 第四卷：元编程即类型论——走进两级类型论 (2LTT) 与编译期宇宙 (066–095)
*故事主线：打造一套在编译期自动完成维度检查与广播推导的静态形状张量库（Static-Shaped Tensor）。*

- [x] **[066：为什么泛型不需要第二套语法——编译期宇宙 ($U_1$) 与运行时宇宙 ($U_0$) 初探](./066-two-universes-u1-and-u0-intro.md)** (`docs/comptime/index.md`, `docs/type/2-level-type.md`)
- [x] **[067：普通函数怎么变成类型构造器——`comptime fn` 与返回 `Type` 的函数](./067-functions-that-return-types.md)** (`docs/functions/comptime_fn.md`, `docs/comptime/purity-and-normal-form.md`)
- [x] **[068：为什么 `Identity<T>` 不等于 `T`——`pure comptime fn Name<...>` 与 Normal-Form 头](./068-normal-form-heads-and-type-identity.md)** (`docs/comptime/purity-and-normal-form.md`, `docs/type/type-identity.md`)
- [x] **[069：连元组都是一个类型函数——深入 `pure comptime fn Tuple<...Es: meta.List<Type>> -> Type`](./069-tuple-as-a-variadic-normal-form-type-function.md)** (`docs/comptime/purity-and-normal-form.md`)
- [x] **[070：把类型参数写进 `where` 子句——子句声明、参数阶段依赖与隐式类型推导](./070-where-clauses-and-parameter-stage-dependencies.md)** (`docs/functions/comptime_fn.md`, `docs/grammar/syntax`)
- [x] **[071：在定义时抓出泛型 Bug——Normalization（定义端符号检查）与 Instantiation（调用端实例化）](./071-normalization-vs-instantiation-two-phases.md)** (`docs/comptime/normalization-and-instantiation.md`)
- [x] **[072：什么时候算、什么时候留——Ordinary 严格求值、Loose 宽松归约与 `defer` 延迟表达式](./072-ordinary-loose-reduction-and-defer.md)** (`docs/comptime/normalization-and-instantiation.md`)
- [x] **[073：穿越两个宇宙的桥梁——隐式与显式的 `Lift`、Quote `'(` 与 Splice `~<...>`](./073-lift-quote-and-splice-across-universes.md)** (`docs/comptime/quoting-and-splicing.md`, `docs/basics/primitives.md`)
- [x] **[074：为什么整数字面量叫 `Integer`——编译期无限精度类型向运行时 Proxy 的 Splice 落地](./074-comptime-integer-float-and-runtime-proxies.md)** (`docs/basics/primitives.md`, `docs/comptime/quoting-and-splicing.md`)
- [x] **[075：把循环和分支在编译期彻底摊平——`comptime { ... }` 块与 `inline if` / `inline for`](./075-comptime-blocks-and-inline-control-flow.md)** (`docs/comptime/inlining-comptime-code.md`)
- [x] **[076：在编译期透视任何类型的骨骼——`Type.size_of`、结构反射与内存布局查询](./076-type-reflection-and-layout-queries.md)** (`docs/type/reflection-and-layout.md`, `docs/type/type_id.md`)
- [x] **[077：元数据也是正经的编译期对象——`^expr` 属性系统、`.id expr` 配置与 `attrs` 包](./077-attribute-system-and-configuration-objects.md)** (`docs/comptime/attribute.md`)
- [x] **[078：当编译期计算遇到副作用与缓存——Comptime 纯度、确定性缓存与分阶段实例化](./078-comptime-purity-memoization-and-freezing.md)** (`docs/comptime/purity-and-normal-form.md`)
- [x] **[079：在类型里写 `Any` 会变成动态语言吗——编译期渐进依赖类型（Gradual Dependent Type）](./079-any-and-comptime-gradual-dependent-types.md)** (`docs/type/comptime-gradual-dependent-type.md`)
- [x] **[080：给类型加上逻辑筛网——精化类型 `refines { expr } T` 与编译期谓词](./080-refinement-types-and-comptime-predicates.md)** (`docs/type/refined-type.md`)
- [x] **[081：当数组长度来自运行时变量——运行时依赖类型（Runtime Dependent Types）入门](./081-runtime-dependent-types-intro.md)** (`docs/type/runtime-dependent-type.md`)
- [x] **[082：运行时依赖类型的等价与替换——动态环境、绑定身份与跨作用域传递](./082-holed-type-cores-and-substitution-equality.md)** (`docs/type/runtime-dependent-type.md`, `docs/type/type-identity.md`)
- [x] **[083：在类型自身上做模式匹配——`expr<type_pattern_argument*>` 类型模式详解](./083-pattern-matching-on-types.md)** (`docs/type/type-patterns.md`, `docs/grammar/syntax`)
- [x] **[084：初识命题类型约束（PTC）——为什么类型系统需要一个专门的逻辑求解器](./084-propositional-type-constraints-ptc-intro.md)** (`docs/type/propositional-constraints/index.md`)
- [x] **[085：PTC 的 19 类原子命题——从类型相等、子类型到 Trait 满足性与位宽关系](./085-the-19-atomic-propositions-of-ptc.md)** (`docs/type/propositional-constraints/atoms.md`)
- [x] **[086：走进分支后类型自动变精确了——Occurrence Typing（出现类型推导）的奥秘](./086-occurrence-typing-in-branches.md)** (`docs/type/propositional-constraints/occurrence-typing.md`)
- [x] **[087：读懂类型约束与诊断信息——组合约束推理、残余条件与签名设计](./087-ptc-theory-solvers-and-diagnostics.md)** (`docs/type/propositional-constraints/index.md`, `docs/type/propositional-constraints/atoms.md`)
- [x] **[088：把枚举行与记录行玩出花——Row Polymorphism（行多态）与唯一标签路由](./088-row-polymorphism-and-scoped-labels.md)** (`docs/type/rows.md`)
- [x] **[089：任意位宽整数 `u5`、`i24` 是怎么炼成的——参数化位宽类型与算术边界](./089-arbitrary-sized-integers-and-bit-widths.md)** (`docs/others/arbitary-sized-integers.md`)
- [x] **[090：一切视角皆 `'`——深入 Take View (`expr'id`) 的语言级扩展机制](./090-deep-dive-into-take-view-mechanism.md)** (`docs/others/view-taking.md`)
- [x] **[091：设计静态形状张量（一）：用编译期列表表示维度 `Shape` 与 `Tensor<T, .dims = s>`](./091-static-tensors-part1-shapes-and-types.md)** (`docs/comptime/static-shaped-tensors.md`)
- [x] **[092：设计静态形状张量（二）：在编译期计算广播规则（Broadcasting）与形状兼容性](./092-static-tensors-part2-comptime-broadcasting.md)** (`docs/comptime/static-shaped-tensors.md`)
- [x] **[093：设计静态形状张量（三）：利用 `inline` 展开生成零开销的多维步长寻址代码](./093-static-tensors-part3-zero-cost-stride-inlining.md)** (`docs/comptime/static-shaped-tensors.md`)
- [x] **[094：根据构建环境定制代码——`build.mode`、包元数据查询与跨平台编译期配置](./094-build-context-and-comptime-configuration.md)** (`docs/code-organization/builtin-packages.md`, `docs/code-organization/configuration-and-scripts.md`)
- [x] **[095：第四卷里程碑：完成一个支持编译期维度检查、广播与内核融合的微型张量引擎](./095-milestone-static-shaped-tensor-engine.md)** (综合实战 066–094)

---

## 第五卷：契约式多态——Trait 系统、两相一致性与自动派生 (096–115)
*故事主线：为传感器与张量引擎设计一套支持零成本泛型、编译期计算与按需动态擦除的序列化/格式化框架。*

- [x] **[096：给类型立下能力契约——`trait` 声明、关联项 `assoc` 与 `impl Trait for Type`](./096-traits-associated-items-and-implementations.md)** (`docs/traits/index.md`, `docs/traits/declarations.md`)
- [x] **[097：组合多个 Trait 约束——`where T :- A + B` 正向约束与 `requires` 否定约束](./097-combining-trait-bounds-and-negative-constraints.md)** (`docs/traits/declarations.md`, `docs/grammar/syntax`)
- [x] **[098：当多个 `impl` 看起来都能匹配——结构索引、特化精度与实现选择算法](./098-implementation-resolution-and-structural-specificity.md)** (`docs/traits/implementation-resolution.md`)
- [x] **[099：守住全局唯一性的底线——Coherence（一致性）、孤儿规则与负向推理证据](./099-trait-coherence-and-orphan-rules.md)** (`docs/traits/coherence.md`)
- [x] **[100：Trait 在两个宇宙里的双重生活——`^comptime_phase`、`comptime impl` 与共享一致性](./100-the-two-comptime-phases-of-traits.md)** (`docs/comptime/the-comptime-phases-of-traits.md`)
- [x] **[101：为什么编译期算好的 Hash 能直接带进运行时——`Eq`/`Hash` 与跨阶段 `Splice` 契约](./101-eq-hash-and-cross-stage-splice-traits.md)** (`docs/comptime/the-comptime-phases-of-traits.md`)
- [x] **[102：揭秘语法糖背后的正规军——算术、比较、索引与 `LiteralInto` 语言协议全览](./102-language-protocols-behind-operators-and-syntax.md)** (`docs/traits/protocols.md`)
- [x] **[103：定制你自己的变参与属性收集器——参数收集协议与 Elaboration 接入](./103-custom-variadic-and-property-collection-protocols.md)** (`docs/functions/parameter-elaboration.md`, `docs/traits/protocols.md`)
- [x] **[104：告别黑盒过程宏派生——用纯语言级 `derive fn` 编写类型安全的自动派生器](./104-writing-type-safe-derivers-with-derive-fn.md)** (`docs/traits/derivation.md`)
- [x] **[105：具名派生策略与默认策略——`^attrs.derivation.default` 与 `derive Trait for Type`](./105-named-and-default-derivation-strategies.md)** (`docs/traits/derivation.md`, `docs/code-organization/builtin-packages.md`)
- [x] **[106：当编译期不知道具体类型时——`dyn Trait` 动态接口与 `.dyn(Trait)` 显式擦除](./106-dynamic-interfaces-dyn-trait-and-erasure.md)** (`docs/traits/dynamic-interfaces.md`)
- [x] **[107：连返回 `Self` 和带依赖环境都能动态调用？——Flurry 动态接口的高级边界](./107-self-results-and-dependent-environments-in-dyn-traits.md)** (`docs/traits/dynamic-interfaces.md`)
- [x] **[108：开放实现绑定是怎么传递环境的——替换推导（Substitution Inference）与依赖交付](./108-substitution-inference-and-dependency-delivery.md)** (`docs/type/substitution-inference.md`)
- [x] **[109：在 Trait 方法里优雅地处理错误与效应——关联错误行与效应行多态](./109-associated-error-and-effect-rows-in-traits.md)** (`docs/traits/declarations.md`, `docs/type/rows.md`)
- [x] **[110：设计零拷贝序列化 Trait `Encode` / `Decode`（一）：静态定宽与动态变长的统一接口](./110-zero-copy-codec-trait-part1-interface.md)** (`docs/traits/index.md`)
- [x] **[111：设计零拷贝序列化 Trait（二）：利用结构反射与 `derive fn` 自动生成编解码实现](./111-zero-copy-codec-trait-part2-auto-deriver.md)** (`docs/traits/derivation.md`, `docs/type/reflection-and-layout.md`)
- [x] **[112：设计零拷贝序列化 Trait（三）：在编译期预计算固定帧长与校验和表](./112-zero-copy-codec-trait-part3-comptime-precomputation.md)** (`docs/comptime/the-comptime-phases-of-traits.md`)
- [x] **[113：当多个接口有同名方法——显式视角 `.as(Trait)` 与方法消歧最佳实践](./113-disambiguating-trait-methods-with-as-views.md)** (`docs/type/receiver-adaptation.md`)
- [x] **[114：跨包扩展与 Umbrella 架构中的 Trait 组织艺术](./114-organizing-traits-across-packages-and-umbrellas.md)** (`docs/code-organization/packages-and-umbrella.md`, `docs/traits/coherence.md`)
- [x] **[115：第五卷里程碑：完成支持自动派生、编译期校验与动态插件的通用编解码器框架](./115-milestone-universal-codec-framework.md)** (综合实战 096–114)

---

## 第六卷：带类型的元编程革命——上下文模态类型论 (CMTT) 与宏系统 (116–140)
*故事主线：构建一个在编译期把正则表达式/协议规范直接编译成零分支跳转冗余状态机（FSM）的代码生成器。*

- [x] **[116：为什么字符串拼接宏和 AST 宏还不够好——带上下文类型的代码值（CMTT）登场](./116-why-cmtt-typed-contextual-code-values.md)** (`docs/cmtt/index.md`)
- [x] **[117：读懂 `< x: i32 |- u64 >`——显式运行时上下文（Context）与表达式代码类型](./117-understanding-contextual-expression-types.md)** (`docs/cmtt/index.md`, `docs/cmtt/quotation-and-stages.md`)
- [x] **[118：把代码装进盒子再拼出来——Quote `'(`、Splice `~<...>` 与阶段安全保证](./118-quoting-and-splicing-typed-code-fragments.md)** (`docs/cmtt/quotation-and-stages.md`)
- [x] **[119：连接两段代码的管道工——上下文替换 `~[ a = b ] <expr>` 与变量重绑定](./119-context-substitution-and-rebinding.md)** (`docs/cmtt/contexts-and-substitution.md`)
- [x] **[120：连模式匹配也能在编译期动态拼装。——模式代码类型 `< T -| x: U >` 与 `' <pat>`](./120-contextual-pattern-types-and-quoted-patterns.md)** (`docs/cmtt/patterns-and-arms.md`)
- [x] **[121：动态组装一个 `match` 表达式——Quoted Arm 与 `match { use arms }` 接入协议](./121-quoted-arms-and-match-use-injection.md)** (`docs/cmtt/patterns-and-arms.md`, `docs/grammar/syntax`)
- [x] **[122：当生成的代码类型还不确定时——CMTT 中的 `Any` 与代码见证（Code Evidence）](./122-any-and-code-evidence-in-cmtt.md)** (`docs/cmtt/any-and-code-evidence.md`)
- [x] **[123：深入 `quote fn` 与 `template quote fn`——可复用的带上下文代码生成组件](./123-quote-functions-and-templates-in-depth.md)** (`docs/cmtt/quote-functions-and-templates.md`)
- [x] **[124：用单洞上下文组合出完整循环与状态机——有限程序组装（Program Construction）](./124-assembling-finite-programs-and-automata.md)** (`docs/cmtt/program-construction.md`)
- [x] **[125：实战 CMTT：把声明式状态转移表编译成内联的零开销跳转代码](./125-compiling-state-tables-to-inline-code-with-cmtt.md)** (`docs/cmtt/program-construction.md`)
- [x] **[126：模式匹配背后的超优化引擎——匹配自动机（Matching Automata）与决策树合成](./126-matching-automata-and-decision-tree-synthesis.md)** (`docs/program-synthesis/matching-automata.md`)
- [x] **[127：轻量级文件内语法减负——模板宏 `define id` 与 `$id` 按组展开](./127-lightweight-template-macros-define-and-expand.md)** (`docs/macro/template-macro.md`)
- [x] **[128：模板宏的边界与卫生性——为什么模板宏先接回调用位置再做语法检查](./128-template-macro-hygiene-and-checking-boundaries.md)** (`docs/macro/template-macro.md`)
- [x] **[129：处理结构化 AST 输入——构造宏 `@define_macro1` 与 `{{ ... }}` Token Buffer](./129-ast-construction-macros-and-token-buffers.md)** (`docs/macro/construction-macro.md`)
- [x] **[130：在 Flurry 里直接嵌一门小语言（DSL）——基于 PEG 语法的构造宏 `@define_macro2`](./130-peg-based-construction-macros-for-custom-dsls.md)** (`docs/macro/construction-macro.md`)
- [x] **[131：原始文本宏的妙用——`@builtin.str.raw { ... }` 与编译期源码生成脚本](./131-raw-strings-and-comptime-source-generation.md)** (`docs/macro/construction-macro.md`, `docs/code-organization/configuration-and-scripts.md`)
- [x] **[132：宏、Comptime 函数、`inline fn` 与 CMTT——四套元编程工具该如何选型？](./132-choosing-between-macros-comptime-inline-and-cmtt.md)** (`docs/cmtt/index.md`, `docs/macro/construction-macro.md`)
- [x] **[133：组合子代数（Combinator Algebra）——让编译器认识并化简你的高阶流管道](./133-combinator-algebra-and-pipeline-simplification.md)** (`docs/program-synthesis/combinator-algebra.md`)
- [x] **[134：张量代数（Tensor Algebra）——从高维算子表达式到分块循环的合成之路](./134-tensor-algebra-and-loop-tiling-synthesis.md)** (`docs/program-synthesis/tensor-algebra.md`)
- [x] **[135：第六卷里程碑（上）：用 PEG 构造宏设计一门迷你数据包过滤语言（Mini-BPF DSL）](./135-milestone-part1-mini-bpf-dsl-with-peg-macros.md)** (综合实战 116–134)
- [x] **[136：第六卷里程碑（下）：用 CMTT 将 Mini-BPF AST 编译为类型安全的零开销原生匹配器](./136-milestone-part2-compiling-mini-bpf-with-cmtt.md)** (综合实战 116–134)
- [x] **[137：用 `template test` 为生成的自动机编写全覆盖性质测试](./137-property-testing-generated-automata-with-templates.md)** (`docs/functions/templates.md`, `docs/cmtt/program-construction.md`)
- [x] **[138：当 CMTT 遇到资源管理——保证生成代码中的仿射值与指针绝不泄漏](./138-resource-safety-inside-cmtt-generated-code.md)** (`docs/cmtt/contexts-and-substitution.md`, `docs/resource-management/index.md`)
- [x] **[139：诊断生成代码中的错误——源码溯源（Span Tracking）与两阶段报错定位](./139-diagnostics-and-span-tracking-in-staged-code.md)** (`docs/cmtt/index.md`, `docs/cmtt/quotation-and-stages.md`)
- [x] **[140：第六卷总结：从“写代码的人”进化为“教编译器写代码的人”](./140-volume-6-retrospective-metaprogramming-as-type-theory.md)**

---

## 第七卷：控制流的大一统——代数效应、限定 Continuation 与结构化并发 (141–170)
*故事主线：从零构建一个支持虚拟时钟测试、协程调度、异步无锁 I/O 与软件事务内存（STM）的微型运行时。*

- [x] **[141：把“做什么”与“怎么做”彻底拆开——代数效应 `effect` 与 `#Effects T` 初探](./141-algebraic-effects-separating-what-from-how.md)** (`docs/algebraic-effect-and-delimited-continuation/index.md`, `docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md`)
- [x] **[142：发起请求与就地拦截——后缀 `#` 传播、`expr# { ... }` 效应消去与 `resume`](./142-effect-propagation-elimination-and-resume.md)** (`docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md`)
- [x] **[143：组织一整套系统能力——`effect enum` 操作组、嵌套处理与未覆盖效应自动转发](./143-effect-enums-nesting-and-forwarding.md)** (`docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md`, `docs/type/qualified-types/effect-type.md`)
- [x] **[144：为什么函数签名里不写 `handles`——基于 Trait 的处理能力（Handling Capability）与 `.use(handler)`](./144-handling-capabilities-and-dot-use-injection.md)** (`docs/algebraic-effect-and-delimited-continuation/handling-capability.md`, `docs/type/handlers.md`)
- [x] **[145：不用切栈也能注入配置与上下文——Direct Passing（直接传值效应）的零开销机制](./145-zero-cost-context-injection-with-direct-passing.md)** (`docs/algebraic-effect-and-delimited-continuation/direct-passing.md`)
- [x] **[146：只跑一次、跑在尾部、还是暂停保存？——尾恢复（Tail-Resumptive）与单次限定 Continuation](./146-tail-resumptive-vs-one-shot-delimited-continuations.md)** (`docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md`)
- [x] **[147：当 Continuation 被中止（Abort）时——跨越挂起栈帧的 `Drop` 逆序清理保证](./147-aborting-continuations-and-guaranteed-drop-cleanup.md)** (`docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md`)
- [x] **[148：用 50 行代码自己写一个生成器（Generator / Iterator）——基于代数效应的按需产出](./148-building-generators-and-iterators-with-effects.md)** (`docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md`)
- [x] **[149：错误 `!Err` 与效应 `#Eff` 的楚河汉界——为什么两者拥有截然不同的 ABI 与组合规则](./149-why-errors-and-effects-have-distinct-abis.md)** (`docs/algebraic-effect-and-delimited-continuation/error-handling.md`, `docs/algebraic-effect-and-delimited-continuation/effect-abi.md`)
- [x] **[150：当 `#Effects !Errors T` 组合在一起——限定前缀的嵌套顺序与交互法则](./150-combining-effect-and-error-qualified-types.md)** (`docs/grammar/index.md`, `docs/type/qualified-types/effect-qualified-type.md`)
- [x] **[151：告别“红蓝函数染色”——`async effect`、`async { ... }` 块与 `.await` 的效应本质](./151-async-effects-without-function-coloring.md)** (`docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md`)
- [x] **[152：绝不留下孤儿任务——结构化并发（Structured Concurrency）、取消传播与作用域退出](./152-structured-concurrency-and-cancellation-safety.md)** (`docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md`)
- [x] **[153：把真实睡眠替换成瞬间快进——用效应 Handler 为异步网络协议编写确定性仿真测试](./153-deterministic-simulation-testing-by-swapping-handlers.md)** (`docs/algebraic-effect-and-delimited-continuation/handling-capability.md`)
- [x] **[154：深入效应 ABI 的发动机舱——证据传递（Evidence Passing）、池化栈帧与跨 C 边界限制](./154-under-the-hood-of-effect-abi-and-evidence-passing.md)** (`docs/algebraic-effect-and-delimited-continuation/effect-abi.md`)
- [x] **[155：告别死锁与锁顺序难题——`atomic(vars) { ... }` 软件事务内存（STM）入门](./155-software-transactional-memory-atomic-blocks.md)** (`docs/concurrency/software-transactional-memory.md`)
- [x] **[156：为什么事务里不能直接发网络包——STM 内部的纯度、重试（Retry）与效应隔离契约](./156-purity-and-effect-isolation-inside-stm-transactions.md)** (`docs/concurrency/software-transactional-memory.md`)
- [x] **[157：在多提示符（Multi-Prompt）栈上精准导航——嵌套 Handler 的路由与标签隔离](./157-multi-prompt-stacks-and-scoped-handler-routing.md)** (`docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md`, `docs/type/rows.md`)
- [x] **[158：当闭包捕获了效应 Handler——`%c` 环境限定、可达性与效应能力的安全传递](./158-capturing-effect-handlers-in-closures-safely.md)** (`docs/algebraic-effect-and-delimited-continuation/handling-capability.md`, `docs/functions/closure-environments.md`)
- [x] **[159：第七卷里程碑（上）：从零实现一个支持协程挂起、超时取消与协作调度的异步执行器](./159-milestone-part1-custom-async-executor-with-effects.md)** (综合实战 141–158)
- [x] **[160：第七卷里程碑（下）：在自定义执行器上运行高并发传感器聚合服务并进行故障注入测试](./160-milestone-part2-fault-injection-on-sensor-service.md)** (综合实战 141–158)
- [x] **[161–170：代数效应进阶专题（10 篇）**：
  - [x] **[161：用效应实现可回滚的解析器回溯（Parser Backtracking）](./161-parser-backtracking-with-effects.md)**
  - [x] **[162：用 Direct Passing 实现零全局变量的结构化日志与分布式追踪（Tracing）](./162-structured-logging-and-tracing-with-direct-passing.md)**
  - [x] **[163：`inline effect`——在编译期完全内联消去的零开销效应协议](./163-inline-effects-for-zero-overhead-abstractions.md)**
  - [x] **[164：异步流（Async Streams）与背压（Backpressure）的效应建模](./164-modeling-async-streams-and-backpressure.md)**
  - [x] **[165：资源池租借（Arena / Pool Lease）与 Handler 作用域生命周期绑定](./165-binding-arena-leases-to-handler-scopes.md)**
  - [x] **[166：当 `resume` 附带 `while` 守卫——条件恢复与状态机驱动](./166-conditional-resume-with-while-guards.md)**
  - [x] **[167：跨模块与跨包边界的效应隐藏与接口封装](./167-encapsulating-effects-across-module-boundaries.md)**
  - [x] **[168：用 `verified` 契约验证效应 Handler 的状态不变量](./168-verifying-effect-handler-invariants.md)**
  - [x] **[169：对比 C++20 Coroutines、Rust Async 与 Flurry 代数效应的底层开销](./169-comparing-coroutines-async-and-flurry-effects.md)**
  - [x] **[170：第七卷总结：控制流不再是语言写死的黑盒](./170-volume-7-retrospective-unified-control-flow.md)**

---

## 第八卷：打通异构硬件与工程架构——流水线投影、高级类型工程与综合实战 (171–200)
*故事主线：把张量与信号处理内核投影到 GPU/SIMD 硬件管线，掌握复杂工程中的类型推理、编译期求值与性能优化法则，并完成端到端异构信号处理系统实战。*

- [x] **[171：一份源码，跨越 CPU 与加速器——流水线投影（Pipeline Projection）总览](./171-pipeline-projection-overview.md)** (`docs/pipeline-projection/index.md`)
- [x] **[172：声明一个 GPU / 设备 Kernel——投影属性、先行类型接口与主机/设备参数契约](./172-declaring-projected-kernels-and-type-interfaces.md)** (`docs/pipeline-projection/declarations-and-interfaces.md`)
- [x] **[173：直接调用底层硬件指令——`expr'intrinsic(args).as(T)` 与输出类型安全契约](./173-hardware-intrinsics-and-typed-outputs.md)** (`docs/pipeline-projection/intrinsics.md`)
- [x] **[174：为什么 Intrinsic 省略 `.as(T)` 默认是 `void`——拒绝隐式返回类型猜测](./174-why-intrinsics-require-explicit-as-output.md)** (`docs/pipeline-projection/intrinsics.md`)
- [x] **[175：把 SIMD 向量数据路径关进安全笼子——目标约束检查、Helper 闭包与内联汇编封装](./175-target-validation-simd-datapaths-and-assembly.md)** (`docs/pipeline-projection/target-validation-and-assembly.md`)
- [x] **[176：宿主后端与设备产物交付——Pipeline 回调、受控句柄与缓存恢复](./176-host-backends-iir-handles-and-artifact-delivery.md)** (`docs/pipeline-projection/backends-and-artifacts.md`)
- [x] **[177：管理大型多包工程——Package、Umbrella 聚合包与最小可见性边界](./177-packages-umbrellas-and-architecture-boundaries.md)** (`docs/code-organization/packages-and-umbrella.md`)
- [x] **[178：Anar 构建调度与编译期脚本——声明式依赖、宏准备与确定性构建缓存](./178-anar-build-scheduling-and-comptime-scripts.md)** (`docs/code-organization/configuration-and-scripts.md`)
- [x] **[179：从源码到运行——编译期与运行时的阶段边界全景](./179-inside-the-compiler-pipeline-phases-overview.md)** (`docs/comptime/index.md`, `docs/comptime/normalization-and-instantiation.md`)
- [x] **[180：属性修饰、代码块边界与后缀视图的语法组合法则](./180-parsing-outer-boundaries-and-hir-lowering.md)** (`docs/comptime/attribute.md`, `docs/others/view-taking.md`)
- [x] **[181：编写自解释的领域模型——结构化调试输出与反射格式化实战](./181-why-all-flurry-irs-dump-as-lisp-s-expressions.md)** (`docs/type/reflection-and-layout.md`, `docs/traits/derivation.md`)
- [x] **[182：多参数协同推导——参数间类型信息传递与高阶回调签名设计](./182-elaboration-sibling-parameters-and-call-plans.md)** (`docs/functions/parameter-elaboration.md`, `docs/functions/parameters.md`)
- [x] **[183：驾驭局部类型推导——跨语句信息流、方法链与显式类型锚点](./183-local-dependencies-wait-tickets-and-wakeups.md)** (`docs/type/receiver-adaptation.md`, `docs/functions/parameter-elaboration.md`)
- [x] **[184：多态接口的清晰消歧——Trait 约束设计、候选唯一定位与视角转换](./184-candidate-solving-isolated-probing-and-commit.md)** (`docs/traits/implementation-resolution.md`, `docs/type/receiver-adaptation.md`)
- [x] **[185：编译期计算与类型检查的协同——纯度契约、求值时机与跨模块确定性](./185-interleaving-elaboration-comptime-and-freezing.md)** (`docs/comptime/purity-and-normal-form.md`, `docs/comptime/normalization-and-instantiation.md`)
- [x] **[186：运行时依赖类型工程指南——动态边界封装、切片桥接与函数间传递](./186-representing-runtime-dependent-types-in-the-compiler.md)** (`docs/type/runtime-dependent-type.md`, `docs/resource-management/contracts-and-verification.md`)
- [x] **[187：构建可快速迭代的大型代码库——接口解耦、编译期计算边界与模块组织](./187-fine-grained-incremental-queries-and-cycle-recovery.md)** (`docs/code-organization/packages-and-umbrella.md`, `docs/code-organization/visibility.md`)
- [x] **[188：零成本抽象的工程落点——高阶控制流、局部可变性与可预测的性能模型](./188-why-iir-preserves-high-level-structure-before-codegen.md)** (`docs/resource-management/runtime-purity.md`, `docs/functions/inline-and-quote.md`)
- [x] **[189：高可靠位级协议工程——`BitVec` 代数、双向编解码与原位字段更新](./189-implementing-verified-bitvec-synthesis-with-fallback.md)** (`docs/program-synthesis/bitvec-algebra.md`, `docs/control-flow/pattern-matching/bitvec.md`)
- [x] **[190–200：终极综合实战：用 Flurry 构建一个端到端的高性能异构流式信号处理系统（11 篇）**：
  - [x] **[190：系统架构设计：从硬件传感器采集、零拷贝缓冲到 SIMD/GPU 滤波内核](./190-capstone-architecture-heterogeneous-signal-system.md)**
  - [x] **[191：第一层（协议层）：用定宽 `enum` 与跨元素 `BitVec` 模式实现零拷贝帧解码](./191-capstone-layer1-zero-copy-frame-decoder.md)**
  - [x] **[192：第二层（内存层）：用可达性类型 `&reachability` 与内存视图构建无锁分片环形队列](./192-capstone-layer2-reachability-safe-sharded-queue.md)**
  - [x] **[193：第三层（验证层）：用 `verified fn` 与 `outcomes` 证明队列并发读写的分离性与无溢出](./193-capstone-layer3-verifying-queue-safety-and-bounds.md)**
  - [x] **[194：第四层（元编程层）：用 2LTT 与 CMTT 在编译期根据滤波器阶数生成展开的卷积代码](./194-capstone-layer4-cmtt-generated-fir-filter.md)**
  - [x] **[195：第五层（硬件加速层）：用 Pipeline Projection 与 Intrinsic 将卷积内核投射到向量指令集](./195-capstone-layer5-projecting-filter-kernels-to-simd.md)**
  - [x] **[196：第六层（控制与效应层）：用代数效应 `#Async`、`!Err` 与前后置 `catch` 编排容错流水线](./196-capstone-layer6-fault-tolerant-effect-pipeline.md)**
  - [x] **[197：第七层（状态协同层）：用 `atomic` STM 块安全汇总多通道实时统计指标](./197-capstone-layer7-stm-metrics-aggregation.md)**
  - [x] **[198：第八层（插件与配置层）：用 `derive fn`、`dyn Trait` 与扩展调用实现声明式配置热加载](./198-capstone-layer8-declarative-config-and-plugins.md)**
  - [x] **[199：第九层（全链路仿真验收）：用 `template test` 与虚拟效应 Handler 跑通百万次确定性故障注入](./199-capstone-layer9-deterministic-chaos-testing.md)**
  - [x] **[200：大结局：回望 200 篇旅程——当正确性、性能与抽象在类型论中合而为一](./200-grand-finale-correctness-performance-and-abstraction.md)**
