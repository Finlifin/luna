# Flurry 演进手记 (026)：上下文自动注入与尾随块——implicit 参数、post do 与 tactic lambda

在 [前几篇 (022–025)](./025-extended-applications-and-property-collection.md) 中，我们已经解锁了可选参数、位置变参与属性收集。今天，我们要完成函数参数拼图的最后、也最惊艳的一块：**参数 Elaboration（`lambda` 标记、`post do`、`tactic lambda` 与 `implicit` 参数）**。

来看看我们的传感器分析仪在编写高层业务规则时遇到的三个“手感”痛点：

1. **痛点一（简单回调的命名噪音）**：我们写了一个通用的双样本比较器 `compare_pair(a, b, compare)`。每次调用它只为了做个简单的小于或相等比较，却不得不反复写 `compare_pair(2, 3, |first, second| first < second)`。为了一个 `<` 号，居然要专门起两个形参名 `first` 和 `second`！
2. **痛点二（多行回调的 `})` 小尾巴）**：当回调函数是一段多行的阈值折算或事务块时，把它塞在圆括号里 `inspect_with(|val| { ... }, 7)` 不仅读起来头重脚轻，末尾还会留下一串丑陋的 `})` 小尾巴。
3. **痛点三（上下文参数的显式透传与覆盖）**：校准算子 `shifted` 和编译期通道选择器 `choose_mode` 需要接收上下文输入（如校准偏移 `bias: u32` 或编译期开关 `use_first: bool`）。它们与固定默认值的 `.id = expr` 不同——我们希望将其声明为由协议供应的 `implicit` 参数，同时在需要定制或跨层转发时，依然能用 `.bias = 3` 精确接管！

让我们从最传统的闭包传参 `v0` 开始，分三步把这些接口打磨成如同语言内置语法般丝滑的形态。

---

## 第一版：传统回调传参的视觉噪音与括号嵌套（v0）

在没有 `post do` 和 `tactic lambda` 之前，无论是只有三个字符的算术表达式，还是长达十行的业务分支，我们都只能老老实实地用竖线语法 `|p1, p2| body` 把显式 lambda 塞进函数调用的圆括号里：

```flurry
use std.prelude.*;

-- v0：普通的函数值参数，没有任何参数 elaboration 标记
fn compare_pair_v0(first: u32, second: u32, compare: fn(u32, u32) -> bool) -> bool {
    compare(first, second)
}

fn apply_v0(value: u32, action: fn(u32) -> u32) -> u32 {
    action(value)
}

test {
    -- 痛点 1：明明只是比个大小，却要手写 `|first, second|`
    let ordered = compare_pair_v0(2u32, 3u32, |first, second| first < second);

    -- 痛点 2：当多行回调塞在圆括号里时，结尾的 `});` 既别扭又容易数错括号！
    let adjusted = apply_v0(10u32, |val| {
        if val > 5u32 {
            val * 2u32
        } else {
            val
        }
    });

    println("{} {}", ordered, adjusted);
}
```

仔细看第 17 到 23 行的 `apply_v0(10u32, |val| { ... });`：
- 因为最外层是函数调用的右圆括号 `)` 而不是 `}`，所以末尾还必须带上分号 `});`；
- 更要命的是，如果回调参数不在最后一个位置（比如 `inspect_with(inspect_fn, value, .fallback = false)`），把一大坨 `{ ... }` 夹在圆括号正中间，后面的实参 `7` 和 `.fallback = false` 就会被彻底淹没！

我们能不能先把这坨多行闭包从圆括号里“解放”出来，放到函数调用的尾巴后面？

---

## 第二版：后缀 `do`（`post do`）与 `lambda` 参数路由（v1）

在 Flurry 中，任何调用表达式后面都可以直接跟一个 **`do`** 尾随语法（在语法表中称为 `post_lambda`）：

```text
  post_lambda -> expr do (lambda | block | expr)
```

当你写下 **`expr(args...) do |x| { ... }`** 时，编译器是怎么知道 `do` 后面的这个闭包应该塞给函数签名里的哪一个参数呢？

让我们画一张 **Code Cartoon**，看懂 Flurry 极其聪明又绝不含糊的 **`post do` 路由规则**（详见 `docs/functions/parameter-elaboration.md`）：

```text
  调用点：func(arg1, arg2) do |x| { ... }
                                  │
                                  ▼
           ┌─────────────────────────────────────────────┐
           │ 编译器扫描 func 的粗 HIR 签名参数表：       │
           └──────────────────────┬──────────────────────┘
                                  │
         ┌────────────────────────┴────────────────────────┐
         ▼ (情况 A：签名中有 `lambda` 标记的参数！)         ▼ (情况 B：签名中没有 `lambda` 标记)
  优先精确路由给带 `lambda` 标记的那个参数！        自动路由给签名中的【最后一个锚定参数】！
  (哪怕它排在第 1 个参数位置，后面还有普通参数       (例如 `fn apply_plain(v: u32, action: fn(u32)->u32)`
   或 `.fallback` 可选参数，也能精准空投过去！)      `do` 会直接接入末尾的 `action`！)
```

这条规则妙在哪里？
1. **默认零侵入**：就算一个普通高阶函数 `apply_plain(value: u32, action: fn(u32) -> u32)` 的作者忘了写 `lambda` 标记，只要回调 `action` 是最后一个锚定参数，调用者依然可以直接写 `apply_plain(5) do |v| v + 2`！
2. **标记解耦位置**：当函数作者在形参前加上 **`lambda`** 标记（如 `fn inspect_with(lambda inspect: fn(u32) -> bool, value: u32, .fallback: bool = false)`）时，`inspect` 就算放在**第 1 个参数位置**，调用者依然可以把 `value` 和 `.fallback` 干干净净地留在圆括号里，而把回调写在最后的 `do` 后面：
   `inspect_with(0, .fallback = true) do |value| value < 10`！
3. **花括号自然收尾**：一旦你用了 `do |v| { ... }`，整条语句的最后一个 token 就变成了右花括号 `}`——根据 Flurry 的分号省略规则，末尾连 `;` 都不用写了，长相跟原生 `if` / `while` 控制流一模一样！

让我们用 `post do` 写出清爽的 `v1` 版本：

```flurry
use std.prelude.*;

-- 情况 B：没有 lambda 标记，post do 自动接入最后一个锚定参数 action
fn apply_plain(value: u32, action: fn(u32) -> u32) -> u32 {
    action(value)
}

-- 情况 A：带 lambda 标记的 inspect 位于第 1 个参数！
-- 后面还跟着锚定参数 value 和可选参数 .fallback
fn inspect_with(
    lambda inspect: fn(u32) -> bool,
    value: u32,
    .fallback: bool = false,
) -> bool {
    if value == 0u32 {
        fallback
    } else {
        inspect(value)
    }
}

test {
    -- 1. 既可以用圆括号传显式 lambda，也可以用尾随 post do！
    let first = apply_plain(5u32, |value| value + 1u32);
    let second = apply_plain(5u32) do |value| {
        value + 2u32
    }
    println("{} {}", first, second);

    -- 2. 哪怕 lambda 参数排在第一位，post do 也能精准路由给 inspect，
    --    而位置实参 7u32 / 0u32 自动顺延绑定给后面的 value！
    let accepted = inspect_with(7u32) do |value| value < 10u32;
    let absent = inspect_with(0u32, .fallback = true) do |value| value < 10u32;
    println("{} {}", accepted, absent);
}
```

运行输出：

```text
6 7
true true
```

注意看 `inspect_with(0u32, .fallback = true) do |value| value < 10u32`：
在调用准备阶段，闭包被构造出来交给了 `inspect`；但由于 `value == 0u32`，函数体直接走 `if` 分支返回了 `fallback`（`true`），根本没有调用 `inspect`！这再次印证了我们在本章开头强调的原则：**构造闭包环境发生在实参准备期间，而执行闭包主体发生在被调函数真正调用它的时候，两者是完全独立的两个时刻！**

---

## 第三版：下标占位符 `tactic lambda`（`_0`, `_1`）与 `implicit` 参数（v2）

`post do` 解决了多行回调的小尾巴问题。但对于像 `compare_pair(2, 3, |a, b| a < b)` 这样只有几个字符的极简回调，写 `do |a, b| a < b` 还是嫌长。

这时候，刚才我们在形参上加的那个 **`lambda` 标记**，就要展现它的第二项超能力了——**`tactic lambda`（下标占位符简写闭包）**！

### 1. 用 `_0`、`_1` 瞬间生成闭包：只认目标签名，不重复求值外部实参！

当一个形参带有 **`lambda`** 标记（如 `lambda compare: fn(u32, u32) -> bool`）时，编译器不仅允许 `post do` 路由到它，还为该实参位置建立了一个**占位符作用域（Placeholder Scope）**！
在这个位置上，你可以省掉 `|a, b|`，直接写包含 **`_0`**、**`_1`**（从 `0` 开始编号）的表达式，编译器会自动按目标签名的完整参数表把它包装成闭包！

让我们画一张 **Code Cartoon**，拆解 `tactic lambda` 的三大黄金法则：

```text
  函数签名：fn compare_pair(first: u32, second: u32, lambda compare: fn(u32, u32) -> bool) -> bool

  法则 1：【完整参数表由目标签名决定，不由最大下标决定！】
     调用：compare_pair(2, 3, _0 == _0)
     虽然表达式里只出现了 `_0`，没出现 `_1`，但它生成的依然是二元闭包：
     `|p0: u32, p1: u32| -> bool { p0 == p0 }` （p1 只是未被使用，绝不会缩成一元函数！）

  法则 2：【重复写 `_0` 只是多次读取闭包形参，绝不重复计算外部实参！】
     调用：apply(produce(), _0 + _0)
     [第 1 步] 实参准备：`produce()` 只执行 1 次！假设打印 "produce" 并返回 6。
     [第 2 步] 进入 apply(6, |p0| p0 + p0)：读取已取得的 p0 (6) 两次，算出 6 + 6 = 12！
     (绝不是宏替换成 `produce() + produce()`！)

  法则 3：【嵌套的 `lambda` 标记建立各自的独立边界！】
     调用：apply(3, apply(_0, _0 + _0))
                    │     ▲   └───┬───┘
                    │     │       └── 内层 apply 的第 2 个实参（带 lambda 标记）：
                    │     │           建立新边界！这里的 `_0 + _0` 指的是【内层闭包】的参数！
                    └─────┴────────── 内层 apply 的第 1 个实参（普通位置）：
                                      继承外层边界！这里的 `_0` 指的是【外层闭包】的参数 (3)！
     等价于：`apply(3, |outer| apply(outer, |inner| inner + inner))` ──► 输出 6！
```

### 2. 编译器的两道 `tactic lambda` 防呆墙（`-- ❌ 编译期直接拒绝！`）

为了防止占位符被滥用导致类型含糊，编译器严格把守着两条红线：

- **红线 A：没有 `lambda` 标记的参数，严禁使用 `_0` 占位符！**
  虽然上一节的 `apply_plain(value: u32, action: fn(u32) -> u32)` 可以用 `post do`，但因为它的 `action` 参数没有写 `lambda` 标记，它就没有开启 `tactic lambda` 边界：

```flurry
fn apply_plain(value: u32, action: fn(u32) -> u32) -> u32 {
    action(value)
}

-- ❌ 编译期直接拒绝！
-- 错误：apply_plain 的参数 action 没有声明 `lambda` 标记，不能独立启用 `_0` tactic lambda！
fn invalid_tactic() -> u32 {
    apply_plain(5u32, _0 + 1u32)
}
```

- **红线 B：占位符下标越界，当场拒绝！**
  `compare` 的类型是 `fn(u32, u32) -> bool`，只有两个参数（合法下标是 `_0` 和 `_1`）。如果你手滑写出了 `_2`：

```flurry
fn compare_pair(first: u32, second: u32, lambda compare: fn(u32, u32) -> bool) -> bool {
    compare(first, second)
}

-- ❌ 编译期直接拒绝！
-- 错误：目标签名 `fn(u32, u32) -> bool` 只有 2 个参数（下标 0..=1），`_2` 越界！
fn invalid_index() -> bool {
    compare_pair(2u32, 3u32, _2 == 0u32)
}
```

### 3. `implicit` 参数与 `.name = expr` 显式覆盖及转发

最后，来看看参数家族的第五位成员：**`implicit` 参数**（如 `implicit bias: u32` 或 `implicit comptime use_first: bool`）。

在 Flurry 中，`implicit` 参数代表一个“在调用处省略时，应当由编译期 elaboration 协议自动供应”的上下文输入。
> **💡 核心心智模型：`implicit` 不是“在调用处词法作用域里随便抓一个同类型局部变量”！**
> 有些语言看到缺少隐式参数，就会去调用者的局部变量表里乱找一个碰巧也是 `u32` 或 `bool` 的变量塞进去——这会导致重命名一个局部变量就悄悄改变了函数行为！
> Flurry **严禁这种退化**！`implicit` 参数要么由明确的 trait 协议供应，要么由调用者通过我们熟悉的 **`.name = expr` 显式按名提供**（直接供应该位置，绕过缺省供应协议）。而且，当外层函数收到一个 `implicit` 参数并想把它传给内层函数时，也可以直接用 `.use_first = use_first` 显式转发！

让我们把 `tactic lambda`、嵌套占位符和 `implicit` 参数组合在一起，写出完整强悍的 `v2` 版本：

```flurry
use std.prelude.*;

fn compare_pair(first: u32, second: u32, lambda compare: fn(u32, u32) -> bool) -> bool {
    compare(first, second)
}

fn produce() -> u32 {
    println("produce called once!");
    6u32
}

fn apply(value: u32, lambda action: fn(u32) -> u32) -> u32 {
    action(value)
}

-- 运行时 implicit 参数
fn shifted(value: u32, implicit bias: u32) -> u32 {
    value + bias
}

-- 编译期 implicit comptime 参数与显式转发
fn choose_mode(first: u32, second: u32, implicit comptime use_first: bool) -> u32 {
    inline if use_first {
        first
    } else {
        second
    }
}

fn selected(first: u32, second: u32, implicit comptime use_first: bool) -> u32 {
    -- 将已取得的 implicit 输入显式转发给下层，不再次触发缺省协议
    choose_mode(first, second, .use_first = use_first)
}

test {
    -- 1. 一个 `lambda` 标记，同时支持 `_0 < _1`、只用部分参数的 `_0 == _0` 以及 `post do`！
    let ordered = compare_pair(2u32, 3u32, _0 < _1);
    let same_first = compare_pair(2u32, 3u32, _0 == _0);
    let explicit = compare_pair(2u32, 3u32) do |first, second| first < second;
    println("compare: {} {} {}", ordered, same_first, explicit);

    -- 2. 验证重复占位符 `_0 + _0` 只求值一次外部实参 `produce()`
    println("doubled: {}", apply(produce(), _0 + _0));

    -- 3. 验证嵌套 tactic lambda 各自拥有独立的占位符边界
    let nested = apply(3u32, apply(_0, _0 + _0));
    println("nested: {}", nested);

    -- 4. 验证显式供应与转发 implicit 参数
    println("shifted: {}", shifted(5u32, .bias = 3u32));
    println("selected: {}", selected(4u32, 9u32, .use_first = false));
}
```

运行输出：

```text
compare: true true true
produce called once!
doubled: 12
nested: 6
shifted: 8
selected: 9
```

---

## 本篇小结

至此，我们已经走完了 Flurry 函数参数设计的全景长廊（021–026）：

| 特性 / 标记 | 语法形式 | 路由与行为契约 |
| :--- | :--- | :--- |
| **`post do` 尾随调用** | `expr(args) do \|x\| { ... }` | 优先接入带 `lambda` 标记的形参；若无标记，则接入**最后一个锚定参数** |
| **`lambda` 形参标记** | `fn foo(lambda cb: fn(u32) -> bool)` | 既作为 `post do` 的优先路由信标（可位于任意参数位置），又开启 `tactic lambda` 占位符作用域 |
| **`tactic lambda`** | `compare_pair(2, 3, _0 < _1)` | `_0`、`_1` 按目标签名完整参数表绑定；重复 `_0` 不重复求值外部实参；嵌套 `lambda` 建立独立新边界 |
| **`implicit` 参数** | `fn foo(v: u32, implicit bias: u32)` | 缺省时由 elaboration 协议供应（绝不乱抓同名局部变量）；支持 `.bias = 3` 显式按名覆盖与转发 |

### 留给下一篇的悬念

在这前六篇里，不管参数怎么传进函数——位置传参、可选默认、元组收集、匿名结构体还是 `post do`——一旦进了函数大门，如果你要根据参数的**不同枚举变体或不同数值形态**执行不同的逻辑，你还是得在函数体第一行写一个 `param match { ... }`。

但是，当我们开始编写传感器分析仪的**指令分派器（Command Dispatcher）**时，有些函数天生就是“按输入形态分片定义的”！
比如：
- 当收到 `.reset` 时直接归零；
- 当收到 `.calibrate(offset)` 且第二个参数是 `true` 时执行校准；
- 甚至不同的处理分支由不同的逻辑块给出！

能不能干脆连函数体最外层的 `match` 都省掉，**直接在函数的参数列表里写模式 `case pattern`**，甚至把同一个函数按不同的模式拆成多段定义？！

在下一篇 **《Flurry 演进手记 (027)：直接按数据形状分派函数——`case` 参数与多输入模式函数》** 中，我们将解锁函数级模式匹配与分散定义的魔法！
