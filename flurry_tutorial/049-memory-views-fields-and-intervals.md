# Flurry 演进手记 (049)：把一块大缓冲区安全切成两半——内存视图（Memory Views）、字段视图与区间视图

在 [前几篇 (046–048)](./048-preventing-dangling-escapes-with-reachability.md) 中，我们已经看到 Flurry 如何通过对象身份、世代（Generation）和可达性限定类型 `&reachability_set T`，在无需 Rust 式 `&mut` 独占借用锁的前提下，阻止了悬垂指针逃逸。

但是，当我们继续推进第三卷的核心工程目标——实现一个**高性能零拷贝环形缓冲区（`RingBuffer`）**时，一个新的问题浮出了水面：

一个环形缓冲区对象通常包含多个组成部分：
- 头部写游标 `head` 与尾部读游标 `tail`（或者一对左右计数器 `left`、`right`）；
- 底层存放数据的连续数组 `buffer: Array<i32, n>`；
- 当写游标回绕（Wrap-around）时，有效数据在物理数组上被切成了两半：前半段区间 `buffer[0..mid]` 和后半段区间 `buffer[mid..n]`，或者两个各异的元素槽位 `buffer[first]` 与 `buffer[second]`。

现在思考一个务实的核心问题：
当我们调用一个辅助方法 `counters.clear_right()` 去清零右侧计数器时，调用方手里原本持有的左侧计数器指针 `left` 以及关于 `left` 的数值事实，是否会因为这次函数调用而“连坐失效”？
当我们先通过 `let selected = values[index].ref;` 取得了某个元素的指针，随后又修改了下标变量 `index = 1usize;`，`selected` 指向的目标是否会跟着漂移？

在传统语言里，函数签名只知道“我接收了一个 `*Counters` 指针”，至于这个函数在肚子里到底是只改了 `.right` 字段、还是把整个 `Counters` 掀了个底朝天，调用方如果不去翻看（甚至内联展开）函数体，根本无从知晓。

为了在**维持未展开态函数实现**的前提下，精确向调用方说明“我只碰了这个结构体的哪个字段、或者这个数组的哪个下标区间”，Flurry 引入了**内存视图（Memory Views）**。

---

## 一、 粗粒度整对象指针带来的状态失效问题

让我们先用最朴素的 `Counters` 结构体写一个初始写法。
假设 `Counters` 有 `left` 和 `right` 两个字段。我们在 `preserve_left` 中先确定了 `counters.left.* == 10`，然后调用了一个外部方法 `counters.clear_right()`，最后想断言 `counters.left.* == 10` 依然成立：

```flurry
use std.prelude.*;

struct Counters {
    left: i32,
    right: i32,

    -- 没有任何区域足迹契约，对外只暴露接收了 *self
    fn clear_right(*self) {
        self.right.* = 0i32;
    }
}

fn preserve_left(counters: *Counters) -> i32 {
    let left_ptr: *i32 = counters.left;
    -- 在这里，我们只调用了 clear_right
    counters.clear_right();
    left_ptr.*
}

test {
    let c = Counters { left: 42i32, right: 99i32 }
    println("{}", preserve_left(c.ref));
}
```

如果站在跨模块契约或编译器分析器的角度看 `clear_right(*self)`：
如果我们把 `*self` 笼统地看作“可能修改整个 `Counters` 对象”，会发生什么问题？

```text
  粗粒度“整对象”视角的尴尬：
  counters (*Counters)
     ├── .left  (42)  ◄── left_ptr 指向这里
     └── .right (99)

  调用 `counters.clear_right()`：
  如果契约只写“修改了 *self 指向的整个对象”：
     ──► 编译器被迫假设 `.left` 也可能被改写了。
     ──► 之前关于 `left_ptr.* == 42` 的所有静态事实全部作废，亦无法将 `left_ptr.*` 的读取优化提升到调用前。
```

更糟糕的是，对于一个容器结构体（比如包含描述符字段 `len` 和底层缓冲区指针 `data`），如果“读取描述符字段 `len`”和“读取缓冲区全部元素 `data[0..n]`”混为一谈，那么任何修改单个元素的操作都会被误认为破坏了描述符。

我们需要一套精确的“解剖刀”语言，把**容器描述符、它指向的缓冲区、结构体中的某个字段、以及数组中的某个元素或区间**，分别命名为独立的目标。

---

## 二、 用字段视图与元素视图精确指代内存区域

在 Flurry 中，**内存视图（Memory View）**就是用来在契约与规格（如 `requires`、`ensures`、`outcomes`）中精确描述内存区域的语法（详见 `docs/resource-management/memory-views.md`）。

最棒的是：内存视图**无需发明一套陌生的DSL语法**，它直接复用我们已经熟悉的字段投影、下标和解引用形态，配合 `read<...>` 和 `write<...>` 足迹构造器：

```text
  projection        -> expr.id           (字段视图：如 self.right)
  index_application -> expr[expr]        (元素/区间视图：如 values.*[first])
  dereference       -> expr.*            (解引用目标视图)
  read_footprint    -> read<expr>        (读取足迹规格)
  write_footprint   -> write<expr>       (写入足迹规格)
```

> **💡 核心心智模型：内存视图描述的是“元层区域”，它不偷偷执行运行时读取。**
> 当你在 `where outcomes write<self.right>` 或 `read<values.*[first]>` 里写下 `self.right` 和 `values.*[first]` 时：
> 1. 它是在**元层（Meta-level）**为验证器指明“哪块内存区域被观察或修改了”，**免于在运行时多执行一次内存读取**。
> 2. 反过来，单凭在规格里写出一个视图名字，也**免于自动授予你访问该内存的合法资格**——你依然需要先证明指针存活、下标未越界。

### 核心原则 1：视图在建立时立刻锁定目标身份，不跟随索引变量“漂移”

初学者最容易搞混的一个问题是：
如果我写了 `let selected = values[index].ref;`，此时 `index` 是 `0`。随后我把变量 `index` 改成了 `1`，那么 `selected` 指向的到底是原来的第 `0` 个元素，还是会跟着 `index` 变成第 `1` 个元素？

下面通过图示看清 Flurry 的**目标身份绑定规则**：

```text
  var values: Array<i32, 2> = [3, 5];
  var index = 0usize;

  [第 1 步] 建立视图 / 指针：`let selected = values[index].ref;`
            在建立的这一刻，立即绑定：
            ┌──────────────────────────────────────────────────┐
            │ 目标对象身份: values                             │
            │ 生命周期世代: gen #1                             │
            │ 锁定偏移/下标: 建立时的符号值 0usize (即首元素!) │
            └────────────────────────┬─────────────────────────┘
                                     │ (焊死在第 0 号槽位。)
                                     ▼
                              values[0] (值为 3)

  [第 2 步] 后来修改索引变量：`index = 1usize;`
            修改新位置：`values[index] = 9;` (改的是 values[1]!)
            ──► `selected` 依然稳稳指向 `values[0]`，读出来的值依然是 3。
```

让我们把“精确字段视图 `write<self.right>`”与“视图目标身份锁定”写进 改进代码：

```flurry
use std.prelude.*;

struct Counters {
    left: i32,
    right: i32,

    -- 用 `where outcomes write<self.right>` 精确声明只修改 right 字段视图。
    fn clear_right(*self)
    where outcomes write<self.right>
    {
        self.right.* = 0i32;
    }
}

fn preserve_left(counters: *Counters) -> i32 {
    let left = counters.left;
    -- 调用方根据 Counters 的字段布局和 clear_right 的精确足迹 `write<self.right>`，
    -- 无需展开 clear_right 的函数体，就能 100% 确定 `left` 视图未被触碰。
    counters.clear_right();
    left.*
}

test {
    -- 1. 验证跨调用边界保留未触碰字段视图
    let c = Counters { left: 42i32, right: 99i32 }
    println("preserved left: {}", preserve_left(c.ref));

    -- 2. 验证已取得的指针/视图在建立时绑定目标，不跟随 index 变量漂移
    var values: Array<i32, 2> = [3i32, 5i32];
    var index = 0usize;
    let selected = values[index].ref;

    index = 1usize;
    values[index] = 9i32;

    -- 输出 3（values[0]），而非 9（values[1]）。
    println("selected is still: {}", selected.*);
}
```

运行输出：

```text
preserved left: 42
selected is still: 3
```

---

## 三、 入口与出口快照位置（`'old`）与多区域元素视图

现在，我们来解决一个在设计环形缓冲区和动态数组时更精妙的问题：
假设一个函数非但读取和修改了内存，而且在调用过程中，被用作下标或长度的字段自己也变了。
比如，向缓冲区末尾追加一个元素 `push(buf, item)`：
- 在进入函数时（**入口状态，Entry Snapshot**），当前长度 `buf.len.*` 是 `3`；
- 函数把新元素写在了下标 `3` 的位置（即入口时的 `buf.len.*`），然后把 `buf.len.*` 加 1 变成了 `4`（**出口状态，Exit Snapshot**）。

这时，如果我们在函数的 `outcomes write<...>` 和 `ensures ...` 里引用视图，这些表达式到底是按**调用前（入口）**的值来算，还是按**调用后（出口）**的值来算？

Flurry 对快照位置（Snapshot Position）做出了清晰明确的划分（详见 `docs/resource-management/memory-views.md` §快照位置）：

```text
  函数调用边界上的快照时间线：

  [调用入口状态 (Entry Snapshot)]
     ├── `requires` 前置条件中的选择器       ──► 按【入口状态】解释。
     └── `outcomes` 效果足迹中的选择器       ──► 按【入口状态】解释。
         (例如追加元素写的是入口长度处，免将调用后 +1 的新长度代入同一个足迹表达式。)
        │
        ▼ (执行函数体，状态发生演进)
        │
  [对应出口状态 (Exit Snapshot)]
     └── `ensures` 后置条件中的选择器        ──► 默认按【出口状态】解释。
         └── 若需在 ensures 中回溯入口值，使用 `'old` 视图（如 `flag'old`）。
```

同时，当我们操作同一个数组 `values: *Array<i32, n>` 中的两个动态下标 `first` 和 `second` 时，我们可以直接在 `outcomes` 中分别写出两个元素视图：
- `write<values.*[first]>`
- `write<values.*[second]>`
- `read<values.*[first]>`

来看看 进阶版本是如何精确刻画数组双下标访问足迹的：

```flurry
use std.prelude.*;

-- 精确声明对数组内两个独立元素视图 `values.*[first]` 与 `values.*[second]` 的读写足迹
fn write_distinct(values: *Array<i32, n>, first: usize, second: usize) -> i32
where n: usize,
    requires first < n and second < n and first != second,
    outcomes write<values.*[first]> and write<values.*[second]>
        and read<values.*[first]>
{
    values.*[first] = 11i32;
    values.*[second] = 22i32;
    values.*[first]
}

test {
    var arr: Array<i32, 4> = [0i32, 0i32, 0i32, 0i32];
    let res = write_distinct(arr.ref, 1usize, 3usize);
    println("res: {}, arr[1]: {}, arr[3]: {}", res, arr[1usize], arr[3usize]);
}
```

运行输出：

```text
res: 11, arr[1]: 11, arr[3]: 22
```

### 编译器防呆：试图先执行可能溢出的地址加法再检查范围？或者对空区间解引用？禁止

在使用区间和下标视图时，Flurry 编译器有两条严格的安全红线：
1. **严禁先溢出再检查**：验证索引或区间边界时，无法先执行可能溢出的整数加法 `start + count < n`，再拿溢出回绕后的结果去假装安全。
2. **严禁在未建立分离或范围依据时宣称视图有效**：如果声明的 `outcomes` 只允许 `write<self.right>`，你却在函数体里偷偷写了 `self.left.* = 0`，编译器会直接报错：

```flurry
struct Counters {
    left: i32,
    right: i32,

    -- ❌ 编译期直接报错。
    -- 错误：契约声明的写足迹仅为 `self.right` 视图，但函数体却越界修改了 `self.left`。
    fn dishonest_clear(*self)
    where outcomes write<self.right>
    {
        self.left.* = 0i32;
    }
}
```

---

## 本篇小结

通过引入内存视图，我们拥有了在不揭开函数实现黑盒的情况下、精确指代任意内存子区域的显微镜：

| 概念 / 语法 | 含义与作用 | 关键契约边界 |
| :--- | :--- | :--- |
| **字段视图 `obj.field`** | 精确指代结构体的特定成员字段区域 | 区分容器描述符字段与底层缓冲区，修改 `.right` 不连坐 `.left` |
| **元素/区间视图 `arr[i]`** | 精确指代数组中的特定元素或半开子区间 `[lo..hi]` | 建立时即锁定对象身份、世代与当时索引值，后续修改索引变量维持原状而未改已建视图目标 |
| **足迹构造 `read<V>` / `write<V>`** | 在 `outcomes` 中描述允许读取或修改的视图目标 | 属于元层规格描述，**免生运行时读取动作**，也不凭空授予未验证的解引用权限 |
| **入口与出口快照** | `requires` 与 `outcomes` 按入口状态解释；`ensures` 按出口状态解释（可用 `'old` 指代入口值） | 避免将调用后改变的长度或游标错误代入入口足迹选择器 |

### 下一篇预告

仔细看我们在 里写的那个函数 `write_distinct`：

```flurry
fn write_distinct(values: *Array<i32, n>, first: usize, second: usize) -> i32
where n: usize,
    requires first < n and second < n and first != second,
    ...
{
    values.*[first] = 11i32;
    values.*[second] = 22i32;
    values.*[first]
}
```

在这段代码里：
1. 我们先往 `values.*[first]` 写了 `11`；
2. 紧接着往 `values.*[second]` 写了 `22`；
3. 最后返回 `values.*[first]`。

为什么编译器能拍着胸脯向你保证最后返回的**一定是 `11`**（甚至可以直接把第 3 行的内存读取优化成常量 `11`）？
如果把 `Counters` 换成 `union`（联合体），两个字段名字也不同（比如 `u.as_int` 和 `u.as_float`），为什么编译器就**不承认**它们是分离的？
更刺激的是：在 Rust 里，如果你想把一个数组切成前半段 `[0..mid]` 和后半段 `[mid..n]` 同时原地修改，你禁止直接切，必须调用标准库里用 `unsafe` 原始指针底层机制包出来的 `slice.split_at_mut(mid)`。而在 Flurry 里，为什么**无需写一行 `unsafe`**，编译器自己就能证明两个视图彼此分离？

在下一篇 **《Flurry 演进手记 (050)：证明两个切片彼此分离——分离证据（Disjointness Evidence）与并行原地修改》** 中，我们将深入揭秘 Flurry 的分离性证明机制。
