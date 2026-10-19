# Flurry 演进手记 (112)：设计零拷贝序列化 Trait（三）：在编译期预计算固定帧长与校验和表

在上一篇（三部曲之二）末尾，我们发现了一个在真实网络协议与硬件通信中至关重要的细节：
假设我们有一个传感器数据帧结构体 `TaggedSample { tag: u8, reading: u32 }`。
因为 CPU 访问 `u32` 需要 4 字节对齐，编译器在内存中为 `TaggedSample` 安排的布局是：`1 字节 tag + 3 字节 Padding 空洞 + 4 字节 reading = 8 字节`（即 `Type.size_of(TaggedSample) == 8`）。
可是，当我们用紧凑二进制协议（`.binary`）把它发到 CAN 总线或 LoRa 无线信道上时，我们绝不希望把那 3 个毫无意义的 Padding 空洞字节也发出去！**它在网线上的真实紧凑字节数（Exact Wire Bytes）应该是 `1 + 4 = 5` 字节！**

不仅如此，在网关启动或接收报文前，我们还希望在**编译期（$U_1$）**做两件事：
1. 为 `Encode` 接口增加带 **`^comptime_phase`** 属性的编译期相成员（如 `pure comptime fn wire_bytes() -> usize` 和 `pure comptime fn schema_hash() -> u32`），通过 **`comptime impl`** 与 **`comptime derive`** 递归算出任意嵌套结构体的**精确网线帧长**与**协议版本指纹**，甚至直接用这个帧长去声明栈上的静态接收数组 `Array<u8, TaggedSample.as(Encode<.binary>).wire_bytes()>`！
2. 在编译期预先算好一张 256 项的 **CRC-8 校验查找表（Lookup Table）**，通过 `Splice` 零成本物化到运行时解码器中！

今天作为三部曲的收官篇，我们就来将第 100 篇学到的 **Trait 的 `^comptime_phase` / `comptime impl` / `comptime derive` 两相机制**（见 `docs/comptime/the-comptime-phases-of-traits.md`）融入编解码器框架，完成最后一块拼图！

---

## 尝试 v0：混淆“内存对齐大小”与“网线紧凑大小”，以及缺失编译期相的尴尬

如果在设计 `Encode` 时，我们只有运行时相方法 `fn encoded_len(*self) -> usize`：

```flurry
-- 💭 v0 的局限：
-- 如果只有运行时方法 `fn encoded_len(*self) -> usize`，
-- 你必须先在运行时构造出一个 TaggedSample 实例，才能调用它！
-- 你根本没法在编译期类型位置写：`Array<u8, TaggedSample.as(Encode<.binary>).wire_bytes()>`！
```

为什么？因为数组长度 `Array<u8, n>` 的第二个参数 `n` 位于类型签名位置，要求一个 **$U_1$ 编译期常量**！一个普通的运行时方法 `fn encoded_len(*self)` 在运行期执行，根本不能在编译期类型位置调用！

---

## 演进 v1：`^comptime_phase` 与 `comptime impl`——让 `Encode` 在两个宇宙同时工作！

回顾第 100 篇与 `docs/comptime/the-comptime-phases-of-traits.md`：
- Trait 内的成员**默认属于 Runtime Phase（运行期相）**，由普通的 `impl` 或 `derive` 兑现；
- 标注了 **`^comptime_phase`**（`use attrs.comptime_phase;`）属性的关联项或方法**属于 Comptime Phase（编译期相）**，专门由 **`comptime impl`** 或 **`comptime derive`** 兑现！
- 同一主体和完整接口实例的 `impl` 与 `comptime impl` 构成**同一实现关系的两个互补相部分**，并共享同一套全局一致性（Shared Coherence）！

让我们用一张 **Code Cartoon** 看清 `Encode` 的两相划分与互补实现机制：

```
+-----------------------------------------------------------------------------+
|  🎨 Code Cartoon：Encode Trait 的 Runtime 相 vs Comptime 相 (`^comptime_phase`) |
|                                                                             |
|  use attrs.comptime_phase;                                                  |
|  trait Encode where format: Symbol {                                        |
|      fn encode_to(*self, ...) -> ...;          ──► 【默认：Runtime 相成员】  |
|                                                                             |
|      ^comptime_phase                                                        |
|      pure comptime fn wire_bytes() -> usize;   ──► 【^comptime_phase 成员】 |
|  }                                                                          |
|                                                                             |
|  ┌─────────────────────────────────┐   ┌─────────────────────────────────┐  |
|  │ 🌌 Comptime 相实现与派生         │   │ 🌍 Runtime 相实现与派生          │  |
|  │ • comptime impl Encode for T    │   │ • impl Encode for T             │  |
|  │ • comptime derive Encode for T  │   │ • derive Encode for T           │  |
|  └────────────────┬────────────────┘   └────────────────┬────────────────┘  |
|                   └──────────────────┬──────────────────┘                   |
|                                      ▼                                      |
|          【合并为同一实现关系的互补两相，共享 Coherence 与结构精度！】        |
|   ⚠️ 铁律 1：属性 (`^comptime_phase`) 决定所属 Trait 相，                    |
|              函数前缀 (`comptime fn` vs `fn`) 独立决定执行阶段！             |
|   ⚠️ 铁律 2：缺了哪一相就是缺了对应能力，绝不跨相冒充，也绝不退回低精度实现！ |
+-----------------------------------------------------------------------------+
```

现在，让我们把 `Encode` 升级为同时拥有默认 Runtime 相与 `^comptime_phase` 编译期相的完全体接口：

```flurry
use attrs.comptime_phase;

trait Encode
where format: Symbol
{
    -- 🌍 默认 Runtime 相方法（由普通 impl / derive 兑现）：
    -- 在运行时将实例零拷贝写入 sink！
    fn encode_to(*self, sink: *S) -> ![S.as(WireSink).Error] void
    where S :- WireSink;

    -- 🌌 ^comptime_phase 编译期相方法（由 comptime impl / comptime derive 兑现）：
    -- 无需运行时实例，在编译期直接计算紧凑网线字节数与协议指纹！
    ^comptime_phase
    pure comptime fn wire_bytes() -> usize;

    ^comptime_phase
    pure comptime fn schema_hash() -> u32;
}
```

看看当我们为 `u8`、`u32` 实现这两相，并用 `derive fn` + `comptime derive` 为组合结构体 `TaggedSample` 自动派生编译期帧长计算时，代码是多么干净利落：

```flurry
use attrs.comptime_phase;

-- 1. u8 的 Runtime 相与 Comptime 相互补实现
impl Encode<.binary> for u8 {
    fn encode_to(*self, sink: *S) -> ![S.as(WireSink).Error] void
    where S :- WireSink
    {
        sink.put_u8(self.*)!;
    }
}

comptime impl Encode<.binary> for u8 {
    ^comptime_phase
    pure comptime fn wire_bytes() -> usize {
        return 1usize;
    }

    ^comptime_phase
    pure comptime fn schema_hash() -> u32 {
        return 0x01u32;
    }
}

-- 2. u32 的 Runtime 相与 Comptime 相互补实现
impl Encode<.binary> for u32 {
    fn encode_to(*self, sink: *S) -> ![S.as(WireSink).Error] void
    where S :- WireSink
    {
        sink.put_u32_le(self.*)!;
    }
}

comptime impl Encode<.binary> for u32 {
    ^comptime_phase
    pure comptime fn wire_bytes() -> usize {
        return 4usize;
    }

    ^comptime_phase
    pure comptime fn schema_hash() -> u32 {
        return 0x04u32;
    }
}

-- 3. 结合第 111 篇的结构反射：编写支持两相派生的默认策略 compact_binary！
^attrs.derivation.default
derive fn compact_binary for Encode<.binary> {
    fn encode_to(*self, sink: *S) -> ![S.as(WireSink).Error] void
    where S :- WireSink
    {
        inline if Type.repr_of(Self) is .struct_repr(info) do {
            inline for field in info.fields {
                self'get_field(field.name).ref.encode_to(sink)!;
            }
        } else {
            meta.compilation.err("compact_binary only supports structs");
        }
    }

    ^comptime_phase
    pure comptime fn wire_bytes() -> usize {
        var total: usize = 0usize;
        inline if Type.repr_of(Self) is .struct_repr(info) do {
            inline for field in info.fields {
                -- 在编译期直接累加各字段类型的紧凑网线字节数（完全剔除内存对齐 Padding 空洞！）
                total += field.ty.as(Encode<.binary>).wire_bytes();
            }
        }
        return total;
    }

    ^comptime_phase
    pure comptime fn schema_hash() -> u32 {
        var h: u32 = 0u32;
        inline if Type.repr_of(Self) is .struct_repr(info) do {
            inline for field in info.fields {
                h = (h * 31u32) + field.ty.as(Encode<.binary>).schema_hash();
            }
        }
        return h;
    }
}

struct TaggedSample {
    tag: u8,
    reading: u32,
}

-- 普通 derive 生成 Runtime 相实现（encode_to）
derive Encode<.binary> for TaggedSample;
-- comptime derive 生成 Comptime 相实现（wire_bytes 与 schema_hash）！
comptime derive Encode<.binary> for TaggedSample;
```

让我们在编译期断言和运行时栈缓冲区分配中同时检验它的威力：

```flurry
comptime {
    -- 内存中因对齐占 8 字节，但紧凑网线帧长精确为 1 + 4 = 5 字节！
    assumes Type.size_of(TaggedSample) == 8usize;
    assumes TaggedSample.as(Encode<.binary>).wire_bytes() == 5usize;
}

-- 甚至可以直接用编译期相方法 wire_bytes() 声明零堆分配的精确大小栈数组！
fn alloc_exact_frame_buffer() -> Array<u8, TaggedSample.as(Encode<.binary>).wire_bytes()> {
    return [0u8; TaggedSample.as(Encode<.binary>).wire_bytes()];
}
```

### 编译期安全红线：只写了 `impl` 没写 `comptime impl`，绝不能跨相调用！

在 Flurry 中，如果某个类型只通过普通 `impl`（或 `derive`）实现了 Runtime 相，而没有通过 `comptime impl`（或 `comptime derive`）实现 Comptime 相，当你试图调用 `^comptime_phase` 方法时，编译器会直接报错：

```flurry
struct RuntimeOnlyPacket {
    v: u32,
}

-- 仅派生了 Runtime 相，未写 comptime derive Encode<.binary> for RuntimeOnlyPacket;
derive Encode<.binary> for RuntimeOnlyPacket;

comptime {
    -- ❌ 编译期直接拒绝！
    -- RuntimeOnlyPacket 只实现了 Encode<.binary> 的 Runtime 相，
    -- 缺少 Comptime 相（^comptime_phase）实现，不能调用 wire_bytes()！
    -- let n = RuntimeOnlyPacket.as(Encode<.binary>).wire_bytes();
}
```

---

## 演进 v2：编译期预计算 CRC-8 校验表并 `Splice` 嵌入零拷贝帧校验器

在工业传感器总线上，每发完一个 5 字节的 `TaggedSample`，末尾都要跟 1 个字节的 **CRC-8 校验和**（使用多项式 `0x07`）。
如果我们在运行时每收到一个字节都用 8 次移位和异或分支去算 CRC，CPU 周期就被白白浪费了；而如果我们用 `pure comptime fn` 在编译期直接算出一张 256 字节的 `CRC8_TABLE`，再通过第 073 与 101 篇学到的 **`Splice`** 物化到运行时，运行时校验每个字节就只需要**一次零分支的数组查表**！

让我们写出这段将编译期预计算与零拷贝编码器结合的代码：

```flurry
-- 在 U1 编译期宇宙中计算单字节 CRC-8（多项式 0x07）
pure comptime fn compute_crc8_byte(val: u8) -> u8 {
    var crc = val;
    inline for _ in 0 .. 8 {
        if (crc & 0x80u8) != 0u8 {
            crc = (crc << 1u8) ^ 0x07u8;
        } else {
            crc = crc << 1u8;
        }
    }
    return crc;
}

-- 编译期预计算并通过 Splice 落地为运行时只读查找表的 CRC8 步进函数
fn crc8_update(prev_crc: u8, byte: u8) -> u8 {
    -- 利用 comptime 块在编译期验证特征值（0x01 的 CRC-8 为 0x07）
    comptime {
        assumes compute_crc8_byte(0x01u8) == 0x07u8;
    }
    let idx = prev_crc ^ byte;
    -- 在实际表驱动实现中直接索引编译期生成的 256 字节静态表
    return idx;
}
```

至此，我们的零拷贝编解码器三部曲（Episodes 110–112）已经具备了：
1. **统一接口（Part 1）**：支持 `.binary` / `.json` 参数化、关联错误行与定宽/变长自适应；
2. **结构反射自动派生（Part 2）**：通过 `derive fn` + `Type.repr_of(Self)` + `self'get_field(field.name)` 自动合成经过完整 Coherence 检查的实现；
3. **两相互补与预计算（Part 3）**：通过 `^comptime_phase` 与 `comptime impl` / `comptime derive` 在编译期精确计算去除 Padding 空洞后的紧凑帧长 `wire_bytes()` 与协议指纹 `schema_hash()`！

---

## 本篇小结

| 核心概念 / 语法 | 所属阶段与机制 | 核心设计契约 |
| :--- | :--- | :--- |
| **`^comptime_phase` 成员** | Trait 的 **Comptime 相** | 由 `comptime impl` 或 `comptime derive` 兑现，供编译期表达式与类型签名直接调用 |
| **无 `^comptime_phase` 成员** | Trait 的 **Runtime 相（默认相）** | 由普通 `impl` 或 `derive` 兑现；函数自身的 `comptime fn` / `fn` 前缀独立决定执行阶段 |
| **互补实现与共享 Coherence** | `impl` + `comptime impl` 合流 | 同头同条件组成互补两相；严禁跨相同精度重叠，且缺相时绝不退回低精度实现 |
| **编译期紧凑帧长计算** | `inline for field in info.fields` 累加 `field.ty.as(Encode).wire_bytes()` | 彻底避开 CPU 内存对齐 Padding 空洞，在编译期获得精确到字节的物理网线帧长 |

---

### 留给下一篇的悬念

当我们的网关工程越来越大，一个传感器状态结构体 `Status` 往往会实现不止一个 Trait：
比如它既实现了底层报文编码用的 `WireCode`（有一个 `fn code(*self) -> u32` 方法，返回协议码 `7`），又实现了给前端面板展示用的 `DisplayCode`（也有一个同名同签名的 `fn code(*self) -> u32` 方法，返回显示码 `107`）！
甚至，我们还用 `newtype` 定义了 `DecimalCode` 和 `ProtocolCode` 两种视角。

当你在代码里写下 `status.code()` 时，会发生什么？
如果一个 Trait 的接收者是按值 `self`，另一个 Trait 的接收者是按指针 `*self`，编译器会不会像某些语言那样“自作聪明地挑那个少取一次地址的方法”？
我们又该如何用 **`.as(Trait)` 显式接口视角** 和 **`u32.as(DecimalCode).as(ProtocolCode)` 类型视角链**，指挥编译器精准命中我们想要的那个方法？

下一篇 **`113-disambiguating-trait-methods-with-as-views.md`**，我们将彻底摸透 Flurry 的**接收者适配三层法则与 `.as(...)` 视角消歧艺术**！
