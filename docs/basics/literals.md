# 字面量

## Intro

字面量是源代码里直接写出的固定值，也是表达式的基本构件。

每种字面量都有对应类型（如整数 → `Integer`，字符串 → `str`）。类型如何处在编译期 / 运行时宇宙、以及如何 **splice**，是语言的通用规则，见 [基本数据类型](./primitives.md)。无后缀时，整数 / 浮点在推导失败时按 `i32` / `f64` 处理。

整数、浮点、字符和字符串字面量后可跟标识符后缀，例如 `23u32`、`"msg"c`。后缀通过 `LiteralInto<suffix, Result>` 选择转换实现；结果类型由上下文与候选共同推断，见[语法协议](../traits/protocols.md#字面量转换)。

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

literal_suffix -> (integer | float | char | str) id

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

宇宙归属与 splice 见 [基本数据类型](./primitives.md)。名称通过相应包与 prelude 提供；自动与显式导入的范围见[内置包](../code-organization/builtin-packages.md)。

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

- 用于整数、浮点、字符和字符串字面量。
- `lit id` 以字面量类型 S 为主体，查询 `LiteralInto<.id, R> for S`，再调用所选 literal_into。
- R 由目标上下文与候选推断；没有唯一合法解时报告歧义，未决候选不能当作不存在。
- 只能加一次后缀：`100u32px` 中的 `u32px` 是一个标识符，不是两层后缀。
- 自定义后缀由相应 LiteralInto 实现提供；多个结果类型按不同完整接口实例区分。

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

```flurry
fn is_ready(tag: Symbol) -> bool {
    tag match {
        .ready => true,
        _ => false,
    }
}

comptime fn selected() -> Symbol { .ready }

test {
    let tag = .ready;
    assumes is_ready(tag);
    assumes tag != .waiting;
}
```

Symbol 已有独立的类型和值表示，可用于 runtime／comptime 调用、常量、模式和 normal-form 值参数。它可以复制并经指针读取；同名字符串仍不是同类型的值，也没有因引入 Symbol 而增加符号的大小次序。模式中的有限符号列表不能覆盖所有符号，通常需要 `_` 分支。

若调用、运算协议或其他输入能提供 enum 目标，编译器先取得该上下文，再解释 `.name`；上下文仍未决定时延迟字面量默认选择。最终没有 enum 目标时得到 Symbol。这保留了 `dispatch(.quit)` 等已有用法，而不把所有符号固化成某个 enum。normal-form 中的 Symbol 保留为值参数，不展开头，也不引入头的单射性。

`'tag_name` 视图也已接通：取得 Symbol 对应的 `str`，不会把符号本身变成字符串类型。操作数只求值一次；调用或提前退出仍按普通表达式次序执行。

```flurry
fn describe(tag: Symbol) -> str { tag'tag_name }
const ready_name: str = .ready'tag_name;
```

LLVM host 已支持 Symbol 的字面量、传参、相等比较、成员字面量模式、指针存取和名称视图。其私有存储共享只读文本，不授予与 str 的隐式转换，也不引入大小次序；库中的 Once/Clone/Copy 实现保留这种值语义。

这个视图可用于两阶段调用、常量、normal-form 的字符串参数和 staged 参数计算。所得字符串不依赖承载 Symbol 的局部对象继续存活。enum 值也支持名称视图：结果是声明中的变体名，分组变体包含组路径，与底层编码独立，具体见 [enum 表示](../type/user-defined-types/enums.md)。

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


```

---

## 与 Rust / Zig / C / C++ 的关键差异

| 语言 | 真实对应物与差异 |
|---|---|
| Rust | 数值后缀指定内建数值类型；普通字符和字符串字面量不能带自定义转换后缀。Flurry 通过 LiteralInto 选择完整转换实例。[Rust literals](https://doc.rust-lang.org/reference/expressions/literal-expr.html) |
| Zig | comptime 数值和显式类型转换可承担类型选择；无对等的 LiteralInto 后缀接口。[Zig](https://ziglang.org/documentation/master/) |
| C | 内建整数、浮点后缀选择对应数值类型；自定义构造可用函数或宏表达，没有 trait 后缀查询。[C11 §6.4.4](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| C++ | 用户定义字面量调用 literal operator；Flurry 的转换通过 LiteralInto 的主体、后缀和结果类型共同解析。[C++ literal operators](https://eel.is/c++draft/over.literal) |

---

## 运行时实现设计引导

1. 字面量产生编译时值；进入运行时按该类型的 [Splice 规则](../comptime/quoting-and-splicing.md)处理。后缀仍按其目标函数的调用规则执行。
2. 可以直接表示为常量的结果按目标类型布局物化；需要运行时构造的结果保留相应转换实例，不能统一视为写入只读数据。
3. 符号保持编译单元内同名同值；比较可实现为标号相等。
4. 后缀在编译期已消除，运行时不再解析后缀语法。

---

## 编译器实现引导

1. **词法**：按本章规则识别整数（含进制与 `_`）、浮点（含科学计数法）、字符/字符串（含转义与 `$` 插值分段）、布尔关键字、符号的 `.` 与 `id`。
2. **语法与协议**：`literal_suffix` 保留字面量及后缀身份，为 LiteralInto 建立结果推断变量；候选试探相互隔离，唯一选定后生成对应转换调用。相邻字符串字面量做合并展开；含插值者改写为 `std.fmt.comptime_format`。`@builtin.str.raw` 按构造宏规则扩展。
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

literal_suffix -> (integer | float | char | str) id

escape -> \\ | \' | \" | \n | \r | \t | \0 | \$ | \x{ hexdigit+ }

raw_str -> @builtin.str.raw { raw_source }
```
