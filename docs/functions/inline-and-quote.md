# Inline 与 quote fn

## Intro

有时调用者希望把被调用计算展开到当前位置。普通内联是代码生成的选择；quote fn 则把代码接入作为应用必须完成的工作。

展开仍是一种有参数接口的计算。普通实参先取得一次，再供主体使用，不把它们按出现次数复制成多次执行。

## 语法

~~~text
inline_function -> inline fn id(parameter*) (-> result)? clauses? (block | (= expr))
quote_function -> quote fn id(parameter*) (-> result)? clauses? (block | (= expr))
implicit_quote_parameter -> implicit quote parameter
application -> expr(argument*)
~~~

quote fn 的普通调用沿用参数语法；contextual 输入使用 implicit quote。完整的代码值、单洞和替换规则见 [CMTT](../cmtt/index.md)。普通值跨阶段的 Splice 机制见[跨阶段值传递](../comptime/quoting-and-splicing.md)。

## 正式定义

### 三种不同工作

| 构造 | 必须完成的工作 | 运行时主体 |
|---|---|---|
| 普通／inline fn | 按接口调用；inline 提供内联意向 | 可保留合法调用，优化须保持行为 |
| comptime fn | 在编译时计算结果 | 不能留下运行时补算调用 |
| quote fn | 参数化代码接入必须完成 | 接入后执行主体中的 runtime 操作 |

quote fn 不属于 comptime fn。所有 quoted code 是 comptime 值，描述的代码仍可以是 runtime 计算。返回代码描述的 comptime 函数与保证展开的 quote fn 有不同的应用规则。

`inline if` 等控制流构造还承担实例化时选择代码的工作。一个普通 fn 可以含 inline if，却仍保留普通调用；一个 quote fn 也可以接入包含普通 runtime if 的代码，其分支要等 runtime 条件。函数展开、编译时分支选择和运行时求值是三件事。

当前实现已将 `inline fn` 的声明意向从解析、HIR 保留到 NIR 和具体 IIR 实例，dump 中以 `(inline true)` 表示。泛型实例、带 comptime 实参的实例、延迟签名实例和已选择的 trait 方法保留各自定义的意向；`pure` 和 comptime 阶段仍分别检查。默认执行路径保留普通调用。显式 NIR 优化入口 `elaboration::program::inlining::expand` 可展开普通 runtime 直接调用，复用卫生重命名、默认参数、契约和返回边界降码；同一 pass 继续展开复制主体中的非递归 inline 调用链；当前展开链重复出现同一声明、达到深度界限或预算耗尽时，保留剩余普通调用。候选展开失败会撤销该候选的代码与元数据修改，保留普通调用。调用者在转换之后重新运行资源验证与清理计划发布。命令行可用 `--interpret entry --inline-functions --copy-protocol Copy --drop-protocol Drop` 显式启用此转换，其中协议名按源程序定义指定。驱动先展开，再检查资源并发布清理计划，最后执行；准备失败在执行前报告，runtime 失败不重跑原程序。此选项不用于无资源检查的解释执行。带 comptime 实参的调用等待输入完成，具体 trait 方法通过受检实现选择取得 inline 方法体，并合并接口的程序契约后展开；在实现的参数、结果和入口快照绑定下，已经继承的完全相同谓词不重复插入；实现放宽入口条件时，也识别以入口快照为守卫的已继承保证，实现自行声明的不同谓词仍分别检查；同一轮转换按接口、成员、Self、接口实参及方法实参的完整表示复用选择结果，不展开 opaque head 来制造缓存命中。尚含开放类型输入的选择与间接调用保留原调用。泛型模板的类型实参完成后，展开候选还会按已检查的实现规范化签名、主体、默认值和契约中的关联类型注解；例如 `emit<T.as(Producer).Item>` 在 Item 选定为 bool 后与调用处的 `emit<bool>` 对齐。这一步不调用类型函数，不展开 normal-form 头，也不把仍开放的投影当作已完成实例。callee 展开之后，可将封闭 direct inline effect 的请求和 handler 分支一并降为局部 pattern match，保留参数单次求值、分支顺序及局部清理。显式 resume 使用请求私有的结果交付边界，交付后回到请求点；请求参数自身的 return 留在边界外。handler 的 return 保持原函数目标，break/continue 保持安装位置的词法循环目标，跨过展开 callee 时仍执行相应清理。handler 局部循环使用每份展开代码的新身份；其中已展开的 inline/quote 调用连同结果槽清理证据一起复制。不含效应的闭包保留自身调用与 return 边界，捕获身份随绑定重新建立；拥有型捕获和指针来源在转换后的资源检查中核验。独立封闭的嵌套安装先展开内部；callee 的效应行可以是当前行的子集或采用不同顺序，逐层按槽映射核对完整操作身份。直接请求外层 effect 的分支可以继续展开：先按安装位置的外层行规范化 handler，再把主体中的外层槽位映射回该行。向外请求同一操作不会重新进入当前 handler；多层 map/filter 风格的安装可逐层消除。部分 direct handler 也可展开：原候选之后生成最后一个外层请求分支，按已经检查的 fallback 槽位转发原载荷；匹配或守卫失败的候选先撤销，实参不重新求值。外层返回后，后续请求仍经过原内层候选。带效应的闭包及仍未展开的 effectful callee 保留相应安装的普通执行路径。更一般的联合控制流转换仍未完成。

展开主体没有当前函数的词法 `return` 时，尾表达式直接交付结果，保留普通块作用域和清理；没有后置条件时，尾位置的显式 `return`（包括尾位置 if/match 分支）也可直接交付；跨越后续语句的早退仍需要私有返回值存储及退出边界。带后置条件的显式返回保留边界，确保条件检查发生在局部清理之前。闭包主体的 `return` 属于闭包自身，捕获初始化表达式仍属于外层。这个区别也使仅含标量观察的尾表达式函数可以在模式 guard 中展开，默认实参仍只求值一次。跨越后续语句的早退通过生成的局部返回槽完成交付。guard 验证检查该区域的局部绑定、循环目标和每个实际调用的观察契约，再用普通资源分析发布清理计划；它不因此获得修改调用者变量、写入指针或转移调用者资源的权限。普通观察块和生成返回区域共用检查，允许布局不含数据指针、且固定的 Drop 选择已证明不会拒绝的拥有型局部值。责任取得时检查这个前提，实际退出仍使用普通清理计划，包括嵌套字段、枚举分支、替换旧值和提前退出。调用者拥有型输入的交付、带环境闭包和含指针的拥有型局部值仍待接入。已知恒真的前后置条件保留检查位置；可能拒绝的条件仍需要尚未接入的失败路径资源转移。

```flurry
inline fn accepts(value: bool) -> bool {
    if value { return true; }
    false
}

fn select(input: bool) -> bool {
    input match {
        value if accepts(value) => true,
        _ => false,
    }
}
```

启用普通函数展开时，`accepts` 的早退只结束这次调用；guard 为 false 时继续下一候选。返回槽及参数绑定均为展开产生的新身份。默认参数和返回代码指针也按同一求值与交付顺序处理。展开函数内部的局部计算模式、带 guard 的绑定、`or` 导出绑定与 `let-else` 也使用普通匹配及清理计划。例如可以将上例中的 `accepts` 替换为：

```flurry
inline fn accepts(value: bool) -> bool {
    let ready if ready = value else { return false; };
    ready
}
```

这里的 `return` 只结束展开的 `accepts` 调用。成功候选导出的标量或 Copy 代码绑定可以继续参与局部计算；`var` 绑定可在本区域内更新。重命名来源必须是本候选实际导出的绑定，目标不能覆盖调用者现有变量。

例如，下面的局部对象在每次 `accepts` 调用结束时清理；guard 失败后尝试下一候选，也不能丢掉本次清理：

```flurry
struct Scratch {}
impl Drop for Scratch {
    fn drop(*self) {}
}

inline fn accepts(value: bool) -> bool {
    let scratch = Scratch {};
    if value { return true; }
    false
}
```

相同局部责任也可以直接写在 guard 的 `do` 块中，不要求生成私有返回槽：

```flurry
fn select(input: bool) -> bool {
    input match {
        value if do {
            let scratch = Scratch {};
            value
        } => true,
        _ => false,
    }
}
```

观察区内对象的 Drop 也可以创建和清理自己的局部对象。验证沿本次选中的 Drop 实现继续检查这些责任，泛型调用使用实际类型实参；某个实例的安全结论不能代替另一个实例的依据。递归清理依赖或分析预算耗尽时，当前观察检查不会通过；不能把循环依赖当作不会拒绝的证明。这仍属于程序与资源验证，不进入 PTC 类型命题求解。

局部对象也可以由已知工厂调用取得。调用本身和返回对象的清理都需要不会拒绝的证据。受检观察区内的调用可以接收标量、满足 Copy 的代码值，以及区内新建的无指针拥有型值；后者可以来自临时构造、工厂结果或局部字段。不能借调用消耗观察区外原有的所有者。

```flurry
fn make_scratch() -> Scratch { Scratch {} }

fn select_from_factory(input: bool) -> bool {
    input match {
        value if do {
            let scratch = make_scratch();
            value
        } => true,
        _ => false,
    }
}
```

工厂结果可以先绑定、投影字段或用于替换已有局部值。替换仍先求出新值，再清理旧值；候选失败时先清理本次取得的结果，再尝试下一候选。展开工厂调用也必须保留这些次序。具有完整候选依据的 Copy 函数指针可以作为工厂目标，未知目标不会因返回类型满足布局要求而获准调用。

辅助函数接收拥有型实参时，责任按普通调用规则交付一次。默认实参可以读取已准备参数的标量字段，不能消耗仍须交付给 callee 的参数；默认表达式本身仍须满足稳定性和终止性要求。

```flurry
fn inspect_scratch(scratch: Scratch, .selected: bool = true) -> bool {
    selected
}

fn select_with_helper(input: bool) -> bool {
    input match {
        value if do {
            let scratch = make_scratch();
            inspect_scratch(scratch, .selected = value)
        } => true,
        _ => false,
    }
}
```

这里 scratch 的责任转交给 inspect_scratch，并在该次调用退出时清理。guard 得到 false 时不会再次清理它；局部值一旦移动，后续读取或再次传参仍属错误。对象中的代码字段保留具体来源，不能仅凭函数类型假定未知回调不会拒绝。

泛型工厂按本次实例的实际类型检查输入和清理，例如：

```flurry
struct Box where T { value: T }
fn wrap(value: T) -> Box<T> where T { Box<T> { value: value } }

fn select_generic(input: bool) -> bool {
    input match {
        value if do {
            let boxed = wrap(value);
            boxed.value
        } => true,
        _ => false,
    }
}
```

拒绝分析的缓存身份包含声明和完整类型实参。即使两个实例有相同字段布局，它们也可能选择不同的 Drop，不能仅按声明复用结论。嵌套泛型调用逐层替换实参，normal-form 头仍保持身份；替换和递归分析受预算约束，耗尽时不发布成功证据。缓存分析结论不会减少工厂的实际调用次数。

这里空析构可以取得不会拒绝的证据。若改为可能失败的检查，观察验证必须保留拒绝义务，不能因为 `scratch` 没有被显式读取就删除它。

观察中的辅助调用也可以正常完成并返回 `void`，包括已知代码指针的间接调用、默认实参和局部绑定。省略返回类型不表示调用没有副作用：实际被调用主体仍须满足观察契约，可能拒绝或来源未知的调用仍不能进入这条路径。匹配候选失败后，已执行的辅助调用不会被撤销；下一候选再次调用时是另一次执行。展开前后必须保持这个次数和顺序。

观察区域检查、拒绝分析和内存摘要共用有限的控制流事实：常量条件选定的分支、布尔短路以及必定退出后的块后缀按实际可达性处理。返回表达式本身和已求值的默认实参仍接受完整检查；未知条件下可能继续执行的路径保留其读取、写入与拒绝义务。


### 实参只执行一次

普通 runtime 实参遵守调用顺序，各求值一次，并建立新鲜形参绑定。主体多次引用形参不重算实参；主体不引用某个形参也不自动省掉该实参的效果。

新鲜的是绑定身份，不是指针目标的身份。替换参数时，仍要保持实参之间已有的依赖、别名和资源关系。

普通参数与 contextual 输入不能互换解释：普通参数取得本次调用交付的值；contextual 输入对应生成代码要使用的绑定。代码接入复制的是受检代码结构，不是任意复制捕获资源或 pointee。

接入必须保持函数接口所要求的结果、控制流和清理。展开不赋予局部地址更长生命，也不取消错误传播或退出责任。

### Contextual 输入

`implicit quote` 将输入作为 runtime contextual 变量。它通过合法的上下文对应接入；普通用途无需在每次 splice 手写 substitute。对应仍须保留绑定身份、类型与依赖，不能只凭同名就任取一个对象。

显式命名供应也须交付正确的符号或绑定，不能把读取到的内容冒充 contextual 变量。编译时操作描述访问，真正的 runtime 读取仍发生在生成程序中。

### 代码接入边界

CMTT 代码采用 rank-1、两阶段和 runtime-only 上下文。每段 quote 至多一个 splice；quote 中的 quote fn 应用计作一个隐式接入点。普通 runtime 函数体并不自动是一段 quote，可以包含多个独立 quote fn 应用。

代码经由 Any/any、容器或别名传递，仍保留原有类别、上下文和接入限制。接口尚未恢复时不能假定其可以作为普通 runtime 函数指针调用。

quote fn 必须展开，不保证任意生成过程都终止。无法完成必须的接入或耗尽编译资源时，编译失败；不能悄悄退回一条普通 quote fn runtime 调用。主体中对普通 fn 的调用则可正常保留。

| 位置 | 接入点如何计算 |
|---|---|
| 一段 quote 中无 splice 或 quote fn 应用 | 零洞，允许 |
| 一段 quote 中一个显式 splice | 一个接入点 |
| 一段 quote 中一个 quote fn 应用 | 一个隐式接入点 |
| 同一 quote 的两个 runtime 分支各有一次接入 | 两个接入点，不能以分支互斥豁免 |
| 普通 runtime 函数体中两次独立 quote fn 应用 | 不把整个函数体视为一个 quote；分别检查接入 |

expr 代码使用上下文；pattern 代码在成功匹配后提供绑定；arm 还保存模式与成功主体的对应。quote fn 与[函数级模式](pattern-functions.md)提供 arm 的表面载体，不能把三个类别都作为普通无环境函数值接入。

## 常见示例

### 内联意向与必须接入

~~~flurry
inline fn smaller(left: u32, right: u32) -> u32 {
    if left < right {
        left
    } else {
        right
    }
}

quote fn smaller_code(left: u32, right: u32) -> u32 {
    if left < right {
        left
    } else {
        right
    }
}

test {
    println("{} {}", smaller(8, 3), smaller_code(8, 3));
}
~~~

输出 `3 3`。两者的数值行为一致，但前者提供内联意向，后者的代码接入必须完成。展开 smaller_code 不要求在 comptime 得到 left、right 的值；生成的比较仍可以是 runtime 比较。

### 重复使用参数不重复计算实参

~~~flurry
quote fn twice(value: u32) -> u32 {
    value + value
}

fn produce() -> u32 {
    println("produce");
    21
}

test {
    println("{}", twice(produce()));
}
~~~

依次输出 `produce`、`42`。其参数交付行为可由下面的独立测试表示：

~~~flurry
test {
    let result = do {
        let argument = produce();
        argument + argument
    }
    println("{}", result);
}
~~~

两个测试分别展示同一种行为。实际接入使用新鲜绑定，不能捕获调用者碰巧同名的 argument。

### 没使用的普通实参仍执行

~~~flurry
quote fn constant(value: u32) -> u32 {
    5
}

test {
    println("{}", constant(produce()));
}
~~~

输出 `produce`、`5`。把函数体展开后直接删掉 produce，会改变源程序行为。

### Contextual 变量保留实际对象

~~~flurry
struct Context {
    base: u32,
}

quote fn shifted(value: u32, implicit quote context: *Context) -> u32 {
    value + context.base.*
}

test {
    let state = Context { base: 10 }
    let context = state.ref;
    let first = shifted(1, .context = context);
    state.base = 20;
    let second = shifted(1, .context = context);
    println("{} {}", first, second);
}
~~~

输出 `11 21`。上下文接入保存同一对象的对应；它没有在 comptime 读取或冻结 base。显式 `.context` 展示供应来源，省略时由 implicit substitution 提供合法对应。

### 从模板的已知上下文自动接入

~~~flurry
quote fn read_current(implicit quote current: *u32) -> u32 {
    current.*
}

template fn with_current(
    value: u32,
    quote lambda body: < current: *u32 |- u32 >,
) -> u32 {
    let stored = value;
    let current = stored.ref;
    ~<body>
}

fn selected_value use with_current {
    read_current()
}

test {
    println("{}", selected_value(7));
}
~~~

输出 `7`。body 的接口已经提供 current，quote fn 在该 contextual 对应下接入，调用处无需再次手写替换表。stored 是本次调用建立的 runtime 对象；read_current 的代码接入不会在 comptime 读取它。详细模板流程见[正文复用](templates.md#基于-cmtt-的正文复用)。

### 新鲜参数绑定保留真实别名

~~~flurry
quote fn store_then_read(target: *u32, source: *u32) -> u32 {
    target.* = 9;
    source.*
}

test {
    let value: u32 = 3;
    let result = store_then_read(value.ref, value.ref);
    println("{} {}", result, value);
}
~~~

输出 `9 9`。target、source 是不同形参绑定，却指向同一对象。展开不能把它们看成两个独立存储，也不能把 source 的读取提前到写入之前。

### 普通函数体中的多个接入

~~~flurry
fn combine(left: u32, right: u32) -> u32 {
    let first = twice(left);
    let second = twice(right);
    first + second
}

test {
    println("{}", combine(2, 3));
}
~~~

输出 `10`。combine 是普通 runtime 主体，不因使用 quote fn 就受“一段 quote 只有一个接入点”的源码计数约束。

### 代码展开仍保留局部清理

~~~flurry
struct Trace {
    id: u32,
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

quote fn observed(value: u32) -> u32 {
    let trace = Trace { id: value }
    value
}

test {
    let result = observed(7);
    println("result {}", result);
}
~~~

输出 `drop 7`、`result 7`。trace 属于本次接入主体的语义作用域。不能把它的清理推迟到调用者整个测试退出，也不能因为返回值不读取 trace 就删除有输出的 Drop。

## 与 Rust / Zig / C / C++ 的关键差异

C++ inline 不要求实现必须以调用展开替代普通调用；Zig inline fn 将内联要求纳入函数调用处理。Flurry 的 quote fn 还保留代码上下文与阶段接口，不能只等同于优化提示或 token 宏。[C++ inline](https://eel.is/c++draft/dcl.inline)、[Zig inline functions](https://ziglang.org/documentation/master/#inline-fn)

C 的函数式宏可能重复展开实参文本；保持求值次数要由宏作者另行保证。quote fn 的普通实参则按调用规则取得一次。[C11 §6.10.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)

## 运行时实现设计引导

生成程序中先按源码顺序计算普通实参并准备责任，全部准备完成后，再按形参声明顺序将责任交付给接入主体的新鲜参数槽。默认实参读取准备帧中的先前输入。参数槽可以经优化消除，但实参求值次数、身份及准备与进入主体的边界必须在分析中保持。

准备阶段失败时，只清理已取得的输入和调用者临时对象；进入主体后失败时，先清理 callee 的局部对象及参数，再清理调用者的实参临时对象。不能因为展开后都表现为局部绑定，就把这两层责任混在同一取得顺序里。命名实参的书写顺序影响求值顺序，不改变形参进入 callee 后的清理顺序。

实参的内部临时对象也保留原调用的表达式生命周期。例如将工厂结果的标量字段传给函数时，取得字段不意味着立即清理其父对象：后续实参和 callee 仍按普通调用顺序执行，临时父对象随后在所属表达式边界清理。展开时不能把每个参数绑定改成独立的完整表达式，从而把 Drop 提前到后续实参之前。当前展开用 CallPreparation 保存这层关系；生成的绑定拥有独立位置，避免与原实参的投影、临时值清理位置发生冲突。模式候选失败后，仍先完成本次观察的清理，再尝试下一候选。

contextual 输入可以直接对应已有位置，不要求为每个名字复制一个 runtime 对象。目标存储的访问资格与内容版本仍按实际执行更新。

对比普通调用的实现，quote fn 可以消去调用指令，却要保留相同的实参准备和逻辑退出：先取得输入，执行接入主体，交付结果，结束该次应用的局部责任，再继续 caller。调用边界被展开不表示语义作用域被合并。

## 编译器实现引导

1. 确定定义类别及本次参数实例，区分普通输入和 contextual 输入。
2. 核验代码接口、阶段与接入点计数；未解析来源保留义务，不先记为零。
3. 为普通实参建立按源码排序的新鲜绑定，保留未使用实参的效果。
4. 取得上下文替换，检查依赖和对象 identity，在绑定层替换主体。
5. 将实际控制流、结果与资源契约交付 NIR 检查，处理退出清理。
6. 确认 quote fn 应用已完成接入，后续优化仅改变合法实现，不补做语义上未完成的展开。

计数针对每份代码来源，不以展开后优化删除了节点为由绕过单洞。接入链的代价由实际生成规模决定；维护展开链、循环检测与预算，报错同时指明应用点和生成来源。

为避免错误的文本替换，至少检查三个不变量：同一普通实参只有一个求值节点；同一原始 runtime 对象在替换后保持 identity；每份局部责任只有一个与实际出口对应的清理路径。对经过 Any/any 的代码，先恢复实际类别、上下文及结果接口，再进行上述检查；相容描述本身不提供一段可展开的代码。
