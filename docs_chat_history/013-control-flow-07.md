# 013 — control-flow — 07

- **日期**: 2026-08-09
- **global_index**: 013
- **theme**: control-flow
- **subindex**: 07
- **本轮目标**: 对抗理解位向量 pattern 与「移至新文件夹」；对照 `pattern-matching/prompt.fl`；**不**改正式 `docs/`。

---

## 作者本轮输入摘要

1. 先前：位向量 pattern 有误，内容多，移至新文件夹；参考 `docs/control-flow/pattern-matching/prompt.fl`。  
2. 随后纠正：**对抗理解而不是直接更改。**

---

## 评估：现行文 vs `prompt.fl`（第 3 节「二进制匹配」）

### 现行 `pattern-matching/index.md` 的问题（候选错误，待你确认）

| # | 现行写法 | `prompt.fl` 所示意向 |
|---|----------|----------------------|
| E1 | `0b(high: 4, low: 4)`：括号内多字段、宽度为裸整数 | 分段**并列**：`0b(sign: u1) 10000010(exponent: u8) …(mantissa: BitVec<23>)` |
| E2 | 对裸 `u8`/`byte` 直接 `match` | 典型对 **`expr'bitvec`**（或已是 `BitVec<N>` 的值）匹配 |
| E3 | 几乎无 `BitVec` / `++` / `and`/`or`/`not` / `.as` | bitvec 是抽象视图 + 可超优化映射；字面/类型 `BitVec<N>` 与取像 `'bitvec` 并存 |
| E4 | 未提绑定类型可以是定宽整数 **或** 同宽 `BitVec` | 明确允许「看起来不太安全」但优于手写位运算 |
| E5 | 未提字面比特串段、`(0: BitVec<23>)`、`...` 等 | 多臂 NaN/Inf 例中有这些形态 |
| E6 | grammar `0b (integer \| (id: expr) \| <expr>)+` | 与 prompt 的「多段拼接」更近，但 `(id: expr)` 的 `expr` 是类型还是宽度？prompt 写的是 **类型**（`u1`/`u8`/`BitVec<23>`） |

### `prompt.fl` 还暗示、但超出「纠正一行 pattern」的范围

- **跨元素匹配**（UTF-8 `dst` enum + `ptr'dst_deref`）把 bitvec pattern 嵌进变体定义。  
- bitvec 作为「高级特性的基础」，不只是模式一小节。  
- 与「无内置位运算符、用 bitvec/math」的全局叙事一致。

因此「移到新文件夹」很可能不只是剪切一节，而是：**位向量 / 二进制匹配（乃至相关视图）单独成题**。

### 尚未由你钉死、我不得当成 Flurry 事实的点

- 新目录路径与是否仍挂在 `control-flow/` 下。  
- 是否连同「跨元素匹配」、BitVec 运算 API 一并迁出。  
- `(id: T)` 中 `T` 的精确种类（仅整数型？任意 `Type.size` 匹配的类型？）。  
- 段与段之间是否允许空白以外的分隔；`0o`/`0x` 是否同构。  
- `...` 在 bitvec 臂上的含义（prompt 有 `... =>`）。  
- grammar 是否改为「类型」而非 `expr` 宽度。

---

## 需要你拍板的问题

1. **「有误」主要指哪些？**
   - A. 主要为 E1–E2（形态 + 匹配对象）；E3–E5 是缺写不是写错。
   - B. E1–E6 整表都算错/过时。
   - C. 你点名行号/片段。
   - Other: …

2. **新文件夹放哪、叫什么？**
   - A. `docs/control-flow/bitvec-pattern/`（仍属控制流/模式大主题）。
   - B. `docs/bitvec/`（独立主题：视图 + 运算 + pattern）。
   - C. `docs/data-structures/bitvec/` 或 `docs/memory-management/…`。
   - Other: …（请给路径）

3. **迁出范围？**
   - A. **仅**二进制 / 位向量 **pattern**（含对 `'bitvec` 的匹配示例）。
   - B. BitVec 类型、取像、`++`/`and`/`or`/`not`/`.as` + pattern（prompt §3 整块）。
   - C. B + **跨元素匹配**（prompt §4 UTF-8）。
   - D. 更广（请列）。
   - Other: …

4. **主章 `pattern-matching/index.md` 留下什么？**
   - A. 删光位向量正文，只留一行链到新文。
   - B. 留一句「存在 bitvec 模式」+ 链，无错误示例。
   - C. 其它。
   - Other: …

5. **规范表面形态（基于 prompt，待你改）？**

```text
-- DRAFT — NOT CANONICAL
bitvec_pattern -> bitvec_radix bitvec_segment+
bitvec_radix   -> 0b | 0o | 0x
bitvec_segment -> integer_bits          -- 如 10000010、11111111、0
                | (id : type_expr)      -- 如 (sign: u1)、(mantissa: BitVec<23>)
                | (literal : type_expr) -- 如 (0: BitVec<23>)？
                | <expr>                -- grammar 已有；prompt 未展示
```

   - A. 采纳上表为讨论基线，再改。
   - B. 段只能是 `(id: type)` 与「比特字面量」，无 `<expr>`。
   - C. 你重写产生式。
   - Other: …

6. **匹配主语默认是什么？**
   - A. 必须是 `BitVec<_>`（通常来自 `x'bitvec` 或本就 BitVec 的值）。
   - B. 整数等也可直接配 bitvec pattern（隐式取像）。
   - C. 其它。
   - Other: …

7. **产出方式（仍先理解）？**
   - A. 先把本节问清，再 history 七段草案（新路径）。
   - B. 问清后你再说「写入文档」。
   - C. 其它。
   - Other: …

---

## 已确认 / 仍待定

- **已确认**：不得在未理解前直接改文；参考 `prompt.fl` 二进制匹配节。  
- **待定**：上表 1–7；正式文与 grammar 均未改。

---

## 文档写入

- **否**。

---

## 作者答复（prompt）+ 对 index 的批评

日期：2026-08-09

`prompt`：
```
1 bitvec pattern是bitvec的pattern, 只能用于这种类型上。bitvec上的运算那是另一回事，不用展开
2 我说的是pattern-matching下的新文件
3 C
4 B
5 segement是(hex | <comptime expr> | (pattern : type_expr))
6 A
7 A

- 必须设置base type
- 整段pattern必须与base type的宽度对齐
- 未启用跨元素定义时，必须与base type宽度一致
```

另：Intro「一等」用错、勿臆造函数参数可用模式；Intro 废话过多；不要用 `log`，用 `println`/`eprintln`。
