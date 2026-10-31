# Flurry 演进手记 (001)：一个字节里的世界——从繁琐掩码到定宽模式

假设今天你的工位上多了一块电路板。硬件工程师递给你一页皱巴巴的手册，上面画着一个 8 比特（`u8`）传感器数据包的格式：

```text
  Bit:   7   6   5   4   3   2   1   0
       +---+---+---+---+---+---+---+---+
       |    Header     |    Payload    |
       |   (4 bits)    |   (4 bits)    |
       +---+---+---+---+---+---+---+---+
```

规则很简单：
1. 如果整个字节是 `0x00`，代表这是一个**空心跳包（Empty）**。
2. 如果高 4 位（Header）是 `0xA`（二进制 `1010`），代表这是一个**数据包（Data）**，低 4 位（Payload）里装着 `0` 到 `15` 的传感器读数。
3. 其他任何比特组合，都是线路噪声引起的**损坏包（Corrupted）**。

现在，我们需要在 Flurry 里写一个解析器，读取一个字节并打印出它的内容，同时还要能把数据包编码回字节发送出去。

如果我们带着 C 或传统系统语言的习惯坐下来，第一版代码会写成什么样？

---

## 一、 把整数当锤子（和随处可见的坑）

在还没有学会用 Flurry 的类型和模式思考之前，我们最容易想到的办法就是：直接拿 `u8` 当数据包，用比较和算术硬拆。

等等——你可能会问：Flurry 里怎么写按位与 `&` 和右移 `>>`？
翻开语法表你会吃一惊：**Flurry 根本没有给普通整数提供 `&`、`|`、`>>` 这些传统位运算符。**

为什么？因为把“一个代表数值大小的整数”和“一串需要按位切开的比特”混为一谈，正是无数隐蔽 bug 的温床（比如有符号数右移到底是算术移位还是逻辑移位？运算符优先级到底是 `a & b == c` 先算谁？）。

在 Flurry 中，如果你想把一个 `u8` 当成比特串来切，你需要通过 **take view（取视图）** 运算符 `'`，显式地用 `raw'bitvec` 拿到它的 8 位比特视图 `BitVec<8>`。

哪怕用上了 `'bitvec` 切片，如果我们仍然沿用“手动算偏移、手动比对”的命令式思维，写出来的代码通常是这样的：

```flurry
use std.prelude.*;

-- 命令式地切比特、比对机制数字
fn process_raw_initial(raw: u8) {
    -- 注意：Flurry 用 `--` 写单行注释，用 `and` / `or` / `not` 写逻辑运算
    if raw == 0u8 {
        println("Empty heartbeat");
    } else {
        -- `'bitvec` 取得 raw 的 BitVec<8> 视图
        -- 下标 0 是最低位（LSB），因此 4..8 是高 4 位，0..4 是低 4 位
        let bits = raw'bitvec;
        let header = bits[4..8];
        let payload = bits[0..4];

        if header == 0xA {
            -- 把 4 位比特无符号扩展为 u8 打印出来
            println("Sensor reading: {}", payload.as(u8));
        } else {
            println("Corrupted byte!");
        }
    }
}
```

> **🔍 语法小贴士：为什么有的语句末尾没有分号？**
> 仔细看上面的 `if ... { ... } else { ... }`，它的右花括号 `}` 后面没有 `;`。
> Flurry 有一条非常清爽的规则：**当一条语句或定义的最后一个 token 是 `}` 时，省略它后面的分号**（千万切勿书写成 `};`）；而以 `)` 或标识符结尾的普通语句（如 `println(...);`、`let bits = ...;`）则正常写 `;`。
> 另外，注意二元运算符（如 `==`）两侧的空格——在 Flurry 中，`+ - * / % < >` 等作为中缀运算符时**两侧必须有空白**，这样编译器在词法阶段就能把 `a < b`（小于）和 `BitVec<8>`（泛型参数）区分开。

### 传统结构示意

让我们钻进编译器和 CPU 的视角，看看 `process_raw_initial` 到底把数据变成了什么样：

```text
  输入: raw (u8) = 0xA5 (二进制 1010_0101)
   │
   ▼
  [ if raw == 0u8 ] ──(否)──► 切分比特视图 (raw'bitvec)
                               │
                 ┌─────────────┴─────────────┐
                 ▼                           ▼
          bits[4..8] (高位)            bits[0..4] (低位)
        ┌───┬───┬───┬───┐           ┌───┬───┬───┬───┐
        │ 1 │ 0 │ 1 │ 0 │           │ 0 │ 1 │ 0 │ 1 │
        └───┴───┴───┴───┘           └───┴───┴───┴───┘
                 │                           │
        [ header == 0xA ? ]                  │ (万一上面判错，
                 │                           │  我还是被切出来了。)
                (是)                         ▼
                 └─────────────────► payload.as(u8) = 5
```

这段代码能跑，但它有三个让人寝食难安的问题：

1. **高低位的心智负担**：在 `bits[lo..hi]` 中，索引 `0` 是最低有效位（LSB），所以 `bits[4..8]` 是高四位；但我们人类阅读二进制 `0b1010_0101` 时，左边却是高位。每次写 `4..8` 和 `0..4`，你都得在脑子里做一次坐标变换。万一协议改成 3 位 Header + 1 位 Flag + 4 位 Payload，手算下标分分钟错一位。
2. **校验与提取脱钩**：`payload` 在 `if header == 0xA` 之前就已经切出来了。即使 `header` 根本并非 `0xA`，后续代码也很容易手滑误用 `payload` 这个变量。
3. **没有领域模型**：出了这个函数，数据包在代码库里依然只是一个赤裸裸的 `u8`。没有任何类型能阻止别的模块把一个 `0xFF`（损坏包）当成合法数据包传给下游。

我们能否先解决第 1 和第 2 个问题——**既不手算下标，又让 `payload` 只有在 `header` 匹配成功时才存在**？

---

## 二、 用位向量模式（BitVec Pattern）一眼看穿比特

在 Flurry 中，模式匹配（Pattern Matching）是一等公民。而且，模式非但能解构元组和结构体，还能**直接按二进制或十六进制字面量切开 `BitVec`**。

来看看 `process_raw` 是怎么写的：

```flurry
use std.prelude.*;

typealias Nibble = BitVec<4>;

fn process_raw(raw: u8) {
    -- 后缀 `match`：把被匹配的值写在前面，顺着数据流往下读
    raw'bitvec match {
        0x00 => println("Empty heartbeat"),
        0xA (payload: Nibble) => {
            println("Sensor reading: {}", payload.as(u8));
        },
        _ => println("Corrupted byte!"),
    }
}
```

停下来欣赏一下第 9 行：

```flurry
0xA (payload: Nibble) => ...
```

这里发生了什么？让我们画张图拆解这个**位向量模式（BitVec Pattern）**：

```text
  被匹配主语: raw'bitvec (总宽度 = 8 bits)

  模式:   0x            A               (payload : BitVec<4>)
          │             │                         │
        进制前缀     字面段 (4 bits)          绑定段 (4 bits)
       (十六进制)   固定要求高 4 位为 1010    切出低 4 位并绑定到变量 payload
                        │                         │
                        ▼                         ▼
                  ┌───┬───┬───┬───┬───┬───┬───┬───┐
                  │ 1   0   1   0 │ 0   1   0   1 │
                  └───────────────┴───────────────┘
                   MSB (最高位)                LSB (最低位)
                   ◄──────── 总宽度 4 + 4 = 8 bits ────────►
```

相比手动移位与掩码，这种方式有以下改进：
- **所见即所得的位序**：位向量模式**永远从最高有效位（MSB）写到最低有效位（LSB）**，和你阅读手册图表、阅读二进制字面量的顺序完全一致。再也无须心算 `4..8` 和 `0..4` 了。
- **编译器替你算宽度**：`raw'bitvec` 是 8 位。模式里十六进制的 `A` 占 4 位，`Nibble`（即 `BitVec<4>`）占 4 位，$4 + 4 = 8$。如果你手滑把 `Nibble` 写成了 `BitVec<5>`，总宽度变成 9 位，编译器当场就会报静态错误。
- **作用域安全**：变量 `payload` 只在 `0xA` 匹配成功的那个分支里存在。在 `0x00` 或 `_` 分支里，你断无可能误触它。

> **🔍 语法小贴士：为什么 `match` 写在后面？**
> 在 C 或 Rust 中，你习惯写 `match expr { ... }`；而在 Flurry 中，`match` 是**后缀操作符**：` raw'bitvec match { ... } `。
> 为什么要这样设计？想象一下当你有一串链式调用 `packet.body().decrypt()'bitvec match { ... }` 时，后缀 `match` 让你从左到右一气呵成地读完数据变换，而无需为了包一层 `match` 跳回行首去加括号。另外要注意：`match` 的各个分支（arm）之间一律用**逗号 `,`** 分隔。

---

## 三、 把编码规则升格为类型——定宽 `enum`

上面的解析逻辑已经清晰了许多。不过，第三个问题依然存在：
如果我们要写一个函数 `send_packet(packet: ???)`，或者在程序里构造一个数据包，难道我们还要到处传递 `u8` 吗？如果传 `u8`，谁来保证传进来的并非 `0xFF` 这种非法编码？

在 Rust 或 C++ 里，你通常会定义一个高层的 `enum Packet { Empty, Data(u8) }`，在解析时把 `u8` 转成 `Packet`，发送时再写个 `encode` 函数把 `Packet` 用位移和或运算拼回 `u8`。但这会产生两个问题：
1. 在内存中，普通的 `enum Packet` 往往需要一个额外的判别标签（tag）字节加上载荷字节，体积膨胀成了 2 字节，破坏了与硬件寄存器或网络字节流的一一对应。
2. 你得维护两套逻辑：一套 `decode` 的模式匹配，一套 `encode` 的位拼接，稍有疏忽两边就会存在偏差。

在 Flurry 中，**`enum` 的变体可以直接定义在底层的比特模式上。**

让我们写出完整的实现版本：

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

短短几行代码，信息量丰富。让我们用一张图看懂 `enum Packet` 到底在内存中长什么样：

```text
  普通语言的 enum (额外 tag):        Flurry 的定宽模式 enum:
  ┌──────────┬──────────┐          ┌─────────────────────┐
  │ Tag (u8) │ Val (u8) │ (16 bits)│     Byte (8 bits)   │ (严格 8 bits!)
  └──────────┴──────────┘          └─────────────────────┘
                                     ▲                 ▲
                                     │                 │
                         empty = 0 ──┤                 ├── data: 0xA (payload)
                         [0000_0000]                   [1010_xxxx]
                         (全 0 即为 empty)              (高 4 位 1010 即为 data，
                                                        低 4 位原地存放 payload!)
```

看看 `enum Packet` 肚子里的那两行配置（Configuration，以 `.name expr` 的形式写在定义体开头）：
1. **`.base_type Byte`**：告诉编译器，这个 `enum` 的底层承载类型就是一个 `u8`（8 比特）。
2. **`.discrete_tag true`**：告诉编译器，**切勿额外分配 tag 字段**。变体之间的区分（判别条件），完全由变体后面写的表达式或位模式自己负责。
3. **`empty = 0`** 与 **`data: 0xA (payload: Nibble)`**：
   - `empty = 0` 是**表达式变体**：它的编码就是整数 `0`（即 `0x00`）。
   - `data: 0xA (payload: Nibble)` 是**模式变体**：只要高 4 位是 `0xA`，它就是 `data`，而低 4 位直接就是它的命名载荷字段 `payload`。

### 编译器在定义时为你做了什么？

当你写下上面这个 `enum Packet` 的瞬间，Flurry 编译器（在定义端的 Normalization 阶段）就已经在后台帮你完成了严格的数学核验：

- **宽度核验**：`data` 的模式是 `0xA`（4 位）+ `Nibble`（4 位）= 8 位，刚好等于 `.base_type Byte` 的 8 位。如果你把 `Nibble` 写成 `BitVec<5>`，直接报错。
- **值域核验**：`empty = 0` 在 `Byte`（`0..=255`）范围内。如果你写成 `empty = 256`，直接报错。
- **互斥性核验（Disjointness）**：`empty` 要求高 4 位是 `0000`（因为值是 `0`），而 `data` 要求高 4 位是 `1010`（`0xA`）。两者难以同时成立。
  想象一下，如果你偶有疏忽把 `empty` 写成了 `empty = 0xA5`：

```flurry
-- ❌ 编译期直接报错。
enum BadPacket {
    .base_type Byte,
    .discrete_tag true,
    empty = 0xA5,                 -- 0xA5 的高 4 位也是 0xA。
    data: 0xA (payload: Nibble),  -- 与 empty = 0xA5 发生编码重叠。
}
```

编译器全然无需跑测试用例，就能直接用位向量求解器抓出 `0xA5` 同时满足两个变体，编译期报错。

### 使用 `Packet`：高层抽象与底层比特的合体

现在，有了 `Packet` 类型，我们怎么构造它、怎么匹配它、怎么把它变成字节发出去？

```flurry
fn encode(packet: Packet) -> Byte {
    -- 取 packet 的位视图，再零开销转换为 Byte
    (packet'bitvec).as(Byte)
}

fn describe(packet: Packet) {
    -- 在高层业务代码里，它就是一个完整的安全 enum。
    -- 注意：这里已无须 `_` 兜底分支了，因为合法的 Packet 只有这两种可能。
    packet match {
        .empty => println("Empty heartbeat"),
        .data { payload: val } => println("Sensor reading: {}", val.as(Byte)),
    }
}

test {
    let p1 = Packet.empty,
    -- 带命名载荷的变体，用花括号指明字段名构造
    let p2 = Packet.data { payload: 5 }

    -- `assumes` 是 Flurry 的契约断言，让我们验证编码结果：
    assumes encode(p1) == 0x00;
    assumes encode(p2) == 0xA5;

    describe(p1);
    describe(p2);
}
```

看看我们收获了什么：
1. **构造即编码**：当你写下 `Packet.data { payload: 5 }` 时，无需手写 `(0xA << 4) | 5`——类型定义里的位模式**既是解码器，又是编码器**。它生成的值在底层直接就是 `0xA5`。
2. **窄化载荷的编译期保护**：`payload` 的类型是 `BitVec<4>`（`Nibble`）。当你写 `payload: 5` 时，因为 $0 \le 5 < 2^4$，编译通过；如果你手滑写了 `Packet.data { payload: 16 }`，编译器在编译期就会直接报错（超出 4 比特表示范围），免如 C 语言那样悄悄截断或污染高位的 `0xA`。
3. **穷尽且无噪的业务匹配**：一旦数据进入了 `Packet` 类型，在 `describe(packet: Packet)` 里做 `match` 时，你只需要处理 `.empty` 和 `.data`——已无须处理“损坏字节”，因为**非法比特根本无法成为一个合法的 `Packet` 值**。

---

## 本篇小结

回顾本篇的三个步骤：

| 版本 | 做法 | 暴露的问题 / 收获的改进 |
| :--- | :--- | :--- |
| **初版** | `u8` + `'bitvec` 切片 `bits[4..8]` | 手算 LSB 下标反直觉，校验与提取分离，缺少类型保护 |
| **改进版** | 后缀 `match` + 位向量模式 `0xA (payload: Nibble)` | 从高位到低位自然书写，宽度由编译器静态核验，绑定作用域安全 |
| **进阶版** | 定宽 `enum Packet`（`.base_type` + `.discrete_tag`） | 零额外内存开销，编译期证明变体互斥，一套模式同时完成构造与匹配 |

在这一篇里，我们顺着一个 8 比特传感器的需求，自然而然地遇到了 Flurry 的几个核心积木：
- **词法与基本习惯**：`--` 注释、中缀运算符两侧的空格规则、以 `}` 结尾省略分号、逻辑词 `and`/`or`/`not`。
- **Take View（`'`）与 `BitVec<n>`**：用 `expr'bitvec` 从数值切换到比特视角，用 `.as(T)` 进行显式转换。
- **后缀 `match` 与位向量模式**：顺着数据流写的 `expr match { ... }`，以及从 MSB 到 LSB 拼接的 `0xA (payload: Nibble)`。
- **定宽编码 `enum`**：把底层二进制格式直接变成零开销、静态验证互斥的高层名义类型。

### 下一篇预告

仔细看上面的 `test` 块：我们测试了怎么构造 `Packet` 并把它 `encode` 成 `Byte`，也测试了怎么用 `describe` 匹配一个已有的 `Packet`。

但是，从串口读进来的原始数据依然是一个可能损坏的 `raw: Byte`（比如 `0xFF`）。
- 如果我们写一个解析函数 `fn decode(raw: Byte) -> ???`，当遇到 `0xFF` 时，它应该返回什么？
- 当我们一次要连续解析一整个帧的多个字节，中途只要有一个字节损坏就要立刻中止报错，或者在报错前统一打印日志、甚至自动触发重试时，Flurry 是如何避免写出满屏的 `if (err != 0) return err;` 的？

在下一篇 **《Flurry 演进手记 (002)：当传感器遇到噪声——从空值 `?T` 到错误限定类型 `!Err T`》** 中，我们将继续改造这个传感器解析器，揭开 Flurry 对偶模式匹配（`?` 与 `!`）和错误消去块的神秘面纱。
