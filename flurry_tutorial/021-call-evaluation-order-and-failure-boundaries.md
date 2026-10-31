# Flurry 演进手记 (021)：实参求值的核心规则——源码从左到右求值、方法接收者与失败回滚边界

欢迎来到 **《Flurry 演进手记》第二卷：函数的十八般武艺**。

在[第一卷的大结局 (020)](./020-milestone-sensor-protocol-analyzer.md) 中，我们已经亲手组装出了一台完整的 8 位传感器协议分析仪。它能用定宽 `enum` 和跨元素 `BitVec` 模式解析硬件帧，也能用 `!Err T` 捕获线路噪声。

从今天开始，我们的分析仪不再只是一个单打独斗的解码脚本——我们要为它构建一套**可配置、支持插件回调、零样板代码的查询与事件处理引擎**。

而在设计任何复杂函数接口之前，我们必须先解决一个在系统编程中潜伏了半个世纪的“幽灵问题”。

今天，硬件团队给分析仪加装了一个带硬件槽位锁（`SlotLease`）的帧发送器。要发送一个传感器数据帧，我们需要依次做三件事：
1. 从串口读取帧头：`read_header()!`（可能因为线路噪声报错 `!IoErr`）；
2. 从硬件缓冲池申请一个发送槽位：`allocate_slot()`（返回一个持有硬件资源的 `SlotLease`，销毁时必须执行 `Drop` 归还槽位）；
3. 从串口读取载荷：`read_payload()!`（同样可能报错 `!IoErr`）；
4. 把这三样东西一起交给 `send_frame(header, slot, payload)` 发送出去。

为了代码紧凑，你很自然地把它们写成了一行嵌套调用：

```flurry
send_frame(read_header()!, allocate_slot(), read_payload()!);
```

停下来想一想：
- **问题一**：`read_header()!`、`allocate_slot()` 和 `read_payload()!`，到底谁先执行？如果换成方法调用 `transmitter.send_frame(...)`，接收者 `transmitter` 和后面的参数谁先求值？如果后续实参顺手修改了 `transmitter` 的字段，会发生什么？
- **问题二**：假设第 1 个参数 `read_header()!` 成功了，第 2 个参数 `allocate_slot()` 也成功拿到了硬件槽位，但第 3 个参数 `read_payload()!` **突然报错提前返回了**。此时 `send_frame` 的函数体根本还没进门，第 2 个参数申请到的 `SlotLease` 归谁管？是否会永久泄漏？

如果我们带着 C/C++ 的陈旧防御习惯来写这段代码，会写成什么样？

---

## 一、 被 C/C++“未指明求值顺序”支配的恐惧

在 C 和 C++（直到 C++17 之前，甚至 C++17 之后的函数各实参之间）中，**函数实参的求值顺序是未指明的（Unspecified）**。
编译器为了迎合不同 CPU 架构的传参寄存器约定（比如有的 ABI 习惯从右往左压栈，有的习惯前几个放寄存器），完全可能先算第 3 个参数 `read_payload()`，再算第 1 个参数 `read_header()`。

如果你在两个实参里读取同一个硬件 FIFO 寄存器，从右往左求值会直接把帧头和载荷读反。更麻烦的是，在早期 C++ 中，像 `foo(std::unique_ptr<T>(new T), may_throw())` 这样的代码曾经是著名的内存泄漏重灾区——因为编译器可能先 `new T`，再去调 `may_throw()`，一旦抛异常，`unique_ptr` 的构造函数还没跑完，内存就永远丢了。

被这种阴影吓怕了的系统程序员，在传统写法中往往不敢写嵌套调用，而是被迫把每一个实参都拆成独立的临时变量语句：

```flurry
use std.prelude.*;

enum IoErr {
    LineNoise,
    PayloadTruncated,
}

struct SlotLease {
    slot_id: u32,
}

impl Drop for SlotLease {
    fn drop(*self) {
        println("Releasing hardware slot {}", self.slot_id.*);
    }
}

fn read_header(port: *u32) -> !IoErr u32 {
    println("1. Reading header...");
    port.* += 1u32;
    return error IoErr.LineNoise while port.* == 99u32;
    0xA0u32
}

fn allocate_slot(id: u32) -> SlotLease {
    println("2. Allocating slot {}...", id);
    SlotLease { slot_id: id }
}

fn read_payload(port: *u32, fail: bool) -> !IoErr u32 {
    println("3. Reading payload...");
    return error IoErr.PayloadTruncated while fail;
    port.* += 1u32;
    0x05u32
}

fn send_frame(header: u32, slot: SlotLease, payload: u32) {
    println("4. Sending frame: header={}, slot={}, payload={}", header, slot.slot_id, payload);
}

-- 因为害怕实参求值乱序和异常泄漏，手动拆成满屏的临时变量
fn transmit_once_initial(port: *u32, fail_payload: bool) -> !IoErr void {
    let h = read_header(port)!;
    let s = allocate_slot(1u32);
    let p = read_payload(port, fail_payload)!;
    send_frame(h, s, p);
}
```

### 传统结构示意

为什么 C/C++ 老兵非要把一行调用拆成四行？因为他们脑子里的调用模型是这样的：

```text
  传统未指明次序语言的难题：
  send_frame(  read_header()  ,  allocate_slot()  ,  read_payload()  )
                    ▲                   ▲                   ▲
                    └─────────── ? ─────┴───── ? ───────────┘
               "编译器为了凑寄存器 ABI，是否会倒着先算 read_payload()？"
               "万一算到一半退出了，allocate_slot() 造出来的临时对象谁来销毁？"
```

拆成 `let h = ...; let s = ...; let p = ...;` 固然安全，但它带来了两个明显的核心问题：
1. **命名污染与样板膨胀**：每次调个函数都要想三个无意义的短命变量名 `h`、`s`、`p`，原本一目了然的数据流被切得支离破碎。
2. **不敢组合具名实参或链式方法**：一旦后面引入具名参数（如 `.payload = ..., .header = ...`）或者方法调用，难道每次调整写法都要重新拆一遍局部变量？

在 Flurry 中，你全然无须这种防御性样板。

---

## 二、 源码从左到右求值的核心规则与调用准备边界

Flurry 在语言规范（`docs/functions/calls-and-returns.md`）的第一句话里就立下了核心规则：

> **调用有自己的执行次序。把一个实参写到另一个实参前面，就决定了它们产生效果的先后；形参的排列决定值交到哪里。**

更具体地说，一次普通运行时调用严格遵循以下四步**调用准备（Call Preparation）**阶段：

```text
  Flurry 的四步调用准备与责任移交时间线：

  [第 1 步：求被调方 / 接收者]
     求值 callee 表达式（如 fn_picker()），或方法调用先取得接收者 (receiver)
        │
        ▼
  [第 2 步：按源码从左到右求值显式实参]
     arg_1 ──(成功，登记临时责任)──► arg_2 ──(成功，登记临时责任)──► arg_3 ...
        │                               │                               │
        └──(若提前退出：后续不求值。)  └──(若提前退出：清理 arg_1。)    └──(若提前退出：逆序清理 arg_2, arg_1。)
        │
        ▼
  [第 3 步：按形参声明依赖次序补齐缺省值] (将在 022 篇详解)
        │
        ▼
  [第 4 步：移交责任并进入函数主体]
     将准备好的全部实参正式移交给 callee 形参，caller 不再持有清理责任，开始执行函数体。
```

注意这里两个尤为关键的设计细节：
1. **求值次序只认源码顺序，与机器 ABI 传参顺序解耦**：无论底层目标机器的寄存器传参约定是从左往右还是从右往左，Flurry 编译器都会**先严格按照你在源码里从左到右写的顺序求值每个实参**，把算出来的值暂存好，最后才在进入机器调用指令前排好寄存器。而且，哪怕某个普通参数在函数体里根本没被用到，它的实参表达式也**必须求值一次**，编译器免因“函数没读这个参数”就悄悄删掉实参里的 I/O、状态修改或 `Drop`。
2. **调用准备阶段的精确回滚边界**：在进入 `send_frame` 主体之前，所有已经求值成功的并非 `Copy` 实参（比如第 2 个参数 `allocate_slot(1u32)` 返回的 `SlotLease`）都由**调用准备阶段（Caller 侧）**登记并持有临时责任。
   - 如果第 3 个参数 `read_payload(...)!` 报错退出：`send_frame` 的主体**根本跳过进入**；调用准备阶段会立即沿着失败退出路径，将此前已经取得的临时实参（`SlotLease`）按逆序执行 `Drop` 清理。
   - 如果第 1 个参数 `read_header(...)!` 就直接报错了：第 2 个参数 `allocate_slot` 和第 3 个参数 `read_payload` **压根跳过执行**，既然没创建资源，自然亦免于触发 `Drop`。
   - 一旦所有实参都准备完毕、正式跨过函数入口边界，清理责任就**原子性地移交给了 `send_frame` 的形参**——规避出现调用方和被调方各自 `Drop` 一次的双重释放（Double Free）。

下面将其重构为直接嵌套调用的形式，并验证它在成功和失败路径下的输出顺序：

```flurry
-- 沿用上面的 IoErr、SlotLease、read_header、allocate_slot、read_payload、send_frame

-- 直接一行嵌套调用，求值次序与失败回滚由语言核心规则保障。
fn transmit_once(port: *u32, fail_payload: bool) -> !IoErr void {
    send_frame(
        read_header(port)!,
        allocate_slot(1u32),
        read_payload(port, fail_payload)!,
    );
}

test {
    var port = 0u32;

    println("=== Case A: Payload fails ===");
    transmit_once(port.ref, true)! {
        IoErr.LineNoise => println("Caught LineNoise"),
        IoErr.PayloadTruncated => println("Caught PayloadTruncated"),
    }

    println("=== Case B: All succeed ===");
    transmit_once(port.ref, false)! {
        IoErr.LineNoise => (),
        IoErr.PayloadTruncated => (),
    }
}
```

看看运行输出，就像钟表齿轮一样严丝合缝：

```text
=== Case A: Payload fails ===
1. Reading header...
2. Allocating slot 1...
3. Reading payload...
Releasing hardware slot 1
Caught PayloadTruncated
=== Case B: All succeed ===
1. Reading header...
2. Allocating slot 1...
3. Reading payload...
4. Sending frame: header=160, slot=1, payload=5
Releasing hardware slot 1
```

在 `Case A` 中：
1. `read_header` 先执行；
2. `allocate_slot(1)` 接着执行，拿到了槽位 1；
3. `read_payload` 执行并触发 `!` 提前返回错误；
4. 注意。第 4 步 `Sending frame` 根本没有执行，但在离开 `transmit_once` 之前，调用准备阶段稳稳地打印出了 `Releasing hardware slot 1`。槽位准确无误地归还给了硬件池。

---

## 三、 方法接收者、具名结果与返回值责任移交

现在，我们把发送逻辑升级为一个结构体 `Transmitter` 的方法，并加上两个新需求：
1. **方法调用与接收者状态**：如果有人写 `tx. push_value(mutate_tx_and_get_arg(tx.ref))`，接收者 `tx` 和参数谁先求值？在 Rust 中，这行代码会直接触发借用检查器报错（因为 `tx.` 已经对 `tx` 产生了可变借用锁，实参里禁止再次借用 `&mut tx`）。在 Flurry 中呢？
2. **带契约的返回值与局部资源清理**：当一个函数要在多个分支里提前 `return` 一个资源（比如从主备两个槽位里挑一个返回），或者需要用 `where ensures` 约束返回值满足某个条件时，返回值和函数内没被选中的局部资源是怎么交接的？

### 1. 接收者先取得，但它免于“冻结”你的对象

在 Flurry 中，方法调用 `receiver.method(arg1, arg2)` 遵循第 1 步规则：**取得接收者发生在求值显式实参之前**。
当方法签名是 `fn update_Seq(*self, next: u32)` 时：
1. 先定位接收者，取得指向对象的普通指针 `*Transmitter`；
2. **这个指针不会建立“独占冻结锁”**。如果后面的实参表达式顺手修改了同一个对象的字段，完全合法——随后进入 `update_seq` 函数体时，看到的就是实参修改之后的最新状态。

```text
  tx.update_seq( bump_and_supply(tx.ref) ) 的执行时间线：

  [第 1 步] 先取得接收者指针：self_ptr = tx.ref (此时 tx.seq == 4)
               │ (注意：没有独占借用锁。对象未冻结。)
               ▼
  [第 2 步] 求值实参 bump_and_supply(tx.ref)：
            将 tx.seq.* 改写为 12，并返回实参值 18
               │
               ▼
  [第 3 步] 进入 update_seq(self_ptr, 18) 主体：
            此时读取 self.seq.* 会读到 12，随后将其更新为 18。
```

### 2. 具名返回值 `-> result: T` 与返回时的责任移交

在写函数返回类型时，除了写 `-> u32`，Flurry 还支持给返回结果起个名字，写成 **`-> result: u32`**。
为什么要给返回值起名字？因为当你配合 `where ensures` 编写后置条件契约（比如保证挑选出来的槽位编号一定小于上限）时，你可以直接在 `ensures` 子句里引用 `result`。

> **🔍 语法小贴士：具名结果 `-> result: T` 并非未初始化的局部变量。**
> 有些语言（如 Pascal 或 Go）的具名返回值会在函数开头自动声明一个默认初始化的局部变量。
> Flurry **不这样做**。在 Flurry 中，`-> result: T` 里的 `result` **只是函数最终交付结果的绑定名，专供结果规格（如 `where ensures result <= limit`）引用**；在函数体内，你依然通过块末尾表达式或 `return expr` 正常交付结果，而非把 `result` 当成一个未初始化的局部槽去随意赋值。
> 另外，如果一个函数体只有单个表达式，你还可以直接用表达式主体语法 **`fn foo(...) -> T = expr;`**（注意以表达式结尾时末尾要写分号 `;`）。

同时，当函数 `return selected;` 返回一个持有资源的局部对象时：
- **被选作返回值的 `selected`**：它的资源责任随着返回值一起**移交（Move）给调用者**，函数退出时不会销毁它；
- **没被选中的其他局部变量（如 `unused`）**：在 `return` 离开函数体之前，自动执行 `Drop` 清理。

让我们把这些特性整合进 进阶代码中：

```flurry
use std.prelude.*;

struct SlotLease {
    slot_id: u32,
}

impl Drop for SlotLease {
    fn drop(*self) {
        println("Drop slot {}", self.slot_id.*);
    }
}

struct Transmitter {
    seq: u32,

    -- 表达式主体函数 (`= expr;`)：简洁描述直接计算
    fn next_seq_preview(*self, step: u32) -> u32 =
        self.seq.* + step;

    fn update_seq(*self, next: u32) {
        println("Inside update_seq: old seq was {}, setting to {}", self.seq.*, next);
        self.seq.* = next;
    }
}

fn hijack_and_supply(target: *Transmitter) -> u32 {
    println("Inside arg: changing tx.seq to 12 first!");
    target.seq.* = 12u32;
    18u32
}

-- 具名结果 `-> result: u32`：直接供 `where ensures` 契约引用。
fn pick_smaller_id(first: u32, second: u32) -> result: u32
where ensures result <= first and result <= second
{
    return first while first < second;
    second
}

-- 返回并非 Copy 资源时：移走被返回值的责任，自动清理剩余局部值
fn select_active_slot() -> SlotLease {
    let backup = SlotLease { slot_id: 2u32 }
    let primary = SlotLease { slot_id: 3u32 }
    -- 提前返回 primary：primary 责任移交给调用方，backup 在离开函数时立即 Drop。
    primary
}

test {
    -- 1. 验证接收者先取得、且不被独占借用冻结
    let tx = Transmitter { seq: 4u32 }
    tx.update_seq(hijack_and_supply(tx.ref));
    println("Final seq: {}", tx.next_seq_preview(0u32));

    -- 2. 验证具名结果与条件返回
    println("Smaller: {}", pick_smaller_id(9u32, 2u32));

    -- 3. 验证返回值的责任移交与局部剩余清理
    let chosen = select_active_slot();
    println("Caller received slot {}", chosen.slot_id);
}
```

运行这段测试，输出清晰地印证了每一条规则：

```text
Inside arg: changing tx.seq to 12 first!
Inside update_seq: old seq was 12, setting to 18
Final seq: 18
Smaller: 2
Drop slot 2
Caller received slot 3
Drop slot 3
```

看到最后三行了吗？
在 `select_active_slot()` 返回的瞬间，没被选中的 `backup`（槽位 2）立刻在函数出口被 `Drop slot 2` 销毁；而被返回的 `primary`（槽位 3）毫发无损地交到了调用者 `chosen` 手中，直到整个 `test` 块结束才执行 `Drop slot 3`。

### 编译器防呆：试图返回指向局部变量的指针？编译期报错

既然返回值会把结果移交给调用者，而剩余的局部存储会在离开函数时销毁，那么如果有人试图在函数里返回一个指向局部变量的指针或视图呢？

```flurry
-- ❌ 编译期直接报错。
fn dangling_slot_ptr() -> *u32 {
    let local_id = 42u32;
    -- 错误。返回结果无法继续依赖离开函数时即被销毁的局部存储 local_id。
    local_id.ref
}
```

正如规范所言：**“函数可以交出一个值，无法让结果继续依赖已经销毁的局部存储。”** 编译器的可达性（Reachability）与生命周期检查会在编译期直接拦下这种悬垂指针逃逸（我们将在第三卷深入展开它的底层原理）。

---

## 本篇小结

作为第二卷的开篇，我们理清了一次函数调用从“准备”到“返回”的完整生命周期：

| 阶段 / 特性 | Flurry 的设计核心规则 | 解决的核心问题 |
| :--- | :--- | :--- |
| **调用准备次序** | 1. 先求被调方或方法接收者；2. 严格按源码从左到右求值显式实参；3. 补齐缺省值；4. 交付并进入主体 | 告别 C/C++ 实参求值顺序未指明导致的玄学 Bug |
| **未使用参数保证** | 即使函数体未读取某普通参数，其实参表达式也必须在运行时求值一次 | 防止内联或死代码消除悄悄吞掉实参的 I/O、状态修改或清理 |
| **准备失败回滚** | 某实参提前返回或报错时，后续实参跳过求值；已求值的并非 `Copy` 临时实参按逆序立即 `Drop` | 一行写完嵌套调用 `send(read()!, alloc(), read()!)`，没有资源泄漏 |
| **方法接收者适配** | 接收者在显式实参前取得；`*self` 接收普通指针，免于建立冻结整个对象的独占借用锁 | 允许 `obj.method(helper(obj.ref))` 顺畅组合，按实际顺序更新状态 |
| **函数定义与返回** | 支持块主体与 `= expr;` 表达式主体；支持具名结果 `-> result: T` 供 `ensures` 引用；返回移走结果责任并清理剩余局部值 | 接口契约表达清晰，返回值与局部资源析构边界分明 |

### 下一篇预告

在刚才绘制“四步调用准备时间线”时，我们在第 2 步（求值显式实参）和第 4 步（进入函数体）之间，留了一个神秘的 **第 3 步**：
> **“按形参声明依赖次序补齐缺失的 implicit 与 optional 输入。”**

现在，我们的传感器分析仪要把采集到的历史样本存进一个内存缓冲区，并提供一个分页读取接口 `page_bounds`。
调用方提出了一连串要求：
- 有时候我只想传总样本数 `page_bounds(100)`，希望自动从 `0` 开始读完所有样本；
- 有时候我想指定起点 `page_bounds(100, .start = 80)`，希望读取的数量自动变成**剩余样本数 `total - start`**（即 `100 - 80 = 20`）。
- 有时候我甚至想把命名参数的顺序反过来写：`page_bounds(100, .count = 5, .start = 20)`。

如果在 C++、Rust 或 Java 里，你会发现：要么根本没有默认参数（得写三个重载函数或 Builder 模式），要么默认参数是个“瞎子”——**C++ 的默认参数根本禁止读取前一个参数 `total` 和 `start` 的值。**

在下一篇 **《Flurry 演进手记 (022)：让配置接口告别重载膨胀——可选参数 `.id = expr` 与前序参数依赖》** 中，我们将揭秘 Flurry 强大的可选参数系统，看看它是如何让后面的默认值直接站在前面参数的肩膀上计算的。
