# Flurry 演进手记 (075)：把循环和分支在编译期彻底摊平——comptime { ... } 块与 inline if / inline for

欢迎来到《Flurry 演进手记》第四卷的收官之作。

在前九篇中，我们学会了用 `comptime fn` 在函数外部计算类型、塑造 `Tensor<T, ...>` 和 `Tuple<...Es>`。
但在真实的边缘端神经网络算子开发中，我们经常需要在**一个普通运行时函数的肚子里面**，直接嵌入编译期的计算、条件裁剪与循环展开：

1. **内联编译期计算块（`comptime { ... }`）**：在算子内部现场计算一个复杂的分块掩码或步长常量，要求编译期直接算出结果，运行时零开销；
2. **编译期分支裁剪（`inline if` / `inline when`）**：当张量的通道数 `c == 1usize` 时走单通道快速路径，当 `c == 4usize` 时走 SIMD 向量化路径，要求生成的目标机器码里**只包含被选中的那条分支**，没有任何运行时 `if` 跳转；
3. **编译期循环摊平与异构元组遍历（`inline for`）**：
   - 把固定小维度的卷积核循环 `0 .. 3` 在编译期彻底展开（Loop Unrolling）成顺序执行的指令；
   - 甚至直接用 `inline for (idx, item) in pack'tuple_enumerate` 遍历一个**每个元素类型都不同**的异构元组 `Tuple<A, B, C>`。

今天，我们就来看看 Flurry 是如何用 `comptime { ... }`、`inline if` 和 `inline for`，让你在运行时函数里享受丝滑的编译期代码内联展开，同时依然坚守类型安全与资源所有权底线的。

---

## 用 `comptime { ... }` 块在普通代码中开启 $U_1$ 任意门

有时候，我们只是想在一个运行时函数内部或者常量声明处做几步编译期运算，为此专门去顶层定义一个 `comptime fn` 未免太兴师动众了。

在 Flurry 中，你随时可以写一个 **`comptime { ... }` 块**。
正如我们在第 072 篇学到的，`comptime { ... }` 块内部运行在 $U_1$ 编译期阶段（并以 `loose` 宽松强度归约），它的求值结果会在编译期直接内联（或通过 `Splice` 落地）到外层代码中：

```flurry
-- 在模块顶层或函数内部使用 comptime { ... } 块计算编译期常量
-- 注意 Flurry 语法习惯：以 `}` 结尾的定义或块语句无需尾随分号
const TILE_AREA = comptime {
    let side = 16usize;
    side * side
}

fn compute_buffer_offset(block_idx: usize) -> usize {
    -- 在运行时函数内部直接嵌入 comptime { ... } 块：
    -- stride 在编译期就已经算好为 4096usize，运行时只做一次乘法。
    let stride = comptime {
        let page = 256usize;
        page * 16usize
    }
    block_idx * stride
}
```

---

## `inline if` 与 `inline when`——既要编译期裁剪分支，又要拒绝 C 语言 `#ifdef` 的失明症

在写高性能张量算子时，我们经常要根据编译期参数（比如 `comptime use_fast_math: bool` 或维度 `n: usize`）选择不同的实现路径。

如果在 C/C++ 里用 `#if` 预处理器宏，未被选中的分支对编译器来说就是一段死文本——哪怕你在未选中的分支里写错了类型、违反了借用规则，只要当前编译配置没打开那个宏，编译器就完全看不见。

而在 Flurry 中，当你在运行时函数（默认处于 `ordinary` 严格检查模式）中写下 **`inline if`**、**`inline when`** 或 **`inline if ... is ... do`** 时，编译器实现了真正的两全其美：

1. **在定义端 Normalization 阶段（双分支全检）**：
   在 `ordinary` 模式下，**无论编译期条件选没选中，所有分支都会接受完整的类型检查、契约检查与资源所有权检查。** 绝不允许未选中的分支藏匿语法或类型毒丸。
2. **在调用端 Instantiation 阶段（零开销裁剪）**：
   当具体编译期参数代入后，编译器在编译期直接选定命中的那条分支；**只有被选中分支里的运行时操作和副作用会进入最终生成的实例**，未被选中的分支在目标代码里彻底消失。

```
+-------------------------------------------------------------------------+
|  inline if 在 Ordinary 函数中的双阶段生命周期                  |
|                                                                         |
|  源码：                                                                 |
|  fn normalize_channel(x: f32) -> f32 where use_bias: bool {             |
|      inline if use_bias {                                               |
|          return x + 1.0f32;   -- 分支 A                                 |
|      } else {                                                           |
|          return x;            -- 分支 B                                 |
|      }                                                                  |
|  }                                                                      |
|                                                                         |
|  【阶段 1：Normalization（定义时）】                                    |
|    • 检查分支 A：`x + 1.0f32` 类型是 `f32` ✅                           |
|    • 检查分支 B：`x` 类型是 `f32` ✅                                    |
|    （哪怕某条分支在特定常量下不走，只要类型写错，定义时立刻报错  ）     |
|                                                                         |
|  【阶段 2：Instantiation（以 use_bias = false 实例化时）】              |
|    • 编译期判定 `use_bias == false`                                     |
|    • 分支 A 被彻底剪除，生成的特化函数体等价于：                        |
|      `fn normalize_channel_false(x: f32) -> f32 { return x; }`          |
+-------------------------------------------------------------------------+
```

让我们看看 `inline if`、多条件分支 `inline when` 以及模式匹配 `inline if ... is ... do` 的实战写法，以及编译器对未选中分支里类型错误的拦截：

```flurry
fn tensor_post_process(val: i32, comptime scale_opt: ?i32) -> i32 {
    var out = val;

    -- 1. inline when：多条件编译期分支选择
    inline when {
        true => {
            out = out + 1i32;
        },
    }

    -- 2. inline if ... is ... do：编译期模式匹配解构
    inline if scale_opt is factor? do {
        out = out * factor;
    }

    out
}

-- ❌ 编译期直接拒绝：（哪怕条件恒为 true，else 分支永远不会在运行时执行。）
fn bad_inline_if_branch(x: i32) -> i32 {
    inline if true {
        x + 1i32
    } else {
        -- 在 Ordinary 模式下，未选中的 else 分支同样要过类型检查。
        -- 返回 bool 与函数签名要求的 -> i32 冲突，编译期直接报错。
        false
    }
}
```

---

## `inline for`——编译期循环摊平与异构元组 `tuple_enumerate` 机制

如果说 `inline if` 解决了分支裁剪，那么 **`inline for`** 就是张量算子与异构元编程的核心机制。

在 Flurry 中，`inline for` 支持两种核心迭代源：

### 武器 1：有界编译期整数区间展开（`0 .. n` 与 `0 ..= n`）

当你写下 `inline for i in 0 .. 3`（甚至带上编译期过滤守卫 `inline for i if i != 1 in 0 .. 3`）时：
- 循环变量 **`i` 是一个真正的编译期 `Integer` 值**。这意味着你可以在循环体里面直接把 `i` 传给 `comptime` 参数、用在 `inline if` 条件里，或者做编译期边界检查；
- 而循环体里的运行时语句，则会被编译器按迭代次数**一轮一轮顺序展开（Unroll）**。

```flurry
comptime fn is_active_tap(i: Integer) -> bool {
    i != 1
}

-- 将 1D 滤波器核循环在编译期彻底摊平。
fn unrolled_tap_accumulate(base: i32) -> i32 {
    var acc = 0i32;

    -- i 是编译期 Integer，带守卫 if is_active_tap(i)
    -- 编译期只会展开 i = 0 和 i = 2 两轮迭代。
    inline for i if is_active_tap(i) in 0 .. 3 {
        let tap_weight: i32 = i + 10;
        acc = acc + base * tap_weight;
    }

    acc
}
```

### 武器 2：用 `'tuple_enumerate` 遍历异构元组（Heterogeneous Tuple Iteration）

还记得我们在第 069 篇提出的那个难题吗？
假设我们的多分支神经网络头输出了一个异构元组 `pack: Tuple<A, B>`，其中第 0 个元素是类型 `A`，第 1 个元素是完全不同的类型 `B`。
在普通语言里，你根本不可能用 `for` 循环去遍历一个元素类型各不相同的元组。

但在 Flurry 中，你只需对元组调用后缀操作 **`'tuple_enumerate`**，然后交给 **`inline for`**。

```
+-------------------------------------------------------------------------+
|  inline for 遍历异构元组 values: Tuple<A, B>                   |
|                                                                         |
|  源码：                                                                 |
|  inline for (index, element) in values'tuple_enumerate {                |
|      consume(index, element);                                           |
|  }                                                                      |
|                                                                         |
|  编译期展开后的等价结构（每轮迭代拥有独立的块作用域与资源清理边界  ）： |
|  +-------------------------------------------------------------------+  |
|  | -- 第 0 轮展开：                                                  |  |
|  | {                                                                 |  |
|  |     comptime index: Integer = 0;                                  |  |
|  |     let element: A = values.0;   -- 第 0 轮 element 类型是 A      |  |
|  |     consume(0, element);                                          |  |
|  | } -- ◄── 第 0 轮局部资源在此精确清理                              |  |
|  |                                                                   |  |
|  | -- 第 1 轮展开：                                                  |  |
|  | {                                                                 |  |
|  |     comptime index: Integer = 1;                                  |  |
|  |     let element: B = values.1;   -- 第 1 轮 element 类型是 B      |  |
|  |     consume(1, element);                                          |  |
|  | } -- ◄── 第 1 轮局部资源在此精确清理                              |  |
|  +-------------------------------------------------------------------+  |
+-------------------------------------------------------------------------+
```

请注意这里最精妙的宇宙分工：
- **`index` 活在 $U_1$（编译期 `Integer`）**：因为每一轮是编译期展开的，`index` 在第 0 轮是编译期常数 `0`，在第 1 轮是编译期常数 `1`；
- **`element` 活在 $U_0$（运行时值）**：在第 0 轮，它的静态类型是 `A`；在第 1 轮，它的静态类型自动变成 `B`。而且它完全不是编译期拷贝，而是真正的运行时元组字段值，**严格遵守 Flurry 的仿射所有权（Affine Ownership / Move 语义）与每轮块末尾的资源析构边界**。

让我们写一段直接消费异构元组 `Tuple<A, B>` 的代码来验证这一特性：

```flurry
fn consume_any(comptime idx: Integer, item: T) where T {
    -- 按元素各自的具体类型 T 安全消费 item（如果 T 实现了 Drop，会在作用域末自动清理）
}

-- 遍历任意异构二元组 Tuple<A, B>，无需 A 和 B 类型相同。
fn consume_heterogeneous_pair(values: Tuple<A, B>)
where
    A,
    B
{
    -- 第 0 轮：index = 0 (comptime Integer), element : A (runtime value)
    -- 第 1 轮：index = 1 (comptime Integer), element : B (runtime value)
    inline for (index, element) in values'tuple_enumerate {
        consume_any(index, element);
    }
}
```

> **🔍 语法小贴士：`inline for` 必须在编译期有界。**
> 因为 `inline for` 是在编译期将循环体物理展开，它的迭代范围（无论是 `start .. end` 还是元组的长度）必须是编译期已知的有界值。如果你试图把一个运行时才能确定长度的动态变量 `n: usize` 塞给 `inline for i in 0 .. n`，编译器会编译期拒绝。

```flurry
fn bad_dynamic_inline_for(runtime_len: usize) {
    -- ❌ 编译期直接拒绝。
    -- `inline for` 的区间上下界必须是编译期可确定的 Integer 值，
    -- 不能使用运行时动态变量 `runtime_len` 进行编译期展开。
    inline for i in 0 .. runtime_len {
        let _ = i;
    }
}
```

---

## 本篇小结（兼第四卷总结）

| 内联构造 | 语法示例 | 编译期（$U_1$）行为 | 运行时（$U_0$）与类型检查行为 |
| :--- | :--- | :--- | :--- |
| **`comptime` 块** | `comptime { 12 * 12 }` | 在编译期（`loose` 强度）直接算出结果 | 结果内联或经 `Splice` 落地为运行时常量 |
| **`inline if` / `when`** | `inline if flag { ... } else { ... }` | 根据编译期条件裁剪未命中的分支 | `ordinary` 模式下**所有分支均做类型与契约检查**；仅选中分支生成运行时代码 |
| **`inline for`（区间）** | `inline for i if p(i) in 0 .. 4 { ... }` | `i` 为编译期 `Integer`，按守卫展开每一轮 | 每一轮生成独立的运行时块作用域，严格维护局部资源清理顺序 |
| **`inline for`（元组）** | `inline for (idx, el) in tup'tuple_enumerate` | `idx` 为编译期 `Integer`（`0, 1, ...`） | `el` 在每一轮绑定为对应元组字段的真实运行时类型（`A`, `B`, ...），保持完整 Move 与所有权语义 |

---

### 第四卷结语与下一卷预告

至此，我们已经完整走完了**《第四卷：元编程即类型论——走进两级类型论 2LTT 与编译期宇宙》**的全部 10 篇旅程：
- 我们明白了为什么 `Type` 只活在 $U_1$ 编译期宇宙，为什么无需发明第二套宏语言就能直接用 `comptime fn` 计算类型；
- 我们掌握了 `pure comptime fn Name(...) -> Type` 的 **Normal-Form Head** 身份核心规则，看懂了连 `Tuple<...Es: meta.List<Type>>` 都是一个变参 Normal-Form 类型函数；
- 我们学会了优雅的 `where` 子句与三阶段参数依赖，深入了 **Normalization（定义端符号检查）** 与 **Instantiation（调用端具体实例化）** 的双阶段引擎，掌握了 `ordinary`/`loose` 与 `defer`；
- 最后，我们跨越了 **Lift、Quote、Splice** 三座宇宙大桥，认识了编译期无限精度 `Integer`/`Float` 与运行时 Proxy，并用 `comptime { ... }`、`inline if`、`inline for` 将编译期智慧无缝织入了运行时代码。

但是，在刚刚编写张量算子时，你一定注意到了我们在 `where` 子句里频频写下的 `T :- Copy`、`T :- Add<T, Output = T>`。
如果我们要把静态形状张量库真正变成一个工业级的通用计算引擎：
- 怎么定义我们自己的 **`trait`**（比如张量元素协议 `Scalar`、序列化协议 `Encode`、或者带关联类型与 GAT 的算子特征）？
- 为什么 Flurry 的孤儿规则（Coherence）和实现消歧比传统语言更灵活又更严密？
- 当我们需要运行时动态多态时，Flurry 的 `dyn Trait` 又是如何与存在类型（Existential Types）结合的？

准备好进入下一卷——**《第五卷：协议、多态与契约——Trait 系统的深度进化》**吧。下一篇 **`076-trait-basics-and-where-constraints.md`**，我们不见不散。
