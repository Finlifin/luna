# Flurry 演进手记 (121)：动态组装一个 `match` 表达式——Quoted Arm 与 `match { use arms }` 接入协议

> **阅读指南**：在上一回（第 120 回）中，我们学会了用 `< T -| Ψ >` 构造模式代码，并在 `~<pattern> => ~<body>` 中将它与表达式代码 `< Ψ |- U >` 手动配对。但当我们在构建协议指令分发器、字节码解释器或编译器重写引擎时，往往有成百上千条匹配规则，每条规则捕获的中间变量 $\Psi_i$ 都不一样！怎样把一条规则的“模式 + 守卫 + 成功主体”紧紧锁成一个高内聚的代码组件？怎样用一行 `use rule_template` 把多条规则批量注入到 `match` 中，同时还让编译器为我们做完整的静态穷尽性检查（Exhaustiveness Check）？本篇我们将结合 `docs/cmtt/patterns-and-arms.md` 与 `docs/cmtt/quote-functions-and-templates.md`，解锁 CMTT 的第三块核心拼图——**Quoted Arm（分支代码）**。

---

## 一、 痛点：当每条分支导出的变量各不相同时，手动配对为什么会失灵？

假设我们在写一个虚拟机指令执行器，指令枚举 `Operation` 有三种形式：
- `.literal(u32)`：携带 1 个操作数；
- `.add(u32, u32)`：携带 2 个操作数 `left` 和 `right`；
- `.multiply(u32, u32)`：携带 2 个操作数 `left` 和 `right`。

### 尝试 `v0`：把模式和主体拆成两个独立的列表传递

如果我们试图用上一回的 `< Operation -| Ψ >` 和 `< Ψ |- u32 >` 把所有规则拆成两个独立的列表传给 `match`（`v0`），马上就会撞上两堵墙：

```flurry
-- 尝试 v0：如果把每条规则拆成独立的 pattern 和 body 两个代码值
-- 痛点 1（内部绑定 Ψ 异构）：
--   .reset 分支导出空上下文 < Operation -| >
--   .literal(v) 分支导出 1 个变量 < Operation -| v: u32 >
--   .add(l, r) 分支导出 2 个变量 < Operation -| l: u32, r: u32 >
--   它们的内部绑定 Ψ 完全不同！
-- 痛点 2（配对错位风险）：
--   如果把模式和主体拆开存放，万一不小心把第 1 条规则的模式和第 2 条规则的主体拼在了一起，
--   或者企图只把主体抽出来当成闭表达式靠外部同名变量硬凑，就会直接破坏绑定来源的安全性！
```

**核心痛点**：
对于外部的 `match` 表达式来说，它根本**不需要也不关心**某一条分支内部自己解构出了几个临时变量 $\Psi$！
外部 `match` 关心的只有三件事：
1. 这条分支需要的**外部上下文 $\Gamma$** 是什么？
2. 这条分支匹配的**主语类型 $T$** 是什么？
3. 这条分支最终算出的**结果类型 $U$** 是什么？

至于分支内部模式产生了什么绑定 $\Psi$、怎么传给它自己的分支主体，应该在分支内部**自产自销、内部闭环**！

---

## 二、 破局（v1）：Quoted Arm——内部闭环 $\Psi$ 的分支代码胶囊

在 Flurry 的 CMTT 理论中，这种把“外部上下文 $\Gamma$、主语类型 $T$、模式导出的内部绑定 $\Psi$ 以及右侧成功主体 $U$”牢牢封装在一起的第 3 类代码值，就叫做 **Quoted Arm（分支代码）**，在编译器内部记作：
$$\text{Arm}(\Gamma;\; T;\; \Psi;\; U)$$

它的精妙之处在于：**模式、守卫（Guard）、成功主体与控制流出口是作为一个不可分割的整体保存的**！你不需要、也不能发明一个裸露的 `Arm<...>` 类型工厂去拆散它；相反，Flurry 直接复用了我们在第一卷学过的 **函数级模式（`case` pattern parameter）** 与 **`quote fn` / `template quote fn`** 作为 Quoted Arm 最自然、最优雅的语法载体！

让我们用一张 Code Cartoon，看清 `Arm(Γ; T; Ψ; U)` 是如何在内部消化掉异构绑定 $\Psi_i$，并对外暴露出统一干净的 `(Γ, T) -> U` 接口的：

```text
  ┌─────────────────────────────────────────────────────────────────────────┐
  │            Quoted Arm 胶囊：Arm(Γ; T = Operation; Ψ_i; U = u32)         │
  │                                                                         │
  │  ┌─ 规则 1: add_rule ────────────────────────────────────────────────┐  │
  │  │ 输入: Operation ──▶ [ case .add(left, right) ]                    │  │
  │  │                              │ 内部私密导出 Ψ_1 = {left, right}   │  │
  │  │                              ▼                                    │  │
  │  │                     [ 主体: left + right ] ──────────▶ 输出: u32  │  │
  │  └───────────────────────────────────────────────────────────────────┘  │
  │                                                                         │
  │  ┌─ 规则 2: reset_rule ──────────────────────────────────────────────┐  │
  │  │ 输入: Operation ──▶ [ case .reset ]                               │  │
  │  │                              │ 内部私密导出 Ψ_2 = ∅ (空绑定!)     │  │
  │  │                              ▼                                    │  │
  │  │                     [ 主体: 0u32 ] ──────────────────▶ 输出: u32  │  │
  │  └───────────────────────────────────────────────────────────────────┘  │
  │                                                                         │
  │  ✨ 外部 match 看到的统一接口：只需输入 Operation，统一交付 u32！       │
  └─────────────────────────────────────────────────────────────────────────┘
```

### 演进到 `v1`：多子句 `quote fn` 保存每条分支的专属绑定关联

在最简单的场景下，我们可以直接定义一个带有多个 `case` 模式子句的 `quote fn`。当调用它时，实参只求值一次，每条分支的模式与其成功主体作为一个完整的 Arm 一起接入：

```flurry
use std.prelude.*;

enum Command {
    reset,
    add(u32),
}

-- 声明一个参数化的 quote fn 签名
quote fn command_value(command: Command) -> u32;

-- 分支 1：匹配 .reset，内部导出空上下文 Ψ = ∅
quote fn command_value(case .reset) -> u32 {
    0u32
}

-- 分支 2：匹配 .add(number)，内部导出 Ψ = { number: u32 } 并供给自身主体
quote fn command_value(case .add(number)) -> u32 {
    number + 1u32
}

fn produce_request() -> Command {
    println("request evaluated once!");
    Command.add(41u32)
}

test {
    -- ✅ produce_request() 严格只求值一次！
    -- .add(number) 分支的绑定 number 与主体 number + 1u32 作为完整 Arm 接入！
    println("result: {}", command_value(produce_request()));
}
```

---

## 三、 终极形态：`template quote fn` 与 `match { use rules }` 规则注入协议（v2）

如果我们正在开发一个大型编译器（比如 Flurry 编译器自己的重写规则库）或可扩展的协议状态机，我们往往希望：
1. 先声明一个代表“某类匹配规则契约”的模板签名 **`template quote fn`**；
2. 允许每条规则单独写成一条**非穷尽（Non-Exhaustive）**的 `quote fn rule_name(case ...) use template_name { ... }`（比如一条规则只负责处理 `.add`，另一条规则只负责处理 `.multiply`）；
3. 最后在主分发函数的 `match` 表达式里，写上一句 **`use template_name`**，把所有注册到该模板下的 Quoted Arm 规则一次性接入进来，并让编译器对**合并后的完整 `match`** 执行穷尽性与互斥性检查！

在 `docs/cmtt/quote-functions-and-templates.md` 中，这被称为 **Arm 接入协议（Arm Import Protocol）**！只需要在 `template quote fn` 上标注两个标准属性：
- `^attrs.quote_fn_template.for_match_arms`：声明该模板提供 `match` 分支接入协议；
- `^attrs.quote_fn_template.allow_non_exhaustive_instance`：允许挂载到该模板下的单条规则实例只覆盖部分模式（不必单条穷尽整个枚举）！

### 演进到 `v2`：用 `match { use evaluate_rule }` 动态拼装穷尽匹配器

让我们直接看这段来自官方规范的经典代码：

```flurry
use std.prelude.*;

enum Operation {
    literal(u32),
    add(u32, u32),
    multiply(u32, u32),
}

-- 1. 定义支持 match 分支接入、允许单条实例非穷尽的规则模板
^attrs.quote_fn_template.for_match_arms
^attrs.quote_fn_template.allow_non_exhaustive_instance
template quote fn evaluate_rule(operation: Operation) -> u32;

-- 2. 独立定义第 1 条 Quoted Arm 规则：只处理 .add(left, right)
quote fn add_rule(case .add(left, right)) use evaluate_rule {
    left + right
}

-- 3. 独立定义第 2 条 Quoted Arm 规则：只处理 .multiply(left, right)
quote fn multiply_rule(case .multiply(left, right)) use evaluate_rule {
    left * right
}

-- 4. 在主 match 中通过 `use evaluate_rule` 批量注入规则，并补齐剩余的 .literal 分支！
fn evaluate(operation: Operation) -> u32 {
    operation match {
        use evaluate_rule,
        .literal(value) => value,
    }
}

test {
    println("{}", evaluate(Operation.multiply(6u32, 7u32)));
}
```

> **💡 核心心智模型：`use evaluate_rule` 的三大铁律**
> 1. **单条允许局部，整体必须穷尽**：`add_rule` 和 `multiply_rule` 各自都只处理了 `Operation` 的一个变体；但当它们通过 `use evaluate_rule` 注入到 `evaluate` 的 `match` 中后，加上手写的 `.literal(value) => value`，正好 100% 穷尽了 `Operation` 的全部三个变体！
> 2. **在单洞规则中只算 1 个接入点**：哪怕 `use evaluate_rule` 背后聚合了 50 条规则分支，在外层 quote 的单洞计数器眼里，一次 `use evaluate_rule` **只计为 1 个接入点**，绝不会按最终展开的分支条数惩罚你！
> 3. **决不靠文件枚举顺序猜重叠模式的优先级**：规则集合的来源与分派契约必须明确（例如互斥模式集合），绝不能把“哪个源文件先被编译器读到”当作重叠模式的匹配优先级！

让我们看看如果在注入 `use evaluate_rule` 后，依然漏掉了 `.literal` 变体，或者企图在一个分支里偷用另一个分支的绑定时，编译器是如何严格把关的：

```flurry
fn broken_evaluate(operation: Operation) -> u32 {
    -- ❌ 编译期直接拒绝！(Non-Exhaustive Match)
    -- evaluate_rule 规则集只覆盖了 .add 和 .multiply，
    -- 缺少对 Operation.literal(_) 的处理，组合后的最终 match 未满足穷尽性契约！
    operation match {
        use evaluate_rule,
    }
}
```

---

## 本篇小结

| 构造 / 属性 | 角色与语法 | 解决的核心痛点 |
| :--- | :--- | :--- |
| **`Arm(Γ; T; Ψ; U)`** | 将外部上下文 $\Gamma$、主语 $T$、导出绑定 $\Psi$ 与主体 $U$ 封装为整体 | 内部消化每条分支异构的临时绑定 $\Psi_i$，防止模式与主体错位拼装 |
| **`quote fn f(case pat)`** | 函数级模式子句 | 实参仅求值一次，将模式与成功主体作为完整分支接入 |
| **`for_match_arms` + `allow_non_exhaustive_instance`** | 修饰 `template quote fn` | 允许将庞大的匹配逻辑拆分为独立维护的单条规则，每条规则无需单独穷尽 |
| **`match { use template, ... }`** | 在 `match` 内部导入规则集合 | 算作单一接入点，注入后与手写分支一起接受编译器完整的静态穷尽性核验 |

---

### 留给下一篇的悬念

到现在为止，我们见到的所有代码值——无论是 `< Ψ |- T >`、`< T -| Ψ >` 还是 `template quote fn`——它们的类型描述在定义时都是完全写死的。

但是，在编写通用的编译期代码生成框架或异构代码容器时，我们有时候会遇到这样的需求：
- 我想写一个编译期辅助函数 `keep(value: Any) -> Any`，或者把几段产出不同类型、拥有不同上下文的代码暂时放进一个通用的编译期变量 `const code: Any = ...` 里；
- 甚至有时候，代码类型写成 `< Ψ |- Any >`（已知是表达式代码，但结果类型用了通配符 `Any`）。

当一段代码值经过了 `Any` 或 `any` 的包装，编译器在最终执行 `~<code>` 接入时，是闭着眼睛随便放行，还是依然能恢复出它底层的真实“代码见证（Code Evidence）”？如果有人异想天开地直接写一句 `~<any>`，想让编译器“随便帮我生成一段能返回 `u32` 的程序”，编译器会答应吗？

下一篇 **《Flurry 演进手记 (122)：当生成的代码类型还不确定时——CMTT 中的 `Any` 与代码见证（Code Evidence）》**，我们将揭开静态通配描述与底层真实代码见证之间的厳密边界！
