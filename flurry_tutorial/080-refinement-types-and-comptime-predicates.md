# Flurry 演进手记 (080)：给类型加上逻辑筛网——精化类型 refines { expr } T 与编译期谓词

在上一回（079）中，我们认识了编译期渐进类型中的 `Any`，也看到了它的局限：裸写 `value: Any` 时，编译器对 `value` 的形态一无所知；而写死具体类型 `Tuple<u32, bool>` 又失去了接纳任意元组的通用性。

回到我们的静态形状张量引擎 `Tensor<T, .dims = s>`：
1. 当我们编写一个只接受“元组类型”作为张量通道包的编译期函数时，怎么在类型签名上精确表达**“这是一个满足 `is_tuple` 的类型”**？
2. 当我们在编译期计算张量维度的切片下标 `next_checked_index(index, length)` 时，怎么把**前置条件（`requires`）**、**后置契约（`ensures`）**与**类型自身的精化约束（`refines`）**无缝串联起来，让越界的下标在编译期当场现形？

今天，我们就来为类型装上削铁如泥的逻辑筛网——**精化类型（Refinement Types，`refines { expr } T`）**与**编译期谓词（`comptime predicate`）**。

---

## 一、 问题背景与设计思路：靠函数体里的 `assert` 守门有什么问题？

在没有精化类型的语言里（传统做法），如果我们想限制一个编译期函数只接受正整数或只接受元组，通常只能把检查塞进函数体内部的 `assert` 里：

```flurry
-- ❌ 传统的被动防御：签名上看不出任何约束，全靠进门后再 assert
pure comptime fn bad_checked_index(index: Integer, length: Integer) -> Integer {
    assumes 0 <= index and index < length;
    index
}
```

这种做法有三个严重问题背景：
1. **接口契约隐形**：调用者只看函数签名 `(index: Integer, length: Integer) -> Integer`，根本无从得知 `index` 和 `length` 之间有什么大小关系要求。
2. **报错位置滞后**：一旦出错，诊断信息指向的是库函数内部的 `assert` 行，而非调用者传入非法参数的犯罪现场。
3. **无法进行逻辑传递（Chaining）**：如果你先调了一个函数算出 `step = next_index(index)`，再把 `step` 传给 `bad_checked_index(step, length)`，编译器在符号推导阶段无法利用前一个函数的返回契约去自动证明后一个函数的入口条件。

---

## 二、 核心语法：`refines { expr } T` 与双生代词 `Itself` / `itself`

在 Flurry 中，精化类型的语法形式是前缀修饰：

$$\text{refinement\_type} \rightarrow \text{\texttt{refines}}\ \{\ expr\ \}\ expr$$

它的直观含义就像一个漏斗：**先取右侧的基础类型（Base Type），再用花括号 `{ expr }` 里的编译期布尔命题作为筛网，只有令命题成立的成员才属于这个精化类型。**

```
         基础域 (如 Any 或 Integer)
       \                           /
        \   [ 逻辑筛网: { expr } ]  /
         \  • Itself (指代目标类型) /
          \ • itself (指代目标值)  /
           \_________   _________/
                     | |
                     v v
          精化后的子集类型 (Refined Type)
```

在编写 `{ expr }` 筛网表达式时，我们经常需要指代“当前正在被筛查的那个对象本身”。为此，Flurry 提供了两个层次分明的内置代词（Pronouns）：

| 代词 | 大小写特征 | 绑定的目标层次 | 典型使用场景 |
| :--- | :--- | :--- | :--- |
| **`Itself`** | 首字母大写（对应 `Type` 层） | 当前正在被精化或检查的**目标类型（Target Type）** | 筛选满足特定结构谓词的类型，如 `refines { Type.predicates.is_tuple(Itself) } Any` |
| **`itself`** | 全小写（对应值层） | 当前正在被精化或检查的**目标值（Target Value）** | 筛选满足数值或属性条件的值，如 `refines { itself > 0 } usize` |

> **🔍 语法小贴士：大小写即宇宙层次。**
> 就像我们在第 076、079 回看到的 `Type` vs `type`、`Any` vs `any` 一样，Flurry 始终保持着十分一致的视觉直觉：**首字母大写的 `Itself` 指代类型（`Type`），全小写的 `itself` 指代值（Value）**。一眼扫过去，你就知道筛网是在筛类型本身，还是在筛类型里的值。

---

## 三、 升级：用 `Itself` 筛选元组类型，用 `comptime predicate` 封装逻辑

让我们先用 `refines { Type.predicates.is_tuple(Itself) } Any` 来升级张量通道包的接收器 `accept_tuple`：

```flurry
-- value 的基础类型是 Any，但被 `Type.predicates.is_tuple(Itself)` 精化：
-- 只有当传入值的类型 Itself 是一个 Tuple<...> 时，才允许通过。
fn accept_tuple(
    value: refines { Type.predicates.is_tuple(Itself) } Any,
) -> Any {
    value
}

-- 我们还可以用 `comptime predicate` 定义可复用的命名逻辑谓词
comptime predicate is_valid_rank(rank: Integer) {
    1 <= rank and rank <= 8
}

test {
    let pair: Tuple<u32, bool> = (7u32, true);
    let echoed = accept_tuple(pair);
    assumes echoed.0 == 7u32;
    assumes echoed.1;
    assumes is_valid_rank(4);
}
```

如果有人试图把一个普通的标量整数 `7u32` 塞进只收元组的 `accept_tuple`：

```flurry
-- ❌ 编译期直接报错：7u32 的类型是 u32，令 Type.predicates.is_tuple(Itself) 为 false，无法通过精化筛网
fn reject_non_tuple() {
    let bad = accept_tuple(7u32);
}
```

---

## 四、 进阶：`requires` 与 `ensures` 的编译期契约接力

精化约束非但能直接写在类型前缀 `refines { ... }` 上，还能以 `where requires ...` 和 `where ensures ...` 子句的形式写在函数签名上。

特别要注意的是：在编译期元编程中，我们使用的是数学整数类型 **`Integer`**（任意精度数学整数，没有机器字长回绕溢出问题）。当多个 `pure comptime fn` 组合调用时，前一个函数的 `ensures`（后置保证）会自动成为当前上下文的已知事实，用来证明后一个函数的 `requires`（前置要求）。

让我们来看张量维度索引递进器 `next_checked_index` 的契约接力全过程：

```
  [ 调用方入口条件 ]
  requires: 0 <= index and index + 1 < length
       │
       ▼
  [ 第 1 棒：调用 step = next_index(index) ]
  • next_index 的后置契约 ensures: result == index + 1
  • 编译器获得新事实：step == index + 1
       │
       ▼
  [ 第 2 棒：调用 checked_index(step, length) ]
  • checked_index 的前置要求 requires: 0 <= step and step < length
  • 编译器代入 step == index + 1 与入口条件：
    0 <= index ──► 0 <= index + 1 (即 0 <= step)  ✅
    index + 1 < length ──► step < length          ✅
  • 契约证明自动闭合。
```

来看真实可运行的 进阶代码：

```flurry
-- 第 1 棒：承诺返回值 result 严格等于 index + 1
pure comptime fn next_index(index: Integer) -> (result: Integer)
where
    ensures result == index + 1
{
    index + 1
}

-- 第 2 棒：要求传入的 index 必须处于 [0, length) 半开区间内
pure comptime fn checked_index(index: Integer, length: Integer) -> Integer
where
    requires 0 <= index and index < length
{
    index
}

-- 组合接力：只需在入口声明 0 <= index and index + 1 < length，
-- 内部调用 checked_index(step, length) 的前置条件就能由 next_index 的 ensures 自动证出。
pure comptime fn next_checked_index(index: Integer, length: Integer) -> Integer
where
    requires 0 <= index and index + 1 < length
{
    let step = next_index(index);
    checked_index(step, length)
}

test {
    -- 2 + 1 < 4 成立，编译期顺利通过并算出 3
    const idx = next_checked_index(2, 4);
    assumes idx == 3;
}
```

如果调用者在编译期传入了会导致下一步越界的 `index = 3, length = 4`（此时 `3 + 1 < 4` 为假）：

```flurry
-- ❌ 编译期直接报错：传入 (3, 4) 违反了 next_checked_index 的前置契约 `index + 1 < length`
comptime {
    let out_of_bounds = next_checked_index(3, 4);
}
```

---

## 五、 语义红线：PTC 编译期命题约束 vs 程序验证层

在结束本篇之前，我们必须重申一条贯穿 Flurry 架构的黄金边界：

> **💡 核心心智模型：编译期命题（PTC）与运行期程序验证（Program Verification）各司其职。**
> - 本篇所讲的 `comptime predicate`、类型上的 `refines { ... }` 以及纯编译期函数上的 `requires / ensures`，全部属于 **PTC（Propositional Type Constraints，命题类型约束）** 的管辖范围。它们只处理**类型关系与编译期纯值命题**。
> - 至于运行期可变内存状态、指针别名修改、借用生命周期以及所有权资源规格（我们在第 3 卷第 060 回见过的运行期契约），则由专门的**程序验证层（Program Verification Layer）**负责。两者不混谈，从而保证了类型检查器本身的纯粹与高效。

---

## 六、 本篇小结

| 语法构造 | 核心角色 | 语义契约与注意点 |
| :--- | :--- | :--- |
| `refines { expr } T` | 精化类型前缀 | 在基础类型 `T` 上附加编译期布尔筛网 `expr` |
| `Itself` | 类型级自指代词 | 在精化或类型约束中指代当前被检查的**目标类型** |
| `itself` | 值级自指代词 | 在值精化约束中指代当前被检查的**目标值** |
| `comptime predicate P(...)` | 命名编译期谓词 | 封装可复用的纯编译期布尔逻辑命题 |
| `where requires expr` | 前置契约子句 | 调用方在传入参数时必须被证明成立的命题 |
| `where ensures expr` | 后置契约子句 | 函数返回时向调用方提供的已证事实（可绑定返回值名如 `(result: Integer)`） |

---

## 七、 下一篇预告

到现在为止，我们的张量形状和数组长度（比如 `Array<f32, 4>`）依赖的都是**编译期已知常数**。

但是在真实的科学计算与网络协议解析中，大量的张量维度和缓冲区长度是在**运行期**从文件头、网络包或用户输入里读出来的（比如 `let n: usize = read_dim();`）。
在其他系统级语言里，一旦长度来自运行期变量，你就禁止再用定长数组类型 `[f32; N]`，而必须退化到另一套完全割裂的切片或动态容器语法（如 `&[f32]` 或 `Vec<f32>`）；更糟糕的是，如果你用整数下标 `u32` 去索引某个图节点池 `NodeStorage`，你根本没法在类型系统里阻止“拿 A 图的节点下标去查 B 图的数组”。

如果在 Flurry 里，**一个类型非但能依赖编译期常数，还能直接依赖一个具有稳定身份的运行期变量（如 `Array<f32, n>` 或 `NodeIndex<storage.ref>`）**呢？

下一回 **《Flurry 演进手记 (081)：当数组长度来自运行时变量——运行时依赖类型（Runtime Dependent Types）入门》**，我们将推开运行期依赖类型的大门。
