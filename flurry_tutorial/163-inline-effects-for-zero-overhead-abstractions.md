# Flurry 演进手记 (163)：`inline effect`——在编译期完全内联消去的零开销效应协议

> **阅读指南**：在上一回（第 162 回）中，我们看到无参数的 `direct_passing` 效应可以在安装时求值一次并直接传值。但如果一个效应操作**带有参数**（比如每次请求传入当前迭代元素 `ask(value: bool) -> bool`），或者中间还隔着多层 **部分处理与外层转发（Partial Handler & Outer Forwarding）**，我们能不能让编译器把生产者函数、中间过滤层 Handler 以及最外层 Handler **联合内联展开（Joint Inlining）**，把动态效应分派彻底编译成一段扁平的局部 `match` 控制流？本篇我们将结合 `docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md` 与 `docs/grammar/syntax`，深入探索 **`inline effect`**！

---

## 一、 痛点：当高频内循环遇上多层效应拦截，间接调用的开销怎么破？

假设我们在编写一个高频信号过滤器或流式算子管线：
- 最内层的 `produce()` 函数在每次循环中不断发出带参数的请求 `ask(value)#`；
- 中间层 `filtered()` 安装了一个**部分处理器（Partial Handler）**：它只拦截 `ask(true)` 并直接回答 `false`，而遇到 `ask(false)` 时则不匹配，自动向外层转发；
- 最外层 `entry()` 安装了兜底处理器，处理所有漏出来的 `ask(value)` 请求。

### 审视 `v0`：普通 Evidence 间接调用在纳米级热循环里的成本

如果按照普通的效应运行期路由（`v0`）：
- 每次执行 `ask(value)#`，程序都要通过当前环境中的 Evidence 指针发起一次间接函数调用（Indirect Call）；
- 当中间层 `filtered()` 发现模式 `ask(true)` 不匹配 `false` 时，还要再沿着外层 Evidence 链转发调用外层的 Handler 函数，完成后再返回内层继续下一次请求！

对于每秒执行上亿次的像素处理、音频采样或硬件轮询内循环来说，哪怕是不切栈的间接函数调用，也会阻碍编译器做跨边界的常量传播与死代码消除！

---

## 二、 破局（v1）：`inline effect`——调用方、被调方与 Handler 的三方联合展开！

在 Flurry 的全局语法（`docs/grammar/syntax` 与 `algebraic-effect.md`）中，效应声明和 `effect enum` 的变体前都可以直接加上 **`inline`** 关键字：

```text
algebraic_effect -> async? inline? effect id(parameter*) (-> result)? clauses?
effect_variant   -> attribute* async? inline? id(parameter*) (-> result)?
```

当你声明了 **`inline effect`** 并配合 `inline fn` 时，Flurry 编译器的内联优化器会启动强大的 **联合展开（Joint Expansion）**：
1. 先将 `inline fn` 被调用方（Callee）展开到安装 Handler 的位置；
2. 再把主体中的每一个 `ask(arg)#` 请求，**原地替换为一段经过卫生重命名（Hygienic Renaming）的局部 `match` 分支**！
3. 如果存在多层嵌套安装或部分处理（Partial Handler），展开器会自动在未匹配的候选之后生成**最后一条外层转发分支**，并逐层向内消除，最终把多层效应管线压平为没有任何间接调用的纯局部算术与布尔指令！

让我们用一张 Code Cartoon，看清包含“部分拦截 + 外层转发”的三层 `inline effect` 是如何在编译期被彻底消骨化简的：

```text
  源码：三层 inline 效应管线
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ inline effect ask(value: bool) -> bool;                                 │
  │ inline fn produce() -> #ask bool {                                      │
  │     let a = ask(true)#;  let b = ask(false)#;  let c = ask(true)#;      │
  │     not a and b and not c                                               │
  │ }                                                                       │
  │ inline fn filtered() -> #ask bool {                                     │
  │     produce()# { ask(true) => false, }  -- 部分处理：仅拦截 true!       │
  │ }                                                                       │
  │ fn entry() -> bool {                                                    │
  │     filtered()# { ask(value) => not value, } -- 外层兜底处理            │
  │ }                                                                       │
  └───────────────────────────────────┬─────────────────────────────────────┘
                                      │
                                      ▼ 联合内联展开 (Joint Inline Expansion)
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 第 1 步：展开 filtered() 中的部分处理器，未命中分支自动转为外层 ask#：  │
  │   • ask(true)#  ──▶ 命中内层 `ask(true) => false`  ──▶ a = false        │
  │   • ask(false)# ──▶ 未命中内层，自动转发外层 ask(false)#                │
  │   • ask(true)#  ──▶ 再次命中内层（注意：转发绝不卸载内层 Handler！）    │
  │                                  │                                      │
  │ 第 2 步：继续展开 entry() 的外层处理器 `ask(value) => not value`：      │
  │   • b = not false = true                                                │
  │   • 最终生成纯局部代码：not false and true and not false ===> true！    │
  │   ✨ 零 Evidence 表、零间接调用、零栈帧切换！                           │
  └─────────────────────────────────────────────────────────────────────────┘
```

### 演进到 `v1`：部分处理转发与效应行槽位映射（Slot Mapping）实战

让我们把上面这段来自 `docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md` 的经典代码写出来，并看看当内层函数只使用了外层效应行的一个**子集**（或声明顺序不同）时，联合展开器是如何精准对齐槽位的：

```flurry
use std.prelude.*;

-- 1. 声明带内联意向的 inline effect
inline effect ask(value: bool) -> bool;

inline fn produce() -> #ask bool {
    let a = ask(true)#;
    let b = ask(false)#;
    let c = ask(true)#;
    not a and b and not c
}

-- 中间层：只拦截 ask(true)，将 ask(false) 转发给外层！
-- 注意：第 2 次请求 ask(false)# 转发给外层后，内层 Handler 依然保留在原位，
-- 因此第 3 次请求 ask(true)# 依然由内层 filtered 回答 false！
inline fn filtered() -> #ask bool {
    produce()# {
        ask(true) => false,
    }
}

fn entry() -> bool {
    filtered()# {
        ask(value) => not value,
    }
}

-- 2. 效应行子集与不同顺序的槽位映射（Slot Mapping）
inline effect read() -> bool;
inline effect poll() -> bool;

-- only_poll 只请求 #poll（在它自己的效应行里处于槽位 0）
inline fn only_poll() -> #poll bool {
    poll()#
}

-- wider 声明了更宽的效应行 #[read, poll]（此时 poll 处于槽位 1！）
inline fn wider() -> #[read, poll] bool {
    only_poll()
}

fn mapped_entry() -> bool {
    -- ✅ 联合展开器逐层组合 EffectRun 槽映射，
    -- 准确将 only_poll 的请求对准 poll() => true 分支，绝不会误调槽位 0 的 read()！
    wider()# {
        read() => false,
        poll() => true,
    }
}

test {
    println("filtered entry: {}", entry());
    println("mapped entry: {}", mapped_entry());
}
```

---

## 三、 严格语义边界：`inline effect` 绝不改变求值次数、控制目标与未覆盖检查！（v2）

在使用 `inline effect` 时，有三条极其重要的语义红线（见 `algebraic-effect.md`）：

> **💡 核心心智模型：`inline` 是编译期展开意向，绝不是篡改语言语义的特权！**
> 1. **实参严格求值一次，候选失败先撤销再转发**：
>    当 `ask(produce_arg())#` 被展开为局部 `match` 时，请求实参 `produce_arg()` **严格在进入分支匹配前只求值一次**！即使内层部分处理器的模式或守卫（Guard）匹配失败、需要转发给外层处理器，也直接转发已求出的载荷，**绝不会把 `produce_arg()` 重新执行第二遍**，也不会凭空给仿射载荷授予 `Copy`！
> 2. **`resume` 与 `return` / `break` / `continue` 的边界泾渭分明**：
>    在展开后的 Handler 分支中：
>    - 写 **`resume val;`** 只跳出当前这一个效应请求的私有结果边界，把 `val` 交回给请求点，被处理的主体继续往下跑；
>    - 而如果在 Handler 分支里直接写 **`return`**、**`break`** 或 **`continue`**，它们保留的是**安装该 Handler 所在的外层函数或外层词法循环目标**！展开器会自动生成清理被退出的生产者与中间层局部资源的析构代码，绝不会让 `resume` 的私有边界误吞掉外层的 `return`！
> 3. **未覆盖的请求若无外层 Handler 兜底，编译期直接拒绝！**
>    部分处理器（如只写了 `ask(true) => false`）之所以合法，是因为 `filtered() -> #ask bool` 在签名上声明了还会向外抛出 `#ask`！如果你在一个宣称已经彻底消去 `#ask` 的函数里漏掉了 `ask(false)` 分支，编译器绝不会把“没匹配上”当成隐式成功，而是直接按非穷尽处理报错！

### 演进到 `v2`：验证 `resume` 与词法 `return` 的精确边界，并拦截未覆盖请求

```flurry
inline effect transform(val: u32) -> u32;

inline fn compute_pair() -> #transform u32 {
    let first = transform(0u32)#;
    let second = transform(10u32)#;
    first + second
}

-- 演进到 v2：在 inline effect 分支中精准使用 resume 提前交付单次请求结果
fn run_inline_transform() -> u32 {
    compute_pair()# {
        transform(val) => {
            if val == 0u32 {
                -- ✅ resume 仅完成本次 transform(0u32)# 请求（交付 1u32），
                -- compute_pair 继续往下执行第二次请求 transform(10u32)#！
                resume 1u32;
            }
            val * 2u32
        },
    }
}

fn broken_partial_handler() -> bool {
    -- ❌ 编译期直接拒绝！
    -- broken_partial_handler 的返回类型是纯 bool（未声明向外抛出 #ask），
    -- 但 Handler 只覆盖了 ask(true)，当 produce() 请求 ask(false)# 时既无内层匹配也无外层入口！
    produce()# {
        ask(true) => false,
    }
}

test {
    -- 1u32 + (10u32 * 2u32) == 21
    println("transformed: {}", run_inline_transform());
}
```

---

## 本篇小结

| 特性维度 | 普通直接效应（`effect`） | 内联效应（`inline effect`） |
| :--- | :--- | :--- |
| **编译期转换** | 生成经由 Evidence 入口的调用指令 | 联合展开调用方、被调方与 Handler，化简为卫生重命名的局部 `match` |
| **部分处理与转发** | 未命中时通过运行期外层路由转发 | 自动在候选末尾合成外层转发分支，支持多层嵌套逐层消除 |
| **效应行子集映射** | 运行期按槽位表传递 | 编译期组合 `EffectRun` 槽位映射，精准对齐操作身份与类型实参 |
| **语义不变性保证** | 实参求值一次；区分 `resume` 与词法 `return` | 与普通效应 **100% 保持相同可观察语义、求值次数与资源清理次序** |

---

### 留给下一篇的悬念

从第 161 回到第 163 回，我们的回溯解析器、结构化追踪和 `inline effect` 都在同一个同步执行流里运转。

但是，在真实的物联网网关、音视频处理或高并发微服务中，我们经常面临一个经典的**异步流（Async Stream）失速难题**：
- 上游是一个高速生产者（比如每秒产生 10 万条采样数据的传感器协程，或者高速读取磁盘文件的解析器）；
- 下游是一个慢速消费者（比如每秒只能通过无线网络发送 1000 条数据的异步网络发射器 `async effect send_packet(...)`）。

如果上游不管三七二十一拼命往内存队列里塞数据，不到几秒钟内存就会彻底撑爆（OOM）！这就是工程界闻之色变的 **背压问题（Backpressure）**。

在 Flurry 中，我们如何把 **生成器效应（`yield_item`）** 与 **异步效应（`async effect` + `.await`）** 结合起来，利用 `resume` 的控制权交接，不写一行复杂的锁或环形队列，就自然实现**“下游没发完，上游就自动挂起等待”**的零缓冲区暴涨背压控制？

下一篇 **《Flurry 演进手记 (164)：异步流（Async Streams）与背压（Backpressure）的效应建模》**，我们马上揭晓！
