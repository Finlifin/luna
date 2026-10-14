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

### 依赖次序

子句参数依次建立；普通参数在这些绑定下依次建立；返回值再引用已经建立的参数及子句参数。组合中的后项可以依赖前项，不能引用尚未建立的后项。

这个次序约束签名与参数交付，不把显式 runtime 实参改成按形参排列求值。调用点仍先按源码求显式实参，再按依赖顺序补缺省。

定义检查使用接口已经提供的关系，不要求先知道所有具体输入。未知 T 的合法操作来自类型要求；未知 runtime 值的访问与算术条件来自程序契约。二者都不能以“某次实例化也许可以运行”代替定义端要求。

### 裸标识符与 trait bound

裸 id 由声明语境解释：lambda 中省略参数类型，依据上下文和使用检查；typealias/newtype、类型子句参数及 normal form head 中的类型参数是 `id: Type` 的简写。

`T :- A + B` 是子句参数 T 加上正向 trait 要求的简写，等价于 `T, requires T :- A + B`。否定要求写入 requires，不扩张 `:-` 的组合语法。

返回 MemoryEffect 的 predicate 中，裸参数 id 简写 `quote id: lift Any`。普通 predicate 默认返回 bool，不因此自动 quote 所有参数。

### 子句位置输入的展开

类型子句实参可通过 `...packet` 展开一个 Type 值元组。位置输入跳过 optional 槽位；`.name = value` 显式覆盖对应可选项。先按源码次序求显式输入，再按声明次序补默认值，空包的生产表达式也必须执行。

~~~flurry
typealias Type = .Type'builtin;

struct Pair where A, .B: Type = A {
    first: A,
    second: B,
}

comptime fn packet(T: Type) -> (Type,) { (T,) }

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

混合 Type／值参数的范式头也支持字面元组展开。先展开位置，再由前面的类型输入为后面的字面量提供上下文：

~~~flurry
pure comptime fn Tagged<T, value: T> -> Type { T }
typealias Seven = Tagged<...(.u8'builtin, 7)>;
~~~

这里的 `7` 按 `.u8'builtin` 检查；不会先把整个包的数字都默认为 Integer。展开后仍检查值域、依赖类型、参数个数和声明要求。带副作用的元素只求值一次，提前退出会跳过后续元素和默认值。

固定参数的混合头也可以展开计算所得元组：

~~~flurry
typealias Byte = .u8'builtin;
comptime fn packet() -> (Type, Byte) { (Byte, 7) }
comptime fn BuildTag() -> Type { Tagged<...packet()> }
~~~

该包已经具有 `(Type, Byte)` 类型，其值按位置交付；取得第一个 Type 值后，仍须检查第二个值是否符合依赖参数 `value: T`。包中的 Byte 不会为了适应另一个 T 而被悄悄重新解释。输入计划保留包生产、解构与每项交付的次序；命名参数可以在包前后求值，尾随空包也不能被删除。默认值只复用已取得的绑定，循环中每次执行都重新建立并清理这组绑定。

普通值的跨阶段 Splice 协议不承担这些子句位置路由。即使已绑定 Splice，类型注解中的包仍在 comptime 求值，不请求 runtime 转换。

包的位置数能够确定时，目标参数类型也会传入包生产表达式。例如，下面两个分支中的数字都取得 Byte 上下文：

~~~flurry
pure comptime fn PairTag<first: Byte, second: Byte> -> Type { bool }
comptime fn Choose(flag: bool) -> Type {
    PairTag<...if flag { (7, 8) } else { (9, 10) }>
}
~~~

block、match 和可从结果上下文推导的泛型包生产函数遵循相同规则。某个字段仍依赖未知 Type 时，该字段保持未决，其他已确定字段的上下文继续传递；这不构成对未决依赖的证明。多个包的位置数都未知时不能任意划分目标槽位，调用者可以通过生产函数签名或元素类型补足信息。已确定为 Integer 的包也不会被重新解释为 Byte 包。

纯类型参数入口同样提供结果上下文。例如 `Pair<...packet()>` 可以为泛型 `packet` 提供 `(Type, Type)` 的目标，从而确定其返回类型。struct、enum、typealias、trait 和函数／方法实例使用同一规则；optional 参数不占位置槽，方法已有的接收者类型实参也不挤占方法自己的展开槽。包依然只求值一次，类型上下文不代替对生产函数要求的检查。

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
