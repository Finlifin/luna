# Flurry 演进手记 (022)：让配置接口告别重载膨胀——可选参数 .id = expr 与前序参数依赖

在 [上一篇 (021)](./021-call-evaluation-order-and-failure-boundaries.md) 中，我们确立了函数调用准备阶段的核心规则：先求值被调用方或方法接收者，再严格按源码从左到右求值显式实参。

今天，我们要为传感器分析仪的历史样本缓冲区设计一个分页切片查询函数 `page_bounds`。
假设缓冲区里一共存了 `total` 条传感器样本记录，我们需要根据起始下标 `start` 和读取条数 `count`，算出本次查询的半开区间 `(start, start + count)`（类型为 `Tuple<u32, u32>`）。

但下游业务团队在调用这个函数时，有三种大相径庭的习惯：
1. **全量导出**：只传一个总数 `page_bounds(100)`，希望默认从下标 `0` 开始，读取全部 `100` 条记录（返回 `0..100`）；
2. **尾部追加读取**：传入总数和起点 `page_bounds(100, .start = 80)`，希望**默认读取从 `start` 到末尾的所有剩余记录**，即 `count` 自动等于 `total - start`（`100 - 80 = 20`，返回 `80..100`）；
3. **精确分页**：同时指定起点和条数，甚至有时候顺手把条数写在了起点前面：`page_bounds(100, .count = 5, .start = 20)`（返回 `20..25`）。

如果你在 Rust、C++ 或 Java 里写过类似的分页或切片接口，看到第 2 条需求（“默认条数等于 `total - start`”）时，你大概已经开始头疼了。

---

## 一、 传统语言里的“重载膨胀”与 `?u32` 样板代码

在没有可选参数的语言（如 Rust 或 Zig）里，或者在虽然有默认参数但**禁止默认表达式读取前序参数**的语言（如 C++，C++ 标准明确规定默认实参中无法求值同一函数的其他参数。）里，你通常只有两条路可走：

- **路子 A（重载膨胀）**：写三个名字不同（或参数个数不同）的函数：`page_bounds_all(total)`、`page_bounds_from(total, start)`、`page_bounds_slice(total, start, count)`。一旦可选配置增加到 4 个，组合爆炸能让你写出 $2^4 = 16$ 个转发函数。
- **路子 B（传 `null` 进函数体手算）**：把可省略的参数全改成可选类型 `?u32`，强迫调用者传一堆毫无可读性的 `null`，然后再在函数体开头写一串解包和补默认值的样板代码。

让我们看看采用可选参数方案时的基础写法：

```flurry
use std.prelude.*;

-- 用 ?u32 模拟可选参数，调用处被迫传 null，函数体里手动补默认值
fn page_bounds(total: u32, maybe_start: ?u32, maybe_count: ?u32) -> Tuple<u32, u32> {
    let start = if maybe_start is s? do { s } else { 0u32 }
    -- 因为必须在 start 算出来之后才能算 total - start，只能在函数体里手写判断
    let count = if maybe_count is c? do { c } else { total - start }
    (start, start + count)
}

test {
    -- 调用处充满了令人迷惑的 `null`："第二个 null 到底代表 start 还是 count？"
    let (s1, e1) = page_bounds(100u32, null, null);
    let (s2, e2) = page_bounds(100u32, 80u32, null);
    -- 如果只想指定 count = 5、让 start 保持默认 0，还得被迫在中间塞一个 null。
    let (s3, e3) = page_bounds(100u32, null, 5u32);

    println("{}..{}, {}..{}, {}..{}", s1, e1, s2, e2, s3, e3);
}
```

看看 `page_bounds(100u32, null, 5u32)` 这一行，存在以下几个明显问题：
1. **调用处语义不自明**：读者看到一排 `null, 5u32`，如果不跳回函数定义去数参数位置，根本猜不出 `5u32` 到底是 `start` 还是 `count`。
2. **契约无法写在函数签名上**：如果我们想在函数签名上加一句前置条件契约 `where requires start <= total and count <= total - start`，在 传统里根本无从写出。因为参数名还是 `maybe_start: ?u32`，真正的 `start` 和 `count` 藏在函数体肚子里。
3. **定义处与函数体职责混淆**：一个参数的默认值本来是**接口契约的一部分**，现在却被迫塞进了函数体的实现代码里。

我们能否直接在函数签名上声明可选参数，而且让后面的默认表达式直接引用前面的参数？

---

## 二、 可选参数 `.id: Type = expr` 与前序参数依赖

在 Flurry 中，参数分为不同的类别（详见 `docs/functions/parameters.md`）：
- **锚定参数（Anchored Parameter）**：普通的 `total: u32`，调用时必须按位置提供实参；
- **可选参数（Optional Parameter）**：在参数名前加一个点号 `.`，并给出默认表达式，写作 **`.start: u32 = 0`**。

更进一步， Flurry 的**前序依赖规则（Forward Dependency）**：
> **默认表达式在定义作用域中解析，可以直接依赖前面已经声明的参数。**

也就是说，因为 `.start` 声明在 `total` 后面，而 `.count` 声明在 `total` 和 `.start` 后面，所以 `.count` 的默认表达式可以直接写 **`= total - start`**。

非但如此，无论调用方省略了 `start`（使用默认值 `0`），还是显式传入了 `.start = 80`，后面的 `.count` 默认表达式拿到的 `start`，**永远是本次调用中 `start` 最终实际绑定的那个值**。

让我们直接写出流畅自然的 改进版本：

```flurry
use std.prelude.*;

-- 可选参数 `.id: T = default_expr`，且 `.count` 直接依赖前序参数 `total` 与 `start`。
fn page_bounds(
    total: u32,
    .start: u32 = 0u32,
    .count: u32 = total - start,
) -> Tuple<u32, u32>
where requires start <= total and count <= total - start
{
    (start, start + count)
}

test {
    -- 1. 两个可选参数全省略：start 取 0，count 取 100 - 0 = 100
    let (first_start, first_end) = page_bounds(100u32);

    -- 2. 显式供应 .start = 80：跳过 start 的默认值，count 自动用 100 - 80 算出 20。
    let (tail_start, tail_end) = page_bounds(100u32, .start = 80u32);

    -- 3. 乱序显式供应：先写 .count = 5，再写 .start = 20。
    let (part_start, part_end) = page_bounds(100u32, .count = 5u32, .start = 20u32);

    println("{}..{}", first_start, first_end);
    println("{}..{}", tail_start, tail_end);
    println("{}..{}", part_start, part_end);
}
```

运行结果干净利落：

```text
0..100
80..100
20..25
```

> **🔍 语法小贴士：调用处的 `.id = expr` 与普通锚定参数**
> 在调用处，我们用 **`.start = 80u32`** 这种“点号 + 参数名 + 等号”的语法（`named_arg`）来显式供应可选参数。
> 但要注意一条边界：**普通锚定参数（如没有点号前缀的 `total: u32`）免于自动变成具名可选参数。** 只有声明为 optional（`.id: T = ...`）、implicit、变参或拓展变参的入口，才接受 `.name = expr` 形式的显式命名供应。

---

## 三、 当“乱序显式传参”遇上“前序依赖默认值”——求值次序全解

看到上面 `test` 里的第 3 种调用 `page_bounds(100u32, .count = 5u32, .start = 20u32)` 时，爱思考的你一定冒出了三个深入的问题：

1. 调用处把 `.count` 写在了 `.start` 前面，那 `.count` 的实参表达式和 `.start` 的实参表达式，**到底谁先求值**？
2. 当我显式传了 `.count = 5u32` 时，签名里的默认表达式 `total - start` **还是否会在背后偷偷算一次，然后再被 `5u32` 覆盖掉**？
3. 如果我有三个连续依赖的可选参数 `.first` -> `.second` -> `.third`，我只显式覆盖了中间的 `.second`，执行顺序又是怎样的？

下面把 [第 021 篇](./021-call-evaluation-order-and-failure-boundaries.md) 的“调用准备时间线”放大，看看编译器是怎么编排显式实参和缺省计算的：

```text
  调用点：choose(true, .third = mark(3), .first = mark(1))
  函数签名：fn choose(seed: bool, .first: u32 = 10, .second: u32 = first + 1, .third: u32 = second + 1)

  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 阶段 A：按【调用处源码从左到右顺序】求值所有显式实参（各一次  ）        │
  ├─────────────────────────────────────────────────────────────────────────┤
  │  1. 求值位置实参 `true`       ──► 绑定到形参槽 `seed = true`            │
  │  2. 求值 `.third = mark(3)`   ──► 打印 "mark 3"，绑定到 `third = 3`     │
  │  3. 求值 `.first = mark(1)`   ──► 打印 "mark 1"，绑定到 `first = 1`     │
  └───────────────────────────────────┬─────────────────────────────────────┘
                                      │ (显式实参全部就绪。)
                                      ▼
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 阶段 B：按【函数签名形参声明依赖顺序】补算未提供的缺省值                │
  ├─────────────────────────────────────────────────────────────────────────┤
  │  4. 检查 `.first`  ──► [已由显式实参提供 (1)] ──► 跳过默认表达式 `10`   │
  │  5. 检查 `.second` ──► [未提供  ] ──► 执行默认表达式 `first + 1`        │
  │                        (读取刚才绑好的 first = 1，算出 second = 2)      │
  │  6. 检查 `.third`  ──► [已由显式实参提供 (3)] ──► 跳过默认表达式        │
  └───────────────────────────────────┬─────────────────────────────────────┘
                                      │
                                      ▼
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 阶段 C：检查调用前提 (requires)，正式进入 choose 函数主体               │
  └─────────────────────────────────────────────────────────────────────────┘
```

仔细品味这张图里的两条核心原则：

1. **“名字决定去哪，源码顺序决定先算谁”**：
   在阶段 A 中，哪怕 `.third = mark(3)` 对应的是最后一个形参，只要你在调用处把它写在了 `.first = mark(1)` 左边，运行时就**一定先执行 `mark(3)`，再执行 `mark(1)`**。
2. **“直接供应与缺省供应完全互斥”**：
   在阶段 B 中，只要某个可选参数在阶段 A 已经收到了显式实参，它的默认表达式就**直接被旁路跳过，跳过执行**。未存“先机械地算一遍默认表达式，再用显式实参覆盖它”的浪费或隐藏行为。而后面真正缺省的参数（如 `.second`），则会直接读取前面参数已经就位的实际值（哪怕那个值是被显式覆盖的）。

让我们写一段带轨迹打印的 进阶代码，亲眼见证这个过程：

```flurry
use std.prelude.*;

fn opposite(value: bool) -> bool {
    println("  -> computing opposite({})", value);
    not value
}

fn mark(label: str, value: bool) -> bool {
    println("  -> explicit arg {}: {}", label, value);
    value
}

fn cascade_flags(
    first: bool,
    .second: bool = opposite(first),
    .third: bool = opposite(second),
) -> Tuple<bool, bool> {
    (second, third)
}

test {
    println("=== Call 1: Override .second explicitly ===");
    -- 显式提供了 .second = true：
    -- 1. second 的默认表达式 opposite(first) 被直接跳过。
    -- 2. third 未提供，按依赖顺序读取显式绑定的 second (true)，调用一次 opposite(true) 得到 false。
    let (s1, t1) = cascade_flags(true, .second = mark("second", true));
    println("Result 1: second={}, third={}", s1, t1);

    println("=== Call 2: Out-of-order explicit args ===");
    -- 先写 .third，再写 .second：
    -- 按源码顺序先算 third 的实参，再算 second 的实参；两个默认表达式全跳过，opposite 零次调用。
    let (s2, t2) = cascade_flags(
        true,
        .third = mark("third", false),
        .second = mark("second", true),
    );
    println("Result 2: second={}, third={}", s2, t2);
}
```

运行输出如下：

```text
=== Call 1: Override .second explicitly ===
  -> explicit arg second: true
  -> computing opposite(true)
Result 1: second=true, third=false
=== Call 2: Out-of-order explicit args ===
  -> explicit arg third: false
  -> explicit arg second: true
Result 2: second=true, third=false
```

看。在 `Call 1` 中，`opposite` 只运行了**一次**（为了算 `third`），而且它读到的 `second` 正是我们显式传入的 `true`；在 `Call 2` 中，`mark("third", ...)` 先于 `mark("second", ...)` 执行，且 `opposite` **一次都没有运行**。

---

## 编译器的三道防呆铁闸（`-- ❌ 编译期直接报错`）

为了让可选参数既灵活又完全可靠，Flurry 编译器在定义端和调用端设立了三道严格的静态防线，任何企图钻空子的写法都会在编译期被当场拦下：

### 防线 1：严禁默认表达式“向后引用”后序参数

前面的参数还没算出来，怎么可能让前面的默认值依赖后面的参数呢？如果在调用处支持乱序写具名实参，那定义处允许逆向依赖岂不是会造出死锁环路？因此，默认表达式**只能引用声明在它前面的参数**：

```flurry
-- ❌ 编译期直接报错。
-- 错误：`.first` 的默认表达式引用了尚未声明的后序参数 `second`。
fn invalid_backward_dep(.first: u32 = second, .second: u32 = 1u32) -> u32 {
    first
}
```

### 防线 2：严禁在同一次调用中重复提供同一个参数

在有些动态语言或配置格式里，写两次同名键 `.limit = 3, .limit = 7` 会静默地“用后一个覆盖前一个”。在系统编程中，这几乎 100% 是复制粘贴导致的严重 Bug。Flurry 严禁对同一参数位置重复供应：

```flurry
fn select_limit(requested: u32, .limit: u32 = requested) -> u32 {
    limit
}

-- ❌ 编译期直接报错。
-- 错误：同一个可选参数 `.limit` 被供应了两次，编译器不静默覆盖。
fn bug_repeated_arg() -> u32 {
    select_limit(12u32, .limit = 3u32, .limit = 7u32)
}
```

### 防线 3：严禁拼写错误的未知参数名

如果你手滑把 `.limit` 打成了 `.limti`，编译器免将它忽略掉或偷偷创建一个新绑定，而是立即报出未知参数名诊断：

```flurry
-- ❌ 编译期直接报错。
-- 错误：`select_limit` 的签名中未存名为 `limti` 的参数。
fn bug_misspelled_arg() -> u32 {
    select_limit(12u32, .limti = 3u32)
}
```

> **💡 核心心智模型：“定义处解析”有别于“定义时只算一次”。**
> 用过 Python 的同学一定踩过 `def foo(items=[]):` 的惊天巨坑——Python 的默认参数在函数定义加载时只求值一次，所有调用共享同一个可变列表。
> 在 Flurry 中，**“默认表达式在定义作用域中解析”指的是它的名字查找（Lexical Scoping）归属定义处**（不会去调用者的局部变量里乱抓同名变量）；但它的**运行时求值，是在每次调用缺少该参数时的调用准备阶段（阶段 B）独立执行的**。

---

## 本篇小结

通过上述改进，我们告别了繁琐的重载函数与满屏的 `null`：

| 维度 | 传统做法 (`初版`) | Flurry 可选参数 (`改进版` / `进阶版`) |
| :--- | :--- | :--- |
| **声明语法** | 拆成多个重载函数，或用 `maybe_x: ?T` | `.name: T = default_expr` 直接写在签名中 |
| **参数间依赖** | C++ 禁止默认实参读取前序参数；必须在函数体内手算 | **原生支持前序依赖**（如 `.count: u32 = total - start`），且自动读取前序参数的最终实际绑定值 |
| **调用处写法** | 必须按固定位置传 `null` 占位 | 省略即走默认；显式传参写 `.name = expr`，支持任意书写顺序 |
| **求值与互斥核心规则** | 函数体内分支判断 | 先按**源码顺序**求显式实参，再按**声明依赖顺序**补算缺省值；已显式供应的参数**跳过执行**默认表达式 |
| **编译期防呆** | 容易传错 `null` 位置 | 静态报错引用后序参数、报错重复供应 `.x = 1, .x = 2`、报错未知参数名 `.limti = 3` |

### 下一篇预告

正如上面所见：**默认表达式甚至可以调用像 `opposite(first)` 这样的普通运行时辅助函数。**

那么，一个邪恶的念头自然而然地冒了出来：
- 如果有人在默认表达式调用的函数里，偷偷写了一句 `println("偷偷打一行日志")` 呢？
- 如果有人在默认表达式里通过指针偷偷修改了调用者传进来的对象 `source.value.* = 999`，或者偶有疏忽写了一个死循环呢？
- 如果一个函数签名看起来平平无奇，结果每次调用它省略参数时都会在背后偷偷发网络包、改全局状态，我们还敢相信函数的行为契约吗？

反过来，如果我们的传感器分析仪有一个配置对象 `source: *Limit`，我们希望默认参数每次调用时自动从指针里**只读**最新的阈值：`.limit: u32 = source.value.*`，这又该不该允许呢？

在下一篇 **《Flurry 演进手记 (023)：为什么默认参数里无法偷偷打日志——运行时默认值与 P3 稳定观测契约》** 中，我们将初次邂逅 Flurry 独步天下的**运行时纯度金字塔（Runtime Purity Hierarchy）**，揭晓为什么默认参数必须宣誓效忠 **`P3`（稳定观测且有限完成）**。
