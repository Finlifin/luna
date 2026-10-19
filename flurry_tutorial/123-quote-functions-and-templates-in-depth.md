# Flurry 演进手记 (123)：深入 `quote fn` 与 `template quote fn`——可复用的带上下文代码生成组件

> **阅读指南**：在上一回（第 122 回）中，我们厘清了 CMTT 代码见证（Code Evidence）的防伪边界。今天，我们要把第一卷第 034、035 回初识的 **`quote fn`** 与 **`template fn` / `template quote fn`** 放到完整的 CMTT 显微镜下深度剖析！结合 `docs/cmtt/quote-functions-and-templates.md` 与 `docs/functions/inline-and-quote.md`，我们将彻底搞懂：
> 1. `inline fn`、`comptime fn` 与 `quote fn` 这三兄弟在编译器里到底有什么本质区别？（牢记 `AGENTS.md` §5 红线：**`quote fn` 不属于 `comptime fn`，普通实参仍求值一次！**）
> 2. 为什么在 `quote fn` 里写 `return` 不会把外层调用者函数提前退栈？它的局部 `Drop` 清理又遵循怎样的作用域边界？
> 3. `implicit quote` 参数是如何与 `template fn` 的 `quote lambda` 完美配合，实现“上下文零样板自动接线”的？

---

## 一、 痛点：C 语言函数式宏的“三大噩梦”与普通 `inline` 的“不確定性”

假设我们在写状态机或数值处理的热路径（Hot Path）代码，需要一个高频复用的代码组件 `clamp_and_double(x)`。为什么在传统语言里，我们总是在“C 宏”和“普通内联函数”之间左右为难？

### 尝试 `v0`：C 文本宏的重复求值与 `return` 劫持

如果用传统文本替换宏（`v0`）来写一个必定内联展开的代码片段：

```text
// 传统 C 宏的三大噩梦 (v0)：
#define TWICE(v) ((v) + (v))
#define CLAMP(v) { if ((v) > 8) return 8; }

// 噩梦 1（实参重复求值）：
//   调用 TWICE(produce_number()) 时，produce_number() 会被执行【两次】！
//   如果里面有 I/O 或状态自增，直接出大事！
// 噩梦 2（未使用实参副作用丢失）：
//   如果宏体没用到某个参数，实参表达式干脆被删掉了，本该发生的副作用凭空消失！
// 噩梦 3（return 劫持外层函数）：
//   在宏里写一句 return 8，展开后竟然直接把调用者 caller() 整个函数给退栈返回了！
```

而如果改用 C++/Rust 的普通 `inline fn` 呢？第一，普通 `inline` 只是给后端优化器的**建议（Hint）**，不保证 100% 在前端完成代码接入；第二，普通函数无法直接声明“我需要从外层调用环境隐式接入一个 CMTT 上下文变量（`implicit quote`）”！

---

## 二、 破局（v1）：`quote fn`——既有函数的严谨契约，又有宏的必定接入！

在 Flurry 中，**`quote fn`（参数化代码函数）** 把“普通函数的参数求值与返回边界”和“CMTT 代码的 100% 编译期接入”完美融为了一体！

首先，让我们用一张对比表（见 `docs/functions/inline-and-quote.md`），牢牢记住 **`inline fn`、`comptime fn` 与 `quote fn` 的本质分工**：

| 函数类别 | 编译期必须完成的工作 | 运行时主体与阶段身份 |
| :--- | :--- | :--- |
| **普通 `fn` / `inline fn`** | 按函数接口类型检查；`inline` 仅提供内联优化意向 | 属于 `runtime` 函数；可保留普通机器调用指令 |
| **`comptime fn`** | 在**编译期**直接跑完整个函数体并算出结果值 | 属于 `comptime` 函数；绝不能留下运行时补算调用 |
| **`quote fn`** | **必须在编译期完成参数化代码接入（Splice）**；无法接入直接报错，绝不退回普通调用 | **不属于 `comptime fn`！** 它描述的主体在 **`runtime` 执行**，且不能转为运行时函数指针 |

让我们用一张 Code Cartoon，拆解当你调用 `quote fn` 时，编译器生成的 **“实参一次求值 + 逻辑完成边界（Logical Completion Boundary）”** 结构：

```text
  源码调用：
    quote fn clamp(value: u32) -> u32 {
        if value > 8u32 { return 8u32; }
        value
    }
    fn caller(x: u32) -> u32 {
        let bounded = clamp(produce(x));
        bounded + 1u32
    }
                                      │
                                      ▼ 编译期必定展开接入 (NIR Quote Expansion)
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ fn caller(x: u32) -> u32 {                                              │
  │     let bounded = ┌─ [clamp 的逻辑完成边界 (Logical Boundary)] ───────┐ │
  │                   │ 1. 实参准备帧：按源码顺序求值一次！               │ │
  │                   │    let fresh_value: u32 = produce(x);             │ │
  │                   │    (哪怕主体用两次或零次，produce(x) 严格跑一次!) │ │
  │                   │ 2. 接入主体：                                     │ │
  │                   │    if fresh_value > 8u32 {                        │ │
  │                   │        -- return 只跳出本逻辑边界，交付 8u32！    │ │
  │                   │        -- ❌ 绝不会把外层的 caller() 提前退栈！   │ │
  │                   │        deliver_to_boundary 8u32;                  │ │
  │                   │    } else {                                       │ │
  │                   │        deliver_to_boundary fresh_value;           │ │
  │                   │    }                                              │ │
  │                   │ 3. 退出边界前：立即清理 clamp 内部的局部 Drop 对象│ │
  │                   └─────────────────────────┬─────────────────────────┘ │
  │     bounded + 1u32  ◀───────────────────────┘ (拿到 8u32，继续算 + 1!)  │
  │ }                                                                       │
  └─────────────────────────────────────────────────────────────────────────┘
```

### 演进到 `v1`：验证一次求值、局部 `return` 边界与即时 `Drop` 清理

让我们通过一段真实的 Flurry 代码，亲眼验证 `quote fn` 的这三大硬核保证：

```flurry
use std.prelude.*;

-- 保证 1：形参 value 出现两次，但传入的实参表达式严格只求值一次！
quote fn twice(value: i32) -> i32 {
    value + value
}

-- 保证 2：即使形参 unused 在主体里完全没用到，传入的实参依然会执行一次！
quote fn ignore_arg(unused: i32) -> i32 {
    5i32
}

-- 保证 3：quote fn 内部的 return 只完成本次代码接入，绝不跳出外层 caller！
quote fn clamp(value: u32) -> u32 {
    if value > 8u32 {
        return 8u32;
    }
    value
}

fn produce_number() -> i32 {
    println("produce!");
    21i32
}

fn caller(value: u32) -> u32 {
    let bounded = clamp(value);
    -- 当 value = 20 时，clamp 内部执行了 return 8u32，
    -- 但控制流准确落回 bounded，继续执行 bounded + 1u32 得到 9！
    bounded + 1u32
}

test {
    -- 只打印一次 "produce!"，输出 42
    println("twice: {}", twice(produce_number()));
    -- 依然打印一次 "produce!"，输出 5
    println("ignore: {}", ignore_arg(produce_number()));
    -- 输出 9 和 4
    println("clamped: {} {}", caller(20u32), caller(3u32));
}
```

而且，如果 `quote fn` 内部创建了带有 `Drop` 析构函数的局部对象（比如 `let trace = Trace { id: value };`），这个局部对象的生命周期**严格限制在该次 `quote fn` 的逻辑完成边界内**——一旦该次 `quote fn` 求值完毕，它的 `drop` 就会立即执行，绝不会因为代码被展开到了调用者函数里，就把析构推迟到整个外层函数结束时才跑！

---

## 三、 `implicit quote` 与 `template fn` 的珠联璧合（v2）

除了普通参数之外，`quote fn` 还可以声明一种专门属于 CMTT 的特殊参数——**`implicit quote`（运行时上下文输入参数）**！

它和普通参数有什么区别？
- **普通参数 `value: u32`**：按值传递，每次调用都会建立一个新的形参绑定槽；
- **上下文参数 `implicit quote current: u32`（或 `*Context`）**：它代表这段 `quote fn` 代码主体在 CMTT 层面期望外部上下文 $\Psi$ 中存在一个名叫 `current` 的绑定！它按绑定对应直接连到原运行时对象上，保持原有的对象身份（Identity）、指针目标与别名关系！

当我们将带有 `implicit quote` 的 `quote fn` 与授予上下文的 **`template fn`** 结合在一起时，奇迹发生了：

### 演进到 `v2`：模板授予上下文 + `quote fn` 自动感应接入

```flurry
use std.prelude.*;

-- 1. 定义一个需要上下文变量 `current: u32` 的 quote fn 组件
quote fn add_current(value: u32, implicit quote current: u32) -> u32 {
    value + current
}

-- 2. 定义一个高级模板函数：
--    通过 quote lambda body: < current: u32 |- u32 > 向实例正文授予 `current` 上下文！
template fn with_current(
    value: u32,
    quote lambda body: < current: u32 |- u32 >,
) -> u32 {
    let current = value;
    ~<body>
}

-- 3. 使用模板实例化出 adjusted 函数：
--    在大括号正文里，由于 with_current 已经授予了 current: u32 上下文，
--    调用 add_current(2u32) 时会自动感应并接入该 current 绑定，连 .current = ... 都不用写！
fn adjusted use with_current {
    add_current(2u32)
}

test {
    -- current 被绑定为 40u32，add_current(2u32) 自动算出 2 + 40 == 42！
    println("adjusted: {}", adjusted(40u32));
}
```

> **🔍 语法小贴士：简单 `template fn` 签名复用 vs. 高级 `quote lambda` 正文复用**
> 在 `docs/cmtt/quote-functions-and-templates.md` 中，模板分为两个层次：
> 1. **简单签名模板**：`template fn unary(value: i32) -> i32;`（没有函数体，以分号结尾）。后续既可以用 `fn ordinary use unary { value * 2i32 }` 生成普通函数，也可以用 `quote fn generated use unary { value * 2i32 }` 生成必须展开的 `quote fn`，各自保留自己的函数类别！
> 2. **带 `quote lambda` 的上下文模板**：如上面的 `with_current`，它不仅复用签名，还提供外围运行时骨架代码（如初始化 `let current = value;`），并通过 `~<body>` 将使用者的代码块在受控上下文 `< current: u32 |- u32 >` 下安全接入！

最后，请牢记 `quote fn` 的两条编译期红线：
1. **在一段 `quote` 或另一个 `quote fn` 内部调用 `quote fn`，计作 1 个隐式接入点（Implicit Splice Hole）**，同样受单洞规则约束（而在普通 `fn` 里调用多次 `quote fn` 则完全不受单洞限制！）；
2. **`quote fn` 不是运行时函数实体，绝不能当作普通函数指针逃逸到运行期**，也不能在缺乏匹配的 `implicit quote` 上下文且未显式提供时凭空调用：

```flurry
quote fn need_ctx(implicit quote current: u32) -> u32 {
    current
}

fn broken_quote_fn_usage() -> u32 {
    -- 负例 1：企图把 quote fn 当作运行时函数指针赋值给变量
    -- ❌ 编译期直接拒绝！
    -- quote fn 是编译期代码模板，在运行期二进制中没有独立函数入口，不能转为函数指针！
    let _fn_ptr: fn() -> u32 = need_ctx;

    -- 负例 2：在没有任何 current 上下文授权的普通函数里裸调 need_ctx()
    -- ❌ 编译期直接拒绝！缺少 implicit quote current: u32 的上下文供应！
    need_ctx()
}
```

---

## 本篇小结

| 特性 / 机制 | `quote fn` 与 `template` 的契约 | 相比传统宏 / 普通内联的优势 |
| :--- | :--- | :--- |
| **函数阶段定位** | `quote fn` 不属于 `comptime fn`；代码描述在编译期接入，主体在运行期执行 | 100% 保证编译期展开，同时绝不在编译期误读未来的运行期实参值 |
| **普通实参一次求值** | 按源码顺序求值一次并绑定新鲜形参；未使用实参仍保留求值副作用 | 彻底根治 C 宏参数重复求值与副作用丢失顽疾 |
| **逻辑完成边界** | 内部 `return` 仅结束本次 `quote fn` 应用；局部 `Drop` 在边界退出时立即清理 | 绝不劫持外层调用者函数的控制流，也不延长临时对象的生命周期 |
| **`implicit quote` + `template`** | 通过上下文对应直接绑定原运行时对象，配合 `quote lambda` 自动接线 | 无需传递冗长的上下文指针参数，保持底层对象 Identity 与别名安全 |

---

### 留给下一篇的悬念

我们已经备齐了 CMTT 的全部基础构件：`< Ψ |- T >` 表达式代码、`< T -| Ψ >` 模式代码、`Quoted Arm`、上下文替换以及 `quote fn`。

现在，让我们回到第六卷的主线任务：**在编译期组装一个完整的有限状态机（FSM）程序！**

你可能会好奇：刚才我们在第 118 回和本回反复强调了 **“单洞规则（Single-Hole Rule）”**——每一层 `'(...)` 最多只能有一个接入点 `~<previous>`。
既然每层 `'(...)` 只能挖一个洞，那如果我们要在编译期用一个 `for` 或 `while` 循环，把 10 条语句顺序串成一个代码块，或者把状态机的 20 条状态转移边 `(source, byte, target)` 组装成一个层层判断的分派函数，**怎么只靠“一个洞”拼出任意多层、甚至带运行期循环回边（Back-Edge）的自动机呢？**

下一篇 **《Flurry 演进手记 (124)：用单洞上下文组合出完整循环与状态机——有限程序组装（Program Construction）》**，我们将结合 `docs/cmtt/program-construction.md`，揭秘“单洞链式累积（Single-Hole Chaining）”的组装艺术！
