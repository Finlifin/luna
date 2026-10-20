# Flurry 演进手记 (003)：`let` 并没有冻结你的对象——绑定、字段指针与失败安全的资源更替

在 [前两篇 (001 & 002)](./002-when-sensors-meet-noise.md) 中，我们已经能解析单个传感器字节，并用 `!DecodeErr` 和 `!SensorErr` 优雅地处理噪声与过热。

现在，系统要上线运行了。我们需要构建一个常驻内存的 **传感器监控器（`SensorMonitor`）**，它的职责是：
1. 记录当前连接的传感器通道编号 `channel`、累计收到的有效读数总和 `total_sum` 以及采集次数 `sample_count`。
2. 允许外部模块只拿着指向某个计数字段的指针（比如只给统计模块传入 `sample_count` 的指针）来更新计数。
3. 当主通道传感器连续故障时，支持**热切换（Hot-Swap）**到备用通道：创建新的监控器实例替换掉旧实例，并在旧实例销毁时自动释放底层硬件通道（`Drop`）。

听起来像是一段再普通不过的面向对象/结构体代码？
别急。当我们动手写下第一行 `struct` 和 `let` 时，如果你脑子里装的是 Rust 的 `let` / `let mut` 或者 C++ 的 `const` / 赋值操作符，Flurry 马上就会给你带来三次认知冲击。

---

## 一、 定义 `struct` 与第一个认知冲击——为什么 `let` 能改字段？

我们先写出监控器的结构体定义。

在 Flurry 中，定义 `struct` 有两条非常明确的规矩（详见 `docs/type/user-defined-types/structs.md`）：
1. **字段支持声明默认值**：比如刚开始采集时，`total_sum` 和 `sample_count` 默认都是 `0u32`，我们可以直接在定义字段时写 `= 0u32`。
2. **构造时必须完整指名**：没有位置构造，也没有同名变量简写（比如不能写 `SensorMonitor { channel }`，必须老老实实写 `SensorMonitor { channel: channel }`）。但有默认值的字段在构造时可以直接省略。

来看看我们的第一版代码：

```flurry
use std.prelude.*;

struct SensorMonitor {
    channel: u8,
    total_sum: u32 = 0u32,
    sample_count: u32 = 0u32,

    -- 在 struct 内部可以直接定义关联方法。
    fn record_sample(*self, reading: u8) {
        self.total_sum.* += reading.as(u32);
        self.sample_count.* += 1u32;
    }
}

test {
    -- 只传必填字段 channel，其余两个字段自动使用默认值 0u32
    let monitor = SensorMonitor { channel: 1u8 }

    -- 等等。monitor 是用 `let` 声明的，下面这两行居然合法？
    monitor.total_sum = 10u32;
    monitor.record_sample(5u8);

    println("sum: {}, count: {}", monitor.total_sum, monitor.sample_count);
}
```

> **🔍 语法小贴士：还记得 `}` 省略分号的规则吗？**
> 看 `let monitor = SensorMonitor { channel: 1u8 }` 这一行。因为结构体构造表达式以右花括号 `}` 结尾，所以整条 `let` 语句的最后一个 token 就是 `}`，后面的分号自动省略。

现在，盯着第 20 行和第 21 行看三秒钟：

```flurry
let monitor = SensorMonitor { channel: 1u8 }
monitor.total_sum = 10u32;   -- ✅ 完全合法。
monitor.record_sample(5u8);  -- ✅ 完全合法。
```

如果你来自 Rust，你现在一定很困惑：**“`monitor` 明明是用 `let` 而不是 `var` 声明的。为什么我可以修改 `monitor.total_sum`，还能把可变指针传给 `*self`？”**

### 核心心智模型：`let` 管的是“名牌”，不是“箱子”

在 Rust 中，`let x = ...` 不仅锁死了绑定名 `x`，还顺带把 `x` 整棵对象树的所有字段都冻成了只读（Deep Immutability）。
但在系统编程和形式化验证中，把“名称能不能重绑定到新对象”和“对象的哪个字段允许被谁写入”绑死在一起，其实是一种过度耦合——很多时候你只是希望“这个变量名永远代表这台监控器，别在半路被手滑赋成另一台监控器”，但监控器内部的计数器当然需要正常递增。

因此，Flurry 做出了一个正交的设计（详见 `docs/resource-management/bindings.md`）：

> **`let` 只限制“不能直接用这个绑定名称作为整值赋值的目标”；它绝不冻结对象内部的内容。**

如下所示，对比 `let` 与 `var` 的真正区别：

```text
  let monitor = SensorMonitor { channel: 1, ... }

     绑定名称 (Name)                对象存储 (Object Storage)
    ┌─────────────┐               ┌─────────────────────────┐
    │   monitor   │ ════(固定)═══►│ channel:      1         │
    └─────────────┘               ├─────────────────────────┤
                                  │ total_sum:    0 ──► 10  │ (✅ 字段写入由对象契约管，
                                  ├─────────────────────────┤  不受 let 限制。)
                                  │ sample_count: 0 ──► 1   │
                                  └─────────────────────────┘

  ❌ monitor = SensorMonitor { channel: 2 }
     (非法。let 禁止直接拿 `monitor` 这个名字作整值赋值目标。)

  ✅ 如果想直接写 `monitor = ...` 整值替换，才需要把声明改成 `var monitor = ...`。
```

甚至，如果你用 `let ptr = monitor.ref;` 拿到了指向 `monitor` 的指针，再通过指针解引用做整值替换 `ptr.* = SensorMonitor { channel: 2 }`，在 `let` 下也是合法的。因为你写的是 `ptr.* = ...`，而不是直接拿绑定名 `monitor` 作赋值左值。

那么你可能会问：“如果我真的想规定某个函数绝对不能修改 `monitor` 的字段，该怎么办？”
答案是：在 Flurry 中，**读写权限属于内存效果（Memory Effects，`read` / `write`）与函数契约**，而不是靠局部变量名上的 `let` 去猜。各司其职，边界分明。

---

## 指针 `.ref`、解引用 `.*` 与天然的字段指针

接下来看第二个需求：我们要写一个独立的辅助函数 `bump_counter`，它不关心整个 `SensorMonitor`，只接收一个指向 `u32` 计数器的指针 `*u32`，每次帮它加 1。

在 Flurry 中，指针操作全部采用**后缀语法**：
- 取地址写 **`expr.ref`**（而不是前缀 `&expr`）；
- 解引用写 **`expr.*`**（而不是前缀 `*expr`）。

因为全部采用后缀语法，你可以从左到右按数据流顺序书写：`obj.ref.field.*`。

但更妙的还在后面。假设手里有一个指向结构体的指针 `mon_ptr: *SensorMonitor`，当你写 **`mon_ptr.sample_count`** 时，会发生什么？

来看看这段代码：

```flurry
fn bump_counter(counter: *u32) {
    -- 先假设（并让编译器检查）不会发生 u32 算术溢出
    assumes counter.* < 1000000u32;
    counter.* += 1u32;
}

test {
    let monitor = SensorMonitor { channel: 1u8 }
    let mon_ptr: *SensorMonitor = monitor.ref;

    -- 注意看这里。mon_ptr 是 *SensorMonitor，
    -- 直接点出 .sample_count，得到的直接就是字段指针 *u32。
    let count_ptr: *u32 = mon_ptr.sample_count;
    bump_counter(count_ptr);

    -- 而在明确要求 u32 值的位置，它又能自动完成 Copy 读取。
    let snapshot: u32 = mon_ptr.sample_count;
    println("count is now: {}", snapshot);
}
```

让我们画张图，看看 **`mon_ptr.sample_count`** 的投影规则（详见 `docs/resource-management/pointers.md` 与 `docs/type/receiver-adaptation.md`）：

```text
  mon_ptr (*SensorMonitor)
     │
     │  直接做字段投影：`mon_ptr.sample_count`
     ▼
  得到字段指针 (*u32)  ◄─── (相当于先解引用、取字段、再取址，一步到位。)
     │
     ├──► 场景 A：赋给 `let count_ptr: *u32` 或传给 `bump_counter(*u32)`
     │            ──► 直接交付该字段指针 `*u32`。
     │
     └──► 场景 B：赋给明确要求值类型的 `let snapshot: u32`
                  ──► 触发“目标引导读取 (Target-Guided Read)”，自动插入一次合法 Copy 读取。
```

这就是为什么刚才我们在写 `SensorMonitor.record_sample(*self, ...)` 时，因为 `self` 的类型是 `*SensorMonitor`：
1. `self.total_sum` 通过指针投影，直接定位到了字段指针 `*u32`；
2. 再加上后缀 `.*`（`self.total_sum.* += ...`），就能直接解引用该字段指针并就地更新字段槽位。

### 顺便聊聊：两个指针指向同一个计数器会报错吗？

如果你写惯了 Rust，看到下面的代码可能会条件反射地紧张起来：

```flurry
fn add_both(first: *u32, second: *u32) -> u32 {
    assumes first.* < 100u32 and second.* < 100u32;
    first.* = 11u32;
    second.*
}

test {
    var num = 3u32;
    -- 把同一个变量的指针同时传给两个参数。
    let res = add_both(num.ref, num.ref);
    println("{}", res); -- 输出 11
}
```

在 Rust 中，同时制造两个可变引用 `&mut num` 是绝对违法的（Aliasing XOR Mutability）。
但在 Flurry 中，上面这段顺序代码**完全合法，输出 `11`**。

为什么？因为 Flurry 的 `*T` **不把“独占无别名（noalias）”强加给每一次普通指针构造**。在单线程顺序代码里，两个指针指向同一个对象再正常不过了：编译器在做流分析时，清楚地知道 `first.* = 11u32` 可能会改变 `second.*` 的值，因此不会做错误的死代码消除或寄存器缓存；只有当某个算法真正需要“两个指针绝对不重叠（分离性）”或并发安全时，才通过契约与可达性系统按需证明。这让系统程序员在写链表、图结构或回调状态更新时，彻底告别了“为了讨好借用检查器而把指针全改成数组下标”的痛苦。

---

## 三、 当热切换遇到故障——`var` 覆盖赋值与 `Drop` 的严格核心规则

现在来到最硬核的第 3 个需求：**通道热切换（Hot-Swap）与资源清理**。

假设每个 `SensorMonitor` 在创建时都打开了底层的硬件通道，因此在销毁时必须执行 `Drop` 来关闭通道。
由于我们需要把旧的 `monitor` 整值替换成新的 `SensorMonitor`，这次我们必须用 **`var`** 而不是 `let` 来声明变量了。

但是，切换新通道的函数 `open_monitor(channel: u8)` 是**可能失败的**（比如备用通道 2 硬件自检未通过，返回了 `!SwitchErr` 错误）。

思考一个关乎系统生死的问题：
当我们写下赋值语句：

```flurry
monitor = open_monitor(2u8)!;
```

**到底应该先销毁旧的 `monitor`，还是先求值右边的 `open_monitor(2u8)!`？**

如果先销毁了旧的 `monitor`（关闭了通道 1），结果右边的 `open_monitor(2u8)!` 报错提前退出了——完蛋。通道 2 没打开，通道 1 也被你关掉了，整个监控系统陷入死寂。

在 Flurry 中，覆盖赋值（Overwriting Assignment）的资源处理次序被严格写入了语言规范（详见 `docs/resource-management/bindings.md`）：

```text
  执行 `target = build_new()!` 的四步核心规则：

  [第 1 步] 先求值右侧表达式 `build_new()!`
      │
      ├──► 若中途失败 (返回 error)：
      │    立即走错误传播路径退出。
      │    此时 target 根本没被触碰，旧值 100% 完好保留。
      │
      ▼ (右侧成功，拿到新值临时责任)
  [第 2 步] 检查已被定位的 target 依然合法可写
      │
      ▼
  [第 3 步] 清理 (Drop) target 持有的旧值。
      │
      ▼
  [第 4 步] 将新值写入 target，完成责任转交。
```

不仅如此，还有第二个极其关键的问题：
假设我们在作用域里依次声明了 `var first`（通道 1）和 `let second`（通道 2）。按照作用域逆序清理规则，退出作用域时本来应该**先 Drop `second`，再 Drop `first`**。
如果我们在中途给 `first` 重新赋了一个全新的值 `first = open_monitor(3u8)!`，那么退出作用域时，这个最晚构造出来的通道 3，会排在 `second` 前面销毁，还是留在 `first` 原来的位置最后销毁？

让我们直接写代码验证这两条核心规则：

```flurry
use std.prelude.*;

struct SensorMonitor {
    channel: u8,
}

impl Drop for SensorMonitor {
    fn drop(*self) {
        println("Closing channel {}", self.channel.*);
    }
}

enum SwitchErr {
    HardwareBusy,
}

fn open_monitor(channel: u8, fail: bool) -> !SwitchErr SensorMonitor {
    println("Opening channel {}...", channel);
    return error SwitchErr.HardwareBusy while fail;
    SensorMonitor { channel: channel }
}

fn try_hot_swap(target: *SensorMonitor, new_ch: u8, fail: bool) -> !SwitchErr void {
    target.* = open_monitor(new_ch, fail)!;
}

test {
    var primary = SensorMonitor { channel: 1u8 }
    let backup  = SensorMonitor { channel: 2u8 }

    -- 第一次热切换：故意让通道 9 打开失败。
    try_hot_swap(primary.ref, 9u8, true)! {
        .HardwareBusy => {
            -- 看看 primary.channel 是多少？依然是 1，且通道 1 绝没有被提前关闭。
            println("Swap failed! Still running on channel {}", primary.channel);
        },
    }

    -- 第二次热切换：成功切换到通道 3。
    -- 此时旧的通道 1 会立刻被 Drop，primary 接管通道 3
    try_hot_swap(primary.ref, 3u8, false)! {
        .HardwareBusy => (),
    }

    println("All running smoothly.");
    -- 作用域即将结束。猜猜看 backup (通道 2) 和 primary (通道 3) 谁先被关闭？
}
```

来看看这段程序的精确输出顺序：

```text
Opening channel 9...
Swap failed! Still running on channel 1
Opening channel 3...
Closing channel 1
All running smoothly.
Closing channel 2
Closing channel 3
```

对照输出，你会发现 Flurry 的资源管理是多么滴水不漏：
1. **失败不伤旧值**：尝试打开通道 9 失败时，`try_hot_swap` 提前返回错误，`primary` 稳稳地保留着通道 1，没有任何提前析构。
2. **成功即时清理旧值**：打开通道 3 成功后，立刻触发 `Closing channel 1`，然后把通道 3 装进 `primary`。
3. **覆盖不重排退出清理顺序**：虽然通道 3 比 `backup`（通道 2）构造得更晚，但因为它装在最先声明的 `primary` 槽位里，退出作用域时依然严格按照**变量声明位置的逆序**——先关后声明的 `backup`（通道 2），最后才关先声明的 `primary`（通道 3）。这保证了无论控制流中间经历了多少次热切换，绑定之间的析构依赖顺序永远是静态固定、可预测的。

---

## 本篇小结

围绕着一个“可热切换的传感器监控器”，我们理清了 Flurry 内存与绑定模型的三大支柱：

| 特性 | 表面语法 | 核心语义契约 |
| :--- | :--- | :--- |
| **`struct` 构造** | `Type { field: val }` | 支持字段默认表达式（每次构造按声明顺序求值）；必须完整指名，拒绝缺漏或重复 |
| **`let` vs `var`** | `let x = ...` / `var x = ...` | `let` 只禁止以 `x` 为直接目标做整值赋值；字段修改与经指针修改由对象与内存契约管理 |
| **指针与字段投影** | `x.ref`、`p.*`、`p.field` | 后缀取址与解引用；对结构体指针做字段投影直接得到字段指针，有明确值目标时可自动读取 Copy 值；顺序代码允许合法别名 |
| **覆盖赋值与 `Drop`** | `x = new_val` | 先求值新值，再清理旧值，最后安装新值；右侧失败保留旧值；覆盖赋值不改变变量在作用域末尾的清理次序 |

### 下一篇预告

现在，我们的 `SensorMonitor` 已经能安全地记录状态、通过指针更新字段、并在故障时安全热切换了。
但是，业务方提出了一个新问题：
> “除了温度传感器，我们现在还有湿度传感器、气压传感器。有的监控函数只需要打印日志，有的需要把多个配置项打包传给初始化函数，有的还希望参数能根据前面的参数自动算出默认值（比如分页查询历史记录时，默认读取从 `start` 到末尾的所有剩余样本 `total - start`）……”

在 C++ 或 Python 里，默认参数和可变参数（`*args`, `**kwargs`）要么是运行时字典开销，要么连前一个参数的值都不能引用。
在下一篇 **《Flurry 演进手记 (004)：参数是怎么来到函数里的——依赖默认值、位置变参 `...items` 与属性收集》** 中，我们将为监控器设计一套既像脚本语言一样灵活、又保持 100% 静态类型与零开销的配置与查询接口。
