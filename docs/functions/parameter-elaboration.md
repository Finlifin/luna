# 参数 elaboration

## Intro

有些参数还规定调用处的代码怎样解释。lambda 标记为简写闭包划出边界，implicit 参数说明一个省略的输入应当由协议供应。

这些规则负责把源码接到参数接口上。接入之后的计算仍遵守普通的类型、阶段和资源要求。

## 语法

~~~text
lambda_param -> lambda parameter
implicit_param -> implicit parameter
quote_param -> quote parameter
catch_param -> catch parameter
post_lambda -> expr do (lambda | block | expr)
named_arg -> .id = expr
lambda -> |anchored_parameter*| (-> expr)? expr
~~~

`tactic lambda` 在带 lambda 标记的实参位置使用 `_0`、`_1` 等下标占位符。下标从 0 起；它们引用该位置对应闭包的参数。

## 正式定义

### Lambda 标记与 post do

post do 优先接入签名中 lambda 标记的参数；没有此标记时，接入最后一个锚定参数。不存在合法目标、存在多个可行目标或目标已经被供应时，拒绝该调用，而不任意覆盖。

路由只决定正文交给哪个位置。实际参数类型仍决定这个正文应形成怎样的闭包或代码值。一个位置能由 post do 接入，不意味着它因此取得 tactic lambda 标记。

lambda 是形参的语法角色，不是一个新的调用阶段。带标记的参数仍可以接收普通函数值或显式 lambda。调用者提供闭包时，创建环境发生在实参准备期间，执行闭包主体发生在接收者实际调用它时。

| 输入写法 | 如何找到边界 | 主体何时执行 |
|---|---|---|
| 普通函数值 | 已有值直接交付目标 | 接收者调用它时 |
| 显式 lambda | 竖线给出参数及主体 | 接收者调用它时 |
| post do | lambda 标记优先，否则最后锚定参数 | 依据目标是闭包还是代码接口 |
| tactic lambda | 当前 lambda 标记建立占位符边界 | 形成闭包后，接收者调用它时 |

### Tactic lambda

带 lambda 标记的位置可以把包含占位符的表达式解释为闭包主体。完整参数表由目标签名决定，表达式中出现的最大下标不决定参数个数。未引用某个参数只是未使用它，不使形参数目缩短。

同一下标多次出现，引用同一个已经取得的参数。若重复使用会重复消耗非 Copy 值，则按资源规则拒绝；不能重新执行外部实参来制造第二份值。

普通子表达式继承当前占位符边界。嵌套的 lambda 标记实参以及显式 lambda 建立新边界；占位符不隐式跨过它们引用外层参数。需要跨层捕获时使用具名的普通 lambda。

没有 lambda 标记的参数，可以接收显式闭包构造，但不能独立启用 tactic lambda。越界占位符或无法确定的目标参数表不能猜成某个可调用类型。

### Implicit 与显式供应

implicit 输入在 elaboration 时由协议供应，可以带 comptime 或 quote。编译时确定供应方式，不代表生成的 runtime 输入计算在编译时执行。

显式 `.name = expr` 直接供应该位置，绕过缺省供应协议，并正常检查输入。`implicit quote` 将输入作为 contextual 绑定处理，见[quote fn](inline-and-quote.md)；不能因为写了 implicit 就自动读取 runtime 对象的内容。

implicit 不表示“在词法作用域任取一个同类型值”。供应需要所选协议或代码接口提供确定的来源；候选缺失、来源歧义和供应结果不满足契约都是失败。显式供应可以消除来源选择，但不能绕过阶段或类型错误。

### 协议的局部边界

同一次调用中，callee 签名、外围期望与各实参的合法类型信息可以共同约束本次推断变量，包括后面的参数帮助前面的 lambda。分支前提、phase 和候选状态仍限制事实的可见范围；这种检查顺序不改变 runtime 实参求值次序。算法见[Elaboration 与上下文检查](../compilation/elaboration.md)。

implicit、变参和拓展变参可以通过 trait 定制 elaboration。每个协议负责自己的参数入口；外层调用统一拥有实参绑定与求值次序。

协议不能隐式重排、重复或吞掉其他实参，不能把普通输入的求值改成可任意重复的源码替换。需要保留未执行代码时，使用明确的 quote 或 lambda 角色。

生成结果必须满足该位置的阶段、类型、责任与契约。名字查找和源码分析可以发生在编译时；实际生成的 runtime 效果仍按调用计划执行一次。

### Catch

catch 参数用于绑定 continuation，使用 builtin.Continuation。它与普通 lambda 参数不是同一种角色。continuation 操作的合法性依赖相应内建操作和效应契约，不能仅凭普通闭包规则推断任意复制、恢复或销毁都安全。

其绑定角色可以在接口中直接看到：

~~~flurry
trait OperationHandler {
    fn handle(*self, value: u32, catch continuation: builtin.Continuation) -> u32;
}
~~~

这是一个接口声明：value 是普通输入，continuation 由对应效应处理入口绑定。它没有定义 continuation 的构造器、普通调用方法或任意多次恢复许可。错误／效应的处理能力由 trait 协议配合 elaboration 接入；完整恢复关系见[handler 契约](../type/handlers.md)。

## 常见示例

### 一个标记，同时服务两种调用写法

~~~flurry
fn compare_pair(first: u32, second: u32, lambda compare: fn(u32, u32) -> bool) -> bool {
    compare(first, second)
}

test {
    let ordered = compare_pair(2, 3, _0 < _1);
    let same_first = compare_pair(2, 3, _0 == _0);
    let explicit = compare_pair(2, 3) do |first, second| first < second;
    println("{} {} {}", ordered, same_first, explicit);
}
~~~

三个结果都是 true。第二个仍是二元闭包，只是没有使用第二参数。第三个由 lambda 标记找到 compare。

### 标记可以位于最后一个锚定参数之前

~~~flurry
fn inspect_with(lambda inspect: fn(u32) -> bool, value: u32, .fallback: bool = false) -> bool {
    if value == 0 {
        fallback
    } else {
        inspect(value)
    }
}

test {
    let accepted = inspect_with(7) do |value| value < 10;
    let absent = inspect_with(0, .fallback = true) do |value| value < 10;
    println("{} {}", accepted, absent);
}
~~~

输出 `true true`。post do 为 inspect 提供输入，位置实参供应 value，命名实参供应 fallback。第二次调用没有执行 inspect 的主体；lambda 的构造与调用仍是两件事。

### 没有标记时仍可使用显式 lambda

~~~flurry
fn apply_plain(value: u32, action: fn(u32) -> u32) -> u32 {
    action(value)
}

test {
    let first = apply_plain(5, |value| value + 1);
    let second = apply_plain(5) do |value| value + 2;
    println("{} {}", first, second);
}
~~~

输出 `6 7`。post do 在这里接到最后一个锚定参数 action。

~~~flurry
-- 拒绝：apply_plain 的 action 没有 lambda 标记。
fn invalid_tactic() -> u32 {
    apply_plain(5, _0 + 1)
}
~~~

### 显式覆盖 implicit 供应

~~~flurry
fn shifted(value: u32, implicit bias: u32) -> u32 {
    value + bias
}

test {
    println("{}", shifted(5, .bias = 3));
}
~~~

输出 `8`。此调用已经提供 bias，无需为 u32 寻找自动供应来源。省略 bias 的调用必须另有合法、可确定的供应依据。

### 转发隐式输入与规定输入阶段

~~~flurry
fn choose_mode(first: u32, second: u32, implicit comptime use_first: bool) -> u32 {
    inline if use_first {
        first
    } else {
        second
    }
}

fn selected(first: u32, second: u32, implicit comptime use_first: bool) -> u32 {
    choose_mode(first, second, .use_first = use_first)
}

test {
    println("{}", selected(4, 9, .use_first = false));
}
~~~

输出 `9`。selected 已取得的输入被显式转发给 choose_mode；这一层没有再次运行缺省供应。use_first 在编译时确定实例，不能把一个未来才知道的 runtime bool 传入这一位置。自动供应会改变来源，不会改变所需阶段。

### 重复占位符不重复外部计算

~~~flurry
fn produce() -> u32 {
    println("produce");
    6
}

fn apply(value: u32, lambda action: fn(u32) -> u32) -> u32 {
    action(value)
}

test {
    println("{}", apply(produce(), _0 + _0));
}
~~~

只输出一次 `produce`，然后输出 `12`。闭包主体读取自己的参数两次。

### 嵌套 tactic lambda 各有自己的参数

接续上面的 apply：

~~~flurry
test {
    let result = apply(3, apply(_0, _0 + _0));
    println("{}", result);
}
~~~

输出 `6`。内层 apply 的第一个实参 `_0` 属于外层闭包；第二个实参位于另一个 lambda 标记位置，其中两次 `_0` 都属于内层闭包。其绑定意义与下面的独立测试相同：

~~~flurry
test {
    let result = apply(3, |outer| apply(outer, |inner| inner + inner));
    println("{}", result);
}
~~~

需要在内层主体同时引用外层参数时，写出 outer、inner 这类具名绑定，并让完整回调类型保留实际环境。占位符下标不承担跨边界查找的工作。

### 目标参数表决定合法下标

接续 compare_pair，以下是独立拒绝例：

~~~flurry
fn invalid_index() -> bool {
    compare_pair(2, 3, _2 == 0)
}
~~~

compare 只有两个参数，合法下标是 0、1。不能因为出现 `_2` 就把目标扩成三元函数，也不能把未引用的前两个参数从参数表移除。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 与 C++ 的 lambda 都有明确的闭包构造语法；这里的 lambda 参数标记进一步提供调用位置上的占位符边界。C 可显式传函数指针，Zig 可显式传函数或由 comptime 组织接口；这些都不是 Flurry 参数协议的同名机制。[Rust closure](https://doc.rust-lang.org/reference/types/closure.html)、[C++ lambda](https://eel.is/c++draft/expr.prim.lambda)、[Zig functions](https://ziglang.org/documentation/master/#Functions)

## 运行时实现设计引导

elaboration 产物应是普通调用、闭包构造或受检代码接入。运行时不遍历参数 trait 的候选列表来决定源表达式怎样解析。

当 implicit 供应生成 runtime 表达式时，把它放进缺省输入计划。它与 optional 按参数依赖次序执行；显式输入在前。供应失败或资源交付失败沿普通调用准备路径清理。

## 编译器实现引导

1. 从已定位定义的粗 HIR 签名读取参数角色，不先要求 lambda 主体已经完成类型检查。
2. 建立实参归属，确定 post do 的唯一目标并检查重复供应。
3. 对 lambda 标记位置建立占位符作用域，按目标签名建立完整参数表。
4. 对主体做绑定解析；越界占位符报错，嵌套边界分别处理。
5. 在候选自己的替换环境中运行参数协议，检查其产物和依赖，失败时回退该候选的暂存信息。
6. 完成阶段、类型和程序验证检查，形成统一调用计划。

占位符转换只替换绑定引用，不复制外部实参树。一次源码遍历可完成边界和占位符登记，成本 O(s)，s 为该表达式大小；协议执行与推断的成本另计。

协议递归或实例化循环须有资源限制与来源链诊断。预算耗尽不是成功供应，也不能被解释为不存在某个 trait。不得在失败后留下半份参数绑定。

供协议与普通调用共同使用的交付记录至少包含：目标参数身份、代码／值类别、输入阶段、runtime 求值位置、结果类型及责任。接收协议产物时重新核对该记录；将一个代码描述错认成已求值实参，或将 runtime 供应提升到 comptime，均为阶段错误。
