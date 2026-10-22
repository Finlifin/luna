# Flurry 演进手记 (085)：PTC 的原子命题体系——从类型相等、结构分类到 Trait 满足性与行关系

在上一篇（084）中，我们了解了 Flurry 类型系统的逻辑约束检验机制 **PTC（Propositional Type Constraints，命题类型约束）**，看到它是如何将外层的布尔逻辑组合（`and`、`or`、`not`）与内层的类型领域规则结合起来的。

无论是源码中的 `newtype` 身份区分、`Type.size_of(*T) == Type.size_of(usize)`、结构分类查询 `Type.predicates.is_tuple(T)`、类型模式匹配 `T match { Array<'u32, n> => ... }`、Trait 约束满足，还是错误行的包含关系，都可以归纳为基础的**类型观察项（Terms）**与**原子关系命题（Atomic Propositions）**。

本篇我们将梳理“观察项（Terms）”与“原子关系命题（Atoms）”的正交设计，并按职责介绍这些类型约束板块。

---

## 一、 核心设计：区分“观察项（Term）”与“原子关系命题（Atom）”

为什么类型系统无需单独为“类型大小相等”或“字段类型相等”各自定义一套专门的相等规则？

这是因为 Flurry 将**“在满足前提下观察类型的某项属性（Observation Terms）”**与**“判断两个对象满足何种关系的原子命题（Relational Atoms）”**进行了正交拆分：

```text
  [ 表面源码表达式 ]
  Type.size_of(*InstructionOperands) == Type.size_of(usize)
  ─────────────────┬────────────────    ─────────┬─────────
                   │                             │
        观察结果项 Term_1 (usize)      观察结果项 Term_2 (usize)
                   └──────────────┬──────────────┘
                                  ▼
                [ 底层归约出的基础相等关系命题 ]
                同域相等比较：Term_1 == Term_2
```

具体而言：
- `Type.size_of(T)`、`Type.align_of(T)`、提取结构体字段类型、获取限定类型的载荷类型，都是带良构前提的**观察项（Terms）**；
- 当这些项在合法定义域内形成后，判断它们是否相等直接复用最基础的**同域相等关系命题（`==`）**。

---

## 二、 PTC 类型原子命题的核心板块

按职责划分，Flurry 的 PTC 原子命题围绕类型检查所需的关系展开（详见 `docs/type/propositional-constraints/atoms.md`）：

### 板块 1：相等性、类型身份与宇宙分类

这一组命题负责回答“两个类型或观察项是否相等”以及“某个类型属于哪种结构分类、位于哪个宇宙层级”：

| 约束类别 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **同域相等性** | 在同一逻辑域下，判断两个类型或观察项 `a` 与 `b` 是否语义相等 | `Word == u32`、`RegisterId != u32`、`Type.size_of(*T) == Type.size_of(usize)` |
| **类型结构分类** | 判断类型 `T` 是否属于已定义的类型分类（如 `tuple`、`struct`、`enum`、`pointer` 等） | `Type.predicates.is_tuple(Tuple<u32, u32>)` |
| **宇宙层级归属** | 判断类型 `T` 所属的宇宙层级（如运行期值宇宙 $U_0$、编译期元宇宙 $U_1$） | 检查某类型能否作为运行期存储类型或仅限编译期使用 |
| **限定类型检查** | 判断类型 `T` 是否具有指定的限定修饰（如 optional `?`、error `!`、effect `#`、closure `%`、reachability `&`） | 检查限定类型结构并提取其内部载荷类型 |

### 板块 2：结构槽位与类型模式匹配

这一组命题用于支撑结构成员检查与类型模式匹配（第 083 篇）：

| 约束类别 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **结构槽位存在性** | 判断类型 `T` 是否存在指定的字段、元组位置或枚举变体槽位 | 验证成员投影与解构操作的结构前提 |
| **Normal-Form 头形态** | 判断保留了 Normal-Form 头的类型对象是否具有指定的构造来源头 | 检查某类型是否由 `Array<...>`、`Tuple<...>` 或 `Identity<...>` 构造 |
| **模式匹配满足性** | 判断受检对象 `x` 是否匹配模式 `p`，并在成功时产出局部绑定证据 | `T is Array<'u32, length>`、`T matches ...` |

### 板块 3：类型关系、Trait 契约与固定大小表示

这一组命题连接了渐进类型相容性（第 079 篇）、子类型关系、Trait 系统以及物理表示定长性：

| 约束类别 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **部分信息相容** | 判断含未知部分的类型描述（如 `Any`）之间是否相容（缺乏传递性） | 定义期检查含 `Any` 或未决描述的接口相容性 |
| **方向性子类型** | 判断类型 `A` 是否为类型 `B` 的子类型（不从一般类型转换自动推导） | `never` 转型、精化类型到基础类型的子类型关系 |
| **Trait 实现满足性** | 在合法实现环境下，判断主体是否满足指定的 Trait 实例 | 泛型 `where T :- Copy`、`T :- Clone` 等约束检查 |
| **值相关大小判定** | 在目标配置下，判断合法表示类型 `T` 的大小是否还依赖运行期值信息 | `not Type.predicates.is_not_sized(T)`（表示具有编译期固定大小） |

### 板块 4：行集合关系与类型谓词调用

这一组命题处理行多态（如错误行 `!Err` 与效应行 `#Eff`）以及开发者定义的 `comptime predicate`：

| 约束类别 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **行成员包含** | 判断错误行或效应行 `R` 是否包含指定标签 `l` | 检查错误集或效应行中是否含有某成员 |
| **错误行子集与彼此分离** | 判断唯一错误行 `R` 是否为错误行 `S` 的子集，或两错误行是否无共同标签 | 错误传播 `!SmallErr` 拓宽为 `!LargeErr`，以及错误集拆分处理 |
| **可执行类型谓词** | 调用稳定的 `comptime predicate` 并按定义展开或执行 | 调用自定义类型谓词如 `is_supported_element(T)` |

> **说明：为什么行子集关系专门针对错误行（`!Err`），而不直接作为效应行（`#Eff`）的通用子集公理？**
> 错误结果（`!Err`）中的标签集合是唯一标签集，天然支持子集包含与彼此分离拆分；而代数效应（`#Eff`）采用词法作用域标签路由，同名效应标签的嵌套出现次序（Occurrence）决定了捕获到哪一层 Handler，因此效应嵌入必须保留具体的路由证据，无法只靠一个布尔子集结论丢掉路由信息。

---

## 三、 综合示例：在一段代码中观察多类类型命题

下面我们通过一段数据声明与编译期检查代码（源自 `docs/type/propositional-constraints/atoms.md`），观察类型等式、`newtype` 身份区分、结构分类、固定大小性质以及指针布局观察如何在同一段代码中工作：

```flurry
use std.prelude.*;

typealias Word = u32;
newtype RegisterId = u32;

struct InstructionOperands {
    destination: RegisterId,
    sources: Tuple<u32, u32>,
}

comptime predicate is_word_or_float(T: Type) {
    T == u32 or T == f32
}

comptime {
    -- 1. 类型相等与 newtype 名义身份区分
    assumes Word == u32;
    assumes RegisterId != u32;

    -- 2. 类型结构分类查询
    assumes Type.predicates.is_tuple(Tuple<u32, u32>);
    assumes not Type.predicates.is_tuple(u32);

    -- 3. 固定大小表示性质与指针大小观察
    assumes not Type.predicates.is_not_sized(InstructionOperands);
    assumes Type.size_of(*InstructionOperands) == Type.size_of(usize);

    -- 4. 自定义类型谓词
    assumes is_word_or_float(Word);
    assumes not is_word_or_float(RegisterId);
}

test {
    let operands = InstructionOperands {
        destination: 1u32.as(RegisterId),
        sources: (2u32, 3u32),
    }
    let (left, right) = operands.sources;
    println("sources: {}, {}", left, right);
    println("operand storage: {}", Type.size_of(InstructionOperands));
}
```

需要注意：如果将布局观察应用到无从适用的纯编译期类型（如 `Type` 自身），编译器会先报告观察前提错误（IllFormed），而不会用 `is_not_sized` 的真或假掩盖前提错误。

---

## 四、 本篇小结

| 类型命题板块 | 涵盖的核心能力 |
| :--- | :--- |
| **相等与分类** | 同域相等（`==`）、类型分类（`Type.predicates.is_*`）、宇宙归属、限定类型观察 |
| **结构与类型模式** | 槽位存在性、Normal-Form 头形态观察、类型模式匹配（`is` / `matches`） |
| **关系、Trait 与定长** | 渐进相容、方向性子类型、Trait 实现满足性（`:-`）、固定大小表示判定（`is_not_sized`） |
| **行集合与类型谓词** | 行成员包含、错误行子集与彼此分离、自定义 `comptime predicate` |

---

## 五、 下一篇预告

在实际编写采用 `ordinary` 检查的运行时泛型函数（`fn`）时，我们经常会使用 `inline if` 对类型条件进行分支判断，例如：
- `inline if T == u32 { ... }`
- `inline if T is do { Array<'u32, length> => ... }`

为什么在运行时函数中需要使用 `inline if`，而在 `comptime fn` 中直接使用普通控制流？进入 `inline if` 分支后，PTC 是如何把分支上的类型条件转化为当前分支的局部类型事实的？离开分支或遇到提前 `return` 时，这些局部类型事实又遵循怎样的作用域规则？

下一篇 **《Flurry 演进手记 (086)：分支中的流敏感类型——Occurrence Typing 与局部类型命题窄化》**，我们将探讨控制流与类型命题的结合。
