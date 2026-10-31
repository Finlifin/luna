# Flurry 演进手记 (184)：Trait 实现特化与消歧指南——结构精度法则、候选竞争与编译期冲突诊断

在[上一篇（第 183 篇）](./183-local-dependencies-wait-tickets-and-wakeups.md)中，我们探讨了局部闭包推导与递归结构体的布局边界。

在构建大型泛型库时，我们经常需要为整族类型提供一个通用的基准 Trait 实现，同时为其中某几种特殊类型（如 `i32` 或特定嵌套容器）提供更高效的特化实现（Specialization）：

```flurry
trait Describe {
    fn describe(*self) -> str;
}

struct Sample
where T: Type
{
    value: T,
}

-- 候选 1：适用于任意 Sample<T> 的通用实现
impl Describe for Sample<T>
where T: Type
{
    fn describe(*self) -> str {
        "generic sample"
    }
}

-- 候选 2：专门针对 Sample<i32> 的特化实现
impl Describe for Sample<i32> {
    fn describe(*self) -> str {
        "signed i32 sample"
    }
}
```

当调用 `signed.describe()`（其中 `signed` 的类型是 `Sample<i32>`）时，候选 1 和候选 2 的类型头都能匹配 `Sample<i32>`。Flurry 是如何确定性地选出候选 2，又如何防止多个候选之间的歧义或抢跑误判的？

今天，我们就依据 `docs/traits/implementation-resolution.md`，梳理 Flurry 的 **结构精度（Structural Specificity）选择法则与消歧实践**。

---

## 一、 核心法则：结构精度（Structural Specificity）决定优先级

在 Flurry 中，当同一个 Trait 的多个 `impl` 同时适用于某个目标类型时，选择依据**只看实现头（Implementation Head）暴露的具体类型结构深度**，而不看声明的先后顺序，也不看 `requires` 子句的长短：

```text
  实现头                         结构模式 (◇ 代表未定泛型槽)      结构精度层级
  ──────────────────────────────────────────────────────────────────────────
  T                              ◇                               0 (最宽泛)
  Sample<T>                      Sample<◇>                       1
  Sample<i32>                    Sample<i32>                     2 (更具体。)
  Sample<Pair<T, U>>             Sample<Pair<◇, ◇>>              2
  Sample<Pair<i32, u32>>         Sample<Pair<i32, u32>>          3 (最具体。)
```

有两条非常重要但容易被忽略的细节：
1. **附加的 `requires` 约束或重复类型变量只决定“是否适用”，免增结构精度**：
   例如，`impl Classify for Pair<T, T> where T: Type` 与 `impl Classify for Pair<A, B> where A: Type, B: Type` 的实现头在第一层都是两个未定泛型槽 `Pair<◇, ◇>`，因此它们的**结构精度相同（都是 1）**。写成 `Pair<T, T>` 只是多了一个 `A == B` 的适用性前提，并未让它比 `Pair<A, B>` 结构精度更高。
2. **必须等待参数类型揭晓，不提前抢跑选用宽泛实现**：
   如果在某个泛型调用点中，变量 `item` 的类型暂时是 `Sample<?M>`（其中 `?M` 稍后由兄弟参数确定为 `i32`），即使宽泛的 `impl Describe for Sample<T>` 已经能匹配任何 `Sample<?M>`，类型检查器也**不会抢先锁定宽泛实现**，而是等待 `?M` 确定为 `i32` 后，准确选中结构精度更高的 `impl Describe for Sample<i32>`。

```flurry
test specificity_selection {
    let signed = Sample<i32> { value: 7 }
    let flag = Sample<bool> { value: true }

    -- Sample<i32> 精确命中结构精度为 2 的特化实现
    assumes signed.describe() == "signed i32 sample";
    -- Sample<bool> 命中结构精度为 1 的通用实现
    assumes flag.describe() == "generic sample";
}
```

---

## 二、 编译期显式错误与候选筛选的边界

在利用 `comptime` 编写条件特化逻辑时，需要注意区分：
- **Trait 的 `where requires` 子句**：用于声明某个 `impl` 的适用前提。当 `requires` 违背时，该 `impl` 候选被安全排除，继续选择其他适用候选；
- **显式调用的 `meta.compilation.err(...)`**：当程序员在已选定执行的 `comptime` 块中显式调用 `meta.compilation.err("...")` 时，它代表一个确定性的编译期报错，免遭当成“某个候选无从适用”而静默忽略：

```flurry
const FAIL: i32 = comptime {
    -- ❌ 显式触发的编译期错误会直接报告给开发者
    meta.compilation.err("explicit configuration failure");
}
```

---

## 三、 负例：同精度重叠冲突与消歧方法

如果两个 `impl` 都能匹配同一个目标类型，且它们的结构精度分不出高下，会发生什么？
Flurry 免按“谁写在前面就选谁”来掩盖冲突，而是直接在编译期报告重叠或二义性错误。

### 负例 1：`Pair<T, i32>` 与 `Pair<i32, U>` 的对称重叠冲突

```flurry
trait Merge {
    fn select(*self) -> i32;
}

struct Pair
where
    A: Type,
    B: Type
{
    first: A,
    second: B,
}

-- 候选 A：固定第二个参数为 i32
impl Merge for Pair<T, i32>
where T: Type
{
    fn select(*self) -> i32 { 10 }
}

-- 候选 B：固定第一个参数为 i32
impl Merge for Pair<i32, U>
where U: Type
{
    fn select(*self) -> i32 { 20 }
}

fn trigger_ambiguity(p: Pair<i32, i32>) -> i32 {
    -- ❌ 编译期报错：对于 Pair<i32, i32>，候选 A 与候选 B 均适用且结构精度相同，存在二义性冲突。
    p.select()
}
```

### 如何解决对称重叠？

要消除 `Pair<i32, i32>` 上的冲突，有两种标准工程做法：
1. **提供更具体的交集特化实现**：额外补充一个 `impl Merge for Pair<i32, i32>`（其结构精度更高，专门接管两个参数都是 `i32` 的情况）；
2. **用 `requires` 使两个同精度实现互斥**：例如在第二个实现上加上 `where U: Type, requires U != i32`，使两者在 `Pair<i32, i32>` 上不再同时适用。

---

## 四、 本篇小结

| 规则维度 | 核心行为 | 最佳实践 |
| :--- | :--- | :--- |
| **结构精度优先** | 实现头暴露的具体类型层数越深，优先级越高（`Sample<i32>` 优先于 `Sample<T>`） | 将特化分支直接体现在 `impl Trait for ConcretePattern` 的实现头上 |
| **`requires` 与精度无关** | `requires` 和重复变量（`Pair<T, T>`）只过滤适用性，不提升结构精度 | 用 `requires` 划分同精度下的互斥区间，而非替代结构特化 |
| **防抢跑保证** | 当更具体的特化候选因待定类型变量暂未决断时，不会提前回退到宽泛实现 | 无论调用参数顺序如何，特化实现的选择结果始终确定一致 |
| **同精度冲突拦截** | 两个同精度实现存在共同适用实例时直接报错 | 为交集类型补充更具体的 `impl`，或用 `requires` 声明互斥条件 |

---

## 五、 下一篇预告

在前面几篇中，我们多次用到了 `pure comptime fn Identity<T: Type>() -> Type` 和 `pure comptime fn SelectType(wide: bool) -> Type`。
- 为什么带尖括号 `<...>` 的 `pure comptime fn Identity<T: Type> -> Type` 定义的是具有独立类型身份的 **Normal-Form 类型构造头**（使得 `Identity<i32> != i32`），而带圆括号 `(...)` 的 `pure comptime fn SelectType(wide: bool) -> Type` 却会在调用时直接归约为 `i32` 或 `i64`？
- 当一个泛型函数在定义点完成检查后，在多个不同调用点被实例化时，如何保证各次调用互无扰于？

下一篇 **[Flurry 演进手记 (185)：类型构造头 `Name<...>` 与类型计算函数 `Name(...)`——类型身份、编译期纯度与多点实例化](./185-interleaving-elaboration-comptime-and-freezing.md)**，我们将理清这两类类型函数的核心差异。
