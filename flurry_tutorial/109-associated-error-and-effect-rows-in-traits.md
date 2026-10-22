# Flurry 演进手记 (109)：在 Trait 方法里优雅地处理错误与效应——关联错误行与效应行多态

在上一篇末尾，我们遇到了设计通用编解码器框架（以及流式 I/O 接口）时常见的一个架构问题：
当我们定义一个底层字节写入器接口 `ByteWriter` 时，不同的实现者有着不同的失败模式与执行模型：
- **定长栈缓冲区 `FixedSliceWriter`**：当空间欠缺时会返回 `![BufferFullErr]` 错误，但全程在内存中同步执行，没有任何异步或日志效应；
- **网络套接字 `TcpSocketWriter`**：写入时可能返回 `![IoErr]` 错误，同时每次写操作都需要触发异步挂起效应 `#[async_wait]`；
- **计数虚拟写入器 `SizeCounterWriter`**：只统计字节数，既**不会返回任何错误（空错误行 `![]`）**，也**免于触发任何代数效应（空效应行 `#[]`）**。

如果把返回类型固定为单一错误枚举，或者把同步接口与效应接口拆成两套独立的 Trait，就会增加大量重复定义。
而在 Flurry 中，借助一等公民的**错误行（Error Row，`!Err`）**与**效应行（Effect Row，`#Eff`）**，再配合 Trait 的 **`assoc` 关联项与契约包含规则**，我们可以用**同一个 Trait** 统合同步、异步、无错与多分支错误。

今天，我们就来探索错误行与效应行的代数法则，以及如何在 Trait 中使用它们。

---

## 把错误和效应写死在 Trait 签名里的局限

假设我们在设计 `ByteWriter` 的初始版本时，把错误类型和效应直接固定在了 Trait 方法签名上：

```flurry
enum IoErr {
    Disconnected,
    Timeout,
}

effect async_yield() -> void;

-- 局限设计：把 ![IoErr] 和 #[async_yield] 固定在接口上
trait RigidWriter {
    fn write_byte(*self, byte: u8) -> #[async_yield] ![IoErr] void;
}
```

这个设计存在两个明显限制：
1. 当你为内存数组实现 `RigidWriter` 时，即使只可能发生缓冲区满，也严禁不映射到 `IoErr`；
2. 当你在一段纯粹同步、禁止任何代数效应的函数里使用内存 `RigidWriter` 时，签名上的 `#[async_yield]` 会阻止在无效应上下文中直接调用它。

要解决这一问题，我们首先来看 Flurry 中 **`!expr`（错误行）** 与 **`#expr`（效应行）** 的代数性质。

---

## 错误行（唯一分支去重）vs 效应行（保留处理层次）

在第二卷中我们初步接触过 `!` 错误与 `#` 效应，而在 `docs/type/rows.md` 中，Flurry 正式定义了它们作为**行类型（Row Types）**的核心代数差异：

```
+-------------------------------------------------------------------------+
|  错误行 (!Row) 与 效应行 (#Row) 的代数差异                              |
|                                                                         |
|  1. 【错误行 !Err —— 有限唯一标签行（Unique-Label Row）】               |
|     • 错误描述的是“返回值可能落入哪个错误结果分支（ABI 结果判别）”。    |
|     • 连续两次可能抛出同一种 `InputErr`，最终结果依然只有那一个         |
|       `InputErr` 分支。                                                 |
|     • 代数归一律（自动合并去重）：                                      |
|       `![InputErr] ![InputErr] u32`  ===  `![InputErr] u32` ✅          |
|     • 空限定消去律：                                                    |
|       `![] u32`  ===  `u32` ✅                                          |
|                                                                         |
|  2. 【效应行 #Eff —— 保留同名层次行（Layered-Occurrence Row）】         |
|     • 效应描述的是“计算过程中向哪一层外围处理器（Handler）发起请求”。   |
|     • 同一个函数内连续调用两次 `read_number()#`，共享同一个内层处理器， |
|       因此效应行依然是单层 `#[read_number]`。                           |
|     • 但如果内层 Handler 在处理 `read_number` 时，又向外层转发了一次    |
|       `read_number()#`，这就代表两层嵌套的 Handler 路由。               |
|     • 代数保持律（同名层次不盲目去重）：                                |
|       `#[read_number] #[read_number] u32`  !==  `#[read_number] u32` ❗ |
|     • 空限定消去律：                                                    |
|       `#[] u32`  ===  `u32` ✅                                          |
+-------------------------------------------------------------------------+
```

我们可以直接用一段 `comptime` 断言代码（参考 `docs/type/rows.md`）来验证这条规则：

```flurry
enum InputErr {
    OutOfRange(u32),
}

effect read_number() -> u32;

typealias OneError = ![InputErr] u32;
typealias RepeatedError = ![InputErr] ![InputErr] u32;

typealias OneLayer = #[read_number] u32;
typealias TwoLayers = #[read_number] #[read_number] u32;

comptime {
    -- ✅ 错误行自动去重合并同一错误身份
    assumes OneError == RepeatedError;

    -- ✅ 效应行严格保留同名效应的处理层次，不盲目合并
    assumes OneLayer != TwoLayers;
}
```

而且，当我们在一个函数里连续调用两次返回 `![InputErr] u32` 的辅助函数并用 `!` 传播错误，或者连续发起两次 `read_number()#` 效应请求时，返回类型都无需按调用次数膨胀：

```flurry
fn checked_byte(value: u32) -> ![InputErr] u32 {
    if value <= 255 {
        value
    } else {
        error InputErr.OutOfRange(value)
    }
}

-- 连续两次 `!` 传播同一种 InputErr，返回类型依然是干净的 ![InputErr] Tuple<u32, u32>
fn byte_pair(left: u32, right: u32) -> ![InputErr] Tuple<u32, u32> {
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

## 在 Trait 中使用 `assoc` 关联错误与效应，实现多态 I/O 契约

既然 `![] T == T` 且 `#[] T == T`，而且 `assoc` 关联项可以声明类型或行并附带默认值（`assoc id: expr = default`），我们就可以在定义 `ByteWriter` Trait 时，把具体的错误与效应交给关联项来表达。

让我们写出支持错误与效应多态的 `ByteWriter`：

```flurry
enum SliceWriteErr {
    BufferFull(usize),
}

enum NoErr {}

trait ByteWriter {
    -- 关联错误类型（默认提供空错误/无错类型，实现者可按需替换为自己的错误）
    assoc WriteErr: Type = NoErr;

    fn write_u8(*self, byte: u8) -> ![WriteErr] void;

    -- 默认方法：写入一个 u16（小端序），直接复用 write_u8 的关联错误行
    -- 因为两次调用的错误都是 ![WriteErr]，错误行自动合并去重为单个 ![WriteErr]
    fn write_u16_le(*self, val: u16) -> ![WriteErr] void {
        let low = (val % 256u16).as(u8);
        let high = (val / 256u16).as(u8);
        self.write_u8(low)!;
        self.write_u8(high)!;
    }
}
```

现在看看不同的写入器如何实现 `ByteWriter`：

```flurry
-- 实现 1：只统计字节数的虚拟写入器（使用默认的 NoErr）
struct ByteCounter {
    count: usize,
}

impl ByteWriter for ByteCounter {
    fn write_u8(*self, byte: u8) -> ![NoErr] void {
        self.count.* = self.count.* + 1usize;
    }
}

-- 实现 2：有界缓冲区写入器（声明专属的 SliceWriteErr）
struct BoundedBuffer {
    len: usize,
    cap: usize,
}

impl ByteWriter for BoundedBuffer {
    assoc WriteErr: Type = SliceWriteErr;

    fn write_u8(*self, byte: u8) -> ![SliceWriteErr] void {
        if self.len.* < self.cap.* {
            self.len.* = self.len.* + 1usize;
        } else {
            error SliceWriteErr.BufferFull(self.cap.*)
        }
    }
}
```

### 编译期安全约束：实现方法无法抛出接口未允许的错误或效应

在 `docs/traits/declarations.md` 中，Flurry 对 Trait 实现契约规定：
> **实现方法的参数、接收者、返回值及阶段必须兑现接口。前提只能在合法替换关系下减弱，结果保证只能保持或加强；错误、效应和环境需求无法超出接口允许的范围。**

这意味着：如果 Trait 的方法签名没有声明某个错误（或者关联错误是 `SliceWriteErr`），你在 `impl` 的方法体里却抛出了另一个接口契约之外的 `IoErr`，或者触发了一个接口签名未声明的未处理效应 `#[read_number]`，编译器会在检查该 `impl` 时直接报错：

```flurry
effect secret_io() -> void;

struct SneakyWriter {}

impl ByteWriter for SneakyWriter {
    assoc WriteErr: Type = SliceWriteErr;

    -- ❌ 编译期报错：
    -- ByteWriter.write_u8 的接口契约没有允许未处理的 `#[secret_io]` 效应，
    -- 实现方法的错误、效应和环境需求无法超出接口允许的范围
    fn write_u8(*self, byte: u8) -> #[secret_io] ![SliceWriteErr] void {
        secret_io()#;
    }
}
```

> **💡 核心心智模型：错误结果与代数效应具有不同 ABI，函数签名省略 `handles`**
> 请注意：
> 1. **`!Err`（错误行）** 是结果值分支的一部分，用后缀 `!` 传播；
> 2. **`#Eff`（效应行）** 是与外层处理器（Handler）交互的上下文控制流协议，用后缀 `#` 调用或挂接 `{ op() => ... }` 处理器块；函数签名直接使用 `#[Eff] ReturnType`，**免于引入 `handles` 关键字**。

---

## 本篇小结

| 维度 | 错误行（`!Err`） | 效应行（`#Eff`） |
| :--- | :--- | :--- |
| **语义本质** | 返回结果的错误分支集合（唯一标签行） | 计算向上请求外层 Handler 的操作层次序列 |
| **重复标签规则** | **自动去重合并**：`![E] ![E] T == ![E] T` | **严格保留层次**：`#[Op] #[Op] T != #[Op] T`（区分内外层嵌套 Handler 路由） |
| **传播与处理语法** | `expr!` 向上传播，`match` 中 `error .Variant => ...` 分支处理 | `expr#` 触发请求，`expr# { op() => ... }` 就地提供 Handler 处理消去一层 |
| **Trait 契约约束** | 可通过 `assoc` 关联类型/行定制；`impl` 方法抛出的错误严禁超出接口允许范围 | 可在泛型与接口中行多态化；`impl` 方法产生的未处理效应严禁超出接口允许范围 |

---

### 下一篇预告

我们已经介绍了：
- 静态与动态接口（`trait` / `dyn Trait`）；
- 编译期常量与两相机制（`assoc` / `comptime fn`）；
- 关联错误行与效应行（`![WriteErr]`）。

从下一篇开始，我们将进入第五卷的**三部曲实战（Episodes 110–112）**：设计并实现一套**零拷贝通用序列化与反序列化框架 `Encode` / `Decode`**。
在第一部曲 **`110-zero-copy-codec-trait-part1-interface.md`** 中，我们将首先展开接口设计：如何让 `Encode` 和 `Decode` 既能以零内存拷贝处理定宽传感器报文，又能流式处理变长数据包。
