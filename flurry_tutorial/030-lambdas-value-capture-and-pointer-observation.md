# Flurry 演进手记 (030)：当 Lambda 捕获外部世界——|x| expr、按值捕获与最小独立位置指针观察

在 [上一篇 (029)](./029-affine-pattern-matching-as-whole-and-or.md) 的结尾，我们开始为传感器分析仪设计动态校准回调：允许调用者就地写一个匿名函数（Lambda 表达式），把外部的校准偏移量 `offset` 或硬件票据 `ticket` 捕获进来。

如果你用过 C++、Rust 或 JavaScript，你一定对闭包的“捕获（Capture）”既爱又恨：
- 在 JS 或 Python 里，闭包总是捕获外部变量的引用，导致经典的循环变量捕获 bug（所有闭包看到的都是循环结束后的最后一个值）；
- 在 C++ 里，你得在 `[=, &x]` 捕获列表里小心翼翼地手写每个变量怎么抓，一旦 `[&]` 悬垂，程序直接原地爆炸；
- 在 Rust 里，编译器试图通过检查函数体“只是读了一下还是改了一下”来自动猜测是按 `&T`、`&mut T` 还是按值 `move` 捕获——结果有时候你只是在闭包里打印了一行 `println!("{}", config.id)`，借用检查器就抱怨整个 `config` 被长期借用，导致后续代码寸步难行。

在 Flurry 中，Lambda 的捕获遵循一条清晰、没有丝缕暗箱猜测的准则（详见 `docs/functions/lambdas.md`）：
> **“想保留创建时的数值，就捕获值（默认按值捕获）；想观察或修改以后发生的变化，就显式捕获指向相应对象的指针（`.ref`）或视图。”**

让我们从第一行 Lambda 语法开始，一步步拆解这套设计。

---

## 一、 认识 Flurry 的 Lambda 语法与“默认按值捕获”

在 Flurry 中，Lambda 表达式的语法非常简洁：
- 有参数且省略类型标注：`|value| value + 1`
- 显式参数类型与返回类型：`|value: u32| -> bool { value <= 10 }`
- 零参数 Lambda：`|| println("tick")`

> **🔍 语法小贴士：为什么 Lambda 参数列表里只允许写普通锚定参数？**
> 翻开 `docs/functions/lambdas.md` 的语法定义：
> `lambda -> |anchored_parameter*| (-> expr)? expr`
> `anchored_parameter -> id | id : expr`
> 注意：Lambda 的 `|...|` 参数列表里**只允许声明普通的锚定参数（Anchored Parameter）**，禁止于里面写默认参数 `.opt = 1`、变参 `...items` 或 `implicit` 参数。为什么？因为 Lambda 是创建运行期可调用值的轻量级表达式；如果你需要复杂的参数协议或默认参数，应该把它们定义在**接收该闭包的高阶函数接口**或具名 `fn` 上。

### 误区测试：以为直接写变量名会自动“按引用观察”

假设一个刚从动态语言或 Rust 转过来的程序员想写一个“实时读取当前传感器阈值 `current`”的零参闭包，他随手写下了初始写法：

```flurry
use std.prelude.*;

test {
    var offset: u32 = 10;
    -- 直接引用外部的 offset
    let add_offset = |value: u32| value + offset;

    -- 随后修改了外部的 offset
    offset = 20;

    -- 猜猜看：add_offset(1) 输出的是 21 还是 11？
    println("{} {}", add_offset(1), offset);
}
```

运行结果是：**`11 20`**。
为什么 `offset = 20` 没有影响 `add_offset(1)` 的结果？

因为 Flurry **默认按值捕获（Capture by Value）**。
- 当求值 `|value: u32| value + offset` 这条 Lambda 表达式的瞬间（注意：**是在创建闭包的时刻，而非调用闭包的时刻。**），编译器就把 `offset` 的值捕获进了闭包环境：
- 对于像 `u32` 这样具备 **`Copy` 能力**的类型，闭包直接复制了一份创建时的数值快照（`10`）；
- 对于**并非 `Copy` 的仿射类型（Once）**，闭包在创建时就直接把所有权**移动（Move）**进了自己的环境里。

### 正确打开方式：想观察后续修改？显式捕获 `.ref` 指针

那如果我们确实希望闭包在调用时读取 `current` 的最新值呢？
答案是：**切勿让编译器猜。在创建闭包前（或闭包内）显式取得指针 `current.ref`，捕获这个指针并在调用时解引用 `source.*`。**

```flurry
use std.prelude.*;

test {
    var current: u32 = 10;
    -- 显式取得指向 current 的指针 *u32
    let source = current.ref;

    -- 闭包按值捕获的是指针 source（复制指针表示，并保留对 current 的目标依赖。）
    let read_current = || source.*;

    -- 修改外部的 current
    current = 20;

    -- 调用时通过指针读取，输出最新的 20。
    println("{}", read_current());
}
```

下面对比“按值捕获快照”与“显式捕获指针观察”：

```text
  [1] 直接捕获 Copy 变量:  let add_offset = |v| v + offset;
      外部栈帧                       闭包环境 (创建时立刻复制!)
     ┌─────────────────┐            ┌─────────────────────────┐
     │ offset: 10 ──►20│            │ captured_offset: 10     │ (与外部脱钩!)
     └─────────────────┘            └─────────────────────────┘

  [2] 显式捕获指针 `.ref`: let source = current.ref; let read_current = || source.*;
      外部栈帧                       闭包环境 (保存指针与目标依赖!)
     ┌─────────────────┐            ┌─────────────────────────┐
     │ current: 10──►20│ ◄──────────┤ captured_source: *u32   │
     └─────────────────┘   读取 .*   └─────────────────────────┘
```

这种设计带来了无与伦比的清晰度：
你看一眼闭包代码里有没有 `.ref` / `.*`，就能 100% 确定它是拿了一份独立的快照/所有权，还是在共享观察外部的状态。

---

## 二、 创建时刻 vs 调用时刻——并非 `Copy` 资源与 `Clone` 的边界

让我们把“创建 Lambda 表达式”和“调用闭包”这两个时刻切开来看（详见 `docs/functions/lambdas.md` §创建与调用）：

| 时刻 | 发生的工作 | 尚未发生的工作 |
| :--- | :--- | :--- |
| **1. 求值 Lambda 表达式** | **取得捕获并建立环境**（`Copy` 值复制，并非 `Copy` 值立即移动。） | **跳过执行函数体**中的打印、循环或计算 |
| **2. 调用闭包 `action()`** | 绑定实参，按当前环境状态执行函数体 | 不重新去外部源变量那里补抓捕获 |
| **3. 环境退出作用域** | 清理仍然留在环境中的捕获责任（`Drop`） | 不会为了清理而先偷偷执行一次函数体 |

来看一个包含并非 `Copy` 独占硬件票据 `Ticket` 的例子：

```flurry
use std.prelude.*;

struct Ticket {
    number: u32,
}

fn finish(ticket: Ticket) {
    println("ticket {}", ticket.number);
}

test {
    let ticket = Ticket { number: 7u32 }

    -- 注意。就在这一行创建 `submit` 的瞬间，ticket 的所有权已经移入闭包环境。
    let submit = || finish(ticket);
    println("closure created, body not run yet");

    -- 直到这里调用时，才执行 finish(ticket)
    submit();
}
```

如果你在创建了 `submit` 之后（哪怕还没调用 `submit()`），试图在外部继续使用 `ticket`，编译器会编译期报错：

```flurry
-- ❌ 编译期直接报错：ticket 已在创建 submit 时移入闭包环境，外部不再持有它。
fn invalid_capture() {
    let ticket = Ticket { number: 8u32 }
    let submit = || finish(ticket);
    finish(ticket); -- ❌ 错误：使用已移出的值 ticket。
}
```

> **💡 为什么编译器不在捕获时自动调用 `.clone()`？**
> 也许你会问：“如果我给 `Ticket` 实现了 `Clone` trait，编译器能否在捕获时自动帮我克隆一份？”
> **行之难通。** 因为在 Flurry 中，`Clone` 是可能包含内存分配、日志输出甚至耗时 I/O 的显式操作。把 `Clone` 的副作用悄悄藏在闭包的隐式捕获规则里，会破坏系统程序的性能可预测性。如果你需要克隆一份给闭包、留一份给自己，**必须在创建闭包前显式调用 `let captured = original.clone();`**。

---

## 三、 最小独立捕获位置——只搬走一个字段，不株连九族

现在来看一个在实际工程中常见、也是许多早期编译器容易犯傻的场景：
假设我们有一个结构体 `Pair { left: Ticket, right: Ticket }`，里面装着左、右两张互无扰于的独占票据。
现在我们创建一个闭包 `let submit_left = || finish(pair.left);`，闭包里**只用到了 `pair.left`**。

如果编译器太笨，看到闭包里出现了 `pair`，就把**整个 `pair`** 都移动进闭包里锁死，那外部紧接着想用 `finish(pair.right)` 就会无辜躺枪报错。

在 Flurry 中，编译器执行的是**最小捕获位置分析（Minimal Capture Location Analysis）**：
对于能证明可独立交付的内建字段投影（如 `pair.left`），编译器**只捕获所需的最小位置**。

```flurry
use std.prelude.*;

struct Ticket {
    number: u32,
}

struct Pair {
    left: Ticket,
    right: Ticket,
}

fn finish(ticket: Ticket) {
    println("ticket {}", ticket.number);
}

test {
    let pair = Pair {
        left: Ticket { number: 1u32 },
        right: Ticket { number: 2u32 },
    }

    -- 闭包只捕获并移走了最小独立位置 `pair.left`。
    let submit_left = || finish(pair.left);

    -- 此时 `pair.right` 在外部依然 100% 完好可用。
    finish(pair.right);

    -- 调用闭包，消费掉刚才捕获的 `pair.left`
    submit_left();
}
```

下面来看看编译器是如何用“投影树（Projection Tree）”解剖捕获需求的：

```text
  外部对象 pair:
  ┌─────────────────────────┬─────────────────────────┐
  │ pair.left (Ticket #1)   │ pair.right (Ticket #2)  │
  └────────────┬────────────┴────────────┬────────────┘
               │                         │
   [创建 submit_left = || finish(pair.left)]
               │                         │
               ▼ (只移入闭包环境!)        ▼ (留在外部原作用域!)
     ┌───────────────────┐       finish(pair.right) ✅ 直接合法使用。
     │ submit_left 环境:  │
     │  pair.left (#1)   │
     └───────────────────┘

  ⚠️ 但如果闭包内部既写了 `pair.left`，又写了整个 `pair` 呢？
     ──► 编译器检测到“父子路径重叠需求”，对于并非 Copy 仿射类型，
         禁止把同一份 `pair.left` 责任拆成两份，直接报错拦截。
```

```flurry
fn consume_pair(p: Pair) {}

-- ❌ 编译期直接报错：父子路径重叠消费：无法既单独移走 pair.left，又移走整个 pair。
fn bad_overlapping_capture(pair: Pair) {
    let broken = || {
        finish(pair.left);
        consume_pair(pair);
    }
}
```

---

## 本篇小结

| 捕获维度 | Flurry 的设计法则 | 解决的核心问题 |
| :--- | :--- | :--- |
| **参数列表语法** | `\|x: T\| -> R { ... }`（仅限锚定参数） | 语法轻量无歧义；无环境时可直接接入普通 `fn(T) -> R` |
| **默认按值捕获 vs 指针观察** | 默认按值（`Copy` 复制、并非 `Copy` 在创建时移动）；观察修改显式传 `.ref` | 避免隐式引用捕获带来的悬垂与生命周期猜测；读写意图一目了然 |
| **免作隐式 `Clone`** | 不在捕获时自动调用 `.clone()` | 保证捕获动作不会隐藏昂贵的分配或副作用 |
| **最小独立位置捕获** | 对可独立交付的内建字段路径（如 `pair.left`），只捕获该字段 | 捕获 `pair.left` 后，外部依然可以自由使用 `pair.right` |

---

### 下一篇预告

在这一篇里，我们已经知道闭包在创建时会建立一个**捕获环境（Closure Environment）**。
但是，仔细想想这三个问题：
1. 在 Rust 中，闭包被硬生生拆成了三个 Trait：`Fn`、`FnMut`、`FnOnce`。而在 Flurry 中，**一个闭包到底能否调用第二次、调用之后它的环境变成了什么状态**，是怎么在类型系统里表达的？为什么 Flurry 里有一种神秘的前缀类型叫 **`%c fn(...)`**（比如 `%{ offset: u32 = offset } fn(u32) -> u32`）？
2. 如果一个闭包按顺序捕获了 `second` 和 `first` 两个带 `Drop` 的资源，结果**一次都没被调用**就离开了作用域，它们俩谁先被 `Drop`？如果调用了一次，消费掉了 `first`，留下的 `second` 又该怎么清理？
3. 如果有人在 `do { let local = 7u32; let p = local.ref; || p.* }` 块里把捕获了局部变量指针的闭包返回到块外面，编译器是怎么抓住这个逃逸悬垂指针的？

在下一篇 **《Flurry 演进手记 (031)：闭包环境的生死簿——%c fn 闭包限定类型、捕获状态更新与逆序清理》** 中，我们将掀开 `%c` 闭包环境限定的底牌。
