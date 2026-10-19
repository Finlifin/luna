# Flurry 演进手记 (078)：当编译期计算遇到副作用与缓存——Comptime 纯度、确定性 memoization 与冻结边界

在上一回（077）和第 076 回中，我们见识了 `pure comptime fn` 在类型工厂、反射和属性计算中的威力。特别是当我们在张量引擎里写下 `Tensor<f32, S>` 或 `SelectType(wide)` 时，整个类型系统的正确性都建立在一个基石之上：**同样的编译期输入，必须产生稳定、可复用的编译期类型结果**。

但是，一旦编译期支持图灵完备的任意代码执行，魔盒也就随之打开了：
1. 如果一个 `comptime fn` 每次调用都返回不同的结果，或者中途报错、耗尽步数限制（Step Limit），类型缓存岂不是会彻底错乱？
2. 为什么同样是 `pure comptime fn`，用尖括号声明的 `Identity<T>` 和用圆括号声明的 `SelectType(wide)` 在类型相等性上表现得截然不同？
3. 当编译器检查一个带 `comptime` 参数的泛型函数定义时，类型推导（Elaboration）与编译期求值（Comptime）是交织在一起的——在这个半成品阶段，编译器到底该**冻结（Freeze）**什么，又绝对**禁止冻结**什么？

今天，我们就来拆解 Flurry 编译期引擎的三大定海神针：**纯度契约与记忆化（Memoization）**、**Normal-Form 头保留**、以及 **Elaboration 与 Comptime 交织下的冻结边界（Freezing Boundary）**。

---

## 一、 破局与痛点：如果编译期计算没有纯度与缓存边界会怎样？

想象我们在为张量引擎设计一个根据精度标志 `wide: bool` 自动切换底层通道类型的函数。如果在 `v0` 中，我们不区分“有副作用的编译期过程”和“纯确定性映射”，也不区分“函数调用”和“类型构造器头”：

```flurry
-- ❌ v0 的混乱世界：
-- 痛点 1：如果不保证纯度与确定性缓存，两次调用 SelectType(true) 可能造出两个不相等的类型！
-- 痛点 2：如果把包装类型 Identity<u32> 当成普通调用直接展开成 u32，名义包装和类型模式匹配将彻底失效！
```

为了解决这些问题，Flurry 在编译期宇宙设立了清晰的契约。

---

## 二、 `v1` 升级：`pure comptime fn` 的记忆化契约 vs Normal-Form 头铁律

### 2.1 `pure comptime fn` 与确定性缓存（Memoization）

在 Flurry 中，当你给编译期函数标上 `pure comptime fn` 时，你向编译器立下了一份**确定性纯度契约**：对于相等的编译期输入，该函数的结果必然唯一确定，编译器可以安全地对它进行**结果记忆化（Memoization）与类型身份复用**。

为了让这份缓存绝对可靠，编译器内部执行四条严密规则：
1. **输入可比较与可哈希**：只有当所有实参在编译期都已完全确定，且满足编译期相等（`Eq`）与哈希（`Hash`）契约时，才启用记忆化查找。
2. **哈希碰撞二次核验**：缓存命中绝不仅仅看 Hash 值相等，遇到相同 Hash 桶时必须执行结构化的编译期相等性（`Eq`）比对，彻底杜绝因哈希碰撞张冠李戴。
3. **非纯操作隔离**：不带 `pure` 的普通 `comptime fn`（可能在局部做非确定性操作或过程式构造）每次按调用语义求值，绝不混入纯类型身份缓存。
4. **失败与超预算绝不缓存为成功**：如果一次编译期求值因为诊断错误、未就绪的外部依赖或**步数/内存预算耗尽（Budget Exhaustion）**而中断，编译器**绝不会**把半成品状态写进成功缓存！

### 2.2 铁律对比：`SelectType(wide)` 立即规约 vs `Identity<T>` 保留头！

现在，让我们把两个长得很像的 `pure comptime fn` 放在显微镜下对比：
- 一个用圆括号声明参数：`pure comptime fn SelectType(wide: bool) -> Type`
- 一个用尖括号声明参数：`pure comptime fn Identity<T: Type> -> Type`

它们在编译期求值时有什么本质区别？

```
  [ 普通纯编译期调用：SelectType(true) ]
  输入 true 就绪 ──► 立即执行函数体 ──► 规约（Reduce）为结果类型 i64！
  因此：SelectType(true) == i64  (完全相等！)

  [ Normal-Form 头构造器：Identity<u32> ]
  输入 u32 就绪 ──► 检查契约与类型合法性 ──► 保留 Identity<...> 构造器头，绝不展开消除！
  因此：Identity<u32> != u32     (头不同，类型不等！)
```

> **💡 核心心智模型（AGENTS.md 语义红线）：Normal Form 的头绝不展开！**
> 为什么 `pure comptime fn Identity<T: Type> -> Type { T }` 明明函数体返回了 `T`，`Identity<u32>` 却**不等于** `u32`（甚至 `Identity<Identity<u32>> != Identity<u32>`）？
> 因为尖括号 `<...>` 声明的是一个**正規形式类型构造器头（Normal-Form Head）**（就像 `Tuple<...>`、`Array<T, N>` 一样）！它的函数体描述的是其内部结构或底层承载关系，但它的外部类型身份永远保留 `Identity<...>` 这个头。如果编译器手欠把头展开丢掉，后续的类型模式匹配（`083` 回）和抽象类型边界就会全面崩塌！

让我们在代码中同时验证这两条规律：

```flurry
-- 1. 普通圆括号纯编译期函数：当参数已知时直接求值出目标类型
pure comptime fn SelectType(wide: bool) -> Type {
    if wide {
        return i64;
    } else {
        return i32;
    }
}

-- 2. 尖括号 Normal-Form 头函数：即使函数体写 { T }，也保留 Identity 头！
pure comptime fn Identity<T: Type> -> Type {
    T
}

test "normal form head vs ordinary comptime call" {
    -- 圆括号调用直接规约到结果类型
    assert(SelectType(false) == i32);
    assert(SelectType(true) == i64);

    -- 尖括号调用保留 Normal-Form 头：同参相等，但绝不等于裸 T 或嵌套层数不同的自身！
    assert(Identity<u32> == Identity<u32>);
    assert(Identity<u32> != u32);
    assert(Identity<Identity<u32>> != Identity<u32>);
}
```

如果有人误以为 `Identity<u32>` 会像 `typealias` 或 `SelectType` 那样自动剥壳，试图把 `u32` 直接当成 `Identity<u32>` 混为一谈：

```flurry
-- ❌ 编译期直接拒绝！Identity<u32> 保留 Normal-Form 头，不会因函数体返回 T 就自动与 u32 相等
comptime {
    let SameTypeProof: refines { Itself == u32 } Type = Identity<u32>;
}
```

---

## 三、 `v2` 进阶：Elaboration 与 Comptime 的交织，以及“冻结边界”

现在来看更刺激的场景：当 `SelectType(wide)` 出现在一个**带编译期参数的函数签名**里时，会发生什么？

```flurry
fn keep_selected(
    comptime wide: bool,
    value: defer SelectType(wide),
) -> defer SelectType(wide) {
    return value;
}
```

请仔细观察 `keep_selected` 的定义时刻（Definition Time）：
在还没有任何人调用 `keep_selected` 之前，形式参数 `wide` 只是一个符号（Symbolic Binder），既不是 `true` 也不是 `false`！此时，`SelectType(wide)` 根本没法算出具体的 `i32` 或 `i64`。

### 3.1 两种编译期求值形态与交织执行

在 Flurry 编译器内部，**Elaboration（类型推导与消解）** 与 **Comptime（编译期求值）** 并不是先跑完一个再跑另一个，而是像齿轮一样**双向交织（Interleaved）**的：
- Elaboration 在推导类型时，可能需要唤醒 Comptime 去算 `SelectType(true)` 或 `Type.size_of(T)`；
- Comptime 在跑一段编译期代码时，如果遇到还未完全具体化的残余表达式，采用 **编译期解释（Comptime Interpretation）** 在高层 IR 上推进；而一旦遇到完全闭合、无未决推导变量的纯计算，则可编译为引擎字节码进行高速 **编译期调用（Comptime Invocation）**。

### 3.2 什么是冻结（Freezing）？什么能冻，什么严禁冻？

当编译器检查完 `keep_selected` 的定义体，准备把它存进符号表供未来千百次调用复用时，会执行一个极其关键的动作——**定义冻结（`freeze_definition`）**。

```
+-------------------------------------------------------------------------+
|                 keep_selected 定义完成时的冻结边界                        |
+-------------------------------------------------------------------------+
|                                                                         |
|  ✅ 允许冻结进定义模板的“稳定资产” (Stable Artifacts)：                    |
|   1. 稳定的符号参数绑定 (Stable Binders)：如 `comptime wide: bool`        |
|   2. 已确立的接口契约 (Contracts)：参数与返回值的依赖关系                 |
|   3. 合法的残余计算描述 (Residual Computations)：如 `defer SelectType(wide)`|
|                                                                         |
|  ==================== [ 严禁跨越的冻结红线 ] ===========================  |
|                                                                         |
|  ❌ 严禁冻结的“局部活动状态” (Transient Solver State)：                   |
|   1. 尚未解出的活动推导元变量 (Active Inference Metavariables, `?T`)     |
|   2. 挂起在求解器队列里的等待者任务 (Active Waiters)                      |
|   3. 尚未提交的投机分支事务 (Candidate Transactions)                      |
|   4. 任何尚未清偿的失败约束或错误债务 (Failed Obligations)                |
+-------------------------------------------------------------------------+
```

为什么要死守这条冻结红线？
想象一下，如果编译器把定义期某个临时的推导元变量槽位 `?M0` 直接冻进了 `keep_selected` 的公共模板里：
- 第一次调用 `keep_selected(false, 3i32)` 时，把 `?M0` 填成了 `i32`；
- 第二次调用 `keep_selected(true, 7i64)` 时，如果复用了同一个可变槽位，`?M0` 里残存的 `i32` 就会与 `i64` 当场撞车，引发灾难性的跨调用污染！

正因为 Flurry 严格只冻结稳定的符号绑定和 `defer SelectType(wide)` 残余描述，每次调用 `keep_selected` 时，编译器都会把冻结模板导入到一个**全新的实例化区域（Fresh Instance Region）**中，代入当次的 `wide = false` 或 `wide = true` 独立完成规约！

```flurry
test "staged defer evaluation and clean instantiation v2" {
    -- 实例 1：wide = false，在独立实例区域内将 defer SelectType(false) 规约为 i32
    let small = keep_selected(false, 3i32);
    assert(small == 3i32);
    assert(small'type == i32);

    -- 实例 2：wide = true，在另一个全新实例区域内规约为 i64，绝不与实例 1 互相污染！
    let large = keep_selected(true, 7i64);
    assert(large == 7i64);
    assert(large'type == i64);
}
```

---

## 四、 本篇小结

| 核心概念 | 语法 / 机制表现 | 语义铁律与工程保障 |
| :--- | :--- | :--- |
| **纯编译期函数** | `pure comptime fn F(...)` | 承诺相同输入产生确定性相同结果；要求实参满足编译期 `Eq` 与 `Hash` 才启用记忆化 |
| **失败不入缓存** | 诊断错误 / 步数预算耗尽 | 任何求值失败或超预算中断绝不写入成功缓存，防止后续查询读到残损结果 |
| **Normal-Form 头** | `pure comptime fn H<T> -> Type` | 尖括号 `<...>` 构造器永远保留头身份，`Identity<T> != T` |
| **延后类型计算** | `defer SelectType(wide)` | 在定义期保留为依赖 `wide` 的合法残余计算，待 `wide` 具象化后再规约 |
| **定义冻结边界** | `freeze_definition` | 只冻结稳定绑定、契约与合法残余项；**严禁冻结**活动推导变量、等待者或候选事务 |

---

## 五、 留给下一篇的悬念

在上面的 `keep_selected` 中，参数类型写成了 `defer SelectType(wide)`。
你可能会好奇：既然在 `keep_selected` 的定义期，`SelectType(wide)` 还没法算出到底是 `i32` 还是 `i64`，那编译器在检查 `keep_selected` 的函数体 `return value;` 时，是怎么看待 `value` 的类型的？

更进一步，如果我直接在函数签名里写 `fn pass_through(value: Any) -> Any { value }`，这里的 **`Any`** 究竟是什么？把 `Any` 放进静态强类型的 Flurry 里，难道会让 Flurry 退化成像 Python 或 TypeScript `any` 那样要在运行期装箱（Boxing）、查虚表、甚至失去类型安全的动态语言吗？

下一回 **《Flurry 演进手记 (079)：在类型里写 Any 会变成动态语言吗——编译期渐进依赖类型（Gradual Dependent Type）》**，我们将揭晓 `Any` 与 `defer` 如何在**零运行期开销**的前提下，实现丝滑的编译期渐进类型检查！
