# Flurry 演进手记 (054)：pure fn 的真正含义——稳定观测、局部可变性与编译器合法优化前提

在 [上一篇 (053)](./053-six-tiers-of-runtime-purity-p0-to-p5.md) 中，我们攀登了 Flurry 的运行时纯度六层阶梯（`P0` 到 `P5`），并知道了在函数定义前写下 **`pure fn`** 时，向编译器和调用者许下的是 **`P4`（值纯，Value Pure）** 承诺：
> **函数的结果与完成行为，只由显式输入的值语义和不可变常量决定；不依赖外部可变存储版本，也不产生外部可观察的副作用或新资源身份。**

但是，很多从纯函数式语言（如 Haskell）或传统 `const fn` 过来的程序员，一听到“纯函数”，就会不由自主地产生一种“苦行僧式”的恐惧：
- “写了 `pure fn`，是不是函数里面就不能用 `var` 循环变量了？必须把所有循环都改写成拗口的尾递归？”
- “是不是里面绝对不能出现 `.ref` 取地址和 `.*` 解引用写回？”
- “是不是连 `word'bitvec[offset..(offset + 4)] = 0b1010;` 这种原地修改半字节的硬件位操作，或者调一个高阶闭包 `apply(action, false)`，也统统被禁止？”

如果你真这么想，那你就太低估 Flurry 的系统编程基因了。

在 Flurry 中，**纯度（Purity）衡量的是一个“计算边界（Computation Boundary）”对外界可观察的行为，而不是剥夺你在边界内部使用命令式局部变量和局部指针的权利。**

今天，我们就通过三次重构演进，看看在 `pure fn` 的铁幕内部，你究竟拥有多么惊人的命令式自由度。

---

## 一、 纯函数与局部可变状态

假设我们要为环形缓冲区的帧校验模块写一个 `pure fn`：给定一个 16 位控制字 `word: u16` 和一个动态位偏移 `offset: u8`（满足 `offset <= 12`），把从 `offset` 开始的 4 个比特改写成 `0b1010`，再统计一下里面有多少个置位比特，或者用局部指针更新一个标志位。

如果要求纯函数内部完全不能出现局部可变变量和局部指针，那么许多直观的局部状态计算就难以直接表达。
但在 Flurry 中，请看 `docs/resource-management/runtime-purity.md`（§计算边界）给出的核心定理：

> **本次执行中新建、没有外部别名、没有逃逸身份的私有临时存储，可以在边界摘要中隐藏。因此实现不需要因为使用了局部 `var` 就失去纯性。**

下面来看清“外部可观察世界”与“`pure fn` 内部私有计算边界 B”的楚河汉界：

```text
  pure fn 的计算边界 B（对外承诺 P4 值纯）：

  外部调用者世界               ┌──────────────── 内部私有计算边界 B ────────────────┐
  (Caller World)               │                                                    │
                               │  1. 局部可变槽：`var flag = false;`                │
  按值传入显式输入 ───────────►│  2. 局部指针链：`let p = flag.ref; let slot = p.ref;`│
  (word: u16, offset: u8)      │     二级解引用写回：`slot.*.* = true;`  (✅ 合法!) │
                               │  3. 私有位段写回：                                 │
  ❌ 严禁读写外部传入的指针    │     `word'bitvec[offset..(offset + 4)] = 0b1010;`  │
  ❌ 严禁局部地址逃逸出边界    │     (修改的是按值传入的私有副本，✅ 合法!)         │
  ❌ 严禁产生 I/O 或新外部身份!│  4. 局部 while 循环与私有闭包环境更新 (✅ 合法!)   │
                               │                                                    │
  ◄────── 交付确定性结果 ──────┴────────────────────────────────────────────────────┘
```

看到了吗？只要你的指针 `p` 和二级指针 `slot` 指向的始终是**本次调用内部新建的私有局部变量 `flag`**（并且没有把 `flag.ref` 逃逸出去），哪怕你在里面绕上两级指针做 `slot.*.* = true;`，对外部调用者来说，这依然是一个 100% 纯净、零外部副作用的 `P4` 计算。
同样，按值传入的参数 `word: u16` 在函数内部也是属于本次调用的私有存储，直接对它做 `word'bitvec[offset..(offset + 4)] = 0b1010;` 位视图写回，完全合法。

让我们直接写出这段酣畅淋漓的 改进代码（源自 `docs/resource-management/runtime-purity.md` 与 `docs/functions/contracts-and-effects.md`）：

```flurry
use std.prelude.*;

typealias Word = u16;
typealias Byte = u8;

-- 1. 在 pure fn 内部使用局部指针与两级解引用写回私有存储。
pure fn update_private() -> bool {
    var flag = false;
    let p = flag.ref;
    let slot = p.ref;
    -- p 和 slot 都指向本次调用的私有存储 flag，写回对外部完全不可见。
    slot.*.* = true;
    flag
}

-- 2. 在 pure fn 内部对按值参数的 BitVec 视图做动态切片原位写回。
pure fn insert_nibble(word: Word, offset: Byte) -> Word
where requires offset <= 12u8
{
    word'bitvec[offset..(offset + 4u8)] = 0b1010;
    word
}

-- 因为 insert_nibble 有限完成且满足 P4（自然也满足 P3），它甚至可以直接用作成默认参数。
fn take_word(.word: Word = insert_nibble(0x1234u16, 4u8)) -> Word {
    word
}

test {
    println("update_private: {}", update_private());
    println("take_word: {}", take_word());
}
```

运行输出：

```text
update_private: true
take_word: 4772
```

（注：`0x1234` 的第 `4..8` 位原本是 `0x3`，被替换成 `0xA`（`0b1010`）后变成了 `0x12A4`，十进制正是 `4772`。）

---

## 二、 在 `pure fn` 中调用高阶回调、Trait 方法与拥有型局部闭包

现在，你可能会进一步追问：
> “如果我在 `pure fn` 里面，需要调用一个 Trait 方法 `value.flip()`，或者调用一个高阶函数 `apply(callback, false)`，甚至构造一个捕获了局部 `var flag` 并每次翻转它的有状态闭包 `let action = || do { flag = not flag; flag };`，编译器还能证明整个函数是 `pure fn`（`P4`）吗？”

在很多语言里，`const fn` 或纯函数一旦遇到函数指针、闭包或高阶调用就直接投降报错了。
但在 Flurry 中，编译器的纯度分析器拥有强大的**代码身份追踪（Code Identity Tracking）**与**私有闭包环境验证**能力。

让我们分三种情况看看 是如何优雅通关的（详见 `docs/functions/contracts-and-effects.md`）：

1. **Trait 接口中的 `pure fn` 承诺**：
   当你在 Trait 定义里写下 `trait Flip { pure fn flip(self) -> Self; }` 时，这个接口本身就许下了 `P4` 承诺。任何实现该 Trait 的类型都必须兑现 `P4`；因此泛型函数 `pure fn flipped(value: T) -> T where T :- Flip` 无需知道 `T` 具体是谁，就能直接调用 `value.flip()` 并保持 `pure`。
2. **已知代码身份的高阶调用与循环**：
   哪怕高阶辅助函数 `fn apply(callback: fn(bool) -> bool, value: bool) -> bool` 签名上**没有**写 `pure`（因为它对未知的任意回调不承诺纯度），只要你在 `pure fn` 里传给它的实参是已知纯洁的 `invert` 或 `|v| not v`（甚至是 `if flag { keep } else { invert }` 这样所有分支候选都满足 `P4` 的选择结果），编译器就会把实际代码身份代入 `apply` 完成 `P4` 证明。
3. **拥有私有可变捕获槽的局部闭包**：
   当你在 `pure fn toggle()` 内部声明 `var flag = false;`，并构造一个按值拥有 `flag` 捕获槽的闭包 `let action = || do { flag = not flag; flag };` 传给 `twice(action)` 时：
   `action` 修改的是它**自己拥有的私有捕获环境槽位**，既没有修改外层的 `flag`，也没有触碰调用者世界。因此，连这种带内部状态翻转的局部闭包调用，都能稳稳通过 `pure fn` 的 `P4` 检查。

让我们把这三种高阶能力写进 进阶代码中：

```flurry
use std.prelude.*;

-- 1. Trait 接口承诺 pure fn (P4)
trait Flip {
    pure fn flip(self) -> Self;
}

impl Flip for bool {
    -- 实现即使省略 pure 关键字，也必须兑现接口的 P4 承诺
    fn flip(self) -> Self {
        not self
    }
}

pure fn flipped(value: T) -> T
where T :- Flip
{
    value.flip()
}

-- 2. 普通高阶函数：配合已知纯回调 候选，在 pure fn 中安全调用。
fn keep(value: bool) -> bool { value }
fn invert(value: bool) -> bool { not value }
fn apply(callback: fn(bool) -> bool, value: bool) -> bool {
    callback(value)
}

pure fn select_and_apply(flag: bool) -> bool {
    -- 两个分支候选 keep 与 invert 都满足 P4，合并后依然满足 P4。
    let action = if flag { keep } else { invert }
    apply(action, flag)
}

-- 3. 局部闭包更新自己私有的按值捕获槽位，对外依然是 P4 值纯。
fn twice(callback: %{flag: bool} fn() -> bool) -> bool {
    let first = callback();
    let second = callback();
    first and not second
}

pure fn toggle_in_pure() -> bool {
    var flag = false;
    let action = || do {
        flag = not flag;
        flag
    }
    -- action 翻转的是闭包自己拥有的捕获槽，外层 flag 依然是 false。
    twice(action) and not flag
}

test {
    println("flipped: {}", flipped(false));
    println("select_and_apply: {}", select_and_apply(true));
    println("toggle_in_pure: {}", toggle_in_pure());
}
```

运行输出：

```text
flipped: true
select_and_apply: true
toggle_in_pure: true
```

---

## 编译器的静态检查：企图用局部变量“洗白”外部指针？编译期拒绝

既然 `pure fn` 允许对局部私有变量取指针 `let p = flag.ref;` 并读写，那么如果有人试图把**外部传入的指针 `ext_ptr: *bool`** 先复制给一个局部变量 `let local_p = ext_ptr;`，甚至用条件分支 `let q = if cond { flag.ref } else { ext_ptr };` 混在一起，再去读写 `q.*`，能不能骗过编译器，假装自己读写的是私有存储？

**绝对不可能。**
正如 `docs/resource-management/runtime-purity.md`（§计算边界）所明确规定的：
> **“把外部指针复制到局部变量不会使其 pointee 私有……任何赋值引入未知或外部来源后，该绑定不再具有私有依据，后续分支也不能凭一次赋值恢复它。”**

让我们看看编译器是如何当场戳穿这种“指针洗白”把戏的：

```flurry
-- ❌ 编译期直接拒绝。
-- 错误：虽然 `p` 是局部变量，但它保存的来源合并了外部指针 `external`，
-- 其指向的存储（pointee）不属于本次调用的私有存储，严禁在 pure fn (P4) 中读写。
pure fn cannot_launder_external_pointer(cond: bool, external: *bool) -> bool {
    var local_flag = false;
    let p = if cond { local_flag.ref } else { external }
    p.* = true;
    p.*
}
```

同样地，如果你在 `pure fn` 里接收了一个未知的函数指针参数 `unknown_cb: fn(bool) -> bool`（签名既没有 `pure` 也没有具体代码身份证据）并试图调用它，编译器也会立即拒绝：

```flurry
-- ❌ 编译期直接拒绝。
-- 错误：形参 unknown_cb 是来源未知的一般函数 (P0)，没有 P4 纯度保证，不能在 pure fn 中直接调用。
pure fn cannot_call_unknown_callback(unknown_cb: fn(bool) -> bool) -> bool {
    unknown_cb(true)
}
```

---

## 本篇小结

通过本篇的深入剖析，我们对 `pure fn` 建立起了既严谨又极其符合系统编程直觉的心智模型：

| 误区 vs 真相 | 传统刻板印象 | Flurry 中 `pure fn` (`P4`) 的真实规则 |
| :--- | :--- | :--- |
| **局部变量与循环** | “纯函数不能有 `var` 和 `while`” | **完全允许。** 未逃逸的私有局部 `var` 与 `while` 循环对外不可观察 |
| **指针与位段写回** | “纯函数不能有 `.ref`、`.*` 和位段赋值” | **只要目标是私有存储就允许。** 支持局部二级指针 `slot.*.* = true` 及按值参数 `word'bitvec[...] = ...` |
| **高阶回调与闭包** | “纯函数不能调闭包或普通高阶函数” | **追踪真实代码候选。** 只要所有可达回调候选均满足 `P4`、且闭包只更新自有私有捕获槽，即可通过验证 |
| **外部指针与未知回调** | 容易因浅层 `const` 漏检 | **`-- ❌ 编译期直接拒绝。`** 严禁读写外部可变 pointee（即使赋给局部指针变量或经分支合并），严禁调用无纯度依据的未知回调 |

### 下一篇预告

回头看我们在本篇 里写的那个 `insert_nibble` 函数：

```flurry
pure fn insert_nibble(word: Word, offset: Byte) -> Word
where requires offset <= 12u8
{
    word'bitvec[offset..(offset + 4u8)] = 0b1010;
    word
}
```

注意看。在 `requires offset <= 12u8` 的前提下，编译器是怎么自动算清楚 **`offset + 4u8 <= 16u8`**（不会超过 `u16` 的 16 位宽度，且 `offset + 4u8` 不会发生 `u8` 加法溢出），从而让你不需要写一行运行时边界检查、也不需要写 `unsafe` 的？

再进一步：当我们在传感器分析仪里查询一个**三维校准查找表** `values: Array<i32, n>`（其中维度是 `width`、`height`、`depth`，坐标是 `(x, y, z)`），计算一维平铺下标：

```flurry
let row = z * height + y;
let offset = row * width + x;
values[offset]
```

展开这个下标式子，它是 **`(z * height + y) * width + x`** —— 这是一个包含三个变量相乘的 **三次多元多项式（Cubic Multivariate Polynomial）**。
在 Rust 或 Zig 里，面对这种三维平铺下标 `values[(z * height + y) * width + x]`，编译器根本算不清它到底小不小于数组长度 `n`，只能在每次读取时硬生生插入一条运行时 `bounds check` 分支指令（或者逼你写 `get_unchecked` 的 `unsafe` 代码）。

而在 Flurry 的默认 `safe` 模式下，**编译器竟然内置了一个能自动证明“总次数不超过三次（$\le 3$）的多元多项式不等式”的算术验证器**。
它是怎么工作的？为什么我们在算 `width * height` 之前，必须先写一句 `assumes height <= n / width;`？如果遇到四次多项式 `a * b * c * d <= limit` 超出了三次预算，又该如何优雅化解？

在下一篇 **《Flurry 演进手记 (055)：当编译器能自动算清下标——三次以内多元多项式的自动算术安全证明》** 中，我们将介绍 Flurry 的自动算术安全证明机制。
