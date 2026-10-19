# Flurry 演进手记 (154)：深入效应 ABI 的发动机舱——证据传递（Evidence Passing）、池化栈帧与跨 C 边界限制

当一位 C/C++ 或 Rust 系统级老兵第一次看到 Flurry 的代数效应时，他在惊叹于代码表达力之余，脑子里一定会冒出一连串关于底层性能与 ABI 的拷问：
1. **“我只是用效应读个配置值或者打一行日志，编译器不会为了这玩意儿每次都去堆上分配协程栈、保存全部 CPU 寄存器吧？！”**
2. **“当函数层层调用时，内层发起一次 `read_number()#`，运行时是不是要像 C++ 异常栈展开（Stack Unwinding）那样一帧一帧往回慢吞吞地搜索 Handler？！”**
3. **“如果我只是想写一个同步生成器（Generator），难道非得把庞大的异步多线程调度器 `runtime` 链接进来吗？”**
4. **“当我们的边缘网关需要导出一个 C 语言 ABI 函数（`^attrs.abi(.c)`）给底层驱动调用时，效应行和 C 栈帧是怎么交互的？”**

今天，我们就直接掀开 `docs/algebraic-effect-and-delimited-continuation/effect-abi.md` 的发动机舱盖！你会发现，Flurry 的效应系统之所以能成为真正的系统级零妥协抽象，全靠它精妙的**五级 ABI 分类、证据传递（Evidence Passing）、可插拔池化栈后端（`.use(pool)`）以及严格的 C 边界屏障**！

---

## 尝试 v0：“把所有效应都当成重型协程切换”的性能灾难

在早期的玩具级代数效应解释器里，不管你的 Handler 有没有捕获 Continuation，每次调用效应操作都会触发一次完整的控制栈切片与堆分配。
这就好比你只是想问一下旁边的同事“今天几号”，结果每次都要先办停薪留职、打包全部办公桌物品搬进仓库、等同事回答完再把办公桌搬回来——这在每秒处理百万帧报文的物联网网关里是绝对不可接受的！

---

## 演进 v1：按需分级！看清 Flurry 效应系统的五种 ABI 交接类别

在 Flurry 中，**“只有真正需要保存剩余计算（声明了 `catch continuation` 或 `async`）时，才涉及 Continuation 的存储与恢复！”**
编译器在编译期就会根据效应声明与 Handler 形式，将交接方式精准划分为以下 **5 种 ABI 类别**（见 `docs/algebraic-effect-and-delimited-continuation/effect-abi.md`）：

| ABI 类别 | 源码声明特征 | 底层交接机制与开销 |
| :--- | :--- | :--- |
| **1. 错误结果（Error Result）** | `-> !Err T` | **零栈切换！** 走独立的返回值成功/错误判别标签与载荷通道，不含恢复权，绝不走异常栈展开 |
| **2. 普通直接处理（Direct Handling）** | `effect read_number() -> u32;` | **零协程栈分配！** 通过 **Evidence（证据入口）**直接调用 Handler 分支函数，拿到 `u32` 原地继续，开销等同于一次普通间接函数调用（甚至可内联消除）！ |
| **3. 直接值传递（Direct Passing）** | `^attrs.effects.direct_passing`<br>`effect installed_number() -> u32;` | **极致零调用开销！** 在安装 Handler 时直接取得固定值并保存在上下文槽中，后续请求直接读取该值，连分支函数都不重复执行！ |
| **4. 捕获处理（Captured Handling）** | `effect captured(catch k: builtin.Continuation) -> u32;` | **池化有栈切换！** 仅在此类声明下才允许捕获剩余计算，从显式供应的栈池（`pool`）领取地址稳定的执行栈 |
| **5. 异步处理（Async Handling）** | `async effect work() -> u32;` | **子任务登记 + Future！** 请求时向 `runtime` 登记子工作并返回 `Future`，`.await` 未就绪时走捕获挂起协议 |

让我们用代码直观对比前三种返回 `u32` 却拥有截然不同底层 ABI 的效应声明：

```flurry
-- 类别 2：普通直接处理（每次请求直接调用处理分支函数，无需切栈）
effect read_number() -> u32;

-- 类别 3：Direct passing（安装时一次性取得值，请求处直接读值，不重复执行分支！）
^attrs.effects.direct_passing
effect installed_number() -> u32;

-- 类别 4：捕获处理（显式声明 catch continuation，才允许挂起与切栈！）
effect captured_number(catch continuation: builtin.Continuation) -> u32;

fn direct() -> #[read_number] u32 { read_number()# }
fn installed() -> #[installed_number] u32 { installed_number()# }
fn captured() -> #[captured_number] u32 { captured_number()# }
```

### 什么是 Evidence Passing（证据传递）？

为什么 `direct()` 里面的 `read_number()#` 不需要像异常栈展开那样沿着调用栈一帧一帧往上翻找 Handler？
因为 Flurry 采用了 **Evidence Passing（证据传递）**机制：
编译器根据当前函数的允许效应行（如 `#[read_number]`）以及同名效应的层次（Occurrence），在底层将所选 Handler 入口的**证据向量（Evidence）**直接传递到位！发起直接效应请求时，直接通过 Evidence 里的入口指针一步到位命中目标 Handler，无需搜索整条动态调用链！

---

## 演进 v2：同步池化栈后端 `.use(pool)`——无需异步 Runtime 也能玩转 Continuation！

现在来看第 4 种类别：当我们需要捕获 Continuation（比如写一个同步的暂停/恢复器或生成器 `pause(catch continuation: builtin.Continuation)`）时，底层栈帧存放在哪里？

在 Flurry 中，**控制栈存储管理（Stack Pool）与异步任务调度器（Async Runtime）是完全解耦的！**
- 如果你在跑异步任务，你用 `.use(runtime)`，由 `runtime` 在内部封装并供应栈池；
- 如果你只是在写**纯粹同步的限定 Continuation**，你根本不需要引入任何异步 `runtime` 或事件循环，只需通过 **`.use(pool)`** 显式接入一个轻量级的同步栈池实例！

```flurry
effect pause(catch continuation: builtin.Continuation);

fn work() -> #[pause] u32 {
    pause()#;
    42u32
}

-- 无需安装任何 async runtime！只需 .use(pool) 供应同步执行栈池：
let result = (work()# {
    pause(continuation) => { resume; },
}).use(pool);

println("{}", result);
```
运行这段代码，直接输出 `42`！

让我们用一张 **Code Cartoon**（对应 `effect-abi.md` 中的双层栈池模型）看看当外层 `H_outer`（使用 `pool_outer`）与内层 `H_inner`（使用 `pool_inner`）嵌套捕获时，池化栈槽是如何精确流转并归还原主的：

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：多级池化栈槽的分配、捕获、恢复与按来源归还            |
|                                                                         |
|  [ 调用栈 S0 ] ──► 安装外层 H_outer (.use(pool_outer))                  |
|        │                                                                |
|        ▼  从 pool_outer 领取地址稳定的栈槽 S1                           |
|  [ 栈槽 S1 (来源: pool_outer) ] ──► 运行 H_outer 主体，                 |
|        │                            安装内层 H_inner (.use(pool_inner)) |
|        ▼  从 pool_inner 领取地址稳定的栈槽 S2                           |
|  [ 栈槽 S2 (来源: pool_inner) ] ──► 运行 H_inner 主体，                 |
|                                     发起对外层 H_outer 的捕获请求！     |
|                                                                         |
|  ⚡ 捕获与恢复全流程（常数次指针拆接！）：                              |
|  1. 捕获：Continuation `K` 持有 S1...S2 的执行链与内部处理关系，        |
|           每段栈各自牢记自己的出生池（S1 -> pool_outer, S2 -> pool_inner）|
|  2. 处理：切回选定边界外执行 H_outer 的处理分支                         |
|  3. 恢复：连接本次返回位置，重装内部关系，向请求点交付结果 Y            |
|  4. 完成：兑现所有局部 Drop 清理后，将 S1 归还给 pool_outer，           |
|           将 S2 归还给 pool_inner！绝不张冠李戴！                       |
+-------------------------------------------------------------------------+
```

---

## 演进 v3：跨越 C 语言边界（`^attrs.abi(.c)`）的两条铁律

在系统级编程中，我们的 Flurry 网关经常需要导出 C ABI 函数供底层 C 语言固件调用，或者在 Flurry 内部调用 C 语言库。

首先请牢记一条语法铁律（来自 `AGENTS.md` §5 与 `effect-abi.md`）：
> **在 Flurry 中，函数的外部 ABI 一律通过属性指定，即 `^attrs.abi(.c)`，绝不引入 `extern "C"` 关键字！**

而在语义上，跨 C 语言边界有两条救命的安全红线：

### 铁律 1：C ABI 入口函数的效应行必须为空！
C 语言的调用者根本不懂什么叫 Flurry 的 Handler Evidence。因此，任何标记了 `^attrs.abi(.c)` 的导出函数，其返回类型上**绝对不能留有未处理的效应行**！如果内部调用了带效应的 Flurry 函数，必须在边界包装层内用 `# { ... }` 把效应彻底消去：

```flurry
effect configured_limit() -> u32;

fn limit_times_two() -> #[configured_limit] u32 {
    let limit = configured_limit()#;
    limit * 2u32
}

-- ✅ 正确！使用 ^attrs.abi(.c) 指定 C ABI（不写 extern 关键字），
-- 并在包装层内部用 # { ... } 彻底消去 configured_limit 效应，对外呈现空效应行！
^attrs.abi(.c)
pub fn exported_limit() -> u32 {
    limit_times_two()# {
        configured_limit() => 8u32,
    }
}

-- ❌ 编译期直接拒绝！
-- C ABI 入口函数的效应行必须为空，不能要求外部 C 调用者供应 #[configured_limit] 的 Evidence！
^attrs.abi(.c)
pub fn bad_c_export() -> #[configured_limit] u32 {
    limit_times_two()#
}
```

### 铁律 2：挂起区域夹着 C 栈帧时，允许“挂起后恢复”，严禁“直接放弃（Abort）”！
想象一种极端情况：Flurry 安装了一个可捕获边界 $\rightarrow$ 调用了一个 C 语言函数 `c_bridge()` $\rightarrow$ C 函数又回调了一个 Flurry 函数并在其中触发了 `pause()#` 挂起！
这时候，被捕获的栈链中间夹着一帧 C 语言的栈帧（C Frame）！
- 如果 Handler 随后执行了 `resume;`**恢复计算**：C 栈帧会正常继续跑完并释放它在 C 里申请的资源——这是**允许**的！
- 但如果 Handler **放弃（Abandon / 不恢复直接丢弃）**了这个 Continuation：Flurry 的隐藏中止通道知道怎么沿途调用 Flurry 栈帧的 `Drop`，却根本不知道中间那帧黑盒 C 语言栈帧里锁了什么互斥量、开了什么文件！
因此，Flurry 严禁跨越 C 栈帧屏障执行放弃操作：如果编译器在静态阶段无法排除这种放弃，会在 C 帧边界插入屏障检查——一旦运行时真正发生跨越 C 栈帧的放弃，立即触发确定性的 **Trap**，绝不静默跳过未知 C 栈帧的资源清理责任！

---

## 本篇小结

| 核心机制 | 语法与底层设计 | 性能与安全保障 |
| :--- | :--- | :--- |
| **五级 ABI 分类** | 错误返回 / 普通直接 / `direct_passing` / `catch` 捕获 / `async` | 不捕获 continuation 的效应完全不分配协程栈，直接经 Evidence 调用或读值 |
| **Evidence Passing** | 按允许行与 occurrence 传递入口证据 | O(1) 定位直接效应入口，无需像异常展开那样逐帧搜索动态调用栈 |
| **显式同步栈池 `.use(pool)`** | `(work()# { ... }).use(pool)` | 同步挂起/恢复与异步调度彻底解耦；每段栈槽记录出生池并在清理后精确归还 |
| **C ABI 边界与屏障** | `^attrs.abi(.c)`（不使用 `extern` 关键字） | C 入口效应行必须为空；挂起栈含 C 帧时可恢复但严禁跨越 C 帧屏障静默放弃 |

---

### 留给下一篇的悬念

到现在为止，我们已经解决了异步任务的结构化并发与效应调度。
但是，当网关的多个并发任务（或者多个操作系统线程）需要同时读写**同一组共享的实时统计面板**（比如同时更新 `total_packets`、`error_packets` 和 `moving_average` 三个变量，并保持它们之间的数学不变量）时，并发编程中最臭名昭著的恶魔出现了：
**互斥锁（Mutex）与死锁（Deadlock）！**

- 如果锁的粒度太粗（整张表一把大锁），并发吞吐量直接跌入谷底；
- 如果锁的粒度太细（每个计数器一把小锁），线程 A 先锁 X 再锁 Y，线程 B 先锁 Y 再锁 X，系统在凌晨三点当场**死锁卡死**！更糟的是，两个各自正确的使用锁的函数，根本没法组合成一个更大的原子操作！

有没有一种机制，能让我们像数据库写事务（Transaction）一样，直接写一句 **`atomic(x, y) { ... }`**，不用管加锁顺序、绝不发生死锁、还能自由组合？
下一篇 **`155-software-transactional-memory-atomic-blocks.md`**，我们将正式开启 Flurry 并发编程的另一大杀器——**软件事务内存（Software Transactional Memory, STM）**！
