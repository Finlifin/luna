# Flurry 演进手记 (076)：在编译期透视任何类型的骨骼——Type.size_of、结构反射与内存布局查询

在上一回（075）中，我们用 `Tuple<...Es: meta.List<Type>>` 为静态形状张量引擎 `Tensor<T, .dims = s>` 构建了维度与通道的编译期类型包，让多路异构张量通道能够放进同一个带有 Normal-Form 头身份的元组容器中。

现在，我们的张量引擎来到了与硬件零拷贝 DMA（Direct Memory Access）控制器的交界处。DMA 控制器可不认识什么高雅的 `Type` 或 `SamplePair`——它只认冷酷的物理事实：
1. 一个数据元素的底层存储究竟占多少字节（`size_of`）？
2. 某个结构体或枚举的底层编码长什么样（`repr_of`）？
3. 当我们为一个张量元素挂上链表指针或动态切片视图时，哪些类型的大小在编译期是确定的，哪些又依赖于运行期信息？

如果像 C/C++ 那样混用宏和手写 `sizeof` 偏移计算，不仅极易在跨平台时踩中对齐（Alignment）与字段重排的暗坑，还会破坏类型身份的边界。今天，我们就来揭开 Flurry **结构反射（Structural Reflection）与内存布局查询（Layout Queries）** 的面纱，看看如何在 `U_1` 编译期宇宙里安全、精确地透视任何类型的骨骼。

---

## 一、 问题背景与设计思路：为什么不能把“类型结构”和“物理字节”混为一谈？

在设计张量 DMA 缓冲描述符时，最容易犯的第一个直觉错误，就是假定“只要我知道一个 `struct` 有哪些字段，我就天然知道它在内存里占几个字节、字段按什么顺序排列”：

```flurry
-- ❌ 传统的天真假设：以为声明顺序就是物理内存顺序，且试图在值上直接调 size_of
struct DmaHeader {
    flags: u8,
    payload_len: u32,
    tag: u8,
}

fn bad_dma_bytes(header: DmaHeader) -> usize {
    -- ❌ 问题 1：如果在值上写 header.size_of()，到底是在调 DmaHeader 的实例方法，
    -- 还是在查类型的大小？语法边界含糊不清。
    -- ❌ 问题 2：直觉以为 1 + 4 + 1 = 6 字节，或者以为 flags 一定排在物理偏移 0，
    -- 殊不知默认布局允许编译器为了对齐重排字段。
    6
}
```

### 1.1 为什么 Flurry 要把反射与布局分成两层观察能力？

在 Flurry 的类型系统中，编译期对一个类型的观察严格区分为两个层次：

```
+-------------------------------------------------------------------------+
|                   Flurry 编译期类型观察的两大层次                         |
+-------------------------------------------------------------------------+
|                                                                         |
|  第 1 层：结构观察 (Structural Observation)                              |
|  • 回答："这个类型由什么组成？身份是谁？"                                 |
|  • 包含：字段列表、枚举分支、变参元素、Type 相等性、Normal-Form 头        |
|  • 依赖：仅依赖类型定义本身，与目标机器（Target）指针字长无关              |
|                                                                         |
|  ------------------------ [ 物理具象化边界 ] ---------------------------  |
|                                                                         |
|  第 2 层：布局观察 (Layout Observation)                                  |
|  • 回答："落到具体硬件内存里占多少字节？怎么编码？"                       |
|  • 包含：Type.size_of(T)、对齐要求、字段物理偏移、Type.repr_of(T)         |
|  • 依赖：要求所有影响布局的参数已具体化，且依赖当前编译目标的布局配置     |
+-------------------------------------------------------------------------+
```

为什么要把这两层拆开？
- 当你写一个泛型元编程函数去遍历结构体字段生成序列化代码时，你只需要**第 1 层（结构观察）**——此时哪怕某些字段的具体目标平台大小还没定下来，结构逻辑依然完全成立。
- 只有当你真正向编译器提问 `Type.size_of(T)` 或 `Type.repr_of(T)` 时，才会跨越边界进入**第 2 层（布局观察）**。而且，除非你显式配置了底层布局属性（我们将在下一篇 `077` 详解），默认结构体布局**不承诺**按源码书写顺序排放字段。

---

## 二、 升级：`Type.size_of`、`Type.repr_of` 与 `value'type` 视图

为了让“类型级查询”与“运行期实例方法调用”在视觉和语义上泾渭分明，Flurry 规定：**布局与反射查询直接作为 `Type` 命名空间上的纯编译期函数来调用**，例如 `Type.size_of(T)` 与 `Type.repr_of(T)`。

那如果我们手里拿到的是一个变量或表达式 `value`，想查它所属类型的大小怎么办？用我们在第 005 回见过的值视图后缀——`value'type`。

让我们写出 版的张量采样对（`SamplePair`）与零开销存储字节查询器：

```flurry
struct SamplePair {
    left: u32,
    right: u32,
}

-- 借助 quote 参数：只观察传入表达式的编译期语法/类型描述，不消费也不搬移运行期值。
pure comptime fn storage_bytes(quote value: Any) -> usize {
    Type.size_of(value'type)
}

test {
    let pair = SamplePair { left: 4, right: 9 }

    -- 1. value'type 取出静态类型，与 SamplePair 完全相等
    assumes pair'type == SamplePair;

    -- 2. 直接通过 Type.size_of 查询类型大小，或通过 quote 函数观察表达式类型的大小
    assumes Type.size_of(SamplePair) == 8;
    assumes storage_bytes(pair) == 8;

    -- 3. Type.repr_of(T) 返回该类型底层物理表示的编译期描述
    assumes Type.repr_of(SamplePair) == Type.repr_of(SamplePair);
}
```

> **🔍 语法小贴士：为什么 `storage_bytes` 要用 `quote value: Any`？**
> 回顾第 072 回，如果参数写成普通运行期参数 `value: T`，调用 `storage_bytes(pair)` 就会涉及到运行期值的传递或所有权移动；而写成 `quote value: Any` 时，`storage_bytes` 是一个 `pure comptime fn`，它在编译期接住 `pair` 的 AST 节点，通过 `value'type` 提取出它的静态类型 `SamplePair`，直接算出 `8usize` 替换在调用点，运行期连半条指令都不会产生。

---

## 三、 指针、递归结构与 `is_not_sized` 的真相

在构建张量计算图（Computation Graph）或链式 DMA 描述符（Scatter-Gather List）时，我们经常会写出包含自引用的结构体。这时候，初学者最容易对“什么结构能算出 `size_of`”产生困惑。

### 3.1 指针截断 vs 值内联死循环

为什么链表节点 `struct Link` 可以顺利算出 `Type.size_of(Link)`，而把指针去掉的 `struct InvalidLink` 却会被编译器当场拦下？

让我们画一张内存骨骼透视图：

```
  [ 合法：通过指针间接引用 (struct Link) ]
  +------------------+------------------+
  | value: u32 (4B)  | next: ?*Link (8B)| ---> 指针本身大小固定等于 usize。
  +------------------+------------------+      不需要展开目标 Link 的大小即可封顶。

  [ 非法：按值直接内联自引用 (struct InvalidLink) ]
  +-------------------------------------+
  | next: InvalidLink                   |
  |   +---------------------------------+
  |   | next: InvalidLink               | ---> 俄罗斯套娃无穷无尽。
  |   |   +-----------------------------+      物理布局方程 Size(X) = Size(X) + ...
  |   |   | ...                         |      无有界解，编译期直接判定为非法布局环。
```

看代码中的正反对比：

```flurry
struct Link {
    value: u32,
    next: ?*Link,
}

test {
    -- 指针的物理大小只取决于目标平台的地址宽度（与 usize 相同），不递归展开被指向类型
    assumes Type.size_of(*SamplePair) == Type.size_of(usize);
    -- 因此包含 ?*Link 的结构体拥有完全确定的编译期大小
    assumes Type.size_of(Link) >= Type.size_of(u32);
}
```

如果有人不小心漏写了 `*`，试图按值内联自身：

```flurry
-- ❌ 编译期直接拒绝：按值递归包含自身，形成无法收敛的非法物理布局环（Layout Cycle）
struct InvalidLink {
    next: InvalidLink,
}
```

### 3.2 `Type.predicates.is_not_sized(T)` 到底是什么意思？

在查阅 `Type.predicates` 时，你会看到一个谓词叫 `Type.predicates.is_not_sized(T)`。千万不要按字面把它误解成“这个类型坏掉了”或者“编译器还没来得及算它的大小”。

> **💡 核心心智模型：`is_not_sized` = 大小依赖运行期信息**
> 在 Flurry 中，`Type.predicates.is_not_sized(T)` 的精确语义是：**该类型的完整物理大小取决于运行期值信息（Runtime Value Information），因此不存在单一的、与运行期实例无关的编译期常数大小**（例如长度由运行期变量决定的动态视图类型）。
> - 对于 `SamplePair`、`u32`、`*SamplePair`、`Link` 这类编译期大小固定的类型，`Type.predicates.is_not_sized(T)` 为 `false`。
> - 对于非法递归的 `InvalidLink`，它根本不是合法的类型定义，直接报错拒绝，而不是返回 `is_not_sized == true`。

```flurry
test {
    assumes not Type.predicates.is_not_sized(SamplePair);
    assumes not Type.predicates.is_not_sized(*SamplePair);
}
```

---

## 四、 进阶：名义类型身份、`typealias`、`newtype` 与纯类型工厂

当我们在张量引擎里用反射检查两个通道的类型是否相同时，必须搞清楚：**在 Flurry 的 `Type` 宇宙里，什么时候 `A == B` 为真？**

Flurry 对 `struct` / `enum` / `union` 采用严格的**名义类型身份（Nominal Type Identity）**，同时提供了三种截然不同的类型复用与构造手段：

1. **`typealias`（透明别名）**：不创建新身份，也不改变布局。`CustomerId == UserId` 为 `true`。
2. **`newtype`（独立身份包装）**：复用底层物理表示（`Type.size_of` 与 `Type.repr_of` 保持一致），但**创立全新的类型身份**。`ArchivedUserId == UserId` 为 `false`。
3. **`pure comptime fn` 类型工厂**：由于纯编译期函数具有确定性记忆化（Memoization），用**相同输入**调用同一个纯类型工厂，得到的 `Type` 具有**完全相同的类型身份**；用不同输入调用则产生不同类型。

下面我们把这三条核心规则放在一起验证：

```flurry
typealias UserId = u64;
typealias CustomerId = UserId;
newtype ArchivedUserId = UserId;

-- 即使两个 struct 的字段名和字段类型一模一样，每一次独立声明都会诞生新的名义身份
struct LeftRecord {
    id: UserId,
}

struct RightRecord {
    id: UserId,
}

-- 纯编译期类型工厂：相同输入必然复用同一个类型身份
pure comptime fn PairOf(T: Type) -> Type {
    struct Pair {
        first: T,
        second: T,
    }
}

test {
    -- 1. typealias 完全透明
    assumes UserId == u64;
    assumes CustomerId == UserId;

    -- 2. newtype 物理大小相同，但类型身份独立。
    assumes ArchivedUserId != UserId;
    assumes Type.size_of(ArchivedUserId) == Type.size_of(UserId);

    -- 3. 独立声明的同名字段 struct 身份不同，哪怕物理 repr 相同
    assumes LeftRecord != RightRecord;
    assumes Type.repr_of(LeftRecord) == Type.repr_of(RightRecord);

    -- 4. 纯类型工厂：同参同身份，异参异身份
    assumes PairOf(u32) == PairOf(u32);
    assumes PairOf(u32) != PairOf(u64);
    assumes Type.size_of(PairOf(u32)) == 8;
}
```

这套规则让我们在写张量 DMA 校验器时游刃有余：如果两个张量元素类型是 `ArchivedUserId` 和 `UserId`，类型检查器会阻止它们在业务层混用（`ArchivedUserId != UserId`），但底层 DMA 搬运引擎却可以通过 `Type.repr_of(A) == Type.repr_of(B)` 和 `Type.size_of(A) == Type.size_of(B)` 确认它们具有相同的 8 字节物理骨骼。

---

## 五、 本篇小结

| 观察工具 / 构造 | 所属层次 | 核心语义与契约 | 典型示例 |
| :--- | :--- | :--- | :--- |
| `value'type` | 结构观察 | 提取表达式或绑定的编译期静态类型，不消费运行期值 | `pair'type == SamplePair` |
| `Type.size_of(T)` | 布局观察 | 查询类型 `T` 在当前目标下的字节大小（要求 `T` 大小编译期确定） | `Type.size_of(SamplePair) == 8` |
| `Type.repr_of(T)` | 布局观察 | 查询类型 `T` 的底层物理表示描述 | `Type.repr_of(LeftRecord)` |
| `Type.predicates.is_not_sized(T)` | 布局观察 | 判断 `T` 的物理大小是否依赖运行期值信息（而非编译期常数） | `not Type.predicates.is_not_sized(u32)` |
| `typealias A = B` | 类型身份 | 透明别名，`A == B` 且共享一切结构与布局 | `CustomerId == UserId` |
| `newtype A = B` | 类型身份 | 创立新类型身份（`A != B`），但保留 `B` 的底层物理表示与大小 | `ArchivedUserId != UserId` |
| `pure comptime fn F(T: Type) -> Type` | 类型身份 | 纯类型工厂对相同输入返回同一类型身份，对不同输入返回不同身份 | `PairOf(u32) == PairOf(u32)` |

---

## 六、 下一篇预告

既然默认情况下，编译器为了对齐性能**不承诺** `struct` 字段按源码顺序排列，也不承诺 `enum` 一定用 `u8` 作为判别子（Tag），那么当我们的张量 DMA 描述符必须严丝合缝地贴合硬件寄存器手册（要求紧凑无填充 `packed`、指定底层判别子类型 `base_type u8`、或者指定 C ABI 调用约定）时，该怎么告诉编译器？

更进一步，在其他语言里，属性（Attribute，如 Rust 的 `#[repr(C)]` 或 C++ 的 `[[...]]`）往往是一套游离在类型系统之外的“特权宏语法”，既不能赋给变量，也很难用普通函数组合。如果**属性本身就是正经的编译期 `Object` 值**呢？

下一回 **《Flurry 演进手记 (077)：元数据也是正经的编译期对象——^expr 属性系统、.id expr 配置与 attrs 包》**，我们将用统一的 `^expr` 与 `.id expr` 为 DMA 硬件描述符定制精确到比特的元数据。
