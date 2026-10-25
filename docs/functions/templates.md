# 函数模板与测试模板

## Intro

多个定义可以共用一份签名。需要复用更多代码时，模板还可以建立局部上下文，再把使用者提供的正文接进去。

这两种用法有不同的复杂度。只想统一参数和结果时，不必先建立一个代码生成框架。

## 语法

~~~text
template_signature -> template fn id(parameter*) (-> result)? clauses?
template_quote_signature -> template quote fn id(parameter*) (-> result)? clauses?
template_function -> template fn id(parameter*) (-> result)? clauses? block
template_instance -> fn id use expr block
    | inline fn id use expr block
    | quote fn id use expr block
body_parameter -> quote lambda parameter
template_test -> template test id clauses block
test_template_instance -> test id use expr block
contextual_expr_type -> < runtime_binding* |- expr >
splice_expr -> ~<expr>
case_param -> case pattern
~~~

这些产生式展示本篇使用的普通、inline 和 quote 函数形式。模板可复用所有非 comptime fn 的接口；具体实例仍需满足所用函数修饰及其原有契约。无主体的签名声明以分号结束。

## 正式定义

### 签名抽象

简单模板提供参数、返回值及相关要求；实例提供主体。实例中的参数名字和依赖按照模板接口绑定，不靠运行时查表取得。

模板不因为复用签名就改变实例的函数类别。普通实例按普通规则调用，quote fn 实例保证代码接入。包含 comptime 参数，也不等于整个实例是 comptime fn。

模板实例继承的是接口中的参数、结果和约束。参数名在实例正文中指向本次接口绑定；函数名则属于实例自身。两份实例可以共享接口却具有不同主体、不同效果和不同的实现代码。

默认回调也属于复用的接口。例如，下面的默认值构造一个回调；调用实例时，实例正文才调用它。两个 `apply` 各自建立 tactic lambda 边界，因此内层 `_0` 指内层接收的 `false`，不与外层 `_0` 共用参数。

~~~flurry
fn apply(value: bool, lambda action: fn(bool) -> bool) -> bool {
    action(value)
}
fn always(value: bool) -> bool { true }

template fn choose(
    value: bool,
    .action: fn(bool) -> bool = |input: bool|
        apply(input, _0 and not apply(false, _0))
) -> bool;

fn first use choose { action(value) }
fn second use choose { action(value) }

fn run() -> bool {
    first(true) and not second(false) and first(false, .action = always)
}
~~~

`run()` 返回 `true`。显式 `.action` 覆盖本次默认回调；签名克隆保留 tactic 的边界，模板参数和闭包捕获则映射到本实例的新鲜绑定。默认值形成仍需普通默认值的稳定性与终止性证据，复用签名不免除这些要求。

默认值内部也可以形成代码值并接入 runtime 端口：

~~~flurry
template fn invert(
    seed: bool,
    .value: bool = do {
        const code: <seed: bool |- bool> = '(not seed);
        ~<code>
    }
) -> bool;

fn first_invert use invert { value }
fn second_invert use invert { value }

fn check_invert() -> bool {
    first_invert(false) and not second_invert(true)
}
~~~

`code` 是 comptime 代码值，其中 `seed` 是 runtime 上下文端口。隐式接入沿模板声明保存的词法关联连接到本实例的 `seed`，不按实例正文中新出现的同名变量重新查找。复制签名时，contextual 绑定、quote 内部绑定以及 pattern quote 的成功输出一并建立新鲜身份；显式 substitution 和单洞限制仍照常检查。这里接入的是默认值中的代码；函数主体仍由各实例提供。

不要把“使用模板定义一个函数”与“调用这个函数”混为一步。前者决定具体定义及其代码，后者才按该实例的普通或 quote 调用规则供应 runtime 输入。

### 基于 CMTT 的正文复用

高级 template fn 通过 quote lambda 参数接收正文。代码接口与规则聚合见 [CMTT 模板接入](../cmtt/quote-functions-and-templates.md)。quote 保留代码，lambda 标记正文入口；模板可以先建立接口授予的 runtime 绑定，再通过 splice 接入用户正文。

上下文只能包含 runtime 绑定。编译时元信息在自己的阶段处理，不能把全部模板描述都放进代码的 runtime 上下文。

接入使用绑定对应，保留类型与依赖。默认通过合法的 implicit substitution 取得对应；无须每次把相同的映射手写一遍。两次实例化分别建立自己的局部身份，不能因为接口中的变量同名就共享存储。

模板中的普通后续语句不自动成为退出清理。真正需要覆盖所有出口的责任仍使用资源与清理机制，不能依赖“写在 splice 后面就总会执行”。

| 参与者 | 提供的内容 | 检查依据 |
|---|---|---|
| 模板声明 | 签名、共同主体、正文的 contextual 接口 | 参数阶段、局部依赖和代码接入规则 |
| `fn name use template` | 本实例的正文 | 模板签名与授予正文的绑定 |
| 实例的调用者 | 本次 runtime 实参及所需编译时输入 | 实例化后的调用接口与契约 |

quote lambda 正文参数在定义实例时绑定，不是让每次 runtime 调用再传一份代码。模板私有局部变量也不自动全部暴露给正文；正文依其明确的上下文接口使用 runtime 绑定。

### 测试正文

template test 通过唯一一个锚定子句参数绑定测试正文。唯一性针对这一种参数角色，不禁止其他允许的子句输入。

模板提供初始化和合法上下文，测试正文在该上下文下检查；实例退出沿实际控制路径清理。一个普通测试不需要为了获得相同的资源清理而先变成模板。

### 分散定义的衔接

template 的 open code 能力还可以组织多入口规则集合，通过 quote fn 与函数级模式表达 quoted arm。模式、成功绑定及主体需要一起交付，不能把同名局部变量当作已有绑定关系。该机制适合编译器 pass 的大量规则等场景；完整分工见[函数级模式与分散定义](pattern-functions.md)。

## 常见示例

### 只共用签名

~~~flurry
template fn unary_u32(value: u32) -> u32;

fn doubled use unary_u32 {
    value + value
}

quote fn doubled_code use unary_u32 {
    value + value
}

test {
    println("{} {}", doubled(6), doubled_code(6));
}
~~~

两个结果都是 `12`。doubled_code 的应用必须展开；模板本身没有把 doubled 改成 quote fn。

### 同一签名的普通与 inline 实例

~~~flurry
template fn bounded(value: u32, limit: u32) -> result: u32
where ensures result <= limit;

fn clamp use bounded {
    if value > limit {
        limit
    } else {
        value
    }
}

inline fn zero use bounded {
    0
}

test {
    println("{} {}", clamp(20, 8), zero(20, 8));
}
~~~

输出 `8 0`。两个主体都必须兑现 result 不超过 limit 的保证；inline 只改变该实例的内联意向。若把 clamp 的主体直接改为 value，输入 value 大于 limit 时便违背共享接口。

### 模板建立局部上下文

~~~flurry
template fn with_adjusted(
    value: u32,
    quote lambda body: < adjusted: u32 |- u32 >,
) -> u32 {
    let adjusted = value + 1;
    ~<body>
}

fn doubled_adjusted use with_adjusted {
    adjusted * 2
}

test {
    println("{}", doubled_adjusted(20));
}
~~~

输出 `42`。body 在模板授予的 adjusted 下检查；调用 doubled_adjusted 时只提供实例接口中的 value，不再手工传入一份 body。模板接入将 adjusted 对应到本次建立的局部值。

### 两个实例不共享模板局部变量

~~~flurry
fn keep_adjusted use with_adjusted {
    adjusted
}

test {
    let first = doubled_adjusted(20);
    let second = keep_adjusted(5);
    println("{} {}", first, second);
}
~~~

输出 `42 6`。第二个实例不会读取前一个实例的 adjusted。

### 带提前失败与清理的共同主体

下面的模板为每次请求建立一个有清理行为的局部对象。只有输入存在时，才向具体正文提供 number。

~~~flurry
struct RequestTrace {
    id: u32,
}

impl Drop for RequestTrace {
    fn drop(*self) {
        println("finish request {}", self.id.*);
    }
}

template fn checked_request(
    request: ?u32,
    quote lambda body: < number: u32 |- u32 >,
) -> ?u32 {
    let trace = RequestTrace { id: 1 }
    let number = request?;
    ~<body>
}

fn halve_request use checked_request {
    println("halve {}", number);
    number / 2
}

fn clamp_request use checked_request {
    if number > 10 {
        10
    } else {
        number
    }
}

test {
    halve_request(8) match {
        value? => println("result {}", value),
        null => println("absent"),
    }
    clamp_request(null) match {
        value? => println("result {}", value),
        null => println("absent"),
    }
}
~~~

输出依次为 `halve 8`、`finish request 1`、`result 4`、`finish request 1`、`absent`。第二次在共同主体中传播缺失，正文不执行，trace 仍须清理。number 只在成功路径绑定，因此正文不能接入缺失路径。

这个例子只需要模板提供一个正文入口。无需把两个普通处理函数组织成 open code 规则集合。

### 正文只能使用公开的上下文

接续 checked_request，以下是拒绝例：

~~~flurry
fn inspect_template_local use checked_request {
    trace.id
}
~~~

trace 是模板内部局部变量；正文接口只提供 number。模板中存在一个同名变量，不意味着正文已经获得它的绑定。扩大接口必须明确改变 contextual 类型及实际交付，而不是让文本拼接绕过检查。

### 测试模板提供上下文

~~~flurry
template test with_counter
where body: < counter: *u32 |- void >
{
    let value: u32 = 0;
    let counter = value.ref;
    ~<body>;
    println("final {}", value);
}

test increment use with_counter {
    counter.* += 1;
}
~~~

输出 `final 1`。body 是唯一锚定子句参数，counter 是正文可用的 runtime 绑定。测试退出时按实际路径处理局部值；若正文提前离开，不能把后面的 println 当作保证执行的清理动作。

### 每个测试有自己的初始化

~~~flurry
test increment_twice use with_counter {
    counter.* += 1;
    counter.* += 1;
}
~~~

接续 with_counter，这个测试单独输出 `final 2`。它的初值仍为 0，不承接 increment 测试的状态；两个测试的总体运行次序由测试执行机制决定，本例不依赖它们的先后。

### 测试模板也保留资源退出

接续 RequestTrace：

~~~flurry
template test traced_test
where body: < |- void >
{
    let trace = RequestTrace { id: 2 }
    ~<body>;
}

test prints_message use traced_test {
    println("test body");
}
~~~

该测试输出 `test body`、`finish request 2`。空 contextual 接口表示正文不需要模板局部绑定，不表示代码纯或没有 runtime 行为。清理来自 trace 的责任，不来自一条放在 splice 后面的普通语句。

## 与 Rust / Zig / C / C++ 的关键差异

C++ 函数模板按模板实参形成函数实例；C 的函数式宏替换预处理 token。Flurry 的签名模板与带绑定上下文的正文接入分别承担接口复用和代码复用，不能只按文本替换理解。[C++ function templates](https://eel.is/c++draft/temp.fct)、[C11 §6.10.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)

Rust 宏与 Zig comptime 可承担各自语言中的代码组织任务；Flurry 的这套模板不是它们的同名机制，其正文必须兑现明确的 CMTT 上下文及接入限制。[Rust macros](https://doc.rust-lang.org/reference/macros.html)、[Zig comptime](https://ziglang.org/documentation/master/#comptime)

## 运行时实现设计引导

模板实例交付检查后的函数或测试主体。运行时无需查找模板定义，也不把正文作为源码字符串解释。

模板建立的局部值与用户正文共享本实例的合法绑定关系。接入后仍按真实控制流计算、返回和清理；不因为代码来自两个定义位置，就执行两套互不知情的析构。

函数级分支可实现为匹配决策结构；quote arm 则在接入后成为目标匹配的一部分。布局或代码共享不得把一个分支引入的绑定交给另一个未成功匹配的分支。

## 编译器实现引导

### 签名与正文

1. 确定模板及其实参，检查实例所属的非 comptime 函数类别。
2. 建立签名绑定映射，实例主体在该接口下检查。
3. 若存在 quote lambda 正文参数，取得代码描述及其 contextual 接口。
4. 在模板建立绑定的位置取得对应，检查类型、依赖及单洞要求。
5. 以绑定替换接入，交付完整控制流，再执行普通函数的资源和契约检查。

模板缓存保留定义、实例类别、语义输入及上下文接口；不能只用 body 的文字内容作为身份。诊断同时指出模板接入点、实例正文和失败的绑定关系。

### 分散规则

分散实例先按模板身份与所属 package 归组，再向目标交付带绑定关系的规则。模板实例形成过程与目标匹配结构的覆盖、效果及资源检查分别执行，详见[函数级模式的编译器引导](pattern-functions.md#编译器实现引导)。

普通模板的检查可以把正文接入视为一个带前后接口的节点：进入时提供声明的 runtime 绑定，退出时接收目标结果及状态变换。所有来自模板和正文的局部责任最终在同一份控制流中检查，不能各自生成一份互不知情的清理。
