# 018 — control-flow — 12

- 日期：2026-08-10
- global_index：018
- theme：control-flow
- subindex：12
- 本轮目标：纠正带 pattern 的 enum variant 表面语法

## 作者本轮输入摘要

`bitvec.md` UTF-8 示例（原 74–77）：带 pattern 定义的 enum variant 语法现为 `id: pattern`；`=` 已用于 `id = expr`。

## 评估

与 `docs/grammar/syntax` 中 `pattern_enum_variant -> id: pattern` 一致；示例误用 `=` 来自 `prompt.fl` 草稿惯性。已改为冒号。

## 已确认 / 仍待定

- 已确认：pattern 变体写作 `id: pattern`；`id = expr` 是另一形。
- 待定：`016` 余问；`prompt.fl` 仍为 `=`（草稿，未改正式文以外的备忘，除非作者要求同步）。

## 文档写入

- 是：`docs/control-flow/pattern-matching/bitvec.md` 四臂 `=` → `:`

## 需要你拍板的问题

1. `prompt.fl` 里对应的 `one = …` 是否同步改成 `:`？（备忘非正式文档）
   - A. 改
   - B. 不动草稿
   - Other: …
