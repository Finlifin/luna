# 接口声明与契约

## Intro

Trait 声明告诉调用者可以依赖什么。方法描述操作，关联项描述操作共同使用的类型或值，默认成员则把可复用代码放在接口中。

接口越准确，使用它的函数就越少依赖某个具体实现的细节。

## 语法

~~~text
trait_def -> (pure comptime)? trait id (:- expr)? clauses? { (assoc_decl | definition | statement)* }
assoc_decl -> assoc id(<parameter*>)?: expr (= default)? clauses?
impl_trait_def -> comptime? impl expr for expr clauses? { (assoc_decl | definition | statement)* }
trait_bound_decl_clause -> id :- expr
normal_form_head_application -> expr<argument*>
~~~

Trait 内的方法可以只有签名，末尾使用分号；带默认主体的方法采用普通函数定义。属性和执行阶段见[两相](../comptime/the-comptime-phases-of-traits.md)。

`pure comptime trait` 将整个接口限定为 comptime phase，`pure` 在这里表示 only，
不表示方法纯度。关联项、方法和默认成员无需逐项添加 phase 属性；实现仍写
`comptime impl`，需要在编译时执行的方法仍显式声明 `comptime fn`。普通 trait
与 pure comptime trait 可以分别为同一类型提供同名方法；未限定的 comptime
成员查找优先使用后者的合法来源，完整限制见上述两相章节。

父接口和 trait bound 中 `:-` 右侧的 expr 可以组合多个约束，例如 `trait Report :- Reading + Named`、`where T :- Reading + Named`。这里复用表达式的 `+` 形式；否定约束写在子句中，例如 `requires not (T :- Disabled)`。

声明类型变量与约束该变量有两种写法：`where T, requires T :- Reading + Named` 先声明 T，再用 requires 子句中的类型命题约束它；`where T :- Reading + Named` 则是声明并约束 T 的语法糖。`requires` 本身不声明变量，也不是放在变量名与 trait 表达式之间的运算符，因此 `where T requires Reading` 非法。例如，迭代器适配器的子句可以写成 `where I :- Iterator, U`，也可以写成 `where I, U, requires I :- Iterator`。

## 正式定义

### 身份与参数

每个 trait 声明具有独立身份。完整接口实例由该身份及全部语义实参确定；接口参数可以是类型，也可以是契约允许的其他值。关联项是实现提供的结果，不因被查询就变成接口输入。

Implementation 主体须是允许用户实现的类型值，包括满足这一条件的运行时依赖类型。普通对象通过它的类型、实现证据和环境使用接口，不因当前内容不同而单独产生实现身份。

Reachability 等嵌入层类型、CMTT 类型和闭包类型不作为用户 impl 的主体。限制针对最终主体，直接实现和泛型实现的匹配都必须检查；typealias 不隐藏真实主体。合法名义包装按自己的身份及资源契约独立检查，不据“含有特殊字段”就一概禁止。

语言提供的调用、代码接入或资源能力不要求用户为这些类型手写 impl。类型能作为值存在，也不意味着用户可以替它构造语言所要求的代码项、绑定或环境。

### 关联项

关联项允许声明任意合法类型的值，`assoc Element: Type` 只是其中一种。其声明阶段、求值和使用仍遵守对应类型与阶段契约；允许任意类型不等于允许运行时对象内容参与任意编译时计算。

关联项通过选定的完整接口实例取得。两个接口都声明 Element，不意味着两个 Element 相等。依赖其他关联项的成员在替换后重新检查要求；关联项的循环定义不能作为自身成立的证明。

有默认值的关联项可由实现替换。没有默认的必需项必须提供；不能以另一 phase 或另一 trait 的同名成员补齐。

泛型关联值的初始化可以依赖接口值参数，并调用 comptime 函数。此时先检查初始化程序，待选择具体实现并完成参数替换后再执行；不能在泛型定义处把尚未确定的参数当作闭合常量求值。

```flurry
comptime fn invert(value: bool) -> bool { not value }

trait Flags where enabled: bool {
    assoc inverted: bool = invert(enabled);
}

struct Switch where enabled: bool {}
impl Flags<enabled> for Switch<enabled> where enabled: bool {}

fn check() -> bool {
    not Switch<true>.as(Flags<true>).inverted
        and Switch<false>.as(Flags<false>).inverted
}
```

这里两个完整接口实例得到不同的关联值。初始化遵守 comptime 调用的前置条件、阶段限制和求值预算；初始化程序不会作为运行时代码遗留在具体实例中。关联项之间可以引用，但循环初始化不能借由延迟求值获得合法性。

初始化也可以选择并返回运行时函数值。冻结结果保留函数声明及泛型实参；trait 方法值还保留接口来源、Self 与接口／方法实参。选择函数在 comptime 执行，所选运行时函数体只在实际调用时执行。当前普通 callable 尚不能承载程序契约时，带程序契约的 trait 方法值明确拒绝转换，不能退化为实现函数来绕过接口要求。


### 默认方法

默认主体在 trait 声明提供的要求下检查。它可以使用同一接口的其他成员和已声明的父接口能力；不能依赖某个具体实现碰巧存在的字段或方法。

实现采用默认主体时，代入 Self、接口实参和关联项，再兑现相应义务。替换默认成员不会产生第二份 implementation，也不形成按导入顺序选择的重载。

### 父接口与契约替换

`trait D :- A + B + C` 要求 D 的实现主体同时满足 A、B、C；`where T :- A + B + C` 对 T 提出相同的能力合取。各项都可以是带实参的完整 trait 表达式，不限于单个声明名。

合取要求每项能力分别成立，不合并各 trait 的声明身份，也不按书写顺序赋予同名成员优先级。提供 D 中同名方法不会自动创建父接口的实现。泛型环境有 D 的合法证据时，可以使用其要求的各项能力；实际调用仍保留各自来源。

这两个 `:-` 位置用于正向能力组合。否定要求放进子句的 requires，例如 `where T :- A + B, requires not (T :- C)`。否定需要有效证明；没有找到 C 的实现或查询仍未决，不自动证明 T 不满足 C。

实现方法的参数、接收者、返回值及阶段必须兑现接口。前提只能在合法替换关系下减弱，结果保证只能保持或加强；错误、效应和环境需求不能超出接口允许的范围。可接受差异必须有已定义的适配依据，不由此开放任意参数逆变或结果协变。

## 常见示例

以下接口同时使用关联类型、关联整数和默认方法：

~~~flurry
trait SampleSource {
    assoc Element: Type;
    assoc capacity: usize = 1;

    fn sample(*self) -> Element;

    fn declared_capacity(*self) -> usize {
        Self.as(SampleSource).capacity
    }
}

struct Counter {
    value: u32,
}

impl SampleSource for Counter {
    assoc Element: Type = u32;

    fn sample(*self) -> u32 {
        self.value.*
    }
}

test {
    let counter = Counter { value: 12 }
    let sample: Counter.as(SampleSource).Element = counter.sample();
    println("{} {}", sample, counter.declared_capacity());
}
~~~

输出 `12 1`。capacity 是关联的整数值，Element 是关联的类型值。默认方法不访问 Counter 的字段，因此可以在接口定义处检查。实现也可以显式提供另一个 capacity 值。

多个父接口分别提供能力，不重复生成实现：

~~~flurry
trait Reading {
    fn read(*self) -> u32;
}

trait Named {
    fn name(*self) -> str;
}

trait Report :- Reading + Named {
    fn report(*self) {
        println("{}: {}", self.as(Named).name(), self.as(Reading).read());
    }
}

struct Meter {
    value: u32,
}

impl Reading for Meter {
    fn read(*self) -> u32 {
        self.value.*
    }
}

impl Named for Meter {
    fn name(*self) -> str {
        "meter"
    }
}

impl Report for Meter {}

fn show_reading(value: *T)
where T :- Reading + Named
{
    println("{}: {}", value.as(Named).name(), value.as(Reading).read());
}

test {
    let meter = Meter { value: 8 }
    meter.report();
    show_reading(meter.ref);
}
~~~

两次输出均为 `meter: 8`。Report 的父接口组合和 show_reading 的参数约束都要求 Reading、Named 分别成立。移除其中任一实现，空的 Report 实现就不能满足父接口要求。若把 read 改成返回 str，也不能兑现 Reading 的 u32 结果。

否定要求使用普通子句表达：

~~~flurry
-- 接续上例的 Reading、Named 声明。
trait Disabled {}

fn show_enabled(value: *T)
where T :- Reading + Named,
    requires not (T :- Disabled)
{
    show_reading(value);
}
~~~

调用方既要交付两项正向能力，也要证明否定要求。该函数没有把否定能力加入父接口列表或 `+` 组合。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 同样有默认方法、关联项和 supertrait；其关联类型与关联常量采用不同声明形式。Flurry 的 assoc 直接给出关联值的类型。[Rust traits](https://doc.rust-lang.org/reference/items/traits.html)

Zig 可用 comptime 检查类型上的声明；C 可将相关常量和函数表一同传入；C++ 的 concepts 检查表达式要求，类模板提供成员类型和实现。它们都不等于 Flurry 中一份名义 trait implementation 的关联项来源。[Zig comptime](https://ziglang.org/documentation/master/#comptime)、[C11 §6.7.6.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[C++ constraints](https://eel.is/c++draft/temp.constr)

## 运行时实现设计引导

类型关联项和编译时已确定的常量可以消去；运行时操作实际使用的值则通过接口允许的环境取得。默认方法调用其他成员时，应使用同一实现证据支持的入口，而不是再次按字符串查找名字。

父接口调用可静态展开，也可从动态接口携带的证据取得入口；两种情况都必须兑现父接口要求。

## 编译器实现引导

1. 为 trait 建立声明身份、参数上下文和成员表；分别记录 phase 与执行阶段。
2. 在声明要求下检查关联项默认值和默认主体；建立关联项依赖图。
3. 对具体实现匹配成员，补入默认项，检查缺项和重复项。
4. 代入关联项后验证所有类型、阶段、前提和结果保证。
5. 将父接口与 trait bound 的 `+` 组合分解为对同一主体的逐项能力义务，分别取得有效证据；保留各自成员来源，检测递归要求中的无进展环。
6. 输出完整成员表和证据；失败不能登记为已成立的实现。

契约检查可按内部义务 `trait 前提 ⇒ impl 前提`、`impl 结果保证 ⇒ trait 结果保证` 组织，但两条蕴含还需共同的合法签名与适配前提。效果检查使用已定义的行关系，不能直接当作普通布尔蕴含。

子句中的否定交给 PTC 的否定查询处理，保留实现世界及其排除依据；不能把正向候选检索的失败直接翻转为否定证据。

依赖图处理在固定图上为 O(V+E)；具体关联项计算和 PTC 查询的成本另计。求值超限与循环是诊断，不是任选默认值的理由。

### 初始化返回匿名编译时函数

关联值初始化也可以返回匿名 comptime callable。无捕获 lambda 保留其已检查的主体和参数绑定；冻结本身不调用主体，也不将 lambda 伪装成具名函数。以下代码在具体实现选定后取得回调，在编译时调用它，再把结果交给 runtime 实例：

~~~flurry
comptime fn select(enabled: bool) -> comptime fn(bool) -> bool {
    if enabled { |value| not value } else { |value| value }
}
fn decide(comptime flag: bool) -> bool { flag }
trait Flags where enabled: bool {
    assoc callback: comptime fn(bool) -> bool = select(enabled);
    fn get() -> bool { decide(Self.as(Flags<enabled>).callback(true)) }
}
struct Holder where enabled: bool {}
impl Flags<enabled> for Holder<enabled> where enabled: bool {}
fn run() -> bool {
    not Holder<true>.as(Flags<true>).get()
        and Holder<false>.as(Flags<false>).get()
}
~~~

当前这条分阶段关联值冻结路径支持无捕获匿名函数；带捕获环境的函数还需要环境图传递，不能通过丢弃捕获或提前执行主体来替代。

### 名义类型参数提供的域证据

名义类型的值参数可传给 trait 的值参数。定义检查使用其声明域，随后才匹配实现；不会因为目标 trait 要求 `bool` 就将未知参数认作 `bool`。例如，接续上面的 `Flags`、`select` 和 `decide`：

~~~flurry
struct Switch where enabled: bool {
    pub fn get() -> bool {
        decide(Switch<enabled>.as(Flags<enabled>).callback(true))
    }
}
impl Flags<enabled> for Switch<enabled> where enabled: bool {}
~~~

泛型定义内的 `enabled` 尚未取具体值，但它的声明身份和 `bool` 域已经确定。实例化后按实际参数执行关联初始化。不同参数即使域相同，也不能因此判为同一个值；值域证据不替代类型要求或程序规格。

名义类型的 trait bound 也保留接口提供的类型保证，包括经父 trait 继承的保证。下面的 `convert` 在泛型定义处即可使用 `S == T`，不需要等到 `bool` 实例出现：

~~~flurry
trait Same where T, enabled: bool, requires Self == T {}
impl Same<T, enabled> for T where T, enabled: bool {}
struct Conversion where enabled: bool, T, S :- Same<T, enabled> {
    pub fn convert(value: S) -> T { value }
}
fn example() -> bool { Conversion<true, bool, bool>.convert(true) }
~~~

去掉 `S` 的接口约束后，函数体不再有 `S == T` 的依据。传入不满足保证的实例也会拒绝。接口中只约束参数、而不涉及 Self 的形成条件仍须独立建立，不能从“假定该接口成立”反向证明该接口可以形成。

### 调用带捕获的关联 comptime 回调

当一次关联 comptime 回调调用的输入已经确定、表达式不再依赖调用者局部绑定时，实例化可以把关联初始化与调用放在同一次编译时执行中，只冻结最终结果。环境在调用结束前保持活跃，不必先将闭包本身冻结成独立的跨阶段值：

~~~flurry
typealias Callback = %{flag: bool} comptime fn() -> bool;
comptime fn make(flag: bool) -> Callback { || flag }
trait Flags where enabled: bool {
    assoc callback: Callback = make(enabled);
}
fn decide(comptime value: bool) -> bool { value }
struct Holder where enabled: bool {
    pub fn get() -> bool {
        decide(Holder<enabled>.as(Flags<enabled>).callback())
    }
}
impl Flags<enabled> for Holder<enabled> where enabled: bool {}
fn example() -> bool { Holder<true>.get() and not Holder<false>.get() }
~~~

一次调用中的嵌套闭包与可变捕获保留其状态，程序条件仍检查。该路径不代表带捕获闭包已能任意跨阶段传递；闭包作为返回值或独立实例输入离开这次调用时，仍需要相应的环境传递支持。runtime 回调不因此获得编译时执行权限。

必需实参可以先建立局部绑定，再调用关联回调。例如，将上例的回调改成接收一个布尔实参：

~~~flurry
typealias Callback = %{flag: bool} comptime fn(bool) -> bool;
comptime fn make(flag: bool) -> Callback { |value| flag and value }
comptime fn identity(value: bool) -> bool where requires value { value }

// Holder.get 的主体：
decide(do {
    let value = if enabled { identity(true) } else { false };
    Holder<enabled>.as(Flags<enabled>).callback(value)
})
~~~

`enabled` 尚未确定时，整个实参保留到实例化；局部绑定、分支和回调在同一次 invocation 中求值。`identity` 的前置条件仍须成立。程序验证在关联值尚未完成时只知道它的类型范围，不会根据其初始化函数猜测值，也不会将程序条件交给 PTC。
