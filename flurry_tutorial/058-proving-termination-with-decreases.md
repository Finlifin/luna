# Flurry 演进手记 (058)：证明你的递归一定能停下来——decreases 终止性度量与良基关系

在 [上一篇 (057)](./057-preconditions-requires-and-postconditions-ensures.md) 的结尾，我们发现了一个在形式化验证与系统编程中至关重要却常被忽视的盲区：**部分正确性（Partial Correctness）$\neq$ 完全正确性（Total Correctness）**！

为什么“证明代码一定会在有限步内停下来（Termination）”在 Flurry 中如此关键？
回忆一下我们在前面几卷遇到的三个场景：
1. **运行时默认参数与 `spec fn`（第 023 & 053 篇）**：当你在函数签名里写 `.capacity: usize = next_power_of_two(min_cap)`，或者写一个供契约使用的可执行规格 `spec fn` 时，语言要求它不仅要满足 `P3` 稳定观测，还**必须在合法入口下有限步完成（Terminate）**！请注意：仅仅在函数前面加上 `pure fn` 只承诺 `P4`（值确定计算），**并不自动保证递归或循环一定会终止**！
2. **环形缓冲区的树状/分治块查找**：在嵌入式固件中，一个死循环或无限递归会直接触发硬件看门狗（Watchdog）复位整机。
3. **逻辑相容性（Soundness）**：如果允许一个永不返回的死循环函数声称 `ensures false`，整个验证逻辑大厦就会瞬间崩塌！

可是，图灵老先生早在 1936 年就证明了通用的停机问题（Halting Problem）是不可判定的。编译器怎么可能知道你的递归或循环一定能停下来呢？
答案是：**不需要编译器去解决通用停机问题，而是由你通过 `decreases expr` 指出一个“每一步都在严格变小、且永远不会跌破底线的良基度量（Well-Founded Measure）”！**（详见 `docs/functions/contracts-and-effects.md` 与 `docs/grammar/syntax`）。

---

## 第一版：隐藏在边界条件里的无限递归陷阱（v0）

假设我们在为环形缓冲区编写一个二分查找块索引的递归函数，或者一个看似简单的计算对齐阶数的递归函数 `log2_floor_v0(n: u32) -> u32`。
如果我们不加任何终止性度量，随手写下 `v0`：

```flurry
use std.prelude.*;

-- v0：试图用递归计算 floor(log2(n))，但漏掉了 n == 0 的边界情况！
pure fn log2_floor_v0(n: u32) -> u32 {
    if n == 1u32 {
        0u32
    } else {
        -- 灾难：如果调用者传入了 n = 0，0 / 2 依然是 0！
        -- 函数会永远以 log2_floor_v0(0) 无限递归下去，直到栈溢出崩溃！
        1u32 + log2_floor_v0(n / 2u32)
    }
}
```

让我们画一张 **Code Cartoon**，看看为什么 `n = 0` 会让这段代码坠入无底深渊：

```text
  正常输入 (n = 4):
    log2_floor_v0(4) ──► log2_floor_v0(2) ──► log2_floor_v0(1) ──► 返回 0 (停机！)
    参数变化: 4  >  2  >  1  (严格递减！)

  异常输入 (n = 0):
    log2_floor_v0(0) ──► log2_floor_v0(0) ──► log2_floor_v0(0) ──► ... 💥 Stack Overflow!
    参数变化: 0 == 0 == 0  (原地踏步，根本没有变小！)
```

在普通语言里，这个 Bug 只有在运行时传进 `0` 把栈撑爆时才会被发现。
而在 Flurry 中，只要你加上一条 **`where decreases`** 子句，编译器在编译期就能立刻抓出这个 `0 == 0` 的无限递归漏洞！

---

## 第二版：在函数签名上声明 `where decreases expr`——良基递减证明！（v1）

在 Flurry 的函数 `where` 子句中，除了 `requires` 和 `ensures`，你还可以声明 **`decreases expr`**（详见 `docs/grammar/syntax` `decreases_clause -> decreases expr`）。

什么叫**良基关系（Well-Founded Relation）**？
用大白话说，就是一个集合上不存在无限严格递减的链条 `$m_0 \succ m_1 \succ m_2 \succ \dots$`。
例如，对于非负整数（如 `u32`、`usize`，天然有下界 $0$）：
- 如果在每一次递归调用发生前，度量值 $m_{\text{next}}$ 都**严格小于**当前入口的度量值 $m_{\text{curr}}$（即 $0 \le m_{\text{next}} < m_{\text{curr}}$），
- 那么不管初始的 $m_0$ 有多大，它最多只能减小有限次，**必然在有限步内触底停机！**

让我们看看如果直接给刚才有漏洞的 `log2_floor_v0` 加上 `where decreases n`，编译器是怎么当场拒绝它的：

```flurry
-- ❌ 编译期直接拒绝！
-- 当 n == 0 时，else 分支发起递归调用 log2_floor_bad(0 / 2)，
-- 新度量 0 并不严格小于旧度量 0 (0 < 0 为假)，无法证明终止性！
pure fn log2_floor_bad(n: u32) -> u32
where decreases n
{
    if n == 1u32 {
        0u32
    } else {
        1u32 + log2_floor_bad(n / 2u32)
    }
}
```

为了修复这个漏洞，我们有两种选择：要么在 `if` 条件里把 `0` 和 `1` 一起拦截（`if n <= 1u32`），要么在 `where` 子句里加上前置条件 `requires n >= 1u32`。
来看看修复后的 `v1` 版本：

```flurry
use std.prelude.*;

-- v1 (方案 A)：对所有 u32 输入全定义且证明终止
pure fn log2_floor(n: u32) -> u32
where decreases n
{
    if n <= 1u32 {
        0u32
    } else {
        -- 进入此分支时已知 n >= 2，因此 1 <= n / 2 < n，
        -- 编译器轻松证明新度量 (n / 2) < 旧度量 (n)，且下界 >= 0！
        1u32 + log2_floor(n / 2u32)
    }
}

-- v1 (方案 B)：结合 requires 与 ensures 的环形缓冲区排空计数递归证明
verified fn drain_steps(remaining: usize, batch_size: usize) -> steps: usize
where requires batch_size > 0usize,
      decreases remaining,
      ensures steps <= remaining
{
    return 0usize while remaining == 0usize;

    if remaining <= batch_size {
        1usize
    } else {
        -- 因为 batch_size > 0 且 remaining > batch_size，
        -- 必然有 remaining - batch_size < remaining！
        1usize + drain_steps(remaining - batch_size, batch_size)
    }
}

test {
    println("log2(8) = {}", log2_floor(8u32));         -- 3
    println("steps = {}", drain_steps(10usize, 3usize)); -- 4
}
```

让我们画一张 **Code Cartoon**，看看编译器在检查 `drain_steps` 时是如何一步步核对良基梯子的：

```text
  函数入口: drain_steps(remaining, batch_size)
   • 前置条件已知: batch_size > 0
   • 记录入口度量: M_old = remaining (usize >= 0)
         │
         ├──► [分支 1: remaining == 0] ──► 直接 return 0 (无递归，立即终止 ✅)
         │
         ├──► [分支 2: remaining <= batch_size] ──► 直接 return 1 (无递归，立即终止 ✅)
         │
         └──► [分支 3: remaining > batch_size]
               发起递归调用: drain_steps(remaining - batch_size, batch_size)
               • 核验 1 (递归调用的 requires): batch_size > 0 依然成立 ✅
               • 核验 2 (度量下界与无溢出): remaining > batch_size 保证减法不越界且 >= 0 ✅
               • 核验 3 (度量严格递减):
                 因为 batch_size >= 1，所以 M_new = (remaining - batch_size) < M_old ✅
```

---

## 第三版：循环中的 `decreases` 语句与复合字典序度量（v2）

除了递归函数，`while` 循环也是潜藏死循环的重灾区！
比如我们在环形缓冲区里写一个扫描探测循环，或者写一个双游标向中间靠拢的收拢循环。怎样证明一个 `while` 循环一定不会死循环？

在 Flurry 中，`decreases` 不仅可以作为函数签名上的 `where decreases expr` 子句，还可以直接作为**循环体内部的 `decreases expr;` 语句**（详见 `docs/grammar/syntax` `decreases_statement -> decreases expr`）！

而且，当单靠一个变量不足以表达“每步必减”（比如经典的二元步进或 Ackermann 风格状态转移：外层步数 `epoch` 不变时内层剩余量 `rem` 减小，或者 `epoch` 减小时 `rem` 重置）时，你还可以用元组 **`(epoch, rem)`** 构成**字典序良基度量（Lexicographic Measure）**！

来看看 `v2` 版本中循环 `decreases` 语句与区间收敛度量的实战：

```flurry
use std.prelude.*;

-- 在环形缓冲区的连续区间 [lo, hi) 中向中间夹逼扫描：
-- 每次循环要么 lo 加 1，要么 hi 减 1，区间宽度 (hi - lo) 严格递减！
verified fn converge_bounds(start_lo: usize, start_hi: usize) -> meet: usize
where requires start_lo <= start_hi,
      ensures meet >= start_lo and meet <= start_hi
{
    var lo = start_lo;
    var hi = start_hi;

    while lo < hi {
        -- 循环不变量：始终保持 start_lo <= lo <= hi <= start_hi
        invariant start_lo <= lo and lo <= hi and hi <= start_hi;
        -- 循环终止性度量：区间长度 (hi - lo) 在每轮迭代中严格减小且 >= 0！
        decreases hi - lo;

        if (hi - lo) % 2usize == 0usize {
            lo += 1usize;
        } else {
            hi -= 1usize;
        }
    }

    lo
}

test {
    println("converged at: {}", converge_bounds(2usize, 7usize));
}
```

看看 `while lo < hi` 肚子里的那两行黄金搭档：
1. **`invariant ...;`**：告诉编译器在每一轮循环开始和结束时始终保持为真的状态不变量（我们将在第 060 篇深入剖析它！）；
2. **`decreases hi - lo;`**：告诉编译器，不管本轮走的是 `lo += 1usize` 分支还是 `hi -= 1usize` 分支，下一轮的 `hi - lo` 都严格比本轮小 `1`！既然 `lo < hi` 保证了进入循环体时 `hi - lo >= 1`，这个循环最多跑 `start_hi - start_lo` 轮就必定终止！

```flurry
-- ❌ 编译期直接拒绝！如果某个分支忘记推进游标，hi - lo 没有严格减小，终止性证明立即失败！
verified fn bad_loop(start_lo: usize, start_hi: usize)
where requires start_lo <= start_hi
{
    var lo = start_lo;
    var hi = start_hi;
    while lo < hi {
        invariant lo <= hi;
        decreases hi - lo;

        if lo == 0usize {
            -- ❌ 糟糕！当 lo == 0 时什么都没改，下一轮 hi - lo 与本轮相等，死循环被当场抓获！
        } else {
            lo += 1usize;
        }
    }
}
```

---

## 本篇小结

| 终止性机制 | 语法位置 | 编译器核验的核心义务 |
| :--- | :--- | :--- |
| **函数递归度量** | `where decreases expr` | 在每次递归调用点，证明新度量严格小于当前函数入口度量，且满足良基下界 |
| **循环迭代度量** | 循环体内的 `decreases expr;` | 在每条回到循环头的路径（含 `continue`）上，证明下一轮度量严格小于本轮度量 |
| **与 `pure` / `spec` 的关系** | `pure fn` vs `spec fn` | `pure fn` 只承诺 `P4` 值确定性（不自动保证终止）；默认参数与 `spec fn` 额外要求有限完成证明 |

---

### 留给下一篇的悬念

在这两篇里，我们顺手写出了一两个标着 **`verified fn`** 的函数，而在第 017 篇里我们又写过标着 **`unsafe fn`** 的函数，平时写的普通函数则什么修饰词都没加。

你心里一定攒了一肚子疑问：
- 在 Flurry 中，**默认不加修饰词的 `safe`（普通安全等级）、`unsafe`（底层接管等级）和 `verified`（形式化数学验证等级）**，这三重境界到底有着怎样的边界规则？
- 为什么说“一刀切的全部 `safe`”和“一刀切的全部形式化证明”在大型系统工程里都行不通？
- 当一个普通的 `safe` 函数去调用一个 `verified fn` 时，它会传染把调用者也变成 `verified` 吗？反过来，我们怎样用 `verified fn` 把一段内部用了 `unsafe` 指针操作的环形缓冲区代码，包装成对外 100% 具有数学安全保证的接口？

在下一篇 **《Flurry 演进手记 (059)：安全的三重境界——normal（默认安全）、unsafe 与 verified 的边界交互》** 中，我们将绘制出 Flurry 三级安全体系的全景地图！
