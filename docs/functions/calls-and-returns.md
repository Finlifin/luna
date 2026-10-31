# 定义、调用与返回

## Intro

调用有自己的执行次序。把一个实参写到另一个实参前面，就决定了它们产生效果的先后；形参的排列决定值交到哪里。

返回同时决定控制流和责任交付。函数可以交出一个值，不能让结果继续依赖已经销毁的局部存储。

## 语法

~~~text
function -> pure? comptime? inline? (unsafe | spec | verified | atomic)? fn id(parameter*) (-> result)? clauses? (block | (= expr))
application -> expr(argument*)
result -> (id :)? expr
return_statement -> return expr? (while expr)?
self_param -> self
self_ref_param -> *self
attr_def -> ^expr definition
~~~

## 正式定义

### 定义与结果

`fn` 声明参数、结果和主体。主体最后一个表达式可交付结果；`return` 提前完成当前函数。省略返回类型时，按无结果函数的规则使用 void。具名结果可以在结果契约中引用，不据此自动建立一个未初始化的局部变量。

`return value while condition` 按条件决定是否执行返回。条件不成立时继续主体；成立时交付 value，并执行相应退出清理。

块主体与 `= expr` 主体交付同一个声明接口。块允许先建立局部值和执行语句，再以最后的表达式交付结果；表达式主体适合直接描述计算。无主体签名描述接口，其定义来源仍须满足所在构造的要求，不能把所有签名声明都解释成外部链接声明。

| 形式 | 结果来自哪里 | 需要保留的边界 |
|---|---|---|
| 块末尾表达式 | 表达式正常交付的值 | 离开主体时清理剩余局部责任 |
| `= expr` | 该表达式的结果 | 参数交付、结果类型与契约和块主体相同 |
| `return value` | 提前交付的 value | 跳过后续语句，但执行相应退出清理 |
| 条件 return | 条件成立的退出路径 | 未退出路径继续检查和执行 |
| 具名结果 | 上述路径交付的同一结果 | 名字供结果规格引用，不是可任意赋值的局部槽 |

### 函数 ABI

函数 ABI 通过属性指定，例如 `^attrs.abi(.c)` 指定 C ABI。属性表达式的名称解析、编译时求值和附着规则沿用[属性系统](../comptime/attribute.md)。

ABI 决定边界上采用的调用约定；参数阶段、访问前提、结果依赖和资源责任仍需分别满足。一个函数使用 C ABI，不据此变成 comptime 函数，也不因参数是裸指针就免除有效性要求。

### 显式实参

一次普通调用遵循以下次序：

1. 求被调用表达式；方法调用先取得接收者。
2. 按实参在源码中的次序求显式 runtime 表达式，各一次。
3. 按形参声明依赖次序求缺失的 implicit 与 optional runtime 输入。
4. 在实际需要的位置兑现访问和调用前提，然后进入主体。

阶段检查可以在这些计算发生前完成，但不能因此提前执行 runtime 表达式。具名实参按名字绑定，不改其源码求值次序。变参收集也保留输入次序。

未被函数体使用的普通参数仍须求值。内联、模板实例化或代码展开不能凭“没有读到参数”丢弃实参的输出、写入或清理。合法的优化必须另有行为依据。

函数调用的次序不替代结构体构造的字段初始化规则；二者分别遵守自己的语义。

### 方法与接收者

`self` 按值接收，Copy 时复制，非 Copy 时移动。`*self` 接收普通指针，使用时仍需兑现访问与内存效果契约。方法查找和接收者适配见[接收者适配](../type/receiver-adaptation.md)。

取得接收者发生在显式实参之前。适配中取得的指针不冻结对象；后续实参若改变了相关对象状态，调用前提必须针对变化后的状态成立。

接收者和其余参数使用相同的 Copy、移动与依赖规则。按值 self 可以交出对象中的责任；指针 self 可以就地修改。方法所属类型、所选 implementation 与自动适配共同确定入口，不能只凭方法名字选择一个同名函数。

### 失败与清理

错误与效应的处理能力由 trait 协议描述，通过 elaboration 接入具体处理行为，见[处理能力](../traits/protocols.md#处理能力)。处理后的结果、剩余错误与效应仍遵守相应的类型与控制流契约。

某个实参没有正常交付时，不继续执行后面的实参、缺省计算或函数主体。已经取得的非 Copy 值仍有清理责任，沿实际退出路径处理；尚未求值的位置没有相应责任。

进入函数后，局部值、参数及结果的移动和销毁遵循[自动清理](../resource-management/drop-elaboration.md)。错误值、效应转移与提前返回不能跳过这些规则。结果中的指针或视图必须保留有效来源。

## 常见示例

### 用表达式主体解码两个字节

~~~flurry
fn decode_word(high: u8, low: u8) -> u32 =
    high.as(u32) * 256u32 + low.as(u32);

test {
    println("{}", decode_word(1, 2));
}
~~~

输出 `258`。两个输入先扩展为 u32，再做乘加；输入范围保证结果至多为 65535。等号后仍是普通表达式，分号结束定义。

### 用属性指定 C ABI

~~~flurry
^attrs.abi(.c)
fn identity(value: u32) -> u32 {
    value
}
~~~

ABI 属性附着在 identity 的函数定义上。

当前实现只完成该属性形式的解析；通用属性的名称解析、comptime Object 求值，以及函数 ABI 信息进入类型与调用检查尚未接通。因此带此属性的函数会在生成已检查 NIR 前被拒绝，包括直接调用、取函数值、inline 函数和 trait 成员；不会退回默认 ABI 来接受程序。此限制不改变上述语言契约。

### 名字绑定与源码次序

~~~flurry
fn mark(value: u32) -> u32 {
    println("arg {}", value);
    value
}

fn show(.left: u32 = 0, .right: u32 = 0) {
    println("body {} {}", left, right);
}

test {
    show(.right = mark(2), .left = mark(1));
}
~~~

依次输出 `arg 2`、`arg 1`、`body 1 2`。值按名字交付，计算按书写次序发生。

### 被调用表达式也先求值

~~~flurry
fn combine(first: u32, second: u32) -> u32 {
    first * 10 + second
}

fn select() -> fn(u32, u32) -> u32 {
    println("callee");
    combine
}

fn produce(value: u32) -> u32 {
    println("value {}", value);
    value
}

test {
    let result = select()(produce(3), produce(4));
    println("result {}", result);
}
~~~

输出顺序是 `callee`、`value 3`、`value 4`、`result 34`。

花括号中的普通输入与展开输入也遵循这一顺序，callee 不限于函数名：

~~~flurry
test {
    let result = select() { ...do { (produce(3), produce(4)) } };
    println("result {}", result);
}
~~~

这里仍先执行 select，再执行两个 produce，最后调用取得的函数。代码块和条件表达式也可以交付 callee；捕获闭包保留真实环境。callee 提前退出时不执行输入，输入提前退出时不执行后续输入和函数主体。函数值不会因为实现中可见其来源，就重新获得声明的参数名、默认值或属性收集协议；这些协议的函数值载体尚未接通。

带 comptime 参数的 runtime 函数也可由 runtime 条件选择：当前物化器支持 `if`、`when`、`match` 和代码块交付的具体目标，物化时先按源码顺序完成需要计算的 staged 实参，每个生产者调用一次，再以冻结的数据构造各候选实例；支持现有调用准备能够冻结的标量、Type、元组、结构体和枚举等输入。这里不调用 runtime 选择条件或普通实参；runtime 仍先确定 callee，再只执行所选路径的普通实参。实参块中的局部资源、拥有型闭包捕获及指向外层循环的退出仍按普通调用规则处理。复制出的候选路径使用独立的局部绑定与清理边界，引用调用者已有变量的连接保持不变；未选路径不会构造或析构资源。候选复制失败不发布部分代码、源码标记或身份分配。

```flurry
fn same(comptime enabled: bool, value: bool) -> bool {
    enabled and value
}

fn opposite(comptime enabled: bool, value: bool) -> bool {
    not (enabled and value)
}

fn choose(flag: bool, value: bool) -> bool {
    let action = flag match {
        true => same,
        false => opposite,
    };
    action(true, value)
}
```

这里 `flag` 仍在 runtime 选择目标，`true` 在物化时绑定到相应实例。staged 输入也可写成 `prepare(config)` 这样的 comptime 计算，其中 `config` 来自外层已完成的 comptime 参数；同一个输入不会因为选择树具有多个候选而重复计算。所需输入计算失败时，不发布部分参数替换；普通 runtime 值仍不能越阶段进入该计算。`match` 的 guard 保留候选顺序；arm 中还可用 `if` 或 `match` 进一步选择具体目标，或先执行普通语句再交付目标。`if` / `when` 的分支和 `do` 代码块也可取得局部资源、执行普通语句，再返回具体函数。主语和 arm 局部资源在开始求值调用实参之前清理，不会被延长到被调函数执行期间。选择阶段的 `return`、`break` 等退出不执行实参。闭包实参捕获其它调用者变量不会阻止紧邻的局部 callee 别名解析；若捕获、读取或改写该别名本身，则仍保留它的可观察绑定。

相同的花括号语法也用于构造对象，必须按 callee 的实际类型区分：comptime 计算交付的 Type 作为构造类型消费，不先转换成 runtime 数据；普通函数值则进入调用准备。这不会取消普通常量跨阶段进入 runtime 时所需的 Splice 检查。

立即调用的 comptime 函数值也可以由 if、when、match 的分支交付。分支尾值保留调用阶段，只有最终调用结果经过 Splice；这一上下文不扩散到选择条件、模式 guard 或分支中的普通语句。若选择依赖尚不可用于 comptime 的 runtime 值，则拒绝调用，不能通过外层括号或花括号把该依赖提前求值。

开放函数中的 inline if、inline when 和 inline 模式选择同样保留这一边界。定义检查验证所有分支的可调用域及选择条件的阶段；实例化取得 comptime 输入后才选择分支，并交付该分支的真实闭包环境。即使本次实例不选择某个分支，也不能隐藏其中的类型或阶段错误。圆括号调用与花括号调用使用相同规则。




### 方法通过指针修改已有对象

~~~flurry
struct Counter {
    value: u32,

    fn set(*self, next: u32) {
        self.value.* = next;
    }

    fn read(*self) -> u32 {
        self.value.*
    }
}

test {
    let counter = Counter { value: 4 }
    counter.set(9);
    println("{}", counter.read());
}
~~~

输出 `9`。字段投影保留指针，读取或写入字段内容通过 `.*` 完成。

### 接收者先取得，内容可以被后续实参改变

接续上面的 Counter：

~~~flurry
fn change_then_supply(target: *Counter) -> u32 {
    target.value.* = 12;
    18
}

test {
    let counter = Counter { value: 4 }
    counter.set(change_then_supply(counter.ref));
    println("{}", counter.read());
}
~~~

输出 `18`。先为 set 取得接收者；实参计算把同一对象写为 12；随后进入 set 并写为 18。接收者适配没有创建冻结整个对象的独占借用。每次访问仍须满足实际内存契约。

### 提前返回及结果契约

~~~flurry
fn smaller(first: u32, second: u32) -> result: u32
where ensures result <= first and result <= second
{
    return first while first < second;
    second
}

test {
    println("{} {}", smaller(3, 8), smaller(9, 2));
}
~~~

输出 `3 2`，两个出口都交付同一个结果契约。

### 实参准备中断

~~~flurry
fn unavailable() -> ?u32 {
    println("stop");
    null
}

fn later() -> u32 {
    println("later");
    9
}

fn add(first: u32, second: u32) -> u32 {
    first + second
}

fn attempt() -> ?u32 {
    add(unavailable()?, later())
}

test {
    let result = attempt();
    result match {
        null => println("absent"),
        value? => println("{}", value),
    }
}
~~~

输出 `stop`、`absent`。传播在调用准备时离开 attempt，later 和 add 的主体均未执行。

### 准备失败会清理已经取得的资源

这是独立的完整例子。Trace 的 Drop 输出用来观察责任最终在哪里结束。

~~~flurry
struct Trace {
    id: u32,
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

fn acquire() -> Trace {
    println("acquire");
    Trace { id: 1 }
}

fn unavailable() -> ?u32 {
    println("unavailable");
    null
}

fn submit(resource: Trace, value: u32) {
    println("submitted {}", value);
}

fn attempt() -> ?u32 {
    submit(acquire(), unavailable()?);
    1
}

test {
    attempt() match {
        null => println("cancelled"),
        value? => println("{}", value),
    }
}
~~~

输出依次为 `acquire`、`unavailable`、`drop 1`、`cancelled`。submit 没有进入；准备期间取得的 Trace 仍须清理。不能把清理只放进 submit 的函数退出路径。

### 返回值移走责任，局部清理处理剩余值

接续 Trace 的定义：

~~~flurry
fn select_resource() -> Trace {
    let unused = Trace { id: 2 }
    let selected = Trace { id: 3 }
    return selected;
}

test {
    let result = select_resource();
    println("received {}", result.id);
}
~~~

输出 `drop 2`、`received 3`、`drop 3`。selected 的责任随结果离开函数，不能在局部退出时先销毁，再把原字节作为有效返回值。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 调用操作数也按源码从左到右求值。C11 不给各实参规定这样的先后；C 程序可先用独立语句保存结果来建立次序。[Rust operand order](https://doc.rust-lang.org/reference/expressions.html#evaluation-order-of-operands)、[C11 §6.5.2.2](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)

C++ 默认实参不能以被求值的方式读取同一函数的前面参数；Flurry 的缺省计算可以依赖已经取得的前面参数。[C++ default arguments](https://eel.is/c++draft/dcl.fct.default)

## 运行时实现设计引导

为调用准备维护按实际取得次序排列的临时值序列。每一步只有成功交付后才登记已初始化状态；发生退出时按资源规则清理仍由调用者持有的部分。

~~~text
prepare callee
for explicit input in source order:
    evaluate once
    register value and obligations
for missing input in parameter dependency order:
    evaluate supplier or default
    register value and obligations
validate and transfer prepared inputs
execute body
receive result and remaining state
~~~

准备临时量到参数的交付改变责任归属，不能让调用点和函数退出各销毁一次。返回值已经移出局部位置后，局部清理不得再次处理同一责任。

| 退出位置 | 谁还持有已取得的输入 | 实现动作 |
|---|---|---|
| callee 求值失败 | 已经建立的外层临时量 | 不开始实参求值，沿当前退出边清理 |
| 某个显式输入失败 | 调用准备持有此前成功输入 | 停止后续输入，清理已取得责任 |
| 缺省供应失败 | 调用准备持有显式输入及此前缺省值 | 不进入主体，按真实状态清理 |
| 主体正常或提前返回 | callee 持有已交付参数和局部责任 | 先确定结果交付，再清理剩余责任 |

实现可直接把值构造到最终参数或结果位置，前提是这些位置的责任状态与上述过程一致。不要求为了表达语义次序额外复制所有输入。

## 编译器实现引导

按通用属性规则读取函数 ABI，并将其作为函数签名及调用降码所需的信息保存。

先建立实参到形参的映射，再按源码生成执行计划。不要为了把机器参数排到 ABI 顺序，就重排有可观察效果的源表达式。机器传参排列发生在值已经取得之后。

每个准备步骤至少记录来源表达式、目标参数、阶段、适配、已初始化状态和失败后继。对默认表达式还记录定义作用域及其前序参数依赖。

normalization 检查可确定的签名与调用要求，instantiation 完成具体参数与环境交付。资源检查覆盖调用准备、主体和返回；缺少调用契约时不得用类型相同替代访问证明。

无协议执行时，n 个实参的映射和计划可用 O(n) 表构造；名字查找采用已建立的参数索引。类型求解、协议与契约验证的成本另计。验收应覆盖具名换序、实参传播、未使用参数、接收者被后续实参影响及结果逃逸。
