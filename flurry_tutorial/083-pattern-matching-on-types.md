# Flurry 演进手记 (083)：在类型自身上做模式匹配——`expr<type_pattern_argument*>` 类型模式详解

> **导读**：在上一篇（082）中，我们介绍了运行时依赖类型的相等性规则、复合环境转发与跨函数传递。本篇我们将回到元编程与泛型开发者的视角，探讨如何**直接在 `Type` 值上进行结构化模式匹配（Type Pattern Matching）**。

在开发静态形状张量库 `Tensor<T, .dims = s>` 或通用容器时，我们经常会拿到一个类型参数 `T: Type`，并需要检查它的结构：
- `T` 是否是一个 `Array`？如果是，它的元素类型是什么，长度又是多少？
- `T` 是否是一个元素类型等于 `u32`（或等于调用者传入的 `Expected` 类型）的 `Array`？

在 Flurry 中，由于 `Type` 是编译期的一等值，我们可以直接对类型值使用 **`T match { ... }`** 或 **`T is ...`** 进行结构化匹配。

---

## 一、 问题背景：模式里的标识符是“新建绑定”还是“引用已有类型”？

假设我们要编写一个编译期函数 `word_array_length(T: Type)`：当传入的类型 `T` 是 `Array<u32, n>` 时，返回 `n * 100`；当传入的是其他元素类型的 `Array<Element, n>` 时，返回 `n`。

如果直接沿用普通值模式的写法而未作区分地写 `Array<u32, length>`，就会产生语法歧义：

```
  [ 模式 Array<u32, length> 中的名字解析问题 ]
  • 第二个位置 `length` 的意图是：引入一个新的模式变量，捕获数组长度；
  • 第一个位置 `u32` 的意图是：与外层已有的 `u32` 类型做相等比对；
  • 如果写成 Array<Expected, length>，编译器如何区分 `Expected` 是要捕获
    任意元素类型的新变量，还是要与外层参数 `Expected: Type` 做相等比对？
```

如果把模式中的裸标识符默认当作新变量，就会把外层的同名类型参数 `Expected` 意外遮蔽（Shadow），使原本期望做“相等匹配”的分支变成“无条件匹配任意元素类型”。

为了消除这一二义性，Flurry 为类型模式规定了明确的区分规则：**无前缀的标识符 `id` 引入新模式绑定；带单引号前缀的 `'id` 引用当前作用域已有的类型或值表达式。**

---

## 二、 核心语法：`expr<type_pattern_argument*>` 与 `'id` 引用前缀

在 Flurry 的类型模式语法规范（`docs/type/type-patterns.md`）中，针对 Normal-Form 头构造器的模式匹配具有如下形式：

$$\text{normal\_form\_head\_application\_pattern} \rightarrow expr\texttt{<}\text{type\_pattern\_argument}^*\texttt{>}$$

$$\text{type\_pattern\_expression} \rightarrow \text{\texttt{'}}id$$

类型模式参数 `<...>` 的常用写法如下：

```
+-------------------------------------------------------------------------+
|                 Flurry 类型模式参数 `<...>` 的写法速查                    |
+-------------------------------------------------------------------------+
|                                                                         |
|  1. 裸标识符 `id` (如 `Element`, `length`)：                             |
|     ──► 引入新模式绑定，捕获该槽位的实际类型或编译期值。                  |
|                                                                         |
|  2. 单引号前缀 `'id` (如 `'u32`, `'Expected`, `'Bool`)：                 |
|     ──► 引用外层已有名字，要求该槽位的实际参数与外层 `id` 相等。          |
|                                                                         |
|  3. 字面量常量 (如 `4`, `true`)：                                        |
|     ──► 直接匹配编译期常量值。                                           |
|                                                                         |
|  4. 组合模式 (`'Bool as Kind` 或 `Kind and Kind is 'T`)：                |
|     ──► 既检查类型相等约束，又把结果绑定给新名字 `Kind`。                 |
+-------------------------------------------------------------------------+
```

---

## 三、 基本用法：匹配 `Array<'u32, length>` 与动态传入的 `'Expected`

借助 `'id` 语法，我们可以清晰地编写数组类型拆解函数：

```flurry
-- 1. 区分 Array<'u32, length>（元素固定为 u32）与通用 Array<Element, length>
pure comptime fn word_array_length(T: Type) -> usize {
    T match {
        -- 'u32 引用内置类型 u32；length 是新引入的模式变量，捕获数组长度
        Array<'u32, length> => length * 100,
        -- Element 和 length 都是新引入的模式变量，匹配任意元素类型的 Array
        Array<Element, length> => length,
        _ => 0,
    }
}

-- 2. 'Expected 也可以直接引用函数参数传入的目标类型 Expected: Type
pure comptime fn array_length_of(T: Type, Expected: Type) -> usize {
    T match {
        Array<'Expected, length> => length,
        _ => 0,
    }
}

test {
    -- 命中第 1 分支 Array<'u32, length>：4 * 100 = 400
    assumes word_array_length(Array<u32, 4>) == 400;
    -- 命中第 2 分支 Array<Element, length>：返回 8
    assumes word_array_length(Array<u8, 8>) == 8;
    -- 并非数组类型命中兜底分支 _：返回 0
    assumes word_array_length(u32) == 0;

    -- 动态对比外层参数 Expected：
    assumes array_length_of(Array<u16, 6>, u16) == 6;
    assumes array_length_of(Array<u16, 6>, u32) == 0;
}
```

> **说明：为什么 `'Expected` 不会遮蔽外层参数？**
> 单引号 `'Expected` 指示编译器切勿创建名为 `Expected` 的新模式变量，而是查找外层作用域中参数 `Expected: Type` 的值（例如第二次调用时的 `u32`），并检验被匹配数组的第一个类型参数是否与之相等。因此 `array_length_of(Array<u16, 6>, u32)` 会跳过第一分支并返回 `0`。

---

## 四、 依赖参数域在类型模式内部的传播（`Box<T, value: T>`）

当被匹配的 Normal-Form 类型构造器带有依赖参数——即后一个参数的类型依赖于前一个参数绑定出的类型时，类型模式同样支持从左向右传播类型信息。

以如下构造器为例：

```flurry
pure comptime fn Box<T, value: T> -> Type {
    T
}
```

在 `Box<T, value: T>` 中，第 1 个参数是类型 `T`，第 2 个参数 `value` 是类型为 `T` 的编译期值（例如 `Box<bool, true>` 或 `Box<u32, 7>`）。当我们对 `Box<...>` 进行模式匹配时，左侧槽位绑定出的类型变量 `Kind` 会向右传播，成为右侧槽位模式变量 `contents` 的静态类型：

```
        Box < ('Bool as Kind),  contents >
              ───────┬───────   ────┬───
                     │              ▲
      1. 匹配外层 Bool 并绑定给 Kind  │
                     └──────────────┘
            2. Kind 的类型信息向右传播：
               编译器据此确定 `contents : Bool`
```

下面是三种常见的依赖类型模式拆解写法：

```flurry
pure comptime fn Box<T, value: T> -> Type {
    T
}

-- 写法 A：使用 `('Bool as Kind)` 同时匹配外层类型别名并绑定给 Kind，右侧 contents 自动获得 Bool 类型
pure comptime fn unbox_bool(B: Type) -> bool {
    typealias Bool = bool;
    B match {
        Box<('Bool as Kind), contents> => contents,
        _ => false,
    }
}

-- 写法 B：使用模式合取 `(Kind and Kind is 'T)` 约束 Kind 等于传入的 T，右侧 contents 自动获得类型 T
pure comptime fn unbox_as(T: Type, B: Type, fallback: T) -> T {
    B match {
        Box<(Kind and Kind is 'T), contents> => contents,
        _ => fallback,
    }
}

test {
    assumes unbox_bool(Box<bool, true>);
    assumes not unbox_bool(Box<u32, 7>);

    assumes unbox_as(u32, Box<u32, 7>, 0) == 7;
    assumes unbox_as(u32, Box<bool, true>, 0) == 0;

    -- 写法 C：在无约束的 Box<Kind, contents> 中将拆出的类型 Kind 和值 contents 重新组装
    let echoed = Box<bool, true> match {
        Box<Kind, contents> => Box<Kind, contents>,
        _ => void,
    }
    assumes echoed == Box<bool, true>;
}
```

---

## 五、 语义边界：类型模式只匹配保留的 Normal-Form 头

在使用类型模式匹配时，需要注意第 068 篇介绍过的语义边界：**类型模式 `<...>` 匹配的是保留下来的 Normal-Form 构造器头（如 `Array<...>`、`Tuple<...>`、`Box<...>`、`Identity<...>`）**。

1. **普通圆括号函数不保留构造器头**：如果定义了一个普通圆括号函数 `pure comptime fn StorageFor(T: Type) -> Type { u32 }`，调用 `StorageFor(bool)` 求值后直接得到 `u32`，不会保留 `StorageFor` 调用头，因此无法经由模式匹配从 `u32` 反推 `bool`。
2. **外层 Normal-Form 头免于自动展开**：如果把 `Box<bool, true>` 包装进 `Identity<Box<bool, true>>`，由于 `Identity<...>` 是独立的 Normal-Form 头，它免于自动剥离为 `Box<bool, true>`：

```flurry
pure comptime fn Identity<T: Type> -> Type {
    T
}

test {
    -- Identity<Box<bool, true>> 的最外层头是 Identity，并非 Box
    -- 因此 unbox_bool 会落入 _ 分支返回 false，除并非显式匹配 Identity<Box<...>>
    assumes not unbox_bool(Identity<Box<bool, true>>);
}
```

此外，如果在类型模式中第 2 槽位的模式与第 1 槽位确定的类型域存在分歧，编译期会直接报错：

```flurry
-- ❌ 编译期报错：当第 1 槽位已限定为 'bool 时，第 2 槽位属于 bool 域，无法使用整数模式 123
pure comptime fn invalid_dependent_pattern(B: Type) -> bool {
    B match {
        Box<'bool, 123> => true,
        _ => false,
    }
}
```

---

## 六、 本篇小结

| 模式写法 | 语法分类 | 语义说明 |
| :--- | :--- | :--- |
| `Head<...>` | Normal-Form 头模式 | 检验目标类型的最外层 Normal-Form 构造器头是否为 `Head`，并逐槽匹配参数 |
| `id`（无单引号） | 新绑定模式变量 | 在当前模式分支中引入新变量 `id`，绑定对应槽位的类型或编译期值 |
| `'id`（带单引号） | 外部类型/值引用模式 | 查找当前作用域已有的 `id`，要求对应槽位的实参必须与之相等 |
| `'Expected as Kind` | 别名绑定模式 | 既检验槽位等于 `Expected`，又将其绑定到模式变量 `Kind` 供右侧依赖槽位使用 |
| `Kind and Kind is 'T` | 合取守卫模式 | 引入绑定 `Kind` 并附加同级模式约束 `Kind is 'T` |

---

## 七、 下一篇预告

回顾前面几章的内容，我们已经接触了多种编译期与类型约束形式：
- `Type.size_of(T) == 8`（布局大小相等）；
- `Type.predicates.is_tuple(Itself)`（结构分类谓词）；
- `0 <= index and index + 1 < length`（编译期整数线性不等式）；
- `T match { Array<'u32, length> => ... }`（类型模式匹配）。

当函数签名中带有 `where requires ...` 子句时，编译器如何在定义阶段对尚未代入具体数值的符号参数进行静态推理？为什么“定义期未能证出某条约束恒成立”有别于“证明了该约束恒为假”？

下一篇 **《Flurry 演进手记 (084)：初识命题类型约束（PTC）——定义期符号推理与约束检验》**，我们将介绍 Flurry 的命题类型约束机制。
