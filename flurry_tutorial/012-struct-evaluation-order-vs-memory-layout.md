# Flurry 演进手记 (012)：字段顺序有别于内存顺序——`struct` 声明序求值、默认字段与物理布局解耦

在 [上一篇 (011)](./011-when-nan-crashes-float-matching.md) 的结尾，我们留下了一个关于结构体 `struct` 初始化的灵魂三问。

今天我们把场景搬到传感器监控台的显示模块：我们需要初始化一个采样视窗结构体 `Window`，它有两个字段：`width`（宽度）和 `height`（高度）。
在硬件驱动层，获取这两个参数的途径往往带有**可观察的副作用（Side Effects）**：
- 有时候，调用者会调用 `supplied_width()` 和 `supplied_height()` 从硬件 FIFO 队列或寄存器里依次弹出配置字，并打印硬件审计日志；
- 有时候，调用者只关心宽度 `width`，希望高度 `height` 自动调用 `default_height()` 获取出厂默认高度（比如 `480`）。

假设你在代码库里看到一位同事写下了这样一段构造代码：

```flurry
let second = Window {
    height: supplied_height(),
    width: supplied_width(),
}
```

请注意：在 `struct Window` 的定义里，`width` 写在前面，`height` 写在后面；但在构造 `second` 的花括号里，同事**先写了 `height`，后写了 `width`**。

这时候，到底谁先执行？`default_height()` 会被触发吗？结构体在内存里的物理字节顺序又是由谁决定的？
让我们从其他语言的历史包袱出发，一步步看清 Flurry 是如何把“名字对应”、“时间顺序”和“空间布局”这三件事解耦的。

---

## 一、 C / C++ / Rust 的历史纠结與隐蔽陷阱

在系统编程语言的演进史上，`struct` 的初始化一直是个充满妥协的战场：

1. **C 语言的自动补零陷阱**：在 C 语言里，如果你写 `struct Window w = { .width = 640 };`，你漏写的 `height` 会被悄悄填成 `0`。对于一个视窗或传感器配置来说，高度为 `0` 往往意味着除零崩溃或非法硬件状态，但编译器却完全不会提醒你漏填了字段。
2. **C++ 指定初始化器（Designated Initializers）的顺序枷锁**：在 C++20 中，因为字段的构造/析构顺序与物理内存布局绑死在声明顺序上，如果你敢像上面那样颠倒顺序写 `Window { .height = ..., .width = ... }`，C++ 编译器会直接报错。
3. **调用处顺序 vs 声明处顺序的撕裂**：如果一门语言允许你按任意顺序写字段，却按照“你在调用处写的顺序”去求值，那么当某些字段使用**声明处的默认表达式**（比如 `height: u32 = default_height()`）时，默认表达式到底插在什么时候执行？阅读代码的人必须在每个构造点重新心算一套求值次序。

如果在 Flurry 里我们也想当然地以为“花括号里谁写在前面就先算谁”，或者以为“省略的字段会自动填 0”，就会产生误解。

---

## 二、 构造完整性与“声明序求值”核心规则

为了终结这些混乱，Flurry 为 `struct` 的构造确立了两条核心规则（详见 `docs/type/user-defined-types/structs.md`）：

### 规则一：100% 显式指名与构造完整性

在 Flurry 中，构造 `struct` **必须完整写出字段名**（`field: expr`）：
- **暂未涵盖位置构造**（禁写 `Window { 640, 480 }`）；
- **暂未涵盖构造侧的同名变量简写**（模式解构里可以写 `let { width, height } = win;`，但构造值时必须老老实实写 `Window { width: width, height: height }`）；
- **非自动补零**：每个字段必须有且仅有一个初始化来源——**显式输入优先，否则使用声明默认表达式；两者都没有则直接报错。**

来看看编译器是如何静态拦截三类残缺或错误的构造的：

```flurry
struct Window {
    width: u32,
    height: u32 = 480u32,
}

test {
    -- ❌ 编译期直接报错 1：缺少必填字段 `width`（不会悄悄帮你把 width 填成 0。）
    let missing = Window {}

    -- ❌ 编译期直接报错 2：重复指定同一个字段 `width`
    let duplicate = Window { width: 1u32, width: 2u32 }

    -- ❌ 编译期直接报错 3：提供了结构体中全然无存的未知字段 `depth`
    let unknown = Window { width: 1u32, depth: 2u32 }
}
```

### 规则二：名字决定“去哪儿”，声明顺序决定“谁先算”

那么，当所有字段都合法时，求值顺序到底听谁的？
Flurry 的规则简洁得令人过目不忘：

> **花括号里的字段名称，只决定输入表达式对应到哪个字段；真正的初始化求值，永远严格按照 `struct` 定义中的字段声明顺序依次执行。**

下面来看看编译器内部的构造函数规划器（`plan_constructor`）在遇到乱序书写和默认字段时，到底做了什么：

```text
  struct Window 声明顺序：
  [第 1 槽位] width:  u32
  [第 2 槽位] height: u32 = default_height()

  ─────────────────────────────────────────────────────────────────────────
  场景 A：乱序显式构造
  源码写的是：Window { height: supplied_height(), width: supplied_width() }
                                      │                      │
         ┌────────────────────────────┼──────────────────────┘
         ▼ (按名字归位到第 1 槽位)     ▼ (按名字归位到第 2 槽位，覆盖默认值!)
  [第 1 步执行] supplied_width()  ──► [第 2 步执行] supplied_height()
  (⚠️ 注意：`default_height()` 被显式输入覆盖，跳过执行。)

  ─────────────────────────────────────────────────────────────────────────
  场景 B：省略带默认值的字段
  源码写的是：Window { width: supplied_width() }
                              │
         ┌────────────────────┴───────────────┐
         ▼ (第 1 槽位：显式输入)              ▼ (第 2 槽位：启用声明默认表达式)
  [第 1 步执行] supplied_width()  ──► [第 2 步执行] default_height()
```

让我们把官方规范中的经典可运行示例搬上来，亲眼见证打印输出的顺序：

```flurry
use std.prelude.*;

fn supplied_width() -> u32 {
    println("width");
    640
}

fn default_height() -> u32 {
    println("default height");
    480
}

fn supplied_height() -> u32 {
    println("explicit height");
    720
}

struct Window {
    width: u32,
    height: u32 = default_height(),

    fn area(self) -> u32 {
        self.width * self.height
    }
}

test {
    -- 第 1 次构造：只传 width，height 走默认表达式
    let first = Window { width: supplied_width() }

    -- 第 2 次构造：故意把 height 写在前面、width 写在后面。
    let second = Window {
        height: supplied_height(),
        width: supplied_width(),
    }

    -- 第 3 次构造：再次省略 height
    let third = Window { width: 320 }

    println("areas: {}, {}, {}", first.area(), second.area(), third.area());
}
```

运行这段代码，控制台打印出的结果是：

```text
width
default height
width
explicit height
default height
areas: 307200, 460800, 153600
```

仔细核对这 6 行输出，你会发现三个尤为重要的保证：
1. **声明序不可动摇**：在构造 `second` 时，尽管源码里 `height: supplied_height()` 写在第一行，控制台却**先打印了 `"width"`，后打印了 `"explicit height"`**。因为 `Window` 声明时 `width` 在 `height` 前面。
2. **被覆盖的默认表达式不陪跑**：在构造 `second` 时，`default height` 根本没有打印。因为一旦某个字段提供了显式输入，它的默认表达式就被跳过，免于在后台偷偷产生副作用。
3. **默认表达式每次构造独立求值**：在构造 `first` 和 `third` 时，`default height` 各自打印了一次。这证明 `height: u32 = default_height()` **并非**在定义类型时只算一次然后共享静态结果，而是在每次实际需要它的构造过程中独立求值一次。

---

## 三、 时间与空间的解耦——为什么固定求值顺序反而解放了内存布局？

读到这里，系统程序员脑子里的警报器可能会响起来：
> “等等。在 C++ 里，初始化顺序之所以固定，是因为初始化顺序被绑死在了物理内存布局上。Flurry 把初始化求值顺序固定为声明顺序，难道意味着 `struct` 的物理内存顺序也被锁死了吗？”

**恰恰相反。这正是 Flurry 最精妙的设计洞察（详见 `docs/type/user-defined-types/structs.md`）：**

在传统语言里，如果求值顺序没有独立出来，或者人们把“声明顺序”、“求值顺序”和“内存偏移顺序”混为一谈，编译器一旦想重排内存字段来消除对齐空洞（Padding），就会担心改变程序的观察行为。

而 Flurry 做了一次干净利落的**时空解耦**：

```text
  ══════════════════ Flurry struct 的“时空解耦”架构 ══════════════════

  [源级契约层]
  字段名字 (Name)          ──► 决定构造输入对应到哪个逻辑字段
  声明顺序 (Decl Order)    ──► 决定【时间维度】的初始化求值先后顺序 (100% 可预测!)
                                    │
                        (两条线完全独立，互不绑架。)
                                    ▼
  [底层表示层]
  物理布局 (Memory Layout) ──► 决定【空间维度】的字节偏移与对齐。
                               编译器可自由重排字段顺序，充分利用缓存与对齐。
```

让我们看一个具体的内存对齐例子。假设我们定义了一个传感器节点结构体 `SensorNode`：

```flurry
struct SensorNode {
    id: u8,         -- 1 字节
    reading: u64,   -- 8 字节（通常要求 8 字节对齐）
    flags: u8,      -- 1 字节
}
```

1. **在时间维度（求值顺序）上**：
   无论你在构造 `SensorNode { flags: f(), reading: r(), id: i() }` 时怎么调换花括号里的顺序，编译器生成的初始化计划永远严格执行：
   `第 1 步算 id` $\rightarrow$ `第 2 步算 reading` $\rightarrow$ `第 3 步算 flags`。你的硬件寄存器读取日志永免乱序。
2. **在空间维度（物理布局）上**：
   因为普通 `struct` **不由初始化顺序承诺物理字段顺序**，编译器在安排内存布局时，完全可以把 8 字节的 `reading` 放在最前面，后面紧挨着放 1 字节的 `id` 和 1 字节的 `flags`：

```text
  如果按声明顺序硬排物理内存 (24 bytes):
  ┌──────┬─────────────────┬──────────────────┬──────┬─────────────────┐
  │id(1B)│  Padding (7B)   │   reading (8B)   │fl(1B)│  Padding (7B)   │
  └──────┴─────────────────┴──────────────────┴──────┴─────────────────┘

  Flurry 解耦后，编译器自由优化的物理布局 (16 bytes):
  ┌──────────────────┬──────┬──────┬─────────────────┐
  │   reading (8B)   │id(1B)│fl(1B)│  Padding (6B)   │ (依然按 id -> reading -> flags 求值。)
  └──────────────────┴──────┴──────┴─────────────────┘
```

更进一步，别忘了 Flurry 的**名义类型（Nominal Typing）**原则：
哪怕你声明了两个字段一模一样的 `struct`：

```flurry
struct CelsiusReading { value: f64 }
struct FahrenheitReading { value: f64 }

-- ❌ 编译期直接报错：独立的 struct 声明具有完全独立的类型身份。
fn bad_mix(c: CelsiusReading) -> FahrenheitReading {
    c
}
```

即便它们的字段名和类型完全相同，它们在类型系统里也是各司其职而互无干犯的两个独立类型，免令你把摄氏度当成华氏度传进控制系统。

---

## 本篇小结

围绕着 `struct` 的定义与构造，我们理清了 Flurry 清晰明确的三层契约：

| 维度 | 决定因素 | 核心规则与工程收益 |
| :--- | :--- | :--- |
| **匹配哪一项** | 字段名称 (`id: expr`) | 必须 100% 显式指名；暂未涵盖位置构造、字段简写、未知字段、重复字段与无默认值的缺失字段 |
| **何时去求值** | 声明顺序 (Declaration Order) | 严格按 `struct` 定义的字段顺序求值；显式输入覆盖默认值时跳过执行默认表达式；默认表达式在每次需要的构造中独立求值一次 |
| **存在哪块内存** | 物理布局 (Physical Layout) | 与初始化顺序解耦。编译器可独立优化对齐与缓存利用，同时无碍于副作用的可预测时机 |

---

### 下一篇预告

用 `struct` 把同构配置打包在一起非常顺手。但是，传感器网关每天要处理几十种不同种类的系统事件：
- 有的是网络连接事件（如连接建立、收到字节、甚至深层嵌套的传输停滞）；
- 有的是绘图或控制指令（如退出、重复 $N$ 次、移动到坐标 `(x, y)`）；
- 还有的是需要发往外部总线、底层以轻量级符号 `Symbol`（如 `.wire_ready`）编码的协议状态。

当一个 `enum Event` 膨胀到三四十个变体时，如果你只想写一个判断函数 `is_network(event: Event) -> bool`，难道要在 `match` 里把二十几个网络相关的变体一行行全抄一遍吗？
还有，当我们想在日志里打印出枚举变体原本的声明路径名（比如 `"network.ready"`），却**不想移动（Move）枚举肚子里装着的仿射载荷**时，该用什么武器？

在下一篇 **《Flurry 演进手记 (013)：从高层消息到底层枚举——`enum` 分组变体 `.network.{...}` 与 `'tag_name` 视图》** 中，我们将解锁 Flurry 枚举系统的层次化分组树与非破坏性标签视图。
