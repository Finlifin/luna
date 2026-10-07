# CMTT：带上下文的代码生成

## Intro

代码生成不仅要保存一段程序，还要知道它使用哪些绑定、产生什么结果。CMTT 把这些信息放在代码的接口中，使生成器可以组合尚未获得真实运行时输入的程序。

表达式代码使用上下文，模式代码在匹配成功后产生上下文，arm 代码把模式与使用其绑定的主体一起保存。

## 语法

~~~text
contextual_expr_type -> < runtime_binding* |- expr >
contextual_pattern_type -> < expr -| runtime_binding* >
quote_expr -> '(expr) | '[ substitution* ] (expr)
quote_pattern -> '<pattern> | '[ substitution* ] <pattern>
splice_expr -> ~<expr> | ~[ substitution* ] <expr>
splice_pattern -> ~<expr> | ~[ substitution* ] <expr>
substitution -> id = id
runtime_binding -> id : expr
~~~

替换表中的项以逗号分隔。pattern splice 的操作数也是一个 comptime 表达式，它求得模式代码；操作数本身不是源码模式。

## 正式定义

### 两种阶段，三种代码

所有 quoted code 都是 comptime 值。生成器在编译时取得、保存和组合这些值；生成的主体在 runtime 执行。引用运行时绑定保存的是绑定关系，不读取未来对象的内容。

| 类别 | 接口方向 | 使用时机 |
|---|---|---|
| expr | 给定上下文 Ψ，产生 T | 接入表达式位置 |
| pattern | 匹配 T，成功产生 Ψ | 接入模式位置 |
| arm | 外部上下文、模式、成功绑定及主体连在一起 | 接入函数级分支或匹配规则 |

上下文 Ψ 只包含 runtime 绑定，可以有多个成员并保持依赖次序。类型、代码值及生成器元信息处于 comptime 环境，不是 Ψ 中的运行时数据。

上下文多态采用 rank-1：生成器或模板的最外层可以量化上下文，使用时实例化；不能把仍可任意实例化的上下文多态方案塞进一个单态代码参数。

每段 quote 至多一个代码接入点。显式 splice 和 runtime 位置的 quote fn 应用均计数；普通函数体不自动是一段 quote。单洞不限制生成器最终能生成多少语句、状态或分支。

### 阅读路线

| 文档 | 内容 |
|---|---|
| [代码引用与阶段](quotation-and-stages.md) | 封闭与开放代码、两阶段、rank-1、单洞 |
| [上下文与替换](contexts-and-substitution.md) | 默认对应、显式改名、依赖与新鲜绑定 |
| [模式与 quoted arm](patterns-and-arms.md) | 成功上下文、定向检查和规则接合 |
| [Any 与代码见证](any-and-code-evidence.md) | 通配描述、真实接口与安全接入 |
| [Quote 函数与模板](quote-functions-and-templates.md) | 参数化代码、一次求值、逻辑返回和模板正文 |
| [组装有限程序](program-construction.md) | 循环生成、状态分派与正则自动机示例 |

## 常见示例

~~~flurry
pure comptime fn answer_code() -> < |- u32 > {
    '(42u32)
}

fn answer() -> u32 {
    ~<answer_code()>
}

test {
    println("{}", answer());
}
~~~

answer_code 在 comptime 返回代码值；answer 的生成主体在 runtime 交付 42。代码值不会作为一个通用运行时代码对象传到目标程序。

开放代码需要绑定供应：

~~~flurry
fn increment(value: i32) -> i32 {
    const code: < x: i32 |- i32 > = '(x + 1i32);
    ~[x = value]<code>
}

test { println("{}", increment(41i32)); }
~~~

x 是代码接口中的绑定，value 是接入处的真实 runtime 绑定；结果为 42。

## 与 Rust / Zig / C / C++ 的关键差异

C 预处理宏和 Rust 声明宏组织 token，C++ 模板与 Zig comptime 可按编译时输入生成不同程序。这些构造不直接提供本篇把显式 runtime 上下文放进代码类型、并让模式输出上下文的组合接口。[Rust 声明宏](https://doc.rust-lang.org/reference/macros-by-example.html)、[Zig comptime](https://ziglang.org/documentation/master/#comptime)

理论基础中的显式上下文与替换可参照 [Contextual Modal Type Theory](https://www.cs.cmu.edu/~fp/papers/tocl07.pdf) 与[上下文多态扩展](https://arxiv.org/abs/1801.09225)。Flurry 的 pattern、arm、单洞及 Any 接合由本主题自己的规则规定，不从论文自动继承其元理论结论。

## 运行时实现设计引导

runtime 只执行接入后的程序。普通调用、分支、对象访问和清理保持原有契约；quote 本身不要求在 runtime 分配代码对象或解释源码。

代码共享不等于资源共享或复制。两次接入如果使用同一份运行时 owner，必须检查生成后的真实使用顺序。

## 编译器实现引导

编译器分别保存阶段环境、runtime 绑定上下文、代码类别、结果契约和控制接口。quote 构造这些信息；splice 恢复接口、建立合法替换、新鲜化内部绑定，并接回目标程序。

接入后的程序仍接受阶段、类型、效果、可达性和资源检查。普通 Splice trait 的数据转换不替代这里的代码接入检查。
