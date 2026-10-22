# Flurry 演进手记 (034)：实参只求值一次的代码拼装——`quote fn` 与普通宏的本质区别

在嵌入式和系统编程领域，我们经常有这样一个底层诉求：**我希望一段通用计算在编译期 100% 展开（接进）调用处，不留下任何运行时函数调用帧或跳转指令。**

比如，在我们的高频传感器采样中断服务程序（ISR）里，每一纳秒都宝贵。我们需要写几个短小且高频调用的运算积木：
1. `twice(value)`：把从硬件 FIFO 队列里弹出的采样值乘以 2（写为 `value + value`）。
2. `clamp(value)`：如果采样值超过上限 `8u32` 就提前截断返回 `8u32`，否则返回原值。
3. `shifted(value)`：在保持单次求值的前提下，直接读取调用者当前作用域里的某个运行时校准上下文指针 `context: *Context`。

在 C/C++ 或传统语言里，面对这种需求，程序员通常只有两个选择：
- **选择 A：写 `inline` 函数**。但在 C/C++（以及大多数语言）里，`inline` 只是给优化器的一个“礼貌建议（Hint）”，编译器在调试模式或复杂控制流下随时可能保留真实函数调用。
- **选择 B：写 C 预处理器宏 `#define TWICE(x) ((x) + (x))`**。它确实 100% 展开了，但一旦你传入带有副作用的表达式 `TWICE(produce())`，难题就降临了。

今天，我们将认识 Flurry 中专门解决这一矛盾的重型武器——**`quote fn`**，并牢记 Flurry 语言设计的一条核心规则（见 `docs/functions/inline-and-quote.md`）：

> **`quote fn` 并非属 `comptime fn`，普通实参仍求值一次。**

---

## 一、 C 式文本宏的问题与 `inline fn` 的边界

假设我们有一个从传感器硬件 FIFO 队列中弹出一个 32 位读数的函数 `produce()`。每次调用 `produce()`，硬件队列指针就会往前走一格，并打印一行日志 `"produce"`：

```flurry
use std.prelude.*;

fn produce() -> u32 {
    println("produce");
    21
}
```

如果我们在 C 语言里写 `#define TWICE(x) ((x) + (x))`，然后调用 `TWICE(produce())`，预处理器会无脑地做文本复制粘贴，把它变成 `(produce() + produce())`——硬件 FIFO 被错误地抽走了两个数据，日志打印了两次 `"produce"`。
更糟糕的是，如果你写 `#define CONSTANT(x) (5)` 然后调用 `CONSTANT(produce())`，因为宏体里没用到 `x`，`produce()` 甚至连一次都跳过执行，硬件队列干脆卡死了。

而在 Flurry 中，如果我们写 `inline fn` 呢？

```flurry
inline fn smaller(left: u32, right: u32) -> u32 {
    if left < right {
        left
    } else {
        right
    }
}
```

`inline fn` 当然是类型安全、单次求值的普通函数，但它提供的是**内联意向（Inline Intent）**——优化器可以展开它，但在默认解释路径、递归链超限或预算耗尽时，它依然被允许保留为一个普通的运行时函数调用。

如果我们正在构建必须 100% 消去调用边界的代码拼装层（比如零开销 DSL、寄存器访问包装或规则引擎），我们需要的是**“作为语义契约必须完成代码接入，但同时严格遵守函数参数单次求值与作用域边界”**的机制。

这就是 **`quote fn`**。

---

## 二、 `quote fn` 的核心契约——普通实参只求值一次，且 `return` 严守边界

让我们把 `smaller`、`twice`、`constant` 和 `clamp` 用 `quote fn` 写出来（详见 `docs/functions/inline-and-quote.md` 与 `docs/cmtt/quote-functions-and-templates.md`）：

```flurry
use std.prelude.*;

-- 1. quote fn：代码接入是应用必须完成的工作。
quote fn twice(value: u32) -> u32 {
    value + value
}

-- 2. 哪怕主体根本没用到形参 value，传入的普通实参依然会执行一次。
quote fn constant(value: u32) -> u32 {
    5
}

-- 3. quote fn 内部的 return 只结束本次 quote fn 应用，免令外层 caller 提前返回。
quote fn clamp(value: u32) -> u32 {
    if value > 8u32 { return 8u32; }
    value
}

fn produce() -> u32 {
    println("produce");
    21
}

fn caller(value: u32) -> u32 {
    let bounded = clamp(value);
    bounded + 1u32
}

test {
    -- 测试 1：value 在 twice 体内出现了两次，但 produce() 只打印一次。
    println("{}", twice(produce()));

    -- 测试 2：constant 体内没用 value，但 produce() 依然打印一次。
    println("{}", constant(produce()));

    -- 测试 3：clamp(20u32) 触发内部 return 8u32，然后继续执行 caller 的 `+ 1u32` 得到 9。
    println("{} {}", caller(20u32), caller(3u32));
}
```

运行这段测试，输出完全一致：

```text
produce
42
produce
5
9 4
```

### `quote fn` 的接入过程 结构示意

为什么 `quote fn` 既能 100% 把代码展开到调用点，又不会犯 C 宏的毛病？
下面对比普通函数、`comptime fn` 与 `quote fn` 的本质分工：

```text
  ┌───────────────┬──────────────────────────────┬────────────────────────────────┐
  │   函数类别     │      编译期必须完成的工作      │         运行时留下的东西        │
  ├───────────────┼──────────────────────────────┼────────────────────────────────┤
  │ inline fn     │ 记录内联意向；检查接口与契约    │ 可展开，也可保留普通 runtime 调用│
  ├───────────────┼──────────────────────────────┼────────────────────────────────┤
  │ comptime fn   │ 在编译期直接算出最终结果值      │ 只有常量结果（不保留 runtime 主体）│
  ├───────────────┼──────────────────────────────┼────────────────────────────────┤
  │ quote fn      │ 必须完成参数化代码接入          │ 实参单次求值绑定 + 接入后的      │
  │ (≠comptime fn)│ (无法完成接入则直接报编译错误!) │ runtime 主体操作与局部清理边界   │
  └───────────────┴──────────────────────────────┴────────────────────────────────┘
```

当你调用 `twice(produce())` 时，编译器在调用点生成的代码结构，在语义上完全等价于下面这段带有**新鲜绑定（Fresh Binding）**和**逻辑完成边界**的块：

```text
  调用点写下：  let ans = twice(produce());
                               │
                               ▼ (编译期代码接入展开)
  ┌────────────────────────────┴────────────────────────────┐
  │ 1. [实参准备阶段] 按源码顺序对每个普通实参求值恰好一次：  │
  │    let fresh_value_0 = produce();   ◄── 只执行 1 次      │
  │                                                         │
  │ 2. [主体接入阶段] 使用新鲜绑定与私有退出汇合点执行主体：  │
  │    fresh_value_0 + fresh_value_0                        │
  │                                                         │
  │ 3. [局部清理阶段] 结束本次 quote fn 作用域内的 Drop 责任 │
  └────────────────────────────┬────────────────────────────┘
                               ▼
                交付最终结果 42 给调用者的 ans。
```

这里有四条核心原则：
1. **`quote fn` 切勿求实参在编译期已知**：因为 `quote fn` 并非属 `comptime fn`，`twice(produce())` 里的 `produce()` 完全是一个运行时函数调用，接入后的 `value + value` 也是运行时加法。
2. **新鲜的是绑定名，但保留真实的指针别名**：如果你写 `quote fn store_then_read(target: *u32, source: *u32) -> u32 { target.* = 9; source.* }`，然后传入同一个变量的地址 `store_then_read(val.ref, val.ref)`，编译器虽然为 `target` 和 `source` 建立了两个新鲜形参绑定，但免将它们当成两块互不相关的内存而错误地重排读写顺序，结果稳稳输出 `9`。
3. **局部 `Drop` 留在 `quote fn` 的语义边界内清理**：如果 `quote fn` 内部创建了一个实现了 `Drop` 的局部变量 `let trace = Trace { id: value }`，当这次 `quote fn` 应用完成时，`trace` 会立刻在本次应用的退出边界被 `Drop`，免因代码展开到了外层函数，就把 `trace` 的寿命偷偷延长到整个外层函数结束。
4. **普通函数体可以包含多次独立的 `quote fn` 调用**：虽然在 CMTT（上下文模态类型理论）的一段显式引用代码 `'(...)` 内部遵守“单洞（至多一个接入点）”限制，但一个**普通的 runtime 函数体**并并非一段 quote，你在 `fn combine(a: u32, b: u32)` 里连续调用十次 `twice(a)`、`twice(b)` 完全合法。

---

## 三、 当生成代码需要对接外层上下文——`implicit quote` 参数

现在看第三个需求：
有时候，我们写的 `quote fn` 并非但要接收调用者传进来的普通值参数 `value: u32`，还希望生成的代码直接绑定到调用者上下文里的某个特定变量（比如当前模块维护的校准基数指针 `context: *Context`）。

在 Flurry 中，这通过 **`implicit quote`** 参数角色来声明（详见 `docs/functions/inline-and-quote.md`）：

```flurry
use std.prelude.*;

struct Context {
    base: u32,
}

-- `implicit quote context: *Context` 标明这是一个 runtime contextual 变量。
quote fn shifted(value: u32, implicit quote context: *Context) -> u32 {
    value + context.base.*
}

test {
    let state = Context { base: 10 }
    let context = state.ref;

    -- 第一次调用：此时 state.base 是 10
    let first = shifted(1, .context = context);

    -- 在运行时修改 state.base 为 20。
    state.base = 20;

    -- 第二次调用：接入的代码读取的是同一个运行时对象 state。
    let second = shifted(1, .context = context);

    println("{} {}", first, second);
}
```

运行结果输出：

```text
11 21
```

看明白 `implicit quote` 的精妙之处了吗？
- 它**并非**在编译期去读取 `context.base.*` 的值（编译期根本还没有运行时的 `state` 对象。），而是在编译期把 `shifted` 主体里的 `context` 与调用点的 runtime 变量 `context` 建立合法的**上下文绑定对应（Contextual Correspondence）**。
- 当你在运行时把 `state.base` 改成 `20` 后，第二次执行 `shifted` 读到的自然就是最新的 `20`。
- 当外层环境（比如我们下一篇马上要讲的 `template fn`）已经在上下文接口中提供了对应的变量名时，你甚至连 `.context = context` 都省去书写，直接通过合法的隐式替换（Implicit Substitution）自动对接。

反之，如果一个 `quote fn` 因为递归无限展开等原因无法在编译期完成代码接入，会发生什么？

```flurry
-- ❌ 编译期直接报错。
-- quote fn 必须完成接入；无法完成接入或耗尽编译预算时直接报错，
-- 编译器不会悄悄把它退化成一个普通的 runtime 函数调用来蒙混过关。
quote fn infinite_expand(x: u32) -> u32 {
    infinite_expand(x + 1u32)
}
```

---

## 本篇小结

| 特性 / 维度 | C 预处理器宏 `#define` | Flurry `inline fn` | Flurry `quote fn` |
| :--- | :--- | :--- | :--- |
| **本质身份** | 词法 Token 文本替换 | 带内联意向的普通函数 | **必须完成代码接入的参数化代码**（非属 `comptime fn`） |
| **普通实参求值** | ❌ 出现几次就算几次（0 次或多次） | ✅ 严格按顺序求值 1 次 | ✅ **严格按顺序求值 1 次**（哪怕形参出现多次或未出现） |
| **内部 `return` 行为** | ❌ 直接从外层调用者函数返回 | ✅ 仅结束本次被调函数 | ✅ **仅结束本次 `quote fn` 应用的逻辑边界** |
| **局部资源 `Drop`** | 无独立作用域保证 | 退出被调函数时清理 | **在本次 `quote fn` 应用完成边界即刻清理**，不拖延到外层退出 |
| **上下文对接** | 靠非卫生的同名变量乱撞 | 显式传参或 `implicit` | 支持 **`implicit quote`** 受检上下文变量对应 |

---

### 下一篇预告

用 `quote fn` 把小段计算拼装进调用点非常爽快。
但是，当我们开发传感器网关的业务层时，遇到了相反方向的重复：
- 我们有 20 个不同的传感器通道处理函数（`halve_request`、`clamp_request`……），它们拥有**一模一样的函数签名、前置校验和请求追踪对象 `RequestTrace`**，唯独中间那两行核心算术逻辑不同。
- 我们还有 50 个单元测试，每个测试都需要先初始化一套相同的计数器指针或追踪环境，跑完测试体后再自动执行资源清理。

难道我们要把函数签名和前置初始化代码复制粘贴 50 遍吗？
在下一篇 **《Flurry 演进手记 (035)：一套签名、百种实现与批量验收——`template fn` 与 `template test`》** 中，我们将解锁 Flurry 的函数模板与测试模板，看看 `fn id use template { ... }` 如何用 `< number: u32 |- u32 >` 上下文模态类型把“公共骨架”与“业务正文”天衣无缝地扣合在一起。
