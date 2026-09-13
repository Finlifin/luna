# 字面量

## Intro

字面量是源代码里直接写出的固定值，也是表达式的基本构件。

每种字面量都有对应类型（如整数 → `Integer`，字符串 → `str`）。类型如何处在编译期 / 运行时宇宙、以及如何 **splice**，是语言的通用规则，见 [基本数据类型](./primitives.md)。无后缀时，整数 / 浮点在推导失败时按 `i32` / `f64` 处理。

数字与字符串字面量后可跟标识符后缀，例如 `23u32`、`"msg"c`。语义上等价于对该类型的调用：`Integer.u32(23)`、`str.c("msg")`。可用 `extend` 为这些类型增加自己的后缀。

`null`、`()`、`_`、`undefined` 见 [unit-null-undefined.md](./unit-null-undefined.md)。

---

## 语法

```text
literal -> integer | float | char | str | bool | symbol

integer -> decimal | binary | octal | hex
decimal -> digit (digit | _)*
binary  -> 0b (bindigit | _)*
octal   -> 0o (octdigit | _)*
hex     -> 0x (hexdigit | _)*

float -> digits . digits
       | digits . digits exp
exp   -> (e | E) (+ | -)? digits

char -> ' char_body '
str  -> " str_body "

bool -> true | false
symbol -> . id

literal_suffix -> (integer | float | str) id

raw_str -> @builtin.str.raw { raw_source }
```

转义见正式定义。更细的全局产生式见 [grammar/syntax](../grammar/syntax)。

---

## 正式定义

### 字面量类型

| 字面量 | 类型 |
|--------|------|
| 整数 | `Integer` |
| 浮点、科学计数法 | `Float` |
| `"…"` | `str` |
| `'…'` | `char` |
| `true` / `false` | `bool` |
| `.id` | `Symbol` |

宇宙归属与 splice 见 [基本数据类型](./primitives.md)。这些名字通常可直接使用；需要时也可写完整模块路径。

### 整数

- 十进制以数字开头；数字之间可插入 `_`（含连续 `_`）。`_23` 是 `_` 与 `23`，不是一个整数。
- 二进制 / 八进制 / 十六进制前缀为 `0b` / `0o` / `0x`（小写）。
- 十六进制中的 `a–f`：去掉 `_` 后须全小写或全大写。
- 负号是前缀运算符 `-`，不是字面量的一部分（与中缀 ` - ` 不同）。

### 浮点

- 形式为 `digits.digits`，可接科学计数法指数。
- 不允许 `1.`、`.5`、`1e6`（指数前必须是完整小数，如 `1.0e6`）。
- 指数的 `e` / `E` 与两侧数字之间不能有空白。

### 后缀

- 仅用于整数、浮点、字符串字面量。
- `lit id` 等价于 `Type.id(lit)`，`Type` 为该字面量的类型（如 `Integer`、`str`）。
- 后缀必须能解析到对应定义，否则为错误。
- 只能加一次后缀：`100u32px` 中的 `u32px` 是一个标识符，不是两层后缀。
- 自定义后缀写在对应类型的 `extend` 上（例如 `extend Integer { … }`）。

### 字符与字符串

- 字符：单引号，恰好一个 Unicode 标量值。
- 字符串：双引号，类型为 `str`。

**转义**（字符与字符串相同）：

| 序列 | 含义 |
|------|------|
| `\\` | `\` |
| `\'` | `'` |
| `\"` | `"` |
| `\n` | 换行 |
| `\r` | 回车 |
| `\t` | 制表 |
| `\0` | NUL |
| `\$` | `$` |
| `\x{H…H}` | Unicode 标量（1–6 位十六进制） |

其它以 `\` 开头的序列为错误。

- 相邻的字符串字面量（中间只有空白）合并为一个字符串。
- **插值**（仅 comptime）：`$name` 或 `${expr}`，只能嵌入 comptime 值。只有 `$` 引入插值；单独的 `{` / `}` 就是普通字符。含插值的字符串等价于一次 `std.fmt.comptime_format` 调用。
- **原始字符串**：`@builtin.str.raw { … }`，括号内内容按原样保留。说明见 [构造宏](../macro/construction-macro.md)。

### 布尔

`true`、`false`。

### 符号

`.id`，类型为 `Symbol`。

- 同名符号在同一编译单元中是同一个值。
- 可在 comptime、模式匹配、属性等场景使用。
- 不是字符串；可用 `.ok'tag_name` 取到 `"ok"`。

常见用途包括枚举无数据变体、属性键、对象键、轻量标签。

---

## 常见示例

```flurry
42;
0b1010_1010;
0o777;
0xFF_23;
0xabcd;
1_000_000;
23u32;
-42;

3.14159;
1.0e-6;
2.718f32;
1_234.567_89;

'a';
'\n';
'\x{3A3}';

"Hello, Flurry!";
"line\nnext\tstop";
"cost \$5";
"say you, " "say me";
"path"c;
"data"b;

const VERSION: str = "0.1.0";
const BANNER: str = "Flurry $VERSION";
const HELLO: str = "hello";
const WORLD: str = "world";
const PAIR: str = "${HELLO}, ${WORLD}";

const RINGS = @builtin.str.raw {
    Three rings for the Elven-kings under the sky
}

true;
false;

.ok;
.error;

extend Integer {
    comptime fn px(self) -> Pixel = Pixel.from_integer(self);
}

const WIDTH = 100px;
```

---

## 与 Rust / Zig / C / C++ 的关键差异

| 点 | Flurry |
|----|--------|
| 数值字面量 | 类型为 `Integer` / `Float`；进入运行时走通用 splice（见 primitives） |
| 后缀 | 可 `extend`；`23u32` 即 `Integer.u32(23)` |
| 浮点写法 | 必须完整小数；`1e6` 不合法，须 `1.0e6` |
| 进制前缀 | 仅小写 `0b` / `0o` / `0x`；hex 字母大小写不得混用 |
| 字符串字面量 | 类型 `str`；插值只在 comptime |
| 原始字符串 | `@builtin.str.raw { … }` |
| 符号 | `.id`，类型 `Symbol` |
| `null` | 见 [unit-null-undefined.md](./unit-null-undefined.md)，不是本章的字面量 |

（上表右侧只锚定 Flurry 已确认行为；其它语言仅作读者背景，不作承诺。）

---

## 运行时实现设计引导

1. 字面量产生编译时值；进入运行时按该类型的 [Splice 规则](../comptime/quoting-and-splicing.md)处理。后缀仍按其目标函数的调用规则执行。
2. 可以直接表示为常量的结果按目标类型布局物化；需要运行时构造的结果保留相应转换实例，不能统一视为写入只读数据。
3. 符号保持编译单元内同名同值；比较可实现为标号相等。
4. 后缀在编译期已消除，运行时不再解析后缀语法。

---

## 编译器实现引导

1. **词法**：按本章规则识别整数（含进制与 `_`）、浮点（含科学计数法）、字符/字符串（含转义与 `$` 插值分段）、布尔关键字、符号的 `.` 与 `id`。
2. **语法**：字面量；`literal_suffix` 将「数值或字符串字面量 + 标识符」改写为 `Type.id(lit)`；失败则诊断。相邻字符串字面量做合并展开；含插值者改写为 `std.fmt.comptime_format`。`@builtin.str.raw` 按构造宏规则扩展。
3. **类型与 splice**：字面量带上表类型；进入运行时上下文时按该类型适用的通用 splice 规则处理（推导失败时无后缀数值按 `i32` / `f64`）。
4. **不变式**：字面量后缀语法不可叠两次；符号同名同值。

---

## 附录：产生式汇总

```text
literal -> integer | float | char | str | bool | symbol

integer -> decimal | binary | octal | hex
decimal -> digit (digit | _)*
binary  -> 0b (bindigit | _)*
octal   -> 0o (octdigit | _)*
hex     -> 0x (hexdigit | _)*

float -> digits . digits (exp)?
exp   -> (e | E) (+ | -)? digits

bool -> true | false
symbol -> . id

literal_suffix -> (integer | float | str) id

escape -> \\ | \' | \" | \n | \r | \t | \0 | \$ | \x{ hexdigit+ }

raw_str -> @builtin.str.raw { raw_source }
```
