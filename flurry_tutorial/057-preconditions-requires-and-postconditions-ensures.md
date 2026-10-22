# Flurry 演进手记 (057)：在函数边界立下军令状——where requires 前置条件与 ensures 后置条件

在 [上一篇 (056)](./056-assumes-runtime-checks-and-fact-invalidation.md) 中，我们学会了在函数体内部用 `assumes expr;` 语句建立受检事实，并见证了当内存被修改时，编译器如何通过版本机制自动将过期的旧事实作废。

但是，如果一个系统有上百个函数，难道每次调用 `ring_buffer.advance_cursor(step)` 或 `quotient(a, b)` 时，调用者和实现者都要翻开对方的函数体源码，一行行去猜“传 `0` 进去是否会除零崩掉？”、“函数返回的新游标到底保不保证小于缓冲区容量 `cap`？”吗？

当然不。真正的模块化验证，必须**在函数签名边界立下军令状（Function Contracts）**，让调用方无须展开被调函数的实现就能完成静态检查，也让实现方无须知道谁会调用自己就能独立验证函数体。

这一篇，我们就来解锁 Flurry 函数契约系统的两大基石：**`where requires` 前置条件** 与 **`ensures` 后置条件**（详见 `docs/functions/contracts-and-effects.md` 与 `docs/resource-management/contracts-and-verification.md`）。

---

## 一、 把前提藏在肚子里的“盲盒函数”

假设我们在编写环形缓冲区（`RingBuffer`）的游标步进计算与平均吞吐率计算函数 `quotient(numerator: u32, denominator: u32) -> u32`。
如果我们省略函数签名契约，而是把 `assumes` 藏在函数体里面：

```flurry
use std.prelude.*;

-- 签名上看不出任何限制，前提检查全藏在函数体肚子里
fn quotient(numerator: u32, denominator: u32) -> u32 {
    assumes denominator != 0u32;
    numerator / denominator
}

fn compute_rate(bytes: u32, elapsed_sec: u32) -> u32 {
    -- 问题 1：调用者在调用处哪怕已经检查过 elapsed_sec > 0，
    --        因为 quotient 的签名没有声明 requires，每次进去还是得做检查。
    -- 问题 2：如果调用者忘了检查、直接传了 0 进来，错误直到深入 quotient 内部才暴露。
    -- 问题 3：调用者拿到的返回值只知道是个 u32，编译器根本无从得知它必然 <= bytes。
    quotient(bytes, elapsed_sec)
}
```

下面来看看没有边界契约时，调用方和被调方是如何“盲人摸象”的：

```text
  [ 调用方 compute_rate ]                 [ 被调方 quotient ]
   我手里其实知道 elapsed_sec == 3 != 0，      我无从得知外面传了啥，只能自己 assumes denominator != 0；
   但我无从得知 quotient 需要这个事实；       我其实算出的结果一定 <= numerator，
   拿到返回值后，我也完全丢失了范围信息。      但签名只写了 -> u32，白白扔掉了这条宝贵性质。
                  │                                         ▲
                  └────────── 只有干巴巴的 fn(u32, u32) -> u32 ──┘
```

我们需要把“入口要求”和“出口承诺”直接提升到函数的 `where` 子句中。

---

## 二、 `where requires` 与具名结果 `ensures`——权责分明的双向契约

在 Flurry 中，你可以在函数签名末尾的 `where` 子句里，使用 **`requires expr`** 声明调用前置条件，使用 **`ensures expr`** 声明正常返回时的后置条件。

而且，为了在 `ensures` 子句中直接引用函数的返回值，Flurry 支持在返回箭头 `->` 后面给返回值起一个名字：**`-> result: Type`**。

来看看升级后的 `quotient` 与环形缓冲区游标前移函数 `next_cursor`：

```flurry
use std.prelude.*;

-- 1. 除法契约：
--    • requires：调用方必须在进入函数前保证 denominator != 0
--    • -> result: u32：为返回值命名为 result，供 ensures 子句引用。
--    • ensures：函数向调用方保证正常返回的 result 必然 <= numerator
fn quotient(numerator: u32, denominator: u32) -> result: u32
where requires denominator != 0u32,
      ensures result <= numerator
{
    numerator / denominator
}

-- 2. 环形缓冲区游标前移契约：
--    调用前要求 cap > 0 且当前 cursor < cap；
--    返回后保证新游标 next 依然严格落在 0 .. cap 合法区间内。
fn next_cursor(cursor: usize, cap: usize) -> next: usize
where requires cap > 0usize and cursor < cap,
      ensures next < cap
{
    let stepped = cursor + 1usize;
    return 0usize while stepped == cap;
    stepped
}

fn checked_quotient(numerator: u32, denominator: u32) -> u32 {
    -- 调用方在调用前通过 assumes（或上文分支条件）建立 denominator != 0 的受检事实，
    -- 从而合法兑现 quotient 的 requires 入口义务。
    assumes denominator != 0u32;
    quotient(numerator, denominator)
}

test {
    println("quotient: {}", checked_quotient(20u32, 3u32)); -- 6
    println("cursor: {}", next_cursor(3usize, 4usize));     -- 0
}
```

下面来看清 `requires` 与 `ensures` 在函数边界上建立的“双向军令状”：

```text
  调用方 (Caller)                                     被调方实现 (Callee Body)
 ┌───────────────────────────────────┐               ┌───────────────────────────────────┐
 │ 1. 【调用方的义务】               │               │ 2. 【实现方的权利】               │
 │    必须在调用发生前证明或受检建立 │ ──requires──► │    进入函数体时，直接将           │
 │    `denominator != 0`             │               │    `denominator != 0` 当作已知真  │
 │    (若未满足，在调用点直接报错!)  │               │    因此 `numerator / denominator` │
 │                                   │               │    无需再次检查除零               │
 │                                   │               │                                   │
 │ 4. 【调用方的收获】               │               │ 3. 【实现方的义务】               │
 │    调用一旦正常返回，立刻获得新事 │ ◄──ensures─── │    必须在每一条正常返回路径上     │
 │    实 `result <= numerator`       │               │    （含尾表达式和提前 return）    │
 └───────────────────────────────────┘               │    兑现 `result <= numerator`     │
                                                     └───────────────────────────────────┘
```

> **🔍 语法与语义小贴士：具名结果 `-> result: T` 并非局部变量。**
> 看到 `fn quotient(...) -> result: u32` 了吗？
> 1. 这里的 `result:` 是专门为结果规格（如 `ensures result <= numerator`）提供的**返回结果绑定名**。
> 2. **它免于在函数体入口自动创建一个未初始化的局部变量 `var result`。** 在函数体里，你依然像往常一样用尾表达式 `numerator / denominator` 或 `return expr;` 返回结果，编译器会自动把所有正常出口交付的值与规格中的 `result` 对应起来。
> 3. **多出口统一核验**：像 `next_cursor` 里既有 `return 0usize while stepped == cap;` 提前返回，又有末尾的 `stepped` 返回，**每一条正常返回路径都必须满足 `ensures next < cap`**。

```flurry
-- ❌ 编译期直接报错：调用方没有证明或受检建立 denominator != 0，违反 requires 契约。
fn bad_caller(a: u32, b: u32) -> u32 {
    quotient(a, b)
}

-- ❌ 编译期直接报错：提前 return 分支返回了 cap，违反了 ensures next < cap 的后置承诺。
fn bad_next_cursor(cursor: usize, cap: usize) -> next: usize
where requires cap > 0usize and cursor < cap,
      ensures next < cap
{
    return cap while cursor == 0usize; -- ❌ 此时 next == cap，违背 next < cap。
    cursor
}
```

---

## 三、 穿越时间对比调用前后——`'old` 入口快照视图与状态契约

当我们的环形缓冲区函数通过指针修改对象状态（比如 `push_count(count_ptr: *u32)` 把计数器加 1，或者在一个带有代数效应 `#poll` 的函数里保持原值）时，我们在写 `ensures` 后置条件时会遇到一个新问题：

**在 `ensures` 执行的时刻，函数已经跑完了，内存或状态已经变成了新值。如果我们想在 `ensures` 里写“调用结束后的新值，等于调用刚进来时的旧值加 1”，怎么拿到“调用刚进来时的旧值”？**

在 Flurry 中，这同样通过我们熟悉的 **Take View（`'`）机制** 完成：只需在表达式后面加上 **`'old`** 视图（详见 `docs/functions/contracts-and-effects.md` 与 `docs/resource-management/memory-views.md`）。

来看看 `'old` 视图以及指针修改后置条件在第二部分中的实战：

```flurry
use std.prelude.*;

-- 1. 指针更新契约：允许读写 target，并保证正常返回后 target.* 的内容等于新传入的 next
fn replace_slot(target: *u32, next: u32) -> previous: u32
where outcomes read<target> and write<target>,
      ensures target.* == next
{
    let saved = target.*;
    target.* = next;
    saved
}

-- 2. 使用 `'old` 视图引用函数入口时的快照状态。
effect poll() -> bool;

fn keep_flag(flag: bool) -> result: #poll bool
where ensures result == flag'old
{
    poll()#;
    flag
}

fn checked_keep() -> result: bool
where ensures result
{
    keep_flag(true)# {
        poll() => false,
    }
}

test {
    var slot: u32 = 4u32;
    let prev = replace_slot(slot.ref, 9u32);
    println("prev={}, current={}", prev, slot); -- prev=4, current=9
    println("kept={}", checked_keep());          -- kept=true
}
```

如下所示，理解 `requires`、`outcomes`、`ensures` 与 `'old` 的时间轴坐标：

```text
  ───────► [ 函数调用入口 (Entry Snapshot) ]
            • 检查 `requires` 前置条件
            • 锁定 `'old` 视图的基准值（例如 `flag'old` 记住入口时的 true）
            • 固定 `outcomes` 区域选择器（如 `target` 指向的对象）
                  │
                  ▼
           [ 执行函数体 / 效应处理 ]
            • 读取旧值 `saved = target.*`
            • 写入新值 `target.* = next`
                  │
                  ├──► (若走 error / 中止退出路径)
                  │     ⚠️ 不应用正常返回值的 `ensures` 保证。
                  │
                  ▼ (走正常返回路径 Normal Exit)
           [ 函数正常出口 (Exit State) ]
            • 绑定具名结果 `previous` / `result`
            • 检查 `ensures`：此时 `target.*` 指代出口新状态，而 `expr'old` 指代入口旧快照。
```

### 什么时候用 `requires`？什么时候用 `!Err` 错误返回？

很多初学者会问：“既然除零有两种写法——一种是 `where requires denominator != 0`，另一种是返回 `!DivideErr u32`，我们在工程里该怎么选？”
规范（`docs/functions/contracts-and-effects.md`）给出了清晰明确的工程准则：
- **选 `where requires`（契约前置条件）**：当违反该条件属于**调用方逻辑 Bug**（例如传给内部环形缓冲区的游标越界、传入的容量为 0），且调用方完全有能力在调用前通过静态不变量或 `assumes` 消除它，从而让被调函数内部做到**零分支检查开销**。
- **选 `!Err T`（错误限定结果）**：当失败来自**失控的外部输入或预期内的可恢复异常**（例如用户刚从命令行输入的除数可能是 0，或者串口收到了噪声字节），希望调用者用 `!` 传播或用 `! { ... }` 分支优雅恢复。

---

## 本篇小结

| 契约子句 / 构造 | 语法形态 | 核心职责与边界 |
| :--- | :--- | :--- |
| **前置条件** | `where requires expr` | 调用前必须成立的前提；调用方负责兑现，被调方在主体内直接作为已知事实使用 |
| **具名结果** | `fn foo(...) -> res: T` | 为正常返回值命名供 `ensures` 引用；不创建未初始化的局部变量 |
| **后置条件** | `where ensures expr` | 所有正常返回路径（含尾表达式与提前 `return`）必须兑现的结果与状态保证 |
| **入口快照视图** | `expr'old` | 在 `ensures` 中引用函数调用入口时刻的值或状态快照 |

---

### 下一篇预告

仔细看我们在总结表格里写的一句话：
> **“`ensures` 描述的是函数在正常返回路径上交付的保证，但它并未自动表示函数一定会终止（Terminate）。”**

什么意思？假设有人写了一个递归函数：
```flurry
pure fn bad_countdown(n: u32) -> result: u32
where ensures result == 0u32
{
    bad_countdown(n) -- 无限原地递归。
}
```
如果只看“一旦返回，返回值一定等于 0”，这个死循环函数在部分正确性（Partial Correctness）下实际上是成立的——因为它永免返回，所以永免返回一个有别于 0 的值。
但在 Flurry 中，如果把这种不会停下来的函数放进编译期计算、运行时默认参数（要求 `P3` 稳定且有限完成）、或者高可信的 `verified fn` 里，就会导致严重错误。

**我们怎样向编译器给出严格的数学证明，证明一个递归函数或 `while` 循环无论如何都一定会在有限步内停下来？**

在下一篇 **《Flurry 演进手记 (058)：证明你的递归一定能停下来——decreases 终止性度量与良基关系》** 中，我们将解锁良基递减度量 `decreases`。
