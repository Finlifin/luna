# Flurry 演进手记 (066)：为什么泛型无需第二套语法——编译期宇宙 ($U_1$) 与运行时宇宙 ($U_0$) 初探

欢迎来到 **《Flurry 演进手记》第四卷：元编程即类型论**。

在前三卷的旅程中，我们的传感器监控平台已经具备了坚如磐石的底层比特解析、流式控制流与仿射资源安全。
今天，算法团队给我们提出了一个有代表性的新需求：我们要在边缘计算节点上直接运行一个轻量级的神经网络推理引擎，用来实时识别传感器采集到的振动波形。

为了避免运行时矩阵维度失配导致的崩溃（比如把一个 $128 \times 64$ 的矩阵错误地加到一个 $32$ 维向量上），我们需要打造一套**静态形状张量库（Static-Shaped Tensor）**：
1. 张量的元素类型和维度形状直接编码在类型里，比如 `Tensor<f64, 128, 128>` 代表二维矩阵，`Tensor<f64, 128>` 代表一维向量；
2. 当我们写下 `matrix + vec` 时，编译器要在**编译期**自动从右往左对齐维度、验证广播（Broadcasting）相容性，并计算出结果类型为 `Tensor<f64, 128, 128>`；
3. 当到达运行时，生成的机器代码只负责纯粹、高速的元素加法循环，**不在运行时存半个字节的类型描述符。**

如果你曾在 C++ 或 Rust 里尝试写过这样的库，你一定经历过那种被“前后存在分歧的语言子系统”支配的困境。让我们先看看传统语言为什么会在这里撞墙，再看 Flurry 是如何用 **两级类型论（Two-Level Type Theory, 2LTT）** 一举终结这场分裂的。

---

## 一、 传统语言中“三套语言打架”的难题

在 C++ 或 Rust 中，当你想要“在编译期操纵类型和常量”时，你很快会发现自己被迫同时写三门大相径庭的语言：

1. **值级普通代码（Value Level）**：你平时写的 `if`、`match`、`for`、普通函数调用。
2. **类型级模板/泛型（Type Level）**：一旦进入 `<T, U>` 的世界，普通的 `if` 禁用了，你得改写偏特化（Partial Specialization）、SFINAE 或复杂的递归 Trait 关联类型（Type-level Peano 算术）来模拟分支和循环。
3. **语法宏 / `const` 泛型（Macro / Const Generics）**：想根据一个整数序列推导广播维度？Rust 的 `const` 泛型连变长形状列表都不好传；想生成代码？又得跳进词法 Token 流的过程宏（Proc-Macro）里，那里完全无从获取类型系统的语义信息。

```text
  传统语言的割裂现状（三套语法、三套规则）：
  ┌────────────────────┐   ┌────────────────────────┐   ┌────────────────────┐
  │ 1. 普通运行时函数   │   │ 2. 泛型 / Trait 特化    │   │ 3. 预处理器 / 过程宏│
  │   if / for / fn    │   │  <T: Trait<Assoc = U>> │   │   TokenStream 拼接 │
  └─────────┬──────────┘   └───────────┬────────────┘   └─────────┬──────────┘
            └──────────────────────────┼──────────────────────────┘
                                       ▼
                     ❌ 语法不通、类型信息断层、报错如天书。
```

为什么写一个“编译期算类型”的程序，禁止直接用我们已经烂熟于心的普通函数、`if`、`match` 和局部变量呢？

Flurry 给出的答案振聋发聩（详见 `docs/introduction.md` 与 `docs/type/2-level-type.md`）：
> **元编程即类型论。泛型代码与编译期程序受同一套语法和类型规则约束，唯一的区别只是它们运行在哪个“计算阶段（宇宙）”。**

---

## 二、 两级类型论（2LTT）——编译期宇宙 $U_1$ 与运行时宇宙 $U_0$

在 Flurry 中，整个程序的世界被清晰地划分为两个分层宇宙（Universes）：

- **$U_1$（编译期宇宙，Comptime Universe）**：
  这里住着一等公民的**类型值（`Type`）**、编译期任意精度整数（`Integer`）、编译期函数（`comptime fn`）、范式头（Normal Form）、编译期常量（`const`）以及属性（`^expr`）。所有 $U_1$ 的计算都由编译器在编译期执行，用来决定程序的类型与代码结构。
- **$U_0$（运行时宇宙，Runtime Universe）**：
  这里住着由 $U_1$ 决定的那些具体类型所约束的运行时值（比如运行时的 `u32` 传感器读数、`f64` 张量元素、内存指针 `*T`）。所有 $U_0$ 的计算最终都会变成目标机器上的机器码执行。

如下所示，俯瞰 $U_1$ 与 $U_0$ 的双层宇宙全景：

```text
  ╔══════════════════════════════════════════════════════════════════════════╗
  ║  🌌 编译期宇宙 U₁ (Comptime Universe —— 由编译器执行，决定类型与代码结构) ║
  ║                                                                          ║
  ║  • 类型是一等值：const Word: Type = u32;                                  ║
  ║  • 编译期函数：  pure comptime fn PairOf(T: Type) -> Type { ... }        ║
  ║  • 精确数值计算：pure comptime fn square(v: Integer) -> Integer { v * v } ║
  ╚══════════════════╤════════════════════════════════════╤══════════════════╝
                     │                                    │
        (1. 类型决定：U₁ 的 Type 值           (2. 跨阶段传递：Lift / Quote / Splice
            直接作为 U₀ 绑定的类型约束。)         在两个宇宙间安全运送常量与代码描述。)
                     ▼                                    ▼
  ╔══════════════════════════════════════════════════════════════════════════╗
  ║  🌍 运行时宇宙 U₀ (Runtime Universe —— 由生成的机器码执行，处理真实数据)   ║
  ║                                                                          ║
  ║  • 运行时绑定：  let samples: PairOf(u32) = ...                         ║
  ║  • 零描述符开销：内存中只存两个 u32 (8 字节)，不存 Type 描述对象。      ║
  ╚══════════════════════════════════════════════════════════════════════════╝
```

来看看在 Flurry 中，如何直接写一个普通的编译期函数 `PairOf` 来生成结构体类型，并在运行时使用它（详见 `docs/type/2-level-type.md`）：

```flurry
use std.prelude.*;

-- 在 U₁（编译期宇宙）中：定义一个接收 Type、返回 Type 的函数。
-- 语法与普通函数一模一样：内部甚至可以直接定义并返回一个 struct。
pure comptime fn PairOf(T: Type) -> Type {
    struct Pair {
        first: T,
        second: T,
    }

    Pair
}

-- 在编译期调用 PairOf(u32)，得到生成的新类型并起个别名 Samples
typealias Samples = PairOf(u32);

-- 在 U₀（运行时宇宙）中：使用 Samples 类型构造和读取真实数据。
fn show_samples(first: u32, second: u32) {
    let samples = Samples { first: first, second: second }
    println("samples: {}, {}", samples.first, samples.second);
}

test {
    show_samples(12, 18);
}
```

运行这段代码，输出：

```text
samples: 12, 18
```

你甚至可以在函数内部直接用局部 `const` 保存类型值、解构编译期计算出的布尔元组，或者把引用代码 `'(...)` 拼接进运行时表达式（详见 `docs/comptime/index.md`）：

```flurry
pure comptime fn invert(value: bool) -> bool {
    const result = not value;
    result
}

fn configured() -> bool {
    -- 在 runtime 函数内部声明局部 const Type。
    const Flag = bool;
    -- 编译期计算元组并未可反驳解构，不留下任何 runtime 初始化开销。
    const (enabled, disabled): Tuple<Flag, Flag> = (invert(false), invert(true));
    const code = '(enabled and not disabled);
    ~<code>
}
```

---

## 三、 严守两个宇宙的物理边界——为什么 `Type` 无法塞进运行时结构体？

虽然 $U_1$ 和 $U_0$ 共享同一套优美的表达式和函数语法，但 **2LTT 的精髓恰恰在于“严格分层，零泄漏”**。

请牢记 `docs/type/2-level-type.md` 中的两条核心规则：

1. **编译器没有“读取未来内存”的超能力**：
   $U_1$ 的编译期计算可以描述 $U_0$ 的运行时程序（或保留对某个运行时对象身份的依赖），但**禁止于编译期读取一个运行时变量未来的实际数值**。
2. **`Type` 是纯编译期宇宙（$U_1$）的居民，没有运行时表示（No Runtime Proxy）**：
   在动态语言（如 Python）或带反射运行时的语言（如 Java/C#）里，类型往往对应着堆内存里的一个 `Class` 或 `TypeInfo` 对象。
   但在系统级语言 Flurry 中，**`Type` 值只存在于编译期宇宙 $U_1$**。生成的运行时机器码里只会有已经被类型约束好的 `u32`、`Pair` 等具体数据，免于在运行时内存里塞一个神秘的“类型描述符指针”。

因此，如果有人糊涂地把 `Type` 当成普通运行时结构体 `struct` 的字段类型，并试图在运行时函数里构造它，会发生什么？

```flurry
struct RuntimeTypeBox {
    stored_type: Type,
}

-- ❌ 编译期直接报错。
-- `Type` 属于 comptime-only 宇宙 (U₁)，没有通用的运行时表示（proxy），
-- 禁止充当运行时结构体 RuntimeTypeBox 的字段在 runtime 函数中交付。
fn make_type_box() -> RuntimeTypeBox {
    RuntimeTypeBox { stored_type: u32 }
}
```

让我们画一张对比图，看清“用 `Type` 控制运行时结构”与“把 `Type` 当运行时数据存储”的天壤之别：

```text
  ✅ 合法的 2LTT 用法（U₁ 控制 U₀ 的类型）：
     const Word: Type = u32;              (在 U₁ 中保存类型值 u32)
     typealias CounterPair = PairOf(Word); (在 U₁ 中算出含两个 u32 字段的结构体类型)
     let c = CounterPair { first: 3, second: 9 } (在 U₀ 中只存储 3 和 9 两个 u32。)

  ❌ 并非法的宇宙越界（试图把 U₁ 的 Type 塞进 U₀ 的内存盒子）：
     let box = RuntimeTypeBox { stored_type: u32 }
     ──► 编译器编译期报错。U₀ 的运行时内存里装不下 U₁ 的 `Type` 本身。
```

同时，也要区分两个经常被初学者混为一谈的概念（详见 `docs/comptime/index.md`）：
- **一个表达式暂时无从算出具体值 $\neq$ 它是 runtime 表达式**：在泛型函数定义里，`sizeof(T)` 暂时无从算出具体数字，只是因为编译期输入 `T` 还没传进来，它依然是 100% 的 `comptime` 表达式。
- **给普通函数传常量实参 $\neq$ 它自动变成了 `comptime fn`**：如果 `fn add_runtime(a: u32, b: u32) -> u32` 是一个普通的运行时函数，哪怕你写 `add_runtime(1, 2)`，它依然是运行时函数，禁止于需要编译期常量的地方直接调用它。

---

## 本篇小结

作为第四卷的开篇，我们建立了 Flurry 两级类型论（2LTT）的顶层宇宙观：

| 概念 / 宇宙 | 所在层级与核心成员 | 关键边界与核心规则 |
| :--- | :--- | :--- |
| **编译期宇宙 ($U_1$)** | `Type`、`Integer`、`Float`、`comptime fn`、`const`、属性 | 由编译器执行；使用与普通代码完全一致的函数、变量与控制流语法 |
| **运行时宇宙 ($U_0$)** | `u32`、`f64`、具体 `struct`/`enum` 实例、机器指令 | 由生成的目标程序执行；不承担重新猜测编译期类型的工作 |
| **跨层核心规则 1** | $U_1 \rightarrow U_0$ 类型约束与常量物化 | `Type` 仅存在于 $U_1$，禁止充当 $U_0$ 运行时结构体的字段在运行时传递 |
| **跨层核心规则 2** | 阶段（Stage）由声明与上下文决定 | 编译期计算无法偷读未来运行时内存；普通 runtime 函数不会仅因传入常量就自动变成 `comptime fn` |

---

### 下一篇预告

既然在编译期宇宙 $U_1$ 中，`Type` 就是一个普通的一等值，我们可以写函数来接收 `Type`、也可以写函数来返回 `Type`。
那么，我们要为静态形状张量库写第一个核心组件了：**根据张量量化精度（比如 8 位、16 位、32 位）或标志位，在编译期动态选择底层存储类型。**

但是，当你准备动笔写一个返回 `Type` 的函数时，请先停一下：
1. 还记得 `docs/grammar/index.md` 里的命名核心规则吗？普通函数用 `snake_case`，但**返回 `Type` 的函数必须用什么命名风格**？
2. 当一个 `comptime fn SelectStorage(bits: usize) -> Type` 在参数类型或返回类型位置被调用时（比如 `value: defer SelectStorage(bits)`），编译器是如何一步步把函数调用展开成真正的底层类型的？
3. 为什么在 `comptime fn` 内部，我们甚至可以声明局部指针 `let p = cell.ref;` 并修改它指向的值？

在下一篇 **《Flurry 演进手记 (067)：普通函数怎么变成类型构造器——`comptime fn` 与返回 `Type` 的函数》** 中，我们将亲手编写类型工厂函数，体验“用写普通代码的姿势写类型级计算”的高度快感。
