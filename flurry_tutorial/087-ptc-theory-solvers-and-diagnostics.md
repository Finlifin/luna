# Flurry 演进手记 (087)：读懂类型约束与诊断信息——组合约束推理、残余条件与签名设计

> **所属卷**：第四卷：元编程即类型论——走进两级类型论 (2LTT) 与编译期宇宙 (066–095)  
> **上一篇**：[086：走进分支后类型自动变精确了——Occurrence Typing（出现类型推导）的奥秘](./086-occurrence-typing-in-branches.md)  
> **下一篇**：[088：把枚举行与记录行玩出花——Row Polymorphism（行多态）与唯一标签路由](./088-row-polymorphism-and-scoped-labels.md)  
> **参考规范**：[`docs/type/propositional-constraints/index.md`](../docs/type/propositional-constraints/index.md), [`docs/type/propositional-constraints/atoms.md`](../docs/type/propositional-constraints/atoms.md)

---

## 一、 问题背景：当多维类型约束交织时，如何定位缺失的前提？

在前面的章节中，我们已经接触了命题类型约束（PTC）的原子命题（085）与分支出现类型（086）。在系统库（如静态形状张量库、定宽位级编解码器或多态容器）中，一个函数往往同时涉及多种类型的约束：
- **Trait 能力约束**：例如 `T :- Copy`；
- **位宽与算术约束**：例如 `W >= 8` 且 `W <= 64`；
- **行标签与结构关系**：例如枚举行包含或排除某个变体标签；
- **分支条件窄化**：在 `inline if` 或 `if ... is ... do` 分支内获得了额外的局部类型事实。

如果把所有约束堆在一起却缺乏清晰的边界，容易遇到两类问题：
1. **调用方难以定位原因**：如果缺乏明确的约束归因，调用方很难迅速判断具体是哪个位宽条件或哪个关联类型约束没有满足；
2. **混淆“类型级命题”与“运行时内存状态”**：试图在类型约束里直接写运行期指针解引用状态，或者在泛型函数体内漏写了必要的 `where requires` 类型前提。

本篇我们将从工程使用者的视角出发，梳理 Flurry 的类型约束推理规则、如何阅读未满足约束的诊断反例，以及如何编写边界清晰的 `where` 约束签名。

---

## 二、 核心职责边界：类型命题（PTC）vs 程序状态验证

在设计复杂函数签名时，首先要明确 Flurry 中两套验证机制的分工：

```
+-------------------------------------------------------------------------+
|               Flurry 静态契约的双轨职责边界                              |
+-------------------------------------------------------------------------+
|                                                                         |
|  1. 命题类型约束 (PTC —— Propositional Type Constraints)                |
|     • 关注对象：类型相等/子类型、Trait 满足性、位宽算术关系、            |
|                 枚举行/记录行包含关系、分支类型窄化                      |
|     • 作用时机：泛型定义检查与调用点类型检查                             |
|                                                                         |
|  2. 程序验证层 (Program Verification —— verified fn / outcomes)         |
|     • 关注对象：运行期数组下标边界、指针存活与世代、内存读写分离 (disjoint)|
|     • 作用时机：函数前置/后置条件证明与内存效果审计                      |
+-------------------------------------------------------------------------+
```

简而言之：**与“类型身份、编译期位宽/维度、Trait 实现”相关的条件属于类型约束（PTC）；与“运行期内存位置、指针解引用、可变状态转移”相关的条件属于程序验证契约。**

---

## 三、 实战演练：多维类型约束的协同推理

让我们看一个结合了**位宽算术约束**与**类型相等构造**的协议字打包函数。假设我们要将两个位宽分别为 `w1` 和 `w2` 的位向量拼接为一个总宽度为 `total_w` 的位向量，并要求总宽度不超过 32 位且等于两部分之和：

```flurry
pure fn concat_protocol_fields(
    high: BitVec<w1>,
    low: BitVec<w2>,
) -> BitVec<total_w>
where
    w1: usize,
    w2: usize,
    total_w: usize,
    requires w1 > 0 and w2 > 0,
    requires total_w == w1 + w2 and total_w <= 32,
{
    (high ++ low).as(BitVec<total_w>)
}

test valid_protocol_field_concat {
    let hdr: BitVec<4> = 0xA;
    let payload: BitVec<12> = 0x123;

    -- 调用点自动推导 w1 = 4, w2 = 12，当期望返回 BitVec<16> 时：
    -- 1. 4 > 0 and 12 > 0 成立
    -- 2. 16 == 4 + 12 and 16 <= 32 成立
    let frame: BitVec<16> = concat_protocol_fields(hdr, payload);
    let raw_word: u16 = frame.as(u16);
    assumes raw_word == 0xA123u16;
}
```

在这段代码中，类型检查器同时完成了三项推理：
1. **结构类型合一**：从实参 `hdr` 和 `payload` 推导出 `w1 = 4`、`w2 = 12`，从返回位置类型注解推导出 `total_w = 16`；
2. **线性算术验证**：验证 `16 == 4 + 12` 与 `16 <= 32` 恒成立；
3. **函数体内部一致性**：在 `concat_protocol_fields` 函数体内部，利用前提 `total_w == w1 + w2` 证明 `high ++ low`（类型为 `BitVec<w1 + w2>`）与目标类型 `BitVec<total_w>` 位宽相等。

---

## 四、 读懂两类典型诊断：残余约束未满足 vs 调用点前提不满足

当类型约束未能通过检查时，Flurry 会将问题明确区分为以下情形，并给出具体缺失的条件或冲突反例。

### 4.1 函数定义处漏写前提（定义端残余约束未满足）

如果在编写泛型函数体时，使用了某个需要特定位宽关系的转换，却忘了在函数的 `where requires` 子句中声明该前提，编译器会在**函数定义处**指出缺失的条件及反例（例如当 `w1 = 4, w2 = 4, total_w = 16` 时 `w1 + w2 != total_w`）：

```flurry
-- [编译期拒绝示例 1] 函数体依赖 total_w == w1 + w2，但签名漏写了该约束
-- pure fn missing_width_constraint(
--     high: BitVec<w1>,
--     low: BitVec<w2>,
-- ) -> BitVec<total_w>
-- where
--     w1: usize,
--     w2: usize,
--     total_w: usize,
-- {
--     -- 编译错误：无法由当前 where 前提证明 `w1 + w2 == total_w`
--     -- 反例：w1 = 1, w2 = 1, total_w = 8
--     (high ++ low).as(BitVec<total_w>)
-- }
```

### 4.2 调用点实参不满足 `where requires` 契约

如果函数定义合法，但调用方传入的位宽违反了 `total_w <= 32` 或 `total_w == w1 + w2`，报错会定位到**调用点**，并指明是哪一条子句未满足：

```flurry
-- [编译期拒绝示例 2] 调用点违反 total_w <= 32 约束
-- fn reject_oversized_concat(a: BitVec<20>, b: BitVec<20>) -> BitVec<40> {
--     -- 编译错误：调用 concat_protocol_fields 时未满足约束 `total_w <= 32`（实际 total_w = 40）
--     concat_protocol_fields(a, b)
-- }
```

### 4.3 利用分支条件自动消除残余约束

如果在泛型或编译期函数内部，某个条件只在一部分情况下成立，我们不需要把所有负担都推给调用方的 `where requires`，而是可以通过 `inline if` 在分支内就地补充条件：

```flurry
pure comptime fn SafeBitChunk(comptime width: usize) -> Type {
    inline if width > 0 and width <= 64 {
        -- 在此分支内，width > 0 and width <= 64 自动作为已知事实生效
        BitVec<width>
    } else {
        -- 超出范围时回退为单位类型 void
        void
    }
}

test branch_refined_type_construction {
    assumes SafeBitChunk(16) == BitVec<16>;
    assumes SafeBitChunk(0) == void;
    assumes SafeBitChunk(128) == void;
}
```

---

## 五、 本篇小结

| 场景 | 编译器检查重点 | 工程实践要点 |
| :--- | :--- | :--- |
| **定义泛型/类型函数** | 验证函数体所需的一切类型相等、Trait 与位宽关系是否均可由 `where` 前提导出 | 将跨参数的不变量（如 `TotalW == W1 + W2`）显式声明在 `where requires` 中 |
| **分支内局部特化** | 自动收集 `inline if` 或模式匹配分支条件，窄化当前分支的已知类型事实 | 对边界情况（如 `width == 0`）先用分支分流，再在合法分支内构造受限类型 |
| **调用带约束接口** | 将具体类型实参代入 `where` 子句，逐条核验约束满足性 | 根据诊断指出的具体未满足子句修正实参类型或补充前置条件 |

---

## 六、 下一篇预告

在前面的错误类型与效应类型中，我们多次见到“往一个枚举或集合里追加新变体”或“在一个函数里只处理一部分分支，把剩下的分支原封不动转发出去”的需求。

在类型系统中，这种对变体标签或字段集合进行灵活扩展与裁剪的能力被称为**行多态（Row Polymorphism）**。下一篇 **《088：行多态（Row Polymorphism）与唯一标签路由》**，我们将探讨行多态在可扩展枚举、错误集与效应行中的设计与应用。
