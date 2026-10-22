# Flurry 演进手记 (087)：读懂类型约束与诊断信息——组合约束推理、残余条件与签名设计

> **所属卷**：第四卷：元编程即类型论——走进两级类型论 (2LTT) 与编译期宇宙 (066–095)  
> **上一篇**：[086：分支中的流敏感类型——Occurrence Typing 与局部类型命题窄化](./086-occurrence-typing-in-branches.md)  
> **下一篇**：[088：Row Polymorphism（行多态）与唯一标签路由](./088-row-polymorphism-and-scoped-labels.md)  
> **参考规范**：[`docs/type/propositional-constraints/index.md`](../docs/type/propositional-constraints/index.md), [`docs/type/propositional-constraints/solver-protocol.md`](../docs/type/propositional-constraints/solver-protocol.md)

---

## 一、 问题背景：当多维类型约束交织时，如何定位缺失的前提？

在前面的章节中，我们已经接触了命题类型约束（PTC）的类型原子命题（085）与分支出现类型（086）。在系统库（如泛型容器、编解码器或多态通信组件）中，一个泛型函数往往同时涉及多种类型命题：
- **Trait 能力约束**：例如 `T :- Copy`；
- **类型身份与分类约束**：例如 `T == u32 or T == f32`、`not Type.predicates.is_not_sized(T)`；
- **行标签与结构关系**：例如错误行包含关系或枚举变体存在性；
- **分支类型窄化**：在运行时泛型函数的 `inline if` 分支内获得了额外的局部类型命题。

如果把所有约束混在一起却缺乏清晰的边界，容易遇到两类问题：
1. **调用方难以定位原因**：如果缺乏明确的约束归因，调用方很难迅速判断具体是哪个 Trait 约束或哪条类型命题没有满足；
2. **混淆“类型级命题”与“运行期程序状态”**：试图在 PTC 中追踪算术大小或运行期指针状态，或者在 `ordinary` 泛型函数体内漏写了必要的 `where` 类型前提。

本篇我们将从工程使用者的视角出发，梳理 Flurry 的类型约束组合推理规则、如何阅读未满足约束的诊断反例，以及如何编写边界清晰的 `where` 约束签名。

---

## 二、 核心职责边界：类型命题（PTC）vs 程序状态验证

在设计复杂函数签名时，首先要明确 Flurry 中两套验证机制的分工：

```text
+-------------------------------------------------------------------------+
|               Flurry 静态契约的双轨职责边界                              |
+-------------------------------------------------------------------------+
|                                                                         |
|  1. 命题类型约束 (PTC —— Propositional Type Constraints)                |
|     • 关注对象：只处理类型命题——类型相等/子类型、Trait 满足性、          |
|                 固定大小性质、枚举行/错误行包含关系、分支类型窄化        |
|     • 作用时机：泛型定义检查 (ordinary normalization) 与调用点类型检查   |
|                                                                         |
|  2. 程序验证层 (Program Verification —— verified fn / outcomes)         |
|     • 关注对象：程序状态、数值与下标算术边界、指针存活与内存读写分离     |
|     • 作用时机：函数前置/后置状态证明与内存效果审计                      |
+-------------------------------------------------------------------------+
```

简而言之：**与“类型身份、结构分类、固定大小表示、Trait 实现、行关系”相关的命题属于类型约束（PTC）；与“数值算术边界、运行期内存位置、指针解引用、可变状态转移”相关的规格属于程序验证层。**

---

## 三、 实战演练：多维类型约束的组合推理

让我们看一个结合了**有限域类型析取（`T == u32 or T == f32`）**、**固定大小表示检查**与**Trait 约束**的示例（参考 `docs/type/propositional-constraints/solver-protocol.md`）：

```flurry
use std.prelude.*;

enum StorageKind {
    integer,
    real,
}

-- 类型工厂：保证返回的 Type 必为 u32 或 f32 之一
pure comptime fn StorageType(kind: StorageKind) -> result: Type
where
    ensures result == u32 or result == f32
{
    kind match {
        .integer => u32,
        .real => f32,
    }
}

-- 泛型运行时函数（采用 ordinary 检查）：
-- 要求 T 是 u32 或 f32，且具备固定大小表示
fn duplicate_scalar_storage(value: T) -> Tuple<T, usize>
where
    T: Copy,
    requires T == u32 or T == f32,
    requires not Type.predicates.is_not_sized(T),
{
    -- 因为 T :- Copy，value 可复制两份；
    -- 因为 not is_not_sized(T)，可查询 Type.size_of(T)
    let cloned = value;
    (cloned, Type.size_of(T))
}

test valid_scalar_storage_constraints {
    let (v1, sz1) = duplicate_scalar_storage(42u32);
    let (v2, sz2) = duplicate_scalar_storage(3.5f32);

    assumes v1 == 42u32 and sz1 == 4usize;
    assumes sz2 == 4usize;
}
```

在这段代码中，类型检查器同时完成了三项类型命题推理：
1. **Trait 证据验证**：确认 `u32` 与 `f32` 均实现了 `Copy`，允许复制 `value`；
2. **表示定长性验证**：确认 `not Type.predicates.is_not_sized(T)` 成立，满足 `Type.size_of(T)` 的观察前提；
3. **有限域类型析取验证**：在调用点代入 `T = u32` 或 `T = f32` 时，验证 `T == u32 or T == f32` 成立。

---

## 四、 读懂三类典型诊断：定义端前提不足、结果保证违约与分支窄化

当类型约束未能通过检查时，Flurry 会将问题明确区分为以下情形，并给出具体缺失的条件或有效反例。

### 4.1 函数定义处漏写类型前提（定义端 `ordinary` 检查失败）

如果在编写 `ordinary` 泛型函数体时，调用了要求 `T == u32 or T == f32` 的函数，却忘了在当前函数的 `where` 子句中声明相应前提，编译器会在**函数定义处**指出缺失的类型条件及合法反例（如 `T = bool`）：

```flurry
pure comptime fn supported_size(T: Type) -> usize
where
    requires T == u32 or T == f32
{
    Type.size_of(T)
}

-- ❌ 编译期（定义端 ordinary 检查）直接报错：
-- 函数体调用 supported_size(T) 要求 `T == u32 or T == f32`，
-- 但当前签名仅声明了 `where T`，存在有效反例 `T = bool`
fn missing_type_constraint(value: T) -> usize where T {
    supported_size(T)
}
```

### 4.2 类型工厂的 `ensures` 结果保证未能成立

如果类型工厂声明了 `ensures result == u32 or result == f32`，但某个分支实际返回了 `bool`，编译器免将作者写的 `ensures` 当作免检公理，而会报错该违约分支（禁止任何依赖该未兑现保证的实例交付）：

```flurry
-- ❌ 编译期报错：.real 分支返回了 bool，违反了 ensures result == u32 or result == f32
pure comptime fn BrokenStorageType(kind: StorageKind) -> result: Type
where
    ensures result == u32 or result == f32
{
    kind match {
        .integer => u32,
        .real => bool,
    }
}
```

### 4.3 在运行时泛型函数中用 `inline if` 就地补充类型前提

如果某个运行时泛型函数希望接收任意类型 `T`，仅当 `T` 满足 `T == u32 or T == f32` 时才调用 `supported_size(T)`，其余类型一律返回 `0usize`，则无需把 `requires` 推给调用方，只需在函数体内使用 `inline if` 进行分支类型窄化：

```flurry
fn size_if_supported(value: T) -> usize where T {
    inline if T == u32 or T == f32 {
        -- 在此分支内，`T == u32 or T == f32` 自动作为局部类型命题生效，
        -- 因此 supported_size(T) 在定义期 ordinary 检查中直接通过
        supported_size(T)
    } else {
        0usize
    }
}

test branch_narrowed_type_constraint {
    assumes size_if_supported(10u32) == 4usize;
    assumes size_if_supported(1.0f32) == 4usize;
    assumes size_if_supported(true) == 0usize;
}
```

---

## 五、 本篇小结

| 场景 | 编译器检查重点 | 工程实践要点 |
| :--- | :--- | :--- |
| **定义 `ordinary` 泛型函数** | 验证函数体所需的一切类型相等、Trait 与表示性质是否均可由 `where` 前提导出 | 将跨参数的类型关系（如 `T :- Copy`、`T == u32 or T == f32`）显式声明在 `where` 中 |
| **分支内局部类型特化** | 在运行时泛型函数中收集 `inline if` 或类型模式分支条件，窄化当前分支的已知类型命题 | 对特定类型先用 `inline if` 分流，再在对应分支内调用受限接口 |
| **核验 `ensures` 类型保证** | 检查类型函数的返回路径是否真实兑现其声明的 `ensures` 命题 | 不把 `ensures` 当成未验证公理；违约定义或实例均会被拦截 |

---

## 六、 下一篇预告

在前面的错误类型与效应类型中，我们多次见到“往一个枚举或集合里追加新变体”或“在一个函数里只处理一部分分支，把剩下的分支维持原貌转发出去”的需求。

在类型系统中，这种对变体标签或字段集合进行灵活扩展与裁剪的能力被称为**行多态（Row Polymorphism）**。下一篇 **《088：行多态（Row Polymorphism）与唯一标签路由》**，我们将探讨行多态在可扩展枚举、错误集与效应行中的设计与应用。
