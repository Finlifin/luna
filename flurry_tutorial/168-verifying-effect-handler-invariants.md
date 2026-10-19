# Flurry 演进手记 (168)：用 `verified` 契约验证效应 Handler 的状态不变量

> **阅读指南**：在上一回（第 167 回）中，我们学会了在模块边界封装与消去内部效应。今天，我们要把第二、三卷学过的 **程序契约与形式化验证（`requires`、`ensures`、`outcomes`、`assumes` 与 `verified fn`）** 与第七卷的 **代数效应（Algebraic Effects）** 正式合龙！结合 `docs/functions/contracts-and-effects.md` 与 `docs/resource-management/contracts-and-verification.md`，我们将彻底理清三个核心问题：
> 1. 在 `fn keep(flag: bool) -> result: #poll bool where ensures result == flag'old` 中，具名结果 `result` 绑定的究竟是什么？
> 2. 当函数中途发出了一个效应请求 `poll()#`（控制权交给了外部未知的 Handler 再恢复回来），函数体内哪些变量的已知事实可以保留，哪些内存事实必须**强制失效（Invalidated）**？
> 3. 如何结合 `verified fn` 与 `outcomes` 证明有状态 Handler 步进时的状态不变量？

---

## 一、 痛点：当函数途经外部效应 Handler 时，我们还能相信哪些局部事实？

假设我们在编写一个高可靠的状态控制器。一个函数接收按值传入的布尔标志 `flag: bool` 和一个指向外部共享计数器的指针 `shared_counter: *u32`。在函数执行中途，它发出了一次效应请求 `poll()#`，随后返回结果。

### 审视 `v0`：要么“盲目相信所有旧事实（不安全）”，要么“一刀切清空所有事实（没法证明）”

如果一门语言的验证器在处理效应调用 `poll()#` 时不够精细（`v0`），就会陷入两难绝境：

```flurry
-- 假想 v0 的两种极端错误策略：
-- 极端 A（盲目乐观，导致验证漏洞）：
--   假设在 poll()# 之前，我们检查了 shared_counter.* == 10。
--   但是外层安装的 poll() Handler 恰好也持有 shared_counter 的指针，并在处理 poll() 时
--   把 shared_counter.* 改成了 999！如果验证器在 poll()# 恢复后还以为 shared_counter.* == 10，
--   就会得出完全错误的证明！
--
-- 极端 B（盲目悲观，清空一切局部事实）：
--   反过来，如果一遇到 poll()# 就把函数里【所有变量】的事实全清空，
--   那连从未取过地址的私有按值参数 `flag: bool` 的值都忘光了，根本没法证明返回值等于 `flag'old`！
```

**核心痛点**：
我们需要一套精确到**存储可达性（Storage Reachability）**的效应契约验证规则：
1. 明确效应限定返回类型 `result: #Eff T` 中 `result` 的绑定目标；
2. 精确区分**绝不会被外部 Handler 染指的私有按值局部绑定**与**可能被 Handler 修改的已暴露内存/词法变量**！

---

## 二、 破局（v1）：`result: #Eff T` 的精确绑定与按可达性失效法则

在 `docs/functions/contracts-and-effects.md` 中，Flurry 对带效应函数的契约给出了极其清晰的正式定义：

1. **命名返回绑定 `result: #Eff T` 指向计算正常完成后交付的值 `T`**：
   - 在 `fn keep(flag: bool) -> result: #poll bool where ensures result == flag'old` 中，`result` 的类型是 **`bool`**（即 `keep` 正常返回时交付的最终布尔值），它既不代表效应环境，也不代表中间 `poll()#` 请求返回的值！
2. **跨越效应请求的事实保留与失效（Reachability-Based Fact Invalidation）**：
   - **保留（Preserved）**：函数私有的、**从未暴露过地址（`.ref`）的按值标量局部变量与形参**（如上面的 `flag: bool` 或循环计数器 `remaining`），外部 Handler 没有任何途径去修改它们！因此关于它们的事实（如 `flag == flag'old`、`remaining > 0`）可以**安全地跨越 `poll()#` 效应请求继续保持有效**！
   - **失效（Invalidated）**：任何 Handler 可触及的词法变量、已取址暴露出去的局部存储、以及外部指针指向的内存观测，在经历外部效应请求或未知回调后，其旧内容事实**必须立即失效**，绝不能沿用过期的内存假设！

让我们用一张 Code Cartoon，看清验证器在跨越 `poll()#` 挂起与恢复边界时的“事实安检门”：

```text
  fn keep(flag: bool, shared: *u32) -> result: #poll bool
  where ensures result == flag'old
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 1. 入口状态 (Before Effect):                                            │
  │    • 私有按值参数 `flag` (未取址): 已知事实 [ flag == flag'old ]        │
  │    • 外部暴露指针 `shared.*`     : 假设刚观察到 [ shared.* == 10 ]      │
  └───────────────────────────────────┬─────────────────────────────────────┘
                                      │
                                      ▼ 发出效应请求 `poll()#` ──▶ 进入外部 Handler
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ ⚡ 外部 Handler 执行期间：                                              │
  │    • Handler 无法触碰 keep 栈帧上未取址的私有按值变量 `flag`！          │
  │    • 但 Handler 可能通过别名修改外部内存 `shared.*`！                   │
  └───────────────────────────────────┬─────────────────────────────────────┘
                                      │
                                      ▼ Handler 执行 `resume` 恢复回到 keep
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 2. 恢复后状态 (After Resume):                                           │
  │    ✅ 保留事实：[ flag == flag'old ] 依然 100% 稳固成立！               │
  │    ❌ 失效事实：[ shared.* == 10 ] 被自动清除（防止使用过期脏事实）！   │
  │                                                                         │
  │ 3. 正常返回 `flag`：                                                    │
  │    ✅ 完美证明后置条件 `ensures result == flag'old`！                   │
  └─────────────────────────────────────────────────────────────────────────┘
```

### 演进到 `v1`：验证跨越效应请求的 `ensures` 契约传递

让我们运行这段来自 `docs/functions/contracts-and-effects.md` 的经典验证示例：

```flurry
use std.prelude.*;

effect poll() -> bool;

-- 1. 带有 #poll 效应与后置契约的函数：
--    result 绑定的是 keep 正常完成后交付的 bool 值！
fn keep(flag: bool) -> result: #poll bool
where
    ensures result == flag'old
{
    -- 发出 poll()# 请求；虽然 poll() 返回的 bool 被忽略了，
    -- 且经历了外部 Handler 调用，但私有按值参数 flag 绝不会被外部修改！
    poll()#;
    flag
}

-- 2. 外层调用方消去 #poll 效应，并直接继承 keep 的 ensures 保证！
fn checked_keep() -> result: bool
where
    ensures result
{
    -- 传入 true，因此 keep 的后置契约直接给出 result == true，
    -- 即使 poll() 的 Handler 返回了 false，也不影响 keep 最终返回 true！
    keep(true)# {
        poll() => false,
    }
}

test {
    println("checked_keep: {}", checked_keep());
}
```

---

## 三、 `verified fn`、`outcomes` 与 Handler 提前退出的契约边界（v2）

当我们在构建一个有状态的效应处理器（比如维护配额计数器或步进状态机）时，我们还可以结合 **`verified fn`**（在 Instantiated IR 上完成完整实现证明）与 **`outcomes read<target> and write<target>`** 显式声明状态修改的行为上界与后置不变量！

### 演进到 `v2`：用 `verified fn` 证明有状态 Handler 的步进不变量

```flurry
use std.prelude.*;

-- 使用 verified fn 形式化证明状态步进函数：
-- 1. requires 保证输入未达上限，防止无符号加法溢出；
-- 2. outcomes 明确只读写 state 指向的位置；
-- 3. ensures 保证步进后状态严格等于旧状态加 1！
verified fn verified_step_counter(state: *u32) -> result: u32
where
    outcomes read<state> and write<state>,
    requires state.* < 1000u32,
    ensures state.* == state'old.* + 1u32,
    ensures result == state.*
{
    let next = state.* + 1u32;
    state.* = next;
    next
}

effect next_tick() -> u32;

fn run_two_ticks() -> #[next_tick] u32 {
    let t1 = next_tick()#;
    let t2 = next_tick()#;
    t1 + t2
}

test {
    var counter: u32 = 10u32;

    let sum = run_two_ticks()# {
        next_tick() => {
            -- 在调用 verified_step_counter 前用 assumes 建立受检前提
            assumes counter < 1000u32;
            verified_step_counter(counter.ref)
        },
    };

    -- t1 (11) + t2 (12) == 23, counter == 12
    println("sum: {}, final counter: {}", sum, counter);
}
```

同时，请牢记 `docs/functions/contracts-and-effects.md` 中的两条铁律：
1. **Handler 提前 `return` 时，不能借用尚未完成的被调函数（Callee）的 `ensures` 保证！**
   如果 Handler 分支没有执行 `resume`，而是直接写了一句词法 `return` 退出了外层函数，此时被处理的计算（如 `keep`）根本没有正常跑完！因此，外层函数的 `return` 必须靠自己证明外层函数的后置条件，绝不能把半途而废的 `keep` 的 `ensures` 当作已成立的事实！
2. **显式声明了只读行为上界 `outcomes read<target>` 的函数，绝不允许在体内偷偷写入 `target.*`，也不能在未满足 `requires` 前提或违反 `ensures` 时蒙混过关**：

```flurry
-- 负例 1：违反 outcomes 行为上界（声明只读却执行写入）
fn broken_readonly_contract(target: *u32)
where
    outcomes read<target>
{
    -- ❌ 编译期直接拒绝！
    -- outcomes 仅声明了 read<target> 上界，不允许在函数体内执行 write<target>！
    target.* = 0u32;
}

-- 负例 2：verified fn 的实现体未能兑现其声明的 ensures 后置条件
verified fn broken_verified_increment(value: u32) -> result: u32
where
    requires value < 1000u32,
    ensures result == value + 1u32
{
    -- ❌ 编译期在 verified 证明阶段直接拒绝！
    -- 实现返回了原值 value，无法证明 result == value + 1u32！
    value
}
```

---

## 本篇小结

| 契约 / 验证维度 | Flurry 的规则与语义边界 | 形式化安全保障 |
| :--- | :--- | :--- |
| **`-> result: #Eff T`** | `ensures` 中的 `result` 绑定整个计算正常完成交付的 `T` 值 | 不与单次效应请求的操作返回值混淆 |
| **私有按值变量跨越效应** | 未取址的按值局部标量（如 `flag`、`remaining`）事实保持有效 | 允许在包含效应请求或回调的循环与函数中完成算术与不变式证明 |
| **暴露存储跨越效应** | 已取址变量、指针目标与 Handler 可写词法变量的旧事实自动失效 | 杜绝外部 Handler 修改共享状态后导致的过期事实误用（Soundness） |
| **Handler 提前词法 `return`** | 不授予被中断主体的 `ensures` 后置条件，须独立证明外层返回契约 | 防止借用未完成计算的虚假后置保证 |

---

### 留给下一篇的悬念

走到这里，我们已经把 Flurry 代数效应与限定 Continuation 的方方面面都摸透了。

此时此刻，如果你之前写过 C++20 的无栈协程（`co_await` / `co_yield`），或者写过 Rust 的异步 `async/await`（`Future` + `Pin<&mut Self>`），你脑海里一定会浮现出一个终极架构之问：
- **C++20 协程、Rust `async/await` 状态机、以及 Flurry 的代数效应（Algebraic Effects），在底层的内存布局、调用约定（ABI）和运行期性能开销上，到底有什么本质区别？**
- 为什么 Rust 的 `async` 会带来“函数染色（Function Coloring）”和痛苦的 `Pin` 自引用生命周期难题？
- 为什么 C++20 协程每次调用默认都要走一次 `operator new` 堆内存分配（除非编译器 HALO 优化侥幸生效）？
- 而 Flurry 又是如何通过 **五级效应 ABI（错误结果 $\to$ Direct Passing $\to$ 普通直接处理 / `inline effect` $\to$ 池化限定 Continuation $\to$ Async 子工作）**，做到“按需付费（Pay Only for What You Use）”的？

下一篇 **《Flurry 演进手记 (169)：对比 C++20 Coroutines、Rust Async 与 Flurry 代数效应的底层开销》**，我们将站在系统架构师的制高点，来一场酣畅淋漓的底层 ABI 大对决！
