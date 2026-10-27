# Once、Clone、Copy

## Intro

消耗一个值，意味着把它交出去，或结束它承担的责任。非 Copy 值交出后，原位置不能再交付同一个值。

如果需要保留原值，可以显式调用 Clone；如果类型允许按位复制，则由 Copy 提供隐式复制。只想读取或修改现有数据时，可以使用[指针](pointers.md)。

## 语法

~~~text
trait_bound_decl_clause -> id :- expr
impl_trait_def -> impl expr for expr clauses? { (assoc_decl | definition | statement)* }
derive_def -> derive expr for expr clauses?
clone_call -> expr.clone()
~~~

正向 trait 约束可以使用 `A + B`。impl 与 derive 的完整规则见[Trait](../traits/index.md)。

## 正式定义

### Once 与按值交付

运行时值默认遵循 Once 的仿射使用边界：值至多被消耗一次，不要求每个值都由用户显式消耗。这一设计借鉴了仿射类型理论——通过限制每个值的消耗次数，编译器可以在静态分析阶段确定每份资源的唯一归属，从而在不依赖垃圾回收的前提下防止重复释放和悬垂引用。当值离开清理范围时，自动清理兑现仍然存在的责任。

对于非 Copy 值，初始化另一个按值绑定、按值传参和返回，均转移该值及其可转移责任，并且源位置成为已移出状态。读取已移出的值、再次移出或再次销毁同一责任均非法。

Once 的一次限制针对消耗，不限制消耗前满足契约的观察次数。通过指针观察不会凭空产生另一份 owner，而修改也仍需满足对象及方法契约。

当前 builtin 库通过显式实现让元组进入要求 Once 的泛型接口：

~~~flurry
impl resources.Once for Tuple<...Es>
where Es: meta.List<Type> {}

fn forward(value: T) -> T where T :- resources.Once { value }
~~~

`Es` 是一个完整类型列表输入，不是若干独立类型参数。实现位于资源 trait 的定义包，构造器单独作为另一个包加载时不会自动取得这份实现。该标记不赋予 Clone／Copy，也不替代元素的清理；包含非 Copy 字段的元组仍只能按移动规则交付。通用逐元素 Clone／Copy 尚未接通。

原生可选类型及单一错误类型的结果也由 builtin 提供 Once 实现：

~~~flurry
impl Once for ?T where T {}
impl Once for !E T where E, T {}
~~~

因此 `forward` 可以直接交付 `?Token`、`??Token` 或 `!Error Token`，无需库包装枚举。空值没有载荷清理；present、成功或错误分支只转移实际存在的责任，离开清理范围时执行一次清理。该实现不要求载荷先具有显式 Once 标记，也不替载荷提供 Clone／Copy。任意异构错误行的通用 Once 实现、借用载荷的通用 Clone 与条件 Copy 仍待接通。

`ErrorResultOps.inspect` 与 `inspect_err` 接收整个结果的所有权，分别在成功或错误分支把载荷的指针交给回调，随后返回原分支及其完整载荷：

~~~flurry
struct Ticket { valid: bool }
struct Failure { retryable: bool }

fn check_ticket(ticket: *Ticket) { assumes ticket.valid.*; }
fn check_failure(failure: *Failure) { assumes failure.retryable.*; }

fn check(value: !Failure Ticket) -> !Failure Ticket {
    value.as(ErrorResultOps<Ticket, Failure>).inspect(check_ticket)
        .as(ErrorResultOps<Ticket, Failure>).inspect_err(check_failure)
}
~~~

这里不要求 Ticket 或 Failure 具有 Copy。每次调用最多执行一次回调，另一分支跳过回调；载荷的清理责任随返回值继续传递。回调自身的捕获环境无论是否被调用都按普通资源规则清理。回调通过指针执行的操作仍须符合内存契约，不能借此把局部载荷的地址逃逸到更长的生命周期。

### Clone

Clone 显式产生另一个合法值，同时保留合法源值。两值之间的共享关系、分配、效果和结果依赖由具体实现契约规定。

克隆可以复制指针，也可以分配独立缓冲区，或通过共享管理协议增加持有责任。因此不能由 Clone 推出"所有可达资源互不相干"。

对于含内部依赖的聚合，克隆 owner 后还必须检查视图指向哪里。逐字段克隆不会自动把一个旧指针重写成指向新缓冲区的指针，因此具体实现需要重建该关系，或交付真实的共享依赖。

### Copy

Copy 提供合法的表示复制。当初始化、赋值、按值参数和返回值需要交付 Copy 值时，编译器复制其表示而保留源值，不隐式调用任意用户 clone 方法。

`Copy :- Clone :- Once` 描述能力关系；不过复制动作仍有独立于显式 clone 调用的契约。

内建数值类型与普通指针按各自定义提供 Copy。用户名义类型必须显式 impl 或 derive；字段都可 Copy 是派生的检查条件，不是自动取得能力的声明。

复制不得重复独占终结责任。仅有一个可复制的指针字段，不足以证明拥有该分配的包装也可 Copy。复制视图保留真实目标依赖，不取得目标的释放权，也不保证目标内容不变。

### 三种动作的区别

| 动作 | 源位置 | 目标及责任 | 对外效果 |
|---|---|---|---|
| 非 Copy 交付 | 已移出 | 取得原值与可转移责任 | 按真实操作记录，不自动释放目标资源 |
| Copy | 保持可用 | 合法表示副本，保留依赖 | 不隐式执行用户克隆算法 |
| Clone | 按实现契约保持合法 | 新值及其实际共享或独立责任 | 按实现计入读取、分配和管理状态变化 |

## 常见示例

### 交付之后不能再读取

~~~flurry
struct Ticket {
    number: u32,
}

fn finish(ticket: Ticket) {
    println("{}", ticket.number);
}

test {
    let ticket = Ticket { number: 7 }
    finish(ticket);
    -- 再次执行 finish(ticket) 会重复交付已移出的值。
}
~~~

Ticket 没有声明 Copy，即使唯一字段是 u32，编译器也按移动规则交付。

### 显式取得 Copy

~~~flurry
struct Point {
    x: i32,
    y: i32,
}

derive Clone for Point;
derive Copy for Point;

test {
    let first = Point { x: 3, y: 4 }
    let second = first;
    println("{} {}", first.x, second.x);
}
~~~

两个 Point 有独立的内联字段。派生检查仍须确认字段复制和整个类型的资源契约相容。

### Clone 可以保留共享目标

~~~flurry
struct NumberView {
    pointer: *i32,

    fn read(*self) -> i32 {
        self.*.pointer.*
    }
}

impl Clone for NumberView {
    fn clone(*self) -> Self {
        Self { pointer: self.pointer.* }
    }
}

test {
    var number = 3i32;
    let first = NumberView { pointer: number.ref }
    let second = first.clone();
    number = 4i32;
    println("{} {}", first.read(), second.read());
}
~~~

输出 `4 4`。两个包装都依赖 number，而不互相依赖对方的局部槽位。

### 泛型重复交付需要能力依据

~~~flurry
fn duplicate(value: T) -> Tuple<T, T>
where T :- Copy
{
    (value, value)
}
~~~

若没有 Copy 依据，第二次按值交付不能成立。编译器不能因为尚未找到 Copy 实现，就在开放约束中自动证明其否定。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的 Copy 同样是隐式表示复制，Clone 则允许自定义行为及共享；名称相近不应被解释为"Clone 必须深复制"。[Rust Copy](https://doc.rust-lang.org/std/marker/trait.Copy.html)、[Rust Clone](https://doc.rust-lang.org/std/clone/trait.Clone.html)

C 的结构赋值复制字段表示，不为拥有资源的指针自动转移释放责任。C++ 的复制和移动构造由相应函数实现；而 Flurry 的普通非 Copy 交付直接改变源位置的可用状态。Zig 的普通值复制不提供这套仿射交付保证，因此需要库协议管理拥有资源的值。

## 运行时实现设计引导

Copy 可降为寄存器复制或按布局复制；move 可采用同样的物理传输，但在语义上责任只保留在目标。运行时无需为了表示"已移出"而擦除原内存字节。

Clone 按已选择实现正常调用。分配失败、引用计数变化、捕获环境和清理都按该调用的实际契约执行，因此不能将 clone 当作可任意合并的无副作用复制。

## 编译器实现引导

对每个按值交付操作：

1. 验证源位置在当前路径上持有合法值。
2. 根据已成立的 Copy 证据选择表示复制，否则应用仿射交付。
3. 复制保留源状态；移动把相应位置置为已移出。
4. 在目标安装实际依赖和可转移责任，保持来源区域身份。
5. 更新后续自动清理，只清理仍持有的责任。

派生按字段收集能力和资源条件，再核验整体不变量。当字段有交叉依赖时，逐字段成功不代表整个产物成立。检查失败时，编译器应指出重复责任、未重建的目标或缺失的能力，而不只报告"不能复制"。
