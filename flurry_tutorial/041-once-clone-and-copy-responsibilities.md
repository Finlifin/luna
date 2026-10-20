# Flurry 演进手记 (041)：三个层次的值宿命——Once 消耗、显式 Clone 与位级 Copy

欢迎来到 **《Flurry 演进手记》第三卷**。

在前两卷中，我们已经打造了一台模块化的 8 比特传感器协议分析仪。但随着采样率从每秒几百次飙升到每秒数百万次（MHz 级高速采集），原本“来一个字节处理一个字节”的同步模式彻底撑不住了。

从这一篇开始，我们将开启一段全新的硬核工程征程：**从零实现一套高性能、零拷贝、带静态安全证明的环形缓冲区（`RingBuffer`）与内存池分配器。**

在动手写 `RingBuffer` 的第一行代码前，我们需要设计三种最核心的基础数据结构：
1. **`DmaTicket`（硬件 DMA 传输凭证）**：代表一次正在进行的硬件直接内存写入任务。每张凭证**只能被提交完成一次**——如果提交两次，DMA 控制器就会触发硬件异常；如果忘了提交，离开作用域时也要自动回收通道。
2. **`SlotCoord`（环形槽位坐标）**：只包含两个 `u32` 整数（`ring_id` 和 `index`），我们需要在计算读写游标时像传普通整数一样到处复制它。
3. **`BufferView`（缓冲区观察视图）**：内部持有一个指向底层数据的指针 `*i32`，允许显式复制出多个观察哨去读取同一个槽位，或者为独立缓冲区分配副本。

如果你来自 C/C++，按值传递默认就是“把内存字节拷一份”（或者触发隐式的拷贝构造函数），稍不留神就会把唯一凭证拷出两份导致双重释放（Double Free）；如果你来自 Rust，你可能对 `Move`、`Clone`、`Copy` 很熟悉，但往往把 `Clone` 等同于“深拷贝”，或者困惑于为什么复制指针不需要借用检查器锁死原对象。

在 Flurry 中，每一个运行时值从诞生的那一刻起，就处在 **`Copy :- Clone :- Once`** 这三层能力阶梯之中（详见 `docs/resource-management/once-clone-copy.md`）。让我们从 传统的踩坑现场开始，一层层揭开它们的宿命。

---

## 一、 为什么只装了一个 `u32` 的结构体，传参后就不能再用了？

为了表示 DMA 传输凭证，我们定义了一个极其简单的结构体 `DmaTicket`，里面只装了一个 `u32` 类型的编号 `number`。同时，我们还定义了一个坐标结构体 `SlotCoord`。

一位习惯了 C 语言“小结构体直接按值拷贝”的工程师写下了这样的 初始代码：

```flurry
-- ❌ 编译期直接拒绝。
use std.prelude.*;

struct DmaTicket {
    number: u32,
}

struct SlotCoord {
    ring_id: u32,
    index: u32,
}

fn finish_dma(ticket: DmaTicket) {
    println("Finished DMA ticket #{}", ticket.number);
}

test {
    let ticket = DmaTicket { number: 7u32 }
    finish_dma(ticket);

    -- 坑 1：工程师不小心又调用了一次 finish_dma(ticket)。
    -- 编译器直接报错：ticket 已经被移出（Moved out），不能第二次按值交付。
    finish_dma(ticket);

    -- 坑 2：工程师以为 SlotCoord 里只有两个 u32，赋值给 c2 后 c1 还能继续按值传走
    let c1 = SlotCoord { ring_id: 0u32, index: 4u32 }
    let c2 = c1;
    let c3 = c1; -- ❌ 编译期同样拒绝。SlotCoord 默认也是 Once 仿射移动。
}
```

这位工程师既庆幸又困惑：
- **庆幸的是**：编译器在编译期就抓住了 `finish_dma(ticket)` 被误调两次的致命 Bug。
- **困惑的是**：`DmaTicket` 和 `SlotCoord` 里面明明只装了 `u32`，而 `u32` 本身明明是可以随便复制的。为什么套了一层 `struct` 外壳，它们就都不能隐式复制了？

---

## 二、 第一层宿命 `Once` 与第三层宿命 `Copy`——名义身份与责任边界

### 1. 默认的仿射契约：`Once`（至多消耗一次）

在 Flurry 中，所有运行时值默认遵循 **`Once` 的仿射类型（Affine Type）边界**：
> **一个非 `Copy` 的值，在其生命周期内至多被“消耗（Consume）”一次。**

什么叫“消耗”？
当你用它来**初始化另一个按值绑定**（`let b = a;`）、**按值传给函数参数**（`finish_dma(ticket)`）或者**按值作为函数返回值**时，这个值连同它背负的可转移清理责任（Responsibility），就从源位置**移动（Move）**到了目标位置，源位置立刻变成**已移出（Moved-out）**状态。

但请特别注意 Flurry 对 `Once` 的两个精准界定：
1. **仿射（Affine）不等于线性（Linear）**：线性类型要求你必须手动显式消费每一个值；而仿射类型允许你“至多消费一次”——如果你直到作用域结束都没有把它移交给别人，编译器的自动清理机制（`Drop`）就会替你兑现它剩余的责任。
2. **限制的是“按值消耗”，绝不限制消耗前的“合法观察”**。在 `ticket` 被移动给 `finish_dma` 之前，你完全可以通过指针（`ticket.ref`）或字段读取观察它无数次；观察不会凭空制造出第二个 Owner。

### 2. 为什么 `SlotCoord` 需要显式 `derive Copy`？

那么，为什么 `SlotCoord { ring_id: u32, index: u32 }` 不会因为字段全是 `u32` 就自动变成可复制的呢？
因为 Flurry 严格区分**结构能力条件**与**名义类型契约**：
- 像 `DmaTicket { number: u32 }` 这样的类型，在物理内存上虽然只是一个 `u32`，但在业务语义上它是一张**不可复制的唯一凭证**。如果编译器因为“里面只有 `u32`”就自作主张让所有这类结构体都能隐式复制，那凭证类型、句柄包装就全都失去仿射保护了。
- 因此，在 Flurry 中，所有用户名义类型（`struct` / `enum`）默认都是 `Once`（移动语义）。只有当你**明确声明或派生 `Copy`** 时，它才获得隐式按位复制的能力；而“所有字段都可 `Copy`”只是编译器允许你派生 `Copy` 的**检查前提**。

由于能力阶梯满足 **`Copy :- Clone :- Once`**（即任何可 `Copy` 的类型首先必须具备 `Clone` 和 `Once` 能力），我们只需为 `SlotCoord` 加上两行 `derive`：

```flurry
struct SlotCoord {
    ring_id: u32,
    index: u32,
}

-- 显式派生 Clone 与 Copy：告诉编译器 SlotCoord 是纯粹的值坐标，允许按位复制。
derive Clone for SlotCoord;
derive Copy for SlotCoord;

-- 在泛型函数中，如果需要把同一个按值参数交付两次，必须显式声明 `where T :- Copy`
fn duplicate_value(value: T) -> Tuple<T, T>
where T :- Copy
{
    (value, value)
}
```

如下所示，对比 `Once` 移动与 `Copy` 复制在编译器账本上的本质区别：

```text
  动作 A：非 Copy 交付（如 `let t2 = ticket;`）
  ┌───────────────────┐          ┌───────────────────┐
  │ 源位置 `ticket`   │          │ 目标位置 `t2`     │
  │ 状态: [已移出 ❌] │ ──转移──►│ 状态: [已初始化 ✅]│
  │ 责任: 无          │          │ 责任: 接管原责任  │
  └───────────────────┘          └───────────────────┘
  (注：物理上可能只是寄存器/栈复制，无需擦除源内存字节，但静态账本禁止再读源位置。)

  动作 B：Copy 交付（如 `let c2 = c1;`，其中 `SlotCoord :- Copy`）
  ┌───────────────────┐          ┌───────────────────┐
  │ 源位置 `c1`       │          │ 目标位置 `c2`     │
  │ 状态: [保持可用 ✅]│ ──复制──►│ 状态: [已初始化 ✅]│
  │ 责任: 保留原状态  │          │ 责任: 合法表示副本│
  └───────────────────┘          └───────────────────┘
```

> **💡 核心心智模型：为什么拥有堆内存的类型绝对不能是 `Copy`？**
> 仔细看内置规则：**普通指针 `*T` 本身是 `Copy` 的**（复制一个指针只是把 64 位地址再抄一份，两份指针依然指向同一个目标）。
> 但是，如果一个结构体 `OwnedBuffer { ptr: *u8 }` 负责在 `Drop` 时释放 `ptr` 指向的内存，哪怕它的唯一字段 `ptr` 在语法上是 `*u8`（可 `Copy`），`OwnedBuffer` 也**绝对不能获得 `Copy`**。因为 **`Copy` 绝不允许复制独占的终结责任（Drop Responsibility）**——一旦允许按位复制出两个 `OwnedBuffer`，作用域结束时就会对同一个指针释放两次。

---

## 三、 第二层宿命 `Clone`——显式克隆不等于“深拷贝断绝关系”

现在来看中间那一层：**`Clone`（`expr.clone()`）**。

很多教程在讲 `Clone` 时，会随口说一句：“`Copy` 是浅拷贝，`Clone` 是深拷贝，克隆出来的对象和原对象互不相干。”
**在系统编程与形式化验证中，这种说法是极其危险的误导。**

为什么？让我们看 `RingBuffer` 里的第三个结构：`BufferView`。
`BufferView` 是一个不拥有底层内存、只包装了一个指针 `*i32` 的观察哨。我们不想让它被随手隐式 `Copy`（希望每次制造新观察哨都在代码里显式写出 `.clone()` 以便审计），于是我们只为它实现 `Clone`，不实现 `Copy`：

```flurry
use std.prelude.*;

struct BufferView {
    pointer: *i32,

    fn read(*self) -> i32 {
        self.*.pointer.*
    }
}

impl Clone for BufferView {
    fn clone(*self) -> Self {
        -- 复制内部保存的指针，制造出第二个指向同一目标的视图。
        Self { pointer: self.pointer.* }
    }
}
```

看看当我们调用 `let second = first.clone();` 时发生了什么：

```text
  底层槽位: var slot_val = 3i32;  ──────► 后来修改为: slot_val = 4i32;
                 ▲                                      ▲
                 ├──────────────────────┐               │
                 │                      │               │
         ┌───────┴────────┐     ┌───────┴────────┐      │
         │ first          │     │ second         │      │
         │ (BufferView)   │     │ (first.clone())│      │
         └────────────────┘     └────────────────┘      │
                 │                      │               │
                 └──────────────────────┴───────────────┘
                 两个视图依然依赖同一个 `slot_val`。
                 此时调用 `first.read()` 和 `second.read()` 都会读到最新的 `4`。
```

看到了吗？在 Flurry 中（详见 `docs/resource-management/once-clone-copy.md`）：
1. **`Clone` 的核心承诺是“显式产生另一个合法值，同时保留源值合法”**；
2. 至于新值和源值之间是**分配了全新独立的缓冲区**，还是像 `BufferView` 或引用计数智能指针那样**共享同一个底层目标**，完全由该类型 `Clone` 实现的具体契约（Contract）决定。
3. 编译器绝不会机械地因为你写了 `.clone()`，就盲目假设“新对象和原对象从此井水不犯河水”——在 `BufferView.clone()` 里，编译器清清楚楚地追踪到 `second` 继承了 `first` 对 `slot_val` 的目标依赖。
4. 同时，因為 `Clone` 是一次正经的方法调用，它不像 `Copy` 那样只是静默的位复制——`Clone` 可能会分配内存、修改引用计数或读取外部状态，不能被编译器当作无副作用的死代码随便合并或消除。

让我们把 `DmaTicket`（`Once`）、`BufferView`（`Clone`）和 `SlotCoord`（`Copy`）组合进一段完整的测试代码中，检验这三层能力阶梯：

```flurry
use std.prelude.*;

-- 1. 第一层 Once：唯一凭证，禁止复制或克隆
struct DmaTicket {
    number: u32,
}

fn consume_ticket(ticket: DmaTicket) -> u32 {
    ticket.number
}

-- 2. 第二层 Clone：显式克隆视图，保留对底层槽位的真实共享依赖
struct BufferView {
    pointer: *i32,

    fn read(*self) -> i32 {
        self.*.pointer.*
    }
}

impl Clone for BufferView {
    fn clone(*self) -> Self {
        Self { pointer: self.pointer.* }
    }
}

-- 3. 第三层 Copy：纯值坐标，支持隐式按位复制与泛型重复交付
struct SlotCoord {
    ring_id: u32,
    index: u32,
}

derive Clone for SlotCoord;
derive Copy for SlotCoord;

fn make_pair(item: T) -> Tuple<T, T>
where T :- Copy
{
    (item, item)
}

test {
    -- 测试 Once：交付一次后责任转移完成
    let ticket = DmaTicket { number: 7u32 }
    let id = consume_ticket(ticket);
    assumes id == 7u32;

    -- 测试 Clone：显式调用 .clone()，两个视图共享对 slot 的依赖
    var slot = 3i32;
    let view1 = BufferView { pointer: slot.ref }
    let view2 = view1.clone();
    slot = 4i32;
    assumes view1.read() == 4i32 and view2.read() == 4i32;

    -- 测试 Copy：隐式复制，c1 在复制后依然可用
    let c1 = SlotCoord { ring_id: 1u32, index: 8u32 }
    let c2 = c1;
    let (p1, p2) = make_pair(c1);
    assumes c2.index == 8u32 and p1.index == p2.index;
}
```

---

## 本篇小结

在开启第三卷的第一站，我们确立了 Flurry 值与资源管理的基石：

| 层次 | 代表特征 | 源位置状态 | 目标位置与责任契约 |
| :--- | :--- | :--- | :--- |
| **`Once`（默认）** | 仿射移动语义 | 按值交付后变为**已移出** | 目标接管原值与可转移责任；不限制移出前的合法指针观察 |
| **`Clone`** | 显式调用 `.clone()` | 保持合法可用 | 按具体实现契约生成新值（可独立分配，也可保留对同一目标的共享依赖） |
| **`Copy`** | 隐式表示复制 (`Copy :- Clone :- Once`) | 保持合法可用 | 仅复制位表示并传播既有目标依赖；**严禁复制独占终结责任**，用户名义类型须显式声明/派生 |

### 下一篇预告

在今天的代码里，无论是移动 `ticket` 还是复制 `c1`，我们操作的都是**一整个结构体**。

但是在实现 `RingBuffer` 的环形槽位节点时，一个槽位结构体 `SlotNode` 往往既有仿射的数据载荷字段 `payload: DataFrame`（非 `Copy`），又有记录元数据的字段 `seq_num: u32` 和 ` backup_payload: DataFrame`：
- 如果我们只把 `node.payload` 这**一个字段**按值移走了（比如交给了消费者：`let taken = node.payload;`），剩下的 `node.seq_num` 和 `node.backup_payload` 还能继续读取或移出吗？
- 更刺激的是：如果 `SlotNode` 自身还实现了 `impl Drop for SlotNode`，在 Rust 里，只要一个结构体实现了 `Drop`，你就**绝对不允许移出它的任何字段**（哪怕 `Drop` 里根本没用到那个字段）。Flurry 也是这么固定吗？
- 如果我们在移走 `node.payload` 之后，紧接着又给它塞进了一个新数据包 `node.payload = new_frame;`，这个被掏空过一半的 `node` 能不能“恢复完整初始化状态”，重新作为一个完整的 `SlotNode` 返回出去？

在下一篇 **《Flurry 演进手记 (042)：把结构体拆开搬走一半——部分移出（Partial Move）、重新初始化与合并交付》** 中，我们将深入编译器内部的“位置状态树”，揭秘字段级外科手术的全部细节。
