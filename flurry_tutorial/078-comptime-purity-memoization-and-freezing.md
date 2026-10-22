# Flurry 演进手记 (078)：当编译期计算遇到副作用与缓存——Comptime 纯度、确定性缓存与分阶段实例化

在上一回（077）和第 076 回中，我们见识了 `pure comptime fn` 在类型工厂、反射和属性计算中的威力。特别是当我们在张量引擎里写下 `Tensor<f32, .dims = s>` 或 `SelectType(wide)` 时，整个类型系统的正确性都建立在一个基石之上：**同样的编译期输入，必须产生稳定、可复用的编译期类型结果**。

但是，一旦编译期支持图灵完备的任意代码执行，魔盒也就随之打开了：
1. 如果一个 `comptime fn` 每次调用都返回不同的结果，或者中途报错、耗尽步数限制（Step Limit），类型缓存岂不是会错乱？
2. 为什么同样是 `pure comptime fn`，用尖括号声明的 `Identity<T>` 和用圆括号声明的 `SelectType(wide)` 在类型相等性上表现得迥然各异？
3. 当一个函数签名中包含依赖编译期参数的延后类型表达式（如 `defer SelectType(wide)`）时，定义期检查与调用点实例化是如何分工并保持隔离的？

今天，我们就来拆解 Flurry 编译期计算的三大支柱：**纯度契约与记忆化（Memoization）**、**Normal-Form 头保留**、以及 **分阶段类型规约与独立实例化**。

---

## 一、 问题背景与设计思路：如果编译期计算没有纯度与缓存边界会怎样？

想象我们在为张量引擎设计一个根据精度标志 `wide: bool` 自动切换底层通道类型的函数。如果不区分“有副作用的编译期过程”和“纯确定性映射”，也不区分“函数调用”和“类型构造器头”：

```flurry
-- 缺乏纯度与构造器头契约时的隐患：
-- 问题 1：如果不保证纯度与确定性缓存，两次调用 SelectType(true) 可能造出两个互异的类型。
-- 问题 2：如果把包装类型 Identity<u32> 当成普通调用直接展开成 u32，名义包装和类型模式匹配将失效。
```

为了解决这些问题，Flurry 在编译期宇宙设立了清晰的契约。

---

## 二、 `pure comptime fn` 的记忆化契约 vs Normal-Form 头规则

### 2.1 `pure comptime fn` 与确定性缓存（Memoization）

在 Flurry 中，当你给编译期函数标上 `pure comptime fn` 时，你向编译器立下了一份**确定性纯度契约**：对于相等的编译期输入，该函数的结果必然唯一确定，编译器可以安全地对它进行**结果记忆化（Memoization）与类型身份复用**。

为了让这份缓存完全可靠，编译器内部执行四条严密规则：
1. **输入可比较与可哈希**：只有当所有实参在编译期都已完全确定，且满足编译期相等（`Eq`）与哈希（`Hash`）契约时，才启用记忆化查找。
2. **哈希碰撞二次核验**：缓存命中非止看 Hash 值相等，遇到相同 Hash 桶时必须执行结构化的编译期相等性（`Eq`）比对，避免因哈希碰撞张冠李戴。
3. **非纯操作隔离**：未带 `pure` 的普通 `comptime fn`（可能在局部做非确定性操作或过程式构造）每次按调用语义求值，不混入纯类型身份缓存。
4. **失败与超预算不缓存为成功**：如果一次编译期求值因为诊断错误、未就绪的外部依赖或**步数/内存预算耗尽（Budget Exhaustion）**而中断，编译器**免除**把半成品状态写进成功缓存。

### 2.2 核心规则对比：`SelectType(wide)` 立即规约 vs `Identity<T>` 保留头

现在，让我们把两个长得很像的 `pure comptime fn` 放在显微镜下对比：
- 一个用圆括号声明参数：`pure comptime fn SelectType(wide: bool) -> Type`
- 一个用尖括号声明参数：`pure comptime fn Identity<T: Type> -> Type`

它们在编译期求值时有什么本质区别？

```
  [ 普通纯编译期调用：SelectType(true) ]
  输入 true 就绪 ──► 立即执行函数体 ──► 规约（Reduce）为结果类型 i64。
  因此：SelectType(true) == i64  (完全相等。)

  [ Normal-Form 头构造器：Identity<u32> ]
  输入 u32 就绪 ──► 检查契约与类型合法性 ──► 保留 Identity<...> 构造器头，维持未展开态消除。
  因此：Identity<u32> != u32     (头不同，类型不等。)
```

> **💡 核心心智模型：Normal Form 的头维持未展开态。**
> 为什么 `pure comptime fn Identity<T: Type> -> Type { T }` 明明函数体返回了 `T`，`Identity<u32>` 却**有别于** `u32`（甚至 `Identity<Identity<u32>> != Identity<u32>`）？
> 因为尖括号 `<...>` 声明的是一个**正規形式类型构造器头（Normal-Form Head）**（就像 `Tuple<...>`、`Array<T, n>` 一样）。它的函数体描述的是其内部结构或底层承载关系，但它的外部类型身份永远保留 `Identity<...>` 这个头。如果编译器手欠把头展开丢掉，后续的类型模式匹配（`083` 回）和抽象类型边界就会全面崩塌。

让我们在代码中同时验证这两条规律：

```flurry
-- 1. 普通圆括号纯编译期函数：当参数已知时直接求值出目标类型
pure comptime fn SelectType(wide: bool) -> Type {
    if wide {
        i64
    } else {
        i32
    }
}

-- 2. 尖括号 Normal-Form 头函数：即使函数体写 { T }，也保留 Identity 头。
pure comptime fn Identity<T: Type> -> Type {
    T
}

test normal_form_head_vs_ordinary_comptime_call {
    -- 圆括号调用直接规约到结果类型
    assumes SelectType(false) == i32;
    assumes SelectType(true) == i64;

    -- 尖括号调用保留 Normal-Form 头：同参相等，但有别于裸 T 或嵌套层数不同的自身。
    assumes Identity<u32> == Identity<u32>;
    assumes Identity<u32> != u32;
    assumes Identity<Identity<u32>> != Identity<u32>;
}
```

如果有人误以为 `Identity<u32>` 会像 `typealias` 或 `SelectType` 那样自动剥壳，试图把 `u32` 直接当成 `Identity<u32>` 混为一谈：

```flurry
-- ❌ 编译期直接报错：Identity<u32> 保留 Normal-Form 头，不会因函数体返回 T 就自动与 u32 相等
comptime {
    let SameTypeProof: refines { Itself == u32 } Type = Identity<u32>;
}
```

---

## 三、 分阶段类型规约：`defer` 残余类型与调用点独立实例化

现在来看一个分阶段计算的典型场景：当 `SelectType(wide)` 出现在一个**带编译期参数的函数签名**里时，它是如何工作的？

```flurry
fn keep_selected(
    comptime wide: bool,
    value: defer SelectType(wide),
) -> defer SelectType(wide) {
    value
}
```

请观察 `keep_selected` 的两个阶段：
1. **定义阶段（Normalization）**：在还没有调用者传入具体 `wide` 时，形参 `wide` 是一个符号绑定。由于标记了 `defer SelectType(wide)`，该函数免于在定义点强行对未知的 `wide` 展开分支，而是验证参数 `value` 与返回值的类型都是基于同一个 `wide` 的 `SelectType(wide)`，并将这一稳定的接口契约保存下来。
2. **调用阶段（Instantiation）**：每次调用 `keep_selected` 时，都会根据当次传入的编译期实参（`false` 或 `true`）独立完成 `SelectType(wide)` 的求值，不同调用实例之间完全隔离、互无扰于：

```flurry
test staged_defer_evaluation_and_instantiation {
    -- 实例 1：wide = false，将 defer SelectType(false) 规约为 i32
    let small = keep_selected(false, 3i32);
    assumes small == 3i32;
    assumes small'type == i32;

    -- 实例 2：wide = true，独立规约为 i64
    let large = keep_selected(true, 7i64);
    assumes large == 7i64;
    assumes large'type == i64;
}
```

---

## 四、 本篇小结

| 核心概念 | 语法 / 机制表现 | 语义规则与工程保障 |
| :--- | :--- | :--- |
| **纯编译期函数** | `pure comptime fn F(...)` | 承诺相同输入产生确定性相同结果；要求实参满足编译期 `Eq` 与 `Hash` 才启用记忆化 |
| **失败不入缓存** | 诊断错误 / 步数预算耗尽 | 任何求值失败或超预算中切断省略入成功缓存，防止后续查询读到残损结果 |
| **Normal-Form 头** | `pure comptime fn H<T> -> Type` | 尖括号 `<...>` 构造器永远保留头身份，`Identity<T> != T` |
| **延后类型计算** | `defer SelectType(wide)` | 在定义期保留为依赖 `wide` 的合法残余计算，待 `wide` 具象化后再规约 |
| **独立实例化** | 调用点代入具体 `comptime` 实参 | 每次调用根据当次编译期实参独立完成类型规约，不同调用实例间完全隔离 |

---

## 五、 下一篇预告

在上面的 `keep_selected` 中，参数类型写成了 `defer SelectType(wide)`。
你可能会好奇：既然在 `keep_selected` 的定义期，`SelectType(wide)` 还没法算出到底是 `i32` 还是 `i64`，那编译器在检查 `keep_selected` 的函数体 `return value;` 时，是怎么看待 `value` 的类型的？

更进一步，如果我直接在函数签名里写 `fn pass_through(value: Any) -> Any { value }`，这里的 **`Any`** 究竟是什么？把 `Any` 放进静态强类型的 Flurry 里，难道会让 Flurry 退化成像 Python 或 TypeScript `any` 那样要在运行期装箱（Boxing）、查虚表、甚至失去类型安全的动态语言吗？

下一回 **《Flurry 演进手记 (079)：在类型里写 Any 会变成动态语言吗——编译期渐进依赖类型（Gradual Dependent Type）》**，我们将揭晓 `Any` 与 `defer` 如何在**零运行期开销**的前提下，实现丝滑的编译期渐进类型检查。
