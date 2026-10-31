# Flurry 演进手记 (084)：初识命题类型约束（PTC）——定义期类型命题推理与约束检验

在前面的章节中，我们已经接触了精化类型 `refines { ... }`、编译期谓词 `comptime predicate`、前后置契约 `requires / ensures` 以及类型模式匹配 `T match { ... }`。

在实际工程（例如泛型容器、编解码器或协议框架）中，函数签名与类型定义往往包含多类**类型命题（Type Propositions）**：
- **布尔逻辑组合**：`A and (B or not C)`；
- **类型身份与有限域选择**：`T == u32 or T == f32`、`RegisterId != u32`；
- **结构分类与表示性质**：`Type.predicates.is_tuple(T)`、`not Type.predicates.is_not_sized(T)`；
- **Trait 实现与行包含关系**：`T :- Copy`、错误行包含等。

为了在定义端（特别是采用 `ordinary` 检查的泛型定义中）对这些类型命题进行一致的逻辑推理，Flurry 的类型系统采用了一套统一的命题约束机制——**PTC（Propositional Type Constraints，命题类型约束）**。本篇我们将介绍定义期类型命题蕴涵与具体实例求值的区别、PTC 的职责边界，以及约束检验的四种结果情形。

---

## 一、 问题背景：具体类型代入求值 vs 定义期类型命题蕴涵

在编写带约束的泛型接口时，首先需要区分“具体类型代入后的计算”和“定义期类型命题蕴涵证明”这两个概念：

```text
  [ 场景 A：具体类型代入 (Concrete Instantiation) ]
  输入已知：T = Tuple<u32, u32>
  检查 `Type.predicates.is_tuple(Tuple<u32, u32>)` ──► 直接得 `true`。

  [ 场景 B：定义期类型命题蕴涵证明 (Symbolic Entailment: Γ ⊢ Q) ]
  在泛型定义体内：
  类型参数 `T` 尚未代入具体类型，只有上下文已知的前提集合 Γ（如 where 子句与前置调用的 ensures）。
  此时编译器需要验证的是逻辑蕴涵问题：
  “在已知类型前提集合 Γ 下，能否保证对一切满足 Γ 的类型输入，目标操作所需的前提命题 Q 必然成立？”
```

例如，在 `ordinary` normalization 中，已知一个尚未具体化的类型 `T` 满足 `Type.predicates.is_tuple(T)`，就足以满足元组遍历操作对容器种类的前置要求——这并切勿求在定义期就已经算出该元组的具体长度或每个元素的具体类型。反之，如果一个函数内部调用了要求 `T == u32 or T == f32` 的操作，而签名只写了无约束的 `where T: Type`，定义期检查就能立即发现类型前提不足。

---

## 二、 核心模型：逻辑骨架与类型领域规则解耦，以及四种查询结果

在 Flurry 中，**PTC 只处理类型命题**（程序运行时的数值状态、内存与资源规格由第三卷介绍的程序验证层负责）。PTC 将约束求解分为两层：
1. **外层：命题逻辑协调层**——处理合取（`and`）、析取（`or`）、否定（`not`）以及从 `where requires`、前置调用 `ensures` 和 `inline` 分支条件中收集到的类型命题前提 $\Gamma$。
2. **内层：类型领域求解器**——处理特定领域的类型原子关系，包括类型相等与同余、结构分类、Normal-Form 头模式匹配、Trait 满足性、表示定长性与行集合关系（我们将在下一篇 `085` 系统梳理这些原子关系）。

当编译器根据当前作用域的已知类型事实集合 $\Gamma$ 去检验某个目标要求 $Q$（即判断 $\Gamma \vdash Q$）时，结果分为以下四种情形：

```text
+-------------------------------------------------------------------------+
|                    类型命题检验 (Γ ⊢ Q) 的四种结果情形                    |
+-------------------------------------------------------------------------+
|                                                                         |
|  1. Entailed (蕴涵成立)    ──► ✅ 检查通过                               |
|     • 含义：在已知前提 Γ 下，目标类型命题 Q 有可靠证明依据。              |
|                                                                         |
|  2. NotEntailed (存在反例) ──► ❌ 当前前提不足以保证 Q                    |
|     • 含义：存在满足 Γ 却不满足 Q 的有效类型反例。                        |
|     • 注意：NotEntailed 不等于证明了 Q 的否定对所有输入都成立。           |
|                                                                         |
|  3. Unknown (暂未决)       ──► ⚠️ 按阶段规则延迟或报错                    |
|     • 含义：在当前能力与预算内尚未完成证明或反例验证。                    |
|     • 注意：这是检查状态，不是命题的第三种布尔真值；ordinary 必检义务     |
|       不能靠 Unknown 通过。                                             |
|                                                                         |
|  4. IllFormed (良构性错误) ──► 🚫 目标或观察不满足定义域前提              |
|     • 含义：例如对不具备物理内存表示的纯编译期对象查询运行期字节大小。    |
+-------------------------------------------------------------------------+
```

---

## 三、 实例分析：类型命题传递与“存在反例（NotEntailed）”

让我们通过一个类型选择与布局查询的例子（参考 `docs/type/propositional-constraints/solver-protocol.md`），理解类型命题如何在函数之间传递，以及“存在反例”与“蕴涵成立”的区别：

```flurry
use std.prelude.*;

enum StorageKind {
    integer,
    real,
}

-- 1. 类型工厂通过 ensures 提供结果类型命题：返回类型必为 u32 或 f32
pure comptime fn StorageType(kind: StorageKind) -> result: Type
where
    ensures result == u32 or result == f32
{
    kind match {
        .integer => u32,
        .real => f32,
    }
}

-- 2. 受约束函数要求传入的类型 T 满足 `T == u32 or T == f32`
pure comptime fn supported_size(T: Type) -> usize
where
    requires T == u32 or T == f32
{
    Type.size_of(T)
}

-- 3. 调用方利用 StorageType 的结果保证满足 supported_size 的前置要求
pure comptime fn selected_size(kind: StorageKind) -> usize {
    let Selected = StorageType(kind);
    supported_size(Selected)
}

test {
    const INTEGER_SIZE = selected_size(.integer);
    const REAL_SIZE = selected_size(.real);
    println("integer size: {}", INTEGER_SIZE);
    println("real size: {}", REAL_SIZE);
}
```

在检查 `selected_size` 时：
- 调用 `StorageType(kind)` 后，当前环境 $\Gamma$ 获得结果契约命题：`Selected == u32 or Selected == f32`；
- 紧接着调用 `supported_size(Selected)` 时，待证目标 $Q$ 正好是 `Selected == u32 or Selected == f32`，因此 $\Gamma \vdash Q$ 直接返回 **Entailed**，无需先知道 `kind` 具体是 `.integer` 还是 `.real`。

反之，如果一个采用 `ordinary` 检查的泛型函数漏写了类型前提：

```flurry
-- ❌ 编译期报错：当前前提 Γ 仅有 `T: Type`，尚难保证 `T == u32 or T == f32`
-- （例如反例 `T = bool` 满足 `T: Type`，但违背 `supported_size` 的要求）
fn bad_generic_caller(value: T) -> usize where T {
    supported_size(T)
}
```

> **说明：`NotEntailed`（存在反例） $\neq$ 证明了 $\neg Q$**
> 对 `bad_generic_caller` 给出反例 `T = bool`，只说明当前前提 `T: Type` 过弱，尚难保证对**所有** `T` 都满足 `T == u32 or T == f32`；非代表没有任何类型能满足该条件。

---

## 四、 良构性错误（IllFormed）与职责边界

### 4.1 观察操作的良构域检查

在 PTC 中，像 `Type.size_of(T)` 或字段类型查询这样的观察项（Observation Terms）都有明确的定义域前提。如果将一个缺乏运行期表示的纯编译期对象传给布局大小查询，或者在缺乏某字段的封闭结构上强行读取该字段类型，PTC 免随便返回一个真假值，而是直接报告 **IllFormed（良构性错误）**。

### 4.2 职责边界：PTC 只处理类型命题

我们再次明确 Flurry 中两类静态检查机制的边界：

```text
+-------------------------------------------------------------------------+
|                    Flurry 两类静态检查机制的职责边界                      |
+-------------------------------------------------------------------------+
|                                                                         |
|  [ PTC (命题类型约束) ]                                                  |
|  • 管辖范围：只处理类型命题——类型身份与相等性、Normal-Form 头模式、       |
|              结构分类、固定大小性质、Trait 实现证据、行包含与子集关系。   |
|  • 不涉及：一般算术值追踪，以及运行期程序状态、内存与资源规格。          |
|                                                                         |
|  ================ [ 类型命题 (PTC) vs 程序状态验证 ] ====================|
|                                                                         |
|  [ 程序验证层 (Program Verification Layer) ]                             |
|  • 管辖范围：程序状态、数值与索引边界、内存与资源规格、指针有效性。      |
+-------------------------------------------------------------------------+
```

---

## 五、 本篇小结

| 约束查询结果 | 逻辑含义（针对 $\Gamma \vdash Q$） | 编译器行为与工程应对 |
| :--- | :--- | :--- |
| **Entailed** | 从已知类型前提 $\Gamma$ 可靠推出目标命题 $Q$ | 检查通过，操作前提得以满足 |
| **NotEntailed** | 存在满足 $\Gamma$ 但违背 $Q$ 的有效反例 | 前提不足（注意：不代表 $\neg Q$ 对所有输入都成立） |
| **Unknown** | 在当前能力与预算内未完成证明或反例验证 | 作为检查状态保留；`ordinary` 必检义务无法靠 `Unknown` 通过 |
| **IllFormed** | 目标或其观察操作违背定义域前提 | 直接报告非法观察/域前提错误，不当作普通真假 |

---

## 六、 下一篇预告

我们已经了解到，PTC 的外层处理布尔逻辑组合，内层处理类型领域的原子关系。那么，Flurry 中的类型相等、布局观察、结构分类、Normal-Form 头模式匹配、子类型转换、Trait 约束以及错误行包含关系，具体是如何由观察项（Terms）与原子命题（Atoms）组合而成的？

下一篇 **《Flurry 演进手记 (085)：PTC 的原子命题体系——从类型相等、结构分类到 Trait 满足性与行关系》**，我们将系统梳理这些类型原子命题。
