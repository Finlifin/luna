# Flurry 演进手记 (185)：类型构造头 `Name<...>` 与类型计算函数 `Name(...)`——类型身份、编译期纯度与多点实例化

在[上一篇（第 184 篇）](./184-candidate-solving-isolated-probing-and-commit.md)中，我们学习了 Trait 实现的结构精度选择法则。

在第四卷和第八卷中，我们见过两种返回 `Type` 的编译期纯函数写法：
1. **尖括号参数列表（Normal-Form 类型构造头）**：
   `pure comptime fn Identity<T: Type> -> Type` 或 `pure comptime fn Tuple<...Es: meta.List<Type>> -> Type`
2. **圆括号参数列表（普通编译期类型计算函数）**：
   `pure comptime fn SelectType(wide: bool) -> Type`

很多读者会问：**“这两种写法不都是返回 `Type` 的编译期纯函数吗？为什么 `SelectType(false)` 求值后直接等于 `i32`，而 `Identity<i32>` 哪怕函数体里写着返回 `T`，在类型比较时却绝不等于 `i32`？”**

今天，我们就结合 `docs/comptime/purity-and-normal-form.md` 与 `docs/type/type-identity.md`，彻底讲透这条贯穿 Flurry 类型系统的核心法则。

---

## 一、 核心对比：Normal-Form 类型构造头 vs 普通类型计算函数

让我们把这两种返回 `Type` 的函数放在一起对比：

```flurry
-- 1. 尖括号声明：定义一个新的 Normal-Form 类型构造头（Nominal / Structural Constructor Head）
pure comptime fn Identity<T: Type> -> Type {
    T
}

-- 2. 圆括号声明：定义一个普通的编译期类型计算函数（Type Factory / Metafunction）
pure comptime fn SelectType(wide: bool) -> Type {
    inline if wide { i64 } else { i32 }
}

fn keep_selected(
    comptime wide: bool,
    value: defer SelectType(wide),
) -> SelectType(wide) {
    value
}

const SMALL: i32 = keep_selected(false, 3i32);
const LARGE: i64 = keep_selected(true, 7i64);
```

### 核心规则：`Identity<T>` 保留类型构造头身份，不展开为 `T`

为什么 Flurry 要严格区分 `Name<...>` 与 `Name(...)`？

```text
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 1. 普通编译期类型函数：SelectType(wide: bool) -> Type                    │
  │    • 角色：根据条件选择或拼装已有类型的“类型工厂函数”                    │
  │    • 行为：当传入 wide = false 时，直接执行函数体归约为结果类型 `i32`     │
  │    • 结论：SelectType(false) == i32，SelectType(true) == i64            │
  ├─────────────────────────────────────────────────────────────────────────┤
  │ 2. Normal-Form 类型构造头：Identity<T: Type> -> Type                    │
  │    • 角色：像 struct Box<T> 或 Tuple<...Es> 一样，定义独立的类型构造器    │
  │    • 行为：Identity<i32> 本身就是归一化的最终类型形式（Normal Form），    │
  │            类型相等比较时【绝不展开函数体】将其抹平为 i32                 │
  │    • 结论：Identity<i32> == Identity<i32>，但 Identity<i32> != i32       │
  └─────────────────────────────────────────────────────────────────────────┘
```

试想：如果 `Array<T, n>`、`Tuple<...Es: meta.List<Type>>` 或 `Identity<T>` 在类型比较时都被无条件展开成它们底层的存储表示，那么：
- 你为 `Identity<i32>` 单独实现的 `impl Describe for Identity<i32>` 就会与 `i32` 本身的实现发生冲突；
- 结构精度匹配（如第 184 篇中的 `Sample<i32>`）也会因为外层构造头消失而彻底失效。

因此，在 Flurry 中：
- 凡是用 **`pure comptime fn Name<...> -> Type`**（以及 `struct`、`enum`、`union`、`newtype`）定义的类型构造子，应用后的 `Name<Args>` 本身就是不可再削去的 **Normal-Form 类型头**；
- 凡是用 **`pure comptime fn Name(...) -> Type`**（或 `typealias`）定义的透明映射，则在编译期求值后直接归约为其返回的目标类型。

---

## 二、 泛型定义的多点独立实例化

再来看 `keep_selected(comptime wide: bool, value: defer SelectType(wide)) -> SelectType(wide)`：
1. 在定义 `keep_selected` 时，`SelectType(wide)` 因为依赖尚未给定的 `comptime wide: bool`，作为合法的编译期延迟类型表达式保存在函数签名契约中；
2. 当第 1 个调用点 `keep_selected(false, 3i32)` 出现时，代入 `wide = false`，在本次调用的独立上下文中算出参数与返回类型为 `i32`；
3. 当第 2 个调用点 `keep_selected(true, 7i64)` 出现时，代入 `wide = true`，在全新的调用上下文中算出参数与返回类型为 `i64`。

各次调用点的实例化彼此完全隔离，互不干扰：

```flurry
test independent_instantiations {
    let a = keep_selected(false, 10i32);
    let b = keep_selected(true, 20i64);

    comptime {
        assumes a'type == i32;
        assumes b'type == i64;
        assumes Identity<i32> != i32;
        assumes SelectType(false) == i32;
    }
    assumes a == 10i32;
    assumes b == 20i64;
}
```

---

## 三、 拒绝例：混淆 `Identity<T>` 与 `T`，或在函数定义内留下未决类型

### 拒绝例 1：试图把 `T` 直接赋给 `Identity<T>`

```flurry
pure comptime fn Identity<T: Type> -> Type {
    T
}

fn reject_identity_mixing(x: i32) -> Identity<i32> {
    -- ❌ 编译期拒绝：Normal-Form 头不展开，Identity<i32> 与 i32 不是同一个类型。
    x
}
```

### 拒绝例 2：函数定义内部存在既不依赖形参、也无法推导出的悬空类型

虽然泛型函数的签名可以保留依赖 `comptime` 形参的 `SelectType(wide)`，但函数体内部绝不允许留下任何既不依赖形参、也没有上下文约束的未决类型变量：

```flurry
fn empty_buffer() -> T
where
    T: Type
{
    ...
}

fn unresolved_internal_local(x: i32) -> i32 {
    -- ❌ 编译期拒绝：局部变量 y 的类型 T 既不依赖 unresolved_internal_local 的形参，
    -- 也没有显式类型注解，不允许在函数定义内保留悬空的未决类型。
    let y = empty_buffer();
    x
}
```

---

## 四、 本篇小结

| 声明形式 | 语法特征 | 类型相等与求值行为 |
| :--- | :--- | :--- |
| **Normal-Form 类型构造头** | `pure comptime fn Name<...> -> Type` | 保持 `Name<Args>` 构造头身份，`Identity<T> != T`，支持独立的 Trait 实现与类型模式匹配 |
| **普通类型计算函数** | `pure comptime fn Name(...) -> Type` | 按实参在编译期执行函数体并直接归约为返回的目标类型（如 `SelectType(false) == i32`） |
| **延迟参数类型 `defer`** | `value: defer SelectType(wide)` | 在定义期验证契约合法性，在调用点获得前序 `comptime` 实参后精确归约 |

---

## 五、 下一篇预告

在 `keep_selected(comptime wide: bool, ...)` 中，被依赖的参数 `wide` 是一个编译期常量。
如果一个类型依赖的是**运行期才会产生的对象或指针**（例如校准器指针 `cal: *Calibration`），我们在前面卷目中学习过可以用 `where quote cal: lift *Calibration` 来声明运行时依赖类型 `Sensor<cal>`。
- 在复杂工程中，当一个容器结构体 `SensorPair` 同时持有两个绑定到不同运行期校准器的 `Sensor<first.ref>` 与 `Sensor<second.ref>` 时，应该如何组织代码？
- 为什么决不能返回绑定在函数局部栈变量上的运行时依赖类型？

下一篇 **[Flurry 演进手记 (186)：运行时依赖类型的工程模式——多实例身份隔离、外层环境共享与逃逸防线](./186-representing-runtime-dependent-types-in-the-compiler.md)**，我们将深入运行时依赖类型的实战模式。
