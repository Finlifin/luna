# Flurry 演进手记 (126)：模式匹配背后的超优化引擎——匹配自动机（Matching Automata）与决策树合成

> **阅读指南**：在上一回（第 125 回）中，我们用 CMTT 把状态转移表编译成了内联分支。而在实际系统编程中——无论是用 CMTT 生成的 `match` 分支，还是程序员手写的复杂结构体、枚举、范围与位向量分段模式（如 `0b(_: u1) 11111111 (mantissa: BitVec<23>)`）——一个 `match` 往往包含几十条彼此交织的候选规则。Flurry 编译器在把 `match` 降级为机器码时，是如何避免从上到下笨拙地重复测试相同字段与相同比特位的？本篇我们将结合 `docs/control-flow/pattern-matching/index.md`、`docs/control-flow/pattern-matching/bitvec.md` 与 `docs/program-synthesis/bitvec-algebra.md`，揭秘模式匹配背后的超优化引擎：**匹配自动机（Matching Automata）与决策树合成（Decision Tree Synthesis）**。

---

## 一、 问题背景：从上到下“逐行硬试”的朴素匹配有多慢？

假设我们在编写一个 IEEE 754 单精度浮点数 `f32` 分类器，或者一个 UTF-8 变长字节解码器。我们利用 Flurry 强大的 **位向量分段模式（BitVec Segment Pattern）** 写下了如下代码：

```flurry
use std.prelude.*;

fn classify_f32(num: f32) -> str {
    num'bitvec match {
        0b(_: u1) 11111111 (mantissa: BitVec<23>)
            if mantissa != 0 => "NaN",
        0b(_: u1) 11111111 00000000000000000000000 => "Infinity",
        0b0 00000000 00000000000000000000000 => "+Zero",
        0b1 00000000 00000000000000000000000 => "-Zero",
        _ => "Finite",
    }
}
```

### 朴素逐臂回溯（Naive Row-by-Row Backtracking）的性能开销

如果把这 5 条分支机械地翻译成一串孤立的 `if-else`：
1. **第 1 行测试**：先掩码取出中间 8 位指数 `(bits >> 23) & 0xFF == 0xFF`，再切出低 23 位 `mantissa` 判断 `mantissa != 0`；
2. **第 2 行测试**：万一 `mantissa == 0`（它是无穷大 `Infinity`），第 1 行失败进入第 2 行——朴素翻译器竟然**又重新做了一次 `(bits >> 23) & 0xFF == 0xFF` 的掩码与比较**。
3. **第 3、4 行测试**：接着又连续两次测试指数位是否为 `00000000` 和尾数是否为 `0`。

当匹配规则从 4 条增长到 RISC-V / x86 指令集解码器的 400 条时，这种重复切位、重复读取结构体字段的朴素代码会让 CPU 分支预测器彻底崩溃。

---

## 二、 匹配自动机与公共前缀决策树合成

在 Flurry 中，模式匹配不仅仅是一套好看的语法糖，它的底层连接着编译器的 **决策树与匹配自动机合成引擎（Matching Automata & Decision Tree Synthesis）**。

编译器会把一个多臂 `match` 看作一张 **模式矩阵（Pattern Matrix）**：
- **提取公共判别列（Column Selection & Common Prefix Factoring）**：对于上面 `classify_f32` 的位向量模式，编译器一眼看出所有分支都在观察 `[23..31]` 这 8 位指数段（Exponent）。
- **一次测试，多路分流（Switch / Mask Dispatch）**：合成引擎先对指数段做**唯一一次**提取与判断：
  - 若指数为 `0b11111111`（`255`），直接进入 `{NaN, Infinity}` 子树，此时只需看一次 `mantissa != 0` 即可瞬间分出 `NaN` 和 `Infinity`，**绝对不会第二次重复检查指数位**。
  - 若指数为 `0b00000000`（`0`）且尾数为 `0`，再看最高位符号位 `sign` 是 `0` 还是 `1`。

下面对比“朴素逐行测试”与“合成后的匹配自动机决策树”：

```text
  ┌─ 朴素逐臂回溯 (大量重复位掩码测试) ─────────────────────────────────────┐
  │ Arm 1: 测 exp == 0xFF ──▶ 测 mantissa != 0 (若失败，丢弃已知的 exp 事实!)│
  │ Arm 2: 又测一遍 exp == 0xFF (冗余!) ──▶ 测 mantissa == 0                 │
  │ Arm 3: 测 sign == 0 ──▶ 测 exp == 0 ──▶ 测 mantissa == 0                 │
  │ Arm 4: 测 sign == 1 ──▶ 又测一遍 exp == 0 和 mantissa == 0 (冗余!)       │
  └─────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼ 匹配自动机与决策树合成
  ┌─ 合成后的零冗余决策树 (每个比特段在每条路径上至多测试一次!) ────────────┐
  │                     [ 提取并测试一次 exp = bits[23..31] ]               │
  │                    /                  │                  \              │
  │        exp == 0xFF                    │ exp == 0x00       \ 其他        │
  │              ▼                        ▼                    ▼            │
  │   [ 测 mantissa != 0 ]       [ 测 mantissa == 0 ]       "Finite"        │
  │      /            \             /            \                          │
  │   true           false       true           false                       │
  │    ▼               ▼          ▼               ▼                         │
  │  "NaN"        "Infinity"  [ 测 sign ]     "Finite"                      │
  │                            /       \                                    │
  │                         0 /         \ 1                                 │
  │                       "+Zero"     "-Zero"                               │
  └─────────────────────────────────────────────────────────────────────────┘
```

### 跨元素位向量模式与掩码不相交判定

更强大的是，当我们在协议或编码解析中使用配置属性（`.base_type`、`.discrete_tag`、`.dst`）定义跨元素的位向量枚举模式时（见 `docs/control-flow/pattern-matching/bitvec.md`），编译器不仅能自动校验每一条位模式的宽度对齐，还能利用**位向量约束求解器**，通过快速掩码公式：
$$((v_1 \oplus v_2) \;\land\; (m_1 \land m_2)) \neq 0$$
在编译期瞬间判定两条位段模式是否**互斥（Disjoint）**。如果两条分支的模式被证明互斥，合成引擎就可以在不改变语义的前提下，根据目标架构的代价模型自由选择最优的掩码合并与跳转表（Jump Table）布局。

```flurry
use std.prelude.*;

-- 利用位向量分段模式定义 UTF-8 变长编码布局
-- 编译器自动核验每条分支的段宽总和（8, 16, 24, 32 bits）都是 base_type (u8) 的整数倍，
-- 并根據前导比特 0 / 110 / 1110 / 11110 合成零回溯的位掩码决策树。
enum Utf8CodePoint {
    .base_type u8,
    .discrete_tag true,
    .dst true,

    one:   0b0(ascii_char: BitVec<7>),
    two:   0b110(ys: BitVec<5>) 10(zs: BitVec<6>),
    three: 0b1110(xs: BitVec<4>) 10(ys: BitVec<6>) 10(zs: BitVec<6>),
    four:  0b11110(ws: BitVec<3>) 10(xs: BitVec<6>) 10(ys: BitVec<6>) 10(zs: BitVec<6>),
}

test {
    let num: f32 = 13.625,
    if num'bitvec matches 0b(sign: u1) (exponent: u8) (mantissa: BitVec<23>)
        if exponent == 130 and mantissa == 0b10110100000000000000000
    {
        println(
            "sign: {}, exponent: {}, mantissa: {}",
            sign'bitvec.as(u8),
            exponent,
            mantissa.as(u32),
        );
    }
}
```

---

## 三、 严格语义边界：优化绝不破坏副作用、所有权与守卫次序

听到“编译器会自动提取公共测试并重排合成决策树”，经验丰富的系统程序员马上会警惕起来：
**“如果我的模式里带有模式守卫 `if accept(item.ready)`、自定义 `case` 视图、或者仿射资源移动（Move）与局部 `Drop` 对象，决策树优化会不会偷偷打乱函数调用的次数和顺序，甚至提前消费掉后面分支还要用的资源？”**

请放一百个心。在 `docs/control-flow/pattern-matching/index.md` 与 `docs/program-synthesis/bitvec-algebra.md` 中，Flurry 为匹配自动机立下了铁一般的 **语义保持红线（Semantics-Preserving Invariants）**：

> **💡 核心心智模型：优化只改变纯判别路径，绝不改变可观察行为与资源事务。**
> 1. **主语（Scrutinee）与计算型端点严格求值一次**：被匹配的表达式只求值一次；范围模式中的计算型端点按源码顺序求值一次。
> 2. **候选失败时的“事务性回滚（Transactional Rollback）”**：
>    - 在尝试某条候选模式（包括 `or` 模式的左支 `((first, second) if accept(first.ready))`）时，在守卫 `accept(...)` 还没返回 `true` 之前，编译器**绝不会提前把仿射字段移动（Move）提交掉**。
>    - 如果守卫内部创建了局部 `Scratch` 对象（带有 `Drop`），当守卫返回 `false` 准备转向下一候选分支前，会**先完成本次观察区的局部 `Drop` 清理**，并撤销未提交的候选绑定，让下一条分支依然拿到完整无损的原主语对象。
> 3. **已执行的辅助调用绝不重放，也绝不跨分支乱序合并**：如果第一候选的守卫调用了函数 `accept(first.ready)` 并返回 `false`，进入第二候选再次调用 `accept(...)` 时，那是实实在在的第二次调用；决策树合成绝不会跨越有观察效果的守卫去乱序重排后续分支。

### 带守卫与 `or` 绑定重命名的安全匹配，以及静态对齐拦截

让我们通过一段结合了 `or` 模式、同时绑定重命名与守卫函数的代码，看看匹配器是如何在保持高效分派的同时，严守所有权与求值次序契约的：

```flurry
use std.prelude.*;

struct Packet
where
    T: Type,
{
    ready: bool,
    payload: T,
}

fn accept(ready: bool) -> bool {
    ready
}

-- 在 or 模式与守卫中安全观察字段，成功后才提交唯一一次 payload 移动
fn unpack_ready_payload(
    pair: Tuple<Packet<T>, Packet<T>>,
) -> ?T
where
    T: Type,
{
    pair match {
        -- 候选 1 先观察第 0 个 Packet 的 ready；
        -- 若守卫返回 false，撤销绑定，绝不移动第 0 个 payload，
        -- 继续在候选 2 中将第 1 个 Packet 绑定为新的 first 并测试。
        ((first, _) if accept(first.ready))
            or ((_, first) if accept(first.ready)) => first.payload,
        _ => null,
    }
}

test {
    let p1 = Packet<u32> { ready: false, payload: 10u32 }
    let p2 = Packet<u32> { ready: true,  payload: 42u32 }
    let res = unpack_ready_payload((p1, p2));
    println("unpacked: {}", res is u32);
}
```

同时，如果我们在写位向量模式时，不小心把比特段的宽度算错了（比如对一个 8 位的 `BitVec<8>` 写了 7 个比特的模式），或者对一个还没取 `'bitvec` 视图的普通整数直接写 `0b...` 分段模式，编译器会在静态检查阶段编译期拒绝：

```flurry
fn broken_bitvec_pattern(byte: u8) {
    -- 负例 1：普通值匹配的被匹配主语不是 BitVec（忘记写 byte'bitvec）
    -- ❌ 编译期直接拒绝。
    -- 位向量分段模式只能直接匹配 BitVec<n>，普通整数 u8 不会自动隐式转为位视图。
    if byte matches 0b1010 (low: BitVec<4>) {
        println("{}", low.as(u8));
    }

    -- 负例 2：各段宽度之和与目标 BitVec<8> 宽度不对齐（4 + 3 == 7 != 8）
    -- ❌ 编译期直接拒绝。
    -- 整条 pattern 的总宽度为 4 + 3 = 7 位，不等于 byte'bitvec 的 8 位宽度。
    if byte'bitvec matches 0b1010 (low: BitVec<3>) {
        println("unreachable");
    }
}
```

---

## 本篇小结

| 机制 / 规则 | 匹配自动机与模式编译器的行为 | 性能与安全收益 |
| :--- | :--- | :--- |
| **公共前缀与列选择合成** | 提取多条分支共享的标签、字段或位段测试，生成紧凑决策树 | 避免朴素逐行回溯中的重复掩码与重复字段比较 |
| **位向量分段对齐检查** | 静态核验 `0b`/`0o`/`0x` 各段宽度之和等于目标 `BitVec<n>` 宽度 | 在编译期拦截任何漏写比特或跨界切位错误 |
| **延迟移动提交（Late Commit）** | 仅在整条模式与外层守卫 `if cond` 全部成功后才提交仿射移动 | 候选失败时原主语保持完整，安全交由下一候选分支继续尝试 |
| **`or` 模式同时重命名与清理** | 各候选独立检查，按实际命中候选的绑定取得逆序执行 `Drop` | 即使不同候选以不同顺序绑定变量，析构顺序也 100% 精确无误 |

---

### 下一篇预告

回顾从第 116 回到第 126 回的旅程，我们已经领略了 CMTT（上下文模态类型论）和匹配自动机的宏伟殿堂：它们有阶段检查、有 `< Ψ |- T >` 和 `< T -| Ψ >` 上下文类型、有单洞规则、有卫生绑定身份。

但是，杀鸡焉用牛刀。
设想一下：你正在写一个硬件驱动或错误码表文件，你只是想在**当前这一个源文件内部**，快速批量声明 20 个格式一模一样的常量（比如 `const FIRST = 1; const SECOND = 2; ...`），或者少敲几行纯粹重复的样板 Token。
为了省这几行字，难道你还得大费周章地去写一个 `pure comptime fn`、声明 `< Ψ |- T >` 上下文类型、再写 `~<...>` 拼接吗？

当然不用。Flurry 专门为这种“轻量级、单文件内部的表面 Token 减负”准备了一把瑞士军刀——**模板宏（Template Macro：`define id` 与 `$id`）**。

下一篇 **《Flurry 演进手记 (127)：轻量级文件内语法减负——模板宏 `define id` 与 `$id` 按组展开》**，我们将作为第六卷上半部的收官之作，解锁这把小巧锋利的语法减负神器。
