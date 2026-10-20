# Flurry 演进手记 (169)：对比无栈协程、异步状态机与 Flurry 代数效应的执行模型

> **阅读指南**：在探讨了恢复性错误、代数效应、界限延续与结构化异步之后，一个很自然的工程问题是：**代数效应的运行时开销究竟如何？它与无栈协程（Stackless Coroutines）以及 `async`/`await` 状态机相比是如何取舍的？**
>
> 本篇沿着 [`docs/algebraic-effect-and-delimited-continuation/effect-abi.md`](../docs/algebraic-effect-and-delimited-continuation/effect-abi.md) 的分级执行模型，通过内存布局与调用路径对比不同方案的工程权衡。

---

## 一、 挂起与恢复机制的常见设计路线

当一段计算需要“在中途暂停、稍后恢复”时，系统编程语言通常有两类主流方案：

### 路线 A：按帧分配的无栈协程

按帧独立编译的无栈协程在编译单个协程函数时往往不知道调用方的完整上下文，默认策略是在进入协程时动态分配一块协程帧（Coroutine Frame）来保存跨挂起点的局部变量。
虽然编译器可以尝试通过堆分配消除优化（HALO）将协程帧内联到调用者栈帧中，但在跨编译单元或间接调用场景下往往回退为堆分配。此外，无栈协程只能挂起直接包含挂起点的单层函数；如果深层调用链需要挂起，沿途每一层函数都必须显式改写并逐层转发。

### 路线 B：编译期状态机展开

将整棵异步调用树在编译期压平为嵌套的状态机枚举，可以避免逐层堆分配，但也会带来几项工程代价：
1. **跨挂起点局部借用与自引用约束**：一旦跨 `.await` 挂起点持有局部变量的指针，生成的状态机内部就会包含自引用指针，需要额外的固定位置约束（如 `Pin`）来防止按值移动破坏内部指针。
2. **同步与异步接口分离**：普通高阶函数难以直接复用异步回调，往往需要维护两套接口。
3. **多层状态分发**：深层挂起被唤醒时，需要从外层状态机逐层匹配分支向下恢复。

```
【不同方案的布局与分发模型】

  按帧无栈协程:
  ┌───────────────┐   分配协程帧     ┌──────────────────────┐
  │ Caller Frame  │ ───────────────► │ Coroutine Frame      │
  └───────────────┘                  │  + 仅限单层挂起      │
                                     └──────────────────────┘

  嵌套状态机枚举:
  ┌─────────────────────────────────────────────────────────┐
  │ 嵌套状态枚举 (OuterFuture -> MidFuture -> LeafFuture)   │
  │  + 跨挂起点持有局部指针时会形成自引用结构               │
  │  + 唤醒时沿状态分支逐层分发                             │
  └─────────────────────────────────────────────────────────┘
```

Flurry 的代数效应（`#Eff`）并没有把所有控制流都放进同一种重型切换机制，而是根据处理器（Handler）的形态，在编译期分别映射到不同的执行路径。

---

## 二、 错误通道与效应通道的正交边界

在讨论效应分级之前，首先要明确恢复性错误（`!Err T`）与代数效应（`#Eff T`）的边界。如果混淆这两条通道，试图用错误处理分支拦截效应请求或延续清理，编译器会直接拒绝：

```flurry
effect ask_radius() -> f64;

enum ParseErr {
    InvalidNumber,
}

-- ❌ 错误示例：混淆 `#ask_radius` 效应请求与 `!ParseErr` 错误处理
fn bad_compute_area() -> !ParseErr #[ask_radius] f64 {
    -- 编译错误：`ask_radius()#` 触发的是效应通道，不产生 `!Err` 错误分支；
    -- 且外层放弃 continuation 时的栈展开清理对业务错误处理分支不可见
    let r = ask_radius()! {
        .InvalidNumber => 0.0f64,
    }
    3.1415926535f64 * r * r
}
```

### `!Err` 与 `#Eff` 的独立 ABI

根据 [`effect-abi.md`](../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)：
- **`!Err T`（错误通道）**：采用值返回 ABI。`error e` 始终是单向退栈，不携带可恢复的挂起帧，因此不需要证据槽或延续对象。
- **`#Eff T`（效应通道）**：采用证据传递（Evidence Passing）与分级处理器机制。当捕获了 `continuation` 的外层处理器放弃恢复、销毁 `continuation` 时，底层沿着被挂起的栈切片逆序执行局部变量的 `Drop` 析构，直到处理器定界符为止，不会进入用户编写的 `! { ... }` 错误处理分支。

---

## 三、 不捕获延续的三类直接执行路径

在很多实际场景中，效应处理器并不捕获 `continuation`，而是在处理分支末尾直接 `resume`。针对这类场景，Flurry 提供了三类不涉及栈切换的执行路径：

1. **`^attrs.effects.direct_passing`（纯值直传）**：处理器安装时求值一次，通过隐式参数槽将值传给被包裹计算，请求点直接读取该值。
2. **普通直接处理器（Direct Handler）**：不声明 `catch continuation` 且在分支内直接 `resume` 的处理器，编译为带上下文指针的普通函数调用，在当前调用栈上原地执行并返回。
3. **`inline effect`（内联效应）**：声明为 `inline effect` 的效应，在编译期将处理器分支与被包裹计算协同内联，消除函数调用边界。

下面的示例同时展示了这三类零切栈效应的组合使用：

```flurry
use std.prelude.*;

^attrs.effects.direct_passing
effect math_epsilon() -> f64;

effect record_step(delta: u64);

inline effect fast_clamp(val: f64) -> f64;

-- 同时使用三种直接效应的数值计算函数
fn integrate_step(raw: f64) -> #[math_epsilon, record_step, fast_clamp] f64 {
    -- 1. direct_passing：直接读取隐式传入的阈值
    let eps = math_epsilon()#;

    -- 2. inline effect：与外层处理器协同内联
    let clamped = fast_clamp(raw + eps)#;

    -- 3. 普通 Direct Handler：按普通函数调用原地执行并返回
    record_step(1u64)#;

    clamped
}

fn run_fast_kernel() -> f64 {
    var total_steps = 0u64;

    let first = integrate_step(0.42f64)# {
        math_epsilon() => 0.000000001f64,
        record_step(delta) => {
            total_steps += delta;
        },
        fast_clamp(val) => {
            resume 1.0f64 while val > 1.0f64;
            val
        },
    }

    let second = integrate_step(1.28f64)# {
        math_epsilon() => 0.000000001f64,
        record_step(delta) => {
            total_steps += delta;
        },
        fast_clamp(val) => {
            resume 1.0f64 while val > 1.0f64;
            val
        },
    }

    first + second
}

test {
    let sum = run_fast_kernel();
    assumes sum > 1.4f64;
}
```

---

## 四、 需要挂起时的定界延续与局部地址稳定性

当计算确实需要在中途挂起并稍后恢复（即声明 `catch continuation: builtin.Continuation` 或使用 `async effect`）时，Flurry 采用多提示符定界延续（Multi-Prompt Delimited Continuation）：

1. **跨挂起点的局部地址稳定性**：
   被定界符包裹的计算运行在独立的定界栈切片上。当计算在中途挂起时，捕获 `continuation` 仅移交该定界切片的控制权与上下文，切片内的局部变量不会在内存中按值搬移。因此，在挂起点之前取得的局部指针（`.ref`），在挂起恢复后依然指向原位置，不会因为挂起本身变成自引用移动问题。
2. **一次性仿射延续与尾恢复**：
   `builtin.Continuation` 是不可 `Copy` 的一次性资源（One-Shot Affine Resource），每次挂起最多恢复一次，恢复时无需复制栈帧。当处理器在尾部直接 `resume` 时，直接切回被恢复的计算体；而在非尾位置通过 `continuation.resume(...)` 手动恢复并继续在处理器后续代码中依赖外层状态时，则需要显式置于 `unsafe` 块中。
3. **跨 C ABI 边界的定界保护**：
   如果不捕获延续的直接处理器穿过 C ABI 栈帧调用，它只是普通函数调用；但如果试图跨越中间的外部 `^attrs.abi(.c)` 栈帧捕获或放弃定界延续，则会破坏外部 C 栈帧的契约，因此受到严格的边界限制。

下面的示例展示了在挂起点前后安全持有局部变量指针 `.ref` 的定界延续处理：

```flurry
use std.prelude.*;

effect inspect_window(
    step: i32,
    addr_check: *i32,
    catch continuation: builtin.Continuation
) -> bool;

fn stateful_scanner() -> #[inspect_window] i32 {
    var local_anchor = 100i32;
    let anchor_ptr: *i32 = local_anchor.ref;

    -- 第一次挂起：定界切片保持原位，anchor_ptr 在恢复后依然有效
    let keep_going = inspect_window(1i32, anchor_ptr)#;
    return anchor_ptr.* while not keep_going;

    -- 修改局部变量并通过原指针观察更新
    local_anchor = 120i32;
    let second_ok = inspect_window(2i32, anchor_ptr)#;
    return anchor_ptr.* + 50i32 while second_ok;

    0i32
}

fn run_scanner() -> i32 {
    stateful_scanner()# {
        inspect_window(step, addr_check, catch continuation) => {
            if step == 1i32 and addr_check.* == 100i32 {
                -- 在非尾位置通过 continuation.resume 显式恢复需要 unsafe
                let next_res = unsafe { continuation.resume(true) }
                next_res
            } else {
                -- 不再调用 resume，continuation 离开作用域时自动清理挂起切片上的局部资源
                999i32
            }
        },
    }
}

test {
    assumes run_scanner() == 999i32;
}
```

---

## 五、 分级执行模型全览

把不同控制流机制的执行路径放在一起对比：

```
=============================================================================
 1. 按帧无栈协程
=============================================================================
  [Caller] ──(分配协程帧)──► [Coroutine Frame] ──(单层挂起)──► 返回句柄

=============================================================================
 2. 异步状态机展开
=============================================================================
  [Outer Poll] ──► match state ──► [Mid Poll] ──► match state ──► [Leaf Poll]

=============================================================================
 3. Flurry 分级错误与效应模型
=============================================================================
  ┌─ 错误返回 (!Err T)
  │    └──► 独立的值返回通道，不涉及证据槽或切栈
  │
  ├─ ^attrs.effects.direct_passing (只读上下文)
  │    └──► 安装时求值一次，隐式参数直传，请求点直接读取
  │
  ├─ 普通 Direct Handler (不捕获 continuation 且直接 resume)
  │    └──► 编译为带上下文指针的普通函数调用，原地执行返回
  │
  ├─ inline effect (内联策略钩子)
  │    └──► 将处理器分支与被包裹计算体协同内联
  │
  ├─ Capturing Handler (catch continuation)
  │    └──► 定界延续切片：
  │         • 局部变量在挂起期间保持原位，指针 `.ref` 地址稳定
  │         • 一次性仿射延续，恢复时无需复制栈帧
  │         • 放弃恢复时自动沿挂起切片逆序执行局部 Drop
  │
  └─ async effect (结构化异步 + .use(runtime) + .await)
       └──► 请求点返回 Future<T>，在 .await 处按需挂起
```

---

## 本篇小结

| 对比维度 | 按帧无栈协程 | 状态机展开 (`async`/`await`) | Flurry 分级代数效应 (`#Eff`) |
| :--- | :--- | :--- | :--- |
| **只读上下文注入** | 需额外传参或线程局部存储 | 需手动传参或任务局部存储 | **`direct_passing` / `inline effect`**：隐式直传或协同内联 |
| **就地恢复的事件处理** | 若用协程则涉及协程帧 | 若用 `async` 则并入状态机 | **Direct Handler**：编译为普通函数调用，无栈切换 |
| **深层调用链挂起** | 仅限单层挂起，需逐层改写 | 需整条调用链声明 `async` | **多提示符定界延续**：可在深层调用链直接挂起至目标 Handler |
| **跨挂起点持有局部指针** | 依赖协程帧保持地址 | 会形成自引用状态机结构 | **定界切片保持原位**：局部变量的 `.ref` 地址在挂起前后保持稳定 |
| **放弃挂起计算时的清理** | 需显式销毁协程句柄 | 析构状态机枚举 | `continuation` 离开作用域时自动沿挂起切片逆序执行局部 `Drop` |

---

### 下一篇预告

到这里，我们已经把局部结构化控制流、恢复性错误行 `!Err`、代数效应 `#Eff`、定界延续、结构化异步 `async effect` 以及软件事务内存（STM）全部串联起来了。

在第七卷的收官篇（第 170 篇）中，我们将总结整卷的控制流机制，给出工程选型指南，并引出第八卷的异构硬件投影与系统架构实战。
