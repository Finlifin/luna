# Flurry 演进手记 (025)：像写 JSON 一样调用函数——扩展调用 fn { k: v } 与匿名结构体属性收集

在 [上一篇 (024)](./024-variadic-tuples-and-direct-supply.md) 中，我们学会了用 `...items: T` 把按位置传入的异构实参收集成一个强类型的**元组（Tuple）**。

但是，随着我们的传感器分析仪功能越来越强大，配置一个采集工作线程或构建一个多层诊断快照时，光靠“位置元组”很快就捉襟见肘了。
想象一下，你想构建一个带嵌套结构的插件配置包：
- 顶层有 `limit: 8u32`、`enabled: true`、`label: "worker"`；
- 里面还嵌套了一层子配置 `nested: { value: false }`；
- 甚至有时候，你还想在同一个调用里，既传入一组位置数据 `...inputs()`，又附上几个命名属性 `label: "worker", enabled: true`。

如果只用上一篇的位置元组 `keep_inputs(8u32, true, "worker")`，调用者拿到的结果只能靠 `.0`、`.1` 或按严格顺序解构 `let (limit, enabled, label) = ...` 来读取——一旦字段超过三个，谁还记得第 2 个布尔值是 `enabled` 还是 `auto_retry`？

而如果在传统静态语言（如 Rust、C++ 或 Go）里，你想让每个字段都有名字，你该怎么做？

---

## 一、 传统静态语言的“一次性结构体膨胀症”与动态字典的妥协

在传统静态语言里，只要你想传递一组带字段名的数据，你通常面临两种典型的取舍：

- **方案 A（一次性 `struct` 膨胀症）**：哪怕某个配置组合只在两个函数之间传递一次，你也必须先跑到文件顶上，手写一个具名的 `struct WorkerConfig { limit: u32, enabled: bool, label: str }`；如果有嵌套，还得再写一个 `struct NestedConfig { value: bool }`。久而久之，代码库里到处都是 `FooOptions`、`BarParams`、`BazConfig` 这种只用一次的样板结构体定义。
- **方案 B（退化为动态字符串字典 `Map<str, Any>`）**：像写 Python 的 `**kwargs` 一样，把键值对塞进哈希表。结果非但付出了运行时哈希查找和堆分配的代价，而且拼错了键名（把 `enabled` 写成 `enabeld`）或搞错了值类型，编译器无从静态察觉，全留到线上运行时去崩溃。

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
> **“位置变参可以把散装的位置输入直接收集成元组 `Tuple<u32, bool, str>`；难道我们就禁止将花括号里的 `key: value` 属性输入，直接收集成一个保留所有字段名和精确类型的匿名结构体吗？”**

在 Flurry 中，答案非但是“能”，而且它的语法优雅得就像在写原生 JSON。

---

## 二、 扩展调用 `expr { k: v }` 与 PTC 约束下的字段直接访问

首先，让我们看 Flurry 语法表（`docs/grammar/syntax`）中的两项基础构造：
1. **匿名结构体字面量（Object）**：`object -> { (property | expr)* }`，例如 `{ value: false }` 或 `{ display: "flex", padding: "10px" }`；
2. **扩展调用（Extended Application）**：`extended_application -> expr { (property | extend_arg | expr)* }`，其中 `property` 为 `id: expr`（使用冒号 `:`）。

```text
  普通调用 (Application):           expr ( argument* )
  扩展调用 (Extended Application):  expr { (property | extend_arg | expr)* }
                                            ▲
                                     property 即 `id : expr`（使用冒号 `:`）
```

当被调函数声明了拓展变参 **`...properties: T`** 时，调用处在花括号 `{ ... }` 中书写的各个 `key: value` 属性会被收集为一个**强类型的匿名结构体（Anonymous Struct / Record）**，并作为 `properties` 传入函数体内。

这里有一个关键问题：
在普通运行时函数 `fn` 中，函数体采用 **ordinary** normalization 检查——此时泛型参数 `T` 尚未代入具体调用处的类型。如果函数签名只写一个无约束的 `where T`，编译器在定义期无从知晓 `properties` 究竟包含哪些字段，自然禁止直接对未知 `T` 编写 `properties.limit`。

那么，被调函数应当如何在函数体内直接读取收集得来的字段？
答案是：**配合 PTC（命题类型约束）的成员约束 `properties:: field : Type`**（既可写在 `where requires` 子句中约束必填属性，也可写在 `inline if` 条件中按需读取可选属性）：

```text
  属性收集与函数内 PTC 约束访问：
  configure_worker { limit: 8u32, enabled: true, label: "worker", retry: 3u32 }
        │
        ▼ 收集为【匿名结构体 (Anonymous Struct)】传入 `...properties: T`
  ┌──────────────────┬───────────────────┬───────────────────┬──────────────────┐
  │ .limit : u32 (8) │ .enabled : bool   │ .label : str      │ .retry : u32 (3) │
  └──────────────────┴───────────────────┴───────────────────┴──────────────────┘
        │
        ├──► 1. 必填属性：在签名 `where T, requires properties:: limit : u32 ...` 声明后，
        │       函数体内直接访问 `properties.limit`、`properties.enabled`、`properties.label`
        │
        └──► 2. 可选属性：在函数体内通过 `inline if properties:: retry : u32 { ... }` 判断，
                进入分支后由 Occurrence Typing 获得字段证据，直接访问 `properties.retry`
```

> **🔍 语法小贴士：分清调用处的 `.id = expr` 与扩展调用里的 `id: expr`**
> 这两组语法职责分明（详见 `docs/functions/parameters.md`）：
> - **`.id = expr`（点号 + 等号，`named_arg`）**：对函数签名中某个形参（如可选参数 `.limit`、拓展变参 `.properties`）做**显式命名供应**。
> - **`id: expr`（无点号 + 冒号，`property`）**：写在扩展调用 `fn { ... }` 内部，向拓展变参收集器输入一个**名为 `id` 的结构体字段属性**。
> 如果手里已经有一个构造好的匿名结构体 `preset`，想将它整体直接传给 `configure_worker(...properties: T)` 而跳过逐项收集，只需使用 **`.properties = preset`**。

下面我们写出改进版本，演示如何在 `configure_worker` 内部通过 `requires` 与 `inline if` 直接访问收集得来的属性字段：

```flurry
use std.prelude.*;

-- 拓展变参 + PTC 字段约束：
-- 1. `requires` 声明调用方必须提供的属性字段与类型；
-- 2. 函数体内还可结合 `inline if properties:: retry : u32` 检查可选属性字段。
fn configure_worker(...properties: T) -> u32
where T,
    requires properties:: limit : u32
        and properties:: enabled : bool
        and properties:: label : str
{
    println("{} {} {}", properties.limit, properties.enabled, properties.label);

    -- 若调用方额外提供了 `retry: u32` 属性，则在编译期选入此分支并直接访问 `properties.retry`
    inline if properties:: retry : u32 {
        println("optional retry={}", properties.retry);
    }

    if properties.enabled {
        properties.limit
    } else {
        0u32
    }
}

test {
    -- 1. 像写 JSON 一样用花括号扩展调用传入各异类型的命名属性（含可选属性 retry）
    let active_limit = configure_worker {
        limit: 8u32,
        enabled: true,
        label: "worker",
        retry: 3u32,
    };
    println("active limit: {}", active_limit);

    -- 2. 构造匿名结构体字面量，并用 `.properties = ...` 直接供应整个属性包
    let preset = {
        limit: 16u32,
        enabled: true,
        label: "standby",
    };
    let supplied_limit = configure_worker(.properties = preset);
    println("supplied limit: {}", supplied_limit);
}
```

运行输出：

```text
8 true worker
optional retry=3
active limit: 8
16 true standby
supplied limit: 16
```

全程无需提前声明一次性具名 `struct`，被调函数在编译期即可安全、直接地读取 `properties.limit`、`properties.enabled`、`properties.label` 以及可选的 `properties.retry`。

---

## 三、 嵌套匿名结构体解构，以及位置展开 `...expr` 与属性交错

现在，我们进一步考察两个常见的组合场景：

### 1. 直接传递嵌套匿名结构体 `{ ... }` 并在函数内模式解构

当配置属性带有多层嵌套结构（如 `nested: { value: false }`）时，内层直接书写匿名结构体字面量 `{ value: false }` 即可，无需额外套用辅助构造函数。
在接收 `...properties: T` 的函数中，只要通过 PTC 约束好嵌套字段类型，除了使用 `properties.nested.value` 点号读取外，还可以直接用 **`let { ... } = properties;`** 模式匹配一次性拆出嵌套字段（详见 `docs/control-flow/pattern-matching/index.md` §匿名结构体的字段模式）：
- 模式中的字段书写顺序**无须与调用处构造顺序一致**；
- 未关心的其余字段可以直接省略；
- 支持多层嵌套解构，例如 **`let { nested: { value }, enabled } = properties;`**。

### 2. 在同一个扩展调用里混用“位置展开 `...expr`”与“命名属性 `k: v`”

假设任务派发函数 `dispatch_worker` 的签名同时声明了两个变参入口：
- 第一个入口收集位置元组：`...items: Tuple<u32, bool>`
- 第二个入口收集命名属性：`...properties: T`

当调用方写下这样一个交错的扩展调用时：

```flurry
dispatch_worker {
    label: make_label(),
    ...inputs(),
    enabled: make_enabled(),
}
```

这里既有 `label: ...` 和 `enabled: ...` 两个属性（用冒号 `:`），中间还夹着一个位置元组展开 `...inputs()`。
编译器的求值与分派流水线如下：

```text
  扩展调用：dispatch_worker { label: make_label(), ...inputs(), enabled: make_enabled() }
  函数签名：fn dispatch_worker(...items: Tuple<u32, bool>, ...properties: T) -> u32
            where T, requires properties:: label : str and properties:: enabled : bool

  [第 1 步：严格按源码从左到右求值所有输入（各求值一次）]
     1. 求值 `make_label()`   ──► 得到 "worker"
     2. 求值 `inputs()`       ──► 得到元组 (7u32, true)
     3. 求值 `make_enabled()` ──► 得到 true
     (核心规则：若其中任一表达式提前返回或报错，后续表达式与被调函数主体均跳过执行)
        │
        ▼
  [第 2 步：按各自语法形态分流路由，互不干扰]
     ├──► 命名属性 (`label: ...`, `enabled: ...`)
     │       └──► 送入属性收集器 `...properties` ──► 组成匿名结构体传入函数内直接访问
     │
     └──► 位置展开 (`...inputs()`)
             └──► 送入位置收集器 `...items`      ──► 按元组次序交付 (7u32, true)
```

把“嵌套匿名结构体解构”和“位置展开与属性交错”结合起来，代码实现如下：

```flurry
use std.prelude.*;

-- A. 在 PTC 约束下直接解构嵌套匿名结构体属性
fn inspect_nested_options(...properties: T) -> bool
where T, N,
    requires properties:: enabled : bool
        and properties:: nested : N
        and N:: value : bool
{
    -- 构造时 enabled 在前、nested 在后；
    -- 解构时把 nested 写在前、enabled 写在后同样合法
    let { nested: { value }, enabled } = properties;
    println("destructured: enabled={}, value={}", enabled, value);
    enabled and not value
}

-- B. 同时接收位置收集 `...items` 与属性收集 `...properties`，并在函数内直接使用
fn dispatch_worker(...items: Tuple<u32, bool>, ...properties: T) -> u32
where T,
    requires properties:: label : str
        and properties:: enabled : bool
{
    let (count, ready) = items;
    println("collected: count={}, ready={}, label={}, enabled={}",
        count, ready, properties.label, properties.enabled);
    if ready and properties.enabled {
        count
    } else {
        0u32
    }
}

fn inputs() -> Tuple<u32, bool> {
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
    -- A. 内层直接书写匿名结构体字面量 `{ value: false }`
    let ok = inspect_nested_options {
        enabled: true,
        nested: { value: false },
    };
    println("nested check: {}", ok);

    -- B. 扩展调用中位置展开 `...inputs()` 与命名属性交错
    let active_count = dispatch_worker {
        label: make_label(),
        ...inputs(),
        enabled: make_enabled(),
    };
    println("active count: {}", active_count);
}
```

运行输出清晰展示了求值次序与字段读取结果：

```text
destructured: enabled=true, value=false
nested check: true
1. evaluating label
2. evaluating inputs()
3. evaluating enabled
collected: count=7, ready=true, label=worker, enabled=true
active count: 7
```

### 编译器防呆：缺少 PTC 字段约束或调用方拼错属性名？编译期直接拦截

匿名结构体的静态检查严格程度与具名 `struct` 完全一致：
1. **定义端检查（Ordinary Normalization）**：如果函数既未在 `where requires` 中声明 `properties:: enabeld : bool`，也未通过 `inline if properties:: enabeld : bool` 守卫，就在函数体里直接读取 `properties.enabeld`，编译器会在定义检查阶段立刻报错；
2. **调用端检查（Instantiation）**：如果函数声明了 `requires properties:: enabled : bool`，而调用方手滑把 `enabled` 拼成了 `enabeld`，编译器会在调用处报出 PTC 约束未满足错误：

```flurry
fn check_enabled(...properties: T) -> bool
where T,
    requires properties:: enabled : bool
{
    properties.enabled
}

-- ❌ 编译期直接报错：
-- 调用处传入的匿名结构体只包含 `enabeld` 与 `limit`，未能满足 `requires properties:: enabled : bool`。
fn invalid_property_call() -> bool {
    check_enabled { enabeld: true, limit: 8u32 }
}
```

既有 JSON 般随手书写的表现力，又具备 100% 静态类型检查的保障。

---

## 本篇小结

对比位置变参与拓展变参（属性收集），我们现在拥有了一套完整的结构化参数收集工具箱：

| 特性维度 | 位置收集 (`fn(a, b)`) | 属性收集 (`fn { k1: v1, k2: v2 }`) |
| :--- | :--- | :--- |
| **调用处形态** | 普通调用或展开 `...tuple_expr` | 扩展调用 `{ id: expr, ... }`（嵌套对象直接写 `{ ... }`） |
| **默认收集结果** | **元组（Tuple）**，按位置顺序对齐 | **匿名结构体（Anonymous Struct）**，保留字段名与各自类型 |
| **直接完整供应** | `.items = my_tuple` | `.properties = my_record` |
| **函数内读取方式** | `let (a, b) = items;` | 配合 PTC 约束（`requires properties:: k1 : T1` 或 `inline if properties:: k1 : T1`）直接点号访问 `properties.k1` 或解构 `let { k2, k1 } = properties;` |
| **混合交错规则** | 在 `fn { label: "x", ...inputs(), enabled: true }` 中，严格按源码顺序求值，再分别路由到位置入口与属性入口 |

### 下一篇预告

到现在为止，我们已经掌握了五种参数类别中的四种：**锚定参数**、**可选参数（`.id = expr`）**、**位置变参（`...items`）** 和 **拓展变参（`...properties`）**。

只差最后一块拼图了。
在为传感器分析仪编写业务代码时，我们发现了两个新的美学与核心问题：

1. **问题一（上下文传递太啰嗦）**：分析仪的很多算子都需要一个校准偏移量 `bias: u32` 或编译期模式开关 `use_first: bool`。大部分时候我们希望编译器能通过参数 elaboration 协议自动供应这个上下文，只有在少数特殊测试里才用 `.bias = 3` 手动覆盖它。
2. **问题二（回调闭包的小尾巴与命名噪音）**：当我们写一个带回调函数的比较或过滤接口时，比如：
   - `compare_pair(2, 3, |first, second| first < second)` —— 只为了写一个 `<` 号，却要写一遍 `|first, second|` 参数声明。能否直接用占位符写成 **`compare_pair(2, 3, _0 < _1)`**？
   - 当回调函数是一大段多行代码块时，写在圆括号里面 `quote_price(80, |price| { ... })` 会在最后留下一串繁琐的 `})` 小尾巴。能否像原生控制流一样，写在圆括号外面的尾随块里：**`quote_price(80) do |price| { ... }`**？

在下一篇 **《Flurry 演进手记 (026)：上下文自动注入与尾随块——`implicit` 参数、`post do` 与 `tactic lambda`》** 中，我们将揭开参数 Elaboration 的核心机制，让你的高阶函数调用进化得像内置语法一样丝滑。
