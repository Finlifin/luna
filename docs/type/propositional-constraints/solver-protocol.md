# PTC 组合、证明与诊断

## Intro

一个约束可能同时涉及类型身份、整数参数和实现选择。组合协议规定各求解器怎样共享事实，以及什么时候可以给出最终结论。

最重要的边界是：没有发现矛盾，不等于已经构造出符合源程序语义的反例。

## 语法

本篇没有新增源码语法。Γ、Q、SAT、UNSAT、activation guard 和下面伪代码均为内部算法记号。

## 正式定义

查询 Γ 是否蕴含 Q，可检查 Γ 与非 Q 是否不可满足。可靠的不可满足证明足以支持目标；可满足的抽象赋值仍需验证能否代表源语义中的合法输入。

结果及阶段处理沿用[PTC 总览](index.md)。局部结论只在其支持前提和依赖有效时成立。分支、实例和实现环境变化后，不能无条件复用旧结论。

用户 ensures 可以支撑 normalization，但其合法兑现必须在依赖实例交付前完成。定义证明或获准具体验证均失败时，依赖该保证的实例不能交付。

## 常见示例

用一个只有两种选择的类型工厂，展示有限域结果怎样进入后续要求：

~~~flurry
enum StorageKind {
    integer,
    real,
}

pure comptime fn storage_type(kind: StorageKind) -> result: Type
where ensures result == u32 or result == f32 {
    kind match {
        .integer => u32,
        .real => f32,
    }
}

pure comptime fn supported_size(T: Type) -> usize
where requires T == u32 or T == f32 {
    Type.size_of(T)
}

pure comptime fn selected_size(kind: StorageKind) -> usize {
    let Selected = storage_type(kind);
    supported_size(Selected)
}

test {
    const INTEGER_SIZE = selected_size(.integer);
    const REAL_SIZE = selected_size(.real);
    println("integer size: {}", INTEGER_SIZE);
    println("real size: {}", REAL_SIZE);
}
~~~

输出两个具体类型在目标上的大小。检查 `selected_size` 的内部调用时，工厂保证提供了 `Selected == u32 or Selected == f32`；`supported_size` 据此接纳该类型。保证本身要由工厂的两个分支兑现。

若查询“Selected 能否既不是 u32，也不是 f32”，组合层必须同时保留有限域析取与两个不等式：

~~~text
已有事实：Selected = u32 OR Selected = f32
候选反例：Selected != u32 AND Selected != f32
联合结果：冲突
~~~

然而，仅把几个类型项登记到等式表而丢掉析取，会让各模块暂时看不到冲突。这不能成为源程序反例，因为工厂没有产生第三种类型的路径。

结果保证也不是作者可以随意填写的公理。下面是有意错误的工厂：

~~~flurry
pure comptime fn broken_storage_type(kind: StorageKind) -> result: Type
where ensures result == u32 or result == f32 {
    kind match {
        .integer => u32,
        .real => bool,
    }
}
~~~

检查 `real` 路径可以发现保证不成立。若按阶段规则延迟具体验证，调用 `broken_storage_type(.real)` 时仍必须失败；任何依赖这份保证的实例都不能交付。

与这个可复现的失败不同，某个未解释调用的抽象模型给出的返回值，未必有合法源执行可以产生。组合层应验证反例的来源；无法完成验证时保留 Unknown，不能把占位值写进用户诊断冒充实参。

## 与 Rust / Zig / C / C++ 的关键差异

这里规定的是 Flurry 编译器的证明协议，没有一条对应的 C、C++、Rust 或 Zig 源码构造。实际对照是这些语言的约束检查、常量求值与诊断机制；不得把某个后端的 SAT 状态直接当作语言层接受规则。

## 运行时实现设计引导

若证明产物支持删除运行时检查，编译器必须确认该删除原本就属于操作契约允许的优化。证明依赖的目标、对象环境与调用来源不能在代码生成前失效。

编译时验证失败不应留下可链接的半成品实例。实例缓存需要区分检查中的条目和已交付条目，避免递归引用误用尚未兑现的保证。

## 编译器实现引导

### 布尔协调层

采用 CDCL(T)/DPLL(T) 分工：布尔层负责决策、传播与学习，theory 接收文字并解释冲突。CDCL(T) 源自自动定理证明领域，其核心优势在于将命题搜索与领域推理解耦，使编译器可以按需接入不同 theory 模块而不改变搜索框架。该分工不要求把整门语言编码到某个现成 SMT 产品。[DPLL(T) 原始论文](https://homepage.cs.uiowa.edu/~tinelli/papers/NieOT-JACM-06.pdf)

~~~text
prove(context, goal):
    check_well_formed(context, goal)
    scoped_formula := context AND NOT goal
    initialize_boolean_abstraction(scoped_formula)

    loop:
        propagate_boolean_literals()
        distribute_new_literals_to_theories()
        collect_explained_propagations_and_conflicts()

        if conflict:
            learn_guarded_clause()
            if conflict_at_query_base:
                return Entailed(proof)
            backtrack_all_modules()
            continue

        if budget_exhausted:
            return Unknown(residual, dependency_summary)

        if assignment_complete:
            joint := validate_joint_model()
            if joint.is_source_witness:
                return NotEntailed(joint.witness)
            if joint.has_explained_conflict:
                add_lemma(joint.explanation)
                continue
            return Unknown(joint.missing_interpretations)

        make_boolean_decision()
~~~

良定义检查自身可能产生义务；不适用操作报告 IllFormed，输入未知时按其合法延期方式保留，不能构造非法项混入模型。

### 每个 theory 的接口

| 接口职责 | 不变量 |
|---|---|
| 登记项与文字 | sort、前提、语义身份明确 |
| 接受赋值 | 正负文字含义对称，不能把负文字当查询失败 |
| 传播 | 每个结论附必要前提 |
| 冲突解释 | 前提在当前赋值下足以构成矛盾 |
| 决策层回退 | 项状态、队列、证明和缓存一致恢复 |
| 模型片段 | 标出完整解释与抽象占位 |
| 残余报告 | 原操作、缺失前提、受限片段、预算消耗 |

学习子句需要保留局部假设的 activation guard。退出分支可以停用 guard，而不必删除全部学习内容；省略 guard 会让局部结论泄漏到其他分支。

### 跨理论通信

共享等式仅在相同语义域内交换。整数与机器位向量、类型身份与头形状、错误行包含与值转换之间需要显式桥接契约。

对于有限域，需要把必要的穷尽析取交给布尔层。BV 不满足经典无限域组合假设；开放行、类型构造与序列也不能仅凭接口相似宣称符合某个组合完备性定理。

联合验证至少检查：

1. 每个共享项在所有模块解释一致。
2. 部分函数的前提成立。
3. 有限域元素满足其合法性约束。
4. 未解释调用能否由合法源计算实现。
5. 量词与见证覆盖所声称的范围。
6. 对象 identity、实现环境和阶段输入一致。

无法验证其中必要项时返回 Unknown。抽象 UNSAT 可以可靠证明，但前提是抽象规则全部可靠；抽象 SAT 仅是候选模型。

### 契约兑现与缓存

缓存键纳入目标、必要前提、实际绑定、target、阶段、实现环境及其他语义依赖。缓存内容包含依赖证明，不只存 true/false。

Unknown 只能表示本配置未完成，不是永久否定。已证明结果可以在前提仍有效时复用；实现环境变化必须重新验证相关依赖。

调用前先证明 requires；调用后才能使用合法 ensures。assumes 按规定阶段执行。具体结果检查失败时，所有依赖该未兑现保证的实例都不得交付。

### 诊断与验收

诊断按原因分类：非法观察、反例、缺少前提、未支持片段、预算耗尽、非法布局环、具体契约失败。递归求值不终止与递归 predicate 未证目标不是同一种错误。

| 验收案例 | 必须保持的结果 |
|---|---|
| a=b 且 f(a)≠f(b)，同余合法 | 冲突有等式链解释 |
| A≈Any 且 Any≈B | 不合并 A/B |
| 2x=1 | 不交付有理数作为整数模型 |
| 未知 pointee 布局 | 指针自身宽度仍可已知 |
| 两个开放行标签 | 不用当前可见集合证明完整包含 |
| 已知掩码兼容、guard 未决 | 不宣称完整相交 |
| 全称只试少量实例 | 不宣称全域成立 |
| 分支局部结论跨分支复用 | 缺少 guard 时拒绝复用 |
| ensures 具体验证失败 | 依赖实例不可交付 |

资源预算采用可重复计数，如项数、实例数、决策数与递归深度。时间限制可补充，但不能成为没有解释的随机接受策略。
