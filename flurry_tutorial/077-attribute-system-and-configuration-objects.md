# Flurry 演进手记 (077)：元数据也是正经的编译期对象——^expr 属性系统、.id expr 配置与 attrs 包

在上一回（076）中，我们学会了用 `Type.size_of(T)` 和 `Type.repr_of(T)` 透视张量元素与 DMA 描述符的物理骨骼，也留了一个关乎硬件生死的悬念：**默认结构体布局不保证字段顺序与无填充紧凑排列**。如果我们的 DMA 硬件控制器要求描述符严格紧凑（`packed`），要求枚举状态码底层用 `u8` 存储（`base_type u8`），要求跨语言回调遵循 C ABI（`abi(.c)`），该怎么向编译器下达指令？

在许多传统语言里，“属性（Attributes / Annotations）”是一门寄居在主语言之外的“小方言”——比如特制的括号 `#[...]` 或 `@...`，里面的参数既非普通表达式，亦无法用普通变量保存、组合或通过普通作用域规则查找。

而在 Flurry 的双层类型理论（2LTT）视角下，答案异常干脆：**元数据（Metadata）全然无需发明一套法外语法，它本来就是编译期宇宙（$U_1$）里的正经 `Object` 值。** 今天，我们就来拆解 Flurry 简洁而强大的 `^expr` 前缀属性、`.id expr` 内部配置以及内置 `attrs` 包。

---

## 一、 问题背景与设计思路：为什么报错“宏方言式”属性？

假设我们要为张量 DMA 引擎定义一组硬件寄存器帧和状态码。如果属性是一套封闭的特殊标签（传统伪代码体验），我们会立刻撞上三个核心问题：

1. **无法复用与参数化**：如果 20 个硬件结构体都需要同一套对齐与序列化配置，你没法把这套配置写成一个普通编译期变量或函数返回值去复用。
2. **名字查找搞特殊化**：特殊属性标签往往有自己独立的命名空间，你无从得知某个名字到底来自标准库、第三方库还是当前作用域。
3. **外部修饰与内部自描述割裂**：有时候我们想在函数头顶挂一个外部修饰（如 `^attrs.abi(.c)`），有时候又想在 `struct` 或模块大括号内部声明它自身的固有配置（如 `.packed true`）。

在 Flurry 中，这一切被统一成了三种清晰的语法构件：

| 语法形式 | 语法名称 | 出现位置 | 核心作用 |
| :--- | :--- | :--- | :--- |
| `id: expr` | **Property（属性项）** | `Object` 字面量或扩展调用内部 | 为对象或调用提供一个具名键值项，如 `{ some_attr: 1 }` |
| `^expr` | **Attribute（前缀属性）** | 任意支持修饰的声明、参数、分支或表达式前 | 在编译期求值 `expr`（结果须为 `Object`），将其键值合并到目标的元数据表 |
| `.id expr` | **Configuration（内部配置）** | 结构体、枚举、模块、文件等定义体内部 | 在定义体内部直接向**外层宿主定义**写入一项键为 `id`、值为 `expr` 的编译期元数据 |

---

## 二、 升级：`^expr` 与 `.id expr` 的双剑合璧

让我们直接用 Flurry 的真实语法来编写 DMA 硬件紧凑帧 `PackedPair`、单字节状态码 `SmallCode` 以及 C ABI 导出函数。

注意看：`.packed true` 写在 `struct PackedPair` 的大括号里面，但它**并非一个运行期数据字段**，而是编译期配置项（Configuration）。

```flurry
-- 1. 在 struct 内部使用 `.id expr` 配置紧凑布局
struct PackedPair {
    .packed true,
    first: u8,
    second: u32,
}

-- 2. 在 enum 内部使用 `.base_type u8` 指定底层判别子表示类型
enum SmallCode {
    .base_type u8,
    ok = 0,
    busy = 1,
}

-- 3. 使用前缀 `^expr` 挂载内置 attrs 包生成的属性对象
^attrs.abi(.c)
fn exported_add(lhs: i32, rhs: i32) -> i32 {
    lhs + rhs
}

test {
    -- 通过 meta.attrs_of(target) 在编译期读取目标上的元数据表。
    assumes meta.attrs_of(PackedPair).get(.packed, bool) == true;
    assumes meta.attrs_of(SmallCode).get(.base_type, Type) == u8;

    -- 紧凑布局消除了 u8 与 u32 之间的 3 字节对齐填充：1 + 4 = 5 字节。
    assumes Type.size_of(PackedPair) == 5;
    -- 指定 base_type 为 u8 后，SmallCode 只占 1 字节。
    assumes Type.size_of(SmallCode) == 1;
}
```

> **🔍 语法小贴士：`.packed true,` 与普通字段 `first: u8,` 的视觉区分**
> - 普通结构体字段用冒号 `id: Type` 声明运行期存储槽位；
> - 内部配置用点号开头加空格 `.id expr,` 声明编译期元数据。它免于在实例中占用任何运行期字段槽位，而是直接把 `{ packed: true }` 写入外层 `PackedPair` 的元数据字典中，并可通过 `meta.attrs_of(PackedPair)` 反射读取。

---

## 三、 深入机制：为什么允许连写 `^a ^b`？多个同名属性谁覆盖谁？

在写复杂工程时，一个函数或结构体上往往会叠加多层属性。比如既有默认配置模板，又有针对某个特定寄存器的局部微调。这时候，有两个核心问题必须钉死：
1. 为什么在参数列表、结构体字段、枚举分支前可以连续挂任意多个 `^expr`？
2. 当多个 `^expr` 或内部 `.id expr` 写了同一个键（Key）时，最终谁生效？

### 3.1 `multi` 语法规则与任意长度的 `^expr` 链

在 Flurry 的语法规范中，凡是由 `multi` 规则解析的列表元素（如结构体字段、枚举变体、函数参数、`match` 分支、语句等），都遵循递归产生式：

$$\text{multi\_element} \rightarrow \text{base\_element} \mid \text{\texttt{\textasciicircum}}expr\ \text{multi\_element}$$

这意味着你非但可以在顶层声明前连写 `^a ^b`，在结构体字段或参数前同样可以挂载任意多个前缀属性。

### 3.2 结合顺序与覆盖核心规则：从内向外应用，外层覆盖内层

假设我们在 `fn annotated()` 前面连写了两个包含同名键 `some_attr` 的属性对象，或者既有内部配置 `.some_attr 2` 又有外部前缀 `^{ some_attr: 1 }`，结果会是多少？

让我们画一张**属性洋葱模型（Attribute Onion Model）**图：

```
  +-------------------------------------------------------------------+
  | 外层前缀属性：^{ some_attr: 1 }   <-- [第 3 步应用：最后写入，最终胜出。]
  |   +---------------------------------------------------------------+
  |   | 内层前缀属性：^{ some_attr: 2 } <-- [第 2 步应用：覆盖内部配置]
  |   |   +-----------------------------------------------------------+
  |   |   | 目标定义体：structconfigured                              |
  |   |   |   .some_attr 3;           <-- [第 1 步应用：最贴近目标，最先打底]
  |   |   +-----------------------------------------------------------+
  +-------------------------------------------------------------------+
```

记住这条契合直觉的工程核心规则：
1. **距离目标越近（越靠内）的元数据越先应用（打底）**；
2. **距离目标越远（越靠外）的前缀属性越晚应用，若遇到同名键则覆盖（Override）先前的值**。
3. 因此：**内部配置 `.id expr` 先于外部前缀 `^expr` 应用；紧贴目标的内层 `^expr` 先于外层 `^expr` 应用**。最终外层前缀属性拥有最高覆盖权。

让我们用代码验证这一覆盖顺序：

```flurry
-- 离目标最近的 ^{ some_attr: 2 } 先应用，外层的 ^{ some_attr: 1 } 后应用并覆盖它
^{ some_attr: 1 }
^{ some_attr: 2 }
fn annotated() {}

-- 内部配置 .some_attr 2 先应用，外部前缀 ^{ some_attr: 1 } 后应用并覆盖它
^{ some_attr: 1 }
struct Configured {
    .some_attr 2,
}

test {
    comptime {
        assumes meta.attrs_of(annotated).get(.some_attr, Integer) == 1;
        assumes meta.attrs_of(Configured).get(.some_attr, Integer) == 1;
    }
}
```

为什么这样设计？想象你有一个内部写了默认配置 `.some_attr 2,` 的组件或内层宏生成的声明，当外部调用者在最外层套上 `^{ some_attr: 1 }` 时，外部显式指定的覆盖自然应该战胜内层默认值。

---

## 四、 进阶：普通词法作用域查找与 `attrs` 内置包

很多从其他语言转过来的同学会问：“我在 `^` 后面写名字时，编译器是否会自动去 `attrs` 包里找？”

**并非如此。** 这正是 Flurry 属性系统最纯粹的地方：`^expr` 里的 `expr` 就是**普普通通的编译期表达式**，遵循 100% 标准的词法作用域（Lexical Lookup）规则。
- 内置包 `attrs`（提供 `attrs.abi`、`attrs.derivation`、`attrs.effects`、`attrs.compilation`、`attrs.comptime_phase`、`attrs.quote_fn_template` 等属性构造接口）之所以能直接写 `^attrs.abi(.c)`，仅仅是因为 `attrs` 是预导入的内置包名。
- 你可以用 `use` 给它起别名，可以把属性对象存进局部 `const` 绑定，甚至如果当前作用域有一个同名局部绑定，它会**正常遮蔽（Shadow）**导入的名字。

来看普通作用域查找与局部遮蔽的实证：

```flurry
use attrs.derivation.default as default_strategy;

-- 1. 使用 use 导入的别名作为属性表达式
^default_strategy
struct GlobalConfig {
    timeout_ms: u32,
}

fn check_lexical_shadowing() {
    -- 2. 在局部作用域定义一个同名编译期对象，它会按正常词法规则遮蔽导入的 default_strategy。
    const default_strategy = { custom_tag: 42 }

    ^default_strategy
    struct LocalConfig {
        retries: u32,
    }

    comptime {
        assumes meta.attrs_of(LocalConfig).get(.custom_tag, Integer) == 42;
    }
}
```

那么，如果有人在 `^expr` 后面写了一个求值结果并非编译期 `Object` 的表达式，会发生什么？

```flurry
-- ❌ 编译期直接报错：`^expr` 要求 expr 在编译期求值为元数据 Object，裸整数 123 并非合法属性对象
^123
fn invalid_attribute_target() {}
```

编译器会干脆利落地报错：属性前缀期待一个编译期键值对象，而非一个孤零零的整数。

---

## 五、 本篇小结

| 特性维度 | Flurry 的设计抉择 | 解决的工程问题 |
| :--- | :--- | :--- |
| **前缀属性 `^expr`** | `expr` 在编译期求值为 `Object` 并附加到目标 | 无需特殊属性宏方言，支持函数调用、别名与组合 |
| **内部配置 `.id expr`** | 在定义体内部向宿主写入元数据（并非运行期字段） | 模块、文件与类型可在内部优雅地自描述布局和配置 |
| **覆盖优先级** | 内部 `.id expr` $\rightarrow$ 内层 `^expr` $\rightarrow$ 外层 `^expr`（后者覆盖前者） | 允许外层装饰器自然覆盖内层默认配置 |
| **名字查找规则** | 100% 遵循普通词法作用域查找，不给 `attrs` 开后门 | 避免幽灵命名空间，支持 `use ... as` 与局部绑定 |
| **反射读取接口** | `meta.attrs_of(target).get(.key, ExpectedType)` | 编译期元编程可以类型安全地读取任何目标的元数据 |

---

## 六、 下一篇预告

在最近两回中，我们频繁写出了 `pure comptime fn`、`Type.size_of(T)`、`defer SelectType(wide)` 这样的编译期计算。不知你有没有思考过编译器底层的两个灵魂问题：
1. 为什么 `pure comptime fn PairOf(T: Type) -> Type` 敢保证“两次传入 `u32` 一定返回同一个类型身份”？如果一个编译期函数悄悄读取了外部可变状态或耗尽了步数预算，编译器怎么防止缓存被污染？
2. 当编译器检查一个带有未决参数的函数签名 `fn keep_selected(comptime wide: bool, value: defer SelectType(wide))` 时，`wide` 的具体值还无从得知，编译器到底把什么东西**冻结（Freeze）**存进了函数定义里，又把什么东西挡在了门外？

下一回 **《Flurry 演进手记 (078)：当编译期计算遇到副作用与缓存——Comptime 纯度、确定性 memoization 与冻结边界》**，我们将深入编译器前端 Elaboration 与 Comptime 交织运转的心脏。
