# Flurry 演进手记 (072)：什么时候算、什么时候留——Ordinary 严格求值、Loose 宽松归约与 defer 延迟表达式

在上一篇中，我们认识了 Flurry 编译期引擎的双阶段流水线：**Normalization（定义端符号检查）** 与 **Instantiation（调用端具体实例化）**。

紧接着，我们在编写静态形状张量库 `Tensor` 时遇到了一个非常实际的工程难题：
我们想给张量写一个通用的运行时路由函数 `read_routed_channel`，或者一个类型安全的格式化日志函数 `log_tensor(comptime fmt: str, args: ...)`。
在这些函数的参数列表里，**后一个参数的类型，是由前一个编译期参数通过类型函数动态算出来的！**

比如：
```flurry
comptime fn Selected(flag: bool) -> Type {
    if flag {
        return i32;
    } else {
        return bool;
    }
}
```
注意看！这里的 `Selected` 是一个带 `if` 分支的 `comptime fn`，它**不是**单点返回的 Normal-Form 头（第 068 篇讲过，只有知道具体 `flag` 是 `true` 还是 `false`，它才能归约成 `i32` 或 `bool`）。

如果我们在一个普通的运行时函数签名里直接写：
`fn read_selected(comptime flag: bool, value: Selected(flag)) -> i32`
在定义端 Normalization 时，`flag` 只是一个未知的符号布尔值，`Selected(flag)` 根本没法算出一个具体的类型！这时候，严格检查的运行时函数签名就会陷入尴尬：算又算不出来，留又不能随便留一个未展开的非 Normal-Form 调用！

今天，我们就来看看 Flurry 是如何用 **Ordinary（严格检查） vs Loose（宽松检查）** 这两个正交强度等级，配合精巧的 **`defer` 延迟表达式**，把这个死结解得干脆利落的！

---

## 尝试 v0：非 Normal-Form 类型函数卡在运行时签名里的困境

让我们先把问题摆在桌面上。
在第 068 篇中我们学过：
- 如果一个类型函数是 **Normal-Form 头**（比如 `pure comptime fn Tensor<T, n: usize> -> Type`），那么即使 `T` 和 `n` 是未知的符号变量，`Tensor<T, n>` 本身就已经是一个合法的终态类型（Normal-Form Type），可以直接停留在函数签名里。
- 但如果一个类型函数**不是** Normal-Form 头（比如根据 `flag: bool` 分支返回 `i32` 或 `bool` 的 `Selected(flag)`，或者根据格式化字符串 `"shape=({}, {})"` 解析出参数元组类型的 `ParseFormat(fmt)`），它必须拿到具体的编译期输入才能算出真正的类型！

```flurry
comptime fn Selected(flag: bool) -> Type {
    if flag {
        return i32;
    } else {
        return bool;
    }
}

-- ❌ 编译期直接拒绝！
-- 普通运行时函数默认处于 Ordinary（严格）检查模式：
-- 在定义端 Normalization 时，flag 还是未知的符号变量，
-- Selected(flag) 既不是 Normal-Form 头，又无法在定义端归约为具体类型！
fn read_selected_v0(comptime flag: bool, value: Selected(flag)) -> i32 {
    return 0i32;
}
```

为什么普通运行时函数默认不让你直接写 `value: Selected(flag)`？
因为在 `Ordinary`（严格检查）模式下，编译器要求每个参数的类型在定义端就必须有明确的 Normal-Form 身份或完整的类型结构，这样才能对函数体里的 `value` 做严格的符号检查。

那么，如果我们确实需要写这种“等到调用端 Instantiation 拿到具体 `flag` 或具体 `fmt` 字符串时，才能算出参数类型”的函数，该怎么办？

在回答这个问题之前，我们先得搞清楚 Flurry 里两个经常被混淆的维度：**执行阶段（Stage）** 与 **检查强度（Strength）**！

---

## 演进 v1：正交的两个维度——Stage（`comptime`/`runtime`） vs Strength（`loose`/`ordinary`）

很多初学者以为：“只要跟编译期沾边的就是宽松检查，跟运行时沾边的就是严格检查。”
**错！在 Flurry 中，代码在哪执行（Stage）与编译器用什么强度检查（Strength），是两个彼此正交的独立概念！**

让我们画一张 **Code Cartoon** 四象限对比图，让你一眼看穿它们的区别：

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：执行阶段（Stage）与检查强度（Strength）的正交矩阵     |
|                                                                         |
|  1. 【执行阶段 Stage】决定“这段代码在什么时候跑、能动什么数据”：        |
|     • comptime：在编译期执行，操纵 U1 编译期值与类型                    |
|     • runtime ：在编译出来的目标程序里执行，操纵 U0 运行时内存与资源    |
|                                                                         |
|  2. 【检查强度 Strength】决定“Normalization 阶段遇到未决类型怎么办”：   |
|     • ordinary（严格检查）：                                            |
|       所有运行时函数 (fn)、普通类型定义与日常代码默认使用！             |
|       要求定义端就能确定每个值的类型结构或 Normal-Form 头部契约。       |
|                                                                         |
|     • loose（宽松检查）：                                               |
|       comptime fn 函数体与 comptime { ... } 块内部使用！                |
|       允许依赖未知编译期参数的动态类型计算保持未决状态，                |
|       将未决的动态类型操作挂起，留到 Instantiation 阶段再去具体兑现。   |
+-------------------------------------------------------------------------+
|  ⚠️ 最关键的正交证明：                                                  |
|  如果你在一个 `comptime { ... }` 块（Loose 环境）里面，                 |
|  定义了一个局部的运行时函数 `fn helper(x: T) -> T`：                    |
|  ——生成这个 helper 的外层块是 Loose 的；                                |
|  ——但 `helper` 本身作为一个运行时函数，它自己的签名和函数体             |
|    依然严格执行 **Ordinary（严格）** 检查！绝不会因为外层是 comptime    |
|    就退化成无契约的宏模板！                                             |
+-------------------------------------------------------------------------+
```

这下你彻底明白了：
- 在 `comptime fn` 或 `comptime { ... }` 块里，因为处于 **Loose（宽松检查）** 模式，你可以自由地写复杂的类型分支、循环拼装类型；
- 而在普通的运行时函数 `fn` 里，因为处于 **Ordinary（严格检查）** 模式，编译器要求参数类型在定义端就清清楚楚。

---

## 演进 v2：用 `defer` 显式延迟类型归约——在 Ordinary 签名中开一扇安全窗

现在回到我们的需求：我们在写一个普通的运行时函数（处于 `Ordinary` 模式），但其中某个参数的类型确实需要调用 `Selected(flag)` 或 `ParseFormat(format)`，必须等到调用端（Instantiation 阶段）传入具体的 `flag` 或 `format` 后才能算出来。

这时候，Flurry 提供了 **`defer expr` 延迟表达式**！

当你写下 `value: defer Selected(flag)` 时，你是在向编译器发出一条极其精准的指令：
> **“请把 `Selected(flag)` 的具体类型求值推迟（Defer）到调用端 Instantiation 阶段、当 `flag` 已知时再执行！而在定义端 Normalization 阶段，先把 `value` 当作满足已知约束的抽象类型（`Any` + 已知修饰约束）来进行严格检查。”**

让我们看看加上 `defer` 后的 `v2` 代码是如何丝滑工作的：

```flurry
comptime fn Selected(flag: bool) -> Type {
    if flag {
        return i32;
    } else {
        return bool;
    }
}

-- ✅ 演进 v2：使用 defer Selected(flag) 显式推迟非 Normal-Form 类型的具体求值！
fn inspect_selected(
    comptime flag: bool,
    value: defer Selected(flag),
) -> i32 {
    -- 在定义端 Normalization 阶段：
    -- value 的具体类型尚未展开，它在函数体中只暴露 Any（及显式附加的契约）。
    -- 当我们在下面用 inline if 根据 flag 分支处理时，
    -- 在调用端 Instantiation 代入具体 flag 后，Selected(flag) 就会立刻算出真身！
    inline if flag {
        return value;
    } else {
        if value {
            return 1i32;
        } else {
            return 0i32;
        }
    }
}

fn demo_defer() {
    -- 调用 1：flag = true
    -- Instantiation 阶段算出 Selected(true) == i32，第二个参数必须传 i32！
    let a = inspect_selected(true, 42i32);

    -- 调用 2：flag = false
    -- Instantiation 阶段算出 Selected(false) == bool，第二个参数必须传 bool！
    let b = inspect_selected(false, true);

    -- ❌ 编译期在调用端直接拒绝！
    -- 当 flag = true 时，defer Selected(true) 归约为 i32，传入 false (bool) 类型不匹配！
    let bad = inspect_selected(true, false);
}
```

### `defer` 的两大王牌应用场景

除了参数类型依赖于前面的 `comptime` 参数外，`defer` 还有两个极其强大的用武之地：

#### 场景 A：类型安全的 `format` / 张量打印签名
想象一下 `print(comptime format: str, args: defer ParseFormat(format))`：
- 在定义 `print` 时，`format` 字符串是未知的，所以用 `defer ParseFormat(format)` 把参数列表类型的计算推迟到调用点；
- 当用户在调用点写下 `print("epoch={}, loss={}", (10usize, 0.05f32))` 时，Instantiation 阶段拿到字面量 `"epoch={}, loss={}"`，当场算出 `ParseFormat` 的结果是 दो元素元组，如果用户少传了一个参数或者类型不对，编译期当场报错！

#### 场景 B：`where` 子句中可选参数的延迟默认值
在 `where` 子句中，如果一个可选子句参数 `.U: Type = defer DefaultTypeFor(T)` 的默认类型需要通过复杂的 `comptime fn` 计算得出，也可以用 `defer` 把默认类型的求值推迟到 `T` 被推导出来的那一刻：

```flurry
comptime fn AccTypeFor(T: Type) -> Type {
    if T == f16 {
        return f32;
    } else {
        return T;
    }
}

-- 可选子句参数 .Acc 的默认值使用 defer AccTypeFor(T)：
-- 等到调用端推导出 T 具体是什么类型时，再计算 AccTypeFor(T)！
fn accumulate_tensor(input: Array<T, n>, init: Acc) -> Acc
where
    T :- Copy,
    n: usize,
    .Acc: Type = defer AccTypeFor(T)
{
    return init;
}
```

> **⚠️ 重要安全边界：在定义端，未展开的 `defer` 类型只能当 `Any` 用！**
> 也许你会问：“既然 `value: defer Selected(flag)` 推迟到了调用端才算具体类型，那如果我在普通代码里直接对 `value` 写 `value + 1i32`（没有分支也没有约束），Normalization 会放过它吗？”
> **绝不会！** 在定义端未展开时，`defer Selected(flag)` 对普通代码只暴露最基础的 `Any`（以及你在 `where`/`requires` 中显式补充的约束）。如果你在普通代码里直接把 `value` 当成 `i32` 做加法，定义端 Normalization 会立刻报错——因为 `Any` 不支持 `+`！

```flurry
fn bad_defer_use(
    comptime flag: bool,
    value: defer Selected(flag),
) -> i32 {
    -- ❌ 编译期在定义处（Normalization 阶段）直接拒绝！
    -- 在未确定 flag 的普通上下文中，value 的已知类型信息只有 Any，
    -- 编译器绝不允许你盲目假设它一定是 i32 并执行 `+ 1i32`！
    return value + 1i32;
}
```

---

## 本篇小结

| 概念 / 关键字 | 所属维度 | 核心语义与行为 |
| :--- | :--- | :--- |
| **`comptime` vs `runtime`** | **执行阶段（Stage）** | 决定代码是在编译期运行（操纵 $U_1$）还是在目标程序运行（操纵 $U_0$） |
| **`ordinary`（严格检查）** | **检查强度（Strength）** | 普通代码与运行时函数默认强度（即使定义在 `comptime` 块内也保持 `ordinary`）；要求类型在定义端可确定结构或 Normal-Form 头 |
| **`loose`（宽松检查）** | **检查强度（Strength）** | `comptime fn` 与 `comptime { ... }` 块的内部强度；允许依赖未知编译期参数的动态类型操作留到 Instantiation 阶段归约 |
| **`defer expr`** | **显式延迟求值** | 将非 Normal-Form 类型表达式或可选子句默认值的具体计算推迟到 Instantiation 阶段；定义端未展开时按 `Any`（加已知约束）严格把关 |

---

### 留给下一篇的悬念

到现在为止，我们已经看到 $U_1$（编译期宇宙）和 $U_0$（运行时宇宙）在各自的轨道上井然有序地运转：
- $U_1$ 里可以算类型、算编译期配置；
- $U_0$ 里跑着高效的张量运行时算子。

但是，这两个宇宙之间是怎么**传递物质与信息**的呢？
1. 如果我们在写一个编译期宏或代码生成函数，怎么把一段 $U_0$ 的运行时代码表达式 `(x + 1)` **打包捕获（Quote）** 成 $U_1$ 宇宙里的语法树数据？
2. 当我们在 $U_1$ 宇宙里算好了一段代码，或者算好了一个编译期张量查找表（Lookup Table），又怎么把它**拼接注入（Splice）** 回 $U_0$ 的运行时世界？
3. 甚至，如果一个引用的目标类型本身还在运行时层面，怎么把它**提升（Lift）** 到编译期类型层？

下一篇 **`073-lift-quote-and-splice-across-universes.md`**，我们将正式踏上连接 $U_1$ 与 $U_0$ 两个宇宙的三座跨界大桥：**Lift、Quote 与 Splice**！
