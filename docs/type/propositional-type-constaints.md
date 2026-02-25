### **形式化定义：命题化类型约束**

#### 1. 语法 (Syntax)

我们首先定义构成命题化类型约束系统的各个语法类别。

**1.1 基础类别**

*   **类型 (Types)**
    $$
        T, U, V ::= \text{Int} \mid \text{Bool} \mid \dots \quad \text{(基础类型)} \\
        \qquad\qquad | \quad \alpha, \beta, \gamma \quad \text{(类型变量)} \\
        \qquad\qquad | \quad C(T_1, \dots, T_n) \quad \text{(类型构造器应用)} \\
        \qquad\qquad | \quad \dots
    $$

*   **项 (Terms)**
    $$
        t, e, v ::= x, y, z \quad \text{(项变量)} \\
        \qquad\qquad | \quad c \quad \text{(常量, e.g., } 1, \text{true)} \\
        \qquad\qquad | \quad f(t_1, \dots, t_n) \quad \text{(函数应用)} \\
        \qquad\qquad | \quad \dots
    $$

*   **Trait (特征)**
    $$
        A, B, C ::= \text{Display} \mid \text{Add} \mid \text{Eq} \mid \dots \quad \text{(特征标识符)}
    $$

**1.2 命题 (Propositions)**

命题 `p, q` 是我们逻辑系统中的核心断言。

*   **原子命题 (Atomic Propositions)**
    $$
    \begin{aligned}
        p_{atomic} ::= \quad & t : T \quad & \text{(类型断言: 项 } t \text{ 拥有类型 } T \text{)} \\
        | \quad & T \texttt{:-} A \quad & \text{(类型特征断言: 类型 } T \text{ 实现了特征 } A \text{)} \\
        | \quad & t \texttt{:-} A \quad & \text{(项特征断言: 项 } t \text{ 的类型实现了特征 } A \text{)} \\
        | \quad & T_1 = T_2 \quad & \text{(类型相等: 类型 } T_1 \text{ 与 } T_2 \text{ 相等)} \\
        | \quad & t_1 = t_2 \quad & \text{(项相等: 项 } t_1 \text{ 与 } t_2 \text{ 相等)} \\
        | \quad & \top \quad & \text{(真, True)} \\
        | \quad & \bot \quad & \text{(假, False)}
    \end{aligned}
    $$

*   **复合命题 (Compound Propositions)**
    $$
    \begin{aligned}
        p, q ::= \quad & p_{atomic} \quad & \text{(原子命题)} \\
        | \quad & \neg p \quad & \text{(否定: 非 } p \text{)} \\
        | \quad & p \wedge q \quad & \text{(合取: } p \text{ 与 } q \text{)} \\
        | \quad & p \vee q \quad & \text{(析取: } p \text{ 或 } q \text{)} \\
        | \quad & p \Rightarrow q \quad & \text{(蕴含: } p \text{ 蕴含 } q \text{)}
    \end{aligned}
    $$

**1.3 上下文与判断 (Contexts and Judgements)**

*   **上下文 (Context)**: 上下文 $\Gamma$ 是一个由类型断言和命题断言构成的集合，用于在类型检查和推理中提供已知的事实。
    $$
        \Gamma ::= \emptyset \quad \text{(空上下文)} \quad | \quad \Gamma, x:T \quad | \quad \Gamma, p
    $$

*   **判断 (Judgement)**: 类型系统的核心判断形式为 $\Gamma \vdash p$，读作“在上下文 $\Gamma$ 中，命题 $p$ 为真”。

#### 2. 语义 (Semantics)

我们非形式化地描述这些构造的含义。

*   **$t: T$ (类型断言)**: 这个命题为真，如果根据语言的类型规则，可以推导出项 `t` 的类型是 `T`。这是传统类型检查的核心。

*   **$T \texttt{:-} A$ (类型特征断言)**: 这个命题为真，如果存在一个该语言中合法的 `impl A for T` 声明。这是一个关于**全局实现**的断言。

*   **$t \texttt{:-} A$ (项特征断言)**: 这是 $T \texttt{:-} A$ 的一种语法糖。它为真，如果存在类型 `T` 使得 $\Gamma \vdash t:T$ 并且 $\Gamma \vdash T \texttt{:-} A$。它断言一个**具体的值**满足某个特征，通常用于需要动态分派或在运行时检查的场景，但在静态分析中，它等价于对其类型的断言。

*   **$T_1 = T_2$ (类型相等)**: 这个命题为真，如果类型 `T₁` 和 `T₂` 在当前类型系统的规则下是等价的（例如，通过别名、类型计算或合一）。

*   **$t_1 = t_2$ (项相等)**: 这个命题为真，如果项 `t₁` 和 `t₂` 在**编译期**可以被证明指向同一个值。这通常只对编译期常量或可被符号执行的纯函数有意义。

*   **逻辑连接词 ($\neg, \wedge, \vee, \Rightarrow$)**: 它们具有标准的一阶逻辑语义。
    *   $\Gamma \vdash \neg p$ 为真，当且仅当 $\Gamma \vdash p$ 不为真。
    *   $\Gamma \vdash p \wedge q$ 为真，当且仅当 $\Gamma \vdash p$ 和 $\Gamma \vdash q$ 都为真。
    *   $\Gamma \vdash p \vee q$ 为真，当且仅当 $\Gamma \vdash p$ 或 $\Gamma \vdash q$ 至少一个为真。
    *   $\Gamma \vdash p \Rightarrow q$ 为真，当且仅当“若 $\Gamma \vdash p$ 为真，则 $\Gamma \vdash q$ 也为真”。

#### 3. 在编程语言构造中的应用

命题化类型约束通过 `where` 子句、`requires` 和 `ensures` 等语言构造，被整合到编程语言中。

*   **泛型函数声明**:
    $$
        \text{fn } f\langle \alpha_1, \dots \rangle(x_1: T_1, \dots) \rightarrow U \text{ where } p
    $$
    这里的 `p` 是一个关于类型变量 $\alpha_i$ 和参数 $x_i$ 的命题。它构成了该函数签名的**契约（Contract）**。

*   **函数体类型检查**:
    对于上述函数 `f`，其函数体的类型检查判断为：
    $$
        \Gamma, x_1:T_1, \dots, p \vdash \text{body} : U' \text{ 并且 } \Gamma, x_1:T_1, \dots, p \vdash U' = U
    $$
    即，在将函数签名中的**命题 `p` 作为假设加入上下文**后，函数体必须是类型正确的。

*   **函数调用检查**:
    对于调用 `f<V₁,...>(e₁,...)$`，编译器需要证明：
    $$
        \Gamma \vdash p[\alpha_1 \mapsto V_1, \dots, x_1 \mapsto e_1, \dots]
    $$
    即，在当前的调用点上下文 $\Gamma$ 中，将泛型参数和实际参数代入到函数签名的命题 `p` 后，得到的**新命题必须为真**。

---

这份形式化定义为你毕业论文的理论章节提供了一个坚实的起点。它清晰地将你的核心思想——用一阶逻辑来丰富类型系统——用学术界通用的语言和符号表达了出来。在此基础上，你可以进一步定义具体的类型规则、推理规则，并展开对发生类型等更高级特性的讨论。