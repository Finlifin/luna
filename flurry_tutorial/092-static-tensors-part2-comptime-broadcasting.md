# Flurry 演进手记 (092)：静态形状张量实战（二）：编译期形状广播（Broadcasting）推导

> **阅读指南**：在上一篇（第 091 篇）中，我们用 `meta.List<usize>` 定义了 `Tensor<T, ...dims>`，并能静态拦截两个形状不同的张量。但在实际科学计算与 AI 模型里，把一个形状为 `(128, 64)` 的批量特征矩阵加上一个形状为 `(64,)` 的偏置向量（Bias），是常见操作。这就要求类型系统不仅支持形状相等判断，还支持 **张量广播（Broadcasting）**。本篇我们将结合 `docs/comptime/static-shaped-tensors.md`，编写 `comptime predicate broadcastable` 与 `pure comptime fn broadcast_shape`，看看 Flurry 如何在编译期从右向左推导广播形状，并在两阶段检查（定义阶段与实例化阶段）中处理符号维度。

---

## 一、 问题背景：运行期广播与固定重载的局限

**形状广播（Broadcasting）** 允许两个不同形状的张量做逐元素运算（如加法、乘法），且**不需要在物理内存中复制扩展数据**（仅将对应轴的步长 Stride 设为 0 做虚拟复用）。

### 把广播留到运行期，或者用固定的重载穷举

如果不在编译期计算广播形状，通常会遇到两类问题：
1. **运行期广播的问题**：假设本意是把 `(batch, features)` 和 `(batch, 1)` 相加，却误传成了 `(batch,)`（少了一个维度）。如果恰好 `batch == features`，运行期广播不会报错，但会把行广播成列；如果 `batch != features`，则会在运行中途报错。
2. **用固定的函数重载列举广播**：

```flurry
-- 手工为固定维度组合编写函数？
fn add_2d_1d(
    a: Tensor<T, r, c>,
    b: Tensor<T, c>,
) -> Tensor<T, r, c>
where
    T: Type, r: usize, c: usize,
{ ... }

fn add_3d_1d(
    a: Tensor<T, d, r, c>,
    b: Tensor<T, c>,
) -> Tensor<T, d, r, c>
where
    T: Type, d: usize, r: usize, c: usize,
{ ... }

-- 局限：
-- 如果 a 是 (256, 1, 64)，b 是 (1, 32, 64)，结果是 (256, 32, 64)。
-- 任意两根轴都可能有一方是 1、另一方是 n，无法靠固定签名穷举。
```

**核心问题**：
广播是作用在两个形状列表 `s1` 与 `s2` 上的**确定性编译期算法**。我们可以直接用 Flurry 的 `comptime` 函数写出这套算法，并把它嵌入到张量加法函数的类型签名与 `requires` 契约中。

---

## 二、 核心机制：从右向左对齐的四条广播规则

在 `docs/comptime/static-shaped-tensors.md` 中，两个静态形状 `s1` 和 `s2` 的广播遵循**从最右侧（尾轴 / 最低维）向左逐轴对齐**的四条规则：

1. **同维保持（Equal Dimensions）**：如果当前轴两边的长度相等（$d_1 == d_2$），则输出轴长度为 $d_1$；
2. **单位轴扩展（Unit Dimension `1`）**：如果其中一方的长度为 `1`（例如 $d_1 == 1, d_2 == k$），则长度为 `1` 的那一方沿着该轴虚拟复用以匹配另一方，输出轴长度为 $k$；
3. **缺失前导轴虚拟补 `1`（Missing Leading Dimensions）**：如果其中一个张量的阶数（Rank）更低，从右向左对齐时它的前导轴已经用完，则视为在左侧**虚拟补 `1`**（仅参与编译期形状推导与步长映射，**不在运行时分配额外内存**），输出轴直接取高阶张量对应的前导轴长度；
4. **其余情况不相容（Incompatible）**：既不相等、又没有任何一方为 `1`（比如 `64` 和 `32`），在编译期拒绝。

下面展示 `(256, 1, 64)` 与 `(32, 64)` 如何在编译期从右向左推导出 `(256, 32, 64)`：

```text
                编译期从右向左（Right-to-Left）逐轴广播推导
      
                 前导轴 (Axis 0)      中间轴 (Axis 1)      尾轴 (Axis 2)
               ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
   形状 s1:    │       256       │  │        1        │  │       64        │
               └────────┬────────┘  └────────┬────────┘  └────────┬────────┘
                        │                    │                    │
   形状 s2:    ┌────────┴────────┐  ┌────────┴────────┐  ┌────────┴────────┐
   [32, 64]    │ (缺失 ⟹ 虚拟 1) │  │       32        │  │       64        │
               └────────┬────────┘  └────────┬────────┘  └────────┬────────┘
                        │                    │                    │
               ┌────────▼────────┐  ┌────────▼────────┐  ┌────────▼────────┐
   适用规则:   │ 规则 3: 前导补 1│  │ 规则 2: 1 扩 32 │  │ 规则 1: 64 == 64│
               └────────┬────────┘  └────────┬────────┘  └────────┬────────┘
                        ▼                    ▼                    ▼
   结果形状:   [       256       ,          32        ,          64       ]
                        ◀───────────── 从右向左对齐 ──────────────
```

### 编写 `broadcastable` 与 `broadcast_shape`

我们可以写出判定可广播性的编译期谓词 `broadcastable`，以及计算结果形状的纯编译期函数 `broadcast_shape`：

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

-- 判定两个形状列表 s1 和 s2（从右向左对齐）是否满足广播契约
comptime predicate broadcastable(
    s1: meta.List<usize>,
    s2: meta.List<usize>,
) {
    let r1 = s1.len();
    let r2 = s2.len();
    let common = inline if r1 < r2 { r1 } else { r2 }

    -- 从右向左逐一检查公共后缀轴是否全部满足 axis_compatible
    var ok = true;
    inline for offset in 1 .. (common + 1) {
        let d1 = s1[r1 - offset];
        let d2 = s2[r2 - offset];
        ok = ok and axis_compatible(d1, d2);
    }
    ok
}

-- 计算两个可广播形状 s1 和 s2 的最终结果形状
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
    let max_rank = inline if r1 > r2 { r1 } else { r2 }

    var out: meta.List<usize> = [];
    inline for idx in 0 .. max_rank {
        -- 将左起索引转换为距最右端的偏移 offset
        let offset = max_rank - idx;
        let d1 = inline if offset <= r1 { s1[r1 - offset] } else { 1 }
        let d2 = inline if offset <= r2 { s2[r2 - offset] } else { 1 }
        out = out.push_back(broadcast_axis(d1, d2));
    }
    out
}
```

---

## 三、 两阶段检查与编译期广播实战

有了 `broadcastable` 和 `broadcast_shape`，我们可以写出泛型广播加法签名：

```flurry
-- 支持任意阶数、编译期自动推导广播结果类型的张量加法
fn tensor_add(
    lhs: Tensor<T, .dims = s1>,
    rhs: Tensor<T, .dims = s2>,
) -> Tensor<T, .dims = broadcast_shape(s1, s2)>
where
    T: Type,
    s1: refines { valid_shape(itself) } meta.List<usize>,
    s2: refines { valid_shape(itself) } meta.List<usize>,
    requires broadcastable(s1, s2)
{
    ...
}
```

在这个签名下：
- 当传入 `lhs: Tensor<f64, 256, 1, 64>` 和 `rhs: Tensor<f64, 32, 64>` 时：
  1. 编译器提取出 `s1 = [256, 1, 64]`，`s2 = [32, 64]`；
  2. 编译期执行 `broadcastable(s1, s2)`，结果为 `true`（满足 `requires`）；
  3. 返回类型中的 `broadcast_shape(s1, s2)` 在编译期求值为 `[256, 32, 64]`，因此返回类型为 **`Tensor<f64, 256, 32, 64>`**。
- 当把一个矩阵 `Tensor<f64, 128, 64>` 和一个 0 阶标量 `Tensor<f64>`（即 `f64`，形状为 `[]`）相加时，`broadcast_shape([128, 64], [])` 求值为 `[128, 64]`。

> **💡 核心心智模型：两阶段检查如何处理未决符号维度？**
> 如果在另一个泛型函数内部调用 `tensor_add`，此时维度还是符号变量（比如 `Tensor<f64, batch, 1, hidden>` 和 `Tensor<f64, 1, seq_len, hidden>`），编译器尚未拿到具体的数字，检查如何进行？
> 正如 `static-shaped-tensors.md` 所述，Flurry 的检查分为两个阶段：
> 1. **阶段一（类型归一化与定义检查）**：对于符号维度 `(batch, 1, hidden)` 与 `(1, seq_len, hidden)`，不需要知道 `batch`、`seq_len`、`hidden` 的具体数值，凭自反性 `hidden == hidden` 和常量 `1 == 1` 即可在定义阶段证明 `broadcastable` 成立，并化简出符号结果形状 `[batch, seq_len, hidden]`。对于依赖未知维度的具体计算，则等待输入确定后再执行。
> 2. **阶段二（具体实例化）**：当调用点传入具体维度常量时，所有形状表达式求值为具体正整数列表，不相容的形状在此时报错。

当两个张量形状无法广播时，编译器会在编译期拒绝：

```flurry
fn broken_broadcast_call(
    features: Tensor<f64, 128, 64>,
    wrong_bias: Tensor<f64, 32>,
) {
    -- ❌ 编译期拒绝：
    -- 从右向左对齐尾轴时，features 的尾轴是 64，wrong_bias 的尾轴是 32。
    -- 64 != 32 且双方均不为 1，broadcastable([128, 64], [32]) 为 false。
    let _ = tensor_add(features, wrong_bias);
}
```

---

## 本篇小结

| 广播情形（从右向左逐轴对比） | 输入轴示例（$d_1$ vs $d_2$） | 编译期裁决与结果轴长 |
| :--- | :--- | :--- |
| **1. 同维相等** | `64` vs `64` | ✅ 兼容，输出 `64` |
| **2. 单位轴扩展** | `1` vs `32`（或 `32` vs `1`） | ✅ 兼容，单位轴虚拟复用，输出 `32` |
| **3. 缺失前导轴** | `256` vs *(无，阶数较低)* | ✅ 兼容，低阶张量左侧虚拟补 `1`（零内存分配），输出 `256` |
| **4. 既不相等也非 1** | `64` vs `32` | ❌ 编译期拒绝（违反 `requires broadcastable(s1, s2)`） |

---

### 下一篇预告

现在 `Tensor` 已经在类型层面具备了形状与广播推导能力。

进一步考虑运行期寻址开销：
- 形状列表 `dims: meta.List<usize>` 是任意长度的。当在运行期用多维坐标访问元素或执行广播循环时，需要把多维坐标 `(i, j, k)` 乘以每一轴的**步长（Stride）**再求和：$\text{offset} = i \cdot \text{stride}_0 + j \cdot \text{stride}_1 + k \cdot \text{stride}_2$。
- 如何避免在运行期每次访问元素时都用动态循环遍历 `shape` 计算步长，而是在编译期把任意 $n$ 维的步长计算展开为扁平乘加运算？

下一篇 **《093 — 静态形状张量实战（三）：用 `inline for` 消除步长计算开销》** 将结合 `docs/comptime/inlining-comptime-code.md` 介绍编译期展开。
