# Flurry 演进手记 (085)：PTC 的原子命题体系——从类型相等、子类型到 Trait 满足性与位宽关系

在上一篇（084）中，我们了解了 Flurry 类型系统的逻辑约束检验机制 **PTC（Propositional Type Constraints，命题类型约束）**，看到它是如何将外层的布尔逻辑组合（`and`、`or`、`not`）与内层的领域约束规则结合起来的。

无论是源码中的 `newtype` 身份区分、`Type.size_of(T) == 8`、类型模式匹配 `T match { Array<'u32, n> => ... }`、Trait 约束满足、编译期整数不等式，还是代数错误集合的包含关系，都可以归纳为五类基础的**原子约束命题（Atomic Propositions）**。

本篇我们将梳理“观察项（Terms）”与“原子关系命题（Atoms）”的正交设计，并按职责介绍这五大类编译期约束。

---

## 一、 核心设计：区分“观察项（Term）”与“原子关系命题（Atom）”

在了解各类原子命题之前，首先需要明确一个设计原则：为什么类型系统不需要单独为“类型大小相等”或“字段类型相等”各自定义一套专门的比较规则？

这是因为 Flurry 将**“计算出一个编译期值或类型的观察操作（Observation Terms）”**与**“判断两个对象满足何种关系的原子命题（Relational Atoms）”**进行了正交拆分：

```
  [ 表面源码表达式 ]
  Type.size_of(*InstructionOperands) == Type.size_of(usize)
  ─────────────────┬────────────────    ─────────┬─────────
                   │                             │
        观察结果项 Term_1 (usize)      观察结果项 Term_2 (usize)
                   └──────────────┬──────────────┘
                                  ▼
                [ 底层归约出的基础相等关系命题 ]
                同类型域相等比较：Term_1 == Term_2
```

具体而言：
- `Type.size_of(T)`、`Type.repr_of(T)`、提取结构体字段类型、获取编译期列表长度，都是产生数据的**观察项（Terms）**；
- 当这些项求出结果后，判断它们是否相等直接复用最基础的**相等关系命题（`==`）**；判断它们的大小顺序直接复用**整数或位向量序关系命题（`<=`）**。

---

## 二、 PTC 原子命题的五大板块

按职责划分，Flurry 的编译期原子命题可以分为以下五大板块（共 19 种基础原子关系，详见 `docs/type/propositional-constraints/atoms.md`）：

### 板块 1：相等性、类型身份与宇宙分类

这一组命题负责回答“两个类型或编译期值是否相等”以及“某个类型属于哪种分类、位于哪个宇宙层级”：

| 约束类别 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **同域相等性** | 在同一类型域下，判断两个编译期项或类型 `a` 与 `b` 是否严格相等 | `T == u32`、`Type.size_of(T) == 8`、`newtype` 与底层原类型不等（`!=`） |
| **结构分类判断** | 判断类型 `T` 是否属于特定的结构分类（如 `struct`、`enum`、`tuple`、`pointer`、`function` 等） | `Type.predicates.is_tuple(Itself)`、`Type.predicates.is_struct(T)` |
| **宇宙层级归属** | 判断类型 `T` 所属的宇宙层级（如运行期值宇宙 $U_0$、编译期元宇宙 $U_1$ 等） | 检查某类型能否落地为运行期存储，或仅限编译期使用 |
| **类型限定符检查** | 判断类型 `T` 是否携带特定的类型限定符或视图修饰（如 `const` 等） | 检查只读限定或特定修饰规则 |

### 板块 2：结构成员与模式匹配

这一组命题用于支撑结构反射（第 076 篇）与类型/编译期值模式匹配（第 083 篇）：

| 约束类别 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **结构槽位存在性** | 判断复合类型 `T` 是否拥有指定的结构槽位（具名字段、元组位置索引或枚举分支） | 结构化反射检查、具名字段存在性验证 |
| **Normal-Form 头形态** | 判断保留了 Normal-Form 头的类型或项，其最外层构造器头与元数形态是否匹配 | 检查某类型是否由 `Array<...>`、`Tuple<...>` 或 `Box<...>` 构造 |
| **模式匹配满足性** | 判断目标类型或编译期值 `x` 是否匹配模式 `p` | `T match { Array<'u32, length> => ... }`、`x is Pattern` |

### 板块 3：类型关系、Trait 契约与物理定长性

这一组命题连接了渐进类型兼容性（第 079 篇）、子类型转换、Trait 系统以及物理布局边界：

| 约束类别 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **渐进描述兼容** | 判断阶段化类型描述之间是否兼容（对称但不具有传递性） | 定义期检查 `Any` 或 `defer Pick(flag)` 的传递与赋值合法性 |
| **子类型关系** | 判断类型 `A` 是否为类型 `B` 的子类型（如 `never` 到任意类型、精化子类型到基础类型等） | 分支汇合、`never` 向上转型、精化类型窄化与宽化 |
| **Trait 实现满足性** | 在当前模块可见的一致性规则下，判断主体是否实现了指定 Trait 契约 | 泛型 `where T :- Clone`、`T :- Copy` 等 Trait 约束求解 |
| **非编译期定长判定** | 在目标平台布局配置下，判断类型 `T` 的物理大小是否依赖运行期值信息 | `Type.predicates.is_not_sized(T)`（其否定形式即为编译期定长） |

### 板块 4：编译期整数算术与定宽位向量关系

这一组命题支撑张量维度计算、步长对齐与底层定宽位操作：

| 约束类别 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **任意精度整数序** | 编译期数学整数（`Integer`）上的小于等于关系 $a \le b$（结合 `==` 与 `not` 可表达 `<, >, >=`） | `0 <= index and index + 1 < length` |
| **整数模同余关系** | 数学整数模同余关系 $a \equiv r \pmod m$（$m > 0$），用于整除与对齐检查 | 检查缓冲区大小或偏移量是否为 16 / 64 字节的整数倍 |
| **定宽无符号序** | 宽度为 `w` 比特的定宽位向量（`BitVec<w>` / 无符号整数）上的无符号序关系 $a \le_u b$ | `u8`、`u32`、`usize` 及 `BitVec<w>` 的无符号边界比较 |
| **定宽有符号序** | 宽度为 `w` 比特的定宽位向量（有符号整数）上的补码有符号序关系 $a \le_s b$ | `i8`、`i32`、`isize` 的有符号边界比较 |

### 板块 5：行集合关系与自定义谓词调用

最后一组命题处理行多态（如错误集 `!Err`）以及开发者自定义的 `comptime predicate`：

| 约束类别 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **行成员包含** | 判断结构行、枚举行或错误行集合 `R` 是否包含指定标签项 | 检查枚举或错误集中是否含有某成员 |
| **错误行子集** | 判断**错误行（Error Row）** `R` 是否为错误行 `S` 的子集 | 错误传播 `!SmallErr` 自动向上拓宽为 `!LargeErr` |
| **错误行不相交** | 判断两个**错误行** `R` 与 `S` 是否互不相交（无重叠分支） | 组合独立错误集或拆分处理部分错误分支 |
| **自定义谓词调用** | 调用开发者定义的 `comptime predicate` | 调用自定义谓词如 `valid_slot(index, limit)` |

> **说明：为什么错误行子集与不相交约束专门针对错误结果（`!Err`），而不直接套用于代数效应（`#Eff`）？**
> 错误结果（`!Err`）与局部代数效应（`#Eff`）具有不同的语义和 ABI：错误返回是值层面的联合体分派，天然支持错误集合的包含拓宽与不相交拆分；而代数效应是词法作用域内的能力协议，通过处理器证据传递（函数签名不添加 `handles` 子句），因此两者的类型约束规则是分离的。

---

## 三、 综合示例：在一段代码中组合多种编译期约束

下面我们通过一段指令集寄存器与操作数校验代码（参考 `docs/type/propositional-constraints/atoms.md`），观察类型相等、布局查询、结构分类、定长性判定与自定义算术谓词如何在同一段代码中协同工作：

```flurry
-- 透明别名 vs 独立名义身份 newtype
typealias Word = u32;
newtype RegisterId = Word;

-- 指令操作数结构体
struct InstructionOperands {
    dst: RegisterId,
    src: RegisterId,
}

-- 自定义编译期谓词（内部由两个整数序关系合取组成）
comptime predicate valid_slot(index: Integer, limit: Integer) {
    0 <= index and index < limit
}

test {
    let pair: Tuple<RegisterId, Word> = (RegisterId(1), 7u32);

    -- 1. 类型相等性与 newtype 名义身份区分
    assumes Word == u32;
    assumes RegisterId != Word;

    -- 2. 布局观察项 Type.size_of / Type.repr_of 的相等比较
    assumes Type.size_of(RegisterId) == Type.size_of(Word);
    assumes Type.repr_of(RegisterId) == Type.repr_of(Word);
    assumes Type.size_of(InstructionOperands) == 8;
    assumes Type.size_of(*InstructionOperands) == Type.size_of(usize);

    -- 3. 结构分类与编译期定长性检查
    assumes Type.predicates.is_tuple(pair'type);
    assumes not Type.predicates.is_not_sized(InstructionOperands);

    -- 4. 自定义谓词与编译期整数边界检查
    assumes valid_slot(2, 4);
    assumes not valid_slot(4, 4);
}
```

如果尝试将一个不满足 `valid_slot` 边界条件的常量赋给精化类型，编译期会直接拒绝：

```flurry
-- ❌ 编译期拒绝：valid_slot(4, 4) 不满足 `4 < 4`
comptime {
    let bad_slot: refines { valid_slot(itself, 4) } Integer = 4;
}
```

---

## 四、 本篇小结

| 原子命题板块 | 涵盖的核心能力 |
| :--- | :--- |
| **相等与宇宙分类** | 同域相等（`==`）、结构分类（`Type.predicates.is_*`）、宇宙层级归属、类型限定符 |
| **结构与模式匹配** | 结构槽位存在性、Normal-Form 头形态匹配、类型与编译期值模式匹配（`match` / `is`） |
| **关系、Trait 与定长** | 渐进兼容、子类型关系、Trait 满足性（`:-`）、编译期定长性判定 |
| **算术与位向量** | 任意精度整数序（`<=`）、整数模同余对齐、定宽无符号/有符号位向量序 |
| **行集合与谓词调用** | 行成员包含、错误行子集与不相交、自定义 `comptime predicate` 调用 |

---

## 五、 下一篇预告

我们已经了解了 PTC 支持的各类原子命题。在实际编写泛型或编译期逻辑时，我们通常不希望在每个局部位置都重复编写显式的类型转换或契约声明。当我们写下条件分支——例如：
- `inline if T is do { Array<'u32, length> => ... }`
- `inline if value > 0 { ... }`

进入分支后，编译器是如何自动将分支条件纳入当前作用域的已知事实，从而缩小类型或数值范围的？离开分支或遇到提前 `return` 时，这些局部事实又遵循怎样的作用域规则？

下一篇 **《Flurry 演进手记 (086)：分支中的流敏感类型——Occurrence Typing 与局部条件窄化》**，我们将探讨控制流与类型约束的结合。
