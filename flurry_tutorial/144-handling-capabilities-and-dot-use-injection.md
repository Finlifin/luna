# Flurry 演进手记 (144)：为什么函数签名里不写 handles——基于 Trait 的处理能力（Handling Capability）与 .use(handler)

> **系列导读**：在上一期（143）中，我们学会了用 `effect enum` 组织系统能力，并弄清了嵌套 Handler 的转发法则。但在工程实践中，我们不可能每次都在调用处手写一大坨匿名的 `expr# { ... }` 分支——比如一个有状态的递增序号生成器 `Counter`、一个超限截断器 `ClampTo`、或者一个协程栈池 `StackPool`，它们本身就是带有字段和方法的 `struct` 对象。今天，我们将探索 Flurry 是如何把**有状态结构体与 Trait 协议**接入效应与错误处理环境的，并深刻理解 Flurry 语言设计的一条核心规则：**“Capability 由 trait 协议和 elaboration 实现；函数签名不添加 `handles`。”**

---

## 一、匿名 Handler 的局限：传统方案的“手写胶水重复症”

假设在我们的微型运行时中，有一个有状态的递增序号分配器 `Counter { value: u32 }`，它实现了 `ProvideNumber` 接口：每调用一次 `provide(*self)`，就返回当前序号并将内部计数器加 1。

在初始写法（纯手动桥接）阶段，我们可以把一个可变的 `var counter` 放进 `expr# { ... }` 的分支里：

```flurry
trait ProvideNumber {
    fn provide(*self) -> u32;
}

struct Counter {
    value: u32,
}

impl ProvideNumber for Counter {
    fn provide(*self) -> u32 {
        let result = self.value.*;
        self.value.* += 1u32;
        result
    }
}

effect next_number() -> u32;

fn total() -> #[next_number] u32 {
    let first = next_number()#;
    let second = next_number()#;
    first + second
}

test {
    var counter = Counter { value: 10u32 }
    -- 在 # { ... } 分支里手动把 next_number() 转发给 counter.provide()
    let result = total()# {
        next_number() => counter.provide(),
    }
    -- 第 1 次得 10，第 2 次得 11，总和为 21，counter.value 变为 12。
    println("{} {}", result, counter.value);
}
```

对于单个简单效应，上面这种写法已经很清晰了（而且它直观展示了：连续两次效应请求访问的是**同一个 `Counter` 实例**，状态在多次请求间自然延续。）。

但是，如果一个处理器对象要同时处理一整组复杂的错误或效应，或者我们要编写一个**接收任意处理器对象 `h` 的通用高阶函数**，每次都手写 `# { ... }` 或 `! { ... }` 分发胶水就太繁琐了。我们需要一种**将对象自身声明的 Trait 处理能力直接挂载到计算上**的标准机制。

---

## 二、核心设计核心规则：为什么函数签名里绝对不加 `handles`？

在设计“把处理器对象传给函数、或者标注对象具备某种处理能力”的语法时，有些实验性语言试图在**函数签名**里发明一套全新的能力段（比如写出类似 `fn run(h: H) handles MyEffect` 的特殊函数签名）。

Flurry 的语言架构师对此立下了极其明确的红线：
> **Capability 由 trait 协议和 elaboration 实现；错误结果与代数效应具有不同 ABI，函数签名不添加 `handles`。**

为什么 Flurry 坚决不在函数签名里引入 `handles` 子句？
1. **避免撕裂多态体系**：我们在第五卷（096–105）已经建立了一套极其完备、拥有统一深度结构精度与全局 Coherence 保证的 **Trait 与 `where` 约束体系**。一个对象“能够处理某种错误或某种效应”，本质上就是**该对象的类型实现了对应的处理器 Trait 协议（如 `HandleError<E, R>`）**。如果在函数签名里另搞一套 `handles` 语法，就会制造出两套平行的接口约束机制。
2. **函数签名保持纯粹**：函数的输入能力约束统一用 `where H :- HandlerTrait` 表达；函数自身执行过程中未消去的输出效应与错误，统一用返回类型上的 `#Effects` 和 `!Errors` 前缀表达。

那么，`handles` 关键字在 Flurry 中到底用在哪里？它又是如何与 `.use(handler)` 配合的？

---

## 三、局部绑定模式 `pattern handles expr` 与 `.use(handler)` 注入

在 Flurry 中，接入对象化处理能力只需两步优雅的配合：

```text
handler_bind        -> pattern handles expr
handler_application -> expr.use(expr)
```

1. **在模式绑定处声明处理能力（`pattern handles expr`）**：
   当你用 `let handler handles LimitErr = ClampTo { limit: 8u32 }` 绑定一个处理器对象时，`handles LimitErr` 告诉编译器的 Elaboration 引擎：“请通过 Trait 协议检索 `ClampTo` 对 `LimitErr` 的处理入口（若需同时声明多个处理能力，在 `handles` 后用 `and` 连接，如 `let h handles ErrA and ErrB = ...`）”。
2. **在目标计算处显式挂载（`expr.use(handler)`）**：
   在需要消去该错误或效应的表达式后面，紧跟后缀 **`.use(handler)`**。Elaboration 引擎会自动把刚才解析好的 Trait 入口与接收者环境连接到该计算的消去边界上。

让我们看一段完整可运行的 改进代码：

```flurry
-- 1. 定义错误处理的 Trait 协议
trait HandleError
where E, R
{
    fn handle_error(*self, value: E) -> R;
}

enum LimitErr {
    TooLarge(u32),
}

-- 2. 定义携带状态字段的处理器结构体 ClampTo
struct ClampTo {
    limit: u32,
}

-- 3. 为 ClampTo 实现 HandleError 协议
impl HandleError<LimitErr, u32> for ClampTo {
    fn handle_error(*self, value: LimitErr) -> u32 {
        value match {
            .TooLarge(_) => self.limit.*,
        }
    }
}

fn checked_count(value: u32) -> !LimitErr u32 {
    if value <= 8u32 {
        value
    } else {
        error LimitErr.TooLarge(value)
    }
}

test {
    -- 第 1 步：用 pattern handles expr 绑定处理器能力
    let handler handles LimitErr = ClampTo { limit: 8u32 }

    -- 第 2 步：用后缀 .use(handler) 将处理能力注入计算，消去 !LimitErr。
    let result = checked_count(12u32).use(handler);
    println("{}", result); -- 输出：8
}
```

### `handles` 绑定与 `.use(handler)` 的四大安全边界

```text
  [ let handler handles LimitErr = ClampTo { limit: 8u32 }; ]
                        │
         (1) 普通 Trait Resolution 查找 HandleError<LimitErr, u32> for ClampTo
         (2) 建立处理入口与接收者环境（对象仅创建和求值一次。）
                        │
                        ▼
  [ checked_count(12u32).use(handler) ]
         ├── 若成功返回 5u32 ──► 直接交付 5u32（绝不调用 handle_error。）
         └── 若返回 TooLarge ──► 调用 handler.handle_error(...) 交付 8u32。
```

在使用 `handles` 与 `.use(handler)` 时，请务必牢记以下四条安全边界：
1. **绑定不等于无中生有**：`let h handles LimitErr = obj;` 只是声明启用 `obj` 已有的处理能力；如果 `obj` 的类型根本没有实现对应的处理 Trait，编译期直接报错，绝不会凭空替对象变出能力。
2. **绑定不等于隐式全局拦截（No Ambient Magic）**：仅仅写下 `let handler handles LimitErr = ...;`，**绝不会**隐式拦截整个作用域里所有的 `LimitErr`。哪个表达式需要使用这个处理器，就必须显式写上 **`expr.use(handler)`**。这使得每一处控制流拦截在源码中都清晰可见、有迹可循。
3. **精准消去，不乱抹其他标签**：即使 `ClampTo` 同时实现了处理 `LimitErr` 和 `ParseErr` 的方法，当它挂载到一个只请求消去 `LimitErr` 的边界，或者只覆盖了部分效应时，未覆盖的路径依然必须有合法的外层路由，绝不会因为对象上碰巧有别的方法就胡乱抹去不相关的行。
4. **共同环境绝不重复复制或双重销毁**：如果一个有状态处理器对象 `handler` 为一个 `effect enum` 提供了多个操作的入口，这几个入口共享**同一个底层对象身份**——既不会因为有 3 个操作就把 `handler` 复制 3 份，在退出 `.use(handler)` 时也严格按剩余责任清理一次，绝不发生双重析构（Double Free）。

---

## 四、更广阔的舞台：用 `.use(pool)` 与 `.use(runtime)` 注入底层执行设施

`.use(...)` 的威力远不止于处理业务级的 `LimitErr` 或 `next_number`。
随着我们在后续章节进入**可恢复限定 Continuation（协程挂起/恢复）**与**异步效应（Async Effects）**，你会发现：
- 当一段代码使用了需要保存挂起栈帧的 Continuation 时，我们不需要在业务函数签名里到处传栈内存分配器，只需在安装边界写上一句 **`(work()# { ... }).use(pool)`**，就能通过处理协议将显式栈池实例 `pool` 注入进去；
- 甚至当一个完整的微内核运行时 `runtime` 同时封装了协程调度器与栈池时，只需写一句 **`.use(runtime)`**，就能一次性供应全套调度与栈存储服务。

---

## 五、本节核心概念速查表

| 语法 / 规则 | 正确写法 | 核心设计哲学与边界 |
| :--- | :--- | :--- |
| **函数签名规则** | 使用 `#Effects` / `!Errors` 与 `where T :- Trait` | **函数签名绝不添加 `handles`**；能力统一由 Trait 协议与 Elaboration 兑现 |
| **处理能力绑定** | `let h handles CapA and CapB = expr;` | 声明绑定对象提供的处理入口（支持嵌入元组等模式），不为未实现类型凭空造能力 |
| **挂载处理器对象** | `computation.use(h)` | 将 `h` 的处理入口与环境显式接入 `computation`；未加 `.use(h)` 的调用不受隐式影响 |
| **状态共享与清理** | 同一对象的多入口共享接收者 | 安装时环境只求值一次，多次请求共享状态，退出时按剩余责任唯一清理 |

---

## 六、下一期预告

到这里，我们已经能够熟练地声明效应、组合 `effect enum`、并用 `# { ... }` 或 `.use(handler)` 注入处理器了。
但此时此刻，每一位对性能有着极致洁癖的系统程序员心里一定都在打鼓：
> **“如果我只是想用代数效应向深层调用链注入一个只读的运行期配置项（比如画面宽度 `Settings.width()` 或采样上限 `limit()`），难道每次调用 `limit()#` 都要走一遍 Handler 分支函数调用、甚至保存寄存器切栈吗？”**

当然不需要。如果每次读取配置都要重新执行一遍 Handler 分支，不仅浪费 CPU 周期，万一分支里有初始化日志，还会被重复执行。
在 Flurry 中，只需在无参效应前加上一个轻量级属性 **`^attrs.effects.direct_passing`**，编译器就会在**安装 Handler 的那一刻只求值一次**，随后通过隐式传值直接把结果送达深层调用点，实现真正的**零切栈、零重复求值开销**。

下一期 **《Flurry 演进手记 (145)：不用切栈也能注入配置与上下文——Direct Passing（直接传值效应）的零开销机制》**，我们将揭晓这个兼顾优雅架构与极致性能的核心机制。
