# Flurry 演进手记 (129)：处理结构化 AST 输入——构造宏 @define_macro1 与 {{ ... }} Token Buffer

在上一回（128）中，我们摸清了 `define` 模板宏的边界：它只认识扁平的 Token，只能在当前源文件内使用，无法理解输入语法的内部结构。

但在构建工业级网络协议栈或编译器前端时，我们常常需要一种**跨模块复用、且能看懂 Flurry 语法树（AST）结构**的宏工具。
举个例子：在解析传感器网关的文本控制指令时，我们希望在 `if answer is do { ... }` 匹配块里写出这样优雅的分支：
```flurry
@alternatives("yes", "Y", "ok") println("accepted"),
```
我们希望 `@alternatives(...)` 自动把任意多个模式用 `or` 串联起来，并補上尾部的 `=>` 箭头，展开为：
```flurry
not _ or "yes" or "Y" or "ok" => println("accepted"),
```
请注意这里的两个硬核诉求：
1. 为什么它不能是一个普通的 `quote fn`？因为 `@alternatives(...)` 生成的是**半截匹配臂（`pattern =>`）**，不是一个完整的表达式。
2. 为什么它比纯 Token 宏更聪明？因为我们希望编译器和 IDE 明确知道：`@alternatives(...)` 括号里填的必须是合法的 **Flurry 模式（`FlurryPattern`）**，而不是随便什么乱七八糟的 Token。

今天，我们就来揭开 Flurry **构造宏（Construction Macros）** 的大幕，重点掌握第一类构造宏 **`@define_macro1`** 与构建输出的核心载体 **`{{ ... }}` Token Buffer**。

---

## 一、 全局鸟瞰：Flurry 构造宏的四级输入光谱

在设计宏系统时，各大语言往往陷入“要么太粗、要么太死”的两难：
- C 语言预处理宏和 Rust 过程宏（Procedural Macro）接收的是粗糙的 `TokenStream`，宏作者得自己在宏里手写半个语法分析器；
- 而如果强制所有宏只能接收固定的表达式 AST，又没法实现像嵌入式 SQL 或正则那样突破宿主语法的 DSL。

Flurry 的构造宏给出了一个极其体系化的解法：**统一输出为 `TokenBuffer`，但按宏需要理解的输入粒度，提供从细到粗的四种定义入口**。

```
+-------------------------------------------------------------------------+
|                   Flurry 构造宏的四级输入粒度光谱                        |
+-------------------------------------------------------------------------+
|                                                                         |
|  [ 第 1 级：Flurry AST 输入 (@define_macro1) ]  <-- 本篇主角             |
|  • 调用形态：@path(arg1, arg2, ...)                                     |
|  • 输入契约：按处理函数签名声明的语法节点（如 FlurryPattern）直接解析    |
|                                                                         |
|  [ 第 2 级：自定义 PEG 语法输入 (@define_macro2) ] <-- 下一篇 (130) 主角  |
|  • 调用形态：@path { custom_dsl_tokens }                                |
|  • 输入契约：按宏内定义的 Entry PEG 文法规则解析花括号内的 Token         |
|                                                                         |
|  [ 第 3 级：裸 Token 流输入 (^macro3) ]                                  |
|  • 调用形态：@path { arbitrary_tokens }                                 |
|  • 输入契约：直接取得未做语法树解析的 `TokenBuffer`。                    |
|                                                                         |
|  [ 第 4 级：原始源码文本输入 (^macro4) ]         <-- 第 131 篇主角       |
|  • 调用形态：@path { raw_source_characters }                            |
|  • 输入契约：在常规词法切分前直接取得原始字符串 `str`                    |
+-------------------------------------------------------------------------+
                       │ 共同返回类型：TokenBuffer (`{{ ... }}`)
                       ▼
           [ 接回调用位置，继续执行外层语法、名称、类型与资源检查。 ]
```

---

## 二、 核心语法：`@use`、`{{ ... $expr ... }}` 与 `@define_macro1`

在动手写第一个构造宏之前，我们先看清构造宏的三个核心语法构件（`docs/macro/construction-macro.md`）：

### 2.1 宏导入：`@use path;`
构造宏必须在消费者源码被解析（Parse）之前准备好（通常定义在依赖包或本包先于 `src/` 编译的 `scripts/` 目录中）。
在需要使用构造宏的源文件顶部，我们使用带 `@` 前缀的 **`@use path;`** 引入宏符号，与普通运行期/编译期代码的 `use path;` 职责分明。

### 2.2 构造输出：`{{ ... $expr ... }}` Token Buffer 字面量
构造宏的处理函数在编译期运行，最终必须返回一个 **`TokenBuffer`**。
怎么在代码里既直观又安全地拼装 `TokenBuffer`？用双花括号 **`{{ ... }}`**。
- 在 `{{ ... }}` 内部，普通 Token（包括诸如 `=>`、`or`、未闭合的语句片段）都会被原样收入缓冲区；
- 凡是遇到 **`$expr`**，编译器就会在编译期求值 `expr`，并将结果（例如一个已有的 `TokenBuffer` 或一个捕获到的 AST 节点如 `pattern_ast`）转换为对应的 Token 序列插值进去。

```
  {{  $acc   or   $pattern_ast  }}
      ──┬─   ─┬   ──────┬─────
        │     │         └──► 将 AST 节点 pattern_ast 序列化为 Token 插入
        │     └────────────► 字面 Token `or`
        └──────────────────► 将前序累积的 TokenBuffer `acc` 展开插入
```

### 2.3 AST 构造宏：`@define_macro1 { mapping_definition }`
在 `@define_macro1` 的花括号内部，我们编写一个映射处理函数（`mapping_definition`）。
关键之处在于：**前端解析器会先读取这个处理函数的参数类型签名（如 `Repetition<meta.ast.FlurryPattern>`），然后据此决定在调用处 `@alternatives(...)` 该启动哪一套 Flurry 语法规则去解析实参。**
比如参数声明要求 `FlurryPattern`，解析器在调用处就直接按“模式（Pattern）”规则解析，而绝不会机械地先按表达式解析再去猜它的意思。

---

## 三、 实战：编写模式折叠宏 `@alternatives`

让我们把上面这套机制组装起来，亲手实现官方规范中的经典案例 `alternatives`。

首先，在先于主源码编译的宏脚本（`scripts/`）中定义 `alternatives`：

```flurry
@use meta.macros.*;

-- 使用 @define_macro1 定义接收可变长 FlurryPattern AST 节点的构造宏
const alternatives = @define_macro1 {
    fn(...patterns: Repetition<meta.ast.FlurryPattern>) -> TokenBuffer {
        -- 从底数 `{{ not _ }}`（永假模式）出发，用 `or` 依次折叠每个捕获到的模式 AST
        let combined_pattern = patterns.fold({{ not _ }}, |acc, pattern_ast| {
            {{ $acc or $pattern_ast }}
        });

        -- 返回拼接了 `=>` 箭头的 TokenBuffer 片段。
        {{ $combined_pattern => }}
    }
}
```

接着，在已通过 `@use` 导入了 `alternatives` 的业务源文件中直接调用它：

```flurry
test {
    let answer = "yes";

    if answer is do {
        -- 1. 解析器根据签名知道括号内需要 FlurryPattern
        -- 2. @alternatives("yes", "Y") 展开为 `not _ or "yes" or "Y" =>`
        -- 3. 紧随其后的 `println("accepted")` 自动成为该匹配臂的右侧语句。
        @alternatives("yes", "Y") println("accepted"),
        _ => println("unknown"),
    }
}
```

> **🔍 语法小贴士：为什么 `@alternatives("yes", "Y")` 后面没有写 `=>`？**
> 因为 `return {{ $combined_pattern => }};` 已经把 `=>` 装进了返回的 `TokenBuffer` 里。当编译器把这段 `TokenBuffer` 接回 `if answer is do { ... }` 的分支位置继续解析时，它与后面的 `println("accepted"),` 严丝合缝地拼成了一条完整的 `case_arm`（`pattern => statement`）。

---

## 四、 进阶：裸 Token 构造宏 `^macro3` 与展开后的双重防线

有时候，我们既想通过 `@path { ... }` 跨模块调用一个块状宏，又不需要前端预先把花括号里的内容解析成特定的 AST 节点，而是想直接拿到一整块 `TokenBuffer` 做原样转发或重组。

这时候，我们可以使用第 3 级构造宏入口——在符合签名的 `comptime fn(tokens: TokenBuffer) -> TokenBuffer` 头顶挂上 **`^macro3`** 属性。

```flurry
-- 在宏脚本环境中定义 ^macro3 裸 Token 处理函数
^macro3
comptime fn passthrough(tokens: TokenBuffer) -> TokenBuffer {
    tokens
}

-- 在导入了 passthrough 的业务文件中使用块状调用 @path { ... }
@passthrough {
    const ANSWER: i32 = 42;
}

test {
    assumes ANSWER == 42;
}
```

### 4.1 构造宏的两道铁面检查防线

请务必牢记（`docs/macro/construction-macro.md`）：**构造宏成功返回了一个 `TokenBuffer`，绝不等于证明了生成的代码已经合法。**
编译器对构造宏设置了两道独立的检查防线：

1. **防线一（入口契约检查）**：在 `@define_macro1` 调用处，如果传入的参数根本不符合处理函数签名要求的语法节点类别（比如要求 `FlurryPattern`，你却传了一段无法作为模式解析的非法语句片段），在进入宏处理函数之前就会直接报错。
2. **防线二（展开后位置检查）**：当宏返回的 `TokenBuffer` 接回原调用位置后，必须完整通过该位置的语法、名称解析、类型推导（Elaboration）与资源检查。

来看一个触发防线二的编译期拒绝例：

```flurry
-- ❌ 编译期直接拒绝：尽管 passthrough 成功返回了 TokenBuffer，
-- 但接回声明位置后，将字符串 "not_an_int" 赋给 i32 违反了静态类型检查。
@passthrough {
    const BROKEN_ANSWER: i32 = "not_an_int";
}
```

而且，Flurry 编译器会精细地关联保存**调用处位置、宏处理函数位置与每个生成片段的来源（Span）**。无论是输入不符合语法、宏函数内部报错、还是展开后的代码类型不匹配，编译器都会精准指出真正的失败边界，绝不会像粗糙的文本宏那样笼统报一句“外围代码缺少标点符号”。

---

## 五、 本篇小结

| 特性 / 构件 | 语法形式 | 核心契约与工程价值 |
| :--- | :--- | :--- |
| **宏导入** | `@use path;` | 为当前文件引入已在依赖包或 `scripts/` 中准备好的构造宏符号 |
| **Token 缓冲区字面量** | `{{ token_part* }}` | 构造 `TokenBuffer`；内部用 `$expr` 将 AST 节点或子 `TokenBuffer` 插值转为 Token |
| **AST 构造宏** | `@define_macro1 { fn(...) -> TokenBuffer }` | 调用形式 `@path(args)`；按函数参数签名指定的 Flurry AST 规则解析实参 |
| **裸 Token 构造宏** | `^macro3 comptime fn(TokenBuffer) -> TokenBuffer` | 调用形式 `@path { ... }`；直接接收花括号内的 `TokenBuffer` |
| **求值阶段隔离** | 编译期生成 vs 运行期执行 | 宏处理与 `TokenBuffer` 拼装在 `comptime` 执行；生成的运行期表达式在目标程序中执行 |

---

## 六、 下一篇预告

`@define_macro1` 让我们能够非常优雅地接收标准的 Flurry AST 节点（如 `FlurryPattern`、`FlurryExpr`）。

但是，如果我们要在传感器网关工程里内嵌一门**自定义的领域特定语言（DSL）**——比如一段自定义语法的协议状态转移表、一条路由规则或者一段受限的语句块——它的顶层结构压根就不是单个标准的 Flurry 表达式，甚至有自己独特的组合顺序和分隔规则；而我们又不想退化到 `^macro3` 去手写整个词法/语法循环解析器。

能不能直接在宏定义里写几行 **PEG（解析表达文法，Parsing Expression Grammar）** 产生式（比如 `Entry -> ...`），让编译器前端自动按我们定制的语法树规则把 `@my_dsl { ... }` 解析好喂给处理函数？

下一回 **《Flurry 演进手记 (130)：在 Flurry 里直接嵌一门小语言（DSL）——基于 PEG 语法的构造宏 @define_macro2》**，我们将解锁构建嵌入式 DSL 的核心机制。
