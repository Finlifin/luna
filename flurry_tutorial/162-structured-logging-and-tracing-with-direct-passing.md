# Flurry 演进手记 (162)：用 Direct Passing 实现零全局变量的结构化日志与分布式追踪（Tracing）

> **阅读指南**：在上一回（第 161 回）中，我们用状态可变的标准效应实现了游标回溯。但在构建大型系统软件时，还有一类无处不在的需求：**隐式环境配置与结构化追踪（Structured Logging & Distributed Tracing）**。我们希望在请求入口绑定 `request_id`、`channel_id` 或日志级别，让深层调用链里的任何函数都能按需读取其中一两项，还能在子模块里临时覆盖（Override）某个配置项，离开子模块后自动恢复原值。结合 `docs/algebraic-effect-and-delimited-continuation/direct-passing.md`，本篇我们将展示如何用 **`^attrs.effects.direct_passing`** 打造零全局变量、零重复求值、按需精确传参的结构化追踪系统。

---

## 一、 问题背景：“大泥球 `Context` 参数” vs. “全局 TLS 污染”

假设我们在写一个存储引擎，一次写请求会依次经过：`handle_request` $\to$ `validate_schema` $\to$ `flush_wal` $\to$ `write_block`。
为了在出问题时能排查故障，我们希望底层日志能自动带上当前的 `request_id`（请求编号）、`trace_depth`（追踪层级）以及 `max_retries`（最大重试次数）。

### 传统方案的三大两难困境

在传统系统语言里，你通常只能从以下三种痛苦的方案里选一个：

```flurry
-- 传统方案 1（显式传递庞大的 Context 结构体）：
struct FullAppConfig {
    request_id: u32,
    trace_depth: u32,
    max_retries: u32,
    db_timeout_ms: u32,
    -- ... 还有 50 个其他模块的配置项。
}
-- 弊端 A：哪怕 write_block 只用到了 request_id 一项，也必须接收整个 FullAppConfig。
-- 弊端 B：如果内层函数想临时把 trace_depth + 1 传给下层，
--         就得把包含 50 个字段的 FullAppConfig 重新拷贝构造一份。

-- 传统方案 2（普通按次执行的代数效应）：
-- 如果用普通 effect request_id() -> u32，每次打日志读取 request_id()# 都要调用一次
-- Handler 分支代码。如果在安装分支里写了一句初始化计算（如生成唯一 ID），
-- 每次读取都会重新执行一遍生成逻辑。
```

**核心问题**：
我们需要一种专门針對“配置与追踪上下文”的效应协议：
1. **安装时求值一次（Evaluate Once at Installation）**：在进入 `# { ... }` 作用域时，提供值的分支**严格只执行一次**，把算出的 `request_id` 存进当前安装实例的值槽里；
2. **按需细粒度传递（Fine-Grained Implicit Passing）**：一个函数如果只写了 `#TraceEnv.request_id`，编译器就只给它传递 `request_id` 这一个值，全然无需构造包含全部配置的大对象。
3. **局部无损覆盖（Scoped Local Override）**：内层可以随时安装一个新的 Handler 覆盖某一项（如 `trace_depth`），不篡改外层实例的值。

---

## 二、 核心机制：`^attrs.effects.direct_passing`——安装即就绪，按需直接传值

在 Flurry 中（见 `docs/algebraic-effect-and-delimited-continuation/direct-passing.md`），只需在无参数的效应声明（或 `effect enum` 的具体变体）上加上 **`^attrs.effects.direct_passing`** 属性，就能瞬间激活这套“安装时求值 + 直接传值”协议。

下面对比“普通效应（每次请求调用分支）”与“Direct Passing 效应（安装时求值一次，后续直接从槽位/寄存器取值）”的执行轨迹：

```text
  ┌─ 1. 普通 Effect：每次请求都跳回 Handler 分支执行 ──────────────────────┐
  │   主体执行： req()# ──▶ 跑一遍分支!   ...   req()# ──▶ 又跑一遍分支!   │
  └─────────────────────────────────────────────────────────────────────────┘

  ┌─ 2. Direct Passing Effect (`^attrs.effects.direct_passing`) ───────────┐
  │                                                                         │
  │  阶段一 [安装时就绪]：在主体开始前，先在外层环境执行提供分支【仅 1 次】! │
  │         算出结果（如 request_id = 9527），存入本安装实例的就绪槽 (Slot) │
  │                                  │                                      │
  │                                  ▼                                      │
  │  阶段二 [直接传值执行]：将就绪值通过隐式参数/寄存器直接传入调用链       │
  │         • 第 1 次读取 `Trace.request_id()#` ──▶ 直接读槽拿到 9527 (O(1))│
  │         • 第 2 次读取 `Trace.request_id()#` ──▶ 直接读槽拿到 9527 (O(1))│
  │         • 内层局部覆盖 `Trace.depth() => 2` ──▶ 建立新内层槽，外层槽不变│
  └─────────────────────────────────────────────────────────────────────────┘
```

> **💡 核心心智模型：`direct_passing` 并非止是“优化提示”，而是严格的语义契约。**
> 请特别注意 `direct-passing.md` 中的两条核心原则：
> 1. **无论主体请求 0 次、1 次还是 100 次，安装时的提供分支严格执行 1 次。**
>    即使被处理的主体最终一次都没调用 `limit()#`，安装 Handler 时提供分支中的可观察动作（如打印 `"install limit"` 或分配 Span ID）依然会在主体开始前发生一次。编译器免因主体没读它就偷偷删掉安装分支的副作用。
> 2. **细粒度单项订阅 vs. 整组订阅**：
>    在 `effect enum Trace` 组里，如果某个辅助函数只关心请求 ID，它的签名可以直接写 **`#Trace.request_id`**；编译器在做 Elaboration 时，只向该函数传递 `request_id` 这一项，全然无须把同组的 `depth` 或 `channel` 一起传过去。

### 构建按需订阅的结构化追踪系统

让我们用 `effect enum Trace` 写出这套零全局变量的结构化日志与追踪代码：

```flurry
use std.prelude.*;

-- 定义支持 direct_passing 的结构化追踪效应组
effect enum Trace {
    ^attrs.effects.direct_passing
    request_id() -> u32,

    ^attrs.effects.direct_passing
    depth() -> u32,
}

-- 辅助函数 A：只要求 #Trace.request_id 一项。
-- 调用它时，无需为其准备或传递 depth。
fn log_metric(metric_val: u32) -> #Trace.request_id u32 {
    let req = Trace.request_id()#;
    let second_read = Trace.request_id()#;
    println("[req={}] metric={}", req, metric_val);
    req + second_read
}

-- 辅助函数 B：要求整组 #Trace（同时使用 request_id 和 depth）
fn trace_step(step_name: str) -> #Trace u32 {
    let req = Trace.request_id()#;
    let d = Trace.depth()#;
    println("[req={}][depth={}] step: {}", req, d, step_name);
    d
}

test {
    -- 1. log_metric 只需要提供 Trace.request_id 即可运行，无需提供 depth。
    --    且尽管内部读取了两次 Trace.request_id()#，"allocate req_id" 严格只打印一次。
    let sum_id = log_metric(100u32)# {
        Trace.request_id() => {
            println("allocate req_id");
            9527u32
        },
    }
    println("sum_id: {}", sum_id);
}
```

---

## 三、 局部覆盖（Scoped Override）与无参 ABI 核心规则

当调用链进入一个子任务（比如从网关层进入存储层子 Span）时，我们希望保留外层的 `request_id = 9527`，但把追踪深度 `depth` 从 `1` 临时覆盖为 `2`；等子任务执行完返回网关层后，`depth` 又能自动恢复为 `1`。

### 在嵌套 Span 中局部覆盖追踪深度，免于污染外层实例

```flurry
-- 内层安装局部覆盖 depth，离开内层后自动恢复外层值。
fn run_with_nested_span() -> #Trace u32 {
    let d_before = trace_step("gateway_enter");

    -- 进入内层子任务：仅覆盖 Trace.depth() 为 d_before + 1。
    -- 外层的 Trace.request_id() 自动透传，且外层的 depth 实例丝分毫无受影响。
    let d_inside = (trace_step("storage_flush"))# {
        Trace.depth() => d_before + 1u32,
    }

    let d_after = trace_step("gateway_exit");
    d_before + d_inside + d_after
}

test {
    let total_depth = run_with_nested_span()# {
        Trace.request_id() => 9527u32,
        Trace.depth() => 1u32,
    }
    -- d_before (1) + d_inside (2) + d_after (1) == 4
    println("total depth sum: {}", total_depth);
}
```

最后，请牢记 `^attrs.effects.direct_passing` 的两条编译期安全红线（见 `docs/algebraic-effect-and-delimited-continuation/direct-passing.md`）：

1. **红线 1：被标记为 `direct_passing` 的效应操作无法带任何参数（包括普通参数和 `catch` Continuation 参数）。**
   为什么？因为 `direct_passing` 的核心契约是在**安装 Handler 的那一刻**就把唯一的结果值算出来存好。如果操作带有参数 `lookup(key: u32)`，每次请求传入的 `key` 都有所差异，断无可能在安装时预先算出一个固定值。
2. **红线 2：`direct_passing` 并非自动授予隐式 `Clone`。**
   如果一个 `direct_passing` 效应返回的是唯一无二的仿射资源（Once Resource），主体一旦在第一次请求时移走（Move）并消费了它，第二次请求再试图消费同一个槽位的值就会被资源检查器直接报错。

```flurry
-- 负例：企图给带有参数的效应操作加上 direct_passing 属性
-- ❌ 编译期直接报错。
-- direct_passing 操作在安装 Handler 时一次性求值就绪，
-- 无法接收任何请求期实参 key: u32 或 catch continuation。
^attrs.effects.direct_passing
effect lookup_config(key: u32) -> u32;
```

---

## 本篇小结

| 特性维度 | 普通代数效应（`effect id()`） | 直接传值效应（`^attrs.effects.direct_passing`） |
| :--- | :--- | :--- |
| **提供分支求值时机** | 每次发出 `id()#` 请求时执行一次分支 | **安装 Handler 时严格执行一次**（即使主体请求 0 次或多次） |
| **参数限制** | 允许任意普通参数与 `catch` 参数 | **必须无参数**（禁止普通参数与 `catch` 参数） |
| **组需求粒度** | 按请求操作路由 | 支持 `#Group.item` 单项按需传值，无需构造整组配置对象 |
| **局部覆盖与恢复** | 内层拦截，向外转发 | 每个安装实例拥有独立结果槽，内层覆盖不篡改外层槽位或已读局部变量 |

---

### 下一篇预告

通过 `^attrs.effects.direct_passing`，我们已经把“无参数环境配置”的开销压到了高度的寄存器/隐式参数传值级别。

那么，对于**带参数的普通效应操作**（比如 `ask(value: bool) -> bool` 或者高频循环里的流过滤器、硬件状态轮询），如果我们既需要每次请求传入不同的实参，又希望编译器在编译期**把被调用函数、效应请求点以及 Handler 分支内联融合到一起**，连一次间接函数指针调用（Evidence Call）都不留，能做到吗？

下一篇 **《Flurry 演进手记 (163)：`inline effect`——在编译期完全内联消去的零开销效应协议》**，我们将揭开 `inline effect` 联合展开器的超强性能秘密。
