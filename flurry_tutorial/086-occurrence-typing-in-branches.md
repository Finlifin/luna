# Flurry 演进手记 (086)：分支中的流敏感类型——Occurrence Typing 与局部条件窄化

> **阅读指南**：在上一篇（085）中，我们梳理了 Flurry 编译期类型约束（PTC）的五大类原子命题。在实际编写编译期函数或泛型逻辑时，我们经常会写出 `inline if T is do { Array<'u32, length> => ... }` 或 `inline if value > 0 { ... }` 这样的分支代码。为什么进入 `then` 分支后，原本未知范围的 `value` 就可以直接传给要求正数的函数，或者直接提取出 `Array` 的长度参数 `length`？而在离开分支后，这些局部事实又为什么不会泄漏到外部？本篇我们将介绍 **出现类型（Occurrence Typing / 流敏感类型窄化）** 与分支条件的作用域规则。

---

## 一、 问题背景：流不敏感检查的局限

假设我们需要编写两个编译期辅助函数：
1. 一个函数要求传入的编译期整数 `value` 必须大于 `0`（由 `requires value > 0` 契约约束）；
2. 另一个函数接收任意类型 `T: Type`，如果 `T` 是元素为 `u32` 的静态数组 `Array<u32, length>`，则返回其长度 `length`；否则返回 `null`。

如果类型检查是“流不敏感（Flow-Insensitive）”的——即一个变量从声明起，其类型和已知约束在整个函数体内保持固定不变——那么即使程序员已经写下了显式的分支判断，检查器也无法利用分支条件：

```flurry
-- 要求编译期正整数的函数
pure comptime fn checked_positive(value: Integer) -> Integer
where
    requires value > 0
{
    value
}

-- 在流不敏感视角下会被拒绝的写法（但在 Flurry 中合法）
pure comptime fn positive_or_one(value: Integer) -> Integer {
    -- 分支条件已经检查了 value > 0
    -- 如果检查器不将分支条件记入当前分支的局部上下文，
    -- 调用 checked_positive(value) 时就无法证明 requires value > 0
    inline if value > 0 {
        checked_positive(value)
    } else {
        checked_positive(1)
    }
}
```

同样地，当对类型参数 `T: Type` 进行模式匹配时，如果在匹配分支内部不能局部收窄 `T` 的结构身份，就无法直接安全地使用解构出的类型参数。

因此，条件判断（无论是关系比较 `value > 0`，还是类型模式匹配 `T is Array<'u32, length>`）应当在对应分支的作用域内转化为可供类型检查器使用的局部前提。

---

## 二、 核心机制：将分支条件转化为分支内的局部事实

在 Flurry 中，上面的 `positive_or_one` 可以直接通过编译。

这是因为 Flurry 支持 **Occurrence Typing（出现类型 / 流敏感类型窄化）**。每当控制流进入 `if`、`inline if` 或 `match` 的某个分支时，编译器会将该分支的进入条件作为一条**局部事实**加入当前分支的已知约束环境 $\Gamma$ 中。

下面看一个同时包含**类型结构窄化**与**编译期数值范围窄化**的示例：

```flurry
use std.prelude.*;

-- 场景 A：对 Type 进行模式匹配与结构提取
pure comptime fn word_array_length(T: Type) -> ?usize {
    inline if T is do {
        Array<'u32, length> => length,
        _ => null,
    }
}

-- 场景 B：对受 requires 契约约束的编译期整数进行分支窄化
pure comptime fn checked_positive(value: Integer) -> Integer
where
    requires value > 0
{
    value
}

pure comptime fn positive_or_one(value: Integer) -> Integer {
    inline if value > 0 {
        -- ✅ 在此分支内，上下文自动获得局部事实：value > 0
        -- 因此 checked_positive(value) 的 requires value > 0 得以满足
        checked_positive(value)
    } else {
        -- 在 else 分支内，局部事实为 not (value > 0)，即 value <= 0
        -- 这里传入常量 1，满足 1 > 0
        checked_positive(1)
    }
}

const MATCHED_LEN = word_array_length(Array<u32, 4>);
const SKIPPED_LEN = word_array_length(bool);
const NORMALIZED = positive_or_one(-3);

test {
    println("matched: {}", MATCHED_LEN is usize);
    println("skipped: {}", SKIPPED_LEN is null);
    println("normalized: {}", NORMALIZED);
}
```

> **复习：类型模式里的单引号 `'u32`**
> 在 `word_array_length` 的模式分支 `Array<'u32, length> => length` 中：
> - 无前缀的标识符 `length` 是**新引入的模式绑定变量**，用于捕获数组的第二个类型参数（长度）；
> - 带单引号前缀的 `'u32` 是**引用已有类型的模式表达式**，表示要求第一个类型参数等于 `u32`。

---

## 三、 分支局部事实的生命周期与汇合（Join）规则

分支内的局部事实如何在进入分支时生效、在离开分支时回收？其作用域与汇合规则如下：

```text
                  ┌─────────────────────────────────────────┐
                  │ 1. 分支前环境 Γ_parent                  │
                  │    已知声明: value : Integer             │
                  │    分支条件: inline if value > 0        │
                  └────────────────────┬────────────────────┘
                                       │
                 ┌─────────────────────┴─────────────────────┐
                 ▼ (进入 then 分支)                          ▼ (进入 else 分支)
┌─────────────────────────────────────┐     ┌─────────────────────────────────────┐
│ 2a. Then 分支环境 Γ_then            │     │ 2b. Else 分支环境 Γ_else            │
│   附加局部事实: value > 0           │     │   附加局部事实: not (value > 0)     │
│                                     │     │                                     │
│   ▶ 检查 checked_positive(value):   │     │   ▶ 此时已知 value <= 0             │
│     当前环境蕴涵 value > 0 (通过)   │     │     若在此调 checked_positive(value)│
│                                     │     │     将因违背前置契约被拒绝          │
└──────────────────┬──────────────────┘     └──────────────────┬──────────────────┘
                   │ (离开 then 分支)                          │ (离开 else 分支)
                   └─────────────────────┬─────────────────────┘
                                         ▼
                  ┌─────────────────────────────────────────┐
                  │ 3. 汇合点 (Join Point) Γ_after          │
                  │    各分支独有的局部假设已退出作用域      │
                  │    仅保留【所有可达前驱共同满足】的结论  │
                  └─────────────────────────────────────────┘
```

局部事实的生命周期遵循三条核心规则：
1. **进入分支时生效**：当控制流进入 `then` 分支时，条件本身（如 `value > 0`）加入当前分支环境；进入 `else` 分支或后续互斥 `match` 分支时，前序已排除条件的否定形式（如 `value <= 0`）加入当前分支环境。
2. **离开分支时失效**：当某个分支检查结束时，仅在该分支内成立的局部事实和模式绑定变量（如 `Array<'u32, length>` 中的 `length`）随之退出作用域，不会泄漏到外层环境或兄弟分支。
3. **汇合点只保留可达前驱的共同结论**：当多个分支执行完毕并在后续位置汇合时，只有**每一个可达前驱分支都共同满足的结论**才会保留在汇合后的环境 $\Gamma_{\text{after}}$ 中。如果某个分支提前 `return` 或发散（`never`），该不可达分支不会参与汇合。

---

## 四、 边界示例：跨分支误用局部假设 vs 卫语句（Early Return）窄化

根据上述作用域规则，如果在 `else` 分支或分支外部使用只属于 `then` 分支的窄化结论，编译器会直接报错：

```flurry
pure comptime fn checked_positive(value: Integer) -> Integer
where
    requires value > 0
{
    value
}

-- 负例：在 else 分支或分支外部使用 then 分支的窄化结论
pure comptime fn broken_branch_assumption(value: Integer) -> Integer {
    inline if value > 0 {
        let _ = checked_positive(value);
    } else {
        -- ❌ 编译期拒绝：
        -- 在 else 分支中，已知事实为 value <= 0，无法满足 requires value > 0
        checked_positive(value)
    }

    -- ❌ 编译期拒绝：
    -- 离开 inline if 之后，then 分支的局部假设已退出作用域，
    -- 此时 value 仍为未知正负的 Integer
    checked_positive(value)
}
```

### 利用提前返回（Early Return）收窄后续主干环境

由于汇合点只考虑**可达前驱（Reachable Predecessors）**，如果我们在不满足条件的分支中直接 `return`，那么后续主干代码唯一的可达前驱就是满足条件的分支，从而使窄化后的事实在整个后续主干中保持有效：

```flurry
-- 利用卫语句（Early Return）将窄化事实延续到后续主干
pure comptime fn normalize_and_double(value: Integer) -> Integer {
    inline if value <= 0 {
        -- 不满足正数条件时直接提前返回
        -- 该分支在此结束，不会到达下方的汇合点
        return checked_positive(1) * 2;
    }

    -- ✅ 能够到达此处，唯一的可达前驱满足 not (value <= 0)，即 value > 0
    -- 因此在后续主干代码中，value > 0 均作为已知事实成立：
    let valid = checked_positive(value);
    valid * 2
}
```

通过这种卫语句写法，无需多层嵌套缩进，Occurrence Typing 就能自动识别出 `value <= 0` 分支已提前返回，并将 `value > 0` 的事实保留到后续主干作用域中。

---

## 五、 本篇小结

| 维度 | 核心机制 | 编译器保证与工程效果 |
| :--- | :--- | :--- |
| **类型模式窄化** | `inline if T is do { Array<'u32, length> => ... }` | 在匹配分支内部自动收敛 `T` 的结构身份，并提取类型内嵌参数（如 `length`） |
| **单引号类型模式** | `'u32` / `'(TypeExpr)` | 区分“匹配已有具体类型（`'u32`）”与“绑定新模式变量（`length`）” |
| **值与契约窄化** | `inline if value > 0 { ... }` | 将分支条件转化为局部已知事实，用于证明分支内调用的 `requires` 契约 |
| **分支作用域与汇合** | 分支内生效、出分支失效、汇合取可达前驱交集 | 局部假设不跨分支泄漏；若反例分支提前 `return`，主干自动继承正面事实 |

---

## 六、 下一篇预告

当一个泛型函数同时涉及位宽算术约束、类型相等构造与分支局部窄化时，如果某条约束未能满足，我们该如何阅读编译器的诊断反例并定位缺失的 `where requires` 前提？同时，编译期类型约束（PTC）与第三卷介绍的运行期程序状态验证（`verified fn` / `outcomes`）在工程实践中应如何配合？

下一篇 **《Flurry 演进手记 (087)：读懂类型约束与诊断信息——组合约束推理、残余条件与签名设计》**，我们将总结类型约束的工程实践与诊断方法。
