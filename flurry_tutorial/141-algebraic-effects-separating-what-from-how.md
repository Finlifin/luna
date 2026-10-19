# Flurry 演进手记 (141)：把“做什么”与“怎么做”彻底拆开——代数效应 effect 与 #Effects T 初探

> **系列导读**：欢迎来到《Flurry 演进手记》**第七卷：控制流的大一统——代数效应、限定 Continuation 与结构化并发（141 – 160）**！在前六卷中，我们掌握了从底层内存、形式化验证、双层类型理论到 Trait 多态与编译管线投影的全套静态武器。今天，我们将进入现代编程语言理论最令人心潮澎湃的领域——**代数效应（Algebraic Effects）与限定延续（Delimited Continuations）**！我们将围绕第七卷的贯穿实战项目——**“支持虚拟时钟确定性测试、用户态协程调度与异步无锁 I/O 的微型运行时（Micro-Runtime）”**，看看 Flurry 如何用一套简洁的正交机制，彻底终结系统编程中“全局状态污染”与“Context 参数传染”的百年难题！

---

## 一、系统架构师的永恒梦魇：版本 `v0` 的“两难死局”

假设我们正在为微型运行时编写一个带超时重试的心跳检测任务 `ping_with_timeout`。在检测过程中，这个函数需要两项外部环境能力：
1. **读取当前毫秒时钟（`now_ms()`）**；
2. **输出诊断日志（`log_msg(...)`）**。

在传统的系统编程语言（C / C++ / Rust / Go）中，当你写到这儿时，立刻会被逼入一个痛苦的二选一死局：

### 痛点 A：直接调全局系统调用（污染纯度，单测沦为噩梦！）

```flurry
-- v0-A：直接硬编码底层系统时钟调用
fn check_elapsed_bad_a(start_ms: u64) -> bool {
    -- ❌ 致命痛点：
    -- 1. 签名上完全看不出它偷偷读取了系统时钟，破坏了纯度与可观测性！
    -- 2. 在单元测试里，为了测试“超时 5000ms”的分支，难道要让测试线程真的傻等 5 秒钟吗？
    let current = os_system_time_ms();
    return current - start_ms >= 5000u64;
}
```

### 痛点 B：把 `Clock` 和 `Logger` 塞进每个函数的参数列表（“Context 参数传染病”）

为了能做确定性的“虚拟时钟（Mock Clock）”测试，工程师被迫把所有环境依赖全部塞进函数签名里：

```flurry
-- v0-B：满屏的 Context / Clock / Logger 参数层层透传！
fn leaf_step(clock: *Clock, logger: *Logger) -> u32 { ... }
fn middle_step(clock: *Clock, logger: *Logger) -> u32 {
    -- middle_step 自己根本不用 clock，却被迫当“二传手”！
    return leaf_step(clock, logger);
}
```

一旦最底层的第 10 层函数新加了一个随机数需求 `Rng`，中间的 9 层函数签名全部要跟着改！更何况，如果遇到像**协程挂起（Yield）**、**生成器按需产出（Generator）**或**异步等待（Async/Await）**这种不仅要拿数据、还要**暂停并保存当前控制流**的需求，传普通参数或回调函数就会立刻退化成臭名昭著的“回调地狱（Callback Hell）”！

能不能有一种机制，既让被调函数只需声明**“我需要请求什么操作（What）”**，又让外层调用方在安装处自由决定**“这次请求具体怎么兑现、甚至何时恢复执行（How）”**，而且全程受静态类型系统严密追踪？

这就是 **代数效应（Algebraic Effects）**！

---

## 二、迭代演进 `v1`：声明代数效应 `effect` 与限定类型 `#Effects T`

在 Flurry 中，一个代数效应操作就像是一个**“由计算向外部调用环境提出的、留待回答的问题”**。

让我们看看它的核心语法：

```text
algebraic_effect      -> async? inline? effect id(parameter*) (-> result)? clauses?
effect_qualified_type -> #expr expr
perform_effect        -> expr#
effect_handle         -> expr# { case_arm* }
```

请注意三条基础语法约定：
1. **声明效应操作**：使用 `effect id(parameter*) (-> result)?;`。它只有签名，**没有函数体，末尾以分号 `;` 结束**；如果省略 `-> result`，默认返回类型为 `void`（遵循命名规范，效应名采用 `snake_case`）。
2. **标注函数的效应需求**：在返回类型前使用前缀 **`#expr ResultType`**（例如 `#[read_clock] u64` 或 `#read_clock u64`）。正如 `?T` 表示可选、`!Err T` 表示错误结果，**`#Eff T` 表示“一段在执行过程中可能请求 `#Eff` 效应、最终产出 `T` 的计算”**！
3. **发起效应请求（Perform）**：在调用效应操作后紧跟后缀 **`#`**（例如 `read_clock()#`）。

让我们用代数效应重写刚才的时钟与计算示例，进入清爽的 `v1` 时代：

```flurry
-- 1. 声明一个代数效应操作：请求获取一个 u32 数值（无函数体，以分号结尾）
effect next_number() -> u32;

-- 2. 编写业务逻辑：只声明“我需要 #[next_number] 效应（What）”，
-- 完全不关心这个数字是从硬件寄存器读的、还是单元测试喂进来的！
fn total() -> #[next_number] u32 {
    -- 使用后缀 # 发起效应请求，等待外部环境回答一个 u32
    let first = next_number()#;
    let second = next_number()#;
    return first + second;
}
```

---

## 三、终极演进 `v2`：用 `expr# { ... }` 安装处理器（Handler），一套代码万种解释！

现在，`total()` 函数提出了两次 `next_number()#` 请求。谁来回答这个问题呢？
答案是：调用方通过后缀 **`expr# { case_arm* }`** 为这次计算安装一个 **效应处理器（Effect Handler）**！

### Code Cartoon：“做什么（What）”与“怎么做（How）”的时空对话

```text
  [ 调用方安装 Handler ]                       [ 被调主体 total() ]
  total()# {                                  fn total() -> #[next_number] u32 {
      next_number() => 6u32,                      let first = next_number()#;
  }        ▲                \                                      │
           │                 \ (2) 回答操作结果 Y = 6u32            │
           │                  \───────────────────────────────────►│
           └───────────────────────────────────────────────────────┘
                          (1) 发出请求 next_number()#
                                                                   ▼
                                                  (3) 拿到 first = 6u32，继续往下跑！
                                                  let second = next_number()#;
                                                  (再次问答得到 6u32，最终返回 12u32！)
```

让我们看看同一个 `total()` 函数（或者 `twice_input()`），在不同的调用环境下如何无需修改一行代码，就无缝切换不同的行为：

```flurry
effect read_number() -> u32;

fn twice_input() -> #[read_number] u32 {
    let number = read_number()#;
    return number * 2u32;
}

test {
    -- 场景 1：在单元测试 A 中，注入恒定返回 7u32 的虚拟处理器
    let first = twice_input()# {
        read_number() => 7u32,
    };

    -- 场景 2：在单元测试 B 中，注入恒定返回 11u32 的虚拟处理器
    let second = twice_input()# {
        read_number() => 11u32,
    };

    println("{} {}", first, second); -- 输出：14 22
}
```

### ⚠️ 严密的类型安全底线：未处理的效应绝不可能漏网！

如果有人调用了 `twice_input()`，却既没有在后面安装 `# { read_number() => ... }` 处理器消去该效应，也没有用后缀 `twice_input()#` 将效应继续向外传播（或者当前外层函数签名根本没有声明 `#[read_number]`），会发生什么？

```flurry
-- 外层函数未声明 #[read_number] 效应行
fn forgot_to_handle_bad() -> u32 {
    -- ❌ 编译期直接拒绝！
    -- twice_input() 的类型是 #[read_number] u32，而非裸的 u32！
    -- 未提供 read_number 处理入口，也未在签名中声明向外传播！
    return twice_input();
}
```

看看这带来了什么革命性的改变：
1. **零隐藏副作用**：一个函数只要可能请求外部时钟、日志或 I/O 效应，它的返回类型 `#Effects T` 就会像水晶一样诚实地反映在签名上；一旦所有效应都被外层 Handler 拦截消去，返回类型就变回了纯净的 `u32`！
2. **多次调用不膨胀类型行**：注意看前面的 `total()` 函数，它在里面连续调用了**两次** `next_number()#`，但它的返回类型依然只是 **`#[next_number] u32`**（而不是写两遍 `next_number`）！因为效应行记录的是“这段计算允许请求哪些操作入口”，只要两次顺序调用使用的是同一层 Handler，就只需要一层 `next_number` 许可！

---

## 四、本节核心概念速查表

| 语法构件 | 角色与位置 | 核心语义规则 |
| :--- | :--- | :--- |
| `effect name(params) -> R;` | 代数效应操作声明 | 无函数体，以分号 `;` 结尾；省略 `-> R` 时结果类型为 `void` |
| `#Eff T` / `#[e1, e2] T` | 效应限定类型（前缀） | 标注计算在产出 `T` 的过程中可能请求的效应行；多次顺序调用同一操作不复制标签 |
| `op(args)#` | 发起效应请求（后缀） | 向当前外层安装的对应 Handler 发出请求，取得操作结果 `R` 后继续执行 |
| `expr# { pat => body, }` | 安装效应处理器（后缀） | 拦截并回答 `expr` 内部发出的效应请求，消去对应的效应层 |

---

## 五、下一期预告

今天我们初识了代数效应的声明与最简单的直接返回处理。
但细心的你一定注意到了两个核心问题：
1. 在多层函数调用链（如 `leaf() -> middle() -> root()`）中，`middle()` 自己并不打算处理 `read_number` 效应，它该如何像第二卷用 `!` 传播错误一样，用后缀 **`#`** 把内层计算的效应顺畅地向外层传播？
2. 更关键的是：在 Handler 分支 `scale(value) => { ... }` 内部，如果我们需要在特定条件下**提前把操作结果交回给请求点**，或者对比为什么错误 `!` 是“一去不复返的短路退出”，而效应 `#` 却能通过 **`resume expr`** 把值送回暂停点继续往下算——这背后的控制流机制究竟是怎样的？

下一期 **《Flurry 演进手记 (142)：发起请求与就地拦截——后缀 # 传播、expr# { ... } 效应消去与 resume》**，我们将深入剖析效应传播、分支恢复关键字 `resume` 以及内层 Handler 如何安全地向外层请求同名效应！
