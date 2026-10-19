# 091 — 静态形状张量实战（一）：用类型级列表编码维度与秩

> **阅读指南**：欢迎来到第四卷的实战高潮——**静态形状张量（Static-Shaped Tensors）三部曲**的第一篇！在科学计算、图形学与深度学习系统中，“张量形状（Shape）不匹配”是头号 Bug 来源。本篇我们将结合 `docs/examples/static-shaped-tensors.md`，动手设计 `Tensor` 类型构造器：如何用编译期列表 `meta.List<usize>` 与变参语法同时支持 `Tensor<f64, 128, 128>` 和显式形状列表 `Tensor<T, .dims = S>`？如何用精化契约拦截 `0` 维轴？以及最神奇的一点：**为什么一个 0 阶（Rank-0）张量 `Tensor<T>` 在类型系统里可以直接等同于标量类型 `T`？**

---

## 一、 痛点：运行期才爆炸的 `ShapeError` 与死板的 `Matrix2D`

假设我们要在系统里处理一维向量、二维矩阵、三维图像纹理和四维神经网络批次特征图。

### 尝试 `v0`：要么放弃静态检查，要么为每个维度手写一个新结构体

在传统静态语言里，我们往往面临两难抉择：

1. **路线 A（动态形状 `DynamicTensor<T>`）**：把形状 `dims: Vec<usize>` 塞进运行期结构体字段里。结果编译器对维度一无所知，把 `128 × 64` 的矩阵和 `128 × 32` 的矩阵相加也能顺利编译通过，直到上线运行才崩溃！
2. **路线 B（按阶数硬编码 `Vector<T, N>`、`Matrix<T, R, C>`、`Tensor3D<T, D, H, W>`）**：

```flurry
-- 尝试 v0：为每一个阶数（Rank）重复发明轮子
struct Vector<T: Type, N: usize> { ... }
struct Matrix<T: Type, Rows: usize, Cols: usize> { ... }
struct Tensor3D<T: Type, D: usize, H: usize, W: usize> { ... }

-- 灾难接踵而至：
-- 1. 5 维、6 维张量怎么办？难道写到 Tensor8D 吗？
-- 2. 逐元素加法 add、广播计算、转置函数怎么写？难道每个结构体都要复制粘贴一遍吗？
-- 3. 没人拦得住用户写出 Matrix<f64, 0, 128> 这种带有 0 长度退化轴的怪胎！
```

**核心痛点**：
张量的阶数（Rank，即维度的个数）可以是任意非负整数 $0, 1, 2, 3, \dots$。我们需要一个**统一的、变参的编译期类型构造器 `Tensor`**，它既能让用户像写 `Tensor<f64, 128, 128>` 那样直观地列出每一轴的长度，又能在泛型函数里把整个形状当成一个完整的编译期列表 `S: meta.List<usize>` 来整体传递和推理！

---

## 二、 破局（v1）：用 `meta.List<usize>` 与精化谓词定义静态形状

在 Flurry 中，因为类型构造器本身就是返回 `Type` 的 `pure comptime fn`，我们可以直接用 **编译期列表 `meta.List<usize>`** 来表示张量的形状（Shape）！

首先，什么样的整数列表才配做一个合法的静态张量形状？
在 `static-shaped-tensors.md` 中，静态形状被严格定义为**由正整数组成的有限列表**：每一根轴的维度 $d$ 都必须满足 $d > 0$！让我们先定义一个编译期谓词 `valid_shape`：

```flurry
-- 检查列表中的每一根轴维度是否都严格大于 0
comptime predicate valid_shape(shape: meta.List<usize>) {
    shape.all(fn(d: usize) -> bool { d > 0 })
}
```

接下来，我们如何让用户既能舒舒服服地写变参 `Tensor<f64, 128, 128>`，又能在泛型签名里直接把一个现成的列表变量 `S` 传进去（比如 `Tensor<T, .dims = S>`）？

还记得我们在第一卷第 024 回学过的 **变参自动收集与 `.param = list` 直接供给语法** 吗？当参数声明为变参 `...dims: meta.List<usize>` 时：
- 用户直接传 `Tensor<f64, 128, 128>`，编译器会自动把 `128, 128` 打包成列表 `#[128, 128]` 喂给 `dims`；
- 而在泛型代码里，我们手里已经有一个列表 `S: meta.List<usize>` 时，只需写 `Tensor<T, .dims = S>`，就能跳过打包、直接把列表 `S` 赋给 `dims`！

### 演进到 `v1`：支持变参维度与 Rank-0 标量退化的 `Tensor`

更妙的是，如果一个张量的形状是空列表 `#[]`（即 **0 阶张量，Rank-0 Tensor**），它在数学上就是一个单一的标量（Scalar）。在 Flurry 中，我们可以在类型构造器的后置契约与分支里直接声明：**当 `dims` 为空时，`Tensor<T>` 就是 `T` 本身！**

```flurry
use std.prelude.*;

comptime predicate valid_shape(shape: meta.List<usize>) {
    shape.all(fn(d: usize) -> bool { d > 0 })
}

-- 演进到 v1：统一的静态形状张量类型构造器
pure comptime fn Tensor<
    T: Type,
    ...dims: meta.List<usize> refines valid_shape(dims),
> -> result: Type
where
    ensures dims.is_empty() implies result == T
{
    inline if dims.is_empty() {
        -- 0 阶张量直接退化为标量类型 T！
        T
    } else {
        -- 非空形状：构造持有连续元素存储的张量结构类型
        struct {
            const element_type: Type = T;
            const shape: meta.List<usize> = dims;
            const rank: usize = dims.len();
        }
    }
}
```

让我们画一张 Code Cartoon，看看不同维度参数下的 `Tensor` 在类型宇宙中是如何呈现的：

```text
               调用 pure comptime fn Tensor<T, ...dims> -> Type
                                      │
          ┌───────────────────────────┼───────────────────────────┐
          ▼                           ▼                           ▼
┌───────────────────┐       ┌───────────────────┐       ┌───────────────────┐
│   Tensor<f64>     │       │ Tensor<f64, 128>  │       │Tensor<f64, 64, 32>│
│  (dims = #[])     │       │ (dims = #[128])   │       │(dims = #[64, 32]) │
└─────────┬─────────┘       └─────────┬─────────┘       └─────────┬─────────┘
          │                           │                           │
          ▼                           ▼                           ▼
┌───────────────────┐       ┌───────────────────┐       ┌───────────────────┐
│ 触发空列表退化:   │       │ Normal-Form 类型头│       │ Normal-Form 类型头│
│   result == f64   │       │ Tensor<f64, .dims │       │ Tensor<f64, .dims │
│                   │       │   = #[128]>       │       │   = #[64, 32]>    │
│ 0 阶张量与标量 f64│       │ 1 阶向量 (Rank 1) │       │ 2 阶矩阵 (Rank 2) │
│ 完全等价、无缝互通│       │ 轴长 128          │       │ 形状 64 × 32      │
└───────────────────┘       └───────────────────┘       └───────────────────┘
```

> **💡 核心心智模型：Normal-Form 类型头与 Rank-0 等价**
> 1. **非空形状保留 Normal-Form 类型身份**：当 `dims` 非空时，`Tensor<f64, 128, 128>` 与 `Tensor<f64, .dims = #[128, 128]>` 归一化到同一个 Normal-Form 类型头。只有当元素类型 `T` 相等、且形状序列在 `SEQ` 与 `INT` 求解器下逐轴相等时，两个张量类型才相等！
> 2. **Rank-0 张量与标量等价**：得益于 `ensures dims.is_empty() implies result == T`，PTC 求解器知道 `Tensor<f64>`（即 `Tensor<f64, .dims = #[]>`）严格等于 `f64`。这意味着你在写泛化到任意阶数的张量算法时，完全不需要为标量单独写一套重载版本！

---

## 三、 严格边界：同形契约保护与非法形状拦截（v2）

有了 `Tensor<T, ...dims>`，我们现在可以写出只有在形状完全一致时才允许调用的安全张量操作，同时让编译器自动拦截任何非法的 `0` 维轴或形状错配！

### 演进到 `v2`：编译期 100% 免疫形状错配

```flurry
use std.prelude.*;

-- 要求两个输入张量必须拥有完全相同的元素类型 T 和完全相同的形状 S
fn assert_same_shape<
    T: Type,
    S: meta.List<usize> refines valid_shape(S),
>(
    lhs: Tensor<T, .dims = S>,
    rhs: Tensor<T, .dims = S>,
) {}

comptime {
    -- ✅ 1. 变参写法与 .dims = 显式列表写法表示完全相同的类型！
    assumes Tensor<f64, 128, 64> == Tensor<f64, .dims = #[128, 64]>;

    -- ✅ 2. 0 阶张量直接等于标量类型 f64！
    assumes Tensor<f64> == f64;

    -- ✅ 3. 哪怕维度相同，只要顺序反了（128×64 vs 64×128），也是截然不同的类型！
    assumes Tensor<f64, 128, 64> != Tensor<f64, 64, 128>;
}

fn broken_tensor_shapes(
    a: Tensor<f64, 128, 64>,
    b: Tensor<f64, 128, 32>,
) {
    -- 负例 1：把 128×64 的张量和 128×32 的张量传给要求同形的函数
    -- ❌ 编译期直接拒绝！
    -- SEQ 与 INT 求解器立即发现第 1 轴维度 64 != 32，
    -- 无法合一为同一个形状列表 S！
    assert_same_shape(a, b);

    -- 负例 2：试图构造包含 0 长度轴的退化张量
    -- ❌ 编译期直接拒绝！
    -- valid_shape(#[128, 0]) 求值为 false，违反了 dims 参数的 refines 契约！
    let _: Tensor<f64, 128, 0> = undef;
}
```

看看上面这两个报错：不需要跑一行 GPU 内核，甚至不需要进入运行期，仅仅在你写下 `assert_same_shape(a, b)` 或 `Tensor<f64, 128, 0>` 的那一刻，PTC 的 `SEQ`、`INT` 和 `PRED` 求解器就已经联手把 Bug 扼杀在了摇篮里！

---

## 本篇小结

| 特性维度 | Flurry 设计方案 | 解决的痛点与收益 |
| :--- | :--- | :--- |
| **形状表示（Shape）** | `...dims: meta.List<usize>` | 用一套类型构造器统一表达从 0 维到任意 $N$ 维的张量 |
| **双重调用人体工程学** | `Tensor<f64, 128, 64>` 与 `Tensor<T, .dims = S>` | 具体代码手写变参直观清爽，泛型代码直接透传形状列表 `S` |
| **合法轴长精化** | `refines valid_shape(dims)`（要求每轴 $d > 0$） | 在编译期杜绝 `Tensor<f64, 128, 0>` 这类退化零轴形状 |
| **0 阶张量退化** | `Tensor<T>`（空形状 `#[]`）`== T` | 标量天然就是 Rank-0 张量，无需为标量与张量维护两套平行接口 |

---

### 留给下一篇的悬念

我们已经成功让编译器认识了 `Tensor<T, .dims = S>`，并且能够强制要求两个张量“形状一模一样（`S == S`）”。

但在真实的深度学习与科学计算中，我们经常需要把一个形状为 `(128, 64)` 的矩阵和一个形状为 `(64,)` 的偏置向量（Bias Vector）相加，或者把 `(256, 1, 64)` 和 `(1, 32, 64)` 的张量相加！这就是著名的 **张量广播（Broadcasting）**。

如果允许广播，那两个输入张量的形状列表 `S1` 和 `S2` 就不相等了！
- 编译器怎么在**编译期**判断 `S1` 和 `S2` 能不能合法广播？
- 广播之后产生的新张量，它的结果形状 `broadcast_shape(S1, S2)` 又是怎么在类型签名里自动算出来的？

下一篇 **《092 — 静态形状张量实战（二）：编译期形状广播（Broadcasting）推导》**，我们将用纯编译期函数把 NumPy / PyTorch 的广播规则完整搬进 Flurry 的类型检查器！
