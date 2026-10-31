# Flurry 演进手记 (133)：组合子代数（Combinator Algebra）——让编译器认识并化简你的高阶流管道

在上一回（132）中，我们梳理了从 `inline fn`、`comptime fn`、CMTT 到构造宏的四级元编程选型决策树。在那些工具里，是由**我们（程序员）**编写生成规则去指导编译器展开代码。

从本篇开始，我们将进入第六卷的下半场——**程序合成与代数优化（Program Synthesis & Algebraic Optimization）**。
我们在第五卷第 118 回已经领略过 `BitVec<n>` 代数的风采：程序员只管用最直观的切片 `bits[0..8]`、拼接 `++` 和逻辑运算描述“输入比特到输出比特的映射”，编译器先生成 100% 正确的**基准实现（Baseline Implementation）**，再在搜索预算内利用代数恒等式合成出最优的目标机器指令。

今天，我们要把这套“先保基准语义、再做等价代数合成”的哲学，应用到日常开发中最高频的高阶抽象上——**组合子代数（Combinator Algebra）与流管道化简（Pipeline Simplification）**。

---

## 一、 问题背景与设计思路：为什么高阶流管道总是面临“优雅 vs 性能/语义安全”的撕裂？

在编写传感器网关的数据清洗流水线时，我们最喜欢用高阶组合子（Combinators）把复杂的业务逻辑拆成清晰的阶段：
1. 先用 `map(decode)` 把原始字节解码为采样值；
2. 再用 `map(calibrate)` 做线性校准；
3. 接着用 `filter(is_in_range)` 过滤掉噪点；
4. 最后用 `fold(0, accumulate)` 汇总结果。

然而，在传统编译器中处理这样的组合子链，往往会掉进两种典型取舍：

```
  [ ❌ 方案 A：机械分层执行（无代数化简） ]
  输入数组 ──► 遍历第 1 遍生成临时数组 T1 (map decode)
           ──► 遍历第 2 遍生成临时数组 T2 (map calibrate)
           ──► 遍历第 3 遍生成临时数组 T3 (filter)
           ──► 遍历第 4 遍求和 (fold)
  • 问题背景：3 次多余的中间缓冲区分配 + 4 倍内存带宽浪费，L1 Cache 失效。

  [ ❌ 方案 B：无脑激进重写（破坏副作用与资源契约） ]
  某些流框架无论三七二十一，把 `stream.map(effectful_log).take(0)` 直接优化成空操作。
  • 问题背景：如果 `effectful_log` 带有必须执行的 I/O 效应、错误传播或消费了仿射资源 `Once`，
          激进删除或乱序合并会直接篡改程序的可观察行为。
```

为什么 Flurry 能够既消除中间冗余步骤，又完好保留而无损于任何一条语言级副作用与资源契约？
秘密就在于 **`docs/` 语言规范 的两条核心架构核心规则**：
1. **高层结构与纯度契约的完整保留**：在编译期与优化阶段，闭包纯度（`P0`–`P5`）、控制流、效应关系与资源契约均是显式类型契约的一部分。
2. **程序合成的基准与等价性契约**：每个组合子管道都有明确定义的**基准降码（Reference Lowering）**；只有当候选化简方案在当前输入域、副作用次序、错误/效应传播以及资源清理责任上被严格证明与基准等价时，才允许替换。

---

## 二、 核心心智模型：组合子代数的三大等价重写律与“可观察行为边界”

在纯计算或副作用受控的有限区域（Region）内，组合子管道满足一系列经典的代数恒等式（Algebraic Laws）：

```
+-------------------------------------------------------------------------+
|                   组合子代数 (Combinator Algebra) 核心重写律              |
+-------------------------------------------------------------------------+
|                                                                         |
|  1. Map-Map 垂直融合律 (Functor Composition Law)：                      |
|     pipe |> map(f) |> map(g)   ===>   pipe |> map(|x| g(f(x)))          |
|     • 消除中间传递，将两次逐元素转换合并为单次复合函数调用                 |
|                                                                         |
|  2. Filter-Filter 合取融合律 (Predicate Conjunction Law)：              |
|     pipe |> filter(p) |> filter(q) ===> pipe |> filter(|x| p(x) and q(x))|
|     • 注意：必须保持短路求值次序（只有 p(x) 为真时才求值 q(x)）            |
|                                                                         |
|  3. Map/Filter-Fold 循环融合律 (Loop Fusion / Deforestation)：          |
|     pipe |> map(f) |> filter(p) |> fold(init, acc_fn)                   |
|     ===> 单层循环内依次执行：let y = f(x); if p(y) { acc = acc_fn(acc, y) }|
|     • 直接消除中间集合分配（Deforestation，去森林化）                     |
+-------------------------------------------------------------------------+
```

### 2.1 什么时候严格禁止盲目套用代数重写？

请看这张编译器内部的**区域提取与行为边界核验图**（与 `docs/program-synthesis/bitvec-algebra.md` 一脉相承）：

```
  [ 基准实现 (Reference) ] ──► 确定求值次数、求值顺序、!Err / #Eff 与 Drop 清理顺序
             │
             ▼
  [ 候选合成器 (Synthesizer) 尝试应用组合子代数重写 ]
             │
             ├──► 检查 1：闭包 f 与 g 是否为按元素顺序同步执行？
             │            （若提前批量跑完所有 f 再跑所有 g，且 f/g 有日志或效应，则不等价。）
             ├──► 检查 2：若存在 `take(0)` 或常数折叠，前序必须执行的输入表达式副作用是否保留？
             ├──► 检查 3：被过滤掉（`filter(p)` 为 false）的元素若持有仿射资源，是否在当次迭代及时清理？
             │
             ├──► ✅ 全部证明成立 (`defined(cand, x) and cand(x) == ref(x)`) ──► 交付融合后的单层循环。
             └──► ⚠️ 求解器返回 Unknown 或预算耗尽 ──────────────────────────► 安全保留合法基准实现。
```

---

## 三、 实战演练：从分步数组管道到融合单遍扫描

让我们写出传感器采样数组的两阶段校准、过滤与汇总管道，看看基准语义与融合后的单遍扫描是如何保持严格一致的：

```flurry
pure fn calibrate(raw: i32) -> i32 {
    raw * 2 + 1
}

pure fn shift_up(val: i32) -> i32 {
    val + 10
}

pure fn is_valid_sample(val: i32) -> bool {
    val >= 15
}

-- 高阶组合子融合执行：对定长采样数组依次执行两层映射、一层过滤与累加折叠
pure fn process_samples_pipeline(
    samples: Array<i32, 4>,
    map1: pure fn(i32) -> i32,
    map2: pure fn(i32) -> i32,
    pred: pure fn(i32) -> bool,
) -> i32 {
    var acc: i32 = 0;
    for raw in samples {
        -- Map-Map 融合 + Filter-Fold 融合的等价单遍执行核：
        let step1 = map1(raw);
        let step2 = map2(step1);
        if pred(step2) {
            acc += step2;
        }
    }
    acc
}

test combinator_pipeline_fusion_equivalence {
    let data: Array<i32, 4> = [1, 2, 3, 4];
    -- raw=1 -> cal=3 -> shift=13 (<15, 过滤掉)
    -- raw=2 -> cal=5 -> shift=15 (>=15, 保留)
    -- raw=3 -> cal=7 -> shift=17 (>=15, 保留)
    -- raw=4 -> cal=9 -> shift=19 (>=15, 保留)
    -- 15 + 17 + 19 = 51
    let total = process_samples_pipeline(data, calibrate, shift_up, is_valid_sample);
    assumes total == 51;
}
```

在这段纯函数组合中，因为 `calibrate`、`shift_up` 和 `is_valid_sample` 都是 `pure fn`，不携带任何外部内存修改或代数效应，编译器可以放心地将 `map(calibrate)` 与 `map(shift_up)` 垂直融合为单次算术表达式 `raw * 2 + 11`，并与外层循环折叠融为一体。

---

## 四、 语义边界：当组合子遇到副作用、效应与仿射资源时的契约

如果传给组合子管道的操作数或闭包**非纯函数**——比如产生输入源的函数带有可观察的日志输出，或者每个流元素都是一个必须在被 `filter` 丢弃时妥善调用 `Drop` 释放的仿射资源（如 `PacketHandle`）——编译器还会鲁莽地把输入删掉吗？

**并非如此。** 让我们验证组合子合成中的两条核心安全不变量：

### 4.1 哪怕结果已知为空或恒定，输入构造的可观察行为亦无法丢

就像在 `BitVec` 代数中 `zero and produce_bits()` 虽然位结果恒为 `0`，但 `produce_bits()` 的副作用必须执行一次一样；在流组合子管道中，即使后续组合子只取 `0` 个元素或直接返回常数，**构造流源头实参表达式的副作用也必须按源码顺序执行一次**：

```flurry
fn produce_source(trace: *u32) -> Array<i32, 2> {
    trace.* += 1;
    [10, 20]
}

fn take_zero_sum(source: Array<i32, 2>) -> i32 {
    -- 哪怕一个元素都免于读取直接返回 0，传入的 source 实参在调用点也已完成一次求值。
    0
}

test observable_input_evaluation_preserved_under_simplification {
    var trace: u32 = 0;
    let sum = take_zero_sum(produce_source(trace.ref));
    assumes sum == 0;
    -- 证明 produce_source 的可观察状态更新并没有被优化器非法抹除。
    assumes trace == 1;
}
```

### 4.2 严禁跨阶段重排有状态/有效应的组合子顺序

如果有人写了一个依赖执行顺序的有状态闭包，或者试图把一个需要运行期可变状态的非纯函数伪装成可任意重排的 `pure fn` 喂给要求纯组合子的代数接口，编译器会直接在类型与纯度检查阶段报错：

```flurry
var global_tick: i32 = 0;

-- ❌ 编译期直接报错：impure_mapper 访问了外部可变状态，违背 pure fn 组合子契约，无法混入纯代数重排接口
pure fn impure_mapper(x: i32) -> i32 {
    global_tick += 1;
    x + global_tick
}
```

---

## 五、 本篇小结

| 组合子代数维度 | 核心规则与变换 | 必须死守的语义边界（语言规范） |
| :--- | :--- | :--- |
| **Map-Map 融合** | 连续两次逐元素映射合并为单次复合调用 | 保持每个元素先执行 `f` 后执行 `g` 的求值次序 |
| **Filter-Filter 融合** | 连续两次谓词过滤合并为短路 `and` | 前一个谓词返回 `false` 时，免于触发后一个谓词的副作用 |
| **去森林化（Loop Fusion）** | 消除 `map / filter / fold` 之间的临时容器分配 | 被 `filter` 淘汰或提前 `break` 跳出的元素，其仿射资源必须按序清理 |
| **常数/空流化简** | 识别 `take(0)` 或恒假过滤等边界 | 形成管道源操作数本身的必执副作用与边界检查**无法被静默删除** |
| **未知与超时回退** | 候选等价性证明遇到 `Unknown` 或预算耗尽 | **安全回退并保留基准降码（Reference Implementation）**，免于生成未证实代码 |

---

## 六、 下一篇预告

一维的数据流管道（`map -> filter -> fold`）通过组合子代数就能漂亮地融合成单层循环。

但是，让我们把目光投向更宏大的多维空间——我们在第四卷亲手打造的**静态形状张量 `Tensor<T, .dims = s>`**。
当科学计算或深度学习工程师在代码里写下一行看似十分简洁的高维张量代数表达式：
```flurry
let Y = matmul(A, B) + Bias;
```
或者写下一个多维张量缩并（Tensor Contraction）与逐元素激活函数的组合时：
1. 如果按笨办法先跑完完整的矩阵乘法 `matmul(A, B)` 写回主存，再从主存读出来加 `Bias`，CPU 的 L1/L2 缓存早就被冲刷得一干二净了。
2. 编译器是如何把高层张量代数表达式转化为**索引表示（Index Notation）**，实施**算子融合（Operator Fusion）**与**缓存分块循环合成（Loop Tiling Synthesis）**的？如果两个张量的内存区域发生了重叠别名（Overlapping Alias），还能无脑原地融合吗？

下一回 **《Flurry 演进手记 (134)：张量代数（Tensor Algebra）——从高维算子表达式到分块循环的合成之路》**，我们将揭开高维张量程序合成的神秘面纱。
