# Flurry 演进手记 (015)：跨越字节边界的比特流——跨元素 `BitVec` 模式与 UTF-8 变长解码

在 [上一篇 (014)](./014-riscv-decoder-with-fixed-width-enums.md) 中，我们用定宽位向量模式漂亮地拿下了 32 位 RISC-V 算术指令。
今天是《Flurry 演进手记》第一卷的收官之战，我们要挑战二进制解析领域最著名的一块试金石——**跨字节边界的变长编码（Variable-Length Encoding）**。

传感器网关的显示屏现在需要渲染来自全球站点的告警日志，所有日志文本都以 **UTF-8** 格式通过基于 `u8` 的字节流传输。
翻开 RFC 3629，一个 UTF-8 码点（Code Point）由 **1 到 4 个连续的 `u8` 字节**组成，其比特排布规则如下：

```text
  变体   字节数   第 1 字节 (u8)     第 2 字节 (u8)     第 3 字节 (u8)     第 4 字节 (u8)
  ────   ──────   ────────────────   ────────────────   ────────────────   ────────────────
  one    1 Byte   0xxxxxxx (7b)
  two    2 Bytes  110yyyyy (5b)      10zzzzzz (6b)
  three  3 Bytes  1110xxxx (4b)      10yyyyyy (6b)      10zzzzzz (6b)
  four   4 Bytes  11110www (3b)      10xxxxxx (6b)      10yyyyyy (6b)      10zzzzzz (6b)
```

同时，我们的硬件诊断工具还需要解析单精度浮点数 `f32` 的 IEEE 754 内部比特结构（1 位符号位 + 8 位指数 + 23 位尾数），从中快速识别出 `NaN`、`Infinity`（无穷大）和有限数。

让我们先用单元素位向量模式热个身，然后直接向跨元素的 `enum Utf8CodePoint` 发起总攻。

---

## 一、 在单个元素内解剖 `f32`——`matches` 布尔测试与无冗余分段

在跨越字节边界之前，我们先解决那个 32 位单元素的诊断小需求：把一个 `f32` 浮点数按 `1 + 8 + 23 = 32` 比特切开。

除了前面我们已经熟知的后缀 `match` 之外，Flurry 还提供了一个极其顺手的布尔后缀运算符：**`expr matches pattern`**（详见 `docs/control-flow/pattern-matching/bitvec.md` 与 `docs/grammar/syntax`）。
当你只想在普通 `if` 条件里判断某个位模式是否匹配（并顺手用 `if` 守卫检查绑定出来的字段），就可以直接写 `if num'bitvec matches ...`：

```flurry
use std.prelude.*;

test {
    let num: f32 = 13.625;

    -- 用 `matches` 搭配位向量模式与守卫，一行拆出符号(1位)、指数(8位)与尾数(23位)。
    if num'bitvec matches 0b(sign: u1) (exponent: u8) (mantissa: BitVec<23>)
        if exponent == 130 and mantissa == 0b10110100000000000000000 {
        println("sign: {}, exponent: {}, mantissa: {}",
            sign'bitvec.as(u8), exponent, mantissa.as(u32));
    }

    -- 用 `match` 分类 IEEE 754 特殊值：
    -- 注意：指数全为 1 (11111111) 时，尾数非 0 是 NaN，尾数全 0 是 Infinity。
    num'bitvec match {
        0b(_: u1) 11111111 (mantissa: BitVec<23>)
            if mantissa != 0 => println("NaN"),
        0b(_: u1) 11111111 00000000000000000000000 => println("Infinity"),
        _ => println("finite"),
    }
}
```

运行这段代码，输出为：

```text
sign: 0, exponent: 130, mantissa: 5898240
finite
```

> **💡 核心心智模型：每一段都占据自己的宽度，绝不重叠。**
> 仔细看上面 `num'bitvec match` 的第一臂：
> `0b(_: u1) 11111111 (mantissa: BitVec<23>)`
> 为什么我们在写了 `11111111` 之后，后面直接写 `(mantissa: BitVec<23>)`，而没有再写一个 `(exponent: u8)`？
> 因为在 Flurry 的位向量模式中，**每一段（Segment）都会實打实地消耗掉它自己的比特宽度，绑定段绝不会为前一段重新命名**。
> `(_: u1)` 消耗了最高 1 位，字面段 `11111111` 消耗了接下来的 8 位，`(mantissa: BitVec<23>)` 消耗了最后 23 位，加起来刚好是 $1 + 8 + 23 = 32$ 位。如果你既想绑定某一段、又想约束它的值，应该像上面第一个 `if` 那样写成 `(exponent: u8)` 再配合守卫 `if exponent == 130`（或内层模式），千万不能把 `11111111` 和 `(exponent: u8)` 并排写在一起（那样总宽度就会变成 $32 + 8 = 40$ 位，直接触发编译期宽度错误）。

---

## 二、 当变长 UTF-8 撞上单元素宽度墙

在 `f32` 上，所有分支的宽度都是整齐划一的 32 比特。
但现在，我们要定义 `enum Utf8CodePoint`。它的底层基本传输单元是单字节 `u8`（即 `.base_type u8`）。

如果我们按照前几篇的习惯，只写 `.base_type u8` 和 `.discrete_tag true`，然后把 UTF-8 的四种字节长度写进去，会发生什么？

```flurry
-- ❌ 编译期直接拒绝：（未启用跨元素布局时，模式总宽度必须严格等于 base_type 宽度 8 bits。）
enum BadUtf8CodePoint {
    .base_type u8,
    .discrete_tag true,

    one:   0b0(ascii_char: BitVec<7>),                                       -- 8 bits  (等于 u8)
    two:   0b110(ys: BitVec<5>) 10(zs: BitVec<6>),                           -- 16 bits (❌ 超出单元素 u8 宽度!)
    three: 0b1110(xs: BitVec<4>) 10(ys: BitVec<6>) 10(zs: BitVec<6>),        -- 24 bits (❌ 超出单元素 u8 宽度!)
    four:  0b11110(ws: BitVec<3>) 10(xs: BitVec<6>) 10(ys: BitVec<6>) 10(zs: BitVec<6>), -- 32 bits (❌ 超出!)
}
```

下面来看看为什么编译器在默认情况下会拦住 `two`、`three` 和 `four`：

```text
  默认单元素模式 (.dst 未启用)：
  对齐基准 = .base_type u8 (严格固定为 8 bits 盒子)

         ┌────────────────────────┐
  one:   │ 0 │ ascii_char (7 bits)│ (8 bits ──► ✅ 刚好装进 1 个 u8 盒子。)
         └────────────────────────┘

         ┌────────────────────────┐ ┌────────────────────────┐
  two:   │ 110 │   ys (5 bits)    │ │ 10 │   zs (6 bits)     │ (16 bits ──► ❌ 1 个 u8 装不下。)
         └────────────────────────┘ └────────────────────────┘
```

在默认情况下（未启用跨元素时），编译器要求**整条 pattern 的总宽度必须严格等于 `.base_type` 的宽度**。这保护了像 `Packet`（固定 1 字节）和 `ArithmeticInstruction`（固定 4 字节）这样的定长类型不会因为程序员算错比特数而越界。

那么，当我们**确实**需要一个逻辑值根据变体不同，动态跨越 1 个、2 个、3 个或 4 个 `u8` base 元素时，该怎么告诉编译器呢？

---

## 三、 开启 `.dst true`——四行模式定义完整 UTF-8 变长编码

在 Flurry 中，当一个枚举表示的是跨越多个 `.base_type` 元素的变长编码时，我们只需要在枚举配置中加上一行点睛之笔：**`.dst true`**（Dynamically Sized Type / 跨元素布局，详见 `docs/control-flow/pattern-matching/bitvec.md`）。

开启跨元素模式后，编译器的宽度对齐规则立刻切换为**跨元素对齐契约**：
1. 一个逻辑值可以跨越多个 `.base_type`（`u8`）元素；
2. 每个变体的位向量模式总宽度不再强制等于 8 比特，而是要求**与 `.base_type u8` 的元素边界对齐**（即 8 比特、16 比特、24 比特、32 比特均为合法的整字节倍数）。

来看看官方规范中这段宛如艺术品的 `enum Utf8CodePoint` 定义：

```flurry
use std.prelude.*,

enum Utf8CodePoint {
    .base_type u8,
    .discrete_tag true,
    .dst true,

    one:   0b0(ascii_char: BitVec<7>),
    two:   0b110(ys: BitVec<5>) 10(zs: BitVec<6>),
    three: 0b1110(xs: BitVec<4>) 10(ys: BitVec<6>) 10(zs: BitVec<6>),
    four:  0b11110(ws: BitVec<3>) 10(xs: BitVec<6>) 10(ys: BitVec<6>) 10(zs: BitVec<6>),
}
```

停下来，仔细端详这四行变体定义：
- **`one`**：`0b0`（1 位）+ `BitVec<7>`（7 位）= **8 bits**（恰好 1 个 `u8`）；
- **`two`**：`0b110`（3 位）+ `BitVec<5>`（5 位）+ `10`（2 位）+ `BitVec<6>`（6 位）= **16 bits**（恰好 2 个 `u8`）；
- **`three`**：$4 + 4 + 2 + 6 + 2 + 6 =$ **24 bits**（恰好 3 个 `u8`）；
- **`four`**：$5 + 3 + 2 + 6 + 2 + 6 + 2 + 6 =$ **32 bits**（恰好 4 个 `u8`）。

你没有写一行 `(byte0 & 0xE0) == 0xC0`，没有写一行 `((byte0 & 0x1F) << 6) | (byte1 & 0x3F)`，甚至不需要关心空白符——因为在同一条 `0b` 模式里，我们在字节交界处留出的空格（如 `0b110(ys: BitVec<5>) 10(zs: BitVec<6>)`）既让人类一眼看清了每一个 `u8` 字节的分界线，又在编译器眼里构成了一条严丝合缝的跨元素逻辑比特流。

### 跨元素位模式 数据流全景

让我们钻进编译器与运行时的视角，看看当 `.dst true` 配合 `.base_type u8` 时，这四行位模式是如何跨越字节边界并转化为高效的多字节决策树的：

```text
  输入序列：连续的 base 元素流 (u8 字节序列)
  ┌───────────────────┬───────────────────┬───────────────────┬───────────────────┐
  │     Byte 0 (u8)   │     Byte 1 (u8)   │     Byte 2 (u8)   │     Byte 3 (u8)   │
  └─────────┬─────────┴─────────┬─────────┴─────────┬─────────┴─────────┬─────────┘
            │                   │                   │                   │
            ▼                   │                   │                   │
  [ 检查 Byte 0 前缀 ]           │                   │                   │
    ├─ 0b0_______ ──► 命中 .one (消耗 1 字节，提取 7-bit ascii_char)     │
    │                           │                   │                   │
    ├─ 0b110_____ ──────────────▼                   │                   │
    │                 [ 检查 Byte 1 == 0b10______ ] │                   │
    │                   └─► 命中 .two (消耗 2 字节，提取 ys:5, zs:6)     │
    │                                               │                   │
    ├─ 0b1110____ ──────────────────────────────────▼                   │
    │                                     [ 检查 Byte 1 & Byte 2 均为 0b10______ ]
    │                                       └─► 命中 .three (消耗 3 字节，提取 xs:4, ys:6, zs:6)
    │                                                                   │
    └─ 0b11110___ ──────────────────────────────────────────────────────▼
                                                        [ 检查 Byte 1..3 均为 0b10______ ]
                                                          └─► 命中 .four (消耗 4 字节，提取 ws,xs,ys,zs)
```

如果有人在写 `three` 分支时，不小心把最后一个 `BitVec<6>` 写成了 `BitVec<5>`（总宽度变成了 $4 + 4 + 2 + 6 + 2 + 5 = 23$ 比特）：

```flurry
-- ❌ 编译期直接拒绝。
enum MisalignedUtf8 {
    .base_type u8,
    .discrete_tag true,
    .dst true,

    -- 23 bits 无法与 .base_type u8 (8 bits) 的跨元素边界对齐。
    three: 0b1110(xs: BitVec<4>) 10(ys: BitVec<6>) 10(zs: BitVec<5>),
}
```

编译器在静态宽度核验阶段就会算出 $23 \pmod 8 \neq 0$，由于不满足跨元素布局的对齐约束，编译期拒绝生成匹配代码。

---

## 本篇小结（兼第一卷收官回顾）

在这一篇中，我们完成了从单元素位向量剖析到跨元素变长协议定义的终极跨越：

| 阶段 / 特性 | 语法与配置 | 核心语义与保证 |
| :--- | :--- | :--- |
| **`matches` 与无冗余分段** | `if x'bitvec matches 0b(s: u1) ...` | 每一段独立消耗自身宽度，绑定段不为前一段重命名；需同时绑定与约束时使用内层模式或 `if` 守卫 |
| **单元素宽度约束（默认）** | `.base_type T`（无 `.dst`） | 整条位模式总宽度必须严格等于单个 `T` 的比特宽度 |
| **跨元素变长模式** | `.base_type u8` + `.discrete_tag true` + `.dst true` | 允许单个逻辑值跨越多个 base 元素（如 1~4 字节的 `Utf8CodePoint`），编译期强制核验总宽度相对对齐基准合法，并生成多元素决策树 |

---

### 第一卷完结与通往第二卷的悬念

至此，我们已经走完了 **《Flurry 演进手记》第一卷（001 ~ 015）** 的全部旅程。
回顾这 15 篇手记，我们从最底层的 1 个字节出发，亲手搭建起了 Flurry 语言的核心地基：
- **从比特到类型**：`BitVec<n>`、`'bitvec` 视图、定宽与跨元素 `.dst` 枚举（`Packet`、`ArithmeticInstruction`、`Utf8CodePoint`）、`'tag_name` 视图与分组变体；
- **从控制流到模式代数**：后缀 `match`、六层模式优先级、`(p)` vs `(p,)`、浮点 `NaN` 穷尽性证明、观察守卫与条件所有权移动；
- **从内存绑定到契约安全**：`let` 与 `var` 的正交边界、后缀指针 `.ref` / `.*`、失败安全的 `Drop` 次序、错误限定类型 `!Err T` 与依赖参数。

但是，故事才刚刚进入最激动人心的篇章。
在第一卷的末尾，看着我们刚刚写好的 `Utf8CodePoint`、`ArithmeticInstruction` 和 `SensorMonitor`，三个全新的问题已经浮出水面：
1. 当我们在匹配 `0b110(ys: BitVec<5>) 10(zs: BitVec<6>)` 拿到了拆散的比特片段 `ys` 和 `zs` 后，在表达式里该怎么把它们**零拷贝拼接（Concatenate）**成一个完整的 11 位码点，甚至**原地写回**某个寄存器的特定位段 `reg'bitvec[4..8] = ...`？
2. 当我们的传感器驱动不再只是被动返回 `!Err T`，而是需要在读取中途向外部请求“等待定时器中断”、“申请一块临时 DMA 缓冲区”或者“发射一条异步遥测日志”，且允许调用方自由决定是**当场恢复执行（Resume）**还是中止时，Flurry 的**代数效应（Algebraic Effects，`#Eff T`）**是如何做到的？
3. 我们在前面多次见到的 `comptime { assumes ..., }` 和 `Tuple<...Es: meta.List<Type>>`，背后的**编译期计算与证明类型检查器（PTC）**又是如何像写普通代码一样操纵类型的？

在即将开启的 **《Flurry 演进手记》第二卷** 中，我们将深入 **`BitVec` 代数与程序合成、代数效应 `#Eff`、能力 Trait（Capabilities）以及编译期元编程与形式化验证**。敬请期待。
