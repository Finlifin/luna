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

目标操作使用 `expr'intrinsic(argument*)`。紧接这种调用的 `.as(T)` 指定操作的输出类型，省略时按 void 处理；这里的 as 不先取得 void 再转换。具体写法与目标检查见 [Intrinsic 与输出类型](../pipeline-projection/intrinsics.md)。

---

## 类型表达式：偏前缀

类型一侧更倾向**前缀**组合，便于嵌套与推导。依赖类型足以表达数组与切片，因此没有单独的 `[T; N]` / `&[T]` 一类语法。

| 写法 | 含义 |
|------|------|
| `?T` | option type |
| `!Errors T` | error qualified type |
| `#Effects T` | effect qualified type |
| `dyn TraitExpr` | trait object type |
| `%C T` | closure type |
| `*T` | pointer type |
| `&ReachabilitySet T` | reachability type |

限定前缀可以组合，例如 `#Effects !Errors T` 保存外层效应与内层错误结果两层结构，不把 `!` 解析为 Effects 上的后缀操作。若限定上下文本身需要使用与下一层前缀有歧义的运算，用括号划定范围。限定类型仍遵守所在语法的边界：lambda 参数的结束 `|`、contextual code type 的 `|-` 与结束 `>` 不属于内部类型表达式。

限定主体包含函数结果箭头，但不吸收外层比较、trait bound 或布尔命题。例如 `#Effects fn() -> T == Other` 比较整个效应限定类型与 Other；`#Effects T :- TraitExpr` 对限定类型施加 trait 约束。`!`、`&`、`%` 的限定主体采用相同边界，括号可以显式指定更内层的组合范围。

---

## 注释

- 单行：`-- …`
- 块注释：`{- … -}`

---

## 分隔符

以 multi 规则解析的语法元素列表允许在每个元素前递归添加属性前缀；属性修饰该元素，分隔符仍由元素本身决定。该规则不把原始 token 列表或任意内部操作数位置改成属性入口。具体边界与示例见[源码解析](../compilation/syntax-and-hir.md)。

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

## 上下文与范围

表达式和模式还受所在构造的外层边界约束：if 等构造的主体花括号不能被当前条件吞成后缀扩展应用。这一边界也通过 matches 传给模式；括号可为内部扩展应用划出完整范围。它与运算符空白规则分别处理，见[表达式与模式的外层边界](../compilation/syntax-and-hir.md#表达式与模式的外层边界)。

## 中缀运算符与空白

`+`、`-`、`*`、`/`、`<`、`>`、`<=`、`>=` 是否作为**中缀运算符**，由两侧空白决定：

- **作中缀**：两侧都必须是空白（空格、制表、换行等），例如 `a - b`、`a < b`
- **不作中缀**：至少一侧紧贴非空白，例如 `Vec<usize>`、`-23`

范围等其他运算符按各自产生式处理，例如 `0..10`。

这把泛型尖括号、一元负号与比较/算术运算符区分开，而不再依赖额外的消歧规则。

---

## 谓词与位运算

布尔组合不用 `||`、`&&`、`!`，请用关键字：

```flurry
not ready or (a and b)
```

语言不提供独立的传统位运算符。`and`、`or`、`not` 在 BitVec 上分别表示逐位与、或、非；其中二元操作求值两侧，各一次，不沿用 bool 的短路语义。需要位运算时：

1. 使用 `math` 库中的函数，例如 `math.bit_and`、`math.bit_or`、`math.bit_xor`；或
2. 使用 [BitVec 代数](../program-synthesis/bitvec-algebra.md)，通过 `'bitvec`、切片、拼接及逐位组合描述映射，由编译器生成并优化位操作。

---

## 命名规范

| 类别 | 风格 |
|------|------|
| 函数、变量、模块、包、宏、普通枚举变体、非类型参数、handler、effect、自定义 case、predicate、lemma、axiom、invariant、outcome | `snake_case` |
| 常量、全局变量 | `SCREAMING_SNAKE_CASE` |
| 类型名、trait 名 | `PascalCase` |
| 错误类型名 | `PascalCase`，以 `Err` 结尾 |
| 错误枚举变体 | `PascalCase` |

标识符由一个或两个单词构成时，建议写出完整单词，不使用缩写；已经约定俗成、接近独立词语的形式可以保留，例如 `vec`。例如，类型名使用 `StructDefinition`，而不是 `StructDef`。
