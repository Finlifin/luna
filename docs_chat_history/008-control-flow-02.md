# 008 — control-flow — 02

- **日期**: 2026-08-09
- **global_index**: 008
- **theme**: control-flow
- **subindex**: 02
- **本轮目标**: 消化 007 的 `prompt`；锁定范围与若干表面语法；追问文件布局、OLD 保留项、`when`/`break` 细节；**不**写正式文档（未授权全文）。

---

## 作者本轮输入摘要

（全文已附于 [`007-control-flow-01.md`](./007-control-flow-01.md)）

1. 除 **inline 控制流**归 comptime 章外，控制流**作一章**（含原 basic + pattern）。  
2. **C**：正文意向优先；grammar 过时，确认后改 grammar。  
3. **B**：OLD-2024 控制流相关部分有效（未列保留清单）。  
4. **A**：守卫用 `while`（如 `return expr while cond`）。理由：`if`/`when` 已是表达式前缀，出错时易歧义、不利诊断。  
5. **`is` 形要 `do`**：否则无法区分 extend/pattern call 与块。  
6. **`when` 臂**：布尔 `expr`，或单个 **`else` 关键字**（不是任意 pattern / 未必是 `_`）。  
7. **A**：无 `loop`。  
8. **A**：先按七段推进「基本控制流」内容（在「一章」前提下优先写这部分）。

---

## 评估

### 已对齐的表面方向

| 点 | 结论 |
|----|------|
| 章节 | 一章；`inline *` 不进本章 |
| grammar | 随后按确认结果改；不以现行 grammar 压正文意向 |
| 守卫 | `… while cond` |
| `if is` / `while is` | 需要 `do` |
| `when` | 臂为 bool expr；兜底为 `else` |
| `loop` | 不存在 |

### 张力 / 缺口

- **「作一章」+「先重写 basic」**：文件是单页 `control-flow/index.md`（pattern 作二级标题），还是一章两文件？须钉。  
- **OLD B** 未给保留条目 → 不能当已定引用。  
- **`when` 的 `else`**：是 `else => body` 还是仅 `else`？`_` 臂是否非法？  
- **无 `loop` 后**：`break` 是否仍可带值？若可，值回到哪个表达式？  
- **`do` 消歧**：作者指向与 `expr { … }`（extended application）/ pattern 块的区分；文档正式定义写清即可，实现细节不泄漏。

### 建议的规范形草案（DRAFT — NOT CANONICAL）

```text
-- 守卫（规范形）
return_statement    -> return expr? (while expr)?
break_statement     -> break id? (while expr)?
continue_statement  -> continue id? (while expr)?

-- is 形（要 do）
if_is_match    -> if expr is pattern do block (else ...)?
if_match       -> if expr is do { case_arm* }
while_is_match -> while label? expr is pattern do block
while_match    -> while label? expr is do { case_arm* }

-- when
when_statement -> when { when_arm* }
when_arm       -> expr => (block | statement)
               | else => (block | statement)   -- else 形态待确认是否必须 =>
```

---

## 需要你拍板的问题

1. **「一章」的文件布局？**
   - A. 单文件 `docs/control-flow/index.md`（含基本控制流 + 模式匹配各节）；删/重定向旧 `basic.md` 与 `pattern-matching/index.md`。
   - B. 逻辑一章、物理两文件：`basic.md` + `pattern-matching/…`，由 `index.md` 作总览入口。
   - C. 单文件但 pattern 仍留在子目录、仅主文链接（不推荐，易再裂成两章）。
   - Other: …

2. **OLD-2024 控制流「部分有效」请点名？**
   - A. 你列出路径/要点后再引用。
   - B. 本主题先完全不引用 OLD；你稍后补清单。
   - C. 其它。
   - Other: …

3. **`when` 兜底臂的精确写法？**
   - A. `else => body`（与现行 grammar `else_arm` 接近，但其它臂是 `expr =>`）。
   - B. `else { body }` / `else body`（无 `=>`）。
   - C. 允许 `else =>`；禁止 `_ =>`。
   - D. A+C。
   - Other: …（请给 1–2 行示例）

4. **`break` / `continue` / `return` 的值？**（无 `loop`）
   - A. `break`/`continue` 不带值；只有 `return` 可带值。
   - B. `break` 可带值，但仅当外围是「循环表达式」（请指明哪种：`while`/`for` 是否都是表达式）。
   - C. 其它。
   - Other: …

5. **`if` / `when` / `while` / `for` 哪些是表达式？**
   - A. `if` 与 `when` 是表达式；`while`/`for` 是语句（或类型为 `void`）。
   - B. 四者都是表达式（循环表达式的值规则见问题 4）。
   - C. 仅 `if` 是表达式；`when` 是语句。
   - Other: …

6. **循环标签表面形式？**（grammar：`label -> : id`，例 `for:outer`）
   - A. 确认：`for:label pat in expr block` / `while:label …`；`break label` / `continue label`（`break` 后为 id，无冒号）。
   - B. 其它写法（请给产生式）。
   - Other: …

7. **后缀 `expr match { case_arm* }` 是否纳入本章（与 `if expr is do { … }` 并存）？**
   - A. 是，两者都要；语义差异请你一句话（或下一问再拆）。
   - B. 只保留一种（请点名）。
   - C. `match` 仅作 `if is` 的糖。
   - Other: …

8. **是否授权开始写「一章」里的基本控制流七段草案进 history（仍不改 `docs/`），或直接写 `docs/`？**
   - A. 先 history 草案，你审后再入 `docs/`。
   - B. 问题 1/3/4/5 答完后直接写 `docs/`。
   - C. 继续只问答。
   - Other: …

---

## 已确认 / 仍待定

### 已确认

| ID | 结论 |
|----|------|
| CF1 | 控制流作一章；inline 控制流归 comptime 章。 |
| CF2 | 正文意向优先；确认后改 grammar（2C）。 |
| CF3 | OLD 部分有效（清单待问题 2）。 |
| CF4 | 守卫关键字为 `while`；不用 `if`（诊断/歧义理由已陈述）。 |
| CF5 | `is` 形必须有 `do`（与 extended application / 块消歧）。 |
| CF6 | `when` 臂为 bool expr 或 `else`。 |
| CF7 | 无 `loop`。 |
| CF8 | 先推进基本控制流七段内容（8A）。 |

### 待定

- 文件布局；OLD 清单；`when`/`else` 精确形；break 带值；何者为表达式；标签；`match` vs `if is`；写入授权。

---

## 文档写入

- **否**。

---

## 作者答复（prompt）

日期：2026-08-09

来源：仓库根目录 `prompt`（全文照录）

```
1 B
2 B
3 A
4 A, 除了return还有代数效应handler里的resume, 这里不用管
5 A
6 A
7 A, 前者是为了方便，后者是为了前面可以加个inline成为inline控制流，并且可以退化为直接在do前写pattern
8 A
```
