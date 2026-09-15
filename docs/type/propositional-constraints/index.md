# PTC：类型检查中的命题约束

## Intro

编译器检查一个操作时，需要知道它的前提是否成立。例如，一个类型是否具有某字段、一个下标是否在界内，或当前上下文是否提供某个 trait 实例。

PTC 把这些问题表示成命题，并组合来自不同语义领域的事实。它可以在输入未知时证明一些结论；无法完成证明时，会明确保留未知，而不是猜测真假。

## 语法

~~~text
requires_clause -> requires expr
ensures_clause -> ensures expr
refinement_type -> refines { expr } expr
bool_matches -> expr matches pattern
~~~

布尔组合与量词沿用全局语法。assumes、inline 条件与阶段检查见[normalization](../../comptime/normalization-and-instantiation.md)。本篇及后续算法的大写名称、希腊字母、箭头和数学运算均为内部记号，不是 Flurry API 或源码语法。

## 正式定义

### 操作先有前提，结果再提供事实

要求使用某个字段之前，必须有适用的结构信息和访问依据。调用函数之前，必须满足其要求。合法调用完成检查后，才能使用它提供的结果契约。

PTC 只接纳具有明确语义的事实：合法声明、已经成立的要求、已定义的结构规律、有效实现证据、合法分支条件与已兑现的结果保证。没有对应语言定义的性质，不能因求解器能够存下它就获得意义。

普通编译时 bool 可以执行，但不一定能提取成稳定的逻辑事实。无法提取的 inline 条件不会自动给分支添加新的类型性质。

### 四种查询结果

设 Γ 为已有事实，Q 为待证目标。

| 结果 | 含义 | 使用方式 |
|---|---|---|
| Entailed | Γ 蕴含 Q，有可靠证明依据 | 当前环境可使用 Q |
| NotEntailed | 有满足 Γ 且不满足 Q 的有效反例 | 此组前提不足以保证 Q |
| Unknown | 在当前能力与预算内没有完成证明或反例验证 | 按阶段规则延迟或拒绝 |
| IllFormed | 目标或其必要观察不满足定义域、前提 | 修复非法操作，不当作普通真假 |

NotEntailed 不等于证明了 Q 的否定。某个 T 在一组允许输入上可能有字段，也可能没有：一个反例只说明不能保证字段存在。

Unknown 是检查状态，不是命题的第三个真值。ordinary 的必检义务不能靠 Unknown 通过；loose 的延迟许可由阶段规则决定。assumes 在实例化阶段执行，不满足是编译错误。

### 三层承诺

PTC 的完整目录列出核心原子模式；公开谓词可以展开到这些模式；用户 predicate 通过受约束的可执行入口接入。有限目录不意味着全部用户程序可判定。

基础能力包括布尔组合、审计过的同余、已定义结构规律和整数差分逻辑。一般整数、递归 predicate、未知长度序列与跨理论组合可以保留残余。实现应公开支持片段及资源界限，使诊断可重复。

## 常见示例

以下程序在编译时确定一个下标，再把数值交给运行时输出。函数体中的调用需要证明两条前提：

~~~flurry
pure comptime fn checked_index(index: Integer, length: Integer) -> Integer
where requires 0 <= index and index < length {
    index
}

pure comptime fn next_checked_index(index: Integer, length: Integer) -> Integer
where requires 0 <= index and index + 1 < length {
    checked_index(index + 1, length)
}

test {
    const INDEX: usize = next_checked_index(2, 4).as(usize);
    println("index: {}", INDEX);
}
~~~

输出为 `index: 3`。检查 `next_checked_index` 的函数体时，PTC 已有 `0 <= index` 和 `index + 1 < length`；调用 `checked_index` 又要求 `0 <= index + 1` 和 `index + 1 < length`。前一项由整数关系得到，后一项直接来自已有前提。证明不需要先知道 index 恰好是 2。

删去调用方的上界要求会怎样？对照这个函数：

~~~flurry
pure comptime fn missing_upper_bound(index: Integer, length: Integer) -> Integer
where requires 0 <= index {
    checked_index(index + 1, length)
}
~~~

定义端不能证明内部调用对所有允许输入合法。`index = 3, length = 4` 满足已有前提，却使下一个下标等于长度，是实际反例。由于这是 comptime 函数，loose 检查可以保留义务，但不能交付违反要求的具体实例：

~~~flurry
comptime {
    assumes missing_upper_bound(2, 4) == 3;
}
~~~

这次实例化可以兑现义务；调用 `missing_upper_bound(3, 4)` 则必须失败。反例说明原前提不够，不说明这个函数的每一次具体调用都必然失败。

再对比一个无法仅靠差分逻辑完成的目标：

~~~flurry
pure comptime fn square_within(value: Integer, limit: Integer) -> bool {
    value * value < limit
}

comptime {
    assumes square_within(3, 16);
}
~~~

具体输入可以执行得到真；对符号 value 的一般非线性命题，基础求解片段没有完整承诺。求解器应继续使用可用依据或报告 Unknown，不能因为基础算法没完成就断言命题为假。

## 与 Rust / Zig / C / C++ 的关键差异

Rust trait bound、C++ 约束和 Zig 编译时断言都能约束泛型程序；C 可使用静态断言和显式检查。PTC 额外规定统一的命题查询结果、理论分工与阶段兑现，不能把某种已有编译器的接受范围当作它的契约。[Rust bound](https://doc.rust-lang.org/reference/trait-bounds.html)、[总览对照](../index.md#与-rust--zig--c--c-的关键差异)

## 运行时实现设计引导

PTC 本身不要求运行时求解器。静态证明可以消去；操作实际需要的路由、环境和实现入口不能一起消去。

对于允许具体执行验证的保证，只有验证成功的实例能交付。验证发生在其阶段契约指定的位置，不能由求解器的试探分支随意执行有副作用的源程序。

## 编译器实现引导

推荐的数据分工：

| 对象 | 内容 |
|---|---|
| 项 | sort、身份、参数、依赖、可用观察 |
| 原子 | 核心种类、操作数、良定义前提 |
| 公式 | 布尔组合及量词绑定 |
| 操作证据 | 成员来源、签名、适配、布局或路由 |
| 查询 | Γ、Q、阶段、目标配置、语义环境、预算 |
| 解释 | 支持前提、推理规则、反例或残余 |

下一步依次阅读[原子目录](atoms.md)、[各 theory](theory-solvers.md)、[组合协议](solver-protocol.md)。三者共同构成可执行的检查骨架；只实现其中一张接口表不能保证可靠性。
