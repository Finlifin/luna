# Flurry 演进手记 (197)：综合实战第七层（状态协同层）：用 `atomic` STM 块安全汇总多通道实时统计指标

在上一篇（第 196 篇）中，我们用代数效应与前后置 `catch` 为 **Orion 异构流式信号处理引擎** 编排了 **Layer 6（控制与效应层）**，让 64 个通道的容错滤波协程在多核工作池上高速运转。

当每个通道完成一个采样批次的 FIR 滤波后，它需要把当前批次的 **总处理帧数（`total_frames`）**、**累计信号能量（`accumulated_energy`）**、**峰值信噪比通道号（`peak_channel`）** 以及 **硬件降级计数（`fallback_count`）** 同步更新到全局共享的遥测状态 `OrionTelemetryBoard` 中；一旦某个批次触发了紧急能量阈值，还要向外部监控中心发送一条不可撤销的网络告警（`emit_urgent_alert#`）。

这就把我们带到了 **Layer 7（状态协同层）**：如何在高并发下同时保证**多字段复合更新的原子一致性**，又不在事务重试期间重复发送不可撤销的外部网络告警？

```
+=============================================================================+
|             Orion Signal Engine —— Layer 7: STM 事务状态协同层                |
+=============================================================================+
| Layer 6: 64 通道并发容错流水线产出 BatchMetrics                               |
|                        |                                                    |
|                        v                                                    |
| Layer 7: atomic(board) { ... } + Commit-Then-Act 模式                <-- 本篇|
|          +-- 1. 事务内允许调用 pure fn (P4/P5 含局部 var) 与 atomic fn (P2)   |
|          +-- 2. 编译器严禁在 atomic 块内触发 P0 不可回滚 I/O 效应             |
|          +-- 3. 事务提交返回 AlertDecision，在事务外安全触发外部告警效应       |
+=============================================================================+
```

---

## 一、 问题背景：为什么传统的“原子整数 + 互斥锁”会在复合指标汇总里翻车？

在传统系统语言中，面对 `OrionTelemetryBoard` 这样的多字段共享状态，开发者通常陷入两难：
1. **如果每个字段各自用 `AtomicU64`**：虽然单个字段的自增是原子的，但多个字段之间**没有任何跨变量事务一致性（No Cross-Variable Atomicity）**。当监控线程读取仪表盘计算“平均每帧能量 = `accumulated_energy / total_frames`”时，很容易读到“能量已经加上了新批次，但帧数还没来得及加”的**撕裂快照（Torn Snapshot）**，算出的尖峰噪声引发误报警。
2. **如果用细粒度互斥锁（`Mutex`）并支持组合转移**：一旦需要同时更新“通道分片表”与“全局汇总表”，不同线程加锁顺序稍有存在分歧就会死锁；更糟糕的是，如果有人在持有锁或乐观重试循环里直接调用了外部网络 I/O（如发送告警包），非但会把锁持有时间拉长上万倍，在乐观并发控制下还会导致**同一条告警在冲突重试时被重复发送十几次**。

---

## 二、 第一步：用 `pure fn`（P4/P5）与 `atomic fn`（P2）构建可组合的指标更新

在第 031～035 篇以及第 157 篇中我们学过，Flurry 将函数与代码块按**副作用可回滚性与观测纯度**划分为严格的 **P0～P5 纯度阶梯**：
- **P4 / P5（`pure fn`）**：对外部世界完全透明的纯函数（允许内部使用局部可变变量 `var` 做循环累加，只要局限于内而未逃逸出函数边界）；
- **P2（`atomic fn` / `atomic(ids) { ... }`）**：软件事务内存（STM）操作。它读写的所有共享事务位置都记录在线程本地的读写集日志（Read/Write Log）中；如果提交时检测到版本冲突，运行时会自动丢弃写日志并重新执行事务块。
- **P0（General Impure / External I/O Effects）**：不可回滚的外部系统调用、网络发送或硬件寄存器写操作。

正因为 **P4/P5 纯函数与 P2 事务函数在冲突回滚时免向外部世界泄露任何不可撤销的痕迹**，它们可以在 `atomic` 块内部自由嵌套组合。

```
+-----------------------------------------------------------------------------+
|            Flurry 纯度阶梯守卫下的 STM 乐观事务生命周期                        |
+-----------------------------------------------------------------------------+
|  atomic(board) {                                                            |
|      1. 调用 pure safe fn normalize_energy(...)  [P4/P5: 局部 var 完全合法]  |
|      2. 调用 atomic safe fn merge_peak(...)      [P2: 事务日志可无缝合并]    |
|      3. 尝试 Commit 提交写集：                                                |
|         +-- 若版本无冲突 ---> 原子生效  返回 AlertDecision 给事务外部         |
|         +-- 若发生写冲突 ---> 自动丢弃写集并安全重跑 (零外部副作用泄露  )     |
|  }                                                                          |
+-----------------------------------------------------------------------------+
```

来看 Orion Layer 7 的事务状态结构与纯辅助函数实现：

```flurry
struct OrionTelemetryBoard {
    total_frames: u64,
    accumulated_energy_milli: u64,
    peak_energy_milli: u64,
    peak_channel: u8,
    fallback_count: u64,
}

struct AlertDecision {
    should_alert: bool,
    channel: u8,
    observed_peak: u64,
}

-- P4 纯函数：内部允许使用局部可变变量 var 进行多次迭代阻尼平滑，外部观测纯度仍为 P4。
pure fn compute_damped_energy(raw_energy_milli: u64, damping_steps: u32) -> u64 {
    var acc = raw_energy_milli;
    var step: u32 = 0;
    while step < damping_steps {
        acc = (acc * 9) / 10;
        step += 1;
    }
    acc
}

-- P2 事务函数：可组合地更新峰值通道与峰值能量，可被外层 atomic 块无缝合并调用
atomic fn update_board_peak(
    board: *OrionTelemetryBoard,
    candidate_ch: u8,
    candidate_energy: u64,
    alert_threshold: u64,
) -> AlertDecision {
    if candidate_energy > board.peak_energy_milli.* {
        board.peak_energy_milli.* = candidate_energy;
        board.peak_channel.* = candidate_ch;
        AlertDecision {
            should_alert: candidate_energy >= alert_threshold,
            channel: candidate_ch,
            observed_peak: candidate_energy,
        }
    }
    AlertDecision {
        should_alert: false,
        channel: candidate_ch,
        observed_peak: board.peak_energy_milli.*,
    }
}
```

---

## 三、 第二步：“先提交、后行动（Commit-Then-Act）”隔离 P0 外部告警

当 `update_board_peak` 判断出新的峰值能量突破了危险阈值（`should_alert == true`）时，我们需要触发外部网络告警效应 `emit_urgent_alert#(ch, peak)`。

请牢记 Flurry STM 的核心规则：**禁止于 `atomic` 块或 `atomic fn` 内部触发 P0 外部效应。**
正确的工业级模式是 **“先提交、后行动（Commit-Then-Act）”**：
1. 在 `atomic(board) { ... }` 块内部，只完成纯计算（`compute_damped_energy`）与事务状态更新（`update_board_peak`），并将是否需要告警的决策打包成纯数据值 `AlertDecision` 作为 `atomic` 块的结果返回；
2. 当控制流走出 `atomic(board)` 的右花括号时，事务已经 **100% 原子提交成功且不再回滚重试**；
3. 此时在事务外部检查 `decision.should_alert`，安全地触发一次性外部效应 `emit_urgent_alert#`。

```flurry
-- 声明不可回滚的外部网络告警效应（属于 P0 外部副作用）
effect emit_urgent_alert(channel: u8, peak_energy_milli: u64) -> void;

-- ✅ 正向示例：使用 Commit-Then-Act 模式协调 STM 事务与外部告警效应
fn commit_channel_batch_metrics(
    board: *OrionTelemetryBoard,
    channel: u8,
    frames_in_batch: u64,
    raw_energy_milli: u64,
    used_cpu_fallback: bool,
) -> #[emit_urgent_alert] void {
    -- 第一阶段：在 atomic(board) 事务块内原子更新全部 5 个关联指标
    var decision = AlertDecision {
        should_alert: false,
        channel: channel,
        observed_peak: 0,
    }

    atomic(board) {
        -- 1. 调用 P4 纯函数计算平滑后的能量
        let damped = compute_damped_energy(raw_energy_milli, 2);

        -- 2. 原子更新帧数、累计能量与降级计数（保证监控端不会读到撕裂状态。）
        board.total_frames.* += frames_in_batch;
        board.accumulated_energy_milli.* += damped;
        if used_cpu_fallback {
            board.fallback_count.* += 1;
        }

        -- 3. 组合调用 P2 atomic fn 更新峰值并获取告警决策值
        decision = update_board_peak(board, channel, damped, 10000);
    }

    -- 第二阶段（Act）：事务已成功提交，在事务外安全触发不可回滚的外部网络告警。
    if decision.should_alert {
        emit_urgent_alert(decision.channel, decision.observed_peak)#;
    }
}
```

同时，当监控线程想要读取一致的“平均每帧能量”快照时，只需同样用一个小巧的只读 `atomic(board)` 块包裹读取即可，保证看到的 `total_frames` 和 `accumulated_energy_milli` 永远来自同一个事务版本：

```flurry
atomic fn read_consistent_average_energy(board: *OrionTelemetryBoard) -> u64 {
    if board.total_frames.* == 0 {
        return 0;
    }
    board.accumulated_energy_milli.* / board.total_frames.*
}
```

---

## 四、 编译器如何拦截混入 `atomic` 块的外部 I/O 效应？（负向示例）

如果某位粗心的工程师忘了 Commit-Then-Act 原则，直接在 `atomic(board)` 块里面调用了 `emit_urgent_alert#(channel, damped)`，会发生什么？

编译器无需等到线上高并发冲突时发出几十条重复告警，在类型与纯度检查阶段就会直接报错拦截：

```flurry
-- ❌ 负例：企图在 atomic 事务块内直接执行不可回滚的外部效应
fn broken_atomic_with_io(
    board: *OrionTelemetryBoard,
    channel: u8,
    raw_energy: u64,
) -> #[emit_urgent_alert] void {
    atomic(board) {
        board.accumulated_energy_milli.* += raw_energy;

        -- 编译错误：Purity violation inside `atomic` block:
        -- effect `emit_urgent_alert` is not transactionally rollback-safe (P0 impure),
        -- and cannot be performed inside an `atomic` transaction (requires >= P2).
        -- Help: return a decision value from the `atomic` block and perform the effect after commit.
        emit_urgent_alert(channel, raw_energy)#;
    }
}
```

---

## 五、 本层通关小结与下一层预告

在 **Layer 7（状态协同层）** 中，我们用 Flurry 的 STM 与纯度阶梯为 Orion 引擎解决了高并发状态同步难题：
- 用 `atomic(board)` 与可组合的 `atomic fn` 消除了多字段指标更新的中间状态撕裂与死锁风险；
- 允许在事务内自由调用包含局部 `var` 循环的 `pure fn`（P4/P5）；
- 用编译器强制执行的纯度守卫与 Commit-Then-Act 模式，消除了乐观事务重试引发的外部效应重复执行。

现在，Orion 引擎的核心计算与状态汇总已经完备。但在实际部署到深空地面站或边缘网关时，运维团队还需要：
1. 用清爽的声明式配置语法（支持默认参数前向推导与扩展键值块 `{ ... }`）一键组装引擎；
2. 将汇总出的遥测快照自动序列化（`derive Encode`），并通过运行时可热插拔的动态接口（`dyn SinkPlugin`）分发给不同的后端存储插件。

在下一篇 **第 198 篇（Layer 8：插件与配置层）** 中，我们将把 **扩展函数调用（Extended Application）、前向依赖可选参数、`derive fn` 与 `dyn Trait`** 结合起来，打造表现力丰富的声明式插件与配置层。
