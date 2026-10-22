# Flurry 演进手记 (106)：当编译期无从得知具体类型时——dyn Trait 动态接口与 .dyn(Trait) 显式擦除

在第五卷的前半程（第 096 篇至第 105 篇），我们见识了 Flurry 静态泛型分派的威力：当你写下 `fn read_sensor(s: *T) where T :- Reading` 时，编译器会在实例化（Instantiation）阶段把 `T` 替换成具体的 `Thermometer` 或 `Barometer`，生成零间接调用开销的机器码。

但是，在正在构建的工业级物联网边缘网关与通用编解码器框架中，我们很快就撞上了一面**编译期单态化无法翻越的墙**：

1. **运行时配置驱动的插件选择**：网关启动时去读一份外部配置文件 `config.toml`，里面写着 `"codec = protobuf"` 或 `"codec = msgpack"`。在编译网关二进制文件时，编译器根本无从得知用户会在现场開啟哪一种编解码器。
2. **异构设备集合（Heterogeneous Collections）**：一块采集背板上插着温度计 `Gauge`、恒值校准源 `FixedReading` 和脉冲计数器 `PulseCounter`。我们想把这三种**具体类型大相径庭、内存大小也不同**的传感器句柄放进同一个巡检列表里，依次调用它们的 `.read()` 方法。

如果只有编译期泛型 `T :- Reading`，一个数组 `[T; 3]` 要求所有元素必须是**同一个具体类型 `T`**。
当编译期无法知道具体类型，或者我们需要抹平不同具体类型之间的差异时，该怎么办？

今天，我们就来认识 Flurry 处理运行时多态的核心武器：**前缀动态接口类型 `dyn Trait`** 与 **后缀显式擦除操作符 `.dyn(Trait)`**。

---

## 静态泛型在异构集合与动态路由面前的窘境

假设我们有两个实现了 `Reading` 接口的传感器类型：`Gauge`（占用 4 字节存储当前读数）和 `FixedReading`（零字节空结构体，固定返回 `100`）：

```flurry
trait Reading {
    fn read(*self) -> u32;
}

struct Gauge {
    value: u32,
}

struct FixedReading {}

impl Reading for Gauge {
    fn read(*self) -> u32 {
        self.value.*
    }
}

impl Reading for FixedReading {
    fn read(*self) -> u32 {
        100
    }
}
```

如果我们试图写一个静态泛型函数，根据运行时的布尔开关返回其中一个传感器的引用，或者把它们塞进同一个同构函数入口而隐匿具体类型：

```flurry
-- ❌ 编译期直接报错。
-- if 的两个分支返回的具体类型不同：一个是 *Gauge，一个是 *FixedReading。
-- 静态类型 T 只能绑定为单一的具体类型，无法同时代表两种各异的传感器。
fn pick_sensor(use_fixed: bool, g: *Gauge, f: *FixedReading) -> *T
where
    T :- Reading
{
    if use_fixed {
        f
    } else {
        g
    }
}
```

我们需要一种类型，它对调用者说：**“我把里面的具体类型（隐藏主体）藏起来了，但我向你交付了一份完整的 `Reading` 接口调用凭证。”**

---

## `dyn Trait` 与 `.dyn(Trait)`——免作隐式装箱，显式完成类型擦除

在有些面向对象语言（如 Java / Go）或早期的 Rust 中，把一个具体对象赋给接口类型往往伴随着隐式的类型擦除（甚至在 Go 里还会触发静默的堆内存逃逸分配）。而在系统级编程中，**“什么时候发生了类型擦除、擦除成了哪个接口”** 直接关系到分派方式与内存布局，无法靠编译器悄悄猜。

因此，Flurry 采用了清晰明确、对称的设计：
- **在类型位置**：使用前缀 **`dyn TraitExpr`**（例如 `dyn Reading`）表示一个动态接口类型；
- **在表达式位置**：使用后缀 **`.dyn(TraitExpr)`**（例如 `gauge.ref.dyn(Reading)`）显式将具体主体封装为动态接口。

让我们把刚才的代码改写如下：

```flurry
-- 接收一个动态接口 dyn Reading：
-- show_dynamic 无从得知里面装的是 Gauge 还是 FixedReading，也无需为它们分别单态化一份代码。
fn show_dynamic(reading: dyn Reading) {
    println("{}", reading.read());
}

test {
    let gauge = Gauge { value: 27 }
    let fixed = FixedReading {}

    -- 使用 .ref 取得指针，再用 .dyn(Reading) 显式封装为 dyn Reading 动态接口。
    show_dynamic(gauge.ref.dyn(Reading));
    show_dynamic(fixed.ref.dyn(Reading));
}
```

运行这段代码，它会干净利落地输出 `27` 和 `100`。

> **🔍 语法小贴士：为什么 `gauge.ref.dyn(Reading)` 无需先写 `impl Reading for *Gauge`？**
> 注意看。我们只实现了 `impl Reading for Gauge`（其方法接收者是 `*self`），并没有单独写一份 `impl Reading for *Gauge`。
> 当我们对指针 `gauge.ref` 调用 `.dyn(Reading)` 时，Flurry 会直接沿用**接收者适配（Receiver Adaptation）**规则取得主体 `Gauge` 的 `Reading` 接口，并把指针 `gauge.ref` 封装进动态视图中。

---

## 揭秘 `dyn Trait` 的内存画卷与生命周期核心规则

当你写下 `gauge.ref.dyn(Reading)` 时，底层到底生成了什么？它占多大内存？谁来负责清理 `gauge`？

在 Flurry 的规范（`docs/traits/dynamic-interfaces.md`）中，**裸 `dyn Trait`（即不携带额外外层闭包环境限定时的 `dyn Trait`）的大小被严格保证为固定两个 `usize`（在 64 位机器上即 16 字节）。**

下面拆解这两个机器字（2 × `usize`）里装的到底是什么，以及**指针封装**与**按值封装**的本质区别：

```
+-------------------------------------------------------------------------+
|  裸 dyn Reading 的双字表示与所有权边界                         |
|                                                                         |
|  栈上局部变量：                     调用点构造的 `dyn Reading`          |
|  let gauge = Gauge { value: 27 }    gauge.ref.dyn(Reading)              |
|  +------------------+               +----------------+---------------+  |
|  | value: 27 (u32)  | ◄──────────── | 数据入口 (usize)| 分派入口(usize)| |
|  +------------------+               +----------------+---------------+  |
|  （仍归原作用域管理）                │ 固定大小：2 * Type.size_of(usize)|
|                                      └───────────────┬───────────────┘  |
|                                                      ▼                  |
|                                       静态只读分派表（Gauge as Reading）|
|                                       +------------------------------+  |
|                                       | read ──► Gauge::read(*self)  |  |
|                                       +------------------------------+  |
+-------------------------------------------------------------------------+
|  ⚠️ 尤为关键的资源与所有权核心规则：                                        |
|  1. 封装已有指针（如 `gauge.ref.dyn(Reading)`）：                       |
|     • 动态视图只是借用了指向 `gauge` 的指针，**不会接管**外部对象的     |
|       析构/清理责任  当 `show_dynamic` 结束时，外面的 `gauge` 依然      |
|       活得好好的，由原作用域负责管理。                                  |
|     • 但这也意味着：动态视图保留了对外部对象 `gauge` 的可达性依赖       |
|                                                                         |
|  2. 按值封装（如将拥有所有权的主体直接 `.dyn(...)`）：                  |
|     • 按主体实际的移动、复制与清理契约交付，退出作用域时兑现清理责任    |
|     • 不应当把所有 `dyn Trait` 都盲目当成“无脑复制的胖指针”             |
+-------------------------------------------------------------------------+
```

### 编译期安全红线：防止通过 `dyn` 逃逸悬垂引用

有些习惯了 C 语言 `void*` 接口的程序员可能会想：“既然 `local.ref.dyn(Reading)` 把具体类型 `*Gauge` 擦除成了 `dyn Reading`，那编译器是否就看不见里面藏着一个指向栈上局部变量 `local` 的指针了？我能否把它直接 `return` 出函数？”

**难以。** 在 Flurry 中，隐藏具体类型**不会消除它对底层存储生命周期的依赖**。让我们看一个被编译器当场抓获的负例：

```flurry
-- ❌ 编译期直接报错。
-- 错误的逃逸无法被 dyn 隐藏。
fn dangling_reading() -> dyn Reading {
    let local = Gauge { value: 5 }
    -- local.ref.dyn(Reading) 构造出的动态视图依然携带对局部变量 `local` 的借用依赖。
    -- 函数退出后 `local` 立即销毁，返回该动态接口会导致悬垂指针。
    local.ref.dyn(Reading)
}
```

同样地，如果你没有显式调用 `.dyn(Reading)`，试图直接把具体类型塞给 `dyn Reading` 参数，编译器也会免作静默隐式转换：

```flurry
fn test_must_be_explicit() {
    let gauge = Gauge { value: 27 }

    -- ❌ 编译期直接报错。
    -- Flurry 免作隐式的动态接口擦除，必须显式写出 `gauge.ref.dyn(Reading)`。
    show_dynamic(gauge.ref);
}
```

> **💡 核心心智模型：实现稳定性（Implementation Stability）**
> 一旦你在调用 `.dyn(Reading)` 的那一刻完成了动态封装，该动态对象所使用的 `Reading` 实现就已经被**永久锁定**为封装时交付的那份实现。哪怕后续调用点所在的作用域导入了别的同名方法或适配路径，运行时亦免于重新去搜索或改变这个动态对象的实现来源。此外，如果编译器在局部通过上下文证明了某个 `dyn Reading` 的具体入口唯一，它甚至可以自动去虚化（Devirtualize）消除间接跳转开销。

---

## 本篇小结

| 核心概念 / 语法 | 形式与规则 | 核心安全契约 |
| :--- | :--- | :--- |
| **动态接口类型** | `dyn TraitExpr`（如 `dyn Reading`） | 隐藏具体主体类型，仅暴露 `TraitExpr` 承诺的可调用成员；裸 `dyn Trait` 固定为 `2 * usize` 大小 |
| **显式擦除构造** | `expr.dyn(TraitExpr)` | 必须显式调用以交付实现证据与分派入口，免作隐式装箱或隐式擦除 |
| **指针封装 vs 按值封装** | `obj.ref.dyn(T)` vs `obj.dyn(T)` | 封装指针不接管外部对象的清理责任，但保留对原对象生命周期的依赖；严禁借 `dyn` 逃逸悬垂引用 |
| **实现稳定性** | 封装时刻绑定 | 动态调用严格使用 `.dyn(...)` 封装时确定的实现，不受后续作用域导入影响 |

---

### 下一篇预告

用过 Rust `dyn Trait` 的同学，心里一定有一个挥之不去的阴影——**Object Safety（对象安全 / `dyn` 兼容性）限制**。
在 Rust 里，只要你的 Trait 里写了一个返回 `Self` 的方法（比如 `fn duplicate(&self) -> Self` 或 `Clone`），编译器就会立刻报错告诉你：`this trait cannot be made into an object (not dyn-compatible)`。因为调用者无从得知隐藏的 `Self` 具体占几个字节，没法直接当裸返回值接住它。
更别提如果某个 `impl` 还额外依赖一个外层的运行时校准器或分配器环境（Environment），普通两字宽的胖指针根本装不下。

那么在 Flurry 中呢？
**Flurry 却允许在 `dyn Duplicate` 上直接调用返回 `Self` 的 `source.duplicate()` 方法？而且还能用 `%c dyn Trait` 精确追踪动态接口携带的额外运行时环境？**

它是怎么做到的？哪些边界是允许的，哪些又是严格禁止的（比如两个不同 `dyn Trait` 混传 `Self` 参数）？
下一篇 **`107-self-results-and-dependent-environments-in-dyn-traits.md`**，我们将深入探索 Flurry 动态接口最令人惊叹的高级边界。
