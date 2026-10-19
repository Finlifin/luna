# Flurry 演进手记 (035)：一套签名、百种实现与批量验收——`template fn` 与 `template test`

在 [上一篇 (034)](./034-quote-functions-evaluate-arguments-once.md) 中，我们学会了用 `quote fn` 把一小段公共逻辑展开到调用点。

但在真实的传感器网关工程中，我们经常遇到另一种让人抓狂的样板代码（Boilerplate）：
1. **十几个函数共享同一套签名与契约**：比如所有的限幅处理器都接收 `(value: u32, limit: u32)`，都返回 `result: u32`，而且都必须遵守同一条后置契约 `where ensures result <= limit`！如果手抄十几遍签名，一旦以后参数名或契约改了，就得改十几个地方。
2. **十几个请求处理器共享同一套“前置校验 + 资源追踪骨架”**：每个处理函数一进来都要先创建 `let trace = RequestTrace { id: 1 }`，再用 `let number = request?;` 解包可选请求，最后只对解包出来的 `number: u32` 做几行不同的计算。
3. **几十个测试用例共享同一套 Fixture 环境**：每个 `test` 都需要先初始化计数器 `value: u32 = 0` 和指针 `counter = value.ref`，执行测试操作后再统一验证结果并清理资源。

在其他语言里，你要么写丑陋的预处理器宏，要么把每个小逻辑都包装成运行时闭包传给高阶函数（引入间接调用开销或复杂的返回类型推导）。
在 Flurry 中，这一切都由 **`template fn`** 与 **`template test`** 正面解决（详见 `docs/functions/templates.md`）！

---

## 第一版：复制粘贴签名的脆弱性 vs 简单签名模板（v0 → v1）

我们先看第一个需求：多个函数共享完全相同的参数列表、返回类型和形式化验证契约 `where ensures result <= limit`。

如果用傳統的 `v0` 写法，你得把 `(value: u32, limit: u32) -> result: u32 where ensures result <= limit` 在每个函数头上抄一遍。
而在 Flurry 的 `v1` 中，我们可以直接声明一个无主体的**简单签名模板（Template Signature，以分号 `;` 结尾）**，然后用 **`fn 名字 use 模板名 { ... }`** 来实例化它：

```flurry
use std.prelude.*;

-- 1. 定义无主体的签名模板（以分号 `;` 结尾）
-- 注意：template 的 function 必须是非 comptime 函数！
template fn bounded(value: u32, limit: u32) -> result: u32
where ensures result <= limit;

-- 2. 普通函数实例：直接通过 `use bounded` 继承参数名 value、limit 与后置契约！
fn clamp use bounded {
    if value > limit {
        limit
    } else {
        value
    }
}

-- 3. inline fn 或 quote fn 实例：保留各自的函数类别与修饰要求！
inline fn zero use bounded {
    0
}

quote fn clamp_code use bounded {
    if value > limit { limit } else { value }
}

test {
    println("{} {} {}", clamp(20, 8), zero(20, 8), clamp_code(20, 8));
}
```

运行这段代码，输出 `8 0 8`！

看看这里发生了什么：
- **参数名自动就位**：在 `clamp`、`zero` 和 `clamp_code` 的花括号里，你不需要再写 `(value: u32, limit: u32)`——因为 `use bounded` 已经把模板接口里的 `value` 和 `limit` 绑定到了当前实例的主体作用域中！
- **不改变实例自身的函数类别**：模板本身只是提供接口。`clamp` 依然是普通函数，`zero` 带有 `inline` 意向，`clamp_code` 则必须作为 `quote fn` 完成代码接入。
- **共享契约 100% 强制执行**：如果某个粗心的程序员写了一个违背 `ensures result <= limit` 的实例：

```flurry
-- ❌ 编译期直接拒绝！
-- 当 value > limit 时，直接返回 value 违反了模板声明的 `ensures result <= limit` 契约！
fn broken_clamp use bounded {
    value
}
```

编译器在检查 `broken_clamp` 时，会连同模板继承来的 `ensures result <= limit` 一起验证，当场抓出契约违背！

---

## 第二版：基于 CMTT 的正文复用——带上下文骨架的 `template fn`（v2）

光复用签名还不够过瘾！现在看第二个需求：
每次处理一个传感器请求 `request: ?u32` 时，我们都希望：
1. 先创建一个带 `Drop` 审计日志的追踪对象 `let trace = RequestTrace { id: 1 }`；
2. 用 `let number = request?;` 检查请求是否为空——如果为 `null`，直接提前返回 `null`（并自动触发 `trace` 的 `Drop` 清理！）；
3. **只有当 `request` 存在时**，才把拆出来的 `number: u32` 交给具体的业务处理函数正文，算出一个新的 `u32` 返回！

在 Flurry 中，这通过带有 **`quote lambda` 正文参数**与 **CMTT 上下文类型 `< Context |- Result >`** 的高级 `template fn` 实现！

让我们先画一张 **Code Cartoon**，看懂 `< number: u32 |- u32 >` 这个酷炫的符号到底是什么意思：

```text
  quote lambda body : < number: u32 |- u32 >
                        └────┬────┘    └─┬─┘
                             │           └── 正文执行完后，必须交出一个 u32 结果！
                             └── 模板向正文承诺提供的 runtime 上下文变量：
                                 在正文 `{ ... }` 里，你可以直接使用 `number: u32`！

  ══════════════════════ 模板与实例正文的扣合过程 ══════════════════════

  template fn checked_request(request: ?u32, quote lambda body: < number: u32 |- u32 >) -> ?u32
  ┌─────────────────────────────────────────────────────────────────────┐
  │ let trace = RequestTrace { id: 1 }   ◄── [模板私有变量，正文不可见!] │
  │ let number = request?;               ◄── [若为 null，在此提前退出   │
  │                                           并 Drop trace！]          │
  │ ~<body>                              ◄── [拼接点 ~<body>：          │
  │    │                                      注入变量 `number: u32`，  │
  │    └──► 嵌入实例正文，例如 `{ number / 2 }`                         │
  └─────────────────────────────────────────────────────────────────────┘
```

来看看完整的可运行代码（详见 `docs/functions/templates.md`）：

```flurry
use std.prelude.*;

struct RequestTrace {
    id: u32,
}

impl Drop for RequestTrace {
    fn drop(*self) {
        println("finish request {}", self.id.*);
    }
}

-- 高级函数模板：建立局部上下文并通过 ~<body> 接入实例正文
template fn checked_request(
    request: ?u32,
    quote lambda body: < number: u32 |- u32 >,
) -> ?u32 {
    let trace = RequestTrace { id: 1 }
    let number = request?;
    ~<body>
}

-- 实例 1：把有效请求折半
fn halve_request use checked_request {
    println("halve {}", number);
    number / 2
}

-- 实例 2：把有效请求限制在 10 以内
fn clamp_request use checked_request {
    if number > 10 {
        10
    } else {
        number
    }
}

test {
    halve_request(8) match {
        value? => println("result {}", value),
        null => println("absent"),
    }
    clamp_request(null) match {
        value? => println("result {}", value),
        null => println("absent"),
    }
}
```

运行这段代码，控制台依次打印：

```text
halve 8
finish request 1
result 4
finish request 1
absent
```

仔细品味这段代码带来的工程震撼：
1. **调用者眼里只有普通函数参数**：当你调用 `halve_request(8)` 时，你**不需要**传什么 `body`——因为 `quote lambda body` 已经在定义 `fn halve_request use checked_request { ... }` 时静态绑定好了！
2. **早期退出依然 100% 触发资源清理**：在 `clamp_request(null)` 中，`let number = request?;` 遇到 `null` 直接提前返回，正文根本没有执行，但模板里创建的 `trace` 依然稳稳地执行了 `Drop`，打印出 `"finish request 1"`！
3. **严格的上下文隔离（绝不泄漏模板私有变量！）**：
   你可能会问：“既然 `~<body>` 把正文拼进了模板里，那正文能不能偷偷访问模板里的局部变量 `trace`？”
   **绝对不行！** 因为 `body` 的类型写得清清楚楚是 `< number: u32 |- u32 >`，模板只向正文授权了 `number`，没有授权 `trace`！

```flurry
-- ❌ 编译期直接拒绝！
fn inspect_template_local use checked_request {
    -- 错误！`trace` 是模板内部的私有局部变量，不在 `< number: u32 |- u32 >` 公开的上下文接口中！
    trace.id
}
```

这彻底杜绝了传统宏或文本模板中“内部临时变量名污染用户代码”或“用户代码意外篡改模板内部状态”的隐患！同时，每个实例（如 `halve_request` 和 `clamp_request`）每次调用都会建立各自独立的局部存储，绝不会跨实例共享同一个 `number` 或 `trace`。

---

## 第三版：批量验收利器——`template test` 与锚定子句参数（v3）

学会了 `template fn`，再看第三个需求——**测试模板 `template test`**——简直是水到渠成！

当我们有几十个测试都需要先准备一套相同的计数器指针或资源追踪器时，我们可以定义一个 `template test`。
它的语法有一点贴心的小区别：因为 `test` 本身没有运行时的调用参数列表 `(...)`，所以 `template test` 通过 `where` 子句中**唯一的一个锚定子句参数（Anchored Clause Parameter，如 `where body: < counter: *u32 |- void >`）**来接收测试正文！

来看看 `template test` 是如何工作的：

```flurry
use std.prelude.*;

-- 定义测试模板：通过唯一锚定子句参数 `body` 授予 `counter: *u32` 上下文
template test with_counter
where body: < counter: *u32 |- void >
{
    let value: u32 = 0;
    let counter = value.ref;
    ~<body>;
    println("final {}", value);
}

-- 测试实例 1：自增一次
test increment use with_counter {
    counter.* += 1;
}

-- 测试实例 2：自增两次（拥有自己全新初始化的 value = 0，绝不与上一个测试串台！）
test increment_twice use with_counter {
    counter.* += 1;
    counter.* += 1;
}
```

运行这两个测试实例：
- `increment` 独立输出 `final 1`；
- `increment_twice` 独立输出 `final 2`！

> **⚠️ 重要心智提醒：写在 `~<body>;` 后面的普通语句不是 `finally`！**
> 在 `template fn` 或 `template test` 中，如果你在 `~<body>` 后面写了一条普通的 `println(...)`，而实例正文里执行了提前 `return`（或错误传播 `!` / `?`），那么控制流会直接沿着退出路径离开，`~<body>` 后面的普通语句会被正常跳过。
> **真正需要覆盖所有提前退出路径的清理动作，永远交给像 `RequestTrace` 那样实现了 `Drop` 的资源对象！** 即使测试正文不需要任何上下文变量（写作空上下文 `where body: < |- void >`），模板里的 `Drop` 对象也会在测试退出时自动完成清理。

---

## 本篇小结

从简单签名复用到带 CMTT 上下文的正文接入，我们在本篇掌握了 Flurry 消除结构性重复的三大模板工具：

| 模板形式 | 声明与实例化语法 | 核心语义与安全边界 |
| :--- | :--- | :--- |
| **简单签名模板** | `template fn f(x: u32) -> u32;` <br> `fn g use f { ... }` | 共享参数名、返回类型与 `requires`/`ensures` 契约；不改变实例自身的 `fn` / `inline fn` / `quote fn` 类别 |
| **上下文正文模板** | `template fn f(..., quote lambda body: < ctx |- R >) { ... ~<body> }` | 模板先完成公共校验与资源准备，再将授权的 `ctx` 绑定接入正文；模板私有变量对正文严格不可见 |
| **测试模板** | `template test t where body: < ctx |- void > { ... ~<body>; }` <br> `test my_test use t { ... }` | 通过唯一锚定子句参数绑定测试正文；每个测试实例拥有独立的初始化状态与受检的 `Drop` 退出清理 |

---

### 留给下一篇的悬念

仔细看我们在 `with_counter` 测试模板里写的代码：
我们在模板里把 `counter` 定义成了指针 `*u32`，所以在测试正文里写了 `counter.* += 1;`。
但回想一下我们在第 003 篇里写的 `Counter` 结构体方法：

```flurry
struct Counter {
    value: u32,
    fn read(*self) -> u32 { self.value }
}
```

如果在一个复杂的传感器拓扑结构中，你手里拿到的不仅是一层指针 `pointer: *Counter`，而是指向指针的二级指针 `outer: **Counter`（甚至三级指针 `***Counter`）！
当你直接写下 **`outer.read()`** 时：
- 为什么你不需要手写 `outer.*.*.read()`，编译器就能顺着多层指针直接找到 `Counter.read`？
- 在穿过 `**Counter` $\rightarrow$ `*Counter` 的过程中，编译器会傻乎乎地把整个 `Counter` 结构体复制（Copy）一份出来吗？
- 当一个类型同时有固有方法（Inherent Method）、多个 Trait 方法以及自动解引用（`Deref`）目标上的方法时，Flurry 到底按照什么优先级顺序决定调用哪一个？

在下一篇 **《Flurry 演进手记 (036)：指针层级再深也能一键调用——方法接收者适配与三层查找优先级》** 中，我们将揭开 Flurry 接收者自动适配引擎与“定义端固定成员来源”的精密法则！
