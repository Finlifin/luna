# 093 — 静态形状张量实战（三）：用 `inline for` 消除步长计算开销

> **阅读指南**：在上一回（第 092 回）中，我们完成了静态形状张量的编译期广播推导。现在，我们要进入静态形状张量三部曲的最后一站：**如何把编译期已知的形状 `shape: meta.List<usize>`，转化为运行时零额外开销的扁平内存寻址？** 本篇我们将结合 `docs/comptime/inlining-comptime-code.md` 与 `docs/examples/static-shaped-tensors.md`，全面掌握 Flurry 的编译期控制流内联四剑客——`comptime { ... }`、`inline if`、`inline when` 与 `inline for`，看它们如何把高维张量的步长（Stride）与坐标点积在编译期彻底摊平！

---

## 一、 痛点：高维张量寻址里的“动态循环税”

在物理内存中，无论是二维矩阵 `(R, C)` 还是四维张量 `(B, C, H, W)`，底层通常都存储在一块**连续的一维线性缓冲区（Flat Buffer）**里。

要把一个多维坐标 $(i_0, i_1, \dots, i_{k-1})$ 映射到一维线性下标 $\text{linear\_index}$，我们需要计算它与每一轴**步长（Stride）**的内积：
$$\text{linear\_index} = \sum_{a=0}^{k-1} i_a \times \text{stride}_a$$
对于行优先（Row-Major）连续存储的张量，最后一轴的步长是 `1`，倒数第二轴的步长是最后一轴的长度，依此类推。比如形状为 `(4, 8, 16)` 的三维张量，它的步长元组就是 `(128, 16, 1)`，寻址公式就是：
$$\text{linear\_index} = i_0 \times 128 + i_1 \times 16 + i_2 \times 1$$

### 尝试 `v0`：在运行时用普通 `for` 循环遍历维度计算偏移

如果我们不区分编译期控制流与运行期控制流，直接在每次访问元素时跑一个运行期 `for` 循环去算步长和偏移（`v0`）：

```flurry
-- 尝试 v0：每次访问元素都在运行期循环计算步长与偏移
-- 灾难 1：哪怕是一个简单的 2D 矩阵访问 mat[r, c]，运行期也要跑循环分支跳转！
-- 灾难 2：步长 (128, 16, 1) 明明只跟类型形状 #[4, 8, 16] 有关，
--         却在每次运行时重复做乘法！
-- 灾难 3：破坏了 LLVM 后端的自动向量化（SIMD Vectorization）！
```

**核心痛点**：
既然张量的阶数 `rank` 和每一轴的维度 `shape` 在编译期都是 100% 确定的常量，那么：
1. 每一轴的步长 $\text{stride}_a$ 应该在**编译期一次性算好**，直接变成字面量常数（如 `128`、`16`、`1`）；
2. 遍历每一轴累加 `coord * stride` 的循环，应该在**编译期按轴数完全展开（Unroll）**，让生成的机器码跟手写的 `i0 * 128 + i1 * 16 + i2` 一模一样！

---

## 二、 破局（v1）：编译期控制流内联四剑客

在 Flurry 中，为了让你能够精确控制“哪些代码在编译期求值并内联，哪些代码留在运行期执行”，语言提供了四把手术刀般精准的编译期控制流构造（见 `docs/comptime/inlining-comptime-code.md`）：

| 构造语法 | 编译期行为 | 对运行期代码（IIR / 机器码）的影响 |
| :--- | :--- | :--- |
| **`comptime { ... }`** | 整个代码块在编译期立即执行 | 不生成任何运行期指令；产生的常量可被后续代码直接 splice 引用 |
| **`inline if cond { A } else { B }`** | `cond` 必须在编译期可判定 | 只对被选中的那个分支做具象化（Elaboration）与代码生成；未选中分支完全擦除 |
| **`inline when` / `inline match`** | 多分支或模式匹配条件在编译期求值 | 编译期直接锁定唯一命中分支，消除运行期分支跳转 |
| **`inline for x in iter { ... }`** | 迭代源 `iter` 的长度与划分在编译期确定 | 循环体按元素个数在编译期**逐次展开**；每次展开中的循环变量 `x` 都是编译期已知常量，而循环体内部的普通语句则保留为高效的运行期指令！ |

让我们用一张 Code Cartoon 看看，当 `inline for` 遇到三维张量 `Tensor<f64, 4, 8, 16>` 的寻址计算时，编译器是如何施展“消骨术”的：

```text
  源码：在 Tensor<f64, 4, 8, 16> (shape = #[4, 8, 16]) 上计算线性下标
  ┌─────────────────────────────────────────────────────────────────┐
  │ const STRIDES = compute_row_major_strides(#[4, 8, 16]);         │
  │ var offset: usize = 0;                                          │
  │ inline for axis in 0 .. 3 {                                     │
  │     offset = offset + coords[axis] * STRIDES[axis];             │
  │ }                                                               │
  └────────────────────────────────┬────────────────────────────────┘
                                   │
                                   ▼ 编译期求值与 inline for 展开
  ┌─────────────────────────────────────────────────────────────────┐
  │ 1. comptime 算出 STRIDES = #[128, 16, 1]                        │
  │ 2. inline for 彻底剥离循环外壳，展开为 3 条纯运行期算术语句：    │
  │                                                                 │
  │    var offset: usize = 0;                                       │
  │    offset = offset + coords[0] * 128;  -- axis = 0 (常量折叠!)  │
  │    offset = offset + coords[1] * 16;   -- axis = 1 (常量折叠!)  │
  │    offset = offset + coords[2] * 1;    -- axis = 2 (常量折叠!)  │
  └─────────────────────────────────────────────────────────────────┘
```

### 演进到 `v1`：用 `pure comptime fn` 预计算步长 + `inline for` 展开寻址

让我们把这个过程写成真实可运行的 Flurry 代码。注意看 `inline for` 是如何既能遍历整数范围 `0 .. rank`，又能配合我们在第 090 回学过的 `'tuple_enumerate` 视图遍历异构/同构元组的：

```flurry
use std.prelude.*;

-- 1. 在编译期预计算行优先（Row-Major）步长列表
pure comptime fn row_major_strides(
    shape: meta.List<usize> refines valid_shape(shape),
) -> meta.List<usize> {
    let rank = shape.len();
    var strides: meta.List<usize> = #[];
    var running: usize = 1;

    -- 从最后一轴向第一轴倒推累乘
    inline for rev_idx in 0 .. rank {
        let axis = rank - 1 - rev_idx;
        strides = #[running] ++ strides;
        running = running * shape[axis];
    }
    strides
}

-- 2. 利用 inline for 将多维坐标寻址在编译期完全展开为扁平乘加
pure fn linear_offset_3d<
    D0: usize refines D0 > 0,
    D1: usize refines D1 > 0,
    D2: usize refines D2 > 0,
>(
    coords: Array<usize, 3>,
) -> usize
where
    requires coords[0] < D0 and coords[1] < D1 and coords[2] < D2
{
    const SHAPE: meta.List<usize> = #[D0, D1, D2];
    const STRIDES: meta.List<usize> = row_major_strides(SHAPE);

    var offset: usize = 0;
    inline for axis in 0 .. 3 {
        -- axis 与 STRIDES[axis] 全部是编译期已知常量！
        -- coords[axis] 是运行期数组读取！
        offset = offset + coords[axis] * STRIDES[axis];
    }
    offset
}

test {
    let idx = linear_offset_3d<4, 8, 16>(#[2, 3, 5]);
    -- 2 * 128 + 3 * 16 + 5 * 1 = 256 + 48 + 5 = 309
    println("linear offset: {}", idx);
}
```

> **🔍 语法小贴士：`inline for` 还能怎么玩？**
> 在 `docs/comptime/inlining-comptime-code.md` 中，`inline for` 除了可以遍历编译期已知端点的区间 `0 .. N` 和编译期列表 `meta.List` 之外，还有一个出镜率极高的黄金搭档——遍历元组的 `'tuple_enumerate` 视图：
> ```flurry
> fn show_indexed<...Ts: meta.List<Type>>(values: Tuple<...Ts>) {
>     inline for (index, element) in values'tuple_enumerate {
>         println("{}: {}", index, element);
>     }
> }
> ```
> 哪怕元组 `values` 里的每一个元素类型都不同（比如第 0 个是 `i32`，第 1 个是 `str`，第 2 个是 `bool`），因为 `inline for` 在编译期将循环体逐次展开并为每一次迭代单独做类型检查，每次迭代里的 `element` 都能精确拥有它在该槽位上的真实静态类型！

---

## 三、 广播步长（Broadcast Strides）与编译期边界红线（v2）

刚才我们算的是普通连续张量的步长。那么在上一回（第 092 回）的**张量广播（Broadcasting）**里，一个形状为 `(1, 64)` 或 `(64,)` 的张量被广播到 `(128, 64)` 时，怎么做到**零内存拷贝**呢？

秘诀就在步长里！
- 如果某根轴是缺失的前导轴，或者原本的长度为 `1`（现在被广播成了 `128`），我们在编译期直接把这根轴的**广播步长设为 `0`**！
- 当 `inline for` 展开寻址公式 `offset = offset + coords[axis] * BCAST_STRIDES[axis]` 时，因为 `BCAST_STRIDES[0] == 0`，编译器直接把 `coords[0] * 0` 常量折叠消除掉了！无论第 0 轴的循环变量怎么变，它都永远指向同一行偏置向量，连一次多余的乘法和内存分配都没有！

### 演进到 `v2`：计算零拷贝广播步长，并严守 `inline` 的阶段红线

```flurry
-- 演进到 v2：为广播输入张量计算编译期虚拟步长（扩展轴步长直接置 0！）
pure comptime fn broadcast_strides(
    source_shape: meta.List<usize> refines valid_shape(source_shape),
    target_shape: meta.List<usize> refines valid_shape(target_shape),
) -> meta.List<usize>
where
    requires broadcastable(source_shape, target_shape),
    requires target_shape == broadcast_shape(source_shape, target_shape)
{
    let base_strides = row_major_strides(source_shape);
    let src_rank = source_shape.len();
    let dst_rank = target_shape.len();

    var out: meta.List<usize> = #[];
    inline for idx in 0 .. dst_rank {
        let offset = dst_rank - idx;
        let stride = inline if offset > src_rank {
            -- 法则 3：缺失的前导轴，虚拟步长为 0！
            0
        } else {
            let src_axis = src_rank - offset;
            inline if source_shape[src_axis] == 1 and target_shape[idx] > 1 {
                -- 法则 2：长度为 1 的扩展轴，虚拟步长也为 0！
                0
            } else {
                base_strides[src_axis]
            }
        };
        out = out.push_back(stride);
    }
    out
}
```

同时，请牢记 `inline if` 与 `inline for` 的一条铁律：**它们的控制条件或迭代边界必须在编译期（`comptime`）可确定！** 如果你试图拿一个运行期才能知道的普通变量去驱动 `inline if` 或 `inline for`，编译器会当场拒绝：

```flurry
fn broken_inline_control(runtime_flag: bool, runtime_limit: usize) {
    -- 负例 1：用运行期布尔值驱动 inline if
    -- ❌ 编译期直接拒绝！
    -- runtime_flag 是运行期参数，编译器无法在编译期决定保留哪个分支！
    inline if runtime_flag {
        println("active");
    }

    -- 负例 2：用运行期整数驱动 inline for 展开
    -- ❌ 编译期直接拒绝！
    -- inline for 必须在编译期知道展开次数，不能绑定未知的 runtime_limit！
    inline for i in 0 .. runtime_limit {
        println("{}", i);
    }
}
```

---

## 本篇小结

| 编译期特性 | 在静态张量引擎中的角色 | 性能与安全收益 |
| :--- | :--- | :--- |
| **`row_major_strides`** | 编译期倒推累乘计算连续步长列表 | 步长完全成为编译期常量，运行期零重复计算 |
| **`broadcast_strides`** | 将缺失前导轴与长度 `1` 扩展轴的步长设为 `0` | 实现真正的零内存拷贝（Zero-Copy）广播视图 |
| **`inline for axis in 0..rank`** | 在编译期将任意阶数的坐标点积循环彻底展开 | 消除运行期循环分支跳转，并将 `stride == 0` 的项直接常量折叠归零 |
| **阶段边界检查** | 强制要求 `inline if` / `inline for` 头部为编译期值 | 防止误将运行期动态分支当作编译期零开销内联 |

---

### 留给下一篇的悬念

至此，我们的静态形状张量三部曲（091–093）已经把类型安全与运行期性能都推到了极致！

不过，在把这个张量库打包发布给其他团队使用之前，我们还面临一个现实的工程问题：
- 在本地开发调试（`debug` 模式）时，我们希望张量引擎开启详细的编译期与运行期跟踪日志（Trace），或者根据目标机器是否开启了 `"cuda"` / `"avx512"` 特性来切换底层算子；
- 而在传统语言里，这类构建配置往往靠丑陋的文本预处理宏 `#ifdef DEBUG` 或随处乱飞的全局环境变量来实现——更可怕的是，上游调用者改了一个全局宏，竟然会偷偷篡改下游依赖包内部的编译行为，导致缓存失效和语义撕裂！

在 Flurry 中，构建配置本身也是强类型的编译期公民！下一篇 **《094 — 构建上下文与编译期配置：`build` 包如何告别 `#ifdef`？》**，我们将揭晓六大内置包与 `build` 上下文的模块化隔离契约！
