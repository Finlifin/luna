# Flurry 演进手记 (018)：同样是 u32，凭什么禁止混用——newtype 独立类型身份与 typealias 透明别名

在 [上一篇 (017)](./017-unions-shared-storage-and-c-abi.md) 的结尾，我们经历了一场每一个系统程序员都似曾相识的午夜惊魂：
在配置传感器 DMA 通道的函数里，有三个参数底层全都是 32 位无符号整数 `u32`：
- 设备地址 `DeviceId`
- 通道编号 `ChannelIndex`
- 超时时间 `TimeoutMs`

为了提高代码可读性，我们写了三个 `typealias`（类型别名）。结果调用者把 `timeout` 和 `channel` 的参数顺序写反了——用 5000 毫秒的超时去索引第 5000 号硬件通道，编译器却连个警告都没给，一路绿灯放行。

为什么 `typealias` 拦不住参数错位？如果我们在 C 或 Rust 里用单字段结构体 `struct DeviceId { raw: u32 }` 包装一层，每次取值都要写 `.raw`、构造都要写 `{ raw: ... }`，非但繁琐，有时还会担心 ABI 传递约定是否会和裸 `u32` 有所差异。

在 Flurry 中，**“给现有类型起个小名”**和**“基于现有类型的物理表示创造一个全新的独立类型”**，由两个迥然各异的关键字掌管：**`typealias`** 与 **`newtype`**（详见 `docs/type/user-defined-types/newtype-and-typealias.md` 与 `docs/type/receiver-adaptation.md`）。

---

## 一、 `typealias` 的真相——它只是个“绰号”，不是身份证

先看使用透明类型别名时的情形：

```flurry
use std.prelude.*;

-- 使用 typealias 起别名
typealias DeviceId = u32;
typealias ChannelIndex = u32;
typealias TimeoutMs = u32;

fn configure_channel(dev: DeviceId, ch: ChannelIndex, timeout: TimeoutMs) {
    println("Configuring dev={}, ch={}, timeout={}ms", dev, ch, timeout);
}

test {
    let dev: DeviceId = 1u32;
    let ch: ChannelIndex = 2u32;
    let timeout: TimeoutMs = 5000u32;

    -- 问题发生：把 timeout (5000) 传给了 ch，把 ch (2) 传给了 timeout。
    -- 编译器认为它们全都是 u32，100% 合法通过。
    configure_channel(dev, timeout, ch);
}
```

为什么会这样？下面来看看编译器眼里的 `typealias`：

```text
  [ typealias 透明别名 ] —— 三个名字贴在同一个类型本体上。

    DeviceId ─────────┐
                      ▼
    ChannelIndex ──► ┌──────────────────────────┐
                      │   类型身份: u32           │
    TimeoutMs ────────┘   (同一个 Nominal ID!)   │
                      └──────────────────────────┘

  当编译器检查 configure_channel(dev, timeout, ch) 时：
  形参要求: (u32, u32, u32)
  实参传入: (u32, u32, u32)  ──► 完全相等。放行。
```

**`typealias` 免增新的类型身份（Type Identity）。**
它的设计目的非常纯粹：当你有一个很长的类型（比如 `BitVec<4>` 或者 `Tagged<bool>.Wrapped<u32>`）时，用 `typealias Nibble = BitVec<4>;` 给它起一个简短好记的绰号。无论你在哪里写 `Nibble`，对编译器来说都跟直接写 `BitVec<4>` 没有任何区别。

那如果我们是真的想在“设备 ID”、“通道号”和“超时毫秒数”之间筑起一道编译器级别的防火墙呢？

---

## 二、 `newtype` 登场——零运行时开销的独立类型身份

把关键字从 `typealias` 换成 **`newtype`**，问题随即解决：

```flurry
use std.prelude.*;

-- typealias：保留给无需隔离身份的简写（比如 Count 就是普通 u32）
typealias Count = u32;

-- newtype：建立拥有全新独立身份的类型，同时 100% 保持底层 u32 的表示。
newtype DeviceId = u32;
newtype ChannelIndex = u32;
newtype TimeoutMs = u32;

fn configure_channel(dev: DeviceId, ch: ChannelIndex, timeout: TimeoutMs) {
    -- 在内部需要当成普通 u32 打印或运算时，用 .as(u32) 显式转换回底层类型
    println("Configuring dev={}, ch={}, timeout={}ms",
        dev.as(u32), ch.as(u32), timeout.as(u32));
}

test {
    let raw_count: Count = 1u32;

    -- 构造 newtype：先拿到合法的底层值，再用 .as(NewType) 显式包装。
    let dev = raw_count.as(DeviceId);
    let ch = 2u32.as(ChannelIndex);
    let timeout = 5000u32.as(TimeoutMs);

    configure_channel(dev, ch, timeout);
}
```

现在，如果那個粗心的同事再敢把参数传反，或者拿一个普通的 `Count`（即 `u32`）直接塞给 `DeviceId`，编译器当场就会把他的代码拍回去：

```flurry
test {
    let dev = 1u32.as(DeviceId);
    let ch = 2u32.as(ChannelIndex);
    let timeout = 5000u32.as(TimeoutMs);
    let count: Count = 1u32;

    -- ❌ 编译期直接报错：TimeoutMs 与 ChannelIndex 具有不同的类型身份。
    configure_channel(dev, timeout, ch);

    -- ❌ 编译期直接报错：Count 只是 u32 的透明别名，禁止直接充当 DeviceId。
    configure_channel(count, ch, timeout);
}
```

下面来看看 `newtype` 是如何做到**“物理表示零开销、逻辑身份硬隔离”**的：

```text
  [ newtype 独立身份 ] —— 物理上是一模一样的 4 字节盒子，但贴上了不可互换的钢印。

               .as(DeviceId)
   ┌─────────┐ ─────────────► ┌────────────────────┐
   │         │  .as(u32)      │ DeviceId (ID #101) │ (32 bits, C ABI 与 u32 完全一致)
   │         │ ◄───────────── └────────────────────┘
   │   u32   │                ┌────────────────────┐
   │ (底层值) │ ─────────────► │ ChannelIndex (#102)│ (❌ 与 #101 并不相等，也不能直接互转。)
   │         │ ◄───────────── └────────────────────┘
   └─────────┘                ┌────────────────────┐
               ─────────────► │ TimeoutMs (ID #103)│
                              └────────────────────┘
```

注意图中一个尤为重要的细节（详见 `docs/type/user-defined-types/newtype-and-typealias.md`）：
- **`newtype` 与它的底层类型（如 `DeviceId` 与 `u32`）之间拥有直接表示边**，因此可以通过 `.as(DeviceId)` 包装、通过 `.as(u32)` 解包，且在机器码层面是**零指令开销**的。
- 但是，**两个各异的 `newtype` 之间（如 `DeviceId` 与 `ChannelIndex`）并没有直接表示边**。你无法 因为 它们底层都是 `u32` 就直接写 `dev.as(ChannelIndex)`。这种“只沿着声明的表示边单步包装/解包”的设计，消除了不同领域概念之间的意外穿透。

---

## 三、 为什么 `newtype` 默认免于继承底层方法？以及 `.as(View)` 视角机制

现在，假设我们有一个表示平面坐标的底层结构体 `Point`，上面定义了一个方法 `horizontal(self) -> i32`。
如果我们用 `newtype ScreenPoint = Point;` 定义了一个屏幕像素坐标类型，能否直接在 `screen: ScreenPoint` 上调用 `screen.horizontal()`？

试试看：

```flurry
struct Point {
    x: i32,
    y: i32,

    fn horizontal(self) -> i32 {
        self.x
    }
}

newtype ScreenPoint = Point;

-- ❌ 编译期直接报错：newtype 默认免于继承底层类型的字段、构造、方法或 trait 实现。
fn bad_call(screen: ScreenPoint) -> i32 {
    screen.horizontal()
}
```

**为什么 `newtype` 默认免于继承底层类型的所有成员方法和字段？**
试想一下：我们在前文定义了 `newtype DeviceId = u32;`。`u32` 作为一个数字，支持乘法 `a * b`、除法 `a / b`。但是，“把 1 号设备 ID 乘以 2 号设备 ID”在现实工程里有任何意义吗？毫无意义。
如果 `newtype` 自动把底层类型的所有方法、字段和算术操作一股脑儿全继承过来，那它就退化成了一个半吊子的别名，无法实现真正的**接口隔离（Interface Isolation）**。

在 Flurry 中，`newtype` 是一张干净的白纸：
1. **如果你想临时借用底层类型的某个方法**：只需用 `.as(Point)` 显式切换到 `Point` 视角：`screen.as(Point).horizontal()`。
2. **如果你想为 `newtype` 赋予专属的领域方法**：直接为它写 `impl` 块。
3. **更酷的是——类型标注中的 `.as(View)` 视角链（Type View Chains）。**

### 揭秘 `T.as(View)`：同一个底层值，按需佩戴不同的“方法滤镜”

假设在我们的固件里，同一个 `u32` 状态码，在十进制日志模块眼里是一个 `DecimalCode`，在十六进制通信协议模块眼里是一个 `ProtocolCode`。两者都有一个同名方法 `.category(self) -> u32`，但计算逻辑不同。

如果我们有一个 `u32` 变量，希望在维持原状而未改它本身是 `u32` 的前提下，让它在当前作用域里**默认优先使用 `DecimalCode` 或 `ProtocolCode` 的方法**，Flurry 允许你直接在变量的类型标注上写 **`.as(...)` 类型视角**（详见 `docs/type/receiver-adaptation.md`）：

```flurry
use std.prelude.*;

newtype DecimalCode = u32;
newtype ProtocolCode = u32;

impl DecimalCode {
    fn category(self) -> u32 {
        10u32
    }
}

impl ProtocolCode {
    fn category(self) -> u32 {
        16u32
    }
}

struct Point {
    x: i32,
    y: i32,

    fn horizontal(self) -> i32 {
        self.x
    }
}

newtype ScreenPoint = Point;

impl ScreenPoint {
    -- 为 ScreenPoint 定制专属方法，内部显式通过 .as(Point) 访问底层表示
    fn is_left_half(self) -> bool {
        self.as(Point).horizontal() < 960
    }
}

test {
    -- 1. ScreenPoint 的显式解包与专属方法
    let pt = (Point { x: 120, y: 80 }).as(ScreenPoint);
    println("x={}, left_half={}", pt.as(Point).horizontal(), pt.is_left_half());

    -- 2. 类型标注上的 `.as(View)` 视角机制。
    -- 注意：decimal 和 protocol 绑定值本身的类型依然是 u32（可以直接赋字面量 7）。
    -- 但 `.as(...)` 为它们挂载了方法查找的优先视角。
    let decimal: u32.as(DecimalCode) = 7u32;
    let protocol: u32.as(DecimalCode).as(ProtocolCode) = 7u32;

    -- 调用 .category() 时，编译器沿着视角链从右向左（D -> C -> B -> A）查找：
    -- decimal 优先命中 DecimalCode.category() -> 10
    -- protocol 最右侧视角是 ProtocolCode，优先命中 ProtocolCode.category() -> 16
    println("categories: {} {}", decimal.category(), protocol.category());
}
```

下面来看 `u32.as(DecimalCode).as(ProtocolCode)` 这一行代码背后的三层接收者适配机制：

```text
  声明: let protocol: u32.as(DecimalCode).as(ProtocolCode) = 7u32;

  1. 值的真实本体 (Base Type):  依然是 u32 (值为 7)。
  2. 挂载的视角候选列表 (从右往左优先):
     [ 第 1 顺位: ProtocolCode ] ──► [ 第 2 顺位: DecimalCode ] ──► [ 第 3 顺位: 本体 u32 ]
                  │
                  ▼ (要求与本体 u32 有双向 cast 关系，注意并非 DecimalCode -> ProtocolCode 链式转换。)
     当调用 protocol.category() 时：
     先看第 1 顺位 ProtocolCode 有没有 .category()？
     ──► 有。直接将 u32 适配为 ProtocolCode 并调用，返回 16。
```

> **💡 核心心智模型：为什么 `A.as(B).as(C)` 是相对于 `A` 的视角栈，而非 `A -> B -> C` 的转换链？**
> 仔细看 `u32.as(DecimalCode).as(ProtocolCode)`：`DecimalCode` 和 `ProtocolCode` 都是独立声明的 `newtype`，它们俩之间根本无法互相转换。
> 为什么能连着写 `.as(DecimalCode).as(ProtocolCode)`？因为在这里，它们都是**相对于底层本体 `u32` 的并行视角（Cast Candidates）**。越靠右的 `.as(...)` 优先级越高。当选中 `ProtocolCode` 时，编译器直接使用 `u32` 到 `ProtocolCode` 的合法适配，全然无需先绕道 `DecimalCode`。

---

## 本篇小结

对比 `typealias` 与 `newtype`，我们为固件的类型系统装上了精密的调节阀：

| 特性维度 | `typealias Name = Base` | `newtype Name = Base` |
| :--- | :--- | :--- |
| **类型身份（Identity）** | **免生新身份**，与 `Base` 完全等同 | **创立全新的名义身份**，与 `Base` 及其他 `newtype` 互互异 |
| **机器表示与 ABI** | 与 `Base` 相同 | **100% 保持 `Base` 的大小、对齐与合法值编码**（零运行时开销） |
| **构造与解包** | 无需转换，直接混用 | 需通过 `.as(Name)` 显式包装、`.as(Base)` 显式解包 |
| **成员与 Trait 继承** | 完全共享 `Base` 的一切成员 | **默认免于继承任何字段、方法或 Trait**（隔离接口，按需 `impl`） |
| **视角适配（`.as(View)`）** | 无需适配 | 支持在表达式上 `x.as(Base)`，或在类型标注上 `Base.as(Name)` 挂载方法查找视角 |

---

### 下一篇预告

用上了 `newtype` 和封装良好的 `struct` 之后，我们的传感器数据包模块终于具备了完整的静态安全保障：
为了防止外部业务代码直接依赖底层的比特排布或内部物理字段，我们把 `SensorReading` 定义成了一个内部字段私有的 `struct`（里面存着原始毫伏电压 `raw_mv: u16` 和内部状态码 `flags: u8`）。

但是，写上层告警逻辑的同事却跑来抱怨了：
> “以前你们直接暴露 `enum` 或公开字段的时候，我写 `reading match { ... }` 别提多爽了。现在你们把内部字段封起来了，难道我每次判断‘是否高温告警’、‘是否低电量’，都得退回到写一堆 `if reading.is_high_temp() { let c = reading.get_celsius(); ... } else if ...` 的命令式面条代码吗？”

**能否鱼与熊掌兼得——既隐匿 `struct` 或 `newtype` 的内部物理结构，又让外部调用者依然能用 `.high_temp(celsius)`、`.low_battery(mv)` 这样优雅的模式匹配来解构它？**

在下一篇 **《Flurry 演进手记 (019)：在隐匿内部结构的情况下匹配——case 自定义匹配视图》** 中，我们将解锁模式匹配与抽象封装的核心桥梁：`case` 声明。
