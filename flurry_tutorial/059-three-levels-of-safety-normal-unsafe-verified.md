# Flurry 演进手记 (059)：安全的三重境界——normal（默认安全）、unsafe 与 verified 的边界交互

在 [前两篇 (057 & 058)](./058-proving-termination-with-decreases.md) 中，我们在写环形缓冲区的辅助函数时，既写过不带任何安全修饰词的普通 `fn`（默认 `safe` 等级），也写过带 `verified fn` 的形式化验证函数；而在更早的第 017 篇操作 `union` 时，我们还写过 `unsafe fn`。

如果你审视今天主流的编程语言，会发现大家往往陷入了一种“非黑即白”的二分法困境：
- **C / C++**：默认允许一切底层操作，指针随便指、数组随便越界，一旦出错全靠线上 Core Dump 抓鬼；
- **Rust**：引入了 `safe` 与 `unsafe` 的二分法。但对于超出借用检查器固定套路的复杂不变量（比如环形缓冲区的头尾游标回绕关系、或者四维张量不溢出的高次乘积），`safe` 自动推导证不出来，程序员最后只能无奈地套上一层 `unsafe { ... }`，然后写两行英文注释 `-- SAFETY: trust me bro!`；
- **Coq / Lean / Dafny 等定理证明器**：要求每一行业务代码都必须写出完整的数学证明，结果为了写一个打印日志的 UI 按钮，程序员得先证明半天算术引理，工程开发效率直接归零。

Flurry 给出的解决方案，是打破传统的二分法，建立一套在同一代码库中无缝共存、可渐进演进的 **三级安全体系（Three Levels of Safety）**：
**默认 `safe`（Normal） $\leftrightarrow$ `unsafe` $\leftrightarrow$ `verified`**（详见 `docs/program-verification/safety.md` 与 `docs/resource-management/contracts-and-verification.md`）。

---

## 一、 先看全景图——三重境界各自管什么、在哪个编译器阶段检查？

下面先把这三个安全等级的职责、静态保证强度和编译器检查载体（IR 阶段）摆在台面上：

```text
 ╔══════════════════════════════════════════════════════════════════════════════╗
 ║                    Flurry 的三重安全境界 (Three Levels of Safety)             ║
 ╚══════════════════════════════════════════════════════════════════════════════╝

  ┌────────────────────────────────────────────────────────────────────────────┐
  │ 1. 默认 `safe` (Normal —— 不写修饰词的日常代码，占工程 90% 以上)             │
  │   • 静态保证：仿射所有权 (Once/Copy/Drop)、可达性防悬垂 (&r)、              │
  │               内存效果与效应追踪、三次以内多项式自动算术边界 + assumes 受检  │
  │   • 检查载体：在 Normalized IR (定义端规范化 IR) 上完成快速、可预测的检查    │
  └─────────────────────────────────────┬──────────────────────────────────────┘
                                        │
            ┌───────────────────────────┴───────────────────────────┐
            ▼ (向下突破：底层控制 / C FFI)                           ▼ (向上升华：核心算法 / 强规范证明)
  ┌─────────────────────────────────────────┐       ┌─────────────────────────────────────────┐
  │ 2. `unsafe` (显式接管底层不变量)          │       │ 3. `verified` (形式化数学验证)           │
  │   • 适用：C FFI、union 重解释、裸硬件寄存器│ ────► │   • 适用：密码学原语、RingBuffer 核心、 │
  │   • 规则：必须显式标注 `unsafe fn` 或     │ 渐进  │           高次算术判定、把 unsafe 关进笼子│
  │           `unsafe { ... }` 块；开发者负   │ 验证  │   • 规则：基于 Outcome Separation Logic │
  │           责在边界兑现前置契约            │ 升级! │   • 检查载体：在 Instantiated IR 上完成 │
  └─────────────────────────────────────────┘       │               完整的数学证明            │
                                                    └─────────────────────────────────────────┘
```

仔细看图中标注的两个编译器检查阶段（详见 `docs/resource-management/contracts-and-verification.md` §两种检查载体）：
1. **默认 `safe` 检查位于 Normalized IR**：它必须在泛型和模块定义端就完成责任、清理和调用契约检查，算术自动推导严格限制在总次数不超过三次的多元多项式片段内——这保证了日常 `safe` 代码的编译速度极快、报错位置极其稳定，绝不会像 C++ 模板那样等到别人实例化时才爆出莫名其妙的内部错误。
2. **`verified` 证明位于 Instantiated IR**：当你为环形缓冲区核心算法或密码学原语标上 `verified fn` 时，编译器会在具体实例化后的 IR 上动用完整的形式化验证引擎（结合 Outcome Separation Logic、Reachability Logic 与 Matching Logic），为你完成严格的数学定理证明。

---

## 二、 从 传统(`unsafe` 裸奔) 到 (`safe` 封装) 再到 (`verified` 数学封神)

为了体会这三重境界在真实工程里是怎么配合的，让我们来看一个环形缓冲区底层内存拷贝与游标递增的演进故事。

### 底层 `unsafe` 原语与显式边界

在最底层，我们需要对接硬件 DMA 或底层无边界检查的字节搬运 `raw_ring_write`。因为它直接操作裸指针偏移，无法单靠类型系统自动证明安全，所以我们必须将它声明为 `unsafe fn`：

```flurry
use std.prelude.*;

-- 底层 unsafe 原语，显式声明前置条件 requires
unsafe fn raw_write_slot(base: *u32, index: usize, cap: usize, val: u32)
where requires index < cap
{
    -- 底层裸指针偏移写入等操作
    base.* = val;
}
```

如果一个普通的 `safe` 函数想直接调用 `raw_write_slot`，而不加 `unsafe { ... }` 块，或者虽然加了 `unsafe` 块却没有满足它的 `requires index < cap` 前置契约，编译器会毫不留情地拦截：

```flurry
-- ❌ 编译期直接拒绝：safe 函数不能在没有 unsafe { ... } 块的情况下直接调用 unsafe fn。
fn bad_safe_caller(base: *u32, idx: usize, cap: usize) {
    raw_write_slot(base, idx, cap, 42u32);
}

-- ❌ 编译期直接拒绝：记住 `BitVec` 与安全规范的核心规则：
--    “进入 unsafe 也不能跳过已声明的入口契约。”
--    这里既没有证明 idx < cap，也没有用 assumes 建立受检事实，依然拒绝。
fn bad_unsafe_block(base: *u32, idx: usize, cap: usize) {
    unsafe {
        raw_write_slot(base, idx, cap, 42u32);
    }
}
```

### 在 `safe` 函数中用 `assumes` 兑现前提并安全包裹

在日常快速开发阶段，我们可以写一个普通的 `safe` 包装函数 `write_slot_safe`：先用 `assumes index < cap;` 建立受检事实，再在 `unsafe { ... }` 块内调用底层原语：

```flurry
fn write_slot_safe(base: *u32, index: usize, cap: usize, val: u32) {
    -- 先在危险动作之前建立受检事实 index < cap
    assumes index < cap;
    unsafe {
        raw_write_slot(base, index, cap, val);
    }
}
```

### 用 `verified fn` 提供零运行时检查开销的数学证明，供 `safe` 无缝调用

随着项目演进，环形缓冲区的游标递增函数 `checked_increment` 成了每秒执行上亿次的核心热点。我们给它标上 **`verified fn`**，写明 `requires` 和 `ensures`：

```flurry
use std.prelude.*;

-- 经过形式化数学验证的 verified fn。
verified fn checked_increment(value: u32) -> result: u32
where requires value < 4294967295u32,
      ensures result == value + 1u32
{
    value + 1u32
}

-- 普通的 safe 函数可以直接调用 verified fn，完全不需要写 unsafe 块，
-- 更不会被传染成 verified fn。
fn next_step(value: u32) -> u32 {
    assumes value < 4294967295u32;
    let res = checked_increment(value);
    -- 在这里，safe 代码自动获得了 verified 契约交付的事实：`res == value + 1u32`。
    res
}

test {
    println("next: {}", next_step(41u32)); -- 42
}
```

---

## 三、 三大等级跨界调用的“交通法规”全表

现在，让我们把 `safe`、`unsafe`、`verified` 三者之间互相调用的规则梳理为如下调用关系图（详见 `docs/program-verification/safety.md` §等级之间的调用规则）：

```text
  调用方 (Caller) ──────► 被调方 (Callee) 的通行法则：

  [ 1. safe 调用别人 ]
     ├──► 调用 safe     ──► ✅ 直接通行。
     ├──► 调用 verified ──► ✅ 直接通行。（调用方按 safe 规则兑现 requires，即可直接享用
     │                        verified 的 ensures 结果保证，且不会把调用方传染成 verified。）
     └──► 调用 unsafe   ──► ⚠️ 必须用 `unsafe { ... }` 块显式包裹，并兑现其入口契约。

  [ 2. verified 调用别人 ]
     ├──► 调用 verified ──► ✅ 直接通行。（数学证明链无缝拼接）
     ├──► 调用 safe     ──► ✅ 通行。（需证明调用满足该 safe 函数的前置条件与契约）
     └──► 调用 unsafe   ──► 🛡️ 必须用 `unsafe { ... }` 块包裹，并给出满足其安全前提的严格数学证明。
                              （这就是“用 verified 驯服 unsafe”的核心机制。）

  [ 3. unsafe 调用别人 ]
     └──► 调用 safe / unsafe / verified ──► ✅ 均可直接调用（仍须遵守已声明的契约与资源规则）
```

> **💡 核心心智模型：为什么 `safe` 调用 `verified` 不会发生“验证传染”？**
> 很多用过形式化验证工具的工程师最怕一件事：“一旦我给底层库的一个函数加了 `verified`，是不是整个项目所有调它的上层业务函数都得被迫写成 `verified`？”
> **在 Flurry 中，绝对不会。**
> `safe` 调用 `verified fn` 时，完全保持 `safe` 自己的检查模式（在 Normalized IR 上做常规检查，无法静态自动证明的入口条件只需一行 `assumes` 即可受检兑现），而编译器会在后台自动记录对那个 `verified fn` 实例的**证明依赖（Proof Dependency）**：只要 `verified fn` 自己的实现证明通过了，`safe` 调用方就能直接白嫖它 `ensures` 出来的强数学结论。如果 `verified fn` 内部写错了、证明没通过，编译器则绝对禁止交付最终程序。

```flurry
-- ❌ 编译期/交付期直接拒绝。
-- verified fn 的实现返回了原值 value，违反了自己声明的 ensures result == value + 1u32。
-- 哪怕外面的调用方全都满足了 requires，只要 verified 实现证明失败，程序就绝不允许交付。
verified fn broken_increment(value: u32) -> result: u32
where requires value < 4294967295u32,
      ensures result == value + 1u32
{
    value
}
```

### 实战典范：用 `predicate` + `verified` 把高次算术判定装进安全胶囊

还记得我们在 [第 055 篇](./055-cubic-polynomial-automatic-bounds-proving.md) 讲过，为了保证日常编译速度，`safe` 的自动算术推导限制在**总次数不超过三次（Cubic）的多元多项式**以内吗？
那如果我们的传感器信号处理需要检查一个**四维数组** `a * b * c * d <= limit`（这是**四次**多项式。）的容量是否放得下、且中间乘法绝不溢出，该怎么办？

我们可以写一个专门的 **`predicate`**（程序谓词），用安全的防溢出除法阶梯实现四维判定，并由验证层支撑它的数学规格；随后普通的 `safe` 代码只需要一行 `assumes four_dimensions_fit(...)`，就能直接享受这个四次关系的保护，根本不需要在业务代码里操心高次证明。

```flurry
use std.prelude.*;

-- 程序 predicate：注意不标 comptime。它属于程序验证层，而非 PTC 类型求解器
predicate four_dimensions_fit(a: usize, b: usize, c: usize, d: usize, limit: usize) {
    if a == 0usize or b == 0usize or c == 0usize or d == 0usize {
        true
    } else if b > limit / a {
        false
    } else {
        let ab = a * b;
        if c > limit / ab {
            false
        } else {
            let abc = ab * c;
            d <= limit / abc
        }
    }
}

-- 日常 safe 函数：一行 assumes 即可安全使用四维容量判定。
fn check_4d_buffer(a: usize, b: usize, c: usize, d: usize, limit: usize) {
    assumes four_dimensions_fit(a, b, c, d, limit);
    println("4D buffer dimensions verified safe!");
}

test {
    check_4d_buffer(4usize, 4usize, 4usize, 4usize, 256usize);
}
```

---

## 本篇小结

| 安全等级 | 关键字标注 | 检查阶段与核心武器 | 工程定位与团队协作角色 |
| :--- | :--- | :--- | :--- |
| **`safe`（默认）** | 无需标注 | Normalized IR；仿射类型、可达性、三次以内自动算术、`assumes` 受检事实 | **90% 日常应用与系统逻辑**：开发飞快、零悬垂零泄漏 |
| **`unsafe`** | `unsafe fn` / `unsafe { }` | 显式放宽特定底层静态检查，由开发者在边界维护不变量 | **硬件驱动、C FFI 与极底层原语**：范围极小、边界醒目易审计 |
| **`verified`** | `verified fn` | Instantiated IR；OSL / RL / Matching Logic 完整形式化证明 | **核心数据结构（如 `RingBuffer`）与高可信模块**：将 `unsafe` 和复杂算术封装成 100% 数学可信的 `safe` 接口 |

---

### 下一篇预告

在这篇的最后，我们惊鸿一瞥地看到了 `predicate four_dimensions_fit(...)` 和 `verified fn`。
你一定跃跃欲试了：
- 为什么 语言规范 特别强调：**“PTC 只处理类型命题；程序状态、内存与资源规格由程序验证层负责”**？这决定了为什么我们的程序 `predicate` **绝对不加 `comptime` 前缀**。
- 如果现在让你从零开始，亲手为环形缓冲区写一个带 `while` 循环的批量清零或累加函数，并用 **`verified fn`**、**`predicate`** 和循环不变量 **`invariant`** 把它从头到尾完整地做一次形式化数学证明，具体该怎么写？当循环不变量写得太弱时，编译器是怎么提醒你的？

在下一篇 **《Flurry 演进手记 (060)：写出第一段经过数学证明的代码——predicate 与 verified fn 实战》** 中，我们将手把手带你完成人生中第一段经过编译器形式化验证的 Flurry 代码。
