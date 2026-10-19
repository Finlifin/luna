# Flurry 演进手记 (060)：写出第一段经过数学证明的代码——predicate 与 verified fn 实战

> **系列导读**：在上一期（059）中，我们厘清了 Flurry 的安全光谱：普通安全代码提供全量静态检查、显式契约证明与循环安全兜底；`unsafe` 局部接管底层责任；而 `verified fn` 则彻底禁止运行时安全兜底，要求所有契约、边界、别名与终止性全部在编译期完成数学证明。今天，我们将亲手为 `RingBuffer` 编写第一段真正的 `verified fn`，并解锁把复杂逻辑契约模块化复用的终极武器——**程序验证谓词（`predicate`）**与**循环不变量（`invariant`）**。

---

## 一、从“契约膨胀”说起：版本 `v0` 的可读性危机

假设我们要为固定容量 8 的 `RingBuffer` 编写一个 `verified fn`：批量清空（归零）从当前读游标 `head` 开始的 `count` 个槽位，并推进 `head` 游标。

在没有任何辅助抽象的 `v0` 版本里，我们把 `RingBuffer` 的所有合法状态条件（游标范围、已存元素数量范围）直接塞进了函数的 `requires` 和 `ensures` 里：

```flurry
struct RingBuffer {
    slots: [8]i32,
    head: usize,
    tail: usize,
    len: usize,
}

-- v0：把所有结构不变量硬编码在函数签名里
verified fn discard_n_v0(buf: *RingBuffer, count: usize) -> void
where
    requires buf.head < 8usize and buf.tail < 8usize and buf.len <= 8usize,
    requires (buf.head + buf.len) % 8usize == buf.tail,
    requires count <= buf.len,
    ensures buf.head < 8usize and buf.tail < 8usize and buf.len <= 8usize,
    ensures (buf.head + buf.len) % 8usize == buf.tail,
    ensures buf.len == buf.len'old - count
{
    buf.head = (buf.head + count) % 8usize;
    buf.len = buf.len - count;
}
```

看着这段签名，你立刻会发现两个严重问题：

1. **契约重复与膨胀**：几乎每一个操作 `RingBuffer` 的函数，都要在 `requires` 和 `ensures` 里重复写一遍 `buf.head < 8usize and buf.tail < 8usize and buf.len <= 8usize` 以及 `(buf.head + buf.len) % 8usize == buf.tail`。
2. **普通函数无法直接放进验证逻辑**：你可能会想：“我写一个 `fn is_valid(buf: *RingBuffer) -> bool` 不就行了吗？”但在程序验证器眼里，普通 `fn` 是一个带有执行控制流和栈帧的黑盒调用，除非把它的后置条件再写一遍，否则验证器并不能像展开逻辑公式一样透明地推理它！

我们需要一种**专门属于程序验证层、没有运行时开销、能被证明器直接作为数学命题展开**的逻辑构件。

---

## 二、迭代演进 `v1`：用 `predicate` 封装程序状态命题

在 Flurry 中，专门用于描述程序状态、内存性质与数据不变量的逻辑定义叫做 **`predicate`（程序谓词）**。

它的语法形态非常干净：

```text
predicate id(parameter*) (-> expr)? clauses? block
```

如果不写返回类型 `-> expr`，`predicate` 默认返回 `bool`。让我们用 `predicate` 把 `RingBuffer` 的结构合法性提取出来，升级到 `v1`：

```flurry
-- 定义 RingBuffer 的结构合法性谓词（默认返回 bool）
predicate valid_ring(buf: *RingBuffer) {
    buf.head < 8usize
        and buf.tail < 8usize
        and buf.len <= 8usize
        and (buf.head + buf.len) % 8usize == buf.tail
}

-- v1：用 predicate 让契约签名如水晶般清晰
verified fn discard_n_v1(buf: *RingBuffer, count: usize) -> void
where
    requires valid_ring(buf) and count <= buf.len,
    ensures valid_ring(buf),
    ensures buf.len == buf.len'old - count,
    ensures buf.head == (buf.head'old + count) % 8usize
{
    buf.head = (buf.head + count) % 8usize;
    buf.len = buf.len - count;
}
```

### ⚠️ 核心边界辨析：`predicate` vs `comptime predicate`

在阅读或编写 Flurry 代码时，有一条**绝对不能混淆的架构红线**：

| 维度 | `comptime predicate`（PTC 层） | `predicate`（程序验证层） |
| :--- | :--- | :--- |
| **所属层次** | **证明论式类型检查（PTC）** | **程序验证层（Program Verification）** |
| **处理对象** | 只处理**编译期类型命题**与编译期常量约束（如 `N > 0usize`、类型等价与 trait 约束） | 处理**运行时程序状态、内存布局、指针可达性与资源规格**（如 `buf.len <= 8usize`、`read<target>`） |
| **关键字前缀** | 必须带 `comptime` | **不加 `comptime`！** |
| **运行期开销** | 零（纯类型检查期消解） | 零（纯静态验证期消解，不生成机器码） |

```text
[ 代码源码 ]
     │
     ├──► comptime predicate ──► [ PTC 类型检查器 ] (只管类型命题与编译期常量)
     │
     └──► predicate          ──► [ 程序验证器 / OSL ] (管程序状态、内存与资源规格)
```

> 💡 **语法贴士**：除了返回 `bool` 的状态谓词外，`predicate` 还可以返回 `MemoryEffect` 来组合复杂的内存效果子句（我们将在下一期 061 详细展开）。

---

## 三、当 `verified fn` 遇上循环：为什么编译器拒绝了你的 `while`？

刚才的 `discard_n_v1` 只是修改了两个标量字段，没有循环。现在我们要实现真正的槽位清理并求和函数 `drain_and_sum`：它要在一个 `while` 循环里，逐个读取从 `head` 开始的 `count` 个元素、把原槽位清零、累加总和，并推进 `head`。

如果我们按照平时写代码的习惯，只写函数头尾的 `requires`/`ensures` 和循环里的 `decreases`：

```flurry
-- 尝试在 verified fn 中直接写循环而不给循环不变量
verified fn drain_and_sum_bad(buf: *RingBuffer, count: usize) -> total: i32
where
    requires valid_ring(buf) and count <= buf.len,
    ensures valid_ring(buf),
    ensures buf.len == buf.len'old - count
{
    var acc = 0i32;
    var step = 0usize;
    while step < count {
        decreases count - step;

        let idx = (buf.head + step) % 8usize;
        -- ❌ 编译期直接拒绝！
        -- 在循环体第 N 次迭代中，没有循环不变量证明 buf.head < 8usize 或 step <= count，
        -- 更无法在循环退出后证明 valid_ring(buf) 依然成立！
        acc = acc .+ buf.slots[idx];
        buf.slots[idx] = 0i32;
        step = step + 1usize;
    }
    buf.head = (buf.head + count) % 8usize;
    buf.len = buf.len - count;
    return acc;
}
```

为什么证明器在循环面前“失忆”了？

### Code Cartoon：数学归纳法与循环不变量（Loop Invariant）

在形式化验证中，编译器**不会**把循环展开一万次去检查（如果 `count` 是运行期参数，也根本无法展开）。相反，证明器把整个循环看作一次**数学归纳法证明**：

```text
          [ 进入循环前 (Entry) ]
        证明：requires ──► Invariant(第 0 步) 成立 (归纳奠基 Base Case)
                    │
                    ▼
        ┌───────────────────────┐
        │   假设 Invariant 成立  │
        │   且 循环条件 step < count │
        │           │           │
        │     执行一次循环体      │
        │           │           │
        │   证明 Invariant' 依然成立 (归纳递推 Inductive Step)
        └───────────┬───────────┘
                    │ 当 !(step < count) 退出循环
                    ▼
          [ 循环退出后 (Exit) ]
        已知：Invariant 成立 且 step >= count
        由此推导：函数后置条件 ensures 成立！
```

如果你不在循环体里写明 **`invariant expr;`**，证明器在进入第 $k$ 次迭代时，就会丢掉所有关于变量范围和结构合法性的上下文假设！

---

## 四、终极版本 `v2`：带 `invariant` 与 `decreases` 的完整证明

在 Flurry 中，循环不变量通过循环体开头的 **`invariant expr;`** 语句声明。你可以连续写多条 `invariant` 语句，它们共同构成归纳假设。

让我们补齐循环不变量，完成 `drain_and_sum` 的 `v2` 完整数学证明版本：

```flurry
predicate valid_ring(buf: *RingBuffer) {
    buf.head < 8usize
        and buf.tail < 8usize
        and buf.len <= 8usize
        and (buf.head + buf.len) % 8usize == buf.tail
}

-- v2：通过 predicate + invariant + decreases 实现 100% 静态数学证明
verified fn drain_and_sum(buf: *RingBuffer, count: usize) -> total: i32
where
    requires valid_ring(buf) and count <= buf.len,
    ensures valid_ring(buf),
    ensures buf.len == buf.len'old - count,
    ensures buf.head == (buf.head'old + count) % 8usize
{
    var acc = 0i32;
    var step = 0usize;

    while step < count {
        -- 1. 归纳不变量：刻画每一次迭代前后都恒成立的状态
        invariant step <= count;
        invariant count <= buf.len;
        invariant valid_ring(buf);
        invariant buf.head == buf.head'old and buf.len == buf.len'old;
        -- 2. 终止度量：严格递减且有下界 0
        decreases count - step;

        -- 证明器推导：因为 (buf.head + step) % 8usize 的除数是 8usize > 0，
        -- 所以余数 idx 恒满足 0 <= idx < 8usize，slots[idx] 100% 静态无越界！
        let idx = (buf.head + step) % 8usize;
        acc = acc .+ buf.slots[idx];
        buf.slots[idx] = 0i32;
        step = step + 1usize;
    }

    -- 循环结束时：已知 step <= count 且 !(step < count)，必有 step == count！
    buf.head = (buf.head + count) % 8usize;
    buf.len = buf.len - count;
    return acc;
}
```

看看这段 `v2` 代码是如何严丝合缝地通过编译器验证的：
1. **进入循环前**：由 `requires` 可知 `0 <= count <= buf.len` 且 `valid_ring(buf)` 成立，全部 4 条 `invariant` 在 `step = 0usize` 时自动得证。
2. **循环体内**：`idx = (buf.head + step) % 8usize` 的结果天然在 `[0, 8)` 区间内，因此 `buf.slots[idx]` 的读写不需要任何运行期边界检查指令；修改 `buf.slots[idx]` 不改变 `head`、`tail` 和 `len`，因此 `valid_ring(buf)` 在迭代末尾依然保持成立；`acc .+ buf.slots[idx]` 使用显式回绕加法，天然无算术溢出风险。
3. **退出循环后**：`step <= count` 结合退出条件 `not (step < count)` 推出 `step == count`。随后更新 `buf.head` 和 `buf.len`，由于 `(buf.head'old + count + (buf.len'old - count)) % 8usize == (buf.head'old + buf.len'old) % 8usize == buf.tail`，后置条件 `valid_ring(buf)` 完美得证！

---

## 五、本节核心概念速查表

| 语法构件 | 适用位置 | 核心作用 |
| :--- | :--- | :--- |
| `predicate name(...) { ... }` | 顶层声明 | 定义程序验证层的逻辑谓词（默认返回 `bool`），不可加 `comptime` |
| `invariant expr;` | 循环体（`while` / `for` / `loop`）内部 | 声明循环归纳不变量，证明器在进入循环前验证奠基，在每次迭代末验证保持 |
| `decreases expr;` | 循环体内部或函数 `where` 子句 | 声明良基严格递减度量，证明循环或递归必定在有限步内终止 |
| `verified fn ...` | 函数定义 | 开启全静态形式化验证模式，零运行时安全兜底开销 |

---

## 六、下一期预告

在上面的 `drain_and_sum` 中，细心的你可能注意到了循环不变量里的这一行：
```flurry
invariant buf.head == buf.head'old and buf.len == buf.len'old;
```
为什么我们在循环里明明只写了 `buf.slots[idx] = 0i32`，却还要告诉证明器 `buf.head` 和 `buf.len` 没有变？如果一个结构体有 20 个字段，或者函数同时接收了两个指针 `first: *i32` 和 `second: *i32`，我们岂不是要写满屏的 `x == x'old`（即著名的**框架问题 Frame Problem**）？更进一步，如果函数可能中途返回错误 `!RingError`，正常分支和错误分支修改的内存状态截然不同，又该如何精确刻画？

下一期 **《Flurry 演进手记 (061)：用 Outcome Separation Logic 刻画状态转移——where outcomes 子句详解》**，我们将揭开 Flurry 形式化验证引擎的皇冠明珠——**结果分离逻辑（Outcome Separation Logic）**与 `where outcomes` 内存脚印契约！
