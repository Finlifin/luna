# Flurry 演进手记 (156)：为什么事务里禁止直接发网络包——STM 内部的纯度、重试（Retry）与效应隔离契约

在上一篇中，我们体验了 `atomic(vars) { ... }` 软件事务内存（STM）如丝般顺滑的无死锁并发编程。
但在文章末尾，我们提出了一个值得警惕的问题：

因为 STM 采用的是**乐观并发控制（Optimistic Concurrency）**，当两个线程同时修改同一个传感器告警阈值发生版本冲突时，STM 运行时会自动丢弃后一个事务的写集草稿，并**从 `atomic` 块的第一行重新执行一遍（Retry）**。
如果在高并发争用下，一个 `atomic` 块可能先投机执行了 3 次（前 2 次因为冲突回滚，第 3 次才成功提交）。

这时候，设想有人在 `atomic` 块里面写了下面三件事之一：
1. **发起了一个外部网络/硬件代数效应**：`fire_alarm_siren()#;`（拉响全厂区防空警报。）——结果因为事务冲突重试了 2 次，警报器被重复拉响了 3 次，甚至第一次投机读到存在分歧状态时误拉了警报，回滚内存却收不回已经发出去的无线电波。
2. **消费（Move）了一个外部传入的单次仿射资源（`Once` Resource）**：比如把一张只能使用一次的加密凭证 `ticket: Ticket` 按值传给了某个销毁函数——第 1 次投机执行就把 `ticket` 消耗掉了，等到冲突回滚要跑第 2 次时，`ticket` 已经没了。
3. **局部创建了一个带 I/O 副作用析构（`Drop`）的对象**：表面上看代码里没写 `println`，但离开块时 `Drop.drop` 偷偷往串口打了日志。

在 C++ 或 Java 里试图用库模拟 STM 时，这些问题全靠程序员“自觉遵守规范”，一旦手滑就会酿成线上事故。
而在 Flurry 中，得益于我们在第三卷学过的**运行时纯度六层金字塔（Runtime Purity P0–P5，`docs/resource-management/runtime-purity.md`）**、**仿射所有权系统**与**代数效应行（`#Eff`）**的深度联防，编译器在**编译期**就能为你筑起一道稳健可靠的**事务隔离防火墙**。

---

## 在事务块里夹带“覆水难收”的外部副作用

让我们看看初学者最容易写出的“带毒事务块”长什么样：

```flurry
effect send_alert(level: u32);

-- ❌ 编译期直接报错。
-- 企图在 atomic 事务块内部直接发起未封闭的外部 I/O 效应 `send_alert`。
fn bad_transactional_alert(threshold: *u32, current: u32) -> #[send_alert] void {
    atomic(threshold) {
        if current > threshold.* {
            threshold.* = current;
            -- 错误。若事务在此之后发生冲突回滚并重试，
            -- 外部效应 send_alert 无法像内存写集日志那样被时光倒流撤回。
            send_alert(current)#;
        }
    }
}
```

为什么编译器必须报错这段代码？
因为**内存日志可以回滚，但外部世界（I/O、网络包、硬件寄存器、已移出的单次资源）没有“撤销键（Undo）”。**

---

## 复习运行时纯度金字塔（P0–P5）——什么代码才能安全进驻 `atomic` 块？

要搞清楚编译器是怎么精确区分“事务内合法操作”与“非法副作用”的，让我们打开 `docs/resource-management/runtime-purity.md`，回顾 Flurry 的**运行时纯度六层金字塔**：

```
+-------------------------------------------------------------------------+
|  Flurry 运行时纯度金字塔与 STM 准入边界                        |
|                                                                         |
|          ▲          ┌──────────────────────────┐                        |
|         ╱ ╲         │ P5: 总值纯 (Total Pure)  │ ──► 确定有限完成 + P4  |
|        ╱───╲        ├──────────────────────────┤                        |
|       ╱  ✅ ╲       │ P4: 值纯 (pure fn)       │ ──► 仅依赖输入值与常量 |
|      ╱ STM   ╲      │     (允许私有局部 var  ) │     无外部可观察副作用 |
|     ╱  安全区 ╲     ├──────────────────────────┤                        |
|    ╱───────────╲    │ P3/P2: 受管事务只读观察   │ ──► 记入 STM Read Set  |
|   ╱─────────────╲   ├──────────────────────────┤                        |
|  ╱  ✅ 仅限受管  ╲  │ P1: 受管事务状态变更      │ ──► 记入 STM Write Set |
| ╱  atomic fn 读写 ╲ │     (无外界事件/可回滚)  │     冲突时可无损丢弃   |
| =================== ╪══════════════════════════╪ ====================== |
| 🚫 STM 严禁越界区   │ P0: 一般外部行为 (I/O、  │ ──► ❌ 不可回滚        |
|                     │     未消去外层效应、含   │     严禁出现在         |
|                     │     I/O 的 Drop、非受管写)│     atomic 事务块内    |
|                     └──────────────────────────┘                        |
+-------------------------------------------------------------------------+
```

从这张图里，我们可以立刻提炼出 `atomic` 事务块内部允许执行的三类安全计算：

1. **所有 `pure fn`（P4 / P5 值纯函数）**：
   注意 `docs/resource-management/runtime-purity.md` 中那条十分人性化的规则：**在 `pure fn` 内部使用局限于内而未逃逸的私有局部可变变量（`var`）及其局部指针，完全无伤于值纯（P4/P5）。** 因为每次调用创建的私有栈帧临时存储在外部完全不可观察，事务重试时重新算一遍总是得出相同的结果。
2. **受 STM 日志接管的事务读写与 `atomic fn`**：
   对声明在 `atomic(...)` 中的受管变量进行的读取（记入 Read Set）和写入（记入 Write Set），以及调用其他 `atomic fn`，在冲突回滚时只需直接丢弃私有日志，免染外部世界。
3. **在事务块内部已经完全自给自足、且处理过程本身满足纯度要求的局部效应**：
   如果一段计算使用了局部代数效应，但它的 Handler 就在事务块内部闭合，且组合后的整体行为零产出 P0 外部事件或外部非受管写，纯度分析器同样能够识别其安全性。

让我们来看一段在 `atomic` 块中愉快调用含局部 `var` 的 `pure fn` 与 `atomic fn` 的合法示例：

```flurry
-- ✅ 1. 声明为 pure fn（满足 P4/P5 值纯）：
--    内部使用了局部可变变量 `var result`，但它属于本次调用的私有存储，
--    没有外部别名或逃逸，在 STM 事务重试中任意次调用都内存安全。
pure fn clamp_watermark(candidate: u32, ceiling: u32) -> u32 {
    var result = candidate;
    if ceiling < result {
        result = ceiling;
    }
    result
}

-- ✅ 2. 声明为 atomic fn：读写受 STM 事务日志保护
atomic fn try_raise_watermark(watermark: *u32, sample: u32, max_limit: u32) -> bool {
    let clamped = clamp_watermark(sample, max_limit);
    if clamped > watermark.* {
        watermark.* = clamped;
        return true;
    }
    false
}
```

---

## “事务内决策，提交后执行（Commit-Then-Act）”——优雅解耦 STM 与外部效应

现在回到我们开头的业务需求：
**“当传感器读数突破历史最高水位线 `watermark` 时，我们确实既要原子地更新 `watermark`，又要向外部发送一条 `send_alert(new_val)#` 告警效应。既然禁止于 `atomic` 块里面发，该怎么写最地道？”**

答案就是所有事务系统最经典的黄金设计模式：**Commit-Then-Act（事务内决策并返回动作凭证，事务提交后再触发外部效应）**。

下面看看结合契约与分阶段检查后的实现：

```flurry
effect send_alert(level: u32);

-- ✅ 改进写法：将“内存状态原子迁移”与“外部单向 I/O 效应”干净分离。
fn observe_and_alert(
    watermark: *u32,
    sample: u32,
    max_limit: u32,
) -> #[send_alert] void {
    -- 第一步：在 atomic 块内纯粹地完成并发冲突检测、水位线计算与状态提交，
    --         并将“是否需要报警及报警数值 (?u32)”作为事务的返回值交出来。
    let alert_to_fire: ?u32 = atomic(watermark) {
        if try_raise_watermark(watermark, sample, max_limit) {
            watermark.*
        } else {
            null
        }
    }

    -- 第二步：一旦走到这里，说明 atomic 事务已经 100% 原子提交成功，不再回滚。
    --         此时再根据返回值触发外部代数效应 send_alert，保证警报只响唯一一次。
    if alert_to_fire is confirmed_level? do {
        send_alert(confirmed_level)#;
    }
}
```

```
+-------------------------------------------------------------------------+
|  “Commit-Then-Act” 模式的事务与效应边界                        |
|                                                                         |
|  ┌─── atomic(watermark) { ... } （可回滚重试区：P1 受管读写 + P4 纯计算）┐|
|  │  • Attempt 1: 读 watermark=50, 准备写 80 ──► 💥 冲突  自动回滚重试   │|
|  │               （此时没有任何网络包发出  ）                           │|
|  │  • Attempt 2: 读 watermark=60, 准备写 80 ──► 🟢 验证通过，原子提交   │|
|  └──────────────────────────────┬──────────────────────────────────────┘|
|                                 │ 交付确定返回值：`Some(80u32)`         |
|  ===============================┼====================================== |
|                                 ▼ （已提交不可逆区：P0 外部效应区）     |
|                  `send_alert(80u32)#;`                                  |
|                  （严格执行且仅执行 1 次  不会重放、不会误报  ）        |
+-------------------------------------------------------------------------+
```

---

## 编译期安全红线：警惕伪装成“安静离开”的 P0 `Drop` 与单次资源（`Once`）消耗

除了显式的 `send_alert()#` 之外，`docs/resource-management/runtime-purity.md` 还提醒我们注意两类富有隐蔽性的违约行为，编译器同样会在 `atomic` 块或 `pure fn` 检查中将它们精准揪出：

### 红线 1：源码里没写 I/O，但局部变量的 `Drop` 析构函数里有 I/O
正如 `runtime-purity.md` 示例 8 所示，如果一个结构体 `Trace` 的 `Drop.drop` 方法里调用了 `println("drop {}", self.id.*)`，那么哪怕你在函数里只是声明了一个 `let local = Trace { id: 7 };` 然后什么都不干就离开作用域，它的隐式退出清理（Cleanup）依然会产生 P0 级别的外部 I/O。

```flurry
struct NoisyTrace {
    id: u32,
}

impl Drop for NoisyTrace {
    fn drop(*self) {
        -- 析构函数包含外部 I/O 输出（属于 P0 一般行为）。
        println("dropping trace {}", self.id.*);
    }
}

fn bad_hidden_drop_in_atomic(counter: *u32) {
    atomic(counter) {
        -- ❌ 编译期直接报错。
        -- 虽然代码块表面没有显式调用 println，
        -- 但 `local` 在退出作用域（包括冲突回滚清理）时触发的 `NoisyTrace.drop`
        -- 属于 P0 外部 I/O 行为，严禁在可回滚重试的事务块或 pure 函数中隐式执行。
        let local = NoisyTrace { id: 1u32 }
        counter.* += 1u32;
    }
}
```

### 红线 2：禁止于可能重试的事务块中按值消耗（Move）外层单次（`Once`）资源
正如 `runtime-purity.md` 强调的：**“无内存效果 $\neq$ 可随意复制调用。输入或结果携带 `Once` 责任。”**
如果你在 `atomic` 块里把外层捕获的一个并非 `Copy` 单次资源 `ticket: Ticket` 按值移交消费掉，一旦事务发生回滚重试，第二次执行就没有合法的 `ticket` 可以交付了。编译器会通过仿射责任流分析直接报错这种跨重试边界的单次消耗。

---

## 本篇小结

| 检查维度 | 在 `atomic` 事务块内的规则 | 为什么这样设计（工程原理） |
| :--- | :--- | :--- |
| **`pure fn`（P4/P5）与局部 `var`** | ✅ **完全允许** | 私有临时栈存储对外不可观察，事务冲突重跑任意次结果均一致且无残留副作用 |
| **受管变量读写与 `atomic fn`** | ✅ **完全允许** | 读写操作被拦截进 STM 私有 Read/Write 日志，冲突时直接丢弃草稿即可无损回滚 |
| **未消去的外层效应与 P0 I/O** | ❌ **编译期报错** | 外部网络包、磁盘写入或硬件控制一旦发出无法随内存回滚撤销；应采用 **Commit-Then-Act** 模式在事务外执行 |
| **带 I/O 的 `Drop` 与外层 `Once` 消耗** | ❌ **编译期报错** | 隐式析构同样纳入纯度摘要检查；单次仿射资源不可在可能执行多次的重试循环中被重复消耗 |

---

### 下一篇预告

在探索了异步效应（`async effect`）、底层 ABI（`.use(pool)`）以及软件事务内存（`atomic`）之后，我们的视野即将再次拔高，直面代数效应与限定 Continuation 最核心的控制流机制——**多提示符栈（Multi-Prompt Stack）上的嵌套路由**。

想象一下，在一段复杂的边缘网关处理管线里，调用栈上像夹心饼干一样同时安装了三层不同的 Handler（提示符 Prompt）：
- **最外层（Prompt 1）**：安装了一个用于全局早退回滚的 `abort_batch` Handler；
- **中间层（Prompt 2）**：安装了一个用于流式产出解析结果的 `yield_item(val, catch k)` 生成器 Handler；
- **最内层（Prompt 3）**：安装了一个提供校准偏移量的 `ask_offset(catch k)` Handler。

当最内层的代码一会儿触发 `ask_offset()#`、一会儿跨越内层直接触发中层的 `yield_item(x)#`、甚至触发同名嵌套效应时：
1. 编译器和运行时是怎么在同一个调用栈上精准切下从请求点到**目标那一层边界**之间的栈帧切片（Delimited Continuation），而不误伤更外层的栈帧的？
2. 当中层的 `yield_item` 恢复（`resume`）那段被切下来的计算时，原本夹在它肚子里的内层 `ask_offset` Handler 还能正常工作吗？
3. **尾恢复（Tail `resume`）**与**非尾恢复（Non-tail Resumption）**在栈帧保留上又有什么本质区别？

下一篇 **`157-multi-prompt-stacks-and-scoped-handler-routing.md`**，我们将通过“多层书签栈帧”模型，详细讲解多提示符限定 Continuation 的精准导航法则。
