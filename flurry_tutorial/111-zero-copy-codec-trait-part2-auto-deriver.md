# Flurry 演进手记 (111)：设计零拷贝序列化 Trait（二）：利用结构反射与 derive fn 自动生成编解码实现

在上一篇（三部曲之一）中，我们设计好了支持多协议（`format: Symbol`）、零拷贝流式读写（`WireSink` / `WireSource`）以及定宽元数据的 `Encode` 与 `Decode` 接口。

但是，当我们面对网关工程里成百上千个传感器数据包结构体时，一个新的工程痛点浮出水面：
如果每个结构体都要程序员手写一遍 `impl Encode<.binary> for SensorFrame`，不仅枯燥乏味，而且一旦后续有人给 `SensorFrame` 新增了一个字段 `crc: u32`，却忘了去更新手写的 `encode_to` 方法，线上就会出现静默丢字段的严重协议 Bug！

在 Rust 中，你会写 `#[derive(Serialize, Deserialize)]`——但要编写那个派生宏，你得新建一个独立的 `proc-macro` crate，引入 `syn` 和 `quote` 把源码当成原始 Token 字符串流来拼接；更糟糕的是，一个接口只能绑死唯一个名叫 `Serialize` 的派生宏，如果你想对同一个接口提供“按字段顺序紧凑编码（`by_fields`）”和“带魔数前缀编码（`with_magic`）”两种不同的生成策略，标准 `#[derive]` 就只能干瞪眼，逼你发明一堆稀奇古怪的属性标签。

而在 Flurry 中，派生器（Deriver）不是脱离类型系统的黑盒文本宏，而是语言一等公民的 **`derive fn`（派生策略函数）**！
- 它直接写在普通模块里，用 **`^attrs.derivation.default`** 标记默认策略；
- 它支持用 **`derive Encode<.binary>.use(strategy) for Type;`** 显式切换不同生成策略，甚至向策略直接传递编译期参数；
- 最重要的是：**它生成出来的产物会原封不动地进入编译器普通的 `impl` 全套契约与一致性（Coherence）检查流水线！**

今天作为三部曲的第二篇，我们就来结合 Flurry 的类型反射（`Type.size_of`、`Type.repr_of`）与 `derive fn`，为我们的零拷贝编解码器装上自动派生的翅膀！

---

## 尝试 v0：手写 100 个 `impl Encode` 的脆弱与文本宏的黑盒陷阱

先看看如果我们手写一个包含两个字段的 `SamplePair` 编码实现：

```flurry
struct SamplePair {
    first: u32,
    second: u32,
}

-- 💭 v0 手写之痛：
-- 每定义一个新结构体，就要把这段模板代码抄一遍；
-- 字段一旦增删，手写的 wire_size 和 encode_to 极易与真实结构体脱节！
impl Encode<.binary> for SamplePair {
    assoc is_fixed_size: bool = true;
    assoc wire_size: usize = 8;

    fn encode_to(*self, sink: *S) -> ![S.as(WireSink).Error] void
    where S :- WireSink
    {
        self.first.encode_to(sink)!;
        self.second.encode_to(sink)!;
    }
}
```

我们真正想要的是：**定义一种可复用的构造策略（Strategy），让编译器观察目标类型 `Self` 的结构与布局，自动为我们合成这份 `impl`！**

---

## 演进 v1：认识 `derive fn` 与 `.use(strategy)`——一等公民的派生策略

在深入字段反射之前，我们先通过 `docs/traits/derivation.md` 掌握 Flurry 派生系统的核心骨架。
在 Flurry 中，定义派生策略和调用派生策略有着极其清晰的语法分工：

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：Flurry 的 derive fn 策略与产物检查流水线              |
|                                                                         |
|  【定义端：编写派生策略 derive fn】                                     |
|    • 默认策略：`^attrs.derivation.default derive fn default_strat ...`  |
|    • 具名策略：`derive fn custom_strat(param: u8) for TraitExpr { ... }`|
|                                                                         |
|  【使用端：应用策略生成 impl】                                          |
|    • 使用默认策略：`derive TraitExpr for MyStruct;`                     |
|    • 显式指定策略：`derive TraitExpr.use(custom_strat(0xAA)) for MyStruct;`|
|                                                                         |
|  【编译器验证流水线（绝不给生成代码开后门！）】                         |
|    在编译期执行 derive fn ──► 产出候选 impl ──► 执行全套普通 impl 检查：|
|    (1) 包归属 (Coherence)   (2) 重叠冲突检查   (3) 关联项与方法契约检查 |
+-------------------------------------------------------------------------+
```

来看一个直观的例子：我们不仅可以为 `Encode<.binary>` 编写默认的紧凑编码策略 `compact_binary`，还可以编写一个接收编译期魔数参数 `magic: u8` 的带头标策略 `magic_prefixed_binary`！

```flurry
-- 策略 1：标记为 Encode<.binary> 的默认派生策略
^attrs.derivation.default
derive fn compact_binary for Encode<.binary> {
    assoc is_fixed_size: bool = true;
    assoc wire_size: usize = Type.size_of(Self);

    fn encode_to(*self, sink: *S) -> ![S.as(WireSink).Error] void
    where S :- WireSink
    {
        -- 利用 Type.repr_of(Self) 结构反射与 inline if / inline for 编译期展开，
        -- 依次通过 self'get_field(field.name) 对 Self 的每个字段调用 encode_to！
        const repr = Type.repr_of(Self);
        inline if repr is do {
            .`struct` => {
                inline for (_, field) in repr.fields.enumerate() {
                    self'get_field(field.name).encode_to(sink)!;
                }
            },
        }
    }
}

-- 策略 2：带编译期参数 magic: u8 的具名派生策略！
-- 注意：magic 是生成器的编译期输入，不会变成生成的 encode_to 方法的运行时参数！
derive fn magic_prefixed(magic: u8) for Encode<.binary> {
    assoc is_fixed_size: bool = true;
    assoc wire_size: usize = 1usize + Type.size_of(Self);

    fn encode_to(*self, sink: *S) -> ![S.as(WireSink).Error] void
    where S :- WireSink
    {
        -- 先写入编译期固化的魔数头标 magic，再按结构反射编码各个字段
        sink.put_u8(magic)!;
        const repr = Type.repr_of(Self);
        inline if repr is do {
            .`struct` => {
                inline for (_, field) in repr.fields.enumerate() {
                    self'get_field(field.name).encode_to(sink)!;
                }
            },
        }
    }
}
```

现在看看用户在定义业务报文结构体时，使用这两种策略有多么清爽：

```flurry
struct RawTelemetry {
    temp: u32,
    pressure: u32,
}

struct FramedCommand {
    opcode: u32,
}

-- 1. 不写 .use(...)，自动匹配 ^attrs.derivation.default 的 compact_binary 策略！
derive Encode<.binary> for RawTelemetry;

-- 2. 用 .use(magic_prefixed(0xFEu8)) 显式选择带魔数前缀的策略！
derive Encode<.binary>.use(magic_prefixed(0xFEu8)) for FramedCommand;
```

> **💡 核心心智模型：生成逻辑在 `comptime` 跑 $\neq$ 生成的方法是 `comptime` 方法！**
> 请注意 `docs/traits/derivation.md` 强调的关键边界：
> - `derive fn` 本身的生成逻辑是在编译期运行的（比如计算 `1usize + Type.size_of(Self)`，把 `magic: u8` 固化进方法体）；
> - 但只要你写的是普通的 `derive Encode<.binary> for FramedCommand;`，它生成出来的 `fn encode_to(*self, ...)` **依然是正宗的运行时（Runtime Phase）方法**！调用 `cmd.encode_to(sink)` 时也不需要再传一遍 `magic` 参数。（注：如果你需要生成编译期相的 Trait 实现，则使用 `comptime derive`，我们将在下一篇揭晓！）

---

## 演进 v2：深入类型反射与布局（`Type.size_of` / `Type.repr_of`）的分层智慧

在刚才的派生策略里，我们用到了 `Type.size_of(Self)`。
在 `docs/type/reflection-and-layout.md` 中，Flurry 揭示了一条极其深刻的设计原则：
**“观察一个结构体有哪些字段（结构反射）” 与 “查询它的内存大小和偏移（布局反射）”，在编译器内部是两个严格分离的层次！**

为什么这两层必须分开？
1. **结构观察不等于布局已决**：当我们在一个泛型上下文或 `derive fn` 中观察一个类型的字段名和字段类型列表时，哪怕某些泛型参数尚未实例化导致最终字节大小还没算出来，结构观察本身也已经合法可用！
2. **指针截断递归，按值环直接拒绝**：
   - 当一个结构体包含指向自身的指针（比如链表节点 `struct Link { value: u32, next: ?*Link }`）时，指针本身的位宽固定等于 `Type.size_of(usize)`，查询 `*Link` 的大小不需要沿着 `next` 无限递归展开；
   - 但如果一个结构体**按值直接或间接包含自身**（比如 `struct InvalidLink { next: InvalidLink }`），它根本无法形成有限的内存表示，查询 `Type.size_of(InvalidLink)` 会在编译期当场报错！

```flurry
struct Link {
    value: u32,
    next: ?*Link,
}

-- ✅ 合法！指针大小固定等于 usize，不会沿 next 陷入死循环
const LINK_PTR_BYTES = Type.size_of(*Link);

comptime {
    assumes LINK_PTR_BYTES == Type.size_of(usize);
}

struct InvalidLink {
    -- ❌ 编译期直接拒绝！
    -- 按值包含自身构成无限大小环，无法形成合法的有限内存布局！
    next: InvalidLink,
}
```

---

## 编译期安全红线：派生策略绝不能产生重复实现或跳过字段检查！

在 Flurry 中，通过 `derive` 生成的实现不会享有任何“法外特权”。让我们看两个被编译器当场拦截的典型负例：

### 红线 1：用不同策略对同一类型派生同一完整接口实例，照样触发冲突！
有些同学可能会想：“我刚才给 `RawTelemetry` 派生了默认的 `compact_binary`，我现在能不能再用 `.use(magic_prefixed(0xAA))` 给它派生第二份 `Encode<.binary>`？”

**绝对不行！** `docs/traits/derivation.md` 明确指出：**策略身份不进入生成物的接口身份。同一主体、同一完整 trait 实例、同一 phase 的两种策略产生的是同精度重叠实现，直接冲突！**

```flurry
struct Record {
    id: u32,
}

derive Encode<.binary> for Record;

-- ❌ 编译期直接拒绝！
-- 选择另一个策略 magic_prefixed 不会制造第二层接口身份！
-- `Record` 已经拥有 `Encode<.binary>` 的同精度实现，重复 derive 直接冲突报错！
derive Encode<.binary>.use(magic_prefixed(0x01u8)) for Record;
```
（相反，如果你派生的是 `Encode<.json>`，因为 `Encode<.binary>` 和 `Encode<.json>` 是不同的**完整接口实例**，它们就可以合法共存！）

### 红线 2：如果某个字段没实现 `Encode<.binary>`，派生出的实现当场报错！
因为派生产物必须通过全套普通的 `impl` 检查，如果结构体里混入了一个没有实现 `Encode<.binary>` 的字段类型 `UncodableHandle`，编译器会在 `derive` 使用点同时指出派生位置、策略定义与失败的具体成员：

```flurry
struct UncodableHandle {
    raw_fd: i32,
}

struct BadPacket {
    header: u32,
    handle: UncodableHandle,
}

-- ❌ 编译期直接拒绝！
-- 派生产物在接受普通 implementation 检查时失败：
-- 字段 `handle` 的类型 `UncodableHandle` 未实现 `Encode<.binary>`！
derive Encode<.binary> for BadPacket;
```

---

## 本篇小结

| 特性 / 语法 | 核心规则 | 工程与安全价值 |
| :--- | :--- | :--- |
| **`derive fn` 与默认策略** | `^attrs.derivation.default`<br>`derive fn name(params...) for TraitExpr` | 将代码生成逻辑变成类型安全的编译期策略函数，支持传入编译期配置参数 |
| **策略选择 `.use(...)`** | `derive TraitExpr for Type;`<br>`derive TraitExpr.use(strat) for Type;` | 无需发明额外属性语法，直接通过 `.use(...)` 显式切换不同生成策略 |
| **策略身份擦除与全套检查** | 策略名不进入最终 `impl` 身份；产物执行完整的 Coherence 与契约检查 | 防止同一类型挂载两套冲突的 `Encode<.binary>`，并在定义处拦截未实现 `Encode` 的非法字段 |
| **反射分层（结构 vs 布局）** | `Type.size_of(T)`、`Type.repr_of(T)`、`value'type` | 区分字段结构观察与字节布局计算，支持合法指针递归并拦截按值无限递归环 |

---

### 留给下一篇的悬念

现在，我们的 `derive Encode<.binary> for RawTelemetry;` 已经能自动生成运行时的 `encode_to` 方法了！
但是，请仔细看一眼我们在上面写的 `wire_size`：
在刚才的简化版里，我们暂时用了 `Type.size_of(Self)`。可是在真实的网络协议中，**结构体的内存大小（包含 CPU 对齐 Padding 空洞）并不等于它在网线上的紧凑编码字节数（Wire Size）！**
比如一个包含 `tag: u8`（1 字节）和 `value: u32`（4 字节）的结构体 `TaggedValue`，在内存里因为 4 字节对齐占了 **8 字节**，但紧凑编码到网线上其实只有 **`1 + 4 = 5` 字节**！

不仅如此，我们还希望在**编译期（$U_1$）**直接对一个类型调用 `TaggedValue.wire_schema_hash()` 算出它的协议指纹，并预先生成一张 256 项的 CRC8 校验查找表，通过 `Splice` 零成本嵌入运行时解码器！

怎么让同一个 `Encode` Trait 既拥有运行时的 `fn encode_to(*self, ...)` 方法，又拥有编译期相的 `comptime fn exact_wire_bytes() -> usize` 方法？
下一篇（三部曲终篇）**`112-zero-copy-codec-trait-part3-comptime-precomputation.md`**，我们将用 **Trait 的 `comptime` / `runtime` 两相机制**与 **`Splice`** 完成最惊艳的一跃！
