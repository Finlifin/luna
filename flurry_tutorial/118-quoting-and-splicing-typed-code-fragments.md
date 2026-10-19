# Flurry 演进手记 (118)：把代码装进盒子再拼出来——Quote `'(`、Splice `~<...>` 与阶段安全保证

> **阅读指南**：在上一回（第 117 回）中，我们把 `< Ψ |- T >` 比作了带插头的电路板。今天，我们要深入研究制造电路板的操作 **`'(expr)`（Quote，代码引用）** 与焊接电路板的操作 **`~<code_expr>`（Splice，代码拼接）**。在结合 `docs/cmtt/quotation-and-stages.md` 与 `docs/comptime/quoting-and-splicing.md` 的学习中，我们将解开三个最关键的设计谜题：
> 1. 为什么把普通编译期整数（如 `24`）放进运行时代码里不用写 `~<...>`，而拼接一段代码值却必须写 `~<...>`？
> 2. 为什么 Flurry 坚持 **两阶段（Two Stages）** 与 **Rank-1 上下文多态**，拒绝无限层级的“俄罗斯套娃”式 Quote？
> 3. 什么是 **单洞规则（Single-Hole Constraint）**？为什么每个 `'(...)` 最多只允许包含一个接入点，而这又为什么丝毫不影响我们生成任意复杂的循环和分支程序？

---

## 一、 痛点：当“数据跨阶段”与“代码跨阶段”混成一团

在很多支持编译期计算的语言里，初学者最容易搞混两件事：
- **事情 A（普通数据跨阶段，Data Splicing）**：我在编译期算出了一个常数（比如状态总数 `const NUM_STATES: u32 = 16u32;`），现在我想在运行时的代码里用这个数字 `16u32`。
- **事情 B（代码片段跨阶段，Code Splicing）**：我在编译期构造了一段含运行时逻辑的代码值（比如 `const check_code: < s: u32 |- bool > = '(s < 16u32);`），现在我想把这段代码焊接到运行时的函数体里去执行。

### 尝试 `v0`：不区分阶段边界与多洞拼接导致的推导灾难

如果一门语言不严格区分“两阶段边界”，甚至允许在一段未定型的 Quote 里随意挖无数个洞（`'( ~<a> + ~<b> + ~<c> )`），或者允许代码值直接泄漏成运行时的动态对象（`v0`）：

```flurry
-- 尝试 v0：把运行时才能决定的条件和编译期代码生成混为一谈
fn confused_stage_v0(runtime_flag: bool) -> u32 {
    -- 错误幻想：在运行期根据 runtime_flag 临时挑一段编译期代码再拼接？
    -- 如果允许这样做，意味着运行时二进制文件里必须塞进一个编译器/解释器！
    const code = if runtime_flag { '(11u32) } else { '(22u32) };
    ~<code>
}
```

不仅如此，如果在同一个 `'(...)` 内部允许任意多个未受控的 `~<left>` 和 `~<right>` 自由组合，当两边各自携带未知的上下文、不同的提前返回边界（`return`/`break`）、不同的代数效应与仿射资源移动时，编译器在尚未展开前根本无法线性地确定这两个洞之间的求值顺序、控制流汇合与上下文合并契约！

---

## 二、 破局（v1）：清晰的两阶段转换与 Rank-1 铁律

Flurry 用极其干净利落的 **两阶段（Two Stages）** 模型斩断了上述混乱：

1. **普通数据走 `Splice` Trait 自动物化**：
   当一个编译期算出的普通数据（如整数 `1 + 23`、布尔值、或实现了 `Splice` trait 的结构体）出现在运行时表达式位置时，它通过 `Splice.splice(comptime data: Self) -> RuntimeTarget` 协议转换为运行时值（常量可直接物化为立即数）。你**不需要**为普通数字写 `~<24>`！
2. **代码值走 `'(...)` 与 `~<...>` 阶段切换**：
   - **Quote `'(`**：从 `comptime` 开启一段**描述未来 `runtime` 主体**的盒子。盒子里写的是未来的运行时操作，而盒子本身（类型为 `< Ψ |- T >`）是一个纯粹的 **`comptime` 值**！
   - **Splice `~<...>`**：在 `runtime` 主体位置打开一个窗口，窗口里的操作数**回到 `comptime` 求值**拿到一份代码盒子，然后把盒子里的运行时主体无缝焊接到当前位置！

让我们用一张 Code Cartoon，看清 `'(...)` 与 `~<...>` 是如何在 `comptime` 和 `runtime` 两个宇宙之间来回穿梭，却始终只保持**两个阶段**的：

```text
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 编译期宇宙 (Comptime Stage)                                             │
  │                                                                         │
  │   pure comptime fn add_one(code: < x: i32 |- i32 >)                     │
  │       -> < x: i32 |- i32 >                                              │
  │   {                                                                     │
  │       '( ~<code> + 1i32 )                                               │
  │       ▲    ▲                                                            │
  │       │    └── 2. ~<...> 的操作数 `code` 回到 comptime 取出内层代码盒子 │
  │       └─────── 1. '(...) 开启一段新的 runtime 主体描述，包装成新盒子返回│
  └───────────────────────────────────┬─────────────────────────────────────┘
                                      │ 交付最终代码值 combined
                                      ▼
  ┌─────────────────────────────────────────────────────────────────────────┐
  │ 运行时宇宙 (Runtime Stage)                                              │
  │                                                                         │
  │   fn add_two(value: i32) -> i32 {                                       │
  │       const original: < x: i32 |- i32 > = '(x + 1i32);                  │
  │       const combined = add_one(original);                               │
  │       ~[x = value]<combined>  ──▶ 3. 焊接完成：(value + 1i32) + 1i32    │
  │   }                                  运行时无任何 quote 对象！          │
  └─────────────────────────────────────────────────────────────────────────┘
```

### 演进到 `v1`：编译期选择代码 vs. 生成包含运行时条件的代码

理解了“生成器在 `comptime`、代码主体在 `runtime`”，我们就能游刃有余地区分**编译期分支选择**与**运行期分支生成**：

```flurry
use std.prelude.*;

-- 情形 A：条件 first 在编译期已知！
-- 生成器在编译期直接挑中其中一份代码返回，未选中的分支完全不进入目标程序。
pure comptime fn chosen_code(first: bool) -> < |- u32 > {
    if first { '(11u32) } else { '(22u32) }
}

fn selected(comptime first: bool) -> u32 {
    ~<chosen_code(first)>
}

-- 情形 B：条件 flag 在运行期才输入！
-- 我们在编译期构造一份【内部包含 runtime if】的开放代码 < flag: bool |- u32 >！
fn runtime_choice(flag: bool) -> u32 {
    const code: < flag: bool |- u32 > = '(if flag { 11u32 } else { 22u32 });
    ~<code>
}

test {
    println("comptime choice: {} {}", selected(true), selected(false));
    println("runtime choice: {} {}", runtime_choice(true), runtime_choice(false));
}
```

> **💡 核心心智模型：什么是 Rank-1 上下文多态？**
> 在 `docs/cmtt/quotation-and-stages.md` 中，Flurry 规定 CMTT 采用 **Rank-1**：
> - **允许（Rank-1）**：在生成器或模板的**最外层**对上下文进行量化（即 `forall Ψ, T. Expr(Ψ, T) -> Expr(Ψ, T)`）。不同调用点可以传入具有不同上下文接口的代码，一旦调用确定了本次的上下文 $\Psi$，函数内的代码参数就引用这同一个确定的接口。
> - **不提供（Higher-Rank）**：不允许把一个“尚未决定上下文、可在函数内部被任意实例化为各种不同上下文的多态代码方案 `forall Ψ. Expr(Ψ, T)`”当作普通单态参数传进函数里到处乱变。
> 这条 Rank-1 边界保证了：**每一次代码生成调用的上下文接口都是具体、透明、可静态核验的，绝不会陷入高阶上下文合一不可判定的泥潭！**

---

## 三、 深入“单洞规则（Single-Hole Rule）”：为什么一洞就够造万物？（v2）

现在来看初学者最常问的一个问题：
在 `docs/cmtt/quotation-and-stages.md` 中规定：**每个 `quote` 的 runtime 主体至多包含一个代码接入点（Single Hole）**。

注意计数规则的精准边界：
1. **一段 `'(...)` 或 `quote fn` 的主体是一个计数单元**：在它里面最多只能出现 **1 个** 显式 `~<...>`（或者 **1 次** 隐式的 `quote fn` 应用）。哪怕把两个 `~<left>` 和 `~<right>` 分别写在同一个 `'(if cond { ~<left> } else { ~<right> })` 的两个互斥分支里，也算作 **2 个洞**，会被直接拒绝！
2. **普通的 `fn` 函数体不是一段 `quote`！** 所以在普通的运行时函数 `fn foo()` 里面，你想写多少个独立的 `~<code1>`、`~<code2>` 或者调用多少次 `quote fn` 都**完全合法**！
3. **每次套娃包装只用一个洞，就能串起任意长的链条！** 就像我们在 `add_one` 里写的那样，第一层 `original` 是 `'(x + 1i32)`（0 个洞），传给 `add_one` 包装成 `'(~<code> + 1i32)`（这层 quote 只有 1 个洞！），要是再套十层 `add_one`，每一层 quote 都严格只有 1 个洞，但最终拼出来的程序却拥有十次加法！

让我们看看违反单洞规则时编译器的拦截，以及如何用**逐层单洞叠加**或**普通函数多点接入**优雅演进到 `v2`：

```flurry
-- 负例：在同一个 quote 主体内部挖了两个 splice 洞
pure comptime fn invalid_sum(
    left: < |- i32 >,
    right: < |- i32 >,
) -> < |- i32 > {
    -- ❌ 编译期直接拒绝！(违反单洞规则 Single-Hole Rule)
    -- 同一个 '(...) 的 runtime 主体中出现了两个显式接入点 ~<left> 与 ~<right>！
    -- 编译器线性扫描源码 quote 即可定位第二个接入点报错，无需展开后才发现。
    '(~<left> + ~<right>)
}

-- ✅ 演进到 v2（方案 A）：在普通 runtime 函数体中，允许任意多个独立接入点！
fn valid_sum_in_fn() -> i32 {
    const left: < |- i32 > = '(20i32);
    const right: < |- i32 > = '(22i32);
    -- 普通 fn 不是 quote，这里两个 ~<...> 各自独立接入，完全合法！
    ~<left> + ~<right>
}

-- ✅ 演进到 v2（方案 B）：在编译期生成器中，通过单洞链式包装逐步累积计算！
pure comptime fn add_constant(
    prev: < x: i32 |- i32 >,
    delta: i32,
) -> < x: i32 |- i32 > {
    -- 每一层 quote 严格只含 1 个洞 ~<prev>，而 delta 是普通编译期数据直接物化！
    '(~<prev> + delta)
}

fn chained_add(input: i32) -> i32 {
    const step1: < x: i32 |- i32 > = '(x);
    const step2 = add_constant(step1, 10i32);
    const step3 = add_constant(step2, 32i32);
    ~[x = input]<step3>
}

test {
    println("sum: {}, chained: {}", valid_sum_in_fn(), chained_add(0i32));
}
```

为什么“单洞规则”如此高明？
因为当每一层 `'(...)` 最多只有一个子代码接入点 `~<prev>` 时，控制流、上下文替换、变量生命周期与代数效应的传递关系变成了一条**没有分叉歧义的单向管道**！编译器对每一份 `quote` 只需做一次 $O(N)$ 的线性扫描，就能在未展开前 100% 锁定接入点的上下文与控制边界！

---

## 本篇小结

| 规则 / 构造 | 核心契约 | 为什么这样设计？ |
| :--- | :--- | :--- |
| **普通数据跨阶段（`Splice`）** | 走 `Splice.splice` 协议自动转换或常量物化 | 让编译期算出的数字/配置直接融入代码，无需手写 `~<...>` |
| **代码跨阶段（`'(...)` / `~<...>`）** | `'(...)` 构造 `comptime` 代码值；`~<...>` 接入 `runtime` 位置 | 严格保持两阶段（Two Stages），绝不在运行期遗留动态代码对象 |
| **Rank-1 上下文多态** | 仅在生成器/模板最外层量化上下文 $\Psi$ | 保证每次调用时上下文接口具体透明，类型推导 100% 可判定 |
| **单洞规则（Single-Hole）** | 每段 `quote` / `quote fn` 主体至多 1 个接入点；普通 `fn` 不限 | 将嵌套组合转化为清晰的线性单洞链，定义期即可精准定位诊断 |

---

### 留给下一篇的悬念

在这两篇里，我们已经用到了一个极其强大的接线语法：`~[x = input]<step3>` 和 `'[port = a](a + b)`。

在构建大型状态机或组合多段协议解析代码时，我们经常会遇到这样的场景：
- 代码片段 A 期望上下文里有两个变量 `< left: i32, right: i32 |- i32 >`，我们想在接入时**把 `left` 和 `right` 的值互换**（`~[left = right, right = left]<code>`）——编译器会不会笨拙地先用 `right` 覆盖掉 `left`，导致两边最后都变成了 `right`？
- 如果一个 `template fn` 已经在内部帮我们算好了局部变量 `let adjusted = value + 1;`，为什么我们在调用模板时连 `~[adjusted = adjusted]` 都不用写，就能直接形成**默认对应（Default Correspondence）**？

下一篇 **《Flurry 演进手记 (119)：连接两段代码的管道工——上下文替换 `~[ a = b ] <expr>` 与变量重绑定》**，我们将把 CMTT 的“管道接线学”研究得明明白白！
