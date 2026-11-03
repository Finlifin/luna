# Flurry 设计认知：重构的语义基线

本文件整理作者已确认的设计，供实现判断使用，不代替正式语言文档。
本阶段不修改 `docs/`。下文称“目标”的结构是实现方案，不增加语言规则。
基线与当前代码差距见 [架构审计](architecture-audit.md)。

## 1. 先区分语言保证、实现策略和当前能力

| 层次 | 可以决定什么 | 不能据此决定什么 |
| --- | --- | --- |
| 作者确认及正式语义 | 合法程序、身份、阶段、求值顺序、契约 | 某一 crate 的长期存在理由 |
| 重构设计 | 表示、所有权、查询边界、算法、缓存策略 | 改写上述语义以迁就实现 |
| 代码现状 | 已接线的路径、成本、拒绝点 | 用拒绝点反向定义语言 |
| 测试及调查报告 | 暴露违反契约的证据、提供定位线索 | 仅以数量、绿灯或建议替代语义判断 |

早期报告第七章中关于 bidirectional elaborator 的替代建议没有被采纳。
这不否认正式文档中目标类型与已知实参共同提供信息的规则。
重构沿用 [elaboration](../docs/compilation/elaboration.md)、
[局部依赖](../docs/compilation/local-dependencies.md)及
[候选事务](../docs/compilation/candidate-solving.md)，不另造推导语言。

## 2. 类型身份与类型计算

- 普通 comptime 调用执行主体；NF 应用保留声明头及普通 comptime 实参。
  函数返回 `Type` 不自动使每种调用都成为 NF；pure 不授予展开权限。
- `Identity<T>` 的主体即使返回 T，也不因此与 T 相等。
  反射取得结构、布局计算和机器表示相同都不能建立这条等式。
- `typealias` 提供名字，newtype 引入独立身份；不能共用“别名展开”路径。
- NF 输入不设 `TypeList` 特权类别。`List<Type>` 是普通 comptime 类型，
  列表值与其他值按有序依赖签名绑定、检查、冻结和比较。
- `Tuple` 是集中绑定的内置 callable，不能伪造递归源码函数体：

```flurry
pub const Tuple: pure comptime fn<...Es: meta.List<Type>> -> Type = .Tuple'builtin;
```

这段仅表示内置包绑定层。使用处复用具名符号；已有整个列表时优先
`Tuple<.Es = Es>`。元素布局视图不改变头身份，也不授予推断特权。
普通用户工厂必须能使用同样的参数绑定和冻结机制。

依据：[纯性与范式](../docs/comptime/purity-and-normal-form.md)、
[类型身份](../docs/type/type-identity.md)及作者关于 Tuple 的明确修正。

## 3. 不同的“未知”不能折叠

| 对象 | 意义 | 处理方式 |
| --- | --- | --- |
| 定义端刚性参数 | 对全部合法实例成立的参数 | 使用契约，不随意赋值 |
| 调用方 metavariable | 获准求解的局部未知量 | 结构合一及模型求解，候选局部提交 |
| TypeCore 的洞 | 不可变类型模板的接口槽 | 在 TypeRef 端替换，不修改共享 core |
| runtime identity | 已建立的对象依赖 | 保持身份，另证合法取得及存活 |
| Any / any | 不完整静态描述／通配描述 | 恢复实际见证，不能由 solver 任意造值 |
| 合法 residual | 语言允许稍后完成的工作 | 保存稳定绑定、契约、控制依赖 |
| Blocked | 存在生产者但输入未就绪 | 登记有版本的观察，恢复本地工作 |
| Unknown | 本次证明未完成 | 保留原因，不当成假、成功或自动等待 |

Type 的信息可以部分可用。例如已知指针类别不要求 pointee 布局先完成。
但“能观察一个字段”不代表整个值已可执行、可冻结或可交付。

## 4. Elaboration、冻结和两个执行阶段

Normalization 建立定义端必须成立的契约，instantiation 取得本次输入后继续
合法残余工作。两者都可能与 comptime 交织，不是两次不可返回的全包遍历。

检查工作的顺序可以由依赖决定；普通实参的执行顺序、次数和控制依赖必须保留。
不能为获得类型信息预先运行所有分支，不能在重试时重复已经发生的不纯调用。

冻结保存稳定残余及其契约，不保存活动 metavariable、waiter、VM 栈或候选事务。
独立 comptime 函数的 invocation 使用 engine 字节码；混合阶段 interpretation
承担就地解释的职责。两者共享语义，不以执行载体划分两套合法程序。

纯 comptime 是作者声明的可信承诺，不是 runtime P4 核验的别名。
当前结果缓存策略只考虑 pure 且结果为 Type 的 invocation；字节码缓存、
稳定定义查询缓存和结果缓存是三件事。不能为了测出速度默认缓存一切调用。

普通参数默认供应与子句默认值也不能混同：runtime 默认供应要求 P3 稳定观测及
合法入口下有限完成；可选子句默认值按 normalization 规则执行，需要延期时显式 defer。

依据：[编译流水线](../docs/compilation/phases.md)、
[冻结](../docs/compilation/staging-and-freezing.md)、
[normalization](../docs/comptime/normalization-and-instantiation.md)。

## 5. PTC 的能力和职责

PTC 处理类型命题。内存、初始化、资源和程序状态规格交给程序验证层。
“都能写成布尔表达式”不是统一进一个 solver 的理由。

完整 CDCL(T) 除了证明，还为结构 unification 提供模型搜索托底，优先支持
Type sort 的变量。语言总体偏显式，不因 solver 更强而扩大隐式推导位置。
满足模型、可表达 substitution、获准提交的类型解分别验证；需要确定某个输出时，
不必强求整个辅助模型唯一，但不能无依据地选一个类型。

NF 同余、来源头形状、匹配和单射是不同关系。能提出一个可验证的实参候选，
不等于能从两个输出相等推导输入相等。trait 事实要保留实现世界与选择证据；
更精确候选未决时，不能提交较宽实现。

依据：[原子目录](../docs/type/propositional-constraints/atoms.md)、
[理论职责](../docs/type/propositional-constraints/theory-solvers.md)、
[组合协议](../docs/type/propositional-constraints/solver-protocol.md)。

## 6. Runtime 依赖、资源与纯度

类型结构、引用端 substitution、对象环境取得计划三者分开。两个引用共享 core
不表示类型相同；知道对象身份也不表示当前使用点能合法取得对象。
环境可以从共同外层交付，不要求每个值重复携带指针。

`Once` 提供默认仿射能力，`impl Once for T where T {}` 不授予 Copy/Clone。
复制描述符不复制其 backing storage，不延长存活；Drop 的读取和控制行为也是契约。
comptime 的 self 为值引用语义，没有源语言指针，不能借 runtime Copy/Clone 修补。

safe mode 的 memory/effect predicate 是程序规格，不因供编译器使用就加 comptime。
自动算术限制在总次数不超过三的多元多项式片段，并有预算；它不是可解性保证。
超出证明能力时，可执行且满足行为要求的 assumes 由运行检查兑现，不能直接当公理。
verified mode 的高级证明可以后续发展，safe mode 的存活和访问检查不能随之推迟。

P0–P5 按外界行为、有界状态影响、只读、范围内稳定观测、值纯、有限完成逐层增加保证。
仍要保存独立的读取足迹、存储版本、trap/控制退出和完成性证据；单个枚举不能授权所有优化。

依据：[runtime 依赖](../docs/compilation/runtime-dependent-types.md)、
[资源](../docs/resource-management/index.md)、
[纯度](../docs/resource-management/runtime-purity.md)。

## 7. CMTT、模板、生成定义

PCMTT 为 rank-1、两阶段，每段 quote 至多一个 splice；上下文只含 runtime 绑定。
quoted expr、pattern、arm 都是 comptime 值，arm 借 quote fn 与函数级模式匹配表达。
Any/any 不擦除其代码类别、上下文、固定依赖、控制目标和证据。

quote fn 不是 comptime fn；普通实参仍按普通调用规则各求值一次，然后确定性展开。
上下文参数通过属性标记。默认 substitution 按已建立的绑定关联，不按同名或同型猜配。
替换必须同时作用于主体、类型、契约和 return/break/continue 目标。

template 既可以只抽象签名，也可以结合 CMTT 复用代码；不能把当前单体输入限制
升级为语言规则。open code 主要服务大量分散子入口的模块化，不作为普通复用的默认手段。
builder 应覆盖各类 definition，组装、语义检查与原子发布分别承担责任。

依据：[CMTT](../docs/cmtt/index.md)、[模板](../docs/functions/templates.md)。

## 8. 错误、效应、迭代和 ABI

Flurry 没有标准库 Option/Result 枚举；`?T` 与 `!E T` 使用语言规定的类型及 ABI。
错误结果交付与代数效应控制转移不同。错误行的唯一标签集合规则不能用于抹掉 scoped
effect occurrence 的路由和 handler 层次。

Capability 由 trait 协议配合 elaboration 实现，不在签名添加 handles。
ABI 通过属性指定，没有 extern 关键字。inline effect 的目标是 callee 与调用者 handler
共同展开，迭代器采用 inline fn + inline effect；不能只内联 handler 后宣布兑现。
runtime 实例可以封装 stack pool，不要求用户重复指定两者。

依据：[错误和代数效应](../docs/algebraic-effect-and-delimited-continuation/index.md)、
[函数](../docs/functions/index.md)及作者对 runtime/stack pool 的明确补充。

## 9. 包与交付

只保留 luna 一个 CLI 产品。六包为 builtin、meta、build、attrs、std、verification。
math 在 builtin，经 builtin.prelude 导出；另有 comptime 用的 meta.math。
str 在 builtin，String 在 std，内存模块叫 mem。五个非 std 包默认导入自己的 prelude
作为对其他包的普通通配来源，不给提供者注入自身 prelude；再导出按声明身份去重。

原始 builtin 访问仅在六包集中绑定定义处使用。六包可为编译器 builtin 类型写 impl，
但不豁免冲突检查。同一类型两个阶段的方法同名时，分别使用 runtime trait 与
pure comptime trait，comptime 优先后者；pure 在该 trait 修饰中表示 only。

std 容器从一开始保留 allocator 实例及错误契约。作者要求 DefaultAllocator.default
不返回 AllocErr，但其不可恢复失败的具体策略未据此确定；不能擅自补成 abort。
基础 Display/Debug 不依赖拥有型 String，格式化方案和 writer 必须保留 Flurry 的能力、
内存和阶段设计，不照抄其他语言接口。

Pipeline 提供接口和目标产物；投影声明不自动具有 host callable ABI。LLVM 暂沿用既有依赖。
所有 IR 必须有稳定、可读、正确转义的 S-expression Display。

## 10. 对现有文档的阅读纠偏

`docs/` 部分章节也追加了“当前实现”段落。原始 builtin 示例、旧 optional 构造器要求、
局部不支持说明不能覆盖上述作者确认；本阶段仅记录冲突，不改正式文档。
如三次算术的概括性解释暗示片段内全部可判定，应以同章的预算、Unknown 和作者限制为准，
不能据此承诺完整非线性整数判定。教程、历史 proposal 和旧开发成功日志同样不提供新公理。
