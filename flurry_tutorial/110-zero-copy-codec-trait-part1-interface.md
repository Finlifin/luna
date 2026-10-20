# Flurry 演进手记 (110)：设计零拷贝序列化 Trait Encode / Decode（一）：静态定宽与动态变长的统一接口

欢迎来到第五卷的核心实战三部曲。
在接下来的三篇（第 110、111、112 篇）中，我们将把前面学到的所有 Trait 武器——关联项（`assoc`）、父接口合取（`:- A + B`）、关联错误行（`![Error]`）、结构反射与自动派生（`derive fn`）、以及 Trait 的编译期/运行时两相（`comptime impl`）——融会贯通，亲手打造一套工业级的**零拷贝通用编解码器框架（Universal Zero-Copy Codec Framework）**。

在物联网边缘网关、高频交易系统或深度学习模型权重加载器中，序列化与反序列化（`Encode` / `Decode`）往往占据了高达 30% 以上的 CPU 开销。
为什么传统序列化库慢？
1. **无脑堆分配**：每次序列化一个小小的 8 字节传感器报文头 `SensorHeader`，都要先在堆上 `malloc` 一个动态 `Vec<u8>`；
2. **定宽与变长割裂**：定宽报文（如固定 12 字节的 CAN 总线帧）明明可以在编译期就知道精确帧长、直接一次性做边界检查，却被迫和变长字符串一起走逐字节动态扩容的慢速路径；
3. **多格式协议冲突**：同一个结构体既要支持紧凑二进制协议（`.binary`），又要支持调试文本协议（`.json`），如果 Trait 不支持协议参数化，你就得定义 `BinaryEncode`、`JsonEncode`、`MsgPackEncode` 一大堆互不相通的接口。

今天作为三部曲的第一篇，我们就来用 Flurry 的名义 Trait 体系，设计出既统一紧凑二进制与文本格式、又兼顾静态定宽与动态变长的 **`Encode` / `Decode` 核心接口**。

---

## 每次编码都返回新分配缓冲区的“内存吞金兽”

在很多高级语言里，最偷懒的序列化接口长这样：

```flurry
-- ❌ 传统反面教材：
-- 1. 强行规定每次 encode 都构造并返回一个固定容器，无法零拷贝写入已有 DMA 环形缓冲区。
-- 2. 没有协议参数，没法让同一个类型同时实现二进制编码与文本编码。
-- 3. 丢失了“定宽类型在编译期已知字节数”的宝贵静态信息。
trait NaiveEncode {
    fn encode(*self) -> Array<u8, 64usize>;
}
```

显然，这种玩具接口根本上不了工业级网关的台面。在嵌入式和边缘网关里，我们希望：
- **零拷贝流式读写**：直接往调用者传入的 `Writer` 或游标视图 `Cursor` 里读写字节，绝不暗中分配堆内存；
- **参数化完整接口实例**：通过 `where format: Symbol` 参数化 Trait，让 `Encode<.binary>` 和 `Encode<.json>` 成为同一 Trait 声明下不同的**完整接口实例（Complete Interface Instances）**；
- **关联元数据**：利用 `assoc` 关联值记录静态字节特征。

---

## 区分“Trait 声明”、“完整接口实例”与“关联值契约”

在动手写 `Encode` 之前，让我们先温习 `docs/traits/index.md` 中强调的三个核心层次：

```
+-------------------------------------------------------------------------+
|  Flurry Trait 系统的三个核心对象                               |
|                                                                         |
|  1. 【Trait 声明 (Trait Declaration)】                                  |
|     `trait Encode where format: Symbol { ... }`                         |
|     ──► 定义接口的名义身份、子句参数与成员契约模板。                    |
|                                                                         |
|  2. 【完整接口实例 (Complete Interface Instance)】                      |
|     `Encode<.binary>`   vs   `Encode<.json>`                            |
|     ──► Trait 声明 + 全部语义实参                                       |
|     ──► 同一个结构体 `SensorFrame` 可以同时实现 `Encode<.binary>`       |
|         和 `Encode<.json>`，两者互不冲突、共存共荣                      |
|                                                                         |
|  3. 【Implementation 实例 (Implementation Instance)】                   |
|     `impl Encode<.binary> for SensorFrame`                              |
|     ──► 具体主体类型如何兑现该完整接口实例（含关联项与方法体）。        |
+-------------------------------------------------------------------------+
```

更棒的是，在 `docs/traits/declarations.md` 中我们看到，Flurry 的 **`assoc` 关联项**不仅可以声明关联类型（如 `assoc Element: Type`），还可以**直接声明任意合法类型的关联值并提供默认值**（如 `assoc fixed_bytes: usize = 0`）。

让我们先定义零拷贝读写游标接口 `WireSink` 与 `WireSource`（复用上一篇学到的关联错误行），然后写出 `Encode` 与 `Decode` 的核心声明：

```flurry
enum CodecErr {
    BufferExhausted,
    InvalidTag(u8),
}

-- 零拷贝字节写入目标接口
trait WireSink {
    assoc Error: Type = CodecErr;
    fn put_u8(*self, byte: u8) -> ![Error] void;
    fn put_u32_le(*self, val: u32) -> ![Error] void;
}

-- 零拷贝字节读取源视图接口（直接切片借用底层报文缓冲区，零内存拷贝。）
trait WireSource {
    assoc Error: Type = CodecErr;
    fn take_u8(*self) -> ![Error] u8;
    fn take_u32_le(*self) -> ![Error] u32;
}

-- 核心序列化接口：按协议格式 format: Symbol 参数化。
trait Encode
where format: Symbol
{
    -- 关联常量：该类型的编码长度是否在编译期固定？默认 false（变长）
    assoc is_fixed_size: bool = false;

    -- 关联常量：若为固定长度，编译期静态字节数是多少？默认 0
    assoc wire_size: usize = 0;

    -- 默认方法：获取当前值的实际编码字节数
    -- 定宽类型直接返回关联常量 wire_size，变长类型可覆盖此方法计算动态长度。
    fn encoded_len(*self) -> usize {
        Self.as(Encode<format>).wire_size
    }

    -- 核心编码方法：将自身零拷贝写入任意实现了 WireSink 的目标 sink
    fn encode_to(*self, sink: *S) -> ![S.as(WireSink).Error] void
    where S :- WireSink;
}

-- 核心反序列化接口：同样按 format: Symbol 参数化
trait Decode
where format: Symbol
{
    fn decode_from(source: *S) -> ![S.as(WireSource).Error] Self
    where S :- WireSource;
}
```

看看这段 `Encode` 声明有多么优雅：
1. **统一关联类型与关联值**：`assoc is_fixed_size: bool = false` 和 `assoc wire_size: usize = 0` 直接携带了默认关联值。如果是变长类型，甚至不需要手写这两个 `assoc`；如果是定宽类型（比如 `u32` 或 `SensorHeader`），只需在 `impl` 中覆盖它们即可。
2. **默认方法自动读取关联项**：默认方法 `encoded_len(*self)` 内部直接通过 `Self.as(Encode<format>).wire_size` 获取当前实现提供的关联帧长。它不依赖任何具体结构体的私有字段，因此在 `trait Encode` 定义当场就通过了编译器的符号检查。

---

## 用父接口合取定义双向 `WireCodec`，并为基元类型与报文实现它

在实际工程中，许多网络协议处理函数要求某个报文类型 `T` **既能编码（`Encode<format>`），又能解码（`Decode<format>`）**。
我们可以利用 `docs/traits/declarations.md` 中的**父接口正向能力合取（`:- A + B`）**，定义一个组合 Trait `WireCodec`：

```flurry
-- 父接口合取：要求实现 WireCodec<format> 的主体必须分别满足 Encode<format> 与 Decode<format>。
trait WireCodec :- Encode<format> + Decode<format>
where format: Symbol
{
    -- 默认方法：基于父接口能力提供往返（Round-trip）帧长检查
    fn fixed_frame_bytes(*self) -> usize {
        self.as(Encode<format>).encoded_len()
    }
}
```

现在，让我们为基础类型 `u8`、`u32` 实现二进制协议 `.binary` 下的编解码器，并编写一个操作任意 `WireCodec<.binary>` 的泛型帧发送函数：

```flurry
impl Encode<.binary> for u8 {
    assoc is_fixed_size: bool = true;
    assoc wire_size: usize = 1;

    fn encode_to(*self, sink: *S) -> ![S.as(WireSink).Error] void
    where S :- WireSink
    {
        sink.put_u8(self.*)!;
    }
}

impl Decode<.binary> for u8 {
    fn decode_from(source: *S) -> ![S.as(WireSource).Error] u8
    where S :- WireSource
    {
        source.take_u8()!
    }
}

impl WireCodec<.binary> for u8 {}

impl Encode<.binary> for u32 {
    assoc is_fixed_size: bool = true;
    assoc wire_size: usize = 4;

    fn encode_to(*self, sink: *S) -> ![S.as(WireSink).Error] void
    where S :- WireSink
    {
        sink.put_u32_le(self.*)!;
    }
}

impl Decode<.binary> for u32 {
    fn decode_from(source: *S) -> ![S.as(WireSource).Error] u32
    where S :- WireSource
    {
        source.take_u32_le()!
    }
}

impl WireCodec<.binary> for u32 {}
```

### 编译期安全红线：空实现 `impl WireCodec` 绝不能绕过父接口契约

请注意 `docs/traits/declarations.md` 中的一条严格规则：
> **合取要求每项能力分别成立，不合并各 trait 的声明身份……移除其中任一实现，空的子接口实现就不能满足父接口要求。**

来看一个被编译器静态拦截的负例：如果我们定义了一个 `RawPacket`，只给它写了 `Encode<.binary>`，却忘了写 `Decode<.binary>`，然后试图写 `impl WireCodec<.binary> for RawPacket {}`：

```flurry
struct RawPacket {
    tag: u8,
}

impl Encode<.binary> for RawPacket {
    assoc is_fixed_size: bool = true;
    assoc wire_size: usize = 1;

    fn encode_to(*self, sink: *S) -> ![S.as(WireSink).Error] void
    where S :- WireSink
    {
        self.tag.encode_to(sink)!;
    }
}

-- ❌ 编译期直接拒绝。
-- `WireCodec<.binary>` 的父接口契约要求 `Encode<.binary> + Decode<.binary>` 分别成立，
-- `RawPacket` 尚未实现 `Decode<.binary>`，无法通过空的 `impl WireCodec` 蒙混过关。
impl WireCodec<.binary> for RawPacket {}
```

同理，如果我们想写一个安全的网络发送函数 `send_frame`，要求类型 `T` 不仅实现了 `Encode<.binary>`，而且**没有**被标记为禁止外发的敏感内部数据类型 `SensitiveInternalOnly`，只需结合正向约束 `:-` 与 `requires not (...)` 否定子句：

```flurry
trait SensitiveInternalOnly {}

fn send_frame(packet: *T, sink: *S) -> ![S.as(WireSink).Error] usize
where
    T :- Encode<.binary>,
    S :- WireSink,
    requires not (T :- SensitiveInternalOnly)
{
    packet.encode_to(sink)!;
    packet.encoded_len()
}
```

---

## 本篇小结

| 接口设计要素 | Flurry 语法与机制 | 解决的核心问题 |
| :--- | :--- | :--- |
| **多协议格式参数化** | `trait Encode where format: Symbol` | 同一个结构体可同时实现 `Encode<.binary>` 与 `Encode<.json>`，无需定义多套重复 Trait |
| **定宽与变长统一元数据** | `assoc is_fixed_size: bool = false`<br>`assoc wire_size: usize = 0` | 利用带默认值的关联常量在同一个接口中同时承载编译期定宽帧长与动态变长计算 |
| **零拷贝流式读写** | `where S :- WireSink` + `![S.as(WireSink).Error]` | 直接向任意底层缓冲区/网络流写入，错误类型随底层流多态推导，零堆内存分配 |
| **双向编解码契约组合** | `trait WireCodec :- Encode<format> + Decode<format>` | 通过父接口正向合取与 `requires not (...)` 否定约束，严密把控编解码能力边界 |

---

### 下一篇预告

在刚才的代码里，当我们为 `RawPacket` 实现 `Encode<.binary>` 时，我们手写了 `self.tag.encode_to(sink)!;`。
如果我们的边缘网关里有 **200 个**不同的传感器报文结构体，每个结构体有 10 个字段——难道我们要手写 200 遍枯燥乏味、极易漏掉字段的 `self.field1.encode_to(sink)!; self.field2.encode_to(sink)!;` 吗？

当然不。
在下一篇 **`111-zero-copy-codec-trait-part2-auto-deriver.md`** 中，我们将祭出 Flurry 的 **`derive fn` 派生策略机制**与**类型反射**，手把手为 `Encode` 和 `Decode` 编写自动派生器。让用户只需写下一行 `derive Encode<.binary> for SensorFrame;`，就能自动获得经过完整类型检查的零拷贝编解码实现。
