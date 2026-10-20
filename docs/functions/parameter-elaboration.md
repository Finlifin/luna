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

开放签名中的 lambda 标记也使用这套边界。调用处先形成 callback 的可调用类型，再按完整参数表检查 tactic lambda：

~~~flurry
typealias Type = .Type'builtin;

comptime fn Select(flag: bool, T: Type) -> Type {
    if flag { T } else { bool }
}

fn apply(comptime flag: bool, lambda action: defer fn(T) -> T, value: T)
    -> defer Select(flag, T)
where T
{
    action(value)
}

fn run() -> bool {
    apply(true, false) do { not _0 }
}
~~~

value 为本次调用提供 T 的依据，action 的类型形成后才检查 `_0`。同一位置仍可接收普通函数值或显式 lambda；嵌套标记建立各自占位符边界。runtime 实参仍只求值一次，并在被调用函数执行前完成准备。目标不明、占位符越界、重复供应和 post do 歧义均拒绝。当前接通的是直接 open 调用的 lambda 路由；implicit 的开放签名缺省协议供应仍未实现。

计算型位置参数包与 post do 可以组合，且 lambda 入口不必在签名末尾：

```flurry
fn apply_packet(comptime flag: bool, lambda action: fn(bool) -> bool, value: bool) -> bool {
    inline if flag { action(value) } else { false }
}
fn forward(comptime flag: bool) -> bool {
    apply_packet(...(flag,), ...do { (false,) }) do { not _0 }
}
```

路由先锚定 action，再确定位置包的范围，因此这里只展开一个 runtime bool。这也适用于显式或推断类型子句的开放调用。包提前退出时不调用 callback，但 callback 的类型、占位符范围和阶段仍须合法；尾随语法不取消 lambda 标记对 tactic lambda 的要求。

### Implicit 与显式供应

implicit 输入在 elaboration 时由协议供应，可以带 comptime 或 quote。编译时确定供应方式，不代表生成的 runtime 输入计算在编译时执行。

显式 `.name = expr` 直接供应该位置，绕过缺省供应协议，并正常检查输入。`implicit quote` 将输入作为 contextual 绑定处理，见[quote fn](inline-and-quote.md)；不能因为写了 implicit 就自动读取 runtime 对象的内容。

开放签名也支持 explicit named supply，包括显式提供 comptime 输入以形成依赖域：

~~~flurry
typealias Type = .Type'builtin;

fn identity(implicit comptime T: Type, value: defer T) -> defer T {
    value
}

fn use_identity() -> bool {
    identity(true, .T = bool)
}
~~~

计算型 tuple 包同样不占用 implicit 槽位，包括尚待调用时供应的 comptime implicit 输入：

```flurry
fn choose_packet(implicit comptime flag: bool, value: bool, implicit bias: bool) -> bool {
    inline if flag { value and not bias } else { false }
}
fn forward_packet(comptime flag: bool) -> bool {
    choose_packet(.flag = flag, ...do { (true,) }, .bias = false)
}
```

显式供应可以写在包前或包后，实际求值按源码顺序。它与 post do 和依赖前序输入的默认值共用调用准备计划；默认值在显式输入之后准备，显式覆盖跳过默认计算。缺少供应仍报告缺少 implicit 输入，不以包中的位置元素或同名局部变量补位。

显式的 runtime implicit 输入按源码实参顺序准备，固定位置输入和元组展开不会占用其命名槽位。后续默认值可以复用已供应输入，但仍须满足默认值的稳定性和资源要求。漏供、重复供应、错误类型以及用 runtime 输入供应 comptime 位置都会拒绝；当前未实现的缺省协议不会退化成同名局部变量查找。

这条显式供应路径也支持开放函数返回 runtime 闭包，再立即调用该闭包的组合。若工厂的参数与结果域已完整建立，定义检查保留工厂的普通调用及阶段信息，不为了取得 callee 而提前完成其 comptime 实参。外层 inline 选择可以继续转发未知的 comptime 输入，同时按名交付 runtime implicit 输入；实际实例化才选择工厂主体。runtime 中仍先执行显式供应表达式，再执行所选工厂，最后准备闭包调用的输入。显式类型子句与计算型参数包也可组合，例如 `factory<T>(flag, ...packet(value))`：T 已完成时直接复用该类型实参，flag 仍可等待调用方供应，packet 在 runtime 执行一次。固定输入和 tuple 变参入口都保留这一规则；连续调用采用不同类型实参时，各次替换独立。尚未建立的依赖签名不被当作已完成签名，自动 implicit 供应协议仍未实现。


implicit 不表示“在词法作用域任取一个同类型值”。供应需要所选协议或代码接口提供确定的来源；候选缺失、来源歧义和供应结果不满足契约都是失败。显式供应可以消除来源选择，但不能绕过阶段或类型错误。

### 协议的局部边界

同一次调用中，callee 签名、外围期望与各实参的合法类型信息可以共同约束本次推断变量，包括后面的参数帮助前面的 lambda。分支前提、phase 和候选状态仍限制事实的可见范围；这种检查顺序不改变 runtime 实参求值次序。算法见[Elaboration 与上下文检查](../compilation/elaboration.md)。

implicit、变参和拓展变参可以通过 trait 定制 elaboration。每个协议负责自己的参数入口；外层调用统一拥有实参绑定与求值次序。

当前默认属性收集协议也可用于开放签名。多个变参入口同时存在时，已知 tuple 域与 record 属性域分别路由；尚未形成的域保留为候选，不为了选择入口而提前求值。不能唯一确定属性入口时报告歧义。冒号属性 `flag: value` 构成属性包，不等同于 `.flag = value` 的命名实参，也不能借此供应 implicit 槽位。属性、位置输入、展开输入与尾随 lambda 共同使用源码顺序的调用准备计划。

开放结果类型不会抹去已知输入域。展开 `do { (1, 2) }` 或条件表达式产生的 tuple 时，已确定的位置参数类型可向包内提供上下文，使目标为 `u8` 的数字按 `u8` 检查，而非先默认成 `i32`。属性入口已由显式类型子句确定时，其余 tuple 入口也复用这份上下文。尚未求解的子句参数或依赖输入不作为已确定类型施加给包表达式；准备阶段不因此提前执行延迟类型计算。越界、类型和元素数量仍正常检查。

定义检查也区分“转发尚未供应的 comptime 输入”和“检查 runtime 参数包”。下面不需要为了检查 middle 的函数体先取得 flag 的值；包的两个元素按目标的 bool 域检查，实际实例化时再选择 select 的分支。outer 经由 middle 传递阶段依赖，不因中间函数使用普通 bool 返回签名而丢失依赖。

```flurry
fn select(comptime flag: bool, first: bool, second: bool) -> bool {
    inline if flag { first } else { second }
}
fn middle(comptime flag: bool) -> bool {
    select(flag, ...do { (true, false) })
}
fn outer(comptime flag: bool) -> bool { middle(flag) }
```

当前这条定义检查路径支持已知参数域、固定位置、optional 及已知 tuple 域变参入口中的 runtime 包；未知 comptime 输入仍作为调用依赖保留，不构造代替它的常量。包 producer 与其他实参按源码顺序执行，包只求值一次；包中的提前 return 会阻止后续 producer 与 callee 执行。返回域仍为 defer 时，定义检查允许 return 并检查其值的操作与阶段，实际实例化后再按完成的返回域检查；这不是从某个 return 提前推导或固定整个函数的返回类型。嵌套闭包仍自行推导返回域，参数默认值仍不允许 return。包本身供应未知 staged 输入，以及完成参数包检查后泛型证据仍不足的组合，尚未由这条路径完整支持。 参数包 producer 确定为 Never 时，整个调用也为 Never：尚未执行的 callee 结果无需匹配外层表达式的结果域。此规则不放宽输入检查，后续实参、展开宽度和字段类型仍须合法；仅部分路径提前返回时，仍检查正常调用结果。固定域调用、函数值、显式泛型调用、属性收集和枚举构造共用这一准备规则。

显式类型子句及其默认值确定后，也可在上述定义检查中使用，不要求先完成转发的 comptime 实参：

```flurry
fn select(comptime flag: bool, first: T, second: T) -> T where T {
    inline if flag { first } else { second }
}
fn choose(comptime flag: bool) -> u8 {
    select<u8>(...(flag,), ...do { (7, 9) })
}
```

这里数字按 u8 检查，flag 仍待调用时供应。 字面包逐元素路由，因此 `(flag,)` 不会先被物化成 runtime tuple；它可与计算型 runtime 包、空包及普通实参混用。只有计算型包的 producer 才需要整体阶段检查，不能借混合展开把 runtime 输入提升为 comptime。显式子句中的类型也可以来自调用方自己的类型变量，trait 约束仍须由调用方提供证据；不同调用的类型替换不互相泄漏。空字面包可以与计算型包混合，包内 inline 选择使用同一元素上下文，仍检查所有分支的操作。

省略显式类型子句时，已知结果上下文可先为包内元素提供类型，随后用准备好的包字段和其它实参完成子句推断。定义检查与实际实例化共用这份上下文推导，避免定义端把数字按 u8 检查、实例化端却重新默认成 i32：

```flurry
fn choose_inferred(comptime flag: bool) -> u8 {
    select(...(flag,), ...do { (7, 9) })
}
```

已知类型的同级 runtime 实参也能提供上下文，无需 callee 的结果类型提到该泛型变量：

```flurry
fn seed() -> u8 { 3 }
fn accept(comptime flag: bool, value: T, callback: fn(T) -> T) -> bool where T {
    inline if flag {
        let ignored = callback(value);
        true
    } else { false }
}
fn with_peer(comptime flag: bool) -> bool {
    accept(flag, seed(), ...do { (|value| value,) })
}
```

这里 seed 的 u8 结果为包内闭包提供参数与返回域。同级实参也可以写在包之后；推断先后不改变 runtime 求值顺序。预检查产生的实参代码进入统一准备计划，实际调用时只执行一次；包提前返回时不执行后续 producer。阶段实参不参与这轮 runtime 预检查，仍按原阶段规则检查与交付。 若输入域是 `T.as(Source).Element`，相关等式可以等待后续实参确定 T；取得主体类型后，参数与返回域使用同普通调用一致的关联类型视图检查。候选完成时仍须解决这些义务，不能仅凭某个 Element 类型反推一个不唯一的主体 T。拥有型同级实参进入准备计划后仍遵循普通移动和清理规则：在包提前退出前已初始化的值清理一次，尚未执行的 producer 不创建资源。

当 callee 返回域仍为 defer 时，返回域不参与这一步上下文推导；包内已有的 bool 等类型仍可用于推断。此过程不调用依赖未知实参的类型计算，也不将 callee 的值绑定索引当作 caller 的类型证据。默认子句、trait 约束及各实参的阶段在子句完成后继续检查。 初步参数上下文推导若与预期结果冲突，可以在不执行源码的情况下去掉结果提示重建候选；这不是接受类型不匹配。参数包完成性确定后，可正常完成的调用仍必须满足结果要求；必然退出的包则不要求交付 callee 的结果。后续实参的类型及 trait 义务仍检查，不能用不可达性绕过声明契约。

显式类型子句也会先为普通开放调用的计算型展开包建立上下文：

~~~flurry
typealias Type = .Type'builtin;
typealias Byte = .u8'builtin;
comptime fn Select(flag: bool, T: Type) -> Type { T }

fn first(comptime flag: bool, a: T, b: U) -> defer Select(flag, T)
    where T, .U: Type = T {
    a
}

fn use_packet() -> Byte {
    first<Byte>(true, ...do { (1, 2) })
}
~~~

这里默认子句 U 也取 Byte，包内两个数字都按 Byte 检查。包中的 runtime producer 仍按源码顺序执行一次；若包提前返回，callee 不执行。一个 comptime 包供应多个 staged 输入时，类型形成从同一份已冻结包值读取各元素，不单独求值临时绑定。包含 runtime 调用的包不能借由混合展开供应 comptime 参数。 即使后面的输入域尚待形成，也保留固定参数的阶段标记；类型未知不等于 runtime。例如下面第一个包供应阶段依赖，第二个包仍可执行 runtime 计算：

~~~flurry
typealias Type = .Type'builtin;
comptime fn Select(flag: bool, T: Type) -> Type { T }
fn produce() -> bool { true }
fn identity(comptime flag: bool, comptime T: Type,
            value: defer Select(flag, T)) -> defer Select(flag, T) {
    value
}
fn separate_packets() -> bool {
    identity(...do { (true, bool) }, ...do { (produce(),) })
}
~~~

阶段检查与包内部分结构上下文分别传递：尚未确定的元素域不会抹去同一个包内其他元素、或同次调用中其他包的已知上下文。已知 `u8` 字段仍按 `u8` 检查，未知字段继续推断；`do` 和条件分支沿用这份上下文。绑定语言定义的 Tuple 时，准备依赖按其协议描述识别字段和生产者，保留原 normal-form 类型身份，不能因为另一个不透明类型形似 tuple 就解构它。收集域尚待推导时，如果只有一个未确定的收集入口、所有展开包都有宽度证据，则扣除固定锚点及其他已知入口后，剩余位置唯一属于该入口。宽度证据只确定路由，实际包字段仍须参与泛型类型推导及最终域检查；不能用猜测的分区代替正式路由。多个未确定入口或缺少包宽度证据的情况仍需进一步协议支持。

```flurry
fn collect(comptime flag: bool, ...items: Args, tail: bool) -> Args where Args {
    items
}
fn forward(comptime flag: bool) -> (bool, bool) {
    collect(flag, ...do { (true, false) }, true)
}
```

这里 `flag` 和 `tail` 是固定锚点，包中两个字段构成 `Args` 的收集值。即使被调函数返回 `bool` 而非 `Args`，正常完成的包仍可独立提供整个收集域的类型证据。必定提前退出的 producer 为 `Never`，没有完成字段类型；此时必须从其他输入或结果上下文取得域证据，不能通过不可达的尾部表达式虚构类型。后续实参提供静态证据并不意味着提前执行它。

已知 tuple 域的收集入口与普通锚点共用这条展开路径。收集到同一个参数的元素可以被具名实参隔开；求值次序仍按源码排列，不按最终参数归组：

```flurry
fn first() -> bool { true }
fn last() -> bool { false }
fn bias() -> bool { true }
fn choose(comptime flag: bool, ...items: (bool, bool), .bias: bool = false) -> bool {
    let (a, b) = items;
    inline if flag { a and bias } else { b }
}
fn forward(comptime flag: bool) -> bool {
    choose(flag, ...do { (first(),) }, .bias = bias(), ...do { (last(),) })
}
```

这里依次调用 `first`、`bias`、`last`，再调用 `choose`，每个 producer 只执行一次。若第二个包提前返回，已完成的输入保持原有资源清理责任，后续输入与 `choose` 不执行；字段数量、类型和阶段要求仍然检查。通过 `.items = value` 供应完整收集值时，先保留整个入口，再将位置包分配给其余锚点。尾随 `do` 的 callback 锚点也先行保留，不占收集包的字段。


属性包的目标域可以依赖同次调用显式供应的 comptime 输入：

~~~flurry
typealias Type = .Type'builtin;

comptime fn schema(...properties: T) -> T where T {
    properties
}
const SAMPLE = schema { flag: true };
typealias Options = SAMPLE'type;

fn collect(comptime T: Type, ...properties: defer T) -> defer T {
    properties
}

fn use_properties() -> bool {
    let result = collect { Options, flag: true };
    result.flag
}
~~~

这里先形成 properties 的域，再检查属性包的字段；`flag` 的错误类型、缺失字段或不接受 record 的目标域都会拒绝。显式子句实例化的开放调用同样支持属性包及 post do，仍检查子句约束和各输入的阶段，不要求先把开放函数转换成普通函数值。 属性入口选择使用显式子句及其默认值完成后的域；例如两个入口分别声明为 T、U，而调用指定 T 为 tuple、U 为 record 时，属性包进入 U。实例化结果贯穿展开准备和最终调用，不重算类型子句；两者都为 record 或都为 tuple 时，默认属性协议不能选出唯一入口。

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

当前实现已在 HIR 保留 catch 参数的独立角色；函数签名的 continuation 绑定尚未接通，因此在 elaboration 明确拒绝，不将其降为可用普通实参调用的参数。

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


收集入口内部的已知元素同样提供推导依据，不必等待整个收集值形成。例如：

```flurry
typealias Byte = .u8'builtin;
fn seed() -> Byte { 9 }
fn accept(comptime flag: bool, ...items: (T, T)) -> bool where T { true }
fn forward(comptime flag: bool) -> bool {
    accept(flag, seed(), ...do { (7,) })
}
```

`seed()` 的 `Byte` 域约束 `T`，于是包中的 `7` 按 `Byte` 检查；把包写在 `seed()` 前面也能使用这份类型依据。预推导保留已经检查的输入，运行时仍只求值一次，并按源码顺序执行。若前面的包提前返回，后面的 `seed()` 不执行。部分收集域只保存在当前推导候选中，不把未解变量写入完成后的 IR。


这份证据还会传到最终泛型调用推导：展开包形成的字段绑定，以及显式局部绑定，均可约束同一收集值中的未标注闭包。例如 `...items: (T, fn(T) -> U)` 中，第一个字段已知为 `bool`、调用结果要求 `U = bool` 时，第二个字段 `|value| not value` 按 `fn(bool) -> bool` 检查。开放定义检查与实际调用采用相同的字段证据规则；改成返回整数的闭包仍被拒绝。 如果 `U` 不受外层结果约束，也可由已获得参数上下文的闭包体推导。若闭包参数域是 `T.as(Source).Element`，收集元素确定 `T` 后先按已成立的 trait 实现化简关联类型，再检查闭包体；缺少实现或化简后的参数域不满足闭包操作时仍拒绝。


普通泛型调用也逐元素推导收集值，不要求调用者先把生产表达式写成局部变量：

```flurry
fn seed() -> bool { false }
fn apply(...items: (T, fn(T) -> U)) -> U where T, U {
    let (value, action) = items;
    action(value)
}
fn run() -> bool { apply(seed(), |value| not value) }
```

检查 `seed()` 后，其结果域约束闭包参数域。每项成功检查后保留结果，再重新推导其他元素；数字默认类型只在独立证据已用尽时选择，所以 `(7, seed_byte())` 中 `seed_byte()` 返回 `u8` 时，`7` 按 `u8` 检查。预推导、依赖参数准备及最终调用之间传递这些结果，不能重复执行生产表达式。具名实参穿插在收集元素之间时仍按源码顺序求值；提前退出只阻止后续运行，不免除其他字段的静态检查。


属性收集也逐字段保留推导结果。记录域中的 `value: T` 可以从 `value: seed()` 得到 `T`，再为 `callback: fn(T) -> U` 的未标注闭包提供上下文；字段名字仍按记录域校验，不能把属性包当作位置元组。检查可以先取得后面字段的类型证据，但生产表达式仍按调用源码顺序执行，提前退出阻止后续生产表达式及被调函数运行。此路径同时适用于 runtime 调用和 comptime invocation。


开放函数的冒号属性先按收集入口路由，再参与定义侧调用检查。依赖扫描区分属性名与具名形参，登记转发到 comptime 参数的绑定依赖；单个属性入口，或可由已形成的 tuple/record 域唯一确定的属性入口，都不需要执行 producer 才能确定这份路由。位置收集与属性收集并存时，前置依赖扫描采用同一入口选择规则；多个仍可能接收属性的入口保持歧义，不能按声明先后任意选择。返回域仍依赖未知 comptime 输入时，保留该依赖，不为推导属性中闭包的类型而提前调用返回域计算。runtime 属性字段仍逐项提供候选证据，实际实例化才取得完成的签名。


混合收集域若由显式泛型实参确定，先应用该组实参再选择属性入口。例如位置入口的域为 `T`、属性入口为 `U` 时，`apply<(bool, bool), Options> { ... }` 在本次类型替换范围内区分 tuple 与 record。前置依赖扫描不为此执行类型实参；即使入口尚未分类，收集区前后的固定锚点仍可提供确定的 comptime 转发依赖。显式调用的阶段检查不变，runtime 绑定不能供应 comptime 锚点。
