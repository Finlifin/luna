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

## 关键字与预留关键字

关键字在相应语法位置有固定含义，不能在普通绑定位置当作未经转义的名称。大小写参与区分，例如 self 与 Self 是不同的词。下表列出本语言使用的完整拼写；符号标点不计入关键字。

| 类别 | 关键字 |
|---|---|
| 定义与组织 | assoc、case、const、define、derive、effect、enum、fn、impl、mod、newtype、private、pub、struct、template、test、trait、typealias、union、use |
| 绑定与参数 | catch、implicit、itself、Itself、lambda、let、quote、self、Self、var |
| 控制与模式 | as、async、await、break、continue、do、else、error、for、handles、if、in、is、match、matches、ref、resume、return、when、while |
| 阶段与检查方式 | atomic、comptime、defer、inline、pure、spec、unsafe、verified |
| 类型与规格 | asserts、assumes、bool、decreases、dyn、ensures、exists、forall、invariant、lift、outcomes、predicate、refines、requires、where |
| 逻辑与固定值 | and、false、not、null、or、true、undefined |

template 用于函数、quote 函数与测试模板；quote 也用于参数角色；defer 在编译时保留表达式；var 与 Itself 分别遵循绑定和反身名称规则。词法识别只确认拼写，不表示任意关键字都能出现在任意参数或定义位置。

以下拼写保留供专门语法使用，本表不赋予它们新的构造语义：

~~~text
Fn  axiom  default  extend  ghost  lemma  move  opaque  opens  static
~~~

预留不等于可以按其他语言同名构造使用。已定义属性路径中的成员名按路径位置解释，例如 attrs.derivation.default；该成员不使 default 成为一个普通的顶层绑定名。

Type、Any、any、u32、attrs、builtin 等名称不因常用或内置就自动成为关键字。它们按各自声明、值或内置命名环境的规则解析。单独的 _ 是通配符标点，其含义由表达式、模式或参数位置决定。

## 运算符与空格敏感性

Flurry 对部分二元运算符引入了**空格敏感**规则，从而在词法层面消除歧义（尤其是泛型尖括号 `<`/`>` 与比较运算符的混淆）。

规则：当 `+` `-` `*` `/` `%` `<` `>` **两侧都有空格**时，词法器将其识别为标准二元运算符；否则（至少一侧无空格）词法器将其识别为其他 token（如泛型分隔符）。

```flurry
let sum = a + b;          -- 二元加法
let numbers: Array<i32, 2> = [1i32, 2i32]; -- 尖括号在这里是泛型分隔符
let gt = a > b;           -- 比较运算符，两侧有空格
```

这一规则鼓励开发者在二元运算中统一加空格，同时让泛型语法保持紧凑。

## 注释

- **行注释**：以 `--` 开始，到行尾结束

```flurry
-- 这是一个行注释
let timeout = 1000; -- 超时时间（毫秒）
```

- **块注释**：以 `{-` 开始、`-}` 结束，可多行，并且支持嵌套

```flurry
{-
  这是一个块注释。
  {- 可以嵌套 -}
-}
```

## 分号与语句结束

Flurry 使用 `;` 分隔语句。但有一条省略规则：

> **当语句的最后一个 token 为 `}` 时，省略其后的分号；其他语句以分号分隔。**

```flurry
struct Point { x: i32, y: i32 }  -- '}' 结尾，省略分号

fn main() {
    println("Hello");             -- ')' 结尾，分号不可省略
    if true {
        do_something();
    }                             -- if 语句以 '}' 结尾，省略分号
}
```
