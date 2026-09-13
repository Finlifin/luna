# 语法风格

Flurry 的表面语法借鉴了 Rust，但在路径、后缀链、类型表达式和运算符空白规则上有自己的选择。本章概括这些「看起来为什么会这样」的约定；更细的产生式见 [syntax](./syntax)。

## 你会学到

- 路径与类型方法如何写成投影
- 为什么普通表达式偏后缀、类型表达式偏前缀
- take view（`'`）、注释、中缀空白、谓词与位运算的约定
- 推荐的命名风格

## 路径与类型

路径使用**投影**语法：`a.b.c` 表示在 `a.b` 上取 `c`。

在 Flurry 里，**类型是编译期表达式**，因此与普通表达式共用同一套语法。若要调用 `Type` 上的方法，为避免与值级投影混淆，请直接写类型名：

```flurry
Type.size_of(usize)
```

---

## 表达式：偏后缀

普通表达式更倾向**后缀**形式，便于链式调用，也更利于机器处理：

| 含义 | 写法 |
|------|------|
| 取地址 | `some_val.ref` |
| 解引用 | `some_val.*` |
| option / error / effect 的传播与消除 | 后缀运算符 |
| 模式匹配 | 后缀 `match`（类似 Scala） |
| 类型转换 | `some_val.as(usize)` |
| trait object | `some_val.dyn(TraitExpr)` |

`as` 与 `dyn` 是关键字，调用形态像函数。

```flurry
let kind = age match {
    ..= 18 => "child",
    ..= 60 => "adult",
    _ => "elder",
}
```

无法也不应为每一种「取值视角」都新增关键字。受 Ada 启发，Flurry 提供 **take view**：`expr ' id`（由 `.` 演化为 `'`，因此不必引入 `type` 关键字）：

```flurry
const SomeType: Type = some_val'type;
```

已有多种 view kind，例如 `bitvec`、`tuple_enumerate`、`multi_deref`、`old`（用于编写 specification）等，扩展空间很大。

---

## 类型表达式：偏前缀

类型一侧更倾向**前缀**组合，便于嵌套与推导。依赖类型足以表达数组与切片，因此没有单独的 `[T; N]` / `&[T]` 一类语法。

| 写法 | 含义 |
|------|------|
| `?T` | option type |
| `!Errors T` | error qualified type |
| `#Effect` | effect qualified type |
| `dyn TraitExpr` | trait object type |
| `^Psi F` | closure type |
| `*T` | pointer type |
| `&ReachabilitySet T` | reachability type |

---

## 注释

- 单行：`-- …`
- 块注释：`{- … -}`

---

## 分隔符

对象或扩展调用中的命名项使用 **property** 形式 `id: expr`。定义作用域中的 **configuration** 使用 `.id expr`，为所属定义编码属性，例如结构体中的 `.packed true`。二者的语义目标不同，见[属性系统](../comptime/attribute.md)。

[`syntax`](./syntax) 里形如 `foo*` 的列表**省略了分隔符**。实际源码中：

- **一般项**（字段、参数、`match` / `when` 臂、元组元素等）：项与项之间用**逗号** `,`
- **语句**（含表达式语句、`let`、`return` 等）：句与句之间用**分号** `;`

分隔符由**该项自身的产生式**决定，而不是由外层「碰巧是 struct / 块」决定。例如在 `struct` 体内：字段定义之间是逗号；若出现语句 / 以语句计的定义，之间是分号。

若某一项的最后一个 token 已是 `}`，**其后本应出现的分号省略不写**（不要写成 `};`）：

```flurry
struct Foo {
    man: Man,
    pub fn foo() {}
}

let kind = age match {
    ..= 18 => "child",
    ..= 60 => "adult",
    _ => "elder",
}
```

上例中 `pub fn foo() {}` 与整个 `let … match { … }` 均以 `}` 结尾，故后面不再写 `;`。不以 `}` 结尾的语句（如 `start()`、`let x = 1`）之间仍须写 `;`。

---

## 中缀运算符与空白

`-`、`+`、`<`、`>` 等字符是否作为**中缀运算符**，由两侧空白决定：

- **作中缀**：两侧都必须是空白（空格、制表、换行等），例如 `a - b`、`a < b`
- **不作中缀**：至少一侧紧贴非空白，例如 `Vec<usize>`、`-23`

这把泛型尖括号、一元负号与比较/算术运算符区分开，而不再依赖额外的消歧规则。

---

## 谓词与位运算

布尔组合不用 `||`、`&&`、`!`，请用关键字：

```flurry
not ready or (a and b)
```

语言**不提供位运算符**。需要位运算时：

1. 使用 `math` 库中的函数，例如 `math.bit_and`、`math.bit_or`、`math.bit_xor`；或
2. 更推荐依赖 **bitvec** 等 view / 抽象，由编译器降级为高效位运算。

---

## 命名规范

| 类别 | 风格 |
|------|------|
| 函数、变量、模块、包、宏、枚举变体、非类型参数、handler、effect、自定义 case、predicate、lemma、axiom、invariant、outcome | `snake_case` |
| 常量、全局变量 | `SCREAMING_SNAKE_CASE` |
| 类型名、用于编码错误的枚举名、trait 名 | `PascalCase` |

标识符由一个或两个单词构成时，建议写出完整单词，不使用缩写；已经约定俗成、接近独立词语的形式可以保留，例如 `vec`。例如，类型名使用 `StructDefinition`，而不是 `StructDef`。
