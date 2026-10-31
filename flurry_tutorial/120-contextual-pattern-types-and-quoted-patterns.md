# Flurry 演进手记 (120)：连模式匹配也能在编译期动态拼装——模式代码类型 `< T -| x: U >` 与 `'<pat>`

> **阅读指南**：在前面四回（第 116–119 回）中，我们学会了如何用 `< Ψ |- T >` 把表达式变成带上下文类型的编译期代码值。今天，我们要见证 Flurry 元编程体系中真正让其他语言望尘莫及的核心特性——**一等公民的带类型模式代码（Typed Contextual Pattern Code）**。结合 `docs/cmtt/patterns-and-arms.md` 与 `docs/grammar/syntax`，我们将搞清楚：
> 1. 为什么模式代码的类型符号 `< T -| Ψ >` 跟表达式代码 `< Ψ |- T >` 恰好是对偶（Dual）反转过来的？
> 2. 如何用 `'<pattern>` 构造模式代码，并用 `~<pattern>` 把它拼接进 `match` 或 `let` 的模式位置？
> 3. 编译器是如何做**配对定向检查（Paired Directed Checking）**，保证只有当模式匹配成功时，导出的绑定 $\Psi$ 才能安全流入右侧的分支主体？

---

## 一、 问题背景：为什么在其他语言的宏里，模式（Pattern）总是“二等公民”？

假设我们在开发一个网络协议或字节码解释器框架，希望编写一个通用的编译期高阶模板函数 `matching_value`：
- 调用者传入一段**模式规则**（比如匹配 `20..100` 范围内的整数并将其绑定为 `number`），
- 同时传入一段**命中后的处理表达式**（比如计算 `number + 1`），
- 框架负责把它们组装进一个标准的 `match` 结构里，没命中的一律走 `_ => 0` 兜底。

### 模式和表达式无法在类型层面对齐“导出了什么变量”

在 Rust 的 `macro_rules!` 或传统 AST 宏里，虽然你可以捕获一个 `$pat:pat` 片段和一个 `$body:expr` 片段，但编译器在宏定义处无从静态察觉：

```flurry
-- 假想初始写法：如果模式代码没有“导出上下文类型”
-- 隐患 1：模式片段到底匹配什么类型的主语（是 u32 还是 Tuple<u32, u32>）？无从得知。
-- 隐患 2：模式片段匹配成功后，到底绑定了几个变量？叫 number 还是叫 (left, right)？无从得知。
-- 隐患 3：如果在 fallback 分支 `_ => ...` 里偶有疏忽也用到了模式绑定的变量 `number`，
--         定义时根本查不出来。
```

**核心问题**：
表达式和模式在数据流向上恰好是**互逆的对偶关系（Duality）**：
- **表达式（Expression）是“绑定消费者”**：它要求环境先给它提供输入绑定 $\Psi$，然后它算出一个类型 $T$ 的值；
- **模式（Pattern）是“绑定生产者”**：它接收一个类型 $T$ 的被匹配主语（Scrutinee），一旦匹配成功，它会向后续作用域**产出（Export）一组全新的运行时绑定 $\Psi$**。

如果禁止将“被匹配主语类型 $T$”和“成功匹配后导出的绑定上下文 $\Psi$”一起写进模式代码的类型里，模式拼接就永远只是危险的文本赌博。

---

## 二、 核心机制：方向反转的艺术——`< T -| Ψ >` 与 `'<pattern>`

为了精确刻画模式这台“绑定生产机”，Flurry 在 CMTT 中引入了与 `< Ψ |- T >` 良好对称的 **模式代码类型（`contextual_pattern_type`）**：

```text
contextual_pattern_type -> < expr -| runtime_binding* >
quote_pattern           -> '<pattern> | '[ substitution* ] <pattern>
splice_pattern          -> ~<expr> | ~[ substitution* ] <expr>
```

请看这个精巧的符号设计 **`< T -| Ψ >`**：
- 横杠在左（`-`）：左边接收类型为 **`T`** 的被匹配主语（Scrutinee）；
- 竖线在右（`|`）：当且仅当匹配成功时，向右导出新的运行时绑定上下文 **`Ψ`**。

下面把“模式代码 `< T -| Ψ >`”与“表达式代码 `< Ψ |- U >`”放在一起做个面对面的插头对接：

```text
   被匹配主语         1. 模式代码值 (Pattern Code)        2. 表达式代码值 (Expr Code)
   value : T        pattern : < T -| number: u32 >       body : < number: u32 |- u32 >
                    ┌────────────────────────────┐       ┌───────────────────────────┐
                    │ '<(20u32..100u32) as number>│       │     '(number + 1u32)      │
   ─────────▶ [ T ] ┤                            ├──────▶┤                           ├──▶ u32
                    │ • 若匹配成功：生产并导出    │number │ • 在成功分支内消费 number │
                    │   新绑定 number : u32 ─────┼──:u32─▶ • 产出最终结果 u32        │
                    └─────────────┬──────────────┘       └───────────────────────────┘
                                  │
                                  ▼ 若匹配失败 (Failure Path)
                    ┌────────────────────────────┐
                    │ 转向下一分支 `_ => 0u32`   │
                    │ ❌ 失败路径上并不存在 number│
                    └────────────────────────────┘
```

> **🔍 语法小贴士：区分表达式 Quote `'(` 与模式 Quote `'<`**
> 注意括号的形状。
> - 构造**表达式代码**用**圆括号**：`'(number + 1u32)`；
> - 构造**模式代码**用**尖括号**：`'<(20u32..100u32) as number>` 或 `'<number>` 或 `'<(left, right)>`。
> - 而在**接入（Splice）**时，无论是接在表达式位置还是模式位置，都统一写作 `~<code_expr>`（或带重命名的 `~[out = new_name]<code_expr>`）。注意：`~<pat_code>` 尖括号里面写的 `pat_code` 是一个**编译期表达式**（它在编译期求值得到一份模式代码），而非直接写源码模式。

### 配对定向检查（Paired Directed Checking）实战

当一个函数同时接收一份模式代码 `pattern: < u32 -| number: u32 >` 和一份表达式代码 `body: < number: u32 |- u32 >` 时，编译器会执行 **配对定向检查**：
- 在 `~<pattern> => ~<body>` 这一分支中，编译器知道 `~<pattern>` 一旦匹配成功，就会在当前分支作用域里兑现 `number: u32`，因此右侧的 `~<body>` 所需的 `< number: u32 |- ... >` 得到了自动满足（无需手写替换表）。

```flurry
use std.prelude.*;

-- 接收配对的模式代码与分支体代码，在编译期完成 100% 类型校验。
fn matching_value(
    value: u32,
    comptime pattern: < u32 -| number: u32 >,
    comptime body: < number: u32 |- u32 >,
) -> u32 {
    value match {
        -- ✅ 1. ~<pattern> 匹配 u32，成功时自动向当前分支导出 number: u32
        -- ✅ 2. 右侧的 ~<body> 在该成功上下文中直接消费 number，算出 u32。
        ~<pattern> => ~<body>,
        _ => 0u32,
    }
}

-- 模式代码同样可以用于 let 解构并导出多个绑定。
fn add_pair(pair: Tuple<u32, u32>) -> u32 {
    const pattern: < Tuple<u32, u32> -| left: u32, right: u32 > = '<(left, right)>;
    const body: < left: u32, right: u32 |- u32 > = '(left + right);

    -- 在 let 左侧接入模式代码，成功导出 left 与 right 供后续 ~<body> 使用
    let ~<pattern> = pair;
    ~<body>
}

test {
    let first = matching_value(
        42u32,
        '<(20u32..100u32) as number>,
        '(number + 1u32),
    );
    let second = matching_value(
        12u32,
        '<(20u32..100u32) as number>,
        '(number + 1u32),
    );
    println("match: {} {}, pair: {}", first, second, add_pair((19u32, 23u32)));
}
```

---

## 三、 每次接入刷新身份与失败路径隔离红线

如果我们把**同一份**模式代码常量 `const pattern: < u32 -| number: u32 > = '<number>;` 在同一个函数里先后 `~<...>` 接入**两次**，会发生变量冲突或者共享同一个运行时内存槽吗？

**并非如此。** `docs/cmtt/patterns-and-arms.md` 明确规定：
> **“每次 pattern splice 都产生新绑定身份。重用同一代码值免令两场匹配共享一份运行时变量。”**

配合我们在上一回（第 119 回）学过的替换表 `~[number = first]<pattern>`，我们可以把同一份模式代码复用任意多次，每次导出为不同的新局部变量名。

### 复用同一模式代码导出独立绑定，并拦截失败路径越权

```flurry
use std.prelude.*;

-- 同一份模式代码接入两次，通过替换表导出为两个完全独立的运行时绑定。
fn extract_twice(left: u32, right: u32) -> u32 {
    const pattern: < u32 -| number: u32 > = '<number>;
    const read: < number: u32 |- u32 > = '(number);

    -- 第一次模式接入：将导出的接口名 number 绑定为新变量 first
    let ~[number = first]<pattern> = left;
    -- 第二次模式接入：将导出的接口名 number 绑定为新变量 second
    let ~[number = second]<pattern> = right;

    -- 分别用 first 和 second 供应给 read 表达式代码
    ~[number = first]<read> + ~[number = second]<read>
}

test {
    println("twice: {}", extract_twice(20u32, 22u32));
}
```

相反，如果有人试图在**匹配失败的 fallback 分支**里偷用模式成功时才导出的绑定，或者试图把两场毫无瓜葛的匹配导出的变量胡乱拼凑成有依赖关系的上下文，编译器会直接报错：

```flurry
fn broken_pattern_splice(
    value: u32,
    comptime pattern: < u32 -| number: u32 >,
    comptime body: < number: u32 |- u32 >,
) -> u32 {
    value match {
        ~<pattern> => ~<body>,
        -- ❌ 编译期直接报错。
        -- 只有 ~<pattern> 匹配成功的分支才拥有导出的绑定 number: u32。
        -- 在失败兜底分支 `_` 中，number 全然无存，无法供应给 ~<body>。
        _ => ~<body>,
    }
}
```

---

## 本篇小结

| 维度 | 表达式代码（Expr Code） | 模式代码（Pattern Code） |
| :--- | :--- | :--- |
| **类型记号与方向** | `< Ψ \|- T >`（消费绑定 $\Psi$，产出值 $T$） | `< T -\| Ψ >`（匹配主语 $T$，成功时导出新绑定 $\Psi$） |
| **Quote 构造语法** | 圆括号：`'(expr)` / `'[sub](expr)` | 尖括号：`'<pattern>` / `'[sub]<pattern>` |
| **Splice 接入语法** | `~<code_expr>`（位于表达式位置） | `~<pat_expr>`（位于 `match` 分支左侧或 `let` 模式位置） |
| **核心安全保证** | 锁定外部引用身份，防止同名变量篡改 | 每次接入生成新鲜绑定身份；严格限制导出绑定仅在成功路径可见 |

---

### 下一篇预告

看看我们在 `matching_value` 里写的代码：
```flurry
~<pattern> => ~<body>,
```
我们已经能够把一个模式代码 `< T -| Ψ >` 和一个表达式代码 `< Ψ |- U >` 手工配对成一条 `match` 分支了。

但是，在真正的编译器或协议状态机里，一个 `match` 往往有**几十条甚至上百条规则分支（Arms）**，而且每一条分支导出的内部绑定 $\Psi_i$ 都各互异（比如第一条分支匹配 `.reset` 导出 0 个变量，第二条分支匹配 `.add(left, right)` 导出 2 个变量，第三条分支匹配 `.jump(target)` 导出 1 个变量）。

如果我们想把“模式 + 守卫 + 右侧成功主体”**打包成一个浑然一体的整体单元**，甚至把分散在不同地方定义的几十条规则一次性通过 `match { use rules, ... }` 注入进同一个 `match` 表达式里，该怎么做？

下一篇 **《Flurry 演进手记 (121)：动态组装一个 `match` 表达式——Quoted Arm 与 `match { use arms }` 接入协议》**，我们将揭晓 CMTT 的第三类代码值——**Quoted Arm（分支代码）**。
