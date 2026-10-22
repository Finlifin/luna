# Flurry 演进手记 (067)：普通函数怎么变成类型构造器——`comptime fn` 与返回 `Type` 的函数

在 [上一篇 (066)](./066-two-universes-u1-and-u0-intro.md) 中，我们推开了两级类型论（2LTT）的大门：在编译期宇宙 $U_1$ 里，类型（`Type`）已非只能写在冒号后面的静态死文本，而是一个可以像整数、布尔值一样自由传递、存储和计算的一等公民。

现在，我们要为边缘神经网络的静态张量库编写第一个实战组件——**自适应精度存储选择器**。
在传感器边缘计算中，不同层的权重张量需要不同的量化位宽：
- 当配置标志为布尔模式（二值网络）时，底层元素存储用 `bool`；
- 当配置标志为 8 位定点量化时，底层元素存储用 `u8`（`Byte`）；
- 当我们需要把任意元素类型 `T` 包装成“双缓冲采样对（Pair）”或“三通道向量（Vec3）”时，希望直接调用一个编译期工厂函数来生成对应的结构体类型。

在 C++ 里，这需要写一堆冗长的 `template<bool Flag> struct SelectStorage; template<> struct SelectStorage<true> { using type = bool; };`（模板特化体操）。
而在 Flurry 中，你只需要写一个再普难以通过的 **`comptime fn`**。

不过，在敲下函数名之前，我们先来明确一条 Flurry 的标志性命名规范。

---

## 一、 命名核心规则与第一个返回 `Type` 的函数

翻开 `docs/grammar/index.md` 与 `docs/comptime/purity-and-normal-form.md`，你会看到一条非常醒目的代码风格与语义提示规范：

> **🔍 语法与命名核心规则：返回 `Type` 的函数使用 `PascalCase`（大驼峰）。**
> 在 Flurry 中：
> - 返回普通数值或数据的函数（哪怕是 `comptime fn square(v: Integer) -> Integer`）一律使用 **`snake_case`**（如 `square`、`ready_code`）；
> - 而**返回结果是 `Type`（即类型构造器 / 类型工厂）的函数，使用 `PascalCase`（大驼峰）命名**（例如 `Identity`、`SelectStorage`、`PairOf`、`Tuple`）。
>
> 为什么要这样规定？因为当你在类型标注位置写 `let x: SelectStorage(true) = ...;` 时，大驼峰的 `SelectStorage` 一眼就能让读者看出“这里调用的是一个产生类型的类型函数”。

让我们立刻用 `PascalCase` 写出我们的第一版类型选择器 `Selected` 和恒等类型函数 `Identity`（详见 `docs/functions/comptime_fn.md`）：

```flurry
use std.prelude.*;

-- 1. 最简单的返回 Type 的编译期函数：圆括号调用直接执行函数体，返回 T。
pure comptime fn Identity(T: Type) -> Type {
    T
}

-- 2. 用普通 if-else 写的类型选择器：从此无需 C++ 模板偏特化了。
comptime fn Selected(flag: bool) -> Type {
    if flag {
        bool
    } else {
        u8
    }
}

-- 在编译期直接调用，赋给 typealias。
typealias Number = Identity(u32);
typealias FlagStorage = Selected(true);
typealias ByteStorage = Selected(false);

test {
    let number: Number = 9u32;
    let flag: FlagStorage = true;
    let byte: ByteStorage = 255u8;

    println("{} {} {}", number, flag, byte);
}
```

运行这段代码，输出：

```text
9 true 255
```

### 普通 `comptime fn` 的类型展开 结构示意

注意看上面的 `Identity(u32)` 和 `Selected(false)`：它们使用的是**圆括号 `( ... )` 普通函数调用语法**。
在 Flurry 中，普通 `comptime fn` 的圆括号调用具有纯粹的**执行语义（Execution Semantics）**——当输入具备时，编译器直接运行函数体里的代码，拿到返回的那个 `Type` 值，然后把调用处直接替换（展开）成那个结果类型。

```text
  编译期调用 (圆括号):  typealias ByteStorage = Selected(false);
                                                    │
                                                    ▼ (进入 U₁ 字节码解释器执行)
                        ┌───────────────────────────────────────────┐
                        │ 执行 `if false { bool } else { u8 }`      │
                        │ ──► 走 else 分支，产出 Type 值：`u8`       │
                        └─────────────────────┬─────────────────────┘
                                              │
                                              ▼ (完全展开，不留函数壳子)
  最终得到的类型身份:   ByteStorage === u8 (与直接写 u8 100% 是同一个类型。)
```

甚至，正如我们在 `docs/type/type-identity.md` 中看到的：
如果你写了一个纯类型工厂函数 `pure comptime fn PairOf(T: Type) -> Type`，只要标记了 **`pure`**（向编译器承诺相同输入必定产生相同输出），那么在同一个编译单元里，两次传入相同参数调用 `PairOf(u32)`，编译器会复用完全相同的缓存结果。

```flurry
pure comptime fn PairOf(T: Type) -> Type {
    struct Pair {
        first: T,
        second: T,
    }
    Pair
}

typealias FirstPair  = PairOf(u32);
typealias SecondPair = PairOf(u32);
typealias FloatPair  = PairOf(f32);

comptime {
    -- ✅ 相同的 pure comptime 调用复用同一个生成类型实例。
    assumes FirstPair == SecondPair;
    -- ✅ 不同输入参数生成不同的类型实例。
    assumes FirstPair != FloatPair;
}
```

---

## 二、 `comptime` 函数内部的真实编程能力——局部状态、指针与多步计算

如果类型级计算只能写成受限的类型族或模板递归表达式，就禁止直接使用常规的局部变量 `var`、循环与辅助函数。

但在 Flurry 的 $U_1$ 编译期宇宙里，一个 `comptime fn` 就是一段完整的完整程序。
你非但可以在里面写 `let`、`var`、`while` 循环、模式匹配，甚至可以在 `comptime fn` 内部使用**临时的编译期局部指针**（详见 `docs/comptime/index.md`）：

```flurry
use std.prelude.*;

-- 在编译期修改指针指向的 bool 值
comptime fn set_flag(target: *bool, value: bool) {
    target.* = value;
}

-- 在编译期通过局部指针与状态计算决定最终返回的 Type。
comptime fn ComputeTensorStorage(use_high_precision: bool) -> Type {
    var need_64bit = false;
    if use_high_precision {
        set_flag(need_64bit.ref, true);
    }

    if need_64bit {
        f64
    } else {
        f32
    }
}

typealias HighPrecElem = ComputeTensorStorage(true);
typealias LowPrecElem  = ComputeTensorStorage(false);

comptime {
    assumes HighPrecElem == f64;
    assumes LowPrecElem == f32;
}
```

看看 `ComputeTensorStorage`：
- 我们在里面声明了可变绑定 `var need_64bit = false;`，取了它的编译期局部指针 `need_64bit.ref` 传给另一个 `comptime fn set_flag`；
- 编译期解释器使用安全的内部存储身份（隐匿宿主机的真实内存地址）完成了指针写入；
- 函数结束后，干干净净地返回了类型值 `f64` 或 `f32`。

当然，编译期解释器的安全边界同样滴水不漏：如果一个编译期局部变量的生命周期已经结束，或者有人试图把指向编译期临时局部存储的指针直接物化（Splice）塞进最终的运行时常量里，编译器会直接报错拦截。

```flurry
-- ❌ 编译期直接报错。
-- 编译期临时存储的指针只能在 comptime 内部计算期间使用，
-- 禁止逃逸到物化常量或 runtime 程序中。
const BAD_ESCAPED_PTR: *bool = comptime {
    let temp = true;
    temp.ref
}
```

---

## 三、 当参数类型依赖前一个参数——开放签名与 `defer Selected(flag)`

现在，我们的张量库遇到了一个更酷的需求：
我们要写一个通用的编译期配置校验函数 `identity(flag: bool, value: ???)`：
- 当第 1 个参数 `flag` 传入 `true` 时，第 2 个参数 `value` 和返回值都必须是 `bool`（即 `Selected(true)`）；
- 当第 1 个参数 `flag` 传入 `false` 时，第 2 个参数 `value` 和返回值都必须是 `u8`（即 `Selected(false)`）。

注意：在**定义** `identity` 这个函数的那一刻，形参 `flag` 的具体值还无从得知（它要等调用 `identity(true, true)` 或 `identity(false, 7)` 时才知道）。
既然定义时还无从得知 `flag` 是 `true` 还是 `false`，编译器怎么在定义 `identity` 时计算 `Selected(flag)` 这个类型呢？

这就是 Flurry 的延迟求值关键字 **`defer`** 大显身手的地方（详见 `docs/functions/comptime_fn.md`，我们会在第 072 篇深入剖析它的全部机制）：

```flurry
use std.prelude.*;

typealias Byte = u8;

comptime fn Selected(flag: bool) -> Type {
    if flag { bool } else { Byte }
}

-- 用 `defer Selected(flag)` 告诉编译器：
-- 在定义检查时先确认 Selected(flag) 返回的是一个 Type；
-- 等调用点传入具体的 flag 后，再就地算出 value 的真实类型。
comptime fn identity(flag: bool, value: defer Selected(flag))
    -> defer Selected(flag) {
    value
}

const FLAG   = identity(true, true);
const NUMBER = identity(false, 7);

-- 用 `'type` 视图取得编译期常量结果的类型。
typealias FlagType   = FLAG'type;
typealias NumberType = NUMBER'type;

fn get_flag() -> FlagType { true }
fn get_number() -> NumberType { 7 }

test {
    println("{} {}", get_flag(), get_number());
}
```

运行这段代码，输出 `true 7`。
而如果你在调用 `identity` 时传错了第 2 个参数的类型（比如 `flag` 传了 `false` 要求 `Byte`，你却在 `value` 位置传了个 `true`）：

```flurry
-- ❌ 编译期直接报错。
-- 当 flag = false 时，defer Selected(false) 求值为 Byte (u8)，
-- 第二个实参 `true` (bool) 与期望类型 u8 失配。
const BAD_CALL = identity(false, true);
```

---

## 本篇小结

| 特性 / 规则 | 语法形式 | 核心语义与工程价值 |
| :--- | :--- | :--- |
| **返回 `Type` 的命名规范** | `comptime fn SelectStorage(...) -> Type` | 返回 `Type` 的函数使用 **`PascalCase`**，普通值函数使用 `snake_case` |
| **圆括号类型函数调用** | `typealias T = Selected(true);` | 普通 `comptime fn` 调用具有**执行语义**，直接运行函数体并将调用处展开为结果类型 |
| **`pure` 缓存复用** | `pure comptime fn PairOf(T: Type) -> Type` | `pure` 承诺相同输入决定相同结果；相同实参调用复用同一个生成的名义类型实例 |
| **开放依赖签名** | `value: defer Selected(flag)` | 允许后序参数和返回值的类型直接通过 `comptime fn` 依赖前序参数的值 |

---

### 下一篇预告

在这篇手记里，我们写了一个用**圆括号**定义和调用的恒等类型函数：

```flurry
pure comptime fn Identity(T: Type) -> Type { T }
typealias Number = Identity(u32);
```

因为圆括号调用会直接执行函数体，所以 `Identity(u32)` 当场就化简展开成了 `u32`——在编译器眼里，`Number` 和 `u32` 完全是同一个东西。

但是，请翻开 `docs/comptime/purity-and-normal-form.md` 的核心规则：
> **“Normal form 的头维持未展开态；`Identity<T>` 不因函数体返回 `T` 就与 `T` 相等。”**

等等。
- 为什么把圆括号 `(T: Type)` 换成**尖括号 `<T: Type>`**，写成 **`pure comptime fn Identity<T: Type> -> Type { T }`** 之后，`Identity<u32>` 却就**有别于** `u32` 了？
- 既然函数体里明明写着 `{ T }`，为什么编译器禁止展开它？
- 在设计我们的静态张量类型 `Tensor<T, ...>`、元组 `Tuple<...>` 或者泛型容器时，这个“打死都维持未展开态函数体”的 **Normal-Form 头（范式头）** 究竟扮演着什么不可替代的灵魂角色？

在下一篇 **《Flurry 演进手记 (068)：为什么 `Identity<T>` 有别于 `T`——`pure comptime fn Name<...>` 与 Normal-Form 头》** 中，我们将直击 Flurry 类型系统的核心脊梁。
