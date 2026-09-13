# 007 — control-flow — 01

- **日期**: 2026-08-09
- **global_index**: 007
- **theme**: control-flow
- **subindex**: 01
- **本轮目标**: 新主题启动；盘点现行 `docs/control-flow/` 与 grammar；确认范围与权威冲突；默认不写入正式文档。

---

## 作者本轮输入摘要

进入新主题：`control-flow`。

---

## 已知材料（非全部已确认）

| 路径 | 内容概要 | 状态 |
|------|----------|------|
| `docs/control-flow/basic.md` | `if`/`else`、条件守卫、`when`、`for`/`while`/`while is`、标签、`break`/`continue`/`return` | 现行短文；多处与 grammar/风格冲突，须逐条重确认 |
| `docs/control-flow/pattern-matching/index.md` | `if is`、`match`、诸模式、`while is`、穷尽性 | 同上 |
| `docs/grammar/syntax` | `if`/`when`/`for`/`while*`/`if_*`/`return`/`break`/`continue`/`label`、后缀 `match` | 表面语法参考；与 basic 文冲突处见下 |
| `docs/comptime/inlining-comptime-code.md` | `inline if/when/for` | 相邻主题；是否纳入本 theme 须问 |
| `docs/OLD-2024/.../control_flow*` | 旧文 | 默认非权威；沿用 literals 时「OLD 废弃」倾向，仍须点名确认 |

### grammar 与现行 basic/pattern 文的明显张力（待裁决，不擅自选边）

1. **条件守卫关键字**：basic 写 `return n if cond` / `break if`；grammar 为 `return expr? (while expr)?`、`break id? (while expr)?`、`continue id? (while expr)?`（后缀条件是 **`while`**，不是 `if`）。
2. **`if is` / `while is` 是否要 `do`**：grammar 有 `if expr is pattern do block`、`while … is pattern do block`、以及 `is do { case_arm* }`；现行示例多为无 `do` 的 `{` 直接开分支。
3. **`when` 臂**：grammar `condition_arm -> pattern => …`；basic 示例臂左侧是布尔表达式（`code == 200`），不是明显的 pattern。
4. **`match` 形态**：grammar 有后缀 `expr match { case_arm* }`；pattern 文同时有前缀感的 `if expr is { … }` 与 `shape match { … }`。
5. **`loop`**：basic 称 `break` 可带值「用于 `loop` 表达式」，但 basic/grammar 未见 `loop` 产生式。
6. **风格/示例**：basic 含 `+=`、`%`、Rust 味枚举 API；须按 grammar/style 自检，未确认前不当作 canon。

### 已从其它主题可沿用的（非本主题新确认）

- 谓词用 `and`/`or`/`not`；注释 `--`；常量命名风格等（grammar/index）。
- 符号 `.id`；字面量规则（literals 章）。
- 正式文档抽象边界：AGENTS §9（通则举例、反泄漏、淘汰概念不提及）。

---

## 评估

主题可开。现行两篇控制流文档更像草稿速览，**不宜**在未确认 grammar 冲突前按七段「润色定稿」。建议先定：**范围切分**、**权威源（grammar vs 现行文）**、**守卫关键字与 `do`**，再分别重写 `basic` 与 pattern（或合并）。

本轮默认只讨论；说「写入文档」才改 `docs/`。

---

## 需要你拍板的问题

1. **本主题 `control-flow` 的文档范围？**
   - A. 本轮只做「基本控制流」：`if`/`when`/`for`/`while`/`break`/`continue`/`return`/标签；模式匹配另开 theme 或后置。
   - B. 本 theme 覆盖 basic + pattern-matching 两章，但分文件仍保持现状。
   - C. 合并为一章「控制流」，pattern 作子节。
   - D. 再含 `inline if/when/for`（与 comptime 文交叉）。
   - Other: …

2. **与 `docs/grammar/syntax` 中控制流产生式的关系？**
   - A. grammar 为待确认草案：逐条对 basic/pattern 提问后，确认的再回灌 grammar + 正文。
   - B. grammar 已可作表面语法权威；正文与之不符处视为正文错误，按 grammar 改（仍须你点头写入）。
   - C. 正文意向优先；grammar 过时，确认后改 grammar。
   - Other: …

3. **`docs/OLD-2024` 下控制流相关旧文？**
   - A. 废弃，仅考古。
   - B. 部分有效（你标注保留项）。
   - C. 其它。
   - Other: …

4. **条件守卫表面关键字？（冲突 1）**
   - A. 用 `while`：`return expr while cond` / `break while cond`（对齐现行 grammar）。
   - B. 用 `if`：`return expr if cond`（对齐现行 basic 文）；并改 grammar。
   - C. 两者都是糖 / 同义（请说明规范形）。
   - Other: …

5. **`if is` / `while is` 是否需要 `do`？（冲突 2）**
   - A. 需要：`if expr is pattern do block`；多臂用 `if expr is do { case_arm* }`（对齐 grammar）。
   - B. 不需要 `do`：`if expr is { case_arm* }` / `if expr is pattern { block }`（对齐现行多数示例）；改 grammar。
   - C. `do` 可选。
   - Other: …（请用产生式写规范形）

6. **`when` 臂左侧是什么？（冲突 3）**
   - A. 任意布尔 `expr`（basic 意向）；grammar 的 `pattern` 应改成 `expr` 或另非终结符。
   - B. 必须是 `pattern`（grammar 意向）；basic 示例应改成模式（含字面量模式等）。
   - C. 允许 `expr` 或 `pattern` 两种臂。
   - Other: …

7. **是否存在 `loop` / `loop` 表达式？**
   - A. 无；删掉 basic 里「break 带值用于 loop」的说法；`break` 不带值或仅在特定循环表达式中带值（请说明）。
   - B. 有 `loop { … }`（请给产生式与 `break` 带值规则）。
   - C. `while true` 即无限循环；`break` 带值规则另定。
   - Other: …

8. **本主题产出物优先级？**
   - A. 先七段重写 `control-flow/basic.md`，pattern 后置。
   - B. 先钉 grammar 控制流片段，再写正文。
   - C. 先只做冲突表确认，不出全文。
   - Other: …

---

## 已确认 / 仍待定

- **已确认**：主题名 `control-flow`；`global_index` 自 007 起；默认不改 `docs/` 直至授权。
- **待定**：上表 1–8；以及 pattern 穷尽性、`matches`、效应/错误消除与控制流边界等（范围确定后再问）。

---

## 文档写入

- **否**。

---

## 作者答复（prompt）

日期：2026-08-09

来源：仓库根目录 `prompt`（全文照录）

```
1 除inline控制流移交comptime章外，作一章
2 C
3 B
4 A, `if`或`when`为表达式前缀，在出现语法错误时容易识别成歧义的表达式而不利于错误报告
5 要do, 不加do无法区分extend pattern call语法和块
6 bool expr或单个else关键字
7 A
8 A
```
