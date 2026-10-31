# Flurry 演进手记 (024)：把异构参数打包进元组——位置变参 ...items 与 .items = tuple 完整供应

在 [前两篇 (022 & 023)](./023-why-default-params-require-p3-stability.md) 中，我们用可选参数 `.id: T = expr` 和 `P3` 稳定观测契约，搞定了可省略的配置项。

今天，传感器分析仪的遥测模块给我们出了两道新难题：

1. **难题一（三维空间网格计算）**：我们需要一个计算传感器阵列网格体积的函数 `volume`。平时大家喜欢直接传三个散装的数字 `volume(2, 3, 4)`；但有时候，上游模块返回给我们的已经是一个打包好的三维元组 `let dims = (2u32, 3u32, 4u32);`。我们希望同一个 `volume` 函数，既能接收散装的 `volume(2, 3, 4)`，又能直接接收整包 `dims`。
2. **难题二（异构遥测快照打包）**：我们需要一个通用的快照记录函数 `keep_inputs`，调用方可能传进来 `(7u32, true, "ready")`——注意，这里第 1 个参数是 `u32`，第 2 个是 `bool`，第 3 个是 `str`。甚至下次调用可能传 2 个或 4 个大相径庭类型的参数。我们希望把它们维持原貌地收集起来返回，并且**保留每个位置原本的精确静态类型**。

如果让你用熟悉的语言来写这两个函数，你会遇到什么麻烦？

---

## 一、 在“固定的多参数”与“存在隐患的 `va_list` / 同质数组”之间左右为难

在传统系统语言里，处理可变数量的参数通常只有三种蹩脚的办法：

- **C 语言的 `...` (`va_list`)**：放弃类型安全。编译器根本跳过检查你传了几个参数、是什么类型。一旦格式串和实参类型存在偏差，轻则读出垃圾内存，重则当场崩溃。
- **Java / Go / TypeScript 的同质变参（`...items: T[]`）**：强迫所有可变参数必须是同一种类型 `T`。如果你想传 `(7u32, true, "ready")`，你就只能把 `T` 退化成失去静态类型的动态接口（如 `any` 或 `Object`），非但有装箱开销，取出来时还得挨个做向下转型。
- **手动套括号传元组（`fn keep_inputs(items: T)`）**：逼着所有调用者在每次调用时都多写一层圆括号 `keep_inputs((7u32, true, "ready"))`；或者为了撰写 `volume`，被迫写 `volume_3(w, h, d)` 和 `volume_tuple(shape)` 两个函数。

来看看在没有位置变参收集时，缺乏变参收集时的写法：

```flurry
use std.prelude.*;

-- 为了同时支持散装传参和元组传参，被迫维护两套函数签名
fn volume_xyz(width: u32, height: u32, depth: u32) -> u32 {
    width * height * depth
}

fn volume_tuple(shape: Tuple<u32, u32, u32>) -> u32 {
    let (width, height, depth) = shape;
    width * height * depth
}

-- 如果想接收任意类型的组合，只能强迫调用方在外面手动多包一层元组括号
fn keep_inputs(items: T) -> T
where T
{
    items
}

test {
    let res1 = volume_xyz(2u32, 3u32, 4u32);
    let res2 = volume_tuple((2u32, 3u32, 4u32));

    -- 注意这里的双层括号 `(( ... ))`。少写一层括号就会报参数个数错误。
    let (num, flag, label) = keep_inputs((7u32, true, "ready"));
    println("{} {} | {} {} {}", res1, res2, num, flag, label);
}
```

上面这种写法有两个明显局限：
1. **散装参数与元组打包割裂**：明明 `(width, height, depth)` 和 `Tuple<u32, u32, u32>` 在数据本质上是一回事，却要分成两个函数。
2. **调用处的双层括号噪音**：`keep_inputs((7u32, true, "ready"))` 里的双层括号非常别扭，它暴露了“我其实并非多参数函数，我只是让你手动构造了一个元组传进来”的实现痕迹。

我们能否只写一个函数签名，既能像普通多参数函数那样自然调用，又能在内部把它们当成一个强类型的元组来操作？

---

## 二、 位置变参 `...id: Type` —— 把位置输入收集为精确类型的元组

在 Flurry 中，当你写下前缀带三个点的参数声明 **`...id: Type`** 时，你就定义了一个**位置变参（Variadic Parameter）**（详见 `docs/functions/parameters.md`）。

Flurry 的位置变参有一条简洁清晰的核心设计：
> **默认情况下，位置变参会将调用处传入的多个位置实参，按顺序收集（Collect）为一个元组（Tuple），并以整个收集结果的类型 `Type` 来约束输入。**

这意味着什么？下面来看看它的两种用法：

```text
  用法 A：固定形状的元组变参
  fn volume(...shape: Tuple<u32, u32, u32>) -> u32
               ▲
               │ 编译器看到目标类型是 Tuple<u32, u32, u32>，
               │ 严格要求收集到的元组必须是 3 个 u32。
  调用处：volume(  2u32  ,  3u32  ,  4u32  )
                    │        │        │
                    └───┬────┴────┬───┘
                        ▼ (按顺序收集成元组)
             shape = (2u32, 3u32, 4u32) : Tuple<u32, u32, u32>

  ───────────────────────────────────────────────────────────────────

  用法 B：泛型异构元组变参
  fn keep_inputs(...items: T) -> T where T
                    ▲
                    │ 目标类型是泛型变量 T。
                    │ 无论传几个、分别是什么类型，自动推导 T 为对应的异构元组类型。
  调用处：keep_inputs(  7u32  ,  true  ,  "ready"  )
                         │        │          │
                         └───┬────┴────┬─────┘
                             ▼ (保留每一项的精确类型。)
             items = (7u32, true, "ready") : Tuple<u32, bool, str>
```

看到了吗？在 Flurry 里，`...id: Type` 里的 `: Type` 修饰的**并非“单个元素的共同类型”**，而是**“收集完成后整个元组包的类型”**。

- 当你写 `...shape: Tuple<u32, u32, u32>` 时，`shape` 在函数体内就是一个 `Tuple<u32, u32, u32>`；
- 当你写 `...items: T where T` 时，因为 `T` 是泛型类型参数，调用 `keep_inputs(7u32, true, "ready")` 会把 3 个不同类型的位置实参收集成 `(7u32, true, "ready")`，此时 `T` 就被精确推导为 `Tuple<u32, bool, str>`。没有任何类型擦除，没有任何动态装箱，三个位置各自保留 `u32`、`bool` 和 `str` 类型。

> **🔍 语法小贴士：元组类型 `Tuple<...>` 与元组值 `(a, b)`**
> 在 Flurry 中，元组类型统一通过语言标准定义的 normal-form 类型函数写为 `Tuple<u32, bool>`（其签名为 `pure comptime fn Tuple<...Es: meta.List<Type>> -> Type`），而圆括号 `(7u32, true)` 用于构造或解构元组**值**（若写 `(u32, bool)`，则表示由两个 `Type` 值组成的编译期元组值，其类型为 `Tuple<Type, Type>`，而非元组类型本身）。
> 在函数体内部，无论你的变参收集了多少个输入，**一个收集参数最终在函数体里都只有一个绑定名**（如 `shape` 或 `items`）。如果你想拆开它，只需用我们熟悉的 `let (width, height, depth) = shape;` 模式解构即可。

让我们用位置变参写出清爽的 改进版本：

```flurry
use std.prelude.*;

-- 1. 固定形状的元组变参
fn volume(...shape: Tuple<u32, u32, u32>) -> u32 {
    let (width, height, depth) = shape;
    width * height * depth
}

-- 2. 泛型异构的元组变参：保留每个位置各自的精确类型。
fn keep_inputs(...items: T) -> T
where T
{
    items
}

test {
    let v = volume(2u32, 3u32, 4u32);
    println("volume: {}", v);

    -- 直接传 3 个不同类型的参数，无需双层括号。
    let (number, enabled, label) = keep_inputs(7u32, true, "ready");
    println("{} {} {}", number, enabled, label);
}
```

运行输出：

```text
volume: 24
7 true ready
```

而且，别忘了我们在 [第 021 篇](./021-call-evaluation-order-and-failure-boundaries.md) 学过的资源与求值核心规则：
**变参收集完全保留每个输入的从左到右求值次序与按值交付（Move / Copy）责任。**
如果在 `keep_inputs(make_res(1), fail()!, make_res(2))` 的收集过程中，第 2 个实参报错退出了，第 1 个已经求值成功的并非 `Copy` 资源会由调用准备阶段立即清理，一旦整个元组包收集成功并交付给 `items`，责任就正式由参数 `items` 接管，免令临时项和元组字段各自销毁一次。

---

## 三、 用 `.items = tuple` 直接供应完整包，以及 `...expr` 展开

现在，我们已经解决了散装传参 `volume(2, 3, 4)`。
那如果在某个调用点，上游已经给了我们一个打包好的元组 `let my_box = (2u32, 3u32, 4u32);`，我们该怎么把它传给 `volume(...shape: Tuple<u32, u32, u32>)` 呢？

如果你直接写位置传参 `volume(my_box)`，会发生什么？
想一想：因为直接写位置表达式走的是**“位置收集路径”**，编译器会把 `my_box` 当成要收集的**第 1 个元素**，收集出来的包就会变成单元素元组 `((2u32, 3u32, 4u32),)`（类型为 `Tuple<Tuple<u32, u32, u32>>`），跟签名要求的 `Tuple<u32, u32, u32>` 根本存在偏差。

在 Python 或 JS 里，你只能靠展开语法 `volume(*my_box)` 或 `volume(...my_box)` 把它拆开再重新收集一遍。
Flurry 当然也支持 **`...expr`（位置展开，`extend_arg`）**；但除此之外，Flurry 还提供了一个更直接、更正交的大招——**显式命名供应（`.name = expr`）**。

还记得我们在 [第 022 篇](./022-optional-parameters-and-forward-dependencies.md) 学的 `.start = 80` 吗？
在 Flurry 中，**非但可选参数接受 `.name = expr`，位置变参 `...shape` 同样接受 `.shape = expr`。**

下面对比“位置收集”与“`.shape = ...` 直接完整供应”的本质区别：

```text
  函数签名：fn volume(...shape: Tuple<u32, u32, u32>) -> u32

  路径 1：位置收集路径 ── volume(2, 3, 4)
     实参 2, 3, 4 ──► [ 启动元组收集器 ] ──► 打包为 (2, 3, 4) ──► 绑定到 shape

  路径 2：直接完整供应路径 ── volume(.shape = (2, 3, 4))
     具名实参 .shape = (2, 3, 4) ──► [ 旁路跳过收集器。直接交付完整包。 ] ──► 绑定到 shape
     (注意：不再多套一层变成 ((2, 3, 4),) 。)
```

看明白了吗？
当你写 **`volume(.shape = (2u32, 3u32, 4u32))`** 时，`.shape = ...` 告诉编译器：**“我给你的不是变参里的某一个元素，而是 `shape` 这个参数位置的完整最终值，请直接关闭收集器，原样接收它。”**

而如果你手里有一个二元元组 `let base = (2u32, 3u32);`，想再拼上一个高度 `4u32` 一起传给 `volume`，你就可以使用 **`...base`** 位置展开语法：`volume(...base, 4u32)`。展开的元组只求值一次，里面的各项按元组顺序依次送入位置收集器。

下面我们同时体验这三种丝滑的传参方式：

```flurry
use std.prelude.*;

fn volume(...shape: Tuple<u32, u32, u32>) -> u32 {
    let (width, height, depth) = shape;
    width * height * depth
}

test {
    -- 方式 1：散装位置收集
    let collected = volume(2u32, 3u32, 4u32);

    -- 方式 2：用 `.shape = ...` 直接供应完整元组包（免多包一层外层元组。）
    let full_box = (2u32, 3u32, 4u32);
    let supplied = volume(.shape = full_box);

    -- 方式 3：用 `...expr` 将已有元组按位置展开，与其他位置实参组合收集
    let base_area = (2u32, 3u32);
    let expanded = volume(...base_area, 4u32);

    println("{} {} {}", collected, supplied, expanded);
}
```

运行输出：

```text
24 24 24
```

### 编译器防呆：既传位置实参，又传 `.shape = ...` 完整包？编译期报错

既然“位置收集路径”和“`.shape = ...` 直接完整供应路径”在设计上是**严格互斥**的，如果有人在同一次调用里，既传了散装的位置参数 `2, 3, 4`，又在后面写了个 `.shape = (5, 6, 7)`，试图“既要收集又要整包覆盖”，会发生什么？

```flurry
fn volume(...shape: Tuple<u32, u32, u32>) -> u32 {
    let (width, height, depth) = shape;
    width * height * depth
}

-- ❌ 编译期直接报错。
-- 错误：同一个变参参数 `shape` 既收到了位置收集输入 (2, 3, 4)，又收到了完整包供应 (.shape = ...)。
fn invalid_volume_call() -> u32 {
    volume(2u32, 3u32, 4u32, .shape = (5u32, 6u32, 7u32))
}
```

编译器在参数归属阶段会为每个参数维护唯一的状态机（`未供应`、`直接供应`、`收集中`、`已完成`……）。一旦发现 `shape` 已经处于“收集中”，又收到了“直接供应” `.shape = ...`，就会立即报出供应来源冲突诊断，禁止一边提供整包、一边还往同一个包里塞散装元素。

---

## 本篇小结

通过引入位置变参 `...id: T` 与直接命名供应 `.id = tuple`，我们用统一的模型解决了异构多参数打包问题：

| 调用 / 声明写法 | 表面语法 | 核心语义与契约 |
| :--- | :--- | :--- |
| **固定形状元组变参** | `fn volume(...shape: Tuple<u32, u32, u32>)` | 将位置实参收集为三元 `u32` 元组，静态核验个数与每个位置的类型 |
| **泛型异构元组变参** | `fn keep(...items: T) -> T where T` | 将任意数量、不同类型的实参收集为异构元组，100% 保留各项精确静态类型 |
| **直接供应完整包** | `volume(.shape = (2, 3, 4))` | 跳过位置收集器，直接将该元组作为 `shape` 的完整值交付，**不多套一层元组** |
| **位置展开输入** | `volume(...base_area, 4)` | 求值一次 `base_area`，将其元素按顺序展开送入位置收集 |
| **冲突防呆检查** | `volume(2, 3, 4, .shape = ...)` | `-- ❌ 编译期直接报错。` 严禁对同一收集参数同时使用位置收集与整包直接供应 |

### 下一篇预告

位置变参 `...items` 把按顺序排列的实参收集成了**元组（Tuple）**，比如 `(8u32, true, "worker")`。
对于像坐标 `(x, y, z)` 这样顺序固定的数据，元组再合适不过了。

但是，当我们开始配置传感器分析仪的后台采集任务时，配置项往往长这样：
- `limit: 8u32`
- `enabled: true`
- `label: "worker"`
- 甚至还有嵌套的子配置：`nested: { value: false }`

如果用位置变参写成 `configure(8u32, true, "worker")`，读者根本难以铭记第 2 个 `true` 是什么意思；如果每次为了传一组临时键值配置，都得先在外面专门定义一个具名的 `struct WorkerConfig { ... }`，代码库里又会塞满几十个只用一次的“一次性配置结构体”。

有没有一种办法，让我们像写 JSON 或结构体字面量一样，直接用花括号调用函数：

```flurry
let active_limit = configure_worker {
    limit: 8u32,
    enabled: true,
    label: "worker",
};
```

并且让编译器把这些 `key: value` 属性收集成**强类型的匿名结构体**传入函数，在 PTC 字段约束下直接访问 `properties.limit` 或解构 `{ nested: { value }, enabled }`？

在下一篇 **《Flurry 演进手记 (025)：像写 JSON 一样调用函数——扩展调用 `fn { k: v }` 与匿名结构体属性收集》** 中，我们将解锁 Flurry 的扩展调用（Extended Application）与拓展变参。
