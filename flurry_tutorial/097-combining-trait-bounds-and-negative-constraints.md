# Flurry 演进手记 (097)：组合多个 Trait 约束——where T :- A + B 正向约束与 requires 否定约束

> **系列导读**：在上一期（096）中，我们用 `trait`、`assoc` 和 `impl` 为遥测传感器立下了第一份名义能力契约。但在真实工程中，能力从来不是孤立存在的：一个遥测上报器往往要求传感器既具备读取能力（`Reading`），又具备身份命名能力（`Named`）；而一个高层次的诊断接口 `Report` 本身就建立在 `Reading` 与 `Named` 的基础之上。更进一步，为了保障系统安全，我们有时还需要明确排除某些标记（例如拒绝任何标记为 `Disabled` 的故障传感器进入生产流水线）。今天，我们将深入探索 Flurry 如何用 `:-` 与 `+` 表达**正向能力组合**，以及为什么**否定约束**必须交给 `requires` 子句由 PTC 严密证明。

---

## 一、单一接口的局限：传统方案的“能力碎片化”

回到我们的遥测引擎。假设我们有两个基础接口：负责读取数值的 `Reading` 和负责提供设备名称的 `Named`：

```flurry
trait Reading {
    fn read(*self) -> u32;
}

trait Named {
    fn name(*self) -> str;
}
```

在 传统阶段，如果我们不懂得如何组合多个 Trait 约束，也不懂得声明父接口（Supertrait），当我们想写一个同时打印设备名称和读数的通用函数 `show_reading` 时，就会陷入尴尬的境地：难道我们要为此专门新建一个把 `read` 和 `name` 复制粘贴一遍的巨型 `ReadingAndNamed` 接口，然后逼着所有传感器再去重复 `impl` 一遍吗？

```flurry
-- 笨拙地重复定义组合接口，导致每个设备都要重复写一遍样板 impl。
trait ReadingAndNamedBad {
    fn read(*self) -> u32;
    fn name(*self) -> str;
}
```

显然，我们需要一种**零样板、可代数合取**的方式，直接把已有的 `Reading` 和 `Named` 组合起来。

---

## 二、用 `:-` 与 `+` 组合正向能力与父接口（Supertraits）

在 Flurry 中，无论是在泛型函数的 `where` 子句里，还是在 `trait` 声明的父接口列表里，**正向能力约束统一使用 `:-` 符号，并直接复用表达式中的 `+` 运算符表示能力合取（Conjunction）**。

让我们看看 是如何用 `Reading + Named` 一步到位解决函数约束与父接口继承的：

```flurry
trait Reading {
    fn read(*self) -> u32;
}

trait Named {
    fn name(*self) -> str;
}

-- 1. 父接口组合（Supertraits）：
-- 声明实现 Report 的主体类型必须先同时满足 Reading 和 Named。
trait Report :- Reading + Named {
    -- 默认方法可以直接使用父接口提供的能力。
    fn report(*self) -> void {
        println("{}: {}", self.as(Named).name(), self.as(Reading).read());
    }
}

struct Meter {
    value: u32,
}

impl Reading for Meter {
    fn read(*self) -> u32 {
        self.value.*
    }
}

impl Named for Meter {
    fn name(*self) -> str {
        "meter"
    }
}

-- Meter 已经分别兑现了 Reading 和 Named，
-- 因此只需一行空块即可获得 Report 的全部默认能力。
impl Report for Meter {}

-- 2. 函数泛型约束组合：使用 where T :- Reading + Named
fn show_reading(value: *T) -> void
where T :- Reading + Named
{
    println("{}: {}", value.as(Named).name(), value.as(Reading).read());
}

test {
    let meter = Meter { value: 8u32 }
    meter.report();
    show_reading(meter.ref);
}
```

### `A + B` 能力合取的三项核心规则

当你在 Flurry 中写下 `trait Report :- Reading + Named` 或 `where T :- Reading + Named` 时，编译器内部是如何组织的？

```text
                [ impl Report for Meter ]
                            │
        编译器拆解 `+` 为逐项独立的能力证据义务 (Obligations)
                   /                 \
                  ▼                   ▼
  [ 证据 1: Meter :- Reading ]     [ 证据 2: Meter :- Named ]
  （保留各自独立的声明身份与成员来源，不因 + 而合并身份。）
```

请牢记这三条关于 `+` 组合的核心语义边界：
1. **逐项独立兑现，绝不偷懒**：如果把上面 `impl Named for Meter` 删掉，哪怕你在 `impl Report for Meter` 里面手写了一个 `fn name(*self) -> str`，编译器也会**直接拒绝**。因为提供子接口里的同名方法，不会自动替你创建父接口 `Named` 的实现证据。
2. **保留各自声明身份，不按书写顺序赋优先级**：`Reading + Named` 和 `Named + Reading` 完全等价。如果两个接口碰巧有同名方法，书写在前面的不会自动覆盖后面的，调用时使用 `.as(Trait)` 显式指明来源即可。
3. **支持带实参的完整接口实例**：`+` 号两边不仅可以是简单的接口名，还可以是带参数的完整接口实例，例如 `where T :- Encode<.json> + Encode<.binary> + Named`。

### 💡 语法辨析：声明并约束类型变量的两种等价写法

在 Flurry 的 `where` 子句中，声明类型变量 `T` 并约束它满足 `Reading + Named` 有两种完全合法且等价的写法：

```flurry
-- 写法 A（推荐语法糖）：在声明 T 的同时直接用 :- 绑定正向约束
fn process_a(value: *T) -> void
where T :- Reading + Named
{
    show_reading(value);
}

-- 写法 B（显式拆解形式）：先在 where 中声明类型变量 T，再用 requires 子句施加类型命题约束
fn process_b(value: *T) -> void
where
    T,
    requires T :- Reading + Named
{
    show_reading(value);
}

-- ❌ 语法错误。requires 是子句引导词，不是放在变量名和 Trait 之间的中缀运算符。
-- fn process_bad(value: *T) -> void where T requires Reading { ... }
```

理解了写法 B（`requires T :- Reading + Named`），你就瞬间明白了：`T :- Trait` 本质上就是一个交由 **PTC（证明论式类型检查器）** 验证的**类型命题（Type Proposition）**。而这也正是通向否定约束的钥匙。

---

## 三、为什么否定约束必须写进 `requires not (T :- Trait)`？

现在，假设我们在遥测系统中引入了一个标记接口 `trait Disabled {}`。任何被标记为已废弃或校准失效的传感器类型（如 `BrokenSensor`），都会实现 `impl Disabled for BrokenSensor {}`。

为了防止工程师在生产流水线中误传 `BrokenSensor`，我们要编写一个只接收正常传感器的安全入口 `show_enabled`。

这时候，很多初学者会下意识地想：“既然正向约束写在 `:-` 后面，那我直接写 `where T :- Reading + Named + not Disabled` 不就行了吗？”

**绝对不行。** 在 Flurry 中，这是一条明确的语法与语义边界：
> **`trait D :- ...` 和 `where T :- ...` 这两个 `:-` 位置专门用于正向能力组合（提供方法与关联项来源）；任何否定要求（如要求类型未实现某接口、或两个类型不相等）必须写进子句的 `requires` 中。**

让我们看看 正确的写法，以及编译器是如何在编译期拦截 `BrokenSensor` 的：

```flurry
trait Disabled {}

struct BrokenSensor {
    code: u32,
}

impl Reading for BrokenSensor {
    fn read(*self) -> u32 {
        self.code.*
    }
}

impl Named for BrokenSensor {
    fn name(*self) -> str {
        "broken"
    }
}

-- BrokenSensor 被标记为 Disabled
impl Disabled for BrokenSensor {}

-- 正向能力用 :- 组合，否定约束写进 requires 子句。
fn show_enabled(value: *T) -> void
where
    T :- Reading + Named,
    requires not (T :- Disabled)
{
    show_reading(value);
}

test {
    let meter = Meter { value: 8u32 }
    -- ✅ Meter 实现了 Reading 和 Named，且证明不满足 Disabled，编译通过。
    show_enabled(meter.ref);

    let broken = BrokenSensor { code: 404u32 }
    -- ❌ 编译期直接拒绝。
    -- BrokenSensor 实现了 Disabled，违反了 requires not (T :- Disabled)。
    -- show_enabled(broken.ref);
}
```

### ⚠️ 深度揭秘：在编译器眼里，“没找到实现”不等于“证明了不实现”

为什么 Flurry 要把 `requires not (T :- Disabled)` 单独交给 PTC 的否定查询引擎处理，而不是简单地在正向查找失败时返回 `true`？

想象下面这个场景：如果有一个泛型中转函数 `forward_unverified`，它只声明了 `where T :- Reading + Named`，**并没有**声明 `requires not (T :- Disabled)`，却试图在函数体内部直接调用 `show_enabled(value)`：

```flurry
fn forward_unverified(value: *T) -> void
where T :- Reading + Named
{
    -- ❌ 编译期直接拒绝。
    -- 在 forward_unverified 内部，T 只是一个尚未具体化的抽象类型参数。
    -- 虽然当前上下文中“找不到 T 实现 Disabled 的正向证据”，
    -- 但“未找到 / 仍未决（Pending）”绝不等于“已证明 not (T :- Disabled)”。
    show_enabled(value);
}
```

```text
[ PTC 对 (T :- Disabled) 的三种判定状态 ]
  ├── 1. 已成立 (Proven True)     ──► 如 BrokenSensor：明确存在 impl Disabled
  ├── 2. 仍未决 (Pending / Open)  ──► 如未加否定约束的泛型参数 T：未来实例化时可能变成 BrokenSensor。
  └── 3. 已排除 (Proven False)    ──► 如 Meter（结合包一致性规则证明绝不可能实现 Disabled）
                                      或当前泛型上下文已显式携带 requires not (T :- Disabled) 证据。
```

只有当状态为第 3 种 **“已排除（Proven False）”** 时，`requires not (T :- Disabled)` 才会放行。这就从根源上堵死了通过泛型擦除绕过否定约束的任何可能。

---

## 四、本节核心概念速查表

| 需求场景 | 标准语法 | 核心规则与避坑指南 |
| :--- | :--- | :--- |
| **父接口组合（Supertraits）** | `trait Sub :- A + B { ... }` | 实现 `Sub` 前必须分别提供 `A` 和 `B` 的有效实现；不可靠同名方法自动生成父接口实现 |
| **泛型多接口正向约束** | `where T :- A + B` | 声明 `T` 并要求其同时满足 `A` 与 `B`（支持带参接口如 `Encode<.json>`） |
| **显式类型命题写法** | `where T, requires T :- A + B` | 与 `where T :- A + B` 等价；切忌写成非法的 `where T requires A` |
| **否定 Trait 约束** | `where T :- A, requires not (T :- B)` | 否定要求必须放进 `requires` 子句；需具备真实排除证明，“查询未决”不能翻转成否定证明 |

---

## 五、下一期预告

现在，我们已经能自如地为类型编写 `impl` 并组合约束了。
但是在构建高性能序列化与遥测框架时，一个新的挑战马上就会出现：**特化（Specialization）**。

比如说，我们既想为任意元素类型的样本包装器提供一个通用的描述实现：
```flurry
impl Describe for Sample<T> where T { ... }
```
又想专门针对有符号整数 `Sample<i32>` 或字节数组 `Sample<[8]u8>` 提供一个高度优化的专属实现：
```flurry
impl Describe for Sample<i32> { ... }
```
当编译器看到 `Sample<i32>` 同时匹配这两份 `impl` 时，它是靠什么算法选出更精确的那一份的？如果有人写出了 `impl Classify for Pair<T, T>` 和 `impl Classify for Pair<A, B>`，重复出现的类型变量 `T` 会让前者的优先级更高吗？

下一期 **《Flurry 演进手记 (098)：当多个 impl 看起来都能匹配——结构索引、特化精度与实现选择算法》**，我们将拆解 Flurry 编译器内部的归一结构深度分解与候选精度偏序算法。
