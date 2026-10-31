# 020 — docs-process — 02

- 日期：2026-08-10
- global_index：020
- theme：docs-process
- subindex：02
- 本轮目标：把 bitvec / pattern 文档回合中的行文教训写入 `AGENTS.md`

## 作者本轮输入摘要

「吸取教训，更新 AGENTS.md」。

## 评估

应固化的失败模式（来自近期 control-flow / bitvec 回合）：

1. Intro 堆使用位置、「一等」等未定义词、未确认的函数参数主张  
2. 臆造 `log`；应用 `println` / `eprintln`  
3. 正式文泄漏过程状态（拟定、history）  
4. 「BitVec 及位视图」同义反复  
5. 把 token 溯源等分析私术写进正式定义，而非编译器引导  
6. 「跨语言差异」复述本章定义、无真实对照物  
7. 示例未核对 grammar（`id: pattern` vs `id = expr`）；草稿 `prompt.fl` 不可直接升格  

## 文档写入

- 是：`AGENTS.md`  
  - §2 七段表与风格分流加严  
  - §3.6 标准库 API + 草稿升格核对  
  - §7 禁止清单扩充  
  - §9.1 / 9.3 补充；新增 §9.5（定义 vs 编译器）、§9.6（跨语言自检）

## 已确认 / 仍待定

- 已确认：作者要求把上述教训写入协议。  
- 待定：无（本轮为过程文件更新）。

## 需要你拍板的问题

1. `AGENTS.md` 增补是否还要再收紧/删改？
   - A. 可维持现状
   - B. 某条过严或重复，指出编号
   - Other: …
