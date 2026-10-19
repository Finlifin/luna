# Flurry 演进手记 (160)：第七卷里程碑（下）：在自定义执行器上运行高并发传感器聚合服务并进行故障注入测试

欢迎来到《Flurry 演进手记》第七卷的终极收官之战！

在上一篇（里程碑上半场）中，我们用 `effect enum CoopSched`、`catch continuation: builtin.Continuation` 和 `async effect` 亲手打造了一个支持**协作式步进配额、超时自动 `Drop` 清理与并发扇出**的执行器内核。

今天，在这篇里程碑下半场中，我们要在这个执行器之上，真正跑起边缘网关的核心业务大脑——**高并发多通道传感器聚合与水位线告警服务（High-Concurrency Sensor Aggregation & Watermark Service）**，并对它展开一场刀刀见血的**确定性混沌故障注入测试（Deterministic Chaos & Fault Injection Testing）**！

我们将在这一个完整的系统里，同时检阅第七卷的所有王牌武器：
1. **多提示符嵌套效应（Multi-Prompt Effects）**：内层跑协作步进配额 `CoopSched`，外层跑异步硬件探头 `probe_channel` 与告警广播 `emit_alarm`；
2. **异步扇出与错误行恢复（`async effect` + `.await` + `! { ... }`）**：并发采集主备双通道传感器，当主通道发生硬件断连故障（`ProbeErr.Disconnected`）时，自动降级采用备通道数据而不中断整体服务；
3. **STM 软件事务内存与纯度隔离（`atomic` + `atomic fn` + `pure fn`）**：将采集到的有效样本通过 `atomic(total_sum, peak_watermark, sample_count)` 原子地聚合进共享统计面板，并采用 **Commit-Then-Act** 模式在事务提交后触发越界告警效应！

---

## 全景架构图：多通道采集、STM 原子聚合与混沌故障注入流水线

让我们先用一张 **Code Cartoon** 看清整套服务与测试沙盒的运转脉络：

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：高并发传感器聚合服务与确定性故障注入沙盒              |
|                                                                         |
|  ┌─── 混沌测试沙盒 (Deterministic Fault Injection Harness) ───────────┐ |
|  │ • 外层 Handler 1 (`probe_channel`)：对通道 1 注入断连故障！⚡       │ |
|  │ • 外层 Handler 2 (`emit_alarm`)   ：捕获告警事件记入审计计数器     │ |
|  │ • 内层 Handler 3 (`CoopSched`)    ：管控 CPU 步进配额与超时 Drop   │ |
|  │                                                                    │ |
|  │   ┌─── 核心业务：aggregate_sensor_round(...) ──────────────────┐   │ |
|  │   │ 1. 并发扇出：`fut1 = probe_channel(ch1)#`                  │   │ |
|  │   │              `fut2 = probe_channel(ch2)#`                  │   │ |
|  │   │ 2. 协作步进：`CoopSched.yield_tick(10)#`                   │   │ |
|  │   │ 3. 容错等待：`fut1.await! { .Disconnected(_) => 0 }`       │   │ |
|  │   │ 4. STM 事务：`atomic(sum, peak, cnt) { ... }`              │   │ |
|  │   │              (调用 pure fn + atomic fn，返回 ?u32 告警载荷)│   │ |
|  │   │ 5. 提交后动作：若返回 Some(peak)，触发 `emit_alarm(peak)#` │   │ |
|  │   └────────────────────────────────────────────────────────────┘   │ |
|  └────────────────────────────────────────────────────────────────────┘ |
+-------------------------------------------------------------------------+
```

---

## 第一步（v0）：定义领域错误、代数效应与满足 P5 纯度的辅助函数

我们首先声明服务所需的错误类型 `ProbeErr`、异步探头效应 `probe_channel`、外部告警效应 `emit_alarm`、协作调度效应组 `CoopSched`，以及一个带有局部私有 `var` 的值纯函数 `pure fn sanitize_reading`：

```flurry
enum ProbeErr {
    Disconnected(u32),
}

-- 异步硬件探头效应：在子 task 中采集通道 ch，可能返回 ProbeErr
async effect probe_channel(ch: u32) -> !ProbeErr u32;

-- 外部不可逆告警广播效应（P0 外部事件，严禁直接放进 STM atomic 块内！）
effect emit_alarm(new_peak: u32);

-- 协作式配额调度效应组
effect enum CoopSched {
    yield_tick(cost: u32, catch continuation: builtin.Continuation)
}

-- 满足 P4/P5 的值纯清洗函数：
-- 内部使用私有局部 var 对噪声读数做上下界钳位，可在 STM 事务块内安全重复调用！
pure fn sanitize_reading(raw: u32, max_valid: u32) -> u32 {
    var cleaned = raw;
    if cleaned > max_valid {
        cleaned = max_valid;
    }
    cleaned
}
```

---

## 第二步（v1）：编写可组合的 `atomic fn` 与“Commit-Then-Act”聚合核心

接下来，我们编写共享统计面板的 STM 事务更新函数 `update_panel_tx`，以及聚合业务主函数 `aggregate_sensor_round`。

请特别注意我们在 `aggregate_sensor_round` 中是如何结合：
1. **双通道异步扇出与局部错误恢复**：用 `fut1.await! { .Disconnected(_) => 0u32, }` 将单通道故障優雅降级为 `0u32`；
2. **STM 原子提交与外部效应隔离**：在 `atomic(sum, peak, count) { ... }` 中完成三个共享变量的原子更新，计算出是否突破了历史峰值 `?u32`；等事务块彻底提交后，再在事务外调用 `emit_alarm(confirmed_peak)#`！

```flurry
-- STM 原子事务辅助函数：同时更新总和、样本计数与历史峰值水位线！
-- 若本次样本突破了历史峰值，返回 Some(cleaned)，否则返回 null
atomic fn update_panel_tx(
    sum: *u32,
    peak: *u32,
    count: *u32,
    raw_sample: u32,
) -> ?u32 {
    let cleaned = sanitize_reading(raw_sample, 500u32);
    sum.* += cleaned;
    count.* += 1u32;

    if cleaned > peak.* {
        peak.* = cleaned;
        return cleaned;
    }
    return null;
}

-- 聚合服务核心函数：
-- 同时携带 #[probe_channel, emit_alarm, CoopSched] 三项效应需求！
fn aggregate_sensor_round(
    primary_ch: u32,
    backup_ch: u32,
    sum: *u32,
    peak: *u32,
    count: *u32,
) -> #[probe_channel, emit_alarm, CoopSched] u32 {
    -- 1. 异步并发扇出（Fan-out）：同时向主通道与备通道发起采集请求！
    let primary_fut = probe_channel(primary_ch)#;
    let backup_fut = probe_channel(backup_ch)#;

    -- 2. 向内层协作调度器汇报 10 单位步进开销
    CoopSched.yield_tick(10u32)#;

    -- 3. 等待异步结果并用 `! { ... }` 就地消去单通道断连错误（故障降级）
    let v1 = (primary_fut.await)! {
        .Disconnected(_) => 0u32,
    };
    let v2 = (backup_fut.await)! {
        .Disconnected(_) => 0u32,
    };
    let merged = v1 + v2;

    -- 4. STM 软件事务内存块：原子更新 sum, peak, count 三个变量！
    --    内部仅调用 atomic fn 与 pure fn，100% 满足纯度与可回滚重试契约！
    let maybe_new_peak: ?u32 = atomic(sum, peak, count) {
        update_panel_tx(sum, peak, count, merged)
    };

    -- 5. Commit-Then-Act：事务已安全提交，若有新峰值则在事务外触发告警效应！
    if maybe_new_peak is Some(confirmed_peak) do {
        emit_alarm(confirmed_peak)#;
    }

    return merged;
}
```

---

## 第三步（v2）：搭建多提示符故障注入沙盒，一键验证故障降级与事务一致性！

现在，我们在 `test` 块中化身“混沌工程（Chaos Engineering）测试官”！
我们不需要启动任何真实的硬件探头，也不需要修改 `aggregate_sensor_round` 的哪怕一个字符，直接通过嵌套安装 Handler：
- **注入故障**：规定当 `ch == 1u32`（主通道）时，故意返回 `error ProbeErr.Disconnected(1u32)`；当 `ch == 2u32`（备通道）时，正常返回读数 `180u32`！
- **验证恢复与告警**：主通道断连降级为 `0u32`，备通道返回 `180u32`，合并值 `180u32` 突破初始峰值 `100u32`，触发唯一一次 `emit_alarm(180u32)`！

```flurry
test {
    -- 共享统计面板状态
    var total_sum: u32 = 0u32;
    var peak_watermark: u32 = 100u32;
    var sample_count: u32 = 0u32;

    -- 混沌测试审计状态
    var remaining_budget: u32 = 50u32;
    var alarm_fired_count: u32 = 0u32;
    var last_alarm_value: u32 = 0u32;

    -- 多提示符（Multi-Prompt）嵌套安装：
    -- 内层处理 CoopSched（接入同步栈池 pool），外层处理 probe_channel 与 emit_alarm（接入 runtime）！
    let round_val = ((aggregate_sensor_round(
        1u32,
        2u32,
        total_sum.ref,
        peak_watermark.ref,
        sample_count.ref,
    )# {
        CoopSched.yield_tick(cost, continuation) => {
            if remaining_budget >= cost {
                remaining_budget -= cost;
                resume;
            }
        },
    }).use(pool)# {
        -- 注入确定性故障：1 号通道模拟断连，2 号通道正常返回 180！
        async probe_channel(ch) => {
            if ch == 1u32 {
                error ProbeErr.Disconnected(1u32)
            } else {
                180u32
            }
        },
        -- 拦截告警广播效应
        emit_alarm(new_peak) => {
            alarm_fired_count += 1u32;
            last_alarm_value = new_peak;
        },
    }).use(runtime);

    -- 输出验证：
    -- round=180, sum=180, peak=180, count=1
    -- budget=40, alarms=1, last_alarm=180
    println(
        "round={}, sum={}, peak={}, count={}",
        round_val,
        total_sum,
        peak_watermark,
        sample_count,
    );
    println(
        "budget={}, alarms={}, last_alarm={}",
        remaining_budget,
        alarm_fired_count,
        last_alarm_value,
    );
}
```

---

## 编译期安全防线终极大考：休想破坏事务纯度或跨越 C 边界泄漏效应！

让我们最后用两个负例，检验编译器是如何守护这套高并发架构的边界的：

```flurry
-- ❌ 防线 1：企图把 `emit_alarm` 或 `CoopSched.yield_tick` 塞进 STM `atomic` 块内部！
fn bad_effect_inside_stm(peak: *u32) -> #[emit_alarm] void {
    atomic(peak) {
        peak.* += 10u32;
        -- 编译期直接拒绝！
        -- `emit_alarm` 是未封闭的外部效应（P0），且 `yield_tick` 涉及切栈挂起，
        -- 均严禁出现在可投机回滚重试的 `atomic` 事务块内！
        emit_alarm(peak.*)#;
    }
}

-- ❌ 防线 2：企图将仍带有未处理效应行的函数直接作为 `^attrs.abi(.c)` 导出给 C 固件！
^attrs.abi(.c)
pub fn bad_c_firmware_entry(
    sum: *u32,
    peak: *u32,
    count: *u32,
) -> #[probe_channel, emit_alarm, CoopSched] u32 {
    -- 编译期直接拒绝！
    -- C ABI 入口函数的效应行必须为空！必须在 Flurry 包装函数内消去全部效应后才能交付给 C！
    return aggregate_sensor_round(1u32, 2u32, sum, peak, count)#;
}
```

---

## 第七卷（Episodes 141–160）全景知识回顾表

| 核心支柱 | 关键语法与构造 | 核心语义与安全保证 |
| :--- | :--- | :--- |
| **代数效应与错误正交** | `effect op(...) -> Y;`、`#[Eff] !Err T`、`expr#`、`expr!` | 错误走值判别返回通道（唯一标签去重）；效应走 Handler 证据路由（保留同名嵌套层次） |
| **多提示符限定 Continuation** | `catch continuation: builtin.Continuation`、`resume`、`.use(pool)` | 仅切片到选定 Handler 边界；Deep 恢复自动重装内层边界；单次消费（`!Copy`），放弃时沿隐藏中止通道执行 `Drop` |
| **无染色异步与结构化并发** | `async effect`、`async pat => ...`、`.await`、`.use(runtime)` | 请求时立即登记子工作并交付 `Future`；安装范围正常退出等待全部子工作，提前退出先取消并等待全部 `Drop` 完成 |
| **五级 ABI 与 C 边界屏障** | 直接处理 / `direct_passing` / 捕获 / `async`、`^attrs.abi(.c)` | 不捕获时不分配协程栈；C ABI 入口效应行必须为空，挂起栈含 C 帧时可恢复但严禁跨 C 帧静默放弃 |
| **STM 软件事务内存** | `atomic(id*) { ... }`、`atomic fn`、纯度金字塔 P0–P5 | 乐观读写日志消除死锁并支持组合；事务内仅允许受管读写与 `pure fn`（含私有局部 `var`），外部 I/O 遵循 Commit-Then-Act |

---

### 第七卷结语与第八卷预告

恭喜你！到这里，我们已经完整走完了《Flurry 演进手记》**第七卷：控制流的大一统——代数效应、限定 Continuation 与结构化并发**！
回望这一卷，你会发现：在别的语言里需要用七八套互不兼容的语法特例（异常、生成器 `yield`、`async/await` 染色状态机、依赖注入框架、协程库、锁管理）拼凑出来的控制流大厦，在 Flurry 中被**代数效应 + 限定 Continuation + 结构化范围 + STM** 以惊人的数学简洁性彻底统一在了同一套类型与资源安全屋檐下！

但是，我们的旅程还没有到终点！
在刚刚编写高并发网关和底层控制后端时，你可能注意到了几个更深邃的关键词：
- 我们在第三卷和本卷中见到了 `requires`、`ensures`、`decreases`、`invariant`、`predicate`、`spec fn` 与 `verified fn`；
- 在面对极致的硬件寄存器操作、SIMD 内联指令（`'intrinsic`）、位向量（`BitVec`）精确位段读写以及零开销形式化验证时：
  - Flurry 是如何区分 **PTC（编译期类型命题检查）** 与 **程序验证层（OSL 分离逻辑、状态精化与前后置契约证明）** 的？
  - 怎么用 **`refines { expr } Type` 精化类型** 和 **`BitVec`** 写出连一个比特位都不会算错的硬件驱动？
  - 当我们不得不进入 `unsafe` 边界时，编译器又是如何核验它的安全封装契约的？

准备好踏入《Flurry 演进手记》的最终高地——**《第八卷：程序验证、精化类型与系统级底层编程》**吧！我们下一卷见！
