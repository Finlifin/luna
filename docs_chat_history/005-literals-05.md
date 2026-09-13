# 005 — literals — 05

- **日期**: 2026-08-09
- **global_index**: 005
- **theme**: literals
- **subindex**: 05
- **本轮目标**: 按作者指正，纠正「编译期专属类型」误述；补充双宇宙同名 proxy 与 splice。

---

## 作者本轮输入摘要

`str`、`Symbol`（并及同节的 `Integer`、`Float`）**不是**编译期专属类型；它们有 splice 规则，且在运行时上下文有**同名 proxy**；类型同在两个宇宙。要求在 `primitives.md` 补充代理类型与 splice 相关内容。

---

## 文档写入

- **是**（作者明确要求补充）。
- `docs/basics/primitives.md`：删除「编译期专属类型」整节对上述四类的归类；新增「双宇宙类型、代理与 splice」；`Type` 单独说明（无同名 runtime proxy 叙事）。
- `docs/basics/literals.md` / `traits/index.md`：去掉「仅 comptime 类型」口吻，与 primitives 对齐。

---

## 已确认 / 仍待定

- **已确认**：`Integer` / `Float` / `str` / `Symbol` 双宇宙同名；U₀ 侧为 proxy；splice 嵌入为同名 proxy 值。
- **待定（未臆造）**：各 proxy 的运行时布局与完整操作集；各类型 splice 规则的逐条形式化（仅写了字面量场景与指向 boostrap 的交叉引用）。`Type` 仍按「无同名 runtime proxy」处理——若有误请纠正。

---

## 需要你拍板的问题

1. **`Type` 是否也有运行时同名 proxy？**
   - A. 否（维持现状：仅类型层 / 编译期宇宙）
   - B. 是（请补一句语义）
   - Other: …

2. **`char` / `bool` 是否也按「双宇宙 + 同名 proxy」叙述？**
   - A. 否，它们就是普通跨上下文同一类型，不必用 proxy 说法
   - B. 是，与 `str` 等并列
   - Other: …

---

## 作者更正（同日）

`Float`、`Integer` **不是**双宇宙同名 proxy：它们**只在 comptime 宇宙**；有 splice 规则，但代理类型**不同名**。

已改 `primitives.md` / `literals.md`：
- `Integer` / `Float` →「编译期数值类型与 splice」
- `str` / `Symbol` → 保留「双宇宙同名 proxy」
- 去掉「Float 是 f64 别名」的定宽表混写（Float 归入编译期节）

待确认：`Integer` / `Float` 的默认 / 具名代理类型名是否就是 `i32` / `f64`，抑或另有名字。

---

## 作者要求：通用规则勿绑死具体类型

splice / 宇宙 / proxy 是通用规则；文档应陈述通则，具体类型只作举例。

已改：
- `primitives.md`：一节「宇宙、代理与 splice」写两类情形；`Integer`/`Float`/`str`/`Symbol`/`Type` 仅作表格式举例。
- `literals.md`：去掉按类型复述 splice 细节；类型表只列名字，细则指回 primitives。
