# Flurry 演进手记 (085)：PTC 的 19 类原子命题——从类型相等、子类型到 Trait 满足性与位宽关系

在上一回（084）中，我们认识了 Flurry 类型系统的逻辑推理引擎 **PTC（Propositional Type Constraints，命题类型约束）**，明白了它是如何将外层的布尔逻辑组合（`and`、`or`、`not`）与内层的领域理论（Domain Theories）分离开来的。

当时我们剧透了一个令所有语言设计爱好者心跳加速的事实：
无论你在源码里写的是 `newtype` 身份区分、`Type.size_of(T) == 8`、类型模式匹配 `T match { Array<'u32, n> => ... }`、Trait 约束满足、编译期整数不等式，还是代数错误集合的包含关系——当编译器把这些五花八门的表面语法消解到底层 PTC 引擎时，**全部归纳为仅仅 19 类不可再分的“原子命题（Atomic Propositions，`A01`–`A19`）”**！

今天，我们就来绘制一张完整的 **PTC 19 类原子命题全景图**，彻底看清“观察结果项（Terms）”与“原子关系命题（Atoms）”是如何像乐高积木一样，搭起整个 Flurry 静态类型大厦的！

---

## 一、 核心基石：先分清“观察项（Term）”与“原子命题（Atom）”！

在翻开 19 类原子命题名录之前，有一个极其关键的设计原则必须先讲透：
为什么在 19 类原子命题里，你找不到一个专门叫 `SizeEquals(T, 8)` 或者 `FieldTypeEquals(T, 0, u32)` 的命题？

因为 Flurry 做了一次极其漂亮的正交化提纯：**把“算出一个值/类型”的观察操作（Observation Terms），与“判断两个东西满足什么关系”的原子命题（Relational Atoms）彻底分开！**

```
  [ 表面源码表达式 ]
  Type.size_of(*InstructionOperands) == Type.size_of(usize)
  ─────────────────┬────────────────    ─────────┬─────────
                   │                             │
        观察结果项 Term_1 (usize)      观察结果项 Term_2 (usize)
                   └──────────────┬──────────────┘
                                  ▼
                [ 底层唯一需要的 PTC 原子命题 ]
                A01: Eq(sort = usize, Term_1, Term_2)！
```

看明白了吗？
- `Type.size_of(T)`、`Type.repr_of(T)`、提取结构体字段类型、获取编译期列表长度，这些都是产生数据的**项（Terms）**；
- 一旦这些项求出了结果，比对它们是否相等，只需要复用最基础的 **`A01: Eq(sort, a, b)`**；比对它们的大小，只需要复用 **`A12: IntLe(a, b)`**！
- 这种正交设计让 PTC 求解器的核心保持得极度精简，却能组合出无穷无尽的表达力。

> **🔍 语法小贴士：`A01`–`A19` 是编译器内部目录编号，不是源码关键字！**
> 在阅读官方文档 `docs/type/propositional-constraints/atoms.md` 和本篇表格时，你会看到 `A01: Eq`、`A10: Implements` 等编号。请注意：**这些是编译器内部 PTC 引擎的规范目录代号（Catalog IDs）**。在写 Flurry 源码时，你写的是自然的 `==`、`is`、`<=`、`Type.predicates.*` 和 `where` 子句，由编译器前端自动翻译为这 19 类原子命题！

---

## 二、 全景巡礼：PTC 的 19 类原子命题（`A01`–`A19`）大盘点

为了方便记忆与查阅，我们将这 19 类原子命题按职责划分为 **五大板块**：

### 板块 1：相等性、类型身份与宇宙分类（`A01`–`A04`）

这是类型系统最底层的“户籍管理系统”，负责回答“两个东西是不是同一个”以及“这个类型属于哪个物种、哪个宇宙”：

| 内部编号与签名 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **`A01: Eq(sort, a, b)`** | 在同一个良构分类 `sort` 下，判断两个编译期项或类型 `a` 与 `b` 是否严格相等 | `T == u32`、`Type.size_of(T) == 8`、`newtype` 与原类型不等（否定形式 `not Eq`） |
| **`A02: Kind(T, k)`** | 判断类型 `T` 是否属于特定的结构分类 `k`（如 `struct`、`enum`、`tuple`、`pointer`、`function` 等） | `Type.predicates.is_tuple(Itself)`、`Type.predicates.is_struct(T)` |
| **`A03: InUniverse(T, u)`** | 判断类型 `T` 是否居住在指定的类型宇宙层级 `u`（如运行期值宇宙 $U_0$、编译期元宇宙 $U_1$、命题宇宙等） | 检查某类型能否落地为运行期存储，或仅限编译期使用 |
| **`A04: Qualified(T, q)`** | 判断类型 `T` 是否携带特定的类型限定符或修饰属性 `q`（如 `const`、特定的视图/引用修饰等） | 检查只读限定或特定修饰规则 |

### 板块 2：结构成员与模式匹配（`A05`–`A07`）

这一组命题负责支撑我们在第 076 回学习的**结构反射**与第 083 回学习的**类型/值模式匹配**：

| 内部编号与签名 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **`A05: HasSlot(T, s)`** | 判断复合类型 `T` 是否拥有指定的结构槽位 `s`（如具名字段、按位置索引的元组槽位或枚举分支） | 结构化反射检查、具名字段存在性验证 |
| **`A06: HasHeadShape(n, h)`** | 判断一个保留了 Normal-Form 头的类型或项 `n`，其最外层构造器头与元数形态是否匹配 `h` | 检查某类型是否由 `Array<...>`、`Tuple<...>` 或 `Box<...>` 构造 |
| **`A07: Matches(d, x, p)`** | 在描述域 `d` 下，判断目标类型或编译期值 `x` 是否匹配模式 `p` | `T match { Array<'u32, length> => ... }`、`x is Pattern` |

### 板块 3：类型关系、Trait 契约与物理定长性（`A08`–`A11`）

这一组命题连接了渐进类型兼容性（079 回）、子类型转换、Trait 系统（第 2 卷）以及物理布局边界（076 回）：

| 内部编号与签名 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **`A08: Compatible(d, a, b)`** | 在渐进类型描述域 `d` 下，判断 `a` 与 `b` 是否满足编译期阶段化描述兼容（注意：对称但**不传递**！） | 定义期检查 `Any` 或 `defer Pick(flag)` 的传递与赋值合法性 |
| **`A09: Subtype(A, B)`** | 判断类型 `A` 是否为类型 `B` 的子类型（如 `never` 到任意类型、精化子类型到基础类型等） | 分支汇合、`never` 向上转型、精化类型窄化与宽化 |
| **`A10: Implements(x, t, world)`** | 在当前可见模块世界 `world` 的一致性（Coherence）规则下，判断主体 `x` 是否实现了 Trait 契约 `t` | 泛型 `where T: Clone`、`T: Copy` 等 Trait 约束求解 |
| **`A11: NotSized(T, target)`** | 在目标平台布局配置 `target` 下，判断类型 `T` 的物理大小是否依赖运行期值信息（即非编译期定长） | `Type.predicates.is_not_sized(T)`（否定形式即为编译期定长 `Sized`） |

### 板块 4：编译期整数算术与定宽位向量关系（`A12`–`A15`）

这是张量维度计算、步长对齐与底层位操作（DMA 寄存器掩码）最依赖的数学引擎：

| 内部编号与签名 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **`A12: IntLe(a, b)`** | 任意精度编译期数学整数（`Integer`）上的小于等于关系 $a \le b$（配合 `Eq` 与 `not` 可表达 `<, >, >=`） | `0 <= index and index + 1 < length` |
| **`A13: IntCongruent(a, r, m)`** | 数学整数模同余关系 $a \equiv r \pmod m$（要求模数 $m > 0$），用于步长整除与对齐检查 | 检查 DMA 缓冲区大小或偏移量是否为 16 / 64 字节的整数倍 |
| **`A14: BVUnsignedLe(a, b, w)`** | 宽度为 `w` 比特的定宽位向量（BitVec / 机器整数）上的**无符号**小于等于关系 $a \le_u b$ | `u8`、`u32`、`usize` 固定位宽下的无符号边界比较 |
| **`A15: BVSignedLe(a, b, w)`** | 宽度为 `w` 比特的定宽位向量上的**有符号（补码）**小于等于关系 $a \le_s b$ | `i8`、`i32`、`isize` 固定位宽下的有符号边界比较 |

### 板块 5：错误/结构行集合与自定义谓词调用（`A16`–`A19`）

最后四块原石负责处理行多态（Row Polymorphism，如错误集 `!Err`）以及开发者自定义的 `comptime predicate`：

| 内部编号与签名 | 核心职责与语义说明 | 对应的典型源码场景 |
| :--- | :--- | :--- |
| **`A16: RowContains(k, R, l)`** | 在行分类 `k` 下，判断行集合 `R` 是否包含指定标签项 `l` | 检查结构行或错误行中是否含有某成员 |
| **`A17: RowSubset(error, R, S)`** | 判断**错误行（Error Row）** `R` 是否为错误行 `S` 的子集（即 `R` 抛出的所有错误都在 `S` 声明范围内） | 错误传播 `!SmallErr` 自动向上拓宽为 `!LargeErr` |
| **`A18: RowDisjoint(error, R, S)`** | 判断两个**错误行** `R` 与 `S` 是否互不相交（无重叠错误分支），确保合并与穷尽匹配无歧义 | 组合独立错误集或拆分处理部分错误分支 |
| **`A19: PredicateCall(def, args, deps)`** | 调用开发者定义的 `comptime predicate`（携带谓词定义 `def`、实参 `args` 与环境依赖 `deps`） | 调用自定义谓词如 `valid_slot(index, limit)` |

> **💡 核心心智模型：为什么 `A17: RowSubset` 和 `A18: RowDisjoint` 明确限定在 `error`（错误行），而不乱套在代数效应（Effect）上？**
> 回想 `AGENTS.md` §5 的语义红线与我们在第 2 卷学过的知识：**错误结果（`!Err`）与局部代数效应（`#Eff`）具有完全不同的语义和 ABI**！错误返回是值层面的联合体分派，天然满足集合包含（子集自动拓宽）与不相交拆分；而代数效应是词法作用域内的能力协议（Capability Protocol），通过隐式处理器证据传递，函数签名不添加 `handles` 子句，因此绝不能把效应混同于简单的静态错误标签子集！

---

## 三、 综合实战：在一段代码里同时唤醒多类 PTC 原子命题！

光看表格还不够过瘾，让我们写一段真实的指令集寄存器与操作数校验代码（源自 `docs/type/propositional-constraints/atoms.md`），看看短短二十几行 Flurry 代码是如何在底层同时驱动 `A01`（相等）、`A02`（结构分类）、`A11`（定长性）、`A12`（整数不等式）与 `A19`（谓词调用）协同工作的：

```flurry
-- 透明别名 vs 独立名义身份 newtype
typealias Word = u32;
newtype RegisterId = Word;

-- 指令操作数结构体
struct InstructionOperands {
    dst: RegisterId,
    src: RegisterId,
}

-- 自定义编译期谓词（对应 A19: PredicateCall，内部展开为两个 A12: IntLe 的合取）
comptime predicate valid_slot(index: Integer, limit: Integer) {
    0 <= index and index < limit
}

test "PTC atomic propositions working in concert" {
    let pair: Tuple<RegisterId, Word> = (RegisterId(1), 7u32);

    -- 1. 触发 A01: Eq(Type, ...) 与 not A01: Eq(Type, ...)
    assert(Word == u32);
    assert(RegisterId != Word);

    -- 2. 观察项 Type.size_of / Type.repr_of 结合 A01: Eq
    assert(Type.size_of(RegisterId) == Type.size_of(Word));
    assert(Type.repr_of(RegisterId) == Type.repr_of(Word));
    assert(Type.size_of(InstructionOperands) == 8);
    assert(Type.size_of(*InstructionOperands) == Type.size_of(usize));

    -- 3. 触发 A02: Kind(T, tuple) 与 not A11: NotSized(T, target)
    assert(Type.predicates.is_tuple(pair'type));
    assert(not Type.predicates.is_not_sized(InstructionOperands));

    -- 4. 触发 A19: PredicateCall 与 A12: IntLe
    assert(valid_slot(2, 4));
    assert(not valid_slot(4, 4));
}
```

如果有人试图把一个违反 `A19` / `A12`（下标越界）或者违反 `A01`（把 `Word` 直接当 `RegisterId`）的表达式塞进精化契约：

```flurry
-- ❌ 编译期直接拒绝！valid_slot(4, 4) 展开后的原子命题 A12: IntLe(4 + 1, 4) 为假
comptime {
    let bad_slot: refines { valid_slot(itself, 4) } Integer = 4;
}
```

---

## 四、 本篇小结

| 原子命题板块 | 编号范围 | 涵盖的核心能力 |
| :--- | :--- | :--- |
| **相等与宇宙分类** | `A01`–`A04` | `Eq`（同域相等）、`Kind`（类型分类）、`InUniverse`（宇宙归属）、`Qualified`（限定符） |
| **结构与模式匹配** | `A05`–`A07` | `HasSlot`（槽位存在）、`HasHeadShape`（Normal-Form 头形态）、`Matches`（模式匹配） |
| **关系、Trait 与定长** | `A08`–`A11` | `Compatible`（渐进兼容）、`Subtype`（子类型）、`Implements`（Trait 满足）、`NotSized`（运行期相关大小） |
| **算术与位向量** | `A12`–`A15` | `IntLe`（整数序）、`IntCongruent`（模同余）、`BVUnsignedLe` / `BVSignedLe`（位宽无符号/有符号序） |
| **行集合与谓词调用** | `A16`–`A19` | `RowContains`、`RowSubset`（错误行子集）、`RowDisjoint`（错误行不相交）、`PredicateCall` |

---

## 五、 留给下一篇的悬念

现在，我们已经集齐了 PTC 的 19 类原子命题，知道了编译器有一颗强大的逻辑推理大脑。

但是，作为每天写业务和底层库代码的程序员，我们肯定不想在每一行代码里都手动去写冗长的 `refines { ... }` 注解！
最理想的情况是：当我们写下一个普通的条件分支——比如：
- `if maybe_val is (x: u32) { ... }`
- 或者 `if x > 0 { ... }`
- 或者对一个宽泛的类型 `T` 走过了 `if Type.predicates.is_tuple(T) { ... }` 的分支门禁，

**一旦代码走进了 `then` 分支的大括号里面，或者走到了 `else` 分支里面，那个原本宽泛的变量或类型的静态类型，能不能顺着控制流“自动变窄、自动变精确”**？

下一回 **《Flurry 演进手记 (086)：走进分支后类型自动变精确了——Occurrence Typing（出现类型推导）的奥秘》**，我们将揭秘控制流与 PTC 命题引擎是如何擦出火花的！
