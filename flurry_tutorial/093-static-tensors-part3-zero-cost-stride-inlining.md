# Flurry 演进手记 (093)：静态形状张量实战（三）：用 `inline for` 消除步长计算开销

> **阅读指南**：在上一篇（第 092 篇）中，我们完成了静态形状张量的编译期广播推导。本篇讨论：**如何把编译期已知的形状 `shape: meta.List<usize>`，转化为运行时无额外循环开销的扁平内存寻址？** 我们将结合 `docs/comptime/inlining-comptime-code.md` 与 `docs/comptime/static-shaped-tensors.md`，介绍 Flurry 的编译期控制流内联机制——`comptime { ... }`、`inline if`、`inline when` 与 `inline for`，看它们如何把高维张量的步长（Stride）与坐标点积在编译期展开。

---

## 一、 问题背景：高维张量寻址中的动态循环开销

在物理内存中，无论是二维矩阵 `(r, c)` 还是四维张量 `(b, c, h, w)`，底层通常存储在连续的一维线性缓冲区（Flat Buffer）里。

要把一个多维坐标 $(i_0, i_1, \dots, i_{k-1})$ 映射到一维线性下标 $\text{linear\_index}$，需要计算它与每一轴**步长（Stride）**的内积：
$$\text{linear\_index} = \sum_{a=0}^{k-1} i_a \times \text{stride}_a$$
对于行优先（Row-Major）连续存储的张量，最后一轴的步长是 `1`，倒数第二轴的步长是最后一轴的长度，依此类推。比如形状为 `(4, 8, 16)` 的三维张量，它的步长为 `(128, 16, 1)`，寻址公式为：
$$\text{linear\_index} = i_0 \times 128 + i_1 \times 16 + i_2 \times 1$$

### 在运行时用普通 `for` 循环遍历维度计算偏移的局限

如果不区分编译期控制流与运行期控制流，而在每次访问元素时用运行期 `for` 循环计算步长和偏移：

```flurry
-- 每次访问元素都在运行期循环计算步长与偏移：
-- 1. 简单的 2D 矩阵访问 mat[r, c] 也要在运行期执行循环分支；
-- 2. 步长 (128, 16, 1) 仅与静态形状 [4, 8, 16] 有关，却在运行时重复计算；
-- 3. 妨碍后端自动向量化（SIMD Vectorization）。
```

**核心目标**：
既然张量的阶数 `rank` 和每一轴的维度 `shape` 在编译期已知，那么：
1. 每一轴的步长 $\text{stride}_a$ 可以在**编译期预先算出**，直接成为常量（如 `128`、`16`、`1`）；
2. 遍历每一轴累加 `coord * stride` 的循环，可以在**编译期按轴数展开（Unroll）**为直接的乘加运算 `i0 * 128 + i1 * 16 + i2`。

---

## 二、 核心机制：编译期控制流与内联展开

为了明确区分哪些结构在编译期求值并展开、哪些代码留在运行期执行，Flurry 提供了四类编译期控制流构造（见 `docs/comptime/inlining-comptime-code.md`）：

| 构造语法 | 编译期行为 | 对运行期生成代码的影响 |
| :--- | :--- | :--- |
| **`comptime { ... }`** | 整个代码块在编译期立即执行 | 免于生成运行期控制流；产生的常量可在后续代码中直接使用 |
| **`inline if cond { A } else { B }`** | `cond` 在编译期求值 | 仅保留被选中的分支进入实例的 runtime 代码 |
| **`inline when` / `inline if ... is do { ... }`** | 多分支或模式匹配条件在编译期求值 | 编译期确定命中分支，免于生成运行期条件跳转 |
| **`inline for x in iter { ... }`** | 迭代源 `iter` 的展开结构在编译期确定 | 循环体按迭代项在编译期逐次展开；循环体内的普通运行期语句保留为对应操作序列 |

下面展示 `inline for` 在三维张量 `Tensor<f64, 4, 8, 16>` 上展开线性下标计算的过程：

```text
  源码：在 Tensor<f64, 4, 8, 16> (shape = [4, 8, 16]) 上计算线性下标
  ┌─────────────────────────────────────────────────────────────────┐
  │ const STRIDES = row_major_strides([4, 8, 16]);                  │
  │ var offset: usize = 0;                                          │
  │ inline for axis in 0 .. 3 {                                     │
  │     offset = offset + coords[axis] * STRIDES[axis];             │
  │ }                                                               │
  └────────────────────────────────┬────────────────────────────────┘
                                   │
                                   ▼ 编译期求值与 inline for 展开
  ┌─────────────────────────────────────────────────────────────────┐
  │ 1. comptime 算出 STRIDES = [128, 16, 1]                         │
  │ 2. inline for 展开为 3 条运行期算术语句：                       │
  │                                                                 │
  │    var offset: usize = 0;                                       │
  │    offset = offset + coords[0] * 128;  -- axis = 0              │
  │    offset = offset + coords[1] * 16;   -- axis = 1              │
  │    offset = offset + coords[2] * 1;    -- axis = 2              │
  └─────────────────────────────────────────────────────────────────┘
```

### 用 `pure comptime fn` 预计算步长 + `inline for` 展开寻址

```flurry
use std.prelude.*;

-- 1. 在编译期预计算行优先（Row-Major）步长列表
pure comptime fn row_major_strides(
    shape: refines { valid_shape(itself) } meta.List<usize>,
) -> meta.List<usize> {
    let rank = shape.len();
    var strides: meta.List<usize> = [];
    var running: usize = 1;

    -- 从最后一轴向第一轴倒推累乘
    for rev_idx in 0 .. rank {
        let axis = rank - 1 - rev_idx;
        strides = [running] ++ strides;
        running = running * shape[axis];
    }
    strides
}

-- 2. 利用 inline for 将多维坐标寻址在编译期展开为扁平乘加
pure fn linear_offset_3d(
    comptime d0: refines { itself > 0 } usize,
    comptime d1: refines { itself > 0 } usize,
    comptime d2: refines { itself > 0 } usize,
    coords: Array<usize, 3>,
) -> usize
where
    requires coords[0] < d0 and coords[1] < d1 and coords[2] < d2
{
    const SHAPE: meta.List<usize> = [d0, d1, d2];
    const STRIDES: meta.List<usize> = row_major_strides(SHAPE);

    var offset: usize = 0;
    inline for axis in 0 .. 3 {
        -- axis 与 STRIDES[axis] 均为编译期已知常量，
        -- coords[axis] 是运行期数组读取。
        offset = offset + coords[axis] * STRIDES[axis];
    }
    offset
}

test {
    let idx = linear_offset_3d(4, 8, 16, [2, 3, 5]);
    -- 2 * 128 + 3 * 16 + 5 * 1 = 256 + 48 + 5 = 309
    println("linear offset: {}", idx);
}
```

> **🔍 语法小贴士：`inline for` 结合 `'tuple_enumerate` 遍历元组**
> 除了遍历编译期已知端点的区间 `0 .. n` 和编译期列表 `meta.List` 之外，`inline for` 还可以配合第 090 篇介绍的 `'tuple_enumerate` 视图遍历元组：
> ```flurry
> fn show_indexed(values: Tuple<...ts>) where ...ts: meta.List<Type> {
>     inline for (index, element) in values'tuple_enumerate {
>         println("{}: {}", index, element);
>     }
> }
> ```
> 即使元组 `values` 中每个元素类型不同，`inline for` 也会在编译期逐项展开并为每次迭代分别检查元素类型与约束。

---

## 三、 广播步长（Broadcast Strides）与编译期阶段边界

在上一篇（第 092 篇）的**张量广播（Broadcasting）**中，一个形状为 `(1, 64)` 或 `(64,)` 的张量与 `(128, 64)` 相加时，如何避免复制底层数据？

可以通过广播步长实现：
- 如果某根轴是缺失的前导轴，或者原本的长度为 `1`（广播目标长度为 `128`），在编译期将该轴的**广播步长设为 `0`**；
- 当 `inline for` 展开寻址公式 `offset = offset + coords[axis] * BCAST_STRIDES[axis]` 时，由于 `BCAST_STRIDES[0] == 0`，无论第 0 轴的索引如何变化，该项偏移始终为 `0`。

### 计算广播步长，并遵守 `inline` 的阶段要求

```flurry
-- 为广播输入张量计算编译期虚拟步长（扩展轴步长置 0）
pure comptime fn broadcast_strides(
    source_shape: refines { valid_shape(itself) } meta.List<usize>,
    target_shape: refines { valid_shape(itself) } meta.List<usize>,
) -> meta.List<usize>
where
    requires broadcastable(source_shape, target_shape),
    requires target_shape == broadcast_shape(source_shape, target_shape)
{
    let base_strides = row_major_strides(source_shape);
    let src_rank = source_shape.len();
    let dst_rank = target_shape.len();

    var out: meta.List<usize> = [];
    for idx in 0 .. dst_rank {
        let offset = dst_rank - idx;
        let stride = if offset > src_rank {
            -- 规则 3：缺失的前导轴，虚拟步长为 0。
            0
        } else {
            let src_axis = src_rank - offset;
            if source_shape[src_axis] == 1 and target_shape[idx] > 1 {
                -- 规则 2：长度为 1 的扩展轴，虚拟步长为 0。
                0
            } else {
                base_strides[src_axis]
            }
        }
        out = out.push_back(stride);
    }
    out
}
```

需要注意 `inline if` 与 `inline for` 的阶段规则：**它们的分支条件或展开结构必须在编译期（`comptime`）可确定。** 如果用运行期普通变量作为 `inline if` 条件或 `inline for` 边界，编译器会报错：

```flurry
fn broken_inline_control(runtime_flag: bool, runtime_limit: usize) {
    -- 负例 1：用运行期布尔值驱动 inline if
    -- ❌ 编译期报错：runtime_flag 是运行期参数，无法在编译期确定分支。
    inline if runtime_flag {
        println("active");
    }

    -- 负例 2：用运行期整数驱动 inline for 展开
    -- ❌ 编译期报错：inline for 必须在编译期确定展开结构。
    inline for i in 0 .. runtime_limit {
        println("{}", i);
    }
}
```

---

## 本篇小结

| 编译期特性 | 在静态张量引擎中的角色 | 作用与收益 |
| :--- | :--- | :--- |
| **`row_major_strides`** | 编译期倒推累乘计算连续步长列表 | 步长在编译期求出，运行期无需重复计算 |
| **`broadcast_strides`** | 将缺失前导轴与长度 `1` 扩展轴的步长设为 `0` | 支持免于复制输入存储的广播寻址 |
| **`inline for axis in 0..rank`** | 在编译期将坐标点积循环按阶数展开 | 消除运行期维度循环分支，并使常量步长直接参与折叠 |
| **阶段边界检查** | 要求 `inline if` / `inline for` 头部在编译期可确定 | 区分编译期展开与运行期动态控制流 |

---

### 下一篇预告

在实际工程中，我们常常需要根据构建配置调整行为：
- 在本地开发调试（`debug` 模式）时开启跟踪日志，或根据当前包启用了哪些 feature 选择不同实现；
- 同时保证不同包拥有各自的构建配置视图，上游调用者无法暗中篡改下游依赖包内部的构建条件。

下一篇 **《094 — 构建上下文与编译期配置：`build` 包与包级配置隔离》** 将介绍内置包与 `build` 上下文的隔离规则。
