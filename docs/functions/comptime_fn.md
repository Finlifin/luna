# 参数阶段与子句

## Intro

一个函数的输入未必同时具备。类型参数可以先确定函数实例，普通实参稍后到来；quote 输入则保留运行时符号，使编译时操作能描述代码而不读取未来的值。

先分清“确定哪个实例”和“执行该实例”，依赖类型及代码生成的接口就容易读懂。

## 语法

~~~text
comptime_function -> pure? comptime fn id(parameter*) (-> result)? clauses? (block | (= expr))
comptime_param -> comptime parameter
quote_param -> quote parameter
trait_bound_decl_clause -> id :- expr
quote_decl_clause -> quote decl_clause
optional_decl_clause -> .id : expr = expr
normal_form_head_def -> pure comptime fn id<parameter*> (-> result)? clauses? (block | (= expr))
~~~

子句参数、普通形参和返回类型都复用表达式。参数列表按次序建立依赖；参数可依赖子句参数，结果可依赖两者。

## 正式定义

### 定义种类与执行阶段

显式 comptime fn 在编译时执行。有子句参数的 definition 对应一个接收这些参数的外层编译时定义；其结果仍可以是普通 runtime 函数、quote fn 或其他定义。

因此，参数化 runtime 函数的实例主体不自动在编译时执行。普通函数中的 comptime 参数只规定该输入的阶段；quote fn 的代码接入规则也不把它归类为 comptime fn。

derive fn 的参数遵循 comptime fn 的参数规则。派生器在编译时生成实现，生成方法的执行阶段由方法及实现自身决定。

范式头的尖括号应用保留头与实参，不通过展开函数体取得普通调用的结果。类型别名指向该应用时，也保留这一语义边界。普通 comptime 调用与范式应用的区别见[纯性与范式](../comptime/purity-and-normal-form.md#范式头与子句参数)。

| 接口中出现的内容 | 编译时取得什么 | 运行时取得什么 |
|---|---|---|
| 子句类型参数 T | 类型及可用约束 | 不传一个运行时 Type 值 |
| 子句中的值参数 | 相应实例化输入及依赖 | 按实际依赖规则保留必要对象关系 |
| 普通函数的 comptime 参数 | 选择本次实例的值 | 其余普通参数与实例主体 |
| 普通参数 | 声明类型和操作契约 | 本次调用提供的值 |
| quote 输入 | 代码／符号及其绑定关系 | 接入后程序使用的实际对象 |
| quote fn 的 implicit quote 输入 | contextual 对应 | 被对应的 runtime 绑定 |

同一参数是否由调用者显式提供，与它属于什么阶段分别判断。implicit comptime、comptime optional 和 comptime 收集输入都须在要求的编译时位置取得；命名供应不豁免这一要求。

### 成员函数中的父类型值参数

成员函数可以在主体中读取父类型的值参数；值参数的域也可以依赖父类型中更早的类型参数。选择成员实例时保留完整的类型与值实参，不会把同一成员的不同实例混用。

~~~flurry
struct Holder where T, tag: T {
    pub comptime fn read_at_comptime() -> T { tag }
    pub fn read() -> T { tag }
    pub fn callback() -> fn() -> T { || tag }
}

fn run() -> bool {
    Holder<bool, true>.read() and not Holder<bool, false>.callback()()
}
~~~

这里的 `tag` 是实例化输入，不是运行时局部捕获。comptime 调用按选定实例取得其值；runtime 实例进入 IIR 前将已经确定的值重建为常量，包括闭包主体中的读取。值的域和整数范围仍由父类型的构造检查，不能借成员访问绕过。此过程只替换参数身份，不展开 normal-form 头。 NIR 的值读取节点不能装载 Type、未绑定的调用槽或与已知标量域不符的值；固定宽度整数也须满足范围。抽象域保留到实例化后检查，残余调用捕获使用同一边界。进入 IIR 前必须完成常量重建，不能将该节点直接交给运行时后端。 显式取得成员函数实例时还会检查继承的值参数域，即使函数体完全不读取该参数；准入发生在查询实例缓存和执行主体之前。依赖域按本次完整的声明实参替换，记录字段和整数范围仍须匹配。无函数体的 runtime 函数身份也遵循相同准入规则，取得身份不授予编译时调用 runtime 函数的权限。

### 依赖次序

子句参数依次建立；普通参数在这些绑定下依次建立；返回值再引用已经建立的参数及子句参数。组合中的后项可以依赖前项，不能引用尚未建立的后项。

这个次序约束签名与参数交付，不把显式 runtime 实参改成按形参排列求值。调用点仍先按源码求显式实参，再按依赖顺序补缺省。

定义检查使用接口已经提供的关系，不要求先知道所有具体输入。未知 T 的合法操作来自类型要求；未知 runtime 值的访问与算术条件来自程序契约。二者都不能以“某次实例化也许可以运行”代替定义端要求。

### 裸标识符与 trait bound

裸 id 由声明语境解释：lambda 中省略参数类型，依据上下文和使用检查；typealias/newtype、类型子句参数及 normal form head 中的类型参数是 `id: Type` 的简写。

`T :- A + B` 是子句参数 T 加上正向 trait 要求的简写，等价于 `T, requires T :- A + B`。否定要求写入 requires，不扩张 `:-` 的组合语法。

返回 MemoryEffect 的 predicate 中，裸参数 id 简写 `quote id: lift Any`。普通 predicate 默认返回 bool，不因此自动 quote 所有参数。

### 普通函数的值子句

普通函数的子句也可以声明值参数。它们选择函数实例，不增加普通参数表中的 runtime 实参。实例身份保留声明和完整子句输入，主体中的值引用在该实例中替换。

~~~flurry
struct Flag where enabled: bool {}

fn enabled(value: Flag<flag>) -> bool
where flag: bool, .fallback: bool = flag {
    fallback
}

fn fixed() -> bool where flag: bool { flag }

fn run() -> bool {
    let off: fn() -> bool = fixed<false>;
    enabled(Flag<true>{}) and fixed<true>() and not off()
}
~~~

嵌套调用的部分类型上下文会区分未知 Type 与未知值索引。例如外层参数需要 `Flag<flag>`，内部泛型调用最终返回 `Flag<true>` 时，可以先取得内部结果类型，再确定外层的 `flag`。泛型效应请求也保留这种区分；`echo<Flag<true>>` 与 `echo<Flag<false>>` 是不同的操作实例，各自需要匹配的 handler，允许的效应行不会被用来猜选实例。

`enabled` 从普通实参的类型身份推导 `flag`，再取得依赖它的默认值。`fixed` 没有可用于推导 `flag` 的实参，因此示例显式选择实例。不同的布尔值、具名结构值等子句输入不会被当作 Type，也不会因为产生相同的函数签名而合并实例。

同一签名的后续 trait bound 可以引用前面的值参数，例如 `where flag: bool, T :- Read<Config{enabled: flag}>`。调用检查同时核验输入声明域与对应 trait 实例；错误的值域或不满足的 bound 都不能通过实例化。值子句可以与 Type 子句混合，并支持显式可选输入及其默认值。

已绑定的 `meta.List<Type>` 也可以作为普通函数的一个值子句输入：

```flurry
comptime fn First() -> Type where Es: meta.List<Type> { Es[0] }
comptime fn Defaulted() -> Type where .Es: meta.List<Type> = [bool] { Es[0] }

fn example() -> bool {
    let first: First<[bool, u8]>() = true;
    let fallback: Defaulted<>() = true;
    first and fallback
}
```

这里 `Es` 是一个完整的类型列表值，不是多个独立子句参数。尖括号先供应子句输入，随后的 `()` 调用 comptime 函数取得结果类型；这不会展开 normal-form 头。默认列表可以用 `.Es = [...]` 显式覆盖。列表元素必须为 Type，索引必须在范围内，调用输入的域仍由已绑定的 meta.List 身份检查；拼写或相同的函数体不能代替该身份。

runtime 函数的签名需要检查列表内容时，可以显式延后该类型计算：

```flurry
fn identity(value: defer First<Es>()) -> defer First<Es>()
where Es: meta.List<Type> {
    value
}

fn example_instances() -> bool {
    let flag = identity<[bool]>(true);
    let byte = identity<[u8]>(42);
    flag and byte == 42
}
```

声明阶段的 `Es` 身份允许转交列表，但不能据此读取尚未提供的第一个元素。`defer` 保留类型计算，调用取得列表后再形成签名并检查普通实参；空列表或不匹配的普通实参仍被拒绝。没有引用未取得子句值的常量类型计算可以立即完成，不因位于泛型作用域中就自动延后。

显式子句输入已经足以完成签名时，可以取得对应的函数值：

```flurry
fn apply(callback: fn(bool) -> bool) -> bool { callback(true) }

fn example_callback() -> bool {
    let callback: fn(bool) -> bool = identity<[bool]>;
    apply(callback)
}
```

形成函数值时完成签名并检查对应函数体，之后可以保存、转交或捕获该值。不同列表实例保留各自的参数和结果域；`identity<[u8]>` 不能赋给 `fn(bool) -> bool`。unsafe 标记与程序契约继续保留。当前实现接通子句可由显式输入、已明确的签名部分和默认值完成，且普通形参均为 runtime 的路径；仍需要普通 comptime 实参才能形成的开放签名，以及 optional／implicit 参数协议转为普通函数值的能力，不能据此视为完成。

这里的契约保留也包括拒绝不合法的转换：当前普通函数类型尚未承载程序前置条件，因此带 `requires gate` 等程序前提的函数实例不能转成不携带该前提的 `fn(...) -> ...`，即使下一次调用看起来会传入合法值也不接受。直接调用仍正常检查前置条件。泛型工厂可以返回不丢失契约信息的函数值，例如 `fn factory() -> fn(T) -> T where T { identity<[T]> }`；外层 Type 捕获随具体工厂实例传递，被调用表达式先求值，随后普通实参按源码顺序各求值一次。

工厂还可以转交外层的声明值参数。它们保留各自的声明域，直到具体工厂实例给出值；不会在检查泛型函数体时凭空求出结果：

```flurry
fn choose(value: defer First<Es>(), state: *bool) -> defer First<Es>()
where Es: meta.List<Type>, flag: bool {
    state.* = flag;
    value
}

fn value_factory() -> fn(bool, *bool) -> bool where flag: bool {
    choose<[bool], flag>
}

fn use_factories() -> bool {
    var state = false;
    let yes = value_factory<true>();
    let no = value_factory<false>();
    let a = yes(true, state.ref);
    assumes state;
    let b = no(true, state.ref);
    a and b and not state
}
```

这里捕获的是 `where` 声明值，不是把普通 runtime 局部变量提升到 comptime。不同值参数产生的实例必须保持区别；实现回归同时覆盖 bool 和 u8，原生示例通过实际 Copy／Drop 协议检查状态指针访问。

默认子句足以完成实例时，也可以直接用函数名取得函数值：

```flurry
fn defaulted(value: defer First<Es>()) -> defer First<Es>()
where .Es: meta.List<Type> = [bool] { value }

fn example_defaulted_callback() -> bool {
    let inferred = defaulted;
    let annotated: fn(bool) -> bool = defaulted;
    let byte: fn(u8) -> u8 = defaulted<.Es=[u8]>;
    inferred(true) and annotated(true) and byte(42) == 42
}
```

沿已有默认子句规则补齐输入后，再形成开放签名并检查函数类型上下文。上下文与所得签名不匹配时拒绝，不为匹配上下文改写默认列表。显式 `.Es=...` 跳过该位置的默认供应计算；省略时，供应计算中的失败仍正常传播。这个规则不擦除普通形参上的 optional／implicit 等调用协议。

若有部分签名已明确，可以先从这些部分推导，再补齐依赖默认值：

```flurry
fn forward(value: T) -> defer First<Es>()
where T, .Es: meta.List<Type> = [T] { value }

fn reverse(value: defer First<Es>()) -> T
where T, .Es: meta.List<Type> = [T] { value }

fn example_context() -> bool {
    let from_parameter: fn(bool) -> bool = forward;
    let from_result: fn(u8) -> u8 = reverse;
    from_parameter(true) and from_result(42) == 42
}
```

forward 从参数域推导 T，reverse 从结果域推导 T；随后各自计算 `[T]` 并完成另一端的签名。尚未完成的 `defer` 注解本身不贡献推导约束，完整签名形成后仍与上下文核验。因此不能从 `First<Es>() == bool` 反向猜测整个 Es，也不能用一个不匹配的上下文绕过最终类型检查。

方法保留外层实例输入，显式子句只填写方法自己的槽位。外层捕获也可以参与值域和默认值：

~~~flurry
struct Host where T {
    pub fn pick() -> T where value: T { value }
}

fn run() -> bool {
    let off: fn() -> bool = Host<bool>.pick<false>;
    Host<bool>.pick<true>() and not off()
}
~~~

这里 `Host<bool>` 已确定 `T`，所以 `pick<true>` 的值域是 `bool`；`pick<'x'>` 被拒绝。若方法声明 `.fallback: bool = enabled`，其中 `enabled` 来自外层实例，则省略该输入时使用该实例捕获的值。

普通实参的展开也参与值子句推导，包括函数返回的参数包：

~~~flurry
struct Flag where enabled: bool {}

fn packet(value: Flag<flag>) -> Tuple<Flag<flag>, bool>
where flag: bool { (value, true) }

fn read(value: Flag<flag>, tail: bool) -> bool
where flag: bool { flag and tail }

fn run() -> bool {
    read(...packet(Flag<true>{}))
        and not read(...packet(Flag<false>{}))
}
~~~

这里从包内元素的类型身份推导 `flag`，不读取 runtime 对象来选择实例。同一子句变量若被两个元素分别约束为 `true` 和 `false`，调用被拒绝。

值子句也能与普通参数表中的 comptime 输入共同推导。两者保留各自的位置：子句从类型身份取得值，普通 comptime 实参则按调用传入，依赖它的签名在实例化时完成。

~~~flurry
typealias Type = .Type'builtin;
struct Flag where enabled: bool {}

fn read(comptime T: Type, value: Flag<flag>, result: T) -> Tuple<T, bool>
where flag: bool, .fallback: bool = flag {
    (result, fallback)
}

fn run() -> bool {
    let (value, enabled) = read(bool, Flag<true>{}, true);
    value and enabled
}
~~~

此处返回类型依赖普通 comptime 参数 `T`，而 `fallback` 等待推导出子句输入 `flag` 后再取默认值。`read(bool, Flag<true>{}, 'x')` 仍因第三个实参与 `T` 不一致而被拒绝。

相同的调用规则适用于方法：外层实例捕获先固定，方法自己的值子句再从实参类型推导。方法参数包可以由函数计算得到；带接收者的调用先求值接收者，再按源码顺序求值参数包，包只执行一次。展开前尚未取得普通 comptime 实参时，不把其依赖类型当成独立的子句变量，待路由完成后再代入检查。


trait 方法同样可以声明值子句，接口与实现必须保持声明域兼容。参数可以重命名，后续依赖域按对应绑定替换后比较：

~~~flurry
trait Pick {
    fn pick(self, value: T) -> T where T, tag: T;
}

struct Host {}
impl Pick for Host {
    fn pick(self, value: U) -> U where U, marker: U { value }
}

fn run() -> bool {
    Host{}.as(Pick).pick<bool, true>(true)
}
~~~

`tag: T` 与 `marker: U` 在对应绑定下具有相同的域。即使值子句没有出现在普通参数或返回类型中，声明域仍属于接口契约：接口声明 `flag: bool` 时，实现不能改为字符值参数。

trait 方法的可选值子句在接口调用处取默认值，然后将完整输入传给选中的实现：

~~~flurry
trait Read {
    fn read(self) -> bool where .flag: bool = true;
}
struct Host {}
impl Read for Host {
    fn read(self) -> bool where .flag: bool = false { flag }
}
fn run() -> bool {
    Host{}.as(Read).read()
        and not Host{}.as(Read).read<.flag = false>()
}
~~~

第一个调用取接口的 `true`，不会在进入实现后重新取 `false`。使用 trait 默认主体时，接口捕获和值子句输入也一起保留；comptime 方法遵循相同规则。

### 子句位置输入的展开

类型子句实参可通过 `...packet` 展开一个 Type 值元组。位置输入跳过 optional 槽位；`.name = value` 显式覆盖对应可选项。先按源码次序求显式输入，再按声明次序补默认值，空包的生产表达式也必须执行。

~~~flurry
typealias Type = .Type'builtin;

struct Pair where A, .B: Type = A {
    first: A,
    second: B,
}

comptime fn packet(T: Type) -> Tuple<Type> { (T,) }

comptime fn Build(T: Type) -> Type {
    Pair<...packet(T)>
}

typealias Flags = Build(bool);

fn run() -> bool {
    let pair: Flags = Pair<...(bool,)> { first: true, second: false };
    pair.first and not pair.second
}
~~~

`packet(T)` 只调用一次；`B` 复用已取得的 A。类型别名、struct／enum、trait 实参及泛型函数／方法实例使用相同的位置路由。展开到范式头时仍保留原头，不执行其函数体。此处的 Type 值元组与 `Tuple` 头收集使用的 `meta.List<Type>` 是不同的值；编译器当前接通的是元组展开路径，自定义展开协议须另行接入。

在 `meta.List<Type>` 上下文中，`[T, U]` 构造类型列表，`[]` 构造空列表。元素须为 Type 值，按源码顺序各求值一次；元素中的提前退出会跳过后续元素。列表可在 comptime 中作为参数、结果和常量传递，也可索引取得元素。构造列表本身不执行其中类型的范式头。

~~~flurry
comptime fn element_types(T: Type) -> meta.List<Type> {
    [T, bool]
}

comptime fn ProductOf(T: Type) -> Type {
    let elements: meta.List<Type> = element_types(T);
    Tuple<...elements>
}
~~~

这里 `Tuple` 收到的是一个完整的类型列表输入，保留 Tuple 的范式头；`[T, bool]` 与 Type 值元组 `(T, bool)` 不可互换。当前字面量构造需由已绑定的 `meta.List<Type>` 提供目标上下文；其它元素域和一般列表构造协议尚未接通。

带固定前缀的范式头也可以将字面元组展开与完整类型列表供应组合：

~~~flurry
pure comptime fn TaggedPack<T, value: T, ...Es: meta.List<Type>> -> Type { T }

typealias Packet = TaggedPack<...(u8, 9), .Es = [bool, u32]>;
~~~

`.Es` 已供应整个收集参数，因此位置输入只属于固定前缀。编译器先展开字面元组，再用 u8 为 9 提供上下文；它不把字面元组与类型列表混同。命名输入可位于前缀前、中、后，实参仍按源码顺序各求值一次，提前退出会跳过后续输入。多余前缀项不能再进入已经完整供应的 Es，重复供应及缺失固定参数仍被拒绝。这一组合也支持计算产生的前缀包：

~~~flurry
comptime fn prefix() -> Tuple<Type, bool> { (bool, true) }
comptime fn tail() -> meta.List<Type> { [u32] }

comptime fn BuildPacket() -> Type {
    TaggedPack<.Es = tail(), ...prefix()>
}
~~~

这里先调用 tail，再调用 prefix，后者只求值并解构一次；最终参数仍按声明顺序存入 TaggedPack 的保留头。固定前缀包不占用 Es 的槽位。计算产生的空包也必须执行，不能因其没有位置项而删除。若 prefix 返回的 Type 与值字段不一致，依赖参数检查仍会拒绝交付；已具有 bool 类型的字段不会为了匹配另一个 Type 而被重新解释。类型别名中的同类调用走相同的编译期输入计划。

默认参数与变参尾部可组合。显式输入按源码顺序完成后，省略的默认值复用先前绑定；省略整个类型列表尾部表示零个收集元素，而非缺失必需参数：

~~~flurry
pure comptime fn CopiedPack<T, value: T, .copy: T = value,
                          ...Es: meta.List<Type>> -> Type { T }

typealias EmptyTail = CopiedPack<...(bool, true)>;
typealias SuppliedTail = CopiedPack<...(bool, true), .Es = [u32]>;
~~~

EmptyTail 保留四项输入：bool、true、默认 copy 的 true，以及空类型列表。默认值不重新调用包生产函数；显式供应 copy 时不计算该默认值。 带默认参数的头也可逐项收集尾部 Type 输入，如 `CopiedPack<bool, true, u8, .copy = false, u32>`：尾部为 `[u8, u32]`，而 copy 的实参仍在两项之间求值。每项先在源码位置取得，再组装最终列表；任一项提前退出时，不计算后续项或默认值。逐项收集与 `.Es = ...` 完整供应不能混用。 固定前缀位置已确定后，也可以在收集项之间展开类型列表：

~~~flurry
comptime fn more_types() -> meta.List<Type> { [u8, u32] }

comptime fn CollectedPacket() -> Type {
    CopiedPack<bool, true, ...more_types(), .copy = false, bool>
}
~~~

展开结果为 `[u8, u32, bool]`。空列表生产调用仍执行一次，展开中的提前退出会跳过后续输入；Type 值元组不能代替列表。若计算前缀包的位置数已经确定，其后也可继续逐项收集和展开列表；位置数未确定时不猜测固定前缀与尾部的分界。 收集项即使经过临时绑定，已知的不可变 Type 元素也仍作为形成约束的证据；已知违反 requires 的列表在 elaboration 时拒绝。 固定前缀包的可确定元素也保留同样的证据，并传递给依赖参数及后续默认值：例如包已经明确提供 u8 和 bool 值时，`value: T` 的冲突立即报错；若 T 仍由尚未完成的调用决定，则保留 invocation 检查。默认 Type 参数复用前面已知的 Type，不能因经过包解构而丢失其形成约束。只有确实依赖尚未完成调用的输入才保留 invocation 检查，不能因收集过程而绕过约束。静态观察不调用输入函数，也不删除原求值计划。 这一输入计划同样用于没有默认参数的范式头，是否声明 optional 参数不影响包只求值一次、尾部列表收集和类型身份。仅固定前缀的字面元组先作语法展开；尾部 `...(T, U)` 仍不能冒充 `...[T, U]`，`...()` 也不能代替空类型列表。计算包和泛型类型别名也遵循此规则，未具体化的 Type 身份仍保留在各自输入中。

混合 Type／值参数的范式头也支持字面元组展开。先展开位置，再由前面的类型输入为后面的字面量提供上下文：

~~~flurry
pure comptime fn Tagged<T, value: T> -> Type { T }
typealias Seven = Tagged<...(.u8'builtin, 7)>;
~~~

这里的 `7` 按 `.u8'builtin` 检查；不会先把整个包的数字都默认为 Integer。展开后仍检查值域、依赖类型、参数个数和声明要求。带副作用的元素只求值一次，提前退出会跳过后续元素和默认值。

固定参数的混合头也可以展开计算所得元组：

~~~flurry
typealias Byte = .u8'builtin;
comptime fn packet() -> Tuple<Type, Byte> { (Byte, 7) }
comptime fn BuildTag() -> Type { Tagged<...packet()> }
~~~

该包已经具有 `Tuple<Type, Byte>` 类型，其值按位置交付；取得第一个 Type 值后，仍须检查第二个值是否符合依赖参数 `value: T`。包中的 Byte 不会为了适应另一个 T 而被悄悄重新解释。输入计划保留包生产、解构与每项交付的次序；命名参数可以在包前后求值，尾随空包也不能被删除。默认值只复用已取得的绑定，循环中每次执行都重新建立并清理这组绑定。

普通值的跨阶段 Splice 协议不承担这些子句位置路由。即使已绑定 Splice，类型注解中的包仍在 comptime 求值，不请求 runtime 转换。

包的位置数能够确定时，目标参数类型也会传入包生产表达式。例如，下面两个分支中的数字都取得 Byte 上下文：

~~~flurry
pure comptime fn PairTag<first: Byte, second: Byte> -> Type { bool }
comptime fn Choose(flag: bool) -> Type {
    PairTag<...if flag { (7, 8) } else { (9, 10) }>
}
~~~

block、match 和可从结果上下文推导的泛型包生产函数遵循相同规则。某个字段仍依赖未知 Type 时，该字段保持未决，其他已确定字段的上下文继续传递；这不构成对未决依赖的证明。多个包的位置数都未知时不能任意划分目标槽位，调用者可以通过生产函数签名或元素类型补足信息。已确定为 Integer 的包也不会被重新解释为 Byte 包。

纯类型参数入口同样提供结果上下文。例如 `Pair<...packet()>` 可以为泛型 `packet` 提供 `Tuple<Type, Type>` 的目标，从而确定其返回类型。struct、enum、typealias、trait 和函数／方法实例使用同一规则；optional 参数不占位置槽，方法已有的接收者类型实参也不挤占方法自己的展开槽。包依然只求值一次，类型上下文不代替对生产函数要求的检查。

### Quote 输入

quote 保留 runtime 符号及其绑定。输入给编译时操作的是代码或绑定描述，不是通过符号偷读运行时内存。它可用于 comptime fn 和子句，也支持 optional；quote fn 的 contextual 输入通过 implicit quote 表示。

quoted code 都是 comptime 值。经由 Any/any 传递不能丢掉其代码类别、上下文、绑定关系和阶段限制；使用前必须有足够依据恢复合法接口。它们与普通数据的 [Splice](../comptime/quoting-and-splicing.md) 分别检查。

### 默认值与纯性

runtime 可选默认计算采用 P3 加合法入口下有限完成。comptime 默认计算不自动受这一约束；可选子句默认值在 normalization 中直接执行，需要延迟时使用 defer。没有得到必需的编译时结果，不能生成一个等待 runtime 补算的实例。

comptime pure 与 runtime pure 的核验政策分别定义，见[编译时纯性](../comptime/purity-and-normal-form.md)与[运行时纯度](../resource-management/runtime-purity.md)。

## 常见示例

### 类型参数确定实例，值参数稍后进入

~~~flurry
fn identity(value: T) -> T
where T
{
    value
}

test {
    let number = identity(7u32);
    let flag = identity(true);
    println("{} {}", number, flag);
}
~~~

两个调用分别取得相应 T 的实例。非 Copy 的 T 按值交付时移动，不因泛型函数体很短就得到复制许可。

### 参数类型依赖子句，结果依赖同一绑定

~~~flurry
fn first(values: Array<T, n>) -> T
where T :- Copy,
    n: usize,
    requires n > 0usize
{
    values[0usize]
}

test {
    println("{}", first([7u32, 8u32, 9u32]));
    println("{}", first([true, false]));
}
~~~

输出 `7`、`true`。Array 的元素与长度提供 T、n 的推断来源；非空要求使下标 0 合法，Copy 要求使读取元素能够交付 T。两个条件分别解决范围与资源问题，不能相互替代。空数组不能满足这里的接口前提。

### 普通函数带 comptime 选择

~~~flurry
fn choose(comptime left_side: bool, left: u32, right: u32) -> u32 {
    inline if left_side {
        left
    } else {
        right
    }
}

fn produce(value: u32) -> u32 {
    println("{}", value);
    value
}

test {
    println("chosen {}", choose(true, produce(1), produce(2)));
}
~~~

依次输出 `1`、`2`、`chosen 1`。实例化选择主体分支，两个普通实参仍各求值一次。

### 编译时可选输入也能显式覆盖

~~~flurry
fn configured_choice(first: u32, second: u32, comptime .use_first: bool = true) -> u32 {
    inline if use_first {
        first
    } else {
        second
    }
}

test {
    println("{}", configured_choice(4, 9));
    println("{}", configured_choice(4, 9, .use_first = false));
}
~~~

输出 `4`、`9`。缺省选择与显式 false 分别确定实例，运行时不再传 use_first。下面的调用没有相应的编译时输入：

~~~flurry
-- 拒绝：use_first 是此函数的普通 runtime 参数。
fn invalid_choice(use_first: bool) -> u32 {
    configured_choice(4, 9, .use_first = use_first)
}
~~~

将外围函数本身参数化、改用 runtime 分支，或在实际取得编译时输入的位置调用，是不同的接口选择。标记 comptime 不会把未来输入变成已知值。

### 编译时收集，运行时执行主体

~~~flurry
fn report_pair(
    first: u32,
    second: u32,
    comptime ...labels: Tuple<str, str>,
) {
    let (left_label, right_label) = labels;
    println("{}: {}, {}: {}", left_label, first, right_label, second);
}

test {
    report_pair(3, 8, "accepted", "total");
    report_pair(4, 9, .labels = ("accepted", "total"));
}
~~~

输出 `accepted: 3, total: 8`、`accepted: 4, total: 9`。两个字符串在 comptime 形成同一个 labels 输入；first、second 和输出行为属于 runtime。第二种写法直接供应完整元组，仍须满足 comptime 要求，不再执行收集。

### 带开放签名的 comptime 调用

comptime 函数也可以让输入域和结果域依赖调用点已完成的编译时计算：

~~~flurry
typealias Type = .Type'builtin;
typealias Byte = .u8'builtin;

comptime fn Selected(flag: bool) -> Type {
    if flag { bool } else { Byte }
}

comptime fn identity(flag: bool, value: defer Selected(flag))
    -> defer Selected(flag) {
    value
}

const FLAG = identity(true, true);
const NUMBER = identity(false, 7);
typealias FlagType = FLAG'type;
typealias NumberType = NUMBER'type;

fn flag() -> FlagType { true }
fn number() -> NumberType { 7 }
~~~

两个 identity 实例分别接收并返回 bool、Byte，缓存不能合并它们的签名。生成实例保留 comptime 阶段，由 required invocation 执行；生成实例之间的调用也作为求值依赖收集。函数体中的 runtime 调用在声明检查时拒绝，不能因结果域尚未形成而推迟或跳过阶段检查。

把 FLAG 或直接的 identity 调用结果用作 runtime 值，仍必须经过绑定的 Splice 协议。得到一个具体类型或值不构成跨阶段转换的豁免；Type 结果则可继续用于类型形成。这不展开 normal-form 的头，也不改变 Identity<T> 的类型身份规则。

当前已接通在调用点通过具体编译时输入完成域形成的实例，包括 comptime 函数中未额外标记的普通形参，以及显式类型子句、计算型展开包和 Splice 交付。普通形参保持原有参数分类；签名计算捕获其编译时值，并按源码次序先完成依赖输入之前的实参，省略的默认值在显式输入之后取得。当所需默认值继续引用前面的可选输入时，依赖沿默认表达式逐层收集；显式覆盖某一项会截断该项的默认依赖。comptime 函数内的显式 comptime 默认参数也可以读取前面的普通形参。签名需要后面的默认值时，前面省略的默认值即使不参与类型形成也先执行，不能被依赖筛选删除或推迟；其中的契约失败、发散仍影响本次调用。普通 comptime 形参的默认计算同样遵循编译时求值政策，不套用 runtime P3 限制。输入域已形成时，交错的位置收集与命名输入按源码位置逐项准备，再组装保留原类型身份的收集值；空收集仍交付空 tuple。收集元素的域需要尚未取得的后续输入时，以及其他尚未取得具体值的依赖输入，仍有未接通的恢复情形；含 pending 域的 pure 定义仍需尚未表示的定义侧行为证据。以上是实现限制，不要求用户以额外 comptime 标记替代语言规定的阶段语义。

例如沿用上面的 `Selected`，下面的收集输入可跨过显式可选参数：

~~~flurry
comptime fn both(.flag: bool = true, ...values: Tuple<bool, bool>)
    -> defer Selected(flag) {
    let (first, second) = values;
    first and second
}

const BOTH = both(true, .flag = true, false);
~~~

这里依次取得第一个收集元素、flag、第二个收集元素，再交付完整 tuple。若将这些字面值换成调用，仍遵守同一次序；前面的失败不能被后面的计算抢先。`flag = false` 会要求 Byte 结果，而此函数体产生 bool，因此该实例被拒绝。

普通 Type 形参也可直接形成延迟域，不需要为了这一用途再标记一次 comptime：

~~~flurry
comptime fn identity_type(T: Type, value: defer T) -> defer T {
    value
}

const DIRECT = identity_type(bool, true);
const PACKED = identity_type(...do { (bool, true) });

comptime fn relay(T: Type, U: defer T, value: defer U) -> defer U {
    value
}

const RELAYED = relay(Type, bool, true);
~~~

relay 先用 T 形成 U 的域，再要求 U 实际交付 Type 值，最后形成 value 和结果的域。`relay(bool, true, true)` 因 U 交付 bool 值而被拒绝。这里保留普通参数的 ABI 分类，类型形成显式等待对应输入值；不把普通形参改造成类型子句或展开 normal-form 头。

普通 Type 输入也可出现在延迟的函数类型结构内：

~~~flurry
comptime fn keep_callback(T: Type, callback: defer fn(T) -> T)
    -> defer fn(T) -> T {
    callback
}

fn identity_bool(value: bool) -> bool { value }
const CALLBACK = keep_callback(bool, identity_bool);
typealias Callback = CALLBACK'type;

fn apply(callback: Callback) -> bool { callback(true) }
~~~

声明检查先确认结构中的各部分应产生 Type；调用点取得 T 后，再形成具体的函数输入与结果域。嵌套函数类型和指针组成部分同样保留这条依赖，前置 Type 输入自身的域尚待形成时也可等待。恢复类型结构不执行作为值传递的 runtime callback；错误的 callback 签名仍被拒绝。这一机制尚不等于任意泛型应用和全部类型构造都已接通。

这些输入也可为泛型子句推断提供已经形成的域：

~~~flurry
comptime fn keep_inferred(U: Type, callback: defer fn(U) -> T)
    -> defer fn(U) -> T where T {
    callback
}

const INFERRED = keep_inferred(bool, identity_bool);
~~~

先取得 U 交付的类型 bool，形成 callback 的输入域，再从 identity_bool 的结果推断 T 为 bool。换成接收 bool、返回 Byte 的函数时，T 推断为 Byte，实例签名分别保留。推断需要准备显式输入时仍遵守源码次序；尚含未求解子句参数的输入域不能被当作具体 expected type 强加给实参。可以先从独立的已形成域完成推断，再准备默认值。作为普通实参交付的函数声明引用已是值，可以原样保留；普通函数类型的前缀实参也可通过 comptime 调用产生，求得结果后复用常量值重建机制交付，包括泛型函数实例。生产过程的契约失败仍先于后续默认计算。位置收集可混合标量与此类函数值，并保留与命名参数交错的源码顺序。仅用于交付的收集允许没有完整符号表示；后续默认依赖复用已取得的交付值，不因缺少符号而重新调用生产函数。普通聚合实参同样适用：例如 `Tuple<bool, fn(bool) -> bool>` 或含 callback 字段的 struct，可以先完成生产、保留函数身份，再准备后续 Type 默认值。仅“没有符号表示”允许继续普通交付，预算耗尽、类型与作用域错误仍会拒绝；若聚合本身被类型计算要求，仍必须取得合法符号。计算型展开包也可以同时包含 Type 输入和普通函数值：先求值并重建整个包，只有签名真正读取的字段才转换为符号；函数字段保持原函数身份交给 runtime 或 comptime 调用。IIR 的准备应用允许完整函数身份作为普通字段交付，但不会把尚未执行的调用表达式当成已完成值。这种交付检查也递归覆盖 tuple 和结构记录，逐项检查字段类型、数量及记录标签；绑定 Tuple 的布局仍来自可信协议元数据，嵌套结构不会抹去函数身份。struct／enum 的普通交付同样保留 nominal 声明与泛型参数身份，并递归检查字段中的完成值；enum 分支不会因为载荷含函数而丢失。nominal 字段布局由常量重建阶段检查，交付阶段不展开类型头。trait 方法值也保留 elaboration 已选择的接口、成员、subject、接口实参与方法实参；准备交付不重新查找同名成员。默认方法、泛型方法以及通过子接口取得的父接口方法均遵循这一规则，实例中的类型必须已完成。若该函数值本身参与类型计算，所需 normal-form 符号表示仍未接通；一般前向依赖恢复也未因此完成。

tuple 类型结构同样可以等待普通 Type 输入：

~~~flurry
comptime fn pair(T: Type, ...values: defer Tuple<T, T>) -> defer Tuple<T, T> {
    values
}

const PAIR = pair(bool, true, false);
typealias Pair = PAIR'type;
~~~

各元素在声明检查中要求产生 Type，取得 T 后再形成具体 tuple 域。嵌套结构 `Tuple<U, Tuple<U, U>>` 也可等待前置 `U: defer T` 的域和值。绑定 Tuple 协议时，最终类型保留 Tuple 的 normal-form 头，结构视图只用于布局和元素检查；元素类型、数量以及非 Type 输入仍分别校验。

下面的包把接口默认方法交给 runtime 调用；`T` 在编译时取得，callback 保留所选方法身份：

```flurry
typealias Type = .Type'builtin;
trait Mapper {
    fn action(value: bool) -> bool { not value }
}
struct Holder {}
impl Mapper for Holder {}

comptime fn packet() -> Tuple<Type, fn(bool) -> bool> {
    (bool, Holder.as(Mapper).action)
}
fn invoke(comptime T: Type, callback: fn(bool) -> bool) -> bool {
    callback(true)
}
fn run() -> bool { invoke(...packet()) }
```

`run()` 返回 false。包的求值阶段不会调用 action；action 在 callback 被调用时执行。

comptime 闭包通过计算型包交付给 comptime 调用时，可保留共享捕获图。例如两个包字段引用同一个 `%{flag: bool} comptime fn() -> bool` 闭包，第一次调用修改 flag 后，第二次调用看到同一份状态；两个外层闭包共享内层闭包时也保留这条关系。包字段可以再包裹 struct 或 tuple：准备其默认参数时仍保留包的原字段绑定，不将每个字段单独重建成互不关联的捕获环境。多级 Type 默认值准备复用同一个包，不重新创建捕获环境。这是 comptime 值图的交付，不把 comptime 中构造的普通 lambda 改成 runtime 闭包；执行阶段不匹配仍被拒绝。需要把图放入 runtime staged preparation 的路径须另行支持，不能据此放开跨阶段捕获。

### 返回类型值的函数

~~~flurry
pure comptime fn Identity(T: Type) -> Type {
    T
}

typealias Number = Identity(u32);

test {
    let number: Number = 9;
    println("{}", number);
}
~~~

Identity 返回 Type 值，使用大驼峰命名。圆括号调用执行函数体，取得 u32，因此 Number 是 u32 的别名，测试输出 `9`。这段示例使用普通 comptime 函数定义与普通调用。

### 范式保留另一种关系

这是独立于上例的范式定义：

~~~flurry
pure comptime fn Identity<T> -> Type {
    T
}

typealias Number = Identity<u32>;

fn keep_number(value: Number) -> Number {
    value
}
~~~

Number 指向 Identity<u32>；keep_number 只要求按原类型交付输入。normalized IR 不会展开 Identity 的主体来建立 Identity<T> 与 T 的关系。因此不能由这个定义推出 Number 可用数值 9 初始化，或拥有 u32 的算术操作。普通执行示例与这一范式示例不能仅交换括号就保持原意。

普通 comptime Type 输入也可以在 `defer` 签名中形成保留头的应用：

```flurry
pure comptime fn Identity<T> -> Type { T }
fn identity(value: Identity<bool>) -> Identity<bool> { value }
comptime fn keep(
    T: Type,
    callback: defer fn(Identity<T>) -> Identity<T>
) -> defer fn(Identity<T>) -> Identity<T> {
    callback
}
const CALLBACK = keep(bool, identity);
```

定义检查确认应用的各输入具有 Type 域；调用取得 T 后，再形成完整签名并检查类型头的应用约束。这里的 callback 接收并返回 Identity<bool>，传入普通 `fn(bool) -> bool` 仍然错误。当前实现接通全部参数域为 Type 的 normal-form 应用，支持位置输入、命名可选子句及依赖前面输入的默认链。例如 `Pair<A, .U: Type = A, .V: Type = U>` 可在 deferred 签名中写成 `Pair<T>`、`Pair<T, .U=T>` 或 `Pair<.U=T, T>`。定义侧按同一槽位规则拒绝重复名称、未知名称、缺失必需项与多余位置项；调用点取得 T 后才完成省略的默认计算。Type 变参支持固定 Type 前缀与逐项收集，例如 `Pack<A, ...Es: meta.List<Type>>` 对应 `Pack<T>`、`Pack<T, bool, T>`，以及仅展开固定前缀的 `Pack<...(T,), bool, T>`。空尾部仍形成空类型列表；tuple 展开不能越过固定前缀或替代尾部列表。普通 comptime 列表参数可用于完整供应或展开：`Pack<T, .Es=types>`、`Pack<.Es=types, T>`、`Pack<T, ...types>` 与 `Pack<T, bool, ...types>` 均保留列表输入语义。列表字面量 `[T]` 和空列表 `[]` 按绑定的 meta.List<Type> 域逐项检查。完整供应不能混用逐项收集，Type 值或 tuple 不能代替列表。签名内可调用已解析的 comptime 列表生产函数，例如 `comptime fn types(T: Type) -> meta.List<Type> { [T, bool] }`，再使用 `Pack<T, ...types(T)>` 或 `Pack<.Es=types(T), T>`。定义侧检查参数域、结果域和执行阶段，不提前调用生产者；调用点取得依赖后再执行，生产过程的契约失败正常传播。任意控制流和动态 callee 的签名组合仍不能据此宣称已全部支持。

普通 comptime 值也可形成保留头的索引，例如 `Indexed<index: bool>` 或 `Indexed<index: u8>`。`defer fn(Indexed<index>) -> Indexed<index>` 在调用点取得 index 后形成具体签名；true/false、不同整数索引仍形成不同类型，不能因头的函数体返回相同 Type 而合并。混合 Type 与固定值域的头支持命名可选输入及默认值依赖，例如 `Indexed<T, flag: bool, .copy: bool = flag>`。索引实参按头声明的域检查，runtime 输入不能因此跨阶段进入类型。参数域也可依赖更早头输入，例如 `<T, value: T, .copy: T = value>`。定义侧用已取得的 Type 输入替换后续域；尚依赖普通 comptime 输入的域只保留描述，不伪造 Type 值。调用点取得 T 后仍须按具体域检查 value 和默认值。因此 `Tagged<bool, value>` 中已知为 u8 的 value 会被拒绝，`Tagged<T, value>` 也不能免除实例中的具体类型检查。未知域只允许描述器已有证据支持的操作，不赋予调用或算术能力。索引位置也可使用有定义侧证据的一元或二元运算，例如 `Indexed<not flag>`、`Indexed<flag and true>` 与任意精度整数的 `Indexed<n + 1>`。调用点计算具体索引，布尔短路仍跳过未选择的生产分支；机器整数运算仍要求范围证据，不因 deferred 签名而绕过检查。条件表达式也可参与索引形成，例如 `Indexed<if flag { false } else { true }>`，或 `Identity<if flag { bool } else { u8 }>`。定义侧检查两条分支的域；调用点取得 flag 后仅执行选中的分支。未选择分支中的生产者不提前执行，但其类型错误不能因此被忽略。索引块可包含局部 let/var、赋值和嵌套遮蔽，例如 `Indexed<do { var selected=flag; selected=not selected; selected }>`。这些绑定由原有作用域和可变性规则检查；给 let 重新赋值，或把整数赋给 bool 局部量仍会被拒绝。局部绑定也可转发待完成的 Type 与依赖值，例如 `do { let U=T; let item=value; Tagged<U, item> }`。嵌套类型应用即使不再直接引用函数形参，也须保留局部依赖的未就绪状态，等调用输入取得后检查实际类型及值身份。在已有域证据覆盖的索引块中，也可以使用 while、break 和 continue：定义侧检查条件和循环体，调用点执行实际迭代并取得最终索引。零次迭代不会执行循环体，但不能借此隐藏循环体中的类型错误。循环仍受编译时执行预算约束；这不代表任意迭代协议或动态调用组合均已接通。

匹配表达式也可形成索引，例如 `Indexed<flag match { true => false, false => true }>`。enum 的分支绑定可继续参与索引计算，如 `choice match { .none => false, .some(value) => not value }`。定义侧仍检查模式、绑定域、各分支类型和穷尽性，调用点根据实际值选取分支；签名挂起不允许省略这些检查。固定值前缀与 Type 变参的组合可复用已有输入计划，例如 `Pack<flag, T>`、`Pack<flag, ...types>` 与 `Pack<.Es=types, flag>`。同类位置 Type 应用也覆盖 struct、enum、newtype 和类型别名，例如 `defer fn(Holder<T>) -> Holder<T>`；调用点形成具体 Holder 实例并检查其 trait 约束。类型别名沿原有别名规则处理，nominal 实例身份与 newtype 边界不会被这一步抹去。

固定参数头也可在 deferred 签名中展开计算产生的 tuple，例如 `Pair<...packet(T)>`，其中 packet 返回 `Tuple<Type, Type>`。定义侧根据已形成的 tuple 域检查字段数量和各槽位的域，不猜测包中具体 Type 值；调用点求得包后再完成类型应用。非 tuple、过多或缺少字段，以及已知的错误字段域会被拒绝，生产者的契约失败正常传播。空 tuple 包虽然不供应位置项，仍须执行生产者；其失败先于后续默认值求值。可用 `comptime fn empty(...items: Args) -> Args where Args { items }` 取得空包。返回 `()` 的 void 函数不能替代它。



### Trait bound 简写

~~~flurry
fn duplicate(value: T) -> Tuple<T, T>
where T :- Copy
{
    (value, value)
}

test {
    let (first, second) = duplicate(8u32);
    println("{} {}", first, second);
}
~~~

Copy 依据使两次交付合法。把约束省去后，不能无条件为任意非 Copy 的 T 复制责任。

trait bound 可以表达多个正向要求；负向条件写在子句中：

~~~flurry
trait Readable {
    fn read(*self) -> u32;
}

trait Tagged {
    fn tag(*self) -> u32;
}

fn describe(value: *T)
where T :- Readable + Tagged
{
    println("{} {}", value.tag(), value.read());
}
~~~

这个定义分别取得两个接口的依据。`T :- Readable + Tagged` 可展开为声明 T，再要求 T 满足这个正向组合；它不创建两个 runtime 参数。具体 implementation 的选择仍按 trait 系统处理。

一个完整的调用可以显式提供两个实现：

~~~flurry
struct Reading {
    value: u32,
}

impl Readable for Reading {
    fn read(*self) -> u32 {
        self.value.*
    }
}

impl Tagged for Reading {
    fn tag(*self) -> u32 {
        1
    }
}

test {
    let reading = Reading { value: 7 }
    describe(reading.ref);
}
~~~

输出 `1 7`。describe 的定义只依赖声明过的两个接口；调用点提供 T=Reading 的实现证据。

### 内存效果的裸参数

~~~flurry
predicate modify(target) -> MemoryEffect {
    read<target> and write<target>
}
~~~

该声明的显式形式如下；两段是同一定义的替代写法：

~~~flurry
predicate modify(quote target: lift Any) -> MemoryEffect {
    read<target> and write<target>
}
~~~

这里构造的是访问规格，不在定义 predicate 时实际执行内存读写，也不要求添加 comptime 关键字。

### Quote 绑定描述访问位置

~~~flurry
predicate copy_region(source, target) -> MemoryEffect {
    read<source> and write<target>
}

fn copy_value(source: *u32, target: *u32)
where outcomes copy_region(source, target)
{
    let value = source.*;
    target.* = value;
}

test {
    let source: u32 = 7;
    let target: u32 = 0;
    copy_value(source.ref, target.ref);
    println("{}", target);
}
~~~

输出 `7`。predicate 的 source、target 是规格使用的 quote 绑定；函数的同名参数是实际 runtime 指针。编译时可以描述将来对这些位置的访问，真正的读取和写入仍在 copy_value 执行时发生。outcomes 是允许行为的上界，不单独证明目标可写，也不声明指针互不别名。

## 与 Rust / Zig / C / C++ 的关键差异

Rust const fn 可以在受限的常量求值环境中调用；Flurry 显式区分 comptime 函数执行与 runtime 实例。Zig 支持在 comptime 环境解释普通函数，不能据此推导 Flurry 的普通函数也可任意提前执行。[Rust const evaluation](https://doc.rust-lang.org/reference/const_eval.html)、[Zig comptime](https://ziglang.org/documentation/master/#comptime)

C 没有返回 Type 值的函数，实际使用 typedef、宏或生成器；C++ 通过模板和别名模板组织相近的类型计算。[C11 §6.7.8](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[C++ alias templates](https://eel.is/c++draft/temp.alias)

## 运行时实现设计引导

runtime 实例只保留仍在运行时交付的输入和操作。去掉 comptime 参数槽不等于去掉参数值所选择的代码与依赖；quote 输入展开产生的 runtime 操作也要进入实例。

结果、环境或动态接口需要 runtime 存储时，按其真实表示交付。编译时类型值或代码描述不能作为未经定义的运行时万能容器。

## 编译器实现引导

为每个输入分别登记绑定身份、供应方式、阶段及所需接口。normalization 检查可用操作，instantiation 代入具体参数；两者之间保留依赖而非重新按文本猜测名字。

normalized IR 保留范式应用的头与实参。即使头的函数体只返回一个参数，也不能据此将应用替换为该参数，或向类型求解器加入两者相等的事实。

类型参数及 trait bound 进入类型检查；memory effect predicate、runtime 可达关系和默认计算行为进入程序验证层。PTC 不求解运行时内存规格。

quote 输入登记它指称的符号和上下文身份，不能只存一个同名字符串。通过 Any 恢复代码接口时，必须检查实际代码种类与所需上下文；查询结果未知不等于开放任意 splice。

实例缓存的键覆盖定义、语义参数、相关环境依赖和实现来源。阶段失败、编译时诊断或执行预算耗尽不应成为合法实例。运行时不保留一个补做失败 comptime 求值的回退路径。

当前实现也支持把闭合的 struct／enum 值传给普通函数的 comptime 参数，包括嵌套载荷、泛型 nominal 实例和默认参数转发：

~~~flurry
struct Settings { enabled: bool }
enum Mode where T { disabled, configured(T), }

fn enabled(comptime mode: Mode<Settings>) -> bool {
    mode match {
        .disabled => false,
        .configured(settings) => settings.enabled,
    }
}

fn example() -> bool {
    enabled(Mode<Settings>.configured(Settings { enabled: true }))
        and not enabled(Mode<Settings>.disabled)
}
~~~

冻结表示保留完整 nominal 类型、struct／enum 的区别、变体索引与字段值。不同类型、变体或载荷不会因外观相似而共用同一个实例身份；重建时按声明实例化字段类型，拒绝错误变体、字段数量和字段类型。字段须属于已支持的闭合值表示；指针、活动环境等仍不能借包装进 struct 或 enum 绕过阶段边界。

已取得的 struct 参数可以提供字段默认值，也可经局部绑定或记录模式解构后转发：

~~~flurry
fn choose(comptime flag: bool) -> bool { flag }

fn configured(comptime settings: Settings,
              comptime .flag: bool = settings.enabled) -> bool {
    flag
}

fn forward(comptime settings: Settings) -> bool {
    let { enabled } = settings;
    choose(enabled)
}
~~~

连续字段投影沿用完整 nominal 类型身份；局部别名不重新调用配置值的生产函数。字段发生 runtime 写入或其根对象地址逃逸后，不能继续把之前的字段值当作已确定的 comptime 输入。普通 runtime 局部也不会只因为具有相同结构就自动成为 comptime 参数。

已确定的 enum 配置也可通过匹配产生后续阶段输入：

~~~flurry
fn configured_mode(comptime mode: Mode<Settings>,
                   comptime .flag: bool = mode match {
                       .configured({ enabled }) if enabled => true,
                       _ => false,
                   }) -> bool {
    flag
}
~~~

当前阶段匹配支持 tuple、struct、enum 及结构记录，保留候选顺序、守卫、析取绑定重命名和 `and … is …` 顺序计算。失败候选的绑定不泄漏到下一候选；没有匹配成功时不求值其守卫，选定分支后也不求值其他主体。匹配产生的闭合值可以先绑定到局部，再传给另一个 comptime 参数。这里使用已支持的不可变表达式折叠；任意调用、副作用和所有高级模式尚未因此获得阶段折叠支持。

这种闭合值也可作为 normal-form 头的值实参。参数域可以依赖前面的 Type 参数；检查时先代入这些类型，再按真实声明检查变体与字段：

~~~flurry
pure comptime fn Configured<T, mode: Mode<T>> -> Type { bool }
typealias Enabled = Configured<Settings,
    Mode<Settings>.configured(Settings { enabled: true })>;

comptime fn is_enabled(ty: Type) -> bool {
    ty match {
        Configured<'Settings, .configured({ enabled: true })> => true,
        _ => false,
    }
}
~~~

`'Settings` 是常量模式，它成功后才为后续载荷提供具体类型。范式模式恢复的是保存的实参，`Configured` 的函数体没有因此执行，`Enabled` 也不会因为函数体返回 bool 就成为 bool。没有声明元数据时不接纳 nominal 值；错误类型、变体、字段数量与字段类型都会拒绝。通用值模式依赖一个尚未确定的前置类型绑定时，仍受上下文模式 elaboration 的现有实现限制。

实现对一组范式输入共享检查预算，计入类型替换和 nominal 布局展开。检查 enum 值只实例化所选变体的载荷，不展开其他变体；无载荷的变体仍检查完整的类型实参。预算不足不会发布一个未检查完的范式或重建值，随后提供充足预算可以重新检查。

按声明次序建立依赖图：先登记子句绑定，再登记形参类型及其供应表达式，最后登记结果和结果契约。对每条边记录需要的是具体值、类型事实还是 runtime 符号；只有第一类要求当场完成对应计算。由此可区分缺少 comptime 值、缺少操作依据和缺少代码上下文三种错误。


高阶 comptime 输入也可以参与普通参数域和结果域的延迟计算。这里的 `Select` 是有显式 `comptime` 的普通函数，调用它会计算出 `T`：

~~~flurry
typealias Type = .Type'builtin;

comptime fn Select(T: Type) -> Type { T }

comptime fn keep(
    select: comptime fn(Type) -> Type,
    T: Type,
    value: defer select(T),
) -> defer select(T) {
    value
}

const answer = keep(Select, bool, true);
~~~

调用点先取得 `select` 与 `T`，再计算 `value` 的期望类型；返回类型也使用实际传入的函数身份。若另一个函数返回不同的 Type，相同的函数签名不能使两者混同。回调需要的具名函数依赖一并进入检查后的计算环境，回调中的失败契约会终止类型计算。runtime 函数不能冒充 comptime 回调。

该输入冻结路径支持无捕获的具名 comptime 函数及其已特化实例，也支持由 producer 返回这样的函数。它保存声明身份和完整的特化实参，不保存解释器环境。捕获闭包还需区分可重建的只读快照与共享可变环境，下面说明当前边界；含可执行成员的聚合物仍需完善输入传递表示。这些实现限制不是语言对闭包能力的限制。normal-form 头的身份规则保持不变。


泛型实例之间以特化实参区分，即使调用签名相同也不会混同：

~~~flurry
comptime fn SelectFixed(ignored: Type) -> Type where T { T }

const first = keep(SelectFixed<bool>, bool, true);
const second = keep(SelectFixed<.u8'builtin>, bool, 7);
~~~

这里 `SelectFixed<bool>` 与 `SelectFixed<.u8'builtin>` 都接受 Type 并返回 Type，但返回的类型不同。恢复冻结输入时仍走函数实例化和约束检查；保存一个声明编号不能绕过 `requires`，也不能丢弃特化实参后调用未特化的主体。


捕获已经完成的 Type 或 builtin 标量的闭包，也可计算后续输入的类型：

~~~flurry
typealias Selector = %{selected: Type} comptime fn() -> Type;

comptime fn make_selector(selected: Type) -> Selector {
    || selected
}

comptime fn keep_selected(
    select: Selector,
    value: defer select(),
) -> defer select() {
    value
}

const selected = keep_selected(make_selector(bool), true);
~~~

comptime 函数的普通调用点现在可以把已经完成的捕获闭包作为活动值传给类型注解。同一次调用中的参数域、返回域按声明顺序使用同一个捕获图；注解对捕获状态的修改保留到最终函数体调用。图的依赖声明、泛型 Type 绑定和调用者已成立的类型前提一并保留，活动值不写入 normal-form 身份或持久化的类型描述。

runtime 函数的显式 comptime 参数也可接收具名 comptime 函数及其泛型实例，用它们完成延迟的参数域和返回域：

~~~flurry
typealias Type = .Type'builtin;

comptime fn Selected() -> Type where T { T }

fn keep(
    comptime select: comptime fn() -> Type,
    value: defer select(),
) -> defer select() {
    value
}

fn boolean() -> bool { keep(Selected<bool>, true) }
fn byte() -> .u8'builtin { keep(Selected<.u8'builtin>, 9) }
~~~

调用点补入回调的声明依赖；定义阶段保存的计算环境不能预先假定用户稍后会传入哪一个函数。实例身份保留回调声明及其泛型实参，最终 runtime 签名消除 select 参数槽。可选回调默认值、显式覆盖，以及具有延迟签名的函数间转发使用同一条路径；计算产生的具名回调也可作为显式 comptime 输入交付。普通 runtime 函数不能冒充 comptime 回调，实参仍按回调实际生成的类型检查。这不意味着具有固定签名的函数体已能延迟任意调用点类型计算，也不意味着捕获可变环境的闭包已经接通 runtime 实例化。

外层函数的签名固定，也可能需要在调用点完成主体中的类型计算。例如：

~~~flurry
fn forward_boolean(
    comptime select: comptime fn() -> Type,
    value: bool,
) -> bool {
    keep(select, value)
}

fn selected_boolean() -> bool {
    forward_boolean(Selected<bool>, true)
}
~~~

forward_boolean 的参数域和结果域都是固定的，但 keep 的 value 域仍依赖实际 select。编译器保留这项主体依赖，取得 select 后再检查内层调用，不在定义阶段凭空要求回调值。此需求可沿多层直接转发传播，不依赖函数的声明顺序。换成 Selected<.u8'builtin> 时，固定的 bool 实参不能满足内层域，调用仍被拒绝；未调用函数中的已知类型错误也不能借延迟检查隐藏。

仅把 comptime 值作为普通 runtime 参数交付，不会自行触发这项延迟。不可变局部绑定可以保留已取得的显式输入身份，连续别名也不重新执行输入的生产函数：

~~~flurry
fn forward_alias(
    comptime select: comptime fn() -> Type,
    value: bool,
) -> bool {
    let chosen = select;
    let again = chosen;
    keep(again, value)
}
~~~

这里只复制已经取得的输入。供阶段计算使用并已被消耗的回调别名不产生 runtime 存储；普通 runtime 输入不能借 `let` 变成 comptime 输入。字段写入或取址后，编译器不再复用对应存储先前的已完成值。高阶函数值、方法和任意局部计算中的普遍依赖恢复仍有未接通的情形。

已完成配置的字段投影也可建立这种局部绑定，例如 `let chosen = config.select;`，或从嵌套配置取得布尔值再用于类型选择。投影保留 nominal 类型及其完整实参身份；结构记录还按字段标签校验位置。它只读取已取得输入，不重新执行配置生产函数。若先修改配置或使其地址逃逸，再读取字段，不能恢复原先的已完成值证据。

配置本身也可以经过连续不可变别名再取字段：`let alias = config; let chosen = alias.select;`。当配置类型只能用于 comptime，且这些绑定的全部使用已经被阶段计算消耗，别名不会遗留到 runtime 存储。具有正常 runtime 表示的值不因此省略其绑定和清理；实际回调选择的类型仍约束调用实参，别名不擦除这种差异。

同样的阶段身份可以通过不可变 tuple 或记录解构保留，例如 `let (chosen, unused) = config;` 或 `let {select} = config;`。纯结构模式包括嵌套 tuple。带 `else` 的结构匹配保留原来的退出分支，顶层守卫在 runtime 检查；具有 runtime 布局的字段也可包含守卫与顺序模式计算。

解构也可以同时取得供类型计算使用的回调与普通 runtime 字段：

~~~flurry
fn from_config(comptime config: Tuple<comptime fn() -> Type, bool>) -> bool {
    let (chosen, value) = config;
    keep(chosen, value)
}
~~~

取得实际输入并检查 `chosen` 产生的类型后，编译器将普通字段物化为 runtime 绑定，回调不占 runtime 存储。物化只读取已完成的字段，不重新执行配置生产函数。解构后继续给回调起不可变别名也可保留其身份；别名消除与混合输入物化共用消除计划，已消耗的中间别名不会阻止普通字段生成 runtime 存储。字段的初始化快照与后续存储状态分开：后续取址、写入和闭包捕获作用于生成的 runtime 绑定，不能因此被改写成对初始常量的读取。仍含未消耗的 comptime 字段引用或未支持的模式内计算时，不能由这条结构转换消除阶段专属存储。

纯 comptime 子包也可以分多步解构，不要求一次展开所有层次：

~~~flurry
fn nested_config(comptime config: Tuple<Tuple<comptime fn() -> Type>, bool>) -> bool {
    let (callbacks, value) = config;
    let (chosen,) = callbacks;
    keep(chosen, value)
}
~~~

`callbacks` 的纯阶段解构参与同一别名消除计划，不能因它的中间读取阻止外层配置物化。多层子包和多个回调仍保留各自实际身份；选择返回错误类型的回调会按通常的调用域检查拒绝。外层配置中的普通资源字段不参与这种纯阶段消除，正常返回与提前退出都保留其原有清理责任。纯阶段子包的解构也可以带 runtime 守卫：

~~~flurry
fn checked_nested_config(
    comptime config: Tuple<Tuple<comptime fn() -> Type>, bool>,
    ready: bool,
) -> bool {
    let (callbacks, value) = config;
    let (chosen,) if accept(ready) = callbacks else { return fallback(); };
    keep(chosen, value)
}
~~~

这里只消除已完成的子包主语读取，守卫与失败分支仍保留。多层守卫按源码顺序各求值一次，前层失败后不执行后层；外层已取得的资源仍在退出时清理。其它存活的 runtime 引用不会因主语读取消除而消失，阶段专属值仍不能进入 runtime 存储。


普通字段即使绑定后未被读取，仍保留其存储与清理责任。不能因为回调已经在 comptime 消耗，就连同这些字段的 Drop 一起删除；正常离开作用域和提前 return 都遵循原有的清理次序。

`_` 忽略字段也不消除资源责任。转换保留具有 runtime 布局的完整子模式与剩余主语，由通常的模式清理处理未交付字段和父对象；被忽略的剩余资源在该模式语句结束时清理，具名绑定继续存活到其作用域结束。生成的构造节点各有独立清理边界，并保留原源码位置用于诊断。

具有 runtime 布局的子模式可以保留穷尽的 `or` 候选，例如 `(chosen, (true or false))`。旁边的 `chosen` 仍保留其显式 comptime 输入身份；runtime 候选继续使用通常的匹配、绑定重映射和清理规则。结构匹配也支持带顶层守卫的 `let-else`，守卫继续遵循通常的候选观察与失败路径限制。

~~~flurry
fn fallback() -> bool { false }

fn require_ready(comptime config: Tuple<comptime fn() -> Type, bool>) -> bool {
    let (chosen, true) = config else { return fallback(); };
    keep(chosen, true)
}
~~~

输入的类型和值在 comptime 取得，不会把失败分支的普通调用变成 comptime invocation。`else` 仍须退出，未调用定义中的已知类型错误也仍被检查。runtime 匹配失败时，成功候选的绑定不交付，原主语保留清理责任；例如 `return fallback()` 先取得返回值，再随退出清理，与普通 `let-else` 使用同一规则。

守卫可以读取候选中的普通字段，也可以将已完成的回调传给要求 comptime 输入的函数；普通调用仍在 runtime 执行。例如：

~~~flurry
fn accept(value: bool) -> bool { value }

fn guarded_config(comptime config: Tuple<comptime fn() -> Type, bool>) -> bool {
    let (chosen, value) if accept(value) = config else { return fallback(); };
    keep(chosen, value)
}
~~~

模式成功后才求值守卫，守卫通过后才交付绑定。转换保留守卫及退出分支，即使整个输入只含可消除的 comptime 回调，也不能将条件和退出一起删除。守卫自身调用产生的临时资源按正常调用边界清理；守卫失败不重复求值，也不取消主语的清理责任。定义检查先建立候选绑定的类型与阶段身份，再检查守卫中的依赖调用；未调用定义里的已知错误仍会被拒绝。

具有 runtime 布局的子模式还可带字段守卫、`or` 内的守卫或顺序计算。例如：

~~~flurry
fn invert(value: bool) -> bool { not value }

fn transform_config(comptime config: Tuple<comptime fn() -> Type, bool>) -> bool {
    let (chosen, (value and invert(value) is result)) = config;
    keep(chosen, result)
}
~~~

`config` 在 comptime 取得，`invert` 仍在 runtime 调用，`result` 是这次普通调用的结果，不能作为已完成的 comptime 输入。相对地，守卫内部的原主语绑定，以及 `and … is …` 左侧从原输入取得的绑定，仍可保留已完成值证据供匹配成功后的依赖调用使用。这里保留的是原输入身份，既不把守卫搬到 comptime，也不把右侧计算结果视为输入别名。取址或写入仍按通常规则使对应存储的旧证据失效；此前复制出的独立别名不随源存储的后续变化而改变。这一阶段错误在定义检查时即被拒绝，不要求先调用函数。`or` 的统一绑定只有在每个候选都从原输入取得该绑定时才能继承输入阶段；候选重命名按同时映射处理，不能借交换名字把计算结果变成输入证据。在实际输入完成后，编译器还要求各候选为该绑定提供完全相同的值身份，才保留可供依赖调用使用的已完成值。例如 `((flag, _) if ready) or (_, flag)` 的两个字段都为 `true` 时可保留 `flag = true`，两个字段不同则不能猜测 runtime 守卫的选择。已完成输入若已证明某个字面量候选不可能匹配，该候选不参加一致值比较。例如 `(true, flag, _) or (false, _, flag)` 可依据已完成的第一个字段排除另一候选；仍可能匹配的候选必须给出相同绑定值。浮点字面量按运行时数值相等规则处理：正负零相等，NaN 不匹配自身。这个检查不执行守卫、不展开 normal-form 头，也不会将右侧普通计算结果纳入一致值证据。原模式树及其 runtime 守卫、失败分支和清理路径仍保留。原有取址／写入失效规则仍适用。

具有 runtime 布局的枚举字段也保留已完成的变体与载荷证据：

~~~flurry
enum Choice where T { left(T, bool), right(bool, T) }

comptime fn Pick(flag: bool) -> Type {
    if flag { bool } else { .u8'builtin }
}

fn consume_choice(comptime flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {
    value
}

fn from_choice(comptime choice: Choice<bool>) -> bool {
    let (.left(flag, _) or .right(_, flag)) = choice;
    consume_choice(flag, true)
}
~~~

实际变体决定载荷字段的位置；另一变体不参加绑定值的一致性比较。泛型实例仍保留完整 nominal 类型身份，错误实例或实际 `flag` 导致的类型不匹配照常拒绝。这只是已完成输入的证据传递，枚举的 runtime 构造、模式选择、守卫与载荷清理仍走原有路径；字段取址或写入后不能复用旧值证据。
模式计算沿原树按候选顺序执行：左侧失败便跳过右侧，失败候选撤销绑定，成功候选只交付一次。这里复用模式观察已有的调用和资源限制，尚不意味着任意拥有型中间结果都已支持。阶段专属容器自身带复杂计算时也仍需进一步接通；上述处理针对可完整保留的 runtime 子模式。

失败分支中的存储变化只影响真实可达的控制流。若该分支退出当前函数，它的取址或写入不会抹掉成功路径仍可使用的已完成输入证据；分支内部仍按写入后的状态检查。若分支通过 `break` 或 `continue` 转移到外层循环，则保留相应失效信息，不能让后续迭代或循环后的代码复用旧值。这是检查器的分支状态区分，不是对实际写入的回滚。


具名 comptime 回调也可以嵌入 tuple、结构记录、struct 和 enum 输入。冻结调用输入时保留每个回调的声明和泛型实参，注解准备递归收集这些声明的执行依赖。下面的配置把类型选择策略和调用接口分开：

~~~flurry
struct Config {
    select: comptime fn() -> Type,
}

comptime fn SelectedConfig(config: Config) -> Type {
    config.select()
}

comptime fn boolean_config() -> Config {
    Config { select: Selected<bool> }
}

fn configured(
    comptime .config: Config = boolean_config(),
    comptime .alias: Config = config,
    .value: defer SelectedConfig(alias) = true,
) -> defer SelectedConfig(config) {
    value
}

fn configured_boolean() -> bool { configured() }
~~~

默认 alias 复用已取得的 config，参数域读取其中的实际回调。取得泛型实参后才执行的 struct／enum 生产函数也可交付此类输入；IIR 冻结核对 nominal 声明、类型实参、字段布局和 enum 分支，不能只记录相同形状的字段。newtype 包装同样保留声明与表示值，包装／解包须经对应的表示声明核验，见 [newtype 的 comptime 输入](../type/user-defined-types/newtype-and-typealias.md#作为-comptime-输入)。这是调用输入的表示能力：不把具名函数变成可任意导出的纯数据，也不把捕获环境、指针或可变存储转换为不可变快照。

具名 comptime 函数实例也能作为调用结果参与泛型签名中的 normal-form 输入。
此时 Type 实参可能仍属于调用者的参数作用域，不能提前换成某个具体类型：

~~~flurry
comptime fn identity(value: T) -> T where T { value }

comptime fn callback(comptime T: Type) -> comptime fn(T) -> T {
    identity<T>
}

pure comptime fn Carrier<T, action: comptime fn(T) -> T> -> Type { bool }

fn accept(comptime T: Type, value: Carrier<T, callback(T)>) -> bool { true }
~~~

`callback(T)` 的结果保留 `identity` 的声明身份和调用者的 `T`；它不保留此次
编译期求值中以临时 Type 身份特化的函数体。随后真正调用此身份时，接收方按普通
实例化规则取得函数体并重新检查适用的约束。仅持有签名或 runtime 函数身份不能
获得 comptime 执行权限。`Carrier` 的 normal-form 头仍然不展开。

这里适用于无捕获的具名实例，包括聚合结果中的此类字段。带捕获闭包具有自身的
环境和共享状态，不能压缩成声明身份；跨上述抽象 Type 作用域恢复这种环境的实现
仍未完成。


需要冻结证据的路径仍使用受检查的闭包重建工厂。这类只读快照接纳 Type、Type 列表、builtin 标量及由它们、具名 comptime 实例和只读闭包组成的 tuple／结构记录图，不能用重建来模拟共享可变状态。泛型推断期间已能为域已形成的显式输入、默认值和位置收集保留活动图；未完成的域和未确定的声明类型实参仍须等待。runtime 函数中显式 comptime 参数的对应活动图路径仍需继续接通。指针与用户资源捕获也须分别满足交付和内存协议，不能因支持闭包状态就跳过这些检查。


从计算得到的 tuple packet 展开这些输入时，也复用 packet 已完成的值：

~~~flurry
comptime fn selector_packet() -> Tuple<Selector, bool> {
    (make_selector(bool), true)
}

const from_packet = keep_selected(...selector_packet());
~~~

普通调用中的类型注解直接使用所选闭包的活动值。不能为了获取一个字段的类型依赖而重新求值 producer；交付最终调用时还必须保留整个 packet 中的共享关系，包括未参与注解计算的字段。需要冻结证据的其他路径仍遵守只读快照边界。


例如可以把选择器包装在另一个闭包中：

~~~flurry
typealias WrappedSelector = %{inner: Selector} comptime fn() -> Type;

comptime fn wrap_selector(inner: Selector) -> WrappedSelector {
    || inner()
}

comptime fn keep_wrapped(
    select: WrappedSelector,
    value: defer select(),
) -> defer select() {
    value
}

const wrapped = keep_wrapped(wrap_selector(make_selector(bool)), true);
~~~

活动值传递同样保留内层选择器的状态。对于仍要求冻结证据的路径，外层只读不足以证明整个捕获图只读：若内层选择器会写入自己的捕获状态，包装后仍不能作为只读快照。嵌套图中的具名泛型函数实例同样保留特化实参，并检查实参在当前类型作用域内合法，而不是只保存声明身份。


冻结路径的只读检查按闭包环境划分作用域，不把新建局部闭包的主体误认成对外层捕获的访问。例如外层每次调用都可以构造一个具有独立计数状态的局部闭包，再使用它计算结果。捕获初始化、默认实参及 handler 分支仍属于需要检查的路径；不能简单跳过所有嵌套控制流来获得这个行为。已经存在于捕获图中的闭包仍逐个接受完整的只读环境检查。


泛型声明的 Type 参数也可进入只读捕获图。快照工厂将这些稳定的类型绑定列为显式参数，调用时替换；不会把它们当作无作用域的闭合常量：

~~~flurry
comptime fn forward_selected(value: T) -> T where T {
    keep_selected(make_selector(T), value)
}

const forwarded = forward_selected<bool>(true);
~~~

同一泛型主体可以用不同 Type 实参调用，也可以先包装闭包或从 packet 展开它。恢复注解时，输入携带的抽象 Type 身份及调用者已成立的类型前提仍然有效；因此闭包内部对泛型函数的 `requires` 检查可以使用合法的声明前提。调用者自身的约束仍须满足，定义侧对未知输入的检查也不因此放宽。这里保存的是稳定的声明绑定及显式替换，不是活动推断变量；模块常量导出仍使用闭合边界。


活动选择器可以让同一签名的两次注解求值得到不同结果：

~~~flurry
typealias AlternatingSelector = %{flag: bool} comptime fn() -> Type;

comptime fn alternating_selector() -> AlternatingSelector {
    var flag = false;
    || {
        flag = not flag;
        if flag { bool } else { .u8'builtin }
    }
}

comptime fn choose(
    select: AlternatingSelector,
    value: defer select(),
) -> defer select() {
    7
}

// 参数域第一次调用 select 得到 bool，返回域第二次调用得到 u8。
const chosen = choose(alternating_selector(), true);
~~~

准备 `value` 的交付时不再次执行已经形成的域。最终函数体接收到的 `select` 保留两次注解调用后的状态；如果主体再调用它，将得到第三次结果 bool。不同调用形成的具体签名分别参与实例身份，不能因为活动参数没有冻结符号而错误复用之前的实例。


普通 comptime 调用需要先完成默认参数才能计算签名时，默认表达式也可以返回或使用活动闭包：

~~~flurry
comptime fn choose_default(
    .select: AlternatingSelector = alternating_selector(),
    .value: defer select() = true,
) -> defer select() {
    7
}

const with_default = choose_default();
const with_explicit = choose_default(.select = alternating_selector());
~~~

这里省略的 `select` 只构造一次。默认值可以直接返回前面已有的闭包，或构造捕获该闭包的新闭包；两者都保留共享状态。已经完成的默认值直接交付，不能在最终调用时重新执行默认表达式，也不能把它的共享图放进一个看不到原绑定的新默认求值环境。

默认表达式按声明顺序执行。即使某个较早默认值的结果不参与签名计算，它的效果仍先于后面被签名依赖的默认值；显式覆盖某项则不执行该项默认表达式。最终交付时，默认值与显式输入、展开包中的字段一起保留共享捕获图，包括默认闭包通过内层捕获间接引用 packet 字段的情况。


位置变参的收集值也可以保存活动闭包图。例如，两个收集元素可以引用同一个选择器：

~~~flurry
comptime fn selector_pair() -> Tuple<AlternatingSelector, AlternatingSelector> {
    let select = alternating_selector();
    (select, select)
}

comptime fn SelectFirst(selectors: Tuple<AlternatingSelector, AlternatingSelector>) -> Type {
    let (first, second) = selectors;
    first()
}

comptime fn SelectSecond(selectors: Tuple<AlternatingSelector, AlternatingSelector>) -> Type {
    let (first, second) = selectors;
    second()
}

comptime fn collect_choice(
    ...selectors: Tuple<AlternatingSelector, AlternatingSelector>,
    .value: defer SelectFirst(selectors) = true,
) -> defer SelectSecond(selectors) {
    7
}

const collected_choice = collect_choice(...selector_pair());
~~~

这里第一次选择得到 bool，第二次得到 u8。收集保留各个元素的值身份，不因闭包主体和初始捕获值相同就合并独立 producer；默认参数返回该收集值时也保留其中的别名。共享关系同时覆盖固定参数及 packet 中未被注解读取的嵌套字段。

收集元素和交错的命名实参仍按源码顺序取得，不能为了先完成一个收集而越过中间实参的失败。空收集没有元素 producer，但它的依赖域仍在声明顺序走到该参数时形成，不能提前执行或重复执行域中的 comptime 调用。收集值保持原 tuple 类型身份；这些图传递规则不改变 Tuple normal-form 头的规则。


泛型推断与活动参数准备可以交织：

~~~flurry
comptime fn inferred_choice(
    select: AlternatingSelector,
    value: defer select(),
    marker: T,
) -> defer select() where T {
    7
}

const inferred_choice_result = inferred_choice(alternating_selector(), true, false);
~~~

准备 `select` 时不需要先确定 T；它的域已经完成。由它形成的参数域和返回域可以参与后续检查，而 T 从 marker 的类型取得。推断所得的调用继续使用这份活动图和已经形成的域，不重新执行 producer 或注解。默认值、位置收集及 packet 中的共享引用遵循相同规则。

反过来，若默认表达式本身依赖尚未确定的声明类型实参，例如 `make_selector(T)`，则该默认计算等待实参确定后再执行，不能把未知 T 当作完成的闭包捕获值交付。后面的省略默认值也不能越过这个等待项。准备对象保留的是本次调用状态；复用已检查的函数实例不会让不同调用共享这份状态。
