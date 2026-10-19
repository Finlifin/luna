# Flurry 演进手记 (166)：当 `resume` 附带 `while` 守卫——条件恢复与状态机驱动

> **阅读指南**：在上一回（第 165 回）结尾，我们发现了 `docs/grammar/syntax` 与 `docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md` 中的一颗语法明珠：`resume_statement -> resume expr? (while expr)?`！在第一卷第 014 回里，我们见识过 `return x while cond;` 和 `break while cond;` 如何消除深层 `if` 缩进。今天，当后缀 `while` 守卫遇上代数效应的核心语句 **`resume`** 时，会发生怎样奇妙的化学反应？在直接处理（Direct Handler）和捕获处理（Capturing Handler）中，当 `while` 守卫不成立、未触发 `resume` 时，控制流与挂起栈的资源清理分别遵循什么规则？让我们一探究竟！

---

## 一、 痛点：Handler 分支里层层嵌套的配额与熔断 `if` 检查

在编写工业控制状态机、网络请求限流器（Rate Limiter）或沙箱指令执行器时，效应 Handler 往往扮演着**“守门人（Gatekeeper）”**的角色：
每当受控主体发出一次效应请求（比如申请读取传感器、申请消耗 1 个 Token 配额、或者申请执行一步状态转移），Handler 都要先检查一系列前置安全条件（如配额是否充足、看门狗是否未超时、输入参数是否在安全阈值内），只有条件成立时才立即 `resume` 交付结果；否则走降级兜底或熔断中止逻辑！

### 审视 `v0`：传统 `if { resume ...; }` 带来的视觉噪音

如果不用后缀 `while` 守卫（`v0`），一个拥有三档动态策略的 Handler 分支写出来是这样的：

```flurry
-- 尝试 v0：在 Handler 分支里堆叠多层 if { resume ...; }
scale(value) => {
    if value == 0u32 {
        resume 1u32;
    }
    if value > 100u32 {
        resume 100u32;
    }
    value * 2u32
}
```

虽然能跑，但每次条件判断都要多套一层大括号，把真正的重头戏——“交付什么结果（`resume 1u32`）”藏在了条件块的内部！

---

## 二、 破局（v1）：`resume expr? while cond;`——把交付的值亮在最前面！

在 Flurry 的全局语法中，四大控制跳转语句（`return`、`break`、`continue`、`resume`）享有一致的后缀守卫哲学：

```text
resume_statement -> resume expr? (while expr)?
```

它的执行语义极其直观：
1. 当控制流到达 `resume val while cond;` 时，先判断守卫条件 `cond`（类型为 `bool`）；
2. **若 `cond` 为 `true`**：计算 `val`，立即将其作为本次效应操作的结果交付给请求点，并**当场结束当前这一个 Handler 分支的执行**！
3. **若 `cond` 为 `false`**：不执行 `resume`（也不提前结束分支），控制流平缓地继续执行该 Handler 分支里的下一条语句！
4. 对于返回 `void` 的效应操作，还可以省略结果表达式，直接写 **`resume while cond;`**！

让我们用一张 Code Cartoon，看清 `resume val while cond;` 在直接处理与捕获处理两条路径上的控制流走向：

```text
                Handler 分支执行到：resume expr while cond;
                                     │
                         [ 先求值守卫条件 cond : bool ]
                                     │
                  ┌──────────────────┴──────────────────┐
                  ▼ cond == true                        ▼ cond == false
   ┌─────────────────────────────────┐   ┌─────────────────────────────────┐
   │ ✅ 立即触发 resume！            │   │ ⏭️ 跳过本条 resume，继续往下走！│
   │ 1. 求值 expr 交付给效应请求点   │   │ 1. 执行分支后续的兜底/熔断语句  │
   │ 2. 立即结束当前 Handler 分支    │   │ 2. 若在捕获分支中直到分支结束   │
   │ 3. 被处理主体从请求点后继续运行 │   │    仍未 resume：自动触发挂起栈  │
   └─────────────────────────────────┘   │    的【放弃与 Drop 清理协议】！ │
                                         └─────────────────────────────────┘
```

### 演进到 `v1`：用 `resume ... while` 重构多档位限幅与特殊值短路

让我们把 `docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md` 中的 `scale` 例子演进到 `v1`，看看用 `resume ... while` 写出来的 Handler 分支有多么清爽：

```flurry
use std.prelude.*;

effect scale(value: u32) -> u32;

fn use_scale() -> #[scale] u32 {
    let first = scale(0u32)#;
    let second = scale(4u32)#;
    let third = scale(80u32)#;
    first + second + third
}

test {
    let result = use_scale()# {
        scale(value) => {
            -- 档位 1：当输入为 0 时，直接恢复交付保底值 1u32！
            resume 1u32 while value == 0u32;

            -- 档位 2：当输入超过安全上限 50 时，直接恢复交付封顶值 100u32！
            resume 100u32 while value > 50u32;

            -- 常规路径：既非 0 也未超限，正常翻倍返回给请求点
            value * 2u32
        },
    };
    -- 1 + (4 * 2) + 100 == 109
    println("scaled sum: {}", result);
}
```

看看这三行代码：
- `resume 1u32 while value == 0u32;`
- `resume 100u32 while value > 50u32;`
- `value * 2u32`
没有任何大括号嵌套，左边整整齐齐地排列着每一种边界情况交付的返回值，右边清清楚楚地写着触发条件！而且请注意：**`resume` 只结束当前这一次请求（比如 `scale(0u32)#`）的处理分支**，主体拿到 `1u32` 后会继续发出第二次请求 `scale(4u32)#` 和第三次请求 `scale(80u32)#`！

---

## 三、 配额看门狗熔断：当捕获分支中 `while` 守卫不成立时，挂起栈如何安全清理？（v2）

现在，让我们把 `resume ... while` 放到更具威力的 **捕获型 Handler（Capturing Handler，带 `catch continuation: builtin.Continuation`）** 中！

设想我们在写一个带配额限制的状态机驱动器（Quota-Guarded Driver）：
- 受控任务在循环中每次执行一步操作，都会发出 `tick_step(step_id, catch continuation)` 请求；
- Handler 维护着一个剩余配额计数器 `var quota: u32 = 2u32;`；
- 每次收到请求时，Handler 写一句：
  ```flurry
  resume while quota > 0u32;
  ```
- 当 `quota > 0u32` 时，扣减配额并继续恢复受控任务；
- **一旦 `quota == 0u32`，`while` 守卫为 `false`！** 此时跳过 `resume`，直接走到分支末尾结束 Handler！

这时候，那个正挂在半空中的受控任务栈（以及它里面已经打开的局部 `Drop` 资源 `StepResource`）会怎么样？
正如我们在第 161 回和 `multi-prompt-delimited-continuation.md` 中学到的：**当捕获分支退出时若尚未消费 `continuation`，编译器会自动启动“放弃清理协议（Abandonment Protocol）”**——沿着隐藏中止通道进入挂起栈，把里面存活的 `StepResource` 析构得干干净净，然后再把栈槽归还给 `pool`！

### 演进到 `v2`：用 `resume while quota > 0` 实现零泄漏的配额熔断器

```flurry
use std.prelude.*;

effect tick_step(step: u32, catch continuation: builtin.Continuation);

struct ActiveSession {
    name: str,
}

impl Drop for ActiveSession {
    fn drop(*self) {
        println("session [{}] safely closed!", self.name.*);
    }
}

fn worker_loop() -> #[tick_step] void {
    let _session = ActiveSession { name: "sensor_pipeline" };
    for step in 1u32..10u32 {
        println("requesting step {}", step);
        tick_step(step)#;
    }
    println("all 9 steps finished (unreachable under quota=2)");
}

test {
    var quota: u32 = 2u32;

    (worker_loop()# {
        tick_step(step, continuation) => {
            if quota > 0u32 {
                quota -= 1u32;
                println("approved step {}, remaining quota: {}", step, quota);
                -- ✅ 此时已扣减配额，恢复挂起计算继续下一步！
                resume while true;
            }

            -- 当配额耗尽（step == 3）时，不执行 resume：
            println("quota exhausted at step {}! aborting worker...", step);
            -- 离开本分支时，未消费的 continuation 自动触发放弃清理协议，
            -- ActiveSession.drop 被自动调用，打印 "session [sensor_pipeline] safely closed!"
        },
    }).use(pool);
}
```

当然，编译器对 `resume expr? while cond;` 的类型与上下文检查也毫不手软：
1. **`resume` 语句（无论带不带 `while`）只能出现在效应 Handler 的处理分支内部**，在普通函数里写 `resume` 会被直接拒绝；
2. **`resume` 交付的值必须与效应操作声明的返回类型 `Y` 完全一致**，且 `while` 后的守卫表达式必须是 `bool` 类型：

```flurry
effect ask_count() -> u32;

fn broken_resume_guard() -> u32 {
    (do {
        ask_count()#
    })# {
        ask_count() => {
            -- ❌ 编译期直接拒绝！
            -- 1. while 守卫条件必须是 bool 类型，不能传入整数 1u32！
            -- 2. 若操作返回 u32，resume 必须提供 u32 结果表达式，不能无参 resume！
            resume 42u32 while 1u32;
            0u32
        },
    }
}
```

---

## 本篇小结

| 语法形式 | 适用场景 | 执行与清理语义 |
| :--- | :--- | :--- |
| **`resume val while cond;`** | 有返回值的效应 Handler 分支 | 若 `cond` 为真则立即向请求点交付 `val` 并结束本分支；否则继续执行分支后续语句 |
| **`resume while cond;`** | 返回 `void` 的效应 Handler 分支 | 若 `cond` 为真则立即恢复请求点（交付 `()`）；否则跳过并继续执行后续熔断逻辑 |
| **捕获分支跳过 `resume` 时** | 配额耗尽、超时熔断、提前取首元素 | 未被消费的 `builtin.Continuation` 在离开作用域时自动触发隐藏中止清理，安全析构挂起栈上的 `Drop` 资源 |

---

### 留给下一篇的悬念

到现在为止，我们已经学会了五花八门的效应技巧：回溯（161）、Direct Passing 追踪（162）、`inline effect`（163）、异步背压（164）、Arena 可达性绑定（165）以及 `resume ... while` 条件熔断（166）。

但是，在设计一个供成百上千名外部开发者调用的大型标准库或商业 SDK 时，我们面临一个至关重要的**软件架构与模块边界问题**：
- 我们在库模块 `parser_engine` 内部为了代码清晰，定义并使用了一个私有的代数效应 `private effect step_cursor() -> u8;`；
- 可是，调用我们公开函数 `pub fn parse_document(text: str) -> Document` 的外部用户，根本不想知道你内部用了什么 `step_cursor` 效应！
- 如果我们不小心把一个内部私有效应 `#step_cursor` 留在了公开函数 `pub fn` 的返回类型签名上，会发生什么？我们应该怎样在模块与包的边界上，做好**效应的隐藏、内部消去（Internal Handling）与高层效应转换**？

下一篇 **《Flurry 演进手记 (167)：跨模块与跨包边界的效应隐藏与接口封装》**，我们将结合 `docs/code-organization/visibility.md` 与 `handling-capability.md`，讲透效应系统的模块化架构之道！
