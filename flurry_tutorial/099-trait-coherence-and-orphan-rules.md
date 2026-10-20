# Flurry 演进手记 (099)：守住全局唯一性的底线——Coherence（一致性）、孤儿规则与负向推理证据

> **系列导读**：在上一期（098）中，我们了解了当同一个包内存在多份适用实现时，编译器如何依靠“结构精度（Structural Specificity）”选出最精确的赢家。但当视野从单个文件扩展到由成百上千个第三方包（Packages）组成的开放生态时，一个新的威胁出现了：如果两个素不相识的第三方库作者，各自在自己的包里为标准库的 `u32` 或外部容器 `Pair<A, B>` 实现了同一个外部接口，当下游用户同时引入这两个库时，整个类型系统的确定性就会瞬间崩塌。今天，我们将深入剖析 Flurry 守卫全局唯一性的护城河——**实现一致性（Coherence）**、**有序本地身份规则（Ordered Local Identity Rule）**与 `newtype` 零开销适配。

---

## 一、开放生态的难题：传统方案的“钻石依赖相撞”

假设我们的遥测核心包 `base` 定义了一个公开接口 `pub trait Label` 和一个通用对偶容器 `pub struct Pair<A, B>`：

```flurry
-- 位于基础包 base 中：
pub trait Label {
    fn label(*self) -> str;
}

pub struct Pair
where A, B
{
    first: A,
    second: B,
}
```

现在，有两个完全独立、互不知情的业务团队分别开发了包 `pkg_a`（温度传感器插件）和包 `pkg_b`（气压传感器插件），它们都只依赖 `base`。

如果语言没有任何包归属限制（传统的混乱世界）：
- `pkg_a` 的作者想：“只要 `Pair` 的第一个元素是我的本地类型 `LocalA`，不管第二个元素 `T` 是什么，我都给它实现 `Label`。”
- `pkg_b` 的作者想：“只要 `Pair` 的第二个元素是我的本地类型 `LocalB`，不管第一个元素 `U` 是什么，我也给它实现 `Label`。”

这两个包在各自独立编译时，看起来都“合情合理”，甚至都包含了各自包的本地类型（`LocalA` 和 `LocalB`）。
然而，当主工程 `app` 同时引入 `pkg_a` 和 `pkg_b`，并写出一行 `Pair<LocalA, LocalB>` 时，问题降临了：

```text
           [ 基础包 base: 定义了 Label 与 Pair<A, B> ]
                    /                       \
                   ▼                         ▼
      [ 业务包 pkg_a ]                   [ 业务包 pkg_b ]
  impl Label for Pair<LocalA, T>     impl Label for Pair<U, LocalB>
                   \                         /
                    ▼                       ▼
           [ 下游主程序 app: 构造 Pair<LocalA, LocalB> ]
       💥 致命冲突。两份精度同为 1 (Pair<◇, ◇>) 的实现正面相撞。
```

更致命的是，如果编译器允许 `pkg_a` 和 `pkg_b` 先各自单独编译通过，等到下游用户组合它们时才报错，那就意味着**把上游库设计的冲突责任转嫁给了无辜的下游使用者**。

我们必须在**定义 `impl` 的那个包编译时**，就有一套严密的数学规则，判断“谁有权写这个 `impl`、谁无权写”。

---

## 二、Flurry 的“有序本地身份规则（Ordered Local Identity Rule）”

在 Flurry 中，决定一个包能否声明某条 `impl Trait for Subject` 的规则称为**包归属规则（Package Ownership / Orphan Rules）**：

1. **固有实现（Inherent `impl Type`）**：只能写在 `Type` 的定义包中。
2. **本地 Trait 实现（Local Trait）**：如果 `Trait` 本身是在当前包定义的，当前包可以为任何允许用户实现的主体类型实现该 `Trait`。
3. **外部 Trait 实现（External Trait）**：当 `Trait` 来自外部依赖包时，编译器会对归一（Normalized）后的 **`Subject` 主体类型** 以及 **`Trait` 的显式实参（按声明顺序）** 执行**从外到内、从左到右的有序本地身份检查**。

### 从外到内、从左到右的“探路者”检查法则

编译器扮演一个沿着类型语法树从外向内、从左向右行进的探路者，严格执行以下三条核心规则：

```text
[ 有序本地身份检查（Ordered Local Identity Rule） ]
  观察序列：先看 Subject 主体 ──► 再按顺序看 Trait<Arg1, Arg2, ...> 的显式实参
  遍历方向：从外到内、从左到右

  规则 1：途中【必须遇到】至少一个归当前包所有的名义定义身份（Local Nominal Identity）。
  规则 2：在到达第一个本地名义身份【之前】，【绝对不允许遇到】任何仍可自由替换的量化参数（如 T, U）或未确定结构。
  规则 3：一旦成功到达第一个本地名义头，它内部的参数以及它右侧的后续位置，都可以自由使用量化参数。
```

现在，让我们用这套探路者法则，重新审视刚才 `pkg_a` 和 `pkg_b` 的代码，看看为什么编译器当场放行了 `pkg_a`，却在编译 `pkg_b` 时直接将它拦下：

```flurry
-- ==========================================
-- 场景 A：在只依赖 base 的包 pkg_a 中
-- ==========================================
struct LocalA {}

-- 探路过程：
-- 1. 外层构造子 Pair（外部 base 包，继续往里走）
-- 2. 第 1 个子参数 LocalA ──► 🎯 命中本地名义身份。且此前未遇到任何自由类型变量。
-- 3. 到达 LocalA 之后的第 2 个子参数 T 允许自由量化。
-- ✅ 编译通过。
impl Label for Pair<LocalA, T>
where T
{
    fn label(*self) -> str {
        "a"
    }
}
```

```flurry
-- ==========================================
-- 场景 B：在只依赖 base 的包 pkg_b 中
-- ==========================================
struct LocalB {}

-- 探路过程：
-- 1. 外层构造子 Pair（外部 base 包，继续往里走）
-- 2. 第 1 个子参数 U ──► 💥 在到达本地身份 LocalB 之前，先撞上了自由量化变量 U。
-- ❌ 编译期直接拒绝：违反有序本地身份规则。
impl Label for Pair<U, LocalB>
where U
{
    fn label(*self) -> str {
        "b"
    }
}
```

停下来赞叹一下这条**从左到右有序检查**的精妙之处：
- 因为 `pkg_a` 必须在第一个槽位放具体的 `LocalA` 才能让第二个槽位泛化为 `T`，而任何其他包 `pkg_b` 如果也想泛化某一个槽位，它也必须在**第一个槽位**放它自己的 `LocalB`（即写成 `Pair<LocalB, U>`）。
- 既然 `LocalA` 和 `LocalB` 是两个不同包的本地名义类型，永远不可能相等（`LocalA != LocalB`），那么 `Pair<LocalA, T>` 和 `Pair<LocalB, U>` 的交集就**在数学上绝对为空**。下游无论怎么组合，都绝不可能发生同精度撞车。

---

## 三、当外部 Trait 遇上外部类型：为什么 `typealias` 没用，必须用 `newtype`？

假设在我们的遥测业务包里，既想使用外部包定义的接口 `trait Code`，又想作用在内置的 `u32` 类型上。
如果我们直接写 `impl Code for u32`，因为 `Code` 和 `u32` 都不是当前包定义的，探路者走完整个头也没遇到本地身份，编译器会立刻拒绝。

有的读者会想：“那我写一行 `typealias MyWord = u32;`，再给 `MyWord` 实现 `Code` 可以吗？”
**不可以。** 因为 `typealias` 只是给原有类型起了一个别名，归一（Normalize）之后它依然是裸的 `u32`，**不创建新的本地名义身份**。

在 Flurry 中，真正零运行时开销、又能创建全新本地名义身份并提供显式类型视角（`.as(...)`）的正道，是使用 **`newtype`**。

```flurry
trait Code {
    fn code(*self) -> u32;
}

-- 1. 使用 newtype 创建归属当前包的全新名义类型身份 LocalWord。
-- 它的底层内存表示与 u32 完全相同（零开销），但拥有独立的本地定义身份。
newtype LocalWord = u32;

-- 2. ✅ LocalWord 是当前包的本地名义身份，完美通过归属检查。
impl Code for LocalWord {
    fn code(*self) -> u32 {
        16u32
    }
}

-- 3. 在函数形参中使用 u32.as(LocalWord) 视角，调用方可直接传入普通 u32。
fn show_word(word: u32.as(LocalWord)) -> void {
    println("code: {}", word.code());
}

test {
    -- 调用方直接传 7，通过参数视角按 LocalWord 的 Code 实现完成调用，
    -- 既无需运行时包装拷贝，也不会污染全局的 u32。
    show_word(7u32);
}
```

---

## 四、Coherence 的隐藏大礼：为 PTC 否定推理提供坚实证据

还记得我们在第 097 期留下的一个悬念吗？
当我们写下否定约束 `requires not (T :- Disabled)` 时，对于一个具体的类型 `Meter`，编译器凭什么敢拍胸脯保证：“在整个程序的任何角落，绝对没有任何一个包给 `Meter` 实现了 `Disabled`”？

答案正是今天学到的 **Coherence（包归属与全局一致性）**。

```text
[ 当 PTC 证明 not (Meter :- Disabled) 时 ]
  │
  ├─► 根据包归属规则（Orphan Rules）：
  │     只有 Disabled 的定义包，或者 Meter 的定义包，才有资格编写 `impl Disabled for Meter`。
  │
  └─► 编译器检查当前可见的合法依赖世界：
        在这两个唯一合法的归属包中，均不存在适用于 Meter 的 Disabled 实现。
        ──► 由此得出 100% 严密的数学结论：not (Meter :- Disabled) 得证。
```

如果没有严格的包归属与一致性检查，任何一个下游未加载的随机模块都可能随时给 `Meter` 塞进一个 `impl Disabled for Meter`，那么编译期的负向推理 `requires not (T :- Disabled)` 就永远不可能安全成立。

---

## 五、本节核心概念速查表

| 概念 / 语法 | 核心规则 | 工程与安全价值 |
| :--- | :--- | :--- |
| **本地 Trait 归属** | 若 `Trait` 定义在当前包，可为任意允许用户实现的主体编写 `impl` | 赋予接口定义者完整的扩展自由 |
| **有序本地身份规则** | 外部 `Trait` 按“主体 $\to$ 接口实参”从外到内、从左到右扫描：首个本地名义身份前**严禁出现自由量化参数** | 在库定义端就扼杀未来的钻石依赖撞车，无需等到下游实例化 |
| `typealias A = B` | 仅建立透明别名，展开后仍是原类型身份 | **不能**用于获取外部类型的 `impl` 归属权 |
| `newtype Local = Ext;` | 建立全新的本地名义类型身份，底层表示与原类型相同 | 零开销为外部类型适配外部 Trait，配合 `val: Ext.as(Local)` 无缝调用 |
| **可见世界一致性** | `use` 仅控制名称查找，不能隐藏已存在的冲突或改变 `impl` 优先级 | 保证任何模块看到的同一个类型+接口实例永远解析到唯一实现 |

---

## 六、下一期预告：迎来第 100 期里程碑

在前四章（096–099）中，我们已经把运行期（Runtime）视角下的 Trait 声明、正负约束组合、结构特化精度和全局一致性研究得透透彻彻。

但是别忘了，Flurry 是一门建立在**双层类型理论（2LTT）**之上的语言。在 Flurry 中，代码不仅活在运行期宇宙 $U_0$，还活在编译期宇宙 $U_1$。
当我们迎来本系列的 **第 100 期里程碑** 时，一个横跨两个宇宙的终极问题摆在了面前：
- 为什么 Flurry 要用 **`^comptime_phase`** 属性（以及对应的 **`comptime impl` / `comptime derive`**）来划分 Trait 的编译期相，而不是简单地看函数前面有没有写 `comptime fn`？
- “Trait 成员的 Phase 归属（`^comptime_phase`）”与“函数自身的执行阶段（`comptime fn` vs `fn`）”为什么必须正交解耦？
- 同一个类型的 `impl` 和 `comptime impl` 如何组成互补实现关系？为什么两相**必须共享同一套 Coherence 一致性检查**，且绝不允许在缺相时偷偷退回低精度实现？

下一期 **《Flurry 演进手记 (100)：Trait 在两个宇宙里的双重生活——`^comptime_phase`、`comptime impl` 与共享一致性》**，让我们共同见证第 100 篇演进手记的诞生。

