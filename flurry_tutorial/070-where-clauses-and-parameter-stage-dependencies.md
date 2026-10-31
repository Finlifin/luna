# Flurry 演进手记 (070)：把类型参数写进 where 子句——子句声明、参数阶段依赖与隐式类型推导

在阅读 C++ 或 Rust 的重度泛型库（比如矩阵运算库 Eigen、深度学习框架 Burn）时，你一定经历过这种被“尖括号海啸”支配的恐惧：
你想看清一个算子函数名叫什么、接收什么参数，但函数名后面先挂着三行半的 `<T: Float + Copy + Send, const B: usize, const C: usize, const H: usize, const W: usize, Acc = f32>`，等你翻到真正的函数参数 `(input: Tensor<...>)` 时，早已忘了这函数本来要干嘛。

而在 Flurry 中，当你翻开静态形状张量库 `Tensor` 的源码时，看到的却是这样清爽如诗的签名：

```flurry
fn conv2d_first_channel(
    input: Tensor<T, b, c, h, w>,
) -> Tensor<T, b, 1usize, h, w>
where
    T :- Copy,
    b: usize,
    c: usize,
    h: usize,
    w: usize,
    requires c > 0usize
```

第一眼看到这段代码，你的注意力立刻聚焦在最核心的业务签名上：`conv2d_first_channel(input: Tensor<T, b, c, h, w>) -> Tensor<T, b, 1usize, h, w>`。至于 `T` 是类型参数、`b/c/h/w` 是编译期 `usize` 维度参数、以及 `c > 0usize` 的前置条件，全都整整齐齐地收纳在末尾的 `where` 子句里。

此外：既然 `T, b, c, h, w` 写在函数参数列表**后面**的 `where` 子句里，为什么前面的参数 `input: Tensor<T, b, c, h, w>` 却能直接引用它们？编译器到底按什么顺序解析参数阶段依赖？调用端又是怎么省去书写 `<f32, 1, 3, 224, 224>` 就能自动完成隐式推导的？

今天，我们就来拆解 Flurry 的 `where` 子句声明体系与**参数阶段依赖（Parameter Stage Dependencies）**。

---

## 前置尖括号 `<...>` 的喧宾夺主与依赖扭曲

在传统泛型语言里，泛型参数列表 `<T, const N: usize>` 必须强行塞在函数名和普通圆括号参数 `(...)` 中间：

```
传统语言签名：
fn reduce_sum<T: Add + Default + Copy, const N: usize, Acc: From<T> = T>(data: Array<T, N>) -> Acc
             └─────────────────── 喧宾夺主的前置泛型噪音 ──────────────────┘
```

这种设计有两个明显的核心问题：
1. **阅读主次颠倒**：读者最关心的是“函数叫什么、输入输出是什么结构”，结果视线先撞上一大堆类型变量声明和 trait 约束。
2. **子句能力割裂**：有些语言既要在函数名后写 `<T, const N: usize>`，又要在函数签名末尾写 `where T: Copy`，同一个参数 `T` 的声明和约束被劈成两半，散落在签名的首尾两端。

在 Flurry 中，对于运行时函数、`struct`、`enum` 和 `trait`，语言设计者做出了一个十分干脆的决定：**将所有子句参数（Clause Parameters）与编译期前置约束统一收归到 `where` 子句中。**（注：仅返回类型的类型函数如 `Tuple<...>` 允许在头部使用 `<...>` 以匹配类型构造器语法，但其语义与 `where` 子句参数完全同构。）

---

## `where` 子句的百宝箱——五种核心子句声明形式

在 Flurry 的语法规范（`docs/grammar/syntax` 与 `docs/functions/comptime_fn.md`）中，`where` 后面跟着的是一组有序的子句项（`where_clause_item`）。让我们结合张量库的真实需求，把最常用的几种武器逐一亮出来：

### 1. 裸类型参数与 Trait 约束简写（`T` 与 `T :- Trait1 + Trait2`）

在 `where` 子句中，如果你只写一个标识符 `T`，它就是 `T: Type` 的语法简写。如果它还需要满足正面的 trait 约束，可以直接写 `T :- Trait1 + Trait2`：

```flurry
-- 1. 裸写 T 等价于 T: Type
fn keep(x: T) -> T where T {
    x
}

-- 2. T :- Copy + Clone 是以下完整写法的语法糖：
--    where T: Type, requires T :- Copy, requires T :- Clone
fn duplicate(x: T) -> Tuple<T, T>
where
    T :- Copy + Clone
{
    (x, x)
}
```

> **🔍 语法小贴士：否定约束（`not (T :- Trait)`）写在哪里？**
> 注意，声明位置的简写 `T :- Trait1 + Trait2` 只用于正向的 trait 约束。如果你需要表达否定约束（例如要求某个类型**不**实现 `Copy`），则需要写在独立的 `requires` 子句中：`where T, requires not (T :- Copy)`。

### 2. 编译期值子句参数（`n: usize`）与 `requires` 前置条件

在 2LTT 体系下，`where` 子句里的参数非但可以是 `Type`，还可以是任意可出现在类型签名中的编译期参数（比如数组长度 `n: usize`、标志位 `flag: bool`、形状列表 `s: meta.List<usize>`、闭包环境 `c: Type.vars.Closure`、可达集 `r: Type.vars.ReachSet`），并且紧接着就能用 `requires` 对这个编译期参数施加约束。

> **📏 子句参数命名规范**：在 `where` 子句和 `<...>` 参数列表中，**只有类型参数（`: Type` 或 `:- Trait`）和类型构造子参数（`-> Type`）使用大驼峰（`PascalCase`）命名**；所有并非类型子句参数（如 `n: usize`、`s: meta.List<usize>`、`format: Symbol`、`c: Type.vars.Closure`、`r: Type.vars.ReachSet` 等）一律使用 `snake_case` 命名。

```flurry
fn first_element(values: Array<T, n>) -> T
where
    T :- Copy,
    n: usize,
    requires n > 0usize
{
    values[0usize]
}
```

### 3. 带默认值的可选子句参数（`.Acc: Type = T`）

在神经网络的归约算子（如 `sum`、`mean`）中，输入张量往往是低精度的 `f16` 或 `i8`，我们默认用同类型累加，但也允许调用者显式指定高精度的累加器类型（如 `f32` 或 `i32`）以防止溢出。
在 Flurry 的 `where` 子句中，以点号 `.` 开头并带有 `= default_expr` 的就是**可选子句参数（Optional Clause Parameter）**，而且它的默认值可以直接依赖前面已经声明的子句参数。

```flurry
-- 在 struct 中使用可选子句参数：默认第二个类型 B 等于第一个类型 A
struct TensorPair where A, .B: Type = A {
    left: A,
    right: B,
}

-- 在函数中使用可选子句参数：默认累加类型 Acc 等于元素类型 T
fn tensor_sum(input: Array<T, n>, init: Acc) -> Acc
where
    T :- Copy,
    n: usize,
    .Acc: Type = T
{
    init
}
```

### 4. 变参子句参数（`...Es: meta.List<Type>`）与引用子句参数（`quote`）

正如我们在第 069 篇看到的，`...Es: meta.List<Type>` 可以收集变长的类型参数列表；而 `quote` 子句参数则用于元编程中捕获语法绑定描述（我们将在第 073 篇深入探索）。

---

## 揭秘参数阶段依赖（Stage Dependency Order）与调用端推导

现在我们来回答开头那个最核心的问题：
**为什么 `where T, n: usize` 写在函数签名最后面，前面的圆括号参数 `(values: Array<T, n>)` 却能认出 `T` 和 `n`？**

因为在 Flurry 中，**源码的书写位置（为了人类阅读体验把 `where` 放后面）**与**编译器的语义绑定阶段（Stage Dependency Order）**是明确分离的。

下面通过图示画出编译器处理函数签名的真实流水线：

```
+-------------------------------------------------------------------------+
|  函数签名的三阶段依赖流水线（Stage Dependency Order）          |
|                                                                         |
|  源码书写顺序：                                                         |
|  fn slice_head(arr: Array<T, n>, len: usize) -> Array<T, 1usize>        |
|  where T :- Copy, n: usize, .Out: Type = T, requires n > 0usize         |
|                                                                         |
|  编译器语义解析顺序（严格从阶段 ① -> 阶段 ② -> 阶段 ③）：               |
|                                                                         |
|  【阶段 ①：按序建立 where 子句参数（Clause Parameters）】               |
|    ┌─ 1a. 绑定 T: Type                                                  |
|    ├─ 1b. 绑定 n: usize                                                 |
|    └─ 1c. 绑定 .Out: Type = T  （后面的子句参数可以依赖前面的 T  ）     |
|                      │                                                  |
|                      ▼                                                  |
|  【阶段 ②：在子句绑定环境中，建立普通参数（Ordinary Parameters）】      |
|    ├─ 2a. 解析 arr: Array<T, n> （此时 T 和 n 已经在作用域中  ）        |
|    └─ 2b. 解析 len: usize                                               |
|                      │                                                  |
|                      ▼                                                  |
|  【阶段 ③：建立返回类型与验证契约（Return Type & Contracts）】          |
|    ├─ 3a. 解析返回类型 -> Array<T, 1usize>                              |
|    └─ 3b. 解析契约 requires n > 0usize / ensures ...                    |
|           （契约与返回类型可以同时依赖阶段 ① 的子句参数和阶段 ② 的参数）|
+-------------------------------------------------------------------------+
```

这条三阶段依赖规则清晰明确：
1. **子句参数内部有序**：在 `where` 子句内部，排在后面的子句参数（比如 `.Out: Type = T` 或 `copy: T = value`）可以依赖排在它前面的子句参数，但反过来不行。
2. **普通参数依赖子句参数**：圆括号 `( ... )` 里的普通参数类型在阶段 ① 建立好的环境中解析，因此天然可以使用 `T` 和 `n`。
3. **调用端的双向畅通（隐式推导 + 显式指定）**：
   在调用像 `first_element(my_array)` 这样的函数时：
   - **默认隐式推导**：编译器会根据你传入的实参 `my_array` 的类型（比如 `Array<f32, 4usize>`），自动逆向推导出阶段 ① 的子句参数 `T = f32, n = 4usize`，并检查 `requires 4usize > 0usize`。
   - **显式 `<...>` 实参或元组展开**：当某个子句参数没有出现在普通参数类型里（或者你想覆盖可选子句参数 `.Acc` 的默认值），你可以在调用时显式传入 `<...>`，甚至用 `...` 将编译期函数返回的元组直接展开进 `<...>`。

来看一段展示隐式推导、显式指定与编译期拦截的完整代码：

```flurry
-- 一个编译期辅助函数，返回类型为 Tuple<Type, usize> 的编译期元组
pure comptime fn f32_vec4_packet() -> Tuple<Type, usize> {
    (f32, 4usize)
}

fn first_element(values: Array<T, n>) -> T
where
    T :- Copy,
    n: usize,
    requires n > 0usize
{
    values[0usize]
}

fn demo_calls(arr4: Array<f32, 4usize>, empty: Array<f32, 0usize>) {
    -- ✅ 方式 1：隐式推导。
    -- 编译器从 arr4: Array<f32, 4usize> 自动推导出 T = f32, n = 4usize
    let a = first_element(arr4);

    -- ✅ 方式 2：显式传入子句参数 <f32, 4usize>
    let b = first_element<f32, 4usize>(arr4);

    -- ✅ 方式 3：用 ... 把编译期元组 (f32, 4usize) 展开进子句参数列表。
    let c = first_element<...f32_vec4_packet()>(arr4);

    -- ❌ 编译期直接报错。
    -- 推导得出 T = f32, n = 0usize，违反了 where 子句中的 requires n > 0usize。
    let bad = first_element(empty);
}
```

再看一个违反**子句参数顺序依赖**的编译期报错负例：

```flurry
-- ❌ 编译期直接报错。
-- 在阶段 ① 建立 where 子句参数时，按从左到右的顺序解析：
-- 解析 .B: Type = A 时，A 还没声明。子句参数必须先声明、后引用。
struct BadPair where .B: Type = A, A {
    left: A,
    right: B,
}
```

---

## 本篇小结

| 核心机制 | 语法形式 | 语义与阶段规则 |
| :--- | :--- | :--- |
| **裸类型与约束简写** | `where T` / `where T :- Copy + Clone` | `T` 简写自 `T: Type`；`:-` 声明正面 trait 约束（负面约束写进 `requires`） |
| **编译期值子句参数** | `where n: usize, requires n > 0usize` | 将维度、配置常量等编译期值纳入类型签名，并配合 `requires` 做静态校验 |
| **可选子句参数** | `where A, .B: Type = A` | 以 `.` 开头并提供默认值表达式，未显式指定时自动使用默认值 |
| **三阶段依赖顺序** | ① `where` 子句参数 $\rightarrow$ ② 普通参数 $\rightarrow$ ③ 返回类型与契约 | 源码把 `where` 放末尾保持阅读清爽，语义上阶段 ① 最先按序建立绑定 |
| **调用端绑定** | `f(x)` / `f<T, n>(x)` / `f<...packet()>(x)` | 支持从实参类型自动推导，也支持显式 `<...>` 指定或编译期元组 `...` 展开 |

---

### 下一篇预告

现在，我们已经能写出简洁清晰的带约束泛型算子签名了：
`fn first_element(values: Array<T, n>) -> T where T :- Copy, n: usize, requires n > 0usize`

但是，思考一个关键问题：
如果在写 `first_element` 的**函数体内部**时，作者偶有疏忽写成了 `return values[n];`（越界访问。因为合法下标是 `0 .. n`），或者对没有声明 `Add` 约束的 `T` 写了 `return values[0usize] + values[0usize];`——
**编译器是在定义 `first_element` 的那一刻（哪怕全工程还没有任何人调用它）就会直接报错？还是像 C++ 模板 / Zig `comptime` 那样，等到某个人用具体类型调用它时才报错？**

下一篇 **`071-normalization-vs-instantiation-two-phases.md`**，我们将揭开 Flurry 编译期架构中最硬核的双阶段引擎：**Normalization（定义端符号检查）** 与 **Instantiation（调用端具体实例化）**。
