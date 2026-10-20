# 函数契约与效果

## Intro

函数签名说明要交付什么值，契约说明这些值在什么条件下可以使用。调用方需要知道入口要求、函数可能改变什么，以及返回后仍能依靠哪些事实。

一个结果类型正确的函数，仍可能用失效指针读取数据，或重复消耗一个资源。类型与程序契约共同约束调用；它们各自提供的证据不能互相冒充。

## 语法

~~~text
result -> (id :)? expr
verification_clause -> requires expr
    | ensures expr
    | decreases expr
outcome_clause -> outcomes expr
assumes_statement -> assumes expr
error_qualified_type -> !expr expr
effect_qualified_type -> #expr expr
program_predicate -> predicate id(parameter*) (-> expr)? clauses? block
~~~

函数、子句、pure、spec 和 verified 的完整形式同[全局语法](../grammar/syntax)。错误／效应限定属于结果类型；处理能力由 [trait 协议配合 elaboration](../traits/protocols.md#处理能力) 提供。

## 正式定义

### 调用方取得的四类信息

| 信息 | 作用 | 不能替代的检查 |
|---|---|---|
| requires | 调用前必须成立的条件 | 不能在危险操作执行后再补检查 |
| ensures | 相应完成路径交付的保证 | 不表示函数必定终止或总是成功 |
| outcomes | 允许的行为范围与规格条件 | 不单独保证读写顺序、所有权或结果内容 |
| 结果及环境依赖 | 结果需要哪些对象、状态与责任 | 不因返回类型可表示而自动延长局部存储生命 |

没有显式 outcomes 时，按主体与被调用操作的契约推导摘要。显式写出行为上界后，实现须在上界以内；不能为通过检查而自动把遗漏的写入补到声明中。

入口和出口需要按实际控制路径组合。显式 return、错误传播、放弃后续实参求值都可能改变退出位置。正常结果保证不能被套用到未正常交付该结果的路径。

### 值参数与指针参数

按值参数交付 Copy 副本或非 Copy 责任。指针参数交付访问某个对象的能力及其实际依赖，不隐含 pointee 的独占所有权，也不隐含整个调用期间对象不会变化。

read、write 等 memory effect 描述操作涉及的位置。结果是否继续依赖输入、哪些区域保持、哪些责任移出，需要相应的状态与资源关系。通过同一个指针先写后读，和先读后写，可以具有相同的效果上界却有不同结果。

### 两类规格系统

将类型命题与程序验证分开处理，是因为二者的判定方法和可信边界不同。类型要求，例如类型具有某个 trait、类型参数满足结构条件，属于 PTC。运行时下标、内存状态、访问合法性及回调影响属于程序验证层。两者可共用 requires 等表面形式，不能因此混用求解器。

程序 predicate 可以用于 safe 的规格，不需要 comptime 前缀。用于计算真值的可执行规格和描述 memory effect 的规格构造也各有语义；写出 read 不等于立即读取被描述的地址。

值依赖可以位于条件分支或局部块内部，不能只检查 `requires` 的最外层运算符。例如：

~~~flurry
fn checked(flag: bool) -> bool
    where requires do { let selected = flag; selected } {
    true
}
~~~

这个前置条件属于程序验证；输入 false 时，入口检查拒绝执行主体。调用表达式中的值实参也参与分类，但被归为程序谓词不代表任意函数调用已经具有合法判定器。另一方面，normal-form 类型的 comptime 值索引不使包围它的 trait 约束变成程序谓词：类型形成仍单独检查，类型身份保持原头。

两个关联标量的 `==` 或 `!=` 也是程序条件，即使表达式没有引用普通函数参数。编译器依据成员声明的类型区分它与关联 Type 的相等；前者进入程序验证，后者仍是类型命题，不以证明失败后的回退来决定含义。

程序谓词也可以读取已检查的关联常量。其初始化仍属于 comptime invocation，读取常量不等于允许在谓词里调用任意函数：

~~~flurry
comptime fn invert(value: bool) -> bool { not value }
trait Flags where enabled: bool { assoc flag: bool = invert(enabled); }
struct Holder where enabled: bool {}
impl Flags<enabled> for Holder<enabled> where enabled: bool {}

comptime fn checked(value: bool) -> bool
    where requires value or Holder<false>.as(Flags<false>).flag {
    true
}
fn decide(comptime value: bool) -> bool { value }
fn example() -> bool { decide(checked(true)) }
~~~

构建必需 invocation 时，前置和后置条件引用的关联初始化及其辅助函数也进入依赖闭包。程序分析对尚未完成的关联值只保留类型范围，不猜测初始化结果。短路分支合流后，若某个局部观察的身份已经失效，保留布尔结果范围而丢弃旧关系，不能继续利用失效证明。

程序条件不必引用普通形参；使用上例声明，也可以写：

~~~flurry
comptime fn guarded() -> bool
    where requires Holder<false>.as(Flags<false>).flag {
    true
}
~~~

该条件及其 `not`、与布尔字面量比较、局部块或条件分支形式均保留为程序前置条件。条件的真假由程序验证与调用检查处理，不加入 PTC 的类型假设。分类发生在证明之前，失败的类型相等或 trait 证明不会因此改为另一种程序解释。

如果调用点还不能静态证明关联条件，可先用受检查的 `assumes` 建立事实：

~~~flurry
fn staged() -> bool {
    decide(do {
        assumes Holder<false>.as(Flags<false>).flag;
        guarded()
    })
}
~~~

程序验证以接口实例、成员和 Self 共同标识关联标量，向被调用者传递同一常量的事实。换成另一组接口参数、另一个成员或另一种 Self 不能复用该事实。这里的 `assumes` 仍须实际检查；关联条件为假时 invocation 失败，而不是建立无条件的类型假设。

同一规则也用于关联整数上界。下面是独立例子，生产者的后置条件可直接用于后续调用：

~~~flurry
typealias Byte = .u8'builtin;
comptime fn keep(value: Byte) -> Byte { value }
trait Limits where maximum: Byte { assoc bound: Byte = keep(maximum); }
struct Holder where maximum: Byte {}
impl Limits<maximum> for Holder<maximum> where maximum: Byte {}

comptime fn produce() -> result: Byte
    where ensures result < Holder<9>.as(Limits<9>).bound {
    let value: Byte = 8;
    assumes value < Holder<9>.as(Limits<9>).bound;
    value
}
comptime fn checked(value: Byte) -> Byte
    where requires value < Holder<9>.as(Limits<9>).bound { value }
fn decide(comptime value: Byte) -> Byte { value }
fn example() -> Byte { decide(checked(produce())) }
~~~

关联常量的观察身份不会因普通调用而失效，但可修改的局部值仍有版本：修改实参后不能继续使用修改前的范围证明。后置条件传递的是已经证明的关系，不会在程序分析中重新执行关联初始化，也不会推断另一关联实例具有相同上界。

泛型成员的程序契约也可以引用由实现提供的关联值，不必重复声明编译器已能从实现证明的 trait 约束。例如：

```flurry
typealias Byte = .u8'builtin;
trait Bounds where limit: Byte { assoc bound: Byte = limit; }
struct Holder where limit: Byte {
    pub comptime fn checked(value: Byte) -> result: Byte
        where requires value < Self.as(Bounds<limit>).bound,
              ensures result < Self.as(Bounds<limit>).bound {
        value
    }
}
impl Bounds<limit> for Holder<limit> where limit: Byte {}
```

成员契约中的关联读取先保留其接口实例、成员和 Self 身份；实现检查完成后，再在该成员的声明上下文中证明接口成立。实现放在成员之前或之后不影响结果。trait 默认方法与显式实现方法也使用这一规则。调用处的关联标量读取保留相同观察身份，即使初始化器只是字面量，受检查的 `assumes` 仍可向方法契约传递事实。这个延后检查不会增加隐式的公开约束，也不会在检查契约时假定实现成立：即使成员从未被调用，缺少所需实现仍会拒绝整个声明。经由子接口读取继承成员时，必须证明请求的子接口，只有父接口的实现不够。


容器的不可变值参数也可以出现在普通方法契约中。定义检查为每个已声明的标量捕获保留独立的符号身份；这不把未知值猜成常量。调用时按参数的 Type／值种类替换契约，并将仍有效的捕获事实和实参事实一起传递。

```flurry
struct Limit where maximum: u8 {
    pub fn keep(value: u8) -> result: u8
        where requires value <= maximum,
              ensures result <= maximum {
        value
    }

    pub fn forward(value: u8) -> result: u8
        where requires value <= maximum,
              ensures result <= maximum {
        Self.keep(value)
    }
}
```

`Limit<10>.forward(9)` 满足契约，传入 11 则不能通过调用检查。不同的上界参数保持不同身份；只有 `value <= left`，不能据此证明 `value <= right`。短路条件两侧使用同一捕获的身份，条件分支与调用返回继续保留已证明的关系。复用声明自身的值绑定时，其依赖域必须保持不变；替换前面的 Type 输入后，不能把旧域的符号值冒充新域中的值。


### 受检前提与证明

safe 在 normalized IR 上完成相应检查。超出自动证明能力且具有合法判定器的条件，可以用 assumes 在依赖动作之前建立受检事实。运行检查只能证明它实际检查的条件，不能复活已移出的责任或已失效的指针。

verified 函数的实现证明在 instantiated IR 完成。调用方按声明契约检查自己的调用；交付程序前，相应证明依赖必须成立。调用 verified 不把调用者整体改成 verified。

### 函数修饰的不同承诺

| 形式 | 本篇关注的承诺 |
|---|---|
| 普通 fn | 按实际契约检查行为；可包含读写和输出 |
| runtime pure fn | 承诺 P4 值确定计算，仍不自动保证终止 |
| 可执行 spec fn | P3 稳定观测、确定性及合法入口下全定义 |
| verified fn | 对所声明契约承担对应实现证明 |
| unsafe fn | 由明确的 unsafe 边界承担其契约要求 |
| comptime pure fn | 编译时纯性采用其独立的可信承诺政策 |

完整纯度层级见[运行时纯度](../resource-management/runtime-purity.md)。ABI 和 inline 属性不代替这些行为承诺。

## 常见示例

### 除法前提与正常结果

~~~flurry
fn quotient(numerator: u32, denominator: u32) -> result: u32
where requires denominator != 0,
    ensures result <= numerator
{
    numerator / denominator
}

fn checked_quotient(numerator: u32, denominator: u32) -> u32 {
    assumes denominator != 0;
    quotient(numerator, denominator)
}

test {
    println("{}", checked_quotient(20, 3));
}
~~~

输出 `6`。若调用方尚不能静态证明分母非零，受检事实须在除法之前建立。检查失败路径不进入 quotient。result 是规格中的返回结果名；函数体直接交付除法结果。

### 效应完成后的结果保证

~~~flurry
effect poll() -> bool;

fn keep(flag: bool) -> result: #poll bool
    where ensures result == flag'old {
    poll()#;
    flag
}

fn checked_keep() -> result: bool where ensures result {
    keep(true)# { poll() => false, }
}
~~~

checked_keep 返回 true。result 绑定 computation 完成后交付的 bool；poll 的响应是另一个值，不替代 keep 的返回值。函数的私有按值参数不会因外部 handler 执行而自行改变；可被 handler 写入的词法变量、已暴露地址及相关内存观测则不能继续沿用旧事实。

相同规则适用于 quote fn 的确定性展开。若 handler 执行词法 return，检查的是它所属函数的返回契约；尚未正常完成的 computation 不提供其 ensures。后置条件仍需逐条返回路径证明，无法证明时须按已有受检前提规则处理。

开放函数的结果声明已经给出完整效应行时，定义检查也在该行内检查主体。comptime 参数只决定之后的实例化选择，不能让未选分支绕过效应资格检查：

~~~flurry
effect read() -> bool;

fn selected(comptime enabled: bool) -> #read bool {
    inline if enabled { read()# } else { false }
}

fn forward(comptime enabled: bool) -> #read bool {
    let value = selected(enabled);
    if value { true } else { false }
}

fn run() -> bool {
    forward(true)# { read() => true, }
}
~~~

这里 value 是行内完成调用后取得的 bool；效应行保留在 forward 的接口上。声明的行只覆盖函数主体，不自动授权参数默认值或 comptime 选择条件发出请求。返回闭包的开放工厂遵守同一规则：工厂效应、handler 安装及后续闭包调用各自保持原来的执行边界，拥有型捕获仍沿实际完成或提前退出路径清理一次。

泛型调用在已声明的行内执行时，结果上下文约束的是调用交付的值，而不是把整个 computation 与普通值类型判为相等：

~~~flurry
effect obtain() -> T where T;

fn fetch(comptime enabled: bool) -> #obtain<T> T where T {
    inline if enabled { obtain<T>()# } else { obtain<T>()# }
}

fn fetch_flag(comptime enabled: bool) -> #obtain<bool> bool {
    fetch(enabled)
}
~~~

fetch_flag 的 bool 结果域足以为该次 fetch 推导 T；若还有普通实参或计算型 tuple 包，它们也共同提供依据。之后仍检查具体操作与类型实参是否属于允许的行，不能用 `obtain<char>` 或另一个同结果类型的操作替代 `obtain<bool>`。显式要求完整 computation 类型时保留完整类型，不套用行内结果交付的推断规则。这不表示已支持一等 computation 存储；当前将 computation 保存为局部值再恢复的解释／IIR 路径尚未完整接通。



### 通过普通结果表达可恢复失败

不希望零分母成为入口检查失败时，可以把它放进错误结果：

~~~flurry
enum DivideErr {
    ZeroDivisor,
}

fn divide(numerator: u32, denominator: u32) -> !DivideErr u32 {
    if denominator == 0 {
        error DivideErr.ZeroDivisor
    } else {
        numerator / denominator
    }
}

fn half_quotient(numerator: u32, denominator: u32) -> !DivideErr u32 {
    let value = divide(numerator, denominator)!;
    value / 2
}

test {
    let first = half_quotient(20, 2)! {
        DivideErr.ZeroDivisor => 0,
    }
    let second = half_quotient(20, 0)! {
        DivideErr.ZeroDivisor => 0,
    }
    println("{} {}", first, second);
}
~~~

输出 `5 0`。第二次在 divide 中产生错误，half_quotient 不继续执行除以 2；调用者在自己的处理位置取得替代值。错误行与成功类型属于接口，具体处理行为由处理位置提供。

### 允许修改的范围与结果内容分别表达

~~~flurry
fn replace(target: *u32, next: u32) -> previous: u32
where outcomes read<target> and write<target>,
    ensures target.* == next
{
    let saved = target.*;
    target.* = next;
    saved
}

test {
    let current: u32 = 4;
    let previous = replace(current.ref, 9);
    println("{} {}", previous, current);
}
~~~

输出 `4 9`。read/write 上界允许访问 target；ensures 说明正常完成时的内容。previous 来自写入前取得的副本。只有 read/write 上界时，调用者不能据此推导最终一定写入 next。

~~~flurry
-- 拒绝：只读上界不允许主体中的写入。
fn invalid_read_only(target: *u32)
where outcomes read<target>
{
    target.* = 0;
}
~~~

### 接口保留效应请求

~~~flurry
effect read_number() -> u32;

fn bounded_input() -> #[read_number] u32 {
    let value = read_number()#;
    if value > 10 {
        10
    } else {
        value
    }
}

test {
    let value = bounded_input()# {
        read_number() => 12,
    }
    println("{}", value);
}
~~~

输出 `10`。处理分支提供操作要求的 u32，主体继续执行截断，再交付自己的结果。操作结果、主体结果和整个 handler 结果是不同位置的要求；更复杂的恢复规则见[handler 的类型与恢复契约](../type/handlers.md)。

### Pure 允许内部计算，spec 用于可执行规格

~~~flurry
pure fn smaller(left: u32, right: u32) -> u32 {
    let result = left;
    if right < result {
        result = right;
    }
    result
}

spec fn within(index: usize, length: usize) -> bool {
    index < length
}

test {
    println("{}", smaller(8, 3));
    assumes within(2usize, 4usize);
}
~~~

输出 `3`。pure 的主体可以修改自己的局部计算状态；承诺约束的是可观察行为与结果依赖。within 是有限的比较判定，适合在访问前建立规格事实。pure 并不自动给予任意递归函数这样的有限完成保证。

### 调用 verified 实现

~~~flurry
verified fn increment(value: u32) -> result: u32
where requires value < 4294967295u32,
    ensures result == value + 1
{
    value + 1
}

fn next(value: u32) -> u32 {
    assumes value < 4294967295u32;
    increment(value)
}

test {
    println("{}", next(41));
}
~~~

输出 `42`。next 仍以 safe 的规则使用契约；increment 的证明须保证所有合法输入上的正常结果。若实现改为返回原 value，仅让调用方通过前提检查不足以交付程序。

### Trait 方法中的纯度承诺

接口中的 runtime `pure fn` 承诺 P4。实现即使省略 `pure` 拼写，也必须兑现该接口承诺；默认方法和覆盖方法同样如此。泛型函数可以依据已建立的 trait 接口组合纯度，而不需要在定义时选定具体实现。

~~~flurry
trait Flip {
    pure fn flip(self) -> Self;
}

impl Flip for bool {
    fn flip(self) -> Self { not self }
}

pure fn flipped(value: T) -> T
where T :- Flip
{
    value.flip()
}

test { println("{}", flipped(false)); }
~~~

输出 `true`。如果接口没有声明纯度，仅有某一个实现是 pure，不足以证明任意满足该 trait 的泛型调用也纯。接口的 P4 承诺不附带终止性；把方法调用放进 runtime 默认值时，仍需证明供应能有限完成。

## 与 Rust / Zig / C / C++ 的关键差异

C 程序可用显式条件和断言保护除法；Rust、Zig、C++ 也各有检查与错误表示手段。这里的区别在于，Flurry 的函数契约还参与访问、效果与资源关系的组合，并区分类型命题和程序验证。具体对照见[契约与验证](../resource-management/contracts-and-verification.md#与-rust--zig--c--c-的关键差异)和[错误类型](../type/qualified-types/error-type.md)。

## 运行时实现设计引导

1. 按调用准备顺序取得实参，在每个危险动作之前兑现其前提。
2. 将已经证明可消除的检查去掉；保留仍需 runtime 执行的合法判定。
3. 在成功边发布新事实，失败边进入实际失败路径并清理责任。
4. 执行主体；沿正常、错误、恢复和提前退出路径应用各自状态变换。
5. 交付结果及依赖，只清理仍留在本作用域的责任。

规格本身不要求每次调用创建一个运行时契约对象。保留多少检查取决于静态证据与动态验证设施。优化读取时还需考虑内容版本；同一地址不证明两次读取结果相同。

## 编译器实现引导

在签名检查中分别登记类型要求与程序要求。为具名结果建立结果绑定，所有正常结果出口都与它对应；不要把它登记为入口时已经初始化的变量。

生成调用检查时依次完成：

1. 将形参、环境和调用者实际对象建立对应。
2. 检查当前状态下的入口要求，包括准备期间实参对对象造成的变化。
3. 组合 callee 的行为摘要，并使受影响的旧事实失效。
4. 按实际出口引入结果关系、剩余行及责任状态。
5. 保留 verified 等跨定义证明依赖，禁止依赖失败的证明交付程序。

outcomes 检查包含隐式适配和 Drop。PTC 只接收类型命题；程序验证层接收内存状态、runtime 数值及路径事实。safe 算术推导遵守总次数不超过三次的多元多项式片段及资源预算；超出范围报告缺少的关系，不伪造证明。

验收分别覆盖：入口成立而后置条件不成立、上界遗漏隐式清理的写入、错误路径误用正常结果事实、调用后旧版本事实未失效，以及 verified 证明失败却仍被链接交付。

当调用的实现已经具体确定时，也可以从实现体获得比接口更强的行为证据。例如下面的接口没有要求所有实现都纯，但 `bool` 的实现及其默认方法链可以证明值稳定且有限完成，因此能够供应 runtime 默认值：

```flurry
trait Flip {
    fn flip(self) -> Self;
    fn twice(self) -> Self { self.flip().flip() }
}
impl Flip for bool {
    fn flip(self) -> Self { not self }
}
fn choose(value: bool, .same: bool = value.as(Flip).twice()) -> bool {
    same
}
fn run() -> bool { choose(true) }
```

这条路径复用正常调用的实现选择与类型实参替换，仍检查接口的调用前置条件。它不能把具体实现的证据推广给尚未确定实现的泛型调用，也不能因为接口承诺 `pure` 就推定递归或循环会终止。

`pure fn` 中的复制同样受资源协议约束。显式绑定 Copy／Drop 语言项后，编译器可以复用资源验证器的表示证据，例如：

```flurry
trait Copy {}
trait Drop { fn drop(*self); }
struct Flag { value: bool }
impl Copy for Flag {}

pure fn duplicate(value: Flag) -> Tuple<Flag, Flag> {
    (value, value)
}
```

仅有 `impl Copy` 还不够：嵌套字段和所有枚举分支都必须满足复制条件，复制也不能复制自定义或未确定的 Drop 责任。协议身份由驱动绑定，代码中恰好存在同名 trait 不会自动启用该规则。已选定的用户 Drop 可以另行通过实现体获得清理纯度证据，但这不会解除 Copy 对自定义 Drop 责任的限制，也不把 Clone 当作表示复制。

清理不等同于复制。绑定 Drop 后，若资源验证器能确定聚合值及其全部字段、枚举分支均没有自定义析构，则退出作用域只结束这些位置的构造责任，可以用于纯函数；该值仍可保持仿射属性。部分移动后的外壳也必须检查，不能因为载荷已经移出就忽略外壳的 Drop。

```flurry
struct Owned { value: bool }
struct Pair { first: Owned, second: Owned }

pure fn first(value: Pair) -> bool {
    value.first.value
}
```

这个例子不要求 `Owned` 或 `Pair` 实现 Copy。它需要编译器依据已绑定的 Drop 协议确定剩余清理不调用用户代码；若任一剩余责任可能调用自定义 Drop，编译器继续检查已选定的析构实现及其调用链，而不是仅根据有没有 `pure` 修饰符决定。


```flurry
struct Ticket {}
impl Drop for Ticket {
    fn drop(*self) { finish() }
}
fn finish() {
    var complete = false;
    complete = true;
}
pure fn retire(value: Ticket) {}
```

这里 Drop 的辅助函数只修改私有局部值，因此可以证明 P4。泛型析构函数按选中的类型实参分析；嵌套字段和枚举分支的清理也必须满足相同要求。P4 不保证终止，清理的内存访问、调用前置条件和运行时环境取得仍由资源验证层检查。当前行为分析尚不能把通过析构接收者读写字段识别为私有资源操作，因此这类 Drop 仍可能被保守拒绝。

高阶调用的行为证明可以依赖实际传入的代码身份。例如 `apply` 接受普通函数值，不承诺对任意回调都纯；已知的 `invert` 实现则可以使这个具体调用满足 P4：

```flurry
fn invert(value: bool) -> bool { not value }
fn apply(callback: fn(bool) -> bool, value: bool) -> bool {
    callback(value)
}
pure fn run() -> bool {
    let action = invert;
    apply(action, false)
}
```

具名函数身份可经局部别名和多层参数转发，行为检查与资源检查使用同一组代码前提。证明缓存区分实际回调，而不是只按函数类型复用结果。若供应链还可以证明有限完成，这种调用也可用于 runtime 默认值。未知回调仍需要行为契约；回调重新赋值时必须合并可能的代码来源，不能沿用唯一的旧身份。循环中的代码赋值按入口值、赋值来源和局部别名计算有预算的候选闭包；未知来源保持未知。无捕获匿名闭包同样保留完整代码身份及参数前提；按值捕获内建可复制值及其结构聚合、并只读使用的环境可参与 P4 证明；仿射资源捕获也可在传递、消费和清理证据齐全时使用；拥有的按值环境可在资源验证通过后更新自身标量槽位及字段；函数返回的代码身份可依据后述工厂摘要恢复。


```flurry
fn apply(callback: fn(bool) -> bool, value: bool) -> bool {
    callback(value)
}
pure fn run() -> bool {
    let action = |value: bool| not value;
    apply(action, false)
}
fn choose(.value: bool = apply(|value| not value, false)) -> bool {
    value
}
```

匿名函数体在调用时检查；把函数值构造出来并不等于执行它。匿名回调自身也可以接收已经确定的代码参数。其内部临时资源的清理继续经过资源分析，不能借闭包边界隐藏 Drop 副作用。默认值中的闭包调用另需有限完成证据，`pure` 的 P4 承诺仍不提供终止性。


```flurry
fn twice(callback: %{flag: bool} fn() -> bool) -> bool {
    callback() and callback()
}
pure fn snapshot() -> bool {
    var flag = true;
    let action = || flag;
    flag = false;
    twice(action) and not flag
}
```

闭包按值保留 `flag` 的快照，之后修改外层绑定不改变这个环境。只读捕获可被重复调用；环境本身的传递和清理仍需要资源证据，因此当前实现应通过绑定 Copy／Drop 协议的检查入口验证。捕获一个指针只取得其表示，不能据此允许读取目标状态。按值构造或按值交付的拥有环境可以更新自身标量捕获及标量字段，但这份私有性不延伸到捕获指针指向的存储。交付、别名和清理仍由资源验证器检查。带环境的默认值供应仍需后续的有限完成与资源证明衔接。


代码工厂的行为和返回代码的行为分别检查：

```flurry
fn factory() -> fn(bool) -> bool {
    |value| not value
}
pure fn run() -> bool {
    let action = factory();
    action(false) and factory()(false)
}
fn choose(.value: bool = factory()(false)) -> bool { value }
```

现有 callable 摘要能确定返回代码候选集合时，纯度分析恢复各候选的定义和类型实参，继续检查调用；代码可以是具名函数、无捕获闭包，或已支持的只读值捕获闭包，也可以从工厂的代码参数原样转发。工厂本身必须先通过行为检查，默认值还要求工厂及返回代码均能有限完成。摘要中的 Unknown 和无正常返回不能被当作已知代码。多候选集合保留所有候选并逐一证明，不能任选一个有利的代码身份代替整个集合。


条件分支和 match 的代码结果可以合并后交给高阶函数：

```flurry
fn keep(value: bool) -> bool { value }
fn invert(value: bool) -> bool { not value }
fn apply(callback: fn(bool) -> bool, value: bool) -> bool { callback(value) }

pure fn select(flag: bool) -> bool {
    let action = if flag { keep } else { invert };
    apply(action, flag)
}
```

两个候选都满足 P4，因此整个调用可通过证明；若用于默认值，还必须分别证明有限完成。具名函数、无捕获闭包和已支持的只读值捕获闭包可以组成候选集合。资源分析保留各自的环境证据并检查每个候选的清理。任何 Unknown 分支都使集合不再具有完整代码证据；相同签名不能替代缺失的实现身份。


循环可以重复调用具有不变代码身份的回调，P4 仍不承诺这个循环终止：

```flurry
fn repeat(callback: fn(bool) -> bool, value: bool, count: .u8'builtin) -> bool {
    var remaining = count;
    var result = value;
    while remaining > 0 {
        result = callback(result);
        remaining = remaining - 1;
    }
    result
}
pure fn run() -> bool { repeat(|value| not value, true, 2) }
```

检查覆盖嵌套循环、分支和参数准备中的赋值；不能因为某个回调在第一轮尚未被修改，就把它的初始身份用于所有后续轮次。循环不变证据同样保留具名与匿名候选集合，但不提供默认值供应所需的有限完成证明。


调用后的程序事实按存储可达性失效，而不是简单地全部清空。即使回调的行为未知，也不能修改从未暴露地址的按值标量局部值，因此上例中 `remaining > 0` 的事实可跨越回调调用，用于证明之后的减法安全。这不要求回调承诺 `pure`。已取址的局部值、指针绑定和外部目标读取仍按未知调用失效，不能借此消除本应保留的运行时检查。


循环中重新绑定回调也可证明 P4：

```flurry
pure fn alternate() -> bool {
    var action = |value: bool| value;
    var remaining: .u8'builtin = 2;
    var value = false;
    while remaining > 0 {
        value = action(value);
        action = |value: bool| not value;
        remaining = remaining - 1;
    }
    value
}
```

检查在第一轮之前合并入口代码和所有赋值来源，并沿局部别名传播到固定点；随后检验全部候选。候选集合覆盖循环内各位置以及提前退出路径，不只记录语法上最后一次赋值。当前赋值来源支持直接代码、局部别名、条件／match、块结果，以及返回摘要完整的具名工厂调用。工厂本身也可来自已知的函数参数或候选集合；无法确定的来源保持 Unknown。这个分析不执行循环，也不提供终止性证明。


工厂调用也可以出现在循环更新中：

```flurry
fn next(previous: fn(bool) -> bool, keep: bool) -> fn(bool) -> bool {
    if keep { previous } else { |value| not value }
}
pure fn rotate() -> bool {
    var action = |value: bool| value;
    var remaining: .u8'builtin = 2;
    var value = false;
    while remaining > 0 {
        value = action(value);
        action = next(action, false);
        remaining = remaining - 1;
    }
    value
}
```

候选闭包计算只提取工厂可能返回的代码，之后仍验证工厂、实参求值及返回代码的行为。它不提前执行工厂，也不把未知代码参数假定为某个已知实现；尚未建立的递归代码前提会保守地阻止证明。


拥有资源的捕获不必实现 Copy。下面的环境将 `Token` 转交给一次调用的结果：

```flurry
struct Token { flag: bool }
fn make(token: Token) -> %{token: Token} fn() -> Token { || token }
fn invoke(action: %{token: Token} fn() -> Token) -> Token { action() }

pure fn run() -> bool {
    let action = make(Token { flag: true });
    invoke(action).flag
}
```

该例需要已绑定的资源协议证明清理行为。纯度检查复用资源验证器对环境交付、捕获消费和剩余清理的证据，不因为捕获类型不是 Copy 就拒绝，也不因为闭包已通过 P4 就允许第二次消费同一资源。部分移走聚合捕获时，剩余字段和外壳的清理仍需验证；有副作用的析构不能借闭包封装绕过检查。


```flurry
fn twice(callback: %{flag: bool} fn() -> bool) -> bool {
    let first = callback();
    let second = callback();
    first and not second
}
pure fn toggle() -> bool {
    var flag = false;
    let action = || do { flag = not flag; flag };
    twice(action) and not flag
}
```

在资源协议检查通过时，`action` 修改的是自己拥有的捕获槽位，外层 `flag` 仍为 false；`twice` 按值取得环境，两次调用继续使用该环境的更新后状态。分别构造的环境拥有不同槽位。拥有结构捕获的标量字段也可更新；替换一般仿射字段、借用环境和通过指针写入目标还需要各自的资源及内存证据，不能仅凭槽位私有就推导。

拥有字段的仿射值也可以替换，前提是旧值的退休和新值的交付都通过资源检查：

```flurry
struct Token { flag: bool }
impl Drop for Token { fn drop(*self) {} }
struct State { item: Token }

pure fn update(state: State) -> bool {
    state.item = Token { flag: true };
    state.item.flag
}
```

在已绑定的资源协议下，空析构的行为可被证明；换成外部写入的析构则不能保持 P4。赋值先完成右侧求值，再退休尚存的旧值。右侧提前返回时没有替换清理，原责任由退出路径承担；`value = value` 先移出再装回，也不能额外析构被转移的值。丢弃拥有值的语句同样检查其清理；不返回值的 `never` 语句不需要凭空获得 Copy。

BitVec 的动态索引、切片和私有写回同样参与行为分析：

```flurry
typealias Word = .u16'builtin;
typealias Byte = .u8'builtin;

pure fn insert_nibble(word: Word, offset: Byte) -> Word
    where requires offset <= 12
{
    word'bitvec[offset..(offset + 4)] = 0b1010;
    word
}

fn take(.word: Word = insert_nibble(0x1234, 4)) -> Word { word }
```

这里修改按值传入的私有存储；拥有字段和闭包拥有的捕获槽位也可使用该规则，但其交付与清理仍须通过资源检查。每层动态选择的端点、右侧值和所调用函数都要独立满足行为要求，不能借位操作隐藏外部访问。程序验证器仍检查位宽、边界和调用前置条件；`pure` 不提供这些事实。

读取先取得值快照，再按顺序计算端点。写回先固定位置、求右侧值，再更新存储的对应位，保留右侧对其余位的合法修改；端点或右侧提前返回就不写回。有限的位运算可以供应默认参数；包含循环的 P4 函数仍不自动获得默认参数所需的有限完成证明。通过指针访问目标的位视图尚不能凭私有指针槽位取得 P4 证据。
