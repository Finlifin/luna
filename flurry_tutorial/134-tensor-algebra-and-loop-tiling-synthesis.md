# Flurry 演进手记 (134)：张量代数（Tensor Algebra）——从高维算子表达式到分块循环的合成之路

在上一回（133）中，我们看到了编译器如何运用组合子代数（Combinator Algebra）将一维的高阶流管道 `map -> filter -> fold` 去森林化（Deforestation），融合成零临时分配的单次遍历。

现在，让我们把维度从一维拉升到高维，与我们在第四卷（066–095）打造的**静态形状张量引擎 `Tensor<T, .dims = s>`** 会师。
在第四卷里，我们已经利用双层类型理论（2LTT）、`Tuple<...>` 和 PTC 命题约束，在编译期解决了张量维度匹配与广播（Broadcasting）的类型安全问题。
但对于一个真正能打的科学计算与机器学习系统来说，光有“编译期形状零报错”才走完了一半路程——另一半生死攸关的问题在**代码生成与程序合成（Program Synthesis）**上：

当你写下 `C = A * B + D`（逐元素乘加）或者矩阵乘加 `Y = matmul(A, B) + Bias` 时，编译器是怎样把高维的张量代数表达式，一步步降维、实施**算子融合（Operator Fusion）**与**缓存分块循环（Loop Tiling）**，并在遇到别名重叠或求解器 `Unknown` 时死守正确性底线的？

今天，我们就来拆解 Flurry **张量代数与循环合成（Tensor Algebra & Loop Synthesis）** 的核心机制。

---

## 一、 问题背景与设计思路：为什么“算子级逐个调用库函数”会卡在内存墙（Memory Wall）上？

在传统的张量库设计中（急切求值模式），每个运算符 `+`、`*`、`matmul` 都是一个独立的黑盒函数调用：

```
  [ ❌ 算子逐个急切执行 (Eager Operator-by-Operator Execution) ]
  计算表达式：Out = (A + B) * C   (假设 A, B, C 均为百万级元素的张量)

  第 1 步：调用 add(A, B)
           ──► 分配百万级临时缓冲 Temp
           ──► 从主存读 A, B，算出 A[i] + B[i] 写回主存 Temp（此时 Temp 挤爆 L1/L2 缓存。）
  第 2 步：调用 mul(Temp, C)
           ──► 再次从慢速主存把刚刚写进去的 Temp 读出来，乘以 C[i] 写入 Out
           ──► 销毁并释放 Temp。
```

看出来问题有多严重了吗？
现代 CPU 的寄存器算术吞吐量快速，但主存（DRAM）带宽却窄得可怜。在逐个算子急切求值模式中，为了算一次简单的 `(A + B) * C`，每个元素却要在主存和寄存器之间来回搬运 **5 次**，还要承受一次昂贵的临时内存分配与释放。

而在大尺寸矩阵乘法 `matmul(A, B)` 中，如果采用最朴素的三重循环 `for i -> for j -> for k`，内层循环沿着矩阵 `B` 的列跳跃访问内存，每次访存都引发 Cache Line 失效（Cache Miss），硬件算力连 5% 都发挥不出来。

---

## 二、 核心心智模型：索引表示（Index Notation）、算子融合与分块变换（Loop Tiling）

在 Flurry 的程序合成架构中，高维张量运算在进入底层机器指令之前，会被提取为**有限无环张量映射区域（Tensor Mapping Region）**，并经历三步严谨的等价合成：

```
+-------------------------------------------------------------------------+
|               Flurry 张量代数到高效循环的三步合成流水线                   |
+-------------------------------------------------------------------------+
|                                                                         |
|  第 1 步：提取索引代数表示 (Index Notation) + 生成合法基准实现 (Reference)|
|  • 逐元素表达式 `Out = (A + B) * C`  ──►  ∀ i, j: Out[i,j] = (A[i,j]+B[i,j])*C[i,j]|
|  • 矩阵乘加 `Y = A @ B + Bias`       ──►  ∀ i, j: Y[i,j] = Bias[i,j] + ∑_k A[i,k]*B[k,j]|
|                                                                         |
|  第 2 步：代数化简与算子融合 (Operator Fusion)                            |
|  • 消除中间张量 Temp：将共享相同迭代空间的连续逐元素算子，直接内联进      |
|    前序算子（如 matmul 累加器写回处）或单次遍历循环体内，实现寄存器级直传  |
|                                                                         |
|  第 3 步：循环调度与缓存分块合成 (Loop Tiling / Blocking)                 |
|  • 将大跨度循环轴 i, j, k 切分为外层块循环 (io, jo, ko) 与内层微核循环    |
|    (ii, ji, ki)，使工作集常驻 L1/L2 Cache 与 SIMD 向量寄存器             |
+-------------------------------------------------------------------------+
```

让我们用一张直观的 ASCII 图看清为什么 **分块循环（Loop Tiling）** 能拯救矩阵乘法的缓存命中率：

```
  [ 朴素三重循环 (i, j, k)：B 按列大跨步跳跃，频繁 Cache Miss ]
  A 的第 i 行: [ *  *  *  * | *  *  *  * ]
  B 的第 j 列:   ↓ (跨整行步长。每次读完一个元素，同一 Cache Line 的邻居全被浪费。)

  [ 分块循环合成 (Tile Size = T x T)：小块常驻 L1 缓存重复复用。 ]
  外层循环枚举块坐标 (io, jo, ko)，内层在 T x T 微块内密集计算：
        B 的子块 B[ko..ko+T, jo..jo+T] (完全装入 L1 Cache!)
              +-------+
              | * * * |
  A 的子块    | * * * |
  +-------+   +-------+
  | * * * |   | += 累加到 C 的子块 C[io..io+T, jo..jo+T] (常驻寄存器!)
  | * * * |   +-------+
  +-------+
```

---

## 三、 实战演练：2x2 分块微核与逐元素偏置融合（Fused MatMul + Bias）

在 Flurry 中，无论合成器搜索出多么精妙的分块参数，它的语义基准永远与显式写出的分块累加 + 尾部融合完全一致。
让我们写出一个静态形状 $4 \times 4$ 矩阵乘加 `Y = A * B + Bias` 的基准模型与 $2 \times 2$ 分块融合实现，并在测试中验证两者在任意输入下 100% 等价：

```flurry
typealias Mat4 = Array<Array<i32, 4>, 4>;

-- 1. 基准降码实现 (Reference Implementation)：朴素三重循环矩阵乘 + 偏置
pure fn matmul_bias_reference(a: Mat4, b: Mat4, bias: Mat4) -> Mat4 {
    var out = bias;
    for i in 0usize..4usize {
        for j in 0usize..4usize {
            var sum: i32 = out[i][j];
            for k in 0usize..4usize {
                sum += a[i][k] * b[k][j];
            }
            out[i][j] = sum;
        }
    }
    out
}

-- 2. 分块循环 + 算子融合实现 (Tiled & Fused Candidate, Tile Size = 2)
--    消除中间矩阵分配，按 2x2 子块遍历以最大化局部性
pure fn matmul_bias_tiled_fused(a: Mat4, b: Mat4, bias: Mat4) -> Mat4 {
    var out = bias;
    -- 外层循环按步长 2 步进分块起点 (io_step, jo_step, ko_step in 0..2)
    for io_block in 0usize..2usize {
        for jo_block in 0usize..2usize {
            for ko_block in 0usize..2usize {
                let io = io_block * 2usize;
                let jo = jo_block * 2usize;
                let ko = ko_block * 2usize;
                -- 内层 2x2 微核循环（可完全展开并装入寄存器）
                for ii in 0usize..2usize {
                    for ji in 0usize..2usize {
                        var acc = out[io + ii][jo + ji];
                        for ki in 0usize..2usize {
                            acc += a[io + ii][ko + ki] * b[ko + ki][jo + ji];
                        }
                        out[io + ii][jo + ji] = acc;
                    }
                }
            }
        }
    }
    out
}

test tensor_algebra_baseline_vs_tiled_fused_equivalence {
    let a: Mat4 = [
        [1, 2, 0, 1],
        [0, 1, 3, 2],
        [2, 0, 1, 1],
        [1, 1, 2, 0],
    ];
    let b: Mat4 = [
        [2, 0, 1, 1],
        [1, 3, 0, 2],
        [0, 1, 2, 1],
        [3, 2, 1, 0],
    ];
    let bias: Mat4 = [
        [10, 10, 10, 10],
        [20, 20, 20, 20],
        [30, 30, 30, 30],
        [40, 40, 40, 40],
    ];

    let ref_result = matmul_bias_reference(a, b, bias);
    let tiled_result = matmul_bias_tiled_fused(a, b, bias);

    -- 验证分块融合候选与基准降码在全元素上严格相等。
    assumes ref_result == tiled_result;
    -- 第 0 行第 0 列：bias(10) + (1*2 + 2*1 + 0*0 + 1*3) = 10 + 7 = 17
    assumes tiled_result[0][0] == 17;
}
```

---

## 四、 语义边界：程序合成的三大安全红线（重叠写回、数值语义与 `Unknown` 回退）

正因为张量代数合成器在编译器内部拥有改写循环结构的强大权力，Flurry 对它施加了与 `BitVec` 代数（第 118 回）完全一致的**三项安全约束（`docs/` 语言规范）**：

### 红线 1：重叠切片写回（Overlapping Writeback）无法边覆盖边读取

回想我们在 `BitVec` 代数中学过的重叠赋值规则：`word'bitvec[4..12] = word'bitvec[0..8]` 必须先完整取得右侧 `[0..8]` 的值，再写入 `[4..12]`，禁止于循环里一边覆盖第 `4..8` 位，一边拿已经被污染的新值去填第 `8..12` 位。

在张量切片与视图原地更新（In-Place View Update）中，这条核心规则同样成立：
如果目标写入区域与右侧读取的张量视图可能指向**同一块底层存储且存在索引依赖冲突**（例如 `row[1..4] = row[0..3]`），合成器**严禁**将其直接融合成从左到右覆盖原地的单层裸循环，而必须先通过快照或反向安全调度保持“先取右侧原值、再提交更新”的可观察语义。

```flurry
-- 安全的重叠平移更新：保持右侧原值语义，不因正向原地覆盖而把 row[0] 一路广播污染到末尾。
fn safe_overlapping_shift(row: Array<i32, 4>) -> Array<i32, 4> {
    let snapshot = row;
    var updated = row;
    for i in 0usize..3usize {
        updated[i + 1usize] = snapshot[i];
    }
    updated
}

test overlapping_tensor_slice_update_preserves_snapshot_semantics {
    let original: Array<i32, 4> = [10, 20, 30, 40];
    let shifted = safe_overlapping_shift(original);
    -- 正确结果应为 [10, 10, 20, 30]，而非被边写边读污染出的 [10, 10, 10, 10]。
    assumes shifted == [10, 10, 20, 30];
}
```

### 红线 2：形状契约在编译期把关，合成器不绕过维度检查

任何进入张量合成区域的表达式，必须先在类型检查（Elaboration / PTC）阶段通过维度一致性检验。如果有人试图把两个形状互斥的张量直接相加，直接在类型检查阶段被维度相等性约束拦下：

```flurry
-- ❌ 编译期直接报错：Array<i32, 4> 与 Array<i32, 3> 维度长度不等，静态形状检查未通过
fn reject_mismatched_tensor_shapes(
    left: Array<i32, 4>,
    right: Array<i32, 3>,
) {
    let invalid: Array<i32, 4> = right;
}
```

### 红线 3：核验公式 `P(x) => defined(cand, x) and cand(x) = ref(x)` 与安全回退

在编译器内部，无论采用代数重写还是分块搜索，每一个候选方案 `candidate` 都必须满足：

$$P(x) \implies \text{defined}(\text{candidate}, x) \land \text{candidate}(x) = \text{reference}(x)$$

- **免于引入新的未定义行为**：比如分块余数边界（当矩阵维度不是分块大小 `T` 的整数倍时的尾部 Cleanup Loop）无法发生越界读取或整数溢出；
- **维持原状而未改浮点结合律敏感语义**：除并非用户显式开启了允许结合律重排的数学优化策略，否则严格遵守 IEEE-754 顺序求值契约；
- **`Unknown` 或超预算时无损回退**：当面对十分复杂的动态步长或符号边界，导致等价性验证器返回 `Unknown` 或搜索预算耗尽时，编译器**直接保留已经生成的合法基准实现（`best = reference`）**，编译照常成功，结果 100% 正确。

---

## 五、 本篇小结

| 合成阶段 / 维度 | 核心机制 | 严格保证的不变量 |
| :--- | :--- | :--- |
| **基准降码 (`reference`)** | 按算子定义直接生成结构化循环 | 任意合法输入下结果有明确定义，独立于搜索器是否成功 |
| **算子融合 (Operator Fusion)** | 将连续逐元素算子内联进同一循环核或累加尾声 | 消除临时张量分配，减少主存往返带宽 |
| **分块合成 (Loop Tiling)** | 将迭代轴拆分为外层块循环与内层微核循环 | 提升 L1/L2 Cache 空间与时间局部性，处理非整除尾部边界 |
| **重叠写回保护** | 检测读写视图的内存别名与索引依赖 | 保持“先取右侧原值、再提交写入”的快照语义，防止原地覆盖污染 |
| **回退契约 (`Unknown` / Timeout)** | 搜索超预算或等价性证明未决时保留 `best` | 优化等级只影响性能选择，维持原状而未改程序可观察结果与资源责任 |

---

## 六、 下一篇预告

理论与武器都已经磨砺到了高度。
在过去十几回里，我们分别掌握了：
- 用 **`@define_macro2` + PEG 文法** 在 Flurry 里解析自定义的嵌入式 DSL（第 130 回）；
- 用 **CMTT 的 `< Ψ |- T >`、`< T -| Ψ >`、Quoted Arm 与编译期循环** 把数据结构组装成零开销的原生状态机（第 120–126 回）；
- 用 **`BitVec<n>` 代数与组合子/张量合成** 榨干底层位操作与循环的每一分硬件性能（第 118、133、134 回）。

现在，是时候迎来**第六卷的综合实战里程碑（双篇连载：135 & 136）**了。
我们要为高吞吐传感器网络网关亲手打造一个类似 Linux 内核 **BPF（Berkeley Packet Filter）** 的声明式数据包过滤引擎——**Mini-BPF**。
- 在上半部（135 回），我们将用 `@define_macro2` 的 PEG 文法设计一门优雅的过滤规则小语言，让工程师能直接在代码里写出声明式的数据包过滤规则，并在编译期将其解析为强类型的规则 AST；
- 在下半部（136 回），我们将用 CMTT 把这棵规则 AST 直接编译成**零解释器开销的原生 Flurry 模式匹配与位检查代码**。

下一回 **《Flurry 演进手记 (135)：第六卷里程碑（上）：用 PEG 构造宏设计一门迷你数据包过滤语言（Mini-BPF DSL）》**，让我们点火起飞。
