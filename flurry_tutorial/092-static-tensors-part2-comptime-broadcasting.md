# 092 — 静态形状张量实战（二）：编译期形状广播（Broadcasting）推导

> **阅读指南**：在上一回（第 091 回）中，我们用 `meta.List<usize>` 定义了 `Tensor<T, ...dims>`，并能静态拦截两个形状不同的张量。但在实际科学计算与 AI 模型里，把一个形状为 `(128, 64)` 的批量特征矩阵加上一个形状为 `(64,)` 的偏置向量（Bias），是最天经地义的操作！这就要求类型系统不仅懂“完全相等”，还要懂 **张量广播（Broadcasting）**。本篇我们将结合 `docs/examples/static-shaped-tensors.md`，亲手编写 `comptime predicate broadcastable` 与 `pure comptime fn broadcast_shape`，看看 Flurry 是如何在编译期从右向左推导广播形状，并在两阶段检查（定义阶段 vs. 实例化阶段）中优雅处理符号维度的！

---

## 一、 痛点：广播规则一旦离开类型系统，就会变成“隐式定时炸弹”

熟悉 NumPy 或 PyTorch 的同学都知道，**形状广播（Broadcasting）** 允许两个不同形状的张量做逐元素运算（如加法、乘法），而且**不需要在物理内存中真正复制扩展数据**（仅仅是将对应轴的步长 Stride 设为 0 做虚拟重复）。

### 尝试 `v0`：把广播留到运行期，或者用死板的重载穷举

如果我们不在编译期计算广播形状，会发生什么？
1. **运行期广播的灾难**：有时候你本来想把 `(Batch, Features)` 和 `(Batch, 1)` 相加，结果手滑传成了 `(Batch,)`（少了一个维度）。如果恰好 `Batch == Features`，运行期广播不会报错，但会把行广播成了列，算出完全错误的梯度；如果 `Batch != Features`，则会在训练中途直接 Crash！
2. **尝试用死板的函数重载穷举广播（`v0`）**：

```flurry
-- 尝试 v0：手工为几种固定维度组合写重载？
fn add_2d_1d<T: Type, R: usize, C: usize>(
    a: Tensor<T, R, C>,
    b: Tensor<T, C>,
) -> Tensor<T, R, C> { ... }

fn add_3d_1d<T: Type, D: usize, R: usize, C: usize>(
    a: Tensor<T, D, R, C>,
    b: Tensor<T, C>,
) -> Tensor<T, D, R, C> { ... }

-- ❌ 根本写不完！
-- 如果 a 是 (256, 1, 64)，b 是 (1, 32, 64) 呢？结果是 (256, 32, 64)！
-- 任意两根轴都可能有一方是 1、另一方是 N，组合数是无限的！
```

**核心痛点**：
广播不是几条孤立的特例，而是一套作用在两个形状列表 `S1` 与 `S2` 上的**确定性编译期算法**。我们需要直接用 Flurry 的 `comptime` 函数写出这套算法，并把它嵌入到张量加法函数的类型签名与 `requires` 契约中！

---

## 二、 破局（v1）：从右向左对齐的四条黄金广播法则

在 `docs/examples/static-shaped-tensors.md` 中，两个静态形状 `S1` 和 `S2` 的广播遵循**从最右侧（尾轴 / 最低维）向左逐轴对齐**的四条黄金法则：

1. **同维保持（Equal Dimensions）**：如果当前轴两边的长度相等（$d_1 == d_2$），则输出轴长度为 $d_1$；
2. **单位轴扩展（Unit Dimension `1`）**：如果其中一方的长度为 `1`（例如 $d_1 == 1, d_2 == k$），则长度为 `1` 的那一方沿着该轴虚拟广播以匹配另一方，输出轴长度为 $k$；
3. **缺失前导轴虚拟补 `1`（Missing Leading Dimensions）**：如果其中一个张量的阶数（Rank）更低，从右向左对齐时它的前导轴已经用完了，则视为在左侧**虚拟补 `1`**（注意：这只是编译期形状推导与步长映射，**绝不会在运行时分配额外内存**！），输出轴直接取高阶张量对应的前导轴长度；
4. **其余情况一律冲突（Incompatible）**：既不相等、又没有任何一方为 `1`（比如 `64` 和 `32`），直接在编译期报错拒绝！

让我们用一张 Code Cartoon 来直观展示 `(256, 1, 64)` 与 `(32, 64)` 是如何在编译期从右向左推导出 `(256, 32, 64)` 的：

```text
               编译期从右向左（Right-to-Left）逐轴广播推导
     
                前导轴 (Axis 0)      中间轴 (Axis 1)      尾轴 (Axis 2)
              ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
  形状 S1:    │       256       │  │        1        │  │       64        │
              └────────┬────────┘  └────────┬────────┘  └────────┬────────┘
                       │                    │                    │
  形状 S2:    ┌────────┴────────┐  ┌────────┴────────┐  ┌────────┴────────┐
  #[32, 64]   │ (缺失 ⟹ 虚拟 1) │  │       32        │  │       64        │
              └────────┬────────┘  └────────┬────────┘  └────────┬────────┘
                       │                    │                    │
              ┌────────▼────────┐  ┌────────▼────────┐  ┌────────▼────────┐
  适用法则:   │ 法则 3: 前导补 1│  │ 法则 2: 1 扩 32 │  │ 法则 1: 64 == 64│
              └────────┬────────┘  └────────┬────────┘  └────────┬────────┘
                       ▼                    ▼                    ▼
  结果形状:   #[      256       ,          32        ,          64       ]
                       ◀───────────── 从右向左对齐 ──────────────
```

### 演进到 `v1`：编写 `broadcastable` 与 `broadcast_shape`

因为在 Flurry 中，类型级函数就是普通的 `pure comptime fn`，我们可以直接写出判定可广播性的编译期谓词 `broadcastable`，以及计算结果形状的纯编译期函数 `broadcast_shape`：

```flurry
use std.prelude.*;

-- 判断单根轴上的两个维度 d1 和 d2 是否兼容
comptime predicate axis_compatible(d1: usize, d2: usize) {
    d1 == d2 or d1 == 1 or d2 == 1
}

-- 计算单根轴广播后的输出维度
pure comptime fn broadcast_axis(d1: usize, d2: usize) -> usize
where
    requires axis_compatible(d1, d2)
{
    inline if d1 == 1 { d2 } else { d1 }
}

-- 判定两个形状列表 S1 和 S2（从右向左对齐）是否满足广播契约
comptime predicate broadcastable(
    s1: meta.List<usize>,
    s2: meta.List<usize>,
) {
    let r1 = s1.len();
    let r2 = s2.len();
    let common = inline if r1 < r2 { r1 } else { r2 };

    -- 从右向左逐一检查公共后缀轴是否全部满足 axis_compatible
    var ok = true;
    inline for offset in 1 .. (common + 1) {
        let d1 = s1[r1 - offset];
        let d2 = s2[r2 - offset];
        ok = ok and axis_compatible(d1, d2);
    }
    ok
}

-- 计算两个可广播形状 S1 和 S2 的最终结果形状
pure comptime fn broadcast_shape(
    s1: meta.List<usize>,
    s2: meta.List<usize>,
) -> result: meta.List<usize>
where
    requires valid_shape(s1) and valid_shape(s2),
    requires broadcastable(s1, s2),
    ensures valid_shape(result)
{
    let r1 = s1.len();
    let r2 = s2.len();
    let max_rank = inline if r1 > r2 { r1 } else { r2 };

    var out: meta.List<usize> = #[];
    inline for idx in 0 .. max_rank {
        -- 将左起索引转换为距最右端的偏移 offset
        let offset = max_rank - idx;
        let d1 = inline if offset <= r1 { s1[r1 - offset] } else { 1 };
        let d2 = inline if offset <= r2 { s2[r2 - offset] } else { 1 };
        out = out.push_back(broadcast_axis(d1, d2));
    }
    out
}
```

---

## 三、 两阶段检查与编译期广播实战（v2）

有了 `broadcastable` 和 `broadcast_shape`，我们现在可以写出梦寐以求的泛型广播加法签名了！

```flurry
-- 演进到 v2：支持任意阶数、编译期自动推导广播结果类型的张量加法
fn tensor_add<
    T: Type,
    S1: meta.List<usize> refines valid_shape(S1),
    S2: meta.List<usize> refines valid_shape(S2),
>(
    lhs: Tensor<T, .dims = S1>,
    rhs: Tensor<T, .dims = S2>,
) -> Tensor<T, .dims = broadcast_shape(S1, S2)>
where
    requires broadcastable(S1, S2)
{
    ...
}
```

仔细品味这个签名：
- 当你传入 `lhs: Tensor<f64, 256, 1, 64>` 和 `rhs: Tensor<f64, 32, 64>` 时：
  1. 编译器提取出 `S1 = #[256, 1, 64]`，`S2 = #[32, 64]`；
  2. PTC 调用 `PRED` 求解器执行 `broadcastable(S1, S2)`，结果为 `true`（满足 `requires`！）；
  3. 返回类型中的 `broadcast_shape(S1, S2)` 在编译期直接归一化为 `#[256, 32, 64]`，所以返回类型自动变成 **`Tensor<f64, 256, 32, 64>`**！
- 甚至当你把一个矩阵 `Tensor<f64, 128, 64>` 和一个 0 阶标量 `Tensor<f64>`（即 `f64`，形状为 `#[]`）相加时，`broadcast_shape(#[128, 64], #[])` 也会自动算出 `#[128, 64]`！

> **💡 核心心智模型：两阶段检查（Two-Phase Checking）如何处理未决符号维度？**
> 如果我们在另一个泛型函数内部调用 `tensor_add`，此时维度还是符号变量（比如 `Tensor<f64, Batch, 1, Hidden>` 和 `Tensor<f64, 1, SeqLen, Hidden>`），编译器还没拿到具体的数字，能检查通过吗？
> **完全可以！** 正如 `static-shaped-tensors.md` 所描述的，Flurry 的检查分为两个阶段：
> 1. **阶段一（类型归一化与定义检查）**：对于符号维度 `(Batch, 1, Hidden)` 与 `(1, SeqLen, Hidden)`，PTC 的 `INT` 与 `EUF` 求解器不需要知道 `Batch`、`SeqLen`、`Hidden` 的具体数值，光凭自反性 `Hidden == Hidden` 和常量 `1 == 1`，就能在定义阶段直接证明 `broadcastable` 成立，并化简出符号结果形状 `#[Batch, SeqLen, Hidden]`！如果遇到定义阶段暂未决断的松散（`loose`）约束，则将其作为残余约束冻结。
> 2. **阶段二（单态化 / 具体实例化）**：当调用点传入具体维度常量时，所有残余形状表达式被彻底求值为具体正整数列表，任何隐藏的维度冲突都会在此时被清零捕获！

让我们看看当两个张量形状无法广播时，编译器是如何当场拒绝的：

```flurry
fn broken_broadcast_call(
    features: Tensor<f64, 128, 64>,
    wrong_bias: Tensor<f64, 32>,
) {
    -- ❌ 编译期直接拒绝！
    -- 从右向左对齐尾轴时：features 的尾轴是 64，wrong_bias 的尾轴是 32。
    -- 64 != 32 且双方均不为 1，broadcastable(#[128, 64], #[32]) 为 false！
    -- 编译器直接指出尾轴 64 与 32 无法广播！
    let _ = tensor_add(features, wrong_bias);
}
```

---

## 本篇小结

| 广播情形（从右向左逐轴对比） | 输入轴示例（$d_1$ vs $d_2$） | 编译期裁决与结果轴长 |
| :--- | :--- | :--- |
| **1. 同维相等** | `64` vs `64` | ✅ 兼容，输出 `64` |
| **2. 单位轴扩展** | `1` vs `32`（或 `32` vs `1`） | ✅ 兼容，单位轴虚拟广播，输出 `32` |
| **3. 缺失前导轴** | `256` vs *(无，阶数较低)* | ✅ 兼容，低阶张量左侧虚拟补 `1`（零内存分配），输出 `256` |
| **4. 既不相等也非 1** | `64` vs `32` | ❌ 编译期直接拒绝（违反 `requires broadcastable(S1, S2)`） |

---

### 留给下一篇的悬念

太酷了！现在我们的 `Tensor` 已经在类型层面拥有了堪比定理证明器的形状与广播推导能力。

但是，作为一个追求极致性能的系统程序员，你心里一定还有一个顾虑：
- 形状列表 `dims: meta.List<usize>` 是任意长度的。当我们在运行期用多维坐标 `tensor[i, j, k]` 访问一个元素，或者执行广播加法循环时，需要把多维坐标 `(i, j, k)` 乘以每一轴的**步长（Stride）**再求和：$\text{offset} = i \cdot \text{stride}_0 + j \cdot \text{stride}_1 + k \cdot \text{stride}_2$。
- 如果在运行期用一个 `for` 循环去遍历 `shape` 数组算步长，每次内存访问岂不是都要多跑一层动态循环？我们怎样才能让编译器把任意 $N$ 维的步长计算，在编译期**100% 展开（Unroll）成零开销的扁平乘加指令**？

下一篇 **《093 — 静态形状张量实战（三）：用 `inline for` 消除步长计算开销》**，我们将结合 `docs/comptime/inlining-comptime-code.md`，解锁编译期展开的终极威力！
