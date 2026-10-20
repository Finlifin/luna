# Flurry 演进手记 (061)：用 Outcome Separation Logic 刻画状态转移——where outcomes 子句详解

> **系列导读**：在上一期（060）中，我们用 `predicate` 和 `invariant` 写出了第一段经过完整数学证明的 `verified fn`。但在验证涉及指针和聚合体的程序时，一个困扰了计算机科学界半个世纪的难题浮出水面——**框架问题（The Frame Problem）**：当一个函数修改了某个指针或某个字段时，我们如何向证明器保证“宇宙中其余所有的内存和兄弟字段都没有被偷偷篡改”？当函数包含多条提前退出分支或可能允许顺序别名（Sequential Aliasing）时，又该如何精确刻画它的内存足迹？今天，我们将深入 Flurry 程序验证层的核心理论基底——**结果分离逻辑（Outcome Separation Logic, OSL）**与 `where outcomes` 子句。

---

## 一、传统契约的问题：传统方案的“框架问题”与别名恐慌

假设我们在 `RingBuffer` 旁设计了一个双通道统计计数器 `ChannelPair`，记录推入通道 `left`（成功入队字节数）和通道 `right`（丢弃字节数）：

```flurry
struct ChannelPair {
    left: i32,
    right: i32,
}
```

现在我们写一个函数 `bump_right(*self, delta: i32)`，它只把 `self.right` 增加 `delta`。在只写了 `requires` 和 `ensures` 的初始写法中，看看会发生什么：

```flurry
-- 只写修改了什么，没声明“只允许修改什么”
fn bump_right_naive(*self, delta: i32) -> void
where
    requires self.right.* <= 1000i32 and delta >= 0i32 and delta <= 100i32,
    ensures self.right.* == (self.right.*)'old + delta
{
    self.right.* = self.right.* + delta;
}

fn caller(pair: *ChannelPair) -> void
where
    requires pair.left.* == 42i32 and pair.right.* == 10i32
{
    bump_right_naive(pair, 5i32);
    -- ❌ 编译期验证困境。
    -- 仅凭 bump_right_naive 的 ensures 子句，它只承诺了 pair.right 变成了 15，
    -- 但没有承诺它没碰过 pair.left。调用方原先已知的 pair.left == 42i32 被迫失效。
}
```

这就是形式化验证中常见的**框架问题（Frame Problem）**。
难道我们要逼迫程序员在 `bump_right_naive` 的 `ensures` 里加上 `ensures self.left == self.left'old` 吗？如果结构体有 30 个字段呢？每次新增一个字段，岂不是要修改所有方法的 `ensures`？

---

## 二、用 `where outcomes` 声明精细的内存脚印（Footprint）

为了根治框架问题，并同时支持多分支结果（如正常返回与错误早退）的推理，Flurry 的程序验证层采用了**结果分离逻辑（Outcome Separation Logic）**，并直接与我们在第 052 期学过的**六种基本内存效果**无缝合流。

在函数的 `where` 子句中，你可以使用 **`outcomes`** 子句声明该函数**允许产生的内存效果上界（Upper Bound）**：

```flurry
-- 用 outcomes 精确锁定字段级内存脚印
fn bump_right_checked(*self, delta: i32) -> void
where
    requires self.right.* <= 1000i32 and delta >= 0i32 and delta <= 100i32,
    outcomes read<self.right> and write<self.right>,
    ensures self.right.* == (self.right.*)'old + delta
{
    self.right.* = self.right.* + delta;
}
```

### 结果分离逻辑（OSL）的自动框架保留法则（Frame Rule）

当 `bump_right_checked` 声明了 `outcomes read<self.right> and write<self.right>` 时，Flurry 的验证器会立刻应用分离逻辑的 **Frame Rule**：

```text
[ 调用前调用方的内存状态堆 (Heap / Frame) ]
  ┌─────────────────────────┬─────────────────────────┐
  │ pair.left == 42i32      │ pair.right == 10i32     │
  │ (不在 outcomes 脚印内)   │ (命中 write<self.right>)│
  └────────────┬────────────┴────────────┬────────────┘
               │                         │
               │ 自动隔离保留 (Frame)     │ 执行 bump_right_checked(pair, 5)
               ▼                         ▼
  ┌─────────────────────────┬─────────────────────────┐
  │ pair.left == 42i32      │ pair.right == 15i32     │
  │ (100% 原封不动继承  )    │ (由 ensures 更新状态)    │
  └─────────────────────────┴─────────────────────────┘
```

1. **对被調函数内部**：编译器严格检查 `bump_right_checked` 的函数体。如果函数体里手滑写了一句 `self.left.* = 0i32;`，会产生 `write<self.left>` 效果，超出 `outcomes` 声明的上界，**编译期直接报错**。
2. **对外部调用方**：调用方看到 `outcomes` 只触碰了 `self.right`，就能 100% 确信 `pair.left`（以及调用方持有的其他不相交变量）在调用前后**绝不可能发生任何改变**，`pair.left == 42i32` 自动跨过函数调用保留下来。

---

## 三、用返回 `MemoryEffect` 的 `predicate` 复用效果模式

在日常编写 `outcomes` 时，`read<target> and write<target>` 是最常见的组合。我们在上一期（060）提到过，程序验证层的 `predicate` 不仅可以返回 `bool`，还可以显式指定返回类型为 **`MemoryEffect`**。

更妙的是，当 `predicate` 的返回类型是 `MemoryEffect` 时，它的裸标识符形参会自动成为 `quote id: lift Any` 的语法糖，可以直接填入 `read<...>` 和 `write<...>` 的尖括号里：

```flurry
-- 定义返回 MemoryEffect 的程序谓词
-- 这里的裸形参 target 等价于 quote target: lift Any
predicate modify(target) -> MemoryEffect {
    read<target> and write<target>
}

-- 在 outcomes 子句中直接复用 modify(...)
verified fn bump_right(*self, delta: i32) -> void
where
    requires self.right.* <= 1000i32 and delta >= 0i32 and delta <= 100i32,
    outcomes modify(self.right),
    ensures self.right.* == (self.right.*)'old + delta
{
    self.right.* = self.right.* + delta;
}
```

如果 alguém 在 `outcomes modify(self.right)` 的约束下试图偷改 `self.left`：

```flurry
verified fn sneaky_bump(*self, delta: i32) -> void
where
    requires self.right.* <= 1000i32 and delta >= 0i32 and delta <= 100i32,
    outcomes modify(self.right)
{
    self.right.* = self.right.* + delta;
    -- ❌ 编译期直接拒绝。
    -- 产生了未在 outcomes 中声明的 write<self.left> 内存效果。
    self.left.* = 0i32;
}
```

---

## 四、突破传统借用检查器的极限：顺序别名（Sequential Aliasing）与按需分离

在采用独占可变引用的语言（如 Rust）中，“可变引用不允许别名（No Mutable Aliasing）”是一条统一规则：哪怕函数内部只是**先写第一个指针，再读第二个指针**，只要传入的两个指针指向同一个变量，编译器也会静态拒绝。

而在 Flurry 中，依靠**代际可达性（`&r`）+ 内存效果（`outcomes`）+ 结果分离逻辑（OSL）**，指针天然支持安全的**顺序别名（Sequential Aliasing）**，只有在真正需要“两块内存互不干扰”时才通过契约按需证明分离。

让我们看两个极具代表性的对比场景：

### 场景 A：允许顺序别名（先写后读同一地址完全合法）

```flurry
fn write_then_read(first: *i32, second: *i32) -> i32
where
    outcomes write<first> and read<second>
{
    first.* = 10i32;
    second.*
}

fn demo_sequential_aliasing() -> i32 {
    var number = 1i32;
    -- ✅ 完全合法。first 和 second 指向同一个 number，
    -- 但 write_then_read 的契约并不要求它们分离，顺序执行先写 10 再读出 10 毫无悬念。
    write_then_read(number.ref, number.ref)
}
```

### 场景 B：按需证明分离（当后置条件要求两者同时保持各自独立值时）

如果一个函数要求执行完后同时满足 `first.* == 1i32 and second.* == 2i32`，那么当 `first` 和 `second` 指向同一块内存时，后置条件在数学上根本不可能成立（一个 `i32` 不可能既等于 1 又等于 2）。此时，我们只需在 `requires` 中加入分离前提 `first != second`：

```flurry
verified fn write_two_distinct(first: *i32, second: *i32) -> void
where
    requires first != second,
    outcomes write<first> and write<second>,
    ensures first.* == 1i32 and second.* == 2i32
{
    first.* = 1i32;
    -- 因为 requires 保证了 first != second（地址分离），
    -- 写入 second.* 绝不会覆盖刚才写入 first.* 的 1i32。
    second.* = 2i32;
}

fn demo_disjoint_check() -> void {
    var a = 0i32;
    var b = 0i32;
    -- ✅ a.ref 与 b.ref 是不同栈槽，静态已知 a.ref != b.ref
    write_two_distinct(a.ref, b.ref);

    -- ❌ 编译期直接拒绝。
    -- 传入相同的 a.ref 违反了 requires first != second 分离前置条件。
    write_two_distinct(a.ref, a.ref);
}
```

```text
[ Rust 的一刀切规则 ]
  只要存在两个 *i32 指向同一地址 ──► ❌ 无论你干什么，一律禁止。

[ Flurry 的 OSL 按需分离规则 ]
  ├── write_then_read(p, p)       ──► 不要求 p != q ──► ✅ 允许顺序别名。
  └── write_two_distinct(p, q)    ──► requires p != q ──► 静态证明地址分离。
```

---

## 五、在 `RingBuffer` 中实战：无伤读取与精确推入脚印

回到我们的 `RingBuffer`。当我们编写 `try_push` 时，通过 `outcomes` 子句，我们可以向所有调用者作出最严密的数学承诺：
- `peek` 只产生 `read<buf.slots>` 和 `read<buf.head>`、`read<buf.len>`，**绝不产生任何 `write` 效果**；
- `push_unchecked` 只修改 `buf.slots`、`buf.tail` 和 `buf.len`，**绝不触碰读游标 `buf.head`**。

```flurry
struct RingBuffer {
    slots: Array<i32, 8>,
    head: usize,
    tail: usize,
    len: usize,
}

predicate modify(target) -> MemoryEffect {
    read<target> and write<target>
}

verified fn push_slot(buf: *RingBuffer, value: i32) -> void
where
    requires buf.tail.* < 8usize and buf.len.* < 8usize,
    outcomes read<buf.slots> and write<buf.slots>
        and modify(buf.tail)
        and modify(buf.len),
    ensures buf.len.* == (buf.len.*)'old + 1usize,
    ensures buf.tail.* == ((buf.tail.*)'old + 1usize) % 8usize
{
    buf.slots.*[buf.tail.*] = value;
    buf.tail.* = (buf.tail.* + 1usize) % 8usize;
    buf.len.* = buf.len.* + 1usize;
}
```

正因为 `push_slot` 的 `outcomes` 没有包含 `write<buf.head>`，任何正在并发或交错读取 `buf.head` 游标统计信息的上层逻辑，都能直接由 Frame Rule 静态推导出：`push_slot` 前后 `buf.head` 保持恒定不变。

---

## 六、本节核心概念速查表

| 概念 / 语法 | 所属机制 | 核心语义与工程价值 |
| :--- | :--- | :--- |
| `where outcomes <EffectExpr>` | 函数内存效果契约 | 声明函数允许产生的内存效果上界；超出该范围的读写在编译期报错 |
| **Frame Rule（框架法则）** | 结果分离逻辑（OSL） | 不在 `outcomes` 写脚印内的内存位置与兄弟字段，在调用前后自动保持原值不变 |
| `predicate p(target) -> MemoryEffect` | 效果谓词定义 | 将常用的内存效果组合（如 `read<target> and write<target>`）封装复用；裸形参等价于 `quote target: lift Any` |
| **顺序别名 vs 按需分离** | 指针与分离逻辑 | 不强制一刀切禁止可变别名；需要互不覆盖时用 `requires p != q` 或不相交内存视图证明分离 |

---

## 七、下一期预告

现在，我们已经能够为一阶函数写出极其严密的 `requires`、`ensures` 和 `outcomes` 契约了。
但是，如果我们的 `RingBuffer` 提供了一个高阶批处理方法：
```flurry
fn drain_with(buf: *RingBuffer, callback: %c fn(i32) -> void) -> void
where
    c: Type.vars.Closure
```
当 `drain_with` 在循环中每弹出一个元素就调用一次外部传入的 `callback(item)` 时，可怕的事情发生了：**万一这个 `callback` 的闭包环境里偷偷捕获了同一个 `buf` 的指针，并在回调里调用了 `buf.reset()` 或篡改了 `buf.head`（即重入别名破坏 Reentrancy Hazard）怎么办？** 此外，当编译器面对一个未知的高阶回调时，如何保证循环计数器不会被回调破坏？

下一期 **《Flurry 演进手记 (062)：高阶回调里的资源契约——当闭包遇到内存效果与可达性验证》**，我们将揭开高阶函数、闭包身份追踪与局部变量隔离在形式化验证下的协同机制。
