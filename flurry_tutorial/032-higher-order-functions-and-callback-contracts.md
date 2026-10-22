# Flurry 演进手记 (032)：写一个零开销的过滤器——高阶函数接口、环境传递与回调纯度契约

在 [上一篇 (031)](./031-closure-environments-and-cleanup-order.md) 中，我们认识了 `%c fn(...)` 闭包环境限定类型。
今天，我们要转换身份——从“写 Lambda 的调用者”，变成**“设计传感器流处理管线的高阶函数库作者”**。

假设我们要为传感器固件编写三个核心高阶工具：
1. **`apply`**：接收一个单步转换函数 `action` 和一个传感器读数 `value: u32`，返回 `action(value)`；
2. **`make_offset`**：一个闭包工厂，接收 `offset: u32`，返回捕获了该偏移量的校准闭包；
3. **`compose`**：接收两个转换回调 `first` 和 `second`（它们可能各自捕获了不同的阈值或状态环境。），把它们串联组合成一个全新的回调函数返回。

如果我们按照 C 语言函数指针的习惯，把高阶函数的参数直接写成 `action: fn(u32) -> u32`，会发生什么？让我们从 传统开始动手试试（详见 `docs/functions/higher-order-functions.md`）。

---

## 一、 只写普通 `fn(u32) -> u32` 的局限——把带捕获的闭包拒之门外

我们先写出只接收无环境函数类型 `fn(u32) -> u32` 的 `apply_plain`：

```flurry
use std.prelude.*;

-- 参数类型是普通的无环境函数类型 fn(u32) -> u32
fn apply_plain(action: fn(u32) -> u32, value: u32) -> u32 {
    action(value)
}

fn increment(value: u32) -> u32 {
    value + 1u32
}

test {
    -- 1. 传入普通顶层函数：完全合法。
    println("{}", apply_plain(increment, 7u32)); -- 8

    -- 2. 传入无捕获的 Lambda：因为空环境 %{} 自动消去为普通 fn，也完全合法。
    println("{}", apply_plain(|value: u32| value + 2u32, 7u32)); -- 9
}
```

你甚至可以在运行时用 `if ... else ...` 动态选择两个无环境的普通函数值，再赋给 `action: fn(u32) -> u32` 调用：

```flurry
fn identity(value: u32) -> u32 { value }
fn halve(value: u32) -> u32 { value / 2u32 }

fn transform(value: u32, reduce: bool) -> u32 {
    -- 在 runtime 根据布尔条件选择一个无环境函数值
    let action: fn(u32) -> u32 = if reduce {
        halve
    } else {
        identity
    }
    action(value)
}
```

但是。一旦调用者想传一个**捕获了外部变量 `offset` 的闭包**给 `apply_plain`，编译器就会立刻亮起红灯：

```flurry
-- ❌ 编译期直接报错。
-- apply_plain 的形参类型是无环境的 fn(u32) -> u32，
-- 无法容纳并交付 action 所需的非空捕获环境 %{ offset: u32 = offset }。
fn invalid_plain_target(offset: u32) -> u32 {
    let action = |value: u32| value + offset;
    apply_plain(action, 7u32)
}
```

**为什么 Flurry 禁止把带捕获的 `action` 偷偷塞进普通的 `fn(u32) -> u32` 里？**
因为在系统编程中，普通的 `fn(u32) -> u32` 不携带任何外部环境存储；如果语言为了“方便”而偷偷在堆上 `malloc` 一个盒子把 `offset` 藏起来擦除掉，就会引入隐式堆分配和失控的生命周期。

那我们怎样才能写出一个**既免生隐式堆分配、又能同时接受“无捕获函数”和“任意带捕获闭包”**的高阶函数接口呢？

---

## 二、 环境参数化——`%c fn(...)` 与 `where c: Type.vars.Closure`

答案就是：**对闭包环境进行参数化（Environment Parameterization）**。

在 Flurry 中，你可以在函数的 `where` 子句里声明一个**闭包环境变量 `c: Type.vars.Closure`**，然后把回调参数的类型写成 **`%c fn(u32) -> u32`**。

下面来看看 `%c fn(u32) -> u32` 是如何同时适配有环境和无环境回调的：

```text
  高阶函数签名:
  fn apply(action: %c fn(u32) -> u32, value: u32) -> u32
  where c: Type.vars.Closure

        ┌───────────────────────────────────┴───────────────────────────────────┐
        ▼ 场景 1：传入 |v| v + offset                                            ▼ 场景 2：传入 |v| v (无捕获)
  c 实例化为: { offset: u32 = offset }                                    c 实例化为: 空环境 {}
  形参变为:   %{ offset: u32 = offset } fn(u32) -> u32                    空环境消去: fn(u32) -> u32
  ──► 零擦除、零堆分配，直接携带 offset 环境调用。                          ──► 直接作为无环境函数调用。
```

来看看升级后的 改进代码，以及原样转发闭包的 `relay` 函数：

```flurry
use std.prelude.*;

-- 1. 接收任意环境 c 的高阶调用函数
fn apply(action: %c fn(u32) -> u32, value: u32) -> u32
where c: Type.vars.Closure
{
    action(value)
}

-- 2. 原样转发带环境的闭包，完整保留它的 %c 环境与资源责任。
fn relay(action: %c fn(u32) -> u32) -> %c fn(u32) -> u32
where c: Type.vars.Closure
{
    action
}

test {
    let offset: u32 = 10;

    -- 同一套 apply 接口，既能接带捕获的闭包，又能接无捕获的闭包。
    let first = apply(|value: u32| value + offset, 5u32);
    let second = apply(|value: u32| value, 5u32);
    println("{} {}", first, second); -- 输出: 15 5

    -- 通过 relay 转交闭包（转交过程不会提前执行闭包主体。）
    let relayed_action = relay(|value: u32| {
        println("running with offset {}", offset);
        value + offset
    });
    println("relayed ready");
    println("result: {}", relayed_action(4u32));
}
```

> **🔍 语法小贴士：函数返回类型中的 `->` 是右结合的。**
> 当一个函数返回另一个函数时，`fn() -> fn() -> bool` 等价于 `fn() -> (fn() -> bool)`。而当返回的函数带有闭包环境限定时，直接把 `%{ ... }` 写在返回的 `fn` 前面即可，比如我们马上要看到的闭包工厂。

---

## 三、 返回闭包与组合双闭包——显式环境绑定签名

现在来到最酷的实战：
1. 我们要写一个工厂函数 `make_offset(offset: u32)`，它在内部创建一个捕获了参数 `offset` 的闭包并返回给调用者；
2. 我们要写一个高阶组合器 `compose(first, second)`，它接收一个环境为 `%a` 的回调 `first` 和一个环境为 `%b` 的回调 `second`，返回一个先调 `first` 再调 `second` 的新闭包。

它们的返回类型该怎么写？
直接用我们在上一篇学到的 **`%{ name: Type = source } fn(...)`** 语法，把新闭包捕获的字段明明白白地写在返回类型上。

```flurry
use std.prelude.*;

-- 1. 闭包工厂：返回环境自己持有 offset (u32 副本) 的闭包
fn make_offset(offset: u32) -> %{ offset: u32 = offset } fn(u32) -> u32 {
    |value: u32| value + offset
}

-- 2. 组合两个带不同环境的回调 (%a 与 %b)：
--    返回的新闭包环境按首次引用顺序依次持有 first 和 second。
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
    -- 测试工厂：同一环境类型可以装有不同的实际运行值。
    let add_10 = make_offset(10u32);
    let add_20 = make_offset(20u32);
    println("factory: {} {}", add_10(1u32), add_20(1u32)); -- 11 21

    -- 测试组合器：clamp 捕获了外部的 limit，halve 无捕获
    let limit: u32 = 10;
    let clamp = |value: u32| {
        if value > limit {
            limit
        } else {
            value
        }
    }
    let halve = |value: u32| value / 2u32;

    -- compose 将 clamp (带 limit 环境) 与 halve 组装成一个流水线闭包。
    let pipeline = compose(clamp, halve);
    println("pipeline: {} {}", pipeline(8u32), pipeline(28u32)); -- 4 5
}
```

如下所示，欣赏一下 `compose(clamp, halve)` 返回的嵌套环境结构：

```text
  pipeline 的类型:
  %{
      first:  %{ limit: u32 = limit } fn(u32) -> u32 = clamp,
      second: fn(u32) -> u32                         = halve,
  } fn(u32) -> u32

  物理与语义环境盒子:
  ┌────────────────────────────────────────────────────────┐
  │ pipeline 环境                                           │
  │  ├─ 字段 #0 (first):  [ clamp 主体 | 环境: limit = 10 ] │
  │  └─ 字段 #1 (second): [ halve 主体 | 空环境 ]           │
  └────────────────────────────────────────────────────────┘
   │
   ├─► 调用 pipeline(28):
   │     1. 先调 first(28) ──► 读取 limit(10) ──► 得到 middle = 10
   │     2. 再调 second(10) ──► 10 / 2 ──► 返回 5。
   │
   └─► 状态与清理保证：
         • 如果 first(value) 中途失败提前退出，不开始调用 second。
         • 两个回调在调用后都没有消耗掉各自的捕获，因此 pipeline 可以安全地调用第二次。
```

### 高阶函数的契约红线：同样的参数返回类型 $\neq$ 可以随便替换

在设计高阶接口时，有一条必须牢记的核心规则（详见 `docs/functions/higher-order-functions.md` §契约保持）：
> **“一个回调只读，另一个会消耗捕获，无法只因为参数和返回类型相同就当作可重复调用的同一种行为。”**

假设你写了一个会在循环里把回调执行 **两次** 的高阶函数 `run_twice(action: %c fn() -> u32)`。如果调用者传进来的 `action` 是一个**调用一次就会把环境里的并非 `Copy` 票据 `Ticket` 移交销毁**的Once闭包，编译器在进行资源与状态验证时，就会发现在第一轮调用出口处 `action` 的环境已经残缺，无法满足第二轮调用的入口前提，从而在编译期精准拦截。

---

## 本篇小结

| 高阶接口形态 | 签名写法 | 适用场景与语义契约 |
| :--- | :--- | :--- |
| **无环境函数目标** | `action: fn(A) -> B` | 仅接收顶层函数或无捕获 Lambda；严禁隐式堆分配偷藏捕获 |
| **环境多态高阶参数** | `action: %c fn(A) -> B` + `where c: Type.vars.Closure` | 零开销同时接收有捕获和无捕获回调，完整保留环境身份与资源责任 |
| **返回闭包的工厂** | `-> %{ id: T = val } fn(A) -> B` | 将按值捕获的环境随返回值安全移交出函数边界 |
| **多回调组合器** | `-> %{ first: ..., second: ... } fn(A) -> B` | 按首次引用次序组合多个子回调及其各自环境，精确传递调用后状态 |

---

### 下一篇预告

现在，我们的传感器流处理管线已经能通过 `%c fn` 接收并组合各种小巧的转换函数了。
但是，在性能敏感的微控制器内核循环里，有些只有两三行的小函数（比如 `smaller(a, b)`，或者在模式匹配守卫 `if accepts(value)` 里调用的判断函数），我们希望编译器把它的函数体在调用处展开，消除函数调用的指令开销。

说到“展开”，你可能会想到两样东西：
1. C/Rust 里的 `inline` 关键字（或者 `#[inline]` 属性）；
2. C 语言里的 `#define MAX(a, b) ((a) > (b) ? (a) : (b))` 文本宏。

可是，C 的宏如果遇到 `MAX(produce(), 10)`，会把带副作用的 `produce()` 展开并执行**两次**。而且如果在展开的代码里写一句 `return`，它会直接把**调用者外层函数**给提前返回掉。
在 Flurry 中，**`inline fn`** 和 **`quote fn`** 到底有什么本质区别？为什么无论怎么展开，Flurry 都严格保证**普通实参只求值一次**，且展开函数里的 `return` 和局部变量的 `Drop` 免将调用者的作用域搞乱？

在下一篇 **《Flurry 演进手记 (033)：比宏更干净、比普通函数更透明——inline fn 与编译期确定性展开》** 中，我们将深入剖析函数内联展开的卫生性与资源边界。
