# Flurry 演进手记 (056)：把动态检查变成静态事实——assumes 断言、事实版本失效与分支精化

在 [上一篇 (055)](./055-cubic-polynomial-automatic-bounds-proving.md) 中，我们依靠 `assumes` 建立的前提条件，让编译器的三次多项式算术证明器自动证明了三维数组下标 `values[(z * height + y) * width + x]` 的绝对安全。

其实从 [第 001 篇](./001-a-world-in-a-byte.md) 开始，我们就一直在写 **`assumes expr;`**。
但如果你来自 C/C++、Rust 或 Zig，看到 `assumes` 这个词时，你脑子里很可能会闪过两个完全极端的固有印象：
- **误解 A（把它当成 C 的 `assert` 或 Rust 的 `assert!`）**：“这不就是一个机械的运行时 `if (!cond) abort();` 吗？就算前面已经明明知道条件成立，它还是会老老实实生成一条比较跳转指令吧？而且它跟类型/验证系统毫无关系吧？”
- **误解 B（把它当成 LLVM 的 `llvm.assume` 或 C++23 的 `[[assume(...)]]`）**：“天哪。在 C++ 和 LLVM 里，`assume` 是告诉优化器‘别检查了，直接无条件盲信我，如果运行时不成立就直接触发未定义行为（UB）’。Flurry 的 `assumes` 不会这么危险吧？”

**两边都错了。**
在 Flurry 中，**`assumes` 既不是与静态验证脱钩的笨重断言，更不是盲目信任引发 UB 的危险指令**——它是连接“动态运行时世界”与“静态形式化证明世界”的**双向智能闸门**。

今天，我们就来彻底解剖 `assumes` 的三大核心机制：
1. **静态已证即消除（Zero-Cost Elimination），静态未知则受检发布事实（Checked Fact Publishing）**；
2. **`assumes` 能证明什么、绝对不能“无中生有”什么**；
3. **当指针目标被修改时，已发布的事实如何按状态版本立即失效（Fact Invalidation）**。

---

## 一、 运行时检查与静态证明脱节的问题

假设我们要写一个从定长数组 `values: Array<i32, n>` 读取指定下标的函数 `read_at`，以及一个对两个数做除法后再复用的函数。

在传统语言中：
- 如果你写 `assert!(index < n)`，然后再写 `values[index]`：要是优化器稍微笨一点没看穿两者的联系，就会先跑一遍你的 `assert!`，紧接着在 `values[index]` 内部又跑一遍自带的越界检查（检查了两次。）。
- 如果被检查的条件是一个带逻辑含义的业务规则（比如 `valid_index(index, n)` 或 `below_limit(source, 10)`），传统编译器只认得它是一个返回 `bool` 的普通函数调用，哪怕你写了 `assert!(below_limit(source, 10))`，下一行调用一个要求 `requires source.* < 10` 的函数时，编译器依然两手一摊：“我不认识你上一行的 `assert!`，请重新证明。”

让我们先看看在 Flurry 中，如何用 **`predicate`（程序谓词）** 配合 **`assumes`**，写出语义与证明合一的 传统-> 演进代码。

---

## 二、 `assumes` 的双重魔力——静态消除 vs 动态受检发布事实

在 Flurry 中，当你写下一条 `assumes expr;` 语句时，编译器的程序验证层会首先在当前控制流位置尝试对 `expr` 进行静态推导。推导结果只有三种可能：**已证明为真（`Proved`）**、**已证伪（`Refuted`）**、或者**静态未知（`Unknown`）**。

下面来看看 `assumes expr;` 在这三种状态下的精确行为（详见 `docs/resource-management/contracts-and-verification.md` §assumes 与运行检查）：

```text
  编译器遇到 `assumes expr;` 时的三路决策引擎：

                       [ 输入：assumes expr; ]
                                  │
                                  ▼
             ┌─────────────────────────────────────────┐
             │ 在当前状态版本与路径条件下静态求解 expr │
             └────────────────────┬────────────────────┘
                                  │
        ┌─────────────────────────┼─────────────────────────┐
        ▼ (1. Proved：静态已证真)  ▼ (2. Unknown：静态未知)   ▼ (3. Refuted：静态已证伪)
  ┌───────────────────┐   ┌───────────────────────────┐   ┌───────────────────┐
  │ 零开销直接消除    │   │ 检查 expr 是否有合法判定器│   │ ❌ 编译期直接拒绝!│
  │ 不生成任何机器码  │   │ (达到 P3 稳定观测+有限完成)│   │ (明知必败的假设)  │
  └─────────┬─────────┘   └─────────────┬─────────────┘   └───────────────────┘
            │                           │
            │              [在危险动作之前保留运行时检查!]
            │                     ├── (运行时判定为 false) ──► 走失败报告并按真实控制流清理责任。
            │                     ▼ (运行时判定为 true：成功边)
            └────────────────────►┼───────────────────────────────────────────┐
                                  │  📢 正式向后续代码发布静态事实：`expr` 为真  │
                                  │  (绑定当前来源身份与状态版本 Version #k)  │
                                  └───────────────────────────────────────────┘
```

仔细看中间那条 **`Unknown`（静态未知）** 路径。这就是为什么 Flurry 的 `assumes` 跟 LLVM 的无条件 `llvm.assume` 完全不是一回事：
1. **绝不盲信未知条件**：当 `index` 是外部传进来的未知 `usize` 时，`assumes valid_index(index, n);` **绝对不会**跳过检查直接盲信，而是会在 `values[index]` 访问之前，老老实实生成一条受检的运行时判定。
2. **一次检查，终身（当前版本内）受益**：一旦运行时检查通过、控制流走上成功边，编译器立刻把 `index < n` 作为**静态已证事实（Proved Fact）**登记到后续代码中。紧接着下一行的 `values[index]` 看到 `index < n` 已经是 `Proved`，就**不再生成任何冗余的下标检查**。如果后面紧跟着还有 `assumes index < n;` 或者需要 `requires index < n` 的调用，也统统走最左边的 `Proved` 分支，**零开销直接消除**。

同样，通过普通的 `if` / `match` 分支判断（分支精化），也能在对应分支内自动建立路径事实。

让我们在 改进代码中体验 `assumes` 与 `predicate` 的默契配合：

```flurry
use std.prelude.*;

-- 定义一个程序谓词（要求 P3 稳定观测 + 有限完成，无需写 comptime）
predicate valid_index(index: usize, length: usize) {
    index < length
}

fn quotient(numerator: u32, denominator: u32) -> result: u32
where requires denominator != 0u32,
    ensures result <= numerator
{
    numerator / denominator
}

fn read_at(values: Array<i32, n>, index: usize) -> i32
where n: usize
{
    -- 此时 n 已知但 index 静态未知：
    -- 编译器在此处保留一次运行检查；检查通过后，在成功边发布事实 `index < n`。
    assumes valid_index(index, n);

    -- 到了这一行，`index < n` 已经是静态 Proved 事实。
    -- 数组读取直接放行，不再插入第二次重复的越界检查。
    values[index]
}

fn checked_quotient(numerator: u32, denominator: u32) -> u32 {
    -- 在调用要求 `requires denominator != 0` 的函数前，用 assumes 受检兑现前提
    assumes denominator != 0u32;
    let q = quotient(numerator, denominator);

    -- 因为 quotient 的出口契约保证了 `ensures result <= numerator`，
    -- 下面这句 assumes 在编译期直接被判定为 Proved，零运行时开销直接消除。
    assumes q <= numerator;
    q
}

test {
    let arr: Array<i32, 4> = [10i32, 20i32, 30i32, 40i32];
    println("read_at: {}", read_at(arr, 2usize));
    println("quotient: {}", checked_quotient(20u32, 3u32));
}
```

运行输出：

```text
read_at: 30
quotient: 6
```

---

## 三、 状态版本与事实失效（Fact Invalidation）——修改指针后旧事实当场作废

现在，来到本篇最精彩、也最能体现 Flurry 验证器精密程度的环节。

假设我们的谓词不再是只看按值整数 `index < n`，而是通过**指针**观察一块可变内存的状态（详见 `docs/resource-management/runtime-purity.md` 示例 5）：

```flurry
predicate below_limit(source: *u32, limit: u32) {
    source.* < limit
}
```

思考下面这个场景：
1. 我们在第 1 行写了 `assumes below_limit(source, 10u32);`；
2. 此时检查通过，验证器发布了事实：`source.* < 10u32`；
3. 如果紧接着在第 2 行，我们直接读取 `source.*`（或者调用一个要求 `requires source.* < 10u32` 的函数），这个事实当然成立；
4. **但是。如果有人在第 2 行写了 `source.* = 12u32;`（或者通过另一个可能指向同一位置的别名指针改写了它），到了第 3 行，刚才发布的 `source.* < 10u32` 事实还能继续用吗？**

如果验证器机械地把 `"source.* < 10u32"` 当作一条永恒不变的数学真理记在账本上，那第 3 行就会导致严重错误。

为此，Flurry 设计了严谨的**状态版本绑定与事实失效机制（Versioned Facts & Fact Invalidation）**：

> **状态事实绑定其来源和版本。检查后若目标被修改、终结或环境改变，事实须按实际依赖失效。**

下面来看看一条关于指针目标的事实是怎么从“诞生”走向“失效”的：

```text
  指针状态事实的版本生命周期（Fact Lifecycle with State Versions）：

  [ 初始状态：source 指向存储槽 S，当前内容版本为 (假设值为 5) ]
        │
        ▼
  1. 执行 `assumes below_limit(source, 10u32);`
     ──► 运行判定器 `source.* < 10u32` (5 < 10，成功。)
     ──► 登记带版本印章的事实：Fact #1 = [ (S @ 版本 #1) < 10u32 ]
        │
        ├──► 2. 紧接着读取或验证 `require_below_10(source)`
        │       检查当前 S 的版本：依然是版本 #1。
        │       ──► 命中 Fact #1 (Proved!)，直接安全放行。
        │
        ▼
  3. 发生内存写入：`source.* = 12u32;` (或调用了含 `write<source>` 的函数)
     ──► 存储槽 S 的内容版本推进到【版本 #2】：
     ──► 💥 所有盖着旧版本印章 `@ 版本 #1` 的事实（Fact #1）瞬间全部失效作废。
        │
        ▼
  4. 再次调用 `require_below_10(source)`：
     ──► 旧事实 Fact #1 (@ 版本 #1) 已作废。当前状态 (@ 版本 #2) 下缺少 `< 10` 依据。
     ──► 若不重新检查，编译器编译期拒绝。
```

让我们用一段 进阶代码展示“带版本事实的正常使用”以及“重新赋值后重新建立事实”：

```flurry
use std.prelude.*;

predicate below_limit(source: *u32, limit: u32) {
    source.* < limit
}

fn read_bounded(source: *u32) -> result: u32
where requires source.* < 10u32,
    outcomes read<source>,
    ensures result < 10u32
{
    source.*
}

test {
    var val = 5u32;
    let ptr = val.ref;

    -- 1. 在状态版本 下建立事实：`ptr.* < 10u32`
    assumes below_limit(ptr, 10u32);

    -- 2. 因为 read_bounded 的效果只有 `outcomes read<source>`（只读，不推进版本。），
    --    所以连续调用两次 read_bounded，版本 的事实始终保持有效。
    let first = read_bounded(ptr);
    let second = read_bounded(ptr);
    println("first={}, second={}", first, second);

    -- 3. 现在修改 val，状态版本推进到 #2。之前状态版本下的 `< 10u32` 事实自动失效。
    val = 8u32;

    -- 4. 对新状态版本 重新建立事实，随后继续安全调用
    assumes below_limit(ptr, 10u32);
    let third = read_bounded(ptr);
    println("third={}", third);
}
```

运行输出：

```text
first=5, second=5
third=8
```

---

## 编译器的三道防呆铁闸：`assumes` 绝对不能做什么？（`-- ❌ 编译期直接拒绝`）

虽然 `assumes` 很强大，但它严格遵守物理与逻辑定律，**绝不是“许愿机”**。以下三种企图滥用 `assumes` 的写法，会被编译器编译期拒绝：

### 铁闸 1：已修改目标导致旧事实失效后，企图直接蒙混过关？拒绝

如果在 `assumes below_limit(source, 10u32);` 之后，你把 `source.*` 改成了 `12u32`，却不重新建立检查就想调用要求 `requires source.* < 10u32` 的函数：

```flurry
predicate below_limit(source: *u32, limit: u32) {
    source.* < limit
}

fn require_under_ten(source: *u32) -> u32
where requires source.* < 10u32
{
    source.*
}

-- ❌ 编译期直接拒绝。
-- 错误：`source.* = 12u32` 产生 write 效果，使先前版本下的 `below_limit(source, 10u32)` 事实失效。
fn bug_stale_fact(source: *u32) -> u32 {
    assumes below_limit(source, 10u32);
    source.* = 12u32;
    require_under_ten(source)
}
```

### 铁闸 2：企图在证明指针有效之前，就用 `assumes` 凭空解引用？拒绝

请牢记 `docs/resource-management/runtime-purity.md`（示例 5）的警告：
> **“读取 `source` 的前提必须先成立；`predicate` 本身不提供凭空解引用的许可。”**
> **“运行检查只能证明它实际检查的条件，不能创造所有权、恢复已移出的值，或使旧世代指针复活。”**

也就是说，如果你手里的非 `Copy` 资源 `ticket` 已经被移走（Moved）消耗掉了，你绝对不能写一句 `assumes ...` 让它起死回生：

```flurry
struct Ticket {
    id: u32,
}

fn consume(t: Ticket) {}

-- ❌ 编译期直接拒绝。
-- 错误：ticket 的所有权责任已在 consume(ticket) 中移出，
-- `assumes` 只能判定数值/状态条件，绝不能复活已移出的仿射资源。
fn cannot_resurrect_moved_value(ticket: Ticket) -> u32 {
    consume(ticket);
    assumes true;
    ticket.id
}
```

### 铁闸 3：企图在 `or`（逻辑或）之后，假装两边都成立？拒绝

如果你写的是 `assumes a < 10u32 or b < 10u32;`（逻辑或 `or`），它只证明了**“至少有一边小于 10”**，绝对不能据此推导出 `a < 10u32` 单独成立（见 `docs/resource-management/memory-effects.md` 第 611 行）。只有 `and`（逻辑与）才能同时发布两侧的事实：

```flurry
fn need_under_ten(x: u32) -> u32
where requires x < 10u32
{
    x
}

-- ❌ 编译期直接拒绝。
-- 错误：`a < 10u32 or b < 10u32` 只保证至少一侧成立，不能据此证明 `a < 10u32` 必然成立。
fn bug_or_fact(a: u32, b: u32) -> u32 {
    assumes a < 10u32 or b < 10u32;
    need_under_ten(a)
}
```

---

## 本篇小结

通过解剖 `assumes` 的前世今生，我们掌握了在 Flurry 中驾驭动态检查与静态事实的核心原则：

| 机制 / 阶段 | `assumes expr;` 的具体行为 | 关键契约与安全边界 |
| :--- | :--- | :--- |
| **静态已证明 (`Proved`)** | 编译期确知 `expr` 为真 | **零运行时开销**，检查指令被安全消除 |
| **静态未知 (`Unknown`)** | 编译器在依赖该条件的动作**之前**生成受检运行检查 | 要求 `expr` 具备合法判定器（`P3` 稳定观测 + 有限完成）；**绝不盲信未知条件导致 UB** |
| **事实发布与绑定** | 在检查成功的控制流边上，将 `expr` 发布为静态事实 | 事实严格绑定**存储来源身份**与**当前状态版本（State Version）** |
| **事实版本失效** | 目标发生 `write`、`kill` 或跨过可能修改它的调用 | 旧版本事实**立即作废**；只读访问（`read`）不推进版本，事实继续有效 |
| **`assumes` 的能力边界** | 仅发布其实际检查的数值与状态事实 | **不能**凭空授予非法指针解引用权限，**不能**复活已移动的 `Once` 资源或已终结世代的指针 |

### 下一篇预告

在今天和前几篇的讲述中，我们站在**调用方（Caller）**的视角，已经能熟练地用 `assumes` 兑现被调函数的 `where requires` 前置条件，并享受被调函数 `where ensures` 交付的出口结果事实了。

现在，让我们把镜头转过来，站在**函数编写者（Callee Author）**的视角。
假设我们要亲手为环形缓冲区编写一整套带契约的核心 API：
- `fn push(*self, item: i32) where requires ..., outcomes ..., ensures ...`
- 当我们在函数签名上写下 `where requires` 和 `where ensures` 时，它们在函数体内部是怎么被检查的？
- 如果一个递归算法（比如在环形缓冲区的有序段上做二分查找或树节点归并）需要向编译器证明**“我这个递归一定会在有限步内停下来，绝对不会死循环”**，我们该如何用 **`where decreases`** 子句给出一个良基递减度量（Termination Metric）？

在下一篇 **《Flurry 演进手记 (057)：在函数边界立下军令状——`where requires` 前置条件与 `ensures` 后置条件》** 以及随后的第 058 篇中，我们将正式开启契约式编程与形式化验证的实战之旅。
