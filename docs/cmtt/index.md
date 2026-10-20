# CMTT：带上下文的代码生成

## 当前实现边界

当前 luna 已接通局部 `const` 表达式代码的解析、类型检查、卫生接入、IIR 实例化与解释执行。引用保留 runtime 绑定身份；可以推导上下文，也可以用 `< Ψ |- T >` 声明虚拟接口。默认对应、构造时暴露端口、接入时同时替换、代码别名、编译时条件选择和单洞组合均进入实际源码测试。

代码值绑定支持 `_` 丢弃和 `as` 整体别名，无须为代码值构造 runtime 表示：

```flurry
fn same(flag: bool) -> bool {
    const _ = '(not flag);
    const (_ as first) as second: <x: bool |- bool> = '(x);
    ~[x = flag]<first> == ~[x = flag]<second>
}
```

丢弃只省略保存结果，仍执行生成器、检查其契约及所声明的代码接口；quoted 主体不会因此执行。多个别名保留同一代码及上下文证据。在生成器内，`var first as second = '(false)` 创建两个可重新赋值的绑定，随后改写 first 不会改写 second；`let` 与局部 `const` 的别名仍不可重新赋值。这里不提供代码结构的解构或反射。

~~~flurry
fn selected(value: bool) -> bool {
    const original: < input: bool |- bool > = '(input);
    const combined = '(not ~[input = value]<original>);
    let value = false;
    not ~<combined>
}
~~~

`selected(true)` 返回 `true`：虚拟 `input` 在组合时接到原形参，后来的同名局部不改变这条绑定关系。代码创建本身不读取或消费形参。每次接入分配独立的内部局部身份和展开位置，生成程序继续接受原有的资源、清理、效应及阶段检查。

显式 `comptime fn`（可带 pure）已能接收完成的数据及具有具体接口的表达式代码参数，组合并返回表达式代码。数据支持标量、Type、Tuple、匿名记录与 struct/enum 值；保留完整名义类型身份，Tuple 使用绑定的 normal-form 头。生成器内支持不可变局部、直接返回和条件表达式；数据参数通过既有 comptime invocation 求值并冻结，代码参数保留上下文见证。

~~~flurry
pure comptime fn invert(code: < x: bool |- bool >) -> < x: bool |- bool > {
    '(not ~<code>)
}

fn twice(value: bool) -> bool {
    const original: < x: bool |- bool > = '(x);
    const generated = invert(invert(original));
    ~[x = value]<generated>
}
~~~

`twice(true)` 返回 `true`。生成器在编译时完成，生成的 runtime 程序仍接受移动、清理及效应检查。未调用的生成器也检查定义，包括未选中的分支；递归构造受预算约束。已有代码可以重新标注兼容接口，改名保留原绑定对应；固定依赖不能借重新标注变成自由端口。

生成器不要求 pure。普通 comptime 生成器可调用同阶段的数据函数、更新自己的数据状态、计算默认值并检查程序契约：

```flurry
comptime fn opposite(value: bool) -> bool { not value }

comptime fn build_opposite(value: bool) -> <|-bool> {
    let result = opposite(value);
    '(result)
}

fn generated() -> bool {
    const code = build_opposite(false);
    ~<code>
}
```

pure 仍按 [comptime 纯性规则](../comptime/purity-and-normal-form.md)作为可信承诺处理，不能把未标 pure 当成拒绝代码生成的理由。定义检查不会执行生成器；每次实际构造仍需完成输入与代码结果，并遵守阶段、契约和资源预算。去掉 pure 不允许读取 quoted runtime 上下文、不开放 runtime 调用，也不使 quote fn 变成 comptime fn。方法生成器尚未接通。

生成器也能接收 comptime 函数作为数据回调，包括已命名函数、显式泛型实例和无捕获 lambda：

```flurry
comptime fn invert(value: bool) -> bool { not value }

comptime fn build_with(action: comptime fn(bool) -> bool,
                      value: bool) -> <|-bool> {
    let result = action(value);
    '(result)
}

fn generated_callback() -> bool {
    const code = build_with(invert, false);
    ~<code>
}
```

已命名回调保存声明身份和实例实参，无捕获 lambda 保留已检查的函数体；两者都不序列化活动捕获图。普通数据运算、默认值与 implicit 具名供应均可转发它；执行时保留回调契约与 comptime 阶段检查。回调可以在数据计算内接收指向生成器局部存储的指针，修改随状态帧一起保存。普通生成器的显式指针参数和引用接收者通过 invocation 局部值传递地址，保持原所有者与写回关系；地址不能冻结为代码常量或运行时结果。runtime 已命名函数及静态 trait 方法可以作为声明身份传递，包括泛型实例；生成器不能直接调用它们，但可以将调用放入返回的 quote，由生成后的 runtime 代码执行。显式闭包类型还可以携带经快照验证的只读捕获环境；修改捕获状态、暴露捕获存储的地址以及含指针的捕获图仍拒绝。结构元组、匿名记录、结构体、枚举和 newtype 可以递归保存无捕获 comptime 回调，各字段独立经过生成器数据重建边界。结构体和枚举按已实例化的字段布局重建，newtype 按表示类型重建，并保留各自的类型身份。这不改变 quoted-code 接口的 rank-1 限制。

例如，生成阶段选择 runtime 函数，调用留在代码内部：

```flurry
fn invert(value: bool) -> bool { not value }
comptime fn emit(action: fn(bool) -> bool) -> <|-bool> {
    '(action(false))
}
fn run() -> bool {
    const code = emit(invert);
    ~<code>
}
```

函数身份重建保留声明与实例参数，不转成普通标量 normal term。调用阶段及函数契约仍由正常检查保证；不能通过传递身份抹去前置条件。接收函数身份时，即使某个值子句未出现在普通函数签名中，也必须检查它的声明域。依赖前序 Type 输入的域先替换再检查；把身份放进元组等聚合中，不会免除这一步。静态 trait 方法身份遵循相同规则。

生成器的值子句可以先组成结构数据，再作为函数实例参数。定义检查保留当前子句身份，实际生成时使用本次实例的值；不会为了组装结构体而读取尚未具现化的局部绑定：

```flurry
struct Config { enabled: bool }
comptime fn config(enabled: bool) -> Config { Config { enabled: enabled } }
fn apply(value: bool) -> bool where config: Config {
    if config.enabled { not value } else { value }
}
comptime fn configured() -> <|-bool> where enabled: bool {
    emit(apply<config(enabled)>)
}
fn configured_result() -> bool {
    const yes = configured<true>();
    const no = configured<false>();
    ~<yes> and not ~<no>
}
```

同样适用于嵌套结构体、枚举载荷及经只读 comptime 闭包转交的函数身份。构造函数仍须属于 comptime 阶段并满足契约；保留符号输入不授权对未知值执行分支判断。

只读 comptime 闭包也可以捕获这种身份，再把它交给生成器。捕获检查递归处理嵌套闭包，保留函数实例及静态 trait 方法身份，并检查实例输入已完成或属于获准的 Type 参数作用域：

```flurry
typealias Selector = %{action: fn(bool) -> bool}
    comptime fn() -> fn(bool) -> bool;
comptime fn make_selector(action: fn(bool) -> bool) -> Selector {
    || action
}
comptime fn emit_selected(select: Selector) -> <|-bool> {
    let action = select();
    '(action(false))
}
fn selected_result() -> bool {
    const code = emit_selected(make_selector(invert));
    ~<code>
}
```

这里只在 comptime 调用 selector 来取得函数身份，实际 action 调用仍在 runtime。若 selector 的正文直接调用 action，阶段检查会拒绝。这个能力也不把 comptime 中构造的普通 lambda 改成 runtime 闭包；捕获图中的指针、可观察的可变状态及未完成实例参数仍受快照边界约束。

只读捕获的名义值保留完整 Type／值参数。例如 `Flag<true>` 中的 true 是布尔索引，不能按 Type 参数冻结：

```flurry
struct Flag where enabled: bool { value: bool }
typealias Reader = %{data: Flag<true>} comptime fn() -> bool;
comptime fn make_reader() -> Reader {
    let data = Flag<true>{value: true};
    || data.value
}
comptime fn emit_reader(read: Reader) -> <|-bool> {
    let value = read();
    '(value)
}
fn reader_result() -> bool { ~<emit_reader(make_reader())> }
```

同样的规则适用于 enum、混合 Type／值参数和结构化值索引。函数身份也可携带 `Config{...}` 一类实例参数；实例形成仍按声明域与名义布局检查字段类型、字段数量和定宽数值范围。快照检查只确认已通过域检查的输入具备稳定表示，不执行参数计算，也不改变类型身份。

回调也可以作为名义配置对象的一部分：

```flurry
struct Action { callback: comptime fn(bool) -> bool }
enum Choice { none, some(Action) }
comptime fn invert(value: bool) -> bool { not value }
comptime fn configured(choice: Choice) -> < |- bool > {
    let result = choice match {
        .some(action) => action.callback(false),
        .none => false,
    };
    '(result)
}
fn configured_result() -> bool {
    const code = configured(Choice.some(Action { callback: invert }));
    ~<code>
}
```

这份配置在 comptime 使用。包含 comptime 回调的名义类型仍不能成为 quoted runtime 接口；包装也不能隐藏指针或尚不支持的可变捕获状态；runtime 函数身份作为数据保存，不改变其调用阶段。

只读捕获需要在闭包类型中声明环境，不能把有环境的闭包当作无捕获函数值传入：

```flurry
typealias Reader = %{flag: bool} comptime fn() -> bool;
comptime fn make_reader(flag: bool) -> Reader { || flag }
comptime fn read_twice(action: Reader) -> < |- bool > {
    let first = action();
    let second = action();
    '(first and second)
}
fn captured_result() -> bool {
    const yes = read_twice(make_reader(true));
    const no = read_twice(make_reader(false));
    ~<yes> and not ~<no>
}
```

生成器重建回调前递归验证捕获数据和闭包主体。捕获的只读回调也接受同样检查，转发不会重新从原变量取得值。快照验证支持标量、元组、匿名记录、结构体、枚举、newtype 及嵌套回调。名义类型实参按当前 Type 作用域验证，字段递归检查；泛型布局与 variant 身份由重建路径保留。回调自身放入名义包装和回调捕获名义对象均已接通，可变共享环境仍需后续实现。

例如，捕获配置对象的工厂可以保留泛型字段类型：

```flurry
struct Box where T { value: T }
comptime fn make_box_reader(value: T)
    -> %{data: Box<T>} comptime fn() -> T where T {
    let data = Box<T> { value: value };
    || data.value
}
comptime fn read_box(action: %{data: Box<T>} comptime fn() -> T)
    -> < |- T > where T {
    let value = action();
    '(value)
}
fn boxed_result() -> bool {
    const code = read_box(make_box_reader(true));
    ~<code>
}
```

嵌套名义对象不能隐藏可变回调或指针。即使外层闭包只调用某个字段，快照检查仍进入该字段保存的回调；其主体若修改自己的捕获状态，整个捕获图仍拒绝。

标记为 lambda 的 comptime 回调参数复用 tactic lambda 与 post do 路由，可以使用 `_0` 等参数占位符：

```flurry
comptime fn build_with(value: bool,
                      lambda action: comptime fn(bool) -> bool) -> <|-bool> {
    let result = action(value);
    '(result)
}

fn generated_tactic() -> bool {
    const code = build_with(false) do { not _0 };
    ~<code>
}
```

同一位置也接受 `build_with(false, not _0)`、显式 lambda 或函数值。计算型参数包可以与 post do 和 implicit 供应组合，回调槽不必在最后；尾随槽先确定，再推导剩余参数包的宽度。多个 lambda 槽导致尾随目标不唯一、占位符越界或调用 runtime 函数时拒绝。回调即使最终未调用也检查正文，但不会为了检查而执行不终止的正文。尚不能证明快照保持行为的捕获环境，以及 lambda 标记的 quoted-code 参数，仍未接通。

回调的参数类型可以先完成，返回类型则从主体继续推断，不必为整个函数类型先给出注解：

```flurry
comptime fn transform(lambda action: comptime fn(T) -> U, value: T)
    -> < |- U > where T, U {
    let result = action(value);
    '(result)
}
fn selected() -> .char'builtin {
    const code = transform(if _0 { 'a' } else { 'b' }, true);
    ~<code>
}
```

这里 `true` 提供 `T = bool`，回调主体随后确定 `U = char`。部分函数上下文同样用于普通无捕获 lambda 和收集包内的函数字段，例如参数包类型 `(comptime fn(bool) -> U, bool)`；嵌套元组与具名整包供应也保留此上下文。准备回调的表达式仍按源码顺序执行，推断不调用回调主体，未调用正文仍检查执行阶段。

直接收集变参时，每个元素检查完成后都会更新调用候选，后续元素立即获得新的类型上下文。例如下面的 lambda 确定 `U = u8`，随后 `200` 就按 `u8` 检查：

```flurry
comptime fn narrow(value: bool) -> .u8'builtin {
    if value { 200 } else { 0 }
}
comptime fn collect(...items: (comptime fn(bool) -> U, U))
    -> < |- U > where U {
    let (action, value) = items;
    let result = action(true);
    '(value)
}
fn collected() -> .u8'builtin {
    const code = collect(|value| narrow(value), 200);
    ~<code>
}
```

新取得的 Type 证据也可以推进子句默认计算，并用于后续交错供应的代码参数。这里只传递已检查元素的类型，不重复调用生产函数，也不提前执行后面的元素。

具名整包若直接供应元组字面量，也沿用逐字段检查：`collect(.items = (|value| narrow(value), 200))` 与上面的直接收集获得相同的类型上下文，不额外嵌套一层元组。字面量中的字段仍按源码顺序执行；`collect(.items = packet())` 或带语句的 `do` 块保留整体计算边界，不为获得字段类型而拆开或重复调用生产函数。



泛型回调写在数据实参之前时，后面的固有类型字面量可以先提供类型信息，无需重排求值：

```flurry
comptime fn transform(lambda action: comptime fn(T) -> T,
                      value: T) -> <|-T> where T {
    let result = action(value);
    '(result)
}

fn transformed() -> bool {
    const code = transform(not _0, false);
    ~<code>
}
```

命名函数值也能在实际准备之前提供完整函数类型，包括函数值的 comptime 阶段。显式泛型函数值和静态 Type 默认项可以代入签名，但预检不执行默认计算：

```flurry
comptime fn narrow(value: .u8'builtin) -> .u8'builtin { value }
comptime fn keep(code: < |- T >, action: comptime fn(T) -> T)
    -> < |- T > where T {
    code
}
fn narrow_code() -> .u8'builtin {
    const code = keep('(200), narrow);
    ~<code>
}
```

这里后面的 `narrow` 先确定 `T = u8`，前面的 quote 因而按 `u8` 检查数字字面量。读取函数签名不调用它；即使函数体发散，单纯传递且不调用这个函数值也不会因此发散。函数类型实参的约束和执行阶段仍在实际准备实参时检查，不能用预检绕过。

已知布局的内建字段也提供同样的证据。沿用上面的 `narrow` 与 `keep`：

```flurry
struct Config { action: comptime fn(.u8'builtin) -> .u8'builtin }
comptime fn config() -> Config { Config { action: narrow } }
fn configured_code() -> .u8'builtin {
    const code = keep('(200), config().action);
    ~<code>
}
```

预检先从 `config` 的返回类型取得字段布局，并据此检查代码字面量；`config()` 仍在其原实参位置求值一次。泛型结构体按类型实参实例化字段类型，匿名记录按字段名读取类型，字段本身若是元组包也可为展开提供宽度和元素类型。当前只读取内建拥有型字段布局，不据此调用 getter、选择方法或读取指针目标。实际字段访问继续检查生产函数的阶段、约束和契约。

对于已知函数类型的计算结果，调用的返回类型也可继续传递证据。例如下面的 `factory()()` 先提供 `(bool,)` 接口，因而可以为更早的 tactic lambda 确定参数类型：

```flurry
comptime fn packet() -> (bool,) { (false,) }
comptime fn factory() -> comptime fn() -> (bool,) { packet }
comptime fn transform_packet(lambda action: comptime fn(T) -> T, value: T)
    -> < |- T > where T {
    let result = action(value);
    '(result)
}
fn packet_result() -> bool {
    const code = transform_packet(not _0, ...factory()());
    ~<code>
}
```

配置字段中的回调、工厂返回的闭包以及具有共同函数类型的条件分支沿用此规则。预检不调用工厂或回调，也不选择分支。若返回类型仍依赖本次调用的普通实参值，则不能仅凭函数类型提前确定；实际调用的参数、阶段和契约检查始终保留。



类型信息预检不调用实参生产函数，不执行回调，也不对数值字面量提前选用默认类型；显式目标代码接口仍可指定 u8 等类型。字面 tuple、已有局部包的类型，以及返回类型不依赖尚未实例化形参的生产函数签名，都可在展开前提供元素类型：

```flurry
comptime fn packet() -> (bool,) { (false,) }

fn from_packet() -> bool {
    const code = transform(not _0, ...packet());
    ~<code>
}
```

同样的信息也适用于普通数据实参，并可来自函数值参数的调用签名：

```flurry
comptime fn from_producer(producer: comptime fn() -> bool) -> <|-bool> {
    transform(not _0, producer())
}

fn generated() -> bool {
    const code = from_producer(|| false);
    ~<code>
}
```

如果 if／when／match 的各分支已有相同的结果类型，也可以合并为类型证据，包括产生展开包的分支：

```flurry
comptime fn choose(flag: bool) -> <|-bool> {
    transform(not _0, ...if flag { packet() } else { packet() })
}
```

该预检不求值条件、主语、守卫或任何分支；实际执行只运行选中分支。普通检查仍验证所有分支、阶段限制和模式覆盖。分支信息缺失或类型不同则不从这里提供类型证据，不根据看似恒真的条件跳过另一分支。

显式供应完整子句实参时，预检也可替换生产函数的类型参数：

```flurry
comptime fn identity(value: T) -> T where T { value }

comptime fn explicitly_typed() -> <|-bool> {
    let Flag = bool;
    transform(not _0, identity<Flag>(false))
}
```

当前可读取 builtin 类型、指针／tuple 类型语法、普通 Type 参数和已完成的不可变局部 Type 值，并按普通子句规则对应位置或具名实参。不带泛型子句或约束的 typealias 可递归转发这些类型，例如 `typealias Flag = bool; typealias Enabled = Flag;`；预检不会执行别名右侧的 Type 生产函数。

函数类型也能作为这类静态证据：预检递归读取普通位置参数及返回类型，保留 `fn`／`comptime fn` 的阶段差别，并替换签名内的 Type 与值索引。例如生产函数显式供应 `comptime fn(Flag<flag>) -> bool` 后，外层自动 lambda 可以据此检查 `_0(Flag<flag>{})`。预检不调用这个函数值。闭包限定 `%{flag: bool} comptime fn() -> bool` 同时保留环境字段类型和顺序；环境绑定可以一致改名，空环境限定消去，但非空环境不能被省略或改成不同类型。带环境来源表达式或尚未支持的函数参数协议不由此提前接受，仍需正常类型检查。

生产函数的值子句同样可以提供签名证据。预检读取字面值、调用者的值子句及引用前序参数的静态默认值，按声明顺序完成依赖域；具名实参的书写顺序不改变这种依赖。例如：

```flurry
struct Flag where enabled: bool {}
comptime fn identity(value: Flag<flag>) -> Flag<flag>
where .flag: bool = true { value }
comptime fn build(lambda action: comptime fn(T) -> bool, value: T) -> <|-bool>
where T {
    let result = action(value);
    '(result)
}
fn run() -> bool {
    const code = build(do { _0; true }, identity<.flag=true>(Flag<true>{}));
    ~<code>
}
```

这里 `identity` 的已特化返回类型先为前面的 lambda 确定参数类型。省略 `.flag` 时，其静态默认值也提供相同证据。若值参数的域引用尚待默认补齐的 Type 参数，调用者值子句保留为符号输入，域完成后再检查；不会尝试执行尚未具现化的调用者参数。一般计算仍留给正常调用阶段，预检不提前执行生产函数或任意默认计算。值域、契约及输入与返回类型的完整检查仍不可省略。

静态 normal-form 应用也可提供类型证据：普通头支持混合 Type／值输入、具名可选输入，以及依赖前序参数的静态默认值。值输入可取字面值或当前值子句；默认值按声明顺序形成，定宽数值仍检查其域。Tuple 额外支持类型列表展开和 `.Es=...` 整包供应。`Tuple<bool, .char'builtin>` 始终保留 Tuple 头，其输入是一个 `TypeListValue`；不会因为 Tuple 的定义正文返回 bool 就与 bool 相等。类型列表可来自字面量或已完成的不可变局部列表。未知的列表计算仍留到正常调用阶段。生成器转调时，normal-form 参数计划保留已有符号值，不为了获得类型证据而读取尚未具现化的局部参数。省略的类型槽可使用静态默认类型，包括引用前序参数的默认值；预检不执行任意 Type 生产函数，也不展开 normal-form 的头。生产函数的类型约束、参数契约及执行阶段仍由实际调用检查。

无显式类型实参的泛型生产函数，也可从已有参数类型、固有类型字面量及嵌套调用的结果取得推断证据：

```flurry
comptime fn inferred() -> <|-bool> {
    transform(not _0, identity(identity(false)))
}
```

此处复用普通调用的类型推断器，尚未解出的变量不会进入生成器接口。数值字面量不在预检中默认成 Integer，目标代码接口仍可指定 u8。生产函数的变参收集、字面 tuple 展开及具名整包供应也可提供信息；空包和异构包均适用，已注册 Tuple 时保留其 normal-form 头。可选 Type／值子句中的静态默认值在推断前固定，依赖前序参数的默认值随该参数完成；例如 `.mirror: T = tag` 保留前序 `tag: T` 的待推导身份，不另建一个可任意选择的参数。固定的布尔默认值也不会因实参冲突而改变。需要执行生产函数的默认值，仍走正常调用检查，必要时需要显式接口。计算型展开包若有已知元素类型，也能参与推断；多个包按源码顺序组成收集类型，包本身仍在原位置执行一次。数据参数默认值、生产函数正文及契约不会为了获得类型证据提前执行。

```flurry
comptime fn second(first: T, second: U) -> U
where T, .U: .Type'builtin = T {
    second
}

fn defaulted() -> bool {
    const inferred = transform(not _0, second(true, false));
    const explicit = transform(not _0, second<bool>(true, false));
    ~<inferred> and ~<explicit>
}
```

两次调用的 U 都沿用 T，不能根据第二个实参另行推断。显式 `.U=...` 覆盖时跳过原默认表达式；即使原默认计算不终止，已覆盖的调用也不执行它。



```flurry
comptime fn pack(...items: T) -> T where T { items }

comptime fn inspect(lambda action: comptime fn(T) -> bool,
                    value: T) -> <|-bool> where T {
    let result = action(value);
    '(result)
}

fn packed() -> bool {
    const code = inspect(do {
        let (flag, letter) = _0;
        not flag and letter == 'x'
    }, pack(false, 'x'));
    ~<code>
}
```

pack 的元素类型先决定整个收集包的类型，再为 inspect 的回调提供参数类型；各元素的实际生产过程仍在原位置执行一次。

例如 `transform(not _0, identity(...packet()))` 可以从 packet 的 `(bool,)` 返回接口取得元素类型，推断 identity 的结果为 bool，再检查回调。这里不会为了推断而调用 packet；它的执行阶段和参数契约仍在正常准备实参时检查。类型尚未取得的计算型包不会被假设成某种元素类型。


属性收集也可以提供匿名记录类型，字段名和顺序随类型证据保留：

```flurry
comptime fn schema(...properties: T) -> T where T { properties }

fn configured() -> bool {
    const code = inspect(_0.enabled and _0.letter == 'x',
                         schema { enabled: true, letter: 'x' });
    ~<code>
}
```

这里只读取属性表达式已有的类型，或不依赖目标类型的字面量类型；不会提前计算属性。属性入口使用普通调用的选择规则，实际调用仍检查重复属性、字段访问、契约和执行阶段。


producer 不会在检查 transform 的回调前执行。泛型生产函数若返回固定的 bool 或 `(bool,)`，其结果也可参与推断；若结果仍包含该生产函数未实例化的类型参数，则不能把它当作调用者的类型证据。

这里先从 packet 的返回类型取得 bool，再检查前面的回调。packet 仍在其原源码位置执行一次；它的前置条件、阶段检查及状态更新不会被类型预检跳过。预检可以读取块尾表达式的返回类型，并沿块内简单按值绑定、内建元组解构以及结构体／匿名记录字段解构传递已知类型。静态类型注解可以提供数字字面量所需的目标类型；没有注解时，只读取初始化表达式已有的类型证据，不提前选择数字默认类型。嵌套块按各自的绑定身份处理，预检结束或出错后恢复外层类型环境。整个过程不执行块内语句。需要实例化泛型或代入生产函数形参才能取得的包类型，仍可能要求显式类型实参或代码接口注解；这尚不是任意表达式之间的完整双向推断调度。

例如，局部注解可以跨块边界为前面的 tactic lambda 提供参数类型：

```flurry
comptime fn map_value(lambda action: comptime fn(T) -> T, value: T)
    -> < |- T > where T {
    let result = action(value);
    '(result)
}
fn annotated_block() -> .u8'builtin {
    const code = map_value(_0, do {
        let value: .u8'builtin = 200;
        value
    });
    ~<code>
}
```

初始化表达式仍按正常路径检查和求值；若这里把 `200` 改为 `256`，已有的 `u8` 类型证据不能使越界初始化合法。

字段解构会保留泛型字段的实例化类型，并支持重命名和与元组模式嵌套。沿用前面的配置例子，`keep('(200), do { let {action: selected} = config(); selected })` 也可以为 quote 确定 `u8` 上下文。预检只读取内建布局，不执行模式协议、guard 或生产函数；未知字段、重复字段和其它无效模式仍由正常模式检查拒绝。

match 分支同样可以使用已知 subject 类型，为内建元组和记录模式中的绑定取得类型。预检按分支隔离绑定，只有各分支结果类型相同才向外提供证据。例如下面的两个 `action` 属于各自的分支，但具有同一函数类型：

```flurry
comptime fn narrow_pair() -> (comptime fn(.u8'builtin) -> .u8'builtin, bool) {
    (narrow, true)
}
fn matched_callback() -> .u8'builtin {
    const code = keep('(200), narrow_pair() match {
        (action, true) => action,
        (action, false) => action,
    });
    ~<code>
}
```

这里沿用前面定义的 `narrow` 与 `keep`。预检不运行 subject、guard 或模式计算，也不根据已知值选择某一分支；实际 match 继续检查覆盖性、模式合法性和分支类型。

已知枚举类型也提供载荷布局，包括泛型实例化、命名字段和嵌套解构。下面两个分支都向前面的 quote 提供 `u8` 类型证据：

```flurry
enum Callback where T {
    direct(comptime fn(T) -> T),
    configured { action: comptime fn(T) -> T, enabled: bool },
}
comptime fn callback() -> Callback<.u8'builtin> {
    Callback<.u8'builtin>.configured { action: narrow, enabled: true }
}
fn enum_callback() -> .u8'builtin {
    const code = keep('(200), callback() match {
        .direct(action) => action,
        .configured { action } => action,
    });
    ~<code>
}
```

此处支持根据上下文枚举类型解析的变体模式。取得载荷类型不执行 `callback()`；它仍在实际参数求值时执行一次。预检不使缺失分支、重复字段、未知字段或错误的载荷形状合法。

模式组合也保留这些证据。`(action, _) as whole` 同时提供内层回调与整个元组的类型；`or` 模式只导出各候选一致的绑定类型。例如沿用上面的定义，可以写：

```flurry
fn combined_callback() -> .u8'builtin {
    const code = keep('(200), callback() match {
        .direct(action) or .configured { action } => action,
    });
    ~<code>
}
```

候选分别使用临时绑定环境，最终绑定身份与正常模式 elaboration 一致。不同候选的同名绑定类型不一致时，不提供该绑定的提前类型证据；正常模式检查仍拒绝类型冲突和绑定名称不一致。`as` 不改变内部模式的匹配条件，带 guard 的模式也只从内部结构取得证据，guard 留在实际匹配过程中检查和执行。






生成器的 implicit 参数现在可通过 `.name = value` 显式供应，数据与表达式代码都适用。普通位置实参与展开包不会占用 implicit 槽；泛型展开包的目的布局也排除这些槽。显式 `comptime` 参数标记可以保留，生成器输入本来就必须在构造阶段取得。

```flurry
pure comptime fn enabled_code(implicit comptime enabled: bool,
                             .code: <|-bool> = '(enabled)) -> <|-bool> {
    code
}

fn selected() -> bool {
    const code = enabled_code(.enabled = true);
    ~<code>
}
```

具名供应仍按源码位置求值，随后计算缺省项和验证调用契约。这里的 implicit 是供应入口，不授予 quoted runtime 上下文的捕获或替换权限。缺少供应、重复供应及 runtime 值越阶段传入均拒绝；自动 implicit 供应仍未实现。其它生成器参数协议尚未接通。

生成器的普通参数支持具名可选入口及默认表达式，数据与表达式代码均可作为默认值。显式实参按源码顺序计算一次；随后按参数顺序计算缺省项，默认值可使用前序参数，不能读取自身或后序参数。覆盖默认值会跳过其执行，定义检查仍核验所有默认值的类型与阶段。这里执行的是 comptime 默认计算，遵循 comptime invocation 政策，不套用 runtime 默认值的 P3 限制。

~~~flurry
pure comptime fn select(
    first: < x: T |- T >,
    .second: < x: T |- T > = first,
) -> < x: T |- T > where T {
    second
}

fn selected(value: bool) -> bool {
    const generated = select('[input = value](value));
    ~<generated>
}
~~~

默认代码沿用前序代码的绑定对应，不复制其 runtime 资源。默认 quote 可以使用自身接口声明的虚拟名字，这些名字只在该默认表达式中可见。省略具有发散默认计算的参数仍会耗尽编译时预算；显式覆盖该参数不运行被跳过的默认计算。数据变参可按普通位置收集规则组成元组，支持空包、异构包和 `.name = packet` 整包供应；有绑定的 Tuple 构造器时保留其 normal-form 身份。收集包仍是一个数据参数，其声明类型约束整个包。位置元素与具名代码实参交错时按原源码顺序计算，再完成收集。元组字面量实参可使用 `...(a, b)` 展开：元素直接进入原参数检查及收集路径，保留目标类型和交错求值顺序。普通元组表达式也可展开：参数包在原源码位置完成一次 comptime invocation，元素冻结后按顺序供应，空包仍执行原表达式的检查。包宽度由已有元组接口确定；只有一个未知宽度且目标位置数量唯一时，也可由目标布局确定宽度。多个未知宽度不能任意分配，开放变参没有唯一宽度时需要明确包接口。代码容器及自动供应协议仍未接通。

~~~flurry
pure comptime fn pack(...items: T) -> < |- T > where T { '(items) }

fn packed() -> bool {
    const code = pack(true, 'a');
    let (enabled, letter) = ~<code>;
    enabled and letter == 'a'
}
~~~

普通包生产者同样保留收集语义：

~~~flurry
pure comptime fn packet() -> Tuple<bool, char> { (true, 'a') }

fn expanded() -> bool {
    const code = pack(...packet(), false);
    let (enabled, letter, extra) = ~<code>;
    enabled and letter == 'a' and not extra
}
~~~

`packet()` 只在编译时求值一次；结果元素与额外的 false 一起交给 pack。包中的数据不能冒充 quoted code 证据，runtime 包生产者也不能因出现 `...` 而提前执行。

`pack(.items = (true, 'a'))` 供应相同的完整包，不会多包一层元组。收集类型不能仅写成某个元素类型，也不能把收集后的元组解释成 Type 值。

生成器直接收集变参时，完整的元素类型也可以为前面的回调提供上下文，不必先写一个独立的 `pack` 调用：

```flurry
comptime fn inspect(lambda action: comptime fn(T) -> bool, ...items: T)
    -> < |- bool > where T {
    let result = action(items);
    '(result)
}
fn checked() -> bool {
    const code = inspect(
        do { let (flag, letter) = _0; flag and letter == 'a' },
        true, 'a'
    );
    ~<code>
}
```

空包、具名整包、属性记录及已有返回类型的展开包可以提供相应的集合证据；有注册 Tuple 构造器时保留其 normal-form 头。预检只取得类型，不执行元素生产函数，也不为缺少上下文的数字字面量选择默认数值类型。实参仍在源码位置检查并求值，runtime 调用不能借预检进入 comptime。集合的部分类型信息也可以参与推断。例如收集参数声明为 `(T, u8)` 时，布尔字段足以确定 `T = bool`，另一个数字字段继续等待 `u8` 上下文：

```flurry
comptime fn inspect_number(
    lambda action: comptime fn(T) -> bool,
    ...items: (T, .u8'builtin)
) -> < |- bool > where T {
    let (value, number) = items;
    let result = action(value) and number == 200;
    '(result)
}
fn checked_number() -> bool {
    const code = inspect_number(not _0, false, 200);
    ~<code>
}
```

这类证据可以递归通过嵌套元组，并推进依赖已知 Type 的默认计算。未知部分只存在于调用候选中，不作为带推断变量的 NIR 类型发布。尚不能由字段、参数声明或目标接口确定的类型仍需要注解；这不意味着任意循环依赖都能自动解开。


扩展调用中的 `name: value` 属性已能收集到生成器的数据变参，组成匿名记录。它与 `.name = value` 的参数整包供应不同：属性名属于记录字段，而不是生成器的形参名。

~~~flurry
pure comptime fn configure(...properties: T) -> < |- T > where T {
    '(properties)
}

fn configured() -> bool {
    const code = configure { enabled: true, letter: 'a' };
    let config = ~<code>;
    config.enabled and config.letter == 'a'
}
~~~

属性可与普通位置实参、元组展开和代码实参交错，各项仍按源码顺序计算一次；收集不会把分散的属性提前到同一个求值位置。记录字段接受声明 schema 提供的目标类型，显式 Type 实参和目标代码接口也可帮助确定收集入口。当前需要唯一的合格记录收集入口，重复属性或多个不能消歧的入口会报错。生成器属性调用仍限于编译时代码构造位置，不能通过扩展调用语法在 runtime 执行。

生成器的 if、when、match 和提前返回路径统一检查代码结果与 runtime 上下文接口。局部代码绑定不能只按第一分支推导，然后把另一分支所需的上下文隐藏起来；即使生成器未调用，也会拒绝这类不兼容分支。接口兼容的上下文改名仍按既有对应规则处理，不把固定依赖变成可替换端口。

生成器还支持 Type 泛型子句、trait／类型命题约束和具名 Type 默认值。`where T` 和 `where T: Type` 使用相同的类型参数身份；显式域也可写成 Type 的别名。类型参数可从数据类型、输入代码的结果与上下文类型、目标代码接口推导，也可以显式指定。实例化同时替换输入接口、输出接口与正文的类型绑定；嵌套生成调用各自保存类型环境，normal-form 头不展开。

显式 `comptime fn` 生成器也支持可冻结数据的值子句，包含标量、聚合值及 `T, value: T` 这样的依赖值域。值子句在构造阶段取得，可以选择生成分支或提升为 quoted 主体中的常量，不会成为新的 runtime 上下文端口：

```flurry
comptime fn configured() -> < input: bool |- bool >
where .inverted: bool = false {
    if inverted {
        '(not input)
    } else {
        '(input)
    }
}

fn apply(value: bool) -> (bool, bool) {
    const direct = configured();
    const inverse = configured<.inverted = true>();
    (~[input = value]<direct>, ~[input = value]<inverse>)
}
```

位置值子句可以通过尖括号显式供应，也可以从数据或代码接口中的类型身份推导；具名可选子句沿用默认值规则；默认计算可以使用前序子句，显式覆盖会跳过其执行。未调用的定义仍检查默认值、主体的所有分支和阶段边界；实际生成继续检查程序契约。嵌套生成调用保存各自的数据环境，数据在进入 quote 前冻结，运行时输入不能借值子句越阶段传入。

值默认计算也会等待前序值子句本身确定；表达式具有已知的 `bool` 类型，不代表它引用的布尔值已经取得。

```flurry
struct Flag where enabled: bool {}
comptime fn enabled(code: <|-Flag<flag>>) -> <|-bool>
where flag: bool, .fallback: bool = flag {
    if fallback { '(true) } else { '(false) }
}
fn run() -> bool {
    const yes = enabled('(Flag<true>{}));
    const no = enabled('(Flag<false>{}));
    ~<yes> and not ~<no>
}
```

这里从 quoted code 的结果类型推导 `flag`，不执行 quoted 主体；随后才计算 `fallback`。同一值子句若从两个输入得到互相冲突的值，调用会被拒绝。

普通数据参数中的值索引也可提供推导证据，包括经过泛型 comptime 生产函数和计算型参数包转交的值。例如：

```flurry
comptime fn make(value: Flag<flag>) -> Flag<flag> where flag: bool { value }
comptime fn enabledData(value: Flag<flag>) -> <|-bool> where flag: bool {
    if flag { '(true) } else { '(false) }
}
fn runData() -> bool {
    const code = enabledData(make(Flag<true>{}));
    ~<code>
}
```

类型预检保留 Type 与值参数的种类，读取可静态取得的类型身份、字面值索引和已有子句绑定。泛型包装中尚未具体化的值保留原子句身份；实际调用时使用当前实例的替换环境，因此连续生成 `true` 与 `false` 两个实例不会共用旧绑定。定宽数字按声明域检查；一般值计算仍由正常调用检查取得，预检不额外调用生产函数。


后续值子句的声明域可以依赖前序值索引，默认值也在替换后的域中检查：

```flurry
comptime fn witnessed() -> <|-bool>
where flag: bool, .witness: Flag<flag> = (Flag<flag>{}) {
    if flag { '(true) } else { '(false) }
}
```

`witnessed<true>()` 的默认见证类型是 `Flag<true>`，显式供应 `Flag<false>{}` 会被拒绝。前序输入按声明顺序可见，不能用后声明的值参数形成较早的域。

值索引也可以从代码上下文端口推导。显式暴露的 `x: Flag<flag>` 端口保留其完整类型身份，接入时不能换成不同索引的 runtime 绑定；普通固定捕获仍不能被提升为可替换端口。

值默认计算可以等待普通实参类型或目标代码接口确定 Type，再按子句声明顺序完成；后续 Type 默认项也可以使用此前已完成的值默认项。例如，目标代码接口可以先确定 T，随后选择相应的 comptime trait 方法取得默认数据：

```flurry
trait Initial {
    comptime fn initial() -> Self;
}

impl Initial for bool {
    comptime fn initial() -> bool { true }
}

comptime fn build() -> < |- T >
where T :- Initial, .value: T = T.as(Initial).initial() {
    '(value)
}

fn enabled() -> bool {
    const code: < |- bool > = build();
    ~<code>
}
```

默认项依赖探查只读取类型与绑定信息，不调用其生产函数。所需 Type 信息就绪后，实际默认计算可在检查后续实参之前完成，为数字字面量、quoted code 参数和展开包提供上下文；同一次调用复用已完成默认项。例如：

```flurry
typealias Type = .Type'builtin;
comptime fn is_bool(kind: Type) -> bool { kind == bool }
comptime fn Width(flag: bool) -> Type {
    if flag { .u8'builtin } else { .u16'builtin }
}
comptime fn choose(code: < |- U >, first: T) -> < |- U >
where T, .flag: bool = is_bool(T), .U: Type = Width(flag) {
    code
}
fn widths() -> (.u8'builtin, .u16'builtin) {
    const narrow = choose('(200), true);
    const wide = choose('(500), 'a');
    (~<narrow>, ~<wide>)
}
```

此处后面的 `true` 与 `'a'` 先提供固有类型信息，默认链确定 `U` 后才检查前面的代码字面量。`choose('(256), true)` 必须拒绝，不能为容纳字面量而覆盖默认得到的 `u8`。未调用的生成器和被显式覆盖的默认项仍接受执行阶段检查，不能隐藏 runtime 调用。

尚不提供一般的值子句反向推断。若尚未完成的默认项本身决定检查实参所需的类型或代码接口，仍可能要求显式子句实参；这也不开放 quote fn 的值泛型子句。

~~~flurry
pure comptime fn forward(code: < x: T |- T >) -> < x: T |- T > where T {
    code
}

pure comptime fn compose(code: < x: T |- T >) -> < x: T |- T > where T {
    forward(code)
}

fn original(value: bool) -> bool {
    const generated = compose('[input = value](value));
    let value = false;
    ~<generated>
}
~~~

`original(true)` 返回 true。类型推导不授予新的绑定对应：显式暴露的端口可以适配接口名，固定依赖仍不能被转换成任意可替换输入。Type 子句默认值沿用普通函数规则，不被调用推导覆盖；所有类型约束先核验，才运行本次生成器。目标接口可以为 quoted 数值字面量供应类型。已保存代码值和数据绑定的类型也会在实参计算前参与推导；前序实参完成后，其类型信息继续传给后续代码参数和变参元素。这里只提前读取已有类型证据，不提前执行生成表达式或读取 runtime 数据，也不改变原有上下文对应。未解出的类型不会进入生成程序；需要后序任意计算结果才能确定的类型，仍可能要求显式类型实参或目标接口。上述支持尚不包括上下文多态或 `implicit quote` 的自动供应。

结构化配置可以选择生成结果，也可以在 quote 中提升已完成的字段数据：

~~~flurry
struct Options { invert: bool }

pure comptime fn configured(options: Options) -> < x: bool |- bool > {
    let {invert} = options;
    if invert { '(not x) } else { '(x) }
}

fn negate(value: bool) -> bool {
    const code = configured(Options { invert: true });
    ~[x = value]<code>
}
~~~

数据局部可以使用不可反驳的 let 模式直接解构，也可以使用普通 match 检查 enum 载荷。let 解构复用普通模式规则，包含嵌套结构、析取绑定重映射及顺序模式计算；未调用的生成器也检查字段、类型、覆盖和阶段。数据计算不能提前读取 quote 的 runtime 上下文或调用 runtime 函数。把 runtime 函数身份交给 comptime 辅助函数本身不等于调用该函数；最终提升的数据仍须通过不可变值冻结。

生成器的 `match` 既可选择表达式代码，也可驱动构造程序的控制流。主语只求值一次，模式和守卫复用普通匹配器；只有成功分支的绑定进入该分支，未选择的分支仍接受定义检查。

~~~flurry
enum Step { skip, wrap(bool), stop }

pure comptime fn generated(first: Step) -> < x: bool |- bool > {
    var step = first;
    var code: < x: bool |- bool > = '(x);
    while true {
        step match {
            .skip => { step = .wrap(true); continue; },
            .wrap(invert) => {
                const previous = code;
                code = if invert { '(not ~<previous>) } else { previous };
                step = .stop;
            },
            .stop => { return code; },
        };
    }
}
~~~

`generated(.skip)` 生成取反，`generated(.stop)` 保留原代码。分支中的 `return`、`break`、`continue` 作用于构造程序，quote 内部的控制流仍作用于生成后的 runtime 程序。尾部 match 的所有有限路径均返回代码时，不需要额外尾表达式；存在漏返回、类型不兼容、覆盖不足或阶段非法的分支则拒绝。各分支可以解构不同类型的载荷，匹配器传递绑定所用的内部类型不发布到 runtime 产物。

生成器的数据与代码局部支持 `var` 和整值赋值，可以逐步包裹已有代码。代码别名保存原值，赋值不会改动已经保存的别名：

~~~flurry
pure comptime fn twice() -> < x: bool |- bool > {
    var code: < x: bool |- bool > = '(x);
    const original = code;
    code = '(not ~<original>);
    code = '(not ~<code>);
    code
}
~~~

赋值维持变量原有的结果类型和上下文接口；兼容代码的接口改名可以适配，默认对应随被赋入的代码值传递。推导出的固定依赖只能接到原有对应，不能凭赋值新增或任意替换 runtime 输入；对应有歧义时需要显式端口。嵌套构造块内对外层变量的更新保留，新声明的局部不泄漏。

`while` 已能驱动代码累积，支持语句 if、嵌套循环、带标签的 break/continue、循环内提前返回和数据局部复合赋值：

~~~flurry
pure comptime fn invert_times(count: u8) -> < x: bool |- bool > {
    var remaining = count;
    var code: < x: bool |- bool > = '(x);
    while remaining > 0 {
        const previous = code;
        code = '(not ~<previous>);
        remaining -= 1;
    }
    code
}
~~~

零次迭代保留初始代码。未调用的生成器只检查循环体，不运行循环；调用时，迭代、环境快照和代码复制共同消耗构造预算，超限不发布部分结果。quote 内的 runtime 循环与生成器循环保持不同控制目标。

局部代码绑定的显式 `< Ψ |- T >` 注解同样用于选择代码构造检查路径。构造块可以通过 while 或 inline for 中的 return 交付代码，不必在末尾重复写一个 quote。例如：

```flurry
pure comptime fn build() -> <|-bool> {
    let code = do {
        inline for index in 0..1 { return '(true); }
    };
    code
}
```

这只识别结果的代码类别，不证明路径一定返回。空范围和可能零次执行的循环仍须提供后续结果。显式代码接口也允许定义端检查没有有限出口的构造块，例如 `let code: <|-bool> = do { while true {} };`；检查不执行它，实际调用仍受构造预算约束。

控制流检查区分继续、返回、各个循环目标的 break/continue，以及没有有限出口的路径。所有能结束的路径都返回代码时，可以省略尾表达式，例如 `while true { return '(true); }`；可能跳出循环却不返回代码则拒绝。不会结束的定义不是漏返回，但实际调用仍会受构造预算限制。检查不运行生成器来证明终止，只对布尔字面量及 not/and/or 作有限的控制流判断；未取得依据的条件保留两条路径，不会跳过不可达代码的类型与阶段检查。协议驱动的普通 `for` 生成尚未接通。生成器已支持有界 Integer 范围上的 `inline for`：

```flurry
typealias Integer = .Integer'builtin;

pure comptime fn invert_times(count: Integer) -> <|- bool> {
    var code = '(true);
    inline for index in 0..count {
        code = '(not ~<code>);
    }
    code
}

fn run() -> bool { ~<invert_times(3)> }
```

范围端点先取得完成的 comptime Integer 值，按升序迭代；支持 `..=`，空范围和反向范围执行零次。循环索引及不可反驳模式产生的绑定在每次迭代中独立建立，不能被赋值，离开迭代后不泄漏。可以用 `inline for index if index != 1 in 0..count` 过滤，也支持嵌套循环、带标签的 break/continue 和提前返回代码。空范围仍检查模式、守卫和循环体，包括模式内部 `and … is …` 调用的执行阶段；即使生成器未被调用，其中的 runtime 调用也会被拒绝。实际构造不执行空范围的模式计算、守卫或循环体，合法但不终止的 comptime 模式计算因此不会在空范围被调用。定义侧只从已检查的整数常量边界取得非空证据，不调用端点函数，也不为抽象索引选取样本值。范围确定非空、守卫确定为真且循环体所有路径都返回代码时，可以省略尾返回，例如 `inline for index in 0..1 {return '(true);}`。空范围、未知边界、可能不命中的守卫，以及 body 中正常完成、break 或 continue 的路径，仍要求相应的后续代码返回；指向外层循环的转移按原目标传播。展开次数、端点算术、模式选择和代码复制共享构造预算；当前每个范围最多展开 100,000 次。生成器也支持非空 tuple 的 `tuple_enumerate`，见下例。

```flurry
pure comptime fn toggleSelected(values: (bool, bool, bool)) -> <|-bool> {
    var code = '(false);
    inline for (index, element) if index != 0 in values'tuple_enumerate {
        if element { code = '(not ~<code>); }
    }
    code
}
```

输入 tuple 在实际构造时求值一次，每个元素按照自己的类型检查循环体，索引与元素绑定不可赋值；异构 tuple 不被收窄为共同元素类型。定义检查不为元素编造值，也不会为了验证生成器而调用输入函数。过滤、continue、break、外层标签转移和提前返回沿用范围遍历规则；被过滤或先前返回遮蔽的元素仍检查类型与执行阶段。无过滤的非空 tuple 若必然返回代码，可以省略尾返回。输入与展开工作计入共享构造预算。空 tuple 的循环体检查规则仍待确定，当前明确拒绝。


生成器支持按顺序选择的 `when`，既可选择代码值，也可执行赋值、返回和循环转移：

```flurry
pure comptime fn choose(first: bool, second: bool) -> <|- bool> {
    let code = when {
        first => '(false),
        second => '(true),
        else => '(false),
    };
    code
}

pure comptime fn early(enabled: bool) -> <|- bool> {
    when {
        enabled => return '(true),
    }
    '(false)
}

fn run() -> bool { ~<choose(false, true)> }
```

生成器中的 `inline if`、`inline when` 和 `inline if value is do { … }` 也可选择代码值，支持局部初始化和代码实参，例如 `let code = inline if enabled { '(true) } else { '(false) };`。其构造决策仍在 comptime 完成；inline 标记不跳过未选分支的类型、阶段或代码接口检查。作为语句时同样保留 return、break、continue 和无 else 的继续路径。

调用时只求值到首个命中的条件，后续条件和分支体不运行；定义检查仍检查所有条件与分支体的类型和阶段。分支局部绑定不逃出分支，对外层可变绑定的赋值则保留。产生代码值的 when 必须带 else；不带 else 的 when 遵循语句规则，未命中时继续执行。若有限可达路径全部返回代码，也可以不写尾表达式，例如 `when {false => return '(false), true => return '(true)}`。生成器 while 内的 when 可执行 break 或 continue，控制目标仍属于生成器，不进入 quote 主体。

生成器也支持 `let-else`／`var-else`，包括模式守卫。成功匹配的绑定进入后续构造作用域，失败候选的绑定不进入 else；else 必须返回、跳转循环或不终止，不能继续到绑定使用处。`var` 的成功绑定可以重新赋值。

代码值也可以使用带 guard 的整体绑定，支持名字、`_` 和 `as` 别名：

```flurry
comptime fn choose(enabled: bool) -> <|-bool> {
    let selected if enabled = '(true)
        else { return '(false); };
    selected
}

fn yes() -> bool { ~<choose(true)> }
fn no() -> bool { ~<choose(false)> }
```

初始化器先构造一次代码值，随后在 comptime 求值 guard。guard 失败不会撤销初始化器或 guard 已经完成的外层修改，但候选绑定不进入 else；同名外层绑定在 else 中仍指向原值。初始化只构造 quote，不执行其 runtime 主体。成功的代码绑定保留上下文端口与默认对应，`var` 可重新赋值；`let` 仍不可重新赋值。定义期检查失败分支的接口、阶段和发散性，即使 guard 是常量 true。这里仍只绑定整个代码值，不提供对代码内部结构的模式解构。

匹配对象及成功绑定可以包含 invocation 内的活动指针，不要求先冻结成常量。初始化表达式只求值一次；模式不匹配时跳过其 guard，guard 返回 false 后已经发生的外层数据修改仍对后续候选或 else 可见。成功绑定带着原来的指针目标进入后续作用域：

```flurry
comptime fn build(enabled: bool) -> <|-bool> {
    var flag = false;
    let (pointer, true) = (flag.ref, enabled)
        else { return '(false); };
    pointer.* = true;
    '(flag)
}
```

普通生成器 match 使用同一活动数据帧。选中 arm 的指针绑定仍不能借 quote 逃逸成 runtime 指针；候选绑定也不会泄漏到其他 arm 或失败分支。


~~~flurry
enum Input { none, some(bool) }

pure comptime fn configured(input: Input) -> < x: bool |- bool > {
    let .some(enabled) = input else { return '(x); };
    '(x and enabled)
}
~~~

`.none` 生成原输入，`.some(false)` 生成与 false 的合取。即使从未调用，定义中的 else 也检查阶段与结果类型；不会执行其中的无限循环。实际调用若进入不终止的 else，仍按构造预算失败。此处只解构完成的数据，不把代码见证伪装成普通可匹配的数据对象。

完成的数据支持 struct／匿名记录的嵌套字段赋值与复合赋值，以及 BitVec 字段视图的普通写回。更新复用普通位置检查，在独立 comptime 调用帧中执行，成功后再保存完成的数据；不会修改已创建 quote 保存的数据快照。`let` 的字段写入沿用[绑定规则](../resource-management/bindings.md)，不把绑定不可重赋值误解为对象只读。写入 runtime 上下文仍拒绝。显式指针可以在本次数据更新帧内临时形成并写入，包括条件选址、字段投影、整值替换和 BitVec 写回；目标先定位一次，再按普通赋值规则执行右侧。聚合值通过指针读取或替换时仍须满足 Copy/Drop 协议检查，不因处于生成器中而豁免。活动指针可以保存在生成器局部 `let`／`var` 中，通过 invocation 局部存储跨语句使用；它不是可冻结常量。BitVec 索引、切片及嵌套切片也支持复合赋值：位置与选择器按源码顺序准备一次，随后读取旧片段、执行右侧和所选运算协议，再写回原位置。右侧对同一对象其它位的修改予以保留。运算协议必须能在 comptime 执行；BitVec 不因支持写回就自动取得 Integer 算术。动态索引继续进行边界检查；局部指针不作为运行时代码或常量逃逸，所有者离开作用域后不能继续解引用。

例如，临时指针可以在两个拥有的数据根之间选址，写回后两者都仍可用于构造代码：

~~~flurry
struct Flag { enabled: bool }

pure comptime fn choose(select: bool) -> < |- bool> {
    let first = Flag { enabled: false };
    let second = Flag { enabled: false };
    (if select { first.enabled.ref } else { second.enabled.ref }).* = true;
    '(first.enabled == select and second.enabled != select)
}

fn run() -> bool { ~<choose(true)> and ~<choose(false)> }
~~~

这里的指针只在赋值帧内存在；若改成先用 `let pointer = first.enabled.ref` 保存到生成器环境，冻结阶段会拒绝。quote 捕获的是更新后完成的数据，不是活动存储地址。

赋值更新帧也保存本次选择器和右侧引用的其它数据绑定。整个表达式按普通求值顺序执行，所有结果冻结成功后才一起写回生成器环境；失败时不发布部分状态。这个规则同时适用于直接 var 赋值、字段赋值与 BitVec 复合写回：

```flurry
struct State { flag: bool }

pure comptime fn update_pair() -> <|-bool> {
    let first = State { flag: false };
    let second = State { flag: false };
    const before = '(not second.flag);
    first.flag = do {
        second.flag = true;
        second.flag
    };
    if first.flag and second.flag { before } else { '(false) }
}
```

实际构造后 first 与 second 都已更新，而 before 继续保存创建时的 false。更新帧只复制本次表达式引用的数据根，复制、执行及冻结仍受构造预算限制；右侧的嵌套写入不能绕过 let 的重新赋值限制或修改 quoted runtime 上下文。

同一状态帧也用于生成器的数据初始化、if／when／while 条件、调用实参与展开包、match 主语和 inline 范围端点。每个实际求值的表达式交付结果及更新后的数据，后续表达式读取新状态；`and`／`or` 短路、未选分支和 when 后续条件仍不执行。

```flurry
struct Counter { count: u8 }

pure comptime fn count_conditions() -> <|-bool> {
    let state = Counter { count: 0 };
    let ready = do { state.count += 1; true };
    while do { state.count += 1; state.count < 3 } {}
    '(ready and state.count == 3)
}
```

实现上，相邻数据求值复用与当前快照匹配的局部存储；进入求值时检查已知标量及记录字段的快照事实，让普通契约验证继续成立。求值或数据冻结失败时恢复存储内容与所有权状态，成功后一起发布更新的快照。旧 quote 仍保留构造时的值；局部指针绑定与指针参数可以在生成器正文的多条语句之间使用，原所有者必须保持存活；包含指针的元组、记录、枚举 payload 和 newtype 可作为局部数据及普通参数传递；变参收集、属性包及展开包同样保留活动指针。包生产表达式仍按源码位置求值一次，展开不复制指针目标，也不允许将这些指针嵌入 runtime code。

定义检查仍只检查类型与阶段，不运行这些更新。普通局部 const 的闭包图快照使用原有冻结机制，不因生成器状态帧而改成普通数据冻结。

不可变局部绑定可以保留已检查表达式中明确的 Type 值，用于后续类型注解和泛型实参。嵌套 tuple／记录解构与整体别名也保留这些信息；记录字面量及已完成的记录常量还支持直接字段投影：

```flurry
pure comptime fn identity(value: T) -> T where T { value }

pure comptime fn typed_code() -> <|-bool> {
    let (Word, Flag) = (.u8'builtin, bool);
    let value: Word = 17;
    let copied = identity<Word>(value);
    let enabled: Flag = true;
    '(enabled and copied == 17)
}
```

例如 `let {word: Word} = Description {word: .u8'builtin};` 和 `let Word = Description {word: .u8'builtin}.word;` 都能交付已知 Type，前提是 Description 已声明相应 Type 字段。字段对应保留记录声明身份与字段索引，匿名记录还核对字段名。取得投影的类型信息不会跳过构造执行：其它字段的计算和契约仍在实际生成时执行并可能失败。

这里保留的是语法检查已经得到的类型，不调用未知 Type 生产函数，也不猜测可变 Type 绑定经过控制流后的值。当前这些未知输入仍不能充当要求完成 Type 的局部注解。normal form 头保持原样；`Identity<Word>` 不因函数体返回 Word 而与 Word 建立相等关系。

已知的不可变 `meta.List<Type>` 也能沿局部绑定和别名传递，作为 Tuple 的展开输入或完整收集输入。类型列表保留为一个输入，不把 Tuple 改成结构元组身份：

```flurry
comptime fn typed_pair() -> <|-bool> {
    let fields: meta.List<Type> = [bool, u8];
    let Product = Tuple<...fields>;
    let pair: Product = (true, 17);
    let (enabled, number) = pair;
    '(enabled and number == 17)
}
```

这里依赖已绑定的 meta.List 与 Tuple 构造器。`Tuple<.Es=fields>` 与已知空类型列表同样可用；空类型列表参与类型构造不等于接通空 tuple 的循环体检查。定义端复用结构化 normal-form 信息分析，不执行列表中的未知 Type 生产函数；实际生成仍按原表达式执行并验证形成契约。





Type 数据留在 comptime，可以参与类型比较；不能将 Type 或包含它的对象交付到 runtime 接口。数据冻结不接受指针、可调用身份或活动存储，即使它们包在 nominal 对象里；也不会为接纳数据而展开用户 normal-form 头。可反驳 let 模式必须提供发散的 else 分支。

quote 函数的已检查正文独立保存在 NIR 的 `quote_templates` 中，dump 使用 `(quote-template ...)` 包装。模板保留泛型、契约、默认值和来源，不是 runtime 入口，也不能转换成普通函数指针。模板内部尚待展开的直接应用可以保留；可执行函数中的直接 quote 应用则必须完成展开。NIR 阶段检查同时检查模板正文，拒绝身份冲突及阶段错误。

具体的普通参数 `quote fn` 已接通源码展开。普通实参按调用顺序各求值一次，未使用的实参仍执行；主体与形参建立新鲜身份，应用没有对应的 runtime 函数调用。逻辑返回边界使用私有结果枚举及循环出口，保留闭包自身的 return；未返回资源在应用内清理，返回资源交给调用者。

~~~flurry
quote fn choose(value: bool) -> bool {
    while true {
        if value { return true; }
        break;
    }
    false
}

fn inverted(value: bool) -> bool {
    not choose(value)
}
~~~

`choose` 内的 return 只完成这次应用，因此 `inverted(true)` 仍返回 false。每份 quote／quote fn 主体独立检查单洞：一个显式 splice 与一个 quote fn 应用不能同时出现；普通 fn 的主体可以包含多次应用。生成器可以返回含 quote fn 应用的表达式代码。递归展开消耗预算，失败不退回 runtime 调用。作为 runtime 函数指针逃逸会拒绝。

默认参数复用普通调用路由：先按源码顺序求值所有显式实参，再在定义的形参环境中依次补齐缺省项。默认表达式读取的是已取得的绑定，不重新求值实参，也不会被调用者的同名局部捕获。对前序资源对象直接读取 Copy 字段，不要求整个对象是 Copy；对象可用性仍由移动检查保证，带构造或调用的取字段表达式不会因结果是 Copy 就自动取得稳定性证据。

~~~flurry
fn opposite(value: bool) -> bool { not value }

quote fn choose(first: bool,
    .second: bool = opposite(first),
    .third: bool = opposite(second)) -> bool {
    second and not third
}

fn selected() -> bool {
    choose(true, .second = true)
}
~~~

这里仅运行一次 `opposite`，用于 third。默认计算仍须满足稳定观测和有限完成的要求；这些证据在 quote 模板退出可执行函数表前检查，展开不会放宽默认值规则。内部返回包具有编译器来源标记，只用于证明结构化清理没有额外用户行为，不为其中的资源授予 Copy。

Type 泛型复用普通调用的推导与实例化，支持 `where T`、`where T :- Trait` 和 requires 类型命题。实例化先替换模板中的类型、默认表达式和普通调用信息，再卫生展开；不会展开 normal form 的头。泛型调用者中的内部返回存储保留同一组类型参数，随调用者实例化。

~~~flurry
quote fn identity(value: T) -> T where T { value }

pure fn forward(value: T) -> T where T { identity(value) }

fn checked() -> bool {
    forward(true) and identity('a') == 'a'
}
~~~

trait 约束与类型等式要求必须在展开前取得证据；不能以删除函数调用绕过它们。显式 `identity<bool>(true)` 也计作 quote 主体的一个接入点。内部返回包的空包覆盖位置和已移出载荷的清理状态有独立证据，因此泛型转发不要求凭空证明任意 T 可销毁，也不会授予 T 额外的复制能力。

Type 子句支持具名默认值；缺省时采用声明默认值，不以调用推导替换它，显式具名实参可覆盖并继续检查约束。默认值可引用前序类型参数。

~~~flurry
quote fn keep(value: T) -> T where .T: Type = bool { value }

fn defaults() -> bool {
    keep(true) and keep<.T = char>('a') == 'a'
}
~~~

固有 `quote` 方法可声明在 struct／enum 内部或固有 impl 中。接收者先求值一次，再按普通参数规则绑定；`*self` 保留指向原位置的引用，展开不会复制被引用的对象。容器和方法自身的类型参数均参与实例化；具名可选实参可以观察已绑定的接收者。方法调用与普通 quote 函数调用一样计入单洞限制，不能通过方法语法绕过 runtime 阶段要求。

~~~flurry
struct Flag { value: bool }

impl Flag {
    quote fn flip(*self) { self.value.* = not self.value.*; }
    quote fn read(self, .answer: bool = self.value) -> bool { answer }
}

fn changed() -> bool {
    var flag = Flag { value: false };
    flag.flip();
    flag.read()
}
~~~

展开保留所有普通实参的求值和资源义务：未使用的资源实参也必须求值并清理，按值返回的接收者把资源义务交给调用者，不产生额外复制。运行产物中没有这些 quote 方法的函数入口。

变参 quote 函数复用普通调用的收集规则，整包类型仍约束整个参数。位置展开和属性收集在展开主体之前建立参数帧，普通实参按原源码顺序各求值一次，空包生产者也执行；默认值随后使用已绑定的完整包。

~~~flurry
quote fn pack(...items: T) -> T where T { return items; }

fn configured() -> bool {
    let (enabled, letter) = pack(...(true, 'a'));
    let config = pack { enabled: enabled, letter: letter };
    config.enabled and config.letter == 'a'
}
~~~

这里两次 pack 都确定性展开，返回的元组／记录正常参与移动与清理；属性不是具名形参。固有方法的接收者先于包和属性求值，展开后仍保留独立的 return 边界。

普通 `lambda` 参数也复用调用路由，支持占位符闭包与尾随 `do`；普通 `implicit` 参数支持显式具名供应。先构造一次闭包值，再展开 quote 主体，捕获发生在它原来的实参位置。属性生产者先于尾随 `do` 执行时，闭包捕获其执行后的状态。闭包自己的 return、参数位置身份和仿射资源义务保持不变。

~~~flurry
quote fn apply(value: T, implicit enabled: bool, lambda action: fn(T) -> T) -> T
    where T {
    if enabled { return action(value); }
    value
}

fn transformed() -> bool {
    apply(false, .enabled = true) do { not _0 }
}
~~~

这里的 `action` 是普通 runtime 闭包参数，`enabled` 是按值传入的普通 implicit 参数。两者都不授予 contextual 绑定对应。缺少 implicit 供应时，当前仍报告自动供应协议尚未实现；多个 lambda 槽导致尾随正文目标不唯一时拒绝。trait 接口中的参数标记同样在选择实现前完成路由，泛型调用者晚期展开不改变其规则。

当前支持普通参数及上述参数协议、位置与属性变参收集、元组包展开、具名可选参数、默认表达式、Type 泛型子句及默认值、类型命题约束和固有 quote 方法。Type 子句同时接受 `where T` 与 `where T: Type`，两者共享类型推导、显式实例化与依赖默认值检查。contextual 参数、值泛型子句及 arm/template 汇合仍未接通。trait 中的 quote 方法声明、默认正文及 quote 实现已能检查与展开，方法契约及 S-expression dump 保留 quote 类别；普通函数实现不能满足它的确定性展开要求。将 quote 实现用于普通 trait 方法所需的 callable 适配仍未接通。其它无主体 quote 声明可进入 HIR，但不能据此生成可执行函数；quote 与 comptime 组合仍拒绝。

`quote fn` 的显式 `comptime` 参数沿用普通分阶段调用规则。输入先完成并冻结，Type 输入可以决定后续参数与结果的类型；展开后的普通实参仍按源码顺序求值一次。comptime 形参不会成为 runtime 参数槽，quote 函数本身仍属于 runtime 代码类别。

~~~flurry
quote fn identity(comptime T: Type, value: T) -> T { value }

comptime fn opposite(value: bool) -> bool { not value }

quote fn choose(comptime flag: bool, value: bool,
    comptime .enabled: bool = opposite(flag)) -> bool {
    if enabled { return value; }
    false
}

fn checked() -> bool {
    identity(bool, true) and choose(false, true)
}
~~~

调用可以通过另一函数的 comptime 参数转发尚未取得的输入。NIR 保留 checked template 与阶段明确的调用；待调用者实例化后，共享的 staged binder 完成输入、默认值与依赖类型，重新检查替换后的类型命题，再卫生展开主体。trait 方法先选择实现并重绑定接口默认形参，随后采用同一路径。IIR 中不保留 quote runtime 入口。

输入不得读取未来的 runtime 参数或调用 runtime 函数。支持的数据域和冻结限制与普通 staged 输入一致；这里不额外开放活动指针或任意闭包图。递归展开受预算限制，耗尽时拒绝，不回退成运行时调用。效应结果沿用下面的 computation 边界，不因接通 comptime 参数而抹去效应行。

`quote fn` 已支持现有可检查程序谓词形式的 `requires`、`ensures` 和参数 `old`。它们属于程序验证层：调用处证明前置条件，每个正常返回路径证明后置条件。展开不会把证明义务变成隐式 `assumes`，也不会通过 PTC 求解程序状态。

~~~flurry
quote fn flip(value: bool) -> result: bool
    where requires value, ensures result == not value'old {
    value = not value;
    return value;
}

fn checked_flip() -> bool { not flip(true) }
~~~

入口快照在实参绑定后建立，仍要求相应类型具有 Copy 权限。后置条件检查放在每条返回路径的值完成之后、离开该逻辑作用域之前；尾表达式也经过同样处理。嵌套闭包的 return 属于闭包自身。结果只求值和交付一次，谓词观察不会消费仿射结果。显式 `assumes` 检查失败时，活跃资源仍沿原清理规则释放。

trait 接口契约与所选实现契约同时保留，形参、结果及旧值快照按身份重绑定；实现使用不同名字或放宽前置条件，均不能抹掉接口要求。尚未调用的模板也要验证其声明的后置条件。带 comptime 参数的模板在绑定输入后沿用同一契约展开路径。

内部 IR 以 `(snapshot ...)`、`(require-predicate ...)` 和 `(ensure-predicate ...)` 区分快照、调用义务与返回义务；它们有运行时检查语义，但程序验证不能将后两者当作凭空获得的事实。实际内存规格仍受现有程序验证器的可表达和可证明范围限制。

效应结果已能随 `quote fn` 展开，包含带 comptime 输入和 trait 实现选择的应用。实参准备留在调用者的效应环境中，主体进入自己声明的 computation 环境；内部返回汇合只保存完成值，效应行及出现位置不进入普通返回存储。

~~~flurry
effect read() -> bool;

quote fn subject(value: bool) -> #read bool {
    read()#;
    return value;
}

fn resumed() -> bool {
    subject(true)# { read() => resume false, }
}
~~~

这里 `resume false` 只交付 read 的结果，subject 继续执行并返回 true。调用者安装的 handler 若执行 return，仍返回它所属的调用者；quote 主体内部安装的 handler 执行 return，则只完成该 quote 应用。

现有 direct／direct-passing 执行路径保留安装、求值和请求顺序。direct-passing provider 先于受处理表达式的实参求值，重复请求不会重复安装；仿射槽位的请求次数检查仍有效，不能通过展开变成可复制资源。缺少匹配的效应环境和 comptime 中的 runtime 应用仍拒绝。

效应限定结果中的命名绑定现在指向 computation 正常完成后交付的值。例如 `-> result: #poll bool` 的 `ensures` 中，result 是 bool；它不表示效应环境。普通 fn、quote fn、staged 参数和 trait 方法共用这一规则。返回值的标量／聚合事实可以传给调用者，但 handler 可修改的外部存储事实必须失效；handler 提前返回需要证明它所退出的调用者契约，不能借用尚未完成的 callee 保证。

带清理调度的 direct 效应执行已接通 quote 展开，包括带 comptime 参数的泛型 quote 函数。载荷按值转入 handler 后，handler 局部资源和载荷先清理；随后退出展开主体，再退出安装者。resume 和从 handler 提前 return 都保留各自的控制边界，不复制载荷，也不重复清理。

```flurry
trait Retire { fn drop(*self); }
struct Ticket { flag: bool }
struct Producer {}
struct Sink {}
impl Retire for Ticket { fn drop(*self) {} }
impl Retire for Producer { fn drop(*self) {} }
impl Retire for Sink { fn drop(*self) {} }

effect deliver(value: T) -> bool where T;

quote fn produce(comptime enabled: bool, item: T) -> #deliver<T> bool where T {
    let producer = Producer {};
    if enabled { return deliver<T>(item)#; }
    false
}

fn entry() -> bool {
    produce(true, Ticket { flag: true })# {
        deliver<Ticket>(ticket) => do {
            let sink = Sink {};
            resume ticket.flag;
        },
    }
}
```

在 Retire 绑定为 Drop 协议的测试环境中，该例按 Sink、Ticket、Producer 的顺序清理；produce 没有 runtime 函数入口。把 resume 改为 `return ticket.flag;`，返回目标是安装 handler 的 entry，展开主体仍先完成清理。comptime 绑定新增的外层作用域与原主体即使指向同一源码范围，也保留不同的清理边界。当 enabled 为 false 时，普通实参 Ticket 仍已求值，但没有转入 handler：先清理 Producer，再清理仍由调用作用域持有的 Ticket，不创建 Sink。重复调用与嵌套 quote 转发分别保留这一所有权判定，不把某次调用的“已转移”状态复用到另一次调用。

契约拒绝同样保留清理义务。前置条件检查发生在普通实参求值之后、quote 主体执行之前；拒绝时清理已准备的 Ticket，不创建 Producer 或 Sink。后置条件拒绝已完成的结果时，按实际控制路径清理剩余所有者，不能再次清理由 handler 消耗的 Ticket。这些异常路径有参考解释器回归：在已检查 IR 中注入失败谓词，分别核验尚未转移和已经转移的载荷；源程序的静态契约证明要求不因此放宽。

把 deliver 声明为 inline effect 时，物化阶段会在 comptime 参数绑定、quote 展开和类型替换完成后，再尝试一次封闭 direct handler 的展开。上述用例的物化 IR 不再包含 handler 安装或 effect request，resume 与提前 return 的资源清理顺序仍相同。早期 NIR 尚未取得 staged quote 主体时，不再因此永久错过展开机会。

这一步复用已有展开器的适用条件和失败回退：尚有未展开的效应调用、未支持的路由或控制结构时保留完整安装，不只删除 handler 的一部分。新生成的返回存储布局与源码追踪一并进入实例图，资源检查和清理计划在转换之后运行。

这一路径也覆盖 trait quote 方法：先选择实现并替换关联类型，再绑定 comptime 参数、卫生展开方法主体，最后消除封闭的 inline handler。按值接收者与传给效应的载荷保留各自所有权；正常完成或程序谓词拒绝时，handler 的局部值、载荷、方法局部值、接收者及调用者依次完成所需清理。接口继承的前后置条件不随方法入口消失；回归中单独注入接口后置条件失败，确认它仍能拒绝结果并触发剩余清理。

trait quote 方法在取得实现选择证据后展开。具体调用可在 NIR 阶段完成；普通泛型调用者先保留接口调用，待 IIR 实例化完成类型实参后选择正文并展开。接口默认实参仍引用接口的形参帧，展开时按参数位置对应到所选实现的新鲜绑定，不因实现改用不同形参名而改变默认值。

~~~flurry
trait Read { quote fn read(self) -> bool; }
struct Flag { value: bool }
impl Read for Flag {
    quote fn read(self) -> bool { return self.value; }
}

fn read(value: T) -> bool where T :- Read { value.as(Read).read() }
fn run() -> bool { read(Flag { value: true }) }
~~~

这里普通函数 read 仍可保留调用，trait 方法 read 必须展开。默认 trait 正文、泛型 impl、方法 Type 参数及引用接收者使用同一选择路径。递归展开仍受深度与工作预算约束；选择失败或缺少代码模板时拒绝，不回退 runtime 分派。延后生成的返回存储及来源进入当前实例图，清理规划使用对应布局，析构依赖扩展实例图时重新计算相关清理计划。

代码生成函数现在可以用程序 `requires` 检查已绑定的数据输入，包括默认参数。定义端检查谓词的 bool 类型、无效应限制及 comptime 阶段；实际调用在显式实参和默认值准备完成后、进入生成主体之前验证。失败即停止本次生成，不运行主体，也不把该谓词交给 PTC。

~~~flurry
pure comptime fn build(allowed: bool, .flag: bool = allowed) -> <|- bool>
    where requires allowed and flag {
    '(flag)
}

fn run() -> bool { ~<build(true)> }
~~~

`build(false)` 或 `build(true, .flag=false)` 在生成前被拒绝。谓词不能读取 quoted runtime 上下文中的绑定；未使用的生成函数也要检查这条阶段边界。当前沿用普通程序谓词支持的表达式范围。

生成器也支持只观察数据输入的 `ensures`，包括不可变参数的入口快照：

~~~flurry
pure comptime fn checked(allowed: bool) -> <|- bool>
    where ensures allowed and allowed == allowed'old {
    return '(allowed);
}
~~~

尾表达式和提前 return 在代码构造完成后统一检查后置条件，失败时不向调用者交付代码。未使用的定义检查谓词而不运行生成主体；不终止的实际生成也不会提前验证后置条件。这里的快照只涉及已冻结、不可变的数据参数，不观察 quoted runtime 上下文。对 quoted code 结果本身的程序规格观察尚未接通。

这些检查同样适用于完成的结构体和枚举输入，不限于标量。例如：

~~~flurry
struct Input { flag: bool }

pure comptime fn checked_input(input: Input, .copy: Input = input) -> <|- bool>
    where requires input.flag,
          ensures copy.flag and copy'old.flag {
    '(true)
}
~~~

显式传入 `.copy=Input {flag:false}` 会使后置条件失败；省略时，默认值保留已完成输入的字段值。枚举的 `'tag_name` 观察也保留原名义身份。契约在输入的抽象类型上检查，然后代入完成数据求值；不会因常量展开成结构体构造节点而改变谓词的合法性，也不会重新运行实参的原始生成过程。

变参入口完成收集后，契约可以通过纯结构解构观察整个参数包：

~~~flurry
pure comptime fn all_ready(...items: (bool, bool)) -> <|- bool>
    where requires do { let (first, second) = items; first and second } {
    '(true)
}

fn run() -> bool { ~<all_ready(...(true, true))> }
~~~

位置实参、展开输入与 `.items=(true,true)` 的整包供应经过同一检查；契约不会在收集尚未完成时提前求值。元组、结构体和匿名记录支持结构解构；已证明不可反驳的 let 模式也可使用无效应的 `and ... is ...` 计算，例如 `let flag and (not flag) is inverted=input;`。其中计算与 match 模式采用同一程序谓词检查，绑定在 trait 契约继承和 quote 方法展开时一并重映射。覆盖检查按计算结果类型检查后续模式，因此也支持 `flag and (not flag,true) is (inverted,enabled)` 这样的元组解构；计算本身不会为了证明覆盖性而执行。后续模式必须覆盖其完整结果域，不能因当前输入恰好匹配就省略失败处理。带独立 guard 或失败分支的 let 仍未接通，不能借解构隐藏函数调用或控制转移。

条件依赖枚举状态时，可以在程序契约中使用穷尽的 match：

~~~flurry
enum State { ready(bool), closed }

pure comptime fn checked_state(input: State) -> <|- bool>
    where requires (input match {
        .ready(flag) if flag => true,
        .ready(_) => false,
        .closed => false,
    }) {
    '(true)
}
~~~

模式可观察字面量、枚举载荷及结构字段并绑定局部值；match 守卫和各分支结果分别接受程序谓词检查。`State.ready(false)` 和 `State.closed` 都不满足上述前提。该检查也可用于普通函数或 trait 方法契约，不进入 PTC。当前还支持绑定一致的 or-pattern，例如 `.left(flag) or .right(flag)`：各候选的内部绑定映射到同一出口绑定，后续守卫与分支主体读取选中候选的值。契约复制到 trait 实现或 quote 方法时，同时替换候选绑定和出口绑定。`and … is …` 也可以使用已支持的无效应程序表达式，例如 `flag and (not flag) is inverted`。计算结果继续匹配后续模式，成功才向分支交付绑定；匹配失败则继续尝试后续候选。计算部分与守卫、分支结果采用同一套程序谓词检查，不能执行未经支持的调用、写入或控制转移。分类时也会检查模式内部的值依赖：常量主语不使其中对函数参数的读取变成 PTC 命题。类型模式尚未接通。




这尚不是完整 CMTT 实现。代码值目前保存在 elaboration 的代码环境中；值子句的完整推断、方法生成器、异构代码容器与 `Any`/`any` 证据、上下文多态、依赖 runtime identity 的类型替换、模式生成函数的完整参数／控制流协议与 arm/template 接入尚未接通，均不能据此宣称可用。局部模式代码已支持显式接口、别名、默认／显式成功输出连接及基本位置数据参数的模式生成函数，见[模式代码](patterns-and-arms.md)。普通局部 `const` 已支持可冻结数据、Type 和不可反驳的聚合解构；带 guard／else 的 runtime 局部 const、外部活跃指针及循环闭包图尚未接通；普通局部常量已可保存函数声明身份及拥有的闭包环境快照，并保留图内共享闭包身份。以下章节描述完整语言契约，不能为了适配这个实现子集而缩减。

## Intro

代码生成不仅要保存一段程序，还要知道它使用哪些绑定、产生什么结果。CMTT 把这些信息放在代码的接口中，使生成器可以组合尚未获得真实运行时输入的程序。

表达式代码使用上下文，模式代码在匹配成功后产生上下文，arm 代码把模式与使用其绑定的主体一起保存。

## 语法

~~~text
contextual_expr_type -> < runtime_binding* |- expr >
contextual_pattern_type -> < expr -| runtime_binding* >
quote_expr -> '(expr) | '[ substitution* ] (expr)
quote_pattern -> '<pattern> | '[ substitution* ] <pattern>
splice_expr -> ~<expr> | ~[ substitution* ] <expr>
splice_pattern -> ~<expr> | ~[ substitution* ] <expr>
substitution -> id = id
runtime_binding -> id : expr
~~~

替换表中的项以逗号分隔。pattern splice 的操作数也是一个 comptime 表达式，它求得模式代码；操作数本身不是源码模式。

## 正式定义

### 两种阶段，三种代码

所有 quoted code 都是 comptime 值。生成器在编译时取得、保存和组合这些值；生成的主体在 runtime 执行。引用运行时绑定保存的是绑定关系，不读取未来对象的内容。

| 类别 | 接口方向 | 使用时机 |
|---|---|---|
| expr | 给定上下文 Ψ，产生 T | 接入表达式位置 |
| pattern | 匹配 T，成功产生 Ψ | 接入模式位置 |
| arm | 外部上下文、模式、成功绑定及主体连在一起 | 接入函数级分支或匹配规则 |

上下文 Ψ 只包含 runtime 绑定，可以有多个成员并保持依赖次序。类型、代码值及生成器元信息处于 comptime 环境，不是 Ψ 中的运行时数据。

上下文多态采用 rank-1：生成器或模板的最外层可以量化上下文，使用时实例化；不能把仍可任意实例化的上下文多态方案塞进一个单态代码参数。

每段 quote 至多一个代码接入点。显式 splice 和 runtime 位置的 quote fn 应用均计数；普通函数体不自动是一段 quote。单洞不限制生成器最终能生成多少语句、状态或分支。

### 阅读路线

| 文档 | 内容 |
|---|---|
| [代码引用与阶段](quotation-and-stages.md) | 封闭与开放代码、两阶段、rank-1、单洞 |
| [上下文与替换](contexts-and-substitution.md) | 默认对应、显式改名、依赖与新鲜绑定 |
| [模式与 quoted arm](patterns-and-arms.md) | 成功上下文、定向检查和规则接合 |
| [Any 与代码见证](any-and-code-evidence.md) | 通配描述、真实接口与安全接入 |
| [Quote 函数与模板](quote-functions-and-templates.md) | 参数化代码、一次求值、逻辑返回和模板正文 |
| [组装有限程序](program-construction.md) | 循环生成、状态分派与正则自动机示例 |

## 常见示例

~~~flurry
pure comptime fn answer_code() -> < |- u32 > {
    '(42u32)
}

fn answer() -> u32 {
    ~<answer_code()>
}

test {
    println("{}", answer());
}
~~~

answer_code 在 comptime 返回代码值；answer 的生成主体在 runtime 交付 42。代码值不会作为一个通用运行时代码对象传到目标程序。

开放代码需要绑定供应：

~~~flurry
fn increment(value: i32) -> i32 {
    const code: < x: i32 |- i32 > = '(x + 1i32);
    ~[x = value]<code>
}

test { println("{}", increment(41i32)); }
~~~

x 是代码接口中的绑定，value 是接入处的真实 runtime 绑定；结果为 42。

### 生成器方法的当前实现范围

下面的代码可以检查生成器方法契约和默认正文；`ready` 仍使用普通 trait 调用路径：

```flurry
trait CodeBuilder {
    comptime fn build(code: <x: bool |- bool>) -> <x: bool |- bool> {
        code
    }
    fn ready(self) -> bool;
}
struct Builder {}
impl CodeBuilder for Builder {
    fn ready(self) -> bool { true }
}
fn builder_ready() -> bool {
    Builder {}.as(CodeBuilder).ready()
}

fn builder_flag(value: bool) -> bool {
    const original: <x: bool |- bool> = '(x);
    const code = Builder.as(CodeBuilder).build(original);
    ~[x = value]<code>
}
```

不写默认正文时，生成器方法可用分号声明，impl 必须提供兼容实现。接口比较检查数据／代码参数类别、上下文与结果类型；默认参数和正文即使未调用也要通过定义检查。静态 trait 调用使用 trait 声明中的参数名、默认值与契约，再将已准备的实参映射到选中的实现。实现内部的参数和代码端口可以改名；结果会映射回公开接口，保留合法的默认上下文连接。输入生产函数只执行一次。

父 trait 的成员查找仍要求调用者所请求的子 trait 视图成立；只有父 trait 的实现不能取得子 trait 的调用资格。泛型生成器和 trait 默认正文可以依据已声明的约束检查成员调用；真正 invocation 时再选择具体实现，不产生 runtime 的代码对象分派。生成器方法的引用接收者及代码结果绑定契约仍有明确的实现限制，这些限制不改变完整语言契约。

固有静态生成器方法已经可以调用，容器可以带 Type 参数，既支持嵌套声明，也支持独立 `impl`。方法自身的 Type 参数可以显式给出或推导：

```flurry
struct CodeBuilder {}
impl CodeBuilder {
    comptime fn forward(code: <input: T |- T>) -> <result: T |- T>
        where T {
        code
    }
}

fn forward_flag(value: bool) -> bool {
    const original: <x: bool |- bool> = '(x);
    const code = CodeBuilder.forward(original);
    ~[result = value]<code>
}
```

该调用先选择适用的固有实现并检查访问权限，再走生成器共用的参数推导、默认参数、展开参数和属性调用流程。无类型标注的 `const` 也能识别这一代码结果。数据实参仍须在 comptime 求值，不能传入 runtime 数据，也不能把生成器成员取作普通函数值。父级 Type 参数会替换代码上下文、数据参数及正文；方法的默认 Type 参数也可引用父级参数。trait 与 impl 的父级值参数已经可捕获；引用接收者已接入固有方法与 trait 分派，按普通接收者规则先形成地址并求值一次，正文通过 `self.*` 访问对象；调用结束后刷新调用者的数据快照。

```flurry
struct TypedBuilder where T {}
impl TypedBuilder<T> where T {
    comptime fn build(code: <|- U>) -> <|- U>
        where .U: .Type'builtin = T {
        code
    }
}

fn generated_flag() -> bool {
    const code = TypedBuilder<bool>.build('(true));
    ~<code>
}
```

这里 `T` 来自已选中的容器实例，`U` 属于方法自己的子句，默认取 `T`。同一方法以不同容器实例调用时分别取得环境；递归调用也不会覆盖外层的类型绑定。

生成器也可以接收 comptime 的对象值。接收者仍必须属于可支持的不可变数据域；runtime 对象不能借方法语法进入代码生成阶段：

```flurry
struct FlagBuilder { flag: bool }
impl FlagBuilder {
    comptime fn build(self) -> <|- bool> {
        let flag = self.flag;
        '(flag)
    }
}
fn generated_value() -> bool {
    const code = FlagBuilder { flag: true }.build();
    ~<code>
}
```

值接收者先于其余显式实参求值一次。trait 方法可以同样声明 `self`，并通过 `builder.as(Build).build()` 选择实现；trait 默认正文中的 `self` 转调也使用同一接口检查。展开参数、属性调用和代码参数的上下文连接继续遵循既有规则。当前引用接收者 `*self` 尚未接入生成器的数据与别名模型。

方法自己的值子句也进入生成器实例化，trait 调用仍使用公开声明的默认值和名称：

```flurry
trait EnabledBuilder {
    comptime fn build() -> <|- bool> where .enabled: bool = true;
}
struct SwitchBuilder {}
impl EnabledBuilder for SwitchBuilder {
    comptime fn build() -> <|- bool> where .flag: bool = false {
        '(flag)
    }
}
fn default_enabled() -> bool {
    ~<SwitchBuilder.as(EnabledBuilder).build()>
}
```

这里结果为 `true`，因为调用采用 trait 的 `.enabled` 默认值。值域可以依赖前面的 Type 子句，例如 `where T, value: T`；实现即使没有使用该值，也必须匹配其域，不能把 `bool` 值参数换成 Type 参数。引用值子句的布尔前置条件属于程序规格，不能送入 PTC；比较带值索引的 normal-form 类型仍是类型命题，且不展开其头。

trait 自身的值参数也可以参与生成代码，并成为方法默认值的来源：

```flurry
trait ConfiguredBuilder where enabled: bool {
    comptime fn build() -> <|- bool> where .flag: bool = enabled {
        '(flag)
    }
}
struct ConfiguredSwitch {}
impl ConfiguredBuilder<true> for ConfiguredSwitch {}
impl ConfiguredBuilder<false> for ConfiguredSwitch {}

fn configured_flag() -> bool {
    ~<ConfiguredSwitch.as(ConfiguredBuilder<true>).build()>
}
```

两个 trait 实例分别保存自己的值；嵌套调用另一个实例后，外层仍使用原来的绑定。父 trait 继承会保留这些参数。默认值在包含父级绑定的完整环境中接受定义检查，调用时显式提供实参不能掩盖错误默认值。

生成器实现也不能加强 trait 的程序前置条件。这在定义阶段检查，即使方法尚未调用；普通数据形参和方法值子句按绑定身份对应，父 trait 的值实参先代入。例如，接口允许 `count > 10` 时，实现可以只要求 `n > 0`，但不能要求 `n > 20`：

```flurry
trait SizedBuilder {
    comptime fn build(count: .u8'builtin) -> <|- bool>
        where requires count > 10;
}
struct SmallBuilder {}
impl SizedBuilder for SmallBuilder {
    comptime fn build(n: .u8'builtin) -> <|- bool>
        where requires n > 0 {
        '(true)
    }
}
```

省略实现的前置条件时，trait 调用仍检查公开条件。显式条件的兼容性证明还包括谓词自身的定义性，例如除法的分母非零。证明只读取 comptime 数据，不把代码参数的 runtime 上下文当作可用程序事实，也不经过 PTC。当前检查复用有预算限制的程序验证器。任意精度整数的单边比较也能推导，例如 `n > 10` 可以证明 `n > 0`；取得同一值的有限上下界后，还会建立区间供后续多项式推导使用。单边事实绑定于程序值的版本，赋值后不能沿用旧事实，分支合并只能保留两侧共同成立的条件。尚未支持的谓词、无法建立所需边界或预算耗尽仍可能返回 Unknown，此时拒绝兼容性证明并保留原因，不能视为已满足契约。公开后置条件继续在生成器调用返回时检查；代码结果绑定契约仍未实现。

单边整数条件还可以随 comptime 局部副本、参数、返回值和元组解构传递。复制建立新值身份，后续写入原变量不会改变副本的既有条件；通过指针改写的局部值则不能沿用旧事实。缺少有限区间时，精确复制等式作为程序规格证据保留，不从等式虚构区间。`Integer` 仍是 comptime 值：比较也须在 comptime 完成，再把 bool 等可接入值交给 runtime。

次数不超过三次的 comptime 整数表达式也保留精确来源，例如 `n + 1`、`n * n` 和 `x * y * z + x`。局部绑定、返回值及元组字段可以携带这些表达式关系；缺少有限上下界只会限制区间推导，不会自动抹去仍然有效的精确表达式。关系引用的是计算当时的输入版本：输入被改写后不能把旧结果解释为新输入的函数，需要引用入口值时使用 `n'old`。比较两边可以先消去相同的多项式项，例如 `n + 1 > n`、`n * n + 1 > n * n` 不要求给 n 提供有限区间。消项之前仍检查每个输入的版本；即使旧变量会在数学式中相消，也不能用过期证据证明当前程序。不可满足的条件（例如 `n < n`）会标记该假设路径不可达。这里仍不承诺任意无界多项式的不等式求解，表达式证据也不替代溢出、除零等检查。

生成器方法的程序契约也能读取关联标量，接口实例参数属于观察身份的一部分：

```flurry
trait Flags where enabled: bool { assoc flag: bool = enabled; }
struct Holder where enabled: bool {}
trait Builder where enabled: bool {
    comptime fn build(code: <x: bool |- bool>) -> <x: bool |- bool>
        where requires Holder<enabled>.as(Flags<enabled>).flag,
              ensures Holder<enabled>.as(Flags<enabled>).flag {
        code
    }
}
impl Builder<enabled> for Holder<enabled> where enabled: bool {}
impl Flags<enabled> for Holder<enabled> where enabled: bool {}

fn invert(value: bool) -> bool {
    const original: <source: bool |- bool> = '(not source);
    const code = Holder<true>.as(Builder<true>).build(original);
    ~[x=value]<code>
}
```

这里先在生成阶段检查 `Flags<true>` 的条件，再交付代码；返回的代码仍依赖显式 runtime 端口 x，接入后才读取 value。切换为 `Builder<false>` 会在前置条件检查时拒绝，不能用另一个实例的事实替代。显式实现可以改名数据参数和代码端口，但不能增加公开契约不能推出的条件；后置条件在生成器交付代码时也实际检查。这些条件不等于对返回代码对象进行相等比较，也不建立生成后程序的额外假设。

## 与 Rust / Zig / C / C++ 的关键差异

C 预处理宏和 Rust 声明宏组织 token，C++ 模板与 Zig comptime 可按编译时输入生成不同程序。这些构造不直接提供本篇把显式 runtime 上下文放进代码类型、并让模式输出上下文的组合接口。[Rust 声明宏](https://doc.rust-lang.org/reference/macros-by-example.html)、[Zig comptime](https://ziglang.org/documentation/master/#comptime)

理论基础中的显式上下文与替换可参照 [Contextual Modal Type Theory](https://www.cs.cmu.edu/~fp/papers/tocl07.pdf) 与[上下文多态扩展](https://arxiv.org/abs/1801.09225)。Flurry 的 pattern、arm、单洞及 Any 接合由本主题自己的规则规定，不从论文自动继承其元理论结论。

## 运行时实现设计引导

runtime 只执行接入后的程序。普通调用、分支、对象访问和清理保持原有契约；quote 本身不要求在 runtime 分配代码对象或解释源码。

代码共享不等于资源共享或复制。两次接入如果使用同一份运行时 owner，必须检查生成后的真实使用顺序。

## 编译器实现引导

编译器分别保存阶段环境、runtime 绑定上下文、代码类别、结果契约和控制接口。quote 构造这些信息；splice 恢复接口、建立合法替换、新鲜化内部绑定，并接回目标程序。

当前表达式代码接口由共享的 `nir::code_interfaces::ExpressionCodeInterface` 保存；完整生成器签名使用 `CodeFunctionSignature`，其中 `CodeInput` 显式区分 comptime 数据参数和代码参数。生成器实例化通过统一入口替换所有参数和输出，不能把代码参数的 runtime 结果类型当成该参数自身的类型。结果类型和上下文中的类型一起替换，绑定身份、开放端口及默认连接保持不变；替换失败不发布半完成接口。复制预算同时覆盖类型结构、端口名称和连接表。方法契约的 `MethodSignature` 现在显式区分普通 callable 和生成器签名，并保留这一类别进行泛型替换。普通函数摘要、协议入口、解释执行和 IIR 调用必须取得普通 callable 签名；生成器类别不能通过这些入口抹去代码上下文。trait 兼容性流程已增加代码签名分支：先比较数据／代码类别、上下文对应、固定依赖和默认连接，再检查其中的类型等价。上下文身份与连接关系不交给 PTC；类型比较保留 normal-form 头。源码中的静态／值接收者生成器方法声明、实现签名检查和默认正文检查已接入；声明可以引用 `Self` 和父级类型参数。生成阶段成员保留在 elaboration 的接口世界中，普通执行产物移除这些成员，同一 trait 的普通方法和 comptime 方法仍可使用。固有静态生成器方法已接入实现选择、证据校验和普通生成器参数流程；父级 Type 参数使用 invocation 局部环境，不改写共享生成器声明。签名替换与正文／默认值环境一起传递，调用结束或失败时恢复旧环境。静态 trait 分派先建立公开接口的输入帧和前置契约，再映射到实现帧；实现结果重映射回公开接口后检查公开后置契约。值接收者通过相同的编译期数据准备流程成为首个输入；预检只取类型证据，接收者本身先于其他实参求值一次。方法值子句的域保存于共享代码签名，实例化保留 Type／comptime value 的类别；trait 父级值参数在定义期保留抽象数据绑定，在调用时按声明域物化；只有 Type 捕获加入类型相等假设。泛型 impl 中从 nominal subject 推导出的父级值参数可传入生成器环境；引用接收者已接入固有方法与 trait 分派，按普通接收者规则先形成地址并求值一次，正文通过 `self.*` 访问对象；调用结束后刷新调用者的数据快照。

接入后的程序仍接受阶段、类型、效果、可达性和资源检查。普通 Splice trait 的数据转换不替代这里的代码接入检查。


当前实现也支持 struct／enum 的已完成值索引，并可在具体索引上选择固有生成器方法。例如，以下两个实现保持不同的 nominal 类型身份：

```flurry
struct Flag where tag: bool {}

impl Flag<true> {
    comptime fn build() -> <|- bool> { '(true) }
}
impl Flag<false> {
    comptime fn build() -> <|- bool> { '(false) }
}

fn run() -> bool {
    ~<Flag<true>.build()> and not ~<Flag<false>.build()>
}
```

这一支持包括混合 Type／值子句、引用前序 Type 参数的值域、具名默认子句及字段默认值。具体布局入口重新检查值域，定宽整数索引不能通过截断进入声明域。不同已完成标量索引的 nominal 身份可用于具体 impl 的互斥判断；这不是把一般值运算或程序规格交给 PTC。newtype 已支持自身声明的值参数、依赖域和默认值，以及嵌套声明的父级 Type／值参数捕获。捕获参与类型身份和底层布局特化；成员的值域与默认值可以引用父级参数，显式实参仍取调用者环境。nominal 声明的一般程序前置规格仍有实现限制。


泛型 impl 可以从 nominal subject 的参数位置推导已完成值，再将该值传入本次生成器 invocation 的环境：

```flurry
struct Flag where tag: bool {}

impl Flag<tag> where tag: bool {
    comptime fn build() -> <|- bool> {
        if tag { '(true) } else { '(false) }
    }
}

fn run() -> bool {
    ~<Flag<true>.build()> and not ~<Flag<false>.build()>
}
```

`tag` 的声明域随 impl 保存，匹配与证据重放都检查推导结果属于该域。混合 `T, tag: T` 的输入按声明顺序替换前序参数；它们不是一组默认属于 Type 的变量。固有生成器与 trait 生成器复用这一选择结果，捕获环境在每次 invocation 后恢复。普通实例方法的签名替换及物化同样保留值参数类别。

当前这里实现的是从 nominal 类型身份中的输入位置推导值。它不反解不透明 normal-form 函数，也不把算术或程序状态断言变成 PTC 命题；trait 实参中的直接值绑定现已可用于推导，完整记录型索引也可以作为一个值捕获。


值参数也可以只从 trait 实参取得，不必出现在 nominal subject 中：

```flurry
trait Build where flag: bool {
    comptime fn build() -> <|- bool>;
}
struct Host {}
impl Build<flag> for Host where flag: bool {
    comptime fn build() -> <|- bool> { '(flag) }
}
fn run() -> bool {
    ~<Host.as(Build<true>).build()>
        and not ~<Host.as(Build<false>).build()>
}
```

同一绑定出现在多个索引位置时，所有位置必须一致。匹配记录型索引时，值字段的身份检查与 Type 字段的类型证明分开；后者保留在选择证据中，重放不能仅检查最终替换列表而跳过原始查询。`T, value: T` 的值域在类型输入推导完成后检查。

对尚未确定的值关系，impl 一致性检查保守地允许其可能相交，再尝试利用 subject 与类型约束证明互斥。例如两个不同结构类型各自实现 `Build<flag>` 不需要引入值方程；同一 subject 的潜在重叠不能因此被放行。不反解任意 comptime 计算，也不向 PTC 添加程序谓词。


实现选择层现可利用部分开放记录的固定字段证明互斥。例如两个记录索引都含尚待推导的 `value` 字段，但嵌套的 `tag` 字段分别固定为 `true`、`false`，这两个实现可以共存。未知字段仍保留可能相交；不透明值计算不能被当成已知常量。选择证据和一致性证明都由原始声明重新构造并校验。

impl 头部也可以通过 comptime 调用组装这样的开放记录。调用搬运尚未具体化的输入，返回值保留原声明身份；实现选择随后从记录字段推导这些输入。

```flurry
comptime fn settings(...properties: T) -> T where T { properties }
typealias Settings = (settings { tag: true, enabled: true })'type;
trait Build where mode: Settings { comptime fn build() -> <|- bool>; }
struct Host {}
impl Build<(settings { tag: true, enabled: flag })> for Host where flag: bool {
    comptime fn build() -> <|- bool> { '(flag) }
}
impl Build<(settings { tag: false, enabled: flag })> for Host where flag: bool {
    comptime fn build() -> <|- bool> { '(not flag) }
}
fn run() -> bool {
    ~<Host.as(Build<(settings { tag: true, enabled: true })>).build()>
        and ~<Host.as(Build<(settings { tag: false, enabled: false })>).build()>
}
```

这里执行的是记录组装，不是反解一个不透明 normal-form 头。嵌套组装和同一输入的多次使用保留相同身份；若两个使用位置给出不一致的具体值，该 impl 不适用。


值索引 nominal 类型也可通过实际 comptime 调用构造、返回和保存：

```flurry
struct Flag where tag: bool { value: bool }
comptime fn make() -> Flag<true> { Flag<true> { value: true } }
const saved = make();
const result = saved.value;
comptime fn run() -> bool { result }
```

常量物化、生成器字段推导和 NIR 解释执行复用保留 Type／值类别的布局替换。具体值进入解释器时还要检查声明域；即使没有任何字段，错误的索引也不能被空布局掩盖。定义期字段推导则允许合法的抽象参数，不把它误作已经完成的具体值。这个支持同样覆盖 enum payload 和带值参数的 NIR trait 分派；它不取消 runtime 使用常量时的 Splice 要求。


抽象值 invocation 的解释器基础现已加入：调用宿主可以显式提供声明身份及类型域，解释器据此接收不透明数据输入。输入可以被传递、封装到记录并再次投影；提供错误类型域、未授权身份或把 Type 输入冒充数据输入都会被拒绝。需要观察实际值的条件、标量比较和判别模式不会据此选择结果，已完成常量导出也拒绝包含这类输入的对象。其 dump 保留声明身份与类型域。

源码 elaboration 已接入当前声明的值参数作用域；调用成功或失败后恢复原来的局部环境。结果回传只在明确的声明域授权下生成带原参数身份的残余值，不能通过已完成常量或普通 invocation 快照导出接口逃逸。当前支持数据搬运和组装：需要按未知值选择分支时会报告未决，Type 结果调用已接入同一调度器：可以接收并传递抽象数据，返回经原有类型检查接受的结果；仍不能按未知数据选择类型，也不能据此执行任意符号算术。上述开放记录源码验收已启用。已知左侧的布尔短路遵守普通求值顺序：`true and flag`、`false or flag` 可以原样传递抽象输入，`false and …`、`true or …` 不求值右侧。未知左侧、逻辑取反、数值运算和模式守卫需要观察数据，报告 `AbstractValueUnresolved`；不能借右侧常量猜测结果，也不能跳过未知守卫选择后续 arm。


类型计算不会仅因捕获抽象数据就被拒绝。例如，下面的字段类型计算不观察 `tag`，可以在泛型声明检查时完成：

```flurry
comptime fn Payload(flag: bool) -> .Type'builtin { bool }
struct Flag where tag: bool { value: Payload(tag) }
```

若正文改为 `if flag { bool } else { bool }`，当前解释执行仍报告未决；即使两个分支文本相同，也不据此绕过条件求值。这个入口不放宽 Type 描述中的开放值索引检查，也不展开 normal-form 的头。


引用接收者通过生成阶段的数据环境修改调用者对象，然后返回独立的代码值：

```flurry
struct Builder { flag: bool }
impl Builder {
    comptime fn build(*self) -> <|-bool> {
        self.*.flag = true;
        let flag = self.*.flag;
        '(flag)
    }
}
comptime fn build_code() -> <|-bool> {
    var builder = Builder { flag: false };
    let code = builder.build();
    if builder.flag { code } else { '(false) }
}
fn run() -> bool { ~<build_code()> }
```

这里的指针只在构造代码时使用，返回的 quote 保存读取到的布尔值。显式 `*Builder` 数据参数采用相同的传递机制；trait 分派转发同一个地址，公开前置／后置条件仍在公开接口的参数环境中检查。运行时参数不能借此进入生成阶段，指针本身也不能成为无上下文 quote 的运行时结果。


局部指针与普通指针参数共享同一存储身份，间接写入会刷新所有者的数据快照：

```flurry
comptime fn build_alias() -> <|-bool> {
    var flag = false;
    let pointer = flag.ref;
    pointer.* = true;
    let value = flag and pointer.*;
    '(value)
}
```

`var` 指针可以重新绑定；`let` 指针不能重新绑定，但不因此把指向的对象改成只读。指针别名保留原目标，不随另一变量的重新绑定而改变。读取产生的普通值可以进入 quote；指针本身不能被隐式转成运行时上下文。


例如，`let pair = (left.ref, right.ref); let (first, second) = pair;` 保留两个指针原来的目标，随后通过 `first.*` 或 `second.*` 写入会更新对应所有者。含指针的名义数据同样保留类型与 payload 身份，不被转成可冻结常量。聚合中同时出现回调时，仍检查回调的捕获图；加入一个指针字段不会允许可变捕获或 runtime 回调绕过既有边界。


生成器后置条件中的入口快照保留参数进入本次调用时的值，即使正文通过该参数的地址间接修改了它：

```flurry
comptime fn build(flag: bool) -> <|-bool>
    where ensures flag and not flag'old {
    let pointer = flag.ref;
    pointer.* = true;
    '(flag)
}

fn run() -> bool { ~<build(false)> }
```

这里 `flag'old` 为 `false`，返回时的 `flag` 为 `true`。聚合参数的入口字段也保持原值；如果字段本身是指针，快照保留的是该地址，不是地址所指对象的深层历史状态。嵌套调用和 trait 分派分别保存自己的入口值，不会覆盖外层的快照。入口复制消耗同一 invocation 预算，并检查存储仍具有有效的构造责任。

活动数据的读取、间接写入后的所有者刷新和 trait 方法转发也消耗 invocation 预算。转发保留共享存储与回调身份，不等同于语言级 Copy；预算耗尽或存储责任无效时拒绝本次生成，不发布部分刷新的快照。


生成器的 `tuple_enumerate` 也可遍历指针或含指针的数据。每次迭代的元素绑定保留原目标，过滤 guard 和 continue 前的修改按执行顺序写回；索引和元素绑定本身仍不可重新赋值。

```flurry
comptime fn toggle() -> <|-bool> {
    var first = false;
    var second = false;
    let pointers = (first.ref, second.ref, first.ref);
    inline for (index, pointer) in pointers'tuple_enumerate {
        pointer.* = not pointer.*;
    }
    let result = not first and second;
    '(result)
}
```

这里 first 被修改两次，second 被修改一次。循环不会为了冻结元素而复制其指向的对象；活动元素同样不能通过 quote 导出成 runtime 指针。
