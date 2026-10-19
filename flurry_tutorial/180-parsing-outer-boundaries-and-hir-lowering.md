# Flurry 演进手记 (180)：语法解析与 HIR 降级——Multi 属性链、外层结构边界与 Take View 专用节点

在[上一篇（第 179 篇）](./179-inside-the-compiler-pipeline-phases-overview.md)中，我们俯瞰了 `luna` 编译器从 AST、HIR、Elaboration/Normalization、Freeze、Instantiation 直到 IIR 的完整流水线全景图。

今天，我们正式走进这条流水线的**第一站：`AST -> HIR`（语法解析与高层中间表示降级）**。
在 Flurry 的核心架构契约（`AGENTS.md` §5 与 `docs/compilation/syntax-and-hir.md`）中，有一条专门针对前端解析与 HIR 降级的铁律：

> **“每个 take view 操作在 AST lowering 时进入对应 HIR node kind；expr/pattern 解析保持外层结构边界和属性的 multi 前缀规则。”**

为什么这条规则如此重要，以至于被写进编译器不可破坏的语义边界里？
让我们通过三个真实的编译器前端工程挑战来揭晓答案：
1. **挑战一（`multi` 属性前缀链）**：当用户在列表元素或实参前面连续挂载多个属性 `^attr1 ^attr2 expr` 时，解析器如何做到既支持任意长度的连续前缀，又按**从内到外**的正确顺序包装，且绝不导致参数被求值两次？
2. **挑战二（外层结构边界 `stop_before_outer_block`）**：Flurry 既有无需括号的 `if cond { body }`，又有支持大括号扩展应用的 `Flag { enabled: true }` 和模式 `.data { value: _ }`。在 `if enabled { 1 }` 和 `if message matches .empty { true }` 里，解析器是怎么刹住车、不让 `enabled` 或 `.empty` 把属于 `if` 分支的 `{ ... }` 给一口吞掉的？
3. **挑战三（Take View 专用 HIR 节点）**：为什么 `value'type`、`value'bitvec`、`value'bitvec'type` 和 `tag'intrinsic` 在进入 HIR 时，必须变成专门的 HIR 节点种类（Node Kind），而绝对不能偷懒做成普通的“字符串字段查找”？

---

## 一、 挑战一：列表元素的 `multi` 属性链与“从内到外”包装

在 Flurry 的语法规则中，列表元素（如函数调用的实参、数组元素、结构体字段等）遵循 **`multi_element`** 产生式：

$$\text{multi\_element} \rightarrow \text{base\_element} \mid \text{\texttt{\textasciicircum}expr}\ \text{multi\_element}$$

这意味着，在一个实参前面，你可以写 0 个、1 个、2 个甚至任意多个 `^expr` 属性前缀！
比如这段代码（源自 `docs/compilation/syntax-and-hir.md`）：

```flurry
fn keep_flag(flag: bool) -> bool {
    flag
}

pub fn demo_multi_attrs() -> bool {
    keep_flag(^{ some_attr: 1 } ^{ some_attr: 2 } true)
}
```

### 1. `v0` 递归解析器的隐患与 AST 包装顺序

如果编译器前端写得过于粗糙，很容易在这里栽两个跟头：
- **跟头 A（顺序颠倒）**：把 `^attr1 ^attr2 true` 包成了 `Attr(attr2, Attr(attr1, true))`，颠倒了右结合的修饰层级。
- **跟头 B（深层递归爆栈或重复求值）**：在生成的机器码里不小心把带两个属性的实参求值了两次。

在 `luna` 的解析器与 AST Lowering 中，处理 `multi_element` 采用的是**迭代收集前缀 + 从内到外包装（`wrap_from_inner_to_outer`）**：

```text
  源码输入:  ^{ some_attr: 1 }   ^{ some_attr: 2 }   true
             ─────────────────   ─────────────────   ────
                prefixes[0]         prefixes[1]      base

  Step 1: 循环收集所有连续的 `^expr` 前缀 -> prefixes = [attr1, attr2]
  Step 2: 解析基底元素 base = BoolLit(true)
  Step 3: wrap_from_inner_to_outer(prefixes, base) (从右往左 / 从内向外折叠):
          ┌───────────────────────────────────────────────────┐
          │ Attr( attr1,                                      │  ◀── 外层属性
          │   ┌───────────────────────────────────────────┐   │
          │   │ Attr( attr2,                              │   │  ◀── 内层属性 (紧贴 true)
          │   │   BoolLit(true)                           │   │
          │   │ )                                         │   │
          │   └───────────────────────────────────────────┘   │
          │ )                                                 │
          └───────────────────────────────────────────────────┘
```

而且，无论实参上挂了多少层元属性（如 `^{ some_attr: 1 } ^{ some_attr: 2 } compute_flag()`），在运行时语义上，基底实参表达式 `compute_flag()` **依然严格只求值一次**！

---

## 二、 挑战二：外层结构边界（`stop_before_outer_block`）与 `matches` 传播

现在来看语法解析器最惊险的“急刹车”艺术。

在 Flurry 中：
- 构造结构体或发起扩展应用可以写：`Flag { enabled: true }`（标识符后面紧跟 `{ ... }`）；
- 匹配带字段的变体模式可以写：`.data { value: _ }`（点变体后面紧跟 `{ ... }`）；
- 而 `if` / `while` / `for` 的条件表达式后面，**也直接跟着分支代码块 `{ ... }`，不需要像 C 语言那样加圆括号 `if (...)`！**

### 1. 如果没有外层边界感知，会发生什么惨剧？

看这行最简单的代码：

```flurry
if enabled { 1 } else { 0 }
```

如果表达式解析器在解析 `if` 条件时“六亲不认、贪婪到底”，它看到标识符 `enabled` 后面跟着 `{ 1 }`，就会大喜过望地说：*“哈！这是一个叫 `enabled` 的扩展应用 `enabled { 1 }`！”*
等它把 `{ 1 }` 吞进条件表达式之后，外层的 `if` 傻眼了：*“我的条件算完了，可我的 then 分支代码块 `{ ... }` 被条件自己吃掉了！语法错误！”*

更微妙的是，当条件里出现 **`matches` 模式匹配表达式**时：

```flurry
enum Message {
    empty,
    data { value: i32 },
}

fn is_empty(message: Message) -> bool {
    if message matches .empty {
        true
    } else {
        false
    }
}
```

请注意 `if message matches .empty { true }`！
在这里，`if` 的条件是一个 `matches` 表达式：`message matches <pattern>`。
表达式解析器把接力棒交给了**模式解析器（Pattern Parser）**去解析 `.empty`。
如果只给表达式解析器加了“遇到 `{` 刹车”的开关，却**忘了把这个边界开关传播给 `matches` 右侧的模式解析器**，那么模式解析器看到 `.empty` 后面的 `{ true }`，就会以为你在写扩展变体模式 `.empty { true }`，再次把 `if` 的分支体吞掉！

### 2. `luna` 的优雅解法：`stop_before_outer_block` 与括号内层作用域恢复

`luna` 的解析器通过一个精确的上下文标志 `stop_before_outer_block` 彻底解决了这个问题：

```text
  [ 场景 A：直接处于 if 条件顶层 (stop_before_outer_block = true) ]
    if  message matches .empty  { true } else { false }
        ──────────────────────  ▲
                  │             └── 看到顶层 `{`，因为 stop_before_outer_block = true，
                  │                 模式 `.empty` 与表达式 `matches` 立即在此刹车！
                  └──────────────── `{ true }` 被完整留给外层 `if` 作为 then 分支体！

  [ 场景 B：用圆括号 (...) 显式开启闭合内层作用域 (stop_before_outer_block = false) ]
    if  (Flag { enabled: true }).enabled  { 1 } else { 0 }
        ▲                      ▲          ▲
        └── 进入 `(`：         └── 出 `)` └── 外层恢复 stop_before_outer_block = true，
            内层允许解析                      `{ 1 }` 作为 if 的 then 分支！
            `Flag { enabled: true }`！

  [ 场景 C：在圆括号内写带花括号的 matches 模式 ]
    if  (message matches .data { value: _ })  { true } else { false }
        ▲                                  ▲  ▲
        └── `(` 内允许 `.data { value: _ }`┘  └── `)` 外 `{ true }` 归属 if 分支！
```

### 3. 反例警告：在未加括号的 `if` 条件顶层直接写裸 `{ ... }` 构造或模式

如果有人在 `if` 条件顶层不加圆括号，直接写带 `{ ... }` 的扩展模式并紧跟分支块，导致大括号边界冲突，解析器会给出清晰的语法诊断：

```flurry
-- ❌ 编译期直接拒绝！
fn broken_unparenthesized_block_in_if(message: Message) -> bool {
    -- 错误示范：在 if 条件顶层未加圆括号就直接写 `.data { value: _ }`，
    -- 导致 `{ value: _ }` 被外层 if 当作分支块的开始，后面的 `{ true }` 沦为悬空语法错误！
    if message matches .data { value: _ } {
        true
    } else {
        false
    }
    -- ✅ 正确写法：用圆括号明确包裹带花括号的内层表达式/模式：
    -- if (message matches .data { value: _ }) { true } else { false }
}
```

---

## 三、 挑战三：Take View（`expr'id`）在 HIR Lowering 中进入专用节点

最后，我们来看 `AST -> HIR` 降级中最核心的一条语义守恒律：**Take View 专用节点降级**。

在前面的篇章里，我们已经用过了 Flurry 的三大视图操作符（`expr'id`）：
1. **`value'type`**：获取表达式 `value` 的静态类型（Type View）；
2. **`value'bitvec`**：获取表达式 `value` 的底层比特向量视图（BitVec View）；
3. **`tag'intrinsic`**：从指令标签获取编译器内置硬件指令可调用视图（Intrinsic View）。

甚至，它们还可以**链式组合**！比如：

```flurry
fn inspect_views(value: u32) -> Type {
    -- 先取 value 的 bitvec 视图 (BitVec<32>)，再取该视图的静态类型 (Type)！
    let bv = value'bitvec;
    value'bitvec'type
}
```

### 为什么绝对不能把 `expr'id` 当成普通的属性/字段名查找？

假设一个偷懒的编译器开发者在把 AST 降级到 HIR 时，把 `value'type` 和 `value.type` 混为一谈，或者只生成一个笼统的 `HirExpr::MemberAccess(value, "'type")` 字符串节点。这会引发严重的架构灾难：
- `value'type` 不求值 `value` 的运行期副作用，只提取静态类型；
- `place'bitvec` 在赋值左值位置（如 `raw'bitvec[0] = .1`）需要触发专门的 **BitVec 位置写回（Place Writeback）** 检查；
- `tag'intrinsic(args).as(T)` 需要被 Elaborator 和目标验证器直接识别为底层 Intrinsic 调用链。

因此，在 `luna` 的 **AST Lowering（`AST -> HIR`）** 阶段，每一个合法的 Take View 操作都会被**一对一地直接降级为专门的 HIR 节点种类（Dedicated HIR Node Kind）**：

| 表面源码语法 (AST `TakeView`) | AST Lowering 后的专用 HIR 节点结构 | 后续 Elaboration 阶段的语义职责 |
| :--- | :--- | :--- |
| **`value'type`** | `HirExpr::TypeOf(lower(value))` | 推算被操作数的类型并返回 `Type`，不产生运行期求值 |
| **`value'bitvec`** | `HirExpr::BitVecView(lower(value))` | 建立 `BitVec<N>` 值视图或左值位置写回（Writeback）契约 |
| **`value'bitvec'type`** | `HirExpr::TypeOf(HirExpr::BitVecView(lower(value)))` | 内外层视图严格保序嵌套，先建 BitVec 视图再取其 `Type` |
| **`tag'intrinsic`** | `HirExpr::IntrinsicView(lower(tag))` | 结合紧邻的 `(args)` 与 `.as(T)` 进行硬件原语签名与目标验证 |

```text
  [ 源码链式视图与调用 ]
    1. `value'bitvec'type`
       └──▶ AST: TakeView(TakeView(Ident("value"), "bitvec"), "type")
       └──▶ HIR: TypeOf(
                   BitVecView(
                     Local("value")
                   )
                 )

    2. `X86Instructions.fma'intrinsic(a, b, c).as(f32)`
       └──▶ HIR 完整保留调用与 `.as(f32)` 的结构邻接性：
            AsOutput(
              Call(
                IntrinsicView(Path("X86Instructions.fma")),
                [a, b, c]
              ),
              Type("f32")
            )
```

如果有人写了一个根本不存在的视图名（比如 `value'unknown_view`），AST Lowering 阶段就会当场识别出这不是任何已知的合法视图种类，直接报错拦截，绝不会让一个非法的字符串混进后续的类型推导器里！

---

## 本篇小结

| 前端阶段与机制 | 核心语法 / 转换规则 | 守护的语言契约 (`AGENTS.md` §5) |
| :--- | :--- | :--- |
| **`multi_element` 属性链** | `multi_element -> base_element \| ^expr multi_element` | 支持元素前连续挂载任意多个 `^expr`，迭代收集并从内向外（`wrap_from_inner_to_outer`）包装，基底实参仅求值一次 |
| **外层块边界控制** | `stop_before_outer_block = true`（进入 `(...)` 时置为 `false`） | 保证 `if enabled { 1 }` 不把 `{ 1 }` 误解析为 `enabled` 的扩展应用；圆括号内正常支持 `Flag { enabled: true }` |
| **`matches` 模式边界传播** | 将 `stop_before_outer_block` 传入 `matches` 右侧的模式解析器 | 保证 `if msg matches .empty { true }` 中 `.empty` 不会吞掉外层 `if` 的 `{ true }` 分支块 |
| **Take View 专用 HIR 节点** | `'type -> TypeOf`、`'bitvec -> BitVecView`、`'intrinsic -> IntrinsicView` | 每个 Take View 在 AST lowering 时直接进入对应 HIR node kind，保持链式组合顺序与 `.as(T)` 邻接关系 |

---

## 留给下一篇的悬念

现在，我们的代码已经顺利通过了语法解析和 AST Lowering，变成了一棵结构清晰、外层边界分明、所有 Take View 都各就各位的 **HIR（高层中间表示）**！

接下来，这棵 HIR 就要迈入整个 `luna` 编译器最核心、最精密的智慧中枢——**双向类型精化（Bidirectional Elaboration）与局部依赖调度器（Local Dependency Scheduler）**！
- 什么叫**双向类型精化（Synthesis $\Rightarrow$ vs Checking $\Leftarrow$）**？为什么 `.empty` 或整数字面量 `42` 在没有上下文时不知道自己是谁，一旦放进函数参数或返回值位置就能瞬间锁定类型？
- 在同一个函数签名 `fn copy_selected(comptime wide: bool, x: defer SelectType(wide))` 里，如果某个类型表达式依赖另一个还没算出来的局部项，编译器内部的 **Task（任务）、Waiter（等待者队列）与唤醒机制**是如何一步步解开依赖网的？
- 如果两个局部项互相等待对方（比如 `a` 的类型等 `b`，`b` 的类型又等 `a`），编译器如何精准检测出死锁并给出清晰的诊断，而不是卡死在无限循环里？

下一篇 **[Flurry 演进手记 (181)：双向类型精化与局部依赖调度——上下文传导、挂起队列与按需唤醒](./181-bidirectional-elaboration-and-local-dependency-scheduling.md)**，我们将揭开 Elaborator 的神秘面纱！
