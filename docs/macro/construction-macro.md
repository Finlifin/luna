# 构造宏

## Intro

宏系统的输入粒度直接影响安全性和易用性。过粗的输入（如原始文本）难以保证语法正确性，过细的输入（如强制 AST）则限制了 DSL 的表达能力。因此，Flurry 的构造宏提供四种输入方式，开发者根据实际需求选择合适的粒度。

构造宏把输入交给编译时程序处理，再把生成的代码放回调用位置。宏可以要求输入符合 Flurry 语法，也可以为输入定义自己的结构。

选择输入方式时，先确定宏需要理解什么：语法节点、某种 DSL、token，还是原始文本。

## 语法

以下是本章使用的宏调用及 token 构造形式；宏体中的普通 Flurry 代码沿用[全局语法](../grammar/syntax)。

```text
macro_import -> @use path
ast_macro_call -> @path(argument*)
block_macro_call -> @path { macro_input }
token_buffer -> {{ token_part* }}
token_part -> token
    | $expr
ast_macro_definition -> @define_macro1 { mapping_definition }
peg_macro_definition -> @define_macro2 { peg_rule* mapping_definition }
```

`mapping_definition` 是由相应定义宏读取的处理函数签名与函数体。`peg_rule` 使用该宏的 PEG 规则语法；二者不是在全局普通函数语法中增加匿名声明形式。

裸 token 与原始文本宏分别用 `^macro3`、`^macro4` 标记符合其接口的 comptime 函数。输入和输出契约如下：

| 输入方式 | 定义入口 | 调用形态 | 处理函数取得的输入 |
|---|---|---|---|
| Flurry AST | `@define_macro1` | `@path(...)` | 按签名声明的语法节点 |
| PEG 语法 | `@define_macro2` | `@path { ... }` | 按 Entry 规则解析的结果 |
| 裸 token | `^macro3` | `@path { ... }` | `TokenBuffer` |
| 原始文本 | `^macro4` | `@path { ... }` | 未经 Flurry 词法处理的 `str` |

这些形式的共同输出是 `TokenBuffer`。宏导入等语句使用分号；实际调用的分隔符须使展开后的代码满足所在位置的语法。

## 正式定义

### 准备与可用性

构造宏由 comptime Flurry 程序定义。宏必须在使用它的源代码被解析之前准备好；可以来自依赖包，也可以放在本包先于 `src/` 编译的 `scripts/` 中。

`@use` 为当前文件引入宏符号。普通代码的 `use` 与宏导入分别承担各自的名称引入工作。

### 输入契约

AST 输入方式从处理函数签名确定参数应按哪一种 Flurry 语法规则读取。例如，参数要求 pattern 节点时，调用输入按模式解析，而不是先按表达式解析后猜测其含义。

PEG 输入方式对花括号内的 token 使用 Entry 规则。规则可复用 Flurry 的语法规则；有序选择、序列、重复与可选分别使用 `|`、`~`、`*`、`?` 等规则构造。有序选择按规则列出的次序尝试。

裸 token 输入方式直接取得 `TokenBuffer`，不要求先建立 AST 或 PEG 解析结果。原始文本输入方式取得原始源码字符，其内容不必先符合 Flurry 的词法规则。

### 输出与展开

处理函数返回 `TokenBuffer`。`{{ ... }}` 构造 token 序列，`$` 插入可转换为 token 的值，例如已有的 token buffer 或 AST 节点。插入会按相应表示转换规则形成输出 token。

输出在调用位置继续接受语法与后续语义检查。构造宏可以生成当前位置所需的代码片段，但返回 `TokenBuffer` 不等于已经证明该片段语法正确或类型正确。

生成代码中的运行时表达式在生成的程序中执行；宏处理输入和构造输出的工作在 comptime 执行。两者的求值与副作用不能混同。

## 常见示例

### 按 Flurry 模式读取输入

以下定义供宏脚本使用，处理函数的参数要求 Flurry pattern 节点。

```flurry
@use meta.macros.*;

const alternatives = @define_macro1 {
    fn(...patterns: Repetition<meta.ast.FlurryPattern>) -> TokenBuffer {
        let combined_pattern = patterns.fold({{ not _ }}, |acc, pattern_ast| {
            {{ $acc or $pattern_ast }}
        });

        {{ $combined_pattern => }}
    }
}
```

在已导入 `alternatives` 的文件中：

```flurry
test {
    let answer = "yes";
    if answer is do {
        @alternatives("yes", "Y") println("accepted"),
        _ => println("unknown"),
    }
}
```

宏生成匹配臂的模式与箭头，后面的语句仍由调用处提供。输入节点种类使编辑器可以知道调用参数处需要的是模式。

### 用 Entry 限定一个语句块

```flurry
@use meta.macros.*;
use meta.ast.*;

const statements = @define_macro2 {
    Entry -> FlurryExtendedStatement*;

    fn(body: Repetition<meta.ast.FlurryExtendedStatement>) -> TokenBuffer {
        {{ $body }}
    }
}
```

在已导入该宏的文件中：

```flurry
test {
    @statements {
        println("first");
        println("second");
    }
}
```

这个宏原样输出已经按 Entry 解析的语句。与直接接收 token 相比，输入必须先符合所声明的语句规则。

### 原样传递 token

在宏脚本的相应元编程环境中，使用 `macro3` 标记：

```flurry
^macro3
comptime fn passthrough(tokens: TokenBuffer) -> TokenBuffer {
    tokens
}
```

在已导入该宏的文件中：

```flurry
@passthrough {
    const ANSWER: i32 = 42;
}
```

处理函数没有把输入解释为语法树；输出回到声明位置后，仍需通过普通检查。

### 原始文本输入

内置原始字符串宏展示了无需先符合 Flurry 词法的输入：

```flurry
const SAMPLE = @builtin.str.raw {
    name = sample
    description = punctuation: $ @ ^
}
```

内容按原始文本处理。该宏的输出接入字符串表达式位置；它的文本输入方式不改变周围代码的语法。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与区别 |
|---|---|
| Rust | 过程宏的基本接口接收和产生 `TokenStream`；Flurry 构造宏还可按处理函数签名接入 Flurry AST 解析，或使用 PEG 入口及原始文本输入。[Rust 过程宏](https://doc.rust-lang.org/reference/procedural-macros.html) |
| Zig | 无对等的这组语法扩展入口；类型和值的生成可通过 comptime 完成。Flurry 构造宏还承担调用位置的源码结构生成。[Zig comptime](https://ziglang.org/documentation/master/#comptime) |
| C | 无对等的标准语言内 AST/PEG 宏处理函数；预处理宏替换 token，更复杂的语法处理可由外部生成器完成。Flurry 将处理函数与解析前的宏准备过程相接。[C 预处理宏](https://gcc.gnu.org/onlinedocs/cpp/Macros.html) |
| C++ | `#define` 执行预处理替换；模板实例化不提供这里的 AST/PEG 输入接口。Flurry 的构造宏返回 token，再对结果进行语言检查。[C++ 宏替换](https://eel.is/c++draft/cpp.replace) |

## 运行时实现设计引导

运行时执行宏生成的代码，不执行宏的输入解析器或 TokenBuffer 构造过程。

1. 取得已经展开、检查并实例化的目标代码。
2. 保留生成代码中的调用、控制流和资源操作。
3. 对生成的代码应用普通优化；只有保持语义的计算才能被删除或合并。
4. 若宏生成了计时、日志或其他运行时操作，按那些操作自己的契约计算成本。

不变量：宏脚本中的编译时副作用与所生成程序的副作用分别归属于各自的执行阶段。不能因为一段代码由宏产生，就把其中的 runtime 操作提前执行。

## 编译器实现引导

### 宏准备与解析接口

构造宏接入语法分析。前端在使用宏之前取得可调用的处理函数及输入描述：

1. 准备依赖包或 scripts 中的宏程序，建立宏导入对应的符号环境。
2. 在允许宏调用的语法位置识别 `@path`，记录调用处当前要求的语法位置。
3. 根据宏的输入描述检查调用分隔形式，并取得相应输入。
4. 执行 comptime 处理函数，验证输出为 TokenBuffer。
5. 将输出接回原解析位置，继续形成外层语法结构。
6. 对形成的代码执行普通的名称解析、normalization 与 instantiation 检查。

调用处、处理函数和生成片段的来源应关联保存。输入不符合规则、处理函数失败、输出类型不符或展开代码非法，应分别指明失败边界，不能统一报告成外围代码缺少标点。

### 按输入方式分派

```text
prepare_input(mode, source_region, descriptor):
    if mode == AST:
        return parse_flurry_nodes(source_region, descriptor.parameter_rules)
    if mode == PEG:
        tokens = obtain_tokens(source_region)
        return parse_entry(tokens, descriptor.entry_rule)
    if mode == TOKENS:
        return obtain_tokens(source_region)
    if mode == SOURCE:
        return original_source_text(source_region)
```

前置条件：调用的输入区域已由其分隔规则确定。原始文本模式必须在该区域被常规词法处理前保留原文，不能从 token 重新拼接一个丢失空白或字形的信息替代品。

### TokenBuffer 插入与后续检查

1. 遍历 token 构造中的固定片段与插入片段。
2. 对插入值使用其类型支持的 token 转换；记录转换失败的源表达式。
3. 按构造次序连接输出，保留每个片段的来源。
4. 将输出放回调用位置，而不是假设每次输出都是一个完整独立的表达式。
5. 将输出形成的代码交给相应语言检查；宏输出不能自行授予类型正确性或操作权限。

设输入长度为 N、处理函数成本为 F、输出长度为 E，总成本应分别计入输入解析、F、输出物化及后续解析。不能由输出载体是 TokenBuffer 推出整个宏过程为 O(N)。采用 PEG 记忆化是解析器实现选择，其缓存也需要计入空间成本。

对于宏准备依赖、展开链和 comptime 执行，应保留资源预算及失败溯源。缓存需要满足相应计算的依赖与纯性契约，不能只按宏名或源码字形缓存所有执行结果。
