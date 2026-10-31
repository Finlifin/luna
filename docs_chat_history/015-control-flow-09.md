# 015 — control-flow — 09

- 日期：2026-08-10
- global_index：015
- theme：control-flow
- subindex：09
- 本轮目标：消化对主章 bitvec 一句的批评（废话 + 抽象泄漏）

## 作者本轮输入摘要

针对 `pattern-matching/index.md` 原第 94 行：
- 「取了位视图不就是 BitVec 类型？」——并列写「及经 `'bitvec` 得到的位视图」是废话。
- 「又泄漏背景了」——「拟定专篇 / 正式写入前 / chat history 草案」等不应出现在正式文档。

## 评估

- 正确：`'bitvec` 的结果类型就是 `BitVec`，适用对象写一次 `BitVec` 即可。
- 正确：过程状态、history 指针、拟定路径说明 = AGENTS §9.1 抽象泄漏，已删。
- 专篇文件尚不存在时，不挂断链；升格 `bitvec.md` 后再加相对链接。

## 已确认 / 仍待定

- 已确认：主章该句改为「仅适用于 `BitVec`。」；不写视图并列、不写写入进度。
- 待定：仍见 `014` 中 bitvec 专篇拍板问题 1–6（本轮未回答）。

## 文档写入

- 是：`docs/control-flow/pattern-matching/index.md` 位向量模式小节缩为单句。
- 否：仍未创建正式 `bitvec.md`。

## 需要你拍板的问题

（无新增；续 `014` 的 1–6 即可。若要主章在专篇落盘后加链接：）

1. 专篇写成后，主章是否改回「仅适用于 `BitVec`。见 [位向量模式](bitvec.md)。」？
   - A. 是
   - B. 主章完全不提专篇（目录/README 导航即可）
   - Other: …

---

## 作者答复（prompt）续

日期：2026-08-10

`014` 问题 6A：可写正式 `bitvec.md` 并改主章为真链接 → 视为同时拍板 `015` 问题 1A（主章加相对链接）。
