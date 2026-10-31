# 014 — control-flow — 08

- 日期：2026-08-09
- global_index：014
- theme：control-flow
- subindex：08
- 本轮目标：消化作者对 Intro / `log` / 函数参数的批评；据 `prompt` 收敛 bitvec 专篇理解；按 7A 只写 history 七段草案，不正式落 `docs/.../bitvec.md`

## 作者本轮输入摘要

1. `pattern-matching/index.md` Intro：「一等 pattern」不是「到处可用」；不会说可以不说；**未确认**可用在函数参数上。
2. Intro 废话过多。
3. 不要用臆造的 `log`；Flurry 用 `println` / `eprintln`。
4. `@prompt` 对 bitvec 问题的答复（已追加进 `013`）：
   - bitvec pattern **只**用于 bitvec 这类匹配对象；bitvec **运算**另文，本章不展开。
   - 新文件放在 `pattern-matching/` 下。
   - 迁出范围 **含** 跨元素匹配（UTF-8 等）。
   - 主章：**一句 + 链接**，删错误示例。
   - segment：`(hex | <comptime expr> | (pattern : type_expr))`
   - 匹配主语必须是 `BitVec`（含 `'bitvec` 视图——待写时措辞对齐）。
   - 本轮：问清后 → **history 七段草案**（7A）。
   - 另：必须设置 base type；整段 pattern 与 base type **宽度对齐**；未启用跨元素时，宽度须与 base type **一致**。

## 可行性评估与澄清

- 「一等」此前被误写成「可出现在一串语法位置」——这是错误定义，已从正文删除；**不猜测**作者心中「一等」另义。
- 函数/闭包参数上的模式：**未确认 → 已从正式定义表、示例、编译器引导删除**。
- `log`：臆造 API；已改为 `println` / `eprintln`（`basic.md` 同步）。
- 主章错误 bitvec 示例 `0b(high: 4, low: 4)` 已删；位向量小节改为「一句 + 拟定路径」，**尚未**创建正式 `bitvec.md`（等「写入文档」）。
- 现行 `docs/grammar/syntax` 中 `bit_vec_*_pattern` 仍是 `(integer | (id: expr) | <expr>)+`，与作者本轮 `segment` 产生式**冲突**——不得私自改 grammar；下列草案按作者 `prompt`；冲突列为待确认。

## 已确认 / 仍待定

### 已确认（本轮 + 上一轮叠加）

- Intro 不写「一等=到处可用」；不写函数参数可用模式（未授权）。
- 打印用 `println` / `eprintln`，不用 `log`。
- Bitvec pattern 专属；运算不展开；新文在 `pattern-matching/`；主章一句+链；含跨元素；主语须为 BitVec；先 history 草案。
- 宽度：须设 base type；整段对齐；未跨元素时与 base 宽度一致。

### 待定

- 「一等 pattern」是否还要在别处定义？若要，作者给出指称义。
- `segment` 中的 `hex`：是否指「**当前进制下的数字字面段**」（0b 下为二进制数字串），还是字面只允许十六进制字形？
- `(pattern : type_expr)` 的内层 `pattern` 可否再嵌套 bitvec / 仅标识符？
- 「必须设置 base type」：写在何处（枚举 `.basic_type`、类型属性、匹配主语注解、其它）？
- 「启用跨元素定义」的表面开关：枚举字段 / attribute / 其它？
- `prompt.fl` 中 `0b1 11111111 (0: BitVec<23>) ...` 的 `...` 语义？
- 正式写入 `bitvec.md` 与回灌 `grammar/syntax` 的授权时机。

---

## DRAFT — NOT CANONICAL

拟定路径：`docs/control-flow/pattern-matching/bitvec.md`

### 1. Intro（草案）

位向量模式按固定进制把一段连续比特拆成若干段：常数段、comptime 段、或「子模式 : 宽度类型」。它只匹配 **`BitVec`**（含对值取 `'bitvec` 得到的视图）。跨元素匹配在同一机制上允许占用多个 base 元素；未启用时，pattern 总宽度必须等于单个 base 的宽度。BitVec 上的连接、按位与或非等**运算**不属于本章。

### 2. 语法（草案）

```text
bit_vec_bin_pattern -> 0b segment+
bit_vec_oct_pattern -> 0o segment+
bit_vec_hex_pattern -> 0x segment+

-- 作者确认的 segment（DRAFT；与现行 grammar 冲突，见待定）
segment -> hex
         | <comptime_expr>
         | (pattern : type_expr)
```

- `hex`：待确认是否表示「当前前缀进制下的数字字面段」。
- `type_expr`：绑定宽度类型，常见 `uN` 或 `BitVec<N>`（与 `prompt.fl` 示例一致；正式库名待库文档）。
- 使用处：与其它 `pattern` 相同（`is` / `match` 等）；**匹配主语类型须为 BitVec**。

### 3. 正式定义（草案）

1. **适用对象**：仅当被匹配值的类型为 `BitVec<…>`（或经 `'bitvec` 得到的等价位视图）。其它类型上写 bitvec pattern → **静态错误**。
2. **进制**：`0b` / `0o` / `0x` 决定数字字面段的解释与书写字母规则（与字面量章一致处交叉引用，不重复发明）。
3. **段**：
   - 数字字面段：提供固定比特（或数字）约束。
   - `<comptime_expr>`：编译期求值得到的段约束（细节：值域与宽度推导 **待定**）。
   - `(pattern : type_expr)`：按 `type_expr` 声明的宽度切一段，再对切出的值匹配内层 `pattern`（常为绑定名）。
4. **Base type**：使用方（类型 / 枚举等）**必须**声明 base type；其宽度为对齐基准。
5. **宽度对齐**：整条 bitvec pattern 的总宽度必须与适用的对齐约束一致。
6. **非跨元素**：未启用跨元素定义时，pattern 总宽度必须 **等于** base type 宽度（单元素）。
7. **跨元素**：启用后，一个逻辑值可跨越多个 base 元素；总宽度仍须与该逻辑布局对齐（UTF-8 码点变体等示例见下）。决策树 / 超优化为实现目标，不作语义替代定义。
8. **与运算分离**：`++`、`and` / `or` / `not`、`.as(T)`、`'bitvec` 取值等属 BitVec **值**层面，见类型/运算文档；本章只定义 **pattern**。

### 4. 常见示例（草案；语法/风格自检）

自检：`println`；`and`/`or`/`not`；注释 `--`；后缀 `'bitvec`；无臆造 `log`；无错误的 `0b(high:4, low:4)`。

```flurry
test {
    let num: f32 = 13.625;
    if num'bitvec is 0b(sign: u1) 10000010(exponent: u8) 10110100000000000000000(mantissa: BitVec<23>) {
        println("sign: {}, exponent: {}, mantissa: {}", sign, exponent, mantissa);
    }

    if num'bitvec is do {
        0b(sign: u1) 11111111(exponent: u8) (mantissa: BitVec<23>) if mantissa != 0 => println("NaN"),
        0b(sign: u1) 11111111(exponent: u8) 00000000000000000000000(mantissa: BitVec<23>) => println("Infinity"),
        _ => (),
    }
}
```

跨元素（示意；完整枚举属性语法以数据/枚举章为准，此处只展示 pattern 段）：

```flurry
-- DRAFT：Utf8CodePoint 变体上的 bitvec pattern
-- one  = 0b0(ascii_char: ArbitarySizedUnsigedInt<7>)
-- two  = 0b110(ys: BitVec<5>) 10(zs: BitVec<6>)
-- three = 0b1110(xs: BitVec<4>) 10(ys: BitVec<6>) 10(zs: BitVec<6>)
-- four = 0b11110(ws: BitVec<3>) 10(xs: BitVec<6>) 10(ys: BitVec<6>) 10(zs: BitVec<6>)
```

### 5. 与 Rust / Zig / C / C++ 的关键差异（草案）

| 点 | Flurry（已确认方向） |
|----|----------------------|
| 匹配对象 | 显式 BitVec / `'bitvec`，不是对任意整数「假装位模式」乱写 |
| 分段 | 拼接多段，而非单一 `(name: width)*` 元组式错误写法 |
| 跨元素 | 可与 DST / 多 base 元素布局结合（启用时） |
| 运算 | 模式与 BitVec 运算分开文档 |

（不把其它语言的 bitfield / `bitmatch` 写成 Flurry 已有物。）

### 6. 运行时实现设计引导（草案）

1. 输入：BitVec 值（或视图）+ 分段 pattern + base 宽度 / 是否跨元素。
2. 校验：总宽度 vs 对齐约束；失败为静态错误（运行时不应接到非法模式）。
3. 匹配：按段从高位到低位（或文档锁定的端序）切片；常数段比较；绑定段写入环境。
4. 跨元素：按 base 序列组装逻辑比特流再切，或等价的多字节决策树；语义以「逻辑比特布局」为准。
5. 失败模式：宽度不对齐、非 BitVec 主语、段类型宽度无法确定 → 编译期拒绝。

### 7. 编译器实现引导（草案）

1. 解析：`0b`/`0o`/`0x` + `segment+`；段形为作者确认的三种；**回灌 grammar 须另授权**（当前 syntax 文件仍旧）。
2. 类型：主语归一到 `BitVec`；`(pattern : type_expr)` 检查 `type_expr` 可提供宽度。
3. 宽度求和与 base / 跨元素开关检查。
4. 降码：优先生成可超优化的比较/掩码树；不在语义文档绑定具体 crate 名。
5. 与 `case` / 枚举 DST 布局的接口：跨元素章与枚举章交叉引用，避免重复定义布局属性名（属性名以作者确认的枚举文档为准）。

---

## 需要你拍板的问题

1. `segment` 里的 `hex` 具体指什么？
   - A. 「当前前缀进制下的数字字面段」（0b→二进制数字，0x→十六进制数字）——与 `prompt.fl` 示例一致
   - B. 字面只允许十六进制字形，与 `0b`/`0o` 前缀正交（代价：示例里的 `10000010` 要改写法）
   - C. 改名为 `digits` / `integer` 写进 grammar，避免「hex」歧义
   - Other: …

2. 现行 `docs/grammar/syntax` 的 `bit_vec_*_pattern`（`integer | (id: expr) | <expr>`）与本轮 `(pattern : type_expr)` **冲突**，正式写入时怎么处理？
   - A. 以本轮 `prompt` 为准，授权后改 grammar
   - B. grammar 为准，修订本轮 segment 说法
   - C. 两者合并：允许 `id` 糖衣 + 完整 `pattern : type_expr`
   - Other: …

3. 「必须设置 base type」写在哪？
   - A. 仅枚举/ADT 的布局字段（如 `.basic_type`）
   - B. 任意参与 bitvec 匹配的类型都要有可查询的 base 宽度
   - C. 匹配处显式注解，不依赖类型声明
   - Other: …

4. 「启用跨元素」的表面形式？
   - A. 类型/枚举上的布尔布局标志（如 `.dst` / `.pattern_view_defined` 一类——名称另表）
   - B. attribute
   - C. 仅有总宽度 > base 宽度即自动跨元素
   - Other: …

5. Intro 还要不要提「一等 pattern」？
   - A. 整章不再出现「一等」
   - B. 另开一小段，由你给出指称定义后再写
   - Other: …

6. 本草案可否升格为正式 `docs/control-flow/pattern-matching/bitvec.md`？
   - A. 可以写，按草案落盘并改主章为真链接
   - B. 先答完 1–5 再写
   - C. 再改一版草案
   - Other: …

## 文档写入

- **已写入（纠错，非 bitvec 专篇）**：
  - `docs/control-flow/pattern-matching/index.md`：缩短 Intro；删函数参数主张；bitvec 改一句+拟定路径；删错误示例；`println`/`eprintln`
  - `docs/control-flow/basic.md`：`log` → `println`/`eprintln`
- **正式 `bitvec.md`**：否（本轮 7A = history 草案）

---

## 作者答复（prompt）

日期：2026-08-10

```
1 任意id或者int token，分析阶段会统一溯源回源码处理
2 A
3 通过attribute设置，详见docs/comptime/attribute.md和docs/control-flow/pattern-matching/prompt.fl:117
4 。
5 B
6 A
```
