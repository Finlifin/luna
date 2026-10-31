# Flurry 演进手记 (013)：从高层消息到底层枚举——`enum` 分组变体 `.network.{...}` 与 `'tag_name` 视图

在 [第一篇 (001)](./001-a-world-in-a-byte.md) 中，我们见识过直接长在比特位上的定宽 `enum Packet`；在 [上一篇 (012)](./012-struct-evaluation-order-vs-memory-layout.md) 中，我们掌握了 `struct` 的声明序求值与布局解耦。

现在，我们要为传感器监控中心构建一套完整的**高层事件与消息分发总线**。随着业务膨胀，这套事件系统遇到了三个典型的核心问题：

1. **多形态载荷混合**：有的控制消息无需任何额外数据（如 `quit`），有的只需要一个位置参数（如 `repeat(3)`），有的则需要带名字的坐标字段（如 `move_to { x: 10, y: 20 }`）。
2. **变体爆炸与分类路由**：系统里的 `enum Event` 包含了网络事件（`connected`、`received(Byte)`、甚至深层子状态 `nested.stalled`）和停机事件（`stopped`）。在路由层，我们只想快速判断“这是否一个网络事件”，却不想因为写了嵌套 `enum NetworkEvent` 而到处包两层构造器。
3. **无损审计日志与底层 `Symbol` 编码**：监控器需要在不消费、不转移枚举载荷所有权的前提下，把当前变体的完整声明路径名（如 `"network.ready"`）打印到日志里；而且有些协议枚举在底层直接用轻量级 `Symbol`（如 `.wire_ready`）编码，我们该怎么区分它的“逻辑声明名”与“底层线缆名”？

让我们从基础的高层 `enum` 开始，一步步演化出 Flurry 独具匠心的**分组变体树（Grouped Variants）**与 **`'tag_name` 视图**。

---

## 一、 三种基础变体与目标驱动的 `.symbol` 简写

在 Flurry 中，`enum` 描述一个封闭的变体集合。不同于必须指定比特位宽的底层协议包，普通的业务 `enum` 只需要直接写出变体的逻辑结构（详见 `docs/type/user-defined-types/enums.md`）：

- **无载荷变体**：直接写名字，如 `quit`；
- **位置载荷变体（Tuple Variant）**：用圆括号 **`()`** 列出载荷类型，如 `repeat(u32)`；
- **命名字段变体（Struct Variant）**：用大括号 **`{}`** 列出具名字段（同样支持默认值。），如 `move_to { x: i32, y: i32 }`。

来看看我们的消息分发器 `dispatch`：

```flurry
use std.prelude.*;

enum Message {
    quit,
    repeat(u32),
    move_to { x: i32, y: i32 },
}

fn dispatch(message: Message) {
    message match {
        .quit => println("quit"),
        .repeat(count) => println("repeat {} times", count),
        .move_to { x: x, y: y } => println("move to ({}, {})", x, y),
    }
}

test {
    dispatch(Message.quit);
    -- 当调用目标的参数类型已经明确是 Message 时，可以直接写 `.quit`。
    dispatch(.quit);
    dispatch(.repeat(3));
    -- 命名字段变体同样遵循 struct 的核心规则：按名字对应，交换书写顺序不交换坐标。
    dispatch(Message.move_to { y: 20, x: 10 });
}
```

运行这段代码，输出为：

```text
quit
quit
repeat 3 times
move to (10, 20)
```

> **🔍 语法小贴士：`.quit` 到底是 `Symbol` 还是 `Message`？**
> 仔细看 `dispatch(.quit);` 这一行。为什么我们省去书写 `Message.quit`？
> 在 Flurry 中（详见 `docs/basics/literals.md`），以点号开头的 `.id` 在没有目标类型上下文时，默认是一个独立的轻量级编译期/运行时字面量类型 **`Symbol`**（比如 `let tag = .ready;` 的类型就是 `Symbol`）。
> 但是，当调用参数、返回类型或匹配主语**已经明确提供了某个 `enum` 目标上下文**（比如 `dispatch` 明确要求 `Message`）时，编译器会先取得该上下文，把 `.quit` 直接解析为 `Message.quit`。
> 注意：这非意味着脱离了上下文的任意 `.quit` 都自动具有 `Message` 类型——这种“有目标则推导为枚举变体，无目标则保持为 `Symbol`”的机制，让你既能享受简洁的 `.quit` 语法，又能在元编程和配置中自由使用真正的 `Symbol`。

---

## 二、 当变体增长到几十个——分组变体 `.network.{...}` 与整组匹配

现在看第二个需求：我们要定义系统的事件枚举 `Event`。
在传统语言（如 Rust）中，如果你有很多网络相关的事件，你通常只有两条路可走：
1. **扁平前缀法**：写 `NetworkConnected`、`NetworkReceived(u8)`、`NetworkNestedStalled`。但当你想写一个判断函数 `is_network(e: Event)` 时，你必须在 `match` 里把所有以 `Network` 开头的变体全都手抄一遍，一旦以后新增一个网络变体，漏改 `match` 就会出 bug。
2. **拆分子枚举法**：单独定义一个 `enum NetworkEvent { ... }`，再在 `Event` 里写 `Network(NetworkEvent)`。但这会强迫你在构造和解构时多套一层类型，而且如果底层需要跨整个 `Event` 统一规划紧凑的判别编码，拆成两个独立类型往往会引入额外的嵌套布局开销。

在 Flurry 中，你可以直接在**同一个 `enum`** 内部使用 **`id.{ ... }` 分组变体语法**，把变体组织成一棵层次分明的树。

让我们看看 改进版本是如何用分组变体改写 `Event` 的：

```flurry
use std.prelude.*;

typealias Byte = u8;

enum Event {
    -- 用 `network.{ ... }` 直接在同一个 enum 内建立层次分组。
    network.{
        connected,
        received(Byte),
        nested.{
            stalled,
        },
    },
    stopped,
}

-- 整组匹配：只需写一个 `.network`，自动覆盖该分组树下的全部后代叶变体。
fn is_network(value: Event) -> bool {
    value match {
        .network => true,
        .stopped => false,
    }
}

test {
    -- 构造时沿着点号路径直达叶变体：
    assumes is_network(Event.network.connected);
    assumes is_network(.network.received(3u8));
    assumes is_network(.network.nested.stalled);
    assumes not is_network(Event.stopped);
}
```

下面来看看编译器眼中的 `enum Event` 变体树与 `.network` 整组匹配：

```text
                enum Event (封闭变体树)
                  ┌────────┴────────┐
                  ▼                 ▼
              [network]          stopped (叶索引 #3)
          (分组节点，非独立值!)
        ┌─────────┼─────────┐
        ▼         ▼         ▼
    connected  received  [nested]
    (叶 #0)    (Byte)       │
               (叶 #1)      ▼
                         stalled
                         (叶 #2)

  ══════════════ 当你写 `.network => true` 时 ══════════════
  编译器自动将 `.network` 展开为整棵子树所有后代叶变体的析取：
  .network.connected  OR  .network.received(_)  OR  .network.nested.stalled
```

这里有三条必须牢记的语义边界：
1. **整组匹配覆盖所有后代叶变体**：写 `.network` 并非但匹配直接子变体 `connected` 和 `received(_)`，连更深一层的 `nested.stalled` 也一并覆盖。同时它未绑定载荷，但会照常履行载荷原有的资源清理责任。如果你需要取出具体载荷，写具体叶路径 `.network.received(byte)` 即可。
2. **分组名并非值，亦非独立类型**：`network` 只是 `Event` 内部的一条分组路径。你**无法**脱离叶变体单独构造一个所谓的“分组值”，也**无法**把 `Event.network` 当成类型写在函数签名上：

```flurry
-- ❌ 编译期直接报错。
fn bad_group_usage() {
    -- 错误 1：`network` 是分组名，不是叶变体，禁止直接构造出值。
    let e = Event.network;
}

-- ❌ 编译期直接报错。
-- 错误 2：`Event.network` 没有独立类型身份，禁用作参数类型。
fn bad_group_param(sub: Event.network) {}
```

3. **路径边界严格按层级划分**：整组匹配按完整路径计算成员集合。哪怕你在同一个 `enum` 里定义了邻组 `net` 和 `network`，匹配 `.net` 免按字符串前缀误伤 `.network`。

---

## 三、 `'tag_name` 视图与 `Symbol` 编码枚举——声明名 vs 底层编码

现在来到第三个需求：
假设我们的网关状态枚举携带了泛型载荷 `Status<T>`，我们需要写一个 `describe` 函数，返回当前变体的名称字符串 `str`（比如 `"network.ready"`），用于打监控日志或作为编译期元编程（Normal-Form / Staged 参数）的键值。

如果手写一个 `match` 把每个变体映射成字符串，非但繁琐，而且万一变体里有仿射资源载荷，偶有疏忽解构出来还会触发所有权移动。

为此，Flurry 提供了原生的一等视图：**`expr'tag_name`**。

```flurry
use std.prelude.*;

enum Status where T {
    network.{
        ready(T),
    },
    closed,
}

fn describe(value: Status<bool>) -> str {
    -- `'tag_name` 直接返回叶变体的声明路径名字符串，且不转移载荷所有权。
    value'tag_name
}

test {
    let s = Status<bool>.network.ready(true);
    -- 读取已有局部变量的 'tag_name 后，原变量 s 依然完好可用。
    assumes s'tag_name == "network.ready";
    assumes describe(s) == "network.ready";
    assumes describe(Status<bool>.closed) == "closed";
}
```

看看 `'tag_name` 的威力：
- **自动保留分组路径**：对于分组下的叶变体 `network.{ ready(T) }`，`'tag_name` 返回的是带点号完整路径的 `"network.ready"`。
- **非破坏性读取**：操作数只求值一次；读取已有局部变量或字段的 `'tag_name` **不会转移它的载荷**，取完名字后原枚举值依然可以继续使用。
- **对 `Symbol` 同样适用**：如果你手里有一个纯符号 `.ok`，写 `.ok'tag_name` 也会直接得到字符串 `"ok"`。

### 当枚举以 `Symbol` 为底层编码：`.ready` vs `.wire_ready`

更进一步：在很多跨模块协议或编译期配置中，我们既希望有一个类型安全的名义枚举 `enum WireMessage`，又希望它在底层的编码直接对应到一组特定的 `Symbol`（例如底层线缆符号叫 `.wire_ready`、`.wire_waiting`，而在高层业务代码里我们想叫它 `.ready`、`.waiting`）。

还记得第一篇里的 `.base_type` 和 `.discrete_tag true` 吗？
在 Flurry 中，**`Symbol` 也可以作为 `enum` 的 `.base_type`。**

```flurry
use std.prelude.*;

comptime fn ready_code() -> Symbol { .wire_ready }

enum WireMessage {
    .base_type Symbol,
    .discrete_tag true,

    -- 可以用编译期函数计算的 Symbol 表达式编码：
    ready = ready_code(),
    -- 也可以用单个常量 Symbol 模式编码：
    waiting: .wire_waiting,
    closed = .wire_closed,
}

fn is_ready(msg: WireMessage) -> bool {
    msg match {
        .ready => true,
        _ => false,
    }
}

test {
    let m = WireMessage.ready,
    assumes is_ready(m);

    -- 重点来了。m'tag_name 返回的是什么？
    -- 是声明名 "ready"，而非底层 Symbol 编码名 "wire_ready"。
    assumes m'tag_name == "ready";
}
```

下面来看清 **`WireMessage.ready`** 的双面人生：

```text
  高层名义视角 (Nominal Type: WireMessage)
  ┌─────────────────────────────────────────┐
  │ 变体身份: WireMessage.ready             │ ──► m'tag_name ──► "ready" (声明名称!)
  │ 匹配写法: .ready => ...                 │
  └────────────────────┬────────────────────┘
                       │ (.base_type Symbol + .discrete_tag true)
                       ▼
  底层编码视角 (Representation Domain: Symbol)
  ┌─────────────────────────────────────────┐
  │ 底层存储值: .wire_ready                 │ ──► (.wire_ready)'tag_name ──► "wire_ready"
  └─────────────────────────────────────────┘
```

而且，因为 `.discrete_tag true` 要求变体之间靠底层编码区分，如果你在定义 `WireMessage` 时偶有疏忽让两个变体指向了同一个底层 `Symbol`：

```flurry
-- ❌ 编译期直接报错。
enum BadWireMessage {
    .base_type Symbol,
    .discrete_tag true,
    ready = .wire_same,
    closed = .wire_same, -- 与 ready 的 Symbol 编码冲突。
}
```

编译器会在定义检查阶段立刻报错，明确指出冲突的变体名与重复的 `Symbol`。同时，因为 `Symbol` 没有隐含的整数位宽，对 `.base_type Symbol` 的枚举**无法同时指定 `.width`，亦无法附加变体载荷**——语义边界干净透明。

---

## 本篇小结

从高层业务消息到底层符号枚举，我们在本篇解锁了 Flurry `enum` 的全景工具箱：

| 特性 | 语法形式 | 核心语义与工程价值 |
| :--- | :--- | :--- |
| **三种基础变体** | `quit` / `repeat(u32)` / `move_to { x: i32, y: i32 }` | 统一支持无载荷、位置载荷与命名字段载荷；在明确目标类型上下文下支持 `.variant` 简写 |
| **层次化分组变体** | `network.{ connected, received(Byte), nested.{stalled} }` | 在同一 `enum` 内组织变体树；模式 `.network` 自动覆盖整棵子树的全部后代叶变体；分组名本身并非值或独立类型 |
| **`'tag_name` 视图** | `val'tag_name` | 取得叶变体的声明路径名字符串（如 `"network.ready"`）或 `Symbol` 的名字；不转移枚举载荷所有权 |
| **`Symbol` 编码枚举** | `.base_type Symbol` + `.discrete_tag true` | 用编译期 `Symbol` 表达式或常量模式直接编码变体，编译期强制检查 `Symbol` 编码互斥 |

---

### 下一篇预告

在这一篇和第 001 篇里，我们看到了 Flurry 枚举系统的两端：
- 一端是高层的逻辑变体与分组树；
- 另一端是第 001 篇里那个只有 8 比特（`u8`）的 `Packet`。

现在，硬件团队给你扔来了一块真正的硬骨头：
传感器主控板上搭载了一颗 **32 位 RISC-V 协处理器**。你需要用 Flurry 为它编写一个指令汇编与反汇编核心模块。
翻开 RISC-V 手册，一条 32 位算术指令（比如 `ADD` 和 `SUB`）被切成了整整 **6 个位段**：
`funct7 (7位) | rs2 (5位) | rs1 (5位) | funct3 (3位) | rd (5位) | opcode (7位)`。

- 三个寄存器编号 `rs2`、`rs1`、`rd` 被夹在中间当三明治；
- 决定它是 `ADD` 还是 `SUB` 的固定位，却分散在最高位的 `funct7`、中间的 `funct3` 和最低位的 `opcode` 三处。
- 如果我们用 `.base_type u32` 定义 `enum ArithmeticInstruction`，编译器难不成要遍历 $2^{32} = 4294967296$ 个整数，才能证明 `add` 和 `sub` 不会撞车（互斥性）吗？

在下一篇 **《Flurry 演进手记 (014)：手写 RISC-V 指令解码器——32 位定宽 `enum` 的互斥性与覆盖性证明》** 中，我们将亲手用两行位向量模式定义出零开销的 RISC-V 指令枚举，并揭秘编译器内部一招制敌的位掩码异或证明公式。
