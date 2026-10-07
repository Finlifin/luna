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
