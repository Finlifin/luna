# Flurry 演进手记 (016)：不用移位和掩码怎么写位操作——BitVec<N> 拼接 ++、切片与原位回写

在 [上一篇 (015)](./015-cross-element-bitvec-patterns-and-utf8.md) 中，我们学会了用跨元素 `BitVec` 模式把连续的字节流像拉链一样拆开。
但协议栈从来不是单行道。当我们从传感器读出温度和状态后，还得把它们**打包成 16 位上行报文头**发给主控芯片，在遇到不同端序的外设时做**字节序翻转（Swap Bytes）**，计算**8 位奇偶/异或校验掩码**，甚至直接修改硬件配置寄存器里第 3 到第 7 位的采样倍率！

如果在 C、C++ 或 Rust 里，你现在已经开始熟练地敲下 `(kind << 13) | (flags << 8) | length` 和 `reg = (reg & ~(0xF << 3)) | ((val & 0xF) << 3)` 了。
但在第一篇中我们就发现：**Flurry 根本没有给整数和 `BitVec` 提供 `<<`、`>>`、`&`、`|`、`^`、`~` 这些传统位运算符！**

不用移位和掩码，怎么写位操作？而且还要写得比移位更不容易出错、生成的机器码一样快？
这一篇，我们就来揭开 **BitVec 代数（BitVec Algebra）与位置原位回写** 的面纱（详见 `docs/program-synthesis/bitvec-algebra.md` 与 `docs/resource-management/bindings.md`）。

---

## 第一版：传统移位掩码的“算术模拟”之痛（v0）

假设我们要组装一个 16 位的传感器上行报文头（`u16`）：
- 高 3 位（Bit 13..16）：报文类型 `kind`（`u3`）
- 中间 5 位（Bit 8..13）：状态标志 `flags`（`u5`）
- 低 8 位（Bit 0..8）：有效载荷长度 `length`（`u8`）

如果有人执意不用 `BitVec` 代数，非要用整数乘法和加法去“模拟”移位拼接，写出来的 `v0` 会是这样一场灾难：

```flurry
use std.prelude.*;

-- v0：试图用整数算术模拟移位拼接（脆弱且反直觉！）
fn pack_header_v0(kind: u3, flags: u5, length: u8) -> u16 {
    let k = kind'bitvec.as(u16);
    let f = flags'bitvec.as(u16);
    let l = length'bitvec.as(u16);

    -- 2^13 = 8192, 2^8 = 256……不仅容易算错，还得时刻提防算术溢出！
    k * 8192u16 + f * 256u16 + l
}
```

让我们画一张 **Code Cartoon**，看看为什么用整数移位/算术去拼拆比特是一条充满陷阱的路：

```text
  传统语言的移位与掩码心智负担：
  ( (kind as u16) << 13 ) | ( (flags as u16) << 8 ) | (length as u16)
                     ▲                          ▲
          1. 偏移量全靠手算 (8 + 5 = 13)，改一个字段宽度全盘重算！
          2. 如果忘了先扩宽成 u16 就左移，直接触发位移溢出 (UB 或截断)！
          3. 从 16 位变窄回 u8 时，`word as u8` 静默截断高位，丢了哪几位全凭默契！
```

我们需要一种**直接描述“哪几段比特按什么顺序拼在一起、哪一段比特从哪里切出来”**的代数语言。

---

## 第二版：拼接 `++`、切片 `[lo..hi]` 与宽度安全转换（v1）

在 Flurry 中，`BitVec<N>` 拥有一套正交的位向量代数。最重要的三个基础积木是：
1. **拼接 `a ++ b`**：把宽度为 $m$ 的 `a` 放在**高端（MSB 侧）**，宽度为 $n$ 的 `b` 放在**低端（LSB 侧）**，直接生成宽度为 `BitVec<m + n>` 的新位向量！
2. **切片 `bits[lo..hi]`**：以索引 `0` 为最低有效位（LSB），切出半开区间 `[lo, hi)`，生成宽度为 `BitVec<hi - lo>` 的位向量。单比特索引 `bits[i]` 则生成 `BitVec<1>`。
3. **显式转换 `.as(T)`**：将位向量转换为目标类型（如 `u16`），或者向更宽的 `BitVec<M>` / `uM`（$M \ge N$）高位补零扩宽。

来看看用 `++` 和切片重写后的 `v1` 报文头组装与字节序翻转：

```flurry
use std.prelude.*;

-- 组装 16 位报文头：3 + 5 + 8 = 16 bits，从高到低一目了然！
pure fn pack_header(kind: u3, flags: u5, length: u8) -> u16 {
    (kind'bitvec ++ flags'bitvec ++ length'bitvec).as(u16)
}

-- 16 位字节序翻转（Swap Bytes）：把原低 8 位拼到高 8 位前面！
pure fn swap_bytes(word: u16) -> u16 {
    let bits = word'bitvec;
    (bits[0..8] ++ bits[8..16]).as(u16)
}

-- 从 16 位字中只取低字节
pure fn low_byte(word: u16) -> u8 {
    word'bitvec[0..8].as(u8)
}

test {
    let header = pack_header(5, 17, 42);
    println("packed: {}", header);
    println("swapped: {}", swap_bytes(0x1234));
    println("low byte: {}", low_byte(0x1234));
}
```

看看 `pack_header`：
`kind'bitvec ++ flags'bitvec ++ length'bitvec` 的顺序，和我们在第一篇学的位向量模式 `0b(kind: u3) (flags: u5) (length: u8)` **完全一致（左高右低）**！

```text
  位向量拼接 `a ++ b` 与切片 `bits[lo..hi]` 的坐标法则：

  pack_header(5, 17, 42):
    kind'bitvec (3b)  ++  flags'bitvec (5b)  ++  length'bitvec (8b)
   ┌─────────────────┐   ┌──────────────────┐   ┌──────────────────┐
   │      101        │   │      10001       │   │     00101010     │
   └────────┬────────┘   └────────┬─────────┘   └────────┬─────────┘
            └─────────────────────┼──────────────────────┘
                                  ▼
   BitVec<16>:  [ 1  0  1 | 1  0  0  0  1 | 0  0  1  0  1  0  1  0 ]
                 MSB (15)                  Bit 8             LSB (0)
                  ◄────────── bits[8..16] ──►◄──── bits[0..8] ────►
```

### 动态起点，编译期固定宽度！

你可能会好奇：在 `bits[lo..hi]` 中，`lo` 和 `hi` 必须是编译期常量吗？
假设我们要写一个滑动窗口巡检函数，根据运行时的通道偏移 `start: usize`，从 16 位状态字里提取连续的 **4 个比特**。在 Flurry 里居然可以直接这样写：

```flurry
fn extract_window(word: u16, start: usize) -> BitVec<4> {
    -- 先声明契约：保证 start + 4 不会越过 16 位边界，且加法不溢出
    assumes start <= 12usize;
    word'bitvec[start..(start + 4usize)]
}
```

> **💡 核心心智模型：起点可以来自 Runtime，差值必须在 Comptime 确定！**
> 为什么 `word'bitvec[start..(start + 4usize)]` 的返回类型能静态确定为 `BitVec<4>`？
> 因为在 Flurry 的 `bits[lo..hi]` 规则中，编译器只要求**区间宽度 `hi - lo` 在编译期可确定**（这里 `(start + 4usize) - start` 恒等于 `4`），而起点 `start` 完全可以是运行时的动态变量！

### `.as(T)` 的宽度安全铁律：严禁静默截断！

在 C 或 Rust 中，把 `u16` 强转成 `u8`（`word as u8`）会悄无声息地砍掉高 8 位。
而在 Flurry 中，`.as(T)` 遵守极其严格的宽度契约：
- **允许同宽重解释或向更宽类型扩宽**（例如 `BitVec<11>` 可以直接 `.as(u32)`，自动在高位补 21 个零）。
- **严禁直接向更窄的 `BitVec` 或无符号整数转换！** 如果你想丢弃某些位，**必须先用切片 `[lo..hi]` 显式指明保留哪几位**，再 `.as(T)`！

```flurry
-- ❌ 编译期直接拒绝！不能把 16 位直接窄化为 u8（拒绝静默截断！）
fn bad_narrow(bits: BitVec<16>) -> u8 {
    bits.as(u8)
}

-- ❌ 编译期直接拒绝！字面量 16 (0b10000) 超出了 BitVec<4> (0..=15) 的表示范围！
fn bad_literal() -> BitVec<4> {
    let b: BitVec<4> = 16;
    b
}
```

---

## 第三版：逐位逻辑 `and` / `or` / `not` 与“绝不短路”法则（v2）

除了拼接和切片，我们还需要对同宽的 `BitVec<N>` 做逐位布尔组合（比如用掩码过滤标志位，或者计算两个字节的按位异或 XOR）。

在 Flurry 中，逐位与、或、非直接复用关键字 **`and`**、**`or`**、**`not`**：
- `a and b`、`a or b`：要求两侧具有**完全相同的宽度 `N`**，结果仍为 `BitVec<N>`。
- `not a`：只翻转这 `N` 个有效位（编译器会自动清除底层存储字节里多余的高位填充），结果为 `BitVec<N>`。
- 需要异或（XOR）时，直接用基础代数组合：`(a and (not b)) or ((not a) and b)`。编译器后端的位向量合成器（Program Synthesis）会自动识别这种代数结构并将其合成为目标 CPU 的单条 `xor` 指令！

```flurry
-- ❌ 编译期直接拒绝！BitVec<4> 与 BitVec<8> 宽度不同，禁止直接 and！
fn bad_mask(left: BitVec<4>, right: BitVec<8>) -> BitVec<8> {
    left and right
}
```

### 关键陷阱排查：为什么 `BitVec` 上的 `and` / `or` 绝不短路？

停下来想一个问题：在 `bool` 类型上，`false and side_effect()` 会发生**短路（Short-circuit）**，右边的 `side_effect()` 根本不会执行。
那么在 `BitVec<8>` 上，如果我们写了 `0x00 and read_hardware_fifo()`，既然左边全是 `0`，按位与的结果注定是 `0x00`，编译器会不会也把右边的 `read_hardware_fifo()` 给短路跳过？

**绝对不会！** 来看看语言规范给出的铁律：

```flurry
use std.prelude.*;

fn read_hardware_fifo() -> BitVec<8> {
    println("FIFO popped 1 byte!");
    0xAF
}

test {
    let zero: BitVec<8> = 0x00;
    -- 在 BitVec 上，and 和 or 是普通二元操作：左右两侧各求值一次！
    let masked = zero and read_hardware_fifo();
    println("masked result: {}", masked.as(u8));
}
```

运行这段代码，控制台会雷打不动地依次输出：
```text
FIFO popped 1 byte!
masked result: 0
```

> **🔍 语法与语义小贴士：`bool` 短路 vs `BitVec` 双侧求值**
> 为什么 `BitVec` 的 `and` / `or` 不短路？
> 因为在 `BitVec<N>` 中，每一位都可能有的是 `0`、有的是 `1`，它是**数据流逐位组合算子**，而不是控制流分支判断！即使编译器在优化阶段发现 `zero and x` 的结果位恒为 `0`，**也绝不能因此删除右侧操作数的求值与可观察副作用**（比如从硬件 FIFO 弹出一个字节）。

---

## 第四版：值快照 vs 位置原位回写（In-place Writeback）（v3）

现在来到本篇最精彩的部分：**如何优雅地修改寄存器或变量里的某几个比特？**

在传统语言里，你想把 16 位配置字 `word` 的低 8 位改成 `0xAB`，或者把第 3 到第 7 位（4 个比特）的增益系数加 `1`，你必须先读出整个 `word`，自己算掩码清零那几位，再把新值移位或进去，最后写回 `word`。

在 Flurry 中，`expr'bitvec` 视图有两种截然不同的角色：
1. **当它出现在右值（读取值）位置时**：`let snapshot = word'bitvec;` 拿到的是当前时刻的一份**独立位值快照（Value Snapshot）**。此后无论 `word` 怎么变，`snapshot` 纹丝不动。
2. **当它直接作用在可写位置（左值）并接上切片 `[lo..hi]` 时**：`word'bitvec[0..8] = 0xAB;` 直接构成**位置原位回写（In-place Location Writeback）**！

让我们画一张 **Code Cartoon**，对比“值快照”与“原位回写”：

```text
  var word: u16 = 0x1234;

  [1] 值读取 (Snapshot):
      let snapshot = word'bitvec;
      ┌───────────────────┐          ┌───────────────────┐
      │ word:     0x1234  │ ──复制──► │ snapshot: 0x1234  │ (独立值！与 word 脱钩)
      └───────────────────┘          └───────────────────┘

  [2] 位置原位回写 (In-place Writeback):
      word'bitvec[0..8] = 0xAB;
      ┌─────────┬─────────┐
      │ High 8b │ Low 8b  │  word 的物理存储槽位
      │  0x12   │  0x34   │
      └─────────┴────▲────┘
      (保持不变!)     └─── 直接把 8 位宽的 0xAB 写入 [0..8] 位段！
                          word 变为 0x12AB，而 snapshot 依然是 0x1234！
```

更酷的是，不仅普通的 `=` 赋值支持位段原位回写，连 **`+=`、`-=`、`*=` 等复合赋值** 也能直接作用在 `BitVec` 子视图上（详见 `docs/resource-management/bindings.md`）！

来看看最终的 `v3` 寄存器位段更新实战：

```flurry
use std.prelude.*;

test {
    var reg: u16 = 0x1234;

    -- 1. 先拍一张值快照
    let snapshot = reg'bitvec;

    -- 2. 直接原位替换低 8 位 (0x34 -> 0xAB)
    reg'bitvec[0..8] = 0xAB;
    println("reg: {}, snapshot: {}", reg, snapshot.as(u16));
    -- 输出: reg: 4779 (0x12AB), snapshot: 4660 (0x1234)

    -- 3. 重叠位段自拷贝：把当前的 [0..8] (0xAB) 写进中间的 [4..12]！
    -- 编译器保证：先完整求出右侧的值 (0xAB)，再提交左侧更新，绝不边写边污染！
    reg'bitvec[4..12] = reg'bitvec[0..8];
    println("after overlap copy: {}", reg);
    -- 输出: 6843 (0x1ABB)

    -- 4. 对第 8..12 位的 4 比特增益字段直接做复合赋值 += 1！
    -- 右侧的 `1u4` 按算术协议与旧位段相加，再原位写回 [8..12]！
    reg'bitvec[8..12] += 1u4;
    println("after gain bump: {}", reg);
}
```

仔细看第 3 步和第 4 步：
- **重叠更新安全**：在 `reg'bitvec[4..12] = reg'bitvec[0..8];` 中，`[4..12]` 和 `[0..8]` 共享了中间的 `[4..8]` 四位！但你完全不用担心写坏数据，因为 Flurry 规定**必须先取得完整的右侧值，再提交位置更新**。
- **位段复合赋值 `+=`**：当你写 `reg'bitvec[8..12] += 1u4;` 时，编译器会先固定 `reg` 的位置、计算并保存范围 `8..12`，再读出那 4 位、加上右侧操作数、最后原位写回那 4 位，其余 12 个比特秋毫无犯！

---

## 本篇小结

回顾我们从移位掩码到 `BitVec` 代数与原位回写的演进：

| 版本 | 做法 | 暴露的问题 / 收获的改进 |
| :--- | :--- | :--- |
| **v0** | 整数乘法/加法或传统移位掩码模拟 | 偏移量靠手算，宽度变动牵一发动全身，容易发生溢出或静默截断 |
| **v1** | `a ++ b` 拼接、`bits[lo..hi]` 切片与 `.as(T)` | 左高右低自然拼接；支持动态起点+编译期定宽切片；严禁向更窄类型直接 `.as` 截断 |
| **v2** | `BitVec` 上的 `and` / `or` / `not` | 强制同宽逐位运算；明确双侧各求值一次（不短路），保留输入副作用；后端自动合成最优指令 |
| **v3** | `reg'bitvec[lo..hi] = ...` 与 `+=` 原位回写 | 严格区分独立的值快照与位置原位更新；无需手写读-改-写掩码即可精准修改任意位段 |

---

### 留给下一篇的悬念

现在，无论是单个字节、跨字节比特流，还是 16 位寄存器里的局部比特段，我们都已经能用 `BitVec` 模式和代数游刃有余地拆解与拼装了。

但是，硬件工程师又扔过来一份 C 语言写的 DMA 控制器驱动头文件。在那份头文件里，有一个 4 字节的硬件描述符槽位：当外部控制寄存器的模式位为 `0` 时，这 4 个字节代表一个 32 位有符号校准偏移 `i32`；当模式位为 `1` 时，**同一块 4 字节内存**里存放的却是一个 IEEE-754 单精度浮点数 `f32`！

注意：这块内存里**没有任何多余的空间放 `enum` 的判别标签（tag）**，而且两种类型的有效位模式几乎全都重叠（无法用 `.discrete_tag true` 的互斥位模式区分）！
在 Flurry 里，当我们需要与 C 语言的 `union` 赤身肉搏、在共享存储上切换不同成员视图时，为什么给 `union` 赋值**绝对不会自动触发旧成员的 `Drop`**？我们又该如何守住 `unsafe` 的安全边界？

在下一篇 **《Flurry 演进手记 (017)：与硬件寄存器赤身肉搏——union 共享存储、C ABI 与显式初始化契约》** 中，我们将深入这块没有标签保护的共享内存无人区！
