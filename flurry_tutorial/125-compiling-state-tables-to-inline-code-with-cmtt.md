# Flurry 演进手记 (125)：实战 CMTT：把声明式状态转移表编译成内联的零开销跳转代码

> **阅读指南**：在上一回（第 124 回）中，我们用单洞链式累积把一张 4 条边的状态表编译成了简单的 `if-else` 链。今天，我们将迎来第六卷上半程的综合实战——结合 `docs/cmtt/program-construction.md` 与 `docs/cmtt/patterns-and-arms.md`，把第 116–124 回学到的 **表达式代码 `< Ψ |- T >`**、**模式代码 `< T -| Ψ >`**、**Quoted Arm** 与 **两层状态分组（Two-Level State Grouping）** 融会贯通，手把手写出一个把声明式协议状态转移表直接编译为**零查表开销、零重复状态判断、且带安全载荷提取**的内联状态机代码生成器。

---

## 一、 问题背景：扁平 `if-else` 链的重复判断与“捕获数据泄露”风险

在上一回的 `build_step` 中，我们把每一条边 `(source, byte, target)` 都无脑生成了一句 `if state == edge.source and byte == edge.byte`。当协议状态机变大时，这种初级生成方案暴露出了两个明显的短板：

### 同一个 `state` 被反复比较，且无法提取协议载荷

```flurry
-- 回顾 传统生成的扁平分派代码：
if state == 0u32 and byte == 0xAAu8 { ... }
else if state == 1u32 and byte == 0x01u8 { ... }
else if state == 1u32 and byte == 0x02u8 { ... }
else if state == 1u32 and byte == 0x03u8 { ... }
else if state == 1u32 and byte == 0x04u8 { ... }

-- 短板 1（冗余的状态比较）：
--   当处于 state == 1 时，为了匹配 0x04，CPU 却连续比较了 4 次 `state == 1u32`。
-- 短板 2（缺乏载荷捕获与累加）：
--   真实的协议帧非但要识别“合法与否”，还要在经过特定状态边时提取有效载荷字节
--   （Capture），并在匹配失败时保证不泄露半成品脏数据。
```

**核心诉求**：
我们希望编译期生成器能像一位经验丰富的系统架构师那样：
1. **外层按 `state` 聚合，内层按 `byte` 分派**：同一个源状态 `s` 只做**一次** `state == s` 判断，进去之后直接对 `byte` 做紧凑分支；
2. **结合模式代码 `< T -| Ψ >` 与动作代码**：支持用模式匹配解构输入帧，并在只有整条协议流成功到达接受状态（Accepting State）时才向调用者交付捕获结果。

---

## 二、 核心机制：两级单洞组装——外层状态树 + 内层字节分派树

怎样在不违反 CMTT **单洞规则**的前提下，在编译期把一张扁平的转移表 `edges: Array<Transition, n>` 变成“外层按状态分派、内层按字节分派”的两级嵌套结构？

答案是：**把生成器拆成两个协同工作的 `pure comptime fn`。**
1. **内层生成器 `build_byte_dispatch(edges, target_state)`**：遍历转移表，只挑出 `edge.source == target_state` 的那些边，用单洞链组装出只依赖 `byte` 的内层代码 **`< byte: u8 |- ?StepOutcome >`**。
2. **外层生成器 `build_state_machine(edges, state_count)`**：按状态编号 `s` 从 `0` 到 `state_count` 遍历，先调用 `build_byte_dispatch` 拿到状态 `s` 的内层字节分派代码，再用 `~<...>` 嵌入外层状态分支，最终组装出 **`< state: u32, byte: u8 |- ?StepOutcome >`**。

下面来看清两级单洞生成器是如何把扁平表变成紧凑两级分派树的：

```text
  声明式状态转移表 edges:
    (s=0, b=0xAA -> s=1), (s=1, b=10 -> s=1, +10), (s=1, b=20 -> s=1, +20), (s=1, b=0xFF -> s=2)
                                         │
         ┌───────────────────────────────┴───────────────────────────────┐
         ▼ 1. 内层生成器 build_byte_dispatch(edges, s=1)                 ▼
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 过滤出 source == 1 的 3 条边，生成 < byte: u8 |- ?StepOutcome >：       │
  │   if byte == 10u8      { StepOutcome { next: 1, delta: 10 } }           │
  │   else if byte == 20u8 { StepOutcome { next: 1, delta: 20 } }           │
  │   else if byte == 0xFF { StepOutcome { next: 2, delta: 0  } }           │
  │   else { null }                                                         │
  │   ✨ 注意：内层代码只依赖 `byte`，完全没有重复的 `state == 1` 判断      │
  └──────────────────────────────────────┬──────────────────────────────────┘
                                         │
                                         ▼ 2. 外层生成器 build_state_machine
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 将每个状态的字节分派树包裹进 < state: u32, byte: u8 |- ?StepOutcome >： │
  │   if state == 0u32 {                                                    │
  │       if byte == 0xAAu8 { ... } else { null }                           │
  │   } else if state == 1u32 {                                             │
  │       -- 这里 state == 1u32 整个状态只判断【一次】                      │
  │       ~<state_1_byte_dispatch>                                          │
  │   } else { null }                                                       │
  └─────────────────────────────────────────────────────────────────────────┘
```

### 解决两级组装中的“双洞挑战”

等等。聪明的你一定马上意识到了一个关键问题：
在外层生成器的循环里，如果我想写：
```flurry
-- 慢着。如果在同一个 quote 里既接入本状态的 ~<byte_code>，
-- 又在 else 分支接入后续状态的 ~<previous>，这岂不是有了【两个洞】？
'(if state == s { ~<byte_code> } else { ~<previous> })
```
**抓得太准了。** 正如我们在第 118 回所学，同一个 `'(...)` 的两个分支各写一个 `~<...>` 会构成 2 个洞，直接被编译器报错。

那么在 CMTT 中，怎样用**严格单洞**把“当前状态的字节分派”与“其余状态的兜底分派”拼在一起呢？
有两种简洁清晰的正统解法：
- **解法 A（展开为带 `state` 守卫的单洞链，或将单状态分派封装为 `quote fn` / 参数化步骤）**；
- **解法 B（为内层生成器传入一个“未命中时的回退代码洞 `fallback: < state: u32, byte: u8 |- ?StepOutcome >`”）**。

看看 **解法 B（单洞延续传递，Continuation-Style Single Hole）** 有多妙：
内层生成器在最底部省略死 `'(null)`，而是直接以传入的 `fallback` 作为初始基底。这样每一层包裹永远只有一个 `~<previous>`，从头到尾每一份 `quote` 都严格只有 **1 个洞**。

```flurry
use std.prelude.*;

struct FrameEdge {
    source: u32,
    byte: u8,
    target: u32,
    weight: u32,
}

struct StepOutcome {
    next_state: u32,
    added_weight: u32,
}

-- 基于延续基底（fallback）的单洞状态分派生成器
pure comptime fn compile_edges_for_state(
    edges: Array<FrameEdge, n>,
    target_source: u32,
    fallback: < state: u32, byte: u8 |- ?StepOutcome >,
) -> < state: u32, byte: u8 |- ?StepOutcome >
where n: usize
{
    -- 从 fallback 延续开始，只为 source == target_source 的边包裹内层分支
    var inner: < state: u32, byte: u8 |- ?StepOutcome > = fallback;
    var i = n;
    while i > 0usize {
        i -= 1usize;
        const edge = edges[i];
        if edge.source == target_source {
            const prev_inner = inner;
            -- ✅ 严格只有 1 个洞 ~<prev_inner>。
            inner = '(if state == target_source and byte == edge.byte {
                StepOutcome {
                    next_state: edge.target,
                    added_weight: edge.weight,
                }
            } else {
                ~<prev_inner>
            });
        }
    }
    inner
}

pure comptime fn compile_table(
    edges: Array<FrameEdge, n>,
    num_states: u32,
) -> < state: u32, byte: u8 |- ?StepOutcome >
where n: usize
{
    var code: < state: u32, byte: u8 |- ?StepOutcome > = '(null);
    var s = num_states;
    while s > 0u32 {
        s -= 1u32;
        -- 将上一状态构建好的完整代码作为 fallback 传给当前状态按组聚合。
        code = compile_edges_for_state(edges, s, code);
    }
    code
}
```

---

## 三、 结合模式代码 `< T -| Ψ >` 与安全捕获交付

在 `docs/cmtt/program-construction.md` 中特别强调了一条安全准则：
> **“若匹配需要返回捕获，生成器还须确定捕获槽、成功交付及失败路径……失败无法泄露未初始化捕获。”**

下面我们把完整的协议帧状态机跑起来，并配合第 120 回学到的 **模式代码 `< ?u32 -| payload: u32 >`** 来安全地解构与消费状态机的最终捕获结果：
- 协议规定：帧以 `0xAA` 开头（状态 `0 -> 1`），中间可携带任意多个权重字节 `10u8` 或 `20u8`（状态 `1 -> 1` 累加权重），并以 `0xFF` 帧尾结束（状态 `1 -> 2` 接受状态）。
- 如果中途遇到非法字节，或者输入结束时没停在接受状态 `2u32`，则立即返回 `null`，不把中途累加的半成品 `total_weight` 泄露给调用者。

```flurry
-- 完整的零查表开销协议状态机 + 模式代码捕获提取
fn execute_protocol_fsm(packet: Array<u8, n>) -> ?u32
where n: usize
{
    const TABLE: Array<FrameEdge, 4> = [
        FrameEdge { source: 0u32, byte: 0xAAu8, target: 1u32, weight: 0u32 },
        FrameEdge { source: 1u32, byte: 10u8,   target: 1u32, weight: 10u32 },
        FrameEdge { source: 1u32, byte: 20u8,   target: 1u32, weight: 20u32 },
        FrameEdge { source: 1u32, byte: 0xFFu8, target: 2u32, weight: 0u32 },
    ];
    const STEP_CODE = compile_table(TABLE, 2u32);

    var pos = 0usize;
    var cur_state = 0u32;
    var captured_weight = 0u32;

    while pos < n {
        let cur_byte = packet[pos];
        -- 接入编译期生成的零查表分派代码。
        let outcome? = ~[state = cur_state, byte = cur_byte]<STEP_CODE> else {
            -- 任何非法转移立即失败退出，不泄露半成品 captured_weight。
            return null;
        }
        cur_state = outcome.next_state;
        captured_weight += outcome.added_weight;
        pos += 1usize;
    }

    -- 只有完整消费输入且停在接受状态 2u32 时，才交付捕获结果。
    if cur_state == 2u32 {
        captured_weight
    } else {
        null
    }
}

-- 使用模式代码 `< ?u32 -| total: u32 >` 安全消费状态机捕获结果
fn verify_frame(
    packet: Array<u8, n>,
    comptime accept_pat: < ?u32 -| total: u32 >,
    comptime on_success: < total: u32 |- u32 >,
) -> u32
where n: usize
{
    execute_protocol_fsm(packet) match {
        ~<accept_pat> => ~<on_success>,
        _ => 0u32,
    }
}

test {
    -- 合法帧：[0xAA, 10, 20, 10, 0xFF]，捕获总权重 = 40，加 2 后输出 42
    let ok_frame = [0xAAu8, 10u8, 20u8, 10u8, 0xFFu8];
    -- 未闭合帧：[0xAA, 10, 20]（缺少 0xFF 帧尾），中途虽算了 30 但不泄露，输出 0
    let incomplete_frame = [0xAAu8, 10u8, 20u8];

    let r1 = verify_frame(ok_frame, '<total?>, '(total + 2u32));
    let r2 = verify_frame(incomplete_frame, '<total?>, '(total + 2u32));
    println("valid frame: {}, incomplete frame: {}", r1, r2);
}
```

如果有人企图在编译期把宿主编译器的某个内部指针地址塞进 `FrameEdge` 带到运行期，或者在失败分支里读取未初始化的捕获绑定，编译器会在编译期立即拦截：

```flurry
fn broken_capture_leak(packet: Array<u8, 3>) -> u32 {
    const pat: < ?u32 -| total: u32 > = '<total?>;
    const body: < total: u32 |- u32 > = '(total);

    execute_protocol_fsm(packet) match {
        ~<pat> => ~<body>,
        -- ❌ 编译期直接报错。
        -- 当 execute_protocol_fsm 返回 null 走入 _ 分支时，
        -- 模式代码 pat 并未匹配成功，捕获绑定 total 在此分支全然无存。
        _ => ~<body>,
    }
}
```

---

## 本篇小结

| 实战模块 | CMTT 核心技术组合 | 运行期收益与安全保障 |
| :--- | :--- | :--- |
| **声明式状态表表示** | `const TABLE: Array<FrameEdge, 4>` | 协议规则清晰集中，在编译期完成遍历与过滤 |
| **单洞延续式按组编译** | `compile_edges_for_state(..., fallback)` | 严格遵守单洞规则，将状态转移表展开为内联分派树，零运行期查表开销 |
| **捕获隔离与事务交付** | 仅当 `cur_state == ACCEPT` 时交付 `captured_weight` | 半截帧或非法字节中途退出时返回 `null`，不泄露脏捕获数据 |
| **模式代码对接捕获** | `< ?u32 -\| total: u32 >` (`'<total?>`) | 仅在成功匹配非空捕获时向后续处理代码暴露 `total` 绑定 |

---

### 下一篇预告

现在，我们已经能够熟练地手写复杂的 `match` 表达式（包括位向量分段模式 `0b110(ys: BitVec<5>) 10(zs: BitVec<6>)`、元组解构、范围模式、`or` 模式与守卫 `if cond`），也能用 CMTT 的 `use evaluate_rule` 和代码生成器在编译期生成包含几十条分支的 `match` 代码了。

这就引出了一个直击编译器后端灵魂的问题：
当我们写下（或生成）一个包含几十条复杂分支的 `match`（比如 UTF-8 变长字节解码、IEEE 754 浮点数分类、或者复杂的协议包头匹配）时，**Flurry 编译器内部难道真的是笨拙地从第 1 条分支到第 $N$ 条分支逐条从头比对吗？**
- 如果前三条分支都要求高 8 位是 `11111111`（比如判断 `NaN` 和 `Infinity`），编译器怎么自动提取公共掩码测试，变成一棵深度最小的**匹配自动机（Matching Automaton / Decision Tree）**？
- 在合并和重排测试顺序时，如果某条分支带有可能产生副作用或依赖前面绑定的守卫 `if accept(item.ready)`，编译器又怎样保证**求值次数、顺序与仿射资源清理 100% 不变**？

下一篇 **《Flurry 演进手记 (126)：模式匹配背后的超优化引擎——匹配自动机（Matching Automata）与决策树合成》**，我们将深入 Flurry 的模式编译与程序合成引擎。
