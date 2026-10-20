# Flurry 演进手记 (124)：用单洞上下文组合出完整循环与状态机——有限程序组装（Program Construction）

> **阅读指南**：在上一回（第 123 回）结束时，我们提出了一个看似不可思议的挑战：CMTT 严格规定每一段 `'(...)` 最多只能有一个代码接入点（**单洞规则，Single-Hole Constraint**）。既然每次引用只能挖“一个洞”，我们怎么可能在编译期把任意 $N$ 条语句顺序串起来，或者把一张包含多条边与运行期回边（Back-Edge）的有限状态自动机（FSM）组装成完整的程序？本篇我们将结合 `docs/cmtt/program-construction.md`，掌握 **单洞链式累积（Single-Hole Chaining）** 与 **生成图 vs. 执行图分离** 的核心技巧。

---

## 一、 问题背景：当我们要生成多步程序或带环状态机时，怎么避免“双洞违例”与“无限递归展开”？

假设我们要在编译期干两件事：
1. 给定编译期次数 `count: usize`，生成一段连续执行 `count` 次动作的展开代码块；
2. 给定正则/协议自动机的状态转移图（注意：**状态 1 读入字符 `'b'` 或 `'c'` 时会跳回状态 1 自身**，即存在环路 Back-Edge。），生成对应的状态机执行代码。

### 掉进“双洞拼接”与“顺着状态回边无限递归生成”的两个大坑

初学者在第一次尝试写编译期程序组装器时，往往会掉进以下两个经典陷阱之一：

```flurry
-- 陷阱 A（违背单洞规则）：企图在一个 quote 里同时拼接左右两段代码
pure comptime fn broken_seq(first: < |- void >, second: < |- void >) -> < |- void > {
    -- ❌ 编译期直接拒绝：同一个 quote 主体里出现了 ~<first> 和 ~<second> 两个洞。
    '(do {
        ~<first>;
        ~<second>;
    })
}

-- 陷阱 B（把“运行时状态回边”当成“编译期生成递归”）：
-- 状态 1 遇到 'b' 跳回状态 1。如果生成器在编译期顺着转移边递归展开：
-- gen_state(1) ──▶ 调用 gen_state(1) ──▶ 调用 gen_state(1) ...
-- ❌ 编译期直接耗尽构造预算（Infinite Unrolling）。因为运行时的输入流可以任意长。
```

**核心问题**：
1. 怎么在不违反“每层 quote 最多一个洞”的前提下，把 $N$ 步操作或 $E$ 条状态转移边组装成一棵多层语法树？
2. 怎么区分 **编译期有限的生成图（Generation Graph）** 与 **运行期可循环的执行图（Execution Graph）**？

---

## 二、 核心机制：单洞链式累积——像包洋葱一样逐层构建任意大的程序

让我们先解决第一个问题：**单洞怎么串起多步程序？**

秘诀非常简单：在编译期使用一个可变代码变量 `var code: < Ψ |- T > = 初始基底代码;`，然后在编译期的 `for` 或 `while` 循环里，每一步先用 `const previous = code;` 锁住上一轮构造好的完整代码快照，再用一层只含 **`~<previous>` 这唯一一个洞** 的新 `'(...)` 把它包裹起来。

下面来看清 `repeated_message(3usize)` 是如何只靠“每层一个洞”，像包洋葱一样长成三层顺序语句的：

```text
  编译期循环组装过程 (var code: < |- void >)
  
  ▶ 初始基底 (Iteration 0):
    code₀ = '(())                                   -- 0 个洞
               │
               ▼ 第 1 轮包裹 (const previous = code₀)
    code₁ = '(do { ~<code₀>; println("tick"); })    -- 本层仅 1 个洞 (~<code₀>)!
               │
               ▼ 第 2 轮包裹 (const previous = code₁)
    code₂ = '(do { ~<code₁>; println("tick"); })    -- 本层仅 1 个洞 (~<code₁>)!
               │
               ▼ 第 3 轮包裹 (const previous = code₂)
    code₃ = '(do { ~<code₂>; println("tick"); })    -- 本层仅 1 个洞 (~<code₂>)!
               │
               ▼ 最终在 three_messages() 中执行 ~<code₃> 接入后的运行时代码：
  ┌───────────────────────────────────────────────────────────┐
  │ do {                                                      │
  │     do {                                                  │
  │         do { (); println("tick"); };                      │
  │         println("tick");                                  │
  │     };                                                    │
  │     println("tick");                                      │
  │ }  ──▶ 连续输出 3 行 "tick"，运行期完全没有循环计数器     │
  └───────────────────────────────────────────────────────────┘
```

### 用单洞链式循环生成多步语句与状态转移链

让我们先看这段来自 `docs/cmtt/program-construction.md` 的经典单洞累积示例：

```flurry
use std.prelude.*;

-- 在编译期循环中，每次仅用 1 个洞 ~<previous> 逐层累积代码。
pure comptime fn repeated_message(count: usize) -> < |- void > {
    var code: < |- void > = '(());
    for index in 0usize..count {
        -- 关键动作：先将上一轮的代码值冻结保存在 const previous 中。
        const previous = code;
        -- 新的一层 quote 里只有 ~<previous> 这 1 个洞，加上本层新增的语句。
        code = '(do {
            ~<previous>;
            println("tick");
        });
    }
    code
}

fn three_messages() {
    ~<repeated_message(3usize)>;
}

test {
    three_messages();
}
```

> **🔍 语法小贴士：为什么循环里要先写 `const previous = code;`？**
> 在 CMTT 中，`'(...)` 内部通过 `~<previous>` 引用的是一个**在编译期已确定的不可变代码值快照**。
> - `var code` 是生成器循环里用来迭代更新的编译期可变绑定；
> - 每次迭代开头写一句 `const previous = code;`，就把当前这一层的代码值稳稳地冻结在了 `previous` 别名里；随后对 `code = '(...)` 的重新赋值**绝不会篡改 `previous` 已经保存的旧值**，也不会造成自引用死循环。

---

## 三、 生成图 vs. 执行图：组装支持回边（Back-Edge）的正则自动机

现在，让我们解决第二个核心问题：**当自动机存在环路（比如正则表达式 `a[bc]*d` 中，状态 `1` 读入 `'b'` 或 `'c'` 会无限次跳回状态 `1` 自身）时，怎么在编译期组装它？**

`docs/cmtt/program-construction.md` 给出了极其深刻的架构洞察：
> **“自动机的状态数和边数在编译时有限。运行时图中存在回边，不要求生成器沿该边无限递归展开；生成器交付有限分派体和一个 runtime 循环即可。”**

换句话说：
1. **编译期生成图（Generation Graph）是有限无环的**：我们的自动机只有 4 条转移边 `edges: Array<Transition, 4>`。编译期生成器 `build_step` 只需要把这 4 条有限边从后往前包裹成一条 4 层的 `if-else` 分派链 `< state: u32, byte: u8 |- ?u32 >`。它的代码体积严格是 $O(E)$，只取决于边的数量 $E = 4$，跟运行时的输入字符串有多长毫无关系。
2. **运行期执行图（Execution Graph）承载回边循环**：在运行时的驱动函数 `matches_input` 里，用一个普通的运行期 `while position < n` 循环保存当前状态 `var state = 0u32;`，每读一个字节就调用一次生成好的单步分派器。当状态 `1` 读到 `'b'` 返回 `1` 时，运行期循环自然而然地实现了回边跳转。

```text
  ┌────────────────────────────────────┐     ┌────────────────────────────────────┐
  │ 1. 编译期生成结构 (O(E) 有限分派树)│     │ 2. 运行期执行状态图 (含回边循环)   │
  ├────────────────────────────────────┤     ├────────────────────────────────────┤
  │  build_step(edges) 逆序包裹 4 条边:│     │         'b' (98), 'c' (99)         │
  │                                    │     │              ┌──────┐              │
  │  if s == 0 and b == 'a' { 1 }      │     │              ▼      │              │
  │  else if s == 1 and b == 'b' { 1 } │     │  ┌───┐ 'a' ┌───┐    │  'd' ┌───┐   │
  │  else if s == 1 and b == 'c' { 1 } │     │  │ 0 ├────▶│ 1 ├────┘ ────▶│ 2 │   │
  │  else if s == 1 and b == 'd' { 2 } │     │  └───┘(97) └───┘     (100) └───┘   │
  │  else { null }                     │     │                             (接受) │
  │                                    │     │  • 运行期可循环经过状态 1 任意多次 │
  │  • 编译期只循环 4 次，绝不无限展开 │     │  • 额外空间复杂度严格为 O(1)       │
  └────────────────────────────────────┘     └────────────────────────────────────┘
```

### 完整实现识别 `a[bc]*d` 的编译期状态机生成器

让我们把这套设计写成完整的 Flurry 代码（注意看 `build_step` 是如何从数组末尾向前包裹，从而保证列表中排在前面的转移边在运行期优先尝试的）：

```flurry
use std.prelude.*;

struct Transition {
    source: u32,
    byte: u8,
    target: u32,
}

-- 编译期生成器：将 n 条转移边组装为开放分派代码 < state: u32, byte: u8 |- ?u32 >
pure comptime fn build_step(edges: Array<Transition, n>)
    -> < state: u32, byte: u8 |- ?u32 >
where n: usize
{
    -- 兜底基底：没有任何边匹配时返回 null
    var code: < state: u32, byte: u8 |- ?u32 > = '(null);
    var remaining = n;

    -- 从最后一条边向前逆序包裹，使得 edges[0] 处于最外层优先判断。
    while remaining > 0usize {
        remaining -= 1usize;
        const edge = edges[remaining];
        const previous = code;

        -- ✅ 每层 quote 严格只有 1 个洞 ~<previous>。
        -- edge.source、edge.byte、edge.target 作为编译期常量直接物化为立即数。
        code = '(if state == edge.source and byte == edge.byte {
            edge.target
        } else {
            ~<previous>
        });
    }
    code
}

fn step(state: u32, byte: u8) -> ?u32 {
    -- 定义识别 a[bc]*d 的 4 条有限状态转移边：
    -- 97='a', 98='b', 99='c', 100='d'
    const edges: Array<Transition, 4> = [
        Transition { source: 0u32, byte: 97u8, target: 1u32 },
        Transition { source: 1u32, byte: 98u8, target: 1u32 },
        Transition { source: 1u32, byte: 99u8, target: 1u32 },
        Transition { source: 1u32, byte: 100u8, target: 2u32 },
    ];
    const dispatch = build_step(edges);

    -- 只需在最外层做一次上下文替换，整个 4 层条件链的所有 state 和 byte 全部就位。
    ~[state = state, byte = byte]<dispatch>
}

-- 运行期驱动循环：消费整个输入数组，判断是否停在唯一接受状态 2u32
fn matches_input(input: Array<u8, n>) -> bool
where n: usize
{
    var position = 0usize;
    var state = 0u32;
    while position < n {
        let next? = step(state, input[position]) else { return false; }
        state = next;
        position += 1usize;
    }
    state == 2u32
}

test {
    -- "ad" -> true, "abcd" -> true, "abc" -> false, "add" -> false
    println("ad: {}", matches_input([97u8, 100u8]));
    println("abcd: {}", matches_input([97u8, 98u8, 99u8, 100u8]));
    println("abc: {}", matches_input([97u8, 98u8, 99u8]));
    println("add: {}", matches_input([97u8, 100u8, 100u8]));
}
```

看看 `step` 函数里的最后一行 `~[state = state, byte = byte]<dispatch>`：
虽然 `dispatch` 内部包含了 4 层嵌套的 `if state == ... and byte == ...`，但因为每一层在构造时都共享同一个公开上下文接口 `< state: u32, byte: u8 |- ?u32 >`，我们在最外层**只需要写一次替换映射**，就能一次性把全部 4 层的虚拟端口同时连接到 `step` 函数的形参 `state` 和 `byte` 上。

---

## 本篇小结

| 组装技巧 / 原则 | 实现方式 | 核心工程价值 |
| :--- | :--- | :--- |
| **单洞链式累积** | `const previous = code; code = '(... ~<previous> ...);` | 严格遵守单洞规则，却能在线性循环中构造出任意层数的顺序或分支结构 |
| **逆序包裹保序分派** | 从 `remaining = n` 倒序递减包裹 `if ... else { ~<previous> }` | 保证排在数组前面的高优先级转移规则处于生成的 `if-else` 最外层 |
| **外层一次替换贯通全局** | 每层保持相同接口 `< state: u32, byte: u8 \|- ?u32 >` | 最终接入时只需一次 `~[state = state, byte = byte]` 即可贯通所有层级 |
| **生成图与执行图分离** | 编译期只展开有限边集合 $O(E)$；运行期用 `while` 承载回边 | 彻底避免沿状态机环路无限递归展开，运行期仅需 $O(1)$ 状态空间 |

---

### 下一篇预告

在这一篇里，我们已经用 `build_step` 把状态转移表 `[Transition { source, byte, target }, ...]` 编译成了一条平坦的 `if-else` 链，消灭了运行期查表（Table Lookup）的开销。

但是，仔细观察刚才生成的 `if-else` 链：
```flurry
if state == 0u32 and byte == 97u8 { 1u32 }
else if state == 1u32 and byte == 98u8 { 1u32 }
else if state == 1u32 and byte == 99u8 { 1u32 }
else if state == 1u32 and byte == 100u8 { 2u32 }
```
你发现可以继续优化的地方了吗？
1. 当 `state == 1u32` 时，如果输入的字节是 `100u8`（`'d'`），上面这段代码竟然要把 **`state == 1u32` 重复判断三次**。
2. 而且，在真实的二进制协议解析里，状态转移往往不只是单字节相等，还会包含**位段模式解构**（比如高 4 位是操作码、低 4 位是载荷长度）以及**载荷捕获（Captures）**。

在下一篇 **《Flurry 演进手记 (125)：实战 CMTT：把声明式状态转移表编译成内联的零开销跳转代码》** 中，我们将把 116–124 学到的 `< S -| ... >` 模式代码、上下文替换与状态机组装结合起来，升级打造一个**按状态分组、支持动作与捕获**的高性能协议状态机编译器。
