# Flurry 演进手记 (071)：在定义时抓出泛型 Bug——Normalization（定义端符号检查）与 Instantiation（调用端实例化）

如果你维护过大型 C++ 模板库或者 Zig 的 `comptime` 泛型代码，你一定经历过这种令人困惑的“定时炸弹”时刻：
你写好了一个泛型算子函数，跑了一遍 `cargo build` / `zig build`，编译器绿灯全亮，你高高兴兴地发布了新版本。结果三天后，下游用户在用某个具体类型调用你的函数时，编译器突然在你的库源码第 284 行爆出一百行错误——原来你在某个分支里手滑把变量名拼错了，或者调用了一个根本没在约束里声明的方法。

为什么编译器在你写下这个函数的时候不提醒你？
因为 C++ 模板和传统无契约 `comptime` 把所有的检查都推迟到了**实例化（Instantiation）**阶段——只要没人调用，泛型函数体就是一团躺在源码里的“半免检文本”。

而在设计 Flurry 的编译期系统时，我们既想要 `comptime fn` 直接用普通代码计算类型的爽快感，又想要像正统类型论一样**在定义泛型函数的当场就抓出所有越权操作和逻辑漏洞**。

怎么做到鱼与熊掌兼得？
答案就是 Flurry 编译期系统的核心双阶段引擎：**Normalization（定义端符号化规范检查）** 与 **Instantiation（调用端具体实例化）**。

---

## “不调用就不报错”的 Duck-Typing 泛型之痛

假设我们在写张量库的一个泛型缩放算子 `scale_first`。我们在 `where` 子句里只写了 `T :- Copy, n: usize, requires n > 0usize`，却忘了声明 `T :- Mul<T, Output = T>`，甚至还在函数体里写了一个明显的错误操作：

```flurry
-- ❌ 设想在 Duck-Typing 模板语言中：
-- 只要没人调用 scale_first，这段漏写约束的代码居然能安静地“编译通过”。
fn scale_first(values: Array<T, n>, factor: T) -> T
where
    T :- Copy,
    n: usize,
    requires n > 0usize
{
    -- 漏洞：T 只声明了 Copy，根本没有声明可以做乘法 (*) 或调用 .non_existent()。
    values[0usize] * factor
}
```

对于一个严肃的系统级与 AI 基础设施语言来说，“不调用就不检查”是不可接受的。库作者必须在**写下函数定义的那一刻**，就知道函数体是否严格遵守了 `where` 子句所承诺的契约边界。

---

## Normalization——没有具体实参，怎么在定义端检查？

在 Flurry 中，**无论是运行时函数（`fn`）还是编译期函数（`comptime fn`），在定义时都必须立刻经历第一阶段：Normalization（定义端规范化与符号检查）。**

你可能会好奇：在定义 `first_element(values: Array<T, n>)` 的时候，编译器还不知道 `T` 具体是 `f32` 还是 `i32`，也不知道 `n` 具体是 `4` 还是 `128`，它怎么检查函数体呢？

答案是：**符号化求值（Symbolic Evaluation）与类型命题收集（PTC）。**

在 Normalization 阶段，编译器会把未知的参数 `T` 和 `n` 当作携带约束的**符号值（Symbolic Values）**，沿着函数体收集以下四类信息来验证每一个操作：
1. **参数显式约束**：例如 `where` 子句中声明的 `T :- Copy`、`n: usize` 以及 `requires n > 0usize`；
2. **控制流路径条件（Path Conditions）**：例如进入 `if i < n { ... }` 的真分支后，当前路径自动获得命题 `i < n`；
3. **被调用操作的契约结果**：例如调用一个保证返回值满足某性质的辅助函数后，获得的后置命题；
4. **`assumes` 编译期谓词提供的类型命题**（我们马上在第二部分中展示）。

```
+-------------------------------------------------------------------------+
|  阶段一 Normalization（定义端符号检查）                        |
|                                                                         |
|  函数定义：                                                             |
|  fn head_plus(values: Array<T, n>) -> T                                 |
|  where T :- Copy, n: usize, requires n > 0usize                         |
|  {                                                                      |
|      let x = values[0usize];      -- 检查 ①：0usize < n 成立吗？        |
|      return x + x;                -- 检查 ②：T 支持 + (Add) 吗？        |
|  }                                                                      |
|                                                                         |
|  Normalization 符号检查引擎（定义当场立刻运行，无需任何调用者  ）：     |
|  +-------------------------------------------------------------------+  |
|  | 已知符号环境：                                                    |  |
|  |   • T : Type,  命题 { T :- Copy }                                 |  |
|  |   • n : usize, 命题 { n > 0usize }                                |  |
|  |                                                                   |  |
|  | ──► 检查 ① values[0usize]：                                       |  |
|  |     数组下标要求 0usize < n。由已知 { n > 0usize } 推出成立  ✅   |  |
|  |                                                                   |  |
|  | ──► 检查 ② x + x：                                                |  |
|  |     运算符 + 要求命题 { T :- Add<T> }。                           |  |
|  |     在已知命题集 { T :- Copy } 中查找... 未找到                   |  |
|  |     ❌ 当场在定义位置报错：类型 `T` 未满足 `Add` 约束             |  |
|  +-------------------------------------------------------------------+  |
+-------------------------------------------------------------------------+
```

不仅如此，Normalization 还会对函数签名与函数体中那些**已经具备完整编译期输入**的子表达式直接归约化简，而把依赖未知参数 `T`、`n` 的部分保留为经过类型与契约验证的**残余形式（Residual Form）**。

来看一个被 Normalization 在**定义当场**直接拦截的例子：

```flurry
-- ❌ 编译期在定义处直接拒绝。（哪怕全项目没有任何地方调用它）
fn bad_generic_add(a: T, b: T) -> T
where
    T :- Copy
{
    -- Normalization 检查报错：
    -- 符号类型 `T` 仅声明了 `T :- Copy`，无法证明 `T :- Add<T, Output = T>`，
    -- 因此 `a + b` 在定义阶段即非法。
    a + b
}
```

只需补上 `T :- Copy + Add<T, Output = T>`，Normalization 就能证明 `a + b` 合法，放行该函数定义。

---

## Instantiation 与 `comptime predicate` 的跨阶段协作

既然定义端已经有 Normalization 了，那**Instantiation（调用端具体实例化）** 又是干什么的呢？

当调用者写下具体调用（例如传入 `T = f32, n = 4usize`）时，编译器启动 **Instantiation**：
1. **代入具体实参**：把定义端保留的符号参数 `T`、`n` 替换为具体的 `f32` 和 `4usize`；
2. **继续完成编译期计算**：把定义端因为缺少具体值而挂起的残余编译期表达式彻底算完；
3. **兑现 `assumes` 谓词与 Coherence 检查**：对具体类型和具体编译期值执行最终验证，并生成针对该具体类型的特化实例。

为了让你直观看到 **Normalization（定义端）** 与 **Instantiation（调用端）** 是如何无缝接力的，我们来看看 Flurry 中专门连接这两个阶段的精妙机制——**`comptime predicate` 与 `assumes`**。

在构建张量库时，我们有时会编写一个复杂的自定义编译期类型检查函数，比如判断一个类型 `T` 是否同时实现了 `Clone` 和 `Debug`：

```flurry
-- 定义一个编译期谓词（comptime predicate）：
-- 它既是一个返回 bool 的编译期检查函数，又能向类型检查器提供命题。
comptime predicate fn clone_and_debug(T: Type) -> bool {
    T :- Clone and T :- Debug
}

-- 在函数签名中用 assumes 引用该谓词：
fn inspect_tensor_elem(x: T) -> T
where
    T,
    assumes clone_and_debug(T)
{
    -- 在函数体中直接调用 .clone()。
    x.clone()
}
```

这段代码在两个阶段分别发生了什么？请看下面的双阶段协作图：

```
+-------------------------------------------------------------------------+
|  Normalization 与 Instantiation 的双阶段接力                   |
|                                                                         |
|  【阶段 1：Normalization（定义 inspect_tensor_elem 时）】               |
|    • 此时 T 还是未知的符号类型。                                        |
|    • 编译器分析 `assumes clone_and_debug(T)` 的谓词函数体，             |
|      提取出类型命题：`T :- Clone` 且 `T :- Debug`                       |
|    • 检查函数体 `x.clone()`：因为已知命题中有 `T :- Clone`，            |
|      定义端符号检查顺利通过  ✅                                         |
|                                                                         |
|  ============================== 跨越阶段 ============================== |
|                                                                         |
|  【阶段 2：Instantiation（调用 inspect_tensor_elem(val) 时）】          |
|    • 情况 A：传入 val: i32                                              |
|      编译器用具体类型 T = i32 真正执行 `clone_and_debug(i32)`           |
|      ──► 返回 true ──► 实例化成功  ✅                                   |
|                                                                         |
|    • 情况 B：传入 val: RawHandle（未实现 Clone 的类型）                 |
|      编译器用具体类型 T = RawHandle 执行 `clone_and_debug(RawHandle)`   |
|      ──► 返回 false ──► ❌ 在调用点报错：不满足 assumes 谓词契约        |
+-------------------------------------------------------------------------+
```

```flurry
struct RawHandle {
    fd: i32,
}

fn test_instantiation(handle: RawHandle) {
    -- ✅ i32 同时满足 Clone 与 Debug，Instantiation 阶段谓词求值为 true
    let ok = inspect_tensor_elem(42i32);

    -- ❌ 编译期在调用端（Instantiation 阶段）直接拒绝。
    -- 代入 T = RawHandle 执行 clone_and_debug(RawHandle) 得到 false，
    -- 违反了 inspect_tensor_elem 的 assumes clone_and_debug(T) 契约。
    let bad = inspect_tensor_elem(handle);
}
```

> **💡 核心心智模型：Normalization 与 Instantiation 适用于所有函数。**
> 在 Flurry 中，不要把 Normalization/Instantiation 和“编译期函数/运行时函数”混为一谈：
> - **不管是 `comptime fn` 还是普通的运行时 `fn`**，只要它有声明，在定义时都会经历 **Normalization**，在被具体实参调用时都会经历 **Instantiation**。
> - Normalization 保证了**定义端不留暗病**（符号契约自洽）；Instantiation 保证了**调用端彻底特化与兑现**（把依赖具体实参的编译期计算算到底）。

---

## 本篇小结

| 阶段名称 | 触发时机 | 已知信息 | 核心工作职责 |
| :--- | :--- | :--- | :--- |
| **Normalization（规范化）** | **定义端**（写下函数/类型定义时立即执行） | 参数的声明类型、`where`/`requires`/`assumes` 契约、分支路径条件 | 1. 算完已确定输入的编译期子表达式<br>2. 用符号命题验证未知参数上的每个操作是否越权<br>3. 生成类型安全的残余形式（Residual Form） |
| **Instantiation（实例化）** | **调用端**（传入具体类型或编译期实参时执行） | 具体的 `Type` 实参、具体的编译期数值/配置实参 | 1. 将具体实参代入残余形式<br>2. 继续执行挂起的编译期求值与 `assumes` 谓词<br>3. 检查 Coherence 并生成最终特化实例 |

---

### 下一篇预告

既然在**Normalization（定义端）** 中，编译器已经尽量去计算编译期表达式并做符号检查了，那么问题来了：

如果我们在写一个**动态的 `comptime fn`**（比如根据一段格式化字符串 `"Tensor({}): shape={}"` 动态拼装出一个元组类型），在定义这个 `comptime fn` 的时候，格式化字符串参数 `fmt_str` 还是未知的符号变量。
如果编译器在定义端用过于严苛的规则去检查它，会不会把本该等到具体字符串传入后才能算出来的类型操作给误杀掉？
反过来，如果在一个普通的运行时函数签名里，某个参数的类型必须调用一个只有在调用端拿到具体值后才能算出来的类型函数 `Selected(flag)`，我们又该如何告诉编译器“**请把这个类型表达式的具体归约推迟到 Instantiation 阶段**”？

下一篇 **`072-ordinary-loose-reduction-and-defer.md`**，我们将揭晓与“执行阶段（comptime/runtime）”正交的另一个核心维度：**Ordinary（严格检查） vs Loose（宽松检查）**，以及专门用来精细控制求值时机的 **`defer` 延迟表达式**。
