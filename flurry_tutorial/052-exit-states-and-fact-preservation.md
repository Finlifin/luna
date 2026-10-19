# Flurry 演进手记 (052)：函数退出时留下了什么——出口状态契约与跨调用事实保持

在 [上一篇 (051)](./051-six-memory-effects-and-behavior-bounds.md) 中，我们学会了用 `where outcomes ...` 为函数的内存访问划定行为上界（`read`、`write`、`kill`、`connect`、`move`、`system`）。

今天，在继续编写环形缓冲区（`RingBuffer`）的游标管理模块时，我们遇到了三个紧接着的问题：

1. **问题一（知道“改了”，但不知道“改成了什么”）**：我们写了一个原子替换游标的函数 `replace(target: *u32, next: u32) -> previous: u32`，它把 `target.*` 的旧值读出来返回，同时把新值 `next` 写进 `target.*`。
   如果签名上只写 `where outcomes read<target> and write<target>`，调用方只知道“它读写了 `target`”，却无法证明**调用结束后 `target.*` 一定等于 `next`**！
2. **问题二（调用一个函数后，旁边的事实为什么没丢？）**：假设我们在一个循环里维护剩余步数 `var remaining: u8 = 2;`，当进入 `while remaining > 0` 循环体后，我们调用了一个传入的未知回调函数 `callback(result)`，紧接着执行减法 `remaining = remaining - 1`。
   调用完未知的 `callback` 后，编译器是怎么确信 **`remaining > 0` 这个事实依然成立**、从而保证无符号减法 `remaining - 1` 绝对不会下溢的？
3. **问题三（成功出口 vs 错误出口的楚河汉界）**：如果一个函数可能提前返回错误（如 `!DivideErr u32`），它正常完成时的 `ensures` 契约，会不会被错误地套用到报错退出的分支上？

这就把我们带到了程序验证中最核心的工程基石：**出口状态契约（Exit State Contracts）** 与 **跨调用事实保持（Fact Preservation / Frame Rule）**！

---

## 第一版：只有 `outcomes` 上界而缺少出口状态关系的盲区（v0）

让我们先看看，如果 `replace_v0` 只声明了 `outcomes` 行为上界，却没有声明出口状态关系（`ensures`），调用方会陷入怎样的窘境：

```flurry
use std.prelude.*;

-- v0：只声明了允许读写 target，但没有说明返回时 target.* 到底变成了什么值！
fn replace_v0(target: *u32, next: u32) -> u32
where outcomes read<target> and write<target>
{
    let saved = target.*;
    target.* = next;
    saved
}

test {
    var current = 4u32;
    let prev = replace_v0(current.ref, 9u32);
    -- 在运行时，current 确实变成了 9；
    -- 但在静态契约层面，因为 outcomes write<target> 已经作废了 current 的旧事实，
    -- 且没有 ensures 补充新事实，调用方静态上只知道 current 是一个未知的 u32！
    println("{} {}", prev, current);
}
```

为什么仅靠 `outcomes read<target> and write<target>` 不足以让调用方推导出 `current == 9`？
正如 `docs/functions/contracts-and-effects.md` 所指出的：
> **“通过同一个指针先写后读，和先读后写，可以具有相同的效果上界却有不同结果。”**

无序的效果集合 `{read<target>, write<target>}` 只划定了“这个函数碰了哪块地盘”，却没有回答“离开现场时这块地盘变成了什么样”。

要完整刻画一次函数调用，调用方一共需要取得**四类信息**！

---

## 第二版：四位一体的完整函数契约与正常/错误出口分流（v1）

让我们画一张 **Code Cartoon**，把函数边界上的四类契约信息（`docs/resource-management/contracts-and-verification.md` §契约的四个部分）拼成一张全景图：

```text
  函数调用的“四位一体”完整契约模型：

  [ 1. 入口要求 (requires) ]
     调用前必须成立的前提（如 `requires denominator != 0`）。
     必须在危险动作发生前兑现，绝不能先除零或溢出后再补检查！
        │
        ▼
  ┌─────────────────────────────────────────────────────────────────────┐
  │ 2. 行为上界 (outcomes)                                              │
  │    允许读取、修改、终结和产生外部事件的范围（如 `write<target>`）。 │
  │    ──► 凡是【不在】写入/终结足迹内、且地址未暴露的无关状态，        │
  │        其原有事实自动跨过调用边界保持不变！（Frame Rule 保持关系）  │
  └─────────────────────┬───────────────────────────────┬───────────────┘
                        │                               │
         (正常返回出口 Normal Exit)       (错误/提前中断出口 Error/Abort Exit)
                        ▼                               ▼
  [ 3A. 正常出口关系 (ensures) ]          [ 3B. 错误出口状态 ]
    正常完成路径交付的保证                  未正常交付结果！绝不能把正常路径的
    （如 `ensures target.* == next`）。     `ensures` 强套到错误传播路径上！
                        │
                        ▼
  [ 4. 结果及环境依赖交付 ]
    返回值与外部保留对象携带的存活依赖、初始化状态与 Once 清理责任。
```

看看这张图里的两个关键点：

1. **用 `outcomes` 配合 `ensures` 完成“破旧立新”**：
   在 `replace(target: *u32, next: u32) -> previous: u32` 中：
   - `outcomes read<target> and write<target>` 告诉调用方：“我改写了 `target`，请把调用前关于 `target.* == 4` 的旧事实作废（破旧）”；
   - `ensures target.* == next` 紧接着在正常返回出口上发布新事实：“当函数正常返回时，我保证 `target.*` 的新值等于 `next`（立新）”！
2. **正常出口与错误出口严格分流**：
   当一个函数返回 `!DivideErr u32`（比如 `divide(numerator, denominator)`）时，如果在 `divide` 中因为分母为 0 走 `error DivideErr.ZeroDivisor` 分支退出，外层的 `half_quotient` 通过 `!` 立即中止后续计算并向外传播错误——编译器**绝对不会**把正常返回才成立的结果保证套用到未正常交付结果的错误退出路径上！

让我们把 `replace` 与可恢复的 `divide` 写进 `v1` 版本：

```flurry
use std.prelude.*;

-- 1. 允许修改的范围 (outcomes) 与正常完成后的状态内容 (ensures) 分别表达！
fn replace(target: *u32, next: u32) -> previous: u32
where outcomes read<target> and write<target>,
    ensures target.* == next
{
    let saved = target.*;
    target.* = next;
    saved
}

-- 2. 用错误限定类型表达可恢复失败：正常路径与错误退出路径各自遵循独立的状态转移
enum DivideErr {
    ZeroDivisor,
}

fn divide(numerator: u32, denominator: u32) -> !DivideErr u32 {
    if denominator == 0u32 {
        error DivideErr.ZeroDivisor
    } else {
        numerator / denominator
    }
}

fn half_quotient(numerator: u32, denominator: u32) -> !DivideErr u32 {
    -- 若 divide 返回错误，`!` 立即沿错误出口离开 half_quotient，绝不执行下面的 `value / 2u32`
    let value = divide(numerator, denominator)!;
    value / 2u32
}

test {
    var current = 4u32;
    let previous = replace(current.ref, 9u32);
    -- 此时静态验证器已经确知 `current == 9u32`！
    assumes current == 9u32;
    println("replace: prev={}, curr={}", previous, current);

    let first = half_quotient(20u32, 2u32)! {
        DivideErr.ZeroDivisor => 0u32,
    }
    let second = half_quotient(20u32, 0u32)! {
        DivideErr.ZeroDivisor => 0u32,
    }
    println("divide: {} {}", first, second);
}
```

运行输出：

```text
replace: prev=4, curr=9
divide: 5 0
```

---

## 第三版：跨调用事实保持（Frame Rule）——哪些事实能活过一次调用，哪些必须失效？（v2）

现在，我们来攻克本篇最精彩的核心问题：
**当我们在代码里建立了一个事实（比如 `remaining > 0` 或 `left.* == 42`），紧接着调用了一个函数 `foo(...)`，调用返回后，刚才的事实到底还算不算数？**

如果编译器太胆小（每次调用任何函数都把当前已知的所有事实全部清空），那你每调一次辅助函数，后面所有的数组下标和无符号减法就全得重新写一遍运行时检查！
反过来，如果编译器太莽撞（调用已经修改了该变量，却还拿旧事实去优化），就会酿成严重的内存安全灾难！

Flurry 采用的是基于**存储可达性与写入足迹的精确失效规则（Reachability & Footprint-Based Invalidation）**：

```text
  跨调用事实保持与失效的黄金判据（调用 `callee(...)` 时）：

  当前已建立的事实 F
        │
        ├──► 情况 1：F 依赖的是【从未取过地址 (.ref) 的按值私有局部变量】（如 `remaining > 0`）
        │       └──► 哪怕 callee 是一个完全未知的外部回调函数（未声明 pure）！
        │            因为外界根本没有任何指向 `remaining` 的指针，callee 绝无可能修改它！
        │            ══► ✅ 事实 F 100% 跨调用保持有效！
        │
        ├──► 情况 2：F 依赖的是【已取址的局部值或外部指针目标】（如 `counters.left.* == 42`），
        │            但 callee 有明确契约（如 `outcomes write<counters.right>`），
        │            且由【分离证据】证明 callee 的写入足迹与 F 依赖的区域完全分离！
        │            ══► ✅ 事实 F 跨调用保持有效！
        │
        └──► 情况 3：F 依赖的是【已取址变量或指针目标】，
                     且 callee 写入了可能重叠的区域，或 callee 是无契约的不透明/未知调用（含可能写内存的隐式 Drop）！
                     ══► ❌ 事实 F 立即按版本失效！后续使用前必须重新检查或建立！
```

看看**情况 1**！这在实际编程中太有用了！
比如我们在写一个高阶循环函数 `repeat(callback, value, count)` 时：

```flurry
fn repeat(callback: fn(bool) -> bool, value: bool, count: u8) -> bool {
    var remaining = count;
    var result = value;
    while remaining > 0u8 {
        result = callback(result);
        -- 注意这里！在调用了完全未知的 `callback(result)` 之后，
        -- 为什么编译器依然确信 `remaining > 0u8` 成立，从而允许安全的无符号减法 `remaining - 1u8`？
        remaining = remaining - 1u8;
    }
    result
}
```

因为 `remaining` 是一个**从未暴露过 `.ref` 地址的按值标量局部变量**！哪怕 `callback` 没有标 `pure`、哪怕它在外面翻江倒海，它也绝对不可能把手伸进 `repeat` 的私有栈帧里改掉 `remaining`！因此，在进入 `while remaining > 0u8` 分支时建立的事实 `remaining > 0u8`，稳稳地跨过了 `callback(result)` 调用，直接证明了下一行 `remaining - 1u8` 不会发生无符号下溢！

同样，哪怕是在代数效应（`#poll`）挂起并交给外部 Handler 执行时（见 `docs/functions/contracts-and-effects.md` 示例 2），函数的私有按值参数 `flag` 也绝不会因为外部 Handler 的执行而自行改变，因此 `ensures result == flag'old` 依然稳如泰山！

让我们把这套跨调用事实保持机制写进 `v2` 代码中：

```flurry
use std.prelude.*;

effect poll() -> bool;

-- 1. 跨越未知回调：未暴露地址的私有按值标量 remaining，其 `remaining > 0u8` 事实跨调用保持！
fn repeat(callback: fn(bool) -> bool, value: bool, count: u8) -> bool {
    var remaining = count;
    var result = value;
    while remaining > 0u8 {
        result = callback(result);
        -- 编译器确知 callback 无法触碰未取址的 remaining，因此 remaining >= 1u8 依然成立！
        remaining = remaining - 1u8;
    }
    result
}

-- 2. 跨越代数效应 Handler：私有按值参数 flag 不会因外部 poll()# 执行而改变！
fn keep(flag: bool) -> result: #poll bool
where ensures result == flag'old
{
    poll()#;
    flag
}

fn checked_keep() -> result: bool
where ensures result
{
    keep(true)# {
        poll() => false,
    }
}

test {
    let r = repeat(|v| not v, true, 3u8);
    let k = checked_keep();
    println("repeat: {}, keep: {}", r, k);
}
```

运行输出：

```text
repeat: false, keep: true
```

### 编译器防呆：试图兑现不了后置条件 `ensures`？当场拒绝！

反过来，如果一个函数声明了 `ensures result == value + 1u32`，或者声明了在出口处某个指针满足某性质，但它的实现却偷工减料（或者在中间调用了一个可能篡改该指针目标的未知函数，导致事实失效），编译器在核验函数出口契约时就会立刻报错拒绝：

```flurry
-- ❌ 编译期直接拒绝！
-- 错误：函数签名承诺 `ensures result > value`，但实现却直接返回了原 `value`，违反出口状态契约！
verified fn broken_increment(value: u32) -> result: u32
where requires value < 100u32,
    ensures result > value
{
    value
}
```

---

## 本篇小结

将第 049 至 052 篇串联起来，我们就得到了 Flurry 处理内存状态演进的完整闭环：

| 契约组件 / 机制 | 解决的核心问题 | 保持与失效规则 |
| :--- | :--- | :--- |
| **入口要求 (`requires`)** | 调用前必须具备的合法前提 | 在危险动作发生前兑现；按调用入口状态快照解释 |
| **行为上界 (`outcomes`)** | 允许触碰哪些内存视图与外部事件 | 凡在 `write`/`kill` 足迹内的旧事实失效；分离区域的事实保留 |
| **出口状态 (`ensures`)** | 正常返回时建立的新状态事实 | 仅在正常完成路径交付（可用 `'old` 引用入口值）；绝不误用于错误退出路径 |
| **跨调用事实保持** | 避免每次调用都清空所有已知事实 | **未取址的按值局部标量**可无条件跨越任意未知回调与效应 Handler；**已取址/指针目标**凭分离证据保持，遇潜在重叠写入即失效 |

### 留给下一篇的悬念

在这几篇的文档和例子里，你一定注意到了三个经常同台亮相的修饰符或概念：
- 我们在 [第 023 篇](./023-why-default-params-require-p3-stability.md) 学过的默认参数契约：**`P3`（稳定观测）**；
- 我们在函数签名上写的：**`pure fn`**（承诺 **`P4` 值纯**）；
- 我们在写可执行规格时用的：**`spec fn`** 与 **`predicate`**；
- 还有最顶层的：**`P5`（总值纯）** 与最底层的 **`P0` / `P1` / `P2`**！

为什么在传统语言里，“纯函数（Pure Function）”只是一个笼统的概念，而在 Flurry 里，却要把运行时纯度像精密仪器一样切分成 **`P0`、`P1`、`P2`、`P3`、`P4`、`P5` 六层阶梯**？
- 为什么一个拥有无限死循环 `while true {}` 的函数 `spin()` 可以是 **`P4`（`pure fn`）**，却绝对不能是 **`P5`**，而且编译器绝对不能因为没人用它的返回值就把它当死代码删掉？！
- 为什么同样是返回 `"same"` 字符串，新建堆缓冲区的 `fresh_text()` 既不是 `P4` 也不是 `P2`，编译器绝不能把两次调用合并成一次（CSE）？

在下一篇 **《Flurry 演进手记 (053)：“纯函数”不是非黑即白——运行时纯度的六层阶梯（`P0` 到 `P5`）全解》** 中，我们将把这六层阶梯画成一张震撼的全景图，带你领略系统级纯度设计的巅峰艺术！
