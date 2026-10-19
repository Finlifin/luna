# 086 — 分支中的敏感类型：Occurrence Typing 如何缩小可能性

> **阅读指南**：在上一回（第 085 回）中，我们探索了类型 Interning 与运行时依赖洞（Holes）如何在编译器内部高效表示复杂的参数化类型。但在实际编写编译期函数或泛型逻辑时，我们经常会写出这样的代码：`inline if T is Array<'u32, length> ...` 或者 `inline if value > 0 ...`。为什么一旦走入 `then` 分支，编译器就突然“变聪明”了，允许我们直接把原本未知的 `value` 传给要求正数的函数，或者直接提取出 `Array` 里的维度参数 `length`？而在离开分支后，这些特权又为什么会立刻消失？本篇我们将揭开 **分支出现类型（Occurrence Typing / Flow-Sensitive Typing）** 与 **分支激活守卫（Branch Activation Guards）** 的神秘面纱。

---

## 一、 痛点：为什么明明已经检查过了，类型检查器还在“装傻”？

假设我们在开发一个硬件寄存器与张量序列化工具。我们需要写两个基础的编译期辅助函数：
1. 一个函数要求传入的编译期整数 `value` 必须大于 `0`（由 `requires value > 0` 契约保护）；
2. 另一个函数接收任意类型 `T: Type`，如果 `T` 恰好是一个元素为 `u32` 的静态数组 `Array<u32, length>`，就返回它的长度 `length`；否则返回 `null`。

### 尝试 `v0`：流不敏感（Flow-Insensitive）世界里的挫败

如果一门语言的类型检查器是“流不敏感（Flow-Insensitive）”的——也就是说，一个变量从声明的那一刻起，它的类型和已知属性在整个作用域里都是死板不变的——那么我们写出的最自然的条件判断就会碰壁：

```flurry
-- 期望一个编译期正整数的底层构造函数
pure comptime fn checked_positive(value: Integer) -> Integer
where
    requires value > 0
{
    value
}

-- 尝试 v0：在普通流不敏感视角下做分支裁剪
pure comptime fn positive_or_one_v0(value: Integer) -> Integer {
    -- 我们明明在分支条件里写了 value > 0！
    -- 但如果编译器不把分支条件记入当前分支的局部环境：
    -- 调用 checked_positive(value) 时，它只看到参数声明 value: Integer，
    -- 根本无法证明 requires value > 0！
    inline if value > 0 {
        checked_positive(value)
    } else {
        checked_positive(1)
    }
}
```

同样地，当我们试图解构一个类型参数 `T: Type` 时，如果在匹配分支内部，`T` 的身份不能被局部收窄（Narrowing），我们就不得不发明各种繁琐的反射 API 去重新查询和强制转换，甚至还可能写出在分支外误用局部绑定变量的危险代码。

**核心痛点**：
条件测试（无论是布尔比较 `value > 0`，还是类型/结构模式匹配 `T is Array<'u32, length>`）本质上都是在**现场生产新的编译期证据（Evidence）**。如果类型系统对控制流视而不见，程序员亲手写下的分支检查就会沦为编译期求解器的“盲区”。

---

## 二、 破局（v1）：把分支条件变成局部环境里的“激活假设”

在 Flurry 中，你不需要为 `v0` 做任何妥协——因为上面那段 `positive_or_one` 代码在 Flurry 中**原生就能直接通过编译**！

为什么？因为 Flurry 的类型与命题检查器支持 **Occurrence Typing（出现类型 / 流敏感类型收窄）**。每当控制流进入 `if`、`inline if` 或 `match` 的某个分支时，编译器都会把决定该分支可达的条件翻译为一条**带激活守卫的局部命题（Guarded Assumption）**，压入当前分支的命题环境 $\Gamma$ 中。

### 演进到 `v1`：类型模式收窄与值范围收窄双剑合璧

让我们看看 Flurry 是如何同时优雅处理**类型结构收窄**与**编译期数值收窄**的：

```flurry
use std.prelude.*;

-- 场景 A：对 Type 本身进行模式匹配与结构提取
pure comptime fn word_array_length(T: Type) -> ?usize {
    inline if T is do {
        Array<'u32, length> => length,
        _ => null,
    }
}

-- 场景 B：对受 requires 契约保护的编译期整数进行分支裁剪
pure comptime fn checked_positive(value: Integer) -> Integer
where
    requires value > 0
{
    value
}

pure comptime fn positive_or_one(value: Integer) -> Integer {
    inline if value > 0 {
        -- ✅ 在这个分支内，环境 Γ 自动获得了激活事实：(value > 0)
        -- 因此 checked_positive(value) 的 requires value > 0 被瞬间证明！
        checked_positive(value)
    } else {
        -- 在 else 分支内，环境 Γ 获得的是 ¬(value > 0)，即 (value <= 0)
        -- 但我们传的是常量 1，1 > 0 显然成立！
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

> **🔍 语法小贴士：类型模式里的单引号 `'u32` 是什么？**
> 仔细观察 `word_array_length` 中的模式分支：
> ```flurry
> Array<'u32, length> => length,
> ```
> 为什么 `u32` 前面有一个单引号 `'u32`，而 `length` 前面没有？
> 在 Flurry 的模式语法（见 `docs/grammar/syntax` 中的 `type_pattern_expression`）里：
> - 如果在模式位置直接写一个裸标识符（如 `length`），它默认是一个**模式绑定变量（Pattern Binding）**，负责把匹配到的第二个泛型实参捕获并绑定到新名字 `length` 上；
> - 而如果我们想在模式里**匹配一个已经存在的具体类型值**（比如要求第一个类型参数必须精确等于 `u32`，而不是把第一个参数绑定到一个名叫 `u32` 的新变量上），我们就使用单引号前缀 `'u32`（或 `'(TypeExpr)`）！它告诉解析器：“这不是新变量绑定，请把它当作类型表达式求值并与目标比对！”

---

## 三、 视觉心智模型：分支激活守卫（Activation Guards）与汇合（Join）

编译器内部究竟是如何做到“进分支时精准收窄，出分支时绝不泄露”的？难道每次进出一个 `if` 分支，编译器都要把整个庞大的类型与命题环境完整深拷贝（Deep Clone）一份吗？

当然不是！Flurry 的命题类型检查器（PTC）采用了一种极其精巧的 **分支激活守卫（Branch Activation Guards）** 机制。让我们用一张 Code Cartoon 来拆解它的生命周期：

```text
                  ┌─────────────────────────────────────────┐
                  │ 1. 父环境 Γ_parent                      │
                  │    已知声明: value : Integer             │
                  │    待决条件: inline if value > 0        │
                  └────────────────────┬────────────────────┘
                                       │
                 ┌─────────────────────┴─────────────────────┐
                 ▼ (分配守卫 α_then)                         ▼ (分配守卫 α_else)
┌─────────────────────────────────────┐     ┌─────────────────────────────────────┐
│ 2a. Then 分支环境 Γ_then            │     │ 2b. Else 分支环境 Γ_else            │
│   激活守卫: α_then = true           │     │   激活守卫: α_else = true           │
│   条件注入: α_then ⟹ (value > 0)    │     │   条件注入: α_else ⟹ ¬(value > 0)   │
│                                     │     │                                     │
│   ▶ 检查 checked_positive(value):   │     │   ▶ 此时 (value <= 0) 被激活        │
│     查询: Γ_then ⊢ value > 0 ?      │     │     若在此调 checked_positive(value)│
│     命中 α_then ⟹ Entailed (通过!)  │     │     将直接触发 NotEntailed (拒绝!)  │
└──────────────────┬──────────────────┘     └──────────────────┬──────────────────┘
                   │ (撤销守卫 α_then)                         │ (撤销守卫 α_else)
                   └─────────────────────┬─────────────────────┘
                                         ▼
                  ┌─────────────────────────────────────────┐
                  │ 3. 汇合点 (Join Point) Γ_after          │
                  │    α_then 与 α_else 均已失效 (Deactivated)│
                  │    仅保留【所有可达前驱共同支持】的结论！ │
                  └─────────────────────────────────────────┘
```

> **💡 核心心智模型：局部证据的三大铁律**
> 1. **进入分支即激活**：当控制流进入 `then` 分支时，编译器分配一个临时守卫标记 $\alpha_{\text{then}}$，并向求解器注入带守卫的事实 $\alpha_{\text{then}} \implies \text{cond}$。同理，`match` 的后续分支不仅能看到当前模式匹配成功的事实，还能在模式可判定不相交时，看到**前面所有已排除分支的否定事实**！
> 2. **离开分支即撤销**：当分支检查结束时，编译器只需将守卫 $\alpha_{\text{then}}$ 标记为失效，所有仅仅依赖该分支假设推导出的局部等式、类型收窄和模式绑定（如 `Array<'u32, length>` 中的 `length`）都会瞬间退栈，绝不会污染父环境或兄弟分支。
> 3. **汇合点（Join）只取交集**：当两个分支正常走完并在后续代码汇合时，只有**每一个可达前驱分支都共同证明了的结论**，才能留存在汇合后的环境 $\Gamma_{\text{after}}$ 中。如果某个分支提前 `return` 或发散（Diverge），不可达的前驱不会稀释存活分支的证据！

---

## 四、 边界与红线：跨分支透支假设与提前返回收窄（v2）

理解了“分支守卫在离开时失效，汇合点只保留交集”，我们就能立刻明白为什么下面两类错误代码会被编译器无情拦截：

```flurry
pure comptime fn checked_positive(value: Integer) -> Integer
where
    requires value > 0
{
    value
}

-- 负例 1：在 else 分支或分支外部“偷用” then 分支的收窄结论
pure comptime fn broken_branch_assumption(value: Integer) -> Integer {
    inline if value > 0 {
        let _ = checked_positive(value);
    } else {
        -- ❌ 编译期直接拒绝！
        -- 在 else 分支中，激活的事实是 ¬(value > 0)（即 value <= 0），
        -- 根本不能满足 checked_positive 的 requires value > 0 契约！
        return checked_positive(value);
    }

    -- ❌ 编译期直接拒绝！
    -- 走出 inline if 之后，α_then 守卫已经撤销！
    -- 此时回到了父环境，value 依然只是一个未知正负的 Integer。
    checked_positive(value)
}
```

那么，如果我们想在函数开头做一个“卫语句（Guard Clause）”式的早期拦截，让后面的主干代码全都享受收窄后的类型或契约，该怎么写呢？

### 演进到 `v2`：用不可达分支（Early Return）实现主干环境收窄

既然汇合点（Join Point）只对**可达前驱（Reachable Predecessors）**求交集，那么只要我们在不满足条件的分支里直接 `return`，后续代码唯一的可达前驱就只剩下满足条件的那个分支了！

```flurry
-- 演进到 v2：利用卫语句（Early Return）将收窄事实延续到后续主干
pure comptime fn normalize_and_double(value: Integer) -> Integer {
    inline if value <= 0 {
        -- 不满足正数条件时直接提前返回！
        -- 该分支在此终结，不会抵达下方的汇合点。
        return checked_positive(1) * 2;
    }

    -- ✅ 能够执行到这里，唯一的可达前驱就是 ¬(value <= 0)，即 value > 0！
    -- 因此在后续所有主干代码中，value > 0 都是稳固成立的已知事实：
    let valid = checked_positive(value);
    valid * 2
}
```

你看，不需要任何多余的嵌套缩进，Occurrence Typing 自动识别出 `value <= 0` 分支已经提前返回，从而将 `value > 0` 的证据无缝保留到了主干作用域中！

---

## 本篇小结

| 维度 | 核心机制 | 编译器保证与程序员收益 |
| :--- | :--- | :--- |
| **类型模式收窄** | `inline if T is do { Array<'u32, length> => ... }` | 在匹配分支内部自动收敛 `T` 的结构身份，并安全提取类型内嵌参数（如 `length`） |
| **单引号类型模式** | `'u32` / `'(TypeExpr)` | 区分“匹配已有具体类型（`'u32`）”与“绑定新模式变量（`length`）” |
| **值与契约收窄** | `inline if value > 0 { ... }` | 将关系条件转化为分支激活命题，直接喂给 `requires` 契约做自动证明 |
| **激活守卫与汇合** | $\alpha_{\text{branch}} \implies \text{cond}$ 与可达前驱交集 | 分支内精准生效，离开分支零泄漏；若反例分支提前 `return`，主干自动继承正面事实 |

---

### 留给下一篇的悬念

在这一篇里，我们看到了一个神奇的幕后功臣：每当我们进入 `inline if value > 0` 分支，或者写下 `requires T == u32 or T == f32` 时，总有一个“幕后大脑”在毫秒之间判断出某条命题是必然成立（`Entailed`）、必然冲突（`NotEntailed`），还是暂时无法确定（`Unknown`）。

如果这个命题同时横跨了**整数算术**（`N > 0`）、**类型相等**（`T == u32`）、**内存布局**（`Type.size_of(T)`）甚至**位掩码**呢？一个单独的算法怎么可能懂这么多不同领域的数学？

下一篇 **《087 — PTC 幕后大脑：十一个理论求解器如何协同破案？》**，我们将深入命题类型检查器（PTC）的核心引擎室，看看 11 个专职理论求解器是如何在 CDCL(T) 与 Nelson-Oppen 协议下联合办案的！
