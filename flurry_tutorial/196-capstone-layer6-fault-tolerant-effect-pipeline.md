# Flurry 演进手记 (196)：终极实战第六层（控制与效应层）：用代数效应 `#Async`、`!Err` 与前后置 `catch` 编排容错流水线

在前面的五篇手记中，我们已经为 **Orion 异构流式信号处理引擎** 建好了从协议解码（Layer 1）、分片环形队列（Layer 2 & 3）到 CMTT 展开卷积与 GPU 硬件加速（Layer 4 & 5）的全套数据与算力底座。

现在，我们要进入 **Layer 6（控制与效应层）**，把这些底层算子串成一条 7×24 小时不间断运行的**异步容错流式管线（Fault-Tolerant Streaming Pipeline）**。在真实生产环境中，这条管线必须同时应对三项挑战：
1. **零捕获开销的分布式追踪上下文**：成千上万个并发采样批次在管线中流转，每个算子都需要读取当前的遥测节点 ID（`trace_node_id`）和退避策略参数，但我们绝不希望每次读取都触发沉重的限定延续（Delimited Continuation）栈帧捕获；
2. **异步非阻塞调度（`#Async`）**：等待 DMA 新帧或等待硬件流完成时，协程需要通过代数效应挂起让出核心；
3. **瞬态硬件故障重试与降级（`!PipelineError`）**：当 GPU 队列出现瞬态繁忙（`GpuTransientBusy`）时，管线应当记录统一遥测日志并自动发起有限次重试；当遇到硬件离线（`DeviceOffline`）或重试耗尽时，则自动降级切换到 Layer 4 的 CPU 展开卷积核！

```
+=============================================================================+
|          Orion Signal Engine —— Layer 6: 代数效应与容错错误处理层             |
+=============================================================================+
| 1. ^attrs.effects.direct_passing effect current_node_id() -> u32            |
|    ---> 不捕获延续 resume，编译期直接降级为隐式函数指针/环境槽调用 (零栈切换)   |
|                                                                             |
| 2. run_gpu_stage()! {                                                       |
|        catch err => log_every_failure(err),   // 前置 catch：统一审计，不拦截 |
|        PipelineError.GpuTransientBusy => ...  // 精确分支：触发外部带标签重试  |
|        catch fallback_err => cpu_fallback()   // 后置 catch：兜底降级为成功值 |
|    }                                                                        |
+=============================================================================+
```

今天，我们就用第五卷（第 086～105 篇）确立的 **代数效应（Algebraic Effects）与直接传递优化（Direct Passing）**，结合 **错误通道（`!Err`）的前后置双 `catch` 消除语法**，编排这条既优雅又零开销的容错流水线！

---

## 一、 边界铁律：为什么错误（`!Err`）与代数效应（`#Effect`）拥有截然不同的 ABI？

在动手写管线之前，我们先重温 `AGENTS.md` §5 与 `docs/algebraic-effect-and-delimited-continuation/error-handling.md` 中最重要的一条架构边界：
> **错误结果（`!Err`）与代数效应（`#Effect`）具有截然不同的 ABI，函数签名绝不添加 `handles` 子句。**

为什么 Flurry 坚决拒绝把错误处理塞进“可恢复代数效应（`resume`）”里？
- **错误（`!Err`）是单向短路退出的控制流**：一旦函数通过 `return! err` 抛出错误，它的局部栈帧已经销毁，**绝不存在所谓的“在抛错点原地 `resume` 恢复执行”**！这使得 `!Err` 可以直接编译为寄存器级的标记联合返回（Tagged Value Return），拥有与普通 `return` 完全相同的极致性能。
- **如果我们需要“失败后重试”怎么办？** 在 Flurry 中，重试不是靠魔法般地回到已经死掉的栈帧里去 `resume`，而是用 **`expr! { ... }` 错误消除块配合外层带标签循环（`while :retry_loop`）** 显式重新发起调用！这样既保持了零开销的值返回 ABI，又让重试的副作用边界与最大次数一目了然。

---

## 二、 实战第一步：用 `^attrs.effects.direct_passing` 注入零延续开销的上下文效应

在 Orion 管线中，每个信号处理阶段都需要通过效应查询当前工作节点的遥测标识 `current_node_id()`，并在发生故障时上报 `record_fault_metric(code: u32)`。
这两个效应有一个共同特点：它们的 `handle` 处理器每次都是**算出结果后立即原路返回给调用点**，永远不会把延续保存起来稍后异步调用，也不会提前中止外层计算。

因此，我们直接为它们标上 `^attrs.effects.direct_passing` 属性（见第 092 篇与 `docs/algebraic-effect-and-delimited-continuation/direct-passing.md`）：

```flurry
-- 声明直接传递效应：禁止捕获限定延续，编译器直接将其降级为轻量级隐式上下文调用！
^attrs.effects.direct_passing
effect current_node_id() -> u32;

^attrs.effects.direct_passing
effect record_fault_metric(node_id: u32, fault_code: u32) -> void;

-- 声明异步调度效应（标准代数效应，允许挂起协程）
effect yield_pipeline_tick() -> void;
```

加上 `^attrs.effects.direct_passing` 后，调用 `current_node_id#()` 的机器码开销与读一个普通环境变量指针完全相同——没有任何协程栈切分或堆分配！

---

## 三、 实战第二步：用“前置 `catch` + 精确匹配 + 后置 `catch`”编排重试与 CPU 降级

现在来看 Orion Layer 6 最精彩的核心编排逻辑。
当管线尝试将一批采样点交给 GPU 加速核处理时，可能会返回 `PipelineError`：

```flurry
enum PipelineError {
    GpuTransientBusy(u32),
    DeviceOffline,
    CorruptedBatchHeader,
}
```

请看第 098 篇与 `docs/algebraic-effect-and-delimited-continuation/error-handling.md` 中独具匠心的 **`!` 错误消除块三段式结构**：
1. **前置 `catch`（Pre-Catch Observer）**：写在所有具体错误模式**之前**的 `catch err => { ... }`。它是一个**纯观察钩子（Non-consuming Tap）**——每当被包裹的表达式产生错误时，前置 `catch` 都会率先执行（比如统一上报故障遥测指标），但它**不会吞掉错误**，错误会继续向下流入后面的模式分支！
2. **中间具体错误分支（Specific Error Arms）**：针对特定变体（如 `PipelineError.GpuTransientBusy(wait_ticks)`）执行精确恢复或 `continue retry_loop` 重试；
3. **后置 `catch`（Post-Catch Fallback）**：写在具体分支**之后**的 `catch remaining_err => fallback_expr`。它负责拦截所有未被中间分支消化的剩余错误，并将其转化为正常返回值（如切换到 CPU 展开滤波核）或向上传播！

```
+-----------------------------------------------------------------------------+
|            Flurry 错误消除块 `! { ... }` 的三段式流水线模型                   |
+-----------------------------------------------------------------------------+
|  try_gpu_filter(batch)! {                                                   |
|      |                                                                      |
|      +---> [1. 前置 catch e] : 统一调用 record_fault_metric#，错误继续往下走！ |
|      |                                                                      |
|      +---> [2. 精确匹配分支] : 命中 GpuTransientBusy?                        |
|      |                         --> yield_pipeline_tick#()                   |
|      |                         --> continue retry_loop (重新发起下一轮尝试！) |
|      |                                                                      |
|      +---> [3. 后置 catch e] : 命中 DeviceOffline 或重试耗尽？                |
|                                --> 无缝降级调用 eval_orion_cpu_fallback()    |
|  }                                                                          |
+-----------------------------------------------------------------------------+
```

来看完整的 `Layer 6` 容错管线实现：

```flurry
-- 模拟 GPU 滤波阶段：可能抛出 PipelineError
fn try_gpu_filter_stage(batch_energy: f32, simulate_busy: bool) -> f32 ! PipelineError {
    if simulate_busy {
        return! PipelineError.GpuTransientBusy(1);
    }
    return batch_energy * 0.95;
}

-- CPU 展开滤波兜底函数（来自 Layer 4 的纯安全计算，永不因 GPU 故障失败）
pure fn run_cpu_fir_fallback(batch_energy: f32) -> f32 {
    return batch_energy * 0.95;
}

-- Orion 第六层核心编排：结合直接传递效应、异步挂起效应与前后置 catch 容错重试
fn process_batch_fault_tolerant(
    batch_energy: f32,
    initial_busy: bool,
) -> f32 ! PipelineError # (current_node_id, record_fault_metric, yield_pipeline_tick) {
    const node_id = current_node_id#();
    var attempts: u32 = 0;
    var is_busy = initial_busy;

    -- 使用带标签循环承载有界重试
    while :retry_loop true {
        const result = try_gpu_filter_stage(batch_energy, is_busy)! {
            -- 【第一段：前置 catch】观察所有错误并记录遥测，绝不吞掉错误！
            catch observed_err => {
                const code: u32 = observed_err match {
                    .GpuTransientBusy(_) => 101,
                    .DeviceOffline => 500,
                    .CorruptedBatchHeader => 400,
                };
                record_fault_metric#(node_id, code);
            },

            -- 【第二段：精确错误分支】如果是瞬态繁忙且未超过 3 次，异步让出时间片后重试！
            PipelineError.GpuTransientBusy(_ticks) => {
                if attempts < 3 {
                    attempts += 1;
                    is_busy = false; -- 第二次重试时恢复正常
                    yield_pipeline_tick#();
                    continue retry_loop;
                }
                -- 超过最大重试次数，平滑降级到 CPU 展开卷积核
                run_cpu_fir_fallback(batch_energy)
            },

            -- 如果帧头本身已损坏，不属于硬件算力故障，直接向更外层传播错误！
            PipelineError.CorruptedBatchHeader => {
                return! PipelineError.CorruptedBatchHeader;
            },

            -- 【第三段：后置 catch】兜底其余硬件错误（如 DeviceOffline），降级到 CPU 核
            catch _fallback_err => run_cpu_fir_fallback(batch_energy),
        };

        return result;
    }
}
```

看看这段代码是多么清晰：
1. 无论是瞬态繁忙、设备离线还是帧头损坏，**前置 `catch observed_err`** 都保证了 `record_fault_metric#(node_id, code)` 100% 被执行，绝不会因为某个分支提前 `continue` 或 `return!` 而漏记监控指标；
2. 遇到 `GpuTransientBusy` 时，通过 `#yield_pipeline_tick` 挂起等待一个调度滴答，随后 `continue retry_loop` 重新执行；
3. 遇到 `DeviceOffline` 时，**后置 `catch _fallback_err`** 自动接管，无缝切换到 Layer 4 生成的 `run_cpu_fir_fallback`！

---

## 四、 编译器如何拦截在 `direct_passing` 效应中非法逃逸延续？（负向示例）

由于 `current_node_id` 标记了 `^attrs.effects.direct_passing`，它的 `handle` 块必须像普通直通函数一样直接返回结果值。如果有人在 `handle` 块里企图把延续 `k` 捕获出来或者不遵循直通规则，编译器会直接拒绝：

```flurry
-- ❌ 拒绝例：违背 ^attrs.effects.direct_passing 契约，试图在处理器中跨控制流搁置延续
fn bad_direct_handler() -> f32 {
    handle {
        -- 编译错误：`current_node_id` is declared with `^attrs.effects.direct_passing`;
        -- its handler must directly return a `u32` value at the tail position
        -- and cannot suspend or abort the caller's continuation.
        current_node_id() => {
            break 0.0;
        }
    } do {
        const id = current_node_id#();
        return id as f32;
    }
}
```

---

## 五、 本层通关小结与下一层预告

在 **Layer 6（控制与效应层）** 中，我们为 Orion 引擎组装了兼具高性能与高可靠性的异步控制骨架：
- 用 `^attrs.effects.direct_passing` 实现了零延续捕获开销的节点上下文与遥测注入；
- 严格区分了单向退出的错误通道（`!PipelineError`）与可挂起的协程调度效应（`#yield_pipeline_tick`）；
- 用三段式“前置 `catch`（统一观测）+ 精确匹配（带标签循环重试）+ 后置 `catch`（CPU 软核降级）”构建了自愈式容错流水线。

当 64 个通道的容错流水线在多核 CPU 上全速并发运转时，每个通道算出的滤波能量峰值、信噪比（SNR）与故障降级计数，都需要实时汇总到全局的 **Orion 遥测仪表盘（Telemetry Aggregator）** 中。

如果用传统的读写锁或细粒度互斥锁（Mutex）去同步多通道指标，不仅容易在复合更新时发生死锁或中间状态撕裂（Torn Read），还会阻塞高速滤波协程！

在下一篇 **第 197 篇（Layer 7：状态协同层）** 中，我们将使用 Flurry 原生的 **`atomic` 软件事务内存（STM）与 P3～P5 纯度守卫**，实现零死锁、乐观并发的多通道实时指标协同汇总！
