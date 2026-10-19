# 095 — 第四卷里程碑：构建编译期 100% 形状安全的迷你张量引擎

> **阅读指南**：恭喜你来到了《Flurry 演进手记》第四卷（066–095）的收官里程碑！在过去的 29 篇教程里，我们深入了 Flurry 最具革命性的核心地带：**编译期宇宙与类型系统（2LTT、一等类型值、Normal-Form 相等性、精化类型 `refines`、命题类型检查器 PTC、行多态、位向量视图与编译期展开）**。今天，我们将把这整整一卷书的武器库全部亮出来，从零组装一个五脏俱全的 **编译期 100% 形状安全的迷你张量引擎（Mini Static-Shaped Tensor Engine）**，并在实战中为即将开启的第五卷（Trait、一致性与派生系统）埋下关键伏笔！

---

## 一、 架构全景图：当 2LTT、PTC 与 `inline for` 汇聚一堂

在动手写代码之前，让我们先用一张架构全景图，看看我们即将构建的迷你张量引擎是如何调动第四卷的每一项核心特性的：

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│                 迷你静态形状张量引擎 (Mini Tensor Engine) 全景图             │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  1. 构建配置层 (094)                                                         │
│     const TRACE_SHAPES: bool = build.mode matches .debug;                    │
│     └─▶ 结合 comptime 参数决定是否在编译期/运行期输出形状跟踪信息            │
│                                                                              │
│  2. 形状契约与 PTC 求解层 (077–087, 091–092)                                 │
│     • comptime predicate valid_shape(S)         ──▶ 拦截任何 d == 0 的退化轴 │
│     • comptime predicate broadcastable(S1, S2)  ──▶ 从右向左校验广播兼容性   │
│     • pure comptime fn broadcast_shape(S1, S2)  ──▶ 编译期算出输出张量形状   │
│     • pure comptime fn flat_element_count(S)    ──▶ 编译期算出底层缓冲区总长 │
│                                                                              │
│  3. 类型构造与分支收窄层 (066–076, 085–086, 090–091)                         │
│     • pure comptime fn Tensor<T, ...dims> -> Type                            │
│       ├─ dims 为空 #[] ──▶ 退化为标量 T (Rank-0 等价!)                       │
│       └─ dims 非空     ──▶ Normal-Form 结构体，内嵌 Array<T, TOTAL_LEN>      │
│     • Occurrence Typing + 'type 视图 ──▶ 静态提取任意张量的阶数与形状        │
│                                                                              │
│  4. 零开销步长展开层 (093)                                                   │
│     • row_major_strides(S) + broadcast_strides(S_src, S_dst)                 │
│     • inline for axis in 0 .. rank ──▶ 编译期完全摊平多维寻址与广播循环！    │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 二、 核心实现：逐步组装迷你张量引擎（v0 -> v1 -> v2）

让我们把这套设计一步步写成完整、严谨的 Flurry 代码。

### 模块一：形状谓词、总元素数与广播形状推导

首先，我们定义合法形状谓词 `valid_shape`、计算底层一维连续数组长度的 `flat_element_count`，以及从右向左推导的广播函数 `broadcast_shape`：

```flurry
use std.prelude.*;

-- 094 回：从当前包的构建实例读取是否为 debug 模式
const TRACE_SHAPES: bool = build.mode matches .debug;

-- 078 & 091 回：精化谓词——静态形状的每一根轴长度必须严格大于 0
comptime predicate valid_shape(shape: meta.List<usize>) {
    shape.all(fn(d: usize) -> bool { d > 0 })
}

-- 计算任意合法形状对应的底层一维连续存储总元素个数
pure comptime fn flat_element_count(
    shape: meta.List<usize> refines valid_shape(shape),
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

-- 092 回：单轴广播兼容性与广播结果维度
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
    let common = inline if r1 < r2 { r1 } else { r2 };

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
    let max_rank = inline if r1 > r2 { r1 } else { r2 };

    var out: meta.List<usize> = #[];
    inline for idx in 0 .. max_rank {
        let offset = max_rank - idx;
        let d1 = inline if offset <= r1 { s1[r1 - offset] } else { 1 };
        let d2 = inline if offset <= r2 { s2[r2 - offset] } else { 1 };
        out = out.push_back(broadcast_axis(d1, d2));
    }
    out
}
```

### 模块二：`Tensor` 类型构造器、步长计算与 Occurrence Typing 提取

接下来，我们写出 `Tensor` 类型构造器。注意它是如何把编译期算出的 `flat_element_count(dims)` 直接塞进底层定长存储 `Array<T, flat_element_count(dims)>` 中的！

```flurry
-- 093 回：编译期预计算行优先连续步长
pure comptime fn row_major_strides(
    shape: meta.List<usize> refines valid_shape(shape),
) -> meta.List<usize> {
    let rank = shape.len();
    var strides: meta.List<usize> = #[];
    var running: usize = 1;

    inline for rev_idx in 0 .. rank {
        let axis = rank - 1 - rev_idx;
        strides = #[running] ++ strides;
        running = running * shape[axis];
    }
    strides
}

-- 071, 085 & 091 回：Normal-Form 类型构造器与 Rank-0 标量退化
pure comptime fn Tensor<
    T: Type,
    ...dims: meta.List<usize> refines valid_shape(dims),
> -> result: Type
where
    ensures dims.is_empty() implies result == T
{
    inline if dims.is_empty() {
        T
    } else {
        struct {
            const element_type: Type = T;
            const shape: meta.List<usize> = dims;
            const rank: usize = dims.len();
            const strides: meta.List<usize> = row_major_strides(dims);
            const len: usize = flat_element_count(dims);

            data: Array<T, flat_element_count(dims)>,
        }
    }
}

-- 086 & 090 回：利用 Occurrence Typing 与类型模式提取任意张量的阶数
pure comptime fn tensor_rank_of(T: Type) -> ?usize {
    inline if T is do {
        Tensor<_, .dims = s> => s.len(),
        _ => null,
    }
}
```

### 模块三：零开销多维寻址与静态广播加法

现在，让我们利用 `inline for` 实现二维张量 `(R, C)` 与一维偏置向量 `(C,)` 的零开销广播加法 `add_matrix_bias`，以及支持编译期 `trace` 定制的二维元素读取 `get_2d`：

```flurry
-- 093 & 094 回：零开销多维寻址 + 可选编译期调试跟踪
fn get_2d<
    R: usize refines R > 0,
    C: usize refines C > 0,
>(
    mat: Tensor<f64, R, C>,
    row: usize,
    col: usize,
    comptime trace: bool = false,
) -> f64
where
    requires row < R and col < C
{
    const STRIDES = row_major_strides(#[R, C]);
    let coords: Array<usize, 2> = #[row, col];

    -- inline for 在编译期彻底展开为：row * C + col * 1
    var linear_idx: usize = 0;
    inline for axis in 0 .. 2 {
        linear_idx = linear_idx + coords[axis] * STRIDES[axis];
    }

    inline if TRACE_SHAPES and trace {
        println("accessing ({}, {}) -> flat index {}", row, col, linear_idx);
    }

    mat.data[linear_idx]
}

-- 092 & 093 回：编译期证明广播兼容性，并返回精确广播形状的张量
fn broadcast_check_and_rank<
    T: Type,
    S1: meta.List<usize> refines valid_shape(S1),
    S2: meta.List<usize> refines valid_shape(S2),
>(
    lhs: Tensor<T, .dims = S1>,
    rhs: Tensor<T, .dims = S2>,
) -> usize
where
    requires broadcastable(S1, S2)
{
    const OUT_SHAPE = broadcast_shape(S1, S2);
    OUT_SHAPE.len()
}
```

---

## 三、 终极验收：编译期证明与静态防呆测试

让我们写一段完整的验收测试，检阅我们的迷你张量引擎如何在编译期放行合法的广播与寻址，并精准拦截每一处违约调用：

```flurry
comptime {
    -- 1. 验证 Rank-0 张量与标量的类型恒等性
    assumes Tensor<f64> == f64;

    -- 2. 验证变参语法与显式 .dims 列表语法的 Normal-Form 相等性
    assumes Tensor<f64, 2, 3> == Tensor<f64, .dims = #[2, 3]>;

    -- 3. 验证编译期广播推导：(256, 1, 64) ⊕ (32, 64) ==> (256, 32, 64)
    assumes broadcast_shape(#[256, 1, 64], #[32, 64]) == #[256, 32, 64];

    -- 4. 验证编译期行优先步长计算：#[2, 3] 的步长为 #[3, 1]
    assumes row_major_strides(#[2, 3]) == #[3, 1];
}

test {
    let mat = Tensor<f64, 2, 3> {
        data: #[
            1.0, 2.0, 3.0,
            4.0, 5.0, 6.0,
        ],
    };

    -- 传入编译期合法坐标 (1, 2)，PTC 的 INT 求解器自动证明 1 < 2 and 2 < 3！
    let val = get_2d(mat, 1, 2, .trace = true);
    println("mat[1, 2] = {}", val);
}
```

再看看当有人试图写出破坏形状或越界的危险代码时，我们的迷你张量引擎是如何筑起铜墙铁壁的：

```flurry
fn broken_tensor_engine_usage(
    mat: Tensor<f64, 2, 3>,
    bad_bias: Tensor<f64, 4>,
) {
    -- 拦截 1：试图声明带有 0 维度的非法张量类型
    -- ❌ 编译期直接拒绝！违反 refines valid_shape(dims) 契约
    let _: Tensor<f64, 2, 0> = undef;

    -- 拦截 2：试图将 (2, 3) 的矩阵与 (4,) 的向量做广播
    -- ❌ 编译期直接拒绝！尾轴 3 与 4 既不相等也非 1，
    -- 违反 requires broadcastable(#[2, 3], #[4]) 契约！
    let _ = broadcast_check_and_rank(mat, bad_bias);

    -- 拦截 3：试图用越界坐标 (2, 0) 访问 2×3 矩阵
    -- ❌ 编译期直接拒绝！INT 求解器抓获 row == 2 不满足 requires row < 2！
    let _ = get_2d(mat, 2, 0);
}
```

---

## 四、 第四卷全卷回顾总结表

在挥别第四卷之际，让我们用一张浓缩的速查表回顾我们在这一卷里点亮的核心技能树：

| 章节模块 | 核心机制与关键字 | 解决的根本问题 |
| :--- | :--- | :--- |
| **066–070：2LTT 与跨阶段传递** | `comptime`、`const`、Splice / Proxy、`quote`、`Type` | 打通编译期元编程与运行期零开销代码的双向桥梁 |
| **071–076：类型构造与反射** | `pure comptime fn` 返回 `Type`、Normal-Form 头、`Type.size_of` | 用普通纯函数统一泛型与类型族，告别晦涩的模板元编程 |
| **077–083：精化与类型契约** | `comptime predicate`、`refines`、`requires` / `ensures`、`loose` | 把值域不变量与维度关系刻进类型签名，两阶段检查兼顾灵活与严密 |
| **084–088：PTC 与多理论求解** | Occurrence Typing、11 大求解器（`EUF`/`INT`/`SEQ`/`ROW`/`BV` 等） | 在分支内自动收窄类型与事实，跨领域协同证明并生成真实反例诊断 |
| **089–090：精确位宽与视图** | `u5`、`BitVec<N>`、`expr'type`、`expr'bitvec`、`expr'tuple_enumerate` | 区分逻辑位宽与物理存储字节，将 `'view` 直接映射为专属 HIR 节点 |
| **091–095：静态张量与构建配置** | `Tensor<T, ...dims>`、`broadcast_shape`、`inline for`、`build.mode` | 实现编译期 100% 形状安全、零步长循环开销与包隔离的条件编译 |

---

### 留给下一卷（第五卷）的悬念

看着我们亲手打造的迷你张量引擎，你是不是已经跃跃欲试，想写出下面这样符合直觉的数学表达式了：

```flurry
-- 我们不想总是写 broadcast_add(a, b) 或 print_tensor(a)！
-- 我们想直接写：
let c = a + b;
println("result: {}", c);
```

甚至，我们还希望：
1. 只要元素类型 `T` 支持加法（`Add`），`Tensor<T, ...dims>` 就能自动获得 `+` 运算符重载，并且直接复用我们的编译期广播推导！
2. 只要一个自定义结构体里的所有字段都支持相等比较（`Eq`）、哈希（`Hash`）或调试格式化（`Debug`），我们能不能写一个普通的编译期函数 `derive fn`，让编译器自动帮我们生成这些 Trait 的实现，彻底告别 Rust 过程宏（Proc-Macro）需要单独建 crate、解析 TokenStream 的痛苦？

这正是统一契约与多态的领地！请翻开第五卷的第一篇 **《096 — Trait 的契约本质：不只是方法集合，更是命题与关联类型》**，我们将正式开启 **Trait、一致性（Coherence）、接收者自动适配与零过程宏 `derive fn`** 的全新篇章！
