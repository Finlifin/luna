# 函数值与高阶接口

## Intro

高阶函数把一部分计算交给另一个可调用值。接口除了说明参数和结果，还需要说明回调需要什么环境，以及调用会怎样影响资源。

一个回调可以没有捕获，也可以保存自己的状态。接受两者的接口应当保留环境参数。

## 语法

~~~text
fn_type -> pure? comptime? inline? (unsafe | spec | verified)? fn(parameter_type*) (-> expr)?
    | pure comptime fn<parameter_type*> (-> expr)?
closure_type -> %closure_context expr
closure_context -> expr | { environment_binding* }
environment_binding -> id : expr (= expr)?
type_bound_decl_clause -> id : expr
lambda_param -> lambda parameter
~~~

环境参数使用 `c: Type.vars.Closure`。

函数返回类型中的 `->` 向右结合：`fn() -> fn() -> bool` 表示返回一个
`fn() -> bool` 函数值的无参函数，即 `fn() -> (fn() -> bool)`。
`comptime fn` 使用相同的分组规则；括号可以显式划定类型表达式的范围。

`pure comptime fn<...> -> Type` 描述范式 callable；尖括号输入属于这个
函数值的类型，不能改写成圆括号的普通 comptime 函数类型。内置包集中绑定层
中的 Tuple 声明使用这一形式：

```flurry
pub const Tuple: pure comptime fn<...Es: meta.List<Type>> -> Type = .Tuple'builtin;
```

命名输入按顺序进入作用域，后续输入及结果类型可引用前面的绑定；嵌套函数
类型建立自己的参数作用域。当前实现已接通语法、HIR 与 NIR 类型表示，支持
依赖参数替换、类型身份比较，以及将该类型作为 comptime Type 值返回和传输。
范式 callable 类型不能作为 runtime 参数或 quote 上下文中的 runtime 绑定。
callable 值本身的取得、应用和上述 builtin 绑定的执行路径仍待接通；Tuple
尚未迁移到该绑定。

## 正式定义

### 普通函数值

普通 fn 目标接收与该接口相容、无需非空环境的函数值。无捕获 lambda 可通过空环境消去接入。带捕获的 lambda 不能无条件隐式封装到这个目标，更不能暗中分配一个盒子来隐藏环境。

普通函数值的表示由其合法 ABI 与实现决定，不从“没有捕获”推出所有函数种类都具有同一种机器地址表示。

函数值的取得与调用分开。例如条件表达式可以选择一个与目标相容的普通函数，随后再用它调用。选择时的 runtime 条件不会把函数变成 comptime 函数。函数名字、函数值和调用结果也不能因为共享一段源码就当成同一种对象。

### 依赖函数类型

显式的 `comptime Type` 输入可以被后续输入和返回类型引用：

```flurry
typealias IdentityCallback = fn(comptime T: Type, value: T) -> T;

fn keep(comptime T: Type, value: T) -> T { value }

fn example() -> bool {
    let callback: IdentityCallback = keep;
    callback(bool, true)
}
```

这些名字属于该签名的局部作用域，按参数顺序绑定。嵌套函数类型可以引用外层输入，也可以用自己的同名参数遮蔽它；签名之外不能引用这些名字。形成类型后，名字被抽象为对应层级的参数位置，因此一致重命名不改变类型身份。`comptime fn` 类型遵循相同的绑定规则并保留自身阶段。

显式 comptime 值输入也可以作为 normal-form 参数。例如 `fn(comptime flag: bool, value: Tagged<flag>) -> Tagged<flag>` 保留 `Tagged` 的头和对 `flag` 的依赖，并不展开 `Tagged` 的函数体。值参数的域可以依赖更早的 Type 输入；嵌套函数类型引用外层值时，值与其域都按原绑定层级解释。

当前编译器已接入上述 Type 依赖、显式 comptime 值向 normal-form 参数的传递、命名位置参数和对应的普通／comptime 函数值调用。普通 runtime 输入不能因此充当静态 Type 或 comptime 值依据。任意依赖值计算，以及可选、隐式等函数类型参数协议仍需独立接入，不由这里的命名语法隐式启用。

### 环境参数化

环境参数化的目的是让高阶函数的签名能同时接受带捕获和无捕获的回调，而不需要类型擦除或隐式堆分配。
`%c fn(...) -> R` 保留回调环境。c 可以为空，也可以含有捕获及依赖；传参、返回或保存时都必须交付对应的真实环境。

函数体文本不参与名义类型身份，环境中的普通数值也不单独制造一种闭包类型。类型等价仍须满足环境结构、绑定关系及真实参与类型的依赖。

闭包、CMTT 类型与 reachability 等嵌入层类型不能通过用户 impl 创建任意语言操作。泛型实现也不能绕过这一主体资格限制。语言提供的调用与资源能力不要求为每个 lambda 编写 impl。

### 契约保持

回调替换不能加强调用者无法满足的前提，不能弱化承诺的结果或扩大允许的效果与环境需求。一个回调只读，另一个会消耗捕获，不能只因为参数和返回类型相同就当作可重复调用的同一种行为。

高阶函数是否保存回调、调用几次、在哪条路径调用，以及怎样交付结果依赖，都影响资源检查。主体可见时按现有推导检查；跨接口时使用已交付的契约，不能假设回调不逃逸或无限次可调用。

设计接口时要分别交代：

| 行为 | 需要保留的关系 |
|---|---|
| 立即调用一次 | 该次调用前提、结果和出口状态 |
| 在循环中调用 | 每轮入口重新成立的依据 |
| 原样返回回调 | 环境及责任随返回值转移 |
| 组合后返回新回调 | 新环境持有两个回调及各自依赖 |
| 保存到外层容器 | 环境和全部可达依赖满足目标有效期 |

同样的参数和结果类型不回答“会调用几次”。也不能把具有一次责任交付的回调塞进一个依赖重复交付的接口，再期待优化内联消除冲突。

内存效果、访问条件与程序状态交给程序验证层。PTC 检查类型命题，不能承担整个高阶程序的行为证明。

## 常见示例

### 无环境目标

~~~flurry
fn apply_plain(action: fn(u32) -> u32, value: u32) -> u32 {
    action(value)
}

fn increment(value: u32) -> u32 {
    value + 1
}

test {
    println("{}", apply_plain(increment, 7));
    println("{}", apply_plain(|value: u32| value + 2, 7));
}
~~~

输出 `8`、`9`。两个可调用值都不需要外部局部环境。

### 在运行时选择一个函数值

~~~flurry
fn identity(value: u32) -> u32 {
    value
}

fn halve(value: u32) -> u32 {
    value / 2
}

fn transform(value: u32, reduce: bool) -> u32 {
    let action: fn(u32) -> u32 = if reduce {
        halve
    } else {
        identity
    }
    action(value)
}

test {
    println("{} {}", transform(20, false), transform(20, true));
}
~~~

输出 `20 10`。reduce 是普通参数，选择发生在 runtime。两个目标具有相容的无环境接口；实现可以通过间接调用，或在有证明时化为直接控制流。

~~~flurry
-- 拒绝：apply_plain 的目标不足以交付 offset。
fn invalid_plain_target(offset: u32) -> u32 {
    let action = |value: u32| value + offset;
    apply_plain(action, 7)
}
~~~

### 接收带环境回调

~~~flurry
fn apply(action: %c fn(u32) -> u32, value: u32) -> u32
where c: Type.vars.Closure
{
    action(value)
}

test {
    let offset: u32 = 10;
    let first = apply(|value: u32| value + offset, 5);
    let second = apply(|value: u32| value, 5);
    println("{} {}", first, second);
}
~~~

输出 `15 5`。这里不要求把捕获统一转换为 Any 或某个公共环境结构。

### 传递时保留完整类型

~~~flurry
fn relay(action: %c fn(u32) -> u32) -> %c fn(u32) -> u32
where c: Type.vars.Closure
{
    action
}

test {
    let offset: u32 = 3;
    let action = relay(|value: u32| value + offset);
    println("{}", action(4));
}
~~~

输出 `7`。relay 交出完整值及其责任；它没有把捕获环境擦除为普通 fn。

### 返回自行拥有的环境

~~~flurry
fn make_offset(offset: u32) -> %{offset: u32 = offset} fn(u32) -> u32 {
    |value: u32| value + offset
}

test {
    let action = make_offset(10);
    println("{}", action(5));
}
~~~

输出 `15`。结果环境保存合法的数值副本，不依赖 make_offset 的局部槽位继续存活。类型里的环境绑定描述与 lambda 的捕获对应，不要求额外执行一次捕获。

### 组合两个回调，返回完整环境

~~~flurry
fn compose(
    first: %a fn(u32) -> u32,
    second: %b fn(u32) -> u32,
) -> %{
    first: %a fn(u32) -> u32 = first,
    second: %b fn(u32) -> u32 = second,
} fn(u32) -> u32
where a: Type.vars.Closure,
    b: Type.vars.Closure
{
    |value: u32| {
        let middle = first(value);
        second(middle)
    }
}

test {
    let limit: u32 = 10;
    let clamp = |value: u32| {
        if value > limit {
            limit
        } else {
            value
        }
    }
    let halve = |value: u32| value / 2;
    let action = compose(clamp, halve);
    println("{} {}", action(8), action(28));
}
~~~

输出 `4 5`。新环境按主体首次引用次序取得 first、second，每个字段都是完整的可调用值；first 内部又保留 limit。两个原参数的局部槽位在 compose 返回后结束，但责任已经交入新环境。

本例两个回调都可在当前状态下再次调用，因而 action 可以调用两次。compose 的语法本身不使任意输入都获得此性质；若某个输入调用后消耗了捕获，新回调必须保留这一状态变化。

### 同一环境类型可以装有不同值

接续 make_offset：

~~~flurry
test {
    let first = make_offset(10);
    let second = make_offset(20);
    println("{} {}", first(1), second(1));
}
~~~

输出 `11 21`。两个环境中的普通数值不同，不单独制造两个名义闭包类型；调用时仍须使用各自的实际环境，不能因为类型比较成功就共用 first 的捕获值。

### 转交闭包不提前执行它

接续 relay：

~~~flurry
test {
    let label: u32 = 7;
    let action = relay(|value: u32| {
        println("callback {}", label);
        value
    });
    println("relayed");
    println("result {}", action(3));
}
~~~

输出 `relayed`、`callback 7`、`result 3`。relay 传递环境及责任，没有为了确定返回类型执行回调主体。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 以闭包类型及调用 trait 表达高阶约束；C++ 可用模板接收闭包类型，也可使用库中的类型擦除包装。Flurry 用环境限定和相应契约保留回调需求，不按 lambda 创建点产生独立名义类型。[Rust closures](https://doc.rust-lang.org/reference/types/closure.html)、[C++ lambda](https://eel.is/c++draft/expr.prim.lambda)

C 的函数指针与上下文指针是实际替代；Zig 可以通过显式结构和 comptime 泛型组织回调。保存上下文表示仍不自动证明其指向对象有效。[C11 §6.7.6.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[Zig functions](https://ziglang.org/documentation/master/#Functions)

## 运行时实现设计引导

一种实现是代码入口加合法环境位置，也可以在特化后直接传环境字段。对于被调用者已知的实例，允许消去间接调用；对于动态来源，按实际接口分派。

跨函数交付不强制复制整个环境。移动、共享与间接取得都必须对应真实资源契约；不能把一个指向短命环境的地址当作已经拥有全部环境。

返回闭包时，结果承载位置必须在局部清理之后仍可用。环境中的外部依赖独立检查，不能仅凭结果承载空间存在就判定所有内部指针有效。

回调组合可将两个环境作为字段保存，也可选择等价的直接传递表示。无论采用哪一种，都要保存调用顺序及所有权：first 的输出成功交付后才调用 second；first 失败时不开始 second；未被调用的环境字段仍有退出清理责任。

## 编译器实现引导

1. 分别比较主体签名、修饰和环境上下文，应用空环境消去及合法绑定改名。
2. 保留回调要求、效果、结果依赖和调用后状态，检查替换是否兑现目标契约。
3. 对每个实际调用实例化环境对应，不用“同类型”代替“同对象”。
4. 检查高阶主体中的保存、调用及结果传播，组合实际控制路径的资源状态。
5. 根据逃逸和 ABI 要求选择承载方案，生成代码与环境交付。

泛型 c 未知时保留环境义务；仅凭 c 存在不能推导其 Copy、无依赖或可任意排列。主体实际允许用户 impl 的资格在实现选择中检查，不能用通配泛型实现替代语言提供的调用语义。

对组合接口，不只组合函数类型，还要组合调用状态变换。每个出口记录哪些环境字段仍持有值、结果依赖哪一份原环境、以及是否需要外部对象继续存活。若无法从接口得到必须的高阶保持关系，应要求足够的程序契约或受检依据；不能把这种不足交给 PTC 猜测。
