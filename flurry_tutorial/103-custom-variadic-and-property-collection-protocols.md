# Flurry 演进手记 (103)：定制你自己的变参与属性收集器——参数收集协议与 Elaboration 接入

> **系列导读**：在上一期（102）中，我们纵览了算术、逻辑非、字面量后缀 `32px` 以及错误处理 `.use(handler)` 背后的 Trait 语言协议。今天，我们将目光投向函数调用发生前的最后一公里——**参数展开与接入（Parameter Elaboration）**。在第二卷中，我们曾体验过位置变参 `...items`、扩展属性调用 `func { width: 2, height: 3 }`、隐式参数 `implicit` 以及带占位符 `_0` 的简写闭包。当这些灵活的调用语法与第五卷的 Trait 协议相遇时，编译器是如何把零散的实参组装成目标类型的？为什么无论参数怎么穿插收集，Flurry 都能保证**源码求值顺序不被篡改**？让我们一探究竟。

---

## 一、从“固定传参”到“声明式收集”：传统方案的局限

在构建遥测与张量引擎的配置接口时，我们经常希望写出干净利落的声明式调用：
1. **位置变参（Positional Variadics）**：传入任意多个不同类型的遥测采样值 `keep_inputs(7u32, true, "ready")`，或者传入三维张量的形状 `volume(2, 3, 4)`；
2. **属性变参（Extended Property Variadics）**：用花括号扩展调用语法 `area_of_rect { width: 2u32, height: 3u32 }` 直接收集一组命名配置项，并在函数体内通过 PTC 字段约束直接读取；
3. **超简写回调（Tactic Lambda & Post `do`）**：在过滤或比较传感器读数时，直接写 `compare_pair(2, 3, _0 < _1)`。

如果在传统的 传统语言里：
- 变参要么像 C 语言的 `va_list` 一样完全丧失类型安全，要么像切片 `&[T]` 一样强迫所有参数必须是同一种类型；
- 如果想传命名属性，调用方必须先手写一堆临时的 `ConfigBuilder::new().width(2).height(3).build()` 样板代码。

而在 Flurry 中，这一切都由编译器的 **参数 Elaboration 流水线** 配合 **参数收集与供应协议** 自动完成。

---

## 二、位置收集、属性收集与显式整包供应（`.name = expr`）

让我们先看 Flurry 默认提供的两套异构收集能力，以及它们与 PTC 字段约束、显式命名供应（`.name = expr`）的精确互动：

```flurry
-- 1. 位置变参收集：默认收集为保留逐项精确类型的 Tuple。
fn keep_inputs(...items: T) -> T
where T
{
    items
}

-- 2. 扩展属性收集：在 expr { key: val, ... } 调用中，默认收集为匿名结构体（Record），
--    配合 PTC 字段约束 `properties:: field : Type` 在函数内直接访问收集得来的字段。
fn area_of_rect(...properties: T) -> u32
where T,
    requires properties:: width : u32
        and properties:: height : u32
{
    inline if properties:: scale : u32 {
        properties.width * properties.height * properties.scale
    } else {
        properties.width * properties.height
    }
}

test {
    -- A. 异构位置收集：u32、bool 和 str 各自保留精确类型，无需转成同一类型。
    let (number, enabled, label) = keep_inputs(7u32, true, "ready");
    println("{} {} {}", number, enabled, label); -- 输出：7 true ready

    -- B. 异构属性收集：冒号属性自动组装为匿名结构体，在被调函数内直接访问 .width 与 .height。
    let base_area = area_of_rect {
        width: 2u32,
        height: 3u32,
    };

    -- C. 显式整包供应（.name = expr）：
    -- 直接把已经构造好的匿名结构体通过 .properties = preset 传给收集参数，
    -- 编译器直接供应整个包，【跳过收集协议，亦免于多包一层外层结构体】。
    let preset = {
        width: 2u32,
        height: 3u32,
        scale: 4u32,
    };
    let scaled_area = area_of_rect(.properties = preset);
    println("{} {}", base_area, scaled_area); -- 输出：6 24
}
```

### ⚠️ 严防混淆：唯一供应源核心规则（Single Supply Source Rule）

在 Flurry 编译器的 Elaboration 状态机中，每一个形参最多只能有一个最终供应来源（直接显式供应 `.name = expr` **或** 逐项收集，二者严格互斥）。如果你试图既传位置散件，又用 `.shape = ...` 传整包，编译器会静态拦截：

```flurry
fn volume(...shape: Tuple<u32, u32, u32>) -> u32 {
    let (width, height, depth) = shape;
    width * height * depth
}

fn invalid_volume_call() -> void {
    -- ❌ 编译期直接报错。
    -- 收集参数 shape 已经收到了位置输入 (2, 3, 4)，
    -- 严格禁止同时再通过 .shape = (5, 6, 7) 重复供应整包。
    volume(2u32, 3u32, 4u32, .shape = (5u32, 6u32, 7u32));
}
```

---

## 三、深度揭秘：穿插展开与 Trait 收集协议的“局部边界核心规则”

在 Flurry 中，`implicit` 参数、位置变参 `...items` 和拓展属性变参 `...properties` 都允许通过 **Trait 协议** 来定制它们的 Elaboration 行为（例如将变参或属性收集到自定义的定长容器或配置校验器中，或者为 `implicit` 参数提供自动环境供应）。

但这里有一个尤为关键的设计问题：**如果允许用户通过 Trait 协议定制参数收集，是否会像宏一样把其他参数的求值顺序搞乱，或者偷偷把某个实参执行两次？**

**并非如此。** Flurry 为所有参数协议划定了难以逾越的 **“局部边界核心规则（Local Boundary of Parameter Protocols）”**：

```text
================================================================================
              Flurry 参数 Elaboration 与收集协议的三大安全规则
================================================================================
  1. 【专属领地原则】：每个参数协议只负责如何供应它自己的那个参数入口；
                       无法重写、吞掉或篡改兄弟参数。
  2. 【源码顺序求值】：无论位置输入、展开包 (...expr)、命名实参 (.bias = ...)
                       与属性 (key: val) 如何交错排列，所有运行期生产表达式
                       严格按照【调用处的源码书写顺序】各执行一次。
  3. 【早退精准清理】：若排在前面的参数生产表达式提前返回（return / error），
                       已完成的临时输入按移动与 Drop 规则精准清理一次，
                       排在后面的生产表达式与被调函数本体跳过执行。
================================================================================
```

### 通过 `VariadicCollector` 与 `ExtendedVariadicCollector` 定制收集目标类型

除了默认收集为元组（`Tuple`）或匿名结构体（Record）之外，目标类型还可以通过实现 `VariadicCollector`（针对位置变参 `...args`）与 `ExtendedVariadicCollector`（针对扩展属性变参 `...properties`）来自定义两阶段收集逻辑。
在 `ExtendedVariadicCollector.extended_variadic_collect(comptime Ps: Type, properties: Ps)` 内部，编译器传入的 `properties: Ps` 依然是收集好的匿名结构体，实现者可以直接使用 `inline if properties:: field : Type` 按需提取并消费各个属性字段：

```flurry
trait ExtendedVariadicCollector {
    assoc Tmp: Type;

    fn extended_variadic_collect(comptime Ps: Type, properties: Ps) -> Tuple<Tmp, Self>;

    ^comptime_phase
    assoc TmpComptime: Type;

    ^comptime_phase
    fn extended_variadic_collect_comptime(Ps: Type, properties: Ps) -> Tuple<TmpComptime, Self>;
}

impl ExtendedVariadicCollector for ComponentPropertySet {
    assoc Tmp = void;

    -- Ps 传入的是收集所得的匿名结构体，在函数体内通过 PTC 字段约束按需读取属性
    fn extended_variadic_collect(comptime Ps: Type, properties: Ps) -> Tuple<void, ComponentPropertySet> {
        var prop_set = ComponentPropertySet.new();

        inline if properties:: src : String {
            prop_set.set_src(properties.src);
        }

        inline if properties:: padding : str {
            prop_set.set_padding(properties.padding);
        }

        ((), prop_set)
    }
}
```

### 当变参展开包与命名实参交错时的真实执行流水线

让我们看一段将位置展开包 `...do { ... }` 与命名可选参数 `.bias = ...` 穿插书写的精彩示例：

```flurry
fn first() -> bool {
    println("1. first");
    true
}

fn bias() -> bool {
    println("2. bias");
    true
}

fn last() -> bool {
    println("3. last");
    false
}

fn choose(comptime flag: bool, ...items: Tuple<bool, bool>, .bias: bool = false) -> bool {
    let (a, b) = items;
    inline if flag { a and bias } else { b }
}

test {
    -- 注意看调用处的源码书写顺序：
    -- 先写 items 的前半截 ...do { (first(),) }，
    -- 中间插一个命名实参 .bias = bias()，
    -- 最后再补上 items 的后半截 ...do { (last(),) }。
    let res = choose(true, ...do { (first(),) }, .bias = bias(), ...do { (last(),) });
    println("result: {}", res);
}
```

```text
[ 调用处源码顺序 ]   ...do { (first(),) }  ──►  .bias = bias()  ──►  ...do { (last(),) }
                             │                        │                      │
[ 运行期求值次序 ]     第 1 步：执行 first()   第 2 步：执行 bias()   第 3 步：执行 last()
                             │                        │                      │
                             └───────────┬────────────┼──────────────────────┘
                                         ▼            ▼
[ 最终形参组装 ]             items = (true, false)   bias = true  ──► 进入 choose()。
```

看到了吗？尽管 `first()` 和 `last()` 最终都被归组收集进了同一个形参 `items: Tuple<bool, bool>`，但编译器**不会为了凑齐 `items` 就擅自把 `last()` 提早到 `bias()` 前面执行**。**“源码顺序决定求值先后，目标协议决定归组布局”**——这就是 Flurry 参数 Elaboration 令人安心的确定性。

---

## 四、`lambda` 参数标记、Tactic Lambda（`_0`）与 `post do` 路由

在参数 Elaboration 家族中，还有一位每天都在提升我们编码幸福感的明星成员——形参上的 **`lambda` 标记**。

当你在函数形参前写上 `lambda` 修饰符（如 `lambda compare: fn(u32, u32) -> bool`）时，这个参数位置立刻同时解锁了两项超能力：

### 超能力 1：精准锚定尾随 `do` 块（哪怕它不是最后一个参数）

在许多支持尾随闭包的语法设计中，尾随代码块固定对应函数的**最后一个参数**。而在 Flurry 中，尾随 `do`（`post do`）遵循**“`lambda` 标记优先，无标记时才退回最后一个锚定参数”**的智能路由规则：

```flurry
-- 注意：带 lambda 标记的 inspect 位于第 1 个参数，后面还有 value 和可选参数 .fallback。
fn inspect_with(
    lambda inspect: fn(u32) -> bool,
    value: u32,
    .fallback: bool = false,
) -> bool {
    if value == 0u32 {
        fallback
    } else {
        inspect(value)
    }
}

test {
    -- ✅ post do 自动精准路由到带有 lambda 标记的 inspect 参数。
    -- 括号里的 7u32 则顺延绑定给位置参数 value。
    let accepted = inspect_with(7u32) do |value| value < 10u32;
    let absent = inspect_with(0u32, .fallback = true) do |value| value < 10u32;
    println("{} {}", accepted, absent); -- 输出：true true
}
```

### 超能力 2：开启 Tactic Lambda 下标占位符（`_0`、`_1`）与单次求值保证

在带有 `lambda` 标记的实参位置，你甚至连 `|a, b|` 都省去书写，直接写包含 `_0`、`_1` 下标占位符的表达式（称为 **Tactic Lambda**），编译器会自动根据目标函数签名把它展开为完整的闭包。

更关键的是：**如果在 Tactic Lambda 里写了 `_0 + _0`，外部传入的实参表达式是否会被重复执行两次？**
**并非如此。** 外部实参在进入被调函数前严格只求值一次，`_0 + _0` 只是在闭包体内读取了两次已经绑定好的闭包形参。

```flurry
fn produce() -> u32 {
    println("produce called once!");
    6u32
}

fn apply(value: u32, lambda action: fn(u32) -> u32) -> u32 {
    action(value)
}

fn compare_pair(first: u32, second: u32, lambda compare: fn(u32, u32) -> bool) -> bool {
    compare(first, second)
}

test {
    -- 1. produce() 只打印一次 "produce called once!"，闭包内读取两次 _0 得到 12。
    println("{}", apply(produce(), _0 + _0));

    -- 2. 二元 Tactic Lambda：完整参数表由 compare: fn(u32, u32) -> bool 决定
    let ordered = compare_pair(2u32, 3u32, _0 < _1);
    -- 即使只用了 _0 没用 _1，它依然是合法的二元闭包（未引用第二参数而已）
    let same_first = compare_pair(2u32, 3u32, _0 == _0);

    -- 3. 嵌套 Tactic Lambda：每个 lambda 标记建立自己独立的占位符边界。
    -- 外层 apply 的第二个实参 apply(_0, _0 + _0) 中：
    -- 第一个 _0 属于外层闭包（值为 3），第二个位置处于内层 lambda 边界，其 _0 + _0 属于内层闭包。
    let nested = apply(3u32, apply(_0, _0 + _0));
    println("{} {} {}", ordered, same_first, nested); -- 输出：true true 6
}
```

如果有人在没有 `lambda` 标记的普通函数参数上乱用 `_0`，或者写了超出目标签名参数个数的 `_2`，编译器会立即精准报错：

```flurry
fn apply_plain(value: u32, action: fn(u32) -> u32) -> u32 {
    action(value)
}

fn bad_tactic_calls() -> void {
    -- ❌ 编译期直接报错：apply_plain 的 action 参数没有声明 lambda 标记，无法启用 _0 简写。
    -- let a = apply_plain(5u32, _0 + 1u32);

    -- ❌ 编译期直接报错：compare_pair 的回调只有 2 个参数（合法下标 0, 1），_2 越界。
    -- let b = compare_pair(2u32, 3u32, _2 == 0u32);
}
```

---

## 五、本节核心概念速查表

| 参数机制 / 语法 | 默认行为与协议连接 | 核心安全与求值边界 |
| :--- | :--- | :--- |
| `...items: T` | 位置变参，默认收集为保持各项精确类型的 `Tuple` | 可由协议定制；传 `.items = val` 直接供应整包，不可混传散件与整包 |
| `...props: T` | 拓展属性变参（`f { k: v }`），默认收集为匿名结构体 | 区分冒号属性 `k: v` 与点号命名实参 `.k = v`；暂未涵盖把属性包当位置元组 |
| `implicit p: T` | 隐式参数，由 Elaboration 协议自动供应 | 可在调用处用 `.p = val` 显式覆盖；免于退化为“随便抓一个同名局部变量” |
| `lambda f: fn(...)` | 优先接收尾随 `post do`，并开启 `_0`, `_1` Tactic Lambda | 参数个数由目标签名决定；`_0 + _0` 互异无重求值外部实参；嵌套边界各自独立 |
| **源码求值顺序核心规则** | 收集项与命名项交错时按源码顺序建立准备计划 | 任何参数定制协议只能供应自身入口，**严禁重排、重复或吞噬兄弟实参** |

---

## 六、下一期预告

到这里，我们已经见识了如何定义 Trait、如何为具体类型手写 `impl`、以及如何把 Trait 连接到运算符和参数收集协议上。
但在实际的传感器与张量工程中，我们有成百上千个数据结构（如 `SensorPacket`、`TelemetryHeader`、`CalibrationRecord`）需要实现 `Encode`、`Decode`、`Eq`、`Hash`、`FormatLabel` 等接口。

如果每个结构体都要手写一遍 `impl Encode for MyStruct`，程序员的手指会被样板代码消平。
在其他语言（如 Rust）中，想要写一个 `#[derive(MyTrait)]` 自动派生宏，你必须跳出主语言，专门新建一个独立的 `proc-macro` crate，引入 `syn` 和 `quote` 库，把源代码当成没有类型信息的 `TokenStream` 字符串拼来拼去——写起来痛苦不堪，报错信息更是天书一般。

而在 Flurry 中，**根本无需任何外部过程宏 crate**。你可以直接在普通代码里，用一等公民语法 **`derive fn`** 编写纯语言级、带完整类型检查与编译期结构反射的自动派生器。

下一期 **《Flurry 演进手记 (104)：告别黑盒过程宏派生——用纯语言级 derive fn 编写类型安全的自动派生器》**，我们将亲手写出第一个优雅十分的 `derive fn`。
