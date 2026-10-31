# Flurry 演进手记 (117)：读懂 `< x: i32 |- u64 >`——显式运行时上下文（Context）与表达式代码类型

> **阅读指南**：在上一回（第 116 回）中，我们开启了第六卷的元编程之旅，认识了 **上下文模态类型论（CMTT）** 的三类一等代码值。今天，我们要把放大镜对准其中最基础、也最重要的一类——**表达式代码类型 `< Ψ |- T >`**。一段包含自由运行时变量 `x` 和 `y` 的代码片段，在编译期究竟长什么样？为什么我们可以把它想象成一块“自带定制插头的电路板”？空上下文 `< |- T >` 和纯函数是一回事吗？为什么“推导出的固定外部依赖”无法随便改名替换成别的变量？本篇我们将结合 `docs/cmtt/index.md` 与 `docs/cmtt/quotation-and-stages.md`，读懂 `< Ψ |- T >`。

---

## 一、 问题背景：一段“半成品”代码，怎么才能在不运行的情况下通过类型检查？

回想一下我们在设计有限状态机（FSM）编译器时的场景：
在编译期，我们要为状态机的每一条转移边（Transition Edge）生成一小段判断代码，比如：
> “如果当前运行时状态 `state` 等于 `0`，且当前读入的运行时字节 `byte` 等于 `'a'`（即 `97u8`），那么产出新状态 `1`，否则……”

请注意：在编译期生成这段代码的那一刻，**程序根本还没有运行。** 既没有真正的网络数据包进来，也没有真正的运行时栈帧上的 `state` 和 `byte` 变量。

### 要么禁止引用未定义的运行时变量，要么放弃定义期检查

如果编译器只有普通的“当前作用域符号表查找”：
- 当你在编译期生成函数 `build_edge()` 里写下 `state == 0 and byte == 97` 时，如果编译器按普通表达式去查表，它会立刻报错：`error: cannot find value 'state' in this scope`。因为 `state` 和 `byte` 是未来接入到目标函数后才存在的运行时变量。
- 反过来，如果为了不报这个错，编译器干脆放弃在 `build_edge()` 里检查 `state == 0 and byte == 97` 的类型，那就又退回到了我们在第 116 回批评过的“无类型 AST 黑盒宏”。

**核心问题**：
我们需要一种类型注解，能够明确告诉编译器：“在这段被引用的代码内部，请假定存在一个类型为 `u32` 的虚拟运行时绑定 `state`，以及一个类型为 `u8` 的虚拟运行时绑定 `byte`；请在这个**虚拟上下文（Virtual Context）**下立即完成对代码主体的类型检查，并把这份上下文需求作为代码值的公开接口保存下来。”

---

## 二、 核心机制：把 `< Ψ |- T >` 看作一块“带插头的电路板”

这正是 Flurry 中 **`contextual_expr_type`** 的语法设计初衷。在 `docs/grammar/syntax` 与 `docs/cmtt/index.md` 中，它的语法定义为：

```text
contextual_expr_type -> < runtime_binding* |- expr >
runtime_binding      -> id : expr
```

比如，类型 `< x: i32, y: bool |- u64 >` 读作：
> **“在提供运行时绑定 `x: i32` 与 `y: bool` 的上下文（`|-` 左侧）下，能够求值并交付 `u64` 类型运行时结果（`|-` 右侧）的表达式代码。”**

我们可以借助**“带插头的电路板（Plug-and-Socket Circuit Board）”**这一类比，来建立对 `< state: u32, byte: u8 |- ?u32 >` 最直观的视觉心智模型：

```text
          表达式代码值：const step_code: < state: u32, byte: u8 |- ?u32 >
          
                  ┌───────────────────────────────────────────┐
     输入端口 1   │               代码电路板主体               │
    state : u32 ──▶─┐                                         │
                  │ ├──▶ [ if state == 0 and byte == 97 ... ] ├──▶ ?u32 (输出插座)
     输入端口 2   │ ┌──▶                                      │
     byte : u8  ──▶─┘                                         │
                  └───────────────────────────────────────────┘
                  ◀─────────── 左侧：运行时上下文 Ψ ──────────▶   ◀─ 右侧：结果 T ─▶
```

看着这块“电路板”，你就能瞬间明白编译器的两步检查逻辑：
1. **造板子时（Quote 构造期）**：编译器根据 `< state: u32, byte: u8 |- ?u32 >` 左侧声明的虚拟端口，在内部建立一个虚拟运行时环境 $\Psi = \{\text{state}: \text{u32}, \text{byte}: \text{u8}\}$，然后顺着电线检查板子内部的电路 `if state == 0u32 and byte == 97u8 { 1u32 } else { null }` 是否合法，以及右侧输出端是否真的能产出 `?u32`。
2. **插板子时（Splice 接入期）**：当你用 `~[state = cur_state, byte = cur_byte]<step_code>` 把板子插进真实的运行时函数时，编译器只需核对焊点上的两根线 `cur_state` 和 `cur_byte` 是否真的满足 `u32` 和 `u8`，根本无需再拆开板子重查一遍内部逻辑。

### 用 `< Ψ |- T >` 编写编译期单步状态判断生成器

让我们写一段真实的 Flurry 代码，体验用 `< state: u32, byte: u8 |- ?u32 >` 在编译期生成状态转移代码：

```flurry
use std.prelude.*;

-- 编译期生成器：给定起始状态 expected_state、触发字节 trigger 和目标状态 next_state，
-- 返回一块带有 (state: u32, byte: u8) 两个输入端口、输出 ?u32 的代码电路板。
pure comptime fn make_single_edge_code(
    expected_state: u32,
    trigger: u8,
    next_state: u32,
) -> < state: u32, byte: u8 |- ?u32 > {
    -- 在这里，state 和 byte 来自返回类型声明的虚拟上下文接口 Ψ。
    -- 而 expected_state、trigger、next_state 则是已完成求值的编译期常量。
    '(if state == expected_state and byte == trigger {
        next_state
    } else {
        null
    })
}

fn run_edge(cur_state: u32, cur_byte: u8) -> ?u32 {
    const edge_code = make_single_edge_code(0u32, 97u8, 1u32);
    -- 将当前运行时形参 cur_state 和 cur_byte 接入电路板的两个端口
    ~[state = cur_state, byte = cur_byte]<edge_code>
}

test {
    let hit = run_edge(0u32, 97u8);
    let miss = run_edge(0u32, 98u8);
    println("hit: {}, miss: {}", hit is u32, miss is null);
}
```

> **🔍 语法小贴士：空上下文 `< |- T >` 代表这段代码“没有副作用”吗？**
> 仔细看 `docs/cmtt/quotation-and-stages.md` 中的重要提醒：
> **“$\Psi$ 为空（即 `< |- T >`）只表示这段代码没有局部 runtime 绑定需求，并未保证代码纯（pure）、无全局访问或无代数效应。”**
> 举个例子：
> ```flurry
> pure comptime fn greeting_code() -> < |- void > {
>     '(println("hello"))
> }
> 
> ```
> - 为什么 `greeting_code` 可以标记为 `pure comptime fn`？因为在**编译期构造** `'(println("hello"))` 这份代码值时，它只是把代码装进了盒子里，**并没有在编译期打印 `"hello"`**。所以编译期构造过程是 100% 纯的。
> - 为什么返回类型是空上下文 `< |- void >`？因为 `println("hello")` 无需读取任何局部运行时变量 `x` 或 `y`；等它在运行期被 `~<greeting_code()>` 接入执行时，才会在运行期真正向终端输出 `"hello"`。

---

## 三、 深入细节：上下文依赖次序与“虚拟端口 vs. 固定依赖”

在使用 `< Ψ |- T >` 时，有三条尤为关键的语义边界（见 `docs/cmtt/index.md` 与 `contexts-and-substitution.md`）决定了你的代码能否通过编译：

### 边界 1：上下文 $\Psi$ 只装 `runtime` 绑定，且保持从左到右的依赖次序

1. **阶段纯粹性**：`< Ψ |- T >` 中的 $\Psi$ **只包含运行时（`runtime`）绑定**。像 `Type`、另一份代码值或编译期配置元信息，全都住在 `comptime` 环境里，无法塞进 $\Psi$ 当成运行时上下文成员。
2. **有序依赖性**：当 $\Psi$ 中有多个绑定 `< a: A, b: B |- T >` 时，它们不是无序集合，而是**按依赖次序排列的列表**——后一项 `b` 的类型可以依赖前一项 `a` 允许携带的 runtime identity（例如依赖长度或关联属性）。

### 边界 2：“显式暴露的虚拟端口” vs. “推导出的固定外部依赖”

当你在函数内部写一段 `const code = ...` 时，有两种迥然各异的引用方式：
- **方式 A（显式虚拟端口）**：通过类型注解 `const code: < input: i32 |- i32 > = '(input + 1i32);` 或显式端口映射 `const code = '[port = a](a + 1i32);` 声明的输入。它是一个**可自由重连的插座端口**，接入时可以用 `~[input = other]<code>` 接到任何类型兼容的变量上。
- **方式 B（直接捕获外层局部变量形成的固定依赖）**：如果你在运行时函数 `fn foo(a: i32)` 里直接写 `const code = '(a + 1i32);`（既没声明虚拟端口，也没用 `'[port = a]` 暴露端口），编译器会自动推导出 `code` 依赖外层的**固定绑定 `a`**。

**核心规则**：**固定外部依赖无法借由重新标注接口或替换表，强行变成可任意替换的自由端口。**

让我们看看违反这些边界时，编译器是如何精准拦截的：

```flurry
fn broken_context_rules(fixed_var: i32, replacement: i32) -> i32 {
    -- 1. 直接捕获外层运行时形参 fixed_var，未将其声明为可替换端口
    const captured = '(fixed_var + 1i32);

    -- 负例 1：企图把固定外部依赖当作自由端口强行重定向给 replacement
    -- ❌ 编译期直接报错。
    -- captured 中的 fixed_var 是推导出的固定依赖，不是公开的可替换端口。
    -- 若想重定向，必须在构造时用 '[port = fixed_var](fixed_var + 1i32) 显式暴露端口。
    let _ = ~[fixed_var = replacement]<captured>;

    -- 负例 2：定义了需要虚拟端口 needed 的开放代码，接入时却未提供任何绑定
    const open_code: < needed: i32 |- i32 > = '(needed * 2i32);
    -- ❌ 编译期直接报错。
    -- 当前作用域既没有默认对应的 needed，也没有通过 ~[needed = ...] 显式供应。
    ~<open_code>
}
```

### 用显式端口暴露实现安全的接口改名与重连

如果我们确实想先根据当前变量 `a` 构造一段代码，同时允许后续接入时把 `a` 的位置替换成别的变量 `replacement`，正确的做法是在构造时用 `'[port = a](...)` 显式把 `a` 暴露为端口，或者直接声明虚拟上下文 `< port: i32 |- i32 >`：

```flurry
-- 显式声明或暴露端口，支持后续安全改名与重连
fn safe_port_redirection(a: i32, b: i32, replacement: i32) -> i32 {
    -- '[port = a] 明确告诉编译器：
    -- 把表达式里的 a 暴露为可替换接口端口 port，而 b 保持为对原形参 b 的固定引用。
    const code = '[port = a](a + b);

    -- ✅ 接入时，将公开端口 port 重定向到 replacement (40)，
    -- 而固定引用 b 自动沿用原形参 b (2)，最终算出 40 + 2 == 42。
    ~[port = replacement]<code>
}

test {
    println("{}", safe_port_redirection(1i32, 2i32, 40i32));
}
```

---

## 本篇小结

| 语法 / 概念 | 含义与作用 | 核心检查契约 |
| :--- | :--- | :--- |
| **`< x: A, y: B \|- T >`** | 带显式运行时上下文 $\Psi$ 的表达式代码类型 | 在虚拟上下文 $\Psi$ 中预先完成代码主体的类型检查 |
| **`< \|- T >`** | 空上下文（Closed）表达式代码类型 | 独立于局部 runtime 绑定，但主体在运行期仍可包含 I/O 或效应 |
| **上下文顺序与阶段** | $\Psi$ 按从左到右依赖顺序排列 | 仅允许包含 `runtime` 绑定；`Type` 与代码值留在 `comptime` |
| **虚拟端口 vs. 固定依赖** | 接口声明的端口可重连；直接捕获的外层变量为固定依赖 | 固定依赖锁定原变量身份，禁止被篡改为任意自由端口 |

---

### 下一篇预告

我们已经读懂了 `< Ψ |- T >` 这块“带插头电路板”的接口说明书，也初步见识了 `'(...)` 和 `~<...>` 的配合。

但作为一个刨根问底的程序员，你一定注意到了 `docs/cmtt/quotation-and-stages.md` 里的两条耐人寻味的限制：
1. **为什么 Flurry 规定 CMTT 采用“两阶段（Two Stages）”与“Rank-1 上下文多态”**，而不像某些学院派语言那样允许无限多层嵌套 `'( '( '( x ) ) )`？
2. 更让人好奇的是：**什么是“单洞规则（Single-Hole Rule）”？** 为什么在一个 `'(...)` 引用内部，写 `'(~<left> + 1)` 完全合法，但直接写 `'(~<left> + ~<right>)`（同一个 quote 里放两个 splice 洞）却会被编译器编译期报错？

下一篇 **《Flurry 演进手记 (118)：把代码装进盒子再拼出来——Quote `'(`、Splice `~<...>` 与阶段安全保证》**，我们将深入拆解这两条让类型推导与性能分析保持 100% 确定性的核心原则。
