# Flurry 演进手记 (164)：异步流（Async Streams）与背压（Backpressure）的效应建模

> **阅读指南**：在上一回（第 163 回）末尾，我们抛出了高并发系统中最让人头疼的经典难题——**异步流（Async Streams）与背压控制（Backpressure）**。当上游数据生产者的速度远超下游异步 I/O 消费者的吞吐能力时，如何防止未消费的数据撑爆内存？本篇我们将结合 `docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md` 与 `multi-prompt-delimited-continuation.md`，看 Flurry 如何将 **生成器的限定 Continuation（`yield_item`）** 与 **结构化异步效应（`async effect` + `.await`）** 无缝组合，用最自然的 `resume` 握手协议实现零队列膨胀的精确背压。

---

## 一、 问题背景：无界缓冲区的 OOM 崩溃 vs. 手写状态机流控的泥潭

假设我们在开发一个边缘计算节点：
- **上游生产者 `read_sensor_stream()`**：从本地高速总线上源源不断地读取传感器采样值 `u32`；
- **下游消费者 `flush_to_cloud(sample)`**：需要通过慢速无线网络把每个采样值异步上传到云端（使用 `async effect upload(sample: u32) -> bool`）。

### 把生产者与消费者解耦成无界队列的问题

在很多异步框架里，初学者最喜欢用一个无界通道（Unbounded Channel）或动态数组把生产者和消费者连起来：

```flurry
-- 假想初始写法：无背压的“先全部生产进队列，再慢慢消费”
-- 问题 1（内存爆炸 OOM）：
--   上游 1 秒钟塞进队列 100,000 个采样包，下游无线网络 1 秒钟只能发走 500 个。
--   不到一分钟，边缘设备的内存就被积压的队列耗尽。
-- 问题 2（手写 Poll 状态机的痛苦）：
--   如果在传统语言里手写带背压的 AsyncStream 状态机，你得自己维护 Poll.Pending、
--   Waker 注册、水位线（High/Low Watermark）和取消清理，代码复杂度飙升十倍。
```

**核心洞察**：
为什么需要复杂的通道和水位线？因为生产者和消费者被割裂成了两个彼此无感知的盲目循环。
而在代数效应的世界里：**每一次 `yield_sample(val)#` 本质上都是生产者主动交出执行权（挂起 Continuation），等待 Handler 调用 `resume` 来唤醒它生产下一个值。**

---

## 二、 核心机制：用 `yield` 挂起 + `async` 消费 + `resume` 唤醒，天然形成背压闭环

让我们把 **同步/可挂起的流产出效应 `emit_sample`** 与 **异步 I/O 效应 `async effect transmit`** 组合在一起。

看看当我们在 `emit_sample(sample, continuation)` 的 Handler 分支内部，先发起异步上传并等待完成（或控制在途并发窗口），**最后才调用 `resume`** 时，会发生什么协同效果：

```text
  ┌─ 上游高速生产者 (Producer) ────────┐   ┌─ 流控 Handler 与异步下游 (Consumer) ───┐
  │                                    │   │                                        │
  │ 1. 产出第 1 个采样:                │   │                                        │
  │    emit_sample(10u32)# ────────────┼──▶│ 2. 拦截到 sample = 10u32 与恢复权 K₁   │
  │    (生产者在此挂起! 内存零积压!)   │   │    │                                   │
  │                                    │   │    ▼ 发起异步上传子任务并等待完成:     │
  │                                    │   │    let fut = transmit(10u32)#;         │
  │                                    │   │    let ack = fut.await;                │
  │                                    │   │    (等待无线网络 ACK 期间，K₁ 保持静止)│
  │                                    │   │    │                                   │
  │ 4. 生产者从挂起点苏醒，            │   │    ▼ 网络发送完毕，消费 K₁ 唤醒上游   │
  │    开始产出第 2 个采样:            │◀──┼─── resume ack;                         │
  │    emit_sample(20u32)# ────────────┼──▶│ 5. 拦截到 sample = 20u32 与新恢复权 K₂ │
  └────────────────────────────────────┘   └────────────────────────────────────────┘
```

看到了吗？
- **零额外缓冲区**：在下游 `transmit(10u32)#.await` 还没有完成之前，Handler 根本没有执行 `resume`。因此上游生产者安安静静地停留在第 1 次 `emit_sample` 的挂起点上，**连第 2 个采样值都还没有生成**。
- **双向信息反馈（Ack-Driven Backpressure）**：`emit_sample(sample: u32) -> FlowControl` 甚至可以有返回值。当下游通过 `resume .continue_producing` 恢复生产者时，生产者继续；如果下游网络断开或收到停止指令，下游只需 `resume .stop_stream`（或者干脆不 `resume` 直接退出 Handler 走放弃清理协议），上游生产者就能立刻优雅停机。

### 一拍一握手的严格步进异步流（Strict Step-Locked Stream）

让我们用真实的 Flurry 代码写出这套步进背压流：

```flurry
use std.prelude.*;

-- 1. 上游流产出效应：提交一个采样值，并等待下游返回是否继续的背压信号
effect emit_sample(
    sample: u32,
    catch continuation: builtin.Continuation,
) -> bool;

-- 2. 下游异步网络传输效应：启动异步子 task 将数据发往云端
async effect transmit_packet(payload: u32) -> bool;

-- 上游传感器生产者：只管按自己的节奏读取并 emit，一旦收到 false 信号立即优雅退出
fn sensor_producer() -> #[emit_sample] u32 {
    var count: u32 = 0u32;
    for raw in 1u32..4u32 {
        let keep_going = emit_sample(raw * 10u32)#;
        count += 1u32;
        if not keep_going {
            return count;
        }
    }
    count
}

test {
    -- 在结构化异步 runtime 下运行流式管道：
    let processed_count = (do {
        sensor_producer()# {
            emit_sample(sample, continuation) => {
                -- 先启动异步传输子任务，并 .await 等待远端确认完成。
                -- 在 .await 完成前，跳过执行 resume，上游生产者被自然背压挂起。
                let pending = transmit_packet(sample)#;
                let ok = pending.await;
                resume ok;
            },
        }
    }# {
        async transmit_packet(payload) => {
            println("async uploaded: {}", payload);
            true
        },
    }).use(runtime);

    println("total processed samples: {}", processed_count);
}
```

---

## 三、 进阶：窗口化流水线并发（Pipelined Window）与结构化范围核心规则

你可能会问：“一拍一握手（发完第 1 个才允许生产第 2 个）虽然内存占用是高度的 $O(1)$，但如果我想允许**最多 2 个数据包同时在网络上飞（In-Flight Pipelining）**，既有并发吞吐量，又严格限制最大内存占用至多 2 个包，该怎么写？”

在 Flurry 中，因为 **`async effect` 在发出请求 `transmit_packet(x)#` 的那一刻就已经登记并启动了子工作（而无需等到 `.await` 才启动。）**（见 `docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md`），我们可以先连续发出两个异步请求拿到两个 `Future`，然后再依次 `.await` 它们。

### 先并发派发双子工作，再按序 `.await` 兑现结构化完成

```flurry
use std.prelude.*;

async effect upload_chunk(chunk_id: u32) -> u32;

-- 有界窗口并发（最多 2 个子工作同时在途，不无界膨胀。）
fn upload_two_chunks_windowed(c1: u32, c2: u32) -> #[upload_chunk] u32 {
    -- 1. 连续发出两个异步效应请求：
    --    注意：请求发出时两个子 task 已经登记启动，无需等到 .await 才开始。
    let future_one = upload_chunk(c1)#;
    let future_two = upload_chunk(c2)#;

    -- 2. 等待当前窗口内的两个子工作全部完成后，才返回进入下一轮窗口。
    let ack1 = future_one.await;
    let ack2 = future_two.await;
    ack1 + ack2
}

test {
    let total_bytes = (upload_two_chunks_windowed(20u32, 22u32)# {
        async upload_chunk(chunk_id) => chunk_id,
    }).use(runtime);

    println("windowed upload total: {}", total_bytes);
}
```

> **💡 核心心智模型：异步效应与结构化并发的三大安全红线**
> 在结合 `async effect` 与流控制时，请牢记 `async-algebraic-effect.md` 的三条核心规则：
> 1. **丢弃 `Future` 有别于取消子工作，安装范围必须负责到底**：
>    即使你写了 `let _ = upload_chunk(7u32)#;` 而没有写 `.await`，子工作依然属于当前 `# { async ... }` 的结构化安装范围。该范围正常退出前会**自动等待所有未完成的派生子工作结束**；若因错误或提前返回退出，则会先发起取消并等待子工作完成资源清理后，才释放它们依赖的外层环境。
> 2. **同一个单次消费的 `Future` 或 `Continuation` 无法被复制或重复等待消费 Once 资源**：
>    如果异步操作的结果 `Y` 是只能消费一次的仿射资源（Once Resource），对同一个 `pending` 连续 `.await` 两次交付所有权会被编译器直接报错。
> 3. **声明为 `async effect` 的操作，其 Handler 分支必须显式写 `async` 标记**：
>    删掉分支前的 `async` 免于“自动帮你包一层已完成的 Future”，而是会因为入口类别失配被编译器静态拦截。

让我们看看漏写 `async` 分支修饰符或企图把并非 `Copy` 结果从同一个 `Future` 里移出两次时的编译期负例：

```flurry
async effect fetch_value(id: u32) -> u32;

fn broken_async_handler() -> u32 {
    (do {
        let fut = fetch_value(1u32)#;
        fut.await
    }# {
        -- ❌ 编译期直接报错：(Handler Entry Kind Mismatch)
        -- fetch_value 声明为 `async effect`，其处理分支必须以 `async fetch_value(id) => ...`
        -- 明确子工作归属与 Future 交付契约，不可省略 `async` 伪装成普通同步分支。
        fetch_value(id) => id + 1u32,
    }).use(runtime)
}
```

---

## 本篇小结

| 流控机制 / 特性 | Flurry 效应建模方式 | 解决的核心问题 |
| :--- | :--- | :--- |
| **步进背压（Step-Locked）** | 在 `emit_sample` 分支内先 `.await` 下游 `async` 操作，再 `resume` | 内存积压严格为 $O(1)$，下游未完成时上游生产者自动静止在挂起点 |
| **窗口化并发（Windowed）** | 先连续调用 $K$ 次 `async_op()#` 启动子工作，再集中 `.await` | 兼顾并发吞吐与严格的有界内存上限 $O(K)$ |
| **结构化范围兜底** | `# { async ... }.use(runtime)` 拥有全部派生子工作 | 退出范围前强制完成等待或取消清理，免生孤儿任务与悬垂栈引用 |
| **入口类别静态校验** | `async effect` 强制配对 `async pattern => ...` 分支 | 避免同步/异步交接协议错位 |

---

### 下一篇预告

在上面的异步流与解析器里，为了避免频繁调用操作系统的全局堆分配器（`malloc` / `free`），高性能系统往往会使用一种神级内存管理技术：**区域内存池 / 竞技场分配器（Arena / Pool Allocator）**。

它的工作方式是：在处理一个请求或一帧协议时，所有的临时对象都从局部 `Arena` 里高速切块分配（只需移动一个指针，耗时仅 1 纳秒。）；等这个请求处理完毕、离开 Handler 作用域时，**一次性重置整个 `Arena`**。

但这带来了一个严重的内存安全挑战：
**如果业务代码通过 `#ArenaAlloc` 效应从局部 Arena 里拿到了一块内存指针 `*Node`，万一业务代码偷偷把这个指针塞进了返回值或者外部容器里，带出了 Handler 的 `.use(...)` 作用域——而此时 Arena 已经被整块重置销毁了——岂不是立刻造成了破坏性的悬垂指针（Use-After-Free）？**

下一篇 **《Flurry 演进手记 (165)：资源池租借（Arena / Pool Lease）与 Handler 作用域生命周期绑定》**，我们将把第三卷的 **`&reachability_set`（可达性系统）** 与第七卷的 **效应 Handler** 双剑合璧，在编译期 100% 封死 Arena 指针逃逸。
