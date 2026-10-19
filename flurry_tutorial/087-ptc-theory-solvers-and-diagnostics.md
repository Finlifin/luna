# 087 — PTC 幕后大脑：十一个理论求解器如何协同破案？

> **阅读指南**：在上一回（第 086 回）中，我们看到分支条件 `value > 0` 和类型模式 `Array<'u32, length>` 能够摇身一变，成为当前分支里的局部已知事实。但当我们的函数契约越来越丰富——比如同时涉及“类型相等或分支选择（`T == u32 or T == f32`）”、“数组维度算术（`N + 1 > 0`）”、“结构体布局大小（`Type.size_of(T)`）”以及“Trait 实现约束”时，编译器究竟是如何把这些风马牛不相及的领域知识串联起来做逻辑推理的？如果推不出来，它又凭什么给出精准的反例诊断而不是一句冷冰冰的“Type Error”？本篇我们将走进 **命题类型检查器（Propositional Type Checker, PTC）** 的幕后大脑。

---

## 一、 痛点：当类型检查遇到“跨学科综合题”

在传统编程语言里，类型检查器通常只会干一件事：**结构合一（Structural Unification）**——比对两棵类型语法树长得一模一样就算通过，不一样就报错。

但在 Flurry 这样一门支持精化类型（`refines`）、类型级前置/后置契约（`requires` / `ensures`）以及静态维度计算的系统语言里，编译器每天面对的都是“跨学科综合题”。

### 尝试 `v0`：单打独斗的检查器如何败下阵来？

设想我们在编写一个底层序列化器，它只支持 32 位整数 `u32` 或 32 位浮点数 `f32`。我们用一个枚举 `StorageKind` 来在编译期映射出对应的存储类型，然后交给 `supported_size` 去查询它的字节大小：

```flurry
enum StorageKind {
    integer,
    real,
}

-- 根据枚举返回对应的类型，并承诺返回值一定是 u32 或 f32 之一
pure comptime fn storage_type(kind: StorageKind) -> result: Type
where
    ensures result == u32 or result == f32
{
    kind match {
        .integer => u32,
        .real => f32,
    }
}

-- 要求传入的 T 必须是 u32 或 f32，然后查询其字节大小
pure comptime fn supported_size(T: Type) -> usize
where
    requires T == u32 or T == f32
{
    Type.size_of(T)
}
```

看看这里发生了什么：
1. 在检查 `storage_type` 的函数体时，编译器需要结合 **模式匹配穷尽性**、**分支条件传播**、**类型相等性** 以及 **布尔析取（`or`）**，才能证明两个分支的返回值都满足 `ensures result == u32 or result == f32`。
2. 在检查 `supported_size` 时，编译器需要知道：虽然 `T` 是一个抽象类型参数，但因为 `T == u32 or T == f32` 保证了无论在哪种分支下 `T` 都是已知布局的具体标量类型（`Sized`），所以 `Type.size_of(T)` 是合法且有定义的！

如果编译器里只有一堆硬编码的 `if-else` 规则，一旦把“布尔逻辑”、“类型相等”、“整数线性算术”和“内存布局”混在一起，规则数量就会指数级爆炸，不仅推理能力极其脆弱，报错时也根本说不清到底是哪个子条件失败了。

---

## 二、 破局（v1）：CDCL(T) 警长与十一位专职神探

为了彻底解决跨领域的编译期推理问题，Flurry 将类型与契约检查建立在一套严密的 **SMT 风格多理论协同架构——CDCL(T) / DPLL(T) + Nelson-Oppen 等式共享** 之上。

> **⚠️ 严守语义红线：PTC 只管“类型命题”，不管“运行时内存状态”！**
> 在进入架构图之前，请牢记一条 Flurry 的核心设计边界（见 `AGENTS.md` §5 与 `docs/propositional-constraints/index.md`）：
> - **PTC（命题类型检查器）** 只处理**编译期与类型层面的命题**（如类型相等、维度整数关系、位宽、布局可计算性、行标签、Trait 证据）；
> - 而运行时的指针别名、内存视图互不相交（Disjointness）、资源移动与运行时 `assumes` 事实失效，属于我们在第二卷学过的 **程序验证层（Program Verification）** 的职责。两者边界清晰，绝不混为一谈！

在 PTC 内部，有一个负责处理布尔逻辑骨架（`and`、`or`、`not`、分支蕴含）的 **SAT/CDCL 核心警长**，以及 **11 个各司其职的理论求解器（Theory Solvers）**：

```text
                    ┌──────────────────────────────────────────┐
                    │       PTC 核心警长：CDCL(T) 布尔引擎       │
                    │  负责拆解 and / or / not / 分支激活守卫 α  │
                    └────────────────────┬─────────────────────┘
                                         │ 共享变量等式 (Nelson-Oppen: x == y)
                                         │ 下发原子命题 & 收集冲突子句 (Conflict Lemma)
       ┌──────────────┬──────────────┬───┴──────────┬──────────────┬──────────────┐
       ▼              ▼              ▼              ▼              ▼              ▼
  ┌─────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐
  │ 1. EUF  │   │ 2. TYPE  │   │3.MATCH/  │   │ 4. TRAIT │   │5. LAYOUT │   │  6. INT  │
  │未解释函数│   │类型分类与│   │   REL    │   │Trait 证据│   │大小/对齐/│   │差分逻辑与│
  │与等式闭包│   │ 修饰符   │   │模式与兼容│   │与一致性  │   │递归环检测│   │线性整数  │
  └─────────┘   └──────────┘   └──────────┘   └──────────┘   └──────────┘   └──────────┘
       ▲              ▲              ▲              ▲              ▲              ▲
       └──────────────┴──────────────┼───┬──────────┴──────────────┴──────────────┘
                                     ▼   ▼
       ┌──────────────┬──────────────┬───┴──────────┬──────────────┐
       ▼              ▼              ▼              ▼              ▼
  ┌─────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐
  │ 7. SEQ  │   │  8. ROW  │   │  9. BV   │   │ 10. PRED │   │11. 量化层│
  │有限序列与│   │错误行与  │   │定宽位向量│   │可执行编译│   │forall 与 │
  │张量形状 │   │代数效应行│   │与掩码相交│   │期纯谓词  │   │ exists   │
  └─────────┘   └──────────┘   └──────────┘   └──────────┘   └──────────┘
```

让我们快速认识一下这 11 位神探各自的拿手绝活与严格红线：

1. **`EUF`（相等与未解释函数闭包）**：维护 `a == b ⟹ f(a) == f(b)` 的传递性与同余闭包。**铁律**：Normal-form 类型头（如 `Identity<T>`、`Tuple<...>`）绝不在 `EUF` 中强行展开函数体！`Identity<T>` 永远不因函数体返回 `T` 就直接与 `T` 画等号。
2. **`TYPE`（类型分类与结构观察）**：判断某个对象是不是合法类型、带有什么修饰符与构造器头部。
3. **`MATCH/REL`（模式匹配、兼容性与子类型）**：处理 `is` / `matches` 与类型兼容关系 `≈`。**注意**：类型兼容 `≈` **不满足传递性**！`A ≈ Any` 且 `Any ≈ B` 都成立，但绝不能因此把 `A` 和 `B` 合并为同一个类型！
4. **`TRAIT`（Trait 证据与一致性）**：查找 `impl`、验证孤儿规则与唯一性。
5. **`LAYOUT`（内存表示与布局图）**：分析 `Sized`、对齐和字段偏移。它知道指针 `*T` 的宽度永远等于 `usize`（哪怕 `T` 的内部布局还没算出来）；但如果一个结构体按值直接或间接包含自己导致无限大小，它会精准揪出布局环（Layout Cycle）。
6. **`INT`（整数算术）**：结合 Bellman-Ford 负环检测处理差分逻辑（$a - b \le c$）与线性整数算术（含分支定界/割平面）。它只承认**整数解**——遇到 $2x = 1$ 时，它会直接判定整数域无解（冲突），绝不会捏造出有理数 $x = 0.5$！
7. **`SEQ`（有限序列与张量形状）**：处理编译期列表 `meta.List`、形状拼接与维度对齐。
8. **`ROW`（行多态求解器）**：专门处理错误集合行 `![E1, E2]` 与代数效应行 `#[eff1, eff2]`（我们将在下一回详细拆解！）。
9. **`BV`（定宽位向量）**：处理 `BitVec<N>`、位切片与带掩码模式的相交判定（例如通过 `((v1 XOR v2) AND (m1 AND m2)) != 0` 快速判断两个位模式互不相交）。
10. **`PRED`（可执行编译期谓词）**：当某个用户自定义的 `comptime predicate` 所有参数都已经变成已知常量时，直接调用编译期求值器算出 `true` 或 `false`。
11. **`Quantifier`（全称与存在量化层）**：基于触发器（Trigger）实例化 `forall` 并做 Skolem 化。

### 演进到 `v1`：多求解器完美配合的正例

当我们运行刚才写的 `storage_type` 与 `supported_size` 时，`CDCL(T)` 警长与 `EUF`、`TYPE`、`LAYOUT` 瞬间完成了接力验证：

```flurry
use std.prelude.*;

enum StorageKind {
    integer,
    real,
}

pure comptime fn storage_type(kind: StorageKind) -> result: Type
where
    ensures result == u32 or result == f32
{
    kind match {
        .integer => u32,
        .real => f32,
    }
}

pure comptime fn supported_size(T: Type) -> usize
where
    requires T == u32 or T == f32
{
    Type.size_of(T)
}

const SELECTED_TYPE = storage_type(.integer);
const SELECTED_SIZE = supported_size(SELECTED_TYPE);

test {
    println("is u32: {}", SELECTED_TYPE == u32);
    println("size: {}", SELECTED_SIZE);
}
```

---

## 三、 四种裁决结果与精准反例诊断（v2）

每次编译器向 PTC 提出一个查询 $\Gamma \vdash Q$（“在当前环境 $\Gamma$ 下，目标命题 $Q$ 是否成立？”），PTC 严格只返回以下 **四种契约结果（Query Outcomes）** 之一：

```text
                       查询：Γ ⊢ Q (目标命题 Q 是否成立？)
                                      │
          ┌──────────────────┬────────┴─────────┬──────────────────┐
          ▼                  ▼                  ▼                  ▼
   ┌─────────────┐    ┌──────────────┐   ┌─────────────┐    ┌─────────────┐
   │ 1. Entailed │    │2. NotEntailed│   │ 3. Unknown  │    │4. IllFormed │
   ├─────────────┤    ├──────────────┤   ├─────────────┤    ├─────────────┤
   │ Γ ∧ ¬Q 冲突 │    │ 找到了合法的 │   │ 算力耗尽或  │    │ 命题本身包含│
   │ 证明 Q 必然 │    │ 反例 Witness!│   │ 信息尚不完整│    │ 非法类型或  │
   │ 100% 成立！ │    │ 立即报错并展 │   │ 保留残余约束│    │ 按值无限递归│
   └─────────────┘    │ 示具体反例！ │   │ 或按需拒绝  │    └─────────────┘
                      └──────────────┘   └─────────────┘
```

> **💡 核心心智模型：`NotEntailed` 与 `Unknown` 的本质区别**
> - **`NotEntailed`（明确反驳）**：求解器不仅没法证明 $Q$，而且实实在在地构造出了一个**满足所有已知前提 $\Gamma$、却让 $Q$ 为假的合法源语言反例（Valid Source Witness）**！注意：反例必须通过所有相关理论求解器的联合审查（例如不能拿 $x = 0.5$ 当整数反例，也不能拿未展开的 `Identity<u32> != Identity<u32>` 当反例）。一旦拿到真反例，编译器会直接把反例塞进报错信息里给你看！
> - **`Unknown`（暂未决断）**：可能是因为泛型参数还没实例化（在 `loose` 定义阶段允许保留为残余约束留待实例化时再证），也可能是因为非线性算术超出了求解器预算。在要求立即证明的普通调用点，`Unknown` 同样会被安全地拒绝，但诊断信息会诚实地告诉你“无法自动证明该约束”，而不会编造假反例。
> - **`IllFormed`（良构性错误）**：命题本身就病了！比如把一个 `bool` 返回值塞给了要求 `Type` 的地方，或者写出了按值直接包含自身的无限大小结构体。

### 演进到 `v2`：看 PTC 如何拦截 `NotEntailed` 与 `IllFormed`

让我们看两个典型的错误案例，体会 PTC 是如何分别给出精准打击的：

```flurry
enum StorageKind {
    integer,
    real,
}

-- 负例 1：返回值类型根本不是 Type，且违背 ensures 契约
pure comptime fn broken_storage_type(kind: StorageKind) -> result: Type
where
    ensures result == u32 or result == f32
{
    -- ❌ 编译期直接拒绝！
    -- TYPE 求解器立即捕获：false 的类型是 bool，不是声明的返回类型 Type！
    -- 同时 EUF 与 CDCL(T) 也会指出 result == false 违背了 ensures 契约。
    false
}

-- 负例 2：LAYOUT 求解器捕获按值无限递归（IllFormed Layout Cycle）
struct InfiniteBox {
    header: u32,
    -- ❌ 编译期直接拒绝！(IllFormed: 布局依赖环)
    -- 如果不加指针 *，InfiniteBox 按值直接包含 InfiniteBox，
    -- LAYOUT 求解器在表示依赖图中检测到强连通环，判定无法形成有限字节大小！
    inner: InfiniteBox,
}
```

如果把 `inner: InfiniteBox` 改成 `inner: ?*InfiniteBox`，`LAYOUT` 求解器就会立刻放行——因为指针 `*InfiniteBox` 的大小固定等于 `usize`，斩断了按值包含的布局依赖环！

---

## 本篇小结

| 核心组件 / 结果 | 职责与边界 | 关键防呆红线 |
| :--- | :--- | :--- |
| **PTC 职责边界** | 仅求解编译期与类型层面的命题 | 绝不越界接管运行时内存状态与别名分析（由程序验证层负责） |
| **CDCL(T) + 11 求解器** | `EUF`, `TYPE`, `MATCH/REL`, `TRAIT`, `LAYOUT`, `INT`, `SEQ`, `ROW`, `BV`, `PRED`, `Quantifier` | 通过 Nelson-Oppen 共享变量等式协同推理；`Identity<T>` 等 normal-form 头绝不自动展开；`≈` 不传递 |
| **`Entailed` / `NotEntailed`** | 必然成立 / 找到合法反例证伪 | 反例必须满足全部 11 个理论域的合法性约束，绝不产生伪反例 |
| **`Unknown` / `IllFormed`** | 证据不足或预算耗尽 / 命题或布局本身非法 | 在 `loose` 阶段可冻结残余约束；在需要立即证明处安全拒绝 |

---

### 留给下一篇的悬念

在这 11 位神探的名单里，第 8 号神探 **`ROW`（行多态求解器）** 显得格外神秘。我们在前面的章节里已经见过错误联合类型 `![IoError, ParseError] u32` 和代数效应类型 `#[read_number, log] u32`。

表面上看，它们都是在方括号里写一串标签，但为什么在 `ROW` 求解器眼里：
- 把两个相同的错误叠在一起 `![InputError] ![InputError] u32`，会自动合并成 `![InputError] u32`；
- 而把两个相同的代数效应叠在一起 `#[read_number] #[read_number] u32`，却**绝不等于**单层的 `#[read_number] u32`？

下一篇 **《088 — 行多态（Row Polymorphism）：错误集合与代数效应的本质差异》**，我们将深入 `ROW` 求解器的心脏，揭开“集合行（Unique-Label Rows）”与“作用域多重集行（Scoped-Label Rows）”的数学之美！
