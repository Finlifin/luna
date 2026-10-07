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

按声明次序建立依赖图：先登记子句绑定，再登记形参类型及其供应表达式，最后登记结果和结果契约。对每条边记录需要的是具体值、类型事实还是 runtime 符号；只有第一类要求当场完成对应计算。由此可区分缺少 comptime 值、缺少操作依据和缺少代码上下文三种错误。
