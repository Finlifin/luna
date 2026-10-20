# Flurry 演进手记 (193)：终极实战第三层（验证层）：用 `verified fn` 与 `outcomes` 证明队列读写的分离性与无溢出

在上一篇（第 192 篇）中，我们用 `&reachability` 与指针代际身份构建了 **Orion 信号处理引擎** 的 **Layer 2（内存层）**，保证了任何分片视图所依赖的底层采样池存储始终存活。

但是，在零开销的高性能系统里，“存储活着”只是安全的一半；另一半更棘手的挑战在于**精确到元素与字节的索引边界与不相交性（Disjointness）**：
1. **二维分片偏移计算的整数溢出**：Orion 环形队列按 `num_shards * shard_stride` 划分二维逻辑网格。当调度器计算总容量或某个分片的起始偏移时，如果先算乘法 `num_shards * shard_stride` 再与物理缓冲区长度 `n` 比较，一旦机器整数乘法回绕溢出，越界检查就会被瞬间绕过。
2. **双槽位原地交换与跨槽位读写的分离性证明**：当信号重排器在同一个底层缓冲区 `buf: *mut [u16; n]` 上同时写入主槽位 `buf.*[primary_slot]` 和镜像槽位 `buf.*[mirror_slot]` 时，普通的借用检查器只能看到“你把整个 `buf` 借出去了两次”，根本无法理解 `primary_slot != mirror_slot` 意味着两块内存毫无重叠。

今天，我们进入 **Layer 3（验证层）**，用 Flurry 的 **`predicate` 程序谓词、`verified fn` 验证契约、`outcomes` 精确读写内存视图，以及三次多元多项式算术边界（Degree-3 Polynomial Bound）**，在不引入任何运行时边界分支的前提下，给出环形队列读写安全的编译期数学证明。

```
+=============================================================================+
|              Orion Signal Engine —— Layer 3: 形式化程序验证层                 |
+=============================================================================+
| Layer 2: ShardedRingArena (&reachability 保证底层存储代际存活)                 |
|                                  |                                          |
|                                  v                                          |
| Layer 3: verified fn + outcomes write<buf.*[a]> and write<buf.*[b]> <-- 本篇|
|          +-- 1. 除法前置守卫 (stride <= n / shards) 消除机器乘法溢出           |
|          +-- 2. OSL 分离逻辑证明主/镜像槽位读写视图严格不相交                  |
|          +-- 3. safe 调用端通过 if 守卫 + assumes 无缝衔接 verified 契约       |
+=============================================================================+
```

---

## 一、 问题背景：为什么先乘后比（`shards * stride <= n`）无法通过验证？

假设我们要为 Orion 队列编写一个经过形式化验证的二维分片槽位写入函数：缓冲区总长度为 `n`，划分为 `shards` 个分片，每个分片跨度为 `stride`。
很多程序员会本能地写出这样的前置条件或运行时守卫：

```
-- 危险的直觉写法：
if shards * stride <= n { ... }
```

为什么 Flurry 的验证器（以及 `safe` 代码的自动算术求解器）会直接拒绝基于 `shards * stride <= n` 的无溢出证明？
因为 `shards` 和 `stride` 是有限位宽的机器整数 `usize`。如果攻击者或损坏的配置传入极大的 `shards` 和 `stride`，使得 `shards * stride` 在机器算术下发生溢出（或者在开启溢出陷阱的模式下直接触发算术故障），这个乘法求值本身就已经越过安全边界了。

### 核心方法：三次多元多项式边界与“先除后乘”范式

在第 178 篇与 `docs/resource-management/contracts-and-verification.md` 中我们学过：
- Flurry 的 `safe` 自动算术检查器内置了针对**最高三次（Degree $\le 3$）多元多项式不等式**的快速判定过程；
- 要在机器整数下安全地建立 `shards * stride <= n` 而不触发中间溢出，必须**先验证正数除数 `shards > 0`，再用除法上界 `stride <= n / shards` 进行守卫**。一旦 `shards > 0` 且 `stride <= n / shards` 成立，三次多项式求解器就能立即自动推导出乘积 `shards * stride` 既不会溢出 `usize.MAX`，也严格小于等于 `n`。

---

## 二、 实战第一步：用 `predicate` 与 `verified fn` 证明二维分片与双槽位分离

让我们为 Orion 环形队列定义核心规范谓词与 `verified fn`。
请牢记语法规范中的规则：**`predicate` 是纯逻辑规范谓词，只存在于验证擦除域，声明时绝对不写 `comptime` 关键字。**

```
+-----------------------------------------------------------------------------+
|       outcomes 精确内存视图：证明同一数组内两个不同下标槽位的分离写权限         |
+-----------------------------------------------------------------------------+
|  底层数组 buf: *mut [u16; n]                                                 |
|  +--------+--------+-----------------+--------+-----------------+--------+  |
|  | slot 0 | slot 1 | primary_idx (p) |  ...   | mirror_idx (m)  | slot n |  |
|  +--------+--------+-----------------+--------+-----------------+--------+  |
|                            |                          |                     |
|         outcomes:   write<buf.*[p]>    and     write<buf.*[m]>              |
|                            \                          /                     |
|                     requires p < n and m < n and p != m                     |
|          ===> OSL 精确到字节区间证明两块写视图不相交 (Disjoint)                |
+-----------------------------------------------------------------------------+
```

来看完整的 `Layer 3` 验证代码：

```flurry
-- 1. 定义纯逻辑规范谓词（注意：绝对不能加 comptime 关键字。）
predicate valid_distinct_pair(primary: usize, mirror: usize, cap: usize) {
    primary < cap and mirror < cap and primary != mirror
}

predicate valid_grid_bounds(shards: usize, stride: usize, total_cap: usize) {
    shards > 0 and stride > 0 and stride <= total_cap / shards
}

-- 2. 经过形式化验证的双槽位镜像写入函数
-- 利用 outcomes 精确声明只触碰 buf.*[primary] 与 buf.*[mirror] 两个不相交槽位
verified fn write_mirrored_sample(
    buf: *Array<u16, n>,
    primary: usize,
    mirror: usize,
    sample_val: u16,
) -> void
where
    n: usize,
    requires valid_distinct_pair(primary, mirror, n),
    ensures buf.*[primary] == sample_val and buf.*[mirror] == sample_val,
    outcomes write<buf.*[primary]> and write<buf.*[mirror]>,
{
    -- 验证器由 valid_distinct_pair(primary, mirror, n) 直接获知：
    -- 1. primary < n 且 mirror < n（绝不越界，无需生成运行时 bounds check）
    -- 2. primary != mirror（两次写入的目标视图严格分离，互不覆盖）
    buf.*[primary] = sample_val;
    buf.*[mirror] = sample_val;
}

-- 3. 经过形式化验证的二维分片偏移计算函数
verified fn compute_shard_slot(
    comptime n: usize,
    shards: usize,
    stride: usize,
    shard_id: usize,
    offset_in_shard: usize,
) -> slot: usize
where
    requires valid_grid_bounds(shards, stride, n)
        and shard_id < shards
        and offset_in_shard < stride,
    ensures slot < n,
{
    -- 因为 shard_id < shards 且 offset_in_shard < stride，
    -- 结合 stride <= n / shards，三次多项式求解器直接证出：
    -- shard_id * stride + offset_in_shard < shards * stride <= n，绝无中间溢出。
    shard_id * stride + offset_in_shard
}
```

看看 `write_mirrored_sample` 的 `outcomes` 子句：
- `outcomes write<buf.*[primary]> and write<buf.*[mirror]>` 精确到了数组元素级别。
- 在客观分离逻辑（OSL）下，当调用方知道 `primary != mirror` 时，这两个写视图对应底层字节空间里完全不相交的两段区间 `[base + 2*primary, base + 2*primary + 2)` 与 `[base + 2*mirror, base + 2*mirror + 2)`。
- 不仅如此，因为 `outcomes` 承诺了该函数**只**修改这两个槽位，数组中任何其他下标 `k`（满足 `k != primary and k != mirror`）上的旧值事实在调用前后都会被验证器**自动通过框架规则（Frame Rule）完整保留**，无需手写一行繁琐的“其余元素不变”公理。

---

## 三、 实战第二步：从 `safe` 业务层通过 `assumes` 桥接调用 `verified fn`

在 Orion 引擎中，并非所有上层调度代码都需要写成重量级的 `verified fn`。大多数日常控制流是普通的 `safe fn`。
当 `safe fn` 想要调用带有 `requires` 前置契约的 `verified fn` 时，规则非常清晰：
1. 先用常规控制流（如 `if` 条件分支）建立所需的算术边界；
2. 用 **`assumes` 局部证明提示** 引导 `safe` 层的轻量级自动证明器确认前置条件已满足，然后直接零开销调用 `verified fn`。

```flurry
-- ✅ 正向示例：在 safe 代码中先建立除法上界守卫，再通过 assumes 调用 verified fn
fn dispatch_sample_to_ring(
    buf: *Array<u16, n>,
    shards: usize,
    stride: usize,
    primary_shard: usize,
    mirror_shard: usize,
    offset: usize,
    sample_val: u16,
) -> bool
where
    n: usize,
{
    -- 第一步：严格按照“先验正数、再比除法上界”的顺序建立路径条件
    if shards == 0 or stride == 0 {
        return false;
    }
    if stride > n / shards {
        return false;
    }
    if primary_shard >= shards or mirror_shard >= shards or primary_shard == mirror_shard {
        return false;
    }
    if offset >= stride {
        return false;
    }

    -- 第二步：此时三次多元多项式求解器已拥有全部路径事实
    assumes valid_grid_bounds(shards, stride, n);
    let p_slot = compute_shard_slot(n, shards, stride, primary_shard, offset);
    let m_slot = compute_shard_slot(n, shards, stride, mirror_shard, offset);

    -- 因为 primary_shard != mirror_shard 且 offset < stride，p_slot 与 m_slot 必然不相等且均小于 n
    if p_slot < n and m_slot < n and p_slot != m_slot {
        assumes valid_distinct_pair(p_slot, m_slot, n);
        write_mirrored_sample(buf, p_slot, m_slot, sample_val);
        true
    } else {
        false
    }
}
```

---

## 四、 编译器如何拦截伪造的假设与重叠槽位？（负向示例）

如果程序员在 `safe` 函数里偷懒，没有检查 `primary != mirror` 或没有检查 `stride <= n / shards`，就企图直接写一句 `assumes valid_distinct_pair(...)` 蒙混过关，会发生什么？

请记住：**`assumes` 在 Flurry 中绝不是放弃检查的“豁免凭证（trust me）”**。它是向当前函数的自动证明器提交的一条局部引理目标——证明器必须能从当前分支已有的路径条件中**直接证出** `assumes` 里的命题，否则直接报错拒绝编译。

```flurry
-- ❌ 拒绝例 1：未建立路径条件就企图用 assumes 强行调用 verified fn
fn reckless_dispatch(
    buf: *Array<u16, n>,
    p_slot: usize,
    m_slot: usize,
    sample_val: u16,
)
where
    n: usize,
{
    if p_slot < n and m_slot < n {
        -- 漏掉了 p_slot != m_slot 的检查。
        -- 编译错误：Cannot prove `assumes valid_distinct_pair(p_slot, m_slot, n)`:
        -- obligation `p_slot != m_slot` is not implied by current path conditions.
        assumes valid_distinct_pair(p_slot, m_slot, n);
        write_mirrored_sample(buf, p_slot, m_slot, sample_val);
    }
}

-- ❌ 拒绝例 2：未做除法前置检查，直接计算 shards * stride 导致潜在溢出
verified fn broken_grid_index(
    comptime n: usize,
    shards: usize,
    stride: usize,
) -> usize
where
    -- 错误：即便要求 shards <= n 且 stride <= n，两者相乘仍可能超出 usize.MAX。
    requires shards <= n and stride <= n,
{
    -- 编译错误：Potential arithmetic overflow in `shards * stride`;
    -- establish `shards > 0 and stride <= usize.max / shards` before multiplying.
    shards * stride
}
```

---

## 五、 本层通关小结与下一层预告

在 **Layer 3（验证层）** 中，我们为 Orion 引擎的分片环形队列加上了数学级安全锁：
- 用不带 `comptime` 的纯逻辑 `predicate` 定义了网格边界与双槽位分离条件；
- 用 `verified fn` 与精确到数组元素的 `outcomes write<buf.*[primary]> and write<buf.*[mirror]>`，在不写一行 `unsafe` 的情况下证明了双槽位并发写入的分离性与无越界；
- 遵循“先验正数、先除后乘”的三次多项式算术范式，通过 `assumes` 让 `safe` 业务代码无缝享受了形式化验证的零开销红利。

现在，采样点已经安全、零拷贝、无溢出一一落入了分片环形队列的槽位中。接下来，我们要对这些采样点执行数字信号处理的核心算法——**有限冲激响应（FIR）卷积滤波**。

在下一篇 **第 194 篇（Layer 4：元编程层）** 中，我们将运用 **两阶段类型论（2LTT）与上下文模态类型论（CMTT）**，在编译期根据滤波器阶数与抽头系数数组，通过**单孔链式拼接（Single-Hole Chain Splicing）** 生成完全循环展开、无分支开销的定制化 FIR 卷积代码。
