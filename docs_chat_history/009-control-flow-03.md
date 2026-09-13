# 009 — control-flow — 03

- **日期**: 2026-08-09
- **global_index**: 009
- **theme**: control-flow
- **subindex**: 03
- **本轮目标**: 锁定 008 答复；按授权写出 **基本控制流** 七段草案（`DRAFT — NOT CANONICAL`），仅落 history，**不**改 `docs/`。

---

## 作者本轮输入摘要

（全文已附于 [`008-control-flow-02.md`](./008-control-flow-02.md)）

1. **B**：逻辑一章，物理两文件 + `index.md` 总览。  
2. **B**：本主题暂不引用 OLD。  
3. **A**：`when` 兜底为 `else => body`。  
4. **A**：`break`/`continue` 不带值；带值的是 `return`（以及效应里的 `resume`，本章不管）。  
5. **A**：`if`/`when` 为表达式；`while`/`for` 为语句。  
6. **A**：标签 `for:lab` / `while:lab`；`break lab` / `continue lab`。  
7. **A**：`expr match {…}` 与 `if expr is do {…}` 并存——前者图方便；后者可加 `inline`，且可退化成 `is pattern do`。  
8. **A**：先 history 七段草案。

---

## 已确认（本轮升格）

| ID | 结论 |
|----|------|
| CF9 | 布局：`index.md` 总览 + `basic.md` + `pattern-matching/…`。 |
| CF10 | 暂不引用 OLD-2024。 |
| CF11 | `when`：`expr => …` 与 `else => …`。 |
| CF12 | `break`/`continue` 无值；`return` 可有值（`resume` 另章）。 |
| CF13 | `if`/`when` 表达式；`while`/`for` 语句。 |
| CF14 | 标签形态确认。 |
| CF15 | `match` 与 `if is do` 并存及动机（方便 vs inline/可退化）。 |
| CF16 | 本轮只出 history 草案。 |

---

## DRAFT — NOT CANONICAL

以下拟作为日后 `docs/control-flow/basic.md` 的正文骨架（模式匹配细节仍归 pattern 文件；此处只收控制流所需的 `is`/`match` 形态）。示例已按 grammar/style 自检意向编写（`and`/`or`、`--`、无 `loop`、守卫为 `while`、`is` 带 `do`）。

---

# 基本控制流

## Intro

控制流决定语句与表达式如何分支、重复与退出。Flurry 里 **`if` 与 `when` 是表达式**（分支结果可以赋给绑定）；**`while` 与 `for` 是语句**。提前退出用 `return` / `break` / `continue`；需要「带条件才退出」时，在其后写 **`while cond`**（不用 `if`，以免与表达式前缀混淆）。

模式相关的多臂形式（`if … is do { … }`、后缀 `match`）与循环上的 `while … is … do` 也在本章给出表面形态；模式本身见同章的模式匹配文档。`inline` 控制流见 comptime 章。

## 语法

```text
if_expr      -> if expr block (else (block | if_expr))?
when_expr    -> when { when_arm* }
when_arm     -> expr => (block | statement)
              | else => (block | statement)

for_stmt     -> for label? pattern in expr block
while_stmt   -> while label? expr block
label        -> : id

while_is     -> while label? expr is pattern do block
while_is_arms-> while label? expr is do { case_arm* }

if_is        -> if expr is pattern do block (else (block | if_is))?
if_is_arms   -> if expr is do { case_arm* }
post_match   -> expr match { case_arm* }

case_arm     -> pattern => (block | statement)

return_stmt  -> return expr? (while expr)?
break_stmt   -> break id? (while expr)?
continue_stmt-> continue id? (while expr)?
```

（确认写入后回灌 `docs/grammar/syntax`：`when` 臂改为 `expr`；守卫保持 `while`。）

## 正式定义

### `if` / `else`

- 条件为布尔表达式；真执行 then 块，否则走 `else`（若有）。
- 作为表达式时，各分支交出的值类型须一致（或可统一）；无 `else` 且当作表达式使用时的规则：**待定**（见文末问题）。
- `else if` 即 `else` 后接另一个 `if`。

### `when`

- 按书写顺序求值各臂左侧的布尔表达式；执行**第一个**为真的臂。
- 兜底臂为 `else => …`（不是 `_`）。
- 作为表达式时，各臂结果类型须一致。

### `for` / `while`

- `for pattern in expr block`：对可迭代对象迭代；每次将元素匹配到 `pattern`。
- `while expr block`：条件为真则重复执行块。
- 二者为**语句**，不产生有意义的值供外部绑定（不同于 `if`/`when`）。
- **标签**：`for:name` / `while:name`（`label -> : id`）。`break name` / `continue name` 作用到该标签循环；无标签则作用到最内层循环。

### `while … is … do`

- `while expr is pattern do block`：每次先对 `expr` 做模式匹配；成功则绑定并执行 `block`，失败则结束循环。
- `while expr is do { case_arm* }`：多臂形式。

### `if … is … do` 与 `match`

- `if expr is pattern do block`：单模式分支，可接 `else`。
- `if expr is do { case_arm* }`：多臂；可写成 `inline if …`（见 comptime 章）而后缀 `match` 不便加 `inline`。
- `expr match { case_arm* }`：同一套多臂匹配的后缀写法，图方便。
- 穷尽性等模式规则见模式匹配文档。

### `return` / `break` / `continue`

- `return` / `return expr`：从当前函数返回；可选值。
- `break` / `break label`：退出循环；**不带值**。
- `continue` / `continue label`：进入循环下一轮；**不带值**。
- 均可后缀 **`while cond`**：仅当 `cond` 为真时执行该控制转移（例：`return x while ready`）。

## 常见实例

```flurry
-- if 表达式
let grade = if score >= 90 {
    "A"
} else if score >= 60 {
    "B"
} else {
    "C"
}

-- when 表达式
let kind = when {
    code >= 200 and code < 300 => "2xx"
    code >= 400 and code < 500 => "4xx"
    else => "other"
}

-- for / while 与标签
for:outer i in 0..3 {
    for j in 0..3 {
        continue outer while i == 1 and j == 1
        break outer while i == 2 and j == 0
    }
}

while count < 3 {
    count = count + 1
}

-- 守卫
for n in numbers {
    return n while n % 2 == 0
}

-- is / match（形态；模式细节另文）
if shape is .circle(r) do {
    use_radius(r)
}

if shape is do {
    .circle(r) => use_radius(r)
    .point => use_point()
}

let area = shape match {
    .circle(r) => r * r
    .point => 0
}

while iter.next() is item? do {
    process(item)
}
```

## 与 Rust / Zig / C / C++ 的关键差异

| 点 | Flurry |
|----|--------|
| 分支 | `if`/`when` 为表达式；循环为语句 |
| 多条件分支 | `when { expr => …; else => … }` |
| 条件退出 | `return`/`break`/`continue` 后接 `while cond` |
| 模式分支 | `if … is … do`（可 `inline`）与后缀 `match` 并存 |
| 无限循环 | 用 `while true`，无 `loop` |
| `break` 值 | 不带值 |

## 运行时实现设计引导

1. `if`/`when`：求条件（`when` 顺序短接），跳转到对应块；表达式形式把块结果写入目标临时值。  
2. `while`/`for`：条件/迭代器协议与块组成循环；标签翻译为可跳转的循环出口/继续点。  
3. `while is`：每次迭代先匹配；失败则离开循环。  
4. `return`/`break`/`continue`：先求可选的 `while` 条件，再执行转移；`break`/`continue` 无值通道。  
5. 失败模式：类型不一致的表达式分支、无效标签、在非循环上下文 `break`/`continue` → 静态错误。

## 编译器实现引导

1. 解析：按上节产生式；`when` 臂为 `expr =>`；守卫为 `while`；`is` 后必须有 `do` 再接块或臂列表。  
2. 将 `if`/`when` 表达式纳入表达式类型检查；`while`/`for` 按语句。  
3. 标签名分辨到包围循环；`break`/`continue` 目标解析。  
4. `if is do` / `match` 共享 case 臂降低与穷尽性检查（细节与 pattern 章衔接）。  
5. 确认后更新 `docs/grammar/syntax` 中 `when` 的 `condition_arm`（`pattern` → `expr`）等与本文不一致处。

---

## 需要你拍板的问题（草案缺口）

1. **表达式 `if` 无 `else`？**
   - A. 非法（必须穷尽）。
   - B. 合法，缺失分支类型为 `void` / `()`。
   - C. 合法，类型为 `?T`。
   - Other: …

2. **`when` 无 `else` 且作表达式？**
   - A. 必须有 `else` 或条件已穷尽（静态检查）。
   - B. 与问题 1 相同策略。
   - Other: …

3. **`for` 的 `in` 右侧要求？**（Iterable 协议名是否已定）
   - A. 文档只写「可迭代表达式」，协议名另章。
   - B. 点名具体 trait/方法（请给名）。
   - Other: …

4. **草案是否可升格写入 `docs/control-flow/basic.md`，并加简短 `index.md`？**
   - A. 是（按草案 + 你对 1–3 的答复微调）。
   - B. 先改草案（请批注）。
   - C. 否，继续只讨论。
   - Other: …

---

## 文档写入

- **否**（仅 history 草案）。示例自检：守卫 `while`、`is`+`do`、`else =>`、无 `loop`、谓词 `and`；`%` 作取模是否允许若与「无位运算」政策冲突则你可改示例。

---

## 作者答复（prompt）

日期：2026-08-09

来源：仓库根目录 `prompt`（全文照录）

```
1 B
2 B
3 B, Iter或IntoIterator
4 A
```

另：作者指示「然后是模式匹配部分」。
