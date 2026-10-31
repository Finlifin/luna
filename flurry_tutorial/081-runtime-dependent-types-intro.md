# Flurry 演进手记 (081)：当数组长度来自运行时变量——运行时依赖类型（Runtime Dependent Types）入门

在上一回（080）中，我们用 `refines { ... }` 与 `requires / ensures` 为编译期类型和常数筑起了严密的逻辑防线。但当时我们也抛出了一个富有颠覆性的工程问题：

在真实的高性能张量引擎、图计算（Graph Computing）或网络协议解析器里，很多关键参数只有在**运行期（Runtime）**才能确定：
- 从模型文件里动态读出的维度长度 `n: usize`，能否直接放进 `Array<f32, n>` 里作为它的类型参数，从而让 `Array<T, n>` 能够统一表达静态定长数组与运行时长度依赖的数组？
- 在使用 SOA（Struct of Arrays，数组结构体）布局管理图节点池或 DMA 缓冲池 `NodeStorage` 时，我们用轻量的 `u32` 下标代替笨重的裸指针。但怎么在编译期从类型上避免**“张冠李戴——拿第 1 个存储池 `first_storage` 的节点下标，去访问第 2 个存储池 `second_storage`”**的恶性 Bug？

欢迎来到 Flurry 类型系统中最令人惊叹的篇章之一：**运行时依赖类型（Runtime Dependent Types）**。今天，我们就来看看一个类型是如何在零额外内存体积膨胀的前提下，“记住”它所归属的运行期环境的。

---

## 一、 问题背景与设计思路：SOA 句柄（Handle / Index）的“张冠李戴”惨案

在编写高性能计算图、游戏引擎 ECS 或编译器 AST 时，为了高度的缓存局部性（Cache Locality）和避免复杂的指针生命周期，工程师们普遍采用 **SOA + 整数句柄（`u32` Index）** 的模式。

在仅使用普通整数作为句柄的方案中，存在一个常见的工程隐患：

```flurry
-- 传统整数句柄问题背景：所有存储池的下标都是同一种类型 NodeId
newtype NodeId = u32;

struct BadStorage {
    values: Array<i32, 3>,
}

-- 问题现场：把属于 graph_a 的节点句柄传给了 graph_b，类型检查器完全放行。
-- 轻则读出静默脏数据，重则触发诡异的数组越界崩溃。
```

有人可能会说：“那我在每个 `NodeId` 结构体里面额外存一个指向所属 `NodeStorage` 的指针 `*NodeStorage` 不就行了？”
**行之难通。** 在运行期检查指针相等非但白白消耗 CPU 分支预测，更重要的是：把一个 8 字节的指针塞进原本只有 4 字节的 `u32` 句柄里，加上对齐填充会让每个句柄瞬间膨胀到 **16 字节**（整整大出 4 倍）。我们之所以用 SOA 和 `u32` 下标，本来就是为了省内存和装进 SIMD 寄存器，怎么能为了防呆让体积暴涨 4 倍？

那么，能否**让 `storage` 只活在 `NodeIndex` 的“类型与操作环境”里，而零占用 `NodeIndex` 实例本身的哪怕 1 个字节内存**？

在 Flurry 中，这正是运行时依赖类型的拿手好戏。

---

## 二、 核心心智模型：“值体（Value Body）”与“类型环境（Environment）”的物理分离

在 Flurry 中，当一个结构体通过 `where quote storage: lift *NodeStorage` 声明它依赖某个运行期对象身份时，编译器会将该类型的信息一分为二：

```
+-------------------------------------------------------------------------+
|            NodeIndex<storage.ref> 的物理存储 vs 类型环境                  |
+-------------------------------------------------------------------------+
|                                                                         |
|  1. 运行期值体 (Runtime Value Body) —— 真正躺在内存 / 寄存器里的字节：    |
|     +-------------------+                                               |
|     | index: u32 (4 字节)|  <-- 只有这一个字段  不会内联重复存储指针        |
|     +-------------------+                                               |
|                                                                         |
|  2. 类型与操作环境 (Type & Operation Environment) —— 编译期追踪的依附契约：|
|     • 静态记录：该实例的类型依赖于稳定运行期身份 `storage.ref`            |
|     • 当调用 `node.value()` 或 `node.left()` 时，编译器自动从外层环境      |
|       把对应的 `storage` 引用接进来，且返回值 `?Self` 自动继承同一环境     |
+-------------------------------------------------------------------------+
```

请仔细品味这张图。
1. **`NodeIndex` 在内存里有多大？** 只有 `index: u32` 的 4 字节。
2. **为什么 `where quote storage: lift *NodeStorage` 要写 `quote` 和 `lift`？**
   - `lift *NodeStorage`：将运行期指针类型 `*NodeStorage` 提升到编译期类型描述层，说明这个依赖项在运行期是一个 `*NodeStorage` 指针；
   - `quote storage`：表明类型参数捕获的是传入实参表达式（如 `first_storage.ref`）所锚定的**运行期稳定身份描述**，而非在类型声明点立刻去解引用读取运行期内存。

---

## 三、 实战演练：编写零体积膨胀、防跨池混用的 `NodeIndex`

让我们直接看这段来自 Flurry 权威规范的 SOA 二叉树节点句柄实现。注意观察 `NodeStorage` 和 `NodeIndex` 是如何配合的：

```flurry
-- 底层 SOA 存储池：用三个定长数组分别存放节点值、左孩子下标和右孩子下标
struct NodeStorage {
    values: Array<i32, 3>,
    left: Array<?u32, 3>,
    right: Array<?u32, 3>,
}

-- 运行时依赖类型：NodeIndex 的类型身份依赖于具体的运行期存储池指针 `storage`。
struct NodeIndex
where
    quote storage: lift *NodeStorage
{
    index: u32,

    -- 直接利用环境中的 storage 读取当前节点的值（集中在 unsafe 方法中检查边界并访问存储）
    pub unsafe fn value(*self) -> ?i32 {
        let idx = self.index.*;
        if idx < 3 {
            storage.*.values[idx.as(usize)]
        } else {
            null
        }
    }

    -- 返回左孩子：注意返回类型 `?Self` 自动保持为 `?NodeIndex<storage>`。
    pub unsafe fn left(*self) -> ?Self {
        let idx = self.index.*;
        if idx < 3 and storage.*.left[idx.as(usize)] is (next_index: u32) do {
            Self { index: next_index }
        } else {
            null
        }
    }

    -- 返回右孩子：同样自动保持在同一个 storage 环境下的 `?Self`
    pub unsafe fn right(*self) -> ?Self {
        let idx = self.index.*;
        if idx < 3 and storage.*.right[idx.as(usize)] is (next_index: u32) do {
            Self { index: next_index }
        } else {
            null
        }
    }
}
```

看看 `left(*self) -> ?Self` 的返回类型设计：
当你从一个属于 `first_storage` 的根节点调用 `.left()` 时，因为返回类型是 `?Self`，解包出来的左孩子**自动具有类型 `NodeIndex<first_storage.ref>`**。你既无需在每次调 `.left()` 时手动传一遍 `&first_storage`，也无需在每个 `NodeIndex` 结构体里浪费 8 字节存指针。

---

## 四、 深入理解：运行期绑定身份（Identity）vs 内容相等性

现在到了最关键的考验时刻：
假设我们在运行期创建了两个存储池 `first_storage` 和 `second_storage`。为了测试编译器是否真的认“身份”而非认“长相”，我们故意把这两个存储池里的数组内容填得**一模一样**。

那么，`NodeIndex<first_storage.ref>` 和 `NodeIndex<second_storage.ref>` 是同一个类型吗？能否把 `second_root` 赋给期待 `NodeIndex<first_storage.ref>` 的变量？

```
  first_storage  (栈地址 0x7ffd...10) ──► 内容: values=[10, 20, 30]
                                           类型: NodeIndex<first_storage.ref>
                                                      ║
                                           【类型互异。严禁混用。】
                                                      ║
  second_storage (栈地址 0x7ffd...40) ──► 内容: values=[10, 20, 30]
                                           类型: NodeIndex<second_storage.ref>
```

> **💡 核心心智模型：运行时依赖类型锚定的是“运行期绑定身份（Identity）”，而非“瞬时值内容（Content）”。**
> 哪怕 `first_storage` 和 `second_storage` 里的每一个字节在当前时刻都相等，它们也是两个独立的运行期绑定实例。因此 `NodeIndex<first_storage.ref>` 与 `NodeIndex<second_storage.ref>` 是**大相径庭的静态类型**。

让我们通过测试代码验证这一性质：

```flurry
test runtime_dependent_node_index_safety {
    let first_storage = NodeStorage {
        values: [10, 20, 30],
        left: [1, null, null],
        right: [2, null, null],
    }

    -- 故意创建一个内容一模一样的第二存储池
    let second_storage = NodeStorage {
        values: [10, 20, 30],
        left: [1, null, null],
        right: [2, null, null],
    }

    -- 创建归属于 first_storage 的根节点下标
    let root = NodeIndex<first_storage.ref> { index: 0 }

    unsafe {
        assumes root.value() == 10;

        -- 顺着 left() 和 right() 游走，返回的节点自动继承 first_storage.ref 环境
        if root.left() is (left_node: NodeIndex<first_storage.ref>) do {
            assumes left_node.value() == 20;
        } else {
            assumes false;
        }

        if root.right() is (right_node: NodeIndex<first_storage.ref>) do {
            assumes right_node.value() == 30;
        } else {
            assumes false;
        }
    }

    -- 核心验证：归属于不同运行期存储池的 NodeIndex 具有迥然各异的类型
    let second_root = NodeIndex<second_storage.ref> { index: 0 }
    assumes root'type == NodeIndex<first_storage.ref>;
    assumes second_root'type == NodeIndex<second_storage.ref>;
    assumes root'type != second_root'type;
}
```

如果有人偶有疏忽写串了池子，试图把 `second_root` 当作 `NodeIndex<first_storage.ref>` 传进去：

```flurry
-- ❌ 编译期直接报错：期望类型 NodeIndex<first_storage.ref>，实际传入 NodeIndex<second_storage.ref>
fn reject_cross_storage_mix(
    first_storage: *NodeStorage,
    second_storage: *NodeStorage,
) {
    let wrong: NodeIndex<first_storage> = NodeIndex<second_storage> { index: 0 }
}
```

无需任何运行期开销，甚至无须在 `NodeIndex` 里多存半个比特，困扰高性能图计算与游戏引擎多年的“跨池句柄混用 Bug”在编译期被终结。

---

## 五、 本篇小结

| 核心维度 | 传统整数句柄 (`u32`) | 传统胖句柄 (`{ idx, ptr }`) | Flurry 运行时依赖类型 (`NodeIndex<storage.ref>`) |
| :--- | :--- | :--- | :--- |
| **实例内存体积** | 4 字节 | 16 字节（含指针与对齐填充） | **4 字节**（值体只存 `index: u32`） |
| **跨池混用检查** | ❌ 完全不防，运行期静默读错 | ⚠️ 需运行期 `assert(a.ptr == b.ptr)` | **✅ 编译期静态类型直接拦截** |
| **方法调用便捷度** | 每次都得手动传 `(storage, idx)` | 无需手传，但浪费内存带宽 | **自动从类型环境获取 `storage`** |
| **语法统一性** | 定长与动态类型割裂 | 需手写包装类 | **`Array<T, n>` 统一接纳编译期与运行期参数** |

---

## 六、 下一篇预告

掌握了单个 `NodeIndex<storage.ref>` 的基本用法后，在更复杂的工程场景中还会遇到新的问题：
- 当一个外层结构体 `SensorPair` 包含多个依赖同一环境的子字段时，如何转发并共享外层依赖环境？
- 当把运行时依赖类型作为参数传给其他函数时，函数签名如何表达不同参数间的环境一致性？

下一回 **《Flurry 演进手记 (082)：运行时依赖类型的等价与替换——动态环境、绑定身份与跨作用域传递》**，我们将继续深入运行时依赖类型的组合与跨函数传递规则。
