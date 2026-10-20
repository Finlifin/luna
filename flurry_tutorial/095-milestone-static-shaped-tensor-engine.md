# Flurry 演进手记 (095)：第四卷里程碑：构建编译期形状安全的迷你张量引擎

> **阅读指南**：本篇是《Flurry 演进手记》第四卷（066–095）的阶段性综合实战。在前面的章节中，我们学习了 **编译期计算与类型系统（2LTT、一等类型值、Normal-Form 相等性、精化类型 `refines`、命题类型检查器 PTC、行类型、位向量视图与编译期展开）**。本篇将这些特性组合起来，实现一个静态形状张量示例（Static-Shaped Tensor Engine），并为第五卷（Trait、一致性与派生系统）做好铺垫。

---

## 一、 架构概览：2LTT、PTC 与 `inline for` 的协同

下面先梳理该静态形状张量示例的四个组成层次：

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│                     迷你静态形状张量引擎 (Mini Tensor Engine)                │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  1. 构建配置层 (094)                                                         │
│     const TRACE_SHAPES: bool = build.mode matches .debug;                    │
│     └─▶ 结合可选参数决定是否输出形状跟踪信息                                 │
│                                                                              │
│  2. 形状契约与 PTC 求解层 (077–087, 091–092)                                 │
│     • comptime predicate valid_shape(s)         ──▶ 拒绝 d == 0 的退化轴     │
│     • comptime predicate broadcastable(s1, s2)  ──▶ 从右向左校验广播兼容性   │
│     • pure comptime fn broadcast_shape(s1, s2)  ──▶ 编译期计算输出张量形状   │
│     • pure comptime fn flat_element_count(s)    ──▶ 编译期计算底层缓冲区总长 │
│                                                                              │
│  3. 类型构造与分支收窄层 (066–076, 085–086, 090–091)                         │
│     • pure comptime fn Tensor<T, ...dims> -> Type                            │
│       ├─ dims 为空 [] ──▶ 退化为标量 T (Rank-0 等价)                         │
│       └─ dims 非空     ──▶ Normal-Form 结构体，内嵌 Array<T, total_len>      │
│     • Occurrence Typing + 类型模式 ──▶ 静态提取张量的阶数与形状              │
│                                                                              │
│  4. 步长预计算与展开层 (093)                                                 │
│     • row_major_strides(s) + broadcast_strides(s_src, s_dst)                 │
│     • inline for axis in 0 .. rank ──▶ 编译期展开多维寻址计算                │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 二、 核心实现：组装迷你张量引擎

### 模块一：形状谓词、总元素数与广播形状推导

首先定义合法形状谓词 `valid_shape`、计算底层一维连续数组长度的 `flat_element_count`，以及从右向左推导的广播函数 `broadcast_shape`：

```flurry
use std.prelude.*;

-- 094 篇：从当前包的构建实例读取是否为 debug 模式
const TRACE_SHAPES: bool = build.mode matches .debug;

-- 078 & 091 篇：精化谓词——静态形状的每一根轴长度必须严格大于 0
comptime predicate valid_shape(shape: meta.List<usize>) {
    shape.all(|d: usize| -> bool d > 0)
}

-- 计算任意合法形状对应的底层一维连续存储总元素个数
pure comptime fn flat_element_count(
    shape: refines { valid_shape(itself) } meta.List<usize>,
) -> result: usize
where
    ensures result > 0
{
    var total: usize = 1;
    inline for d in shape {
        total = total * d;
    }
    total
}

-- 092 篇：单轴广播兼容性与广播结果维度
comptime predicate axis_compatible(d1: usize, d2: usize) {
    d1 == d2 or d1 == 1 or d2 == 1
}

pure comptime fn broadcast_axis(d1: usize, d2: usize) -> usize
where
    requires axis_compatible(d1, d2)
{
    inline if d1 == 1 { d2 } else { d1 }
}

comptime predicate broadcastable(
    s1: meta.List<usize>,
    s2: meta.List<usize>,
) {
    let r1 = s1.len();
    let r2 = s2.len();
    let common = inline if r1 < r2 { r1 } else { r2 }

    var ok = true;
    inline for offset in 1 .. (common + 1) {
        ok = ok and axis_compatible(s1[r1 - offset], s2[r2 - offset]);
    }
    ok
}

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
        let offset = max_rank - idx;
        let d1 = inline if offset <= r1 { s1[r1 - offset] } else { 1 }
        let d2 = inline if offset <= r2 { s2[r2 - offset] } else { 1 }
        out = out.push_back(broadcast_axis(d1, d2));
    }
    out
}
```

### 模块二：`Tensor` 类型构造器、步长计算与 Occurrence Typing 提取

接下来写出 `Tensor` 类型构造器，将编译期算出的 `flat_element_count(dims)` 用于底层定长存储 `Array<T, flat_element_count(dims)>`：

```flurry
-- 093 篇：编译期预计算行优先连续步长
pure comptime fn row_major_strides(
    shape: refines { valid_shape(itself) } meta.List<usize>,
) -> meta.List<usize> {
    let rank = shape.len();
    var strides: meta.List<usize> = [];
    var running: usize = 1;

    inline for rev_idx in 0 .. rank {
        let axis = rank - 1 - rev_idx;
        strides = [running] ++ strides;
        running = running * shape[axis];
    }
    strides
}

-- 071, 085 & 091 篇：Normal-Form 类型构造器与 Rank-0 标量退化
pure comptime fn Tensor<
    T: Type,
    ...dims: refines { valid_shape(itself) } meta.List<usize>,
> -> result: Type
where
    ensures dims.is_empty() ==> result == T
{
    inline if dims.is_empty() {
        T
    } else {
        struct {
            const ElementType: Type = T;
            const SHAPE: meta.List<usize> = dims;
            const RANK: usize = dims.len();
            const STRIDES: meta.List<usize> = row_major_strides(dims);
            const LEN: usize = flat_element_count(dims);

            data: Array<T, flat_element_count(dims)>,
        }
    }
}

-- 083 & 086 篇：利用 Occurrence Typing 与类型模式提取张量的阶数
pure comptime fn tensor_rank_of(T: Type) -> ?usize {
    inline if T is do {
        Tensor<_, .dims = s> => s.len(),
        _ => null,
    }
}
```

### 模块三：多维寻址展开与静态广播校验

利用 `inline for` 实现支持编译期 `trace` 定制的二维元素读取 `get_2d`，以及编译期校验广播兼容性的 `broadcast_check_and_rank`：

```flurry
-- 093 & 094 篇：多维寻址展开 + 可选调试跟踪
fn get_2d(
    mat: Tensor<f64, r, c>,
    row: usize,
    col: usize,
    .trace: bool = false,
) -> f64
where
    r: refines { itself > 0 } usize,
    c: refines { itself > 0 } usize,
    requires row < r and col < c
{
    const STRIDES = row_major_strides([r, c]);
    let coords: Array<usize, 2> = [row, col];

    -- inline for 在编译期展开为：row * c + col * 1
    var linear_idx: usize = 0;
    inline for axis in 0 .. 2 {
        linear_idx = linear_idx + coords[axis] * STRIDES[axis];
    }

    if TRACE_SHAPES and trace {
        println("accessing ({}, {}) -> flat index {}", row, col, linear_idx);
    }

    mat.data[linear_idx]
}

-- 092 & 093 篇：编译期证明广播兼容性，并计算广播结果的阶数
fn broadcast_check_and_rank(
    lhs: Tensor<T, .dims = s1>,
    rhs: Tensor<T, .dims = s2>,
) -> usize
where
    T: Type,
    s1: refines { valid_shape(itself) } meta.List<usize>,
    s2: refines { valid_shape(itself) } meta.List<usize>,
    requires broadcastable(s1, s2)
{
    const OUT_SHAPE = broadcast_shape(s1, s2);
    OUT_SHAPE.len()
}
```

---

## 三、 验收测试：正向证明与负向拒绝用例

下面验证合法调用与违约调用的检查行为：

```flurry
comptime {
    -- 1. 验证 Rank-0 张量与标量的类型相等性
    assumes Tensor<f64> == f64;

    -- 2. 验证变参语法与显式 .dims 列表语法的 Normal-Form 相等性
    assumes Tensor<f64, 2, 3> == Tensor<f64, .dims = [2, 3]>;

    -- 3. 验证编译期广播推导：(256, 1, 64) 与 (32, 64) 广播为 (256, 32, 64)
    assumes broadcast_shape([256, 1, 64], [32, 64]) == [256, 32, 64];

    -- 4. 验证编译期行优先步长计算：[2, 3] 的步长为 [3, 1]
    assumes row_major_strides([2, 3]) == [3, 1];
}

test {
    let mat = Tensor<f64, 2, 3> {
        data: [
            1.0, 2.0, 3.0,
            4.0, 5.0, 6.0,
        ],
    }

    -- 传入合法坐标 (1, 2)，满足 1 < 2 and 2 < 3
    let val = get_2d(mat, 1, 2, .trace = true);
    println("mat[1, 2] = {}", val);
}
```

再看违约调用如何被编译期拒绝：

```flurry
fn broken_tensor_engine_usage(
    mat: Tensor<f64, 2, 3>,
    bad_bias: Tensor<f64, 4>,
) {
    -- 拒绝 1：试图声明带有 0 维度的非法张量类型
    -- ❌ 编译期拒绝：违反 refines { valid_shape(itself) } 契约
    let _: Tensor<f64, 2, 0> = undefined;

    -- 拒绝 2：试图将 (2, 3) 的矩阵与 (4,) 的向量做广播
    -- ❌ 编译期拒绝：尾轴 3 与 4 既不相等也非 1，
    -- 违反 requires broadcastable([2, 3], [4]) 契约
    let _ = broadcast_check_and_rank(mat, bad_bias);

    -- 拒绝 3：试图用越界坐标 (2, 0) 访问 2×3 矩阵
    -- ❌ 编译期拒绝：row == 2 不满足 requires row < 2
    let _ = get_2d(mat, 2, 0);
}
```

---

## 四、 第四卷回顾总结表

| 章节模块 | 核心机制与关键字 | 解决的问题 |
| :--- | :--- | :--- |
| **066–070：2LTT 与子句参数** | `comptime`、`const`、`quote`、`Type`、`where` | 统一编译期计算、类型参数与非类型子句参数声明 |
| **071–076：类型构造与反射** | `pure comptime fn` 返回 `Type`、Normal-Form 头、`Type.size_of` | 用纯编译期函数表达泛型与类型构造，保留名义头身份 |
| **077–083：精化与类型模式** | `comptime predicate`、`refines`、`requires` / `ensures`、类型模式 | 将值域约束与结构关系写入签名，支持类型解构 |
| **084–088：PTC 与命题约束** | Occurrence Typing、算术/序列/行/位宽等理论协同求解 | 在分支内收窄类型与事实，检验编译期类型命题 |
| **089–090：精确位宽与视图** | `u5`、`BitVec<n>`、`expr'type`、`expr'bitvec`、`expr'tuple_enumerate` | 区分逻辑位宽与物理存储，通过 `'view` 切换观察视角 |
| **091–095：静态张量与构建配置** | `Tensor<T, ...dims>`、`broadcast_shape`、`inline for`、`build.mode` | 在编译期完成形状检验、步长展开与包隔离配置读取 |

---

### 下一卷预告

在这个张量示例的基础上，我们自然会希望支持运算符与通用接口：

```flurry
let c = a + b;
println("result: {}", c);
```

例如：
1. 当元素类型 `T` 支持加法（`Add`）时，为 `Tensor<T, ...dims>` 实现 `+` 运算符并接入编译期广播推导；
2. 当自定义结构体的字段都支持相等比较（`Eq`）、哈希（`Hash`）或格式化时，通过 `derive fn` 生成对应的 Trait 实现。

下一篇 **《096 — Trait 的契约本质：关联项、默认方法与实现边界》** 将开启第五卷：Trait、一致性（Coherence）、接收者自动适配与 `derive fn`。
