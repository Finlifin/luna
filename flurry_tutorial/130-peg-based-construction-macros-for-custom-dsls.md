# Flurry 演进手记 (130)：在 Flurry 里直接嵌一门小语言（DSL）——基于 PEG 语法的构造宏 @define_macro2

在上一回（129）中，我们学习了第一类构造宏 `@define_macro1`。它通过读取处理函数的参数类型签名，直接按 Flurry 现成的 AST 节点（如 `FlurryPattern`）解析 `@path(args)` 的输入。

然而，在真实的系统工程中，我们经常渴望在代码里直接内嵌一门**领域特定语言（Domain-Specific Language，DSL）**：
- 比如在写硬件驱动时，想直接写寄存器位域配置表；
- 在写网络网关时，想直接写状态机转移规则或数据包过滤规则（我们将在第 135 回用它打造 Mini-BPF。）；
- 或者想定义一个块状宏 `@statements { ... }`，规定花括号里面**只能写零条或多条语句（`FlurryExtendedStatement*`）**，绝不允许混入顶层模块声明。

面对这种需求，`@define_macro1` 的圆括号参数列表不够用了，而如果退化到第 3 级 `^macro3`（裸 `TokenBuffer`），宏作者又得苦哈哈地自己从零手写递归下降解析器、处理报错位置和回溯。

为此，Flurry 提供了构造宏家族中最富创造力的二号成员——**基于 PEG（解析表达文法，Parsing Expression Grammar）的构造宏 `@define_macro2`**。今天，我们就来看看如何用几行文法规则，在 Flurry 里合法地“发明一门小语言”。

---

## 一、 问题背景与设计思路：为什么内嵌 DSL 最怕手写 Token 解析器？

在传统支持过程宏的语言里（如 Rust 的 `proc_macro`），当你写下 `my_dsl! { ... }` 时，编译器丢给你的是一个扁平的 `TokenStream`。
如果你想在里面解析哪怕最简单的自定义语法：
1. **语法规则藏在过程式代码里**：别人看你的宏源码，只能看到满屏的 `iter.next()`、`peek()`、`if token == ...`，根本看不出这门 DSL 的正式语法（BNF / PEG）长什么样。
2. **很难复用宿主语言的现成语法**：假如你的 DSL 里有一半是自定义的头部，另一半就是一个标准的 Flurry 表达式或语句，手写解析器很难无缝调用宿主编译器自己的 AST 解析规则。
3. **IDE 与报错体验割裂**：手写的 `if-else` 解析一旦失败，很难给出标准统一的语法期望提示。

Flurry 的 `@define_macro2` 是怎么彻底解决这三个问题的？
它让你直接在宏定义的开头**声明式地写出 PEG 语法规则**，并且允许你在 PEG 规则里**直接引用 Flurry 自身的语法非终结符（如 `FlurryExtendedStatement`、`FlurryExpr`、`FlurryPattern`）**。

---

## 二、 核心语法：`@define_macro2 { peg_rule* mapping_definition }`

让我们打开 `docs/macro/construction-macro.md`，看看 `@define_macro2` 的语法结构与解析原理：

```
+-------------------------------------------------------------------------+
|                 @define_macro2 的双段式内部结构                          |
+-------------------------------------------------------------------------+
|                                                                         |
|  const my_dsl = @define_macro2 {                                        |
|      -- [ 第 1 段：PEG 语法产生式 (peg_rule*) ]                          |
|      -- 从入口规则 `Entry -> ...` 开始，定义花括号内的合法语法结构         |
|      Entry -> FlurryExtendedStatement*;                                 |
|                                                                         |
|      -- [ 第 2 段：映射处理函数 (mapping_definition) ]                   |
|      -- 接收按 Entry 规则解析出的强类型 AST 结构，返回 TokenBuffer         |
|      fn(body: Repetition<meta.ast.FlurryExtendedStatement>)             |
|          -> TokenBuffer {                                               |
|          {{ $body }}                                                    |
|      }                                                                  |
|  }                                                                      |
+-------------------------------------------------------------------------+
```

### 2.1 内建 PEG 组合算子速查

在 `@define_macro2` 的第一段中，你可以使用标准的 PEG（Parsing Expression Grammar）算子来拼装语法规则：

| PEG 算子 | 语法表示 | 精确语义与解析行为 |
| :--- | :--- | :--- |
| **入口规则** | `Entry -> ...;` | 整个 `@path { ... }` 花括号内部 Token 流的顶层解析起点 |
| **序列（Sequence）** | `A ~ B`（或并列） | 先匹配规则 `A`，紧接着匹配规则 `B` |
| **有序选择（Ordered Choice）** | `A \| B` | **按列出的先后次序依次尝试**。若 `A` 匹配成功则直接采用 `A`，不再尝试 `B`（无歧义。） |
| **零或多次重复** | `A*` | 贪婪匹配零个或多个连续的 `A`，在映射函数中对应 `Repetition<...>` |
| **可选出现** | `A?` | 匹配零个或一个 `A` |
| **复用宿主 AST** | `FlurryExtendedStatement` 等 | 直接调用 Flurry 编译器内建的语法规则解析该片段 |

> **💡 核心心智模型：为什么选 PEG 的“有序选择（`|`）”而不是普通 CFG（上下文无关文法）？**
> 因为传统的 CFG 在遇到两条规则都能匹配同一段输入时，会产生致命的**语法二义性（Ambiguity）**，导致编译期报错或不可预测的行为。而 PEG 的有序选择 `RuleA | RuleB` 严格按照源码书写的先后顺序尝试：**谁排在前面且率先匹配成功，就归谁**。这保证了任何 `@define_macro2` 定义的 DSL 解析器都是 100% 确定性的。

---

## 三、 实战：用 `Entry` 限定并处理结构化语句块 `@statements`

让我们先从官方文档（`docs/macro/construction-macro.md`）中的基准示例 `@statements` 开始。
我们想定义一个块状宏 `@statements { ... }`：它要求花括号里的输入必须是一串合法的 `FlurryExtendedStatement*`，解析完成后由映射函数接收 `Repetition<meta.ast.FlurryExtendedStatement>` 并生成 `TokenBuffer`。

在宏准备脚本中：

```flurry
@use meta.macros.*;
use meta.ast.*;

const statements = @define_macro2 {
    -- 1. PEG 入口规则：花括号内必须是由 0 个或多个扩展语句组成的序列
    Entry -> FlurryExtendedStatement*;

    -- 2. 映射函数：直接拿到已经按 Entry 解析好的语句重复序列 body
    fn(body: Repetition<meta.ast.FlurryExtendedStatement>) -> TokenBuffer {
        {{ $body }}
    }
}
```

在已通过 `@use` 导入 `@statements` 的业务测试文件中：

```flurry
test {
    @statements {
        println("first");
        println("second");
    }
}
```

对比一下上一回（129）的 `^macro3 passthrough(tokens: TokenBuffer)`：
- `@passthrough { ... }` 对花括号里的内容**不做任何前置语法结构约束**，哪怕你在里面写半截乱序的括号对，也要等到展开后才在外部上下文里报错；
- 而 `@statements { ... }` 在进入映射函数 `fn(body: ...)` **之前**，就已经由 `Entry -> FlurryExtendedStatement*;` 严格核验了输入结构。在映射函数内部，你拿到的不是一盘散沙的 Token，而是整整齐齐的 `FlurryExtendedStatement` 语法树节点列表。

---

## 四、 进阶：定制多规则 DSL 与有序选择优先级

现在，让我们更进一步：不仅复用 `FlurryExpr`，还要在 `@define_macro2` 里加入我们自定义的字面关键字与子规则，为传感器网关定义一个小巧的**通道阈值配置 DSL `@channel_guard`**。

假设我们希望用户在代码里直接写出这样直观的网关检查块：
```flurry
@channel_guard {
    check temp <= 80;
    check voltage <= 12;
}
```
每次写 `check <expr> <= <expr>;`，都自动展开成一条 `assumes <lhs> <= <rhs>;`。

来看看在 `@define_macro2` 中如何用 `Entry` 和子规则 `CheckItem` 实现它：

```flurry
@use meta.macros.*;
use meta.ast.*;

const channel_guard = @define_macro2 {
    -- 定义单条检查规则与顶层入口规则
    Entry -> CheckItem*;
    CheckItem -> "check" ~ FlurryExpr ~ "<=" ~ FlurryExpr ~ ";";

    fn(items: Repetition<Tuple<FlurryExpr, FlurryExpr>>) -> TokenBuffer {
        let checks = items.fold({{}}, |acc, pair| {
            let lhs = pair.0;
            let rhs = pair.1;
            {{
                $acc
                assumes $lhs <= $rhs;
            }}
        });
        checks
    }
}

test {
    let temp: i32 = 65;
    let voltage: i32 = 11;

    @channel_guard {
        check temp <= 80;
        check voltage <= 12;
    }
}
```

那么，如果调用者在 `@channel_guard { ... }` 或 `@statements { ... }` 的花括号里写了违反 `Entry` PEG 文法的内容，会发生什么？

```flurry
-- ❌ 编译期直接拒绝：输入未能通过 @channel_guard 的 Entry PEG 规则解析（缺少 `<=` 与右侧上界表达式）
fn reject_invalid_peg_dsl_syntax() {
    let temp: i32 = 65;
    @channel_guard {
        check temp;
    }
}
```

编译器在 `prepare_input(mode == PEG)` 阶段就会静态拦截，并准确指出 `@channel_guard` 内部第几行第几列的 Token 违反了 `Entry` / `CheckItem` 规则。

---

## 五、 本篇小结

| 维度 | `@define_macro1`（AST 宏） | `@define_macro2`（PEG 构造宏） | `^macro3`（裸 Token 宏） |
| :--- | :--- | :--- | :--- |
| **调用括号形态** | `@path(arg1, arg2)` | `@path { ... }` | `@path { ... }` |
| **输入解析依据** | 处理函数参数签名中的 Flurry AST 类型 | 宏头部的 **`Entry -> ...` PEG 文法规则** | 仅做词法切分，不建语法树 |
| **自定义 DSL 语法能力** | 受限于逗号分隔的实参列表 | **极强**：可任意组合关键字、`~`、`|`、`*`、`?` 与宿主 AST | 需宏作者在函数内手写全套解析器 |
| **编译期成本计账** | 实参 AST 解析 + 映射函数 + 输出物化 | PEG 解析（含可选记忆化缓存）+ 映射函数 + 输出物化 | 词法获取 + 映射函数 + 输出物化 |

---

## 六、 下一篇预告

回顾我们在第 129 回列出的“构造宏四级输入光谱”：
- 第 1 级 `@define_macro1`（Flurry AST）——已掌握。
- 第 2 级 `@define_macro2`（PEG 语法）——已掌握。
- 第 3 级 `^macro3`（裸 `TokenBuffer`）——已掌握。

唯独还剩下最神秘的**第 4 级（`^macro4`：未经 Flurry 词法处理的原始源码文本 `str`）**。
你有没有注意过一个有趣的现象：在其他语言里，原始字符串（Raw String）往往是一套特殊的引号语法（比如 Rust 的 `r#"..."#` 或 C++ 的 `R"(...)"`）；而在 Flurry 里，原始字符串却写作：
```flurry
const SAMPLE = @builtin.str.raw {
    name = sample
    description = punctuation: $ @ ^
}
```
为什么 `@builtin.str.raw { ... }` 前面带着一个宏调用的 `@` 号？为什么它里面的 `$`、`@`、`^` 甚至未闭合的单双引号都不需要转义？当我们在 `package.fl` 和 `scripts/` 构建脚本里结合它生成源码时，构建系统又是如何调度的？

下一回 **《Flurry 演进手记 (131)：原始文本宏的妙用——@builtin.str.raw { ... } 与编译期源码生成脚本》**，我们将揭开原始文本宏与编译期构建脚本的协同奥秘。
