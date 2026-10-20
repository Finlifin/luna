# Flurry 演进手记 (025)：像写 JSON 一样调用函数——扩展调用 fn { k: v } 与匿名结构体属性收集

在 [上一篇 (024)](./024-variadic-tuples-and-direct-supply.md) 中，我们学会了用 `...items: T` 把按位置传入的异构实参收集成一个强类型的**元组（Tuple）**。

但是，随着我们的传感器分析仪功能越来越强大，配置一个采集工作线程或构建一个多层诊断快照时，光靠“位置元组”很快就不够用了。
想象一下，你想构建一个带嵌套结构的插件配置包：
- 顶层有 `limit: 8u32`、`enabled: true`、`label: "worker"`；
- 里面还嵌套了一层子配置 `nested: { value: false }`；
- 甚至有时候，你还想在同一个调用里，既传入一组位置数据 `...inputs()`，又附上几个命名属性 `label: "worker", enabled: true`。

如果只用上一篇的位置元组 `keep_inputs(8u32, true, "worker")`，调用者拿到的结果只能靠 `.0`、`.1` 或按严格顺序解构 `let (limit, enabled, label) = ...` 来读取——一旦字段超过三个，谁还记得第 2 个布尔值是 `enabled` 还是 `auto_retry`？

而如果在传统静态语言（如 Rust、C++ 或 Go）里，你想让每个字段都有名字，你该怎么做？

---

## 一、 传统静态语言的“一次性结构体膨胀症”与动态字典的妥协

在传统静态语言里，只要你想传递一组带字段名的数据，你通常面临两个极端的选择：

- **极端 A（一次性 `struct` 膨胀症）**：哪怕某个配置组合只在两个函数之间传递一次，你也必须先跑到文件顶上，手写一个具名的 `struct WorkerConfig { limit: u32, enabled: bool, label: str }`；如果有嵌套，还得再写一个 `struct NestedConfig { value: bool }`。久而久之，代码库里到处都是 `FooOptions`、`BarParams`、`BazConfig` 这种只用一次的样板结构体定义。
- **极端 B（退化为动态字符串字典 `Map<str, Any>`）**：像写 Python 的 `**kwargs` 一样，把键值对塞进哈希表。结果不仅付出了运行时哈希查找和堆分配的代价，而且拼错了键名（把 `enabled` 写成 `enabeld`）或搞错了值类型，编译器完全是个睁眼瞎，全留到线上运行时去崩溃。

让我们看看在第一个示例中，为了构造和读取一个两层嵌套的配置，我们得写多少样板代码：

```flurry
use std.prelude.*;

-- 为了传一组带名字的嵌套属性，被迫提前定义一堆具名 struct
struct InnerOptionsV0 {
    value: bool,
}

struct WorkerOptionsV0 {
    limit: u32,
    enabled: bool,
    label: str,
    nested: InnerOptionsV0,
}

fn inspect_options(options: WorkerOptionsV0) -> bool {
    -- 读取时倒是可以点出字段名，但前提是你在上面手写了两个 struct 定义。
    options.enabled and not options.nested.value
}

test {
    let opts = WorkerOptionsV0 {
        limit: 8u32,
        enabled: true,
        label: "worker",
        nested: InnerOptionsV0 { value: false },
    }
    println("{}", inspect_options(opts));
}
```

看着上面那两个只为了打包一次参数而存在的 `InnerOptionsV0` 和 `WorkerOptionsV0`，你不禁要问：
> **“位置变参可以把散装的位置输入直接收集成元组 `(u32, bool, str)`；难道我们就不能把花括号里的 `key: value` 属性输入，直接收集成一个保留所有字段名和精确类型的匿名结构体吗？”**

在 Flurry 中，答案不仅是“能”，而且它的语法优雅得就像在写原生 JSON。

---

## 二、 扩展调用 `expr { k: v }` 与匿名结构体属性收集

首先，让我们揭开 Flurry 语法表（`docs/grammar/syntax`）里一个极其重要的概念：
在 Flurry 中，当你写下 `Foo { a: 1, b: 2 }` 时，它不仅仅是结构体字面量——在语法层面上，任何 `expr { ... }` 都被称为**扩展调用（Extended Application）**。

```text
  普通调用 (Application):           expr ( argument* )
  扩展调用 (Extended Application):  expr { (property | extend_arg | expr)* }
                                            ▲
                                     property 即 `id : expr`（注意是冒号 `:`。）
```

当被调用的函数声明了收集参数 **`...properties: T`**（在收集属性输入时称为**拓展变参**）时，你在花括号 `{ ... }` 里写下的所有 `key: value` 属性，都会被默认收集为一个**强类型的匿名结构体（Anonymous Struct / Record）**。

下面对比上一篇的“位置收集”与本篇的“属性收集”：

```text
  位置收集（圆括号 + 位置表达式）：
  keep_inputs( 8u32, true, "worker" )
        │
        ▼ 收集为【元组 (Tuple)】—— 按位置编号对齐
  ┌──────────┬──────────┬──────────┐
  │ 0: u32   │ 1: bool  │ 2: str   │
  └──────────┴──────────┴──────────┘

  ───────────────────────────────────────────────────────────────────

  属性收集（花括号扩展调用 + `id: expr` 属性）：
  keep_properties { limit: 8u32, enabled: true, label: "worker" }
        │
        ▼ 收集为【匿名结构体 (Anonymous Struct)】—— 保留字段名与各自精确类型。
  ┌──────────────────┬───────────────────┬─────────────────────┐
  │ .limit : u32 (8) │ .enabled : bool   │ .label : str        │
  └──────────────────┴───────────────────┴─────────────────────┘
     ▲ 支持直接点号读取：`config.limit`、`config.enabled`
     ▲ 支持直接完整供应：`keep_properties(.properties = config)`
```

> **🔍 语法小贴士：分清调用处的 `.id = expr` 与扩展调用里的 `id: expr`。**
> 这是一个新手非常容易混淆、但一旦看懂就无比清晰的边界（详见 `docs/functions/parameters.md`）：
> - **`.id = expr`（点号 + 等号，`named_arg`）**：是对函数签名里某个特定形参（如 optional `.limit`、变参 `.properties`）做**显式命名供应**。
> - **`id: expr`（无点号 + 冒号，`property`）**：写在扩展调用 `fn { ... }` 里面，表示向拓展变参收集器输入一个**名为 `id` 的结构体字段属性**。
> 如果你手里已经有一个收集好的匿名结构体 `collected`，想把它整体直接塞给 `keep_properties(...properties: T)` 而不再重新收集，用的依然是 **`.properties = collected`**。

让我们写出 改进版本，看看用四行代码定义的 `keep_properties` 是如何凭空长出一整套类型安全的配置系统的：

```flurry
use std.prelude.*;

-- 拓展变参：将扩展调用 { k: v } 中的属性收集为匿名结构体 T。
fn keep_properties(...properties: T) -> T
where T
{
    properties
}

test {
    -- 1. 像写 JSON 一样用花括号扩展调用收集不同种类的字段。
    let config = keep_properties {
        limit: 8u32,
        enabled: true,
        label: "worker",
    }

    -- 每个字段都保留了各自的名字和精确类型（u32、bool、str），绝不是转换成同一种元素类型。
    println("{} {} {}", config.limit, config.enabled, config.label);

    -- 2. 用 `.properties = ...` 直接供应整个匿名结构体包。
    let supplied = keep_properties(.properties = config);
    println("supplied limit: {}", supplied.limit);
}
```

运行输出：

```text
8 true worker
supplied limit: 8
```

没有提前手写任何 `struct Config`。但编译器在编译期百分百知道 `config` 有 `.limit`（`u32`）、`.enabled`（`bool`）和 `.label`（`str`）三个字段。

---

## 三、 嵌套匿名结构体模式解构，以及位置展开 `...expr` 与属性交错

现在，我们把难度再往上提两个台阶：

### 1. 用模式匹配直接解构嵌套的匿名结构体（顺序无关）

既然 `keep_properties { ... }` 返回的是真正的匿名结构体，那么除了用 `config.limit` 点号读取之外，我们能不能像拆解普通具名 `struct` 一样，用 **`let { ... } = options;`** 模式匹配一次性把嵌套字段全拆出来？

完全可以。（详见 `docs/control-flow/pattern-matching/index.md` §匿名结构体的字段模式）：
- 模式中的字段书写顺序**不必与构造时的顺序相同**；
- 不关心的字段可以直接省略，不参与匹配；
- 支持任意深度的嵌套解构，例如 **`let { nested: { value }, enabled } = options;`**。

### 2. 在同一个扩展调用里混用“位置展开 `...expr`”与“命名属性 `k: v`”

假设我们的传感器任务收集函数 `collect` 签名里同时有两个变参入口：
- 第一个入口收集位置元组：`...items: (u32, bool)`
- 第二个入口收集命名属性：`...properties: T`

当调用方写下这样一个交错的扩展调用时：

```flurry
collect {
    label: "worker",
    ...inputs(),
    enabled: true,
}
```

这里既有 `label: "worker"` 和 `enabled: true` 两个属性（用冒号 `:`），中间还夹着一个位置元组展开 `...inputs()`。
编译器是怎么分派它们、又是按什么顺序求值的？

下面通过图示看穿它的分派与求值流水线：

```text
  扩展调用：collect { label: make_label(), ...inputs(), enabled: make_flag() }
  函数签名：fn collect(...items: (u32, bool), ...properties: T) -> ((u32, bool), T)

  [第 1 步：严格按源码从左到右求值所有输入（各求值一次。）]
     1. 求值 `make_label()` ──► 得到 "worker"
     2. 求值 `inputs()`     ──► 得到元组 (7u32, true)
     3. 求值 `make_flag()`  ──► 得到 true
     (核心规则：若其中任何一个表达式提前返回或报错，后续表达式与 collect 主体绝不执行。)
        │
        ▼
  [第 2 步：按各自形态分流路由，互不干扰。]
     ├──► 命名属性 (`label: ...`, `enabled: ...`)
     │       └──► 送入属性收集器 `...properties` ──► 组成匿名结构体 { label: "worker", enabled: true }
     │
     └──► 位置展开 (`...inputs()`)
             └──► 送入位置收集器 `...items`      ──► 按元组次序交付 (7u32, true)
                  (注意：`...inputs()` 仍供应位置输入，绝不会被误解释成属性包。)
```

把“嵌套匿名结构体模式解构”和“位置展开与属性交错”结合起来，我们写出功能完备的 进阶版本：

```flurry
use std.prelude.*;

fn build(...properties: T) -> T
where T
{
    properties
}

fn collect(...items: (u32, bool), ...properties: T) -> ((u32, bool), T)
where T
{
    (items, properties)
}

fn inputs() -> (u32, bool) {
    println("2. evaluating inputs()");
    (7u32, true)
}

fn make_label() -> str {
    println("1. evaluating label");
    "worker"
}

fn make_enabled() -> bool {
    println("3. evaluating enabled");
    true
}

test {
    -- A. 构造嵌套匿名结构体，并用字段模式（乱序。）直接解构
    let options = build {
        enabled: true,
        nested: build { value: false },
    }
    -- 注意：构造时 enabled 在前、nested 在后；
    -- 模式解构时我们故意把 nested 写在前、enabled 写在后，完全合法。
    let { nested: { value }, enabled } = options;
    println("destructured: enabled={}, value={}", enabled, value);

    -- B. 扩展调用中位置展开 `...inputs()` 与命名属性交错
    let ((count, ready), props) = collect {
        label: make_label(),
        ...inputs(),
        enabled: make_enabled(),
    }
    println("collected: count={}, ready={}, label={}, enabled={}",
        count, ready, props.label, props.enabled);
}
```

看看运行输出，求值次序与解构结果一目了然：

```text
destructured: enabled=true, value=false
1. evaluating label
2. evaluating inputs()
3. evaluating enabled
collected: count=7, ready=true, label=worker, enabled=true
```

### 编译器防呆：试图解构不存在的字段或重复字段？编译期拒绝

虽然匿名结构体不用你手写 `struct` 声明，但它的类型严格程度和具名 `struct` 毫无二致。如果你在模式解构或点号访问时写了一个根本不存在的字段名（比如手滑把 `enabled` 写成了 `enabeld`），或者在同一个模式里重复匹配同一个字段，编译器会在编译期立刻拒绝：

```flurry
fn build(...properties: T) -> T
where T
{
    properties
}

-- ❌ 编译期直接拒绝。
-- 错误：匿名结构体 options 中只包含 `enabled` 与 `limit`，不存在未知字段 `enabeld`。
fn invalid_anonymous_field() -> bool {
    let options = build { enabled: true, limit: 8u32 }
    let { enabeld } = options;
    enabeld
}
```

既有 JSON 般随手书写的自由度，又有 100% 静态类型检查的铁壁防护。

---

## 本篇小结

对比位置变参与拓展变参（属性收集），我们现在拥有了一套完整的结构化参数收集工具箱：

| 特性维度 | 位置收集 (`fn(a, b)`) | 属性收集 (`fn { k1: 改进版, k2: 进阶版 }`) |
| :--- | :--- | :--- |
| **调用处形态** | 普通调用或展开 `...tuple_expr` | 扩展调用 `{ id: expr, ... }` |
| **默认收集结果** | **元组（Tuple）**，按位置顺序对齐 | **匿名结构体（Anonymous Struct）**，保留字段名与各自类型 |
| **直接完整供应** | `.items = my_tuple` | `.properties = my_record` |
| **读取与拆解方式** | `let (a, b) = items;` | 字段点号 `props.k1` 或顺序无关解构 `let { k2, k1 } = props;` |
| **混合交错规则** | 在 `fn { label: "x", ...inputs(), enabled: true }` 中，严格按源码顺序求值，再分别路由到位置入口与属性入口 |

### 下一篇预告

到现在为止，我们已经掌握了五种参数类别中的四种：**锚定参数**、**可选参数（`.id = expr`）**、**位置变参（`...items`）** 和 **拓展变参（`...properties`）**。

只差最后一块拼图了。
在为传感器分析仪编写业务代码时，我们发现了两个新的美学与核心问题：

1. **问题一（上下文传递太啰嗦）**：分析仪的很多算子都需要一个校准偏移量 `bias: u32` 或编译期模式开关 `use_first: bool`。大部分时候我们希望编译器能通过参数 elaboration 协议自动供应这个上下文，只有在极少数特殊测试里才用 `.bias = 3` 手动覆盖它。
2. **问题二（回调闭包的小尾巴与命名噪音）**：当我们写一个带回调函数的比较或过滤接口时，比如：
   - `compare_pair(2, 3, |first, second| first < second)` —— 只为了写一个 `<` 号，居然要写一遍 `|first, second|` 参数声明。能不能直接用占位符写成 **`compare_pair(2, 3, _0 < _1)`**？
   - 当回调函数是一大段多行代码块时，写在圆括号里面 `quote_price(80, |price| { ... })` 会在最后留下一串繁琐的 `})` 小尾巴。能不能像原生控制流一样，写在圆括号外面的尾随块里：**`quote_price(80) do |price| { ... }`**？

在下一篇 **《Flurry 演进手记 (026)：上下文自动注入与尾随块——`implicit` 参数、`post do` 与 `tactic lambda`》** 中，我们将揭开参数 Elaboration 的终极机制，让你的高阶函数调用进化得像内置语法一样丝滑。
