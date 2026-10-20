# Flurry 演进手记 (194)：终极实战第四层（元编程层）：用 2LTT 与 CMTT 在编译期根据滤波器阶数生成展开的卷积代码

在前面三篇手记中，我们的 **Orion 异构流式信号处理引擎** 已经打通了底层数据通路：
- **Layer 1（协议层）**：用跨元素 `BitVec` 枚举完成了零拷贝电报帧解码；
- **Layer 2（内存层）**：用双层 `&reachability` 与代际身份构建了分片环形队列；
- **Layer 3（验证层）**：用 `verified fn` 与 `outcomes` 证明了分片槽位读写的分离性与无溢出。

现在，采样数据已经稳稳躺在环形队列的滑动窗口 `window: Array<f32, n>` 中。接下来是 **Layer 4（元编程与算法生成层）**：我们需要对每个通道的采样窗口执行 **FIR（有限冲激响应，Finite Impulse Response）卷积滤波**：
$$y[\text{idx}] = \sum_{k=0}^{K-1} h[k] \cdot x[\text{idx} - k]$$

```
+=============================================================================+
|              Orion Signal Engine —— Layer 4: CMTT 编译期代码生成层            |
+=============================================================================+
| Layer 3: 已验证无越界的采样窗口 window: Array<f32, n> 与游标 cursor: usize    |
|                                  |                                          |
|                                  v                                          |
| Layer 4: pure comptime fn build_fir_expr (2LTT + CMTT 模态代码构造)  <-- 本篇 |
|          +-- 输入: 编译期抽头系数 taps: Slice<f32> (含稀疏零抽头)             |
|          +-- 过程: 单孔链式构造 (Single-Hole Chain) 自动跳过 coeff == 0.0     |
|          +-- 输出: < samples: Array<f32, n>, idx: usize |- f32 >             |
+=============================================================================+
```

如果在运行时用一个普通的 `for` 循环去遍历滤波器的抽头系数数组 `taps`：
1. 每次采样都要承受循环计数器自增、分支跳转与系数内存读取的开销；
2. 许多工业级半带滤波器（Half-band Filter）里有一半以上的抽头系数恒等于 `0.0`（或者恒等于 `1.0`），运行时循环仍然会傻傻地执行乘零与加零指令。

在 C++ 里，你可能会去写递归模板特化；在传统宏系统里，你会去拼接繁琐的字符串或无类型的 Token 树，一旦写错变量名就要面对几百行展开后的天书报错。

而在 Flurry 中，我们拥有 **两阶段类型论（2LTT）与上下文模态类型论（CMTT，见第四卷第 066～085 篇）**。今天，我们就用带显式上下文的盒模态类型 `< Γ |- T >` 与单孔链式拼接，在编译期把任意稀疏抽头数组直接编译成一段**零循环、零乘零、百分之百类型安全**的展开卷积表达式。

---

## 一、 核心设计原则：为什么 CMTT 代码生成必须遵守“单孔链式构造”？

回顾我们在第 072 篇与 `docs/cmtt/program-construction.md` 中确立的核心准则：
在编写 `pure comptime fn` 递归或循环构造代码片段时，我们要把状态累积在一段携带自由变量上下文 $\Gamma$ 的闭合盒类型 `< Γ |- T >` 中。

对于 Orion 的 FIR 滤波器，生成的目标表达式需要在运行时访问两个外部自由变量：
- `samples: Array<f32, n>`：当前通道的采样滑动窗口；
- `idx: usize`：当前正在计算的中心采样点下标。

因此，我们的编译期代码生成器返回的类型签名是：
```flurry
< samples: Array<f32, n>, idx: usize |- f32 >
```

### 警惕 CMTT 构造反模式：双孔拼接导致语法树指数级膨胀

很多刚接触模态元编程的开发者，在写编译期展开时容易写出这样的“双孔分支引号”：
`'(if cond { ~<branch_a> } else { ~<branch_b> })` 或者在单次引号里同时把 `~<acc>` 填进两个孔里。
请记住 `docs/cmtt/program-construction.md` 中的核心规则：
> **永远在 `comptime` 宿主控制流中先做分支判断，再用单孔引号（Single-Hole Quote）扩展前一步的 `acc_code`，绝不要把已经构造好的代码片段在同一个引号里重复拼接两次，也绝不要指望后端优化器替你裁剪死分支代码。**

```
+-----------------------------------------------------------------------------+
|            CMTT 单孔链式构造 (Single-Hole Chain Construction)                |
+-----------------------------------------------------------------------------+
| 初始状态 (Step 0):                                                          |
|   acc_code = '(0.0)                                                         |
|                                                                             |
| 第 1 抽头 (tap = 0, coeff = 0.25 != 0.0) ---> 命中非零分支，单孔包裹 previous:|
|   let previous = acc_code;                                                |
|   acc_code = '(~<previous> + samples[idx - 0] * 0.25)                       |
|                                                                             |
| 第 2 抽头 (tap = 1, coeff = 0.0)         ---> 编译期直接跳过  零语法树节点    |
|                                                                             |
| 第 3 抽头 (tap = 2, coeff = 0.50 != 0.0) ---> 继续单孔包裹 previous:         |
|   let previous = acc_code;                                                |
|   acc_code = '(~<previous> + samples[idx - 2] * 0.50)                       |
+-----------------------------------------------------------------------------+
```

---

## 二、 实战第一步：用 `pure comptime fn` 实现稀疏感知的 FIR 代码生成器

让我们写出 Orion Layer 4 的核心编译期生成器 `build_sparse_fir_kernel`。
它接收窗口大小 `n` 和编译期已知的抽头系数数组 `taps: Array<f32, k>`，在编译期遍历每一个抽头：
- 如果 `coeff == 0.0`，编译期直接 `continue`，连一个加法节点都不生成；
- 如果 `coeff == 1.0`，编译期省去乘法，只生成 `+ samples[idx - tap]`；
- 否则，生成标准的乘累加项 `+ samples[idx - tap] * coeff`。

```flurry
-- 在编译期根据抽头系数数组 taps 生成完全展开的 FIR 卷积表达式代码
pure comptime fn build_sparse_fir_kernel(
    n: usize,
    taps: Array<f32, k>,
) -> < samples: Array<f32, n>, idx: usize |- f32 >
where
    k: usize,
{
    -- 初始累加器代码片段：常数 0.0，在上下文 < samples, idx > 下合法
    var acc_code: < samples: Array<f32, n>, idx: usize |- f32 > = '(0.0);

    var tap: usize = 0;
    while tap < k {
        let coeff = taps[tap];
        let tap_offset = tap;

        -- 在编译期（Stage U1）直接裁剪掉零系数与单位系数。
        if coeff == 0.0 {
            -- 稀疏零抽头：什么都不生成。
        } else if coeff == 1.0 {
            let previous = acc_code;
            acc_code = '(~<previous> + samples[idx - tap_offset]);
        } else {
            let previous = acc_code;
            acc_code = '(~<previous> + samples[idx - tap_offset] * coeff);
        }

        tap += 1;
    }

    acc_code
}
```

仔细品味这段代码里的两阶段（2LTT）交织之美：
1. `while tap < k` 和 `if coeff == 0.0` 全部运行在 **编译期元语言阶段（Stage $U_1$）**；
2. `tap_offset` 和 `coeff` 是编译期算出的标量值，当它们出现在引号 `'(` 内部时，通过**跨阶段持久化（Cross-Stage Persistence）** 直接物化为目标代码里的字面量常数（如 `0`、`2`、`0.25`）；
3. `let previous = acc_code; acc_code = '(~<previous> + ...)` 严格遵循**单孔链式构造**，每一步只把前一步的语法树填进唯一的一个孔 `~<previous>` 中，语法树大小与非零抽头个数严格呈 $O(K_{\text{nonzero}})$ 线性关系。

---

## 三、 实战第二步：用显式上下文代换 `~[...]<code_box>` 实例化滤波器

生成了类型为 `< samples: Array<f32, n>, idx: usize |- f32 >` 的开放代码盒（Open Code Box）之后，我们怎么在运行时的信号处理函数 `apply_halfband_filter` 里调用它？

在第 070 篇与 `docs/cmtt/contexts-and-substitution.md` 中我们学过：当被拼接的代码盒携带非空上下文 `< samples: ..., idx: ... |- f32 >` 时，拼接点必须通过 **显式上下文环境代换（Explicit Context Substitution）** `~[samples = window, idx = cursor]<fir_code>`，把调用点作用域里的真实运行期变量绑定到代码盒要求的自由变量名上。

```flurry
-- 定义一个 5 阶稀疏半带滤波器系数（其中第 1、第 3 抽头为 0.0，中心抽头为 1.0）
const ORION_HALFBAND_TAPS: Array<f32, 5> = [0.25, 0.0, 1.0, 0.0, 0.25];

-- 在运行时安全函数中拼接编译期生成的展开卷积核
pure fn eval_orion_fir_point(
    window: Array<f32, 64>,
    cursor: usize,
) -> f32
where
    requires cursor >= 4 and cursor < 64,
{
    -- 1. 编译期调用生成器，得到 < samples: Array<f32, 64>, idx: usize |- f32 >
    const fir_code = build_sparse_fir_kernel(64, ORION_HALFBAND_TAPS);

    -- 2. 通过显式代换列表 [samples = window, idx = cursor] 将开放代码盒内联拼接到此处。
    -- 展开后等价于：
    -- (((0.0 + window[cursor - 0] * 0.25) + window[cursor - 2]) + window[cursor - 4] * 0.25)
    ~[samples = window, idx = cursor]<fir_code>
}
```

看看展开后的实际效果：
原本 5 个抽头的循环，在编译期被自动削去了 2 个零抽头（`tap = 1` 和 `tap = 3`），省去了中心抽头（`tap = 2`）的乘 `1.0` 运算，最终内联进 `eval_orion_fir_point` 的只有 **2 次乘法和 3 次加法**。没有任何循环开销，也没有任何卫生的名字冲突风险。

---

## 四、 编译器如何拦截缺失的上下文代换？（负向示例）

在传统 C 宏里，如果宏体偷偷引用了一个叫 `idx` 的外部变量，而调用宏的函数里恰好没有定义 `idx`（或者定义了一个类型完全不对的 `idx: String`），你只有在宏展开后的几百行报错栈里慢慢找原因。

而在 Flurry 的 CMTT 中，上下文 `< samples: Array<f32, 64>, idx: usize |- f32 >` 是类型签名的一部分。如果拼接点漏传了 `idx`，或者传错了类型，编译器会在拼接点静态拦截：

```flurry
-- ❌ 拒绝例：拼接开放代码盒时遗漏了上下文要求的自由变量 `idx`
pure fn broken_fir_eval(window: Array<f32, 64>) -> f32 {
    const fir_code = build_sparse_fir_kernel(64, ORION_HALFBAND_TAPS);

    -- 编译错误：Incomplete context substitution at splice `~[...]<fir_code>`:
    -- code box requires context `< samples: Array<f32, 64>, idx: usize >`,
    -- but variable `idx: usize` was neither provided in the substitution map
    -- nor available in the lexical environment.
    ~[samples = window]<fir_code>
}
```

---

## 五、 本层通关小结与下一层预告

在 **Layer 4（元编程层）** 中，我们用 2LTT 与 CMTT 为 Orion 引擎打造了编译期定制化的 FIR 卷积核生成器：
- 用显式上下文盒类型 `< samples: Array<f32, n>, idx: usize |- f32 >` 严格声明了生成代码的自由变量依赖；
- 用 `pure comptime fn` 配合**单孔链式构造**，在编译期自动消除了稀疏零抽头与单位乘法；
- 用显式上下文代换 `~[samples = window, idx = cursor]<fir_code>` 安全地将生成代码缝合进运行时函数。

不过，目前 `eval_orion_fir_point` 还是在单核 CPU 上一次计算一个采样点。当 Orion 引擎面对 64 通道并发的宽带射电阵列时，我们需要把这颗卷积内核直接投射到硬件 SIMD 乘累加指令（FMA）与 GPU 异构计算管线上。

在下一篇 **第 195 篇（Layer 5：硬件加速层）** 中，我们将使用 Flurry 的 **`^intrinsic_enum`、`'intrinsic` 视图与 `^cuda.global` 管线投射（Pipeline Projection）**，实现 CPU/GPU 同源的硬件加速滤波内核。
