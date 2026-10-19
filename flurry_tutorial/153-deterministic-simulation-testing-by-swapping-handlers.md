# Flurry 演进手记 (153)：把真实睡眠替换成瞬间快进——用效应 Handler 为异步网络协议编写确定性仿真测试

在分布式系统与物联网边缘网关领域，FoundationDB 和 TigerBeetle 带火了一项让所有基础设施工程师梦寐以求的终极测试技术——**确定性仿真测试（Deterministic Simulation Testing, DST）**。

为什么传统的异步网络单元测试既慢又不可靠（Flaky Tests）？
假设你在写一个传感器网关的**指数退避重试协议（Exponential Backoff Retry）**：
- 第 1 次发送失败后，调用 `sleep_ms(1000)#` 等待 1 秒；
- 第 2 次发送失败后，调用 `sleep_ms(2000)#` 等待 2 秒；
- 第 3 次发送失败后，调用 `sleep_ms(4000)#` 等待 4 秒，如果还失败就触发限流降级。

如果在传统语言里，你在业务代码里直接调了操作系统定时器或绑定了重型异步运行时的 `sleep`，每次跑这一个单测就要真刀真枪地干等 **7 秒钟**！要是想为了 make sure 稳定性跑 10,000 次随机丢包仿真，就得等上整整两天！
更头疼的是，在传统语言里想把时间、网络和错误处理抽成可替换的 Mock 接口，你得在每个函数的参数列表里层层传递 `clock: &mut dyn Clock, net: &mut dyn Network, err_policy: &dyn ErrorPolicy`——业务代码还没写两行，函数签名已经被依赖注入（Dependency Injection）的脚手架撑爆了！

而在 Flurry 中，**代数效应（Algebraic Effects）天生就是类型安全、零样板代码的终极依赖注入与仿真引擎！**
今天，我们就结合 `docs/algebraic-effect-and-delimited-continuation/handling-capability.md` 与 `async-algebraic-effect.md`，看看如何不改一行协议代码，用有状态 Handler、`.use(handler)` 与 `handles` 能力绑定，把 7 秒钟的真实睡眠变成**纳秒级的虚拟时间快进**！

---

## 尝试 v0：硬编码系统时钟 vs 笨重的层层参数注入

在没有代数效应之前，我们通常陷入两难：

```flurry
-- 💭 痛点 A：直接调系统真实睡眠，单测慢如蜗牛且不可复现！
-- 💭 痛点 B：把 Clock 和 Net 塞进每一层函数的普通参数里，造成严重的“参数钻孔（Prop Drilling）”！
fn poll_with_backoff_v0(clock: *SimClock, net: *SimNet, max_retries: u32) -> u32 {
    return 0u32;
}
```

而在 Flurry 中，函数只需要在返回类型的效应行（`#[sleep_ms, fetch_sample]`）和错误行（`!LimitErr`）中如实声明它要发出什么请求；至于这个请求在生产环境是走真实的 Linux `epoll` 定时器，还是在 `test` 块里走内存虚拟时钟累加器，完全由外围的安装处决定！

---

## 演进 v1：将有状态仿真对象接入效应——把 7000 毫秒睡眠变成 3 次整数加法！

让我们先看 `docs/algebraic-effect-and-delimited-continuation/handling-capability.md` 中展示的核心模式：**将一个有状态的普通 Trait/结构体对象接入代数效应处理器！**

假设我们的传感器重连协议需要两个效应：
1. `effect sleep_ms(duration: u32);` —— 请求睡眠指定毫秒数；
2. `async effect request_reading(attempt: u32) -> u32;` —— 异步发起第 `attempt` 次传感器读数请求。

在协议业务函数 `fetch_with_backoff` 中，我们只管写出最纯粹的协议逻辑：

```flurry
effect sleep_ms(duration: u32);
async effect request_reading(attempt: u32) -> u32;

-- 纯粹的协议逻辑：
-- 尝试请求读数；若返回 0（代表本次探测超时丢包），则按 1000ms -> 2000ms 退避睡眠后重试！
fn fetch_with_backoff() -> #[sleep_ms, request_reading] u32 {
    let first_fut = request_reading(1u32)#;
    let first_val = first_fut.await;
    if first_val != 0u32 {
        return first_val;
    }

    -- 第 1 次失败，退避睡眠 1000ms
    sleep_ms(1000u32)#;

    let second_fut = request_reading(2u32)#;
    let second_val = second_fut.await;
    if second_val != 0u32 {
        return second_val;
    }

    -- 第 2 次失败，指数退避睡眠 2000ms
    sleep_ms(2000u32)#;

    let third_fut = request_reading(3u32)#;
    return third_fut.await;
}
```

现在，我们要为 `fetch_with_backoff` 编写一个**零真实等待、100% 确定性的仿真测试**！
我们定义一个普通的有状态仿真器结构体 `SimEnv`（记录虚拟流逝时间 `virtual_time_ms` 和模拟在第几次尝试才成功返回数据），并在 `test` 块中通过 `# { ... }` 将效应请求直接连接到 `sim` 的方法上：

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：用有状态仿真器拦截效应请求，瞬间快进虚拟时钟          |
|                                                                         |
|  协议主体 `fetch_with_backoff()`             有状态仿真对象 `var sim`   |
|  ───────────────────────────────             ────────────────────────   |
|  1. request_reading(1)#.await   ──────────►  attempt 1 < 3，返回 0 (丢包)|
|  2. sleep_ms(1000)#             ──────────►  sim.virtual_time += 1000   |
|                                              (耗时 0.000001ms！瞬间快进)|
|  3. request_reading(2)#.await   ──────────►  attempt 2 < 3，返回 0 (丢包)|
|  4. sleep_ms(2000)#             ──────────►  sim.virtual_time += 2000   |
|  5. request_reading(3)#.await   ──────────►  attempt 3 == 3，返回 42！  |
|                                                                         |
|  ✅ 测试结束断言：                                                      |
|     • 最终取得读数 == 42u32                                             |
|     • 虚拟时钟精确流逝 == 3000u32 毫秒（现实世界耗时：不到 1 微秒！）   |
+-------------------------------------------------------------------------+
```

```flurry
trait VirtualClock {
    fn advance(*self, delta_ms: u32);
}

struct SimClock {
    elapsed_ms: u32,
}

impl VirtualClock for SimClock {
    fn advance(*self, delta_ms: u32) {
        self.elapsed_ms.* += delta_ms;
    }
}

test {
    var clock = SimClock { elapsed_ms: 0u32 }

    let reading = (fetch_with_backoff()# {
        -- 1. 拦截 sleep_ms 效应：直接拨快虚拟时钟 clock，绝不真正阻塞线程！
        sleep_ms(duration) => clock.advance(duration),

        -- 2. 拦截 async effect request_reading：
        --    注意保留 `async` 关键字以兑现子工作归属与 Future 交付契约！
        --    模拟前两次尝试丢包返回 0，第 3 次尝试成功返回 42！
        async request_reading(attempt) => {
            if attempt < 3u32 {
                0u32
            } else {
                42u32
            }
        },
    }).use(runtime);

    -- 输出：42 3000
    -- 在不到 1 微秒的时间内，精准验证了“前两次超时 + 1000ms/2000ms 指数退避 + 第三次成功”！
    println("{} {}", reading, clock.elapsed_ms);
}
```

---

## 演进 v2：使用 `handles` 绑定模式与 `.use(handler)` 复用标准处理策略

除了手写 `# { ... }` 分支将请求转发给 `clock.advance(...)` 之外，如果我们有一套在多个测试或业务模块之间反复复用的标准错误/效应处理器（比如“遇到传感器读数超限错误 `LimitErr.TooLarge` 时，自动截断钳位到最大安全值 `ClampTo`”），能不能把它封装成一个可复用的处理器对象，直接用 `.use(handler)` 挂载上去？

当然可以！这正是 `docs/algebraic-effect-and-delimited-continuation/handling-capability.md` 中定义的**处理能力绑定（`pattern handles expr`）与 `.use(handler)` 协议接入**！

让我们看看它的标准写法：

```flurry
trait HandleError
where E, R
{
    fn handle_error(*self, value: E) -> R;
}

enum LimitErr {
    TooLarge(u32),
}

struct ClampTo {
    limit: u32,
}

impl HandleError<LimitErr, u32> for ClampTo {
    fn handle_error(*self, value: LimitErr) -> u32 {
        value match {
            .TooLarge(_) => self.limit.*,
        }
    }
}

fn checked_count(value: u32) -> !LimitErr u32 {
    if value <= 8u32 {
        value
    } else {
        error LimitErr.TooLarge(value)
    }
}

test {
    -- 使用 `pattern handles Capability` 声明绑定对象的处理能力（多项能力可用 `and` 连接）
    let handler handles LimitErr = ClampTo { limit: 8u32 }

    -- 使用 `.use(handler)` 将处理器对象直接接入计算！
    -- 当 checked_count(12u32) 返回 LimitErr.TooLarge(12) 时，经 ClampTo 处理得到 8！
    let result = checked_count(12u32).use(handler);
    println("{}", result);
}
```

> **💡 核心心智模型（来自 `AGENTS.md` §5 与 `handling-capability.md`）：`handles` 只出现在局部绑定模式，绝不写进函数签名！**
> 请特别注意两条语义铁律：
> 1. **函数签名不添加 `handles`**：函数的错误与效应需求始终写在返回类型的限定前缀上（如 `-> !LimitErr u32` 或 `-> #[next_number] u32`）。`handles` 只是局部绑定模式（`let handler handles LimitErr = ...`）中的能力注解，用来告诉 elaboration 该对象提供哪些处理入口！
> 2. **绑定 `handles` 不等于隐式全局生效**：写下 `let handler handles LimitErr = ...` 并不会像动态作用域全局变量那样自动吞掉整个函数里所有的 `LimitErr`；你仍然必须在需要处理的位置显式写 `.use(handler)` 来安装它！

---

## 进阶技巧与编译期红线：在内层 Handler 中转发同名请求（外层供应保持）

在构建分层仿真测试时，我们经常会写一个“中间件式”的内层 Handler：
比如内层 Handler 拦截 `next_number()` 请求，但它自己并不凭空造数，而是**向外层再发起一次 `next_number()#` 请求**，拿到外层给的基准值后加 `1` 返回！

```flurry
effect next_number() -> u32;

fn total() -> #[next_number] u32 {
    let first = next_number()#;
    let second = next_number()#;
    first + second
}

-- 内层 Handler 分支自己又向外请求了 `next_number()#`！
-- 因此，total_with_offset 的返回类型依然保留外层需求：`#[next_number] u32`！
fn total_with_offset() -> #[next_number] u32 {
    total()# {
        next_number() => {
            -- 这里的 next_number()# 会精准路由到外层环境，绝不会死循环递归调用自己！
            let outside = next_number()#;
            outside + 1u32
        },
    }
}

test {
    -- 外层提供基准值 5u32 ──► 内层每次加 1 变成 6u32 ──► 6 + 6 = 12！
    let result = total_with_offset()# { next_number() => 5u32, }
    println("{}", result);
}
```

这里紧跟着一条编译期安全红线：
**因为内层处理分支 `next_number() => { let outside = next_number()#; ... }` 自己又向外部发起了 `next_number` 请求，所以它并没有把整个函数的 `#[next_number]` 需求彻底消去！**
如果你试图在 `total_with_offset` 的函数签名上把 `#[next_number]` 删掉，谎称自己已经不需要外层供应了，编译器会当场报错拦截：

```flurry
-- ❌ 编译期直接拒绝！
-- 内层处理分支自身发起了对外层 `next_number()#` 的请求，
-- 该外层效应需求尚未被消去，函数返回类型必须保留 `#[next_number] u32`！
fn bad_swallow_outer_requirement() -> u32 {
    total()# {
        next_number() => {
            let outside = next_number()#;
            outside + 1u32
        },
    }
}
```

---

## 本篇小结

| 机制 / 语法 | 核心规则与用法 | 工程与测试价值 |
| :--- | :--- | :--- |
| **有状态对象接入 Handler** | `expr# { op(x) => sim.method(x), }` | 将真实睡眠、网络 I/O 替换为内存状态机，实现纳秒级确定性仿真测试（DST） |
| **异步效应同步仿真分支** | `async op(x) => mock_calc(x),` | 测试中可直接计算模拟值，保留 `async` 关键字以维持 `Future` 与结构化范围契约 |
| **能力绑定与 `.use` 接入** | `let h handles Cap1 and Cap2 = ...;`<br>`comp.use(h)` | 封装可复用的错误/效应处理策略；通过 `.use(h)` 显式安装，函数签名不添加 `handles` |
| **内层转发与外层供应** | 分支内调用同名 `op()#` 路由至外层 Handler | 支持中间件式的偏移/日志装饰器，且编译器严格检查未消去的外层效应行 |

---

### 留给下一篇的悬念

看到代数效应在异步并发和确定性仿真测试中如此神通广大，一个关心底层性能的系统级程序员一定会忍不住发问：
> **“这套效应查找、直接返回、安装值传递、以及捕获 Continuation 挂起恢复的机制，在底层机器码和 ABI 层面到底是怎么跑的？会不会每次调个简单的配置读取效应都要去遍历整条调用栈或者分配堆内存？如果挂起区域中间夹着一个 C 语言函数（`^attrs.abi(.c)`），又会发生什么？”**

下一篇 **`154-under-the-hood-of-effect-abi-and-evidence-passing.md`**，我们将直接掀开 `docs/algebraic-effect-and-delimited-continuation/effect-abi.md` 的发动机舱盖，一探**证据传递（Evidence Passing）、五种 ABI 交接类别、显式池化栈后端（`.use(pool)`）与跨 C 语言边界屏障**的底层奥秘！
