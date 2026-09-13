# 012 — control-flow — 06

- **日期**: 2026-08-09
- **global_index**: 012
- **theme**: control-flow
- **subindex**: 06
- **本轮目标**: 按 011 答复写入 `pattern-matching/index.md`；记录 `case` 演进语法与对偶设计。

---

## 作者本轮输入摘要

见 `011` 末 prompt：`error`+pattern 与 `error expr` 对偶；`?`/`!` 同理；`case id(param*) -> ty clauses* body`；匹配侧 `.name`；纯度/可分析性本章留空待定；授权写正式文。

---

## 文档写入

- **是**：`docs/control-flow/pattern-matching/index.md` 七段重写。
- `docs/control-flow/basic.md`：此前已补 Iter/IntoIterator。

---

## 已确认（本轮）

| ID | 结论 |
|----|------|
| PM10 | `error pat` ≡ `error`+pattern；与 `error expr` 对偶；`pat?`/`pat!` 与消除/传播对偶。 |
| PM11 | `case` 签名：`case id(param*) -> return_type_expr clauses* body`；常见 `?Tuple<…>`。 |
| PM12 | 匹配侧 `.name` / `.name(pat*)`。 |
| PM13 | 纯度/可分析性：本章占位待定。 |

---

## 仍待定 / 可后续收紧

- 谓词型 case（仅 `bool`）是否仍允许，或一律 `?Tuple`。
- `case` 允许的宿主列表（struct / extend / enum / …）的穷尽列举。
- grammar 全文回灌 `case_def` 与模式细节。
- 作者长示例中的效应子句、SOA `get` 等属库/效应主题，未写入通法。

---

## 需要你拍板（可选，不阻塞已写正文）

1. **仅返回 `bool` 的谓词 `case`（如旧 `f64` 无穷例）？**
   - A. 允许：`-> bool`。
   - B. 一律改为 `-> ?Tuple<>` / `-> ?()`。
   - C. 其它。
   - Other: …

2. **是否现在把 `case_def` 回灌 `docs/grammar/syntax`？**
   - A. 是。
   - B. 否，等宿主列表钉死再灌。
   - Other: …

---

## 后续：分隔符规则

作者确认：`syntax` 中 `*` 列表省略分隔符；一般项用逗号、语句用分号；分隔符由项的产生式决定（非上下文）；项以 `}` 结尾时可省略其后分号。

已修正示例并写入 `docs/grammar/index.md`「分隔符」；`AGENTS.md` 风格自检增补分隔符一条。

---

## 后续：示例充实与「实例→示例」

作者：示例太贫瘠；章节名应为「示例」不是「实例」。

已改：`AGENTS.md` 骨架用「常见示例」；`basic.md` / `pattern-matching/index.md` 换标题并换成更完整的函数级示例；`literals.md` 标题同步。

---

## 后续：let/var 解构与 else

作者：`let`/`var` 可解构，末尾可 `else`，相关规则与 Rust 类似。

已写入：`pattern-matching/index.md`（正式定义 + 示例）、`bindings.md` 摘要、`grammar/syntax` 的 `let_decl` / `var_decl`。
