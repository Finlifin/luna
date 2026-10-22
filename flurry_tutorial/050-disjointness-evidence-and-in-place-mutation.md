# Flurry 演进手记 (050)：证明两个切片彼此分离——分离证据（Disjointness Evidence）与并行原地修改

在 [上一篇 (049)](./049-memory-views-fields-and-intervals.md) 中，我们学会了用内存视图（如 `self.right`、`values.*[first]`）在元层精确指代结构体的字段或数组的元素。

今天，我们的环形缓冲区（`RingBuffer`）开发进入了深水区：我们需要实现**双区域原地批处理（In-Place Dual-Region Mutation）**。
具体来说，有两个高频场景：
1. **场景 A（双通道状态独立更新与重排）**：结构体 `Pair { left: i32, right: i32 }` 里有两个相邻字段，我们依次调用 `set_left(pair, 1)` 和 `set_right(pair, 2)`。编译器能否证明这两次写入互无扰于，从而在保持 `left` 已建立事实的同时，甚至允许指令级重排？
2. **场景 B（同数组异位交换与半开区间分割）**：在同一个连续数组 `values: *Array<i32, n>` 中，我们要同时修改 `values.*[first]` 和 `values.*[second]`，或者把数组从中间切点 `mid` 分成前半段 `[0..mid]` 和后半段 `[mid..n]`，把前半段的数据原地校准后写进后半段。

如果你写过 Rust，看到“在同一个数组上同时拿到两个可变切片”时，你一定会想起那个常见的借用检查报错：
`cannot borrow *values as mutable more than once at a time`。
在 Rust 中，因为借用检查器未识整数下标算术，哪怕你明明白白写了 `arr[0..mid]` 和 `arr[mid..n]`，编译器也难以读懂 `0..mid` 和 `mid..n` 彼此分离——你被迫去调用标准库里的 `split_at_mut(mid)`，而翻开标准库源码一看，里面赫然是用 `unsafe` 和裸指针强行绕过借用检查器的。

而在 C 语言中，如果你想让编译器相信两个指针彼此分离，你得手写 `restrict` 关键字；可一旦你偶有疏忽传了两个重叠的指针进去，编译器一声不吭，直接生成错误的机器码（Undefined Behavior）。

在 Flurry 中，我们既不想写盲目信任的 `restrict`，也不想为了切个数组就退化到 `unsafe`。那么，Flurry 是如何直接用语言内置的**分离证据（Disjointness Evidence）**解决这个千古难题的？

---

## 一、 没有分离证据时的保守失效与性能枷锁

让我们先看看，如果我们在操作同一个数组的两个下标 `first` 和 `second` 时，**没有提供任何关于 `first` 和 `second` 是否相等的证据**（即初始写法），会发生什么：

```flurry
use std.prelude.*;

-- 只要求 first < n 且 second < n，但没有要求 first != second。
fn write_maybe_overlapping(values: *Array<i32, n>, first: usize, second: usize) -> i32
where n: usize,
    requires first < n and second < n
{
    values.*[first] = 11i32;
    values.*[second] = 22i32;
    -- 问题来了：如果调用者传进来的 first 和 second 相等（比如都是 0usize），
    -- 那么第 2 行就会把第 1 行刚写进去的 11 覆盖成 22。
    values.*[first]
}

test {
    var arr: Array<i32, 2> = [0i32, 0i32];
    -- 传不同下标时输出 11，传相同下标 (0, 0) 时输出 22。
    println("{} {}",
        write_maybe_overlapping(arr.ref, 0usize, 1usize),
        write_maybe_overlapping(arr.ref, 0usize, 0usize));
}
```

注意：在 Flurry 中，由于顺序代码允许合法别名（我们在 [第 003 篇](../flurry_tutorial/003-let-does-not-freeze-your-object.md) 讲过），`write_maybe_overlapping` **在顺序执行下是完全内存安全的**。
但是，因为缺少 `first != second` 的分离证据，它付出了两个代价：
1. **静态事实被击碎（Fact Invalidation）**：在第 1 行执行完 `values.*[first] = 11i32` 时，验证器记下了事实 `values.*[first] == 11`。但紧接着执行第 2 行 `values.*[second] = 22i32` 时，因为验证器无从得知 `second` 是否会等于 `first`，它必须**保守合并影响**——将之前关于 `values.*[first] == 11` 的事实作废。
2. **优化与并行被锁死**：编译器不敢把第 3 行的 `values.*[first]` 直接常量折叠为 `11`，也不敢把前两行写入重排或交由 SIMD/多线程并行执行。

怎样才能让编译器获得确凿的**分离证据（Disjointness Evidence）**呢？

---

## 二、 Flurry 认可的三类合法分离证据

翻开 `docs/resource-management/memory-views.md`（§分离性）与 `docs/resource-management/runtime-purity.md`（示例 10），Flurry 明确规定：
> **区域分离必须有对象身份、布局或区间证据。**

具体来说，编译器通过以下三条铁证来判定两个内存视图 $V_1$ 与 $V_2$ 彼此分离：

```text
  Flurry 的三大分离证据（Disjointness Evidence）来源：

  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 证据 1：已证明互不重叠的结构体字段布局 (Struct Field Layout)            │
  │   struct Pair { left: i32, right: i32 }                                 │
  │   ┌──────────────┬───────────────┐                                      │
  │   │ .left (0..4) │ .right (4..8) │ ──► 物理布局证明两字段字节区间分离   │
  │   └──────────────┴───────────────┘                                      │
  ├─────────────────────────────────────────────────────────────────────────┤
  │ 证据 2：同一有效数组中的不相交元素或半开区间 (Disjoint Index/Interval)  │
  │   values: *Array<i32, n>                                                │
  │   - 元素分离：`first < n and second < n and first != second`            │
  │   - 半开区间分离：`[0..mid]` 与 `[mid..n]` (分割点 mid 位于半开边界  )  │
  │   ┌──────────────────────┬──────────────────────┐                       │
  │   │  [0 .. mid)          │  [mid .. n)          │ ──► 交集为空集 ∅      │
  │   └──────────────────────┴──────────────────────┘                       │
  ├─────────────────────────────────────────────────────────────────────────┤
  │ 证据 3：已核验契约交付的独立分配或稳定节点 (Independent Allocations)    │
  │   两个不同局部变量槽位，或由分配器/容器契约证明互不重叠的独立节点。     │
  └─────────────────────────────────────────────────────────────────────────┘
```

来看看这三条证据在代码里是怎么发挥威力的。

### 1. 用结构布局证明字段写入可保持、可交换

在下面的 `Pair` 结构体中，`set_left` 声明 `outcomes write<pair.left>`，`set_right` 声明 `outcomes write<pair.right>`。
当我们在 `set_both` 中依次调用它们时，编译器直接根据 `struct Pair` 的物理内存布局，证明了 `pair.left` 与 `pair.right` 完全分离。

### 2. 用 `first != second` 证明同数组双元素分离

而在 `write_distinct` 中，只要我们在 `where requires` 子句中补上 **`first != second`**，编译器结合 `i32` 的非零独立元素布局、合法下标 `first < n and second < n` 以及不等条件 `first != second`，立刻得出 `values.*[first]` 与 `values.*[second]` 分离的铁证。

让我们写出 改进版本：

```flurry
use std.prelude.*;

struct Pair {
    left: i32,
    right: i32,
}

fn set_left(pair: *Pair, value: i32)
where outcomes write<pair.left>
{
    pair.left.* = value;
}

fn set_right(pair: *Pair, value: i32)
where outcomes write<pair.right>
{
    pair.right.* = value;
}

-- 证据 1（结构体布局分离）：
-- 编译器证明 pair.left 与 pair.right 彼此分离，set_right 完好保留而无损于 set_left 写入的事实，
-- 甚至在满足入口条件下允许编译器安全交换这两次写入的机器指令次序。
fn set_both(pair: *Pair) {
    set_left(pair, 1i32);
    set_right(pair, 2i32);
}

-- 证据 2（同数组不等下标分离）：
-- 有了 `first != second`，第 2 行对 second 的写入不触碰 first，返回值恒为 11。
fn write_distinct(values: *Array<i32, n>, first: usize, second: usize) -> result: i32
where n: usize,
    requires first < n and second < n and first != second,
    outcomes write<values.*[first]> and write<values.*[second]>
        and read<values.*[first]>,
    ensures result == 11i32
{
    values.*[first] = 11i32;
    values.*[second] = 22i32;
    values.*[first]
}

test {
    let p = Pair { left: 0i32, right: 0i32 }
    set_both(p.ref);
    println("pair: {} {}", p.left, p.right);

    var arr: Array<i32, 4> = [0i32, 0i32, 0i32, 0i32];
    println("distinct result: {}", write_distinct(arr.ref, 0usize, 2usize));
}
```

运行输出：

```text
pair: 1 2
distinct result: 11
```

看 `write_distinct` 签名上的 `ensures result == 11i32`。
因为有了 `first != second` 的分离证据，`values.*[first] == 11i32` 这个事实稳稳地跨过了 `values.*[second] = 22i32` 这行写入，直接帮我们证明了后置条件 `result == 11i32`。

---

## 三、 半开区间分割 `[0..mid]` 与 `[mid..n]`，以及伪分离陷阱拦截

除了单个元素下标，环形缓冲区在发生回绕或分块拷贝时，最常用的是**半开区间（Half-open Intervals）**分割。

为什么系统编程偏爱左闭右开的半开区间 `lo..hi`（即包含 `lo`，排除 `hi`）？
看看下面的区间切分图就明白了：

```text
  数组总区间：0 .. n  (其中 0 <= mid <= n)

  左半区间：0 .. mid      ──► 包含下标 { i | 0 <= i < mid }
  右半区间：mid .. n      ──► 包含下标 { j | mid <= j < n }
                ▲
         分割点 `mid` 可以位于任一端（包括 mid == 0 或 mid == n 的空区间情况）。
         因为左边是 `< mid`，右边是 `>= mid`，两个集合天然彼此分离。
```

当 `mid == 0` 时，左区间 `0..0` 是**空区间**（排除任何可解引用元素），右区间是 `0..n`。
> **🔍 语法与实现小贴士：空区间的安全红线**
> 正如规范（`docs/resource-management/memory-views.md`）所强调的：**“区间采用半开边界时，分割点可以位于任一端；空区间允许作为描述存在，但严禁通过首元素解引用来构造它。”**
> 此外，从逻辑元素区间映射到物理字节区间时，编译器还会一并核查元素尺寸、对齐和地址运算不会溢出。

下面我们编写一个把数组在 `mid` 处一分为二、把左半段元素加上偏移量写进右半段对应位置的函数：

```flurry
use std.prelude.*;

-- 将数组分为 [0..mid] 与 [mid..(mid + mid)] 两个彼此分离区间，
-- 从左半区读取数据，原地写入右半区。
fn copy_and_calibrate_half(values: *Array<i32, n>, mid: usize, offset: i32)
where n: usize,
    requires mid <= n / 2usize
{
    var i = 0usize;
    while i < mid {
        -- 左区下标 i < mid，右区下标 mid + i >= mid，严格分离。
        let raw = values.*[i];
        values.*[mid + i] = raw + offset;
        i += 1usize;
    }
}

test {
    var buf: Array<i32, 6> = [10i32, 20i32, 30i32, 0i32, 0i32, 0i32];
    copy_and_calibrate_half(buf.ref, 3usize, 5i32);
    println("{} {} {} | {} {} {}",
        buf[0usize], buf[1usize], buf[2usize],
        buf[3usize], buf[4usize], buf[5usize]);
}
```

运行输出：

```text
10 20 30 | 15 25 35
```

### 警惕三大“伪分离”陷阱编译器当场拆穿（`-- ❌ 编译期直接报错`）

看到 `struct Pair { left: i32, right: i32 }` 里仅凭 `.left` 和 `.right` 字段名不同就能证明分离，有些同学可能会产生错觉：“是否只要两个访问路径的名字或 key 有所差异，编译器就会认为它们分离？”

**大错特错。** Flurry 的分离性分析不是天真的“名字比对器”，它严格建立在物理内存布局与契约之上。以下三种常见的“伪分离”，编译器统统不承认它们分离：

```text
  三大“伪分离”陷阱（名字不同，但物理内存重叠。）：

  陷阱 1：Union（联合体）的不同成员名
     union Word { as_u32: u32, as_i32: i32 }
     ┌────────────────────────────────┐
     │ 共享同一块 4 字节物理存储。    │ ◄── `.as_u32` 与 `.as_i32` 100% 重叠。
     └────────────────────────────────┘

  陷阱 2：零大小类型 (ZST) 或共享位域
     两个零大小字段的地址可能完全相同，共享位域落在同一个字节内，无法单凭名字或地址数值不等判分离。

  陷阱 3：未核验容器的“不同 Key 查找”
     map.get(k1) 与 map.get(k2)：如果容器没有交付“不同 key 必定返回独立稳定节点”的已核验契约，
     不同 key 完全可能因为哈希冲突处理或内部缓存共享同一槽位。
```

让我们看看，如果有人试图在 `union` 上套用 `struct` 的字段分离假设，指望修改了第 2 个成员后第 1 个成员的事实还能保留，编译器会如何静态拦截：

```flurry
union RawSlot {
    signed: i32,
    unsigned: u32,
}

-- ❌ 编译期直接报错。
-- 错误：RawSlot 是 union（共享存储），不同字段名 `.signed` 与 `.unsigned` 缺乏分离性。
-- 写入 `slot.unsigned` 会立即使 `slot.signed.* == 10` 的事实失效，无法证明 ensures。
verified fn union_not_disjoint(slot: *RawSlot) -> result: i32
where ensures result == 10i32
{
    unsafe {
        slot.signed.* = 10i32;
        slot.unsigned.* = 99u32;
        slot.signed.*
    }
}
```

---

## 本篇小结

对比 C、Rust 与 Flurry 在处理“多区域彼此分离修改”时的方案，高下立判：

| 语言 / 方案 | 如何处理同数组/同对象多区域修改 | 安全性与易用性局限 |
| :--- | :--- | :--- |
| **C (`restrict`)** | 程序员手写 `restrict` 口头保证彼此分离 | **零验证**：传了重叠指针直接触发未定义行为（UB） |
| **Rust (`&mut`)** | 借用检查器一刀切禁止同一数组两次 `&mut` 下标 | **过度报错**：必须退化到标准库内部的 `unsafe`（如 `split_at_mut`） |
| **Flurry (分离证据)** | 编译器基于**布局（`struct` 字段）、算术区间（`i != j`、`0..mid` 与 `mid..n`）及契约**自动证明分离 | **既安全又无需 `unsafe`**：保留跨调用静态事实，严防 `union` 等伪分离陷阱 |

### 下一篇预告

在这两篇里，我们多次在函数签名下写了形如：

```flurry
where outcomes write<self.right>
```

以及：

```flurry
where outcomes write<values.*[first]> and read<values.*[first]>
```

还记得我们在 [第 003 篇](./003-let-does-not-freeze-your-object.md) 讲“`let` 不冻结字段”时留下的那个大悬念吗？
当时我们问：**“如果 `let buf = ...` 无法阻止别人修改 `buf` 的字段，那当我们写一个只想读取缓冲区的函数 `inspect(buf: *RingBuffer)` 时，怎么从契约上 100% 保证它不会偷偷修改或释放 `buf`？”**

答案就是 `where outcomes ...` 里的**内存效果（Memory Effects）**。
而且，除了我们见过的 `read` 和 `write`，Flurry 一共定义了 **六种核心内存效果**。
- 为什么销毁一个对象不叫 `free` 而叫 **`kill`**？
- 当你复制一个指针视图时，为什么会产生 **`connect`** 效果？
- 为什么 `outcomes` 被设计为**行为上界（Behavior Upper Bound）**，而且普通函数平时根本无须手写它？

在下一篇 **《Flurry 演进手记 (051)：给每次内存触碰记账——六种内存效果（`read`/`write`/`kill`...）与行为上界》** 中，我们将全面揭开 Flurry 内存效果记账系统的面纱。
