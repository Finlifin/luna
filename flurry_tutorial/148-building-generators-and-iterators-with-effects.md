# Flurry 演进手记 (148)：用 50 行代码自己写一个生成器（Generator / Iterator）——基于代数效应的按需产出

> **系列导读**：在前两期（146–147）中，我们彻底掌握了单次限定 Continuation 的捕获、尾恢复（`resume`）以及未恢复放弃时的确定性 `Drop` 逆序清理保证。今天，是时候来一场酣畅淋漓的工程实战了！在许多主流语言中，为了实现“按需产出一系列值”的**生成器（Generator / `yield`）**，语言标准不得不专门增加 `yield` 关键字，并在编译器里硬编码一套脆弱的无栈状态机转换——甚至在子函数里调 `yield` 都不被允许（必须写成 `yield from`）。而在 Flurry 中，**无需任何专用生成器关键字**，短短几十行普通的代数效应代码，就能写出支持跨多层函数产出、支持流式中间件组合、且随时中途停止都能 100% 安全释放资源的工业级生成器！

---

## 一、传统流式处理的痛点：版本 `v0` 的“全量预分配”与“手写状态机地狱”

假设在我们的微型运行时中，有一个传感器采集序列：它会持续从硬件总线读取原始刻度、经过校准计算，并逐个产出给上层分析器。上层分析器只需要找到**第一个大于阈值 `50u32` 的异常峰值**，找到后就立刻停止采集并关闭传感器会话！

在没有代数效应的 `v0` 时代，你只有两条痛苦的路可走：
1. **笨办法 A（全量数组预分配）**：先把 10000 个传感器数据全部采完、存进一个大数组里返回，然后再去遍历数组找第一个大于 `50u32` 的数——哪怕第 1 个数就满足条件，后面 9999 次高昂的硬件采集也白做了，甚至面对无限数据流时直接内存撑爆！
2. **笨办法 B（手写 `Iterator::next(&mut self)` 状态机结构体）**：如果采集逻辑包含两层嵌套循环和三个子函数调用，你就必须把所有循环变量 `i`、`j`、局部资源 `SensorSession` 全部拆散，手动塞进一个巨大的 `struct SensorIterState` 里维护 `state_id: 0, 1, 2...`——原本 10 行清晰的顺序代码瞬间膨胀成 150 行面目全非的状态机！

现在，让我们看看代数效应是如何用最自然的顺序代码，直接赋予我们按需流式生成能力的！

---

## 二、迭代演进 `v1`：零切栈的流式生成器与管道中间件（Middleware Pipeline）

在写需要挂起栈帧的生成器之前，我们先看一个极其重要却常被忽视的事实：
如果你的消费者只是想**依次处理或过滤生成的每一个元素**，你甚至连 `catch continuation` 都不需要声明！直接用一个普通的 `effect emit(value: u32);`（甚至加上 `inline effect`），就能写出**零切栈开销的流式生成器与多层管道转换器**！

让我们在 `v1` 中编写一个数据源 `produce_readings`、一个将所有偶数翻倍的流式转换中间件 `double_evens`，以及最终的求和消费者：

```flurry
-- 1. 声明一个轻量级流式产出效应（返回 void）
effect emit(value: u32);

-- 2. 任意子函数都可以直接调用 emit(x)#，完全不需要像 Python 那样写 yield from！
fn emit_pair(base: u32) -> #[emit] void {
    emit(base)#;
    emit(base + 1u32)#;
}

fn produce_readings() -> #[emit] void {
    emit_pair(10u32)#;
    emit_pair(20u32)#;
}

-- 3. 流式中间件（Stream Transformer）：
-- 拦截内层产出的 value，只保留并翻倍偶数，然后向更外层的 emit 转发！
fn double_Only_evens() -> #[emit] void {
    produce_readings()# {
        emit(value) => {
            if value % 2u32 == 0u32 {
                -- 向外层处理器发出翻倍后的值！
                emit(value * 2u32)#;
            }
            -- 若为奇数则什么都不做，自然返回 void，主体继续产出下一个数！
        },
    };
}

test {
    var total = 0u32;
    -- 4. 终端消费者：累加所有经过管道过滤后的数值
    double_only_evens()# {
        emit(value) => {
            total += value;
        },
    };
    -- 原始产出：10, 11, 20, 21
    -- 经过 double_only_evens 过滤翻倍：20, 40
    -- 最终累加结果：60！
    println("stream total: {}", total);
}
```

### Code Cartoon：代数效应为什么是“天然可跨函数穿透”的超级生成器？

对比一下传统语言的 `yield` 与 Flurry 的 `#emit`：

```text
  [ 传统语言的无栈 Generator (如 Python / C# / Rust gen) ]
    fn produce() {
        helper();  <── ❌ helper() 只是普通函数，里面不能直接向 produce 的外层 yield！
    }                     必须把 helper 也改成 Generator 并層层写 yield from！

  ──────────────────────────────────────────────────────────────────────────────────

  [ Flurry 基于代数效应的生成器 ]
    double_only_evens()# { ... }
        └── produce_readings()
                └── emit_pair(10u32)
                        └── emit(10u32)#  ──► 🎯 直接穿透任意多层调用栈命中 Handler！
```

因为代数效应的作用域是动态按 Handler 边界匹配的（同时受静态类型行 `#[emit]` 严密检查），无论你的生成逻辑拆分成了多少个辅助函数、递归调用甚至高阶回调，只要签名上带着 `#[emit]`，随时随地一句 `emit(x)#` 就能直接把数据送到外层消费者手里！

---

## 三、终极演进 `v2`：支持“随时按需暂停、提前 `Abort` 且自动 `Drop` 传感器会话”的完整生成器

现在，让我们回到本节开头的终极需求：
1. 生成器 `sensor_stream()` 在开始采集前，打开了一个独占的硬件传感器会话 `let session = SensorSession { id: 1u32 };`（实现了 `Drop`，退出时必须关闭硬件会话）；
2. 生成器在循环中不断产出校准后的读数：`yield_reading(calibrated)#;`；
3. 外层消费者 `find_first_spike()` 只要读到**第一个大于等于 `50u32` 的异常峰值**，就立刻停止生成，直接拿走这个峰值，并且要求**挂起在半空中的 `SensorSession` 必须立刻确定性地执行 `Drop` 关闭硬件**！

这时候，我们只需要把前两期学过的 **`catch continuation: builtin.Continuation`**、**`resume`** 与 **放弃清理协议** 组合起来，短短几十行代码就能完美实现：

```flurry
-- 1. 声明带捕获许可的按需产出效应
effect yield_reading(value: u32, catch continuation: builtin.Continuation);

-- 2. 模拟持有底层硬件资源的传感器会话（必须通过 Drop 确定性关闭！）
struct SensorSession {
    id: u32,
}

impl Drop for SensorSession {
    fn drop(*self) -> void {
        println("[Drop] closing sensor session #{}", self.id.*);
    }
}

-- 3. 传感器流生成器：顺序编写，自然直观！
fn sensor_stream() -> #[yield_reading] void {
    let session = SensorSession { id: 1u32 };
    println("[Stream] session #{} opened", session.id);

    -- 模拟连续 5 次硬件采样：10, 30, 60(峰值!), 80, 90
    let raw_samples = [10u32, 30u32, 60u32, 80u32, 90u32];
    for idx in 0usize..5usize {
        let current = raw_samples[idx];
        println("[Stream] yielding sample: {}", current);
        -- 产出当前读数并挂起，等待外部消费者决定是否继续！
        yield_reading(current)#;
    }

    -- 如果消费者中途提前停止，下面这行代码绝不会执行！
    println("[Stream] all 5 samples exhausted");
}

-- 4. 按需消费者：找到第一个 >= 50u32 的峰值后立即停止，不再 resume！
test {
    var spike_found: u32 = 0u32;

    (sensor_stream()# {
        yield_reading(value, continuation) => {
            if value >= 50u32 {
                println("[Consumer] spike detected ({})! Stopping early.", value);
                spike_found = value;
                -- 🎯 关键时刻：我们找到了目标峰值 60u32！
                -- 这里故意【不调用 resume】，直接结束分支！
                -- continuation 在分支末尾离开作用域，立即触发隐藏中止通道，
                -- 沿着挂起的 sensor_stream() 栈帧逆序调用 session.drop()！
            } else {
                println("[Consumer] normal reading ({}), requesting next...", value);
                -- 尚未达到阈值，尾恢复生成器继续采下一个点！
                resume;
            }
        },
    }).use(pool);

    println("[Result] spike = {}", spike_found);
}
```

### Code Cartoon：`sensor_stream` 从挂起、按需恢复到提前中止清理的完整生命周期

让我们对照上面代码的控制台输出，欣赏这曲精妙的控制流交响乐：

```text
[Stream] session #1 opened               <── 1. 创建 session 资源，进入循环
[Stream] yielding sample: 10             <── 2. 第 1 次 yield_reading(10)# 挂起 (K1)
[Consumer] normal reading (10), requesting next...  ──► 调用 resume 消费 K1！
[Stream] yielding sample: 30             <── 3. 第 2 次 yield_reading(30)# 挂起 (K2)
[Consumer] normal reading (30), requesting next...  ──► 调用 resume 消费 K2！
[Stream] yielding sample: 60             <── 4. 第 3 次 yield_reading(60)# 挂起 (K3)
[Consumer] spike detected (60)! Stopping early.     ──► 分支不调 resume，丢弃 K3！
[Drop] closing sensor session #1         <── 5. 隐藏中止通道切回挂起栈帧，精准析构 session！
[Result] spike = 60                      <── 6. 栈槽归还 pool，外层拿到最终结果 60！
```

注意看：
- 第 4 个采样点 `80`、第 5 个采样点 `90` 以及末尾的 `all 5 samples exhausted` **根本没有执行**（真正的按需惰性求值，绝不做一丝一毫的多余工作！）；
- 而挂起在 `sensor_stream()` 栈帧顶部的 `session: SensorSession` 却在第 5 步**分秒不差地完成了 `Drop` 析构**！

---

## 四、本节核心概念速查表

| 生成器模式 | 声明写法 | 适用场景与性能特征 |
| :--- | :--- | :--- |
| **零切栈推流生成器** | `effect emit(val: T);`（无 `catch`） | 适合全量消费、流式 `map`/`filter` 管道组合；直接编译为函数调用或内联匹配 |
| **可挂起按需生成器** | `effect yield_item(val: T, catch c: builtin.Continuation);` | 适合按需暂停、交错合并多个流、或找到目标后**中途提前终止（Early Stop）** |
| **跨函数直接产出** | 子函数直接标注 `#[yield_item]` 并调用 `#` | 无需 `yield from` 语法糖，天然穿透任意深度的辅助函数调用链 |
| **提前停止资源保证** | 消费者分支不调用 `resume` 直接退出 | 自动触发 `Continuation` 放弃协议，逆序 `Drop` 生成器栈帧上所有活跃资源 |

---

## 五、下一期预告

看到代数效应连“中途停止并逆序清理栈帧”都能做到，一个极具深度的问题一定会浮现在你的脑海里：

> **“既然代数效应如果不调用 `resume` 就能直接中止内层计算并清理栈帧，这看起来已经完全包含了‘抛出错误并短路退出’的能力啊！为什么 Flurry 还要专门设计一套独立的 `!Err T` 错误限定类型？为什么 `AGENTS.md` 铁律特意强调‘错误结果与代数效应具有不同 ABI’？”**

这是一个区分“玩具级学术语言”与“工业级系统语言”的终极分水岭！
在下一期 **《Flurry 演进手记 (149)：错误 !Err 与效应 #Eff 的楚河汉界——为什么两者拥有截然不同的 ABI 与组合规则》** 中，我们将从底层机器码 ABI、行类型去重代数、以及独具匠心的**前置/后置 `catch` 错误重试机制**三个维度，彻底讲透 `!Err` 与 `#Eff` 为什么绝不能混为一谈！
