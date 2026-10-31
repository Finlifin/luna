# 参数化代码、模板与规则接入

## Intro

quote fn 把一份表达式代码参数化。应用时，它把普通实参绑定好，再将主体接入调用位置，因此可以复用运行时代码而不依赖优化器是否决定内联。

模板可以只复用签名，也可以建立供使用者正文使用的上下文。两种用途分别解决接口重复与代码重复。

## 语法

~~~text
quote_function -> quote fn id(parameter*) (-> result)? clauses? (block | (= expr))?
contextual_parameter -> implicit quote parameter
body_parameter -> quote lambda parameter
template_signature -> template fn id(parameter*) (-> result)? clauses?
template_quote_signature -> template quote fn id(parameter*) (-> result)? clauses?
template_instance -> fn id use expr block
    | quote fn id use expr block
    | quote fn id(parameter*) use expr block
arm_import -> use expr
~~~

模板签名以分号结束。arm_import 出现在接收规则的 match 内，按分支项用逗号分隔；它的目标须提供 arm 接入协议。

## 正式定义

### 函数类别与代码阶段

quote fn 不属于 comptime fn。它描述的代码在 runtime 执行，描述本身可由 comptime 计算保存和组织。返回代码值的 comptime fn 则在编译时执行生成工作，再由使用处 splice。

子句参数对应的外层 comptime 定义与实例化得到的 quote fn 分别判断。普通参数、comptime 参数和 contextual 参数也各有自己的阶段，不能仅因展开发生在编译时，就读取普通实参的未来内容。

### 普通实参与 contextual 输入

普通 runtime 实参按调用顺序各求值一次，为形参建立新鲜绑定。形参出现两次不重算实参；形参未使用也不自动删除实参的效果。

implicit quote 标明 runtime contextual 输入。它按合法绑定对应接入，保持对象身份、别名和依赖，不要求每项额外建立一个普通值参数槽。普通 implicit 参数则按自己的供应协议处理。

quote fn 必须完成接入；失败时报告编译错误，不退回一个普通的 quote fn runtime 调用。主体对普通 fn 的调用仍可保留，递归生成也必须实际完成，不能因称为 quote 而承诺无限展开终止。

### 逻辑完成边界

quote fn 内 return 完成本次应用，不返回其调用者。编译器可以用结果汇合出口表达这层逻辑边界，不必为它保留真实调用帧。

参数临时量、局部对象、错误传播和退出清理仍遵守该逻辑边界。展开不延长局部对象寿命，也不取消清理。

### 模板的两层复用

简单 template 提供签名；普通 fn、inline fn、quote fn 等非 comptime 实例提供主体，并保留各自类别。

高级 template fn 用 quote lambda 参数绑定实例正文，并在明确的 contextual 接口下接入。template test 则以唯一锚定子句参数绑定正文。模板授予哪些局部绑定由接口规定，不自动暴露所有内部变量。

### Quoted arm 与 open code

模板的 arm 接入属性让函数级模式及主体作为完整规则交付。每条规则拥有自己的成功绑定；contextual 输入先接到外部接口，再一致地供应模式、结果类型和主体。

单条规则可以按其模板契约不穷尽；最终 match 仍须穷尽或提供合法失败路径。规则选择集合和分派顺序由接入协议确定，不能从文件顺序推导。这里的 use 消费 arm 接口，不让任意 comptime 容器自动成为规则集合。

## 常见示例

### 实参出现两次，求值一次

~~~flurry
quote fn twice(value: i32) -> i32 { value + value }

fn produce_number() -> i32 {
    println("produce");
    21i32
}

test { println("{}", twice(produce_number())); }
~~~

输出 produce、42。生成程序先取得 produce_number 的结果，再使用该绑定两次。

### Return 只完成本次应用

~~~flurry
quote fn clamp(value: u32) -> u32 {
    if value > 8u32 { return 8u32; }
    value
}

fn caller(value: u32) -> u32 {
    let bounded = clamp(value);
    bounded + 1u32
}

test { println("{} {}", caller(20u32), caller(3u32)); }
~~~

输出 9 4。clamp 中的 return 不会使 caller 直接返回 8。

### 模板签名不改变实例类别

~~~flurry
template fn unary(value: i32) -> i32;
fn ordinary use unary { value * 2i32 }
quote fn generated use unary { value * 2i32 }

test { println("{} {}", ordinary(21i32), generated(21i32)); }
~~~

两个结果都是 42。generated 必须接入，ordinary 仍是普通调用。

### Contextual 变量由模板供应

~~~flurry
quote fn add_current(value: u32, implicit quote current: u32) -> u32 {
    value + current
}

template fn with_current(
    value: u32,
    quote lambda body: < current: u32 |- u32 >,
) -> u32 {
    let current = value;
    ~<body>
}

fn adjusted use with_current { add_current(2u32) }

test { println("{}", adjusted(40u32)); }
~~~

输出 42。正文已经在模板授予的 current 下检查，add_current 使用同一上下文对应。implicit quote 不读取 comptime 中不存在的 runtime 值。

### 将多入口规则接入匹配

下面是编译器中大量规则的一个缩小片段。本例的模板接入集合包含这里定义的 add_rule 与 multiply_rule；模式互斥，不依赖规则收集顺序。

~~~flurry
enum Operation {
    literal(u32),
    add(u32, u32),
    multiply(u32, u32),
}

^attrs.quote_fn_template.for_match_arms
^attrs.quote_fn_template.allow_non_exhaustive_instance
template quote fn evaluate_rule(operation: Operation) -> u32;

quote fn add_rule(case .add(left, right)) use evaluate_rule {
    left + right
}

quote fn multiply_rule(case .multiply(left, right)) use evaluate_rule {
    left * right
}

fn evaluate(operation: Operation) -> u32 {
    operation match {
        use evaluate_rule,
        .literal(value) => value,
    }
}

test { println("{}", evaluate(Operation.multiply(6u32, 7u32))); }
~~~

输出 42。单条规则不覆盖全部 Operation，组合后则覆盖三个变体。每条规则的 left 和 right 只属于自己的成功主体。

## 与 Rust / Zig / C / C++ 的关键差异

C++ inline 不承诺消除机器调用，函数模板负责实例化；C 宏通过文本替换复用代码。Rust 宏和 Zig comptime 也可以控制生成结构。Flurry 的 quote fn 另有必须接入、普通实参一次求值和 contextual 绑定契约。[C++ inline](https://eel.is/c++draft/dcl.inline)、[C++ 函数模板](https://eel.is/c++draft/temp.fct)

## 运行时实现设计引导

接入的代码保留实参临时量、局部控制出口及普通资源责任。尾部结果进入调用位置，提前 return 进入本次应用的结果汇合点。

模板的后续普通语句不是 finally。正文提前退出时，只有资源协议要求的清理保证执行；不能因代码来自模板就额外执行被跳过语句。

## 编译器实现引导

1. 取得实际定义类别与参数角色，并执行本次合法的编译时实例化。
2. 按源码次序生成普通实参的一次求值与新鲜绑定。
3. 通过已有接口供应 contextual 输入，保持依赖对象身份。
4. 为 quote fn 建立逻辑完成边界，接入主体；对模板正文使用它授予的控制出口。
5. 对 arm 同时实例化模式和成功主体，再检查最终目标的覆盖与结果。
6. 统计所属 quote 的隐式接入点；被调用代码独立检查，不能按最终节点数重新免除限制。
7. 完成目标的类型、效果和资源检查，无法兑现的接入报告展开链与来源。

缓存必须区分模板接口、实例主体、规则集合和替换。不同实例同名的局部绑定不能共享 runtime 身份。
