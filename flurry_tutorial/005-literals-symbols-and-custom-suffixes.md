# Flurry 演进手记 (005)：给数字贴上单位标签——编译期字面量、Symbol 与 LiteralInto 后缀协议

在 [上一篇 (004)](./004-why-a-lt-b-is-not-less-than.md) 中，我们掌握了 Flurry 的词法边界与空白规则。现在，让我们回到那块 8 比特传感器的电路板上，编写它的**采样时序控制与校准模块**。

在嵌入式与系统编程中，最昂贵的 Bug 往往不是高深的并发死锁，而是极其低级的**物理单位混淆**（1999 年 NASA 火星气候探测器的坠毁，就是因为一个模块传了磅力秒，另一个模块当成了牛顿秒）：
- 硬件手册规定：传感器唤醒需要等待 **100 毫秒（ms）**，随后每隔 **250 微秒（us）** 采集一次，采样电压乘以校准系数 **`1.5 × 10⁻³`**；
- 同时，驱动层需要根据轻量级的命令标签（如 `.reset`、`.measure`、`.sleep`）切换模式，并在启动时打印一段包含固件版本号和多行硬件拓扑图的横幅日志。

如果我们像在传统语言里那样，把时间、电压、命令全写成裸露的 `u32`、`f64` 和字符串，会发生什么？下面我们逐步分析。

---

## 一、 裸数字、机制字符串与“毫秒变微秒”的惨剧

先看第一种直观写法：

```flurry
use std.prelude.*;

-- 参数要求传入微秒 (us)，但类型只是毫无防备的 u32。
fn wait_microseconds(delay_us: u32) {
    println("Waiting {} us...", delay_us);
}

fn dispatch_cmd(cmd: str) {
    if cmd == "measure" {
        -- 问题：工程师想等 100 毫秒 (100_000 微秒)，却随手传了 100。
        -- 硬件只等了 100 微秒就强行读取，采回来一堆乱码。
        wait_microseconds(100u32);
    }
}
```

不仅如此，当这位工程师尝试在代码里写几个常数时，还接连撞上了两次编译错误：

```flurry
-- ❌ 编译期直接拒绝。
const BAD_HEX = 0xAb_cD;   -- 错误 1：十六进制字母大小写混用。
const BAD_FLOAT1 = .5;     -- 错误 2：小数点前缺少数字。
const BAD_FLOAT2 = 1e-3;   -- 错误 3：科学计数法指数 `e` 前面必须是完整小数。
```

为什么 Flurry 对字面量的拼写如此挑剔？又该如何给 `100` 贴上真正的物理单位标签 `100ms` 和 `100us`？
让我们先揭开 Flurry 字面量的编译期本质。

---

## 二、 来自编译期宇宙的 `Integer` 与 `Float`、字符串插值与 `Symbol`

### 1. 为什么整数字面量的类型叫 `Integer` 而不是 `i32`？

在 C 或 Rust 中，你写下一个数字 `42`，编译器马上就想把它塞进某个固定宽度的机器寄存器（如 `i32`）里。
但在 Flurry 中，语言被清晰地划分为**编译期宇宙（Comptime Universe）**与**运行时宇宙（Runtime Universe）**（详见 `docs/basics/primitives.md` 与 `docs/basics/literals.md`）：

```text
  编译期宇宙 (Comptime)                    运行时宇宙 (Runtime)
  ┌─────────────────────────┐              ┌─────────────────────────┐
  │ 42, 0xFF_00, 1_000_000  │              │ i8, u8, i32, u32, usize │
  │ 类型: Integer (任意精度) │ ──(Splice)──►│ (按目标上下文落地为定宽；│
  ├─────────────────────────┤              │  无上下文时默认回退 i32)│
  │ 3.14, 1.5e-3            │              ├─────────────────────────┤
  │ 类型: Float   (编译期)   │ ──(Splice)──►│ f32, f64                │
  └─────────────────────────┘              │ (默认回退 f64)          │
                                           └─────────────────────────┘
```

当你写下 `1_000_000` 时，它在编译期拥有无限精度的 **`Integer`** 类型；只有当它进入运行时上下文（这个跨宇宙落地的过程叫做 **Splice**）时，才会根据目标类型变成 `u32` 或 `i64`（如果上下文没有任何线索，无后缀整数默认按 `i32` 处理，无后缀浮点默认按 `f64` 处理）。

而为了让你在阅读底层硬件代码时绝对不会看走眼，Flurry 对数值字面量制定了三条防呆核心规则：
1. **进制前缀与 `_` 分隔符**：二进制 `0b`、八进制 `0o`、十六进制 `0x` 前缀必须小写；数字之间可以插入 `_`（如 `1_000_000`、`0b1010_0101`），但不能以 `_` 开头（`_23` 是一个标识符，不是数字。）。
2. **十六进制大小写一致性**：在 `0x...` 中，去掉 `_` 之后，字母 `a–f` **必须全小写（`0xff_23`）或全大写（`0xFF_23`）**，严禁写出 `0xAb_cD` 这种大小写混杂的视觉污染。
3. **浮点数必须“有头有尾”**：浮点数必须写成 `digits.digits` 形式。**严禁写 `1.`、`.5` 或 `1e-3`。** 为什么？试想在支持后缀方法调用的语言里，如果允许 `1.`，那么 `1.as(u32)` 到底是浮点数 `1.0` 还是整数 `1` 调用 `.as(u32)`？因此，科学计数法的指数 `e`/`E` 前面也必须是完整的小数，必须老老实实写成 **`0.5`** 和 **`1.5e-3`**。

### 2. 编译期字符串插值 `"$name"` 与原始字符串 `@builtin.str.raw`

在准备固件启动日志时，我们经常需要把编译期常量（如版本号、构建目标）拼进字符串里，或者嵌入一段包含大量引号和反斜杠的硬件配置文本。

Flurry 的字符串（`str`）为此提供了三件贴心武器：
- **编译期插值 `"$name"` 与 `"${expr}"`**：在双引号字符串里，用 `$` 即可直接嵌入 **comptime 值**（等价于调用一次 `std.fmt.comptime_format`）。注意：**只有 `$` 会触发插值，单独的 `{` 和 `}` 只是普通字符**。如果你想写字面意义的美元符号，只需转义写成 `\$`。
- **相邻字符串自动合并**：两个双引号字符串中间只要只有空白（如 `"Hello, " "Flurry!"`），词法器就会自动把它们合并成一个字符串。
- **原始字符串构造宏 `@builtin.str.raw { ... }`**：花括号内部的内容按原样保留，再也不用数反斜杠了。

```flurry
const FIRMWARE_VER: str = "0.4.2";
const BOARD_NAME: str = "Luna-Sensor-RevB";

-- $FIRMWARE_VER 和 ${BOARD_NAME} 在编译期直接完成格式化拼接。
-- 注意后面的 `{ ready: true }`：因为前面没有 `$`，花括号就是普通字符，不用转义。
const BOOT_BANNER: str = "Booting $FIRMWARE_VER on ${BOARD_NAME} { ready: true } (cost: \$5)";

const TOPOLOGY_MAP = @builtin.str.raw {
    [MCU] ---> (UART0: "115200\n") ---> [Sensor #1]
}
```

### 3. 轻量级符号 `.id`（`Symbol`）与 `'tag_name` 视图

再看前面代码中用字符串 `"measure"` 作命令标签的问题。用运行时字符串比较既慢，拼错了 `"mesure"` 编译器也发现不了；但如果每次为了两三个临时标签都去专门声明一个顶层 `enum`，有时候又显得太重。

为此，Flurry 提供了内建的**符号字面量（Symbol Literal）**：以点号开头的 **`.id`**，它的类型是 **`Symbol`**。

```text
  Symbol (`.ready`, `.measure`, `.reset`) 的三大特性：
  ┌──────────────────────────────────────────────────────────────────┐
  │ 1. 编译单元内“同名同值”：所有 `.measure` 在底层共享同一个轻量级标号，│
  │    判等 `tag == .measure` 就像比较整数标号一样快                    │
  │ 2. 两宇宙通行：既能作 comptime 常量和属性键，也能在 runtime 传递。 │
  │ 3. 绝不与字符串混淆：`.ok` 不是 `"ok"`  但可用 `.ok'tag_name`      │
  │    通过 take view 零拷贝拿到对应的字符串 `"ok"`                    │
  └──────────────────────────────────────────────────────────────────┘
```

更聪明的是 **`.id` 与 `enum` 的上下文协同**（详见 `docs/basics/literals.md`）：
- 当目标位置的上下文明确是一个 `enum`（比如传给接收 `Packet` 的函数，或者匹配 `Packet`）时，编译器会优先在那个 `enum` 里解释 `.empty`；
- 当目标上下文没有指定任何 `enum` 时，`.reset`、`.measure` 就自然地作为独立的 `Symbol` 类型值。

来看看用 `Symbol` 和 `'tag_name` 改造后的命令分发：

```flurry
fn describe_symbol(cmd: Symbol) -> str {
    -- `'tag_name` 视图：直接取出 Symbol 对应的字符串名称（如 .measure -> "measure"）
    cmd'tag_name
}

fn is_active_cmd(cmd: Symbol) -> bool {
    -- 注意：因为 Symbol 的集合是开放的，match Symbol 时必须有 `_` 兜底分支。
    cmd match {
        .measure => true,
        .calibrate => true,
        _ => false,
    }
}

test {
    let current_cmd = .measure;
    assumes is_active_cmd(current_cmd);
    assumes current_cmd != .sleep;
    assumes describe_symbol(.reset) == "reset";
}
```

---

## 三、 自定义字面量后缀——`LiteralInto<.suffix, Result>` 协议

现在，到了解决本篇最核心问题的时候了：**怎么防止把 `100` 毫秒当成 `100` 微秒传给 `wait_microseconds`？**

你早就见过 `100u32`、`2.718f32` 这样的写法。在大多数语言里，`u32`、`f32` 是编译器写死的特权语法，用户自己定义的类型根本享受不到这种待遇（或者像 C++ `operator"" _ms` 那样必须加下划线，且缺乏基于目标类型的统一重载解析）。

但在 Flurry 中，**所有的字面量后缀（包括整数、浮点、字符、字符串后面的标识符）都走同一套公开的 Trait 语法协议：`LiteralInto<suffix, Result>`。**（详见 `docs/traits/protocols.md`）。

当你写下 **`100ms`** 时，编译器到底在背后做了什么？让我们画一张 

```text
  源码写下：  let delay: Micros = 100ms;
                                  ───┬─
                                   │ └──── 2. 后缀标识符 `ms` ──► 变成符号 `.ms`
                                   └────── 1. 字面量主体 `100` ──► 类型 S = Integer
                                  │
                                  ▼
  编译器发起 Trait 协议查询：
  ┌────────────────────────────────────────────────────────────────┐
  │ 查询目标： LiteralInto<.ms, Result> for Integer                │
  │ 已知条件： S = Integer, suffix = .ms, 上下文期望 Result = Micros│
  └────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼ (唯一命中实现。)
  自动改写为调用：
  LiteralInto.literal_into(100)  ──►  在编译期/初始化时算出 Micros { us: 100_000u32 }。
```

看到了吗？在 `LiteralInto<.ms, Result> for Integer` 中：
- 第一个参数是**后缀对应的 `Symbol`**（即 `.ms`）；
- 第二个参数是**转换的目标类型 `Result`**；
- 主体 `Self` 是**字面量本身的类型**（这里是 `Integer`，也可以为 `Float`、`str`、`char` 实现后缀。）。

因为 `Result` 也参与 Trait 实例区分，同一个 `.ms` 后缀甚至可以根据上下文目标类型的不同，转换到不同的时间表示类型（只要在当前调用上下文能唯一确定合法结果 `Result`；如果无法唯一确定，编译器会诚实地报告歧义错误，绝不乱猜）。

让我们亲手定义微秒类型 `Micros`，并为 `Integer` 实现 `.us`（微秒）和 `.ms`（毫秒，自动乘以 1000 转为微秒）两个字面量后缀：

```flurry
use std.prelude.*;

struct Micros {
    us: u32,

    fn new(us: u32) -> Self {
        Self { us: us }
    }
}

-- 1. 实现 `us` 后缀：100us -> Micros { us: 100 }
impl LiteralInto<.us, Micros> for Integer {
    fn literal_into(comptime value: Integer) -> Micros {
        Micros.new(value.as(u32))
    }
}

-- 2. 实现 `ms` 后缀：100ms -> Micros { us: 100 * 1000 }
impl LiteralInto<.ms, Micros> for Integer {
    fn literal_into(comptime value: Integer) -> Micros {
        Micros.new((value * 1_000).as(u32))
    }
}

-- 现在，延时函数只接收类型安全的 Micros，绝不接收裸 u32。
fn wait_sensor(delay: Micros) {
    println("Waiting {} microseconds", delay.us);
}

test {
    -- 无论是写 250us 还是 100ms，都直接、清晰、零歧义地变成 Micros。
    let sample_interval: Micros = 250us;
    let wakeup_delay: Micros = 100ms;

    assumes sample_interval.us == 250u32;
    assumes wakeup_delay.us == 100_000u32;

    wait_sensor(250us);
    wait_sensor(100ms);
}
```

看看 `wait_sensor(250us)` 和 `wait_sensor(100ms)`。
- 如果有人再手滑写 `wait_sensor(100)`，编译器编译期拒绝（`Integer` 不是 `Micros`）；
- 每一个时间常数后面都必须挂着清晰的物理单位 `us` 或 `ms`，既有脚本语言般的优雅可读性，又在编译期通过 `comptime value: Integer` 直接算出结果，运行时零额外解析开销。

> **🔍 语法小贴士：为什么不能叠加两个后缀写 `100u32ms`？**
> 也许你会突发奇想：“既然 `u32` 是后缀，`ms` 也是后缀，我能不能连着写 `100u32ms`？”
> **绝对不行。**
> ```flurry
> -- ❌ 编译期直接拒绝。
> let bad: Micros = 100u32ms;
> 
> ```
> 根据 `docs/basics/literals.md` 的词法产生式 `literal_suffix -> (integer | float | char | str) id`，字面量后面**只能跟一个标识符后缀**。当你写下 `100u32ms` 时，词法器会把 `u32ms` 识别为**单个名叫 `.u32ms` 的后缀标识符**，而不是“先转 `u32` 再转 `ms`”的两层后缀；紧接着编译器去查 `LiteralInto<.u32ms, Micros>` 找不到实现，就会立刻报错。

---

## 本篇小结

通过改造传感器的时序与配置模块，我们解锁了 Flurry 的字面量与符号体系：

| 字面量 / 机制 | 示例写法 | 编译期类型 | 核心规则与安全保证 |
| :--- | :--- | :--- | :--- |
| **整数字面量** | `1_000`, `0xFF_00`, `0b1010` | `Integer` | 进制前缀小写；`0x` 字母大小写须统一；无上下文默认回退 `i32` |
| **浮点字面量** | `0.5`, `1.5e-3` | `Float` | 必须为 `digits.digits`（拒绝 `.5`、`1.`、`1e-3`）；默认回退 `f64` |
| **字符串与插值** | `"ver: $VER"`, `@builtin.str.raw { ... }` | `str` | `$name` / `${expr}` 仅限 comptime 插值；单独 `{}` 不转义；相邻字符串自动合并 |
| **符号 (`Symbol`)** | `.measure`, `.ok'tag_name` | `Symbol` | 编译单元内同名同值；有 `enum` 上下文时优先解析为变体；`'tag_name` 取名称字符串 |
| **自定义后缀** | `100ms`, `250us`, `"path"c` | 由 `LiteralInto` 决定 | 查询 `LiteralInto<.suffix, R> for S`；禁止叠加双后缀（如 `100u32ms`） |

### 下一篇预告

仔细看我们在上面代码里写过的 `wait_sensor(delay: Micros)`——它没有写 `-> ReturnType`，也就是说它的返回类型是 `void`。
而在之前的章节里，我们还见过：
- 在 `match` 分支里什么都不做时写的 **`()`**；
- 在返回 `?u8` 的函数里表示空心跳包时写的 **`null`**；
- 在模式匹配和 `let _ = ...` 里用来忽略值的 **`_`**。

这时候，一位写慣了 C 和 Zig 的系统程序员凑过来问了一个极其危险的问题：
> “如果我要分配一个容纳 64 个传感器样本的缓冲区，在硬件 DMA 把数据填进去之前，我不想浪费 CPU 周期去把内存清零——Flurry 里有没有表示**未初始化内存**的 **`undefined`**？`()`、`null`、`_` 和 `undefined` 这四个代表‘空无一物’的家伙，在底层生命周期和安全性上到底有什么区别？特别是如果我写了 `let _ = open_monitor(1u8);`，通道是当场关闭，还是等函数结束才关闭？”

在下一篇 **《Flurry 演进手记 (006)：空无一物的三种面孔——`()`、`null`、`_` 与危险的 `undefined`》** 中，我们将把这四个最容易被混为一谈的“空”彻底扒开。
