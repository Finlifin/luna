# 引用代码与控制生成阶段

## Intro

引用一个表达式会取得它的代码，而不是执行它。生成器可以在编译时选择这份代码，等接入完成后再由运行时执行其操作。

这使程序结构能够在编译时确定，同时保留真实输入在运行时才出现的边界。

## 语法

~~~text
contextual_expr_type -> < runtime_binding* |- expr >
quote_expr -> '(expr) | '[ substitution* ] (expr)
splice_expr -> ~<expr> | ~[ substitution* ] <expr>
quote_function -> quote fn id(parameter*) (-> result)? clauses? (block | (= expr))?
~~~

## 正式定义

### 引用保存什么

< Ψ |- T > 表示一份表达式代码：当 Ψ 中的绑定得到合法供应时，该表达式按 T 交付 runtime 结果。Ψ 为空只表示没有这些局部绑定需求，不保证代码纯、无全局访问或无效应。

quote 固定代码中已经解析的声明来源和绑定关系。它可以保留尚待实例化的参数与义务，但接入目标前必须兑现本次需要的检查。

### 两阶段

生成器及代码值在 comptime，代码主体在 runtime。splice 的操作数回到 comptime 取得代码，再把主体接到 runtime 位置。操作数中的生成器可以构造另一份 quote，这仍只有两个阶段。

不能生成一个 runtime 通用 quote 值，也不能通过 Any 把只有编译时表示的代码放进运行时对象。类型或 quoted code 用作编译时参数，与真正作为运行时字段交付，是不同要求。

普通数据通过 Splice 转换为运行时值；代码 splice 接入程序和绑定。为数据实现 Splice，不会自动得到代码类别、上下文或控制接口的合法性。

### Rank-1

上下文参数在生成器或模板最外层量化。一次应用确定相应上下文后，代码参数引用这次实例的接口。以下为内部记号：

~~~text
允许：forall Ψ,T. Expr(Ψ,T) -> Expr(Ψ,T)
不提供：把 forall Ψ. Expr(Ψ,T) 作为普通参数，函数内任意实例化 Ψ
~~~

第一条可以在不同调用处接受不同上下文；它不允许在某次调用内部凭空改变固定外部对象身份。any 不代表一个新的上下文量化变量。

### 单洞

每个 quote 的 runtime 主体至多一个接入点。块、lambda 和运行时分支里的接入都属于这份主体；互斥分支各写一次仍是两个。重复使用同一代码值也不抵消次数。

quote fn 在 quote 内的应用计一个隐式接入；它自己的主体独立检查。通过带 arm 接入协议的 use 聚合规则也计一个接入，而不按最终 arm 数量累计。

splice 操作数中的 comptime 计算不属于外层 runtime 主体。普通 runtime 函数体不是 quote，可有多个独立接入。展开后删除 splice 节点或常量折叠不能补救原本有两个洞的 quote。

## 常见示例

### 引用输出动作没有提前输出

~~~flurry
pure comptime fn greeting_code() -> < |- void > {
    '(println("hello"))
}

fn greet() { ~<greeting_code()>; }

test {
    println("before");
    greet();
    println("after");
}
~~~

运行时输出 before、hello、after。生成 greeting_code 的代码值不执行 println，因此生成器的 pure 与生成主体的输出并不矛盾。

### 一个洞增加一层计算

~~~flurry
pure comptime fn add_one(code: < x: i32 |- i32 >)
    -> < x: i32 |- i32 >
{
    '(~<code> + 1i32)
}

fn add_two(value: i32) -> i32 {
    const original: < x: i32 |- i32 > = '(x + 1i32);
    const combined = add_one(original);
    ~[x = value]<combined>
}

test { println("{}", add_two(40i32)); }
~~~

输出 42。两次构造各自满足单洞，最终生成的程序仍可包含两次加法。

### 编译时选择代码

~~~flurry
pure comptime fn chosen_code(first: bool) -> < |- u32 > {
    if first { '(11u32) } else { '(22u32) }
}

fn selected(comptime first: bool) -> u32 {
    ~<chosen_code(first)>
}

test { println("{} {}", selected(true), selected(false)); }
~~~

输出 11 22。如果 first 是 runtime 输入，生成器不能提前读取它；应生成包含 runtime 条件的代码。

### 拒绝同一 quote 的两个洞

~~~flurry
pure comptime fn invalid_sum(left: < |- i32 >, right: < |- i32 >)
    -> < |- i32 >
{
    '(~<left> + ~<right>)
}
~~~

这是拒绝例。把两个 splice 分别放进 if 的两支，或通过 Any 传递它们，也不能改变计数。需要一个能接收完整接口的组合协议或模板来承担组合，不能以事后优化作为合法性依据。

## 与 Rust / Zig / C / C++ 的关键差异

Zig comptime 和 C++ 模板都可以让编译时输入决定生成结构；C 与 Rust 宏可以先替换 token。Flurry 在这里另外规定代码值的阶段、runtime 上下文和每份 quote 的接入数量，不能直接将这些语言的宏或模板文本当作 quoted code。[Zig comptime](https://ziglang.org/documentation/master/#comptime)、[Rust 声明宏](https://doc.rust-lang.org/reference/macros-by-example.html)

## 运行时实现设计引导

运行时不存在“执行 quote 操作数”的步骤。代码接入后保留普通计算；被编译时选择排除的代码不参与目标执行。生成器内的纯计算不能用于删除已选 runtime 主体中的可观察动作。

## 编译器实现引导

1. 划分当前 quote 的 runtime 主体与 splice 操作数的 comptime 子树。
2. 根据目标接口建立虚拟 runtime 绑定，检查主体的普通使用。
3. 在代码接口分类完成后统计显式与隐式接入点；未决应用保留计数义务。
4. 核验 runtime-only 上下文和 rank-1 实例化，不把未知上下文变成空上下文。
5. 保存声明来源、主体、绑定、结果契约及未决义务。
6. 接入前取得具体所需证据，接入后不留无法实现的第三阶段操作。

逐节点计数对一份源码 quote 线性进行，不需要先完整展开所有生成代码；被接入代码保存自己的检查结果。诊断应指向第二个接入点及所属 quote，而非只报告最终目标类型不匹配。
