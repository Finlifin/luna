# 词法结构

Flurry 代码由一系列**词法单元（Token）**构成。理解这些基本规则是阅读和编写 Flurry 代码的前提。

## 标识符

标识符用于命名变量、函数、类型、模块等：

- 以**字母**（`a-z`、`A-Z`）或**下划线**（`_`）开头
- 其后可接任意数量的字母、数字（`0-9`）或下划线
- 大小写敏感：`myVar` 与 `myvar` 是不同的标识符

```flurry
let user_name = "Alice";
const MAX_CONNECTIONS = 100;
fn _internal_helper() { ... }
```

## 关键字

关键字是保留给语言本身的标识符，不可用作普通名称。常见关键字包括：

`fn` `let` `const` `struct` `enum` `impl` `trait` `if` `else` `when` `for` `while` `return` `break` `continue` `use` `mod` `pub` `comptime` `inline` `unsafe` `verified` `effect` `handles` `using` `where` `requires` `derive`

完整列表见 [grammar/syntax](../grammar/syntax)。

## 运算符与空格敏感性

Flurry 对部分二元运算符引入了**空格敏感**规则，用于在词法层面消除歧义（尤其是泛型尖括号 `<`/`>` 与比较运算符的混淆）。

规则：当 `+` `-` `*` `/` `%` `<` `>` **两侧都有空格**时，识别为标准二元运算符；否则（至少一侧无空格）识别为其他 token（如泛型分隔符）。

```flurry
let sum = a + b;          -- 二元加法
let list: Vec<i32>;       -- < 和 > 是泛型分隔符，两侧无空格
let gt = a > b;           -- 比较运算符，两侧有空格
```

这一规则鼓励在二元运算中统一加空格，同时让泛型语法保持紧凑。

## 注释

- **行注释**：以 `--` 开始，到行尾结束

```flurry
-- 这是一个行注释
let timeout = 1000; -- 超时时间（毫秒）
```

- **块注释**：以 `{-` 开始、`-}` 结束，可多行，支持嵌套

```flurry
{-
  这是一个块注释。
  {- 可以嵌套 -}
-}
```

## 分号与语句结束

Flurry 使用 `;` 分隔语句。但有一条省略规则：

> **当语句最后一个 token 的最后一个字符是 `}` 时，末尾的分号可以省略。**

```flurry
struct Point { x: i32, y: i32 }  -- '}' 结尾，分号可省略

fn main() {
    println("Hello");             -- ')' 结尾，分号不可省略
    if true {
        do_something();
    }                             -- if 语句的 '}' 结尾，分号可省略
}
```
