# Flurry 演进手记 (159)：第七卷里程碑（上）：从零实现一个支持协程挂起、超时取消与协作调度的异步执行器

欢迎来到《Flurry 演进手记》第七卷里程碑实战的上半场！

在许多语言里，“协程调度器（Coroutine Scheduler）”和“异步运行时（Async Executor）”往往是藏在编译器底层或万行级第三方库里的黑魔法。普通开发者只敢在表面写 `async/await`，一旦遇到“我想给某个边缘网关任务设定严格的步数/时间配额，超配额时自动中断任务并安全执行其栈上所有 `Drop` 析构函数”这样的定制调度需求，就往往束手无策。

而在 Flurry 中，通过前几篇对**限定 Continuation（`catch continuation: builtin.Continuation`）**、**异步效应（`async effect`）**与**池化控制后端（`.use(pool)` / `.use(runtime)`）**的深入学习，你已经看穿了异步调度的本质：
**所谓的协程挂起、恢复与超时取消，不过是在 `# { ... }` 效应处理器中接收、消费或放弃单次恢复权（Continuation）的标准控制流操作！**

今天，在这篇里程碑（上）中，我们将结合 `docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md` 与 `async-algebraic-effect.md`，从零实现一套支持**协作式步进挂起、配额超时自动取消（带完整 `Drop` 清理）以及异步并发子工作汇聚**的轻量级网关调度执行器！

---

## 阶段一（v0）：剖析执行器内部的状态机——单次恢复权（`K`）如何在队列与结果槽之间流转？

在动手写调度器代码之前，让我们先把 `multi-prompt-delimited-continuation.md` 与 `async-algebraic-effect.md` 中规定的**结果槽与恢复权状态机**画成一张清晰的 **Code Cartoon**。
为什么很多手写的玩具协程库会在高并发或取消时崩溃？因为它们没有遵守**“单次恢复权（Single-Shot Continuation）唯一所有权交接”**铁律！

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：执行器内部的结果槽与恢复权（Continuation）生命周期     |
|                                                                         |
|  [ 任务运行中 ]                                                         |
|       │                                                                 |
|       ▼ 发起挂起效应请求 (yield_step / await 未就绪结果)                |
|  [ Handler 分支取得唯一恢复权 K1 ]                                      |
|       │                                                                 |
|       ├──► 路径 A（继续调度 / 移入就绪队列）：                          |
|       │      • 分支移交或立即 `resume` 消费 K1                          |
|       │      • 原分支不再拥有 K1；计算恢复运行                          |
|       │      • 若再次挂起，产生全新的恢复权身份 K2！                    |
|       │                                                                 |
|       └──► 路径 B（配额耗尽 / 超时取消）：                              |
|              • 争夺并取得唯一的恢复权 K1                                |
|              • 不执行 `resume`，直接触发**放弃协议（Abandonment）**：   |
|                沿隐藏中止通道进入挂起栈 ──► 逐帧执行局部资源 `Drop`     |
|                ──► 到达限定边界停止 ──► 将栈槽归还来源 `pool`！         |
+-------------------------------------------------------------------------+
|  ⚠️ 核心不变量：                                                        |
|  “完成唤醒（Resume）”与“超时取消（Cancel）”争用同一份唯一恢复权 K1！    |
|  绝不允许把 K1 按位复制成两份同时塞进就绪队列和取消队列！               |
+-------------------------------------------------------------------------+
```

---

## 阶段二（v1）：用 `effect enum` 与单次恢复构建“带步数配额与超时自动清理”的协作调度器！

在工业边缘网关中，为了防止某个失控的传感器采集协程陷入死循环耗尽 CPU，我们需要一个**有界步数协作调度器（Bounded-Step Cooperative Scheduler）**：
- 采集任务每完成一步采样，就调用一次协作让出效应 `Scheduler.yield_tick(cost)#` 汇报本次消耗的配额，或者调用 `Scheduler.emit_sample(val)#` 吐出采样数据；
- 调度器维护一个剩余配额计数器 `remaining_budget`：
  - 只要 `remaining_budget >= cost`，调度器扣除配额并执行 `resume;`（尾切换恢复任务继续运行）；
  - 一旦 `remaining_budget < cost`（配额耗尽超时！），调度器打印超时告警，并**直接退出分支而不调用 `resume`**——此时 Flurry 的放弃协议自动接管，沿着挂起栈精准析构任务内部持有的硬件通道锁 `ChannelGuard`，确保零资源泄漏！

让我们用 `effect enum`（见 `docs/grammar/syntax` 第 364 行：`effect enum id clauses? { effect_variant* }`）把这组调度操作优雅地组织起来，并写出完整的 v1 调度器：

```flurry
-- 使用 effect enum 组织一组协作式调度效应操作
effect enum CoopSched {
    -- 汇报步进消耗并请求协作挂起（带 catch continuation 捕获许可）
    yield_tick(cost: u32, catch continuation: builtin.Continuation)
    -- 向调度器上报一条采集到的有效样本值
    record_sample(sample: u32)
}

-- 模拟采集协程内部持有的硬件 DMA 通道锁资源（实现 Drop 验证取消清理！）
struct ChannelGuard {
    channel_id: u32,
}

impl Drop for ChannelGuard {
    fn drop(*self) {
        println("released channel {}", self.channel_id.*);
    }
}

-- 运行在调度器之上的传感器持续采集任务：
-- 它全然不知外围的配额上限是多少，只管按部就班地持有 ChannelGuard 并在每轮循环让出控制权！
fn sensor_harvest_loop(channel_id: u32, rounds: u32) -> #[CoopSched] void {
    let guard = ChannelGuard { channel_id: channel_id }

    for step in 1u32 ..= rounds {
        -- 每轮消耗 10 个时间片配额
        CoopSched.yield_tick(10u32)#;
        -- 若未被取消并成功恢复，上报本轮采样值 (step * 100)
        CoopSched.record_sample(step * 100u32)#;
    }

    println("harvest finished all rounds");
}
```

现在，让我们编写调度执行器函数 `run_with_budget`，把有状态的调度上下文接入 `# { ... }` 与 `.use(pool)`：

```flurry
struct ExecStats {
    remaining_budget: u32,
    sample_sum: u32,
    timed_out: bool,
}

test {
    -- 给定初始时间片配额为 25：
    -- 第 1 轮消耗 10 (剩余 15) -> 采样 100
    -- 第 2 轮消耗 10 (剩余 5)  -> 采样 200
    -- 第 3 轮请求消耗 10 (剩余 5 < 10，配额耗尽触发超时取消！)
    var stats = ExecStats {
        remaining_budget: 25u32,
        sample_sum: 0u32,
        timed_out: false,
    }

    (sensor_harvest_loop(7u32, 5u32)# {
        CoopSched.yield_tick(cost, continuation) => {
            if stats.remaining_budget >= cost {
                stats.remaining_budget -= cost;
                -- 配额充足：尾切换恢复采集协程继续执行！
                resume;
            } else {
                -- 配额耗尽！记录超时标志，且【不执行 resume】！
                -- 离开本分支时，continuation 自动执行放弃协议（Abandonment Protocol），
                -- 挂起栈内的 `guard: ChannelGuard` 立即触发 Drop 输出 "released channel 7"！
                stats.timed_out = true;
                println("budget exhausted, cancelling task");
            }
        },
        CoopSched.record_sample(sample) => {
            stats.sample_sum += sample;
        },
    }).use(pool);

    -- 依次输出：
    -- budget exhausted, cancelling task
    -- released channel 7
    -- sum=300, rem=5, timeout=true
    -- （注意："harvest finished all rounds" 绝不会执行，但 ChannelGuard 100% 被安全释放！）
    println("sum={}, rem={}, timeout={}", stats.sample_sum, stats.remaining_budget, stats.timed_out);
}
```

请仔细欣赏上面这段测试的执行轨迹：
1. 在第 1 轮和第 2 轮中，`yield_tick` 扣减了 `stats.remaining_budget` 并通过 `resume;` 尾切换恢复协程，`sample_sum` 累加到了 `100 + 200 = 300`；
2. 到第 3 轮时，剩余配额 `5 < 10`，Handler 分支直接设置 `stats.timed_out = true` 并自然结束分支（没有调用 `resume`）；
3. 就在 `continuation` 离开分支作用域的那一瞬间，底层控制后端沿着隐藏中止通道进入挂起栈帧，精准调用了 `guard` 的 `Drop::drop` 打印出 `"released channel 7"`，然后干干净净地把栈槽归还给了 `pool`！

---

## 阶段三（v2）：融合 `async effect` 并发子任务扇出（Fan-Out）与协作式收集！

在网关的实际运行中，单个协程内部不仅要按步进配额运行，还会通过 `async effect` 同时向多个硬件探头扇出（Fan-out）异步校准请求，再用 `.await` 收集结果！

让我们把 `async effect probe_channel(ch: u32) -> !ProbeErr u32` 与我们的协作式配额调度器叠加在一起（多提示符栈的威力在此刻尽显无疑！）：

```flurry
enum ProbeErr {
    disconnected(u32),
}

async effect probe_channel(ch: u32) -> !ProbeErr u32;

-- 同时使用异步探头效应 probe_channel 与协作步进效应 CoopSched！
fn fanout_dual_probes(ch1: u32, ch2: u32) -> #[probe_channel, CoopSched] !ProbeErr u32 {
    -- 1. 扇出（Fan-out）：同时登记两个异步探头子任务，立即取得两个 Future！
    let fut1 = probe_channel(ch1)#;
    let fut2 = probe_channel(ch2)#;

    -- 2. 在等待硬件响应前，汇报一次调度器步进消耗
    CoopSched.yield_tick(5u32)#;

    -- 3. 扇入（Fan-in）：依次 .await 取得 `!ProbeErr u32` 结果并用 `!` 传播可能的错误
    let val1 = fut1.await!;
    let val2 = fut2.await!;
    let combined = val1 + val2;

    CoopSched.record_sample(combined)#;
    return combined;
}
```

看看 `fanout_dual_probes` 的函数签名：
`-> #[probe_channel, CoopSched] !ProbeErr u32`
- **`#[probe_channel, CoopSched]`**：效应行清晰列出了它需要的异步探头能力与协作调度能力；
- **`!ProbeErr u32`**：错误行清晰列出了它可能向上传递的探头断连错误。

如果我们在编写外围执行器包装函数时，只消去了 `CoopSched`，却忘了消去 `probe_channel`（或者反过来），编译器会通过效应行检查当场提醒我们：

```flurry
-- ❌ 编译期直接拒绝！
-- 函数体内只用 `# { ... }` 处理了 `CoopSched`，
-- 尚未处理 `probe_channel` 异步效应，返回类型卻漏写了 `#[probe_channel]`！
fn bad_incomplete_executor() -> !ProbeErr u32 {
    return (fanout_dual_probes(1u32, 2u32)# {
        CoopSched.yield_tick(_, continuation) => { resume; },
        CoopSched.record_sample(_) => {},
    }).use(pool);
}
```

---

## 本篇小结

| 执行器核心能力 | Flurry 实现机制 | 工程与安全收益 |
| :--- | :--- | :--- |
| **协作式挂起与步进** | `effect enum CoopSched` + `catch continuation` + `resume;` | 尾切换恢复开销极低，无需将业务逻辑拆碎成手写状态机 |
| **配额耗尽超时取消** | 分支内不调用 `resume` 直接退出，自动触发放弃协议（Abandonment） | 沿隐藏中止通道逐帧执行挂起栈上的 `Drop`（如释放 `ChannelGuard`），零资源泄漏 |
| **异步扇出与汇聚** | 连续调用 `probe_channel(ch)#` 登记子工作，随后分别 `.await!` | 请求即启动并发子工作，结构化范围兜底任务归属，错误行与效应行正交组合 |

---

### 留给下一篇的悬念

在今天的里程碑（上）中，我们打造出了支持配额取消与并发扇出的异步调度执行器内核。
现在，我们要迎接第七卷的**大结局——第 160 篇 `160-milestone-part2-fault-injection-on-sensor-service.md`**！

在下一篇中，我们将把：
1. 今天写的**异步扇出与配额调度器（`CoopSched` + `probe_channel`）**；
2. 第 155–156 篇写的**软件事务内存（`atomic(vars) { ... }` + `atomic fn` + Commit-Then-Act 纯度隔离）**；
3. 第 153 篇写的**确定性仿真与故障注入 Handler（Fault Injection）**；
全部组装在一起！我们将运行一个完整的高并发多通道传感器聚合与告警服务，并在测试中向它注入“2 号探头断连故障”、“并发事务水位线更新”与“步数配额耗尽”，完成第七卷的巅峰验收！
