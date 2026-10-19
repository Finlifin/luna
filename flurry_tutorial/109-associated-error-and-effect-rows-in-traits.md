# Flurry 演进手记 (109)：在 Trait 方法里优雅地处理错误与效应——关联错误行与效应行多态

在上一篇末尾，我们遇到了设计通用编解码器框架（以及一切流式 I/O 接口）时最经典的一个架构难题：
当我们定义一个底层字节写入器接口 `ByteWriter` 时，不同的实现者有着截然不同的失败模式与执行模型：
- **定长栈缓冲区 `FixedSliceWriter`**：当空间不够时会返回 `![BufferFullError]` 错误，但全程在内存中同步执行，没有任何异步或日志效应；
- **网络套接字 `TcpSocketWriter`**：写入时可能返回 `![IoError]` 错误，同时每次写操作都需要触发异步挂起效应 `#[async_wait]`；
- **计数虚拟写入器 `SizeCounterWriter`**：只统计字节数，既**不会返回任何错误（空错误行 `![]`）**，也**不触发任何代数效应（空效应行 `#[]`）**！

在传统语言里：
- 如果你在 Trait 方法里把返回类型写死成 `Result<usize, IoError>`，那么连永远不可能失败的 `SizeCounterWriter` 都被迫包装一层 `Ok(...)`，调用者也被迫写无意义的 `unwrap()`；
- 更要命的是，在 C++ / Rust / Java 里，**同步接口（Sync）与异步/效应接口（Async / Effectful）是彻底割裂的两套平行世界**（著名的“函数染色问题 Function Coloring”），你不得不维护 `Write` 和 `AsyncWrite` 两套几乎一模一样的 Trait！

而在 Flurry 中，得益于一等公民的**错误行（Error Row，`!Err`）**与**效应行（Effect Row，`#Eff`）**，再配合 Trait 的 **`assoc` 关联项与契约包含规则**，我们可以用**同一个 Trait** 统合同步、异步、无错与多分支错误！

今天，我们就来深入探索错误行与效应行的代数法则，以及如何在 Trait 中驾驭它们！

---

## 尝试 v0：把错误和效应写死在 Trait 签名里的“一刀切”之痛

假设我们在设计 `ByteWriter` 的第 0 版时，把错误类型和效应直接硬编码在了 Trait 方法签名上：

```flurry
enum IoError {
    disconnected,
    timeout,
}

effect async_yield() -> void;

-- ❌ v0 的僵化设计：强行把 ![IoError] 和 #[async_yield] 焊死在接口上！
trait RigidWriter {
    fn write_byte(*self, byte: u8) -> #[async_yield] ![IoError] void;
}
```

这个设计有多难受？
1. 当你为内存数组实现 `RigidWriter` 时，你明明只可能报 `BufferOverflow`，却被迫伪装成毫不相干的 `IoError.disconnected`；
2. 当你在一段纯粹同步、不允许任何代数效应的内核中断处理函数里使用内存 `RigidWriter` 时，编译器看到签名上有 `#[async_yield]`，直接禁止你在无效应上下文里调用它！

要打破这个僵局，我们首先得看清 Flurry 中 **`!expr`（错误行）** 与 **`#expr`（效应行）** 的真实代数脾气！

---

## 演进 v1：复习与进阶——错误行（唯一分支去重）vs 效应行（保留处理层次）

在第二卷中我们初步接触过 `!` 错误与 `#` 效应，而在 `docs/type/rows.md` 中，Flurry 正式定义了它们作为**行类型（Row Types）**的核心代数差异：

```
+-------------------------------------------------------------------------+
|  🎨 Code Cartoon：错误行 (!Row) 与 效应行 (#Row) 的根本代数差异          |
|                                                                         |
|  1. 【错误行 !Err —— 有限唯一标签行（Unique-Label Row）】               |
|     • 错误描述的是“返回值可能落入哪个错误结果分支（ABI 结果判别）”。    |
|     • 连续两次可能抛出同一种 `InputError`，最终结果依然只有那一个       |
|       `InputError` 分支！                                               |
|     • 代数归一律（自动合并去重）：                                      |
|       `![InputError] ![InputError] u32`  ===  `![InputError] u32` ✅    |
|     • 空限定消去律：                                                    |
|       `![] u32`  ===  `u32` ✅                                          |
|                                                                         |
|  2. 【效应行 #Eff —— 保留同名层次行（Layered-Occurrence Row）】         |
|     • 效应描述的是“计算过程中向哪一层外围处理器（Handler）发起请求”。   |
|     • 同一个函数内连续调用两次 `read_number()#`，共享同一个内层处理器， |
|       因此效应行依然是单层 `#[read_number]`。                           |
|     • 但如果内层 Handler 在处理 `read_number` 时，又向外层转发了一次    |
|       `read_number()#`，这就代表两层嵌套的 Handler 路由！               |
|     • 代数保持律（同名层次绝不盲目去重！）：                            |
|       `#[read_number] #[read_number] u32`  !==  `#[read_number] u32` ❗ |
|     • 空限定消去律：                                                    |
|       `#[] u32`  ===  `u32` ✅                                          |
+-------------------------------------------------------------------------+
```

我们可以直接用一段 `comptime` 断言代码（来自 `docs/type/rows.md`）来验证这条铁律：

```flurry
enum InputError {
    out_of_range(u32),
}

effect read_number() -> u32;

typealias OneError = ![InputError] u32;
typealias RepeatedError = ![InputError] ![InputError] u32;

typealias OneLayer = #[read_number] u32;
typealias TwoLayers = #[read_number] #[read_number] u32;

comptime {
    -- ✅ 错误行自动去重合并同一错误身份！
    assumes OneError == RepeatedError;

    -- ✅ 效应行严格保留同名效应的处理层次，绝不盲目合并！
    assumes OneLayer != TwoLayers;
}
```

而且，当我们在一个函数里连续调用两次返回 `![InputError] u32` 的辅助函数并用 `!` 传播错误，或者连续发起两次 `read_number()#` 效应请求时，返回类型都不需要按调用次数膨胀：

```flurry
fn checked_byte(value: u32) -> ![InputError] u32 {
    if value <= 255 {
        value
    } else {
        error InputError.out_of_range(value)
    }
}

-- 连续两次 `!` 传播同一种 InputError，返回类型依然是干净的 ![InputError] Tuple<u32, u32>
fn byte_pair(left: u32, right: u32) -> ![InputError] Tuple<u32, u32> {
    let first = checked_byte(left)!;
    let second = checked_byte(right)!;
    (first, second)
}

-- 连续两次 `#` 调用同一效应，共享同一层处理器入口，返回类型依然是 #[read_number] Tuple<u32, u32>
fn number_pair() -> #[read_number] Tuple<u32, u32> {
    let first = read_number()#;
    let second = read_number()#;
    (first, second)
}
```

---

## 演进 v2：在 Trait 中使用 `assoc` 关联错误与效应，实现多态 I/O 契约！

既然 `![] T == T` 且 `#[] T == T`，而且 `assoc` 关联项可以声明类型或行并附带默认值（`assoc id: expr = default`），我们就可以在定义 `ByteWriter` Trait 时，把具体的错误与效应交给关联项来表达！

让我们写出支持错误与效应多态的 `ByteWriter`：

```flurry
enum SliceWriteError {
    buffer_full(usize),
}

enum NoError {}

trait ByteWriter {
    -- 关联错误类型（默认提供空错误/无错类型，实现者可按需替换为自己的错误）
    assoc WriteError: Type = NoError;

    fn write_u8(*self, byte: u8) -> ![WriteError] void;

    -- 默认方法：写入一个 u16（小端序），直接复用 write_u8 的关联错误行！
    -- 因为两次调用的错误都是 ![WriteError]，错误行自动合并去重为单个 ![WriteError]！
    fn write_u16_le(*self, val: u16) -> ![WriteError] void {
        let low = (val & 0xFFu16).as(u8);
        let high = ((val >> 8u16) & 0xFFu16).as(u8);
        self.write_u8(low)!;
        self.write_u8(high)!;
    }
}
```

现在看看不同的写入器实现 `ByteWriter` 时有多么自在：

```flurry
-- 实现 1：只统计字节数的虚拟写入器（使用默认的 NoError）
struct ByteCounter {
    count: usize,
}

impl ByteWriter for ByteCounter {
    fn write_u8(*self, byte: u8) -> ![NoError] void {
        self.count.* = self.count.* + 1usize;
    }
}

-- 实现 2：有界缓冲区写入器（声明专属的 SliceWriteError）
struct BoundedBuffer {
    len: usize,
    cap: usize,
}

impl ByteWriter for BoundedBuffer {
    assoc WriteError: Type = SliceWriteError;

    fn write_u8(*self, byte: u8) -> ![SliceWriteError] void {
        if self.len.* < self.cap.* {
            self.len.* = self.len.* + 1usize;
        } else {
            error SliceWriteError.buffer_full(self.cap.*)
        }
    }
}
```

### 编译期安全红线：实现方法绝不能“超标”抛出接口未允许的错误或效应！

在 `docs/traits/declarations.md` 中，Flurry 对 Trait 实现契约规定了一条铁律：
> **实现方法的参数、接收者、返回值及阶段必须兑现接口。前提只能在合法替换关系下减弱，结果保证只能保持或加强；错误、效应和环境需求不能超出接口允许的范围！**

这意味着：如果 Trait 的方法签名没有声明某个错误（或者关联错误是 `SliceWriteError`），你在 `impl` 的方法体里却偷偷抛出了另一个接口契约之外的 `IoError`，或者偷偷触发了一个接口签名未声明的未处理效应 `#[read_number]`，编译器会在检查该 `impl` 时**当场拦截**！

```flurry
effect secret_io() -> void;

struct SneakyWriter {}

impl ByteWriter for SneakyWriter {
    assoc WriteError: Type = SliceWriteError;

    -- ❌ 编译期直接拒绝！
    -- ByteWriter::write_u8 的接口契约没有允许未处理的 `#[secret_io]` 效应，
    -- 实现方法的错误、效应和环境需求绝不能超出接口允许的范围！
    fn write_u8(*self, byte: u8) -> #[secret_io] ![SliceWriteError] void {
        secret_io()#;
    }
}
```

> **💡 核心心智模型（来自 `AGENTS.md` §5）：错误结果与代数效应具有不同 ABI，函数签名不加 `handles`！**
> 请时刻牢记：
> 1. **`!Err`（错误行）** 是结果值分支的一部分（类似零开销可判别返回联合），用后缀 `!` 传播；
> 2. **`#Eff`（效应行）** 是与外层处理器（Handler）交互的上下文控制流协议，用后缀 `#` 调用或挂接 `{ op() => ... }` 处理器块；函数签名直接使用 `#[Eff] ReturnType`，**绝不引入所谓的 `handles` 关键字**！

---

## 本篇小结

| 维度 | 错误行（`!Err`） | 效应行（`#Eff`） |
| :--- | :--- | :--- |
| **语义本质** | 返回结果的错误分支集合（唯一标签行） | 计算向上请求外层 Handler 的操作层次序列 |
| **重复标签规则** | **自动去重合并**：`![E] ![E] T == ![E] T` | **严格保留层次**：`#[Op] #[Op] T != #[Op] T`（区分内外层嵌套 Handler 路由） |
| **传播与处理语法** | `expr!` 向上传播，`match` 中 `error .variant => ...` 分支处理 | `expr#` 触发请求，`expr# { op() => ... }` 就地提供 Handler 处理消去一层 |
| **Trait 契约约束** | 可通过 `assoc` 关联类型/行定制；`impl` 方法抛出的错误不得超出接口允许范围 | 可在泛型与接口中行多态化；`impl` 方法产生的未处理效应不得超出接口允许范围 |

---

### 留给下一篇的悬念

万事俱备！我们已经集齐了：
- 静态与动态接口（`trait` / `dyn Trait`）；
- 编译期常量与两相机制（`assoc` / `comptime fn`）；
- 关联错误行与效应行（`![WriteError]`）。

从下一篇开始，我们将进入第五卷最激动人心的**三部曲实战（Episodes 110–112）**：从零设计并实现一套工业级的**零拷贝通用序列化与反序列化框架 `Encode` / `Decode`**！
在第一部曲 **`110-zero-copy-codec-trait-part1-interface.md`** 中，我们将首先攻克接口设计：如何让 `Encode` 和 `Decode` 既能以零内存拷贝处理定宽传感器报文，又能流式处理变长数据包？我们马上揭晓！
