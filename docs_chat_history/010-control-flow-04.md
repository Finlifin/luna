# 010 — control-flow — 04

- **日期**: 2026-08-09
- **global_index**: 010
- **theme**: control-flow
- **subindex**: 04
- **本轮目标**: 写入已授权的 basic/index；启动**模式匹配**子部分的对抗确认（默认先问答 / history 草案，未授权不改 pattern 正式文）。

---

## 作者本轮输入摘要

`prompt`：1B（无 else 的 if 表达式 → void）、2B（when 同策）、3B（`Iter` 或 `IntoIterator`）、4A（写入 basic + index）。并指示继续**模式匹配部分**。

---

## 本轮已写入（basic）

| 路径 | 动作 |
|------|------|
| `docs/control-flow/basic.md` | 七段定稿（按 009 草案 + 本轮答复） |
| `docs/control-flow/index.md` | 新建总览 |
| `docs/grammar/syntax` | `condition_arm`：`pattern` → `expr` |
| `docs/README.md` | 增加控制流总览链 |

`Iter` / `IntoIterator` 正式名仍待拍板（basic 中写「或」）。

---

## 模式匹配：现行材料与张力

现行 `pattern-matching/index.md` 为速览，与已确认控制流规则冲突处：

- 示例多为无 `do` 的 `if … is { … }`（已确认**必须** `do`）。
- 单模式 `if opt is data? {` 无 `do`。
- 「与模式」示例混用 `and … is` 与尾部 `if` 守卫，且块前无 `do`。
- 模式守卫写作 `pattern if expr`（grammar 有 `if_guard_pattern`）；与语句级守卫 `while` 不同层——须确认模式层是否仍用 `if`。
- `error e`、`value!`、`0b(high:4, low:4)`、自定义 pattern view 等均未在本主题确认。
- 穷尽性：文称 `match` 与 `if is` 都要求穷尽；单臂 `if is pattern do` + 可选 `else` 如何计穷尽，待定。

grammar 模式节（`or_pattern`、`as_bind`、`option_some_pattern`、bitvec 等）作**待确认草案**，正文意向优先（CF2）。

---

## 需要你拍板的问题（模式匹配）

1. **模式文档产出方式？**
   - A. 先 history 七段草案，你审后再写 `pattern-matching/index.md`。
   - B. 关键问题答完后直接写正式文。
   - C. 先只确认模式种类清单，再写。
   - Other: …

2. **本章收录哪些模式种类？**（可多选意向；或不在清单内的删掉）
   - A. 维持现行文已列：字面量、绑定、`_`、符号、结构体、枚举变体、列表、范围、`or`、`and is`、`as`、模式守卫、`?`、`!` / `error`、位向量、（自定义 view 另节）。
   - B. 先只收「核心」：字面量、绑定、`_`、符号、元组/结构体、枚举应用、`or`、`as`、守卫、`?`；列表/范围/位向量/error/view 后置。
   - C. 以 grammar 前缀/中缀/后缀清单为准全收，现行文缺的补上（如 `not`、`ref`、`async`、`..<expr>` 等）——请确认哪些进入 v1 文档。
   - Other: …（请给收录/推迟表）

3. **模式守卫关键字？**（`pattern if expr` vs 语句守卫 `while`）
   - A. 模式层仍用 `if`：`pat if cond`（与 grammar `if_guard_pattern` 一致）；语句层继续 `while`。
   - B. 模式层也改成 `while`：`pat while cond`。
   - C. 其它。
   - Other: …

4. **`and … is` 链（现行「与模式」）是否保留？**
   - A. 是，规范形为 grammar 的 `pattern and expr is pattern`（可连锁）；写清求值/绑定顺序。
   - B. 否，改为其它写法（请给）。
   - C. 推迟出文档。
   - Other: …

5. **可选 / 错误模式表面？**
   - A. 成功：`pat?` / `pat!`；错误：`error pat`（对齐 grammar）。
   - B. 现行文 `error e` 这种「关键字 + 绑定」也合法（请给产生式）。
   - C. 其它。
   - Other: …

6. **位向量模式？**
   - A. 纳入本章，产生式对齐 grammar 的 `0b`/`0o`/`0x` 分段形。
   - B. 推迟到 bitvec / 专章。
   - C. 删除，不存在。
   - Other: …

7. **自定义 Pattern View？**
   - A. 本章只留一节「有此机制，语法/定义另订」，不写细节。
   - B. 本轮给出定义语法（请草拟或指向已有文）。
   - C. 本章不提。
   - Other: …

8. **穷尽性范围？**
   - A. 多臂 `if … is do {…}` 与 `match` 必须穷尽（或 `_`）；单臂 `if … is pat do` 不强制穷尽（失败走 `else` 或跳过）。
   - B. 凡 `is`/`match` 多臂均穷尽；单臂+无 `else` 若作表达式则类型 `void`，作语句则匹配失败为运行时/静态？（请选）
   - C. 其它。
   - Other: …

9. **`let` / 函数参数中的模式是否写入本章？**
   - A. 是，简短一节 + 链到绑定/函数章。
   - B. 否，只写 `is`/`match`/`while is`/`for` 中的模式。
   - Other: …

10. **迭代协议正式名？（补 basic）**
    - A. `Iter`
    - B. `IntoIterator`
    - C. 两者都有（请一句话分工）
    - Other: …

---

## 已确认 / 仍待定

- **已确认（basic）**：无 else 的 if/when 表达式 → `void`；写入 basic/index；grammar `when` 臂为 `expr`。
- **待定**：模式 1–10；pattern 正式文未改。

---

## 文档写入

- basic / index / README / grammar `condition_arm`：**是**。
- pattern-matching：**否**（待本轮问题）。

---

## 作者答复（prompt）

日期：2026-08-09

来源：仓库根目录 `prompt`（全文照录，含 case 示例长文）

```
1 C
2 A
3 A，没有if开头的pattern,不用改
4 A
5 AB不一样吗？
6 A
7 
[struct Student { case adult ... } / extend f64 { case negative_infinity ... } /
 extend Bitvec / struct Instruction { case add ... } 等长示例]
关于pattern definition的纯度和可分析性，分别分为三级，前者将在内存管理章节讲，而后者暂定。
8 。
9 A, 可以提一嘴闭包参数也可以用穷尽pattern
10 C, Iter是迭代器需要实现的trait，IntoIterator是容器本身可以选择实现的默认迭代方法
```
