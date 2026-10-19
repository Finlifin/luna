# Flurry 演进手记 (098)：当多个 impl 看起来都能匹配——结构索引、特化精度与实现选择算法

> **系列导读**：在上一期（097）中，我们掌握了如何用 `where T :- A + B` 和 `requires not (T :- C)` 声明复杂的能力约束。但在为遥测与张量序列化引擎编写具体实现（`impl`）时，我们很快会遇到一个所有现代静态语言都绕不开的核心难题——**多重匹配与特化（Specialization）**：当我们既想为一族泛型容器 `Sample<T>` 提供通用的默认实现，又想专门为 `Sample<i32>` 提供一份定制实现时，编译器该听谁的？如果两份实现的区别仅仅是 `where requires` 里的条件长短不同，或者写成 `Pair<T, T>` 与 `Pair<A, B>`，谁的优先级更高？今天，我们将深入 Flurry 编译器的实现检索引擎，彻底看懂**统一深度结构精度（Structural Specificity）**与**未决候选保护机制**。

---

## 一、特化的两难困境：版本 `v0` 的“一刀切”与“黑魔法”

在设计遥测数据包装器 `Sample<T>` 的描述接口 `Describe` 时，我们有一个非常自然的工程诉求：
- 对于绝大多数类型的样本 `Sample<T>`，我们只需要返回通用标签 `"sample"`；
- 但对于有符号 32 位整数样本 `Sample<i32>`，我们需要输出更精确的诊断标签 `"signed sample"`。

在其他系统语言里，处理这种重叠往往走向两个极端：
- **极端 A（一刀切禁止重叠）**：像标准 Rust（不开启 nightly `specialization` 特性）那样，直接报错说 `Sample<T>` 已经覆盖了 `Sample<i32>`，逼迫你用丑陋的宏或者手动为几十个具体类型逐一编写实现；
- **极端 B（按文件声明顺序或复杂的约束包含偏序猜测）**：谁写在前面听谁的，或者谁的 `where` 子句看起来更复杂听谁的——结果一旦跨模块导入顺序变了，或者两个复杂 `where` 子句无法比较包含关系，程序的行为就会发生诡异的静默漂移！

Flurry 是如何给出一个**既支持优雅特化、又 100% 确定且不依赖声明顺序**的数学解答的？

---

## 二、迭代演进 `v1`：按暴露的类型结构选优——`Sample<i32>` 为何稳赢 `Sample<T>`？

在 Flurry 中，当多个 `impl` 同时适用于同一个查询时，编译器遵循一条铁律：
> **选择依据只看归一后实现头（Implementation Head）中暴露的类型结构本身——暴露的具体结构越深（结构精度越高），优先级越高！**

让我们先看一段可以直接在 Flurry 中合法运行的代码：

```flurry
trait Describe {
    fn describe(*self) -> str;
}

struct Sample
where T
{
    value: T,
}

-- 候选 1：面向整个 Sample<T> 类型族的通用实现（结构键：Sample<◇>，精度 1）
impl Describe for Sample<T>
where T
{
    fn describe(*self) -> str {
        return "sample";
    }
}

-- 候选 2：面向具体 Sample<i32> 的特化实现（结构键：Sample<i32>，精度 2）
impl Describe for Sample<i32> {
    fn describe(*self) -> str {
        return "signed sample";
    }
}

test {
    let signed = Sample<i32> { value: 7i32 };
    let flag = Sample<bool> { value: true };

    -- signed 匹配两份实现，但 Sample<i32> 结构精度更高（2 > 1），稳赢！
    println("{}", signed.describe()); -- 输出：signed sample
    -- flag 只匹配通用实现 Sample<T>
    println("{}", flag.describe());   -- 输出：sample
}
```

为什么 `Sample<i32>` 的精度是 `2`，而 `Sample<T>` 的精度是 `1`？让我们打开编译器内部的**统一深度分解算法（Unified Depth Decomposition）**！

---

## 三、Code Cartoon：统一深度结构键（Structural Key）与短板效应

Flurry 编译器在索引每一个 `impl` 的主体类型头时，会把尚未确定的类型参数看作通配槽 **`◇`**（注意：`◇` 是编译器内部用于结构分析的通配占位符，不是源语言里的 `Any`）。

然后，编译器从根节点出发，**一层一层同步向下展开**，直到撞上**距离根节点最近的那个未知通配槽 `◇`** 为止！

```text
[ 算法核心：由最浅的未知参数 ◇ 决定整棵结构键能统一展开到的最大深度！ ]

  例 1: Box<Pair<T, U>>                  例 2: Pair<Box<T>, U>
          Box  (第 1 层：具体构造子)             Pair (第 1 层：具体构造子)
           │                                   /    \
          Pair (第 2 层：具体构造子)         Box     ◇ (U 在第 1 层子位置就已经是未知槽！)
         /    \                             │
        ◇      ◇ (T, U 在第 2 层遇阻)       (被右侧的 ◇ 拦住，第 2 层不再继续展开！)
        │                                   │
  结构键：Box<Pair<◇, ◇>> (精度 = 2)     结构键：Pair<◇, ◇> (精度 = 1)
```

如果另一侧分支已经到达了没有子节点的具体叶子类型（比如 `i32`），那么继续加深层数时，这个已经见底的具体叶子 `i32` 会原样保留：

| 实现头（Implementation Head） | 编译器提取的统一结构键 | 结构精度（Specificity） | 直观解释 |
| :--- | :--- | :---: | :--- |
| `T` | `◇` | **0** | 根部就是未知参数，最泛化 |
| `Sample<T>` / `Box<T>` | `Sample<◇>` / `Box<◇>` | **1** | 暴露了第 1 层外层名义构造子 |
| `Pair<Box<T>, U>` | `Pair<◇, ◇>` | **1** | 被右分支第 1 层的未知 `U` 限制在了深度 1 |
| `Sample<i32>` | `Sample<i32>` | **2** | 全部是具体叶子，完全展开 |
| `Box<Pair<T, U>>` | `Box<Pair<◇, ◇>>` | **2** | 第 1 层 `Box`、第 2 层 `Pair` 均已暴露 |
| `Pair<Box<T>, i32>` | `Pair<Box<◇>, i32>` | **2** | 右侧 `i32` 是已见底的具体叶子，左侧可继续展开到 `Box<◇>` |
| `Box<Pair<i32, u32>>` | `Box<Pair<i32, u32>>` | **3** | 3 层具体构造子与具体叶子全部暴露 |

看懂这张表，你就彻底掌握了 Flurry 的特化哲学：**只有真正在实现头里多剥开一层具体的类型构造子或具体叶子，结构精度才会提升！**

---

## 四、三大黄金边界：什么东西**不会**提高特化优先级？

很多有 C++ 模板经验的读者可能会问：“如果我不把具体类型写进结构里，而是通过**重复类型变量**（如 `Pair<T, T>`）、或者写**更长的 `requires` 约束**（如 `where T :- Reading + Named + Copy`），能让它的优先级高于 `Pair<A, B>` 吗？”

Flurry 的回答非常干脆：**绝对不会！**

> **核心规则**：重复出现的类型变量、逻辑 refine 条件、以及额外的 `where` / `requires` 约束，**只负责判断该实现“是否适用（Applicability）”，绝不增加结构精度（Specificity）！**

### 边界 1：`Pair<T, T>` 与 `Pair<A, B>` 同精度冲突

让我们看下面这个被编译器直接拒绝的经典反例：

```flurry
trait Classify {
    fn classification(*self) -> u32;
}

struct Pair
where A, B
{
    first: A,
    second: B,
}

-- 候选 1：要求两个字段类型相同（结构键依然是 Pair<◇, ◇>，精度 = 1！）
impl Classify for Pair<T, T>
where T
{
    fn classification(*self) -> u32 {
        return 1u32;
    }
}

-- 候选 2：任意两个字段类型（结构键同样是 Pair<◇, ◇>，精度 = 1！）
-- ❌ 编译期直接拒绝！
-- 两份实现都归入精度为 1 的 Pair<◇, ◇>，且存在共同适用实例（如 Pair<i32, i32>）！
-- 重复出现类型变量 T 只是多了一条 A == B 的适用性等式，绝不提升结构精度！
impl Classify for Pair<A, B>
where A, B
{
    fn classification(*self) -> u32 {
        return 2u32;
    }
}
```

但反过来，如果两份同精度实现的 `requires` 条件在数学上**可以证明完全互斥（Mutually Exclusive，共同适用实例集为空）**，那么即使它们都在同一个精度桶里，也永远不可能同时命中同一个具体类型，此时编译器允许它们和平共处！

### 边界 2：未决候选（Pending Candidates）的延迟保护铁律

这又是 Flurry 防止“泛型特化不一致”的一道关键防线！
假设我们在一个尚未完全实例化的泛型或开放上下文里查询 `Sample<T>` 的 `Describe` 实现（此时 `T` 还只是一个未知的类型参数）。

编译器在检索索引时，会同时看到两份候选：
1. 精度为 `2` 的 `impl Describe for Sample<i32>` —— 当前状态：**仍未决（Pending）**（因为现在还不知道 `T` 最终会不会被实例化成 `i32`！）；
2. 精度为 `1` 的 `impl Describe for Sample<T>` —— 当前状态：**已适用（Applicable）**。

```text
[ 当查询 Sample<T> (T 尚未知) 时的候选表 ]
  ├── 精度 2: impl Describe for Sample<i32>  ──► 状态：⏳ 仍未决 (Pending)
  └── 精度 1: impl Describe for Sample<T>    ──► 状态：✅ 已适用 (Applicable)
                    │
                    ▼
  🛑 编译器抉择：因为比当前适用者（精度 1）更精确的候选（精度 2）仍处于 Pending 状态，
     绝对禁止在此刻草率提交精度 1 的通用实现！必须等待 T 确定排除 i32（或等于 i32）后再做最终裁决！
```

试想一下：如果编译器在 `T` 还没确定时就急匆匆地锁死了精度 `1` 的 `Sample<T>` 实现，那么当外层调用者把 `Sample<i32>` 传进来时，同一个 `Sample<i32>` 直接调用 `.describe()` 得到 `"signed sample"`，经由泛型函数调用却得到 `"sample"`！
通过**“高一层的候选未全部排除前，绝不提交低一层的候选”**这一铁律，Flurry 保证了无论你是直接调用还是经过泛型转发，同一个具体类型选出的 `impl` 永远 100% 一致！

---

## 五、本节核心概念速查表

| 概念 / 机制 | 核心定义 | 编译器行为与设计目的 |
| :--- | :--- | :--- |
| **结构精度（Specificity）** | 归一类型头从根向叶统一分解的深度 | 暴露的具体构造子/具体叶子越深，精度越高；受最浅的未知参数 `◇` 限制 |
| **适用性（Applicability）** | `requires` 子句、Trait 约束与重复变量等式 | 只决定候选属于“已适用 / 已排除 / 仍未决”，**不增加结构精度** |
| **同精度冲突判定** | 同一接口实例、同一精度的两份独立实现 | 若存在共同适用实例（如 `Pair<T, T>` 与 `Pair<A, B>` 共同覆盖 `Pair<i32, i32>`），直接报错 |
| **未决延迟保护** | 高精度候选处于 `Pending` 时阻塞低精度提交 | 防止泛型代码在类型具体化之前过早绑定泛化实现，保证全局行为一致 |
| **不同接口同名方法** | 两个不同 Trait 拥有同名方法 `read()` | 结构精度**只在同一个已确定的 Trait 内部**比较候选，绝不跨 Trait 抢夺方法名 |

---

## 六、下一期预告

在今天的例子里，`Sample<T>`、`Describe` 以及它的两份 `impl` 都写在同一个包（Package）里。
但在真实的软件生态中，项目往往由十几个第三方库拼接而成：
- 标准库 `std` 定义了基础类型 `u32` 和元组/对偶结构 `Pair<A, B>`；
- 序列化基础库 `base` 定义了接口 `pub trait Label`；
- 现在，两个互不知情的业务包 `pkg_a` 和 `pkg_b` 同时引入了 `base`，并且都想为 `Pair` 实现 `Label`！
  - `pkg_a` 写了：`impl Label for Pair<LocalA, T> where T`
  - `pkg_b` 写了：`impl Label for Pair<U, LocalB> where U`

仔细看这两行代码：如果编译器放任它们各自编译通过，那么当主程序同时链接 `pkg_a` 和 `pkg_b`，并构造出一个 `Pair<LocalA, LocalB>` 时，宇宙大爆炸发生了——两份同精度的实现撞车了！最关键的是：**编译器凭什么在单独编译 `pkg_b` 的那一刻（还根本不知道世界上有 `pkg_a` 存在时），就当场判处 `pkg_b` 的 `Pair<U, LocalB>` 违法，却放行了 `pkg_a` 的 `Pair<LocalA, T>`？**

下一期 **《Flurry 演进手记 (099)：守住全局唯一性的底线——Coherence（一致性）、孤儿规则与负向推理证据》**，我们将揭开极其精妙的 **“有序本地身份规则（Ordered Local Identity Rule）”**！
