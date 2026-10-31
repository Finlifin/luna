# Flurry 演进手记 (107)：连返回 Self 和带依赖环境都能动态调用？——Flurry 动态接口的高级边界

在上一篇中，我们学会了用 `dyn Reading` 和 `.dyn(Reading)` 把不同种类的传感器放进统一的运行时多态视图里。

紧接着，我们的边缘网关架构师提出了一个在传统系统语言里几乎棘手的需求：
> **“我们的动态插件接口 `Duplicate` 需要提供一个快照复制方法 `fn duplicate(*self) -> Self`。拿到一个 `source: dyn Duplicate` 之后，我想直接调用 `let copied = source.duplicate();` 拿到一份独立的快照，然后继续在 `copied` 上调用接口方法。”**

如果你是从 Rust 转过来的老手，听到这句话的第一反应一定是摇头苦笑：
在 Rust 中，一旦一个 Trait 的方法直接返回按值的 `Self`，这个 Trait 就会立刻丧失 `dyn` 兼容性（违反 Object Safety）。因为调用者在编译期无从得知隐藏的 `Self` 具体是谁，自然难以按裸具体类型接收它。你严禁不去搞一套繁琐的 `Box<dyn DuplicateClone>` 辅助 Trait 曲线救国。

非但如此，如果某个传感器的 `Reading` 实现还需要依赖一个外部运行时校准器或内存空间 `space: Space`，裸的两个 `usize` 根本装不下这份额外环境，怎么防止环境在跨界传递时被悄悄弄丢？

今天，我们就来看看 Flurry 是如何通过**返回 `Self` 的动态再封闭（Re-enclosure）机制**与**`%c dyn Trait` 显式环境闭包限定**，正面攻克这两座大山的。

---

## 为什么返回 `Self` 和接收 `Self` 参数是两码事？

在深入代码之前，我们先思考一个本质问题：
同样是在 `trait` 方法签名里出现 `Self`，**“返回 `Self`（Output `Self`）”** 和 **“接收 `Self` 参数（Input `Self`）”** 在动态调用时有什么根本区别？

```
+-------------------------------------------------------------------------+
|  返回 Self vs 接收 Self 参数的不对称性                         |
|                                                                         |
|  【情况 A：返回 Self —— `fn duplicate(*self) -> Self`】                 |
|    • 被调用的具体实现（比如 `CodeValue::duplicate`）自己清清楚楚知道     |
|      `Self` 是 `CodeValue`                                              |
|    • 它在内部构造出新的 `CodeValue` 后，只要连同 `CodeValue as Duplicate`|
|      的实现证据和环境一起**重新封闭（Re-enclose）**成一个新的           |
|      `dyn Duplicate` 交给调用者，调用者就能无缝继续使用  ✅             |
|                                                                         |
|  【情况 B：接收 Self 参数 —— `fn combine(*self, other: Self)`】         |
|    • 假设你有 `a: dyn Combiner`（里面藏着 `Gauge`）                    |
|      和 `b: dyn Combiner`（里面藏着 `PulseCounter`）。                 |
|    • 它们的表面类型都叫 `dyn Combiner`，但它们的隐藏主体 `Self`         |
|      根本不是同一个类型                                                 |
|    • 如果允许调用 `a.combine(b)`，`Gauge::combine` 就会把               |
|      `PulseCounter` 的内存当成 `Gauge` 来读，当场引爆内存安全问题  ❌   |
+-------------------------------------------------------------------------+
```

看明白这层非对称性，你就读懂了 Flurry 动态接口的设计哲学：
**凡是表示、入口和环境可以合法交付的操作（如将返回的 `Self` 重新封闭为动态接口），语言就全力支持；凡是缺乏具体类型同一性证明或运行时无法执行的操作，编译器就坚决拦截。**

---

## 返回 `Self` 的动态再封闭——让 `source.duplicate()` 成为现实

让我们直接来看 `docs/traits/dynamic-interfaces.md` 中的经典示例。
我们定义一个带 `fn duplicate(*self) -> Self` 的接口 `Duplicate`，并为 `CodeValue` 实现它：

```flurry
trait Duplicate {
    fn duplicate(*self) -> Self;
    fn code(*self) -> u32;
}

struct CodeValue {
    value: u32,
}

impl Duplicate for CodeValue {
    fn duplicate(*self) -> Self {
        Self { value: self.value.* }
    }

    fn code(*self) -> u32 {
        self.value.*
    }
}

test {
    let original = CodeValue { value: 9 }

    -- 1. source 是借用 original 指针封装的动态视图
    let source = original.ref.dyn(Duplicate);

    -- 2. 直接在动态接口上调用返回 Self 的 duplicate()。
    -- 内部生成新的 CodeValue，并自动连同实现证据重新封闭为新的动态接口对象 copied。
    let copied = source.duplicate();

    println("{} {}", source.code(), copied.code());
}
```

运行这段代码，它会稳稳输出 `9 9`。

请仔细体会 `source` 与 `copied` 在所有权与资源责任上的精妙差别：
1. **`source`** 是通过 `original.ref.dyn(Duplicate)` 按指针封装的，它只借用外部的 `original`，不负责清理 `original`；
2. **`copied`** 是 `source.duplicate()` 把新构造出来的 `CodeValue` 主体**按值重新封闭（Re-enclosed）**得到的动态接口对象。因此，`copied` 自己拥有这个新构造的隐藏主体，并独立承担新结果的生命周期与清理责任。

### 编译期红线 1：两个独立的 `dyn Trait` 无法假装拥有相同的 `Self` 参数

反过来，如果一个接口方法要求输入另一个 `Self` 参数（例如 `fn merge(*self, other: *Self) -> u32`），你能否拿两个都写成 `dyn Mergeable` 的变量互相传进去？

```flurry
trait Mergeable {
    fn merge(*self, other: *Self) -> u32;
}

fn bad_dyn_self_param(left: dyn Mergeable, right: dyn Mergeable) -> u32 {
    -- ❌ 编译期直接报错。
    -- 虽然 left 和 right 的类型都写作 `dyn Mergeable`，
    -- 但这完全无法证明它们各自隐藏的具体主体 `Self` 是同一个类型。
    -- 在没有同一隐藏主体证据的情况下，严禁跨对象混传 `Self` 参数。
    left.merge(right)
}
```

---

## 当动态接口携带额外环境——`%c dyn Trait` 显式环境追踪

在第 106 篇中我们提到：**裸 `dyn Trait`** 的大小固定为两个 `usize`（数据入口 + 分派入口）。
但是，如果某个具体实现非但需要对象本身，还依赖一个外部运行时环境（比如分配器空间 `space: Space`，或者校准器对象，我们将在第 108 篇深入学习开放实现环境），那这两个 `usize` 怎么放得下额外环境？如果把额外环境悄悄擦除掉，调用方法或执行 `Drop` 时岂不是无从寻获环境了？

Flurry 的答案是：**不暗中丢弃环境，而是用 `%c dyn Trait` 把所需的环境闭包限定（Closure Context）显式反映在类型上。**

- 当所选实现免去额外环境时，环境限定 `c` 为空，自动消去限定，化简为固定两个 `usize` 的**裸 `dyn Trait`**；
- 当所选实现需要额外环境时（例如需要名为 `space` 的 `Space` 环境），构造动态接口会推导出带环境限定的类型，如 **`%{space: Space = space} dyn Drop`**。

```
+-------------------------------------------------------------------------+
|  裸 dyn Trait vs 带环境限定的 %c dyn Trait                     |
|                                                                         |
|  1. 无额外环境实现 ──► 裸 `dyn Reading`                                 |
|     +----------------+----------------+                                 |
|     | 数据入口(usize)| 分派入口(usize)|  （固定 2 * usize）             |
|     +----------------+----------------+                                 |
|                                                                         |
|  2. 携带额外环境实现 ──► `%{space: Space = space} dyn Drop`             |
|     +----------------+----------------+------------------------------+  |
|     | 数据入口(usize)| 分派入口(usize)| 环境绑定 (space: Space = ...)|  |
|     +----------------+----------------+------------------------------+  |
|     ⚠️ 完整限定值的承载成本由环境 c 及其交付方式决定，                  |
|        不能把非空环境 c 悄悄抹掉塞进裸 `dyn Drop` 里                    |
+-------------------------------------------------------------------------+
```

那么，如果我们要写一个通用的转发或包装函数，既能接收未带环境的裸 `dyn Reading`，又能接收携带任意合法环境 `c` 的动态接口，该怎么写签名？
使用 `where c: Type.vars.Closure` 对环境做参数化即可。

```flurry
-- 环境参数化的转交函数：
-- 当 c 为空时，%c dyn Reading 就是裸 dyn Reading；
-- 当 c 并非空时，环境 c 随完整动态接口值一同安全交付。
fn relay_reading(reading: %c dyn Reading) -> %c dyn Reading
where
    c: Type.vars.Closure
{
    reading
}
```

如果你试图在转交函数里把返回类型强行改成裸 `dyn Reading`，企图把非空的依赖环境 `c` 偷偷抹掉：

```flurry
-- ❌ 编译期直接报错。
-- 当 c 为非空环境变量时，返回裸 `dyn Reading` 意味着丢弃了方法执行所必需的环境 `c`。
fn bad_strip_env(reading: %c dyn Reading) -> dyn Reading
where
    c: Type.vars.Closure
{
    reading
}
```

---

## 动态接口的四条可封装准则（何时无法动态调用？）

除了 `Self` 参数和环境擦除之外，`docs/traits/dynamic-interfaces.md` 还为我们划定了清晰的动态接口边界。在使用 `dyn Trait` 时，请牢记以下四条准则：

1. **`comptime`-only 成员禁止充当运行时分派入口**：
   如果一个 Trait 里包含 `comptime fn marker() -> Integer`，它可以在 `.dyn(...)` 封装之前用来计算关联常量或布局，但调用者**禁止于运行时通过 `dyn` 虚表去发起一次任意的 `comptime` 查询**（因为运行时没有编译器引擎。）。
2. **未知的关联类型禁用 `Any` 糊弄过去**：
   动态接口调用的方法如果涉及关联项（如 `assoc Output: Type`），该关联项必须已被具体固定、通过已有依赖交付，或者随隐藏主体一同封闭。编译器免将未知的关联类型偷偷替换成 `Any` 来掩盖调用表示缺失。
3. **带泛型子句参数的方法需在边界兑现实例**：
   无法只在运行时虚表里放一个普通函数指针，就承诺支持未来任意未知的类型参数调用。
4. **封装环境跨边界后仍须有效**：
   如果动态接口依赖某个外层局部环境对象，当那个外层对象生命周期结束失效后，无法靠“在调用点随便找一个同类型的替代对象”来凑数。

---

## 本篇小结

| 特性 / 场景 | Flurry `dyn Trait` 的处理机制 | 核心安全红线 |
| :--- | :--- | :--- |
| **方法返回 `Self`** | **合法支持。** 在内部构造具体 `Self` 后，连同实现证据与环境**重新封闭（Re-enclose）**为对应的动态接口返回 | 禁止将隐藏的具体 `Self` 当作已知裸具体类型交给调用者；若涉及分配须遵守合法分配约定 |
| **方法接收 `Self` 参数** | 要求证明输入与当前接收者属于**同一隐藏具体类型及依赖关系** | 两个表面类型都叫 `dyn Trait` 的值无法证明 `Self` 相同，无证据混传直接报错 |
| **额外运行时环境** | 推导出 **`%c dyn Trait`**（如 `%{space: Space = space} dyn Drop`），随完整值或共享外层交付 | 只有环境 `c` 为空时才消去为两个 `usize` 的裸 `dyn Trait`；严禁把非空环境擦除赋给裸 `dyn` |
| **`comptime` 成员** | 可在封装前参与编译期计算 | 严禁作为 runtime 虚表分派入口在运行时动态调用 |

---

### 下一篇预告

在刚才探讨 `%c dyn Trait` 的时候，我们提到了一个让人非常感兴趣的概念：**“某个 Trait 实现依赖一个外部运行时对象（比如校准器 `Calibration` 或分配器 `Allocator`）”**。

等等。一个 `impl Measure for Sensor` 或者 `impl Measure for Reading`，怎么能依赖一个运行时的校准器对象 `calibration: *Calibration` 呢？
- 如果 `Sensor<calibration>` 的类型参数里已经记录了这个校准器，调用 `sensor.measure()` 时，编译器是怎么从目标类型里**自动恢复（Substitution Inference）**出校准器绑定的？
- 如果 `Reading` 本身只是一个普通结构体，类型里根本没写校准器，调用者又是怎么用 **`reading.as(Measure)<.calibration = first.ref>.measure()`** 显式把校准器环境塞给这次实现调用的？

下一篇 **`108-substitution-inference-and-dependency-delivery.md`**，我们将揭开 Flurry 独步天下的**替换推导（Substitution Inference）与开放实现运行时依赖交付机制**。
