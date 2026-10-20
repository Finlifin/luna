# 函数与闭包

## Intro

函数为计算建立一个接口：调用者提供输入，函数在约定的条件下使用它们，再交付结果。读一个接口，既要看参数与返回值，也要看输入何时具备、计算需要什么环境、结束后留下什么责任。

闭包让一段计算带着所需的局部值离开创建位置。创建环境、调用主体和清理环境是三个独立时刻；理解它们，就能解释回调为什么可以保存一个数值，却不能保存一个已经失效的局部地址。

### 阅读路线

| 读者要做的事 | 章节 | 贯穿用例 |
|---|---|---|
| 定义函数，理解一次调用的全过程 | [定义、调用与返回](calls-and-returns.md) | 字节解码、接收者修改、调用准备失败与清理 |
| 设计位置、命名及收集接口 | [参数与默认值](parameters.md) | 分页配置、异型元组、属性包与直接供应 |
| 让调用语法接入参数协议 | [参数 elaboration](parameter-elaboration.md) | 比较器、post do、嵌套 tactic lambda、implicit |
| 分开类型输入、编译时输入和运行时输入 | [参数阶段与子句](comptime_fn.md) | 数组接口、编译时选择、范式与普通执行 |
| 说明前提、结果和允许行为 | [函数契约与效果](contracts-and-effects.md) | 安全除法、状态修改、错误传播与效应处理 |
| 创建回调并选择捕获内容 | [Lambda 与捕获](lambdas.md) | 数值快照、指针观察、显式克隆和字段交付 |
| 判断回调还能否调用、如何结束 | [闭包环境、调用与清理](closure-environments.md) | 可重复读取、一次性交付、未调用清理与逃逸 |
| 接收、转交或返回带环境回调 | [函数值与高阶接口](higher-order-functions.md) | 动态选择、完整环境转交、回调组合 |
| 确定什么时候发生计算与展开 | [Inline 与 quote fn](inline-and-quote.md) | 普通实参一次求值、上下文访问、展开后的清理 |
| 复用签名、正文与测试上下文 | [函数模板与测试模板](templates.md) | 带检查的请求处理、多个正文、测试资源清理 |
| 把不同输入形态交给不同主体 | [函数级模式与分散定义](pattern-functions.md) | 命令分派、多参数模式、quoted arm 的绑定方向 |

每篇先给使用规则，再给完整例子及观察结果。标为“拒绝”的代码独立阅读，不和合法例子组成同一程序。后续代码块依赖前文定义时会明确说明。

## 语法

~~~text
function -> pure? comptime? inline? (unsafe | spec | verified | atomic)? fn id(parameter*) (-> result)? clauses? (block | (= expr))?
application -> expr(argument*)
lambda -> |anchored_parameter*| (-> expr)? expr
closure_type -> %closure_context expr
closure_context -> expr | { environment_binding* }
environment_binding -> id : expr (= expr)?
result -> (id :)? expr
quote_function -> quote fn id(parameter*) (-> result)? clauses? (block | (= expr))?
template_instance -> fn id use expr block
    | quote fn id use expr block
~~~

参数角色、函数修饰和子句见相应分篇及[全局语法](../grammar/syntax)。函数 ABI 由属性指定，例如 `^attrs.abi(.c)`。

## 正式定义

### 从接口到结果

一次普通 runtime 调用依次取得 callee 或接收者、按源码顺序计算显式实参、按声明依赖顺序补齐缺失的 implicit 和 optional 输入，再交付参数并进入主体。普通实参各计算一次。具名实参改变绑定位置，不改变源码求值次序。

按值交付 Copy 值保留源值；非 Copy 值移动责任。结果交付、错误传播和提前返回都必须处理实际留下的资源。返回一个指针或闭包，还必须交付其所依赖对象的有效性依据。

### 四个不同维度

| 维度 | 要回答的问题 | 例子 |
|---|---|---|
| 输入供应 | 谁提供这一个参数？ | 位置实参、optional 默认值、implicit 协议、变参收集 |
| 阶段 | 现在持有值，还是描述未来运行时的代码？ | comptime 参数、普通参数、quote 输入 |
| 执行方式 | 生成的程序怎样执行主体？ | 普通调用、优化内联、quote fn 的确定性接入 |
| 行为契约 | 允许做什么，完成后保证什么？ | requires、ensures、outcomes、环境与资源状态 |

这些维度可以在同一个接口中出现，但不是任意修饰符都能自由组合。lambda 的调用形参只有锚定参数；quote fn 不属于 comptime fn；类型范式的应用也不因使用编译时参数而展开成函数结果。

### 环境属于完整接口

普通 fn 类型接收无所需环境的可调用值。`%c fn(...) -> R` 同时保留函数接口和环境 c；c 为空时可以消去限定。非空环境不能靠隐式分配或丢弃捕获，伪装成普通 fn。

环境类型相同不表示运行时值相同。两个闭包可有相同接口，却保存不同的偏移量、指向不同对象，或具有不同的调用后状态。替换回调时还要满足行为契约。

## 常见示例

### 一个完整的带环境接口

下面按用户选择给出报价。折扣上限随回调一起交付，高阶函数只负责调用一次。

~~~flurry
fn quote_price(price: u32, lambda adjust: %c fn(u32) -> u32) -> u32
where c: Type.vars.Closure
{
    adjust(price)
}

test {
    let discount: u32 = 10;
    let reduced = quote_price(80) do |price: u32| {
        if price < discount {
            0
        } else {
            price - discount
        }
    }
    let unchanged = quote_price(80, |price: u32| price);
    println("{} {}", reduced, unchanged);
}
~~~

输出 `70 80`。第一次创建闭包时复制 discount，执行主体时才比较和减价。第二个闭包没有捕获。环境参数 c 使同一接口能够接收两者；lambda 标记使第一种 post do 写法有明确的接入位置。

### 一个接口中的不同阶段

~~~flurry
fn choose(comptime use_first: bool, first: u32, second: u32) -> u32 {
    inline if use_first {
        first
    } else {
        second
    }
}

fn input(value: u32) -> u32 {
    println("input {}", value);
    value
}

test {
    println("chosen {}", choose(true, input(4), input(9)));
}
~~~

输出依次是 `input 4`、`input 9`、`chosen 4`。use_first 在 comptime 决定实例，两个普通实参仍在 runtime 计算一次。选中的代码只使用 first，不授权删除 second 的输入效果。

## 与 Rust / Zig / C / C++ 的关键差异

C 的函数指针可以配合单独的上下文指针传递回调；Flurry 把环境要求放进完整类型。Rust 和 C++ 的闭包表达式具有各自独特的闭包类型；Flurry 按主体与上下文关系比较，而不按创建位置赋予名义身份。[C11 §6.7.6.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[Rust closure](https://doc.rust-lang.org/reference/types/closure.html)、[C++ lambda](https://eel.is/c++draft/expr.prim.lambda)

Zig 的 comptime 参数也能参与函数实例化；Flurry 另有明确的 comptime 函数类别、运行时环境限定与参数 elaboration 角色。[Zig functions](https://ziglang.org/documentation/master/#Functions)

## 运行时实现设计引导

按准备、交付、执行、退出四步实现调用。每一步保存实际取得的值和责任，失败时从当前状态退出；尚未取得的值不参与清理，已经交给 callee 的责任不在 caller 重复销毁。

闭包可以用代码入口与环境位置表示，也可以在特化后直接传递字段。ABI 排列、布局和去虚化属于后续选择，必须保留求值次序、环境 identity、返回依赖和清理行为。没有运行时开销的优化结果也不能改变源程序允许的操作。

## 编译器实现引导

对每一个定义建立四份相互关联的信息：

1. **输入描述**：绑定身份、参数角色、供应来源、阶段和前序依赖。
2. **完整类型**：主体签名、环境上下文、结果依赖、错误与效应限定。
3. **行为契约**：入口要求、允许效果、各出口的状态变换与责任交付。
4. **执行计划**：普通调用、代码接入或模板实例中的实际控制流及清理边。

先定位定义并完成参数 elaboration，再按各自阶段检查输入；lambda 捕获分析交付有次序的环境；模板和 quote 接入保留绑定来源。normalization 不展开类型范式的函数体。safe 检查在 normalized IR 完成，verified 的实现证明在 instantiated IR 完成。

PTC 负责类型命题；运行时访问、内存版本、可达性与调用后的资源状态由程序验证层处理。两层交付证据。最终生成代码前，必须确认实例所需的绑定、表示、代码接入和证明依赖均可交付。
