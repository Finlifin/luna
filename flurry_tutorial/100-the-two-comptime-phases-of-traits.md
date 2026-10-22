# Flurry 演进手记 (100)：Trait 在两个宇宙里的双重生活——`^comptime_phase`、`comptime impl` 与共享一致性

> **系列导读**：欢迎来到《Flurry 演进手记》的**第 100 期纪念里程碑**。在过去的 99 期里，我们从最基础的变量绑定一路攀登到了形式化验证（第三卷）、双层类型理论 2LTT（第四卷）以及 Trait 名义接口与全局一致性（096–099）。今天，在第 100 期这个特殊的节点上，第四卷的 **“编译期/运行期双层宇宙（2LTT）”** 与第五卷的 **“Trait 名义多态体系”** 终于迎来了伟大的历史性交汇。我们将揭开 Flurry 独树一帜的设计——**用 `^comptime_phase` 属性划分 Trait 的两相归属**、**成员相归属（Trait Phase）与函数执行阶段（Execution Stage）的正交解耦**、**`impl` 与 `comptime impl` 的互补实现关系**，以及为什么两个宇宙必须**共享同一套全局一致性（Shared Coherence）**。

---

## 一、当同一个协议横跨两个宇宙：传统方案的“两套体系割裂症”

在构建我们的遥测与张量序列化引擎（以及 Flurry 语言内置的模式匹配视图协议，如 `ConcatPatternView`、`ListPatternView`）时，同一个抽象能力往往需要同时在两个各异的宇宙中工作：

1. **在运行期宇宙（Runtime Phase）**：当网关在运行期收到一帧真实的字节流或字符串时，我们需要通过指针接收者 `*self` 调用 `concat_pattern_view_prefix(*self, comptime prefix: ConcatWith) -> ?Self` 或 `read_code(*self) -> Code`；
2. **在编译期宇宙（Comptime Phase）**：当我们在 `comptime` 块中做编译期协议仿真、编译期字符串解构或静态路由表生成时，操作的对象是编译期值 `self`，甚至它返回的元素类型本身就是编译期专属类型（如 `ElementComptime`）。

在传统语言（或早期的粗糙设计初始写法）中，人们很容易犯两个严重的错误：
- **错误假想 1**：把“Trait 的哪一相（Trait Phase）”和“函数前面有没有写 `comptime fn`（函数执行阶段）”混为一谈。如果只要写 `comptime fn` 就强行算作另一相，那么一个普通运行期 `impl` 想要顺带提供一个供编译期查询帧长的辅助函数 `pure comptime fn wire_bytes() -> usize`，难道还要被迫拆成两个 `impl` 吗？
- **错误假想 2**：把运行期实现和编译期实现混在同一个 `impl` 块里，导致当某个类型只需要实现运行期相（比如硬件寄存器只支持运行期读取）时，没法单独兑现其中一相。

Flurry 是如何用一套**正交、优雅且严密**的机制解决这个难题的？

---

## 二、`^comptime_phase` 属性、`impl` 与 `comptime impl`

在 Flurry 中，**Trait 成员所属的相（Phase）由属性 `^comptime_phase`（完整路径为 `^attrs.comptime_phase`）显式决定，而由 `impl` / `derive` 与 `comptime impl` / `comptime derive` 分别兑现两相**。

```text
trait_def       -> trait id (:- expr)? clauses? { (assoc_decl | definition | statement)* }
impl_trait_def  -> comptime? impl expr for expr clauses? { (assoc_decl | definition | statement)* }
derive_def      -> comptime? derive expr for expr clauses?
attr_def        -> ^expr definition
```

让我们立下 Flurry Trait 两相体系的**第一条核心原则**：
1. **默认属于 Runtime Phase**：在 `trait` 内部，凡是**没有**标注 `^comptime_phase` 属性的关联项（`assoc`）和方法，默认全部属于 **Runtime Phase（运行期相 / 默认相）**，由普通的 **`impl Trait for T`** 或 **`derive Trait for T`** 负责实现。
2. **`^comptime_phase` 标记属于 Comptime Phase**：凡是显式标注了 **`^comptime_phase`**（即 `^attrs.comptime_phase`）属性的关联项和方法，属于 **Comptime Phase（编译期相）**，专门由 **`comptime impl Trait for T`** 或 **`comptime derive Trait for T`** 负责实现。

让我们看两个最原汁原味的 Flurry 范例——遥测编码读取接口 `ReadCode` 与语言内置的字符串前缀匹配视图协议 `ConcatPatternView`：

```flurry
use attrs.comptime_phase;

-- 示例 1：遥测设备码读取接口 ReadCode
trait ReadCode {
    -- 默认相的只读签名关联类型，可被两相方法签名共同引用
    assoc Code: Type = u32;

    -- 1. 未标 ^comptime_phase：属于默认的 Runtime Phase（由普通 impl 兑现）
    fn read_code(*self) -> Code;

    -- 2. 标注 ^comptime_phase：属于 Comptime Phase（由 comptime impl 兑现）
    ^comptime_phase
    comptime fn static_code(self) -> Code;
}

-- 示例 2：Flurry 模式匹配视图协议 ConcatPatternView
trait ConcatPatternView {
    assoc ConcatWith: Type;

    -- 默认 Runtime Phase 方法（供运行期 `"Bearer " ++ token` 模式解构调用）
    fn concat_pattern_view_prefix(*self, comptime prefix: ConcatWith) -> ?Self;
    fn concat_pattern_view_postfix(*self, comptime postfix: ConcatWith) -> ?Self;

    -- 标注 ^comptime_phase 的 Comptime Phase 方法（供编译期模式解构调用）
    ^comptime_phase
    comptime fn concat_pattern_view_prefix_comptime(self, prefix: ConcatWith) -> ?Self;
    ^comptime_phase
    comptime fn concat_pattern_view_postfix_comptime(self, postfix: ConcatWith) -> ?Self;
}
```

### 按需分别兑现两相：`impl` vs `comptime impl`

正因为两相通过 `^comptime_phase` 清晰划分，我们可以为 `DeviceTag` 同时提供两相的互补实现，也可以为只活在编译期的 `SchemaStub` 仅提供 `comptime impl`：

```flurry
use attrs.comptime_phase;

struct DeviceTag {
    code: u32,
}

-- 普通 impl：兑现 ReadCode 的默认相（Runtime Phase）成员
impl ReadCode for DeviceTag {
    fn read_code(*self) -> u32 {
        self.code.*
    }
}

-- comptime impl：兑现 ReadCode 的编译期相（^comptime_phase）成员
comptime impl ReadCode for DeviceTag {
    ^comptime_phase
    comptime fn static_code(self) -> u32 {
        self.code
    }
}

-- 如果某个类型只在编译期做协议仿真，只写 comptime impl：
struct SchemaOnlyStub {
    id: u32,
}

comptime impl ReadCode for SchemaOnlyStub {
    ^comptime_phase
    comptime fn static_code(self) -> u32 {
        self.id
    }
}

fn bad_runtime_use(stub: *SchemaOnlyStub) -> u32 {
    -- ❌ 编译期直接报错。
    -- SchemaOnlyStub 只通过 comptime impl 实现了 ReadCode 的 comptime phase，
    -- 缺少 runtime phase 的实现。编译器不会拿 comptime impl 跨相顶替。
    stub.read_code()
}
```

---

## 三、深度辨析：为什么“Phase 属性归属”与“函数执行阶段”必须正交解耦？

这是理解 Flurry Trait 设计精髓的核心钥匙。
初学者常问：**“为什么要特意用 `^comptime_phase` 属性来标记 Trait 的编译期相，而非直接把所有 `comptime fn` 都当成 Comptime Phase？”**

因为 **“一个成员归属于哪一相的实现契约（Trait Phase Attribution）”** 与 **“这个函数自身或它的参数在哪个阶段执行（Execution Stage）”** 是两个完全正交的维度。

下面通过图示把这四种组合看清：

```text
+-----------------------------------------------------------------------------------+
|  Trait Phase 归属（^comptime_phase）vs 函数执行阶段（comptime fn）         |
|                                                                                   |
|  维度 1：归属哪个 Trait Phase？（决定写在 `impl` 还是 `comptime impl` 里）          |
|    • 无 `^comptime_phase` 属性 ──► 属于 Runtime Phase（由普通 `impl` / `derive` 兑现）|
|    • 有 `^comptime_phase` 属性 ──► 属于 Comptime Phase（由 `comptime impl` 兑现）   |
|                                                                                   |
|  维度 2：函数自身与参数如何执行？（由 `comptime fn` / `fn` 与参数标记决定）         |
|  ┌──────────────────────────────────────┬───────────────────────────────────────┐ |
|  │ 组合 ①：无 ^comptime_phase + 普通 fn │ 组合 ②：无 ^comptime_phase            │ |
|  │ fn read(*self) -> u32;               │        + 带 comptime 参数的普通 fn    │ |
|  │ • 归属：Runtime Phase (`impl`)       │ fn splice(comptime d: Self) -> Target │ |
|  │ • 执行：运行期直接执行               │ • 归属：Runtime Phase (`impl`)        │ |
|  │                                      │ • 执行：编译期用 d 实例化，运行期执行 │ |
|  ├──────────────────────────────────────┼───────────────────────────────────────┤ |
|  │ 组合 ③：无 ^comptime_phase           │ 组合 ④：有 ^comptime_phase            │ |
|  │        + pure comptime fn            │        + comptime fn / fn             │ |
|  │ pure comptime fn wire_bytes() -> ... │ ^comptime_phase                       │ |
|  │ • 归属：Runtime Phase (`impl`)       │ comptime fn static_code(self) -> Code │ |
|  │ • 执行：随普通 impl 一起提供，但该   │ • 归属：Comptime Phase (`comptime impl`)│
|  │         方法在编译期求值（无须拆相  ）│ • 执行：专为编译期相协议与求值服务    │ |
|  └──────────────────────────────────────┴───────────────────────────────────────┘ |
+-----------------------------------------------------------------------------------+
```

你看。正因为把 `^comptime_phase`（相归属）和 `comptime fn`（函数执行阶段）解耦了：
1. 在第 112 篇我们将编写的零拷贝序列化接口 `Encode` 中，如果你希望每一个普通的 `impl Encode<.binary> for T` 顺带提供一个编译期计算紧凑字节数的 `pure comptime fn wire_bytes() -> usize`，你**无需**给它贴 `^comptime_phase` 逼用户写两遍 `impl`，它直接作为默认相契约的一部分随普通 `impl` 一起交付，并在编译期求值。
2. 而对于像 `ListPatternView`、`MapPatternView`、`VariadicCollector` 或 `Eq`/`Hash` 这样真正需要**按运行期目标 vs 编译期目标分开实现（甚至两相拥有各自独立的关联类型）**的接口，则使用 `^comptime_phase` + `comptime impl` / `comptime derive` 干净利落地分相实现。

### 关联项的分相与只读签名共享

在同一个 Trait 中，关联项（`assoc`）同样遵守这套规则：
- **分相关联项**：如果编译期相需要自己独立的关联类型，直接给 `assoc` 加上 `^comptime_phase`。例如标准模式视图与变参收集器：
  ```flurry
  use attrs.comptime_phase;

  trait MapPatternView {
      assoc FieldType: pure comptime fn(Symbol) -> Type;
      ^comptime_phase
      assoc FieldTypeComptime: pure comptime fn(Symbol) -> Type;

      fn map_pattern_view_get(*self, comptime property: Symbol) -> ?FieldType(property);

      ^comptime_phase
      comptime fn map_pattern_view_get_comptime(self, property: Symbol) -> ?FieldTypeComptime(property);
  }
  ```
- **跨相只读签名关联依赖共享**：像前面 `ConcatPatternView` 中的 `assoc ConcatWith: Type;` 或 `ReadCode` 中的 `assoc Code: Type = u32;`，虽然声明在默认相中，但因为它是只读的签名级类型依赖，`^comptime_phase` 方法的签名可以直接引用它。当 `impl` 与 `comptime impl` 分别实现时，两边对共享关联项给出的定义必须一致。

---

## 四、互补实现关系与“两相共享 Coherence”

现在，让我们思考一个关乎整个类型系统健全性的核心问题：
既然一个 Trait 的两相分别用 `impl` 和 `comptime impl` 实现，那么：
1. 为同一个类型 `DeviceTag` 同时写 `impl ReadCode for DeviceTag` 和 `comptime impl ReadCode for DeviceTag`，是否属于冲突？
2. 能否为泛型 `Packet<T, T>` 写一个 `comptime impl`，同时又为同精度的 `Packet<A, B>` 写一个普通的 runtime `impl`？
3. 如果高精度实现 `Packet<i32, i32>` 只写了 runtime `impl` 没写 `comptime impl`，而在编译期相调用它时，编译器是否会悄悄回退去用粗糙的泛型 `comptime impl Packet<A, B>`？

Flurry 语言规范用一条十分严密的 **“共同实现关系与共享 Coherence 核心规则”** 一次性回答了这三个问题：

> 1. **互补合流**：归一头（Normalized Head）和适用条件完全相同的 `impl` 与 `comptime impl`，不会互相冲突，而是自动组成**同一份实现关系的两个互补 phase 部分**。
> 2. **共享 Coherence**：两个 phase 处于**同一个实现世界**，共享同一套结构精度排序与同精度重叠冲突检查。禁止因为一个是 `comptime impl`、一个是普通 `impl` 就绕过重叠检查。
> 3. **选中后不因缺相降级回退**：查询先按统一的结构精度选定唯一的最精确实现关系，然后再检查该关系是否兑现了所需的 phase。如果选中的高精度实现关系缺少该 phase，**直接报错，不偷偷退回较粗精度的泛化实现**。

### 互补合流 vs 跨相重叠冲突 vs 禁止缺相回退

```text
  [ 情形 A：同头同条件 ──► 互补合流 ✅ ]
    impl ReadCode for DeviceTag          (提供 Runtime Phase)
  + comptime impl ReadCode for DeviceTag (提供 Comptime Phase)
  ══════════════════════════════════════════════════════════════
  = 合并为 DeviceTag 的完整两相实现关系。

  [ 情形 B：同精度重叠 ──► 跨相冲突报错 ❌ ]
    comptime impl ReadCode for Packet<T, T>  (精度 1：Packet<◇, ◇>)
  ⚡ impl ReadCode for Packet<A, B>           (精度 1：Packet<◇, ◇>)
  ══════════════════════════════════════════════════════════════
  = 在 Packet<i32, i32> 上同精度重叠。哪怕相不同也编译期报错。

  [ 情形 C：高精度缺相 ──► 直接报错，严禁退回低精度 ❌ ]
    泛型底层 (精度 1)：impl + comptime impl ReadCode for Sample<T>
    具体特化 (精度 2)：仅实现了 impl ReadCode for Sample<i32>（未写 comptime impl。）
  ══════════════════════════════════════════════════════════════
  查询 Sample<i32> 的 Comptime Phase：
    第 1 步：按结构精度命中精度 2 的 Sample<i32> 实现关系；
    第 2 步：发现 Sample<i32> 缺少 comptime impl ──► ❌ 立即诊断缺相错误。
            （不回退去调用精度 1 的 Sample<T> comptime impl，防止两相人格分裂。）
```

让我们在代码中亲眼验证这一防线：

```flurry
use attrs.comptime_phase;

trait ProtocolId {
    fn runtime_id(*self) -> u32;

    ^comptime_phase
    comptime fn proto_id(self) -> u32;
}

struct Packet
where A, B
{
    first: A,
    second: B,
}

-- 实现 1：为 Packet<T, T> 提供 comptime impl（结构键 Packet<◇, ◇>，精度 1）
comptime impl ProtocolId for Packet<T, T>
where T
{
    ^comptime_phase
    comptime fn proto_id(self) -> u32 {
        1u32
    }
}

-- ❌ 编译期直接报错：（共享 Coherence 冲突拦截）
-- 尽管下面这份实现是普通的 runtime impl，针对的是 Packet<A, B>，
-- 但它与上面的 comptime impl Packet<T, T> 处于同一个共享 Coherence 世界中，
-- 两者同属精度 1 的 Packet<◇, ◇> 且在 Packet<i32, i32> 上重叠冲突。
impl ProtocolId for Packet<A, B>
where A, B
{
    fn runtime_id(*self) -> u32 {
        2u32
    }
}
```

通过将 **属性归属（`^comptime_phase`）**、**互补实现（`impl` + `comptime impl`）** 与 **共享 Coherence（禁止跨相重叠与缺相回退）** 融为一体，Flurry 保证了：任何一个类型在编译期宇宙和运行期宇宙中看到的接口契约、特化优先级与关联类型，都是**完全统一、没有前后存在分歧**的。

---

## 五、本节核心概念速查表

| 概念 / 语法 | 所属机制与含义 | 核心语义边界 |
| :--- | :--- | :--- |
| 无 `^comptime_phase` 的成员 | 属于 Trait 的 **Runtime Phase（默认相）** | 由普通 `impl Trait for T` 或 `derive Trait for T` 兑现 |
| `^comptime_phase` 成员 | 属于 Trait 的 **Comptime Phase（编译期相）** | 可修饰 `assoc` 或方法；由 `comptime impl` 或 `comptime derive` 兑现 |
| **Phase 归属 vs 函数阶段** | `^comptime_phase` $\neq$ `comptime fn` | 属性决定属于哪一相实现契约；`comptime fn` / `fn` 及参数 `comptime` 标记独立决定执行阶段 |
| **只读签名关联依赖共享** | 默认相 `assoc` 可被 `^comptime_phase` 签名引用 | 互补的 `impl` 与 `comptime impl` 必须给出一致的关联项值 |
| **互补实现与共享 Coherence** | 两相共用同一实现世界与结构精度排序 | 同头同条件互补合流；同精度跨相重叠报错；高精度缺相直接报错，**不回退到低精度实现** |

---

## 六、下一期预告

在第 100 期的最后，我们看到了 `^comptime_phase` 与“两相共享 Coherence”如何保卫了编译期与运行期的人格统一。
现在，让我们顺着这个思路再往前迈出激动人心的一步：
1. 在编译器内部执行编译期计算或构建静态查找表时，为什么 **`Eq` 和 `Hash` 的 Comptime 相（`comptime impl`）** 扮演着如此特殊的角色？用 `Eq`/`Hash` 做编译期查询缓存，能否代替 PTC 的逻辑相等证明？
2. 当我们在编译期算出了一个复杂的配置结构体、数组甚至一个整数 `24`，想要把它**从编译期宇宙跨越次元壁传递到运行期程序中使用**时，既然编译期虚拟机的宿主内存地址禁止直接变成运行期机器码的内存地址，编译器到底是靠什么协议把编译期值“物化（Materialize / Splice）”成运行期值的？

下一期 **《Flurry 演进手记 (101)：为什么编译期算好的 Hash 能直接带进运行时——Eq/Hash 与跨阶段 Splice 契约》**，我们将揭开跨阶段值传递协议 `Splice` 与有限自动派生的神秘面纱。
