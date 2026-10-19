# 第 182 期：Elaboration 深度拆解——兄弟参数、求值计划与双向流

> **系列导航**：[Flurry 语言漫游（200 期规划）](./README.md)  
> **当前卷目**：第八卷 · 编译器原理与全栈实战（181–200）  
> **上一期**：[第 181 期：为什么 Flurry 的所有 IR 都转储为 Lisp S-Expression？](./181-why-all-flurry-irs-dump-as-lisp-s-expressions.md)  
> **下一期**：[第 183 期：局部依赖图、等待票据（Wait Tickets）与无竞态唤醒](./183-local-dependencies-wait-tickets-and-wakeups.md)

---

## 一、开篇痛点：当闭包写在第一个参数，为什么严格从左到右的类型检查会卡死？

在日常编程中，我们经常写出下面这样自然流畅的高阶函数调用：

```flurry
fn apply_first(step: fn(T) -> R, value: T) -> R where T, R {
    return step(value);
}

const RESULT: i32 = apply_first(|item| item + 1, 41i32);
```

请仔细观察 `apply_first(|item| item + 1, 41i32)` 这一行代码：
- 第一个实参是一个没有写参数类型注解的匿名函数 `|item| item + 1`；
- 第二个实参是一个具体类型明确的整数字面量 `41i32`。

人类一眼就能看明白：既然第二个形参 `value: T` 传入了 `41i32`，那么泛型参数 `T` 就一定是 `i32`；既然 `T` 是 `i32`，那么第一个形参 `step: fn(T) -> R` 接收的 `item` 自然也是 `i32`，所以 `item + 1` 完全合法，返回值 `R` 也是 `i32`！

### 初代尝试（`v0`）：严格按源码从左到右单向推导的检查器

然而，如果我们按照传统编译器最朴素的思路（`v0`）来写函数调用的类型检查器——**严格按照实参列表从左到右的顺序，逐个把实参类型彻底检查完，再去合一（Unify）下一个实参**：

```
  [ v0 严格从左到右单向检查的崩溃现场 ]
  调用: apply_first( |item| item + 1 ,  41i32 )
                     ^^^^^^^
  第 1 步：先检查第 1 个实参 |item| item + 1
          此时 T 还是未知元变量 ?T0！
          检查函数体 `item + 1`：尝试在未知类型 ?T0 上解析算术加法 `+` -> ❌ 报错：无法在未知类型上执行 `+`！
  （编译器在第 1 个参数就放弃报错了，根本还没看一眼右边的 `41i32`！）
```

这就是许多早期语言迫使程序员把 `apply_first(value, |item| ...)` 的参数顺序反过来写，或者强迫程序员在每个短闭包上冗余地写出 `|item: i32| item + 1` 的根本原因。

那有的同学会问：**“既然从左到右会卡住，我们能不能干脆在编译器里把实参顺序调换一下，先处理 `41i32` 再处理闭包？”**

慢着！如果第二个参数不是字面量 `41i32`，而是一个带有副作用的函数调用 `produce_value()`，第一個参数也是一个带有副作用的表达式 `make_Transform()` 呢？如果我们为了编译期类型推导而随意调换实参的处理顺序，会不会导致**运行时的参数求值顺序**也被悄悄颠倒了？

---

## 二、机制迭代（`v0 -> v1 -> v2`）：构建双向信息流与独立的求值计划

让我们看看 Luna 编译器的 **Elaboration（精化）** 引擎是如何分两步彻底解决这个难题的。

### 第一次迭代（`v1`）：两遍扫描（先粗略收集具体类型，再检查闭包）

一种常见的改进（`v1`）是“硬编码两遍扫描”：第一遍跳过所有闭包，先检查非闭包实参；第二遍再回过头来检查闭包。

但 `v1` 很快就会在稍微复杂一点的场景下失效：
- 如果第一个参数不是直接的闭包，而是包裹在结构体构造或元组里的闭包呢？
- 如果默认参数（Default Parameter）依赖前面某个显式参数的类型或值呢？
- 如果外层期待类型（Expected Type，比如 `const RESULT: i32 = ...`）也需要参与约束呢？

### 第二次迭代（`v2`，Luna 最终方案）：共享调用推断区域 + 编译期信息流与运行时求值计划解耦

在 Luna 编译器中，每一次函数调用或构造器调用的 Elaboration（`elaborate_call`）都遵循两个核心原则：

#### 原则 1：双向与兄弟参数联合约束（Bidirectional & Sibling Constraint Flow）

当 Elaboration 处理一个调用时，它会先为该调用绑定的泛型参数创建**新鲜的推断变量（Fresh Inference Variables）**，并把三股信息流接入同一个调用推断区域：
1. **被调用者签名（Callee Signature）**：形参类型 `step: fn(?T, ) -> ?R` 与 `value: ?T`，以及返回类型 `?R`；
2. **外层期待类型（Outer Expectation）**：例如 `const RESULT: i32` 从外向内推入约束 `?R = i32`；
3. **所有兄弟实参（Sibling Arguments）**：第二个实参 `41i32` 确定 `?T = i32`，这个结论立刻通过共享推断变量 `?T` 流入第一个实参 `|item| item + 1` 的输入期待类型中！

```
                    外层期待: i32
                         |
                         v (下行推入 ?R = i32)
              +----------------------+
              |  调用推断区域 (Call)  |
              |  ?T = i32, ?R = i32  |
              +----------------------+
               /                    \
   (流入输入类型 ?T = i32)       (上行综合出 ?T = i32)
             v                        \
  实参 1: |item| item + 1            实参 2: 41i32
  在 item: i32 下检查 item + 1 -> OK!
```

#### 原则 2：编译期类型信息流绝不重排运行时求值顺序（Evaluation Plan）

为了彻底杜绝“编译期先利用了右边参数的类型信息，结果不小心把右边参数的运行时求值也提前了”的隐患，`elaborate_call` 的输出被严格定义为三元组：

```
elaborate_call(callee, args, expected_type) -> (result_ir, evidence, evaluation_plan)
```

其中 **`evaluation_plan`（求值计划）** 显式锁死了运行时的求值契约：

```flurry
fn produce_step() -> fn(i32) -> i32 {
    return |item| item + 1;
}

fn produce_arg() -> i32 {
    return 41i32;
}

fn demo() -> i32 {
    -- 编译期：两边共同约束 T = i32, R = i32
    -- 运行时：严格按源码顺序，先求值第 1 个实参，再求值第 2 个实参，绝不重排或重复求值！
    return apply_first(|item| item + 1, produce_arg());
}
```

在 `evaluation_plan` 中，编译器明确记录了四个维度的执行契约：
1. **显式实参源码顺序（Explicit Argument Source Order）**：`arg_0` 必须在 `arg_1` 之前求值，且每个显式实参在运行时精确求值一次；
2. **默认参数依赖顺序（Default Parameter Dependencies）**：未显式传入、使用默认值表达式的形参，在所有它所依赖的先行形参绑定完成后求值；
3. **隐式参数与证据供给（Implicit & Evidence Supply）**：通过上下文查找或 trait 求解合成的隐式证据，按契约位置注入；
4. **传递角色（Delivery Roles）**：区分普通按值传递、借用传递以及 `quote` 代码片段传递。

让我们用第 181 期学到的 S-Expression 转储来看看 `apply_first(|item| item + 1, produce_arg())` 精化后的 IIR 与求值计划：

```scheme
(iir-call
  :callee (fn-inst "apply_first" :subst ((T i32) (R i32)))
  :ret-ty i32
  :eval-plan (eval-plan
    (step 0 :role explicit-value :param "step"
      :expr (iir-closure :params ((item i32)) :ret-ty i32
              (iir-binop add (iir-local "item") (iir-lit 1 i32))))
    (step 1 :role explicit-value :param "value"
      :expr (iir-call :callee (fn-inst "produce_arg") :ret-ty i32))))
```

看！尽管在编译期，`step 0` 里的闭包体是因为 `step 1` 确定了 `T = i32` 才完成类型检查的，但在最终生成的 `eval-plan` 中，`step 0` 和 `step 1` 的运行时执行顺序依然丝毫不差地保持着源码中的先后次序！

---

## 三、深度法则：新鲜推断变量 vs 定义点的刚性类型参数（Rigid Parameters）

理解了调用点的双向推断后，很多初学者会产生一个疑问：**“既然编译器在调用 `apply_first(|item| item + 1, 41i32)` 时能自动推导出 `T = i32`，那如果我在定义泛型函数时直接写 `value + 1`，编译器能不能也帮我自动猜出 `T = i32`？”**

答案是：**绝对不行！** 这是 Luna 类型系统中最关键的一道防火墙——区分**新鲜推断变量（Fresh Inference Variables）**与**刚性类型参数（Rigid / Skolem Parameters）**。

让我们对比下面两段代码：

```
  [ 调用点 (Call Site) ]                         [ 定义点 (Definition Site) ]
  apply_first(|item| item + 1, 41i32)           fn invalid_increment(value: T) -> T where T {
                                                    return value + 1;
                                                }
  这里的 T 实例化为【新鲜推断变量 ?T0】            这里的 T 是【全称量化的刚性符号 Rigid(T)】
  职责：找到一个满足当前调用的 ?T0                职责：证明对【宇宙间任意 T】函数体都成立！
  ?T0 可以被收窄为 i32 -> ✅ 合法                Rigid(T) 绝不能为了迁就 + 1 而擅自收窄为 i32 -> ❌ 拒绝！
```

当你在函数签名写下 `fn invalid_increment(value: T) -> T where T` 时，你向所有的调用者许下了一个契约：**“无论你传入任何类型 `T`（哪怕是一个布尔值 `bool` 或一个网络连接 `Socket`），我的函数体都能正常工作。”**

如果在检查 `invalid_increment` 的函数体时，编译器因为看到了 `value + 1` 就自作主张地把泛型参数 `T` 当成推断变量并赋值为 `i32`，那就等于单方面撕毁了泛型契约！

因此，在 Luna 的 Elaboration 引擎中：
- **定义点绑定的参数（`where T`、显式绑定在当前定义域的符号）** 被标记为 **Rigid（刚性符号）**。除非定义点显式书写了 `where` 约束或通过编译期分支证明了类型等式，否则求解器绝不允许为了凑齐某个操作符或候选实现而逆向猜测或收窄刚性参数；
- **只有为某一次具体调用（Call Site）或局部占位符新创建的推断变量（`?T0`）**，才允许在双向流中被具体类型赋值收窄。

---

## 四、边界与编译期拒绝：刚性参数越权与无界二义性

让我们通过两个编译期拒绝例，看看 Elaboration 是如何严格执行这些契约的。

### 拒绝例 1：试图在无约束的刚性泛型参数上执行算术运算

```flurry
fn invalid_increment(value: T) -> T where T {
    return value + 1; -- ❌ 编译期直接拒绝！`T` 是当前定义点的刚性类型参数（Rigid Parameter），缺少加法的类型契约，编译器绝不会逆向猜测 `T = i32`
}
```

### 拒绝例 2：调用点结束后仍有影响语义的推断变量悬而未决

如果一个调用既没有兄弟实参提供具体类型，也没有外层期待类型提供约束，导致关键推断变量在局部区域结束时依然处于未决定状态（Unconstrained），Elaboration 绝不会随便瞎猜一个默认类型蒙混过关，而是立即报告二义性错误：

```flurry
fn parse_any(text: String) -> R where R {
    -- 假设根据目标类型 R 解析字符串
    ...
}

fn ambiguous_caller() {
    -- 既没有传入能约束 R 的兄弟实参，也没有把返回值赋给带类型注解的变量
    let _ = parse_any("42"); -- ❌ 编译期直接拒绝！调用结束后类型变量 `?R` 仍未被唯一确定，拒绝生成含未决类型空洞的 IIR
}
```

---

## 五、本期小结

| 核心概念 | 编译期职责 | 运行时/语义边界 |
| :--- | :--- | :--- |
| **双向与兄弟参数流** | 被调用者签名、外层期待类型与所有兄弟实参共享调用推断变量，互相传递类型约束 | 允许后置实参（如 `41i32`）为前置闭包（如 `\|item\| item + 1`）提供输入类型上下文 |
| **求值计划 (`evaluation_plan`)** | 显式记录实参源码顺序、默认参数依赖、隐式证据供给与传递角色 | 编译期的跨参数信息流**绝不改变**运行时的从左到右求值顺序与单次求值保证 |
| **新鲜推断变量 (`?T`)** | 在具体调用点或显式占位处创建，允许被合一算法赋值为具体类型 | 必须在当前推断区域结束前被唯一确定，不得残留在最终 IIR 中 |
| **刚性类型参数 (`Rigid(T)`)** | 代表定义点的全称量化契约（如 `where T`） | 只能依靠已声明的 `where` 前提或内建等价关系推理，绝不允许被求解器逆向猜测收窄 |

现在，一个更深层的工程问题浮出水面了：在 `apply_first(|item| item + 1, 41i32)` 的精化过程中，当编译器先看到 `|item| item + 1` 时，`?T` 确实还是空的。编译器究竟是用什么数据结构把 `|item| item + 1` 暂时“挂起（Suspend）”，等处理完右边的 `41i32` 之后，又是如何精准、无死锁、无竞态地把挂起的闭包检查任务“唤醒（Wake up）”的呢？

在下一期（**第 183 期**）中，我们将深入 Luna 求解器的心脏——**局部依赖图、等待票据（Wait Tickets）与无竞态唤醒机制**！我们下期见！
