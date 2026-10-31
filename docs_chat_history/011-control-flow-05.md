# 011 — control-flow — 05

- **日期**: 2026-08-09
- **global_index**: 011
- **theme**: control-flow
- **subindex**: 05
- **本轮目标**: 消化模式匹配 `prompt`；锁定种类清单与若干规则；澄清 Q5；评估 `case` 模式定义示例；追问形式化缺口；**暂不**改 pattern 正式文（1C：清单确认后再写；`case` 细节未齐）。

---

## 作者本轮输入摘要

见 `010` 末「作者答复（prompt）」：1C、2A、3A、4A、5 反问 AB 是否不同、6A、7 长示例（`case` 定义）、8「。」、9A、10C。

---

## 评估

### 对问题 5（`?` / `!` / `error`）

**A 与 B 在已给 grammar 下通常是同一套表面：**

- A：`pat?`、`pat!`、`error pat`（`error_pattern -> error pattern`）。
- 现行文 `error e`：即 `error` + 模式 `e`（标识符绑定），**落在 A 内**，不是第二种无关语法。

若你还想要别的形态（例如强制 `error(e)`、或 `err e` 关键字），才与 A 不同——请在本轮问题 1 说明。默认按 **A（含 `error e`）** 锁定。

### 已锁定（模式匹配）

| ID | 结论 |
|----|------|
| PM1 | 产出：先确认种类清单再写正式文（1C）。 |
| PM2 | v1 收录现行文已列种类（2A）+ 位向量（6A）+ `case` 自定义模式定义（7）。 |
| PM3 | 模式守卫：`pat if cond`；因无「以 `if` 开头的 pattern」，不必改成 `while`（3A）。 |
| PM4 | 保留 `pattern and expr is pattern` 链（4A）。 |
| PM5 | 可选/错误：`?` / `!` / `error pat`（见上澄清）。 |
| PM6 | 位向量模式入章（6A）。 |
| PM7 | 穷尽性：采纳 8 → 多臂 `is do` / `match` 须穷尽（或 `_`）；单臂不强制穷尽（对齐 010-Q8A）。 |
| PM8 | `let`/参数/闭包参数可提模式与穷尽（9A）。 |
| PM9 | `Iter` = 迭代器 trait；`IntoIterator` = 容器可选的默认迭代（10C）；已写入 `basic.md`。 |

### `case` 模式定义（作者示例）— 可行性与缺口

意向清楚：**在类型 / `extend` 上用 `case` 声明可匹配的视图**；匹配侧写作 `.name(...)`；块内计算并返回「参数元组外包一层 option」之类结果（成人例）；也可是谓词型 case（`f64` 无穷）；`Bitvec` 上可与 `0b(...)` 模式配合且标注 exhaustive。

**纯度三级 → 内存管理章；可分析性三级 → 暂定**——本章只陈述有此分级，不展开（反抽象泄漏 / 勿臆造三级定义）。

示例中的**表面问题**（写入前须改，不当作已确认语法）：

- 控制流已确认要 `do`，示例仍写 `student match` / 无 `do` 的 `is`——匹配侧应用已确认形态。
- `"Alice".to_string()`、`String` 等未确认 API。
- `self'bitvec`、`0xFFF0_…` 等依赖 bitvec / 字面量已确认部分。
- `(operand, )` 单元素元组 trailing comma 是否合法未确认。
- `enum Op { .preferred_bit_width 10, one_addr.{…} }` 属枚举/层级语法，非本章单独能定。

### 种类清单（v1，待你点头后开写）

控制流形态（已在 basic）：`if is … do`、`match`、`while is … do`、`for … in`。

模式：字面量、绑定、`_`、符号、结构体/记录、枚举变体/应用、列表、范围、`or`、`and is`、`as`、`pat if`、`?`、`!`、`error pat`、位向量、`case` 定义的自定义模式；`let`/参数/闭包参数中的模式（短节）。

暂不展开：纯度/可分析性三级细节；枚举层级语法本体。

---

## 需要你拍板的问题

1. **`error` 模式：是否就确认 = `error` + `pattern`（故 `error e` 合法）？**
   - A. 是，无其它形态。
   - B. 另有 `error(e)` 等（请写产生式）。
   - Other: …

2. **`case` 定义的规范产生式？**（据你示例草拟，待选）
   - A. `case id (param*) block` 与 `case id block`（无参谓词/零元）；仅出现在 `struct`/`extend`/`enum` 体内（请圈定允许的宿主）。
   - B. 另需 `case id(param*) => expr` 单表达式形。
   - C. 你改写产生式。
   - Other: …

3. **`case` 块的结果约定？**
   - A. 必须返回 `?Args`（或等价）：`null` 表示不匹配；`Some` 载荷与 case 参数对应（成人例）。
   - B. 谓词 case（`f64` 无穷）返回 `bool`；解构 case 返回 `?Tuple`——两种签名并存。
   - C. 统一一种返回类型（请点名）。
   - Other: …

4. **匹配侧如何写自定义 case？**
   - A. 一律 `.case_name(pat*)` / `.case_name`（与符号/变体同形）。
   - B. 其它。
   - Other: …

5. **本章对「纯度 / 可分析性」写多少？**
   - A. 各一句：存在三级；纯度见内存管理；可分析性待定。
   - B. 完全不提，等专章。
   - Other: …

6. **是否现在开写 pattern 正式文（七段）？**
   - A. 是：按清单写入 `pattern-matching/index.md`；`case` 节用已确认点 + 你示例改写后的合法形态；未确认处标「待定」并仍先问——或只写已钉部分。
   - B. 先 history 七段草案（含 `case`）。
   - C. 再答 1–5 后再写。
   - Other: …

---

## 已确认 / 仍待定

- **已确认**：PM1–PM9；basic 已补 Iter/IntoIterator。
- **待定**：`error` 是否仅 A；`case` 产生式/返回类型/匹配侧；纯度文笔；写入时机。

---

## 文档写入

- `basic.md` Iter/IntoIterator：**是**（小补）。
- `pattern-matching/index.md`：**否**。

---

## 作者答复（prompt）

日期：2026-08-09

来源：仓库根目录 `prompt`（全文照录）

```
1 就是`error`关键字加一个pattern，这与错误限定类型中错误分支的构造表达式语法是对偶的，即`error expression`
    值得一提的是，`pattern !`和option some的`pattern?`也是与相应类型的elimination/propagation表达式语法对偶，非常优雅的设计

2 不，演进一下，case id(param*) -> return_type_expr clauses* body, param一般是*self或者self,
    而返回值一般是一个option type, 内部是一个Tuple，这样更严谨，比如：
    [pub case deref(*self) -> ?Tuple<...> where requires ... { ... }]

3 。
4 A
5 在本章，但是留空待定
6 A
```
