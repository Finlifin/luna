# Flurry 演进手记 (180)：复杂表达式的边界法则——多属性前缀、条件块边界与链式后缀视图

在[上一篇（第 179 篇）](./179-inside-the-compiler-pipeline-phases-overview.md)中，我们梳理了 Flurry 从定义期检查到调用点实例化的阶段边界。

今天，我们来解决编写复杂表达式和声明时经常遇到的三个语法与语义问题：
1. **多属性前缀链（Multi-Attribute Prefix）**：当我们在声明、列表元素或函数实参前连续挂载多个属性 `^attr1 ^attr2 expr` 时，它们按什么顺序生效？是否会导致被修饰的实参求值多次？
2. **控制流条件与花括号边界**：Flurry 的 `if` / `while` / `for` 条件无需外层圆括号，同时又支持 `Flag { enabled: true }` 构造与 `.data { value: _ }` 模式。当条件表达式里包含花括号时，什么时候必须加圆括号 `(...)`？
3. **链式后缀视图（Chained Take Views）**：当我们连续使用单引号视图（如 `value'bitvec'type`）或调用硬件原语 `tag'intrinsic(args).as(T)` 时，后缀链遵循怎样的结合规则？

---

## 一、 列表元素的多属性前缀与“从内向外”修饰规则

在 Flurry 的语法规则（`docs/grammar/syntax` 与 `docs/comptime/attribute.md`）中，列表元素（如函数调用的实参、数组元素、结构体字段等）支持连续前置多个 `^expr` 属性：

```text
multi_element -> base_element | ^expr multi_element
```

这意味着，在一个实参或声明前面，可以写零个、一个或多个 `^expr` 属性前缀：

```flurry
fn keep_flag(flag: bool) -> bool {
    flag
}

pub fn demo_multi_attrs() -> bool {
    keep_flag(^{ some_attr: 1 } ^{ some_attr: 2 } true)
}
```

### 修饰层级与单次求值保证

当多个属性前缀连续出现时，它们遵循右结合（从内向外）的包装关系：

```text
  源码书写:  ^{ some_attr: 1 }   ^{ some_attr: 2 }   true
             ─────────────────   ─────────────────   ────
                 外层属性             内层属性       基底表达式

  修饰层级:
  ┌───────────────────────────────────────────────────┐
  │ ^{ some_attr: 1 }                                 │  ◀── 外层修饰
  │   ┌───────────────────────────────────────────┐   │
  │   │ ^{ some_attr: 2 }                         │   │  ◀── 内层修饰 (紧贴基底表达式)
  │   │   true                                    │   │
  │   └───────────────────────────────────────────┘   │
  └───────────────────────────────────────────────────┘
```

同时，无论实参前面挂载了多少层编译期属性（例如 `^{ some_attr: 1 } ^{ some_attr: 2 } compute_flag()`），属性只在编译期提供附加元数据，在运行时基底实参表达式 `compute_flag()` **依然严格只求值一次**。

---

## 二、 控制流头部的外层花括号边界规则

在 Flurry 中：
- 构造结构体或发起扩展调用使用花括号：`Flag { enabled: true }`；
- 匹配带命名字段的变体模式也使用花括号：`.data { value: _ }`；
- 而 `if`、`while`、`for`、`match` 的头部条件后面，同样直接紧跟代码块 `{ ... }`，无需像 C 语言那样强制加圆括号。

那么，语言是如何区分“属于条件内部的花括号”与“属于控制流分支体的花括号”的？

### 1. 顶层条件在遇到首个外层 `{` 时结束

看下面两段最常见的代码：

```flurry
enum Message {
    empty,
    data { value: i32 },
}

fn check_flag(enabled: bool) -> i32 {
    if enabled { 1 } else { 0 }
}

fn is_empty(message: Message) -> bool {
    if message matches .empty {
        true
    } else {
        false
    }
}
```

在 `if`、`while`、`for`、`match` 的头部条件顶层（未进入圆括号内部时），任何紧跟在表达式或模式后面的 `{ ... }` 都归属于外层控制流的分支代码块：
- 在 `if enabled { 1 }` 中，`enabled` 后面遇到 `{`，条件立即结束，`{ 1 }` 是 `if` 的分支体；
- 在 `if message matches .empty { true }` 中，`.empty` 后面遇到 `{`，模式与条件同样立即结束，`{ true }` 是 `if` 的分支体。

### 2. 在条件内部使用带 `{ ... }` 的构造或模式时，用圆括号 `(...)` 明确包裹

如果我们确实需要在 `if` 条件中现场构造一个结构体，或者使用带花括号字段的变体模式 `.data { value: _ }`，只需用圆括号 `(...)` 将内层表达式或 `matches` 判断括起来：

```text
  [ 场景 A：直接处于 if 条件顶层 ]
    if  message matches .empty  { true } else { false }
        ──────────────────────  ▲
                  │             └── 顶层遇到 `{`，条件在此结束，`{ true }` 归属 if 分支
                  └──────────────── 条件部分

  [ 场景 B：在圆括号 (...) 内构造结构体 ]
    if  (Flag { enabled: true }).enabled  { 1 } else { 0 }
        ▲                      ▲          ▲
        └── `(` 内允许解析      └── `)` 外恢复顶层边界，`{ 1 }` 归属 if 分支
            `Flag { enabled: true }`

  [ 场景 C：在圆括号 (...) 内书写带花括号的 matches 模式 ]
    if  (message matches .data { value: _ })  { true } else { false }
        ▲                                  ▲  ▲
        └── `(` 内允许 `.data { value: _ }`┘  └── `)` 外 `{ true }` 归属 if 分支
```

### 3. 负例：在未加圆括号的 `if` 条件顶层直接写 `{ ... }` 模式

如果在 `if` 条件顶层省略圆括号，直接写出 `.data { value: _ }` 并紧跟 `{ true }`，第一个 `{ value: _ }` 会被视为 `if` 分支体的开始，从而导致语法错误：

```flurry
-- ❌ 编译期语法报错：未加圆括号导致花括号边界冲突
fn broken_unparenthesized_block_in_if(message: Message) -> bool {
    if message matches .data { value: _ } {
        true
    } else {
        false
    }
    -- ✅ 正确写法：
    -- if (message matches .data { value: _ }) { true } else { false }
}
```

---

## 三、 链式后缀视图（`expr'id`）与左结合规则

在前面的章节里，我们已经使用过单引号后缀视图（Take View）：
1. **`value'type`**：获取表达式 `value` 的静态类型（Type View）；
2. **`value'bitvec`**：获取表达式 `value` 的定宽位向量视图（BitVec View）；
3. **`tag'intrinsic`**：从 `^intrinsic_enum` 标签获取硬件原语可调用入口。

所有的后缀操作（包括 `.field`、`[idx]`、`(args)`、`'view`、`.as(T)`）都遵循**从左到右（左结合）**的组合顺序，因此我们可以自然地链式组合多个视图：

```flurry
pure comptime fn InspectViews(value: u32) -> Type {
    -- 先取 value 的位向量视图 (类型为 BitVec<32>)，
    -- 再取该位向量视图的静态类型 (得到 Type: BitVec<32>)。
    value'bitvec'type
}
```

```text
  [ 后缀链从左到右依次作用 ]
    1. `value'bitvec'type`
       └──▶ 第 1 步：value'bitvec         (产出一个 BitVec<32> 视图表达式)
       └──▶ 第 2 步：(value'bitvec)'type  (观察该视图表达式的静态类型，得到 BitVec<32>)

    2. `unsafe { X86Inst.fma'intrinsic(a, b, c).as(f32) }`
       └──▶ 第 1 步：X86Inst.fma'intrinsic          (取得硬件原语入口)
       └──▶ 第 2 步：... (a, b, c)                  (传入实参调用)
       └──▶ 第 3 步：... .as(f32)                   (紧邻指定输出类型为 f32)
```

如果拼写了未定义的视图名（例如 `value'unknown_view`），会在语法与视图检查阶段被立即拦截，免与普通的结构体字段访问混淆。

---

## 四、 本篇小结

| 语法特性 | 书写规则 | 语义保证 |
| :--- | :--- | :--- |
| **`^expr` 多属性前缀** | 支持在声明或列表元素前连续书写 `^attr1 ^attr2 item` | 从内向外依次修饰目标项；运行时实参仍然只求值一次 |
| **控制流条件花括号边界** | 条件顶层遇到 `{` 即进入分支块；内层 `{ ... }` 需用 `(...)` 包裹 | 消除无括号 `if cond { ... }` 与结构体构造/模式的歧义 |
| **后缀视图链 `expr'a'b`** | 从左到右依次转换视角；`'intrinsic(...)` 的返回类型通过紧邻 `.as(T)` 指定 | 保持数据流与类型转换方向从左到右一致可读 |

---

## 五、 下一篇预告

掌握了复杂表达式与后缀视图链的书写规则后，下一篇我们将探讨如何结合 `'type` 视图、`Type.repr_of` 编译期结构反射与 CMTT 代码值，在编译期自检类型结构与布局不变量。

下一篇 **[Flurry 演进手记 (181)：用编译期反射与类型视图审视代码结构——结构内省、布局断言与契约自检](./181-why-all-flurry-irs-dump-as-lisp-s-expressions.md)**，我们继续推进。
