# Flurry 演进手记 (083)：在类型自身上做模式匹配——expr<type_pattern_argument*> 类型模式详解

> **导读**：在上一回（082）中，我们深入编译器内部，看清了带洞类型核 `TypeCore` 与引用端替换 `Substitution` 是如何协同工作的。今天，我们将回到元编程开发者的视角，解锁张量维度推导中最顺手的一件神兵利器——**直接在 `Type` 值上进行结构化模式匹配（Type Pattern Matching）**！

在开发静态形状张量库 `Tensor<T, S>` 时，我们经常会拿到一个未知的类型参数 `T: Type`，并需要向它发问：
- “你是不是一个 `Array`？如果是，你的元素类型是什么？长度又是多少？”
- “你是不是一个元素类型**恰好等于 `u32`**（或者恰好等于调用者传入的 `Expected` 类型）的 `Array`？”

在 C++ 里，这需要写一堆令人头秃的模板偏特化（Partial Template Specialization `struct Traits<Array<T, N>>`）；在缺乏类型模式匹配的语言里，你只能写一长串繁琐的反射 API 调用。而在 Flurry 中，既然 `Type` 是编译期的一等公民值，我们当然可以直接对它写 **`T match { ... }`**！

---

## 一、 破局与痛点：模式里的名字，到底是“新建绑定”还是“引用已有类型”？

假设我们要写一个编译期函数 `word_array_length(T: Type)`：当传入的类型 `T` 是 `Array<u32, N>` 时，返回 `N * 100`；当传入的是其他元素的 `Array<Element, N>` 时，返回 `N`。

如果我们照搬普通语言的值模式语法，不加区分地直接写 `Array<u32, length>`（`v0` 语法歧义陷阱），会发生什么？

```
  [ 语法设计者的灵魂拷问：在模式 Array<u32, length> 中 ]
  • 为什么第二个位置 `length` 是“引入一个新的模式变量，用来捕获数组长度”？
  • 而第一个位置 `u32` 却是“要求跟外层已有的 u32 类型做相等比对”？
  • 如果我不小心写成了 Array<Expected, length>，编译器怎么知道 Expected
    是要捕获任意元素类型的新变量，还是要跟外层参数 `Expected: Type` 做相等比对？！
```

在很多语言里，如果你在 `match` 模式里写了一个标识符 `Expected`，它会自动变成一个**全新的通配绑定变量**，把外层的同名参数 `Expected` 悄悄遮蔽（Shadow）掉，导致原本想做“相等匹配”的逻辑变成了“无条件匹配一切”的致命静默 Bug！

为了从语法根源上消灭这种二义性，Flurry 为类型模式设计了一条极其干净的规则：**无前缀的 `id` 永远引入新绑定；带单引号前缀的 `'id` 专门引用当前作用域已有的类型或值表达式！**

---

## 二、 核心语法：`expr<type_pattern_argument*>` 与 `'id` 锚定符

在 Flurry 的类型模式语法规范（`docs/type/type-patterns.md`）中，针对 Normal-Form 头构造器的模式匹配具有如下形式：

$$\text{normal\_form\_head\_application\_pattern} \rightarrow expr\texttt{<}\text{type\_pattern\_argument}^*\texttt{>}$$

$$\text{type\_pattern\_expression} \rightarrow \text{\texttt{'}}id$$

让我们用一张对比卡片记住类型模式参数里的黄金法则：

```
+-------------------------------------------------------------------------+
|                 Flurry 类型模式参数 `<...>` 的写法速查                    |
+-------------------------------------------------------------------------+
|                                                                         |
|  1. 裸标识符 `id` (如 `Element`, `length`)：                             |
|     ──► 【引入新模式绑定】！捕获该槽位的实际类型或编译期值。               |
|                                                                         |
|  2. 单引号前缀 `'id` (如 `'u32`, `'Expected`, `'Bool`)：                 |
|     ──► 【引用外层已有名字】！要求该槽位的实际参数必须与外层 `id` 相等。   |
|                                                                         |
|  3. 字面量常量 (如 `4`, `true`)：                                        |
|     ──► 【直接匹配编译期常量值】！                                        |
|                                                                         |
|  4. 组合模式 (`'Bool as Kind` 或 `Kind and Kind is 'T`)：                |
|     ──► 【既做相等/守卫约束，又把结果绑定给新名字 `Kind`】！               |
+-------------------------------------------------------------------------+
```

---

## 三、 `v1` 升级：精准拆解 `Array<'u32, length>` 与动态传入的 `'Expected`

有了 `'id` 语法，我们刚才的张量缓冲区长度提取器立刻变得清晰无比、滴水不漏：

```flurry
-- 1. 区分 Array<'u32, length>（元素固定为 u32）与通用 Array<Element, length>
pure comptime fn word_array_length(T: Type) -> usize {
    return T match {
        -- 'u32 引用内置类型 u32；length 是新引入的模式变量，捕获数组长度！
        Array<'u32, length> => length * 100,
        -- Element 和 length 都是新引入的模式变量，匹配任意元素类型的 Array
        Array<Element, length> => length,
        _ => 0,
    };
}

-- 2. 'Expected 还能直接引用函数参数传入的目标类型 Expected: Type！
pure comptime fn array_length_of(T: Type, Expected: Type) -> usize {
    return T match {
        Array<'Expected, length> => length,
        _ => 0,
    };
}

test "type pattern matching on Array v1" {
    -- 命中第 1 分支 Array<'u32, length>：4 * 100 = 400
    assert(word_array_length(Array<u32, 4>) == 400);
    -- 命中第 2 分支 Array<Element, length>：返回 8
    assert(word_array_length(Array<u8, 8>) == 8);
    -- 非数组类型命中兜底分支 _：返回 0
    assert(word_array_length(u32) == 0);

    -- 动态对比外层参数 Expected：
    assert(array_length_of(Array<u16, 6>, u16) == 6);
    assert(array_length_of(Array<u16, 6>, u32) == 0);
}
```

> **🔍 语法小贴士：为什么 `'Expected` 不会误捕所有类型？**
> 因为单引号 `'Expected` 明确告诉语法分析器：不要创建名叫 `Expected` 的新模式变量，而是去外层作用域查找参数 `Expected: Type` 的值（比如第二次调用时的 `u32`），并检验被匹配数组的第一个类型参数是否等于 `u32`！因此 `array_length_of(Array<u16, 6>, u32)` 精准地跳过了第一分支并返回 `0`。

---

## 四、 `v2` 进阶：在类型模式内部传播依赖参数域（`Box<T, value: T>`）

如果被匹配的 Normal-Form 类型构造器本身是一个**依赖类型头**——也就是说，它后面第 2 个参数的类型，依赖于前面第 1 个模式绑定出来的类型，Flurry 的类型模式匹配还能应付吗？

来看看这个极具代表性的构造器：
```flurry
pure comptime fn Box<T, value: T> -> Type {
    T
}
```
在 `Box<T, value: T>` 中，第 1 个参数是类型 `T`，第 2 个参数 `value` 是属于类型 `T` 的编译期值（例如 `Box<bool, true>` 或 `Box<u32, 7>`）。

当我们对 `Box<...>` 做模式匹配时，**左侧槽位绑定出来的类型变量 `Kind`，会立即向右传播，成为右侧槽位模式变量 `contents` 的静态类型**！

```
        Box < ('Bool as Kind),  contents >
              ───────┬───────   ────┬───
                     │              ▲
      1. 匹配外层 Bool 并绑定给 Kind  │
                     └──────────────┘
            2. Kind 的类型信息立即向右传播：
               编译器当场获知 `contents : Bool`！
```

让我们在 `v2` 中体验三种不同写法的依赖类型模式拆解：

```flurry
pure comptime fn Box<T, value: T> -> Type {
    T
}

-- 写法 A：使用 `('Bool as Kind)` 同时匹配外层类型别名并绑定给 Kind，右侧 contents 自动获得 Bool 类型！
pure comptime fn unbox_bool(B: Type) -> bool {
    typealias Bool = bool;
    return B match {
        Box<('Bool as Kind), contents> => contents,
        _ => false,
    };
}

-- 写法 B：使用模式合取 `(Kind and Kind is 'T)` 约束 Kind 必须等于传入的 T，右侧 contents 自动获得类型 T！
pure comptime fn unbox_as(T: Type, B: Type, fallback: T) -> T {
    return B match {
        Box<(Kind and Kind is 'T), contents> => contents,
        _ => fallback,
    };
}

test "dependent domain propagation in type patterns v2" {
    assert(unbox_bool(Box<bool, true>));
    assert(not unbox_bool(Box<u32, 7>));

    assert(unbox_as(u32, Box<u32, 7>, 0) == 7);
    assert(unbox_as(u32, Box<bool, true>, 0) == 0);

    -- 写法 C：甚至可以直接在无约束的 Box<Kind, contents> 中把拆出来的类型 Kind 和值 contents 重新组装！
    let echoed = Box<bool, true> match {
        Box<Kind, contents> => Box<Kind, contents>,
        _ => void,
    };
    assert(echoed == Box<bool, true>);
}
```

---

## 五、 边界警示：类型模式只匹配保留的 Normal-Form 头，不是任意函数的逆运算！

在享受类型模式匹配的强大能力时，必须牢记我们在第 078 回学过的语义边界：
**类型模式 `<...>` 观察的是被保留下来的 Normal-Form 构造器头（如 `Array<...>`、`Tuple<...>`、`Box<...>`、`Identity<...>`）**。

1. **普通圆括号函数不留头**：如果你写了一个普通圆括号函数 `pure comptime fn storage_for(T: Type) -> Type { u32 }`，调用 `storage_for(bool)` 求值完直接就是裸 `u32`，它身上**没有任何关于 `bool` 的头记录**，自然不可能通过模式匹配从 `u32` 里倒推出 `bool`。
2. **外层 Normal-Form 头不会自动剥壳**：如果你把 `Box<bool, true>` 包进了 `Identity<Box<bool, true>>` 里，由于 `Identity<...>` 是一个独立的 Normal-Form 头，它**绝不会**自动展开成里面的 `Box<bool, true>`！

```flurry
pure comptime fn Identity<T: Type> -> Type {
    T
}

test "outer normal form head does not silently unfold in pattern match" {
    -- Identity<Box<bool, true>> 的最外层头是 Identity，不是 Box！
    -- 因此 unbox_bool 会落入 _ 分支返回 false，除非你显式匹配 Identity<Box<...>>
    assert(not unbox_bool(Identity<Box<bool, true>>));
}
```

还有，如果在类型模式里试图把一个与第 1 槽位类型 `Kind` 毫不相干的常量写进第 2 槽位（违反依赖参数域）：

```flurry
-- ❌ 编译期直接拒绝！当第 1 槽位已锚定为 'bool 时，第 2 槽位属于 bool 域，不能用整数模式 123 匹配
pure comptime fn invalid_dependent_pattern(B: Type) -> bool {
    return B match {
        Box<'bool, 123> => true,
        _ => false,
    };
}
```

---

## 六、 本篇小结

| 模式写法 | 语法分类 | 精确语义 |
| :--- | :--- | :--- |
| `Head<...>` | Normal-Form 头模式 | 检验目标类型的最外层 Normal-Form 构造器头是否为 `Head`，并逐槽匹配参数 |
| `id`（无单引号） | 新绑定模式变量 | 在当前模式分支中引入新变量 `id`，绑定对应槽位的类型或编译期值 |
| `'id`（带单引号） | 外部类型/值引用模式 | 查找当前作用域已有的 `id`，要求对应槽位的实参必须与之相等 |
| `'Expected as Kind` | 别名绑定模式 | 既检验槽位等于 `Expected`，又将其绑定到模式变量 `Kind` 供右侧依赖槽位使用 |
| `Kind and Kind is 'T` | 合取守卫模式 | 引入绑定 `Kind` 并附加同级模式约束 `Kind is 'T` |

---

## 七、 留给下一篇的悬念

回顾我们从第 076 回一路走来的旅程：
- 我们写过 `Type.size_of(T) == 8`（布局大小相等）；
- 写过 `Type.predicates.is_tuple(Itself)`（结构分类谓词）；
- 写过 `0 <= index and index + 1 < length`（编译期整数线性不等式）；
- 写过 `T match { Array<'u32, length> => ... }`（类型模式匹配）。

你有没有想过：在 Flurry 编译器内部，是谁在统一指挥和求解这些五花八门的逻辑命题？
当编译器面对一个复杂的 `where requires ...` 子句时，如果它既证不出命题为真，也找不出具体的反例（比如遇到了一个复杂的非线性方程 `value * value < limit`），编译器会给出什么结论？为什么说**“未能证出（`NotEntailed`）”绝不等于“证明了它的反面为真”**？

下一回 **《Flurry 演进手记 (084)：初识命题类型约束（PTC）——为什么类型系统需要一个专门的逻辑求解器》**，我们将揭开 Flurry 类型检查器背后的逻辑大脑——**PTC（Propositional Type Constraints）** 的架构奥秘！
