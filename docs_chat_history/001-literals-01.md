# 001 — literals — 01

- **日期**: 2026-08-09
- **global_index**: 001
- **theme**: literals
- **subindex**: 01
- **本轮目标**: 评估作者提交的「Flurry 语言字面量详解」草稿；对照现行文档与 grammar；拆成可独立确认的命题并追问；默认不写入 `docs/`。

---

## 作者本轮输入摘要

作者提交了一份字面量总览草稿，要点包括：

1. 字面量是表达式基本单位；字面量类型直接对应 **comptime 宇宙** 中的类型。
2. 数值/字符串字面量末尾可加 `id`，对该 comptime 类型的 **extension 方法** 做 apply（例：`23u32` ↔ `Integer` 上的 `fn u32(self) -> builtin.u32`）；这是「字面量变体」的内置机制。
3. 整数：十进制 / `0o`·`0O` / `0x`·`0X` / `0b`·`0B`；可穿插 `_`；首字符须为数字或负号。
4. 浮点：小数点、科学计数法 `e`/`E`、后缀如 `f32`/`f64`、下划线规则同上。
5. 字符：单引号；转义；Unicode `\x{...}`。
6. 字符串：双引号；后缀 `c`（C 风格）、`b`（字节数组）；转义同字符；相邻字面量自动合并；编译时插值 `$id` / `${...}`；类型为 comptime `str`；原始字符串经 `@builtin.str.raw { ... }`。
7. 布尔：`true` / `false`。
8. 符号：写作「以 `symbol -> .` 开头，后跟 `id`」，示例为 `symbol -> .id`。

未授权写入正式文档。

---

## 可行性评估与关键澄清

### 总评

- **方向可行且重要**：把「类型后缀」提升为「对 comptime 字面量类型做 extension apply」比现行 `docs/basics/literals.md` 把 `u64`/`f32` 写成固定类型后缀更一致，也与 `docs/traits/index.md` 的「字面量拓展脱糖为 `literal.suffix()`」、以及 `docs/comptime/inlining-comptime-code.md`「裸字面量是 comptime `Integer` 再沉降」相合。
- **草稿形态**：偏教材散文 + 营销总结；若定稿应按 AGENTS 七段骨架重写（Intro / 语法 / 正式定义 / 实例 / 跨语言差异 / 运行时引导 / 编译器引导）。本轮不代写定稿。
- **禁止臆造**：下文凡标「现行文档写法」仅作冲突对照，不升格为已确认 Flurry 事实。

### 与现行材料的冲突（待作者裁决，不擅自选边）

| # | 主题 | 作者草稿 | 现行 `docs/basics/literals.md`（及其他） |
|---|------|----------|------------------------------------------|
| C1 | 数值后缀模型 | comptime 类型上的 extension 方法 | 写成「类型后缀」列表；默认无后缀 → `i32` / `f64` |
| C2 | 后缀挂载点 | 例：`Integer.u32(self)` | `traits/index.md` 示例为 `extend u32 { fn px }` 且 `100px` ≡ `(100u32).px()`（与「先 u32 再 px」链式关系未说清） |
| C3 | 原始字符串 | `@builtin.str.raw { ... }` | `builtin.raw_str'{...}`（另：OLD-2024 亦为 `builtin.raw_str`） |
| C4 | 符号表面语法 | 示例写成 `symbol -> .id` | 表面为 `.id`；grammar：`symbol -> .id`（产生式名被误写成表面语法） |
| C5 | 插值 | `$id` 与 `${}` 皆可，id 可省括号 | 仅写 `$identifier` |
| C6 | `null` 等 | 未提 | 现行有 `null`；grammar 另有 `()`、`_`、`undefined`、`self_cap`、`self_lower` |
| C7 | 浮点 comptime 名 | 「Float」 | `primitives.md` 编译期类型为 `Real` |
| C8 | 宏调用 `@` | `@builtin.str.raw` | 现行 raw 示例无 `@`；`@` 是否为宏/属性表面语法未在本主题确认 |
| C9 | 与 OLD-2024 | 结构接近 OLD-2024 `core/basics/literals.md` | 现行 `docs/basics/literals.md` 更短；归档是否废弃须问 |

### 草稿内示例的语法/风格自检问题（未改写语义）

- 符号示例 `symbol -> .id`：若意图是表面语法，不合 grammar 的 `.id`；若意图是引用产生式，不应放进「示例代码」。
- 常量名 `word` / `lyris`：风格指南常量倾向 `SCREAMING_SNAKE_CASE`（是否强制待确认）。
- 注释应使用 `--`（草稿部分已用，部分中文说明未用注释形态）。
- 合并示例 `"say you" " ," "say me\n"` 意向可读，但需确认空白与逗号是否故意。
- 负号：`-242.32f32` 与 grammar「`-` 紧贴非空白 → 非中缀」及 `negative -> -expr` 的关系须定义（字面量含符号 vs 一元负）。

### 机制健全性风险（后缀 = extension）

若采纳「裸字面量 ∈ comptime 类型 + 后缀 = 方法 apply」：

1. **词法 vs 解析**：`23u32` 是单一 token、还是 `23` + 后缀 `u32` 的 postfix？与普通 `expr.id` / `expr()` 优先级如何？（OLD/现行都暗示很高优先级的「字面量拓展」。）
2. **查找范围**：只在字面量类型的固有/prelude extension 中找，还是任意 `use` 进来的 `extend` 都可当后缀？
3. **与沉降**：无后缀 `23` 在运行时上下文如何变成 `i32`——默认 extension、上下文推断、还是单独 lowering 规则？这与现行「默认 i32」叙述可能互斥。
4. **字符串 `c`/`b`**：是否同为 `str` 上的 extension？返回类型与编码（UTF-8？嵌入 `\0`？非法字节？）未定义。
5. **链式后缀**：`100u32px` 是否合法？traits 示例暗示可能存在。

---

## 需要你拍板的问题

### 文档范围与权威源

1. **本轮主题确认**：是否以 `literals` 为 theme，目标是重写/补全 `docs/basics/literals.md`（七段骨架），符号细节仍主要放 `symbols.md`？
   - A. 是：字面量章管表面形态 + comptime 类型 + 后缀机制；符号章管语义与用途。
   - B. 字面量章写全符号正式定义，`symbols.md` 降为索引/删并。
   - C. 先只定「后缀/comptime 模型」短文，整数进制等 downstream 后补。
   - Other: …

2. **`docs/OLD-2024/core/basics/literals.md` 地位？**
   - A. 已废弃，仅作考古；冲突一律以你本轮确认 + 现行 `docs/` 为准。
   - B. 仍有部分条款有效；请逐条标注保留项。
   - C. 整篇升格意向，现行短文应被替换。
   - Other: …

3. **现行 `docs/basics/literals.md` / `primitives.md` / `traits/index.md` 中与草稿冲突的条款，在你确认前是否一律视为「未确认草稿」？**
   - A. 是：现行 basics 文档也不是最终权威，逐条重确认。
   - B. 否：现行已写的（如默认 `i32`、`null`、`.ok`）视为已确认，只增量改后缀模型。
   - C. 分文件：grammar + style 已确认；basics 叙述未确认。
   - Other: …

### 字面量与 comptime 类型

4. **每种字面量对应的 comptime 类型名？**
   - A. 整数→`Integer`，浮点→`Real`，字符串→`str`，字符→`char`，布尔→`bool`，符号→`Symbol`（对齐 `primitives.md`）。
   - B. 浮点 comptime 叫 `Float`（按你草稿），`Real` 废弃或另指。
   - C. 字符也是某代数量（例如 `Char`/`Codepoint`），运行时再沉降为 `char`。
   - Other: …

5. **无后缀数值字面量在运行时上下文的沉降规则？**
   - A. 固定默认：整数→`i32`，浮点→`f64`（现行叙述）。
   - B. 仅上下文推断；无上下文则编译错误（无默认）。
   - C. 保持为 `Integer`/`Real` 直到必须具化；具化规则另表（含常量泛型等）。
   - D. 隐式调用 prelude 的默认 extension（如 `.i32()` / `.f64()`），表面可省略。
   - Other: …

6. **「字面量类型直接对应 comptime 宇宙类型」是否包含布尔、字符、符号、`null`？**
   - A. 全部都是 comptime 值（可沉降或仅存在于 U₁）。
   - B. 仅数值与 `str` 是「代数字面量」；`true`/`false`/`'x'`/`.id` 直接是运行时/具体类型。
   - C. 符号严格 comptime；布尔/字符运行时；数值/`str` 代数字。
   - Other: …

### 后缀 / 字面量变体机制

7. **`23u32` 的正式脱糖？**
   - A. `(23 : Integer).u32()`（或等价：对字面量值调用 `u32`）。
   - B. `Integer.u32(23)` 类型关联函数形态。
   - C. 词法级类型后缀（不经方法查找）；用户 `extend` 另用不同语法。
   - D. A，但方法必须来自特定 prelude extension，用户不可覆写同名。
   - Other: …

8. **用户自定义字面量后缀（如 `100px`）与内置 `u32`/`f64`/`c`/`b` 是否同一机制？**
   - A. 同一：都是字面量类型上的 extension 方法。
   - B. 内置后缀是词法/特判；用户后缀才是 `extend`。
   - C. 同一机制，但内置名在词法上保留字/不可被用户 shadow。
   - Other: …

9. **后缀方法的接收者类型与查找？**（对照 traits 示例张力）
   - A. 始终在**裸字面量**的 comptime 类型上查找（`Integer.px`），`100px` 不必先写 `u32`。
   - B. 允许链式：`100u32px` = `(100.u32()).px()`，`px` 定义在 `u32` 的 extend 上（贴近现行 traits 文）。
   - C. 只允许单后缀；链式非法。
   - D. A 与 B 都合法，按最长/贪心匹配。
   - Other: …

10. **后缀与标识符的词法边界**：`0xFFu64`、`0xFF_u64`、`23u32` 哪些合法？
    - A. 数字与后缀之间**可选** `_`，两者皆法。
    - B. **必须** `_` 分隔（避免与 hex 字母歧义时强制）。
    - C. 十进制可贴后缀；hex/bin/oct **必须** `_` 或禁止字母开头的后缀紧贴。
    - D. 后缀必须是已知集合，词法器最长匹配，无需 `_`。
    - Other: …

11. **字符串后缀 `c`、`b` 是否同为 `str` 的 extension？返回类型？**
    - A. 是；`c`→某 C 字符串类型（请命名），`b`→`Slice<u8>` 或等价。
    - B. 是；但 `b` 要求字面量内容为字节可解释子集，非法 Unicode 码点/逃逸则静态错误。
    - C. 否：特判变体，不是方法。
    - Other: …（请给出确切类型名）

### 整数 / 浮点表面语法

12. **进制前缀大小写**：`0b`/`0B`、`0o`/`0O`、`0x`/`0X`？**
    - A. 大小写均合法（草稿 + OLD）。
    - B. 仅小写 `0b`/`0o`/`0x`。
    - C. 前缀小写；hex 数字字母 a–f 大小写均法。
    - Other: …

13. **下划线规则精确化**：除「首字符不能是 `_`」外，是否禁止首尾 `_`、连续 `__`、前缀后立刻 `_`（如 `0x_FF`）、后缀前 `_`？
    - A. 仅忽略数字之间的 `_`；禁止前导/尾随/连续（类 Rust）。
    - B. 更宽松：除前导外任意位置可 `_`。
    - C. 按你草稿：首字符数字或负号即可，其余不限。
    - Other: …

14. **负号是字面量一部分还是一元 `negative`？**
    - A. 一律 `negative` 作用于正字面量（`-` 紧贴，符合空白规则）。
    - B. 词法上存在负数字面量 token。
    - C. 语义等价，实现任选，文档写 A。
    - Other: …

15. **浮点形式**：是否允许 `1.`、`.5`、`1e6`（无小数点）？
    - A. 必须有小数点，或必须有 `e`/`E` 之一（OLD 倾向「点或指数」）。
    - B. 允许 `1.`；不允许 `.5`。
    - C. `1.`、`.5`、`1e6` 皆法。
    - D. 仅 `digits.digits` 与带指数形式；无 `1.`/`.5`。
    - Other: …

16. **超出目标位宽 / 非整表示**：`300u8`、`1.0u32`（若有人乱写后缀）？
    - A. 静态错误（方法实现内检查或类型系统）。
    - B. 截断（须标 UB 或实现定义——通常不推荐）。
    - C. 仅 extension 方法文档约定，语言不特判。
    - Other: …

### 字符 / 字符串

17. **转义表最终集合？**
    - A. `\\` `\'` `\"` `\n` `\t` `\r` `\0` + `\x{hex}`。
    - B. A 再加 `\u{...}`（与 `\x{...}` 二选一或同义）。
    - C. 保持最小集：草稿列举的 + `\x{}`；其余以后再加。
    - Other: …（请列完整表）

18. **`\x{...}` 约束**：码点位数、是否必须是 Unicode 标量值（禁止 surrogate）、字符字面量是否恰好一个标量？
    - A. 1–6 位 hex；必须是标量值；字符字面量 = 恰好一个标量（现行/OLD 方向）。
    - B. 允许任意到 U+10FFFF，含非标量则静态错误或运行时 trap（选一）。
    - C. 字符是 UTF-8 码位/字节，不是标量（更偏 C）。
    - Other: …

19. **字符串插值形态？**
    - A. 仅 `$id`（简单标识符）。
    - B. `$id` 与 `${expr}`（expr 须 comptime 可求值）。
    - C. 仅 `${...}`；`$id` 是糖。
    - D. 插值只允许常量/`comptime` 绑定，禁止任意 expr。
    - Other: …

20. **插值与合并的阶段**：相邻 `"a" "b"` 合并、插值展开，何者先？能否跨拼接插值？
    - A. 先合并再插值（一个逻辑字面量）。
    - B. 先各自插值再合并。
    - C. 禁止插值串与非插值串混拼（静态错误）。
    - Other: …

21. **原始字符串表面语法？**（冲突 C3）
    - A. `@builtin.str.raw { ... }`（你草稿；确认 `@` 与路径）。
    - B. `builtin.raw_str'{...}`（现行）。
    - C. `builtin.str.raw'{...}`（路径折中 + take-view/宏调用形态）。
    - D. 专用词法 raw string（无宏），另定分隔符。
    - Other: …

22. **raw 内容边界**：首尾换行是否剥除？`}` 如何放入内容？是否支持自定义定界符？
    - A. 花括号内原样；首尾换行保留；内容若需 `}` 则改用定界符形式（待定语法）。
    - B. 剥除紧随 `{` 后与 `}` 前的单个换行（类 Zig）。
    - C. 仅单行 `raw'...`；多行另议。
    - Other: …

### 其它 literal 产生式

23. **`null` / `()` / `_` / `undefined` 是否写入字面量章？**
    - A. 写入；并给出类型与合法上下文。
    - B. 仅 `null` + 布尔 + 本节已列类型；`()`/`_`/`undefined` 放到别章。
    - C. grammar 有的都在字面量章给一行定义 + 链到专章。
    - Other: …

24. **符号表面语法确认**：是否即为 `.id`（grammar `symbol -> .id`）？
    - A. 是；文档示例只写 `.ok`，不写产生式名。
    - B. 另有 `symbol .id` 或其它糖（请写产生式）。
    - C. 符号不是 literal，是 prefix expr（grammar 已如此）——字面量章只交叉引用。
    - Other: …

### 语法段（若写入）应有的产生式草案方向

25. **是否在字面量章「语法」段补充 integer/float/char/str 的细产生式**（全局 `syntax` 目前只写到 `literal -> integer | float | ...`）？
    - A. 本章给局部增量；确认后回灌 `docs/grammar/syntax`。
    - B. 只写在 `syntax`；本章用文字 + 示例。
    - C. 两处都写，以 `syntax` 为准。
    - Other: …

---

## 已确认 / 仍待定

### 已确认（本轮）

- 无。作者提交的是待审草稿，未对上述拍板项作答；亦未授权改 `docs/`。

### 从现行文档读取到、但按问题 3 可能降级的「候选已有叙述」（非正式确认）

- grammar：`literal` 含 integer/float/char/str/bool/`()`/`_`/`null`/`undefined`/…；`symbol -> .id` 在 prefix。
- 风格：`--` 注释；常量 `SCREAMING_SNAKE_CASE`；无 `&&`/`||`。
- 多份文档提及：字符串字面量类型为 comptime `str`；裸整数可为 comptime `Integer` 并沉降。

### 待定

- 后缀 = extension 的精确定义、查找、链式、词法边界。
- 默认沉降 vs 推断。
- 原始字符串与 `@`。
- 插值 `${}`。
- `null` 等是否入章。
- Float vs Real。
- 转义完整表与 `\x{}` 约束。
- OLD-2024 与现行短文的权威关系。
- 文档写入授权。

---

## 文档写入

- **否**（默认）。未收到「可以写进文档」授权。
- 提醒：确认后目标路径倾向 `docs/basics/literals.md`，并可能联动 `docs/grammar/syntax`、`docs/basics/primitives.md`、`docs/traits/index.md` 中冲突段落——均须你显式点名后才改。

---

## DRAFT — NOT CANONICAL

本轮不提供「建议直接粘贴进正式文档」的完整七段草案。待问题 4–11（comptime 类型 + 后缀机制）拍板后，下一轮可给：

1. Intro 短段；
2. 局部产生式（integer/float/…）；
3. 正式定义：脱糖规则、沉降、错误条件；
4. 自检过的示例。

**机制方向备忘（仍为草案，非 canon）**：

```text
DRAFT — NOT CANONICAL
裸整数字面量 : Integer
23u32  ~>  apply extension method u32 to literal 23
无后缀在 runtime 上下文中的具化规则：待确认（问题 5）
```

---

## Flurry 示例自检说明

本 history **未**把作者草稿中的示例当作已确认合法代码转载为推荐写法。指出的不合项：符号示例形态、raw 宏路径、常量命名风格；正式示例须在拍板后按 `docs/grammar/syntax` 与 `docs/grammar/index.md` 重写并再自检。

---

## 作者答复（prompt）

日期：2026-08-09

来源：仓库根目录 `prompt`（全文照录）

```
1、可以；B
2、A
3、A
4、flurry除了依赖包的外所有符号均在builtin、meta、std、attrs、math、verification 6个根模块和当前包中。其中std.prelude.*默认导入到当前包根作用域。你所指的这些符号属于当前包
    所有字面量对应的comptime类型都在builtin下，因为它们都有runtime的代理类型，这涉及comptime to runtime splicing（“沉降”已更名为更符合2ltt描述的splice机制）机制后面会讲。

5、类型推导系统对此有拓展，推导不出时，A
6、null不是。null是option type的null分支构造表达式，我也不知道是否该叫它字面量，我想是不该
7、B，所以用户也可以定义 extend Integer { comptime fn px(self) -> my_lib.Px = Px.from<Integer>(math.abs(self.u32())) }，这样flurry就可以识别32px了
8、A, extension是implementation、derivation那样的机制，后缀是去糖到函数调用的机制，因此后缀只限定于对数字或者字符串字面量使用。普通包中通过extension定义，
    而builtin中就直接定义implementaion了，用户接触不到，所以叫extension后缀，这在用户使用场景中更容易理解
9、C，显然你加了后缀之后，这个表达式就不是字面量了，因此不能嵌套使用，而且b中的例子显然会把`u32px` lex为一个id
10、根据lex的行为，23_____u32会被lex为int("23_____")和id("u32")，前者中允许任意位置和长度的下划线
11、都说了是extension后缀
12、前缀仅允许小写，hex部分要么全大写，要么全小写
13、根据lex的行为，"_23"会被识别为下划线和一个int字面量
14、A，在flurry中，`-`和` - `是两个不同的token类型，也只有前者才能作为negative表达式的前缀
15、D
16、由extension/implementation中的具体的构造函数定义。期望的行为是，在数据不合规格时直接调用meta.compilation.emit_err(...)
17、除却这里提到的规则外，从c、rust、swift中选取适合flurry风格的转译规则，给出全表，str和char通用
18、A
19、B。再次提醒字符串差值只能在comptime宇宙使用，也就是说只能插comptime值
20、`"${"hello"}, ${"world"}"`脱糖为`std.fmt.comptime_format("{}, {}", "hello", "world")`
21、A，最新宏系统在docs/macro中介绍较完整，这是最新的宏调用形式。
22、对于四类构造宏来说（比如builtin.str.raw），flurry会将两个`{``}`token之间的所有源码字节直接交由宏处理函数
23、独立到新文件中，但不用新文件夹，用户会在寻找这些构造表达式时来翻阅字面量文档，即使他们不是字面量，这时候他们会看到的
24、符号表达式就是一个`.`token和一个id token
25、所有产生式写到文件末尾，以附录的形式
```

另：作者要求修改 `AGENTS.md`：经由 `prompt` 的回复须附加到对应 history 文件下面（已执行）。
