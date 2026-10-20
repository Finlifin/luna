# Flurry 演进手记 (091)：静态形状张量实战（一）：用类型级列表编码维度与秩

> **阅读指南**：欢迎来到第四卷的静态形状张量（Static-Shaped Tensors）专题第一篇。在科学计算、图形学与深度学习系统中，“张量形状（Shape）不匹配”是常见错误来源。本篇我们将结合 `docs/comptime/static-shaped-tensors.md`，设计 `Tensor` 类型构造器：如何用编译期列表 `meta.List<usize>` 与变参语法同时支持 `Tensor<f64, 128, 128>` 和显式形状列表 `Tensor<T, .dims = s>`？如何用精化契约拦截 `0` 维轴？以及为什么一个 0 阶（Rank-0）张量 `Tensor<T>` 在类型系统里可以直接等同于标量类型 `T`？

---

## 一、 问题背景：运行期才暴露的 `ShapeError` 与固定的 `Matrix2D`

假设我们要在系统里处理一维向量、二维矩阵、三维图像纹理和四维神经网络批次特征图。

### 要么放弃静态检查，要么为每个维度手写一个新结构体

在传统静态语言里，我们往往面临两难抉择：

1. **路线 A（动态形状 `DynamicTensor<T>`）**：把形状 `dims: Vec<usize>` 塞进运行期结构体字段里。结果编译器对维度一无所知，把 `128 × 64` 的矩阵和 `128 × 32` 的矩阵相加也能编译通过，直到运行时才报错。
2. **路线 B（按阶数硬编码 `Vector<T, n>`、`Matrix<T, rows, cols>`、`Tensor3D<T, d, h, w>`）**：

```flurry
-- 为每一个阶数（Rank）重复定义结构体
struct Vector where T: Type, n: usize { ... }
struct Matrix where T: Type, rows: usize, cols: usize { ... }
struct Tensor3D where T: Type, d: usize, h: usize, w: usize { ... }

-- 问题：
-- 1. 5 维、6 维张量需要继续定义新的结构体；
-- 2. 逐元素加法 add、广播计算、转置函数需要在每个结构体上重复实现；
-- 3. 难以统一拦截 Matrix<f64, 0, 128> 这种带有 0 长度退化轴的非法形式。
```

**核心问题**：
张量的阶数（Rank，即维度的个数）可以是任意非负整数 $0, 1, 2, 3, \dots$。我们需要一个**统一的、变参的编译期类型构造器 `Tensor`**，它既能让用户像写 `Tensor<f64, 128, 128>` 那样直观地列出每一轴的长度，又能在泛型函数里把整个形状当成一个完整的编译期列表 `s: meta.List<usize>` 来整体传递和推理。

---

## 二、 核心机制：用 `meta.List<usize>` 与精化谓词定义静态形状

在 Flurry 中，因为类型构造器本身就是返回 `Type` 的 `pure comptime fn`，我们可以直接用 **编译期列表 `meta.List<usize>`** 来表示张量的形状（Shape）。

首先，什么样的整数列表才是一个合法的静态张量形状？
在 `static-shaped-tensors.md` 中，静态形状被定义为**由正整数组成的有限列表**：每一根轴的维度 $d$ 都必须满足 $d > 0$。我们先定义一个编译期谓词 `valid_shape`：

```flurry
-- 检查列表中的每一根轴维度是否都严格大于 0
comptime predicate valid_shape(shape: meta.List<usize>) {
    shape.all(|d: usize| -> bool d > 0)
}
```

接下来，我们如何让用户既能写变参 `Tensor<f64, 128, 128>`，又能在泛型签名里直接把一个现成的列表变量 `s` 传进去（比如 `Tensor<T, .dims = s>`）？

根据第一卷第 024 篇介绍的 **变参自动收集与 `.param = list` 直接供应语法**，当参数声明为变参 `...dims: meta.List<usize>` 时：
- 用户直接传 `Tensor<f64, 128, 128>`，编译器会自动把 `128, 128` 收集为列表 `[128, 128]` 绑定给 `dims`；
- 而在泛型代码里，已经有一个列表 `s: meta.List<usize>` 时，只需写 `Tensor<T, .dims = s>`，就能直接把列表 `s` 供应给 `dims`。

### 支持变参维度与 Rank-0 标量退化的 `Tensor`

如果一个张量的形状是空列表 `[]`（即 **0 阶张量，Rank-0 Tensor**），它在数学上就是一个单一的标量（Scalar）。在 Flurry 中，我们可以在类型构造器的后置契约与分支里直接声明：**当 `dims` 为空时，`Tensor<T>` 就是 `T` 本身。**

```flurry
use std.prelude.*;

comptime predicate valid_shape(shape: meta.List<usize>) {
    shape.all(|d: usize| -> bool d > 0)
}

-- 统一的静态形状张量类型构造器
pure comptime fn Tensor<
    T: Type,
    ...dims: refines { valid_shape(itself) } meta.List<usize>,
> -> result: Type
where
    ensures dims.is_empty() ==> result == T
{
    inline if dims.is_empty() {
        -- 0 阶张量直接退化为标量类型 T。
        T
    } else {
        -- 非空形状：构造持有连续元素存储的张量结构类型
        struct {
            const ElementType: Type = T;
            const SHAPE: meta.List<usize> = dims;
            const RANK: usize = dims.len();
        }
    }
}
```

下面来看看不同维度参数下的 `Tensor` 在类型系统中的结构：

```text
                调用 pure comptime fn Tensor<T, ...dims> -> Type
                                      │
          ┌───────────────────────────┼───────────────────────────┐
          ▼                           ▼                           ▼
┌───────────────────┐       ┌───────────────────┐       ┌───────────────────┐
│   Tensor<f64>     │       │ Tensor<f64, 128>  │       │Tensor<f64, 64, 32>│
│  (dims = [])      │       │ (dims = [128])    │       │(dims = [64, 32])  │
└─────────┬─────────┘       └─────────┬─────────┘       └─────────┬─────────┘
          │                           │                           │
          ▼                           ▼                           ▼
┌───────────────────┐       ┌───────────────────┐       ┌───────────────────┐
│ 触发空列表退化:   │       │ Normal-Form 类型头│       │ Normal-Form 类型头│
│   result == f64   │       │ Tensor<f64, .dims │       │ Tensor<f64, .dims │
│                   │       │   = [128]>        │       │   = [64, 32]>     │
│ 0 阶张量与标量 f64│       │ 1 阶向量 (Rank 1) │       │ 2 阶矩阵 (Rank 2) │
│ 完全等价          │       │ 轴长 128          │       │ 形状 64 × 32      │
└───────────────────┘       └───────────────────┘       └───────────────────┘
```

> **💡 核心心智模型：Normal-Form 类型头与 Rank-0 等价**
> 1. **非空形状保留 Normal-Form 类型身份**：当 `dims` 非空时，`Tensor<f64, 128, 128>` 与 `Tensor<f64, .dims = [128, 128]>` 归一化到同一个 Normal-Form 类型头。只有当元素类型 `T` 相等且形状序列逐轴相等时，两个张量类型才相等。
> 2. **Rank-0 张量与标量等价**：通过 `ensures dims.is_empty() ==> result == T`，PTC 能够证明 `Tensor<f64>`（即 `Tensor<f64, .dims = []>`）等于 `f64`。因此在编写泛化到任意阶数的张量算法时，不需要为标量单独维护一套重载版本。

---

## 三、 严格边界：同形契约保护与非法形状拦截

有了 `Tensor<T, ...dims>`，我们现在可以写出只有在形状一致时才允许调用的张量操作，同时让编译器静态拦截非法的 `0` 维轴或形状错配。

### 编译期拦截形状错配

```flurry
use std.prelude.*;

-- 要求两个输入张量必须拥有相同的元素类型 T 和相同的形状 s
fn assert_same_shape(
    lhs: Tensor<T, .dims = s>,
    rhs: Tensor<T, .dims = s>,
)
where
    T: Type,
    s: refines { valid_shape(itself) } meta.List<usize>,
{}

comptime {
    -- ✅ 1. 变参写法与 .dims = 显式列表写法表示相同的类型。
    assumes Tensor<f64, 128, 64> == Tensor<f64, .dims = [128, 64]>;

    -- ✅ 2. 0 阶张量直接等于标量类型 f64。
    assumes Tensor<f64> == f64;

    -- ✅ 3. 维度顺序不同（128×64 vs 64×128）对应不同的类型。
    assumes Tensor<f64, 128, 64> != Tensor<f64, 64, 128>;
}

fn broken_tensor_shapes(
    a: Tensor<f64, 128, 64>,
    b: Tensor<f64, 128, 32>,
) {
    -- 负例 1：把 128×64 的张量和 128×32 的张量传给要求同形的函数
    -- ❌ 编译期拒绝：第 1 轴维度 64 != 32，无法合一为同一个形状列表 s。
    assert_same_shape(a, b);

    -- 负例 2：试图构造包含 0 长度轴的退化张量
    -- ❌ 编译期拒绝：valid_shape([128, 0]) 求值为 false，违反了 dims 参数的 refines 契约。
    let _: Tensor<f64, 128, 0> = undefined;
}
```

---

## 本篇小结

| 特性维度 | Flurry 设计方案 | 核心作用与收益 |
| :--- | :--- | :--- |
| **形状表示（Shape）** | `...dims: meta.List<usize>` | 用一套类型构造器统一表达从 0 维到任意 $n$ 维的张量 |
| **双重调用形式** | `Tensor<f64, 128, 64>` 与 `Tensor<T, .dims = s>` | 具体代码直接写变参，泛型代码直接供应形状列表 `s` |
| **合法轴长精化** | `refines { valid_shape(itself) }`（要求每轴 $d > 0$） | 在编译期拒绝 `Tensor<f64, 128, 0>` 这类退化零轴形状 |
| **0 阶张量退化** | `Tensor<T>`（空形状 `[]`）`== T` | 标量即为 Rank-0 张量，无需为标量与张量维护两套平行接口 |

---

### 下一篇预告

我们已经用 `Tensor<T, .dims = s>` 表达了静态形状张量，并能要求两个张量具有相同形状。

但在实际计算中，我们经常需要把一个形状为 `(128, 64)` 的矩阵和一个形状为 `(64,)` 的偏置向量相加，或者把 `(256, 1, 64)` 和 `(1, 32, 64)` 的张量相加，即 **张量广播（Broadcasting）**。

如果允许广播，两个输入张量的形状列表 `s1` 和 `s2` 可以不同：
- 编译器如何在**编译期**判断 `s1` 和 `s2` 是否满足广播条件？
- 广播之后产生的新张量，其结果形状 `broadcast_shape(s1, s2)` 如何在类型签名中计算？

下一篇 **《092 — 静态形状张量实战（二）：编译期形状广播（Broadcasting）推导》** 将介绍如何用纯编译期函数实现广播规则。
