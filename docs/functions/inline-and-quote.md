# Inline 与 quote fn

## Intro

有时调用者希望把被调用计算展开到当前位置。普通内联是代码生成的选择；quote fn 则把代码接入作为应用必须完成的工作。

展开仍是一种有参数接口的计算。普通实参先取得一次，再供主体使用，不把它们按出现次数复制成多次执行。

## 语法

~~~text
inline_function -> inline fn id(parameter*) (-> result)? clauses? (block | (= expr))
quote_function -> quote fn id(parameter*) (-> result)? clauses? (block | (= expr))
implicit_quote_parameter -> implicit quote parameter
application -> expr(argument*)
~~~

quote fn 的普通调用沿用参数语法；contextual 输入使用 implicit quote。完整的代码值、单洞和替换规则见 [CMTT](../cmtt/index.md)。普通值跨阶段的 Splice 机制见[跨阶段值传递](../comptime/quoting-and-splicing.md)。

## 正式定义

### 三种不同工作

| 构造 | 必须完成的工作 | 运行时主体 |
|---|---|---|
| 普通／inline fn | 按接口调用；inline 提供内联意向 | 可保留合法调用，优化须保持行为 |
| comptime fn | 在编译时计算结果 | 不能留下运行时补算调用 |
| quote fn | 参数化代码接入必须完成 | 接入后执行主体中的 runtime 操作 |

quote fn 不属于 comptime fn。所有 quoted code 是 comptime 值，描述的代码仍可以是 runtime 计算。返回代码描述的 comptime 函数与保证展开的 quote fn 有不同的应用规则。

`inline if` 等控制流构造还承担实例化时选择代码的工作。一个普通 fn 可以含 inline if，却仍保留普通调用；一个 quote fn 也可以接入包含普通 runtime if 的代码，其分支要等 runtime 条件。函数展开、编译时分支选择和运行时求值是三件事。

当前实现已将 `inline fn` 的声明意向从解析、HIR 保留到 NIR 和具体 IIR 实例，dump 中以 `(inline true)` 表示。泛型实例、带 comptime 实参的实例、延迟签名实例和已选择的 trait 方法保留各自定义的意向；`pure` 和 comptime 阶段仍分别检查。默认执行路径保留普通调用。显式 NIR 优化入口 `elaboration::program::inlining::expand` 可展开普通 runtime 直接调用，复用卫生重命名、默认参数、契约和返回边界降码；一次 pass 只遍历原调用点，递归主体中的新调用保留。候选展开失败会撤销该候选的代码与元数据修改，保留普通调用。调用者在转换之后重新运行资源验证与清理计划发布。命令行可用 `--interpret entry --inline-functions --copy-protocol Copy --drop-protocol Drop` 显式启用此转换，其中协议名按源程序定义指定。驱动先展开，再检查资源并发布清理计划，最后执行；准备失败在执行前报告，runtime 失败不重跑原程序。此选项不用于无资源检查的解释执行。带 comptime 实参的调用等待输入完成，间接调用与 trait 方法选择暂不由此入口展开。handler 自身的局部控制流变换尚未接通，callee 与 handler 联合展开仍未完成。

### 实参只执行一次

普通 runtime 实参遵守调用顺序，各求值一次，并建立新鲜形参绑定。主体多次引用形参不重算实参；主体不引用某个形参也不自动省掉该实参的效果。

新鲜的是绑定身份，不是指针目标的身份。替换参数时，仍要保持实参之间已有的依赖、别名和资源关系。

普通参数与 contextual 输入不能互换解释：普通参数取得本次调用交付的值；contextual 输入对应生成代码要使用的绑定。代码接入复制的是受检代码结构，不是任意复制捕获资源或 pointee。

接入必须保持函数接口所要求的结果、控制流和清理。展开不赋予局部地址更长生命，也不取消错误传播或退出责任。

### Contextual 输入

`implicit quote` 将输入作为 runtime contextual 变量。它通过合法的上下文对应接入；普通用途无需在每次 splice 手写 substitute。对应仍须保留绑定身份、类型与依赖，不能只凭同名就任取一个对象。

显式命名供应也须交付正确的符号或绑定，不能把读取到的内容冒充 contextual 变量。编译时操作描述访问，真正的 runtime 读取仍发生在生成程序中。

### 代码接入边界

CMTT 代码采用 rank-1、两阶段和 runtime-only 上下文。每段 quote 至多一个 splice；quote 中的 quote fn 应用计作一个隐式接入点。普通 runtime 函数体并不自动是一段 quote，可以包含多个独立 quote fn 应用。

代码经由 Any/any、容器或别名传递，仍保留原有类别、上下文和接入限制。接口尚未恢复时不能假定其可以作为普通 runtime 函数指针调用。

quote fn 必须展开，不保证任意生成过程都终止。无法完成必须的接入或耗尽编译资源时，编译失败；不能悄悄退回一条普通 quote fn runtime 调用。主体中对普通 fn 的调用则可正常保留。

| 位置 | 接入点如何计算 |
|---|---|
| 一段 quote 中无 splice 或 quote fn 应用 | 零洞，允许 |
| 一段 quote 中一个显式 splice | 一个接入点 |
| 一段 quote 中一个 quote fn 应用 | 一个隐式接入点 |
| 同一 quote 的两个 runtime 分支各有一次接入 | 两个接入点，不能以分支互斥豁免 |
| 普通 runtime 函数体中两次独立 quote fn 应用 | 不把整个函数体视为一个 quote；分别检查接入 |

expr 代码使用上下文；pattern 代码在成功匹配后提供绑定；arm 还保存模式与成功主体的对应。quote fn 与[函数级模式](pattern-functions.md)提供 arm 的表面载体，不能把三个类别都作为普通无环境函数值接入。

## 常见示例

### 内联意向与必须接入

~~~flurry
inline fn smaller(left: u32, right: u32) -> u32 {
    if left < right {
        left
    } else {
        right
    }
}

quote fn smaller_code(left: u32, right: u32) -> u32 {
    if left < right {
        left
    } else {
        right
    }
}

test {
    println("{} {}", smaller(8, 3), smaller_code(8, 3));
}
~~~

输出 `3 3`。两者的数值行为一致，但前者提供内联意向，后者的代码接入必须完成。展开 smaller_code 不要求在 comptime 得到 left、right 的值；生成的比较仍可以是 runtime 比较。

### 重复使用参数不重复计算实参

~~~flurry
quote fn twice(value: u32) -> u32 {
    value + value
}

fn produce() -> u32 {
    println("produce");
    21
}

test {
    println("{}", twice(produce()));
}
~~~

依次输出 `produce`、`42`。其参数交付行为可由下面的独立测试表示：

~~~flurry
test {
    let result = do {
        let argument = produce();
        argument + argument
    }
    println("{}", result);
}
~~~

两个测试分别展示同一种行为。实际接入使用新鲜绑定，不能捕获调用者碰巧同名的 argument。

### 没使用的普通实参仍执行

~~~flurry
quote fn constant(value: u32) -> u32 {
    5
}

test {
    println("{}", constant(produce()));
}
~~~

输出 `produce`、`5`。把函数体展开后直接删掉 produce，会改变源程序行为。

### Contextual 变量保留实际对象

~~~flurry
struct Context {
    base: u32,
}

quote fn shifted(value: u32, implicit quote context: *Context) -> u32 {
    value + context.base.*
}

test {
    let state = Context { base: 10 }
    let context = state.ref;
    let first = shifted(1, .context = context);
    state.base = 20;
    let second = shifted(1, .context = context);
    println("{} {}", first, second);
}
~~~

输出 `11 21`。上下文接入保存同一对象的对应；它没有在 comptime 读取或冻结 base。显式 `.context` 展示供应来源，省略时由 implicit substitution 提供合法对应。

### 从模板的已知上下文自动接入

~~~flurry
quote fn read_current(implicit quote current: *u32) -> u32 {
    current.*
}

template fn with_current(
    value: u32,
    quote lambda body: < current: *u32 |- u32 >,
) -> u32 {
    let stored = value;
    let current = stored.ref;
    ~<body>
}

fn selected_value use with_current {
    read_current()
}

test {
    println("{}", selected_value(7));
}
~~~

输出 `7`。body 的接口已经提供 current，quote fn 在该 contextual 对应下接入，调用处无需再次手写替换表。stored 是本次调用建立的 runtime 对象；read_current 的代码接入不会在 comptime 读取它。详细模板流程见[正文复用](templates.md#基于-cmtt-的正文复用)。

### 新鲜参数绑定保留真实别名

~~~flurry
quote fn store_then_read(target: *u32, source: *u32) -> u32 {
    target.* = 9;
    source.*
}

test {
    let value: u32 = 3;
    let result = store_then_read(value.ref, value.ref);
    println("{} {}", result, value);
}
~~~

输出 `9 9`。target、source 是不同形参绑定，却指向同一对象。展开不能把它们看成两个独立存储，也不能把 source 的读取提前到写入之前。

### 普通函数体中的多个接入

~~~flurry
fn combine(left: u32, right: u32) -> u32 {
    let first = twice(left);
    let second = twice(right);
    first + second
}

test {
    println("{}", combine(2, 3));
}
~~~

输出 `10`。combine 是普通 runtime 主体，不因使用 quote fn 就受“一段 quote 只有一个接入点”的源码计数约束。

### 代码展开仍保留局部清理

~~~flurry
struct Trace {
    id: u32,
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

quote fn observed(value: u32) -> u32 {
    let trace = Trace { id: value }
    value
}

test {
    let result = observed(7);
    println("result {}", result);
}
~~~

输出 `drop 7`、`result 7`。trace 属于本次接入主体的语义作用域。不能把它的清理推迟到调用者整个测试退出，也不能因为返回值不读取 trace 就删除有输出的 Drop。

## 与 Rust / Zig / C / C++ 的关键差异

C++ inline 不要求实现必须以调用展开替代普通调用；Zig inline fn 将内联要求纳入函数调用处理。Flurry 的 quote fn 还保留代码上下文与阶段接口，不能只等同于优化提示或 token 宏。[C++ inline](https://eel.is/c++draft/dcl.inline)、[Zig inline functions](https://ziglang.org/documentation/master/#inline-fn)

C 的函数式宏可能重复展开实参文本；保持求值次数要由宏作者另行保证。quote fn 的普通实参则按调用规则取得一次。[C11 §6.10.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)

## 运行时实现设计引导

生成程序中先计算普通实参并取得责任，再执行接入后的主体。新鲜参数槽可以经优化消除，但其身份、求值次数和依赖关系必须在分析中保持。

contextual 输入可以直接对应已有位置，不要求为每个名字复制一个 runtime 对象。目标存储的访问资格与内容版本仍按实际执行更新。

对比普通调用的实现，quote fn 可以消去调用指令，却要保留相同的实参准备和逻辑退出：先取得输入，执行接入主体，交付结果，结束该次应用的局部责任，再继续 caller。调用边界被展开不表示语义作用域被合并。

## 编译器实现引导

1. 确定定义类别及本次参数实例，区分普通输入和 contextual 输入。
2. 核验代码接口、阶段与接入点计数；未解析来源保留义务，不先记为零。
3. 为普通实参建立按源码排序的新鲜绑定，保留未使用实参的效果。
4. 取得上下文替换，检查依赖和对象 identity，在绑定层替换主体。
5. 将实际控制流、结果与资源契约交付 NIR 检查，处理退出清理。
6. 确认 quote fn 应用已完成接入，后续优化仅改变合法实现，不补做语义上未完成的展开。

计数针对每份代码来源，不以展开后优化删除了节点为由绕过单洞。接入链的代价由实际生成规模决定；维护展开链、循环检测与预算，报错同时指明应用点和生成来源。

为避免错误的文本替换，至少检查三个不变量：同一普通实参只有一个求值节点；同一原始 runtime 对象在替换后保持 identity；每份局部责任只有一个与实际出口对应的清理路径。对经过 Any/any 的代码，先恢复实际类别、上下文及结果接口，再进行上述检查；相容描述本身不提供一段可展开的代码。
