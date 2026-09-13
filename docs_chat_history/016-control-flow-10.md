# 016 — control-flow — 10

- 日期：2026-08-10
- global_index：016
- theme：control-flow
- subindex：10
- 本轮目标：消化 `prompt` 对 `014` 的答复；按 6A 写入正式 `bitvec.md`；回灌 grammar；主章加链接

## 作者本轮输入摘要（`prompt`）

```
1 任意id或者int token，分析阶段会统一溯源回源码处理
2 A
3 通过attribute设置，详见docs/comptime/attribute.md和docs/control-flow/pattern-matching/prompt.fl:117
4 。
5 B
6 A
```

（全文已追加到 `014`；`015` 问题 1 视为随 6A 选 A。）

## 评估

| 题 | 解读 | 动作 |
|----|------|------|
| 1 | 原 `hex` = 表面 `id` 或 `integer` token；分析溯源源码再按进制解释 | grammar / 正文按此写 |
| 2A | 以本轮 segment 为准改 grammar | 已改 `docs/grammar/syntax` |
| 3 | base type **通过属性**；配置写法见 attribute 章与 `prompt.fl` 枚举块 | 正文交叉引用 attribute，示例用 `.base_type` |
| 4 | 仅「。」——**未给出可执行选项** | 暂用同块中的 `.dst true` 作为跨元素属性示例；须作者确认或改写 |
| 5B | 要「一等 pattern」指称定义后再写 | **未提供定义** → 主章仍不写「一等」 |
| 6A | 正式写入 `bitvec.md` + 主章真链接 | 已写 |

风险：Q4 空白可能表示「同 Q3 / 见同一引用」，也可能是漏答；把 `.dst` 写进正式文有一点越权——已标待确认。

UTF-8 示例中 `one` 的载荷类型：作者草稿为 `ArbitarySizedUnsigedInt<7>`；正文暂用 `BitVec<7>`（与「uN / BitVec\<N\>」绑定约定一致）。若必须保留专用类型名，请改回。

## 已确认 / 仍待定

### 已确认

- segment：`id` \| `integer` \| `<expr>` \| `(pattern : type_expr)`；id/int 溯源源码。
- grammar 按此更新。
- base type 由属性设置。
- 可写正式 `bitvec.md`；主章「仅适用于 `BitVec`。见 [位向量模式](bitvec.md)。」
- 仍不写「一等」直到作者给出指称定义（5B）。

### 待定

- Q4：跨元素启用的**确切**属性约定（是否就是 `.dst true`，是否还依赖 `.pattern_view_defined` 等）。
- 「一等 pattern」的指称定义（5B 所需）。
- `one` 载荷类型名：`BitVec<7>` vs 作者草稿专用类型。
- 比特序（高位在前等）是否需在正式定义单列一句（当前写「与类型布局一致」）。

## 文档写入

- **是**：
  - `docs/control-flow/pattern-matching/bitvec.md`（七段）
  - `docs/control-flow/pattern-matching/index.md`（链接）
  - `docs/grammar/syntax`（`segment` / `bit_vec_*_pattern`）
  - `docs/README.md`、`docs/control-flow/index.md`（目录项）

## 需要你拍板的问题

1. `prompt` 第 4 题只有「。」——跨元素如何启用？
   - A. 就是配置属性 `.dst true`（可与 `.base_type` 同块）
   - B. 另有专用属性名（请写出）
   - C. 凡 pattern 总宽 > base 宽即跨元素，`.dst` 只是示例噪音
   - Other: …

2. 「一等 pattern」请给出你要写进文档的指称定义（5B）。示例骨架（供改写，非提案）：
   - Other: 「模式是 ……；因此 ……。」

3. UTF-8 `one` 臂载荷类型？
   - A. 正式文用 `BitVec<7>`（当前）
   - B. 用你草稿中的专用宽度整型名（请给出**正确拼写**的规范名）
   - Other: …

4. 比特切段顺序要不要在正式定义里写死？
   - A. 写「从 MSB 到 LSB（与 `0b` 字面书写一致）」
   - B. 只写「与该类型内存/逻辑布局一致」，不写 MSB/LSB
   - C. 另有端序规则（请述）
   - Other: …
