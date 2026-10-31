# 参数化、Any 与上下文检查

## Intro

编写参数化函数时，作者先规定操作需要什么，再让调用提供具体类型。信息逐步变得精确，但一次操作不能在缺少依据时自行成立。

Any 表示任意类型，any 表示任意 comptime 值。它们参与计算与检查；把一个描述放宽到 Any，并不会自动创建运行时动态分派。这种渐进式类型（gradual typing）设计让程序在精确描述和通用表达之间取得平衡，而不牺牲静态检查的可靠性。

## 语法

~~~text
fn_def -> fn id(parameter*) (-> expr)? clauses? { statement* }
clauses -> where clause*
type_decl_clause -> id
type_bound_decl_clause -> id : expr
trait_bound_decl_clause -> id :- expr
requires_clause -> requires expr
normal_form_head_application -> expr<argument*>
~~~

这是与本篇相关的局部语法；参数种类、函数前缀与其他子句见[函数](../functions/index.md)和[comptime](../comptime/purity-and-normal-form.md)。

## 正式定义

### 信息与使用要求

Any 是任意类型的描述；any 是任意 comptime 值的描述。相容关系允许检查接纳相应未知描述，但不把各具体类型或值合并为同一个东西。

只知道 T: Type，不自动获得固定大小、某个 trait 或某个字段。后续操作所需的事实可以来自声明、当前要求、合法结构观察或证明。没有写出一个要求，并不禁止编译器推导它；没有推导出它，也不能默认为真。

含 Any 的 runtime 参数在调用点实例化。实例获得具体输入之后继续检查；调用点单态化不授权定义端任意使用未知成员。ordinary 与 loose 的边界遵循[normalization](../comptime/normalization-and-instantiation.md)。

### 延迟域与后续 comptime 参数

runtime 参数的类型计算尚未完成，不会吞掉它在签名中的位置，也不要求后续参数全部变为 runtime。当前实现支持在 `defer` runtime 域之后继续声明域已知的 comptime 参数，再用它们形成后续输入和结果类型：

```flurry
typealias Type = .Type'builtin;

comptime fn Pick(flag: bool) -> Type {
    if flag { bool } else { .u8'builtin }
}

fn select(
    comptime first: bool,
    unused: defer Pick(first),
    comptime second: bool,
    value: defer Pick(second)
) -> defer Pick(second) {
    value
}

fn run() -> bool {
    let flag = select(false, 7, true, true);
    let byte = select(true, false, false, 9);
    flag and byte == 9
}
```

后置的 `comptime T: Type`、comptime 默认值和依赖这些输入的类型约束也保留原始参数位置。类型计算只能捕获已知域的 comptime 输入；未知 runtime 域不是占位类型，其值不能被 comptime 计算读取。调用在输入齐备后必须完成所有类型约束，不能把尚未取得输入的约束当成成立。

comptime 参数自己的域也可以使用 `defer`。调用按声明顺序形成域，再按所得类型检查并求值该参数；不能因为参数处于 comptime 就跳过域检查：

```flurry
fn staged(comptime flag: bool,
          comptime value: defer Pick(flag)) -> defer Pick(flag) {
    value
}

fn flag_value() -> bool { staged(true, true) }
fn byte_value() -> .u8'builtin { staged(false, 9) }
```

这里沿用前文的 `Pick`。参数默认值同样按实例的域检查；显式实参覆盖默认值。若域计算得到 `Type`，完成后的签名把该参数记为 Type 输入；得到其他类型则记为 comptime 值输入。域尚未形成时只能保留其阶段和位置，不能假定它是 Type，也不能把它捕获进已检查的 comptime 代码。

直接引用此前 comptime 输入的 `defer T` 可以继续等待该输入：

```flurry
comptime fn Domain(flag: bool) -> Type {
    if flag { Type } else { bool }
}

fn deliver(comptime flag: bool,
           comptime T: defer Domain(flag),
           value: defer T) -> defer T {
    value
}

fn delivered() -> bool { deliver(true, bool, true) }
```

调用先形成 T 的域，再求得 T 的值，确认它是 Type 后才形成 value 的域和返回类型。`deliver(false, true, true)` 虽然给 T 提供了符合 bool 域的值，却仍会因 true 不是 Type 而拒绝。这种依赖可以串联多个 comptime 输入，也适用于默认值；不能引用 runtime 输入或尚在后面的参数。

下游也可以调用 comptime 函数。例如把上例中两处 `defer T` 改为 `defer Identity(T)`，其中 `Identity` 定义为：

```flurry
pure comptime fn Identity(value: Type) -> Type { value }
```

定义端先按 ordinary 描述检查这段调用，不能因此放行 `T.missing` 之类缺少依据的操作。输入尚不完整时保存待恢复的源码位置，而不是把带有未知域的输入捕获到已冻结代码中。调用端取得前置域和值之后，继续 elaboration，再执行通过检查的类型计算。当前路径覆盖普通 comptime 调用、嵌套调用、if/when 分支、match 和带局部绑定的 block；未知域参数也可以是 bool 等 comptime 值，而不只是 Type。match 的选择值和 pattern 仍须在定义端具有匹配依据，带 guard 的匹配仍需满足穷尽性检查；挂起分支结果不会取消这些要求。

普通函数类型构造也可以出现在待定注解中，例如 `comptime U: defer Domain(flag)` 后面的 `callback: defer fn(U) -> T`。定义端分别检查参数域与结果域表达式必须交付 Type，允许 U 的值保持待定；具体调用若发现 U 不是 Type，或者 callback 的实际函数类型不匹配，仍会拒绝。`fn(true) -> T`、`fn(U) -> true` 及 `fn(U.missing) -> T` 不会借此通过检查。指针与可选类型也逐项要求其内部表达式交付 Type，可在待定函数域中嵌套，例如 `fn(*U) -> T`、`fn(?U) -> T`、`fn(*?U) -> T`。可选类型仍要求驱动绑定有效的 Optional 声明，未知描述不替代这个协议。结构中也可以引用已经解析且不带待应用子句的命名类型，例如 `fn(U, T) -> Result` 中的 Result 别名。恢复时仍由通常的类型表达式检查器解析该名字；别名指向 normal form 时保留其头身份，`Identity<Byte>` 不因函数体返回 Byte 而等于 Byte。当前覆盖普通位置参数的函数类型构造；这提供的是构造结果的 Type 描述，并不允许把未知域的 callback 直接当成已知可调用值。具体调用可以按下面的顺序形成输入域，再从结构类型中取得推断证据。

```flurry
comptime fn Domain(flag: bool) -> Type {
    if flag { Type } else { bool }
}
fn number(value: bool) -> Byte { if value { 9 } else { 7 } }
fn accepts(comptime flag: bool, comptime U: defer Domain(flag),
    callback: defer fn(U) -> T, value: defer U) -> bool where T {
    false
}
let accepted = accepts(true, bool, number, true);
```

这个例子只检查参数交付：定义端 callback 的完整域未知，主体不能直接调用它。调用端先准备 flag，执行不依赖未决声明参数的 Domain，确认 U 的域为 Type；取得 U 后形成 `fn(bool) -> T`，由 number 推断 T 为 Byte。已形成的参数域随调用准备结果复用，最终调用仍替换声明参数并检查实参。前面无关的 runtime 域可以继续待定，例如插入 `earlier: defer Select(flag, T)`；后面的 U 与 callback 仍可先形成并推断 T，随后才完成 earlier 的域和实参检查。注解恢复使用保持原始位置的可选域表，不压缩未知参数，也不提供伪造的类型。普通描述检查不会执行这些待定计算；提前形成要求全部相关类型义务可先被证明。例如更早的 `comptime S: Type` 配合 `requires S == bool`，可以先取得 S 并证明约束；不成立时拒绝，不执行后面的待定注解计算。仍提及未决声明参数 T 的义务继续等待，不能借定义端假设自证。计算本身须已检查且不实际引用未决声明参数，悬挂源码恢复目前覆盖简单函数、指针及可选类型结构。含未知声明参数的类型计算不做逆向求解，带约束或更复杂待定结构的组合仍需完善。

同样的前向形成也适用于返回域，并在实参采用数字默认类型之前利用调用目标：

```flurry
fn factory(comptime flag: bool, comptime U: defer Domain(flag),
    callback: defer fn(U) -> T) -> defer fn(U) -> T where T {
    callback
}
fn make() -> fn(bool) -> Byte {
    factory(true, bool, |value| if value { 9 } else { 7 })
}
```

这里返回目标先约束 T 为 Byte，再为 lambda 的参数和返回值提供上下文。将 lambda 改成返回 bool 或超出 Byte 范围的整数仍会拒绝；不能先默认成 i32，再假装整个函数类型可隐式转换。不同结果目标产生各自正确的实例，新形成的返回域也随准备结果复用。

泛型 comptime 调用可以从结果目标或已知实参取得类型信息。例如 `comptime fn identity(value: T) -> T where T { value }` 用于 `defer identity(T)` 时，这里要求产生 Type，可据此推断 identity 的泛型输入类型。未知实参仍保持未确定描述，不能被当作一个叫 Any 的具体类型加入求解。嵌套调用缺少信息时可以等待其它实参；只有缺少上下文的失败允许重试，类型冲突和约束失败直接报告。具体调用仍需重新检查所有实参及泛型约束。当前这条描述推断路径覆盖已形成签名中的普通位置参数、普通默认参数、类型子句默认值和已知的显式类型实参；与依赖签名的完整组合仍需完善。

类型子句默认值复用普通泛型调用的求解规则。例如 `.U: Type = Identity(T)` 等到 T 的推断结果可用之后才计算，再将 U 的结果带入后续实参检查。显式给出 `keep<.T = Type>(value)` 会覆盖对应类型默认值；即使被覆盖的默认表达式不能终止，也不应执行它。不能因为 T 和 U 最终具有相同结构就略过默认 invocation 或展开 normal-form 头。

带 `defer` 注解的 runtime 函数也可以声明类型子句，并通过显式类型实参完成调用：

```flurry
typealias Byte = .u8'builtin;

comptime fn Select(flag: bool, T: Type) -> Type {
    if flag { T } else { bool }
}

fn choose(comptime flag: bool, value: defer Select(flag, T))
    -> defer Select(flag, T) where T {
    value
}

fn byte() -> Byte { choose<Byte>(true, 9) }
fn boolean() -> bool { choose<bool>(true, true) }
fn alternate() -> bool { choose<Byte>(false, true) }

fn forward(value: T) -> T where T { choose<T>(true, value) }
```

类型子句先通过普通的参数与约束检查，再用于形成各个延迟注解。类型子句默认值与显式覆盖也适用；已知域中的 T、程序主体及输出的类型约束必须使用同一份替换。实例身份同时包含声明参数与普通 comptime 输入，不能因 flag 相同而混用 `choose<Byte>` 与 `choose<bool>`。

从 `forward<T>` 转发时，T 仍是调用方的抽象 Type，不是某个猜测的具体类型。类型计算与生成的 runtime 模板分别保留这份绑定，后续具体实例化再替换它。此过程不展开 normal-form 头。当前实现已接通上述显式声明参数路径；抽象声明值捕获、方法和完整高阶组合仍待完善。

调用方已有足够类型信息时，也可以省略声明类型实参。当前推断使用已形成的参数域和结果域；具体调用可以先替换已准备的普通 comptime 输入：

```flurry
fn from_input(comptime flag: bool, value: T)
    -> defer Select(flag, T) where T { value }

fn from_result(comptime flag: bool, value: defer Select(flag, T))
    -> T where T { value }

fn inferred() -> Byte {
    let n: Byte = 9;
    from_input(true, n)
}

fn contextual() -> Byte { from_result(true, 9) }
```

前者从 n 的类型推断 T，后者从结果目标 Byte 推断 T，再形成 value 的域。类型子句默认值等到其依赖确定后完成；普通数字默认类型只在其它证据不能继续推进时启用。已经检查的实参保留到调用准备中，runtime 求值仍按源码顺序进行一次。

已形成域的 comptime 实参也可以提供类型证据，但阶段检查不会因此放宽：

```flurry
comptime fn number() -> Byte { 9 }

fn staged_value(comptime flag: bool, comptime value: T,
                comptime .other: T = value)
    -> defer Select(flag, T) where T { other }

fn staged_byte() -> Byte { staged_value(true, number()) }
fn staged_bool() -> bool { staged_value(true, true, .other = false) }
```

推断先取得 `number()` 的结果类型 Byte，参数准备再完成其 comptime 值并形成输出域。已检查的实参表达式直接交给这一步，不重新 elaboration 同一实参；默认值使用本次调用准备的 value，显式 `.other` 覆盖它。把 `number` 改为 runtime 函数，或直接传入一个 runtime 参数，仍会因缺少合法 comptime 输入而拒绝。

待定域不作为具体类型参与这一步求解，也不通过逆向执行 `Select` 猜测 T。因此此前的 `choose(true, true)` 仍需要显式 T：它的参数域和结果域都尚未形成。具体调用中，已形成的域若依赖普通 comptime 输入，可以先准备显式提供且域已知的输入，再用替换后的域推断。例如：

```flurry
fn apply(comptime U: Type, comptime flag: bool, f: fn(U) -> T, value: U)
    -> defer Select(flag, T) where T {
    f(value)
}

fn number(value: bool) -> Byte { if value { 9 } else { 7 } }
// 先取得 U = bool，再由 fn(bool) -> Byte 推断 T = Byte。
let n = apply(bool, true, number, true);
```

为这一步准备的 comptime 值与符号由最终调用复用，不重新 invocation 输入供应函数。runtime 值不能用来完成这些输入，错误的函数参数域也不能靠替换蒙混通过。已形成域所需的 staged 输入省略时，也可以准备其默认值，例如把上例 U 改为 `comptime .U: Type = bool`；最终调用复用该默认值，显式 `.U` 覆盖后不执行默认供应函数。默认表达式所读的更早输入会递归加入准备过程，例如 `.seed: bool = true`、`.Base: Type = Domain(seed)`、`.U: Type = Base` 可以依次取得 U。链中值的域也可以依赖更早的 Type 输入，例如 `S: Type`、`seed: S`、`.U: Type = TypeOf(seed)`；先取得 S、检查并准备 seed，再执行默认供应函数。依赖来自已检查表达式中的局部引用与类型引用；被显式覆盖的默认表达式不贡献依赖。默认输入自身的域和实际读取的输入必须具备依据；前面无关的待定 runtime 域不阻塞它，默认值作用域保留这些参数的原始位置和未知域空位。取得声明参数后仍须完成这些待定域并检查实参，不能借此放过错误输入。不从待定注解逆推其输入；需要在推断过程中执行待定注解，以及定义端未知描述的完整组合仍需完善，可先使用显式声明参数。

未提供普通参数时，默认表达式已经检查过的独立类型描述也可参与推断：

```flurry
fn default_bool(comptime flag: bool, .value: T = true)
    -> defer Select(flag, T) where T, requires T == bool {
    value
}

fn default_result() -> bool { default_bool(true) }
```

定义端的约束使默认表达式 true 合法，其 bool 描述为本次调用提供 T 的证据；调用仍须通过类型约束检查。默认值引用此前 bool 参数时，也可以使用该参数的类型描述，而不用读取它的 runtime 值。

该证据只在整个定义检查成功后保存，并且只用于没有被显式实参覆盖的槽。仍含同一未确定 T 的默认值类型不能反过来证明 T，普通输入绑定先用本次准备的符号替换，再作为调用方证据；尚未取得的绑定继续等待。例如 `comptime U: Type`、`comptime seed: U`、`comptime .value: T = seed` 配合 `requires T == U` 时，可以在取得 U 后从 seed 的类型推断 T。该约束必须在定义端支持默认值的交付，并在调用端替换普通输入后再次证明；显式提供不同类型的 value 不能绕过它。推断不会执行默认供应函数：即使 comptime 默认供应函数不能终止，显式覆盖后也不应执行它；如果它的类型仅为未确定的 T，省略实参应报告缺少推断证据，而不是试着运行它寻找答案。

另一 open 定义可以用已知类型描述进行隐式转发。例如：

```flurry
fn forward_inferred(comptime flag: bool, value: T)
    -> defer Select(flag, T) where T {
    from_input(flag, value)
}
```

定义端从 value 的已知描述 T 推断 callee 的声明参数，不需要执行 flag 或形成待定返回域。调用方的 T 可以保持抽象，随后具体实例化再替换。callee 的约束必须能由当前定义的证据证明；若其 value 是 comptime 参数，runtime value 仍不能传入。只有 Any 描述而没有其它已形成的类型证据时，不凭空指定声明参数。

若未知域操作数本身就是明确的推断来源，定义端可以保留待定描述：

```flurry
fn forward_unknown(comptime flag: bool, value: defer Pick(flag))
    -> defer Pick(flag) {
    from_input(true, value)
}
```

`from_input` 的参数域是 T，因此其 T 等待 value 的实际类型；这不同于逆向分析一个待定的 `Select(flag, T)` 域。定义端不确定 T，也不冻结活动求解器状态。具体调用先形成 value 的域，再重新完成这次泛型调用。由该 T 决定的类型子句默认值可一同等待；无来源的 Phantom 参数及默认值依赖环不能因此获得依据。

当前挂起路径要求 callee 没有类型约束或 capability 义务，并且所有未确定参数都有上述来源。其余实参仍逐个检查，不能把无效成员访问藏在待定参数域后面。与未确定参数无关的已知结果描述可以保留，例如 bool 结果仍能用于 `and`；待定结果则不能供任意成员操作使用。

转发函数自身也可以有尚未形成的参数域：

```flurry
fn forward_deferred(comptime flag: bool, value: defer Select(flag, T))
    -> defer Select(flag, T) where T {
    choose<T>(flag, value)
}
```

定义端按显式 T 替换 callee 的已知类型描述，待定参数域和结果仍保持待定；不能为了检查这次调用而执行依赖 flag 的 `Select`。callee 的类型约束仍须由定义端证据支持，例如 callee 要求 `T == bool` 时，未受约束的转发定义不能假定它成立。调用得到的待定结果也不提供 `.missing` 等成员操作的依据。具体实例中才继续形成各个域并检查所有实参。

普通参数默认值也保留声明端绑定。例如 `comptime fn keep(value: T, .other: T = value) -> T where T { other }` 的 other 默认值引用本次调用的 value。描述检查复用已检查的默认表达式；其已确定类型可以提供推断信息，但仍含该声明泛型绑定的类型不能反过来充当确定这些绑定的依据。显式提供 `.other` 会覆盖默认计算；描述推断不能为了补充类型信息而执行被覆盖的默认表达式。

后置 Type 参数也可以受 `requires T :- Source` 约束，并用于 trait 方法调用或 `T.as(Source).Item` 关联类型。定义端依赖声明的 trait 证据；调用端仍需取得具体实现。实例化后，参数和结果使用该实现给出的关联类型视图，不把不同实例的关联类型合并。检查局部代码片段时，即使省略了未知 runtime 域，也须同步重映射类型引用及其契约，不能改变后置 Type 的身份。

类型构造器默认参数中的 comptime invocation 也遵循这个规则。例如 `Tagged<T, .Other: Type = Identity(T)>` 的默认值用调用者提供的 T 计算；即使 T 前面存在未知 runtime 参数，也不能丢失其绑定位置。展开默认参数时，名字仍按声明位置解析，代入的类型输入则保留调用者的作用域。嵌套默认参数必须继续携带这份环境。默认输入求值不会展开 `Tagged` 自身的 normal-form 头，也不会使它与返回相同类型的另一个构造器相等。

无约束的 runtime 泛型调用也可以利用结果目标描述未知实参。例如：

```flurry
fn identity(value: T) -> T where T { value }

fn choose(comptime flag: bool, value: defer Pick(flag)) -> bool {
    identity(identity(value))
}
```

这里沿用前文的 Pick。定义检查可由 bool 结果目标向内推断两个 identity 的类型参数，但 value 本身仍是未知描述。`choose(true, true)` 可以执行；`choose(false, 9)` 在具体实例检查时拒绝，因为 u8 不能交付 bool。当前 runtime 描述推断只覆盖无类型命题和 capability 约束的普通泛型签名；带约束的调用仍要求定义端已知实参域，不能把结果目标当作未知实参满足 trait 的证据。

没有结果目标时，未知值也可以经无约束的泛型调用转发：

```flurry
fn forward(comptime flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {
    let saved = identity(value);
    identity(saved)
}
```

定义端的 saved 保持 Any 描述，不能由此取得字段、算术或 trait 操作的依据。若泛型函数返回与未知类型参数无关的已知类型，则仍保留该结果描述，例如返回 bool 的函数可以用于后续分支条件。只有未知实参涉及的泛型输入可以暂缓；既无实参依据、也无目标或默认值依据的额外类型参数仍是歧义。这里保存的是描述，不是带活动推断变量的类型或 NIR 调用。具体实例仍重新完成泛型推断、实参检查和资源检查。

挂起的 comptime 类型计算同样允许这种无约束泛型转发。例如，以下片段中的 identity 是 comptime 函数：

```flurry
comptime fn identity(value: T) -> T where T { value }

fn choose(comptime flag: bool, comptime seed: defer Domain(flag),
          value: defer do { let saved = identity(seed); identity(saved) })
    -> defer do { let saved = identity(seed); identity(saved) } {
    value
}
```

这里 Domain 沿用前文按 flag 返回 Type 或 bool 的定义。输入域尚未形成时，saved 只是未知描述；调用端完成 seed 的域和值之后，才重新检查并执行整个块。seed 为 bool 类型值时，块未必产生 Type，因此不能仅凭定义端接受该描述就接受具体调用。runtime 输入、错误阶段的函数调用、未知值的成员操作仍会拒绝；带类型或 trait 约束的未决泛型调用也不能通过这条无约束转发路径跳过证明。

默认类型也可以提供待恢复的推导来源。例如 `where T, .U: Type = Identity(T), .V: Type = Identity(U)` 中，若 T 等待未知实参的域，U 和 V 可以随之等待，而不是作为无来源的额外泛型参数报错。此时只沿默认值的依赖关系确认其来源，不执行默认 invocation，也不建立 U、V 与 T 的类型相等关系。具体调用仍须计算所需默认值；即使某个默认类型未出现在结果中，也不能因此省略它。独立且没有推导来源的泛型参数仍然拒绝，默认值依赖分析也消耗 elaboration 预算。

挂起块中已检查的局部值可以继续参与同阶段运算。例如：

```flurry
fn choose(comptime flag: bool, comptime seed: defer Domain(flag),
          value: defer do {
              let condition = flag;
              if not condition { bool } else { seed }
          }) -> defer do {
              let condition = flag;
              if not condition { bool } else { seed }
          } {
    value
}
```

condition 的 bool 域和 comptime 来源均有依据，所以可以使用 not。嵌套块和已经检查的解构绑定也保留这种来源；离开作用域后恢复外层环境。描述检查只将被引用的已知局部值放入临时验证环境，不将它们变成可冻结代码的捕获授权。未知域局部值仍不能用于布尔运算，runtime 输入也不能因先绑定到另一个名字而进入 comptime 计算。具体实例仍检查整个类型表达式，不会只凭条件值略过不合法的分支。

match 的模式绑定也遵守同一阶段规则。例如在上述挂起类型表达式中，`flag match { condition => if not condition { bool } else { seed } }` 可以使用 condition 的已知 bool 域。必须先验证选择值的阶段，模式绑定才继承该来源；嵌套匹配与元组解构同样适用。每个 arm 的绑定仅在该分支有效。guard 仍参与完整的模式程序验证，不能借模式绑定夹带 runtime 参数；穷尽性要求也不因此放宽。

挂起的 comptime 块可以使用 `var saved = seed; saved = identity(seed); saved` 转发未知域的值。赋值目标必须是在该阶段建立的局部变量；`let` 仍不可重赋值。定义描述不把赋值后的未知域猜成某个具体类型，实例化时仍按局部变量的实际类型检查每次写入。例如 seed 的域为 Type 时，`saved = bool` 是写入另一个 Type 值，最终可改变所生成的类型；`saved = true` 则是类型错误。阶段来源只说明局部值在哪个阶段建立，不代替其类型或操作证据。

循环体和赋值右侧也参与挂起类型计算的依赖发现。例如 `defer do { var saved = bool; while flag { saved = seed; break; } saved }` 中，即使未知域输入 seed 只在循环内出现，也必须先等待其域形成。saved 的域已知为 Type，可以按这个目标描述写入，具体实例再检查 seed 是否真的交付 Type。局部更新、continue 和命名 break 保留正常控制流；循环未执行并不使其内部的类型错误合法。

挂起块也可创建并调用捕获已知域 comptime 值的局部闭包。按值捕获保留创建时的值，后续更新外层局部变量不改变该快照；嵌套闭包工厂继续保持各层捕获环境。runtime 参数不能借闭包进入 comptime 计算。阶段内存活的局部存储可以通过指针捕获读写标量，写入会影响后续类型计算；指针不延长该存储寿命，也不能作为常量或闭包常量的捕获被物化。聚合值的指针读取与替换须取得已绑定 Copy/Drop 的具体资源证据；标量指针的位写回保留范围、右值和提交顺序，见 [comptime 的临时存储边界](../comptime/index.md)。

### 分支与目标上下文

无目标上下文时，同型分支汇合为该类型，异型分支汇合为 Any。没有自动合成一般 union 或公共父类型的规则。

汇合只考虑正常完成并交付结果的路径。已确定不正常完成的分支（例如执行 `return`）不把另一条路径的结果放宽到 Any；这条控制流规则不建立 `NoReturn` 与该结果类型相等的命题。`void` 仍是正常完成的结果，不能按相同方式忽略。

存在目标类型时，先把目标要求传入分支，按已定义转换检查各分支，再检查交付。目标类型提供检查方向，不是允许任意强制转换的开关。

Any 可以出现在返回描述中。是否合法取决于具体实例能否交付所需类型、操作与表示，不能只因名字出现便拒绝，也不能因其允许出现便假设通用装箱。

### 不同适配分别处理

| 输入与目标 | 检查要点 |
|---|---|
| 普通值进入 optional | 按目标进行成功注入 |
| 已有 optional 进入更外层 optional | 保留源值整体，再包外层 |
| 无具体源类型的 null | 构造目标 optional 的外层空值 |
| 普通成功值进入 error qualified type | 成功注入 |
| 已有 error 值进入更宽错误行 | 需要独立的行嵌入及载荷映射 |
| 有成员要求的 Any | 所用操作还需要合法成员来源与签名依据 |

可接受性检查和实际值转换必须同时成立。单独证明某项性质，不会产生传递该值所需的表示方案。

## 常见示例

参数类型的关系可以先由签名说明，具体类型再由调用给出：

~~~flurry
fn identity(value: T) -> T
where T {
    value
}

fn pass_through(value: Any) -> Any {
    value
}

test {
    let integer: u32 = identity(7);
    let flag: bool = identity(true);
    let forwarded_integer: u32 = pass_through(integer);
    let forwarded_flag: bool = pass_through(flag);

    println("integer: {}", forwarded_integer);
    println("flag: {}", forwarded_flag);
}
~~~

输出为 `integer: 7`、`flag: true`。`identity` 明确保持输入与输出的类型关系；`pass_through` 的具体调用也能交付各自的结果。这些调用不要求先把整数和布尔值装入统一运行时盒子。

下面让返回目标影响分支检查：

~~~flurry
fn selected_limit(enabled: bool) -> ?u32 {
    if enabled {
        8
    } else {
        null
    }
}

test {
    selected_limit(true) match {
        limit? => println("limit: {}", limit),
        null => println("disabled"),
    }

    selected_limit(false) match {
        limit? => println("limit: {}", limit),
        null => println("disabled"),
    }
}
~~~

输出为 `limit: 8`、`disabled`。两个分支检查时已知道目标是 `?u32`，因此数值分支注入成功值，`null` 分支构造缺失。不能把这个结果归因于“任何异型分支都会自动组成 optional”。

给参数一个名字仍不授予未知成员。以下普通函数缺少调用依据：

~~~flurry
fn unspecified_length(value: T) -> usize
where T {
    value.len()
}
~~~

仅有 `T` 不能证明存在 `len` 或确定其来源。应提供实际需要的能力要求，或者采用能合法观察具体输入的编译时计算；把参数改名为 `Any` 不会补出成员契约。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的泛型参数和 C++ 的约束模板提供按参数检查操作的实际对应物；C 的 void 指针则必须靠显式协议解释数据，不能对应 Any 的调用点实例化。Zig 的 anytype 可用于比较推导参数类型，但不能据此推导 Flurry 的分支汇合与返回规则。[Rust 泛型](https://doc.rust-lang.org/reference/items/generics.html)、[基础对照](index.md#与-rust--zig--c--c-的关键差异)

## 运行时实现设计引导

对已实例化的函数，确定每个参数和返回值的主体表示以及环境。若一个实例仍包含异型运行时分支，必须检查后续路径是否有已定义的统一交付方式。

例如两个分支都满足某个“可显示”要求，只解决操作能力的一部分；仍需知道调用入口、接收者适配与值的传递形式。实现不能仅据布尔能力事实生成未经定义的动态对象。

## 编译器实现引导

使用双向检查时，可将无目标推导和带目标检查分成两个入口，但共享相同语义规则。

1. 带目标检查先向子表达式传播目标。
2. 各分支在自己的事实环境中检查，并记录具体转换。
3. 无目标汇合只使用同型或 Any 规则；相等未证明不能伪装成同型。
4. 分支后的事实必须由所有可达前驱支持；类型汇合不能替代事实汇合。
5. 在实例化边界替换实例参数，保留定义端已确定的成员来源。
6. 交付前验证必要布局、环境、调用来源及结果契约。

诊断应区分“这个操作没有事实依据”和“这些已知分支缺少交付表示”。两者可能都发生在 Any 附近，但修复方式不同。
