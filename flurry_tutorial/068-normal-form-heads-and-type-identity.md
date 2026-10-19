# Flurry 演进手记 (068)：为什么 `Identity<T>` 不等于 `T`——`pure comptime fn Name<...>` 与 Normal-Form 头

在 [上一篇 (067)](./067-functions-that-return-types.md) 中，我们见识了用圆括号调用的普通编译期类型函数：

```flurry
pure comptime fn Identity(T: Type) -> Type {
    T
}
typealias Number = Identity(u32);
```

因为圆括号调用 `Identity(u32)` 执行了函数体，返回的正是 `u32`，所以 `Number` 完全等价于 `u32`，你可以直接写 `let n: Number = 9u32;`。

但是，在语言规范（`docs/comptime/purity-and-normal-form.md`、`docs/type/type-identity.md`）以及开发协作协议（`AGENTS.md` §5 第一条红线）中，矗立着一条铁律：

> **Normal form 的头不展开；`Identity<T>` 不因函数体返回 `T` 就与 `T` 相等！**

初学者第一次看到这句话时，脑袋里一定会冒出三个大大的问号：
1. 什么是 **Normal-Form 头（范式头）**？它跟刚才的圆括号 `comptime fn` 在语法上到底差在哪儿？
2. 既然我在 `pure comptime fn Identity<T: Type> -> Type { T }` 的函数体里明明写了 `{ T }`，为什么编译器偏偏**不展开**它，非要让 `Identity<i32>` 和 `i32` 不相等？
3. 如果所有的类型函数都像圆括号那样无脑展开，我们的静态张量库 `Tensor<f64, 128>`、Trait 实现和类型模式匹配会发生什么毁灭性的灾难？

今天，让我们通过一次惊心动魄的设计推演，彻底搞懂为什么 **Normal-Form 头**是 Flurry 统一泛型与元编程的定海神针！

---

## 第一版：如果所有类型构造器都自动展开——“类型身份蒸发”惨案（v0）

假设我们在设计静态张量库时，想要定义两种不同的张量包装类型，或者两个在底层碰巧都用 `u32` 存储的泛型标签类型：
- `SensorTag<T>`：代表传感器通道 `T` 的硬件句柄编号；
- `StorageFor<T>`：代表类型 `T` 对应的寄存器槽位。

如果一门语言里**只有**“一调用就立刻执行函数体并展开成返回值”的圆括号类型函数（`v0`），看看会发生什么可怕的事情：

```flurry
use std.prelude.*;

-- v0：使用普通圆括号 comptime fn，调用后会彻底展开为返回值 u32！
pure comptime fn SensorTag_v0(T: Type) -> Type {
    u32
}

pure comptime fn StorageFor_v0(T: Type) -> Type {
    u32
}

comptime {
    -- 😱 灾难 1：两个风马牛不相及的领域概念，展开后全都变成了 u32，居然完全相等！
    assumes SensorTag_v0(i32) == StorageFor_v0(f32);
}
```

让我们画一张 **Code Cartoon**，看看“无条件展开”为什么会摧毁泛型系统的根基：

```text
  当所有类型函数都执行展开时（身份蒸发！）：

  SensorTag_v0(i32) ──(执行函数体)──► u32 ◄──(执行函数体)── StorageFor_v0(f32)
                                      │
          ┌───────────────────────────┼───────────────────────────┐
          ▼                           ▼                           ▼
   [灾难 1：类型混淆]          [灾难 2：无法反推参数 T]     [灾难 3：无法单独特化 Trait]
   温度通道的 Tag 可以直接      泛型函数收到一个 `u32`，      你想写 `impl Print for SensorTag<T>`，
   传给需要电压 Storage 的函数！ 根本不知道它原本是 i32       结果变成给底层的 `u32` 实现 Print！
                              还是 f32 算出来的！
```

看到了吗？
在 `docs/type/type-identity.md` 中有一个经典的例子：
```flurry
pure comptime fn storage_for(T: Type) -> Type { u32 }
```
因为 `storage_for(i32)` 和 `storage_for(f32)` 执行后都变成了 `u32`，编译器拿到 `u32` 时，原来的类型构造器名字 `storage_for` 和类型实参 `i32` / `f32` **已经在求值过程中彻底蒸发了**！你既不可能从结果 `u32` 逆推出输入 `T` 是 `i32` 还是 `f32`，也不可能在类型模式匹配里写 `ty match { storage_for(T) => ... }`！

要想让一个参数化类型像名义类型（Nominal Type）一样**保住自己的姓氏（Head）和血统参数（Arguments）**，我们需要一种**在语法和语义上明确声明“保留头部，绝不化简展开”**的定义形式！

---

## 第二版：尖括号 `<...>` 登场——Normal-Form 头定义与应用（v1）

在 Flurry 中，这正是**尖括号 `<...>`** 与**圆括号 `(...)`** 的神圣分工（详见 `docs/grammar/syntax` 与 `docs/comptime/purity-and-normal-form.md`）：

- **圆括号 `pure? comptime fn Name(params) -> Type`**：
  这是**普通编译期函数（Ordinary Comptime Function）**。调用 `Name(args)` 具有**执行语义**——运行函数体，拿返回值替换掉自身。
- **尖括号 `pure comptime fn Name<params> -> Type`**：
  这是 **Normal-Form 头定义（范式头定义，`normal_form_head_def`）**！
  当你用尖括号调用它——写成 **`Name<args>`（范式头应用，`normal_form_head_application`）** 时，编译器在归约后的中间表示（Normalized IR）中**严格保留 `Name` 这个头（Head）以及它的实参 `args`，绝对不会通过展开 `Name` 的函数体来把它化简成里面的返回表达式！**

让我们把规范中最核心的对比代码写出来（`v1`）：

```flurry
use std.prelude.*;

-- ============================================================================
-- 形态 A：圆括号 `(T: Type)` —— 普通 comptime 函数（执行并展开！）
-- ============================================================================
pure comptime fn IdentityEval(T: Type) -> Type {
    T
}

typealias EvalElem = IdentityEval(i32);

-- ============================================================================
-- 形态 B：尖括号 `<T: Type>` —— Normal-Form 头定义（保留头与实参，绝不展开！）
-- ============================================================================
pure comptime fn Identity<T: Type> -> Type {
    T
}

-- typealias 只给目标起另一个名字；当目标是范式应用 Identity<i32> 时，
-- Element 指向的依然是保留了头的 Identity<i32>，绝不会变成 i32！
typealias Element = Identity<i32>;

-- ✅ 合法：签名两边都是 Element (即 Identity<i32>)，类型身份完全一致！
fn keep_element(value: Element) -> Element {
    value
}

test {
    -- 对于形态 A (圆括号展开)：EvalElem 就是 i32，可以直接赋 42 并做加法！
    let a: EvalElem = 42i32;
    println("{}", a + 1i32);
}
```

现在，如果我们试图把一个普通的 `i32` 字面量（比如 `9i32`）直接赋给形态 B 的 `Element`（即 `Identity<i32>`），或者试图在 `Element` 上直接用 `i32` 的算术加法，会发生什么？

```flurry
-- ❌ 编译期直接拒绝！
fn bad_identity_assumption(x: Identity<i32>) -> i32 {
    -- 错误！Normal-Form 的头不展开！
    -- 尽管 Identity<T> 的定义体里写着 `{ T }`，
    -- 但 `Identity<i32>` 在类型系统里是一个保留了 `Identity` 头的独立范式项，
    -- 它不等于 `i32`，不能直接用数值初始化，也不会自动白嫖 `i32` 的操作能力！
    let bad_init: Identity<i32> = 9i32;
    x + 1i32
}
```

### 给圆括号 vs 尖括号画张透视图（Code Cartoon）

让我们用一张 **Code Cartoon** 把这两者在编译器内部的 Normalized IR（归约后中间表示）彻底看清：

```text
  ══════════════════ 圆括号普通调用 vs 尖括号 Normal-Form 应用 ══════════════════

  [1] 普通调用 (Execution Semantics):
      源码:   pure comptime fn IdentityEval(T: Type) -> Type { T }
              typealias EvalElem = IdentityEval(i32);
                                        │
                                        ▼ (执行函数体，剥掉外壳！)
      Normalized IR 中的类型节点:  [ Primitive: i32 ]  (与直接写 i32 完全相等！)

  ─────────────────────────────────────────────────────────────────────────────

  [2] 范式应用 (Normal-Form Head Retention):
      源码:   pure comptime fn Identity<T: Type> -> Type { T }
              typealias Element = Identity<i32>;
                                        │
                                        ▼ (保留头身份与实参，封印函数体！)
      Normalized IR 中的类型节点:  [ NormalFormApp: head = Identity, args = [i32] ]
                                        │
                                        └──► ❌ 绝不等于 [ Primitive: i32 ]！
                                             ✅ 仅等于同样拥有 head = Identity 且实参相等的 `Identity<i32>`！
```

---

## 第三版：为什么保留 Normal-Form 头如此重要？——模式匹配、值索引与张量标识（v2）

读到这里，你一定会问：“既然 `Identity<T>` 的函数体不会被自动展开，那我们定义 `pure comptime fn Name<...> -> Type` 有什么用呢？”

用处太大了！正是因为 Normal-Form 头**不展开、不蒸发**，它才具备了普通函数调用梦寐以求的三大超能力：

### 超能力 1：支持编译期类型模式匹配（Type Pattern Matching）！

因为 `Configured<Settings, ...>` 保留了它的头 `Configured` 和里面的实参，我们可以在编译期函数里直接对一个 `ty: Type` 做 **`match` 解构**，把当初传进尖括号里的类型和配置值原封不动地掏出来（详见 `docs/functions/comptime_fn.md`）：

```flurry
use std.prelude.*;

struct Settings {
    enabled: bool,
}

enum Mode where T {
    disabled,
    configured(T),
}

-- 定义一个带 Type 参数和值参数的 Normal-Form 头！
pure comptime fn Configured<T, mode: Mode<T>> -> Type {
    bool
}

typealias EnabledType = Configured<
    Settings,
    Mode<Settings>.configured(Settings { enabled: true })
>;

-- 在编译期直接对 Type 值做模式匹配，拆开 Normal-Form 头的尖括号实参！
comptime fn is_enabled(ty: Type) -> bool {
    ty match {
        -- `'Settings` 是类型模式中的常量模式，匹配成功后掏出后面的枚举载荷！
        Configured<'Settings, .configured({ enabled: true })> => true,
        _ => false,
    }
}

comptime {
    -- ✅ 虽然 Configured 的函数体写着 `{ bool }`，但 EnabledType 绝没有变成 bool！
    -- 它完整保留了 Configured 头和里面的实参，因此 is_enabled 匹配成功！
    assumes is_enabled(EnabledType);
    assumes not is_enabled(bool);
}
```

想象一下：如果 `Configured<...>` 像圆括号函数那样在定义 `EnabledType` 时就展开成了 `bool`，那么传给 `is_enabled` 的就只剩下一个干瘪的 `bool`，里面的 `Settings` 和 `enabled: true` 早就灰飞烟灭了，根本不可能做类型模式匹配！

### 超能力 2：用编译期值索引不同的类型身份（Indexed Types）

在我们的静态张量库中，不同维度、不同标志的张量（如 `Indexed<true>` 与 `Indexed<false>`，或者 `Tensor<f64, 128>` 与 `Tensor<f64, 64>`）必须在类型系统里保持严格区分。
通过 Normal-Form 头，哪怕两个范式应用的底层函数体写得一样，只要尖括号里的索引值不同，它们就是两个截然不同的类型：

```flurry
pure comptime fn Indexed<index: u8> -> Type {
    u32
}

comptime {
    -- ✅ 相同的头 + 相同的索引值 ──► 类型相等！
    assumes Indexed<1u8> == Indexed<1u8>;
    -- ✅ 相同的头 + 不同的索引值 ──► 类型严格不等！（绝不会因为函数体都是 u32 就合并！）
    assumes Indexed<1u8> != Indexed<2u8>;
}
```

### 超能力 3：尖括号内部依然支持完整的编译期表达式计算！

注意：**“Normal-Form 的头不展开”指的是外层的头 `Indexed<...>` 不展开成函数体，但尖括号 `< ... >` 里面的实参表达式，依然是货真价实的编译期表达式，会正常求值化简！**

```flurry
comptime {
    -- 尖括号内部是真实表达式，不是词法文本替换！
    -- `1u8 + 1u8` 在编译期正常求值为 `2u8`，因此与 `Indexed<2u8>` 完全相等！
    assumes Indexed<1u8 + 1u8> == Indexed<2u8>;
}
```

---

## 本篇小结

| 对比维度 | 普通编译期类型函数 `( ... )` | Normal-Form 头 `< ... >` |
| :--- | :--- | :--- |
| **声明语法** | `pure? comptime fn Foo(T: Type) -> Type` | `pure comptime fn Foo<T: Type> -> Type` |
| **应用语法** | 圆括号 `Foo(i32)` | 尖括号 `Foo<i32>` |
| **归约行为** | **执行函数体**，直接化简为返回的 `Type` | **保留头 `Foo` 与实参**，绝不展开函数体 |
| **类型等价性** | 看函数体执行后的最终结果（`IdentityEval(i32) == i32`） | 看头身份与归约后的实参（`Identity<i32> != i32`） |
| **模式匹配解构** | ❌ 外壳已蒸发，无法匹配 `Foo(...)` | ✅ 可在 `ty match` 中直接匹配 `Foo<...>` 并提取实参 |

---

### 留给下一篇的悬念

既然我们搞懂了 Normal-Form 头 `pure comptime fn Name<...> -> Type`，现在请回想一下我们在前几卷里天天都在用的一个基础复合类型——**元组（`Tuple`）**！

在几乎所有其他编程语言（C++、Rust、Python、TypeScript）里，元组 `(A, B, C)` 都是编译器硬编码开后门的内置特殊语法，因为它的元素个数是任意可变的（0 个、1 个、2 个、10 个……），普通泛型根本写不出“接收任意多个不同类型参数”的类型构造器。

但是，请翻开 `AGENTS.md` §5 的第二条神圣红线：
> **“`Tuple` 的签名是 `pure comptime fn Tuple<...Es: meta.List<Type>> -> Type`；变参收集为一个类型列表输入，类型身份保留 `Tuple` 的 normal-form 头。”**

天哪！在 Flurry 里，连元组 **`Tuple`** 都不是什么高高在上的特权语法，它竟然只是一个带有变参收集 `...Es: meta.List<Type>` 的 **Normal-Form 头类型函数**！
- 当你写下 `Tuple<i32, bool>` 时，尖括号里的 `i32, bool` 是怎么变成一个 `meta.List<Type>` 的？
- 既然 `Tuple` 是一个保留头的 Normal-Form 类型函数，我们能不能模仿 `Tuple` 的签名，写出我们自己的变参张量类型 `Tensor<T: Type, ...dims: meta.List<usize>>` 或自定义类型包 `Pack<A, ...Es: meta.List<Type>>`？

在下一篇 **《Flurry 演进手记 (069)：连元组都是一个类型函数——深入 `pure comptime fn Tuple<...Es: meta.List<Type>> -> Type`》** 中，我们将亲手拆解这个惊艳的设计！
