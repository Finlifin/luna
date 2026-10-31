# Flurry 演进手记 (086)：分支中的流敏感类型——Occurrence Typing 与局部类型命题窄化

> **阅读指南**：在上一篇（085）中，我们梳理了 Flurry 命题类型约束（PTC）的原子命题体系。在编写泛型运行时函数（`fn`）时，它的定义体处于 **ordinary（严格）** normalization 模式——即使具体类型参数 `T` 尚未代入，函数体内的每一个分支也必须在定义期通过类型检查。当我们在泛型运行时函数中使用 `inline if T == u32 { ... }` 或 `inline if T is do { Array<'u32, length> => ... }` 根据编译期类型条件分流时，为什么进入特定分支后，原本类型为未知 `T` 的运行时值 `value` 就能直接传给只接受 `u32` 或特定容器类型的函数？而在离开分支后，这些局部类型事实又为什么免于泄漏到外部？本篇我们将介绍 **出现类型（Occurrence Typing / 流敏感类型窄化）** 在 ordinary normalization 中的作用与作用域规则。

---

## 一、 前提辨析：`ordinary` vs `loose` Normalization，以及 `inline` 控制流的定位

在讨论分支类型窄化之前，必须先厘清两条核心边界：

1. **为什么 `inline if` 用于运行时函数（`fn`），而非 `comptime fn`？**
   - **`comptime fn`（编译期函数）** 整段代码都在编译期执行，其中的普通 `if`、`match`、`for` 本身就是编译期控制流，无需、也不应该加 `inline`。同时，`comptime fn` 采用 **loose** normalization：依赖未知编译期参数的约束检查可以留到调用端（Instantiation）代入具体实参时再兑现。
   - **普通运行时函数 `fn`** 采用 **ordinary** normalization：定义端必须立刻完成所有分支的操作合法性检查，而无法等待未来某次实例化选中某个分支才去检查。在运行时函数内部，如果希望根据编译期类型或常量条件在编译期裁剪分支，才使用 **`inline if` / `inline when` / `inline for`**。

2. **PTC 只追踪类型命题，不追踪算术值问题**
   - PTC（命题类型约束）的职责是处理**类型命题**（例如类型相等 `T == u32`、类型模式匹配 `T is Array<'u32, length>`、结构分类 `Type.predicates.is_tuple(T)`、Trait 满足性等）。
   - 普通数值上的算术大小比较非属 PTC 的类型命题追踪范畴；程序运行时的数值与内存状态规格由程序验证层负责。

```flurry
-- 对比 1：comptime fn 本身就在编译期执行（采用 loose 检查），直接使用普通 match / if
pure comptime fn word_array_length(T: Type) -> ?usize {
    T match {
        Array<'u32, length> => length,
        _ => null,
    }
}

-- 对比 2：运行时泛型函数 fn（采用 ordinary 检查），使用 inline if 在编译期按类型分流
fn accept_u32(x: u32) -> u32 {
    x + 1u32
}

fn dispatch_runtime_value(value: T) -> u32 where T {
    -- 在 ordinary normalization 中，两个分支都必须在定义期通过类型检查：
    -- 进入 then 分支时，PTC 将类型命题 `T == u32` 记入当前分支环境，
    -- 因此将 `value: T` 传给 `accept_u32(x: u32)` 在定义期即可证明合法。
    inline if T == u32 {
        accept_u32(value)
    } else {
        0u32
    }
}
```

如果 `ordinary` 检查是“流脱敏（Flow-Insensitive）”的——即忽略 `inline if T == u32` 这一分支条件，始终只把 `value` 当作未知类型 `T`——那么定义端检查 `accept_u32(value)` 时就会因为无法证明 `T == u32` 而报错。

---

## 二、 核心机制：将 `inline` 分支条件转化为局部类型命题

Flurry 在 `ordinary` normalization 中支持 **Occurrence Typing（出现类型 / 流敏感类型窄化）**。当运行时函数通过 `inline if` 或 `inline if ... is ... do` 对类型条件进行分支判断时，编译器会从分支条件中提取可识别的**类型命题**，将其作为局部事实加入对应分支的约束环境 $\Gamma$ 中。

下面看一个同时包含**类型相等窄化**与**类型模式结构窄化**的运行时泛型函数示例：

```flurry
use std.prelude.*;

fn sum_u32_pair(pair: Tuple<u32, u32>) -> u32 {
    let (left, right) = pair;
    left + right
}

fn first_u32_element(arr: Array<u32, n>) -> u32
where
    n: usize,
{
    arr[0usize]
}

-- 运行时泛型函数：在 ordinary normalization 下检查全部分支
fn inspect_runtime_container(value: T) -> usize where T {
    inline if T is do {
        -- 分支 1：匹配 Array<'u32, length>
        -- 局部环境获得类型命题 `T == Array<u32, length>` 并引入编译期绑定 `length: usize`
        Array<'u32, length> => length,

        -- 分支 2：匹配 Tuple<'u32, 'u32>
        -- 局部环境获得类型命题 `T == Tuple<u32, u32>`，
        -- 因此类型为 T 的运行时参数 `value` 可直接传给 `sum_u32_pair`
        Tuple<'u32, 'u32> => sum_u32_pair(value).as(usize),

        -- 兜底分支：未享有上述类型相等命题
        _ => 0usize,
    }
}

test {
    let arr: Array<u32, 4> = [10u32, 20u32, 30u32, 40u32];
    let pair: Tuple<u32, u32> = (15u32, 27u32);

    assumes inspect_runtime_container(arr) == 4usize;
    assumes inspect_runtime_container(pair) == 42usize;
    assumes inspect_runtime_container(true) == 0usize;
}
```

> **复习：类型模式里的单引号 `'u32`**
> 在 `Array<'u32, length>` 与 `Tuple<'u32, 'u32>` 中：
> - 无前缀的标识符 `length` 是**新引入的模式绑定变量**，捕获数组的第二个类型参数（长度）；
> - 带单引号前缀的 `'u32` 是**引用已有类型的模式表达式**，要求对应位置的类型参数等于 `u32`。

---

## 三、 分支局部类型事实的生命周期与汇合（Join）规则

分支内的局部类型命题如何在进入分支时生效、在离开分支时回收？其作用域与汇合规则如下：

```text
                   ┌─────────────────────────────────────────┐
                   │ 1. 分支前环境 Γ_parent                  │
                   │    已知声明: T : Type, value : T        │
                   │    分支条件: inline if T == u32         │
                   └────────────────────┬────────────────────┘
                                        │
                  ┌─────────────────────┴─────────────────────┐
                  ▼ (进入 then 分支)                          ▼ (进入 else 分支)
 ┌─────────────────────────────────────┐     ┌─────────────────────────────────────┐
 │ 2a. Then 分支环境 Γ_then            │     │ 2b. Else 分支环境 Γ_else            │
 │   附加局部类型命题: T == u32        │     │   附加局部类型命题: T != u32        │
 │                                     │     │                                     │
 │   ▶ 检查 accept_u32(value):         │     │   ▶ 此时已知 T != u32               │
 │     由 value : T 与 T == u32        │     │     若在此调用 accept_u32(value)    │
 │     证得 value : u32 (通过)         │     │     在定义期 ordinary 检查直接报错  │
 └──────────────────┬──────────────────┘     └──────────────────┬──────────────────┘
                    │ (离开 then 分支)                          │ (离开 else 分支)
                    └─────────────────────┬─────────────────────┘
                                          ▼
                   ┌─────────────────────────────────────────┐
                   │ 3. 汇合点 (Join Point) Γ_after          │
                   │    各分支独有的局部类型假设已退出作用域  │
                   │    仅保留【所有可达前驱共同满足】的结论  │
                   └─────────────────────────────────────────┘
```

局部类型事实的生命周期遵循三条核心规则：
1. **进入分支时生效**：当控制流进入 `then` 分支或匹配成功的模式臂时，提取出的类型命题（如 `T == u32` 或 `T == Tuple<u32, u32>`）加入当前分支环境；进入 `else` 分支或后续互斥分支时，已排除类型命题的否定形式（如 `T != u32`）加入当前分支环境。
2. **离开分支时失效**：当某个分支检查结束时，仅在该分支内成立的局部类型事实和模式绑定变量（如 `Array<'u32, length>` 中的 `length`）随之退出作用域，免于泄漏到外层环境或兄弟分支。
3. **汇合点只保留可达前驱的共同结论**：当多个分支在后续位置汇合时，只有**每一个可达前驱分支都共同满足的类型结论**才会保留在汇合后的环境 $\Gamma_{\text{after}}$ 中。如果某个分支提前 `return` 或发散（`never`），该不可达分支未参与汇合。

---

## 四、 边界示例：跨分支误用局部类型假设 vs 卫语句（Early Return）窄化

根据上述作用域规则，如果在 `else` 分支或离开 `inline if` 之后继续把 `value: T` 当作 `u32` 使用，编译器会在定义端 `ordinary` 检查阶段直接报错：

```flurry
fn accept_u32(x: u32) -> u32 {
    x + 1u32
}

-- ❌ 编译期（ordinary 定义检查阶段）直接报错：
-- 在 else 分支或分支汇合后误用仅属于 then 分支的类型命题 `T == u32`
fn broken_branch_type_assumption(value: T) -> u32 where T {
    inline if T == u32 {
        let _ = accept_u32(value);
    } else {
        -- ❌ 错误：在 else 分支中已知 T != u32，禁止将 value: T 当作 u32 传给 accept_u32
        return accept_u32(value);
    }

    -- ❌ 错误：离开 inline if 后，then 分支的局部类型事实已退出作用域，
    -- 此时 T 重新回到无约束状态，无权调用 accept_u32(value)
    accept_u32(value)
}
```

### 利用提前返回（Early Return）收窄后续主干环境

由于汇合点只考虑**可达前驱（Reachable Predecessors）**，如果我们在 `inline if T != u32` 分支中直接 `return`，那么后续主干代码唯一的可达前驱就满足 `not (T != u32)`（即 `T == u32`），从而使该类型命题在后续主干中持续有效：

```flurry
-- ✅ 利用卫语句（Early Return）将类型相等事实延续到后续主干
fn extract_and_double_if_u32(value: T) -> u32 where T {
    inline if T != u32 {
        -- 当 T 并非 u32 时直接提前返回，该分支不会到达下方主干
        return 0u32;
    }

    -- ✅ 能够到达此处，唯一的可达前驱满足 T == u32
    -- 因此在后续主干代码中，value: T 可直接作为 u32 使用：
    let stepped = accept_u32(value);
    stepped * 2u32
}

test {
    assumes extract_and_double_if_u32(10u32) == 22u32;
    assumes extract_and_double_if_u32(true) == 0u32;
}
```

---

## 五、 本篇小结

| 维度 | 核心机制 | 编译器保证与工程边界 |
| :--- | :--- | :--- |
| **`ordinary` vs `loose`** | 运行时 `fn` 采用 `ordinary` 检查；`comptime fn` 采用 `loose` 检查 | `inline if/when/for` 用于运行时 `fn` 的编译期分支裁剪；`comptime fn` 直接使用普通控制流 |
| **PTC 的职责边界** | 仅提取和追踪**类型命题**（类型相等、类型模式、结构分类、Trait 等） | 不在 PTC 中追踪普通算术大小问题 |
| **类型模式与相等窄化** | `inline if T == u32` / `inline if T is do { ... }` | 在匹配分支内部自动收敛 `T` 的类型身份，使 `value: T` 可按收窄后的类型通过定义期检查 |
| **分支作用域与汇合** | 分支内生效、出分支失效、汇合取可达前驱共同结论 | 局部类型假设不跨分支泄漏；若反例分支提前 `return`，主干自动继承正面类型命题 |

---

## 六、 下一篇预告

当一个泛型函数同时涉及类型相等、Trait 约束、行包含关系与分支局部类型窄化时，如果某条类型约束未能满足，我们该如何阅读编译器的诊断反例并定位缺失的 `where requires` 前提？

下一篇 **《Flurry 演进手记 (087)：读懂类型约束与诊断信息——组合约束推理、残余条件与签名设计》**，我们将总结类型约束的工程实践与诊断方法。
