# 002 — literals — 02

- **日期**: 2026-08-09
- **global_index**: 002
- **theme**: literals
- **subindex**: 02
- **本轮目标**: 消化作者对 001 问题的 `prompt` 答复；锁定已确认项；暴露 7/脱糖与 4/路径等矛盾；给出转义全表候选；继续追问；**不**写入正式 `docs/` 字面量章（未授权）。已按作者要求更新 `AGENTS.md`（`prompt` 答复落盘规则）。

---

## 作者本轮输入摘要

作者在 `prompt` 中逐条回答了 001 的 1–25，并要求：`prompt` 中的回复附加到对应 history；修改 `AGENTS.md` 固化该流程。

全文已追加到 [`001-literals-01.md`](./001-literals-01.md) 末尾「作者答复（prompt）」；下文为解读与追问，不改写其技术立场。

---

## 可行性评估与关键澄清

### 机制图景（按作者答复归纳，待个别矛盾消除后升格）

1. **后缀**：仅适用于**数字或字符串字面量**；解析上去糖为对该字面量 comptime 类型上的方法调用；用户侧通过 `extend` 定义，`builtin` 内用用户不可见的 implementation。
2. **单后缀**：加后缀后不再是字面量，故不可嵌套；`100u32px` 会把 `u32px` 收成一个 `id`（非法/非预期链式）。
3. **词法**：整数字面量 token 内 `_` 任意位置与长度；`"_23"` → `_` + int；后缀是**随后的 `id` token**（例：`23_____u32` → int + id）。
4. **负号**：`-`（紧贴）与 ` - `（中缀）为不同 token；仅前者作 `negative` 前缀。
5. **浮点形态**：选 D——不允许 `1.` / `.5`；须 `digits.digits` 或合规科学计数法形态（细节仍须钉死指数部分）。
6. **进制**：前缀仅小写 `0b`/`0o`/`0x`；hex 数字字母要么全大写要么全小写。
7. **越界/不合规**：由具体 extension/implementation 构造逻辑处理；期望调用 `meta.compilation.emit_err(...)`。
8. **插值**：`$id` 与 `${expr}`；仅 comptime 宇宙、仅插 comptime 值；示例脱糖到 `std.fmt.comptime_format`。
9. **raw**：确认 `@builtin.str.raw { ... }` 为四类构造宏；`{`/`}` 之间源码字节交给宏（与 `docs/macro/construction-macro.md` 一致）。
10. **null**：不是字面量；是 option 的 null 分支构造表达式；与 `()`/`_`/`undefined` 等进**同目录新文件**，字面量文档中提供导引。
11. **符号**：`.` token + `id` token；字面量章写全符号正式定义，`symbols.md` 降为索引/删并（问题 1 = B）。
12. **术语**：原「沉降 / lowering」→ **splice**（comptime→runtime，2LTT 叙事）；细节后话。
13. **模块根**：`builtin`、`meta`、`std`、`attrs`、`math`、`verification` + 当前包；`std.prelude.*` 默认导入当前包根；字面量 comptime 类型在 **`builtin`**，并有 runtime 代理类型。

### 必须追问的矛盾 / 缺口

#### 矛盾 X1 — 问题 7 选项 B vs 示例（`self` 方法）

作者选 **7-B**（`Integer.u32(23)` 类型关联形态），但紧接着的合法示例是：

```text
extend Integer {
    comptime fn px(self) -> my_lib.Px = Px.from<Integer>(math.abs(self.u32()))
}
-- 识别 32px
```

这是 **带 `self` 的实例方法**，脱糖更接近 **7-A：`(32).px()`**，且内部再 `(self).u32()`。  
若正式脱糖是 `Integer.px(32)`，则 `self` 只是定义糖，须写明。  
**不能在文档里同时写「关联函数 B」与「后缀 = 对字面量调用方法」却不统一。**

#### 缺口 X2 — 问题 4 未给出确切类型路径/名称

已知：comptime 字面量类型在 `builtin`，有 runtime 代理，splice 后话。  
未确认：`Integer`/`Real`/`str`/`char`/`bool`/`Symbol` 的**规范路径**（`builtin.Integer`？ prelude 再导出为 `Integer`？）、浮点 comptime 名是 `Real` 还是其它。

#### 缺口 X3 — 相邻字符串合并

原稿有 `"a" "b"` 自动合并；问题 20 作者只给了插值脱糖，未回答合并是否仍存在、与插值如何共存。

#### 缺口 X4 — `23u32` 无下划线时的词法

已说明 `23_____u32` → int + id。  
未说明：无空白、无 `_` 的 `23u32` 是否同样拆成 int(`23`)+id(`u32`)（强烈预期为是，需一句确认）。

#### 缺口 X5 — `c`/`b` 的返回类型名

已确认是 extension 后缀；仍缺 `builtin`/`std` 中的**返回类型规范名**与编码/`\0` 约定。

#### 缺口 X6 — 插值示例自检

作者示例：`"${"hello"}, ${"world"}"`  
- 外层是否为字符串字面量？若是，内层 `"hello"` 的引号如何嵌套——是否应写成 `"${\"hello\"}, ${\"world\"}"`，或 `${}` 内是表达式所以写 `${"hello"}` 而外层用另一种引号策略？  
- 脱糖为 `std.fmt.comptime_format("{}, {}", "hello", "world")`：格式串与参数的对应规则、`$id` 如何脱糖，待补。

#### 文档冲突（记录，确认写入前再改）

| 项 | 作者确认方向 | 现行文档 |
|----|--------------|----------|
| raw | `@builtin.str.raw` | `basics/literals.md` 仍为 `builtin.raw_str'…` |
| 符号章 | 并入 literals，symbols 降级 | 独立 `symbols.md` |
| null | 非字面量，另文件 | literals 含 null |
| 后缀挂载 | `extend Integer` + 单后缀 | traits 示例 `(100u32).px()` 链式 |
| 术语 splice | 取代沉降 | comptime 文仍写沉降 |

### 转义全表候选（问题 17，DRAFT — NOT CANONICAL）

约束：char 与 str 共用；保留已确认的 `\x{hex}`（标量值，1–6 hex）；不引入 octal；不引入 `\a\b\f\v`（噪声大、C 遗物）；Flurry 无位运算符风格偏好 → 不强调字节级 `\xHH` 于普通 str（字节串 `b` 另议）。

| 序列 | 码点/含义 | 取舍理由 |
|------|-----------|----------|
| `\\` | U+005C | C/Rust/Swift 共通 |
| `\'` | U+0027 | 共通；char 必需 |
| `\"` | U+0022 | 共通；str 必需 |
| `\n` | U+000A | 共通 |
| `\r` | U+000D | 共通 |
| `\t` | U+0009 | 共通 |
| `\0` | U+0000 | C/Rust/Swift；利於 `"..."c` |
| `\x{H…H}` | Unicode 标量（1–6 hex） | 已确认；**不**再设 `\u{…}` 别名（避免双语法） |

**刻意不纳入（待你否决或放行）：**

| 序列 | 来源 | 不纳入理由（候选） |
|------|------|-------------------|
| `\u{…}` | Rust/Swift | 与 `\x{…}` 重复 |
| `\xHH`（恰两位数） | Rust byte / C | 与 `\x{…}` 易混；若需要仅限 `"…"b` |
| `\nnn` octal | C | 与进制字面量心智冲突 |
| `\a` `\b` `\f` `\v` `\e` | C / 部分 | 少用；可用 `\x{…}` |
| `\s` `\p{…}` 等 | 非字面量传统 | 不属于字面量转义 |

**非法转义**：未知 `\X` → 静态错误（调用/等同 `emit_err`），还是保留为字面 `\`+`X`？待拍板。

---

## 需要你拍板的问题

1. **统一 `23u32` / `32px` 的脱糖（消解 X1）？**
   - A. 后缀 `lit id` ~> `(lit).id()`（实例方法；`self` 即字面量值）。定义可写在 `extend Integer` / `extend builtin.str`。
   - B. 后缀 ~> `ReceiverType.id(lit)`（关联函数；定义无 `self` 或 `self` 仅糖）。
   - C. 两种表面定义都合法，脱糖规范为 A，类型检查按方法解析。
   - Other: …

2. **comptime 字面量类型的规范路径与名称？**
   - A. `builtin.Integer` / `builtin.Real` / `builtin.str` / `builtin.char` / `builtin.bool` / `builtin.Symbol`，并由 `std.prelude` 导出短名。
   - B. 同上但浮点 comptime 不叫 `Real`（请给名）。
   - C. 短名即规范名（就叫 `Integer`），仅实现上住在 `builtin` 模块。
   - Other: …（请列六张表：字面量种类 → comptime 类型 → runtime 代理）

3. **无空白的 `23u32` 词法是否恒为 `int("23")` + `id("u32")`？**
   - A. 是（字母开始处切开；与 `23_____u32` 同一规则）。
   - B. 否（请给 lex 规则）。
   - Other: …

4. **后缀 apply 的语法位置**：`id` 必须与字面量**相邻无空白**？
   - A. 必须紧贴（`23 u32` 不是后缀，而是两表达式/非法并置）。
   - B. 允许空白（弱化「后缀」观感）。
   - C. 紧贴；且仅当 `id` 在字面量类型的可见方法集中才改写，否则报错（而非当成并置）。
   - Other: …

5. **`"…"c` / `"…"b` 的返回类型与语义？**
   - A. 请你直接填：`c` → `________`；`b` → `________`；编码/是否保证尾 `\0`/是否要求 UTF-8。
   - B. 暂定名进入文档为待定，只写「extension 返回值由 builtin 定义」。
   - Other: …

6. **相邻 `"a" "b"` 自动合并是否仍存在？**
   - A. 是；仅空白分隔的纯字符串字面量合并；再作为整体插值/类型检查。
   - B. 是；但含 `$`/`${}` 的串**不**参与合并。
   - C. 否；废除合并，只保留 `++` 或 `comptime_format`。
   - D. 合并是 `comptime_format`/宏的表面糖，不另做词法合并。
   - Other: …

7. **插值表面形式与脱糖（钉死 Q19–20）？**
   - A. `"…$id…"` ~> `comptime_format("…{}…", id)`；`"…${e}…"` ~> `comptime_format("…{}…", e)`；多段按出现顺序变参。
   - B. 整段字符串若含任意插值，整串重写为单次 `comptime_format`，静态文本成为格式串字面量（需转义 `{}` 规则）。
   - C. 其它（请给 2–3 个具体前后例子，含嵌套引号）。
   - Other: …

8. **格式串中字面 `{`/`}` 如何写？**
   - A. `{{` / `}}`（Rust 风格，经 comptime_format）。
   - B. 禁止在插值字符串中出现未转义花括号，静态文本用 `\x{7B}` 等。
   - C. 只有 `${` 启动插值，单独 `{` 仍字面（与脱糖实现需一致）。
   - Other: …

9. **转义全表是否采纳上文候选 8 项 + 拒绝 `\u`/`\xHH`/octal/ `\a…`？**
   - A. 采纳该表；未知转义 = 静态错误。
   - B. 采纳该表；另增 ……（请列）。
   - C. 采纳；但对 `"…"b` 额外允许 `\xHH` 字节逃逸。
   - D. 另给全表。
   - Other: …

10. **科学计数法精确形态（在 15-D 下）？**
    - A. `digits.digits` 与 `digits.digits[eE][+/-]?digits`；指数前必须有小数点部分（即禁止 `1e6`）。
    - B. 允许 `digits[eE][+/-]?digits`（无小数点的 `1e6`），仍禁止 `1.` / `.5`。
    - C. 允许 `digits.[eE]…`（`1.e3`）——一般不推荐，是否禁止？
    - Other: …

11. **hex 字母「全大写或全小写」是否含前缀？** `0x` 已小写；`0xFFff` 非法；`0xFF_ff`？
    - A. 仅 a–f/A–F 主体一致；`_` 忽略后再检查。
    - B. 每个「分段」可不同（不推荐）。
    - C. 其它。
    - Other: …

12. **符号章并入后的文件布局？**
    - A. `docs/basics/literals.md` 含符号七段；`symbols.md` 只留跳转。
    - B. `literals.md` + 同目录 `symbol-literal.md`；原 `symbols.md` 重定向。
    - C. 删并 `symbols.md` 进 `literals.md`，README 改链。
    - Other: …

13. **非字面量构造表达式新文件名？**（null / `()` / `_` / `undefined` / …）
   - A. `docs/basics/special-constructors.md`
   - B. `docs/basics/unit-null-undefined.md`
   - C. `docs/basics/construct-exprs.md`
   - Other: …

14. **`true`/`false` 是否仍算「字面量」章内容？**（bool 有 comptime/runtime 代理）
    - A. 是，字面量章收录。
    - B. 否，与 null 一并进特殊构造文件。
    - Other: …

15. **现行 `traits/index.md` 中 `100px` ≡ `(100u32).px()` 示例？**
    - A. 确认为错误示例；改为 `extend Integer { fn px(self)…}` + `100px`。
    - B. 保留另一种产品语义（与本轮单后缀冲突，需解释）。
    - Other: …

16. **术语替换范围（沉降→splice）？**
    - A. 本主题文档只用 splice；其它章待你开专题再改。
    - B. 一俟写入 literals，同时改 `comptime/inlining-*.md` / `primitives.md` 中「沉降」字样。
    - C. 两词并存一段时间，括号注明旧称。
    - Other: …

17. **本轮是否授权开始写 `DRAFT` 七段进 history（仍不改 `docs/`）？**
    - A. 是，在 1–6 等关键项答完后的下一轮给全文草案。
    - B. 是，现在就基于已确认项出一版残缺草案（未确认处标待定）。
    - C. 否，继续只问答。
    - Other: …

---

## 已确认 / 仍待定

### 已确认（升格自作者 prompt；标争议者除外）

| ID | 结论 |
|----|------|
| T1 | 主题 `literals`；目标重写 `docs/basics/literals.md`；符号正式定义并入字面量章，`symbols.md` 降为索引/删并（1B）。 |
| T2 | OLD-2024 literals 废弃（2A）。 |
| T3 | 现行 basics 等叙述须逐条重确认（3A）。 |
| T4 | 六根模块 + 当前包；`std.prelude.*` 默认导入；字面量 comptime 类型在 `builtin`，有 runtime 代理；沉降更名为 **splice**（细节后话）。类型短名路径见问题 2。 |
| T5 | 无后缀数值：推导优先，推导不出默认整数 `i32`、浮点 `f64`（5A + 推导拓展）。 |
| T6 | `null` 不是字面量；是 option null 分支构造；另文件记录并在字面量章导引（6, 23）。 |
| T7 | 用户可用 `extend Integer { … fn px(self) …}` 得到 `32px`；后缀限数字/字符串字面量；builtin 用 implementation，对用户称 extension 后缀（8）。**脱糖 A vs B 见问题 1。** |
| T8 | 不可嵌套后缀；`u32px` 会 lex 成单一 id（9C）。 |
| T9 | 整数字面量 token 内 `_` 任意；`_23` → `_` + int（10, 13）。 |
| T10 | 前缀仅 `0b`/`0o`/`0x`；hex 字母全大或全小（12）。 |
| T11 | 负号：紧贴 `-` 为 negative token（14A）。 |
| T12 | 浮点：禁止 `1.` / `.5`（15D）；指数细节见问题 10。 |
| T13 | 不合规模态 → 实现内 `meta.compilation.emit_err`（16）。 |
| T14 | `\x{…}`：1–6 hex、标量值、char=单标量（18A）；转义全表待确认候选。 |
| T15 | 插值 `$id` 与 `${expr}`；仅 comptime 值（19B）；脱糖涉及 `std.fmt.comptime_format`（20，细则待问）。 |
| T16 | raw = `@builtin.str.raw {…}` 四类宏；括号间原始字节（21A, 22）。 |
| T17 | 符号 = `.` + `id`（24）。 |
| T18 | 产生式放本章文末附录（25）。 |
| T19 | `AGENTS.md` 已加：`prompt` 答复须追加到被回答的 history，再开新轮次。 |

### 待定

- 脱糖确切形态（问题 1 / X1）
- 类型规范路径表（问题 2）
- `23u32` 紧贴切开、空白规则（问题 3–4）
- `c`/`b` 返回类型（问题 5）
- 相邻串合并（问题 6）
- 插值脱糖与 `{}` 转义（问题 7–8）
- 转义全表拍板（问题 9）
- 科学计数法（问题 10）
- hex 一致性与 `_`（问题 11）
- 文件布局与 bool 归属（问题 12–14）
- traits 错例（问题 15）
- splice 替换范围（问题 16）
- 是否开始草案（问题 17）

---

## 文档写入

- **正式 `docs/`**：否（未授权）。
- **`AGENTS.md`**：是（作者本轮明确要求；已写入 `prompt` 答复落盘规则）。
- **history**：`001` 已附作者答复；本文件为 002。

---

## DRAFT — NOT CANONICAL

### 后缀（残缺，待问题 1）

```text
suffix_apply -> (integer | float | str) id   -- 语义层；词法为 literal_token 后紧邻 id
-- 脱糖：待确认 (lit).id()  vs  Type.id(lit)
-- 单次：结果不是字面量，不可再 suffix_apply
```

### 转义表

见上文「转义全表候选」；未确认前不得写入正式文档。

### 插值

```text
-- 作者举例（嵌套引号表面形式待确认）：
"${"hello"}, ${"world"}"
  ~> std.fmt.comptime_format("{}, {}", "hello", "world")
```

### 示例自检说明

本轮引用作者 `extend Integer` / `32px` / `@builtin.str.raw` 作为**设计意向**；在问题 1–5 拍板前不作为正式文档示例定稿。`px` 示例中 `math.abs`、`Px.from<Integer>` 的路径与风格未做完整 grammar 审计（依赖未确认 API）。

---

## 作者答复（prompt）

日期：2026-08-09

来源：仓库根目录 `prompt`（全文照录）

```
1 你就是方法调用最后也是脱糖成b的形式，而字面量本身已经有了对应的comptime类型，直接用b的形式就好
2 这些如此基础的类型，std也会使用，所以一般地，std.prelude.*会重导出这些类型，
    直接可达，如果为了规避name shadowing，那么也可以指明全路径
3 显然是；理想上我希望comptime更代数一点、高层一点，但是我在实现过程中常常会碰到一些阻碍，
    我也不好说要不要保留Real, （comptime已经f64和f32）
4 parser只看token而不考虑源码
5 都说了这些会被去糖，返回类型没有额外约束，另外，对函数本身有一个约束，要么是comptime函数（有唯一self参数），
    要么该函数有唯一comptime参数（不是self），因为comptime类型不能定义runtime方法，这是符合直觉的，后者只是普通函数，
    这也是2b去糖规则这样选择的原因；\0按c行不行，其他语言有没有，为什么
6 A
7 B, 依赖于flurry的表达力，不需要逐段重写
8 插值说得还不够清楚吗，没有$正常处理
9 A
10 A，e前必须是完整的小数格式，整个科学计数法必须作为单一token，e前后不得有空隙
11 A
12 C
13 B
14 A，它们有对应的类型，显然是字面量
15 你表达不清楚
16 “沉降”概念已经完全改名
17 A
```
