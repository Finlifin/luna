# Flurry 演进手记 (002)：当传感器遇到噪声——从可选类型 `?T` 到错误行 `!Err T`

在 [上一篇 (001)](./001-a-world-in-a-byte.md) 中，我们为 8 比特传感器协议设计了一个极其紧凑的定宽类型 `Packet`：

```flurry
use std.prelude.*;

typealias Byte = u8;
typealias Nibble = BitVec<4>;

enum Packet {
    .base_type Byte,
    .discrete_tag true,

    empty = 0,
    data: 0xA (payload: Nibble),
}
```

只要手里有一个 `Packet`，我们就知道它要么是 `0x00`（`empty`），要么高 4 位是 `0xA`（`data`）。
但问题来了：**真实世界的串口线可不会直接吐出 `Packet`。**

串口每次给你的是一个原始的 `raw: Byte`（`0..=255`）。如果线路上闪过一道电磁干扰，读进来了 `0xFF` 或者 `0x37`，我们的解析函数 `decode` 该怎么向调用者汇报：“嘿，这个字节不是合法的 `Packet`”？

更进一步：假设传感器每次采集会连续发来 **两个字节组成的一帧（Frame）**，要求我们把两个数据包里的读数加起来。如果中途遇到空包、读数超标或者线路损坏，我们又该如何优雅地中断、恢复甚至重试？

让我们从最天真的做法开始，一步步迭代出 Flurry 的可选类型与错误处理体系。

---

## 一、 用“哨兵值”充当哑巴报警器

在C 语言风格里，当函数不想引入复杂的返回结构时，最常见的偷懒办法就是**借用返回类型里的某个特殊值（比如 `255` 或 `-1`）代表失败**：

```flurry
use std.prelude.*,

-- 返回 0..=15 代表正常读数，返回 255 代表空包或损坏
fn decode_reading(raw: u8) -> u8 {
    raw'bitvec match {
        0xA (payload: BitVec<4>) => payload.as(u8),
        _ => 255u8, -- 无论是 0x00 空包还是 0xFF 损坏，统统返回 255。
    }
}

fn sum_two_readings(b1: u8, b2: u8) -> u8 {
    let r1 = decode_reading(b1);
    let r2 = decode_reading(b2);
    -- 糟糕。如果忘了检查 r1 == 255，直接相加就会算出荒谬的结果甚至溢出。
    if r1 == 255u8 or r2 == 255u8 {
        return 255u8;
    }
    r1 + r2
}
```

上面这种用哨兵值表示错误的做法存在明显缺陷：
1. **编译器完全不知道 `255` 是错误**：调用者哪怕忘了写 `if r1 == 255u8`，直接把 `r1 + r2` 拿去控制电机转速，编译器也一声不吭。
2. **合法的“无数据”与非法的“坏字节”混淆了**：`0x00` 是协议规定的正常空心跳包，而 `0xFF` 是硬件故障导致的线路损坏。把它们都压扁成 `255`，下游根本无法区分“暂时没数据，忽略即可”和“硬件坏了，快拉警报”。

我们先解决第一个问题：**当一个值可能存在、也可能不存在时，怎么让类型系统替我们守门？**

---

## 二、 可选类型 `?T` 与对偶模式 `pat?`

在 Flurry 中，表示“要么是一个 `T`，要么什么都没有”不需要写冗长的 `Option<T>`——直接在类型前面加上前缀 `?`，写成 **`?T`**。

> **🔍 语法小贴士：类型偏前缀，表达式偏后缀**
> 还记得上一篇我们说过 Flurry 的普通表达式偏爱**后缀**（比如 `raw'bitvec`、`x.ref`、`x.*`、`x.as(u8)`）吗？
> 与之相对，Flurry 的**类型表达式偏爱前缀**。
> - `*T`：指向 `T` 的指针类型
> - `?T`：可选类型（Option Type）
> - `!Err T`：错误限定类型（我们马上就会见到。）
> 这样设计让类型从左到右像自然语言修饰词一样层层嵌套（比如 `?*Packet` 一眼就能读出是“可选的 Packet 指针”），而值操作则从左到右顺着数据流链式推进。

让我们先写一个只负责从合法 `Packet` 中提取读数的函数 `extract_reading`：如果是 `.data` 就返回读数，如果是 `.empty` 心跳包，它不是错误，只是“没有读数”，所以我们返回 **`null`**：

```flurry
-- 返回 ?u8：有读数时直接返回 u8（自动注入 ?u8），空心跳包时返回 null
fn extract_reading(packet: Packet) -> ?u8 {
    packet match {
        .empty => null,
        .data { payload: val } => val.as(u8),
    }
}
```

注意看 `.data` 分支：我们直接写了 `val.as(u8)`，并没有写类似 `Some(val.as(u8))` 的包装器。当目标类型明确是 `?u8` 时，非空值会自动完成成功注入，而 `null` 则构造“无值”状态。

那么，调用方拿到 `?u8` 后怎么把它拆开呢？
在 Flurry 里，**表达式怎么消除它，模式就怎么匹配它**——这叫做**表达式与模式的对偶（Duality）**：

```text
  表达式侧 (传播/取出):             模式侧 (对偶匹配):
  let x = maybe_val?;              if maybe_val is x? do { ... }
                   ▲                               ▲
          后缀 `?`：有值则取出，            后缀 `?` 模式：匹配“有值”分支，
          为 null 则向外提前返回 null       并将内部载荷绑定到 x。
```

来看看用 `?u8`、后缀 `?` 传播、以及 `let ... else` 写出来的帧求和函数：

```flurry
-- 方式 A：如果任意一个包是空包，整帧直接返回 null（用后缀 `?` 传播）
fn sum_both_or_none(p1: Packet, p2: Packet) -> ?u8 {
    let r1 = extract_reading(p1)?;
    let r2 = extract_reading(p2)?;
    r1 + r2
}

-- 方式 B：用 `if ... is pat? do` 或 `let pat? = ... else` 精确控制空包行为
fn read_with_default(p: Packet) -> u8 {
    -- `let r? = ... else`：匹配有值分支绑到 r；若为 null 则进入 else 块（必须发散退出）
    let r? = extract_reading(p) else {
        0u8
    }
    r
}
```

> **🔍 语法小贴士：`if ... is ... do` 与 `let ... else`**
> 1. 当你只想匹配一种模式时，不用写多分支的 `match`，可以用 `if expr is pattern do { ... }`。注意 `pattern` 后面的关键字 **`do`** 是必须写的（为了防止和结构体构造 `Type { ... }` 产生语法歧义）。
> 2. `let` 和 `var` 的左边天然就是模式。如果模式可能匹配失败（比如 `let r? = ...`），你可以在后面接 `else { return ...; }`。和之前一样，`else { ... }` 以 `}` 结尾，后面**省略分号**。

---

## 三、 错误限定类型 `!Err T`——让每种失败都有名有姓

`?u8` 完美表达了 `.empty` 心跳包的“无读数”状态。
但现在回到最核心的问题：如果串口收到了损坏的字节（比如 `0xFF`），或者虽然收到了 `0xA (payload)`，但传感器自检规定读数 `15`（`0b1111`）代表**传感器硬件过热溢出（Overflow）**，这时候再用 `null` 就彻底丢失故障原因了。

我们需要告诉调用者：**这个函数要么成功返回 `T`，要么带着具体的错误原因失败。**

在 Flurry 中，这通过**错误限定类型（Error-Qualified Type）** `!Err T` 来实现。

### 命名规矩：向编译器报上错误名号

先记住 Flurry 的两条强制命名核心规则（详见 `docs/grammar/index.md`）：
- **错误类型名**：必须使用 `PascalCase`，且**以 `Err` 结尾**（例如 `DecodeErr`、`SensorErr`）。
- **错误枚举的变体名**：不同于普通枚举变体的 `snake_case`，**错误枚举的变体必须使用 `PascalCase`**（例如 `BadHeader`、`SensorOverflow`）。

让我们定义两个独立的错误枚举，分别代表“字节解码层面的错误”和“传感器数值层面的错误”：

```flurry
enum DecodeErr {
    -- 记录下那个损坏的原始字节，方便排查硬件噪声
    BadByte(Byte),
}

enum SensorErr {
    -- 读数达到 15 (0xF) 时表示传感器量程溢出
    Overflow,
}
```

### 构造错误（`error expr`）与传播错误（`expr!`）

现在，我们来写单字节解码函数 `decode_packet`。它的返回类型写作 `!DecodeErr Packet`（或者用错误行列表写作 `![DecodeErr] Packet`）：

```flurry
fn decode_packet(raw: Byte) -> !DecodeErr Packet {
    raw'bitvec match {
        0x00 => Packet.empty,
        0xA (payload: Nibble) => Packet.data { payload: payload },
        -- 用 `error expr` 构造错误分支结果
        _ => error DecodeErr.BadByte(raw),
    }
}
```

接着，我们写一个从 `Packet` 提取有效读数的函数 `read_valid_sensor`。如果遇到读数 `15`，它就报 `SensorErr.Overflow`：

```flurry
fn read_valid_sensor(packet: Packet) -> !SensorErr ?u8 {
    packet match {
        .empty => null,
        .data { payload: val } => {
            let num = val.as(u8);
            -- 顺便学一个酷炫特性：return 后面可以接 `while cond` 语句守卫。
            -- 含义是：“仅当 num == 15u8 为真时，才执行前面的 return”
            return error SensorErr.Overflow while num == 15u8;
            num
        },
    }
}
```

> **💡 实用特性：为什么条件返回写 `return ... while cond` 而不是 `if`？**
> 看到第 7 行的 `return error SensorErr.Overflow while num == 15u8;` 了吗？
> 在 Flurry 中，`return`、`break`、`continue` 后面都可以直接跟 `while 条件`，表示“当条件成立时才提前退出”。
> 为什么不用 `if`？因为在 Flurry 里 `if` 是表达式前缀。如果允许写 `return a if cond { ... }`，编译器和读者都会困惑 `if` 到底是修饰 `return` 的守卫，还是返回了一个 `if` 表达式。用 `while` 作转移守卫，既读起来像自然语言（“return error while num == 15”），又在语法上彻底消除了歧义。

### 当两种不同的错误汇合：错误行 `![DecodeErr, SensorErr]`

现在好戏开场了。我们要写 `read_byte(raw: Byte)`，它先调用 `decode_packet(raw)`（可能报 `DecodeErr`），再调用 `read_valid_sensor(packet)`（可能报 `SensorErr`）。

在组合来自不同模块的错误类型时，如果采用名义包装枚举（例如手写 `enum CombinedError { Decode(DecodeErr), Sensor(SensorErr) }`），往往需要编写较多的转换样板代码。

而在 Flurry 中，错误限定类型采用的是**错误行（Error Row）**机制。你只需要在类型前缀 `![...]` 里把可能发生的错误列出来，然后用后缀 **`!`** 一路畅通无阻地传播：

```flurry
fn read_byte(raw: Byte) -> ![DecodeErr, SensorErr] ?u8 {
    -- 后缀 `!`：如果成功就取出 Packet；如果失败，自动把 DecodeErr 嵌入返回值的错误行并提前返回。
    let packet = decode_packet(raw)!;
    -- 后缀 `!`：同样自动把 SensorErr 嵌入到 ![DecodeErr, SensorErr] 中。
    let reading = read_valid_sensor(packet)!;
    reading
}
```

下面通过图示看看错误行是怎么在函数之间流动和汇合的：

```text
  raw: Byte
    │
    ▼
  [ decode_packet(raw)! ]
    ├── (错误: DecodeErr) ──────────────────────┐
    │                                           │ (分支嵌入 Error Row)
    ▼ (成功: Packet)                             ▼
  [ read_valid_sensor(packet)! ]        ![DecodeErr, SensorErr] ?u8
    ├── (错误: SensorErr) ──────────────────────▲
    │                                           │
    ▼ (成功: ?u8) ─────────(成功注入)─────────────┘
```

而且，Flurry 的错误限定类型拥有**独立的结果 ABI**：
构造 `error ...` 和用 `!` 传播错误，本质上只是返回一个带标签的结果值——**它绝不会触发昂贵的 C++ 式异常栈展开（Stack Unwinding），也不会捕获 continuation。** 它既有返回码的零开销可预测性能，又有类型系统强制检查的绝对安全。

---

## 四、 局部消去、日志与重试——`! { ... }` 与前后置 `catch`

有了 `read_byte`，调用者拿到一个 `![DecodeErr, SensorErr] ?u8` 时，有三种方式来处理它。

### 1. 用对偶模式 `val!` 和 `error pat` 做普通 `match`

还记得 `?` 的对偶模式是 `pat?` 吗？`!` 也是一样。
- **`val!`** 匹配错误限定类型的**成功值**；
- **`error pat`** 匹配它的**错误分支**。

```flurry
fn inspect_result(raw: Byte) {
    read_byte(raw) match {
        -- 成功且有传感器读数（嵌套了 `?` 模式和 `!` 模式。）
        (num?)! => println("Valid reading: {}", num),
        -- 成功但是空心跳包
        null! => println("Heartbeat (no reading)"),
        -- 错误分支
        error DecodeErr.BadByte(b) => println("Line noise: {}", b),
        error SensorErr.Overflow => println("Sensor overheated!"),
    }
}
```

看到 `(num?)!` 和 `null!` 了吗？内层是 `?u8` 的模式（`num?` 或 `null`），外层套上 `!` 代表成功分支。正交组合，丝滑无比。

### 2. 用错误消去块 `expr! { ... }` 就地提供默认值或削减错误行

很多时候，成功的值我们只想直接赋值给变量，只有出错时才想特殊处理。这时不用写 `match`，直接在表达式后面接 **`! { ... }` 错误消去块**：

```flurry
-- 只消去 SensorErr（过热时当作读数 14 饱和处理），保留 DecodeErr 继续向外传播。
fn read_saturated(raw: Byte) -> !DecodeErr ?u8 {
    read_byte(raw)! {
        SensorErr.Overflow => 14u8,
    }
}
```

仔细看 `read_saturated` 的返回类型：输入是 `![DecodeErr, SensorErr] ?u8`，因为我们在 `! { ... }` 里处理掉了 `SensorErr.Overflow` 并给出了替代值 `14u8`，未覆盖的 `DecodeErr` 会自动沿着返回边界向外传播，所以返回类型精确地缩小成了 `!DecodeErr ?u8`。

### 3. 前置与后置 `catch`：统一审计与循环重试

假设现在硬件团队提出了一个很实在的工程需求：
> “每次调用 `read_byte` 出错时（不管是 `BadByte` 还是 `Overflow`），我们都希望**先统一打印一行告警前缀**，然后再打印具体是哪种错误；而且在带重试的读取循环里，如果是可恢复的失败，就在最后**统一决定是重试还是放弃返回**。”

如果用传统的 `match` 或 `try/catch`，你要么在每个错误分支里复制粘贴一遍日志和重试代码，要么写两层嵌套的 `match`。

Flurry 的 `! { ... }` 错误消去块为此专门设计了 **前置 `catch`** 与 **后置 `catch`**：

```text
  read_byte(raw)! {
      catch raw_err => [ 1. 前置 catch：任何错误发生时，最先执行。（如统一打日志） ],
      DecodeErr.BadByte(b) => [ 2. 普通错误分支：针对具体错误载荷做处理 ],
      SensorErr.Overflow   => [ 2. 普通错误分支：针对具体错误载荷做处理 ],
      catch raw_err => [ 3. 后置 catch：普通分支跑完后汇合到此，决定最终兜底值或控制流。 ],
  }
```

让我们用前置/后置 `catch` 和带标签的 `while :label` 循环，写出一个工业级带重试的传感器读取函数：

```flurry
fn read_with_retry(port: *Byte) -> ![DecodeErr, SensorErr] ?u8 {
    var attempts = 0u8;

    -- 带标签 `:retry_loop` 的循环
    while :retry_loop true {
        let reading = read_byte(port.*)! {
            -- [前置 catch]：只要出错，先统一记账
            catch _ => {
                attempts += 1u8;
                println("Attempt {} failed:", attempts);
            },
            -- [具体错误分支]：分别打印诊断细节
            DecodeErr.BadByte(b) => println("  -> Corrupted byte: {}", b),
            SensorErr.Overflow => println("  -> Sensor overflow"),
            -- [后置 catch]：绑定完整的原始错误结果 `failure`，统一决定重试还是彻底退出。
            catch failure => {
                continue retry_loop while attempts < 3u8;
                return failure;
            },
        }
        -- 如果 read_byte 成功，两个 catch 和错误分支全部跳过，直接拿到成功值返回。
        reading
    }
}
```

让我们顺着控制流走一遍：
1. **如果 `read_byte(port.*)` 成功**：`! { ... }` 里的所有分支（包括两个 `catch`）全部跳过，`reading` 直接拿到 `?u8` 并 `return reading;`。
2. **如果 `read_byte(port.*)` 失败**：
   - 第一步：执行**前置 `catch`**，`attempts` 加 1，打印 `"Attempt 1 failed:"`。
   - 第二步：匹配对应的具体错误分支（`BadByte` 或 `Overflow`），打印具体原因。
   - 第三步：进入**后置 `catch failure`**。注意这里的 `failure` 绑定的是**完整的原始错误结果**（类型为 `![DecodeErr, SensorErr] ?u8`）。
   - 如果 `attempts < 3u8`，`continue retry_loop while attempts < 3u8;` 生效，直接跳回循环开头重试。
   - 如果已经试满 3 次，直接 `return failure;`，把原汁原味的错误结果原封不动地向上交付。

没有任何隐藏的控制流机制，重试就是清清楚楚的 `while :retry_loop` 循环，但前后置 `catch` 把“公共前置动作 -> 具体错误分类 -> 公共善后/重试”组织得如同流水线一般清晰。

---

## 本篇小结

通过上述步骤，我们的错误处理完成了四级跳：

| 版本 | 做法 | 解决的问题 |
| :--- | :--- | :--- |
| **初版** | 返回特殊哨兵值 `255u8` | 编译器无法强制检查，正常空包与硬件错误混为一谈 |
| **改进版** | 可选类型 `?u8` + `null` + `?` 传播/模式 | 在类型层面区分“有值”与“空心跳包”，支持 `let r? = ... else` |
| **进阶版** | 错误限定类型 `![DecodeErr, SensorErr] ?u8` | 多种错误自动在错误行中组合，用 `!` 零开销传播，命名规则清晰 |
| **完整版** | 对偶模式 `(num?)!` 与消去块 `! { catch ... }` | 支持局部消除特定错误，并用前后置 `catch` 优雅组织公共日志与循环重试 |

本篇引入的 Flurry 核心特性：
- **前缀类型修饰**：`?T`（可选类型）与 `!Err T` / `![E1, E2] T`（错误限定类型）。
- **表达式与模式对偶**：
  - 表达式 `expr?` / `expr!` 传播或消去；模式 `pat?` / `pat!` 匹配成功值。
  - 表达式 `error expr` 构造错误；模式 `error pat` 匹配错误载荷。
- **控制流利器**：
  - 单模式匹配：`if expr is pattern do { ... }` 与 `let pattern = expr else { ... }`。
  - 语句转移守卫：`return expr while cond`、`continue label while cond`。
  - 错误消去块 `expr! { ... }` 中的前置与后置 `catch`。

### 下一篇预告

在 `read_with_retry` 的签名里，我们悄悄写了一个参数：`port: *Byte`，并且在函数体里写了 `let attempts = ...; var attempts = 0u8;` 和 `port.*`。

等等：
- 在上一篇和这一篇里，我们既用过 `let`，也用过 `var`。你可能会想：“这不就是 Rust 的 `let` 和 `let mut`，或者 JS 的 `const` 和 `let` 吗？”
- **大错特错。** 如果我在 Flurry 里写 `let cell = Cell { value: 1 }`，我竟然可以直接写 `cell.value = 2;`。为什么 `let` 允许修改字段？
- 如果我们在重试循环里用一个结构体 `Monitor` 来保存传感器状态和资源句柄，当覆盖赋值 `mon = build_new_monitor()!` 发生错误时，旧的 `Monitor` 会被提前销毁吗？它的 `Drop` 顺序又是怎么决定的？

在下一篇 **《Flurry 演进手记 (003)：`let` 并没有冻结你的对象——绑定、指针 `.ref`/`. *` 与失败安全的资源更替》** 中，我们将把单个传感器升级为带状态的传感器监控器，彻底打破你对 `let`/`var` 和指针别名的固有印象。
