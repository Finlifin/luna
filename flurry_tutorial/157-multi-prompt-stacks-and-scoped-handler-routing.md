# Flurry 演进手记 (157)：在多提示符（Multi-Prompt）栈上精准导航——嵌套 Handler 的路由与标签隔离

在早期的 Scheme 语言里，有一个让无数程序员既敬畏又头疼的控制流怪兽——`call/cc`（未限定 Continuation）。
为什么 `call/cc` 难以在系统级工程中使用？因为它捕获的是**从当前点一直到整个进程 `main` 函数最底部的全部调用栈**！你没法告诉它：“我只想切下从当前点到外围第 2 层循环之间的那几帧局部栈，别动更外层的网络服务器栈帧！”

而在现代编程语言理论中，代数效应之所以能够把生成器（Generator）、协程（Coroutine）、异常早退与回溯搜索大一统，核心就在于它建立在**多提示符限定 Continuation（Multi-Prompt Delimited Continuations）**之上！

想象一下，你在读书时往一本书的不同页码里夹了三张不同颜色的**书签（提示符 Prompt / 限定边界）**：
- 第 10 页夹了**红书签**（外层边界 `H_outer`）；
- 第 30 页夹了**蓝书签**（内层边界 `H_inner`）；
- 现在你读到了第 50 页！
  - 当你触发属于蓝书签的效应时，运行时只精准切下**第 30 页到第 50 页**这 20 页纸；
  - 当你跨越蓝书签、直接触发属于红书签的外层效应时，运行时会切下**第 10 页到第 50 页**（中间第 30 页的蓝书签也完好无损地夹在这叠纸中间！），等你恢复（`resume`）执行时，中间的第 30 页蓝书签依然在原位生效！

今天，我们就深入 `docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md` 与 `docs/type/rows.md`，用直观的“多层书签栈帧” **Code Cartoon**，彻底看透多提示符栈上的精准切片、Deep 恢复、尾恢复切换与放弃清理机制！

---

## 尝试 v0：单层栈切换无法应对的“多层控制流叠加”

在实际的物联网网关数据流管线中，控制流从来不是单层的：
比如我们有一个传感器数据流生成器 `produce()`，它在内部不断通过 `yield_number(value)#` 向外吐出采样值；而在生成器的内部，它还可能需要通过另一个效应 `read_calibration()#` 向更外层的配置中心查询校准系数，同时在栈帧上还挂着一个需要在退出时自动释放硬件句柄的 `ProducerTrace`（实现了 `Drop`）！

如果运行时只能“一刀切”地挂起整个线程栈，或者在跨越内层 Handler 捕获外层 Continuation 时把内层 Handler 给弄丢了，整个多层管线就会瞬间崩溃。

---

## 演进 v1：理解“书签切片”——捕获到指定边界、Deep 恢复与三次挂起三个恢复权！

在 Flurry 中，当你声明一个带 `catch continuation: builtin.Continuation` 的效应时：

```flurry
effect yield_number(value: u32, catch continuation: builtin.Continuation);
```
请注意语法细节（来自 `multi-prompt-delimited-continuation.md`）：
- **在 `effect` 声明处**：使用 `catch continuation: builtin.Continuation` 授予捕获剩余计算的许可；请求者在调用 `yield_number(value)#` 时**只需传入普通实参 `value`**，不需要手动传 `continuation`！
- **在 `# { ... }` 处理分支处**：直接写普通参数模式 `yield_number(value, continuation) => { ... }` 接收控制协议交付的 `continuation`（分支模式中**不再重复写 `catch` 关键字**）！

让我们用一张 **Code Cartoon** 看看当内层计算跨越内层边界 `H_inner` 向外层边界 `H_outer` 发起捕获请求，随后执行 **`resume`** 时，多提示符栈上到底发生了什么：

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：多提示符（Multi-Prompt）栈切片与 Deep 恢复重装        |
|                                                                         |
|  【挂起前：调用栈上有两个嵌套的书签边界（Prompts）】                    |
|   [ S0: 主程序外围栈 ]                                                  |
|     ├── 🔖 红书签：安装外层 H_outer (处理 ask_outer)                    |
|     │     [ S1: H_outer 主体栈帧 ]                                      |
|     │       ├── 📘 蓝书签：安装内层 H_inner (处理 yield_number)         |
|     │       │     [ S2: H_inner 主体栈帧 ]                              |
|     │       │       └── ⚡ 发起 `ask_outer()#` 请求！                   |
|                                                                         |
|  【切片捕获：精准切到红书签 H_outer 边界为止！】                        |
|   • 主程序回到 `[ S0 ]`（在红书签边界之外执行 `H_outer` 的处理分支！）  |
|   • 捕获到的限定 Continuation `K` 内部完整保存了：                      |
|     `[ S1 ] ──► 📘 蓝书签 (H_inner) ──► [ S2 (停在 ask_outer 处) ]`     |
|                                                                         |
|  【Deep 恢复：当 H_outer 分支执行 `resume` 时】                         |
|   • 将切片 `K` 重新接回当前恢复点！                                     |
|   • 夹在 `K` 肚子里的 **📘 蓝书签 (`H_inner`) 自动随恢复原封不动重装**！|
|   • 后续 `[ S2 ]` 继续发起 `yield_number()#` 时，依然精准命中蓝书签！✅ |
+-------------------------------------------------------------------------+
```

接下来，让我们通过官方规范中的 `produce()` 示例，观察两个极其重要的生命周期特性：
1. **每次挂起产生一个全新的恢复权（Single-Shot Resumption）**：循环 3 次调用 `yield_number(value)#`，就会先后交出 3 个独立的 `continuation`；每次 `resume;` 都会**消费掉当前这一份恢复权**，并以**尾切换（Tail Switch）**的方式直接切回挂起点，绝不在栈上堆积无用的等待帧！
2. **跨越多次挂起的局部资源存活，以及不恢复时的自动放弃清理（Abandonment Cleanup）**：

```flurry
effect yield_number(value: u32, catch continuation: builtin.Continuation);

struct ProducerTrace {}

impl Drop for ProducerTrace {
    fn drop(*self) { println("cleanup producer"); }
}

fn produce() -> #[yield_number] void {
    -- trace 在进入循环前创建，跨越三次挂起与恢复一直保持存活！
    let trace = ProducerTrace {}
    for value in 0u32..3u32 {
        yield_number(value)#;
    }
    println("finished");
}

test {
    -- 场景 A：每次收到值都执行 `resume;` 恢复剩余计算
    -- 依次输出：0、1、2、finished、cleanup producer
    (produce()# {
        yield_number(value, continuation) => {
            println("{}", value);
            resume;
        },
    }).use(pool);
}
```

如果我们在 Handler 分支里**只想要第 1 个值（`0`）**，拿到之后**故意不写 `resume;`** 直接结束分支呢？

```flurry
test {
    -- 场景 B：只接受第一个值，不执行 resume！
    -- 依次输出：first: 0、cleanup producer（注意：finished 不会执行！）
    (produce()# {
        yield_number(value, continuation) => {
            println("first: {}", value);
            -- 未恢复也未转交 continuation！
            -- 离开分支范围时，自动触发放弃协议（Abandonment Protocol）：
            -- 沿隐藏中止通道进入挂起栈，精准执行 `trace` 的 Drop 输出 "cleanup producer"，
            -- 并在限定边界处停止，然后再将栈槽归还给 pool！
        },
    }).use(pool);
}
```
看到这一幕，你一定能感受到 Flurry 资源管理与控制流结合的深厚功力：**哪怕你在半路上直接掐断了一个生成器，挂起栈里已经构造好的局部对象 `trace` 也绝对会被一分不差地析构清理，绝不因为放弃 Continuation 而泄漏任何一滴资源！**

---

## 演进 v2：区分“三种结果类型（$Y, A, B$）”与“尾恢复 vs 非尾恢复”

在深入编写自定义调度器之前，我们还需要看懂限定 Continuation 内部最核心的三个类型角色（见 `multi-prompt-delimited-continuation.md`）：
- **$Y$（操作结果 Operation Result）**：效应操作本身声明返回给请求点的类型（例如 `effect ask(...) -> u32;` 中的 `u32`）；
- **$A$（主体结果 Body Result）**：被包裹在 `# { ... }` 里的主计算正常跑完时的结果类型；
- **$B$（整体处理结果 Overall Handler Result）**：整个 `expr# { ... }` 表达式最终交付给外围的结果类型。

在绝大多数日常场景中，我们使用 **`resume expr;`** 语句：它把操作结果 $Y$ 交还给挂起点，同时**立即结束当前 Handler 分支**（实行**尾恢复 Tail Resumption**，不保留当前分支的栈帧）。

但如果你需要在恢复被捕获的计算之后，**等整個主体算完拿到结果 $B$，再在当前 Handler 分支里对结果做进一步加工（比如 `+ 1u32`）**——这就叫做**非尾恢复（Non-Tail Resumption）**！
在底层（需配合控制后端提供的 `unsafe` 原始显式恢复适配接口），非尾恢复会保留当前正在等待的分支栈帧：

```flurry
-- 底层控制后端提供的非尾恢复适配声明（消费恢复权、交付 u32，并等待返回整体 u32 结果）
unsafe fn resume_u32(continuation: builtin.Continuation, value: u32) -> u32;

effect ask(catch continuation: builtin.Continuation) -> u32;

fn twice_answer() -> #[ask] u32 {
    let answer = ask()#;
    answer * 2u32
}

test {
    let result = (twice_answer()# {
        ask(continuation) => unsafe {
            -- 非尾恢复：向 ask()# 交付 6u32 ──► twice_answer 算出 6 * 2 = 12u32
            -- ──► 回到本分支继续执行 12u32 + 1u32 = 13u32！
            let resumed = resume_u32(continuation, 6u32);
            resumed + 1u32
        },
    }).use(pool);

    -- 输出：13
    println("{}", result);
}
```

---

## 编译期安全红线：同名效应层次隔离，且 Continuation 严禁二次消费（Single-Shot Only）！

在多提示符栈上导航时，编译器为你守住了两条绝对不可逾越的静态红线：

### 红线 1：`builtin.Continuation` 既非 `Copy` 也无 `Clone`，严禁恢复两次！
为什么 Flurry 坚持**单次恢复（Single-Shot / Affine Continuation）**？
因为如果允许把同一个挂起栈帧 `continuation` 恢复两次，栈帧里持有的唯一仿射资源（比如 `ProducerTrace` 或文件锁）就会被凭空复制成两份并析构两次（Double Free）！
因此，每个 `continuation` 都是严格的仿射（Once）资源：一旦执行了 `resume`（或者把它移交进了任务队列），原绑定就已被消费，试图再次恢复或放弃它会被编译器当场拦截！

```flurry
effect step(catch continuation: builtin.Continuation);

fn bad_double_resume(work: fn() -> #[step] void) {
    (work()# {
        step(continuation) => {
            resume;
            -- ❌ 编译期直接拒绝！
            -- `resume` 已经消费了 `continuation` 并结束了当前分支控制流，
            -- 恢复权不可复制（!Copy / !Clone），严禁对同一 continuation 二次消费！
            resume;
        },
    }).use(pool);
}
```

### 红线 2：同名嵌套效应的层次（Occurrence）不可混淆！
正如我们在第 109 篇学过的，`#[read_number] #[read_number] u32` 代表栈上有两层嵌套的同名提示符（内层 Prompt 与外层 Prompt）。编译器在路由和重排不同效应标签时，如果两个符号效应标签尚未证明不相等，绝不允许随意交换它们的顺序，从而保证每一次请求都 100% 命中对应层次的那个 Prompt！

---

## 本篇小结

| 核心概念 | 运作机制与规则 | 工程与安全价值 |
| :--- | :--- | :--- |
| **多提示符限定边界** | 捕获仅从请求点切到目标 Handler 安装边界（Prompt），不触碰更外层栈帧 | 支持多层控制流（如事务 + 超时 + 生成器）在同一个线程栈上自由嵌套 |
| **Deep 恢复重装** | `resume` 时自动重装被捕获片段内部的嵌套 Handler，外层入口由恢复点供应 | 内外层效应各司其职，跨层捕获绝不会破坏内层已安装的处理器 |
| **单次恢复与自动放弃** | `builtin.Continuation` 仅可消费一次；未恢复且未转交时自动走隐藏中止通道执行 `Drop` | 杜绝栈帧克隆导致的 Double-Free，确保早退/中断生成器时局部资源零泄漏 |
| **尾恢复 vs 非尾恢复** | `resume` 语句执行尾切换（零残留等待帧）；显式非尾恢复保留后继帧等待结果 $B$ | 日常协程/生成器迭代拥有常数级栈开销，同时保留底层非尾后处理能力 |

---

### 留给下一篇的悬念

思考一个极具挑战性的场景：
假设我们在一个 `# { ask() => 42u32 }` 效应处理块**内部**，定义了一个闭包 `let getter = || ask()#;`，然后我们试图把这个闭包 `getter` **作为返回值 `return` 到 `# { ... }` 处理块的外面去调用**！

等一下！当代码走出 `# { ... }` 块之后，那个负责回答 `ask()` 请求的 Handler（提示符边界）**已经从栈上卸载销毁了**！如果允许在外面调用 `getter()`，它发起的 `ask()#` 去找谁要结果？！
反过来，如果一个闭包捕获了外层的局部变量指针或单次资源 `Ticket`，在调用一次之后它的环境状态变成了什么样？

下一篇 **`158-capturing-effect-handlers-in-closures-safely.md`**，我们将深入 `docs/functions/closure-environments.md` 与 `handling-capability.md`，揭秘 Flurry 是如何通过 **`%C` 闭包环境限定、可达性追踪与效应行约束**，彻底封死“闭包携带已卸载 Handler 或悬垂引用逃逸”的漏洞的！
