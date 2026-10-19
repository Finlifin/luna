# Flurry 演进手记 (073)：穿越两个宇宙的桥梁——隐式与显式的 Lift、Quote 与 Splice

在第 066 篇开启第四卷时，我们确立了 2LTT（两级类型论）最核心的宇宙法则：
**$U_1$（编译期宇宙）与 $U_0$（运行时宇宙）之间有一道严密的时间与空间防火墙。**

如果两个宇宙完全老死不相往来，那编译期算出来的东西怎么喂给运行时？运行时的代码结构又怎么交给编译期去分析和展开？

在边缘端神经网络推理引擎中，我们马上就会遇到三个典型的“跨宇宙通信”场景：
1. **编译期预计算查找表（Lookup Table）落地到运行时**：我们在编译期用高精度泰勒展开算好了一张激活函数 GELU 的 256 点量化查找表 `LUT`，怎么把这个 $U_1$ 的编译期数组变成 $U_0$ 运行时可以直接读取的数据？
2. **捕获运行时表达式结构（Code Quoting）**：我们想写一个自动算子融合（Kernel Fusion）或调试追踪工具，把用户写的运行时表达式 `a * x + b` 的语法结构捕获进 $U_1$ 分析，再把变换后的代码贴回 $U_0$ 执行；
3. **跨层级引用参数（`quote` 参数与 `lift` 类型提升）**：当一个参数既要捕获调用点的运行时绑定描述，它的类型本身又跨越了运行时与编译期边界时，怎么在签名里精确声明？

在 Flurry 中，穿越 $U_1$ 与 $U_0$ 的所有通道，都被严格收拢在三座显式或隐式的桥梁上：**Lift（提升）、Quote（引用捕获）与 Splice（拼接落地）**！

---

## 全景心智图：连接 $U_1$ 与 $U_0$ 的三座大桥

在深入每一座桥之前，让我们先用一张 **Code Cartoon** 看清这三座桥在两个宇宙之间的方向与职责：

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：穿越 U1 与 U0 宇宙的三座桥梁                          |
|                                                                         |
|   🌌 U1 编译期宇宙（Types, Comptime Values, Quoted ASTs/Bindings）      |
|       ▲                        ▲                        │               |
|       │ ① Lift (lift T)        │ ② Quote ('(e), quote)  │ ③ Splice      |
|       │ 将运行时类型表示        │ 将运行时代码或符号     │ (~<e> 或      |
|       │ 提升到编译期类型层      │ 捕获为 U1 编译期对象   │  Splice.splice)|
|       │                        │                        ▼               |
|   🌍 U0 运行时宇宙（Runtime Values, Expressions, Bindings, Memory）     |
+-------------------------------------------------------------------------+
|  ⚠️ 铁律不变：                                                          |
|  Quote 捕获的是 U0 代码的**静态语法结构或绑定描述**，                   |
|  绝不是穿越时空去读取未来运行时的寄存器内存值！                         |
+-------------------------------------------------------------------------+
```

---

## 第一座桥：Quote（`'(expr)`、`'<pat>` 与 `quote` 参数）——把 $U_0$ 结构装进 $U_1$ 胶囊

### 尝试 v0：直接拿运行时变量进 `comptime`，撞上宇宙防火墙

正如我们在第 066 篇见过的，如果你直接把一个运行时变量 `x` 传给 `comptime` 参数，编译器会当场拦截，因为编译期还不知道 `x` 运行时的值是多少。

但如果我们想在编译期拿到的**不是 `x` 运行时的数值，而是 `x + 1` 这段代码的语法树（Quoted Code）或者 `x` 这个绑定符号本身（Symbol/Binding）**呢？

### 演进 v1：用 `'(expr)` 引用代码，用 `quote` 参数捕获绑定

Flurry 提供了清晰直观的 Quote 语法：
1. **表达式引用 `'(expr)` / `'{ ... }`**：把圆括号或花括号里的代码打包成一个 $U_1$ 编译期的代码片段对象（不会在当前位置执行它！）；
2. **模式引用 `'<pat>`**：把一个匹配模式打包成编译期模式对象；
3. **`quote` 参数修饰符**：在函数参数或 `where` 子句前加上 `quote`，表示该参数在编译期捕获调用点传入的符号或绑定描述！

```flurry
-- 1. 捕获表达式与模式作为编译期对象
comptime fn build_checked_expr() {
    -- '(x + 1) 不会去计算 x + 1，而是把它打包成 U1 的代码片段！
    let code_piece = '(x + 1);
    let pat_piece = '<Some(v)>;
}
```

### 关键细节：`lift` 与 `quote` 参数如何联手？

当你在函数签名里写 `quote target: ...` 来捕获调用点的一个任意运行时目标时，`target` 原本所处的类型世界是 $U_0$（例如 `Any`），而 `quote` 参数本身工作在 $U_1$ 编译期层。
这时候，**第二座桥 `lift`** 就登场了！`lift expr` 专门用于将运行时类型表示提升（Lift）进编译期类型层：

```flurry
-- quote target 捕获调用点传入的目标描述，
-- lift Any 将运行时顶层类型 Any 提升到编译期引用参数的类型位置！
fn trace_binding(quote target: lift Any, current_val: i32) -> i32 {
    return current_val;
}
```

> **💡 核心心智模型（来自 `AGENTS.md` §5）：带 `quote` 参数的函数不是 `comptime fn`！**
> 这是一个极其重要、绝不能混淆的语义边界：
> - 带有 `quote` 参数的普通函数（有时被通俗称为 quote 函数）**依然是运行时函数，不属于 `comptime fn`！**
> - 当你调用 `trace_binding(my_var, compute_expensive())` 时，它的普通运行时实参 `compute_expensive()` **仍然严格按照普通函数调用规则求值一次（Evaluate Once）**，然后再进入函数体！它绝不会像 C 语言 `#define` 文本宏那样把 `compute_expensive()` 复制粘贴五遍导致重复求值副作用！

---

## 第三座桥：Splice（`~<expr>` 与 `Splice` Trait）——把 $U_1$ 成果落地回 $U_0$

现在，我们在 $U_1$ 编译期宇宙里算好了一段代码，或者算好了一个编译期数据结构。怎么把它送回 $U_0$ 运行时宇宙？
在 Flurry 中，根据你要送回的是**代码片段（Code）**还是**数据值（Value）**，Splice 分为两种形态：

### 形态 A：代码拼接 `~<expr>`（Code Splice）

如果 `expr` 是一个在编译期构造好的代码片段（Quoted Code），使用 **`~<expr>`** 就能在编译期把它无缝拼接到当前的语法位置（表达式位置、模式位置或类型位置）：

```flurry
-- 将编译期生成的代码片段 ~<piece> 拼接到运行时表达式中
fn eval_spliced(x: i32) -> i32 {
    let piece = comptime { '(x * 2i32 + 1i32) };
    -- ~<piece> 在编译期展开为 (x * 2i32 + 1i32)
    return ~<piece>;
}
```

### 形态 B：值拼接 `Splice` Trait（Value Splice）

如果我们不是拼接 AST 代码，而是把一个在编译期算出来的**值**（比如编译期算好的常量、元组或结构体配置 `Config`）落地成运行时变量呢？

在日常写代码时，你经常不知不觉地用到**隐式值 Splice**：
比如你在编译期算出了一个维度或掩码，然后在运行时代码里直接读取它。
但在底层，Flurry 并没有搞黑盒魔法，而是用了一个极其正统的标准库 Trait —— **`Splice`**！

让我们看看 `Splice` Trait 在 `docs/comptime/quoting-and-splicing.md` 中的真实定义：

```flurry
trait Splice {
    -- 该编译期类型落地到运行时宇宙后，对应的运行时目标类型是什么？
    assoc RuntimeTarget: Type;

    -- 注意！splice 本身是一个普通的运行时函数签名，
    -- 它的输入参数带有 comptime 修饰：comptime data: Self！
    fn splice(comptime data: Self) -> RuntimeTarget;
}
```

看到这个签名，你一定会拍案叫绝：
**`fn splice(comptime data: Self) -> RuntimeTarget` 连语法特例都不需要！** 它就是一个接收 `comptime` 参数、返回运行时 `RuntimeTarget` 值的函数！

当一个类型把自己的编译期表示和运行时表示设为同一个类型（即 `RuntimeTarget = Self`）时，我们可以直接实现或自动派生 `Splice`：

```flurry
struct QuantParam {
    scale_num: i32,
    zero_point: i32,
}

impl Splice for QuantParam {
    type RuntimeTarget = Self;

    fn splice(comptime data: Self) -> Self {
        -- 将各个字段从编译期递归 splice 到运行时，组装成运行时结构体！
        return Self {
            scale_num: data.scale_num.splice(),
            zero_point: data.zero_point.splice(),
        };
    }
}
```

### 为什么不是所有编译期值都能随便 `Splice` 到运行时？（有限可物化边界）

试想一下：如果你在编译期构造了一个包含**循环引用指针**的图，或者一个持有**编译期文件句柄/锁（需要 `Drop` 析构）**的对象，甚至是一个还没初始化完毕的半成品结构体，它能直接复制粘贴成目标二进制文件里的运行时静态数据吗？

显然不能！编译期内存和运行时目标机器内存是两个不同的世界。因此，Flurry 对 `Splice` 的自动派生设定了极其严密的**有限可物化（Finite Materialization）安全规则**。以下四类情况在编译期会**直接拒绝自动 `Splice`**：

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：Splice 自动派生的四道安全安检门                       |
|                                                                         |
|  U1 编译期对象 ──► [ 安检门 ] ──► U0 运行时数据                         |
|                                                                         |
|  ❌ 拦截 1：共享引用 / 借用指针（编译期堆地址在运行时毫无意义！）       |
|  ❌ 拦截 2：引用环 / 循环结构（无法展开为有限大小的静态树！）           |
|  ❌ 拦截 3：包含未初始化字段（违反运行时内存初始化安全契约！）          |
|  ❌ 拦截 4：包含实现了 Drop 的资源字段（生命周期与清理责任不可跨界偷渡）|
+-------------------------------------------------------------------------+
```

来看一个编译器当场拦截非法跨宇宙 `Splice` 的负例：

```flurry
-- 假设 TempFile 实现了 Drop，代表编译期打开的临时资源句柄
struct BadComptimeCarrier {
    -- 包含编译期借用引用或 Drop 资源
    borrowed_ref: &i32,
}

fn try_bad_splice() {
    -- ❌ 编译期直接拒绝！
    -- BadComptimeCarrier 包含引用 &i32，编译期内存地址不能跨宇宙直接 Splice
    -- 为运行时指针！无法自动派生或执行 Splice。
    let runtime_val = comptime {
        let local = 10i32;
        BadComptimeCarrier { borrowed_ref: local.ref }
    };
}
```

---

## 本篇小结

| 跨界桥梁 | 方向 | 语法与机制 | 核心安全契约 |
| :--- | :--- | :--- | :--- |
| **Lift（提升）** | $U_0 \rightarrow U_1$（类型层） | `lift expr`（如 `quote target: lift Any`） | 将运行时类型表示提升至编译期类型操作位置 |
| **Quote（引用捕获）** | $U_0 \rightarrow U_1$（代码/符号） | `'(expr)`、`'<pat>`、`quote` 参数 | 只捕获静态语法结构或绑定描述；带 `quote` 参数的普通函数仍对常规实参求值一次 |
| **Code Splice（代码拼接）** | $U_1 \rightarrow U_0$（语法树） | `~<expr>` | 将编译期构造的 Quoted 代码片段展开到表达式、模式或类型位置 |
| **Value Splice（值落地）** | $U_1 \rightarrow U_0$（数据值） | `Splice` trait (`fn splice(comptime data: Self) -> RuntimeTarget`) | 仅允许有限、已完整初始化、无共享引用与无环、无 `Drop` 资源的值自动物化到运行时 |

---

### 留给下一篇的悬念

在刚才研究 `Splice` Trait 的时候，有一句话非常耐人寻味：
`trait Splice { assoc RuntimeTarget: Type; ... }` —— **为什么编译期类型 `Self` 落地到运行时之后的目标类型 `RuntimeTarget`，不一定等于 `Self` 自己？**

这就要引出你在写 Flurry 代码时可能早就好奇过的一个现象了：
当你在代码里写下一串很大的整数字面量计算：
`const TOTAL_BYTES = 1000000 * 1000000 * 1000000;`
你既没有加 `i64` 后缀，也没有加 `u128` 后缀，为什么它在编译期相乘时**永远不会发生 32 位整数溢出**？
在 Flurry 的 $U_1$ 编译期宇宙里，一个不带后缀的整数字面量 `42`，它的真实类型到底叫什么？当它通过 `Splice` 落入 $U_0$ 运行时宇宙时，又是怎么化身为 `i32`、`u8` 或 `usize` 的？

下一篇 **`074-comptime-integer-float-and-runtime-proxies.md`**，我们将揭开编译期无限精度数值类型 **`Integer` / `Float`** 与运行时 **Proxy（代理类型）** 的神秘面纱！
