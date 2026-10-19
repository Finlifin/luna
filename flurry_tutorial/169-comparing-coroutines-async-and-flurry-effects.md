# Flurry 演进手记 (169)：对比 C++20 Coroutines、Rust Async 与 Flurry 代数效应的底层开销

> **系列导读**：在经历了近二十篇对恢复性错误、代数效应、界限延续与结构化异步的深度探索后，几乎每一位系统级程序员心中都会浮现出同一个终极灵魂拷问：**“代数效应听起来像魔法一样通用，但它的底层开销到底有多大？它和 C++20 的无栈协程（Stackless Coroutines）、Rust 的 `async`/`await` 状态机相比，究竟是如何取舍的？”**
>
> 这一篇，我们将彻底掀开编译器后端的底牌，沿着 [`docs/algebraic-effect-and-delimited-continuation/effect-abi.md`](../docs/algebraic-effect-and-delimited-continuation/effect-abi.md) 的五层分级 ABI（Tiered Effect ABI），用硬核的内存布局图与指令路径对比 C++20、Rust 与 Flurry 的底层代价。

---

## 一、 痛点：为什么工业界在“无栈状态机”与“有栈协程”之间反复横跳？

在现代系统编程语言中，每当我们需要让一段计算“在中途暂停、稍后恢复”，语言设计者通常只能在两条看似水火不容的道路之间二选一：

### 路线 A：C++20 无栈协程（Per-Frame Heap Allocation + HALO 赌博）

C++20 引入的 `co_await` / `co_yield` 采用了**按帧独立编译的无栈协程（Stackless Coroutines）**。因为编译器在编译单个协程函数时，往往不知道调用方的栈帧有多大，C++20 的默认策略是：**每次调用协程时，在堆上调用 `operator new` 动态分配一块协程帧（Coroutine Frame）**，用来保存跨挂起点的局部变量。

虽然 C++ 标准委员会寄希望于编译器的 **HALO（Heap Allocation eLision Optimization，堆分配消除优化）** 把协程帧内联到调用者栈帧上，但一旦遇到跨编译单元调用、虚函数分发或复杂的生命周期，HALO 就会静默失效，退化为隐式的堆分配！更痛的是，无栈协程只能挂起**直接包含 `co_await` 的那一层函数**——如果你在深层递归的语法分析器第 8 层需要挂起回溯，整条调用链上的每一层函数都必须改写成协程并逐层 `co_await`，引发灾难性的**函数染色（Function Coloring）**与**逐层拆箱转发开销**。

### 路线 B：Rust `async`/`await` 状态机（零堆分配，但付出了 `Pin` 与多层轮询的代价）

Rust 为了实现极致的零默认堆分配，将整棵异步调用树在编译期压平成一个嵌套的巨型状态机枚举（State Machine `enum`）。这确实消除了逐层 `operator new`，但也带来了三个让系统程序员头疼的工程痛点：
1. **自引用结构体与 `Pin<&mut Self>` 心智负担**：一旦你在跨 `.await` 挂起点之间借用了栈上的局部变量（比如 `let buf = [0u8; 64]; let slice = &buf[..]; read(slice).await;`），生成的枚举状态机内部就会包含指向自身字段的指针！为了防止状态机在内存中被 `memcpy` 移动导致悬垂指针，Rust 被迫引入了极其晦涩的 `Pin` 契约。
2. **全链路函数染色（Function Coloring）**：普通高阶函数（如 `Iterator::map`）无法直接传入 `async` 闭包；同步世界与异步世界彻底割裂。
3. **深层调用链的 `poll` 穿透成本**：每次底层 I/O 就绪被唤醒时，执行器都必须从最外层的根 `Future::poll` 开始，沿着嵌套的 `match state` 一层一层往下分发到最深处的叶子节点。

```
【传统方案的两难困境】

  C++20 Stackless Coroutines:
  ┌───────────────┐   operator new   ┌──────────────────────┐
  │ Caller Frame  │ ───────────────► │ Heap Coroutine Frame │ (HALO 失败即堆分配！)
  └───────────────┘                  │  + 仅限单层挂起！     │
                                     └──────────────────────┘

  Rust Async State Machine:
  ┌─────────────────────────────────────────────────────────┐
  │ 巨型嵌套 Enum (OuterFuture -> MidFuture -> LeafFuture)  │
  │  + 跨挂起点局部借用导致自引用结构体 -> 必须引入 Pin<P>！ │
  │  + 强制函数染色，深层唤醒需逐层 poll 分发！              │
  └─────────────────────────────────────────────────────────┘
```

那么，Flurry 的代数效应（`#Eff`）难道是用“所有请求都走重型有栈协程切换”来换取表达力的吗？
**恰恰相反！** 如果每次读取一个配置或打一行日志都要保存寄存器、切换延续栈，那代数效应在系统级编程中根本无法生存。Flurry 的破局之道是：**不把所有控制流都塞进同一种运行时机制，而是根据处理器（Handler）的静态形态，在编译期分层降级为五套截然不同的零开销或低开销 ABI！**

---

## 二、 演进 第一步（`v0`）：把所有机制混为一谈的“伪统一”误区

在许多学术型效应语言中，异常（Exceptions）、环境读取（Reader）、生成器（Yield）和异步（Async）全部被塞进同一个“捕获界限延续（Capture Delimited Continuation）”的运行时通道里。如果我们在直觉中把 Flurry 写成这种“一刀切”的模型（`v0`），就会试图用捕获延续的方式去处理普通业务错误，或者指望错误拦截分支能顺便抓到底层的延续取消信号：

```flurry
-- v0：试图把业务错误 `!Err` 当成代数效应 `#Eff` 或用错误分支拦截延续清理信号
effect AskRadius {
    fn radius() -> F64;
}

type ParseError = enum {
    InvalidNumber,
};

-- ❌ 误区 1：以为 `#AskRadius` 一定会像协程一样分配帧或切栈
-- ❌ 误区 2：以为在调用点用 `! { ... }` 就能拦住外层延续放弃时发出的内部清理信号
fn compute_area_v0() -> !ParseError #AskRadius F64 {
    let r = (AskRadius::radius())! {
        -- ❌ 编译期直接拒绝！
        -- `AskRadius::radius()` 的类型是 `#AskRadius F64`，不包含任何错误行 `!Err`；
        -- 更重要的是，外层 continuation 被丢弃（drop）时触发的隐藏中止通道（Abort Channel）
        -- 只负责沿栈执行 RAII `Drop` 析构，绝不会被用户的 `! { ... }` 错误恢复分支截获！
        any_err => {
            return 0.0;
        },
    };
    return 3.1415926535 * r * r;
}
```

### 为什么 `v0` 被拒绝？第 0 层铁律：`!Err` 与 `#Eff` 拥有彻底正交的 ABI！

正如 [`effect-abi.md`](../docs/algebraic-effect-and-delimited-continuation/effect-abi.md) 第 1 节与 `AGENTS.md` 第 5 条所确立的铁律：
**在 Flurry 中，恢复性错误结果（`!Err T`）与代数效应（`#Eff T`）拥有完全不同的 ABI！**
- **`!Err T`（错误通道）**：走的是纯正的**值返回 ABI（Tagged Value Return / 寄存器标志位）**。`error e` 永远是单向退栈，绝不会携带任何可恢复的挂起帧，因此它连半个字节的证据槽（Evidence Slot）或延续对象都不需要。
- **`#Eff T`（效应通道）**：走的是**隐式证据传递（Evidence Passing）+ 分级处理器降码 ABI**。当捕获了 `continuation` 的外层处理器放弃恢复、直接丢弃 `continuation` 时，底层触发的是一个**对用户源码完全不可见的内部中止通道（Internal Abort Channel）**——它精准地沿着被挂起的栈切片逐帧调用局部变量的 `Drop` 析构函数，直到该处理器定界符（Delimiter）为止，**绝不会穿过或触发沿途任何用户编写的 `expr! { ... }` 错误恢复分支**！

---

## 三、 演进 第二步（`v1`）：不捕获延续的三大零切栈层级（Tier 1 ~ Tier 3）

在日常系统编程中，超过 **85% 的代数效应处理器根本不捕获 `continuation`（即在尾部就地 `resume`）**！针对这部分场景，Flurry 的编译器（[`effect-abi.md`](../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)）提供了三个连半次栈切换、半字节帧分配都不会发生的极速层级：

1. **Tier 1：`^attrs.effects.direct_passing`（纯值直传 ABI）**——处理器安装时一次性求值，直接通过隐式寄存器/栈槽把值传给被调函数，请求点退化为**单条寄存器读取指令**（零分支、零间接调用）。
2. **Tier 2：普通 Direct Handler（证据函数指针直调 ABI）**——省略 `catch continuation` 且在尾部 `resume v` 的处理器，直接编译为接收证据上下文的普通辅助函数（`fn(env, x) -> y`），在当前调用栈上原地执行并返回 `y`（零延续分配、零栈切换）。
3. **Tier 3：`inline effect`（跨边界协同内联 ABI）**——声明为 `inline effect` 的效应，在单态化阶段将处理器代码与被包裹计算体融合成同一个函数栈帧，效应请求直接内联展开为原地表达式或 `match`（零函数调用开销）。

让我们在 `v1` 中同时展示这三层零切栈 ABI 的协同威力：

```flurry
-- v1：Tier 1 (direct_passing) + Tier 2 (Direct Handler) + Tier 3 (inline effect) 零切栈协同
^attrs.effects.direct_passing
effect MathPrecision {
    fn epsilon() -> F64;
}

effect AuditCounter {
    fn record_step(delta: U64) -> Unit;
}

inline effect FastClamp {
    fn clamp(val: F64) -> F64;
}

-- 这是一个同时携带三种效应的高频数值计算内核：
-- 在底层 ABI 中，它既不需要 operator new 分配协程帧，也不需要变成状态机 enum！
fn integrate_step(raw: F64) -> #[MathPrecision, AuditCounter, FastClamp] F64 {
    -- [Tier 1] 直接读取隐式传入的常量槽，0 次函数调用！
    let eps = MathPrecision::epsilon();

    -- [Tier 3] 随 `inline effect` 协同内联为原地的 `if/else` 比较，0 次间接跳转！
    let clamped = FastClamp::clamp(raw + eps);

    -- [Tier 2] 普通 Direct Handler：如同调用一个带隐式 env 指针的普通 C 函数，原地返回！
    AuditCounter::record_step(1);

    return clamped;
}

fn run_fast_kernel() -> F64 {
    let mut total_steps: U64 = 0;

    return {
        integrate_step(0.42) + integrate_step(1.28)
    } handle MathPrecision {
        -- Tier 1：安装时求值一次 `1e-9`，直接裝入寄存器传给被调体
        epsilon() => 0.000000001,
    } handle AuditCounter {
        -- Tier 2：省略 `catch continuation` 且尾部 `resume`，编译为原地普通函数调用
        record_step(delta) => {
            total_steps = total_steps + delta;
            resume ();
        },
    } handle FastClamp {
        -- Tier 3：`inline effect` 在单态化后直接把分支内联进 `integrate_step` 内部
        clamp(val) => {
            if val > 1.0 do {
                resume 1.0;
            } else {
                resume val;
            }
        },
    };
}
```

---

## 四、 演进 第三步（`v2`）：需要真正挂起时的杀手锏——池化定界栈切片与地址稳定性（Tier 4 & Tier 5）

那么，当我们确实需要像协程、回溯搜索或异步 I/O 那样**在中途挂起计算、稍后恢复**（即捕获 `catch continuation: builtin.Continuation` 或使用 `async effect`）时，Flurry 的开销又是怎样的呢？

这就是 Flurry 相比 C++20 和 Rust `async` 最具颠覆性的架构设计——**基于显式池（`.use(pool)`）的多提示符定界栈切片（Pooled Multi-Prompt Delimited Stack Slices）**：

1. **无需 `Pin` 的原生地址稳定性（Address Stability Across Suspension）**：
   在 Rust `async` 中，挂起点前后的局部变量之所以不能随意互相借用，是因为状态机 `enum` 在未 `Pin` 之前随时可能被按值移动（`memcpy`）。而在 Flurry 的 Tier 4（Capturing Handler）与 Tier 5（`async effect`）中，被定界符包裹的计算体运行在由 `.use(pool)` 分配的**独立定界栈切片（Stack Slice / Contiguous Segment）**上。当计算在中途挂起（Yield / Await）时，所谓“捕获 `continuation`”仅仅是**移交该栈切片的头尾提示符（Prompt Bounds）与寄存器上下文控制权**，**栈切片上的任何局部变量连一个字节都不会在内存中发生搬移（No `memcpy`）**！因此，你在挂起点之前获取的局部引用（`.ref`），在挂起恢复之后天然保持绝对的物理地址稳定，**彻底消灭了自引用状态机和 `Pin<&mut Self>` 的存在必要**！
2. **显式 `.use(pool)` 消除隐式 `operator new`**：
   任何捕获 `continuation` 的 `handle` 块，都必须显式挂载 `.use(pool)`。你可以传入一个在栈上预分配好的固定容量 `StackPool` 或线程局部的 Arena 池，让挂起/恢复变成**纳秒级的指针交换与帧切片复用**，绝不会像 C++20 协程那样在背后偷偷调用全局堆分配器。
3. **一次性仿射延续（One-Shot Affine Continuation）与尾恢复保证（Tail-Resume Guarantee）**：
   `builtin.Continuation<Resume, Out, E, Eff>` 是严格的不可拷贝（Non-`Copy`）仿射资源，每次挂起最多只能消费一次（`resume` 一次或随作用域析构放弃），因此**恢复时永远不需要深拷贝栈帧（Zero Frame Cloning）**。且当处理器在最后一步执行 `resume val` 时，编译器强制实施**尾恢复优化（Tail-Resume Guarantee）**——直接跳回被恢复的栈切片，不在处理器一侧残留任何等待返回的多余调用帧！

让我们在 `v2` 中编写一个在挂起点前后安全持有局部缓冲区引用（无需任何 `Pin` 包装！）的零堆分配回溯/挂起管线，并观察 C ABI 屏障的安全保护：

```flurry
-- v2：Tier 4 池化定界延续 —— 跨挂起点天然保持局部地址稳定，无需 Pin！
effect YieldSlice {
    fn inspect_window(slice_first: I32, addr_check: I32.ref) -> Bool;
}

-- 注意看：这个函数完全是一个普通的 Flurry 函数（只多了 `#YieldSlice` 效应行），
-- 它在跨 `YieldSlice::inspect_window` 挂起点前后，直接持有栈上局部变量 `local_anchor` 的引用！
fn stateful_scanner() -> #YieldSlice I32 {
    let local_anchor: I32 = 100;
    -- 获取栈上局部变量的引用 `.ref`
    let anchor_ref: I32.ref = local_anchor.ref;

    -- 第一次挂起：将 `anchor_ref` 传给处理器检验；
    -- 此时整个定界栈切片原地冻结，没有任何字节被移动，无需 Rust 式的 Pin！
    let keep_going = YieldSlice::inspect_window(1, anchor_ref);
    return anchor_ref.* when not keep_going;

    -- 挂起恢复后，`anchor_ref.*` 依然安全指向同一个物理栈槽！
    let second_ok = YieldSlice::inspect_window(2, anchor_ref);
    return anchor_ref.* + 50 when second_ok;

    return 0;
}

fn run_pooled_continuation(pool: ContinuationPool.mut_ref) -> I32 {
    return {
        stateful_scanner()
    } handle YieldSlice.use(pool) {
        inspect_window(step, addr_check) catch continuation: builtin.Continuation => {
            -- 处理器可以检查传入的局部引用值
            if step == 1 and addr_check.* == 100 do {
                -- 尾位置恢复（Tail-Resume）：零栈拷贝直接切回定界栈切片！
                resume true with continuation;
            } else {
                -- 当我们决定不再恢复、让 `continuation` 在作用域末尾自动析构时，
                -- 隐藏的 Abort Channel 会沿着该定界栈切片精确执行所有活跃局部变量的 Drop！
                return 999;
            }
        },
    };
}
```

### 跨越外部 C ABI 栈帧时的安全铁律（Foreign C-Frame Barrier）

既然被捕获的 `continuation` 允许在深层调用栈中随时挂起或被提前丢弃（Abandon），那如果调用链中间夹着一层用 C 语言编写的外部函数（比如 `qsort` 的回调）会怎样？
[`effect-abi.md`](../docs/algebraic-effect-and-delimited-continuation/effect-abi.md) 第 5.4 节给出了极其严谨的系统级安全边界：
- 如果外层是一个**不捕获 `continuation` 的 Direct Handler（Tier 1~3）**，穿过 C 栈帧调用它是**完全合法的**，因为它本质上只是一次原地执行的普通函数调用。
- 但如果外层是一个**捕获了 `continuation` 的处理器（Tier 4/5）**，任何试图**跨越中间的外部 C ABI 栈帧**捕获延续或丢弃延续触发析构回退的行为，都会被运行时的 **C-Frame 定界屏障（Foreign Frame Barrier）** 立即拦截并触发确定性的安全 Trap（或者在编译期对已知的 `^attrs.abi("C")` 边界直接拒绝）！绝不允许像 C 语言 `longjmp` 那样把外部 C 库的半途状态搅成内存腐败。

---

## 五、 Code Cartoon：C++20 / Rust Async / Flurry 五层效应 ABI 全景对决

让我们把这三种现代控制流模型的底层执行路径放在同一张架构图下直观对比：

```
=============================================================================
 1. C++20 无栈协程 (co_await / co_yield)
=============================================================================
  [Caller] ──(operator new 堆分配帧)──► [Heap Frame] ──(挂起仅限单层)──► 返回句柄
  * 缺陷：HALO 一旦失败即产生隐式堆分配；深层挂起必须逐层改写 co_await（函数染色）。

=============================================================================
 2. Rust Async 状态机 (async / .await)
=============================================================================
  [Outer Poll] ──► match state ──► [Mid Poll] ──► match state ──► [Leaf Poll]
  * 优势：编译期压平为单一 Enum，默认零堆分配。
  * 缺陷：跨挂起点借用局部变量会产生自引用结构体，被迫引入 Pin<&mut Self>；
          同步与异步生态分裂（Function Coloring）；深层唤醒需逐层 poll 分发。

=============================================================================
 3. Flurry 分级代数效应 ABI (Tiered Effect ABI)
=============================================================================
  ┌─ Tier 0: 错误返回 (!Err T)
  │    └──► 独立的值返回 ABI (Tagged Return)，0 证据槽、0 切栈，与效应通道完全正交！
  │
  ├─ Tier 1: ^attrs.effects.direct_passing (只读环境/追踪上下文)
  │    └──► 安装时求值 1 次，隐式寄存器直传，请求点 = 单条读取指令 (0 分支/0 调用)！
  │
  ├─ Tier 2: 普通 Direct Handler (省略 catch continuation 且尾部 resume)
  │    └──► 编译为带 env 指针的普通函数直调 (fn(env, req) -> resp)，0 切栈、0 帧分配！
  │
  ├─ Tier 3: inline effect (高频内联策略钩子)
  │    └──► 单态化阶段将 Handler 与计算体协同内联，请求点直接展开为原地表达式/match！
  │
  ├─ Tier 4: Capturing Handler (catch continuation + .use(pool))
  │    └──► 显式池化定界栈切片 (Pooled Stack Slice)：
  │         • 挂起仅交换 Prompt 边界与寄存器，局部变量物理地址绝对稳定 -> 无需 Pin！
  │         • 仿射一次性延续 (One-Shot Affine) -> 恢复时 0 栈拷贝，尾部 resume 0 残留帧！
  │         • 放弃恢复时走隐藏 Abort Channel 精确触发 RAII Drop，严守 C-Frame 屏障！
  │
  └─ Tier 5: async effect (结构化异步 + .use(runtime) + .await)
       └──► 请求点注册子任务并立即返回 Future<T>；
            • 若已就绪：.await 走快路径原地取值（0 挂起）！
            • 若未就绪：复用 runtime 的池化定界延续挂起当前任务，无全链路 async 染色！
```

> **💡 核心心智模型**：
> 在 Flurry 中，**“写得像通用代数效应，跑得像手写定制状态机”** 不是一句口号，而是由编译器静态分析处理器形态后精确兑现的五级降码契约：你不需要自己去给函数涂上红蓝颜色，也不需要手写 `Pin` 投影——只要你的处理器没有捕获 `continuation`，你就自动获得了比拟手写回调的零切栈性能；而当你真正需要跨多层调用栈挂起时，`.use(pool)` 的定界栈切片又为你保住了局部引用的物理地址稳定性与确定性的资源预算！

---

## 本篇小结

| 对比维度 | C++20 无栈协程 (`co_await`) | Rust 状态机 (`async`/`await`) | Flurry 分级代数效应 (`#Eff` + Tiered ABI) |
| :--- | :--- | :--- | :--- |
| **只读上下文 / 策略注入** | 需额外传参或 TLS | 需手动传参或 `task_local!` | **Tier 1 (`direct_passing`) / Tier 3 (`inline effect`)**：零切栈、零间接调用开销 |
| **就地恢复的事件处理** | 若用协程则触发帧分配 | 若用 `async` 则状态机膨胀 | **Tier 2（Direct Handler）**：降级为普通函数直调，零栈切换 |
| **深层调用链挂起（回溯/协程）** | ❌ 只能单层挂起，必须逐层 `co_await` | ❌ 必须整条调用链全部标为 `async` | ✅ **多提示符定界延续**：任意深度直接挂起至目标 Handler，中间层零感知 |
| **跨挂起点持有局部引用** | 靠堆分配协程帧维持地址 | ❌ 产生自引用枚举，必须使用 `Pin<&mut Self>` | ✅ **定界栈切片原地冻结**：局部变量的 `.ref` 地址天然稳定，**无需 `Pin`** |
| **挂起帧内存分配策略** | 默认隐式 `operator new`（赌 HALO） | 编译期压平为巨型嵌套 `enum` | **强制显式 `.use(pool)` / `.use(runtime)`**：由调用者掌控池化预算，拒绝隐式堆分配 |
| **放弃挂起计算时的清理** | 需手动调用 `coroutine_handle::destroy()` | 直接 `drop(future)` 析构状态机 | `continuation` 离开作用域自动通过**隐藏 Abort Channel** 沿栈切片执行 `Drop` |

---

### 留给下一篇的悬念

到这里，我们已经把错误传递、代数效应、界限延续、异步并发和底层的五级 ABI 全部拆解完毕了！
回望整个第七卷（第 141 篇至第 169 篇），从最基础的 `if`/`while`/`match`、语句后置守卫 `when`、循环条件恢复 `resume ... while`，到唯一标签错误行 `!Err`、作用域多重集效应行 `#Eff`、结构化异步 `async effect`，再到由 `Transactional` 纯度契约护航的软件事务内存（STM `atomic` / `retry` / `or_else`）——你会发现这原本在其他语言中支离破碎的七八套控制流机制，在 Flurry 中竟然拼成了一张严丝合缝的统一蓝图！

在第七卷的收官之作（第 170 篇）中，我们将站在全卷制高点，为你绘制完整的**《Flurry 统一控制流全景图与架构选型指南》**，并正式揭开最后一卷——**第八卷：异构硬件投影（`~>`）与 Luna 编译器内部架构**的序幕！
