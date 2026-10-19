# Flurry 演进手记 (084)：初识命题类型约束（PTC）——为什么类型系统需要一个专门的逻辑求解器

在前面的几个篇章中，我们已经陆续使用过了精化类型 `refines { ... }`、编译期谓词 `comptime predicate`、前后置契约 `requires / ensures` 以及类型模式匹配 `T match { ... }`。

当我们的张量引擎 `Tensor<T, S>` 越来越庞大，各种编译期约束开始交织在一起：
- 有的约束在问**布尔逻辑组合**：`A and (B or not C)`；
- 有的约束在问**编译期算术不等式**：`0 <= index and index + 1 < length`；
- 有的约束在问**类型相等与结构分类**：`Type.size_of(T) == 8 and Type.predicates.is_tuple(Itself)`。

如果编译器内部只是用一堆零散的 `if-else` 硬编码去拼凑这些检查，很快就会陷入“换个写法就推不出来”或者“死循环卡死”的泥潭。
为此，Flurry 在编译期类型系统中内置了一个受现代 **SMT（Satisfiability Modulo Theories，模理论可满足性）** 启发而设计的逻辑推理引擎——**PTC（Propositional Type Constraints，命题类型约束）**！

今天，我们就来认识这位幕后功臣，搞清楚它是如何把“布尔逻辑骨架”与“领域理论（Domain Theories）”解耦的，以及它在面对任何编译期契约提问 $\Gamma \vdash Q$ 时给出的**四种精确裁决结果**！

---

## 一、 破局与痛点：为什么不能把“类型推导”当成普通布尔求值？

在编写编译期代码时，初学者最容易把“已具体化的编译期常量求值”和“带符号参数的定义期约束蕴涵（Entailment）”混为一谈（`v0` 误区）：

```
  [ 场景 A：具体常量求值 (Concrete Evaluation) ]
  输入已知：index = 2, length = 4
  计算 `0 <= 2 and 2 < 4` ──► 直接算出布尔常量 `true`！

  [ 场景 B：定义期符号蕴涵证明 (Symbolic Entailment: Γ ⊢ Q) ]
  在函数 `pure comptime fn foo(index: Integer, length: Integer)` 的定义体内：
  `index` 和 `length` 只是符号（连具体数值都没有）！
  此时编译器面对的是一个逻辑证明题：
  “在已知前提集合 Γ 下，能否保证对一切合法输入，目标命题 Q 必然成立？”
```

如果编译器没有一个专门的符号命题求解器（PTC），它面对符号 `index` 和 `length` 时就只能两手一摊：要么完全放弃定义期检查（退化回 C++ 模板那种只有实例化才报错的体验），要么连最基本的 `result == index + 1` 加 `index + 1 < length` 推出 `result < length` 都做不到！

---

## 二、 核心心智模型：SMT 风格的两层解耦与 $\Gamma \vdash Q$ 的四大裁决

在 Flurry 中，PTC 负责且仅负责 **`U_1` 编译期与类型层的逻辑命题推理**。它将复杂的约束求解拆成了正交的两层：
1. **外层：命题逻辑层（Propositional Skeleton）**——处理合取（`and`）、析取（`or`）、否定（`not`）与分支收集到的上下文假设 $\Gamma$。
2. **内层：领域原子理论层（Domain Theories）**——专门回答特定领域的原子关系，例如线性整数算术（Presburger 算术片段）、位向量大小关系、类型相等性、子类型与 Trait 满足性（我们将在下一回 `085` 盘点全部 19 类原子命题）。

当编译器拿着当前作用域收集到的已知事实集合 $\Gamma$（例如外层函数的 `requires` 条件、前序调用留下的 `ensures` 契约、`if` 分支守卫），去检验某个目标要求 $Q$（即判断蕴涵式 $\Gamma \vdash Q$）时，PTC 求解器会返回以下**四种截然不同的状态之一**：

```
+-------------------------------------------------------------------------+
|                    PTC 对逻辑提问 (Γ ⊢ Q) 的四大裁决                      |
+-------------------------------------------------------------------------+
|                                                                         |
|  1. Entailed (已蕴涵 / 必然成立)  ──► ✅ 证明通过！                       |
|     • 含义：在前提 Γ 下，目标命题 Q 在逻辑上 100% 必然成立。               |
|                                                                         |
|  2. NotEntailed (未蕴涵 / 存在反例) ──► ❌ 存在反例，契约不足！            |
|     • 含义：前提 Γ 太弱，求解器能找到至少一组满足 Γ 却令 Q 为假的反例！    |
|     • 铁律：NotEntailed 绝不等于“证明了 ¬Q 对所有输入都成立”！             |
|                                                                         |
|  3. Unknown (可判定片段外 / 预算受限) ──► ⚠️ 状态未决，需显式证据或特化    |
|     • 含义：命题超出了自动求解的理论片段（如一般非线性算术）或步数预算。    |
|     • 铁律：Unknown 是编译器的“检查状态”，绝不是第三种运行期布尔真值！     |
|                                                                         |
|  4. IllFormed (良构性破坏 / 域错误) ──► 🚫 前提或操作域本身非法！          |
|     • 含义：命题自身违反了类型域或布局前提（如对纯编译期无布局类型查布局）。|
+-------------------------------------------------------------------------+
```

---

## 三、 `v1` 实战：`Entailed` vs `NotEntailed`——为什么“存在反例”不等于“永远错误”？

让我们仔细体会第 1 种（`Entailed`）和第 2 种（`NotEntailed`）裁决之间的微妙区别。

在上一回（080）中，我们写过一个正确的 `next_checked_index`：它的入口前提 $\Gamma$ 是 `0 <= index and index + 1 < length`。在这个 $\Gamma$ 下，内部调用 `checked_index(index + 1, length)` 所需的目标 $Q$（即 `0 <= index + 1 and index + 1 < length`）裁决结果为 **`Entailed`**。

现在，假设某个粗心的程序员在写 `missing_upper_bound` 时，把入口条件里的 `index + 1 < length` 写漏成了 `index < length`！

```flurry
pure comptime fn next_index(index: Integer) -> (result: Integer)
where
    ensures result == index + 1
{
    return index + 1;
}

pure comptime fn checked_index(index: Integer, length: Integer) -> Integer
where
    requires 0 <= index and index < length
{
    return index;
}

-- 注意看这里的 requires：只写了 index < length，漏掉了 + 1！
comptime fn missing_upper_bound(index: Integer, length: Integer) -> Integer
where
    requires 0 <= index and index < length
{
    let step = next_index(index);
    return checked_index(step, length);
}
```

我们来分析一下，当 PTC 在检查 `missing_upper_bound` 内部那句 `checked_index(step, length)` 时，发生了什么：
- **已知前提 $\Gamma$**：`0 <= index`、`index < length`、`step == index + 1`
- **待证目标 $Q$**：`0 <= step and step < length`（即 `index + 1 < length`）
- **PTC 的推理过程**：如果 `index = 3, length = 4`，前提 $\Gamma$（`0 <= 3 and 3 < 4`）完全满足，但目标 $Q$（`3 + 1 < 4`）却变成了假！因此，$\Gamma \vdash Q$ 的裁决结果是 **`NotEntailed`（未蕴涵，存在边界反例 `index == length - 1`）**！

> **💡 核心心智模型：`NotEntailed` $\neq$ 证明了 $\neg Q$！**
> 请注意：$\Gamma \vdash Q$ 返回 `NotEntailed`，意思是“光凭 $\Gamma$ 不能保证 $Q$ **永远**为真（因为当 `index = 3, length = 4` 时 $Q$ 为假）”；但对于很多其他输入（比如 `index = 2, length = 4`），$Q$ 其实是真的！
> - 如果你把函数声明为要求定义期完整自洽的契约检查，定义期看到 `NotEntailed` 就会直接报错提醒你补强 `requires`；
> - 而在阶段化编译期函数 `comptime fn` 的渐进调用中，如果你在调用点传入的是 `missing_upper_bound(2, 4)`，代入具体常量后 `2 + 1 < 4` 成立，调用依然能够通过；只有当你传入反例 `missing_upper_bound(3, 4)` 时，才会在调用点被精确拦截！

```flurry
test "Entailed vs NotEntailed concrete instantiation v1" {
    -- 代入 (2, 4) 时，具体化后的命题 3 < 4 为 Entailed，顺利算出 3
    assert(missing_upper_bound(2, 4) == 3);
}
```

而一旦传入那个触发边界反例的 `(3, 4)`：

```flurry
-- ❌ 编译期直接拒绝！当 index = 3, length = 4 时，step = 4 违反了 checked_index 的 `step < length`
comptime {
    let boom = missing_upper_bound(3, 4);
}
```

---

## 四、 `v2` 进阶：`Unknown`（非线性边界）与 `IllFormed`（良构性错误）

除了 `Entailed` 和 `NotEntailed`，很多同学还会好奇：**什么时候会出现 `Unknown` 和 `IllFormed`？**

### 4.1 什么是 `Unknown`？

为了保证编译速度可控、可终止，PTC 的算术求解器核心针对的是**线性整数算术（Linear Integer Arithmetic）**与有界位向量等可判定片段。
如果你在符号变量上写出了一个一般性的**非线性多项式不等式**（例如对未知符号 `value` 和 `limit` 要求证明 `value * value * value + 7 * value < limit`），或者触发了极其复杂的递归谓词展开步数上限，求解器既不能在有限预算内证明它对所有情况成立（非 `Entailed`），也不能随便捏造一个假反例（非 `NotEntailed`）。此时，它的内部检查状态就是 **`Unknown`**！

记住两条关于 `Unknown` 的铁律（`AGENTS.md` §5）：
1. **`Unknown` 是编译期求解器的“检查状态”，绝不是语言里的第三种布尔值**（Flurry 的 `bool` 永远只有 `true` 和 `false`）！
2. 当一个必须在当前阶段完成的静态契约得到 `Unknown` 时，编译器不会侥幸放行，而是要求开发者补充更直接的线性契约或待具体化实参到达后再求值：

```flurry
pure comptime fn checked_square(value: Integer, limit: Integer) -> Integer
where
    requires value * value < limit
{
    return value * value;
}

test "non-linear constraint resolves once inputs are concrete v2" {
    -- 当传入具体常量 (3, 10) 时，3 * 3 < 10 立即化简为常量比较 9 < 10，
    -- 顺利判定为 Entailed，不会卡在 Unknown！
    assert(checked_square(3, 10) == 9);
}
```

### 4.2 什么是 `IllFormed`？

如果一个命题本身在提问时就破坏了被观察对象的良构域（Domain Precondition），比如：
- 把一个根本不是类型的值传给了只接受 `Type` 的布局查询；
- 或者试图对一个只能在编译期存在、没有任何物理内存表示的纯元编程构造强行询问目标机器布局，

那么在进入真假推理之前，PTC 就会直接给出第 4 种裁决：**`IllFormed`（良构性错误）**，并立即产生编译期诊断报错！

---

## 五、 再次铭记黄金边界：PTC 只管类型与编译期命题！

在进入下一回盘点 PTC 的 19 类原子命题之前，让我们用一张清晰的架构边界图，再次加固 `AGENTS.md` §5 中的核心铁律：

```
+-------------------------------------------------------------------------+
|                    Flurry 两大验证引擎的职责分界线                        |
+-------------------------------------------------------------------------+
|                                                                         |
|  [ PTC (命题类型约束引擎) ]  <-- 本卷 (Vol 4) 主角！                      |
|  • 管辖范围：U_1 编译期宇宙、Type 关系、Normal-Form 头、精化类型筛网、     |
|              编译期纯算术 (Integer / BitVec)、Trait 与行集合约束。         |
|  • 绝不染指：运行期堆内存修改、可变借用状态机、运行期指针解引用有效性。     |
|                                                                         |
|  ================ [ 严禁混谈的编译期 / 运行期契约边界 ] ================  |
|                                                                         |
|  [ 程序验证层 (Program Verification Layer) ] <-- 第 3 卷 (060 回) 主角！ |
|  • 管辖范围：U_0 运行期程序状态、内存与资源规格、所有权生命周期状态转换。  |
+-------------------------------------------------------------------------+
```

正因为 PTC 绝不把运行期可变内存状态搅和进来，它面对的所有项（Terms）都是**纯粹、不可变、数学意义上良定义的编译期对象**，这正是它能够快如闪电、确定性复用的根本原因！

---

## 六、 本篇小结

| PTC 裁决状态 | 逻辑含义（针对 $\Gamma \vdash Q$） | 编译器行为与工程启示 |
| :--- | :--- | :--- |
| **`Entailed`** | 从前提 $\Gamma$ 必然推出目标 $Q$ | 静默通过，契约完全满足 |
| **`NotEntailed`** | 存在至少一组满足 $\Gamma$ 但令 $Q$ 为假的反例 | 拒绝不充分的定义期契约（注意：不代表 $\neg Q$ 恒成立） |
| **`Unknown`** | 超出当前可判定理论片段（如一般非线性）或预算上限 | 作为求解器检查状态（非第三布尔值），要求补充证据或待具体化 |
| **`IllFormed`** | 命题自身违反了类型域或观察前提 | 直接报良构性错误，不进入真假推理 |

---

## 七、 留给下一篇的悬念

既然我们知道了 PTC 的外层是布尔逻辑骨架，内层是专门处理不同领域关系的**原子命题（Atomic Propositions）**——那么，整个 Flurry 语言看似千变万化的类型检查、布局比对、模式匹配、子类型转换、Trait 约束、整数同余、位向量比较以及错误行（Error Row）包含关系，底层到底由多少种原子命题支撑起来？

说出来你可能不敢相信：**仅仅 19 类原子命题（`A01`–`A19`），就拼出了整个 Flurry 编译期类型宇宙的全部静态法则！**

下一回 **《Flurry 演进手记 (085)：PTC 的 19 类原子命题——从类型相等、子类型到 Trait 满足性与位宽关系》**，我们将首次完整拆解这 19 块逻辑原石！
