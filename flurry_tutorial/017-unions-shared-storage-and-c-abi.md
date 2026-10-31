# Flurry 演进手记 (017)：与硬件寄存器直接交互——union 共享存储、C ABI 与显式初始化契约

在 [上一篇 (016)](./016-bitvec-concatenation-slicing-and-writeback.md) 的结尾，硬件工程师给我们出了一道难题：
我们要对接一块 C 语言编写的传感器 DMA 硬件描述符。其中有一个 4 字节（32 位）的校准槽位 `CalibrationSlot`：
- 当外围控制寄存器的标志位为 `0` 时，这 4 个字节存放的是一个 32 位整数偏移量 `i32`；
- 当外围控制寄存器的标志位为 `1` 时，**同一块 4 字节内存**存放的却是一个 32 位单精度浮点缩放系数 `f32`。

你可能会立刻想到我们在 [第 001 篇](./001-a-world-in-a-byte.md) 和 [第 013 篇](./013-enum-grouped-variants-and-tag-names.md) 学过的 `enum`。
但是，停下来算一笔账：
1. 如果用普通的带载荷枚举 `enum TaggedSlot { integer(i32), real(f32) }`，为了在运行时记住“当前存的到底是 `i32` 还是 `f32`”，编译器必须在 4 字节数据之外**额外分配一个判别标签（Tag）**。加上对齐填充，它的大小会变成 8 字节——直接把 C 语言的 4 字节硬件描述符布局撑爆。
2. 如果想用 `.discrete_tag true` 的零额外开销枚举呢？也不行。因为 `i32` 和 `f32` 几乎占满了全部 $2^{32}$ 种比特组合，它们在位模式上**完全重叠，根本没有互斥的空闲比特（Niche）可以用来区分彼此**。

当判别信息保存在外部（比如另一个硬件寄存器里），而当前这 4 个字节必须**准确无误地共享同一块物理内存**时，Flurry 的 **`union`（联合体）** 登场了（详见 `docs/type/user-defined-types/unions.md`）。

---

## 一、 带标签 `enum` 与无标签 `union` 的内存正面对决

下面先用 `enum` 写出带内置标签的版本，再引入 `union` 写出遵循 C ABI 共享存储的版本，看看它们在内存里的本质差异。

### 安全但占用了额外空间的 `enum`

在一般业务逻辑里，只要无需强行塞进外部无标签的 4 字节槽位，你永远应该首选 `enum`：

```flurry
use std.prelude.*;

-- 自带变体判别标签的安全 enum
enum TaggedNumber {
    integer(i32),
    real(f32),
}

fn show_number(number: TaggedNumber) {
    -- 因为 enum 自己记住了当前是哪个变体，所以可以安全地直接 match。
    number match {
        .integer(val) => println("integer: {}", val),
        .real(val) => println("real: {}", val),
    }
}

test {
    show_number(TaggedNumber.integer(12));
    show_number(TaggedNumber.real(1.5));
}
```

### 零额外标签、严格遵循 C ABI 的 `union`

现在，为了精确匹配硬件手册里的 4 字节共享槽位，我们改用 **`union`** 定义 `Number`：

```flurry
use std.prelude.*;

-- 多个成员共享同一块存储，布局遵循目标 C ABI
union Number {
    integer: i32,
    real: f32,
}
```

下面把 `enum TaggedNumber` 和 `union Number` 放在显微镜下对比：

```text
  [ enum TaggedNumber ] —— 自带判别员的“双人间” (通常 8 Bytes)
  ┌──────────────┬──────────────┬──────────────────────────────┐
  │  Tag (判别码) │   Padding    │     Payload (4 Bytes)        │
  │  0 = integer │   (对齐填充)  │  [ i32 ]  或  [ f32 ]        │
  │  1 = real    │              │                              │
  └──────────────┴──────────────┴──────────────────────────────┘
   ▲ 每次 match 时，编译器先检查 Tag，难以把 f32 错当成 i32 读出来。

  [ union Number ] —— 没有门牌号的“共享单间” (严格 4 Bytes, C ABI!)
  ┌────────────────────────────────────────────────────────────┐
  │                 Shared Storage (4 Bytes)                   │
  │  integer (i32): [ Bit 31 ........................ Bit 0 ]  │
  │  real    (f32): [ Sign | Exponent (8b) | Mantissa (23b) ]  │
  └────────────────────────────────────────────────────────────┘
   ▲ 两个成员叠在同一块物理地址上。盒子里没有任何 Tag 记录当前住的是谁。
```

> **🔍 语法小贴士：`union` 的声明与构造语法**
> 仔细看 `union Number` 的定义：
> 1. **成员声明**：采用 `成员名: 类型` 的形式，成员之间用**逗号 `,`** 分隔（例如 `integer: i32, real: f32,`）。每个成员类型都必须具有固定大小的表示（Fixed-size representation）。
> 2. **构造实例**：使用 **`UnionType.member_name(value)`** 选择具体成员来构造一个 `union` 值。例如 `Number.integer(12)` 或 `Number.real(1.5)`。
> 3. **为什么构造是安全的，而后续操作需要 `unsafe`？**
>    当你写 `var slot = Number.integer(12);` 时，你只是把一个合法的 `i32` 放进了 4 字节槽位里，没有任何歧义，因此在普通安全代码中就可以直接构造。但是，一旦构造完成，由于 `union` 自身不保存标签，**除构造以外的任何 `union` 实例操作（如通过指针覆写共享存储、解释读取成员）都必须放在 `unsafe` 作用域或 `unsafe fn` 中**，由程序员为“当前存储里的位模式确实对应哪个成员”立下军令状。

```flurry
-- ❌ 编译期直接报错：union 没有内置判别标签，无法像 enum 那样在安全代码里直接 match
fn bad_match(slot: Number) {
    slot match {
        .integer(v) => println("{}", v),
        .real(v) => println("{}", v),
    }
}
```

---

## 二、 核心问题——为什么给 `union` 赋值非自动调用旧成员的 `Drop`？

现在，把安全带系紧。我们要触碰系统编程语言里最容易踩爆的地雷之一：**当 `union` 遇上资源清理（`Drop`）和覆盖赋值时，会发生什么？**

回忆一下我们在 [第 003 篇](./003-let-does-not-freeze-your-object.md) 学过的普通变量覆盖赋值核心规则：
对于普通的 `struct` 或 `enum` 变量 `var x = old_val;`，当你执行 `x = new_val;` 时，编译器会：
1. 先求值 `new_val`；
2. **自动调用 `old_val` 的 `Drop` 清理旧资源**；
3. 把 `new_val` 写入 `x`。

那么问题来了：如果 `slot` 是一个指向 `union` 的指针 `*Number`，或者一个 `union` 变量，当我们执行 **`slot.* = Number.real(1.5);`** 时，编译器是否会自动帮我们调用旧成员的 `Drop`？

**不会。在 Flurry 中，`union` 赋值被明确规定为“不隐式执行旧成员的 `Drop`”。**

为什么？下面来看看如果编译器隐式去调 `Drop` 会发生什么内存安全问题：

```text
  假设 union PacketUnion 包含两个成员：
    - raw_id:   u64          (纯数字，比如 0x0000_0000_0000_1234，无需 Drop)
    - buf_ptr:  OwnedBuffer  (持有堆内存指针，Drop 时会调用 free(ptr) 释放内存。)

  此时共享存储里放的是：PacketUnion.raw_id(0x1234)
  ┌──────────────────────────────────────────────────────┐
  │ Shared Storage: 0x0000_0000_0000_1234                │
  └──────────────────────────────────────────────────────┘
                          │
      现在执行赋值：slot.* = PacketUnion.buf_ptr(new_buf);
                          ▼
  ❓ 编译器灵魂发问：“我要切勿在写入 new_buf 之前，把旧值 Drop 掉？”
     - 如果按 raw_id (u64) 处理：什么都无须做。
     - 如果按 buf_ptr (OwnedBuffer) 处理：把整数 0x1234 当成堆指针传给 free(0x1234) ──► 💥 当场段错误崩溃。
     - 可是 union 里根本没有 Tag。编译器在运行时怎么可能知道里面之前装的是 raw_id 还是 buf_ptr？
```

看明白了吗？
因为 `union` 的核心契约就是**“不携带自动活跃成员状态（No automatic active-variant state）”**。既然没有内置标签记录上次写入的是哪个成员，编译器就没有任何通用法则能猜出旧存储当前应该按哪个成员来释放。

因此，Flurry 在语言规范（`docs/type/user-defined-types/unions.md` 与 `docs/resource-management/bindings.md`）中划下了一条清晰的红线：
- **普通绑定赋值**：编译器知道确切类型（或 `enum` 自带运行时标签），自动执行旧值 `Drop`。
- **`union` 共享存储赋值**：**非自动插入旧成员析构。** 如果旧成员持有需要清理的资源，程序员必须在覆写 `union` 之前，根据外部状态显式完成旧成员资源的移出或清理；然后才能在 `unsafe` 块中覆写共享存储。

来看看遵循这一契约的标准 进阶代码：

```flurry
use std.prelude.*;

union Number {
    integer: i32,
    real: f32,
}

-- 构造以外的 union 实例操作（包括整体替换和成员读取/写入）均要求 unsafe
unsafe fn replace_with_real(slot: *Number, value: f32) {
    -- 直接以 f32 成员的位表示替换共享存储，免于触发任何隐式旧成员 Drop
    slot.* = Number.real(value);
}

unsafe fn read_real(slot: *Number) -> f32 {
    -- 在 unsafe 边界内按当前活跃成员 real 读取共享存储
    slot.real.*
}

test {
    -- 1. 初始构造：明确选择 integer 成员写入 12（安全代码可直接构造）
    var slot = Number.integer(12);

    -- 2. 在 unsafe 边界内更新共享存储为浮点数 1.5 并读取
    unsafe {
        replace_with_real(slot.ref, 1.5);
        println("real member: {}", read_real(slot.ref));
    }

    -- 3. 类型反射查询（如 Type.size_of）观察的是 Number 在目标 C ABI 下的布局，
    --    这属于类型描述查询，完全安全且在编译期即可确定。
    println("storage size: {}", Type.size_of(Number));
}
```

```flurry
-- ❌ 编译期直接报错：在并非 unsafe 函数中直接读取或写入已构造的 union 实例成员
fn bad_access(slot: *Number) -> i32 {
    slot.* = Number.real(3.14);
    slot.integer.*
}
```

---

## 三、 给野马套上缰绳——用外部标志封装安全的硬件描述符

既然裸操作 `union` 需要 `unsafe`，我们在写固件驱动时，难道要让业务代码里到处飘满 `unsafe { ... }` 吗？
当然不。系统编程的最佳实践，永远是**把 `union` 关进一个小型封装结构体里，用外部寄存器或状态位作为判别依据，向业务层暴露 100% 安全的接口**。

同时，还要谨记 `docs/type/user-defined-types/unions.md` 中的另一条核心规则：
> **“进入 `unsafe` 亦无法凭空授予任意位串作为任意成员值的合法性。”**

比如你在 `union` 里放了一个布尔成员 `flag: bool`（合法位模式只有 `0` 和 `1`）和一个字节成员 `raw: u8`。如果你先写入了 `raw(255)`，就算钻进 `unsafe` 块里把它当成 `bool` 读出来，那也是违反类型表示契约的非法行为。

而在我们的硬件校准描述符里，`i32` 和 `f32` 都是 4 字节定宽类型，且我们有一个明确的外部模式位 `is_float: bool` 与之配对：

```flurry
use std.prelude.*;

union CalibrationPayload {
    offset: i32,
    scale: f32,
}

-- 用结构体将外部控制位与共享存储封装在一起
struct DmaCalibrationDesc {
    is_float: bool,
    payload: CalibrationPayload,

    -- 安全构造器 1：整数偏移模式
    pub fn from_offset(offset: i32) -> DmaCalibrationDesc {
        DmaCalibrationDesc {
            is_float: false,
            payload: CalibrationPayload.offset(offset),
        }
    }

    -- 安全构造器 2：浮点缩放模式
    pub fn from_scale(scale: f32) -> DmaCalibrationDesc {
        DmaCalibrationDesc {
            is_float: true,
            payload: CalibrationPayload.scale(scale),
        }
    }

    -- 安全切换方法：把 unsafe 严格收敛在两行代码之内，同时同步更新外部标志位。
    pub fn switch_to_scale(*self, scale: f32) {
        self.is_float.* = true;
        unsafe {
            self.payload.* = CalibrationPayload.scale(scale);
        }
    }
}

test {
    let desc = DmaCalibrationDesc.from_offset(-50);
    println("is_float before: {}", desc.is_float);

    desc.switch_to_scale(1.25);
    println("is_float after: {}, payload size: {}",
        desc.is_float, Type.size_of(CalibrationPayload));
}
```

看看 `DmaCalibrationDesc`：
1. 底层 `CalibrationPayload` 严格保持 C ABI 的 4 字节 `union` 布局，可以直接映射给 C 驱动或硬件 DMA 寄存器；
2. 每次切换活跃成员时，`switch_to_scale` 负责同时更新 `self.is_float.*` 和 `self.payload.*`，将 `unsafe` 锁死在底层封装内部；
3. 外部调用者只需要调用 `from_offset`、`from_scale` 和 `switch_to_scale`，连一个 `unsafe` 关键字都省去书写。

---

## 本篇小结

对比 Flurry 中处理“多变体数据”的三种武器，你就能在任何场景下做出最精准的选型：

| 类型工具 | 内存布局与判别机制 | 适用场景 | 安全与 `Drop` 契约 |
| :--- | :--- | :--- | :--- |
| **普通 `enum`** | 编译器自动管理 Tag + 载荷 | 95% 的高层业务逻辑与状态机 | 100% 安全 `match`；覆盖赋值自动清理旧变体 |
| **定宽模式 `enum`** (`.base_type` + `.discrete_tag`) | 零额外 Tag，靠变体间的互斥比特位模式判别 | 协议报文、指令集解码（如 RISC-V）、紧凑标志位 | 编译期证明变体互斥；100% 安全构造与匹配 |
| **`union`** | 严格 C ABI 共享存储，**无内置 Tag** | 对接 C 语言 `union`、由外部状态位决定含义的硬件寄存器 | 构造安全，其余实例操作需 `unsafe`；**赋值非自动触发旧成员 `Drop`** |

---

### 下一篇预告

在编写 DMA 描述符和传感器通道管理器时，我们顺手写了好几个 `u32` 类型的字段和参数：
- 设备硬件地址 `device_id: u32`
- 传感器通道索引 `channel_idx: u32`
- 采样超时毫秒数 `timeout_ms: u32`

为了代码好看，我们在文件开头写了三行类型别名：
```flurry
typealias DeviceId = u32;
typealias ChannelIndex = u32;
typealias TimeoutMs = u32;
```
结果昨天晚上联调时，同事在调用 `configure_dma(dev: DeviceId, ch: ChannelIndex, timeout: TimeoutMs)` 时，手滑写成了 `configure_dma(dev_id, 5000, ch_idx)`——**把 5000 毫秒的超时当成通道号传了进去，把通道号 2 当成了 2 毫秒超时。**

更要命的是，编译器却**一声不吭地编译通过了**。
为什么 `typealias` 拦不住这种张冠李戴的低级错误？如果我们想要创造一个**底层机器表示 100% 还是 `u32`（零运行时开销），但在类型系统里拥有完全独立身份、甚至连 `u32` 的方法都免随便暴露出来的全新类型**，该用什么？

在下一篇 **《Flurry 演进手记 (018)：同样是 u32，凭什么禁止混用——newtype 独立类型身份与 typealias 透明别名》** 中，我们将用 `newtype` 和 `.as(View)` 类型视角终结“基本类型参数传错位置”的难题。
