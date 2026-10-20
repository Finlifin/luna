# Flurry 演进手记 (019)：在不暴露内部结构的情况下匹配——case 自定义匹配视图

在 [上一篇 (018)](./018-newtype-identity-vs-typealias.md) 的结尾，我们遇到了一个在软件工程中困扰了无数语言几十年的经典两难困境——**“模式匹配的便利性”与“信息隐藏（封装）”之间的矛盾**。

来看我们手头的真实场景：
我们的硬件传感器采集包 `SensorFrame` 原本在底层由两个私有字段组成：
- `raw_code: u16`：高 8 位是设备内部校准码，低 8 位是原始温度读数（单位：半摄氏度，需要除以 2 再减去 40 才是真实摄氏度 `i16`）。
- `battery_mv: u16`：电池电压毫伏数（低于 `3300` 毫伏代表低电量告警）。

如果我们直接把 `raw_code` 和 `battery_mv` 暴露给外部业务模块去写 `{ raw_code, battery_mv }` 模式匹配，一旦下个月硬件换代、温度换算公式变了，全公司几十个业务文件里的模式匹配全都得跟着重写。
可是，如果我们把字段私有化、只提供普通的 Getter 方法，外部调用者就失去了模式匹配的超能力，被迫退回繁琐的 `if-else` 链条。

在 Flurry 中，你不需要在这两者之间做妥协。通过 **`case` 自定义匹配视图（Custom Pattern Views）**，你可以让任何封装严密的 `struct` 或 `newtype`，向外部提供像 `enum` 变体一样丝滑的模式匹配接口（详见 `docs/control-flow/pattern-matching/index.md`）。

---

## 一、 把字段藏起来之后，退回 `if-else` 的憋屈

让我们先看看如果不使用 `case` 视图，只靠普通方法（Getter / Predicate）封装 `SensorFrame` 时，调用者的代码会退化成什么样：

```flurry
use std.prelude.*;

struct SensorFrame {
    raw_temp_half_c: u16,
    battery_mv: u16,

    -- 普通方法：计算摄氏度
    pub fn celsius(*self) -> i16 {
        (self.raw_temp_half_c / 2u16).as(i16) - 40i16
    }

    -- 普通方法：判断是否高温告警 (>= 85°C)
    pub fn is_overheated(*self) -> bool {
        self.celsius() >= 85i16
    }
}

-- 调用者被迫放弃模式匹配，写回命令式的 if-else 嵌套
fn handle_frame_initial(frame: SensorFrame) {
    if frame.is_overheated() {
        -- 问题 1：is_overheated() 内部已经算过一次 celsius()，这里为了拿值又得算一次。
        let temp = frame.celsius();
        println("CRITICAL: Overheated at {} C (bat={} mV)!", temp, frame.battery_mv);
    } else if frame.battery_mv < 3300u16 {
        println("WARNING: Low battery ({} mV)", frame.battery_mv);
    } else {
        println("Normal operation");
    }
}
```

看看 `handle_frame_initial` 的三大问题背景：
1. **判别与取值割裂**：`frame.is_overheated()` 检查条件时已经算出了摄氏度，但返回的是 `bool`；进了分支体还得再调一次 `frame.celsius()`。
2. **无法与外层模式组合**：如果 `SensorFrame` 嵌在一个可选类型 `?SensorFrame` 或结果类型 `!DecodeErr SensorFrame` 里面，普通方法根本没法直接塞进 `( .overheated(temp) )!` 这样的嵌套模式里一气呵成地拆包。
3. **如果改用返回 `?Tuple<i16, u16>` 的普通函数呢？** 哪怕外部调用者只关心温度 `temp`、写成 `(temp, _)` 不关心电池电压 `battery_mv`，普通函数也必须在返回前**把元组里的每一个元素都完整读取并构造出来**。

能不能让类型自己定义一种“按需提取、长得像变体模式一样”的自定义视图？

---

## 二、 引入 `case` 自定义匹配视图——像匹配 `enum` 一样匹配封装对象

在 Flurry 中，你可以在 `struct` 或 `impl` 块内部，使用关键字 **`case`**（而不是 `fn`）来声明一个**自定义匹配视图**。

它的语法与核心协议非常直观（详见 `docs/control-flow/pattern-matching/index.md`）：

```text
  定义侧 (在 struct / impl 内部):
  pub case view_name(*self) -> ?Tuple<T1, T2, ...> {
      if 满足匹配条件 {
          (payload_1, payload_2, ...)   -- 成功：自动注入 Option，交付给匹配臂绑定。
      } else {
          null                          -- 失败：返回 null，通知 match 尝试下一个分支。
      }
  }

  调用侧 (在 match / if is / let else 中):
  obj match {
      .view_name(a, b, ...) => { ... }, -- 像匹配枚举变体一样用 `.view_name(...)`。
      _ => { ... },
  }
```

下面来看看当 `frame match { .overheated(temp, mv) => ... }` 执行时，底层发生了怎样的对话：

```text
  调用方模式匹配:                      SensorFrame 内部的 `case overheated` 视图:
  frame match {                       ┌────────────────────────────────────────────┐
      .overheated(temp, mv) => ... ──►│ 1. 接收 *self                              │
                                      │ 2. 计算 c = (raw_temp / 2) - 40            │
                                      │ 3. 检查 c >= 85 ?                          │
                                      │    ├─ (否) ──► 返回 null                   │
      (若为 null，跳到下一分支!) ◄──────┼────┘                                       │
                                      │    └─ (是) ──► 交付 (c, self.battery_mv)   │
      temp ◄── c, mv ◄── battery_mv ◄─┼────┘                                       │
      执行当前分支体。                  └────────────────────────────────────────────┘
```

现在，让我们用 `case` 视图把 `SensorFrame` 重构为 改进版本：

```flurry
use std.prelude.*;

struct SensorFrame {
    raw_temp_half_c: u16,
    battery_mv: u16,

    -- 自定义视图 1：高温告警（提取换算后的摄氏度 + 当前电池电压）
    pub case overheated(*self) -> ?Tuple<i16, u16> {
        let c = (self.raw_temp_half_c / 2u16).as(i16) - 40i16;
        if c >= 85i16 {
            (c, self.battery_mv)
        } else {
            null
        }
    }

    -- 自定义视图 2：低电量告警（单值载荷或单元素 Tuple）
    pub case low_battery(*self) -> ?Tuple<u16> {
        if self.battery_mv < 3300u16 {
            (self.battery_mv,)
        } else {
            null
        }
    }

    -- 自定义视图 3：正常读数（无条件提取换算后的摄氏度）
    pub case normal_temp(*self) -> ?Tuple<i16> {
        let c = (self.raw_temp_half_c / 2u16).as(i16) - 40i16;
        (c,)
    }
}

fn handle_frame(frame: SensorFrame) {
    -- 看。外部代码完全不知道 raw_temp_half_c 是怎么编码的，
    -- 却能像匹配原生 enum 一样优雅地拆出摄氏度。
    frame match {
        .overheated(temp, mv) => {
            println("CRITICAL: Overheated at {} C (bat={} mV)!", temp, mv);
        },
        .low_battery(mv) => {
            println("WARNING: Low battery ({} mV)", mv);
        },
        .normal_temp(temp) => {
            println("Normal temp: {} C", temp);
        },
        _ => (),
    }
}

test {
    -- 260 / 2 - 40 = 90°C (高温告警!)
    let hot_frame = SensorFrame { raw_temp_half_c: 260u16, battery_mv: 3600u16 }
    -- 130 / 2 - 40 = 25°C, 但电池只有 3100mV (低电量告警!)
    let low_bat_frame = SensorFrame { raw_temp_half_c: 130u16, battery_mv: 3100u16 }

    handle_frame(hot_frame);
    handle_frame(low_bat_frame);
}
```

> **🔍 语法小贴士：单元素元组 `(x,)` 与普通括号分组 `(x)`**
> 注意看上面 `low_battery` 和 `normal_temp` 的返回类型是 `?Tuple<u16>`，而函数体里写的是 **`(self.battery_mv,)`**（末尾有一个逗号 `,`）。
> 还记得我们在 [第 010 篇](./010-pattern-precedence-grouping-and-ranges.md) 学过的括号法则吗？
> - `(expr)` 只是普通的优先级分组括号，类型依然是 `u16`；
> - **`(expr,)`** 才是**单元素元组（1-Tuple）**。
> 在匹配侧，调用者写 `.low_battery(mv)` 时，括号是模式应用语法 `pat(pat*)`，直接把元组里的第 1 个载荷元素绑定给 `mv`，既清爽又自然。

---

## 三、 深入 `case` 的两大关键特性——按需惰性读取与全场景模式组合

你可能会问：“`pub case overheated(*self) -> ?Tuple<i16, u16>` 看起来和写一个返回 `?Tuple<i16, u16>` 的普通方法很像啊，为什么语言要专门提供 `case` 机制？”

答案藏在两个普通函数根本做不到的关键特性里。

### 关键特性 1：未在模式中用到的载荷字段，编译器不必强制读取

翻开 `docs/control-flow/pattern-matching/index.md` 的正式定义，你会读到这样一句关键说明：
> **“未在模式中用到的字段，实现不必强制读取——这是用 `case` 做视图、而不是普通函数强行返回完整元组的原因之一。”**

什么意思？让我们画张图对比一下：

```text
  假设 `case overheated(*self) -> ?Tuple<i16, u16>` 导出了两个位置：(temp, battery_mv)
  而调用者在匹配时只关心温度，把第二个位置写成了通配符 `_`：

      if frame is .overheated(temp, _) do { ... }

  ┌─────────────────────────────────────────────────────────────────────┐
  │ 如果是个普通函数 fn get_overheated() -> ?Tuple<i16, u16>:            │
  │   必须在函数返回前无条件读取 self.battery_mv，构造完整的二元组返回，   │
  │   然后调用方再把第二个元素丢弃                                        │
  ├─────────────────────────────────────────────────────────────────────┤
  │ 因为它是 `case` 视图:                                                │
  │   编译器在展开匹配视图时看到调用方写了 `_`，根本不需要绑定第二个元素， │
  │   因此未在模式中使用的 `self.battery_mv` 字段甚至都不必强制读取        │
  └─────────────────────────────────────────────────────────────────────┘
```

当你的结构体里包含昂贵的计算字段、跨缓存行的冷数据字段，或者需要特殊访问权限的字段时，`case` 视图的这一特性让你既能定义一个包含丰富信息的完整视图，又不用担心调用方只取其中一两个字段时白白付出读取全部字段的代价。

### 关键特性 2：与 `?`、`!`、`let ... else`、`and is` 无缝嵌套组合

因为 `.overheated(...)` 是一个正儿八经的**一等公民模式（Pattern）**，它可以无缝嵌入到 Flurry 的任何模式上下文里。

来看看 进阶版本中，我们是如何把 `!DecodeErr`、`?SensorFrame`、`let ... else` 和 `.overheated` 视图组合在一行模式里的：

```flurry
use std.prelude.*;

enum DecodeErr {
    Corrupted,
}

fn fetch_next_frame(raw: u16) -> !DecodeErr ?SensorFrame {
    return error DecodeErr.Corrupted while raw == 0xFFFFu16;
    return null while raw == 0u16;
    SensorFrame { raw_temp_half_c: raw, battery_mv: 3500u16 }
}

fn monitor_step(raw: u16) {
    -- 一行模式同时完成：
    -- 1. 外层 `!`：匹配 !DecodeErr 的成功分支
    -- 2. 中层 `?`：匹配 ?SensorFrame 的非空分支
    -- 3. 内层 `.overheated(temp, _)`：调用自定义 case 视图提取摄氏度，忽略电压。
    if fetch_next_frame(raw) is (.overheated(temp, _))?! do {
        println("Immediate shutdown! Temp reached {} C", temp);
    }
}

-- 在 let ... else 中使用自定义 case 视图做前置守卫解构
fn require_normal_celsius(frame: SensorFrame) -> i16 {
    let .normal_temp(c) = frame else {
        -999i16
    }
    c
}

test {
    monitor_step(260u16);
    println("normal: {}", require_normal_celsius(
        SensorFrame { raw_temp_half_c: 130u16, battery_mv: 3500u16 }
    ));
}
```

看看第 16 行那颗璀璨的明珠：
```flurry
if fetch_next_frame(raw) is (.overheated(temp, _))?! do { ... }
```
如果不用 `case` 视图而用普通 Getter 方法，你得先拆 `!`、再拆 `?`、再绑定临时变量 `f`、再调 `if f.is_overheated()`、再调 `f.celsius()`——足足五层缩进。而有了 `case` 视图，五步操作浓缩成了短短一个声明式模式 `(.overheated(temp, _))?!`，而且 `SensorFrame` 的内部字段依然保持 100% 封装隐藏。

### 编译期拒绝例：违反 `case` 签名契约或穷尽性要求

当然，编译器对 `case` 视图的检查同样毫不留情：

```flurry
struct BadSensor {
    val: u16,

    -- ❌ 编译期直接拒绝：case 视图必须显式声明返回类型（如 -> ?Tuple<u16>），
    -- 且载荷形状必须与匹配臂的参数个数严格对应。
    pub case broken(*self) -> ?Tuple<u16> {
        -- 错误：签名要求返回 ?Tuple<u16>，这里却返回了二元组 (u16, u16)。
        (self.val, self.val)
    }
}

fn bad_exhaustive(frame: SensorFrame) -> i16 {
    -- ❌ 编译期直接拒绝：多臂 match 必须穷尽。
    -- 自定义 case 视图（如 .overheated）本质上是可反驳模式（可能返回 null），
    -- 编译器不会盲目相信一个自定义视图能覆盖所有 SensorFrame，因此缺少 `_` 兜底分支会报错。
    frame match {
        .overheated(temp, _) => temp,
    }
}
```

---

## 本篇小结

通过引入 `case` 自定义匹配视图，我们打通了抽象封装与模式匹配之间的最后一公里：

| 方案 | 封装性（隐藏物理布局） | 模式组合能力（嵌套 `?`/`!`） | 字段按需读取 | 适用场景 |
| :--- | :--- | :--- | :--- | :--- |
| **直接暴露 `struct` 字段 / `enum`** | ❌ 外部直接耦合内部布局 | ✅ 原生支持 | ✅ 原生支持 | 纯数据传输对象（DTO）或定宽协议报文 |
| **私有字段 + 普通 Getter 方法** | ✅ 隐藏内部布局 | ❌ 只能退回 `if-else` | ❌ 返回元组时必须全部求值 | 简单的命令式属性查询 |
| **私有字段 + `pub case` 视图** | ✅ **完美隐藏内部物理表示** | ✅ **像原生变体一样任意嵌套匹配** | ✅ **模式中写 `_` 的字段不必强制读取** | 需要演进内部表示、同时向外提供声明式解构的领域模型 |

---

### 下一篇预告（第一卷收官之战）

不知不觉，从 [第 001 篇](./001-a-world-in-a-byte.md) 那个皱巴巴的 8 比特传感器手册出发，我们已经走过了 19 篇跌宕起伏的设计演进之旅。
我们手里已经攒下了整整一套系统编程的硬核心机制库：
- **底层比特与布局**：定宽模式 `enum`、跨元素 `BitVec` 模式、`BitVec` 代数拼接 `++` 与原位回写、`union` C ABI 共享存储；
- **类型与抽象边界**：`?T` 可选类型、`!Err T` 错误行、`newtype` 独立身份与 `.as(View)` 视角、`struct` 默认字段與 `case` 自定义匹配视图；
- **控制流与资源安全**：后缀 `match`、`while ... is ... do`、`let ... else`、带 `while` 守卫的跳转、前后置 `catch` 容错重试、`let`/`var` 绑定契约与失败安全的 `Drop` 热切换。

是时候迎来我们 **第一卷（001–020）的里程碑大考** 了。
在下一篇 **《Flurry 演进手记 (020)：第一卷里程碑——组装一台完整的 8 位传感器协议分析仪》** 中，我们将把前 19 篇学到的所有语言积木组装成一个完整、严密、可直接运行的微型固件模块，并用一张全景架构图为你点亮通往第二卷（函数的十八般武艺）的大门。
