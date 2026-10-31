# 004 — literals — 04（主题终止 · 已写入文档）

- **日期**: 2026-08-09
- **global_index**: 004
- **theme**: literals
- **subindex**: 04
- **本轮目标**: 消化 003 的 `prompt`；**终止本主题问答**；按已确认结论写入正式 `docs/`。

---

## 作者本轮输入摘要

（全文已追加于 [`003-literals-03.md`](./003-literals-03.md)）

1. 废除 `Real`；引入 `Float` 作为 `f64` 别名，作浮点/科学计数法字面量的 comptime 类型。  
2. 后缀为独立语法；查不到则报错。  
3. `"…"c` 的 `\0`/尾 NUL 等属**具体函数**语义，不是字面量层该问的。  
4. 只有在对应 comptime 类型上 `extend` 才有字面量后缀。  
5. 相邻字符串合并是语法糖（勿再拆「阶段」空问）。  
6. 只有 `$` 启动插值，不是 `{`。  
7. 字面量层只管脱糖。  
8–9. 对类型表 / 科学计数法切分无异议（「。」）。  
10. **主题终止，开始写文档**。  
附加：转义表加美元符（`\$`）；要求问题想清楚、表达勿含糊。

---

## 本轮写入的正式文档

| 路径 | 动作 |
|------|------|
| `docs/basics/literals.md` | 按七段骨架重写 + 文末产生式附录；含符号；转义含 `\$` |
| `docs/basics/unit-null-undefined.md` | **新建**：`()` / `null` / `_` / `undefined` |
| `docs/basics/symbols.md` | 降为重定向入口 |
| `docs/basics/primitives.md` | `Real`→`Float`（`f64` 别名）；splice 用语 |
| `docs/traits/index.md` | 字面量拓展例改为 `extend Integer` + `Integer.px(100)` |
| `docs/README.md` | 目录更新 |
| `docs/comptime/inlining-comptime-code.md` | 「沉降」→ **splice** |

未改 `docs/OLD-2024/**`（归档；主题 2A 已废）。

---

## 已确认结论（写入依据，闭合清单）

- comptime：`Integer` / `Float`(=`f64` 别名) / `str` / `char` / `bool` / `Symbol`；在 `builtin`，prelude 重导出。  
- 后缀：独立语法；`Type.id(lit)`；失败报错；单次；限数字与字符串；`extend` 挂 comptime 类型。  
- 词法：小写进制前缀；hex 同案；`_` 任意于数字 token；浮点须 `digits.digits`；科学计数法单 token、`e` 前完整小数。  
- 转义：`\\ \' \" \n \r \t \0 \$ \x{…}`；未知转义错误。  
- 字符串：合并为糖；插值 `$`/`${}`、整串 `comptime_format`；raw=`@builtin.str.raw`。  
- `null` 等非字面量；bool 与符号是字面量。  
- 术语：splice，不用沉降。

---

## 需要你拍板的问题

本主题问答已终止。若审阅正式文稿后要改条款，请直接标路径+段落，或开新 theme。

可选（非阻塞）：是否把 `literals.md` 附录回灌进 `docs/grammar/syntax`？

- A. 是，下一短轮只做 grammar 回灌  
- B. 否，以字面量章附录为准即可  
- Other: …

---

## 已确认 / 仍待定

- **已确认**：上表；文档已写。  
- **待定**：grammar 是否回灌（可选）；`"…"c`/`b` 的**库函数**契约（属 std/builtin API 主题，非字面量语法）。

---

## 文档写入

- **是**（作者 10：主题终止，开始写文档）。

---

## 示例自检

正式章示例已按 `docs/grammar/syntax` 与 `docs/grammar/index.md` 自检：`--` 注释、常量 `SCREAMING_SNAKE_CASE`、投影路径、`@builtin.str.raw`、后缀脱糖叙述与 `extend Integer` 一致；插值示例避免在外层字符串内嵌未转义 `"`。`Pixel` 为示意类型。

---

## 后续修订（同日）

作者要求：已淘汰概念不得在正式文档中出现，包括「已废除 / 不再使用」类提及。

已从正式文档删除：
- `literals.md` 中对「沉降」的对照说明
- `primitives.md` 中「已废除旧名 Real」

`docs_chat_history/` 与 `docs/OLD-2024/` 保留历史记录，不改写。

---

## 后续修订：去除抽象泄漏

作者指出：许多仅属设计者内部考量的细节，文档没有告知义务。

已收紧正式文档：
- `literals.md`：删 M1/M2、`emit_err`、parser/token 内幕、builtin implementation 对照、代理类型辩解、排除转义清单式辩解、宏「四类/原始字节」展开等；Intro/正式定义改为读者语义；实现引导只留阶段契约。
- `unit-null-undefined.md`：去掉「不是字面量」说教与产生式邻接辩解。
- `traits/index.md`：字面量拓展表述缩短。
